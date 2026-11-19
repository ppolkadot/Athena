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
    /// Every live row of the month, settlements included. `summary.count` keeps its financial
    /// meaning (settlements excluded), so a settlement-only month is still discoverable.
    pub activity_count: i64,
}

/// One day's live activity: every kind counts once toward `activity_count`; `spending` is expenses
/// minus refunds only (income and settlements add nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayActivity {
    pub date: NaiveDate,
    pub spending: Money,
    pub activity_count: i64,
}

/// Every day of `month` in order, zero-filled where nothing happened.
#[must_use]
pub fn zero_filled_days(month: YearMonth, activity: &[DayActivity]) -> Vec<DayActivity> {
    let mut out = Vec::new();
    let mut day = month.first_day();
    while month.contains(day) {
        out.push(
            activity
                .iter()
                .find(|a| a.date == day)
                .cloned()
                .unwrap_or(DayActivity {
                    date: day,
                    spending: Money::ZERO,
                    activity_count: 0,
                }),
        );
        match day.succ_opt() {
            Some(next) => day = next,
            None => break,
        }
    }
    out
}

/// Whether an insight describes a finished month, the current month so far, or a month to come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthPhase {
    Completed,
    /// The current month: figures run through today only.
    ToDate,
    /// A future month: planned entries, no comparison.
    Planned,
}

impl MonthPhase {
    #[must_use]
    pub fn of(month: YearMonth, today: NaiveDate) -> Self {
        match month.cmp(&YearMonth::from_date(today)) {
            std::cmp::Ordering::Less => MonthPhase::Completed,
            std::cmp::Ordering::Equal => MonthPhase::ToDate,
            std::cmp::Ordering::Greater => MonthPhase::Planned,
        }
    }
}

/// The largest single expense of a month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LargestExpense {
    pub id: i64,
    pub title: String,
    pub amount: Money,
}

/// Everything a monthly insight needs, structured so closing capture can reuse it. Labels are
/// resolved by id when drawn, so renames show up without recomputing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsightInputs {
    pub month: YearMonth,
    pub phase: MonthPhase,
    /// The unfiltered month (through today for [`MonthPhase::ToDate`]).
    pub current: MonthSummary,
    /// Net spending (expenses minus refunds) of the full previous month; may be negative.
    pub prior_spending: Money,
    /// Financial entry count of the full previous month.
    pub prior_count: i64,
    /// The category with the greatest positive net spending; `None` id is Uncategorized.
    pub winning_category: Option<(Option<i64>, Money)>,
    pub largest_expense: Option<LargestExpense>,
    /// The person with the greatest positive responsibility, for the live rail only.
    pub leading_person: Option<(i64, Money)>,
}

/// Which headline applies to a month. Checked in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightState {
    NoEntries,
    IncomeOnly,
    RefundsOnly,
    /// Refunds exceed expenses by this (positive) amount.
    NetCredits(Money),
    ZeroSpending,
    Spending(Money),
}

/// How a month's spending compares with the previous month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    /// Future months, and months without financial entries, have no comparison.
    None,
    Up(PercentChange),
    Down(PercentChange),
    Unchanged,
    /// A nonpositive month on either side: both labeled amounts, no percentage.
    Amounts,
}

/// A percentage of the previous month, half-up in integer arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PercentChange {
    /// A nonzero difference under one percent.
    LessThanOne,
    Whole(u64),
}

impl PercentChange {
    /// `|current - prior|` over `prior` in percent. `prior` must be positive and the two must
    /// differ; computed in `i128`, half-up, with a sub-percent difference reported as such.
    #[must_use]
    pub fn between(current: Money, prior: Money) -> Self {
        let diff = (i128::from(current.cents()) - i128::from(prior.cents())).abs();
        let base = i128::from(prior.cents());
        if diff * 100 < base {
            return PercentChange::LessThanOne;
        }
        let half_up = (diff * 200 + base) / (2 * base);
        PercentChange::Whole(u64::try_from(half_up).unwrap_or(u64::MAX))
    }
}

impl InsightInputs {
    /// The headline state of the month.
    #[must_use]
    pub fn state(&self) -> InsightState {
        let m = &self.current;
        let zero = Money::ZERO;
        if m.count == 0 {
            return InsightState::NoEntries;
        }
        if m.income > zero && m.expenses == zero && m.refunds == zero {
            return InsightState::IncomeOnly;
        }
        if m.refunds > zero && m.income == zero && m.expenses == zero {
            return InsightState::RefundsOnly;
        }
        let spending = m.spending();
        if spending.is_negative() {
            return InsightState::NetCredits(Money::from_cents(spending.cents().saturating_neg()));
        }
        if spending == zero {
            return InsightState::ZeroSpending;
        }
        InsightState::Spending(spending)
    }

    /// The comparison with the previous month: a percentage only when both months have positive
    /// spending, both amounts when either does not, and none for planned months or months with
    /// no financial entries.
    #[must_use]
    pub fn comparison(&self) -> Comparison {
        if self.phase == MonthPhase::Planned || self.current.count == 0 {
            return Comparison::None;
        }
        let current = self.current.spending();
        let prior = self.prior_spending;
        if current > Money::ZERO && prior > Money::ZERO {
            return match current.cmp(&prior) {
                std::cmp::Ordering::Equal => Comparison::Unchanged,
                std::cmp::Ordering::Greater => {
                    Comparison::Up(PercentChange::between(current, prior))
                }
                std::cmp::Ordering::Less => {
                    Comparison::Down(PercentChange::between(current, prior))
                }
            };
        }
        Comparison::Amounts
    }
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
    // The bar fraction is derived from exact halves of cent counts, so exact comparison is
    // meaningful here.
    #![allow(clippy::float_cmp)]

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

    fn summary(income: i64, expenses: i64, refunds: i64, count: i64) -> MonthSummary {
        MonthSummary::new(
            Money::from_cents(income),
            Money::from_cents(expenses),
            Money::from_cents(refunds),
            count,
        )
        .unwrap()
    }

    fn inputs(current: MonthSummary, prior: i64, phase: MonthPhase) -> InsightInputs {
        InsightInputs {
            month: YearMonth::new(2026, 10).unwrap(),
            phase,
            current,
            prior_spending: Money::from_cents(prior),
            prior_count: 3,
            winning_category: None,
            largest_expense: None,
            leading_person: None,
        }
    }

    #[test]
    fn insight_states_follow_the_documented_order() {
        let state = |s| inputs(s, 0, MonthPhase::Completed).state();
        assert_eq!(state(summary(0, 0, 0, 0)), InsightState::NoEntries);
        assert_eq!(state(summary(500, 0, 0, 1)), InsightState::IncomeOnly);
        assert_eq!(state(summary(0, 0, 300, 1)), InsightState::RefundsOnly);
        assert_eq!(
            state(summary(0, 100, 250, 2)),
            InsightState::NetCredits(Money::from_cents(150))
        );
        assert_eq!(state(summary(0, 100, 100, 2)), InsightState::ZeroSpending);
        assert_eq!(
            state(summary(900, 400, 100, 3)),
            InsightState::Spending(Money::from_cents(300))
        );
        // Income plus a refund is neither income-only nor refunds-only.
        assert_eq!(
            state(summary(500, 0, 100, 2)),
            InsightState::NetCredits(Money::from_cents(100))
        );
    }

    #[test]
    fn percentages_are_half_up_integers_with_a_sub_percent_label() {
        let pct =
            |cur, prior| PercentChange::between(Money::from_cents(cur), Money::from_cents(prior));
        assert_eq!(pct(5, 3), PercentChange::Whole(67), "66.67% rounds up");
        assert_eq!(
            pct(203, 200),
            PercentChange::Whole(2),
            "1.5% rounds half up"
        );
        assert_eq!(
            pct(301, 200),
            PercentChange::Whole(51),
            "50.5% rounds half up"
        );
        assert_eq!(pct(10_100, 10_000), PercentChange::Whole(1), "exactly 1%");
        assert_eq!(
            pct(10_050, 10_000),
            PercentChange::LessThanOne,
            "0.5% is sub-percent"
        );
        assert_eq!(pct(9_950, 10_000), PercentChange::LessThanOne);
        assert_eq!(
            pct(1_000_000_001, 1_000_000_000),
            PercentChange::LessThanOne
        );
        // The difference is absolute and the arithmetic is 128-bit.
        assert_eq!(pct(100, 200), PercentChange::Whole(50));
        assert_eq!(
            pct(i64::MAX, 1),
            PercentChange::Whole(
                u64::try_from((i128::from(i64::MAX) - 1) * 100).unwrap_or(u64::MAX)
            )
        );
    }

    #[test]
    fn comparison_covers_equality_direction_amounts_and_planned_months() {
        let spending = |expenses| summary(0, expenses, 0, 1);
        let cmp = |cur, prior, phase| inputs(spending(cur), prior, phase).comparison();
        assert_eq!(cmp(500, 500, MonthPhase::Completed), Comparison::Unchanged);
        assert_eq!(
            cmp(600, 500, MonthPhase::Completed),
            Comparison::Up(PercentChange::Whole(20))
        );
        assert_eq!(
            cmp(400, 500, MonthPhase::ToDate),
            Comparison::Down(PercentChange::Whole(20))
        );
        // A nonpositive prior month shows both amounts and never a percentage.
        assert_eq!(cmp(500, 0, MonthPhase::Completed), Comparison::Amounts);
        assert_eq!(cmp(500, -50, MonthPhase::Completed), Comparison::Amounts);
        // So does a nonpositive current month against a positive one.
        assert_eq!(
            inputs(summary(100, 0, 0, 1), 500, MonthPhase::Completed).comparison(),
            Comparison::Amounts
        );
        assert_eq!(cmp(600, 500, MonthPhase::Planned), Comparison::None);
        assert_eq!(
            inputs(summary(0, 0, 0, 0), 500, MonthPhase::Completed).comparison(),
            Comparison::None
        );
    }

    #[test]
    fn month_phase_uses_the_local_today() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let phase = |y, m| MonthPhase::of(YearMonth::new(y, m).unwrap(), today);
        assert_eq!(phase(2026, 10), MonthPhase::Completed);
        assert_eq!(phase(2025, 12), MonthPhase::Completed);
        assert_eq!(phase(2026, 11), MonthPhase::ToDate);
        assert_eq!(phase(2026, 12), MonthPhase::Planned);
        assert_eq!(phase(2027, 1), MonthPhase::Planned);
    }

    #[test]
    fn days_are_zero_filled_for_every_month_length() {
        let day = |d| NaiveDate::from_ymd_opt(2024, 2, d).unwrap();
        let activity = [DayActivity {
            date: day(29),
            spending: Money::from_cents(-500),
            activity_count: 2,
        }];
        let feb = zero_filled_days(YearMonth::new(2024, 2).unwrap(), &activity);
        assert_eq!(feb.len(), 29, "leap February");
        assert_eq!(feb[0].activity_count, 0);
        assert_eq!(feb[28].spending, Money::from_cents(-500));
        assert_eq!(
            zero_filled_days(YearMonth::new(2025, 2).unwrap(), &[]).len(),
            28
        );
        assert_eq!(
            zero_filled_days(YearMonth::new(2025, 12).unwrap(), &[]).len(),
            31
        );
        assert_eq!(
            zero_filled_days(YearMonth::new(2026, 1).unwrap(), &[]).len(),
            31
        );
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
