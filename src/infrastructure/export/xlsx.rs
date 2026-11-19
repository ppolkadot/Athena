//! The workbook export: an `Entries` sheet (one row per transaction, the CSV columns plus the id and
//! an amount representation) and an `Allocations` sheet (one row per share). Both sheets start with
//! a labeled scope row and a frozen header row.
//!
//! Every user value is written with an explicit string write, so text that begins with `=`, `+`,
//! `-` or `@` is stored as text and can never become a formula or link. Dates are ISO text and
//! transaction ids are exact text. Money is a numeric cell while its cent count has at most 13
//! digits (the integer cents then survive the `f64` a spreadsheet stores); larger magnitudes are
//! written as exact two-decimal text and flagged with the localized "Exact text" indicator, never
//! rounded.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use rusqlite::Connection;
use rust_xlsxwriter::{Format, Workbook, Worksheet, XlsxError};

use super::csv::type_label;
use super::{ExportReceipt, atomic_write, ordered_transactions, people_by_id};
use crate::domain::export::ExportScope;
use crate::domain::settings::Language;
use crate::domain::transaction::Transaction;
use crate::i18n::{Strings, month_label, strings};
use crate::infrastructure::db::categories_repo;
use crate::shared::error::{AppError, Result};

/// The largest cent magnitude written as a number.
pub const MAX_NUMERIC_CENTS: u64 = 9_999_999_999_999;

/// Row of the scope label, then of the headings; data starts below both.
const SCOPE_ROW: u32 = 0;
const HEADER_ROW: u32 = 1;
const FIRST_DATA_ROW: u32 = 2;

fn xlsx_error(err: &XlsxError) -> AppError {
    AppError::Export(err.to_string())
}

/// How one amount is stored.
#[derive(Debug, Clone, PartialEq)]
pub enum AmountCell {
    /// Reais as a number; `cents` is recovered exactly by `round(value * 100)`.
    Number(f64),
    /// Exact `-?digits.dd` text for a magnitude beyond [`MAX_NUMERIC_CENTS`].
    Exact(String),
}

/// `1234.50`, `-0.05` or `92233720368547758.07`: dot decimal, no grouping, exact for every `i64`.
#[must_use]
pub fn exact_amount_text(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    let sign = if cents < 0 { "-" } else { "" };
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

/// Pick the cell kind for `cents`.
#[must_use]
pub fn amount_cell(cents: i64) -> AmountCell {
    if cents.unsigned_abs() <= MAX_NUMERIC_CENTS {
        #[allow(clippy::cast_precision_loss)]
        AmountCell::Number(cents as f64 / 100.0)
    } else {
        AmountCell::Exact(exact_amount_text(cents))
    }
}

struct Formats {
    header: Format,
    number: Format,
}

impl Formats {
    fn new() -> Self {
        Self {
            header: Format::new().set_bold(),
            number: Format::new().set_num_format("#,##0.00"),
        }
    }
}

/// Write an amount and its representation label into the two columns starting at `col`.
fn write_amount(
    sheet: &mut Worksheet,
    formats: &Formats,
    s: &Strings,
    row: u32,
    col: u16,
    cents: i64,
) -> Result<()> {
    match amount_cell(cents) {
        AmountCell::Number(value) => {
            sheet
                .write_number_with_format(row, col, value, &formats.number)
                .map_err(|e| xlsx_error(&e))?;
            sheet
                .write_string(row, col + 1, s.xlsx_form_number)
                .map_err(|e| xlsx_error(&e))?;
        }
        AmountCell::Exact(text) => {
            sheet
                .write_string(row, col, text)
                .map_err(|e| xlsx_error(&e))?;
            sheet
                .write_string(row, col + 1, s.xlsx_form_exact)
                .map_err(|e| xlsx_error(&e))?;
        }
    }
    Ok(())
}

fn write_text(sheet: &mut Worksheet, row: u32, col: u16, text: &str) -> Result<()> {
    sheet
        .write_string(row, col, text)
        .map(|_| ())
        .map_err(|e| xlsx_error(&e))
}

/// The scope row, the heading row and the freeze below them.
fn start_sheet(
    sheet: &mut Worksheet,
    formats: &Formats,
    name: &str,
    scope_label: &str,
    scope_value: &str,
    headers: &[&str],
) -> Result<()> {
    sheet.set_name(name).map_err(|e| xlsx_error(&e))?;
    sheet
        .write_string_with_format(SCOPE_ROW, 0, scope_label, &formats.header)
        .map_err(|e| xlsx_error(&e))?;
    write_text(sheet, SCOPE_ROW, 1, scope_value)?;
    for (col, header) in headers.iter().enumerate() {
        let col = u16::try_from(col).map_err(|_| AppError::Export("too many columns".into()))?;
        sheet
            .write_string_with_format(HEADER_ROW, col, *header, &formats.header)
            .map_err(|e| xlsx_error(&e))?;
        sheet
            .set_column_width(col, 18)
            .map_err(|e| xlsx_error(&e))?;
    }
    sheet
        .set_freeze_panes(FIRST_DATA_ROW, 0)
        .map_err(|e| xlsx_error(&e))?;
    Ok(())
}

fn data_row(offset: usize) -> Result<u32> {
    u32::try_from(offset)
        .ok()
        .and_then(|n| n.checked_add(FIRST_DATA_ROW))
        .ok_or_else(|| AppError::Export("too many rows for a workbook".into()))
}

struct Names {
    people: HashMap<i64, String>,
    categories: HashMap<i64, String>,
}

impl Names {
    fn person(&self, id: i64) -> &str {
        self.people.get(&id).map_or("?", String::as_str)
    }
}

fn write_entries(
    sheet: &mut Worksheet,
    formats: &Formats,
    language: Language,
    names: &Names,
    transactions: &[Transaction],
) -> Result<()> {
    let s = strings(language);
    for (index, tx) in transactions.iter().enumerate() {
        let row = data_row(index)?;
        write_text(sheet, row, 0, &tx.id.to_string())?;
        write_text(sheet, row, 1, &tx.date.format("%Y-%m-%d").to_string())?;
        write_text(sheet, row, 2, &tx.title)?;
        write_text(sheet, row, 3, type_label(tx.kind, language))?;
        write_amount(sheet, formats, s, row, 4, tx.amount.cents())?;
        let category = tx
            .category_id
            .and_then(|id| names.categories.get(&id))
            .map_or("", String::as_str);
        write_text(sheet, row, 6, category)?;
        write_text(sheet, row, 7, tx.payer_id.map_or("", |id| names.person(id)))?;
        let people: Vec<&str> = tx
            .allocations
            .iter()
            .map(|a| names.person(a.person_id))
            .collect();
        write_text(sheet, row, 8, &people.join(", "))?;
        let split: Vec<String> = tx
            .allocations
            .iter()
            .map(|a| format!("{}:{}", names.person(a.person_id), a.amount.br_number()))
            .collect();
        write_text(sheet, row, 9, &split.join("|"))?;
        write_text(sheet, row, 10, tx.description.as_deref().unwrap_or(""))?;
    }
    Ok(())
}

fn write_allocations(
    sheet: &mut Worksheet,
    formats: &Formats,
    language: Language,
    names: &Names,
    transactions: &[Transaction],
) -> Result<()> {
    let s = strings(language);
    let mut index = 0;
    for tx in transactions {
        for allocation in &tx.allocations {
            let row = data_row(index)?;
            write_text(sheet, row, 0, &tx.id.to_string())?;
            write_text(sheet, row, 1, names.person(allocation.person_id))?;
            write_amount(sheet, formats, s, row, 2, allocation.amount.cents())?;
            index += 1;
        }
    }
    Ok(())
}

fn build(conn: &Connection, language: Language, scope: ExportScope) -> Result<(Vec<u8>, usize)> {
    let s = strings(language);
    let transactions = ordered_transactions(conn, &scope.filter())?;
    let names = Names {
        people: people_by_id(conn)?
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect(),
        categories: categories_repo::list(conn, true)?
            .into_iter()
            .map(|c| (c.id, c.name))
            .collect(),
    };
    let scope_value = match scope {
        ExportScope::All => s.xlsx_scope_all.to_string(),
        ExportScope::Month(month) => month_label(month, language),
    };
    let formats = Formats::new();
    let mut workbook = Workbook::new();

    let entries = workbook.add_worksheet();
    start_sheet(
        entries,
        &formats,
        s.xlsx_sheet_entries,
        s.xlsx_scope,
        &scope_value,
        &[
            s.xlsx_id,
            s.csv_date,
            s.csv_title,
            s.csv_type,
            s.csv_amount,
            s.xlsx_amount_form,
            s.csv_category,
            s.csv_payer,
            s.csv_people,
            s.csv_allocations,
            s.csv_description,
        ],
    )?;
    write_entries(entries, &formats, language, &names, &transactions)?;

    let allocations = workbook.add_worksheet();
    start_sheet(
        allocations,
        &formats,
        s.xlsx_sheet_allocations,
        s.xlsx_scope,
        &scope_value,
        &[
            s.xlsx_transaction_id,
            s.xlsx_participant,
            s.csv_amount,
            s.xlsx_amount_form,
        ],
    )?;
    write_allocations(allocations, &formats, language, &names, &transactions)?;

    let bytes = workbook.save_to_buffer().map_err(|e| xlsx_error(&e))?;
    Ok((bytes, transactions.len()))
}

/// Write the transactions in `scope` to `path` as a workbook. Headers follow `language`.
///
/// # Errors
/// Database and I/O errors, or [`AppError::Export`] when a value exceeds a workbook limit; the
/// destination is left untouched on failure.
pub fn write_xlsx(
    conn: &Connection,
    language: Language,
    scope: ExportScope,
    path: &Path,
) -> Result<ExportReceipt> {
    write_xlsx_with(conn, language, scope, path, |_| Ok(()))
}

/// [`write_xlsx`] with a check that runs after the file is complete and synced but before it
/// replaces `path`. An error from `before_publish` abandons the export.
pub(crate) fn write_xlsx_with(
    conn: &Connection,
    language: Language,
    scope: ExportScope,
    path: &Path,
    before_publish: impl FnOnce(&mut std::fs::File) -> Result<()>,
) -> Result<ExportReceipt> {
    let (bytes, record_count) = build(conn, language, scope)?;
    atomic_write(path, |file| {
        file.write_all(&bytes)?;
        file.flush()?;
        file.sync_all()?;
        before_publish(file)
    })?;
    Ok(ExportReceipt {
        path: path.to_path_buf(),
        bytes: std::fs::metadata(path)?.len(),
        record_count,
    })
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;

    use calamine::{Data, Reader, Xlsx, open_workbook};

    use super::*;
    use crate::domain::date::YearMonth;
    use crate::domain::money::Money;
    use crate::infrastructure::db::{Db, people_repo, transactions_repo};
    use crate::shared::test_fixtures::{date, draft_with, seed_all_kinds, seed_people};

    type Book = Xlsx<BufReader<std::fs::File>>;

    fn open(path: &Path) -> Book {
        open_workbook(path).unwrap()
    }

    fn text(data: Option<&Data>) -> String {
        match data {
            Some(Data::String(s)) => s.clone(),
            Some(Data::Empty) | None => String::new(),
            other => panic!("expected text, found {other:?}"),
        }
    }

    /// The cents of a numeric cell: what a spreadsheet user would recover.
    fn cents(data: Option<&Data>) -> i64 {
        match data {
            #[allow(clippy::cast_possible_truncation)]
            Some(Data::Float(f)) => (f * 100.0).round() as i64,
            Some(Data::Int(i)) => i * 100,
            other => panic!("expected a number, found {other:?}"),
        }
    }

    #[test]
    fn exact_text_is_exact_for_every_i64() {
        assert_eq!(exact_amount_text(0), "0.00");
        assert_eq!(exact_amount_text(5), "0.05");
        assert_eq!(exact_amount_text(-5), "-0.05");
        assert_eq!(exact_amount_text(123_456), "1234.56");
        assert_eq!(exact_amount_text(i64::MAX), "92233720368547758.07");
        assert_eq!(exact_amount_text(i64::MIN), "-92233720368547758.08");
    }

    #[test]
    fn the_numeric_threshold_is_exactly_thirteen_nines_of_cents() {
        let max = i64::try_from(MAX_NUMERIC_CENTS).unwrap();
        for c in [0, 1, -1, max - 1, max, -max] {
            assert!(matches!(amount_cell(c), AmountCell::Number(_)), "{c}");
        }
        assert_eq!(
            amount_cell(max + 1),
            AmountCell::Exact("100000000000.00".into())
        );
        assert_eq!(
            amount_cell(-max - 1),
            AmountCell::Exact("-100000000000.00".into())
        );
        // The number really recovers the integer cent.
        for c in [max, max - 1, -max, 1, 7, 99, 100, 101, 123_456] {
            let AmountCell::Number(v) = amount_cell(c) else {
                panic!("{c}")
            };
            #[allow(clippy::cast_possible_truncation)]
            let back = (v * 100.0).round() as i64;
            assert_eq!(back, c);
        }
    }

    #[test]
    fn negative_and_boundary_amounts_round_trip_through_a_workbook() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("amounts.xlsx");
        let max = i64::try_from(MAX_NUMERIC_CENTS).unwrap();
        let samples = [-max - 1, -max, -1, 0, 1, max - 1, max, max + 1];
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        let formats = Formats::new();
        let s = strings(Language::En);
        for (i, c) in samples.iter().enumerate() {
            write_amount(sheet, &formats, s, u32::try_from(i).unwrap(), 0, *c).unwrap();
        }
        workbook.save(&path).unwrap();
        let mut book = open(&path);
        let range = book.worksheet_range_at(0).unwrap().unwrap();
        for (i, c) in samples.iter().enumerate() {
            let r = u32::try_from(i).unwrap();
            let value = range.get_value((r, 0));
            let form = text(range.get_value((r, 1)));
            if c.unsigned_abs() <= MAX_NUMERIC_CENTS {
                assert_eq!(cents(value), *c, "{c}");
                assert_eq!(form, s.xlsx_form_number);
            } else {
                assert_eq!(text(value), exact_amount_text(*c), "{c}");
                assert_eq!(form, s.xlsx_form_exact);
            }
        }
    }

    fn seeded_with_extras() -> Db {
        let db = Db::open_in_memory().unwrap();
        let (_, partner) = seed_all_kinds(&db);
        let conn = db.conn();
        // Hostile text, another month, a trashed row, and an archived person and category.
        let mut hostile = draft_with(vec![crate::domain::transaction::Allocation {
            person_id: partner,
            amount: Money::from_cents(500),
        }]);
        hostile.title = "=1+1".into();
        hostile.description = Some("@SUM(A1)".into());
        hostile.date = date(2026, 11, 12);
        transactions_repo::create(conn, &hostile).unwrap();
        for (title, day) in [("+cmd", 13), ("-5", 14)] {
            let mut d = draft_with(vec![]);
            d.title = title.into();
            d.date = date(2026, 11, day);
            transactions_repo::create(conn, &d).unwrap();
        }
        let mut october = draft_with(vec![]);
        october.title = "October only".into();
        october.date = date(2026, 10, 31);
        transactions_repo::create(conn, &october).unwrap();
        let mut gone = draft_with(vec![]);
        gone.title = "Trashed".into();
        gone.date = date(2026, 11, 15);
        let gone_id = transactions_repo::create(conn, &gone).unwrap();
        transactions_repo::delete(conn, gone_id).unwrap();
        people_repo::set_archived(conn, partner, true).unwrap();
        db
    }

    fn rows(book: &mut Book, sheet: &str) -> calamine::Range<Data> {
        book.worksheet_range(sheet).unwrap()
    }

    #[test]
    fn xlsx_round_trips_sheets_headers_scope_and_formula_safety() {
        let db = seeded_with_extras();
        let dir = tempfile::tempdir().unwrap();
        for language in [Language::En, Language::PtBr] {
            let s = strings(language);
            for (scope, scope_text, expected) in [
                (
                    ExportScope::Month(YearMonth::new(2026, 11).unwrap()),
                    month_label(YearMonth::new(2026, 11).unwrap(), language),
                    7,
                ),
                (ExportScope::All, s.xlsx_scope_all.to_string(), 8),
            ] {
                let path = dir.path().join(format!("{language:?}-{expected}.xlsx"));
                let receipt = write_xlsx(db.conn(), language, scope, &path).unwrap();
                assert_eq!(receipt.record_count, expected);
                assert_eq!(receipt.bytes, std::fs::metadata(&path).unwrap().len());

                let mut book = open(&path);
                assert_eq!(
                    book.sheet_names(),
                    vec![s.xlsx_sheet_entries, s.xlsx_sheet_allocations]
                );
                // Nothing in either sheet is a formula, whatever the user typed.
                for sheet in [s.xlsx_sheet_entries, s.xlsx_sheet_allocations] {
                    let formulas = book.worksheet_formula(sheet).unwrap();
                    assert!(formulas.cells().all(|(_, _, f)| f.is_empty()), "{sheet}");
                }
                let entries = rows(&mut book, s.xlsx_sheet_entries);
                assert_eq!(text(entries.get_value((0, 0))), s.xlsx_scope);
                assert_eq!(text(entries.get_value((0, 1))), scope_text);
                let headers: Vec<String> =
                    (0..11).map(|c| text(entries.get_value((1, c)))).collect();
                assert_eq!(
                    headers,
                    [
                        s.xlsx_id,
                        s.csv_date,
                        s.csv_title,
                        s.csv_type,
                        s.csv_amount,
                        s.xlsx_amount_form,
                        s.csv_category,
                        s.csv_payer,
                        s.csv_people,
                        s.csv_allocations,
                        s.csv_description,
                    ]
                );
                assert_eq!(entries.height(), 2 + expected, "scope, headings, rows");

                let titles: Vec<String> = (2..entries.height())
                    .map(|r| text(entries.get_value((u32::try_from(r).unwrap(), 2))))
                    .collect();
                assert!(
                    !titles.contains(&"Trashed".to_string()),
                    "Trash is excluded"
                );
                assert_eq!(
                    titles.contains(&"October only".to_string()),
                    scope == ExportScope::All
                );
                // Formula-looking text stays exactly as typed, as text.
                for hostile in ["=1+1", "+cmd", "-5"] {
                    assert!(titles.contains(&hostile.to_string()), "{hostile}");
                }
                let hostile_row = titles.iter().position(|t| t == "=1+1").unwrap() + 2;
                let hostile_row = u32::try_from(hostile_row).unwrap();
                assert_eq!(text(entries.get_value((hostile_row, 10))), "@SUM(A1)");
                assert!(matches!(
                    entries.get_value((hostile_row, 2)),
                    Some(Data::String(_))
                ));
                // The archived partner and the settlement still resolve and are labeled.
                assert_eq!(text(entries.get_value((hostile_row, 8))), "Ana");
                let types: Vec<String> = (2..entries.height())
                    .map(|r| text(entries.get_value((u32::try_from(r).unwrap(), 3))))
                    .collect();
                assert!(
                    types.contains(&s.settlement.to_string()),
                    "settlements included"
                );
                // ISO text dates and exact text ids.
                let first = 2;
                assert!(matches!(
                    entries.get_value((first, 0)),
                    Some(Data::String(_))
                ));
                let date_text = text(entries.get_value((first, 1)));
                assert_eq!(date_text.len(), 10);
                assert!(date_text.chars().nth(4) == Some('-'));

                let allocations = rows(&mut book, s.xlsx_sheet_allocations);
                assert_eq!(text(allocations.get_value((0, 0))), s.xlsx_scope);
                assert_eq!(text(allocations.get_value((0, 1))), scope_text);
                assert_eq!(text(allocations.get_value((1, 0))), s.xlsx_transaction_id);
                assert_eq!(text(allocations.get_value((1, 1))), s.xlsx_participant);
                assert!(allocations.height() > 2);
            }
        }
    }

    #[test]
    fn the_entries_amounts_and_allocations_read_back_to_the_cent() {
        let db = Db::open_in_memory().unwrap();
        let (owner, partner) = seed_people(db.conn());
        let max = i64::try_from(MAX_NUMERIC_CENTS).unwrap();
        let mut ids = Vec::new();
        for (title, cents_, day) in [
            ("below", max - 1, 1),
            ("at", max, 2),
            ("beyond", max + 1, 3),
        ] {
            let mut d = draft_with(vec![
                crate::domain::transaction::Allocation {
                    person_id: owner,
                    amount: Money::from_cents(cents_ - 1),
                },
                crate::domain::transaction::Allocation {
                    person_id: partner,
                    amount: Money::from_cents(1),
                },
            ]);
            d.title = title.into();
            d.amount = Money::from_cents(cents_);
            d.date = date(2026, 11, day);
            ids.push(transactions_repo::create(db.conn(), &d).unwrap());
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("precision.xlsx");
        write_xlsx(db.conn(), Language::En, ExportScope::All, &path).unwrap();
        let mut book = open(&path);
        let entries = rows(&mut book, "Entries");
        let s = strings(Language::En);
        for (i, (expected, exact)) in [(max - 1, false), (max, false), (max + 1, true)]
            .into_iter()
            .enumerate()
        {
            let r = u32::try_from(i + 2).unwrap();
            assert_eq!(text(entries.get_value((r, 0))), ids[i].to_string());
            if exact {
                assert_eq!(text(entries.get_value((r, 4))), exact_amount_text(expected));
                assert_eq!(text(entries.get_value((r, 5))), s.xlsx_form_exact);
            } else {
                assert_eq!(cents(entries.get_value((r, 4))), expected);
                assert_eq!(text(entries.get_value((r, 5))), s.xlsx_form_number);
            }
        }
        // Allocations: the large share is numeric or exact text by its own size.
        let allocations = rows(&mut book, "Allocations");
        assert_eq!(allocations.height(), 2 + 6);
        let big_share = max; // the "beyond" row's owner share is max + 1 - 1
        assert_eq!(cents(allocations.get_value((6, 2))), big_share);
        assert_eq!(text(allocations.get_value((6, 0))), ids[2].to_string());
        assert_eq!(cents(allocations.get_value((7, 2))), 1);
    }

    #[test]
    fn an_empty_export_still_has_both_sheets_and_headings() {
        let db = Db::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.xlsx");
        let receipt = write_xlsx(db.conn(), Language::PtBr, ExportScope::All, &path).unwrap();
        assert_eq!(receipt.record_count, 0);
        let mut book = open(&path);
        assert_eq!(book.sheet_names(), vec!["Lançamentos", "Alocações"]);
        let entries = rows(&mut book, "Lançamentos");
        assert_eq!(entries.height(), 2);
        assert_eq!(text(entries.get_value((0, 1))), "Todo o histórico");
    }

    #[test]
    fn xlsx_atomic_write_preserves_existing_on_failure() {
        let db = seeded_with_extras();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keep.xlsx");
        std::fs::write(&path, b"existing workbook bytes").unwrap();
        let failure = write_xlsx_with(db.conn(), Language::En, ExportScope::All, &path, |_| {
            Err(AppError::Export("injected before publish".into()))
        });
        assert!(failure.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing workbook bytes");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temp file is left behind"
        );
        // And a good export replaces it whole.
        write_xlsx(db.conn(), Language::En, ExportScope::All, &path).unwrap();
        assert!(open_workbook::<Xlsx<_>, _>(&path).is_ok());
    }

    #[test]
    fn a_value_beyond_the_workbook_limit_fails_without_truncation() {
        let db = Db::open_in_memory().unwrap();
        seed_people(db.conn());
        let mut d = draft_with(vec![]);
        d.title = "x".repeat(40_000);
        d.date = date(2026, 11, 1);
        transactions_repo::create(db.conn(), &d).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.xlsx");
        std::fs::write(&path, b"keep").unwrap();
        let err = write_xlsx(db.conn(), Language::En, ExportScope::All, &path);
        assert!(matches!(err, Err(AppError::Export(_))));
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
}
