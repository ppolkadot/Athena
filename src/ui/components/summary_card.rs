//! A compact labelled figure on a card: used for the balance strip and the month totals.

use eframe::egui::{self, CornerRadius, Frame, Margin, RichText, Stroke};

use crate::domain::money::Money;
use crate::domain::settings::Theme;
use crate::ui::theme::{palette, typography};

/// How the figure is colored. Color is never the only cue: callers also show a sign or a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Positive,
    Negative,
}

impl Tone {
    /// Positive for gains, negative for losses, neutral for zero.
    #[must_use]
    pub fn for_amount(amount: Money) -> Self {
        match amount.cents().signum() {
            1 => Tone::Positive,
            -1 => Tone::Negative,
            _ => Tone::Neutral,
        }
    }
}

/// `R$ 10,00` or `−R$ 10,00`: like `Money::format_brl` but with a real minus sign, so negative
/// figures look the same everywhere on screen. No `+` is added to positive amounts.
#[must_use]
pub fn plain_brl(amount: Money) -> String {
    amount.format_brl().replacen("-R$", "−R$", 1)
}

/// `+R$ 10,00`, `−R$ 10,00` (a real minus sign) or `R$ 0,00`.
#[must_use]
pub fn signed_brl(amount: Money) -> String {
    match amount.cents().signum() {
        1 => format!("+{}", amount.format_brl()),
        -1 => amount.format_brl().replacen("-R$", "−R$", 1),
        _ => amount.format_brl(),
    }
}

/// Draw the card at the full width available. `caption` is a small muted line under the value,
/// such as the range the figure covers.
pub fn show(
    ui: &mut egui::Ui,
    theme: Theme,
    title: &str,
    value: &str,
    tone: Tone,
    caption: Option<&str>,
) {
    let p = palette(theme);
    let color = match tone {
        Tone::Neutral => p.text,
        Tone::Positive => p.positive,
        Tone::Negative => p.negative,
    };
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Cards in a row stay the same height whether or not they carry a caption.
            ui.set_min_height(64.0);
            ui.label(
                RichText::new(title)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
            ui.label(
                RichText::new(value)
                    .font(typography::tabular_font(typography::SECTION))
                    .strong()
                    .color(color),
            );
            if let Some(caption) = caption {
                ui.label(
                    RichText::new(caption)
                        .text_style(egui::TextStyle::Small)
                        .color(p.muted),
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_amounts_use_a_real_minus_and_an_explicit_plus() {
        assert_eq!(signed_brl(Money::from_cents(1_000)), "+R$ 10,00");
        assert_eq!(signed_brl(Money::from_cents(-123_456)), "−R$ 1.234,56");
        assert_eq!(signed_brl(Money::ZERO), "R$ 0,00");
    }

    #[test]
    fn plain_amounts_use_a_real_minus_and_no_plus() {
        assert_eq!(plain_brl(Money::from_cents(5_000)), "R$ 50,00");
        assert_eq!(plain_brl(Money::from_cents(-5_000)), "−R$ 50,00");
        assert_eq!(plain_brl(Money::ZERO), "R$ 0,00");
    }

    #[test]
    fn tone_follows_the_sign() {
        assert_eq!(Tone::for_amount(Money::from_cents(5)), Tone::Positive);
        assert_eq!(Tone::for_amount(Money::from_cents(-5)), Tone::Negative);
        assert_eq!(Tone::for_amount(Money::ZERO), Tone::Neutral);
    }
}
