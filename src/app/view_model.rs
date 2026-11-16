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
use crate::domain::transaction::{Transaction, TransactionType, TxFilter};
use crate::infrastructure::db::{
    categories_repo, people_repo, profile_repo, transactions_repo,
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
    /// Most recent backup/restore/import/purge operation notice in this session.
    pub last_operation: Option<OperationNotice>,
    /// Where the database file lives, for the settings screen.
    pub data_path: String,
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
            last_operation: None,
            data_path: String::new(),
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
    /// Returns whether it reloaded.
    ///
    /// # Errors
    /// Database errors.
    pub fn sync(&mut self, store: &Store, ui: &UiState) -> Result<bool> {
        let key = self.key(store, ui);
        if self.loaded.as_ref() == Some(&key) || self.failed_load_key.as_ref() == Some(&key) {
            return Ok(false);
        }
        self.reload(store, ui, RefreshScope::All)?;
        Ok(true)
    }

    /// Reload what `scope` covers. People and category changes reload only those lists.
    /// Stages all required queries before mutating `self` so an error never leaves half-updated
    /// state, and isolates optional settlement math failures into `settlement_unavailable`.
    ///
    /// # Errors
    /// Database or validation errors on required queries.
    pub fn reload(&mut self, store: &Store, ui: &UiState, scope: RefreshScope) -> Result<()> {
        let key = self.key(store, ui);
        match self.load_scope(store, ui, scope, &key) {
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
            RefreshScope::Trash => {
                let trash_count = trash_repo::count(conn)?;
                let trash = trash_repo::list(conn)?;
                self.trash_count = trash_count;
                self.trash = trash;
                self.record_trash_load();
            }
            RefreshScope::Backups => {}
            RefreshScope::Month | RefreshScope::Calendar | RefreshScope::All => {
                let profile = profile_repo::load(conn)?;
                let all_people = people_repo::list(conn, true)?;
                let all_categories = categories_repo::list(conn, true)?;
                let transactions = transactions_repo::list(conn, &key.filter)?;
                let summary = reports::month_summary(conn, ui.month)?;
                let category_totals = reports::category_totals(conn, ui.month)?;
                let person_totals = reports::person_totals(conn, ui.month)?;
                let calendar = reports::year_calendar(conn, ui.calendar_year)?;
                let start = profile.as_ref().map_or(Money::ZERO, |p| p.start_balance);
                let balance = reports::tracked_balance(conn, start)?;
                let trash_count = trash_repo::count(conn)?;
                let data_path = store.dirs().db_path.display().to_string();
                let (trash, trash_loaded) = if ui.screen == Screen::Trash {
                    (trash_repo::list(conn)?, true)
                } else {
                    (Vec::new(), false)
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
                self.summary = summary;
                self.category_totals = category_totals;
                self.person_totals = person_totals;
                self.calendar = calendar;
                self.balance = balance;
                self.settlement = settlement;
                self.settlement_unavailable = settlement_unavailable;
                self.trash_count = trash_count;
                self.data_path = data_path;
                self.trash = trash;
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
}
