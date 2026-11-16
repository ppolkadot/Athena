//! Pure report result types and arithmetic. SQL aggregation lives in `infrastructure/db`.

use chrono::NaiveDate;

use super::date::YearMonth;
use super::money::Money;
use crate::shared::error::{AppError, Result};

/// Income, expenses and refunds for a range. `net = income - expenses + refunds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MonthSummary {
    pub income: Money,
    pub expenses: Money,
    pub refunds: Money,
    pub net: Money,
    pub count: i64,
}

impl MonthSummary {
    /// Expenses minus refunds. Refunds are never income.
    #[must_use]
    pub fn spending(&self) -> Money {
        Money::from_cents(self.expenses.cents().saturating_sub(self.refunds.cents()))
    }

    /// Checked expenses minus refunds.
    ///
    /// # Errors
    /// [`AppError::Validation`] if `expenses - refunds` overflows `i64`.
    pub fn checked_spending(&self) -> Result<Money> {
        self.expenses
            .cents()
            .checked_sub(self.refunds.cents())
            .map(Money::from_cents)
            .ok_or_else(|| AppError::Validation("report calculation overflow".into()))
    }

    /// Build a summary and compute `net` in exactly one place.
    ///
    /// # Errors
    /// [`AppError::Validation`] if `net` (`income - expenses + refunds`) or `expenses - refunds` overflows `i64`.
    pub fn new(income: Money, expenses: Money, refunds: Money, count: i64) -> Result<Self> {
        let net =
            i128::from(income.cents()) - i128::from(expenses.cents()) + i128::from(refunds.cents());
        let net = i64::try_from(net)
            .map_err(|_| AppError::Validation("report calculation overflow".into()))?;
        expenses
            .cents()
            .checked_sub(refunds.cents())
            .ok_or_else(|| AppError::Validation("report calculation overflow".into()))?;
        Ok(Self {
            income,
            expenses,
            refunds,
            net: Money::from_cents(net),
            count,
        })
    }
}

/// Spending (expenses minus refunds) for one category; `None` means uncategorized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryTotal {
    pub category_id: Option<i64>,
    pub net: Money,
}

/// One person's responsibility (allocations of expenses minus refunds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonTotal {
    pub person_id: i64,
    pub net: Money,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarMonthTotal {
    pub month: YearMonth,
    pub summary: MonthSummary,
}

/// Net spending for one day (expenses minus refunds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayTotal {
    pub date: NaiveDate,
    pub net: Money,
}

/// Length of a bar as a fraction `0.0..=1.0` of the largest value. This is presentation only; all
/// money arithmetic stays in integer cents. Returns `0.0` when `max` is not positive, so an empty
/// month never divides by zero, and negative values draw no bar.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
pub fn bar_fraction(value: Money, max: Money) -> f32 {
    if max.cents() <= 0 {
        return 0.0;
    }
    let fraction = value.cents().max(0) as f64 / max.cents() as f64;
    fraction.clamp(0.0, 1.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_fraction_is_safe_and_bounded() {
        assert_eq!(bar_fraction(Money::ZERO, Money::ZERO), 0.0);
        assert_eq!(
            bar_fraction(Money::from_cents(5), Money::from_cents(-1)),
            0.0
        );
        assert_eq!(
            bar_fraction(Money::from_cents(-5), Money::from_cents(10)),
            0.0
        );
        assert_eq!(
            bar_fraction(Money::from_cents(5), Money::from_cents(10)),
            0.5
        );
        assert_eq!(
            bar_fraction(Money::from_cents(50), Money::from_cents(10)),
            1.0
        );
    }

    #[test]
    fn summary_net_is_income_minus_expenses_plus_refunds() {
        let s = MonthSummary::new(
            Money::from_cents(300),
            Money::from_cents(150),
            Money::from_cents(20),
            4,
        )
        .unwrap();
        assert_eq!(s.net, Money::from_cents(170));
        assert_eq!(s.spending(), Money::from_cents(130));
        assert_eq!(s.checked_spending().unwrap(), Money::from_cents(130));
        assert_eq!(MonthSummary::default().net, Money::ZERO);
    }

    #[test]
    fn summary_overflow_returns_validation_error() {
        let err = MonthSummary::new(
            Money::from_cents(i64::MAX),
            Money::ZERO,
            Money::from_cents(1),
            1,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));

        let err2 = MonthSummary::new(
            Money::ZERO,
            Money::from_cents(i64::MAX),
            Money::from_cents(-1),
            1,
        )
        .unwrap_err();
        assert!(matches!(err2, AppError::Validation(_)));
    }
}
