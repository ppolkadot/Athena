//! The day grid and its detail panel, shared by Calendar (one month opened from its card) and the
//! Month screen (a Days toggle above the register). It only reads the `ViewModel` and edits the
//! day fields of `UiState`.
//!
//! Heat shows positive net spending only; a separate dot and count show activity, so a salary-only
//! or net-zero day is still discoverable. Only the cursor cell takes keyboard focus (Tab enters and
//! leaves the grid, arrows move within the month), and every cell has a full accessible name.

use chrono::{Datelike, NaiveDate, Weekday};
use eframe::egui::{
    self, Align2, CornerRadius, EventFilter, Frame, Id, Key, Label, Margin, Modifiers, Rect,
    RichText, Role, Sense, Stroke, StrokeKind, Vec2, WidgetInfo,
};

use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::date::{YearMonth, month_grid};
use crate::domain::money::Money;
use crate::domain::reports::{DayActivity, bar_fraction, zero_filled_days};
use crate::i18n::{format_date, strings, weekday_short};
use crate::ui::components::summary_card::plain_brl;
use crate::ui::components::transaction_row::{self, RowAction};
use crate::ui::theme::{Palette, composite, palette, typography};
use crate::ui::tokens;

/// Side by side from this width; the detail drops below the grid under it.
const WIDE: f32 = 880.0;
const GRID_WIDTH: f32 = 480.0;
const CELL_HEIGHT: f32 = 52.0;
const GAP: f32 = 4.0;
/// Heat never exceeds this alpha, so the day number keeps its contrast on every cell.
const MAX_HEAT: f32 = 64.0;

/// What the grid or detail asked for.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DayOutput {
    /// An Edit or Delete on a day row.
    pub row: Option<RowAction>,
    /// Escape left the grid: the caller closes it if it can, focus is already on the opener.
    pub left: bool,
}

/// Stable id of a grid cell.
#[must_use]
pub fn cell_id(date: NaiveDate) -> Id {
    Id::unique(("day-cell", date))
}

/// Give `response` keyboard focus if `id` is the widget `state.pending_focus` is waiting for.
pub fn take_pending_focus(state: &mut UiState, id: Id, response: &egui::Response) {
    if state.pending_focus == Some(id) {
        response.request_focus();
        state.pending_focus = None;
    }
}

/// The Days toggle of the Month screen: a labeled, focusable button that shows its state.
pub fn toggle(
    ui: &mut egui::Ui,
    p: &Palette,
    id: Id,
    text: &str,
    name: &str,
    on: bool,
) -> egui::Response {
    let font = typography::tabular_font(typography::BODY);
    let width = ui.fonts_mut(|f| {
        f.layout_no_wrap(text.to_owned(), font.clone(), p.text)
            .size()
            .x
    }) + 2.0 * tokens::space_l();
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(width, tokens::control_height()), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, on, name));
    let radius = CornerRadius::same(tokens::CONTROL_RADIUS);
    let fill = if on {
        tokens::selected_fill(p, p.card)
    } else if response.hovered() {
        p.card_hover
    } else {
        p.card
    };
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(tokens::BORDER_WIDTH, if on { p.accent } else { p.border }),
        StrokeKind::Inside,
    );
    if response.has_focus() {
        painter.rect_stroke(rect, radius, Stroke::new(2.0, p.focus), StrokeKind::Inside);
    }
    painter.text(rect.center(), Align2::CENTER_CENTER, text, font, p.text);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn weekday_at(index: usize) -> Weekday {
    let mut day = Weekday::Mon;
    for _ in 0..index {
        day = day.succ();
    }
    day
}

/// The grid cell the keyboard cursor sits on: the stored cursor if it is in the month, else today,
/// else the 1st.
fn cursor_in(state: &UiState, month: YearMonth, today: NaiveDate) -> NaiveDate {
    state
        .day_cursor
        .or(state.selected_day)
        .filter(|d| month.contains(*d))
        .or_else(|| month.contains(today).then_some(today))
        .unwrap_or_else(|| month.first_day())
}

/// Draw the grid and, once a date is selected, its detail. `opener` is the widget that opened the
/// grid; Escape sends focus back to it.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState, opener: Id) -> DayOutput {
    let mut out = DayOutput::default();
    let Some(month) = state.grid_month() else {
        return out;
    };
    let p = palette(vm.theme());
    let s = strings(vm.language());
    if vm.refresh_failed {
        ui.label(RichText::new(s.day_detail_unavailable).color(p.muted));
        return out;
    }
    ui.label(
        RichText::new(s.day_grid_hint)
            .text_style(egui::TextStyle::Small)
            .color(p.muted),
    );
    ui.add_space(tokens::space_s());
    if ui.available_width() >= WIDE {
        let total = ui.available_width();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(GRID_WIDTH, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(GRID_WIDTH);
                    grid(ui, vm, state, month, opener, &mut out);
                },
            );
            let rest = total - GRID_WIDTH - tokens::space_l();
            ui.allocate_ui_with_layout(
                Vec2::new(rest, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(rest);
                    detail(ui, vm, state, &mut out);
                },
            );
        });
    } else {
        grid(ui, vm, state, month, opener, &mut out);
        ui.add_space(tokens::space_m());
        detail(ui, vm, state, &mut out);
    }
    out
}

#[must_use]
pub fn activity_text(vm: &ViewModel, count: i64) -> String {
    let s = strings(vm.language());
    match count {
        0 => s.day_activity_none.to_string(),
        1 => s.day_activity_one.to_string(),
        n => s.day_activity_many.replace("{count}", &n.to_string()),
    }
}

fn grid(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    state: &mut UiState,
    month: YearMonth,
    opener: Id,
    out: &mut DayOutput,
) {
    let p = palette(vm.theme());
    let days = zero_filled_days(month, &vm.day_activity);
    let max = days
        .iter()
        .map(|d| d.spending)
        .filter(|m| m.cents() > 0)
        .max()
        .unwrap_or(Money::ZERO);
    let cell_w = ((ui.available_width() - GAP * 6.0) / 7.0).floor().max(36.0);
    let size = Vec2::new(cell_w, CELL_HEIGHT);
    let spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing = Vec2::splat(GAP);

    ui.horizontal(|ui| {
        for index in 0..7 {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(cell_w, 20.0), Sense::hover());
            ui.put(
                rect,
                Label::new(
                    RichText::new(weekday_short(vm.language(), weekday_at(index)))
                        .font(typography::tabular_font(typography::SECONDARY))
                        .color(p.muted),
                ),
            );
        }
    });
    let cells = month_grid(month.year, month.month);
    for week in cells.chunks(7) {
        ui.horizontal(|ui| {
            for cell in week {
                match cell {
                    None => {
                        ui.allocate_space(size);
                    }
                    Some(date) => {
                        let activity = &days[date.day0() as usize];
                        day_cell(ui, vm, state, &p, activity, max, size, month, opener, out);
                    }
                }
            }
        });
    }
    ui.spacing_mut().item_spacing = spacing;
}

#[allow(clippy::too_many_arguments)]
fn day_cell(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    state: &mut UiState,
    p: &Palette,
    activity: &DayActivity,
    max: Money,
    size: Vec2,
    month: YearMonth,
    opener: Id,
    out: &mut DayOutput,
) {
    let s = strings(vm.language());
    let date = activity.date;
    let id = cell_id(date);
    let is_cursor = date == cursor_in(state, month, vm.today);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let sense = if is_cursor {
        Sense::click()
    } else {
        Sense::CLICK
    };
    let response = ui.interact(rect, id, sense);
    let selected = state.selected_day == Some(date);
    let label = s
        .day_cell_label
        .replace("{date}", &format_date(date, vm.language()))
        .replace("{activity}", &activity_text(vm, activity.activity_count))
        .replace("{amount}", &plain_brl(activity.spending));
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &label));

    if is_cursor {
        take_pending_focus(state, id, &response);
    }
    if response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                    ..EventFilter::default()
                },
            );
        });
        navigate(ui, state, month, date, opener, out);
    }
    if response.clicked() {
        state.select_day(date);
        response.request_focus();
    }

    if ui.is_rect_visible(rect) {
        paint_cell(ui, p, rect, activity, max, selected, &response);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand);
}

/// Arrow keys move the cursor one day (left/right) or one week (up/down) inside the month, and
/// Escape hands focus back to the opener.
fn navigate(
    ui: &egui::Ui,
    state: &mut UiState,
    month: YearMonth,
    from: NaiveDate,
    opener: Id,
    out: &mut DayOutput,
) {
    let delta = ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::ArrowLeft) {
            Some(-1)
        } else if i.consume_key(Modifiers::NONE, Key::ArrowRight) {
            Some(1)
        } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            Some(-7)
        } else if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            Some(7)
        } else {
            None
        }
    });
    if let Some(delta) = delta {
        let target = from + chrono::Duration::days(delta);
        if month.contains(target) {
            state.day_cursor = Some(target);
            state.pending_focus = Some(cell_id(target));
            ui.ctx().request_repaint();
        }
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        state.pending_focus = Some(opener);
        out.left = true;
        ui.ctx().request_repaint();
    }
}

fn paint_cell(
    ui: &egui::Ui,
    p: &Palette,
    rect: Rect,
    activity: &DayActivity,
    max: Money,
    selected: bool,
    response: &egui::Response,
) {
    let painter = ui.painter();
    let radius = CornerRadius::same(tokens::CONTROL_RADIUS);
    let base = if response.hovered() {
        p.card_hover
    } else {
        p.card
    };
    painter.rect_filled(rect, radius, base);
    if activity.spending.cents() > 0 {
        let fraction = bar_fraction(activity.spending, max);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let alpha = (16.0 + (MAX_HEAT - 16.0) * fraction)
            .round()
            .clamp(0.0, 255.0) as u8;
        painter.rect_filled(rect, radius, composite(p.negative, alpha, base));
    }
    let border = if selected {
        Stroke::new(2.0, p.accent)
    } else {
        Stroke::new(tokens::BORDER_WIDTH, p.border)
    };
    painter.rect_stroke(rect, radius, border, StrokeKind::Inside);
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(2.0),
            radius,
            Stroke::new(2.0, p.focus),
            StrokeKind::Inside,
        );
    }
    painter.text(
        rect.left_top() + Vec2::new(8.0, 6.0),
        Align2::LEFT_TOP,
        activity.date.day().to_string(),
        typography::tabular_font(typography::BODY),
        p.text,
    );
    if activity.activity_count > 0 {
        let y = rect.bottom() - 12.0;
        painter.circle_filled(egui::pos2(rect.left() + 12.0, y), 3.0, p.accent);
        painter.text(
            egui::pos2(rect.left() + 20.0, y),
            Align2::LEFT_CENTER,
            activity.activity_count.to_string(),
            typography::tabular_font(typography::SECONDARY),
            p.muted,
        );
    }
}

fn detail(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState, out: &mut DayOutput) {
    let Some(day) = state.selected_day else {
        return;
    };
    let p = palette(vm.theme());
    let s = strings(vm.language());
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(tokens::BORDER_WIDTH, p.border))
        .corner_radius(CornerRadius::same(tokens::CARD_RADIUS))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let title = s.day_detail_title.replace(
                "{date}",
                &format!(
                    "{}  {}",
                    weekday_short(vm.language(), day.weekday()),
                    format_date(day, vm.language())
                ),
            );
            let heading = ui.add(
                Label::new(
                    RichText::new(title)
                        .text_style(typography::section())
                        .strong(),
                )
                .sense(Sense::focusable_noninteractive()),
            );
            let rows: Vec<_> = vm.day_rows.iter().filter(|t| t.date == day).collect();

            // A row that vanished (deleted) leaves focus on the heading, not on nothing.
            let memory = Id::unique("day-detail-rows");
            let previous: Option<(NaiveDate, usize)> = ui.data(|d| d.get_temp(memory));
            ui.data_mut(|d| d.insert_temp(memory, (day, rows.len())));
            let vanished = previous.is_some_and(|(d, n)| d == day && n > rows.len());
            if vanished || state.day_heading_focus {
                heading.request_focus();
                state.day_heading_focus = false;
            }

            ui.label(
                RichText::new(activity_text(
                    vm,
                    i64::try_from(rows.len()).unwrap_or(i64::MAX),
                ))
                .color(p.muted),
            );
            ui.label(
                RichText::new(s.day_detail_scope)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
            ui.add_space(tokens::space_s());
            if rows.is_empty() {
                ui.label(RichText::new(s.day_detail_empty).color(p.muted));
                return;
            }
            for (i, tx) in rows.iter().enumerate() {
                if i > 0 {
                    ui.separator();
                }
                if let Some(action) = transaction_row::show(ui, vm, tx) {
                    out.row = Some(action);
                }
            }
        });
}
