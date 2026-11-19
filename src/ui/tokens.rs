//! Shared layout tokens (contract §14.1): spacing, radii, control sizes and shell widths. Semantic
//! colors stay in [`crate::ui::theme::Palette`]; type sizes stay in [`crate::ui::theme::typography`].

use eframe::egui::Color32;

use crate::ui::theme::{Palette, composite};

/// Spacing steps in points.
pub const SPACE_XS: f32 = 4.0;
pub const SPACE_S: f32 = 8.0;
pub const SPACE_M: f32 = 12.0;
pub const SPACE_L: f32 = 16.0;
pub const SPACE_XL: f32 = 24.0;

/// Card and control corner radii in points.
pub const CARD_RADIUS: u8 = 16;
pub const CONTROL_RADIUS: u8 = 10;

/// Default minimum control height.
pub const CONTROL_HEIGHT: f32 = 36.0;
/// Border width.
pub const BORDER_WIDTH: f32 = 1.0;
/// Icon paint size inside a control and the stroke the icon set is drawn with, in 24-unit space.
pub const ICON_SIZE: f32 = 20.0;

/// Expanded sidebar width.
pub const SIDEBAR_WIDTH: f32 = 220.0;
/// Collapsed icon rail width.
pub const RAIL_WIDTH: f32 = 64.0;
/// Content column cap.
pub const CONTENT_MAX_WIDTH: f32 = 1100.0;
/// Content narrower than this collapses the sidebar to the icon rail.
pub const NARROW_CONTENT: f32 = 480.0;

/// Opacity of the selected-surface accent tint, out of 255.
pub const SELECTED_ALPHA: u8 = 36;

fn step(comfortable: f32, compact: f32) -> f32 {
    if crate::ui::theme::active::compact() {
        compact
    } else {
        comfortable
    }
}

/// 8 pt, or 6 pt in compact density.
#[must_use]
pub fn space_s() -> f32 {
    step(SPACE_S, 6.0)
}

/// 12 pt, or 8 pt in compact density.
#[must_use]
pub fn space_m() -> f32 {
    step(SPACE_M, 8.0)
}

/// 16 pt, or 12 pt in compact density.
#[must_use]
pub fn space_l() -> f32 {
    step(SPACE_L, 12.0)
}

/// 24 pt, or 18 pt in compact density.
#[must_use]
pub fn space_xl() -> f32 {
    step(SPACE_XL, 18.0)
}

/// 36 pt controls, or a 32 pt minimum in compact density.
#[must_use]
pub fn control_height() -> f32 {
    step(CONTROL_HEIGHT, 32.0)
}

/// The accent tint over `surface`: the selected row/nav fill as actually composited.
#[must_use]
pub fn selected_fill(p: &Palette, surface: Color32) -> Color32 {
    composite(p.accent, SELECTED_ALPHA, surface)
}

/// Whether a viewport leaves the sidebar enough room beside 480 pt of content.
#[must_use]
pub fn is_narrow(viewport_width: f32) -> bool {
    viewport_width - SIDEBAR_WIDTH - 2.0 * SPACE_XL < NARROW_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::settings::Theme;
    use crate::ui::theme::{contrast_ratio, palette};

    fn surfaces(p: &Palette) -> Vec<(&'static str, Color32)> {
        let mut out = vec![
            ("bg", p.bg),
            ("panel", p.panel),
            ("card", p.card),
            ("card_hover", p.card_hover),
        ];
        for (name, base) in [("panel", p.panel), ("card", p.card), ("bg", p.bg)] {
            out.push((
                match name {
                    "panel" => "selected panel",
                    "card" => "selected card",
                    _ => "selected bg",
                },
                selected_fill(p, base),
            ));
        }
        out
    }

    #[test]
    fn design_tokens_meet_contrast_thresholds_in_dark_and_light() {
        for theme in [Theme::Dark, Theme::Light] {
            let p = palette(theme);
            for (surface_name, surface) in surfaces(&p) {
                for (role, color) in [
                    ("text", p.text),
                    ("weak/muted", p.muted),
                    ("accent", p.accent),
                    ("positive", p.positive),
                    ("negative", p.negative),
                    ("warning", p.warning),
                ] {
                    // Weak text is not drawn on selected fills.
                    if role == "weak/muted" && surface_name.starts_with("selected") {
                        continue;
                    }
                    let ratio = contrast_ratio(color, surface);
                    assert!(
                        ratio >= 4.5,
                        "{theme:?} {role} on {surface_name}: {ratio:.2} < 4.5"
                    );
                }
                // Focus ring and essential icon boundaries need 3:1.
                for (role, color) in [
                    ("focus", p.focus),
                    ("icon", p.text),
                    ("border icon", p.muted),
                ] {
                    let ratio = contrast_ratio(color, surface);
                    assert!(
                        ratio >= 3.0,
                        "{theme:?} {role} on {surface_name}: {ratio:.2} < 3.0"
                    );
                }
            }
            let ratio = contrast_ratio(p.on_accent, p.accent);
            assert!(ratio >= 4.5, "{theme:?} primary button text: {ratio:.2}");
        }
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn layout_tokens_match_the_contract() {
        assert_eq!(
            [SPACE_XS, SPACE_S, SPACE_M, SPACE_L, SPACE_XL],
            [4.0, 8.0, 12.0, 16.0, 24.0]
        );
        assert_eq!((CARD_RADIUS, CONTROL_RADIUS), (16, 10));
        assert!((CONTROL_HEIGHT - 36.0).abs() < f32::EPSILON);
        assert!((SIDEBAR_WIDTH - 220.0).abs() < f32::EPSILON);
        assert!((CONTENT_MAX_WIDTH - 1100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn narrow_threshold_collapses_below_480_pt_of_content() {
        // 220 sidebar + 48 margins + 480 content = 748.
        assert!(is_narrow(747.0));
        assert!(!is_narrow(748.0));
        assert!(is_narrow(640.0));
    }
}
