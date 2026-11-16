//! Household people management card.

use eframe::egui::{self, Align, Button, RichText, TextEdit, Vec2};

use crate::app::actions::AppAction;
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::person::PersonKind;
use crate::domain::settings::Language;
use crate::i18n::strings;
use crate::ui::screens::ScreenEvent;
use crate::ui::theme::palette;

use super::card;

fn kind_label(kind: PersonKind, language: Language) -> &'static str {
    let s = strings(language);
    match kind {
        PersonKind::Owner => s.person_owner,
        PersonKind::Partner => s.onb_summary_partner,
        PersonKind::Other => s.person_other,
    }
}

#[allow(clippy::too_many_lines)] // one card of plain layout code
pub(super) fn people_card(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    ui_state: &mut UiState,
    events: &mut Vec<ScreenEvent>,
) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let settings = &mut ui_state.settings;
    card(ui, &p, s.settings_people, |ui| {
        for person in &vm.all_people {
            let mut draft = settings
                .person_drafts
                .get(&person.id)
                .cloned()
                .unwrap_or_else(|| person.name.clone());
            let (mut ask_archive, mut save) = (false, false);
            ui.horizontal(|ui| {
                let edit = ui.add(
                    TextEdit::singleline(&mut draft)
                        .desired_width(220.0)
                        .margin(Vec2::new(12.0, 10.0)),
                );
                edit.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::TextInput, true, &person.name)
                });
                ui.label(
                    RichText::new(kind_label(person.kind, vm.language()))
                        .text_style(egui::TextStyle::Small)
                        .color(p.muted),
                );
                if person.archived {
                    ui.label(
                        RichText::new(s.archived_tag)
                            .text_style(egui::TextStyle::Small)
                            .color(p.warning),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if !person.archived
                        && person.kind != PersonKind::Owner
                        && ui
                            .add(
                                Button::new(RichText::new(s.archive).color(p.negative))
                                    .frame(false),
                            )
                            .clicked()
                    {
                        ask_archive = true;
                    }
                    if draft.trim() != person.name
                        && ui
                            .add(Button::new(RichText::new(s.save).color(p.accent)).frame(false))
                            .clicked()
                    {
                        save = true;
                    }
                });
            });
            settings.person_drafts.insert(person.id, draft);
            if ask_archive {
                settings.confirm_archive_person = Some(person.id);
            }
            if save {
                match settings.person_rename_action(vm, person.id) {
                    Ok(Some(action)) => events.push(ScreenEvent::Apply(action)),
                    Ok(None) => {}
                    Err(e) => settings.error = Some(e),
                }
            }
            if settings.confirm_archive_person == Some(person.id) {
                ui.label(
                    RichText::new(s.archive_person_body.replace("{name}", &person.name))
                        .color(p.muted),
                );
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            Button::new(RichText::new(s.archive).color(p.bg).strong())
                                .fill(p.negative),
                        )
                        .clicked()
                    {
                        events.push(ScreenEvent::Apply(AppAction::ArchivePerson(person.id)));
                    }
                    if ui.button(s.cancel).clicked() {
                        settings.confirm_archive_person = None;
                    }
                });
            }
        }
        ui.add_space(8.0);
        let caption = ui.label(RichText::new(s.people_add).text_style(egui::TextStyle::Small));
        ui.horizontal(|ui| {
            let field = ui.add(
                TextEdit::singleline(&mut settings.new_person)
                    .desired_width(220.0)
                    .margin(Vec2::new(12.0, 10.0)),
            );
            field.labelled_by(caption.id);
            if ui.button(s.add).clicked() {
                match settings.add_person_action() {
                    Ok(action) => events.push(ScreenEvent::Apply(action)),
                    Err(e) => settings.error = Some(e),
                }
            }
        });
    });
}
