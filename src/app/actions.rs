//! The only way the UI changes persisted data: build an [`AppAction`], hand it to
//! `Store::apply`, show the [`ActionOutcome`].

use std::path::PathBuf;

use crate::application::services::import::ImportPlan;
use crate::application::services::onboarding::OnboardingResult;
use crate::domain::appearance::AppearancePrefs;
use crate::domain::category::Category;
use crate::domain::date::YearMonth;
use crate::domain::goal::{ContributionDraft, GoalDraft, GoalTransition};
use crate::domain::person::Person;
use crate::domain::settings::Profile;
use crate::domain::template::TemplateDraft;
use crate::domain::transaction::TxDraft;
use crate::i18n::ToastKey;

pub use crate::infrastructure::db::restore::RestoreSource;
use crate::infrastructure::media::assets::StagedMedia;

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    CompleteOnboarding(OnboardingResult),
    SaveTransaction(TxDraft),
    UpdateTransaction(i64, TxDraft),
    /// Insert the entry and record the template's `last_used_at` in one transaction.
    SaveTransactionFromTemplate(TxDraft, i64),
    SaveTemplate(TemplateDraft),
    UpdateTemplate(i64, TemplateDraft),
    DeleteTemplate(i64),
    DeleteTransaction(i64),
    RestoreTransaction(i64),
    PurgeTransaction(i64),
    EmptyTrash,
    SaveProfile(Profile),
    /// `id == 0` creates a person; otherwise renames the existing one.
    SavePerson(Person),
    ArchivePerson(i64),
    RestorePerson(i64),
    /// `id == 0` creates a category; otherwise edits the existing one.
    SaveCategory(Category),
    ArchiveCategory(i64),
    CreateBackup,
    /// Permanently delete one manual backup file from the backups folder.
    DeleteBackup(std::path::PathBuf),
    RestoreBackup(RestoreSource),
    ImportProfile(Box<ImportPlan>),
    /// Write the full appearance row (contract §14.2).
    ApplyAppearance(Box<AppearanceApply>),
    /// Delete the appearance row so defaults derive from the profile again.
    ResetAppearance,
    /// Create a goal in the Active status (contract §14.3).
    CreateGoal(GoalDraft),
    UpdateGoal(i64, GoalDraft),
    ChangeGoalStatus(i64, GoalTransition),
    AddContribution(i64, ContributionDraft),
    DeleteContribution {
        goal_id: i64,
        contribution_id: i64,
    },
    /// Save a closing for a completed local month (contract §14.3).
    CloseMonth(YearMonth),
    /// Replace a month's saved closing with a fresh capture.
    RecalculateClosing(YearMonth),
}

/// What Apply commits: the preferences and, for a newly chosen local wallpaper, the staged copy
/// that is published before the row references it.
#[derive(Debug, Clone, PartialEq)]
pub struct AppearanceApply {
    pub prefs: AppearancePrefs,
    pub media: Option<StagedMedia>,
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
    /// Entry templates only.
    Templates,
    /// The optional appearance row only.
    Appearance,
    /// The goals list and the selected goal's detail.
    Goals,
    /// Saved closings and their staleness.
    Closings,
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
    /// The goal a `CreateGoal` action just created, so the screen can select it.
    pub created_goal: Option<i64>,
}
