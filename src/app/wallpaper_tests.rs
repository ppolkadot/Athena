//! The wallpaper through the whole app (contract §14.2): one shared live frame behind every main
//! screen, legible rendered samples over extreme wallpapers, and nothing decorative when paused.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};
use egui_kittest::Harness;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};

use super::actions::{AppAction, AppearanceApply};
use super::navigation::Screen;
use super::{AppCore, AthenaApp};
use crate::application::services::onboarding::OnboardingResult;
use crate::domain::appearance::{AppearancePrefs, Fit, MotionPref, ThemeMode, Wallpaper};
use crate::domain::money::Money;
use crate::domain::settings::{Language, Theme};
use crate::infrastructure::db::Db;
use crate::infrastructure::media::assets::{MediaStore, StagedMedia};
use crate::infrastructure::media::test_gif::{TestFrame, gif};
use crate::infrastructure::platform::{
    HighContrast, PlatformPreferences, PlatformPrefs, SystemColors,
};
use crate::ui::appearance::background::{composed_pixel, scrim_alpha};
use crate::ui::snapshot_tests::GPU;
use crate::ui::theme::{self, contrast_ratio, stock_palette};

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

fn solid_png(color: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::new();
    let pixels: Vec<u8> = (0..16 * 9)
        .flat_map(|_| [color[0], color[1], color[2], 255])
        .collect();
    PngEncoder::new(&mut out)
        .write_image(&pixels, 16, 9, ExtendedColorType::Rgba8)
        .unwrap();
    out
}

/// Frame 0 is red, frame 1 is blue, 100 ms each, looping forever.
fn two_frame_gif() -> Vec<u8> {
    gif(
        16,
        9,
        Some(0),
        &[TestFrame::full(16, 9, 0, 10), TestFrame::full(16, 9, 2, 10)],
    )
}

struct Rig {
    harness: Harness<'static, AthenaApp>,
    _root: tempfile::TempDir,
    t0: Instant,
    platform: Arc<Mutex<PlatformPrefs>>,
}

fn rig(
    bytes: Vec<u8>,
    prefs: impl FnOnce(Wallpaper) -> AppearancePrefs,
    size: egui::Vec2,
    ppp: f32,
) -> Rig {
    let root = tempfile::tempdir().unwrap();
    let mut core =
        AppCore::empty_for_test_in(Db::open_in_memory().unwrap(), root.path().to_path_buf());
    core.complete_onboarding(OnboardingResult {
        display_name: "A3".into(),
        partner_name: Some("Ana".into()),
        start_balance: Money::ZERO,
        language: Language::En,
    })
    .unwrap();
    let store = MediaStore::new(core.store.dirs());
    let staged: StagedMedia = store.stage(Arc::new(bytes)).unwrap();
    let prefs = prefs(Wallpaper::Local(staged.id.clone()));
    core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
        prefs,
        media: Some(staged),
    })))
    .unwrap();
    let platform = Arc::new(Mutex::new(supported()));
    let t0 = Instant::now();
    let mut app = AthenaApp::for_test_with_platform(core, Box::new(Fake(platform.clone())));
    app.pin_instant(t0);
    let harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(ppp)
        .with_step_dt(0.05)
        .with_context_setup(theme::typography::install_test_fonts)
        .build_ui_state(
            |ui, app: &mut AthenaApp| {
                ui.ctx()
                    .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                app.show(ui);
            },
            app,
        );
    let mut rig = Rig {
        harness,
        _root: root,
        t0,
        platform,
    };
    rig.wait_ready();
    rig
}

impl Rig {
    fn app(&self) -> &AthenaApp {
        self.harness.state()
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.harness.run_steps(2);
            let rt = self.app().wallpaper.as_ref();
            if rt.is_some_and(|rt| rt.is_ready(false)) {
                break;
            }
            assert!(Instant::now() < deadline, "wallpaper never decoded");
            std::thread::sleep(Duration::from_millis(5));
        }
        self.harness.run_steps(3);
    }

    fn at(&mut self, millis: u64) {
        self.harness
            .state_mut()
            .pin_instant(self.t0 + Duration::from_millis(millis));
        self.harness.run_steps(2);
    }

    fn pixel(&mut self, x_from_right: u32, y_from_bottom: u32) -> Color32 {
        let image = self.harness.render().unwrap();
        let (w, h) = image.dimensions();
        let p = image.get_pixel(w - x_from_right, h - y_from_bottom);
        Color32::from_rgb(p[0], p[1], p[2])
    }
}

fn close(a: Color32, b: Color32, tolerance: u8) -> bool {
    [(a.r(), b.r()), (a.g(), b.g()), (a.b(), b.b())]
        .iter()
        .all(|(x, y)| x.abs_diff(*y) <= tolerance)
}

fn go(rig: &mut Rig, screen: Screen) {
    rig.harness.state_mut().core.set_screen(screen);
    rig.harness.run_steps(3);
}

#[test]
fn one_live_gif_spans_every_main_screen_without_restarting() {
    let mut rig = rig(
        two_frame_gif(),
        |wallpaper| AppearancePrefs {
            wallpaper,
            motion: MotionPref::Full,
            ..AppearancePrefs::default()
        },
        egui::Vec2::new(1100.0, 760.0),
        1.0,
    );
    let rt = |r: &Rig| r.app().wallpaper.as_ref().unwrap().stats();
    let texture = rig
        .app()
        .wallpaper
        .as_ref()
        .unwrap()
        .texture_id(false)
        .unwrap();
    assert_eq!(rt(&rig).decodes_submitted, 1);
    let mut elapsed_before = Duration::ZERO;
    let mut now = 0;
    for screen in [
        Screen::Month,
        Screen::Calendar,
        Screen::Settings,
        Screen::Trash,
        Screen::Month,
    ] {
        go(&mut rig, screen);
        now += 90;
        rig.at(now);
        let elapsed = rig
            .app()
            .wallpaper
            .as_ref()
            .unwrap()
            .elapsed(false)
            .unwrap();
        assert!(
            elapsed >= elapsed_before,
            "{screen:?}: playback never moves backwards"
        );
        elapsed_before = elapsed;
        assert_eq!(
            rig.app().wallpaper.as_ref().unwrap().texture_id(false),
            Some(texture),
            "{screen:?}: the same texture backs every screen"
        );
    }
    // Navigation re-decoded nothing and re-allocated nothing; frames were rewritten in place.
    let stats = rt(&rig);
    assert_eq!((stats.decodes_submitted, stats.texture_allocations), (1, 1));
    assert!(stats.texture_writes >= 3, "frames advanced: {stats:?}");
    assert!(
        elapsed_before >= Duration::from_millis(400),
        "{elapsed_before:?}"
    );
}

#[test]
fn the_canvas_is_legible_over_black_white_and_saturated_wallpapers_at_two_scales() {
    let _turn = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (theme, mode) in [
        (Theme::Dark, ThemeMode::Dark),
        (Theme::Light, ThemeMode::Light),
    ] {
        let p = stock_palette(theme);
        for color in [
            [0, 0, 0],
            [255, 255, 255],
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [255, 255, 0],
        ] {
            for (size, ppp) in [
                (egui::Vec2::new(900.0, 640.0), 1.0),
                (egui::Vec2::new(640.0, 410.0), 2.0),
            ] {
                let mut rig = rig(
                    solid_png(color),
                    |wallpaper| AppearancePrefs {
                        theme_mode: mode,
                        wallpaper,
                        fit: Fit::Fill,
                        dim: 0,
                        motion: MotionPref::Reduced,
                        ..AppearancePrefs::default()
                    },
                    size,
                    ppp,
                );
                let expected = composed_pixel(
                    &p,
                    scrim_alpha(&p, 0),
                    Color32::from_rgb(color[0], color[1], color[2]),
                );
                // A canvas pixel near the bottom-right corner (the harness frames the window with an 8 px
                // border), away from cards and chrome.
                let sample = rig.pixel(20, 20);
                assert!(
                    close(sample, expected, 3),
                    "{theme:?} {color:?} scale {ppp}: rendered {sample:?} expected {expected:?}"
                );
                for (role, fg, needed) in [
                    ("text", p.text, 4.5),
                    ("muted", p.muted, 4.5),
                    ("accent", p.accent, 4.5),
                    ("focus", p.focus, 3.0),
                ] {
                    assert!(
                        contrast_ratio(fg, sample) >= needed,
                        "{theme:?} {role} over rendered {color:?} at {ppp}x"
                    );
                }
            }
        }
    }
}

#[test]
fn high_contrast_renders_the_plain_canvas_without_the_wallpaper() {
    let _turn = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut rig = rig(
        solid_png([255, 0, 0]),
        |wallpaper| AppearancePrefs {
            wallpaper,
            motion: MotionPref::Full,
            ..AppearancePrefs::default()
        },
        egui::Vec2::new(900.0, 640.0),
        1.0,
    );
    *rig.platform.lock().unwrap() = PlatformPrefs {
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
    };
    // Resume re-reads the platform; the next frames restyle.
    let blurred = crate::infrastructure::platform::WindowState {
        focused: false,
        visible: true,
    };
    let active = crate::infrastructure::platform::WindowState {
        focused: true,
        visible: true,
    };
    rig.harness.state().platform.update_window(blurred);
    rig.harness.state().platform.update_window(active);
    rig.harness.run_steps(4);
    let sample = rig.pixel(20, 20);
    assert_eq!(
        sample,
        Color32::BLACK,
        "the wallpaper is omitted, not dimmed"
    );
    assert_eq!(rig.app().look.wallpaper, Wallpaper::None);
}

#[test]
fn a_paused_wallpaper_asks_for_no_further_frames() {
    let mut rig = rig(
        two_frame_gif(),
        |wallpaper| AppearancePrefs {
            wallpaper,
            motion: MotionPref::Full,
            paused: true,
            ..AppearancePrefs::default()
        },
        egui::Vec2::new(900.0, 640.0),
        1.0,
    );
    let before = rig.app().wallpaper.as_ref().unwrap().stats();
    for step in 1..=30 {
        rig.at(step * 500);
    }
    assert_eq!(
        rig.app().wallpaper.as_ref().unwrap().stats().texture_writes,
        before.texture_writes
    );
    assert_eq!(
        rig.app().wallpaper.as_ref().unwrap().elapsed(false),
        Some(Duration::ZERO)
    );
}

/// Closing the window must release the textures and join the workers before the renderer drops.
#[test]
fn shutdown_releases_wallpaper_textures_and_workers_and_is_repeatable() {
    let mut rig = rig(
        solid_png([255, 0, 0]),
        |wallpaper| AppearancePrefs {
            wallpaper,
            motion: MotionPref::Full,
            ..AppearancePrefs::default()
        },
        egui::Vec2::new(900.0, 640.0),
        1.0,
    );
    assert_eq!(
        rig.app().wallpaper.as_ref().unwrap().live_textures(),
        1,
        "the wallpaper is bound before shutdown"
    );
    rig.harness.state_mut().shutdown();
    assert!(
        rig.app().wallpaper.is_none(),
        "runtime and textures dropped"
    );
    rig.harness.state_mut().shutdown();
    // A frame after shutdown must not panic or rebuild anything that blocks.
    rig.harness.run_steps(1);
}
