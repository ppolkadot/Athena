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
    dirs.ensure()?;
    let _instance = app::instance::acquire(&dirs.root)?;
    let _log_guard = init_tracing(&dirs)?;
    tracing::info!("athena {} starting", env!("CARGO_PKG_VERSION"));

    let session = open_session(dirs)?;

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
            | AppError::Migration(_)
            | AppError::SchemaTooNew { .. }
            | AppError::Db(_)
    )
}

/// Open the app on the database, or start the recovery session when it cannot be read.
///
/// # Errors
/// Startup failures that the recovery screen cannot help with, such as [`AppError::Dirs`].
pub fn open_session(dirs: AppDirs) -> Result<AppSession> {
    let opened = if db_recovery::needs_recovery_before_open(&dirs)? {
        Err(AppError::LiveDatabaseMissingOrEmpty)
    } else {
        Db::open(&dirs.db_path).and_then(|db| AppCore::new(db, dirs.clone()))
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
}
