//! Report use cases. SQL lives in `infrastructure::db::reports_repo`; this layer only composes.

use rusqlite::Connection;

use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::reports::{
    CalendarMonthTotal, CategoryTotal, DayTotal, MonthSummary, PersonTotal,
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
    Ok((1..=12)
        .filter_map(|m| YearMonth::new(year, m))
        .map(|month| CalendarMonthTotal {
            month,
            summary: by_month.remove(&month).unwrap_or_default(),
        })
        .collect())
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
    use crate::domain::transaction::Allocation;
    use crate::infrastructure::db::{Db, integrity, transactions_repo};
    use crate::shared::test_fixtures::{
        date, draft_with, seed_mixed, seed_month, seed_people, seed_shared, seeded_category,
    };

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
}
