//! Resolves applied or previewed preferences plus platform state into the look one frame paints
//! with (contract §14.2). Pure: nothing here reads the clock, the OS or the database.

use eframe::egui::Color32;

use super::active::StyleKey;
use super::palette::{Palette, composite, contrast_ratio, stock_palette};
use crate::domain::appearance::{
    Accent, AppearancePrefs, Density, Fit, MotionPref, Quality, ThemeMode, Wallpaper,
};
use crate::domain::settings::Theme;
use crate::infrastructure::platform::{PlatformPrefs, Rgb, SystemColors};

/// Selected-surface and pressed-control accent tints, as the egui style composites them.
const SELECTED_ALPHA: u8 = 36;
const PRESSED_ALPHA: u8 = 50;

fn color((r, g, b): (u8, u8, u8)) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn rgb(c: Rgb) -> Color32 {
    color((c[0], c[1], c[2]))
}

/// Whether `p` meets the contrast the shell relies on for its accent: accent text on every
/// surface and selected fill, focus rings, text on pressed and selected fills, and button text.
fn accent_states_valid(p: &Palette) -> bool {
    let surfaces = [p.bg, p.panel, p.card, p.card_hover];
    let accent_ok = surfaces.iter().all(|&s| {
        contrast_ratio(p.accent, s) >= 4.5
            && contrast_ratio(p.focus, s) >= 3.0
            && contrast_ratio(p.text, composite(p.accent, PRESSED_ALPHA, s)) >= 4.5
            && contrast_ratio(p.text, composite(p.accent, SELECTED_ALPHA, s)) >= 4.5
    }) && [p.bg, p.panel, p.card].iter().all(|&s| {
        // Accent-colored text on a selected row or nav item (a hovered selection keeps text color).
        contrast_ratio(p.accent, composite(p.accent, SELECTED_ALPHA, s)) >= 4.5
    });
    accent_ok && contrast_ratio(p.on_accent, p.accent) >= 4.5
}

/// A safe foreground for filled accent controls: the stock choice, else black or white.
fn on_accent_for(accent: Color32, stock: Color32) -> Color32 {
    [stock, Color32::BLACK, Color32::WHITE]
        .into_iter()
        .max_by(|a, b| {
            contrast_ratio(*a, accent)
                .partial_cmp(&contrast_ratio(*b, accent))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(stock)
}

/// The palette for `theme` with `accent`. A candidate that fails the composed-state checks falls
/// back to the stock blue palette, so no accent can make text illegible.
#[must_use]
pub fn accent_palette(theme: Theme, accent: Accent) -> Palette {
    let stock = stock_palette(theme);
    let candidate = match (accent, theme) {
        (Accent::Blue, _) => return stock,
        (Accent::Teal, Theme::Dark) => (0x59, 0xDD, 0xCC),
        (Accent::Teal, Theme::Light) => (0x00, 0x6F, 0x66),
        (Accent::Violet, Theme::Dark) => (0xC2, 0xAC, 0xFF),
        (Accent::Violet, Theme::Light) => (0x59, 0x39, 0xA6),
    };
    let accent_color = color(candidate);
    let derived = Palette {
        accent: accent_color,
        focus: accent_color,
        on_accent: on_accent_for(accent_color, stock.on_accent),
        ..stock
    };
    if accent_states_valid(&derived) {
        derived
    } else {
        stock
    }
}

fn luminance_dark(c: Color32) -> bool {
    contrast_ratio(c, Color32::BLACK) < contrast_ratio(c, Color32::WHITE)
}

/// The opaque high-contrast palette: the Windows `Window`, `WindowText`, `Highlight` and `HighlightText`
/// colors when they are coherent, otherwise a fixed black background with white text.
#[must_use]
pub fn high_contrast_palette(colors: Option<SystemColors>) -> (Theme, Palette) {
    let fallback = SystemColors {
        window: [0, 0, 0],
        window_text: [255, 255, 255],
        highlight: [255, 255, 0],
        highlight_text: [0, 0, 0],
    };
    let valid = |c: &SystemColors| {
        contrast_ratio(rgb(c.window_text), rgb(c.window)) >= 7.0
            && contrast_ratio(rgb(c.highlight_text), rgb(c.highlight)) >= 4.5
    };
    let c = colors.filter(valid).unwrap_or(fallback);
    let (window, text, highlight, highlight_text) = (
        rgb(c.window),
        rgb(c.window_text),
        rgb(c.highlight),
        rgb(c.highlight_text),
    );
    let theme = if luminance_dark(window) {
        Theme::Dark
    } else {
        Theme::Light
    };
    let base = stock_palette(theme);
    let on_window = |preferred: Color32, floor: f32| {
        if contrast_ratio(preferred, window) >= floor {
            preferred
        } else {
            text
        }
    };
    let accent = if contrast_ratio(highlight, window) >= 4.5 {
        highlight
    } else {
        text
    };
    let p = Palette {
        bg: window,
        panel: window,
        card: window,
        card_hover: window,
        border: text,
        text,
        muted: text,
        accent,
        positive: on_window(base.positive, 4.5),
        negative: on_window(base.negative, 4.5),
        warning: on_window(base.warning, 4.5),
        focus: accent,
        on_accent: if contrast_ratio(highlight_text, accent) >= 4.5 {
            highlight_text
        } else {
            window
        },
    };
    (theme, p)
}

/// How the wallpaper chrome is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Material {
    /// Translucent chrome over the wallpaper.
    Tint,
    /// Opaque chrome; the canvas may still show a stationary wallpaper.
    Solid,
}

/// The look for one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct EffectiveAppearance {
    pub theme: Theme,
    pub palette: Palette,
    pub text_scale: u16,
    pub density: Density,
    pub high_contrast: bool,
    /// Decorative motion is off (reduced preference, OS setting, unknown OS state or high contrast).
    pub reduced_motion: bool,
    /// Reduced motion was imposed by the OS or high contrast rather than chosen by the user.
    pub motion_forced: bool,
    pub material: Material,
    /// Solid chrome was imposed by the OS, unknown OS state or high contrast.
    pub transparency_forced: bool,
    /// `None` under high contrast.
    pub wallpaper: Wallpaper,
    pub fit: Fit,
    pub dim: u8,
    pub paused: bool,
    pub quality: Quality,
}

impl EffectiveAppearance {
    /// The part of the look that changes egui styling.
    #[must_use]
    pub fn style_key(&self) -> StyleKey {
        StyleKey {
            theme: self.theme,
            palette: self.palette,
            text_scale: self.text_scale,
            compact: self.density == Density::Compact,
        }
    }
}

/// Resolve `prefs` (applied or previewed) against the legacy profile theme, the OS theme signal
/// and the platform state. Unknown platform state resolves to the accessible choice.
#[must_use]
pub fn resolve(
    prefs: &AppearancePrefs,
    legacy_theme: Theme,
    os_theme: Option<Theme>,
    platform: PlatformPrefs,
) -> EffectiveAppearance {
    let hc = platform.high_contrast.filter(|h| h.on);
    let high_contrast = hc.is_some();
    let (theme, palette) = if let Some(h) = hc {
        high_contrast_palette(h.colors)
    } else {
        let theme = match prefs.theme_mode {
            ThemeMode::Dark => Theme::Dark,
            ThemeMode::Light => Theme::Light,
            ThemeMode::System => os_theme.unwrap_or(legacy_theme),
        };
        (theme, accent_palette(theme, prefs.accent))
    };
    let os_motion = platform.animations == Some(true);
    let reduced_motion = high_contrast
        || match prefs.motion {
            MotionPref::Reduced => true,
            MotionPref::Full => false,
            MotionPref::System => !os_motion,
        };
    let motion_forced = reduced_motion
        && (high_contrast
            || (prefs.motion == MotionPref::System && platform.animations != Some(true)));
    let os_transparent = platform.transparency == Some(true);
    let transparency_forced = high_contrast || !os_transparent;
    let solid = high_contrast || prefs.reduce_transparency || !os_transparent;
    EffectiveAppearance {
        theme,
        palette,
        text_scale: prefs.text_scale.percent(),
        density: prefs.density,
        high_contrast,
        reduced_motion,
        motion_forced,
        material: if solid {
            Material::Solid
        } else {
            Material::Tint
        },
        transparency_forced,
        wallpaper: if high_contrast {
            Wallpaper::None
        } else {
            prefs.wallpaper.clone()
        },
        fit: prefs.fit,
        dim: prefs.dim,
        paused: prefs.paused,
        quality: prefs.quality,
    }
}

/// The appearance for a profile with no stored row (what Reset returns to).
#[must_use]
pub fn resolve_defaults(legacy_theme: Theme, platform: PlatformPrefs) -> EffectiveAppearance {
    resolve(
        &AppearancePrefs::defaults_for(legacy_theme),
        legacy_theme,
        None,
        platform,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::appearance::TextScale;
    use crate::infrastructure::platform::HighContrast;

    fn all_on() -> PlatformPrefs {
        PlatformPrefs {
            animations: Some(true),
            transparency: Some(true),
            high_contrast: Some(HighContrast {
                on: false,
                colors: None,
            }),
        }
    }

    #[test]
    fn every_accent_keeps_composed_states_legible_in_both_themes() {
        for theme in [Theme::Dark, Theme::Light] {
            for accent in Accent::ALL {
                let p = accent_palette(theme, *accent);
                assert!(accent_states_valid(&p), "{theme:?} {accent:?}");
            }
        }
        // The specified candidates are used, not the fallback.
        assert_eq!(
            accent_palette(Theme::Dark, Accent::Teal).accent,
            Color32::from_rgb(0x59, 0xDD, 0xCC)
        );
        assert_eq!(
            accent_palette(Theme::Light, Accent::Teal).accent,
            Color32::from_rgb(0x00, 0x6F, 0x66)
        );
        assert_eq!(
            accent_palette(Theme::Dark, Accent::Violet).accent,
            Color32::from_rgb(0xC2, 0xAC, 0xFF)
        );
        assert_eq!(
            accent_palette(Theme::Light, Accent::Violet).accent,
            Color32::from_rgb(0x59, 0x39, 0xA6)
        );
        assert_eq!(
            accent_palette(Theme::Dark, Accent::Blue),
            stock_palette(Theme::Dark)
        );
    }

    #[test]
    fn unsafe_accent_candidates_fall_back_to_stock() {
        // A palette whose accent equals the canvas can never pass.
        let p = Palette {
            accent: stock_palette(Theme::Dark).bg,
            focus: stock_palette(Theme::Dark).bg,
            ..stock_palette(Theme::Dark)
        };
        assert!(!accent_states_valid(&p));
    }

    #[test]
    fn theme_resolution_covers_system_with_and_without_a_signal() {
        let prefs = AppearancePrefs {
            theme_mode: ThemeMode::System,
            ..AppearancePrefs::default()
        };
        assert_eq!(
            resolve(&prefs, Theme::Light, Some(Theme::Dark), all_on()).theme,
            Theme::Dark
        );
        assert_eq!(
            resolve(&prefs, Theme::Light, None, all_on()).theme,
            Theme::Light,
            "no OS signal falls back to the legacy theme"
        );
        let dark = AppearancePrefs {
            theme_mode: ThemeMode::Dark,
            ..AppearancePrefs::default()
        };
        assert_eq!(
            resolve(&dark, Theme::Light, Some(Theme::Light), all_on()).theme,
            Theme::Dark
        );
    }

    #[test]
    fn motion_and_transparency_follow_prefs_then_the_os() {
        let base = AppearancePrefs::default();
        let e = resolve(&base, Theme::Dark, None, all_on());
        assert!(!e.reduced_motion && e.material == Material::Tint);
        // Unknown platform state is the accessible choice.
        let e = resolve(&base, Theme::Dark, None, PlatformPrefs::default());
        assert!(e.reduced_motion && e.motion_forced);
        assert!(e.material == Material::Solid && e.transparency_forced);
        // Full opts into motion regardless of the OS.
        let full = AppearancePrefs {
            motion: MotionPref::Full,
            ..AppearancePrefs::default()
        };
        let e = resolve(
            &full,
            Theme::Dark,
            None,
            PlatformPrefs {
                animations: Some(false),
                ..all_on()
            },
        );
        assert!(!e.reduced_motion);
        // OS animation off under System is forced, a user Reduced is not.
        let e = resolve(
            &base,
            Theme::Dark,
            None,
            PlatformPrefs {
                animations: Some(false),
                ..all_on()
            },
        );
        assert!(e.reduced_motion && e.motion_forced);
        let reduced = AppearancePrefs {
            motion: MotionPref::Reduced,
            ..AppearancePrefs::default()
        };
        let e = resolve(&reduced, Theme::Dark, None, all_on());
        assert!(e.reduced_motion && !e.motion_forced);
        // User Reduce transparency is not "forced".
        let solid = AppearancePrefs {
            reduce_transparency: true,
            ..AppearancePrefs::default()
        };
        let e = resolve(&solid, Theme::Dark, None, all_on());
        assert!(e.material == Material::Solid && !e.transparency_forced);
        let e = resolve(
            &base,
            Theme::Dark,
            None,
            PlatformPrefs {
                transparency: Some(false),
                ..all_on()
            },
        );
        assert!(e.material == Material::Solid && e.transparency_forced);
    }

    #[test]
    fn high_contrast_forces_solid_reduced_and_omits_the_wallpaper() {
        let prefs = AppearancePrefs {
            wallpaper: Wallpaper::Builtin,
            motion: MotionPref::Full,
            text_scale: TextScale::P150,
            accent: Accent::Violet,
            ..AppearancePrefs::default()
        };
        let platform = PlatformPrefs {
            high_contrast: Some(HighContrast {
                on: true,
                colors: Some(SystemColors {
                    window: [0, 0, 0],
                    window_text: [255, 255, 255],
                    highlight: [26, 235, 255],
                    highlight_text: [0, 0, 0],
                }),
            }),
            ..all_on()
        };
        let e = resolve(&prefs, Theme::Light, None, platform);
        assert!(e.high_contrast && e.reduced_motion && e.motion_forced);
        assert_eq!(e.material, Material::Solid);
        assert_eq!(e.wallpaper, Wallpaper::None);
        assert_eq!(e.palette.bg, Color32::BLACK);
        assert_eq!(e.text_scale, 150, "text scale is still honored");
        for surface in [e.palette.bg, e.palette.card, e.palette.card_hover] {
            assert!(contrast_ratio(e.palette.text, surface) >= 7.0);
            assert!(contrast_ratio(e.palette.accent, surface) >= 4.5);
        }
    }

    #[test]
    fn incoherent_system_colors_use_the_tested_fallback_palette() {
        let (theme, p) = high_contrast_palette(Some(SystemColors {
            window: [10, 10, 10],
            window_text: [20, 20, 20],
            highlight: [30, 30, 30],
            highlight_text: [40, 40, 40],
        }));
        assert_eq!(theme, Theme::Dark);
        assert_eq!(p.text, Color32::WHITE);
        let (_, none) = high_contrast_palette(None);
        assert_eq!(none, p);
        for surface in [p.bg, p.card, p.card_hover] {
            for fg in [p.text, p.muted, p.accent, p.positive, p.negative, p.warning] {
                assert!(contrast_ratio(fg, surface) >= 4.5);
            }
            assert!(contrast_ratio(p.on_accent, p.accent) >= 4.5);
        }
        // A light system scheme resolves to the light base.
        let (light, lp) = high_contrast_palette(Some(SystemColors {
            window: [255, 255, 255],
            window_text: [0, 0, 0],
            highlight: [0, 0, 160],
            highlight_text: [255, 255, 255],
        }));
        assert_eq!(light, Theme::Light);
        assert_eq!(lp.bg, Color32::WHITE);
        assert!(contrast_ratio(lp.negative, lp.bg) >= 4.5);
    }

    #[test]
    fn style_key_changes_only_for_styling_inputs() {
        let base = resolve(&AppearancePrefs::default(), Theme::Dark, None, all_on());
        let mut other = base.clone();
        other.dim = 80;
        other.paused = true;
        other.fit = Fit::Fit;
        assert_eq!(base.style_key(), other.style_key());
        other.text_scale = 130;
        assert_ne!(base.style_key(), other.style_key());
        let compact = AppearancePrefs {
            density: Density::Compact,
            ..AppearancePrefs::default()
        };
        assert!(
            resolve(&compact, Theme::Dark, None, all_on())
                .style_key()
                .compact
        );
    }

    #[test]
    fn missing_row_resolves_to_the_legacy_theme_whatever_the_language() {
        for theme in [Theme::Dark, Theme::Light] {
            let e = resolve_defaults(theme, all_on());
            assert_eq!(e.theme, theme);
            assert_eq!(e.palette, stock_palette(theme));
            assert_eq!(e.text_scale, 100);
        }
    }
}
