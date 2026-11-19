//! The optional single `appearance` row (id = 1, contract §14.2). A missing row means defaults.

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{bad_code, now_rfc3339};
use crate::domain::appearance::{
    Accent, AppearancePrefs, Density, Fit, MotionPref, Preset, Quality, TextScale, ThemeMode,
    Wallpaper, WallpaperKind,
};
use crate::shared::error::{AppError, Result};

const SELECT: &str = "SELECT theme_mode, preset, accent, density, text_scale, motion,
    reduce_transparency, quality, wallpaper_kind, wallpaper_asset_id, fit, dim, paused
    FROM appearance WHERE id = 1";

fn parse<T>(
    column: usize,
    what: &str,
    row: &Row<'_>,
    from_code: impl Fn(&str) -> Option<T>,
) -> rusqlite::Result<T> {
    let code: String = row.get(column)?;
    from_code(&code).ok_or_else(|| bad_code(column, what, &code))
}

fn to_prefs(row: &Row<'_>) -> rusqlite::Result<AppearancePrefs> {
    let scale: i64 = row.get(4)?;
    let kind = parse(8, "wallpaper kind", row, WallpaperKind::from_code)?;
    let asset: Option<String> = row.get(9)?;
    let dim: i64 = row.get(11)?;
    Ok(AppearancePrefs {
        theme_mode: parse(0, "theme mode", row, ThemeMode::from_code)?,
        preset: parse(1, "preset", row, Preset::from_code)?,
        accent: parse(2, "accent", row, Accent::from_code)?,
        density: parse(3, "density", row, Density::from_code)?,
        text_scale: TextScale::from_percent(scale)
            .ok_or_else(|| bad_code(4, "text scale", &scale.to_string()))?,
        motion: parse(5, "motion", row, MotionPref::from_code)?,
        reduce_transparency: row.get::<_, i64>(6)? != 0,
        quality: parse(7, "quality", row, Quality::from_code)?,
        wallpaper: Wallpaper::from_parts(kind, asset.as_deref())
            .map_err(|_| bad_code(9, "wallpaper", "asset"))?,
        fit: parse(10, "fit", row, Fit::from_code)?,
        dim: u8::try_from(dim)
            .ok()
            .filter(|d| *d <= 100)
            .ok_or_else(|| bad_code(11, "dim", &dim.to_string()))?,
        paused: row.get::<_, i64>(12)? != 0,
    })
}

/// `None` when the user has never applied appearance settings.
///
/// # Errors
/// Database errors, or a stored value this build does not accept. The error text never contains
/// media paths or personal data.
pub fn load(conn: &Connection) -> Result<Option<AppearancePrefs>> {
    Ok(conn.query_row(SELECT, [], to_prefs).optional()?)
}

/// Write the full row.
///
/// # Errors
/// [`AppError::Validation`] for an out-of-range value; database errors.
pub fn save(conn: &Connection, prefs: &AppearancePrefs) -> Result<()> {
    prefs.validate()?;
    conn.execute(
        "INSERT INTO appearance (id, theme_mode, preset, accent, density, text_scale, motion,
            reduce_transparency, quality, wallpaper_kind, wallpaper_asset_id, fit, dim, paused,
            updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(id) DO UPDATE SET
            theme_mode = excluded.theme_mode, preset = excluded.preset,
            accent = excluded.accent, density = excluded.density,
            text_scale = excluded.text_scale, motion = excluded.motion,
            reduce_transparency = excluded.reduce_transparency, quality = excluded.quality,
            wallpaper_kind = excluded.wallpaper_kind,
            wallpaper_asset_id = excluded.wallpaper_asset_id, fit = excluded.fit,
            dim = excluded.dim, paused = excluded.paused, updated_at = excluded.updated_at",
        params![
            prefs.theme_mode.code(),
            prefs.preset.code(),
            prefs.accent.code(),
            prefs.density.code(),
            i64::from(prefs.text_scale.percent()),
            prefs.motion.code(),
            i64::from(prefs.reduce_transparency),
            prefs.quality.code(),
            prefs.wallpaper.kind().code(),
            prefs.wallpaper.asset_id().map(|id| id.as_str().to_owned()),
            prefs.fit.code(),
            i64::from(prefs.dim),
            i64::from(prefs.paused),
            now_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Delete the row so appearance resolves to the profile-derived defaults. Idempotent.
///
/// # Errors
/// Database errors.
pub fn reset(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM appearance WHERE id = 1", [])?;
    Ok(())
}

/// Whether the stored row exists but cannot be read; used to report without exposing its content.
#[must_use]
pub fn is_unreadable(conn: &Connection) -> bool {
    matches!(load(conn), Err(AppError::Db(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::appearance::AssetId;
    use crate::domain::settings::{Language, Profile, Theme};
    use crate::infrastructure::db::migrations::{MIGRATIONS, run_migrations_with};
    use crate::infrastructure::db::{Db, integrity, profile_repo, templates_repo};

    fn sample() -> AppearancePrefs {
        AppearancePrefs {
            theme_mode: ThemeMode::System,
            preset: Preset::Aurora,
            accent: Accent::Violet,
            density: Density::Compact,
            text_scale: TextScale::P130,
            motion: MotionPref::Full,
            reduce_transparency: true,
            quality: Quality::Low,
            wallpaper: Wallpaper::Local(
                AssetId::parse("0123456789abcdef0123456789abcdef").unwrap(),
            ),
            fit: Fit::Fit,
            dim: 65,
            paused: true,
        }
    }

    #[test]
    fn missing_row_loads_none_and_saves_round_trip() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(load(db.conn()).unwrap(), None);
        save(db.conn(), &sample()).unwrap();
        assert_eq!(load(db.conn()).unwrap(), Some(sample()));
        let changed = AppearancePrefs {
            wallpaper: Wallpaper::Builtin,
            ..sample()
        };
        save(db.conn(), &changed).unwrap();
        assert_eq!(load(db.conn()).unwrap(), Some(changed));
        let rows: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM appearance", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        reset(db.conn()).unwrap();
        assert_eq!(load(db.conn()).unwrap(), None);
        reset(db.conn()).unwrap();
        assert!(integrity::check(db.conn()).unwrap().is_empty());
    }

    #[test]
    fn out_of_range_dim_is_rejected_before_writing() {
        let db = Db::open_in_memory().unwrap();
        let bad = AppearancePrefs {
            dim: 101,
            ..AppearancePrefs::default()
        };
        assert!(matches!(
            save(db.conn(), &bad),
            Err(AppError::Validation(_))
        ));
        assert_eq!(load(db.conn()).unwrap(), None);
    }

    #[test]
    fn sql_checks_match_the_domain_allowlist() {
        let db = Db::open_in_memory().unwrap();
        let c = db.conn();
        // Closed enums and ranges, checked column by column through a parameterised row.
        let attempt = |column: &str, value: &str| {
            c.execute("DELETE FROM appearance", []).unwrap();
            let mut cols = vec![
                ("theme_mode", "'dark'"),
                ("preset", "'onyx'"),
                ("accent", "'blue'"),
                ("density", "'comfortable'"),
                ("text_scale", "100"),
                ("motion", "'system'"),
                ("reduce_transparency", "0"),
                ("quality", "'auto'"),
                ("wallpaper_kind", "'none'"),
                ("wallpaper_asset_id", "NULL"),
                ("fit", "'fill'"),
                ("dim", "0"),
                ("paused", "0"),
            ];
            for entry in &mut cols {
                if entry.0 == column {
                    entry.1 = value;
                }
            }
            let names: Vec<_> = cols.iter().map(|c| c.0).collect();
            let values: Vec<_> = cols.iter().map(|c| c.1).collect();
            c.execute(
                &format!(
                    "INSERT INTO appearance (id, {}, updated_at) VALUES (1, {}, 'x')",
                    names.join(", "),
                    values.join(", ")
                ),
                [],
            )
            .is_ok()
        };
        assert!(attempt("theme_mode", "'light'"));
        assert!(!attempt("theme_mode", "'auto'"));
        assert!(!attempt("preset", "'glass'"));
        assert!(!attempt("accent", "'red'"));
        assert!(!attempt("density", "'tight'"));
        assert!(attempt("text_scale", "150"));
        assert!(!attempt("text_scale", "120"));
        assert!(!attempt("motion", "'off'"));
        assert!(!attempt("quality", "'ultra'"));
        assert!(!attempt("reduce_transparency", "2"));
        assert!(!attempt("fit", "'stretch'"));
        assert!(attempt("dim", "100"));
        assert!(!attempt("dim", "101"));
        assert!(!attempt("dim", "-1"));
        assert!(!attempt("paused", "2"));
        assert!(!attempt("wallpaper_kind", "'video'"));
        // The asset id is required for local and forbidden otherwise.
        assert!(!attempt("wallpaper_kind", "'local'"));
        assert!(!attempt(
            "wallpaper_asset_id",
            "'0123456789abcdef0123456789abcdef'"
        ));
        c.execute("DELETE FROM appearance", []).unwrap();
        let local = |id: &str| {
            c.execute(
                &format!(
                    "INSERT INTO appearance (id, theme_mode, preset, accent, density, text_scale,
                       motion, reduce_transparency, quality, wallpaper_kind, wallpaper_asset_id,
                       fit, dim, paused, updated_at)
                     VALUES (1, 'dark', 'onyx', 'blue', 'comfortable', 100, 'system', 0, 'auto',
                       'local', {id}, 'fill', 0, 0, 'x')"
                ),
                [],
            )
            .is_ok()
        };
        assert!(!local("'ABCDEF0123456789abcdef0123456789'"));
        assert!(!local("'short'"));
        assert!(local("'0123456789abcdef0123456789abcdef'"));
    }

    #[test]
    fn migration_0005_preserves_profile_and_templates() {
        for level in 1..MIGRATIONS.len() {
            let mut conn = rusqlite::Connection::open_in_memory().unwrap();
            conn.pragma_update(None, "foreign_keys", true).unwrap();
            run_migrations_with(&mut conn, &MIGRATIONS[..level]).unwrap();
            conn.execute(
                "INSERT INTO profile (id, start_balance_cents, theme, language, onboarding_complete,
                    created_at, updated_at) VALUES (1, 12345, 'light', 'pt-BR', 1, 'x', 'x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO people (name, kind, archived, created_at) VALUES ('A3', 'owner', 0, 'x')",
                [],
            )
            .unwrap();
            let has_templates = level >= 4;
            if has_templates {
                conn.execute(
                    "INSERT INTO templates (name, title, amount_cents, kind, responsibility, created_at)
                     VALUES ('Rent', 'Rent', 100000, 'expense', 'none', 'x')",
                    [],
                )
                .unwrap();
            }
            run_migrations_with(&mut conn, MIGRATIONS).unwrap();
            let version: i64 = conn
                .pragma_query_value(None, "user_version", |r| r.get(0))
                .unwrap();
            assert_eq!(version, i64::try_from(MIGRATIONS.len()).unwrap());
            let profile = profile_repo::load(&conn).unwrap().unwrap();
            assert_eq!(
                profile.theme,
                Theme::Light,
                "legacy theme kept from v{level}"
            );
            assert_eq!(profile.language, Language::PtBr);
            assert_eq!(profile.start_balance.cents(), 12_345);
            let templates = if has_templates {
                templates_repo::count(&conn).unwrap()
            } else {
                0
            };
            assert_eq!(templates, usize::from(has_templates));
            assert_eq!(load(&conn).unwrap(), None, "no appearance row is invented");
            assert!(integrity::check(&conn).unwrap().is_empty());
        }
        // The legacy profile still rejects anything but dark/light.
        let db = Db::open_in_memory().unwrap();
        let bad = Profile {
            start_balance: crate::domain::money::Money::ZERO,
            theme: Theme::Dark,
            language: Language::En,
            onboarding_complete: true,
        };
        profile_repo::save(db.conn(), &bad).unwrap();
        assert!(
            db.conn()
                .execute("UPDATE profile SET theme = 'system'", [])
                .is_err()
        );
    }

    #[test]
    fn unreadable_row_reports_an_error_without_row_content() {
        let db = Db::open_in_memory().unwrap();
        save(db.conn(), &sample()).unwrap();
        db.conn()
            .execute_batch(
                "PRAGMA ignore_check_constraints = ON;
                 UPDATE appearance SET accent = 'secret-value';
                 PRAGMA ignore_check_constraints = OFF;",
            )
            .unwrap();
        assert!(is_unreadable(db.conn()));
    }
}
