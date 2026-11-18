//! Settings → Templates: the stored entry templates, alphabetical, with full-field Edit and a
//! named Delete confirmation. Editing reuses the Quick Add form without a date.

use eframe::egui::{self, Align, Button, RichText};

use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::i18n::{format_date, kind_label, strings};
use crate::ui::screens::ScreenEvent;
use crate::ui::theme::palette;

use super::card;

pub(super) fn templates_card(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    _ui_state: &mut UiState,
    events: &mut Vec<ScreenEvent>,
) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    card(ui, &p, s.settings_templates, |ui| {
        ui.label(RichText::new(s.settings_templates_hint).color(p.muted));
        ui.add_space(8.0);
        if vm.templates.is_empty() {
            ui.label(RichText::new(s.settings_templates_empty).color(p.muted));
            return;
        }
        for template in &vm.templates {
            let d = &template.draft;
            let needs_repair = !d.issues(&vm.all_people, &vm.all_categories).is_empty();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&d.name).strong());
                    let summary = s
                        .settings_template_summary
                        .replace("{amount}", &d.amount.format_brl())
                        .replace("{kind}", kind_label(d.kind, vm.language()));
                    let used = template.last_used_at.as_deref().map_or_else(
                        || s.settings_template_never_used.to_string(),
                        |stamp| {
                            let day = stamp.get(..10).and_then(|text| {
                                chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
                            });
                            day.map_or_else(
                                || s.settings_template_never_used.to_string(),
                                |day| {
                                    s.settings_template_last_used
                                        .replace("{date}", &format_date(day, vm.language()))
                                },
                            )
                        },
                    );
                    ui.label(
                        RichText::new(format!("{summary} · {used}"))
                            .text_style(egui::TextStyle::Small)
                            .color(p.muted),
                    );
                    if needs_repair {
                        ui.label(
                            RichText::new(s.qa_template_needs_repair)
                                .text_style(egui::TextStyle::Small)
                                .color(p.warning),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let delete_name = s.settings_template_delete_named.replace("{name}", &d.name);
                    let delete =
                        ui.add(Button::new(RichText::new(s.delete).color(p.negative)).frame(false));
                    delete.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, &delete_name)
                    });
                    if delete.clicked() {
                        events.push(ScreenEvent::DeleteTemplate(template.id));
                    }
                    let edit_name = s.settings_template_edit_named.replace("{name}", &d.name);
                    let edit =
                        ui.add(Button::new(RichText::new(s.edit).color(p.accent)).frame(false));
                    edit.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, &edit_name)
                    });
                    if edit.clicked() {
                        events.push(ScreenEvent::EditTemplate(template.id));
                    }
                });
            });
            ui.separator();
        }
    });
}
