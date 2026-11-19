//! Pixel snapshots of Settings → Appearance and the wallpaper shell (contract §14.2): solid and a
//! fixed bright wallpaper frame, in English/dark and Português/light. To accept an intended change
//! run `UPDATE_SNAPSHOTS=1 cargo test appearance_snapshot` and review `src/ui/snapshots`.

use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::{Harness, SnapshotOptions, SnapshotResults};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};

use crate::app::actions::{AppAction, AppearanceApply};
use crate::app::navigation::Screen;
use crate::app::{AppCore, AthenaApp};
use crate::domain::appearance::{AppearancePrefs, Fit, MotionPref, Preset, Wallpaper};
use crate::domain::settings::{Language, Theme};
use crate::infrastructure::db::Db;
use crate::infrastructure::media::assets::MediaStore;
use crate::infrastructure::platform::{FixedPlatform, HighContrast, PlatformPrefs};
use crate::shared::test_fixtures::today;
use crate::ui::screens::settings::SettingsPage;
use crate::ui::snapshot_tests::{GPU, SIZE, variants};
use crate::ui::theme;

/// A bright, fixed wallpaper: a pastel diagonal gradient that stresses the scrim.
#[allow(clippy::many_single_char_names)]
fn bright_png() -> Vec<u8> {
    let (w, h) = (160_u32, 90_u32);
    let mut pixels = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let r = 200 + (x * 55 / (w - 1));
            let g = 215 + (y * 40 / (h - 1));
            let b = 160 + ((x + y) * 95 / (w + h - 2));
            pixels.extend_from_slice(&[
                u8::try_from(r).unwrap(),
                u8::try_from(g).unwrap(),
                u8::try_from(b).unwrap(),
                255,
            ]);
        }
    }
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(&pixels, w, h, ExtendedColorType::Rgba8)
        .unwrap();
    out
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

/// A core in its own data folder, optionally with the bright wallpaper applied.
fn core_with(
    language: Language,
    theme_choice: Theme,
    root: &std::path::Path,
    wallpaper: bool,
) -> AppCore {
    let mut core = AppCore::empty_for_test_in(Db::open_in_memory().unwrap(), root.to_path_buf());
    core.complete_onboarding(crate::application::services::onboarding::OnboardingResult {
        display_name: "A3".into(),
        partner_name: Some("Ana".into()),
        start_balance: crate::domain::money::Money::ZERO,
        language,
    })
    .unwrap();
    let mut profile = core.vm.profile.clone().unwrap();
    profile.theme = theme_choice;
    profile.language = language;
    core.save_profile(profile).unwrap();
    if wallpaper {
        let store = MediaStore::new(core.store.dirs());
        let staged = store.stage(std::sync::Arc::new(bright_png())).unwrap();
        let prefs = AppearancePrefs {
            wallpaper: Wallpaper::Local(staged.id.clone()),
            fit: Fit::Fill,
            dim: 0,
            motion: MotionPref::Reduced,
            ..AppearancePrefs::default()
        }
        .with_preset(if theme_choice == Theme::Dark {
            Preset::Onyx
        } else {
            Preset::Pearl
        });
        core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
            prefs: AppearancePrefs {
                motion: MotionPref::Reduced,
                ..prefs
            },
            media: Some(staged),
        })))
        .unwrap();
    }
    core
}

fn snapshot(results: &mut SnapshotResults, name: &str, mut app: AthenaApp, size: egui::Vec2) {
    let _turn = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    app.pin_today(today());
    let t0 = Instant::now();
    app.pin_instant(t0);
    let mut harness = Harness::builder()
        .with_size(size)
        .with_context_setup(theme::typography::install_test_fonts)
        .build_ui_state(
            |ui, app: &mut AthenaApp| {
                ui.ctx()
                    .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                app.show_for_test(ui);
            },
            app,
        );
    // Wait for the (asynchronous) wallpaper decode, then settle the frame.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.run_steps(2);
        if harness.state().wallpaper_settled_for_test() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "wallpaper never finished loading"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.run_steps(6);
    let options = SnapshotOptions::new()
        .output_path("src/ui/snapshots")
        .max_failed_pixels(150);
    harness.snapshot_options(name, &options);
    results.extend_harness(&mut harness);
}

fn appearance_page(core: &mut AppCore) {
    core.ui.settings.page = SettingsPage::Appearance;
    core.set_screen(Screen::Settings);
}

fn app(core: AppCore) -> AthenaApp {
    AthenaApp::for_test_with_platform(core, Box::new(FixedPlatform(supported())))
}

#[test]
fn snapshot_settings_appearance() {
    let mut results = SnapshotResults::new();
    for (language, theme_choice, tag) in variants() {
        // Solid: the default look, so nothing is drawn behind the chrome.
        let root = tempfile::tempdir().unwrap();
        let mut core = core_with(language, theme_choice, root.path(), false);
        appearance_page(&mut core);
        snapshot(
            &mut results,
            &format!("settings_appearance_solid_{tag}"),
            app(core),
            SIZE,
        );
        // A fixed bright wallpaper frame behind opaque cards and a translucent sidebar.
        let root = tempfile::tempdir().unwrap();
        let mut core = core_with(language, theme_choice, root.path(), true);
        appearance_page(&mut core);
        snapshot(
            &mut results,
            &format!("settings_appearance_wallpaper_{tag}"),
            app(core),
            SIZE,
        );
    }
    results.unwrap();
}

#[test]
fn snapshot_shell_with_wallpaper() {
    let mut results = SnapshotResults::new();
    for (language, theme_choice, tag) in variants() {
        let root = tempfile::tempdir().unwrap();
        let mut core = core_with(language, theme_choice, root.path(), true);
        core.set_screen(Screen::Month);
        snapshot(
            &mut results,
            &format!("shell_wallpaper_{tag}"),
            app(core),
            SIZE,
        );
    }
    results.unwrap();
}
