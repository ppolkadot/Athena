//! Owns the database and applies [`AppAction`]s. `apply` is the only mutation path for persisted
//! domain state; every successful action bumps `data_version`.

use chrono::{DateTime, NaiveDate, Utc};

use super::actions::{ActionOutcome, AppAction, OperationKind, OperationNotice, RefreshScope};
use crate::application::services::import::ImportPlan;
use crate::application::services::onboarding;
use crate::domain::transaction::Transaction;
use crate::i18n::ToastKey;
use crate::infrastructure::config::AppDirs;
use crate::infrastructure::db::{
    Db, appearance_repo, backup, categories_repo, import_repo, integrity, people_repo,
    profile_repo, restore, templates_repo, transactions_repo, trash_repo,
};
use crate::infrastructure::media::assets::MediaStore;
use crate::shared::error::{AppError, ImportError, Result};

#[derive(Debug)]
pub struct Store {
    db: Db,
    dirs: AppDirs,
    data_version: u64,
}

impl Store {
    #[must_use]
    pub fn new(db: Db, dirs: AppDirs) -> Self {
        Self {
            db,
            dirs,
            data_version: 0,
        }
    }

    #[must_use]
    pub fn db(&self) -> &Db {
        &self.db
    }

    #[must_use]
    pub fn dirs(&self) -> &AppDirs {
        &self.dirs
    }

    /// Increases on every committed action; the view model reloads when it changes.
    #[must_use]
    pub fn data_version(&self) -> u64 {
        self.data_version
    }

    /// Automatic >30d trash purging (backed by verified pre-purge snapshots and Task 8 restore UI).
    pub(crate) const AUTO_TRASH_RETENTION_ENABLED: bool = true;

    /// Once per launch: the daily snapshot, Trash 30-day retention (skipped if the snapshot failed
    /// or if the clock rolled back before `max(deleted_at)`), and, in debug builds, an integrity
    /// check whose findings are logged as a count only. Both backup tasks are skipped entirely
    /// until onboarding has completed: a fresh install has nothing to protect.
    pub fn run_startup_tasks(&self, today: NaiveDate, now_utc: DateTime<Utc>) {
        if let Err(err) = backup::clear_staging(&self.dirs) {
            tracing::warn!("could not clear staging folder: {err}");
        }
        let onboarded = match profile_repo::load(self.db.conn()) {
            Ok(Some(profile)) => profile.onboarding_complete,
            Ok(None) => false,
            Err(err) => {
                tracing::warn!("could not read the profile before startup tasks: {err}");
                false
            }
        };
        if onboarded {
            match backup::daily_snapshot(self.db.conn(), &self.dirs, today) {
                Ok(Some(_)) => {
                    tracing::info!("daily snapshot written");
                    if Self::AUTO_TRASH_RETENTION_ENABLED {
                        self.run_trash_retention(now_utc);
                    }
                }
                Ok(None) => {
                    if Self::AUTO_TRASH_RETENTION_ENABLED {
                        self.run_trash_retention(now_utc);
                    }
                }
                Err(err) => tracing::warn!("daily snapshot failed, skipping trash purge: {err}"),
            }
        } else {
            tracing::debug!(
                "profile is missing or not onboarded yet: skipping daily snapshot and trash retention"
            );
        }
        if cfg!(debug_assertions) {
            match integrity::check(self.db.conn()) {
                Ok(issues) if issues.is_empty() => {}
                Ok(issues) => tracing::error!("integrity check found {} issue(s)", issues.len()),
                Err(err) => tracing::warn!("integrity check failed: {err}"),
            }
        }
    }

    pub(crate) fn run_trash_retention(&self, now_utc: DateTime<Utc>) {
        self.run_trash_retention_with(now_utc, |conn, dirs, now| {
            backup::pre_purge_snapshot(conn, dirs, now)
        });
    }

    fn run_trash_retention_with<F>(&self, now_utc: DateTime<Utc>, make_snapshot: F)
    where
        F: FnOnce(&rusqlite::Connection, &AppDirs, DateTime<Utc>) -> Result<std::path::PathBuf>,
    {
        let conn = self.db.conn();
        let max_deleted = match trash_repo::max_deleted_at(conn) {
            Ok(Some(stamp)) => stamp,
            Ok(None) => return,
            Err(err) => {
                tracing::warn!("could not inspect trash timestamps: {err}");
                return;
            }
        };
        let Ok(max_dt) = DateTime::parse_from_rfc3339(&max_deleted) else {
            tracing::warn!("unparseable deleted_at timestamp in trash: {max_deleted}");
            return;
        };
        if now_utc < max_dt.with_timezone(&Utc) {
            tracing::warn!("clock appears earlier than latest deletion; skipping trash purge");
            return;
        }
        let Some(cutoff_dt) = now_utc.checked_sub_days(chrono::Days::new(30)) else {
            return;
        };
        let cutoff = cutoff_dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let eligible = match trash_repo::count_older_than(conn, &cutoff) {
            Ok(0) => return,
            Ok(n) => n,
            Err(err) => {
                tracing::warn!("could not count expired trash entries: {err}");
                return;
            }
        };
        match make_snapshot(conn, &self.dirs, now_utc) {
            Ok(snapshot_path) => {
                tracing::info!(
                    "verified pre-purge snapshot {} before purging {eligible} expired row(s)",
                    snapshot_path.display()
                );
            }
            Err(err) => {
                tracing::warn!("pre-purge snapshot failed, skipping trash purge: {err}");
                return;
            }
        }
        match trash_repo::purge_older_than(conn, &cutoff) {
            Ok(0) => {}
            Ok(n) => tracing::info!("purged {n} expired transaction(s) from trash"),
            Err(err) => tracing::warn!("trash retention purge failed: {err}"),
        }
    }

    /// # Errors
    /// [`crate::shared::error::AppError::NotFound`] when no such transaction exists.
    pub fn transaction(&self, id: i64) -> Result<Transaction> {
        transactions_repo::get(self.db.conn(), id)
    }

    /// Apply one action. Nothing changes (and `data_version` stays put) when it fails.
    ///
    /// # Errors
    /// Validation, not-found and database errors from the repositories.
    pub fn apply(&mut self, action: AppAction) -> Result<ActionOutcome> {
        let outcome = self.run(action)?;
        self.data_version += 1;
        Ok(outcome)
    }

    fn run(&mut self, action: AppAction) -> Result<ActionOutcome> {
        match action {
            AppAction::CreateBackup => {
                let file = backup::create_manual(self.db.conn(), &self.dirs, Utc::now())?;
                return Ok(ActionOutcome {
                    inverse: None,
                    toast: Some(ToastKey::BackupCreated),
                    refresh: RefreshScope::Backups,
                    notice: Some(OperationNotice {
                        kind: OperationKind::BackupCreated,
                        snapshot: Some(file.path),
                        source: None,
                    }),
                });
            }
            AppAction::DeleteBackup(path) => {
                backup::delete_manual(&self.dirs.backups_dir, &path)?;
                return Ok(outcome(
                    None,
                    Some(ToastKey::BackupDeleted),
                    RefreshScope::Backups,
                ));
            }
            AppAction::RestoreBackup(source) => {
                let receipt =
                    restore::restore_backup(&mut self.db, &self.dirs, &source, Utc::now())?;
                return Ok(ActionOutcome {
                    inverse: None,
                    toast: Some(ToastKey::Restored),
                    refresh: RefreshScope::All,
                    notice: Some(OperationNotice {
                        kind: OperationKind::Restored,
                        snapshot: Some(receipt.snapshot),
                        source: receipt
                            .source
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned()),
                    }),
                });
            }
            AppAction::ImportProfile(plan) => {
                return self.run_import_with(&plan, Utc::now(), |conn, dirs, now| {
                    backup::pre_import_snapshot(conn, dirs, now)
                });
            }
            _ => {}
        }
        let conn = self.db.conn();
        match action {
            AppAction::CompleteOnboarding(result) => {
                onboarding::complete(conn, &result)?;
                Ok(outcome(None, Some(ToastKey::Welcome), RefreshScope::All))
            }
            AppAction::SaveTemplate(_)
            | AppAction::UpdateTemplate(_, _)
            | AppAction::DeleteTemplate(_) => Self::run_template_action(conn, action),
            AppAction::SaveTransaction(_)
            | AppAction::SaveTransactionFromTemplate(_, _)
            | AppAction::UpdateTransaction(_, _)
            | AppAction::DeleteTransaction(_)
            | AppAction::RestoreTransaction(_)
            | AppAction::PurgeTransaction(_)
            | AppAction::EmptyTrash => Self::run_transaction_action(conn, action),
            AppAction::ApplyAppearance(_) | AppAction::ResetAppearance => {
                Self::run_appearance_action(&self.dirs, conn, action)
            }
            AppAction::SaveProfile(_)
            | AppAction::SavePerson(_)
            | AppAction::ArchivePerson(_)
            | AppAction::RestorePerson(_)
            | AppAction::SaveCategory(_)
            | AppAction::ArchiveCategory(_) => Self::run_settings_action(conn, action),
            AppAction::CreateBackup
            | AppAction::DeleteBackup(_)
            | AppAction::RestoreBackup(_)
            | AppAction::ImportProfile(_) => {
                unreachable!()
            }
        }
    }

    pub(crate) fn run_import_with<F>(
        &mut self,
        plan: &ImportPlan,
        now_utc: DateTime<Utc>,
        make_snapshot: F,
    ) -> Result<ActionOutcome>
    where
        F: FnOnce(&rusqlite::Connection, &AppDirs, DateTime<Utc>) -> Result<std::path::PathBuf>,
    {
        let snapshot = make_snapshot(self.db.conn(), &self.dirs, now_utc)
            .map_err(|err| AppError::Import(vec![ImportError::SnapshotFailed(err.to_string())]))?;
        import_repo::replace(self.db.conn(), plan)?;
        if let Err(err) = backup::prune(&self.dirs.backups_dir, Some(&snapshot)) {
            tracing::warn!("could not prune backups after import: {err}");
        }
        Ok(ActionOutcome {
            inverse: None,
            toast: Some(ToastKey::Imported),
            refresh: RefreshScope::All,
            notice: Some(OperationNotice {
                kind: OperationKind::Imported,
                snapshot: Some(snapshot),
                source: Some(plan.source_name.clone()),
            }),
        })
    }

    fn run_transaction_action(
        conn: &rusqlite::Connection,
        action: AppAction,
    ) -> Result<ActionOutcome> {
        match action {
            AppAction::SaveTransaction(draft) => {
                let id = transactions_repo::create(conn, &draft)?;
                Ok(outcome(
                    Some(AppAction::DeleteTransaction(id)),
                    Some(ToastKey::TransactionSaved),
                    RefreshScope::Month,
                ))
            }
            AppAction::SaveTransactionFromTemplate(draft, template_id) => {
                let id = transactions_repo::create_from_template(conn, &draft, template_id)?;
                Ok(outcome(
                    Some(AppAction::DeleteTransaction(id)),
                    Some(ToastKey::TransactionSaved),
                    RefreshScope::Month,
                ))
            }
            AppAction::UpdateTransaction(id, draft) => {
                let previous = transactions_repo::get(conn, id)?.to_draft();
                transactions_repo::update(conn, id, &draft)?;
                Ok(outcome(
                    Some(AppAction::UpdateTransaction(id, previous)),
                    Some(ToastKey::TransactionUpdated),
                    RefreshScope::Month,
                ))
            }
            AppAction::DeleteTransaction(id) => {
                transactions_repo::delete(conn, id)?;
                Ok(outcome(
                    Some(AppAction::RestoreTransaction(id)),
                    Some(ToastKey::TransactionDeleted),
                    RefreshScope::Month,
                ))
            }
            AppAction::RestoreTransaction(id) => {
                trash_repo::restore(conn, id)?;
                Ok(outcome(
                    Some(AppAction::DeleteTransaction(id)),
                    Some(ToastKey::TransactionRestored),
                    RefreshScope::Month,
                ))
            }
            AppAction::PurgeTransaction(id) => {
                trash_repo::purge(conn, id)?;
                Ok(ActionOutcome {
                    inverse: None,
                    toast: None,
                    refresh: RefreshScope::Trash,
                    notice: Some(trash_purged_notice()),
                })
            }
            AppAction::EmptyTrash => {
                trash_repo::empty(conn)?;
                Ok(ActionOutcome {
                    inverse: None,
                    toast: Some(ToastKey::TrashEmptied),
                    refresh: RefreshScope::Trash,
                    notice: Some(trash_purged_notice()),
                })
            }
            _ => unreachable!(),
        }
    }

    fn run_template_action(
        conn: &rusqlite::Connection,
        action: AppAction,
    ) -> Result<ActionOutcome> {
        match action {
            AppAction::SaveTemplate(draft) => {
                templates_repo::create(conn, &draft)?;
                Ok(outcome(
                    None,
                    Some(ToastKey::TemplateSaved),
                    RefreshScope::Templates,
                ))
            }
            AppAction::UpdateTemplate(id, draft) => {
                templates_repo::update(conn, id, &draft)?;
                Ok(outcome(
                    None,
                    Some(ToastKey::TemplateUpdated),
                    RefreshScope::Templates,
                ))
            }
            AppAction::DeleteTemplate(id) => {
                templates_repo::delete(conn, id)?;
                Ok(outcome(
                    None,
                    Some(ToastKey::TemplateDeleted),
                    RefreshScope::Templates,
                ))
            }
            _ => unreachable!(),
        }
    }

    fn run_appearance_action(
        dirs: &AppDirs,
        conn: &rusqlite::Connection,
        action: AppAction,
    ) -> Result<ActionOutcome> {
        match action {
            AppAction::ApplyAppearance(apply) => {
                apply.prefs.validate()?;
                let store = MediaStore::new(dirs);
                let published = match &apply.media {
                    Some(media) => {
                        if apply.prefs.wallpaper.asset_id() != Some(&media.id) {
                            return Err(AppError::Validation(
                                "the staged wallpaper is not the selected one".into(),
                            ));
                        }
                        Some(store.publish(media)?)
                    }
                    None => None,
                };
                if let Err(err) = appearance_repo::save(conn, &apply.prefs) {
                    if let (Some(media), Some(published)) = (&apply.media, published) {
                        store.discard(media, published);
                    }
                    return Err(err);
                }
                Ok(outcome(
                    None,
                    Some(ToastKey::AppearanceSaved),
                    RefreshScope::Appearance,
                ))
            }
            AppAction::ResetAppearance => {
                appearance_repo::reset(conn)?;
                Ok(outcome(
                    None,
                    Some(ToastKey::AppearanceReset),
                    RefreshScope::Appearance,
                ))
            }
            _ => unreachable!(),
        }
    }

    fn run_settings_action(
        conn: &rusqlite::Connection,
        action: AppAction,
    ) -> Result<ActionOutcome> {
        match action {
            AppAction::SaveProfile(profile) => {
                profile_repo::save(conn, &profile)?;
                Ok(outcome(
                    None,
                    Some(ToastKey::ProfileSaved),
                    RefreshScope::All,
                ))
            }
            AppAction::SavePerson(person) => {
                if person.id == 0 {
                    people_repo::insert(conn, &person.name, person.kind)?;
                } else {
                    people_repo::update(conn, &person)?;
                }
                Ok(outcome(
                    None,
                    Some(ToastKey::PersonSaved),
                    RefreshScope::People,
                ))
            }
            AppAction::ArchivePerson(id) => {
                people_repo::set_archived(conn, id, true)?;
                Ok(outcome(
                    None,
                    Some(ToastKey::PersonArchived),
                    RefreshScope::People,
                ))
            }
            AppAction::RestorePerson(id) => {
                people_repo::set_archived(conn, id, false)?;
                Ok(outcome(
                    None,
                    Some(ToastKey::PersonRestored),
                    RefreshScope::People,
                ))
            }
            AppAction::SaveCategory(category) => {
                if category.id == 0 {
                    categories_repo::insert(
                        conn,
                        &category.name,
                        &category.color,
                        category.icon.as_deref(),
                        category.applies_to,
                    )?;
                } else {
                    categories_repo::update(conn, &category)?;
                }
                Ok(outcome(
                    None,
                    Some(ToastKey::CategorySaved),
                    RefreshScope::Categories,
                ))
            }
            AppAction::ArchiveCategory(id) => {
                categories_repo::set_archived(conn, id, true)?;
                Ok(outcome(
                    None,
                    Some(ToastKey::CategoryArchived),
                    RefreshScope::Categories,
                ))
            }
            _ => unreachable!(),
        }
    }
}

fn outcome(
    inverse: Option<AppAction>,
    toast: Option<ToastKey>,
    refresh: RefreshScope,
) -> ActionOutcome {
    ActionOutcome {
        inverse,
        toast,
        refresh,
        notice: None,
    }
}

fn trash_purged_notice() -> OperationNotice {
    OperationNotice {
        kind: OperationKind::TrashPurged,
        snapshot: None,
        source: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::domain::transaction::Allocation;
    use crate::shared::error::AppError;
    use crate::shared::test_fixtures::{draft_with, seed_people};

    fn store() -> Store {
        Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(std::env::temp_dir().join("athena-store-test")),
        )
    }

    fn onboarded_profile() -> crate::domain::settings::Profile {
        crate::domain::settings::Profile {
            start_balance: Money::ZERO,
            theme: crate::domain::settings::Theme::Dark,
            language: crate::domain::settings::Language::En,
            onboarding_complete: true,
        }
    }

    #[test]
    fn delete_backup_allows_only_local_manual_files_and_refreshes_backups() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let db = Db::open(&dirs.db_path).unwrap();
        let manual = dirs.backups_dir.join("manual-2026-11-16_12-00-00.db");
        let auto = dirs.backups_dir.join("auto-2026-11-16.db");
        let outside = dirs.root.join("manual-2026-11-16_12-00-00.db");
        for path in [&manual, &auto, &outside] {
            std::fs::write(path, b"snapshot").unwrap();
        }
        let mut store = Store::new(db, dirs);
        for path in [&auto, &outside] {
            assert!(store.apply(AppAction::DeleteBackup(path.clone())).is_err());
            assert_eq!(store.data_version(), 0);
            assert!(path.exists());
        }
        let outcome = store
            .apply(AppAction::DeleteBackup(manual.clone()))
            .unwrap();
        assert_eq!(outcome.refresh, RefreshScope::Backups);
        assert_eq!(outcome.toast, Some(ToastKey::BackupDeleted));
        assert!(!manual.exists());
    }

    #[test]
    fn create_returns_a_delete_inverse_that_undoes_it() {
        let mut store = store();
        let (owner, _) = seed_people(store.db().conn());
        let draft = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(500),
        }]);
        let outcome = store.apply(AppAction::SaveTransaction(draft)).unwrap();
        assert_eq!(outcome.toast, Some(ToastKey::TransactionSaved));
        assert_eq!(outcome.refresh, RefreshScope::Month);
        let Some(AppAction::DeleteTransaction(id)) = outcome.inverse else {
            panic!("expected a delete inverse")
        };
        assert!(store.transaction(id).is_ok());
        store.apply(AppAction::DeleteTransaction(id)).unwrap();
        assert!(matches!(store.transaction(id), Err(AppError::NotFound(_))));
    }

    #[test]
    fn template_actions_save_update_delete_and_record_use_with_an_undoable_entry() {
        use crate::domain::template::{ResponsibilityMode, TemplateDraft};
        use crate::domain::transaction::TransactionType;
        let mut store = store();
        let (owner, partner) = seed_people(store.db().conn());
        let share = |person_id, cents| Allocation {
            person_id,
            amount: Money::from_cents(cents),
        };
        let draft = TemplateDraft {
            name: "Rent".into(),
            title: "Rent".into(),
            amount: Money::from_cents(10_001),
            kind: TransactionType::Expense,
            category_id: None,
            payer_id: Some(owner),
            description: None,
            responsibility: ResponsibilityMode::Even,
            allocations: vec![share(partner, 0), share(owner, 0)],
        };
        let outcome = store.apply(AppAction::SaveTemplate(draft)).unwrap();
        assert_eq!(outcome.refresh, RefreshScope::Templates);
        assert_eq!(outcome.toast, Some(ToastKey::TemplateSaved));
        let template = templates_repo::list(store.db().conn()).unwrap().remove(0);
        assert_eq!(
            template.draft.allocations,
            vec![share(owner, 5_001), share(partner, 5_000)]
        );

        let mut renamed = template.draft.clone();
        renamed.name = "Flat rent".into();
        store
            .apply(AppAction::UpdateTemplate(template.id, renamed))
            .unwrap();

        // Using the template saves the entry and its usage together; the inverse undoes the entry.
        let entry = template
            .draft
            .to_tx_draft(crate::shared::test_fixtures::today());
        let version = store.data_version();
        let outcome = store
            .apply(AppAction::SaveTransactionFromTemplate(
                entry.clone(),
                template.id,
            ))
            .unwrap();
        assert_eq!(store.data_version(), version + 1);
        assert_eq!(outcome.refresh, RefreshScope::Month);
        assert!(
            templates_repo::get(store.db().conn(), template.id)
                .unwrap()
                .last_used_at
                .is_some()
        );
        let Some(AppAction::DeleteTransaction(id)) = outcome.inverse else {
            panic!("expected a delete inverse")
        };
        store.apply(AppAction::DeleteTransaction(id)).unwrap();

        // A missing template fails the whole action and changes nothing.
        let version = store.data_version();
        assert!(
            store
                .apply(AppAction::SaveTransactionFromTemplate(entry, 999))
                .is_err()
        );
        assert_eq!(store.data_version(), version);
        assert_eq!(transactions_repo::count_live(store.db().conn()).unwrap(), 0);

        store.apply(AppAction::DeleteTemplate(template.id)).unwrap();
        assert_eq!(templates_repo::count(store.db().conn()).unwrap(), 0);
        assert!(integrity::check(store.db().conn()).unwrap().is_empty());
    }

    #[test]
    fn update_returns_the_previous_draft_as_its_inverse() {
        let mut store = store();
        let original = draft_with(vec![]);
        store
            .apply(AppAction::SaveTransaction(original.clone()))
            .unwrap();
        let id = transactions_repo::list(
            store.db().conn(),
            &crate::domain::transaction::TxFilter::default(),
        )
        .unwrap()[0]
            .id;
        let mut edited = original.clone();
        edited.title = "Edited".into();
        let outcome = store
            .apply(AppAction::UpdateTransaction(id, edited))
            .unwrap();
        assert_eq!(store.transaction(id).unwrap().title, "Edited");
        store.apply(outcome.inverse.unwrap()).unwrap();
        assert_eq!(store.transaction(id).unwrap().title, "Test");
    }

    #[test]
    fn delete_is_soft_and_restorable() {
        let mut store = store();
        let (owner, _) = seed_people(store.db().conn());
        let allocs = vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(100),
        }];
        store
            .apply(AppAction::SaveTransaction(draft_with(allocs.clone())))
            .unwrap();
        let id = transactions_repo::list(
            store.db().conn(),
            &crate::domain::transaction::TxFilter::default(),
        )
        .unwrap()[0]
            .id;
        let deleted = store.apply(AppAction::DeleteTransaction(id)).unwrap();
        assert_eq!(deleted.inverse, Some(AppAction::RestoreTransaction(id)));
        assert_eq!(deleted.toast, Some(ToastKey::TransactionDeleted));
        assert!(matches!(store.transaction(id), Err(AppError::NotFound(_))));
        assert_eq!(trash_repo::count(store.db().conn()).unwrap(), 1);

        let restored = store.apply(deleted.inverse.unwrap()).unwrap();
        assert_eq!(restored.inverse, Some(AppAction::DeleteTransaction(id)));
        assert_eq!(restored.toast, Some(ToastKey::TransactionRestored));
        let tx = store.transaction(id).unwrap();
        assert_eq!(tx.allocations, allocs);
        assert_eq!(trash_repo::count(store.db().conn()).unwrap(), 0);
    }

    #[test]
    fn only_successful_actions_bump_the_data_version() {
        let mut store = store();
        assert_eq!(store.data_version(), 0);
        store
            .apply(AppAction::SaveTransaction(draft_with(vec![])))
            .unwrap();
        assert_eq!(store.data_version(), 1);
        assert!(store.apply(AppAction::DeleteTransaction(9_999)).is_err());
        let mut bad = draft_with(vec![]);
        bad.title = "  ".into();
        assert!(store.apply(AppAction::SaveTransaction(bad)).is_err());
        assert_eq!(store.data_version(), 1);
    }

    #[test]
    fn people_and_category_actions_scope_their_refresh() {
        let mut store = store();
        let person = crate::domain::person::Person {
            id: 0,
            name: "A3".into(),
            kind: crate::domain::person::PersonKind::Owner,
            archived: false,
            created_at: String::new(),
        };
        assert_eq!(
            store.apply(AppAction::SavePerson(person)).unwrap().refresh,
            RefreshScope::People
        );
        let category = crate::domain::category::Category {
            name: "Pets".into(),
            ..Default::default()
        };
        assert_eq!(
            store
                .apply(AppAction::SaveCategory(category))
                .unwrap()
                .refresh,
            RefreshScope::Categories
        );
        assert!(
            categories_repo::list(store.db().conn(), false)
                .unwrap()
                .iter()
                .any(|c| c.name == "Pets")
        );
    }

    #[test]
    fn startup_tasks_write_one_snapshot_per_day() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        let store = Store::new(Db::open_in_memory().unwrap(), dirs.clone());
        profile_repo::save(store.db().conn(), &onboarded_profile()).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        let now_utc = DateTime::parse_from_rfc3339("2026-11-12T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.run_startup_tasks(today, now_utc);
        store.run_startup_tasks(today, now_utc);
        let snapshots = std::fs::read_dir(&dirs.backups_dir).unwrap().count();
        assert_eq!(snapshots, 1);
    }

    #[test]
    fn startup_tasks_skip_snapshot_before_onboarding() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let store = Store::new(Db::open(&dirs.db_path).unwrap(), dirs.clone());
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let expired = transactions_repo::create(store.db().conn(), &draft_with(vec![])).unwrap();
        store
            .db()
            .conn()
            .execute(
                "UPDATE transactions SET deleted_at = '2026-10-01T00:00:00Z' WHERE id = ?1",
                [expired],
            )
            .unwrap();

        store.run_startup_tasks(today, now_utc);
        let autos = || {
            std::fs::read_dir(&dirs.backups_dir)
                .unwrap()
                .filter_map(std::result::Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with("auto-"))
                .count()
        };
        assert_eq!(autos(), 0, "no daily snapshot before onboarding");
        assert_eq!(
            trash_repo::count(store.db().conn()).unwrap(),
            1,
            "trash retention must not purge before onboarding"
        );

        profile_repo::save(store.db().conn(), &onboarded_profile()).unwrap();
        store.run_startup_tasks(today, now_utc);
        assert!(
            dirs.backups_dir.join("auto-2026-11-16.db").exists(),
            "the snapshot is written once onboarding is complete"
        );
    }

    #[test]
    fn retention_purges_after_snapshot_only() {
        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let seed = |store: &Store| {
            let (_owner, partner) = seed_people(store.db().conn());
            let mut expired_draft = draft_with(vec![Allocation {
                person_id: partner,
                amount: Money::from_cents(4_200),
            }]);
            expired_draft.title = "Expired with alloc".into();
            let expired = transactions_repo::create(store.db().conn(), &expired_draft).unwrap();
            let recent = transactions_repo::create(store.db().conn(), &draft_with(vec![])).unwrap();
            store
                .db()
                .conn()
                .execute(
                    "UPDATE transactions SET deleted_at = '2026-10-10T00:00:00Z' WHERE id = ?1",
                    [expired],
                )
                .unwrap();
            store
                .db()
                .conn()
                .execute(
                    "UPDATE transactions SET deleted_at = '2026-11-01T00:00:00Z' WHERE id = ?1",
                    [recent],
                )
                .unwrap();
            (expired, recent)
        };

        // Snapshot failure (file blocking backups_dir): nothing is purged.
        let blocked_dir = tempfile::tempdir().unwrap();
        let blocked_dirs = AppDirs::from_root(blocked_dir.path().to_path_buf());
        std::fs::write(&blocked_dirs.backups_dir, b"not a dir").unwrap();
        let blocked_store = Store::new(Db::open_in_memory().unwrap(), blocked_dirs);
        seed(&blocked_store);
        blocked_store.run_trash_retention(now_utc);
        assert_eq!(trash_repo::count(blocked_store.db().conn()).unwrap(), 2);

        // Snapshot failure injected directly even when backups_dir is valid: nothing is purged.
        let fail_dir = tempfile::tempdir().unwrap();
        let fail_dirs = AppDirs::from_root(fail_dir.path().to_path_buf());
        fail_dirs.ensure().unwrap();
        let fail_store = Store::new(Db::open_in_memory().unwrap(), fail_dirs);
        seed(&fail_store);
        fail_store.run_trash_retention_with(now_utc, |_, _, _| {
            Err(AppError::Backup("injected snapshot failure".into()))
        });
        assert_eq!(trash_repo::count(fail_store.db().conn()).unwrap(), 2);

        // Pre-purge snapshot succeeds: >30d row is purged, recent row stays, and the
        // pre-purge snapshot preserves the purged row and its allocations.
        let ok_dir = tempfile::tempdir().unwrap();
        let ok_dirs = AppDirs::from_root(ok_dir.path().to_path_buf());
        let ok_store = Store::new(Db::open_in_memory().unwrap(), ok_dirs.clone());
        let (expired, recent) = seed(&ok_store);
        ok_store.run_trash_retention(now_utc);
        assert_eq!(trash_repo::count(ok_store.db().conn()).unwrap(), 1);
        assert!(matches!(
            trash_repo::get_deleted(ok_store.db().conn(), expired),
            Err(AppError::NotFound(_))
        ));
        assert!(trash_repo::get_deleted(ok_store.db().conn(), recent).is_ok());

        let pre_purge = ok_dirs.backups_dir.join("pre-purge-2026-11-16_12-00-00.db");
        assert!(pre_purge.exists());
        assert!(backup::is_valid_athena_snapshot(&pre_purge).unwrap());
        let snap_db = Db::open_read_only(&pre_purge).unwrap();
        let preserved = trash_repo::get_deleted(snap_db.conn(), expired).unwrap();
        assert_eq!(preserved.tx.title, "Expired with alloc");
        assert_eq!(preserved.tx.allocations.len(), 1);
        assert_eq!(preserved.tx.allocations[0].amount, Money::from_cents(4_200));
    }

    #[test]
    fn retention_creates_fresh_pre_purge_snapshot_even_when_same_day_auto_snapshot_is_stale() {
        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let store = Store::new(Db::open_in_memory().unwrap(), dirs.clone());
        let (_owner, partner) = seed_people(store.db().conn());

        // Seed an earlier transaction and take today's auto- snapshot BEFORE the expired row
        // gets its final content/allocation and deletion timestamp.
        let mut draft = draft_with(vec![]);
        draft.title = "Old content before edit".into();
        let tx_id = transactions_repo::create(store.db().conn(), &draft).unwrap();
        assert!(
            backup::daily_snapshot(store.db().conn(), &dirs, today)
                .unwrap()
                .is_some()
        );

        // Now update the row with new content + allocation, and mark it expired in trash.
        let mut updated = draft_with(vec![Allocation {
            person_id: partner,
            amount: Money::from_cents(7_500),
        }]);
        updated.title = "Edited before delete".into();
        transactions_repo::update(store.db().conn(), tx_id, &updated).unwrap();
        store
            .db()
            .conn()
            .execute(
                "UPDATE transactions SET deleted_at = '2026-10-01T00:00:00Z' WHERE id = ?1",
                [tx_id],
            )
            .unwrap();

        store.run_trash_retention(now_utc);
        assert_eq!(trash_repo::count(store.db().conn()).unwrap(), 0);
        let pre_purge = dirs.backups_dir.join("pre-purge-2026-11-16_12-00-00.db");
        let snap_db = Db::open_read_only(&pre_purge).unwrap();
        let preserved = trash_repo::get_deleted(snap_db.conn(), tx_id).unwrap();
        assert_eq!(preserved.tx.title, "Edited before delete");
        assert_eq!(preserved.tx.allocations.len(), 1);
        assert_eq!(preserved.tx.allocations[0].amount, Money::from_cents(7_500));
    }

    #[test]
    fn startup_auto_purges_and_pre_purge_snapshot_restores_deleted_rows() {
        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let mut store = Store::new(Db::open(&dirs.db_path).unwrap(), dirs.clone());
        profile_repo::save(
            store.db().conn(),
            &crate::domain::settings::Profile {
                start_balance: Money::ZERO,
                theme: crate::domain::settings::Theme::Dark,
                language: crate::domain::settings::Language::En,
                onboarding_complete: true,
            },
        )
        .unwrap();
        let (_owner, partner) = seed_people(store.db().conn());

        let mut live_draft = draft_with(vec![]);
        live_draft.title = "Live Lunch".into();
        let live_id = transactions_repo::create(store.db().conn(), &live_draft).unwrap();

        let mut expired_draft = draft_with(vec![Allocation {
            person_id: partner,
            amount: Money::from_cents(2_500),
        }]);
        expired_draft.title = "Expired Coffee".into();
        let expired_id = transactions_repo::create(store.db().conn(), &expired_draft).unwrap();
        store
            .db()
            .conn()
            .execute(
                "UPDATE transactions SET deleted_at = '2026-10-01T00:00:00Z' WHERE id = ?1",
                [expired_id],
            )
            .unwrap();

        store.run_startup_tasks(today, now_utc);
        assert!(dirs.backups_dir.join("auto-2026-11-16.db").exists());
        let pre_purge = dirs.backups_dir.join("pre-purge-2026-11-16_12-00-00.db");
        assert!(pre_purge.exists());
        assert_eq!(trash_repo::count(store.db().conn()).unwrap(), 0);

        store
            .apply(AppAction::RestoreBackup(restore::RestoreSource::Listed(
                pre_purge,
            )))
            .unwrap();
        assert_eq!(trash_repo::count(store.db().conn()).unwrap(), 1);
        let restored = trash_repo::get_deleted(store.db().conn(), expired_id).unwrap();
        assert_eq!(restored.tx.title, "Expired Coffee");
        assert_eq!(store.transaction(live_id).unwrap().title, "Live Lunch");
    }

    #[test]
    fn retention_skips_on_clock_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        let store = Store::new(Db::open_in_memory().unwrap(), dirs);
        let expired = transactions_repo::create(store.db().conn(), &draft_with(vec![])).unwrap();
        let future = transactions_repo::create(store.db().conn(), &draft_with(vec![])).unwrap();
        store
            .db()
            .conn()
            .execute(
                "UPDATE transactions SET deleted_at = '2026-10-01T00:00:00Z' WHERE id = ?1",
                [expired],
            )
            .unwrap();
        store
            .db()
            .conn()
            .execute(
                "UPDATE transactions SET deleted_at = '2026-12-01T00:00:00Z' WHERE id = ?1",
                [future],
            )
            .unwrap();

        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.run_trash_retention(now_utc);
        assert_eq!(trash_repo::count(store.db().conn()).unwrap(), 2);
    }

    #[test]
    fn retention_skips_purge_when_existing_daily_snapshot_is_untrusted() {
        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();

        for case in ["empty", "corrupt", "foreign_sqlite", "directory"] {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            dirs.ensure().unwrap();
            let store = Store::new(Db::open_in_memory().unwrap(), dirs.clone());
            profile_repo::save(store.db().conn(), &onboarded_profile()).unwrap();
            let expired =
                transactions_repo::create(store.db().conn(), &draft_with(vec![])).unwrap();
            store
                .db()
                .conn()
                .execute(
                    "UPDATE transactions SET deleted_at = '2026-10-01T00:00:00Z' WHERE id = ?1",
                    [expired],
                )
                .unwrap();

            let snap = dirs.backups_dir.join("auto-2026-11-16.db");
            match case {
                "empty" => std::fs::write(&snap, b"").unwrap(),
                "corrupt" => std::fs::write(&snap, b"broken").unwrap(),
                "foreign_sqlite" => {
                    let conn = rusqlite::Connection::open(&snap).unwrap();
                    conn.execute_batch("CREATE TABLE other (x INTEGER);")
                        .unwrap();
                }
                "directory" => std::fs::create_dir_all(&snap).unwrap(),
                _ => unreachable!(),
            }

            assert!(
                backup::daily_snapshot(store.db().conn(), &dirs, today).is_err(),
                "daily_snapshot must fail closed on {case}"
            );
            store.run_startup_tasks(today, now_utc);
            assert_eq!(
                trash_repo::count(store.db().conn()).unwrap(),
                1,
                "expired row must survive when auto snapshot is {case}"
            );
            match case {
                "empty" => assert_eq!(std::fs::read(&snap).unwrap(), b""),
                "corrupt" => assert_eq!(std::fs::read(&snap).unwrap(), b"broken"),
                "foreign_sqlite" => assert!(!backup::is_valid_athena_snapshot(&snap).unwrap()),
                "directory" => assert!(snap.is_dir()),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn recorded_settlement_clears_the_balance() {
        use crate::application::services::settlement;
        use crate::domain::date::YearMonth;
        use crate::domain::transaction::{Allocation, TransactionType};
        use crate::shared::test_fixtures::{date, seed_month, seed_people};

        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(dir.path().to_path_buf()),
        );
        seed_month(store.db());
        let (owner, partner) = seed_people(store.db().conn());
        let nov = YearMonth::new(2026, 11).unwrap();

        let before = settlement::for_month(store.db().conn(), nov).unwrap();
        assert_eq!(before.transfers.len(), 1);
        assert_eq!(before.transfers[0].from, partner);
        assert_eq!(before.transfers[0].to, owner);
        assert_eq!(before.transfers[0].amount, Money::from_cents(2_500));

        let mut payback = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(2_500),
        }]);
        payback.title = "Ana → A3".into();
        payback.kind = TransactionType::Settlement;
        payback.payer_id = Some(partner);
        payback.date = date(2026, 11, 20);
        store.apply(AppAction::SaveTransaction(payback)).unwrap();

        let after = settlement::for_month(store.db().conn(), nov).unwrap();
        assert!(after.transfers.is_empty());
        assert_eq!(after.balances.len(), 2);
        assert!(after.balances.iter().all(|b| b.balance.is_zero()));
    }

    #[test]
    fn staging_is_cleared_at_startup_and_only_staging() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        std::fs::write(dirs.staging_dir.join("leftover.db"), b"stale").unwrap();
        std::fs::create_dir_all(dirs.staging_dir.join("nested")).unwrap();
        std::fs::write(dirs.staging_dir.join("nested/file.tmp"), b"stale").unwrap();

        let keep_backup = dirs.backups_dir.join("manual-2026-11-15_10-00-00.db");
        std::fs::write(&keep_backup, b"keep").unwrap();
        let keep_export = dirs.exports_dir.join("export.json");
        std::fs::write(&keep_export, b"keep").unwrap();

        let store = Store::new(Db::open(&dirs.db_path).unwrap(), dirs.clone());
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.run_startup_tasks(today, now_utc);

        assert!(dirs.staging_dir.is_dir());
        assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
        assert!(keep_backup.exists());
        assert!(keep_export.exists());
        assert!(dirs.db_path.exists());
    }

    fn import_plan_fixture() -> ImportPlan {
        use crate::domain::category::{Category, CategoryScope};
        use crate::domain::person::{Person, PersonKind};
        use crate::domain::settings::{Language, Profile, Theme};
        ImportPlan {
            source_name: "imported.json".into(),
            format_version: 2,
            app_version: "1.1.0".into(),
            exported_at: "2026-11-15T12:00:00Z".into(),
            used_timestamp_fallback: false,
            profile: Profile {
                start_balance: Money::from_cents(77_700),
                theme: Theme::Light,
                language: Language::PtBr,
                onboarding_complete: true,
            },
            people: vec![Person {
                id: 5,
                name: "Marina".into(),
                kind: PersonKind::Owner,
                archived: false,
                created_at: "2026-11-01T00:00:00Z".into(),
            }],
            categories: vec![Category {
                id: 8,
                name: "ImportedCat".into(),
                color: "#4FC3F7".into(),
                icon: None,
                applies_to: CategoryScope::Expense,
                is_default: false,
                archived: false,
                created_at: "2026-11-01T00:00:00Z".into(),
            }],
            transactions: vec![],
            templates: vec![],
            widened_categories: Vec::new(),
            appearance: None,
        }
    }

    #[test]
    fn import_writes_pre_import_snapshot_first() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let mut store = Store::new(Db::open(&dirs.db_path).unwrap(), dirs.clone());
        let (owner, _) = seed_people(store.db().conn());
        let mut existing = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(2_500),
        }]);
        existing.title = "Before Import".into();
        transactions_repo::create(store.db().conn(), &existing).unwrap();

        let plan = import_plan_fixture();
        let outcome = store
            .apply(AppAction::ImportProfile(Box::new(plan)))
            .unwrap();
        assert_eq!(outcome.toast, Some(ToastKey::Imported));
        assert_eq!(outcome.refresh, RefreshScope::All);
        let notice = outcome.notice.unwrap();
        assert_eq!(notice.kind, OperationKind::Imported);
        assert_eq!(notice.source.as_deref(), Some("imported.json"));
        let snapshot = notice.snapshot.unwrap();
        assert!(
            snapshot
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pre-import-")
        );
        assert!(backup::is_valid_athena_snapshot(&snapshot).unwrap());

        // The safety snapshot captures the pre-import database contents.
        let snap_db = Db::open_read_only(&snapshot).unwrap();
        let snap_txs = transactions_repo::list(
            snap_db.conn(),
            &crate::domain::transaction::TxFilter::default(),
        )
        .unwrap();
        assert_eq!(snap_txs.len(), 1);
        assert_eq!(snap_txs[0].title, "Before Import");

        // Live database now holds the imported data.
        assert_eq!(
            people_repo::owner(store.db().conn()).unwrap().unwrap().name,
            "Marina"
        );
    }

    #[test]
    fn snapshot_failure_aborts_import() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let mut store = Store::new(Db::open(&dirs.db_path).unwrap(), dirs.clone());
        let (owner, _) = seed_people(store.db().conn());
        let mut existing = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(1_200),
        }]);
        existing.title = "Keep Intact".into();
        transactions_repo::create(store.db().conn(), &existing).unwrap();
        let version_before = store.data_version();

        let plan = import_plan_fixture();
        let err = store
            .run_import_with(&plan, Utc::now(), |_, _, _| {
                Err(AppError::Backup("disk full".into()))
            })
            .unwrap_err();
        assert!(matches!(
            err,
            AppError::Import(ref errs)
                if matches!(errs.as_slice(), [ImportError::SnapshotFailed(_)])
        ));
        assert_eq!(store.data_version(), version_before);
        let txs = transactions_repo::list(
            store.db().conn(),
            &crate::domain::transaction::TxFilter::default(),
        )
        .unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].title, "Keep Intact");
        assert_eq!(
            people_repo::owner(store.db().conn()).unwrap().unwrap().name,
            "A3"
        );
    }
}
