//! Quick-add modal: log an expense, income or refund in seconds. Amount first, Enter saves,
//! Esc closes, and after a save the form is ready for the next entry.
//!
//! This file holds the form state (pure and tested) and the egui modal that edits it.

use chrono::NaiveDate;
use eframe::egui::{self, Align, Button, ComboBox, Id, Key, Layout, Modifiers, RichText};

use crate::app::view_model::ViewModel;
use crate::domain::category::Category;
use crate::domain::date::YearMonth;
use crate::domain::money::{Money, MoneyError};
use crate::domain::person::Person;
use crate::domain::quick_entry::{ParsedLine, QuickEntryError, parse_line};
use crate::domain::settings::Language;
use crate::domain::template::{
    ResponsibilityMode, Template, TemplateDraft, TemplateError, TemplateIssue,
};
use crate::domain::transaction::{
    Allocation, SplitMode, Transaction, TransactionType, TxDraft, ValidationError, split_evenly,
};
use crate::i18n::{
    date_example, format_date, kind_label, money_error_message, month_label, parse_date, strings,
    template_error_message, validation_message,
};
use crate::ui::components::money_input;
use crate::ui::quick_add_prompts::{self as prompts, NameChoice, PromptChoice};
use crate::ui::theme::{Palette, palette};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickAddMode {
    Create,
    Edit(i64),
    /// Editing a stored template from Settings: the same form without a date.
    Template(i64),
}

/// Stable id of the one-line entry field, so Enter handling can tell where focus was.
const LINE_ID: &str = "quick-add-line";

/// A reference inherited from a template that needs the user's explicit repair before saving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    Category(i64),
    Payer(i64),
    Person(i64),
}

/// What the user asked for that would replace or abandon a dirty form. It waits in the modal,
/// behind a Keep editing / Discard prompt, and is performed exactly once on Discard.
#[derive(Debug, Clone, PartialEq)]
pub enum QuickAddIntent {
    Close,
    ApplyTemplate(i64),
    ReplaceWithParsed(ParsedLine, TransactionType),
    /// The native window was asked to close.
    CloseWindow,
}

/// What a confirmed save will do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingSave {
    Create,
    CreateFromTemplate(i64),
    Edit(i64),
}

/// The first entry a possible duplicate matched, for the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateExample {
    pub date: NaiveDate,
    pub amount: Money,
    pub title: String,
}

/// The user's pending decision on a possible duplicate: bound to exactly one action, its draft
/// and the normalized set of matching rows, and valid only while none of them changes.
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateConsent {
    pub save: PendingSave,
    pub draft: TxDraft,
    pub match_ids: Vec<i64>,
    pub first: DuplicateExample,
    pub count: usize,
    pub store_version: u64,
}

/// Why a template could not be named and saved from the Save as template prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptError {
    Template(TemplateError),
    Failed,
}

/// The naming prompt of Save as template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplatePrompt {
    pub draft: TemplateDraft,
    pub name: String,
    pub error: Option<PromptError>,
}

/// Raw editable values, compared to detect unsaved changes. Focus, expanded sections, menus and
/// error decorations are deliberately not part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FormSnapshot {
    title: String,
    amount_input: String,
    date_text: String,
    kind: TransactionType,
    category_id: Option<i64>,
    payer_id: Option<i64>,
    split: SplitChoice,
    shared_amounts: Vec<(i64, String)>,
    description: String,
    line_input: String,
    template_name: String,
}

/// Who is responsible for an expense or refund.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitChoice {
    Person(i64),
    Shared,
    NoOne,
}

/// Which part of the form a problem belongs to, for inline display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Amount,
    Title,
    Date,
    People,
    Category,
    Payer,
    Name,
    General,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickAddError {
    Amount(MoneyError),
    /// One person's share could not be read.
    Share(i64, MoneyError),
    Date,
    Validation(ValidationError),
    /// A template reference that must be repaired first.
    Repair(Repair),
    /// A template rule (name, shape) the form breaks.
    Template(TemplateError),
    /// Looking for duplicates failed, so nothing was saved.
    DuplicateLookup,
    /// The store refused the save.
    Save,
}

impl QuickAddError {
    #[must_use]
    pub fn field(&self) -> FormField {
        match self {
            QuickAddError::Amount(_)
            | QuickAddError::Validation(ValidationError::AmountNotPositive) => FormField::Amount,
            QuickAddError::Validation(ValidationError::TitleEmpty) => FormField::Title,
            QuickAddError::Date => FormField::Date,
            QuickAddError::Share(..)
            | QuickAddError::Validation(_)
            | QuickAddError::Repair(Repair::Person(_)) => FormField::People,
            QuickAddError::Repair(Repair::Category(_)) => FormField::Category,
            QuickAddError::Repair(Repair::Payer(_)) => FormField::Payer,
            QuickAddError::Template(e) => match e {
                TemplateError::NameEmpty | TemplateError::NameTaken => FormField::Name,
                TemplateError::CategoryMissing
                | TemplateError::CategoryArchived
                | TemplateError::CategoryIncompatible => FormField::Category,
                TemplateError::PersonMissing(_)
                | TemplateError::PersonArchived(_)
                | TemplateError::NoneHasShares
                | TemplateError::PersonNeedsOneFullShare
                | TemplateError::SharedNeedsTwoPeople
                | TemplateError::EvenSharesNotCanonical
                | TemplateError::Shares(_) => FormField::People,
                _ => FormField::General,
            },
            QuickAddError::DuplicateLookup | QuickAddError::Save => FormField::General,
        }
    }
}

/// Localized text for a form problem. `name_of` resolves a person id to a display name.
#[must_use]
pub fn error_message(
    error: &QuickAddError,
    language: Language,
    name_of: impl Fn(i64) -> String,
) -> String {
    let s = strings(language);
    match error {
        QuickAddError::Amount(e) => money_error_message(*e, language).to_string(),
        QuickAddError::Share(person, e) => {
            format!(
                "{}: {}",
                name_of(*person),
                money_error_message(*e, language)
            )
        }
        QuickAddError::Date => s
            .v_date_invalid
            .replace("{example}", &date_example(language)),
        QuickAddError::Validation(v) => validation_message(v, language),
        QuickAddError::Repair(Repair::Category(_)) => s.qa_repair_category.to_string(),
        QuickAddError::Repair(Repair::Payer(_)) => s.qa_repair_payer.to_string(),
        QuickAddError::Repair(Repair::Person(id)) => {
            s.qa_repair_person.replace("{name}", &name_of(*id))
        }
        QuickAddError::Template(e) => template_error_message(e, language),
        QuickAddError::DuplicateLookup => s.qa_dup_lookup_failed.to_string(),
        QuickAddError::Save => s.qa_save_failed.to_string(),
    }
}

/// Plain form state; its flags are independent switches, not a state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct QuickAddState {
    pub mode: QuickAddMode,
    pub title: String,
    pub amount_input: String,
    /// Typed in the language's own format.
    pub date_text: String,
    pub kind: TransactionType,
    pub category_id: Option<i64>,
    pub payer_id: Option<i64>,
    pub split: SplitChoice,
    /// One entry per person who can share the entry, in picker order.
    pub shared_amounts: Vec<(i64, String)>,
    pub description: String,
    pub show_more: bool,
    pub errors: Vec<QuickAddError>,
    /// True once the user edits a share by hand, so a changed total no longer overwrites it.
    pub shared_dirty: bool,
    /// Ask the modal to focus the amount field on the next frame.
    pub focus_amount: bool,
    /// Same for the title and date fields, used after a refused save.
    pub focus_title: bool,
    pub focus_date: bool,
    /// While true the payer follows the responsible person.
    payer_auto: bool,
    /// Set when recording a settlement while viewing a different month than `today`'s month.
    pub settlement_month_mismatch: Option<YearMonth>,
    person_ids: Vec<i64>,
    language: Language,
    /// The template this form was filled from; saving records its use.
    pub template_id: Option<i64>,
    /// The name field, used when editing a stored template.
    pub template_name: String,
    /// The one-line entry text and what is wrong with it.
    pub line_input: String,
    pub line_error: Option<QuickEntryError>,
    /// A parsed negative amount waiting for the user to choose Expense or Refund.
    pub kind_choice: Option<ParsedLine>,
    /// Set when this entry came from a one-line parse, so a save returns to the empty line.
    pub from_line: bool,
    /// Ask the modal to focus the one-line field on the next frame.
    pub focus_line: bool,
    /// A request that would discard changes, waiting for Keep editing / Discard.
    pub pending_intent: Option<QuickAddIntent>,
    /// A possible-duplicate decision waiting for Keep editing / Save anyway.
    pub duplicate: Option<DuplicateConsent>,
    /// The Save as template naming prompt.
    pub template_prompt: Option<TemplatePrompt>,
    /// Focus the prompt's safe choice (or name field) on its first drawn frame.
    pub focus_prompt: bool,
    /// The widget last edited, to return to when a prompt is dismissed.
    pub last_focus: Option<Id>,
    /// The exact people of a template's shared split, apart from the picker's candidates.
    participants: Option<Vec<i64>>,
    /// References inherited from a template that were broken when it was applied.
    broken: Vec<Repair>,
    /// Recording a settlement from the settlement card: no templates, no one-line entry.
    settlement_locked: bool,
    baseline: Option<FormSnapshot>,
}

impl QuickAddState {
    /// A blank entry. `people` are the active people, owner first. The responsible person and
    /// the payer both default to the first of them.
    #[must_use]
    pub fn new(people: &[Person], today: NaiveDate, language: Language) -> Self {
        let first = people.first().map(|p| p.id);
        let mut state = Self {
            mode: QuickAddMode::Create,
            title: String::new(),
            amount_input: String::new(),
            date_text: format_date(today, language),
            kind: TransactionType::Expense,
            category_id: None,
            payer_id: first,
            split: first.map_or(SplitChoice::NoOne, SplitChoice::Person),
            shared_amounts: Vec::new(),
            description: String::new(),
            show_more: false,
            errors: Vec::new(),
            shared_dirty: false,
            focus_amount: true,
            focus_title: false,
            focus_date: false,
            payer_auto: true,
            settlement_month_mismatch: None,
            person_ids: people.iter().map(|p| p.id).collect(),
            language,
            template_id: None,
            template_name: String::new(),
            line_input: String::new(),
            line_error: None,
            kind_choice: None,
            from_line: false,
            focus_line: false,
            pending_intent: None,
            duplicate: None,
            template_prompt: None,
            focus_prompt: false,
            last_focus: None,
            participants: None,
            broken: Vec::new(),
            settlement_locked: false,
            baseline: None,
        };
        state.rebase();
        state
    }

    #[must_use]
    pub fn for_settlement(
        people: &[Person],
        from: i64,
        to: i64,
        amount: Money,
        today: NaiveDate,
        title: String,
        language: Language,
    ) -> Self {
        let mut state = Self::new(people, today, language);
        state.mode = QuickAddMode::Create;
        state.kind = TransactionType::Settlement;
        state.payer_id = Some(from);
        state.payer_auto = false;
        state.split = SplitChoice::Person(to);
        state.category_id = None;
        state.shared_dirty = false;
        state.shared_amounts.clear();
        state.amount_input = amount.br_number();
        state.title = title;
        state.focus_amount = false;
        state.settlement_locked = true;
        state.rebase();
        state
    }

    /// Record a non-blocking date warning when opened via Settlement Record from a month other
    /// than `today`'s month (D-P3 Option C).
    #[must_use]
    pub fn with_viewed_month_warning(mut self, viewed: YearMonth, today: NaiveDate) -> Self {
        if viewed != YearMonth::from_date(today) {
            self.settlement_month_mismatch = Some(viewed);
        }
        self
    }

    /// Reopen a stored transaction for editing, in the split mode it was saved with. People who
    /// are on the entry but archived stay in the split so the history remains editable.
    #[must_use]
    pub fn from_transaction(tx: &Transaction, people: &[Person], language: Language) -> Self {
        let mut state = Self::new(people, tx.date, language);
        for allocation in &tx.allocations {
            if !state.person_ids.contains(&allocation.person_id) {
                state.person_ids.push(allocation.person_id);
            }
        }
        state.mode = QuickAddMode::Edit(tx.id);
        state.title.clone_from(&tx.title);
        state.amount_input = tx.amount.br_number();
        state.kind = tx.kind;
        state.category_id = tx.category_id;
        state.payer_id = tx.payer_id;
        state.payer_auto = false;
        state.description = tx.description.clone().unwrap_or_default();
        state.show_more = !state.description.is_empty();
        state.focus_amount = false;
        state.split = match tx.allocations.as_slice() {
            [] => SplitChoice::NoOne,
            [one] => SplitChoice::Person(one.person_id),
            _ => SplitChoice::Shared,
        };
        if state.split == SplitChoice::Shared {
            state.shared_dirty = tx.split_mode == Some(SplitMode::Custom);
            state.shared_amounts = state
                .person_ids
                .iter()
                .map(|id| {
                    // A person who was not on the entry starts blank, meaning "not in the split".
                    let share = tx
                        .allocations
                        .iter()
                        .find(|a| a.person_id == *id)
                        .map_or_else(String::new, |a| a.amount.br_number());
                    (*id, share)
                })
                .collect();
        }
        state.rebase();
        state
    }

    /// Choose who is responsible. While the payer is automatic it follows: the person chosen, or
    /// the owner (the first person) when the entry is shared or unassigned.
    pub fn set_split(&mut self, choice: SplitChoice) {
        self.split = choice;
        if self.payer_auto {
            self.payer_id = match choice {
                SplitChoice::Person(id) => Some(id),
                SplitChoice::Shared | SplitChoice::NoOne => self.person_ids.first().copied(),
            };
        }
        if choice == SplitChoice::Shared {
            self.apply_amount_split();
        }
    }

    /// Choose the payer by hand; the payer then stops following the responsible person.
    pub fn set_payer(&mut self, payer: Option<i64>) {
        self.payer_id = payer;
        self.payer_auto = false;
    }

    /// Choose the recipient for a settlement without altering the payer.
    pub fn set_recipient(&mut self, recipient: Option<i64>) {
        self.split = recipient.map_or(SplitChoice::NoOne, SplitChoice::Person);
    }

    /// Switch between expense, income, refund and settlement. `allowed_categories` are the
    /// category ids valid for the new kind; a chosen category outside them is cleared.
    /// Entering settlement clears category and split-mode data while preserving shared fields.
    pub fn set_kind(&mut self, kind: TransactionType, allowed_categories: &[i64]) {
        self.kind = kind;
        if kind == TransactionType::Settlement {
            self.category_id = None;
            self.shared_dirty = false;
            self.shared_amounts.clear();
            if self.payer_id.is_none() {
                self.payer_id = self.person_ids.first().copied();
            }
            self.payer_auto = false;
            let recipient = match self.split {
                SplitChoice::Person(id) if Some(id) != self.payer_id => Some(id),
                _ => self
                    .person_ids
                    .iter()
                    .copied()
                    .find(|id| Some(*id) != self.payer_id),
            };
            self.split = recipient.map_or(SplitChoice::NoOne, SplitChoice::Person);
            return;
        }
        self.settlement_month_mismatch = None;
        if self
            .category_id
            .is_some_and(|id| !allowed_categories.contains(&id))
        {
            self.category_id = None;
        }
    }

    /// Fill the per-person shares evenly from the total, unless the user has edited a share.
    pub fn apply_amount_split(&mut self) {
        if self.shared_dirty {
            return;
        }
        let people = self.split_people();
        let shares = Money::parse(&self.amount_input)
            .ok()
            .filter(|m| !m.is_negative())
            .and_then(|m| split_evenly(m, &vec![1; people.len()]).ok());
        self.shared_amounts = match shares {
            Some(shares) => people
                .iter()
                .copied()
                .zip(shares.iter().map(|m| m.br_number()))
                .collect(),
            None => people.iter().map(|id| (*id, String::new())).collect(),
        };
    }

    /// Hand-edit one share. The shares stay as typed until [`QuickAddState::reset_even`].
    pub fn edit_share(&mut self, person_id: i64, text: String) {
        if let Some(entry) = self
            .shared_amounts
            .iter_mut()
            .find(|(id, _)| *id == person_id)
        {
            entry.1 = text;
            self.shared_dirty = true;
        }
    }

    /// Go back to an even split and forget hand edits.
    pub fn reset_even(&mut self) {
        self.shared_dirty = false;
        self.apply_amount_split();
    }

    /// Everything entered, validated: the draft to save, or every problem found.
    ///
    /// # Errors
    /// The list of [`QuickAddError`]s, never empty when `Err`.
    pub fn to_draft(&self) -> Result<TxDraft, Vec<QuickAddError>> {
        let mut errors = Vec::new();
        let amount = match Money::parse(&self.amount_input) {
            Ok(amount) => amount,
            Err(e) => {
                errors.push(QuickAddError::Amount(e));
                Money::ZERO
            }
        };
        let date = parse_date(&self.date_text, self.language);
        if date.is_none() {
            errors.push(QuickAddError::Date);
        }

        let mut allocations = Vec::new();
        let mut payer_id = self.payer_id;
        let mut split_mode = None;
        match (self.kind, self.split) {
            (TransactionType::Income, _) => payer_id = None,
            (TransactionType::Settlement, SplitChoice::Person(id)) => {
                allocations.push(Allocation {
                    person_id: id,
                    amount,
                });
            }
            (TransactionType::Settlement, _) | (_, SplitChoice::NoOne) => {}
            (_, SplitChoice::Person(id)) => allocations.push(Allocation {
                person_id: id,
                amount,
            }),
            (_, SplitChoice::Shared) => {
                for (person_id, text) in &self.shared_amounts {
                    // A blank share means the person is not part of this split.
                    if text.trim().is_empty() {
                        continue;
                    }
                    match Money::parse(text) {
                        Ok(share) => allocations.push(Allocation {
                            person_id: *person_id,
                            amount: share,
                        }),
                        Err(e) => errors.push(QuickAddError::Share(*person_id, e)),
                    }
                }
                let even = split_evenly(amount, &vec![1; allocations.len().max(1)]).ok();
                let is_even = even.is_some_and(|shares| {
                    shares.len() == allocations.len()
                        && shares.iter().zip(&allocations).all(|(s, a)| *s == a.amount)
                });
                split_mode = Some(if self.shared_dirty || !is_even {
                    SplitMode::Custom
                } else {
                    SplitMode::Even
                });
            }
        }

        // A field that cannot be read blocks building the draft; still report a missing title.
        let (true, Some(date)) = (errors.is_empty(), date) else {
            if self.title.trim().is_empty() {
                errors.push(QuickAddError::Validation(ValidationError::TitleEmpty));
            }
            return Err(errors);
        };
        let description = self.description.trim();
        let draft = TxDraft {
            title: self.title.trim().to_string(),
            amount,
            date,
            kind: self.kind,
            category_id: self.category_id,
            payer_id,
            description: (!description.is_empty()).then(|| description.to_string()),
            allocations,
            split_mode,
        };
        if let Err(found) = draft.validate() {
            errors.extend(found.into_iter().map(QuickAddError::Validation));
        }
        errors.extend(
            self.current_repairs()
                .into_iter()
                .map(QuickAddError::Repair),
        );
        if errors.is_empty() {
            Ok(draft)
        } else {
            Err(errors)
        }
    }

    /// After a refused save, put the keyboard on the first field with a problem. Pressing Enter
    /// makes a text field let go of focus, so without this the user would have to click back in.
    pub fn focus_first_error(&mut self) {
        match self.errors.first().map(QuickAddError::field) {
            Some(FormField::Amount) => self.focus_amount = true,
            Some(FormField::Title) => self.focus_title = true,
            Some(FormField::Date) => self.focus_date = true,
            _ => {}
        }
    }

    /// Ready the form for the next entry after a successful save. The date, kind, responsible
    /// person and payer carry over; the rest clears and the amount field takes focus.
    pub fn reset_for_next(&mut self) {
        self.title.clear();
        self.amount_input.clear();
        self.description.clear();
        self.category_id = None;
        self.show_more = false;
        self.errors.clear();
        self.settlement_month_mismatch = None;
        self.shared_dirty = false;
        self.shared_amounts.clear();
        self.focus_amount = true;
        self.template_id = None;
        self.participants = None;
        self.broken.clear();
        self.line_input.clear();
        self.line_error = None;
        self.kind_choice = None;
        self.duplicate = None;
        if self.split == SplitChoice::Shared {
            self.apply_amount_split();
        }
        self.rebase();
    }
}

impl QuickAddState {
    fn snapshot(&self) -> FormSnapshot {
        FormSnapshot {
            title: self.title.clone(),
            amount_input: self.amount_input.clone(),
            date_text: self.date_text.clone(),
            kind: self.kind,
            category_id: self.category_id,
            payer_id: self.payer_id,
            split: self.split,
            shared_amounts: self.shared_amounts.clone(),
            description: self.description.clone(),
            line_input: self.line_input.clone(),
            template_name: self.template_name.clone(),
        }
    }

    /// Take the current values as the clean state: at open, after a confirmed replacement and
    /// after a successful create reset.
    pub fn rebase(&mut self) {
        self.baseline = Some(self.snapshot());
    }

    /// Whether any raw value, including invalid input and unparsed one-line text, differs from the
    /// clean state.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.baseline
            .as_ref()
            .is_some_and(|b| *b != self.snapshot())
    }

    /// Like [`Self::is_dirty`] but ignoring the one-line text, which a parse consumes.
    #[must_use]
    pub fn structured_dirty(&self) -> bool {
        self.baseline.as_ref().is_some_and(|b| {
            let mut now = self.snapshot();
            now.line_input.clone_from(&b.line_input);
            *b != now
        })
    }

    /// Templates and the one-line field are for a normal new entry only.
    #[must_use]
    pub fn create_tools_visible(&self) -> bool {
        self.mode == QuickAddMode::Create && !self.settlement_locked
    }

    /// Whether a child prompt (discard, duplicate, template name) owns the keyboard.
    #[must_use]
    pub fn has_child_prompt(&self) -> bool {
        self.pending_intent.is_some() || self.duplicate.is_some() || self.template_prompt.is_some()
    }

    /// The people an even split spreads over: the template's exact participants in ascending id
    /// order, otherwise every candidate in picker order.
    fn split_people(&self) -> Vec<i64> {
        match &self.participants {
            Some(ids) => {
                let mut ids = ids.clone();
                ids.sort_unstable();
                ids
            }
            None => self.person_ids.clone(),
        }
    }

    fn reset_fields(&mut self, today: NaiveDate) {
        let first = self.person_ids.first().copied();
        self.title.clear();
        self.amount_input.clear();
        self.date_text = format_date(today, self.language);
        self.kind = TransactionType::Expense;
        self.category_id = None;
        self.payer_id = first;
        self.payer_auto = true;
        self.split = first.map_or(SplitChoice::NoOne, SplitChoice::Person);
        self.shared_amounts.clear();
        self.shared_dirty = false;
        self.description.clear();
        self.show_more = false;
        self.errors.clear();
        self.settlement_month_mismatch = None;
        self.template_id = None;
        self.participants = None;
        self.broken.clear();
        self.line_input.clear();
        self.from_line = false;
        self.line_error = None;
        self.kind_choice = None;
        self.duplicate = None;
    }

    /// The template references that are still in the form and still broken. They block saving
    /// until the user replaces or clears them.
    #[must_use]
    pub fn current_repairs(&self) -> Vec<Repair> {
        self.broken
            .iter()
            .copied()
            .filter(|repair| match *repair {
                Repair::Category(id) => {
                    self.category_id == Some(id) && self.kind != TransactionType::Settlement
                }
                Repair::Payer(id) => {
                    self.payer_id == Some(id) && self.kind != TransactionType::Income
                }
                Repair::Person(id) => self.uses_person(id),
            })
            .collect()
    }

    fn uses_person(&self, id: i64) -> bool {
        if matches!(
            self.kind,
            TransactionType::Income | TransactionType::Settlement
        ) {
            return false;
        }
        match self.split {
            SplitChoice::Person(person) => person == id,
            SplitChoice::Shared => self
                .shared_amounts
                .iter()
                .any(|(person, text)| *person == id && !text.trim().is_empty()),
            SplitChoice::NoOne => false,
        }
    }

    /// Fill every field from `template` for `today`, keeping its exact participants and its
    /// payer (including none) as explicit values. Nothing is saved. Broken references are kept
    /// visible and must be repaired before the entry can be saved.
    pub fn apply_template(
        &mut self,
        template: &Template,
        people: &[Person],
        categories: &[Category],
        today: NaiveDate,
    ) {
        let d = &template.draft;
        self.reset_fields(today);
        self.title.clone_from(&d.title);
        self.amount_input = d.amount.br_number();
        self.kind = d.kind;
        self.category_id = d.category_id;
        self.payer_id = d.payer_id;
        self.payer_auto = false;
        self.description = d.description.clone().unwrap_or_default();
        self.show_more = !self.description.is_empty();
        let mut allocations = d.allocations.clone();
        allocations.sort_by_key(|a| a.person_id);
        self.split = match (d.responsibility, allocations.first()) {
            (ResponsibilityMode::Person, Some(only)) => SplitChoice::Person(only.person_id),
            (ResponsibilityMode::Even | ResponsibilityMode::Custom, _) => SplitChoice::Shared,
            _ => SplitChoice::NoOne,
        };
        if self.split == SplitChoice::Shared {
            self.participants = Some(allocations.iter().map(|a| a.person_id).collect());
            self.shared_dirty = d.responsibility == ResponsibilityMode::Custom;
            self.shared_amounts = allocations
                .iter()
                .map(|a| (a.person_id, a.amount.br_number()))
                .collect();
        }
        self.broken = d
            .issues(people, categories)
            .into_iter()
            .filter_map(|issue| match issue {
                TemplateIssue::CategoryMissing
                | TemplateIssue::CategoryArchived
                | TemplateIssue::CategoryIncompatible => d.category_id.map(Repair::Category),
                TemplateIssue::PayerMissing | TemplateIssue::PayerArchived => {
                    d.payer_id.map(Repair::Payer)
                }
                TemplateIssue::PersonMissing(id) | TemplateIssue::PersonArchived(id) => {
                    Some(Repair::Person(id))
                }
            })
            .collect();
        self.template_id = Some(template.id);
        self.focus_amount = true;
        self.rebase();
    }

    /// The form for editing a stored template from Settings: all fields, a name, no date.
    #[must_use]
    pub fn for_template(
        template: &Template,
        people: &[Person],
        all_people: &[Person],
        categories: &[Category],
        today: NaiveDate,
        language: Language,
    ) -> Self {
        let mut state = Self::new(people, today, language);
        state.apply_template(template, all_people, categories, today);
        state.mode = QuickAddMode::Template(template.id);
        state.template_id = None;
        state.template_name.clone_from(&template.draft.name);
        state.focus_amount = false;
        state.rebase();
        state
    }

    /// The template for this form under `name`, validated as a stored template would be.
    ///
    /// # Errors
    /// Every form or template problem found.
    pub fn to_template_draft(&self, name: &str) -> Result<TemplateDraft, Vec<QuickAddError>> {
        let draft = self.to_draft()?;
        let template = TemplateDraft::from_tx_draft(name, &draft)
            .map_err(|e| vec![QuickAddError::Template(e)])?;
        match template.validate() {
            Ok(()) => Ok(template),
            Err(found) => Err(found.into_iter().map(QuickAddError::Template).collect()),
        }
    }

    /// Parse the one-line text. A valid positive line is returned as a replacement for the form;
    /// a negative one waits for the user to choose Expense or Refund; an invalid one leaves the
    /// line editable with the reason.
    pub fn review_line(&mut self, today: NaiveDate) -> Option<(ParsedLine, TransactionType)> {
        self.line_error = None;
        self.kind_choice = None;
        match parse_line(&self.line_input, today) {
            Err(e) => {
                self.line_error = Some(e);
                self.focus_line = true;
                None
            }
            Ok(parsed) if parsed.negative => {
                self.kind_choice = Some(parsed);
                None
            }
            Ok(parsed) => Some((parsed, TransactionType::Expense)),
        }
    }

    /// Resolve a pending negative amount as `kind` (Expense or Refund).
    pub fn choose_line_kind(
        &mut self,
        kind: TransactionType,
    ) -> Option<(ParsedLine, TransactionType)> {
        self.kind_choice.take().map(|parsed| (parsed, kind))
    }

    fn apply_parsed(&mut self, parsed: &ParsedLine, kind: TransactionType) {
        self.reset_fields(parsed.date);
        self.title.clone_from(&parsed.title);
        self.amount_input = parsed.amount.br_number();
        self.kind = kind;
        self.line_input.clear();
        self.from_line = true;
        self.focus_amount = false;
        self.focus_title = true;
        self.rebase();
    }

    /// After a one-line entry saved: a fresh expense form with the empty line focused.
    pub fn reset_for_line(&mut self, today: NaiveDate) {
        self.reset_fields(today);
        self.line_input.clear();
        self.from_line = false;
        self.focus_amount = false;
        self.focus_line = true;
        self.rebase();
    }

    /// Ask to abandon or replace the form. A dirty form waits behind the discard prompt; a clean
    /// one performs the intent now.
    pub fn request(&mut self, intent: QuickAddIntent, vm: &ViewModel) -> Option<QuickAddEvent> {
        let guard = match intent {
            QuickAddIntent::ReplaceWithParsed(..) => self.structured_dirty(),
            _ => self.is_dirty(),
        };
        if guard {
            self.pending_intent = Some(intent);
            self.focus_prompt = true;
            None
        } else {
            self.perform_intent(intent, vm)
        }
    }

    /// Do what was asked, once.
    pub fn perform_intent(
        &mut self,
        intent: QuickAddIntent,
        vm: &ViewModel,
    ) -> Option<QuickAddEvent> {
        self.pending_intent = None;
        match intent {
            QuickAddIntent::Close => Some(QuickAddEvent::Close),
            QuickAddIntent::CloseWindow => Some(QuickAddEvent::QuitConfirmed),
            QuickAddIntent::ApplyTemplate(id) => {
                if let Some(template) = vm.templates.iter().find(|t| t.id == id) {
                    self.apply_template(template, &vm.all_people, &vm.all_categories, vm.today);
                }
                None
            }
            QuickAddIntent::ReplaceWithParsed(parsed, kind) => {
                self.apply_parsed(&parsed, kind);
                None
            }
        }
    }

    /// Keep editing: drop the pending request and the prompts, and ask to return to the field
    /// that was being edited.
    pub fn keep_editing(&mut self, ctx: &egui::Context) {
        self.pending_intent = None;
        self.duplicate = None;
        self.template_prompt = None;
        match self.last_focus {
            Some(id) => ctx.memory_mut(|m| m.request_focus(id)),
            None => self.focus_amount = true,
        }
    }

    /// Drop a duplicate decision once the form no longer matches the draft it was given for.
    pub fn revise_consent(&mut self) {
        let stale = self
            .duplicate
            .as_ref()
            .is_some_and(|c| self.to_draft().ok().as_ref() != Some(&c.draft));
        if stale {
            self.duplicate = None;
        }
    }
}

/// What the modal wants the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum QuickAddEvent {
    Close,
    Submit(TxDraft),
    /// Save despite the possible duplicate the user was shown.
    ConfirmDuplicate,
    /// Save the form as a new template under the name the user typed.
    SaveAsTemplate(TemplateDraft),
    /// Save edits to a stored template (Settings).
    SubmitTemplate(TemplateDraft),
    /// Discard was confirmed for a native window close.
    QuitConfirmed,
}

fn field_errors(state: &QuickAddState, field: FormField) -> Vec<&QuickAddError> {
    state.errors.iter().filter(|e| e.field() == field).collect()
}

fn show_errors(
    ui: &mut egui::Ui,
    p: &Palette,
    state: &QuickAddState,
    vm: &ViewModel,
    field: FormField,
) {
    let language = vm.language();
    for error in field_errors(state, field) {
        let name_of = |id: i64| vm.person(id).map_or_else(String::new, |p| p.name.clone());
        ui.label(RichText::new(error_message(error, language, name_of)).color(p.negative));
    }
}

fn person_name(vm: &ViewModel, id: Option<i64>, none: &str) -> String {
    id.and_then(|id| vm.person(id))
        .map_or_else(|| none.to_string(), |p| p.name.clone())
}

fn show_settlement_people(
    ui: &mut egui::Ui,
    p: &Palette,
    state: &mut QuickAddState,
    vm: &ViewModel,
) -> bool {
    let language = vm.language();
    let s = strings(language);
    let people = vm.active_people();
    let mut edited = false;

    ui.horizontal_wrapped(|ui| {
        ui.vertical(|ui| {
            let from_caption =
                ui.label(RichText::new(s.qa_from).text_style(egui::TextStyle::Small));
            let from_name = person_name(vm, state.payer_id, s.no_one);
            let mut from = state.payer_id;
            let from_box = ComboBox::from_id_salt("quick-add-from")
                .selected_text(from_name)
                .width(200.0)
                .show_ui(ui, |ui| {
                    for person in &people {
                        ui.selectable_value(&mut from, Some(person.id), &person.name);
                    }
                });
            from_box.response.labelled_by(from_caption.id);
            if from != state.payer_id {
                state.set_payer(from);
                edited = true;
            }
        });

        ui.vertical(|ui| {
            let to_caption = ui.label(RichText::new(s.qa_to).text_style(egui::TextStyle::Small));
            let current_to = match state.split {
                SplitChoice::Person(id) => Some(id),
                _ => None,
            };
            let to_name = person_name(vm, current_to, s.no_one);
            let mut to = current_to;
            let to_box = ComboBox::from_id_salt("quick-add-to")
                .selected_text(to_name)
                .width(200.0)
                .show_ui(ui, |ui| {
                    for person in &people {
                        ui.selectable_value(&mut to, Some(person.id), &person.name);
                    }
                });
            to_box.response.labelled_by(to_caption.id);
            if to != current_to {
                state.set_recipient(to);
                edited = true;
            }
        });
    });
    show_errors(ui, p, state, vm, FormField::People);
    ui.add_space(8.0);
    edited
}

fn fresh_enter(ctx: &egui::Context) -> bool {
    ctx.input(|i| {
        i.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    repeat: false,
                    ..
                }
            )
        })
    })
}

fn line_error_text(error: QuickEntryError, language: Language) -> &'static str {
    let s = strings(language);
    match error {
        QuickEntryError::Empty => s.qa_line_err_empty,
        QuickEntryError::Amount(_) => s.qa_line_err_amount,
        QuickEntryError::ZeroAmount => s.qa_line_err_zero,
        QuickEntryError::AmountTooLarge => s.qa_line_err_too_large,
        QuickEntryError::MissingTitle => s.qa_line_err_title,
        QuickEntryError::InvalidDate => s.qa_line_err_date,
    }
}

/// What the form asked for this frame, collected while drawing and acted on afterwards.
#[derive(Default)]
#[allow(clippy::struct_excessive_bools)]
struct Asks {
    review: bool,
    close: bool,
    save: bool,
    save_as_template: bool,
    apply_template: Option<i64>,
    line_kind: Option<TransactionType>,
}

/// The Templates picker and the one-line entry field, above Amount.
fn show_create_tools(
    ui: &mut egui::Ui,
    p: &Palette,
    state: &mut QuickAddState,
    vm: &ViewModel,
    asks: &mut Asks,
) {
    let language = vm.language();
    let s = strings(language);
    let caption = ui.label(RichText::new(s.qa_templates_label).text_style(egui::TextStyle::Small));
    if vm.templates.is_empty() {
        ui.label(RichText::new(s.qa_templates_empty).color(p.muted));
    } else {
        let current = state
            .template_id
            .and_then(|id| vm.templates.iter().find(|t| t.id == id))
            .map_or(s.qa_templates_choose, |t| t.draft.name.as_str());
        let picker = ComboBox::from_id_salt("quick-add-templates")
            .selected_text(current)
            .width(260.0)
            .show_ui(ui, |ui| {
                for template in &vm.templates {
                    let needs_repair = !template
                        .draft
                        .issues(&vm.all_people, &vm.all_categories)
                        .is_empty();
                    let label = if needs_repair {
                        format!("{} ({})", template.draft.name, s.qa_template_needs_repair)
                    } else {
                        template.draft.name.clone()
                    };
                    if ui
                        .selectable_label(state.template_id == Some(template.id), label)
                        .clicked()
                    {
                        asks.apply_template = Some(template.id);
                    }
                }
            });
        picker.response.labelled_by(caption.id);
    }
    ui.add_space(8.0);

    let line_caption = ui.label(RichText::new(s.qa_line_label).text_style(egui::TextStyle::Small));
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 110.0).max(120.0);
        let line = ui.add(
            egui::TextEdit::singleline(&mut state.line_input)
                .id(Id::unique(LINE_ID))
                .hint_text(s.qa_line_hint)
                .desired_width(width)
                .margin(egui::Vec2::new(12.0, 10.0)),
        );
        let line = line.labelled_by(line_caption.id);
        if std::mem::take(&mut state.focus_line) {
            line.request_focus();
        }
        if ui
            .add_sized([96.0, 36.0], Button::new(s.qa_line_review))
            .clicked()
        {
            asks.review = true;
        }
    });
    ui.label(
        RichText::new(s.qa_line_help)
            .text_style(egui::TextStyle::Small)
            .color(p.muted),
    );
    if let Some(error) = state.line_error {
        ui.label(RichText::new(line_error_text(error, language)).color(p.negative));
    }
    if state.kind_choice.is_some() {
        ui.horizontal_wrapped(|ui| {
            ui.label(s.qa_kind_choice);
            for kind in [TransactionType::Expense, TransactionType::Refund] {
                if ui.button(kind_label(kind, language)).clicked() {
                    asks.line_kind = Some(kind);
                }
            }
        });
    }
    ui.add_space(8.0);
}

/// Draw the modal and its child prompts. Returns what the user asked for this frame, if anything.
#[allow(clippy::too_many_lines)]
pub fn show(
    ctx: &egui::Context,
    state: &mut QuickAddState,
    vm: &ViewModel,
) -> Option<QuickAddEvent> {
    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    let mut event = None;
    let mut asks = Asks::default();
    // Input is read once, before any widget: a fresh (non-repeat) Enter, and where focus was. A
    // prompt that opened last frame owns the keyboard, and a repeat event never saves.
    let enter_fresh = fresh_enter(ctx);
    let prompt_ready = state.has_child_prompt();
    let line_focused = ctx.memory(|m| m.has_focus(Id::unique(LINE_ID)));
    state.revise_consent();

    let modal = egui::Modal::new(Id::unique("quick-add")).show(ctx, |ui| {
        ui.set_width((ctx.content_rect().width() - 48.0).clamp(280.0, 520.0));
        let heading = match state.mode {
            QuickAddMode::Edit(_) => s.qa_edit_title,
            QuickAddMode::Template(_) => s.qa_edit_template_title,
            QuickAddMode::Create => s.new_transaction,
        };
        ui.label(
            RichText::new(heading)
                .text_style(egui::TextStyle::Heading)
                .strong(),
        );
        ui.add_space(12.0);
        // The form can be taller than a small window (a shared split adds a row per person),
        // so the fields scroll while the title and the Save/Cancel row stay in view.
        let max_height = (ctx.content_rect().height() - 220.0).clamp(160.0, 720.0);
        egui::ScrollArea::vertical()
            .max_height(max_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if state.create_tools_visible() {
                    show_create_tools(ui, &p, state, vm, &mut asks);
                }
                if matches!(state.mode, QuickAddMode::Template(_)) {
                    let caption = ui.label(
                        RichText::new(s.qa_template_name).text_style(egui::TextStyle::Small),
                    );
                    let name = ui.add(
                        egui::TextEdit::singleline(&mut state.template_name)
                            .hint_text(s.qa_template_name_hint)
                            .desired_width(f32::INFINITY)
                            .margin(egui::Vec2::new(12.0, 10.0)),
                    );
                    name.labelled_by(caption.id);
                    show_errors(ui, &p, state, vm, FormField::Name);
                    ui.add_space(4.0);
                }
                // Amount first, focused on open.
                let amount = money_input::show(
                    ui,
                    vm.theme(),
                    language,
                    s.qa_amount,
                    &mut state.amount_input,
                    state.focus_amount,
                );
                state.focus_amount = false;
                let mut edited = amount.response.changed();
                if edited && state.split == SplitChoice::Shared {
                    state.apply_amount_split();
                }
                show_errors(ui, &p, state, vm, FormField::Amount);

                // Title.
                let caption =
                    ui.label(RichText::new(s.qa_title_label).text_style(egui::TextStyle::Small));
                let title = ui.add(
                    egui::TextEdit::singleline(&mut state.title)
                        .hint_text(s.qa_title_hint)
                        .desired_width(f32::INFINITY)
                        .margin(egui::Vec2::new(12.0, 10.0)),
                );
                let title = title.labelled_by(caption.id);
                if std::mem::take(&mut state.focus_title) {
                    title.request_focus();
                }
                edited |= title.changed();
                show_errors(ui, &p, state, vm, FormField::Title);
                ui.add_space(4.0);

                // Date and type on one row. A template has no date.
                ui.horizontal(|ui| {
                    if !matches!(state.mode, QuickAddMode::Template(_)) {
                        ui.vertical(|ui| {
                            let caption = ui
                                .label(RichText::new(s.qa_date).text_style(egui::TextStyle::Small));
                            let date = ui.add(
                                egui::TextEdit::singleline(&mut state.date_text)
                                    .desired_width(130.0)
                                    .font(crate::ui::theme::typography::money_font())
                                    .margin(egui::Vec2::new(12.0, 10.0)),
                            );
                            let date = date.labelled_by(caption.id);
                            if std::mem::take(&mut state.focus_date) {
                                date.request_focus();
                            }
                            edited |= date.changed();
                        });
                    }
                    ui.vertical(|ui| {
                        ui.label(RichText::new(s.qa_type).text_style(egui::TextStyle::Small));
                        ui.horizontal_wrapped(|ui| {
                            for kind in [
                                TransactionType::Expense,
                                TransactionType::Income,
                                TransactionType::Refund,
                                TransactionType::Settlement,
                            ] {
                                if kind == TransactionType::Settlement
                                    && matches!(state.mode, QuickAddMode::Template(_))
                                {
                                    continue;
                                }
                                let selected = state.kind == kind;
                                if ui
                                    .selectable_label(selected, kind_label(kind, language))
                                    .clicked()
                                    && !selected
                                {
                                    let allowed: Vec<i64> =
                                        vm.active_categories(kind).iter().map(|c| c.id).collect();
                                    state.set_kind(kind, &allowed);
                                    edited = true;
                                }
                            }
                        });
                    });
                });
                show_errors(ui, &p, state, vm, FormField::Date);
                if state.kind == TransactionType::Settlement
                    && let Some(viewed) = state.settlement_month_mismatch
                {
                    let text = s
                        .qa_settlement_month_mismatch
                        .replace("{viewed}", &month_label(viewed, language))
                        .replace("{today}", &format_date(vm.today, language));
                    ui.add(egui::Label::new(RichText::new(text).color(p.warning)).wrap());
                }
                ui.add_space(4.0);

                // Category, filtered by type (hidden for settlements). An archived category already
                // on the entry stays selectable, and a broken template category is flagged.
                if state.kind != TransactionType::Settlement {
                    let category_caption =
                        ui.label(RichText::new(s.qa_category).text_style(egui::TextStyle::Small));
                    let selected_name = state
                        .category_id
                        .and_then(|id| vm.category(id))
                        .map_or(s.qa_no_category, |c| c.name.as_str());
                    let category_box = ComboBox::from_id_salt("quick-add-category")
                        .selected_text(selected_name)
                        .width(240.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut state.category_id, None, s.qa_no_category);
                            let mut shown: Vec<_> = vm.active_categories(state.kind);
                            if let Some(current) = state
                                .category_id
                                .and_then(|id| vm.category(id))
                                .filter(|current| !shown.iter().any(|c| c.id == current.id))
                            {
                                shown.push(current);
                            }
                            for category in shown {
                                ui.selectable_value(
                                    &mut state.category_id,
                                    Some(category.id),
                                    &category.name,
                                );
                            }
                        });
                    category_box.response.labelled_by(category_caption.id);
                    show_errors(ui, &p, state, vm, FormField::Category);
                    ui.add_space(8.0);
                }

                if state.kind == TransactionType::Settlement {
                    edited |= show_settlement_people(ui, &p, state, vm);
                } else if state.kind != TransactionType::Income {
                    edited |= show_responsibility(ui, &p, state, vm);
                }

                // Optional details.
                let more = egui::CollapsingHeader::new(s.qa_more)
                    .default_open(state.show_more)
                    .show(ui, |ui| {
                        let caption = ui.label(
                            RichText::new(s.qa_description).text_style(egui::TextStyle::Small),
                        );
                        let description = ui.add(
                            egui::TextEdit::multiline(&mut state.description)
                                .desired_rows(2)
                                .desired_width(f32::INFINITY),
                        );
                        let description = description.labelled_by(caption.id);
                        edited |= description.changed();
                    });
                state.show_more = more.fully_open();

                // Who is responsible and who pays, visible before saving.
                show_summary(ui, &p, state, vm);
                show_errors(ui, &p, state, vm, FormField::General);

                if edited {
                    state.errors.clear();
                }
            });

        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let primary =
                Button::new(RichText::new(s.save).color(p.on_accent).strong()).fill(p.accent);
            asks.save = ui.add_sized([120.0, 36.0], primary).clicked();
            if ui.add_sized([96.0, 36.0], Button::new(s.cancel)).clicked() {
                asks.close = true;
            }
            if state.create_tools_visible()
                && state.kind != TransactionType::Settlement
                && ui.button(s.qa_save_as_template).clicked()
            {
                asks.save_as_template = true;
            }
        });
        // Enter: while the one-line field has focus it belongs to Review alone; otherwise a fresh
        // press saves. Repeats are swallowed, and a prompt that was open owns the key.
        // While a prompt is open the key stays in the input for it to read.
        let enter = !prompt_ready && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        if enter && enter_fresh {
            if line_focused && state.create_tools_visible() {
                asks.review = true;
            } else {
                asks.save = true;
            }
        }
    });

    // Remember where the keyboard was, for returning from a prompt.
    if !prompt_ready && let Some(id) = ctx.memory(egui::Memory::focused) {
        state.last_focus = Some(id);
    }
    if prompt_ready {
        asks.save = false;
        asks.review = false;
        asks.save_as_template = false;
        asks.close = false;
        asks.apply_template = None;
        asks.line_kind = None;
    }
    if asks.close {
        event = state.request(QuickAddIntent::Close, vm);
    }
    let mut parsed = None;
    if asks.review {
        parsed = state.review_line(vm.today);
    }
    if let Some(kind) = asks.line_kind {
        parsed = state.choose_line_kind(kind);
    }
    if event.is_none()
        && let Some((line, kind)) = parsed
    {
        event = state.request(QuickAddIntent::ReplaceWithParsed(line, kind), vm);
    }
    if event.is_none()
        && let Some(id) = asks.apply_template
    {
        event = state.request(QuickAddIntent::ApplyTemplate(id), vm);
    }
    if asks.save_as_template {
        match state.to_template_draft("x") {
            Ok(draft) => {
                state.template_prompt = Some(TemplatePrompt {
                    draft,
                    name: String::new(),
                    error: None,
                });
                state.focus_prompt = true;
            }
            Err(errors) => {
                state.errors = errors
                    .into_iter()
                    .filter(|e| !matches!(e, QuickAddError::Template(TemplateError::NameEmpty)))
                    .collect();
                state.focus_first_error();
            }
        }
    }
    if asks.save && event.is_none() && state.pending_intent.is_none() {
        event = save_event(state);
    }
    if event.is_none() && !prompt_ready && modal.should_close() {
        event = state.request(QuickAddIntent::Close, vm);
    }

    // Child prompts draw one frame after they open, so the key that opened one cannot also act
    // on it.
    if prompt_ready {
        event = event.or_else(|| show_prompts(ctx, state, vm));
    } else if state.has_child_prompt() {
        ctx.request_repaint();
    }
    event
}

/// Validate the form and build the event for Save (or the template Save in Settings).
fn save_event(state: &mut QuickAddState) -> Option<QuickAddEvent> {
    if let QuickAddMode::Template(_) = state.mode {
        let name = state.template_name.clone();
        return match state.to_template_draft(&name) {
            Ok(draft) => Some(QuickAddEvent::SubmitTemplate(draft)),
            Err(errors) => {
                state.errors = errors;
                state.focus_first_error();
                None
            }
        };
    }
    match state.to_draft() {
        Ok(draft) => Some(QuickAddEvent::Submit(draft)),
        Err(errors) => {
            state.errors = errors;
            state.focus_first_error();
            None
        }
    }
}

fn show_prompts(
    ctx: &egui::Context,
    state: &mut QuickAddState,
    vm: &ViewModel,
) -> Option<QuickAddEvent> {
    let focus = std::mem::take(&mut state.focus_prompt);
    if state.pending_intent.is_some() {
        match prompts::discard(ctx, vm, focus) {
            Some(PromptChoice::Confirm) => {
                let intent = state.pending_intent.take()?;
                return state.perform_intent(intent, vm);
            }
            Some(PromptChoice::KeepEditing) => state.keep_editing(ctx),
            None => state.focus_prompt = focus,
        }
    } else if let Some(consent) = state.duplicate.clone() {
        match prompts::duplicate(ctx, vm, &consent, focus) {
            Some(PromptChoice::Confirm) => return Some(QuickAddEvent::ConfirmDuplicate),
            Some(PromptChoice::KeepEditing) => state.keep_editing(ctx),
            None => state.focus_prompt = focus,
        }
    } else if let Some(prompt) = state.template_prompt.as_mut() {
        match prompts::template_name(ctx, vm, prompt, focus) {
            Some(NameChoice::Save) => {
                let name = prompt.name.trim().to_string();
                if name.is_empty() {
                    prompt.error = Some(PromptError::Template(TemplateError::NameEmpty));
                } else {
                    let mut draft = prompt.draft.clone();
                    draft.name = name;
                    return Some(QuickAddEvent::SaveAsTemplate(draft));
                }
            }
            Some(NameChoice::Cancel) => state.keep_editing(ctx),
            None => state.focus_prompt = focus,
        }
    }
    None
}

/// Responsible person, shared split rows, payer and their repair messages.
fn show_responsibility(
    ui: &mut egui::Ui,
    p: &Palette,
    state: &mut QuickAddState,
    vm: &ViewModel,
) -> bool {
    let language = vm.language();
    let s = strings(language);
    let mut edited = false;
    let people = vm.active_people();
    // Responsible person: every active person, Shared (two or more people) and No one.
    ui.label(RichText::new(s.qa_for).text_style(egui::TextStyle::Small));
    ui.horizontal_wrapped(|ui| {
        for person in &people {
            let choice = SplitChoice::Person(person.id);
            if ui
                .selectable_label(state.split == choice, &person.name)
                .clicked()
            {
                state.set_split(choice);
            }
        }
        if people.len() >= 2
            && ui
                .selectable_label(state.split == SplitChoice::Shared, s.shared)
                .clicked()
        {
            state.set_split(SplitChoice::Shared);
        }
        if ui
            .selectable_label(state.split == SplitChoice::NoOne, s.no_one)
            .clicked()
        {
            state.set_split(SplitChoice::NoOne);
        }
    });
    if state.split == SplitChoice::Shared {
        ui.add_space(4.0);
        ui.label(RichText::new(s.qa_split_between).color(p.muted));
        let rows: Vec<(i64, String)> = state.shared_amounts.clone();
        for (person_id, mut text) in rows {
            let archived = vm.person(person_id).is_some_and(|person| person.archived);
            let name = person_name(vm, Some(person_id), "");
            let label = if archived {
                format!("{name} ({})", s.archived_tag)
            } else {
                name
            };
            let field = money_input::show(ui, vm.theme(), language, &label, &mut text, false);
            if field.response.changed() {
                state.edit_share(person_id, text);
                edited = true;
            }
            if archived
                && !state
                    .shared_amounts
                    .iter()
                    .any(|(id, t)| *id == person_id && t.trim().is_empty())
                && ui.button(s.qa_remove_from_split).clicked()
            {
                state.edit_share(person_id, String::new());
                edited = true;
            }
        }
        if state.shared_dirty && ui.add(Button::new(s.qa_even_split).frame(false)).clicked() {
            state.reset_even();
        }
    }
    show_errors(ui, p, state, vm, FormField::People);
    ui.add_space(8.0);

    // Payer.
    let payer_caption = ui.label(RichText::new(s.qa_paid_by).text_style(egui::TextStyle::Small));
    let payer_name = person_name(vm, state.payer_id, s.no_one);
    let mut payer = state.payer_id;
    let payer_box = ComboBox::from_id_salt("quick-add-payer")
        .selected_text(payer_name)
        .width(240.0)
        .show_ui(ui, |ui| {
            for person in &people {
                ui.selectable_value(&mut payer, Some(person.id), &person.name);
            }
            ui.selectable_value(&mut payer, None, s.no_one);
        });
    payer_box.response.labelled_by(payer_caption.id);
    if payer != state.payer_id {
        state.set_payer(payer);
    }
    show_errors(ui, p, state, vm, FormField::Payer);
    ui.add_space(8.0);
    edited
}

fn show_summary(ui: &mut egui::Ui, p: &Palette, state: &QuickAddState, vm: &ViewModel) {
    let s = strings(vm.language());
    if state.kind == TransactionType::Settlement {
        let from = person_name(vm, state.payer_id, s.no_one);
        let to_id = match state.split {
            SplitChoice::Person(id) => Some(id),
            _ => None,
        };
        let to = person_name(vm, to_id, s.no_one);
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                s.qa_settlement_summary
                    .replace("{from}", &from)
                    .replace("{to}", &to),
            )
            .color(p.muted),
        );
    } else if state.kind != TransactionType::Income {
        let person = match state.split {
            SplitChoice::Person(id) => person_name(vm, Some(id), s.no_one),
            SplitChoice::Shared => s.shared.to_string(),
            SplitChoice::NoOne => s.no_one.to_string(),
        };
        let payer = person_name(vm, state.payer_id, s.no_one);
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                s.qa_summary
                    .replace("{person}", &person)
                    .replace("{payer}", &payer),
            )
            .color(p.muted),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::test_fixtures::{three_people, today, two_people};

    fn id(p: &[Person], i: usize) -> i64 {
        p[i].id
    }

    #[test]
    fn defaults_to_first_person_full_allocation_and_payer() {
        let people = two_people();
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.title = "Market".into();
        s.amount_input = "45,90".into();
        let d = s.to_draft().unwrap();
        assert_eq!(
            d.allocations,
            vec![Allocation {
                person_id: id(&people, 0),
                amount: Money::from_cents(4590)
            }]
        );
        assert_eq!(d.payer_id, Some(id(&people, 0)));
        assert_eq!(d.split_mode, None);
        assert_eq!(d.date, today());
    }

    #[test]
    fn shared_split_can_be_customized_and_must_match_total() {
        let people = two_people();
        let (a, b) = (id(&people, 0), id(&people, 1));
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.title = "Dinner".into();
        s.amount_input = "200,00".into();
        s.set_split(SplitChoice::Shared);
        assert_eq!(
            s.shared_amounts,
            vec![(a, "100,00".into()), (b, "100,00".into())]
        );
        assert_eq!(s.to_draft().unwrap().split_mode, Some(SplitMode::Even));

        s.edit_share(a, "120,00".into());
        s.edit_share(b, "80,00".into());
        let d = s.to_draft().unwrap();
        assert_eq!(
            d.allocations.iter().map(|x| x.amount.cents()).sum::<i64>(),
            20_000
        );
        assert_eq!(d.split_mode, Some(SplitMode::Custom));

        s.edit_share(b, "70,00".into());
        let errors = s.to_draft().unwrap_err();
        assert!(errors.iter().any(|e| matches!(
            e,
            QuickAddError::Validation(ValidationError::ShareSumMismatch { .. })
        )));
    }

    #[test]
    fn hand_edited_shares_survive_a_changed_total_until_reset() {
        let people = two_people();
        let a = id(&people, 0);
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.amount_input = "100,00".into();
        s.set_split(SplitChoice::Shared);
        s.edit_share(a, "70,00".into());
        s.amount_input = "200,00".into();
        s.apply_amount_split();
        assert_eq!(s.shared_amounts[0].1, "70,00");
        s.reset_even();
        assert_eq!(s.shared_amounts[0].1, "100,00");
        assert!(!s.shared_dirty);
    }

    #[test]
    fn shared_split_handles_three_people() {
        let people = three_people();
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.title = "Trip".into();
        s.amount_input = "100,00".into();
        s.set_split(SplitChoice::Shared);
        let d = s.to_draft().unwrap();
        assert_eq!(
            d.allocations.iter().map(|x| x.amount.cents()).sum::<i64>(),
            10_000
        );
        assert_eq!(d.allocations.len(), 3);
        assert_eq!(
            d.allocations[0].amount,
            Money::from_cents(3334),
            "the extra cent goes to the owner"
        );
        assert_eq!(d.split_mode, Some(SplitMode::Even));
    }

    #[test]
    fn an_explicit_zero_share_is_valid() {
        let people = two_people();
        let (a, b) = (id(&people, 0), id(&people, 1));
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.title = "Gift".into();
        s.amount_input = "50,00".into();
        s.set_split(SplitChoice::Shared);
        s.edit_share(a, "50,00".into());
        s.edit_share(b, "0,00".into());
        assert!(s.to_draft().is_ok());
    }

    #[test]
    fn payer_defaults_to_owner_when_switching_to_shared() {
        let people = two_people();
        let (owner, partner) = (id(&people, 0), id(&people, 1));
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.set_split(SplitChoice::Person(partner));
        assert_eq!(
            s.payer_id,
            Some(partner),
            "the payer follows the responsible person"
        );
        s.set_split(SplitChoice::Shared);
        assert_eq!(s.payer_id, Some(owner));
        s.set_split(SplitChoice::NoOne);
        assert_eq!(s.payer_id, Some(owner));
    }

    #[test]
    fn a_payer_chosen_by_hand_is_left_alone() {
        let people = two_people();
        let (owner, partner) = (id(&people, 0), id(&people, 1));
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.set_payer(Some(partner));
        s.set_split(SplitChoice::Shared);
        assert_eq!(s.payer_id, Some(partner));
        s.set_split(SplitChoice::Person(owner));
        assert_eq!(s.payer_id, Some(partner));
        s.set_payer(None);
        s.title = "x".into();
        s.amount_input = "1".into();
        assert_eq!(s.to_draft().unwrap().payer_id, None);
    }

    #[test]
    fn changing_kind_clears_an_incompatible_category() {
        let people = two_people();
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.category_id = Some(5);
        s.set_kind(TransactionType::Income, &[10, 11]);
        assert_eq!((s.kind, s.category_id), (TransactionType::Income, None));
        s.category_id = Some(10);
        s.set_kind(TransactionType::Refund, &[10, 12]);
        assert_eq!(
            s.category_id,
            Some(10),
            "a category valid for the new kind is kept"
        );
    }

    #[test]
    fn income_has_no_people_or_payer() {
        let people = two_people();
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.title = "Salary".into();
        s.amount_input = "3000".into();
        s.kind = TransactionType::Income;
        let d = s.to_draft().unwrap();
        assert!(d.allocations.is_empty());
        assert_eq!((d.payer_id, d.split_mode), (None, None));
    }

    #[test]
    fn problems_are_collected_by_field() {
        let people = two_people();
        let mut s = QuickAddState::new(&people, today(), Language::PtBr);
        s.amount_input = "0,005".into();
        s.date_text = "31/02/2026".into();
        let errors = s.to_draft().unwrap_err();
        assert!(errors.contains(&QuickAddError::Amount(MoneyError::TooPrecise)));
        assert!(errors.contains(&QuickAddError::Date));
        let fields: Vec<FormField> = errors.iter().map(QuickAddError::field).collect();
        assert!(fields.contains(&FormField::Amount) && fields.contains(&FormField::Date));

        s.amount_input = "10".into();
        s.date_text = "05/11/2026".into();
        let errors = s.to_draft().unwrap_err();
        assert_eq!(
            errors,
            vec![QuickAddError::Validation(ValidationError::TitleEmpty)]
        );
        assert_eq!(errors[0].field(), FormField::Title);
    }

    #[test]
    fn a_bad_share_is_reported_for_that_person() {
        let people = two_people();
        let b = id(&people, 1);
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.title = "x".into();
        s.amount_input = "10".into();
        s.set_split(SplitChoice::Shared);
        s.edit_share(b, "abc".into());
        let errors = s.to_draft().unwrap_err();
        assert_eq!(errors, vec![QuickAddError::Share(b, MoneyError::Invalid)]);
        let text = error_message(&errors[0], Language::En, |_| "Ana".into());
        assert_eq!(text, "Ana: That is not a valid amount.");
    }

    #[test]
    fn error_messages_are_localized() {
        for error in [
            QuickAddError::Amount(MoneyError::Empty),
            QuickAddError::Date,
            QuickAddError::Save,
            QuickAddError::Validation(ValidationError::TitleEmpty),
        ] {
            let en = error_message(&error, Language::En, |_| String::new());
            let pt = error_message(&error, Language::PtBr, |_| String::new());
            assert_ne!(en, pt);
            assert!(!en.contains('{') && !pt.contains('{'));
        }
    }

    #[test]
    fn editing_round_trips_a_stored_transaction() {
        let people = two_people();
        let (a, b) = (id(&people, 0), id(&people, 1));
        let tx = Transaction {
            id: 7,
            title: "Dinner".into(),
            amount: Money::from_cents(20_000),
            date: today(),
            kind: TransactionType::Expense,
            category_id: Some(3),
            payer_id: Some(b),
            description: Some("with friends".into()),
            split_mode: Some(SplitMode::Custom),
            created_at: "2026-11-11T00:00:00Z".into(),
            updated_at: "2026-11-11T00:00:00Z".into(),
            allocations: vec![
                Allocation {
                    person_id: a,
                    amount: Money::from_cents(12_000),
                },
                Allocation {
                    person_id: b,
                    amount: Money::from_cents(8_000),
                },
            ],
        };
        let s = QuickAddState::from_transaction(&tx, &people, Language::En);
        assert_eq!(s.mode, QuickAddMode::Edit(7));
        assert!(s.show_more && s.shared_dirty && !s.focus_amount);
        assert_eq!(s.to_draft().unwrap(), tx.to_draft());
    }

    #[test]
    fn editing_keeps_an_archived_person_on_the_split() {
        let people = two_people();
        let gone = 99;
        let tx = Transaction {
            id: 1,
            title: "Old".into(),
            amount: Money::from_cents(100),
            date: today(),
            kind: TransactionType::Expense,
            category_id: None,
            payer_id: Some(gone),
            description: None,
            split_mode: Some(SplitMode::Custom),
            created_at: "2026-11-11T00:00:00Z".into(),
            updated_at: "2026-11-11T00:00:00Z".into(),
            allocations: vec![
                Allocation {
                    person_id: id(&people, 0),
                    amount: Money::from_cents(60),
                },
                Allocation {
                    person_id: gone,
                    amount: Money::from_cents(40),
                },
            ],
        };
        let s = QuickAddState::from_transaction(&tx, &people, Language::En);
        let draft = s.to_draft().unwrap();
        assert_eq!(draft.allocations.len(), 2);
        assert_eq!(draft.payer_id, Some(gone));
    }

    #[test]
    fn reset_for_next_keeps_the_context_and_refocuses_the_amount() {
        let people = two_people();
        let partner = id(&people, 1);
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.title = "Market".into();
        s.amount_input = "45,90".into();
        s.description = "weekly".into();
        s.category_id = Some(2);
        s.show_more = true;
        s.set_split(SplitChoice::Person(partner));
        s.kind = TransactionType::Refund;
        s.errors.push(QuickAddError::Save);
        s.focus_amount = false;
        s.reset_for_next();
        assert!(s.title.is_empty() && s.amount_input.is_empty() && s.description.is_empty());
        assert_eq!(
            (s.category_id, s.show_more, s.errors.len()),
            (None, false, 0)
        );
        assert_eq!(
            (s.split, s.kind, s.payer_id),
            (
                SplitChoice::Person(partner),
                TransactionType::Refund,
                Some(partner)
            )
        );
        assert_eq!(s.date_text, "2026-11-11");
        assert!(s.focus_amount);
    }

    #[test]
    fn no_people_means_an_unassigned_entry() {
        let mut s = QuickAddState::new(&[], today(), Language::En);
        s.title = "x".into();
        s.amount_input = "5".into();
        let d = s.to_draft().unwrap();
        assert!(d.allocations.is_empty());
        assert_eq!(d.payer_id, None);
    }

    /// The harness has no theme of its own; install ours the way the real app does.
    fn themed(ui: &mut egui::Ui, vm: &ViewModel) {
        // The harness only has egui's bundled fonts, which lack some of our symbols.
        ui.ctx()
            .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
        crate::ui::theme::apply(ui.ctx(), vm.theme());
        let style = ui.ctx().global_style();
        ui.set_style(style);
    }

    fn onboarded_vm() -> ViewModel {
        use crate::application::services::onboarding::OnboardingResult;
        let db = crate::infrastructure::db::Db::open_in_memory().unwrap();
        let mut core = crate::app::AppCore::empty_for_test(db);
        core.complete_onboarding(OnboardingResult {
            display_name: "A3".into(),
            partner_name: Some("Ana".into()),
            start_balance: Money::ZERO,
            language: Language::En,
        })
        .unwrap();
        core.vm.clone()
    }

    type ModalApp = (QuickAddState, ViewModel, Option<QuickAddEvent>);

    fn modal_harness(vm: ViewModel) -> egui_kittest::Harness<'static, ModalApp> {
        let people: Vec<Person> = vm.active_people().into_iter().cloned().collect();
        let state = QuickAddState::new(&people, today(), Language::En);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(800.0, 1000.0))
            .build_ui_state(
                |ui, (state, vm, event): &mut ModalApp| {
                    themed(ui, vm);
                    if let Some(e) = show(ui.ctx(), state, vm) {
                        *event = Some(e);
                    }
                },
                (state, vm, None),
            );
        // The modal fades in and the scroll area settles over the first frames.
        harness.run_steps(12);
        harness
    }

    /// Drives the real modal: Enter on an empty form reports problems, typing fills the fields,
    /// and Enter on a valid form submits.
    #[test]
    fn the_modal_takes_typed_input_and_submits_on_enter() {
        use egui_kittest::kittest::Queryable;
        let mut harness = modal_harness(onboarded_vm());
        harness.run_steps(3);

        harness.key_press(Key::Enter);
        harness.run_steps(3);
        assert!(harness.state().2.is_none());
        assert!(!harness.state().0.errors.is_empty());

        harness
            .get_all_by_role(egui::Role::TextInput)
            .nth(1)
            .unwrap()
            .focus();
        harness.run_steps(2);
        harness
            .get_all_by_role(egui::Role::TextInput)
            .nth(1)
            .unwrap()
            .type_text("45,90");
        harness.run_steps(3);
        assert_eq!(harness.state().0.amount_input, "45,90");
        assert!(
            harness.state().0.errors.is_empty(),
            "editing clears the old errors"
        );
        assert!(
            harness.query_by_label("R$ 45,90").is_some(),
            "a valid amount shows a preview"
        );

        harness
            .get_all_by_role(egui::Role::TextInput)
            .nth(2)
            .unwrap()
            .focus();
        harness.run_steps(2);
        harness
            .get_all_by_role(egui::Role::TextInput)
            .nth(2)
            .unwrap()
            .type_text("Market");
        harness.run_steps(3);
        harness.key_press(Key::Enter);
        harness.run_steps(3);

        let Some(QuickAddEvent::Submit(draft)) = harness.state().2.clone() else {
            panic!("Enter should submit a valid form");
        };
        assert_eq!(draft.title, "Market");
        assert_eq!(draft.amount, Money::from_cents(4_590));
        assert_eq!(
            draft.allocations.len(),
            1,
            "the responsible person defaults to the owner"
        );
    }

    #[test]
    fn choosing_shared_shows_one_amount_per_person_and_a_summary() {
        use egui_kittest::kittest::Queryable;
        let mut harness = modal_harness(onboarded_vm());
        harness.run_steps(3);
        assert!(harness.query_by_label("For A3 · paid by A3").is_some());
        harness.get_by_label("Shared").click();
        harness.run_steps(3);
        assert_eq!(harness.state().0.split, SplitChoice::Shared);
        assert_eq!(harness.state().0.shared_amounts.len(), 2);
        assert!(harness.query_by_label("For Shared · paid by A3").is_some());
    }

    #[test]
    fn the_cancel_button_closes() {
        use egui_kittest::kittest::Queryable;
        let mut harness = modal_harness(onboarded_vm());
        harness.run_steps(3);
        harness.get_by_label("Cancel").click();
        harness.run_steps(3);
        assert_eq!(harness.state().2, Some(QuickAddEvent::Close));
    }

    /// Found by driving the real window: after a refused Enter the field that needs fixing must
    /// have the keyboard, because Enter makes a text field let go of it.
    #[test]
    fn a_refused_save_puts_the_keyboard_on_the_first_problem() {
        use egui_kittest::kittest::Queryable;
        let mut harness = modal_harness(onboarded_vm());
        harness.run_steps(3);
        harness.key_press(Key::Enter);
        harness.run_steps(3);
        assert!(
            harness
                .get_all_by_role(egui::Role::TextInput)
                .nth(1)
                .unwrap()
                .is_focused(),
            "amount first"
        );

        harness
            .get_all_by_role(egui::Role::TextInput)
            .nth(1)
            .unwrap()
            .type_text("10");
        harness.run_steps(3);
        harness.key_press(Key::Enter);
        harness.run_steps(3);
        assert!(
            harness
                .get_all_by_role(egui::Role::TextInput)
                .nth(2)
                .unwrap()
                .is_focused(),
            "then the empty title"
        );
        harness
            .get_all_by_role(egui::Role::TextInput)
            .nth(2)
            .unwrap()
            .type_text("Lunch");
        harness.run_steps(3);
        harness.key_press(Key::Enter);
        harness.run_steps(3);
        assert!(matches!(harness.state().2, Some(QuickAddEvent::Submit(_))));
    }

    #[test]
    fn the_first_error_decides_which_field_gets_focus() {
        let people = two_people();
        let mut s = QuickAddState::new(&people, today(), Language::En);
        s.focus_amount = false;
        s.errors = vec![
            QuickAddError::Validation(ValidationError::TitleEmpty),
            QuickAddError::Date,
        ];
        s.focus_first_error();
        assert!(s.focus_title && !s.focus_amount && !s.focus_date);
        s.focus_title = false;
        s.errors = vec![QuickAddError::Date];
        s.focus_first_error();
        assert!(s.focus_date);
        s.focus_date = false;
        s.errors = vec![QuickAddError::Save];
        s.focus_first_error();
        assert!(!s.focus_amount && !s.focus_title && !s.focus_date);
    }

    /// Found by driving the real window: a shared split made the form taller than the window, so
    /// the heading and the Save and Cancel buttons were pushed off screen.
    #[test]
    fn the_form_scrolls_instead_of_pushing_its_buttons_off_a_small_window() {
        use egui_kittest::kittest::Queryable;
        let vm = onboarded_vm();
        let people: Vec<Person> = vm.active_people().into_iter().cloned().collect();
        let state = QuickAddState::new(&people, today(), Language::En);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(900.0, 640.0))
            .build_ui_state(
                |ui, (state, vm, event): &mut ModalApp| {
                    themed(ui, vm);
                    if let Some(e) = show(ui.ctx(), state, vm) {
                        *event = Some(e);
                    }
                },
                (state, vm, None),
            );
        harness.run_steps(3);
        harness.get_by_label("Shared").click();
        harness.run_steps(4);
        for label in ["Save", "Cancel", "New transaction"] {
            let rect = harness.get_by_label(label).rect();
            assert!(
                rect.top() >= 0.0 && rect.bottom() <= 640.0,
                "{label} stays on screen: {rect:?}"
            );
        }
    }

    #[test]
    fn quick_add_fits_within_640x410_scaled_viewport() {
        use egui_kittest::kittest::Queryable;
        let vm = onboarded_vm();
        let people: Vec<Person> = vm.active_people().into_iter().cloned().collect();
        let state = QuickAddState::new(&people, today(), Language::En);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(640.0, 410.0))
            .with_context_setup(crate::ui::theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, (state, vm, event): &mut ModalApp| {
                    themed(ui, vm);
                    if let Some(e) = show(ui.ctx(), state, vm) {
                        *event = Some(e);
                    }
                },
                (state, vm, None),
            );
        harness.run_steps(3);
        harness.get_by_label("Shared").click();
        harness.run_steps(4);
        for label in ["Save", "Cancel", "New transaction"] {
            let rect = harness.get_by_label(label).rect();
            assert!(
                rect.top() >= 0.0 && rect.bottom() <= 410.0,
                "{label} overflows 410 px height: {rect:?}"
            );
            assert!(
                rect.left() >= 0.0 && rect.right() <= 640.0,
                "{label} overflows 640 px width: {rect:?}"
            );
        }
    }

    #[test]
    fn quick_add_settlement_mode_builds_a_valid_draft() {
        let people = two_people();
        let mut state = QuickAddState::new(&people, today(), Language::En);
        state.amount_input = "80,00".into();
        state.title = "Ana -> A3".into();
        state.category_id = Some(1);
        state.set_kind(TransactionType::Settlement, &[1, 2]);
        assert_eq!(state.category_id, None);

        // Selecting From = Ana (2), To = A3 (1) yields a valid settlement draft.
        state.set_payer(Some(2));
        state.set_recipient(Some(1));
        let draft = state.to_draft().unwrap();
        assert_eq!(draft.kind, TransactionType::Settlement);
        assert_eq!(draft.payer_id, Some(2));
        assert_eq!(draft.category_id, None);
        assert_eq!(draft.split_mode, None);
        assert_eq!(
            draft.allocations,
            vec![Allocation {
                person_id: 1,
                amount: Money::from_cents(8_000),
            }]
        );

        // Same person blocks save with a localized error.
        state.set_recipient(Some(2));
        let errs = state.to_draft().unwrap_err();
        assert!(errs.contains(&QuickAddError::Validation(
            ValidationError::SettlementSamePerson
        )));
        let msg = error_message(&errs[0], Language::En, |_| String::new());
        assert!(!msg.trim().is_empty());
        assert_ne!(
            msg,
            error_message(&errs[0], Language::PtBr, |_| String::new())
        );
    }

    #[test]
    fn quick_add_kind_switch_keeps_shared_fields() {
        let people = two_people();
        let mut state = QuickAddState::new(&people, today(), Language::En);
        state.title = "Payback".into();
        state.amount_input = "50,00".into();
        state.date_text = "2026-11-15".into();
        state.description = "PIX".into();
        state.category_id = Some(3);
        state.set_split(SplitChoice::Shared);
        state.set_kind(TransactionType::Settlement, &[3]);
        assert_eq!(state.title, "Payback");
        assert_eq!(state.amount_input, "50,00");
        assert_eq!(state.date_text, "2026-11-15");
        assert_eq!(state.description, "PIX");
        assert_eq!(state.category_id, None);
        assert!(state.shared_amounts.is_empty());
    }
}
