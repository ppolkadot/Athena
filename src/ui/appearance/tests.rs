//! Playback clock and wallpaper runtime tests with pinned monotonic time (contract §14.2).

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;

use super::*;
use crate::domain::appearance::{AppearancePrefs, MotionPref};
use crate::domain::settings::Theme;
use crate::infrastructure::config::AppDirs;
use crate::infrastructure::media::test_gif::{TestFrame, gif};
use crate::infrastructure::platform::{HighContrast, PlatformPrefs};
use crate::ui::theme::effective::resolve;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn playback(delays: &[u32], repeat: Repeat) -> Playback {
    Playback::new(Timeline::new(delays, repeat))
}

#[test]
fn frames_follow_elapsed_time_and_cumulative_delays() {
    let t = Timeline::new(&[100, 200, 100], Repeat::Plays(1));
    assert_eq!(t.frame_at(0), (0, false, Some(100_000)));
    assert_eq!(t.frame_at(99_999).0, 0);
    assert_eq!(t.frame_at(100_000), (1, false, Some(200_000)));
    assert_eq!(t.frame_at(299_999).0, 1);
    assert_eq!(t.frame_at(300_000).0, 2);
    assert_eq!(t.frame_at(399_999), (2, false, Some(1)));
    // One play-through holds the last frame, then asks for nothing more.
    assert_eq!(t.frame_at(400_000), (2, true, None));
    assert_eq!(t.frame_at(10_000_000), (2, true, None));
    // A single frame is static.
    assert_eq!(
        Timeline::new(&[100], Repeat::Infinite).frame_at(5),
        (0, true, None)
    );
}

#[test]
fn finite_repeats_and_infinite_loops() {
    let two = Timeline::new(&[100, 100], Repeat::Plays(2));
    assert_eq!(
        two.frame_at(399_999),
        (1, false, Some(1)),
        "second play still running"
    );
    assert_eq!(two.frame_at(400_000), (1, true, None));
    let forever = Timeline::new(&[100, 100], Repeat::Infinite);
    assert_eq!(forever.frame_at(200_000).0, 0, "wraps to the first frame");
    assert!(!forever.frame_at(10_000_000_000).1);
    // Zero plays (never produced by the decoder) degrade to one.
    assert!(
        Timeline::new(&[100, 100], Repeat::Plays(0))
            .frame_at(200_000)
            .1
    );
}

#[test]
fn a_running_clock_wakes_only_for_the_next_frame_boundary() {
    let t0 = Instant::now();
    let mut p = playback(&[100, 100, 100], Repeat::Infinite);
    let first = p.step(t0, Mode::Run);
    assert_eq!(
        first,
        Step {
            frame: 0,
            upload: true,
            wake: Some(ms(100))
        }
    );
    // Nothing is uploaded or requested before the boundary.
    let mid = p.step(t0 + ms(40), Mode::Run);
    assert_eq!((mid.frame, mid.upload), (0, false));
    assert_eq!(mid.wake, Some(ms(60)));
    let next = p.step(t0 + ms(100), Mode::Run);
    assert_eq!(
        (next.frame, next.upload, next.wake),
        (1, true, Some(ms(100)))
    );
}

#[test]
fn uploads_are_capped_near_30_hz_and_superseded_frames_are_skipped() {
    let t0 = Instant::now();
    let mut p = playback(&[20; 10], Repeat::Infinite);
    let mut uploads = Vec::new();
    for tick in 0..=50 {
        let now = t0 + ms(tick * 20);
        let step = p.step(now, Mode::Run);
        if step.upload {
            uploads.push((tick * 20, step.frame));
        }
    }
    // Over one second at most 30 uploads (floor 33.34 ms), and every gap respects the floor.
    assert!(uploads.len() <= 30, "{} uploads", uploads.len());
    for pair in uploads.windows(2) {
        assert!(Duration::from_millis(pair[1].0 - pair[0].0) >= UPLOAD_FLOOR);
    }
    let frames: Vec<_> = uploads.iter().map(|u| u.1).collect();
    assert!(
        frames.windows(2).any(|w| (w[1] + 10 - w[0]) % 10 > 1),
        "some intermediate frames were skipped: {frames:?}"
    );
    // The wake after a suppressed frame is the remaining floor, never a tight loop.
    let mut p = playback(&[20; 10], Repeat::Infinite);
    p.step(t0, Mode::Run);
    let held = p.step(t0 + ms(20), Mode::Run);
    assert!(!held.upload);
    assert_eq!(held.wake, Some(UPLOAD_FLOOR.saturating_sub(ms(20))));
}

#[test]
fn pause_blur_and_hidden_freeze_without_catch_up() {
    let t0 = Instant::now();
    let mut p = playback(&[100; 5], Repeat::Infinite);
    p.step(t0, Mode::Run);
    p.step(t0 + ms(250), Mode::Run);
    assert_eq!(p.elapsed(), ms(250));
    // Frozen for ten seconds (blur, minimized or a user pause).
    let frozen = p.step(t0 + ms(500), Mode::Frozen);
    assert_eq!((frozen.upload, frozen.wake), (false, None));
    let still = p.step(t0 + Duration::from_secs(10), Mode::Frozen);
    assert_eq!((still.frame, still.upload, still.wake), (2, false, None));
    // Resume continues from the frozen position: no catch-up for the time spent suspended.
    let resumed = p.step(t0 + Duration::from_secs(10) + ms(10), Mode::Run);
    assert_eq!(resumed.frame, 2);
    assert!(p.elapsed() < ms(250) + ms(20));
}

#[test]
fn reduced_motion_shows_the_poster_and_restarts_from_zero() {
    let t0 = Instant::now();
    let mut p = playback(&[100; 5], Repeat::Infinite);
    p.step(t0, Mode::Run);
    p.step(t0 + ms(350), Mode::Run);
    let poster = p.step(t0 + ms(360), Mode::Poster);
    assert_eq!(poster.frame, 0);
    assert_eq!(
        poster.wake,
        Some(UPLOAD_FLOOR.saturating_sub(ms(10))),
        "the poster waits out the floor"
    );
    let settled = p.step(t0 + ms(400), Mode::Poster);
    assert_eq!(
        (settled.frame, settled.upload, settled.wake),
        (0, true, None)
    );
    assert_eq!(p.elapsed(), Duration::ZERO);
    let again = p.step(t0 + ms(600), Mode::Poster);
    assert_eq!(
        (again.upload, again.wake),
        (false, None),
        "no further updates"
    );
    // Leaving reduced motion restarts at zero rather than resuming mid-animation.
    p.step(t0 + ms(700), Mode::Run);
    assert_eq!(p.elapsed(), Duration::ZERO);
}

#[test]
fn paused_minimized_and_reduced_clocks_ask_for_no_decorative_wakeups() {
    let t0 = Instant::now();
    for mode in [Mode::Frozen, Mode::Poster] {
        let mut p = playback(&[100; 4], Repeat::Infinite);
        p.step(t0, Mode::Run);
        p.step(t0 + ms(30), mode);
        let mut wakes = 0;
        let mut uploads = 0;
        for tick in 1..=1000_u64 {
            let step = p.step(t0 + Duration::from_secs(1) + ms(tick * 16), mode);
            wakes += usize::from(step.wake.is_some());
            uploads += usize::from(step.upload);
        }
        assert!(uploads <= 1, "{mode:?} uploads {uploads}");
        assert!(wakes <= 1, "{mode:?} wakes {wakes}");
    }
    // A finished play-through also stops asking.
    let mut p = playback(&[100, 100], Repeat::Plays(1));
    p.step(t0, Mode::Run);
    let done = p.step(t0 + ms(250), Mode::Run);
    assert_eq!((done.frame, done.wake), (1, None));
    let later = p.step(t0 + Duration::from_secs(5), Mode::Run);
    assert_eq!((later.frame, later.upload, later.wake), (1, false, None));
}

#[test]
fn mode_follows_reduced_motion_opaque_chrome_pause_and_window_state() {
    let platform = PlatformPrefs {
        animations: Some(true),
        transparency: Some(true),
        high_contrast: Some(HighContrast {
            on: false,
            colors: None,
        }),
    };
    let base = AppearancePrefs {
        motion: MotionPref::Full,
        ..AppearancePrefs::default()
    };
    let look = |prefs: &AppearancePrefs| resolve(prefs, Theme::Dark, None, platform);
    assert_eq!(Mode::for_look(&look(&base), true, true), Mode::Run);
    assert_eq!(Mode::for_look(&look(&base), false, true), Mode::Frozen);
    assert_eq!(Mode::for_look(&look(&base), true, false), Mode::Frozen);
    let paused = AppearancePrefs {
        paused: true,
        ..base.clone()
    };
    assert_eq!(Mode::for_look(&look(&paused), true, true), Mode::Frozen);
    let reduced = AppearancePrefs {
        motion: MotionPref::Reduced,
        paused: true,
        ..base.clone()
    };
    assert_eq!(Mode::for_look(&look(&reduced), true, true), Mode::Poster);
    let solid = AppearancePrefs {
        reduce_transparency: true,
        ..base
    };
    assert_eq!(Mode::for_look(&look(&solid), true, true), Mode::Poster);
}

// --- runtime ---

struct Env {
    ctx: egui::Context,
    store: MediaStore,
    _root: tempfile::TempDir,
    rt: WallpaperRuntime,
}

fn env() -> Env {
    let root = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let store = MediaStore::new(&AppDirs::from_root(root.path().to_path_buf()));
    let rt = WallpaperRuntime::new(&ctx);
    Env {
        ctx,
        store,
        _root: root,
        rt,
    }
}

fn look_for(prefs: &AppearancePrefs, platform: PlatformPrefs) -> EffectiveAppearance {
    resolve(prefs, Theme::Dark, None, platform)
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

fn animated(frames: u8) -> Vec<u8> {
    let frames: Vec<_> = (0..frames)
        .map(|i| TestFrame::full(8, 6, i % 4, 5))
        .collect();
    gif(8, 6, Some(0), &frames)
}

fn staged(store: &MediaStore, bytes: Vec<u8>) -> StagedMedia {
    store.stage(Arc::new(bytes)).unwrap()
}

impl Env {
    #[allow(clippy::too_many_arguments)]
    fn sync(
        &mut self,
        active: &Wallpaper,
        active_staged: Option<&StagedMedia>,
        preview: Option<(&Wallpaper, Option<&StagedMedia>)>,
        look: &EffectiveAppearance,
        window: (bool, bool),
        now: Instant,
    ) -> (Backdrop, Option<Duration>) {
        self.rt.sync(
            &self.ctx,
            &self.store,
            Wanted {
                wallpaper: active,
                staged: active_staged,
            },
            preview.map(|(wallpaper, staged)| Wanted { wallpaper, staged }),
            look,
            window,
            now,
        )
    }

    fn settle(
        &mut self,
        active: &Wallpaper,
        active_staged: Option<&StagedMedia>,
        preview: Option<(&Wallpaper, Option<&StagedMedia>)>,
        look: &EffectiveAppearance,
    ) -> (Backdrop, Option<Duration>) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let now = Instant::now();
            let out = self.sync(active, active_staged, preview, look, (true, true), now);
            let loading =
                self.rt.is_loading(false) || (preview.is_some() && self.rt.is_loading(true));
            if !loading {
                return out;
            }
            assert!(Instant::now() < deadline, "decode never finished");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn the_builtin_gradient_needs_no_decode_and_uploads_once() {
    let mut e = env();
    let prefs = AppearancePrefs {
        wallpaper: Wallpaper::Builtin,
        motion: MotionPref::Full,
        ..AppearancePrefs::default()
    };
    let look = look_for(&prefs, supported());
    let now = Instant::now();
    let (backdrop, wake) = e.sync(&Wallpaper::Builtin, None, None, &look, (true, true), now);
    assert!(backdrop.has_wallpaper());
    assert_eq!(wake, None, "a static wallpaper schedules nothing");
    assert_eq!(e.rt.stats().decodes_submitted, 0);
    assert_eq!(e.rt.stats().texture_allocations, 1);
    // Navigating between screens is just more frames: no decode, no upload, no wake.
    for tick in 1..50 {
        let (b, wake) = e.sync(
            &Wallpaper::Builtin,
            None,
            None,
            &look,
            (true, true),
            now + ms(tick * 16),
        );
        assert_eq!((b.texture, wake), (backdrop.texture, None));
    }
    assert_eq!(e.rt.stats().texture_writes, 1);
    assert!(e.rt.budget().used() > 0);
}

#[test]
fn a_gif_decodes_once_shares_one_texture_and_updates_in_place() {
    let mut e = env();
    let bytes = animated(6);
    let st = staged(&e.store, bytes);
    let wp = Wallpaper::Local(st.id.clone());
    let prefs = AppearancePrefs {
        wallpaper: wp.clone(),
        motion: MotionPref::Full,
        ..AppearancePrefs::default()
    };
    let look = look_for(&prefs, supported());
    let (backdrop, _) = e.settle(&wp, Some(&st), None, &look);
    assert!(backdrop.has_wallpaper());
    assert_eq!(e.rt.stats().decodes_submitted, 1);
    let id = backdrop.texture.unwrap().0;
    // Play for a few seconds of pinned time; the same texture is rewritten in place.
    let t0 = Instant::now();
    for tick in 0..60 {
        let (b, _) = e.sync(
            &wp,
            Some(&st),
            None,
            &look,
            (true, true),
            t0 + ms(tick * 40),
        );
        assert_eq!(b.texture.unwrap().0, id);
    }
    let stats = e.rt.stats();
    assert_eq!(
        stats.texture_allocations, 1,
        "same-size frames are partial updates"
    );
    assert!(stats.texture_writes > 3);
    assert_eq!(
        stats.decodes_submitted, 1,
        "no decode or reload during playback"
    );
    assert_eq!(e.rt.live_textures(), 1);
}

#[test]
fn reduced_motion_and_pause_stop_uploads_and_wakeups() {
    let mut e = env();
    let st = staged(&e.store, animated(6));
    let wp = Wallpaper::Local(st.id.clone());
    let mut prefs = AppearancePrefs {
        wallpaper: wp.clone(),
        motion: MotionPref::Full,
        ..AppearancePrefs::default()
    };
    let look = look_for(&prefs, supported());
    e.settle(&wp, Some(&st), None, &look);
    let t0 = Instant::now() + Duration::from_secs(1);
    let (_, running) = e.sync(&wp, Some(&st), None, &look, (true, true), t0);
    assert!(
        running.is_some(),
        "an animating wallpaper schedules the next frame"
    );

    // Paused by the user, blurred and minimized: no wake-ups, no uploads.
    prefs.paused = true;
    let paused = look_for(&prefs, supported());
    let before = e.rt.stats().texture_writes;
    for (i, window) in [(true, true), (false, true), (true, false)]
        .into_iter()
        .enumerate()
    {
        for tick in 1..40 {
            let look = if i == 0 { &paused } else { &look };
            let (_, wake) = e.sync(
                &wp,
                Some(&st),
                None,
                look,
                window,
                t0 + Duration::from_secs(1) + ms(tick * 16),
            );
            assert_eq!(wake, None, "window {window:?}");
        }
    }
    assert_eq!(
        e.rt.stats().texture_writes,
        before,
        "no texture writes while suspended"
    );

    // Reduced motion shows frame zero only.
    let reduced = look_for(
        &AppearancePrefs {
            motion: MotionPref::Reduced,
            ..prefs.clone()
        },
        supported(),
    );
    let (_, wake) = e.sync(
        &wp,
        Some(&st),
        None,
        &reduced,
        (true, true),
        t0 + Duration::from_secs(5),
    );
    assert_eq!(wake, None);
}

#[test]
fn a_preview_uses_the_second_texture_and_closing_it_releases_it() {
    let mut e = env();
    let applied = AppearancePrefs {
        wallpaper: Wallpaper::Builtin,
        motion: MotionPref::Full,
        ..AppearancePrefs::default()
    };
    let applied_look = look_for(&applied, supported());
    let (active_backdrop, _) = e.settle(&Wallpaper::Builtin, None, None, &applied_look);
    let used_active = e.rt.budget().used();

    let st = staged(&e.store, animated(4));
    let wp = Wallpaper::Local(st.id.clone());
    let previewed = AppearancePrefs {
        wallpaper: wp.clone(),
        ..applied.clone()
    };
    let preview_look = look_for(&previewed, supported());
    let (preview_backdrop, _) = e.settle(
        &Wallpaper::Builtin,
        None,
        Some((&wp, Some(&st))),
        &preview_look,
    );
    assert_ne!(
        preview_backdrop.texture.unwrap().0,
        active_backdrop.texture.unwrap().0
    );
    assert_eq!(e.rt.live_textures(), 2);
    assert!(
        e.rt.budget().used() > used_active,
        "active and preview share one cap"
    );

    // Dismissing the preview frees the second texture and its retained frames.
    e.rt.clear_preview();
    let (back, _) = e.sync(
        &Wallpaper::Builtin,
        None,
        None,
        &applied_look,
        (true, true),
        Instant::now(),
    );
    assert_eq!(back.texture.unwrap().0, active_backdrop.texture.unwrap().0);
    assert_eq!(e.rt.live_textures(), 1);
    assert_eq!(e.rt.budget().used(), used_active);
}

#[test]
fn a_preview_of_the_applied_wallpaper_shares_its_slot() {
    let mut e = env();
    let prefs = AppearancePrefs {
        wallpaper: Wallpaper::Builtin,
        ..AppearancePrefs::default()
    };
    let look = look_for(&prefs, supported());
    let (a, _) = e.settle(&Wallpaper::Builtin, None, None, &look);
    let (p, _) = e.settle(
        &Wallpaper::Builtin,
        None,
        Some((&Wallpaper::Builtin, None)),
        &look,
    );
    assert_eq!(a.texture.unwrap().0, p.texture.unwrap().0);
    assert_eq!(e.rt.live_textures(), 1);
    assert_eq!(e.rt.stats().texture_allocations, 1);
}

#[test]
fn a_late_preview_result_after_closing_it_is_dropped() {
    let mut e = env();
    let st = staged(&e.store, animated(20));
    let wp = Wallpaper::Local(st.id.clone());
    let prefs = AppearancePrefs {
        wallpaper: wp.clone(),
        ..AppearancePrefs::default()
    };
    let look = look_for(&prefs, supported());
    // Start the preview decode, then close the preview before consuming the result.
    e.sync(
        &Wallpaper::None,
        None,
        Some((&wp, Some(&st))),
        &look,
        (true, true),
        Instant::now(),
    );
    e.rt.clear_preview();
    std::thread::sleep(Duration::from_millis(300));
    let none_look = look_for(&AppearancePrefs::default(), supported());
    let (back, _) = e.sync(
        &Wallpaper::None,
        None,
        None,
        &none_look,
        (true, true),
        Instant::now(),
    );
    assert!(!back.has_wallpaper());
    assert!(!e.rt.is_ready(true));
    assert_eq!(e.rt.live_textures(), 0);
    assert_eq!(
        e.rt.budget().used(),
        0,
        "the late result released its memory"
    );
}

#[test]
fn reset_import_and_restore_rebind_even_when_preferences_are_equal() {
    let mut e = env();
    let prefs = AppearancePrefs {
        wallpaper: Wallpaper::Builtin,
        ..AppearancePrefs::default()
    };
    let look = look_for(&prefs, supported());
    e.settle(&Wallpaper::Builtin, None, None, &look);
    assert_eq!(e.rt.stats().texture_allocations, 1);
    e.rt.invalidate_all();
    assert_eq!(e.rt.live_textures(), 0);
    e.settle(&Wallpaper::Builtin, None, None, &look);
    assert_eq!(
        e.rt.stats().texture_allocations,
        2,
        "the texture was rebuilt"
    );
}

#[test]
fn a_missing_managed_file_falls_back_to_solid_with_one_attempt() {
    let mut e = env();
    let id = AssetId::parse("0123456789abcdef0123456789abcdef").unwrap();
    let wp = Wallpaper::Local(id);
    let prefs = AppearancePrefs {
        wallpaper: wp.clone(),
        ..AppearancePrefs::default()
    };
    let look = look_for(&prefs, supported());
    let (backdrop, wake) = e.settle(&wp, None, None, &look);
    assert!(!backdrop.has_wallpaper());
    assert_eq!(wake, None);
    assert_eq!(e.rt.failure(false), Some(MediaError::Missing));
    for tick in 1..30 {
        e.sync(
            &wp,
            None,
            None,
            &look,
            (true, true),
            Instant::now() + ms(tick * 16),
        );
    }
    assert_eq!(
        e.rt.stats().decodes_submitted,
        1,
        "a failed wallpaper is not retried every frame"
    );
}

#[test]
fn high_contrast_omits_the_wallpaper_and_decodes_nothing() {
    let mut e = env();
    let prefs = AppearancePrefs {
        wallpaper: Wallpaper::Builtin,
        ..AppearancePrefs::default()
    };
    let hc = PlatformPrefs {
        high_contrast: Some(HighContrast {
            on: true,
            colors: None,
        }),
        ..supported()
    };
    let look = look_for(&prefs, hc);
    let (backdrop, wake) = e.sync(
        &Wallpaper::Builtin,
        None,
        None,
        &look,
        (true, true),
        Instant::now(),
    );
    assert!(!backdrop.has_wallpaper());
    assert_eq!(wake, None);
    assert_eq!(e.rt.stats().texture_allocations, 0);
    assert_eq!(e.rt.budget().used(), 0);
}

#[test]
fn a_new_quality_tier_requests_a_new_decode_while_resizing_does_not() {
    let mut e = env();
    let st = staged(&e.store, animated(3));
    let wp = Wallpaper::Local(st.id.clone());
    let mut prefs = AppearancePrefs {
        wallpaper: wp.clone(),
        ..AppearancePrefs::default()
    };
    e.settle(&wp, Some(&st), None, &look_for(&prefs, supported()));
    assert_eq!(e.rt.stats().decodes_submitted, 1);
    // Resizing the window is only a different viewport rect; nothing is re-requested.
    for _ in 0..10 {
        e.sync(
            &wp,
            Some(&st),
            None,
            &look_for(&prefs, supported()),
            (true, true),
            Instant::now(),
        );
    }
    assert_eq!(e.rt.stats().decodes_submitted, 1);
    prefs.quality = Quality::Low;
    e.settle(&wp, Some(&st), None, &look_for(&prefs, supported()));
    assert_eq!(e.rt.stats().decodes_submitted, 2);
}
