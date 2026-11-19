//! Goal read models and workflows (contract §14.3). Writes are checked in the repository inside one
//! transaction; this service shapes what the screens read and never touches the ledger.

use chrono::NaiveDate;
use rusqlite::Connection;

use crate::domain::goal::{
    Contribution, ContributionDraft, Goal, GoalDraft, GoalStatus, GoalTransition, Progress,
    progress,
};
use crate::domain::money::Money;
use crate::infrastructure::db::goals_repo;
use crate::shared::error::Result;

/// A goal with its funding progress, for the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalListItem {
    pub goal: Goal,
    pub progress: Progress,
}

/// The selected goal with its contribution history (`date DESC, id DESC`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalDetail {
    pub goal: Goal,
    pub progress: Progress,
    pub contributions: Vec<Contribution>,
}

/// Which lifecycle tab a goal belongs to.
#[must_use]
pub fn in_filter(goal: &Goal, filter: GoalStatus) -> bool {
    goal.status == filter
}

/// Every goal with progress, newest first within the stable order the screen filters further.
///
/// # Errors
/// Database errors, or an unrepresentable contribution total.
pub fn list_items(conn: &Connection) -> Result<Vec<GoalListItem>> {
    let totals = goals_repo::contributed_totals(conn)?;
    Ok(goals_repo::list(conn)?
        .into_iter()
        .map(|goal| {
            let contributed = totals.get(&goal.id).copied().unwrap_or(Money::ZERO);
            let progress = progress(goal.target, contributed);
            GoalListItem { goal, progress }
        })
        .collect())
}

/// One goal's detail. `None` when it no longer exists.
///
/// # Errors
/// Database errors other than a missing goal.
pub fn detail(conn: &Connection, id: i64) -> Result<Option<GoalDetail>> {
    let goal = match goals_repo::get(conn, id) {
        Ok(goal) => goal,
        Err(crate::shared::error::AppError::NotFound(_)) => return Ok(None),
        Err(err) => return Err(err),
    };
    let contributed = goals_repo::contributed(conn, id)?;
    Ok(Some(GoalDetail {
        progress: progress(goal.target, contributed),
        contributions: goals_repo::contributions(conn, id)?,
        goal,
    }))
}

/// # Errors
/// See [`goals_repo::create`].
pub fn create(conn: &Connection, draft: &GoalDraft) -> Result<i64> {
    goals_repo::create(conn, draft)
}

/// # Errors
/// See [`goals_repo::update`].
pub fn update(conn: &Connection, id: i64, draft: &GoalDraft) -> Result<()> {
    goals_repo::update(conn, id, draft)
}

/// # Errors
/// See [`goals_repo::change_status`].
pub fn change_status(conn: &Connection, id: i64, transition: GoalTransition) -> Result<()> {
    goals_repo::change_status(conn, id, transition)
}

/// # Errors
/// See [`goals_repo::add_contribution`]; `today` is the Store's local date.
pub fn add_contribution(
    conn: &Connection,
    goal_id: i64,
    draft: &ContributionDraft,
    today: NaiveDate,
) -> Result<i64> {
    goals_repo::add_contribution(conn, goal_id, draft, today)
}

/// # Errors
/// See [`goals_repo::delete_contribution`].
pub fn delete_contribution(conn: &Connection, goal_id: i64, contribution_id: i64) -> Result<()> {
    goals_repo::delete_contribution(conn, goal_id, contribution_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::goal::Frequency;
    use crate::infrastructure::db::Db;

    #[test]
    fn list_items_and_detail_carry_exact_progress() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let today = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let id = create(
            c,
            &GoalDraft {
                name: "Trip".into(),
                target: Money::from_cents(1000),
                target_date: None,
                frequency: Frequency::Monthly,
                attribution_id: None,
            },
        )
        .unwrap();
        for cents in [300, 900] {
            add_contribution(
                c,
                id,
                &ContributionDraft {
                    amount: Money::from_cents(cents),
                    date: today,
                    note: Some(format!("note {cents}")),
                },
                today,
            )
            .unwrap();
        }
        let items = list_items(c).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].progress.excess.cents(), 200);
        assert!(items[0].progress.funded);
        let d = detail(c, id).unwrap().unwrap();
        assert_eq!(d.contributions.len(), 2);
        assert_eq!(d.contributions[0].note.as_deref(), Some("note 900"));
        assert!(detail(c, 999).unwrap().is_none());
    }
}
