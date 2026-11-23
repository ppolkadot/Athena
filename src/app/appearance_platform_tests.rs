//! Injected platform preferences driving the whole app (contract §14.2): supported, missing,
//! high contrast and focus/resume transitions while a preview, a paused wallpaper and a draft exist.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;

use super::actions::{AppAction, AppearanceApply};
use super::appearance_edit::AppearanceEdit;
use super::{AppCore, AthenaApp};
use crate::application::services::onboarding::OnboardingResult;
use crate::domain::appearance::{Accent, AppearancePrefs, MotionPref, TextScale, Wallpaper};
use crate::domain::money::Money;
use crate::domain::settings::Language;
use crate::infrastructure::db::Db;
use crate::infrastructure::platform::{
    HighContrast, PlatformPreferences, PlatformPrefs, SystemColors, WindowState,
};
use crate::ui::screens::settings::SettingsPage;
use crate::ui::theme::effective::Material;
use crate::ui::theme::{self, contrast_ratio};

struct Fake(Arc<Mutex<PlatformPrefs>>);

impl PlatformPreferences for Fake {
    fn read(&self) -> PlatformPrefs {
        *self.0.lock().unwrap()
    }
}

fn supported() -> PlatformPrefs {
    PlatformPrefs {
        animations: Some(true),
        transparency: Some(true),
        high_contrast: Some(HighContrast {
            on: false,
            colors: None,
        }),
    }
}

fn high_contrast() -> PlatformPrefs {
    PlatformPrefs {
        high_contrast: Some(HighContrast {
            on: true,
            colors: Some(SystemColors {
                window: [0, 0, 0],
                window_text: [255, 255, 255],
                highlight: [26, 235, 255],
                highlight_text: [0, 0, 0],
            }),
        }),
        ..supported()
    }
}

fn app(platform: &Arc<Mutex<PlatformPrefs>>) -> Harness<'static, AthenaApp> {
    let mut core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
    core.complete_onboarding(OnboardingResult {
        display_name: "A3".into(),
        partner_name: Some("Ana".into()),
        start_balance: Money::ZERO,
        language: Language::En,
    })
    .unwrap();
    core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
        prefs: AppearancePrefs {
            accent: Accent::Violet,
            text_scale: TextScale::P130,
            motion: MotionPref::Full,
            wallpaper: Wallpaper::Builtin,
            paused: true,
            ..AppearancePrefs::default()
        },
        media: None,
    })))
    .unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::Vec2::new(1280.0, 900.0))
        .with_step_dt(0.05)
        .with_context_setup(theme::typography::install_test_fonts)
        .build_ui_state(
            |ui, app: &mut AthenaApp| {
                ui.ctx()
                    .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                app.show(ui);
            },
            AthenaApp::for_test_with_platform(core, Box::new(Fake(platform.clone()))),
        );
    harness.run_steps(6);
    harness
}

#[test]
fn focused_preference_refresh_does_not_schedule_a_frame() {
    let _reset = theme::active::ResetOnDrop;
    let cell = Arc::new(Mutex::new(supported()));
    let mut h = app(&cell);
    let ctx = egui::Context::default();
    for pass in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                focused: true,
                ..Default::default()
            },
            |_| {
                h.state_mut().refresh_appearance(&ctx);
            },
        );
        output.textures_delta.clear();
        if pass == 2 {
            assert_eq!(
                output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                Duration::MAX,
                "unchanged platform preferences should never schedule a focused repaint"
            );
        }
    }
}

#[test]
fn supported_platform_state_gives_tint_motion_and_the_chosen_accent() {
    let _reset = theme::active::ResetOnDrop;
    let cell = Arc::new(Mutex::new(supported()));
    let h = app(&cell);
    let look = &h.state().look;
    assert!(!look.reduced_motion && look.material == Material::Tint);
    assert_eq!(look.text_scale, 130);
    assert_eq!(
        look.palette.accent,
        theme::effective::accent_palette(look.theme, Accent::Violet).accent
    );
}

#[test]
fn missing_platform_state_falls_back_to_solid_and_reduced() {
    let _reset = theme::active::ResetOnDrop;
    let cell = Arc::new(Mutex::new(PlatformPrefs::default()));
    let h = app(&cell);
    let look = &h.state().look;
    // The user chose Full motion, so unknown OS state does not take it away; chrome goes solid.
    assert!(!look.reduced_motion);
    assert_eq!(look.material, Material::Solid);
    assert!(look.transparency_forced);
}

#[test]
fn high_contrast_resume_restyles_while_previewing_and_paused_and_keeps_focus() {
    let _reset = theme::active::ResetOnDrop;
    let cell = Arc::new(Mutex::new(supported()));
    let mut h = app(&cell);
    // An open draft owns focus; a preview and a paused wallpaper are in play.
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(14);
    assert!(h.state().quick_add.is_some());
    let focused = h.ctx.memory(egui::Memory::focused);
    assert!(focused.is_some());
    // An open, unapplied Appearance draft previews live.
    let mut edit = AppearanceEdit::start(&h.state().core.vm);
    edit.draft.text_scale = TextScale::P150;
    edit.draft.wallpaper = Wallpaper::None;
    h.state_mut().core.ui.settings.appearance = Some(edit);
    h.state_mut().core.ui.settings.page = SettingsPage::Appearance;
    h.state_mut().core.ui.screen = crate::app::navigation::Screen::Settings;
    h.run_steps(3);
    assert_eq!(h.state().look.text_scale, 150);
    let before_style = h.ctx.global_style().visuals.panel_fill;

    // The user turns high contrast on while the window is in the background.
    *cell.lock().unwrap() = high_contrast();
    let blurred = WindowState {
        focused: false,
        visible: true,
    };
    let active = WindowState {
        focused: true,
        visible: true,
    };
    h.state().platform.update_window(blurred);
    h.state().platform.update_window(active);
    assert_eq!(h.state().platform.current(), high_contrast());
    h.run_steps(3);

    let look = h.state().look.clone();
    assert!(look.high_contrast && look.reduced_motion);
    assert_eq!(look.wallpaper, Wallpaper::None, "wallpaper is omitted");
    assert_eq!(look.text_scale, 150, "preview still applies its scale");
    let panel = h.ctx.global_style().visuals.panel_fill;
    assert_ne!(panel, before_style);
    assert_eq!(panel, look.palette.bg);
    for fg in [
        look.palette.text,
        look.palette.positive,
        look.palette.negative,
    ] {
        assert!(contrast_ratio(fg, panel) >= 4.5, "numbers stay legible");
    }
    // The applied pause and the draft survive, and the same widget keeps focus.
    assert!(h.state().core.vm.appearance.as_ref().unwrap().paused);
    assert!(h.state().quick_add.is_some());
    assert_eq!(h.ctx.memory(egui::Memory::focused), focused);

    // Turning it back off (and closing the preview) restores the chosen look.
    h.state_mut().core.ui.settings.appearance = None;
    h.state_mut().core.ui.settings.page = SettingsPage::General;
    *cell.lock().unwrap() = supported();
    h.state().platform.update_window(blurred);
    h.state().platform.update_window(active);
    h.run_steps(3);
    assert!(!h.state().look.high_contrast);
    assert_eq!(h.state().look.wallpaper, Wallpaper::Builtin);
}
