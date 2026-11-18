//! Quick Add flow regressions at the state level (contract §14.1): template apply and repair,
//! dirty-draft detection, typed pending intents, and one-line entry.

use chrono::NaiveDate;

use super::quick_add::{
    QuickAddError, QuickAddEvent, QuickAddIntent, QuickAddMode, QuickAddState, Repair, SplitChoice,
};
use crate::app::view_model::ViewModel;
use crate::domain::category::{Category, CategoryScope};
use crate::domain::money::Money;
use crate::domain::person::{Person, PersonKind};
use crate::domain::quick_entry::QuickEntryError;
use crate::domain::settings::Language;
use crate::domain::template::{ResponsibilityMode, Template, TemplateDraft};
use crate::domain::transaction::{Allocation, SplitMode, TransactionType, ValidationError};
use crate::shared::test_fixtures::{today, two_people};

fn person(id: i64, name: &str, archived: bool) -> Person {
    Person {
        id,
        name: name.into(),
        kind: if id == 1 {
            PersonKind::Owner
        } else {
            PersonKind::Other
        },
        archived,
        created_at: String::new(),
    }
}

fn category(id: i64, name: &str, scope: CategoryScope, archived: bool) -> Category {
    Category {
        id,
        name: name.into(),
        color: "#FF8A65".into(),
        icon: None,
        applies_to: scope,
        is_default: false,
        archived,
        created_at: String::new(),
    }
}

fn share(person_id: i64, cents: i64) -> Allocation {
    Allocation {
        person_id,
        amount: Money::from_cents(cents),
    }
}

fn template(id: i64, mode: ResponsibilityMode, amount: i64, shares: Vec<Allocation>) -> Template {
    Template {
        id,
        draft: TemplateDraft {
            name: format!("Template {id}"),
            title: "Groceries".into(),
            amount: Money::from_cents(amount),
            kind: TransactionType::Expense,
            category_id: Some(10),
            payer_id: Some(1),
            description: Some("Weekly".into()),
            responsibility: mode,
            allocations: shares,
        },
        created_at: "2026-11-01T00:00:00Z".into(),
        last_used_at: None,
    }
}

/// Four people (the last archived) and two categories (one archived).
fn vm_with(templates: Vec<Template>) -> ViewModel {
    let mut vm = ViewModel::new(Language::En);
    vm.all_people = vec![
        person(1, "A3", false),
        person(2, "Ana", false),
        person(5, "Eve", false),
        person(9, "Zed", true),
    ];
    vm.all_categories = vec![
        category(10, "Food", CategoryScope::Expense, false),
        category(11, "Old", CategoryScope::Expense, true),
        category(12, "Salary", CategoryScope::Income, false),
    ];
    vm.templates = templates;
    vm.today = today();
    vm
}

fn blank(vm: &ViewModel) -> QuickAddState {
    let people: Vec<Person> = vm.active_people().into_iter().cloned().collect();
    QuickAddState::new(&people, vm.today, Language::En)
}

fn apply(state: &mut QuickAddState, vm: &ViewModel, id: i64) {
    let t = vm.templates.iter().find(|t| t.id == id).unwrap().clone();
    state.apply_template(&t, &vm.all_people, &vm.all_categories, vm.today);
}

#[test]
fn apply_template_fills_every_field_with_todays_visible_date() {
    let vm = vm_with(vec![template(
        1,
        ResponsibilityMode::Custom,
        10_000,
        vec![share(1, 7_000), share(2, 3_000)],
    )]);
    let mut state = blank(&vm);
    state.date_text = "01/01/2020".into();
    apply(&mut state, &vm, 1);
    assert_eq!(state.title, "Groceries");
    assert_eq!(state.amount_input, "100,00");
    assert_eq!(
        state.date_text,
        crate::i18n::format_date(today(), Language::En)
    );
    assert_eq!(state.kind, TransactionType::Expense);
    assert_eq!(state.category_id, Some(10));
    assert_eq!(state.payer_id, Some(1));
    assert_eq!(state.description, "Weekly");
    assert!(state.show_more, "a populated description opens More");
    assert_eq!(state.split, SplitChoice::Shared);
    assert_eq!(state.template_id, Some(1));
    let draft = state.to_draft().unwrap();
    assert_eq!(draft.date, today());
    assert_eq!(draft.split_mode, Some(SplitMode::Custom));
    assert_eq!(draft.allocations, vec![share(1, 7_000), share(2, 3_000)]);
    assert!(!state.is_dirty(), "applying rebases the clean state");
}

#[test]
fn an_even_template_keeps_its_exact_subset_and_stable_remainder() {
    let vm = vm_with(vec![template(
        1,
        ResponsibilityMode::Even,
        10_001,
        vec![share(1, 3_334), share(2, 3_334), share(5, 3_333)],
    )]);
    let mut state = blank(&vm);
    apply(&mut state, &vm, 1);
    // Participants stay exactly {1, 2, 5}; the roster has no other active person to spill into.
    assert_eq!(state.shared_amounts.len(), 3);
    // An amount edit re-splits among exactly those people, remainder to the lowest ids.
    state.amount_input = "100,02".into();
    state.apply_amount_split();
    let ids: Vec<i64> = state.shared_amounts.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![1, 2, 5]);
    let draft = state.to_draft().unwrap();
    assert_eq!(
        draft.allocations,
        vec![share(1, 3_334), share(2, 3_334), share(5, 3_334)]
    );
    assert_eq!(draft.split_mode, Some(SplitMode::Even));
    state.amount_input = "100,01".into();
    state.apply_amount_split();
    assert_eq!(
        state.to_draft().unwrap().allocations,
        vec![share(1, 3_334), share(2, 3_334), share(5, 3_333)]
    );
}

#[test]
fn a_subset_template_does_not_spread_over_the_whole_roster() {
    let vm = vm_with(vec![template(
        1,
        ResponsibilityMode::Even,
        10_000,
        vec![share(2, 5_000), share(5, 5_000)],
    )]);
    let mut state = blank(&vm);
    apply(&mut state, &vm, 1);
    state.amount_input = "50,00".into();
    state.apply_amount_split();
    let draft = state.to_draft().unwrap();
    assert_eq!(draft.allocations, vec![share(2, 2_500), share(5, 2_500)]);
}

#[test]
fn a_custom_template_needs_repair_after_an_amount_edit() {
    let vm = vm_with(vec![template(
        1,
        ResponsibilityMode::Custom,
        10_000,
        vec![share(1, 6_000), share(2, 4_000)],
    )]);
    let mut state = blank(&vm);
    apply(&mut state, &vm, 1);
    state.amount_input = "120,00".into();
    state.apply_amount_split();
    let errors = state.to_draft().unwrap_err();
    assert!(errors.iter().any(|e| matches!(
        e,
        QuickAddError::Validation(ValidationError::ShareSumMismatch { .. })
    )));
    state.edit_share(1, "80,00".into());
    assert!(
        state.to_draft().is_ok(),
        "the user repairs the shares by hand"
    );
}

#[test]
fn payer_none_and_a_single_person_template_stay_explicit() {
    let mut t = template(1, ResponsibilityMode::Person, 900, vec![share(2, 900)]);
    t.draft.payer_id = None;
    let vm = vm_with(vec![t]);
    let mut state = blank(&vm);
    apply(&mut state, &vm, 1);
    assert_eq!(state.split, SplitChoice::Person(2));
    let draft = state.to_draft().unwrap();
    assert_eq!(
        draft.payer_id, None,
        "an unknown payer is not replaced by the owner"
    );
    // Changing who is responsible does not silently change the explicit payer.
    state.set_split(SplitChoice::Person(1));
    assert_eq!(state.to_draft().unwrap().payer_id, None);
}

#[test]
fn archived_and_incompatible_references_block_saving_until_repaired() {
    let mut t = template(
        1,
        ResponsibilityMode::Custom,
        10_000,
        vec![share(1, 6_000), share(9, 4_000)],
    );
    t.draft.category_id = Some(11); // archived
    t.draft.payer_id = Some(9); // archived
    let vm = vm_with(vec![t]);
    let mut state = blank(&vm);
    apply(&mut state, &vm, 1);
    let errors = state.to_draft().unwrap_err();
    for repair in [Repair::Category(11), Repair::Payer(9), Repair::Person(9)] {
        assert!(
            errors.contains(&QuickAddError::Repair(repair)),
            "{repair:?}"
        );
    }
    // Each is repaired by an explicit choice.
    state.category_id = None;
    state.set_payer(Some(1));
    state.edit_share(9, String::new());
    assert!(!state.current_repairs().contains(&Repair::Person(9)));
    // One person left means a single responsible person, not a shared split.
    state.set_split(SplitChoice::Person(1));
    assert!(state.current_repairs().is_empty());
    assert!(state.to_draft().is_ok());

    // A category whose scope no longer fits the kind is a repair too.
    let mut drifted = template(2, ResponsibilityMode::None, 500, vec![]);
    drifted.draft.category_id = Some(12);
    let vm = vm_with(vec![drifted]);
    let mut state = blank(&vm);
    apply(&mut state, &vm, 2);
    assert!(
        state
            .to_draft()
            .unwrap_err()
            .contains(&QuickAddError::Repair(Repair::Category(12)))
    );
    state.category_id = Some(10);
    assert!(state.to_draft().is_ok());
}

#[test]
fn dirty_tracks_raw_values_not_focus_sections_or_decorations() {
    let vm = vm_with(vec![]);
    let mut state = blank(&vm);
    assert!(!state.is_dirty());
    // Focus, expanded sections and error decorations are not edits.
    state.show_more = true;
    state.focus_amount = false;
    state.focus_title = true;
    state.errors = vec![QuickAddError::Save];
    assert!(!state.is_dirty());
    // Invalid input and unparsed one-line text are edits.
    state.amount_input = "abc".into();
    assert!(state.is_dirty());
    state.amount_input.clear();
    assert!(!state.is_dirty());
    state.line_input = "45,90 Coffee".into();
    assert!(state.is_dirty());
    assert!(
        !state.structured_dirty(),
        "the line alone is not structured"
    );
    state.line_input.clear();
    // A participant selection counts.
    state.amount_input = "10,00".into();
    state.set_split(SplitChoice::Shared);
    state.rebase();
    state.edit_share(2, String::new());
    assert!(state.is_dirty());
    state.rebase();
    assert!(!state.is_dirty());
}

#[test]
fn a_clean_form_closes_at_once_and_a_dirty_one_waits_for_the_prompt() {
    let vm = vm_with(vec![]);
    let mut state = blank(&vm);
    assert_eq!(
        state.request(QuickAddIntent::Close, &vm),
        Some(QuickAddEvent::Close)
    );
    state.title = "Typed".into();
    assert_eq!(state.request(QuickAddIntent::Close, &vm), None);
    assert_eq!(state.pending_intent, Some(QuickAddIntent::Close));
    assert_eq!(state.title, "Typed", "nothing was touched while asking");
    // Discard performs the stored intent exactly once.
    let intent = state.pending_intent.take().unwrap();
    assert_eq!(
        state.perform_intent(intent, &vm),
        Some(QuickAddEvent::Close)
    );
    assert_eq!(state.pending_intent, None);
}

#[test]
fn replacing_a_dirty_form_with_a_template_is_cancellable_and_discard_applies_once() {
    let vm = vm_with(vec![template(1, ResponsibilityMode::None, 500, vec![])]);
    let mut state = blank(&vm);
    state.title = "Half typed".into();
    state.amount_input = "7".into();
    assert_eq!(state.request(QuickAddIntent::ApplyTemplate(1), &vm), None);
    assert_eq!(state.pending_intent, Some(QuickAddIntent::ApplyTemplate(1)));
    assert_eq!(state.title, "Half typed");
    // Keep editing: nothing changes.
    state.pending_intent = None;
    assert_eq!(
        (state.title.as_str(), state.amount_input.as_str()),
        ("Half typed", "7")
    );
    // Ask again and discard.
    state.request(QuickAddIntent::ApplyTemplate(1), &vm);
    let intent = state.pending_intent.take().unwrap();
    assert_eq!(state.perform_intent(intent, &vm), None);
    assert_eq!(state.title, "Groceries");
    assert_eq!(state.amount_input, "5,00");
    assert_eq!(state.template_id, Some(1));
    assert!(!state.is_dirty());
}

#[test]
fn one_line_review_parses_without_saving_and_populates_a_fresh_form() {
    let vm = vm_with(vec![]);
    let mut state = blank(&vm);
    state.line_input = "45,90 Café @2026-11-05".into();
    let (parsed, kind) = state.review_line(vm.today).expect("a valid positive line");
    assert_eq!(kind, TransactionType::Expense);
    // The line is a structured-clean form, so it replaces without a prompt.
    assert_eq!(
        state.request(QuickAddIntent::ReplaceWithParsed(parsed, kind), &vm),
        None
    );
    assert_eq!(state.pending_intent, None);
    assert_eq!(state.title, "Café");
    assert_eq!(state.amount_input, "45,90");
    assert_eq!(
        state.date_text,
        crate::i18n::format_date(NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(), Language::En)
    );
    assert_eq!(state.category_id, None, "no inferred entity matches");
    assert!(state.line_input.is_empty());
    assert!(state.from_line);
    assert_eq!(state.mode, QuickAddMode::Create);
    // Save stays explicit: the draft is only built when asked for.
    assert_eq!(state.to_draft().unwrap().amount, Money::from_cents(4_590));
}

#[test]
fn a_negative_line_asks_for_expense_or_refund() {
    let vm = vm_with(vec![]);
    let mut state = blank(&vm);
    state.line_input = "-20,00 Estorno".into();
    assert!(state.review_line(vm.today).is_none());
    assert!(state.kind_choice.is_some());
    assert_eq!(state.line_error, None);
    let (parsed, kind) = state.choose_line_kind(TransactionType::Refund).unwrap();
    assert_eq!(kind, TransactionType::Refund);
    state.request(QuickAddIntent::ReplaceWithParsed(parsed, kind), &vm);
    assert_eq!(state.kind, TransactionType::Refund);
    assert_eq!(state.amount_input, "20,00");
    assert!(state.kind_choice.is_none());
}

#[test]
fn bad_lines_stay_editable_and_do_not_touch_the_form() {
    let vm = vm_with(vec![]);
    for (line, error) in [
        ("", QuickEntryError::Empty),
        ("45,90", QuickEntryError::MissingTitle),
        ("0 Free", QuickEntryError::ZeroAmount),
        ("10 x @2026-13-45", QuickEntryError::InvalidDate),
        ("99999999999999999999 Huge", QuickEntryError::AmountTooLarge),
    ] {
        let mut state = blank(&vm);
        state.title = "Keep me".into();
        state.line_input = line.into();
        assert!(state.review_line(vm.today).is_none(), "{line}");
        assert_eq!(state.line_error, Some(error), "{line}");
        assert_eq!(state.line_input, line, "the line stays editable");
        assert_eq!(state.title, "Keep me");
    }
}

#[test]
fn replacing_a_structurally_dirty_form_with_a_parsed_line_asks_first() {
    let vm = vm_with(vec![]);
    let mut state = blank(&vm);
    state.title = "Typed".into();
    state.line_input = "10 Tea".into();
    let (parsed, kind) = state.review_line(vm.today).unwrap();
    assert_eq!(
        state.request(QuickAddIntent::ReplaceWithParsed(parsed.clone(), kind), &vm),
        None
    );
    assert_eq!(
        state.pending_intent,
        Some(QuickAddIntent::ReplaceWithParsed(parsed, kind))
    );
    assert_eq!(state.title, "Typed");
}

#[test]
fn create_tools_are_hidden_for_edit_and_locked_settlement_recording() {
    let people = two_people();
    let state = QuickAddState::new(&people, today(), Language::En);
    assert!(state.create_tools_visible());
    let locked = QuickAddState::for_settlement(
        &people,
        1,
        2,
        Money::from_cents(100),
        today(),
        "x".into(),
        Language::En,
    );
    assert!(!locked.create_tools_visible());
}
