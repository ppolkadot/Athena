//! Cumulative household settlement card for the month screen's right rail.

use chrono::NaiveDate;
use eframe::egui::{self, Button, CornerRadius, Frame, Label, Margin, RichText, Stroke};

use crate::app::view_model::ViewModel;
use crate::domain::money::Money;
use crate::domain::person::Person;
use crate::domain::settings::Language;
use crate::domain::settlement::SuggestedTransfer;
use crate::domain::transaction::SettlementIssue;
use crate::i18n::{PluralKey, format_as_of_date, plural, strings};
use crate::ui::theme::{palette, typography};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementEvent {
    Record {
        from: i64,
        to: i64,
        amount: Money,
    },
    RestorePerson(i64),
    FilterIssue {
        issue: SettlementIssue,
        as_of: NaiveDate,
    },
}

/// Format a participant's display name, appending the localized `(archived)` suffix when archived.
#[must_use]
pub fn person_display_name(person: &Person, language: Language) -> String {
    if person.archived {
        format!(
            "{}{}",
            person.name,
            strings(language).settlement_archived_suffix
        )
    } else {
        person.name.clone()
    }
}

/// Format one transfer from the owner's perspective when the owner is a party.
#[must_use]
pub fn transfer_sentence(
    transfer: SuggestedTransfer,
    owner_id: Option<i64>,
    from_name: &str,
    to_name: &str,
    language: Language,
) -> String {
    let s = strings(language);
    let amount = transfer.amount.format_brl();
    if Some(transfer.to) == owner_id {
        s.settlement_owes_you
            .replace("{from}", from_name)
            .replace("{amount}", &amount)
    } else if Some(transfer.from) == owner_id {
        s.settlement_you_owe
            .replace("{to}", to_name)
            .replace("{amount}", &amount)
    } else {
        s.settlement_owes_other
            .replace("{from}", from_name)
            .replace("{to}", to_name)
            .replace("{amount}", &amount)
    }
}

/// Localized all-settled summary line for an empty transfer list.
#[must_use]
pub fn settled_sentence(as_of: NaiveDate, language: Language) -> String {
    let date = format_as_of_date(as_of, language);
    strings(language)
        .settlement_all_settled
        .replace("{date}", &date)
}

fn transfer_row(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    transfer: SuggestedTransfer,
    owner_id: Option<i64>,
    max_w: f32,
) -> Option<SettlementEvent> {
    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    let from_person = vm.person(transfer.from);
    let to_person = vm.person(transfer.to);
    let from_name = from_person
        .map(|person| person_display_name(person, language))
        .unwrap_or_default();
    let to_name = to_person
        .map(|person| person_display_name(person, language))
        .unwrap_or_default();
    let sentence = transfer_sentence(transfer, owner_id, &from_name, &to_name, language);
    let mut event = None;

    ui.scope(|ui| {
        ui.set_max_width(max_w);
        ui.add(Label::new(RichText::new(sentence)).wrap());
        ui.add_space(4.0);
        let archived = [from_person, to_person]
            .into_iter()
            .flatten()
            .find(|person| person.archived);
        if let Some(person) = archived {
            let label = s
                .settlement_restore_to_record
                .replace("{name}", &person.name);
            if ui
                .add(Button::new(RichText::new(label).color(p.accent)).wrap())
                .clicked()
            {
                event = Some(SettlementEvent::RestorePerson(person.id));
            }
        } else if ui
            .add(Button::new(RichText::new(s.settlement_record)).wrap())
            .clicked()
        {
            event = Some(SettlementEvent::Record {
                from: transfer.from,
                to: transfer.to,
                amount: transfer.amount,
            });
        }
    });
    event
}

fn issue_link(
    ui: &mut egui::Ui,
    count: usize,
    key: PluralKey,
    issue: SettlementIssue,
    as_of: NaiveDate,
    vm: &ViewModel,
) -> Option<SettlementEvent> {
    let language = vm.language();
    let p = palette(vm.theme());
    let label = plural(key, count, language);
    ui.add(
        Button::new(
            RichText::new(label)
                .text_style(egui::TextStyle::Small)
                .color(p.accent),
        )
        .frame(false)
        .wrap(),
    )
    .clicked()
    .then_some(SettlementEvent::FilterIssue { issue, as_of })
}

fn exclusions_line(
    ui: &mut egui::Ui,
    vm: &ViewModel,
    unknown: usize,
    unassigned: usize,
    as_of: NaiveDate,
    max_w: f32,
) -> Option<SettlementEvent> {
    if unknown == 0 && unassigned == 0 {
        return None;
    }
    let p = palette(vm.theme());
    let mut event = None;
    ui.add_space(8.0);
    ui.scope(|ui| {
        ui.set_max_width(max_w);
        ui.horizontal_wrapped(|ui| {
            if unknown > 0
                && let Some(ev) = issue_link(
                    ui,
                    unknown,
                    PluralKey::SettlementUnknownPayer,
                    SettlementIssue::UnknownPayer,
                    as_of,
                    vm,
                )
            {
                event = Some(ev);
            }
            if unknown > 0 && unassigned > 0 {
                ui.label(
                    RichText::new("·")
                        .text_style(egui::TextStyle::Small)
                        .color(p.muted),
                );
            }
            if unassigned > 0
                && let Some(ev) = issue_link(
                    ui,
                    unassigned,
                    PluralKey::SettlementUnassigned,
                    SettlementIssue::Unassigned,
                    as_of,
                    vm,
                )
            {
                event = Some(ev);
            }
        });
    });
    event
}

/// Whether the Month rail should display the settlement card.
#[must_use]
pub fn should_show(vm: &ViewModel) -> bool {
    vm.settlement_unavailable
        || vm
            .settlement
            .as_ref()
            .is_some_and(|s| s.balances.len() >= 2 || s.excluded.any())
}

fn show_unavailable(ui: &mut egui::Ui, vm: &ViewModel, max_w: f32) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    ui.scope(|ui| {
        ui.set_max_width(max_w);
        ui.add(
            Label::new(
                RichText::new(s.settlement_title)
                    .text_style(typography::section())
                    .strong(),
            )
            .wrap(),
        );
        ui.add_space(6.0);
        ui.add(Label::new(RichText::new(s.settlement_unavailable).color(p.warning)).wrap());
    });
}

/// Render the settlement card. Returns `None` without drawing anything when unavailable is false,
/// fewer than two participants have non-zero settlement activity, and no exclusions exist.
pub fn show(ui: &mut egui::Ui, vm: &ViewModel) -> Option<SettlementEvent> {
    if !should_show(vm) {
        return None;
    }

    let language = vm.language();
    let s = strings(language);
    let p = palette(vm.theme());
    let owner_id = vm.owner().map(|o| o.id);
    let mut event = None;

    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            let max_w = ui.available_width();
            ui.set_width(max_w);
            ui.set_max_width(max_w);

            if vm.settlement_unavailable {
                show_unavailable(ui, vm, max_w);
                return;
            }

            let Some(settlement) = vm.settlement.as_ref() else {
                return;
            };

            ui.scope(|ui| {
                ui.set_max_width(max_w);
                ui.add(
                    Label::new(
                        RichText::new(s.settlement_title)
                            .text_style(typography::section())
                            .strong(),
                    )
                    .wrap(),
                );
                let as_of = s
                    .settlement_as_of
                    .replace("{date}", &format_as_of_date(settlement.as_of, language));
                ui.add(
                    Label::new(
                        RichText::new(as_of)
                            .text_style(egui::TextStyle::Small)
                            .color(p.muted),
                    )
                    .wrap(),
                );
            });
            ui.add_space(8.0);

            if settlement.balances.len() < 2 {
                ui.scope(|ui| {
                    ui.set_max_width(max_w);
                    ui.add(
                        Label::new(RichText::new(s.settlement_no_balances).color(p.muted)).wrap(),
                    );
                });
            } else if settlement.transfers.is_empty() {
                ui.scope(|ui| {
                    ui.set_max_width(max_w);
                    ui.add(
                        Label::new(
                            RichText::new(settled_sentence(settlement.as_of, language))
                                .color(p.muted),
                        )
                        .wrap(),
                    );
                });
            } else {
                for (idx, transfer) in settlement.transfers.iter().enumerate() {
                    if idx > 0 {
                        ui.add_space(8.0);
                    }
                    if let Some(ev) = transfer_row(ui, vm, *transfer, owner_id, max_w) {
                        event = Some(ev);
                    }
                }
            }

            if let Some(ev) = exclusions_line(
                ui,
                vm,
                settlement.excluded.unknown_payer,
                settlement.excluded.unassigned,
                settlement.as_of,
                max_w,
            ) {
                event = Some(ev);
            }
        });

    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::person::PersonKind;
    use crate::domain::settings::{Profile, Theme};
    use crate::domain::settlement::{ExclusionCounts, PersonBalance, SettlementView};

    #[test]
    fn sentences_cover_owner_creditor_debtor_third_party_and_settled() {
        let as_of = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        let owner_creditor = SuggestedTransfer {
            from: 2,
            to: 1,
            amount: Money::from_cents(8_000),
        };
        let owner_debtor = SuggestedTransfer {
            from: 1,
            to: 2,
            amount: Money::from_cents(8_000),
        };
        let third_party = SuggestedTransfer {
            from: 2,
            to: 3,
            amount: Money::from_cents(8_000),
        };

        assert_eq!(
            transfer_sentence(owner_creditor, Some(1), "Ana", "A3", Language::En),
            "Ana owes you R$ 80,00"
        );
        assert_eq!(
            transfer_sentence(owner_debtor, Some(1), "A3", "Ana", Language::En),
            "You owe Ana R$ 80,00"
        );
        assert_eq!(
            transfer_sentence(third_party, Some(1), "Ana", "Bia", Language::En),
            "Ana owes Bia R$ 80,00"
        );
        assert_eq!(
            settled_sentence(as_of, Language::En),
            "All settled up as of 30 Nov"
        );

        assert_eq!(
            transfer_sentence(owner_creditor, Some(1), "Ana", "A3", Language::PtBr),
            "Ana te deve R$ 80,00"
        );
        assert_eq!(
            transfer_sentence(owner_debtor, Some(1), "A3", "Ana", Language::PtBr),
            "Você deve R$ 80,00 a Ana"
        );
        assert_eq!(
            transfer_sentence(third_party, Some(1), "Ana", "Bia", Language::PtBr),
            "Ana deve R$ 80,00 a Bia"
        );
        assert_eq!(
            settled_sentence(as_of, Language::PtBr),
            "Tudo acertado até 30/11"
        );
    }

    fn max_cents_vm(language: Language) -> ViewModel {
        let mut vm = ViewModel::new(language);
        vm.profile = Some(Profile {
            start_balance: Money::ZERO,
            theme: Theme::Dark,
            language,
            onboarding_complete: true,
        });
        vm.all_people = vec![
            Person {
                id: 1,
                name: "Constantine Alexandrovich".into(),
                kind: PersonKind::Owner,
                archived: false,
                created_at: String::new(),
            },
            Person {
                id: 2,
                name: "Evangelina Maximiliano-Silva".into(),
                kind: PersonKind::Partner,
                archived: false,
                created_at: String::new(),
            },
            Person {
                id: 3,
                name: "Bartholomew Chrysostomides".into(),
                kind: PersonKind::Other,
                archived: true,
                created_at: String::new(),
            },
        ];
        vm.settlement = Some(SettlementView {
            as_of: NaiveDate::from_ymd_opt(2026, 11, 30).unwrap(),
            balances: vec![
                PersonBalance {
                    person_id: 1,
                    paid: Money::from_cents(i64::MAX),
                    owed: Money::ZERO,
                    balance: Money::from_cents(i64::MAX),
                },
                PersonBalance {
                    person_id: 2,
                    paid: Money::ZERO,
                    owed: Money::from_cents(i64::MAX / 2),
                    balance: Money::from_cents(-(i64::MAX / 2)),
                },
                PersonBalance {
                    person_id: 3,
                    paid: Money::ZERO,
                    owed: Money::from_cents(i64::MAX - i64::MAX / 2),
                    balance: Money::from_cents(-(i64::MAX - i64::MAX / 2)),
                },
            ],
            transfers: vec![
                SuggestedTransfer {
                    from: 3,
                    to: 1,
                    amount: Money::from_cents(i64::MAX),
                },
                SuggestedTransfer {
                    from: 2,
                    to: 1,
                    amount: Money::from_cents(i64::MAX / 2),
                },
            ],
            excluded: ExclusionCounts {
                unassigned: 3,
                unknown_payer: 2,
                invalid: 0,
            },
        });
        vm
    }

    #[test]
    fn settlement_card_stays_within_rail_bounds_at_max_cents() {
        use egui_kittest::kittest::Queryable;

        for language in [Language::En, Language::PtBr] {
            for rail_width in [300.0_f32, 220.0_f32] {
                let mut harness = egui_kittest::Harness::builder()
                    .with_size(egui::Vec2::new(rail_width, 500.0))
                    .with_context_setup(crate::ui::theme::typography::install_test_fonts)
                    .build_ui_state(
                        |ui, vm: &mut ViewModel| {
                            ui.ctx()
                                .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                            crate::ui::theme::apply(ui.ctx(), Theme::Dark);
                            ui.set_style(ui.ctx().global_style());
                            ui.allocate_ui_with_layout(
                                egui::Vec2::new(rail_width, 0.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    show(ui, vm);
                                },
                            );
                        },
                        max_cents_vm(language),
                    );
                harness.run_steps(2);

                for node in harness.get_all_by_role(egui::Role::Label) {
                    let rect = node.rect();
                    assert!(
                        rect.min.x >= -0.5 && rect.max.x <= rail_width + 0.5,
                        "label rect {rect:?} escaped 0..={rail_width} for {language:?}"
                    );
                }
                for node in harness.get_all_by_role(egui::Role::Button) {
                    let rect = node.rect();
                    assert!(
                        rect.min.x >= -0.5 && rect.max.x <= rail_width + 0.5,
                        "button rect {rect:?} escaped 0..={rail_width} for {language:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn settlement_unavailable_renders_card_with_notice() {
        use egui_kittest::kittest::Queryable;

        for (language, expected_title, expected_msg) in [
            (
                Language::En,
                "Settlement",
                "Settlement totals could not be calculated for this data.",
            ),
            (
                Language::PtBr,
                "Acertos",
                "Não foi possível calcular os acertos para estes dados.",
            ),
        ] {
            let mut vm = ViewModel::new(language);
            vm.settlement_unavailable = true;

            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::Vec2::new(320.0, 300.0))
                .with_context_setup(crate::ui::theme::typography::install_test_fonts)
                .build_ui_state(
                    |ui, vm: &mut ViewModel| {
                        ui.ctx()
                            .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                        crate::ui::theme::apply(ui.ctx(), Theme::Dark);
                        ui.set_style(ui.ctx().global_style());
                        show(ui, vm);
                    },
                    vm,
                );
            harness.run_steps(2);
            assert!(harness.query_by_label(expected_title).is_some());
            assert!(harness.query_by_label(expected_msg).is_some());
        }
    }

    #[test]
    fn rail_shows_settlement_card_for_single_participant_when_exclusions_exist() {
        use crate::domain::transaction::SettlementIssue;
        use egui_kittest::kittest::Queryable;

        let as_of = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        let mut vm = ViewModel::new(Language::En);
        vm.all_people = vec![crate::shared::test_fixtures::person(
            1,
            "A3",
            PersonKind::Owner,
        )];
        vm.profile = Some(Profile {
            start_balance: Money::ZERO,
            theme: Theme::Dark,
            language: Language::En,
            onboarding_complete: true,
        });
        vm.settlement = Some(SettlementView {
            as_of,
            balances: vec![PersonBalance {
                person_id: 1,
                paid: Money::from_cents(1_000),
                owed: Money::from_cents(1_000),
                balance: Money::ZERO,
            }],
            transfers: vec![],
            excluded: ExclusionCounts {
                unassigned: 1,
                unknown_payer: 0,
                invalid: 0,
            },
        });

        assert!(should_show(&vm));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(320.0, 300.0))
            .with_context_setup(crate::ui::theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, (vm, emitted): &mut (ViewModel, Option<SettlementEvent>)| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    crate::ui::theme::apply(ui.ctx(), Theme::Dark);
                    ui.set_style(ui.ctx().global_style());
                    if let Some(ev) = show(ui, vm) {
                        *emitted = Some(ev);
                    }
                },
                (vm, None),
            );
        harness.run_steps(2);
        assert!(harness.query_by_label("Settlement").is_some());
        assert!(harness.query_by_label("As of 30 Nov").is_some());
        harness
            .get_by_label("1 entry with no one responsible")
            .click();
        harness.run_steps(2);
        assert_eq!(
            harness.state().1,
            Some(SettlementEvent::FilterIssue {
                issue: SettlementIssue::Unassigned,
                as_of,
            })
        );
    }

    #[test]
    fn rail_hides_settlement_card_when_fewer_than_two_participants_and_no_exclusions_or_error() {
        let as_of = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        let mut vm = ViewModel::new(Language::En);
        vm.settlement = Some(SettlementView {
            as_of,
            balances: vec![PersonBalance {
                person_id: 1,
                paid: Money::from_cents(1_000),
                owed: Money::from_cents(1_000),
                balance: Money::ZERO,
            }],
            transfers: vec![],
            excluded: ExclusionCounts::default(),
        });
        assert!(!should_show(&vm));

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(320.0, 200.0))
            .with_context_setup(crate::ui::theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, (vm, drew): &mut (ViewModel, bool)| {
                    if show(ui, vm).is_some() {
                        *drew = true;
                    }
                },
                (vm, false),
            );
        harness.run_steps(2);
        assert!(!harness.state().1);
    }
}
