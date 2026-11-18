pub mod app;
pub mod application;
pub mod domain;
pub mod i18n;
pub mod infrastructure;
pub mod shared;
pub mod ui;

use crate::app::recovery::{AppSession, RecoveryApp, RecoveryState};
use crate::app::{AppCore, AthenaApp};
use crate::infrastructure::config::{AppDirs, init_tracing};
use crate::infrastructure::db::{Db, recovery as db_recovery};
use crate::shared::error::{AppError, Result};

/// Resolve folders, take the single-instance lock, start logging, open the window.
///
/// # Errors
/// Any startup failure that a person cannot fix inside the app. An unreadable database is not one
/// of them: see [`open_session`].
pub fn run() -> Result<()> {
    let dirs = AppDirs::resolve()?;
    let (_instance, _log_guard, session) = boot_session_with_dirs(dirs, true)?;

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Athena")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([760.0, 560.0])
            .with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Athena",
        options,
        Box::new(move |cc| Ok(Box::new(AthenaWindow::new(cc, session)))),
    )
    .map_err(|e| AppError::Ui(e.to_string()))
}

fn boot_session_with_dirs(
    dirs: AppDirs,
    initialize_tracing: bool,
) -> Result<(
    app::instance::InstanceGuard,
    Option<tracing_appender::non_blocking::WorkerGuard>,
    AppSession,
)> {
    dirs.ensure_essential()?;
    let instance = app::instance::acquire(&dirs.root)?;
    let log_guard = if initialize_tracing {
        let guard = init_tracing(&dirs)?;
        tracing::info!("athena {} starting", env!("CARGO_PKG_VERSION"));
        Some(guard)
    } else {
        None
    };
    let session = open_session(dirs)?;
    Ok((instance, log_guard, session))
}

/// Whether a startup failure leaves the app usable through the recovery session (`§11`).
///
/// `AlreadyRunning` and `Dirs` stay fatal: no restore can fix a second copy of Athena or a folder
/// that cannot be created, and the binary reports them in a message box.
#[must_use]
pub fn is_recoverable(error: &AppError) -> bool {
    matches!(
        error,
        AppError::Corrupt { .. }
            | AppError::LiveDatabaseMissingOrEmpty
            | AppError::RecoveryScanUncertain(_)
            | AppError::LiveMetadataIndeterminate(_)
            | AppError::RecoveryConflict(_)
            | AppError::Migration(_)
            | AppError::SchemaTooNew { .. }
            | AppError::Db(_)
            | AppError::Io(_)
    )
}

/// Open the app on the database, or start the recovery session when it cannot be read.
///
/// # Errors
/// Startup failures that the recovery screen cannot help with, such as [`AppError::Dirs`].
pub fn open_session(dirs: AppDirs) -> Result<AppSession> {
    let opened = match db_recovery::probe_startup(&dirs) {
        db_recovery::StartupProbe::FirstLaunch | db_recovery::StartupProbe::ReadyToOpen => dirs
            .ensure_recoverable()
            .and_then(|()| Db::open(&dirs.db_path))
            .and_then(|db| AppCore::new(db, dirs.clone())),
        db_recovery::StartupProbe::MissingOrEmpty {
            scan_error: None, ..
        } => Err(AppError::LiveDatabaseMissingOrEmpty),
        db_recovery::StartupProbe::MissingOrEmpty {
            scan_error: Some(detail),
            ..
        } => Err(AppError::RecoveryScanUncertain(detail)),
        db_recovery::StartupProbe::IndeterminateLive(detail) => {
            Err(AppError::LiveMetadataIndeterminate(detail))
        }
        db_recovery::StartupProbe::Conflict(detail) => Err(AppError::RecoveryConflict(detail)),
    };
    match opened {
        Ok(core) => Ok(AppSession::Active(Box::new(core))),
        Err(err) if is_recoverable(&err) => {
            tracing::warn!("the database could not be opened; starting in recovery: {err}");
            Ok(AppSession::Recovery(Box::new(RecoveryState::new(
                dirs, err,
            ))))
        }
        Err(err) => Err(err),
    }
}

/// The window the event loop drives. A recovery session becomes an active app in place once a
/// restore succeeds, so the process keeps its single-instance lock and its log.
enum AthenaWindow {
    Active(Box<AthenaApp>),
    Recovery(Box<RecoveryApp>),
}

impl AthenaWindow {
    fn new(cc: &eframe::CreationContext<'_>, session: AppSession) -> Self {
        match session {
            AppSession::Active(core) => Self::Active(Box::new(AthenaApp::new(cc, *core))),
            AppSession::Recovery(state) => {
                Self::Recovery(Box::new(RecoveryApp::new(&cc.egui_ctx, *state)))
            }
        }
    }
}

impl eframe::App for AthenaWindow {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, frame: &mut eframe::Frame) {
        match self {
            Self::Active(app) => <AthenaApp as eframe::App>::ui(app.as_mut(), ui, frame),
            Self::Recovery(app) => {
                let activated = app.show(ui);
                if let Some(core) = activated {
                    *self = Self::Active(Box::new(AthenaApp::from_core(ui.ctx(), core)));
                }
            }
        }
    }
}

/// The 64x64 RGBA window and taskbar icon, embedded in the binary.
fn window_icon() -> eframe::egui::IconData {
    eframe::egui::IconData {
        rgba: include_bytes!("../assets/icon-64.rgba").to_vec(),
        width: 64,
        height: 64,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp_dirs() -> (tempfile::TempDir, AppDirs) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        (dir, dirs)
    }

    #[test]
    fn already_running_still_uses_message_box() {
        assert!(!is_recoverable(&AppError::AlreadyRunning));
        assert!(!is_recoverable(&AppError::Dirs("no home folder".into())));
        assert!(!is_recoverable(&AppError::NotFound("profile")));
    }

    #[test]
    fn unreadable_database_failures_enter_recovery() {
        assert!(is_recoverable(&AppError::Corrupt {
            path: "athena.db".into()
        }));
        assert!(is_recoverable(&AppError::Migration("step 3".into())));
        assert!(is_recoverable(&AppError::SchemaTooNew {
            found: 9,
            supported: 3
        }));
        assert!(is_recoverable(&AppError::Db(
            rusqlite::Error::QueryReturnedNoRows
        )));
    }

    #[test]
    fn corrupt_database_opens_recovery_session() {
        let (_dir, dirs) = temp_dirs();
        std::fs::write(&dirs.db_path, b"this is not a database").unwrap();

        let session = open_session(dirs).unwrap();

        match session {
            AppSession::Recovery(state) => {
                assert!(state.backups.is_empty());
                assert!(!state.detail(state.language).trim().is_empty());
            }
            AppSession::Active(_) => panic!("garbage at athena.db must not open the app"),
        }
    }

    #[test]
    fn a_missing_database_still_opens_the_app() {
        let (_dir, dirs) = temp_dirs();

        let session = open_session(dirs).unwrap();

        assert!(matches!(session, AppSession::Active(_)));
    }
    #[test]
    fn interrupted_recovery_does_not_create_a_fresh_profile_on_restart() {
        let (_dir, dirs) = temp_dirs();
        let broken = dirs.root.join("broken-2026-11-17_12-00-00.db");
        std::fs::write(&broken, b"original database").unwrap();
        assert!(matches!(
            open_session(dirs.clone()).unwrap(),
            AppSession::Recovery(_)
        ));
        assert!(!dirs.db_path.exists());
        assert_eq!(std::fs::read(broken).unwrap(), b"original database");
    }

    #[test]
    fn recovery_does_not_initialize_a_zero_byte_database() {
        let (_dir, dirs) = temp_dirs();
        std::fs::write(&dirs.db_path, []).unwrap();
        assert!(matches!(
            open_session(dirs.clone()).unwrap(),
            AppSession::Recovery(_)
        ));
        assert_eq!(std::fs::metadata(&dirs.db_path).unwrap().len(), 0);
    }

    #[test]
    fn scan_failures_indeterminate_live_and_conflicts_enter_recovery_with_strict_start_fresh_gating()
     {
        assert!(is_recoverable(&AppError::RecoveryScanUncertain(
            "io".into()
        )));
        assert!(is_recoverable(&AppError::LiveMetadataIndeterminate(
            "dir".into()
        )));
        assert!(is_recoverable(&AppError::RecoveryConflict(
            "mismatch".into()
        )));
        assert!(is_recoverable(&AppError::Io(std::io::Error::other("io"))));

        // Indeterminate live path enters Recovery without Start fresh.
        {
            let (_dir, dirs) = temp_dirs();
            std::fs::create_dir(&dirs.db_path).unwrap();
            let AppSession::Recovery(state) = open_session(dirs).unwrap() else {
                panic!("directory at db_path must enter Recovery");
            };
            assert!(matches!(
                state.error,
                AppError::LiveMetadataIndeterminate(_)
            ));
            assert!(!state.can_start_fresh());
        }

        // Scan failure with missing DB enters Recovery with Start fresh allowed.
        {
            let (_dir, dirs) = temp_dirs();
            std::fs::remove_dir(&dirs.backups_dir).unwrap();
            std::fs::write(&dirs.backups_dir, b"not a directory").unwrap();
            let AppSession::Recovery(state) = open_session(dirs).unwrap() else {
                panic!("unreadable backups_dir must enter Recovery");
            };
            assert!(matches!(state.error, AppError::RecoveryScanUncertain(_)));
            assert!(state.can_start_fresh());
        }
    }

    #[test]
    fn production_startup_sequence_reaches_recovery_when_backups_or_staging_is_a_regular_file() {
        // 1. Backups replaced by a regular file with missing DB enters Recovery with Start fresh.
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            std::fs::write(&dirs.backups_dir, b"not a directory").unwrap();

            let (_guard, _log, session) = boot_session_with_dirs(dirs, false).unwrap();
            let AppSession::Recovery(state) = session else {
                panic!("backups replaced by regular file must reach Recovery via startup sequence");
            };
            assert!(matches!(state.error, AppError::RecoveryScanUncertain(_)));
            assert!(state.can_start_fresh());
        }

        // 2. Staging replaced by a regular file with missing DB enters Recovery with Start fresh.
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            std::fs::write(&dirs.staging_dir, b"not a directory").unwrap();

            let (_guard, _log, session) = boot_session_with_dirs(dirs, false).unwrap();
            let AppSession::Recovery(state) = session else {
                panic!("staging replaced by regular file must reach Recovery via startup sequence");
            };
            assert!(matches!(state.error, AppError::RecoveryScanUncertain(_)));
            assert!(state.can_start_fresh());
        }

        // 3. Backups replaced by a regular file alongside a valid live DB enters Recovery without Start fresh.
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            dirs.ensure().unwrap();
            drop(Db::open(&dirs.db_path).unwrap());
            std::fs::remove_dir(&dirs.backups_dir).unwrap();
            std::fs::write(&dirs.backups_dir, b"not a directory").unwrap();

            let (_guard, _log, session) = boot_session_with_dirs(dirs, false).unwrap();
            let AppSession::Recovery(state) = session else {
                panic!("blocked backups dir alongside valid DB must reach Recovery");
            };
            assert!(matches!(state.error, AppError::Io(_)));
            assert!(!state.can_start_fresh());
        }
    }

    #[test]
    fn child_startup_sequence_with_tracing_survives_blocked_backups_file() {
        const ENV_ROOT: &str = "ATHENA_TEST_BOOT_ROOT";
        if let Ok(root) = std::env::var(ENV_ROOT) {
            let dirs = AppDirs::from_root(PathBuf::from(root));
            let (_guard, _log_guard, session) = boot_session_with_dirs(dirs, true).unwrap();
            assert!(matches!(session, AppSession::Recovery(_)));
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        std::fs::write(&dirs.backups_dir, b"regular file blocking backups dir").unwrap();

        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(exe)
            .args([
                "--exact",
                "tests::child_startup_sequence_with_tracing_survives_blocked_backups_file",
                "--nocapture",
            ])
            .env(ENV_ROOT, &dirs.root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child startup failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(dirs.log_dir.is_dir());
    }
}
