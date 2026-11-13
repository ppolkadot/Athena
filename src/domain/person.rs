//! People who share the household. The owner row is the only source of the owner's name.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PersonKind {
    Owner,
    Partner,
    Other,
}

impl PersonKind {
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            PersonKind::Owner => "owner",
            PersonKind::Partner => "partner",
            PersonKind::Other => "other",
        }
    }

    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "owner" => Some(PersonKind::Owner),
            "partner" => Some(PersonKind::Partner),
            "other" => Some(PersonKind::Other),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub id: i64,
    pub name: String,
    pub kind: PersonKind,
    pub archived: bool,
    /// UTC RFC 3339.
    pub created_at: String,
}
