//! Data folder, exports, and About cards.

use eframe::egui::{self, RichText};

use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::export::ExportScope;
use crate::i18n::{month_label, strings};
use crate::ui::screens::{ExportRequest, ScreenEvent};
use crate::ui::theme::{palette, typography};

use super::card;

pub(super) fn data_and_about_cards(
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
        let month_text = s
            .export_csv_month
            .replace("{month}", &month_label(ui_state.month, vm.language()));
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
        });
    });
    card(ui, &p, s.settings_about, |ui| {
        ui.label(format!(
            "Athena · {} {}",
            s.version,
            env!("CARGO_PKG_VERSION")
        ));
    });
}
