//! Cached read data for the screens. Screens read this and never query the database. It reloads
//! when the store's `data_version` or the selected month, year or filter changes.

use super::actions::{OperationNotice, RefreshScope};
use super::navigation::Screen;
use super::store::Store;
use super::ui_state::UiState;
use crate::application::services::{reports, settlement};
use crate::domain::category::Category;
use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::person::{Person, PersonKind};
use crate::domain::reports::{CalendarMonthTotal, CategoryTotal, MonthSummary, PersonTotal};
use crate::domain::settings::{Language, Profile, Theme};
use crate::domain::settlement::SettlementView;
use crate::domain::template::Template;
use crate::domain::transaction::{Transaction, TransactionType, TxFilter};
use crate::infrastructure::db::{
    backup::{self, BackupFile},
    categories_repo, people_repo, profile_repo, templates_repo, transactions_repo,
    trash_repo::{self, TrashItem},
};
use crate::shared::error::Result;

/// What the cached data was loaded for. A different key means the cache is stale.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LoadKey {
    version: u64,
    month: YearMonth,
    year: i32,
    filter: TxFilter,
    today: chrono::NaiveDate,
    screen: Screen,
}

#[derive(Debug, Clone)]
pub struct ViewModel {
    fallback_language: Language,
    pub profile: Option<Profile>,
    /// Archived people included, so history labels always resolve.
    pub all_people: Vec<Person>,
    /// Archived categories included.
    pub all_categories: Vec<Category>,
    pub transactions: Vec<Transaction>,
    /// Entry templates, alphabetical by folded name then id.
    pub templates: Vec<Template>,
    pub summary: MonthSummary,
    pub category_totals: Vec<CategoryTotal>,
    pub person_totals: Vec<PersonTotal>,
    pub calendar: Vec<CalendarMonthTotal>,
    /// Start balance plus all income, expenses and refunds, all time.
    pub balance: Money,
    /// Cumulative household settlement balances and suggested transfers as of `ui.month`.
    pub settlement: Option<SettlementView>,
    /// `true` when the latest settlement calculation failed (e.g. overflow) and is unavailable.
    pub settlement_unavailable: bool,
    /// `true` when the latest view refresh failed on a required query; prior loaded state stays coherent.
    pub refresh_failed: bool,
    /// Number of deleted transactions currently in Trash.
    pub trash_count: usize,
    /// Soft-deleted transactions, loaded only when viewing the Trash screen.
    pub trash: Vec<TrashItem>,
    /// Managed `.db` snapshots in `backups_dir`, loaded only when viewing Settings.
    pub backups: Vec<BackupFile>,
    /// Most recent backup/restore/import/purge operation notice in this session.
    pub last_operation: Option<OperationNotice>,
    /// Where the database file lives, for the settings screen.
    pub data_path: String,
    /// Where managed backup snapshots are stored.
    pub backups_dir: String,
    /// The local date, kept here so screens never read the clock and tests can pin it.
    pub today: chrono::NaiveDate,
    #[cfg(test)]
    pub(crate) trash_loads: usize,
    loaded: Option<LoadKey>,
    failed_load_key: Option<LoadKey>,
}

impl ViewModel {
    /// Before anything is loaded: the given language, the dark theme, no data.
    #[must_use]
    pub fn new(fallback_language: Language) -> Self {
        Self {
            fallback_language,
            profile: None,
            all_people: Vec::new(),
            all_categories: Vec::new(),
            transactions: Vec::new(),
            templates: Vec::new(),
            summary: MonthSummary::default(),
            category_totals: Vec::new(),
            person_totals: Vec::new(),
            calendar: Vec::new(),
            balance: Money::ZERO,
            settlement: None,
            settlement_unavailable: false,
            refresh_failed: false,
            trash_count: 0,
            trash: Vec::new(),
            backups: Vec::new(),
            last_operation: None,
            data_path: String::new(),
            backups_dir: String::new(),
            today: chrono::Local::now().date_naive(),
            #[cfg(test)]
            trash_loads: 0,
            loaded: None,
            failed_load_key: None,
        }
    }

    #[must_use]
    pub fn language(&self) -> Language {
        self.profile
            .as_ref()
            .map_or(self.fallback_language, |p| p.language)
    }

    #[must_use]
    pub fn theme(&self) -> Theme {
        self.profile.as_ref().map_or(Theme::Dark, |p| p.theme)
    }

    #[must_use]
    pub fn onboarding_complete(&self) -> bool {
        self.profile.as_ref().is_some_and(|p| p.onboarding_complete)
    }

    #[must_use]
    pub fn owner(&self) -> Option<&Person> {
        self.all_people.iter().find(|p| p.kind == PersonKind::Owner)
    }

    #[must_use]
    pub fn owner_name(&self) -> Option<&str> {
        self.owner().map(|p| p.name.as_str())
    }

    #[must_use]
    pub fn person(&self, id: i64) -> Option<&Person> {
        self.all_people.iter().find(|p| p.id == id)
    }

    #[must_use]
    pub fn category(&self, id: i64) -> Option<&Category> {
        self.all_categories.iter().find(|c| c.id == id)
    }

    /// People a picker may offer: not archived, owner first.
    #[must_use]
    pub fn active_people(&self) -> Vec<&Person> {
        self.all_people.iter().filter(|p| !p.archived).collect()
    }

    /// Categories a picker may offer for `kind`: not archived and scoped to the kind.
    #[must_use]
    pub fn active_categories(&self, kind: TransactionType) -> Vec<&Category> {
        self.all_categories
            .iter()
            .filter(|c| !c.archived && c.applies_to.allows(kind))
            .collect()
    }

    fn key(&self, store: &Store, ui: &UiState) -> LoadKey {
        LoadKey {
            version: store.data_version(),
            month: ui.month,
            year: ui.calendar_year,
            filter: ui.filter.clone(),
            today: self.today,
            screen: ui.screen,
        }
    }

    #[cfg_attr(
        not(test),
        allow(clippy::unused_self, clippy::needless_pass_by_ref_mut)
    )]
    fn record_trash_load(&mut self) {
        #[cfg(test)]
        {
            self.trash_loads += 1;
        }
    }

    /// Clear recorded refresh failure so the next [`Self::sync`] retries loading the current key.
    pub fn clear_refresh_failure(&mut self) {
        self.failed_load_key = None;
        self.refresh_failed = false;
    }

    /// Reload everything if the store or the selection changed since the last load.
    /// When only `filter`, `screen`, or `calendar_year` changed at the same `data_version`,
    /// `month`, and `today`, only the affected slices are reloaded.
    /// Returns whether it reloaded.
    ///
    /// # Errors
    /// Database errors.
    pub fn sync(&mut self, store: &Store, ui: &UiState) -> Result<bool> {
        let key = self.key(store, ui);
        if self.loaded.as_ref() == Some(&key) || self.failed_load_key.as_ref() == Some(&key) {
            return Ok(false);
        }
        if !self.refresh_failed
            && let Some(prev) = self.loaded.clone()
            && prev.version == key.version
            && prev.month == key.month
            && prev.today == key.today
        {
            if prev.year == key.year {
                return self
                    .sync_filter_or_screen(store, ui, &prev, key)
                    .map(|()| true);
            }
            if prev.filter == key.filter && prev.screen == key.screen {
                return self.sync_calendar_year(store, ui, key).map(|()| true);
            }
        }
        self.reload(store, ui, RefreshScope::All)?;
        Ok(true)
    }

    fn sync_filter_or_screen(
        &mut self,
        store: &Store,
        ui: &UiState,
        prev: &LoadKey,
        key: LoadKey,
    ) -> Result<()> {
        let filter_changed = prev.filter != key.filter;
        let enter_trash = ui.screen == Screen::Trash && prev.screen != Screen::Trash;
        let enter_settings = ui.screen == Screen::Settings && prev.screen != Screen::Settings;
        let conn = store.db().conn();
        let staged: Result<_> = (|| {
            let transactions = if filter_changed {
                Some(transactions_repo::list(conn, &key.filter)?)
            } else {
                None
            };
            let trash = if enter_trash {
                Some((trash_repo::count(conn)?, trash_repo::list(conn)?))
            } else {
                None
            };
            let backups = if enter_settings {
                Some(backup::list(&store.dirs().backups_dir)?)
            } else {
                None
            };
            Ok((transactions, trash, backups))
        })();

        match staged {
            Ok((transactions, trash, backups)) => {
                if let Some(txs) = transactions {
                    self.transactions = txs;
                }
                if ui.screen == Screen::Trash {
                    if let Some((count, items)) = trash {
                        self.trash_count = count;
                        self.trash = items;
                        self.record_trash_load();
                    }
                } else {
                    self.trash.clear();
                }
                if ui.screen == Screen::Settings {
                    if let Some(items) = backups {
                        self.backups = items;
                    }
                } else {
                    self.backups.clear();
                }
                self.loaded = Some(key);
                self.failed_load_key = None;
                self.refresh_failed = false;
                Ok(())
            }
            Err(err) => {
                self.failed_load_key = Some(key);
                self.refresh_failed = true;
                Err(err)
            }
        }
    }

    /// Refresh only the backups listing (used after a failed restore or import that already wrote
    /// a safety snapshot while Settings stays open).
    ///
    /// # Errors
    /// I/O errors listing backups.
    pub fn refresh_backups(&mut self, store: &Store) -> Result<()> {
        self.backups = backup::list(&store.dirs().backups_dir)?;
        Ok(())
    }

    fn sync_calendar_year(&mut self, store: &Store, ui: &UiState, key: LoadKey) -> Result<()> {
        match reports::year_calendar(store.db().conn(), ui.calendar_year) {
            Ok(calendar) => {
                self.calendar = calendar;
                self.loaded = Some(key);
                self.failed_load_key = None;
                self.refresh_failed = false;
                Ok(())
            }
            Err(err) => {
                self.failed_load_key = Some(key);
                self.refresh_failed = true;
                Err(err)
            }
        }
    }

    /// Reload what `scope` covers. People and category changes reload only those lists.
    /// Stages all required queries before mutating `self` so an error never leaves half-updated
    /// state, and isolates optional settlement math failures into `settlement_unavailable`.
    ///
    /// # Errors
    /// Database or validation errors on required queries.
    pub fn reload(&mut self, store: &Store, ui: &UiState, scope: RefreshScope) -> Result<()> {
        let key = self.key(store, ui);
        let effective_scope = if self.loaded.is_none() || self.refresh_failed {
            RefreshScope::All
        } else {
            scope
        };
        match self.load_scope(store, ui, effective_scope, &key) {
            Ok(()) => {
                self.loaded = Some(key);
                self.failed_load_key = None;
                self.refresh_failed = false;
                Ok(())
            }
            Err(err) => {
                self.failed_load_key = Some(key);
                self.refresh_failed = true;
                Err(err)
            }
        }
    }

    fn load_scope(
        &mut self,
        store: &Store,
        ui: &UiState,
        scope: RefreshScope,
        key: &LoadKey,
    ) -> Result<()> {
        let conn = store.db().conn();
        match scope {
            RefreshScope::People => {
                let all_people = people_repo::list(conn, true)?;
                self.all_people = all_people;
            }
            RefreshScope::Categories => {
                let all_categories = categories_repo::list(conn, true)?;
                self.all_categories = all_categories;
            }
            RefreshScope::Templates => {
                self.templates = templates_repo::list(conn)?;
            }
            RefreshScope::Trash => {
                let trash_count = trash_repo::count(conn)?;
                let trash = if ui.screen == Screen::Trash {
                    trash_repo::list(conn)?
                } else {
                    Vec::new()
                };
                self.trash_count = trash_count;
                self.trash = trash;
                if ui.screen == Screen::Trash {
                    self.record_trash_load();
                }
            }
            RefreshScope::Backups => {
                self.backups = if ui.screen == Screen::Settings {
                    backup::list(&store.dirs().backups_dir)?
                } else {
                    Vec::new()
                };
            }
            RefreshScope::Month | RefreshScope::Calendar | RefreshScope::All => {
                let profile = profile_repo::load(conn)?;
                let all_people = people_repo::list(conn, true)?;
                let all_categories = categories_repo::list(conn, true)?;
                let transactions = transactions_repo::list(conn, &key.filter)?;
                let templates = templates_repo::list(conn)?;
                let summary = reports::month_summary(conn, ui.month)?;
                let category_totals = reports::category_totals(conn, ui.month)?;
                let person_totals = reports::person_totals(conn, ui.month)?;
                let calendar = reports::year_calendar(conn, ui.calendar_year)?;
                let start = profile.as_ref().map_or(Money::ZERO, |p| p.start_balance);
                let balance = reports::tracked_balance(conn, start)?;
                let trash_count = trash_repo::count(conn)?;
                let data_path = store.dirs().db_path.display().to_string();
                let backups_dir = store.dirs().backups_dir.display().to_string();
                let (trash, trash_loaded) = if ui.screen == Screen::Trash {
                    (trash_repo::list(conn)?, true)
                } else {
                    (Vec::new(), false)
                };
                let backups = if ui.screen == Screen::Settings {
                    backup::list(&store.dirs().backups_dir)?
                } else {
                    Vec::new()
                };
                let (settlement, settlement_unavailable) =
                    match settlement::for_month(conn, ui.month) {
                        Ok(view) => (Some(view), false),
                        Err(err) => {
                            tracing::warn!(
                                "settlement unavailable for {:04}-{:02}: {err}",
                                ui.month.year,
                                ui.month.month
                            );
                            (None, true)
                        }
                    };

                self.profile = profile;
                self.all_people = all_people;
                self.all_categories = all_categories;
                self.transactions = transactions;
                self.templates = templates;
                self.summary = summary;
                self.category_totals = category_totals;
                self.person_totals = person_totals;
                self.calendar = calendar;
                self.balance = balance;
                self.settlement = settlement;
                self.settlement_unavailable = settlement_unavailable;
                self.trash_count = trash_count;
                self.data_path = data_path;
                self.backups_dir = backups_dir;
                self.trash = trash;
                self.backups = backups;
                if trash_loaded {
                    self.record_trash_load();
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::config::AppDirs;
    use crate::infrastructure::db::Db;

    #[test]
    fn load_key_changes_at_midnight() {
        let store = Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(std::env::temp_dir().join("athena-vm-midnight-test")),
        );
        let ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        let mut vm = ViewModel::new(Language::En);
        let day1 = chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();
        let day2 = chrono::NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();

        vm.today = day1;
        assert!(vm.sync(&store, &ui).unwrap());
        assert!(!vm.sync(&store, &ui).unwrap());

        vm.today = day2;
        assert!(vm.sync(&store, &ui).unwrap());
        assert!(!vm.sync(&store, &ui).unwrap());
    }

    #[test]
    fn screen_change_reloads_screen_specific_models() {
        let store = Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(std::env::temp_dir().join("athena-vm-screen-test")),
        );
        let mut ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        let mut vm = ViewModel::new(Language::En);
        vm.today = chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();

        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.trash_loads, 0);

        ui.set_screen(Screen::Calendar);
        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.trash_loads, 0);

        ui.set_screen(Screen::Trash);
        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.trash_loads, 1);

        assert!(!vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.trash_loads, 1);
    }

    #[test]
    fn trash_lists_newest_first_and_loads_only_on_trash_screen() {
        use crate::shared::test_fixtures::draft_with;

        let store = Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(std::env::temp_dir().join("athena-vm-trash-test")),
        );
        let mut older_draft = draft_with(vec![]);
        older_draft.title = "Older".into();
        let older_id = transactions_repo::create(store.db().conn(), &older_draft).unwrap();
        let mut newer_draft = draft_with(vec![]);
        newer_draft.title = "Newer".into();
        let newer_id = transactions_repo::create(store.db().conn(), &newer_draft).unwrap();
        store
            .db()
            .conn()
            .execute(
                "UPDATE transactions SET deleted_at = '2026-11-10T10:00:00Z' WHERE id = ?1",
                [older_id],
            )
            .unwrap();
        store
            .db()
            .conn()
            .execute(
                "UPDATE transactions SET deleted_at = '2026-11-12T10:00:00Z' WHERE id = ?1",
                [newer_id],
            )
            .unwrap();

        let mut ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        let mut vm = ViewModel::new(Language::En);
        vm.today = chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();

        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.trash_count, 2);
        assert!(vm.trash.is_empty(), "trash does not load off-screen");
        assert_eq!(vm.trash_loads, 0);

        ui.set_screen(Screen::Trash);
        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.trash_loads, 1);
        let titles: Vec<_> = vm.trash.iter().map(|t| t.tx.title.as_str()).collect();
        assert_eq!(titles, vec!["Newer", "Older"]);
    }

    #[test]
    fn settlement_overflow_keeps_app_loadable_and_other_fields_coherent() {
        use crate::app::actions::AppAction;
        use crate::domain::transaction::{Allocation, TransactionType};
        use crate::shared::test_fixtures::{date, draft_with, seed_people};

        let mut store = Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(std::env::temp_dir().join("athena-vm-overflow-test")),
        );
        let (owner, partner) = seed_people(store.db().conn());
        let mut expense = draft_with(vec![Allocation {
            person_id: partner,
            amount: Money::from_cents(i64::MAX / 2 + 100),
        }]);
        expense.title = "Huge expense".into();
        expense.kind = TransactionType::Expense;
        expense.payer_id = Some(owner);
        expense.date = date(2026, 11, 10);
        store.apply(AppAction::SaveTransaction(expense)).unwrap();

        // Settlement is excluded from reports_repo SUM, so only settlement::for_month overflows.
        let mut settlement_tx = draft_with(vec![Allocation {
            person_id: partner,
            amount: Money::from_cents(i64::MAX / 2 + 100),
        }]);
        settlement_tx.title = "Huge settlement".into();
        settlement_tx.kind = TransactionType::Settlement;
        settlement_tx.payer_id = Some(owner);
        settlement_tx.category_id = None;
        settlement_tx.split_mode = None;
        settlement_tx.date = date(2026, 11, 11);
        store
            .apply(AppAction::SaveTransaction(settlement_tx))
            .unwrap();

        let ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        let mut vm = ViewModel::new(Language::En);
        vm.today = date(2026, 11, 16);

        assert!(vm.sync(&store, &ui).unwrap());
        assert!(
            !vm.sync(&store, &ui).unwrap(),
            "must not retry every frame for same key"
        );
        assert_eq!(vm.transactions.len(), 2);
        assert!(vm.settlement.is_none());
        assert!(vm.settlement_unavailable);
        assert!(!vm.refresh_failed);
    }

    #[test]
    fn required_query_failure_stages_without_half_mutation_and_suppresses_same_key_retry() {
        use crate::app::actions::AppAction;
        use crate::application::services::onboarding::OnboardingResult;
        use crate::shared::test_fixtures::{date, draft_with};

        let mut store = Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(std::env::temp_dir().join("athena-vm-required-err-test")),
        );
        store
            .apply(AppAction::CompleteOnboarding(OnboardingResult {
                display_name: "A3".into(),
                partner_name: None,
                start_balance: Money::ZERO,
                language: Language::En,
            }))
            .unwrap();
        let mut expense = draft_with(vec![]);
        expense.amount = Money::from_cents(100);
        expense.date = date(2026, 11, 10);
        store.apply(AppAction::SaveTransaction(expense)).unwrap();

        let ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        let mut vm = ViewModel::new(Language::En);
        vm.today = date(2026, 11, 16);
        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.profile.as_ref().unwrap().start_balance, Money::ZERO);

        // SaveProfile updates start_balance to i64::MIN, causing tracked_balance (i64::MIN - 100)
        // to fail AFTER profile_repo::load succeeds inside reload.
        let mut bad_profile = vm.profile.clone().unwrap();
        bad_profile.start_balance = Money::from_cents(i64::MIN);
        store.apply(AppAction::SaveProfile(bad_profile)).unwrap();

        let err = vm.sync(&store, &ui).unwrap_err();
        assert!(matches!(err, crate::shared::error::AppError::Validation(_)));
        assert!(vm.refresh_failed);
        // profile must NOT be half-mutated to i64::MIN when tracked_balance failed.
        assert_eq!(vm.profile.as_ref().unwrap().start_balance, Money::ZERO);
        // Same-key retry must be suppressed to avoid per-frame query/log loops.
        assert!(!vm.sync(&store, &ui).unwrap());

        // Fixing start_balance bumps data_version (new LoadKey), clearing refresh_failed on sync.
        let mut good_profile = vm.profile.clone().unwrap();
        good_profile.start_balance = Money::from_cents(500);
        store.apply(AppAction::SaveProfile(good_profile)).unwrap();
        assert!(vm.sync(&store, &ui).unwrap());
        assert!(!vm.refresh_failed);
        assert_eq!(vm.balance, Money::from_cents(400));
    }

    #[cfg(test)]
    fn seed_fifty_thousand_rows(conn: &rusqlite::Connection) {
        use crate::application::services::onboarding::{self, OnboardingResult};
        use crate::infrastructure::db::people_repo;

        onboarding::complete(
            conn,
            &OnboardingResult {
                display_name: "Owner".into(),
                partner_name: Some("Partner".into()),
                start_balance: Money::ZERO,
                language: Language::En,
            },
        )
        .unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        let people = people_repo::list(&tx, true).unwrap();
        let owner = people
            .iter()
            .find(|p| p.kind == PersonKind::Owner)
            .unwrap()
            .id;
        let partner = people
            .iter()
            .find(|p| p.kind == PersonKind::Partner)
            .unwrap()
            .id;
        let expense_cat: i64 = tx
            .query_row(
                "SELECT id FROM categories WHERE applies_to IN ('expense', 'both') ORDER BY id LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();

        let mut tx_stmt = tx
            .prepare_cached(
                "INSERT INTO transactions (date, title, amount_cents, kind, category_id, payer_id, split_mode, description, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '', '2026-11-16T00:00:00Z', '2026-11-16T00:00:00Z')",
            )
            .unwrap();
        let mut alloc_stmt = tx
            .prepare_cached(
                "INSERT INTO allocations (transaction_id, person_id, amount_cents) VALUES (?1, ?2, ?3)",
            )
            .unwrap();

        for i in 0_i64..50_000 {
            let year = 2024 + (i % 3);
            let month = 1 + ((i / 3) % 12);
            let day = 1 + ((i / 36) % 28);
            let date_str = format!("{year:04}-{month:02}-{day:02}");
            let payer = if i % 2 == 0 { owner } else { partner };
            let other = if i % 2 == 0 { partner } else { owner };
            let (title, amount, kind, cat, split, allocs) = match i % 10 {
                0 => (
                    format!("Payback {i}"),
                    1_000_i64,
                    "settlement",
                    None::<i64>,
                    None::<&str>,
                    vec![(other, 1_000_i64)],
                ),
                1 => (
                    format!("Refund {i}"),
                    800_i64,
                    "refund",
                    Some(expense_cat),
                    Some("even"),
                    vec![(owner, 400_i64), (partner, 400_i64)],
                ),
                _ => (
                    format!("Expense {i}"),
                    2_000_i64,
                    "expense",
                    Some(expense_cat),
                    Some("even"),
                    vec![(owner, 1_000_i64), (partner, 1_000_i64)],
                ),
            };
            tx_stmt
                .execute(rusqlite::params![
                    date_str, title, amount, kind, cat, payer, split
                ])
                .unwrap();
            let tx_id = tx.last_insert_rowid();
            for (person, share) in allocs {
                alloc_stmt
                    .execute(rusqlite::params![tx_id, person, share])
                    .unwrap();
            }
        }
        drop(tx_stmt);
        drop(alloc_stmt);
        tx.commit().unwrap();
    }

    #[test]
    #[ignore = "Task R-6 50,000-row performance triage benchmark"]
    fn benchmark_fifty_thousand_rows_settlement_and_sync() {
        use std::time::Instant;

        use crate::infrastructure::db::settlement_repo;

        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let db = Db::open(&dirs.db_path).unwrap();
        seed_fifty_thousand_rows(db.conn());

        let target_month = YearMonth::new(2026, 12).unwrap();

        let t0 = Instant::now();
        let inputs = settlement_repo::list_inputs(db.conn(), target_month.last_day()).unwrap();
        let t_list = t0.elapsed();
        let t1 = Instant::now();
        let calc = crate::domain::settlement::calculate(target_month.last_day(), &inputs).unwrap();
        let t_calc = t1.elapsed();
        let t2 = Instant::now();
        let view = settlement::for_month(db.conn(), target_month).unwrap();
        let t_for_month = t2.elapsed();
        assert_eq!(inputs.len(), 50_000);
        assert_eq!(calc.balances, view.balances);

        let store = Store::new(db, dirs);
        let mut ui = UiState::new(target_month);
        let mut vm = ViewModel::new(Language::En);
        vm.today = chrono::NaiveDate::from_ymd_opt(2026, 12, 15).unwrap();

        let t_init_start = Instant::now();
        assert!(vm.sync(&store, &ui).unwrap());
        let t_init_sync = t_init_start.elapsed();

        let t_trash_start = Instant::now();
        ui.set_screen(Screen::Trash);
        assert!(vm.sync(&store, &ui).unwrap());
        let t_to_trash = t_trash_start.elapsed();

        let t_settings_start = Instant::now();
        ui.set_screen(Screen::Settings);
        assert!(vm.sync(&store, &ui).unwrap());
        let t_to_settings = t_settings_start.elapsed();

        let t_month_start = Instant::now();
        ui.set_screen(Screen::Month);
        assert!(vm.sync(&store, &ui).unwrap());
        let t_to_month = t_month_start.elapsed();

        let t_search_start = Instant::now();
        ui.edit_search("Expense 499".into(), 1.0);
        assert!(ui.commit_search_if_due(1.5));
        assert!(vm.sync(&store, &ui).unwrap());
        let t_search_sync = t_search_start.elapsed();

        println!(
            "R-6 50k benchmark: list_inputs={t_list:?}, calculate={t_calc:?}, for_month={t_for_month:?}, init_sync={t_init_sync:?}, Month->Trash={t_to_trash:?}, Trash->Settings={t_to_settings:?}, Settings->Month={t_to_month:?}, search_sync={t_search_sync:?}"
        );
    }

    #[test]
    fn incremental_sync_updates_only_affected_slices_on_filter_and_screen_changes() {
        use crate::app::actions::AppAction;
        use crate::domain::transaction::Allocation;
        use crate::shared::test_fixtures::{date, draft_with, seed_people};

        let mut store = Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(std::env::temp_dir().join("athena-vm-incremental-sync-test")),
        );
        let (owner, partner) = seed_people(store.db().conn());
        let mut d1 = draft_with(vec![Allocation {
            person_id: partner,
            amount: Money::from_cents(2_000),
        }]);
        d1.title = "Coffee".into();
        d1.payer_id = Some(owner);
        d1.date = date(2026, 11, 5);
        store.apply(AppAction::SaveTransaction(d1)).unwrap();

        let mut d2 = draft_with(vec![]);
        d2.title = "Bakery".into();
        d2.date = date(2026, 11, 6);
        let d2_id = transactions_repo::create(store.db().conn(), &d2).unwrap();
        transactions_repo::delete(store.db().conn(), d2_id).unwrap();

        let mut ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        let mut vm = ViewModel::new(Language::En);
        vm.today = date(2026, 11, 16);

        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.transactions.len(), 1);
        assert_eq!(vm.trash_count, 1);
        assert!(vm.trash.is_empty());
        assert!(vm.settlement.is_some());

        // Search filter update refreshes transactions cleanly without dropping settlement or loading trash.
        ui.edit_search("Nomatch".into(), 1.0);
        assert!(ui.commit_search_if_due(1.5));
        assert!(vm.sync(&store, &ui).unwrap());
        assert!(vm.transactions.is_empty());
        assert!(vm.settlement.is_some());
        assert_eq!(vm.trash_loads, 0);

        ui.edit_search(String::new(), 2.0);
        assert!(ui.commit_search_if_due(2.5));
        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.transactions.len(), 1);

        // Screen switch to Trash loads trash, leaving Trash clears vm.trash, returning to Month preserves state.
        ui.set_screen(Screen::Trash);
        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.trash.len(), 1);
        assert_eq!(vm.trash_loads, 1);

        ui.set_screen(Screen::Settings);
        assert!(vm.sync(&store, &ui).unwrap());
        assert!(vm.trash.is_empty());
        assert_eq!(vm.trash_loads, 1);

        ui.set_screen(Screen::Month);
        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.transactions.len(), 1);
        assert!(vm.settlement.is_some());
        assert_eq!(vm.trash_loads, 1);
    }

    #[test]
    fn create_backup_action_refreshes_backups_scope() {
        use crate::app::AppCore;
        use crate::app::actions::{AppAction, OperationKind};
        use crate::i18n::ToastKey;

        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let mut core = AppCore::new(Db::open(&dirs.db_path).unwrap(), dirs).unwrap();

        // Off-screen: backups list stays unloaded on Month screen.
        assert!(core.vm.backups.is_empty());

        // Switching to Settings loads existing backups.
        let outcome = core.apply(AppAction::CreateBackup).unwrap();
        assert_eq!(outcome.toast, Some(ToastKey::BackupCreated));
        let notice = outcome
            .notice
            .expect("CreateBackup records OperationNotice");
        assert_eq!(notice.kind, OperationKind::BackupCreated);
        let snap_path = notice.snapshot.expect("snapshot path recorded");
        assert!(snap_path.exists());
        assert_eq!(
            core.vm.last_operation.as_ref().map(|n| n.kind),
            Some(OperationKind::BackupCreated)
        );
        // Still on Month screen: backups list only loads on Screen::Settings.
        assert!(core.vm.backups.is_empty());

        core.set_screen(Screen::Settings);
        assert!(core.sync().unwrap());
        // An unfinished setup has no daily snapshot: only the manual backup exists.
        assert_eq!(core.vm.backups.len(), 1);
        assert!(core.vm.backups.iter().any(|b| b.path == snap_path));

        // Creating another backup while on Settings refreshes vm.backups directly via RefreshScope::Backups.
        let outcome2 = core.apply(AppAction::CreateBackup).unwrap();
        let snap2 = outcome2.notice.and_then(|n| n.snapshot).unwrap();
        assert_eq!(core.vm.backups.len(), 2);
        assert!(core.vm.backups.iter().any(|b| b.path == snap2));
    }
}
