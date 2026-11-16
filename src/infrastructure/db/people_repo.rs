//! People: one owner, an optional partner, others. Archive instead of delete.

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{bad_code, now_rfc3339};
use crate::domain::person::{Person, PersonKind};
use crate::domain::text::fold;
use crate::shared::error::{AppError, Result};

const COLUMNS: &str = "id, name, kind, archived, created_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Person> {
    let kind: String = row.get(2)?;
    Ok(Person {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: PersonKind::from_code(&kind).ok_or_else(|| bad_code(2, "person kind", &kind))?,
        archived: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
    })
}

/// Owner first, then by name.
///
/// # Errors
/// Database errors.
pub fn list(conn: &Connection, include_archived: bool) -> Result<Vec<Person>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM people {} ORDER BY (kind = 'owner') DESC, name COLLATE NOCASE, id",
        if include_archived {
            ""
        } else {
            "WHERE archived = 0"
        }
    );
    let mut stmt = conn.prepare(&sql)?;
    let people = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(people)
}

/// # Errors
/// Database errors.
pub fn get(conn: &Connection, id: i64) -> Result<Option<Person>> {
    let sql = format!("SELECT {COLUMNS} FROM people WHERE id = ?1");
    Ok(conn.query_row(&sql, [id], from_row).optional()?)
}

/// The single owner row, if onboarding has created it.
///
/// # Errors
/// Database errors.
pub fn owner(conn: &Connection) -> Result<Option<Person>> {
    let sql = format!("SELECT {COLUMNS} FROM people WHERE kind = 'owner'");
    Ok(conn.query_row(&sql, [], from_row).optional()?)
}

fn check_name(conn: &Connection, name: &str, except_id: Option<i64>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("a name is required".into()));
    }
    let wanted = fold(name);
    for other in list(conn, false)? {
        if Some(other.id) != except_id && fold(&other.name) == wanted {
            return Err(AppError::Validation(format!("{name} already exists")));
        }
    }
    Ok(name.to_string())
}

/// # Errors
/// [`AppError::Validation`] for an empty or duplicate (accent/case-insensitive) active name; the
/// one-owner rule surfaces as a database error.
pub fn insert(conn: &Connection, name: &str, kind: PersonKind) -> Result<i64> {
    let name = check_name(conn, name, None)?;
    conn.execute(
        "INSERT INTO people (name, kind, created_at) VALUES (?1, ?2, ?3)",
        params![name, kind.code(), now_rfc3339()],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Rename a person. Historical transactions show the new name (names resolve through this row).
///
/// # Errors
/// As [`insert`].
pub fn update(conn: &Connection, person: &Person) -> Result<()> {
    let name = check_name(conn, &person.name, Some(person.id))?;
    conn.execute(
        "UPDATE people SET name = ?1, kind = ?2 WHERE id = ?3",
        params![name, person.kind.code(), person.id],
    )?;
    Ok(())
}

/// Archive or restore. The owner cannot be archived (table CHECK).
/// When restoring (`archived == false`), active name uniqueness is enforced.
///
/// # Errors
/// [`AppError::NotFound`] if missing, [`AppError::Validation`] if an active person already uses
/// this name, or database errors.
pub fn set_archived(conn: &Connection, id: i64, archived: bool) -> Result<()> {
    if !archived {
        let person = get(conn, id)?.ok_or(AppError::NotFound("person"))?;
        check_name(conn, &person.name, Some(id))?;
    }
    conn.execute(
        "UPDATE people SET archived = ?1 WHERE id = ?2",
        params![i64::from(archived), id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::Db;

    #[test]
    fn owner_is_unique_and_cannot_be_archived() {
        let db = Db::open_in_memory().unwrap();
        let owner_id = insert(db.conn(), "A3", PersonKind::Owner).unwrap();
        assert!(insert(db.conn(), "Other", PersonKind::Owner).is_err());
        assert!(set_archived(db.conn(), owner_id, true).is_err());
        assert_eq!(owner(db.conn()).unwrap().unwrap().id, owner_id);
    }

    #[test]
    fn list_puts_the_owner_first_and_hides_archived() {
        let db = Db::open_in_memory().unwrap();
        let ana = insert(db.conn(), "Ana", PersonKind::Partner).unwrap();
        let bia = insert(db.conn(), "Bia", PersonKind::Other).unwrap();
        let me = insert(db.conn(), "Zed", PersonKind::Owner).unwrap();
        let names: Vec<_> = list(db.conn(), false)
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["Zed", "Ana", "Bia"]);
        set_archived(db.conn(), bia, true).unwrap();
        assert_eq!(list(db.conn(), false).unwrap().len(), 2);
        assert_eq!(list(db.conn(), true).unwrap().len(), 3);
        assert!(get(db.conn(), bia).unwrap().unwrap().archived);
        assert_eq!(
            get(db.conn(), ana).unwrap().unwrap().kind,
            PersonKind::Partner
        );
        assert!(get(db.conn(), me + 99).unwrap().is_none());
    }

    #[test]
    fn rename_keeps_the_id_and_rejects_duplicates_ignoring_accents() {
        let db = Db::open_in_memory().unwrap();
        let ana = insert(db.conn(), "Ana", PersonKind::Partner).unwrap();
        insert(db.conn(), "João", PersonKind::Other).unwrap();
        let mut person = get(db.conn(), ana).unwrap().unwrap();
        person.name = "Aninha".into();
        update(db.conn(), &person).unwrap();
        assert_eq!(get(db.conn(), ana).unwrap().unwrap().name, "Aninha");
        person.name = "JOAO".into();
        assert!(matches!(
            update(db.conn(), &person),
            Err(AppError::Validation(_))
        ));
        person.name = "Aninha".into();
        update(db.conn(), &person).unwrap();
    }

    #[test]
    fn empty_names_are_rejected_and_names_are_trimmed() {
        let db = Db::open_in_memory().unwrap();
        assert!(insert(db.conn(), "   ", PersonKind::Other).is_err());
        let id = insert(db.conn(), "  Bia ", PersonKind::Other).unwrap();
        assert_eq!(get(db.conn(), id).unwrap().unwrap().name, "Bia");
    }

    #[test]
    fn an_archived_name_can_be_reused() {
        let db = Db::open_in_memory().unwrap();
        let old = insert(db.conn(), "Bia", PersonKind::Other).unwrap();
        set_archived(db.conn(), old, true).unwrap();
        assert!(insert(db.conn(), "Bia", PersonKind::Other).is_ok());
    }

    #[test]
    fn restoring_person_checks_active_name_uniqueness() {
        let db = Db::open_in_memory().unwrap();
        let old = insert(db.conn(), "Bia", PersonKind::Other).unwrap();
        let carlos = insert(db.conn(), "Carlos", PersonKind::Other).unwrap();
        set_archived(db.conn(), old, true).unwrap();
        set_archived(db.conn(), carlos, true).unwrap();

        insert(db.conn(), "Bia", PersonKind::Other).unwrap();
        assert!(matches!(
            set_archived(db.conn(), old, false),
            Err(AppError::Validation(_))
        ));
        assert!(get(db.conn(), old).unwrap().unwrap().archived);

        set_archived(db.conn(), carlos, false).unwrap();
        assert!(!get(db.conn(), carlos).unwrap().unwrap().archived);
    }
}
