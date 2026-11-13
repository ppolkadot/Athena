//! Ephemeral UI state: which screen is showing, the selected month and year, filters.

use super::navigation::Screen;
use crate::domain::date::YearMonth;
use crate::domain::transaction::TxFilter;
use crate::ui::screens::settings::SettingsState;

#[derive(Debug, Clone)]
pub struct UiState {
    pub screen: Screen,
    pub month: YearMonth,
    pub calendar_year: i32,
    /// `filter.month` always equals `Some(month)`.
    pub filter: TxFilter,
    /// What is typed in the search box. It reaches `filter.text` after a short pause.
    pub search_input: String,
    search_edited_at: Option<f64>,
    /// Edits in progress on the settings screen.
    pub settings: SettingsState,
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
        }
    }

    pub fn set_screen(&mut self, screen: Screen) {
        if screen != Screen::Settings {
            // Leaving settings drops unsaved edits, so reopening shows what is stored.
            self.settings.reset();
        }
        self.screen = screen;
    }

    /// Select a month. Filters that only make sense inside one month (day, added in 1.1) are
    /// cleared here so the register never looks empty after navigating.
    pub fn set_month(&mut self, month: YearMonth) {
        self.month = month;
        self.filter.month = Some(month);
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
}
