//! Embedded Salted Flutters Polaris subsets, with Windows fonts as glyph fallbacks.
//! Main UI text and buttons use Semibold, and titles (section, heading, display) use Bold.
//! Money and table dates use the frozen-tabular Semibold face.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{Context, FontData, FontDefinitions, FontFamily, FontId, Style, TextStyle};

pub const BODY: f32 = 15.0;
pub const SECONDARY: f32 = 13.0;
pub const SECTION: f32 = 20.0;
pub const HEADING: f32 = 28.0;
pub const DISPLAY: f32 = 34.0;

const POLARIS_REGULAR: &str = "athena-polaris-regular";
const POLARIS_MEDIUM: &str = "athena-polaris-medium";
const POLARIS_SEMIBOLD: &str = "athena-polaris-semibold";
const POLARIS_BOLD: &str = "athena-polaris-bold";
const POLARIS_TABULAR: &str = "athena-polaris-tabular";
const POLARIS_DISPLAY: &str = "athena-polaris-display";
const POLARIS_UI_MEDIUM: &str = "athena-polaris-ui-medium";

const REGULAR_BYTES: &[u8] = include_bytes!("../../../assets/fonts/AthenaPolaris-Regular.otf");
const MEDIUM_BYTES: &[u8] = include_bytes!("../../../assets/fonts/AthenaPolaris-Medium.otf");
const SEMIBOLD_BYTES: &[u8] = include_bytes!("../../../assets/fonts/AthenaPolaris-Semibold.otf");
const BOLD_BYTES: &[u8] = include_bytes!("../../../assets/fonts/AthenaPolaris-Bold.otf");
const TABULAR_BYTES: &[u8] = include_bytes!("../../../assets/fonts/AthenaPolaris-Tabular.otf");

const SEGOE: (&str, &str) = ("segoeui", "segoeui.ttf");
const CONSOLAS: (&str, &str) = ("consola", "consola.ttf");
/// Fallback for symbols Segoe UI lacks, such as the refund arrow and sidebar icons.
const SEGOE_SYMBOLS: (&str, &str) = ("seguisym", "seguisym.ttf");

fn named_family(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

/// Bold family for section titles, headings, and display text.
#[must_use]
pub fn display_family() -> FontFamily {
    named_family(POLARIS_DISPLAY)
}

/// Semibold family for buttons and primary controls.
#[must_use]
pub fn medium_family() -> FontFamily {
    named_family(POLARIS_UI_MEDIUM)
}

/// Frozen-tabular face for money and table dates.
#[must_use]
pub fn tabular_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// `FontId` for 20 px section titles with the Bold face.
#[must_use]
pub fn section_font() -> FontId {
    FontId::new(SECTION, display_family())
}

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

/// Monospace family for monetary values and table dates. The first font is the
/// frozen-tabular Polaris face; Consolas and egui defaults are fallbacks.
#[must_use]
pub fn money_font() -> FontId {
    tabular_font(BODY)
}

/// Apply the size scale and family mapping to a style.
pub fn apply_text_styles(style: &mut Style) {
    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(SECONDARY, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(BODY, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(BODY, medium_family())),
        (TextStyle::Heading, FontId::new(HEADING, display_family())),
        (TextStyle::Monospace, tabular_font(BODY)),
        (section(), section_font()),
        (display(), FontId::new(DISPLAY, display_family())),
    ]
    .into();
}

fn windows_fonts_dir() -> PathBuf {
    let windir =
        std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    windir.join("Fonts")
}

fn embedded_definitions() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    for (name, bytes) in [
        (POLARIS_REGULAR, REGULAR_BYTES),
        (POLARIS_MEDIUM, MEDIUM_BYTES),
        (POLARIS_SEMIBOLD, SEMIBOLD_BYTES),
        (POLARIS_BOLD, BOLD_BYTES),
        (POLARIS_TABULAR, TABULAR_BYTES),
    ] {
        defs.font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    defs.families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, POLARIS_SEMIBOLD.to_owned());
    defs.families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, POLARIS_TABULAR.to_owned());
    defs
}

fn add_custom_families(defs: &mut FontDefinitions) {
    let fallback = defs
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (family, weight) in [
        (POLARIS_DISPLAY, POLARIS_BOLD),
        (POLARIS_UI_MEDIUM, POLARIS_SEMIBOLD),
    ] {
        let mut chain = vec![weight.to_owned()];
        if weight != POLARIS_SEMIBOLD {
            chain.push(POLARIS_SEMIBOLD.to_owned());
        }
        chain.extend(
            fallback
                .iter()
                .filter(|name| name.as_str() != POLARIS_SEMIBOLD)
                .cloned(),
        );
        defs.families.insert(named_family(family), chain);
    }
}

fn font_definitions_with(system_fonts: Option<&Path>) -> FontDefinitions {
    let mut defs = embedded_definitions();
    if let Some(fonts_dir) = system_fonts {
        let segoe_loaded = std::fs::read(fonts_dir.join(SEGOE.1)).is_ok();
        for (name, file, family, position) in [
            (SEGOE.0, SEGOE.1, FontFamily::Proportional, 1),
            (
                SEGOE_SYMBOLS.0,
                SEGOE_SYMBOLS.1,
                FontFamily::Proportional,
                usize::from(segoe_loaded) + 1,
            ),
            (CONSOLAS.0, CONSOLAS.1, FontFamily::Monospace, 1),
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
    }
    add_custom_families(&mut defs);
    defs
}

/// Font definitions with embedded Polaris first and any available system fonts after it.
#[must_use]
pub fn font_definitions(fonts_dir: &Path) -> FontDefinitions {
    font_definitions_with(Some(fonts_dir))
}

/// Deterministic definitions for snapshots and font tests: no host fonts.
#[cfg(test)]
pub(crate) fn test_font_definitions() -> FontDefinitions {
    font_definitions_with(None)
}

#[cfg(test)]
pub(crate) fn install_test_fonts(ctx: &Context) {
    ctx.set_fonts(test_font_definitions());
}

/// Install embedded Polaris plus Segoe UI and Consolas from the Windows fonts folder when present.
pub fn install_fonts(ctx: &Context) {
    ctx.set_fonts(font_definitions(&windows_fonts_dir()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> Context {
        let ctx = Context::default();
        install_test_fonts(&ctx);
        let mut output = ctx.run_ui(eframe::egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        ctx
    }

    #[test]
    fn polaris_is_primary_and_system_fonts_follow() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("segoeui.ttf"), b"not a real font").unwrap();
        std::fs::write(dir.path().join("seguisym.ttf"), b"not a real font").unwrap();
        std::fs::write(dir.path().join("consola.ttf"), b"not a real font").unwrap();
        let defs = font_definitions(dir.path());
        let proportional = &defs.families[&FontFamily::Proportional];
        assert_eq!(proportional[0], POLARIS_SEMIBOLD);
        assert_eq!(proportional[1], SEGOE.0);
        assert_eq!(proportional[2], SEGOE_SYMBOLS.0);
        let monospace = &defs.families[&FontFamily::Monospace];
        assert_eq!(monospace[0], POLARIS_TABULAR);
        assert_eq!(monospace[1], CONSOLAS.0);
    }

    #[test]
    fn missing_system_fonts_keep_polaris_primary() {
        let dir = tempfile::tempdir().unwrap();
        let defs = font_definitions(dir.path());
        assert_eq!(
            defs.families[&FontFamily::Proportional][0],
            POLARIS_SEMIBOLD
        );
        assert_eq!(defs.families[&FontFamily::Monospace][0], POLARIS_TABULAR);
    }

    #[test]
    fn custom_families_start_with_the_right_weight() {
        let defs = test_font_definitions();
        assert_eq!(defs.families[&display_family()][0], POLARIS_BOLD);
        assert_eq!(defs.families[&medium_family()][0], POLARIS_SEMIBOLD);
    }

    #[test]
    fn embedded_fonts_cover_portuguese_and_money() {
        let ctx = test_context();
        let body = FontId::new(BODY, FontFamily::Proportional);
        let money = money_font();
        ctx.fonts_mut(|fonts| {
            assert!(fonts.has_glyphs(&body, "R$ 1.234,56 − + ãõçáéíóúâêôàü"));
            assert!(fonts.has_glyphs(&money, "R$ 1.234,56 − +"));
        });
    }

    #[test]
    fn money_digits_have_one_advance_width() {
        let ctx = test_context();
        let money = money_font();
        let widths: Vec<f32> = ctx.fonts_mut(|fonts| {
            "0123456789"
                .chars()
                .map(|ch| fonts.glyph_width(&money, ch))
                .collect()
        });
        assert!(
            widths
                .windows(2)
                .all(|pair| (pair[0] - pair[1]).abs() < 0.01),
            "money digits differ: {widths:?}"
        );

        let proportional = FontId::new(BODY, FontFamily::Proportional);
        let proportional_widths: Vec<f32> = ctx.fonts_mut(|fonts| {
            "0123456789"
                .chars()
                .map(|ch| fonts.glyph_width(&proportional, ch))
                .collect()
        });
        assert!(
            proportional_widths
                .iter()
                .any(|width| (width - proportional_widths[0]).abs() > 1.0),
            "Proportional digits should stay proportional: {proportional_widths:?}"
        );
    }

    #[test]
    fn the_symbol_font_sits_right_after_segoe_ui() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["segoeui.ttf", "seguisym.ttf"] {
            std::fs::write(dir.path().join(file), b"not a real font").unwrap();
        }
        let proportional = &font_definitions(dir.path()).families[&FontFamily::Proportional];
        assert_eq!(
            &proportional[..3],
            [POLARIS_SEMIBOLD, "segoeui", "seguisym"]
        );
        let only_symbols = tempfile::tempdir().unwrap();
        std::fs::write(only_symbols.path().join("seguisym.ttf"), b"x").unwrap();
        let only_proportional =
            &font_definitions(only_symbols.path()).families[&FontFamily::Proportional];
        assert_eq!(&only_proportional[..2], [POLARIS_SEMIBOLD, "seguisym"]);
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

    #[test]
    fn text_styles_use_the_polaris_families() {
        let mut style = Style::default();
        apply_text_styles(&mut style);
        assert_eq!(
            style.text_styles[&TextStyle::Body].family,
            FontFamily::Proportional
        );
        assert_eq!(
            style.text_styles[&TextStyle::Small].family,
            FontFamily::Proportional
        );
        assert_eq!(
            style.text_styles[&TextStyle::Button].family,
            medium_family()
        );
        assert_eq!(
            style.text_styles[&TextStyle::Heading].family,
            display_family()
        );
        assert_eq!(style.text_styles[&section()].family, display_family());
        assert_eq!(style.text_styles[&display()].family, display_family());
        assert_eq!(
            style.text_styles[&TextStyle::Monospace].family,
            FontFamily::Monospace
        );
        assert_eq!(section_font().family, display_family());
    }
}
