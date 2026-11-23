//! Goals: lifecycle tabs, a list and the selected goal's detail with its contribution history. It
//! reads the `ViewModel` and edits `UiState`; every write is an event the app turns into an action.
//! Goal money never touches the ledger.

use eframe::egui::{
    self, Align, Button, CornerRadius, Frame, Label, Layout, Margin, Rect, RichText, Role, Sense,
    Stroke, Vec2, WidgetInfo,
};

use crate::app::ui_state::{GoalFocus, UiState, filtered_goals};
use crate::app::view_model::ViewModel;
use crate::application::services::goals::{GoalDetail, GoalListItem};
use crate::domain::goal::{
    Contribution, ContributionPlan, Frequency, Goal, GoalStatus, GoalTransition, Progress,
    contribution_plan,
};
use crate::domain::settings::Language;
use crate::i18n::{Strings, format_date, strings};
use crate::ui::components::empty_state;
use crate::ui::components::summary_card::plain_brl;
use crate::ui::icons::{self, Icon};
use crate::ui::screens::ScreenEvent;
use crate::ui::theme::{Palette, palette, typography};
use crate::ui::tokens;

/// Content at least this wide shows the list and the detail side by side.
pub const TWO_PANE_WIDTH: f32 = 720.0;
const LIST_WIDTH: f32 = 300.0;

/// What the Goals screen asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalEvent {
    New,
    Edit(i64),
    AddContribution(i64),
    DeleteContribution { goal_id: i64, contribution_id: i64 },
    Transition(i64, GoalTransition),
}

/// The stable id of the list heading, so focus can return to it.
#[must_use]
pub fn heading_id() -> egui::Id {
    egui::Id::NULL.with("goals-heading")
}

fn row_id(id: i64) -> egui::Id {
    egui::Id::NULL.with(("goal-row", id))
}

/// The localized name of a lifecycle status.
#[must_use]
pub fn status_label(s: &Strings, status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => s.goal_status_active,
        GoalStatus::Paused => s.goal_status_paused,
        GoalStatus::Completed => s.goal_status_completed,
        GoalStatus::Archived => s.goal_status_archived,
    }
}

fn tab_label(s: &Strings, status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => s.goals_tab_active,
        GoalStatus::Paused => s.goals_tab_paused,
        GoalStatus::Completed => s.goals_tab_completed,
        GoalStatus::Archived => s.goals_tab_archived,
    }
}

/// `R$ 1,00 of R$ 10,00`.
fn progress_text(s: &Strings, progress: &Progress) -> String {
    s.goal_progress
        .replace("{contributed}", &plain_brl(progress.contributed))
        .replace("{target}", &plain_brl(progress.target))
}

/// Remaining money, or the excess when the goal is overfunded; `None` when exactly funded.
fn gap_text(s: &Strings, progress: &Progress) -> Option<String> {
    if progress.excess.cents() > 0 {
        Some(
            s.goal_excess
                .replace("{amount}", &plain_brl(progress.excess)),
        )
    } else if progress.remaining.cents() > 0 {
        Some(
            s.goal_remaining
                .replace("{amount}", &plain_brl(progress.remaining)),
        )
    } else {
        None
    }
}

/// The schedule hint, or `None` when it is hidden.
fn plan_text(s: &Strings, goal: &Goal, progress: &Progress, vm: &ViewModel) -> Option<String> {
    match contribution_plan(goal, progress.contributed, vm.today) {
        ContributionPlan::Hidden => None,
        ContributionPlan::DueToday(amount) => Some(
            s.goal_plan_due_today
                .replace("{amount}", &plain_brl(amount)),
        ),
        ContributionPlan::Overdue(amount) => {
            Some(s.goal_plan_overdue.replace("{amount}", &plain_brl(amount)))
        }
        ContributionPlan::PerPeriod {
            periods,
            per_period,
        } => {
            let template = match (goal.frequency, periods == 1) {
                (Frequency::Daily, true) => s.goal_plan_daily_one,
                (Frequency::Daily, false) => s.goal_plan_daily_many,
                (Frequency::Weekly, true) => s.goal_plan_weekly_one,
                (Frequency::Weekly, false) => s.goal_plan_weekly_many,
                (Frequency::Monthly, true) => s.goal_plan_monthly_one,
                (Frequency::Monthly, false) => s.goal_plan_monthly_many,
            };
            Some(
                template
                    .replace("{amount}", &plain_brl(per_period))
                    .replace("{n}", &periods.to_string()),
            )
        }
    }
}

/// Who a goal is for, with the archived marker, or `None` when unattributed.
fn person_text(s: &Strings, goal: &Goal, vm: &ViewModel) -> Option<String> {
    let person = vm.person(goal.attribution_id?)?;
    let suffix = if person.archived {
        s.settlement_archived_suffix
    } else {
        ""
    };
    Some(
        s.goal_for_person
            .replace("{name}", &format!("{}{suffix}", person.name)),
    )
}

fn deadline_text(s: &Strings, goal: &Goal, language: Language) -> Option<String> {
    goal.target_date
        .map(|d| s.goal_deadline.replace("{date}", &format_date(d, language)))
}

/// A horizontal bar capped at 100 %; the excess is described in text, never drawn past the end.
fn progress_bar(ui: &mut egui::Ui, p: &Palette, percent: u8, height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let radius = CornerRadius::same(4);
    ui.painter().rect_filled(rect, radius, p.border);
    let fill_width = rect.width() * f32::from(percent) / 100.0;
    if fill_width > 0.0 {
        let fill = Rect::from_min_size(rect.min, Vec2::new(fill_width, rect.height()));
        ui.painter().rect_filled(fill, radius, p.accent);
    }
}

fn funded_badge(ui: &mut egui::Ui, p: &Palette, s: &Strings) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        icons::paint(ui, rect, Icon::Target, p.positive);
        ui.label(
            RichText::new(s.goal_funded)
                .text_style(egui::TextStyle::Small)
                .color(p.positive)
                .strong(),
        );
    });
}

fn header(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState, events: &mut Vec<ScreenEvent>) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            let label = ui.label(
                RichText::new(s.goals)
                    .text_style(typography::display())
                    .strong(),
            );
            let focus = ui.interact(label.rect, heading_id(), Sense::focusable_noninteractive());
            focus.widget_info(|| WidgetInfo::labeled(Role::Heading, true, s.goals));
            if state.goals.pending_focus == Some(GoalFocus::ListHeading)
                && ui.ctx().memory(|m| m.top_modal_layer().is_none())
            {
                focus.request_focus();
                state.goals.pending_focus = None;
            }
            if focus.has_focus() {
                ui.painter().rect_stroke(
                    label.rect.expand(3.0),
                    CornerRadius::same(tokens::CONTROL_RADIUS),
                    Stroke::new(2.0, p.focus),
                    egui::StrokeKind::Outside,
                );
            }
            let underline = Rect::from_min_size(
                egui::pos2(label.rect.left(), label.rect.bottom() + 2.0),
                Vec2::new(label.rect.width(), 3.0),
            );
            ui.painter()
                .rect_filled(underline, CornerRadius::same(2), p.accent);
            ui.add_space(5.0);
        });
        // With no goals at all the empty state carries the only New goal action.
        if vm.goals.is_empty() {
            return;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let primary =
                Button::new(RichText::new(s.goals_new).color(p.on_accent).strong()).fill(p.accent);
            if ui
                .add_sized([160.0, tokens::control_height()], primary)
                .clicked()
            {
                events.push(ScreenEvent::Goal(GoalEvent::New));
            }
        });
    });
}

fn tabs(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) {
    let s = strings(vm.language());
    ui.horizontal_wrapped(|ui| {
        for status in GoalStatus::ALL {
            let selected = state.goals.filter == status;
            let resp = ui.add(Button::selectable(selected, tab_label(s, status)));
            resp.widget_info(|| {
                WidgetInfo::selected(Role::Tab, true, selected, tab_label(s, status))
            });
            if resp.clicked() {
                state.goals.set_filter(status);
            }
        }
    });
}

fn goal_row(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    state: &mut UiState,
    item: &GoalListItem,
    selected: bool,
) {
    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    let goal = &item.goal;
    let fill = if selected {
        tokens::selected_fill(&p, p.card)
    } else {
        p.card
    };
    let frame = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(tokens::BORDER_WIDTH, p.border))
        .corner_radius(CornerRadius::same(tokens::CARD_RADIUS))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(Label::new(RichText::new(&goal.name).strong()).wrap());
            ui.add_space(4.0);
            progress_bar(ui, &p, item.progress.percent, 6.0);
            ui.add_space(4.0);
            ui.add(
                Label::new(
                    RichText::new(progress_text(s, &item.progress))
                        .font(typography::tabular_font(typography::SECONDARY)),
                )
                .wrap(),
            );
            ui.horizontal_wrapped(|ui| {
                if let Some(gap) = gap_text(s, &item.progress) {
                    ui.label(
                        RichText::new(gap)
                            .text_style(egui::TextStyle::Small)
                            .color(p.muted),
                    );
                }
                if item.progress.funded {
                    funded_badge(ui, &p, s);
                }
            });
        });
    let response = ui.interact(frame.response.rect, row_id(goal.id), Sense::click());
    let label = format!(
        "{}, {}",
        s.goal_progress_a11y
            .replace("{name}", &goal.name)
            .replace("{percent}", &item.progress.percent.to_string())
            .replace("{progress}", &progress_text(s, &item.progress)),
        status_label(s, goal.status)
    );
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &label));
    if state.goals.pending_focus == Some(GoalFocus::Row(goal.id))
        && ui.ctx().memory(|m| m.top_modal_layer().is_none())
    {
        response.request_focus();
        state.goals.pending_focus = None;
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            frame.response.rect,
            CornerRadius::same(tokens::CARD_RADIUS),
            Stroke::new(2.0, p.focus),
            egui::StrokeKind::Inside,
        );
    }
    if response.clicked() {
        state.goals.select(goal.id);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand);
}

fn goal_list(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState, items: &[&GoalListItem]) {
    for item in items {
        let selected = state.goals.selected == Some(item.goal.id);
        goal_row(ui, vm, state, item, selected);
        ui.add_space(tokens::space_s());
    }
}

fn contribution_row(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    goal: &Goal,
    contribution: &Contribution,
    events: &mut Vec<ScreenEvent>,
) {
    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(tokens::BORDER_WIDTH, p.border))
        .corner_radius(CornerRadius::same(tokens::CONTROL_RADIUS))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format_date(contribution.date, language))
                        .font(typography::tabular_font(typography::SECONDARY))
                        .color(p.muted),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(plain_brl(contribution.amount))
                            .font(typography::money_font())
                            .color(p.positive),
                    );
                });
            });
            if let Some(note) = &contribution.note {
                ui.add(Label::new(RichText::new(note)).wrap());
            }
            if goal.status.is_editable() {
                let name = s
                    .contribution_remove_a11y
                    .replace("{amount}", &plain_brl(contribution.amount))
                    .replace("{date}", &format_date(contribution.date, language));
                let remove = ui.add(
                    Button::new(
                        RichText::new(s.contribution_remove)
                            .text_style(egui::TextStyle::Small)
                            .color(p.negative),
                    )
                    .frame(false),
                );
                remove.widget_info(|| WidgetInfo::labeled(Role::Button, true, &name));
                if remove.clicked() {
                    remove.request_focus();
                    events.push(ScreenEvent::Goal(GoalEvent::DeleteContribution {
                        goal_id: goal.id,
                        contribution_id: contribution.id,
                    }));
                }
            }
        });
}

/// The primary action for a status, or `None` when there is none.
fn primary_action(s: &Strings, status: GoalStatus) -> (&'static str, GoalEventKind) {
    match status {
        GoalStatus::Active | GoalStatus::Paused => {
            (s.goal_add_contribution, GoalEventKind::AddContribution)
        }
        GoalStatus::Completed => (
            s.goal_reopen,
            GoalEventKind::Transition(GoalTransition::Reopen),
        ),
        GoalStatus::Archived => (
            s.restore,
            GoalEventKind::Transition(GoalTransition::Restore),
        ),
    }
}

#[derive(Clone, Copy)]
enum GoalEventKind {
    AddContribution,
    Transition(GoalTransition),
}

fn actions(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    state: &mut UiState,
    goal: &Goal,
    events: &mut Vec<ScreenEvent>,
) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    ui.horizontal_wrapped(|ui| {
        let (label, kind) = primary_action(s, goal.status);
        let button = ui.add_sized(
            [180.0, tokens::control_height()],
            Button::new(RichText::new(label).color(p.on_accent).strong()).fill(p.accent),
        );
        if state.goals.pending_focus == Some(GoalFocus::Primary)
            && ui.ctx().memory(|m| m.top_modal_layer().is_none())
        {
            button.request_focus();
            state.goals.pending_focus = None;
        }
        if button.clicked() {
            events.push(ScreenEvent::Goal(match kind {
                GoalEventKind::AddContribution => GoalEvent::AddContribution(goal.id),
                GoalEventKind::Transition(t) => GoalEvent::Transition(goal.id, t),
            }));
        }
        if goal.status == GoalStatus::Archived {
            return;
        }
        let menu = ui.menu_button(s.goal_more, |ui| {
            let mut pick = |label: &str, event: GoalEvent| {
                if ui.button(label).clicked() {
                    events.push(ScreenEvent::Goal(event));
                    ui.close();
                }
            };
            if goal.status.is_editable() {
                pick(s.goal_edit, GoalEvent::Edit(goal.id));
            }
            match goal.status {
                GoalStatus::Active => {
                    pick(
                        s.goal_pause,
                        GoalEvent::Transition(goal.id, GoalTransition::Pause),
                    );
                }
                GoalStatus::Paused => {
                    pick(
                        s.goal_resume,
                        GoalEvent::Transition(goal.id, GoalTransition::Resume),
                    );
                }
                _ => {}
            }
            if goal.status.is_editable() {
                pick(
                    s.goal_complete,
                    GoalEvent::Transition(goal.id, GoalTransition::Complete),
                );
            }
            pick(
                s.goal_archive,
                GoalEvent::Transition(goal.id, GoalTransition::Archive),
            );
        });
        menu.response
            .widget_info(|| WidgetInfo::labeled(Role::Button, true, s.goal_more));
    });
}

fn detail(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    state: &mut UiState,
    detail: &GoalDetail,
    events: &mut Vec<ScreenEvent>,
) {
    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    let goal = &detail.goal;
    ui.add(Label::new(
        RichText::new(&goal.name)
            .text_style(typography::section())
            .strong(),
    ));
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(status_label(s, goal.status))
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        if detail.progress.funded {
            funded_badge(ui, &p, s);
        }
        if let Some(person) = person_text(s, goal, vm) {
            ui.label(
                RichText::new(person)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
        }
        if let Some(deadline) = deadline_text(s, goal, language) {
            ui.label(
                RichText::new(deadline)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
        }
    });
    ui.add_space(tokens::space_m());
    progress_bar(ui, &p, detail.progress.percent, 10.0);
    ui.add_space(6.0);
    ui.add(
        Label::new(
            RichText::new(progress_text(s, &detail.progress)).font(typography::money_font()),
        )
        .wrap(),
    );
    if let Some(gap) = gap_text(s, &detail.progress) {
        ui.label(RichText::new(gap).color(p.muted));
    }
    if let Some(plan) = plan_text(s, goal, &detail.progress, vm) {
        ui.add_space(4.0);
        ui.add(Label::new(RichText::new(plan).color(p.accent)).wrap());
    }
    ui.add_space(tokens::space_m());
    actions(ui, vm, state, goal, events);
    ui.add_space(tokens::space_l());
    ui.label(
        RichText::new(s.goal_contributions_title)
            .text_style(typography::section())
            .strong(),
    );
    ui.add_space(tokens::space_s());
    if detail.contributions.is_empty() {
        ui.label(RichText::new(s.goal_no_contributions).color(p.muted));
    }
    for contribution in &detail.contributions {
        contribution_row(ui, vm, goal, contribution, events);
        ui.add_space(tokens::space_s());
    }
}

/// Draw the screen. Returns what the app should do.
#[must_use]
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) -> Vec<ScreenEvent> {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let mut events = Vec::new();

    // Keep the visible ids current even when the list is not drawn, so Back and a removed
    // selection find the right row.
    let items = filtered_goals(&vm.goals, state.goals.filter);
    state.goals.visible = items.iter().map(|g| g.goal.id).collect();

    let wide = ui.available_width() >= TWO_PANE_WIDTH;
    // On narrow widths an open goal replaces the list; Back returns to it.
    let drilled = !wide && state.goals.selected.is_some() && vm.goal_detail.is_some();
    if drilled {
        let back = ui.add(Button::new(RichText::new(s.goals_back).color(p.accent)).frame(false));
        back.widget_info(|| WidgetInfo::labeled(Role::Button, true, s.goals_back));
        if back.clicked() {
            state.goals.back();
            state.goals.pending_focus = state.goals.visible.first().map(|_| GoalFocus::ListHeading);
        }
        ui.add_space(tokens::space_s());
        if let Some(open) = &vm.goal_detail {
            detail(ui, vm, state, open, &mut events);
        }
        return events;
    }

    header(ui, vm, state, &mut events);
    ui.add_space(tokens::space_s());
    tabs(ui, vm, state);
    ui.add_space(tokens::space_m());

    if vm.goals.is_empty() {
        if empty_state::show(
            ui,
            vm.theme(),
            Icon::Target,
            s.goals_empty_title,
            s.goals_empty_hint,
            Some(s.goals_new),
        ) {
            events.push(ScreenEvent::Goal(GoalEvent::New));
        }
        return events;
    }
    if items.is_empty() {
        empty_state::show(
            ui,
            vm.theme(),
            Icon::Target,
            s.goals_empty_filter_title,
            s.goals_empty_filter_hint,
            None,
        );
        return events;
    }

    if wide {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(LIST_WIDTH, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(LIST_WIDTH);
                    goal_list(ui, vm, state, &items);
                },
            );
            ui.add_space(tokens::space_l());
            ui.vertical(|ui| {
                if let Some(open) = &vm.goal_detail {
                    detail(ui, vm, state, open, &mut events);
                } else {
                    ui.label(RichText::new(s.goals_select_hint).color(p.muted));
                }
            });
        });
    } else {
        goal_list(ui, vm, state, &items);
    }
    events
}
