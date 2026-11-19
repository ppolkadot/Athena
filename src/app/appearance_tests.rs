//! Appearance preferences through the Store: isolated from finance, optional on read (§14.2).

use super::AppCore;
use super::actions::{AppAction, AppearanceApply, RefreshScope};
use crate::domain::appearance::{Accent, AppearancePrefs, Density, TextScale, ThemeMode};
use crate::domain::settings::Theme;
use crate::infrastructure::db::{integrity, profile_repo};
use crate::shared::test_fixtures::{profile, seeded_db};

fn core() -> AppCore {
    let db = seeded_db();
    profile_repo::save(db.conn(), &profile()).unwrap();
    AppCore::empty_for_test(db)
}

fn prefs() -> AppearancePrefs {
    AppearancePrefs {
        theme_mode: ThemeMode::Light,
        accent: Accent::Teal,
        density: Density::Compact,
        text_scale: TextScale::P130,
        dim: 40,
        ..AppearancePrefs::default()
    }
}

#[test]
fn appearance_apply_cancel_reset_isolated() {
    let mut core = core();
    let before_profile = core.vm.profile.clone();
    let before_rows = core.vm.transactions.clone();
    let before_summary = core.vm.summary;
    assert_eq!(core.vm.appearance, None);

    // Apply commits the full row and refreshes only the appearance read.
    let version = core.store.data_version();
    let outcome = core
        .apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
            prefs: prefs(),
            media: None,
        })))
        .unwrap();
    assert_eq!(outcome.refresh, RefreshScope::Appearance);
    assert_eq!(core.store.data_version(), version + 1);
    assert_eq!(core.vm.appearance, Some(prefs()));
    assert_eq!(core.vm.profile, before_profile, "legacy theme is untouched");
    assert_eq!(core.vm.transactions, before_rows);
    assert_eq!(core.vm.summary, before_summary);

    // A rejected Apply (a cancelled draft never reaches the store) changes nothing.
    let version = core.store.data_version();
    let invalid = AppearancePrefs {
        dim: 101,
        ..prefs()
    };
    assert!(
        core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
            prefs: invalid,
            media: None,
        })))
        .is_err()
    );
    assert_eq!(core.store.data_version(), version);
    assert_eq!(core.vm.appearance, Some(prefs()));

    // Reset deletes the row and recovers the profile-derived defaults, nothing else.
    let outcome = core.apply(AppAction::ResetAppearance).unwrap();
    assert_eq!(outcome.refresh, RefreshScope::Appearance);
    assert_eq!(core.vm.appearance, None);
    assert_eq!(core.vm.theme(), Theme::Dark);
    assert_eq!(core.vm.profile, before_profile);
    assert_eq!(core.vm.transactions, before_rows);
    assert!(integrity::check(core.store.db().conn()).unwrap().is_empty());
}

#[test]
fn bad_appearance_row_leaves_financial_workspace_available() {
    let mut core = core();
    core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
        prefs: prefs(),
        media: None,
    })))
    .unwrap();
    core.store
        .db()
        .conn()
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             UPDATE appearance SET preset = 'nonsense';
             PRAGMA ignore_check_constraints = OFF;",
        )
        .unwrap();

    // A full reload (restore/import/retry) still succeeds and keeps every financial field.
    core.vm
        .reload(&core.store, &core.ui, RefreshScope::All)
        .unwrap();
    assert!(!core.vm.refresh_failed);
    assert!(core.vm.appearance_unreadable);
    assert_eq!(core.vm.appearance, None);
    assert_eq!(core.vm.transactions.len(), 3);
    assert!(core.vm.onboarding_complete());
    assert_eq!(core.vm.theme(), Theme::Dark);

    // A new Apply repairs the row.
    core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
        prefs: prefs(),
        media: None,
    })))
    .unwrap();
    assert!(!core.vm.appearance_unreadable);
    assert_eq!(core.vm.appearance, Some(prefs()));
}

#[test]
fn language_switch_keeps_the_applied_appearance_and_defaults() {
    let mut core = core();
    core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
        prefs: prefs(),
        media: None,
    })))
    .unwrap();
    let mut changed = core.vm.profile.clone().unwrap();
    changed.language = crate::domain::settings::Language::PtBr;
    core.apply(AppAction::SaveProfile(changed)).unwrap();
    assert_eq!(core.vm.language(), crate::domain::settings::Language::PtBr);
    assert_eq!(core.vm.appearance, Some(prefs()));
    // Without a row the defaults still follow the legacy theme, whatever the language.
    core.apply(AppAction::ResetAppearance).unwrap();
    assert_eq!(core.vm.appearance, None);
    assert_eq!(core.vm.profile_theme(), Theme::Dark);
}

mod media {
    use std::sync::Arc;

    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder};

    use super::*;
    use crate::domain::appearance::{AssetId, Wallpaper};
    use crate::infrastructure::db::Db;
    use crate::infrastructure::media::MediaError;
    use crate::infrastructure::media::assets::{MAX_FILES, MediaStore, StagedMedia};
    use crate::shared::error::AppError;

    fn png(seed: u8) -> Arc<Vec<u8>> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(&[seed, 1, 2, 255], 1, 1, ExtendedColorType::Rgba8)
            .unwrap();
        Arc::new(out)
    }

    fn core_in(root: &tempfile::TempDir) -> AppCore {
        let db = seeded_db();
        profile_repo::save(db.conn(), &profile()).unwrap();
        AppCore::empty_for_test_in(db, root.path().to_path_buf())
    }

    fn local(staged: &StagedMedia) -> AppearancePrefs {
        AppearancePrefs {
            wallpaper: Wallpaper::Local(staged.id.clone()),
            ..AppearancePrefs::default()
        }
    }

    fn apply(
        core: &mut AppCore,
        prefs: AppearancePrefs,
        media: Option<StagedMedia>,
    ) -> Result<(), AppError> {
        core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
            prefs,
            media,
        })))
        .map(|_| ())
    }

    #[test]
    fn apply_publishes_the_staged_copy_before_the_row_references_it() {
        let root = tempfile::tempdir().unwrap();
        let mut core = core_in(&root);
        let store = MediaStore::new(core.store.dirs());
        let staged = store.stage(png(1)).unwrap();
        apply(&mut core, local(&staged), Some(staged.clone())).unwrap();
        assert_eq!(core.vm.appearance, Some(local(&staged)));
        assert_eq!(store.read(&staged.id).unwrap(), *staged.bytes);
        assert!(integrity::check(core.store.db().conn()).unwrap().is_empty());
        // The selected wallpaper must be the staged one.
        let other = store.stage(png(2)).unwrap();
        let err = apply(&mut core, local(&staged), Some(other)).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
        assert_eq!(store.usage().unwrap().0, 1, "nothing extra was published");
    }

    #[test]
    fn a_failed_apply_removes_only_its_own_new_file_and_keeps_the_old_state() {
        let root = tempfile::tempdir().unwrap();
        let mut core = core_in(&root);
        let store = MediaStore::new(core.store.dirs());
        let first = store.stage(png(1)).unwrap();
        apply(&mut core, local(&first), Some(first.clone())).unwrap();
        let before = core.vm.appearance.clone();
        let version = core.store.data_version();

        // Break the write after publication: the new copy must be removed again.
        core.store
            .db()
            .conn()
            .execute_batch("ALTER TABLE appearance RENAME TO appearance_gone")
            .unwrap();
        let second = store.stage(png(2)).unwrap();
        assert!(apply(&mut core, local(&second), Some(second.clone())).is_err());
        assert_eq!(store.read(&second.id).unwrap_err(), MediaError::Missing);
        assert_eq!(core.store.data_version(), version);
        // A failed apply that reused an existing copy never deletes it.
        let reused = store.stage(png(1)).unwrap();
        assert_eq!(reused.id, first.id);
        assert!(apply(&mut core, local(&reused), Some(reused)).is_err());
        assert_eq!(store.read(&first.id).unwrap(), *first.bytes);
        core.store
            .db()
            .conn()
            .execute_batch("ALTER TABLE appearance_gone RENAME TO appearance")
            .unwrap();
        assert_eq!(core.vm.appearance, before);
    }

    #[test]
    fn quota_exhaustion_keeps_the_current_wallpaper_and_explains_the_limit() {
        let root = tempfile::tempdir().unwrap();
        let mut core = core_in(&root);
        let store = MediaStore::new(core.store.dirs());
        let first = store.stage(png(0)).unwrap();
        apply(&mut core, local(&first), Some(first.clone())).unwrap();
        for seed in 1..u8::try_from(MAX_FILES).unwrap() {
            let staged = store.stage(png(seed)).unwrap();
            store.publish(&staged).unwrap();
        }
        let extra = store.stage(png(250)).unwrap();
        let version = core.store.data_version();
        let err = apply(&mut core, local(&extra), Some(extra)).unwrap_err();
        assert!(matches!(err, AppError::Media(MediaError::QuotaExceeded)));
        assert_eq!(core.vm.appearance, Some(local(&first)));
        assert_eq!(core.store.data_version(), version);
        let text = crate::i18n::media_error_message(
            MediaError::QuotaExceeded,
            crate::domain::settings::Language::En,
        );
        assert!(text.contains("32 files") && text.contains("kept"));
    }

    #[test]
    fn a_missing_managed_file_is_not_an_integrity_problem() {
        let root = tempfile::tempdir().unwrap();
        let mut core = core_in(&root);
        let id = AssetId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let prefs = AppearancePrefs {
            wallpaper: Wallpaper::Local(id.clone()),
            ..AppearancePrefs::default()
        };
        apply(&mut core, prefs.clone(), None).unwrap();
        assert_eq!(core.vm.appearance, Some(prefs));
        assert!(integrity::check(core.store.db().conn()).unwrap().is_empty());
        assert_eq!(
            MediaStore::new(core.store.dirs()).read(&id).unwrap_err(),
            MediaError::Missing
        );
        let _ = Db::open_in_memory;
    }
}
