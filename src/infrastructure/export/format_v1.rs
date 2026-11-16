//! Frozen wire DTOs for Athena Format 1 exports (1.0.x `schema_version: 1`).
//! Never derive these from domain types: domain changes must not alter Format 1 parsing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentV1 {
    pub schema_version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub profile: ProfileV1,
    pub people: Vec<PersonV1>,
    pub categories: Vec<CategoryV1>,
    pub transactions: Vec<TransactionV1>,
    #[serde(default)]
    pub templates: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileV1 {
    pub start_balance: i64,
    pub theme: String,
    pub language: String,
    pub onboarding_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonV1 {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub archived: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryV1 {
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
pub struct TransactionV1 {
    pub id: i64,
    pub title: String,
    pub amount: i64,
    pub date: String,
    pub kind: String,
    pub category_id: Option<i64>,
    pub payer_id: Option<i64>,
    pub description: Option<String>,
    pub split_mode: Option<String>,
    pub allocations: Vec<AllocationV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationV1 {
    pub person_id: i64,
    pub amount: i64,
}
