//! Text folding for accent- and case-insensitive search and duplicate-name checks.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// NFD, strip combining marks, lowercase. `"Pão"` -> `"pao"`, `"ALIMENTAÇÃO"` -> `"alimentacao"`.
#[must_use]
pub fn fold(text: &str) -> String {
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_removes_accents_and_case() {
        assert_eq!(fold("Pão"), "pao");
        assert_eq!(fold("ALIMENTAÇÃO"), fold("Alimentação"));
    }

    #[test]
    fn fold_leaves_plain_ascii_alone() {
        assert_eq!(fold("market 2"), "market 2");
        assert_eq!(fold(""), "");
    }
}
