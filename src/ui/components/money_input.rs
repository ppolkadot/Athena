//! A money text field with live feedback: a formatted preview when the text is a valid amount
//! and a localized message when it is not. Parsing is the strict `Money::parse`; nothing rounds.

use eframe::egui::{self, Align2, RichText, TextEdit, Vec2};

use crate::domain::money::{Money, MoneyError};
use crate::domain::settings::{Language, Theme};
use crate::i18n::money_error_message;
use crate::ui::theme::{palette, typography};

pub struct MoneyInput {
    pub response: egui::Response,
    /// Why the current text is not an amount. `None` for blank or valid text.
    pub error: Option<MoneyError>,
}

/// Blank text is neither valid nor an error: the caller decides whether a blank is allowed.
#[must_use]
pub fn check(text: &str) -> (Option<Money>, Option<MoneyError>) {
    if text.trim().is_empty() {
        return (None, None);
    }
    match Money::parse(text) {
        Ok(money) => (Some(money), None),
        Err(e) => (None, Some(e)),
    }
}

/// Draw `label`, the field, and its preview or error. Pass `focus = true` to take keyboard focus.
pub fn show(
    ui: &mut egui::Ui,
    theme: Theme,
    language: Language,
    label: &str,
    value: &mut String,
    focus: bool,
) -> MoneyInput {
    let p = palette(theme);
    let caption = ui.label(RichText::new(label).text_style(egui::TextStyle::Small));
    let response = ui.add(
        TextEdit::singleline(value)
            .font(typography::money_font())
            .align(Align2::RIGHT_CENTER)
            .hint_text("0,00")
            .desired_width(f32::INFINITY)
            .margin(Vec2::new(12.0, 10.0)),
    );
    let response = response.labelled_by(caption.id);
    if focus {
        response.request_focus();
    }
    let (money, error) = check(value);
    if let Some(error) = error {
        ui.label(RichText::new(money_error_message(error, language)).color(p.negative));
    } else if let Some(money) = money {
        ui.label(
            RichText::new(super::summary_card::plain_brl(money))
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
    }
    ui.add_space(4.0);
    MoneyInput { response, error }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_separates_blank_valid_and_invalid() {
        assert_eq!(check("  "), (None, None));
        assert_eq!(check("1.234,56"), (Some(Money::from_cents(123_456)), None));
        assert_eq!(check("0,005"), (None, Some(MoneyError::TooPrecise)));
        assert_eq!(check("abc"), (None, Some(MoneyError::Invalid)));
    }
}
