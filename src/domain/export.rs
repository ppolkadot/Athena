//! What an export covers. Exporters take this explicitly and never read the clock.

use chrono::NaiveDate;

use super::date::YearMonth;
use super::transaction::TxFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportScope {
    All,
    Month(YearMonth),
}

impl ExportScope {
    /// The transaction filter that selects exactly this scope.
    #[must_use]
    pub fn filter(self) -> TxFilter {
        match self {
            ExportScope::All => TxFilter::default(),
            ExportScope::Month(month) => TxFilter {
                month: Some(month),
                ..TxFilter::default()
            },
        }
    }
}

/// `athena-YYYY-MM.csv` for a month, `athena-all.csv` for everything.
#[must_use]
pub fn default_export_name(scope: ExportScope) -> String {
    match scope {
        ExportScope::All => "athena-all.csv".to_string(),
        ExportScope::Month(month) => format!("athena-{}.csv", month.key()),
    }
}

/// `athena-full-YYYY-MM-DD.json`. The JSON export is always the complete profile.
#[must_use]
pub fn default_json_name(today: NaiveDate) -> String {
    format!("athena-full-{}.json", today.format("%Y-%m-%d"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_names_carry_scope_and_date() {
        let nov = YearMonth::new(2026, 11).unwrap();
        assert_eq!(
            default_export_name(ExportScope::Month(nov)),
            "athena-2026-11.csv"
        );
        assert_eq!(default_export_name(ExportScope::All), "athena-all.csv");
        assert_eq!(
            default_json_name(NaiveDate::from_ymd_opt(2026, 11, 12).unwrap()),
            "athena-full-2026-11-12.json"
        );
    }

    #[test]
    fn scope_filters_select_the_month_or_everything() {
        let nov = YearMonth::new(2026, 11).unwrap();
        assert_eq!(ExportScope::All.filter(), TxFilter::default());
        assert_eq!(ExportScope::Month(nov).filter().month, Some(nov));
    }
}
