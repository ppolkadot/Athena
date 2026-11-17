//! Eight-step validated backup restore pipeline (`§9.3`).
//!
//! Each step is an explicit named function behind [`RestoreSteps`] so tests can inject a failure
//! at every seam and verify rollback, staging cleanup, and live-data preservation.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, ErrorCode, OpenFlags};

use super::{Db, backup, integrity, migrations, profile_repo, register_functions};
use crate::infrastructure::config::AppDirs;
use crate::shared::error::{AppError, RestoreError, Result};

/// Where a backup to restore came from (`§8.1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreSource {
    /// A file inside `dirs.backups_dir`.
    Listed(PathBuf),
    /// A file chosen from outside `dirs.backups_dir`.
    External(PathBuf),
}

impl RestoreSource {
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Listed(path) | Self::External(path) => path,
        }
    }
}

/// Summary returned after a successful restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReceipt {
    pub source: PathBuf,
    pub snapshot: PathBuf,
}

type VerifyCheckFn = fn(&Connection) -> Result<()>;

#[derive(Clone, Copy)]
pub(crate) struct RestoreSteps {
    pub resolve: fn(&AppDirs, &RestoreSource) -> Result<PathBuf>,
    pub stage: fn(&Path, &Path) -> Result<()>,
    pub validate: fn(&Path, i64) -> Result<()>,
    pub upgrade: fn(&Path) -> Result<()>,
    pub snapshot: fn(&Connection, &AppDirs, DateTime<Utc>) -> Result<PathBuf>,
    pub copy: fn(&mut Db, &Path) -> Result<()>,
    pub verify: fn(&mut Db, &Path, &Path) -> Result<()>,
    pub cleanup: fn(&AppDirs, &Path, &Path, &Path) -> Result<()>,
}

impl Default for RestoreSteps {
    fn default() -> Self {
        Self {
            resolve: step_resolve,
            stage: step_stage,
            validate: step_validate,
            upgrade: step_upgrade,
            snapshot: step_snapshot,
            copy: step_copy,
            verify: step_verify,
            cleanup: step_cleanup,
        }
    }
}

/// Restore `source` into `live_db` (`§9.3`).
///
/// # Errors
/// [`AppError::Restore`] when the source cannot be resolved, staged, validated, upgraded,
/// snapshotted, copied, or verified. Steps 1–6 leave `live_db` untouched; step 7 rolls back to
/// the pre-restore snapshot.
pub fn restore_backup(
    live_db: &mut Db,
    dirs: &AppDirs,
    source: &RestoreSource,
    now_utc: DateTime<Utc>,
) -> Result<RestoreReceipt> {
    restore_backup_with_steps(live_db, dirs, source, now_utc, RestoreSteps::default())
}

pub(crate) fn restore_backup_with_steps(
    live_db: &mut Db,
    dirs: &AppDirs,
    source: &RestoreSource,
    now_utc: DateTime<Utc>,
    steps: RestoreSteps,
) -> Result<RestoreReceipt> {
    let resolved = (steps.resolve)(dirs, source)?;
    let live_page_size: i64 = live_db
        .conn()
        .pragma_query_value(None, "page_size", |r| r.get(0))
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;

    let staged_path = staged_restore_path(dirs, now_utc);
    if let Err(err) = (steps.stage)(&resolved, &staged_path) {
        remove_staged(&staged_path);
        return Err(err);
    }

    if let Err(err) = (steps.validate)(&staged_path, live_page_size) {
        remove_staged(&staged_path);
        return Err(err);
    }

    if let Err(err) = (steps.upgrade)(&staged_path) {
        remove_staged(&staged_path);
        return Err(err);
    }

    let snapshot = match (steps.snapshot)(live_db.conn(), dirs, now_utc) {
        Ok(path) => path,
        Err(err) => {
            remove_staged(&staged_path);
            return Err(err);
        }
    };

    if let Err(err) = (steps.copy)(live_db, &staged_path) {
        remove_staged(&staged_path);
        return Err(err);
    }

    if let Err(err) = (steps.verify)(live_db, &resolved, &snapshot) {
        remove_staged(&staged_path);
        return Err(err);
    }

    if let Err(err) = (steps.cleanup)(dirs, &staged_path, &snapshot, &resolved) {
        tracing::warn!("post-restore cleanup failed: {err}");
        remove_staged(&staged_path);
    }
    Ok(RestoreReceipt {
        source: resolved,
        snapshot,
    })
}

fn staged_restore_path(dirs: &AppDirs, now_utc: DateTime<Utc>) -> PathBuf {
    let base = format!("restore-{}", now_utc.format("%Y-%m-%d_%H-%M-%S"));
    let first = dirs.staging_dir.join(format!("{base}.db"));
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    for n in 1_u32.. {
        let candidate = dirs.staging_dir.join(format!("{base}-{n}.db"));
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    unreachable!()
}

fn remove_staged(staged_path: &Path) {
    let _ = std::fs::remove_file(staged_path);
    for suffix in ["-wal", "-shm"] {
        if let Some(name) = staged_path.file_name().and_then(|n| n.to_str()) {
            let _ = std::fs::remove_file(staged_path.with_file_name(format!("{name}{suffix}")));
        }
    }
}

/// Step 1: Reject the live database path (canonicalized), non-files, empty files, and `Listed`
/// paths outside `dirs.backups_dir`.
pub(crate) fn step_resolve(dirs: &AppDirs, source: &RestoreSource) -> Result<PathBuf> {
    let raw = source.path();
    let Ok(meta) = std::fs::symlink_metadata(raw) else {
        return Err(AppError::Restore(RestoreError::NotAthena));
    };
    if !meta.is_file() {
        return Err(AppError::Restore(RestoreError::NotAthena));
    }
    if meta.len() == 0 {
        return Err(AppError::Restore(RestoreError::Corrupt));
    }
    let resolved = std::fs::canonicalize(raw)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    if matches!(source, RestoreSource::Listed(_)) {
        let Ok(backups_dir) = std::fs::canonicalize(&dirs.backups_dir) else {
            return Err(AppError::Restore(RestoreError::NotAthena));
        };
        if !resolved.starts_with(&backups_dir) {
            return Err(AppError::Restore(RestoreError::NotAthena));
        }
    }
    if let Ok(live_canonical) = std::fs::canonicalize(&dirs.db_path)
        && live_canonical == resolved
    {
        return Err(AppError::Restore(RestoreError::SameAsLive));
    }
    Ok(resolved)
}

/// Step 2: Open source read-only and copy it with the `SQLite` backup API into `staging_dir`.
pub(crate) fn step_stage(source: &Path, staged_path: &Path) -> Result<()> {
    if let Some(parent) = staged_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    }
    remove_staged(staged_path);
    copy_sqlite_read_only_to_path(source, staged_path)
}

fn copy_sqlite_read_only_to_path(source: &Path, dest: &Path) -> Result<()> {
    let src = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| AppError::Restore(RestoreError::Corrupt))?;
    let mut target = Connection::open(dest)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    let backup = rusqlite::backup::Backup::new(&src, &mut target)
        .map_err(|_| AppError::Restore(RestoreError::Corrupt))?;
    if let Err(err) = backup.run_to_completion(256, Duration::ZERO, None) {
        drop(backup);
        drop(target);
        remove_staged(dest);
        return Err(map_sqlite_backup_err(&err));
    }
    Ok(())
}

/// Step 3: Validate the staged copy (integrity, version, Athena identity, `page_size`).
pub(crate) fn step_validate(staged_path: &Path, live_page_size: i64) -> Result<()> {
    let meta = backup::inspect_athena_snapshot(staged_path).map_err(AppError::Restore)?;
    if meta.page_size != live_page_size {
        return Err(AppError::Restore(RestoreError::PageSize));
    }
    Ok(())
}

/// Step 4: Upgrade the staged copy and verify domain integrity + profile presence.
pub(crate) fn step_upgrade(staged_path: &Path) -> Result<()> {
    let mut conn = Connection::open(staged_path)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    register_functions(&conn)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    migrations::run_migrations(&mut conn).map_err(|err| match err {
        AppError::SchemaTooNew { found, supported } => {
            AppError::Restore(RestoreError::TooNew { found, supported })
        }
        AppError::Migration(msg) => AppError::Restore(RestoreError::Failed(msg)),
        other @ AppError::Restore(_) => other,
        other => AppError::Restore(RestoreError::Failed(other.to_string())),
    })?;
    verify_live_db(&conn)
}

/// Step 5: Write and verify `pre-restore-<ts>.db` from the live connection.
pub(crate) fn step_snapshot(
    live_conn: &Connection,
    dirs: &AppDirs,
    now_utc: DateTime<Utc>,
) -> Result<PathBuf> {
    backup::pre_operation_snapshot(live_conn, dirs, "pre-restore-", now_utc)
        .map_err(|err| AppError::Restore(RestoreError::SnapshotFailed(err.to_string())))
}

/// Step 6: Replace the live database content from the staged copy using the `SQLite` backup API.
pub(crate) fn step_copy(live_db: &mut Db, staged_path: &Path) -> Result<()> {
    let src = Connection::open_with_flags(staged_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    {
        let backup = rusqlite::backup::Backup::new(&src, live_db.conn_mut())
            .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
        backup
            .run_to_completion(256, Duration::ZERO, None)
            .map_err(|err| map_sqlite_backup_err(&err))?;
    }
    live_db
        .conn()
        .pragma_update(None, "foreign_keys", true)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    Ok(())
}

/// Step 7: Verify live DB integrity; on any failure, roll back from the step-5 snapshot.
pub(crate) fn step_verify(live_db: &mut Db, source: &Path, snapshot_path: &Path) -> Result<()> {
    step_verify_with(live_db, source, snapshot_path, verify_live_db)
}

pub(crate) fn step_verify_with(
    live_db: &mut Db,
    source: &Path,
    snapshot_path: &Path,
    check_fn: VerifyCheckFn,
) -> Result<()> {
    if check_fn(live_db.conn()).is_ok() {
        return Ok(());
    }
    let rollback = (|| -> Result<()> {
        let snap = Connection::open_with_flags(snapshot_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        {
            let backup = rusqlite::backup::Backup::new(&snap, live_db.conn_mut())?;
            backup.run_to_completion(256, Duration::ZERO, None)?;
        }
        live_db.conn().pragma_update(None, "foreign_keys", true)?;
        Ok(())
    })();
    match rollback {
        Ok(()) => Err(AppError::Restore(RestoreError::VerificationFailed {
            source_path: source.display().to_string(),
            snapshot_path: snapshot_path.display().to_string(),
        })),
        Err(err) => Err(AppError::Restore(RestoreError::RollbackFailed {
            snapshot_path: snapshot_path.display().to_string(),
            detail: err.to_string(),
        })),
    }
}

/// Step 8: Delete the staged copy and apply `§9.1` retention while protecting `snapshot_path` and
/// `source_path`.
pub(crate) fn step_cleanup(
    dirs: &AppDirs,
    staged_path: &Path,
    snapshot_path: &Path,
    source_path: &Path,
) -> Result<()> {
    remove_staged(staged_path);
    backup::prune_protecting(&dirs.backups_dir, &[snapshot_path, source_path])?;
    Ok(())
}

/// Enforce the post-restore domain invariants on `conn`: no foreign-key violations, no integrity
/// issues, a profile row, and exactly one non-archived owner (the same owner rule the import
/// validator applies).
///
/// # Errors
/// [`AppError::Restore`] on any violated invariant.
pub(crate) fn verify_live_db(conn: &Connection) -> Result<()> {
    let fk_violations: i64 = conn
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    if fk_violations > 0 {
        return Err(AppError::Restore(RestoreError::Failed(format!(
            "foreign key check found {fk_violations} violation(s)"
        ))));
    }
    let issues = integrity::check(conn)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    if !issues.is_empty() {
        return Err(AppError::Restore(RestoreError::Invalid(issues)));
    }
    match profile_repo::load(conn) {
        Ok(Some(profile)) if !profile.onboarding_complete => return Ok(()),
        Ok(Some(_)) => {}
        Ok(None) => return Err(AppError::Restore(RestoreError::NotAthena)),
        Err(err) => return Err(AppError::Restore(RestoreError::Failed(err.to_string()))),
    }
    let (owners, archived_owners): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN archived <> 0 THEN 1 ELSE 0 END), 0)
             FROM people WHERE kind = 'owner'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    if owners != 1 {
        return Err(AppError::Restore(RestoreError::Failed(format!(
            "expected exactly one owner, found {owners}"
        ))));
    }
    if archived_owners != 0 {
        return Err(AppError::Restore(RestoreError::Failed(
            "the owner row must not be archived".into(),
        )));
    }
    Ok(())
}

fn map_sqlite_backup_err(err: &rusqlite::Error) -> AppError {
    match err {
        rusqlite::Error::SqliteFailure(e, _)
            if matches!(e.code, ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) =>
        {
            AppError::Restore(RestoreError::Corrupt)
        }
        other => AppError::Restore(RestoreError::Failed(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::services::export;
    use crate::domain::money::Money;
    use crate::domain::settings::{Language, Profile, Theme};
    use crate::domain::transaction::Allocation;
    use crate::infrastructure::db::{categories_repo, migrations, transactions_repo, trash_repo};
    use crate::shared::test_fixtures::{date, draft_with, seed_month, seed_people};

    const STAMP: &str = "2026-11-16T12:00:00Z";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(STAMP)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn seeded_file_env() -> (tempfile::TempDir, AppDirs, Db) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let db = Db::open(&dirs.db_path).unwrap();
        let profile = Profile {
            start_balance: Money::from_cents(10_000),
            theme: Theme::Dark,
            language: Language::En,
            onboarding_complete: true,
        };
        profile_repo::save(db.conn(), &profile).unwrap();
        seed_month(&db);
        (dir, dirs, db)
    }

    fn canonical_json_bytes(db: &Db) -> Vec<u8> {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("canonical.json");
        let profile = profile_repo::load(db.conn()).unwrap().unwrap();
        export::export_json(db.conn(), &profile, STAMP, &out).unwrap();
        std::fs::read(&out).unwrap()
    }

    fn make_valid_candidate(path: &Path, title: &str) {
        let db = Db::open(path).unwrap();
        profile_repo::save(
            db.conn(),
            &Profile {
                start_balance: Money::from_cents(55_000),
                theme: Theme::Light,
                language: Language::PtBr,
                onboarding_complete: true,
            },
        )
        .unwrap();
        let (owner, _) = seed_people(db.conn());
        let mut draft = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(3_300),
        }]);
        draft.title = title.into();
        draft.date = date(2026, 11, 15);
        transactions_repo::create(db.conn(), &draft).unwrap();
    }

    #[test]
    fn restore_rejects_live_path_empty_file_and_non_sqlite() {
        let (_dir, dirs, mut db) = seeded_file_env();
        let before = canonical_json_bytes(&db);

        assert!(matches!(
            restore_backup(
                &mut db,
                &dirs,
                &RestoreSource::External(dirs.db_path.clone()),
                now()
            ),
            Err(AppError::Restore(RestoreError::SameAsLive))
        ));

        let empty = dirs.root.join("empty.db");
        std::fs::write(&empty, b"").unwrap();
        assert!(matches!(
            restore_backup(&mut db, &dirs, &RestoreSource::External(empty), now()),
            Err(AppError::Restore(RestoreError::Corrupt))
        ));

        let garbage = dirs.root.join("garbage.db");
        std::fs::write(&garbage, b"this is not sqlite").unwrap();
        assert!(matches!(
            restore_backup(&mut db, &dirs, &RestoreSource::External(garbage), now()),
            Err(AppError::Restore(RestoreError::Corrupt))
        ));

        let missing = dirs.root.join("missing.db");
        assert!(matches!(
            restore_backup(&mut db, &dirs, &RestoreSource::External(missing), now()),
            Err(AppError::Restore(RestoreError::NotAthena))
        ));

        assert_eq!(canonical_json_bytes(&db), before);
        assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
    }

    #[test]
    fn restore_rejects_unrelated_sqlite_database() {
        let (_dir, dirs, mut db) = seeded_file_env();
        let before = canonical_json_bytes(&db);

        let foreign = dirs.root.join("foreign.db");
        {
            let conn = Connection::open(&foreign).unwrap();
            conn.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);")
                .unwrap();
            conn.pragma_update(None, "user_version", 1_i64).unwrap();
        }
        assert!(matches!(
            restore_backup(&mut db, &dirs, &RestoreSource::External(foreign), now()),
            Err(AppError::Restore(RestoreError::NotAthena))
        ));
        assert_eq!(canonical_json_bytes(&db), before);
        assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
    }

    #[test]
    fn restore_rejects_newer_schema() {
        let (_dir, dirs, mut db) = seeded_file_env();
        let before = canonical_json_bytes(&db);

        let newer = dirs.root.join("newer.db");
        make_valid_candidate(&newer, "Future");
        {
            let conn = Connection::open(&newer).unwrap();
            conn.pragma_update(None, "user_version", migrations::latest_version() + 1)
                .unwrap();
        }
        assert!(matches!(
            restore_backup(&mut db, &dirs, &RestoreSource::External(newer), now()),
            Err(AppError::Restore(RestoreError::TooNew { .. }))
        ));
        assert_eq!(canonical_json_bytes(&db), before);
        assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
    }

    #[test]
    fn restore_rejects_page_size_mismatch() {
        let (_dir, dirs, mut db) = seeded_file_env();
        let before = canonical_json_bytes(&db);

        let mismatched = dirs.root.join("page8k.db");
        {
            let conn = Connection::open(&mismatched).unwrap();
            conn.pragma_update(None, "page_size", 8192_i64).unwrap();
            conn.execute_batch("VACUUM;").unwrap();
        }
        make_valid_candidate(&mismatched, "OtherPage");
        let ps: i64 = Connection::open(&mismatched)
            .unwrap()
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .unwrap();
        assert_eq!(ps, 8192);

        assert!(matches!(
            restore_backup(&mut db, &dirs, &RestoreSource::External(mismatched), now()),
            Err(AppError::Restore(RestoreError::PageSize))
        ));
        assert_eq!(canonical_json_bytes(&db), before);
        assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
    }

    #[test]
    fn restore_rejects_domain_invalid_candidate() {
        let (_dir, dirs, mut db) = seeded_file_env();
        let before = canonical_json_bytes(&db);

        let bad = dirs.root.join("bad-alloc.db");
        make_valid_candidate(&bad, "Broken");
        {
            let conn = Connection::open(&bad).unwrap();
            conn.execute("UPDATE allocations SET amount_cents = amount_cents + 1", [])
                .unwrap();
        }
        assert!(matches!(
            restore_backup(&mut db, &dirs, &RestoreSource::External(bad), now()),
            Err(AppError::Restore(RestoreError::Invalid(_)))
        ));
        assert_eq!(canonical_json_bytes(&db), before);
        assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
    }

    #[test]
    fn restore_upgrades_a_v1_backup() {
        let (_dir, dirs, mut db) = seeded_file_env();
        let v1_path = dirs.root.join("backup-1.0.2.db");
        {
            let mut conn = Connection::open(&v1_path).unwrap();
            migrations::run_migrations_with(&mut conn, &migrations::MIGRATIONS[..1]).unwrap();
            conn.execute(
                "INSERT INTO profile (id, start_balance_cents, theme, language, onboarding_complete, created_at, updated_at)
                 VALUES (1, 77700, 'dark', 'en', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO people (name, kind, archived, created_at) VALUES ('Legacy', 'owner', 0, '2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            let cat_id: i64 = conn
                .query_row("SELECT id FROM categories LIMIT 1", [], |r| r.get(0))
                .unwrap();
            conn.execute(
                "INSERT INTO transactions (title, amount_cents, date, kind, category_id, payer_id, split_mode, created_at, updated_at)
                 VALUES ('Legacy row', 4500, '2026-05-01', 'expense', ?1, 1, NULL, '2026-05-01T10:00:00Z', '2026-05-01T10:00:00Z')",
                [cat_id],
            )
            .unwrap();
        }

        let receipt =
            restore_backup(&mut db, &dirs, &RestoreSource::External(v1_path), now()).unwrap();
        assert!(receipt.snapshot.exists());
        assert_eq!(
            migrations::schema_version(db.conn()).unwrap(),
            migrations::latest_version()
        );
        let profile = profile_repo::load(db.conn()).unwrap().unwrap();
        assert_eq!(profile.start_balance, Money::from_cents(77_700));
        let rows =
            transactions_repo::list(db.conn(), &crate::domain::transaction::TxFilter::default())
                .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Legacy row");
        // Verify fold() and categories still work after restore.
        assert!(!categories_repo::list(db.conn(), false).unwrap().is_empty());
    }

    #[test]
    fn restore_while_trash_nonempty_replaces_trash() {
        let (_dir, dirs, mut db) = seeded_file_env();
        let candidate = dirs.root.join("candidate.db");
        make_valid_candidate(&candidate, "Fresh");

        let live_tx =
            transactions_repo::list(db.conn(), &crate::domain::transaction::TxFilter::default())
                .unwrap()[0]
                .id;
        transactions_repo::delete(db.conn(), live_tx).unwrap();
        assert_eq!(trash_repo::count(db.conn()).unwrap(), 1);

        restore_backup(&mut db, &dirs, &RestoreSource::External(candidate), now()).unwrap();
        assert_eq!(trash_repo::count(db.conn()).unwrap(), 0);
        let rows =
            transactions_repo::list(db.conn(), &crate::domain::transaction::TxFilter::default())
                .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Fresh");
    }

    fn injected_steps(step: usize) -> RestoreSteps {
        let mut s = RestoreSteps::default();
        match step {
            1 => s.resolve = |_, _| Err(AppError::Restore(RestoreError::Failed("step 1".into()))),
            2 => s.stage = |_, _| Err(AppError::Restore(RestoreError::Failed("step 2".into()))),
            3 => s.validate = |_, _| Err(AppError::Restore(RestoreError::Failed("step 3".into()))),
            4 => s.upgrade = |_| Err(AppError::Restore(RestoreError::Failed("step 4".into()))),
            5 => {
                s.snapshot = |_, _, _| {
                    Err(AppError::Restore(RestoreError::SnapshotFailed(
                        "step 5".into(),
                    )))
                }
            }
            6 => s.copy = |_, _| Err(AppError::Restore(RestoreError::Failed("step 6".into()))),
            7 => {
                s.verify = |live, src, snap| {
                    step_verify_with(live, src, snap, |_| {
                        Err(AppError::Restore(RestoreError::Failed("step 7".into())))
                    })
                }
            }
            8 => {
                s.cleanup =
                    |_, _, _, _| Err(AppError::Restore(RestoreError::Failed("step 8".into())));
            }
            _ => unreachable!(),
        }
        s
    }

    #[test]
    fn restore_failure_injection_matrix() {
        for step in 1..=8 {
            let (_dir, dirs, mut db) = seeded_file_env();
            let before = canonical_json_bytes(&db);
            let candidate = dirs.root.join("candidate.db");
            make_valid_candidate(&candidate, "Candidate");
            let expected_candidate = {
                let cdb = Db::open_read_only(&candidate).unwrap();
                canonical_json_bytes(&cdb)
            };

            let res = restore_backup_with_steps(
                &mut db,
                &dirs,
                &RestoreSource::External(candidate),
                now(),
                injected_steps(step),
            );
            if step == 8 {
                assert!(res.is_ok(), "step 8 cleanup failure must not fail restore");
            } else {
                assert!(res.is_err(), "step {step} injection must fail");
            }

            let pre_restore_files: Vec<_> = backup::list(&dirs.backups_dir)
                .unwrap()
                .into_iter()
                .filter(|b| b.kind == backup::BackupKind::PreRestore)
                .collect();

            if step <= 5 {
                assert!(
                    pre_restore_files.is_empty(),
                    "step {step} failed before snapshot completed"
                );
            } else {
                assert_eq!(
                    pre_restore_files.len(),
                    1,
                    "step {step} completed step 5 so pre-restore snapshot must exist"
                );
            }

            if step <= 6 {
                assert_eq!(
                    canonical_json_bytes(&db),
                    before,
                    "step {step} must leave live data unchanged"
                );
                assert_eq!(
                    std::fs::read_dir(&dirs.staging_dir).unwrap().count(),
                    0,
                    "step {step} must clear staging"
                );
            } else if step == 7 {
                assert!(matches!(
                    res,
                    Err(AppError::Restore(RestoreError::VerificationFailed { .. }))
                ));
                let snap_db = Db::open_read_only(&pre_restore_files[0].path).unwrap();
                assert_eq!(
                    canonical_json_bytes(&db),
                    canonical_json_bytes(&snap_db),
                    "step 7 must roll back live data to the pre-restore snapshot"
                );
                assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
            } else {
                assert_eq!(
                    canonical_json_bytes(&db),
                    expected_candidate,
                    "step 8 must keep restored candidate live"
                );
                assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
            }
        }
    }

    #[test]
    fn restore_rejects_listed_source_outside_backups_dir() {
        let (_dir, dirs, mut db) = seeded_file_env();
        let outside = dirs.root.join("outside-candidate.db");
        make_valid_candidate(&outside, "Outside");
        let err =
            restore_backup(&mut db, &dirs, &RestoreSource::Listed(outside), now()).unwrap_err();
        assert!(matches!(err, AppError::Restore(RestoreError::NotAthena)));
    }

    #[test]
    fn restore_from_oldest_pre_restore_preserves_source_and_snapshot() {
        let (_dir, dirs, mut db) = seeded_file_env();
        let oldest = dirs.backups_dir.join("pre-restore-2026-11-01_00-00-00.db");
        make_valid_candidate(&oldest, "Oldest Source");
        for day in 2..=5_u32 {
            let p = dirs
                .backups_dir
                .join(format!("pre-restore-2026-11-{day:02}_00-00-00.db"));
            make_valid_candidate(&p, &format!("Other {day}"));
        }

        let receipt = restore_backup(
            &mut db,
            &dirs,
            &RestoreSource::Listed(oldest.clone()),
            now(),
        )
        .unwrap();
        assert!(
            oldest.exists(),
            "the restored source backup must not be pruned during step 8"
        );
        assert!(
            receipt.snapshot.exists(),
            "the new pre-restore snapshot must exist"
        );
        let rows =
            transactions_repo::list(db.conn(), &crate::domain::transaction::TxFilter::default())
                .unwrap();
        assert_eq!(rows[0].title, "Oldest Source");
    }

    #[test]
    fn verify_live_db_accepts_unfinished_setup_without_an_owner() {
        let db = Db::open_in_memory().unwrap();
        let mut profile = crate::shared::test_fixtures::profile();
        profile.onboarding_complete = false;
        profile_repo::save(db.conn(), &profile).unwrap();
        assert!(verify_live_db(db.conn()).is_ok());
    }

    #[test]
    fn verify_live_db_requires_exactly_one_active_owner() {
        let active_profile = || Profile {
            start_balance: Money::ZERO,
            theme: Theme::Dark,
            language: Language::En,
            onboarding_complete: true,
        };

        // Zero owners: rejected.
        let db = Db::open_in_memory().unwrap();
        profile_repo::save(db.conn(), &active_profile()).unwrap();
        assert!(matches!(
            verify_live_db(db.conn()),
            Err(AppError::Restore(RestoreError::Failed(ref msg)))
                if msg.contains("exactly one owner")
        ));

        // Archived owner (the shipped schema CHECK forbids it, so rebuild the table): rejected.
        let db = Db::open_in_memory().unwrap();
        profile_repo::save(db.conn(), &active_profile()).unwrap();
        db.conn()
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
        db.conn()
            .execute_batch(
                "DROP TABLE people;
                 CREATE TABLE people (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    archived INTEGER NOT NULL DEFAULT 0,
                    created_at TEXT NOT NULL
                 );
                 INSERT INTO people (id, name, kind, archived, created_at)
                 VALUES (1, 'A3', 'owner', 1, '2026-01-01T00:00:00Z');",
            )
            .unwrap();
        assert!(matches!(
            verify_live_db(db.conn()),
            Err(AppError::Restore(RestoreError::Failed(ref msg)))
                if msg.contains("must not be archived")
        ));

        // Exactly one active owner: accepted.
        let db = Db::open_in_memory().unwrap();
        profile_repo::save(db.conn(), &active_profile()).unwrap();
        let _ = seed_people(db.conn());
        assert!(verify_live_db(db.conn()).is_ok());
    }

    #[test]
    #[ignore = "50,000-row restore benchmark (< 1 s in --release)"]
    fn benchmark_restore_fifty_thousand_rows_under_one_second() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let src_path = dir.path().join("source-50k.db");
        {
            let src_db = Db::open(&src_path).unwrap();
            seed_50k_rows(&src_db);
        }
        let mut live_db = Db::open(&dirs.db_path).unwrap();
        seed_50k_rows(&live_db);

        let started = std::time::Instant::now();
        let receipt = restore_backup(
            &mut live_db,
            &dirs,
            &RestoreSource::External(src_path),
            now(),
        )
        .unwrap();
        let elapsed = started.elapsed();
        assert!(receipt.snapshot.exists());
        eprintln!("50k-row restore_backup elapsed: {elapsed:?}");
        assert!(
            elapsed < Duration::from_secs(1),
            "50k restore exceeded 1s budget: {elapsed:?}"
        );
    }

    fn seed_50k_rows(db: &Db) {
        profile_repo::save(
            db.conn(),
            &Profile {
                start_balance: Money::ZERO,
                theme: Theme::Dark,
                language: Language::En,
                onboarding_complete: true,
            },
        )
        .unwrap();
        let (owner, partner) = seed_people(db.conn());
        let cat_id: i64 = db
            .conn()
            .query_row(
                "SELECT id FROM categories WHERE applies_to = 'expense' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let tx = db.conn().unchecked_transaction().unwrap();
        {
            let mut insert_tx = tx
                .prepare(
                    "INSERT INTO transactions (title, amount_cents, date, kind, category_id, payer_id, split_mode, created_at, updated_at)
                     VALUES (?1, 1000, '2026-11-10', 'expense', ?2, ?3, 'even', '2026-11-10T12:00:00Z', '2026-11-10T12:00:00Z')",
                )
                .unwrap();
            let mut insert_alloc = tx
                .prepare(
                    "INSERT INTO allocations (transaction_id, person_id, amount_cents) VALUES (?1, ?2, 500)",
                )
                .unwrap();
            for i in 0..50_000_i64 {
                let payer = if i % 2 == 0 { owner } else { partner };
                insert_tx
                    .execute(rusqlite::params![format!("Row {i}"), cat_id, payer])
                    .unwrap();
                let id = tx.last_insert_rowid();
                insert_alloc.execute(rusqlite::params![id, owner]).unwrap();
                insert_alloc
                    .execute(rusqlite::params![id, partner])
                    .unwrap();
            }
        }
        tx.commit().unwrap();
    }
}
