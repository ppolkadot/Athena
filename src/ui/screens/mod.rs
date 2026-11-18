//! Screen dispatch. Each screen module replaces its placeholder as its task lands.

use eframe::egui::{self, RichText};

use crate::app::actions::AppAction;
use crate::app::navigation::Screen;
use crate::app::ui_state::UiState;
use crate::app::view_model::ViewModel;
use crate::i18n::strings;
use crate::ui::components::settlement_card::SettlementEvent;
use crate::ui::theme::{palette, typography};

pub mod calendar;
pub mod month;
pub mod onboarding;
pub mod recovery;
pub mod settings;
pub mod trash;

/// A file export the user asked for. The app picks the destination and runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportRequest {
    /// The complete profile.
    Json,
    Csv(crate::domain::export::ExportScope),
}

/// Something a screen needs the app to do, beyond editing `UiState`.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenEvent {
    NewTransaction,
    EditTransaction(i64),
    DeleteTransaction(i64),
    /// Open the template editor (Quick Add form without a date).
    EditTemplate(i64),
    /// Ask to delete a template, by name.
    DeleteTemplate(i64),
    PurgeTransaction(i64),
    EmptyTrash,
    /// Open the month register for this month.
    OpenMonth(crate::domain::date::YearMonth),
    /// Apply this action; the screen that raised it shows the result.
    Apply(AppAction),
    Settlement(SettlementEvent),
    OpenDataFolder,
    Export(ExportRequest),
    RequestRestoreListed(crate::infrastructure::db::backup::BackupFile),
    /// Save a manual backup; failures stay on the Data tab so drafts are not dropped.
    CreateBackup,
    /// Delete one manual backup after the user confirms.
    DeleteBackup(std::path::PathBuf),
    PickRestoreFile,
    PickImportFile,
    RetryRefresh,
}

pub fn show(ui: &mut egui::Ui, vm: &ViewModel, state: &mut UiState) -> Vec<ScreenEvent> {
    match state.screen {
        Screen::Month => month::show(ui, vm, state)
            .map(|event| match event {
                month::MonthEvent::NewTransaction => ScreenEvent::NewTransaction,
                month::MonthEvent::Edit(id) => ScreenEvent::EditTransaction(id),
                month::MonthEvent::Delete(id) => ScreenEvent::DeleteTransaction(id),
                month::MonthEvent::Settlement(ev) => ScreenEvent::Settlement(ev),
            })
            .into_iter()
            .collect(),
        Screen::Calendar => calendar::show(ui, vm, state)
            .map(|event| match event {
                calendar::CalendarEvent::OpenMonth(month) => ScreenEvent::OpenMonth(month),
            })
            .into_iter()
            .collect(),
        Screen::Trash => trash::show(ui, vm, state),
        Screen::Settings => settings::show(ui, vm, state),
    }
}

/// Title-only placeholder, kept for screens that do not exist yet.
#[allow(dead_code)]
fn placeholder(ui: &mut egui::Ui, vm: &ViewModel, state: &UiState) {
    let s = strings(vm.language());
    let p = palette(vm.theme());
    ui.label(RichText::new(state.screen.title(vm.language())).text_style(typography::display()));
    ui.add_space(8.0);
    ui.label(RichText::new(s.coming_soon).color(p.muted));
}
