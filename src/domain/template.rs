//! Entry templates (contract §14.1): a named, reusable entry without a date. Applying one fills
//! the Quick Add form only; nothing is saved until the user saves the entry.

use thiserror::Error;

use super::money::Money;
use super::text::fold;
use super::transaction::{
    Allocation, SplitMode, TransactionType, TxDraft, ValidationError, split_evenly,
};

/// Who is responsible for the entry, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResponsibilityMode {
    /// No one: no allocations.
    None,
    /// One person bears the whole amount.
    Person,
    /// Two or more people, split evenly in ascending person id order.
    Even,
    /// Two or more people with explicit shares (zeros allowed).
    Custom,
}

impl ResponsibilityMode {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            ResponsibilityMode::None => "none",
            ResponsibilityMode::Person => "person",
            ResponsibilityMode::Even => "even",
            ResponsibilityMode::Custom => "custom",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "none" => Some(ResponsibilityMode::None),
            "person" => Some(ResponsibilityMode::Person),
            "even" => Some(ResponsibilityMode::Even),
            "custom" => Some(ResponsibilityMode::Custom),
            _ => Option::None,
        }
    }

    /// The transaction split mode a mode applies as, if any.
    #[must_use]
    pub fn split_mode(self) -> Option<SplitMode> {
        match self {
            ResponsibilityMode::Even => Some(SplitMode::Even),
            ResponsibilityMode::Custom => Some(SplitMode::Custom),
            ResponsibilityMode::None | ResponsibilityMode::Person => Option::None,
        }
    }
}

/// The editable fields of a template: the normal entry fields without a date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateDraft {
    pub name: String,
    pub title: String,
    pub amount: Money,
    pub kind: TransactionType,
    pub category_id: Option<i64>,
    pub payer_id: Option<i64>,
    pub description: Option<String>,
    pub responsibility: ResponsibilityMode,
    pub allocations: Vec<Allocation>,
}

/// A stored template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub id: i64,
    pub draft: TemplateDraft,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/// The form field a template validation message belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TemplateField {
    Name,
    Title,
    Amount,
    Kind,
    Category,
    Payer,
    People,
}

/// Structured template validation failures; the UI localizes each.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateError {
    #[error("the template name is empty")]
    NameEmpty,
    #[error("the title is empty")]
    TitleEmpty,
    #[error("the amount must be greater than zero")]
    AmountNotPositive,
    #[error("settlements cannot be templates")]
    SettlementNotAllowed,
    #[error("income has no payer and no responsibility")]
    IncomeShape,
    #[error("no responsibility means no shares")]
    NoneHasShares,
    #[error("one person must bear the full amount")]
    PersonNeedsOneFullShare,
    #[error("a shared split needs at least two different people")]
    SharedNeedsTwoPeople,
    #[error("even shares do not match the stable even split")]
    EvenSharesNotCanonical,
    #[error("another template already has this name")]
    NameTaken,
    #[error("person #{0} does not exist")]
    PersonMissing(i64),
    #[error("person #{0} is archived")]
    PersonArchived(i64),
    #[error("the category does not exist")]
    CategoryMissing,
    #[error("the category is archived")]
    CategoryArchived,
    #[error("the category cannot be used for this kind of entry")]
    CategoryIncompatible,
    #[error(transparent)]
    Shares(ValidationError),
}

impl TemplateError {
    #[must_use]
    pub fn field(&self) -> TemplateField {
        match self {
            TemplateError::NameEmpty | TemplateError::NameTaken => TemplateField::Name,
            TemplateError::CategoryMissing
            | TemplateError::CategoryArchived
            | TemplateError::CategoryIncompatible => TemplateField::Category,
            TemplateError::PersonMissing(_) | TemplateError::PersonArchived(_) => {
                TemplateField::People
            }
            TemplateError::TitleEmpty => TemplateField::Title,
            TemplateError::AmountNotPositive => TemplateField::Amount,
            TemplateError::SettlementNotAllowed => TemplateField::Kind,
            TemplateError::IncomeShape => TemplateField::Payer,
            TemplateError::NoneHasShares
            | TemplateError::PersonNeedsOneFullShare
            | TemplateError::SharedNeedsTwoPeople
            | TemplateError::EvenSharesNotCanonical
            | TemplateError::Shares(_) => TemplateField::People,
        }
    }
}

/// Collapse internal whitespace runs to single spaces and trim.
#[must_use]
pub fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The key two template names are compared by: trimmed, folded.
#[must_use]
pub fn name_key(name: &str) -> String {
    fold(name.trim())
}

/// Even shares for `people` (any order) in ascending person id order, so the remainder cents are
/// stable no matter how the roster is ordered.
///
/// # Errors
/// `None` when there is nobody to split between or the amount cannot be split.
#[must_use]
pub fn even_allocations(amount: Money, people: &[i64]) -> Option<Vec<Allocation>> {
    let mut ids = people.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let weights = vec![1_u32; ids.len()];
    let shares = split_evenly(amount, &weights).ok()?;
    Some(
        ids.into_iter()
            .zip(shares)
            .map(|(person_id, amount)| Allocation { person_id, amount })
            .collect(),
    )
}

impl TemplateDraft {
    /// Trim text, order allocations by person id and, for an even split, recompute the stable
    /// even shares. Validation runs on the normalized draft.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_string();
        self.title = self.title.trim().to_string();
        self.description = self
            .description
            .take()
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        self.allocations.sort_by_key(|a| a.person_id);
        if self.responsibility == ResponsibilityMode::Even {
            let people: Vec<i64> = self.allocations.iter().map(|a| a.person_id).collect();
            if let Some(shares) = even_allocations(self.amount, &people) {
                self.allocations = shares;
            }
        }
        self
    }

    /// The entry this template produces for `date`.
    #[must_use]
    pub fn to_tx_draft(&self, date: chrono::NaiveDate) -> TxDraft {
        TxDraft {
            title: self.title.clone(),
            amount: self.amount,
            date,
            kind: self.kind,
            category_id: self.category_id,
            payer_id: self.payer_id,
            description: self.description.clone(),
            allocations: self.allocations.clone(),
            split_mode: self.responsibility.split_mode(),
        }
    }

    /// A template from an entry, named `name`. Settlements are not templatable.
    ///
    /// # Errors
    /// [`TemplateError::SettlementNotAllowed`] for a settlement.
    pub fn from_tx_draft(name: &str, draft: &TxDraft) -> Result<Self, TemplateError> {
        if draft.kind == TransactionType::Settlement {
            return Err(TemplateError::SettlementNotAllowed);
        }
        let responsibility = match (draft.split_mode, draft.allocations.len()) {
            (Some(SplitMode::Even), _) => ResponsibilityMode::Even,
            (Some(SplitMode::Custom), _) => ResponsibilityMode::Custom,
            (Option::None, 0) => ResponsibilityMode::None,
            (Option::None, _) => ResponsibilityMode::Person,
        };
        Ok(Self {
            name: name.to_string(),
            title: draft.title.clone(),
            amount: draft.amount,
            kind: draft.kind,
            category_id: draft.category_id,
            payer_id: draft.payer_id,
            description: draft.description.clone(),
            responsibility,
            allocations: draft.allocations.clone(),
        }
        .normalized())
    }

    /// Collects every problem. Run on a [`normalized`](Self::normalized) draft.
    ///
    /// # Errors
    /// The list of [`TemplateError`]s, never empty when `Err`.
    pub fn validate(&self) -> Result<(), Vec<TemplateError>> {
        let mut errors = Vec::new();
        if self.name.trim().is_empty() {
            errors.push(TemplateError::NameEmpty);
        }
        if self.kind == TransactionType::Settlement {
            errors.push(TemplateError::SettlementNotAllowed);
        }
        // Common rules (title, amount, shares) come from the entry validation, with any date.
        if let Err(found) = self.to_tx_draft(chrono::NaiveDate::MIN).validate() {
            for e in found {
                match e {
                    ValidationError::TitleEmpty => errors.push(TemplateError::TitleEmpty),
                    ValidationError::AmountNotPositive => {
                        errors.push(TemplateError::AmountNotPositive);
                    }
                    // Split shape is reported by the responsibility checks below, and settlement
                    // shape by `SettlementNotAllowed`.
                    ValidationError::SplitModeWithoutSharedSplit
                    | ValidationError::SettlementNeedsFrom
                    | ValidationError::SettlementNeedsOneRecipient
                    | ValidationError::SettlementSamePerson
                    | ValidationError::SettlementHasCategory
                    | ValidationError::SettlementHasSplitMode => {}
                    other => errors.push(TemplateError::Shares(other)),
                }
            }
        }
        if self.kind == TransactionType::Income
            && (self.payer_id.is_some() || self.responsibility != ResponsibilityMode::None)
        {
            errors.push(TemplateError::IncomeShape);
        }
        let n = self.allocations.len();
        match self.responsibility {
            ResponsibilityMode::None => {
                if n != 0 {
                    errors.push(TemplateError::NoneHasShares);
                }
            }
            ResponsibilityMode::Person => {
                if n != 1 || self.allocations[0].amount != self.amount {
                    errors.push(TemplateError::PersonNeedsOneFullShare);
                }
            }
            ResponsibilityMode::Even | ResponsibilityMode::Custom => {
                if n < 2 {
                    errors.push(TemplateError::SharedNeedsTwoPeople);
                }
                if self.responsibility == ResponsibilityMode::Even && n >= 2 {
                    let people: Vec<i64> = self.allocations.iter().map(|a| a.person_id).collect();
                    let mut sorted = self.allocations.clone();
                    sorted.sort_by_key(|a| a.person_id);
                    if even_allocations(self.amount, &people).as_deref() != Some(&sorted[..]) {
                        errors.push(TemplateError::EvenSharesNotCanonical);
                    }
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(mode: ResponsibilityMode, allocations: Vec<Allocation>) -> TemplateDraft {
        TemplateDraft {
            name: "Weekly market".into(),
            title: "Market".into(),
            amount: Money::from_cents(10_000),
            kind: TransactionType::Expense,
            category_id: None,
            payer_id: Some(1),
            description: None,
            responsibility: mode,
            allocations,
        }
    }

    fn share(person_id: i64, cents: i64) -> Allocation {
        Allocation {
            person_id,
            amount: Money::from_cents(cents),
        }
    }

    #[test]
    fn template_domain_validation() {
        assert!(
            draft(ResponsibilityMode::None, vec![])
                .normalized()
                .validate()
                .is_ok()
        );
        let mut blank = draft(ResponsibilityMode::None, vec![]);
        blank.name = "  ".into();
        blank.title = String::new();
        blank.amount = Money::ZERO;
        let errors = blank.normalized().validate().unwrap_err();
        assert!(errors.contains(&TemplateError::NameEmpty));
        assert!(errors.contains(&TemplateError::TitleEmpty));
        assert!(errors.contains(&TemplateError::AmountNotPositive));

        let mut settlement = draft(ResponsibilityMode::None, vec![]);
        settlement.kind = TransactionType::Settlement;
        assert_eq!(
            settlement.normalized().validate().unwrap_err(),
            vec![TemplateError::SettlementNotAllowed]
        );

        // None has no allocations.
        assert!(
            draft(ResponsibilityMode::None, vec![share(1, 10_000)])
                .normalized()
                .validate()
                .unwrap_err()
                .contains(&TemplateError::NoneHasShares)
        );
        // Person has exactly one full share.
        assert!(
            draft(ResponsibilityMode::Person, vec![share(1, 10_000)])
                .normalized()
                .validate()
                .is_ok()
        );
        assert!(
            draft(ResponsibilityMode::Person, vec![share(1, 5_000)])
                .normalized()
                .validate()
                .is_err()
        );
        // Shared modes need two distinct people and a matching total.
        assert!(
            draft(ResponsibilityMode::Custom, vec![share(1, 10_000)])
                .normalized()
                .validate()
                .unwrap_err()
                .contains(&TemplateError::SharedNeedsTwoPeople)
        );
        let dup = draft(
            ResponsibilityMode::Custom,
            vec![share(1, 5_000), share(1, 5_000)],
        )
        .normalized()
        .validate()
        .unwrap_err();
        assert!(dup.contains(&TemplateError::Shares(ValidationError::DuplicatePerson(1))));
        assert!(
            draft(
                ResponsibilityMode::Custom,
                vec![share(1, 5_000), share(2, 4_000)]
            )
            .normalized()
            .validate()
            .is_err()
        );
        // Custom keeps explicit zeros.
        assert!(
            draft(
                ResponsibilityMode::Custom,
                vec![share(1, 10_000), share(2, 0)]
            )
            .normalized()
            .validate()
            .is_ok()
        );
        // Income: no payer, no responsibility.
        let mut income = draft(ResponsibilityMode::None, vec![]);
        income.kind = TransactionType::Income;
        assert_eq!(
            income.clone().normalized().validate().unwrap_err(),
            vec![TemplateError::IncomeShape]
        );
        income.payer_id = None;
        assert!(income.normalized().validate().is_ok());
    }

    #[test]
    fn template_even_remainder_is_stable_by_participant_id() {
        // 100,01 across three people: the remainder cent goes to the lowest id, whatever order the
        // caller lists the participants in.
        let mut a = draft(
            ResponsibilityMode::Even,
            vec![share(9, 0), share(2, 0), share(5, 0)],
        );
        a.amount = Money::from_cents(10_001);
        let a = a.normalized();
        assert_eq!(
            a.allocations,
            vec![share(2, 3_334), share(5, 3_334), share(9, 3_333)]
        );
        assert!(a.validate().is_ok());
        let mut b = draft(
            ResponsibilityMode::Even,
            vec![share(5, 0), share(9, 0), share(2, 0)],
        );
        b.amount = Money::from_cents(10_001);
        assert_eq!(b.normalized().allocations, a.allocations);
        // A non-canonical stored even split is rejected.
        let mut bad = a.clone();
        bad.allocations = vec![share(2, 3_333), share(5, 3_334), share(9, 3_334)];
        assert!(
            bad.validate()
                .unwrap_err()
                .contains(&TemplateError::EvenSharesNotCanonical)
        );
    }

    #[test]
    fn applying_fills_an_entry_for_the_given_date() {
        let d = draft(
            ResponsibilityMode::Custom,
            vec![share(1, 7_000), share(2, 3_000)],
        );
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 11).unwrap();
        let entry = d.to_tx_draft(today);
        assert_eq!(entry.date, today);
        assert_eq!(entry.split_mode, Some(SplitMode::Custom));
        assert_eq!(entry.allocations, d.allocations);
        assert!(entry.validate().is_ok());
    }

    #[test]
    fn a_template_round_trips_through_an_entry() {
        let entry = TxDraft {
            title: " Rent ".into(),
            amount: Money::from_cents(10_001),
            date: chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            kind: TransactionType::Expense,
            category_id: Some(3),
            payer_id: Some(1),
            description: Some("  ".into()),
            allocations: vec![share(2, 5_000), share(1, 5_001)],
            split_mode: Some(SplitMode::Even),
        };
        let template = TemplateDraft::from_tx_draft("Rent", &entry).unwrap();
        assert_eq!(template.responsibility, ResponsibilityMode::Even);
        assert_eq!(template.title, "Rent");
        assert_eq!(template.description, None);
        assert_eq!(template.allocations, vec![share(1, 5_001), share(2, 5_000)]);
        assert!(template.validate().is_ok());
        let mut settlement = entry;
        settlement.kind = TransactionType::Settlement;
        assert_eq!(
            TemplateDraft::from_tx_draft("x", &settlement),
            Err(TemplateError::SettlementNotAllowed)
        );
    }

    #[test]
    fn names_compare_folded_and_trimmed() {
        assert_eq!(name_key("  Pão  "), name_key("PAO"));
        assert_eq!(collapse_whitespace("  a   b\tc "), "a b c");
    }

    #[test]
    fn responsibility_codes_round_trip() {
        for mode in [
            ResponsibilityMode::None,
            ResponsibilityMode::Person,
            ResponsibilityMode::Even,
            ResponsibilityMode::Custom,
        ] {
            assert_eq!(ResponsibilityMode::from_code(mode.code()), Some(mode));
        }
        assert_eq!(ResponsibilityMode::from_code("other"), None);
    }
}
