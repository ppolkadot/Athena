//! Settlement input queries over live transactions.

use rusqlite::Connection;

use super::{bad_code, transactions_repo};
use crate::domain::money::Money;
use crate::domain::settlement::SettlementInput;
use crate::domain::transaction::TransactionType;
use crate::shared::error::Result;

/// Load every live non-income row up to `as_of` with its allocations, for cumulative
/// settlement calculation.
///
/// # Errors
/// Database or malformed row errors.
pub fn list_inputs(conn: &Connection, as_of: chrono::NaiveDate) -> Result<Vec<SettlementInput>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.kind, t.amount_cents, t.payer_id
         FROM live_transactions t
         WHERE t.date <= ?1 AND t.kind <> 'income'
         ORDER BY t.date ASC, t.id ASC",
    )?;
    let as_of_str = transactions_repo::date_text(as_of);
    let rows = stmt
        .query_map([as_of_str], |row| {
            let id: i64 = row.get(0)?;
            let kind_code: String = row.get(1)?;
            let kind = TransactionType::from_code(&kind_code)
                .ok_or_else(|| bad_code(1, "transaction kind", &kind_code))?;
            Ok((
                id,
                SettlementInput {
                    kind,
                    amount: Money::from_cents(row.get(2)?),
                    payer_id: row.get(3)?,
                    allocations: Vec::new(),
                },
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let ids: Vec<i64> = rows.iter().map(|(id, _)| *id).collect();
    let mut allocations = transactions_repo::load_allocations_map(conn, &ids)?;
    Ok(rows
        .into_iter()
        .map(|(id, mut input)| {
            input.allocations = allocations.remove(&id).unwrap_or_default();
            input
        })
        .collect())
}
