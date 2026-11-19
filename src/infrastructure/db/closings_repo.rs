//! Saved monthly closings and their comparison fingerprints (contract §14.3). Ledger rows are read
//! through `live_transactions` only; a closing never writes the ledger.

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::domain::closing::{
    ClosingError, ClosingExpense, ClosingSnapshot, FingerprintRow, fingerprint,
};
use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::shared::error::{AppError, Result};

const COLUMNS: &str = "month, closed_at, income_cents, expenses_cents, refunds_cents, \
    result_cents, entry_count, previous_spending_cents, previous_count, top_category_id, \
    top_category_net_cents, largest_expense_id, largest_expense_title, largest_expense_cents, \
    fingerprint_version, current_fingerprint, previous_fingerprint";

/// Parse a canonical `YYYY-MM` key.
#[must_use]
pub fn parse_month(key: &str) -> Option<YearMonth> {
    let (year, month) = key.split_once('-')?;
    if year.len() != 4 || month.len() != 2 {
        return None;
    }
    let ym = YearMonth::new(year.parse().ok()?, month.parse().ok()?)?;
    (ym.key() == key).then_some(ym)
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<ClosingSnapshot> {
    let key: String = row.get(0)?;
    let month = parse_month(&key).ok_or_else(|| super::bad_code(0, "closing month", &key))?;
    let top_id: Option<i64> = row.get(9)?;
    let top_net: Option<i64> = row.get(10)?;
    let expense_title: Option<String> = row.get(12)?;
    let expense_cents: Option<i64> = row.get(13)?;
    let version: i64 = row.get(14)?;
    Ok(ClosingSnapshot {
        month,
        closed_at: row.get(1)?,
        income: Money::from_cents(row.get(2)?),
        expenses: Money::from_cents(row.get(3)?),
        refunds: Money::from_cents(row.get(4)?),
        result: Money::from_cents(row.get(5)?),
        count: row.get(6)?,
        previous_spending: Money::from_cents(row.get(7)?),
        previous_count: row.get(8)?,
        top_category: top_net.map(|net| (top_id, Money::from_cents(net))),
        largest_expense: match (expense_title, expense_cents) {
            (Some(title), Some(cents)) => Some(ClosingExpense {
                id: row.get(11)?,
                title,
                amount: Money::from_cents(cents),
            }),
            _ => None,
        },
        fingerprint_version: u32::try_from(version).unwrap_or(0),
        current_fingerprint: row.get(15)?,
        previous_fingerprint: row.get(16)?,
    })
}

/// Every saved closing in month order.
///
/// # Errors
/// Database errors.
pub fn list(conn: &Connection) -> Result<Vec<ClosingSnapshot>> {
    let sql = format!("SELECT {COLUMNS} FROM closings ORDER BY month");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One month's closing, if saved.
///
/// # Errors
/// Database errors.
pub fn get(conn: &Connection, month: YearMonth) -> Result<Option<ClosingSnapshot>> {
    let sql = format!("SELECT {COLUMNS} FROM closings WHERE month = ?1");
    Ok(conn.query_row(&sql, [month.key()], from_row).optional()?)
}

/// Number of saved closings.
///
/// # Errors
/// Database errors.
pub fn count(conn: &Connection) -> Result<usize> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM closings", [], |r| r.get(0))?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// Whether `id` is a live (not trashed, not purged) transaction.
///
/// # Errors
/// Database errors.
pub fn is_live_transaction(conn: &Connection, id: i64) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row("SELECT 1 FROM live_transactions WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(found.is_some())
}

/// Every live financial row dated in `month`, for the comparison record. Settlements are excluded;
/// descriptions, allocations and display labels are not part of it.
///
/// # Errors
/// Database errors.
pub fn fingerprint_rows(conn: &Connection, month: YearMonth) -> Result<Vec<FingerprintRow>> {
    let (start, end) = month.bounds();
    let mut stmt = conn.prepare(
        "SELECT id, date, kind, amount_cents, category_id, title, created_at, updated_at
         FROM live_transactions
         WHERE date >= ?1 AND date < ?2 AND kind <> 'settlement'
         ORDER BY date, id",
    )?;
    let rows = stmt
        .query_map(
            [
                start.format("%Y-%m-%d").to_string(),
                end.format("%Y-%m-%d").to_string(),
            ],
            |r| {
                Ok(FingerprintRow {
                    id: r.get(0)?,
                    date: r.get(1)?,
                    kind: r.get(2)?,
                    amount_cents: r.get(3)?,
                    category_id: r.get(4)?,
                    title: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_at: r.get(7)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The version-1 fingerprint of `month`'s live financial rows.
///
/// # Errors
/// Database errors.
pub fn month_fingerprint(conn: &Connection, month: YearMonth) -> Result<String> {
    Ok(fingerprint(month, &fingerprint_rows(conn, month)?))
}

fn insert_row(conn: &Connection, s: &ClosingSnapshot) -> Result<()> {
    conn.execute(
        "INSERT INTO closings (month, closed_at, income_cents, expenses_cents, refunds_cents,
            result_cents, entry_count, previous_spending_cents, previous_count, top_category_id,
            top_category_net_cents, largest_expense_id, largest_expense_title,
            largest_expense_cents, fingerprint_version, current_fingerprint, previous_fingerprint)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            s.month.key(),
            s.closed_at,
            s.income.cents(),
            s.expenses.cents(),
            s.refunds.cents(),
            s.result.cents(),
            s.count,
            s.previous_spending.cents(),
            s.previous_count,
            s.top_category.and_then(|(id, _)| id),
            s.top_category.map(|(_, net)| net.cents()),
            s.largest_expense.as_ref().and_then(|e| e.id),
            s.largest_expense.as_ref().map(|e| e.title.as_str()),
            s.largest_expense.as_ref().map(|e| e.amount.cents()),
            i64::from(s.fingerprint_version),
            s.current_fingerprint,
            s.previous_fingerprint,
        ],
    )?;
    Ok(())
}

fn invalid(error: ClosingError) -> AppError {
    AppError::ClosingInvalid(error)
}

/// Save a new closing; the month must not already have one. Runs inside the caller's transaction.
///
/// # Errors
/// [`AppError::ClosingInvalid`] when the month is closed already or the snapshot is inconsistent.
pub fn insert(conn: &Connection, snapshot: &ClosingSnapshot) -> Result<()> {
    snapshot.validate().map_err(invalid)?;
    if get(conn, snapshot.month)?.is_some() {
        return Err(invalid(ClosingError::AlreadyClosed));
    }
    insert_row(conn, snapshot)
}

/// Replace an existing closing completely. The delete and the insert happen together: any failure
/// leaves the old closing as it was.
///
/// # Errors
/// [`AppError::ClosingInvalid`] when the month has no closing or the snapshot is inconsistent;
/// database errors (including a refused reference) roll the replacement back.
pub fn replace(conn: &Connection, snapshot: &ClosingSnapshot) -> Result<()> {
    snapshot.validate().map_err(invalid)?;
    // A savepoint nests inside the capture transaction and stands alone otherwise.
    conn.execute_batch("SAVEPOINT replace_closing")?;
    let result = (|| {
        let deleted = conn.execute(
            "DELETE FROM closings WHERE month = ?1",
            [snapshot.month.key()],
        )?;
        if deleted == 0 {
            return Err(invalid(ClosingError::NotClosed));
        }
        insert_row(conn, snapshot)
    })();
    match result {
        Ok(()) => conn.execute_batch("RELEASE replace_closing")?,
        Err(err) => {
            conn.execute_batch("ROLLBACK TO replace_closing; RELEASE replace_closing")?;
            return Err(err);
        }
    }
    Ok(())
}
