//! The readable report: one row per transaction. UTF-8 with a BOM, `;` delimiter and decimal
//! comma so it opens correctly in Brazilian spreadsheet software.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use rusqlite::Connection;

use super::{ExportReceipt, atomic_write, ordered_transactions, people_by_id};
use crate::domain::export::ExportScope;
use crate::domain::settings::Language;
use crate::domain::transaction::{Transaction, TransactionType};
use crate::i18n::strings;
use crate::infrastructure::db::categories_repo;
use crate::shared::error::{AppError, Result};

const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

/// Neutralize spreadsheet formulas: text that starts with `=`, `+`, `-`, `@`, tab or carriage
/// return gets a leading `'`, so it is shown as text and never executed. Apply it to text cells
/// only, never to dates or amounts.
#[must_use]
pub fn csv_safe(text: &str) -> String {
    match text.chars().next() {
        Some('=' | '+' | '-' | '@' | '\t' | '\r') => format!("'{text}"),
        _ => text.to_string(),
    }
}

pub(super) fn type_label(kind: TransactionType, language: Language) -> &'static str {
    let s = strings(language);
    match kind {
        TransactionType::Expense => s.expense,
        TransactionType::Income => s.income,
        TransactionType::Refund => s.refund,
        TransactionType::Settlement => s.settlement,
    }
}

fn row(
    tx: &Transaction,
    language: Language,
    people: &HashMap<i64, String>,
    categories: &HashMap<i64, String>,
) -> [String; 9] {
    let name_of = |id: i64| people.get(&id).map_or("?", String::as_str);
    let category = tx
        .category_id
        .and_then(|id| categories.get(&id))
        .map_or("", String::as_str);
    let payer = tx.payer_id.map_or("", name_of);
    let names: Vec<&str> = tx
        .allocations
        .iter()
        .map(|a| name_of(a.person_id))
        .collect();
    let split: Vec<String> = tx
        .allocations
        .iter()
        .map(|a| format!("{}:{}", name_of(a.person_id), a.amount.br_number()))
        .collect();
    [
        tx.date.format("%Y-%m-%d").to_string(),
        csv_safe(&tx.title),
        csv_safe(type_label(tx.kind, language)),
        tx.amount.br_number(),
        csv_safe(category),
        csv_safe(payer),
        csv_safe(&names.join(", ")),
        csv_safe(&split.join("|")),
        csv_safe(tx.description.as_deref().unwrap_or("")),
    ]
}

/// Write the transactions in `scope` to `path`. Headers follow `language`.
///
/// # Errors
/// Database and I/O errors, or [`AppError::Export`] if the written file does not read back with
/// the expected number of rows.
pub fn write_csv(
    conn: &Connection,
    language: Language,
    scope: ExportScope,
    path: &Path,
) -> Result<ExportReceipt> {
    let s = strings(language);
    let transactions = ordered_transactions(conn, &scope.filter())?;
    let people: HashMap<i64, String> = people_by_id(conn)?
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
    let categories: HashMap<i64, String> = categories_repo::list(conn, true)?
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();
    let header = [
        s.csv_date,
        s.csv_title,
        s.csv_type,
        s.csv_amount,
        s.csv_category,
        s.csv_payer,
        s.csv_people,
        s.csv_allocations,
        s.csv_description,
    ];
    let record_count = transactions.len();

    atomic_write(path, |file| {
        file.write_all(&BOM)?;
        let mut writer = csv::WriterBuilder::new()
            .delimiter(b';')
            .terminator(csv::Terminator::CRLF)
            .from_writer(&mut *file);
        writer
            .write_record(header)
            .map_err(|e| AppError::Export(e.to_string()))?;
        for tx in &transactions {
            writer
                .write_record(row(tx, language, &people, &categories))
                .map_err(|e| AppError::Export(e.to_string()))?;
        }
        writer.flush()?;
        drop(writer);

        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        if !bytes.starts_with(&BOM) {
            return Err(AppError::Export(
                "written file lost its byte order mark".into(),
            ));
        }
        let rows = csv::ReaderBuilder::new()
            .delimiter(b';')
            .has_headers(false)
            .from_reader(&bytes[BOM.len()..])
            .records()
            .count();
        if rows != record_count + 1 {
            return Err(AppError::Export(format!(
                "expected {} rows, found {rows}",
                record_count + 1
            )));
        }
        Ok(())
    })?;
    Ok(ExportReceipt {
        path: path.to_path_buf(),
        bytes: std::fs::metadata(path)?.len(),
        record_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_like_text_is_neutralized() {
        for text in ["=SUM(A1)", "+1", "-1", "@cmd", "\tx", "\rx"] {
            assert!(csv_safe(text).starts_with('\''), "{text:?}");
        }
        for text in ["Market", "", "1,00", "a=b", "R$ 5"] {
            assert_eq!(csv_safe(text), text);
        }
    }
}
