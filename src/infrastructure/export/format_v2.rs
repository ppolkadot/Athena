//! Frozen wire DTOs for Athena Format 2 exports (`"format": "athena-profile"`, `format_version: 2`).
//! Never derive these from domain types: domain changes must not silently alter the wire contract.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentV2 {
    pub format: String,
    pub format_version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub profile: ProfileV2,
    pub people: Vec<PersonV2>,
    pub categories: Vec<CategoryV2>,
    pub transactions: Vec<TransactionV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileV2 {
    pub start_balance_cents: i64,
    pub theme: String,
    pub language: String,
    pub onboarding_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonV2 {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub archived: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryV2 {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub icon: Option<String>,
    pub applies_to: String,
    pub is_default: bool,
    pub archived: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionV2 {
    pub id: i64,
    pub title: String,
    pub amount_cents: i64,
    pub date: String,
    pub kind: String,
    pub category_id: Option<i64>,
    pub payer_id: Option<i64>,
    pub description: Option<String>,
    pub split_mode: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub allocations: Vec<AllocationV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationV2 {
    pub person_id: i64,
    pub amount_cents: i64,
}
