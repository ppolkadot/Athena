//! Goals and their contributions (contract §14.3). Lifecycle, ownership and amount rules run inside
//! the write transaction. Nothing here reads or writes `transactions`: goal money never touches
//! the ledger.

use std::collections::HashMap;

use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{bad_code, now_rfc3339, people_repo};
use crate::domain::goal::{
    Contribution, ContributionDraft, Frequency, Goal, GoalDraft, GoalError, GoalStatus,
    GoalTransition, apply_transition,
};
use crate::domain::money::Money;
use crate::shared::error::{AppError, Result};

const COLUMNS: &str = "id, name, target_cents, target_date, frequency, attribution_id, status, \
                       pre_archive_status, created_at, updated_at";

fn parse_date(column: usize, text: &str) -> rusqlite::Result<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| bad_code(column, "date", text))
}

fn from_row(row: &Row<'_>) -> rusqlite::Result<Goal> {
    let target_date: Option<String> = row.get(3)?;
    let frequency: String = row.get(4)?;
    let status: String = row.get(6)?;
    let pre_archive: Option<String> = row.get(7)?;
    Ok(Goal {
        id: row.get(0)?,
        name: row.get(1)?,
        target: Money::from_cents(row.get(2)?),
        target_date: target_date.map(|d| parse_date(3, &d)).transpose()?,
        frequency: Frequency::from_code(&frequency)
            .ok_or_else(|| bad_code(4, "goal frequency", &frequency))?,
        attribution_id: row.get(5)?,
        status: GoalStatus::from_code(&status)
            .ok_or_else(|| bad_code(6, "goal status", &status))?,
        pre_archive_status: pre_archive
            .map(|code| {
                GoalStatus::from_code(&code).ok_or_else(|| bad_code(7, "goal status", &code))
            })
            .transpose()?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn contribution_from_row(row: &Row<'_>) -> rusqlite::Result<Contribution> {
    let date: String = row.get(3)?;
    Ok(Contribution {
        id: row.get(0)?,
        goal_id: row.get(1)?,
        amount: Money::from_cents(row.get(2)?),
        date: parse_date(3, &date)?,
        note: row.get(4)?,
        created_at: row.get(5)?,
    })
}

const CONTRIBUTION_COLUMNS: &str = "id, goal_id, amount_cents, date, note, created_at";

/// Every goal, by id.
///
/// # Errors
/// Database errors.
pub fn list(conn: &Connection) -> Result<Vec<Goal>> {
    let sql = format!("SELECT {COLUMNS} FROM goals ORDER BY id");
    let mut stmt = conn.prepare(&sql)?;
    let goals = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(goals)
}

/// # Errors
/// [`AppError::NotFound`] when no such goal exists.
pub fn get(conn: &Connection, id: i64) -> Result<Goal> {
    let sql = format!("SELECT {COLUMNS} FROM goals WHERE id = ?1");
    conn.query_row(&sql, [id], from_row)
        .optional()?
        .ok_or(AppError::NotFound("goal"))
}

/// Contributed cents per goal id, summed with checked arithmetic. Goals without contributions are
/// absent.
///
/// # Errors
/// [`GoalError::Overflow`] (as [`AppError::GoalInvalid`]) when a goal's total is unrepresentable.
pub fn contributed_totals(conn: &Connection) -> Result<HashMap<i64, Money>> {
    let mut stmt = conn.prepare("SELECT goal_id, amount_cents FROM goal_contributions")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    let mut totals: HashMap<i64, i64> = HashMap::new();
    for row in rows {
        let (goal_id, cents) = row?;
        let total = totals.entry(goal_id).or_insert(0);
        *total = total
            .checked_add(cents)
            .ok_or_else(|| AppError::GoalInvalid(vec![GoalError::Overflow]))?;
    }
    Ok(totals
        .into_iter()
        .map(|(id, cents)| (id, Money::from_cents(cents)))
        .collect())
}

/// The checked contributed total of one goal.
///
/// # Errors
/// As [`contributed_totals`].
pub fn contributed(conn: &Connection, goal_id: i64) -> Result<Money> {
    let mut stmt =
        conn.prepare("SELECT amount_cents FROM goal_contributions WHERE goal_id = ?1")?;
    let rows = stmt.query_map([goal_id], |r| r.get::<_, i64>(0))?;
    let mut total = 0_i64;
    for cents in rows {
        total = total
            .checked_add(cents?)
            .ok_or_else(|| AppError::GoalInvalid(vec![GoalError::Overflow]))?;
    }
    Ok(Money::from_cents(total))
}

/// A goal's contributions, newest first: `date DESC, id DESC`.
///
/// # Errors
/// Database errors.
pub fn contributions(conn: &Connection, goal_id: i64) -> Result<Vec<Contribution>> {
    let sql = format!(
        "SELECT {CONTRIBUTION_COLUMNS} FROM goal_contributions WHERE goal_id = ?1
         ORDER BY date DESC, id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([goal_id], contribution_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// New or changed attribution must be an active person.
fn check_attribution(
    conn: &Connection,
    attribution_id: Option<i64>,
    unchanged: Option<i64>,
    errors: &mut Vec<GoalError>,
) -> Result<()> {
    let Some(id) = attribution_id else {
        return Ok(());
    };
    if unchanged == Some(id) {
        return Ok(());
    }
    match people_repo::get(conn, id)? {
        None => errors.push(GoalError::AttributionMissing),
        Some(p) if p.archived => errors.push(GoalError::AttributionArchived),
        Some(_) => {}
    }
    Ok(())
}

/// Create a goal in the Active status. Returns its id.
///
/// # Errors
/// [`AppError::GoalInvalid`] listing every problem; nothing is written on error.
pub fn create(conn: &Connection, draft: &GoalDraft) -> Result<i64> {
    let draft = draft.clone().normalized();
    let mut errors = draft.validate().err().unwrap_or_default();
    check_attribution(conn, draft.attribution_id, None, &mut errors)?;
    if !errors.is_empty() {
        return Err(AppError::GoalInvalid(errors));
    }
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO goals (name, target_cents, target_date, frequency, attribution_id, status,
            pre_archive_status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', NULL, ?6, ?6)",
        params![
            draft.name,
            draft.target.cents(),
            draft.target_date.map(|d| d.format("%Y-%m-%d").to_string()),
            draft.frequency.code(),
            draft.attribution_id,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Replace every editable field of an Active or Paused goal.
///
/// # Errors
/// [`AppError::NotFound`], or [`AppError::GoalInvalid`] for validation, a non-editable status or a
/// newly chosen archived person.
pub fn update(conn: &Connection, id: i64, draft: &GoalDraft) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let goal = get(&tx, id)?;
    let draft = draft.clone().normalized();
    let mut errors = draft.validate().err().unwrap_or_default();
    if !goal.status.is_editable() {
        errors.push(GoalError::NotEditable);
    }
    check_attribution(&tx, draft.attribution_id, goal.attribution_id, &mut errors)?;
    if !errors.is_empty() {
        return Err(AppError::GoalInvalid(errors));
    }
    tx.execute(
        "UPDATE goals SET name = ?1, target_cents = ?2, target_date = ?3, frequency = ?4,
            attribution_id = ?5, updated_at = ?6 WHERE id = ?7",
        params![
            draft.name,
            draft.target.cents(),
            draft.target_date.map(|d| d.format("%Y-%m-%d").to_string()),
            draft.frequency.code(),
            draft.attribution_id,
            now_rfc3339(),
            id
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Apply a workflow step, checked against the stored status inside the transaction.
///
/// # Errors
/// [`AppError::NotFound`] or [`AppError::GoalInvalid`] for a step that does not apply.
pub fn change_status(conn: &Connection, id: i64, transition: GoalTransition) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let goal = get(&tx, id)?;
    let (status, pre_archive) = apply_transition(goal.status, goal.pre_archive_status, transition)
        .map_err(|e| AppError::GoalInvalid(vec![e]))?;
    tx.execute(
        "UPDATE goals SET status = ?1, pre_archive_status = ?2, updated_at = ?3 WHERE id = ?4",
        params![
            status.code(),
            pre_archive.map(GoalStatus::code),
            now_rfc3339(),
            id
        ],
    )?;
    tx.commit()?;
    Ok(())
}

/// Add a contribution to an Active or Paused goal. `today` is the app's local date: a contribution
/// may not be dated later. Returns the new contribution id.
///
/// # Errors
/// [`AppError::NotFound`]; [`AppError::GoalInvalid`] for validation, a non-editable goal or an
/// unrepresentable total (the write is rolled back).
pub fn add_contribution(
    conn: &Connection,
    goal_id: i64,
    draft: &ContributionDraft,
    today: NaiveDate,
) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let goal = get(&tx, goal_id)?;
    let draft = draft.clone().normalized();
    let mut errors = draft.validate(today).err().unwrap_or_default();
    if !goal.status.is_editable() {
        errors.push(GoalError::NotEditable);
    }
    if !errors.is_empty() {
        return Err(AppError::GoalInvalid(errors));
    }
    contributed(&tx, goal_id)?
        .cents()
        .checked_add(draft.amount.cents())
        .ok_or_else(|| AppError::GoalInvalid(vec![GoalError::Overflow]))?;
    tx.execute(
        "INSERT INTO goal_contributions (goal_id, amount_cents, date, note, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            goal_id,
            draft.amount.cents(),
            draft.date.format("%Y-%m-%d").to_string(),
            draft.note,
            now_rfc3339()
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.commit()?;
    Ok(id)
}

/// Delete one contribution of an Active or Paused goal. The goal's manual status is untouched.
///
/// # Errors
/// [`AppError::NotFound`] when the goal or the contribution (for this goal) is missing;
/// [`AppError::GoalInvalid`] when the goal is not editable.
pub fn delete_contribution(conn: &Connection, goal_id: i64, contribution_id: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let goal = get(&tx, goal_id)?;
    if !goal.status.is_editable() {
        return Err(AppError::GoalInvalid(vec![GoalError::NotEditable]));
    }
    let deleted = tx.execute(
        "DELETE FROM goal_contributions WHERE id = ?1 AND goal_id = ?2",
        params![contribution_id, goal_id],
    )?;
    if deleted == 0 {
        return Err(AppError::NotFound("contribution"));
    }
    tx.commit()?;
    Ok(())
}

/// Number of stored goals.
///
/// # Errors
/// Database errors.
pub fn count(conn: &Connection) -> Result<usize> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM goals", [], |r| r.get(0))?;
    Ok(usize::try_from(n).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::{Db, integrity};
    use crate::shared::test_fixtures::seed_people;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn today() -> NaiveDate {
        d(2026, 11, 19)
    }

    fn draft(name: &str, target: i64) -> GoalDraft {
        GoalDraft {
            name: name.into(),
            target: Money::from_cents(target),
            target_date: Some(d(2027, 6, 1)),
            frequency: Frequency::Monthly,
            attribution_id: None,
        }
    }

    fn contribution(cents: i64, date: NaiveDate) -> ContributionDraft {
        ContributionDraft {
            amount: Money::from_cents(cents),
            date,
            note: None,
        }
    }

    fn clean(db: &Db) {
        assert_eq!(integrity::check(db.conn()).unwrap(), vec![]);
    }

    #[test]
    fn create_trims_names_and_allows_duplicates() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let a = create(c, &draft("  Trip  ", 1000)).unwrap();
        let b = create(c, &draft("Trip", 2000)).unwrap();
        assert_ne!(a, b);
        let goal = get(c, a).unwrap();
        assert_eq!(goal.name, "Trip");
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.pre_archive_status, None);
        assert!(goal.created_at.ends_with('Z'));
        assert_eq!(list(c).unwrap().len(), 2);
        let bad = create(c, &draft("   ", 0));
        assert!(matches!(
            bad,
            Err(AppError::GoalInvalid(e)) if e == vec![GoalError::NameEmpty, GoalError::TargetNotPositive]
        ));
        clean(&db);
    }

    #[test]
    fn goal_workflow_preserves_pre_archive_state() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        for start in [
            GoalStatus::Active,
            GoalStatus::Paused,
            GoalStatus::Completed,
        ] {
            let id = create(c, &draft("Goal", 1000)).unwrap();
            match start {
                GoalStatus::Paused => change_status(c, id, GoalTransition::Pause).unwrap(),
                GoalStatus::Completed => change_status(c, id, GoalTransition::Complete).unwrap(),
                _ => {}
            }
            change_status(c, id, GoalTransition::Archive).unwrap();
            let archived = get(c, id).unwrap();
            assert_eq!(archived.status, GoalStatus::Archived);
            assert_eq!(archived.pre_archive_status, Some(start));
            // Archived goals reject edits, contributions and every step but Restore.
            assert!(update(c, id, &draft("X", 5)).is_err());
            assert!(add_contribution(c, id, &contribution(1, today()), today()).is_err());
            assert!(change_status(c, id, GoalTransition::Complete).is_err());
            assert!(change_status(c, id, GoalTransition::Pause).is_err());
            change_status(c, id, GoalTransition::Restore).unwrap();
            let restored = get(c, id).unwrap();
            assert_eq!(restored.status, start);
            assert_eq!(restored.pre_archive_status, None);
        }
        // Completed goals are read-only until reopened.
        let id = create(c, &draft("Done", 1000)).unwrap();
        change_status(c, id, GoalTransition::Complete).unwrap();
        assert!(update(c, id, &draft("X", 5)).is_err());
        assert!(add_contribution(c, id, &contribution(1, today()), today()).is_err());
        change_status(c, id, GoalTransition::Reopen).unwrap();
        add_contribution(c, id, &contribution(1, today()), today()).unwrap();
        clean(&db);
    }

    #[test]
    fn paused_goals_accept_contributions_and_completion_below_target_keeps_the_gap() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let id = create(c, &draft("Trip", 1000)).unwrap();
        change_status(c, id, GoalTransition::Pause).unwrap();
        add_contribution(c, id, &contribution(300, today()), today()).unwrap();
        change_status(c, id, GoalTransition::Complete).unwrap();
        assert_eq!(get(c, id).unwrap().status, GoalStatus::Completed);
        assert_eq!(contributed(c, id).unwrap().cents(), 300);
        clean(&db);
    }

    #[test]
    fn contributions_validate_dates_and_order_newest_first() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let id = create(c, &draft("Trip", 1000)).unwrap();
        assert!(matches!(
            add_contribution(c, id, &contribution(10, d(2026, 11, 20)), today()),
            Err(AppError::GoalInvalid(e)) if e == vec![GoalError::DateInFuture]
        ));
        assert!(add_contribution(c, id, &contribution(0, today()), today()).is_err());
        let a = add_contribution(c, id, &contribution(10, d(2026, 11, 1)), today()).unwrap();
        let b = add_contribution(c, id, &contribution(20, today()), today()).unwrap();
        let tie = add_contribution(c, id, &contribution(30, today()), today()).unwrap();
        let order: Vec<i64> = contributions(c, id).unwrap().iter().map(|x| x.id).collect();
        assert_eq!(order, vec![tie, b, a]);
        assert_eq!(contributed(c, id).unwrap().cents(), 60);
        // Another goal's contribution is not deletable through this goal.
        let other = create(c, &draft("Car", 5000)).unwrap();
        assert!(matches!(
            delete_contribution(c, other, a),
            Err(AppError::NotFound(_))
        ));
        delete_contribution(c, id, a).unwrap();
        assert_eq!(contributed(c, id).unwrap().cents(), 50);
        clean(&db);
    }

    #[test]
    fn contribution_overflow_rolls_back() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let id = create(c, &draft("Big", i64::MAX)).unwrap();
        add_contribution(c, id, &contribution(i64::MAX - 5, today()), today()).unwrap();
        let before = contributions(c, id).unwrap();
        let err = add_contribution(c, id, &contribution(6, today()), today()).unwrap_err();
        assert!(matches!(err, AppError::GoalInvalid(e) if e == vec![GoalError::Overflow]));
        assert_eq!(contributions(c, id).unwrap(), before);
        // The exact boundary still fits.
        add_contribution(c, id, &contribution(5, today()), today()).unwrap();
        assert_eq!(contributed(c, id).unwrap().cents(), i64::MAX);
        clean(&db);
    }

    #[test]
    fn goal_progress_distinguishes_overfunded_and_manual_completion() {
        use crate::domain::goal::progress;
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let id = create(c, &draft("Trip", 1000)).unwrap();
        let over = add_contribution(c, id, &contribution(1500, today()), today()).unwrap();
        let p = progress(get(c, id).unwrap().target, contributed(c, id).unwrap());
        assert!(p.funded);
        assert_eq!(
            (p.percent, p.remaining.cents(), p.excess.cents()),
            (100, 0, 500)
        );
        // Funded is independent of the manual status: Active stays Active.
        assert_eq!(get(c, id).unwrap().status, GoalStatus::Active);
        // Removing the overfunding unfunds it without touching the manual status.
        change_status(c, id, GoalTransition::Complete).unwrap();
        change_status(c, id, GoalTransition::Reopen).unwrap();
        delete_contribution(c, id, over).unwrap();
        let p = progress(get(c, id).unwrap().target, contributed(c, id).unwrap());
        assert!(!p.funded);
        // Manual completion below target keeps the honest percentage.
        add_contribution(c, id, &contribution(250, today()), today()).unwrap();
        change_status(c, id, GoalTransition::Complete).unwrap();
        let goal = get(c, id).unwrap();
        let p = progress(goal.target, contributed(c, id).unwrap());
        assert_eq!(goal.status, GoalStatus::Completed);
        assert_eq!((p.percent, p.remaining.cents(), p.funded), (25, 750, false));
        // Raising the target of a funded goal recomputes Funded only.
        let id2 = create(c, &draft("Car", 100)).unwrap();
        add_contribution(c, id2, &contribution(100, today()), today()).unwrap();
        update(c, id2, &draft("Car", 400)).unwrap();
        let p = progress(get(c, id2).unwrap().target, contributed(c, id2).unwrap());
        assert!(!p.funded);
        assert_eq!(get(c, id2).unwrap().status, GoalStatus::Active);
        clean(&db);
    }

    #[test]
    fn attribution_must_be_active_when_new_or_changed_but_history_survives() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let (owner, partner) = seed_people(c);
        let mut d1 = draft("Trip", 1000);
        d1.attribution_id = Some(partner);
        let id = create(c, &d1).unwrap();
        people_repo::set_archived(c, partner, true).unwrap();
        // Unchanged archived attribution is preserved on edit.
        d1.name = "Trip 2".into();
        update(c, id, &d1).unwrap();
        // A new goal or a changed attribution cannot choose an archived person.
        assert!(matches!(
            create(c, &d1),
            Err(AppError::GoalInvalid(e)) if e == vec![GoalError::AttributionArchived]
        ));
        let other = create(c, &draft("Car", 10)).unwrap();
        assert!(update(c, other, &d1).is_err());
        let mut missing = draft("Bad", 10);
        missing.attribution_id = Some(9999);
        assert!(create(c, &missing).is_err());
        // The owner stays selectable and a person with goals cannot be deleted (RESTRICT).
        let mut d2 = draft("Own", 10);
        d2.attribution_id = Some(owner);
        create(c, &d2).unwrap();
        assert!(
            c.execute("DELETE FROM people WHERE id = ?1", [partner])
                .is_err()
        );
        clean(&db);
    }

    #[test]
    fn goals_never_touch_ledger_or_settlement_figures() {
        use crate::application::services::{reports, settlement};
        use crate::domain::date::YearMonth;
        use crate::domain::transaction::Allocation;
        use crate::infrastructure::db::transactions_repo;
        use crate::shared::test_fixtures::draft_with;
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let (owner, partner) = seed_people(c);
        let mut tx = draft_with(vec![
            Allocation {
                person_id: owner,
                amount: Money::from_cents(500),
            },
            Allocation {
                person_id: partner,
                amount: Money::from_cents(500),
            },
        ]);
        tx.payer_id = Some(owner);
        tx.date = d(2026, 11, 5);
        transactions_repo::create(c, &tx).unwrap();
        let month = YearMonth::new(2026, 11).unwrap();
        let snapshot = |c: &Connection| {
            (
                reports::month_summary(c, month).unwrap(),
                reports::tracked_balance(c, Money::ZERO).unwrap(),
                settlement::for_month(c, month).unwrap().balances,
                transactions_repo::list(c, &crate::domain::transaction::TxFilter::default())
                    .unwrap()
                    .len(),
            )
        };
        let before = snapshot(c);
        let id = create(c, &draft("Trip", 1000)).unwrap();
        add_contribution(c, id, &contribution(700, d(2026, 11, 6)), today()).unwrap();
        change_status(c, id, GoalTransition::Complete).unwrap();
        assert_eq!(snapshot(c), before);
        clean(&db);
    }
}
