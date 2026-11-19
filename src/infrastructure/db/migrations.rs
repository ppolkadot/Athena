//! Schema migrations: embedded SQL files, `PRAGMA user_version`, one SQL transaction per file.

use rusqlite::Connection;

use crate::shared::error::{AppError, Result};

/// Ordered migrations. The schema version equals the number of applied entries.
/// Tests assert `MIGRATIONS.len()`, never a hard-coded version.
pub const MIGRATIONS: &[&str] = &[
    include_str!("../../../migrations/0001_init.sql"),
    include_str!("../../../migrations/0002_settlement_kind.sql"),
    include_str!("../../../migrations/0003_trash.sql"),
    include_str!("../../../migrations/0004_templates.sql"),
    include_str!("../../../migrations/0005_appearance.sql"),
];

/// The schema version this build produces: the number of embedded migrations.
#[must_use]
pub fn latest_version() -> i64 {
    i64::try_from(MIGRATIONS.len()).unwrap_or(i64::MAX)
}

/// Read `PRAGMA user_version`.
///
/// # Errors
/// Database errors.
pub fn schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.pragma_query_value(None, "user_version", |r| r.get(0))?)
}

/// Apply every embedded migration the database has not seen.
///
/// # Errors
/// [`AppError::SchemaTooNew`] for a newer database; [`AppError::Migration`] when a step fails (the
/// failed step is rolled back and the previous version stays in place).
pub fn run_migrations(conn: &mut Connection) -> Result<()> {
    run_migrations_with(conn, MIGRATIONS)
}

fn run_step(conn: &mut Connection, step: usize, sql: &str, check_foreign_keys: bool) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch(sql)
        .map_err(|e| AppError::Migration(format!("step {step}: {e}")))?;
    if check_foreign_keys {
        let violation: Option<(String, i64, String, i64)> = {
            let mut stmt = tx.prepare("PRAGMA foreign_key_check")?;
            let mut rows =
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.next().transpose()?
        };
        if let Some((table, rowid, parent, _)) = violation {
            return Err(AppError::Migration(format!(
                "step {step}: foreign key check failed on {table} row {rowid} -> {parent}"
            )));
        }
    }
    tx.pragma_update(
        None,
        "user_version",
        i64::try_from(step).unwrap_or(i64::MAX),
    )?;
    tx.commit()?;
    Ok(())
}

/// Same as [`run_migrations`] for an explicit list, so tests can exercise failure and upgrades.
///
/// # Errors
/// See [`run_migrations`].
pub fn run_migrations_with(conn: &mut Connection, migrations: &[&str]) -> Result<()> {
    let supported = i64::try_from(migrations.len()).unwrap_or(i64::MAX);
    let found = schema_version(conn)?;
    if found > supported {
        return Err(AppError::SchemaTooNew { found, supported });
    }
    let start = usize::try_from(found).unwrap_or(0);
    for (index, sql) in migrations.iter().enumerate().skip(start) {
        let step = index + 1;
        let fk_off = sql.lines().next().map(str::trim) == Some("-- athena:foreign-keys-off");
        if fk_off {
            let prev_fk: bool = conn.pragma_query_value(None, "foreign_keys", |r| r.get(0))?;
            conn.pragma_update(None, "foreign_keys", false)?;
            let outcome = run_step(conn, step, sql, true);
            let restore = conn.pragma_update(None, "foreign_keys", prev_fk);
            outcome?;
            restore?;
        } else {
            run_step(conn, step, sql, false)?;
        }
    }
    Ok(())
}
