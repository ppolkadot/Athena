//! The year calendar: twelve compact month cards. Clicking a card opens that month's register.
//! It only reads the `ViewModel` (`calendar`, loaded for `UiState::calendar_year`).

use chrono::Datelike;
use eframe::egui::{
    self, Align2, Button, CornerRadius, FontId, RichText, Role, Sense, Stroke, StrokeKind, Vec2,
    WidgetInfo,
};

use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::date::YearMonth;
use crate::domain::reports::CalendarMonthTotal;
use crate::i18n::{PluralKey, month_label, month_name, plural, strings};
use crate::ui::components::summary_card::{plain_brl, signed_brl};
use crate::ui::theme::{palette, typography};

const CARD_HEIGHT: f32 = 120.0;
const GAP: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarEvent {
    OpenMonth(YearMonth),
}

/// How many cards fit per row: three on a normal window, two on a narrow one.
#[must_use]
pub fn columns_for(width: f32) -> usize {
    if width >= 720.0 { 3 } else { 2 }
}

fn month_card(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    entry: &CalendarMonthTotal,
    width: f32,
    is_current: bool,
) -> bool {
    let p = palette(vm.theme());
    let s = strings(vm.language());
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CARD_HEIGHT), Sense::click());
    let name = month_label(entry.month, vm.language());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
    let hovered = response.hovered();
    let fill = if hovered { p.card_hover } else { p.card };
    let stroke = if is_current {
        Stroke::new(2.0, p.accent)
    } else if hovered {
        Stroke::new(1.0, p.accent)
    } else {
        Stroke::new(1.0, p.border)
    };
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(16),
        fill,
        stroke,
        StrokeKind::Inside,
    );

    let left = rect.left() + 16.0;
    let title = month_name(vm.language(), entry.month.month);
    let title_font = FontId::new(typography::SECTION, egui::FontFamily::Proportional);
    let title_color = if is_current { p.accent } else { p.text };
    painter.text(
        egui::pos2(left, rect.top() + 14.0),
        Align2::LEFT_TOP,
        title,
        title_font,
        title_color,
    );

    let summary = &entry.summary;
    let money = typography::money_font();
    let small = FontId::new(typography::SECONDARY, egui::FontFamily::Proportional);
    let right = rect.right() - 16.0;
    let row = |y: f32, label: &str, value: String, color: egui::Color32| {
        painter.text(
            egui::pos2(left, y),
            Align2::LEFT_TOP,
            label,
            small.clone(),
            p.muted,
        );
        painter.text(
            egui::pos2(right, y),
            Align2::RIGHT_TOP,
            value,
            money.clone(),
            color,
        );
    };
    if summary.count == 0 {
        painter.text(
            egui::pos2(left, rect.top() + 52.0),
            Align2::LEFT_TOP,
            plural(PluralKey::Entries, 0, vm.language()),
            small,
            p.muted,
        );
    } else {
        row(
            rect.top() + 48.0,
            s.expenses,
            plain_brl(summary.spending()),
            p.text,
        );
        row(
            rect.top() + 70.0,
            s.income_total,
            signed_brl(summary.income),
            p.positive,
        );
        painter.text(
            egui::pos2(left, rect.top() + 94.0),
            Align2::LEFT_TOP,
            plural(
                PluralKey::Entries,
                usize::try_from(summary.count).unwrap_or(0),
                vm.language(),
            ),
            small,
            p.muted,
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// Draw the calendar. Returns the month the user opened, if any.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) -> Option<CalendarEvent> {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let mut event = None;

    ui.horizontal(|ui| {
        let step = |ui: &mut egui::Ui, glyph: &str, name: &str| {
            let response = ui.add_sized([36.0, 36.0], Button::new(glyph));
            response.widget_info(|| WidgetInfo::labeled(Role::Button, true, name));
            response.on_hover_text(name).clicked()
        };
        if step(ui, "‹", s.year_prev) {
            state.set_calendar_year(state.calendar_year - 1);
        }
        ui.label(
            RichText::new(state.calendar_year.to_string())
                .text_style(typography::display())
                .strong(),
        );
        if step(ui, "›", s.year_next) {
            state.set_calendar_year(state.calendar_year + 1);
        }
        let this_year = vm.today.year();
        if state.calendar_year != this_year
            && ui
                .add(Button::new(RichText::new(s.this_year).color(p.accent)).frame(false))
                .clicked()
        {
            state.set_calendar_year(this_year);
        }
    });
    ui.add_space(12.0);

    if vm.calendar.iter().all(|m| m.summary.count == 0) {
        ui.label(
            RichText::new(
                s.empty_year
                    .replace("{year}", &state.calendar_year.to_string()),
            )
            .color(p.muted),
        );
        ui.add_space(8.0);
    }

    let current = YearMonth::from_date(vm.today);
    let columns = columns_for(ui.available_width());
    #[allow(clippy::cast_precision_loss)]
    let width = (ui.available_width() - GAP * (columns as f32 - 1.0)) / columns as f32;
    ui.spacing_mut().item_spacing = Vec2::splat(GAP);
    for row in vm.calendar.chunks(columns) {
        ui.horizontal(|ui| {
            for entry in row {
                if month_card(ui, vm, entry, width, entry.month == current) {
                    event = Some(CalendarEvent::OpenMonth(entry.month));
                }
            }
        });
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppCore;
    use crate::domain::settings::Language;
    use crate::infrastructure::db::Db;
    use crate::shared::test_fixtures::seed_month;

    #[test]
    fn card_columns_adapt_to_the_window() {
        assert_eq!(columns_for(1100.0), 3);
        assert_eq!(columns_for(720.0), 3);
        assert_eq!(columns_for(600.0), 2);
    }

    type CalApp = (ViewModel, UiState, Option<CalendarEvent>);

    fn harness(core: &AppCore) -> egui_kittest::Harness<'static, CalApp> {
        egui_kittest::Harness::builder()
            .with_size(Vec2::new(1100.0, 700.0))
            .build_ui_state(
                |ui, (vm, state, event): &mut CalApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    crate::ui::theme::apply(ui.ctx(), vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    if let Some(e) = show(ui, vm, state) {
                        *event = Some(e);
                    }
                },
                (core.vm.clone(), core.ui.clone(), None),
            )
    }

    fn core_with_data() -> AppCore {
        let db = Db::open_in_memory().unwrap();
        seed_month(&db);
        AppCore::empty_for_test(db)
    }

    #[test]
    fn calendar_has_twelve_zero_filled_months() {
        let core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        assert_eq!(core.vm.calendar.len(), 12);
        assert!(core.vm.calendar.iter().all(|m| m.summary.count == 0));
        assert_eq!(core.vm.calendar[0].month, YearMonth::new(2026, 1).unwrap());
    }

    #[test]
    fn twelve_cards_are_shown_and_a_click_opens_that_month() {
        use egui_kittest::kittest::Queryable;
        let core = core_with_data();
        let mut harness = harness(&core);
        harness.run_steps(3);
        for month in 1..=12 {
            let label = month_label(YearMonth::new(2026, month).unwrap(), Language::En);
            assert!(
                harness.query_by_label(&label).is_some(),
                "{label} has a card"
            );
        }
        assert!(harness.query_by_label("2026").is_some());
        harness.get_by_label("November 2026").click();
        harness.run_steps(3);
        assert_eq!(
            harness.state().2,
            Some(CalendarEvent::OpenMonth(YearMonth::new(2026, 11).unwrap()))
        );
    }

    #[test]
    fn the_year_stepper_moves_the_calendar_year() {
        use egui_kittest::kittest::Queryable;
        let core = core_with_data();
        let mut harness = harness(&core);
        harness.run_steps(3);
        harness.get_by_label("Next year").click();
        harness.run_steps(3);
        assert_eq!(harness.state().1.calendar_year, 2027);
        harness.get_by_label("Previous year").click();
        harness.get_by_label("Previous year").click();
        harness.run_steps(3);
        assert_eq!(harness.state().1.calendar_year, 2025);
        assert_eq!(
            harness.state().2,
            None,
            "stepping years never opens a month"
        );
    }

    #[test]
    fn an_empty_year_shows_a_quiet_hint_not_a_broken_grid() {
        use egui_kittest::kittest::Queryable;
        let core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        let mut harness = harness(&core);
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label("Nothing logged in 2026 yet. Pick a month to start.")
                .is_some()
        );
        assert!(
            harness.query_by_label("June 2026").is_some(),
            "the twelve cards are still there"
        );
    }

    #[test]
    fn labels_follow_the_language() {
        use egui_kittest::kittest::Queryable;
        let mut core = core_with_data();
        core.vm.profile = Some(crate::domain::settings::Profile {
            start_balance: crate::domain::money::Money::ZERO,
            theme: crate::domain::settings::Theme::Light,
            language: Language::PtBr,
            onboarding_complete: true,
        });
        let mut harness = harness(&core);
        harness.run_steps(3);
        assert!(harness.query_by_label("Novembro 2026").is_some());
        assert!(harness.query_by_label("Ano seguinte").is_some());
    }
}
