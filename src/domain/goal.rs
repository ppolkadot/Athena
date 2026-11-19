//! Savings goals (contract §14.3). Goal money is independent of the ledger: contributions track
//! progress only and never create or edit transactions. Pure rules live here; SQL is in
//! `infrastructure/db/goals_repo.rs`.

use chrono::{Datelike, NaiveDate};
use thiserror::Error;

use super::date::YearMonth;
use super::money::Money;

/// Longest contribution note, in characters.
pub const NOTE_MAX_CHARS: usize = 1000;

/// The manual workflow status of a goal. `Funded` is separate and derived from progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoalStatus {
    Active,
    Paused,
    Completed,
    Archived,
}

impl GoalStatus {
    pub const ALL: [GoalStatus; 4] = [
        GoalStatus::Active,
        GoalStatus::Paused,
        GoalStatus::Completed,
        GoalStatus::Archived,
    ];

    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Paused => "paused",
            GoalStatus::Completed => "completed",
            GoalStatus::Archived => "archived",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "active" => Some(GoalStatus::Active),
            "paused" => Some(GoalStatus::Paused),
            "completed" => Some(GoalStatus::Completed),
            "archived" => Some(GoalStatus::Archived),
            _ => None,
        }
    }

    /// Edits and contributions are allowed only while Active or Paused.
    #[must_use]
    pub fn is_editable(self) -> bool {
        matches!(self, GoalStatus::Active | GoalStatus::Paused)
    }

    /// A status a goal may be archived from (and restored to).
    #[must_use]
    pub fn can_be_pre_archive(self) -> bool {
        self != GoalStatus::Archived
    }
}

/// How often the user plans to contribute; drives the schedule hint only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
}

impl Frequency {
    pub const ALL: [Frequency; 3] = [Frequency::Daily, Frequency::Weekly, Frequency::Monthly];

    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Frequency::Daily => "daily",
            Frequency::Weekly => "weekly",
            Frequency::Monthly => "monthly",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "daily" => Some(Frequency::Daily),
            "weekly" => Some(Frequency::Weekly),
            "monthly" => Some(Frequency::Monthly),
            _ => None,
        }
    }
}

/// The user-editable fields of a goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalDraft {
    pub name: String,
    pub target: Money,
    pub target_date: Option<NaiveDate>,
    pub frequency: Frequency,
    pub attribution_id: Option<i64>,
}

impl GoalDraft {
    /// Trim the name.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self
    }

    /// Every rule that needs no database.
    ///
    /// # Errors
    /// All violations, in field order.
    pub fn validate(&self) -> Result<(), Vec<GoalError>> {
        let mut errors = Vec::new();
        if self.name.trim().is_empty() {
            errors.push(GoalError::NameEmpty);
        }
        if self.target.cents() <= 0 {
            errors.push(GoalError::TargetNotPositive);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// A stored goal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Goal {
    pub id: i64,
    pub name: String,
    pub target: Money,
    pub target_date: Option<NaiveDate>,
    pub frequency: Frequency,
    pub attribution_id: Option<i64>,
    pub status: GoalStatus,
    /// Set exactly while archived: the status Restore returns to.
    pub pre_archive_status: Option<GoalStatus>,
    pub created_at: String,
    pub updated_at: String,
}

impl Goal {
    #[must_use]
    pub fn draft(&self) -> GoalDraft {
        GoalDraft {
            name: self.name.clone(),
            target: self.target,
            target_date: self.target_date,
            frequency: self.frequency,
            attribution_id: self.attribution_id,
        }
    }
}

/// A goal with all its contributions, as exported and imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalRecord {
    pub goal: Goal,
    /// In id order.
    pub contributions: Vec<Contribution>,
}

/// One saved contribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contribution {
    pub id: i64,
    pub goal_id: i64,
    pub amount: Money,
    pub date: NaiveDate,
    pub note: Option<String>,
    pub created_at: String,
}

/// The fields of a new contribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributionDraft {
    pub amount: Money,
    pub date: NaiveDate,
    pub note: Option<String>,
}

impl ContributionDraft {
    /// Trim the note and drop an empty one.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.note = self
            .note
            .take()
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty());
        self
    }

    /// Validate against the app's local `today`.
    ///
    /// # Errors
    /// All violations.
    pub fn validate(&self, today: NaiveDate) -> Result<(), Vec<GoalError>> {
        let mut errors = Vec::new();
        if self.amount.cents() <= 0 {
            errors.push(GoalError::AmountNotPositive);
        }
        if self.date > today {
            errors.push(GoalError::DateInFuture);
        }
        if self
            .note
            .as_deref()
            .is_some_and(|n| n.chars().count() > NOTE_MAX_CHARS)
        {
            errors.push(GoalError::NoteTooLong);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// The form field a goal validation message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoalField {
    Name,
    Target,
    Attribution,
    Amount,
    Date,
    Note,
    /// Not tied to a field: shown as a general error.
    General,
}

/// Structured goal failures; the UI localizes each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GoalError {
    #[error("the goal name is empty")]
    NameEmpty,
    #[error("the target must be greater than zero")]
    TargetNotPositive,
    #[error("the person does not exist")]
    AttributionMissing,
    #[error("the person is archived")]
    AttributionArchived,
    #[error("the amount must be greater than zero")]
    AmountNotPositive,
    #[error("the contribution date is in the future")]
    DateInFuture,
    #[error("the note is too long")]
    NoteTooLong,
    #[error("the goal cannot change this way from its current status")]
    InvalidTransition,
    #[error("only active or paused goals can be edited")]
    NotEditable,
    #[error("contributions exceed the supported total")]
    Overflow,
}

impl GoalError {
    #[must_use]
    pub fn field(self) -> GoalField {
        match self {
            GoalError::NameEmpty => GoalField::Name,
            GoalError::TargetNotPositive => GoalField::Target,
            GoalError::AttributionMissing | GoalError::AttributionArchived => {
                GoalField::Attribution
            }
            GoalError::AmountNotPositive | GoalError::Overflow => GoalField::Amount,
            GoalError::DateInFuture => GoalField::Date,
            GoalError::NoteTooLong => GoalField::Note,
            GoalError::InvalidTransition | GoalError::NotEditable => GoalField::General,
        }
    }
}

/// A workflow step the user can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GoalTransition {
    Pause,
    Resume,
    Complete,
    Reopen,
    Archive,
    Restore,
}

/// Status and stored pre-archive status after a transition.
pub type StatusPair = (GoalStatus, Option<GoalStatus>);

/// Apply `transition` to a goal's `(status, pre_archive_status)`.
///
/// # Errors
/// [`GoalError::InvalidTransition`] when the step does not apply from the current status.
pub fn apply_transition(
    status: GoalStatus,
    pre_archive: Option<GoalStatus>,
    transition: GoalTransition,
) -> Result<StatusPair, GoalError> {
    use GoalStatus::{Active, Archived, Completed, Paused};
    use GoalTransition as T;
    match (status, transition) {
        (Active, T::Pause) => Ok((Paused, None)),
        (Paused, T::Resume) | (Completed, T::Reopen) => Ok((Active, None)),
        (Active | Paused, T::Complete) => Ok((Completed, None)),
        (Active | Paused | Completed, T::Archive) => Ok((Archived, Some(status))),
        (Archived, T::Restore) => match pre_archive {
            Some(previous) if previous.can_be_pre_archive() => Ok((previous, None)),
            _ => Err(GoalError::InvalidTransition),
        },
        _ => Err(GoalError::InvalidTransition),
    }
}

/// Funding progress, independent of the manual status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub contributed: Money,
    pub target: Money,
    /// `max(target − contributed, 0)`.
    pub remaining: Money,
    /// `max(contributed − target, 0)`.
    pub excess: Money,
    /// Floor percentage of the target, capped at 100 for the bar.
    pub percent: u8,
    /// `contributed ≥ target`, whatever the manual status.
    pub funded: bool,
}

/// Progress for a target and contributed total. Percentages use `i128` intermediates; money stays
/// exact. A nonpositive target has no progress to report.
#[must_use]
pub fn progress(target: Money, contributed: Money) -> Progress {
    let t = i128::from(target.cents());
    let c = i128::from(contributed.cents());
    let remaining = i64::try_from((t - c).max(0)).unwrap_or(i64::MAX);
    let excess = i64::try_from((c - t).max(0)).unwrap_or(i64::MAX);
    let percent = if t > 0 {
        u8::try_from((c.max(0) * 100 / t).min(100)).unwrap_or(100)
    } else {
        0
    };
    Progress {
        contributed,
        target,
        remaining: Money::from_cents(remaining),
        excess: Money::from_cents(excess),
        percent,
        funded: t > 0 && c >= t,
    }
}

/// What the schedule hint says. Hidden unless the goal is Active, unfunded and has a deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContributionPlan {
    Hidden,
    /// The deadline is today: the full remaining amount.
    DueToday(Money),
    /// The deadline has passed: the full remaining amount.
    Overdue(Money),
    PerPeriod {
        periods: u64,
        per_period: Money,
    },
}

/// `date` advanced by `months` calendar months, anchored on its day and clamped to the last day of
/// the destination month.
#[must_use]
pub fn add_months(date: NaiveDate, months: u32) -> Option<NaiveDate> {
    let index = i64::from(date.year()) * 12 + i64::from(date.month0()) + i64::from(months);
    let year = i32::try_from(index.div_euclid(12)).ok()?;
    let month = u32::try_from(index.rem_euclid(12)).ok()? + 1;
    let target = YearMonth::new(year, month)?;
    NaiveDate::from_ymd_opt(year, month, date.day().min(target.last_day().day()))
}

/// Number of monthly periods from `today` until `target` (after `today`): the smallest positive
/// count of month advances that reaches or passes it.
fn monthly_periods(today: NaiveDate, target: NaiveDate) -> Option<u64> {
    let diff = (i64::from(target.year()) - i64::from(today.year())) * 12
        + (i64::from(target.month()) - i64::from(today.month()));
    let mut n = u32::try_from(diff.max(1)).ok()?;
    if add_months(today, n)? < target {
        n = n.checked_add(1)?;
    }
    Some(u64::from(n))
}

/// The contribution schedule hint for a goal on the injected local `today`.
#[must_use]
pub fn contribution_plan(goal: &Goal, contributed: Money, today: NaiveDate) -> ContributionPlan {
    let progress = progress(goal.target, contributed);
    let Some(target_date) = goal.target_date else {
        return ContributionPlan::Hidden;
    };
    if goal.status != GoalStatus::Active || progress.funded {
        return ContributionPlan::Hidden;
    }
    let remaining = progress.remaining;
    if target_date == today {
        return ContributionPlan::DueToday(remaining);
    }
    if target_date < today {
        return ContributionPlan::Overdue(remaining);
    }
    let days = u64::try_from((target_date - today).num_days()).unwrap_or(1);
    let periods = match goal.frequency {
        Frequency::Daily => Some(days),
        Frequency::Weekly => Some(days.div_ceil(7)),
        Frequency::Monthly => monthly_periods(today, target_date),
    };
    let Some(periods) = periods.filter(|p| *p > 0) else {
        return ContributionPlan::Overdue(remaining);
    };
    let divisor = i128::from(periods);
    let per_period = (i128::from(remaining.cents()) + divisor - 1) / divisor;
    ContributionPlan::PerPeriod {
        periods,
        per_period: Money::from_cents(i64::try_from(per_period).unwrap_or(i64::MAX)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn goal(target: i64, date: Option<NaiveDate>, frequency: Frequency) -> Goal {
        Goal {
            id: 1,
            name: "Trip".into(),
            target: Money::from_cents(target),
            target_date: date,
            frequency,
            attribution_id: None,
            status: GoalStatus::Active,
            pre_archive_status: None,
            created_at: "2026-11-01T00:00:00Z".into(),
            updated_at: "2026-11-01T00:00:00Z".into(),
        }
    }

    fn periods(plan: ContributionPlan) -> (u64, i64) {
        match plan {
            ContributionPlan::PerPeriod {
                periods,
                per_period,
            } => (periods, per_period.cents()),
            other => panic!("expected a per-period plan, got {other:?}"),
        }
    }

    #[test]
    fn goal_schedule_handles_calendar_boundaries() {
        let money = Money::ZERO;
        // Jan 31 -> Feb 28 (and leap Feb 29) is one monthly period.
        let g = goal(1000, Some(d(2027, 2, 28)), Frequency::Monthly);
        assert_eq!(periods(contribution_plan(&g, money, d(2027, 1, 31))).0, 1);
        let g = goal(1000, Some(d(2028, 2, 29)), Frequency::Monthly);
        assert_eq!(periods(contribution_plan(&g, money, d(2028, 1, 31))).0, 1);
        // Jan 31 -> Mar 1 is two.
        let g = goal(1000, Some(d(2027, 3, 1)), Frequency::Monthly);
        assert_eq!(periods(contribution_plan(&g, money, d(2027, 1, 31))).0, 2);
        // Same month, later day: one period. Next month same day: one period.
        let g = goal(1000, Some(d(2027, 1, 20)), Frequency::Monthly);
        assert_eq!(periods(contribution_plan(&g, money, d(2027, 1, 5))).0, 1);
        let g = goal(1000, Some(d(2027, 2, 5)), Frequency::Monthly);
        assert_eq!(periods(contribution_plan(&g, money, d(2027, 1, 5))).0, 1);
        let g = goal(1000, Some(d(2027, 2, 6)), Frequency::Monthly);
        assert_eq!(periods(contribution_plan(&g, money, d(2027, 1, 5))).0, 2);
        // Year boundary.
        let g = goal(1000, Some(d(2027, 1, 15)), Frequency::Monthly);
        assert_eq!(periods(contribution_plan(&g, money, d(2026, 12, 20))).0, 1);
        // Weekly: eight days is two periods; seven is one.
        let g = goal(1000, Some(d(2026, 11, 27)), Frequency::Weekly);
        assert_eq!(periods(contribution_plan(&g, money, d(2026, 11, 19))).0, 2);
        let g = goal(1000, Some(d(2026, 11, 26)), Frequency::Weekly);
        assert_eq!(periods(contribution_plan(&g, money, d(2026, 11, 19))).0, 1);
        // Daily counts day differences.
        let g = goal(1000, Some(d(2026, 11, 24)), Frequency::Daily);
        assert_eq!(
            periods(contribution_plan(&g, money, d(2026, 11, 19))),
            (5, 200)
        );
        // Ceiling cents: 101 over two periods is 51.
        let g = goal(101, Some(d(2026, 11, 21)), Frequency::Daily);
        assert_eq!(
            periods(contribution_plan(&g, money, d(2026, 11, 19))),
            (2, 51)
        );
    }

    #[test]
    fn schedule_due_overdue_and_hidden_states() {
        let today = d(2026, 11, 19);
        let paid = Money::from_cents(300);
        let g = goal(1000, Some(today), Frequency::Monthly);
        assert_eq!(
            contribution_plan(&g, paid, today),
            ContributionPlan::DueToday(Money::from_cents(700))
        );
        let g = goal(1000, Some(d(2026, 11, 18)), Frequency::Weekly);
        assert_eq!(
            contribution_plan(&g, paid, today),
            ContributionPlan::Overdue(Money::from_cents(700))
        );
        // No deadline, funded, paused, completed and archived goals show no plan.
        let none = goal(1000, None, Frequency::Monthly);
        assert_eq!(
            contribution_plan(&none, paid, today),
            ContributionPlan::Hidden
        );
        let mut g = goal(1000, Some(d(2027, 1, 1)), Frequency::Monthly);
        assert_eq!(
            contribution_plan(&g, Money::from_cents(1000), today),
            ContributionPlan::Hidden
        );
        for status in [
            GoalStatus::Paused,
            GoalStatus::Completed,
            GoalStatus::Archived,
        ] {
            g.status = status;
            assert_eq!(contribution_plan(&g, paid, today), ContributionPlan::Hidden);
        }
    }

    #[test]
    fn progress_caps_the_bar_and_reports_the_excess() {
        let p = progress(Money::from_cents(1000), Money::from_cents(250));
        assert_eq!(
            (p.percent, p.remaining.cents(), p.excess.cents()),
            (25, 750, 0)
        );
        assert!(!p.funded);
        let p = progress(Money::from_cents(1000), Money::from_cents(1000));
        assert_eq!(
            (p.percent, p.remaining.cents(), p.excess.cents()),
            (100, 0, 0)
        );
        assert!(p.funded);
        let p = progress(Money::from_cents(1000), Money::from_cents(1500));
        assert_eq!(
            (p.percent, p.remaining.cents(), p.excess.cents()),
            (100, 0, 500)
        );
        // Large values never overflow the percentage arithmetic.
        let p = progress(Money::from_cents(i64::MAX), Money::from_cents(i64::MAX - 1));
        assert_eq!(p.percent, 99);
        assert_eq!(p.remaining.cents(), 1);
    }

    #[test]
    fn transitions_cover_the_whole_workflow() {
        use GoalStatus::{Active, Archived, Completed, Paused};
        use GoalTransition::{Archive, Complete, Pause, Reopen, Restore, Resume};
        assert_eq!(apply_transition(Active, None, Pause), Ok((Paused, None)));
        assert_eq!(apply_transition(Paused, None, Resume), Ok((Active, None)));
        assert_eq!(
            apply_transition(Active, None, Complete),
            Ok((Completed, None))
        );
        assert_eq!(
            apply_transition(Paused, None, Complete),
            Ok((Completed, None))
        );
        assert_eq!(
            apply_transition(Completed, None, Reopen),
            Ok((Active, None))
        );
        for from in [Active, Paused, Completed] {
            let (status, pre) = apply_transition(from, None, Archive).unwrap();
            assert_eq!((status, pre), (Archived, Some(from)));
            assert_eq!(apply_transition(status, pre, Restore), Ok((from, None)));
        }
        for bad in [
            (Active, Resume),
            (Active, Reopen),
            (Active, Restore),
            (Paused, Pause),
            (Completed, Pause),
            (Completed, Complete),
            (Archived, Archive),
            (Archived, Pause),
            (Archived, Complete),
            (Archived, Reopen),
        ] {
            assert_eq!(
                apply_transition(bad.0, Some(Active), bad.1),
                Err(GoalError::InvalidTransition),
                "{bad:?}"
            );
        }
        // An archived goal without a stored status cannot be restored.
        assert!(apply_transition(Archived, None, Restore).is_err());
    }

    #[test]
    fn drafts_validate_without_a_database() {
        let draft = GoalDraft {
            name: "  ".into(),
            target: Money::ZERO,
            target_date: None,
            frequency: Frequency::Monthly,
            attribution_id: None,
        };
        assert_eq!(
            draft.validate(),
            Err(vec![GoalError::NameEmpty, GoalError::TargetNotPositive])
        );
        let today = d(2026, 11, 19);
        let c = ContributionDraft {
            amount: Money::from_cents(1),
            date: today,
            note: Some("  ".into()),
        }
        .normalized();
        assert_eq!(c.note, None);
        assert!(c.validate(today).is_ok());
        let late = ContributionDraft {
            date: d(2026, 11, 20),
            ..c.clone()
        };
        assert_eq!(late.validate(today), Err(vec![GoalError::DateInFuture]));
    }
}
