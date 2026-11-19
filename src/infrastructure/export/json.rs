//! The complete structured export (Format 5): integer cents, ISO dates, record timestamps, each
//! live transaction with its allocations, the entry templates, the portable appearance, and the
//! goals with their contributions and saved closings. Written bytes are re-read and verified
//! before replacing the destination.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use rusqlite::Connection;

use super::format_v2::{AllocationV2, CategoryV2, PersonV2, ProfileV2, TransactionV2};
use super::format_v3::TemplateV3;
use super::format_v4::AppearanceV4;
use super::format_v5::{ClosingV5, ContributionV5, DocumentV5, GoalV5};
use super::{ExportReceipt, atomic_write, ordered_transactions, people_by_id};
use crate::domain::appearance::{AppearancePrefs, Wallpaper};
use crate::domain::export::ExportScope;
use crate::domain::settings::Profile;
use crate::infrastructure::db::{
    appearance_repo, categories_repo, closings_repo, goals_repo, templates_repo,
};
use crate::shared::error::{AppError, Result};

pub const FORMAT_NAME: &str = "athena-profile";
pub const FORMAT_VERSION: u32 = 5;

/// The portable wire form of the preferences: a local wallpaper is exported as `none`.
#[must_use]
pub fn appearance_dto(prefs: &AppearancePrefs) -> AppearanceV4 {
    let prefs = prefs.portable();
    AppearanceV4 {
        theme_mode: prefs.theme_mode.code().to_string(),
        preset: prefs.preset.code().to_string(),
        accent: prefs.accent.code().to_string(),
        density: prefs.density.code().to_string(),
        text_scale: i64::from(prefs.text_scale.percent()),
        motion: prefs.motion.code().to_string(),
        reduce_transparency: prefs.reduce_transparency,
        quality: prefs.quality.code().to_string(),
        background: if prefs.wallpaper == Wallpaper::Builtin {
            "builtin"
        } else {
            "none"
        }
        .to_string(),
        fit: prefs.fit.code().to_string(),
        dim: i64::from(prefs.dim),
        paused: prefs.paused,
    }
}

/// The wire form of a saved closing. A largest-expense reference that is not in the exported
/// (live) rows is written as null; the captured title and amount stay.
fn closing_dto(
    closing: &crate::domain::closing::ClosingSnapshot,
    live_ids: &std::collections::HashSet<i64>,
) -> ClosingV5 {
    let expense = closing.largest_expense.as_ref();
    ClosingV5 {
        month: closing.month.key(),
        closed_at: closing.closed_at.clone(),
        income_cents: closing.income.cents(),
        expenses_cents: closing.expenses.cents(),
        refunds_cents: closing.refunds.cents(),
        result_cents: closing.result.cents(),
        entry_count: closing.count,
        previous_spending_cents: closing.previous_spending.cents(),
        previous_count: closing.previous_count,
        top_category_id: closing.top_category.and_then(|(id, _)| id),
        top_category_net_cents: closing.top_category.map(|(_, net)| net.cents()),
        largest_expense_id: expense
            .and_then(|e| e.id)
            .filter(|id| live_ids.contains(id)),
        largest_expense_title: expense.map(|e| e.title.clone()),
        largest_expense_cents: expense.map(|e| e.amount.cents()),
        fingerprint_version: i64::from(closing.fingerprint_version),
        current_fingerprint: closing.current_fingerprint.clone(),
        previous_fingerprint: closing.previous_fingerprint.clone(),
    }
}

/// Collect everything into a Format 5 document, in the shared deterministic order.
///
/// # Errors
/// Database errors.
#[allow(clippy::too_many_lines)]
pub fn build_document(
    conn: &Connection,
    profile: &Profile,
    exported_at: &str,
) -> Result<DocumentV5> {
    let mut categories = categories_repo::list(conn, true)?;
    categories.sort_by_key(|c| c.id);
    let people = people_by_id(conn)?;
    let transactions = ordered_transactions(conn, &ExportScope::All.filter())?;

    let mut templates = templates_repo::list(conn)?;
    templates.sort_by_key(|t| t.id);
    // An unreadable row is not exported; it never blocks the financial export.
    let appearance = match appearance_repo::load(conn) {
        Ok(prefs) => prefs.as_ref().map(appearance_dto),
        Err(err) => {
            tracing::warn!("appearance preferences were not exported: {err}");
            None
        }
    };
    let goals = goals_repo::records(conn)?;
    let closings = closings_repo::list(conn)?;
    let live_ids: std::collections::HashSet<i64> = transactions.iter().map(|t| t.id).collect();
    Ok(DocumentV5 {
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
        templates: templates
            .into_iter()
            .map(|t| {
                let mut allocations = t.draft.allocations;
                allocations.sort_by_key(|a| a.person_id);
                TemplateV3 {
                    id: t.id,
                    name: t.draft.name,
                    title: t.draft.title,
                    amount_cents: t.draft.amount.cents(),
                    kind: t.draft.kind.code().to_string(),
                    category_id: t.draft.category_id,
                    payer_id: t.draft.payer_id,
                    responsibility: t.draft.responsibility.code().to_string(),
                    description: t.draft.description,
                    created_at: t.created_at,
                    last_used_at: t.last_used_at,
                    allocations: allocations
                        .into_iter()
                        .map(|a| AllocationV2 {
                            person_id: a.person_id,
                            amount_cents: a.amount.cents(),
                        })
                        .collect(),
                }
            })
            .collect(),
        appearance,
        goals: goals
            .into_iter()
            .map(|record| GoalV5 {
                id: record.goal.id,
                name: record.goal.name,
                target_cents: record.goal.target.cents(),
                target_date: record
                    .goal
                    .target_date
                    .map(|d| d.format("%Y-%m-%d").to_string()),
                frequency: record.goal.frequency.code().to_string(),
                attribution_id: record.goal.attribution_id,
                status: record.goal.status.code().to_string(),
                pre_archive_status: record.goal.pre_archive_status.map(|s| s.code().to_string()),
                created_at: record.goal.created_at,
                updated_at: record.goal.updated_at,
                contributions: record
                    .contributions
                    .into_iter()
                    .map(|c| ContributionV5 {
                        id: c.id,
                        amount_cents: c.amount.cents(),
                        date: c.date.format("%Y-%m-%d").to_string(),
                        note: c.note,
                        created_at: c.created_at,
                    })
                    .collect(),
            })
            .collect(),
        closings: closings
            .iter()
            .map(|closing| closing_dto(closing, &live_ids))
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
        let parsed: DocumentV5 = serde_json::from_str(&text)
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
