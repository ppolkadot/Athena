//! A Keep editing / Discard prompt for unsaved drafts. Keep editing is the default; Esc keeps.

use eframe::egui::{self, Align, Button, Id, Layout, RichText};

use crate::domain::settings::{Language, Theme};
use crate::i18n::strings;
use crate::ui::theme::{palette, typography};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardChoice {
    Keep,
    Discard,
}

/// Draw the prompt. `focus_keep` moves focus to Keep editing on the frame it is true.
#[allow(clippy::must_use_candidate)]
pub fn show(
    ctx: &egui::Context,
    theme: Theme,
    language: Language,
    title: &str,
    body: &str,
    focus_keep: bool,
) -> Option<DiscardChoice> {
    let p = palette(theme);
    let s = strings(language);
    let mut choice = None;
    let modal = egui::Modal::new(Id::unique("discard-prompt")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(
            RichText::new(title)
                .text_style(typography::section())
                .strong(),
        );
        ui.add_space(8.0);
        ui.label(RichText::new(body).color(p.muted));
        ui.add_space(16.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let discard = Button::new(RichText::new(s.qa_discard).color(p.bg).strong())
                .fill(p.negative)
                .min_size(egui::Vec2::new(110.0, 36.0));
            if ui.add(discard).clicked() {
                choice = Some(DiscardChoice::Discard);
            }
            let keep =
                ui.add(Button::new(s.qa_keep_editing).min_size(egui::Vec2::new(120.0, 36.0)));
            if focus_keep {
                keep.request_focus();
            }
            if keep.clicked() {
                choice = Some(DiscardChoice::Keep);
            }
        });
    });
    if choice.is_none() && modal.should_close() {
        choice = Some(DiscardChoice::Keep);
    }
    choice
}
