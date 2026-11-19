//! Format 3 (1.2) import/export regressions: templates round-trip, validation, rollback and the
//! shipped `format3-1.2.0.json` fixture (contract §10.2.1, §14.1).

use serde_json::{Value, json};

use super::import::{ImportPlan, parse_and_validate};
use crate::domain::money::Money;
use crate::domain::settings::Profile;
use crate::domain::template::{ResponsibilityMode, Template, TemplateDraft};
use crate::domain::transaction::{Allocation, TransactionType};
use crate::infrastructure::db::{Db, import_repo, integrity, templates_repo};
use crate::infrastructure::export::format_v3::DocumentV3;
use crate::infrastructure::export::format_v5::DocumentV5;
use crate::infrastructure::export::json::{FORMAT_VERSION, build_document, write_json};
use crate::shared::error::{AppError, ImportError};

const V1: &[u8] = include_bytes!("../../../tests/fixtures/format1-1.0.2.json");
const V2: &[u8] = include_bytes!("../../../tests/fixtures/format2-1.1.0.json");
const V3: &[u8] = include_bytes!("../../../tests/fixtures/format3-1.2.0.json");
const FIXTURE_PATH: &str = "tests/fixtures/format3-1.2.0.json";
const EXPORTED_AT: &str = "2026-11-18T12:00:00Z";

fn share(person_id: i64, cents: i64) -> Allocation {
    Allocation {
        person_id,
        amount: Money::from_cents(cents),
    }
}

fn template(id: i64, draft: TemplateDraft, created: &str, used: Option<&str>) -> Template {
    Template {
        id,
        draft,
        created_at: created.into(),
        last_used_at: used.map(str::to_string),
    }
}

/// The format 3 fixture's data: the format 2 fixture plus five templates covering every
/// responsibility mode, explicit zero shares, archived references and `last_used_at`.
fn seeded() -> (Db, Profile) {
    let v2 = parse_and_validate(V2, "format2-1.1.0.json").unwrap();
    let db = Db::open_in_memory().unwrap();
    import_repo::replace(db.conn(), &v2).unwrap();
    let draft = |name: &str, title: &str, cents: i64, kind, category, payer, mode, allocations| {
        TemplateDraft {
            name: name.into(),
            title: title.into(),
            amount: Money::from_cents(cents),
            kind,
            category_id: category,
            payer_id: payer,
            description: None,
            responsibility: mode,
            allocations,
        }
    };
    let mut weekly = draft(
        "Weekly groceries",
        "Groceries",
        10_001,
        TransactionType::Expense,
        Some(1),
        Some(1),
        ResponsibilityMode::Even,
        vec![share(1, 5_001), share(2, 5_000)],
    );
    weekly.description = Some("Every Saturday".into());
    let rows = [
        template(
            1,
            weekly,
            "2026-11-18T11:55:00Z",
            Some("2026-11-18T11:58:00Z"),
        ),
        template(
            2,
            draft(
                "Dinner with Bia",
                "Dinner",
                4_000,
                TransactionType::Expense,
                Some(13),
                Some(3),
                ResponsibilityMode::Custom,
                vec![share(1, 1_500), share(2, 0), share(3, 2_500)],
            ),
            "2026-11-18T11:56:00Z",
            None,
        ),
        template(
            3,
            draft(
                "Salary",
                "Paycheck",
                300_000,
                TransactionType::Income,
                Some(11),
                None,
                ResponsibilityMode::None,
                vec![],
            ),
            "2026-11-18T11:57:00Z",
            None,
        ),
        template(
            4,
            draft(
                "Coffee",
                "Coffee",
                900,
                TransactionType::Expense,
                None,
                Some(2),
                ResponsibilityMode::Person,
                vec![share(2, 900)],
            ),
            "2026-11-18T11:57:30Z",
            Some("2026-11-18T11:59:00Z"),
        ),
        template(
            5,
            draft(
                "Store refund",
                "Refund",
                2_500,
                TransactionType::Refund,
                Some(1),
                Some(1),
                ResponsibilityMode::None,
                vec![],
            ),
            "2026-11-18T11:58:00Z",
            None,
        ),
    ];
    for t in &rows {
        templates_repo::insert_imported(db.conn(), t).unwrap();
    }
    (db, v2.profile)
}

/// Exactly the bytes `write_json` produces, with the release `app_version` pinned.
fn fixture_bytes() -> Vec<u8> {
    let (db, profile) = seeded();
    // The frozen format-3 shape: today's document without the format-4 appearance section.
    let doc = build_document(db.conn(), &profile, EXPORTED_AT).unwrap();
    let v3 = DocumentV3 {
        format: doc.format,
        format_version: 3,
        app_version: "1.2.0".into(),
        exported_at: doc.exported_at,
        profile: doc.profile,
        people: doc.people,
        categories: doc.categories,
        transactions: doc.transactions,
        templates: doc.templates,
    };
    serde_json::to_vec_pretty(&v3).unwrap()
}

/// Regenerates `tests/fixtures/format3-1.2.0.json`:
/// `cargo test --lib regenerate_format3_fixture -- --ignored`.
#[test]
#[ignore = "writes tests/fixtures/format3-1.2.0.json"]
fn regenerate_format3_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_PATH);
    std::fs::write(path, fixture_bytes()).unwrap();
}

fn value() -> Value {
    serde_json::from_slice(V3).unwrap()
}

fn errors(doc: &Value) -> Vec<ImportError> {
    match parse_and_validate(&serde_json::to_vec(doc).unwrap(), "t.json") {
        Err(AppError::Import(errs)) => errs,
        other => panic!("expected import errors, got {other:?}"),
    }
}

#[test]
fn format_3_fixture_is_lf_and_reproducible_from_its_seed() {
    assert!(
        !V3.windows(2).any(|w| w == b"\r\n"),
        "format3-1.2.0.json must stay pinned to LF"
    );
    assert_eq!(
        String::from_utf8(V3.to_vec()).unwrap(),
        String::from_utf8(fixture_bytes()).unwrap(),
        "regenerate with `cargo test --lib regenerate_format3_fixture -- --ignored`"
    );
}

#[test]
fn format_3_exports_templates_and_allocations_with_deny_unknown_fields() {
    let (db, profile) = seeded();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v3.json");
    write_json(db.conn(), &profile, EXPORTED_AT, &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let doc: DocumentV5 = serde_json::from_str(&text).unwrap();
    assert_eq!(doc.format_version, FORMAT_VERSION);
    assert_eq!(doc.templates.len(), 5);
    let ids: Vec<i64> = doc.templates.iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![1, 2, 3, 4, 5], "templates are written by id");
    let dinner = &doc.templates[1];
    assert_eq!(dinner.responsibility, "custom");
    assert_eq!(dinner.payer_id, Some(3));
    assert_eq!(dinner.category_id, Some(13));
    assert_eq!(
        dinner
            .allocations
            .iter()
            .map(|a| (a.person_id, a.amount_cents))
            .collect::<Vec<_>>(),
        vec![(1, 1_500), (2, 0), (3, 2_500)]
    );
    assert_eq!(
        doc.templates[0].last_used_at.as_deref(),
        Some("2026-11-18T11:58:00Z")
    );

    // Round trip back through the importer and into a fresh database.
    let mut plan = parse_and_validate(text.as_bytes(), "v3.json").unwrap();
    assert_eq!(plan.format_version, FORMAT_VERSION);
    let fresh = Db::open_in_memory().unwrap();
    import_repo::replace(fresh.conn(), &plan).unwrap();
    assert!(integrity::check(fresh.conn()).unwrap().is_empty());
    let mut from_db = ImportPlan::from_db(fresh.conn(), &plan.profile).unwrap();
    plan.source_name.clear();
    plan.exported_at.clear();
    plan.app_version.clear();
    from_db.app_version.clear();
    assert_eq!(plan, from_db);

    // Unknown fields are rejected at every level.
    let mut extra = value();
    extra["templates"][0]["surprise"] = json!(1);
    assert!(matches!(
        errors(&extra).as_slice(),
        [ImportError::MalformedJson(_)]
    ));
    let mut extra = value();
    extra["templates"][0]["allocations"][0]["surprise"] = json!(1);
    assert!(matches!(
        errors(&extra).as_slice(),
        [ImportError::MalformedJson(_)]
    ));
    let mut missing = value();
    missing.as_object_mut().unwrap().remove("templates");
    assert!(matches!(
        errors(&missing).as_slice(),
        [ImportError::MalformedJson(_)]
    ));
}

#[test]
fn explicit_dispatch_accepts_v1_v2_v3_and_rejects_v5() {
    for (bytes, version) in [(V1, 1), (V2, 2), (V3, 3)] {
        let plan = parse_and_validate(bytes, "f.json").unwrap();
        assert_eq!(plan.format_version, version);
        let db = Db::open_in_memory().unwrap();
        import_repo::replace(db.conn(), &plan).unwrap();
        assert!(integrity::check(db.conn()).unwrap().is_empty());
    }
    // Older fixtures default to no templates; the v3 fixture carries five.
    assert!(
        parse_and_validate(V1, "f.json")
            .unwrap()
            .templates
            .is_empty()
    );
    assert!(
        parse_and_validate(V2, "f.json")
            .unwrap()
            .templates
            .is_empty()
    );
    assert_eq!(parse_and_validate(V3, "f.json").unwrap().templates.len(), 5);

    // A version above the current one is rejected before the body is parsed.
    let future = json!({"format": "athena-profile", "format_version": 6, "garbage": [1]});
    assert_eq!(
        errors(&future),
        vec![ImportError::TooNew {
            found: 6,
            supported: FORMAT_VERSION
        }]
    );
    // A format 2 envelope cannot smuggle templates in.
    let mut v2: Value = serde_json::from_slice(V2).unwrap();
    v2["templates"] = json!([]);
    assert!(matches!(
        errors(&v2).as_slice(),
        [ImportError::MalformedJson(_)]
    ));
}

fn set(doc: &mut Value, index: usize, key: &str, v: Value) {
    doc["templates"][index][key] = v;
}

type Mutation = Box<dyn Fn(&mut Value)>;

#[test]
#[allow(clippy::too_many_lines)]
fn invalid_templates_are_reported_with_their_ids() {
    let cases: Vec<(&str, Mutation, ImportError)> = vec![
        (
            "non-positive id",
            Box::new(|d| set(d, 0, "id", json!(0))),
            ImportError::NonPositiveTemplateId { index: 0, id: 0 },
        ),
        (
            "duplicate id",
            Box::new(|d| set(d, 1, "id", json!(1))),
            ImportError::DuplicateTemplateId { id: 1 },
        ),
        (
            "blank name",
            Box::new(|d| set(d, 0, "name", json!("   "))),
            ImportError::BlankTemplateName { id: 1 },
        ),
        (
            "duplicate folded name",
            Box::new(|d| set(d, 1, "name", json!("  WEEKLY GROCERIES "))),
            ImportError::DuplicateTemplateName {
                name: "  WEEKLY GROCERIES ".into(),
            },
        ),
        (
            "blank title",
            Box::new(|d| set(d, 0, "title", json!(""))),
            ImportError::BlankTemplateTitle { id: 1 },
        ),
        (
            "zero amount",
            Box::new(|d| set(d, 3, "amount_cents", json!(0))),
            ImportError::NonPositiveTemplateAmount { id: 4 },
        ),
        (
            "settlement template",
            Box::new(|d| set(d, 0, "kind", json!("settlement"))),
            ImportError::InvalidTemplateKind { id: 1 },
        ),
        (
            "unknown responsibility",
            Box::new(|d| set(d, 0, "responsibility", json!("all"))),
            ImportError::InvalidTemplateResponsibility { id: 1 },
        ),
        (
            "missing category",
            Box::new(|d| set(d, 0, "category_id", json!(99))),
            ImportError::TemplateMissingCategory {
                id: 1,
                category_id: 99,
            },
        ),
        (
            "missing payer",
            Box::new(|d| set(d, 0, "payer_id", json!(99))),
            ImportError::TemplateMissingPayer {
                id: 1,
                payer_id: 99,
            },
        ),
        (
            "missing participant",
            Box::new(|d| d["templates"][0]["allocations"][1]["person_id"] = json!(99)),
            ImportError::TemplateMissingPerson {
                id: 1,
                person_id: 99,
            },
        ),
        (
            "bad timestamp",
            Box::new(|d| set(d, 0, "last_used_at", json!("yesterday"))),
            ImportError::InvalidTemplateTimestamp { id: 1 },
        ),
        (
            "shares do not add up",
            Box::new(|d| d["templates"][1]["allocations"][0]["amount_cents"] = json!(1_499)),
            ImportError::InvalidTemplateShape { id: 2 },
        ),
        (
            "negative share",
            Box::new(|d| {
                d["templates"][1]["allocations"][1]["amount_cents"] = json!(-1);
                d["templates"][1]["allocations"][0]["amount_cents"] = json!(1_501);
            }),
            ImportError::InvalidTemplateShape { id: 2 },
        ),
        (
            "duplicate participant",
            Box::new(|d| d["templates"][0]["allocations"][1]["person_id"] = json!(1)),
            ImportError::InvalidTemplateShape { id: 1 },
        ),
        (
            "none with shares",
            Box::new(|d| set(d, 4, "responsibility", json!("none"))),
            ImportError::InvalidTemplateShape { id: 5 },
        ),
        (
            "non-canonical even split",
            Box::new(|d| {
                d["templates"][0]["allocations"][0]["amount_cents"] = json!(5_000);
                d["templates"][0]["allocations"][1]["amount_cents"] = json!(5_001);
            }),
            ImportError::InvalidTemplateShape { id: 1 },
        ),
        (
            "income with a payer",
            Box::new(|d| set(d, 2, "payer_id", json!(1))),
            ImportError::InvalidTemplateShape { id: 3 },
        ),
        (
            "person with two shares",
            Box::new(|d| set(d, 3, "responsibility", json!("even"))),
            ImportError::InvalidTemplateShape { id: 4 },
        ),
    ];
    for (label, mutate, expected) in cases {
        let mut doc = value();
        // Case-specific setup: template 5 starts as a refund with no shares.
        if label == "none with shares" {
            doc["templates"][4]["allocations"] = json!([{"person_id": 1, "amount_cents": 2_500}]);
        }
        mutate(&mut doc);
        let found = errors(&doc);
        assert!(
            found.contains(&expected),
            "{label}: expected {expected:?} in {found:?}"
        );
    }
}

#[test]
fn template_share_overflow_is_an_error_not_a_panic() {
    let mut doc = value();
    doc["templates"][1]["amount_cents"] = json!(i64::MAX);
    doc["templates"][1]["allocations"] = json!([
        {"person_id": 1, "amount_cents": i64::MAX},
        {"person_id": 2, "amount_cents": i64::MAX},
        {"person_id": 3, "amount_cents": i64::MAX},
    ]);
    assert!(errors(&doc).contains(&ImportError::InvalidTemplateShape { id: 2 }));
}

type TableCounts = Vec<(String, Vec<String>)>;

/// Every table's rows, rendered as text, in stable order.
fn dump(db: &Db) -> TableCounts {
    let c = db.conn();
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|t| {
            let cols = c
                .prepare(&format!("SELECT * FROM {t}"))
                .unwrap()
                .column_count();
            let rows: Vec<String> = c
                .prepare(&format!("SELECT * FROM {t} ORDER BY 1, 2"))
                .unwrap()
                .query_map([], |r| {
                    Ok((0..cols)
                        .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            (t, rows)
        })
        .collect()
}

#[test]
fn failed_template_import_preserves_every_existing_table() {
    let plan = parse_and_validate(V3, "f.json").unwrap();

    // Failure injected after people are inserted, with existing templates in the database.
    let (existing, _) = seeded();
    let before = dump(&existing);
    let err = import_repo::replace_with(existing.conn(), &plan, |_| {
        Err(AppError::Validation("injected".into()))
    })
    .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    assert_eq!(dump(&existing), before, "rolled back to the prior data");
    assert!(integrity::check(existing.conn()).unwrap().is_empty());

    // A template row that violates a constraint also rolls everything back.
    let mut bad = plan.clone();
    bad.templates[0].draft.allocations[0].person_id = 999;
    let err = import_repo::replace(existing.conn(), &bad).unwrap_err();
    assert!(matches!(err, AppError::Import(_)), "{err:?}");
    assert_eq!(dump(&existing), before);

    // A successful import replaces templates entirely (no leftovers from the old data).
    let empty_templates = parse_and_validate(V2, "f2.json").unwrap();
    import_repo::replace(existing.conn(), &empty_templates).unwrap();
    assert_eq!(templates_repo::count(existing.conn()).unwrap(), 0);
    let allocations: i64 = existing
        .conn()
        .query_row("SELECT COUNT(*) FROM template_allocations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(allocations, 0);
    assert!(integrity::check(existing.conn()).unwrap().is_empty());
}

#[test]
fn archived_references_and_scope_drift_stay_portable() {
    let (db, profile) = seeded();
    // A category changes scope after the template was saved: it stays exportable and importable.
    db.conn()
        .execute(
            "UPDATE categories SET applies_to = 'income' WHERE id = 1",
            [],
        )
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drift.json");
    write_json(db.conn(), &profile, EXPORTED_AT, &path).unwrap();
    let plan = crate::application::services::import::read_and_validate(&path).unwrap();
    assert_eq!(plan.templates.len(), 5);
    let drifted = plan.templates.iter().find(|t| t.id == 1).unwrap();
    assert_eq!(drifted.draft.category_id, Some(1));
    // Archived person/category references are kept verbatim.
    let archived = plan.templates.iter().find(|t| t.id == 2).unwrap();
    assert_eq!(archived.draft.payer_id, Some(3));
    assert_eq!(archived.draft.category_id, Some(13));
    let fresh = Db::open_in_memory().unwrap();
    import_repo::replace(fresh.conn(), &plan).unwrap();
    assert!(integrity::check(fresh.conn()).unwrap().is_empty());
    // Applying it still produces a valid entry draft: repair is a UI/save-time concern.
    let entry = templates_repo::get(fresh.conn(), 1)
        .unwrap()
        .draft
        .to_tx_draft(chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
    assert!(entry.validate().is_ok());
}
