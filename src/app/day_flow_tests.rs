//! Day drill-down and the monthly insight, driven through the whole app (contract §14.1).

use eframe::egui::{self, Id, Key};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::actions::AppAction;
use super::navigation::Screen;
use super::{AppCore, AthenaApp};
use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::settings::{Language, Profile, Theme};
use crate::domain::transaction::TransactionType;
use crate::infrastructure::db::Db;
use crate::shared::test_fixtures::{date, draft_with, seed_month, today};
use crate::ui::theme;

fn core() -> AppCore {
    let db = Db::open_in_memory().unwrap();
    seed_month(&db);
    let mut core = AppCore::empty_for_test(db);
    core.save_profile(Profile {
        start_balance: Money::ZERO,
        theme: Theme::Dark,
        language: Language::En,
        onboarding_complete: true,
    })
    .unwrap();
    core
}

fn harness_sized(core: AppCore, size: egui::Vec2) -> Harness<'static, AthenaApp> {
    let mut app = AthenaApp::for_test(core);
    app.pin_today(today());
    let mut harness = Harness::builder()
        .with_size(size)
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

fn harness(core: AppCore) -> Harness<'static, AthenaApp> {
    harness_sized(core, egui::Vec2::new(1280.0, 1300.0))
}

fn cell_label(h: &Harness<'static, AthenaApp>, day: u32, activity: &str, amount: &str) -> String {
    let s = crate::i18n::strings(h.state().core.vm.language());
    s.day_cell_label
        .replace(
            "{date}",
            &crate::i18n::format_date(date(2026, 11, day), h.state().core.vm.language()),
        )
        .replace("{activity}", activity)
        .replace("{amount}", amount)
}

fn click_cell(h: &mut Harness<'static, AthenaApp>, day: u32, activity: &str, amount: &str) {
    let label = cell_label(h, day, activity, amount);
    h.get_by_label(&label).click();
    h.run_steps(4);
}

fn focused(h: &Harness<'static, AthenaApp>, id: Id) -> bool {
    h.ctx.memory(|m| m.has_focus(id))
}

#[test]
fn month_days_grid_shows_activity_and_selecting_lists_that_days_rows() {
    let mut h = harness(core());
    h.get_by_label("Days of November 2026").click();
    h.run_steps(4);
    assert!(h.state().core.ui.month_days);

    // Salary and Market share the 5th: two activities, spending from the expense only. The 8th
    // holds a refund alone, so its spending is negative and its count still shows.
    assert!(
        h.query_by_label(&cell_label(&h, 5, "2 activities", "R$ 50,00"))
            .is_some()
    );
    assert!(
        h.query_by_label(&cell_label(&h, 8, "1 activity", "−R$ 20,00"))
            .is_some()
    );
    assert!(
        h.query_by_label(&cell_label(&h, 2, "No activity", "R$ 0,00"))
            .is_some()
    );
    // Monday-first localized weekday headers.
    assert!(h.query_by_label("Mon").is_some() && h.query_by_label("Sun").is_some());

    click_cell(&mut h, 5, "2 activities", "R$ 50,00");
    assert_eq!(h.state().core.ui.selected_day, Some(date(2026, 11, 5)));
    // Each row shows twice: in the register and in the day detail.
    assert_eq!(h.query_all_by_label("Edit Market").count(), 2);
    assert_eq!(h.query_all_by_label("Edit Salary").count(), 2);
    assert_eq!(h.query_all_by_label("Edit Rent").count(), 1);
    assert!(h.query_by_label("2 activities").is_some());

    // An empty day is selectable and says so.
    click_cell(&mut h, 2, "No activity", "R$ 0,00");
    assert!(h.query_by_label("Nothing recorded on this day.").is_some());
    assert_eq!(h.query_all_by_label("Edit Market").count(), 1);

    // Leaving the month drops the selection.
    h.get_by_label("Next month").click();
    h.run_steps(4);
    assert_eq!(h.state().core.ui.selected_day, None);
}

#[test]
fn grid_keyboard_moves_within_the_month_selects_and_escape_returns_to_the_opener() {
    let mut h = harness(core());
    h.get_by_label("Days of November 2026").click();
    h.run_steps(4);
    click_cell(&mut h, 1, "1 activity", "R$ 100,00");
    let cursor = |h: &Harness<'static, AthenaApp>| h.state().core.ui.day_cursor;
    assert_eq!(cursor(&h), Some(date(2026, 11, 1)));
    assert!(focused(
        &h,
        crate::ui::components::day_grid::cell_id(date(2026, 11, 1))
    ));

    // Arrows move one day or one week, never out of the month.
    h.key_press(Key::ArrowLeft);
    h.run_steps(3);
    assert_eq!(cursor(&h), Some(date(2026, 11, 1)), "stays inside November");
    h.key_press(Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(cursor(&h), Some(date(2026, 11, 1)));
    h.key_press(Key::ArrowRight);
    h.run_steps(3);
    assert_eq!(cursor(&h), Some(date(2026, 11, 2)));
    h.key_press(Key::ArrowDown);
    h.run_steps(3);
    assert_eq!(cursor(&h), Some(date(2026, 11, 9)));
    assert!(focused(
        &h,
        crate::ui::components::day_grid::cell_id(date(2026, 11, 9))
    ));

    // Enter selects the focused day.
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().core.ui.selected_day, Some(date(2026, 11, 9)));

    // Escape sends focus back to the Days toggle without closing the month or losing the day.
    h.key_press(Key::Escape);
    h.run_steps(4);
    assert!(focused(&h, Id::unique("month-days-toggle")));
    assert_eq!(h.state().core.ui.screen, Screen::Month);
}

#[test]
fn calendar_days_action_opens_a_month_grid_with_back_and_open_month() {
    let mut core = core();
    core.set_screen(Screen::Calendar);
    let mut h = harness(core);
    h.get_by_label("Days of November 2026").click();
    h.run_steps(4);
    assert_eq!(
        h.state().core.ui.calendar_days,
        YearMonth::new(2026, 11),
        "Days opens the grid inside Calendar"
    );
    assert!(h.query_by_label("Back to year").is_some());
    click_cell(&mut h, 8, "1 activity", "−R$ 20,00");
    assert_eq!(h.query_all_by_label("Edit Refund").count(), 1);

    h.get_by_label("Back to year").click();
    h.run_steps(4);
    assert_eq!(h.state().core.ui.calendar_days, None);
    assert_eq!(h.state().core.ui.selected_day, None);
    assert!(
        focused(
            &h,
            Id::unique(("calendar-days", YearMonth::new(2026, 11).unwrap()))
        ),
        "focus returns to the card's Days action"
    );

    h.get_by_label("Days of November 2026").click();
    h.run_steps(4);
    h.get_by_label("Open month").click();
    h.run_steps(4);
    assert_eq!(h.state().core.ui.screen, Screen::Month);
    assert_eq!(h.state().core.ui.month, YearMonth::new(2026, 11).unwrap());
    assert_eq!(h.state().core.ui.calendar_days, None);
}

#[test]
fn calendar_escape_leaves_the_grid_for_the_year() {
    let mut core = core();
    core.set_screen(Screen::Calendar);
    let mut h = harness(core);
    h.get_by_label("Days of November 2026").click();
    h.run_steps(4);
    click_cell(&mut h, 1, "1 activity", "R$ 100,00");
    h.key_press(Key::Escape);
    h.run_steps(4);
    assert_eq!(h.state().core.ui.calendar_days, None);
    assert!(focused(
        &h,
        Id::unique(("calendar-days", YearMonth::new(2026, 11).unwrap()))
    ));
}

#[test]
fn a_vanished_row_leaves_focus_on_the_day_heading() {
    let mut h = harness(core());
    h.get_by_label("Days of November 2026").click();
    h.run_steps(4);
    click_cell(&mut h, 5, "2 activities", "R$ 50,00");
    let market = h
        .state()
        .core
        .vm
        .day_rows
        .iter()
        .find(|t| t.title == "Market")
        .unwrap()
        .id;
    h.state_mut()
        .core
        .apply(AppAction::DeleteTransaction(market))
        .unwrap();
    h.run_steps(6);
    assert_eq!(h.query_all_by_label("Edit Market").count(), 0);
    // The register's day group and the detail heading share this text; the heading has focus.
    assert!(
        h.get_all_by_label("Thu  2026-11-05")
            .any(|node| node.is_focused()),
        "focus lands on the day heading"
    );
}

#[test]
fn midnight_rolls_the_insight_heading_and_planned_months_say_so() {
    let mut h = harness(core());
    // 11 Nov is inside November: month to date, with the prior month as a separate line.
    assert!(h.query_by_label("Month to date").is_some());
    assert!(h.query_by_label("Net spending R$ 130,00").is_some());
    assert!(
        h.query_by_label("Month to date R$ 130,00 · all of October 2026 R$ 0,00")
            .is_some()
    );
    h.get_by_label("Next month").click();
    h.run_steps(4);
    assert!(h.query_by_label("Planned").is_some());
    assert!(
        h.query_by_label("No income, expenses, or refunds")
            .is_some()
    );
    h.get_by_label("Previous month").click();
    h.get_by_label("Previous month").click();
    h.run_steps(4);
    // October is complete and empty: no heading, the full "this month" wording.
    assert!(h.query_by_label("Month to date").is_none());
    assert!(
        h.query_by_label("No income, expenses, or refunds this month")
            .is_some()
    );
}

#[test]
fn insight_names_follow_a_rename_and_the_language() {
    let mut h = harness(core());
    assert!(h.query_by_label_contains("Top category: Housing").is_some());
    for c in &mut h.state_mut().core.vm.all_categories {
        if c.name == "Housing" {
            c.name = "Home".into();
        }
    }
    h.run_steps(3);
    assert!(h.query_by_label_contains("Top category: Home").is_some());

    h.state_mut().core.vm.profile = Some(Profile {
        start_balance: Money::ZERO,
        theme: Theme::Light,
        language: Language::PtBr,
        onboarding_complete: true,
    });
    h.run_steps(3);
    assert!(h.query_by_label_contains("Maior categoria: Home").is_some());
    assert!(h.query_by_label("Mês até hoje").is_some());
}

#[test]
fn settlement_only_days_and_months_are_activity() {
    let mut core = core();
    let (owner, partner) = (core.vm.all_people[0].id, core.vm.all_people[1].id);
    let mut settle = draft_with(vec![crate::domain::transaction::Allocation {
        person_id: partner,
        amount: Money::from_cents(2_500),
    }]);
    settle.kind = TransactionType::Settlement;
    settle.payer_id = Some(owner);
    settle.split_mode = None;
    settle.title = "Payback".into();
    settle.date = date(2026, 3, 12);
    core.apply(AppAction::SaveTransaction(settle)).unwrap();
    core.set_screen(Screen::Calendar);
    let mut h = harness(core);
    // March has only a settlement: no financial entries, but the year is not empty.
    let march = &h.state().core.vm.calendar[2];
    assert_eq!((march.summary.count, march.activity_count), (0, 1));
    assert!(
        h.query_by_label_contains("Nothing logged in 2026")
            .is_none()
    );
    h.get_by_label("Days of March 2026").click();
    h.run_steps(4);
    let label = crate::i18n::strings(Language::En)
        .day_cell_label
        .replace("{date}", "2026-03-12")
        .replace("{activity}", "1 activity")
        .replace("{amount}", "R$ 0,00");
    h.get_by_label(&label).click();
    h.run_steps(4);
    assert_eq!(h.query_all_by_label("Edit Payback").count(), 1);
}

#[test]
fn day_detail_and_insight_fit_the_narrow_window() {
    let mut core = core();
    core.ui.month_days = true;
    core.ui.select_day(date(2026, 11, 5));
    let mut h = harness_sized(core, egui::Vec2::new(640.0, 410.0));
    h.run_steps(4);
    assert!(h.query_by_label("Month to date").is_some());
    assert!(h.query_by_label("Mon").is_some());
}
