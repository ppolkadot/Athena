//! Entry templates. The type exists in 1.0 so the JSON profile can carry an empty `templates`
//! array; repository and UI arrive in 1.1.

use serde::{Deserialize, Serialize};

use super::money::Money;
use super::transaction::TransactionType;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub title: String,
    pub amount: Money,
    pub kind: TransactionType,
    pub category_id: Option<i64>,
    pub person_id: Option<i64>,
    pub description: Option<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}
