//! The startup recovery screen (`§11`).
//!
//! Drawn instead of the app when the database cannot be opened. It names the failure, offers the
//! data folder and Quit, and lists the backups with a per-row Restore action. It always runs on the
//! dark theme: the theme and language choices live in the database that could not be read, so the
//! system language is used.

use eframe::egui::{
    self, Align, Button, CornerRadius, Frame, Layout, Margin, RichText, Role, ScrollArea, Stroke,
    WidgetInfo,
};

use crate::app::recovery::RecoveryState;
use crate::domain::settings::Theme;
use crate::i18n::{backup_kind_label, format_date, strings};
use crate::ui::theme::{Palette, palette, typography};

/// What the user asked the recovery screen to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryEvent {
    OpenDataFolder,
    Quit,
    StartFresh,
    Restore(std::path::PathBuf),
}

pub fn show(ui: &mut egui::Ui, state: &RecoveryState) -> Vec<RecoveryEvent> {
    let p = palette(Theme::Dark);
    let language = state.language;
    let s = strings(language);
    let mut events = Vec::new();

    Frame::new()
        .fill(p.bg)
        .inner_margin(Margin::same(32))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(ui.available_height());
            ui.label(
                RichText::new(s.recovery_title)
                    .text_style(typography::display())
                    .color(p.text),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    s.recovery_body
                        .replace("{path}", &state.dirs.db_path.display().to_string()),
                )
                .color(p.muted),
            );
            ui.add_space(6.0);
            ui.label(RichText::new(state.detail(language)).color(p.negative));
            if let Some(warning) = state.restore_warning(language) {
                ui.add_space(6.0);
                ui.label(RichText::new(warning).color(p.warning));
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new(s.recovery_restore_hint)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
            if let Some(err) = &state.restore_error {
                ui.add_space(6.0);
                ui.label(RichText::new(err).color(p.negative));
            }
            ui.add_space(16.0);
            backup_list(ui, state, &p, &mut events);
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui.button(s.open_data_folder).clicked() {
                    events.push(RecoveryEvent::OpenDataFolder);
                }
                if state.can_start_fresh() && ui.button(s.recovery_start_fresh).clicked() {
                    events.push(RecoveryEvent::StartFresh);
                }
                if ui.button(s.recovery_quit).clicked() {
                    events.push(RecoveryEvent::Quit);
                }
            });
        });

    events
}

fn backup_list(
    ui: &mut egui::Ui,
    state: &RecoveryState,
    p: &Palette,
    events: &mut Vec<RecoveryEvent>,
) {
    let language = state.language;
    let s = strings(language);
    if state.backups.is_empty() {
        ui.label(RichText::new(s.recovery_no_backups).color(p.muted));
        return;
    }
    ui.label(
        RichText::new(s.recovery_backups_title)
            .text_style(typography::section())
            .strong()
            .color(p.text),
    );
    ui.add_space(8.0);
    ScrollArea::vertical().show(ui, |ui| {
        for file in &state.backups {
            let file_name = file.path.file_name().map_or_else(
                || file.path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let local = file.created_utc.with_timezone(&chrono::Local);
            let date_text = if file.kind == crate::infrastructure::db::backup::BackupKind::Auto {
                format_date(
                    crate::infrastructure::db::backup::auto_calendar_date(&file.path)
                        .unwrap_or_else(|| local.date_naive()),
                    language,
                )
            } else {
                format!(
                    "{} {}",
                    format_date(local.date_naive(), language),
                    local.format("%H:%M")
                )
            };
            let kind_label = backup_kind_label(file.kind, language);
            ui.push_id(&file.path, |ui| {
                Frame::new()
                    .fill(p.card)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    RichText::new(&file_name)
                                        .font(typography::money_font())
                                        .color(p.text),
                                );
                                ui.label(
                                    RichText::new(format!("{kind_label} · {date_text}"))
                                        .text_style(egui::TextStyle::Small)
                                        .color(p.muted),
                                );
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let restore = ui.add(Button::new(s.restore_ellipsis));
                                restore.widget_info(|| {
                                    WidgetInfo::labeled(
                                        Role::Button,
                                        true,
                                        format!("{} {}", s.restore_ellipsis, file_name),
                                    )
                                });
                                if restore.clicked() {
                                    events.push(RecoveryEvent::Restore(file.path.clone()));
                                }
                            });
                        });
                    });
            });
            ui.add_space(6.0);
        }
    });
}
