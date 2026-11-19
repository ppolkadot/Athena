//! Cached read data for the screens. Screens read this and never query the database. It reloads
//! when the store's `data_version` or the selected month, year or filter changes.

use super::actions::{OperationNotice, RefreshScope};
use super::navigation::Screen;
use super::store::Store;
use super::ui_state::UiState;
use crate::application::services::closings::{self, FingerprintCache};
use crate::application::services::goals::{self, GoalDetail, GoalListItem};
use crate::application::services::{reports, settlement};
use crate::domain::appearance::AppearancePrefs;
use crate::domain::category::Category;
use crate::domain::closing::{ClosingSnapshot, ClosingStatus};
use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::person::{Person, PersonKind};
use crate::domain::reports::{
    CalendarMonthTotal, CategoryTotal, DayActivity, InsightInputs, MonthSummary, PersonTotal,
};
use crate::domain::settings::{Language, Profile, Theme};
use crate::domain::settlement::SettlementView;
use crate::domain::template::Template;
use crate::domain::transaction::{Transaction, TransactionType, TxFilter};
use crate::infrastructure::db::{
    appearance_repo,
    backup::{self, BackupFile},
    categories_repo, closings_repo, people_repo, profile_repo, templates_repo, transactions_repo,
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
    /// The month whose day grid is shown, if any.
    grid_month: Option<YearMonth>,
    /// The selected day, if any.
    day: Option<chrono::NaiveDate>,
    /// The open goal, whose detail is cached.
    goal: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ViewModel {
    fallback_language: Language,
    pub profile: Option<Profile>,
    /// The applied appearance row; `None` means defaults derived from the profile theme.
    pub appearance: Option<AppearancePrefs>,
    /// The stored appearance row exists but could not be read (solid defaults are in use).
    pub appearance_unreadable: bool,
    /// The theme the shell paints with, once the app resolved it; the profile theme until then.
    effective_theme: Option<Theme>,
    /// Archived people included, so history labels always resolve.
    pub all_people: Vec<Person>,
    /// Archived categories included.
    pub all_categories: Vec<Category>,
    pub transactions: Vec<Transaction>,
    /// Entry templates, alphabetical by folded name then id.
    pub templates: Vec<Template>,
    /// Every goal with its progress, in stable id order.
    pub goals: Vec<GoalListItem>,
    /// Every saved monthly closing by month.
    pub closings: std::collections::BTreeMap<YearMonth, ClosingSnapshot>,
    /// Months whose saved largest expense is still a live entry (so it can be opened).
    pub closing_live_expense: std::collections::HashSet<YearMonth>,
    /// Whether the ledger changed since each closing the open screen shows: the viewed month on
    /// Month, the displayed year on Calendar. Computed on data, month, year and screen changes,
    /// never per frame and never for a filter change.
    pub closing_status: std::collections::HashMap<YearMonth, ClosingStatus>,
    /// The selected goal with its contributions, if one is selected and exists.
    pub goal_detail: Option<GoalDetail>,
    pub summary: MonthSummary,
    pub category_totals: Vec<CategoryTotal>,
    pub person_totals: Vec<PersonTotal>,
    pub calendar: Vec<CalendarMonthTotal>,
    /// Days with live activity in the displayed grid month (sparse; the display zero-fills).
    pub day_activity: Vec<DayActivity>,
    /// Every live row (any kind) on the selected day.
    pub day_rows: Vec<Transaction>,
    /// Structured insight inputs for the viewed month; `None` if they could not be read.
    pub insight: Option<InsightInputs>,
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
    #[cfg(test)]
    pub(crate) goal_list_loads: usize,
    #[cfg(test)]
    pub(crate) goal_detail_loads: usize,
    closing_cache: FingerprintCache,
    closing_cache_version: u64,
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
            appearance: None,
            appearance_unreadable: false,
            effective_theme: None,
            all_people: Vec::new(),
            all_categories: Vec::new(),
            transactions: Vec::new(),
            templates: Vec::new(),
            goals: Vec::new(),
            closings: std::collections::BTreeMap::new(),
            closing_live_expense: std::collections::HashSet::new(),
            closing_status: std::collections::HashMap::new(),
            closing_cache: FingerprintCache::default(),
            closing_cache_version: 0,
            goal_detail: None,
            summary: MonthSummary::default(),
            category_totals: Vec::new(),
            person_totals: Vec::new(),
            calendar: Vec::new(),
            day_activity: Vec::new(),
            day_rows: Vec::new(),
            insight: None,
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
            #[cfg(test)]
            goal_list_loads: 0,
            #[cfg(test)]
            goal_detail_loads: 0,
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

    /// The theme the shell paints with: the resolved appearance theme, else the legacy profile theme.
    #[must_use]
    pub fn theme(&self) -> Theme {
        self.effective_theme.unwrap_or_else(|| self.profile_theme())
    }

    /// The legacy profile theme (the fallback when no appearance is applied).
    #[must_use]
    pub fn profile_theme(&self) -> Theme {
        self.profile.as_ref().map_or(Theme::Dark, |p| p.theme)
    }

    /// Record the theme the app resolved this frame; `None` returns to the profile theme.
    pub fn set_effective_theme(&mut self, theme: Option<Theme>) {
        self.effective_theme = theme;
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
            grid_month: ui.grid_month(),
            day: ui.selected_day,
            goal: ui.goals.selected,
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
            if prev.filter == key.filter
                && prev.screen == key.screen
                && prev.grid_month == key.grid_month
                && prev.day == key.day
            {
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
            let (day_activity, day_rows) = read_days(conn, prev, &key)?;
            let goal_detail = if prev.goal == key.goal {
                None
            } else {
                Some(read_goal_detail(conn, key.goal)?)
            };
            Ok((
                transactions,
                trash,
                backups,
                day_activity,
                day_rows,
                goal_detail,
            ))
        })();

        match staged {
            Ok((transactions, trash, backups, day_activity, day_rows, goal_detail)) => {
                if let Some(txs) = transactions {
                    self.transactions = txs;
                }
                if let Some(detail) = goal_detail {
                    self.goal_detail = detail;
                    self.record_goal_detail_load();
                }
                if prev.screen != key.screen {
                    self.refresh_closing_status(store, ui, key.version);
                }
                self.stage_days(day_activity, day_rows);
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
                self.fail_day_cache();
                self.failed_load_key = Some(key);
                self.refresh_failed = true;
                Err(err)
            }
        }
    }

    fn stage_days(&mut self, activity: Option<Vec<DayActivity>>, rows: Option<Vec<Transaction>>) {
        if let Some(activity) = activity {
            self.day_activity = activity;
        }
        if let Some(rows) = rows {
            self.day_rows = rows;
        }
    }

    /// Optional read: an unreadable row falls back to defaults and never fails the workspace.
    fn stage_appearance(&mut self, read: std::result::Result<Option<AppearancePrefs>, Unreadable>) {
        self.appearance_unreadable = read.is_err();
        self.appearance = read.unwrap_or(None);
    }

    #[cfg_attr(
        not(test),
        allow(clippy::unused_self, clippy::needless_pass_by_ref_mut)
    )]
    fn record_goal_detail_load(&mut self) {
        #[cfg(test)]
        {
            self.goal_detail_loads += 1;
        }
    }

    #[cfg_attr(
        not(test),
        allow(clippy::unused_self, clippy::needless_pass_by_ref_mut)
    )]
    fn record_goal_list_load(&mut self) {
        #[cfg(test)]
        {
            self.goal_list_loads += 1;
        }
    }

    /// Recompute the closing statuses the open screen needs: the viewed month on Month, the
    /// displayed year on Calendar. Fingerprints are cached per ledger version, so adjacent months
    /// and repeated checks reuse each other; a failed check shows as unavailable.
    fn refresh_closing_status(&mut self, store: &Store, ui: &UiState, version: u64) {
        if self.closing_cache_version != version {
            self.closing_cache.clear();
            self.closing_cache_version = version;
        }
        let months: Vec<YearMonth> = match ui.screen {
            Screen::Month => vec![ui.month],
            Screen::Calendar => (1..=12)
                .filter_map(|m| YearMonth::new(ui.calendar_year, m))
                .collect(),
            _ => Vec::new(),
        };
        self.closing_status = closings::statuses(
            store.db().conn(),
            &mut self.closing_cache,
            &self.closings,
            &months,
        );
    }

    /// Run unavailable closing checks again (the Retry control).
    pub fn retry_closing_checks(&mut self, store: &Store, ui: &UiState) {
        self.closing_cache.clear();
        self.refresh_closing_status(store, ui, store.data_version());
    }

    /// A failed read leaves no day detail behind to be mistaken for current data.
    fn fail_day_cache(&mut self) {
        self.day_activity.clear();
        self.day_rows.clear();
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
                self.refresh_closing_status(store, ui, key.version);
                self.loaded = Some(key);
                self.failed_load_key = None;
                self.refresh_failed = false;
                Ok(())
            }
            Err(err) => {
                self.fail_day_cache();
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
                self.fail_day_cache();
                self.failed_load_key = Some(key);
                self.refresh_failed = true;
                Err(err)
            }
        }
    }

    #[allow(clippy::too_many_lines)]
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
            RefreshScope::Appearance => {
                self.stage_appearance(read_appearance(conn));
            }
            RefreshScope::Closings => {
                let (list, live) = read_closings(conn)?;
                self.closings = list;
                self.closing_live_expense = live;
                self.refresh_closing_status(store, ui, key.version);
            }
            RefreshScope::Goals => {
                let list = goals::list_items(conn)?;
                let detail = read_goal_detail(conn, key.goal)?;
                self.goals = list;
                self.goal_detail = detail;
                self.record_goal_list_load();
                self.record_goal_detail_load();
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
                let appearance = (scope == RefreshScope::All).then(|| read_appearance(conn));
                let summary = reports::month_summary(conn, ui.month)?;
                let category_totals = reports::category_totals(conn, ui.month)?;
                let person_totals = reports::person_totals(conn, ui.month)?;
                let calendar = reports::year_calendar(conn, ui.calendar_year)?;
                let (day_activity, day_rows) = read_all_days(conn, key)?;
                let insight = read_insight(conn, ui.month, key.today);
                let goal_list = goals::list_items(conn)?;
                let goal_detail = read_goal_detail(conn, key.goal)?;
                let (closing_list, closing_live) = read_closings(conn)?;
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
                if let Some(appearance) = appearance {
                    self.stage_appearance(appearance);
                }
                self.summary = summary;
                self.category_totals = category_totals;
                self.person_totals = person_totals;
                self.calendar = calendar;
                self.day_activity = day_activity;
                self.day_rows = day_rows;
                self.insight = insight;
                self.goals = goal_list;
                self.closings = closing_list;
                self.closing_live_expense = closing_live;
                self.refresh_closing_status(store, ui, key.version);
                self.goal_detail = goal_detail;
                self.record_goal_list_load();
                self.record_goal_detail_load();
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

/// Every saved closing and which of them still point at a live largest-expense entry.
fn read_closings(
    conn: &rusqlite::Connection,
) -> Result<(
    std::collections::BTreeMap<YearMonth, ClosingSnapshot>,
    std::collections::HashSet<YearMonth>,
)> {
    let list = closings::all(conn)?;
    let mut live = std::collections::HashSet::new();
    for (month, snapshot) in &list {
        if let Some(id) = snapshot.largest_expense.as_ref().and_then(|e| e.id)
            && closings_repo::is_live_transaction(conn, id)?
        {
            live.insert(*month);
        }
    }
    Ok((list, live))
}

/// The open goal's detail; `None` when nothing is selected or the goal no longer exists.
fn read_goal_detail(conn: &rusqlite::Connection, goal: Option<i64>) -> Result<Option<GoalDetail>> {
    match goal {
        Some(id) => goals::detail(conn, id),
        None => Ok(None),
    }
}

/// The appearance row. `None` means it could not be read; it is logged without row content.
fn read_appearance(
    conn: &rusqlite::Connection,
) -> std::result::Result<Option<AppearancePrefs>, Unreadable> {
    match appearance_repo::load(conn) {
        Ok(prefs) => Ok(prefs),
        Err(err) => {
            tracing::warn!(
                "stored appearance preferences are unreadable ({}); using defaults",
                match err {
                    crate::shared::error::AppError::Db(_) => "invalid or unreadable row",
                    _ => "unexpected failure",
                }
            );
            Err(Unreadable)
        }
    }
}

/// The stored appearance row could not be read.
#[derive(Debug, Clone, Copy)]
struct Unreadable;

/// The month's insight inputs; a failed read hides the card instead of failing the screen.
fn read_insight(
    conn: &rusqlite::Connection,
    month: YearMonth,
    today: chrono::NaiveDate,
) -> Option<InsightInputs> {
    match reports::insight_inputs(conn, month, today) {
        Ok(inputs) => Some(inputs),
        Err(err) => {
            tracing::warn!("insight unavailable: {err}");
            None
        }
    }
}

/// The day grid and day detail to restage, each only when it changed.
type DayReads = (Option<Vec<DayActivity>>, Option<Vec<Transaction>>);

/// The whole day cache for a full reload.
fn read_all_days(
    conn: &rusqlite::Connection,
    key: &LoadKey,
) -> Result<(Vec<DayActivity>, Vec<Transaction>)> {
    let activity = match key.grid_month {
        Some(month) => reports::day_activity(conn, month)?,
        None => Vec::new(),
    };
    Ok((activity, day_rows(conn, key.day)?))
}

/// What the day cache needs after a selection change: selecting a day refreshes only the detail,
/// and a text filter change refreshes neither.
fn read_days(conn: &rusqlite::Connection, prev: &LoadKey, key: &LoadKey) -> Result<DayReads> {
    let grid_changed = prev.grid_month != key.grid_month;
    let activity = if grid_changed {
        Some(match key.grid_month {
            Some(month) => reports::day_activity(conn, month)?,
            None => Vec::new(),
        })
    } else {
        None
    };
    let rows = if grid_changed || prev.day != key.day {
        Some(day_rows(conn, key.day)?)
    } else {
        None
    };
    Ok((activity, rows))
}

/// Every live row of any kind on `day`, newest id last; empty when no day is selected.
fn day_rows(
    conn: &rusqlite::Connection,
    day: Option<chrono::NaiveDate>,
) -> Result<Vec<Transaction>> {
    let Some(day) = day else {
        return Ok(Vec::new());
    };
    let mut rows = transactions_repo::list(
        conn,
        &TxFilter {
            day: Some(day),
            ..TxFilter::default()
        },
    )?;
    rows.sort_by_key(|t| t.id);
    Ok(rows)
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

    /// A store with salary, a market expense, a refund and a deleted row across two days.
    fn day_store(name: &str) -> (Store, i64) {
        use crate::app::actions::AppAction;
        use crate::domain::transaction::TransactionType;
        use crate::shared::test_fixtures::{date, draft_with};

        let mut store = Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(std::env::temp_dir().join(name)),
        );
        let mut add = |title: &str, kind, cents, day| {
            let mut d = draft_with(vec![]);
            d.title = title.into();
            d.kind = kind;
            d.amount = Money::from_cents(cents);
            d.date = date(2026, 11, day);
            store.apply(AppAction::SaveTransaction(d)).unwrap();
        };
        add("Salary", TransactionType::Income, 300_000, 5);
        add("Market", TransactionType::Expense, 4_000, 10);
        add("Refund", TransactionType::Refund, 1_000, 10);
        add("Gone", TransactionType::Expense, 700, 10);
        let gone = transactions_repo::list(store.db().conn(), &TxFilter::default())
            .unwrap()
            .iter()
            .find(|t| t.title == "Gone")
            .unwrap()
            .id;
        store.apply(AppAction::DeleteTransaction(gone)).unwrap();
        (store, gone)
    }

    fn month_ui() -> UiState {
        let mut ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        ui.month_days = true;
        ui
    }

    #[test]
    fn day_cache_separates_spending_heat_from_activity_count() {
        use crate::shared::test_fixtures::date;
        let (store, _) = day_store("athena-vm-day-cache");
        let mut ui = month_ui();
        let mut vm = ViewModel::new(Language::En);
        vm.today = date(2026, 11, 18);
        vm.sync(&store, &ui).unwrap();

        let on = |d| vm.day_activity.iter().find(|a| a.date == date(2026, 11, d));
        let salary = on(5).expect("a salary-only day is discoverable");
        assert_eq!(salary.activity_count, 1);
        assert_eq!(salary.spending, Money::ZERO, "income is not spending");
        let market = on(10).unwrap();
        assert_eq!(market.activity_count, 2, "the trashed row is not activity");
        assert_eq!(market.spending, Money::from_cents(3_000));
        assert!(vm.day_rows.is_empty(), "nothing selected yet");

        // Selecting a day loads that day's live rows, newest id last, trash excluded.
        ui.select_day(date(2026, 11, 10));
        vm.sync(&store, &ui).unwrap();
        let titles: Vec<_> = vm.day_rows.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["Market", "Refund"]);

        // An empty day is selectable and shows no rows.
        ui.select_day(date(2026, 11, 20));
        vm.sync(&store, &ui).unwrap();
        assert!(vm.day_rows.is_empty());

        // Changing month clears the selection and the grid follows the month.
        ui.select_day(date(2026, 11, 10));
        ui.set_month(YearMonth::new(2026, 12).unwrap());
        assert_eq!(ui.selected_day, None);
        vm.sync(&store, &ui).unwrap();
        assert!(vm.day_activity.is_empty() && vm.day_rows.is_empty());
    }

    #[test]
    fn day_grid_months_cross_december_january_and_leap_february() {
        use crate::shared::test_fixtures::{date, draft_with};
        let mut store = Store::new(
            Db::open_in_memory().unwrap(),
            AppDirs::from_root(std::env::temp_dir().join("athena-vm-day-edges")),
        );
        for (y, m, d) in [(2025, 12, 31), (2026, 1, 1), (2024, 2, 29)] {
            let mut draft = draft_with(vec![]);
            draft.date = date(y, m, d);
            store
                .apply(crate::app::actions::AppAction::SaveTransaction(draft))
                .unwrap();
        }
        let mut vm = ViewModel::new(Language::En);
        vm.today = date(2026, 1, 15);
        let mut ui = month_ui();
        for (month, last_day) in [
            (YearMonth::new(2025, 12).unwrap(), 31),
            (YearMonth::new(2026, 1).unwrap(), 31),
            (YearMonth::new(2024, 2).unwrap(), 29),
        ] {
            ui.set_month(month);
            vm.sync(&store, &ui).unwrap();
            let days = crate::domain::reports::zero_filled_days(month, &vm.day_activity);
            assert_eq!(days.len(), last_day, "{month:?}");
            assert_eq!(days.iter().map(|d| d.activity_count).sum::<i64>(), 1);
        }
    }

    #[test]
    fn midnight_reloads_the_insight_through_the_new_today() {
        use crate::domain::reports::MonthPhase;
        use crate::shared::test_fixtures::date;
        let (store, _) = day_store("athena-vm-insight-midnight");
        let ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        let mut vm = ViewModel::new(Language::En);
        vm.today = date(2026, 11, 7);
        vm.sync(&store, &ui).unwrap();
        // Through the 7th only the salary counts: the 10th is still ahead.
        let first = vm.insight.clone().unwrap();
        assert_eq!(first.current.count, 1);
        assert_eq!(first.phase, MonthPhase::ToDate);
        vm.today = date(2026, 11, 12);
        assert!(vm.sync(&store, &ui).unwrap(), "midnight changes the key");
        assert_eq!(vm.insight.as_ref().unwrap().current.count, 3);
        vm.today = date(2026, 12, 1);
        vm.sync(&store, &ui).unwrap();
        assert_eq!(vm.insight.as_ref().unwrap().phase, MonthPhase::Completed);
    }

    #[test]
    fn a_text_filter_change_reloads_only_the_register() {
        use crate::shared::test_fixtures::date;
        let (store, _) = day_store("athena-vm-filter-only");
        let mut ui = month_ui();
        let mut vm = ViewModel::new(Language::En);
        vm.today = date(2026, 11, 18);
        ui.select_day(date(2026, 11, 10));
        vm.sync(&store, &ui).unwrap();
        let (activity, rows) = (vm.day_activity.clone(), vm.day_rows.clone());
        ui.filter.text = "salary".into();
        assert!(vm.sync(&store, &ui).unwrap());
        assert_eq!(vm.transactions.len(), 1, "the register is filtered");
        assert_eq!(
            vm.day_activity, activity,
            "the grid ignores register filters"
        );
        assert_eq!(vm.day_rows, rows, "so does the day detail");
    }

    #[test]
    fn a_failed_day_read_clears_the_day_cache_and_suppresses_retry() {
        use crate::shared::test_fixtures::date;
        let (store, _) = day_store("athena-vm-day-fail");
        let mut ui = month_ui();
        let mut vm = ViewModel::new(Language::En);
        vm.today = date(2026, 11, 18);
        ui.select_day(date(2026, 11, 5));
        vm.sync(&store, &ui).unwrap();
        assert_eq!(vm.day_rows.len(), 1);
        // Corrupt the selected day's row so only the detail read fails.
        let conn = store.db().conn();
        conn.execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        conn.execute(
            "UPDATE transactions SET kind = 'bogus' WHERE title = 'Salary'",
            [],
        )
        .unwrap();
        ui.select_day(date(2026, 11, 10));
        vm.sync(&store, &ui).unwrap();
        ui.select_day(date(2026, 11, 5));
        assert!(vm.sync(&store, &ui).is_err());
        assert!(vm.refresh_failed);
        assert!(
            vm.day_rows.is_empty() && vm.day_activity.is_empty(),
            "no stale detail is left to be mistaken for current data"
        );
        assert!(
            !vm.sync(&store, &ui).unwrap(),
            "same key is not retried per frame"
        );
    }
}
