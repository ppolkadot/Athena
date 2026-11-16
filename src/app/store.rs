//! Owns the database and applies [`AppAction`]s. `apply` is the only mutation path for persisted
//! domain state; every successful action bumps `data_version`.

use chrono::{DateTime, NaiveDate, Utc};

use super::actions::{ActionOutcome, AppAction, OperationKind, OperationNotice, RefreshScope};
use crate::application::services::onboarding;
use crate::domain::transaction::Transaction;
use crate::i18n::ToastKey;
use crate::infrastructure::config::AppDirs;
use crate::infrastructure::db::{
    Db, backup, categories_repo, integrity, people_repo, profile_repo, transactions_repo,
    trash_repo,
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

    /// Once per launch: the daily snapshot (a failure is logged, never fatal), Trash 30-day
    /// retention (skipped if the snapshot failed or if the clock rolled back before `max(deleted_at)`),
    /// and, in debug builds, an integrity check whose findings are logged as a count only.
    pub fn run_startup_tasks(&self, today: NaiveDate, now_utc: DateTime<Utc>) {
        match backup::daily_snapshot(self.db.conn(), &self.dirs, today) {
            Ok(Some(_)) => {
                tracing::info!("daily snapshot written");
                self.run_trash_retention(now_utc);
            }
            Ok(None) => self.run_trash_retention(now_utc),
            Err(err) => tracing::warn!("daily snapshot failed, skipping trash purge: {err}"),
        }
        if cfg!(debug_assertions) {
            match integrity::check(self.db.conn()) {
                Ok(issues) if issues.is_empty() => {}
                Ok(issues) => tracing::error!("integrity check found {} issue(s)", issues.len()),
                Err(err) => tracing::warn!("integrity check failed: {err}"),
            }
        }
    }

    fn run_trash_retention(&self, now_utc: DateTime<Utc>) {
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
        let conn = self.db.conn();
        match action {
            AppAction::CompleteOnboarding(result) => {
                onboarding::complete(conn, &result)?;
                Ok(outcome(None, Some(ToastKey::Welcome), RefreshScope::All))
            }
            AppAction::SaveTransaction(_)
            | AppAction::UpdateTransaction(_, _)
            | AppAction::DeleteTransaction(_)
            | AppAction::RestoreTransaction(_)
            | AppAction::PurgeTransaction(_)
            | AppAction::EmptyTrash => Self::run_transaction_action(conn, action),
            AppAction::SaveProfile(_)
            | AppAction::SavePerson(_)
            | AppAction::ArchivePerson(_)
            | AppAction::RestorePerson(_)
            | AppAction::SaveCategory(_)
            | AppAction::ArchiveCategory(_) => Self::run_settings_action(conn, action),
        }
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
    fn retention_purges_after_snapshot_only() {
        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();

        let seed = |store: &Store| {
            let expired =
                transactions_repo::create(store.db().conn(), &draft_with(vec![])).unwrap();
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
        blocked_store.run_startup_tasks(today, now_utc);
        assert_eq!(trash_repo::count(blocked_store.db().conn()).unwrap(), 2);

        // Snapshot succeeds: >30d row is purged, recent row stays.
        let ok_dir = tempfile::tempdir().unwrap();
        let ok_dirs = AppDirs::from_root(ok_dir.path().to_path_buf());
        let ok_store = Store::new(Db::open_in_memory().unwrap(), ok_dirs.clone());
        let (expired, recent) = seed(&ok_store);
        ok_store.run_startup_tasks(today, now_utc);
        assert_eq!(trash_repo::count(ok_store.db().conn()).unwrap(), 1);
        assert!(matches!(
            trash_repo::get_deleted(ok_store.db().conn(), expired),
            Err(AppError::NotFound(_))
        ));
        assert!(trash_repo::get_deleted(ok_store.db().conn(), recent).is_ok());
        assert!(ok_dirs.backups_dir.join("auto-2026-11-16.db").exists());
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

        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.run_startup_tasks(today, now_utc);
        assert_eq!(trash_repo::count(store.db().conn()).unwrap(), 2);
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
}
