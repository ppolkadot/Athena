//! Cached read data for the screens. Screens read this and never query the database. It reloads
//! when the store's `data_version` or the selected month, year or filter changes.

use super::actions::{OperationNotice, RefreshScope};
use super::navigation::Screen;
use super::store::Store;
use super::ui_state::UiState;
use crate::application::services::reports;
use crate::domain::category::Category;
use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::person::{Person, PersonKind};
use crate::domain::reports::{CalendarMonthTotal, CategoryTotal, MonthSummary, PersonTotal};
use crate::domain::settings::{Language, Profile, Theme};
use crate::domain::transaction::{Transaction, TransactionType, TxFilter};
use crate::infrastructure::db::{
    categories_repo, people_repo, profile_repo, transactions_repo, trash_repo,
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
    /// Number of deleted transactions currently in Trash.
    pub trash_count: usize,
    /// Most recent backup/restore/import/purge operation notice in this session.
    pub last_operation: Option<OperationNotice>,
    /// Where the database file lives, for the settings screen.
    pub data_path: String,
    /// The local date, kept here so screens never read the clock and tests can pin it.
    pub today: chrono::NaiveDate,
    #[cfg(test)]
    pub(crate) trash_loads: usize,
    loaded: Option<LoadKey>,
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
            trash_count: 0,
            last_operation: None,
            data_path: String::new(),
            today: chrono::Local::now().date_naive(),
            #[cfg(test)]
            trash_loads: 0,
            loaded: None,
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
            filter: TxFilter {
                month: Some(ui.month),
                ..ui.filter.clone()
            },
            today: self.today,
            screen: ui.screen,
        }
    }

    #[cfg_attr(
        not(test),
        allow(clippy::unused_self, clippy::needless_pass_by_ref_mut)
    )]
    fn load_trash_screen_stub(&mut self) {
        #[cfg(test)]
        {
            self.trash_loads += 1;
        }
    }

    /// Reload everything if the store or the selection changed since the last load.
    /// Returns whether it reloaded.
    ///
    /// # Errors
    /// Database errors.
    pub fn sync(&mut self, store: &Store, ui: &UiState) -> Result<bool> {
        if self.loaded.as_ref() == Some(&self.key(store, ui)) {
            return Ok(false);
        }
        self.reload(store, ui, RefreshScope::All)?;
        Ok(true)
    }

    /// Reload what `scope` covers. People and category changes reload only those lists.
    ///
    /// # Errors
    /// Database errors.
    pub fn reload(&mut self, store: &Store, ui: &UiState, scope: RefreshScope) -> Result<()> {
        let conn = store.db().conn();
        match scope {
            RefreshScope::People => self.all_people = people_repo::list(conn, true)?,
            RefreshScope::Categories => self.all_categories = categories_repo::list(conn, true)?,
            RefreshScope::Trash => {
                self.trash_count = trash_repo::count(conn)?;
                self.load_trash_screen_stub();
            }
            RefreshScope::Backups => {}
            RefreshScope::Month | RefreshScope::Calendar | RefreshScope::All => {
                let key = self.key(store, ui);
                self.profile = profile_repo::load(conn)?;
                self.all_people = people_repo::list(conn, true)?;
                self.all_categories = categories_repo::list(conn, true)?;
                self.transactions = transactions_repo::list(conn, &key.filter)?;
                self.summary = reports::month_summary(conn, ui.month)?;
                self.category_totals = reports::category_totals(conn, ui.month)?;
                self.person_totals = reports::person_totals(conn, ui.month)?;
                self.calendar = reports::year_calendar(conn, ui.calendar_year)?;
                let start = self
                    .profile
                    .as_ref()
                    .map_or(Money::ZERO, |p| p.start_balance);
                self.balance = reports::tracked_balance(conn, start)?;
                self.trash_count = trash_repo::count(conn)?;
                self.data_path = store.dirs().db_path.display().to_string();
                if ui.screen == Screen::Trash {
                    self.load_trash_screen_stub();
                }
            }
        }
        self.loaded = Some(self.key(store, ui));
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
}
