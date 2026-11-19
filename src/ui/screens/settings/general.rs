//! General settings card: owner, partner, starting balance, theme, and language.

use eframe::egui::{self, Button, RichText};

use crate::app::actions::AppAction;
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::settings::{Language, Profile};
use crate::i18n::strings;
use crate::ui::components::money_input;
use crate::ui::screens::ScreenEvent;
use crate::ui::theme::palette;

use super::{card, error_message, labeled_edit};

pub(super) fn profile_card(
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
        ui.label(RichText::new(s.ap_general_link_body).color(p.muted));
        if ui.button(s.ap_open).clicked() {
            settings.page = super::SettingsPage::Appearance;
        }
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
