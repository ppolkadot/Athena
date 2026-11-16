//! All user-visible copy, in English and Brazilian Portuguese.
//!
//! Each language is one `const Strings` with a field per key, so a missing translation is a
//! compile error. Services, exporters and screens all read from here; no screen holds a visible
//! string literal.

mod en;
mod pt_br;

use chrono::{NaiveDate, Weekday};

use crate::domain::date::YearMonth;
use crate::domain::money::MoneyError;
use crate::domain::settings::Language;
use crate::domain::transaction::{TransactionType, ValidationError};
use crate::shared::error::AppError;

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
    v_color_invalid,
    color_label,
    csv_date,
    csv_title,
    csv_type,
    csv_amount,
    csv_category,
    csv_people,
    csv_allocations,
    csv_description,
    export_json,
    export_csv_month,
    export_csv_all,
    export_hint,
    toast_exported,
    toast_export_failed,
    startup_failed,
    startup_schema_too_new,
    startup_corrupt,
    startup_already_running,
    startup_generic,
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
    toast_category_saved,
    toast_category_archived,
    toast_welcome,
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
}

/// Which confirmation to show after an action. Maps one-to-one onto string keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKey {
    TransactionSaved,
    TransactionUpdated,
    TransactionDeleted,
    TransactionRestored,
    TrashEmptied,
    ProfileSaved,
    PersonSaved,
    PersonArchived,
    CategorySaved,
    CategoryArchived,
    Welcome,
}

impl ToastKey {
    #[must_use]
    pub fn text(self, language: Language) -> &'static str {
        let s = strings(language);
        match self {
            ToastKey::TransactionSaved => s.toast_transaction_saved,
            ToastKey::TransactionUpdated => s.toast_transaction_updated,
            ToastKey::TransactionDeleted => s.toast_transaction_deleted,
            ToastKey::TransactionRestored => s.toast_transaction_restored,
            ToastKey::TrashEmptied => s.toast_trash_emptied,
            ToastKey::ProfileSaved => s.toast_profile_saved,
            ToastKey::PersonSaved => s.toast_person_saved,
            ToastKey::PersonArchived => s.toast_person_archived,
            ToastKey::CategorySaved => s.toast_category_saved,
            ToastKey::CategoryArchived => s.toast_category_archived,
            ToastKey::Welcome => s.toast_welcome,
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

/// `November 2026` / `Novembro 2026`.
#[must_use]
pub fn month_label(month: YearMonth, language: Language) -> String {
    format!("{} {}", month_name(language, month.month), month.year)
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

/// Sign plus label for a transaction kind, so meaning never rests on color alone:
/// `− Expense`, `+ Income`, `↩ Refund`, `⇄ Settlement`.
#[must_use]
pub fn kind_label(kind: TransactionType, language: Language) -> String {
    let s = strings(language);
    match kind {
        TransactionType::Expense => format!("− {}", s.expense),
        TransactionType::Income => format!("+ {}", s.income),
        TransactionType::Refund => format!("↩ {}", s.refund),
        TransactionType::Settlement => format!("⇄ {}", s.settlement),
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
}

/// Count-aware copy: both languages distinguish exactly one from every other number.
#[must_use]
pub fn plural(key: PluralKey, n: usize, language: Language) -> String {
    let one = n == 1;
    let text = match (key, language, one) {
        (PluralKey::EntriesWithoutPayer, Language::En, true) => "entry has no payer",
        (PluralKey::EntriesWithoutPayer, Language::En, false) => "entries have no payer",
        (PluralKey::EntriesWithoutPayer, Language::PtBr, true) => "lançamento sem pagador",
        (PluralKey::EntriesWithoutPayer, Language::PtBr, false) => "lançamentos sem pagador",
        (PluralKey::Entries, Language::En, true) => "entry",
        (PluralKey::Entries, Language::En, false) => "entries",
        (PluralKey::Entries, Language::PtBr, true) => "lançamento",
        (PluralKey::Entries, Language::PtBr, false) => "lançamentos",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;

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
            CategorySaved,
            CategoryArchived,
            Welcome,
        ] {
            assert_ne!(key.text(Language::En), key.text(Language::PtBr));
        }
    }

    #[test]
    fn dates_and_weekdays_are_localized() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 11).unwrap();
        assert_eq!(format_date(d, Language::En), "2026-11-11");
        assert_eq!(format_date(d, Language::PtBr), "11/11/2026");
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
    fn kinds_carry_a_sign_and_a_label() {
        assert_eq!(
            kind_label(TransactionType::Expense, Language::En),
            "− Expense"
        );
        assert_eq!(
            kind_label(TransactionType::Income, Language::PtBr),
            "+ Receita"
        );
        assert!(kind_label(TransactionType::Refund, Language::En).starts_with('↩'));
    }

    #[test]
    fn plurals_distinguish_one_from_many_in_both_languages() {
        for language in [Language::En, Language::PtBr] {
            for key in [PluralKey::EntriesWithoutPayer, PluralKey::Entries] {
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
