//! First-run setup: profile, owner and optional partner, created together or not at all.

use rusqlite::Connection;

use crate::domain::money::Money;
use crate::domain::person::PersonKind;
use crate::domain::settings::{Language, Profile, Theme};
use crate::domain::text::fold;
use crate::infrastructure::db::{people_repo, profile_repo};
use crate::shared::error::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingResult {
    pub display_name: String,
    pub partner_name: Option<String>,
    pub start_balance: Money,
    pub language: Language,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingError {
    NameRequired,
    PartnerNameTaken,
}

impl OnboardingResult {
    /// # Errors
    /// [`OnboardingError::NameRequired`] for a blank name; [`OnboardingError::PartnerNameTaken`]
    /// when the partner has the same name as the owner (ignoring case and accents).
    pub fn validate(&self) -> std::result::Result<(), OnboardingError> {
        let owner = self.display_name.trim();
        if owner.is_empty() {
            return Err(OnboardingError::NameRequired);
        }
        let partner = self
            .partner_name
            .as_deref()
            .map(str::trim)
            .unwrap_or_default();
        if !partner.is_empty() && fold(partner) == fold(owner) {
            return Err(OnboardingError::PartnerNameTaken);
        }
        Ok(())
    }
}

/// Create the profile, owner and partner in one SQL transaction. A failure leaves nothing behind.
/// Re-running keeps the existing theme and renames the existing owner instead of adding a second.
///
/// # Errors
/// [`crate::shared::error::AppError::Validation`] for invalid input; database errors otherwise.
pub fn complete(conn: &Connection, result: &OnboardingResult) -> Result<()> {
    result
        .validate()
        .map_err(|e| crate::shared::error::AppError::Validation(format!("{e:?}")))?;
    let tx = conn.unchecked_transaction()?;
    let theme = profile_repo::load(&tx)?.map_or(Theme::Dark, |p| p.theme);
    profile_repo::save(
        &tx,
        &Profile {
            start_balance: result.start_balance,
            theme,
            language: result.language,
            onboarding_complete: true,
        },
    )?;
    match people_repo::owner(&tx)? {
        Some(mut owner) => {
            owner.name.clone_from(&result.display_name);
            people_repo::update(&tx, &owner)?;
        }
        None => {
            people_repo::insert(&tx, &result.display_name, PersonKind::Owner)?;
        }
    }
    if let Some(partner) = result
        .partner_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        let existing = people_repo::list(&tx, false)?
            .into_iter()
            .any(|p| p.kind == PersonKind::Partner);
        if !existing {
            people_repo::insert(&tx, partner, PersonKind::Partner)?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::Db;

    fn result(name: &str, partner: Option<&str>) -> OnboardingResult {
        OnboardingResult {
            display_name: name.into(),
            partner_name: partner.map(Into::into),
            start_balance: Money::from_cents(10_000),
            language: Language::PtBr,
        }
    }

    #[test]
    fn onboarding_rejects_blank_name() {
        assert_eq!(
            result("  ", None).validate(),
            Err(OnboardingError::NameRequired)
        );
        assert_eq!(
            result("A3", Some("a3")).validate(),
            Err(OnboardingError::PartnerNameTaken)
        );
        assert!(result("A3", Some("  ")).validate().is_ok());
    }

    #[test]
    fn creates_profile_owner_and_partner_together() {
        let db = Db::open_in_memory().unwrap();
        complete(db.conn(), &result("A3", Some("Ana"))).unwrap();
        let profile = profile_repo::load(db.conn()).unwrap().unwrap();
        assert!(profile.onboarding_complete);
        assert_eq!(profile.language, Language::PtBr);
        assert_eq!(profile.start_balance, Money::from_cents(10_000));
        let people = people_repo::list(db.conn(), true).unwrap();
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].kind, PersonKind::Owner);
    }

    #[test]
    fn partner_is_optional() {
        let db = Db::open_in_memory().unwrap();
        complete(db.conn(), &result("A3", None)).unwrap();
        assert_eq!(people_repo::list(db.conn(), true).unwrap().len(), 1);
    }

    #[test]
    fn a_failure_leaves_nothing_behind() {
        let db = Db::open_in_memory().unwrap();
        // Skip validate() by calling with a blank partner that passes, but break the second insert:
        // an existing archived-free person with the partner's folded name makes insert fail.
        people_repo::insert(db.conn(), "Ana", PersonKind::Other).unwrap();
        let outcome = complete(db.conn(), &result("A3", Some("ANA")));
        assert!(outcome.is_err());
        assert_eq!(profile_repo::load(db.conn()).unwrap(), None);
        let names: Vec<_> = people_repo::list(db.conn(), true)
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["Ana"]);
    }

    #[test]
    fn rerunning_renames_the_owner_and_keeps_the_theme() {
        let db = Db::open_in_memory().unwrap();
        complete(db.conn(), &result("A3", Some("Ana"))).unwrap();
        let mut profile = profile_repo::load(db.conn()).unwrap().unwrap();
        profile.theme = Theme::Light;
        profile_repo::save(db.conn(), &profile).unwrap();
        complete(db.conn(), &result("Alex", Some("Ana"))).unwrap();
        assert_eq!(people_repo::list(db.conn(), true).unwrap().len(), 2);
        assert_eq!(people_repo::owner(db.conn()).unwrap().unwrap().name, "Alex");
        assert_eq!(
            profile_repo::load(db.conn()).unwrap().unwrap().theme,
            Theme::Light
        );
    }
}
