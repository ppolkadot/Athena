//! The singleton `profile` row (id = 1). It has no display name: the owner lives in `people`.

use rusqlite::{Connection, OptionalExtension, params};

use super::{bad_code, now_rfc3339};
use crate::domain::money::Money;
use crate::domain::settings::{Language, Profile, Theme};
use crate::shared::error::Result;

/// `None` before onboarding has created the row.
///
/// # Errors
/// Database errors, or a stored code this build does not know.
pub fn load(conn: &Connection) -> Result<Option<Profile>> {
    let profile = conn
        .query_row(
            "SELECT start_balance_cents, theme, language, onboarding_complete FROM profile WHERE id = 1",
            [],
            |row| {
                let theme: String = row.get(1)?;
                let language: String = row.get(2)?;
                Ok(Profile {
                    start_balance: Money::from_cents(row.get(0)?),
                    theme: Theme::from_code(&theme).ok_or_else(|| bad_code(1, "theme", &theme))?,
                    language: Language::from_code(&language)
                        .ok_or_else(|| bad_code(2, "language", &language))?,
                    onboarding_complete: row.get::<_, i64>(3)? != 0,
                })
            },
        )
        .optional()?;
    Ok(profile)
}

/// Insert or update the profile. `created_at` is set once; `updated_at` on every save.
///
/// # Errors
/// Database errors.
pub fn save(conn: &Connection, profile: &Profile) -> Result<()> {
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO profile (id, start_balance_cents, theme, language, onboarding_complete, created_at, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(id) DO UPDATE SET
            start_balance_cents = excluded.start_balance_cents,
            theme = excluded.theme,
            language = excluded.language,
            onboarding_complete = excluded.onboarding_complete,
            updated_at = excluded.updated_at",
        params![
            profile.start_balance.cents(),
            profile.theme.code(),
            profile.language.code(),
            i64::from(profile.onboarding_complete),
            now
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::Db;

    fn sample() -> Profile {
        Profile {
            start_balance: Money::from_cents(50_000),
            theme: Theme::Dark,
            language: Language::En,
            onboarding_complete: true,
        }
    }

    #[test]
    fn profile_round_trips_without_display_name() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(load(db.conn()).unwrap(), None);
        save(db.conn(), &sample()).unwrap();
        assert_eq!(load(db.conn()).unwrap().unwrap(), sample());
    }

    #[test]
    fn saving_twice_updates_the_single_row() {
        let db = Db::open_in_memory().unwrap();
        save(db.conn(), &sample()).unwrap();
        let created: String = db
            .conn()
            .query_row("SELECT created_at FROM profile", [], |r| r.get(0))
            .unwrap();
        let changed = Profile {
            start_balance: Money::from_cents(-2_500),
            theme: Theme::Light,
            language: Language::PtBr,
            onboarding_complete: false,
        };
        save(db.conn(), &changed).unwrap();
        assert_eq!(load(db.conn()).unwrap().unwrap(), changed);
        let (rows, created_after): (i64, String) = db
            .conn()
            .query_row("SELECT COUNT(*), MIN(created_at) FROM profile", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(created_after, created);
    }
}
