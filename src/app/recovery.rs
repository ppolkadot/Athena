//! Startup recovery session (`§11`).
//!
//! When the database cannot be opened, Athena still starts and shows the recovery screen instead
//! of the app. This module owns that session: the failure, the read-only backups list, the restore
//! attempt, and the switch back to a normal [`AppCore`] once a restore succeeds.

use std::path::Path;

use chrono::Utc;
use eframe::egui;

use crate::domain::settings::{Language, Theme};
use crate::i18n::{restore_error_message, strings};
use crate::infrastructure::config::AppDirs;
use crate::infrastructure::db::backup::{self, BackupFile};
use crate::infrastructure::db::{Db, recovery};
use crate::shared::error::AppError;
use crate::ui::screens::recovery::{self as recovery_screen, RecoveryEvent};
use crate::ui::theme;

use super::AppCore;
use super::files::open_folder;

/// Which session the window holds for the rest of the process. `Store.db` stays a non-optional
/// [`Db`]; recovery is a separate state, not an app with a missing database.
#[derive(Debug)]
pub enum AppSession {
    Active(Box<AppCore>),
    Recovery(Box<RecoveryState>),
}

/// What the recovery screen can act on: the folder, the failure, and the backups worth trying.
#[derive(Debug)]
pub struct RecoveryState {
    pub dirs: AppDirs,
    pub error: AppError,
    pub backups: Vec<BackupFile>,
    pub restore_error: Option<String>,
    pub language: Language,
}

impl RecoveryState {
    /// Read the backups list once. A folder that cannot be listed just yields an empty list: the
    /// recovery screen must still open so the user can reach the data folder and quit.
    #[must_use]
    pub fn new(dirs: AppDirs, error: AppError) -> Self {
        let backups = backup::list(&dirs.backups_dir).unwrap_or_else(|err| {
            tracing::warn!("could not list backups for recovery: {err}");
            Vec::new()
        });
        Self {
            dirs,
            error,
            backups,
            restore_error: None,
            language: Language::from_system(),
        }
    }

    /// The failure in the user's language: the localized corrupt-file advice, or the raw error for
    /// the cases that carry no dedicated copy.
    #[must_use]
    pub fn detail(&self, language: Language) -> String {
        match &self.error {
            AppError::Corrupt { path } => strings(language).startup_corrupt.replace("{path}", path),
            AppError::LiveDatabaseMissingOrEmpty => strings(language).recovery_missing.into(),
            AppError::RecoveryScanUncertain(detail) => strings(language)
                .recovery_scan_uncertain
                .replace("{detail}", detail),
            AppError::LiveMetadataIndeterminate(detail) => strings(language)
                .recovery_live_indeterminate
                .replace("{detail}", detail),
            AppError::RecoveryConflict(detail) => strings(language)
                .recovery_conflict
                .replace("{detail}", detail),
            other => other.to_string(),
        }
    }

    pub(crate) fn can_start_fresh(&self) -> bool {
        matches!(
            self.error,
            AppError::LiveDatabaseMissingOrEmpty | AppError::RecoveryScanUncertain(_)
        )
    }

    /// Re-probe disk recovery state and backup list after a failed recovery operation so UI copy
    /// and `Start fresh` eligibility reflect the current disk state.
    pub(crate) fn refresh_after_error(&mut self, operation_err: &AppError) {
        self.backups = backup::list(&self.dirs.backups_dir).unwrap_or_else(|err| {
            tracing::warn!("could not list backups for recovery: {err}");
            Vec::new()
        });
        match recovery::probe_startup(&self.dirs) {
            recovery::StartupProbe::Conflict(detail) => {
                self.error = AppError::RecoveryConflict(detail);
            }
            recovery::StartupProbe::IndeterminateLive(detail) => {
                self.error = AppError::LiveMetadataIndeterminate(detail);
            }
            recovery::StartupProbe::MissingOrEmpty {
                scan_error: Some(detail),
                ..
            } => {
                self.error = AppError::RecoveryScanUncertain(detail);
            }
            recovery::StartupProbe::MissingOrEmpty {
                scan_error: None, ..
            }
            | recovery::StartupProbe::FirstLaunch => {
                self.error = AppError::LiveDatabaseMissingOrEmpty;
            }
            recovery::StartupProbe::ReadyToOpen => {
                self.error = match operation_err {
                    AppError::Corrupt { path } => AppError::Corrupt { path: path.clone() },
                    AppError::Migration(msg) => AppError::Migration(msg.clone()),
                    AppError::SchemaTooNew { found, supported } => AppError::SchemaTooNew {
                        found: *found,
                        supported: *supported,
                    },
                    AppError::RecoveryConflict(detail) => {
                        AppError::RecoveryConflict(detail.clone())
                    }
                    AppError::LiveMetadataIndeterminate(detail) => {
                        AppError::LiveMetadataIndeterminate(detail.clone())
                    }
                    _ if self.can_start_fresh() => AppError::RecoveryConflict(format!(
                        "{} is no longer missing or empty",
                        self.dirs.db_path.display()
                    )),
                    _ => return,
                };
            }
        }
    }

    /// The warning shown before a restore when the data is newer than this build, which a restore
    /// would replace.
    #[must_use]
    pub fn restore_warning(&self, language: Language) -> Option<String> {
        match &self.error {
            AppError::SchemaTooNew { found, supported } => Some(
                strings(language)
                    .startup_schema_too_new
                    .replace("{found}", &found.to_string())
                    .replace("{supported}", &supported.to_string()),
            ),
            _ => None,
        }
    }
}

/// The recovery window. It draws the screen and, after a successful restore, hands back the
/// [`AppCore`] the caller swaps in.
pub struct RecoveryApp {
    state: RecoveryState,
    confirm_start_fresh: bool,
}

impl RecoveryApp {
    #[must_use]
    pub fn new(ctx: &egui::Context, state: RecoveryState) -> Self {
        theme::install_fonts(ctx);
        theme::apply(ctx, Theme::Dark);
        Self {
            state,
            confirm_start_fresh: false,
        }
    }

    #[must_use]
    pub fn state(&self) -> &RecoveryState {
        &self.state
    }

    /// One frame of the recovery screen. Returns the opened core once a restore succeeded.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<AppCore> {
        let ctx = ui.ctx().clone();
        for event in recovery_screen::show(ui, &self.state) {
            match event {
                RecoveryEvent::OpenDataFolder => open_folder(&self.state.dirs.root),
                RecoveryEvent::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                RecoveryEvent::StartFresh => self.confirm_start_fresh = true,
                RecoveryEvent::Restore(path) => {
                    if let Some(core) = restore_and_activate(&mut self.state, &path) {
                        return Some(core);
                    }
                }
            }
        }
        if self.confirm_start_fresh {
            let s = strings(self.state.language);
            let modal =
                egui::Modal::new(egui::Id::unique("recovery-start-fresh")).show(&ctx, |ui| {
                    ui.set_max_width(380.0);
                    ui.heading(s.recovery_start_fresh);
                    ui.label(s.recovery_start_fresh_confirm);
                    ui.horizontal(|ui| {
                        (
                            ui.button(s.cancel).clicked(),
                            ui.button(s.recovery_start_fresh).clicked(),
                        )
                    })
                    .inner
                });
            if modal.inner.0 || modal.should_close() {
                self.confirm_start_fresh = false;
            } else if modal.inner.1 {
                self.confirm_start_fresh = false;
                return start_fresh_and_activate(&mut self.state);
            }
        }
        None
    }
}

fn start_fresh_and_activate(state: &mut RecoveryState) -> Option<AppCore> {
    let result = recovery::start_fresh(&state.dirs, Utc::now())
        .and_then(|()| Db::open(&state.dirs.db_path))
        // An explicit fresh start must not rotate or replace the user's old backups.
        .and_then(|db| AppCore::open(db, state.dirs.clone(), false));
    match result {
        Ok(core) => {
            state.restore_error = None;
            Some(core)
        }
        Err(err) => {
            tracing::warn!("could not start a fresh profile: {err}");
            state.restore_error = Some(strings(state.language).recovery_start_fresh_failed.into());
            state.refresh_after_error(&err);
            None
        }
    }
}

/// Restore `source` over the unreadable database and open the app on it. On failure the state keeps
/// the localized message so the screen can show it and stay in recovery.
pub fn restore_and_activate(state: &mut RecoveryState, source: &Path) -> Option<AppCore> {
    match recovery::restore_over_broken(&state.dirs, source, Utc::now()) {
        Ok(broken) => {
            tracing::info!(
                "recovery restored {} over the unreadable database; broken file kept at {}",
                source.display(),
                broken.display()
            );
            state.restore_error = None;
            match open_core(state) {
                Ok(core) => Some(core),
                Err(err) => {
                    record_failure(state, &err);
                    None
                }
            }
        }
        Err(err) => {
            record_failure(state, &err);
            None
        }
    }
}

fn open_core(state: &RecoveryState) -> crate::shared::error::Result<AppCore> {
    let db = Db::open(&state.dirs.db_path)?;
    AppCore::new(db, state.dirs.clone())
}

fn record_failure(state: &mut RecoveryState, err: &AppError) {
    tracing::warn!("recovery could not restore the database: {err}");
    let detail = match err {
        AppError::Restore(restore_err) => restore_error_message(restore_err, state.language),
        other => other.to_string(),
    };
    state.restore_error = Some(
        strings(state.language)
            .recovery_restore_failed
            .replace("{detail}", &detail),
    );
    state.refresh_after_error(err);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::domain::settings::Profile;
    use crate::infrastructure::db::profile_repo;
    use crate::shared::test_fixtures::seed_people;

    fn temp_dirs() -> (tempfile::TempDir, AppDirs) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        (dir, dirs)
    }

    /// A valid Athena database written at `path`, the shape a backup has.
    fn write_valid_db(path: &Path) {
        let db = Db::open(path).unwrap();
        profile_repo::save(
            db.conn(),
            &Profile {
                start_balance: Money::from_cents(10_000),
                theme: Theme::Dark,
                language: Language::En,
                onboarding_complete: true,
            },
        )
        .unwrap();
        seed_people(db.conn());
    }

    #[test]
    fn schema_too_new_warns_before_restore() {
        let (_dir, dirs) = temp_dirs();
        let state = RecoveryState::new(
            dirs,
            AppError::SchemaTooNew {
                found: 9,
                supported: 3,
            },
        );

        let warning = state.restore_warning(Language::En).unwrap();
        assert!(warning.contains('9') && warning.contains('3'), "{warning}");
        assert!(state.restore_warning(Language::PtBr).is_some());
        assert!(!state.detail(Language::En).trim().is_empty());
    }

    #[test]
    fn only_a_newer_schema_carries_a_restore_warning() {
        let (_dir, dirs) = temp_dirs();
        let corrupt = RecoveryState::new(
            dirs.clone(),
            AppError::Corrupt {
                path: dirs.db_path.display().to_string(),
            },
        );
        assert!(corrupt.restore_warning(Language::En).is_none());
        assert!(corrupt.detail(Language::En).contains("athena.db"));

        let migration = RecoveryState::new(dirs, AppError::Migration("step 2".into()));
        assert!(migration.restore_warning(Language::En).is_none());
        assert!(migration.detail(Language::En).contains("step 2"));
    }

    #[test]
    fn a_failed_restore_stays_in_recovery_with_a_localized_message() {
        let (_dir, dirs) = temp_dirs();
        let garbage = dirs.backups_dir.join("manual-garbage.db");
        std::fs::write(&garbage, b"not a database").unwrap();
        let mut state = RecoveryState::new(
            dirs.clone(),
            AppError::Corrupt {
                path: dirs.db_path.display().to_string(),
            },
        );

        assert!(restore_and_activate(&mut state, &garbage).is_none());

        let message = state.restore_error.as_deref().unwrap();
        assert!(!message.trim().is_empty());
        assert_ne!(
            message,
            strings(Language::En).recovery_restore_failed,
            "the message keeps its detail"
        );
    }

    #[test]
    fn start_fresh_keeps_backups_and_enters_onboarding_only_after_confirmation() {
        let (_dir, dirs) = temp_dirs();
        let backup = dirs.backups_dir.join("manual-2026-11-16_12-00-00.db");
        write_valid_db(&backup);
        let backup_bytes = std::fs::read(&backup).unwrap();
        let AppSession::Recovery(mut state) = crate::open_session(dirs.clone()).unwrap() else {
            panic!("existing backups must prompt for recovery");
        };
        assert!(state.can_start_fresh());
        assert!(!dirs.db_path.exists());
        let core = start_fresh_and_activate(&mut state).unwrap();
        assert!(!core.vm.onboarding_complete());
        assert_eq!(std::fs::read(&backup).unwrap(), backup_bytes);
        assert_eq!(std::fs::read_dir(&dirs.backups_dir).unwrap().count(), 1);
        drop(core);
        assert!(matches!(
            crate::open_session(dirs).unwrap(),
            AppSession::Active(_)
        ));
    }

    #[test]
    fn recovery_restore_moves_broken_files_aside_and_activates() {
        let (_dir, dirs) = temp_dirs();
        let source = dirs.backups_dir.join("manual-good.db");
        write_valid_db(&source);
        std::fs::write(&dirs.db_path, b"not a database").unwrap();
        let mut state = RecoveryState::new(
            dirs.clone(),
            AppError::Corrupt {
                path: dirs.db_path.display().to_string(),
            },
        );

        let core = restore_and_activate(&mut state, &source).unwrap();

        assert_eq!(
            core.vm.profile.as_ref().unwrap().start_balance,
            Money::from_cents(10_000)
        );
        assert!(state.restore_error.is_none());
    }

    #[test]
    fn start_fresh_gating_and_localized_details_cover_probe_and_conflict_states() {
        let (_dir, dirs) = temp_dirs();
        let scan = RecoveryState::new(
            dirs.clone(),
            AppError::RecoveryScanUncertain("permission denied".into()),
        );
        assert!(scan.can_start_fresh());

        let indeterminate = RecoveryState::new(
            dirs.clone(),
            AppError::LiveMetadataIndeterminate("not a regular file".into()),
        );
        assert!(!indeterminate.can_start_fresh());

        let conflict = RecoveryState::new(
            dirs,
            AppError::RecoveryConflict("unexpected live file".into()),
        );
        assert!(!conflict.can_start_fresh());

        for state in [&scan, &indeterminate, &conflict] {
            let en = state.detail(Language::En);
            let pt = state.detail(Language::PtBr);
            assert!(!en.trim().is_empty() && !pt.trim().is_empty());
            assert_ne!(en, pt);
            assert!(!en.contains('{') && !pt.contains('{'));
        }
    }

    #[test]
    fn failed_recovery_operations_refresh_eligibility_and_error_state() {
        // 1. Start fresh fails when a non-empty live database appears -> Start fresh becomes disabled.
        {
            let (_dir, dirs) = temp_dirs();
            let backup = dirs.backups_dir.join("manual-2026-11-16_12-00-00.db");
            write_valid_db(&backup);
            let AppSession::Recovery(mut state) = crate::open_session(dirs.clone()).unwrap() else {
                panic!("existing backup must enter Recovery");
            };
            assert!(state.can_start_fresh());

            std::fs::write(&dirs.db_path, b"unexpected nonempty live db").unwrap();
            assert!(start_fresh_and_activate(&mut state).is_none());
            assert!(state.restore_error.is_some());
            assert!(!state.can_start_fresh());
            assert!(matches!(state.error, AppError::RecoveryConflict(_)));
        }

        // 2. Restore fails because a conflicting intent appeared on disk -> state refreshes to Conflict.
        {
            let (_dir, dirs) = temp_dirs();
            let backup = dirs.backups_dir.join("manual-2026-11-16_12-00-00.db");
            write_valid_db(&backup);
            let AppSession::Recovery(mut state) = crate::open_session(dirs.clone()).unwrap() else {
                panic!("existing backup must enter Recovery");
            };
            assert!(state.can_start_fresh());

            std::fs::write(dirs.root.join("recovery-intent.json"), b"corrupt json").unwrap();
            assert!(restore_and_activate(&mut state, &backup).is_none());
            assert!(state.restore_error.is_some());
            assert!(!state.can_start_fresh());
            assert!(matches!(state.error, AppError::RecoveryConflict(_)));
        }
    }
}
