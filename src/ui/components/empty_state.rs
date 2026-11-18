//! Friendly placeholder for a screen with nothing to show: a glyph, one sentence of guidance and
//! an optional primary action.

use eframe::egui::{self, Align, Button, Layout, RichText};

use crate::domain::settings::Theme;
use crate::ui::icons::{self, Icon};
use crate::ui::theme::{palette, typography};

/// Returns whether the action button was clicked.
pub fn show(
    ui: &mut egui::Ui,
    theme: Theme,
    icon: Icon,
    title: &str,
    hint: &str,
    action: Option<&str>,
) -> bool {
    let p = palette(theme);
    let mut clicked = false;
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.add_space(32.0);
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(40.0), egui::Sense::hover());
        icons::paint(ui, rect, icon, p.muted);
        ui.add_space(8.0);
        ui.label(
            RichText::new(title)
                .text_style(typography::section())
                .strong(),
        );
        ui.label(RichText::new(hint).color(p.muted));
        if let Some(label) = action {
            ui.add_space(16.0);
            let button =
                Button::new(RichText::new(label).color(p.on_accent).strong()).fill(p.accent);
            clicked = ui.add_sized([180.0, 36.0], button).clicked();
        }
        ui.add_space(32.0);
    });
    clicked
}
