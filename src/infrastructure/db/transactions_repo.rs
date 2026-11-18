//! Transactions and their allocations. `save_with_allocations` is the only function that writes
//! either table.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};

use super::{bad_code, categories_repo, now_rfc3339, people_repo};
use crate::domain::money::Money;
use crate::domain::text::fold;
use crate::domain::transaction::{
    Allocation, SettlementIssue, SplitMode, Transaction, TransactionType, TxDraft, TxFilter,
};
use crate::shared::error::{AppError, Result};

pub(crate) const COLUMNS: &str = "t.id, t.title, t.amount_cents, t.date, t.kind, t.category_id, t.payer_id, t.description, t.split_mode, t.created_at, t.updated_at";

pub(crate) fn date_text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

pub(crate) fn from_row(row: &Row<'_>) -> rusqlite::Result<Transaction> {
    let date: String = row.get(3)?;
    let kind: String = row.get(4)?;
    let split: Option<String> = row.get(8)?;
    Ok(Transaction {
        id: row.get(0)?,
        title: row.get(1)?,
        amount: Money::from_cents(row.get(2)?),
        date: NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|_| bad_code(3, "date", &date))?,
        kind: TransactionType::from_code(&kind)
            .ok_or_else(|| bad_code(4, "transaction kind", &kind))?,
        category_id: row.get(5)?,
        payer_id: row.get(6)?,
        description: row.get(7)?,
        split_mode: match split {
            None => None,
            Some(code) => {
                Some(SplitMode::from_code(&code).ok_or_else(|| bad_code(8, "split mode", &code))?)
            }
        },
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        allocations: Vec::new(),
    })
}

/// Attach allocations with one query per chunk of ids (not one per row).
pub(crate) fn load_allocations_map(
    conn: &Connection,
    tx_ids: &[i64],
) -> Result<HashMap<i64, Vec<Allocation>>> {
    let mut by_tx: HashMap<i64, Vec<Allocation>> = HashMap::new();
    for chunk in tx_ids.chunks(500) {
        let marks = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT transaction_id, person_id, amount_cents FROM allocations
             WHERE transaction_id IN ({marks}) ORDER BY transaction_id, id"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter().copied()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Allocation {
                    person_id: r.get(1)?,
                    amount: Money::from_cents(r.get(2)?),
                },
            ))
        })?;
        for row in rows {
            let (tx_id, allocation) = row?;
            by_tx.entry(tx_id).or_default().push(allocation);
        }
    }
    Ok(by_tx)
}

pub(crate) fn load_allocations(conn: &Connection, transactions: &mut [Transaction]) -> Result<()> {
    let ids: Vec<i64> = transactions.iter().map(|t| t.id).collect();
    let mut by_tx = load_allocations_map(conn, &ids)?;
    for tx in transactions {
        tx.allocations = by_tx.remove(&tx.id).unwrap_or_default();
    }
    Ok(())
}

/// # Errors
/// [`AppError::NotFound`] when no such live transaction exists.
pub fn get(conn: &Connection, id: i64) -> Result<Transaction> {
    let sql = format!("SELECT {COLUMNS} FROM live_transactions t WHERE t.id = ?1");
    let found = conn.query_row(&sql, [id], from_row).optional()?;
    let mut tx = found.ok_or(AppError::NotFound("transaction"))?;
    load_allocations(conn, std::slice::from_mut(&mut tx))?;
    Ok(tx)
}

fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Newest first (`date DESC, id DESC`).
///
/// # Errors
/// Database errors.
pub fn list(conn: &Connection, filter: &TxFilter) -> Result<Vec<Transaction>> {
    let mut sql = format!("SELECT {COLUMNS} FROM live_transactions t WHERE 1 = 1");
    let mut args: Vec<Value> = Vec::new();
    if let Some(month) = filter.month {
        let (start, end) = month.bounds();
        sql.push_str(" AND t.date >= ? AND t.date < ?");
        args.push(Value::Text(date_text(start)));
        args.push(Value::Text(date_text(end)));
    }
    let needle = fold(filter.text.trim());
    if !needle.is_empty() {
        let pattern = format!("%{}%", escape_like(&needle));
        sql.push_str(
            " AND (fold(t.title) LIKE ? ESCAPE '\\' OR fold(COALESCE(t.description, '')) LIKE ? ESCAPE '\\')",
        );
        args.push(Value::Text(pattern.clone()));
        args.push(Value::Text(pattern));
    }
    if let Some(category) = filter.category_id {
        sql.push_str(" AND t.category_id = ?");
        args.push(Value::Integer(category));
    }
    if let Some(person) = filter.person_id {
        sql.push_str(
            " AND (EXISTS (SELECT 1 FROM allocations a WHERE a.transaction_id = t.id AND a.person_id = ?)
             OR (t.kind = 'settlement' AND t.payer_id = ?))",
        );
        args.push(Value::Integer(person));
        args.push(Value::Integer(person));
    }
    if filter.shared_only {
        sql.push_str(
            " AND t.kind <> 'settlement' AND (SELECT COUNT(*) FROM allocations a WHERE a.transaction_id = t.id) >= 2",
        );
    }
    if let Some(kind) = filter.kind {
        sql.push_str(" AND t.kind = ?");
        args.push(Value::Text(kind.code().to_string()));
    }
    if let Some(until) = filter.until {
        sql.push_str(" AND t.date <= ?");
        args.push(Value::Text(date_text(until)));
    }
    if let Some(issue) = filter.issue {
        sql.push_str(match issue {
            SettlementIssue::Unassigned => {
                " AND t.kind IN ('expense', 'refund')
                  AND NOT EXISTS (SELECT 1 FROM allocations a WHERE a.transaction_id = t.id)"
            }
            SettlementIssue::UnknownPayer => {
                " AND t.kind IN ('expense', 'refund')
                  AND t.payer_id IS NULL
                  AND EXISTS (SELECT 1 FROM allocations a WHERE a.transaction_id = t.id)"
            }
        });
    }
    sql.push_str(" ORDER BY t.date DESC, t.id DESC");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt
        .query_map(params_from_iter(args), from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    load_allocations(conn, &mut rows)?;
    Ok(rows)
}

/// Count all live (non-trashed) transactions.
///
/// # Errors
/// Database errors.
pub fn count_live(conn: &Connection) -> Result<usize> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM live_transactions", [], |r| r.get(0))?;
    Ok(usize::try_from(n).unwrap_or(0))
}

pub(crate) fn category_has_incompatible_transactions(
    conn: &Connection,
    category_id: i64,
    applies_to: crate::domain::category::CategoryScope,
) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT DISTINCT kind FROM transactions WHERE category_id = ?1")?;
    let mut rows = stmt.query([category_id])?;
    while let Some(row) = rows.next()? {
        let code: String = row.get(0)?;
        if let Some(kind) = TransactionType::from_code(&code)
            && !applies_to.allows(kind)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Validation(message.into())
}

/// New references must point at active people and an active category whose scope fits the kind.
/// Rows being edited may keep what they already reference, so history stays editable.
fn check_references(
    conn: &Connection,
    existing: Option<&Transaction>,
    draft: &TxDraft,
) -> Result<()> {
    let people = people_repo::list(conn, true)?;
    let known: HashMap<i64, bool> = people.iter().map(|p| (p.id, p.archived)).collect();
    let kept: HashSet<i64> = existing
        .map(|t| {
            t.allocations
                .iter()
                .map(|a| a.person_id)
                .chain(t.payer_id)
                .collect()
        })
        .unwrap_or_default();
    for person_id in draft
        .allocations
        .iter()
        .map(|a| a.person_id)
        .chain(draft.payer_id)
    {
        match known.get(&person_id) {
            None => return Err(invalid("that person does not exist")),
            Some(true) if !kept.contains(&person_id) => {
                return Err(invalid("that person is archived"));
            }
            Some(_) => {}
        }
    }
    if let Some(category_id) = draft.category_id {
        let category = categories_repo::get(conn, category_id)?
            .ok_or_else(|| invalid("that category does not exist"))?;
        let unchanged =
            existing.is_some_and(|t| t.category_id == Some(category_id) && t.kind == draft.kind);
        if !unchanged {
            if category.archived {
                return Err(invalid("that category is archived"));
            }
            if !category.applies_to.allows(draft.kind) {
                return Err(invalid(
                    "that category cannot be used for this kind of entry",
                ));
            }
        }
    }
    Ok(())
}

/// Insert (`id = None`) or update a transaction with its allocations in one SQL transaction.
/// Returns the transaction id.
///
/// # Errors
/// [`AppError::Validation`] for an invalid draft or stale references; [`AppError::NotFound`] when
/// updating a missing row. Nothing is written on error.
pub fn save_with_allocations(conn: &Connection, id: Option<i64>, draft: &TxDraft) -> Result<i64> {
    save_with_hook(conn, id, draft, |_, _| Ok(()))
}

/// Insert an entry and record the template it came from in one SQL transaction, with one
/// timestamp for the row's `created_at`/`updated_at` and the template's `last_used_at`. A failed
/// usage update (for example a deleted template) rolls the entry back.
///
/// # Errors
/// As [`save_with_allocations`], plus [`AppError::NotFound`] for a missing template.
pub fn create_from_template(conn: &Connection, draft: &TxDraft, template_id: i64) -> Result<i64> {
    save_with_hook(conn, None, draft, |tx, now| {
        super::templates_repo::touch_last_used(tx, template_id, now)
    })
}

/// The shared transactional body of every entry save. `after` runs inside the SQL transaction,
/// after the rows are written and before the commit, with the transaction's timestamp.
fn save_with_hook(
    conn: &Connection,
    id: Option<i64>,
    draft: &TxDraft,
    after: impl FnOnce(&Connection, &str) -> Result<()>,
) -> Result<i64> {
    if let Err(errors) = draft.validate() {
        let text: Vec<String> = errors.iter().map(ToString::to_string).collect();
        return Err(invalid(text.join("; ")));
    }
    let existing = id.map(|id| get(conn, id)).transpose()?;
    check_references(conn, existing.as_ref(), draft)?;

    let title = draft.title.trim();
    let description = draft
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let now = now_rfc3339();
    let tx = conn.unchecked_transaction()?;
    let tx_id = if let Some(id) = id {
        tx.execute(
            "UPDATE transactions SET title = ?1, amount_cents = ?2, date = ?3, kind = ?4, category_id = ?5,
                payer_id = ?6, description = ?7, split_mode = ?8, updated_at = ?9 WHERE id = ?10 AND deleted_at IS NULL",
            params![
                title,
                draft.amount.cents(),
                date_text(draft.date),
                draft.kind.code(),
                draft.category_id,
                draft.payer_id,
                description,
                draft.split_mode.map(SplitMode::code),
                now,
                id
            ],
        )?;
        tx.execute("DELETE FROM allocations WHERE transaction_id = ?1", [id])?;
        id
    } else {
        tx.execute(
            "INSERT INTO transactions (title, amount_cents, date, kind, category_id, payer_id, description,
                split_mode, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                title,
                draft.amount.cents(),
                date_text(draft.date),
                draft.kind.code(),
                draft.category_id,
                draft.payer_id,
                description,
                draft.split_mode.map(SplitMode::code),
                now
            ],
        )?;
        tx.last_insert_rowid()
    };
    for allocation in &draft.allocations {
        tx.execute(
            "INSERT INTO allocations (transaction_id, person_id, amount_cents) VALUES (?1, ?2, ?3)",
            params![tx_id, allocation.person_id, allocation.amount.cents()],
        )?;
    }
    after(&tx, &now)?;
    tx.commit()?;
    Ok(tx_id)
}

/// Live entries (any kind) on `date` with exactly `amount_cents`, in id order, whose normalized
/// title (folded, trimmed, whitespace collapsed) equals `title`'s. `exclude` skips the entry being
/// edited. Independent of every register filter; Trash never matches.
///
/// # Errors
/// Database errors.
pub fn find_duplicates(
    conn: &Connection,
    date: NaiveDate,
    amount_cents: i64,
    title: &str,
    exclude: Option<i64>,
) -> Result<Vec<Transaction>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM live_transactions t WHERE t.date = ?1 AND t.amount_cents = ?2 ORDER BY t.id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params![date_text(date), amount_cents], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let wanted = normalize_title(title);
    let mut matches: Vec<Transaction> = rows
        .into_iter()
        .filter(|t| Some(t.id) != exclude && normalize_title(&t.title) == wanted)
        .collect();
    load_allocations(conn, &mut matches)?;
    Ok(matches)
}

/// Fold, trim and collapse internal whitespace: the duplicate-title comparison key.
#[must_use]
pub fn normalize_title(title: &str) -> String {
    crate::domain::template::collapse_whitespace(&fold(title))
}

/// # Errors
/// As [`save_with_allocations`].
pub fn create(conn: &Connection, draft: &TxDraft) -> Result<i64> {
    save_with_allocations(conn, None, draft)
}

/// # Errors
/// As [`save_with_allocations`].
pub fn update(conn: &Connection, id: i64, draft: &TxDraft) -> Result<()> {
    save_with_allocations(conn, Some(id), draft).map(|_| ())
}

/// Soft-delete a live transaction (`deleted_at` = now UTC RFC 3339, leaving `updated_at` alone).
///
/// # Errors
/// [`AppError::NotFound`] when no such live transaction exists.
pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    let deleted_at = now_rfc3339();
    let updated = conn.execute(
        "UPDATE transactions SET deleted_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
        params![deleted_at, id],
    )?;
    if updated == 0 {
        return Err(AppError::NotFound("transaction"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::date::YearMonth;
    use crate::infrastructure::db::{Db, integrity};
    use crate::shared::test_fixtures::{date, draft_with, seed_people, seeded_category};

    fn alloc(person_id: i64, cents: i64) -> Allocation {
        Allocation {
            person_id,
            amount: Money::from_cents(cents),
        }
    }

    fn assert_consistent(db: &Db) {
        assert!(integrity::check(db.conn()).unwrap().is_empty());
    }

    #[test]
    fn creates_and_loads_with_allocations_and_payer() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let mut draft = draft_with(vec![alloc(owner, 12_000), alloc(partner, 8_000)]);
        draft.payer_id = Some(partner);
        let id = create(db.conn(), &draft).unwrap();
        let tx = get(db.conn(), id).unwrap();
        assert_eq!(tx.amount, Money::from_cents(20_000));
        assert_eq!(
            tx.allocations,
            vec![alloc(owner, 12_000), alloc(partner, 8_000)]
        );
        assert_eq!(tx.payer_id, Some(partner));
        assert_eq!(tx.split_mode, Some(SplitMode::Even));
        assert_consistent(&db);
    }

    #[test]
    fn soft_delete_hides_transaction_and_keeps_allocations() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let id = create(db.conn(), &draft_with(vec![alloc(owner, 100)])).unwrap();
        let updated_before: String = db
            .conn()
            .query_row(
                "SELECT updated_at FROM transactions WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        delete(db.conn(), id).unwrap();
        let (deleted_at, updated_after): (Option<String>, String) = db
            .conn()
            .query_row(
                "SELECT deleted_at, updated_at FROM transactions WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(deleted_at.is_some());
        assert_eq!(updated_after, updated_before);
        let n: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM allocations WHERE transaction_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "allocations remain for restore");
        assert!(matches!(delete(db.conn(), id), Err(AppError::NotFound(_))));
        assert!(matches!(get(db.conn(), id), Err(AppError::NotFound(_))));
        assert_consistent(&db);
    }

    #[test]
    fn update_and_delete_of_deleted_row_are_not_found() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let draft = draft_with(vec![alloc(owner, 100)]);
        let id = create(db.conn(), &draft).unwrap();
        delete(db.conn(), id).unwrap();
        let mut edited = draft;
        edited.title = "Edited".into();
        assert!(matches!(
            update(db.conn(), id, &edited),
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(delete(db.conn(), id), Err(AppError::NotFound(_))));
    }

    #[test]
    fn filters_by_month_person_and_text() {
        let db = Db::open_in_memory().unwrap();
        let (_, partner) = seed_people(db.conn());
        let mut market = draft_with(vec![alloc(partner, 100)]);
        market.title = "Market".into();
        create(db.conn(), &market).unwrap();
        let filter = TxFilter {
            month: Some(YearMonth::new(2026, 11).unwrap()),
            text: "market".into(),
            category_id: None,
            person_id: Some(partner),
            shared_only: false,
            kind: None,
            ..TxFilter::default()
        };
        assert_eq!(list(db.conn(), &filter).unwrap().len(), 1);
        let other_month = TxFilter {
            month: Some(YearMonth::new(2026, 10).unwrap()),
            ..filter
        };
        assert!(list(db.conn(), &other_month).unwrap().is_empty());
        assert_consistent(&db);
    }

    #[test]
    fn text_search_ignores_accents_and_case() {
        let db = Db::open_in_memory().unwrap();
        let mut d = draft_with(vec![]);
        d.title = "Pão de queijo".into();
        create(db.conn(), &d).unwrap();
        let f = TxFilter {
            text: "PAO".into(),
            ..TxFilter::default()
        };
        assert_eq!(list(db.conn(), &f).unwrap().len(), 1);
        let miss = TxFilter {
            text: "leite".into(),
            ..TxFilter::default()
        };
        assert!(list(db.conn(), &miss).unwrap().is_empty());
    }

    #[test]
    fn text_search_treats_like_wildcards_literally() {
        let db = Db::open_in_memory().unwrap();
        let mut a = draft_with(vec![]);
        a.title = "Discount 100% off".into();
        create(db.conn(), &a).unwrap();
        let mut b = draft_with(vec![]);
        b.title = "Plain".into();
        create(db.conn(), &b).unwrap();
        let f = TxFilter {
            text: "%".into(),
            ..TxFilter::default()
        };
        assert_eq!(list(db.conn(), &f).unwrap().len(), 1);
        let underscore = TxFilter {
            text: "_".into(),
            ..TxFilter::default()
        };
        assert!(list(db.conn(), &underscore).unwrap().is_empty());
    }

    #[test]
    fn split_mode_round_trips() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let mut d = draft_with(vec![alloc(owner, 60), alloc(partner, 40)]);
        d.split_mode = Some(SplitMode::Custom);
        let id = create(db.conn(), &d).unwrap();
        assert_eq!(
            get(db.conn(), id).unwrap().split_mode,
            Some(SplitMode::Custom)
        );
        assert_consistent(&db);
    }

    #[test]
    fn update_replaces_allocations_and_fields() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let id = create(
            db.conn(),
            &draft_with(vec![alloc(owner, 50), alloc(partner, 50)]),
        )
        .unwrap();
        let mut edited = draft_with(vec![alloc(partner, 300)]);
        edited.title = "Edited".into();
        edited.date = date(2026, 12, 1);
        update(db.conn(), id, &edited).unwrap();
        let tx = get(db.conn(), id).unwrap();
        assert_eq!(
            (tx.title.as_str(), tx.amount, tx.date),
            ("Edited", Money::from_cents(300), date(2026, 12, 1))
        );
        assert_eq!(tx.allocations, vec![alloc(partner, 300)]);
        assert_eq!(tx.split_mode, None);
        let missing = update(db.conn(), id + 99, &edited);
        assert!(matches!(missing, Err(AppError::NotFound(_))));
        assert_consistent(&db);
    }

    #[test]
    fn invalid_draft_writes_nothing() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let mut bad = draft_with(vec![alloc(owner, 100)]);
        bad.amount = Money::from_cents(150);
        assert!(matches!(
            create(db.conn(), &bad),
            Err(AppError::Validation(_))
        ));
        let n: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn new_entries_reject_archived_people_but_history_stays_editable() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let id = create(
            db.conn(),
            &draft_with(vec![alloc(owner, 50), alloc(partner, 50)]),
        )
        .unwrap();
        people_repo::set_archived(db.conn(), partner, true).unwrap();
        // The archived partner may stay on the existing entry while other fields change.
        let mut keep = draft_with(vec![alloc(owner, 50), alloc(partner, 50)]);
        keep.title = "Renamed".into();
        update(db.conn(), id, &keep).unwrap();
        // But a brand new entry cannot use them, as allocation or payer.
        let fresh = draft_with(vec![alloc(partner, 10)]);
        assert!(create(db.conn(), &fresh).is_err());
        let mut payer = draft_with(vec![]);
        payer.payer_id = Some(partner);
        assert!(create(db.conn(), &payer).is_err());
        // And an unknown person id is rejected.
        assert!(create(db.conn(), &draft_with(vec![alloc(9_999, 10)])).is_err());
        assert_consistent(&db);
    }

    #[test]
    fn category_must_exist_be_active_and_fit_the_kind() {
        let db = Db::open_in_memory().unwrap();
        let food = seeded_category(db.conn(), "Food");
        let salary = seeded_category(db.conn(), "Salary");
        let mut ok = draft_with(vec![]);
        ok.category_id = Some(food);
        let id = create(db.conn(), &ok).unwrap();
        let mut wrong_kind = draft_with(vec![]);
        wrong_kind.category_id = Some(salary);
        assert!(create(db.conn(), &wrong_kind).is_err());
        wrong_kind.kind = TransactionType::Income;
        assert!(create(db.conn(), &wrong_kind).is_ok());
        let mut missing = draft_with(vec![]);
        missing.category_id = Some(9_999);
        assert!(create(db.conn(), &missing).is_err());
        // Archiving a category keeps old entries editable under it, but hides it for new ones.
        categories_repo::set_archived(db.conn(), food, true).unwrap();
        ok.title = "Still editable".into();
        update(db.conn(), id, &ok).unwrap();
        assert!(create(db.conn(), &ok).is_err());
        assert_consistent(&db);
    }

    #[test]
    fn list_orders_newest_first_and_applies_every_filter() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let food = seeded_category(db.conn(), "Food");
        let mut first = draft_with(vec![alloc(owner, 100)]);
        first.date = date(2026, 11, 1);
        let a = create(db.conn(), &first).unwrap();
        let mut shared = draft_with(vec![alloc(owner, 50), alloc(partner, 50)]);
        shared.date = date(2026, 11, 20);
        shared.category_id = Some(food);
        let b = create(db.conn(), &shared).unwrap();
        let mut income = draft_with(vec![]);
        income.kind = TransactionType::Income;
        income.date = date(2026, 11, 20);
        let c = create(db.conn(), &income).unwrap();
        let ids = |f: &TxFilter| -> Vec<i64> {
            list(db.conn(), f)
                .unwrap()
                .into_iter()
                .map(|t| t.id)
                .collect()
        };
        assert_eq!(ids(&TxFilter::default()), vec![c, b, a]);
        assert_eq!(
            ids(&TxFilter {
                shared_only: true,
                ..TxFilter::default()
            }),
            vec![b]
        );
        assert_eq!(
            ids(&TxFilter {
                category_id: Some(food),
                ..TxFilter::default()
            }),
            vec![b]
        );
        assert_eq!(
            ids(&TxFilter {
                kind: Some(TransactionType::Income),
                ..TxFilter::default()
            }),
            vec![c]
        );
        assert_eq!(
            ids(&TxFilter {
                person_id: Some(partner),
                ..TxFilter::default()
            }),
            vec![b]
        );
        assert_eq!(
            ids(&TxFilter {
                person_id: Some(owner),
                ..TxFilter::default()
            }),
            vec![b, a]
        );
    }

    #[test]
    fn month_boundaries_split_cleanly() {
        let db = Db::open_in_memory().unwrap();
        let mut last = draft_with(vec![]);
        last.date = date(2026, 12, 31);
        let mut first = draft_with(vec![]);
        first.date = date(2027, 1, 1);
        let dec_id = create(db.conn(), &last).unwrap();
        let jan_id = create(db.conn(), &first).unwrap();
        let in_month = |y, m| -> Vec<i64> {
            let f = TxFilter {
                month: YearMonth::new(y, m),
                ..TxFilter::default()
            };
            list(db.conn(), &f)
                .unwrap()
                .into_iter()
                .map(|t| t.id)
                .collect()
        };
        assert_eq!(in_month(2026, 12), vec![dec_id]);
        assert_eq!(in_month(2027, 1), vec![jan_id]);
    }

    #[test]
    fn blank_description_is_stored_as_null() {
        let db = Db::open_in_memory().unwrap();
        let mut d = draft_with(vec![]);
        d.description = Some("   ".into());
        let id = create(db.conn(), &d).unwrap();
        assert_eq!(get(db.conn(), id).unwrap().description, None);
    }

    #[test]
    fn integrity_check_finds_a_tampered_sum_and_is_clean_otherwise() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let id = create(
            db.conn(),
            &draft_with(vec![alloc(owner, 50), alloc(partner, 50)]),
        )
        .unwrap();
        assert!(integrity::check(db.conn()).unwrap().is_empty());
        db.conn()
            .execute(
                "UPDATE allocations SET amount_cents = 1 WHERE transaction_id = ?1 AND person_id = ?2",
                [id, owner],
            )
            .unwrap();
        assert_eq!(
            integrity::check(db.conn()).unwrap(),
            vec![integrity::IntegrityIssue::AllocationSumMismatch { tx_id: id }]
        );
    }

    #[test]
    fn integrity_check_flags_a_split_mode_without_a_split() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let id = create(db.conn(), &draft_with(vec![alloc(owner, 100)])).unwrap();
        db.conn()
            .execute(
                "UPDATE transactions SET split_mode = 'even' WHERE id = ?1",
                [id],
            )
            .unwrap();
        assert_eq!(
            integrity::check(db.conn()).unwrap(),
            vec![integrity::IntegrityIssue::SplitModeWithoutAllocations { tx_id: id }]
        );
    }

    #[test]
    fn update_replaces_allocations_and_keeps_payer() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let mut draft = draft_with(vec![alloc(owner, 100)]);
        draft.payer_id = Some(owner);
        let id = create(db.conn(), &draft).unwrap();
        let mut edit = draft_with(vec![alloc(partner, 100)]);
        edit.title = "Edited".into();
        edit.payer_id = Some(partner);
        update(db.conn(), id, &edit).unwrap();
        let tx = get(db.conn(), id).unwrap();
        assert_eq!(tx.title, "Edited");
        assert_eq!(tx.allocations[0].person_id, partner);
        assert_eq!(tx.payer_id, Some(partner));
        assert_consistent(&db);
    }

    #[test]
    fn create_from_template_inserts_and_touches_usage_with_one_timestamp() {
        use crate::domain::template::{ResponsibilityMode, TemplateDraft};
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let template_id = crate::infrastructure::db::templates_repo::create(
            db.conn(),
            &TemplateDraft {
                name: "Coffee".into(),
                title: "Coffee".into(),
                amount: Money::from_cents(900),
                kind: TransactionType::Expense,
                category_id: None,
                payer_id: Some(owner),
                description: None,
                responsibility: ResponsibilityMode::None,
                allocations: vec![],
            },
        )
        .unwrap();
        let mut draft = draft_with(vec![]);
        draft.amount = Money::from_cents(900);
        let id = create_from_template(db.conn(), &draft, template_id).unwrap();
        let stored = get(db.conn(), id).unwrap();
        let used = crate::infrastructure::db::templates_repo::get(db.conn(), template_id)
            .unwrap()
            .last_used_at;
        assert_eq!(used.as_deref(), Some(stored.created_at.as_str()));
        assert_eq!(stored.created_at, stored.updated_at);
        assert!(integrity::check(db.conn()).unwrap().is_empty());
    }

    #[test]
    fn a_failed_usage_update_rolls_the_entry_back() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let draft = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(100),
        }]);
        let err = create_from_template(db.conn(), &draft, 999).unwrap_err();
        assert!(matches!(err, AppError::NotFound("template")));
        assert_eq!(count_live(db.conn()).unwrap(), 0, "no entry is left behind");
        let allocations: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM allocations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(allocations, 0);
        // The standalone wrapper is unchanged.
        let id = save_with_allocations(db.conn(), None, &draft).unwrap();
        assert!(get(db.conn(), id).is_ok());
    }

    #[test]
    fn duplicates_match_date_cents_and_normalized_title_across_kinds_and_filters() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let alloc = |cents| Allocation {
            person_id: owner,
            amount: Money::from_cents(cents),
        };
        let mut a = draft_with(vec![alloc(4_590)]);
        a.title = "Café  da   manhã".into();
        a.amount = Money::from_cents(4_590);
        a.date = date(2026, 11, 5);
        let a_id = create(db.conn(), &a).unwrap();
        // Same key but another kind and another day / amount / title do not collide, except kind.
        let mut refund = a.clone();
        refund.kind = TransactionType::Refund;
        let refund_id = create(db.conn(), &refund).unwrap();
        let mut other_day = a.clone();
        other_day.date = date(2026, 11, 6);
        create(db.conn(), &other_day).unwrap();
        let mut other_title = a.clone();
        other_title.title = "Jantar".into();
        create(db.conn(), &other_title).unwrap();

        let found =
            find_duplicates(db.conn(), date(2026, 11, 5), 4_590, " CAFE da manha ", None).unwrap();
        assert_eq!(
            found.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![a_id, refund_id],
            "all live kinds, deterministic id order"
        );
        // An edited row never matches itself.
        let found =
            find_duplicates(db.conn(), date(2026, 11, 5), 4_590, &a.title, Some(a_id)).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, refund_id);
        // Trashed rows do not match; restoring brings them back.
        delete(db.conn(), a_id).unwrap();
        assert_eq!(
            find_duplicates(db.conn(), date(2026, 11, 5), 4_590, &a.title, None)
                .unwrap()
                .len(),
            1
        );
        crate::infrastructure::db::trash_repo::restore(db.conn(), a_id).unwrap();
        assert_eq!(
            find_duplicates(db.conn(), date(2026, 11, 5), 4_590, &a.title, None)
                .unwrap()
                .len(),
            2
        );
        assert!(
            find_duplicates(db.conn(), date(2026, 11, 5), 4_591, &a.title, None)
                .unwrap()
                .is_empty()
        );
    }
}
