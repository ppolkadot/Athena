//! Startup recovery for an unreadable live database (`§11`).
//!
//! Recovery cannot use the normal restore pipeline: there is no live connection to snapshot, and
//! the live file is the thing that cannot be read. The source is therefore staged, validated, and
//! upgraded exactly as `restore.rs` does for steps 1–4, then the broken `athena.db`, `-wal`, and
//! `-shm` are renamed `broken-<ts>.db…` to free the path, and the staged copy is written in their
//! place. Nothing is overwritten before the staged copy passed validation.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use super::{Db, backup, restore};
use crate::infrastructure::config::AppDirs;
use crate::shared::error::{AppError, RestoreError, Result};

/// Prefix of the files a recovery restore moves aside.
pub const BROKEN_PREFIX: &str = "broken-";

/// Restore `source` over an unreadable live database, leaving a database file that opens.
///
/// The staged copy is validated and upgraded first; a bad backup therefore never reaches the live
/// path. A failure after the rename keeps the broken files renamed and intact, removes anything
/// half-written in their place, and returns the error so the caller stays on the recovery screen.
///
/// # Errors
/// [`AppError::Restore`] when the source is not a usable Athena backup, and database or I/O errors
/// when the live path cannot be replaced.
pub fn restore_over_broken(
    dirs: &AppDirs,
    source: &Path,
    now_utc: DateTime<Utc>,
) -> Result<PathBuf> {
    let staged = staged_recovery_path(dirs, now_utc)?;
    if let Err(err) = stage_and_upgrade(source, &staged) {
        remove_sqlite_files(&staged);
        return Err(err);
    }
    let broken = match move_broken_aside(dirs, now_utc) {
        Ok(path) => path,
        Err(err) => {
            remove_sqlite_files(&staged);
            return Err(err);
        }
    };
    if let Err(err) = install_staged(&staged, dirs) {
        remove_sqlite_files(&dirs.db_path);
        remove_sqlite_files(&staged);
        return Err(err);
    }
    remove_sqlite_files(&staged);
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

/// Write the staged copy at the live path and prove the result opens and migrates.
fn install_staged(staged: &Path, dirs: &AppDirs) -> Result<()> {
    restore::step_stage(staged, &dirs.db_path)?;
    drop(Db::open(&dirs.db_path)?);
    Ok(())
}

/// Rename the broken live files out of the way, returning the first path that moved.
fn move_broken_aside(dirs: &AppDirs, now_utc: DateTime<Utc>) -> Result<PathBuf> {
    let broken = unique_broken_path(dirs, now_utc)?;
    let mut files = Vec::new();
    for suffix in ["", "-wal", "-shm"] {
        let live = with_suffix(&dirs.db_path, suffix);
        match std::fs::symlink_metadata(&live) {
            Ok(_) => files.push((live, with_suffix(&broken, suffix))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    let mut moved = Vec::new();
    for (live, destination) in files {
        if let Err(err) = std::fs::rename(&live, &destination) {
            for (original, saved) in moved.iter().rev() {
                if let Err(rollback) = std::fs::rename(saved, original) {
                    tracing::error!("could not roll back broken-file move: {rollback}");
                }
            }
            return Err(err.into());
        }
        moved.push((live, destination));
    }
    Ok(broken)
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
    for suffix in ["", "-wal", "-shm"] {
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
    for suffix in ["-wal", "-shm"] {
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
}
