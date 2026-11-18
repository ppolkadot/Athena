//! Data folder layout and logging setup.

use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::WorkerGuard;

use crate::shared::error::{AppError, Result};

/// Marker file next to the executable that switches Athena to portable mode.
pub const PORTABLE_MARKER: &str = "athena.portable";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    pub root: PathBuf,
    pub db_path: PathBuf,
    pub log_dir: PathBuf,
    pub exports_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub staging_dir: PathBuf,
}

impl AppDirs {
    #[must_use]
    pub fn from_root(root: PathBuf) -> Self {
        Self {
            db_path: root.join("athena.db"),
            log_dir: root.join("logs"),
            exports_dir: root.join("exports"),
            backups_dir: root.join("backups"),
            staging_dir: root.join("staging"),
            root,
        }
    }

    /// Portable mode keeps data in `<exe dir>/data`; otherwise `<appdata>/Athena`.
    #[must_use]
    pub fn resolve_with(exe_dir: &Path, appdata: &Path) -> Self {
        if exe_dir.join(PORTABLE_MARKER).exists() {
            Self::from_root(exe_dir.join("data"))
        } else {
            Self::from_root(appdata.join("Athena"))
        }
    }

    /// # Errors
    /// Returns [`AppError::Dirs`] when the executable or app-data location is unknown.
    pub fn resolve() -> Result<Self> {
        let exe = std::env::current_exe().map_err(|e| AppError::Dirs(e.to_string()))?;
        let exe_dir = exe
            .parent()
            .ok_or_else(|| AppError::Dirs("executable has no parent folder".into()))?;
        let base = directories::BaseDirs::new()
            .ok_or_else(|| AppError::Dirs("no home folder available".into()))?;
        Ok(Self::resolve_with(exe_dir, base.data_dir()))
    }

    /// Create the root and log folders required to take `athena.lock` and initialize logging.
    ///
    /// # Errors
    /// Returns [`AppError::Io`] when an essential folder cannot be created.
    pub fn ensure_essential(&self) -> Result<()> {
        for dir in [&self.root, &self.log_dir] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// Create the secondary data folders (`exports`, `backups`, `staging`) after recovery probing.
    ///
    /// # Errors
    /// Returns [`AppError::Io`] when a recoverable folder cannot be created.
    pub fn ensure_recoverable(&self) -> Result<()> {
        for dir in [&self.exports_dir, &self.backups_dir, &self.staging_dir] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// Create every data folder.
    ///
    /// # Errors
    /// Returns [`AppError::Io`] when a folder cannot be created.
    pub fn ensure(&self) -> Result<()> {
        self.ensure_essential()?;
        self.ensure_recoverable()
    }
}

/// Daily rolling log files. Keep the returned guard alive for the process lifetime.
/// Transaction text is never logged anywhere in the app.
///
/// # Errors
/// Returns [`AppError::Ui`] when a global subscriber is already installed.
pub fn init_tracing(dirs: &AppDirs) -> Result<WorkerGuard> {
    let appender = tracing_appender::rolling::daily(&dirs.log_dir, "athena.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .map_err(|e| AppError::Ui(e.to_string()))?;
    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_dirs_are_under_root() {
        let dirs = AppDirs::from_root(PathBuf::from(r"C:\tmp\Athena"));
        assert_eq!(dirs.db_path, PathBuf::from(r"C:\tmp\Athena\athena.db"));
        assert_eq!(dirs.log_dir, PathBuf::from(r"C:\tmp\Athena\logs"));
        assert_eq!(dirs.exports_dir, PathBuf::from(r"C:\tmp\Athena\exports"));
        assert_eq!(dirs.backups_dir, PathBuf::from(r"C:\tmp\Athena\backups"));
        assert_eq!(dirs.staging_dir, PathBuf::from(r"C:\tmp\Athena\staging"));
    }

    #[test]
    fn portable_marker_switches_data_dir() {
        let exe = tempfile::tempdir().unwrap();
        let appdata = tempfile::tempdir().unwrap();
        assert_eq!(
            AppDirs::resolve_with(exe.path(), appdata.path()).root,
            appdata.path().join("Athena")
        );
        std::fs::write(exe.path().join(PORTABLE_MARKER), b"").unwrap();
        assert_eq!(
            AppDirs::resolve_with(exe.path(), appdata.path()).root,
            exe.path().join("data")
        );
    }

    #[test]
    fn ensure_creates_every_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(tmp.path().join("Athena"));
        dirs.ensure().unwrap();
        assert!(
            dirs.log_dir.is_dir()
                && dirs.exports_dir.is_dir()
                && dirs.backups_dir.is_dir()
                && dirs.staging_dir.is_dir()
        );
    }
}
