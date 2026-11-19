//! The Goals screen driven through the whole app (contract §14.3): selection and detail caching,
//! forms with draft preservation, confirmations, focus after lifecycle changes, and the layouts.

use eframe::egui::{self, Id, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::actions::{AppAction, AppearanceApply};
use super::navigation::Screen;
use super::{AppCore, AthenaApp};
use crate::domain::appearance::{AppearancePrefs, MotionPref, TextScale};
use crate::domain::goal::{
    ContributionDraft, Frequency, GoalDraft, GoalStatus, GoalTransition, contribution_plan,
};
use crate::domain::money::Money;
use crate::domain::settings::{Language, Profile, Theme};
use crate::infrastructure::db::{Db, goals_repo, integrity};
use crate::shared::test_fixtures::{date, seed_month, today};
use crate::ui::theme;

fn core() -> AppCore {
    let db = Db::open_in_memory().unwrap();
    seed_month(&db);
    let mut core = AppCore::empty_for_test(db);
    core.save_profile(Profile {
        start_balance: Money::ZERO,
        theme: Theme::Dark,
        language: Language::En,
        onboarding_complete: true,
    })
    .unwrap();
    core.store.freeze_today(today());
    core
}

fn goal(name: &str, target: i64) -> GoalDraft {
    GoalDraft {
        name: name.into(),
        target: Money::from_cents(target),
        target_date: Some(date(2027, 3, 1)),
        frequency: Frequency::Monthly,
        attribution_id: None,
    }
}

fn contribution(cents: i64, day: u32) -> ContributionDraft {
    ContributionDraft {
        amount: Money::from_cents(cents),
        date: date(2026, 11, day),
        note: None,
    }
}

fn create(core: &mut AppCore, name: &str, target: i64) -> i64 {
    core.apply(AppAction::CreateGoal(goal(name, target)))
        .unwrap()
        .created_goal
        .unwrap()
}

fn harness_sized(core: AppCore, size: egui::Vec2) -> Harness<'static, AthenaApp> {
    let mut app = AthenaApp::for_test(core);
    app.pin_today(today());
    let mut harness = Harness::builder()
        .with_size(size)
        .with_step_dt(0.05)
        .with_context_setup(theme::typography::install_test_fonts)
        .build_ui_state(
            |ui, app: &mut AthenaApp| {
                ui.ctx()
                    .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                theme::apply(ui.ctx(), app.core.vm.theme());
                let style = ui.ctx().global_style();
                ui.set_style(style);
                app.show(ui);
            },
            app,
        );
    harness.run_steps(6);
    harness
}

fn wide(core: AppCore) -> Harness<'static, AthenaApp> {
    harness_sized(core, egui::Vec2::new(1280.0, 900.0))
}

fn open_goals(h: &mut Harness<'static, AthenaApp>) {
    h.key_press_modifiers(Modifiers::COMMAND, Key::Num5);
    h.run_steps(4);
    assert_eq!(h.state().core.ui.screen, Screen::Goals);
}

fn focused(h: &Harness<'static, AthenaApp>, id: Id) -> bool {
    h.ctx.memory(|m| m.has_focus(id))
}

#[test]
fn goal_detail_refreshes_on_selection_contribution_and_midnight() {
    let mut core = core();
    let mut due_tomorrow = goal("Trip", 100_000);
    due_tomorrow.target_date = Some(today().succ_opt().unwrap());
    due_tomorrow.frequency = Frequency::Daily;
    let a = core
        .apply(AppAction::CreateGoal(due_tomorrow))
        .unwrap()
        .created_goal
        .unwrap();
    let b = create(&mut core, "Car", 500_000);
    // Creating selects the new row and loads its detail immediately.
    assert_eq!(core.ui.goals.selected, Some(b));
    assert_eq!(core.vm.goal_detail.as_ref().unwrap().goal.id, b);

    // Changing the selection reloads only the detail.
    let (list_loads, detail_loads) = (core.vm.goal_list_loads, core.vm.goal_detail_loads);
    core.ui.goals.select(a);
    core.sync().unwrap();
    assert_eq!(core.vm.goal_list_loads, list_loads);
    assert_eq!(core.vm.goal_detail_loads, detail_loads + 1);
    assert_eq!(core.vm.goal_detail.as_ref().unwrap().goal.id, a);

    // A contribution shows in both the detail and the list summary at once.
    core.apply(AppAction::AddContribution(a, contribution(25_000, 10)))
        .unwrap();
    let detail = core.vm.goal_detail.as_ref().unwrap();
    assert_eq!(detail.contributions.len(), 1);
    assert_eq!(detail.progress.contributed.cents(), 25_000);
    let item = core.vm.goals.iter().find(|g| g.goal.id == a).unwrap();
    assert_eq!(item.progress.percent, 25);

    // At the next local midnight the key changes and the schedule is recomputed from the new day.
    let before = contribution_plan(&detail.goal, detail.progress.contributed, core.vm.today);
    assert!(matches!(
        before,
        crate::domain::goal::ContributionPlan::PerPeriod { periods: 1, .. }
    ));
    core.vm.today = today().succ_opt().unwrap();
    assert!(core.sync().unwrap());
    let detail = core.vm.goal_detail.as_ref().unwrap();
    let after = contribution_plan(&detail.goal, detail.progress.contributed, core.vm.today);
    assert!(matches!(
        after,
        crate::domain::goal::ContributionPlan::DueToday(_)
    ));
    assert_eq!(core.vm.goal_list_loads, list_loads + 2);
    assert_eq!(integrity::check(core.store.db().conn()).unwrap(), vec![]);
}

#[test]
fn a_missing_or_filtered_out_selection_is_cleared_and_restore_resets_goals() {
    let mut core = core();
    let id = create(&mut core, "Trip", 1000);
    assert_eq!(core.ui.goals.selected, Some(id));
    // Archiving removes it from the Active partition: the selection clears and its detail goes.
    core.apply(AppAction::ChangeGoalStatus(id, GoalTransition::Archive))
        .unwrap();
    assert_eq!(core.ui.goals.selected, None);
    assert!(core.vm.goal_detail.is_none());
    // A stale id (deleted behind the app's back) is dropped on the next sync.
    core.ui.goals.set_filter(GoalStatus::Archived);
    core.ui.goals.select(999);
    core.sync().unwrap();
    assert_eq!(core.ui.goals.selected, None);
}

#[test]
fn goal_archive_restore_returns_focus_and_status() {
    let mut core = core();
    let first = create(&mut core, "Trip", 100_000);
    let second = create(&mut core, "Car", 500_000);
    core.apply(AppAction::AddContribution(second, contribution(500, 3)))
        .unwrap();
    core.apply(AppAction::ChangeGoalStatus(second, GoalTransition::Pause))
        .unwrap();
    core.ui.goals.set_filter(GoalStatus::Paused);
    core.ui.goals.select(second);
    let mut h = wide(core);
    open_goals(&mut h);
    h.run_steps(3);

    // Archive from Paused through the menu and its confirmation.
    h.get_by_label("More actions").click();
    h.run_steps(3);
    h.get_by_label("Archive").click();
    h.run_steps(3);
    assert!(h.query_by_label("Archive this goal?").is_some());
    h.get_by_label("Archive").click();
    h.run_steps(6);
    let stored = goals_repo::get(h.state().core.store.db().conn(), second).unwrap();
    assert_eq!(stored.status, GoalStatus::Archived);
    assert_eq!(stored.pre_archive_status, Some(GoalStatus::Paused));
    // Nothing valid is left in the Paused tab: focus lands on the list heading.
    assert_eq!(h.state().core.ui.goals.selected, None);
    assert!(focused(&h, crate::ui::screens::goals::heading_id()));

    // Restore from Archived returns the stored status, and focus moves on again.
    h.get_by_label("Archived").click();
    h.run_steps(4);
    h.state_mut().core.ui.goals.select(second);
    h.run_steps(4);
    h.get_by_label("Restore").click();
    h.run_steps(6);
    let stored = goals_repo::get(h.state().core.store.db().conn(), second).unwrap();
    assert_eq!(stored.status, GoalStatus::Paused);
    assert_eq!(stored.pre_archive_status, None);
    assert!(focused(&h, crate::ui::screens::goals::heading_id()));
    assert_eq!(
        goals_repo::get(h.state().core.store.db().conn(), first)
            .unwrap()
            .status,
        GoalStatus::Active
    );
    assert_eq!(
        integrity::check(h.state().core.store.db().conn()).unwrap(),
        vec![]
    );
}

#[test]
fn creating_a_goal_through_the_form_selects_it_and_failed_writes_keep_the_draft() {
    let mut h = wide(core());
    open_goals(&mut h);
    h.get_by_label("New goal").click();
    h.run_steps(4);
    assert!(h.state().goal_form.is_some());

    // A blank form is refused field by field and stays open with what was typed.
    h.get_by_label("Name").type_text("Holiday");
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(4);
    assert!(h.state().goal_form.is_some());
    assert!(
        h.query_by_label("Enter a target greater than zero.")
            .is_some()
    );
    assert_eq!(h.state().goal_form.as_ref().unwrap().name, "Holiday");

    h.get_by_label("Target amount").type_text("1500,00");
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(6);
    assert!(h.state().goal_form.is_none());
    let goals = &h.state().core.vm.goals;
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].goal.name, "Holiday");
    assert_eq!(goals[0].goal.target.cents(), 150_000);
    assert_eq!(h.state().core.ui.goals.selected, Some(goals[0].goal.id));
    assert!(h.query_by_label("Add contribution").is_some());
}

#[test]
fn a_refused_contribution_keeps_the_typed_values_and_a_valid_one_shows_at_once() {
    let mut core = core();
    let id = create(&mut core, "Trip", 100_000);
    let mut h = wide(core);
    open_goals(&mut h);
    h.get_by_label("Add contribution").click();
    h.run_steps(4);
    h.get_by_label("Amount").type_text("10,00");
    // A date after today is refused by the store; the form keeps its text and says why.
    h.state_mut().goal_form.as_mut().unwrap().date_text = "2999-01-01".into();
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(4);
    assert!(h.state().goal_form.is_some());
    assert!(
        h.query_by_label("A contribution cannot be dated in the future.")
            .is_some()
    );
    assert_eq!(h.state().goal_form.as_ref().unwrap().amount_text, "10,00");
    assert!(
        h.state()
            .core
            .vm
            .goal_detail
            .as_ref()
            .unwrap()
            .contributions
            .is_empty()
    );

    h.state_mut().goal_form.as_mut().unwrap().date_text = "2026-11-10".into();
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(6);
    assert!(h.state().goal_form.is_none());
    let detail = h.state().core.vm.goal_detail.as_ref().unwrap();
    assert_eq!(detail.goal.id, id);
    assert_eq!(detail.progress.contributed.cents(), 1000);
}

#[test]
fn completing_below_target_asks_and_names_the_remaining_money() {
    let mut core = core();
    let id = create(&mut core, "Trip", 100_000);
    core.apply(AppAction::AddContribution(id, contribution(25_000, 5)))
        .unwrap();
    let mut h = wide(core);
    open_goals(&mut h);
    h.get_by_label("More actions").click();
    h.run_steps(3);
    h.get_by_label("Mark as completed").click();
    h.run_steps(4);
    assert!(h.query_by_label("Complete this goal?").is_some());
    assert!(
        h.query_by_label_contains("R$ 750,00 short of its target")
            .is_some()
    );
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    let stored = goals_repo::get(h.state().core.store.db().conn(), id).unwrap();
    assert_eq!(stored.status, GoalStatus::Active);
    // Confirming completes it; the contribution row is no longer removable.
    h.get_by_label("More actions").click();
    h.run_steps(3);
    h.get_by_label("Mark as completed").click();
    h.run_steps(4);
    h.get_by_label("Complete").click();
    h.run_steps(5);
    let stored = goals_repo::get(h.state().core.store.db().conn(), id).unwrap();
    assert_eq!(stored.status, GoalStatus::Completed);
}

#[test]
fn removing_a_contribution_confirms_with_its_date_and_amount() {
    let mut core = core();
    let id = create(&mut core, "Trip", 100_000);
    core.apply(AppAction::AddContribution(id, contribution(12_345, 7)))
        .unwrap();
    let mut h = wide(core);
    open_goals(&mut h);
    h.get_by_label_contains("Remove the contribution").click();
    h.run_steps(4);
    assert!(h.query_by_label("Remove this contribution?").is_some());
    assert!(
        h.query_by_label("R$ 123,45 on 2026-11-07 will be removed from Trip.")
            .is_some()
    );
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(
        h.state()
            .core
            .vm
            .goal_detail
            .as_ref()
            .unwrap()
            .contributions
            .len(),
        1
    );
    h.get_by_label_contains("Remove the contribution").click();
    h.run_steps(4);
    h.get_by_label("Remove").click();
    h.run_steps(5);
    assert!(
        h.state()
            .core
            .vm
            .goal_detail
            .as_ref()
            .unwrap()
            .contributions
            .is_empty()
    );
}

#[test]
fn narrow_widths_drill_in_and_back_out() {
    let mut core = core();
    let id = create(&mut core, "Trip", 100_000);
    core.ui.goals.back();
    let mut h = harness_sized(core, egui::Vec2::new(640.0, 410.0));
    open_goals(&mut h);
    // The list shows alone; opening a goal replaces it with the detail and a Back control.
    assert!(h.query_by_label("Back to goals").is_none());
    h.state_mut().core.ui.goals.select(id);
    h.run_steps(4);
    assert!(h.query_by_label("Back to goals").is_some());
    h.get_by_label("Back to goals").click();
    h.run_steps(4);
    assert_eq!(h.state().core.ui.goals.selected, None);
    assert!(h.query_by_label("Back to goals").is_none());
}

#[test]
fn paused_goals_keep_their_actions_at_every_text_scale() {
    let _reset = theme::active::ResetOnDrop;
    for text_scale in TextScale::ALL {
        let mut core = core();
        core.apply(AppAction::ApplyAppearance(Box::new(AppearanceApply {
            prefs: AppearancePrefs {
                text_scale,
                motion: MotionPref::Reduced,
                ..AppearancePrefs::default()
            },
            media: None,
        })))
        .unwrap();
        let id = create(&mut core, "Trip", 100_000);
        core.apply(AppAction::ChangeGoalStatus(id, GoalTransition::Pause))
            .unwrap();
        core.ui.goals.set_filter(GoalStatus::Paused);
        core.ui.goals.select(id);
        let mut h = wide(core);
        open_goals(&mut h);
        assert_eq!(h.state().look.text_scale, text_scale.percent());
        assert!(h.state().look.reduced_motion);
        assert!(h.query_by_label("Add contribution").is_some());
        assert!(h.query_by_label("More actions").is_some());
    }
}
