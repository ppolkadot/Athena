//! Frozen wire DTOs for Athena Format 5 exports (`"format": "athena-profile"`, `format_version: 5`),
//! written by 1.4. Format 5 is format 4 plus required `goals` (each with nested `contributions`)
//! and `closings` arrays (contract §14.3, §10.2.3). Never derive these from domain types.

use serde::{Deserialize, Serialize};

use super::format_v2::{CategoryV2, PersonV2, ProfileV2, TransactionV2};
use super::format_v3::TemplateV3;
use super::format_v4::AppearanceV4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentV5 {
    pub format: String,
    pub format_version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub profile: ProfileV2,
    pub people: Vec<PersonV2>,
    pub categories: Vec<CategoryV2>,
    pub transactions: Vec<TransactionV2>,
    pub templates: Vec<TemplateV3>,
    /// Absent when no appearance was applied; importing then clears appearance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<AppearanceV4>,
    /// In id order, each with its contributions in id order.
    pub goals: Vec<GoalV5>,
    /// In month order.
    pub closings: Vec<ClosingV5>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalV5 {
    pub id: i64,
    pub name: String,
    pub target_cents: i64,
    pub target_date: Option<String>,
    pub frequency: String,
    pub attribution_id: Option<i64>,
    pub status: String,
    pub pre_archive_status: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub contributions: Vec<ContributionV5>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContributionV5 {
    pub id: i64,
    pub amount_cents: i64,
    pub date: String,
    pub note: Option<String>,
    pub created_at: String,
}

/// A saved closing. The comparison fingerprints are stored values, never regenerated on import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClosingV5 {
    pub month: String,
    pub closed_at: String,
    pub income_cents: i64,
    pub expenses_cents: i64,
    pub refunds_cents: i64,
    pub result_cents: i64,
    pub entry_count: i64,
    pub previous_spending_cents: i64,
    pub previous_count: i64,
    pub top_category_id: Option<i64>,
    pub top_category_net_cents: Option<i64>,
    pub largest_expense_id: Option<i64>,
    pub largest_expense_title: Option<String>,
    pub largest_expense_cents: Option<i64>,
    pub fingerprint_version: i64,
    pub current_fingerprint: String,
    pub previous_fingerprint: String,
}
