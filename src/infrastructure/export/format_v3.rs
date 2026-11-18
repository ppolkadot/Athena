//! Frozen wire DTOs for Athena Format 3 exports (`"format": "athena-profile"`, `format_version: 3`),
//! written by 1.2. Format 3 is format 2 plus a required `templates` array with nested allocations;
//! the unchanged record shapes are shared with format 2. Never derive these from domain types.

use serde::{Deserialize, Serialize};

use super::format_v2::{AllocationV2, CategoryV2, PersonV2, ProfileV2, TransactionV2};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentV3 {
    pub format: String,
    pub format_version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub profile: ProfileV2,
    pub people: Vec<PersonV2>,
    pub categories: Vec<CategoryV2>,
    pub transactions: Vec<TransactionV2>,
    pub templates: Vec<TemplateV3>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateV3 {
    pub id: i64,
    pub name: String,
    pub title: String,
    pub amount_cents: i64,
    pub kind: String,
    pub category_id: Option<i64>,
    pub payer_id: Option<i64>,
    pub responsibility: String,
    pub description: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub allocations: Vec<AllocationV2>,
}
