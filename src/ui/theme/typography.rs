//! Fonts and text sizes. Segoe UI for text and Consolas for numbers when present (Windows),
//! otherwise egui's bundled fonts.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily, FontId, Style, TextStyle};

pub const BODY: f32 = 15.0;
pub const SECONDARY: f32 = 13.0;
pub const SECTION: f32 = 20.0;
pub const HEADING: f32 = 28.0;
pub const DISPLAY: f32 = 34.0;

const SEGOE: (&str, &str) = ("segoeui", "segoeui.ttf");
const CONSOLAS: (&str, &str) = ("consola", "consola.ttf");
/// Fallback for symbols Segoe UI lacks, such as the refund arrow and sidebar icons.
const SEGOE_SYMBOLS: (&str, &str) = ("seguisym", "seguisym.ttf");

/// `TextStyle` for 20 px section titles.
#[must_use]
pub fn section() -> TextStyle {
    TextStyle::Name("section".into())
}

/// `TextStyle` for 34 px display text (wordmark, onboarding).
#[must_use]
pub fn display() -> TextStyle {
    TextStyle::Name("display".into())
}

/// Monospace face for monetary values and table dates.
#[must_use]
pub fn money_font() -> FontId {
    FontId::new(BODY, FontFamily::Monospace)
}

/// Apply the size scale to a style.
pub fn apply_text_styles(style: &mut Style) {
    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(SECONDARY, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(BODY, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(BODY, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(HEADING, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(BODY, FontFamily::Monospace),
        ),
        (section(), FontId::new(SECTION, FontFamily::Proportional)),
        (display(), FontId::new(DISPLAY, FontFamily::Proportional)),
    ]
    .into();
}

fn windows_fonts_dir() -> PathBuf {
    let windir =
        std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    windir.join("Fonts")
}

/// Font definitions with any available system fonts placed first. Missing or unreadable files are
/// skipped silently, leaving egui's defaults.
#[must_use]
pub fn font_definitions(fonts_dir: &Path) -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let segoe_loaded = std::fs::read(fonts_dir.join(SEGOE.1)).is_ok();
    for (name, file, family, position) in [
        (SEGOE.0, SEGOE.1, FontFamily::Proportional, 0),
        // Right after Segoe UI, so text keeps its look and only missing symbols fall through.
        (
            SEGOE_SYMBOLS.0,
            SEGOE_SYMBOLS.1,
            FontFamily::Proportional,
            usize::from(segoe_loaded),
        ),
        (CONSOLAS.0, CONSOLAS.1, FontFamily::Monospace, 0),
    ] {
        if let Ok(bytes) = std::fs::read(fonts_dir.join(file)) {
            defs.font_data
                .insert(name.to_string(), Arc::new(FontData::from_owned(bytes)));
            defs.families
                .entry(family)
                .or_default()
                .insert(position, name.to_string());
        }
    }
    defs
}

/// Install Segoe UI and Consolas from the Windows fonts folder when present.
pub fn install_fonts(ctx: &Context) {
    ctx.set_fonts(font_definitions(&windows_fonts_dir()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fonts_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let defs = font_definitions(dir.path());
        assert!(!defs.font_data.contains_key("segoeui"));
        assert_eq!(
            defs.families[&FontFamily::Proportional],
            FontDefinitions::default().families[&FontFamily::Proportional]
        );
    }

    #[test]
    fn found_fonts_go_first_in_their_family() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("segoeui.ttf"), b"not a real font").unwrap();
        std::fs::write(dir.path().join("consola.ttf"), b"not a real font").unwrap();
        let defs = font_definitions(dir.path());
        assert_eq!(defs.families[&FontFamily::Proportional][0], "segoeui");
        assert_eq!(defs.families[&FontFamily::Monospace][0], "consola");
        assert!(
            defs.families[&FontFamily::Proportional].len() > 1,
            "defaults stay as fallback"
        );
    }

    #[test]
    fn the_symbol_font_sits_right_after_segoe_ui() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["segoeui.ttf", "seguisym.ttf"] {
            std::fs::write(dir.path().join(file), b"not a real font").unwrap();
        }
        let proportional = &font_definitions(dir.path()).families[&FontFamily::Proportional];
        assert_eq!(&proportional[..2], ["segoeui", "seguisym"]);
        let only_symbols = tempfile::tempdir().unwrap();
        std::fs::write(only_symbols.path().join("seguisym.ttf"), b"x").unwrap();
        assert_eq!(
            font_definitions(only_symbols.path()).families[&FontFamily::Proportional][0],
            "seguisym"
        );
    }

    #[test]
    fn text_styles_follow_the_scale() {
        let mut style = Style::default();
        apply_text_styles(&mut style);
        assert!((style.text_styles[&TextStyle::Body].size - 15.0).abs() < f32::EPSILON);
        assert!((style.text_styles[&TextStyle::Small].size - 13.0).abs() < f32::EPSILON);
        assert!((style.text_styles[&section()].size - 20.0).abs() < f32::EPSILON);
        assert!((style.text_styles[&TextStyle::Heading].size - 28.0).abs() < f32::EPSILON);
        assert!((style.text_styles[&display()].size - 34.0).abs() < f32::EPSILON);
    }
}
