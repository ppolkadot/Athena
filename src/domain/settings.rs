//! Theme, language and the profile singleton. Defined once; no other module redefines them.

use serde::{Deserialize, Serialize};

use super::money::Money;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    #[serde(rename = "dark")]
    Dark,
    #[serde(rename = "light")]
    Light,
}

impl Theme {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "dark" => Some(Theme::Dark),
            "light" => Some(Theme::Light),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    En,
    #[serde(rename = "pt-BR")]
    PtBr,
}

impl Language {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::PtBr => "pt-BR",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "en" => Some(Language::En),
            "pt-BR" => Some(Language::PtBr),
            _ => None,
        }
    }

    /// Any system locale starting with `pt` selects Portuguese; everything else is English.
    #[must_use]
    pub fn from_locale(locale: Option<&str>) -> Self {
        match locale {
            Some(l) if l.to_ascii_lowercase().starts_with("pt") => Language::PtBr,
            _ => Language::En,
        }
    }

    #[must_use]
    pub fn from_system() -> Self {
        Self::from_locale(sys_locale::get_locale().as_deref())
    }
}

/// The singleton profile row. It has no display name: the owner lives in `people`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub start_balance: Money,
    pub theme: Theme,
    pub language: Language,
    pub onboarding_complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_round_trip() {
        assert_eq!(Language::En.code(), "en");
        assert_eq!(Language::PtBr.code(), "pt-BR");
        assert_eq!(Language::from_code("pt-BR"), Some(Language::PtBr));
        assert_eq!(Language::from_code("en"), Some(Language::En));
        assert_eq!(Language::from_code("xx"), None);
    }

    #[test]
    fn locale_detection_prefers_portuguese_only_for_pt() {
        assert_eq!(Language::from_locale(Some("pt-BR")), Language::PtBr);
        assert_eq!(Language::from_locale(Some("PT_pt")), Language::PtBr);
        assert_eq!(Language::from_locale(Some("en-US")), Language::En);
        assert_eq!(Language::from_locale(None), Language::En);
    }

    #[test]
    fn theme_codes_and_defaults() {
        assert_eq!(Theme::default(), Theme::Dark);
        assert_eq!(Theme::from_code("light"), Some(Theme::Light));
        assert_eq!(Theme::Dark.code(), "dark");
        assert_eq!(Theme::from_code("blue"), None);
    }

    #[test]
    fn serde_uses_the_schema_codes() {
        assert_eq!(serde_json::to_string(&Language::PtBr).unwrap(), "\"pt-BR\"");
        assert_eq!(serde_json::to_string(&Theme::Light).unwrap(), "\"light\"");
        assert_eq!(
            serde_json::to_string(&Money::from_cents(123)).unwrap(),
            "123"
        );
    }
}
