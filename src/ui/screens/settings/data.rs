//! Data folder, exports, backups & restore, and About cards.

use eframe::egui::{
    self, Align, Button, CornerRadius, Frame, Layout, Margin, RichText, Role, Stroke, WidgetInfo,
};

use crate::app::actions::{OperationKind, OperationNotice};
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::export::ExportScope;
use crate::domain::settings::Language;
use crate::i18n::{backup_kind_label, format_date, month_label, strings};
use crate::infrastructure::db::backup::BackupKind;
use crate::ui::screens::{ExportRequest, ScreenEvent};
use crate::ui::theme::{Palette, palette, typography};

use super::card;

pub(super) fn about_card(ui: &mut egui::Ui, vm: &ViewModel) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    card(ui, &p, s.settings_about, |ui| {
        ui.label(format!(
            "Athena · {} {}",
            s.version,
            env!("CARGO_PKG_VERSION")
        ));
    });
}

pub(super) fn data_card(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    ui_state: &UiState,
    events: &mut Vec<ScreenEvent>,
) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    card(ui, &p, s.settings_data, |ui| {
        ui.label(
            RichText::new(s.data_path)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        ui.label(RichText::new(&vm.data_path).font(typography::money_font()));
        ui.add_space(8.0);
        if ui.button(s.open_data_folder).clicked() {
            events.push(ScreenEvent::OpenDataFolder);
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new(s.export_hint)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        let month_name = month_label(ui_state.month, vm.language());
        let month_text = s.export_csv_month.replace("{month}", &month_name);
        let xlsx_month_text = s.export_xlsx_month.replace("{month}", &month_name);
        ui.horizontal_wrapped(|ui| {
            if ui.button(s.export_json).clicked() {
                events.push(ScreenEvent::Export(ExportRequest::Json));
            }
            if ui.button(&month_text).clicked() {
                events.push(ScreenEvent::Export(ExportRequest::Csv(ExportScope::Month(
                    ui_state.month,
                ))));
            }
            if ui.button(s.export_csv_all).clicked() {
                events.push(ScreenEvent::Export(ExportRequest::Csv(ExportScope::All)));
            }
            if ui.button(&xlsx_month_text).clicked() {
                events.push(ScreenEvent::Export(ExportRequest::Xlsx(
                    ExportScope::Month(ui_state.month),
                )));
            }
            if ui.button(s.export_xlsx_all).clicked() {
                events.push(ScreenEvent::Export(ExportRequest::Xlsx(ExportScope::All)));
            }
            if ui.button(s.import_ellipsis).clicked() {
                events.push(ScreenEvent::PickImportFile);
            }
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new(s.ap_scope_note)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        if let Some(err) = &ui_state.settings.import_error {
            ui.add_space(6.0);
            ui.label(RichText::new(err).color(p.negative));
        }
    });
}

pub(super) fn backups_card(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    ui_state: &UiState,
    events: &mut Vec<ScreenEvent>,
) {
    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    card(ui, &p, s.backups_title, |ui| {
        ui.label(
            RichText::new(s.backups_hint)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        if let Some(notice) = &vm.last_operation {
            ui.add_space(6.0);
            ui.label(
                RichText::new(format_operation_notice(notice, language))
                    .text_style(egui::TextStyle::Small)
                    .color(p.accent),
            );
        }
        if let Some(err) = &ui_state.settings.restore_error {
            ui.add_space(6.0);
            ui.label(RichText::new(err).color(p.negative));
        }
        if let Some(err) = &ui_state.settings.backup_error {
            ui.add_space(6.0);
            ui.label(RichText::new(err).color(p.negative));
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button(s.back_up_now).clicked() {
                events.push(ScreenEvent::CreateBackup);
            }
            if ui.button(s.restore_from_file).clicked() {
                events.push(ScreenEvent::PickRestoreFile);
            }
        });
        ui.add_space(12.0);
        if vm.backups.is_empty() {
            ui.label(RichText::new(s.backups_empty).color(p.muted));
        } else {
            backup_rows(ui, vm, &p, events);
        }
    });
}

fn backup_rows(ui: &mut egui::Ui, vm: &ViewModel, p: &Palette, events: &mut Vec<ScreenEvent>) {
    let language = vm.language();
    let s = strings(language);
    for file in &vm.backups {
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
        let size_text = format_bytes(file.bytes);

        ui.push_id(&file.path, |ui| {
            Frame::new()
                .fill(p.panel)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&file_name).font(typography::money_font()));
                            ui.label(
                                RichText::new(format!("{kind_label} · {date_text} · {size_text}"))
                                    .text_style(egui::TextStyle::Small)
                                    .color(p.muted),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let btn = ui.add(Button::new(s.restore_ellipsis));
                            btn.widget_info(|| {
                                WidgetInfo::labeled(
                                    Role::Button,
                                    true,
                                    format!("{} {}", s.restore_ellipsis, file_name),
                                )
                            });
                            if btn.clicked() {
                                btn.request_focus();
                                events.push(ScreenEvent::RequestRestoreListed(file.clone()));
                            }
                            if file.kind == BackupKind::Manual {
                                let delete = ui.add(Button::new(s.backup_delete));
                                delete.widget_info(|| {
                                    WidgetInfo::labeled(
                                        Role::Button,
                                        true,
                                        format!("{} {}", s.backup_delete, file_name),
                                    )
                                });
                                if delete.clicked() {
                                    delete.request_focus();
                                    events.push(ScreenEvent::DeleteBackup(file.path.clone()));
                                }
                            }
                        });
                    });
                });
        });
        ui.add_space(6.0);
    }
}

fn format_operation_notice(notice: &OperationNotice, language: Language) -> String {
    let s = strings(language);
    let snap_name = notice.snapshot.as_ref().map_or_else(String::new, |p| {
        p.file_name().map_or_else(
            || p.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    });
    let src_name = notice.source.as_deref().unwrap_or("");
    match notice.kind {
        OperationKind::BackupCreated => s.notice_backup_created.replace("{snapshot}", &snap_name),
        OperationKind::Restored => s
            .notice_restored
            .replace("{source}", src_name)
            .replace("{snapshot}", &snap_name),
        OperationKind::Imported => s
            .notice_imported
            .replace("{source}", src_name)
            .replace("{snapshot}", &snap_name),
        OperationKind::TrashPurged => s.notice_trash_purged.to_string(),
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1_024 {
        format!("{bytes} B")
    } else if bytes < 1_024 * 1_024 {
        format!("{} KB", bytes / 1_024)
    } else {
        let tenths = (bytes * 10) / (1_024 * 1_024);
        format!("{}.{} MB", tenths / 10, tenths % 10)
    }
}
