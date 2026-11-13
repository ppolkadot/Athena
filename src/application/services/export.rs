//! Export use cases. The file formats live in `infrastructure::export`; this layer is what the
//! app calls. Nothing here reads the clock: callers pass the timestamp and the scope.

use std::path::Path;

use rusqlite::Connection;

use crate::domain::export::ExportScope;
use crate::domain::settings::{Language, Profile};
pub use crate::infrastructure::export::ExportReceipt;
use crate::infrastructure::export::{csv as csv_export, json as json_export};
use crate::shared::error::Result;

/// The complete profile as JSON: settings, people, categories and every transaction with its
/// allocations. `exported_at` is an RFC 3339 timestamp supplied by the caller.
///
/// # Errors
/// Database and I/O errors; the destination is left untouched on failure.
pub fn export_json(
    conn: &Connection,
    profile: &Profile,
    exported_at: &str,
    path: &Path,
) -> Result<ExportReceipt> {
    json_export::write_json(conn, profile, exported_at, path)
}

/// The readable report for `scope` as CSV, with headers in `language`.
///
/// # Errors
/// Database and I/O errors; the destination is left untouched on failure.
pub fn export_csv(
    conn: &Connection,
    language: Language,
    scope: ExportScope,
    path: &Path,
) -> Result<ExportReceipt> {
    csv_export::write_csv(conn, language, scope, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::date::YearMonth;
    use crate::domain::money::Money;
    use crate::infrastructure::db::transactions_repo;
    use crate::infrastructure::export::json::ProfileDocument;
    use crate::shared::test_fixtures::{date, draft_with, profile, seed_month, seeded_db};

    const STAMP: &str = "2026-11-12T10:00:00Z";

    #[test]
    fn json_round_trips_and_counts() {
        let db = seeded_db();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        let receipt = export_json(db.conn(), &profile(), STAMP, &path).unwrap();
        assert!(receipt.bytes > 0);
        assert_eq!(receipt.record_count, 3);
        let text = std::fs::read_to_string(&path).unwrap();
        let doc: ProfileDocument = serde_json::from_str(&text).unwrap();
        assert_eq!(doc.schema_version, 1);
        assert_eq!(doc.exported_at, STAMP);
        assert_eq!(doc.transactions.len(), 3);
        assert!(doc.templates.is_empty());
        let shared = doc
            .transactions
            .iter()
            .find(|t| t.title == "Test" && t.amount.cents() == 20_000)
            .unwrap();
        assert_eq!(shared.allocations.len(), 2);
        assert_eq!(
            shared
                .allocations
                .iter()
                .map(|a| a.amount.cents())
                .sum::<i64>(),
            shared.amount.cents()
        );
        assert_eq!(doc.people.len(), 2);
        assert_eq!(doc.categories.len(), 12);
        assert_eq!(doc.profile, profile());
    }

    #[test]
    fn json_writes_cents_and_iso_dates_in_a_deterministic_order() {
        let db = seeded_db();
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.json"), dir.path().join("b.json"));
        export_json(db.conn(), &profile(), STAMP, &a).unwrap();
        export_json(db.conn(), &profile(), STAMP, &b).unwrap();
        let text = std::fs::read_to_string(&a).unwrap();
        assert_eq!(
            text,
            std::fs::read_to_string(&b).unwrap(),
            "same data, same bytes"
        );
        assert!(
            text.contains("\"amount\": 123456"),
            "money is integer cents"
        );
        assert!(text.contains("\"date\": \"2026-11-10\""));
        let doc: ProfileDocument = serde_json::from_str(&text).unwrap();
        let dates: Vec<_> = doc.transactions.iter().map(|t| (t.date, t.id)).collect();
        let mut sorted = dates.clone();
        sorted.sort();
        assert_eq!(dates, sorted, "transactions go by date then id");
    }

    #[test]
    fn a_1_0_document_without_templates_still_parses() {
        let db = seeded_db();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.json");
        export_json(db.conn(), &profile(), STAMP, &path).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        value.as_object_mut().unwrap().remove("templates");
        let doc: ProfileDocument = serde_json::from_value(value).unwrap();
        assert!(doc.templates.is_empty());
    }

    #[test]
    fn csv_has_bom_semicolon_decimal_comma_and_allocations() {
        let db = seeded_db();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transactions.csv");
        let receipt = export_csv(db.conn(), Language::En, ExportScope::All, &path).unwrap();
        assert_eq!(receipt.record_count, 3);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..3], &[0xEF, 0xBB, 0xBF]);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("Date;Title;Type;Amount;Category;People;Allocations;Description"));
        assert!(text.contains(";1234,56;"));
        assert!(text.contains("A3:100,00|Ana:100,00"));
        assert!(text.contains("A3, Ana"), "people are a readable list");
        assert!(text.contains("\r\n"));
    }

    #[test]
    fn csv_headers_and_type_labels_follow_the_language() {
        let db = seeded_db();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pt.csv");
        export_csv(db.conn(), Language::PtBr, ExportScope::All, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Data;Título;Tipo;Valor;Categoria;Pessoas;Divisão;Descrição"));
        assert!(text.contains("Reembolso") && text.contains("Despesa"));
    }

    #[test]
    fn csv_prefixes_formula_like_text_but_not_amounts() {
        let db = seeded_db();
        let mut d = draft_with(vec![]);
        d.title = "=SUM(A1:A2)".into();
        d.amount = Money::from_cents(100);
        transactions_repo::create(db.conn(), &d).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("formula.csv");
        export_csv(db.conn(), Language::En, ExportScope::All, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("'=SUM(A1:A2)"));
        assert!(text.contains(";1,00;"), "amounts are written unchanged");
        assert!(text.contains("2026-11-11"), "dates are written unchanged");
    }

    #[test]
    fn csv_quotes_delimiters_inside_text() {
        let db = seeded_db();
        let mut d = draft_with(vec![]);
        d.title = "Fish; chips \"and\" peas".into();
        transactions_repo::create(db.conn(), &d).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quotes.csv");
        export_csv(db.conn(), Language::En, ExportScope::All, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"Fish; chips \"\"and\"\" peas\""));
    }

    #[test]
    fn a_month_scope_exports_only_that_month() {
        let db = crate::infrastructure::db::Db::open_in_memory().unwrap();
        seed_month(&db);
        let mut october = draft_with(vec![]);
        october.title = "Old".into();
        october.date = date(2026, 10, 31);
        transactions_repo::create(db.conn(), &october).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nov.csv");
        let nov = ExportScope::Month(YearMonth::new(2026, 11).unwrap());
        let receipt = export_csv(db.conn(), Language::En, nov, &path).unwrap();
        assert_eq!(receipt.record_count, 4);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("Old") && text.contains("Rent"));
        let all = export_csv(
            db.conn(),
            Language::En,
            ExportScope::All,
            &dir.path().join("all.csv"),
        )
        .unwrap();
        assert_eq!(all.record_count, 5);
    }

    #[test]
    fn rows_are_ordered_by_date_then_id_and_archived_names_still_resolve() {
        let db = crate::infrastructure::db::Db::open_in_memory().unwrap();
        seed_month(&db);
        let (_, partner) = crate::shared::test_fixtures::seed_people(db.conn());
        crate::infrastructure::db::people_repo::set_archived(db.conn(), partner, true).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("o.csv");
        export_csv(db.conn(), Language::En, ExportScope::All, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let titles: Vec<&str> = text
            .lines()
            .skip(1)
            .filter_map(|l| l.split(';').nth(1))
            .collect();
        assert_eq!(titles, ["Rent", "Market", "Salary", "Refund"]);
        assert!(
            text.contains("A3:25,00|Ana:25,00"),
            "an archived person keeps their name in history"
        );
    }

    #[test]
    fn a_failed_export_leaves_the_existing_file_alone() {
        let db = seeded_db();
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("keep.json");
        std::fs::write(&existing, b"precious").unwrap();
        // A directory in the way makes the final rename fail after everything else succeeded.
        let blocked = dir.path().join("blocked.json");
        std::fs::create_dir(&blocked).unwrap();
        assert!(export_json(db.conn(), &profile(), STAMP, &blocked).is_err());
        assert!(blocked.is_dir());
        assert_eq!(std::fs::read(&existing).unwrap(), b"precious");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "no temp files are left"
        );
    }
}
