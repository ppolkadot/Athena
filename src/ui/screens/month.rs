//! The month register: month stepper, all-time balance, this month's totals, filters, the
//! day-grouped register and the category and person breakdowns. It only reads the `ViewModel`
//! and edits `UiState`; it never touches the database.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use eframe::egui::{
    self, Align, Button, ComboBox, CornerRadius, Frame, Margin, Rect, RichText, Role, Sense,
    Stroke, TextEdit, Vec2, WidgetInfo,
};

use crate::app::ui_state::{SEARCH_DEBOUNCE, UiState};
use crate::app::view_model::ViewModel;
use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::reports::bar_fraction;
use crate::domain::transaction::{SettlementIssue, Transaction};
use crate::i18n::{format_date, format_short_month_year, month_label, strings, weekday_short};
use crate::ui::components::settlement_card::{self, SettlementEvent};
use crate::ui::components::summary_card::{self, Tone, plain_brl, signed_brl};
use crate::ui::components::transaction_row::RowAction;
use crate::ui::components::{empty_state, transaction_row};
use crate::ui::theme::{Palette, palette, parse_hex, typography};

/// Below this width the right rail drops under the register.
const WIDE: f32 = 880.0;
const RAIL_WIDTH: f32 = 300.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthEvent {
    NewTransaction,
    Edit(i64),
    Delete(i64),
    Settlement(SettlementEvent),
}

/// Newest day first; entries keep their incoming order within a day.
#[must_use]
pub fn group_by_day(transactions: &[Transaction]) -> Vec<(NaiveDate, Vec<&Transaction>)> {
    let mut days: BTreeMap<Reverse<NaiveDate>, Vec<&Transaction>> = BTreeMap::new();
    for tx in transactions {
        days.entry(Reverse(tx.date)).or_default().push(tx);
    }
    days.into_iter().map(|(date, txs)| (date.0, txs)).collect()
}

/// A day's effect on the balance: income and refunds add, expenses subtract.
#[must_use]
pub fn day_net(transactions: &[&Transaction]) -> Money {
    transactions.iter().map(|t| t.signed_amount()).sum()
}

fn header(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState, event: &mut Option<MonthEvent>) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    ui.horizontal(|ui| {
        let step = |ui: &mut egui::Ui, glyph: &str, name: &str| {
            let response = ui.add_sized([36.0, 36.0], Button::new(glyph));
            response.widget_info(|| WidgetInfo::labeled(Role::Button, true, name));
            response.on_hover_text(name).clicked()
        };
        if step(ui, "‹", s.month_prev) {
            state.set_month(state.month.prev());
        }
        ui.vertical(|ui| {
            let label = ui.label(
                RichText::new(month_label(state.month, vm.language()))
                    .text_style(typography::display())
                    .strong(),
            );
            let underline = Rect::from_min_size(
                egui::pos2(label.rect.left(), label.rect.bottom() + 2.0),
                Vec2::new(label.rect.width(), 3.0),
            );
            ui.painter()
                .rect_filled(underline, CornerRadius::same(2), p.accent);
            ui.add_space(5.0);
        });
        if step(ui, "›", s.month_next) {
            state.set_month(state.month.next());
        }
        if state.month != YearMonth::from_date(vm.today)
            && ui
                .add(Button::new(RichText::new(s.current_month).color(p.accent)).frame(false))
                .clicked()
        {
            state.set_month(YearMonth::from_date(vm.today));
        }
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let primary =
                Button::new(RichText::new(s.new_transaction).color(p.bg).strong()).fill(p.accent);
            if ui.add_sized([180.0, 36.0], primary).clicked() {
                *event = Some(MonthEvent::NewTransaction);
            }
        });
    });
}

fn summaries(ui: &mut egui::Ui, vm: &ViewModel) {
    let s = strings(vm.language());
    let theme = vm.theme();
    summary_card::show(
        ui,
        theme,
        &format!("{} · {}", s.all_time, s.balance),
        &plain_brl(vm.balance),
        Tone::for_amount(vm.balance),
        None,
    );
    ui.add_space(8.0);
    ui.label(
        RichText::new(s.this_month)
            .text_style(egui::TextStyle::Small)
            .color(palette(theme).muted),
    );
    let m = &vm.summary;
    let refunds_note = (!m.refunds.is_zero())
        .then(|| s.after_refunds.replace("{amount}", &m.refunds.format_brl()));
    ui.columns(3, |cols| {
        summary_card::show(
            &mut cols[0],
            theme,
            s.income_total,
            &signed_brl(m.income),
            Tone::for_amount(m.income),
            None,
        );
        let spending = m.spending();
        let tone = if spending.cents() > 0 {
            Tone::Negative
        } else {
            Tone::Neutral
        };
        summary_card::show(
            &mut cols[1],
            theme,
            s.expenses,
            &plain_brl(spending),
            tone,
            refunds_note.as_deref(),
        );
        summary_card::show(
            &mut cols[2],
            theme,
            s.result,
            &signed_brl(m.net),
            Tone::for_amount(m.net),
            None,
        );
    });
}

fn filters(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    let now = ui.ctx().input(|i| i.time);
    ui.horizontal_wrapped(|ui| {
        let people = vm.active_people();
        let all = state.filter.person_id.is_none() && !state.filter.shared_only;
        if ui.selectable_label(all, s.all).clicked() {
            state.show_all_people();
        }
        for person in &people {
            if ui
                .selectable_label(state.filter.person_id == Some(person.id), &person.name)
                .clicked()
            {
                state.filter_person(person.id);
            }
        }
        if people.len() >= 2
            && ui
                .selectable_label(state.filter.shared_only, s.shared)
                .clicked()
        {
            state.filter_shared();
        }

        let caption = ui.label(
            RichText::new(s.qa_category)
                .text_style(egui::TextStyle::Small)
                .color(p.muted),
        );
        let selected = state
            .filter
            .category_id
            .and_then(|id| vm.category(id))
            .map_or(s.all_categories, |c| c.name.as_str());
        let mut category = state.filter.category_id;
        let combo = ComboBox::from_id_salt("month-category-filter")
            .width(ui.available_width().clamp(120.0, 200.0))
            .truncate()
            .selected_text(selected)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut category, None, s.all_categories);
                for c in vm.all_categories.iter().filter(|c| !c.archived) {
                    ui.selectable_value(&mut category, Some(c.id), &c.name);
                }
            });
        combo.response.labelled_by(caption.id);
        if category != state.filter.category_id {
            state.set_category_filter(category);
        }

        let mut text = state.search_input.clone();
        let search = ui.add(
            TextEdit::singleline(&mut text)
                .hint_text(s.search_hint)
                .desired_width(220.0),
        );
        if search.changed() {
            state.edit_search(text, now);
        }
        if let (Some(issue), Some(until)) = (state.filter.issue, state.filter.until) {
            let month_text = format_short_month_year(until, vm.language());
            let template = match issue {
                SettlementIssue::UnknownPayer => s.filter_missing_payer_until,
                SettlementIssue::Unassigned => s.filter_unassigned_until,
            };
            let chip = format!("{} · ✕", template.replace("{month}", &month_text));
            if ui
                .add(Button::new(RichText::new(chip).color(p.accent)))
                .clicked()
            {
                state.clear_repair_filter();
            }
        }
        if state.has_filters()
            && ui
                .add(Button::new(RichText::new(s.clear_filters).color(p.accent)).frame(false))
                .clicked()
        {
            state.clear_filters();
        }
    });
    if state.commit_search_if_due(now) {
        ui.ctx().request_repaint();
    } else if state.search_pending() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(SEARCH_DEBOUNCE));
    }
}

fn register(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    state: &mut UiState,
    event: &mut Option<MonthEvent>,
) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    if vm.transactions.is_empty() {
        if state.has_filters() {
            if empty_state::show(ui, vm.theme(), "⌕", s.no_matches, "", Some(s.clear_filters)) {
                state.clear_filters();
            }
        } else {
            let title = s
                .empty_month
                .replace("{month}", &month_label(state.month, vm.language()));
            if empty_state::show(
                ui,
                vm.theme(),
                "☰",
                &title,
                s.empty_month_hint,
                Some(s.new_transaction),
            ) {
                *event = Some(MonthEvent::NewTransaction);
            }
        }
        return;
    }
    for (date, txs) in group_by_day(&vm.transactions) {
        Frame::new()
            .fill(p.card)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::symmetric(16, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let net = day_net(&txs);
                ui.horizontal(|ui| {
                    let day = format!(
                        "{}  {}",
                        weekday_short(vm.language(), date.weekday()),
                        format_date(date, vm.language())
                    );
                    ui.label(
                        RichText::new(day)
                            .font(typography::money_font())
                            .color(p.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let color = match Tone::for_amount(net) {
                            Tone::Positive => p.positive,
                            Tone::Negative => p.negative,
                            Tone::Neutral => p.muted,
                        };
                        ui.label(
                            RichText::new(signed_brl(net))
                                .font(typography::money_font())
                                .color(color),
                        );
                        ui.label(
                            RichText::new(s.net_label)
                                .text_style(egui::TextStyle::Small)
                                .color(p.muted),
                        );
                    });
                });
                for (i, tx) in txs.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    match transaction_row::show(ui, vm, tx) {
                        Some(RowAction::Edit(id)) => *event = Some(MonthEvent::Edit(id)),
                        Some(RowAction::Delete(id)) => *event = Some(MonthEvent::Delete(id)),
                        None => {}
                    }
                }
            });
        ui.add_space(8.0);
    }
}

fn bar(ui: &mut egui::Ui, p: &Palette, fraction: f32, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(3), p.border);
    if fraction > 0.0 {
        let filled = Rect::from_min_size(
            rect.min,
            Vec2::new((rect.width() * fraction).max(6.0), rect.height()),
        );
        ui.painter()
            .rect_filled(filled, CornerRadius::same(3), color);
    }
}

fn rail_row(
    ui: &mut egui::Ui,
    p: &Palette,
    name: &str,
    amount: Money,
    fraction: f32,
    color: egui::Color32,
) {
    let amount_str = plain_brl(amount);
    let amount_font = typography::money_font();
    let amount_width = ui.fonts_mut(|f| {
        f.layout_no_wrap(amount_str.clone(), amount_font.clone(), p.text)
            .size()
            .x
    });
    let avail = ui.available_width();
    if avail < amount_width + 56.0 {
        ui.vertical(|ui| {
            ui.add(egui::Label::new(name).truncate())
                .on_hover_text(name);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(Align::Min), |ui| {
                    ui.scope(|ui| {
                        ui.set_max_width(avail);
                        ui.add(
                            egui::Label::new(RichText::new(amount_str).font(amount_font)).wrap(),
                        );
                    });
                });
            });
        });
    } else {
        ui.horizontal(|ui| {
            let max_name_width = avail - amount_width - 12.0;
            ui.scope(|ui| {
                ui.set_max_width(max_name_width);
                ui.add(egui::Label::new(name).truncate())
                    .on_hover_text(name);
            });
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(amount_str).font(amount_font));
            });
        });
    }
    bar(ui, p, fraction, color);
    ui.add_space(6.0);
}

/// Spending by category and by person for the month. These ignore the register filters on
/// purpose: they describe the whole month, and say so.
fn rail(ui: &mut egui::Ui, vm: &ViewModel, event: &mut Option<MonthEvent>) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    if vm
        .settlement
        .as_ref()
        .is_some_and(|settlement| settlement.balances.len() >= 2)
    {
        if let Some(ev) = settlement_card::show(ui, vm) {
            *event = Some(MonthEvent::Settlement(ev));
        }
        ui.add_space(12.0);
    }
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(s.by_category)
                    .text_style(typography::section())
                    .strong(),
            );
            ui.label(
                RichText::new(s.this_month)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            let spending: Vec<_> = vm
                .category_totals
                .iter()
                .filter(|t| t.net.cents() > 0)
                .collect();
            let max = spending.iter().map(|t| t.net).max().unwrap_or(Money::ZERO);
            for total in &spending {
                let category = total.category_id.and_then(|id| vm.category(id));
                let name = category.map_or(s.qa_no_category, |c| c.name.as_str());
                let color = category
                    .and_then(|c| parse_hex(&c.color))
                    .unwrap_or(p.muted);
                rail_row(ui, &p, name, total.net, bar_fraction(total.net, max), color);
            }
            if spending.is_empty() {
                ui.label(RichText::new("—").color(p.muted));
            }

            ui.add_space(12.0);
            ui.label(
                RichText::new(s.by_person)
                    .text_style(typography::section())
                    .strong(),
            );
            ui.label(
                RichText::new(s.this_month)
                    .text_style(egui::TextStyle::Small)
                    .color(p.muted),
            );
            ui.add_space(8.0);
            let people: Vec<_> = vm
                .person_totals
                .iter()
                .filter(|t| t.net.cents() > 0)
                .collect();
            let max = people.iter().map(|t| t.net).max().unwrap_or(Money::ZERO);
            for total in &people {
                let name = vm.person(total.person_id).map_or("", |p| p.name.as_str());
                rail_row(
                    ui,
                    &p,
                    name,
                    total.net,
                    bar_fraction(total.net, max),
                    p.accent,
                );
            }
            if people.is_empty() {
                ui.label(RichText::new("—").color(p.muted));
            }
        });
}

/// Draw the month screen. Returns what it needs the app to do, if anything.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) -> Option<MonthEvent> {
    let mut event = None;
    header(ui, vm, state, &mut event);
    ui.add_space(12.0);
    summaries(ui, vm);
    ui.add_space(12.0);
    filters(ui, vm, state);
    ui.add_space(8.0);
    let width = ui.available_width();
    if width >= WIDE {
        ui.horizontal_top(|ui| {
            // A horizontal parent would lay children out left to right; each column stacks downward.
            let column = |ui: &mut egui::Ui, w: f32, content: &mut dyn FnMut(&mut egui::Ui)| {
                ui.allocate_ui_with_layout(
                    Vec2::new(w, 0.0),
                    egui::Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(w);
                        content(ui);
                    },
                );
            };
            column(ui, width - RAIL_WIDTH - 16.0, &mut |ui| {
                register(ui, vm, state, &mut event);
            });
            column(ui, RAIL_WIDTH, &mut |ui| rail(ui, vm, &mut event));
        });
    } else {
        register(ui, vm, state, &mut event);
        rail(ui, vm, &mut event);
    }
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppCore;
    use crate::domain::settings::Language;
    use crate::infrastructure::db::Db;
    use crate::shared::test_fixtures::{seed_month, tx_on};

    #[test]
    fn groups_by_day_descending() {
        let txs = vec![
            tx_on(2026, 11, 10),
            tx_on(2026, 11, 11),
            tx_on(2026, 11, 11),
        ];
        let groups = group_by_day(&txs);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, NaiveDate::from_ymd_opt(2026, 11, 11).unwrap());
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[1].0, NaiveDate::from_ymd_opt(2026, 11, 10).unwrap());
        assert!(group_by_day(&[]).is_empty());
    }

    #[test]
    fn day_net_adds_income_and_refunds_and_subtracts_expenses() {
        use crate::domain::transaction::TransactionType;
        let expense = tx_on(2026, 11, 11);
        let mut income = tx_on(2026, 11, 11);
        income.kind = TransactionType::Income;
        income.amount = Money::from_cents(1_000);
        let mut refund = tx_on(2026, 11, 11);
        refund.kind = TransactionType::Refund;
        refund.amount = Money::from_cents(250);
        assert_eq!(
            day_net(&[&expense, &income, &refund]),
            Money::from_cents(1_150)
        );
        assert_eq!(day_net(&[]), Money::ZERO);
    }

    /// Several widgets can share a text (a card and a row both show `+R$ 300,00`).
    fn has(harness: &egui_kittest::Harness<'_, MonthApp>, label: &str) -> bool {
        use egui_kittest::kittest::Queryable;
        harness.query_all_by_label(label).count() > 0
    }

    fn core_with_month() -> AppCore {
        let db = Db::open_in_memory().unwrap();
        seed_month(&db);
        AppCore::empty_for_test(db)
    }

    type MonthApp = (ViewModel, UiState, Option<MonthEvent>);

    fn harness(core: &AppCore) -> egui_kittest::Harness<'static, MonthApp> {
        egui_kittest::Harness::builder()
            .with_size(Vec2::new(1100.0, 900.0))
            .build_ui_state(
                |ui, (vm, state, event): &mut MonthApp| {
                    // The harness only has egui's bundled fonts, which lack some of our symbols.
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    crate::ui::theme::apply(ui.ctx(), vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    if let Some(e) = show(ui, vm, state) {
                        *event = Some(e);
                    }
                },
                (core.vm.clone(), core.ui.clone(), None),
            )
    }

    #[test]
    fn the_register_shows_the_month_totals_and_every_entry() {
        use egui_kittest::kittest::Queryable;
        let core = core_with_month();
        let mut harness = harness(&core);
        harness.run_steps(3);
        assert!(harness.query_by_label("November 2026").is_some());
        assert!(harness.query_by_label("All time · Balance").is_some());
        assert!(has(&harness, "R$ 170,00"), "start balance 0 + 170,00 net");
        assert!(has(&harness, "+R$ 300,00"), "income card");
        assert!(has(&harness, "R$ 130,00"), "spending net of the refund");
        assert!(has(&harness, "+R$ 170,00"), "result card");
        for title in ["Rent", "Market", "Salary"] {
            assert!(harness.query_by_label(title).is_some(), "{title} is listed");
        }
        assert!(harness.query_by_label("Spending by category").is_some());
        assert!(has(&harness, "Housing"));
        assert!(harness.query_by_label("Spending by person").is_some());
        assert!(has(&harness, "This month"));
    }

    #[test]
    fn the_stepper_and_filters_edit_the_ui_state() {
        use egui_kittest::kittest::Queryable;
        let core = core_with_month();
        let mut harness = harness(&core);
        harness.run_steps(3);
        harness.get_by_label("Next month").click();
        harness.run_steps(3);
        assert_eq!(harness.state().1.month, YearMonth::new(2026, 12).unwrap());
        assert_eq!(harness.state().1.filter.month, YearMonth::new(2026, 12));
        assert!(harness.query_by_label("December 2026").is_some());
        harness.get_by_label("Previous month").click();
        harness.run_steps(3);
        assert_eq!(harness.state().1.month, YearMonth::new(2026, 11).unwrap());

        harness.get_by_label("Shared").click();
        harness.run_steps(3);
        assert!(harness.state().1.filter.shared_only);
        assert!(harness.query_by_label("Clear filters").is_some());
        harness.get_by_label("Clear filters").click();
        harness.run_steps(3);
        assert!(!harness.state().1.has_filters());
    }

    #[test]
    fn typed_search_reaches_the_filter_after_a_pause() {
        use egui_kittest::kittest::Queryable;
        let core = core_with_month();
        let mut harness = harness(&core);
        harness.run_steps(3);
        harness
            .get_all_by_role(Role::TextInput)
            .next()
            .unwrap()
            .focus();
        harness.run_steps(2);
        harness
            .get_all_by_role(Role::TextInput)
            .next()
            .unwrap()
            .type_text("pao");
        harness.run_steps(1);
        assert_eq!(harness.state().1.search_input, "pao");
        harness.run_steps(8);
        assert_eq!(
            harness.state().1.filter.text,
            "pao",
            "the filter follows once typing pauses"
        );
    }

    #[test]
    fn an_empty_month_invites_the_first_entry() {
        use egui_kittest::kittest::Queryable;
        let db = Db::open_in_memory().unwrap();
        let core = AppCore::empty_for_test(db);
        let mut harness = harness(&core);
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label("Nothing logged in November 2026 yet.")
                .is_some()
        );
        assert!(has(&harness, "R$ 0,00"));
        harness
            .get_all_by_label("New transaction")
            .last()
            .unwrap()
            .click();
        harness.run_steps(3);
        assert_eq!(harness.state().2, Some(MonthEvent::NewTransaction));
    }

    #[test]
    fn the_screen_follows_the_language() {
        use egui_kittest::kittest::Queryable;
        let mut core = core_with_month();
        core.vm.profile = Some(crate::domain::settings::Profile {
            start_balance: Money::ZERO,
            theme: crate::domain::settings::Theme::Light,
            language: Language::PtBr,
            onboarding_complete: true,
        });
        let mut harness = harness(&core);
        harness.run_steps(3);
        assert!(harness.query_by_label("Novembro 2026").is_some());
        assert!(harness.query_by_label("Todo o período · Saldo").is_some());
        assert!(harness.query_by_label("Gastos por categoria").is_some());
    }

    #[test]
    fn each_row_has_edit_and_delete_buttons_named_after_the_entry() {
        use egui_kittest::kittest::Queryable;
        let core = core_with_month();
        let rent = core
            .vm
            .transactions
            .iter()
            .find(|t| t.title == "Rent")
            .unwrap()
            .id;
        let market = core
            .vm
            .transactions
            .iter()
            .find(|t| t.title == "Market")
            .unwrap()
            .id;
        let mut harness = harness(&core);
        harness.run_steps(3);
        for title in ["Rent", "Market", "Salary", "Refund"] {
            assert!(has(&harness, &format!("Edit {title}")), "Edit {title}");
            assert!(has(&harness, &format!("Delete {title}")), "Delete {title}");
        }
        harness.get_by_label("Edit Rent").click();
        harness.run_steps(3);
        assert_eq!(harness.state().2, Some(MonthEvent::Edit(rent)));
        harness.state_mut().2 = None;
        harness.get_by_label("Delete Market").click();
        harness.run_steps(3);
        assert_eq!(harness.state().2, Some(MonthEvent::Delete(market)));
    }

    #[test]
    fn long_category_names_do_not_overlap_rail_or_row_amounts() {
        use egui_kittest::kittest::Queryable;
        let long_name = "Categoria com nome extremamente longo ãõçáéíóúâêôàü para testar layout";
        let mut core = core_with_month();
        core.vm
            .all_categories
            .iter_mut()
            .find(|c| c.name == "Housing")
            .unwrap()
            .name = long_name.into();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(Vec2::new(1100.0, 900.0))
            .with_context_setup(crate::ui::theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, (vm, state, event): &mut MonthApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    crate::ui::theme::apply(ui.ctx(), vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    if let Some(e) = show(ui, vm, state) {
                        *event = Some(e);
                    }
                },
                (core.vm.clone(), core.ui.clone(), None),
            );
        harness.run_steps(3);

        // Both the transaction row and the right rail render the category name with truncation,
        // and each keeps the label strictly to the left of the amount on that row.
        let mut categories: Vec<_> = harness
            .query_all_by_label(long_name)
            .map(|node| node.rect())
            .collect();
        assert_eq!(
            categories.len(),
            2,
            "expected truncated category labels in both register row and right rail, found: {categories:?}"
        );
        // Transaction row is in the left column, rail row is in the right column.
        categories.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
        let row_cat = categories[0];
        let rail_cat = categories[1];

        let row_amount = harness
            .query_all_by_label("−R$ 100,00")
            .map(|node| node.rect())
            .find(|r| (r.center().y - row_cat.center().y).abs() < 20.0)
            .expect("row amount on the same row as the long category");
        let row_person = harness
            .query_all_by_label("A3")
            .map(|node| node.rect())
            .find(|r| (r.center().y - row_cat.center().y).abs() < 20.0)
            .expect("person pill on the same row as the long category");
        let rail_amount = harness
            .query_all_by_label("R$ 100,00")
            .map(|node| node.rect())
            .find(|r| (r.center().y - rail_cat.center().y).abs() < 20.0)
            .expect("rail amount on the same row as the long category");

        assert!(
            row_cat.max.x < row_person.min.x,
            "row category ({row_cat:?}) overlaps person pill ({row_person:?})"
        );
        assert!(
            row_cat.max.x < row_amount.min.x,
            "row category ({row_cat:?}) overlaps row amount ({row_amount:?})"
        );
        assert!(
            rail_cat.max.x < rail_amount.min.x,
            "rail category ({rail_cat:?}) overlaps rail amount ({rail_amount:?})"
        );
    }

    const LONG_CAT: &str = "Categoria com nome extremamente longo ãõçáéíóúâêôàü para testar layout";
    const LONG_TITLE: &str =
        "Compra parcelada com descrição muito longa para testar quebra e truncamento";

    fn narrow_layout_core(lang: Language) -> AppCore {
        use crate::domain::settings::{Profile, Theme};
        use crate::domain::transaction::Allocation;

        let mut core = core_with_month();
        core.vm.profile = Some(Profile {
            start_balance: Money::ZERO,
            theme: Theme::Dark,
            language: lang,
            onboarding_complete: true,
        });
        core.vm
            .all_categories
            .iter_mut()
            .find(|c| c.name == "Housing")
            .unwrap()
            .name = LONG_CAT.into();
        let rent = core
            .vm
            .transactions
            .iter_mut()
            .find(|t| t.title == "Rent")
            .unwrap();
        rent.title = LONG_TITLE.into();
        rent.amount = Money::from_cents(123_456_789);
        rent.allocations = vec![
            Allocation {
                person_id: 1,
                amount: Money::from_cents(61_728_395),
            },
            Allocation {
                person_id: 2,
                amount: Money::from_cents(61_728_394),
            },
        ];
        let cat_id = rent.category_id;
        core.ui.set_category_filter(cat_id);
        if let Some(t) = core
            .vm
            .category_totals
            .iter_mut()
            .find(|t| t.category_id == cat_id)
        {
            t.net = Money::from_cents(123_456_789);
        }
        core
    }

    fn assert_narrow_layout_for(
        lang: Language,
        edit_word: &str,
        delete_word: &str,
        width: f32,
        height: f32,
    ) {
        use egui_kittest::kittest::Queryable;

        let core = narrow_layout_core(lang);
        let content_width = (width - 268.0).min(1100.0);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(Vec2::new(content_width, height))
            .with_context_setup(crate::ui::theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, (vm, state, event): &mut MonthApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    crate::ui::theme::apply(ui.ctx(), vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(ui.available_width());
                            if let Some(e) = show(ui, vm, state) {
                                *event = Some(e);
                            }
                        });
                },
                (core.vm.clone(), core.ui.clone(), None),
            );
        harness.run_steps(3);

        let row_amount = harness
            .query_all_by_label("−R$ 1.234.567,89")
            .map(|n| n.rect())
            .next()
            .unwrap_or_else(|| panic!("{lang:?} {width}x{height}: missing row amount"));
        let row_person = harness
            .query_all_by_label("A3 · Ana")
            .map(|n| n.rect())
            .next()
            .unwrap_or_else(|| panic!("{lang:?} {width}x{height}: missing A3 · Ana pill"));
        let edit_rect = harness
            .get_by_label(&format!("{edit_word} {LONG_TITLE}"))
            .rect();
        let delete_rect = harness
            .get_by_label(&format!("{delete_word} {LONG_TITLE}"))
            .rect();

        for (name, rect) in [
            ("row_amount", row_amount),
            ("row_person", row_person),
            ("edit", edit_rect),
            ("delete", delete_rect),
        ] {
            assert!(
                rect.left() >= -1.0 && rect.right() <= content_width + 1.0,
                "{lang:?} {width}x{height} {name} out of 0..={content_width}: {rect:?}"
            );
        }

        let title_rect = harness
            .query_all_by_label(LONG_TITLE)
            .map(|n| n.rect())
            .next()
            .unwrap_or_else(|| panic!("{lang:?} {width}x{height}: missing long title"));
        for (name, right_rect) in [
            ("row_amount", row_amount),
            ("row_person", row_person),
            ("edit", edit_rect),
            ("delete", delete_rect),
        ] {
            assert!(
                !title_rect.intersects(right_rect),
                "{lang:?} {width}x{height} title ({title_rect:?}) intersects {name} ({right_rect:?})"
            );
        }

        let mut cats: Vec<_> = harness
            .query_all_by_label(LONG_CAT)
            .map(|n| n.rect())
            .collect();
        assert!(
            cats.len() >= 2,
            "{lang:?} {width}x{height}: expected category labels, got {}",
            cats.len()
        );
        cats.sort_by(|a, b| a.min.y.total_cmp(&b.min.y));
        assert!(
            cats[0].right() <= content_width + 1.0,
            "{lang:?} {width}x{height}: top category overflowed {content_width}: {:?}",
            cats[0]
        );
        for cat_rect in &cats[1..] {
            assert!(
                cat_rect.right() <= content_width + 1.0,
                "{lang:?} {width}x{height}: category rect overflowed {content_width}: {cat_rect:?}"
            );
            assert!(
                !cat_rect.intersects(row_amount) && !cat_rect.intersects(row_person),
                "{lang:?} {width}x{height}: category ({cat_rect:?}) intersects row controls"
            );
        }
    }

    #[test]
    fn narrow_viewports_and_long_labels_never_overlap_or_clip_amounts() {
        for (lang, edit_word, delete_word) in [
            (Language::En, "Edit", "Delete"),
            (Language::PtBr, "Editar", "Excluir"),
        ] {
            for (width, height) in [
                (760.0_f32, 560.0_f32),
                (640.0_f32, 410.0_f32),
                (1100.0_f32, 760.0_f32),
            ] {
                assert_narrow_layout_for(lang, edit_word, delete_word, width, height);
            }
        }
    }

    #[test]
    fn rail_rows_stay_compact_in_a_scrolling_rail() {
        use crate::domain::settings::{Language, Theme};
        use egui_kittest::kittest::Queryable;

        for language in [Language::En, Language::PtBr] {
            let _ = language;
            for width in [120.0_f32, 200.0_f32] {
                let mut harness = egui_kittest::Harness::builder()
                    .with_size(Vec2::new(width + 16.0, 260.0))
                    .with_context_setup(crate::ui::theme::typography::install_test_fonts)
                    .build_ui_state(
                        |ui, (row_rect, max_h): &mut (egui::Rect, f32)| {
                            ui.ctx()
                                .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                            crate::ui::theme::apply(ui.ctx(), Theme::Dark);
                            ui.set_style(ui.ctx().global_style());
                            let p = palette(Theme::Dark);
                            let body_font = ui.style().text_styles[&egui::TextStyle::Body].clone();
                            let money_font = typography::money_font();
                            let line_h = ui.fonts_mut(|f| {
                                f.row_height(&body_font).max(f.row_height(&money_font))
                            });
                            let spacing_y = ui.spacing().item_spacing.y;
                            let interact_y = ui.spacing().interact_size.y;
                            *max_h = 2.0 * interact_y + 2.0 * line_h + 6.0 + 3.0 * spacing_y;
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_max_width(width);
                                    *row_rect = ui
                                        .scope(|ui| {
                                            rail_row(
                                                ui,
                                                &p,
                                                LONG_CAT,
                                                Money::from_cents(i64::MAX),
                                                0.5,
                                                p.accent,
                                            );
                                        })
                                        .response
                                        .rect;
                                    ui.label("Following row");
                                });
                        },
                        (egui::Rect::NOTHING, 0.0_f32),
                    );
                harness.run_steps(3);

                let (row_rect, max_h) = *harness.state();
                assert!(
                    row_rect.height() <= max_h,
                    "{language:?} {width}px: rail_row height {} exceeded bound {max_h}: {row_rect:?}",
                    row_rect.height()
                );
                let following = harness.get_by_label("Following row").rect();
                assert!(following.top() >= row_rect.bottom());
                assert!(
                    following.bottom() <= 252.0,
                    "{language:?} {width}px: following row is offscreen: {following:?}"
                );
                assert!(
                    row_rect.width() <= width + 1.0,
                    "{language:?} {width}px: rail_row width {} exceeded {width}: {row_rect:?}",
                    row_rect.width()
                );
                let amount_label = plain_brl(Money::from_cents(i64::MAX));
                let amount_rect = harness.get_by_label(&amount_label).rect();
                assert!(
                    amount_rect.left() >= row_rect.left() - 1.0
                        && amount_rect.right() <= row_rect.right() + 1.0,
                    "{language:?} {width}px: rail amount ({amount_rect:?}) overflowed row bounds ({row_rect:?})"
                );
            }
        }
    }
}
