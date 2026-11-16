//! Spending and income categories.

use serde::{Deserialize, Serialize};

use super::transaction::TransactionType;

/// Which transaction kinds a category may be used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CategoryScope {
    #[default]
    Expense,
    Income,
    Both,
}

impl CategoryScope {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            CategoryScope::Expense => "expense",
            CategoryScope::Income => "income",
            CategoryScope::Both => "both",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "expense" => Some(CategoryScope::Expense),
            "income" => Some(CategoryScope::Income),
            "both" => Some(CategoryScope::Both),
            _ => None,
        }
    }

    /// Refunds use the expense scope. Settlements never use categories.
    #[must_use]
    pub fn allows(self, kind: TransactionType) -> bool {
        if kind == TransactionType::Settlement {
            return false;
        }
        matches!(
            (self, kind),
            (CategoryScope::Both, _)
                | (CategoryScope::Income, TransactionType::Income)
                | (
                    CategoryScope::Expense,
                    TransactionType::Expense | TransactionType::Refund
                )
        )
    }
}

pub const DEFAULT_CATEGORY_COLOR: &str = "#8F98A8";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: i64,
    pub name: String,
    /// `#RRGGBB`.
    pub color: String,
    pub icon: Option<String>,
    pub applies_to: CategoryScope,
    pub is_default: bool,
    pub archived: bool,
    /// UTC RFC 3339.
    pub created_at: String,
}

impl Default for Category {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            color: DEFAULT_CATEGORY_COLOR.to_string(),
            icon: None,
            applies_to: CategoryScope::default(),
            is_default: false,
            archived: false,
            created_at: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_filters_by_kind() {
        assert!(CategoryScope::Expense.allows(TransactionType::Expense));
        assert!(CategoryScope::Expense.allows(TransactionType::Refund));
        assert!(!CategoryScope::Expense.allows(TransactionType::Income));
        assert!(CategoryScope::Income.allows(TransactionType::Income));
        assert!(!CategoryScope::Income.allows(TransactionType::Expense));
        assert!(CategoryScope::Both.allows(TransactionType::Income));
        assert!(CategoryScope::Both.allows(TransactionType::Refund));
    }

    #[test]
    fn category_scope_never_allows_settlement() {
        for scope in [
            CategoryScope::Expense,
            CategoryScope::Income,
            CategoryScope::Both,
        ] {
            assert!(!scope.allows(TransactionType::Settlement));
        }
    }

    #[test]
    fn default_category_uses_the_neutral_color() {
        assert_eq!(Category::default().color, "#8F98A8");
        assert_eq!(CategoryScope::from_code("both"), Some(CategoryScope::Both));
        assert_eq!(CategoryScope::from_code("x"), None);
    }
}
