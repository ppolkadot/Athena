//! Saved monthly closings (contract §14.3): a historical snapshot of a completed month, plus the
//! version-1 comparison fingerprints used to tell whether the ledger changed since. Pure rules
//! live here; SQL is in `infrastructure/db/closings_repo.rs`.

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::date::YearMonth;
use super::money::Money;
use super::reports::{InsightInputs, LargestExpense, MonthPhase, MonthSummary};

/// The comparison-record algorithm version stored with every closing.
pub const FINGERPRINT_VERSION: u32 = 1;

/// The largest expense as captured: its row may be gone later, the title and amount stay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosingExpense {
    /// The live row at capture time; `None` once it was purged (or when exported without it).
    pub id: Option<i64>,
    pub title: String,
    pub amount: Money,
}

/// A saved closing. Figures are the values at capture time and stay even if live totals differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosingSnapshot {
    pub month: YearMonth,
    pub closed_at: String,
    pub income: Money,
    pub expenses: Money,
    pub refunds: Money,
    /// `income − expenses + refunds`.
    pub result: Money,
    /// Financial entries (settlements excluded).
    pub count: i64,
    /// Net spending of the previous month at capture time; may be negative.
    pub previous_spending: Money,
    pub previous_count: i64,
    /// The winning category with its positive net spending. A `None` id is Uncategorized; a
    /// missing winner is `None`.
    pub top_category: Option<(Option<i64>, Money)>,
    pub largest_expense: Option<ClosingExpense>,
    pub fingerprint_version: u32,
    /// Fingerprint of the closed month's live financial rows.
    pub current_fingerprint: String,
    /// Fingerprint of the previous month's live financial rows.
    pub previous_fingerprint: String,
}

/// Why a closing is refused or inconsistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ClosingError {
    #[error("only a completed month can be closed")]
    NotCompleted,
    #[error("this month is already closed")]
    AlreadyClosed,
    #[error("this month has no saved closing")]
    NotClosed,
    #[error("the saved figures are inconsistent")]
    Inconsistent,
    #[error("the figures exceed the supported range")]
    Overflow,
}

/// Whether the ledger still matches what the closing saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosingStatus {
    /// Neither the month nor its predecessor changed.
    Current,
    /// Either month changed since the closing.
    Changed,
    /// The check could not run; freshness is not claimed.
    Unavailable,
}

/// Whether `month` may be closed on `today`'s local date: it must be strictly before the current
/// month.
#[must_use]
pub fn is_completed(month: YearMonth, today: chrono::NaiveDate) -> bool {
    month < YearMonth::from_date(today)
}

/// `true` for a 64-character lower-case hex string.
#[must_use]
pub fn is_fingerprint(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// One live financial row of the comparison record. Field order is part of the format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FingerprintRow {
    pub id: i64,
    pub date: String,
    pub kind: String,
    pub amount_cents: i64,
    pub category_id: Option<i64>,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
struct Record<'a> {
    version: u32,
    month: String,
    rows: &'a [FingerprintRow],
}

/// The canonical bytes of month `month`'s comparison record (version 1): compact JSON of
/// `{"version":1,"month":"YYYY-MM","rows":[…]}`, rows sorted by `(date, id)`.
#[must_use]
pub fn canonical_bytes(month: YearMonth, rows: &[FingerprintRow]) -> Vec<u8> {
    let mut sorted: Vec<&FingerprintRow> = rows.iter().collect();
    sorted.sort_by(|a, b| a.date.cmp(&b.date).then(a.id.cmp(&b.id)));
    let ordered: Vec<FingerprintRow> = sorted.into_iter().cloned().collect();
    let record = Record {
        version: FINGERPRINT_VERSION,
        month: month.key(),
        rows: &ordered,
    };
    serde_json::to_vec(&record).unwrap_or_default()
}

/// The lower-case hex SHA-256 of [`canonical_bytes`].
#[must_use]
pub fn fingerprint(month: YearMonth, rows: &[FingerprintRow]) -> String {
    let digest = Sha256::digest(canonical_bytes(month, rows));
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    out
}

impl ClosingSnapshot {
    /// Build a snapshot from a completed month's insight inputs and its two fingerprints.
    ///
    /// # Errors
    /// [`ClosingError::NotCompleted`] unless the inputs describe a completed month;
    /// [`ClosingError::Overflow`] when the figures cannot be represented.
    pub fn capture(
        inputs: &InsightInputs,
        closed_at: String,
        current_fingerprint: String,
        previous_fingerprint: String,
    ) -> Result<Self, ClosingError> {
        if inputs.phase != MonthPhase::Completed {
            return Err(ClosingError::NotCompleted);
        }
        let c = &inputs.current;
        Ok(Self {
            month: inputs.month,
            closed_at,
            income: c.income,
            expenses: c.expenses,
            refunds: c.refunds,
            result: c.net,
            count: c.count,
            previous_spending: inputs.prior_spending,
            previous_count: inputs.prior_count,
            top_category: inputs.winning_category.filter(|(_, net)| net.cents() > 0),
            largest_expense: inputs.largest_expense.as_ref().map(|e| ClosingExpense {
                id: Some(e.id),
                title: e.title.clone(),
                amount: e.amount,
            }),
            fingerprint_version: FINGERPRINT_VERSION,
            current_fingerprint,
            previous_fingerprint,
        })
    }

    /// The insight inputs the saved card words: the saved figures as a completed month, without
    /// the live-only leading person.
    #[must_use]
    pub fn insight_inputs(&self) -> InsightInputs {
        InsightInputs {
            month: self.month,
            phase: MonthPhase::Completed,
            current: MonthSummary {
                income: self.income,
                expenses: self.expenses,
                refunds: self.refunds,
                net: self.result,
                count: self.count,
            },
            prior_spending: self.previous_spending,
            prior_count: self.previous_count,
            winning_category: self.top_category,
            largest_expense: self.largest_expense.as_ref().map(|e| LargestExpense {
                id: e.id.unwrap_or(0),
                title: e.title.clone(),
                amount: e.amount,
            }),
            leading_person: None,
        }
    }

    /// Every consistency rule that needs no database: nonnegative parts, a checked result,
    /// positive optional amounts, fingerprint shape and version.
    ///
    /// # Errors
    /// [`ClosingError::Inconsistent`] or [`ClosingError::Overflow`].
    pub fn validate(&self) -> Result<(), ClosingError> {
        if self.income.cents() < 0
            || self.expenses.cents() < 0
            || self.refunds.cents() < 0
            || self.count < 0
            || self.previous_count < 0
        {
            return Err(ClosingError::Inconsistent);
        }
        let result = i128::from(self.income.cents()) - i128::from(self.expenses.cents())
            + i128::from(self.refunds.cents());
        let result = i64::try_from(result).map_err(|_| ClosingError::Overflow)?;
        if result != self.result.cents() {
            return Err(ClosingError::Inconsistent);
        }
        self.expenses
            .cents()
            .checked_sub(self.refunds.cents())
            .ok_or(ClosingError::Overflow)?;
        if self.top_category.is_some_and(|(_, net)| net.cents() <= 0) {
            return Err(ClosingError::Inconsistent);
        }
        if let Some(e) = &self.largest_expense
            && (e.amount.cents() <= 0
                || e.title.trim().is_empty()
                || e.id.is_some_and(|id| id <= 0))
        {
            return Err(ClosingError::Inconsistent);
        }
        if self.fingerprint_version != FINGERPRINT_VERSION
            || !is_fingerprint(&self.current_fingerprint)
            || !is_fingerprint(&self.previous_fingerprint)
        {
            return Err(ClosingError::Inconsistent);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, date: &str, title: &str) -> FingerprintRow {
        FingerprintRow {
            id,
            date: date.into(),
            kind: "expense".into(),
            amount_cents: 1000,
            category_id: Some(3),
            title: title.into(),
            created_at: "2026-10-01T00:00:00Z".into(),
            updated_at: "2026-10-01T00:00:00Z".into(),
        }
    }

    fn oct() -> YearMonth {
        YearMonth::new(2026, 10).unwrap()
    }

    #[test]
    fn the_canonical_record_is_compact_json_sorted_by_date_then_id() {
        let rows = vec![
            row(2, "2026-10-05", "B"),
            row(1, "2026-10-05", "A"),
            row(3, "2026-10-01", "C"),
        ];
        let bytes = String::from_utf8(canonical_bytes(oct(), &rows)).unwrap();
        assert!(bytes.starts_with(r#"{"version":1,"month":"2026-10","rows":[{"id":3,"date":"2026-10-01","kind":"expense","amount_cents":1000,"category_id":3,"title":"C","created_at":"2026-10-01T00:00:00Z","updated_at":"2026-10-01T00:00:00Z"},{"id":1,"#));
        assert!(bytes.ends_with("}]}"));
        let empty = String::from_utf8(canonical_bytes(oct(), &[])).unwrap();
        assert_eq!(empty, r#"{"version":1,"month":"2026-10","rows":[]}"#);
    }

    #[test]
    fn fingerprints_are_stable_lower_case_hex_and_order_independent() {
        let a = vec![row(1, "2026-10-01", "A"), row(2, "2026-10-02", "B")];
        let b = vec![row(2, "2026-10-02", "B"), row(1, "2026-10-01", "A")];
        let fp = fingerprint(oct(), &a);
        assert_eq!(fp, fingerprint(oct(), &b));
        assert!(is_fingerprint(&fp));
        // The SHA-256 of the empty record is fixed: pin it so the algorithm cannot drift.
        assert_eq!(
            fingerprint(oct(), &[]),
            "89bb171690c495f380cdd0ef30ec49ab5752f7c1043c13753e297a1e4d187f7e"
        );
        // Every field and the month key matter.
        assert_ne!(fp, fingerprint(YearMonth::new(2026, 9).unwrap(), &a));
        let mut renamed = a.clone();
        renamed[0].title = "A2".into();
        assert_ne!(fp, fingerprint(oct(), &renamed));
        let mut touched = a.clone();
        touched[1].updated_at = "2026-10-09T00:00:00Z".into();
        assert_ne!(fp, fingerprint(oct(), &touched));
    }

    #[test]
    fn only_a_month_before_the_current_one_is_completed() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        assert!(is_completed(oct(), today));
        assert!(!is_completed(YearMonth::new(2026, 11).unwrap(), today));
        assert!(!is_completed(YearMonth::new(2026, 12).unwrap(), today));
        assert!(is_completed(
            YearMonth::new(2026, 12).unwrap(),
            chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        ));
    }
}
