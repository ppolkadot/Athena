//! All user-visible copy, in English and Brazilian Portuguese.
//!
//! Each language is one `const Strings` with a field per key, so a missing translation is a
//! compile error. Services, exporters and screens all read from here; no screen holds a visible
//! string literal.

mod en;
mod insight;
mod pt_br;

pub use insight::{InsightText, insight_text};

use chrono::{Datelike as _, NaiveDate, Weekday};

use crate::domain::date::YearMonth;
use crate::domain::money::MoneyError;
use crate::domain::settings::Language;
use crate::domain::template::TemplateError;
use crate::domain::transaction::{TransactionType, ValidationError};
use crate::infrastructure::db::backup::BackupKind;
use crate::infrastructure::media::MediaError;
use crate::shared::error::{AppError, ImportError, RestoreError};

/// Declares [`Strings`] and a `fields()` listing used by the completeness tests.
macro_rules! strings {
    ($($field:ident),* $(,)?) => {
        /// One `&'static str` per visible label.
        #[derive(Debug)]
        pub struct Strings {
            $(pub $field: &'static str),*
        }

        impl Strings {
            /// `(key, text)` pairs, for tests.
            #[must_use]
            pub fn fields(&self) -> Vec<(&'static str, &'static str)> {
                vec![$((stringify!($field), self.$field)),*]
            }
        }
    };
}

strings! {
    app_name,
    month,
    calendar,
    trash,
    settings,
    version,
    all_time,
    this_month,
    balance,
    income,
    expenses,
    expense,
    refund,
    settlement,
    result,
    new_transaction,
    save,
    cancel,
    delete,
    edit,
    undo,
    close,
    back,
    continue_label,
    skip,
    start,
    search,
    all,
    shared,
    no_one,
    language,
    theme,
    dark,
    light,
    english,
    portuguese,
    coming_soon,
    income_total,
    month_prev,
    month_next,
    current_month,
    all_categories,
    search_hint,
    by_category,
    by_person,
    empty_month,
    empty_month_hint,
    no_matches,
    clear_filters,
    net_label,
    after_refunds,
    this_year,
    year_prev,
    year_next,
    empty_year,
    settings_profile,
    settings_people,
    settings_categories,
    settings_data,
    settings_about,
    settings_tab_general,
    settings_tab_people,
    settings_tab_categories,
    settings_tab_data,
    person_owner,
    person_other,
    archived_tag,
    archive,
    add,
    people_add,
    categories_add,
    scope_both,
    data_path,
    open_data_folder,
    archive_person_body,
    archive_category_body,
    settings_save_failed,
    settings_scope_in_use,
    v_color_invalid,
    color_label,
    csv_date,
    csv_title,
    csv_type,
    csv_amount,
    csv_category,
    csv_payer,
    csv_people,
    csv_allocations,
    csv_description,
    export_json,
    export_csv_month,
    export_csv_all,
    export_hint,
    import_ellipsis,
    import_preview_title,
    import_preview_confirm,
    import_preview_meta,
    import_preview_format_v1,
    import_preview_format_v2,
    import_preview_timestamp_fallback,
    import_preview_counts,
    import_preview_consequence,
    import_preview_snapshot,
    import_preview_widened,
    import_errors_more,
    toast_exported,
    toast_export_failed,
    backups_title,
    backups_hint,
    backups_empty,
    back_up_now,
    backup_delete,
    restore_ellipsis,
    restore_from_file,
    backup_kind_auto,
    backup_kind_manual,
    backup_kind_pre_purge,
    backup_kind_pre_restore,
    backup_kind_pre_import,
    backup_kind_pre_migration,
    notice_backup_created,
    notice_restored,
    notice_imported,
    notice_trash_purged,
    confirm_restore_title,
    confirm_restore_body,
    confirm_delete_backup_title,
    confirm_delete_backup_body,
    unknown_date,
    err_restore_not_athena,
    err_restore_too_new,
    err_restore_corrupt,
    err_restore_page_size,
    err_restore_invalid,
    err_restore_snapshot_failed,
    err_restore_same_as_live,
    err_restore_verification_failed,
    err_restore_rollback_failed,
    err_restore_failed,
    err_backup_failed,
    err_backup_delete_failed,
    err_import_too_large,
    err_import_io,
    err_import_malformed_json,
    err_import_not_athena,
    err_import_too_new,
    err_import_invalid_version,
    err_import_invalid_exported_at,
    err_import_templates_not_supported,
    err_import_onboarding_incomplete,
    err_import_invalid_theme,
    err_import_invalid_language,
    err_import_non_positive_person_id,
    err_import_duplicate_person_id,
    err_import_blank_person_name,
    err_import_duplicate_person_name,
    err_import_invalid_person_kind,
    err_import_invalid_person_timestamp,
    err_import_owner_count,
    err_import_owner_archived,
    err_import_non_positive_category_id,
    err_import_duplicate_category_id,
    err_import_blank_category_name,
    err_import_duplicate_category_name,
    err_import_invalid_category_color,
    err_import_invalid_category_scope,
    err_import_invalid_category_timestamp,
    err_import_non_positive_transaction_id,
    err_import_duplicate_transaction_id,
    err_import_blank_transaction_title,
    err_import_non_positive_amount,
    err_import_invalid_date,
    err_import_invalid_transaction_kind,
    err_import_missing_category,
    err_import_missing_payer,
    err_import_missing_allocation_person,
    err_import_duplicate_allocation_person,
    err_import_negative_allocation_share,
    err_import_allocation_sum_mismatch,
    err_import_invalid_split_mode,
    err_import_split_mode_without_shared_split,
    err_import_invalid_settlement_shape,
    err_import_invalid_transaction_timestamp,
    err_import_overflow,
    err_import_snapshot_failed,
    err_import_verification_failed,
    startup_failed,
    startup_schema_too_new,
    startup_corrupt,
    startup_already_running,
    startup_generic,
    recovery_title,
    recovery_body,
    recovery_backups_title,
    recovery_no_backups,
    recovery_restore_hint,
    recovery_broken_files,
    recovery_restore_failed,
    recovery_quit,
    recovery_missing,
    recovery_scan_uncertain,
    recovery_live_indeterminate,
    recovery_conflict,
    recovery_start_fresh,
    recovery_start_fresh_confirm,
    recovery_start_fresh_failed,
    confirm_delete_title,
    confirm_delete_body,
    toast_deleted_named,
    trash_retention_hint,
    trash_empty_title,
    trash_empty_hint,
    trash_deleted_on,
    restore,
    delete_forever,
    empty_trash,
    confirm_purge_title,
    confirm_purge_body,
    confirm_empty_trash_title,
    confirm_empty_trash_body,
    onb_welcome_title,
    onb_welcome_body,
    onb_name_title,
    onb_your_name,
    onb_partner_name,
    onb_balance_title,
    onb_balance_body,
    onb_balance_label,
    onb_ready_title,
    onb_ready_body,
    onb_summary_name,
    onb_summary_partner,
    onb_none,
    onb_save_failed,
    onb_step_of,
    qa_edit_title,
    qa_amount,
    qa_title_label,
    qa_title_hint,
    qa_date,
    qa_today,
    qa_type,
    qa_category,
    qa_no_category,
    qa_paid_by,
    qa_from,
    qa_to,
    qa_for,
    qa_split_between,
    qa_even_split,
    qa_more,
    qa_description,
    qa_summary,
    qa_settlement_summary,
    qa_settlement_month_mismatch,
    qa_save_failed,
    v_date_invalid,
    toast_transaction_saved,
    toast_transaction_updated,
    toast_transaction_deleted,
    toast_transaction_restored,
    toast_trash_emptied,
    toast_profile_saved,
    toast_person_saved,
    toast_person_archived,
    toast_person_restored,
    toast_category_saved,
    toast_category_archived,
    toast_backup_created,
    toast_backup_failed,
    toast_backup_deleted,
    toast_restored,
    toast_restored_named,
    toast_imported,
    toast_imported_named,
    toast_welcome,
    toast_undo_failed,
    settlement_title,
    settlement_as_of,
    settlement_all_settled,
    settlement_no_balances,
    settlement_owes_you,
    settlement_you_owe,
    settlement_owes_other,
    settlement_record,
    settlement_restore_to_record,
    settlement_restore_name_taken,
    settlement_archived_suffix,
    settlement_unavailable,
    view_refresh_failed,
    retry,
    filter_missing_payer_until,
    filter_unassigned_until,
    v_name_required,
    v_partner_name_taken,
    v_title_required,
    v_amount_positive,
    v_duplicate_person,
    v_negative_share,
    v_shares_mismatch,
    v_split_needs_two,
    v_settlement_needs_from,
    v_settlement_needs_to,
    v_settlement_same_person,
    v_settlement_no_category,
    v_settlement_no_split,
    m_empty,
    m_invalid,
    m_too_precise,
    m_too_large,
    settlement_from_to,
    clear_filter_chip,
    err_import_non_positive_template_id,
    err_import_duplicate_template_id,
    err_import_blank_template_name,
    err_import_duplicate_template_name,
    err_import_blank_template_title,
    err_import_non_positive_template_amount,
    err_import_invalid_template_kind,
    err_import_invalid_template_responsibility,
    err_import_template_missing_category,
    err_import_template_missing_payer,
    err_import_template_missing_person,
    err_import_invalid_template_shape,
    err_import_invalid_template_timestamp,
    tpl_err_name_empty,
    tpl_err_title_empty,
    tpl_err_amount,
    tpl_err_settlement,
    tpl_err_income_shape,
    tpl_err_none_has_shares,
    tpl_err_person_full,
    tpl_err_shared_two,
    tpl_err_even_shares,
    tpl_err_name_taken,
    tpl_err_person_missing,
    tpl_err_person_archived,
    tpl_err_category_missing,
    tpl_err_category_archived,
    tpl_err_category_incompatible,
    import_preview_format_v3,
    toast_template_saved,
    toast_template_updated,
    toast_template_deleted,
    qa_templates_label,
    qa_templates_choose,
    qa_templates_empty,
    qa_template_needs_repair,
    qa_save_as_template,
    qa_template_name,
    qa_template_name_hint,
    qa_template_save,
    qa_template_applied,
    qa_line_label,
    qa_line_hint,
    qa_line_help,
    qa_line_review,
    qa_line_err_empty,
    qa_line_err_amount,
    qa_line_err_zero,
    qa_line_err_too_large,
    qa_line_err_title,
    qa_line_err_date,
    qa_kind_choice,
    qa_discard_title,
    qa_discard_body,
    qa_keep_editing,
    qa_discard,
    qa_dup_title,
    qa_dup_body_one,
    qa_dup_body_many,
    qa_save_anyway,
    qa_dup_lookup_failed,
    qa_repair_category,
    qa_repair_payer,
    qa_repair_person,
    qa_remove_from_split,
    qa_edit_template_title,
    qa_template_failed,
    settings_templates,
    settings_templates_hint,
    settings_templates_empty,
    settings_template_summary,
    settings_template_last_used,
    settings_template_never_used,
    settings_template_delete_title,
    settings_template_delete_body,
    settings_template_edit_named,
    settings_template_delete_named,
    days,
    days_of_month,
    back_to_year,
    open_month,
    day_cell_label,
    day_activity_none,
    day_activity_one,
    day_activity_many,
    day_detail_title,
    day_detail_scope,
    day_detail_empty,
    day_detail_unavailable,
    day_grid_hint,
    insight_title,
    insight_heading_mtd,
    insight_heading_planned,
    insight_none,
    insight_income_only,
    insight_refunds_only,
    insight_none_neutral,
    insight_income_only_neutral,
    insight_refunds_only_neutral,
    insight_net_credits,
    insight_zero,
    insight_spending,
    insight_pct,
    insight_pct_less_than_one,
    insight_cmp_up,
    insight_cmp_down,
    insight_cmp_unchanged,
    insight_cmp_up_full,
    insight_cmp_down_full,
    insight_cmp_unchanged_full,
    insight_cmp_amounts,
    insight_cmp_amounts_full,
    insight_top_category,
    insight_largest_expense,
    insight_leading_person,
    xlsx_sheet_entries,
    xlsx_sheet_allocations,
    xlsx_scope,
    xlsx_scope_all,
    xlsx_id,
    xlsx_amount_form,
    xlsx_form_number,
    xlsx_form_exact,
    xlsx_transaction_id,
    xlsx_participant,
    export_xlsx_month,
    export_xlsx_all,
    toast_appearance_saved,
    toast_appearance_reset,
    appearance_unreadable_notice,
    media_err_unsupported,
    media_err_corrupt,
    media_err_source_too_large,
    media_err_dimensions,
    media_err_frames,
    media_err_budget,
    media_err_io,
    media_err_missing,
    media_err_quota,
    media_err_unsafe,
    media_err_cancelled,
}

/// Which confirmation to show after an action. Maps one-to-one onto string keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKey {
    TransactionSaved,
    TemplateSaved,
    TemplateUpdated,
    TemplateDeleted,
    AppearanceSaved,
    AppearanceReset,
    TransactionUpdated,
    TransactionDeleted,
    TransactionRestored,
    TrashEmptied,
    ProfileSaved,
    PersonSaved,
    PersonArchived,
    PersonRestored,
    CategorySaved,
    CategoryArchived,
    BackupCreated,
    BackupFailed,
    BackupDeleted,
    Restored,
    Imported,
    Welcome,
    UndoFailed,
}

impl ToastKey {
    #[must_use]
    pub fn text(self, language: Language) -> &'static str {
        let s = strings(language);
        match self {
            ToastKey::TransactionSaved => s.toast_transaction_saved,
            ToastKey::TemplateSaved => s.toast_template_saved,
            ToastKey::TemplateUpdated => s.toast_template_updated,
            ToastKey::TemplateDeleted => s.toast_template_deleted,
            ToastKey::AppearanceSaved => s.toast_appearance_saved,
            ToastKey::AppearanceReset => s.toast_appearance_reset,
            ToastKey::TransactionUpdated => s.toast_transaction_updated,
            ToastKey::TransactionDeleted => s.toast_transaction_deleted,
            ToastKey::TransactionRestored => s.toast_transaction_restored,
            ToastKey::TrashEmptied => s.toast_trash_emptied,
            ToastKey::ProfileSaved => s.toast_profile_saved,
            ToastKey::PersonSaved => s.toast_person_saved,
            ToastKey::PersonArchived => s.toast_person_archived,
            ToastKey::PersonRestored => s.toast_person_restored,
            ToastKey::CategorySaved => s.toast_category_saved,
            ToastKey::CategoryArchived => s.toast_category_archived,
            ToastKey::BackupCreated => s.toast_backup_created,
            ToastKey::BackupFailed => s.toast_backup_failed,
            ToastKey::BackupDeleted => s.toast_backup_deleted,
            ToastKey::Restored => s.toast_restored,
            ToastKey::Imported => s.toast_imported,
            ToastKey::Welcome => s.toast_welcome,
            ToastKey::UndoFailed => s.toast_undo_failed,
        }
    }
}

#[must_use]
pub fn strings(language: Language) -> &'static Strings {
    match language {
        Language::En => &en::STRINGS,
        Language::PtBr => &pt_br::STRINGS,
    }
}

const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_PT: [&str; 12] = [
    "Janeiro",
    "Fevereiro",
    "Março",
    "Abril",
    "Maio",
    "Junho",
    "Julho",
    "Agosto",
    "Setembro",
    "Outubro",
    "Novembro",
    "Dezembro",
];

/// Full month name for `1..=12`; an out-of-range number yields an empty string.
#[must_use]
pub fn month_name(language: Language, month: u32) -> &'static str {
    let names = match language {
        Language::En => &MONTHS_EN,
        Language::PtBr => &MONTHS_PT,
    };
    usize::try_from(month)
        .ok()
        .and_then(|m| m.checked_sub(1))
        .and_then(|i| names.get(i))
        .copied()
        .unwrap_or("")
}

/// `Nov` / `Fev`.
#[must_use]
pub fn month_short(language: Language, month: u32) -> String {
    month_name(language, month).chars().take(3).collect()
}

/// `November 2026` / `Novembro 2026`.
#[must_use]
pub fn month_label(month: YearMonth, language: Language) -> String {
    format!("{} {}", month_name(language, month.month), month.year)
}

/// `30 Nov` / `30/11`.
#[must_use]
pub fn format_as_of_date(date: NaiveDate, language: Language) -> String {
    match language {
        Language::En => format!("{} {}", date.day(), month_short(language, date.month())),
        Language::PtBr => date.format("%d/%m").to_string(),
    }
}

/// `Nov 2026`.
#[must_use]
pub fn format_short_month_year(date: NaiveDate, language: Language) -> String {
    format!("{} {}", month_short(language, date.month()), date.year())
}

#[must_use]
pub fn weekday_short(language: Language, weekday: Weekday) -> &'static str {
    match (language, weekday) {
        (Language::En, Weekday::Mon) => "Mon",
        (Language::En, Weekday::Tue) => "Tue",
        (Language::En, Weekday::Wed) => "Wed",
        (Language::En, Weekday::Thu) => "Thu",
        (Language::En, Weekday::Fri) => "Fri",
        (Language::En, Weekday::Sat) => "Sat",
        (Language::En, Weekday::Sun) => "Sun",
        (Language::PtBr, Weekday::Mon) => "Seg",
        (Language::PtBr, Weekday::Tue) => "Ter",
        (Language::PtBr, Weekday::Wed) => "Qua",
        (Language::PtBr, Weekday::Thu) => "Qui",
        (Language::PtBr, Weekday::Fri) => "Sex",
        (Language::PtBr, Weekday::Sat) => "Sáb",
        (Language::PtBr, Weekday::Sun) => "Dom",
    }
}

/// Localized label for a transaction kind. The embedded kind icon (`Icon::for_kind`) accompanies
/// it wherever there is room, so meaning never rests on color alone.
#[must_use]
pub fn kind_label(kind: TransactionType, language: Language) -> &'static str {
    let s = strings(language);
    match kind {
        TransactionType::Expense => s.expense,
        TransactionType::Income => s.income,
        TransactionType::Refund => s.refund,
        TransactionType::Settlement => s.settlement,
    }
}

/// Parse a date typed by the user. The language's own format is expected, but ISO `2026-11-11`
/// and `11/11/2026` are both accepted so pasted dates work in either language.
#[must_use]
pub fn parse_date(text: &str, language: Language) -> Option<NaiveDate> {
    let text = text.trim();
    let formats: [&str; 2] = match language {
        Language::En => ["%Y-%m-%d", "%d/%m/%Y"],
        Language::PtBr => ["%d/%m/%Y", "%Y-%m-%d"],
    };
    formats
        .iter()
        .find_map(|f| NaiveDate::parse_from_str(text, f).ok())
}

/// An example date in the language's format, for hints and error messages.
#[must_use]
pub fn date_example(language: Language) -> String {
    format_date(
        NaiveDate::from_ymd_opt(2026, 11, 30).unwrap_or(NaiveDate::MIN),
        language,
    )
}

/// `2026-11-11` in English, `11/11/2026` in Portuguese. No screen formats dates itself.
#[must_use]
pub fn format_date(date: NaiveDate, language: Language) -> String {
    match language {
        Language::En => date.format("%Y-%m-%d").to_string(),
        Language::PtBr => date.format("%d/%m/%Y").to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluralKey {
    /// `3 entries have no payer`.
    EntriesWithoutPayer,
    /// `3 entries`.
    Entries,
    /// `2 entries without payer`.
    SettlementUnknownPayer,
    /// `5 with no one responsible`.
    SettlementUnassigned,
}

/// Count-aware copy: both languages distinguish exactly one from every other number.
#[must_use]
pub fn plural(key: PluralKey, n: usize, language: Language) -> String {
    let one = n == 1;
    let text = match (key, language, one) {
        (PluralKey::EntriesWithoutPayer, Language::En, true) => "entry has no payer",
        (PluralKey::EntriesWithoutPayer, Language::En, false) => "entries have no payer",
        (
            PluralKey::EntriesWithoutPayer | PluralKey::SettlementUnknownPayer,
            Language::PtBr,
            true,
        ) => "lançamento sem pagador",
        (
            PluralKey::EntriesWithoutPayer | PluralKey::SettlementUnknownPayer,
            Language::PtBr,
            false,
        ) => "lançamentos sem pagador",
        (PluralKey::Entries, Language::En, true) => "entry",
        (PluralKey::Entries, Language::En, false) => "entries",
        (PluralKey::Entries, Language::PtBr, true) => "lançamento",
        (PluralKey::Entries, Language::PtBr, false) => "lançamentos",
        (PluralKey::SettlementUnknownPayer, Language::En, true) => "entry without payer",
        (PluralKey::SettlementUnknownPayer, Language::En, false) => "entries without payer",
        (PluralKey::SettlementUnassigned, Language::En, true) => "entry with no one responsible",
        (PluralKey::SettlementUnassigned, Language::En, false) => "with no one responsible",
        (PluralKey::SettlementUnassigned, Language::PtBr, true) => "lançamento sem responsável",
        (PluralKey::SettlementUnassigned, Language::PtBr, false) => "sem responsável",
    };
    format!("{n} {text}")
}

/// Localized message for one validation failure.
#[must_use]
pub fn validation_message(error: &ValidationError, language: Language) -> String {
    let s = strings(language);
    match error {
        ValidationError::TitleEmpty => s.v_title_required.to_string(),
        ValidationError::AmountNotPositive => s.v_amount_positive.to_string(),
        ValidationError::DuplicatePerson(_) => s.v_duplicate_person.to_string(),
        ValidationError::NegativeShare(_) => s.v_negative_share.to_string(),
        ValidationError::ShareSumMismatch { expected, actual } => s
            .v_shares_mismatch
            .replace("{actual}", &actual.format_brl())
            .replace("{expected}", &expected.format_brl()),
        ValidationError::SplitModeWithoutSharedSplit => s.v_split_needs_two.to_string(),
        ValidationError::SettlementNeedsFrom => s.v_settlement_needs_from.to_string(),
        ValidationError::SettlementNeedsOneRecipient => s.v_settlement_needs_to.to_string(),
        ValidationError::SettlementSamePerson => s.v_settlement_same_person.to_string(),
        ValidationError::SettlementHasCategory => s.v_settlement_no_category.to_string(),
        ValidationError::SettlementHasSplitMode => s.v_settlement_no_split.to_string(),
    }
}

/// The text for a fatal startup failure, shown in a message box because release builds have no
/// console. The profile may be unreadable at that point, so the caller passes the system language.
#[must_use]
pub fn startup_error_message(error: &AppError, language: Language) -> String {
    let s = strings(language);
    let specific = match error {
        AppError::AlreadyRunning => return s.startup_already_running.to_string(),
        AppError::SchemaTooNew { found, supported } => s
            .startup_schema_too_new
            .replace("{found}", &found.to_string())
            .replace("{supported}", &supported.to_string()),
        AppError::Corrupt { path } => s.startup_corrupt.replace("{path}", path),
        other => s.startup_generic.replace("{detail}", &other.to_string()),
    };
    format!("{}\n\n{specific}", s.startup_failed)
}

/// Localized message for a wallpaper media failure.
#[must_use]
pub fn media_error_message(error: MediaError, language: Language) -> &'static str {
    let s = strings(language);
    match error {
        MediaError::Unsupported => s.media_err_unsupported,
        MediaError::Corrupt => s.media_err_corrupt,
        MediaError::SourceTooLarge => s.media_err_source_too_large,
        MediaError::DimensionsTooLarge => s.media_err_dimensions,
        MediaError::TooManyFrames => s.media_err_frames,
        MediaError::OverBudget => s.media_err_budget,
        MediaError::Io => s.media_err_io,
        MediaError::Missing => s.media_err_missing,
        MediaError::QuotaExceeded => s.media_err_quota,
        MediaError::Unsafe => s.media_err_unsafe,
        MediaError::Cancelled => s.media_err_cancelled,
    }
}

/// Localized message for a template validation failure.
#[must_use]
pub fn template_error_message(error: &TemplateError, language: Language) -> String {
    let s = strings(language);
    match error {
        TemplateError::NameEmpty => s.tpl_err_name_empty,
        TemplateError::TitleEmpty => s.tpl_err_title_empty,
        TemplateError::AmountNotPositive => s.tpl_err_amount,
        TemplateError::SettlementNotAllowed => s.tpl_err_settlement,
        TemplateError::IncomeShape => s.tpl_err_income_shape,
        TemplateError::NoneHasShares => s.tpl_err_none_has_shares,
        TemplateError::PersonNeedsOneFullShare => s.tpl_err_person_full,
        TemplateError::SharedNeedsTwoPeople => s.tpl_err_shared_two,
        TemplateError::EvenSharesNotCanonical => s.tpl_err_even_shares,
        TemplateError::NameTaken => s.tpl_err_name_taken,
        TemplateError::PersonMissing(_) => s.tpl_err_person_missing,
        TemplateError::PersonArchived(_) => s.tpl_err_person_archived,
        TemplateError::CategoryMissing => s.tpl_err_category_missing,
        TemplateError::CategoryArchived => s.tpl_err_category_archived,
        TemplateError::CategoryIncompatible => s.tpl_err_category_incompatible,
        TemplateError::Shares(inner) => return validation_message(inner, language),
    }
    .to_string()
}

/// Localized message for a money-input failure.
#[must_use]
pub fn money_error_message(error: MoneyError, language: Language) -> &'static str {
    let s = strings(language);
    match error {
        MoneyError::Empty => s.m_empty,
        MoneyError::Invalid => s.m_invalid,
        MoneyError::TooPrecise => s.m_too_precise,
        MoneyError::TooLarge => s.m_too_large,
    }
}

/// Localized label for a [`BackupKind`].
#[must_use]
pub fn backup_kind_label(kind: BackupKind, language: Language) -> &'static str {
    let s = strings(language);
    match kind {
        BackupKind::Auto => s.backup_kind_auto,
        BackupKind::Manual => s.backup_kind_manual,
        BackupKind::PrePurge => s.backup_kind_pre_purge,
        BackupKind::PreRestore => s.backup_kind_pre_restore,
        BackupKind::PreImport => s.backup_kind_pre_import,
        BackupKind::PreMigration => s.backup_kind_pre_migration,
    }
}

/// Localized message for every [`RestoreError`] variant.
#[must_use]
pub fn restore_error_message(error: &RestoreError, language: Language) -> String {
    let s = strings(language);
    match error {
        RestoreError::NotAthena => s.err_restore_not_athena.to_string(),
        RestoreError::TooNew { found, supported } => s
            .err_restore_too_new
            .replace("{found}", &found.to_string())
            .replace("{supported}", &supported.to_string()),
        RestoreError::Corrupt => s.err_restore_corrupt.to_string(),
        RestoreError::PageSize => s.err_restore_page_size.to_string(),
        RestoreError::Invalid(issues) => s
            .err_restore_invalid
            .replace("{count}", &issues.len().to_string()),
        RestoreError::SnapshotFailed(detail) => {
            s.err_restore_snapshot_failed.replace("{detail}", detail)
        }
        RestoreError::SameAsLive => s.err_restore_same_as_live.to_string(),
        RestoreError::VerificationFailed {
            source_path,
            snapshot_path,
        } => s
            .err_restore_verification_failed
            .replace("{source}", source_path)
            .replace("{snapshot}", snapshot_path),
        RestoreError::RollbackFailed {
            snapshot_path,
            detail,
        } => s
            .err_restore_rollback_failed
            .replace("{snapshot}", snapshot_path)
            .replace("{detail}", detail),
        RestoreError::Failed(detail) => s.err_restore_failed.replace("{detail}", detail),
    }
}

/// Localized message for a failed manual backup.
#[must_use]
pub fn backup_error_message(error: &AppError, language: Language) -> String {
    strings(language)
        .err_backup_failed
        .replace("{detail}", &error.to_string())
}

/// Localized message for a failed manual backup deletion.
#[must_use]
pub fn backup_delete_error_message(error: &AppError, language: Language) -> String {
    strings(language)
        .err_backup_delete_failed
        .replace("{detail}", &error.to_string())
}

/// Localized message for every [`ImportError`] variant.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn import_error_message(error: &ImportError, language: Language) -> String {
    let s = strings(language);
    match error {
        ImportError::TooLarge => s.err_import_too_large.to_string(),
        ImportError::Io(detail) => s.err_import_io.replace("{detail}", detail),
        ImportError::MalformedJson(detail) => {
            s.err_import_malformed_json.replace("{detail}", detail)
        }
        ImportError::NotAthena => s.err_import_not_athena.to_string(),
        ImportError::TooNew { found, supported } => s
            .err_import_too_new
            .replace("{found}", &found.to_string())
            .replace("{supported}", &supported.to_string()),
        ImportError::InvalidVersion => s.err_import_invalid_version.to_string(),
        ImportError::InvalidExportedAt => s.err_import_invalid_exported_at.to_string(),
        ImportError::TemplatesNotSupported => s.err_import_templates_not_supported.to_string(),
        ImportError::OnboardingIncomplete => s.err_import_onboarding_incomplete.to_string(),
        ImportError::InvalidTheme => s.err_import_invalid_theme.to_string(),
        ImportError::InvalidLanguage => s.err_import_invalid_language.to_string(),
        ImportError::NonPositivePersonId { index, id } => s
            .err_import_non_positive_person_id
            .replace("{index}", &index.to_string())
            .replace("{id}", &id.to_string()),
        ImportError::DuplicatePersonId { id } => s
            .err_import_duplicate_person_id
            .replace("{id}", &id.to_string()),
        ImportError::BlankPersonName { id } => s
            .err_import_blank_person_name
            .replace("{id}", &id.to_string()),
        ImportError::DuplicatePersonName { name } => {
            s.err_import_duplicate_person_name.replace("{name}", name)
        }
        ImportError::InvalidPersonKind { id } => s
            .err_import_invalid_person_kind
            .replace("{id}", &id.to_string()),
        ImportError::InvalidPersonTimestamp { id } => s
            .err_import_invalid_person_timestamp
            .replace("{id}", &id.to_string()),
        ImportError::OwnerCount { count } => s
            .err_import_owner_count
            .replace("{count}", &count.to_string()),
        ImportError::OwnerArchived => s.err_import_owner_archived.to_string(),
        ImportError::NonPositiveCategoryId { index, id } => s
            .err_import_non_positive_category_id
            .replace("{index}", &index.to_string())
            .replace("{id}", &id.to_string()),
        ImportError::DuplicateCategoryId { id } => s
            .err_import_duplicate_category_id
            .replace("{id}", &id.to_string()),
        ImportError::BlankCategoryName { id } => s
            .err_import_blank_category_name
            .replace("{id}", &id.to_string()),
        ImportError::DuplicateCategoryName { name } => {
            s.err_import_duplicate_category_name.replace("{name}", name)
        }
        ImportError::InvalidCategoryColor { id } => s
            .err_import_invalid_category_color
            .replace("{id}", &id.to_string()),
        ImportError::InvalidCategoryScope { id } => s
            .err_import_invalid_category_scope
            .replace("{id}", &id.to_string()),
        ImportError::InvalidCategoryTimestamp { id } => s
            .err_import_invalid_category_timestamp
            .replace("{id}", &id.to_string()),
        ImportError::NonPositiveTransactionId { index, id } => s
            .err_import_non_positive_transaction_id
            .replace("{index}", &index.to_string())
            .replace("{id}", &id.to_string()),
        ImportError::DuplicateTransactionId { id } => s
            .err_import_duplicate_transaction_id
            .replace("{id}", &id.to_string()),
        ImportError::BlankTransactionTitle { id } => s
            .err_import_blank_transaction_title
            .replace("{id}", &id.to_string()),
        ImportError::NonPositiveAmount { id } => s
            .err_import_non_positive_amount
            .replace("{id}", &id.to_string()),
        ImportError::InvalidDate { id } => {
            s.err_import_invalid_date.replace("{id}", &id.to_string())
        }
        ImportError::InvalidTransactionKind { id } => s
            .err_import_invalid_transaction_kind
            .replace("{id}", &id.to_string()),
        ImportError::MissingCategory { id, category_id } => s
            .err_import_missing_category
            .replace("{id}", &id.to_string())
            .replace("{category_id}", &category_id.to_string()),
        ImportError::MissingPayer { id, payer_id } => s
            .err_import_missing_payer
            .replace("{id}", &id.to_string())
            .replace("{payer_id}", &payer_id.to_string()),
        ImportError::MissingAllocationPerson { id, person_id } => s
            .err_import_missing_allocation_person
            .replace("{id}", &id.to_string())
            .replace("{person_id}", &person_id.to_string()),
        ImportError::DuplicateAllocationPerson { id, person_id } => s
            .err_import_duplicate_allocation_person
            .replace("{id}", &id.to_string())
            .replace("{person_id}", &person_id.to_string()),
        ImportError::NegativeAllocationShare { id } => s
            .err_import_negative_allocation_share
            .replace("{id}", &id.to_string()),
        ImportError::AllocationSumMismatch { id } => s
            .err_import_allocation_sum_mismatch
            .replace("{id}", &id.to_string()),
        ImportError::InvalidSplitMode { id } => s
            .err_import_invalid_split_mode
            .replace("{id}", &id.to_string()),
        ImportError::SplitModeWithoutSharedSplit { id } => s
            .err_import_split_mode_without_shared_split
            .replace("{id}", &id.to_string()),
        ImportError::InvalidSettlementShape { id } => s
            .err_import_invalid_settlement_shape
            .replace("{id}", &id.to_string()),
        ImportError::InvalidTransactionTimestamp { id } => s
            .err_import_invalid_transaction_timestamp
            .replace("{id}", &id.to_string()),
        ImportError::NonPositiveTemplateId { index, id } => s
            .err_import_non_positive_template_id
            .replace("{index}", &index.to_string())
            .replace("{id}", &id.to_string()),
        ImportError::DuplicateTemplateId { id } => s
            .err_import_duplicate_template_id
            .replace("{id}", &id.to_string()),
        ImportError::BlankTemplateName { id } => s
            .err_import_blank_template_name
            .replace("{id}", &id.to_string()),
        ImportError::DuplicateTemplateName { name } => {
            s.err_import_duplicate_template_name.replace("{name}", name)
        }
        ImportError::BlankTemplateTitle { id } => s
            .err_import_blank_template_title
            .replace("{id}", &id.to_string()),
        ImportError::NonPositiveTemplateAmount { id } => s
            .err_import_non_positive_template_amount
            .replace("{id}", &id.to_string()),
        ImportError::InvalidTemplateKind { id } => s
            .err_import_invalid_template_kind
            .replace("{id}", &id.to_string()),
        ImportError::InvalidTemplateResponsibility { id } => s
            .err_import_invalid_template_responsibility
            .replace("{id}", &id.to_string()),
        ImportError::TemplateMissingCategory { id, category_id } => s
            .err_import_template_missing_category
            .replace("{id}", &id.to_string())
            .replace("{category_id}", &category_id.to_string()),
        ImportError::TemplateMissingPayer { id, payer_id } => s
            .err_import_template_missing_payer
            .replace("{id}", &id.to_string())
            .replace("{payer_id}", &payer_id.to_string()),
        ImportError::TemplateMissingPerson { id, person_id } => s
            .err_import_template_missing_person
            .replace("{id}", &id.to_string())
            .replace("{person_id}", &person_id.to_string()),
        ImportError::InvalidTemplateShape { id } => s
            .err_import_invalid_template_shape
            .replace("{id}", &id.to_string()),
        ImportError::InvalidTemplateTimestamp { id } => s
            .err_import_invalid_template_timestamp
            .replace("{id}", &id.to_string()),
        ImportError::Overflow => s.err_import_overflow.to_string(),
        ImportError::SnapshotFailed(detail) => {
            s.err_import_snapshot_failed.replace("{detail}", detail)
        }
        ImportError::VerificationFailed(detail) => {
            s.err_import_verification_failed.replace("{detail}", detail)
        }
    }
}

/// Render up to 5 localized import errors as a multi-line summary, collapsing extra lines with
/// `import_errors_more`.
#[must_use]
pub fn format_import_errors(errors: &[ImportError], language: Language) -> String {
    const LIMIT: usize = 5;
    if errors.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = errors
        .iter()
        .take(LIMIT)
        .map(|err| import_error_message(err, language))
        .collect();
    let joined = lines.join("\n");
    if errors.len() > LIMIT {
        strings(language)
            .import_errors_more
            .replace("{first}", &joined)
            .replace("{more}", &(errors.len() - LIMIT).to_string())
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::infrastructure::db::integrity::IntegrityIssue;

    #[test]
    fn dictionaries_cover_core_labels() {
        assert_eq!(strings(Language::En).month, "Month");
        assert_eq!(strings(Language::PtBr).month, "Mês");
        assert_eq!(month_name(Language::En, 11), "November");
        assert_eq!(month_name(Language::PtBr, 11), "Novembro");
        assert_eq!(
            month_label(YearMonth::new(2026, 11).unwrap(), Language::PtBr),
            "Novembro 2026"
        );
        assert_eq!(
            month_label(YearMonth::new(2026, 3).unwrap(), Language::En),
            "March 2026"
        );
    }

    #[test]
    fn month_names_handle_every_month_and_out_of_range() {
        for m in 1..=12 {
            assert_ne!(month_name(Language::En, m), "");
            assert_ne!(month_name(Language::PtBr, m), "");
        }
        assert_eq!(month_name(Language::En, 0), "");
        assert_eq!(month_name(Language::En, 13), "");
    }

    #[test]
    fn every_key_is_filled_in_both_languages() {
        for language in [Language::En, Language::PtBr] {
            for (key, text) in strings(language).fields() {
                assert!(!text.trim().is_empty(), "{key} is empty in {language:?}");
            }
        }
    }

    #[test]
    fn translations_differ_where_they_should_and_keep_the_same_placeholders() {
        let en = strings(Language::En).fields();
        let pt = strings(Language::PtBr).fields();
        assert_eq!(en.len(), pt.len());
        let tokens = |text: &str| -> Vec<String> {
            let mut found: Vec<String> = text
                .split('{')
                .skip(1)
                .filter_map(|rest| rest.split('}').next().map(str::to_string))
                .collect();
            found.sort();
            found
        };
        for ((key, e), (_, p)) in en.iter().zip(pt.iter()) {
            assert_eq!(tokens(e), tokens(p), "placeholders differ for {key}");
        }
        assert_ne!(strings(Language::En).income, strings(Language::PtBr).income);
    }

    #[test]
    fn every_toast_is_localized() {
        use ToastKey::*;
        for key in [
            TransactionSaved,
            TransactionUpdated,
            TransactionDeleted,
            TransactionRestored,
            TrashEmptied,
            ProfileSaved,
            PersonSaved,
            PersonArchived,
            PersonRestored,
            CategorySaved,
            CategoryArchived,
            BackupCreated,
            BackupFailed,
            BackupDeleted,
            Restored,
            Imported,
            Welcome,
            UndoFailed,
        ] {
            assert_ne!(key.text(Language::En), key.text(Language::PtBr));
        }
    }

    #[test]
    fn restore_errors_are_localized() {
        let cases = [
            RestoreError::NotAthena,
            RestoreError::TooNew {
                found: 9,
                supported: 3,
            },
            RestoreError::Corrupt,
            RestoreError::PageSize,
            RestoreError::Invalid(vec![IntegrityIssue::AllocationSumMismatch { tx_id: 1 }]),
            RestoreError::SnapshotFailed("disk full".into()),
            RestoreError::SameAsLive,
            RestoreError::VerificationFailed {
                source_path: "src.db".into(),
                snapshot_path: "pre.db".into(),
            },
            RestoreError::RollbackFailed {
                snapshot_path: "pre.db".into(),
                detail: "io".into(),
            },
            RestoreError::Failed("err".into()),
        ];
        for err in &cases {
            let en = restore_error_message(err, Language::En);
            let pt = restore_error_message(err, Language::PtBr);
            assert!(!en.trim().is_empty() && !pt.trim().is_empty());
            assert_ne!(en, pt, "{err:?}");
            assert!(
                !en.contains('{') && !pt.contains('{'),
                "{err:?}: {en} / {pt}"
            );
        }
    }

    #[test]
    fn backup_errors_are_localized() {
        let failures = [
            AppError::Backup("verification failed".into()),
            AppError::Io(std::io::Error::other("disk full")),
        ];
        for err in &failures {
            let save_en = backup_error_message(err, Language::En);
            let save_pt = backup_error_message(err, Language::PtBr);
            let delete_en = backup_delete_error_message(err, Language::En);
            let delete_pt = backup_delete_error_message(err, Language::PtBr);
            for text in [&save_en, &save_pt, &delete_en, &delete_pt] {
                assert!(!text.trim().is_empty());
                assert!(!text.contains('{'), "{text}");
            }
            assert_ne!(save_en, save_pt);
            assert_ne!(delete_en, delete_pt);
            assert_ne!(save_en, delete_en);
        }
    }

    #[test]
    fn import_errors_are_localized() {
        let cases = [
            ImportError::TooLarge,
            ImportError::Io("read".into()),
            ImportError::MalformedJson("eof".into()),
            ImportError::NotAthena,
            ImportError::TooNew {
                found: 9,
                supported: 2,
            },
            ImportError::InvalidVersion,
            ImportError::InvalidExportedAt,
            ImportError::TemplatesNotSupported,
            ImportError::OnboardingIncomplete,
            ImportError::InvalidTheme,
            ImportError::InvalidLanguage,
            ImportError::NonPositivePersonId { index: 0, id: 0 },
            ImportError::DuplicatePersonId { id: 1 },
            ImportError::BlankPersonName { id: 1 },
            ImportError::DuplicatePersonName { name: "Ana".into() },
            ImportError::InvalidPersonKind { id: 1 },
            ImportError::InvalidPersonTimestamp { id: 1 },
            ImportError::OwnerCount { count: 2 },
            ImportError::OwnerArchived,
            ImportError::NonPositiveCategoryId { index: 0, id: -1 },
            ImportError::DuplicateCategoryId { id: 2 },
            ImportError::BlankCategoryName { id: 2 },
            ImportError::DuplicateCategoryName {
                name: "Food".into(),
            },
            ImportError::InvalidCategoryColor { id: 2 },
            ImportError::InvalidCategoryScope { id: 2 },
            ImportError::InvalidCategoryTimestamp { id: 2 },
            ImportError::NonPositiveTransactionId { index: 0, id: 0 },
            ImportError::DuplicateTransactionId { id: 3 },
            ImportError::BlankTransactionTitle { id: 3 },
            ImportError::NonPositiveAmount { id: 3 },
            ImportError::InvalidDate { id: 3 },
            ImportError::InvalidTransactionKind { id: 3 },
            ImportError::MissingCategory {
                id: 3,
                category_id: 9,
            },
            ImportError::MissingPayer { id: 3, payer_id: 9 },
            ImportError::MissingAllocationPerson {
                id: 3,
                person_id: 9,
            },
            ImportError::DuplicateAllocationPerson {
                id: 3,
                person_id: 1,
            },
            ImportError::NegativeAllocationShare { id: 3 },
            ImportError::AllocationSumMismatch { id: 3 },
            ImportError::InvalidSplitMode { id: 3 },
            ImportError::SplitModeWithoutSharedSplit { id: 3 },
            ImportError::InvalidSettlementShape { id: 3 },
            ImportError::InvalidTransactionTimestamp { id: 3 },
            ImportError::Overflow,
            ImportError::SnapshotFailed("disk".into()),
            ImportError::VerificationFailed("fk".into()),
        ];
        assert_eq!(cases.len(), 45);
        for err in &cases {
            let en = import_error_message(err, Language::En);
            let pt = import_error_message(err, Language::PtBr);
            assert!(!en.trim().is_empty() && !pt.trim().is_empty());
            assert_ne!(en, pt, "{err:?}");
            assert!(
                !en.contains('{') && !pt.contains('{'),
                "{err:?}: {en} / {pt}"
            );
        }
    }

    #[test]
    fn dates_and_weekdays_are_localized() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 11).unwrap();
        assert_eq!(format_date(d, Language::En), "2026-11-11");
        assert_eq!(format_date(d, Language::PtBr), "11/11/2026");
        let end_nov = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        assert_eq!(format_as_of_date(end_nov, Language::En), "30 Nov");
        assert_eq!(format_as_of_date(end_nov, Language::PtBr), "30/11");
        assert_eq!(format_short_month_year(end_nov, Language::En), "Nov 2026");
        assert_eq!(weekday_short(Language::PtBr, Weekday::Sat), "Sáb");
        assert_eq!(weekday_short(Language::En, Weekday::Mon), "Mon");
    }

    #[test]
    fn dates_parse_in_the_expected_and_the_other_format() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        assert_eq!(parse_date("5/11/2026", Language::PtBr), Some(d));
        assert_eq!(parse_date("05/11/2026", Language::En), Some(d));
        assert_eq!(parse_date(" 2026-11-05 ", Language::PtBr), Some(d));
        assert_eq!(parse_date("2026-13-01", Language::En), None);
        assert_eq!(parse_date("", Language::En), None);
        assert_eq!(date_example(Language::PtBr), "30/11/2026");
        assert_eq!(
            parse_date(&format_date(d, Language::PtBr), Language::PtBr),
            Some(d)
        );
    }

    #[test]
    fn kinds_carry_a_plain_localized_label() {
        assert_eq!(
            kind_label(TransactionType::Expense, Language::En),
            "Expense"
        );
        assert_eq!(
            kind_label(TransactionType::Income, Language::PtBr),
            "Receita"
        );
        assert_eq!(kind_label(TransactionType::Refund, Language::En), "Refund");
    }

    #[test]
    fn plurals_distinguish_one_from_many_in_both_languages() {
        for language in [Language::En, Language::PtBr] {
            for key in [
                PluralKey::EntriesWithoutPayer,
                PluralKey::Entries,
                PluralKey::SettlementUnknownPayer,
                PluralKey::SettlementUnassigned,
            ] {
                assert_ne!(
                    plural(key, 1, language),
                    plural(key, 3, language).replace('3', "1")
                );
                assert!(plural(key, 1, language).starts_with("1 "));
                assert!(plural(key, 0, language).starts_with("0 "));
            }
        }
        assert_eq!(plural(PluralKey::Entries, 3, Language::En), "3 entries");
        assert_eq!(
            plural(PluralKey::EntriesWithoutPayer, 1, Language::PtBr),
            "1 lançamento sem pagador"
        );
    }

    #[test]
    fn validation_and_money_messages_are_localized() {
        let mismatch = ValidationError::ShareSumMismatch {
            expected: Money::from_cents(10_000),
            actual: Money::from_cents(6_000),
        };
        let en = validation_message(&mismatch, Language::En);
        assert!(en.contains("R$ 60,00") && en.contains("R$ 100,00"), "{en}");
        assert!(!en.contains('{'));
        assert_ne!(en, validation_message(&mismatch, Language::PtBr));
        assert_ne!(
            validation_message(&ValidationError::TitleEmpty, Language::En),
            validation_message(&ValidationError::TitleEmpty, Language::PtBr)
        );
        assert_ne!(
            money_error_message(MoneyError::TooPrecise, Language::En),
            money_error_message(MoneyError::TooPrecise, Language::PtBr)
        );
    }

    #[test]
    fn every_fatal_startup_error_has_a_localized_message() {
        let errors = [
            AppError::AlreadyRunning,
            AppError::SchemaTooNew {
                found: 9,
                supported: 1,
            },
            AppError::Corrupt {
                path: "C:/x/athena.db".into(),
            },
            AppError::Dirs("no home".into()),
            AppError::Migration("step 2".into()),
            AppError::Io(std::io::Error::other("disk full")),
        ];
        for error in &errors {
            let en = startup_error_message(error, Language::En);
            let pt = startup_error_message(error, Language::PtBr);
            assert!(!en.trim().is_empty() && !pt.trim().is_empty());
            assert_ne!(en, pt, "{error:?}");
            assert!(
                !en.contains('{') && !pt.contains('{'),
                "{error:?}: {en} / {pt}"
            );
        }
        let corrupt = startup_error_message(&errors[2], Language::En);
        assert!(corrupt.contains("C:/x/athena.db") && corrupt.contains("backups"));
        let newer = startup_error_message(&errors[1], Language::En);
        assert!(newer.contains('9') && newer.contains("not changed"));
    }
}
