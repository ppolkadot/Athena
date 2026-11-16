//! The single application error type. Later tasks extend it; they must not add a second enum.

use thiserror::Error;

use crate::infrastructure::db::integrity::IntegrityIssue;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RestoreError {
    #[error("the chosen file is not an Athena backup")]
    NotAthena,
    #[error(
        "this backup was created by a newer Athena (schema {found}, this version supports {supported})"
    )]
    TooNew { found: i64, supported: i64 },
    #[error("the chosen backup file is empty or damaged")]
    Corrupt,
    #[error("the backup uses an incompatible SQLite page size")]
    PageSize,
    #[error("the backup contains inconsistent records ({} issue(s))", .0.len())]
    Invalid(Vec<IntegrityIssue>),
    #[error("could not create a pre-restore safety snapshot: {0}")]
    SnapshotFailed(String),
    #[error("cannot restore from the active database file")]
    SameAsLive,
    #[error("restore verification failed for {source_path}; rolled back to {snapshot_path}")]
    VerificationFailed {
        source_path: String,
        snapshot_path: String,
    },
    #[error("restore verification failed and rollback from {snapshot_path} also failed: {detail}")]
    RollbackFailed {
        snapshot_path: String,
        detail: String,
    },
    #[error("restore failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("data folder error: {0}")]
    Dirs(String),
    #[error("migration failed: {0}")]
    Migration(String),
    #[error(
        "this database was created by a newer Athena (schema {found}, this version supports {supported})"
    )]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("{0} was not found")]
    NotFound(&'static str),
    #[error("backup failed: {0}")]
    Backup(String),
    #[error(transparent)]
    Restore(#[from] RestoreError),
    #[error(
        "the database at {path} could not be read. Rename it and restore a backup from the backups folder"
    )]
    Corrupt { path: String },
    #[error("invalid amount: {0}")]
    Money(String),
    #[error("invalid data: {0}")]
    Validation(String),
    #[error("export failed: {0}")]
    Export(String),
    #[error("interface error: {0}")]
    Ui(String),
    #[error("Athena is already running")]
    AlreadyRunning,
}

pub type Result<T> = std::result::Result<T, AppError>;
