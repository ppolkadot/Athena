//! The year calendar: twelve compact month cards. Clicking a card opens that month's register.
//! It only reads the `ViewModel` (`calendar`, loaded for `UiState::calendar_year`).

use chrono::Datelike;
use eframe::egui::{
    self, Align2, Button, CornerRadius, Id, Rect, RichText, Role, Sense, Stroke, StrokeKind, Vec2,
    WidgetInfo,
};

use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::date::YearMonth;
use crate::domain::reports::CalendarMonthTotal;
use crate::i18n::{PluralKey, month_label, month_name, plural, strings};
use crate::ui::components::closing_card;
use crate::ui::components::day_grid;
use crate::ui::components::summary_card::{plain_brl, signed_brl};
use crate::ui::components::transaction_row::RowAction;
use crate::ui::icons::{self, Icon};
use crate::ui::theme::{palette, typography};

const CARD_HEIGHT: f32 = 156.0;
const DAYS_BUTTON_HEIGHT: f32 = 28.0;
const GAP: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarEvent {
    OpenMonth(YearMonth),
    Edit(i64),
    Delete(i64),
}

/// The Days button of a month card (also where Escape from that month's grid returns focus).
fn days_id(month: YearMonth) -> Id {
    Id::unique(("calendar-days", month))
}

/// How many cards fit per row: three on a normal window, two on a narrow one.
#[must_use]
pub fn columns_for(width: f32) -> usize {
    if width >= 720.0 { 3 } else { 2 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CardAction {
    None,
    Open,
    Days,
}

#[allow(clippy::too_many_lines)]
fn month_card(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    entry: &CalendarMonthTotal,
    width: f32,
    is_current: bool,
    state: &mut UiState,
) -> CardAction {
    let p = palette(vm.theme());
    let s = strings(vm.language());
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CARD_HEIGHT), Sense::click());
    let name = month_label(entry.month, vm.language());
    // A saved closing adds its status in words to the card's accessible name.
    let marker = closing_card::marker_parts(vm, entry.month);
    let accessible = marker.map_or_else(|| name.clone(), |(_, text, _)| format!("{name}, {text}"));
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &accessible));
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
    let title_font = typography::section_font();
    let title_color = if is_current { p.accent } else { p.text };
    painter.text(
        egui::pos2(left, rect.top() + 14.0),
        Align2::LEFT_TOP,
        title,
        title_font,
        title_color,
    );

    if let Some((icon, _, attention)) = marker {
        let icon_rect = Rect::from_min_size(
            egui::pos2(rect.right() - 16.0 - 18.0, rect.top() + 14.0),
            Vec2::splat(18.0),
        );
        icons::paint_with(
            painter,
            icon_rect,
            icon,
            if attention { p.warning } else { p.accent },
        );
    }

    let summary = &entry.summary;
    let money = typography::money_font();
    let small = typography::proportional(typography::SECONDARY);
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
        // A settlement-only month has no financial entries but is still activity.
        let text = if entry.activity_count > 0 {
            day_grid::activity_text(vm, entry.activity_count)
        } else {
            plural(PluralKey::Entries, 0, vm.language())
        };
        painter.text(
            egui::pos2(left, rect.top() + 52.0),
            Align2::LEFT_TOP,
            text,
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
    let opened = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked();

    if days_button(ui, vm, entry.month, rect, left, &name, state) {
        CardAction::Days
    } else if opened {
        CardAction::Open
    } else {
        CardAction::None
    }
}

/// The Days action sits on top of the card, so its click never opens the month.
fn days_button(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    month: YearMonth,
    card: Rect,
    left: f32,
    month_name: &str,
    state: &mut UiState,
) -> bool {
    let p = palette(vm.theme());
    let s = strings(vm.language());
    let id = days_id(month);
    let button = Rect::from_min_size(
        egui::pos2(left, card.bottom() - DAYS_BUTTON_HEIGHT - 10.0),
        Vec2::new(96.0, DAYS_BUTTON_HEIGHT),
    );
    let days = ui.interact(button, id, Sense::click());
    let name = s.days_of_month.replace("{month}", month_name);
    days.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
    day_grid::take_pending_focus(state, id, &days);
    let painter = ui.painter();
    painter.rect(
        button,
        CornerRadius::same(10),
        if days.hovered() { p.card_hover } else { p.card },
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    if days.has_focus() {
        painter.rect_stroke(
            button,
            CornerRadius::same(10),
            Stroke::new(2.0, p.focus),
            StrokeKind::Inside,
        );
    }
    painter.text(
        button.center(),
        Align2::CENTER_CENTER,
        s.days,
        typography::proportional(typography::SECONDARY),
        p.accent,
    );
    days.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// Draw the calendar. Returns what the user asked for, if anything.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) -> Option<CalendarEvent> {
    if let Some(month) = state.calendar_days {
        return days_view(ui, vm, state, month);
    }
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let mut event = None;

    ui.horizontal(|ui| {
        if icons::button(ui, &p, Icon::ChevronLeft, s.year_prev).clicked() {
            state.set_calendar_year(state.calendar_year - 1);
        }
        ui.label(
            RichText::new(state.calendar_year.to_string())
                .text_style(typography::display())
                .strong(),
        );
        if icons::button(ui, &p, Icon::ChevronRight, s.year_next).clicked() {
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

    if vm.calendar.iter().all(|m| m.activity_count == 0) {
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
                match month_card(ui, vm, entry, width, entry.month == current, state) {
                    CardAction::Open => event = Some(CalendarEvent::OpenMonth(entry.month)),
                    CardAction::Days => {
                        state.calendar_days = Some(entry.month);
                        state.clear_day_selection();
                    }
                    CardAction::None => {}
                }
            }
        });
    }
    event
}

/// One month's day grid inside the Calendar, with a way back to the year and on to the register.
fn days_view(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    state: &mut UiState,
    month: YearMonth,
) -> Option<CalendarEvent> {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let mut event = None;
    let mut back = false;
    ui.horizontal_wrapped(|ui| {
        back = ui
            .add(Button::new(RichText::new(s.back_to_year).color(p.accent)))
            .clicked();
        if ui
            .add(Button::new(RichText::new(s.open_month).color(p.accent)))
            .clicked()
        {
            event = Some(CalendarEvent::OpenMonth(month));
        }
    });
    ui.add_space(8.0);
    ui.label(
        RichText::new(
            s.days_of_month
                .replace("{month}", &month_label(month, vm.language())),
        )
        .text_style(typography::display())
        .strong(),
    );
    if let Some((icon, text, attention)) = closing_card::marker_parts(vm, month) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            icons::paint(ui, rect, icon, if attention { p.warning } else { p.accent });
            let marker = ui
                .add(
                    Button::new(RichText::new(text).color(if attention {
                        p.warning
                    } else {
                        p.accent
                    }))
                    .frame(false),
                )
                .on_hover_text(s.open_month);
            marker.widget_info(|| {
                WidgetInfo::labeled(Role::Button, true, format!("{text}, {}", s.open_month))
            });
            if marker.clicked() {
                event = Some(CalendarEvent::OpenMonth(month));
            }
        });
    }
    ui.add_space(8.0);
    let out = day_grid::show(ui, vm, state, days_id(month));
    match out.row {
        Some(RowAction::Edit(id)) => event = Some(CalendarEvent::Edit(id)),
        Some(RowAction::Delete(id)) => event = Some(CalendarEvent::Delete(id)),
        None => {}
    }
    if back || out.left {
        state.calendar_days = None;
        state.clear_day_selection();
        if back {
            state.pending_focus = Some(days_id(month));
        }
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
