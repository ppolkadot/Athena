//! A modal that asks before an action that cannot be undone.

use eframe::egui::{self, Align, Button, Id, Layout, RichText};

use crate::app::actions::AppAction;
use crate::domain::settings::{Language, Theme};
use crate::i18n::strings;
use crate::ui::theme::{palette, typography};

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmDialog {
    pub title: String,
    pub body: String,
    pub confirm_label: String,
    /// Draws the confirm button in the warning color.
    pub danger: bool,
    /// Applied only if the user confirms.
    pub action: AppAction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmChoice {
    Confirmed(AppAction),
    Cancelled,
}

/// Draw the dialog. Returns the user's choice once they make one (Esc counts as cancel).
#[allow(clippy::must_use_candidate)]
pub fn show(
    ctx: &egui::Context,
    dialog: &ConfirmDialog,
    theme: Theme,
    language: Language,
) -> Option<ConfirmChoice> {
    let p = palette(theme);
    let s = strings(language);
    let mut choice = None;
    let modal = egui::Modal::new(Id::unique("confirm-dialog")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(
            RichText::new(&dialog.title)
                .text_style(typography::section())
                .strong(),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(&dialog.body).color(p.muted));
        ui.add_space(16.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let color = if dialog.danger { p.negative } else { p.accent };
            let confirm =
                Button::new(RichText::new(&dialog.confirm_label).color(p.bg).strong()).fill(color);
            if ui.add_sized([120.0, 36.0], confirm).clicked() {
                choice = Some(ConfirmChoice::Confirmed(dialog.action.clone()));
            }
            if ui.add_sized([96.0, 36.0], Button::new(s.cancel)).clicked() {
                choice = Some(ConfirmChoice::Cancelled);
            }
        });
    });
    if choice.is_none() && modal.should_close() {
        choice = Some(ConfirmChoice::Cancelled);
    }
    choice
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> ConfirmDialog {
        ConfirmDialog {
            title: "Delete this transaction?".into(),
            body: "Market · R$ 45,90 will be removed for good.".into(),
            confirm_label: "Delete".into(),
            danger: true,
            action: AppAction::DeleteTransaction(7),
        }
    }

    fn harness() -> egui_kittest::Harness<'static, Option<ConfirmChoice>> {
        egui_kittest::Harness::new_ui_state(
            |ui, choice: &mut Option<ConfirmChoice>| {
                ui.ctx()
                    .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                crate::ui::theme::apply(ui.ctx(), Theme::Dark);
                let style = ui.ctx().global_style();
                ui.set_style(style);
                if let Some(c) = show(ui.ctx(), &dialog(), Theme::Dark, Language::En) {
                    *choice = Some(c);
                }
            },
            None,
        )
    }

    #[test]
    fn the_dialog_names_what_it_will_do_and_confirms() {
        use egui_kittest::kittest::Queryable;
        let mut harness = harness();
        harness.run_steps(3);
        assert!(harness.query_by_label("Delete this transaction?").is_some());
        assert!(
            harness
                .query_by_label("Market · R$ 45,90 will be removed for good.")
                .is_some()
        );
        harness.get_by_label("Delete").click();
        harness.run_steps(3);
        assert_eq!(
            *harness.state(),
            Some(ConfirmChoice::Confirmed(AppAction::DeleteTransaction(7)))
        );
    }

    #[test]
    fn cancel_does_nothing_destructive() {
        use egui_kittest::kittest::Queryable;
        let mut harness = harness();
        harness.run_steps(3);
        harness.get_by_label("Cancel").click();
        harness.run_steps(3);
        assert_eq!(*harness.state(), Some(ConfirmChoice::Cancelled));
    }

    #[test]
    fn escape_cancels() {
        let mut harness = harness();
        harness.run_steps(3);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(3);
        assert_eq!(*harness.state(), Some(ConfirmChoice::Cancelled));
    }
}
