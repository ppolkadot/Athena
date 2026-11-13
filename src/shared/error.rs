//! The single application error type. Later tasks extend it; they must not add a second enum.

use thiserror::Error;

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
