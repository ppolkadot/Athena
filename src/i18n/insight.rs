//! Localized monthly insight copy. The rules that pick a message live in
//! `domain::reports::InsightInputs`; this module only words them.

use super::{month_label, strings};
use crate::domain::money::Money;
use crate::domain::reports::{Comparison, InsightInputs, InsightState, MonthPhase, PercentChange};
use crate::domain::settings::Language;

/// The wording of one insight card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsightText {
    /// "Month to date" or "Planned" for an unfinished or future month.
    pub heading: Option<&'static str>,
    pub headline: String,
    /// The comparison line, with a separate wording for month-to-date against a full month.
    pub comparison: Option<String>,
    /// Winning category, largest expense and (live rail only) the leading person.
    pub details: Vec<String>,
}

fn brl(amount: Money) -> String {
    amount.format_brl().replacen("-R$", "−R$", 1)
}

fn percent(change: PercentChange, language: Language) -> String {
    let s = strings(language);
    match change {
        PercentChange::LessThanOne => s.insight_pct_less_than_one.to_string(),
        PercentChange::Whole(n) => s.insight_pct.replace("{n}", &n.to_string()),
    }
}

/// Word an insight. Names are resolved by the caller from current ids, so a rename shows at once.
/// `include_person` adds the leading-person line used by the live rail (saved closings omit it).
#[must_use]
pub fn insight_text(
    inputs: &InsightInputs,
    language: Language,
    category_name: impl Fn(Option<i64>) -> String,
    person_name: impl Fn(i64) -> String,
    include_person: bool,
) -> InsightText {
    let s = strings(language);
    // An unfinished or future month says "so far" or "planned" in its heading, so its state copy
    // drops the "this month" of a finished one.
    let neutral = inputs.phase != MonthPhase::Completed;
    let headline = match inputs.state() {
        InsightState::NoEntries => if neutral {
            s.insight_none_neutral
        } else {
            s.insight_none
        }
        .to_string(),
        InsightState::IncomeOnly => if neutral {
            s.insight_income_only_neutral
        } else {
            s.insight_income_only
        }
        .to_string(),
        InsightState::RefundsOnly => if neutral {
            s.insight_refunds_only_neutral
        } else {
            s.insight_refunds_only
        }
        .to_string(),
        InsightState::NetCredits(amount) => s.insight_net_credits.replace("{amount}", &brl(amount)),
        InsightState::ZeroSpending => s.insight_zero.to_string(),
        InsightState::Spending(amount) => s.insight_spending.replace("{amount}", &brl(amount)),
    };

    let prior_month = month_label(inputs.month.prev(), language);
    let full = inputs.phase == MonthPhase::ToDate;
    let comparison = match inputs.comparison() {
        Comparison::None => None,
        Comparison::Unchanged => Some(
            if full {
                s.insight_cmp_unchanged_full
            } else {
                s.insight_cmp_unchanged
            }
            .replace("{month}", &prior_month),
        ),
        Comparison::Up(change) => Some(
            if full {
                s.insight_cmp_up_full
            } else {
                s.insight_cmp_up
            }
            .replace("{percent}", &percent(change, language))
            .replace("{month}", &prior_month),
        ),
        Comparison::Down(change) => Some(
            if full {
                s.insight_cmp_down_full
            } else {
                s.insight_cmp_down
            }
            .replace("{percent}", &percent(change, language))
            .replace("{month}", &prior_month),
        ),
        Comparison::Amounts => Some(
            if full {
                s.insight_cmp_amounts_full
            } else {
                s.insight_cmp_amounts
            }
            .replace("{current}", &brl(inputs.current.spending()))
            .replace("{month}", &prior_month)
            .replace("{prior}", &brl(inputs.prior_spending)),
        ),
    };

    let mut details = Vec::new();
    if let Some((category, amount)) = inputs.winning_category {
        details.push(
            s.insight_top_category
                .replace("{name}", &category_name(category))
                .replace("{amount}", &brl(amount)),
        );
    }
    if let Some(expense) = &inputs.largest_expense {
        details.push(
            s.insight_largest_expense
                .replace("{title}", &expense.title)
                .replace("{amount}", &brl(expense.amount)),
        );
    }
    if include_person && let Some((person, amount)) = inputs.leading_person {
        details.push(
            s.insight_leading_person
                .replace("{name}", &person_name(person))
                .replace("{amount}", &brl(amount)),
        );
    }
    InsightText {
        heading: match inputs.phase {
            MonthPhase::Completed => None,
            MonthPhase::ToDate => Some(s.insight_heading_mtd),
            MonthPhase::Planned => Some(s.insight_heading_planned),
        },
        headline,
        comparison,
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::date::YearMonth;
    use crate::domain::reports::{LargestExpense, MonthSummary};

    fn inputs(summary: MonthSummary, prior: i64, phase: MonthPhase) -> InsightInputs {
        InsightInputs {
            month: YearMonth::new(2026, 10).unwrap(),
            phase,
            current: summary,
            prior_spending: Money::from_cents(prior),
            prior_count: 4,
            winning_category: Some((Some(2), Money::from_cents(40_000))),
            largest_expense: Some(LargestExpense {
                id: 9,
                title: "Rent".into(),
                amount: Money::from_cents(30_000),
            }),
            leading_person: Some((1, Money::from_cents(25_000))),
        }
    }

    fn summary(income: i64, expenses: i64, refunds: i64, count: i64) -> MonthSummary {
        MonthSummary::new(
            Money::from_cents(income),
            Money::from_cents(expenses),
            Money::from_cents(refunds),
            count,
        )
        .unwrap()
    }

    fn text(i: &InsightInputs, language: Language) -> InsightText {
        insight_text(
            i,
            language,
            |id| id.map_or("Uncategorized".into(), |_| "Food".into()),
            |_| "Ana".into(),
            true,
        )
    }

    #[test]
    fn every_headline_state_has_the_documented_copy_in_both_languages() {
        let cases = [
            (
                summary(0, 0, 0, 0),
                "No income, expenses, or refunds this month",
                "Sem receitas, despesas ou reembolsos neste mês",
            ),
            (
                summary(500, 0, 0, 1),
                "Income only this month",
                "Apenas receitas neste mês",
            ),
            (
                summary(0, 0, 300, 1),
                "Refunds only this month",
                "Apenas reembolsos neste mês",
            ),
            (
                summary(0, 100, 350, 2),
                "Refunds exceed expenses by R$ 2,50",
                "Os reembolsos superam as despesas em R$ 2,50",
            ),
            (
                summary(0, 100, 100, 2),
                "Net spending is zero",
                "Os gastos líquidos são zero",
            ),
            (
                summary(900, 400, 100, 3),
                "Net spending R$ 3,00",
                "Gastos líquidos de R$ 3,00",
            ),
        ];
        for (s, en, pt) in cases {
            let i = inputs(s, 0, MonthPhase::Completed);
            assert_eq!(text(&i, Language::En).headline, en);
            assert_eq!(text(&i, Language::PtBr).headline, pt);
        }
    }

    #[test]
    fn unfinished_and_planned_months_use_a_heading_and_date_neutral_state_copy() {
        let mtd = inputs(summary(500, 0, 0, 1), 0, MonthPhase::ToDate);
        let t = text(&mtd, Language::En);
        assert_eq!(t.heading, Some("Month to date"));
        assert_eq!(t.headline, "Income only");
        let planned = inputs(summary(0, 0, 0, 0), 0, MonthPhase::Planned);
        let t = text(&planned, Language::PtBr);
        assert_eq!(t.heading, Some("Planejado"));
        assert_eq!(t.headline, "Sem receitas, despesas ou reembolsos");
        assert_eq!(t.comparison, None, "a planned month has no comparison");
        assert_eq!(
            text(
                &inputs(summary(500, 0, 0, 1), 0, MonthPhase::Completed),
                Language::En
            )
            .heading,
            None
        );
    }

    #[test]
    fn comparisons_name_the_previous_month_and_say_when_it_is_the_full_month() {
        let up = inputs(summary(0, 12_000, 0, 1), 10_000, MonthPhase::Completed);
        assert_eq!(
            text(&up, Language::En).comparison.as_deref(),
            Some("20% higher than September 2026")
        );
        let down_mtd = inputs(summary(0, 8_000, 0, 1), 10_000, MonthPhase::ToDate);
        assert_eq!(
            text(&down_mtd, Language::En).comparison.as_deref(),
            Some("20% lower than all of September 2026")
        );
        let same = inputs(summary(0, 10_000, 0, 1), 10_000, MonthPhase::Completed);
        assert_eq!(
            text(&same, Language::PtBr).comparison.as_deref(),
            Some("Igual a Setembro 2026")
        );
        let tiny = inputs(summary(0, 10_050, 0, 1), 10_000, MonthPhase::Completed);
        assert_eq!(
            text(&tiny, Language::En).comparison.as_deref(),
            Some("Less than 1% higher than September 2026")
        );
        let amounts = inputs(summary(0, 5_000, 0, 1), -300, MonthPhase::Completed);
        assert_eq!(
            text(&amounts, Language::En).comparison.as_deref(),
            Some("Net spending R$ 50,00 · September 2026 −R$ 3,00")
        );
        let amounts_mtd = inputs(summary(0, 5_000, 0, 1), 0, MonthPhase::ToDate);
        assert_eq!(
            text(&amounts_mtd, Language::En).comparison.as_deref(),
            Some("Month to date R$ 50,00 · all of September 2026 R$ 0,00")
        );
    }

    #[test]
    fn detail_lines_resolve_current_labels_and_the_person_is_for_the_live_rail() {
        let i = inputs(summary(0, 40_000, 0, 2), 0, MonthPhase::Completed);
        let live = text(&i, Language::En);
        assert_eq!(
            live.details,
            vec![
                "Top category: Food · R$ 400,00".to_string(),
                "Largest expense: Rent · R$ 300,00".to_string(),
                "Most responsible: Ana · R$ 250,00".to_string(),
            ]
        );
        let saved = insight_text(&i, Language::En, |_| "Food".into(), |_| "Ana".into(), false);
        assert_eq!(
            saved.details.len(),
            2,
            "a saved card omits the live-only person"
        );
        let uncategorized = InsightInputs {
            winning_category: Some((None, Money::from_cents(1))),
            ..i
        };
        assert!(text(&uncategorized, Language::En).details[0].contains("Uncategorized"));
    }
}
