//! Pixel snapshots of every screen in English/dark and Portuguese/light. They guard the visual
//! design against accidental change.
//!
//! The harness renders with egui's bundled fonts, a pinned date and simulated time, so output does
//! not depend on installed fonts, the clock or the window system. A small pixel tolerance absorbs
//! GPU differences. To accept an intended change run
//! `UPDATE_SNAPSHOTS=1 cargo test snapshot` and review the images in `src/ui/snapshots`.

use egui_kittest::{Harness, SnapshotOptions, SnapshotResults};

use crate::app::navigation::Screen;
use crate::app::{AppCore, AthenaApp};
use crate::application::services::onboarding::OnboardingResult;
use crate::domain::money::Money;
use crate::domain::settings::{Language, Profile, Theme};
use crate::infrastructure::db::Db;
use crate::shared::test_fixtures::{seed_month, today};
use crate::ui::screens::onboarding::OnboardingState;
use crate::ui::theme;

use eframe::egui;

/// A fully set-up app, optionally with November's sample data.
fn onboarded_core(language: Language, theme: Theme, with_data: bool) -> AppCore {
    let db = Db::open_in_memory().unwrap();
    if with_data {
        seed_month(&db);
    }
    let mut core = AppCore::empty_for_test(db);
    if !with_data {
        core.complete_onboarding(OnboardingResult {
            display_name: "A3".into(),
            partner_name: Some("Ana".into()),
            start_balance: Money::ZERO,
            language,
        })
        .unwrap();
    }
    core.save_profile(Profile {
        start_balance: Money::ZERO,
        theme,
        language,
        onboarding_complete: true,
    })
    .unwrap();
    core
}

/// GPU rendering is not safe to run from several test threads at once (it crashes some drivers),
/// so snapshot tests take turns.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn snapshot(results: &mut SnapshotResults, name: &str, mut app: AthenaApp, size: egui::Vec2) {
    let _turn = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Time is simulated; the same number of steps always draws the same frame.
    let mut harness = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut AthenaApp| {
            ui.ctx()
                .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
            theme::apply(ui.ctx(), app.theme_for_test());
            let style = ui.ctx().global_style();
            ui.set_style(style);
            app.show_for_test(ui);
        },
        {
            app.pin_today(today());
            app
        },
    );
    harness.run_steps(6);
    let options = SnapshotOptions::new()
        .output_path("src/ui/snapshots")
        .max_failed_pixels(150);
    harness.snapshot_options(name, &options);
    results.extend_harness(&mut harness);
}

const SIZE: egui::Vec2 = egui::Vec2::new(1100.0, 760.0);

fn variants() -> [(Language, Theme, &'static str); 2] {
    [
        (Language::En, Theme::Dark, "en_dark"),
        (Language::PtBr, Theme::Light, "pt_light"),
    ]
}

#[test]
fn snapshot_onboarding() {
    let mut results = SnapshotResults::new();
    for (language, tag) in [(Language::En, "en_dark"), (Language::PtBr, "pt_dark")] {
        let core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        let mut app = AthenaApp::for_test(core);
        app.set_onboarding_for_test(OnboardingState::new(language));
        snapshot(&mut results, &format!("onboarding_{tag}"), app, SIZE);
    }
    results.unwrap();
}

#[test]
fn snapshot_empty_month() {
    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        let app = AthenaApp::for_test(onboarded_core(language, theme, false));
        snapshot(&mut results, &format!("month_empty_{tag}"), app, SIZE);
    }
    results.unwrap();
}

#[test]
fn snapshot_populated_month() {
    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        let app = AthenaApp::for_test(onboarded_core(language, theme, true));
        snapshot(&mut results, &format!("month_populated_{tag}"), app, SIZE);
    }
    results.unwrap();
}

#[test]
fn snapshot_calendar() {
    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        let mut core = onboarded_core(language, theme, true);
        core.set_screen(Screen::Calendar);
        snapshot(
            &mut results,
            &format!("calendar_{tag}"),
            AthenaApp::for_test(core),
            SIZE,
        );
    }
    results.unwrap();
}

#[test]
fn snapshot_settings() {
    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        let mut core = onboarded_core(language, theme, false);
        core.set_screen(Screen::Settings);
        snapshot(
            &mut results,
            &format!("settings_{tag}"),
            AthenaApp::for_test(core),
            SIZE,
        );
    }
    results.unwrap();
}
