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
