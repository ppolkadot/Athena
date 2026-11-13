//! Calendar months. Local calendar month, Monday-first grids. No display strings live here.

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct YearMonth {
    pub year: i32,
    pub month: u32,
}

impl YearMonth {
    /// `None` when `month` is outside `1..=12` or the year is outside chrono's range.
    #[must_use]
    pub fn new(year: i32, month: u32) -> Option<Self> {
        NaiveDate::from_ymd_opt(year, month, 1).map(|_| Self { year, month })
    }

    #[must_use]
    pub fn from_date(date: NaiveDate) -> Self {
        Self {
            year: date.year(),
            month: date.month(),
        }
    }

    #[must_use]
    pub fn current_local() -> Self {
        Self::from_date(chrono::Local::now().date_naive())
    }

    #[must_use]
    pub fn next(self) -> Self {
        if self.month >= 12 {
            Self {
                year: self.year + 1,
                month: 1,
            }
        } else {
            Self {
                year: self.year,
                month: self.month + 1,
            }
        }
    }

    #[must_use]
    pub fn prev(self) -> Self {
        if self.month <= 1 {
            Self {
                year: self.year - 1,
                month: 12,
            }
        } else {
            Self {
                year: self.year,
                month: self.month - 1,
            }
        }
    }

    /// First day of the month. A value built through [`YearMonth::new`] is always valid; the
    /// fallback only guards hand-built values with an impossible month.
    #[must_use]
    pub fn first_day(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.year, self.month, 1).unwrap_or(NaiveDate::MIN)
    }

    #[must_use]
    pub fn last_day(self) -> NaiveDate {
        self.next()
            .first_day()
            .pred_opt()
            .unwrap_or_else(|| self.first_day())
    }

    /// `(first day, first day of the next month)`: a half-open range for SQL.
    #[must_use]
    pub fn bounds(self) -> (NaiveDate, NaiveDate) {
        (self.first_day(), self.next().first_day())
    }

    /// `2026-11`.
    #[must_use]
    pub fn key(self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }

    #[must_use]
    pub fn contains(self, date: NaiveDate) -> bool {
        Self::from_date(date) == self
    }
}

/// Monday-first month grid. `None` pads before the 1st and after the last day so the length is a
/// multiple of 7. An impossible month yields an empty grid.
#[must_use]
pub fn month_grid(year: i32, month: u32) -> Vec<Option<NaiveDate>> {
    let Some(ym) = YearMonth::new(year, month) else {
        return Vec::new();
    };
    let first = ym.first_day();
    let lead = first.weekday().num_days_from_monday() as usize;
    let mut cells: Vec<Option<NaiveDate>> = vec![None; lead];
    let mut day = first;
    while ym.contains(day) {
        cells.push(Some(day));
        match day.succ_opt() {
            Some(next) => day = next,
            None => break,
        }
    }
    while !cells.len().is_multiple_of(7) {
        cells.push(None);
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_navigation_crosses_years() {
        let nov = YearMonth::new(2026, 11).unwrap();
        assert_eq!(nov.prev(), YearMonth::new(2026, 10).unwrap());
        assert_eq!(
            YearMonth::new(2026, 1).unwrap().prev(),
            YearMonth::new(2025, 12).unwrap()
        );
        assert_eq!(
            YearMonth::new(2026, 12).unwrap().next(),
            YearMonth::new(2027, 1).unwrap()
        );
        assert_eq!(nov.key(), "2026-11");
    }

    #[test]
    fn month_grid_is_monday_first_and_padded() {
        let grid = month_grid(2024, 2);
        assert_eq!(grid.len() % 7, 0);
        assert_eq!(grid.len(), 35);
        assert_eq!(grid[0], None);
        assert_eq!(grid[3], Some(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()));
        assert_eq!(
            grid[31],
            Some(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap())
        );
    }

    #[test]
    fn bounds_are_half_open_and_cross_the_year() {
        let dec = YearMonth::new(2026, 12).unwrap();
        let (start, end) = dec.bounds();
        assert_eq!(start, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(end, NaiveDate::from_ymd_opt(2027, 1, 1).unwrap());
        assert_eq!(
            dec.last_day(),
            NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()
        );
    }

    #[test]
    fn rejects_impossible_months_and_orders_chronologically() {
        assert!(YearMonth::new(2026, 0).is_none());
        assert!(YearMonth::new(2026, 13).is_none());
        assert!(YearMonth::new(2026, 12).unwrap() < YearMonth::new(2027, 1).unwrap());
        assert_eq!(month_grid(2026, 13), Vec::<Option<NaiveDate>>::new());
    }
}
