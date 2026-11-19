//! Top-level screens.

use eframe::egui::Key;

use crate::domain::settings::Language;
use crate::i18n::strings;
use crate::ui::icons::Icon;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Screen {
    #[default]
    Month,
    Calendar,
    Goals,
    Trash,
    Settings,
}

/// Explicit Ctrl+number navigation order, independent of [`Screen::ALL`] sidebar order.
pub const SHORTCUTS: [(Key, Screen); 5] = [
    (Key::Num1, Screen::Month),
    (Key::Num2, Screen::Calendar),
    (Key::Num3, Screen::Settings),
    (Key::Num4, Screen::Trash),
    (Key::Num5, Screen::Goals),
];

impl Screen {
    pub const ALL: [Screen; 5] = [
        Screen::Month,
        Screen::Calendar,
        Screen::Goals,
        Screen::Trash,
        Screen::Settings,
    ];

    #[must_use]
    pub fn title(self, language: Language) -> &'static str {
        let s = strings(language);
        match self {
            Screen::Month => s.month,
            Screen::Calendar => s.calendar,
            Screen::Goals => s.goals,
            Screen::Trash => s.trash,
            Screen::Settings => s.settings,
        }
    }

    /// Embedded icon for the sidebar and rail.
    #[must_use]
    pub fn icon(self) -> Icon {
        match self {
            Screen::Month => Icon::List,
            Screen::Calendar => Icon::CalendarDays,
            Screen::Goals => Icon::Target,
            Screen::Trash => Icon::Trash2,
            Screen::Settings => Icon::Settings,
        }
    }
}

impl Screen {
    /// The digit of this screen's `Ctrl+digit` shortcut, for the visible hint.
    #[must_use]
    pub fn shortcut_digit(self) -> Option<u8> {
        let (key, _) = SHORTCUTS.iter().find(|(_, screen)| *screen == self)?;
        match key {
            Key::Num1 => Some(1),
            Key::Num2 => Some(2),
            Key::Num3 => Some(3),
            Key::Num4 => Some(4),
            Key::Num5 => Some(5),
            _ => None,
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
        assert_eq!(Screen::Goals.title(Language::En), "Goals");
        assert_eq!(Screen::Goals.title(Language::PtBr), "Metas");
    }

    #[test]
    fn every_screen_has_an_icon_and_the_list_is_complete() {
        assert_eq!(Screen::ALL.len(), 5);
        for screen in Screen::ALL {
            assert!(!screen.icon().paths().is_empty());
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
                (Key::Num5, Screen::Goals),
            ]
        );
        assert_eq!(Screen::ALL[2], Screen::Goals);
        assert_eq!(Screen::ALL[3], Screen::Trash);
        assert_eq!(Screen::ALL[4], Screen::Settings);
        for (index, (_, screen)) in SHORTCUTS.iter().enumerate() {
            assert_eq!(screen.shortcut_digit(), u8::try_from(index + 1).ok());
        }
    }
}
