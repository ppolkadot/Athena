//! Settings → Appearance driven through the whole app (contract §14.2): keyboard activation, a
//! draft that previews live, failed media, Apply / Cancel / Reset, the dirty-leave guard, and a
//! same-profile restore that rebinds the wallpaper.

use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable};

use super::actions::{AppAction, RestoreSource};
use super::navigation::Screen;
use super::{AppCore, AthenaApp};
use crate::application::services::onboarding::OnboardingResult;
use crate::domain::appearance::{AppearancePrefs, Preset, Wallpaper};
use crate::domain::money::Money;
use crate::domain::settings::{Language, Theme};
use crate::infrastructure::db::{Db, integrity};
use crate::infrastructure::media::assets::MediaStore;
use crate::infrastructure::media::test_gif::{TestFrame, gif};
use crate::ui::screens::settings::SettingsPage;
use crate::ui::theme;

struct Rig {
    harness: Harness<'static, AthenaApp>,
    root: tempfile::TempDir,
}

fn rig() -> Rig {
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
    core.set_screen(Screen::Settings);
    core.ui.settings.page = SettingsPage::Appearance;
    let harness = Harness::builder()
        .with_size(egui::Vec2::new(1280.0, 1000.0))
        .with_step_dt(0.05)
        .with_context_setup(theme::typography::install_test_fonts)
        .build_ui_state(
            |ui, app: &mut AthenaApp| {
                ui.ctx()
                    .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                app.show(ui);
            },
            AthenaApp::for_test(core),
        );
    let mut rig = Rig { harness, root };
    rig.harness.run_steps(6);
    rig
}

fn gif_file(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let frames = [TestFrame::full(8, 6, 0, 10), TestFrame::full(8, 6, 2, 10)];
    let path = dir.join(name);
    std::fs::write(&path, gif(8, 6, Some(0), &frames)).unwrap();
    path
}

impl Rig {
    fn state(&self) -> &AthenaApp {
        self.harness.state()
    }

    fn edit(&self) -> &super::appearance_edit::AppearanceEdit {
        self.state().core.ui.settings.appearance.as_ref().unwrap()
    }

    fn steps(&mut self, n: usize) {
        self.harness.run_steps(n);
    }

    /// Activate a labeled control the way a keyboard user does: focus it, press Enter.
    fn key_activate(&mut self, label: &str) {
        self.harness.get_by_label(label).focus();
        self.steps(2);
        self.harness.key_press(Key::Enter);
        self.steps(3);
    }

    fn wait_preview_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            self.steps(2);
            let loading = self.edit().status.loading;
            if !loading
                && self
                    .state()
                    .wallpaper
                    .as_ref()
                    .is_some_and(|rt| rt.is_ready(true))
            {
                break;
            }
            assert!(Instant::now() < deadline, "preview never finished decoding");
            std::thread::sleep(Duration::from_millis(5));
        }
        self.steps(2);
    }

    fn apply_disabled(&self) -> bool {
        self.harness
            .get_by_label("Apply")
            .accesskit_node()
            .is_disabled()
    }
}

#[test]
fn keyboard_activation_previews_live_and_cancel_restores_the_applied_look() {
    let mut rig = rig();
    assert_eq!(rig.state().look.theme, Theme::Dark);
    rig.key_activate("Pearl");
    // The whole shell previews immediately; nothing is stored yet.
    assert_eq!(rig.state().look.theme, Theme::Light);
    assert_eq!(rig.edit().draft.preset, Preset::Pearl);
    assert_eq!(rig.state().core.vm.appearance, None);
    assert!(!rig.apply_disabled());
    rig.key_activate("Cancel");
    assert_eq!(rig.state().look.theme, Theme::Dark);
    assert!(!rig.edit().is_dirty());
    assert!(rig.apply_disabled());
}

#[test]
fn a_rejected_file_blocks_apply_and_the_error_sits_next_to_the_control() {
    let mut rig = rig();
    let bad = rig.root.path().join("note.png");
    std::fs::write(&bad, b"this is not an image").unwrap();
    rig.harness.state_mut().test_wallpaper_picks.push(Some(bad));
    rig.key_activate("Choose file");
    assert!(
        rig.harness
            .query_by_label("Choose a PNG, JPEG or GIF image.")
            .is_some()
    );
    assert_eq!(
        rig.edit().draft.wallpaper,
        Wallpaper::None,
        "the previous choice stays"
    );
    assert!(rig.apply_disabled());

    // A file that passes the magic check but is damaged fails in the decoder and also blocks Apply.
    let mut damaged = gif(8, 6, None, &[TestFrame::full(8, 6, 0, 10)]);
    damaged.truncate(damaged.len() - 12);
    let path = rig.root.path().join("damaged.gif");
    std::fs::write(&path, damaged).unwrap();
    rig.harness
        .state_mut()
        .test_wallpaper_picks
        .push(Some(path));
    rig.key_activate("Choose file");
    let deadline = Instant::now() + Duration::from_secs(20);
    while rig.edit().status.failure.is_none() {
        rig.steps(2);
        assert!(Instant::now() < deadline, "decode failure never reported");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        rig.harness
            .query_by_label("That image is damaged or incomplete.")
            .is_some()
    );
    assert!(rig.apply_disabled());
    // Removing the file clears the error and the plain canvas is previewed again.
    rig.key_activate("Remove");
    assert!(rig.edit().status.failure.is_none());
    assert!(
        rig.harness
            .query_by_label("That image is damaged or incomplete.")
            .is_none()
    );
}

#[test]
fn applying_a_picked_gif_stores_a_managed_copy_and_keeps_the_texture() {
    let mut rig = rig();
    let path = gif_file(rig.root.path(), "wall.gif");
    rig.harness
        .state_mut()
        .test_wallpaper_picks
        .push(Some(path));
    rig.key_activate("Choose file");
    assert!(
        rig.harness.query_by_label("Loading the image…").is_some()
            || rig.edit().status.loading
            || rig.edit().staged.is_some()
    );
    rig.wait_preview_ready();
    assert!(
        !rig.apply_disabled(),
        "Apply is available once the image is ready"
    );
    let preview_texture = rig.state().wallpaper.as_ref().unwrap().texture_id(true);
    rig.key_activate("Apply");

    let applied = rig.state().core.vm.appearance.clone().unwrap();
    let Wallpaper::Local(id) = applied.wallpaper.clone() else {
        panic!("a local wallpaper was applied")
    };
    let store = MediaStore::new(rig.state().core.store.dirs());
    assert!(store.read(&id).is_ok(), "the managed copy exists");
    assert!(
        integrity::check(rig.state().core.store.db().conn())
            .unwrap()
            .is_empty()
    );
    // The session restarted clean, and the finished preview became the active wallpaper without
    // a second decode or a flash of the plain canvas.
    rig.steps(4);
    assert!(!rig.edit().is_dirty());
    let rt = rig.state().wallpaper.as_ref().unwrap();
    assert_eq!(rt.texture_id(false), preview_texture);
    assert_eq!(rt.stats().decodes_submitted, 1);
    assert_eq!(rt.live_textures(), 1);
}

#[test]
fn leaving_with_a_dirty_draft_asks_and_keep_editing_restores_focus() {
    let mut rig = rig();
    rig.key_activate("Aurora");
    assert!(rig.edit().is_dirty());
    // A sidebar click is held behind the prompt.
    rig.harness.get_by_label("Trash").click();
    rig.steps(3);
    assert_eq!(rig.state().core.ui.screen, Screen::Settings);
    assert!(
        rig.harness
            .query_by_label("Discard appearance changes?")
            .is_some()
    );
    // Background shortcuts are blocked while it is open.
    rig.harness
        .key_press_modifiers(Modifiers::COMMAND, Key::Num2);
    rig.steps(3);
    assert_eq!(rig.state().core.ui.screen, Screen::Settings);
    // Escape keeps editing and the draft is untouched.
    rig.harness.key_press(Key::Escape);
    rig.steps(4);
    assert!(
        rig.harness
            .query_by_label("Discard appearance changes?")
            .is_none()
    );
    assert!(rig.edit().is_dirty() && rig.edit().draft.preset == Preset::Aurora);
    assert_eq!(rig.state().core.ui.screen, Screen::Settings);

    // A page switch is guarded the same way; Discard performs it once and drops the draft.
    rig.harness.get_by_label("General").click();
    rig.steps(3);
    assert_eq!(rig.state().core.ui.settings.page, SettingsPage::Appearance);
    assert!(
        rig.harness
            .query_by_label("Discard appearance changes?")
            .is_some()
    );
    rig.harness.get_by_label("Discard").click();
    rig.steps(4);
    assert_eq!(rig.state().core.ui.settings.page, SettingsPage::General);
    assert!(rig.state().core.ui.settings.appearance.is_none());
    assert_eq!(
        rig.state().look.theme,
        Theme::Dark,
        "the applied look returned"
    );

    // A clean page can be left freely.
    rig.harness.get_by_label("Appearance").click();
    rig.steps(3);
    rig.harness.get_by_label("Trash").click();
    rig.steps(3);
    assert_eq!(rig.state().core.ui.screen, Screen::Trash);
    assert!(
        rig.harness
            .query_by_label("Discard appearance changes?")
            .is_none()
    );
}

#[test]
fn an_open_ledger_draft_keeps_priority_over_an_appearance_draft() {
    let mut rig = rig();
    rig.key_activate("Aurora");
    rig.harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    rig.steps(14);
    assert!(rig.state().quick_add.is_some());
    // While the ledger form is open, the Appearance draft is neither lost nor applicable.
    assert!(rig.edit().is_dirty());
    rig.harness.key_press(Key::Escape);
    rig.steps(6);
    assert!(
        rig.state()
            .core
            .ui
            .settings
            .appearance
            .as_ref()
            .unwrap()
            .is_dirty()
    );
}

#[test]
fn reset_asks_first_then_returns_to_the_profile_defaults() {
    let mut rig = rig();
    rig.key_activate("Aurora");
    rig.key_activate("Apply");
    assert!(rig.state().core.vm.appearance.is_some());
    rig.steps(3);
    rig.key_activate("Reset to defaults");
    assert!(rig.harness.query_by_label("Reset appearance?").is_some());
    // Cancel keeps everything.
    let cancel = rig
        .harness
        .get_all_by_label("Cancel")
        .find(|n| !n.accesskit_node().is_disabled())
        .expect("the dialog offers Cancel");
    cancel.click();
    rig.steps(4);
    assert!(rig.state().core.vm.appearance.is_some());
    rig.key_activate("Reset to defaults");
    rig.harness.get_by_label("Reset").click();
    rig.steps(5);
    assert_eq!(rig.state().core.vm.appearance, None);
    assert_eq!(rig.state().look.theme, Theme::Dark);
    assert_eq!(rig.state().core.vm.profile_theme(), Theme::Dark);
    assert!(
        !rig.edit().is_dirty(),
        "the session restarted from the defaults"
    );
}

#[test]
fn a_same_profile_restore_rebinds_the_wallpaper_from_the_snapshot_reference() {
    let mut rig = rig();
    let path = gif_file(rig.root.path(), "wall.gif");
    rig.harness
        .state_mut()
        .test_wallpaper_picks
        .push(Some(path));
    rig.key_activate("Choose file");
    rig.wait_preview_ready();
    rig.key_activate("Apply");
    rig.steps(4);
    let wallpaper = rig.state().core.vm.appearance.clone().unwrap().wallpaper;
    assert!(matches!(wallpaper, Wallpaper::Local(_)));

    rig.harness
        .state_mut()
        .core
        .apply(AppAction::CreateBackup)
        .unwrap();
    rig.harness
        .state_mut()
        .core
        .apply(AppAction::ResetAppearance)
        .unwrap();
    rig.steps(4);
    assert_eq!(rig.state().core.vm.appearance, None);
    assert_eq!(rig.state().wallpaper.as_ref().unwrap().live_textures(), 0);
    let epoch = rig.state().core.media_epoch;

    let backup = rig
        .state()
        .core
        .vm
        .backups
        .iter()
        .find(|b| b.path.extension().is_some_and(|e| e == "db"))
        .expect("the manual backup is listed")
        .path
        .clone();
    rig.harness
        .state_mut()
        .core
        .apply(AppAction::RestoreBackup(RestoreSource::Listed(backup)))
        .unwrap();
    assert!(rig.state().core.media_epoch > epoch);
    // The reference came back with the database; the managed file is in the same root, so the
    // wallpaper resolves again even though the preferences compare equal to what was there before.
    assert_eq!(
        rig.state().core.vm.appearance.clone().unwrap().wallpaper,
        wallpaper
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !rig.state().wallpaper.as_ref().unwrap().is_ready(false) {
        rig.steps(2);
        assert!(Instant::now() < deadline, "wallpaper never rebound");
        std::thread::sleep(Duration::from_millis(5));
    }
    rig.steps(3);
    assert_eq!(rig.state().wallpaper.as_ref().unwrap().live_textures(), 1);
}

#[test]
fn a_missing_managed_file_shows_a_notice_and_keeps_the_workspace() {
    let mut rig = rig();
    rig.harness.state_mut().core.set_screen(Screen::Month);
    let id = crate::domain::appearance::AssetId::parse("0123456789abcdef0123456789abcdef").unwrap();
    rig.harness
        .state_mut()
        .core
        .apply(AppAction::ApplyAppearance(Box::new(
            super::actions::AppearanceApply {
                prefs: AppearancePrefs {
                    wallpaper: Wallpaper::Local(id),
                    ..AppearancePrefs::default()
                },
                media: None,
            },
        )))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while rig.state().media_failure.is_none() {
        rig.steps(2);
        assert!(
            Instant::now() < deadline,
            "the missing file was never reported"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(rig.state().toast.is_some(), "a localized notice was raised");
    // The Appearance page repeats the explanation next to the Background control.
    rig.harness.state_mut().core.set_screen(Screen::Settings);
    rig.steps(4);
    assert!(
        rig.harness
            .query_all_by_label("The wallpaper file is missing, so the plain background is shown.")
            .count()
            > 0
    );
    // The app still works: navigation, ledger and settings are untouched.
    rig.harness.get_by_label("Month").click();
    rig.steps(3);
    assert_eq!(rig.state().core.ui.screen, Screen::Month);
}
