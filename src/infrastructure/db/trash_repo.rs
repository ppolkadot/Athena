//! Trash repository: querying, restoring, and purging soft-deleted transactions.

use rusqlite::{Connection, OptionalExtension};

use super::transactions_repo::{COLUMNS, from_row, load_allocations};
use crate::domain::transaction::Transaction;
use crate::shared::error::{AppError, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct TrashItem {
    pub tx: Transaction,
    pub deleted_at: String,
}

/// Number of soft-deleted transactions currently in Trash.
///
/// # Errors
/// Database errors.
pub fn count(conn: &Connection) -> Result<usize> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM transactions WHERE deleted_at IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// All soft-deleted transactions, newest `deleted_at` first (then `id DESC`).
///
/// # Errors
/// Database errors.
pub fn list(conn: &Connection) -> Result<Vec<TrashItem>> {
    let sql = format!(
        "SELECT {COLUMNS}, t.deleted_at FROM transactions t
         WHERE t.deleted_at IS NOT NULL
         ORDER BY t.deleted_at DESC, t.id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], |r| Ok((from_row(r)?, r.get::<_, String>(9)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let (mut txs, stamps): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
    load_allocations(conn, &mut txs)?;
    Ok(txs
        .into_iter()
        .zip(stamps)
        .map(|(tx, deleted_at)| TrashItem { tx, deleted_at })
        .collect())
}

/// Fetch one deleted transaction by id.
///
/// # Errors
/// [`AppError::NotFound`] when no deleted transaction matches `id`.
pub fn get_deleted(conn: &Connection, id: i64) -> Result<TrashItem> {
    let sql = format!(
        "SELECT {COLUMNS}, t.deleted_at FROM transactions t WHERE t.id = ?1 AND t.deleted_at IS NOT NULL"
    );
    let found = conn
        .query_row(&sql, [id], |r| Ok((from_row(r)?, r.get::<_, String>(9)?)))
        .optional()?;
    let (mut tx, deleted_at) = found.ok_or(AppError::NotFound("transaction"))?;
    load_allocations(conn, std::slice::from_mut(&mut tx))?;
    Ok(TrashItem { tx, deleted_at })
}

/// Restore one deleted transaction (`deleted_at = NULL`). Allows archived people/categories.
///
/// # Errors
/// [`AppError::NotFound`] when no deleted transaction matches `id`.
pub fn restore(conn: &Connection, id: i64) -> Result<()> {
    let updated = conn.execute(
        "UPDATE transactions SET deleted_at = NULL WHERE id = ?1 AND deleted_at IS NOT NULL",
        [id],
    )?;
    if updated == 0 {
        return Err(AppError::NotFound("transaction"));
    }
    Ok(())
}

/// Hard-delete one already-deleted transaction. Allocations cascade.
///
/// # Errors
/// [`AppError::NotFound`] when the transaction does not exist or is still live.
pub fn purge(conn: &Connection, id: i64) -> Result<()> {
    let removed = conn.execute(
        "DELETE FROM transactions WHERE id = ?1 AND deleted_at IS NOT NULL",
        [id],
    )?;
    if removed == 0 {
        return Err(AppError::NotFound("transaction"));
    }
    Ok(())
}

/// Newest `deleted_at` timestamp currently in Trash, if any.
///
/// # Errors
/// Database errors.
pub fn max_deleted_at(conn: &Connection) -> Result<Option<String>> {
    let max: Option<String> = conn.query_row(
        "SELECT MAX(deleted_at) FROM transactions WHERE deleted_at IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    Ok(max)
}

/// Hard-delete trashed rows whose `deleted_at` is strictly older than `cutoff_utc` (UTC RFC 3339).
///
/// # Errors
/// Database errors.
pub fn purge_older_than(conn: &Connection, cutoff_utc: &str) -> Result<usize> {
    let removed = conn.execute(
        "DELETE FROM transactions WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
        [cutoff_utc],
    )?;
    Ok(removed)
}

/// Hard-delete all deleted transactions. Returns how many rows were removed.
///
/// # Errors
/// Database errors.
pub fn empty(conn: &Connection) -> Result<usize> {
    let removed = conn.execute("DELETE FROM transactions WHERE deleted_at IS NOT NULL", [])?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::domain::transaction::Allocation;
    use crate::infrastructure::db::{Db, people_repo, transactions_repo};
    use crate::shared::test_fixtures::{draft_with, seed_people};

    #[test]
    fn purge_requires_a_deleted_row() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let id = transactions_repo::create(
            db.conn(),
            &draft_with(vec![Allocation {
                person_id: owner,
                amount: Money::from_cents(100),
            }]),
        )
        .unwrap();

        // Live rows cannot be purged directly.
        assert!(matches!(purge(db.conn(), id), Err(AppError::NotFound(_))));
        assert!(transactions_repo::get(db.conn(), id).is_ok());

        // Soft-delete, verify in trash, then purge cascades allocations.
        transactions_repo::delete(db.conn(), id).unwrap();
        assert_eq!(count(db.conn()).unwrap(), 1);
        assert!(get_deleted(db.conn(), id).is_ok());

        purge(db.conn(), id).unwrap();
        assert_eq!(count(db.conn()).unwrap(), 0);
        let allocs: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM allocations WHERE transaction_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(allocs, 0);
        assert!(matches!(purge(db.conn(), id), Err(AppError::NotFound(_))));
    }

    #[test]
    fn empty_trash_purges_only_deleted() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let live_id = transactions_repo::create(db.conn(), &draft_with(vec![])).unwrap();
        let deleted_id = transactions_repo::create(
            db.conn(),
            &draft_with(vec![Allocation {
                person_id: partner,
                amount: Money::from_cents(100),
            }]),
        )
        .unwrap();
        let third_deleted = transactions_repo::create(db.conn(), &draft_with(vec![])).unwrap();
        transactions_repo::delete(db.conn(), deleted_id).unwrap();
        transactions_repo::delete(db.conn(), third_deleted).unwrap();

        // Archiving the referenced person does not prevent restoring from trash.
        people_repo::set_archived(db.conn(), partner, true).unwrap();
        restore(db.conn(), deleted_id).unwrap();
        assert!(transactions_repo::get(db.conn(), deleted_id).is_ok());
        assert_eq!(count(db.conn()).unwrap(), 1);

        // Emptying removes only the remaining deleted row and keeps all live rows.
        let removed = empty(db.conn()).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(count(db.conn()).unwrap(), 0);
        assert!(transactions_repo::get(db.conn(), live_id).is_ok());
        assert!(transactions_repo::get(db.conn(), deleted_id).is_ok());
        assert!(matches!(
            get_deleted(db.conn(), third_deleted),
            Err(AppError::NotFound(_))
        ));
        let _ = owner;
    }
}
