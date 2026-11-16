//! The complete structured export (Format 2): integer cents, ISO dates, record timestamps, and
//! each live transaction with its allocations. Written bytes are re-read and verified before
//! replacing the destination.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use rusqlite::Connection;

use super::format_v2::{AllocationV2, CategoryV2, DocumentV2, PersonV2, ProfileV2, TransactionV2};
use super::{ExportReceipt, atomic_write, ordered_transactions, people_by_id};
use crate::domain::export::ExportScope;
use crate::domain::settings::Profile;
use crate::infrastructure::db::categories_repo;
use crate::shared::error::{AppError, Result};

pub const FORMAT_NAME: &str = "athena-profile";
pub const FORMAT_VERSION: u32 = 2;

/// Collect everything into a Format 2 document, in the shared deterministic order.
///
/// # Errors
/// Database errors.
pub fn build_document(
    conn: &Connection,
    profile: &Profile,
    exported_at: &str,
) -> Result<DocumentV2> {
    let mut categories = categories_repo::list(conn, true)?;
    categories.sort_by_key(|c| c.id);
    let people = people_by_id(conn)?;
    let transactions = ordered_transactions(conn, &ExportScope::All.filter())?;

    Ok(DocumentV2 {
        format: FORMAT_NAME.to_string(),
        format_version: FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        exported_at: exported_at.to_string(),
        profile: ProfileV2 {
            start_balance_cents: profile.start_balance.cents(),
            theme: profile.theme.code().to_string(),
            language: profile.language.code().to_string(),
            onboarding_complete: profile.onboarding_complete,
        },
        people: people
            .into_iter()
            .map(|p| PersonV2 {
                id: p.id,
                name: p.name,
                kind: p.kind.code().to_string(),
                archived: p.archived,
                created_at: p.created_at,
            })
            .collect(),
        categories: categories
            .into_iter()
            .map(|c| CategoryV2 {
                id: c.id,
                name: c.name,
                color: c.color,
                icon: c.icon,
                applies_to: c.applies_to.code().to_string(),
                is_default: c.is_default,
                archived: c.archived,
                created_at: c.created_at,
            })
            .collect(),
        transactions: transactions
            .into_iter()
            .map(|t| TransactionV2 {
                id: t.id,
                title: t.title,
                amount_cents: t.amount.cents(),
                date: t.date.format("%Y-%m-%d").to_string(),
                kind: t.kind.code().to_string(),
                category_id: t.category_id,
                payer_id: t.payer_id,
                description: t.description,
                split_mode: t.split_mode.map(|m| m.code().to_string()),
                created_at: t.created_at,
                updated_at: t.updated_at,
                allocations: t
                    .allocations
                    .into_iter()
                    .map(|a| AllocationV2 {
                        person_id: a.person_id,
                        amount_cents: a.amount.cents(),
                    })
                    .collect(),
            })
            .collect(),
    })
}

/// Write the full profile to `path`. `exported_at` is passed in so exporting never reads the clock.
///
/// # Errors
/// Database and I/O errors, or [`AppError::Export`] if the written bytes do not parse back.
pub fn write_json(
    conn: &Connection,
    profile: &Profile,
    exported_at: &str,
    path: &Path,
) -> Result<ExportReceipt> {
    let document = build_document(conn, profile, exported_at)?;
    let record_count = document.transactions.len();
    atomic_write(path, |file| {
        serde_json::to_writer_pretty(&mut *file, &document)
            .map_err(|e| AppError::Export(e.to_string()))?;
        file.flush()?;
        // Validate the bytes that were actually written, not the in-memory value.
        file.seek(SeekFrom::Start(0))?;
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        let parsed: DocumentV2 = serde_json::from_str(&text)
            .map_err(|e| AppError::Export(format!("written file is not valid: {e}")))?;
        if parsed != document {
            return Err(AppError::Export(
                "written file does not match the data".into(),
            ));
        }
        Ok(())
    })?;
    Ok(ExportReceipt {
        path: path.to_path_buf(),
        bytes: std::fs::metadata(path)?.len(),
        record_count,
    })
}
