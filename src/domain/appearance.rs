//! Appearance preferences (contract §14.2): closed enums and ranges, validated once here and again
//! by the `appearance` table's CHECKs. Colors and rendering live in the UI layer.

use crate::shared::error::{AppError, Result};

/// Define a closed enum with stable wire codes.
macro_rules! code_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $code:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            #[must_use]
            pub fn code(self) -> &'static str {
                match self { $(Self::$variant => $code),+ }
            }

            #[must_use]
            pub fn from_code(code: &str) -> Option<Self> {
                match code { $($code => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

code_enum!(
    /// `system` follows the OS theme signal and falls back to the legacy profile theme.
    ThemeMode { System => "system", Dark => "dark", Light => "light" }
);
code_enum!(
    /// A named starting point: it sets theme and accent in the draft only.
    Preset { Onyx => "onyx", Pearl => "pearl", Aurora => "aurora" }
);
code_enum!(Accent { Blue => "blue", Teal => "teal", Violet => "violet" });
code_enum!(Density { Comfortable => "comfortable", Compact => "compact" });
code_enum!(
    /// Decorative motion: `system` follows the OS animation preference.
    MotionPref { System => "system", Reduced => "reduced", Full => "full" }
);
code_enum!(
    /// Wallpaper decode and render tier.
    Quality { Auto => "auto", Low => "low", High => "high" }
);
code_enum!(Fit { Fit => "fit", Fill => "fill" });
code_enum!(
    /// The wire form of the background choice.
    WallpaperKind { None => "none", Builtin => "builtin", Local => "local" }
);

/// Text scale as a closed percentage allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextScale {
    P100,
    P115,
    P130,
    P150,
}

impl TextScale {
    pub const ALL: [Self; 4] = [Self::P100, Self::P115, Self::P130, Self::P150];

    #[must_use]
    pub fn percent(self) -> u16 {
        match self {
            Self::P100 => 100,
            Self::P115 => 115,
            Self::P130 => 130,
            Self::P150 => 150,
        }
    }

    #[must_use]
    pub fn from_percent(percent: i64) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|s| i64::from(s.percent()) == percent)
    }
}

/// An opaque managed-media id: exactly 32 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AssetId(String);

impl AssetId {
    /// # Errors
    /// [`AppError::Validation`] unless `text` is 32 lowercase hex digits.
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() == 32 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            Ok(Self(text.to_owned()))
        } else {
            Err(AppError::Validation("invalid wallpaper asset id".into()))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The chosen wallpaper. A local choice carries its managed asset id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Wallpaper {
    #[default]
    None,
    Builtin,
    Local(AssetId),
}

impl Wallpaper {
    #[must_use]
    pub fn kind(&self) -> WallpaperKind {
        match self {
            Self::None => WallpaperKind::None,
            Self::Builtin => WallpaperKind::Builtin,
            Self::Local(_) => WallpaperKind::Local,
        }
    }

    #[must_use]
    pub fn asset_id(&self) -> Option<&AssetId> {
        match self {
            Self::Local(id) => Some(id),
            _ => None,
        }
    }

    /// Rebuild from the two stored columns.
    ///
    /// # Errors
    /// [`AppError::Validation`] for a missing, unexpected or malformed asset id.
    pub fn from_parts(kind: WallpaperKind, asset_id: Option<&str>) -> Result<Self> {
        match (kind, asset_id) {
            (WallpaperKind::None, None) => Ok(Self::None),
            (WallpaperKind::Builtin, None) => Ok(Self::Builtin),
            (WallpaperKind::Local, Some(id)) => Ok(Self::Local(AssetId::parse(id)?)),
            _ => Err(AppError::Validation(
                "wallpaper kind and asset id disagree".into(),
            )),
        }
    }
}

/// Highest allowed dim value.
pub const MAX_DIM: u8 = 100;

/// The full preference row. `Default` is what a missing row and Reset resolve to (the theme mode
/// is `Dark` here only as a stand-in; resolution maps a missing row to the legacy profile theme).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppearancePrefs {
    pub theme_mode: ThemeMode,
    pub preset: Preset,
    pub accent: Accent,
    pub density: Density,
    pub text_scale: TextScale,
    pub motion: MotionPref,
    pub reduce_transparency: bool,
    pub quality: Quality,
    pub wallpaper: Wallpaper,
    pub fit: Fit,
    /// Requested dimming, `0..=100`; clamped to the compositor floor at render time.
    pub dim: u8,
    pub paused: bool,
}

impl Default for AppearancePrefs {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::Dark,
            preset: Preset::Onyx,
            accent: Accent::Blue,
            density: Density::Comfortable,
            text_scale: TextScale::P100,
            motion: MotionPref::System,
            reduce_transparency: false,
            quality: Quality::Auto,
            wallpaper: Wallpaper::None,
            fit: Fit::Fill,
            dim: 0,
            paused: false,
        }
    }
}

impl AppearancePrefs {
    /// The defaults a missing row resolves to: the legacy theme becomes an explicit mode so the
    /// settings page starts from what the user already sees.
    #[must_use]
    pub fn defaults_for(legacy: super::settings::Theme) -> Self {
        use super::settings::Theme;
        match legacy {
            Theme::Dark => Self::default(),
            Theme::Light => Self {
                theme_mode: ThemeMode::Light,
                preset: Preset::Pearl,
                ..Self::default()
            },
        }
    }

    /// The theme, accent and preset a recipe sets (Onyx: Dark + blue, Pearl: Light + blue, Aurora:
    /// Dark + teal). Background and accessibility choices are untouched.
    #[must_use]
    pub fn with_preset(mut self, preset: Preset) -> Self {
        let (mode, accent) = match preset {
            Preset::Onyx => (ThemeMode::Dark, Accent::Blue),
            Preset::Pearl => (ThemeMode::Light, Accent::Blue),
            Preset::Aurora => (ThemeMode::Dark, Accent::Teal),
        };
        self.preset = preset;
        self.theme_mode = mode;
        self.accent = accent;
        self
    }

    /// Whether theme and accent still match the selected preset's recipe.
    #[must_use]
    pub fn matches_preset_recipe(&self) -> bool {
        let recipe = Self::default().with_preset(self.preset);
        self.theme_mode == recipe.theme_mode && self.accent == recipe.accent
    }

    /// # Errors
    /// [`AppError::Validation`] when `dim` is out of range. The enum fields are closed by type.
    pub fn validate(&self) -> Result<()> {
        if self.dim > MAX_DIM {
            return Err(AppError::Validation("dim must be between 0 and 100".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::settings::Theme;

    #[test]
    fn every_code_round_trips_and_unknown_codes_are_rejected() {
        for t in ThemeMode::ALL {
            assert_eq!(ThemeMode::from_code(t.code()), Some(*t));
        }
        for t in Preset::ALL {
            assert_eq!(Preset::from_code(t.code()), Some(*t));
        }
        for t in Accent::ALL {
            assert_eq!(Accent::from_code(t.code()), Some(*t));
        }
        assert_eq!(Density::from_code("tight"), None);
        assert_eq!(MotionPref::from_code("System"), None);
        assert_eq!(Quality::from_code(""), None);
        assert_eq!(Fit::from_code("stretch"), None);
        assert_eq!(WallpaperKind::from_code("video"), None);
    }

    #[test]
    fn text_scale_allowlist_is_closed() {
        for p in [100_u16, 115, 130, 150] {
            assert_eq!(TextScale::from_percent(i64::from(p)).unwrap().percent(), p);
        }
        for p in [0, 99, 101, 120, 151, 200, -100] {
            assert_eq!(TextScale::from_percent(p), None);
        }
    }

    #[test]
    fn asset_ids_are_32_lowercase_hex() {
        let ok = "0123456789abcdef0123456789abcdef";
        assert_eq!(AssetId::parse(ok).unwrap().as_str(), ok);
        for bad in [
            "",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "../123456789abcdef0123456789abcd",
            "0123456789abcdef0123456789abcdeg",
        ] {
            assert!(AssetId::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn wallpaper_parts_must_agree() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(
            Wallpaper::from_parts(WallpaperKind::None, None).unwrap(),
            Wallpaper::None
        );
        assert!(Wallpaper::from_parts(WallpaperKind::None, Some(id)).is_err());
        assert!(Wallpaper::from_parts(WallpaperKind::Builtin, Some(id)).is_err());
        assert!(Wallpaper::from_parts(WallpaperKind::Local, None).is_err());
        assert_eq!(
            Wallpaper::from_parts(WallpaperKind::Local, Some(id))
                .unwrap()
                .kind(),
            WallpaperKind::Local
        );
    }

    #[test]
    fn dim_boundaries() {
        for dim in [0, 1, 99, 100] {
            let prefs = AppearancePrefs {
                dim,
                ..AppearancePrefs::default()
            };
            assert!(prefs.validate().is_ok());
        }
        let prefs = AppearancePrefs {
            dim: 101,
            ..AppearancePrefs::default()
        };
        assert!(prefs.validate().is_err());
    }

    #[test]
    fn presets_set_theme_and_accent_only() {
        let base = AppearancePrefs {
            wallpaper: Wallpaper::Builtin,
            dim: 40,
            reduce_transparency: true,
            ..AppearancePrefs::default()
        };
        let aurora = base.clone().with_preset(Preset::Aurora);
        assert_eq!(
            (aurora.theme_mode, aurora.accent),
            (ThemeMode::Dark, Accent::Teal)
        );
        assert_eq!(aurora.wallpaper, Wallpaper::Builtin);
        assert!(aurora.reduce_transparency);
        let pearl = base.with_preset(Preset::Pearl);
        assert_eq!(
            (pearl.theme_mode, pearl.accent),
            (ThemeMode::Light, Accent::Blue)
        );
        assert!(pearl.matches_preset_recipe());
        let changed = AppearancePrefs {
            accent: Accent::Violet,
            ..pearl
        };
        assert!(!changed.matches_preset_recipe());
    }

    #[test]
    fn defaults_follow_the_legacy_theme() {
        assert_eq!(
            AppearancePrefs::defaults_for(Theme::Light).theme_mode,
            ThemeMode::Light
        );
        assert_eq!(
            AppearancePrefs::defaults_for(Theme::Dark),
            AppearancePrefs::default()
        );
    }
}
