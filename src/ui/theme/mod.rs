//! Applies the palette, spacing and type scale to egui for both themes.

pub mod palette;
pub mod typography;

use eframe::egui::{self, Color32, CornerRadius, Stroke, Style, Vec2, Visuals};

pub use palette::{Palette, composite, contrast_ratio, palette, parse_hex};
pub use typography::install_fonts;

use crate::domain::settings::Theme;

const CARD_RADIUS: u8 = 16;
const CONTROL_RADIUS: u8 = 10;
const SELECTION_BG_ALPHA: u8 = 36;

fn egui_theme(theme: Theme) -> egui::Theme {
    match theme {
        Theme::Dark => egui::Theme::Dark,
        Theme::Light => egui::Theme::Light,
    }
}

fn tint(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// Build the egui style for one theme.
fn configure(style: &mut Style, p: &Palette, theme: Theme) {
    let mut v = if theme == Theme::Dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    let border = Stroke::new(1.0, p.border);
    let control = CornerRadius::same(CONTROL_RADIUS);

    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.window_stroke = border;
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.menu_corner_radius = CornerRadius::same(CONTROL_RADIUS);
    v.extreme_bg_color = p.panel;
    v.faint_bg_color = p.card;
    v.code_bg_color = p.card_hover;
    v.weak_text_color = Some(p.muted);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.negative;
    v.selection.bg_fill = tint(p.accent, SELECTION_BG_ALPHA);
    v.selection.stroke = Stroke::new(1.0, p.accent);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = border;
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = control;

    w.inactive.bg_fill = p.card;
    w.inactive.weak_bg_fill = p.card;
    w.inactive.bg_stroke = border;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = control;

    // Hover changes border and background only; no movement.
    w.hovered.bg_fill = p.card_hover;
    w.hovered.weak_bg_fill = p.card_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.corner_radius = control;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = tint(p.accent, 50);
    w.active.weak_bg_fill = tint(p.accent, 50);
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.active.corner_radius = control;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.card_hover;
    w.open.weak_bg_fill = p.card_hover;
    w.open.bg_stroke = border;
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = control;

    style.visuals = v;
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 8.0);
    style.spacing.interact_size = Vec2::new(0.0, 36.0);
    typography::apply_text_styles(style);
}

/// Configure both egui styles from the palette and select `theme`. Safe to call every frame
/// the theme changes; fonts are installed separately, once, with [`install_fonts`].
pub fn apply(ctx: &egui::Context, theme: Theme) {
    for t in [Theme::Dark, Theme::Light] {
        ctx.style_mut_of(egui_theme(t), |style| configure(style, &palette(t), t));
    }
    ctx.set_theme(match theme {
        Theme::Dark => egui::ThemePreference::Dark,
        Theme::Light => egui::ThemePreference::Light,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_sets_the_canvas_and_selects_the_theme() {
        let ctx = egui::Context::default();
        apply(&ctx, Theme::Dark);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            palette(Theme::Dark).bg
        );
        assert_eq!(
            ctx.style_of(egui::Theme::Light).visuals.panel_fill,
            palette(Theme::Light).bg
        );
        assert!(ctx.style_of(egui::Theme::Dark).visuals.dark_mode);
        apply(&ctx, Theme::Light);
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::Light
        );
    }

    #[test]
    fn controls_are_36_tall_with_the_card_radius() {
        let ctx = egui::Context::default();
        apply(&ctx, Theme::Dark);
        let style = ctx.style_of(egui::Theme::Dark);
        assert!((style.spacing.interact_size.y - 36.0).abs() < f32::EPSILON);
        assert_eq!(style.visuals.window_corner_radius, CornerRadius::same(16));
    }

    #[test]
    fn selected_visuals_meet_wcag_aa_contrast() {
        let ctx = egui::Context::default();
        apply(&ctx, Theme::Dark);
        for theme in [Theme::Dark, Theme::Light] {
            let p = palette(theme);
            let style = ctx.style_of(egui_theme(theme));
            let sel_fill = style.visuals.selection.bg_fill;
            let sel_fg = style.visuals.selection.stroke.color;
            for surface in [p.bg, p.panel, p.card, p.card_hover] {
                let composited = surface.blend(sel_fill);
                let ratio = contrast_ratio(sel_fg, composited);
                assert!(
                    ratio >= 4.5,
                    "{theme:?} selected control on {surface:?}: {ratio:.2} < 4.5"
                );
            }
        }
    }
}
