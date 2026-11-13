//! Owns the database and applies [`AppAction`]s. `apply` is the only mutation path for persisted
//! domain state; every successful action bumps `data_version`.

use chrono::NaiveDate;

use super::actions::{ActionOutcome, AppAction, RefreshScope};
use crate::application::services::onboarding;
use crate::domain::transaction::Transaction;
use crate::i18n::ToastKey;
use crate::infrastructure::config::AppDirs;
use crate::infrastructure::db::{
    Db, backup, categories_repo, integrity, people_repo, profile_repo, transactions_repo,
};
use crate::shared::error::Result;

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

    /// Once per launch: the daily snapshot (a failure is logged, never fatal) and, in debug
    /// builds, an integrity check whose findings are logged as a count only.
    pub fn run_startup_tasks(&self, today: NaiveDate) {
        match backup::daily_snapshot(self.db.conn(), &self.dirs, today) {
            Ok(Some(_)) => tracing::info!("daily snapshot written"),
            Ok(None) => {}
            Err(err) => tracing::warn!("daily snapshot failed: {err}"),
        }
        if cfg!(debug_assertions) {
            match integrity::check(self.db.conn()) {
                Ok(issues) if issues.is_empty() => {}
                Ok(issues) => tracing::error!("integrity check found {} issue(s)", issues.len()),
                Err(err) => tracing::warn!("integrity check failed: {err}"),
            }
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

    fn run(&self, action: AppAction) -> Result<ActionOutcome> {
        let conn = self.db.conn();
        let (inverse, toast, refresh) = match action {
            AppAction::CompleteOnboarding(result) => {
                onboarding::complete(conn, &result)?;
                (None, Some(ToastKey::Welcome), RefreshScope::All)
            }
            AppAction::SaveTransaction(draft) => {
                let id = transactions_repo::create(conn, &draft)?;
                (
                    Some(AppAction::DeleteTransaction(id)),
                    Some(ToastKey::TransactionSaved),
                    RefreshScope::Month,
                )
            }
            AppAction::UpdateTransaction(id, draft) => {
                let previous = transactions_repo::get(conn, id)?.to_draft();
                transactions_repo::update(conn, id, &draft)?;
                (
                    Some(AppAction::UpdateTransaction(id, previous)),
                    Some(ToastKey::TransactionUpdated),
                    RefreshScope::Month,
                )
            }
            AppAction::DeleteTransaction(id) => {
                // 1.0 deletes for good, so there is nothing to undo; 1.1 returns `RestoreTransaction`.
                transactions_repo::delete(conn, id)?;
                (
                    None,
                    Some(ToastKey::TransactionDeleted),
                    RefreshScope::Month,
                )
            }
            AppAction::SaveProfile(profile) => {
                profile_repo::save(conn, &profile)?;
                (None, Some(ToastKey::ProfileSaved), RefreshScope::All)
            }
            AppAction::SavePerson(person) => {
                if person.id == 0 {
                    people_repo::insert(conn, &person.name, person.kind)?;
                } else {
                    people_repo::update(conn, &person)?;
                }
                (None, Some(ToastKey::PersonSaved), RefreshScope::People)
            }
            AppAction::ArchivePerson(id) => {
                people_repo::set_archived(conn, id, true)?;
                (None, Some(ToastKey::PersonArchived), RefreshScope::People)
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
                (
                    None,
                    Some(ToastKey::CategorySaved),
                    RefreshScope::Categories,
                )
            }
            AppAction::ArchiveCategory(id) => {
                categories_repo::set_archived(conn, id, true)?;
                (
                    None,
                    Some(ToastKey::CategoryArchived),
                    RefreshScope::Categories,
                )
            }
        };
        Ok(ActionOutcome {
            inverse,
            toast,
            refresh,
        })
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
    fn hard_delete_has_no_inverse_in_1_0() {
        let mut store = store();
        store
            .apply(AppAction::SaveTransaction(draft_with(vec![])))
            .unwrap();
        let id = transactions_repo::list(
            store.db().conn(),
            &crate::domain::transaction::TxFilter::default(),
        )
        .unwrap()[0]
            .id;
        assert_eq!(
            store
                .apply(AppAction::DeleteTransaction(id))
                .unwrap()
                .inverse,
            None
        );
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
        let today = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        store.run_startup_tasks(today);
        store.run_startup_tasks(today);
        let snapshots = std::fs::read_dir(&dirs.backups_dir).unwrap().count();
        assert_eq!(snapshots, 1);
    }
}
