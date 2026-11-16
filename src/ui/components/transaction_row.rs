//! One register row: type badge (icon and label), title, category, people and the signed amount.
//!
//! Right-aligned content inside a vertical layout must sit in `ui.horizontal`; a bare
//! `right_to_left` layout takes all remaining height.

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

struct RowLayout<'a> {
    p: &'a crate::ui::theme::Palette,
    s: &'a crate::i18n::Strings,
    icon: &'a str,
    label: &'a str,
    amount_color: egui::Color32,
    category: Option<&'a crate::domain::category::Category>,
    amount_str: String,
    people_str: String,
    badge_w: f32,
    actions_w: f32,
    actions_and_amount_w: f32,
    right_w: f32,
    people_w: f32,
    title_need: f32,
    cat_need: Option<f32>,
    spacing: f32,
}

fn measure_row<'a>(
    ui: &mut egui::Ui,
    vm: &'a ViewModel,
    tx: &'a Transaction,
    p: &'a crate::ui::theme::Palette,
    s: &'a crate::i18n::Strings,
) -> RowLayout<'a> {
    let language = vm.language();
    let (icon, label) = type_badge(tx.kind, language);
    let amount_color = match tx.kind {
        TransactionType::Expense => p.negative,
        TransactionType::Income | TransactionType::Refund => p.positive,
    };
    let category = tx.category_id.and_then(|id| vm.category(id));
    let amount_str = amount_text(tx.kind, tx.amount);
    let people_str = people_label(tx, vm);

    let small_font = ui.style().text_styles[&egui::TextStyle::Small].clone();
    let body_font = ui.style().text_styles[&egui::TextStyle::Body].clone();
    let amount_font = typography::money_font();
    let spacing = ui.spacing().item_spacing.x;
    let measure = |ui: &mut egui::Ui, text: &str, font: &egui::FontId| -> f32 {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(text.to_owned(), font.clone(), p.text)
                .size()
                .x
        })
    };

    let badge_w =
        (measure(ui, icon, &body_font) + measure(ui, label, &small_font) + spacing + 12.0)
            .max(96.0);
    let delete_w = measure(ui, s.delete, &small_font);
    let edit_w = measure(ui, s.edit, &small_font);
    let actions_w = delete_w + edit_w + spacing + 8.0;
    let amount_w = measure(ui, &amount_str, &amount_font) + 8.0;
    let actions_and_amount_w = delete_w + edit_w + amount_w + 2.0 * spacing + 12.0;
    let people_w = measure(ui, &people_str, &small_font) + 22.0;
    let right_w = actions_and_amount_w + people_w + spacing;
    let title_need = measure(ui, &tx.title, &body_font) + 8.0;
    let cat_need = category.map(|c| measure(ui, &c.name, &small_font) + 10.0 + spacing + 8.0);

    RowLayout {
        p,
        s,
        icon,
        label,
        amount_color,
        category,
        amount_str,
        people_str,
        badge_w,
        actions_w,
        actions_and_amount_w,
        right_w,
        people_w,
        title_need,
        cat_need,
        spacing,
    }
}

/// Draw one row with its Edit and Delete buttons. The buttons are always present, so keyboard
/// and screen-reader users reach them the same way as everyone else; each is named after the
/// entry it acts on.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, tx: &Transaction) -> Option<RowAction> {
    let p = palette(vm.theme());
    let s = strings(vm.language());
    let layout = measure_row(ui, vm, tx, &p, s);
    let mut action = None;

    egui::Frame::new()
        .inner_margin(Margin::symmetric(0, 6))
        .show(ui, |ui| {
            let left_budget =
                ui.available_width() - layout.badge_w - layout.right_w - 2.0 * layout.spacing;
            if left_budget >= 180.0 {
                draw_single_line_row(ui, vm.theme(), tx, &layout, left_budget, &mut action);
            } else {
                draw_narrow_row(ui, vm.theme(), tx, &layout, &mut action);
            }
        });
    action
}

fn draw_single_line_row(
    ui: &mut egui::Ui,
    theme: crate::domain::settings::Theme,
    tx: &Transaction,
    layout: &RowLayout<'_>,
    left_budget: f32,
    action: &mut Option<RowAction>,
) {
    let (title_max, cat_label_max) = allocate_left_widths(
        left_budget,
        layout.title_need,
        layout.cat_need,
        layout.spacing,
    );
    ui.horizontal(|ui| {
        ui.set_min_height(32.0);
        draw_badge(
            ui,
            layout.badge_w,
            layout.icon,
            layout.label,
            layout.amount_color,
            layout.p.muted,
        );
        draw_title_and_category(
            ui,
            &tx.title,
            layout.category,
            title_max,
            cat_label_max,
            layout.p.muted,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            draw_row_actions_and_amount(ui, tx, layout, action);
            person_pill::show_bounded(ui, theme, &layout.people_str, layout.people_w);
        });
    });
}

fn draw_narrow_row(
    ui: &mut egui::Ui,
    theme: crate::domain::settings::Theme,
    tx: &Transaction,
    layout: &RowLayout<'_>,
    action: &mut Option<RowAction>,
) {
    ui.vertical(|ui| {
        let row_w = ui.available_width();
        draw_narrow_header(ui, tx, layout, row_w, action);
        draw_narrow_details(ui, theme, tx, layout, row_w);
    });
}

fn draw_narrow_header(
    ui: &mut egui::Ui,
    tx: &Transaction,
    layout: &RowLayout<'_>,
    row_w: f32,
    action: &mut Option<RowAction>,
) {
    if row_w >= layout.badge_w + layout.actions_and_amount_w + layout.spacing {
        ui.horizontal(|ui| {
            ui.set_min_height(28.0);
            draw_badge(
                ui,
                layout.badge_w,
                layout.icon,
                layout.label,
                layout.amount_color,
                layout.p.muted,
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                draw_row_actions_and_amount(ui, tx, layout, action);
            });
        });
        return;
    }

    if row_w >= layout.badge_w + layout.actions_w + layout.spacing {
        ui.horizontal(|ui| {
            ui.set_min_height(26.0);
            draw_badge(
                ui,
                layout.badge_w,
                layout.icon,
                layout.label,
                layout.amount_color,
                layout.p.muted,
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                draw_row_actions(ui, layout.s, tx, layout.p, action);
            });
        });
    } else {
        draw_badge(
            ui,
            layout.badge_w,
            layout.icon,
            layout.label,
            layout.amount_color,
            layout.p.muted,
        );
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                draw_row_actions(ui, layout.s, tx, layout.p, action);
            });
        });
    }

    // Bound the cross-axis before centering, rather than consuming the scroll area's height.
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.scope(|ui| {
                ui.set_max_width(row_w);
                draw_row_amount(ui, &layout.amount_str, layout.amount_color);
            });
        });
    });
}

fn draw_narrow_details(
    ui: &mut egui::Ui,
    theme: crate::domain::settings::Theme,
    tx: &Transaction,
    layout: &RowLayout<'_>,
    row_w: f32,
) {
    let remaining_for_text = row_w - layout.people_w - layout.spacing;
    if remaining_for_text >= 160.0 {
        let (title_max, cat_label_max) = allocate_left_widths(
            remaining_for_text,
            layout.title_need,
            layout.cat_need,
            layout.spacing,
        );
        ui.horizontal(|ui| {
            ui.set_min_height(24.0);
            draw_title_and_category(
                ui,
                &tx.title,
                layout.category,
                title_max,
                cat_label_max,
                layout.p.muted,
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                person_pill::show_bounded(ui, theme, &layout.people_str, layout.people_w);
            });
        });
        return;
    }

    let (title_max, cat_label_max) =
        allocate_left_widths(row_w, layout.title_need, layout.cat_need, layout.spacing);
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        draw_title_and_category(
            ui,
            &tx.title,
            layout.category,
            title_max,
            cat_label_max,
            layout.p.muted,
        );
    });
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            person_pill::show_bounded(ui, theme, &layout.people_str, row_w);
        });
    });
}

fn allocate_left_widths(
    budget: f32,
    title_need: f32,
    cat_need: Option<f32>,
    spacing: f32,
) -> (f32, f32) {
    let Some(cat_need) = cat_need else {
        return (budget.max(40.0), 0.0);
    };
    if title_need + cat_need + spacing <= budget {
        return (title_need, (cat_need - 10.0 - spacing).max(24.0));
    }
    let title_min = title_need.min(budget * 0.55);
    let cat_alloc = cat_need
        .min((budget - title_min - spacing).max(44.0))
        .min(budget * 0.45);
    let title_max = (budget - cat_alloc - spacing).max(32.0);
    let cat_label_max = (cat_alloc - 10.0 - spacing).max(24.0);
    (title_max, cat_label_max)
}

fn draw_badge(
    ui: &mut egui::Ui,
    badge_w: f32,
    icon: &str,
    label: &str,
    amount_color: egui::Color32,
    muted: egui::Color32,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(badge_w, 24.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(RichText::new(icon).strong().color(amount_color));
            ui.label(
                RichText::new(label)
                    .text_style(egui::TextStyle::Small)
                    .color(muted),
            );
        },
    );
}

fn draw_title_and_category(
    ui: &mut egui::Ui,
    title: &str,
    category: Option<&crate::domain::category::Category>,
    title_max: f32,
    cat_label_max: f32,
    muted: egui::Color32,
) {
    // Long titles end in an ellipsis instead of pushing right controls away; the full title is in the tooltip.
    ui.scope(|ui| {
        ui.set_max_width(title_max);
        ui.add(egui::Label::new(RichText::new(title).strong()).truncate())
            .on_hover_text(title);
    });
    if let Some(category) = category {
        let dot = parse_hex(&category.color).unwrap_or(muted);
        // Drawn, not typed, so it never depends on a font having the glyph.
        let (dot_rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter().circle_filled(dot_rect.center(), 4.0, dot);
        ui.scope(|ui| {
            ui.set_max_width(cat_label_max);
            ui.add(
                egui::Label::new(
                    RichText::new(&category.name)
                        .text_style(egui::TextStyle::Small)
                        .color(muted),
                )
                .truncate(),
            )
            .on_hover_text(&category.name);
        });
    }
}

fn draw_row_actions_and_amount(
    ui: &mut egui::Ui,
    tx: &Transaction,
    layout: &RowLayout<'_>,
    action: &mut Option<RowAction>,
) {
    draw_row_actions(ui, layout.s, tx, layout.p, action);
    draw_row_amount(ui, &layout.amount_str, layout.amount_color);
}

fn draw_row_actions(
    ui: &mut egui::Ui,
    s: &crate::i18n::Strings,
    tx: &Transaction,
    p: &crate::ui::theme::Palette,
    action: &mut Option<RowAction>,
) {
    let ghost = |ui: &mut egui::Ui, text: &str, name: &str, danger: bool| {
        let color = if danger { p.negative } else { p.muted };
        let button = egui::Button::new(
            RichText::new(text)
                .text_style(egui::TextStyle::Small)
                .color(color),
        )
        .frame(false);
        let response = ui.add(button);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    };
    if ghost(ui, s.delete, &format!("{} {}", s.delete, tx.title), true) {
        *action = Some(RowAction::Delete(tx.id));
    }
    if ghost(ui, s.edit, &format!("{} {}", s.edit, tx.title), false) {
        *action = Some(RowAction::Edit(tx.id));
    }
}

fn draw_row_amount(ui: &mut egui::Ui, amount_str: &str, amount_color: egui::Color32) {
    ui.add(
        egui::Label::new(
            RichText::new(amount_str)
                .font(typography::money_font())
                .strong()
                .color(amount_color),
        )
        .truncate(),
    )
    .on_hover_text(amount_str);
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

    #[test]
    fn narrow_fallback_rows_stay_compact_in_a_scrolling_register() {
        use crate::domain::category::Category;
        use egui_kittest::kittest::Queryable;

        for language in [Language::En, Language::PtBr] {
            for (width, cents, long_names) in [
                (372.0, 123_456_789, true),
                (372.0, i64::MAX, false),
                (140.0, 123_456_789, true),
            ] {
                let mut vm = ViewModel::new(language);
                vm.all_people = vec![
                    person(
                        1,
                        if long_names {
                            "Alexandre Fernandes de Albuquerque"
                        } else {
                            "A3"
                        },
                        PersonKind::Owner,
                    ),
                    person(
                        2,
                        if long_names {
                            "Maria Aparecida de Albuquerque"
                        } else {
                            "Ana"
                        },
                        PersonKind::Partner,
                    ),
                ];
                vm.all_categories = vec![Category {
                    id: 1,
                    name: "Compras".into(),
                    ..Category::default()
                }];
                let mut tx = tx_on(2026, 11, 14);
                tx.title = "Compra parcelada com descrição longa".into();
                tx.category_id = Some(1);
                tx.amount = Money::from_cents(cents);
                tx.allocations = vec![
                    Allocation {
                        person_id: 1,
                        amount: Money::from_cents(cents / 2),
                    },
                    Allocation {
                        person_id: 2,
                        amount: Money::from_cents(cents - cents / 2),
                    },
                ];
                let mut harness = egui_kittest::Harness::builder()
                    .with_size(Vec2::new(width + 16.0, 260.0))
                    .with_context_setup(crate::ui::theme::typography::install_test_fonts)
                    .build_ui_state(
                        |ui, (row_rect, max_row_h): &mut (egui::Rect, f32)| {
                            ui.ctx()
                                .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                            crate::ui::theme::apply(ui.ctx(), vm.theme());
                            ui.set_style(ui.ctx().global_style());
                            let interact_y = ui.spacing().interact_size.y;
                            let spacing_y = ui.spacing().item_spacing.y;
                            let amount_line_h =
                                ui.fonts_mut(|f| f.row_height(&typography::money_font()));
                            // Bound: 3 interactive rows + 2 wrapped amount lines + vertical spacing/frame margins, plus 15% margin.
                            *max_row_h =
                                (3.0 * interact_y + 2.0 * amount_line_h + 6.0 * spacing_y) * 1.15;
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    *row_rect = ui.scope(|ui| show(ui, &vm, &tx)).response.rect;
                                    ui.label("Following transaction");
                                });
                        },
                        (egui::Rect::NOTHING, 0.0_f32),
                    );
                harness.run_steps(3);

                let (row, max_row_h) = *harness.state();
                assert!(
                    row.height() <= max_row_h,
                    "{language:?} {width}px {cents} cents: row height {} exceeded bound {max_row_h}: {row:?}",
                    row.height()
                );
                let following = harness.get_by_label("Following transaction").rect();
                assert!(following.top() >= row.bottom());
                assert!(
                    following.bottom() <= 252.0,
                    "{language:?} {width}px: following row is offscreen: {following:?}"
                );
            }
        }
    }

    #[test]
    fn narrow_row_handles_max_amounts_and_long_person_names_without_overlap() {
        use crate::domain::category::Category;
        use crate::domain::settings::{Profile, Theme};
        use egui_kittest::kittest::Queryable;

        let title = "Compra parcelada com descrição muito longa para testar quebra e truncamento";
        let cat_name = "Categoria com nome extremamente longo ãõçáéíóúâêôàü para testar layout";
        let owner_name = "Maria Fernanda Cavalcante";
        let partner_name = "João Pedro Albuquerque";
        let third_name = "Guilherme Henrique Vasconcelos";
        let expected_people = format!("{owner_name} · {partner_name} · {third_name}");

        for (lang, badge_text, edit_word, delete_word) in [
            (Language::PtBr, "Despesa", "Editar", "Excluir"),
            (Language::En, "Expense", "Edit", "Delete"),
        ] {
            for width in [280.0_f32, 372.0_f32, 492.0_f32, 784.0_f32] {
                let mut vm = ViewModel::new(lang);
                vm.profile = Some(Profile {
                    start_balance: Money::ZERO,
                    theme: Theme::Dark,
                    language: lang,
                    onboarding_complete: true,
                });
                vm.all_people = vec![
                    person(1, owner_name, PersonKind::Owner),
                    person(2, partner_name, PersonKind::Partner),
                    person(3, third_name, PersonKind::Partner),
                ];
                vm.all_categories = vec![Category {
                    id: 1,
                    name: cat_name.into(),
                    color: "#6C8CFF".into(),
                    ..Category::default()
                }];
                let mut tx = tx_on(2026, 11, 14);
                tx.title = title.into();
                tx.category_id = Some(1);
                tx.amount = Money::from_cents(i64::MAX);
                tx.allocations = vec![
                    Allocation {
                        person_id: 1,
                        amount: Money::from_cents(100),
                    },
                    Allocation {
                        person_id: 2,
                        amount: Money::from_cents(100),
                    },
                    Allocation {
                        person_id: 3,
                        amount: Money::from_cents(100),
                    },
                ];
                let expected_amount = amount_text(tx.kind, tx.amount);

                let mut harness = egui_kittest::Harness::builder()
                    .with_size(Vec2::new(width + 16.0, 260.0))
                    .with_context_setup(crate::ui::theme::typography::install_test_fonts)
                    .build_ui(|ui| {
                        ui.ctx()
                            .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                        crate::ui::theme::apply(ui.ctx(), vm.theme());
                        let style = ui.ctx().global_style();
                        ui.set_style(style);
                        show(ui, &vm, &tx);
                    });
                harness.run_steps(2);

                let items = [
                    ("badge", harness.get_by_label(badge_text).rect()),
                    ("title", harness.get_by_label(title).rect()),
                    ("category", harness.get_by_label(cat_name).rect()),
                    ("people", harness.get_by_label(&expected_people).rect()),
                    ("amount", harness.get_by_label(&expected_amount).rect()),
                    (
                        "edit",
                        harness.get_by_label(&format!("{edit_word} {title}")).rect(),
                    ),
                    (
                        "delete",
                        harness
                            .get_by_label(&format!("{delete_word} {title}"))
                            .rect(),
                    ),
                ];

                for (name, rect) in &items {
                    assert!(
                        rect.left() >= 7.0 && rect.right() <= width + 9.0,
                        "{lang:?} {width}px {name} out of 8..={}: {rect:?}",
                        width + 8.0
                    );
                }
                for (i, (a_name, a)) in items.iter().enumerate() {
                    for (b_name, b) in &items[i + 1..] {
                        assert!(
                            !a.intersects(*b),
                            "{lang:?} {width}px {a_name} ({a:?}) overlaps {b_name} ({b:?})"
                        );
                    }
                }
            }
        }
    }
}
