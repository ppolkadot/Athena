//! One register row: type badge (icon and label), title, category, people and the signed amount.

use eframe::egui::{self, Align, Layout, Margin, RichText, Sense, Vec2};

use crate::app::view_model::ViewModel;
use crate::domain::money::Money;
use crate::domain::settings::Language;
use crate::domain::transaction::{Transaction, TransactionType};
use crate::i18n::strings;
use crate::ui::components::person_pill;
use crate::ui::theme::{palette, parse_hex, typography};

/// Icon and label for a kind. Meaning never rests on color alone.
#[must_use]
pub fn type_badge(kind: TransactionType, language: Language) -> (&'static str, &'static str) {
    let s = strings(language);
    match kind {
        TransactionType::Expense => ("−", s.expense),
        TransactionType::Income => ("+", s.income),
        TransactionType::Refund => ("↩", s.refund),
    }
}

/// `−R$ 45,90` for expenses, `+R$ 45,90` for income and refunds.
#[must_use]
pub fn amount_text(kind: TransactionType, amount: Money) -> String {
    let sign = if kind == TransactionType::Expense {
        "−"
    } else {
        "+"
    };
    format!("{sign}{}", amount.format_brl())
}

/// Names of the people on an entry, or `No one` when it is unassigned.
#[must_use]
pub fn people_label(tx: &Transaction, vm: &ViewModel) -> String {
    let names: Vec<String> = tx
        .allocations
        .iter()
        .map(|a| {
            vm.person(a.person_id)
                .map_or_else(String::new, |p| p.name.clone())
        })
        .collect();
    if names.is_empty() {
        strings(vm.language()).no_one.to_string()
    } else {
        names.join(" · ")
    }
}

/// What the user asked for on a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    Edit(i64),
    Delete(i64),
}

/// Draw one row with its Edit and Delete buttons. The buttons are always present, so keyboard
/// and screen-reader users reach them the same way as everyone else; each is named after the
/// entry it acts on.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, tx: &Transaction) -> Option<RowAction> {
    let p = palette(vm.theme());
    let language = vm.language();
    let (icon, label) = type_badge(tx.kind, language);
    let amount_color = match tx.kind {
        TransactionType::Expense => p.negative,
        TransactionType::Income | TransactionType::Refund => p.positive,
    };
    let category = tx.category_id.and_then(|id| vm.category(id));

    let s = strings(language);
    let mut action = None;
    egui::Frame::new()
        .inner_margin(Margin::symmetric(0, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(32.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(96.0, 24.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.label(RichText::new(icon).strong().color(amount_color));
                        ui.label(
                            RichText::new(label)
                                .text_style(egui::TextStyle::Small)
                                .color(p.muted),
                        );
                    },
                );
                // Long titles end in an ellipsis instead of pushing the amount away; the full
                // title is in the tooltip.
                ui.scope(|ui| {
                    ui.set_max_width((ui.available_width() - 380.0).max(120.0));
                    ui.add(egui::Label::new(RichText::new(&tx.title).strong()).truncate())
                        .on_hover_text(&tx.title);
                });
                if let Some(category) = category {
                    let dot = parse_hex(&category.color).unwrap_or(p.muted);
                    // Drawn, not typed, so it never depends on a font having the glyph.
                    let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    ui.painter().circle_filled(dot_rect.center(), 4.0, dot);
                    ui.label(
                        RichText::new(&category.name)
                            .text_style(egui::TextStyle::Small)
                            .color(p.muted),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ghost = |ui: &mut egui::Ui, text: &str, name: &str, danger: bool| {
                        let color = if danger { p.negative } else { p.muted };
                        let button = egui::Button::new(
                            RichText::new(text)
                                .text_style(egui::TextStyle::Small)
                                .color(color),
                        )
                        .frame(false);
                        let response = ui.add(button);
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, name)
                        });
                        response
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                    };
                    if ghost(ui, s.delete, &format!("{} {}", s.delete, tx.title), true) {
                        action = Some(RowAction::Delete(tx.id));
                    }
                    if ghost(ui, s.edit, &format!("{} {}", s.edit, tx.title), false) {
                        action = Some(RowAction::Edit(tx.id));
                    }
                    ui.label(
                        RichText::new(amount_text(tx.kind, tx.amount))
                            .font(typography::money_font())
                            .strong()
                            .color(amount_color),
                    );
                    person_pill::show(ui, vm.theme(), &people_label(tx, vm));
                });
            });
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::person::PersonKind;
    use crate::domain::transaction::Allocation;
    use crate::shared::test_fixtures::{person, tx_on};

    #[test]
    fn type_badges_are_not_color_only() {
        assert_eq!(
            type_badge(TransactionType::Expense, Language::En),
            ("−", "Expense")
        );
        assert_eq!(
            type_badge(TransactionType::Income, Language::En),
            ("+", "Income")
        );
        assert_eq!(
            type_badge(TransactionType::Refund, Language::En),
            ("↩", "Refund")
        );
        assert_eq!(
            type_badge(TransactionType::Refund, Language::PtBr),
            ("↩", "Reembolso")
        );
    }

    #[test]
    fn amounts_carry_a_sign_for_every_kind() {
        let m = Money::from_cents(4_590);
        assert_eq!(amount_text(TransactionType::Expense, m), "−R$ 45,90");
        assert_eq!(amount_text(TransactionType::Income, m), "+R$ 45,90");
        assert_eq!(amount_text(TransactionType::Refund, m), "+R$ 45,90");
    }

    #[test]
    fn people_are_named_in_full() {
        let mut vm = ViewModel::new(Language::En);
        vm.all_people = vec![
            person(1, "A3", PersonKind::Owner),
            person(2, "Ana", PersonKind::Partner),
        ];
        let mut tx = tx_on(2026, 11, 11);
        assert_eq!(people_label(&tx, &vm), "No one");
        tx.allocations = vec![Allocation {
            person_id: 2,
            amount: Money::from_cents(100),
        }];
        assert_eq!(people_label(&tx, &vm), "Ana");
        tx.allocations.push(Allocation {
            person_id: 1,
            amount: Money::ZERO,
        });
        assert_eq!(people_label(&tx, &vm), "Ana · A3");
    }
}
