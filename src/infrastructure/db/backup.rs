//! `VACUUM INTO` snapshots. Never raw-copies a live WAL database.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use rusqlite::{Connection, OpenFlags};

use super::migrations;
use crate::infrastructure::config::AppDirs;
use crate::shared::error::{AppError, RestoreError, Result};

/// How many `auto-` snapshots are kept.
pub const AUTO_SNAPSHOTS_KEPT: usize = 7;
/// How many `pre-purge-` / `pre-restore-` / `pre-import-` snapshots are kept.
pub const PRE_OPERATION_SNAPSHOTS_KEPT: usize = 5;
/// SQLite `PRAGMA application_id` set on Athena databases from schema v2 onward (`"ATNA"`).
pub const ATHENA_APPLICATION_ID: i64 = 0x4154_4E41;

/// Which category of snapshot a managed backup file belongs to (`§9.1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackupKind {
    Auto,
    Manual,
    PrePurge,
    PreRestore,
    PreImport,
    PreMigration,
}

/// Metadata for a managed `.db` snapshot listed from `backups_dir` (`§9.1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupFile {
    pub path: PathBuf,
    pub kind: BackupKind,
    pub created_utc: DateTime<Utc>,
    pub bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SnapshotMeta {
    pub version: i64,
    pub page_size: i64,
}

const V1_TABLES: [&str; 5] = [
    "allocations",
    "categories",
    "people",
    "profile",
    "transactions",
];

/// Run `PRAGMA integrity_check` on the database file at `path` (opened read-only).
///
/// # Errors
/// Database errors opening or querying the file.
pub fn integrity_ok(path: &Path) -> Result<bool> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare("PRAGMA integrity_check")?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.len() == 1 && rows[0] == "ok")
}

/// Inspect `path` as an Athena SQLite snapshot, distinguishing `Corrupt`, `TooNew`, and
/// `NotAthena` for the restore pipeline (`§9.3`).
pub(crate) fn inspect_athena_snapshot(
    path: &Path,
) -> std::result::Result<SnapshotMeta, RestoreError> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Err(RestoreError::NotAthena);
    };
    if !meta.is_file() {
        return Err(RestoreError::NotAthena);
    }
    if meta.len() == 0 {
        return Err(RestoreError::Corrupt);
    }
    let Ok(conn) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return Err(RestoreError::Corrupt);
    };
    let Ok(mut integrity_stmt) = conn.prepare("PRAGMA integrity_check") else {
        return Err(RestoreError::Corrupt);
    };
    let Ok(integrity_rows) = integrity_stmt
        .query_map([], |r| r.get::<_, String>(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
    else {
        return Err(RestoreError::Corrupt);
    };
    if integrity_rows.len() != 1 || integrity_rows[0] != "ok" {
        return Err(RestoreError::Corrupt);
    }
    let Ok(version) = migrations::schema_version(&conn) else {
        return Err(RestoreError::Corrupt);
    };
    let supported = migrations::latest_version();
    if version > supported {
        return Err(RestoreError::TooNew {
            found: version,
            supported,
        });
    }
    if version < 1 {
        return Err(RestoreError::NotAthena);
    }
    let Ok(app_id) = conn.pragma_query_value(None, "application_id", |r| r.get::<_, i64>(0)) else {
        return Err(RestoreError::Corrupt);
    };
    let Ok(page_size) = conn.pragma_query_value(None, "page_size", |r| r.get::<_, i64>(0)) else {
        return Err(RestoreError::Corrupt);
    };
    let Ok(mut table_stmt) = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    ) else {
        return Err(RestoreError::Corrupt);
    };
    let Ok(tables) = table_stmt
        .query_map([], |r| r.get::<_, String>(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
    else {
        return Err(RestoreError::Corrupt);
    };

    if version == 1 {
        let exact_v1 = tables.iter().map(String::as_str).eq(V1_TABLES);
        return if exact_v1 && (app_id == 0 || app_id == ATHENA_APPLICATION_ID) {
            Ok(SnapshotMeta { version, page_size })
        } else {
            Err(RestoreError::NotAthena)
        };
    }

    if app_id != ATHENA_APPLICATION_ID {
        return Err(RestoreError::NotAthena);
    }
    if !V1_TABLES
        .iter()
        .all(|required| tables.iter().any(|t| t == required))
    {
        return Err(RestoreError::NotAthena);
    }
    if version >= 3 {
        let Ok(has_live_view) = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'view' AND name = 'live_transactions')",
            [],
            |r| r.get::<_, bool>(0),
        ) else {
            return Err(RestoreError::Corrupt);
        };
        if !has_live_view {
            return Err(RestoreError::NotAthena);
        }
    }
    Ok(SnapshotMeta { version, page_size })
}

/// Verify that `path` is a regular, non-empty, integrity-clean SQLite file belonging to Athena
/// with a supported schema version (`1..=latest_version()`). Returns `Ok(false)` (never panics)
/// when the path is missing, a directory/symlink, empty, corrupt, foreign, or too new.
///
/// # Errors
/// Reserved for unexpected non-SQLite I/O failures; malformed files return `Ok(false)`.
pub fn is_valid_athena_snapshot(path: &Path) -> Result<bool> {
    Ok(inspect_athena_snapshot(path).is_ok())
}

/// Write a consistent copy of the open database to `dest`.
///
/// The destination is bound as a parameter (never formatted into SQL). The copy is built under a
/// temporary name, integrity-checked, then renamed, so a failure never leaves a half-written file.
///
/// # Errors
/// [`AppError::Backup`] when the copy fails its integrity check or the path is not valid text;
/// database and I/O errors otherwise.
pub fn vacuum_into(conn: &Connection, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file_name = dest
        .file_name()
        .ok_or_else(|| AppError::Backup("destination has no file name".into()))?
        .to_string_lossy()
        .into_owned();
    let tmp = dest.with_file_name(format!("{file_name}.tmp"));
    let tmp_text = tmp
        .to_str()
        .ok_or_else(|| AppError::Backup("destination path is not valid text".into()))?;
    if tmp.exists() {
        std::fs::remove_file(&tmp)?;
    }
    if let Err(err) = conn.execute("VACUUM INTO ?1", [tmp_text]) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    let verified = integrity_ok(&tmp);
    match verified {
        Ok(true) => {}
        Ok(false) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(AppError::Backup(
                "the copy failed its integrity check".into(),
            ));
        }
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(err);
        }
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Write `backups/auto-YYYY-MM-DD.db` if today's does not exist yet, then keep only the newest
/// [`AUTO_SNAPSHOTS_KEPT`] `auto-` files. If a path already exists at today's snapshot name, it
/// must pass [`is_valid_athena_snapshot`] (failing closed without overwriting it otherwise).
/// Returns the new path, or `None` when today's verified snapshot already exists.
///
/// # Errors
/// See [`vacuum_into`]; [`AppError::Backup`] when an existing daily snapshot fails verification;
/// I/O errors listing or pruning the folder.
pub fn daily_snapshot(
    conn: &Connection,
    dirs: &AppDirs,
    today: NaiveDate,
) -> Result<Option<PathBuf>> {
    std::fs::create_dir_all(&dirs.backups_dir)?;
    let path = dirs
        .backups_dir
        .join(format!("auto-{}.db", today.format("%Y-%m-%d")));
    if std::fs::symlink_metadata(&path).is_ok() {
        if is_valid_athena_snapshot(&path)? {
            return Ok(None);
        }
        return Err(AppError::Backup(format!(
            "existing daily snapshot {} failed verification",
            path.display()
        )));
    }
    vacuum_into(conn, &path)?;
    if !is_valid_athena_snapshot(&path)? {
        let _ = std::fs::remove_file(&path);
        return Err(AppError::Backup(
            "newly written daily snapshot failed Athena verification".into(),
        ));
    }
    prune_prefix_protected(&dirs.backups_dir, "auto-", AUTO_SNAPSHOTS_KEPT, Some(&path))?;
    Ok(Some(path))
}

/// Write and verify a `manual-YYYY-MM-DD_HH-MM-SS.db` snapshot of `conn` (`§9.2`). Manual
/// backups are never auto-pruned.
///
/// # Errors
/// [`AppError::Backup`], SQLite, or I/O errors if the snapshot cannot be written or verified.
pub fn create_manual(
    conn: &Connection,
    dirs: &AppDirs,
    now_utc: DateTime<Utc>,
) -> Result<BackupFile> {
    std::fs::create_dir_all(&dirs.backups_dir)?;
    let stem = format!("manual-{}", now_utc.format("%Y-%m-%d_%H-%M-%S"));
    let path = unique_backup_path(&dirs.backups_dir, &stem);
    vacuum_into(conn, &path)?;
    if !is_valid_athena_snapshot(&path)? {
        let _ = std::fs::remove_file(&path);
        return Err(AppError::Backup(
            "manual backup failed Athena verification".into(),
        ));
    }
    let bytes = std::fs::metadata(&path)?.len();
    Ok(BackupFile {
        path,
        kind: BackupKind::Manual,
        created_utc: now_utc,
        bytes,
    })
}

/// Write and verify a fresh `pre-purge-YYYY-MM-DD_HH-MM-SS.db` snapshot of `conn` before automatic
/// trash retention purges expired rows, then prune managed prefixes while protecting the newly
/// created snapshot.
///
/// # Errors
/// [`AppError::Backup`], SQLite, or I/O errors if the snapshot cannot be written or verified.
pub fn pre_purge_snapshot(
    conn: &Connection,
    dirs: &AppDirs,
    now_utc: DateTime<Utc>,
) -> Result<PathBuf> {
    let path = pre_operation_snapshot(conn, dirs, "pre-purge-", now_utc)?;
    prune(&dirs.backups_dir, Some(&path))?;
    Ok(path)
}

/// Write and verify a fresh `pre-import-YYYY-MM-DD_HH-MM-SS.db` snapshot of `conn` before an
/// import replaces the database. Call [`prune`] after the replacement succeeds so a failed import
/// preserves older snapshots.
///
/// # Errors
/// [`AppError::Backup`], SQLite, or I/O errors if the snapshot cannot be written or verified.
pub fn pre_import_snapshot(
    conn: &Connection,
    dirs: &AppDirs,
    now_utc: DateTime<Utc>,
) -> Result<PathBuf> {
    pre_operation_snapshot(conn, dirs, "pre-import-", now_utc)
}

pub(crate) fn pre_operation_snapshot(
    conn: &Connection,
    dirs: &AppDirs,
    prefix: &str,
    now_utc: DateTime<Utc>,
) -> Result<PathBuf> {
    std::fs::create_dir_all(&dirs.backups_dir)?;
    let stem = format!("{prefix}{}", now_utc.format("%Y-%m-%d_%H-%M-%S"));
    let path = unique_backup_path(&dirs.backups_dir, &stem);
    vacuum_into(conn, &path)?;
    if !is_valid_athena_snapshot(&path)? {
        let _ = std::fs::remove_file(&path);
        return Err(AppError::Backup(format!(
            "{prefix} snapshot failed Athena verification"
        )));
    }
    Ok(path)
}

fn unique_backup_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.db"));
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    for n in 1_u32.. {
        let candidate = dir.join(format!("{stem}-{n}.db"));
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    unreachable!()
}

/// List managed `.db` snapshots in `dir`, newest first (`§9.1`). Ignores non-regular files,
/// `.tmp` files, non-`.db` files, and unknown prefixes.
///
/// # Errors
/// I/O errors reading `dir` (a missing `dir` returns an empty list).
pub fn list(dir: &Path) -> Result<Vec<BackupFile>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some((kind, created_utc)) = parse_backup_name(name) else {
            continue;
        };
        files.push(BackupFile {
            path,
            kind,
            created_utc,
            bytes: meta.len(),
        });
    }
    files.sort_by(|a, b| {
        b.created_utc
            .cmp(&a.created_utc)
            .then_with(|| b.path.cmp(&a.path))
    });
    Ok(files)
}

fn parse_backup_name(name: &str) -> Option<(BackupKind, DateTime<Utc>)> {
    let stem = name
        .strip_suffix(".db")
        .or_else(|| name.strip_suffix(".DB"))?;
    if let Some(date_str) = stem.strip_prefix("auto-") {
        let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d").ok()?;
        let dt = date.and_hms_opt(0, 0, 0)?.and_utc();
        return Some((BackupKind::Auto, dt));
    }
    if let Some(rest) = stem.strip_prefix("manual-") {
        return Some((BackupKind::Manual, parse_timestamp_with_suffix(rest)?));
    }
    if let Some(rest) = stem.strip_prefix("pre-purge-") {
        return Some((BackupKind::PrePurge, parse_timestamp_with_suffix(rest)?));
    }
    if let Some(rest) = stem.strip_prefix("pre-restore-") {
        return Some((BackupKind::PreRestore, parse_timestamp_with_suffix(rest)?));
    }
    if let Some(rest) = stem.strip_prefix("pre-import-") {
        return Some((BackupKind::PreImport, parse_timestamp_with_suffix(rest)?));
    }
    if let Some(after_v) = stem.strip_prefix("pre-migration-v") {
        let (ver, ts) = after_v.split_once('-')?;
        if ver.is_empty() || !ver.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return Some((BackupKind::PreMigration, parse_timestamp_with_suffix(ts)?));
    }
    None
}

fn parse_timestamp_with_suffix(rest: &str) -> Option<DateTime<Utc>> {
    let ts = if rest.len() == 19 {
        rest
    } else if rest.len() > 20
        && rest.as_bytes().get(19) == Some(&b'-')
        && rest[20..].chars().all(|c| c.is_ascii_digit())
    {
        &rest[..19]
    } else {
        return None;
    };
    NaiveDateTime::parse_from_str(ts, "%Y-%m-%d_%H-%M-%S")
        .ok()
        .map(|ndt| ndt.and_utc())
}

/// Apply `§9.1` retention across all managed prefixes in `dir`: keep newest 7 `auto-`, newest 5
/// `pre-purge-` / `pre-restore-` / `pre-import-`, never auto-prune `manual-` or `pre-migration-`,
/// and never delete `protected` or unrecognized files.
///
/// # Errors
/// I/O errors.
pub fn prune(dir: &Path, protected: Option<&Path>) -> Result<usize> {
    let slice: Vec<&Path> = protected.into_iter().collect();
    prune_protecting(dir, &slice)
}

/// Apply `§9.1` retention across all managed prefixes in `dir`, never deleting any path in
/// `protected` or unrecognized files.
///
/// # Errors
/// I/O errors.
pub fn prune_protecting(dir: &Path, protected: &[&Path]) -> Result<usize> {
    let mut removed = 0;
    removed += prune_prefix_protecting(dir, "auto-", AUTO_SNAPSHOTS_KEPT, protected)?;
    for prefix in ["pre-purge-", "pre-restore-", "pre-import-"] {
        removed += prune_prefix_protecting(dir, prefix, PRE_OPERATION_SNAPSHOTS_KEPT, protected)?;
    }
    Ok(removed)
}

/// Delete everything inside `dirs.staging_dir` without touching `backups_dir` or any other folder.
///
/// # Errors
/// I/O errors removing staged entries.
pub fn clear_staging(dirs: &AppDirs) -> Result<()> {
    let entries = match std::fs::read_dir(&dirs.staging_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// Delete the oldest `.db` files whose name starts with `prefix`, keeping `keep` (names sort by date).
///
/// # Errors
/// I/O errors.
pub fn prune_prefix(dir: &Path, prefix: &str, keep: usize) -> Result<usize> {
    prune_prefix_protecting(dir, prefix, keep, &[])
}

/// Delete the oldest `.db` files whose name starts with `prefix`, keeping `keep`, while never
/// deleting `protected`.
///
/// # Errors
/// I/O errors.
pub fn prune_prefix_protected(
    dir: &Path,
    prefix: &str,
    keep: usize,
    protected: Option<&Path>,
) -> Result<usize> {
    let slice: Vec<&Path> = protected.into_iter().collect();
    prune_prefix_protecting(dir, prefix, keep, &slice)
}

/// Delete the oldest `.db` files whose name starts with `prefix`, keeping `keep`, while never
/// deleting any path in `protected`.
///
/// # Errors
/// I/O errors.
pub fn prune_prefix_protecting(
    dir: &Path,
    prefix: &str,
    keep: usize,
    protected: &[&Path],
) -> Result<usize> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            n.starts_with(prefix)
                && Path::new(n)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("db"))
        })
        .collect();
    names.sort();
    let excess = names.len().saturating_sub(keep);
    let mut removed = 0;
    for name in &names {
        if removed >= excess {
            break;
        }
        let candidate = dir.join(name);
        if is_protected_path(&candidate, protected) {
            continue;
        }
        std::fs::remove_file(&candidate)?;
        removed += 1;
    }
    Ok(removed)
}

fn is_protected_path(candidate: &Path, protected: &[&Path]) -> bool {
    let candidate_canon = std::fs::canonicalize(candidate).ok();
    protected.iter().any(|p| {
        if candidate == *p {
            return true;
        }
        match (&candidate_canon, std::fs::canonicalize(p).ok()) {
            (Some(a), Some(b)) => a == &b,
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::Db;

    #[test]
    fn daily_snapshot_is_once_per_day_and_keeps_seven() {
        let db = Db::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        std::fs::write(dirs.backups_dir.join("athena-manual-keep.db"), b"x").unwrap();
        for d in 1..=9 {
            let day = NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
            assert!(daily_snapshot(db.conn(), &dirs, day).unwrap().is_some());
        }
        let again = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        assert!(daily_snapshot(db.conn(), &dirs, again).unwrap().is_none());
        let autos = std::fs::read_dir(&dirs.backups_dir)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("auto-"))
            .count();
        assert_eq!(autos, 7);
        assert!(dirs.backups_dir.join("athena-manual-keep.db").exists());
        assert!(!dirs.backups_dir.join("auto-2026-11-02.db").exists());
        assert!(dirs.backups_dir.join("auto-2026-11-03.db").exists());
    }

    #[test]
    fn vacuum_into_handles_paths_with_quotes_and_spaces() {
        let db = Db::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("it's a backup.db");
        vacuum_into(db.conn(), &dest).unwrap();
        assert!(dest.exists());
        assert!(integrity_ok(&dest).unwrap());
        assert!(!dir.path().join("it's a backup.db.tmp").exists());
    }

    #[test]
    fn snapshot_contains_the_seeded_data() {
        let db = Db::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("copy.db");
        vacuum_into(db.conn(), &dest).unwrap();
        let copy = Db::open_read_only(&dest).unwrap();
        let n: i64 = copy
            .conn()
            .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 12);
    }

    #[test]
    fn verify_athena_snapshot_rejects_empty_corrupt_dir_and_foreign_sqlite() {
        let dir = tempfile::tempdir().unwrap();

        // 1. Directory at path
        let as_dir = dir.path().join("dir.db");
        std::fs::create_dir_all(&as_dir).unwrap();
        assert!(!is_valid_athena_snapshot(&as_dir).unwrap_or(false));

        // 2. Empty file
        let empty = dir.path().join("empty.db");
        std::fs::write(&empty, b"").unwrap();
        assert!(!is_valid_athena_snapshot(&empty).unwrap_or(false));

        // 3. Corrupt garbage
        let corrupt = dir.path().join("corrupt.db");
        std::fs::write(&corrupt, b"not sqlite at all").unwrap();
        assert!(!is_valid_athena_snapshot(&corrupt).unwrap_or(false));

        // 4. Valid non-Athena SQLite database (with user_version = 1 and even matching application_id)
        let foreign = dir.path().join("foreign.db");
        {
            let conn = Connection::open(&foreign).unwrap();
            conn.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);")
                .unwrap();
            conn.pragma_update(None, "user_version", 1_i64).unwrap();
        }
        assert!(!is_valid_athena_snapshot(&foreign).unwrap_or(false));

        let foreign_app_id = dir.path().join("foreign_app_id.db");
        {
            let conn = Connection::open(&foreign_app_id).unwrap();
            conn.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);")
                .unwrap();
            conn.pragma_update(None, "user_version", 3_i64).unwrap();
            conn.pragma_update(None, "application_id", ATHENA_APPLICATION_ID)
                .unwrap();
        }
        assert!(!is_valid_athena_snapshot(&foreign_app_id).unwrap_or(false));

        // 5. Too-new schema version
        let too_new = dir.path().join("too_new.db");
        {
            let mut conn = Connection::open(&too_new).unwrap();
            migrations::run_migrations(&mut conn).unwrap();
            conn.pragma_update(None, "user_version", 99_i64).unwrap();
        }
        assert!(!is_valid_athena_snapshot(&too_new).unwrap_or(false));

        // 6. Valid Athena v1 database (without application_id set)
        let v1 = dir.path().join("v1.db");
        {
            let mut conn = Connection::open(&v1).unwrap();
            crate::infrastructure::db::run_migrations_with(
                &mut conn,
                &crate::infrastructure::db::MIGRATIONS[..1],
            )
            .unwrap();
        }
        assert!(is_valid_athena_snapshot(&v1).unwrap());

        // 7. Valid Athena latest database
        let latest = dir.path().join("latest.db");
        {
            let mut conn = Connection::open(&latest).unwrap();
            migrations::run_migrations(&mut conn).unwrap();
        }
        assert!(is_valid_athena_snapshot(&latest).unwrap());
    }

    #[test]
    fn manual_backup_is_verified_and_named() {
        let db = Db::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T14:03:05Z")
            .unwrap()
            .with_timezone(&Utc);

        let created = create_manual(db.conn(), &dirs, now_utc).unwrap();
        assert_eq!(created.kind, BackupKind::Manual);
        assert_eq!(created.created_utc, now_utc);
        assert_eq!(
            created.path.file_name().unwrap().to_string_lossy(),
            "manual-2026-11-16_14-03-05.db"
        );
        assert!(is_valid_athena_snapshot(&created.path).unwrap());
        assert!(created.bytes > 0);
        assert_eq!(
            created.bytes,
            std::fs::metadata(&created.path).unwrap().len()
        );
    }

    #[test]
    fn listing_recognizes_every_prefix_and_ignores_others() {
        let dir = tempfile::tempdir().unwrap();
        let bdir = dir.path().join("backups");
        std::fs::create_dir_all(&bdir).unwrap();

        let valid = [
            ("auto-2026-11-10.db", BackupKind::Auto),
            ("manual-2026-11-11_09-00-00.db", BackupKind::Manual),
            ("pre-purge-2026-11-12_10-00-00.db", BackupKind::PrePurge),
            ("pre-restore-2026-11-13_11-00-00.db", BackupKind::PreRestore),
            ("pre-import-2026-11-14_12-00-00.db", BackupKind::PreImport),
            (
                "pre-migration-v1-2026-11-15_13-00-00.db",
                BackupKind::PreMigration,
            ),
        ];
        for (name, _) in valid {
            std::fs::write(bdir.join(name), name.as_bytes()).unwrap();
        }
        // Ignored entries: unknown prefixes, .tmp files, non-.db files, directories
        for ignored in [
            "athena-manual-keep.db",
            "unknown-2026-11-16.db",
            "manual-2026-11-16_00-00-00.db.tmp",
            "auto-2026-11-16.txt",
            "manual-not-a-date.db",
        ] {
            std::fs::write(bdir.join(ignored), b"ignored").unwrap();
        }
        std::fs::create_dir_all(bdir.join("auto-2026-11-16.db")).unwrap();

        let listed = list(&bdir).unwrap();
        let kinds: Vec<(String, BackupKind)> = listed
            .iter()
            .map(|b| {
                (
                    b.path.file_name().unwrap().to_string_lossy().into_owned(),
                    b.kind,
                )
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                (
                    "pre-migration-v1-2026-11-15_13-00-00.db".into(),
                    BackupKind::PreMigration
                ),
                (
                    "pre-import-2026-11-14_12-00-00.db".into(),
                    BackupKind::PreImport
                ),
                (
                    "pre-restore-2026-11-13_11-00-00.db".into(),
                    BackupKind::PreRestore
                ),
                (
                    "pre-purge-2026-11-12_10-00-00.db".into(),
                    BackupKind::PrePurge
                ),
                ("manual-2026-11-11_09-00-00.db".into(), BackupKind::Manual),
                ("auto-2026-11-10.db".into(), BackupKind::Auto),
            ]
        );
    }

    #[test]
    fn retention_keeps_newest_five_pre_files_and_never_prunes_manual_or_pre_migration() {
        let dir = tempfile::tempdir().unwrap();
        let bdir = dir.path();

        for i in 1..=9 {
            std::fs::write(bdir.join(format!("auto-2026-11-0{i}.db")), b"x").unwrap();
        }
        for prefix in ["pre-purge-", "pre-restore-", "pre-import-"] {
            for i in 1..=7 {
                std::fs::write(bdir.join(format!("{prefix}2026-11-0{i}_10-00-00.db")), b"x")
                    .unwrap();
            }
        }
        for i in 1..=8 {
            std::fs::write(bdir.join(format!("manual-2026-11-0{i}_10-00-00.db")), b"x").unwrap();
            std::fs::write(
                bdir.join(format!("pre-migration-v1-2026-11-0{i}_10-00-00.db")),
                b"x",
            )
            .unwrap();
        }
        std::fs::write(bdir.join("unrelated.db"), b"x").unwrap();

        prune(bdir, None).unwrap();

        let count = |p: &str| {
            std::fs::read_dir(bdir)
                .unwrap()
                .filter_map(std::result::Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with(p))
                .count()
        };
        assert_eq!(count("auto-"), 7);
        assert_eq!(count("pre-purge-"), 5);
        assert_eq!(count("pre-restore-"), 5);
        assert_eq!(count("pre-import-"), 5);
        assert_eq!(count("manual-"), 8);
        assert_eq!(count("pre-migration-"), 8);
        assert!(bdir.join("unrelated.db").exists());
        assert!(!bdir.join("pre-restore-2026-11-02_10-00-00.db").exists());
        assert!(bdir.join("pre-restore-2026-11-03_10-00-00.db").exists());
    }

    #[test]
    fn retention_protects_the_in_progress_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let bdir = dir.path();

        let oldest = bdir.join("pre-restore-2026-11-01_10-00-00.db");
        std::fs::write(&oldest, b"keep").unwrap();
        for i in 2..=7 {
            std::fs::write(
                bdir.join(format!("pre-restore-2026-11-0{i}_10-00-00.db")),
                b"x",
            )
            .unwrap();
        }

        prune(bdir, Some(&oldest)).unwrap();
        assert!(oldest.exists(), "protected snapshot must survive");
        assert!(!bdir.join("pre-restore-2026-11-02_10-00-00.db").exists());
        let remaining = std::fs::read_dir(bdir)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .count();
        assert_eq!(remaining, 5);
    }
}
