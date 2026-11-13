//! Exact money in integer cents. No floating point anywhere in this module.

use std::fmt;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};
use std::str::FromStr;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::shared::error::AppError;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Money(i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MoneyError {
    #[error("enter an amount")]
    Empty,
    #[error("that is not a valid amount")]
    Invalid,
    #[error("use at most two decimal places")]
    TooPrecise,
    #[error("that amount is too large")]
    TooLarge,
}

impl From<MoneyError> for AppError {
    fn from(err: MoneyError) -> Self {
        AppError::Money(err.to_string())
    }
}

impl Money {
    pub const ZERO: Money = Money(0);

    #[must_use]
    pub const fn from_cents(cents: i64) -> Self {
        Self(cents)
    }

    #[must_use]
    pub const fn cents(self) -> i64 {
        self.0
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Parse user input: `45`, `45.90`, `45,90`, `1.234,56`, `R$ 1.234,56`, `-R$ 10,00`.
    /// Rejects more than two decimal places and ambiguous separators. Never rounds.
    ///
    /// # Errors
    /// See [`MoneyError`].
    pub fn parse(input: &str) -> Result<Self, MoneyError> {
        let cleaned: String = input
            .trim()
            .replace("R$", "")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if cleaned.is_empty() {
            return Err(MoneyError::Empty);
        }
        let (negative, body) = match cleaned.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, cleaned.strip_prefix('+').unwrap_or(&cleaned)),
        };
        if body.is_empty()
            || !body
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
        {
            return Err(MoneyError::Invalid);
        }
        let (int_digits, frac_digits) = split_decimal(body)?;
        if frac_digits.len() > 2 {
            return Err(MoneyError::TooPrecise);
        }
        let int_part = if int_digits.is_empty() {
            "0"
        } else {
            int_digits.as_str()
        };
        let normalized = format!("{int_part}.{frac_digits:0<2}");
        let mut value = Decimal::from_str(&normalized).map_err(|_| MoneyError::TooLarge)?;
        if negative {
            value = -value;
        }
        let cents = value
            .checked_mul(Decimal::from(100))
            .and_then(|d| d.to_i64())
            .ok_or(MoneyError::TooLarge)?;
        Ok(Self(cents))
    }

    /// `R$ 1.234,56`, `-R$ 10,00`.
    #[must_use]
    pub fn format_brl(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        format!("{sign}R$ {}", self.magnitude(true))
    }

    /// `1234,56`: no prefix, no thousands separator. Negative values keep a leading `-`.
    #[must_use]
    pub fn br_number(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        format!("{sign}{}", self.magnitude(false))
    }

    fn magnitude(self, group: bool) -> String {
        let abs = self.0.unsigned_abs();
        let (whole, frac) = (abs / 100, abs % 100);
        let whole = whole.to_string();
        let whole = if group {
            group_thousands(&whole)
        } else {
            whole
        };
        format!("{whole},{frac:02}")
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format_brl())
    }
}

impl Add for Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money {
        Money(self.0 + rhs.0)
    }
}

impl Sub for Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money {
        Money(self.0 - rhs.0)
    }
}

impl Neg for Money {
    type Output = Money;
    fn neg(self) -> Money {
        Money(-self.0)
    }
}

impl AddAssign for Money {
    fn add_assign(&mut self, rhs: Money) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Money {
    fn sub_assign(&mut self, rhs: Money) {
        self.0 -= rhs.0;
    }
}

impl std::iter::Sum for Money {
    fn sum<I: Iterator<Item = Money>>(iter: I) -> Money {
        iter.fold(Money::ZERO, |acc, m| acc + m)
    }
}

fn group_thousands(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    out
}

/// `1.234` or `12.345.678`: a first group of 1-3 digits then groups of exactly 3.
fn is_thousands_grouped(text: &str) -> bool {
    let mut groups = text.split('.');
    let Some(first) = groups.next() else {
        return false;
    };
    if first.is_empty() || first.len() > 3 || !first.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    groups.all(|g| g.len() == 3 && g.chars().all(|c| c.is_ascii_digit()))
}

/// Returns `(integer digits, fractional digits)` after resolving which separator is which.
fn split_decimal(body: &str) -> Result<(String, String), MoneyError> {
    let commas = body.matches(',').count();
    let dots = body.matches('.').count();
    if commas > 1 {
        return Err(MoneyError::Invalid);
    }
    if commas == 1 {
        // The comma is the decimal separator; dots may only group the integer part.
        let (int_text, frac) = body.split_once(',').ok_or(MoneyError::Invalid)?;
        if frac.is_empty() || frac.contains('.') {
            return Err(MoneyError::Invalid);
        }
        let int_digits = if int_text.contains('.') {
            if !is_thousands_grouped(int_text) {
                return Err(MoneyError::Invalid);
            }
            int_text.replace('.', "")
        } else {
            int_text.to_string()
        };
        return Ok((int_digits, frac.to_string()));
    }
    match dots {
        0 => Ok((body.to_string(), String::new())),
        1 => {
            let (left, right) = body.split_once('.').ok_or(MoneyError::Invalid)?;
            if right.is_empty() {
                return Err(MoneyError::Invalid);
            }
            let left_is_nonzero = left.chars().any(|c| c != '0');
            if right.len() == 3 && left_is_nonzero && left.len() <= 3 {
                Ok((format!("{left}{right}"), String::new()))
            } else {
                Ok((left.to_string(), right.to_string()))
            }
        }
        _ => {
            if is_thousands_grouped(body) {
                Ok((body.replace('.', ""), String::new()))
            } else {
                Err(MoneyError::Invalid)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_brazilian_and_plain_formats() {
        assert_eq!(Money::parse("45.90").unwrap().cents(), 4590);
        assert_eq!(Money::parse("45,90").unwrap().cents(), 4590);
        assert_eq!(Money::parse("1.234,56").unwrap().cents(), 123_456);
        assert_eq!(Money::parse("R$ 1.234,56").unwrap().cents(), 123_456);
        assert_eq!(Money::parse("1234.56").unwrap().cents(), 123_456);
        assert_eq!(Money::parse("1.234").unwrap().cents(), 123_400);
        assert_eq!(Money::parse("45").unwrap().cents(), 4500);
    }

    #[test]
    fn rejects_bad_money_and_excess_precision() {
        assert_eq!(Money::parse(""), Err(MoneyError::Empty));
        assert!(Money::parse("abc").is_err());
        assert_eq!(Money::parse("0,005"), Err(MoneyError::TooPrecise));
        assert_eq!(Money::parse("1.234,567"), Err(MoneyError::TooPrecise));
        // A leading zero makes the single-dot form a decimal, not thousands.
        assert_eq!(Money::parse("0.005"), Err(MoneyError::TooPrecise));
        // Mixed separator conventions are ambiguous and rejected.
        assert!(Money::parse("1,234.56").is_err());
        assert!(Money::parse("--10,00").is_err());
        // Misplaced thousands separators never parse silently.
        assert!(Money::parse("1.2,50").is_err());
        assert!(Money::parse("5,").is_err());
        assert!(Money::parse("5.").is_err());
    }

    #[test]
    fn rejects_overflow() {
        assert_eq!(
            Money::parse("99999999999999999999,00"),
            Err(MoneyError::TooLarge)
        );
    }

    #[test]
    fn accepts_currency_symbol_on_either_side_of_the_sign() {
        assert_eq!(Money::parse("-R$ 10,00").unwrap().cents(), -1000);
        assert_eq!(Money::parse("R$ -10,00").unwrap().cents(), -1000);
        assert_eq!(Money::parse("R$\u{a0}1.234,56").unwrap().cents(), 123_456);
        assert_eq!(Money::parse("1.234.567").unwrap().cents(), 123_456_700);
    }

    #[test]
    fn allows_negative_values() {
        assert_eq!(Money::parse("-10,00").unwrap().cents(), -1000);
        assert_eq!(Money::from_cents(-1000).format_brl(), "-R$ 10,00");
    }

    #[test]
    fn formats_extremes_without_overflow() {
        assert_eq!(
            Money::from_cents(i64::MIN).format_brl(),
            "-R$ 92.233.720.368.547.758,08"
        );
        assert_eq!(
            Money::parse("-92233720368547758,08").unwrap().cents(),
            i64::MIN
        );
    }

    #[test]
    fn formats_brl() {
        assert_eq!(Money::from_cents(123_456).format_brl(), "R$ 1.234,56");
        assert_eq!(Money::from_cents(5).format_brl(), "R$ 0,05");
        assert_eq!(Money::from_cents(0).format_brl(), "R$ 0,00");
        assert_eq!(Money::from_cents(123_456).br_number(), "1234,56");
        assert_eq!(Money::from_cents(-5).br_number(), "-0,05");
    }

    #[test]
    fn arithmetic_and_sum_are_exact() {
        let total: Money = [Money::from_cents(10), Money::from_cents(20)]
            .into_iter()
            .sum();
        assert_eq!(total, Money::from_cents(30));
        assert_eq!(-Money::from_cents(5), Money::from_cents(-5));
    }

    proptest::proptest! {
        #[test]
        fn money_format_parse_round_trips(cents in -99_999_999_999_i64..=99_999_999_999_i64) {
            let m = Money::from_cents(cents);
            proptest::prop_assert_eq!(Money::parse(&m.format_brl()).unwrap(), m);
        }

        #[test]
        fn three_decimals_are_always_rejected(whole in 0u32..100_000, frac in 100u32..1000) {
            let text = format!("{whole},{frac:03}");
            proptest::prop_assert!(Money::parse(&text).is_err());
        }
    }
}
