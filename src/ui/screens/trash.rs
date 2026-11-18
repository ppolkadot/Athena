//! Trash screen: recover recently deleted entries within 30 days or permanently purge them.

use eframe::egui::{
    self, Align, Button, CornerRadius, Frame, Layout, Margin, Rect, RichText, Role, Stroke, Vec2,
    WidgetInfo,
};

use crate::app::actions::AppAction;
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::settings::Language;
use crate::domain::transaction::TransactionType;
use crate::i18n::{format_date, strings};
use crate::infrastructure::db::trash_repo::TrashItem;
use crate::ui::components::transaction_row::{amount_text, people_label, type_badge};
use crate::ui::components::{empty_state, person_pill};
use crate::ui::icons::{self, Icon};
use crate::ui::screens::ScreenEvent;
use crate::ui::theme::{Palette, palette, typography};

fn deleted_date_label(deleted_at: &str, language: Language) -> String {
    let s = strings(language);
    let date_text = chrono::DateTime::parse_from_rfc3339(deleted_at)
        .ok()
        .map_or_else(
            || deleted_at.to_string(),
            |dt| format_date(dt.with_timezone(&chrono::Local).date_naive(), language),
        );
    s.trash_deleted_on.replace("{date}", &date_text)
}

fn header(ui: &mut egui::Ui, vm: &ViewModel, p: &Palette, events: &mut Vec<ScreenEvent>) {
    let s = strings(vm.language());
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            let label = ui.label(
                RichText::new(s.trash)
                    .text_style(typography::display())
                    .strong(),
            );
            let underline = Rect::from_min_size(
                egui::pos2(label.rect.left(), label.rect.bottom() + 2.0),
                Vec2::new(label.rect.width(), 3.0),
            );
            ui.painter()
                .rect_filled(underline, CornerRadius::same(2), p.accent);
            ui.add_space(5.0);
        });
        if !vm.trash.is_empty() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = Button::new(RichText::new(s.empty_trash).color(p.negative).strong())
                    .min_size(Vec2::new(120.0, 36.0));
                let resp = ui.add(button);
                if resp.clicked() {
                    resp.request_focus();
                    events.push(ScreenEvent::EmptyTrash);
                }
            });
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new(s.trash_retention_hint).color(p.muted));
}

fn trash_row(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    state: &mut UiState,
    index: usize,
    item: &TrashItem,
    focus_target: Option<usize>,
    events: &mut Vec<ScreenEvent>,
) {
    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    let tx = &item.tx;
    let (icon, label) = type_badge(tx.kind, language);
    let amount_color = match tx.kind {
        TransactionType::Expense => p.negative,
        TransactionType::Income | TransactionType::Refund => p.positive,
        TransactionType::Settlement => p.accent,
    };

    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (icon_rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
                icons::paint(ui, icon_rect, icon, amount_color);
                ui.label(
                    RichText::new(label)
                        .text_style(egui::TextStyle::Small)
                        .color(p.muted),
                );
                ui.add(egui::Label::new(RichText::new(&tx.title).strong()).truncate())
                    .on_hover_text(&tx.title);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(amount_text(tx.kind, tx.amount))
                            .font(typography::money_font())
                            .color(amount_color),
                    );
                    person_pill::show(ui, vm.theme(), &people_label(tx, vm));
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let tx_date = format_date(tx.date, language);
                let deleted = deleted_date_label(&item.deleted_at, language);
                ui.label(
                    RichText::new(format!("{tx_date}  ·  {deleted}"))
                        .text_style(egui::TextStyle::Small)
                        .color(p.muted),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let purge_btn = ui.add(
                        Button::new(
                            RichText::new(s.delete_forever)
                                .text_style(egui::TextStyle::Small)
                                .color(p.negative),
                        )
                        .frame(false),
                    );
                    purge_btn.widget_info(|| {
                        WidgetInfo::labeled(
                            Role::Button,
                            true,
                            format!("{} {}", s.delete_forever, tx.title),
                        )
                    });
                    if purge_btn.clicked() {
                        purge_btn.request_focus();
                        state.pending_purge_focus_row = Some(index);
                        events.push(ScreenEvent::PurgeTransaction(tx.id));
                    }

                    let restore_btn = ui.add(
                        Button::new(
                            RichText::new(s.restore)
                                .text_style(egui::TextStyle::Small)
                                .color(p.accent)
                                .strong(),
                        )
                        .frame(false),
                    );
                    restore_btn.widget_info(|| {
                        WidgetInfo::labeled(
                            Role::Button,
                            true,
                            format!("{} {}", s.restore, tx.title),
                        )
                    });
                    if focus_target == Some(index) {
                        restore_btn.request_focus();
                    }
                    if restore_btn.clicked() {
                        state.trash_focus_row = Some(index);
                        events.push(ScreenEvent::Apply(AppAction::RestoreTransaction(tx.id)));
                    }
                });
            });
        });
}

#[must_use]
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) -> Vec<ScreenEvent> {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let mut events = Vec::new();

    header(ui, vm, &p, &mut events);
    ui.add_space(16.0);

    if vm.trash.is_empty() {
        state.trash_focus_row = None;
        empty_state::show(
            ui,
            vm.theme(),
            Icon::Trash2,
            s.trash_empty_title,
            s.trash_empty_hint,
            None,
        );
        return events;
    }

    let focus_target = if state.trash_focus_row.is_some() {
        if ui.ctx().memory(|m| m.top_modal_layer().is_none()) {
            state
                .trash_focus_row
                .take()
                .map(|idx| idx.min(vm.trash.len() - 1))
        } else {
            ui.ctx().request_repaint();
            None
        }
    } else {
        None
    };

    for (index, item) in vm.trash.iter().enumerate() {
        trash_row(ui, vm, state, index, item, focus_target, &mut events);
        ui.add_space(8.0);
    }

    events
}
