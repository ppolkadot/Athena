//! Settings: profile, people, categories, data location and version. Edits become `AppAction`s
//! for the app to apply; language and theme changes take effect immediately.

use std::collections::HashMap;

use eframe::egui::{
    self, Align, Button, ComboBox, CornerRadius, Frame, Margin, RichText, Sense, Stroke, TextEdit,
    Vec2,
};

use crate::app::actions::AppAction;
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::category::{Category, CategoryScope, DEFAULT_CATEGORY_COLOR};
use crate::domain::export::ExportScope;
use crate::domain::money::MoneyError;
use crate::domain::person::{Person, PersonKind};
use crate::domain::settings::{Language, Profile, Theme};
use crate::i18n::month_label;
use crate::i18n::{money_error_message, strings};
use crate::ui::components::money_input;
use crate::ui::screens::{ExportRequest, ScreenEvent};
use crate::ui::theme::{Palette, palette, parse_hex, typography};

/// Why a settings form cannot be saved. Rendered through i18n so it follows the language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsError {
    NameRequired,
    Balance(MoneyError),
    ColorInvalid,
    /// The store refused the change, usually because the name is already used.
    SaveRejected,
}

#[must_use]
pub fn error_message(error: SettingsError, language: Language) -> &'static str {
    let s = strings(language);
    match error {
        SettingsError::NameRequired => s.v_name_required,
        SettingsError::Balance(e) => money_error_message(e, language),
        SettingsError::ColorInvalid => s.v_color_invalid,
        SettingsError::SaveRejected => s.settings_save_failed,
    }
}

/// An in-progress edit of one category row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryDraft {
    pub name: String,
    pub color: String,
    pub scope: CategoryScope,
}

#[derive(Debug, Clone)]
pub struct SettingsState {
    initialized: bool,
    pub owner_name: String,
    pub partner_name: String,
    pub balance_input: String,
    pub new_person: String,
    pub new_category: String,
    pub category_color: String,
    pub category_scope: CategoryScope,
    pub person_drafts: HashMap<i64, String>,
    pub category_drafts: HashMap<i64, CategoryDraft>,
    pub confirm_archive_person: Option<i64>,
    pub confirm_archive_category: Option<i64>,
    pub error: Option<SettingsError>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            initialized: false,
            owner_name: String::new(),
            partner_name: String::new(),
            balance_input: String::new(),
            new_person: String::new(),
            new_category: String::new(),
            category_color: DEFAULT_CATEGORY_COLOR.to_string(),
            category_scope: CategoryScope::Expense,
            person_drafts: HashMap::new(),
            category_drafts: HashMap::new(),
            confirm_archive_person: None,
            confirm_archive_category: None,
            error: None,
        }
    }
}

fn partner_of(vm: &ViewModel) -> Option<&Person> {
    vm.all_people
        .iter()
        .find(|p| p.kind == PersonKind::Partner && !p.archived)
}

impl SettingsState {
    /// Forget every in-progress edit; the next frame reloads the fields from the view model.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn load(&mut self, vm: &ViewModel) {
        self.owner_name = vm.owner_name().unwrap_or_default().to_string();
        self.partner_name = partner_of(vm).map(|p| p.name.clone()).unwrap_or_default();
        self.balance_input = vm
            .profile
            .as_ref()
            .map_or_else(String::new, |p| p.start_balance.br_number());
        self.initialized = true;
    }

    /// The actions that save the profile card: owner and partner names, then the starting
    /// balance. Only what changed is included.
    ///
    /// # Errors
    /// A blank owner name or an invalid balance.
    pub fn profile_actions(&self, vm: &ViewModel) -> Result<Vec<AppAction>, SettingsError> {
        let mut actions = Vec::new();
        let owner_name = self.owner_name.trim();
        if owner_name.is_empty() {
            return Err(SettingsError::NameRequired);
        }
        let balance = if self.balance_input.trim().is_empty() {
            crate::domain::money::Money::ZERO
        } else {
            crate::domain::money::Money::parse(&self.balance_input)
                .map_err(SettingsError::Balance)?
        };
        if let Some(owner) = vm.owner().filter(|o| o.name != owner_name) {
            actions.push(AppAction::SavePerson(Person {
                name: owner_name.to_string(),
                ..owner.clone()
            }));
        }
        let partner_name = self.partner_name.trim();
        match partner_of(vm) {
            Some(partner) if !partner_name.is_empty() && partner.name != partner_name => {
                actions.push(AppAction::SavePerson(Person {
                    name: partner_name.to_string(),
                    ..partner.clone()
                }));
            }
            None if !partner_name.is_empty() => actions.push(AppAction::SavePerson(Person {
                id: 0,
                name: partner_name.to_string(),
                kind: PersonKind::Partner,
                archived: false,
                created_at: String::new(),
            })),
            _ => {}
        }
        if let Some(profile) = vm.profile.as_ref().filter(|p| p.start_balance != balance) {
            actions.push(AppAction::SaveProfile(Profile {
                start_balance: balance,
                ..profile.clone()
            }));
        }
        Ok(actions)
    }

    /// # Errors
    /// [`SettingsError::NameRequired`] for a blank name.
    pub fn add_person_action(&self) -> Result<AppAction, SettingsError> {
        let name = self.new_person.trim();
        if name.is_empty() {
            return Err(SettingsError::NameRequired);
        }
        Ok(AppAction::SavePerson(Person {
            id: 0,
            name: name.to_string(),
            kind: PersonKind::Other,
            archived: false,
            created_at: String::new(),
        }))
    }

    /// The rename action for a person row, or `None` when nothing changed.
    ///
    /// # Errors
    /// [`SettingsError::NameRequired`] when the draft is blank.
    pub fn person_rename_action(
        &self,
        vm: &ViewModel,
        id: i64,
    ) -> Result<Option<AppAction>, SettingsError> {
        let (Some(draft), Some(person)) = (self.person_drafts.get(&id), vm.person(id)) else {
            return Ok(None);
        };
        let name = draft.trim();
        if name.is_empty() {
            return Err(SettingsError::NameRequired);
        }
        if name == person.name {
            return Ok(None);
        }
        Ok(Some(AppAction::SavePerson(Person {
            name: name.to_string(),
            ..person.clone()
        })))
    }

    /// # Errors
    /// A blank name or a color that is not `#RRGGBB`.
    pub fn add_category_action(&self) -> Result<AppAction, SettingsError> {
        let name = self.new_category.trim();
        if name.is_empty() {
            return Err(SettingsError::NameRequired);
        }
        let color = self.category_color.trim();
        parse_hex(color).ok_or(SettingsError::ColorInvalid)?;
        Ok(AppAction::SaveCategory(Category {
            name: name.to_string(),
            color: color.to_uppercase(),
            applies_to: self.category_scope,
            ..Category::default()
        }))
    }

    /// The save action for a category row, or `None` when nothing changed.
    ///
    /// # Errors
    /// A blank name or a malformed color.
    pub fn category_save_action(
        &self,
        vm: &ViewModel,
        id: i64,
    ) -> Result<Option<AppAction>, SettingsError> {
        let (Some(draft), Some(category)) = (self.category_drafts.get(&id), vm.category(id)) else {
            return Ok(None);
        };
        let name = draft.name.trim();
        if name.is_empty() {
            return Err(SettingsError::NameRequired);
        }
        let color = draft.color.trim().to_uppercase();
        parse_hex(&color).ok_or(SettingsError::ColorInvalid)?;
        if name == category.name
            && color == category.color.to_uppercase()
            && draft.scope == category.applies_to
        {
            return Ok(None);
        }
        Ok(Some(AppAction::SaveCategory(Category {
            name: name.to_string(),
            color,
            applies_to: draft.scope,
            ..category.clone()
        })))
    }
}

fn card(ui: &mut egui::Ui, p: &Palette, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(title)
                    .text_style(typography::section())
                    .strong(),
            );
            ui.add_space(8.0);
            content(ui);
        });
    ui.add_space(12.0);
}

fn labeled_edit(ui: &mut egui::Ui, label: &str, value: &mut String) -> egui::Response {
    let caption = ui.label(RichText::new(label).text_style(egui::TextStyle::Small));
    let response = ui.add(
        TextEdit::singleline(value)
            .desired_width(f32::INFINITY)
            .margin(Vec2::new(12.0, 10.0)),
    );
    response.labelled_by(caption.id)
}

fn kind_label(kind: PersonKind, language: Language) -> &'static str {
    let s = strings(language);
    match kind {
        PersonKind::Owner => s.person_owner,
        PersonKind::Partner => s.onb_summary_partner,
        PersonKind::Other => s.person_other,
    }
}

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

fn profile_card(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    ui_state: &mut UiState,
    events: &mut Vec<ScreenEvent>,
) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let settings = &mut ui_state.settings;
    card(ui, &p, s.settings_profile, |ui| {
        labeled_edit(ui, s.onb_your_name, &mut settings.owner_name);
        ui.add_space(4.0);
        labeled_edit(ui, s.onb_partner_name, &mut settings.partner_name);
        ui.add_space(4.0);
        money_input::show(
            ui,
            vm.theme(),
            vm.language(),
            s.onb_balance_title,
            &mut settings.balance_input,
            false,
        );
        ui.add_space(4.0);
        if ui
            .add_sized(
                [120.0, 36.0],
                Button::new(RichText::new(s.save).color(p.bg).strong()).fill(p.accent),
            )
            .clicked()
        {
            match settings.profile_actions(vm) {
                Ok(actions) => {
                    settings.error = None;
                    events.extend(actions.into_iter().map(ScreenEvent::Apply));
                }
                Err(e) => settings.error = Some(e),
            }
        }
        if let Some(error) = settings.error {
            ui.label(RichText::new(error_message(error, vm.language())).color(p.negative));
        }
        ui.add_space(12.0);

        let Some(profile) = vm.profile.as_ref() else {
            return;
        };
        ui.label(
            RichText::new(s.theme)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        ui.horizontal(|ui| {
            for (theme, label) in [(Theme::Dark, s.dark), (Theme::Light, s.light)] {
                if ui.selectable_label(profile.theme == theme, label).clicked()
                    && profile.theme != theme
                {
                    events.push(ScreenEvent::Apply(AppAction::SaveProfile(Profile {
                        theme,
                        ..profile.clone()
                    })));
                }
            }
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(s.language)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        ui.horizontal(|ui| {
            for (language, label) in [(Language::En, s.english), (Language::PtBr, s.portuguese)] {
                if ui
                    .selectable_label(profile.language == language, label)
                    .clicked()
                    && profile.language != language
                {
                    events.push(ScreenEvent::Apply(AppAction::SaveProfile(Profile {
                        language,
                        ..profile.clone()
                    })));
                }
            }
        });
    });
}

#[allow(clippy::too_many_lines)] // one card of plain layout code
fn people_card(
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

#[allow(clippy::too_many_lines)] // one card of plain layout code
fn categories_card(
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

/// Draw the settings screen. Returns what the app should apply or do.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, ui_state: &mut UiState) -> Vec<ScreenEvent> {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let mut events = Vec::new();
    if !ui_state.settings.initialized {
        ui_state.settings.load(vm);
    }
    ui.label(
        RichText::new(s.settings)
            .text_style(typography::display())
            .strong(),
    );
    ui.add_space(12.0);

    profile_card(ui, vm, ui_state, &mut events);
    people_card(ui, vm, ui_state, &mut events);
    categories_card(ui, vm, ui_state, &mut events);
    let shown = ui_state
        .settings
        .error
        .filter(|e| matches!(e, SettingsError::SaveRejected | SettingsError::ColorInvalid));
    if let Some(error) = shown {
        ui.label(RichText::new(error_message(error, vm.language())).color(p.negative));
        ui.add_space(8.0);
    }

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
    events
}

/// The program and arguments that open a folder in the system file manager.
#[must_use]
pub fn folder_command(folder: &std::path::Path) -> (&'static str, Vec<std::ffi::OsString>) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    (program, vec![folder.as_os_str().to_owned()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppCore;
    use crate::application::services::onboarding::OnboardingResult;
    use crate::domain::money::Money;
    use crate::infrastructure::db::Db;

    fn core(partner: Option<&str>) -> AppCore {
        let mut core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        core.complete_onboarding(OnboardingResult {
            display_name: "A3".into(),
            partner_name: partner.map(Into::into),
            start_balance: Money::from_cents(1_000),
            language: Language::En,
        })
        .unwrap();
        core
    }

    fn loaded(core: &AppCore) -> SettingsState {
        let mut state = SettingsState::default();
        state.load(&core.vm);
        state
    }

    #[test]
    fn owner_rename_updates_profile_and_history() {
        let mut core = core(None);
        let mut owner = core.owner().unwrap().clone();
        owner.name = "New".into();
        core.save_person(owner).unwrap();
        assert_eq!(core.owner().unwrap().name, "New");
        assert_eq!(core.vm.profile.as_ref().unwrap().language, Language::En);
    }

    #[test]
    fn the_profile_card_saves_only_what_changed() {
        let core = core(Some("Ana"));
        let mut state = loaded(&core);
        assert_eq!(
            (
                state.owner_name.as_str(),
                state.partner_name.as_str(),
                state.balance_input.as_str()
            ),
            ("A3", "Ana", "10,00")
        );
        assert_eq!(
            state.profile_actions(&core.vm),
            Ok(vec![]),
            "an untouched form changes nothing"
        );

        state.owner_name = "  Alex ".into();
        state.partner_name = "Bia".into();
        state.balance_input = "-5,50".into();
        let actions = state.profile_actions(&core.vm).unwrap();
        assert_eq!(actions.len(), 3);
        let AppAction::SavePerson(owner) = &actions[0] else {
            panic!("owner first")
        };
        assert_eq!(
            (owner.name.as_str(), owner.kind),
            ("Alex", PersonKind::Owner)
        );
        let AppAction::SavePerson(partner) = &actions[1] else {
            panic!("partner second")
        };
        assert_eq!(
            (partner.name.as_str(), partner.kind),
            ("Bia", PersonKind::Partner)
        );
        assert!(
            partner.id != 0,
            "an existing partner is renamed, not duplicated"
        );
        let AppAction::SaveProfile(profile) = &actions[2] else {
            panic!("balance last")
        };
        assert_eq!(profile.start_balance, Money::from_cents(-550));
        assert_eq!(
            profile.language,
            Language::En,
            "other profile fields carry over"
        );
    }

    #[test]
    fn a_missing_partner_can_be_added_from_the_profile_card() {
        let core = core(None);
        let mut state = loaded(&core);
        state.partner_name = "Ana".into();
        let actions = state.profile_actions(&core.vm).unwrap();
        let [AppAction::SavePerson(partner)] = actions.as_slice() else {
            panic!("one action")
        };
        assert_eq!((partner.id, partner.kind), (0, PersonKind::Partner));
    }

    #[test]
    fn profile_validation_blocks_blank_names_and_bad_balances() {
        let core = core(None);
        let mut state = loaded(&core);
        state.owner_name = "  ".into();
        assert_eq!(
            state.profile_actions(&core.vm),
            Err(SettingsError::NameRequired)
        );
        state.owner_name = "A3".into();
        state.balance_input = "0,005".into();
        assert_eq!(
            state.profile_actions(&core.vm),
            Err(SettingsError::Balance(MoneyError::TooPrecise))
        );
        state.balance_input = String::new();
        let actions = state.profile_actions(&core.vm).unwrap();
        assert!(
            matches!(actions.as_slice(), [AppAction::SaveProfile(p)] if p.start_balance == Money::ZERO)
        );
    }

    #[test]
    fn people_rows_rename_and_add_with_trimmed_names() {
        let core = core(Some("Ana"));
        let ana = core
            .vm
            .all_people
            .iter()
            .find(|p| p.kind == PersonKind::Partner)
            .unwrap()
            .id;
        let mut state = loaded(&core);
        assert_eq!(state.person_rename_action(&core.vm, ana), Ok(None));
        state.person_drafts.insert(ana, " Aninha ".into());
        let Ok(Some(AppAction::SavePerson(renamed))) = state.person_rename_action(&core.vm, ana)
        else {
            panic!()
        };
        assert_eq!((renamed.id, renamed.name.as_str()), (ana, "Aninha"));
        state.person_drafts.insert(ana, "   ".into());
        assert_eq!(
            state.person_rename_action(&core.vm, ana),
            Err(SettingsError::NameRequired)
        );

        assert_eq!(state.add_person_action(), Err(SettingsError::NameRequired));
        state.new_person = " Bia ".into();
        let Ok(AppAction::SavePerson(added)) = state.add_person_action() else {
            panic!()
        };
        assert_eq!(
            (added.id, added.name.as_str(), added.kind),
            (0, "Bia", PersonKind::Other)
        );
    }

    #[test]
    fn categories_validate_the_color_and_carry_the_scope() {
        let core = core(None);
        let mut state = loaded(&core);
        assert_eq!(
            state.add_category_action(),
            Err(SettingsError::NameRequired)
        );
        state.new_category = "Pets".into();
        state.category_color = "red".into();
        assert_eq!(
            state.add_category_action(),
            Err(SettingsError::ColorInvalid)
        );
        state.category_color = "#ff8a65".into();
        state.category_scope = CategoryScope::Both;
        let Ok(AppAction::SaveCategory(category)) = state.add_category_action() else {
            panic!()
        };
        assert_eq!(
            (category.id, category.color.as_str(), category.applies_to),
            (0, "#FF8A65", CategoryScope::Both)
        );

        let food = core
            .vm
            .all_categories
            .iter()
            .find(|c| c.name == "Food")
            .unwrap()
            .clone();
        state.category_drafts.insert(
            food.id,
            CategoryDraft {
                name: food.name.clone(),
                color: food.color.to_lowercase(),
                scope: food.applies_to,
            },
        );
        assert_eq!(
            state.category_save_action(&core.vm, food.id),
            Ok(None),
            "a case-only color change is no change"
        );
        state.category_drafts.get_mut(&food.id).unwrap().scope = CategoryScope::Both;
        let Ok(Some(AppAction::SaveCategory(saved))) =
            state.category_save_action(&core.vm, food.id)
        else {
            panic!()
        };
        assert_eq!(
            (saved.id, saved.applies_to, saved.is_default),
            (food.id, CategoryScope::Both, true)
        );
    }

    #[test]
    fn errors_are_localized() {
        for error in [
            SettingsError::NameRequired,
            SettingsError::Balance(MoneyError::Invalid),
            SettingsError::ColorInvalid,
            SettingsError::SaveRejected,
        ] {
            assert_ne!(
                error_message(error, Language::En),
                error_message(error, Language::PtBr)
            );
        }
    }

    #[test]
    fn opening_the_data_folder_uses_the_system_file_manager() {
        let (program, args) = folder_command(std::path::Path::new("C:/Data"));
        assert_eq!(args, vec![std::ffi::OsString::from("C:/Data")]);
        assert!(["explorer", "open", "xdg-open"].contains(&program));
    }

    type SettingsApp = (ViewModel, UiState, Vec<ScreenEvent>);

    fn harness(core: &AppCore) -> egui_kittest::Harness<'static, SettingsApp> {
        egui_kittest::Harness::builder()
            .with_size(Vec2::new(1100.0, 2600.0))
            .build_ui_state(
                |ui, (vm, state, events): &mut SettingsApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    crate::ui::theme::apply(ui.ctx(), vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    events.extend(show(ui, vm, state));
                },
                (core.vm.clone(), core.ui.clone(), Vec::new()),
            )
    }

    #[test]
    fn the_screen_lists_every_section_and_row() {
        use egui_kittest::kittest::Queryable;
        let core = core(Some("Ana"));
        let mut harness = harness(&core);
        harness.run_steps(3);
        for label in [
            "Settings",
            "Profile",
            "People",
            "Categories",
            "Data",
            "About",
            "Dark",
            "Light",
            "English",
            "Português",
        ] {
            assert!(harness.query_all_by_label(label).count() > 0, "{label}");
        }
        let about = format!("Athena · Version {}", env!("CARGO_PKG_VERSION"));
        assert!(harness.query_by_label(about.as_str()).is_some());
        for category in ["Food", "Salary", "Other income"] {
            assert!(
                harness.query_all_by_label(category).count() > 0,
                "{category}"
            );
        }
    }

    #[test]
    fn theme_and_language_switch_with_one_click() {
        use egui_kittest::kittest::Queryable;
        let core = core(None);
        let mut harness = harness(&core);
        harness.run_steps(3);
        harness.get_by_label("Light").click();
        harness.run_steps(2);
        harness.get_by_label("Português").click();
        harness.run_steps(2);
        let events = harness.state().2.clone();
        let theme = events.iter().find_map(|e| match e {
            ScreenEvent::Apply(AppAction::SaveProfile(p)) if p.theme == Theme::Light => {
                Some(p.clone())
            }
            _ => None,
        });
        assert!(theme.is_some(), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, ScreenEvent::Apply(AppAction::SaveProfile(p)) if p.language == Language::PtBr)));
    }

    #[test]
    fn archiving_a_person_asks_first_and_explains() {
        use egui_kittest::kittest::Queryable;
        let core = core(Some("Ana"));
        let ana = core
            .vm
            .all_people
            .iter()
            .find(|p| p.kind == PersonKind::Partner)
            .unwrap()
            .id;
        let mut harness = harness(&core);
        harness.run_steps(3);
        // Only the partner has an Archive button; the owner never does.
        let archive_buttons = harness.get_all_by_label("Archive").count();
        assert!(archive_buttons >= 1);
        harness.get_all_by_label("Archive").next().unwrap().click();
        harness.run_steps(3);
        assert_eq!(harness.state().1.settings.confirm_archive_person, Some(ana));
        assert!(harness.state().2.is_empty(), "the first click only asks");
        assert!(harness.query_by_label("Ana will no longer appear when adding entries. Their past entries keep their name.").is_some());
    }

    #[test]
    fn the_data_folder_button_asks_the_app_to_open_it() {
        use egui_kittest::kittest::Queryable;
        let core = core(None);
        let mut harness = harness(&core);
        harness.run_steps(3);
        harness.get_by_label("Open data folder").click();
        harness.run_steps(2);
        assert!(harness.state().2.contains(&ScreenEvent::OpenDataFolder));
    }

    #[test]
    fn the_export_buttons_ask_for_the_right_file_and_scope() {
        use egui_kittest::kittest::Queryable;
        let core = core(None);
        let mut harness = harness(&core);
        harness.run_steps(3);
        harness.get_by_label("Export JSON (everything)").click();
        harness.run_steps(2);
        harness.get_by_label("Export CSV for November 2026").click();
        harness.run_steps(2);
        harness.get_by_label("Export CSV (all time)").click();
        harness.run_steps(2);
        let events = harness.state().2.clone();
        let nov = crate::domain::date::YearMonth::new(2026, 11).unwrap();
        assert!(events.contains(&ScreenEvent::Export(ExportRequest::Json)));
        assert!(events.contains(&ScreenEvent::Export(ExportRequest::Csv(
            ExportScope::Month(nov)
        ))));
        assert!(events.contains(&ScreenEvent::Export(ExportRequest::Csv(ExportScope::All))));
    }
}
