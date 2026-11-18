//! Startup recovery for an unreadable live database (`§11`).
//!
//! Prepare a standalone replacement before moving the old files. Publish the replacement
//! in one move, and keep the original database and its `SQLite` sidecars together.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::{Db, backup, restore};
use crate::infrastructure::config::AppDirs;
use crate::shared::error::{AppError, RestoreError, Result};

/// Prefix of the files a recovery restore moves aside.
pub const BROKEN_PREFIX: &str = "broken-";
const SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

/// Whether the live path is absent or holds an empty regular file.
pub(crate) fn live_is_missing_or_empty(dirs: &AppDirs) -> Result<bool> {
    match std::fs::symlink_metadata(&dirs.db_path) {
        Ok(meta) => Ok(meta.is_file() && meta.len() == 0),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(true),
        Err(err) => Err(err.into()),
    }
}

/// A missing database without remnants is the normal first launch. A zero-byte database,
/// orphaned `SQLite` sidecars, moved-aside files, or recognized backups require a user choice.
pub(crate) fn needs_recovery_before_open(dirs: &AppDirs) -> Result<bool> {
    if !live_is_missing_or_empty(dirs)? {
        return Ok(false);
    }
    if dirs.db_path.exists() || !path_available(&dirs.db_path)? {
        return Ok(true);
    }
    for entry in std::fs::read_dir(&dirs.root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let base = SIDECARS
            .iter()
            .find_map(|suffix| name.strip_suffix(suffix))
            .unwrap_or(&name);
        if base.starts_with(BROKEN_PREFIX)
            && Path::new(base)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("db"))
        {
            return Ok(true);
        }
    }
    Ok(!backup::list(&dirs.backups_dir)?.is_empty())
}

/// Restore `source` over an unreadable live database. If it fails after moving files aside,
/// put those files back wherever possible and leave any unreverted originals at their saved paths.
///
/// # Errors
/// Backup validation and file or database errors preparing or installing the replacement.
pub fn restore_over_broken(
    dirs: &AppDirs,
    source: &Path,
    now_utc: DateTime<Utc>,
) -> Result<PathBuf> {
    restore_with_install(dirs, source, now_utc, install_staged)
}

fn restore_with_install(
    dirs: &AppDirs,
    source: &Path,
    now_utc: DateTime<Utc>,
    install: fn(&Path, &Path) -> Result<()>,
) -> Result<PathBuf> {
    let staged = staged_recovery_path(dirs, now_utc)?;
    let result = (|| {
        stage_and_upgrade(source, &staged)?;
        prepare_standalone(&staged)?;
        replace_live(dirs, now_utc, &staged, install)
    })();
    remove_sqlite_files(&staged);
    result
}

/// Create a new profile only after the user chooses to start fresh on an absent or empty database.
pub(crate) fn start_fresh(dirs: &AppDirs, now_utc: DateTime<Utc>) -> Result<()> {
    if !live_is_missing_or_empty(dirs)? {
        return Err(AppError::Restore(RestoreError::SameAsLive));
    }
    let staged = staged_recovery_path(dirs, now_utc)?;
    let result = (|| {
        drop(Db::open(&staged)?);
        prepare_standalone(&staged)?;
        replace_live(dirs, now_utc, &staged, install_staged)?;
        Ok(())
    })();
    remove_sqlite_files(&staged);
    result
}

fn replace_live(
    dirs: &AppDirs,
    now_utc: DateTime<Utc>,
    staged: &Path,
    install: fn(&Path, &Path) -> Result<()>,
) -> Result<PathBuf> {
    let broken = unique_broken_path(dirs, now_utc)?;
    let moved = move_broken_aside(dirs, &broken)?;
    if let Err(err) = install(staged, &dirs.db_path) {
        roll_back_moves(dirs, &moved);
        return Err(err);
    }
    Ok(broken)
}

/// Steps 1–4 of `§9.3` against the staged copy. The page-size comparison has no live database to
/// match in recovery, so only identity, integrity, version, migrations, and the domain invariants
/// are enforced.
fn stage_and_upgrade(source: &Path, staged: &Path) -> Result<()> {
    match std::fs::symlink_metadata(source) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => {}
        Ok(meta) if meta.is_file() => return Err(AppError::Restore(RestoreError::Corrupt)),
        _ => return Err(AppError::Restore(RestoreError::NotAthena)),
    }
    restore::step_stage(source, staged)?;
    let _snapshot = backup::inspect_athena_snapshot(staged).map_err(AppError::Restore)?;
    restore::step_upgrade(staged)
}

/// Close `SQLite` with a standalone database, then flush the file before publishing it.
fn prepare_standalone(staged: &Path) -> Result<()> {
    let conn = rusqlite::Connection::open(staged)?;
    let mode: String = conn.query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("delete") {
        return Err(AppError::Restore(RestoreError::Failed(
            "could not make the staged database standalone".into(),
        )));
    }
    drop(conn);
    std::fs::OpenOptions::new()
        .write(true)
        .open(staged)?
        .sync_all()?;
    Ok(())
}

/// Publish a complete database without overwriting a file that appeared at the destination.
fn install_staged(staged: &Path, live: &Path) -> Result<()> {
    tempfile::TempPath::try_from_path(staged)?
        .persist_noclobber(live)
        .map_err(|err| err.error)?;
    Ok(())
}

fn move_broken_aside(dirs: &AppDirs, broken: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut files = Vec::new();
    for suffix in std::iter::once("").chain(SIDECARS) {
        let live = with_suffix(&dirs.db_path, suffix);
        match std::fs::symlink_metadata(&live) {
            Ok(_) => files.push((live, with_suffix(broken, suffix))),
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    let mut moved = Vec::new();
    for (live, destination) in files {
        if let Err(err) = std::fs::rename(&live, &destination) {
            roll_back_moves(dirs, &moved);
            return Err(err.into());
        }
        moved.push((live, destination));
    }
    Ok(moved)
}

fn roll_back_moves(dirs: &AppDirs, moved: &[(PathBuf, PathBuf)]) {
    // The live database was moved first. If it has since reappeared, keep its old sidecars
    // alongside the moved copy rather than attaching them to the unexpected live file.
    if !matches!(std::fs::symlink_metadata(&dirs.db_path), Err(err) if err.kind() == ErrorKind::NotFound)
    {
        return;
    }
    for (original, saved) in moved.iter().rev() {
        if let Err(err) = tempfile::TempPath::try_from_path(saved).and_then(|path| {
            path.persist_noclobber(original).map_err(|err| {
                // Retain the saved database even if the live path became occupied.
                let _ = err.path.keep();
                err.error
            })
        }) {
            tracing::error!(path = %saved.display(), "could not roll back broken-file move: {err}");
            return;
        }
    }
}

fn staged_recovery_path(dirs: &AppDirs, now_utc: DateTime<Utc>) -> Result<PathBuf> {
    unique_path(
        &dirs.staging_dir,
        &format!("recovery-{}", now_utc.format("%Y-%m-%d_%H-%M-%S")),
    )
}

fn unique_broken_path(dirs: &AppDirs, now_utc: DateTime<Utc>) -> Result<PathBuf> {
    unique_path(
        &dirs.root,
        &format!("{BROKEN_PREFIX}{}", now_utc.format("%Y-%m-%d_%H-%M-%S")),
    )
}

/// `<dir>/<stem>.db`, or `<dir>/<stem>-<n>.db` when that already exists.
fn unique_path(dir: &Path, stem: &str) -> Result<PathBuf> {
    let first = dir.join(format!("{stem}.db"));
    if path_available(&first)? {
        return Ok(first);
    }
    for n in 1_u32.. {
        let candidate = dir.join(format!("{stem}-{n}.db"));
        if path_available(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn path_available(path: &Path) -> Result<bool> {
    for suffix in std::iter::once("").chain(SIDECARS) {
        match std::fs::symlink_metadata(with_suffix(path, suffix)) {
            Ok(_) => return Ok(false),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(true)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    if suffix.is_empty() {
        return path.to_path_buf();
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return path.to_path_buf();
    };
    path.with_file_name(format!("{name}{suffix}"))
}

fn remove_sqlite_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in SIDECARS {
        let _ = std::fs::remove_file(with_suffix(path, suffix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::domain::settings::{Language, Profile, Theme};
    use crate::infrastructure::db::profile_repo;
    use crate::shared::test_fixtures::{seed_month, seed_people};

    const STAMP: &str = "2026-11-16T12:00:00Z";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(STAMP)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// A temp root with a valid Athena backup at `backups/manual-good.db` and garbage bytes at
    /// the live path.
    fn broken_env() -> (tempfile::TempDir, AppDirs, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();

        let source = dirs.backups_dir.join("manual-good.db");
        {
            let db = Db::open(&source).unwrap();
            profile_repo::save(
                db.conn(),
                &Profile {
                    start_balance: Money::from_cents(10_000),
                    theme: Theme::Dark,
                    language: Language::En,
                    onboarding_complete: true,
                },
            )
            .unwrap();
            let (_, _) = seed_people(db.conn());
            seed_month(&db);
        }
        std::fs::write(&dirs.db_path, b"this is not a database").unwrap();
        std::fs::write(with_suffix(&dirs.db_path, "-wal"), b"stale wal").unwrap();
        std::fs::write(with_suffix(&dirs.db_path, "-shm"), b"stale shm").unwrap();
        (dir, dirs, source)
    }

    fn broken_files(dirs: &AppDirs) -> Vec<PathBuf> {
        std::fs::read_dir(&dirs.root)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(BROKEN_PREFIX))
            })
            .collect()
    }

    #[test]
    fn recovery_preserves_orphan_broken_sidecars_on_name_collision() {
        let (_dir, dirs, source) = broken_env();
        let orphan = dirs.root.join("broken-2026-11-16_12-00-00.db-wal");
        std::fs::write(&orphan, b"previous evidence").unwrap();
        let saved = restore_over_broken(&dirs, &source, now()).unwrap();
        assert_eq!(
            saved.file_name().unwrap(),
            "broken-2026-11-16_12-00-00-1.db"
        );
        assert_eq!(std::fs::read(orphan).unwrap(), b"previous evidence");
    }

    #[test]
    fn recovery_restore_moves_broken_files_aside_and_opens_the_live_path() {
        let (_dir, dirs, source) = broken_env();

        let broken = restore_over_broken(&dirs, &source, now()).unwrap();

        assert_eq!(
            broken.file_name().and_then(|n| n.to_str()),
            Some("broken-2026-11-16_12-00-00.db")
        );
        assert_eq!(std::fs::read(&broken).unwrap(), b"this is not a database");
        assert!(with_suffix(&broken, "-wal").is_file());
        assert!(with_suffix(&broken, "-shm").is_file());
        assert!(!with_suffix(&dirs.db_path, "-wal").exists());
        assert!(!with_suffix(&dirs.db_path, "-shm").exists());

        let db = Db::open(&dirs.db_path).unwrap();
        assert_eq!(
            profile_repo::load(db.conn())
                .unwrap()
                .unwrap()
                .start_balance,
            Money::from_cents(10_000)
        );
        assert!(
            std::fs::read_dir(&dirs.staging_dir)
                .unwrap()
                .next()
                .is_none(),
            "the staged copy must be removed"
        );
    }

    #[test]
    fn recovery_restore_failure_stays_in_recovery_with_files_intact() {
        let (_dir, dirs, _source) = broken_env();
        let garbage = dirs.backups_dir.join("manual-garbage.db");
        std::fs::write(&garbage, b"also not a database").unwrap();

        let result = restore_over_broken(&dirs, &garbage, now());

        assert!(matches!(result, Err(AppError::Restore(_))), "{result:?}");
        assert_eq!(
            std::fs::read(&dirs.db_path).unwrap(),
            b"this is not a database"
        );
        assert!(with_suffix(&dirs.db_path, "-wal").is_file());
        assert!(broken_files(&dirs).is_empty(), "nothing moved aside");
        assert!(
            std::fs::read_dir(&dirs.staging_dir)
                .unwrap()
                .next()
                .is_none(),
            "the rejected staged copy must be removed"
        );
    }

    #[test]
    fn a_second_recovery_restore_uses_a_fresh_broken_name() {
        let (_dir, dirs, source) = broken_env();
        restore_over_broken(&dirs, &source, now()).unwrap();
        std::fs::write(&dirs.db_path, b"broken again").unwrap();

        let second = restore_over_broken(&dirs, &source, now()).unwrap();

        assert_eq!(
            second.file_name().and_then(|n| n.to_str()),
            Some("broken-2026-11-16_12-00-00-1.db")
        );
        assert!(Db::open(&dirs.db_path).is_ok());
    }
    #[test]
    fn failed_install_puts_back_the_original_database_and_sidecars() {
        let (_dir, dirs, source) = broken_env();
        let journal = with_suffix(&dirs.db_path, "-journal");
        std::fs::write(&journal, b"original journal").unwrap();
        let source_before = std::fs::read(&source).unwrap();

        let error = restore_with_install(&dirs, &source, now(), |candidate, live| {
            assert!(!live.exists(), "failure occurs after the original moves");
            assert!(backup::is_valid_athena_snapshot(candidate).unwrap());
            Err(std::io::Error::other("simulated publish failure").into())
        });
        assert!(error.is_err());
        assert_eq!(
            std::fs::read(&dirs.db_path).unwrap(),
            b"this is not a database"
        );
        assert_eq!(
            std::fs::read(with_suffix(&dirs.db_path, "-wal")).unwrap(),
            b"stale wal"
        );
        assert_eq!(
            std::fs::read(with_suffix(&dirs.db_path, "-shm")).unwrap(),
            b"stale shm"
        );
        assert_eq!(std::fs::read(journal).unwrap(), b"original journal");
        assert_eq!(std::fs::read(source).unwrap(), source_before);
        assert!(broken_files(&dirs).is_empty());
        assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
        assert!(matches!(
            crate::open_session(dirs).unwrap(),
            crate::AppSession::Recovery(_)
        ));
    }

    #[test]
    fn fresh_start_refuses_to_replace_a_nonempty_live_database() {
        let (_dir, dirs, _source) = broken_env();
        let original = std::fs::read(&dirs.db_path).unwrap();
        assert!(start_fresh(&dirs, now()).is_err());
        assert_eq!(std::fs::read(&dirs.db_path).unwrap(), original);
        assert!(broken_files(&dirs).is_empty());
    }

    #[test]
    fn recovery_keeps_the_original_rollback_journal_with_its_database() {
        let (_dir, dirs, source) = broken_env();
        let journal = with_suffix(&dirs.db_path, "-journal");
        std::fs::write(&journal, b"original journal").unwrap();
        let broken = restore_over_broken(&dirs, &source, now()).unwrap();
        assert!(!journal.exists());
        assert_eq!(
            std::fs::read(with_suffix(&broken, "-journal")).unwrap(),
            b"original journal"
        );
    }
}
