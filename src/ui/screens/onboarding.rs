//! First-run onboarding: welcome + language, name + partner, starting balance, ready.
//! Full window, drawn instead of the shell until onboarding is complete.

use eframe::egui::{
    self, Align, Button, Color32, Frame, Key, Layout, Mesh, Modifiers, Pos2, Rect, RichText, Sense,
    TextEdit, Vec2,
};

use crate::application::services::onboarding::{OnboardingError, OnboardingResult};
use crate::domain::money::{Money, MoneyError};
use crate::domain::settings::{Language, Theme};
use crate::i18n::{money_error_message, strings};
use crate::ui::theme::{Palette, palette, typography};

const CONTENT_WIDTH: f32 = 560.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Name,
    Balance,
    Ready,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Welcome, Step::Name, Step::Balance, Step::Ready];

    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Step::Welcome => 0,
            Step::Name => 1,
            Step::Balance => 2,
            Step::Ready => 3,
        }
    }

    fn next(self) -> Step {
        Self::ALL[(self.index() + 1).min(3)]
    }

    fn prev(self) -> Step {
        Self::ALL[self.index().saturating_sub(1)]
    }

    /// Optional steps can be skipped.
    #[must_use]
    pub fn is_optional(self) -> bool {
        self == Step::Balance
    }
}

/// Why the current step cannot continue. Rendered through i18n so it follows the language toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingIssue {
    Name(OnboardingError),
    Balance(MoneyError),
    SaveFailed,
}

#[derive(Debug, Clone)]
pub struct OnboardingState {
    pub step: Step,
    pub display_name: String,
    pub partner_name: String,
    pub balance_input: String,
    pub language: Language,
    pub error: Option<OnboardingIssue>,
    focus_pending: bool,
}

impl OnboardingState {
    #[must_use]
    pub fn new(language: Language) -> Self {
        Self {
            step: Step::Welcome,
            display_name: String::new(),
            partner_name: String::new(),
            balance_input: String::new(),
            language,
            error: None,
            focus_pending: false,
        }
    }

    /// A blank balance means zero; anything else must be a valid amount.
    fn parse_balance(&self) -> Result<Money, OnboardingIssue> {
        if self.balance_input.trim().is_empty() {
            return Ok(Money::ZERO);
        }
        Money::parse(&self.balance_input).map_err(OnboardingIssue::Balance)
    }

    fn assemble(&self, start_balance: Money) -> OnboardingResult {
        let partner = self.partner_name.trim();
        OnboardingResult {
            display_name: self.display_name.trim().to_string(),
            partner_name: (!partner.is_empty()).then(|| partner.to_string()),
            start_balance,
            language: self.language,
        }
    }

    /// Everything entered so far, validated.
    ///
    /// # Errors
    /// The first problem found, name before balance.
    pub fn build_result(&self) -> Result<OnboardingResult, OnboardingIssue> {
        let partial = self.assemble(Money::ZERO);
        partial.validate().map_err(OnboardingIssue::Name)?;
        Ok(self.assemble(self.parse_balance()?))
    }

    fn validate_current(&self) -> Result<(), OnboardingIssue> {
        match self.step {
            Step::Welcome => Ok(()),
            Step::Name => self
                .assemble(Money::ZERO)
                .validate()
                .map_err(OnboardingIssue::Name),
            Step::Balance => self.parse_balance().map(|_| ()),
            Step::Ready => self.build_result().map(|_| ()),
        }
    }

    fn go_to(&mut self, step: Step) {
        self.step = step;
        self.error = None;
        self.focus_pending = matches!(step, Step::Name | Step::Balance);
    }

    /// Continue. On the last step, returns the finished result instead of moving on. A problem is
    /// stored in `error` and the step does not change.
    pub fn advance(&mut self) -> Option<OnboardingResult> {
        if let Err(issue) = self.validate_current() {
            self.error = Some(issue);
            // Enter makes a text field let go of the keyboard; give it back so the user can
            // fix the value without reaching for the mouse.
            self.focus_pending = matches!(self.step, Step::Name | Step::Balance);
            return None;
        }
        if self.step == Step::Ready {
            return self.build_result().ok();
        }
        self.go_to(self.step.next());
        None
    }

    pub fn back(&mut self) {
        self.go_to(self.step.prev());
    }

    /// Skip an optional step, leaving its fields blank.
    pub fn skip(&mut self) {
        if self.step.is_optional() {
            self.balance_input.clear();
            self.go_to(self.step.next());
        }
    }

    /// `(current, total)`, one-based, for the progress label.
    #[must_use]
    pub fn progress(&self) -> (usize, usize) {
        (self.step.index() + 1, Step::ALL.len())
    }
}

impl Default for OnboardingState {
    fn default() -> Self {
        Self::new(Language::default())
    }
}

/// Localized text for a problem.
#[must_use]
pub fn issue_message(issue: OnboardingIssue, language: Language) -> &'static str {
    let s = strings(language);
    match issue {
        OnboardingIssue::Name(OnboardingError::NameRequired) => s.v_name_required,
        OnboardingIssue::Name(OnboardingError::PartnerNameTaken) => s.v_partner_name_taken,
        OnboardingIssue::Balance(e) => money_error_message(e, language),
        OnboardingIssue::SaveFailed => s.onb_save_failed,
    }
}

fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// Two slow translucent circles and a faint vertical gradient.
#[allow(clippy::cast_possible_truncation)]
fn paint_background(ui: &egui::Ui, p: &Palette, rect: Rect) {
    let t = ui.ctx().input(|i| i.time);
    let (w, h) = (rect.width(), rect.height());
    let wobble = |speed: f64, phase: f64| (t * speed + phase).sin() as f32;
    let painter = ui.painter();

    let mut mesh = Mesh::default();
    let clear = Color32::TRANSPARENT;
    let glow = tint(p.accent, 26);
    mesh.colored_vertex(rect.left_top(), clear);
    mesh.colored_vertex(rect.right_top(), clear);
    mesh.colored_vertex(rect.right_bottom(), glow);
    mesh.colored_vertex(rect.left_bottom(), glow);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);

    let one = Pos2::new(
        rect.left() + w * (0.22 + 0.04 * wobble(0.31, 0.0)),
        rect.top() + h * (0.28 + 0.05 * wobble(0.23, 1.0)),
    );
    let two = Pos2::new(
        rect.left() + w * (0.80 + 0.04 * wobble(0.27, 2.0)),
        rect.top() + h * (0.78 + 0.05 * wobble(0.19, 3.0)),
    );
    painter.circle_filled(one, 300.0, tint(p.accent, 20));
    painter.circle_filled(two, 340.0, tint(p.positive, 12));
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(33));
}

#[allow(clippy::cast_precision_loss)]
fn progress_dots(ui: &mut egui::Ui, p: &Palette, state: &OnboardingState) {
    let (current, total) = state.progress();
    let label = strings(state.language)
        .onb_step_of
        .replace("{current}", &current.to_string())
        .replace("{total}", &total.to_string());
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(total as f32 * 16.0, 12.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, &label));
    for i in 0..total {
        let center = Pos2::new(rect.left() + 6.0 + i as f32 * 16.0, rect.center().y);
        let color = match (i + 1).cmp(&current) {
            std::cmp::Ordering::Equal => p.accent,
            std::cmp::Ordering::Less => tint(p.accent, 120),
            std::cmp::Ordering::Greater => p.border,
        };
        ui.painter().circle_filled(center, 4.0, color);
    }
}

fn text_field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, focus: &mut bool) {
    let caption = ui.label(RichText::new(label).text_style(egui::TextStyle::Small));
    let response = ui.add(
        TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .margin(Vec2::new(12.0, 10.0)),
    );
    let response = response.labelled_by(caption.id);
    if *focus {
        response.request_focus();
        *focus = false;
    }
    ui.add_space(8.0);
}

fn step_body(ui: &mut egui::Ui, p: &Palette, state: &mut OnboardingState) {
    let s = strings(state.language);
    let mut focus = state.focus_pending;
    match state.step {
        Step::Welcome => {
            ui.label(
                RichText::new(s.onb_welcome_title)
                    .text_style(typography::display())
                    .strong(),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(s.onb_welcome_body).color(p.muted));
            ui.add_space(24.0);
            ui.label(
                RichText::new(s.language)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
            ui.horizontal(|ui| {
                ui.selectable_value(&mut state.language, Language::En, s.english);
                ui.selectable_value(&mut state.language, Language::PtBr, s.portuguese);
            });
        }
        Step::Name => {
            ui.label(
                RichText::new(s.onb_name_title)
                    .text_style(egui::TextStyle::Heading)
                    .strong(),
            );
            ui.add_space(16.0);
            text_field(ui, s.onb_your_name, &mut state.display_name, "", &mut focus);
            let mut no_focus = false;
            text_field(
                ui,
                s.onb_partner_name,
                &mut state.partner_name,
                "",
                &mut no_focus,
            );
        }
        Step::Balance => {
            ui.label(
                RichText::new(s.onb_balance_title)
                    .text_style(egui::TextStyle::Heading)
                    .strong(),
            );
            ui.add_space(8.0);
            ui.label(RichText::new(s.onb_balance_body).color(p.muted));
            ui.add_space(16.0);
            text_field(
                ui,
                s.onb_balance_label,
                &mut state.balance_input,
                "0,00",
                &mut focus,
            );
        }
        Step::Ready => {
            ui.label(
                RichText::new(s.onb_ready_title)
                    .text_style(egui::TextStyle::Heading)
                    .strong(),
            );
            ui.add_space(16.0);
            let summary = state.build_result().ok();
            Frame::new()
                .fill(p.card)
                .stroke(egui::Stroke::new(1.0, p.border))
                .corner_radius(egui::CornerRadius::same(16))
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let none = s.onb_none.to_string();
                    let (name, partner, balance) = summary.as_ref().map_or_else(
                        || (String::new(), none.clone(), String::new()),
                        |r| {
                            (
                                r.display_name.clone(),
                                r.partner_name.clone().unwrap_or_else(|| none.clone()),
                                crate::ui::components::summary_card::plain_brl(r.start_balance),
                            )
                        },
                    );
                    for (label, value) in [
                        (s.onb_summary_name, name),
                        (s.onb_summary_partner, partner),
                        (s.onb_balance_title, balance),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label).color(p.muted));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(value).font(typography::money_font()));
                            });
                        });
                    }
                });
            ui.add_space(16.0);
            ui.label(RichText::new(s.onb_ready_body).color(p.muted));
        }
    }
    state.focus_pending = focus;
}

enum Nav {
    Next,
    Back,
    Skip,
}

fn nav_row(ui: &mut egui::Ui, p: &Palette, state: &OnboardingState) -> Option<Nav> {
    let s = strings(state.language);
    let mut nav = None;
    ui.horizontal(|ui| {
        if state.step != Step::Welcome && ui.add_sized([96.0, 36.0], Button::new(s.back)).clicked()
        {
            nav = Some(Nav::Back);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if state.step == Step::Ready {
                s.start
            } else {
                s.continue_label
            };
            let primary = Button::new(RichText::new(label).color(p.bg).strong()).fill(p.accent);
            if ui.add_sized([140.0, 36.0], primary).clicked() {
                nav = Some(Nav::Next);
            }
            if state.step.is_optional()
                && ui
                    .add(Button::new(RichText::new(s.skip).color(p.muted)).frame(false))
                    .clicked()
            {
                nav = Some(Nav::Skip);
            }
        });
    });
    nav
}

/// Draw onboarding. Returns the finished setup when the user presses Start.
pub fn show(
    ui: &mut egui::Ui,
    theme: Theme,
    state: &mut OnboardingState,
) -> Option<OnboardingResult> {
    let p = palette(theme);
    let mut finished = None;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(p.bg))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            paint_background(ui, &p, rect);
            let width = CONTENT_WIDTH.min((rect.width() - 48.0).max(280.0));
            let left = rect.center().x - width / 2.0;
            let content = Rect::from_min_max(
                Pos2::new(left, rect.top() + rect.height() * 0.14),
                Pos2::new(left + width, rect.bottom()),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                ui.label(
                    RichText::new(strings(state.language).app_name)
                        .text_style(typography::section())
                        .strong()
                        .color(p.accent),
                );
                ui.add_space(8.0);
                progress_dots(ui, &p, state);
                ui.add_space(32.0);

                let fade = ui.ctx().animate_bool_with_time(
                    ui.make_persistent_id(("onboarding-step", state.step.index())),
                    true,
                    0.16,
                );
                ui.set_opacity(fade);
                ui.add_space((1.0 - fade) * 8.0);
                step_body(ui, &p, state);

                if let Some(issue) = state.error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(issue_message(issue, state.language)).color(p.negative));
                }
                ui.add_space(24.0);
                let mut nav = nav_row(ui, &p, state);
                if nav.is_none() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                    nav = Some(Nav::Next);
                }
                match nav {
                    Some(Nav::Next) => finished = state.advance(),
                    Some(Nav::Back) => state.back(),
                    Some(Nav::Skip) => state.skip(),
                    None => {}
                }
            });
        });
    finished
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The harness has no theme of its own; install ours the way the real app does.
    fn themed(ui: &mut egui::Ui, theme: Theme) {
        // The harness only has egui's bundled fonts, which lack some of our symbols.
        ui.ctx()
            .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
        crate::ui::theme::apply(ui.ctx(), theme);
        let style = ui.ctx().global_style();
        ui.set_style(style);
    }

    fn state_with_name() -> OnboardingState {
        let mut s = OnboardingState::new(Language::En);
        s.display_name = "A3".into();
        s
    }

    #[test]
    fn builds_result_and_validates_balance() {
        let mut s = state_with_name();
        s.balance_input = "1.234,56".into();
        let r = s.build_result().unwrap();
        assert_eq!(r.start_balance, Money::from_cents(123_456));
        assert_eq!(r.display_name, "A3");
        assert_eq!(r.partner_name, None);

        s.balance_input = "0,005".into();
        assert_eq!(
            s.build_result(),
            Err(OnboardingIssue::Balance(MoneyError::TooPrecise))
        );
    }

    #[test]
    fn a_blank_balance_is_zero_and_a_negative_one_is_allowed() {
        let mut s = state_with_name();
        assert_eq!(s.build_result().unwrap().start_balance, Money::ZERO);
        s.balance_input = "-50,00".into();
        assert_eq!(
            s.build_result().unwrap().start_balance,
            Money::from_cents(-5_000)
        );
    }

    #[test]
    fn the_partner_is_trimmed_and_optional() {
        let mut s = state_with_name();
        s.partner_name = "  Ana ".into();
        assert_eq!(
            s.build_result().unwrap().partner_name.as_deref(),
            Some("Ana")
        );
        s.partner_name = "   ".into();
        assert_eq!(s.build_result().unwrap().partner_name, None);
        s.partner_name = "a3".into();
        assert_eq!(
            s.build_result(),
            Err(OnboardingIssue::Name(OnboardingError::PartnerNameTaken))
        );
    }

    #[test]
    fn steps_advance_only_when_valid() {
        let mut s = OnboardingState::new(Language::PtBr);
        assert_eq!(s.progress(), (1, 4));
        assert!(s.advance().is_none());
        assert_eq!(s.step, Step::Name);

        assert!(s.advance().is_none());
        assert_eq!(s.step, Step::Name, "a blank name blocks");
        assert_eq!(
            s.error,
            Some(OnboardingIssue::Name(OnboardingError::NameRequired))
        );

        s.display_name = "A3".into();
        assert!(s.advance().is_none());
        assert_eq!((s.step, s.error), (Step::Balance, None));

        s.balance_input = "abc".into();
        assert!(s.advance().is_none());
        assert_eq!(s.step, Step::Balance);
        assert_eq!(s.error, Some(OnboardingIssue::Balance(MoneyError::Invalid)));

        s.balance_input = "10".into();
        s.advance();
        assert_eq!(s.step, Step::Ready);
        let done = s.advance().expect("the last step returns the result");
        assert_eq!(done.start_balance, Money::from_cents(1_000));
        assert_eq!(done.language, Language::PtBr);
    }

    #[test]
    fn skip_only_works_on_optional_steps_and_clears_the_field() {
        let mut s = state_with_name();
        s.step = Step::Name;
        s.skip();
        assert_eq!(s.step, Step::Name, "the name is required, never skippable");
        s.step = Step::Balance;
        s.balance_input = "junk".into();
        s.skip();
        assert_eq!(s.step, Step::Ready);
        assert!(s.balance_input.is_empty());
        assert!(s.build_result().is_ok());
    }

    #[test]
    fn back_walks_to_the_start_and_clears_errors() {
        let mut s = OnboardingState::new(Language::En);
        s.step = Step::Balance;
        s.error = Some(OnboardingIssue::SaveFailed);
        s.back();
        assert_eq!((s.step, s.error), (Step::Name, None));
        s.back();
        s.back();
        assert_eq!(s.step, Step::Welcome);
    }

    #[test]
    fn issues_are_localized() {
        for issue in [
            OnboardingIssue::Name(OnboardingError::NameRequired),
            OnboardingIssue::Name(OnboardingError::PartnerNameTaken),
            OnboardingIssue::Balance(MoneyError::TooPrecise),
            OnboardingIssue::SaveFailed,
        ] {
            assert_ne!(
                issue_message(issue, Language::En),
                issue_message(issue, Language::PtBr)
            );
        }
    }

    /// Drives the real screen: the background animates forever, so the harness steps a fixed count.
    #[test]
    fn the_screen_can_be_completed_through_the_ui() {
        use egui_kittest::kittest::Queryable;

        type App = (OnboardingState, Option<OnboardingResult>);
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, (state, done): &mut App| {
                themed(ui, Theme::Dark);
                if let Some(result) = show(ui, Theme::Dark, state) {
                    *done = Some(result);
                }
            },
            (OnboardingState::new(Language::En), None),
        );
        harness.run_steps(3);
        harness.get_by_label("Continue").click();
        harness.run_steps(3);
        assert_eq!(harness.state().0.step, Step::Name);

        // A blank name shows its error and stays on the step.
        harness.get_by_label("Continue").click();
        harness.run_steps(3);
        assert_eq!(harness.state().0.step, Step::Name);
        assert!(harness.query_by_label("Enter your name.").is_some());

        for (index, text) in ["A3", "Ana"].into_iter().enumerate() {
            harness
                .get_all_by_role(egui::Role::TextInput)
                .nth(index)
                .unwrap()
                .focus();
            harness.run_steps(2);
            harness
                .get_all_by_role(egui::Role::TextInput)
                .nth(index)
                .unwrap()
                .type_text(text);
            harness.run_steps(2);
        }
        assert_eq!(harness.state().0.display_name, "A3");
        harness.get_by_label("Continue").click();
        harness.run_steps(3);
        assert_eq!(harness.state().0.step, Step::Balance);

        harness.get_by_role(egui::Role::TextInput).focus();
        harness.run_steps(2);
        harness
            .get_by_role(egui::Role::TextInput)
            .type_text("1.234,56");
        harness.run_steps(2);
        harness.get_by_label("Continue").click();
        harness.run_steps(3);
        assert_eq!(harness.state().0.step, Step::Ready);
        assert!(harness.query_by_label("R$ 1.234,56").is_some());

        harness.get_by_label("Start").click();
        harness.run_steps(3);
        let result = harness
            .state()
            .1
            .clone()
            .expect("Start finishes onboarding");
        assert_eq!(result.display_name, "A3");
        assert_eq!(result.partner_name.as_deref(), Some("Ana"));
        assert_eq!(result.start_balance, Money::from_cents(123_456));
    }

    #[test]
    fn the_language_toggle_retranslates_the_screen() {
        use egui_kittest::kittest::Queryable;

        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, state: &mut OnboardingState| {
                themed(ui, Theme::Light);
                show(ui, Theme::Light, state);
            },
            OnboardingState::new(Language::En),
        );
        harness.run_steps(3);
        assert!(harness.query_by_label("Welcome to Athena").is_some());
        harness.get_by_label("Português").click();
        harness.run_steps(3);
        assert!(harness.query_by_label("Bem-vindo ao Athena").is_some());
        assert_eq!(harness.state().language, Language::PtBr);
    }

    /// Found by driving the real window: Enter in a text field makes it let go of the keyboard, so
    /// after a refused Enter the user had to click back in. The field must keep focus.
    #[test]
    fn a_refused_enter_keeps_the_keyboard_in_the_field() {
        use egui_kittest::kittest::Queryable;
        let mut state = OnboardingState::new(Language::En);
        state.display_name = "A3".into();
        state.step = Step::Balance;
        state.focus_pending = true;
        let mut harness = egui_kittest::Harness::new_ui_state(
            |ui, state: &mut OnboardingState| {
                themed(ui, Theme::Dark);
                show(ui, Theme::Dark, state);
            },
            state,
        );
        harness.run_steps(3);
        harness.get_by_role(egui::Role::TextInput).focus();
        harness.run_steps(2);
        harness
            .get_by_role(egui::Role::TextInput)
            .type_text("0,005");
        harness.run_steps(2);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(3);
        assert_eq!(harness.state().step, Step::Balance, "the bad amount blocks");
        assert!(
            harness.get_by_role(egui::Role::TextInput).is_focused(),
            "focus came back to the field"
        );
        // The user can keep typing without clicking.
        harness.get_by_role(egui::Role::TextInput).type_text("1");
        harness.run_steps(2);
        assert_eq!(harness.state().balance_input, "0,0051");
    }
}
