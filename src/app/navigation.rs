//! Top-level screens.

use eframe::egui::Key;

use crate::domain::settings::Language;
use crate::i18n::strings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Screen {
    #[default]
    Month,
    Calendar,
    Trash,
    Settings,
}

/// Explicit Ctrl+number navigation order, independent of [`Screen::ALL`] sidebar order.
pub const SHORTCUTS: [(Key, Screen); 4] = [
    (Key::Num1, Screen::Month),
    (Key::Num2, Screen::Calendar),
    (Key::Num3, Screen::Settings),
    (Key::Num4, Screen::Trash),
];

impl Screen {
    pub const ALL: [Screen; 4] = [
        Screen::Month,
        Screen::Calendar,
        Screen::Trash,
        Screen::Settings,
    ];

    #[must_use]
    pub fn title(self, language: Language) -> &'static str {
        let s = strings(language);
        match self {
            Screen::Month => s.month,
            Screen::Calendar => s.calendar,
            Screen::Trash => s.trash,
            Screen::Settings => s.settings,
        }
    }

    /// Small inline glyph for the sidebar.
    #[must_use]
    pub fn icon(self) -> &'static str {
        match self {
            Screen::Month => "☰",
            Screen::Calendar => "▦",
            Screen::Trash => "🗑",
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
        assert_eq!(Screen::Trash.title(Language::En), "Trash");
        assert_eq!(Screen::Trash.title(Language::PtBr), "Lixeira");
        assert_eq!(Screen::Settings.title(Language::En), "Settings");
    }

    #[test]
    fn every_screen_has_an_icon_and_the_list_is_complete() {
        assert_eq!(Screen::ALL.len(), 4);
        for screen in Screen::ALL {
            assert!(!screen.icon().is_empty());
        }
        assert_eq!(Screen::default(), Screen::Month);
    }

    #[test]
    fn shortcuts_follow_the_explicit_table() {
        assert_eq!(
            SHORTCUTS,
            [
                (Key::Num1, Screen::Month),
                (Key::Num2, Screen::Calendar),
                (Key::Num3, Screen::Settings),
                (Key::Num4, Screen::Trash),
            ]
        );
        assert_eq!(Screen::ALL[2], Screen::Trash);
        assert_eq!(Screen::ALL[3], Screen::Settings);
    }
}
