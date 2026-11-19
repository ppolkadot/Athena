//! Cross-row invariants that `SQLite` cannot express as CHECK constraints.

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
    /// A settlement row violates its shape constraints.
    SettlementShape { tx_id: i64 },
    /// A template's responsibility mode, payer or allocations disagree with its amount and kind.
    TemplateShape { template_id: i64 },
    /// A goal has invalid shape, a missing reference or an overflowing contribution total.
    GoalShape { goal_id: i64 },
    /// A saved closing violates its consistency rules (checked result, optional groups,
    /// fingerprint shape).
    ClosingShape { month: String },
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
    for tx_id in ids(
        conn,
        "SELECT t.id FROM transactions t WHERE t.kind = 'settlement' AND (
            t.payer_id IS NULL
            OR t.category_id IS NOT NULL
            OR t.split_mode IS NOT NULL
            OR (SELECT COUNT(*) FROM allocations a WHERE a.transaction_id = t.id) <> 1
            OR EXISTS (
                SELECT 1 FROM allocations a
                WHERE a.transaction_id = t.id
                  AND (a.person_id = t.payer_id OR a.amount_cents <> t.amount_cents)
            )
        ) ORDER BY t.id",
    )? {
        issues.push(IntegrityIssue::SettlementShape { tx_id });
    }
    for template_id in ids(
        conn,
        "SELECT t.id FROM templates t WHERE
            (t.kind = 'income' AND (t.payer_id IS NOT NULL OR t.responsibility <> 'none'))
            OR (t.responsibility = 'none'
                AND EXISTS (SELECT 1 FROM template_allocations a WHERE a.template_id = t.id))
            OR (t.responsibility = 'person'
                AND (SELECT COUNT(*) FROM template_allocations a WHERE a.template_id = t.id) <> 1)
            OR (t.responsibility IN ('even', 'custom')
                AND (SELECT COUNT(*) FROM template_allocations a WHERE a.template_id = t.id) < 2)
            OR (t.responsibility <> 'none'
                AND COALESCE((SELECT SUM(a.amount_cents) FROM template_allocations a
                              WHERE a.template_id = t.id), 0) <> t.amount_cents)
         ORDER BY t.id",
    )? {
        issues.push(IntegrityIssue::TemplateShape { template_id });
    }
    for goal_id in goal_issues(conn)? {
        issues.push(IntegrityIssue::GoalShape { goal_id });
    }
    for closing in super::closings_repo::list(conn)? {
        if closing.validate().is_err() {
            issues.push(IntegrityIssue::ClosingShape {
                month: closing.month.key(),
            });
        }
    }
    Ok(issues)
}

/// Goals with invalid shape, references, dates or contribution totals.
fn goal_issues(conn: &Connection) -> Result<Vec<i64>> {
    let valid = |text: &str| chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok();
    let mut bad = ids(
        conn,
        "SELECT g.id FROM goals g LEFT JOIN people p ON p.id = g.attribution_id
         WHERE (g.attribution_id IS NOT NULL AND p.id IS NULL)
            OR length(trim(g.name)) = 0 OR g.target_cents <= 0
            OR g.frequency NOT IN ('daily', 'weekly', 'monthly')
            OR g.status NOT IN ('active', 'paused', 'completed', 'archived')
            OR (g.status = 'archived') <> (g.pre_archive_status IS NOT NULL)
            OR g.pre_archive_status NOT IN ('active', 'paused', 'completed')
         UNION
         SELECT c.goal_id FROM goal_contributions c LEFT JOIN goals g ON g.id = c.goal_id
         WHERE g.id IS NULL OR c.amount_cents <= 0",
    )?;
    let mut stmt = conn.prepare("SELECT id, target_date FROM goals ORDER BY id")?;
    let goals = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, date) in goals {
        if date.as_deref().is_some_and(|d| !valid(d)) {
            bad.push(id);
        }
    }
    let mut stmt = conn.prepare(
        "SELECT goal_id, date, amount_cents FROM goal_contributions ORDER BY goal_id, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut totals: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    for (goal_id, date, cents) in rows {
        let total = totals.entry(goal_id).or_insert(0);
        match total.checked_add(cents) {
            Some(next) if valid(&date) => *total = next,
            _ => bad.push(goal_id),
        }
    }
    bad.sort_unstable();
    bad.dedup();
    Ok(bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::Db;
    use crate::shared::test_fixtures::seed_people;

    #[test]
    fn integrity_flags_invalid_goal_lifecycle_and_orphan_contribution() {
        let db = Db::open_in_memory().unwrap();
        let conn = db.conn();
        conn.execute_batch(
            "PRAGMA ignore_check_constraints = ON;
             PRAGMA foreign_keys = OFF;
             INSERT INTO goals (id, name, target_cents, frequency, status, created_at, updated_at)
             VALUES (1, 'Trip', 100, 'monthly', 'archived', '2026-11-11T00:00:00Z',
                     '2026-11-11T00:00:00Z');
             INSERT INTO goal_contributions (goal_id, amount_cents, date, created_at)
             VALUES (2, 100, '2026-11-11', '2026-11-11T00:00:00Z');
             PRAGMA foreign_keys = ON;
             PRAGMA ignore_check_constraints = OFF;",
        )
        .unwrap();
        assert_eq!(
            check(conn).unwrap(),
            vec![
                IntegrityIssue::GoalShape { goal_id: 1 },
                IntegrityIssue::GoalShape { goal_id: 2 },
            ]
        );

        conn.execute_batch(
            "UPDATE goals SET pre_archive_status = 'active' WHERE id = 1;
             UPDATE goal_contributions SET goal_id = 1 WHERE goal_id = 2;",
        )
        .unwrap();
        assert!(check(conn).unwrap().is_empty());
    }

    #[test]
    fn integrity_flags_settlement_shape() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let c = db.conn();
        c.execute(
            "INSERT INTO transactions (title, amount_cents, date, kind, category_id, payer_id, split_mode, created_at, updated_at)
             VALUES ('Bad', 1000, '2026-11-14', 'settlement', NULL, ?1, NULL, '2026-11-14T00:00:00Z', '2026-11-14T00:00:00Z')",
            [partner],
        )
        .unwrap();
        let tx_id = c.last_insert_rowid();
        c.execute(
            "INSERT INTO allocations (transaction_id, person_id, amount_cents) VALUES (?1, ?2, 500)",
            [tx_id, owner],
        )
        .unwrap();
        c.execute(
            "INSERT INTO allocations (transaction_id, person_id, amount_cents) VALUES (?1, ?2, 500)",
            [tx_id, partner],
        )
        .unwrap();
        assert!(
            check(c)
                .unwrap()
                .contains(&IntegrityIssue::SettlementShape { tx_id })
        );
    }
}
