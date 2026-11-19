//! Report use cases. SQL lives in `infrastructure::db::reports_repo`; this layer only composes.

use rusqlite::Connection;

use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::reports::{
    CalendarMonthTotal, CategoryTotal, DayActivity, DayTotal, InsightInputs, MonthPhase,
    MonthSummary, PersonTotal,
};
use crate::infrastructure::db::reports_repo;
use crate::shared::error::{AppError, Result};

/// # Errors
/// Database errors.
pub fn month_summary(conn: &Connection, month: YearMonth) -> Result<MonthSummary> {
    let (start, end) = month.bounds();
    reports_repo::summary_between(conn, start, end)
}

/// # Errors
/// Database errors.
pub fn category_totals(conn: &Connection, month: YearMonth) -> Result<Vec<CategoryTotal>> {
    reports_repo::category_totals(conn, month)
}

/// # Errors
/// Database errors.
pub fn person_totals(conn: &Connection, month: YearMonth) -> Result<Vec<PersonTotal>> {
    reports_repo::person_totals(conn, month)
}

/// # Errors
/// Database errors.
pub fn day_totals(conn: &Connection, month: YearMonth) -> Result<Vec<DayTotal>> {
    reports_repo::day_totals(conn, month)
}

/// Always twelve entries, January first; months with no activity are zero.
///
/// # Errors
/// Database errors.
pub fn year_calendar(conn: &Connection, year: i32) -> Result<Vec<CalendarMonthTotal>> {
    let mut by_month = reports_repo::summaries_for_year(conn, year)?;
    let mut activity = reports_repo::activity_counts_for_year(conn, year)?;
    Ok((1..=12)
        .filter_map(|m| YearMonth::new(year, m))
        .map(|month| CalendarMonthTotal {
            month,
            summary: by_month.remove(&month).unwrap_or_default(),
            activity_count: activity.remove(&month).unwrap_or(0),
        })
        .collect())
}

/// Every day of `month` with live activity (all kinds), oldest first. The display model
/// zero-fills the rest.
///
/// # Errors
/// Database errors.
pub fn day_activity(conn: &Connection, month: YearMonth) -> Result<Vec<DayActivity>> {
    reports_repo::day_activity(conn, month)
}

/// The structured inputs of a month's insight. The unfiltered month is read through `today` when
/// it is the current month, so scheduled future entries never leak in; a completed or future
/// month is read whole. The comparison month is always the full previous month.
///
/// # Errors
/// Database errors or [`AppError::Validation`] on arithmetic overflow.
pub fn insight_inputs(
    conn: &Connection,
    month: YearMonth,
    today: chrono::NaiveDate,
) -> Result<InsightInputs> {
    let phase = MonthPhase::of(month, today);
    let (start, month_end) = month.bounds();
    let end = match phase {
        MonthPhase::ToDate => today.succ_opt().unwrap_or(month_end).min(month_end),
        MonthPhase::Completed | MonthPhase::Planned => month_end,
    };
    let current = reports_repo::summary_between(conn, start, end)?;
    let prior_month = month.prev();
    let prior = month_summary(conn, prior_month)?;
    let winning_category = reports_repo::category_totals_between(conn, start, end)?
        .into_iter()
        .find(|t| t.net.cents() > 0)
        .map(|t| (t.category_id, t.net));
    let leading_person = reports_repo::person_totals_between(conn, start, end)?
        .into_iter()
        .find(|t| t.net.cents() > 0)
        .map(|t| (t.person_id, t.net));
    Ok(InsightInputs {
        month,
        phase,
        current,
        prior_spending: prior.checked_spending()?,
        prior_count: prior.count,
        winning_category,
        largest_expense: reports_repo::largest_expense_between(conn, start, end)?,
        leading_person,
    })
}

/// Start balance + all income - all expenses + all refunds.
///
/// # Errors
/// Database errors or [`AppError::Validation`] on arithmetic overflow.
pub fn tracked_balance(conn: &Connection, start_balance: Money) -> Result<Money> {
    let net = reports_repo::lifetime_summary(conn)?.net;
    start_balance
        .cents()
        .checked_add(net.cents())
        .map(Money::from_cents)
        .ok_or_else(|| AppError::Validation("tracked balance overflow".into()))
}

#[cfg(test)]
mod tests {
    // The bar fraction is derived from exact halves of cent counts, so exact comparison is
    // meaningful here.
    #![allow(clippy::float_cmp)]

    use super::*;
    use crate::domain::reports::bar_fraction;
    use crate::domain::transaction::{Allocation, TransactionType};
    use crate::infrastructure::db::{Db, integrity, transactions_repo};
    use crate::shared::test_fixtures::{
        date, draft_with, seed_mixed, seed_month, seed_people, seed_shared, seeded_category,
    };
    use chrono::NaiveDate;

    fn nov() -> YearMonth {
        YearMonth::new(2026, 11).unwrap()
    }

    #[test]
    fn month_summary_nets_refunds_and_income() {
        let db = Db::open_in_memory().unwrap();
        seed_month(&db);
        let s = month_summary(db.conn(), nov()).unwrap();
        assert_eq!(s.expenses, Money::from_cents(15_000));
        assert_eq!(s.refunds, Money::from_cents(2_000));
        assert_eq!(s.income, Money::from_cents(30_000));
        assert_eq!(s.net, Money::from_cents(17_000));
        assert_eq!(s.spending(), Money::from_cents(13_000));
        assert_eq!(s.count, 4);
        assert!(integrity::check(db.conn()).unwrap().is_empty());
    }

    #[test]
    fn person_totals_do_not_double_count() {
        let db = Db::open_in_memory().unwrap();
        seed_shared(&db);
        let totals = person_totals(db.conn(), nov()).unwrap();
        assert_eq!(totals.iter().map(|t| t.net.cents()).sum::<i64>(), 20_000);
        assert_eq!(totals.len(), 2);
        assert_eq!(
            month_summary(db.conn(), nov()).unwrap().spending(),
            Money::from_cents(20_000)
        );
    }

    #[test]
    fn refunds_reduce_person_and_category_spending() {
        let db = Db::open_in_memory().unwrap();
        seed_month(&db);
        let (owner, partner) = seed_people(db.conn());
        let people = person_totals(db.conn(), nov()).unwrap();
        let net_of = |id| {
            people
                .iter()
                .find(|t| t.person_id == id)
                .map(|t| t.net.cents())
        };
        assert_eq!(net_of(owner), Some(10_500));
        assert_eq!(net_of(partner), Some(2_500));
        let categories = category_totals(db.conn(), nov()).unwrap();
        let housing = seeded_category(db.conn(), "Housing");
        let food = seeded_category(db.conn(), "Food");
        assert_eq!(
            categories[0],
            CategoryTotal {
                category_id: Some(housing),
                net: Money::from_cents(10_000)
            }
        );
        assert_eq!(
            categories[1],
            CategoryTotal {
                category_id: Some(food),
                net: Money::from_cents(5_000)
            }
        );
        // The uncategorized refund shows as negative spending, never as income.
        assert_eq!(
            categories[2],
            CategoryTotal {
                category_id: None,
                net: Money::from_cents(-2_000)
            }
        );
        assert_eq!(
            categories.iter().map(|c| c.net.cents()).sum::<i64>(),
            13_000
        );
    }

    #[test]
    fn calendar_and_balance_are_consistent() {
        let db = Db::open_in_memory().unwrap();
        seed_mixed(&db);
        let cal = year_calendar(db.conn(), 2026).unwrap();
        assert_eq!(cal.len(), 12);
        assert_eq!(cal[10].month.month, 11);
        assert_eq!(cal[10].summary, month_summary(db.conn(), nov()).unwrap());
        assert_eq!(cal[0].summary, MonthSummary::default());
        assert_eq!(
            tracked_balance(db.conn(), Money::from_cents(100_000)).unwrap(),
            Money::from_cents(117_000)
        );
    }

    #[test]
    fn day_totals_are_net_spending_by_day() {
        let db = Db::open_in_memory().unwrap();
        seed_month(&db);
        let days = day_totals(db.conn(), nov()).unwrap();
        let pairs: Vec<(u32, i64)> = days
            .iter()
            .map(|d| (chrono::Datelike::day(&d.date), d.net.cents()))
            .collect();
        assert_eq!(pairs, vec![(1, 10_000), (5, 5_000), (8, -2_000)]);
    }

    #[test]
    fn december_january_boundary_lands_in_the_right_month() {
        let db = Db::open_in_memory().unwrap();
        let mut a = draft_with(vec![]);
        a.amount = Money::from_cents(100);
        a.date = date(2026, 12, 31);
        let mut b = draft_with(vec![]);
        b.amount = Money::from_cents(200);
        b.date = date(2027, 1, 1);
        transactions_repo::create(db.conn(), &a).unwrap();
        transactions_repo::create(db.conn(), &b).unwrap();
        let dec = month_summary(db.conn(), YearMonth::new(2026, 12).unwrap()).unwrap();
        let jan = month_summary(db.conn(), YearMonth::new(2027, 1).unwrap()).unwrap();
        assert_eq!(dec.expenses.cents(), 100);
        assert_eq!(jan.expenses.cents(), 200);
        assert_eq!(
            year_calendar(db.conn(), 2026).unwrap()[11]
                .summary
                .expenses
                .cents(),
            100
        );
        assert_eq!(
            year_calendar(db.conn(), 2027).unwrap()[0]
                .summary
                .expenses
                .cents(),
            200
        );
    }

    #[test]
    fn empty_database_has_no_nan_or_division_by_zero() {
        let db = Db::open_in_memory().unwrap();
        let totals = category_totals(db.conn(), nov()).unwrap();
        assert!(totals.is_empty());
        assert_eq!(bar_fraction(Money::ZERO, Money::ZERO), 0.0);
        assert_eq!(
            month_summary(db.conn(), nov()).unwrap(),
            MonthSummary::default()
        );
        assert!(person_totals(db.conn(), nov()).unwrap().is_empty());
        assert!(day_totals(db.conn(), nov()).unwrap().is_empty());
        assert_eq!(year_calendar(db.conn(), 2026).unwrap().len(), 12);
        assert_eq!(
            tracked_balance(db.conn(), Money::from_cents(-500)).unwrap(),
            Money::from_cents(-500)
        );
    }

    #[test]
    fn unassigned_expenses_count_in_the_transaction_view_only() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        transactions_repo::create(db.conn(), &draft_with(vec![])).unwrap();
        transactions_repo::create(
            db.conn(),
            &draft_with(vec![Allocation {
                person_id: owner,
                amount: Money::from_cents(300),
            }]),
        )
        .unwrap();
        assert_eq!(
            month_summary(db.conn(), nov()).unwrap().expenses.cents(),
            400
        );
        let people = person_totals(db.conn(), nov()).unwrap();
        assert_eq!(people.iter().map(|p| p.net.cents()).sum::<i64>(), 300);
    }

    #[test]
    fn settlement_is_excluded_from_every_aggregate() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = crate::shared::test_fixtures::seed_all_kinds(&db);
        let summary = month_summary(db.conn(), nov()).unwrap();
        assert_eq!(summary.income, Money::from_cents(30_000));
        assert_eq!(summary.expenses, Money::from_cents(10_000));
        assert_eq!(summary.refunds, Money::from_cents(2_000));
        assert_eq!(summary.net, Money::from_cents(22_000));
        assert_eq!(summary.count, 3);

        let cats = category_totals(db.conn(), nov()).unwrap();
        assert_eq!(cats.iter().map(|c| c.net.cents()).sum::<i64>(), 8_000);

        let people = person_totals(db.conn(), nov()).unwrap();
        assert_eq!(people.iter().map(|p| p.net.cents()).sum::<i64>(), 8_000);
        assert!(people.iter().all(|p| p.person_id == owner));

        let days = day_totals(db.conn(), nov()).unwrap();
        assert_eq!(days.len(), 2);
        assert!(days.iter().all(|d| d.date != date(2026, 11, 10)));

        let cal = year_calendar(db.conn(), 2026).unwrap();
        assert_eq!(cal[10].summary, summary);
        assert_eq!(
            tracked_balance(db.conn(), Money::from_cents(100_000)).unwrap(),
            Money::from_cents(122_000)
        );

        let shared_only = transactions_repo::list(
            db.conn(),
            &crate::domain::transaction::TxFilter {
                shared_only: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(shared_only.is_empty());

        let by_payer = transactions_repo::list(
            db.conn(),
            &crate::domain::transaction::TxFilter {
                person_id: Some(partner),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_payer.len(), 1);
        let by_recipient = transactions_repo::list(
            db.conn(),
            &crate::domain::transaction::TxFilter {
                person_id: Some(owner),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(by_recipient.len(), 3);
    }

    #[derive(Debug, PartialEq)]
    struct ReportSnapshot {
        txs: Vec<crate::domain::transaction::Transaction>,
        search_len: usize,
        summary: MonthSummary,
        cats: Vec<CategoryTotal>,
        people: Vec<PersonTotal>,
        days: Vec<DayTotal>,
        cal: Vec<CalendarMonthTotal>,
        balance: Money,
        settlement: crate::domain::settlement::SettlementView,
        csv_count: usize,
        csv_has_market: bool,
        json_count: usize,
        json_titles: Vec<String>,
    }

    impl ReportSnapshot {
        fn capture(db: &Db, tag: &str) -> Self {
            use crate::application::services::export::{export_csv, export_json};
            use crate::domain::export::ExportScope;
            use crate::domain::settings::Language;
            use crate::domain::transaction::TxFilter;
            use crate::infrastructure::export::format_v3::DocumentV3;
            use crate::shared::test_fixtures::profile;

            let dir = tempfile::tempdir().unwrap();
            let csv_path = dir.path().join(format!("{tag}.csv"));
            let csv_count = export_csv(db.conn(), Language::En, ExportScope::All, &csv_path)
                .unwrap()
                .record_count;
            let csv_has_market = std::fs::read_to_string(&csv_path)
                .unwrap()
                .contains("Market");
            let json_path = dir.path().join(format!("{tag}.json"));
            let json_count = export_json(db.conn(), &profile(), "2026-11-12T10:00:00Z", &json_path)
                .unwrap()
                .record_count;
            let doc: DocumentV3 =
                serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
            Self {
                txs: transactions_repo::list(db.conn(), &TxFilter::default()).unwrap(),
                search_len: transactions_repo::list(
                    db.conn(),
                    &TxFilter {
                        text: "Market".into(),
                        ..TxFilter::default()
                    },
                )
                .unwrap()
                .len(),
                summary: month_summary(db.conn(), nov()).unwrap(),
                cats: category_totals(db.conn(), nov()).unwrap(),
                people: person_totals(db.conn(), nov()).unwrap(),
                days: day_totals(db.conn(), nov()).unwrap(),
                cal: year_calendar(db.conn(), 2026).unwrap(),
                balance: tracked_balance(db.conn(), Money::from_cents(100_000)).unwrap(),
                settlement: crate::application::services::settlement::for_month(db.conn(), nov())
                    .unwrap(),
                csv_count,
                csv_has_market,
                json_count,
                json_titles: doc.transactions.into_iter().map(|t| t.title).collect(),
            }
        }
    }

    #[test]
    fn deleted_rows_vanish_everywhere() {
        use crate::infrastructure::db::trash_repo;

        let db = Db::open_in_memory().unwrap();
        seed_month(&db);
        let baseline = ReportSnapshot::capture(&db, "base");
        let market_id = baseline
            .txs
            .iter()
            .find(|t| t.title == "Market")
            .unwrap()
            .id;

        transactions_repo::delete(db.conn(), market_id).unwrap();
        let deleted = ReportSnapshot::capture(&db, "del");
        assert_eq!(deleted.txs.len(), 3);
        assert_eq!(deleted.search_len, 0);
        assert_eq!(deleted.summary.expenses, Money::from_cents(10_000));
        assert_eq!(deleted.summary.count, 3);
        let food = seeded_category(db.conn(), "Food");
        assert!(deleted.cats.iter().all(|c| c.category_id != Some(food)));
        assert_eq!(
            deleted.people.iter().map(|p| p.net.cents()).sum::<i64>(),
            8_000
        );
        assert!(deleted.days.iter().all(|d| d.date != date(2026, 11, 5)));
        assert_eq!(deleted.cal[10].summary, deleted.summary);
        assert_eq!(deleted.balance, Money::from_cents(122_000));
        assert!(deleted.settlement.transfers.is_empty());
        assert_eq!(deleted.settlement.balances.len(), 1);
        assert_eq!(deleted.csv_count, 3);
        assert!(!deleted.csv_has_market);
        assert_eq!(deleted.json_count, 3);
        assert!(deleted.json_titles.iter().all(|t| t != "Market"));

        trash_repo::restore(db.conn(), market_id).unwrap();
        assert_eq!(ReportSnapshot::capture(&db, "restored"), baseline);

        // Soft-deleting a Settlement payback removes it from settlement math until restored.
        let (owner, partner) = seed_people(db.conn());
        let mut payback = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(2_500),
        }]);
        payback.title = "Payback".into();
        payback.kind = crate::domain::transaction::TransactionType::Settlement;
        payback.payer_id = Some(partner);
        payback.date = date(2026, 11, 20);
        let settlement_id = transactions_repo::create(db.conn(), &payback).unwrap();
        let after_payback =
            crate::application::services::settlement::for_month(db.conn(), nov()).unwrap();
        assert!(after_payback.transfers.is_empty());
        assert_eq!(after_payback.balances.len(), 2);

        transactions_repo::delete(db.conn(), settlement_id).unwrap();
        assert_eq!(ReportSnapshot::capture(&db, "del_settlement"), baseline);

        trash_repo::restore(db.conn(), settlement_id).unwrap();
        let after_restore =
            crate::application::services::settlement::for_month(db.conn(), nov()).unwrap();
        assert!(after_restore.transfers.is_empty());
    }

    #[test]
    fn tracked_balance_overflow_returns_validation_error() {
        let db = Db::open_in_memory().unwrap();
        let mut inc = draft_with(vec![]);
        inc.kind = crate::domain::transaction::TransactionType::Income;
        inc.amount = Money::from_cents(100);
        transactions_repo::create(db.conn(), &inc).unwrap();

        let err = tracked_balance(db.conn(), Money::from_cents(i64::MAX)).unwrap_err();
        assert!(matches!(err, crate::shared::error::AppError::Validation(_)));
    }

    fn tx(
        db: &Db,
        owner: i64,
        title: &str,
        cents: i64,
        kind: TransactionType,
        day: NaiveDate,
        category: Option<i64>,
    ) -> i64 {
        let mut draft = draft_with(vec![]);
        draft.title = title.into();
        draft.amount = Money::from_cents(cents);
        draft.kind = kind;
        draft.date = day;
        draft.category_id = category;
        if kind != TransactionType::Income {
            draft.payer_id = Some(owner);
            draft.allocations = vec![Allocation {
                person_id: owner,
                amount: Money::from_cents(cents),
            }];
        }
        transactions_repo::create(db.conn(), &draft).unwrap()
    }

    #[test]
    fn day_activity_counts_every_kind_but_spends_only_expenses_minus_refunds() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let d = |day| date(2026, 11, day);
        tx(
            &db,
            owner,
            "Salary",
            300_000,
            TransactionType::Income,
            d(5),
            None,
        );
        tx(
            &db,
            owner,
            "Payback",
            4_000,
            TransactionType::Refund,
            d(8),
            None,
        );
        tx(
            &db,
            owner,
            "Lunch",
            2_500,
            TransactionType::Expense,
            d(12),
            None,
        );
        tx(
            &db,
            owner,
            "Lunch back",
            2_500,
            TransactionType::Refund,
            d(12),
            None,
        );
        let trashed = tx(
            &db,
            owner,
            "Gone",
            900,
            TransactionType::Expense,
            d(14),
            None,
        );
        transactions_repo::delete(db.conn(), trashed).unwrap();
        let mut settlement = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(1_000),
        }]);
        settlement.kind = TransactionType::Settlement;
        settlement.payer_id = Some(partner);
        settlement.date = d(20);
        transactions_repo::create(db.conn(), &settlement).unwrap();

        let days = day_activity(db.conn(), nov()).unwrap();
        let on = |day| days.iter().find(|a| a.date == d(day));
        // Income-only: discoverable by count, no spending.
        assert_eq!(
            (on(5).unwrap().activity_count, on(5).unwrap().spending),
            (1, Money::ZERO)
        );
        // Refund-only: negative spending, still one activity.
        assert_eq!(on(8).unwrap().spending, Money::from_cents(-4_000));
        // Offsetting expense and refund: two activities, zero net spending.
        assert_eq!(
            (on(12).unwrap().activity_count, on(12).unwrap().spending),
            (2, Money::ZERO)
        );
        // Settlement-only: one activity, zero spending.
        assert_eq!(
            (on(20).unwrap().activity_count, on(20).unwrap().spending),
            (1, Money::ZERO)
        );
        // Trashed and empty days are absent here and zero-filled by the display model.
        assert!(on(14).is_none());
        let filled = crate::domain::reports::zero_filled_days(nov(), &days);
        assert_eq!(filled.len(), 30);
        assert_eq!(filled[13].activity_count, 0);
        // The financial count is unchanged: settlements stay out of it.
        assert_eq!(month_summary(db.conn(), nov()).unwrap().count, 4);
        assert!(integrity::check(db.conn()).unwrap().is_empty());
    }

    #[test]
    fn calendar_activity_count_makes_a_settlement_only_month_discoverable() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let mut settlement = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(1_000),
        }]);
        settlement.kind = TransactionType::Settlement;
        settlement.payer_id = Some(partner);
        settlement.date = date(2026, 3, 2);
        transactions_repo::create(db.conn(), &settlement).unwrap();
        let calendar = year_calendar(db.conn(), 2026).unwrap();
        let march = &calendar[2];
        assert_eq!(
            march.summary.count, 0,
            "financial counts keep their meaning"
        );
        assert_eq!(march.activity_count, 1);
        assert_eq!(calendar.iter().filter(|m| m.activity_count > 0).count(), 1);
    }

    #[test]
    fn the_day_filter_is_exact_across_kinds_and_ignores_the_month() {
        use crate::domain::transaction::TxFilter;
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        tx(
            &db,
            owner,
            "A",
            100,
            TransactionType::Expense,
            date(2026, 12, 31),
            None,
        );
        tx(
            &db,
            owner,
            "B",
            100,
            TransactionType::Income,
            date(2026, 12, 31),
            None,
        );
        tx(
            &db,
            owner,
            "C",
            100,
            TransactionType::Expense,
            date(2027, 1, 1),
            None,
        );
        tx(
            &db,
            owner,
            "D",
            100,
            TransactionType::Expense,
            date(2024, 2, 29),
            None,
        );
        let on = |day| {
            transactions_repo::list(
                db.conn(),
                &TxFilter {
                    day: Some(day),
                    ..TxFilter::default()
                },
            )
            .unwrap()
        };
        assert_eq!(on(date(2026, 12, 31)).len(), 2);
        assert_eq!(on(date(2027, 1, 1)).len(), 1);
        assert_eq!(on(date(2024, 2, 29)).len(), 1);
        assert!(on(date(2026, 12, 30)).is_empty());
    }

    #[test]
    fn insights_read_the_current_month_through_today_and_the_full_prior_month() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let food = seeded_category(db.conn(), "Food");
        let d = |m, day| date(2026, m, day);
        tx(
            &db,
            owner,
            "Oct rent",
            50_000,
            TransactionType::Expense,
            d(10, 3),
            None,
        );
        tx(
            &db,
            owner,
            "Oct late",
            10_000,
            TransactionType::Expense,
            d(10, 28),
            Some(food),
        );
        tx(
            &db,
            owner,
            "Nov early",
            8_000,
            TransactionType::Expense,
            d(11, 4),
            Some(food),
        );
        tx(
            &db,
            owner,
            "Nov later",
            90_000,
            TransactionType::Expense,
            d(11, 25),
            None,
        );
        let today = d(11, 10);
        let nov_inputs = insight_inputs(db.conn(), nov(), today).unwrap();
        assert_eq!(nov_inputs.phase, MonthPhase::ToDate);
        // Scheduled future entries never leak into a month in progress.
        assert_eq!(nov_inputs.current.spending(), Money::from_cents(8_000));
        assert_eq!(nov_inputs.prior_spending, Money::from_cents(60_000));
        assert_eq!(nov_inputs.prior_count, 2);
        assert_eq!(
            nov_inputs.winning_category,
            Some((Some(food), Money::from_cents(8_000)))
        );
        assert_eq!(
            nov_inputs.largest_expense.as_ref().unwrap().title,
            "Nov early"
        );
        assert_eq!(
            nov_inputs.leading_person,
            Some((owner, Money::from_cents(8_000)))
        );
        // A completed month reads whole; a future month is planned.
        let oct = insight_inputs(db.conn(), YearMonth::new(2026, 10).unwrap(), d(11, 10)).unwrap();
        assert_eq!(oct.phase, MonthPhase::Completed);
        assert_eq!(oct.current.spending(), Money::from_cents(60_000));
        assert_eq!(oct.largest_expense.unwrap().title, "Oct rent");
        let dec = insight_inputs(db.conn(), YearMonth::new(2026, 12).unwrap(), d(11, 10)).unwrap();
        assert_eq!(dec.phase, MonthPhase::Planned);
        assert_eq!(dec.current.count, 0);
        assert_eq!(
            dec.prior_spending,
            Money::from_cents(98_000),
            "full November"
        );
    }

    #[test]
    fn insight_ties_prefer_uncategorized_then_the_lowest_id() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let food = seeded_category(db.conn(), "Food");
        let day = date(2026, 10, 5);
        let a = tx(
            &db,
            owner,
            "Alpha",
            5_000,
            TransactionType::Expense,
            day,
            Some(food),
        );
        tx(
            &db,
            partner,
            "Beta",
            5_000,
            TransactionType::Expense,
            day,
            None,
        );
        let inputs = insight_inputs(
            db.conn(),
            YearMonth::new(2026, 10).unwrap(),
            date(2026, 11, 20),
        )
        .unwrap();
        assert_eq!(
            inputs.winning_category,
            Some((None, Money::from_cents(5_000))),
            "an uncategorized tie wins"
        );
        assert_eq!(
            inputs.largest_expense.unwrap().id,
            a,
            "equal amounts: lowest id"
        );
        // Equal people: the lower id leads.
        assert_eq!(inputs.leading_person.unwrap().0, owner.min(partner));
    }
}
