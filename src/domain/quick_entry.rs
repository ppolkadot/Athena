//! One-line quick entry (contract §14.1): `<amount> <title> [@YYYY-MM-DD]`.
//!
//! Pure parsing only. The first Enter (or the Review button) turns a line into a reviewable draft;
//! nothing here saves.

use chrono::NaiveDate;
use thiserror::Error;

use super::money::{Money, MoneyError};

/// A successfully parsed line. `amount` is always positive; `negative` records a leading minus,
/// which asks the user to choose Expense or Refund.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLine {
    pub amount: Money,
    pub negative: bool,
    pub title: String,
    pub date: NaiveDate,
}

/// Why a line cannot become a draft. The line stays editable in every case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum QuickEntryError {
    #[error("the line is empty")]
    Empty,
    #[error("the amount could not be read")]
    Amount(MoneyError),
    #[error("the amount is zero")]
    ZeroAmount,
    #[error("the amount is too large")]
    AmountTooLarge,
    #[error("the title is missing")]
    MissingTitle,
    #[error("the date after @ is not a valid YYYY-MM-DD date")]
    InvalidDate,
}

/// Parse `<amount> <title> [@YYYY-MM-DD]`. The first whitespace-delimited token is the amount
/// (`Money::parse`: `45,90`, `45.90`, `1.234,56`, `-20,00`); the title is the trimmed remainder;
/// only a final standalone token starting with `@` is the date, otherwise `today`.
///
/// # Errors
/// [`QuickEntryError`]; a zero amount or one whose magnitude does not fit is an error.
pub fn parse_line(input: &str, today: NaiveDate) -> Result<ParsedLine, QuickEntryError> {
    let line = input.trim();
    if line.is_empty() {
        return Err(QuickEntryError::Empty);
    }
    let (amount_token, rest) = match line.split_once(char::is_whitespace) {
        Some((first, rest)) => (first, rest.trim()),
        None => (line, ""),
    };
    let signed = Money::parse(amount_token).map_err(|e| match e {
        MoneyError::TooLarge => QuickEntryError::AmountTooLarge,
        other => QuickEntryError::Amount(other),
    })?;
    if signed == Money::ZERO {
        return Err(QuickEntryError::ZeroAmount);
    }
    let negative = signed.is_negative();
    let amount = if negative {
        Money::from_cents(
            signed
                .cents()
                .checked_neg()
                .ok_or(QuickEntryError::AmountTooLarge)?,
        )
    } else {
        signed
    };

    let (title, date) = match rest.rsplit_once(char::is_whitespace) {
        Some((head, last)) if last.starts_with('@') => (head.trim(), Some(last)),
        None if rest.starts_with('@') => ("", Some(rest)),
        _ => (rest, None),
    };
    let date = match date {
        Some(token) => parse_iso_date(&token[1..]).ok_or(QuickEntryError::InvalidDate)?,
        None => today,
    };
    if title.is_empty() {
        return Err(QuickEntryError::MissingTitle);
    }
    Ok(ParsedLine {
        amount,
        negative,
        title: title.to_string(),
        date,
    })
}

fn parse_iso_date(text: &str) -> Option<NaiveDate> {
    let b = text.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b[..4].iter().all(u8::is_ascii_digit)
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || !b[8..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 11).unwrap()
    }

    fn ok(input: &str) -> ParsedLine {
        parse_line(input, today()).unwrap()
    }

    #[test]
    fn comma_and_point_decimals_default_to_today() {
        for line in ["45,90 Café", "45.90 Coffee"] {
            let parsed = ok(line);
            assert_eq!(parsed.amount, Money::from_cents(4_590));
            assert!(!parsed.negative);
            assert_eq!(parsed.date, today());
        }
        assert_eq!(ok("45,90 Café").title, "Café");
        assert_eq!(
            ok("1.234,56 Rent  for   the flat ").title,
            "Rent  for   the flat"
        );
        assert_eq!(ok("1.234,56 Rent").amount, Money::from_cents(123_456));
    }

    #[test]
    fn a_final_at_token_is_the_date_in_every_language() {
        let parsed = ok("45,90 Café @2026-11-05");
        assert_eq!(parsed.date, NaiveDate::from_ymd_opt(2026, 11, 5).unwrap());
        assert_eq!(parsed.title, "Café");
        // Only a final standalone token is reserved; an @ inside the title is text.
        assert_eq!(ok("10 mail me@home").title, "mail me@home");
        assert_eq!(ok("10 pay @bob later").title, "pay @bob later");
        // The date is checked as a real calendar date.
        assert_eq!(
            parse_line("10 x @2026-02-30", today()),
            Err(QuickEntryError::InvalidDate)
        );
        assert_eq!(
            parse_line("10 x @05/11/2026", today()),
            Err(QuickEntryError::InvalidDate)
        );
        assert_eq!(
            parse_line("10 x @", today()),
            Err(QuickEntryError::InvalidDate)
        );
    }

    #[test]
    fn negative_amounts_carry_a_positive_magnitude_and_a_flag() {
        let parsed = ok("-20,00 Estorno");
        assert!(parsed.negative);
        assert_eq!(parsed.amount, Money::from_cents(2_000));
        assert_eq!(parsed.title, "Estorno");
    }

    #[test]
    fn bad_lines_stay_editable_with_a_specific_reason() {
        let t = today();
        assert_eq!(parse_line("", t), Err(QuickEntryError::Empty));
        assert_eq!(parse_line("   ", t), Err(QuickEntryError::Empty));
        assert_eq!(parse_line("45,90", t), Err(QuickEntryError::MissingTitle));
        assert_eq!(
            parse_line("45,90 @2026-11-05", t),
            Err(QuickEntryError::MissingTitle)
        );
        assert_eq!(
            parse_line("Café", t),
            Err(QuickEntryError::Amount(MoneyError::Invalid))
        );
        assert_eq!(parse_line("0 Nothing", t), Err(QuickEntryError::ZeroAmount));
        assert_eq!(
            parse_line("0,00 Nothing", t),
            Err(QuickEntryError::ZeroAmount)
        );
        assert_eq!(
            parse_line("-0,00 Nothing", t),
            Err(QuickEntryError::ZeroAmount)
        );
        assert_eq!(
            parse_line("1,234 Nothing", t),
            Err(QuickEntryError::Amount(MoneyError::TooPrecise))
        );
        assert_eq!(
            parse_line("99999999999999999999 Huge", t),
            Err(QuickEntryError::AmountTooLarge)
        );
    }

    #[test]
    fn the_largest_magnitudes_are_checked_not_wrapped() {
        // i64::MAX cents is representable; one cent more is not.
        let max = parse_line("92233720368547758,07 Max", today()).unwrap();
        assert_eq!(max.amount, Money::from_cents(i64::MAX));
        assert_eq!(
            parse_line("92233720368547758,08 Over", today()),
            Err(QuickEntryError::AmountTooLarge)
        );
        // i64::MIN has no positive magnitude.
        assert_eq!(
            parse_line("-92233720368547758,08 Min", today()),
            Err(QuickEntryError::AmountTooLarge)
        );
        let neg = parse_line("-92233720368547758,07 Max", today()).unwrap();
        assert!(neg.negative);
        assert_eq!(neg.amount, Money::from_cents(i64::MAX));
    }
}
