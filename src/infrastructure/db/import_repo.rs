//! Atomic profile replacement from a validated [`ImportPlan`] (`§10.5`).

use rusqlite::{Connection, Transaction, params};

use super::{integrity, now_rfc3339, transactions_repo::date_text};
use crate::application::services::import::ImportPlan;
use crate::domain::category::Category;
use crate::domain::person::Person;
use crate::domain::settings::Profile;
use crate::domain::transaction::Transaction as DomainTx;
use crate::shared::error::{AppError, ImportError, Result};

/// Replace all user data (profile, people, categories, live and trashed transactions, and
/// allocations) inside a single `SQLite` transaction (`§10.5` step 5).
///
/// Any failure — including `PRAGMA foreign_key_check` or [`integrity::check`] violations — rolls
/// back the transaction and leaves the database untouched.
///
/// # Errors
/// [`AppError::Import`] (`VerificationFailed`) when replacement or verification fails.
pub fn replace(conn: &Connection, plan: &ImportPlan) -> Result<()> {
    replace_with(conn, plan, |_| Ok(())).map_err(|err| match err {
        other @ AppError::Import(_) => other,
        other => AppError::Import(vec![ImportError::VerificationFailed(other.to_string())]),
    })
}

pub(crate) fn replace_with<F>(
    conn: &Connection,
    plan: &ImportPlan,
    after_people_hook: F,
) -> Result<()>
where
    F: FnOnce(&Transaction<'_>) -> Result<()>,
{
    let tx = conn.unchecked_transaction()?;
    clear_tables(&tx)?;
    write_profile(&tx, &plan.profile)?;
    insert_people(&tx, &plan.people)?;
    after_people_hook(&tx)?;
    insert_categories(&tx, &plan.categories)?;
    insert_transactions(&tx, &plan.transactions)?;
    verify_tx(&tx)?;
    tx.commit()?;
    Ok(())
}

fn clear_tables(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "DELETE FROM allocations;
         DELETE FROM transactions;
         DELETE FROM people;
         DELETE FROM categories;",
    )?;
    Ok(())
}

fn write_profile(tx: &Transaction<'_>, profile: &Profile) -> Result<()> {
    let stamp = now_rfc3339();
    let updated = tx.execute(
        "UPDATE profile
         SET start_balance_cents = ?1, theme = ?2, language = ?3, onboarding_complete = ?4, updated_at = ?5
         WHERE id = 1",
        params![
            profile.start_balance.cents(),
            profile.theme.code(),
            profile.language.code(),
            i32::from(profile.onboarding_complete),
            stamp,
        ],
    )?;
    if updated == 0 {
        tx.execute(
            "INSERT INTO profile (id, currency, start_balance_cents, theme, language, onboarding_complete, created_at, updated_at)
             VALUES (1, 'BRL', ?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                profile.start_balance.cents(),
                profile.theme.code(),
                profile.language.code(),
                i32::from(profile.onboarding_complete),
                stamp,
            ],
        )?;
    }
    Ok(())
}

fn insert_people(tx: &Transaction<'_>, people: &[Person]) -> Result<()> {
    let mut stmt = tx.prepare(
        "INSERT INTO people (id, name, kind, archived, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for p in people {
        stmt.execute(params![
            p.id,
            p.name.trim(),
            p.kind.code(),
            i32::from(p.archived),
            p.created_at,
        ])?;
    }
    Ok(())
}

fn insert_categories(tx: &Transaction<'_>, categories: &[Category]) -> Result<()> {
    let mut stmt = tx.prepare(
        "INSERT INTO categories (id, name, applies_to, color, icon, is_default, archived, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for c in categories {
        stmt.execute(params![
            c.id,
            c.name.trim(),
            c.applies_to.code(),
            c.color,
            c.icon.as_deref(),
            i32::from(c.is_default),
            i32::from(c.archived),
            c.created_at,
        ])?;
    }
    Ok(())
}

fn insert_transactions(tx: &Transaction<'_>, transactions: &[DomainTx]) -> Result<()> {
    let mut tx_stmt = tx.prepare(
        "INSERT INTO transactions (
             id, title, amount_cents, date, kind, category_id, payer_id,
             description, split_mode, created_at, updated_at, deleted_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)",
    )?;
    let mut alloc_stmt = tx.prepare(
        "INSERT INTO allocations (transaction_id, person_id, amount_cents)
         VALUES (?1, ?2, ?3)",
    )?;
    for item in transactions {
        tx_stmt.execute(params![
            item.id,
            item.title.trim(),
            item.amount.cents(),
            date_text(item.date),
            item.kind.code(),
            item.category_id,
            item.payer_id,
            item.description.as_deref(),
            item.split_mode
                .map(crate::domain::transaction::SplitMode::code),
            item.created_at,
            item.updated_at,
        ])?;
        for alloc in &item.allocations {
            alloc_stmt.execute(params![item.id, alloc.person_id, alloc.amount.cents()])?;
        }
    }
    Ok(())
}

fn verify_tx(tx: &Transaction<'_>) -> Result<()> {
    let fk_violation: Option<(String, i64, String, i64)> = {
        let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
        let mut rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.next().transpose()?
    };
    if let Some((table, rowid, parent, _)) = fk_violation {
        return Err(AppError::Import(vec![ImportError::VerificationFailed(
            format!("foreign key check failed on {table} row {rowid} -> {parent}"),
        )]));
    }
    let issues = integrity::check(tx)?;
    if !issues.is_empty() {
        return Err(AppError::Import(vec![ImportError::VerificationFailed(
            format!("{} integrity issue(s)", issues.len()),
        )]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::category::CategoryScope;
    use crate::domain::export::ExportScope;
    use crate::domain::money::Money;
    use crate::domain::person::PersonKind;
    use crate::domain::settings::{Language, Theme};
    use crate::domain::transaction::{Allocation, SplitMode, TransactionType};
    use crate::infrastructure::db::{
        Db, categories_repo, people_repo, profile_repo, transactions_repo, trash_repo,
    };
    use crate::shared::test_fixtures::{date, draft_with, seed_month, seed_people};

    fn sample_plan() -> ImportPlan {
        ImportPlan {
            source_name: "incoming.json".into(),
            format_version: 2,
            app_version: "1.1.0".into(),
            exported_at: "2026-11-15T10:00:00Z".into(),
            used_timestamp_fallback: false,
            profile: Profile {
                start_balance: Money::from_cents(42_000),
                theme: Theme::Light,
                language: Language::PtBr,
                onboarding_complete: true,
            },
            widened_categories: Vec::new(),
            people: vec![
                Person {
                    id: 10,
                    name: "Helena".into(),
                    kind: PersonKind::Owner,
                    archived: false,
                    created_at: "2026-10-01T08:00:00Z".into(),
                },
                Person {
                    id: 11,
                    name: "Rafael".into(),
                    kind: PersonKind::Partner,
                    archived: false,
                    created_at: "2026-10-02T09:00:00Z".into(),
                },
            ],
            categories: vec![Category {
                id: 20,
                name: "Coffee".into(),
                color: "#6D4C41".into(),
                icon: None,
                applies_to: CategoryScope::Expense,
                is_default: false,
                archived: false,
                created_at: "2026-10-03T10:00:00Z".into(),
            }],
            transactions: vec![DomainTx {
                id: 30,
                title: "Espresso".into(),
                amount: Money::from_cents(1_500),
                date: date(2026, 11, 14),
                kind: TransactionType::Expense,
                category_id: Some(20),
                payer_id: Some(10),
                description: Some("Morning".into()),
                split_mode: Some(SplitMode::Even),
                created_at: "2026-11-14T07:30:00Z".into(),
                updated_at: "2026-11-14T08:00:00Z".into(),
                allocations: vec![
                    Allocation {
                        person_id: 10,
                        amount: Money::from_cents(750),
                    },
                    Allocation {
                        person_id: 11,
                        amount: Money::from_cents(750),
                    },
                ],
            }],
        }
    }

    #[test]
    fn import_replaces_everything_with_explicit_ids_and_timestamps() {
        let db = Db::open_in_memory().unwrap();
        seed_month(&db);
        let live_before = transactions_repo::list(db.conn(), &ExportScope::All.filter()).unwrap();
        let trashed_id = live_before[0].id;
        transactions_repo::delete(db.conn(), trashed_id).unwrap();
        assert_eq!(trash_repo::count(db.conn()).unwrap(), 1);

        let plan = sample_plan();
        replace(db.conn(), &plan).unwrap();

        assert_eq!(trash_repo::count(db.conn()).unwrap(), 0);
        let profile = profile_repo::load(db.conn()).unwrap().unwrap();
        assert_eq!(profile, plan.profile);

        let mut canonical = ImportPlan::from_db(db.conn(), &profile).unwrap();
        canonical.source_name = plan.source_name.clone();
        canonical.app_version = plan.app_version.clone();
        canonical.exported_at = plan.exported_at.clone();
        assert_eq!(canonical, plan);
    }

    #[test]
    fn late_failure_rolls_back_completely() {
        let db = Db::open_in_memory().unwrap();
        seed_month(&db);
        let original_profile = Profile {
            start_balance: Money::from_cents(9_900),
            theme: Theme::Dark,
            language: Language::En,
            onboarding_complete: true,
        };
        profile_repo::save(db.conn(), &original_profile).unwrap();
        let existing_live = transactions_repo::list(db.conn(), &ExportScope::All.filter()).unwrap();
        let trashed_id = existing_live[0].id;
        transactions_repo::delete(db.conn(), trashed_id).unwrap();

        let before_people = people_repo::list(db.conn(), true).unwrap();
        let before_categories = categories_repo::list(db.conn(), true).unwrap();
        let before_live = transactions_repo::list(db.conn(), &ExportScope::All.filter()).unwrap();
        let before_trash = trash_repo::list(db.conn()).unwrap();

        let plan = sample_plan();
        let err = replace_with(db.conn(), &plan, |_| {
            Err(AppError::Io(std::io::Error::other("injected failure")))
        })
        .unwrap_err();
        assert!(matches!(err, AppError::Io(_)));

        assert_eq!(
            profile_repo::load(db.conn()).unwrap().unwrap(),
            original_profile
        );
        assert_eq!(people_repo::list(db.conn(), true).unwrap(), before_people);
        assert_eq!(
            categories_repo::list(db.conn(), true).unwrap(),
            before_categories
        );
        assert_eq!(
            transactions_repo::list(db.conn(), &ExportScope::All.filter()).unwrap(),
            before_live
        );
        assert_eq!(trash_repo::list(db.conn()).unwrap(), before_trash);
    }

    #[test]
    fn import_rechecks_integrity_inside_the_transaction() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = seed_people(db.conn());
        let mut before_tx = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(500),
        }]);
        before_tx.title = "Keep Me".into();
        transactions_repo::create(db.conn(), &before_tx).unwrap();

        let mut bad_plan = sample_plan();
        // Mismatched allocation sum (750 + 500 != 1500) passes SQLite CHECKs, caught by integrity::check.
        bad_plan.transactions[0].allocations[1].amount = Money::from_cents(500);

        let err = replace(db.conn(), &bad_plan).unwrap_err();
        assert!(matches!(
            err,
            AppError::Import(ref errs)
                if matches!(errs.as_slice(), [ImportError::VerificationFailed(_)])
        ));

        let remaining = transactions_repo::list(db.conn(), &ExportScope::All.filter()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].title, "Keep Me");
    }

    #[test]
    fn imported_ids_do_not_collide_with_future_inserts() {
        let db = Db::open_in_memory().unwrap();
        let plan = sample_plan();
        replace(db.conn(), &plan).unwrap();

        let new_person = people_repo::insert(db.conn(), "Clara", PersonKind::Other).unwrap();
        let new_cat =
            categories_repo::insert(db.conn(), "Snacks", "#FFB74D", None, CategoryScope::Expense)
                .unwrap();
        let mut draft = draft_with(vec![Allocation {
            person_id: new_person,
            amount: Money::from_cents(800),
        }]);
        draft.title = "Cookie".into();
        draft.category_id = Some(new_cat);
        let new_tx = transactions_repo::create(db.conn(), &draft).unwrap();

        assert!(new_person > 11);
        assert!(new_cat > 20);
        assert!(new_tx > 30);
    }
}
