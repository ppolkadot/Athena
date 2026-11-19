pub mod actions;
pub mod appearance_edit;
pub mod clock;
pub mod files;
pub mod instance;
pub mod modals;
pub mod navigation;
pub mod recovery;
pub mod store;
pub mod ui_state;
pub mod view_model;

#[cfg(test)]
mod appearance_flow_tests;
#[cfg(test)]
mod appearance_platform_tests;
#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod day_flow_tests;
#[cfg(test)]
mod goal_flow_tests;
#[cfg(test)]
mod quick_flow_tests;
#[cfg(test)]
mod wallpaper_tests;

use eframe::egui::{self, Key, Modifiers};

use self::appearance_edit::{AppearanceEdit, PreviewStatus};
use crate::application::services::onboarding::OnboardingResult;
use crate::domain::appearance::{AppearancePrefs, Wallpaper};
use crate::domain::category::Category;
use crate::domain::date::YearMonth;
use crate::domain::goal::{GoalStatus, GoalTransition};
use crate::domain::person::Person;
use crate::domain::settings::{Language, Profile, Theme};
use crate::domain::transaction::TxDraft;
use crate::i18n::{
    ToastKey, backup_error_message, format_import_errors, goal_error_message, strings,
};
use crate::infrastructure::config::AppDirs;
use crate::infrastructure::db::Db;
use crate::infrastructure::media::MediaError;
use crate::infrastructure::media::assets::{MediaStore, StagedMedia};
use crate::infrastructure::platform::{
    self, PlatformPreferences, PlatformPrefs, PlatformWatch, WindowState,
};
use crate::shared::error::{AppError, Result};
use crate::ui::appearance::background::Backdrop;
use crate::ui::appearance::{WallpaperRuntime, Wanted};
use crate::ui::components::confirm_dialog::{self, ConfirmChoice, ConfirmDialog};
use crate::ui::components::discard_prompt::{self, DiscardChoice};
use crate::ui::components::import_preview::{self, ImportPreviewChoice, ImportPreviewDialog};
use crate::ui::components::settlement_card::SettlementEvent;
use crate::ui::components::toast::{self, Toast, ToastAction};
use crate::ui::goal_form::{self, GoalFormEvent, GoalFormState};
use crate::ui::quick_add::{self, QuickAddState};
use crate::ui::screens::ScreenEvent;
use crate::ui::screens::goals::GoalEvent;
use crate::ui::screens::onboarding::{self, OnboardingIssue, OnboardingState};
use crate::ui::screens::settings::SettingsPage;
use crate::ui::theme::active::StyleKey;
use crate::ui::theme::effective::{self, EffectiveAppearance};
use crate::ui::{shell, theme};

use self::actions::{ActionOutcome, AppAction, RestoreSource};
pub use self::files::run_export;
use self::files::{
    now_rfc3339, open_folder, pick_export_path, pick_import_path, pick_restore_path,
};
pub use self::modals::{
    ModalAfter, apply_confirmed, apply_confirmed_outcome, apply_settings_action,
    archive_goal_confirmation, begin_edit, begin_edit_template, complete_goal_confirmation,
    delete_backup_confirmation, delete_confirmation, delete_contribution_confirmation,
    delete_template_confirmation, empty_trash_confirmation, handle_quick_add,
    import_preview_dialog, purge_confirmation, restore_confirmation,
};
use self::navigation::Screen;
use self::store::Store;
use self::ui_state::{GoalFocus, UiState};
use self::view_model::ViewModel;

/// What the user was doing when the Keep editing / Discard prompt for Appearance opened. Discard
/// performs it once.
#[derive(Debug, Clone, PartialEq)]
enum AppearanceIntent {
    Navigate(Screen),
    SettingsPage(SettingsPage),
    Event(ScreenEvent),
}

/// Thin composition root. It owns the three state parts and holds no logic: every method builds an
/// [`AppAction`] (or sets UI state), applies it through the store, and refreshes the view model.
#[derive(Debug)]
pub struct AppCore {
    pub store: Store,
    pub vm: ViewModel,
    pub ui: UiState,
    /// Bumped by restore, import and Reset so the wallpaper rebinds even if preferences compare
    /// equal.
    pub media_epoch: u64,
}

impl AppCore {
    /// Open the app on a migrated database: load the view model and run the once-per-launch
    /// startup tasks (daily snapshot, debug integrity check).
    ///
    /// # Errors
    /// Database errors loading the first view.
    pub fn new(db: Db, dirs: AppDirs) -> Result<Self> {
        Self::open(db, dirs, true)
    }

    fn open(db: Db, dirs: AppDirs, run_startup_tasks: bool) -> Result<Self> {
        let mut core = Self {
            store: Store::new(db, dirs),
            vm: ViewModel::new(Language::from_system()),
            ui: UiState::default(),
            media_epoch: 0,
        };
        if run_startup_tasks {
            core.store
                .run_startup_tasks(chrono::Local::now().date_naive(), chrono::Utc::now());
        }
        core.sync()?;
        Ok(core)
    }

    /// An in-memory app for tests. Skips the startup tasks and keeps its data folder out of the
    /// way of any real one.
    ///
    /// # Panics
    /// If the first view cannot be loaded from the given database.
    #[cfg(test)]
    #[must_use]
    pub fn empty_for_test(db: Db) -> Self {
        Self::empty_for_test_in(db, std::env::temp_dir().join("athena-core-test"))
    }

    /// Like [`Self::empty_for_test`] with its own data folder, for tests that write files.
    ///
    /// # Panics
    /// If the first view cannot be loaded from the given database.
    #[cfg(test)]
    #[must_use]
    pub fn empty_for_test_in(db: Db, root: std::path::PathBuf) -> Self {
        let dirs = AppDirs::from_root(root);
        let mut core = Self {
            store: Store::new(db, dirs),
            vm: ViewModel::new(Language::En),
            ui: UiState::new(YearMonth::new(2026, 11).unwrap()),
            media_epoch: 0,
        };
        core.vm.today = crate::shared::test_fixtures::today();
        core.sync().unwrap();
        core
    }

    /// Reload the view model if the store or the selection changed.
    ///
    /// # Errors
    /// Database errors.
    pub fn sync(&mut self) -> Result<bool> {
        let mut reloaded = self.vm.sync(&self.store, &self.ui)?;
        // A goal that vanished or left the open tab clears the selection; its detail follows.
        if self.ui.goals.reconcile(&self.vm.goals) {
            reloaded |= self.vm.sync(&self.store, &self.ui)?;
        }
        Ok(reloaded)
    }

    /// Apply an action and reload what it affected.
    ///
    /// # Errors
    /// Whatever the action fails with; nothing changes in that case.
    pub fn apply(&mut self, action: AppAction) -> Result<ActionOutcome> {
        let reset_ui = matches!(
            action,
            AppAction::RestoreBackup(_) | AppAction::ImportProfile(_)
        );
        let is_reset = matches!(action, AppAction::ResetAppearance);
        let outcome = match self.store.apply(action) {
            Ok(outcome) => outcome,
            Err(err) => {
                if reset_ui
                    && self.ui.screen == Screen::Settings
                    && let Err(list_err) = self.vm.refresh_backups(&self.store)
                {
                    tracing::warn!("could not refresh backups after failure: {list_err}");
                }
                return Err(err);
            }
        };
        self.record_outcome_notice(&outcome);
        if reset_ui || is_reset {
            self.media_epoch += 1;
        }
        if reset_ui {
            self.ui.clear_filters();
            self.ui.clear_day_view();
            self.ui.set_screen(Screen::Month);
            self.ui.settings.reset_all();
            self.ui.goals.reset();
        }
        if let Some(id) = outcome.created_goal {
            self.ui.goals.set_filter(GoalStatus::Active);
            self.ui.goals.select(id);
        }
        if let Err(err) = self.vm.reload(&self.store, &self.ui, outcome.refresh) {
            tracing::error!("could not refresh the view after applying action: {err}");
        }
        if self.ui.goals.reconcile(&self.vm.goals)
            && let Err(err) = self.vm.sync(&self.store, &self.ui)
        {
            tracing::error!("could not refresh the goal detail: {err}");
        }
        Ok(outcome)
    }

    fn record_outcome_notice(&mut self, outcome: &ActionOutcome) {
        if let Some(notice) = &outcome.notice {
            self.vm.last_operation = Some(notice.clone());
        }
    }

    /// # Errors
    /// Database errors.
    pub fn set_month(&mut self, month: YearMonth) -> Result<()> {
        self.ui.set_month(month);
        self.sync().map(|_| ())
    }

    /// # Errors
    /// Database errors.
    pub fn set_calendar_year(&mut self, year: i32) -> Result<()> {
        self.ui.set_calendar_year(year);
        self.sync().map(|_| ())
    }

    pub fn set_screen(&mut self, screen: Screen) {
        self.ui.set_screen(screen);
    }

    /// Show the register for `month`: select it and switch to the Month screen.
    ///
    /// # Errors
    /// Database errors loading the month.
    pub fn open_month(&mut self, month: YearMonth) -> Result<()> {
        self.set_screen(Screen::Month);
        self.set_month(month)
    }

    #[must_use]
    pub fn owner(&self) -> Option<&Person> {
        self.vm.owner()
    }

    /// # Errors
    /// See [`AppCore::apply`].
    pub fn complete_onboarding(&mut self, result: OnboardingResult) -> Result<ActionOutcome> {
        self.apply(AppAction::CompleteOnboarding(result))
    }

    /// # Errors
    /// See [`AppCore::apply`].
    pub fn create_transaction(&mut self, draft: TxDraft) -> Result<ActionOutcome> {
        self.apply(AppAction::SaveTransaction(draft))
    }

    /// # Errors
    /// See [`AppCore::apply`].
    pub fn update_transaction(&mut self, id: i64, draft: TxDraft) -> Result<ActionOutcome> {
        self.apply(AppAction::UpdateTransaction(id, draft))
    }

    /// # Errors
    /// See [`AppCore::apply`].
    pub fn delete_transaction(&mut self, id: i64) -> Result<ActionOutcome> {
        self.apply(AppAction::DeleteTransaction(id))
    }

    /// # Errors
    /// See [`AppCore::apply`].
    pub fn save_profile(&mut self, profile: Profile) -> Result<ActionOutcome> {
        self.apply(AppAction::SaveProfile(profile))
    }

    /// # Errors
    /// See [`AppCore::apply`].
    pub fn save_person(&mut self, person: Person) -> Result<ActionOutcome> {
        self.apply(AppAction::SavePerson(person))
    }

    /// # Errors
    /// See [`AppCore::apply`]; the owner cannot be archived.
    pub fn archive_person(&mut self, id: i64) -> Result<ActionOutcome> {
        self.apply(AppAction::ArchivePerson(id))
    }

    /// # Errors
    /// See [`AppCore::apply`].
    pub fn save_category(&mut self, category: Category) -> Result<ActionOutcome> {
        self.apply(AppAction::SaveCategory(category))
    }

    /// # Errors
    /// See [`AppCore::apply`].
    pub fn archive_category(&mut self, id: i64) -> Result<ActionOutcome> {
        self.apply(AppAction::ArchiveCategory(id))
    }
}

pub struct AthenaApp {
    core: AppCore,
    /// OS preferences (animation, high contrast, transparency), refreshed on focus and a slow poll.
    platform: PlatformWatch,
    /// The styling last pushed to egui, so unchanged frames restyle nothing.
    applied_style: Option<StyleKey>,
    /// The resolved look for the current frame.
    look: EffectiveAppearance,
    /// Unapplied appearance edits shown live in the shell; `None` outside the Appearance page.
    appearance_preview: Option<AppearancePrefs>,
    /// The picked-but-unapplied wallpaper bytes behind a preview of a local file.
    preview_staged: Option<StagedMedia>,
    /// Decode worker, playback clock and textures; created when a wallpaper is first wanted.
    wallpaper: Option<WallpaperRuntime>,
    /// The `AppCore::media_epoch` the wallpaper was last bound at.
    seen_media_epoch: u64,
    /// Why the shown wallpaper is unavailable, if it is (the canvas falls back to solid).
    media_failure: Option<MediaError>,
    /// The failure already announced with a toast, so it is announced once.
    announced_media_failure: Option<MediaError>,
    /// Tests pin the monotonic clock the wallpaper playback reads.
    pinned_instant: Option<std::time::Instant>,
    /// A leave or import/restore that is waiting on the Appearance discard prompt.
    appearance_intent: Option<AppearanceIntent>,
    /// Move focus to Keep editing on the first frame of the prompt.
    appearance_prompt_focus: bool,
    /// Tests answer the wallpaper file dialog from here.
    #[cfg(test)]
    test_wallpaper_picks: Vec<Option<std::path::PathBuf>>,
    onboarding: OnboardingState,
    quick_add: Option<QuickAddState>,
    /// The goal or contribution form, when open.
    goal_form: Option<GoalFormState>,
    confirm: Option<ConfirmDialog>,
    import_preview: Option<ImportPreviewDialog>,
    toast: Option<Toast>,
    /// Widget that opened the active modal, so Esc/Cancel can restore focus to it.
    modal_opener: Option<egui::Id>,
    /// Opener id deferred until the first non-modal frame so background widgets allow focus.
    pending_focus_restore: Option<egui::Id>,
    /// Tests pin the date so they do not depend on when they run.
    pinned_today: Option<chrono::NaiveDate>,
    /// The user confirmed discarding a dirty Quick Add to quit; the next close request passes.
    close_confirmed: bool,
}

impl AthenaApp {
    #[must_use]
    pub fn new(cc: &eframe::CreationContext<'_>, core: AppCore) -> Self {
        Self::from_core(&cc.egui_ctx, core)
    }

    /// Build the window on an already-open core. Used by `new` and by the recovery session once a
    /// restore has produced a working database.
    #[must_use]
    pub fn from_core(ctx: &egui::Context, core: AppCore) -> Self {
        theme::install_fonts(ctx);
        let platform = PlatformWatch::new(platform::system_provider(), std::time::Instant::now());
        Self::with_platform(ctx, core, platform)
    }

    fn with_platform(ctx: &egui::Context, core: AppCore, platform: PlatformWatch) -> Self {
        let look = Self::resolve_look(&core, None, platform.current(), None);
        theme::apply_effective(ctx, &look);
        let onboarding = OnboardingState::new(core.vm.language());
        Self {
            core,
            platform,
            applied_style: Some(look.style_key()),
            look,
            appearance_preview: None,
            preview_staged: None,
            wallpaper: None,
            seen_media_epoch: 0,
            media_failure: None,
            announced_media_failure: None,
            pinned_instant: None,
            appearance_intent: None,
            appearance_prompt_focus: false,
            #[cfg(test)]
            test_wallpaper_picks: Vec::new(),
            onboarding,
            quick_add: None,
            goal_form: None,
            confirm: None,
            import_preview: None,
            toast: None,
            modal_opener: None,
            pending_focus_restore: None,
            pinned_today: None,
            close_confirmed: false,
        }
    }

    /// Build the app on a core with injected platform preferences (tests and the real provider).
    #[must_use]
    pub fn with_provider(
        ctx: &egui::Context,
        core: AppCore,
        provider: Box<dyn PlatformPreferences>,
    ) -> Self {
        theme::install_fonts(ctx);
        let platform = PlatformWatch::new(provider, std::time::Instant::now());
        Self::with_platform(ctx, core, platform)
    }

    /// Applied or previewed preferences, the legacy theme, the OS theme signal and platform state
    /// resolved into one look. Onboarding keeps its solid stock style.
    fn resolve_look(
        core: &AppCore,
        preview: Option<&AppearancePrefs>,
        platform: PlatformPrefs,
        os_theme: Option<Theme>,
    ) -> EffectiveAppearance {
        let legacy = core.vm.profile_theme();
        if !core.vm.onboarding_complete() {
            return effective::resolve_defaults(legacy, PlatformPrefs::default());
        }
        let prefs = preview
            .or(core.vm.appearance.as_ref())
            .cloned()
            .unwrap_or_else(|| AppearancePrefs::defaults_for(legacy));
        effective::resolve(&prefs, legacy, os_theme, platform)
    }

    /// Re-read platform preferences when due, resolve the look and restyle egui only when the
    /// styling key changed. Schedules the next platform check separately from decorative repaints.
    fn refresh_appearance(&mut self, ctx: &egui::Context) -> bool {
        let (focused, minimized) =
            ctx.input(|i| (i.focused, i.viewport().minimized.unwrap_or(false)));
        let (_, wait) = self.platform.poll(
            std::time::Instant::now(),
            WindowState {
                focused,
                visible: !minimized,
            },
        );
        if let Some(wait) = wait {
            ctx.request_repaint_after(wait);
        }
        let os_theme = ctx.system_theme().map(|t| match t {
            egui::Theme::Dark => Theme::Dark,
            egui::Theme::Light => Theme::Light,
        });
        let look = Self::resolve_look(
            &self.core,
            self.appearance_preview.as_ref(),
            self.platform.current(),
            os_theme,
        );
        let key = look.style_key();
        let restyle = self.applied_style != Some(key) || theme::active::get() != Some(key);
        if restyle {
            theme::apply_effective(ctx, &look);
            self.applied_style = Some(key);
        }
        self.core.vm.set_effective_theme(Some(look.theme));
        self.look = look;
        restyle
    }

    /// Follow the applied or previewed wallpaper, advance its clock and return what the shell
    /// paints behind its chrome. Costs nothing until a wallpaper is first wanted.
    fn update_wallpaper(&mut self, ctx: &egui::Context) -> Backdrop {
        let applied = self
            .core
            .vm
            .appearance
            .as_ref()
            .map_or(Wallpaper::None, |a| a.wallpaper.clone());
        let preview = self
            .appearance_preview
            .as_ref()
            .map(|p| p.wallpaper.clone());
        if self.seen_media_epoch != self.core.media_epoch {
            // Reset, import or restore replaced what was applied: an open draft is stale, and the
            // wallpaper rebinds even if the new preferences compare equal to the old.
            self.seen_media_epoch = self.core.media_epoch;
            self.core.ui.settings.appearance = None;
            self.appearance_preview = None;
            self.preview_staged = None;
            if let Some(runtime) = self.wallpaper.as_mut() {
                runtime.invalidate_all();
            }
        }
        let wanted =
            applied != Wallpaper::None || preview.as_ref().is_some_and(|w| *w != Wallpaper::None);
        if !wanted && self.wallpaper.is_none() {
            self.media_failure = None;
            return Backdrop {
                material: self.look.material,
                ..Backdrop::solid()
            };
        }
        let runtime = self
            .wallpaper
            .get_or_insert_with(|| WallpaperRuntime::new(ctx));
        let store = MediaStore::new(self.core.store.dirs());
        let (focused, minimized) =
            ctx.input(|i| (i.focused, i.viewport().minimized.unwrap_or(false)));
        let staged = self.preview_staged.as_ref();
        let (backdrop, wake) = runtime.sync(
            ctx,
            &store,
            Wanted {
                wallpaper: &applied,
                staged: None,
            },
            preview
                .as_ref()
                .map(|wallpaper| Wanted { wallpaper, staged }),
            &self.look,
            (focused, !minimized),
            self.pinned_instant.unwrap_or_else(std::time::Instant::now),
        );
        if let Some(wake) = wake {
            ctx.request_repaint_after(wake);
        }
        self.media_failure = runtime.displayed_failure();
        if self.media_failure != self.announced_media_failure {
            self.announced_media_failure = self.media_failure;
            if let Some(error) = self
                .media_failure
                .filter(|_| self.appearance_preview.is_none())
            {
                let text = crate::i18n::media_error_message(error, self.core.vm.language());
                self.raise_toast_text(ctx, Some(text.to_owned()));
            }
        }
        backdrop
    }

    fn remember_modal_opener(&mut self, ctx: &egui::Context) {
        self.modal_opener = ctx.memory(egui::Memory::focused);
    }

    fn restore_modal_opener(&mut self, ctx: &egui::Context) {
        self.pending_focus_restore = self.modal_opener.take();
        if self.pending_focus_restore.is_some() {
            ctx.request_repaint();
        }
    }

    /// Act on what a screen asked for. Returns `false` when an applied action was refused.
    #[allow(clippy::too_many_lines)]
    fn handle_screen_event(&mut self, ctx: &egui::Context, event: ScreenEvent) -> bool {
        let idle = self.quick_add.is_none()
            && self.goal_form.is_none()
            && self.confirm.is_none()
            && self.import_preview.is_none()
            && self.appearance_intent.is_none();
        if idle
            && matches!(
                event,
                ScreenEvent::PickImportFile
                    | ScreenEvent::RequestRestoreListed(_)
                    | ScreenEvent::PickRestoreFile
            )
            && !self.guard_appearance(ctx, AppearanceIntent::Event(event.clone()))
        {
            return true;
        }
        match event {
            ScreenEvent::PickWallpaper if idle => self.pick_wallpaper(),
            ScreenEvent::ApplyAppearance if idle => self.apply_appearance(ctx),
            ScreenEvent::ResetAppearance if idle => {
                let s = strings(self.core.vm.language());
                self.remember_modal_opener(ctx);
                self.confirm = Some(ConfirmDialog {
                    title: s.ap_reset_title.to_owned(),
                    body: s.ap_reset_body.to_owned(),
                    confirm_label: s.ap_reset_confirm.to_owned(),
                    danger: false,
                    action: AppAction::ResetAppearance,
                });
            }
            ScreenEvent::Goal(event) if idle => self.handle_goal_event(ctx, event),
            ScreenEvent::NewTransaction if self.quick_add.is_none() => {
                self.remember_modal_opener(ctx);
                self.open_quick_add();
            }
            ScreenEvent::EditTransaction(id) if idle => match begin_edit(&self.core, id) {
                Ok(state) => {
                    self.remember_modal_opener(ctx);
                    self.quick_add = Some(state);
                }
                Err(err) => tracing::warn!("could not open the transaction: {err}"),
            },
            ScreenEvent::EditTemplate(id) if idle => match begin_edit_template(&self.core, id) {
                Ok(state) => {
                    self.remember_modal_opener(ctx);
                    self.quick_add = Some(state);
                }
                Err(err) => tracing::warn!("could not open the template: {err}"),
            },
            ScreenEvent::DeleteTemplate(id) if idle => {
                match delete_template_confirmation(&self.core, id) {
                    Ok(dialog) => {
                        self.remember_modal_opener(ctx);
                        self.confirm = Some(dialog);
                    }
                    Err(err) => tracing::warn!("could not open the template: {err}"),
                }
            }
            ScreenEvent::DeleteTransaction(id) if idle => match delete_confirmation(&self.core, id)
            {
                Ok(dialog) => {
                    self.remember_modal_opener(ctx);
                    self.confirm = Some(dialog);
                }
                Err(err) => tracing::warn!("could not open the transaction: {err}"),
            },
            ScreenEvent::PurgeTransaction(id) if idle => match purge_confirmation(&self.core, id) {
                Ok(dialog) => {
                    self.remember_modal_opener(ctx);
                    self.confirm = Some(dialog);
                }
                Err(err) => tracing::warn!("could not open the trashed transaction: {err}"),
            },
            ScreenEvent::EmptyTrash if idle => {
                self.remember_modal_opener(ctx);
                self.confirm = Some(empty_trash_confirmation(&self.core));
            }
            ScreenEvent::OpenMonth(month) => {
                if let Err(err) = self.core.open_month(month) {
                    tracing::error!("could not open the month: {err}");
                }
            }
            ScreenEvent::Apply(action) => {
                let (text, ok) = apply_settings_action(&mut self.core, action);
                self.raise_toast_text(ctx, text);
                return ok;
            }
            ScreenEvent::CreateBackup => return self.handle_create_backup(ctx),
            ScreenEvent::DeleteBackup(path) if idle => {
                self.remember_modal_opener(ctx);
                self.confirm = Some(delete_backup_confirmation(&self.core, &path));
            }
            ScreenEvent::Settlement(event) => {
                return self.handle_settlement_event(ctx, event, idle);
            }
            ScreenEvent::OpenDataFolder => open_folder(&self.core.store.dirs().root),
            ScreenEvent::Export(request) => {
                if let Some(path) = pick_export_path(&self.core, request, self.today()) {
                    let text = run_export(&self.core, request, &path, &now_rfc3339());
                    self.raise_toast_text(ctx, Some(text));
                }
            }
            ScreenEvent::RequestRestoreListed(file) if idle => {
                self.remember_modal_opener(ctx);
                self.confirm = Some(restore_confirmation(
                    &self.core,
                    RestoreSource::Listed(file.path),
                    Some(file.created_utc),
                ));
            }
            ScreenEvent::PickRestoreFile if idle => {
                if let Some(path) = pick_restore_path(&self.core) {
                    let modified_utc = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .map(chrono::DateTime::<chrono::Utc>::from);
                    self.remember_modal_opener(ctx);
                    self.confirm = Some(restore_confirmation(
                        &self.core,
                        RestoreSource::External(path),
                        modified_utc,
                    ));
                }
            }
            ScreenEvent::PickImportFile if idle => {
                self.core.ui.settings.import_error = None;
                if let Some(path) = pick_import_path(&self.core) {
                    self.open_import_preview(ctx, &path);
                }
            }
            ScreenEvent::RetryRefresh => {
                self.core.vm.clear_refresh_failure();
                if let Err(err) = self.core.sync() {
                    tracing::error!("could not refresh the view: {err}");
                }
            }
            _ => {}
        }
        true
    }

    /// Save a manual backup from the Data tab. Unlike the other settings actions this keeps the
    /// screen's drafts: a failed backup is not a refused edit, so it shows inline and as a toast
    /// instead of clearing the form.
    fn handle_create_backup(&mut self, ctx: &egui::Context) -> bool {
        let language = self.core.vm.language();
        match self.core.apply(AppAction::CreateBackup) {
            Ok(outcome) => {
                self.core.ui.settings.backup_error = None;
                let text = outcome.toast.map(|key| key.text(language).to_string());
                self.raise_toast_text(ctx, text);
                true
            }
            Err(err) => {
                tracing::warn!("could not save a backup: {err}");
                let message = backup_error_message(&err, language);
                self.core.ui.settings.backup_error = Some(message);
                self.raise_toast_text(ctx, Some(ToastKey::BackupFailed.text(language).to_string()));
                false
            }
        }
    }

    pub(crate) fn open_import_preview(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        self.core.ui.settings.import_error = None;
        match crate::application::services::import::read_and_validate(path) {
            Ok(plan) => {
                self.remember_modal_opener(ctx);
                self.import_preview = Some(import_preview_dialog(&self.core, plan));
            }
            Err(AppError::Import(errs)) => {
                let message = format_import_errors(&errs, self.core.vm.language());
                self.core.ui.settings.import_error = Some(message.clone());
                self.raise_toast_text(ctx, Some(message));
            }
            Err(err) => {
                let message = err.to_string();
                self.core.ui.settings.import_error = Some(message.clone());
                self.raise_toast_text(ctx, Some(message));
            }
        }
    }

    fn handle_settlement_event(
        &mut self,
        ctx: &egui::Context,
        event: SettlementEvent,
        idle: bool,
    ) -> bool {
        match event {
            SettlementEvent::Record { from, to, amount } if idle => {
                let from_name = self.core.vm.person(from).map_or("", |p| p.name.as_str());
                let to_name = self.core.vm.person(to).map_or("", |p| p.name.as_str());
                let title = format!("{from_name} → {to_name}");
                let people: Vec<_> = self.core.vm.active_people().into_iter().cloned().collect();
                let today = self.today();
                self.remember_modal_opener(ctx);
                self.quick_add = Some(
                    QuickAddState::for_settlement(
                        &people,
                        from,
                        to,
                        amount,
                        today,
                        title,
                        self.core.vm.language(),
                    )
                    .with_viewed_month_warning(self.core.ui.month, today),
                );
            }
            SettlementEvent::RestorePerson(id) => {
                match self.core.apply(AppAction::RestorePerson(id)) {
                    Ok(outcome) => {
                        let text = outcome
                            .toast
                            .map(|key| key.text(self.core.vm.language()).to_string());
                        self.raise_toast_text(ctx, text);
                    }
                    Err(err) => {
                        tracing::warn!("could not restore person: {err}");
                        let language = self.core.vm.language();
                        let s = strings(language);
                        let text = match (err, self.core.vm.person(id)) {
                            (crate::shared::error::AppError::Validation(_), Some(person)) => s
                                .settlement_restore_name_taken
                                .replace("{name}", &person.name),
                            _ => s.settings_save_failed.to_string(),
                        };
                        self.raise_toast_text(ctx, Some(text));
                        return false;
                    }
                }
            }
            SettlementEvent::FilterIssue { issue, as_of } => {
                self.core.ui.set_repair_filter(issue, as_of);
            }
            SettlementEvent::Record { .. } => {}
        }
        true
    }

    /// An app around `core` for tests, with the date pinned to [`crate::shared::test_fixtures::today`].
    #[cfg(test)]
    pub(crate) fn for_test(mut core: AppCore) -> Self {
        core.store
            .freeze_today(crate::shared::test_fixtures::today());
        Self {
            platform: PlatformWatch::new(
                Box::new(platform::UnknownPlatform),
                std::time::Instant::now(),
            ),
            applied_style: None,
            look: effective::resolve_defaults(core.vm.profile_theme(), PlatformPrefs::default()),
            appearance_preview: None,
            preview_staged: None,
            wallpaper: None,
            seen_media_epoch: 0,
            media_failure: None,
            announced_media_failure: None,
            pinned_instant: None,
            appearance_intent: None,
            appearance_prompt_focus: false,
            test_wallpaper_picks: Vec::new(),
            onboarding: OnboardingState::new(core.vm.language()),
            core,
            quick_add: None,
            goal_form: None,
            confirm: None,
            import_preview: None,
            toast: None,
            modal_opener: None,
            pending_focus_restore: None,
            pinned_today: Some(crate::shared::test_fixtures::today()),
            close_confirmed: false,
        }
    }

    /// Like [`Self::for_test`] with an injected platform-preference provider.
    #[cfg(test)]
    pub(crate) fn for_test_with_platform(
        core: AppCore,
        provider: Box<dyn PlatformPreferences>,
    ) -> Self {
        let mut app = Self::for_test(core);
        app.platform = PlatformWatch::new(provider, std::time::Instant::now());
        app
    }

    /// Whether the wallpaper shown has finished loading (or failed), so a snapshot is stable.
    #[cfg(test)]
    pub(crate) fn wallpaper_settled_for_test(&self) -> bool {
        let wanted = self.look.wallpaper != Wallpaper::None;
        match (&self.wallpaper, wanted) {
            (_, false) => true,
            (None, true) => false,
            (Some(rt), true) => {
                !rt.displayed_loading()
                    && (rt.is_ready(false) || rt.is_ready(true) || rt.displayed_failure().is_some())
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn pin_instant(&mut self, now: std::time::Instant) {
        self.pinned_instant = Some(now);
    }

    #[cfg(test)]
    pub(crate) fn pin_today(&mut self, today: chrono::NaiveDate) {
        self.core.store.freeze_today(today);
        self.pinned_today = Some(today);
    }

    #[cfg(test)]
    pub(crate) fn set_onboarding_for_test(&mut self, state: OnboardingState) {
        self.onboarding = state;
    }

    #[cfg(test)]
    pub(crate) fn theme_for_test(&self) -> Theme {
        self.core.vm.theme()
    }

    #[cfg(test)]
    pub(crate) fn show_for_test(&mut self, ui: &mut egui::Ui) {
        self.show(ui);
    }

    fn today(&self) -> chrono::NaiveDate {
        self.pinned_today
            .unwrap_or_else(|| chrono::Local::now().date_naive())
    }

    /// Wake the app at the next local midnight (or sooner, so a changed system clock is noticed)
    /// to re-read the date. Independent of wallpaper playback and platform preference polls.
    fn schedule_date_check(&self, ctx: &egui::Context) {
        if self.pinned_today.is_none() {
            ctx.request_repaint_after(clock::next_date_check(&chrono::Local::now()));
        }
    }

    /// Open a goal form, confirmation or lifecycle step the Goals screen asked for.
    fn handle_goal_event(&mut self, ctx: &egui::Context, event: GoalEvent) {
        let language = self.core.vm.language();
        match event {
            GoalEvent::New => {
                self.remember_modal_opener(ctx);
                self.goal_form = Some(GoalFormState::new_goal(language));
            }
            GoalEvent::Edit(id) => {
                if let Some(item) = self.core.vm.goals.iter().find(|g| g.goal.id == id) {
                    let form = GoalFormState::edit_goal(&item.goal, language);
                    self.remember_modal_opener(ctx);
                    self.goal_form = Some(form);
                }
            }
            GoalEvent::AddContribution(id) => {
                self.remember_modal_opener(ctx);
                self.goal_form = Some(GoalFormState::contribution(id, self.today(), language));
            }
            GoalEvent::DeleteContribution {
                goal_id,
                contribution_id,
            } => {
                if let Some(dialog) =
                    delete_contribution_confirmation(&self.core, goal_id, contribution_id)
                {
                    self.remember_modal_opener(ctx);
                    self.confirm = Some(dialog);
                }
            }
            GoalEvent::Transition(id, transition) => {
                let dialog = match transition {
                    GoalTransition::Complete => complete_goal_confirmation(&self.core, id),
                    GoalTransition::Archive => archive_goal_confirmation(&self.core, id),
                    _ => None,
                };
                if let Some(dialog) = dialog {
                    self.remember_modal_opener(ctx);
                    self.confirm = Some(dialog);
                } else {
                    match self.core.apply(AppAction::ChangeGoalStatus(id, transition)) {
                        Ok(outcome) => self.raise_toast(ctx, outcome.toast),
                        Err(err) => {
                            tracing::warn!("could not change the goal: {err}");
                            let text = match err {
                                AppError::GoalInvalid(rules) => {
                                    rules.first().map(|r| goal_error_message(*r, language))
                                }
                                _ => None,
                            }
                            .unwrap_or(strings(language).goal_err_save);
                            self.raise_toast_text(ctx, Some(text.to_owned()));
                        }
                    }
                }
            }
        }
    }

    /// Run the goal form: a committed write closes it (and selects a created goal); a refused one
    /// keeps every typed value and shows the reason next to its field.
    fn handle_goal_form(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.goal_form.take() else {
            return;
        };
        match goal_form::show(ctx, &mut state, &self.core.vm) {
            Some(GoalFormEvent::Close) => self.restore_modal_opener(ctx),
            Some(GoalFormEvent::Submit(action)) => match self.core.apply(action) {
                Ok(outcome) => {
                    self.raise_toast(ctx, outcome.toast);
                    if outcome.created_goal.is_some() {
                        self.core.ui.goals.pending_focus = Some(GoalFocus::Primary);
                        self.modal_opener = None;
                    } else {
                        self.restore_modal_opener(ctx);
                    }
                }
                Err(err) => {
                    tracing::warn!("could not save the goal form: {err}");
                    match err {
                        AppError::GoalInvalid(rules) => state.refuse(&rules),
                        _ => state.refuse(&[]),
                    }
                    self.goal_form = Some(state);
                }
            },
            None => self.goal_form = Some(state),
        }
    }

    fn open_quick_add(&mut self) {
        let people: Vec<_> = self.core.vm.active_people().into_iter().cloned().collect();
        let today = self.today();
        self.quick_add = Some(QuickAddState::new(&people, today, self.core.vm.language()));
    }

    fn raise_toast(&mut self, ctx: &egui::Context, key: Option<ToastKey>) {
        let text = key.map(|key| key.text(self.core.vm.language()).to_string());
        self.raise_toast_text(ctx, text);
    }

    fn raise_toast_text(&mut self, ctx: &egui::Context, text: Option<String>) {
        if let Some(text) = text {
            // The clock starts when the toast is first drawn, not now: a native dialog or a slow
            // export may block this thread for seconds before the next frame.
            self.toast = Some(Toast::pending(text));
            ctx.request_repaint();
        }
    }

    fn raise_confirmed_toast(
        &mut self,
        ctx: &egui::Context,
        toast: Option<(String, Option<ToastAction>)>,
    ) {
        if let Some((text, action)) = toast {
            self.toast = Some(Toast::pending(text).with_action(action));
            ctx.request_repaint();
        }
    }

    fn apply_undo(&mut self, ctx: &egui::Context, action: AppAction) {
        if let Err(err) = self.core.apply(action) {
            tracing::warn!("undo action failed: {err}");
            self.raise_toast(ctx, Some(ToastKey::UndoFailed));
        }
    }
}

impl eframe::App for AthenaApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

impl AthenaApp {
    /// One frame of the whole app. Kept apart from `eframe::App::ui` so tests can drive it.
    fn show(&mut self, ui: &mut egui::Ui) {
        self.core.vm.today = self.today();
        if let Err(err) = self.core.sync() {
            tracing::error!("could not refresh the view: {err}");
        }
        let frame_ctx = ui.ctx().clone();
        self.schedule_date_check(&frame_ctx);
        if self.refresh_appearance(&frame_ctx) {
            ui.set_style(frame_ctx.global_style());
        }
        self.guard_window_close(ui.ctx());
        if !self.core.vm.onboarding_complete() {
            let theme = self.core.vm.theme();
            if let Some(result) = onboarding::show(ui, theme, &mut self.onboarding) {
                match self.core.complete_onboarding(result) {
                    Ok(outcome) => {
                        self.core.set_screen(Screen::Month);
                        let ctx = ui.ctx().clone();
                        self.raise_toast(&ctx, outcome.toast);
                    }
                    Err(err) => {
                        tracing::error!("onboarding could not be saved: {err}");
                        self.onboarding.error = Some(OnboardingIssue::SaveFailed);
                    }
                }
            }
            return;
        }
        let ctx = ui.ctx().clone();
        if self.quick_add.is_none()
            && self.goal_form.is_none()
            && self.confirm.is_none()
            && self.import_preview.is_none()
            && self.appearance_intent.is_none()
        {
            if self.pending_focus_restore.is_some() {
                if ctx.memory(|m| m.top_modal_layer().is_none()) {
                    if let Some(id) = self.pending_focus_restore.take() {
                        ctx.memory_mut(|m| m.request_focus(id));
                    }
                } else {
                    ctx.request_repaint();
                }
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::N)) {
                self.remember_modal_opener(&ctx);
                self.open_quick_add();
            }
            for &(key, screen) in &navigation::SHORTCUTS {
                if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, key)) {
                    self.navigate(&ctx, screen);
                }
            }
            let text_focused = ctx
                .memory(egui::Memory::focused)
                .is_some_and(|id| egui::text_edit::TextEditState::load(&ctx, id).is_some());
            if !text_focused
                && self.toast.as_ref().is_some_and(|t| t.action.is_some())
                && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z))
                && let Some(ToastAction::Undo(action)) = self.toast.take().and_then(|t| t.action)
            {
                self.apply_undo(&ctx, action);
            }
        }
        let backdrop = self.update_wallpaper(&ctx);
        let page_before = self.core.ui.settings.page;
        let output = shell::show_with(ui, &self.core.vm, &mut self.core.ui, &backdrop);
        if let Some(screen) = output.navigate {
            self.navigate(&ctx, screen);
        }
        self.guard_appearance_page(&ctx, page_before);
        let mut settings_failed = false;
        for event in output.events {
            // After one refused settings change, the rest of that batch is dropped.
            if settings_failed && matches!(event, ScreenEvent::Apply(_)) {
                continue;
            }
            settings_failed |= !self.handle_screen_event(&ctx, event);
        }

        self.sync_appearance_session(&ctx);
        self.handle_modals(&ctx);
        if let Some(undo_action) = toast::show(
            &ctx,
            &mut self.toast,
            self.core.vm.theme(),
            self.core.vm.language(),
        ) {
            self.apply_undo(&ctx, undo_action);
        }
    }

    /// A native close request with unsaved Quick Add changes is cancelled and routed through the
    /// same Keep editing / Discard prompt; confirming Discard asks the window to close once.
    fn guard_window_close(&mut self, ctx: &egui::Context) {
        if self.close_confirmed || !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if let Some(state) = self.quick_add.as_mut()
            && state.is_dirty()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if state.pending_intent.is_none() {
                state.pending_intent = Some(quick_add::QuickAddIntent::CloseWindow);
                state.focus_prompt = true;
            }
            ctx.request_repaint();
        }
    }

    /// Whether the open Appearance session has changes that are not applied yet.
    fn appearance_dirty(&self) -> bool {
        self.core
            .ui
            .settings
            .appearance
            .as_ref()
            .is_some_and(AppearanceEdit::is_dirty)
    }

    /// Run `intent` now, or park it behind the Keep editing / Discard prompt when the Appearance
    /// draft is dirty. Returns whether it may proceed now.
    fn guard_appearance(&mut self, ctx: &egui::Context, intent: AppearanceIntent) -> bool {
        if !self.appearance_dirty() {
            return true;
        }
        if self.appearance_intent.is_none() {
            self.remember_modal_opener(ctx);
            self.appearance_intent = Some(intent);
            self.appearance_prompt_focus = true;
            ctx.request_repaint();
        }
        false
    }

    /// Switch screens, guarding a dirty Appearance draft.
    fn navigate(&mut self, ctx: &egui::Context, screen: Screen) {
        if screen != self.core.ui.screen
            && self.core.ui.screen == Screen::Settings
            && !self.guard_appearance(ctx, AppearanceIntent::Navigate(screen))
        {
            return;
        }
        self.core.set_screen(screen);
    }

    /// A Settings sub-page switch away from a dirty Appearance draft is undone and prompted.
    fn guard_appearance_page(&mut self, ctx: &egui::Context, before: SettingsPage) {
        let now = self.core.ui.settings.page;
        if before == SettingsPage::Appearance && now != before && self.appearance_dirty() {
            self.core.ui.settings.page = before;
            self.guard_appearance(ctx, AppearanceIntent::SettingsPage(now));
        }
    }

    fn perform_intent(&mut self, ctx: &egui::Context, intent: AppearanceIntent) {
        match intent {
            AppearanceIntent::Navigate(screen) => self.core.set_screen(screen),
            AppearanceIntent::SettingsPage(page) => self.core.ui.settings.page = page,
            AppearanceIntent::Event(event) => {
                self.handle_screen_event(ctx, event);
            }
        }
    }

    /// Open the file dialog and stage the chosen image for the preview.
    fn pick_wallpaper(&mut self) {
        #[cfg(test)]
        let picked = if self.test_wallpaper_picks.is_empty() {
            None
        } else {
            self.test_wallpaper_picks.remove(0)
        };
        #[cfg(not(test))]
        let picked = files::pick_wallpaper_path();
        let Some(path) = picked else { return };
        let store = MediaStore::new(self.core.store.dirs());
        let staged = crate::infrastructure::media::assets::read_source(&path)
            .and_then(|bytes| store.stage(std::sync::Arc::new(bytes)));
        if let Some(edit) = self.core.ui.settings.appearance.as_mut() {
            match staged {
                Ok(staged) => edit.choose_file(staged),
                Err(error) => edit.pick_error = Some(error),
            }
        }
    }

    /// Commit the Appearance draft. A failure keeps the draft, the staged file and the current
    /// wallpaper, and is shown on the page.
    fn apply_appearance(&mut self, ctx: &egui::Context) {
        let Some(edit) = self.core.ui.settings.appearance.as_mut() else {
            return;
        };
        if !edit.can_apply() {
            return;
        }
        let action = edit.apply_action();
        match self.core.apply(action) {
            Ok(outcome) => {
                self.core.ui.settings.appearance = None;
                self.appearance_preview = None;
                self.preview_staged = None;
                self.raise_toast(ctx, outcome.toast);
            }
            Err(err) => {
                tracing::warn!("appearance could not be applied: {err}");
                if let Some(edit) = self.core.ui.settings.appearance.as_mut() {
                    edit.apply_failed = true;
                    edit.apply_error = match err {
                        AppError::Media(error) => Some(error),
                        _ => None,
                    };
                }
            }
        }
    }

    /// Keep the live preview in step with the open Appearance draft: end the session when the
    /// page is left, publish the draft and its staged file to the shell, and report decode status
    /// and effective fallbacks back to the page.
    fn sync_appearance_session(&mut self, ctx: &egui::Context) {
        let on_page = self.core.ui.screen == Screen::Settings
            && self.core.ui.settings.page == SettingsPage::Appearance;
        if !on_page && self.appearance_intent.is_none() {
            self.core.ui.settings.appearance = None;
        }
        let (draft, staged) = self
            .core
            .ui
            .settings
            .appearance
            .as_ref()
            .map_or((None, None), |e| (Some(e.draft.clone()), e.staged.clone()));
        if draft != self.appearance_preview {
            ctx.request_repaint();
        }
        self.appearance_preview = draft;
        self.preview_staged = staged;
        let loading = self
            .wallpaper
            .as_ref()
            .is_some_and(WallpaperRuntime::displayed_loading);
        let failure = self
            .wallpaper
            .as_ref()
            .and_then(WallpaperRuntime::displayed_failure);
        let look = &self.look;
        if let Some(edit) = self.core.ui.settings.appearance.as_mut() {
            edit.status = PreviewStatus {
                loading,
                failure,
                high_contrast: look.high_contrast,
                reduced_motion: look.reduced_motion,
                motion_forced_by_os: look.motion_forced && !look.high_contrast,
                transparency_forced: look.transparency_forced,
            };
        }
    }

    #[allow(clippy::too_many_lines)]
    fn handle_modals(&mut self, ctx: &egui::Context) {
        if let Some(intent) = self.appearance_intent.take() {
            let s = strings(self.core.vm.language());
            let focus = std::mem::take(&mut self.appearance_prompt_focus);
            match discard_prompt::show(
                ctx,
                self.core.vm.theme(),
                self.core.vm.language(),
                s.ap_discard_title,
                s.ap_discard_body,
                focus,
            ) {
                Some(DiscardChoice::Keep) => self.restore_modal_opener(ctx),
                Some(DiscardChoice::Discard) => {
                    self.core.ui.settings.appearance = None;
                    self.appearance_preview = None;
                    self.preview_staged = None;
                    self.modal_opener = None;
                    self.perform_intent(ctx, intent);
                }
                None => self.appearance_intent = Some(intent),
            }
        }
        if let Some(mut state) = self.quick_add.take() {
            match quick_add::show(ctx, &mut state, &self.core.vm) {
                Some(quick_add::QuickAddEvent::QuitConfirmed) => {
                    self.close_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Some(event) => {
                    let (after, toast) = handle_quick_add(&mut self.core, &mut state, event);
                    self.raise_toast(ctx, toast);
                    if after == ModalAfter::Keep {
                        self.quick_add = Some(state);
                    } else {
                        self.restore_modal_opener(ctx);
                    }
                }
                None => self.quick_add = Some(state),
            }
        }
        self.handle_goal_form(ctx);
        if let Some(dialog) = self.confirm.take() {
            match confirm_dialog::show(ctx, &dialog, self.core.vm.theme(), self.core.vm.language())
            {
                Some(ConfirmChoice::Confirmed(action)) => {
                    let is_purge = matches!(action, AppAction::PurgeTransaction(_));
                    let is_delete = matches!(action, AppAction::DeleteTransaction(_));
                    let is_restore = matches!(action, AppAction::RestoreBackup(_));
                    let purged_id = match action {
                        AppAction::PurgeTransaction(id) => Some(id),
                        _ => None,
                    };
                    let emptied_trash = matches!(action, AppAction::EmptyTrash);
                    let was_reset = matches!(action, AppAction::ResetAppearance);
                    let goal_step = matches!(
                        action,
                        AppAction::ChangeGoalStatus(_, _) | AppAction::DeleteContribution { .. }
                    );
                    let removed_contribution =
                        matches!(action, AppAction::DeleteContribution { .. });
                    let before_version = self.core.store.data_version();
                    let toast = apply_confirmed_outcome(&mut self.core, action);
                    if was_reset && self.core.store.data_version() != before_version {
                        self.core.ui.settings.appearance = None;
                        self.appearance_preview = None;
                        self.preview_staged = None;
                    }
                    let restored_ok =
                        is_restore && self.core.store.data_version() != before_version;
                    if self.core.store.data_version() != before_version {
                        if restored_ok {
                            self.toast = None;
                        } else if let Some(id) = purged_id
                            && self.toast.as_ref().is_some_and(|t| {
                                matches!(
                                    &t.action,
                                    Some(ToastAction::Undo(AppAction::RestoreTransaction(
                                        restore_id
                                    ))) if *restore_id == id
                                )
                            })
                        {
                            self.toast = None;
                        } else if emptied_trash
                            && self.toast.as_ref().is_some_and(|t| {
                                matches!(
                                    &t.action,
                                    Some(ToastAction::Undo(AppAction::RestoreTransaction(_)))
                                )
                            })
                        {
                            self.toast = None;
                        }
                    }
                    self.raise_confirmed_toast(ctx, toast);
                    if is_purge {
                        self.core.ui.trash_focus_row = self.core.ui.pending_purge_focus_row.take();
                        self.modal_opener = None;
                    } else if goal_step && self.core.store.data_version() != before_version {
                        // The opener (a menu or a contribution row) is gone; focus moves on.
                        self.modal_opener = None;
                        if removed_contribution {
                            self.core.ui.goals.pending_focus = Some(GoalFocus::Primary);
                        }
                    } else if is_delete || restored_ok {
                        self.modal_opener = None;
                    } else {
                        self.restore_modal_opener(ctx);
                    }
                }
                Some(ConfirmChoice::Cancelled) => {
                    self.core.ui.pending_purge_focus_row = None;
                    self.restore_modal_opener(ctx);
                }
                None => self.confirm = Some(dialog),
            }
        }
        self.handle_import_preview_modal(ctx);
    }

    fn handle_import_preview_modal(&mut self, ctx: &egui::Context) {
        if let Some(dialog) = self.import_preview.take() {
            match import_preview::show(ctx, &dialog, self.core.vm.theme(), self.core.vm.language())
            {
                Some(ImportPreviewChoice::Confirmed(plan)) => {
                    let before_version = self.core.store.data_version();
                    let toast =
                        apply_confirmed_outcome(&mut self.core, AppAction::ImportProfile(plan));
                    let imported_ok = self.core.store.data_version() != before_version;
                    if imported_ok {
                        self.toast = None;
                    }
                    self.raise_confirmed_toast(ctx, toast);
                    if imported_ok {
                        self.modal_opener = None;
                    } else {
                        self.restore_modal_opener(ctx);
                    }
                }
                Some(ImportPreviewChoice::Cancelled) => {
                    self.restore_modal_opener(ctx);
                }
                None => self.import_preview = Some(dialog),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::money::Money;
    use crate::domain::person::PersonKind;
    use crate::domain::transaction::{Allocation, TransactionType};
    use crate::i18n::ToastKey;
    use crate::shared::test_fixtures::draft_with;
    use crate::ui::quick_add::{QuickAddError, QuickAddEvent, QuickAddMode};
    use crate::ui::screens::ExportRequest;
    use crate::ui::screens::settings::SettingsError;

    fn onboarded(partner: Option<&str>) -> AppCore {
        let mut core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        core.complete_onboarding(OnboardingResult {
            display_name: "A3".into(),
            partner_name: partner.map(Into::into),
            start_balance: Money::from_cents(10_000),
            language: Language::En,
        })
        .unwrap();
        core
    }

    #[test]
    fn onboarding_creates_owner_and_partner() {
        let core = onboarded(Some("Ana"));
        assert_eq!(core.vm.all_people.len(), 2);
        assert_eq!(core.vm.all_people[0].kind, PersonKind::Owner);
        assert_eq!(core.vm.all_people[1].kind, PersonKind::Partner);
        assert_eq!(core.owner().unwrap().name, "A3");
        assert!(core.vm.profile.as_ref().unwrap().onboarding_complete);
        assert_eq!(core.vm.owner_name(), Some("A3"));
        assert_eq!(core.vm.balance, Money::from_cents(10_000));
    }

    #[test]
    fn before_onboarding_the_view_model_is_empty_but_usable() {
        let core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        assert!(!core.vm.onboarding_complete());
        assert_eq!(core.vm.language(), Language::En);
        assert_eq!(core.vm.theme(), Theme::Dark);
        assert_eq!(core.vm.owner_name(), None);
        assert_eq!(core.vm.calendar.len(), 12);
        assert_eq!(core.vm.summary.count, 0);
    }

    #[test]
    fn language_and_theme_come_from_the_profile() {
        let mut core = onboarded(None);
        let mut profile = core.vm.profile.clone().unwrap();
        profile.language = Language::PtBr;
        profile.theme = Theme::Light;
        let outcome = core.save_profile(profile).unwrap();
        assert_eq!(outcome.toast, Some(ToastKey::ProfileSaved));
        assert_eq!(core.vm.language(), Language::PtBr);
        assert_eq!(core.vm.theme(), Theme::Light);
    }

    #[test]
    fn archived_person_leaves_active_list_keeps_history() {
        let mut core = onboarded(Some("Ana"));
        let partner = core
            .vm
            .all_people
            .iter()
            .find(|p| p.kind == PersonKind::Partner)
            .unwrap()
            .id;
        core.archive_person(partner).unwrap();
        assert!(
            !core
                .vm
                .all_people
                .iter()
                .any(|p| p.id == partner && !p.archived)
        );
        assert!(
            core.vm
                .all_people
                .iter()
                .any(|p| p.id == partner && p.archived)
        );
        assert!(core.vm.active_people().iter().all(|p| p.id != partner));
        assert_eq!(
            core.vm.person(partner).map(|p| p.name.as_str()),
            Some("Ana")
        );
    }

    #[test]
    fn the_owner_cannot_be_archived() {
        let mut core = onboarded(None);
        let owner = core.owner().unwrap().id;
        let version = core.store.data_version();
        assert!(core.archive_person(owner).is_err());
        assert_eq!(core.store.data_version(), version);
        assert!(!core.owner().unwrap().archived);
    }

    #[test]
    fn archived_category_still_resolves_its_label() {
        let mut core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        let id = core
            .vm
            .all_categories
            .iter()
            .find(|c| c.name == "Food")
            .unwrap()
            .id;
        core.apply(AppAction::ArchiveCategory(id)).unwrap();
        assert_eq!(core.vm.category(id).map(|c| c.name.as_str()), Some("Food"));
        assert!(
            core.vm
                .active_categories(TransactionType::Expense)
                .iter()
                .all(|c| c.id != id)
        );
    }

    #[test]
    fn category_pickers_follow_the_kind() {
        let core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        let names = |kind| -> Vec<&str> {
            core.vm
                .active_categories(kind)
                .iter()
                .map(|c| c.name.as_str())
                .collect()
        };
        assert!(names(TransactionType::Income).contains(&"Salary"));
        assert!(!names(TransactionType::Expense).contains(&"Salary"));
        assert!(names(TransactionType::Refund).contains(&"Food"));
    }

    #[test]
    fn creating_a_transaction_refreshes_the_register_and_totals() {
        let mut core = onboarded(Some("Ana"));
        let owner = core.owner().unwrap().id;
        let draft = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(2_500),
        }]);
        core.create_transaction(draft).unwrap();
        assert_eq!(core.vm.transactions.len(), 1);
        assert_eq!(core.vm.summary.expenses, Money::from_cents(2_500));
        assert_eq!(core.vm.balance, Money::from_cents(7_500));
        assert_eq!(core.vm.person_totals.len(), 1);
        assert_eq!(core.vm.calendar[10].summary.count, 1);
    }

    #[test]
    fn changing_month_reloads_the_register_without_another_action() {
        let mut core = onboarded(None);
        core.create_transaction(draft_with(vec![])).unwrap();
        assert_eq!(core.vm.transactions.len(), 1);
        core.set_month(YearMonth::new(2026, 10).unwrap()).unwrap();
        assert!(core.vm.transactions.is_empty());
        assert_eq!(core.vm.summary.count, 0);
        core.set_month(YearMonth::new(2026, 11).unwrap()).unwrap();
        assert_eq!(core.vm.transactions.len(), 1);
    }

    #[test]
    fn sync_reloads_only_when_something_changed() {
        let mut core = onboarded(None);
        assert!(!core.sync().unwrap());
        core.ui.filter.text = "x".into();
        assert!(core.sync().unwrap());
        assert!(!core.sync().unwrap());
        core.set_calendar_year(2027).unwrap();
        assert_eq!(core.vm.calendar[0].month.year, 2027);
    }

    #[test]
    fn scoped_reloads_leave_other_data_alone() {
        let mut core = onboarded(None);
        core.create_transaction(draft_with(vec![])).unwrap();
        core.save_person(Person {
            id: 0,
            name: "Bia".into(),
            kind: PersonKind::Other,
            archived: false,
            created_at: String::new(),
        })
        .unwrap();
        assert_eq!(core.vm.all_people.len(), 2);
        assert_eq!(core.vm.transactions.len(), 1);
        assert!(
            !core.sync().unwrap(),
            "a scoped reload leaves the cache current"
        );
    }

    #[test]
    fn failed_actions_change_nothing() {
        let mut core = onboarded(None);
        let mut bad = draft_with(vec![]);
        bad.amount = Money::ZERO;
        assert!(core.create_transaction(bad).is_err());
        assert!(core.vm.transactions.is_empty());
    }

    fn quick_state(core: &AppCore) -> QuickAddState {
        let people: Vec<_> = core.vm.active_people().into_iter().cloned().collect();
        QuickAddState::new(&people, crate::shared::test_fixtures::today(), Language::En)
    }

    fn valid_draft(state: &mut QuickAddState) -> crate::domain::transaction::TxDraft {
        state.title = "Market".into();
        state.amount_input = "45,90".into();
        state.to_draft().unwrap()
    }

    #[test]
    fn a_new_entry_saves_and_leaves_the_modal_ready_for_the_next() {
        let mut core = onboarded(Some("Ana"));
        let mut state = quick_state(&core);
        let draft = valid_draft(&mut state);
        let (after, toast) = handle_quick_add(&mut core, &mut state, QuickAddEvent::Submit(draft));
        assert_eq!(
            (after, toast),
            (ModalAfter::Keep, Some(ToastKey::TransactionSaved))
        );
        assert_eq!(core.vm.transactions.len(), 1);
        assert_eq!(core.vm.transactions[0].title, "Market");
        assert_eq!(core.vm.summary.expenses, Money::from_cents(4_590));
        assert!(state.title.is_empty() && state.amount_input.is_empty());
        assert!(
            state.focus_amount,
            "focus returns to the amount for repeated entry"
        );
        assert_eq!(state.payer_id, core.owner().map(|p| p.id));
    }

    #[test]
    fn editing_saves_and_closes() {
        let mut core = onboarded(None);
        let mut state = quick_state(&core);
        let draft = valid_draft(&mut state);
        handle_quick_add(&mut core, &mut state, QuickAddEvent::Submit(draft));
        let stored = core.vm.transactions[0].clone();
        let people: Vec<_> = core.vm.active_people().into_iter().cloned().collect();
        let mut edit = QuickAddState::from_transaction(&stored, &people, Language::En);
        edit.title = "Supermarket".into();
        let draft = edit.to_draft().unwrap();
        let (after, toast) = handle_quick_add(&mut core, &mut edit, QuickAddEvent::Submit(draft));
        assert_eq!(
            (after, toast),
            (ModalAfter::Close, Some(ToastKey::TransactionUpdated))
        );
        assert_eq!(core.vm.transactions[0].title, "Supermarket");
        assert_eq!(core.vm.transactions.len(), 1);
    }

    #[test]
    fn a_refused_save_keeps_the_modal_and_the_input() {
        let mut core = onboarded(Some("Ana"));
        let mut state = quick_state(&core);
        let mut draft = valid_draft(&mut state);
        draft.payer_id = Some(9_999);
        let (after, toast) = handle_quick_add(&mut core, &mut state, QuickAddEvent::Submit(draft));
        assert_eq!((after, toast), (ModalAfter::Keep, None));
        assert_eq!(state.errors, vec![QuickAddError::Save]);
        assert_eq!(state.title, "Market", "nothing the user typed is lost");
        assert!(core.vm.transactions.is_empty());
    }

    #[test]
    fn close_discards_without_saving() {
        let mut core = onboarded(None);
        let mut state = quick_state(&core);
        valid_draft(&mut state);
        // A dirty form never closes silently: the request waits behind the discard prompt, and
        // only an explicit Discard closes it, without saving.
        assert!(state.is_dirty());
        assert_eq!(
            state.request(quick_add::QuickAddIntent::Close, &core.vm),
            None
        );
        let intent = state
            .pending_intent
            .take()
            .expect("Close waits for the prompt");
        let event = state
            .perform_intent(intent, &core.vm)
            .expect("Discard closes");
        assert_eq!(event, QuickAddEvent::Close);
        let (after, toast) = handle_quick_add(&mut core, &mut state, event);
        assert_eq!((after, toast), (ModalAfter::Close, None));
        assert!(core.vm.transactions.is_empty());
    }

    #[test]
    fn opening_a_month_from_the_calendar_selects_it_and_shows_the_register() {
        let mut core = onboarded(None);
        let mut draft = draft_with(vec![]);
        draft.date = crate::shared::test_fixtures::date(2026, 3, 9);
        core.create_transaction(draft).unwrap();
        core.set_screen(Screen::Calendar);
        core.open_month(YearMonth::new(2026, 3).unwrap()).unwrap();
        assert_eq!(core.ui.screen, Screen::Month);
        assert_eq!(core.ui.month, YearMonth::new(2026, 3).unwrap());
        assert_eq!(core.vm.transactions.len(), 1);
        assert_eq!(core.vm.summary.count, 1);
    }

    #[test]
    fn editing_a_stored_transaction_changes_it_in_place_and_closes() {
        let mut core = onboarded(Some("Ana"));
        let mut state = quick_state(&core);
        let draft = valid_draft(&mut state);
        handle_quick_add(&mut core, &mut state, QuickAddEvent::Submit(draft));
        let id = core.vm.transactions[0].id;

        let mut edit = begin_edit(&core, id).unwrap();
        assert_eq!(edit.mode, QuickAddMode::Edit(id));
        assert_eq!(edit.title, "Market");
        assert_eq!(edit.amount_input, "45,90");
        assert_eq!(
            edit.payer_id,
            core.owner().map(|p| p.id),
            "the payer is prefilled too"
        );
        edit.title = "Supermarket".into();
        edit.amount_input = "50,00".into();
        let draft = edit.to_draft().unwrap();
        let (after, toast) = handle_quick_add(&mut core, &mut edit, QuickAddEvent::Submit(draft));
        assert_eq!(
            (after, toast),
            (ModalAfter::Close, Some(ToastKey::TransactionUpdated))
        );
        assert_eq!(core.vm.transactions.len(), 1);
        assert_eq!(core.vm.transactions[0].title, "Supermarket");
        assert_eq!(core.vm.summary.expenses, Money::from_cents(5_000));
        assert!(begin_edit(&core, 9_999).is_err());
    }

    #[test]
    fn deleting_asks_first_names_the_entry_and_removes_it_only_when_confirmed() {
        let mut core = onboarded(None);
        let mut state = quick_state(&core);
        let draft = valid_draft(&mut state);
        handle_quick_add(&mut core, &mut state, QuickAddEvent::Submit(draft));
        let id = core.vm.transactions[0].id;

        let dialog = delete_confirmation(&core, id).unwrap();
        assert!(dialog.danger);
        assert!(
            dialog.body.contains("Market") && dialog.body.contains("R$ 45,90"),
            "{}",
            dialog.body
        );
        assert_eq!(dialog.action, AppAction::DeleteTransaction(id));
        assert_eq!(
            core.vm.transactions.len(),
            1,
            "building the dialog deletes nothing"
        );

        let toast = apply_confirmed(&mut core, dialog.action);
        assert_eq!(toast.as_deref(), Some("Deleted \u{201c}Market\u{201d}."));
        assert!(core.vm.transactions.is_empty());
        assert_eq!(core.vm.summary.count, 0);
        assert!(delete_confirmation(&core, id).is_err());
    }

    #[test]
    fn a_failed_confirmed_action_shows_no_toast() {
        let mut core = onboarded(None);
        assert_eq!(
            apply_confirmed(&mut core, AppAction::DeleteTransaction(9_999)),
            None
        );
    }

    #[test]
    fn other_confirmed_actions_use_their_standard_toast() {
        let mut core = onboarded(Some("Ana"));
        let partner = core
            .vm
            .all_people
            .iter()
            .find(|p| p.kind == PersonKind::Partner)
            .unwrap()
            .id;
        let toast = apply_confirmed(&mut core, AppAction::ArchivePerson(partner));
        assert_eq!(toast.as_deref(), Some("Person archived."));
    }

    /// The whole app, driven like a user: onboarding, add an entry, edit it, delete it, browse the
    /// calendar. The date is pinned so the test does not depend on when it runs.
    #[test]
    #[allow(clippy::too_many_lines)] // one long journey is the point of this test
    fn a_user_can_set_up_log_edit_delete_and_browse() {
        use egui_kittest::kittest::Queryable;

        let core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        let mut app = AthenaApp::for_test(core);
        app.core.ui.set_month(YearMonth::new(2026, 11).unwrap());
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 820.0))
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    // The harness only has egui's bundled fonts, which lack some of our symbols.
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                app,
            );
        let type_into =
            |harness: &mut egui_kittest::Harness<'_, AthenaApp>, index: usize, text: &str| {
                harness
                    .get_all_by_role(egui::Role::TextInput)
                    .nth(index)
                    .unwrap()
                    .focus();
                harness.run_steps(2);
                harness
                    .get_all_by_role(egui::Role::TextInput)
                    .nth(index)
                    .unwrap()
                    .type_text(text);
                harness.run_steps(2);
            };

        harness.run_steps(3);
        assert!(harness.query_by_label("Welcome to Athena").is_some());
        harness.get_by_label("Continue").click();
        harness.run_steps(3);
        type_into(&mut harness, 0, "A3");
        harness.get_by_label("Continue").click();
        harness.run_steps(3);
        harness.get_by_label("Skip").click();
        harness.run_steps(3);
        harness.get_by_label("Start").click();
        harness.run_steps(4);
        assert!(harness.state().core.vm.onboarding_complete());
        assert_eq!(
            harness.state().core.owner().map(|p| p.name.as_str()),
            Some("A3")
        );
        assert!(
            harness.query_by_label("November 2026").is_some(),
            "the Month screen opens"
        );
        assert!(
            harness
                .query_by_label("Nothing logged in November 2026 yet.")
                .is_some()
        );

        harness
            .get_all_by_label("New transaction")
            .next()
            .unwrap()
            .click();
        harness.run_steps(3);
        // The register's search box comes first in the tree, then the modal's fields.
        // After the register's search box: the one-line field, then amount and title.
        type_into(&mut harness, 2, "12,50");
        type_into(&mut harness, 3, "Coffee");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(4);
        assert_eq!(harness.state().core.vm.transactions.len(), 1);
        assert!(
            harness
                .state()
                .quick_add
                .as_ref()
                .is_some_and(|q| q.title.is_empty())
        );
        assert!(
            harness.state().toast.is_some(),
            "a confirmation toast appears"
        );
        harness.key_press(egui::Key::Escape);
        harness.run_steps(3);
        assert!(harness.state().quick_add.is_none());
        assert!(harness.query_by_label("Edit Coffee").is_some());
        assert_eq!(
            harness.state().core.vm.summary.expenses,
            Money::from_cents(1_250)
        );

        harness.get_by_label("Edit Coffee").click();
        harness.run_steps(3);
        assert!(harness.query_by_label("Edit transaction").is_some());
        type_into(&mut harness, 2, " Beans");
        harness.key_press(egui::Key::Enter);
        harness.run_steps(4);
        assert!(
            harness.state().quick_add.is_none(),
            "an edit closes the modal"
        );
        assert_eq!(
            harness.state().core.vm.transactions[0].title,
            "Coffee Beans"
        );

        harness.get_by_label("Delete Coffee Beans").click();
        harness.run_steps(3);
        assert!(harness.query_by_label("Delete this transaction?").is_some());
        harness.get_by_label("Cancel").click();
        harness.run_steps(3);
        assert_eq!(
            harness.state().core.vm.transactions.len(),
            1,
            "cancel keeps the row"
        );
        harness.get_by_label("Delete Coffee Beans").click();
        harness.run_steps(3);
        harness.get_by_label("Delete").click();
        harness.run_steps(4);
        assert!(harness.state().core.vm.transactions.is_empty());
        assert!(
            harness
                .query_by_label("Nothing logged in November 2026 yet.")
                .is_some()
        );

        harness.get_by_label("Calendar").click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.ui.screen, Screen::Calendar);
        harness.get_by_label("March 2026").click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.ui.screen, Screen::Month);
        assert_eq!(
            harness.state().core.ui.month,
            YearMonth::new(2026, 3).unwrap()
        );
        assert!(harness.query_by_label("March 2026").is_some());
    }

    #[test]
    fn settings_changes_apply_and_the_screen_reloads_what_is_stored() {
        let mut core = onboarded(Some("Ana"));
        core.set_screen(Screen::Settings);
        core.ui.settings.owner_name = "Half typed".into();
        let mut profile = core.vm.profile.clone().unwrap();
        profile.language = Language::PtBr;
        let (text, ok) = apply_settings_action(&mut core, AppAction::SaveProfile(profile));
        assert!(ok);
        assert_eq!(
            text.as_deref(),
            Some("Perfil salvo."),
            "the toast speaks the new language"
        );
        assert_eq!(core.vm.language(), Language::PtBr);
        assert_eq!(
            core.ui.settings.owner_name, "",
            "edits reset, so the form reloads from the store"
        );
    }

    #[test]
    fn a_refused_settings_change_is_reported_and_changes_nothing() {
        let mut core = onboarded(Some("Ana"));
        let mut owner = core.owner().unwrap().clone();
        owner.name = "ana".into();
        let before = core.store.data_version();
        let (text, ok) = apply_settings_action(&mut core, AppAction::SavePerson(owner));
        assert_eq!((text, ok), (None, false));
        assert_eq!(core.ui.settings.error, Some(SettingsError::SaveRejected));
        assert_eq!(core.store.data_version(), before);
        assert_eq!(core.owner().unwrap().name, "A3");
    }

    #[test]
    fn leaving_settings_drops_unsaved_edits() {
        let mut core = onboarded(None);
        core.set_screen(Screen::Settings);
        core.ui.settings.new_person = "Typed".into();
        core.set_screen(Screen::Month);
        assert_eq!(core.ui.settings.new_person, "");
    }

    #[test]
    fn an_archived_person_leaves_the_pickers_but_keeps_their_name_on_old_entries() {
        let mut core = onboarded(Some("Ana"));
        let ana = core
            .vm
            .all_people
            .iter()
            .find(|p| p.kind == PersonKind::Partner)
            .unwrap()
            .id;
        let mut state = quick_state(&core);
        state.set_split(crate::ui::quick_add::SplitChoice::Person(ana));
        let draft = valid_draft(&mut state);
        handle_quick_add(&mut core, &mut state, QuickAddEvent::Submit(draft));

        let (_, ok) = apply_settings_action(&mut core, AppAction::ArchivePerson(ana));
        assert!(ok);
        assert!(
            core.vm.active_people().iter().all(|p| p.id != ana),
            "hidden from new entries"
        );
        assert!(quick_state(&core).to_draft().is_err() || quick_state(&core).payer_id != Some(ana));
        let stored = &core.vm.transactions[0];
        assert_eq!(
            crate::ui::components::transaction_row::people_label(stored, &core.vm),
            "Ana"
        );
    }

    /// Settings through the real app: rename the owner, switch language and theme, archive the
    /// partner. The sidebar and every screen follow immediately.
    #[test]
    fn settings_changes_show_up_across_the_whole_app() {
        use egui_kittest::kittest::Queryable;

        let core = onboarded(Some("Ana"));
        let app = AthenaApp::for_test(core);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 2600.0))
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                app,
            );
        harness.run_steps(3);
        harness.get_by_label("Settings").click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.ui.screen, Screen::Settings);

        harness
            .get_all_by_role(egui::Role::TextInput)
            .next()
            .unwrap()
            .focus();
        harness.run_steps(2);
        harness
            .get_all_by_role(egui::Role::TextInput)
            .next()
            .unwrap()
            .type_text("x");
        harness.run_steps(2);
        assert_eq!(harness.state().core.ui.settings.owner_name, "A3x");
        harness.get_by_label("Save").click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.owner().unwrap().name, "A3x");
        assert!(
            harness.query_all_by_label("A3x").count() > 0,
            "the sidebar shows the owner"
        );
        assert!(harness.state().toast.is_some());

        // The immediate theme control moved to Appearance: a link opens the page, the choice
        // previews live, and Apply stores it without touching the legacy profile theme.
        harness.get_by_label("Open Appearance").click();
        harness.run_steps(4);
        assert_eq!(
            harness.state().core.ui.settings.page,
            SettingsPage::Appearance
        );
        harness.get_by_label("Light").click();
        harness.run_steps(4);
        assert_eq!(harness.state().look.theme, Theme::Light, "live preview");
        assert_eq!(harness.state().core.vm.profile_theme(), Theme::Dark);
        harness.get_by_label("Apply").click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.vm.theme(), Theme::Light);
        assert_eq!(harness.state().core.vm.profile_theme(), Theme::Dark);
        harness.get_by_label("General").click();
        harness.run_steps(3);
        harness.get_by_label("Português").click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.vm.language(), Language::PtBr);
        assert!(harness.query_all_by_label("Configurações").count() > 0);
        assert!(harness.query_all_by_label("Perfil").count() > 0);

        harness.get_by_label("Pessoas").click();
        harness.run_steps(2);
        harness.get_all_by_label("Arquivar").next().unwrap().click();
        harness.run_steps(3);
        assert_eq!(
            harness.state().core.vm.active_people().len(),
            2,
            "asking archives nothing"
        );
        // People come before categories in the tree: the partner's own button, then the confirm button.
        let confirm = harness.get_all_by_label("Arquivar").nth(1).unwrap();
        confirm.click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.vm.active_people().len(), 1);
        assert_eq!(
            harness.state().core.vm.all_people.len(),
            2,
            "the archived person stays in history"
        );
    }

    #[test]
    fn exports_report_where_they_went_and_how_much() {
        let mut core = onboarded(Some("Ana"));
        let mut state = quick_state(&core);
        let draft = valid_draft(&mut state);
        handle_quick_add(&mut core, &mut state, QuickAddEvent::Submit(draft));
        let dir = tempfile::tempdir().unwrap();

        let json = dir.path().join("all.json");
        let text = run_export(&core, ExportRequest::Json, &json, "2026-11-12T10:00:00Z");
        assert!(text.starts_with("Exported 1 entry to "), "{text}");
        assert!(text.contains("all.json") && json.exists());

        let csv = dir.path().join("all.csv");
        let scope = crate::domain::export::ExportScope::All;
        let text = run_export(&core, ExportRequest::Csv(scope), &csv, "");
        assert!(text.contains("all.csv") && csv.exists());
        let csv_text = std::fs::read_to_string(&csv).unwrap();
        assert!(csv_text.contains("Market") && csv_text.contains(";45,90;"));

        let xlsx = dir.path().join("all.xlsx");
        let text = run_export(&core, ExportRequest::Xlsx(scope), &xlsx, "");
        assert!(text.starts_with("Exported 1 entry to "), "{text}");
        assert!(text.contains("all.xlsx") && xlsx.exists());
    }

    #[test]
    fn a_failed_export_says_so_and_keeps_the_old_file() {
        let core = onboarded(None);
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("blocked.csv");
        std::fs::create_dir(&blocked).unwrap();
        let text = run_export(
            &core,
            ExportRequest::Csv(crate::domain::export::ExportScope::All),
            &blocked,
            "",
        );
        assert_eq!(text, "Export failed. Any existing file was left untouched.");
        assert!(blocked.is_dir());
    }

    #[test]
    fn export_toasts_follow_the_language() {
        let mut core = onboarded(None);
        let mut profile = core.vm.profile.clone().unwrap();
        profile.language = Language::PtBr;
        core.save_profile(profile).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let text = run_export(
            &core,
            ExportRequest::Csv(crate::domain::export::ExportScope::All),
            &dir.path().join("x.csv"),
            "",
        );
        assert!(text.starts_with("Exportado: 0 lançamentos em "), "{text}");
    }

    /// Keyboard-first: Ctrl+N opens quick add from any screen, Esc closes it, Ctrl+1/2/3 switch
    /// screens, and the shortcuts stay out of the way while a modal is open.
    #[test]
    fn keyboard_shortcuts_work_from_every_screen() {
        let app = AthenaApp::for_test(onboarded(Some("Ana")));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 900.0))
            .with_step_dt(0.05)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                app,
            );
        harness.run_steps(3);
        let ctrl = Modifiers::COMMAND;

        harness.key_press_modifiers(ctrl, Key::Num2);
        harness.run_steps(3);
        assert_eq!(harness.state().core.ui.screen, Screen::Calendar);
        harness.key_press_modifiers(ctrl, Key::Num3);
        harness.run_steps(3);
        assert_eq!(harness.state().core.ui.screen, Screen::Settings);
        harness.key_press_modifiers(ctrl, Key::Num4);
        harness.run_steps(3);
        assert_eq!(harness.state().core.ui.screen, Screen::Trash);
        harness.key_press_modifiers(ctrl, Key::Num5);
        harness.run_steps(3);
        assert_eq!(harness.state().core.ui.screen, Screen::Goals);
        harness.key_press_modifiers(ctrl, Key::Num1);
        harness.run_steps(3);
        assert_eq!(harness.state().core.ui.screen, Screen::Month);
        harness.key_press_modifiers(ctrl, Key::Num5);
        harness.run_steps(3);
        harness.key_press_modifiers(ctrl, Key::Num3);
        harness.run_steps(3);
        assert_eq!(harness.state().core.ui.screen, Screen::Settings);

        harness.key_press_modifiers(ctrl, Key::N);
        harness.run_steps(3);
        assert!(harness.state().quick_add.is_some());
        harness.key_press(Key::Escape);
        harness.run_steps(3);
        assert!(harness.state().quick_add.is_none());
        assert!(harness.state().core.vm.transactions.is_empty());

        harness.key_press_modifiers(ctrl, Key::N);
        harness.run_steps(3);
        harness.key_press_modifiers(ctrl, Key::Num1);
        harness.run_steps(3);
        assert_eq!(harness.state().core.ui.screen, Screen::Settings);
        harness.key_press(Key::Escape);
        harness.run_steps(3);
        harness.key_press_modifiers(ctrl, Key::Num1);
        harness.run_steps(3);
        assert_eq!(harness.state().core.ui.screen, Screen::Month);
    }

    fn polish_harness(core: AppCore) -> egui_kittest::Harness<'static, AthenaApp> {
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 900.0))
            .with_context_setup(theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    ui.set_style(ui.ctx().global_style());
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            )
    }

    #[test]
    fn ctrl_z_undoes_visible_delete_but_not_while_typing() {
        undo_toast_restores_and_clears();
    }

    #[test]
    fn trash_keyboard_only_flow() {
        use egui_kittest::kittest::Queryable;
        let mut core = onboarded(None);
        for title in ["First", "Second"] {
            let mut draft = draft_with(vec![]);
            draft.title = title.into();
            core.create_transaction(draft).unwrap();
        }
        for id in core
            .vm
            .transactions
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>()
        {
            core.delete_transaction(id).unwrap();
        }
        core.set_screen(Screen::Trash);
        core.sync().unwrap();
        let mut h = polish_harness(core);
        h.run_steps(3);
        // Start at a known keyboard focus anchor, then navigate to the restore action.
        h.get_by_label("Delete forever Second").focus();
        h.key_press(Key::Tab);
        h.run_steps(2);
        assert!(h.get_by_label("Restore Second").is_focused());
        h.key_press(Key::Enter);
        h.run_steps(4);
        assert_eq!(h.state().core.vm.trash.len(), 1);
        assert!(h.get_by_label("Restore First").is_focused());
    }

    #[test]
    fn screen_reader_labels_for_settlement_and_trash() {
        use egui_kittest::kittest::Queryable;
        let mut core = onboarded(Some("Ana"));
        let owner = core.owner().unwrap().id;
        let partner = core
            .vm
            .all_people
            .iter()
            .find(|p| p.name == "Ana")
            .unwrap()
            .id;
        let mut draft = draft_with(vec![Allocation {
            person_id: partner,
            amount: Money::from_cents(100),
        }]);
        draft.payer_id = Some(owner);
        core.create_transaction(draft).unwrap();
        let mut h = polish_harness(core);
        h.run_steps(3);
        assert!(h.query_by_label("Settlement").is_some());
        let settlement = h.state().core.vm.settlement.as_ref().unwrap();
        let sentence = crate::ui::components::settlement_card::transfer_sentence(
            settlement.transfers[0],
            Some(owner),
            "Ana",
            "A3",
            Language::En,
        );
        assert!(h.query_by_label(&sentence).is_some());
        assert!(
            h.query_by_label(strings(Language::En).settlement_record)
                .is_some()
        );
        let id = h.state().core.vm.transactions[0].id;
        h.state_mut().core.delete_transaction(id).unwrap();
        h.state_mut().core.set_screen(Screen::Trash);
        h.state_mut().core.sync().unwrap();
        h.run_steps(3);
        assert!(h.query_by_label("Restore Test").is_some());
        assert!(h.query_by_label("Delete forever Test").is_some());
    }

    #[test]
    fn dialogs_return_focus_to_their_opener() {
        use egui_kittest::kittest::Queryable;
        let (_dir, mut core) = file_backed_core();
        core.create_transaction(draft_with(vec![])).unwrap();
        let mut h = polish_harness(core);
        h.run_steps(3);
        h.get_by_label("Delete Test").focus();
        h.key_press(Key::Enter);
        h.run_steps(3);
        h.key_press(Key::Escape);
        h.run_steps(4);
        assert!(h.get_by_label("Delete Test").is_focused());
        // Purge has its own row-focus path, covered through a real cancel flow.
        trash_screen_restore_and_delete_forever();
        h.state_mut().core.set_screen(Screen::Settings);
        h.state_mut().core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Data;
        h.state_mut().core.sync().unwrap();
        h.run_steps(3);
        for import in [false, true] {
            h.get_by_label("Back up now").focus();
            h.run_steps(1);
            let ctx = h.ctx.clone();
            if import {
                let path = std::path::Path::new("tests/fixtures/format1-1.0.2.json");
                h.state_mut().open_import_preview(&ctx, path);
            } else {
                let app = h.state_mut();
                app.remember_modal_opener(&ctx);
                app.confirm = Some(restore_confirmation(
                    &app.core,
                    RestoreSource::External("backup.db".into()),
                    None,
                ));
            }
            h.run_steps(3);
            h.key_press(Key::Escape);
            h.run_steps(4);
            assert!(h.get_by_label("Back up now").is_focused());
        }
    }

    #[test]
    fn backups_keep_unsaved_settings_on_success_and_failure() {
        let (_dir, mut core) = file_backed_core();
        core.set_screen(Screen::Settings);
        core.sync().unwrap();
        let mut app = AthenaApp::for_test(core);
        app.core.ui.settings.owner_name = "Unsaved owner".into();
        app.core.ui.settings.new_person = "Unsaved person".into();
        app.core.ui.settings.new_category = "Unsaved category".into();
        app.core.ui.settings.backup_error = Some("old error".into());
        let ctx = egui::Context::default();
        assert!(app.handle_create_backup(&ctx));
        assert_eq!(app.core.vm.backups.len(), 1);
        assert!(app.core.vm.last_operation.is_some());
        assert!(app.core.ui.settings.backup_error.is_none());
        // A regular file blocks directory creation without relying on OS permissions.
        let backups = app.core.store.dirs().backups_dir.clone();
        std::fs::remove_file(&app.core.vm.backups[0].path).unwrap();
        std::fs::remove_dir(&backups).unwrap();
        std::fs::write(&backups, b"blocked").unwrap();
        assert!(!app.handle_create_backup(&ctx));
        assert!(app.core.ui.settings.backup_error.is_some());
        assert_eq!(app.core.ui.settings.owner_name, "Unsaved owner");
        assert_eq!(app.core.ui.settings.new_person, "Unsaved person");
        assert_eq!(app.core.ui.settings.new_category, "Unsaved category");
        assert_eq!(
            app.toast.as_ref().unwrap().text,
            ToastKey::BackupFailed.text(Language::En)
        );
    }

    #[test]
    fn manual_backup_delete_confirms_refreshes_and_preserves_drafts() {
        use egui_kittest::kittest::Queryable;
        let (_dir, mut core) = file_backed_core();
        let path = core
            .apply(AppAction::CreateBackup)
            .unwrap()
            .notice
            .unwrap()
            .snapshot
            .unwrap();
        core.set_screen(Screen::Settings);
        core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Data;
        core.sync().unwrap();
        let mut h = polish_harness(core);
        h.run_steps(3);
        h.state_mut().core.ui.settings.owner_name = "Unsaved".into();
        let label = format!(
            "Delete\u{2026} {}",
            path.file_name().unwrap().to_str().unwrap()
        );
        h.get_by_label(&label).click();
        h.run_steps(3);
        assert!(path.exists());
        h.get_by_label(strings(Language::En).backup_delete).click();
        h.run_steps(4);
        assert!(!path.exists());
        assert!(h.state().core.vm.backups.is_empty());
        assert_eq!(h.state().core.ui.settings.owner_name, "Unsaved");
    }

    #[test]
    fn undo_toast_restores_and_clears() {
        use egui_kittest::kittest::Queryable;

        let mut core = onboarded(None);
        let mut draft = draft_with(vec![]);
        draft.title = "Coffee".into();
        core.create_transaction(draft).unwrap();

        let app = AthenaApp::for_test(core);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 900.0))
            .with_step_dt(0.05)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                app,
            );
        harness.run_steps(3);
        assert_eq!(harness.state().core.vm.transactions.len(), 1);

        // Delete from register and confirm -> Undo toast appears.
        harness.get_by_label("Delete Coffee").click();
        harness.run_steps(3);
        harness.get_by_label("Delete").click();
        harness.run_steps(4);
        assert!(harness.state().core.vm.transactions.is_empty());
        assert_eq!(harness.state().core.vm.trash_count, 1);
        assert!(harness.query_by_label("Undo").is_some());

        // Click Undo on the toast -> row returns and toast is gone.
        harness.get_by_label("Undo").click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.vm.transactions.len(), 1);
        assert_eq!(harness.state().core.vm.transactions[0].title, "Coffee");
        assert_eq!(harness.state().core.vm.trash_count, 0);
        assert!(harness.state().toast.is_none());

        // Delete again -> Ctrl+Z is blocked while a TextEdit has focus, and works otherwise.
        harness.get_by_label("Delete Coffee").click();
        harness.run_steps(3);
        harness.get_by_label("Delete").click();
        harness.run_steps(4);
        assert!(harness.state().core.vm.transactions.is_empty());

        harness
            .get_all_by_role(egui::Role::TextInput)
            .next()
            .unwrap()
            .focus();
        harness.run_steps(2);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        harness.run_steps(3);
        assert!(
            harness.state().core.vm.transactions.is_empty(),
            "Ctrl+Z does not steal focus from a text box"
        );
        assert!(harness.state().toast.is_some());

        harness.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        harness.run_steps(2);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        harness.run_steps(4);
        assert_eq!(harness.state().core.vm.transactions.len(), 1);
        assert!(harness.state().toast.is_none());
    }

    #[test]
    fn outcome_notice_is_kept_as_last_operation() {
        use actions::{OperationKind, OperationNotice, RefreshScope};
        use std::path::PathBuf;

        let mut core = onboarded(None);
        assert_eq!(core.vm.last_operation, None);

        let notice = OperationNotice {
            kind: OperationKind::BackupCreated,
            snapshot: Some(PathBuf::from("backups/manual-2026-11-16_10-00-00.db")),
            source: None,
        };
        core.record_outcome_notice(&ActionOutcome {
            created_goal: None,
            inverse: None,
            toast: None,
            refresh: RefreshScope::Backups,
            notice: Some(notice.clone()),
        });
        assert_eq!(core.vm.last_operation.as_ref(), Some(&notice));

        // Subsequent actions without a notice preserve last_operation for the session.
        core.create_transaction(draft_with(vec![])).unwrap();
        assert_eq!(core.vm.last_operation.as_ref(), Some(&notice));
    }

    #[test]
    fn a_long_title_is_truncated_not_allowed_to_push_the_amount_away() {
        use egui_kittest::kittest::Queryable;
        let mut core = onboarded(None);
        let mut state = quick_state(&core);
        state.title = "A very long title ".repeat(20);
        state.amount_input = "9,99".into();
        let draft = state.to_draft().unwrap();
        handle_quick_add(&mut core, &mut state, QuickAddEvent::Submit(draft));
        let app = AthenaApp::for_test(core);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1000.0, 800.0))
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                app,
            );
        harness.run_steps(4);
        let amount = harness.get_all_by_label("−R$ 9,99").next().unwrap().rect();
        assert!(
            amount.right() <= 1000.0,
            "the amount stays inside the window: {amount:?}"
        );
    }

    #[test]
    fn trash_screen_restore_and_delete_forever() {
        use egui_kittest::kittest::Queryable;

        let mut core = onboarded(None);
        for (idx, (title, stamp)) in [
            ("First", "2026-11-10T14:00:00Z"),
            ("Second", "2026-11-11T14:00:00Z"),
            ("Third", "2026-11-12T14:00:00Z"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut d = draft_with(vec![]);
            d.title = title.into();
            d.amount = Money::from_cents((i64::try_from(idx).unwrap() + 1) * 1_000);
            core.create_transaction(d).unwrap();
            let id = core.vm.transactions[0].id;
            core.delete_transaction(id).unwrap();
            core.store
                .db()
                .conn()
                .execute(
                    "UPDATE transactions SET deleted_at = ?1 WHERE id = ?2",
                    rusqlite::params![stamp, id],
                )
                .unwrap();
        }
        core.set_screen(Screen::Trash);
        core.sync().unwrap();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1100.0, 760.0))
            .with_step_dt(0.05)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);
        assert!(harness.query_by_label("Third").is_some());
        assert!(harness.query_by_label("Second").is_some());
        assert!(harness.query_by_label("First").is_some());

        // Restoring the middle row ("Second") brings it back and moves focus to the nearest row ("First").
        harness.get_by_label("Restore Second").click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.vm.trash.len(), 2);
        assert_eq!(harness.state().core.vm.trash_count, 2);
        assert!(harness.query_by_label("Second").is_none());
        assert!(
            harness.get_by_label("Restore First").is_focused(),
            "focus moves to the nearest remaining row after restore"
        );

        // Delete forever asks for confirmation, states permanence, and returns focus if cancelled.
        harness.get_by_label("Delete forever Third").focus();
        harness.run_steps(2);
        harness.key_press(Key::Enter);
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label("Delete this transaction forever?")
                .is_some()
        );
        assert!(
            harness
                .query_by_label(
                    "Third · R$ 30,00 will be permanently removed from Trash. This cannot be undone."
                )
                .is_some()
        );
        harness.key_press(Key::Escape);
        harness.run_steps(3);
        assert_eq!(harness.state().core.vm.trash.len(), 2);
        assert!(
            harness.get_by_label("Delete forever Third").is_focused(),
            "cancelling returns focus to the button that opened the dialog"
        );

        // Confirming permanent deletion purges Third and focuses the nearest remaining row ("First").
        harness.get_by_label("Delete forever Third").click();
        harness.run_steps(3);
        harness.get_by_label("Delete forever").click();
        harness.run_steps(4);
        assert_eq!(harness.state().core.vm.trash.len(), 1);
        assert_eq!(harness.state().core.vm.trash[0].tx.title, "First");
        assert!(
            harness.get_by_label("Restore First").is_focused(),
            "focus moves to the nearest remaining row after permanent deletion"
        );
    }

    #[test]
    fn empty_trash_states_count_and_confirms() {
        use egui_kittest::kittest::Queryable;

        let mut core = onboarded(None);
        for title in ["Alpha", "Beta"] {
            let mut d = draft_with(vec![]);
            d.title = title.into();
            core.create_transaction(d).unwrap();
            let id = core.vm.transactions[0].id;
            core.delete_transaction(id).unwrap();
        }
        core.set_screen(Screen::Trash);
        core.sync().unwrap();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1100.0, 760.0))
            .with_step_dt(0.05)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);
        harness.get_by_label("Empty trash").click();
        harness.run_steps(3);
        assert!(harness.query_by_label("Empty trash?").is_some());
        assert!(
            harness
                .query_by_label("Permanently delete 2 entries from Trash? This cannot be undone.")
                .is_some()
        );
        // Header button + confirm dialog button share label "Empty trash"; confirm is second.
        harness
            .get_all_by_label("Empty trash")
            .nth(1)
            .unwrap()
            .click();
        harness.run_steps(4);
        assert!(harness.state().core.vm.trash.is_empty());
        assert_eq!(harness.state().core.vm.trash_count, 0);
        assert!(harness.query_by_label("Trash is empty.").is_some());
    }

    #[test]
    fn record_button_prefills_quick_add_without_saving() {
        use egui_kittest::kittest::Queryable;

        let mut core = onboarded(None);
        crate::shared::test_fixtures::seed_month(core.store.db());
        core.vm
            .reload(&core.store, &core.ui, actions::RefreshScope::All)
            .unwrap();
        let before_count = core.vm.transactions.len();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 1150.0))
            .with_context_setup(theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);

        assert!(harness.query_by_label("Ana owes you R$ 25,00").is_some());
        harness.get_by_label("Record").click();
        harness.run_steps(3);

        let app = harness.state();
        assert_eq!(app.core.vm.transactions.len(), before_count);
        let qa = app.quick_add.as_ref().expect("quick add should open");
        assert_eq!(
            qa.kind,
            crate::domain::transaction::TransactionType::Settlement
        );
        assert_eq!(qa.title, "Ana → A3");
        assert_eq!(qa.amount_input, "25,00");
        assert_eq!(qa.date_text, "2026-11-11");
    }

    #[test]
    fn archived_party_offers_restore_instead_of_record() {
        use egui_kittest::kittest::Queryable;

        let mut core = onboarded(None);
        crate::shared::test_fixtures::seed_month(core.store.db());
        core.vm
            .reload(&core.store, &core.ui, actions::RefreshScope::All)
            .unwrap();
        let ana_id = core
            .vm
            .all_people
            .iter()
            .find(|p| p.name == "Ana")
            .unwrap()
            .id;
        core.apply(AppAction::ArchivePerson(ana_id)).unwrap();
        core.sync().unwrap();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 1150.0))
            .with_context_setup(theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);

        assert!(
            harness
                .query_by_label("Ana (archived) owes you R$ 25,00")
                .is_some()
        );
        assert!(harness.query_by_label("Record").is_none());
        harness.get_by_label("Restore Ana to record this").click();
        harness.run_steps(3);

        assert!(!harness.state().core.vm.person(ana_id).unwrap().archived);
        assert!(harness.state().quick_add.is_none());
        assert!(
            harness
                .query_by_label("Restore Ana to record this")
                .is_none()
        );
        assert!(harness.query_by_label("Record").is_some());
    }

    #[test]
    fn apply_preserves_committed_outcome_when_settlement_refresh_errors() {
        use crate::domain::transaction::{Allocation, TransactionType};
        use crate::ui::quick_add::QuickAddEvent;

        let mut core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        let (owner, partner) = crate::shared::test_fixtures::seed_people(core.store.db().conn());
        core.sync().unwrap();

        let mut d1 = draft_with(vec![Allocation {
            person_id: partner,
            amount: Money::from_cents(i64::MAX / 2 + 100),
        }]);
        d1.payer_id = Some(owner);
        core.apply(AppAction::SaveTransaction(d1)).unwrap();

        // Second large entry (Settlement) triggers settlement paid overflow while itself being valid.
        let mut d2 = draft_with(vec![Allocation {
            person_id: partner,
            amount: Money::from_cents(i64::MAX / 2 + 100),
        }]);
        d2.kind = TransactionType::Settlement;
        d2.payer_id = Some(owner);
        d2.category_id = None;
        d2.split_mode = None;
        d2.title = "Paid back".into();

        let mut qa = QuickAddState::new(
            &core
                .vm
                .active_people()
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
            core.vm.today,
            Language::En,
        );
        let (after, toast) = handle_quick_add(&mut core, &mut qa, QuickAddEvent::Submit(d2));
        assert_eq!(after, ModalAfter::Keep);
        assert!(
            qa.errors.is_empty(),
            "committed write must not report SaveFailed"
        );
        assert_eq!(toast, Some(ToastKey::TransactionSaved));
        assert_eq!(core.vm.transactions.len(), 2);
        assert!(core.vm.settlement_unavailable);
        assert!(!core.vm.refresh_failed);
    }

    #[test]
    fn apply_preserves_committed_outcome_when_required_refresh_errors() {
        let mut core = onboarded(None);
        let owner = core.owner().unwrap().id;
        let expense = draft_with(vec![Allocation {
            person_id: owner,
            amount: Money::from_cents(100),
        }]);
        core.create_transaction(expense).unwrap();

        let mut bad_profile = core.vm.profile.clone().unwrap();
        bad_profile.start_balance = Money::from_cents(i64::MIN);
        let outcome = core
            .apply(AppAction::SaveProfile(bad_profile))
            .expect("committed store action must return Ok(outcome) even when reload fails");
        assert_eq!(outcome.toast, Some(ToastKey::ProfileSaved));
        assert!(core.vm.refresh_failed);
        // Same-key retry suppression prevents a per-frame loop.
        assert!(!core.sync().unwrap());
    }

    #[test]
    fn repair_filter_clears_shared_filter_and_pending_search_in_ui() {
        use egui_kittest::kittest::Queryable;

        let mut core = onboarded(Some("Ana"));
        let owner = core.owner().unwrap().id;
        let partner = core
            .vm
            .all_people
            .iter()
            .find(|p| p.kind == PersonKind::Partner)
            .unwrap()
            .id;

        // Seed shared expense so the settlement card has >= 2 participants, plus an unassigned
        // expense in the previous month so it only shows when repair filter clears month+shared+search.
        let mut shared = draft_with(vec![
            Allocation {
                person_id: owner,
                amount: Money::from_cents(1_000),
            },
            Allocation {
                person_id: partner,
                amount: Money::from_cents(1_000),
            },
        ]);
        shared.title = "Shared Dinner".into();
        shared.payer_id = Some(owner);
        core.create_transaction(shared).unwrap();

        let mut unassigned = draft_with(vec![]);
        unassigned.title = "Orphan Receipt".into();
        unassigned.payer_id = Some(owner);
        unassigned.date = crate::shared::test_fixtures::date(2026, 10, 15);
        core.create_transaction(unassigned).unwrap();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 1150.0))
            .with_context_setup(theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);

        // Turn on Shared filter and type a pending search before debounce completes.
        harness.get_by_label("Shared").click();
        harness.run_steps(2);
        harness
            .get_all_by_role(egui::Role::TextInput)
            .next()
            .unwrap()
            .focus();
        harness.run_steps(1);
        harness
            .get_all_by_role(egui::Role::TextInput)
            .next()
            .unwrap()
            .type_text("nomatch");
        harness.run_steps(1);
        assert!(harness.state().core.ui.search_pending());

        // Click the settlement Unassigned exclusion link while Shared + pending search are active.
        harness
            .get_by_label("1 entry with no one responsible")
            .click();
        // Step well past SEARCH_DEBOUNCE (0.2s) to ensure pending search does not re-apply.
        harness.run_steps(10);

        let app = harness.state();
        assert!(!app.core.ui.filter.shared_only);
        assert!(!app.core.ui.search_pending());
        assert!(app.core.ui.filter.text.is_empty());
        assert!(app.core.ui.search_input.is_empty());
        assert!(harness.query_by_label("Orphan Receipt").is_some());
    }

    #[test]
    fn record_settlement_from_other_month_keeps_today_and_shows_mismatch_warning() {
        use egui_kittest::kittest::Queryable;

        let mut core = onboarded(Some("Ana"));
        // Seed an expense in October 2026 so October's settlement card has a balance to record,
        // while today is pinned to 2026-11-11.
        let ana_id = core
            .vm
            .all_people
            .iter()
            .find(|p| p.name == "Ana")
            .unwrap()
            .id;
        let owner_id = core.owner().unwrap().id;
        let mut oct_expense = draft_with(vec![Allocation {
            person_id: ana_id,
            amount: Money::from_cents(4_000),
        }]);
        oct_expense.title = "October Dinner".into();
        oct_expense.payer_id = Some(owner_id);
        oct_expense.date = crate::shared::test_fixtures::date(2026, 10, 10);
        core.create_transaction(oct_expense).unwrap();
        core.set_month(YearMonth::new(2026, 10).unwrap()).unwrap();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 1150.0))
            .with_context_setup(theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);

        harness.get_by_label("Record").click();
        harness.run_steps(3);

        {
            let app = harness.state();
            let qa = app.quick_add.as_ref().expect("quick add should open");
            assert_eq!(qa.date_text, "2026-11-11");
            assert_eq!(
                qa.settlement_month_mismatch,
                Some(YearMonth::new(2026, 10).unwrap())
            );
        }
        assert!(
            harness
                .query_by_label(
                    "Viewing October 2026; this settlement will be dated 2026-11-11 (today)."
                )
                .is_some()
        );

        // Saving keeps date = 2026-11-11 (today) cleanly without blocking.
        harness.get_by_label("Save").click();
        harness.run_steps(4);

        let app = harness.state();
        let qa = app
            .quick_add
            .as_ref()
            .expect("create mode keeps quick add open after save");
        assert!(qa.title.is_empty());
        assert_eq!(qa.settlement_month_mismatch, None);
        let stored = crate::infrastructure::db::transactions_repo::list(
            app.core.store.db().conn(),
            &crate::domain::transaction::TxFilter::default(),
        )
        .unwrap();
        let settlement = stored
            .iter()
            .find(|tx| tx.kind == TransactionType::Settlement)
            .expect("settlement should be saved");
        assert_eq!(
            settlement.date,
            crate::shared::test_fixtures::date(2026, 11, 11)
        );
    }

    #[test]
    fn purge_clears_matching_undo_toast_and_undo_failure_shows_toast() {
        use egui_kittest::kittest::Queryable;

        let mut core = onboarded(None);
        let mut d = draft_with(vec![]);
        d.title = "Coffee".into();
        core.create_transaction(d).unwrap();
        let tx_id = core.vm.transactions[0].id;

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 900.0))
            .with_step_dt(0.05)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);

        // Delete -> Undo toast for RestoreTransaction(tx_id) appears.
        harness.get_by_label("Delete Coffee").click();
        harness.run_steps(3);
        harness.get_by_label("Delete").click();
        harness.run_steps(3);
        assert!(matches!(
            harness.state().toast.as_ref().and_then(|t| t.action.as_ref()),
            Some(ToastAction::Undo(AppAction::RestoreTransaction(id))) if *id == tx_id
        ));

        // Switch to Trash and purge Coffee permanently -> stale Undo toast is cleared.
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Num4);
        harness.run_steps(3);
        harness.get_by_label("Delete forever Coffee").click();
        harness.run_steps(3);
        harness.get_by_label("Delete forever").click();
        harness.run_steps(3);
        assert!(
            harness.state().toast.is_none(),
            "purging must clear the stale RestoreTransaction Undo toast"
        );

        // Pressing Ctrl+Z after purge must not trigger a failed restore.
        harness.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        harness.run_steps(1);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        harness.run_steps(3);
        assert!(harness.state().toast.is_none());

        // Clicking a failing Undo action shows the localized undo-failed toast.
        harness.state_mut().toast = Some(Toast::pending("Deleted “Missing”.").with_action(Some(
            ToastAction::Undo(AppAction::RestoreTransaction(999_999)),
        )));
        harness.run_steps(2);
        harness.get_by_label("Undo").click();
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label("Could not undo that change.")
                .is_some()
        );
    }

    #[test]
    fn restore_person_duplicate_name_shows_specific_localized_error() {
        use egui_kittest::kittest::Queryable;

        let mut core = onboarded(Some("Ana"));
        let owner_id = core.owner().unwrap().id;
        let ana_id = core
            .vm
            .all_people
            .iter()
            .find(|p| p.kind == PersonKind::Partner)
            .unwrap()
            .id;
        let mut d = draft_with(vec![Allocation {
            person_id: ana_id,
            amount: Money::from_cents(2_000),
        }]);
        d.payer_id = Some(owner_id);
        core.create_transaction(d).unwrap();
        core.archive_person(ana_id).unwrap();
        core.save_person(Person {
            id: 0,
            name: "Ana".into(),
            kind: PersonKind::Other,
            archived: false,
            created_at: String::new(),
        })
        .unwrap();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1280.0, 1150.0))
            .with_context_setup(theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);

        harness.get_by_label("Restore Ana to record this").click();
        harness.run_steps(3);

        assert!(
            harness
                .query_by_label("Cannot restore Ana: an active person already uses that name.")
                .is_some()
        );
        assert!(harness.state().core.vm.person(ana_id).unwrap().archived);
    }

    fn file_backed_core() -> (tempfile::TempDir, AppCore) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let db = Db::open(&dirs.db_path).unwrap();
        let mut core = AppCore {
            store: Store::new(db, dirs),
            vm: ViewModel::new(Language::En),
            ui: UiState::new(YearMonth::new(2026, 11).unwrap()),
            media_epoch: 0,
        };
        core.vm.today = crate::shared::test_fixtures::today();
        core.complete_onboarding(OnboardingResult {
            display_name: "A3".into(),
            partner_name: Some("Ana".into()),
            start_balance: Money::from_cents(10_000),
            language: Language::En,
        })
        .unwrap();
        (dir, core)
    }

    #[test]
    fn restore_success_reloads_everything_and_resets_ui() {
        let (_dir, mut core) = file_backed_core();
        let mut before = draft_with(vec![]);
        before.title = "Saved Before Backup".into();
        core.create_transaction(before).unwrap();

        let backup = core.apply(AppAction::CreateBackup).unwrap();
        let backup_path = backup.notice.unwrap().snapshot.unwrap();

        let mut after = draft_with(vec![]);
        after.title = "Added After Backup".into();
        core.create_transaction(after).unwrap();
        assert_eq!(core.vm.transactions.len(), 2);

        core.ui.filter_shared();
        core.ui.edit_search("Added".into(), 1.0);
        core.set_screen(Screen::Settings);
        core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Data;
        core.sync().unwrap();

        let outcome = core
            .apply(AppAction::RestoreBackup(RestoreSource::Listed(backup_path)))
            .unwrap();
        let snapshot_path = outcome.notice.unwrap().snapshot.unwrap();

        assert_eq!(core.ui.screen, Screen::Month);
        assert!(!core.ui.has_filters());
        assert_eq!(
            core.ui.settings.page,
            crate::ui::screens::settings::SettingsPage::General
        );
        assert_eq!(core.vm.transactions.len(), 1);
        assert_eq!(core.vm.transactions[0].title, "Saved Before Backup");
        assert_eq!(
            core.vm
                .last_operation
                .as_ref()
                .and_then(|n| n.snapshot.as_ref()),
            Some(&snapshot_path)
        );
    }

    #[test]
    fn data_page_lists_backups_and_confirms_restore() {
        use egui_kittest::kittest::Queryable;

        let (_dir, mut core) = file_backed_core();
        let mut d1 = draft_with(vec![]);
        d1.title = "In Backup".into();
        core.create_transaction(d1).unwrap();
        let backup = core.apply(AppAction::CreateBackup).unwrap();
        let backup_path = backup.notice.unwrap().snapshot.unwrap();
        let backup_name = backup_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let mut d2 = draft_with(vec![]);
        d2.title = "After Backup".into();
        core.create_transaction(d2).unwrap();

        core.set_screen(Screen::Settings);
        core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Data;
        core.sync().unwrap();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1100.0, 900.0))
            .with_context_setup(theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);
        assert!(harness.query_by_label(backup_name.as_str()).is_some());

        harness
            .get_by_label(format!("Restore… {backup_name}").as_str())
            .click();
        harness.run_steps(3);

        let dialog = harness.state().confirm.as_ref().expect("confirm dialog");
        assert!(dialog.body.contains(&backup_name));
        assert!(dialog.body.contains("Replaces everything in Athena"));
        assert!(dialog.body.contains(&harness.state().core.vm.backups_dir));

        harness.get_by_label("Restore").click();
        harness.run_steps(4);

        let app = harness.state();
        assert_eq!(app.core.ui.screen, Screen::Month);
        assert_eq!(app.core.vm.transactions.len(), 1);
        assert_eq!(app.core.vm.transactions[0].title, "In Backup");
    }

    #[test]
    fn back_up_now_adds_a_row() {
        use egui_kittest::kittest::Queryable;

        let (_dir, mut core) = file_backed_core();
        core.set_screen(Screen::Settings);
        core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Data;
        core.sync().unwrap();
        assert!(core.vm.backups.is_empty());

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1100.0, 900.0))
            .with_context_setup(theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label("No backups in this folder yet.")
                .is_some()
        );

        harness.get_by_label("Back up now").click();
        harness.run_steps(4);

        let app = harness.state();
        assert_eq!(app.core.vm.backups.len(), 1);
        let name = app.core.vm.backups[0]
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.starts_with("manual-"));
        assert!(harness.query_by_label(name.as_str()).is_some());
        assert_eq!(
            app.core.ui.settings.page,
            crate::ui::screens::settings::SettingsPage::Data
        );
    }

    #[test]
    fn import_success_resets_ui_and_names_snapshot() {
        let (_dir, mut core) = file_backed_core();
        let mut live = draft_with(vec![]);
        live.title = "Existing Live".into();
        core.create_transaction(live).unwrap();

        let mut trashed = draft_with(vec![]);
        trashed.title = "Existing Trashed".into();
        core.create_transaction(trashed).unwrap();
        let trashed_id = core
            .vm
            .transactions
            .iter()
            .find(|t| t.title == "Existing Trashed")
            .unwrap()
            .id;
        core.apply(AppAction::DeleteTransaction(trashed_id))
            .unwrap();
        assert_eq!(core.vm.trash_count, 1);

        core.ui.filter_shared();
        core.ui.edit_search("Existing".into(), 1.0);
        core.set_screen(Screen::Settings);
        core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Data;
        core.sync().unwrap();

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/format1-1.0.2.json");
        let plan = crate::application::services::import::read_and_validate(&fixture).unwrap();
        let outcome = core
            .apply(AppAction::ImportProfile(Box::new(plan)))
            .unwrap();
        let snapshot_path = outcome.notice.unwrap().snapshot.unwrap();
        let snap_name = snapshot_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(snap_name.starts_with("pre-import-"), "{snap_name}");
        assert!(snapshot_path.exists());

        assert_eq!(core.ui.screen, Screen::Month);
        assert!(!core.ui.has_filters());
        assert_eq!(
            core.ui.settings.page,
            crate::ui::screens::settings::SettingsPage::General
        );
        assert_eq!(core.vm.trash_count, 0);
        assert_eq!(core.vm.transactions.len(), 3);
        let notice = core.vm.last_operation.as_ref().expect("import notice");
        assert_eq!(notice.kind, crate::app::actions::OperationKind::Imported);
        assert_eq!(notice.source.as_deref(), Some("format1-1.0.2.json"));
        assert_eq!(notice.snapshot.as_ref(), Some(&snapshot_path));
    }

    #[test]
    fn data_page_import_preview_confirms_and_reports_invalid_files() {
        use egui_kittest::kittest::Queryable;

        let (dir, mut core) = file_backed_core();
        let mut live = draft_with(vec![]);
        live.title = "Keep Until Import".into();
        core.create_transaction(live).unwrap();

        let mut trashed = draft_with(vec![]);
        trashed.title = "To Be Emptied".into();
        core.create_transaction(trashed).unwrap();
        let trashed_id = core
            .vm
            .transactions
            .iter()
            .find(|t| t.title == "To Be Emptied")
            .unwrap()
            .id;
        core.apply(AppAction::DeleteTransaction(trashed_id))
            .unwrap();

        let bad_path = dir.path().join("bad.json");
        std::fs::write(&bad_path, "{ not valid json").unwrap();

        core.set_screen(Screen::Settings);
        core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Data;
        core.sync().unwrap();
        let backups_dir = core.vm.backups_dir.clone();

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(1100.0, 900.0))
            .with_context_setup(theme::typography::install_test_fonts)
            .build_ui_state(
                |ui, app: &mut AthenaApp| {
                    ui.ctx()
                        .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                    theme::apply(ui.ctx(), app.core.vm.theme());
                    let style = ui.ctx().global_style();
                    ui.set_style(style);
                    app.show(ui);
                },
                AthenaApp::for_test(core),
            );
        harness.run_steps(3);
        assert!(harness.query_by_label("Import…").is_some());

        let ctx = harness.ctx.clone();
        harness.state_mut().open_import_preview(&ctx, &bad_path);
        harness.run_steps(3);

        let err_msg = harness
            .state()
            .core
            .ui
            .settings
            .import_error
            .clone()
            .expect("invalid file sets import_error");
        assert!(harness.state().import_preview.is_none());
        assert_eq!(harness.query_all_by_label(err_msg.as_str()).count(), 2);

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/format1-1.0.2.json");
        harness.state_mut().open_import_preview(&ctx, &fixture);
        harness.run_steps(3);

        assert!(harness.state().core.ui.settings.import_error.is_none());
        assert!(harness.state().import_preview.is_some());
        assert!(harness.query_by_label("Import profile?").is_some());
        assert!(
            harness
                .query_by_label("Entry times weren't recorded in this file.")
                .is_some()
        );
        let cur_cats = harness.state().core.vm.all_categories.len();
        assert!(cur_cats > 0);
        let expected_counts = format!(
            "Incoming: 2 people · 2 categories · 3 entries · 0 templates\nCurrent: 2 people · {cur_cats} categories · 1 entries · 0 templates · 1 in Trash (will be emptied)"
        );
        assert!(harness.query_by_label(expected_counts.as_str()).is_some());
        assert!(
            harness
                .query_by_label("Replaces everything in Athena, including Trash. JSON files do not include Trash; use a database backup to restore Trash.")
                .is_some()
        );
        let expected_snapshot = format!("A safety snapshot will be saved in:\n{backups_dir}");
        assert!(harness.query_by_label(expected_snapshot.as_str()).is_some());

        harness.get_by_label("Import").click();
        harness.run_steps(4);

        let app = harness.state();
        assert!(app.import_preview.is_none());
        assert_eq!(app.core.ui.screen, Screen::Month);
        assert_eq!(app.core.vm.transactions.len(), 3);
        assert_eq!(app.core.vm.trash_count, 0);
        assert_eq!(app.core.vm.owner_name(), Some("Owner"));
        let toast_text = app
            .toast
            .as_ref()
            .expect("import toast should appear")
            .text
            .as_str();
        assert!(
            toast_text.starts_with("Imported format1-1.0.2.json. Safety snapshot: pre-import-"),
            "{toast_text}"
        );
    }

    #[test]
    fn failed_import_on_settings_refreshes_backups_list() {
        let (_dir, mut core) = file_backed_core();
        core.set_screen(Screen::Settings);
        core.ui.settings.page = crate::ui::screens::settings::SettingsPage::Data;
        core.sync().unwrap();
        assert!(core.vm.backups.is_empty());

        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/format1-1.0.2.json");
        let mut bad_plan =
            crate::application::services::import::read_and_validate(&fixture).unwrap();
        // Force a SQLite constraint / integrity failure after the pre-import snapshot is written.
        bad_plan
            .transactions
            .iter_mut()
            .find(|t| !t.allocations.is_empty())
            .expect("fixture has an allocated transaction")
            .allocations[0]
            .amount = Money::from_cents(1);

        let err = core
            .apply(AppAction::ImportProfile(Box::new(bad_plan)))
            .unwrap_err();
        assert!(matches!(err, AppError::Import(_)));
        assert_eq!(core.ui.screen, Screen::Settings);
        assert_eq!(core.vm.backups.len(), 1);
        assert_eq!(
            core.vm.backups[0].kind,
            crate::infrastructure::db::backup::BackupKind::PreImport
        );
    }
}
