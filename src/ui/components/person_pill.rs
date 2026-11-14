//! A small rounded label for a person's name, or any short tag.

use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke};

use crate::domain::settings::Theme;
use crate::ui::theme::palette;

/// Always shows the full name, never initials alone, truncating with ellipsis only if wider
/// than `ui.available_width()`.
pub fn show(ui: &mut egui::Ui, theme: Theme, name: &str) {
    show_bounded(ui, theme, name, ui.available_width());
}

/// Draw the person pill within at most `max_width` pixels, truncating with a hover tooltip when
/// the label exceeds the available budget.
pub fn show_bounded(ui: &mut egui::Ui, theme: Theme, name: &str, max_width: f32) {
    let p = palette(theme);
    let inner_max = (max_width - 18.0).max(24.0);
    Frame::new()
        .fill(p.card_hover)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.set_max_width(inner_max);
            ui.add(
                egui::Label::new(
                    RichText::new(name)
                        .text_style(egui::TextStyle::Small)
                        .color(p.text),
                )
                .truncate(),
            )
            .on_hover_text(name);
        });
}
