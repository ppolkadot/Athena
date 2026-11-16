//! SQLite access. The UI never issues SQL; repositories in this folder do.

pub mod backup;
pub mod categories_repo;
pub mod integrity;
pub mod migrations;
pub mod people_repo;
pub mod profile_repo;
pub mod reports_repo;
pub mod transactions_repo;

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::functions::FunctionFlags;
use rusqlite::{Connection, ErrorCode, OpenFlags};

pub use migrations::{MIGRATIONS, latest_version, run_migrations, run_migrations_with};

use crate::shared::error::{AppError, Result};

#[derive(Debug)]
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (creating if needed) the database file, take a pre-migration backup when upgrading an
    /// existing file, then migrate.
    ///
    /// A database newer than this build supports is refused before anything is written to it.
    ///
    /// # Errors
    /// [`AppError::SchemaTooNew`], [`AppError::Corrupt`], [`AppError::Migration`] and I/O or
    /// database errors.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, MIGRATIONS)
    }

    pub(crate) fn open_with(path: &Path, migrations: &[&str]) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let supported = i64::try_from(migrations.len()).unwrap_or(i64::MAX);
        let existing = if path.exists() {
            peek_version(path)?
        } else {
            0
        };
        if existing > supported {
            return Err(AppError::SchemaTooNew {
                found: existing,
                supported,
            });
        }
        let mut conn = Connection::open(path)?;
        register_functions(&conn)?;
        configure_file(&conn)?;
        if existing > 0 && existing < supported {
            let dest = backups_dir_for(path).join(format!(
                "pre-migration-v{existing}-{}.db",
                chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S")
            ));
            backup::vacuum_into(&conn, &dest)?;
            return run_migrations_with(&mut conn, migrations)
                .map_err(|err| match err {
                    AppError::Migration(msg) => AppError::Migration(format!(
                        "{msg}. Your data was backed up to {}",
                        dest.display()
                    )),
                    other => other,
                })
                .map(|()| Self { conn });
        }
        run_migrations_with(&mut conn, migrations)?;
        Ok(Self { conn })
    }

    /// A migrated in-memory database. Used by tests and previews.
    ///
    /// # Errors
    /// Database or migration errors.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        register_functions(&conn)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        run_migrations(&mut conn)?;
        Ok(Self { conn })
    }

    /// Read-only handle used to verify backups. Runs no migrations.
    ///
    /// # Errors
    /// Database errors.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        register_functions(&conn)?;
        Ok(Self { conn })
    }

    #[must_use]
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

/// Current time as UTC RFC 3339 with second precision: `2026-11-12T14:03:00Z`.
pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Row-mapping error for a stored code that this build does not know.
pub(crate) fn bad_code(column: usize, what: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        format!("unknown {what} '{value}'").into(),
    )
}

/// SQL helper functions every connection needs. `fold(text)` is the accent- and case-insensitive
/// folding from `domain::text`, used by search.
fn register_functions(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "fold",
        1,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let text: Option<String> = ctx.get(0)?;
            Ok(crate::domain::text::fold(
                text.as_deref().unwrap_or_default(),
            ))
        },
    )?;
    Ok(())
}

fn configure_file(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    Ok(())
}

/// `backups/` next to the database file, matching the `AppDirs` layout.
fn backups_dir_for(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .map_or_else(|| PathBuf::from("backups"), |p| p.join("backups"))
}

/// Read `user_version` through a read-only handle so a too-new or unreadable file is never modified.
fn peek_version(path: &Path) -> Result<i64> {
    let corrupt = || AppError::Corrupt {
        path: path.display().to_string(),
    };
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| corrupt())?;
    migrations::schema_version(&conn).map_err(|err| match err {
        AppError::Db(rusqlite::Error::SqliteFailure(e, _))
            if matches!(e.code, ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) =>
        {
            corrupt()
        }
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(conn: &Connection) -> i64 {
        conn.pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn opens_and_seeds_database() {
        let db = Db::open_in_memory().unwrap();
        let categories: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM categories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(categories, 12);
        let income: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE applies_to = 'income'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(income, 2);
        assert_eq!(version(db.conn()), latest_version());
        assert_ne!(MIGRATIONS.len(), 0);
        let fk: i64 = db
            .conn()
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn rejects_newer_schema() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 99_i64).unwrap();
        assert!(matches!(
            run_migrations(&mut conn),
            Err(AppError::SchemaTooNew { found: 99, .. })
        ));
    }

    #[test]
    fn failed_migration_rolls_back() {
        let mut conn = Connection::open_in_memory().unwrap();
        let bad = ["CREATE TABLE bad (id INTEGER,"];
        assert!(run_migrations_with(&mut conn, &bad).is_err());
        assert_eq!(version(&conn), 0);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'bad'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
    }

    #[test]
    fn half_applied_step_rolls_back_the_whole_step() {
        let mut conn = Connection::open_in_memory().unwrap();
        let steps = [
            "CREATE TABLE a (id INTEGER);",
            "CREATE TABLE b (id INTEGER); INSERT INTO nope VALUES (1);",
        ];
        assert!(run_migrations_with(&mut conn, &steps).is_err());
        assert_eq!(version(&conn), 1);
        let b: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(b, 0);
    }

    #[test]
    fn file_database_uses_wal_and_foreign_keys() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("athena.db")).unwrap();
        let mode: String = db
            .conn()
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        let fk: i64 = db
            .conn()
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
        drop(db);
        let again = Db::open(&dir.path().join("athena.db")).unwrap();
        assert_eq!(version(again.conn()), latest_version());
        assert!(!dir.path().join("backups").exists());
    }

    #[test]
    fn upgrading_an_existing_file_writes_a_pre_migration_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("athena.db");
        let first = ["CREATE TABLE a (id INTEGER); INSERT INTO a VALUES (7);"];
        drop(Db::open_with(&path, &first).unwrap());
        let both = [first[0], "CREATE TABLE b (id INTEGER);"];
        let db = Db::open_with(&path, &both).unwrap();
        assert_eq!(version(db.conn()), 2);
        let backups: Vec<String> = std::fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].starts_with("pre-migration-v1-"));
        assert_eq!(
            Path::new(&backups[0]).extension().and_then(|e| e.to_str()),
            Some("db")
        );
        let copy = Db::open_read_only(&dir.path().join("backups").join(&backups[0])).unwrap();
        let n: i64 = copy
            .conn()
            .query_row("SELECT id FROM a", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 7);
    }

    #[test]
    fn failed_upgrade_keeps_the_old_version_and_reports_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("athena.db");
        let first = ["CREATE TABLE a (id INTEGER);"];
        drop(Db::open_with(&path, &first).unwrap());
        let bad = [first[0], "CREATE TABLE broken ("];
        let err = Db::open_with(&path, &bad).unwrap_err();
        let AppError::Migration(message) = err else {
            panic!("expected a migration error")
        };
        assert!(message.contains("pre-migration-v1-"), "{message}");
        let conn = Connection::open(&path).unwrap();
        assert_eq!(version(&conn), 1);
    }

    #[test]
    fn newer_schema_file_is_refused_without_being_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("athena.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE t (x INTEGER);").unwrap();
            conn.pragma_update(None, "user_version", 99_i64).unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        let err = Db::open(&path).unwrap_err();
        assert!(matches!(err, AppError::SchemaTooNew { found: 99, .. }));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.path().join("athena.db-wal").exists());
        assert!(!dir.path().join("athena.db-shm").exists());
    }

    #[test]
    fn unreadable_file_reports_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("athena.db");
        std::fs::write(
            &path,
            b"this is definitely not a sqlite database, just text padding.....",
        )
        .unwrap();
        let err = Db::open(&path).unwrap_err();
        let AppError::Corrupt { path: reported } = err else {
            panic!("expected Corrupt")
        };
        assert!(reported.ends_with("athena.db"));
    }

    #[test]
    fn schema_enforces_owner_and_money_rules() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let now = "2026-11-11T00:00:00Z";
        c.execute(
            "INSERT INTO people (name, kind, created_at) VALUES ('A3', 'owner', ?1)",
            [now],
        )
        .unwrap();
        assert!(
            c.execute(
                "INSERT INTO people (name, kind, created_at) VALUES ('B', 'owner', ?1)",
                [now]
            )
            .is_err()
        );
        assert!(
            c.execute("INSERT INTO people (name, kind, archived, created_at) VALUES ('X', 'owner', 1, ?1)", [now])
                .is_err()
        );
        assert!(
            c.execute(
                "INSERT INTO people (name, kind, created_at) VALUES ('  ', 'partner', ?1)",
                [now]
            )
            .is_err()
        );
        c.execute(
            "INSERT INTO people (name, kind, created_at) VALUES ('Ana', 'partner', ?1)",
            [now],
        )
        .unwrap();
        let bad_amount = "INSERT INTO transactions (title, amount_cents, date, kind, created_at, updated_at) VALUES ('t', 0, '2026-11-11', 'expense', ?1, ?1)";
        assert!(c.execute(bad_amount, [now]).is_err());
        let bad_date = "INSERT INTO transactions (title, amount_cents, date, kind, created_at, updated_at) VALUES ('t', 1, '11/11/2026', 'expense', ?1, ?1)";
        assert!(c.execute(bad_date, [now]).is_err());
        let ok = "INSERT INTO transactions (title, amount_cents, date, kind, created_at, updated_at) VALUES ('t', 100, '2026-11-11', 'expense', ?1, ?1)";
        c.execute(ok, [now]).unwrap();
        c.execute(
            "INSERT INTO allocations (transaction_id, person_id, amount_cents) VALUES (1, 1, 100)",
            [],
        )
        .unwrap();
        // One allocation per person per transaction; a person with allocations cannot be deleted.
        assert!(c.execute("INSERT INTO allocations (transaction_id, person_id, amount_cents) VALUES (1, 1, 0)", []).is_err());
        assert!(c.execute("DELETE FROM people WHERE id = 1", []).is_err());
        c.execute("DELETE FROM transactions WHERE id = 1", [])
            .unwrap();
        let left: i64 = c
            .query_row("SELECT COUNT(*) FROM allocations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn category_names_are_unique_ignoring_ascii_case() {
        let db = Db::open_in_memory().unwrap();
        let dup =
            "INSERT INTO categories (name, created_at) VALUES ('food', '2026-11-11T00:00:00Z')";
        assert!(db.conn().execute(dup, []).is_err());
    }

    #[test]
    fn fk_off_migration_restores_foreign_keys_on_success_and_failure() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        let ok_steps = [
            "CREATE TABLE parent (id INTEGER PRIMARY KEY);",
            "-- athena:foreign-keys-off\nCREATE TABLE child (id INTEGER PRIMARY KEY, p INTEGER REFERENCES parent(id));",
        ];
        run_migrations_with(&mut conn, &ok_steps).unwrap();
        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);

        let mut fail_conn = Connection::open_in_memory().unwrap();
        fail_conn.pragma_update(None, "foreign_keys", true).unwrap();
        let bad_steps = [
            "CREATE TABLE parent (id INTEGER PRIMARY KEY);",
            "-- athena:foreign-keys-off\nCREATE TABLE broken (id INTEGER,",
        ];
        assert!(run_migrations_with(&mut fail_conn, &bad_steps).is_err());
        let fk_after_err: i64 = fail_conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk_after_err, 1);
    }

    #[test]
    fn fk_off_migration_rolls_back_on_foreign_key_violation() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        let steps = [
            "CREATE TABLE parent (id INTEGER PRIMARY KEY); CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL REFERENCES parent(id));",
            "-- athena:foreign-keys-off\nINSERT INTO child (id, parent_id) VALUES (1, 999);",
        ];
        let err = run_migrations_with(&mut conn, &steps).unwrap_err();
        assert!(
            matches!(err, AppError::Migration(ref msg) if msg.contains("foreign key check failed")),
            "unexpected error: {err:?}"
        );
        assert_eq!(version(&conn), 1);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM child", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let fk: i64 = conn
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn upgrade_v1_preserves_rows_allocations_and_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("athena.db");
        let expected_txs;
        {
            let v1 = Db::open_with(&path, &MIGRATIONS[..1]).unwrap();
            crate::shared::test_fixtures::seed_mixed(&v1);
            expected_txs = transactions_repo::list(
                v1.conn(),
                &crate::domain::transaction::TxFilter::default(),
            )
            .unwrap();
        }
        let upgraded = Db::open(&path).unwrap();
        assert_eq!(version(upgraded.conn()), latest_version());
        let actual_txs = transactions_repo::list(
            upgraded.conn(),
            &crate::domain::transaction::TxFilter::default(),
        )
        .unwrap();
        assert_eq!(actual_txs, expected_txs);
        assert!(integrity::check(upgraded.conn()).unwrap().is_empty());
    }

    #[test]
    fn upgrade_v1_does_not_reuse_hard_deleted_ids() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        run_migrations_with(&mut conn, &MIGRATIONS[..1]).unwrap();
        for i in 1..=3 {
            conn.execute(
                "INSERT INTO transactions (title, amount_cents, date, kind, created_at, updated_at)
                 VALUES (?1, 100, '2026-11-11', 'expense', '2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z')",
                [format!("T{i}")],
            )
            .unwrap();
        }
        conn.execute("DELETE FROM transactions WHERE id = 3", [])
            .unwrap();
        run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO transactions (title, amount_cents, date, kind, created_at, updated_at)
             VALUES ('T4', 100, '2026-11-11', 'expense', '2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z')",
            [],
        )
        .unwrap();
        assert!(conn.last_insert_rowid() >= 4);
    }

    #[test]
    fn schema_accepts_settlement_and_guards_its_shape() {
        let db = Db::open_in_memory().unwrap();
        let (owner, _) = crate::shared::test_fixtures::seed_people(db.conn());
        let cat = crate::shared::test_fixtures::seeded_category(db.conn(), "Food");
        let c = db.conn();
        c.execute(
            "INSERT INTO transactions (title, amount_cents, date, kind, category_id, payer_id, split_mode, created_at, updated_at)
             VALUES ('Payback', 500, '2026-11-11', 'settlement', NULL, ?1, NULL, '2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z')",
            [owner],
        )
        .unwrap();
        assert!(
            c.execute(
                "INSERT INTO transactions (title, amount_cents, date, kind, category_id, payer_id, split_mode, created_at, updated_at)
                 VALUES ('No payer', 500, '2026-11-11', 'settlement', NULL, NULL, NULL, '2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z')",
                [],
            )
            .is_err()
        );
        assert!(
            c.execute(
                "INSERT INTO transactions (title, amount_cents, date, kind, category_id, payer_id, split_mode, created_at, updated_at)
                 VALUES ('With cat', 500, '2026-11-11', 'settlement', ?1, ?2, NULL, '2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z')",
                [cat, owner],
            )
            .is_err()
        );
        assert!(
            c.execute(
                "INSERT INTO transactions (title, amount_cents, date, kind, category_id, payer_id, split_mode, created_at, updated_at)
                 VALUES ('With split', 500, '2026-11-11', 'settlement', NULL, ?1, 'even', '2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z')",
                [owner],
            )
            .is_err()
        );
    }

    #[test]
    fn trash_column_view_and_application_id_exist() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        let app_id: i64 = c
            .pragma_query_value(None, "application_id", |r| r.get(0))
            .unwrap();
        assert_eq!(app_id, 0x4154_4E41);

        let cols: Vec<String> = c
            .prepare("PRAGMA table_info(transactions)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert!(cols.contains(&"deleted_at".to_string()));

        assert!(
            c.execute(
                "INSERT INTO transactions (title, amount_cents, date, kind, deleted_at, created_at, updated_at)
                 VALUES ('Bad deleted', 100, '2026-11-11', 'expense', 'not-a-timestamp', '2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z')",
                [],
            )
            .is_err()
        );

        c.execute(
            "INSERT INTO transactions (title, amount_cents, date, kind, deleted_at, created_at, updated_at)
             VALUES ('Live', 100, '2026-11-11', 'expense', NULL, '2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO transactions (title, amount_cents, date, kind, deleted_at, created_at, updated_at)
             VALUES ('Deleted', 100, '2026-11-11', 'expense', '2026-11-16T10:00:00Z', '2026-11-11T00:00:00Z', '2026-11-11T00:00:00Z')",
            [],
        )
        .unwrap();
        let live_count: i64 = c
            .query_row("SELECT COUNT(*) FROM live_transactions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(live_count, 1);
    }

    #[test]
    fn upgrade_writes_pre_migration_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("athena.db");
        drop(Db::open_with(&path, &MIGRATIONS[..1]).unwrap());
        let db = Db::open(&path).unwrap();
        assert_eq!(version(db.conn()), latest_version());
        let backups: Vec<String> = std::fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("pre-migration-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert!(backups[0].starts_with("pre-migration-v1-"));
    }
}
