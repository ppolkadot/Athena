//! Injected platform preferences driving the whole app (contract §14.2): supported, missing,
//! high contrast and focus/resume transitions while a preview, a paused wallpaper and a draft exist.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;

use super::actions::{AppAction, AppearanceApply};
use super::{AppCore, AthenaApp};
use crate::application::services::onboarding::OnboardingResult;
use crate::domain::appearance::{Accent, AppearancePrefs, MotionPref, TextScale, Wallpaper};
use crate::domain::money::Money;
use crate::domain::settings::Language;
use crate::infrastructure::db::Db;
use crate::infrastructure::platform::{
    HighContrast, PlatformPreferences, PlatformPrefs, SystemColors, WindowState,
};
use crate::ui::theme::effective::Material;
use crate::ui::theme::{self, contrast_ratio};

struct Fake(Rc<Cell<PlatformPrefs>>);

impl PlatformPreferences for Fake {
    fn read(&self) -> PlatformPrefs {
        self.0.get()
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

fn app(platform: &Rc<Cell<PlatformPrefs>>) -> Harness<'static, AthenaApp> {
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
fn supported_platform_state_gives_tint_motion_and_the_chosen_accent() {
    let _reset = theme::active::ResetOnDrop;
    let cell = Rc::new(Cell::new(supported()));
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
    let cell = Rc::new(Cell::new(PlatformPrefs::default()));
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
    let cell = Rc::new(Cell::new(supported()));
    let mut h = app(&cell);
    // An open draft owns focus; a preview and a paused wallpaper are in play.
    h.key_press_modifiers(Modifiers::COMMAND, Key::N);
    h.run_steps(14);
    assert!(h.state().quick_add.is_some());
    let focused = h.ctx.memory(egui::Memory::focused);
    assert!(focused.is_some());
    h.state_mut().appearance_preview = Some(AppearancePrefs {
        text_scale: TextScale::P150,
        ..AppearancePrefs::default()
    });
    h.run_steps(3);
    assert_eq!(h.state().look.text_scale, 150);
    let before_style = h.ctx.global_style().visuals.panel_fill;

    // The user turns high contrast on while the window is in the background.
    cell.set(high_contrast());
    let t = Instant::now() + Duration::from_secs(10);
    let blurred = WindowState {
        focused: false,
        visible: true,
    };
    let active = WindowState {
        focused: true,
        visible: true,
    };
    h.state_mut().platform.poll(t, blurred);
    let (changed, _) = h.state_mut().platform.poll(t, active);
    assert!(changed, "resume re-reads the platform");
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
    h.state_mut().appearance_preview = None;
    cell.set(supported());
    let later = t + Duration::from_secs(5);
    h.state_mut().platform.poll(later, blurred);
    h.state_mut().platform.poll(later, active);
    h.run_steps(3);
    assert!(!h.state().look.high_contrast);
    assert_eq!(h.state().look.wallpaper, Wallpaper::Builtin);
}
