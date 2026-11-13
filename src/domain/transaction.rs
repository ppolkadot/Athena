//! Transactions, drafts, validation and the exact split.

use std::collections::HashSet;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::date::YearMonth;
use super::money::Money;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransactionType {
    Expense,
    Income,
    Refund,
}

impl TransactionType {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            TransactionType::Expense => "expense",
            TransactionType::Income => "income",
            TransactionType::Refund => "refund",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "expense" => Some(TransactionType::Expense),
            "income" => Some(TransactionType::Income),
            "refund" => Some(TransactionType::Refund),
            _ => None,
        }
    }
}

/// How a shared split was entered, so editing reopens in the same mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitMode {
    Even,
    Custom,
}

impl SplitMode {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            SplitMode::Even => "even",
            SplitMode::Custom => "custom",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "even" => Some(SplitMode::Even),
            "custom" => Some(SplitMode::Custom),
            _ => None,
        }
    }
}

/// One person's responsibility for part of a transaction. Not who paid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allocation {
    pub person_id: i64,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: i64,
    pub title: String,
    pub amount: Money,
    pub date: NaiveDate,
    pub kind: TransactionType,
    pub category_id: Option<i64>,
    pub payer_id: Option<i64>,
    pub description: Option<String>,
    pub split_mode: Option<SplitMode>,
    pub allocations: Vec<Allocation>,
}

/// What the user entered; becomes a [`Transaction`] once stored.
#[derive(Debug, Clone, PartialEq)]
pub struct TxDraft {
    pub title: String,
    pub amount: Money,
    pub date: NaiveDate,
    pub kind: TransactionType,
    pub category_id: Option<i64>,
    pub payer_id: Option<i64>,
    pub description: Option<String>,
    pub allocations: Vec<Allocation>,
    pub split_mode: Option<SplitMode>,
}

impl Transaction {
    /// The effect on the balance: expenses subtract, income and refunds add.
    #[must_use]
    pub fn signed_amount(&self) -> Money {
        match self.kind {
            TransactionType::Expense => -self.amount,
            TransactionType::Income | TransactionType::Refund => self.amount,
        }
    }

    /// The editable fields of a stored transaction, for edits and undo.
    #[must_use]
    pub fn to_draft(&self) -> TxDraft {
        TxDraft {
            title: self.title.clone(),
            amount: self.amount,
            date: self.date,
            kind: self.kind,
            category_id: self.category_id,
            payer_id: self.payer_id,
            description: self.description.clone(),
            allocations: self.allocations.clone(),
            split_mode: self.split_mode,
        }
    }
}

/// The form field a validation message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxField {
    Title,
    Amount,
    Date,
    Category,
    Payer,
    People,
}

/// Structured validation failures. The UI turns each into localized copy.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("title is empty")]
    TitleEmpty,
    #[error("amount must be greater than zero")]
    AmountNotPositive,
    #[error("a person appears twice in the split")]
    DuplicatePerson(i64),
    #[error("a share is negative")]
    NegativeShare(i64),
    #[error("shares add up to {actual} but the total is {expected}")]
    ShareSumMismatch { expected: Money, actual: Money },
    #[error("a split mode needs at least two people")]
    SplitModeWithoutSharedSplit,
}

impl ValidationError {
    #[must_use]
    pub fn field(&self) -> TxField {
        match self {
            ValidationError::TitleEmpty => TxField::Title,
            ValidationError::AmountNotPositive => TxField::Amount,
            ValidationError::DuplicatePerson(_)
            | ValidationError::NegativeShare(_)
            | ValidationError::ShareSumMismatch { .. }
            | ValidationError::SplitModeWithoutSharedSplit => TxField::People,
        }
    }
}

impl TxDraft {
    /// Collects every problem so the form can show them all at once.
    ///
    /// # Errors
    /// The list of [`ValidationError`]s, never empty when `Err`.
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        if self.title.trim().is_empty() {
            errors.push(ValidationError::TitleEmpty);
        }
        if self.amount <= Money::ZERO {
            errors.push(ValidationError::AmountNotPositive);
        }
        let mut seen = HashSet::new();
        for alloc in &self.allocations {
            if !seen.insert(alloc.person_id) {
                errors.push(ValidationError::DuplicatePerson(alloc.person_id));
            }
            if alloc.amount.is_negative() {
                errors.push(ValidationError::NegativeShare(alloc.person_id));
            }
        }
        if !self.allocations.is_empty() {
            let actual: Money = self.allocations.iter().map(|a| a.amount).sum();
            if actual != self.amount {
                errors.push(ValidationError::ShareSumMismatch {
                    expected: self.amount,
                    actual,
                });
            }
        }
        if self.split_mode.is_some() && self.allocations.len() < 2 {
            errors.push(ValidationError::SplitModeWithoutSharedSplit);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SplitError {
    #[error("no people to split between")]
    NoWeights,
    #[error("weights add up to zero")]
    ZeroTotalWeight,
    #[error("cannot split a negative amount")]
    NegativeTotal,
}

/// Split `total` in proportion to `weights`. Shares are floored, then the leftover cents go to the
/// largest fractional remainders, ties by list order (owner first). The sum always equals `total`.
///
/// # Errors
/// [`SplitError`] for empty weights, all-zero weights or a negative total.
pub fn split_evenly(total: Money, weights: &[u32]) -> Result<Vec<Money>, SplitError> {
    if weights.is_empty() {
        return Err(SplitError::NoWeights);
    }
    let weight_sum: i128 = weights.iter().map(|w| i128::from(*w)).sum();
    if weight_sum == 0 {
        return Err(SplitError::ZeroTotalWeight);
    }
    if total.is_negative() {
        return Err(SplitError::NegativeTotal);
    }
    let total_cents = i128::from(total.cents());
    let mut shares: Vec<i128> = Vec::with_capacity(weights.len());
    let mut remainders: Vec<(usize, i128)> = Vec::with_capacity(weights.len());
    for (index, weight) in weights.iter().enumerate() {
        let scaled = total_cents * i128::from(*weight);
        shares.push(scaled / weight_sum);
        remainders.push((index, scaled % weight_sum));
    }
    let leftover = total_cents - shares.iter().sum::<i128>();
    remainders.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (index, _) in remainders
        .into_iter()
        .take(usize::try_from(leftover).unwrap_or(0))
    {
        shares[index] += 1;
    }
    Ok(shares
        .into_iter()
        .map(|s| Money::from_cents(i64::try_from(s).unwrap_or(i64::MAX)))
        .collect())
}

/// Month filter, text search and person/category/kind filters for the register.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TxFilter {
    pub month: Option<YearMonth>,
    pub text: String,
    pub category_id: Option<i64>,
    pub person_id: Option<i64>,
    pub shared_only: bool,
    pub kind: Option<TransactionType>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(allocations: Vec<Allocation>, split_mode: Option<SplitMode>) -> TxDraft {
        TxDraft {
            title: "Test".into(),
            amount: Money::from_cents(100),
            date: NaiveDate::from_ymd_opt(2026, 11, 11).unwrap(),
            kind: TransactionType::Expense,
            category_id: None,
            payer_id: None,
            description: None,
            allocations,
            split_mode,
        }
    }

    #[test]
    fn split_evenly_sums_to_total() {
        let parts = split_evenly(Money::from_cents(10_000), &[1, 1, 1]).unwrap();
        assert_eq!(parts.iter().map(|m| m.cents()).sum::<i64>(), 10_000);
        assert_eq!(
            parts,
            vec![
                Money::from_cents(3334),
                Money::from_cents(3333),
                Money::from_cents(3333)
            ]
        );
    }

    #[test]
    fn split_handles_one_cent_and_invalid_weights() {
        assert_eq!(
            split_evenly(Money::from_cents(1), &[1, 1]).unwrap(),
            vec![Money::from_cents(1), Money::from_cents(0)]
        );
        assert_eq!(
            split_evenly(Money::from_cents(100), &[]),
            Err(SplitError::NoWeights)
        );
        assert_eq!(
            split_evenly(Money::from_cents(100), &[0, 0]),
            Err(SplitError::ZeroTotalWeight)
        );
        assert_eq!(
            split_evenly(Money::from_cents(-1), &[1]),
            Err(SplitError::NegativeTotal)
        );
    }

    #[test]
    fn one_cent_across_three_people_sums_exactly() {
        let parts = split_evenly(Money::from_cents(1), &[1, 1, 1]).unwrap();
        assert_eq!(parts, vec![Money::from_cents(1), Money::ZERO, Money::ZERO]);
    }

    #[test]
    fn proportional_split_uses_largest_remainders() {
        let parts = split_evenly(Money::from_cents(100), &[1, 2]).unwrap();
        assert_eq!(parts, vec![Money::from_cents(33), Money::from_cents(67)]);
    }

    #[test]
    fn draft_rejects_bad_allocations() {
        let d = draft(
            vec![Allocation {
                person_id: 1,
                amount: Money::from_cents(60),
            }],
            None,
        );
        let errors = d.validate().unwrap_err();
        assert!(matches!(
            errors[0],
            ValidationError::ShareSumMismatch { .. }
        ));
        assert_eq!(errors[0].field(), TxField::People);
    }

    #[test]
    fn draft_rejects_split_mode_without_a_shared_split() {
        let one = draft(
            vec![Allocation {
                person_id: 1,
                amount: Money::from_cents(100),
            }],
            Some(SplitMode::Even),
        );
        assert!(one.validate().is_err());
        let none = TxDraft {
            allocations: vec![],
            split_mode: Some(SplitMode::Custom),
            ..one
        };
        assert!(none.validate().is_err());
    }

    #[test]
    fn draft_accepts_zero_cent_share() {
        let d = draft(
            vec![
                Allocation {
                    person_id: 1,
                    amount: Money::from_cents(100),
                },
                Allocation {
                    person_id: 2,
                    amount: Money::ZERO,
                },
            ],
            Some(SplitMode::Even),
        );
        assert!(d.validate().is_ok());
    }

    #[test]
    fn draft_collects_every_problem() {
        let mut d = draft(
            vec![
                Allocation {
                    person_id: 1,
                    amount: Money::from_cents(50),
                },
                Allocation {
                    person_id: 1,
                    amount: Money::from_cents(-10),
                },
            ],
            None,
        );
        d.title = "   ".into();
        d.amount = Money::ZERO;
        let errors = d.validate().unwrap_err();
        assert!(errors.contains(&ValidationError::TitleEmpty));
        assert!(errors.contains(&ValidationError::AmountNotPositive));
        assert!(errors.contains(&ValidationError::DuplicatePerson(1)));
        assert!(errors.contains(&ValidationError::NegativeShare(1)));
    }

    #[test]
    fn signed_amount_follows_the_kind() {
        let mut tx = crate::shared::test_fixtures::tx_on(2026, 11, 11);
        assert_eq!(tx.signed_amount(), Money::from_cents(-100));
        tx.kind = TransactionType::Income;
        assert_eq!(tx.signed_amount(), Money::from_cents(100));
        tx.kind = TransactionType::Refund;
        assert_eq!(tx.signed_amount(), Money::from_cents(100));
    }

    #[test]
    fn unassigned_draft_is_valid() {
        assert!(draft(vec![], None).validate().is_ok());
    }

    #[test]
    fn type_codes_round_trip() {
        for kind in [
            TransactionType::Expense,
            TransactionType::Income,
            TransactionType::Refund,
        ] {
            assert_eq!(TransactionType::from_code(kind.code()), Some(kind));
        }
        assert_eq!(SplitMode::from_code("custom"), Some(SplitMode::Custom));
        assert_eq!(SplitMode::from_code("x"), None);
    }

    proptest::proptest! {
        #[test]
        fn even_split_always_sums_and_differs_by_at_most_one_cent(total in 0i64..10_000_000, n in 1usize..8) {
            let weights = vec![1u32; n];
            let shares = split_evenly(Money::from_cents(total), &weights).unwrap();
            let cents: Vec<i64> = shares.iter().map(|m| m.cents()).collect();
            proptest::prop_assert_eq!(cents.iter().sum::<i64>(), total);
            let (min, max) = (cents.iter().min().unwrap(), cents.iter().max().unwrap());
            proptest::prop_assert!(max - min <= 1);
        }

        #[test]
        fn weighted_split_always_sums(total in 0i64..10_000_000, weights in proptest::collection::vec(0u32..50, 1..6)) {
            proptest::prop_assume!(weights.iter().any(|w| *w > 0));
            let shares = split_evenly(Money::from_cents(total), &weights).unwrap();
            proptest::prop_assert_eq!(shares.iter().map(|m| m.cents()).sum::<i64>(), total);
        }
    }
}
