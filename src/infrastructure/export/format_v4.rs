//! Frozen wire DTOs for Athena Format 4 exports (`"format": "athena-profile"`, `format_version: 4`),
//! written by 1.3. Format 4 is format 3 plus an optional `appearance` object with the portable
//! preferences: a local wallpaper is exported as `none` (retaining fit, dim and paused), and asset
//! ids, paths and image bytes are never written. Never derive these from domain types.

use serde::{Deserialize, Serialize};

use super::format_v2::{CategoryV2, PersonV2, ProfileV2, TransactionV2};
use super::format_v3::TemplateV3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentV4 {
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
}

/// Enums are strings and numbers are wide integers so that an unknown value or an out-of-range
/// number is a validation error rather than a parse failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppearanceV4 {
    pub theme_mode: String,
    pub preset: String,
    pub accent: String,
    pub density: String,
    pub text_scale: i64,
    pub motion: String,
    pub reduce_transparency: bool,
    pub quality: String,
    /// `none` or `builtin`.
    pub background: String,
    pub fit: String,
    pub dim: i64,
    pub paused: bool,
}
