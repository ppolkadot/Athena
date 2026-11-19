//! Format 4 (1.3): portable appearance (contract §14.2, §10.2.2).

use serde_json::{Value, json};

use super::import::{ImportPlan, parse_and_validate};
use crate::domain::appearance::{
    Accent, AppearancePrefs, AssetId, Density, Fit, MotionPref, Preset, Quality, TextScale,
    ThemeMode, Wallpaper,
};
use crate::infrastructure::db::{Db, appearance_repo, import_repo, integrity};
use crate::infrastructure::export::format_v4::DocumentV4;
use crate::infrastructure::export::format_v5::DocumentV5;
use crate::infrastructure::export::json::{FORMAT_VERSION, write_json};
use crate::shared::error::{AppError, ImportError};

const V3: &[u8] = include_bytes!("../../../tests/fixtures/format3-1.2.0.json");
const V4: &[u8] = include_bytes!("../../../tests/fixtures/format4-1.3.0.json");

fn fixture() -> Value {
    serde_json::from_slice(V4).unwrap()
}

fn errors(doc: &Value) -> Vec<ImportError> {
    match parse_and_validate(&serde_json::to_vec(doc).unwrap(), "t.json") {
        Err(AppError::Import(errs)) => errs,
        other => panic!("expected import errors, got {other:?}"),
    }
}

fn fixture_prefs() -> AppearancePrefs {
    AppearancePrefs {
        theme_mode: ThemeMode::System,
        preset: Preset::Aurora,
        accent: Accent::Teal,
        density: Density::Compact,
        text_scale: TextScale::P130,
        motion: MotionPref::Reduced,
        reduce_transparency: true,
        quality: Quality::Low,
        wallpaper: Wallpaper::Builtin,
        fit: Fit::Fit,
        dim: 55,
        paused: true,
    }
}

fn local_prefs() -> AppearancePrefs {
    AppearancePrefs {
        wallpaper: Wallpaper::Local(AssetId::parse("0123456789abcdef0123456789abcdef").unwrap()),
        fit: Fit::Fit,
        dim: 70,
        paused: true,
        ..AppearancePrefs::default()
    }
}

#[test]
fn format_4_fixture_is_lf_canonical_and_non_default() {
    assert!(
        !V4.windows(2).any(|w| w == b"\r\n"),
        "format4-1.3.0.json must stay pinned to LF"
    );
    let doc: DocumentV4 = serde_json::from_slice(V4).unwrap();
    assert_eq!(doc.format_version, 4);
    assert_ne!(
        doc.appearance.as_ref().unwrap().theme_mode,
        AppearancePrefs::default().theme_mode.code()
    );
    assert_eq!(
        String::from_utf8(serde_json::to_vec_pretty(&doc).unwrap()).unwrap(),
        String::from_utf8(V4.to_vec()).unwrap(),
        "the fixture is exactly what the writer produces"
    );
    let plan = parse_and_validate(V4, "format4-1.3.0.json").unwrap();
    assert_eq!(plan.format_version, 4);
    assert_eq!(plan.appearance, Some(fixture_prefs()));
    assert_eq!(plan.templates.len(), 5, "the V3 collections are all there");
}

#[test]
fn the_fixture_round_trips_through_the_database_and_back_to_the_same_bytes() {
    let plan = parse_and_validate(V4, "f.json").unwrap();
    let db = Db::open_in_memory().unwrap();
    import_repo::replace(db.conn(), &plan).unwrap();
    assert!(integrity::check(db.conn()).unwrap().is_empty());
    assert_eq!(
        appearance_repo::load(db.conn()).unwrap(),
        Some(fixture_prefs())
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.json");
    write_json(db.conn(), &plan.profile, &plan.exported_at, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let doc: DocumentV5 = serde_json::from_str(&text).unwrap();
    assert_eq!(doc.format_version, FORMAT_VERSION);
    assert!(doc.goals.is_empty() && doc.closings.is_empty());
    // The current writer adds only the format 5 envelope; every format 4 field is reproduced.
    let mut again: Value = serde_json::from_str(&text).unwrap();
    again["app_version"] = json!("1.3.0");
    again["format_version"] = json!(4);
    again.as_object_mut().unwrap().remove("goals");
    again.as_object_mut().unwrap().remove("closings");
    assert_eq!(
        again,
        fixture(),
        "export reproduces the fixture apart from the app version"
    );

    let mut from_db = ImportPlan::from_db(db.conn(), &plan.profile).unwrap();
    from_db.format_version = plan.format_version;
    let mut expected = plan;
    from_db.source_name.clear();
    expected.source_name.clear();
    from_db.exported_at.clear();
    expected.exported_at.clear();
    from_db.app_version.clear();
    expected.app_version.clear();
    assert_eq!(from_db, expected);
}

#[test]
fn a_local_wallpaper_is_exported_as_none_without_ids_paths_or_bytes() {
    let plan = parse_and_validate(V4, "f.json").unwrap();
    let db = Db::open_in_memory().unwrap();
    import_repo::replace(db.conn(), &plan).unwrap();
    appearance_repo::save(db.conn(), &local_prefs()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.json");
    write_json(db.conn(), &plan.profile, &plan.exported_at, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("0123456789abcdef"),
        "no asset id is exported"
    );
    assert!(!text.contains(".media") && !text.contains("appearance\\"));
    let doc: DocumentV5 = serde_json::from_str(&text).unwrap();
    let appearance = doc.appearance.unwrap();
    assert_eq!(appearance.background, "none");
    assert_eq!(
        (appearance.fit.as_str(), appearance.dim, appearance.paused),
        ("fit", 70, true)
    );
    // Importing that document restores the portable choice with no wallpaper.
    let back = parse_and_validate(text.as_bytes(), "o.json")
        .unwrap()
        .appearance
        .unwrap();
    assert_eq!(back, local_prefs().portable());
}

#[test]
fn formats_1_to_3_and_a_format_4_without_appearance_clear_the_stored_row() {
    let plan = parse_and_validate(V3, "v3.json").unwrap();
    assert_eq!(plan.appearance, None, "format 3 carries no appearance");
    let db = Db::open_in_memory().unwrap();
    import_repo::replace(db.conn(), &plan).unwrap();
    appearance_repo::save(db.conn(), &fixture_prefs()).unwrap();
    import_repo::replace(db.conn(), &plan).unwrap();
    assert_eq!(appearance_repo::load(db.conn()).unwrap(), None);

    for fixture in [
        &include_bytes!("../../../tests/fixtures/format1-1.0.2.json")[..],
        &include_bytes!("../../../tests/fixtures/format2-1.1.0.json")[..],
    ] {
        let legacy = parse_and_validate(fixture, "legacy.json").unwrap();
        assert_eq!(legacy.appearance, None);
        appearance_repo::save(db.conn(), &fixture_prefs()).unwrap();
        import_repo::replace(db.conn(), &legacy).unwrap();
        assert_eq!(appearance_repo::load(db.conn()).unwrap(), None);
    }

    let mut omitted = fixture();
    omitted.as_object_mut().unwrap().remove("appearance");
    let plan = parse_and_validate(&serde_json::to_vec(&omitted).unwrap(), "o.json").unwrap();
    assert_eq!(plan.appearance, None);
    appearance_repo::save(db.conn(), &fixture_prefs()).unwrap();
    import_repo::replace(db.conn(), &plan).unwrap();
    assert_eq!(appearance_repo::load(db.conn()).unwrap(), None);
    assert!(integrity::check(db.conn()).unwrap().is_empty());
}

#[test]
fn malformed_enums_and_ranges_abort_the_import() {
    let with = |field: &str, new: Value| {
        let mut doc = fixture();
        doc["appearance"][field] = new;
        doc
    };
    for (field, bad) in [
        ("theme_mode", json!("auto")),
        ("preset", json!("glass")),
        ("accent", json!("red")),
        ("density", json!("tight")),
        ("motion", json!("off")),
        ("quality", json!("ultra")),
        ("fit", json!("stretch")),
        ("background", json!("local")),
        ("background", json!("glass")),
        ("text_scale", json!(120)),
        ("text_scale", json!(-100)),
        ("dim", json!(101)),
        ("dim", json!(-1)),
        ("dim", json!(1_000_000_000_000_i64)),
    ] {
        assert_eq!(
            errors(&with(field, bad.clone())),
            vec![ImportError::InvalidAppearance],
            "{field} = {bad}"
        );
    }
    // A null or wrongly typed value is a parse error, which also aborts the import.
    for (field, bad) in [
        ("theme_mode", Value::Null),
        ("dim", json!("55")),
        ("paused", json!(1)),
    ] {
        let errs = errors(&with(field, bad.clone()));
        assert!(
            matches!(errs.as_slice(), [ImportError::MalformedJson(_)]),
            "{field} = {bad}"
        );
    }
    // Boundaries that are valid.
    for (field, ok) in [
        ("dim", json!(0)),
        ("dim", json!(100)),
        ("text_scale", json!(150)),
    ] {
        let doc = with(field, ok);
        assert!(parse_and_validate(&serde_json::to_vec(&doc).unwrap(), "ok.json").is_ok());
    }
}

#[test]
fn unknown_and_future_fields_are_rejected() {
    let mut doc = fixture();
    doc["appearance"]["glass"] = json!(true);
    assert!(matches!(
        errors(&doc).as_slice(),
        [ImportError::MalformedJson(_)]
    ));
    let mut doc = fixture();
    doc["wallpaper_path"] = json!("C:/secret/pic.png");
    assert!(matches!(
        errors(&doc).as_slice(),
        [ImportError::MalformedJson(_)]
    ));
    let mut doc = fixture();
    doc["appearance"]["asset_id"] = json!("0123456789abcdef0123456789abcdef");
    assert!(
        matches!(errors(&doc).as_slice(), [ImportError::MalformedJson(_)]),
        "asset ids are never part of a document"
    );
    // A format above the current one is rejected before its body is read.
    let future = json!({"format": "athena-profile", "format_version": 6, "appearance": 1});
    assert_eq!(
        errors(&future),
        vec![ImportError::TooNew {
            found: 6,
            supported: FORMAT_VERSION
        }]
    );
}

#[test]
fn a_failed_import_keeps_an_existing_local_wallpaper_reference() {
    let plan = parse_and_validate(V4, "f.json").unwrap();
    let db = Db::open_in_memory().unwrap();
    import_repo::replace(db.conn(), &parse_and_validate(V3, "v3.json").unwrap()).unwrap();
    appearance_repo::save(db.conn(), &local_prefs()).unwrap();
    let before = appearance_repo::load(db.conn()).unwrap();
    // A failure after the appearance write point rolls everything back, including the row.
    let result = import_repo::replace_with(db.conn(), &plan, |_| {
        Err(AppError::Import(vec![ImportError::VerificationFailed(
            "injected".into(),
        )]))
    });
    assert!(result.is_err());
    assert_eq!(appearance_repo::load(db.conn()).unwrap(), before);
    assert!(integrity::check(db.conn()).unwrap().is_empty());
}
