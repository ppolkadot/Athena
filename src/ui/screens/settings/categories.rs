//! Categories management card.

use eframe::egui::{self, Align, Button, ComboBox, RichText, Sense, TextEdit, Vec2};

use crate::app::actions::AppAction;
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::category::CategoryScope;
use crate::domain::settings::Language;
use crate::i18n::strings;
use crate::ui::screens::ScreenEvent;
use crate::ui::theme::{palette, parse_hex, typography};

use super::{CategoryDraft, card};

fn scope_label(scope: CategoryScope, language: Language) -> &'static str {
    let s = strings(language);
    match scope {
        CategoryScope::Expense => s.expenses,
        CategoryScope::Income => s.income_total,
        CategoryScope::Both => s.scope_both,
    }
}

fn scope_combo(
    ui: &mut egui::Ui,
    salt: &str,
    language: Language,
    scope: &mut CategoryScope,
    name: &str,
) {
    let combo = ComboBox::from_id_salt(salt)
        .selected_text(scope_label(*scope, language))
        .width(130.0)
        .show_ui(ui, |ui| {
            for option in [
                CategoryScope::Expense,
                CategoryScope::Income,
                CategoryScope::Both,
            ] {
                ui.selectable_value(scope, option, scope_label(option, language));
            }
        });
    combo
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
}

fn swatch(ui: &mut egui::Ui, color: &str, fallback: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), 7.0, parse_hex(color).unwrap_or(fallback));
}

#[allow(clippy::too_many_lines)] // one card of plain layout code
pub(super) fn categories_card(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    ui_state: &mut UiState,
    events: &mut Vec<ScreenEvent>,
) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let settings = &mut ui_state.settings;
    card(ui, &p, s.settings_categories, |ui| {
        for category in &vm.all_categories {
            let mut draft = settings
                .category_drafts
                .get(&category.id)
                .cloned()
                .unwrap_or_else(|| CategoryDraft {
                    name: category.name.clone(),
                    color: category.color.clone(),
                    scope: category.applies_to,
                });
            let (mut ask_archive, mut save) = (false, false);
            ui.horizontal(|ui| {
                swatch(ui, &draft.color, p.muted);
                let name = ui.add(
                    TextEdit::singleline(&mut draft.name)
                        .desired_width(180.0)
                        .margin(Vec2::new(12.0, 10.0)),
                );
                name.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::TextInput, true, &category.name)
                });
                scope_combo(
                    ui,
                    &format!("category-scope-{}", category.id),
                    vm.language(),
                    &mut draft.scope,
                    &category.name,
                );
                let color = ui.add(
                    TextEdit::singleline(&mut draft.color)
                        .desired_width(90.0)
                        .font(typography::money_font())
                        .margin(Vec2::new(8.0, 10.0)),
                );
                color.widget_info(|| {
                    let name = format!("{} {}", s.color_label, category.name);
                    egui::WidgetInfo::labeled(egui::Role::TextInput, true, &name)
                });
                if category.archived {
                    ui.label(
                        RichText::new(s.archived_tag)
                            .text_style(egui::TextStyle::Small)
                            .color(p.warning),
                    );
                }
                let changed = draft.name.trim() != category.name
                    || draft.color.trim().to_uppercase() != category.color.to_uppercase()
                    || draft.scope != category.applies_to;
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if !category.archived
                        && ui
                            .add(
                                Button::new(RichText::new(s.archive).color(p.negative))
                                    .frame(false),
                            )
                            .clicked()
                    {
                        ask_archive = true;
                    }
                    if changed
                        && ui
                            .add(Button::new(RichText::new(s.save).color(p.accent)).frame(false))
                            .clicked()
                    {
                        save = true;
                    }
                });
            });
            settings.category_drafts.insert(category.id, draft);
            if ask_archive {
                settings.confirm_archive_category = Some(category.id);
            }
            if save {
                match settings.category_save_action(vm, category.id) {
                    Ok(Some(action)) => events.push(ScreenEvent::Apply(action)),
                    Ok(None) => {}
                    Err(e) => settings.error = Some(e),
                }
            }
            if settings.confirm_archive_category == Some(category.id) {
                ui.label(
                    RichText::new(s.archive_category_body.replace("{name}", &category.name))
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
                        events.push(ScreenEvent::Apply(AppAction::ArchiveCategory(category.id)));
                    }
                    if ui.button(s.cancel).clicked() {
                        settings.confirm_archive_category = None;
                    }
                });
            }
        }
        ui.add_space(8.0);
        let caption = ui.label(RichText::new(s.categories_add).text_style(egui::TextStyle::Small));
        ui.horizontal(|ui| {
            swatch(ui, &settings.category_color, p.muted);
            let field = ui.add(
                TextEdit::singleline(&mut settings.new_category)
                    .desired_width(180.0)
                    .margin(Vec2::new(12.0, 10.0)),
            );
            field.labelled_by(caption.id);
            scope_combo(
                ui,
                "new-category-scope",
                vm.language(),
                &mut settings.category_scope,
                s.categories_add,
            );
            let color = ui.add(
                TextEdit::singleline(&mut settings.category_color)
                    .desired_width(90.0)
                    .font(typography::money_font())
                    .margin(Vec2::new(8.0, 10.0)),
            );
            color.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TextInput, true, s.color_label)
            });
            if ui.button(s.add).clicked() {
                match settings.add_category_action() {
                    Ok(action) => events.push(ScreenEvent::Apply(action)),
                    Err(e) => settings.error = Some(e),
                }
            }
        });
    });
}
