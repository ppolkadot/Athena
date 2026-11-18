//! Entry templates and their allocations (contract §14.1). Templates are portable: stored or
//! imported references to archived people or categories stay readable. A new or modified template
//! must use active people and a compatible active category.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{bad_code, categories_repo, now_rfc3339, people_repo};
use crate::domain::money::Money;
use crate::domain::template::{
    ResponsibilityMode, Template, TemplateDraft, TemplateError, name_key,
};
use crate::domain::transaction::{Allocation, TransactionType};
use crate::shared::error::{AppError, Result};

const COLUMNS: &str = "id, name, title, amount_cents, kind, category_id, payer_id, responsibility, description, created_at, last_used_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Template> {
    let kind: String = row.get(4)?;
    let mode: String = row.get(7)?;
    Ok(Template {
        id: row.get(0)?,
        draft: TemplateDraft {
            name: row.get(1)?,
            title: row.get(2)?,
            amount: Money::from_cents(row.get(3)?),
            kind: TransactionType::from_code(&kind)
                .ok_or_else(|| bad_code(4, "template kind", &kind))?,
            category_id: row.get(5)?,
            payer_id: row.get(6)?,
            description: row.get(8)?,
            responsibility: ResponsibilityMode::from_code(&mode)
                .ok_or_else(|| bad_code(7, "responsibility", &mode))?,
            allocations: Vec::new(),
        },
        created_at: row.get(9)?,
        last_used_at: row.get(10)?,
    })
}

fn load_allocations(conn: &Connection) -> Result<HashMap<i64, Vec<Allocation>>> {
    let mut stmt = conn.prepare(
        "SELECT template_id, person_id, amount_cents FROM template_allocations
         ORDER BY template_id, person_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            Allocation {
                person_id: r.get(1)?,
                amount: Money::from_cents(r.get(2)?),
            },
        ))
    })?;
    let mut by_template: HashMap<i64, Vec<Allocation>> = HashMap::new();
    for row in rows {
        let (template_id, allocation) = row?;
        by_template.entry(template_id).or_default().push(allocation);
    }
    Ok(by_template)
}

/// Every template in a stable order: folded name, then id.
///
/// # Errors
/// Database errors.
pub fn list(conn: &Connection) -> Result<Vec<Template>> {
    let sql = format!("SELECT {COLUMNS} FROM templates ORDER BY id");
    let mut stmt = conn.prepare(&sql)?;
    let mut templates = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut allocations = load_allocations(conn)?;
    for template in &mut templates {
        template.draft.allocations = allocations.remove(&template.id).unwrap_or_default();
    }
    templates.sort_by(|a, b| {
        name_key(&a.draft.name)
            .cmp(&name_key(&b.draft.name))
            .then(a.id.cmp(&b.id))
    });
    Ok(templates)
}

/// # Errors
/// [`AppError::NotFound`] when no such template exists.
pub fn get(conn: &Connection, id: i64) -> Result<Template> {
    let sql = format!("SELECT {COLUMNS} FROM templates WHERE id = ?1");
    let mut template = conn
        .query_row(&sql, [id], from_row)
        .optional()?
        .ok_or(AppError::NotFound("template"))?;
    let mut stmt = conn.prepare(
        "SELECT person_id, amount_cents FROM template_allocations
         WHERE template_id = ?1 ORDER BY person_id",
    )?;
    template.draft.allocations = stmt
        .query_map([id], |r| {
            Ok(Allocation {
                person_id: r.get(0)?,
                amount: Money::from_cents(r.get(1)?),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(template)
}

fn check_name(
    conn: &Connection,
    except: Option<i64>,
    draft: &TemplateDraft,
    errors: &mut Vec<TemplateError>,
) -> Result<()> {
    let key = name_key(&draft.name);
    if key.is_empty() {
        return Ok(());
    }
    let taken = list(conn)?
        .iter()
        .any(|t| Some(t.id) != except && name_key(&t.draft.name) == key);
    if taken {
        errors.push(TemplateError::NameTaken);
    }
    Ok(())
}

/// New or modified templates must reference active people and a compatible active category.
fn check_references(
    conn: &Connection,
    draft: &TemplateDraft,
    errors: &mut Vec<TemplateError>,
) -> Result<()> {
    let known: HashMap<i64, bool> = people_repo::list(conn, true)?
        .iter()
        .map(|p| (p.id, p.archived))
        .collect();
    for person_id in draft
        .allocations
        .iter()
        .map(|a| a.person_id)
        .chain(draft.payer_id)
    {
        match known.get(&person_id) {
            None => errors.push(TemplateError::PersonMissing(person_id)),
            Some(true) => errors.push(TemplateError::PersonArchived(person_id)),
            Some(false) => {}
        }
    }
    if let Some(category_id) = draft.category_id {
        match categories_repo::get(conn, category_id)? {
            None => errors.push(TemplateError::CategoryMissing),
            Some(c) if c.archived => errors.push(TemplateError::CategoryArchived),
            Some(c) if !c.applies_to.allows(draft.kind) => {
                errors.push(TemplateError::CategoryIncompatible);
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn save(conn: &Connection, id: Option<i64>, draft: &TemplateDraft) -> Result<i64> {
    if let Some(id) = id {
        get(conn, id)?;
    }
    let draft = draft.clone().normalized();
    let mut errors = draft.validate().err().unwrap_or_default();
    check_name(conn, id, &draft, &mut errors)?;
    check_references(conn, &draft, &mut errors)?;
    if !errors.is_empty() {
        errors.dedup();
        return Err(AppError::TemplateInvalid(errors));
    }
    let tx = conn.unchecked_transaction()?;
    let template_id = if let Some(id) = id {
        let changed = tx.execute(
            "UPDATE templates SET name = ?1, title = ?2, amount_cents = ?3, kind = ?4,
                category_id = ?5, payer_id = ?6, responsibility = ?7, description = ?8
             WHERE id = ?9",
            params![
                draft.name,
                draft.title,
                draft.amount.cents(),
                draft.kind.code(),
                draft.category_id,
                draft.payer_id,
                draft.responsibility.code(),
                draft.description,
                id
            ],
        )?;
        if changed == 0 {
            return Err(AppError::NotFound("template"));
        }
        tx.execute(
            "DELETE FROM template_allocations WHERE template_id = ?1",
            [id],
        )?;
        id
    } else {
        tx.execute(
            "INSERT INTO templates (name, title, amount_cents, kind, category_id, payer_id,
                responsibility, description, created_at, last_used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
            params![
                draft.name,
                draft.title,
                draft.amount.cents(),
                draft.kind.code(),
                draft.category_id,
                draft.payer_id,
                draft.responsibility.code(),
                draft.description,
                now_rfc3339()
            ],
        )?;
        tx.last_insert_rowid()
    };
    insert_allocations(&tx, template_id, &draft.allocations)?;
    tx.commit()?;
    Ok(template_id)
}

fn insert_allocations(
    conn: &Connection,
    template_id: i64,
    allocations: &[Allocation],
) -> Result<()> {
    for allocation in allocations {
        conn.execute(
            "INSERT INTO template_allocations (template_id, person_id, amount_cents)
             VALUES (?1, ?2, ?3)",
            params![template_id, allocation.person_id, allocation.amount.cents()],
        )?;
    }
    Ok(())
}

/// Save a new template. Returns its id.
///
/// # Errors
/// [`AppError::TemplateInvalid`] listing every problem; nothing is written on error.
pub fn create(conn: &Connection, draft: &TemplateDraft) -> Result<i64> {
    save(conn, None, draft)
}

/// Replace every editable field of a template.
///
/// # Errors
/// [`AppError::TemplateInvalid`], or [`AppError::NotFound`] for a missing template.
pub fn update(conn: &Connection, id: i64, draft: &TemplateDraft) -> Result<()> {
    save(conn, Some(id), draft).map(|_| ())
}

/// Delete a template; its allocations cascade and nothing else changes.
///
/// # Errors
/// [`AppError::NotFound`] when no such template exists.
pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    let deleted = conn.execute("DELETE FROM templates WHERE id = ?1", [id])?;
    if deleted == 0 {
        return Err(AppError::NotFound("template"));
    }
    Ok(())
}

/// Record that a template was used for a saved entry. Runs inside the caller's transaction.
///
/// # Errors
/// [`AppError::NotFound`] when no such template exists.
pub fn touch_last_used(conn: &Connection, id: i64, stamp: &str) -> Result<()> {
    let updated = conn.execute(
        "UPDATE templates SET last_used_at = ?1 WHERE id = ?2",
        params![stamp, id],
    )?;
    if updated == 0 {
        return Err(AppError::NotFound("template"));
    }
    Ok(())
}

/// Number of stored templates.
///
/// # Errors
/// Database errors.
pub fn count(conn: &Connection) -> Result<usize> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM templates", [], |r| r.get(0))?;
    Ok(usize::try_from(n).unwrap_or(0))
}

/// Insert a template exactly as imported: explicit id and timestamps, no reference-activity
/// rules (history may reference archived people and categories).
pub(crate) fn insert_imported(conn: &Connection, template: &Template) -> Result<()> {
    let d = &template.draft;
    conn.execute(
        "INSERT INTO templates (id, name, title, amount_cents, kind, category_id, payer_id,
            responsibility, description, created_at, last_used_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            template.id,
            d.name.trim(),
            d.title.trim(),
            d.amount.cents(),
            d.kind.code(),
            d.category_id,
            d.payer_id,
            d.responsibility.code(),
            d.description,
            template.created_at,
            template.last_used_at,
        ],
    )?;
    insert_allocations(conn, template.id, &d.allocations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::infrastructure::db::{Db, MIGRATIONS, integrity, run_migrations_with};
    use crate::shared::test_fixtures::{draft_with, seed_people, seeded_category};

    fn share(person_id: i64, cents: i64) -> Allocation {
        Allocation {
            person_id,
            amount: Money::from_cents(cents),
        }
    }

    fn weekly(owner: i64, partner: i64, category: Option<i64>) -> TemplateDraft {
        TemplateDraft {
            name: "Weekly market".into(),
            title: "Market".into(),
            amount: Money::from_cents(10_001),
            kind: TransactionType::Expense,
            category_id: category,
            payer_id: Some(owner),
            description: Some("groceries".into()),
            responsibility: ResponsibilityMode::Even,
            allocations: vec![share(partner, 0), share(owner, 0)],
        }
    }

    #[test]
    fn migration_0004_adds_templates_and_allocations() {
        for level in 1..MIGRATIONS.len() {
            let mut conn = Connection::open_in_memory().unwrap();
            conn.pragma_update(None, "foreign_keys", true).unwrap();
            run_migrations_with(&mut conn, &MIGRATIONS[..level]).unwrap();
            // Seed rows that must survive the 0004 upgrade.
            let c = &conn;
            c.execute(
                "INSERT INTO people (name, kind, archived, created_at) VALUES ('A3', 'owner', 0, 'x')",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO transactions (title, amount_cents, date, kind, created_at, updated_at)
                 VALUES ('Old', 500, '2026-10-01', 'expense', 'x', 'x')",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO allocations (transaction_id, person_id, amount_cents) VALUES (1, 1, 500)",
                [],
            )
            .unwrap();
            let categories_before: i64 = c
                .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
                .unwrap();
            run_migrations_with(&mut conn, MIGRATIONS).unwrap();
            let version: i64 = conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(version, i64::try_from(MIGRATIONS.len()).unwrap());
            for table in ["people", "transactions", "allocations"] {
                let n: i64 = conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap();
                assert_eq!(n, 1, "{table} survives upgrade from v{level}");
            }
            let categories_after: i64 = conn
                .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
                .unwrap();
            assert_eq!(categories_before, categories_after);
            assert_eq!(count(&conn).unwrap(), 0);
            let n: i64 = conn
                .query_row("SELECT COUNT(*) FROM template_allocations", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(n, 0);
            assert!(integrity::check(&conn).unwrap().is_empty());
        }
    }

    #[test]
    fn template_schema_rejects_settlement_and_orphan_allocations() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let c = db.conn();
        let insert = |kind: &str, mode: &str, amount: i64| {
            c.execute(
                "INSERT INTO templates (name, title, amount_cents, kind, responsibility, created_at)
                 VALUES ('n', 't', ?1, ?2, ?3, 'x')",
                params![amount, kind, mode],
            )
        };
        assert!(insert("settlement", "none", 100).is_err());
        assert!(insert("expense", "everyone", 100).is_err());
        assert!(insert("expense", "none", 0).is_err());
        insert("expense", "none", 100).unwrap();
        let tid = c.last_insert_rowid();
        // Orphan allocation (no such template), negative share, duplicate participant.
        assert!(
            c.execute(
                "INSERT INTO template_allocations (template_id, person_id, amount_cents) VALUES (999, ?1, 1)",
                [owner],
            )
            .is_err()
        );
        assert!(
            c.execute(
                "INSERT INTO template_allocations (template_id, person_id, amount_cents) VALUES (?1, ?2, -1)",
                [tid, owner],
            )
            .is_err()
        );
        c.execute(
            "INSERT INTO template_allocations (template_id, person_id, amount_cents) VALUES (?1, ?2, 50)",
            [tid, owner],
        )
        .unwrap();
        assert!(
            c.execute(
                "INSERT INTO template_allocations (template_id, person_id, amount_cents) VALUES (?1, ?2, 50)",
                [tid, owner],
            )
            .is_err()
        );
        // Restrictive FKs: a referenced person or category cannot be deleted.
        assert!(
            c.execute("DELETE FROM people WHERE id = ?1", [owner])
                .is_err()
        );
        c.execute(
            "UPDATE templates SET payer_id = ?1 WHERE id = ?2",
            [partner, tid],
        )
        .unwrap();
        assert!(
            c.execute("DELETE FROM people WHERE id = ?1", [partner])
                .is_err()
        );
        let food = seeded_category(c, "Food");
        c.execute(
            "UPDATE templates SET category_id = ?1 WHERE id = ?2",
            [food, tid],
        )
        .unwrap();
        assert!(
            c.execute("DELETE FROM categories WHERE id = ?1", [food])
                .is_err()
        );
    }

    #[test]
    fn template_crud_normalizes_and_keeps_integrity() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let food = seeded_category(db.conn(), "Food");
        let id = create(db.conn(), &weekly(owner, partner, Some(food))).unwrap();
        let t = get(db.conn(), id).unwrap();
        // Even remainder cent goes to the lowest participant id, regardless of input order.
        assert_eq!(
            t.draft.allocations,
            vec![share(owner, 5_001), share(partner, 5_000)]
        );
        assert_eq!(t.last_used_at, None);
        assert_eq!(list(db.conn()).unwrap().len(), 1);

        let mut edited = t.draft.clone();
        edited.name = "  Weekly shop ".into();
        edited.responsibility = ResponsibilityMode::Custom;
        edited.allocations = vec![share(owner, 10_001), share(partner, 0)];
        update(db.conn(), id, &edited).unwrap();
        let t = get(db.conn(), id).unwrap();
        assert_eq!(t.draft.name, "Weekly shop");
        assert_eq!(
            t.draft.allocations,
            vec![share(owner, 10_001), share(partner, 0)]
        );
        assert!(matches!(
            update(db.conn(), 999, &edited),
            Err(AppError::NotFound("template"))
        ));
        touch_last_used(db.conn(), id, "2026-11-12T10:00:00Z").unwrap();
        assert_eq!(
            get(db.conn(), id).unwrap().last_used_at.as_deref(),
            Some("2026-11-12T10:00:00Z")
        );
        delete(db.conn(), id).unwrap();
        assert!(matches!(delete(db.conn(), id), Err(AppError::NotFound(_))));
        assert!(integrity::check(db.conn()).unwrap().is_empty());
    }

    #[test]
    fn template_names_are_unique_under_the_trimmed_fold() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let a = create(db.conn(), &weekly(owner, partner, None)).unwrap();
        let mut dup = weekly(owner, partner, None);
        dup.name = "  WEEKLY MARKET ".into();
        let err = create(db.conn(), &dup).unwrap_err();
        assert!(matches!(
            err,
            AppError::TemplateInvalid(ref e) if e.contains(&TemplateError::NameTaken)
        ));
        // Accents fold too; a template can keep its own name when edited.
        dup.name = "Weekly Márket".into();
        assert!(create(db.conn(), &dup).is_err());
        dup.name = "Weekly shop".into();
        let b = create(db.conn(), &dup).unwrap();
        let mut again = get(db.conn(), a).unwrap().draft;
        again.title = "Market 2".into();
        update(db.conn(), a, &again).unwrap();
        let mut clash = get(db.conn(), b).unwrap().draft;
        clash.name = "weekly market".into();
        assert!(update(db.conn(), b, &clash).is_err());
        // Listing is alphabetical by folded name, then id.
        let names: Vec<String> = list(db.conn())
            .unwrap()
            .into_iter()
            .map(|t| t.draft.name)
            .collect();
        assert_eq!(names, vec!["Weekly market", "Weekly shop"]);
    }

    #[test]
    fn new_or_modified_templates_need_active_compatible_references() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let salary = seeded_category(db.conn(), "Salary");
        let food = seeded_category(db.conn(), "Food");
        let mut incompatible = weekly(owner, partner, Some(salary));
        incompatible.name = "bad".into();
        let err = create(db.conn(), &incompatible).unwrap_err();
        assert!(matches!(
            err,
            AppError::TemplateInvalid(ref e) if e.contains(&TemplateError::CategoryIncompatible)
        ));
        let mut missing = weekly(owner, partner + 100, None);
        missing.name = "missing".into();
        assert!(create(db.conn(), &missing).is_err());

        // An archived reference blocks new saves but a stored template stays readable.
        let id = create(db.conn(), &weekly(owner, partner, Some(food))).unwrap();
        db.conn()
            .execute("UPDATE people SET archived = 1 WHERE id = ?1", [partner])
            .unwrap();
        db.conn()
            .execute("UPDATE categories SET archived = 1 WHERE id = ?1", [food])
            .unwrap();
        let stored = get(db.conn(), id).unwrap();
        assert_eq!(stored.draft.allocations.len(), 2);
        let err = update(db.conn(), id, &stored.draft).unwrap_err();
        let AppError::TemplateInvalid(errors) = err else {
            panic!("expected TemplateInvalid");
        };
        assert!(errors.contains(&TemplateError::PersonArchived(partner)));
        assert!(errors.contains(&TemplateError::CategoryArchived));
        assert!(integrity::check(db.conn()).unwrap().is_empty());
    }

    #[test]
    fn template_delete_cascades_only_its_allocations() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let keep = create(db.conn(), &weekly(owner, partner, None)).unwrap();
        let mut other = weekly(owner, partner, None);
        other.name = "Other".into();
        let doomed = create(db.conn(), &other).unwrap();
        let tx = draft_with(vec![share(owner, 100)]);
        crate::infrastructure::db::transactions_repo::create(db.conn(), &tx).unwrap();
        delete(db.conn(), doomed).unwrap();
        let rows = |table: &str| -> i64 {
            db.conn()
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(rows("templates"), 1);
        assert_eq!(rows("template_allocations"), 2);
        assert_eq!(rows("allocations"), 1);
        assert_eq!(rows("people"), 2);
        assert_eq!(get(db.conn(), keep).unwrap().draft.allocations.len(), 2);
        assert!(integrity::check(db.conn()).unwrap().is_empty());
    }

    #[test]
    fn integrity_flags_template_shape() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let c = db.conn();
        c.execute(
            "INSERT INTO templates (name, title, amount_cents, kind, responsibility, created_at)
             VALUES ('n', 't', 100, 'expense', 'person', 'x')",
            [],
        )
        .unwrap();
        let tid = c.last_insert_rowid();
        // person with no allocation
        assert_eq!(
            integrity::check(c).unwrap(),
            vec![integrity::IntegrityIssue::TemplateShape { template_id: tid }]
        );
        c.execute(
            "INSERT INTO template_allocations (template_id, person_id, amount_cents) VALUES (?1, ?2, 100)",
            [tid, owner],
        )
        .unwrap();
        assert!(integrity::check(c).unwrap().is_empty());
        // allocations that do not add up
        c.execute("UPDATE template_allocations SET amount_cents = 90", [])
            .unwrap();
        assert_eq!(
            integrity::check(c).unwrap(),
            vec![integrity::IntegrityIssue::TemplateShape { template_id: tid }]
        );
    }
}
