//! Top-level screens.

use crate::domain::settings::Language;
use crate::i18n::strings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Screen {
    #[default]
    Month,
    Calendar,
    Settings,
}

impl Screen {
    pub const ALL: [Screen; 3] = [Screen::Month, Screen::Calendar, Screen::Settings];

    #[must_use]
    pub fn title(self, language: Language) -> &'static str {
        let s = strings(language);
        match self {
            Screen::Month => s.month,
            Screen::Calendar => s.calendar,
            Screen::Settings => s.settings,
        }
    }

    /// Small inline glyph for the sidebar.
    #[must_use]
    pub fn icon(self) -> &'static str {
        match self {
            Screen::Month => "☰",
            Screen::Calendar => "▦",
            Screen::Settings => "⚙",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screens_have_titles() {
        assert_eq!(Screen::Month.title(Language::En), "Month");
        assert_eq!(Screen::Calendar.title(Language::PtBr), "Calendário");
        assert_eq!(Screen::Settings.title(Language::En), "Settings");
    }

    #[test]
    fn every_screen_has_an_icon_and_the_list_is_complete() {
        assert_eq!(Screen::ALL.len(), 3);
        for screen in Screen::ALL {
            assert!(!screen.icon().is_empty());
        }
        assert_eq!(Screen::default(), Screen::Month);
    }
}
