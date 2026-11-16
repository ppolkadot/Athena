//! The only way the UI changes persisted data: build an [`AppAction`], hand it to
//! `Store::apply`, show the [`ActionOutcome`].

use std::path::PathBuf;

use crate::application::services::onboarding::OnboardingResult;
use crate::domain::category::Category;
use crate::domain::person::Person;
use crate::domain::settings::Profile;
use crate::domain::transaction::TxDraft;
use crate::i18n::ToastKey;

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    CompleteOnboarding(OnboardingResult),
    SaveTransaction(TxDraft),
    UpdateTransaction(i64, TxDraft),
    DeleteTransaction(i64),
    RestoreTransaction(i64),
    PurgeTransaction(i64),
    EmptyTrash,
    SaveProfile(Profile),
    /// `id == 0` creates a person; otherwise renames the existing one.
    SavePerson(Person),
    ArchivePerson(i64),
    /// `id == 0` creates a category; otherwise edits the existing one.
    SaveCategory(Category),
    ArchiveCategory(i64),
}

/// What the view model must reload after an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshScope {
    /// Transactions and everything derived from them: register, summaries, calendar, balance.
    Month,
    Calendar,
    Trash,
    Backups,
    People,
    Categories,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    BackupCreated,
    Restored,
    Imported,
    TrashPurged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationNotice {
    pub kind: OperationKind,
    pub snapshot: Option<PathBuf>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionOutcome {
    /// The action that undoes this one, when one exists.
    pub inverse: Option<AppAction>,
    pub toast: Option<ToastKey>,
    pub refresh: RefreshScope,
    pub notice: Option<OperationNotice>,
}
