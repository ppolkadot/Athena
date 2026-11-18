//! Quick Add flows driven through the whole app: templates, dirty-draft prompts, one-line entry,
//! duplicate consent and the Settings Templates page (contract §14.1).

use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::actions::AppAction;
use super::{AppCore, AthenaApp};
use crate::application::services::onboarding::OnboardingResult;
use crate::domain::category::CategoryScope;
use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::settings::Language;
use crate::domain::template::{ResponsibilityMode, TemplateDraft};
use crate::domain::transaction::{Allocation, TransactionType, TxFilter};
use crate::infrastructure::db::{Db, integrity, templates_repo, transactions_repo};
use crate::shared::test_fixtures::{date, draft_with, today};
use crate::ui::quick_add::QuickAddMode;
use crate::ui::theme;

fn core() -> AppCore {
    let mut core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
    core.complete_onboarding(OnboardingResult {
        display_name: "A3".into(),
        partner_name: Some("Ana".into()),
        start_balance: Money::ZERO,
        language: Language::En,
    })
    .unwrap();
    core.ui.set_month(YearMonth::new(2026, 11).unwrap());
    core
}

fn harness(core: AppCore) -> Harness<'static, AthenaApp> {
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(1280.0, 1000.0))
        .with_step_dt(0.05)
        .with_context_setup(theme::typography::install_test_fonts)
        .build_ui_state(
            |ui, app: &mut AthenaApp| {
                ui.ctx()
                    .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                theme::apply(ui.ctx(), app.core.vm.theme());
                let style = ui.ctx().global_style();
                ui.set_style(style);
                app.show(ui);
            },
            AthenaApp::for_test(core),
        );
    harness.run_steps(6);
    harness
}

fn open_quick_add(h: &mut Harness<'static, AthenaApp>) {
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(14);
    assert!(h.state().quick_add.is_some());
}

fn input<'a>(h: &'a Harness<'static, AthenaApp>, index: usize) -> egui_kittest::Node<'a> {
    h.get_all_by_role(egui::Role::TextInput).nth(index).unwrap()
}

/// Text inputs on Month with Quick Add open: 0 search, 1 one-line, 2 amount, 3 title, 4 date.
const LINE: usize = 1;
const AMOUNT: usize = 2;
const TITLE: usize = 3;

fn type_replacing(h: &mut Harness<'static, AthenaApp>, index: usize, text: &str) {
    input(h, index).focus();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.run_steps(2);
    input(h, index).type_text(text);
    h.run_steps(3);
}

fn people(core: &AppCore) -> (i64, i64) {
    (core.vm.all_people[0].id, core.vm.all_people[1].id)
}

fn category_id(core: &AppCore, name: &str) -> i64 {
    core.vm
        .all_categories
        .iter()
        .find(|c| c.name == name)
        .unwrap()
        .id
}

fn weekly(core: &AppCore) -> TemplateDraft {
    let (owner, partner) = people(core);
    TemplateDraft {
        name: "Weekly groceries".into(),
        title: "Groceries".into(),
        amount: Money::from_cents(10_000),
        kind: TransactionType::Expense,
        category_id: Some(category_id(core, "Food")),
        payer_id: Some(owner),
        description: Some("Saturday".into()),
        responsibility: ResponsibilityMode::Even,
        allocations: vec![
            Allocation {
                person_id: owner,
                amount: Money::from_cents(5_000),
            },
            Allocation {
                person_id: partner,
                amount: Money::from_cents(5_000),
            },
        ],
    }
}

fn rows(h: &Harness<'static, AthenaApp>) -> usize {
    transactions_repo::list(h.state().core.store.db().conn(), &TxFilter::default())
        .unwrap()
        .len()
}

#[test]
fn create_apply_template_edit_amount_and_save_writes_once() {
    let mut core = core();
    let template = weekly(&core);
    core.apply(AppAction::SaveTemplate(template)).unwrap();
    let mut h = harness(core);
    open_quick_add(&mut h);
    let version = h.state().core.store.data_version();

    h.get_by_label("Templates").click();
    h.run_steps(4);
    h.get_by_label("Weekly groceries").click();
    h.run_steps(6);

    let qa = h.state().quick_add.as_ref().unwrap();
    assert_eq!(qa.title, "Groceries");
    assert_eq!(qa.amount_input, "100,00");
    assert_eq!(qa.date_text, "2026-11-11", "Today is visible in the form");
    assert_eq!(qa.template_id, Some(1));
    assert_eq!(
        h.state().core.store.data_version(),
        version,
        "applying a template never writes"
    );
    assert_eq!(rows(&h), 0);

    // Edit the amount: the even split follows it, then a deliberate Save writes once.
    type_replacing(&mut h, AMOUNT, "100,01");
    assert_eq!(h.state().quick_add.as_ref().unwrap().amount_input, "100,01");
    h.key_press(Key::Enter);
    h.run_steps(8);

    assert_eq!(rows(&h), 1);
    let conn = h.state().core.store.db().conn();
    let saved = &transactions_repo::list(conn, &TxFilter::default()).unwrap()[0];
    assert_eq!(saved.amount, Money::from_cents(10_001));
    assert_eq!(saved.date, today());
    let shares: Vec<i64> = saved.allocations.iter().map(|a| a.amount.cents()).collect();
    assert_eq!(
        shares,
        vec![5_001, 5_000],
        "remainder goes to the lowest id"
    );
    let used = templates_repo::get(conn, 1).unwrap().last_used_at;
    assert_eq!(used.as_deref(), Some(saved.created_at.as_str()));
    assert!(integrity::check(conn).unwrap().is_empty());
    // The form is ready for the next entry and the template link is cleared.
    let qa = h.state().quick_add.as_ref().unwrap();
    assert!(qa.title.is_empty());
    assert_eq!(qa.template_id, None);
}

#[test]
fn escape_on_a_dirty_form_asks_and_keep_editing_restores_the_form() {
    let mut h = harness(core());
    open_quick_add(&mut h);
    type_replacing(&mut h, TITLE, "Half typed");
    h.key_press(Key::Escape);
    h.run_steps(8);
    assert!(h.query_by_label("Discard this entry?").is_some());
    assert_eq!(h.state().quick_add.as_ref().unwrap().title, "Half typed");

    // Escape on the prompt is Keep editing: the form is untouched.
    h.key_press(Key::Escape);
    h.run_steps(8);
    assert!(h.query_by_label("Discard this entry?").is_none());
    assert!(h.state().quick_add.is_some());
    assert_eq!(h.state().quick_add.as_ref().unwrap().title, "Half typed");

    // Ask again; the button also keeps editing; Discard closes without saving.
    h.key_press(Key::Escape);
    h.run_steps(8);
    h.get_by_label("Keep editing").click();
    h.run_steps(6);
    assert!(h.state().quick_add.is_some());
    h.key_press(Key::Escape);
    h.run_steps(8);
    h.get_by_label("Discard").click();
    h.run_steps(8);
    assert!(h.state().quick_add.is_none());
    assert_eq!(rows(&h), 0);
}

#[test]
fn a_clean_form_closes_on_escape_without_a_prompt() {
    let mut h = harness(core());
    open_quick_add(&mut h);
    h.key_press(Key::Escape);
    h.run_steps(8);
    assert!(h.state().quick_add.is_none());
}

#[test]
fn clicking_away_from_a_dirty_form_asks_before_discarding() {
    let mut h = harness(core());
    open_quick_add(&mut h);
    type_replacing(&mut h, TITLE, "Typed");
    // A click on the backdrop, far from the modal.
    let pos = egui::pos2(8.0, 8.0);
    h.hover_at(pos);
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(2);
    }
    h.run_steps(6);
    assert!(h.query_by_label("Discard this entry?").is_some());
    assert!(h.state().quick_add.is_some());
    assert_eq!(h.state().quick_add.as_ref().unwrap().title, "Typed");
}

#[test]
fn one_line_enter_only_reviews_and_a_deliberate_save_writes_once() {
    let mut h = harness(core());
    open_quick_add(&mut h);
    let version = h.state().core.store.data_version();
    input(&h, LINE).focus();
    h.run_steps(2);
    input(&h, LINE).type_text("45,90 Café @2026-11-05");
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(8);

    let qa = h.state().quick_add.as_ref().unwrap();
    assert_eq!(qa.title, "Café");
    assert_eq!(qa.amount_input, "45,90");
    assert_eq!(qa.date_text, "2026-11-05");
    assert!(qa.line_input.is_empty());
    assert_eq!(
        h.state().core.store.data_version(),
        version,
        "parsing never writes"
    );
    assert_eq!(rows(&h), 0);

    h.key_press(Key::Enter);
    h.run_steps(8);
    assert_eq!(rows(&h), 1);
    let saved = transactions_repo::list(h.state().core.store.db().conn(), &TxFilter::default())
        .unwrap()
        .remove(0);
    assert_eq!(
        (saved.title.as_str(), saved.date),
        ("Café", date(2026, 11, 5))
    );
    // A one-line save resets to a fresh form and focuses the empty line.
    let qa = h.state().quick_add.as_ref().unwrap();
    assert!(qa.title.is_empty() && qa.line_input.is_empty());
    assert!(!qa.from_line);
}

#[test]
fn the_review_button_matches_enter_and_invalid_lines_stay_editable() {
    let mut h = harness(core());
    open_quick_add(&mut h);
    input(&h, LINE).focus();
    h.run_steps(2);
    input(&h, LINE).type_text("45,90");
    h.run_steps(3);
    h.get_by_label("Review").click();
    h.run_steps(6);
    let qa = h.state().quick_add.as_ref().unwrap();
    assert!(qa.line_error.is_some(), "a bare amount is rejected");
    assert_eq!(qa.line_input, "45,90", "the line stays editable");
    assert!(h.query_by_label("Add a title after the amount.").is_some());
    assert_eq!(rows(&h), 0);

    input(&h, LINE).type_text(" Coffee");
    h.run_steps(3);
    h.get_by_label("Review").click();
    h.run_steps(6);
    let qa = h.state().quick_add.as_ref().unwrap();
    assert_eq!(
        (qa.title.as_str(), qa.amount_input.as_str()),
        ("Coffee", "45,90")
    );
    assert_eq!(rows(&h), 0, "Review never saves");
}

#[test]
fn a_negative_line_asks_for_the_kind_before_creating_the_draft() {
    let mut h = harness(core());
    open_quick_add(&mut h);
    input(&h, LINE).focus();
    h.run_steps(2);
    input(&h, LINE).type_text("-20,00 Estorno");
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(8);
    assert!(h.query_by_label("Negative amount. Save it as:").is_some());
    assert!(h.state().quick_add.as_ref().unwrap().title.is_empty());
    // The choice buttons come before the kind selector, which also has a Refund.
    h.get_all_by_label("Refund").next().unwrap().click();
    h.run_steps(6);
    let qa = h.state().quick_add.as_ref().unwrap();
    assert_eq!(qa.kind, TransactionType::Refund);
    assert_eq!(
        (qa.title.as_str(), qa.amount_input.as_str()),
        ("Estorno", "20,00")
    );
    assert_eq!(rows(&h), 0);
}

#[test]
fn held_enter_repeats_never_save_twice_or_act_across_the_review() {
    let mut h = harness(core());
    open_quick_add(&mut h);
    input(&h, LINE).focus();
    h.run_steps(2);
    input(&h, LINE).type_text("10 Tea");
    h.run_steps(3);
    // A held key: one press, then a stream of repeat events.
    h.key_down(Key::Enter);
    for _ in 0..6 {
        h.event(egui::Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: true,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.key_up(Key::Enter);
    h.run_steps(6);
    assert_eq!(rows(&h), 0, "the review frame and its repeats never save");
    assert_eq!(h.state().quick_add.as_ref().unwrap().title, "Tea");

    // Now a held Enter on the populated form saves once.
    h.key_down(Key::Enter);
    h.run_steps(2);
    for _ in 0..6 {
        h.event(egui::Event::Key {
            key: Key::Enter,
            physical_key: Some(Key::Enter),
            pressed: true,
            repeat: true,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(1);
    }
    h.key_up(Key::Enter);
    h.run_steps(6);
    assert_eq!(rows(&h), 1, "one press saves once; repeats are ignored");
}

#[test]
fn a_write_failure_keeps_the_typed_form_and_reports_it() {
    let mut core = core();
    let template = weekly(&core);
    core.apply(AppAction::SaveTemplate(template)).unwrap();
    let mut h = harness(core);
    open_quick_add(&mut h);
    h.get_by_label("Templates").click();
    h.run_steps(4);
    h.get_by_label("Weekly groceries").click();
    h.run_steps(6);
    // The template disappears behind the form (for example deleted from another window).
    templates_repo::delete(h.state().core.store.db().conn(), 1).unwrap();
    h.key_press(Key::Enter);
    h.run_steps(8);
    assert_eq!(rows(&h), 0, "the failed usage update rolled the entry back");
    let qa = h.state().quick_add.as_ref().unwrap();
    assert_eq!(qa.title, "Groceries", "the typed form is retained");
    assert!(!qa.errors.is_empty());
    assert!(
        h.query_by_label("Could not save. Please try again.")
            .is_some()
    );
}

#[test]
fn a_duplicate_needs_consent_by_keyboard_or_mouse() {
    let mut core = core();
    let (owner, _) = people(&core);
    let mut existing = draft_with(vec![Allocation {
        person_id: owner,
        amount: Money::from_cents(4_590),
    }]);
    existing.title = "Café".into();
    existing.amount = Money::from_cents(4_590);
    existing.date = date(2026, 11, 11);
    core.create_transaction(existing).unwrap();
    let mut h = harness(core);
    open_quick_add(&mut h);
    type_replacing(&mut h, AMOUNT, "45,90");
    type_replacing(&mut h, TITLE, "  café ");
    h.key_press(Key::Enter);
    h.run_steps(8);
    assert!(h.query_by_label("Possible duplicate").is_some());
    assert_eq!(rows(&h), 1, "nothing is written while the prompt is open");

    // Keep editing returns to the form, still unsaved.
    h.get_by_label("Keep editing").click();
    h.run_steps(6);
    assert!(h.query_by_label("Possible duplicate").is_none());
    assert_eq!(rows(&h), 1);

    // Mouse: Save anyway writes once.
    h.key_press(Key::Enter);
    h.run_steps(8);
    h.get_by_label("Save anyway").click();
    h.run_steps(8);
    assert_eq!(rows(&h), 2);
    assert!(h.state().quick_add.as_ref().unwrap().duplicate.is_none());
}

#[test]
fn a_changed_draft_invalidates_the_duplicate_consent() {
    let mut core = core();
    let (owner, _) = people(&core);
    let mut existing = draft_with(vec![Allocation {
        person_id: owner,
        amount: Money::from_cents(100),
    }]);
    existing.title = "Tea".into();
    existing.date = date(2026, 11, 11);
    core.create_transaction(existing).unwrap();
    let mut h = harness(core);
    open_quick_add(&mut h);
    type_replacing(&mut h, AMOUNT, "1,00");
    type_replacing(&mut h, TITLE, "Tea");
    h.key_press(Key::Enter);
    h.run_steps(8);
    assert!(h.state().quick_add.as_ref().unwrap().duplicate.is_some());
    // Any change to the draft clears the consent, so Save anyway can no longer apply to it.
    h.state_mut().quick_add.as_mut().unwrap().title = "Tea for two".into();
    h.run_steps(4);
    assert!(h.state().quick_add.as_ref().unwrap().duplicate.is_none());
    assert_eq!(rows(&h), 1);
}

#[test]
fn settings_templates_edit_and_named_delete() {
    use crate::app::navigation::Screen;
    use crate::ui::screens::settings::SettingsPage;

    let mut core = core();
    let template = weekly(&core);
    core.apply(AppAction::SaveTemplate(template)).unwrap();
    core.ui.settings.page = SettingsPage::Templates;
    core.set_screen(Screen::Settings);
    let mut h = harness(core);
    assert!(h.query_by_label("Weekly groceries").is_some());

    // Edit: the same form, with a name and no date.
    h.get_by_label("Edit template Weekly groceries").click();
    h.run_steps(14);
    let qa = h.state().quick_add.as_ref().unwrap();
    assert_eq!(qa.mode, QuickAddMode::Template(1));
    assert!(h.query_by_label("Edit template").is_some());
    assert!(h.query_by_label("Date").is_none(), "no date in a template");
    assert_eq!(
        h.state().quick_add.as_ref().unwrap().template_name,
        "Weekly groceries"
    );
    // Settings is the page behind the modal, so the inputs are: name, amount, title.
    h.state_mut().quick_add.as_mut().unwrap().template_name = "Saturday shop".into();
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(10);
    assert!(h.state().quick_add.is_none());
    let renamed = templates_repo::get(h.state().core.store.db().conn(), 1).unwrap();
    assert_eq!(renamed.draft.name, "Saturday shop");
    assert!(h.query_by_label("Saturday shop").is_some());

    // Delete asks by name; Cancel keeps it, confirming removes only that template.
    h.get_by_label("Delete template Saturday shop").click();
    h.run_steps(8);
    assert!(h.query_by_label("Delete this template?").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(6);
    assert_eq!(
        templates_repo::count(h.state().core.store.db().conn()).unwrap(),
        1
    );
    h.get_by_label("Delete template Saturday shop").click();
    h.run_steps(8);
    h.get_by_label("Delete").click();
    h.run_steps(8);
    assert_eq!(
        templates_repo::count(h.state().core.store.db().conn()).unwrap(),
        0
    );
    assert!(h.state().core.vm.templates.is_empty());
    let _ = CategoryScope::Both;
}

#[test]
fn closing_the_window_with_a_dirty_form_uses_the_discard_prompt() {
    let mut h = harness(core());
    open_quick_add(&mut h);
    type_replacing(&mut h, TITLE, "Unsaved");
    // The window manager asks to close.
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.run_steps(8);
    assert!(h.query_by_label("Discard this entry?").is_some());
    assert!(!h.state().close_confirmed);
    assert_eq!(h.state().quick_add.as_ref().unwrap().title, "Unsaved");
    h.get_by_label("Discard").click();
    h.run_steps(8);
    assert!(
        h.state().close_confirmed,
        "Discard asks the window to close once"
    );
    assert_eq!(rows(&h), 0);
}

#[test]
fn save_as_template_asks_for_a_name_and_keeps_the_form() {
    let mut h = harness(core());
    open_quick_add(&mut h);
    type_replacing(&mut h, AMOUNT, "12,50");
    type_replacing(&mut h, TITLE, "Bakery");
    h.get_by_label("Save as template").click();
    h.run_steps(10);
    assert!(h.query_by_label("Template name").is_some());
    assert_eq!(
        templates_repo::count(h.state().core.store.db().conn()).unwrap(),
        0
    );

    // An empty name is refused in the prompt; the form behind it is untouched.
    h.get_by_label("Save template").click();
    h.run_steps(6);
    assert!(h.query_by_label("Give the template a name.").is_some());
    h.get_by_label("Template name").focus();
    h.run_steps(2);
    h.get_by_label("Template name").type_text("Morning bakery");
    h.run_steps(3);
    h.key_press(Key::Enter);
    h.run_steps(8);

    let templates = &h.state().core.vm.templates;
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].draft.name, "Morning bakery");
    assert_eq!(templates[0].draft.amount, Money::from_cents(1_250));
    assert_eq!(templates[0].draft.title, "Bakery");
    assert_eq!(rows(&h), 0, "only the template is saved");
    let qa = h.state().quick_add.as_ref().unwrap();
    assert_eq!(qa.title, "Bakery", "the form is retained");
    assert!(qa.template_prompt.is_none());
    assert!(
        integrity::check(h.state().core.store.db().conn())
            .unwrap()
            .is_empty()
    );

    // A second template with the same folded name is refused with a message in the prompt.
    h.get_by_label("Save as template").click();
    h.run_steps(10);
    h.get_by_label("Template name").focus();
    h.run_steps(2);
    h.get_by_label("Template name").type_text("MORNING BAKERY");
    h.run_steps(3);
    h.get_by_label("Save template").click();
    h.run_steps(8);
    assert!(
        h.query_by_label("Another template already has this name.")
            .is_some()
    );
    assert_eq!(h.state().core.vm.templates.len(), 1);
    // Escape cancels only the prompt.
    h.key_press(Key::Escape);
    h.run_steps(8);
    assert!(
        h.state()
            .quick_add
            .as_ref()
            .unwrap()
            .template_prompt
            .is_none()
    );
    assert_eq!(h.state().quick_add.as_ref().unwrap().title, "Bakery");
}

#[test]
fn the_duplicate_check_ignores_register_filters_and_excludes_the_edited_row() {
    let mut core = core();
    let (owner, _) = people(&core);
    let mut existing = draft_with(vec![Allocation {
        person_id: owner,
        amount: Money::from_cents(500),
    }]);
    existing.title = "Lunch".into();
    existing.amount = Money::from_cents(500);
    existing.date = date(2026, 11, 11);
    let id = {
        core.create_transaction(existing).unwrap();
        core.vm.transactions[0].id
    };
    // A register filter that hides the row does not hide the duplicate.
    core.ui.filter.text = "nothing matches this".into();
    let mut h = harness(core);
    open_quick_add(&mut h);
    type_replacing(&mut h, AMOUNT, "5,00");
    type_replacing(&mut h, TITLE, "Lunch");
    h.key_press(Key::Enter);
    h.run_steps(8);
    assert!(h.query_by_label("Possible duplicate").is_some());
    h.get_by_label("Keep editing").click();
    h.run_steps(6);
    h.key_press(Key::Escape);
    h.run_steps(6);
    h.get_by_label("Discard").click();
    h.run_steps(8);

    // Editing the row itself never matches itself.
    h.state_mut().core.ui.clear_filters();
    h.run_steps(6);
    let state = super::modals::begin_edit(&h.state().core, id).unwrap();
    h.state_mut().quick_add = Some(state);
    h.run_steps(12);
    h.key_press(Key::Enter);
    h.run_steps(8);
    assert!(h.query_by_label("Possible duplicate").is_none());
    assert!(h.state().quick_add.is_none(), "an edit saves and closes");
    assert_eq!(rows(&h), 1);
}
