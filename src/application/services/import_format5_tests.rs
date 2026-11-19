//! Format 5 (1.4): goals with contributions and saved closings (contract §14.3, §10.2.3).

use chrono::NaiveDate;
use serde_json::{Value, json};

use super::closings;
use super::import::{ImportPlan, parse_and_validate};
use crate::domain::closing::ClosingError;
use crate::domain::date::YearMonth;
use crate::domain::goal::{Contribution, Frequency, Goal, GoalRecord, GoalStatus};
use crate::domain::money::Money;
use crate::infrastructure::db::{
    Db, appearance_repo, closings_repo, goals_repo, import_repo, integrity,
};
use crate::infrastructure::export::format_v5::DocumentV5;
use crate::infrastructure::export::json::{FORMAT_VERSION, build_document, write_json};
use crate::shared::error::{AppError, ClosingIssue, ContributionIssue, GoalIssue, ImportError};

const V1: &[u8] = include_bytes!("../../../tests/fixtures/format1-1.0.2.json");
const V2: &[u8] = include_bytes!("../../../tests/fixtures/format2-1.1.0.json");
const V3: &[u8] = include_bytes!("../../../tests/fixtures/format3-1.2.0.json");
const V4: &[u8] = include_bytes!("../../../tests/fixtures/format4-1.3.0.json");
const V5: &[u8] = include_bytes!("../../../tests/fixtures/format5-1.4.0.json");
const FIXTURE_PATH: &str = "tests/fixtures/format5-1.4.0.json";
const EXPORTED_AT: &str = "2026-11-19T12:00:00Z";
const STAMP: &str = "2026-11-19T12:00:00Z";
const CREATED: &str = "2026-11-18T12:00:00Z";

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn month(y: i32, m: u32) -> YearMonth {
    YearMonth::new(y, m).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn goal(
    id: i64,
    name: &str,
    target: i64,
    date: Option<NaiveDate>,
    frequency: Frequency,
    attribution: Option<i64>,
    status: GoalStatus,
    pre_archive: Option<GoalStatus>,
) -> Goal {
    Goal {
        id,
        name: name.into(),
        target: Money::from_cents(target),
        target_date: date,
        frequency,
        attribution_id: attribution,
        status,
        pre_archive_status: pre_archive,
        created_at: CREATED.into(),
        updated_at: "2026-11-18T12:30:00Z".into(),
    }
}

fn put(id: i64, goal_id: i64, cents: i64, date: NaiveDate, note: Option<&str>) -> Contribution {
    Contribution {
        id,
        goal_id,
        amount: Money::from_cents(cents),
        date,
        note: note.map(str::to_owned),
        created_at: "2026-11-18T12:10:00Z".into(),
    }
}

/// The format 4 fixture's data plus two more entries, three saved closings (one whose largest
/// expense was purged afterwards) and four goals with contributions: everything nonempty.
fn seeded() -> Db {
    let v4 = parse_and_validate(V4, "format4-1.3.0.json").unwrap();
    let db = Db::open_in_memory().unwrap();
    import_repo::replace(db.conn(), &v4).unwrap();
    let c = db.conn();
    for (id, title, cents, date, category) in [
        (6, "Old sofa", 90_000, "2026-10-12", Some(1)),
        (7, "Bus pass", 2_000, "2026-09-03", None::<i64>),
    ] {
        c.execute(
            "INSERT INTO transactions (id, title, amount_cents, date, kind, category_id, payer_id,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'expense', ?5, 1, ?6, ?6)",
            rusqlite::params![id, title, cents, date, category, CREATED],
        )
        .unwrap();
    }
    let today = day(2026, 12, 1);
    for m in [month(2026, 9), month(2026, 10), month(2026, 11)] {
        closings::close_month(c, m, today, STAMP).unwrap();
    }
    // The October largest expense is purged afterwards: its title and amount stay on the closing.
    crate::infrastructure::db::transactions_repo::delete(c, 6).unwrap();
    crate::infrastructure::db::trash_repo::purge(c, 6).unwrap();

    let records = [
        GoalRecord {
            goal: goal(
                1,
                "Holiday fund",
                500_000,
                Some(day(2027, 3, 1)),
                Frequency::Monthly,
                Some(2),
                GoalStatus::Active,
                None,
            ),
            contributions: vec![
                put(1, 1, 120_000, day(2026, 10, 5), None),
                put(
                    2,
                    1,
                    80_050,
                    day(2026, 11, 2),
                    Some("Bonus, saved in one go.\nSecond line of a long note."),
                ),
            ],
        },
        GoalRecord {
            goal: goal(
                2,
                "Emergency reserve",
                100_000,
                None,
                Frequency::Weekly,
                None,
                GoalStatus::Completed,
                None,
            ),
            contributions: vec![put(3, 2, 100_000, day(2026, 11, 1), Some("Exactly funded"))],
        },
        GoalRecord {
            goal: goal(
                3,
                "Old laptop",
                80_000,
                Some(day(2026, 8, 1)),
                Frequency::Daily,
                Some(3),
                GoalStatus::Archived,
                Some(GoalStatus::Paused),
            ),
            contributions: vec![
                put(4, 3, 50_000, day(2026, 6, 15), None),
                put(
                    5,
                    3,
                    45_000,
                    day(2026, 7, 20),
                    Some("Overfunded before archiving"),
                ),
            ],
        },
        GoalRecord {
            goal: goal(
                4,
                "Paused hobby",
                25_000,
                Some(day(2026, 11, 15)),
                Frequency::Monthly,
                Some(1),
                GoalStatus::Paused,
                None,
            ),
            contributions: Vec::new(),
        },
    ];
    for record in &records {
        goals_repo::insert_imported(c, record).unwrap();
    }
    assert_eq!(integrity::check(c).unwrap(), vec![]);
    db
}

fn fixture_bytes() -> Vec<u8> {
    let db = seeded();
    let profile = crate::infrastructure::db::profile_repo::load(db.conn())
        .unwrap()
        .unwrap();
    let mut doc = build_document(db.conn(), &profile, EXPORTED_AT).unwrap();
    doc.app_version = "1.4.0".into();
    serde_json::to_vec_pretty(&doc).unwrap()
}

/// Regenerates the fixture: `cargo test --lib regenerate_format5_fixture -- --ignored`.
#[test]
#[ignore = "writes tests/fixtures/format5-1.4.0.json"]
fn regenerate_format5_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_PATH);
    std::fs::write(path, fixture_bytes()).unwrap();
}

fn value() -> Value {
    serde_json::from_slice(V5).unwrap()
}

fn errors(doc: &Value) -> Vec<ImportError> {
    match parse_and_validate(&serde_json::to_vec(doc).unwrap(), "t.json") {
        Err(AppError::Import(errs)) => errs,
        other => panic!("expected import errors, got {other:?}"),
    }
}

fn plan_of(bytes: &[u8]) -> ImportPlan {
    parse_and_validate(bytes, "f.json").unwrap()
}

#[test]
fn format_5_fixture_is_lf_and_reproducible_from_its_seed() {
    assert!(
        !V5.windows(2).any(|w| w == b"\r\n"),
        "format5-1.4.0.json must stay pinned to LF"
    );
    assert_eq!(
        String::from_utf8(V5.to_vec()).unwrap(),
        String::from_utf8(fixture_bytes()).unwrap(),
        "regenerate with `cargo test --lib regenerate_format5_fixture -- --ignored`"
    );
}

#[test]
fn the_fixture_has_every_collection_nonempty_in_canonical_order() {
    let doc: DocumentV5 = serde_json::from_slice(V5).unwrap();
    assert_eq!(doc.format_version, 5);
    assert!(!doc.templates.is_empty() && doc.appearance.is_some());
    let ids: Vec<i64> = doc.goals.iter().map(|g| g.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4], "goals are written by id");
    for g in &doc.goals {
        let ids: Vec<i64> = g.contributions.iter().map(|c| c.id).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "contributions by id");
    }
    let months: Vec<&str> = doc.closings.iter().map(|c| c.month.as_str()).collect();
    assert_eq!(months, vec!["2026-09", "2026-10", "2026-11"]);
    // The purged October expense is written as null with its fallback title and amount.
    let october = &doc.closings[1];
    assert_eq!(october.largest_expense_id, None);
    assert_eq!(october.largest_expense_title.as_deref(), Some("Old sofa"));
    assert_eq!(october.largest_expense_cents, Some(90_000));
    // November's largest expense is a live entry of the file; September's winner is Uncategorized.
    assert_eq!(doc.closings[2].largest_expense_id, Some(2));
    assert_eq!(doc.closings[0].top_category_id, None);
    assert_eq!(doc.closings[0].top_category_net_cents, Some(2_000));
}

#[test]
fn the_fixture_round_trips_through_the_database_and_back_to_the_same_bytes() {
    let plan = plan_of(V5);
    assert_eq!(plan.format_version, 5);
    assert_eq!(plan.goals.len(), 4);
    assert_eq!(plan.closings.len(), 3);
    let db = Db::open_in_memory().unwrap();
    import_repo::replace(db.conn(), &plan).unwrap();
    assert_eq!(integrity::check(db.conn()).unwrap(), vec![]);

    let mut from_db = ImportPlan::from_db(db.conn(), &plan.profile).unwrap();
    let mut expected = plan.clone();
    for p in [&mut from_db, &mut expected] {
        p.source_name.clear();
        p.exported_at.clear();
        p.app_version.clear();
    }
    assert_eq!(from_db, expected, "nothing is regenerated on import");

    // Stored fingerprints survive untouched, even though October's data changed since.
    let saved = closings_repo::get(db.conn(), month(2026, 10))
        .unwrap()
        .unwrap();
    assert_eq!(saved, expected.closings[1]);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.json");
    write_json(db.conn(), &plan.profile, EXPORTED_AT, &path).unwrap();
    let mut again: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    again["app_version"] = json!("1.4.0");
    assert_eq!(again, value());
}

#[test]
fn older_formats_import_with_no_goals_or_closings_and_clear_existing_ones() {
    for (bytes, version) in [(V1, 1), (V2, 2), (V3, 3), (V4, 4)] {
        let plan = plan_of(bytes);
        assert_eq!(plan.format_version, version);
        assert!(plan.goals.is_empty() && plan.closings.is_empty());
        let db = seeded();
        assert_eq!(goals_repo::count(db.conn()).unwrap(), 4);
        import_repo::replace(db.conn(), &plan).unwrap();
        assert_eq!(goals_repo::count(db.conn()).unwrap(), 0);
        assert_eq!(closings_repo::count(db.conn()).unwrap(), 0);
        assert_eq!(integrity::check(db.conn()).unwrap(), vec![]);
    }
}

#[test]
fn dispatch_accepts_5_rejects_6_before_parsing_and_requires_the_new_arrays() {
    assert_eq!(FORMAT_VERSION, 5);
    let future = json!({"format": "athena-profile", "format_version": 6, "garbage": [1]});
    assert_eq!(
        errors(&future),
        vec![ImportError::TooNew {
            found: 6,
            supported: 5
        }]
    );
    for key in ["goals", "closings"] {
        let mut doc = value();
        doc.as_object_mut().unwrap().remove(key);
        assert!(matches!(
            errors(&doc).as_slice(),
            [ImportError::MalformedJson(_)]
        ));
    }
    let mut doc = value();
    doc["goals"][0]["surprise"] = json!(1);
    assert!(matches!(
        errors(&doc).as_slice(),
        [ImportError::MalformedJson(_)]
    ));
    let mut doc = value();
    doc["goals"][0]["contributions"][0]["surprise"] = json!(1);
    assert!(matches!(
        errors(&doc).as_slice(),
        [ImportError::MalformedJson(_)]
    ));
    // A format 4 envelope cannot smuggle goals in.
    let mut v4: Value = serde_json::from_slice(V4).unwrap();
    v4["goals"] = json!([]);
    assert!(matches!(
        errors(&v4).as_slice(),
        [ImportError::MalformedJson(_)]
    ));
}

fn goal_issue(id: i64, issue: GoalIssue) -> ImportError {
    ImportError::InvalidGoal { id, issue }
}

#[test]
fn invalid_goals_and_contributions_are_rejected_with_specific_errors() {
    let case = |edit: &dyn Fn(&mut Value)| {
        let mut doc = value();
        edit(&mut doc);
        errors(&doc)
    };
    assert!(
        case(&|d| d["goals"][0]["id"] = json!(0))
            .contains(&goal_issue(0, GoalIssue::NonPositiveId))
    );
    assert!(
        case(&|d| d["goals"][1]["id"] = json!(1)).contains(&goal_issue(1, GoalIssue::DuplicateId))
    );
    assert!(
        case(&|d| d["goals"][0]["name"] = json!("  "))
            .contains(&goal_issue(1, GoalIssue::BlankName))
    );
    assert!(
        case(&|d| d["goals"][0]["target_cents"] = json!(0))
            .contains(&goal_issue(1, GoalIssue::NonPositiveTarget))
    );
    assert!(
        case(&|d| d["goals"][0]["target_date"] = json!("2027-02-30"))
            .contains(&goal_issue(1, GoalIssue::InvalidDate))
    );
    assert!(
        case(&|d| d["goals"][0]["frequency"] = json!("hourly"))
            .contains(&goal_issue(1, GoalIssue::InvalidFrequency))
    );
    assert!(
        case(&|d| d["goals"][0]["status"] = json!("done"))
            .contains(&goal_issue(1, GoalIssue::InvalidStatus))
    );
    // Archive shape: stored only for archived goals, and never archived itself.
    assert!(
        case(&|d| d["goals"][0]["pre_archive_status"] = json!("active"))
            .contains(&goal_issue(1, GoalIssue::ArchiveShape))
    );
    assert!(
        case(&|d| d["goals"][2]["pre_archive_status"] = Value::Null)
            .contains(&goal_issue(3, GoalIssue::ArchiveShape))
    );
    assert!(
        case(&|d| d["goals"][2]["pre_archive_status"] = json!("archived"))
            .contains(&goal_issue(3, GoalIssue::ArchiveShape))
    );
    assert!(
        case(&|d| d["goals"][0]["attribution_id"] = json!(99))
            .contains(&goal_issue(1, GoalIssue::MissingPerson(99)))
    );
    assert!(
        case(&|d| d["goals"][0]["created_at"] = json!("yesterday"))
            .contains(&goal_issue(1, GoalIssue::InvalidTimestamp))
    );

    let contribution = |goal_id, id, issue| ImportError::InvalidContribution { goal_id, id, issue };
    assert!(
        case(&|d| d["goals"][0]["contributions"][0]["id"] = json!(0)).contains(&contribution(
            1,
            0,
            ContributionIssue::NonPositiveId
        ))
    );
    assert!(
        case(&|d| d["goals"][0]["contributions"][1]["id"] = json!(1)).contains(&contribution(
            1,
            1,
            ContributionIssue::DuplicateId
        ))
    );
    assert!(
        case(&|d| d["goals"][0]["contributions"][0]["amount_cents"] = json!(0))
            .contains(&contribution(1, 1, ContributionIssue::NonPositiveAmount))
    );
    assert!(
        case(&|d| d["goals"][0]["contributions"][0]["date"] = json!("2026-13-01"))
            .contains(&contribution(1, 1, ContributionIssue::InvalidDate))
    );
    assert!(
        case(&|d| d["goals"][0]["contributions"][0]["created_at"] = json!(""))
            .contains(&contribution(1, 1, ContributionIssue::InvalidTimestamp))
    );
    // Totals that cannot be represented are an overflow, never a wrapped sum.
    let overflow = case(&|d| {
        d["goals"][0]["contributions"][0]["amount_cents"] = json!(i64::MAX);
        d["goals"][0]["contributions"][1]["amount_cents"] = json!(1);
    });
    assert!(overflow.contains(&ImportError::Overflow));
    // A past contribution date is history, whatever the clock says.
    let mut old = value();
    old["goals"][0]["contributions"][0]["date"] = json!("2001-01-01");
    assert!(parse_and_validate(&serde_json::to_vec(&old).unwrap(), "t.json").is_ok());
}

#[test]
fn invalid_closings_are_rejected_without_regeneration() {
    let case = |edit: &dyn Fn(&mut Value)| {
        let mut doc = value();
        edit(&mut doc);
        errors(&doc)
    };
    let closing = |m: &str, issue| ImportError::InvalidClosing {
        month: m.into(),
        issue,
    };
    assert!(
        case(&|d| d["closings"][0]["month"] = json!("2026-13"))
            .contains(&closing("2026-13", ClosingIssue::InvalidMonth))
    );
    assert!(
        case(&|d| d["closings"][0]["month"] = json!("2026-9"))
            .contains(&closing("2026-9", ClosingIssue::InvalidMonth))
    );
    assert!(
        case(&|d| d["closings"][1]["month"] = json!("2026-09"))
            .contains(&closing("2026-09", ClosingIssue::DuplicateMonth))
    );
    assert!(
        case(&|d| d["closings"][2]["result_cents"] = json!(1))
            .contains(&closing("2026-11", ClosingIssue::Inconsistent))
    );
    assert!(
        case(&|d| d["closings"][2]["entry_count"] = json!(-1))
            .contains(&closing("2026-11", ClosingIssue::Inconsistent))
    );
    assert!(
        case(&|d| d["closings"][2]["current_fingerprint"] = json!("ABC"))
            .contains(&closing("2026-11", ClosingIssue::Inconsistent))
    );
    assert!(
        case(&|d| d["closings"][2]["previous_fingerprint"] = json!("G".repeat(64)))
            .contains(&closing("2026-11", ClosingIssue::Inconsistent))
    );
    assert!(
        case(&|d| d["closings"][2]["fingerprint_version"] = json!(2))
            .contains(&closing("2026-11", ClosingIssue::Inconsistent))
    );
    assert!(
        case(&|d| d["closings"][2]["top_category_net_cents"] = json!(0))
            .contains(&closing("2026-11", ClosingIssue::Inconsistent))
    );
    assert!(
        case(&|d| d["closings"][2]["top_category_net_cents"] = Value::Null)
            .contains(&closing("2026-11", ClosingIssue::Inconsistent))
    );
    assert!(
        case(&|d| d["closings"][2]["largest_expense_cents"] = Value::Null)
            .contains(&closing("2026-11", ClosingIssue::Inconsistent))
    );
    assert!(
        case(&|d| d["closings"][2]["closed_at"] = json!("soon"))
            .contains(&closing("2026-11", ClosingIssue::InvalidTimestamp))
    );
    // Dangling references are rejected, not repaired.
    assert!(
        case(&|d| d["closings"][2]["top_category_id"] = json!(999))
            .contains(&closing("2026-11", ClosingIssue::MissingCategory(999)))
    );
    assert!(
        case(&|d| d["closings"][2]["largest_expense_id"] = json!(999))
            .contains(&closing("2026-11", ClosingIssue::MissingExpense(999)))
    );
    // Overflowing figures are an overflow.
    let big = case(&|d| {
        d["closings"][2]["income_cents"] = json!(i64::MAX);
        d["closings"][2]["expenses_cents"] = json!(0);
        d["closings"][2]["refunds_cents"] = json!(i64::MAX);
    });
    assert!(big.contains(&ImportError::Overflow));
    // A fingerprint the file carries is kept exactly: it is never recomputed from the data.
    let mut kept = value();
    kept["closings"][2]["current_fingerprint"] = json!("0".repeat(64));
    let plan = plan_of(&serde_json::to_vec(&kept).unwrap());
    assert_eq!(plan.closings[2].current_fingerprint, "0".repeat(64));
}

#[test]
fn a_missing_live_largest_expense_is_exported_as_null_with_its_fallback() {
    let db = seeded();
    // Trash the live November largest expense: it is no longer in the live export.
    let before = closings_repo::get(db.conn(), month(2026, 11))
        .unwrap()
        .unwrap();
    let id = before.largest_expense.as_ref().unwrap().id.unwrap();
    crate::infrastructure::db::transactions_repo::delete(db.conn(), id).unwrap();
    let profile = crate::infrastructure::db::profile_repo::load(db.conn())
        .unwrap()
        .unwrap();
    let doc = build_document(db.conn(), &profile, EXPORTED_AT).unwrap();
    let november = doc.closings.iter().find(|c| c.month == "2026-11").unwrap();
    assert_eq!(november.largest_expense_id, None);
    assert_eq!(
        november.largest_expense_title,
        before.largest_expense.as_ref().map(|e| e.title.clone())
    );
    assert_eq!(
        november.largest_expense_cents,
        before.largest_expense.as_ref().map(|e| e.amount.cents())
    );
    // The exported document imports cleanly and keeps the saved figures and fingerprints.
    let bytes = serde_json::to_vec(&doc).unwrap();
    let plan = plan_of(&bytes);
    let fresh = Db::open_in_memory().unwrap();
    import_repo::replace(fresh.conn(), &plan).unwrap();
    let kept = closings_repo::get(fresh.conn(), month(2026, 11))
        .unwrap()
        .unwrap();
    assert_eq!(kept.current_fingerprint, before.current_fingerprint);
    assert_eq!(kept.largest_expense.as_ref().unwrap().id, None);
    assert_eq!(integrity::check(fresh.conn()).unwrap(), vec![]);
}

#[test]
fn a_failed_replacement_keeps_goals_closings_templates_appearance_and_financial_data() {
    let db = seeded();
    appearance_repo::save(
        db.conn(),
        &crate::domain::appearance::AppearancePrefs::default(),
    )
    .unwrap();
    let profile = crate::infrastructure::db::profile_repo::load(db.conn())
        .unwrap()
        .unwrap();
    let before = ImportPlan::from_db(db.conn(), &profile).unwrap();
    assert!(
        !before.templates.is_empty() && !before.goals.is_empty() && !before.closings.is_empty()
    );
    let incoming = plan_of(V5);
    // The hook fails after the old rows were cleared and the new people written.
    let failed = import_repo::replace_with(db.conn(), &incoming, |_| {
        Err(AppError::Validation("injected failure".into()))
    });
    assert!(failed.is_err());
    let after = ImportPlan::from_db(db.conn(), &profile).unwrap();
    assert_eq!(after, before);
    assert_eq!(
        appearance_repo::load(db.conn()).unwrap(),
        Some(crate::domain::appearance::AppearancePrefs::default())
    );
    assert_eq!(integrity::check(db.conn()).unwrap(), vec![]);
    // A snapshot that fails verification (inconsistent closing smuggled past validation) also
    // rolls everything back.
    let mut bad = incoming.clone();
    bad.closings[0].result = Money::from_cents(1);
    assert!(import_repo::replace(db.conn(), &bad).is_err());
    assert_eq!(ImportPlan::from_db(db.conn(), &profile).unwrap(), before);
}

#[test]
fn closing_actions_refuse_after_an_import_replaces_the_months() {
    // The same month cannot hold two closings, whether created or imported.
    let db = seeded();
    assert!(matches!(
        closings::close_month(db.conn(), month(2026, 10), day(2026, 12, 1), STAMP),
        Err(AppError::ClosingInvalid(ClosingError::AlreadyClosed))
    ));
    assert!(
        db.conn()
            .execute(
                "INSERT INTO closings SELECT * FROM closings WHERE month = '2026-10'",
                []
            )
            .is_err(),
        "the month is the primary key"
    );
}
