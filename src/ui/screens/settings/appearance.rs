//! Settings → Appearance (contract §14.2): theme and presets, accent/density/text size, background
//! and accessibility, with a live shell preview and Apply / Cancel / Reset below a scrolling form.

use eframe::egui::{self, Button, Color32, RichText, ScrollArea, Sense, Stroke, Vec2};

use crate::app::appearance_edit::AppearanceEdit;
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::domain::appearance::{
    Accent, Density, Fit, MotionPref, Preset, Quality, TextScale, ThemeMode, Wallpaper,
};
use crate::domain::settings::Theme;
use crate::i18n::{Strings, media_error_message, strings};
use crate::ui::appearance::background::min_dim_percent;
use crate::ui::screens::ScreenEvent;
use crate::ui::theme::effective::accent_palette;
use crate::ui::theme::{Palette, palette};

use super::{SettingsPage, card};

/// Space kept below the scrolling form for the title, tabs, status and the action row.
const CHROME_HEIGHT: f32 = 330.0;

/// One group of mutually exclusive choices under a small caption.
fn choice_row<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    p: &Palette,
    caption: &str,
    items: &[(T, &str)],
    current: &mut T,
) -> bool {
    let mut changed = false;
    ui.label(
        RichText::new(caption)
            .text_style(egui::TextStyle::Small)
            .color(p.muted),
    );
    ui.horizontal_wrapped(|ui| {
        for &(value, label) in items {
            if ui.selectable_label(*current == value, label).clicked() && *current != value {
                *current = value;
                changed = true;
            }
        }
    });
    ui.add_space(8.0);
    changed
}

fn swatch(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 6.0, color);
    ui.painter()
        .circle_stroke(rect.center(), 6.0, Stroke::new(1.0, Color32::GRAY));
}

fn theme_section(ui: &mut egui::Ui, p: &Palette, s: &Strings, edit: &mut AppearanceEdit) {
    card(ui, p, s.ap_sec_theme, |ui| {
        choice_row(
            ui,
            p,
            s.theme,
            &[
                (ThemeMode::System, s.ap_system),
                (ThemeMode::Dark, s.dark),
                (ThemeMode::Light, s.light),
            ],
            &mut edit.draft.theme_mode,
        );
        ui.label(
            RichText::new(s.ap_preset)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        ui.horizontal_wrapped(|ui| {
            for (preset, label) in [
                (Preset::Onyx, s.ap_preset_onyx),
                (Preset::Pearl, s.ap_preset_pearl),
                (Preset::Aurora, s.ap_preset_aurora),
            ] {
                if ui
                    .selectable_label(edit.draft.preset == preset, label)
                    .clicked()
                {
                    edit.draft = edit.draft.clone().with_preset(preset);
                }
            }
            if !edit.draft.matches_preset_recipe() {
                ui.label(RichText::new(s.ap_customized).color(p.muted));
            }
        });
    });
}

fn look_section(ui: &mut egui::Ui, p: &Palette, s: &Strings, edit: &mut AppearanceEdit) {
    card(ui, p, s.ap_sec_look, |ui| {
        ui.label(
            RichText::new(s.ap_accent)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        let theme = if p.bg.r() > 128 {
            Theme::Light
        } else {
            Theme::Dark
        };
        ui.horizontal_wrapped(|ui| {
            for (accent, label) in [
                (Accent::Blue, s.ap_accent_blue),
                (Accent::Teal, s.ap_accent_teal),
                (Accent::Violet, s.ap_accent_violet),
            ] {
                swatch(ui, accent_palette(theme, accent).accent);
                if ui
                    .selectable_label(edit.draft.accent == accent, label)
                    .clicked()
                {
                    edit.draft.accent = accent;
                }
            }
        });
        ui.add_space(8.0);
        choice_row(
            ui,
            p,
            s.ap_density,
            &[
                (Density::Comfortable, s.ap_density_comfortable),
                (Density::Compact, s.ap_density_compact),
            ],
            &mut edit.draft.density,
        );
        let labels: Vec<(TextScale, String)> = TextScale::ALL
            .iter()
            .map(|scale| {
                (
                    *scale,
                    s.ap_percent
                        .replace("{value}", &scale.percent().to_string()),
                )
            })
            .collect();
        let items: Vec<(TextScale, &str)> = labels.iter().map(|(k, v)| (*k, v.as_str())).collect();
        choice_row(ui, p, s.ap_text_scale, &items, &mut edit.draft.text_scale);
    });
}

fn background_section(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &Strings,
    language: crate::domain::settings::Language,
    edit: &mut AppearanceEdit,
    events: &mut Vec<ScreenEvent>,
) {
    card(ui, p, s.ap_sec_background, |ui| {
        let is_local = matches!(edit.draft.wallpaper, Wallpaper::Local(_));
        ui.horizontal_wrapped(|ui| {
            if ui
                .selectable_label(edit.draft.wallpaper == Wallpaper::None, s.ap_bg_none)
                .clicked()
            {
                edit.choose(Wallpaper::None);
            }
            if ui
                .selectable_label(edit.draft.wallpaper == Wallpaper::Builtin, s.ap_bg_builtin)
                .clicked()
            {
                edit.choose(Wallpaper::Builtin);
            }
            if ui.selectable_label(is_local, s.ap_bg_file).clicked() && !is_local {
                if matches!(edit.baseline.wallpaper, Wallpaper::Local(_)) {
                    edit.choose(edit.baseline.wallpaper.clone());
                } else {
                    events.push(ScreenEvent::PickWallpaper);
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let label = if is_local {
                s.ap_replace_file
            } else {
                s.ap_choose_file
            };
            if ui.button(label).clicked() {
                events.push(ScreenEvent::PickWallpaper);
            }
            if edit.draft.wallpaper != Wallpaper::None && ui.button(s.ap_remove).clicked() {
                edit.choose(Wallpaper::None);
            }
        });
        // Why the wallpaper is not (yet) usable stays next to the control.
        if let Some(error) = edit.pick_error.or(edit.status.failure) {
            ui.label(RichText::new(media_error_message(error, language)).color(p.negative));
        } else if edit.status.loading {
            ui.label(RichText::new(s.ap_file_loading).color(p.muted));
        } else if is_local && edit.staged.is_some() {
            ui.label(RichText::new(s.ap_file_ready).color(p.positive));
        }
        if let Some(error) = edit.apply_error {
            ui.label(RichText::new(media_error_message(error, language)).color(p.negative));
        }
        ui.add_space(8.0);
        if edit.draft.wallpaper != Wallpaper::None {
            choice_row(
                ui,
                p,
                s.ap_fit,
                &[(Fit::Fill, s.ap_fit_fill), (Fit::Fit, s.ap_fit_fit)],
                &mut edit.draft.fit,
            );
            let min = min_dim_percent(p);
            let mut dim = edit.draft.dim.max(min);
            let summary = s
                .ap_dim_value
                .replace("{value}", &dim.to_string())
                .replace("{min}", &min.to_string());
            let slider = egui::Slider::new(&mut dim, min..=100)
                .text(s.ap_dim)
                .custom_formatter(move |v, _| s.ap_percent.replace("{value}", &format!("{v:.0}")));
            let response = ui.add(slider);
            if response.changed() {
                edit.draft.dim = dim;
            }
            ui.label(
                RichText::new(summary)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            let pause = if edit.draft.paused {
                s.ap_resume
            } else {
                s.ap_pause
            };
            if ui.button(pause).clicked() {
                edit.draft.paused = !edit.draft.paused;
            }
        }
    });
}

fn accessibility_section(ui: &mut egui::Ui, p: &Palette, s: &Strings, edit: &mut AppearanceEdit) {
    card(ui, p, s.ap_sec_access, |ui| {
        choice_row(
            ui,
            p,
            s.ap_motion,
            &[
                (MotionPref::System, s.ap_motion_system),
                (MotionPref::Reduced, s.ap_motion_reduced),
                (MotionPref::Full, s.ap_motion_full),
            ],
            &mut edit.draft.motion,
        );
        let status = edit.status;
        let note = if status.high_contrast {
            s.ap_motion_now_hc
        } else if !status.reduced_motion {
            s.ap_motion_now_full
        } else if status.motion_forced_by_os {
            s.ap_motion_now_os
        } else {
            s.ap_motion_now_user
        };
        ui.label(
            RichText::new(note)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        ui.add_space(8.0);
        ui.checkbox(
            &mut edit.draft.reduce_transparency,
            s.ap_reduce_transparency,
        );
        let transparency_note = if status.high_contrast {
            Some(s.ap_transparency_now_hc)
        } else if edit.draft.reduce_transparency {
            Some(s.ap_transparency_now_user)
        } else if status.transparency_forced {
            Some(s.ap_transparency_now_os)
        } else {
            None
        };
        if let Some(note) = transparency_note {
            ui.label(
                RichText::new(note)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
        }
        ui.add_space(8.0);
        choice_row(
            ui,
            p,
            s.ap_quality,
            &[
                (Quality::Auto, s.ap_quality_auto),
                (Quality::Low, s.ap_quality_low),
                (Quality::High, s.ap_quality_high),
            ],
            &mut edit.draft.quality,
        );
    });
}

fn footer(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &Strings,
    edit: &mut AppearanceEdit,
    events: &mut Vec<ScreenEvent>,
) {
    ui.separator();
    let status = if edit.status.loading {
        s.ap_status_loading
    } else if edit.is_dirty() {
        s.ap_status_preview
    } else {
        s.ap_status_clean
    };
    ui.label(RichText::new(status).color(p.muted));
    if edit.apply_failed {
        ui.label(RichText::new(s.ap_apply_failed).color(p.negative));
    }
    ui.horizontal_wrapped(|ui| {
        let apply = Button::new(RichText::new(s.ap_apply).color(p.on_accent).strong())
            .fill(p.accent)
            .min_size(Vec2::new(96.0, crate::ui::tokens::control_height()));
        if ui.add_enabled(edit.can_apply(), apply).clicked() {
            events.push(ScreenEvent::ApplyAppearance);
        }
        if ui
            .add_enabled(edit.is_dirty(), Button::new(s.cancel))
            .clicked()
        {
            edit.cancel();
        }
        if ui.button(s.ap_reset).clicked() {
            events.push(ScreenEvent::ResetAppearance);
        }
    });
}

/// The Appearance page. The edit session starts the first time the page is shown.
pub(super) fn appearance_page(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    ui_state: &mut UiState,
    events: &mut Vec<ScreenEvent>,
) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    debug_assert_eq!(ui_state.settings.page, SettingsPage::Appearance);
    let edit = ui_state
        .settings
        .appearance
        .get_or_insert_with(|| AppearanceEdit::start(vm));
    if vm.appearance_unreadable {
        ui.label(RichText::new(s.appearance_unreadable_notice).color(p.warning));
        ui.add_space(8.0);
    }
    let form_height = (ui.ctx().content_rect().height() - CHROME_HEIGHT).max(200.0);
    ScrollArea::vertical()
        .id_salt("appearance-form")
        .max_height(form_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            theme_section(ui, &p, s, edit);
            look_section(ui, &p, s, edit);
            background_section(ui, &p, s, vm.language(), edit, events);
            accessibility_section(ui, &p, s, edit);
        });
    footer(ui, &p, s, edit, events);
}
