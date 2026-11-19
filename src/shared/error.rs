//! The single application error type. Later tasks extend it; they must not add a second enum.

use thiserror::Error;

use crate::domain::goal::GoalError;
use crate::domain::template::TemplateError;
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

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ImportError {
    #[error("the chosen file exceeds the 64 MiB limit")]
    TooLarge,
    #[error("could not read the import file: {0}")]
    Io(String),
    #[error("the chosen file is not valid Athena JSON: {0}")]
    MalformedJson(String),
    #[error("the chosen file is not an Athena export")]
    NotAthena,
    #[error(
        "this export was created by a newer Athena (format {found}, this version supports {supported})"
    )]
    TooNew { found: u64, supported: u32 },
    #[error("invalid export format version")]
    InvalidVersion,
    #[error("invalid exported_at timestamp")]
    InvalidExportedAt,
    #[error("templates are not supported in format 1 imports")]
    TemplatesNotSupported,
    #[error("the export comes from an unfinished setup")]
    OnboardingIncomplete,
    #[error("invalid profile theme")]
    InvalidTheme,
    #[error("invalid profile language")]
    InvalidLanguage,
    /// The `appearance` section has an unknown value or an out-of-range number.
    #[error("invalid appearance settings")]
    InvalidAppearance,
    #[error("person at index {index} has a non-positive id ({id})")]
    NonPositivePersonId { index: usize, id: i64 },
    #[error("duplicate person id {id}")]
    DuplicatePersonId { id: i64 },
    #[error("person #{id} has a blank name")]
    BlankPersonName { id: i64 },
    #[error("duplicate person name \"{name}\"")]
    DuplicatePersonName { name: String },
    #[error("person #{id} has an unknown kind")]
    InvalidPersonKind { id: i64 },
    #[error("person #{id} has an invalid created_at timestamp")]
    InvalidPersonTimestamp { id: i64 },
    #[error("the export must contain exactly one owner (found {count})")]
    OwnerCount { count: usize },
    #[error("the owner cannot be archived")]
    OwnerArchived,
    #[error("category at index {index} has a non-positive id ({id})")]
    NonPositiveCategoryId { index: usize, id: i64 },
    #[error("duplicate category id {id}")]
    DuplicateCategoryId { id: i64 },
    #[error("category #{id} has a blank name")]
    BlankCategoryName { id: i64 },
    #[error("duplicate category name \"{name}\"")]
    DuplicateCategoryName { name: String },
    #[error("category #{id} has an invalid color")]
    InvalidCategoryColor { id: i64 },
    #[error("category #{id} has an invalid scope")]
    InvalidCategoryScope { id: i64 },
    #[error("category #{id} has an invalid created_at timestamp")]
    InvalidCategoryTimestamp { id: i64 },
    #[error("transaction at index {index} has a non-positive id ({id})")]
    NonPositiveTransactionId { index: usize, id: i64 },
    #[error("duplicate transaction id {id}")]
    DuplicateTransactionId { id: i64 },
    #[error("transaction #{id} has a blank title")]
    BlankTransactionTitle { id: i64 },
    #[error("transaction #{id} has a non-positive amount")]
    NonPositiveAmount { id: i64 },
    #[error("transaction #{id} has an invalid date")]
    InvalidDate { id: i64 },
    #[error("transaction #{id} has an unknown kind")]
    InvalidTransactionKind { id: i64 },
    #[error("transaction #{id} references unknown category #{category_id}")]
    MissingCategory { id: i64, category_id: i64 },
    #[error("transaction #{id} references unknown payer #{payer_id}")]
    MissingPayer { id: i64, payer_id: i64 },
    #[error("transaction #{id} references unknown person #{person_id} in allocations")]
    MissingAllocationPerson { id: i64, person_id: i64 },
    #[error("transaction #{id} lists person #{person_id} more than once in allocations")]
    DuplicateAllocationPerson { id: i64, person_id: i64 },
    #[error("transaction #{id} has a negative allocation share")]
    NegativeAllocationShare { id: i64 },
    #[error("transaction #{id} allocations do not sum to its amount")]
    AllocationSumMismatch { id: i64 },
    #[error("transaction #{id} has an unknown split mode")]
    InvalidSplitMode { id: i64 },
    #[error("transaction #{id} sets a split mode with fewer than two allocations")]
    SplitModeWithoutSharedSplit { id: i64 },
    #[error("settlement #{id} has an invalid shape")]
    InvalidSettlementShape { id: i64 },
    #[error("transaction #{id} has an invalid timestamp")]
    InvalidTransactionTimestamp { id: i64 },
    #[error("template at index {index} has a non-positive id ({id})")]
    NonPositiveTemplateId { index: usize, id: i64 },
    #[error("duplicate template id {id}")]
    DuplicateTemplateId { id: i64 },
    #[error("template #{id} has a blank name")]
    BlankTemplateName { id: i64 },
    #[error("duplicate template name \"{name}\"")]
    DuplicateTemplateName { name: String },
    #[error("template #{id} has a blank title")]
    BlankTemplateTitle { id: i64 },
    #[error("template #{id} has a non-positive amount")]
    NonPositiveTemplateAmount { id: i64 },
    #[error("template #{id} has an unknown or unsupported kind")]
    InvalidTemplateKind { id: i64 },
    #[error("template #{id} has an unknown responsibility mode")]
    InvalidTemplateResponsibility { id: i64 },
    #[error("template #{id} references unknown category #{category_id}")]
    TemplateMissingCategory { id: i64, category_id: i64 },
    #[error("template #{id} references unknown payer #{payer_id}")]
    TemplateMissingPayer { id: i64, payer_id: i64 },
    #[error("template #{id} references unknown person #{person_id} in its shares")]
    TemplateMissingPerson { id: i64, person_id: i64 },
    #[error("template #{id} has inconsistent shares or responsibility")]
    InvalidTemplateShape { id: i64 },
    #[error("template #{id} has an invalid timestamp")]
    InvalidTemplateTimestamp { id: i64 },
    #[error("imported totals exceed the 64-bit integer limit")]
    Overflow,
    #[error("could not create a pre-import safety snapshot: {0}")]
    SnapshotFailed(String),
    #[error("import verification failed: {0}")]
    VerificationFailed(String),
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
    #[error("import failed ({} issue(s))", .0.len())]
    Import(Vec<ImportError>),
    #[error(
        "the database at {path} could not be read. Rename it and restore a backup from the backups folder"
    )]
    Corrupt { path: String },
    #[error("the live database is missing or empty")]
    LiveDatabaseMissingOrEmpty,
    #[error("the live database is missing or empty, and inspecting recovery files failed: {0}")]
    RecoveryScanUncertain(String),
    #[error("the active database metadata could not be verified: {0}")]
    LiveMetadataIndeterminate(String),
    #[error("a previous recovery attempt left conflicting files: {0}")]
    RecoveryConflict(String),
    #[error("invalid amount: {0}")]
    Money(String),
    #[error("invalid data: {0}")]
    Validation(String),
    #[error("this category is used by entries that its scope would exclude")]
    CategoryScopeInUse,
    #[error("the template is invalid ({} issue(s))", .0.len())]
    TemplateInvalid(Vec<TemplateError>),
    #[error("the goal request is invalid ({} issue(s))", .0.len())]
    GoalInvalid(Vec<GoalError>),
    #[error(transparent)]
    Media(#[from] crate::infrastructure::media::MediaError),
    #[error("export failed: {0}")]
    Export(String),
    #[error("interface error: {0}")]
    Ui(String),
    #[error("Athena is already running")]
    AlreadyRunning,
}

impl From<ImportError> for AppError {
    fn from(err: ImportError) -> Self {
        Self::Import(vec![err])
    }
}

impl From<Vec<ImportError>> for AppError {
    fn from(errs: Vec<ImportError>) -> Self {
        Self::Import(errs)
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
