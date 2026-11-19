//! Monthly closings driven through the whole app (contract §14.3): Close, Edit the previous month,
//! Recalculate, and the Calendar markers. Saved figures stay fixed until a confirmed recalculation.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::actions::{AppAction, AppearanceApply};
use super::navigation::Screen;
use super::{AppCore, AthenaApp};
use crate::domain::appearance::{AppearancePrefs, TextScale, ThemeMode, Wallpaper};
use crate::domain::closing::ClosingStatus;
use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::settings::{Language, Profile, Theme};
use crate::domain::transaction::TransactionType;
use crate::infrastructure::db::{Db, closings_repo, integrity, trash_repo};
use crate::shared::test_fixtures::{date, draft_with, seed_month, today};
use crate::ui::theme;

fn oct() -> YearMonth {
    YearMonth::new(2026, 10).unwrap()
}

fn core_in(language: Language) -> AppCore {
    let db = Db::open_in_memory().unwrap();
    seed_month(&db);
    let mut core = AppCore::empty_for_test(db);
    core.save_profile(Profile {
        start_balance: Money::ZERO,
        theme: Theme::Dark,
        language,
        onboarding_complete: true,
    })
    .unwrap();
    core.store.freeze_today(today());
    core
}

fn save(core: &mut AppCore, on: chrono::NaiveDate, cents: i64, title: &str) {
    let mut draft = draft_with(vec![]);
    draft.date = on;
    draft.amount = Money::from_cents(cents);
    draft.kind = TransactionType::Expense;
    draft.title = title.into();
    core.apply(AppAction::SaveTransaction(draft)).unwrap();
}

fn harness(core: AppCore) -> Harness<'static, AthenaApp> {
    let mut app = AthenaApp::for_test(core);
    app.pin_today(today());
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(1280.0, 1400.0))
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
            app,
        );
    harness.run_steps(6);
    harness
}

fn stored(h: &Harness<'static, AthenaApp>) -> Option<crate::domain::closing::ClosingSnapshot> {
    closings_repo::get(h.state().core.store.db().conn(), oct()).unwrap()
}

#[test]
fn close_cancel_close_edit_prior_month_recalculate_cancel_confirm() {
    let mut core = core_in(Language::En);
    save(&mut core, date(2026, 9, 12), 10_000, "September shop");
    save(&mut core, date(2026, 10, 5), 40_000, "Rent");
    save(&mut core, date(2026, 10, 20), 2_500, "Coffee beans");
    core.open_month(oct()).unwrap();
    let mut h = harness(core);

    // A completed, unsaved month offers Close month; the current and a future month do not.
    assert!(h.query_by_label("Close month October 2026").is_some());
    h.state_mut()
        .core
        .set_month(YearMonth::new(2026, 11).unwrap())
        .unwrap();
    h.run_steps(4);
    assert!(h.query_by_label_contains("Close month").is_none());
    h.state_mut()
        .core
        .set_month(YearMonth::new(2027, 3).unwrap())
        .unwrap();
    h.run_steps(4);
    assert!(h.query_by_label_contains("Close month").is_none());
    h.state_mut().core.set_month(oct()).unwrap();
    h.run_steps(4);

    // Cancel writes nothing; confirming saves the figures.
    h.get_by_label("Close month October 2026").click();
    h.run_steps(4);
    assert!(h.query_by_label("Close October 2026?").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert!(stored(&h).is_none());
    h.get_by_label("Close month October 2026").click();
    h.run_steps(4);
    h.get_by_label("Close month").click();
    h.run_steps(6);
    let saved = stored(&h).expect("the closing was saved");
    assert_eq!(saved.expenses.cents(), 42_500);
    assert_eq!(saved.previous_spending.cents(), 10_000);
    assert!(h.query_by_label("Saved closing").is_some());
    assert!(h.query_by_label("Changed since closing").is_none());
    assert_eq!(
        h.state().core.vm.closing_status.get(&oct()),
        Some(&ClosingStatus::Current)
    );
    assert!(
        h.query_by_label("Recalculate closing October 2026")
            .is_some()
    );
    assert!(h.query_by_label("Close month October 2026").is_none());

    // Editing the previous month marks the closing as changed; the saved numbers do not move.
    h.state_mut()
        .core
        .apply(AppAction::SaveTransaction({
            let mut d = draft_with(vec![]);
            d.date = date(2026, 9, 30);
            d.amount = Money::from_cents(5_000);
            d.title = "Late September".into();
            d
        }))
        .unwrap();
    h.run_steps(6);
    assert!(h.query_by_label("Changed since closing").is_some());
    assert_eq!(stored(&h).unwrap(), saved);

    // Recalculate: Cancel keeps the old snapshot, Confirm replaces it and clears the marker.
    h.get_by_label("Recalculate closing October 2026").click();
    h.run_steps(4);
    assert!(
        h.query_by_label("Recalculate the October 2026 closing?")
            .is_some()
    );
    assert!(
        h.query_by_label_contains("replaced with today's figures")
            .is_some()
    );
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert_eq!(stored(&h).unwrap(), saved);
    assert!(h.query_by_label("Changed since closing").is_some());
    h.get_by_label("Recalculate closing October 2026").click();
    h.run_steps(4);
    h.get_by_label("Recalculate").click();
    h.run_steps(6);
    let replaced = stored(&h).unwrap();
    assert_eq!(replaced.previous_spending.cents(), 15_000);
    assert_ne!(replaced.previous_fingerprint, saved.previous_fingerprint);
    assert!(h.query_by_label("Changed since closing").is_none());
    assert_eq!(
        integrity::check(h.state().core.store.db().conn()).unwrap(),
        vec![]
    );
}

#[test]
fn calendar_markers_name_the_status_and_open_the_saved_month() {
    let mut core = core_in(Language::En);
    save(&mut core, date(2026, 10, 5), 40_000, "Rent");
    core.apply(AppAction::CloseMonth(oct())).unwrap();
    core.set_screen(Screen::Calendar);
    core.set_calendar_year(2026).unwrap();
    let mut h = harness(core);
    assert!(h.query_by_label("October 2026, Closing saved").is_some());
    // A month without a closing has no marker.
    assert!(h.query_by_label("September 2026").is_some());

    // A change in the closed month flips the marker to its changed wording.
    h.state_mut()
        .core
        .apply(AppAction::SaveTransaction({
            let mut d = draft_with(vec![]);
            d.date = date(2026, 10, 6);
            d.amount = Money::from_cents(100);
            d.title = "Extra".into();
            d
        }))
        .unwrap();
    h.run_steps(6);
    assert!(
        h.query_by_label("October 2026, Changed since closing")
            .is_some()
    );
    // Selecting the card reaches the same saved card on Month.
    h.get_by_label("October 2026, Changed since closing")
        .click();
    h.run_steps(6);
    assert_eq!(h.state().core.ui.screen, Screen::Month);
    assert_eq!(h.state().core.ui.month, oct());
    assert!(h.query_by_label("Saved closing").is_some());
    assert!(h.query_by_label("Changed since closing").is_some());

    h.state_mut().core.set_screen(Screen::Calendar);
    h.run_steps(6);
    h.get_by_label("Days of October 2026").click();
    h.run_steps(6);
    h.get_by_label("Changed since closing, Open month").focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(6);
    assert_eq!(h.state().core.ui.screen, Screen::Month);
    assert_eq!(h.state().core.ui.month, oct());
    assert!(h.query_by_label("Saved closing").is_some());
}

#[test]
fn staleness_is_staged_on_screen_changes_and_not_on_filter_changes() {
    let mut core = core_in(Language::En);
    save(&mut core, date(2026, 10, 5), 40_000, "Rent");
    core.apply(AppAction::CloseMonth(oct())).unwrap();
    core.open_month(oct()).unwrap();
    // Month checks the viewed month and its predecessor only.
    assert_eq!(core.vm.closing_status.len(), 1);
    // A filter change does not recompute.
    let before = core.vm.closing_status.clone();
    core.ui.filter.text = "rent".into();
    core.sync().unwrap();
    assert_eq!(core.vm.closing_status, before);
    // Calendar checks the displayed year.
    core.set_screen(Screen::Calendar);
    core.set_calendar_year(2026).unwrap();
    assert_eq!(
        core.vm.closing_status.get(&oct()),
        Some(&ClosingStatus::Current)
    );
    // A failed check shows unavailable instead of claiming freshness; retry recovers.
    core.store
        .db()
        .conn()
        .execute_batch("SAVEPOINT inject; DROP VIEW live_transactions")
        .unwrap();
    core.vm.retry_closing_checks(&core.store, &core.ui);
    assert_eq!(
        core.vm.closing_status.get(&oct()),
        Some(&ClosingStatus::Unavailable)
    );
    core.store
        .db()
        .conn()
        .execute_batch("ROLLBACK TO inject; RELEASE inject")
        .unwrap();
    core.vm.retry_closing_checks(&core.store, &core.ui);
    assert_eq!(
        core.vm.closing_status.get(&oct()),
        Some(&ClosingStatus::Current)
    );
}

#[test]
fn the_saved_expense_opens_only_while_live_and_keeps_a_fallback_otherwise() {
    let mut core = core_in(Language::En);
    save(&mut core, date(2026, 10, 5), 90_000, "Sofa");
    save(&mut core, date(2026, 10, 6), 1_000, "Pen");
    core.apply(AppAction::CloseMonth(oct())).unwrap();
    core.open_month(oct()).unwrap();
    let sofa = core
        .vm
        .transactions
        .iter()
        .find(|t| t.title == "Sofa")
        .unwrap()
        .id;
    let mut h = harness(core);
    assert!(h.query_by_label("Open entry").is_some());
    h.get_by_label("Open entry").click();
    h.run_steps(4);
    assert!(h.state().quick_add.is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(4);

    // Deleted (in Trash) or purged: the card keeps the captured title and amount, without a link.
    h.state_mut()
        .core
        .apply(AppAction::DeleteTransaction(sofa))
        .unwrap();
    h.run_steps(6);
    assert!(h.query_by_label("Open entry").is_none());
    assert!(
        h.query_by_label_contains("The largest expense entry is no longer available")
            .is_some()
    );
    assert!(h.query_by_label_contains("Sofa").is_some());
    trash_repo::purge(h.state().core.store.db().conn(), sofa).unwrap();
    h.state_mut().core.apply(AppAction::EmptyTrash).unwrap();
    h.run_steps(6);
    assert_eq!(
        stored(&h).unwrap().largest_expense.as_ref().unwrap().id,
        None
    );
    assert!(h.query_by_label_contains("Sofa").is_some());
}

#[test]
fn the_closing_flow_is_localized_in_portuguese() {
    let _reset = theme::active::ResetOnDrop;
    let mut core = core_in(Language::PtBr);
    core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
        prefs: AppearancePrefs {
            theme_mode: ThemeMode::Light,
            text_scale: TextScale::P150,
            wallpaper: Wallpaper::Builtin,
            ..AppearancePrefs::default()
        },
        media: None,
    })))
    .unwrap();
    save(&mut core, date(2026, 10, 5), 40_000, "Aluguel");
    core.open_month(oct()).unwrap();
    let mut h = harness(core);
    assert_eq!(h.state().look.theme, Theme::Light);
    assert_eq!(h.state().look.text_scale, 150);
    assert!(
        h.state()
            .wallpaper
            .as_ref()
            .is_some_and(|rt| rt.texture_id(false).is_some())
    );
    h.get_by_label("Fechar mês Outubro 2026").click();
    h.run_steps(4);
    assert!(h.query_by_label("Fechar Outubro 2026?").is_some());
    h.get_by_label("Fechar mês").click();
    h.run_steps(6);
    assert!(h.query_by_label("Fechamento salvo").is_some());
    let saved = stored(&h).unwrap();
    save(
        &mut h.state_mut().core,
        date(2026, 10, 12),
        1_500,
        "Mercado",
    );
    h.run_steps(4);
    h.get_by_label("Recalcular fechamento Outubro 2026").focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert_eq!(stored(&h).unwrap(), saved);
    h.get_by_label("Recalcular").focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(6);
    assert_eq!(stored(&h).unwrap().expenses.cents(), 41_500);
    assert_eq!(
        h.state().core.vm.closing_status.get(&oct()),
        Some(&ClosingStatus::Current)
    );
}
