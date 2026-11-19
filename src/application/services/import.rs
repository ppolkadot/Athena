//! Import service: bounded file read, envelope dispatch, Format 1 and 2 adaptation to the Format 3
//! shape, and complete §10.4 + D-P5 validation returning up to 50 collected [`ImportError`]s.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use chrono::{DateTime, Datelike, NaiveDate};
use rusqlite::Connection;

use crate::domain::appearance::{
    Accent, AppearancePrefs, Density, Fit, MotionPref, Preset, Quality, TextScale, ThemeMode,
    Wallpaper,
};
use crate::domain::category::{Category, CategoryScope};
use crate::domain::export::ExportScope;
use crate::domain::money::Money;
use crate::domain::person::{Person, PersonKind};
use crate::domain::settings::{Language, Profile, Theme};
use crate::domain::template::{ResponsibilityMode, Template, TemplateDraft, name_key};
use crate::domain::text::fold;
use crate::domain::transaction::{Allocation, SplitMode, Transaction, TransactionType};
use crate::infrastructure::db::{
    appearance_repo, categories_repo, people_repo, templates_repo, transactions_repo,
};
use crate::infrastructure::export::format_v1::DocumentV1;
use crate::infrastructure::export::format_v2::{
    CategoryV2, DocumentV2, PersonV2, ProfileV2, TransactionV2,
};
use crate::infrastructure::export::format_v3::DocumentV3;
use crate::infrastructure::export::format_v4::{AppearanceV4, DocumentV4};
use crate::infrastructure::export::json::{FORMAT_NAME, FORMAT_VERSION};
use crate::shared::error::{AppError, ImportError, Result};

/// Maximum JSON import file size (64 MiB).
pub const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ERRORS: usize = 50;

/// Validated, normalized profile ready for preview and atomic database replacement.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportPlan {
    pub source_name: String,
    pub format_version: u32,
    pub app_version: String,
    pub exported_at: String,
    pub used_timestamp_fallback: bool,
    pub profile: Profile,
    /// Category names whose scope was widened to [`CategoryScope::Both`] so that existing entries
    /// stay valid. Empty for exports that already agree with the current rules.
    pub widened_categories: Vec<String>,
    pub people: Vec<Person>,
    pub categories: Vec<Category>,
    pub transactions: Vec<Transaction>,
    /// Entry templates (format 3); empty for formats 1 and 2.
    pub templates: Vec<Template>,
    /// Portable appearance (format 4). `None` clears the stored appearance on replacement.
    pub appearance: Option<AppearancePrefs>,
}

impl ImportPlan {
    /// Build a canonical plan from the current live database for comparison in tests.
    ///
    /// # Errors
    /// Database errors.
    pub fn from_db(conn: &Connection, profile: &Profile) -> Result<Self> {
        let mut people = people_repo::list(conn, true)?;
        people.sort_by_key(|p| p.id);
        let mut categories = categories_repo::list(conn, true)?;
        categories.sort_by_key(|c| c.id);
        let mut transactions = transactions_repo::list(conn, &ExportScope::All.filter())?;
        transactions.sort_by_key(|t| (t.date, t.id));
        for tx in &mut transactions {
            tx.allocations.sort_by_key(|a| a.person_id);
        }
        let mut templates = templates_repo::list(conn)?;
        templates.sort_by_key(|t| t.id);
        let appearance = appearance_repo::load(conn)
            .ok()
            .flatten()
            .map(|a| a.portable());
        Ok(Self {
            source_name: String::new(),
            format_version: FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            exported_at: String::new(),
            used_timestamp_fallback: false,
            profile: profile.clone(),
            widened_categories: Vec::new(),
            people,
            categories,
            transactions,
            templates,
            appearance,
        })
    }
}

struct ErrorCollector(Vec<ImportError>);

impl ErrorCollector {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn push(&mut self, err: ImportError) {
        if self.0.len() < MAX_ERRORS {
            self.0.push(err);
        }
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

struct RawDocument {
    format_version: u32,
    app_version: String,
    exported_at: String,
    used_timestamp_fallback: bool,
    is_format_1: bool,
    start_balance_cents: i64,
    theme: String,
    language: String,
    onboarding_complete: bool,
    people: Vec<RawPerson>,
    categories: Vec<RawCategory>,
    transactions: Vec<RawTransaction>,
    templates: Vec<RawTemplate>,
    appearance: Option<AppearanceV4>,
}

struct RawPerson {
    id: i64,
    name: String,
    kind: String,
    archived: bool,
    created_at: String,
}

struct RawCategory {
    id: i64,
    name: String,
    color: String,
    icon: Option<String>,
    applies_to: String,
    is_default: bool,
    archived: bool,
    created_at: String,
}

struct RawTransaction {
    id: i64,
    title: String,
    amount_cents: i64,
    date: String,
    kind: String,
    category_id: Option<i64>,
    payer_id: Option<i64>,
    description: Option<String>,
    split_mode: Option<String>,
    created_at: String,
    updated_at: String,
    allocations: Vec<(i64, i64)>,
}

struct RawTemplate {
    id: i64,
    name: String,
    title: String,
    amount_cents: i64,
    kind: String,
    category_id: Option<i64>,
    payer_id: Option<i64>,
    responsibility: String,
    description: Option<String>,
    created_at: String,
    last_used_at: Option<String>,
    allocations: Vec<(i64, i64)>,
}

/// Read a JSON export from `path`, dispatch by envelope, adapt Format 1 if needed, and run full
/// validation.
///
/// # Errors
/// [`AppError::Import`] containing up to 50 [`ImportError`]s.
pub fn read_and_validate(path: &Path) -> Result<ImportPlan> {
    let bytes = read_capped_bytes(path)?;
    let source_name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    parse_and_validate(&bytes, &source_name)
}

/// Parse and validate an in-memory JSON export buffer.
///
/// # Errors
/// [`AppError::Import`] containing up to 50 [`ImportError`]s.
pub fn parse_and_validate(bytes: &[u8], source_name: &str) -> Result<ImportPlan> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(AppError::Import(vec![ImportError::TooLarge]));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| AppError::Import(vec![ImportError::MalformedJson(e.to_string())]))?;
    let mut errors = ErrorCollector::new();
    let raw = dispatch_value(value, &mut errors)?;
    validate_raw(raw, source_name.to_string(), errors)
}

fn read_capped_bytes(path: &Path) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path)
        .map_err(|e| AppError::Import(vec![ImportError::Io(e.to_string())]))?;
    if meta.len() > MAX_IMPORT_BYTES as u64 {
        return Err(AppError::Import(vec![ImportError::TooLarge]));
    }
    let file =
        File::open(path).map_err(|e| AppError::Import(vec![ImportError::Io(e.to_string())]))?;
    let mut bytes = Vec::new();
    file.take(MAX_IMPORT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::Import(vec![ImportError::Io(e.to_string())]))?;
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(AppError::Import(vec![ImportError::TooLarge]));
    }
    Ok(bytes)
}

fn dispatch_value(value: serde_json::Value, errors: &mut ErrorCollector) -> Result<RawDocument> {
    let Some(obj) = value.as_object() else {
        return Err(AppError::Import(vec![ImportError::NotAthena]));
    };
    if let Some(format_val) = obj.get("format") {
        if format_val.as_str() != Some(FORMAT_NAME) {
            return Err(AppError::Import(vec![ImportError::NotAthena]));
        }
        let Some(ver) = obj
            .get("format_version")
            .and_then(serde_json::Value::as_u64)
        else {
            return Err(AppError::Import(vec![ImportError::InvalidVersion]));
        };
        if ver > u64::from(FORMAT_VERSION) {
            return Err(AppError::Import(vec![ImportError::TooNew {
                found: ver,
                supported: FORMAT_VERSION,
            }]));
        }
        return match ver {
            2 => {
                let doc: DocumentV2 = serde_json::from_value(value).map_err(|e| {
                    AppError::Import(vec![ImportError::MalformedJson(e.to_string())])
                })?;
                Ok(adapt_v2(doc))
            }
            3 => {
                let doc: DocumentV3 = serde_json::from_value(value).map_err(|e| {
                    AppError::Import(vec![ImportError::MalformedJson(e.to_string())])
                })?;
                Ok(adapt_v3(doc))
            }
            4 => {
                let doc: DocumentV4 = serde_json::from_value(value).map_err(|e| {
                    AppError::Import(vec![ImportError::MalformedJson(e.to_string())])
                })?;
                Ok(adapt_v4(doc))
            }
            _ => Err(AppError::Import(vec![ImportError::InvalidVersion])),
        };
    }
    if let Some(schema_val) = obj.get("schema_version") {
        let Some(ver) = schema_val.as_u64() else {
            return Err(AppError::Import(vec![ImportError::InvalidVersion]));
        };
        if ver > u64::from(FORMAT_VERSION) {
            return Err(AppError::Import(vec![ImportError::TooNew {
                found: ver,
                supported: FORMAT_VERSION,
            }]));
        }
        return match ver {
            1 => {
                let doc: DocumentV1 = serde_json::from_value(value).map_err(|e| {
                    AppError::Import(vec![ImportError::MalformedJson(e.to_string())])
                })?;
                Ok(adapt_v1(doc, errors))
            }
            _ => Err(AppError::Import(vec![ImportError::InvalidVersion])),
        };
    }
    Err(AppError::Import(vec![ImportError::NotAthena]))
}

fn adapt_v1(doc: DocumentV1, errors: &mut ErrorCollector) -> RawDocument {
    if !doc.templates.is_empty() {
        errors.push(ImportError::TemplatesNotSupported);
    }
    let fallback_ts = doc.exported_at.clone();
    RawDocument {
        format_version: 1,
        app_version: doc.app_version,
        exported_at: doc.exported_at,
        used_timestamp_fallback: true,
        is_format_1: true,
        start_balance_cents: doc.profile.start_balance,
        theme: doc.profile.theme,
        language: doc.profile.language,
        onboarding_complete: doc.profile.onboarding_complete,
        people: doc
            .people
            .into_iter()
            .map(|p| RawPerson {
                id: p.id,
                name: p.name,
                kind: p.kind,
                archived: p.archived,
                created_at: p.created_at,
            })
            .collect(),
        categories: doc
            .categories
            .into_iter()
            .map(|c| RawCategory {
                id: c.id,
                name: c.name,
                color: c.color,
                icon: c.icon,
                applies_to: c.applies_to,
                is_default: c.is_default,
                archived: c.archived,
                created_at: c.created_at,
            })
            .collect(),
        transactions: doc
            .transactions
            .into_iter()
            .map(|t| RawTransaction {
                id: t.id,
                title: t.title,
                amount_cents: t.amount,
                date: t.date,
                kind: t.kind,
                category_id: t.category_id,
                payer_id: t.payer_id,
                description: t.description,
                split_mode: t.split_mode,
                created_at: fallback_ts.clone(),
                updated_at: fallback_ts.clone(),
                allocations: t
                    .allocations
                    .into_iter()
                    .map(|a| (a.person_id, a.amount))
                    .collect(),
            })
            .collect(),
        templates: Vec::new(),
        appearance: None,
    }
}

fn adapt_v2(doc: DocumentV2) -> RawDocument {
    let mut raw = adapt_shared(
        doc.format_version,
        doc.app_version,
        doc.exported_at,
        doc.profile,
        doc.people,
        doc.categories,
        doc.transactions,
    );
    raw.templates = Vec::new();
    raw
}

fn adapt_v3(doc: DocumentV3) -> RawDocument {
    let mut raw = adapt_shared(
        doc.format_version,
        doc.app_version,
        doc.exported_at,
        doc.profile,
        doc.people,
        doc.categories,
        doc.transactions,
    );
    raw.templates = doc
        .templates
        .into_iter()
        .map(|t| RawTemplate {
            id: t.id,
            name: t.name,
            title: t.title,
            amount_cents: t.amount_cents,
            kind: t.kind,
            category_id: t.category_id,
            payer_id: t.payer_id,
            responsibility: t.responsibility,
            description: t.description,
            created_at: t.created_at,
            last_used_at: t.last_used_at,
            allocations: t
                .allocations
                .into_iter()
                .map(|a| (a.person_id, a.amount_cents))
                .collect(),
        })
        .collect();
    raw
}

fn adapt_v4(doc: DocumentV4) -> RawDocument {
    let v3 = DocumentV3 {
        format: doc.format,
        format_version: doc.format_version,
        app_version: doc.app_version,
        exported_at: doc.exported_at,
        profile: doc.profile,
        people: doc.people,
        categories: doc.categories,
        transactions: doc.transactions,
        templates: doc.templates,
    };
    let mut raw = adapt_v3(v3);
    raw.appearance = doc.appearance;
    raw
}

/// Validate the portable appearance section. Every enum must be in its allowlist and every number
/// in range; a local wallpaper never appears in a document.
fn validate_appearance(raw: &AppearanceV4) -> Option<AppearancePrefs> {
    let wallpaper = match raw.background.as_str() {
        "none" => Wallpaper::None,
        "builtin" => Wallpaper::Builtin,
        _ => return None,
    };
    let dim = u8::try_from(raw.dim).ok().filter(|d| *d <= 100)?;
    Some(AppearancePrefs {
        theme_mode: ThemeMode::from_code(&raw.theme_mode)?,
        preset: Preset::from_code(&raw.preset)?,
        accent: Accent::from_code(&raw.accent)?,
        density: Density::from_code(&raw.density)?,
        text_scale: TextScale::from_percent(raw.text_scale)?,
        motion: MotionPref::from_code(&raw.motion)?,
        reduce_transparency: raw.reduce_transparency,
        quality: Quality::from_code(&raw.quality)?,
        wallpaper,
        fit: Fit::from_code(&raw.fit)?,
        dim,
        paused: raw.paused,
    })
}

/// The record shapes format 2 and 3 share, as a [`RawDocument`] without templates.
fn adapt_shared(
    format_version: u32,
    app_version: String,
    exported_at: String,
    profile: ProfileV2,
    people: Vec<PersonV2>,
    categories: Vec<CategoryV2>,
    transactions: Vec<TransactionV2>,
) -> RawDocument {
    RawDocument {
        format_version,
        app_version,
        exported_at,
        used_timestamp_fallback: false,
        is_format_1: false,
        start_balance_cents: profile.start_balance_cents,
        theme: profile.theme,
        language: profile.language,
        onboarding_complete: profile.onboarding_complete,
        people: people
            .into_iter()
            .map(|p| RawPerson {
                id: p.id,
                name: p.name,
                kind: p.kind,
                archived: p.archived,
                created_at: p.created_at,
            })
            .collect(),
        categories: categories
            .into_iter()
            .map(|c| RawCategory {
                id: c.id,
                name: c.name,
                color: c.color,
                icon: c.icon,
                applies_to: c.applies_to,
                is_default: c.is_default,
                archived: c.archived,
                created_at: c.created_at,
            })
            .collect(),
        transactions: transactions
            .into_iter()
            .map(|t| RawTransaction {
                id: t.id,
                title: t.title,
                amount_cents: t.amount_cents,
                date: t.date,
                kind: t.kind,
                category_id: t.category_id,
                payer_id: t.payer_id,
                description: t.description,
                split_mode: t.split_mode,
                created_at: t.created_at,
                updated_at: t.updated_at,
                allocations: t
                    .allocations
                    .into_iter()
                    .map(|a| (a.person_id, a.amount_cents))
                    .collect(),
            })
            .collect(),
        templates: Vec::new(),
        appearance: None,
    }
}

fn is_rfc3339(ts: &str) -> bool {
    DateTime::parse_from_rfc3339(ts).is_ok()
}

fn parse_iso_date(s: &str) -> Option<NaiveDate> {
    let b = s.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b[..4].iter().all(u8::is_ascii_digit)
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || !b[8..10].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn is_hex_color(color: &str) -> bool {
    let b = color.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(u8::is_ascii_hexdigit)
}

fn validate_raw(
    raw: RawDocument,
    source_name: String,
    mut errors: ErrorCollector,
) -> Result<ImportPlan> {
    if !is_rfc3339(&raw.exported_at) {
        errors.push(ImportError::InvalidExportedAt);
    }
    if !raw.onboarding_complete {
        errors.push(ImportError::OnboardingIncomplete);
    }
    let theme = Theme::from_code(&raw.theme).unwrap_or_else(|| {
        errors.push(ImportError::InvalidTheme);
        Theme::Dark
    });
    let language = Language::from_code(&raw.language).unwrap_or_else(|| {
        errors.push(ImportError::InvalidLanguage);
        Language::En
    });

    let (mut people, seen_person_ids) = validate_people(&raw.people, &mut errors);
    let (mut categories, seen_category_ids, category_scopes) =
        validate_categories(&raw.categories, &mut errors);
    let mut widen = HashSet::new();
    let mut transactions = validate_transactions(
        &raw,
        &seen_person_ids,
        &seen_category_ids,
        &category_scopes,
        &mut widen,
        &mut errors,
    );

    let mut templates = validate_templates(
        &raw.templates,
        &seen_person_ids,
        &seen_category_ids,
        &mut errors,
    );

    if !check_aggregates_and_settlement(raw.start_balance_cents, &transactions) {
        errors.push(ImportError::Overflow);
    }
    let appearance = raw.appearance.as_ref().and_then(|a| {
        let valid = validate_appearance(a);
        if valid.is_none() {
            errors.push(ImportError::InvalidAppearance);
        }
        valid
    });

    if !errors.is_empty() {
        return Err(AppError::Import(errors.0));
    }

    let widened_categories = widen_scopes(&mut categories, &widen);

    people.sort_by_key(|p| p.id);
    categories.sort_by_key(|c| c.id);
    transactions.sort_by_key(|t| (t.date, t.id));
    for tx in &mut transactions {
        tx.allocations.sort_by_key(|a| a.person_id);
    }
    templates.sort_by_key(|t| t.id);
    for template in &mut templates {
        template.draft.allocations.sort_by_key(|a| a.person_id);
    }

    Ok(ImportPlan {
        source_name,
        format_version: raw.format_version,
        app_version: raw.app_version,
        exported_at: raw.exported_at,
        used_timestamp_fallback: raw.used_timestamp_fallback,
        profile: Profile {
            start_balance: Money::from_cents(raw.start_balance_cents),
            theme,
            language,
            onboarding_complete: raw.onboarding_complete,
        },
        widened_categories,
        people,
        categories,
        transactions,
        templates,
        appearance,
    })
}

/// Replace a rejected mismatch with the narrowest scope that keeps every imported entry valid.
/// Returns the affected names, sorted and deduplicated, for the preview notice.
fn widen_scopes(categories: &mut [Category], widen: &HashSet<i64>) -> Vec<String> {
    let mut names: Vec<String> = categories
        .iter_mut()
        .filter(|c| widen.contains(&c.id))
        .map(|c| {
            c.applies_to = CategoryScope::Both;
            c.name.clone()
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

fn validate_people(
    raw_people: &[RawPerson],
    errors: &mut ErrorCollector,
) -> (Vec<Person>, HashSet<i64>) {
    let mut seen_ids = HashSet::new();
    let mut seen_active_names = HashSet::new();
    let mut owner_count = 0usize;
    let mut out = Vec::with_capacity(raw_people.len());

    for (index, p) in raw_people.iter().enumerate() {
        if p.id <= 0 {
            errors.push(ImportError::NonPositivePersonId { index, id: p.id });
        } else if !seen_ids.insert(p.id) {
            errors.push(ImportError::DuplicatePersonId { id: p.id });
        }
        let trimmed = p.name.trim();
        if trimmed.is_empty() {
            errors.push(ImportError::BlankPersonName { id: p.id });
        } else if !p.archived && !seen_active_names.insert(fold(trimmed)) {
            errors.push(ImportError::DuplicatePersonName {
                name: p.name.clone(),
            });
        }
        let kind = match PersonKind::from_code(&p.kind) {
            Some(PersonKind::Owner) => {
                owner_count += 1;
                if p.archived {
                    errors.push(ImportError::OwnerArchived);
                }
                PersonKind::Owner
            }
            Some(k) => k,
            None => {
                errors.push(ImportError::InvalidPersonKind { id: p.id });
                PersonKind::Other
            }
        };
        if !is_rfc3339(&p.created_at) {
            errors.push(ImportError::InvalidPersonTimestamp { id: p.id });
        }
        out.push(Person {
            id: p.id,
            name: p.name.clone(),
            kind,
            archived: p.archived,
            created_at: p.created_at.clone(),
        });
    }
    if owner_count != 1 {
        errors.push(ImportError::OwnerCount { count: owner_count });
    }
    (out, seen_ids)
}

fn validate_categories(
    raw_categories: &[RawCategory],
    errors: &mut ErrorCollector,
) -> (Vec<Category>, HashSet<i64>, HashMap<i64, CategoryScope>) {
    let mut seen_ids = HashSet::new();
    let mut seen_names = HashSet::new();
    let mut scopes = HashMap::new();
    let mut out = Vec::with_capacity(raw_categories.len());

    for (index, c) in raw_categories.iter().enumerate() {
        if c.id <= 0 {
            errors.push(ImportError::NonPositiveCategoryId { index, id: c.id });
        } else if !seen_ids.insert(c.id) {
            errors.push(ImportError::DuplicateCategoryId { id: c.id });
        }
        let trimmed = c.name.trim();
        if trimmed.is_empty() {
            errors.push(ImportError::BlankCategoryName { id: c.id });
        } else if !seen_names.insert(fold(trimmed)) {
            errors.push(ImportError::DuplicateCategoryName {
                name: c.name.clone(),
            });
        }
        if !is_hex_color(&c.color) {
            errors.push(ImportError::InvalidCategoryColor { id: c.id });
        }
        let scope = if let Some(s) = CategoryScope::from_code(&c.applies_to) {
            scopes.entry(c.id).or_insert(s);
            s
        } else {
            errors.push(ImportError::InvalidCategoryScope { id: c.id });
            CategoryScope::Expense
        };
        if !is_rfc3339(&c.created_at) {
            errors.push(ImportError::InvalidCategoryTimestamp { id: c.id });
        }
        out.push(Category {
            id: c.id,
            name: c.name.clone(),
            color: c.color.clone(),
            icon: c.icon.clone(),
            applies_to: scope,
            is_default: c.is_default,
            archived: c.archived,
            created_at: c.created_at.clone(),
        });
    }
    (out, seen_ids, scopes)
}

fn validate_transactions(
    raw: &RawDocument,
    seen_person_ids: &HashSet<i64>,
    seen_category_ids: &HashSet<i64>,
    category_scopes: &HashMap<i64, CategoryScope>,
    widen: &mut HashSet<i64>,
    errors: &mut ErrorCollector,
) -> Vec<Transaction> {
    let mut seen_ids = HashSet::new();
    let mut out = Vec::with_capacity(raw.transactions.len());

    for (index, t) in raw.transactions.iter().enumerate() {
        if t.id <= 0 {
            errors.push(ImportError::NonPositiveTransactionId { index, id: t.id });
        } else if !seen_ids.insert(t.id) {
            errors.push(ImportError::DuplicateTransactionId { id: t.id });
        }
        if t.title.trim().is_empty() {
            errors.push(ImportError::BlankTransactionTitle { id: t.id });
        }
        if t.amount_cents <= 0 {
            errors.push(ImportError::NonPositiveAmount { id: t.id });
        }
        let parsed_date = parse_iso_date(&t.date).unwrap_or_else(|| {
            errors.push(ImportError::InvalidDate { id: t.id });
            NaiveDate::MIN
        });
        let parsed_kind = TransactionType::from_code(&t.kind)
            .filter(|k| !(raw.is_format_1 && *k == TransactionType::Settlement));
        if parsed_kind.is_none() {
            errors.push(ImportError::InvalidTransactionKind { id: t.id });
        }
        validate_tx_refs(
            t,
            parsed_kind,
            seen_person_ids,
            seen_category_ids,
            category_scopes,
            widen,
            errors,
        );
        let allocations = validate_tx_allocations(t, seen_person_ids, errors);
        let split_mode = validate_tx_split_and_shape(t, parsed_kind, errors);
        if !raw.used_timestamp_fallback
            && (!is_rfc3339(&t.created_at) || !is_rfc3339(&t.updated_at))
        {
            errors.push(ImportError::InvalidTransactionTimestamp { id: t.id });
        }
        if let Some(kind) = parsed_kind
            && parsed_date != NaiveDate::MIN
            && t.amount_cents > 0
        {
            out.push(Transaction {
                id: t.id,
                title: t.title.clone(),
                amount: Money::from_cents(t.amount_cents),
                date: parsed_date,
                kind,
                category_id: t.category_id,
                payer_id: t.payer_id,
                description: t.description.clone(),
                split_mode,
                created_at: t.created_at.clone(),
                updated_at: t.updated_at.clone(),
                allocations,
            });
        }
    }
    out
}

/// Participant existence and internal share consistency (no duplicates, no negatives, a checked
/// total equal to the amount). Returns `(people_exist, shape_ok)`.
fn check_template_allocations(
    t: &RawTemplate,
    seen_person_ids: &HashSet<i64>,
    errors: &mut ErrorCollector,
) -> (bool, bool) {
    let mut people = HashSet::new();
    let mut sum: Option<i64> = Some(0);
    let (mut exist, mut shape_ok) = (true, true);
    for &(person_id, cents) in &t.allocations {
        if !seen_person_ids.contains(&person_id) {
            errors.push(ImportError::TemplateMissingPerson {
                id: t.id,
                person_id,
            });
            exist = false;
        }
        if !people.insert(person_id) || cents < 0 {
            shape_ok = false;
        }
        sum = sum.and_then(|acc| acc.checked_add(cents));
    }
    if !t.allocations.is_empty() && sum != Some(t.amount_cents) {
        shape_ok = false;
    }
    (exist, shape_ok)
}

/// Template references and shape (contract §14.1). Archived references and category scope drift
/// are portable history, so only ids and internal allocation shape are checked here.
fn validate_templates(
    raw_templates: &[RawTemplate],
    seen_person_ids: &HashSet<i64>,
    seen_category_ids: &HashSet<i64>,
    errors: &mut ErrorCollector,
) -> Vec<Template> {
    let mut seen_ids = HashSet::new();
    let mut seen_names = HashSet::new();
    let mut out = Vec::with_capacity(raw_templates.len());
    for (index, t) in raw_templates.iter().enumerate() {
        let mut ok = true;
        if t.id <= 0 {
            errors.push(ImportError::NonPositiveTemplateId { index, id: t.id });
            ok = false;
        } else if !seen_ids.insert(t.id) {
            errors.push(ImportError::DuplicateTemplateId { id: t.id });
            ok = false;
        }
        let name = t.name.trim();
        if name.is_empty() {
            errors.push(ImportError::BlankTemplateName { id: t.id });
            ok = false;
        } else if !seen_names.insert(name_key(name)) {
            errors.push(ImportError::DuplicateTemplateName {
                name: t.name.clone(),
            });
            ok = false;
        }
        if t.title.trim().is_empty() {
            errors.push(ImportError::BlankTemplateTitle { id: t.id });
            ok = false;
        }
        if t.amount_cents <= 0 {
            errors.push(ImportError::NonPositiveTemplateAmount { id: t.id });
            ok = false;
        }
        let kind =
            TransactionType::from_code(&t.kind).filter(|k| *k != TransactionType::Settlement);
        if kind.is_none() {
            errors.push(ImportError::InvalidTemplateKind { id: t.id });
            ok = false;
        }
        let mode = ResponsibilityMode::from_code(&t.responsibility);
        if mode.is_none() {
            errors.push(ImportError::InvalidTemplateResponsibility { id: t.id });
            ok = false;
        }
        if let Some(category_id) = t.category_id
            && !seen_category_ids.contains(&category_id)
        {
            errors.push(ImportError::TemplateMissingCategory {
                id: t.id,
                category_id,
            });
            ok = false;
        }
        if let Some(payer_id) = t.payer_id
            && !seen_person_ids.contains(&payer_id)
        {
            errors.push(ImportError::TemplateMissingPayer { id: t.id, payer_id });
            ok = false;
        }
        let (people_ok, shape_ok) = check_template_allocations(t, seen_person_ids, errors);
        ok &= people_ok;
        if !is_rfc3339(&t.created_at) || t.last_used_at.as_deref().is_some_and(|s| !is_rfc3339(s)) {
            errors.push(ImportError::InvalidTemplateTimestamp { id: t.id });
            ok = false;
        }
        let (Some(kind), Some(mode)) = (kind, mode) else {
            continue;
        };
        let draft = TemplateDraft {
            name: name.to_string(),
            title: t.title.trim().to_string(),
            amount: Money::from_cents(t.amount_cents),
            kind,
            category_id: t.category_id,
            payer_id: t.payer_id,
            description: t.description.clone(),
            responsibility: mode,
            allocations: t
                .allocations
                .iter()
                .map(|&(person_id, cents)| Allocation {
                    person_id,
                    amount: Money::from_cents(cents),
                })
                .collect(),
        };
        // Shape rules run only on internally consistent numbers, so they cannot overflow.
        if ok && (!shape_ok || t.amount_cents <= 0 || draft.validate().is_err()) {
            errors.push(ImportError::InvalidTemplateShape { id: t.id });
            ok = false;
        }
        if ok {
            out.push(Template {
                id: t.id,
                draft,
                created_at: t.created_at.clone(),
                last_used_at: t.last_used_at.clone(),
            });
        }
    }
    out
}

fn validate_tx_refs(
    t: &RawTransaction,
    parsed_kind: Option<TransactionType>,
    seen_person_ids: &HashSet<i64>,
    seen_category_ids: &HashSet<i64>,
    category_scopes: &HashMap<i64, CategoryScope>,
    widen: &mut HashSet<i64>,
    errors: &mut ErrorCollector,
) {
    if let Some(cat_id) = t.category_id {
        if !seen_category_ids.contains(&cat_id) {
            errors.push(ImportError::MissingCategory {
                id: t.id,
                category_id: cat_id,
            });
        } else if let (Some(scope), Some(kind)) = (category_scopes.get(&cat_id), parsed_kind)
            && kind != TransactionType::Settlement
            && !scope.allows(kind)
        {
            // Older exports can hold entries whose category scope was narrowed afterwards. Widening
            // the category keeps the history intact instead of rejecting the whole import.
            widen.insert(cat_id);
        }
    }
    if let Some(payer_id) = t.payer_id
        && !seen_person_ids.contains(&payer_id)
    {
        errors.push(ImportError::MissingPayer { id: t.id, payer_id });
    }
}

fn validate_tx_allocations(
    t: &RawTransaction,
    seen_person_ids: &HashSet<i64>,
    errors: &mut ErrorCollector,
) -> Vec<Allocation> {
    let mut seen_people = HashSet::new();
    let mut sum: Option<i64> = Some(0);
    let mut out = Vec::with_capacity(t.allocations.len());
    for &(person_id, amount_cents) in &t.allocations {
        if !seen_person_ids.contains(&person_id) {
            errors.push(ImportError::MissingAllocationPerson {
                id: t.id,
                person_id,
            });
        }
        if !seen_people.insert(person_id) {
            errors.push(ImportError::DuplicateAllocationPerson {
                id: t.id,
                person_id,
            });
        }
        if amount_cents < 0 {
            errors.push(ImportError::NegativeAllocationShare { id: t.id });
        }
        sum = sum.and_then(|acc| acc.checked_add(amount_cents));
        out.push(Allocation {
            person_id,
            amount: Money::from_cents(amount_cents),
        });
    }
    if !t.allocations.is_empty() && sum != Some(t.amount_cents) {
        errors.push(ImportError::AllocationSumMismatch { id: t.id });
    }
    out
}

fn validate_tx_split_and_shape(
    t: &RawTransaction,
    parsed_kind: Option<TransactionType>,
    errors: &mut ErrorCollector,
) -> Option<SplitMode> {
    let split_mode = match &t.split_mode {
        None => None,
        Some(code) => {
            if let Some(mode) = SplitMode::from_code(code) {
                if parsed_kind != Some(TransactionType::Settlement) && t.allocations.len() < 2 {
                    errors.push(ImportError::SplitModeWithoutSharedSplit { id: t.id });
                }
                Some(mode)
            } else {
                errors.push(ImportError::InvalidSplitMode { id: t.id });
                None
            }
        }
    };
    if parsed_kind == Some(TransactionType::Settlement) {
        let valid_shape = t.payer_id.is_some()
            && t.category_id.is_none()
            && t.split_mode.is_none()
            && t.allocations.len() == 1
            && t.allocations[0].1 == t.amount_cents
            && t.payer_id != Some(t.allocations[0].0);
        if !valid_shape {
            errors.push(ImportError::InvalidSettlementShape { id: t.id });
        }
    }
    split_mode
}

#[derive(Default)]
struct TotalsBucket {
    income: i64,
    expenses: i64,
    refunds: i64,
}

impl TotalsBucket {
    fn add_tx(&mut self, kind: TransactionType, amount: i64) -> bool {
        match kind {
            TransactionType::Income => {
                let Some(next) = self.income.checked_add(amount) else {
                    return false;
                };
                self.income = next;
            }
            TransactionType::Expense => {
                let Some(next) = self.expenses.checked_add(amount) else {
                    return false;
                };
                self.expenses = next;
            }
            TransactionType::Refund => {
                let Some(next) = self.refunds.checked_add(amount) else {
                    return false;
                };
                self.refunds = next;
            }
            TransactionType::Settlement => {}
        }
        true
    }

    fn fits_i64(&self, start_balance_cents: i64) -> bool {
        if self.income.checked_add(self.refunds).is_none()
            || self.expenses.checked_sub(self.refunds).is_none()
        {
            return false;
        }
        let net_i128 =
            i128::from(self.income) - i128::from(self.expenses) + i128::from(self.refunds);
        let Ok(net) = i64::try_from(net_i128) else {
            return false;
        };
        start_balance_cents.checked_add(net).is_some()
    }
}

fn check_aggregates_and_settlement(start_balance_cents: i64, txs: &[Transaction]) -> bool {
    let mut lifetime = TotalsBucket::default();
    let mut by_month: BTreeMap<(i32, u32), TotalsBucket> = BTreeMap::new();
    for tx in txs {
        let amt = tx.amount.cents();
        if !lifetime.add_tx(tx.kind, amt) {
            return false;
        }
        let ym = (tx.date.year(), tx.date.month());
        if !by_month.entry(ym).or_default().add_tx(tx.kind, amt) {
            return false;
        }
    }
    if !lifetime.fits_i64(start_balance_cents) {
        return false;
    }
    if !by_month.values().all(|b| b.fits_i64(start_balance_cents)) {
        return false;
    }
    check_settlement_prefixes(txs)
}

fn check_settlement_prefixes(txs: &[Transaction]) -> bool {
    let mut sorted: Vec<&Transaction> = txs.iter().collect();
    sorted.sort_by_key(|t| (t.date, t.id));
    let mut totals: BTreeMap<i64, (i64, i64)> = BTreeMap::new();
    let mut current_ym: Option<(i32, u32)> = None;

    for tx in sorted {
        let ym = (tx.date.year(), tx.date.month());
        if let Some(prev) = current_ym
            && ym != prev
            && !check_month_end_balances(&totals)
        {
            return false;
        }
        current_ym = Some(ym);
        if !accumulate_settlement_tx(&mut totals, tx) {
            return false;
        }
    }
    check_month_end_balances(&totals)
}

fn accumulate_settlement_tx(totals: &mut BTreeMap<i64, (i64, i64)>, tx: &Transaction) -> bool {
    match tx.kind {
        TransactionType::Income => true,
        TransactionType::Expense | TransactionType::Refund => {
            if tx.allocations.is_empty()
                || tx.payer_id.is_none()
                || !allocations_valid(tx.amount.cents(), &tx.allocations)
            {
                return true;
            }
            let Some(payer_id) = tx.payer_id else {
                return true;
            };
            let sign = if tx.kind == TransactionType::Refund {
                -1i64
            } else {
                1i64
            };
            let Some(paid_delta) = tx.amount.cents().checked_mul(sign) else {
                return false;
            };
            if !add_person_delta(totals, payer_id, paid_delta, 0) {
                return false;
            }
            for alloc in &tx.allocations {
                let Some(owed_delta) = alloc.amount.cents().checked_mul(sign) else {
                    return false;
                };
                if !add_person_delta(totals, alloc.person_id, 0, owed_delta) {
                    return false;
                }
            }
            true
        }
        TransactionType::Settlement => {
            let (Some(from), [recipient]) = (tx.payer_id, tx.allocations.as_slice()) else {
                return true;
            };
            if tx.amount.cents() <= 0
                || recipient.person_id == from
                || recipient.amount != tx.amount
            {
                return true;
            }
            let amt = tx.amount.cents();
            add_person_delta(totals, from, amt, 0)
                && add_person_delta(totals, recipient.person_id, 0, amt)
        }
    }
}

fn allocations_valid(amount: i64, allocations: &[Allocation]) -> bool {
    let mut seen = HashSet::with_capacity(allocations.len());
    let mut sum = 0i64;
    for a in allocations {
        if a.amount.cents() < 0 || !seen.insert(a.person_id) {
            return false;
        }
        let Some(next) = sum.checked_add(a.amount.cents()) else {
            return false;
        };
        sum = next;
    }
    sum == amount
}

fn add_person_delta(
    totals: &mut BTreeMap<i64, (i64, i64)>,
    person_id: i64,
    paid_delta: i64,
    owed_delta: i64,
) -> bool {
    let entry = totals.entry(person_id).or_insert((0, 0));
    let (Some(p), Some(o)) = (
        entry.0.checked_add(paid_delta),
        entry.1.checked_add(owed_delta),
    ) else {
        return false;
    };
    *entry = (p, o);
    true
}

fn check_month_end_balances(totals: &BTreeMap<i64, (i64, i64)>) -> bool {
    for &(paid, owed) in totals.values() {
        let Some(balance) = paid.checked_sub(owed) else {
            return false;
        };
        if balance < 0 && 0i64.checked_sub(balance).is_none() {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::application::services::export::export_json;
    use crate::domain::person::PersonKind;
    use crate::infrastructure::db::{Db, categories_repo, people_repo, transactions_repo};
    use crate::shared::test_fixtures::{date, draft_with, profile, seed_all_kinds};

    fn valid_v2_value() -> Value {
        json!({
            "format": "athena-profile",
            "format_version": 2,
            "app_version": "1.1.0",
            "exported_at": "2026-11-16T14:03:00Z",
            "profile": {
                "start_balance_cents": 0,
                "theme": "dark",
                "language": "en",
                "onboarding_complete": true
            },
            "people": [
                {
                    "id": 1,
                    "name": "Owner",
                    "kind": "owner",
                    "archived": false,
                    "created_at": "2026-11-01T10:00:00Z"
                },
                {
                    "id": 2,
                    "name": "Ana",
                    "kind": "partner",
                    "archived": false,
                    "created_at": "2026-11-01T10:00:00Z"
                }
            ],
            "categories": [
                {
                    "id": 1,
                    "name": "Food",
                    "color": "#FF8A65",
                    "icon": null,
                    "applies_to": "expense",
                    "is_default": true,
                    "archived": false,
                    "created_at": "2026-11-01T10:00:00Z"
                },
                {
                    "id": 2,
                    "name": "Salary",
                    "color": "#3DDC97",
                    "icon": null,
                    "applies_to": "income",
                    "is_default": true,
                    "archived": false,
                    "created_at": "2026-11-01T10:00:00Z"
                }
            ],
            "transactions": [
                {
                    "id": 1,
                    "title": "Lunch",
                    "amount_cents": 2000,
                    "date": "2026-11-10",
                    "kind": "expense",
                    "category_id": 1,
                    "payer_id": 1,
                    "description": null,
                    "split_mode": "even",
                    "created_at": "2026-11-10T12:00:00Z",
                    "updated_at": "2026-11-10T12:00:00Z",
                    "allocations": [
                        { "person_id": 1, "amount_cents": 1000 },
                        { "person_id": 2, "amount_cents": 1000 }
                    ]
                }
            ]
        })
    }

    fn parse_val(val: &Value) -> std::result::Result<ImportPlan, Vec<ImportError>> {
        let bytes = serde_json::to_vec(val).unwrap();
        match parse_and_validate(&bytes, "test.json") {
            Ok(plan) => Ok(plan),
            Err(AppError::Import(errs)) => Err(errs),
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn format_1_fixture_adapts() {
        let raw = include_bytes!("../../../tests/fixtures/format1-1.0.2.json");
        let plan = parse_and_validate(raw, "format1-1.0.2.json").unwrap();
        assert_eq!(plan.format_version, 1);
        assert!(plan.used_timestamp_fallback);
        assert_eq!(plan.app_version, "1.0.2");
        assert_eq!(plan.exported_at, "2026-11-12T10:00:00Z");
        assert_eq!(plan.profile.start_balance, Money::from_cents(100_000));
        assert_eq!(plan.people.len(), 2);
        assert_eq!(plan.categories.len(), 2);
        assert_eq!(plan.transactions.len(), 3);
        for tx in &plan.transactions {
            assert_eq!(tx.created_at, "2026-11-12T10:00:00Z");
            assert_eq!(tx.updated_at, "2026-11-12T10:00:00Z");
        }
    }

    #[test]
    fn format_1_with_templates_is_invalid() {
        let mut val: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/format1-1.0.2.json"))
                .unwrap();
        val["templates"] = json!([{"id": 1}]);
        let errs = parse_val(&val).unwrap_err();
        assert!(
            errs.contains(&ImportError::TemplatesNotSupported),
            "{errs:?}"
        );
    }

    #[test]
    fn format_1_cannot_contain_settlements() {
        let mut val: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/format1-1.0.2.json"))
                .unwrap();
        val["transactions"].as_array_mut().unwrap().push(json!({
            "id": 9,
            "title": "Payback",
            "amount": 500,
            "date": "2026-11-11",
            "kind": "settlement",
            "category_id": null,
            "payer_id": 2,
            "description": null,
            "split_mode": null,
            "allocations": [{ "person_id": 1, "amount": 500 }]
        }));
        let errs = parse_val(&val).unwrap_err();
        assert!(
            errs.contains(&ImportError::InvalidTransactionKind { id: 9 }),
            "{errs:?}"
        );
    }

    #[test]
    fn future_format_is_rejected_before_parsing() {
        let v2_future = json!({
            "format": "athena-profile",
            "format_version": 99,
            "completely_invalid_body": [1, 2, 3]
        });
        assert_eq!(
            parse_val(&v2_future).unwrap_err(),
            vec![ImportError::TooNew {
                found: 99,
                supported: FORMAT_VERSION
            }]
        );

        let v1_future = json!({
            "schema_version": 99,
            "completely_invalid_body": true
        });
        assert_eq!(
            parse_val(&v1_future).unwrap_err(),
            vec![ImportError::TooNew {
                found: 99,
                supported: FORMAT_VERSION
            }]
        );
    }

    #[test]
    fn unknown_fields_are_rejected_in_both_formats() {
        let mut v2 = valid_v2_value();
        v2["extra_root"] = json!(true);
        assert!(matches!(
            parse_val(&v2).unwrap_err().as_slice(),
            [ImportError::MalformedJson(_)]
        ));

        let mut v2_tx = valid_v2_value();
        v2_tx["transactions"][0]["extra_tx"] = json!(1);
        assert!(matches!(
            parse_val(&v2_tx).unwrap_err().as_slice(),
            [ImportError::MalformedJson(_)]
        ));

        let mut v1: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/format1-1.0.2.json"))
                .unwrap();
        v1["transactions"][0]["created_at"] = json!("2026-11-12T10:00:00Z");
        assert!(matches!(
            parse_val(&v1).unwrap_err().as_slice(),
            [ImportError::MalformedJson(_)]
        ));
    }

    #[test]
    fn not_athena_json_is_rejected() {
        for val in [
            json!({}),
            json!([]),
            json!({"format": "other", "format_version": 2}),
        ] {
            assert_eq!(parse_val(&val).unwrap_err(), vec![ImportError::NotAthena]);
        }
    }

    #[test]
    fn oversized_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.json");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_IMPORT_BYTES as u64 + 1).unwrap();
        let Err(AppError::Import(errs)) = read_and_validate(&path) else {
            panic!("expected TooLarge");
        };
        assert_eq!(errs, vec![ImportError::TooLarge]);
    }

    #[test]
    fn validator_reports_envelope_and_profile_rules() {
        let mut val = valid_v2_value();
        val["exported_at"] = json!("not-a-timestamp");
        val["profile"]["theme"] = json!("sepia");
        val["profile"]["language"] = json!("fr");
        let errs = parse_val(&val).unwrap_err();
        assert!(errs.contains(&ImportError::InvalidExportedAt));
        assert!(errs.contains(&ImportError::InvalidTheme));
        assert!(errs.contains(&ImportError::InvalidLanguage));

        let mut bad_ver = valid_v2_value();
        bad_ver["format_version"] = json!(1);
        assert_eq!(
            parse_val(&bad_ver).unwrap_err(),
            vec![ImportError::InvalidVersion]
        );
    }

    #[test]
    fn validator_reports_people_rules() {
        let mut val = valid_v2_value();
        val["people"] = json!([
            {"id": 0, "name": " ", "kind": "owner", "archived": true, "created_at": "bad"},
            {"id": 2, "name": "Dup", "kind": "alien", "archived": false, "created_at": "2026-11-01T10:00:00Z"},
            {"id": 2, "name": "Owner2", "kind": "owner", "archived": false, "created_at": "2026-11-01T10:00:00Z"}
        ]);
        let errs = parse_val(&val).unwrap_err();
        assert!(errs.contains(&ImportError::NonPositivePersonId { index: 0, id: 0 }));
        assert!(errs.contains(&ImportError::BlankPersonName { id: 0 }));
        assert!(errs.contains(&ImportError::OwnerArchived));
        assert!(errs.contains(&ImportError::InvalidPersonTimestamp { id: 0 }));
        assert!(errs.contains(&ImportError::InvalidPersonKind { id: 2 }));
        assert!(errs.contains(&ImportError::DuplicatePersonId { id: 2 }));
        assert!(errs.contains(&ImportError::OwnerCount { count: 2 }));
    }

    #[test]
    fn validator_reports_category_rules() {
        let mut val = valid_v2_value();
        val["categories"] = json!([
            {"id": -1, "name": "  ", "color": "red", "icon": null, "applies_to": "bad", "is_default": false, "archived": false, "created_at": "bad"},
            {"id": 1, "name": "Food", "color": "#112233", "icon": null, "applies_to": "expense", "is_default": false, "archived": false, "created_at": "2026-11-01T10:00:00Z"},
            {"id": 1, "name": "food", "color": "#112233", "icon": null, "applies_to": "expense", "is_default": false, "archived": false, "created_at": "2026-11-01T10:00:00Z"}
        ]);
        let errs = parse_val(&val).unwrap_err();
        assert!(errs.contains(&ImportError::NonPositiveCategoryId { index: 0, id: -1 }));
        assert!(errs.contains(&ImportError::BlankCategoryName { id: -1 }));
        assert!(errs.contains(&ImportError::InvalidCategoryColor { id: -1 }));
        assert!(errs.contains(&ImportError::InvalidCategoryScope { id: -1 }));
        assert!(errs.contains(&ImportError::InvalidCategoryTimestamp { id: -1 }));
        assert!(errs.contains(&ImportError::DuplicateCategoryId { id: 1 }));
        assert!(errs.contains(&ImportError::DuplicateCategoryName {
            name: "food".into()
        }));
    }

    #[test]
    fn validator_reports_transaction_scalar_and_ref_rules() {
        let mut val = valid_v2_value();
        val["transactions"] = json!([
            {
                "id": 0, "title": "  ", "amount_cents": 0, "date": "2026-02-30", "kind": "bonus",
                "category_id": 99, "payer_id": 99, "description": null, "split_mode": null,
                "created_at": "bad", "updated_at": "2026-11-10T12:00:00Z", "allocations": []
            },
            {
                "id": 1, "title": "ScopeBad", "amount_cents": 100, "date": "2026-1-5", "kind": "income",
                "category_id": 1, "payer_id": null, "description": null, "split_mode": null,
                "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z", "allocations": []
            },
            {
                "id": 1, "title": "Dup", "amount_cents": 100, "date": "2026-11-10", "kind": "expense",
                "category_id": 1, "payer_id": 1, "description": null, "split_mode": null,
                "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z", "allocations": []
            }
        ]);
        let errs = parse_val(&val).unwrap_err();
        assert!(errs.contains(&ImportError::NonPositiveTransactionId { index: 0, id: 0 }));
        assert!(errs.contains(&ImportError::BlankTransactionTitle { id: 0 }));
        assert!(errs.contains(&ImportError::NonPositiveAmount { id: 0 }));
        assert!(errs.contains(&ImportError::InvalidDate { id: 0 }));
        assert!(errs.contains(&ImportError::InvalidDate { id: 1 }));
        assert!(errs.contains(&ImportError::InvalidTransactionKind { id: 0 }));
        assert!(errs.contains(&ImportError::MissingCategory {
            id: 0,
            category_id: 99
        }));
        assert!(errs.contains(&ImportError::MissingPayer {
            id: 0,
            payer_id: 99
        }));
        assert!(errs.contains(&ImportError::InvalidTransactionTimestamp { id: 0 }));
        assert!(errs.contains(&ImportError::DuplicateTransactionId { id: 1 }));
    }

    #[test]
    fn validator_rejects_an_unfinished_setup_export() {
        let mut val = valid_v2_value();
        val["profile"]["onboarding_complete"] = json!(false);
        assert_eq!(
            parse_val(&val).unwrap_err(),
            vec![ImportError::OnboardingIncomplete]
        );
    }

    #[test]
    fn validator_rejects_duplicate_active_person_names_ignoring_accents() {
        let mut val = valid_v2_value();
        val["people"] = json!([
            {"id": 1, "name": "Owner", "kind": "owner", "archived": false, "created_at": "2026-11-01T10:00:00Z"},
            {"id": 2, "name": "Joao", "kind": "partner", "archived": false, "created_at": "2026-11-01T10:00:00Z"},
            {"id": 3, "name": "JOÃO", "kind": "other", "archived": false, "created_at": "2026-11-01T10:00:00Z"}
        ]);
        let errs = parse_val(&val).unwrap_err();
        assert!(
            errs.contains(&ImportError::DuplicatePersonName {
                name: "JOÃO".into()
            }),
            "{errs:?}"
        );

        // An archived name may be reused by an active person.
        let mut ok = valid_v2_value();
        ok["people"] = json!([
            {"id": 1, "name": "Owner", "kind": "owner", "archived": false, "created_at": "2026-11-01T10:00:00Z"},
            {"id": 2, "name": "Bia", "kind": "other", "archived": true, "created_at": "2026-11-01T10:00:00Z"},
            {"id": 3, "name": "Bia", "kind": "other", "archived": false, "created_at": "2026-11-01T10:00:00Z"}
        ]);
        assert!(parse_val(&ok).is_ok());
    }

    #[test]
    fn category_names_are_compared_ignoring_accents() {
        let mut val = valid_v2_value();
        val["categories"] = json!([
            {"id": 1, "name": "Alimentação", "color": "#112233", "icon": null, "applies_to": "expense", "is_default": false, "archived": false, "created_at": "2026-11-01T10:00:00Z"},
            {"id": 2, "name": "ALIMENTACAO", "color": "#112233", "icon": null, "applies_to": "expense", "is_default": false, "archived": false, "created_at": "2026-11-01T10:00:00Z"}
        ]);
        val["transactions"] = json!([]);
        let errs = parse_val(&val).unwrap_err();
        assert!(
            errs.contains(&ImportError::DuplicateCategoryName {
                name: "ALIMENTACAO".into()
            }),
            "{errs:?}"
        );
    }

    #[test]
    fn legacy_scope_mismatch_widens_the_category_to_both() {
        let mut val = valid_v2_value();
        val["transactions"] = json!([{
            "id": 1, "title": "Income on Food", "amount_cents": 500, "date": "2026-11-10", "kind": "income",
            "category_id": 1, "payer_id": 1, "description": null, "split_mode": null,
            "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z", "allocations": []
        }]);
        let plan = parse_val(&val).unwrap();
        assert_eq!(plan.widened_categories, vec!["Food".to_string()]);
        let food = plan.categories.iter().find(|c| c.id == 1).unwrap();
        assert_eq!(food.applies_to, CategoryScope::Both);
        let salary = plan.categories.iter().find(|c| c.id == 2).unwrap();
        assert_eq!(salary.applies_to, CategoryScope::Income);
        assert_eq!(plan.transactions.len(), 1);
        assert_eq!(plan.transactions[0].category_id, Some(1));
    }

    #[test]
    fn format_1_legacy_scope_widens_and_round_trips() {
        let mut value: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/format1-1.0.2.json"))
                .unwrap();
        for category in value["categories"].as_array_mut().unwrap() {
            category["applies_to"] = json!("income");
        }
        let plan = parse_val(&value).unwrap();
        assert!(!plan.widened_categories.is_empty());
        let db = crate::infrastructure::db::Db::open_in_memory().unwrap();
        crate::infrastructure::db::import_repo::replace(db.conn(), &plan).unwrap();
    }

    #[test]
    fn validator_reports_allocation_split_and_settlement_rules() {
        let mut val = valid_v2_value();
        val["transactions"] = json!([
            {
                "id": 10, "title": "Allocs", "amount_cents": 500, "date": "2026-11-10", "kind": "expense",
                "category_id": 1, "payer_id": 1, "description": null, "split_mode": "even",
                "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z",
                "allocations": [
                    { "person_id": 99, "amount_cents": -10 }
                ]
            },
            {
                "id": 11, "title": "DupAlloc", "amount_cents": 500, "date": "2026-11-10", "kind": "expense",
                "category_id": 1, "payer_id": 1, "description": null, "split_mode": "weird",
                "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z",
                "allocations": [
                    { "person_id": 1, "amount_cents": 200 },
                    { "person_id": 1, "amount_cents": 200 }
                ]
            },
            {
                "id": 12, "title": "BadSettle", "amount_cents": 500, "date": "2026-11-10", "kind": "settlement",
                "category_id": 1, "payer_id": 1, "description": null, "split_mode": null,
                "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z",
                "allocations": [
                    { "person_id": 1, "amount_cents": 500 }
                ]
            }
        ]);
        let errs = parse_val(&val).unwrap_err();
        assert!(errs.contains(&ImportError::MissingAllocationPerson {
            id: 10,
            person_id: 99
        }));
        assert!(errs.contains(&ImportError::NegativeAllocationShare { id: 10 }));
        assert!(errs.contains(&ImportError::SplitModeWithoutSharedSplit { id: 10 }));
        assert!(errs.contains(&ImportError::DuplicateAllocationPerson {
            id: 11,
            person_id: 1
        }));
        assert!(errs.contains(&ImportError::AllocationSumMismatch { id: 11 }));
        assert!(errs.contains(&ImportError::InvalidSplitMode { id: 11 }));
        assert!(errs.contains(&ImportError::InvalidSettlementShape { id: 12 }));
    }

    #[test]
    fn lifetime_sums_overflow_is_rejected() {
        let mut val = valid_v2_value();
        val["profile"]["start_balance_cents"] = json!(10);
        val["transactions"] = json!([
            {
                "id": 1, "title": "HugeIncome", "amount_cents": i64::MAX, "date": "2026-11-10", "kind": "income",
                "category_id": 2, "payer_id": null, "description": null, "split_mode": null,
                "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z",
                "allocations": []
            }
        ]);
        assert_eq!(parse_val(&val).unwrap_err(), vec![ImportError::Overflow]);
    }

    #[test]
    fn monthly_aggregate_overflow_is_rejected() {
        // Month 11 has income + refund > i64::MAX even though month 12 expense keeps lifetime net small.
        let mut val = valid_v2_value();
        val["transactions"] = json!([
            {
                "id": 1, "title": "BigInc", "amount_cents": i64::MAX, "date": "2026-11-10", "kind": "income",
                "category_id": 2, "payer_id": null, "description": null, "split_mode": null,
                "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z",
                "allocations": []
            },
            {
                "id": 2, "title": "Ref", "amount_cents": 1, "date": "2026-11-11", "kind": "refund",
                "category_id": 1, "payer_id": null, "description": null, "split_mode": null,
                "created_at": "2026-11-11T12:00:00Z", "updated_at": "2026-11-11T12:00:00Z",
                "allocations": []
            },
            {
                "id": 3, "title": "Offset", "amount_cents": i64::MAX, "date": "2026-12-01", "kind": "expense",
                "category_id": 1, "payer_id": null, "description": null, "split_mode": null,
                "created_at": "2026-12-01T12:00:00Z", "updated_at": "2026-12-01T12:00:00Z",
                "allocations": []
            }
        ]);
        assert_eq!(parse_val(&val).unwrap_err(), vec![ImportError::Overflow]);
    }

    #[test]
    fn settlement_prefix_overflow_is_rejected() {
        // In Nov, Ana accumulates paid = 0, owed = i64::MAX + 1? Wait: paid = 0, owed = i64::MAX,
        // wait: -i64::MAX negates fine, whereas paid = -1, owed = i64::MAX -> balance = i64::MIN!
        // A refund of 1 paid by Ana gives paid = -1; an expense of i64::MAX owed by Ana gives owed = i64::MAX.
        // Then at Nov month-end balance = -1 - i64::MAX = i64::MIN -> debtor negation overflows!
        // Even if Dec settles 10 back, Nov month-end must fail.
        let mut val = valid_v2_value();
        val["transactions"] = json!([
            {
                "id": 1, "title": "Ref", "amount_cents": 1, "date": "2026-11-10", "kind": "refund",
                "category_id": 1, "payer_id": 2, "description": null, "split_mode": null,
                "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z",
                "allocations": [{ "person_id": 1, "amount_cents": 1 }]
            },
            {
                "id": 2, "title": "HugeExp", "amount_cents": i64::MAX, "date": "2026-11-12", "kind": "expense",
                "category_id": 1, "payer_id": 1, "description": null, "split_mode": null,
                "created_at": "2026-11-12T12:00:00Z", "updated_at": "2026-11-12T12:00:00Z",
                "allocations": [{ "person_id": 2, "amount_cents": i64::MAX }]
            },
            {
                "id": 3, "title": "SettleLater", "amount_cents": 10, "date": "2026-12-01", "kind": "settlement",
                "category_id": null, "payer_id": 2, "description": null, "split_mode": null,
                "created_at": "2026-12-01T12:00:00Z", "updated_at": "2026-12-01T12:00:00Z",
                "allocations": [{ "person_id": 1, "amount_cents": 10 }]
            }
        ]);
        assert_eq!(parse_val(&val).unwrap_err(), vec![ImportError::Overflow]);
    }

    #[test]
    fn error_collector_caps_at_fifty() {
        let mut val = valid_v2_value();
        let bad_txs: Vec<Value> = (1..=60)
            .map(|i| {
                json!({
                    "id": i, "title": "", "amount_cents": 100, "date": "2026-11-10", "kind": "expense",
                    "category_id": 1, "payer_id": 1, "description": null, "split_mode": null,
                    "created_at": "2026-11-10T12:00:00Z", "updated_at": "2026-11-10T12:00:00Z",
                    "allocations": []
                })
            })
            .collect();
        val["transactions"] = Value::Array(bad_txs);
        let errs = parse_val(&val).unwrap_err();
        assert_eq!(errs.len(), MAX_ERRORS);
    }

    #[test]
    fn round_trip_is_canonical() {
        let db = Db::open_in_memory().unwrap();
        seed_all_kinds(&db);
        let extra_person =
            people_repo::insert(db.conn(), "ArchivedFriend", PersonKind::Other).unwrap();
        people_repo::set_archived(db.conn(), extra_person, true).unwrap();
        let extra_cat =
            categories_repo::insert(db.conn(), "OldCat", "#112233", None, CategoryScope::Expense)
                .unwrap();
        categories_repo::set_archived(db.conn(), extra_cat, true).unwrap();

        let mut trashed = draft_with(vec![]);
        trashed.title = "InTrash".into();
        trashed.date = date(2026, 11, 15);
        let trashed_id = transactions_repo::create(db.conn(), &trashed).unwrap();
        transactions_repo::delete(db.conn(), trashed_id).unwrap();

        let prof = profile();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("canonical.json");
        export_json(db.conn(), &prof, "2026-11-16T14:03:00Z", &path).unwrap();

        let mut from_file = read_and_validate(&path).unwrap();
        assert_eq!(from_file.source_name, "canonical.json");
        assert_eq!(from_file.exported_at, "2026-11-16T14:03:00Z");

        let mut from_db = ImportPlan::from_db(db.conn(), &prof).unwrap();
        from_file.source_name.clear();
        from_file.exported_at.clear();
        from_file.app_version.clear();
        from_db.app_version.clear();
        assert_eq!(from_file, from_db);
    }

    #[test]
    fn explicit_version_dispatch_covers_accepted_and_rejected_versions() {
        let mut valid_v1: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/format1-1.0.2.json"))
                .unwrap();
        assert_eq!(parse_val(&valid_v1).unwrap().format_version, 1);

        let v2_fixture: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/format2-1.1.0.json"))
                .unwrap();
        assert_eq!(parse_val(&v2_fixture).unwrap().format_version, 2);

        for bad_v2_ver in [json!(0), json!(1), json!("2"), json!(null)] {
            let mut doc = valid_v2_value();
            doc["format_version"] = bad_v2_ver;
            assert_eq!(
                parse_val(&doc).unwrap_err(),
                vec![ImportError::InvalidVersion]
            );
        }

        for future_ver in [5u64, 6, 99] {
            let mut doc = valid_v2_value();
            doc["format_version"] = json!(future_ver);
            assert_eq!(
                parse_val(&doc).unwrap_err(),
                vec![ImportError::TooNew {
                    found: future_ver,
                    supported: FORMAT_VERSION,
                }]
            );
        }

        for bad_v1_ver in [json!(0), json!(2), json!("1"), json!(null)] {
            let mut doc = valid_v1.clone();
            doc["schema_version"] = bad_v1_ver;
            assert_eq!(
                parse_val(&doc).unwrap_err(),
                vec![ImportError::InvalidVersion]
            );
        }

        // The legacy envelope only ever means format 1; anything above the current format is too
        // new, and the versions in between are not legacy versions.
        valid_v1["schema_version"] = json!(5);
        assert_eq!(
            parse_val(&valid_v1).unwrap_err(),
            vec![ImportError::TooNew {
                found: 5,
                supported: FORMAT_VERSION,
            }]
        );
        valid_v1["schema_version"] = json!(4);
        assert_eq!(
            parse_val(&valid_v1).unwrap_err(),
            vec![ImportError::InvalidVersion]
        );

        // Format 4 (1.3) is accepted through its own adapter, with its appearance.
        let v4_fixture: Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/format4-1.3.0.json"))
                .unwrap();
        let v4 = parse_val(&v4_fixture).unwrap();
        assert_eq!(v4.format_version, 4);
        assert!(v4.appearance.is_some());
    }

    #[test]
    fn every_shipped_fixture_imports_and_round_trips() {
        // Fixture provenance & reproducibility (`tests/fixtures/format2-1.1.0.json`, pinned LF in `.gitattributes`):
        // Generated via `application::services::export::export_json` on shipped `v1.1.0` (`e09f49f`;
        // GUI binary has no CLI export flag) from a disposable DB (`Db::open`) seeded at `2026-11-18T11:50:27Z`:
        // - Profile: `start_balance = 100_000` (`$1,000.00`), `theme = dark`, `language = en`,
        //   `onboarding_complete = true`
        // - People (3): `A3` (owner), `Ana` (partner), `Bia` (other, archived)
        // - Categories (13): 12 defaults from `Db::open` (`Food`..`Other income`) + `Legacy`
        //   (`#607D8B`, expense, archived)
        // - Transactions (5): `Paycheck` (`income` 300000), `Groceries` (`expense` 15000, even split
        //   between A3/Ana, description `"Weekly food"`), `Archived dinner` (`expense` 4000, custom split
        //   using archived `Legacy`/`Bia`), `Store refund` (`refund` 2500), `Ana -> A3` (`settlement` 5000)
        // - Fixed `exported_at = "2026-11-17T12:00:00Z"` (`app_version = "1.1.0"`).
        let v1_bytes = include_bytes!("../../../tests/fixtures/format1-1.0.2.json");
        let v2_bytes = include_bytes!("../../../tests/fixtures/format2-1.1.0.json");
        assert!(
            !v2_bytes.windows(2).any(|w| w == b"\r\n"),
            "format2-1.1.0.json must stay pinned to LF"
        );

        let v1_plan = parse_and_validate(v1_bytes, "format1-1.0.2.json").unwrap();
        assert_eq!(v1_plan.format_version, 1);
        let db1 = Db::open_in_memory().unwrap();
        crate::infrastructure::db::import_repo::replace(db1.conn(), &v1_plan).unwrap();
        assert!(
            crate::infrastructure::db::integrity::check(db1.conn())
                .unwrap()
                .is_empty()
        );

        let mut v2_plan = parse_and_validate(v2_bytes, "format2-1.1.0.json").unwrap();
        assert_eq!(v2_plan.format_version, 2);
        assert_eq!(v2_plan.app_version, "1.1.0");
        assert!(!v2_plan.used_timestamp_fallback);
        assert!(v2_plan.widened_categories.is_empty());
        assert_eq!(v2_plan.people.len(), 3);
        assert_eq!(v2_plan.categories.len(), 13);
        assert_eq!(v2_plan.transactions.len(), 5);
        assert!(
            v2_plan
                .transactions
                .iter()
                .any(|t| t.kind == TransactionType::Settlement)
        );
        assert!(v2_plan.people.iter().any(|p| p.archived));
        assert!(v2_plan.categories.iter().any(|c| c.archived));

        let db2 = Db::open_in_memory().unwrap();
        crate::infrastructure::db::import_repo::replace(db2.conn(), &v2_plan).unwrap();
        assert!(
            crate::infrastructure::db::integrity::check(db2.conn())
                .unwrap()
                .is_empty()
        );

        let mut from_db = ImportPlan::from_db(db2.conn(), &v2_plan.profile).unwrap();
        v2_plan.source_name.clear();
        v2_plan.exported_at.clear();
        v2_plan.app_version.clear();
        from_db.app_version.clear();
        from_db.format_version = v2_plan.format_version;
        assert_eq!(v2_plan, from_db);

        // Format 3 (1.2): same data plus templates, canonical in the current format.
        let v3_bytes = include_bytes!("../../../tests/fixtures/format3-1.2.0.json");
        assert!(
            !v3_bytes.windows(2).any(|w| w
                == b"
"),
            "format3-1.2.0.json must stay pinned to LF"
        );
        let mut v3_plan = parse_and_validate(v3_bytes, "format3-1.2.0.json").unwrap();
        assert_eq!(v3_plan.format_version, 3);
        assert_eq!(v3_plan.templates.len(), 5);
        let db3 = Db::open_in_memory().unwrap();
        crate::infrastructure::db::import_repo::replace(db3.conn(), &v3_plan).unwrap();
        assert!(
            crate::infrastructure::db::integrity::check(db3.conn())
                .unwrap()
                .is_empty()
        );
        let mut from_db3 = ImportPlan::from_db(db3.conn(), &v3_plan.profile).unwrap();
        from_db3.format_version = v3_plan.format_version;
        v3_plan.source_name.clear();
        v3_plan.exported_at.clear();
        v3_plan.app_version.clear();
        from_db3.app_version.clear();
        assert_eq!(v3_plan, from_db3);

        // Format 4 (1.3): the same data plus a non-default portable appearance, canonical now.
        let v4_bytes = include_bytes!("../../../tests/fixtures/format4-1.3.0.json");
        assert!(
            !v4_bytes.windows(2).any(|w| w
                == b"
"),
            "format4-1.3.0.json must stay pinned to LF"
        );
        let mut v4_plan = parse_and_validate(v4_bytes, "format4-1.3.0.json").unwrap();
        assert_eq!(v4_plan.format_version, 4);
        assert!(v4_plan.appearance.is_some());
        let db4 = Db::open_in_memory().unwrap();
        crate::infrastructure::db::import_repo::replace(db4.conn(), &v4_plan).unwrap();
        assert!(
            crate::infrastructure::db::integrity::check(db4.conn())
                .unwrap()
                .is_empty()
        );
        let mut from_db4 = ImportPlan::from_db(db4.conn(), &v4_plan.profile).unwrap();
        v4_plan.source_name.clear();
        v4_plan.exported_at.clear();
        v4_plan.app_version.clear();
        from_db4.app_version.clear();
        assert_eq!(v4_plan, from_db4);
    }
}
