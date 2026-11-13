#![cfg_attr(test, allow(clippy::assert_is_empty))]

pub mod app;
pub mod application;
pub mod domain;
pub mod i18n;
pub mod infrastructure;
pub mod shared;
pub mod ui;

use crate::app::{AppCore, AthenaApp};
use crate::infrastructure::config::{AppDirs, init_tracing};
use crate::infrastructure::db::Db;
use crate::shared::error::{AppError, Result};

/// Resolve folders, take the single-instance lock, start logging, open the window.
///
/// # Errors
/// Any startup failure. The binary shows it in a message box and exits non-zero.
pub fn run() -> Result<()> {
    let dirs = AppDirs::resolve()?;
    dirs.ensure()?;
    let _instance = app::instance::acquire(&dirs.root)?;
    let _log_guard = init_tracing(&dirs)?;
    tracing::info!("athena {} starting", env!("CARGO_PKG_VERSION"));

    let db = Db::open(&dirs.db_path)?;
    let core = AppCore::new(db, dirs)?;

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
        Box::new(move |cc| Ok(Box::new(AthenaApp::new(cc, core)))),
    )
    .map_err(|e| AppError::Ui(e.to_string()))
}

/// The 64x64 RGBA window and taskbar icon, embedded in the binary.
fn window_icon() -> eframe::egui::IconData {
    eframe::egui::IconData {
        rgba: include_bytes!("../assets/icon-64.rgba").to_vec(),
        width: 64,
        height: 64,
    }
}
