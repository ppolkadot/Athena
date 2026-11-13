//! File exports. Every write goes through [`atomic_write`]: a temp file in the destination
//! folder, checked, then renamed into place, so a failure never damages an existing export.

pub mod csv;
pub mod json;

use std::fs::File;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use tempfile::NamedTempFile;

use crate::domain::person::Person;
use crate::domain::transaction::{Transaction, TxFilter};
use crate::infrastructure::db::{people_repo, transactions_repo};
use crate::shared::error::{AppError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReceipt {
    pub path: PathBuf,
    pub bytes: u64,
    pub record_count: usize,
}

/// Write `path` atomically. `write` fills (and may re-read, to validate) a temp file in the same
/// folder; only if it succeeds is the file synced and renamed over `path`. On any error the temp
/// file is removed and `path` is left exactly as it was.
///
/// # Errors
/// Whatever `write` returns, or I/O errors creating, syncing or renaming the file.
pub fn atomic_write(path: &Path, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = NamedTempFile::new_in(dir)?;
    write(temp.as_file_mut())?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| AppError::Io(e.error))?;
    Ok(())
}

/// Live transactions in one fixed order for every format: `date ASC, id ASC`, with each
/// transaction's allocations by person id.
pub(crate) fn ordered_transactions(
    conn: &Connection,
    filter: &TxFilter,
) -> Result<Vec<Transaction>> {
    let mut transactions = transactions_repo::list(conn, filter)?;
    transactions.sort_by_key(|t| (t.date, t.id));
    for tx in &mut transactions {
        tx.allocations.sort_by_key(|a| a.person_id);
    }
    Ok(transactions)
}

/// Every person, archived included, by id.
pub(crate) fn people_by_id(conn: &Connection) -> Result<Vec<Person>> {
    let mut people = people_repo::list(conn, true)?;
    people.sort_by_key(|p| p.id);
    Ok(people)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_export_does_not_truncate_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing.csv");
        std::fs::write(&existing, b"keep").unwrap();
        let result = atomic_write(&existing, |file| {
            std::io::Write::write_all(file, b"partial").unwrap();
            Err(AppError::Export("validation failed".into()))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&existing).unwrap(), b"keep");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temp file is left behind"
        );
    }

    #[test]
    fn a_successful_write_replaces_the_file_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        std::fs::write(&path, b"old old old old").unwrap();
        atomic_write(&path, |file| {
            std::io::Write::write_all(file, b"new")?;
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_new_file_is_created_and_a_missing_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.txt");
        atomic_write(&path, |file| {
            std::io::Write::write_all(file, b"x")?;
            Ok(())
        })
        .unwrap();
        assert!(path.exists());
        let missing = dir.path().join("nope").join("x.txt");
        assert!(atomic_write(&missing, |_| Ok(())).is_err());
    }
}
