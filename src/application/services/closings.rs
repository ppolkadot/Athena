//! Monthly closings: capture from one consistent SQL snapshot, replacement, and staleness
//! (contract §14.3). Reads go through `live_transactions`; closing never writes the ledger.

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::reports;
use crate::domain::closing::{ClosingError, ClosingSnapshot, ClosingStatus, is_completed};
use crate::domain::date::YearMonth;
use crate::infrastructure::db::closings_repo;
use crate::shared::error::{AppError, Result};

fn refuse(error: ClosingError) -> AppError {
    AppError::ClosingInvalid(error)
}

/// Capture `month` as of `today`: one consistent read of the completed month, its predecessor and
/// both fingerprints. The caller keeps the surrounding transaction open until the write.
fn capture(
    conn: &Connection,
    month: YearMonth,
    today: NaiveDate,
    closed_at: &str,
) -> Result<ClosingSnapshot> {
    if !is_completed(month, today) {
        return Err(refuse(ClosingError::NotCompleted));
    }
    let inputs = reports::insight_inputs(conn, month, today)?;
    let current = closings_repo::month_fingerprint(conn, month)?;
    let previous = closings_repo::month_fingerprint(conn, month.prev())?;
    ClosingSnapshot::capture(&inputs, closed_at.to_string(), current, previous).map_err(refuse)
}

fn immediate(conn: &Connection) -> Result<Transaction<'_>> {
    Ok(Transaction::new_unchecked(
        conn,
        TransactionBehavior::Immediate,
    )?)
}

/// Save a closing for a completed local month that has none.
///
/// # Errors
/// [`AppError::ClosingInvalid`] for a current or future month or an existing closing; database
/// errors. Nothing is written on error.
pub fn close_month(
    conn: &Connection,
    month: YearMonth,
    today: NaiveDate,
    closed_at: &str,
) -> Result<()> {
    let tx = immediate(conn)?;
    if closings_repo::get(&tx, month)?.is_some() {
        return Err(refuse(ClosingError::AlreadyClosed));
    }
    let snapshot = capture(&tx, month, today, closed_at)?;
    closings_repo::insert(&tx, &snapshot)?;
    tx.commit()?;
    Ok(())
}

/// Replace an existing closing with a fresh capture of the same month.
///
/// # Errors
/// [`AppError::ClosingInvalid`] for a current or future month or a month without a closing;
/// database errors. The old closing stays on any error.
pub fn recalculate(
    conn: &Connection,
    month: YearMonth,
    today: NaiveDate,
    closed_at: &str,
) -> Result<()> {
    let tx = immediate(conn)?;
    if closings_repo::get(&tx, month)?.is_none() {
        return Err(refuse(ClosingError::NotClosed));
    }
    let snapshot = capture(&tx, month, today, closed_at)?;
    closings_repo::replace(&tx, &snapshot)?;
    tx.commit()?;
    Ok(())
}

/// Every saved closing keyed by month.
///
/// # Errors
/// Database errors.
pub fn all(conn: &Connection) -> Result<BTreeMap<YearMonth, ClosingSnapshot>> {
    Ok(closings_repo::list(conn)?
        .into_iter()
        .map(|c| (c.month, c))
        .collect())
}

/// Month fingerprints computed once per ledger version and reused between adjacent months. The
/// owner drops it whenever the ledger or the data set changes.
#[derive(Debug, Default, Clone)]
pub struct FingerprintCache {
    by_month: HashMap<YearMonth, String>,
}

impl FingerprintCache {
    pub fn clear(&mut self) {
        self.by_month.clear();
    }

    /// The fingerprint of `month`, computed on first use.
    ///
    /// # Errors
    /// Database errors.
    pub fn get(&mut self, conn: &Connection, month: YearMonth) -> Result<String> {
        if let Some(found) = self.by_month.get(&month) {
            return Ok(found.clone());
        }
        let computed = closings_repo::month_fingerprint(conn, month)?;
        self.by_month.insert(month, computed.clone());
        Ok(computed)
    }

    /// How many months are cached (tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_month.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_month.is_empty()
    }
}

/// Whether the ledger still matches `snapshot`: its own month and its predecessor are recomputed
/// and compared with the saved fingerprints.
///
/// # Errors
/// Database errors; the caller shows [`ClosingStatus::Unavailable`] and never claims freshness.
pub fn status(
    conn: &Connection,
    cache: &mut FingerprintCache,
    snapshot: &ClosingSnapshot,
) -> Result<ClosingStatus> {
    let current = cache.get(conn, snapshot.month)?;
    let previous = cache.get(conn, snapshot.month.prev())?;
    Ok(
        if current == snapshot.current_fingerprint && previous == snapshot.previous_fingerprint {
            ClosingStatus::Current
        } else {
            ClosingStatus::Changed
        },
    )
}

/// Statuses for `months` that have a saved closing. A failed check marks every month it touched
/// [`ClosingStatus::Unavailable`] instead of failing the refresh.
pub fn statuses(
    conn: &Connection,
    cache: &mut FingerprintCache,
    closings: &BTreeMap<YearMonth, ClosingSnapshot>,
    months: &[YearMonth],
) -> HashMap<YearMonth, ClosingStatus> {
    let mut out = HashMap::new();
    for month in months {
        let Some(snapshot) = closings.get(month) else {
            continue;
        };
        let result = status(conn, cache, snapshot).unwrap_or_else(|err| {
            tracing::warn!("closing check unavailable for {}: {err}", month.key());
            ClosingStatus::Unavailable
        });
        out.insert(*month, result);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::domain::reports::{Comparison, PercentChange};
    use crate::domain::transaction::{TransactionType, TxDraft};
    use crate::infrastructure::db::{
        Db, categories_repo, integrity, transactions_repo, trash_repo,
    };
    use crate::shared::test_fixtures::{date, draft_with, seeded_category};

    const STAMP: &str = "2026-11-19T12:00:00Z";

    fn ym(year: i32, month: u32) -> YearMonth {
        YearMonth::new(year, month).unwrap()
    }

    fn today() -> NaiveDate {
        date(2026, 11, 19)
    }

    fn entry(
        db: &Db,
        on: NaiveDate,
        kind: TransactionType,
        cents: i64,
        category: Option<i64>,
        title: &str,
    ) -> i64 {
        let mut draft: TxDraft = draft_with(vec![]);
        draft.date = on;
        draft.kind = kind;
        draft.amount = Money::from_cents(cents);
        draft.category_id = category;
        draft.title = title.into();
        transactions_repo::create(db.conn(), &draft).unwrap()
    }

    fn expense(db: &Db, on: NaiveDate, cents: i64, category: Option<i64>, title: &str) -> i64 {
        entry(db, on, TransactionType::Expense, cents, category, title)
    }

    fn clean(db: &Db) {
        assert_eq!(integrity::check(db.conn()).unwrap(), vec![]);
    }

    fn comparison(snapshot: &ClosingSnapshot) -> Comparison {
        snapshot.insight_inputs().comparison()
    }

    /// An edit of an existing entry's title, keeping everything else.
    fn retitle(db: &Db, id: i64, title: &str) {
        let stored = transactions_repo::get(db.conn(), id).unwrap();
        let mut draft = draft_with(vec![]);
        draft.title = title.into();
        draft.date = stored.date;
        draft.amount = stored.amount;
        draft.kind = stored.kind;
        draft.category_id = stored.category_id;
        transactions_repo::update(db.conn(), id, &draft).unwrap();
    }

    #[test]
    fn closing_uses_previous_spending_when_previous_results_match() {
        // Both previous months have a net result of -R$ 1,00, but different spending.
        let low = Db::open_in_memory().unwrap();
        expense(&low, date(2026, 9, 3), 100, None, "Low");
        let high = Db::open_in_memory().unwrap();
        expense(&high, date(2026, 9, 3), 400, None, "High");
        entry(
            &high,
            date(2026, 9, 4),
            TransactionType::Income,
            300,
            None,
            "Pay",
        );
        for db in [&low, &high] {
            expense(db, date(2026, 10, 3), 200, None, "Current");
            close_month(db.conn(), ym(2026, 10), today(), STAMP).unwrap();
            clean(db);
        }
        let a = closings_repo::get(low.conn(), ym(2026, 10))
            .unwrap()
            .unwrap();
        let b = closings_repo::get(high.conn(), ym(2026, 10))
            .unwrap()
            .unwrap();
        assert_eq!(a.previous_spending.cents(), 100);
        assert_eq!(b.previous_spending.cents(), 400);
        assert_eq!((a.previous_count, b.previous_count), (1, 2));
        // Spending went up against R$ 1,00 and down against R$ 4,00.
        assert_eq!(comparison(&a), Comparison::Up(PercentChange::Whole(100)));
        assert_eq!(comparison(&b), Comparison::Down(PercentChange::Whole(50)));
    }

    #[test]
    fn capture_stores_the_whole_month_and_stable_winners() {
        let db = Db::open_in_memory().unwrap();
        let food = seeded_category(db.conn(), "Food");
        let big = expense(&db, date(2026, 10, 2), 5000, Some(food), "Big shop");
        expense(&db, date(2026, 10, 9), 5000, None, "Uncategorized tie");
        entry(
            &db,
            date(2026, 10, 12),
            TransactionType::Income,
            20_000,
            None,
            "Pay",
        );
        entry(
            &db,
            date(2026, 10, 13),
            TransactionType::Refund,
            700,
            Some(food),
            "Return",
        );
        close_month(db.conn(), ym(2026, 10), today(), STAMP).unwrap();
        let c = closings_repo::get(db.conn(), ym(2026, 10))
            .unwrap()
            .unwrap();
        assert_eq!(
            (c.income.cents(), c.expenses.cents(), c.refunds.cents()),
            (20_000, 10_000, 700)
        );
        assert_eq!(c.result.cents(), 10_700);
        assert_eq!(c.count, 4);
        // Food nets 4,300 and Uncategorized 5,000: the uncategorized winner has no id.
        assert_eq!(c.top_category, Some((None, Money::from_cents(5000))));
        // Equal amounts pick the lower id.
        assert_eq!(c.largest_expense.as_ref().unwrap().id, Some(big));
        assert_eq!(c.closed_at, STAMP);
        assert_eq!(c.fingerprint_version, 1);
        assert!(c.validate().is_ok());
        clean(&db);
    }

    #[test]
    fn empty_and_refund_only_months_close_with_no_winners() {
        let db = Db::open_in_memory().unwrap();
        close_month(db.conn(), ym(2026, 9), today(), STAMP).unwrap();
        let empty = closings_repo::get(db.conn(), ym(2026, 9)).unwrap().unwrap();
        assert_eq!(empty.count, 0);
        assert_eq!(empty.top_category, None);
        assert_eq!(empty.largest_expense, None);
        entry(
            &db,
            date(2026, 10, 5),
            TransactionType::Refund,
            300,
            None,
            "Refund",
        );
        close_month(db.conn(), ym(2026, 10), today(), STAMP).unwrap();
        let refund = closings_repo::get(db.conn(), ym(2026, 10))
            .unwrap()
            .unwrap();
        assert_eq!(refund.refunds.cents(), 300);
        assert_eq!(refund.result.cents(), 300);
        assert_eq!(refund.top_category, None);
        assert_eq!(refund.previous_count, 0);
        clean(&db);
    }

    #[test]
    fn only_completed_months_close_and_the_two_actions_check_existence() {
        let db = Db::open_in_memory().unwrap();
        for month in [ym(2026, 11), ym(2026, 12), ym(2030, 1)] {
            assert!(matches!(
                close_month(db.conn(), month, today(), STAMP),
                Err(AppError::ClosingInvalid(ClosingError::NotCompleted))
            ));
        }
        // The last day of the month is still the current month; the next day completes it.
        assert!(close_month(db.conn(), ym(2026, 10), date(2026, 10, 31), STAMP).is_err());
        close_month(db.conn(), ym(2026, 10), date(2026, 11, 1), STAMP).unwrap();
        assert!(matches!(
            close_month(db.conn(), ym(2026, 10), today(), STAMP),
            Err(AppError::ClosingInvalid(ClosingError::AlreadyClosed))
        ));
        assert!(matches!(
            recalculate(db.conn(), ym(2026, 8), today(), STAMP),
            Err(AppError::ClosingInvalid(ClosingError::NotClosed))
        ));
        assert!(matches!(
            recalculate(db.conn(), ym(2026, 11), today(), STAMP),
            Err(AppError::ClosingInvalid(ClosingError::NotClosed))
        ));
        assert_eq!(closings_repo::count(db.conn()).unwrap(), 1);
        clean(&db);
    }

    #[test]
    fn closing_detects_both_months_and_ignores_label_renames() {
        let db = Db::open_in_memory().unwrap();
        let food = seeded_category(db.conn(), "Food");
        let sep = expense(&db, date(2026, 9, 10), 100, Some(food), "Sep row");
        let oct = expense(&db, date(2026, 10, 10), 200, Some(food), "Oct row");
        let oct2 = expense(&db, date(2026, 10, 11), 300, None, "Other");
        close_month(db.conn(), ym(2026, 10), today(), STAMP).unwrap();
        let snap = || {
            closings_repo::get(db.conn(), ym(2026, 10))
                .unwrap()
                .unwrap()
        };
        let check = |db: &Db| status(db.conn(), &mut FingerprintCache::default(), &snap()).unwrap();
        assert_eq!(check(&db), ClosingStatus::Current);

        // A category rename is a label change, not a financial one.
        let mut renamed = categories_repo::get(db.conn(), food).unwrap().unwrap();
        renamed.name = "Groceries".into();
        categories_repo::update(db.conn(), &renamed).unwrap();
        assert_eq!(check(&db), ClosingStatus::Current);

        // Editing the previous month changes the comparison.
        retitle(&db, sep, "Sep renamed");
        assert_eq!(check(&db), ClosingStatus::Changed);
        // The saved figures stay fixed until recalculated; recalculation clears the marker.
        assert_eq!(snap().previous_spending.cents(), 100);
        recalculate(db.conn(), ym(2026, 10), today(), "2026-11-19T13:00:00Z").unwrap();
        assert_eq!(snap().closed_at, "2026-11-19T13:00:00Z");
        assert_eq!(check(&db), ClosingStatus::Current);

        // Editing the closed month itself (a non-maximum row) is detected.
        retitle(&db, oct, "Oct renamed");
        assert_eq!(check(&db), ClosingStatus::Changed);
        recalculate(db.conn(), ym(2026, 10), today(), STAMP).unwrap();

        // Same timestamps with different content: a direct change of the title is still caught.
        db.conn()
            .execute(
                "UPDATE transactions SET title = 'changed' WHERE id = ?1",
                [oct2],
            )
            .unwrap();
        assert_eq!(check(&db), ClosingStatus::Changed);
        db.conn()
            .execute(
                "UPDATE transactions SET title = 'Other' WHERE id = ?1",
                [oct2],
            )
            .unwrap();
        assert_eq!(check(&db), ClosingStatus::Current);

        // Soft delete changes the live set; undo (restore) brings the same record back.
        transactions_repo::delete(db.conn(), oct2).unwrap();
        assert_eq!(check(&db), ClosingStatus::Changed);
        trash_repo::restore(db.conn(), oct2).unwrap();
        assert_eq!(check(&db), ClosingStatus::Current);

        // Moving a row to another month changes the record of both months.
        let mut moved = draft_with(vec![]);
        moved.title = "Other".into();
        moved.amount = Money::from_cents(300);
        moved.date = date(2026, 11, 2);
        transactions_repo::update(db.conn(), oct2, &moved).unwrap();
        assert_eq!(check(&db), ClosingStatus::Changed);
        clean(&db);
    }

    #[test]
    fn a_purged_largest_expense_keeps_its_title_and_amount() {
        let db = Db::open_in_memory().unwrap();
        let big = expense(&db, date(2026, 10, 10), 9000, None, "Sofa");
        expense(&db, date(2026, 10, 11), 100, None, "Pen");
        close_month(db.conn(), ym(2026, 10), today(), STAMP).unwrap();
        let before = closings_repo::get(db.conn(), ym(2026, 10))
            .unwrap()
            .unwrap();
        assert!(closings_repo::is_live_transaction(db.conn(), big).unwrap());
        transactions_repo::delete(db.conn(), big).unwrap();
        // In the Trash the row is no longer live, but the reference is kept.
        assert!(!closings_repo::is_live_transaction(db.conn(), big).unwrap());
        trash_repo::purge(db.conn(), big).unwrap();
        let after = closings_repo::get(db.conn(), ym(2026, 10))
            .unwrap()
            .unwrap();
        let kept = after.largest_expense.unwrap();
        assert_eq!(kept.id, None);
        assert_eq!((kept.title.as_str(), kept.amount.cents()), ("Sofa", 9000));
        assert_eq!(after.expenses, before.expenses);
        clean(&db);
    }

    #[test]
    fn closing_replacement_is_atomic() {
        let db = Db::open_in_memory().unwrap();
        expense(&db, date(2026, 10, 10), 500, None, "A");
        close_month(db.conn(), ym(2026, 10), today(), STAMP).unwrap();
        let original = closings_repo::get(db.conn(), ym(2026, 10))
            .unwrap()
            .unwrap();
        // A replacement whose reference is refused part-way leaves the old closing untouched.
        let mut bad = original.clone();
        bad.closed_at = "2099-01-01T00:00:00Z".into();
        bad.top_category = Some((Some(9999), Money::from_cents(1)));
        assert!(closings_repo::replace(db.conn(), &bad).is_err());
        assert_eq!(
            closings_repo::get(db.conn(), ym(2026, 10))
                .unwrap()
                .unwrap(),
            original
        );
        // An inconsistent snapshot is refused before anything is touched.
        let mut skewed = original.clone();
        skewed.result = Money::from_cents(1);
        assert!(matches!(
            closings_repo::replace(db.conn(), &skewed),
            Err(AppError::ClosingInvalid(ClosingError::Inconsistent))
        ));
        assert_eq!(closings_repo::count(db.conn()).unwrap(), 1);
        // A successful recalculation replaces the whole row.
        expense(&db, date(2026, 10, 12), 700, None, "B");
        recalculate(db.conn(), ym(2026, 10), today(), "2026-11-19T15:00:00Z").unwrap();
        let replaced = closings_repo::get(db.conn(), ym(2026, 10))
            .unwrap()
            .unwrap();
        assert_eq!(replaced.expenses.cents(), 1200);
        assert_ne!(replaced.current_fingerprint, original.current_fingerprint);
        assert_eq!(closings_repo::count(db.conn()).unwrap(), 1);
        clean(&db);
    }

    #[test]
    fn fingerprints_are_deterministic_across_reopening_a_database_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("athena.db");
        let first = {
            let db = Db::open(&path).unwrap();
            expense(&db, date(2026, 10, 10), 500, None, "A");
            expense(&db, date(2026, 10, 10), 600, None, "B");
            close_month(db.conn(), ym(2026, 10), today(), STAMP).unwrap();
            closings_repo::get(db.conn(), ym(2026, 10))
                .unwrap()
                .unwrap()
        };
        let db = Db::open(&path).unwrap();
        let mut cache = FingerprintCache::default();
        assert_eq!(
            status(db.conn(), &mut cache, &first).unwrap(),
            ClosingStatus::Current
        );
        assert_eq!(
            cache.get(db.conn(), ym(2026, 10)).unwrap(),
            first.current_fingerprint
        );
    }

    #[test]
    fn statuses_reuse_adjacent_months_and_mark_failures_unavailable() {
        let db = Db::open_in_memory().unwrap();
        for month in 1..=3 {
            expense(&db, date(2026, month, 5), 100, None, "x");
            close_month(db.conn(), ym(2026, month), today(), STAMP).unwrap();
        }
        let saved = all(db.conn()).unwrap();
        let months = [ym(2026, 1), ym(2026, 2), ym(2026, 3)];
        let mut cache = FingerprintCache::default();
        let found = statuses(db.conn(), &mut cache, &saved, &months);
        assert!(found.values().all(|s| *s == ClosingStatus::Current));
        // Three months plus the predecessor of the first: four fingerprints, not six.
        assert_eq!(cache.len(), 4);
        // A failing read never claims freshness.
        db.conn()
            .execute("DROP VIEW live_transactions", [])
            .unwrap();
        let mut fresh = FingerprintCache::default();
        let failed = statuses(db.conn(), &mut fresh, &saved, &months);
        assert!(failed.values().all(|s| *s == ClosingStatus::Unavailable));
    }
}
