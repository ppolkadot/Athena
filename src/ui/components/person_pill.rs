//! A small rounded label for a person's name, or any short tag.

use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke};

use crate::domain::settings::Theme;
use crate::ui::theme::palette;

/// Always shows the full name, never initials alone.
pub fn show(ui: &mut egui::Ui, theme: Theme, name: &str) {
    let p = palette(theme);
    Frame::new()
        .fill(p.card_hover)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                RichText::new(name)
                    .text_style(egui::TextStyle::Small)
                    .color(p.text),
            );
        });
}
