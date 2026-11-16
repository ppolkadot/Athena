//! File-system and export workflows invoked by the UI.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::application::services::export;
use crate::domain::export::{default_export_name, default_json_name};
use crate::i18n::{PluralKey, plural, strings};
use crate::ui::screens::ExportRequest;
use crate::ui::screens::settings;

use super::AppCore;

/// Show `folder` in the system file manager. A failure is logged and otherwise ignored.
pub(crate) fn open_folder(folder: &Path) {
    let (program, args) = settings::folder_command(folder);
    if let Err(err) = std::process::Command::new(program).args(args).spawn() {
        tracing::warn!("could not open the data folder: {err}");
    }
}

/// Ask where to save an export, suggesting the standard file name in the exports folder.
/// `None` when the user cancels the dialog.
pub(crate) fn pick_export_path(
    core: &AppCore,
    request: ExportRequest,
    today: NaiveDate,
) -> Option<PathBuf> {
    let (name, filter_name, extensions): (String, &str, &[&str]) = match request {
        ExportRequest::Json => (default_json_name(today), "JSON", &["json"]),
        ExportRequest::Csv(scope) => (default_export_name(scope), "CSV", &["csv"]),
    };
    rfd::FileDialog::new()
        .set_file_name(name)
        .set_directory(&core.store.dirs().exports_dir)
        .add_filter(filter_name, extensions)
        .save_file()
}

/// Ask which backup file to restore, starting in the backups folder. `None` when cancelled.
pub(crate) fn pick_restore_path(core: &AppCore) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_directory(&core.store.dirs().backups_dir)
        .add_filter("SQLite", &["db"])
        .pick_file()
}

/// Ask which JSON export file to import, starting in the exports folder. `None` when cancelled.
pub(crate) fn pick_import_path(core: &AppCore) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_directory(&core.store.dirs().exports_dir)
        .add_filter("JSON", &["json"])
        .pick_file()
}

pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Run an export to `path` and return the toast text: where it went and how much, or that it
/// failed (in which case any existing file at `path` is untouched).
pub fn run_export(
    core: &AppCore,
    request: ExportRequest,
    path: &Path,
    exported_at: &str,
) -> String {
    let language = core.vm.language();
    let s = strings(language);
    let conn = core.store.db().conn();
    let result = match request {
        ExportRequest::Json => match core.vm.profile.as_ref() {
            Some(profile) => export::export_json(conn, profile, exported_at, path),
            None => Err(crate::shared::error::AppError::Export(
                "no profile yet".into(),
            )),
        },
        ExportRequest::Csv(scope) => export::export_csv(conn, language, scope, path),
    };
    match result {
        Ok(receipt) => s
            .toast_exported
            .replace(
                "{count}",
                &plural(PluralKey::Entries, receipt.record_count, language),
            )
            .replace("{path}", &receipt.path.display().to_string()),
        Err(err) => {
            tracing::warn!("export failed: {err}");
            s.toast_export_failed.to_string()
        }
    }
}
