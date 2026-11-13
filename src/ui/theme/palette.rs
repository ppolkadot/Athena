//! Color tokens from the design spec, §12. Dark is a near-black canvas; light is a full peer theme.

use eframe::egui::Color32;

use crate::domain::settings::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub positive: Color32,
    pub negative: Color32,
    pub warning: Color32,
}

#[allow(clippy::cast_possible_truncation)]
const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

#[must_use]
pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette {
            bg: rgb(0x05_06_08),
            panel: rgb(0x0A_0C_10),
            card: rgb(0x0F_12_18),
            card_hover: rgb(0x15_1A_22),
            border: rgb(0x23_2A_36),
            text: rgb(0xEC_EE_F4),
            muted: rgb(0x8B_95_A7),
            accent: rgb(0x6C_8C_FF),
            positive: rgb(0x3D_DC_97),
            negative: rgb(0xFF_6B_7A),
            warning: rgb(0xF5_C4_51),
        },
        Theme::Light => Palette {
            bg: rgb(0xF6_F7_FB),
            panel: rgb(0xFF_FF_FF),
            card: rgb(0xFF_FF_FF),
            card_hover: rgb(0xF0_F2_F8),
            border: rgb(0xE2_E5_EE),
            text: rgb(0x14_18_21),
            muted: rgb(0x5C_64_76),
            accent: rgb(0x4F_6B_FF),
            positive: rgb(0x1F_9E_6E),
            negative: rgb(0xD6_45_5A),
            warning: rgb(0xB0_7B_12),
        },
    }
}

/// `#RRGGBB` to a color; `None` for anything else.
#[must_use]
pub fn parse_hex(text: &str) -> Option<Color32> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    Some(rgb(value))
}

fn linear(channel: u8) -> f32 {
    let c = f32::from(channel) / 255.0;
    if c <= 0.039_28 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(color: Color32) -> f32 {
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

/// WCAG 2 contrast ratio between two opaque colors, `1.0..=21.0`.
#[must_use]
pub fn contrast_ratio(fg: Color32, bg: Color32) -> f32 {
    let (a, b) = (luminance(fg), luminance(bg));
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_light_palettes_differ() {
        assert_ne!(palette(Theme::Dark).bg, palette(Theme::Light).bg);
        assert_eq!(palette(Theme::Dark).bg, Color32::from_rgb(0x05, 0x06, 0x08));
        assert_eq!(
            palette(Theme::Light).bg,
            Color32::from_rgb(0xF6, 0xF7, 0xFB)
        );
        assert_eq!(
            palette(Theme::Dark).accent,
            Color32::from_rgb(0x6C, 0x8C, 0xFF)
        );
        assert_eq!(
            palette(Theme::Dark).card,
            Color32::from_rgb(0x0F, 0x12, 0x18)
        );
    }

    #[test]
    fn text_colors_meet_wcag_aa_on_every_surface() {
        for theme in [Theme::Dark, Theme::Light] {
            let p = palette(theme);
            for surface in [p.bg, p.panel, p.card, p.card_hover] {
                assert!(
                    contrast_ratio(p.text, surface) >= 4.5,
                    "{theme:?} text on {surface:?}"
                );
                assert!(
                    contrast_ratio(p.muted, surface) >= 4.5,
                    "{theme:?} muted on {surface:?}"
                );
            }
        }
    }

    #[test]
    fn amount_and_accent_colors_are_readable_as_large_text() {
        for theme in [Theme::Dark, Theme::Light] {
            let p = palette(theme);
            for color in [p.accent, p.positive, p.negative, p.warning] {
                assert!(
                    contrast_ratio(color, p.bg) >= 3.0,
                    "{theme:?} {color:?} on bg"
                );
                assert!(
                    contrast_ratio(color, p.card) >= 3.0,
                    "{theme:?} {color:?} on card"
                );
            }
        }
    }

    #[test]
    fn hex_colors_parse_or_are_rejected() {
        assert_eq!(
            parse_hex("#FF8A65"),
            Some(Color32::from_rgb(0xFF, 0x8A, 0x65))
        );
        assert_eq!(parse_hex("#ff8a65"), parse_hex("#FF8A65"));
        assert_eq!(parse_hex("FF8A65"), None);
        assert_eq!(parse_hex("#FFF"), None);
        assert_eq!(parse_hex("#GG0000"), None);
    }

    #[test]
    fn contrast_ratio_matches_known_values() {
        let white = Color32::WHITE;
        let black = Color32::BLACK;
        assert!((contrast_ratio(white, black) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(white, white) - 1.0).abs() < 0.001);
    }
}
