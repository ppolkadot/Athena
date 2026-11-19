//! The goal and contribution forms: validated fields, localized errors next to the field that
//! caused them, and drafts that survive a refused write. The form builds an [`AppAction`]; the app
//! applies it.

use chrono::NaiveDate;
use eframe::egui::{self, Align, Button, ComboBox, Id, Layout, RichText, TextEdit};

use crate::app::actions::AppAction;
use crate::app::view_model::ViewModel;
use crate::domain::goal::{
    ContributionDraft, Frequency, Goal, GoalDraft, GoalError, GoalField, NOTE_MAX_CHARS,
};
use crate::domain::money::{Money, MoneyError};
use crate::domain::settings::Language;
use crate::i18n::{
    date_example, format_date, goal_error_message, money_error_message, parse_date, strings,
};
use crate::ui::components::money_input;
use crate::ui::theme::{palette, typography};

/// What the form edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalFormMode {
    NewGoal,
    EditGoal(i64),
    /// Adding a contribution to this goal.
    Contribution(i64),
}

/// Why a field was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalFormError {
    Name,
    Money(GoalField, MoneyError),
    /// A blank required amount.
    AmountBlank(GoalField),
    Date(GoalField),
    /// A rule the repository enforced.
    Rule(GoalError),
    /// The write failed for another reason; nothing was changed.
    Save,
}

impl GoalFormError {
    #[must_use]
    pub fn field(self) -> GoalField {
        match self {
            GoalFormError::Name => GoalField::Name,
            GoalFormError::Money(field, _)
            | GoalFormError::AmountBlank(field)
            | GoalFormError::Date(field) => field,
            GoalFormError::Rule(rule) => rule.field(),
            GoalFormError::Save => GoalField::General,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalFormState {
    pub mode: GoalFormMode,
    pub name: String,
    /// Target amount (goal) or contribution amount, as typed.
    pub amount_text: String,
    /// Target date (goal, optional) or contribution date, as typed.
    pub date_text: String,
    pub frequency: Frequency,
    pub person: Option<i64>,
    pub note: String,
    pub errors: Vec<GoalFormError>,
    focus_first: bool,
    language: Language,
}

/// What the form asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum GoalFormEvent {
    Close,
    Submit(AppAction),
}

impl GoalFormState {
    #[must_use]
    pub fn new_goal(language: Language) -> Self {
        Self {
            mode: GoalFormMode::NewGoal,
            name: String::new(),
            amount_text: String::new(),
            date_text: String::new(),
            frequency: Frequency::Monthly,
            person: None,
            note: String::new(),
            errors: Vec::new(),
            focus_first: true,
            language,
        }
    }

    #[must_use]
    pub fn edit_goal(goal: &Goal, language: Language) -> Self {
        Self {
            mode: GoalFormMode::EditGoal(goal.id),
            name: goal.name.clone(),
            amount_text: goal.target.br_number(),
            date_text: goal
                .target_date
                .map(|d| format_date(d, language))
                .unwrap_or_default(),
            frequency: goal.frequency,
            person: goal.attribution_id,
            ..Self::new_goal(language)
        }
    }

    #[must_use]
    pub fn contribution(goal_id: i64, today: NaiveDate, language: Language) -> Self {
        Self {
            mode: GoalFormMode::Contribution(goal_id),
            date_text: format_date(today, language),
            ..Self::new_goal(language)
        }
    }

    /// The error shown for `field`, if any.
    #[must_use]
    pub fn error_for(&self, field: GoalField) -> Option<GoalFormError> {
        self.errors.iter().copied().find(|e| e.field() == field)
    }

    fn parse_amount(&self, field: GoalField) -> Result<Money, GoalFormError> {
        match money_input::check(&self.amount_text) {
            (Some(money), _) => Ok(money),
            (None, Some(error)) => Err(GoalFormError::Money(field, error)),
            (None, None) => Err(GoalFormError::AmountBlank(field)),
        }
    }

    /// Validate every field and build the action, or list every problem found.
    ///
    /// # Errors
    /// All field errors, in field order.
    pub fn submit(&self) -> Result<AppAction, Vec<GoalFormError>> {
        let mut errors = Vec::new();
        match self.mode {
            GoalFormMode::NewGoal | GoalFormMode::EditGoal(_) => {
                if self.name.trim().is_empty() {
                    errors.push(GoalFormError::Name);
                }
                let target = self.parse_amount(GoalField::Target);
                if let Err(e) = target {
                    errors.push(e);
                }
                let target_date = if self.date_text.trim().is_empty() {
                    None
                } else if let Some(date) = parse_date(&self.date_text, self.language) {
                    Some(date)
                } else {
                    errors.push(GoalFormError::Date(GoalField::Date));
                    None
                };
                if !errors.is_empty() {
                    return Err(errors);
                }
                let draft = GoalDraft {
                    name: self.name.trim().to_string(),
                    target: target.unwrap_or(Money::ZERO),
                    target_date,
                    frequency: self.frequency,
                    attribution_id: self.person,
                };
                if let Err(rules) = draft.validate() {
                    return Err(rules.into_iter().map(GoalFormError::Rule).collect());
                }
                Ok(match self.mode {
                    GoalFormMode::EditGoal(id) => AppAction::UpdateGoal(id, draft),
                    _ => AppAction::CreateGoal(draft),
                })
            }
            GoalFormMode::Contribution(goal_id) => {
                let amount = self.parse_amount(GoalField::Amount);
                if let Err(e) = amount {
                    errors.push(e);
                }
                let date = parse_date(&self.date_text, self.language);
                if date.is_none() {
                    errors.push(GoalFormError::Date(GoalField::Date));
                }
                if self.note.trim().chars().count() > NOTE_MAX_CHARS {
                    errors.push(GoalFormError::Rule(GoalError::NoteTooLong));
                }
                let (Ok(amount), Some(date), true) = (amount, date, errors.is_empty()) else {
                    return Err(errors);
                };
                Ok(AppAction::AddContribution(
                    goal_id,
                    ContributionDraft {
                        amount,
                        date,
                        note: Some(self.note.clone()),
                    },
                ))
            }
        }
    }

    /// Show a refused write next to the field it concerns; unknown failures stay general.
    pub fn refuse(&mut self, rules: &[GoalError]) {
        self.errors = if rules.is_empty() {
            vec![GoalFormError::Save]
        } else {
            rules.iter().copied().map(GoalFormError::Rule).collect()
        };
        self.focus_first = true;
    }
}

fn error_text(error: GoalFormError, language: Language) -> String {
    let s = strings(language);
    match error {
        GoalFormError::Name => s.goal_err_name.to_string(),
        GoalFormError::Money(_, e) => money_error_message(e, language).to_string(),
        GoalFormError::AmountBlank(GoalField::Target) => s.goal_err_target.to_string(),
        GoalFormError::AmountBlank(_) => s.goal_err_amount.to_string(),
        GoalFormError::Date(_) => s
            .goal_err_date
            .replace("{example}", &date_example(language)),
        GoalFormError::Rule(rule) => goal_error_message(rule, language).to_string(),
        GoalFormError::Save => s.goal_err_save.to_string(),
    }
}

fn field_error(ui: &mut egui::Ui, state: &GoalFormState, field: GoalField, vm: &ViewModel) {
    if let Some(error) = state.error_for(field) {
        let p = palette(vm.theme());
        ui.label(RichText::new(error_text(error, vm.language())).color(p.negative));
    }
}

fn text_field(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    hint: &str,
    focus: bool,
) -> egui::Response {
    let caption = ui.label(RichText::new(label).text_style(egui::TextStyle::Small));
    let response = ui
        .add(
            TextEdit::singleline(value)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        )
        .labelled_by(caption.id);
    if focus {
        response.request_focus();
    }
    response
}

fn frequency_label(frequency: Frequency, language: Language) -> &'static str {
    let s = strings(language);
    match frequency {
        Frequency::Daily => s.goal_freq_daily,
        Frequency::Weekly => s.goal_freq_weekly,
        Frequency::Monthly => s.goal_freq_monthly,
    }
}

/// Draw the form as a modal. Returns what the user asked for.
#[allow(clippy::too_many_lines)]
pub fn show(
    ctx: &egui::Context,
    state: &mut GoalFormState,
    vm: &ViewModel,
) -> Option<GoalFormEvent> {
    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    let mut event = None;
    let focus_first = std::mem::take(&mut state.focus_first);
    let first_error = state.errors.first().map(|e| e.field());
    let modal = egui::Modal::new(Id::NULL.with("goal-form")).show(ctx, |ui| {
        ui.set_width(400.0);
        let title = match state.mode {
            GoalFormMode::NewGoal => s.goal_form_new_title,
            GoalFormMode::EditGoal(_) => s.goal_form_edit_title,
            GoalFormMode::Contribution(_) => s.contrib_form_title,
        };
        ui.label(
            RichText::new(title)
                .text_style(typography::section())
                .strong(),
        );
        ui.add_space(8.0);
        let mut submit = false;
        match state.mode {
            GoalFormMode::NewGoal | GoalFormMode::EditGoal(_) => {
                let focus_name = focus_first && first_error.is_none_or(|f| f == GoalField::Name);
                let name = text_field(ui, s.goal_f_name, &mut state.name, "", focus_name);
                submit |= name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                field_error(ui, state, GoalField::Name, vm);
                ui.add_space(4.0);
                let focus_target = focus_first && first_error == Some(GoalField::Target);
                let money = money_input::show(
                    ui,
                    vm.theme(),
                    language,
                    s.goal_f_target,
                    &mut state.amount_text,
                    focus_target,
                );
                submit |=
                    money.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if money.error.is_none() {
                    field_error(ui, state, GoalField::Target, vm);
                }
                let focus_date = focus_first && first_error == Some(GoalField::Date);
                let date = text_field(
                    ui,
                    s.goal_f_date,
                    &mut state.date_text,
                    &date_example(language),
                    focus_date,
                );
                submit |= date.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                field_error(ui, state, GoalField::Date, vm);
                ui.add_space(4.0);
                ui.label(RichText::new(s.goal_f_frequency).text_style(egui::TextStyle::Small));
                ComboBox::from_id_salt("goal-form-frequency")
                    .selected_text(frequency_label(state.frequency, language))
                    .show_ui(ui, |ui| {
                        for frequency in Frequency::ALL {
                            ui.selectable_value(
                                &mut state.frequency,
                                frequency,
                                frequency_label(frequency, language),
                            );
                        }
                    });
                ui.add_space(4.0);
                ui.label(RichText::new(s.goal_f_person).text_style(egui::TextStyle::Small));
                let selected_name = state
                    .person
                    .and_then(|id| vm.person(id))
                    .map_or(s.goal_f_nobody.to_string(), |person| person.name.clone());
                ComboBox::from_id_salt("goal-form-person")
                    .selected_text(selected_name)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut state.person, None, s.goal_f_nobody);
                        for person in &vm.all_people {
                            // Archived people stay visible only while they are the current choice.
                            if person.archived && state.person != Some(person.id) {
                                continue;
                            }
                            ui.selectable_value(&mut state.person, Some(person.id), &person.name);
                        }
                    });
                field_error(ui, state, GoalField::Attribution, vm);
            }
            GoalFormMode::Contribution(_) => {
                let focus_amount = focus_first
                    && first_error
                        .is_none_or(|f| f == GoalField::Amount || f == GoalField::General);
                let money = money_input::show(
                    ui,
                    vm.theme(),
                    language,
                    s.qa_amount,
                    &mut state.amount_text,
                    focus_amount,
                );
                submit |=
                    money.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if money.error.is_none() {
                    field_error(ui, state, GoalField::Amount, vm);
                }
                let focus_date = focus_first && first_error == Some(GoalField::Date);
                let date = text_field(
                    ui,
                    s.contrib_f_date,
                    &mut state.date_text,
                    &date_example(language),
                    focus_date,
                );
                submit |= date.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                field_error(ui, state, GoalField::Date, vm);
                ui.add_space(4.0);
                let caption =
                    ui.label(RichText::new(s.contrib_f_note).text_style(egui::TextStyle::Small));
                ui.add(
                    TextEdit::multiline(&mut state.note)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(caption.id);
                field_error(ui, state, GoalField::Note, vm);
            }
        }
        field_error(ui, state, GoalField::General, vm);
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let save = Button::new(RichText::new(s.save).color(p.on_accent).strong())
                .fill(p.accent)
                .min_size(egui::Vec2::new(110.0, 36.0));
            submit |= ui.add(save).clicked();
            if ui
                .add(Button::new(s.cancel).min_size(egui::Vec2::new(96.0, 36.0)))
                .clicked()
            {
                event = Some(GoalFormEvent::Close);
            }
        });
        if submit {
            match state.submit() {
                Ok(action) => {
                    state.errors.clear();
                    event = Some(GoalFormEvent::Submit(action));
                }
                Err(errors) => {
                    state.errors = errors;
                    state.focus_first = true;
                }
            }
        }
    });
    if event.is_none() && modal.should_close() {
        event = Some(GoalFormEvent::Close);
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::goal::GoalStatus;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 19).unwrap()
    }

    #[test]
    fn a_goal_form_validates_every_field_and_builds_the_action() {
        let mut state = GoalFormState::new_goal(Language::En);
        let errors = state.submit().unwrap_err();
        assert!(errors.contains(&GoalFormError::Name));
        assert!(errors.contains(&GoalFormError::AmountBlank(GoalField::Target)));
        state.name = "  Trip  ".into();
        state.amount_text = "1.234,50".into();
        state.date_text = "31/02/2027".into();
        assert_eq!(
            state.submit().unwrap_err(),
            vec![GoalFormError::Date(GoalField::Date)]
        );
        state.date_text = "2027-06-01".into();
        state.person = Some(2);
        let Ok(AppAction::CreateGoal(draft)) = state.submit() else {
            panic!("expected a create action");
        };
        assert_eq!(draft.name, "Trip");
        assert_eq!(draft.target.cents(), 123_450);
        assert_eq!(draft.target_date, NaiveDate::from_ymd_opt(2027, 6, 1));
        assert_eq!(draft.attribution_id, Some(2));
        state.amount_text = "0".into();
        assert_eq!(
            state.submit().unwrap_err(),
            vec![GoalFormError::Rule(GoalError::TargetNotPositive)]
        );
    }

    #[test]
    fn a_contribution_form_defaults_to_today_and_keeps_a_note() {
        let mut state = GoalFormState::contribution(7, today(), Language::PtBr);
        assert_eq!(state.date_text, "19/11/2026");
        state.amount_text = "12,34".into();
        state.note = "  from the bonus  ".into();
        let Ok(AppAction::AddContribution(7, draft)) = state.submit() else {
            panic!("expected a contribution action");
        };
        assert_eq!(draft.amount.cents(), 1234);
        assert_eq!(draft.date, today());
        assert!(state.submit().is_ok());
        state.date_text = "nope".into();
        assert!(state.submit().is_err());
    }

    #[test]
    fn editing_prefills_the_stored_goal() {
        let goal = Goal {
            id: 3,
            name: "Car".into(),
            target: Money::from_cents(500_000),
            target_date: NaiveDate::from_ymd_opt(2027, 1, 2),
            frequency: Frequency::Weekly,
            attribution_id: Some(2),
            status: GoalStatus::Paused,
            pre_archive_status: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let state = GoalFormState::edit_goal(&goal, Language::PtBr);
        assert_eq!(state.date_text, "02/01/2027");
        assert_eq!(state.frequency, Frequency::Weekly);
        let Ok(AppAction::UpdateGoal(3, draft)) = state.submit() else {
            panic!("expected an update action");
        };
        assert_eq!(draft, goal.draft());
    }
}
