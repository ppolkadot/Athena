//! Cross-row invariants that SQLite cannot express as CHECK constraints.

use rusqlite::Connection;

use crate::shared::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityIssue {
    /// Allocations exist but do not add up to the transaction amount.
    AllocationSumMismatch { tx_id: i64 },
    /// An allocation references a person row that does not exist.
    AllocationMissingPerson { tx_id: i64 },
    /// A split mode is set but the transaction has fewer than two allocations.
    SplitModeWithoutAllocations { tx_id: i64 },
}

fn ids(conn: &Connection, sql: &str) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(rows)
}

/// Find invariant violations. An empty list means the data is consistent.
///
/// # Errors
/// Database errors.
pub fn check(conn: &Connection) -> Result<Vec<IntegrityIssue>> {
    let mut issues = Vec::new();
    for tx_id in ids(
        conn,
        "SELECT t.id FROM transactions t JOIN allocations a ON a.transaction_id = t.id
         GROUP BY t.id HAVING SUM(a.amount_cents) <> t.amount_cents ORDER BY t.id",
    )? {
        issues.push(IntegrityIssue::AllocationSumMismatch { tx_id });
    }
    for tx_id in ids(
        conn,
        "SELECT DISTINCT a.transaction_id FROM allocations a
         LEFT JOIN people p ON p.id = a.person_id WHERE p.id IS NULL ORDER BY 1",
    )? {
        issues.push(IntegrityIssue::AllocationMissingPerson { tx_id });
    }
    for tx_id in ids(
        conn,
        "SELECT t.id FROM transactions t WHERE t.split_mode IS NOT NULL
         AND (SELECT COUNT(*) FROM allocations a WHERE a.transaction_id = t.id) < 2 ORDER BY t.id",
    )? {
        issues.push(IntegrityIssue::SplitModeWithoutAllocations { tx_id });
    }
    Ok(issues)
}
