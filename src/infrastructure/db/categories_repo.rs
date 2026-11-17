//! Categories. Delete means archive; archived rows stay queryable for history and reports.

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{bad_code, now_rfc3339};
use crate::domain::category::{Category, CategoryScope};
use crate::domain::text::fold;
use crate::domain::transaction::TransactionType;
use crate::shared::error::{AppError, Result};

const COLUMNS: &str = "id, name, color, icon, applies_to, is_default, archived, created_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Category> {
    let scope: String = row.get(4)?;
    Ok(Category {
        id: row.get(0)?,
        name: row.get(1)?,
        color: row.get(2)?,
        icon: row.get(3)?,
        applies_to: CategoryScope::from_code(&scope)
            .ok_or_else(|| bad_code(4, "category scope", &scope))?,
        is_default: row.get::<_, i64>(5)? != 0,
        archived: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
    })
}

/// Seed order (by id). Pass `true` to include archived rows, which labels and reports need.
///
/// # Errors
/// Database errors.
pub fn list(conn: &Connection, include_archived: bool) -> Result<Vec<Category>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM categories {} ORDER BY id",
        if include_archived {
            ""
        } else {
            "WHERE archived = 0"
        }
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Active categories a picker may offer for `kind` (refunds use the expense scope).
///
/// # Errors
/// Database errors.
pub fn list_for_kind(conn: &Connection, kind: TransactionType) -> Result<Vec<Category>> {
    Ok(list(conn, false)?
        .into_iter()
        .filter(|c| c.applies_to.allows(kind))
        .collect())
}

/// # Errors
/// Database errors.
pub fn get(conn: &Connection, id: i64) -> Result<Option<Category>> {
    let sql = format!("SELECT {COLUMNS} FROM categories WHERE id = ?1");
    Ok(conn.query_row(&sql, [id], from_row).optional()?)
}

fn check(conn: &Connection, name: &str, color: &str, except_id: Option<i64>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("a name is required".into()));
    }
    let bytes = color.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(AppError::Validation("color must look like #RRGGBB".into()));
    }
    let wanted = fold(name);
    for other in list(conn, true)? {
        if Some(other.id) != except_id && fold(&other.name) == wanted {
            return Err(AppError::Validation(format!("{name} already exists")));
        }
    }
    Ok(name.to_string())
}

/// # Errors
/// [`AppError::Validation`] for an empty or duplicate (accent/case-insensitive, archived included)
/// name or a malformed color.
pub fn insert(
    conn: &Connection,
    name: &str,
    color: &str,
    icon: Option<&str>,
    applies_to: CategoryScope,
) -> Result<i64> {
    let name = check(conn, name, color, None)?;
    conn.execute(
        "INSERT INTO categories (name, color, icon, applies_to, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![name, color, icon, applies_to.code(), now_rfc3339()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Rename, recolor, change icon or scope. Historical transactions show the new name.
///
/// # Errors
/// As [`insert`].
pub fn update(conn: &Connection, category: &Category) -> Result<()> {
    let name = check(conn, &category.name, &category.color, Some(category.id))?;
    if super::transactions_repo::category_has_incompatible_transactions(
        conn,
        category.id,
        category.applies_to,
    )? {
        return Err(AppError::CategoryScopeInUse);
    }
    conn.execute(
        "UPDATE categories SET name = ?1, color = ?2, icon = ?3, applies_to = ?4 WHERE id = ?5",
        params![
            name,
            category.color,
            category.icon,
            category.applies_to.code(),
            category.id
        ],
    )?;
    Ok(())
}

/// # Errors
/// Database errors.
pub fn set_archived(conn: &Connection, id: i64, archived: bool) -> Result<()> {
    conn.execute(
        "UPDATE categories SET archived = ?1 WHERE id = ?2",
        params![i64::from(archived), id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::Db;

    #[test]
    fn category_names_are_case_insensitive_unique() {
        let db = Db::open_in_memory().unwrap();
        assert!(insert(db.conn(), "food", "#000000", None, CategoryScope::Expense).is_err());
    }

    #[test]
    fn accented_duplicates_are_rejected_too() {
        let db = Db::open_in_memory().unwrap();
        insert(
            db.conn(),
            "Alimentação",
            "#112233",
            None,
            CategoryScope::Expense,
        )
        .unwrap();
        let err = insert(
            db.conn(),
            "ALIMENTACAO",
            "#112233",
            None,
            CategoryScope::Expense,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn archived_category_hidden_from_active_list() {
        let db = Db::open_in_memory().unwrap();
        let id = insert(db.conn(), "Pets", "#123456", None, CategoryScope::Expense).unwrap();
        set_archived(db.conn(), id, true).unwrap();
        assert!(!list(db.conn(), false).unwrap().iter().any(|c| c.id == id));
        assert!(list(db.conn(), true).unwrap().iter().any(|c| c.id == id));
        // The archived name stays reserved so history never has two rows with one name.
        assert!(insert(db.conn(), "pets", "#123456", None, CategoryScope::Expense).is_err());
    }

    #[test]
    fn pickers_filter_categories_by_kind() {
        let db = Db::open_in_memory().unwrap();
        let names = |kind| -> Vec<String> {
            list_for_kind(db.conn(), kind)
                .unwrap()
                .into_iter()
                .map(|c| c.name)
                .collect()
        };
        let expense = names(TransactionType::Expense);
        let income = names(TransactionType::Income);
        let refund = names(TransactionType::Refund);
        assert!(expense.contains(&"Food".to_string()) && !expense.contains(&"Salary".to_string()));
        assert!(income.contains(&"Salary".to_string()) && income.contains(&"Other".to_string()));
        assert!(!income.contains(&"Food".to_string()));
        assert_eq!(refund, expense);
    }

    #[test]
    fn update_persists_name_color_and_scope() {
        let db = Db::open_in_memory().unwrap();
        let mut cat = list(db.conn(), false)
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Food")
            .unwrap();
        cat.name = "Alimentação".into();
        cat.color = "#00FF00".into();
        cat.applies_to = CategoryScope::Both;
        update(db.conn(), &cat).unwrap();
        let back = get(db.conn(), cat.id).unwrap().unwrap();
        assert_eq!(
            (back.name.as_str(), back.color.as_str(), back.applies_to),
            ("Alimentação", "#00FF00", CategoryScope::Both)
        );
        assert!(back.is_default);
        cat.color = "red".into();
        assert!(update(db.conn(), &cat).is_err());
    }

    #[test]
    fn update_blocks_narrowing_scope_when_used_by_transactions() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = crate::shared::test_fixtures::seed_people(db.conn());
        let mut cat = list(db.conn(), false)
            .unwrap()
            .into_iter()
            .find(|c| c.name == "Food")
            .unwrap();
        let mut draft = crate::shared::test_fixtures::draft_with(vec![]);
        draft.category_id = Some(cat.id);
        draft.payer_id = Some(owner);
        let tx_id =
            crate::infrastructure::db::transactions_repo::create(db.conn(), &draft).unwrap();

        // Widening Expense -> Both succeeds.
        cat.applies_to = CategoryScope::Both;
        update(db.conn(), &cat).unwrap();

        // Narrowing Both -> Income fails while an expense references the category.
        cat.applies_to = CategoryScope::Income;
        assert!(matches!(
            update(db.conn(), &cat),
            Err(AppError::CategoryScopeInUse)
        ));

        // Even when the transaction is in Trash, narrowing must stay blocked.
        crate::infrastructure::db::transactions_repo::delete(db.conn(), tx_id).unwrap();
        assert!(matches!(
            update(db.conn(), &cat),
            Err(AppError::CategoryScopeInUse)
        ));

        // Once purged permanently, narrowing succeeds.
        crate::infrastructure::db::trash_repo::purge(db.conn(), tx_id).unwrap();
        update(db.conn(), &cat).unwrap();
    }
}
