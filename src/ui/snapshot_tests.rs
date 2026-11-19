//! Pixel snapshots of every screen in English/dark and Portuguese/light. They guard the visual
//! design against accidental change.
//!
//! The harness renders with embedded Polaris and egui's bundled fallback fonts, a pinned date,
//! and simulated time, so output does not depend on host fonts, the clock, or the window system. A
//! small pixel tolerance absorbs GPU differences. To accept an intended change run
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
pub(crate) static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn snapshot(results: &mut SnapshotResults, name: &str, mut app: AthenaApp, size: egui::Vec2) {
    let _turn = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Time is simulated; the same number of steps always draws the same frame.
    let mut harness = Harness::builder()
        .with_size(size)
        .with_context_setup(theme::typography::install_test_fonts)
        .build_ui_state(
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
        snapshot(
            &mut results,
            &format!("month_populated_{tag}"),
            app,
            egui::Vec2::new(1280.0, 820.0),
        );
    }
    results.unwrap();
}

/// 640 x 410 logical points: the narrow supported viewport.
const NARROW: egui::Vec2 = egui::Vec2::new(640.0, 410.0);

/// Seed the month plus an entry with a long title and a very large amount.
fn narrow_core(language: Language, theme: Theme, screen: Screen) -> AppCore {
    let mut core = onboarded_core(language, theme, true);
    let mut draft = crate::shared::test_fixtures::draft_with(vec![]);
    draft.title =
        "Supermercado com um nome muito longo que precisa quebrar a linha sem cortar".into();
    draft.amount = Money::from_cents(99_999_999_999);
    draft.date = today();
    core.create_transaction(draft).unwrap();
    core.set_screen(screen);
    core
}

#[test]
fn snapshot_narrow_month_and_calendar() {
    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        for (screen, name) in [(Screen::Month, "month"), (Screen::Calendar, "calendar")] {
            snapshot(
                &mut results,
                &format!("{name}_narrow_{tag}"),
                AthenaApp::for_test(narrow_core(language, theme, screen)),
                NARROW,
            );
        }
    }
    results.unwrap();
}

#[test]
fn narrow_viewport_collapses_to_a_labeled_icon_rail() {
    use egui_kittest::kittest::Queryable;
    for (language, theme, _) in variants() {
        let s = crate::i18n::strings(language);
        for (size, rail) in [(NARROW, true), (SIZE, false)] {
            let app = AthenaApp::for_test(narrow_core(language, theme, Screen::Month));
            let mut harness = Harness::builder()
                .with_size(size)
                .with_context_setup(theme::typography::install_test_fonts)
                .build_ui_state(
                    |ui, app: &mut AthenaApp| {
                        theme::apply(ui.ctx(), app.theme_for_test());
                        ui.set_style(ui.ctx().global_style());
                        app.show_for_test(ui);
                    },
                    {
                        let mut app = app;
                        app.pin_today(today());
                        app
                    },
                );
            harness.run_steps(6);
            // Every screen keeps its localized accessible name in both modes.
            for name in [s.month, s.calendar, s.trash, s.settings] {
                assert!(
                    harness.query_by_label(name).is_some(),
                    "{language:?} {size:?} missing nav name {name}"
                );
            }
            // The wordmark only shows in the expanded sidebar.
            assert_eq!(harness.query_by_label(s.app_name).is_some(), !rail);
            // Icon-only month stepper buttons stay named in the accessibility tree.
            assert!(harness.query_by_label(s.month_prev).is_some());
            assert!(harness.query_by_label(s.month_next).is_some());
        }
    }
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

/// The Month screen with its day grid open and a day selected, wide and narrow.
#[test]
fn snapshot_month_day_detail_and_insight() {
    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        for (size, name) in [
            (egui::Vec2::new(1280.0, 1000.0), "month_days"),
            (egui::Vec2::new(640.0, 1500.0), "month_days_narrow"),
        ] {
            let mut core = onboarded_core(language, theme, true);
            core.ui.month_days = true;
            core.ui
                .select_day(crate::shared::test_fixtures::date(2026, 11, 5));
            snapshot(
                &mut results,
                &format!("{name}_{tag}"),
                AthenaApp::for_test(core),
                size,
            );
        }
    }
    results.unwrap();
}

/// The Calendar with one month's grid open.
#[test]
fn snapshot_calendar_days() {
    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        let mut core = onboarded_core(language, theme, true);
        core.set_screen(Screen::Calendar);
        core.ui.calendar_days = crate::domain::date::YearMonth::new(2026, 11);
        core.ui
            .select_day(crate::shared::test_fixtures::date(2026, 11, 8));
        snapshot(
            &mut results,
            &format!("calendar_days_{tag}"),
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

#[test]
fn snapshot_settings_data() {
    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        let mut core = onboarded_core(language, theme, false);
        let _ = std::fs::remove_dir_all(&core.store.dirs().backups_dir);
        core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Data;
        core.set_screen(Screen::Settings);
        core.sync().unwrap();
        core.vm.data_path = "athena/athena.db".into();
        core.vm.backups_dir = "athena/backups".into();
        snapshot(
            &mut results,
            &format!("settings_data_{tag}"),
            AthenaApp::for_test(core),
            SIZE,
        );
    }
    results.unwrap();
}

#[test]
fn snapshot_settings_templates() {
    use crate::app::actions::AppAction;
    use crate::domain::template::{ResponsibilityMode, TemplateDraft};
    use crate::domain::transaction::{Allocation, TransactionType};

    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        let mut core = onboarded_core(language, theme, false);
        let owner = core.vm.all_people[0].id;
        let partner = core.vm.all_people[1].id;
        let food = core
            .vm
            .all_categories
            .iter()
            .find(|c| c.name == "Food")
            .unwrap()
            .id;
        let share = |person_id, cents| Allocation {
            person_id,
            amount: Money::from_cents(cents),
        };
        for draft in [
            TemplateDraft {
                name: "Weekly groceries".into(),
                title: "Groceries".into(),
                amount: Money::from_cents(18_450),
                kind: TransactionType::Expense,
                category_id: Some(food),
                payer_id: Some(owner),
                description: None,
                responsibility: ResponsibilityMode::Even,
                allocations: vec![share(owner, 9_225), share(partner, 9_225)],
            },
            TemplateDraft {
                name: "Coffee".into(),
                title: "Coffee".into(),
                amount: Money::from_cents(900),
                kind: TransactionType::Expense,
                category_id: None,
                payer_id: Some(owner),
                description: None,
                responsibility: ResponsibilityMode::Person,
                allocations: vec![share(owner, 900)],
            },
        ] {
            core.apply(AppAction::SaveTemplate(draft)).unwrap();
        }
        core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Templates;
        core.set_screen(Screen::Settings);
        snapshot(
            &mut results,
            &format!("settings_templates_{tag}"),
            AthenaApp::for_test(core),
            SIZE,
        );
    }
    results.unwrap();
}

#[test]
fn snapshot_trash() {
    let mut results = SnapshotResults::new();
    for (language, theme, tag) in variants() {
        let mut core = onboarded_core(language, theme, true);
        let ids: Vec<i64> = core.vm.transactions.iter().take(2).map(|t| t.id).collect();
        for (id, stamp) in ids
            .into_iter()
            .zip(["2026-11-12T14:00:00Z", "2026-11-10T14:00:00Z"])
        {
            core.delete_transaction(id).unwrap();
            core.store
                .db()
                .conn()
                .execute(
                    "UPDATE transactions SET deleted_at = ?1 WHERE id = ?2",
                    rusqlite::params![stamp, id],
                )
                .unwrap();
        }
        core.set_screen(Screen::Trash);
        snapshot(
            &mut results,
            &format!("trash_{tag}"),
            AthenaApp::for_test(core),
            SIZE,
        );
    }
    let mut empty = onboarded_core(Language::En, Theme::Dark, false);
    empty.set_screen(Screen::Trash);
    snapshot(
        &mut results,
        "trash_empty_en_dark",
        AthenaApp::for_test(empty),
        SIZE,
    );
    results.unwrap();
}

#[test]
fn snapshot_recovery() {
    let _turn = GPU
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dirs = crate::infrastructure::config::AppDirs::from_root("athena".into());
    let state = crate::app::recovery::RecoveryState {
        dirs,
        error: crate::shared::error::AppError::Corrupt {
            path: "athena/athena.db".into(),
        },
        backups: vec![],
        restore_error: None,
        language: Language::En,
    };
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_context_setup(theme::typography::install_test_fonts)
        .build_ui(move |ui| {
            ui.ctx()
                .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
            theme::apply(ui.ctx(), Theme::Dark);
            ui.set_style(ui.ctx().global_style());
            crate::ui::screens::recovery::show(ui, &state);
        });
    h.run_steps(6);
    h.snapshot_options(
        "recovery_en_dark",
        &SnapshotOptions::new()
            .output_path("src/ui/snapshots")
            .max_failed_pixels(150),
    );
}
