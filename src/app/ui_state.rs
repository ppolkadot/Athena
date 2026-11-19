//! Ephemeral UI state: which screen is showing, the selected month and year, filters.

use chrono::NaiveDate;

use super::navigation::Screen;
use crate::domain::date::YearMonth;
use crate::domain::transaction::{SettlementIssue, TxFilter};
use crate::ui::screens::settings::SettingsState;

#[derive(Debug, Clone)]
pub struct UiState {
    pub screen: Screen,
    pub month: YearMonth,
    pub calendar_year: i32,
    /// `filter.month` equals `Some(month)` unless `filter.issue` is set.
    pub filter: TxFilter,
    /// What is typed in the search box. It reaches `filter.text` after a short pause.
    pub search_input: String,
    search_edited_at: Option<f64>,
    /// Edits in progress on the settings screen.
    pub settings: SettingsState,
    /// Target row index to focus on Trash after restoring or purging an item.
    pub trash_focus_row: Option<usize>,
    /// Row index awaiting permanent-delete confirmation on Trash.
    pub pending_purge_focus_row: Option<usize>,
    /// Calendar: the month whose day grid is open instead of the year cards.
    pub calendar_days: Option<YearMonth>,
    /// Month: whether the day grid shows above the register.
    pub month_days: bool,
    /// The date whose live rows the day detail shows (any date, including empty days).
    pub selected_day: Option<NaiveDate>,
    /// The grid cell that takes keyboard focus; arrows move it within the month.
    pub day_cursor: Option<NaiveDate>,
    /// A widget to give keyboard focus to when it is next drawn (returning from the grid).
    pub pending_focus: Option<eframe::egui::Id>,
    /// Move focus to the day detail heading after a row disappears.
    pub day_heading_focus: bool,
}

/// How long typing must pause before the search runs, in seconds.
pub const SEARCH_DEBOUNCE: f64 = 0.2;

impl UiState {
    #[must_use]
    pub fn new(month: YearMonth) -> Self {
        Self {
            screen: Screen::default(),
            month,
            calendar_year: month.year,
            filter: TxFilter {
                month: Some(month),
                ..TxFilter::default()
            },
            search_input: String::new(),
            search_edited_at: None,
            settings: SettingsState::default(),
            trash_focus_row: None,
            pending_purge_focus_row: None,
            calendar_days: None,
            month_days: false,
            selected_day: None,
            day_cursor: None,
            pending_focus: None,
            day_heading_focus: false,
        }
    }

    /// The month whose day grid is cached: the Calendar's open month, or the viewed month while
    /// the Month screen shows its grid.
    #[must_use]
    pub fn grid_month(&self) -> Option<YearMonth> {
        match self.screen {
            Screen::Calendar => self.calendar_days,
            Screen::Month if self.month_days => Some(self.month),
            _ => None,
        }
    }

    /// Select a date for the day detail.
    pub fn select_day(&mut self, date: NaiveDate) {
        self.selected_day = Some(date);
        self.day_cursor = Some(date);
    }

    /// Forget the selection and the keyboard cursor.
    pub fn clear_day_selection(&mut self) {
        self.selected_day = None;
        self.day_cursor = None;
        self.day_heading_focus = false;
    }

    /// Leave every day view: used when the data is replaced (restore, import).
    pub fn clear_day_view(&mut self) {
        self.calendar_days = None;
        self.month_days = false;
        self.clear_day_selection();
    }

    pub fn set_screen(&mut self, screen: Screen) {
        if screen != self.screen {
            // A selected day belongs to the view that showed it.
            self.clear_day_selection();
        }
        if screen != Screen::Calendar {
            self.calendar_days = None;
        }
        if screen != Screen::Settings {
            // Leaving settings drops unsaved edits and subpage selection, so reopening starts fresh.
            self.settings.reset_all();
        }
        if screen != Screen::Trash {
            self.trash_focus_row = None;
            self.pending_purge_focus_row = None;
        }
        self.screen = screen;
    }

    /// Select a month. Filters that only make sense inside one month (day, added in 1.1) are
    /// cleared here so the register never looks empty after navigating.
    pub fn set_month(&mut self, month: YearMonth) {
        if month != self.month {
            self.clear_day_selection();
        }
        self.month = month;
        self.filter.issue = None;
        self.filter.until = None;
        self.filter.month = Some(month);
    }

    pub fn set_repair_filter(&mut self, issue: SettlementIssue, as_of: NaiveDate) {
        self.clear_day_selection();
        self.filter = TxFilter {
            month: None,
            until: Some(as_of),
            issue: Some(issue),
            ..TxFilter::default()
        };
        self.search_input.clear();
        self.search_edited_at = None;
    }

    pub fn clear_repair_filter(&mut self) {
        self.filter.issue = None;
        self.filter.until = None;
        self.filter.month = Some(self.month);
    }

    /// Record what is typed in the search box; the filter itself waits for a pause.
    pub fn edit_search(&mut self, text: String, now: f64) {
        self.search_input = text;
        self.search_edited_at = Some(now);
    }

    /// Whether typed search text is still waiting to be applied.
    #[must_use]
    pub fn search_pending(&self) -> bool {
        self.search_edited_at.is_some()
    }

    /// Apply typed search text once the user has paused. Returns whether the filter changed.
    pub fn commit_search_if_due(&mut self, now: f64) -> bool {
        match self.search_edited_at {
            Some(at) if now - at >= SEARCH_DEBOUNCE => {
                self.search_edited_at = None;
                if self.filter.text == self.search_input {
                    return false;
                }
                self.filter.text.clone_from(&self.search_input);
                true
            }
            _ => false,
        }
    }

    /// Show everyone's entries: clears the person and shared filters.
    pub fn show_all_people(&mut self) {
        self.filter.person_id = None;
        self.filter.shared_only = false;
    }

    pub fn filter_person(&mut self, person_id: i64) {
        self.filter.person_id = Some(person_id);
        self.filter.shared_only = false;
    }

    pub fn filter_shared(&mut self) {
        self.filter.person_id = None;
        self.filter.shared_only = true;
    }

    pub fn set_category_filter(&mut self, category_id: Option<i64>) {
        self.filter.category_id = category_id;
    }

    /// Whether anything narrows the register beyond the selected month.
    #[must_use]
    pub fn has_filters(&self) -> bool {
        let f = &self.filter;
        f.person_id.is_some()
            || f.shared_only
            || f.category_id.is_some()
            || f.issue.is_some()
            || !f.text.is_empty()
            || !self.search_input.is_empty()
    }

    /// Drop every filter except the month.
    pub fn clear_filters(&mut self) {
        self.filter = TxFilter {
            month: Some(self.month),
            ..TxFilter::default()
        };
        self.search_input.clear();
        self.search_edited_at = None;
    }

    pub fn set_calendar_year(&mut self, year: i32) {
        if year != self.calendar_year {
            self.calendar_days = None;
            if self.screen == Screen::Calendar {
                self.clear_day_selection();
            }
        }
        self.calendar_year = year;
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new(YearMonth::current_local())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_month_keeps_the_filter_in_step() {
        let mut ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        assert_eq!(ui.filter.month, YearMonth::new(2026, 11));
        ui.filter.text = "market".into();
        ui.set_month(YearMonth::new(2026, 12).unwrap());
        assert_eq!(ui.filter.month, YearMonth::new(2026, 12));
        assert_eq!(ui.filter.text, "market");
        assert_eq!(ui.calendar_year, 2026);
        ui.set_calendar_year(2027);
        assert_eq!(ui.calendar_year, 2027);
        assert_eq!(ui.month, YearMonth::new(2026, 12).unwrap());
    }

    #[test]
    fn search_waits_for_a_pause_before_it_filters() {
        let mut ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        ui.edit_search("mar".into(), 10.0);
        assert!(ui.search_pending());
        assert!(!ui.commit_search_if_due(10.1));
        assert_eq!(ui.filter.text, "");
        ui.edit_search("market".into(), 10.15);
        assert!(
            !ui.commit_search_if_due(10.3),
            "typing again restarts the wait"
        );
        assert!(ui.commit_search_if_due(10.4));
        assert_eq!(ui.filter.text, "market");
        assert!(!ui.search_pending());
        assert!(!ui.commit_search_if_due(11.0), "nothing left to apply");
    }

    #[test]
    fn retyping_the_same_text_does_not_reload() {
        let mut ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        ui.edit_search("a".into(), 1.0);
        assert!(ui.commit_search_if_due(2.0));
        ui.edit_search("a".into(), 3.0);
        assert!(!ui.commit_search_if_due(4.0));
    }

    #[test]
    fn people_filters_are_mutually_exclusive_and_clear_together() {
        let mut ui = UiState::new(YearMonth::new(2026, 11).unwrap());
        assert!(!ui.has_filters());
        ui.filter_person(2);
        assert_eq!(
            (ui.filter.person_id, ui.filter.shared_only),
            (Some(2), false)
        );
        ui.filter_shared();
        assert_eq!((ui.filter.person_id, ui.filter.shared_only), (None, true));
        ui.show_all_people();
        assert!(!ui.has_filters());
        ui.filter_person(1);
        ui.set_category_filter(Some(4));
        ui.edit_search("x".into(), 0.0);
        assert!(ui.has_filters());
        ui.clear_filters();
        assert!(!ui.has_filters() && !ui.search_pending());
        assert_eq!(
            ui.filter.month,
            YearMonth::new(2026, 11),
            "the month survives"
        );
    }

    #[test]
    fn repair_filter_lists_all_months_until_as_of_and_clears() {
        use crate::domain::money::Money;
        use crate::domain::transaction::{Allocation, TransactionType};
        use crate::infrastructure::db::{Db, transactions_repo};
        use crate::shared::test_fixtures::{date, draft_with, seed_people};

        let nov = YearMonth::new(2026, 11).unwrap();
        let dec = YearMonth::new(2026, 12).unwrap();
        let mut ui = UiState::new(nov);
        assert_eq!(ui.filter.month, Some(nov));

        ui.set_repair_filter(SettlementIssue::UnknownPayer, nov.last_day());
        assert_eq!(ui.filter.month, None);
        assert_eq!(ui.filter.until, Some(nov.last_day()));
        assert_eq!(ui.filter.issue, Some(SettlementIssue::UnknownPayer));
        assert!(ui.has_filters());

        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let alloc = vec![Allocation {
            person_id: partner,
            amount: Money::from_cents(1_000),
        }];

        let mut oct_missing = draft_with(alloc.clone());
        oct_missing.title = "Oct missing".into();
        oct_missing.date = date(2026, 10, 15);
        transactions_repo::create(db.conn(), &oct_missing).unwrap();

        let mut nov_missing = draft_with(alloc.clone());
        nov_missing.title = "Nov missing".into();
        nov_missing.kind = TransactionType::Refund;
        nov_missing.date = date(2026, 11, 20);
        transactions_repo::create(db.conn(), &nov_missing).unwrap();

        let mut dec_missing = draft_with(alloc.clone());
        dec_missing.title = "Dec missing".into();
        dec_missing.date = date(2026, 12, 1);
        transactions_repo::create(db.conn(), &dec_missing).unwrap();

        let mut nov_unassigned = draft_with(vec![]);
        nov_unassigned.title = "Nov unassigned".into();
        nov_unassigned.date = date(2026, 11, 10);
        transactions_repo::create(db.conn(), &nov_unassigned).unwrap();

        let mut nov_complete = draft_with(alloc);
        nov_complete.title = "Nov complete".into();
        nov_complete.payer_id = Some(owner);
        nov_complete.date = date(2026, 11, 12);
        transactions_repo::create(db.conn(), &nov_complete).unwrap();

        let unknown_titles: Vec<String> = transactions_repo::list(db.conn(), &ui.filter)
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(unknown_titles, vec!["Nov missing", "Oct missing"]);

        ui.set_repair_filter(SettlementIssue::Unassigned, nov.last_day());
        let unassigned_titles: Vec<String> = transactions_repo::list(db.conn(), &ui.filter)
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(unassigned_titles, vec!["Nov unassigned"]);

        ui.clear_repair_filter();
        assert_eq!(ui.filter.month, Some(nov));
        assert_eq!(ui.filter.until, None);
        assert_eq!(ui.filter.issue, None);
        assert!(!ui.has_filters());

        ui.set_repair_filter(SettlementIssue::UnknownPayer, nov.last_day());
        ui.set_month(dec);
        assert_eq!(ui.filter.month, Some(dec));
        assert_eq!(ui.filter.issue, None);
        assert_eq!(ui.filter.until, None);

        ui.set_repair_filter(SettlementIssue::Unassigned, dec.last_day());
        ui.clear_filters();
        assert_eq!(ui.filter.month, Some(dec));
        assert_eq!(ui.filter.issue, None);
        assert_eq!(ui.filter.until, None);
    }

    #[test]
    fn repair_filter_clears_conflicting_filters_and_pending_search() {
        use crate::domain::transaction::TransactionType;
        use crate::infrastructure::db::{Db, transactions_repo};
        use crate::shared::test_fixtures::{date, draft_with, seed_people};

        let nov = YearMonth::new(2026, 11).unwrap();
        let mut ui = UiState::new(nov);
        let db = Db::open_in_memory().unwrap();
        let (_owner, partner) = seed_people(db.conn());

        let mut unassigned = draft_with(vec![]);
        unassigned.title = "Unassigned item".into();
        unassigned.date = date(2026, 10, 5);
        transactions_repo::create(db.conn(), &unassigned).unwrap();

        // Activate every conflicting filter + pending search debounce.
        ui.filter_shared();
        ui.set_category_filter(Some(1));
        ui.filter.kind = Some(TransactionType::Income);
        ui.filter.text = "nomatch".into();
        ui.edit_search("still typing".into(), 10.0);

        ui.set_repair_filter(SettlementIssue::Unassigned, nov.last_day());
        assert_eq!(ui.filter.person_id, None);
        assert!(!ui.filter.shared_only);
        assert_eq!(ui.filter.category_id, None);
        assert_eq!(ui.filter.kind, None);
        assert!(ui.filter.text.is_empty());
        assert!(ui.search_input.is_empty());
        assert!(!ui.search_pending());
        assert!(
            !ui.commit_search_if_due(11.0),
            "pending search must not re-apply"
        );

        let rows = transactions_repo::list(db.conn(), &ui.filter).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Unassigned item");

        // Also verify Person filter + Unassigned yields the item after repair filter activation.
        ui.filter_person(partner);
        ui.set_repair_filter(SettlementIssue::Unassigned, nov.last_day());
        let rows2 = transactions_repo::list(db.conn(), &ui.filter).unwrap();
        assert_eq!(rows2.len(), 1);
    }
}
