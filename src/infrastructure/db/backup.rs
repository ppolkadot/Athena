//! `VACUUM INTO` snapshots. Never raw-copies a live WAL database.

use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{Connection, OpenFlags};

use super::migrations;
use crate::infrastructure::config::AppDirs;
use crate::shared::error::{AppError, Result};

/// How many `auto-` snapshots are kept.
pub const AUTO_SNAPSHOTS_KEPT: usize = 7;
/// How many `pre-purge-` / `pre-restore-` / `pre-import-` snapshots are kept.
pub const PRE_OPERATION_SNAPSHOTS_KEPT: usize = 5;
/// SQLite `PRAGMA application_id` set on Athena databases from schema v2 onward (`"ATNA"`).
pub const ATHENA_APPLICATION_ID: i64 = 0x4154_4E41;

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

/// Verify that `path` is a regular, non-empty, integrity-clean SQLite file belonging to Athena
/// with a supported schema version (`1..=latest_version()`). Returns `Ok(false)` (never panics)
/// when the path is missing, a directory/symlink, empty, corrupt, foreign, or too new.
///
/// # Errors
/// Reserved for unexpected non-SQLite I/O failures; malformed files return `Ok(false)`.
pub fn is_valid_athena_snapshot(path: &Path) -> Result<bool> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(false);
    };
    if !meta.is_file() || meta.len() == 0 {
        return Ok(false);
    }
    let Ok(conn) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return Ok(false);
    };
    let Ok(mut integrity_stmt) = conn.prepare("PRAGMA integrity_check") else {
        return Ok(false);
    };
    let Ok(integrity_rows) = integrity_stmt
        .query_map([], |r| r.get::<_, String>(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
    else {
        return Ok(false);
    };
    if integrity_rows.len() != 1 || integrity_rows[0] != "ok" {
        return Ok(false);
    }
    let Ok(version) = migrations::schema_version(&conn) else {
        return Ok(false);
    };
    if version < 1 || version > migrations::latest_version() {
        return Ok(false);
    }
    let Ok(app_id) = conn.pragma_query_value(None, "application_id", |r| r.get::<_, i64>(0)) else {
        return Ok(false);
    };
    let Ok(mut table_stmt) = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    ) else {
        return Ok(false);
    };
    let Ok(tables) = table_stmt
        .query_map([], |r| r.get::<_, String>(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
    else {
        return Ok(false);
    };

    if version == 1 {
        let exact_v1 = tables.iter().map(String::as_str).eq(V1_TABLES);
        return Ok(exact_v1 && (app_id == 0 || app_id == ATHENA_APPLICATION_ID));
    }

    if app_id != ATHENA_APPLICATION_ID {
        return Ok(false);
    }
    if !V1_TABLES
        .iter()
        .all(|required| tables.iter().any(|t| t == required))
    {
        return Ok(false);
    }
    if version >= 3 {
        let Ok(has_live_view) = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'view' AND name = 'live_transactions')",
            [],
            |r| r.get::<_, bool>(0),
        ) else {
            return Ok(false);
        };
        if !has_live_view {
            return Ok(false);
        }
    }
    Ok(true)
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
    prune_prefix(&dirs.backups_dir, "auto-", AUTO_SNAPSHOTS_KEPT)?;
    Ok(Some(path))
}

/// Write and verify a fresh `pre-purge-YYYY-MM-DD_HH-MM-SS.db` snapshot of `conn` before automatic
/// trash retention purges expired rows, then prune older `pre-purge-` files while protecting the
/// newly created snapshot.
///
/// # Errors
/// [`AppError::Backup`], SQLite, or I/O errors if the snapshot cannot be written or verified.
pub fn pre_purge_snapshot(
    conn: &Connection,
    dirs: &AppDirs,
    now_utc: DateTime<Utc>,
) -> Result<PathBuf> {
    std::fs::create_dir_all(&dirs.backups_dir)?;
    let stem = format!("pre-purge-{}", now_utc.format("%Y-%m-%d_%H-%M-%S"));
    let path = unique_backup_path(&dirs.backups_dir, &stem);
    vacuum_into(conn, &path)?;
    if !is_valid_athena_snapshot(&path)? {
        let _ = std::fs::remove_file(&path);
        return Err(AppError::Backup(
            "pre-purge snapshot failed Athena verification".into(),
        ));
    }
    prune_prefix_protected(
        &dirs.backups_dir,
        "pre-purge-",
        PRE_OPERATION_SNAPSHOTS_KEPT,
        Some(&path),
    )?;
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

/// Delete the oldest `.db` files whose name starts with `prefix`, keeping `keep` (names sort by date).
///
/// # Errors
/// I/O errors.
pub fn prune_prefix(dir: &Path, prefix: &str, keep: usize) -> Result<usize> {
    prune_prefix_protected(dir, prefix, keep, None)
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

fn is_protected_path(candidate: &Path, protected: Option<&Path>) -> bool {
    let Some(protected) = protected else {
        return false;
    };
    if candidate == protected {
        return true;
    }
    match (
        std::fs::canonicalize(candidate),
        std::fs::canonicalize(protected),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
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
}
