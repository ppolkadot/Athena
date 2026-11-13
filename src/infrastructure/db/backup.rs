//! `VACUUM INTO` snapshots. Never raw-copies a live WAL database.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rusqlite::{Connection, OpenFlags};

use crate::infrastructure::config::AppDirs;
use crate::shared::error::{AppError, Result};

/// How many `auto-` snapshots are kept.
pub const AUTO_SNAPSHOTS_KEPT: usize = 7;

/// Run `PRAGMA integrity_check` on the database file at `path` (opened read-only).
///
/// # Errors
/// Database errors opening or querying the file.
pub fn integrity_ok(path: &Path) -> Result<bool> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let verdict: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    Ok(verdict == "ok")
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
/// [`AUTO_SNAPSHOTS_KEPT`] `auto-` files. Files with any other prefix are never touched.
/// Returns the new path, or `None` when today's snapshot already exists.
///
/// # Errors
/// See [`vacuum_into`]; I/O errors listing or pruning the folder.
pub fn daily_snapshot(
    conn: &Connection,
    dirs: &AppDirs,
    today: NaiveDate,
) -> Result<Option<PathBuf>> {
    std::fs::create_dir_all(&dirs.backups_dir)?;
    let path = dirs
        .backups_dir
        .join(format!("auto-{}.db", today.format("%Y-%m-%d")));
    if path.exists() {
        return Ok(None);
    }
    vacuum_into(conn, &path)?;
    prune_prefix(&dirs.backups_dir, "auto-", AUTO_SNAPSHOTS_KEPT)?;
    Ok(Some(path))
}

/// Delete the oldest `.db` files whose name starts with `prefix`, keeping `keep` (names sort by date).
///
/// # Errors
/// I/O errors.
pub fn prune_prefix(dir: &Path, prefix: &str, keep: usize) -> Result<usize> {
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
    for name in names.iter().take(excess) {
        std::fs::remove_file(dir.join(name))?;
    }
    Ok(excess)
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
}
