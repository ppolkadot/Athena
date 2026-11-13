//! The complete structured export: cents as integers, ISO dates, each transaction with its
//! allocations. The file is re-read and parsed before it replaces anything.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::{ExportReceipt, atomic_write, ordered_transactions, people_by_id};
use crate::domain::category::Category;
use crate::domain::export::ExportScope;
use crate::domain::person::Person;
use crate::domain::settings::Profile;
use crate::domain::template::Template;
use crate::domain::transaction::Transaction;
use crate::infrastructure::db::categories_repo;
use crate::shared::error::{AppError, Result};

/// Additive changes keep this at 1; collections added after 1.0 are `#[serde(default)]` so older
/// documents still parse.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileDocument {
    pub schema_version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub profile: Profile,
    pub people: Vec<Person>,
    pub categories: Vec<Category>,
    pub transactions: Vec<Transaction>,
    /// Templates arrive in 1.1; 1.0 always writes an empty list.
    #[serde(default)]
    pub templates: Vec<Template>,
}

/// Collect everything into a document, in the shared deterministic order.
///
/// # Errors
/// Database errors.
pub fn build_document(
    conn: &Connection,
    profile: &Profile,
    exported_at: &str,
) -> Result<ProfileDocument> {
    let mut categories = categories_repo::list(conn, true)?;
    categories.sort_by_key(|c| c.id);
    Ok(ProfileDocument {
        schema_version: SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        exported_at: exported_at.to_string(),
        profile: profile.clone(),
        people: people_by_id(conn)?,
        categories,
        transactions: ordered_transactions(conn, &ExportScope::All.filter())?,
        templates: Vec::new(),
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
        let parsed: ProfileDocument = serde_json::from_str(&text)
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
