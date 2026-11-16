//! SQL aggregates for reports. Ranges are half-open `[start, end)` over ISO dates.
//! Spending is expenses minus refunds; refunds never count as income.

use std::collections::HashMap;

use chrono::NaiveDate;
use rusqlite::{Connection, params};

use crate::domain::date::YearMonth;
use crate::domain::money::Money;
use crate::domain::reports::{CategoryTotal, DayTotal, MonthSummary, PersonTotal};
use crate::shared::error::{AppError, Result};

const SPEND_SIGN: &str =
    "CASE t.kind WHEN 'expense' THEN t.amount_cents WHEN 'refund' THEN -t.amount_cents END";

fn text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn summary_from(income: i64, expenses: i64, refunds: i64, count: i64) -> MonthSummary {
    MonthSummary::new(
        Money::from_cents(income),
        Money::from_cents(expenses),
        Money::from_cents(refunds),
        count,
    )
}

const SUMMARY_SELECT: &str = "SELECT
    COALESCE(SUM(CASE t.kind WHEN 'income' THEN t.amount_cents END), 0),
    COALESCE(SUM(CASE t.kind WHEN 'expense' THEN t.amount_cents END), 0),
    COALESCE(SUM(CASE t.kind WHEN 'refund' THEN t.amount_cents END), 0),
    COUNT(CASE WHEN t.kind <> 'settlement' THEN 1 END)
    FROM transactions t";

/// # Errors
/// Database errors.
pub fn summary_between(
    conn: &Connection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<MonthSummary> {
    let sql = format!("{SUMMARY_SELECT} WHERE t.date >= ?1 AND t.date < ?2");
    let (i, e, r, n): (i64, i64, i64, i64) =
        conn.query_row(&sql, params![text(start), text(end)], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
    Ok(summary_from(i, e, r, n))
}

/// All-time totals, for the tracked balance.
///
/// # Errors
/// Database errors.
pub fn lifetime_summary(conn: &Connection) -> Result<MonthSummary> {
    let (i, e, r, n): (i64, i64, i64, i64) = conn.query_row(SUMMARY_SELECT, [], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    Ok(summary_from(i, e, r, n))
}

/// Per-month summaries for one calendar year. Months with no entries are absent.
///
/// # Errors
/// Database errors; [`AppError::Validation`] for a year chrono cannot represent.
pub fn summaries_for_year(
    conn: &Connection,
    year: i32,
) -> Result<HashMap<YearMonth, MonthSummary>> {
    let start =
        YearMonth::new(year, 1).ok_or_else(|| AppError::Validation("year out of range".into()))?;
    let end = YearMonth::new(year + 1, 1)
        .ok_or_else(|| AppError::Validation("year out of range".into()))?;
    let sql = "SELECT substr(t.date, 6, 2),
        COALESCE(SUM(CASE t.kind WHEN 'income' THEN t.amount_cents END), 0),
        COALESCE(SUM(CASE t.kind WHEN 'expense' THEN t.amount_cents END), 0),
        COALESCE(SUM(CASE t.kind WHEN 'refund' THEN t.amount_cents END), 0),
        COUNT(*)
        FROM transactions t WHERE t.date >= ?1 AND t.date < ?2 AND t.kind <> 'settlement'
        GROUP BY substr(t.date, 6, 2)";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        params![text(start.first_day()), text(end.first_day())],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        },
    )?;
    let mut by_month = HashMap::new();
    for row in rows {
        let (month, i, e, r, n) = row?;
        if let Some(ym) = month
            .parse::<u32>()
            .ok()
            .and_then(|m| YearMonth::new(year, m))
        {
            by_month.insert(ym, summary_from(i, e, r, n));
        }
    }
    Ok(by_month)
}

/// Spending per category (uncategorized is `None`), largest first.
///
/// # Errors
/// Database errors.
pub fn category_totals(conn: &Connection, month: YearMonth) -> Result<Vec<CategoryTotal>> {
    let (start, end) = month.bounds();
    let sql = format!(
        "SELECT t.category_id, SUM({SPEND_SIGN}) AS net FROM transactions t
         WHERE t.date >= ?1 AND t.date < ?2 AND t.kind IN ('expense', 'refund')
         GROUP BY t.category_id ORDER BY net DESC, t.category_id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![text(start), text(end)], |r| {
            Ok(CategoryTotal {
                category_id: r.get(0)?,
                net: Money::from_cents(r.get(1)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Responsibility per person: their allocations of expenses minus refunds. Shared purchases are
/// split across people here, so the sum equals the transaction view.
///
/// # Errors
/// Database errors.
pub fn person_totals(conn: &Connection, month: YearMonth) -> Result<Vec<PersonTotal>> {
    let (start, end) = month.bounds();
    let sql = "SELECT a.person_id,
            SUM(CASE t.kind WHEN 'expense' THEN a.amount_cents WHEN 'refund' THEN -a.amount_cents END) AS net
        FROM allocations a JOIN transactions t ON t.id = a.transaction_id
        WHERE t.date >= ?1 AND t.date < ?2 AND t.kind IN ('expense', 'refund')
        GROUP BY a.person_id ORDER BY net DESC, a.person_id";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params![text(start), text(end)], |r| {
            Ok(PersonTotal {
                person_id: r.get(0)?,
                net: Money::from_cents(r.get(1)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Net spending per day with activity, oldest first.
///
/// # Errors
/// Database errors.
pub fn day_totals(conn: &Connection, month: YearMonth) -> Result<Vec<DayTotal>> {
    let (start, end) = month.bounds();
    let sql = format!(
        "SELECT t.date, SUM({SPEND_SIGN}) FROM transactions t
         WHERE t.date >= ?1 AND t.date < ?2 AND t.kind IN ('expense', 'refund')
         GROUP BY t.date ORDER BY t.date"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![text(start), text(end)], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (date, net) = row?;
        let date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|_| AppError::Validation(format!("bad stored date {date}")))?;
        out.push(DayTotal {
            date,
            net: Money::from_cents(net),
        });
    }
    Ok(out)
}
