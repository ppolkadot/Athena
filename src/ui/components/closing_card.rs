//! The saved-closing surface of a month: a distinct card below the live totals that shows what was
//! saved, whether the ledger changed since, and the Close month / Recalculate closing actions. It
//! reads the `ViewModel` only. Saved figures never change until the user confirms a recalculation.

use eframe::egui::{
    self, Align, Button, CornerRadius, Frame, Label, Layout, Margin, RichText, Role, Sense, Stroke,
    Vec2, WidgetInfo,
};

use crate::app::view_model::ViewModel;
use crate::domain::closing::{ClosingSnapshot, ClosingStatus, is_completed};
use crate::domain::date::YearMonth;
use crate::i18n::{PluralKey, format_date, insight_text, month_label, plural, strings};
use crate::ui::components::summary_card::{plain_brl, signed_brl};
use crate::ui::icons::{self, Icon};
use crate::ui::theme::{Palette, palette, typography};
use crate::ui::tokens;

/// What the card asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosingEvent {
    /// Ask to close the month (the app confirms first).
    Close(YearMonth),
    /// Ask to replace the saved closing (the app confirms first).
    Recalculate(YearMonth),
    /// Open the saved largest expense, which is still a live entry.
    OpenEntry(i64),
    /// Run the staleness check again.
    RetryCheck,
}

/// The localized, screen-reader-ready description of a closing marker.
#[must_use]
pub fn marker_label(vm: &ViewModel, month: YearMonth) -> Option<String> {
    vm.closings.get(&month)?;
    let s = strings(vm.language());
    let template = match vm.closing_status.get(&month) {
        Some(ClosingStatus::Changed) => s.closing_marker_changed,
        Some(ClosingStatus::Unavailable) => s.closing_marker_unavailable,
        _ => s.closing_marker_current,
    };
    Some(template.replace("{month}", &month_label(month, vm.language())))
}

/// The icon and short text of a marker. Shape and words carry the status, never color alone.
#[must_use]
pub fn marker_parts(vm: &ViewModel, month: YearMonth) -> Option<(Icon, &'static str, bool)> {
    vm.closings.get(&month)?;
    let s = strings(vm.language());
    Some(match vm.closing_status.get(&month) {
        Some(ClosingStatus::Changed) => (Icon::TriangleAlert, s.closing_changed, true),
        Some(ClosingStatus::Unavailable) => {
            (Icon::TriangleAlert, s.closing_check_unavailable, true)
        }
        _ => (Icon::Bookmark, s.closing_marker_text_current, false),
    })
}

fn figure(ui: &mut egui::Ui, p: &Palette, label: &str, value: &str, color: egui::Color32) {
    // The row is as tall as one line of text, whatever the layout direction would claim.
    let height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), height),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(
                RichText::new(label)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(value)
                        .font(typography::tabular_font(typography::BODY))
                        .color(color),
                );
            });
        },
    );
}

fn status_line(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    p: &Palette,
    month: YearMonth,
    event: &mut Option<ClosingEvent>,
) {
    let s = strings(vm.language());
    match vm.closing_status.get(&month) {
        Some(ClosingStatus::Changed) => {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                icons::paint(ui, rect, Icon::TriangleAlert, p.warning);
                ui.label(RichText::new(s.closing_changed).color(p.warning).strong());
            });
        }
        Some(ClosingStatus::Unavailable) => {
            ui.horizontal_wrapped(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                icons::paint(ui, rect, Icon::TriangleAlert, p.warning);
                ui.label(RichText::new(s.closing_check_unavailable).color(p.warning));
                let retry =
                    ui.add(Button::new(RichText::new(s.retry).color(p.accent)).frame(false));
                retry.widget_info(|| {
                    WidgetInfo::labeled(Role::Button, true, s.closing_check_retry_a11y)
                });
                if retry.clicked() {
                    *event = Some(ClosingEvent::RetryCheck);
                }
            });
        }
        _ => {}
    }
}

fn saved_body(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    p: &Palette,
    snapshot: &ClosingSnapshot,
    event: &mut Option<ClosingEvent>,
) {
    let language = vm.language();
    let s = strings(language);
    ui.label(
        RichText::new(
            s.closing_saved_on
                .replace("{date}", &saved_date(&snapshot.closed_at, language)),
        )
        .text_style(egui::TextStyle::Small)
        .color(p.muted),
    );
    ui.label(
        RichText::new(
            s.closing_compared_with
                .replace("{month}", &month_label(snapshot.month.prev(), language)),
        )
        .text_style(egui::TextStyle::Small)
        .color(p.muted),
    );
    status_line(ui, vm, p, snapshot.month, event);
    ui.add_space(tokens::space_s());
    figure(
        ui,
        p,
        s.income_total,
        &signed_brl(snapshot.income),
        p.positive,
    );
    figure(ui, p, s.expenses, &plain_brl(snapshot.expenses), p.text);
    figure(
        ui,
        p,
        s.closing_refunds,
        &plain_brl(snapshot.refunds),
        p.text,
    );
    figure(ui, p, s.result, &signed_brl(snapshot.result), p.text);
    figure(
        ui,
        p,
        s.closing_entries_row,
        &plural(
            PluralKey::Entries,
            usize::try_from(snapshot.count).unwrap_or(0),
            language,
        ),
        p.text,
    );
    ui.add_space(tokens::space_s());
    // Saved wording from the saved figures; category names resolve from current ids.
    let text = insight_text(
        &snapshot.insight_inputs(),
        language,
        |id| {
            id.and_then(|id| vm.category(id))
                .map_or(s.qa_no_category, |c| c.name.as_str())
                .to_string()
        },
        |_| String::new(),
        false,
    );
    ui.add(Label::new(RichText::new(&text.headline).strong()).wrap());
    if let Some(comparison) = &text.comparison {
        ui.add(Label::new(RichText::new(comparison).color(p.muted)).wrap());
    }
    for detail in &text.details {
        ui.add(Label::new(RichText::new(detail).color(p.muted)).wrap());
    }
    // The saved expense keeps its title and amount; it opens only while that entry is live.
    if let Some(expense) = &snapshot.largest_expense {
        match expense
            .id
            .filter(|_| vm.closing_live_expense.contains(&snapshot.month))
        {
            Some(id) => {
                let open = ui.add(
                    Button::new(RichText::new(s.closing_open_entry).color(p.accent)).frame(false),
                );
                if open.clicked() {
                    *event = Some(ClosingEvent::OpenEntry(id));
                }
            }
            None => {
                ui.add(
                    Label::new(
                        RichText::new(s.closing_entry_gone)
                            .text_style(egui::TextStyle::Small)
                            .color(p.muted),
                    )
                    .wrap(),
                );
            }
        }
    }
}

/// The date a closing was saved, in the local calendar.
fn saved_date(closed_at: &str, language: crate::domain::settings::Language) -> String {
    chrono::DateTime::parse_from_rfc3339(closed_at).map_or_else(
        |_| closed_at.to_string(),
        |t| format_date(t.with_timezone(&chrono::Local).date_naive(), language),
    )
}

/// Draw the card for `month`: the saved closing when there is one, a Close month prompt for a
/// completed month without one, and nothing for the current or a future month.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, month: YearMonth) -> Option<ClosingEvent> {
    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    let saved = vm.closings.get(&month);
    if saved.is_none() && !is_completed(month, vm.today) {
        return None;
    }
    let mut event = None;
    let month_text = month_label(month, language);
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(tokens::BORDER_WIDTH * 1.5, p.accent))
        .corner_radius(CornerRadius::same(tokens::CARD_RADIUS))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                icons::paint(ui, rect, Icon::Bookmark, p.accent);
                ui.label(
                    RichText::new(s.closing_title)
                        .text_style(typography::section())
                        .strong(),
                );
            });
            ui.add_space(tokens::space_s());
            if let Some(snapshot) = saved {
                saved_body(ui, vm, &p, snapshot, &mut event);
                ui.add_space(tokens::space_s());
                let recalc = ui.add(
                    Button::new(s.closing_recalculate)
                        .min_size(Vec2::new(180.0, tokens::control_height())),
                );
                recalc.widget_info(|| {
                    WidgetInfo::labeled(
                        Role::Button,
                        true,
                        format!("{} {month_text}", s.closing_recalculate),
                    )
                });
                if recalc.clicked() {
                    event = Some(ClosingEvent::Recalculate(month));
                }
            } else {
                ui.add(Label::new(RichText::new(s.closing_not_saved).color(p.muted)).wrap());
                ui.add_space(tokens::space_s());
                let close = ui.add_sized(
                    [180.0, tokens::control_height()],
                    Button::new(
                        RichText::new(s.closing_close_month)
                            .color(p.on_accent)
                            .strong(),
                    )
                    .fill(p.accent),
                );
                close.widget_info(|| {
                    WidgetInfo::labeled(
                        Role::Button,
                        true,
                        format!("{} {month_text}", s.closing_close_month),
                    )
                });
                if close.clicked() {
                    event = Some(ClosingEvent::Close(month));
                }
            }
        });
    event
}
