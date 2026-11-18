//! Modal, confirmation, and settings-action orchestration around [`AppCore`].

use chrono::{DateTime, Local, Utc};

use crate::application::services::import::ImportPlan;
use crate::domain::transaction::{Transaction, TxDraft};
use crate::i18n::{
    PluralKey, ToastKey, backup_delete_error_message, format_date, format_import_errors, plural,
    restore_error_message, strings,
};
use crate::infrastructure::db::{
    restore::RestoreSource, templates_repo, transactions_repo, trash_repo,
};
use crate::shared::error::{AppError, Result};
use crate::ui::components::confirm_dialog::ConfirmDialog;
use crate::ui::components::import_preview::ImportPreviewDialog;
use crate::ui::components::toast::ToastAction;
use crate::ui::quick_add::{
    DuplicateConsent, DuplicateExample, PendingSave, PromptError, QuickAddError, QuickAddEvent,
    QuickAddMode, QuickAddState,
};
use crate::ui::screens::settings::SettingsError;

use super::AppCore;
use super::actions::{AppAction, OperationKind};

/// Whether the quick-add modal stays up after handling an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalAfter {
    Keep,
    Close,
}

/// Apply what the quick-add modal asked for. A new entry saves and leaves the modal open, reset
/// for the next one; an edit saves and closes it. A refused save keeps the modal and its input.
///
/// Every normal create or edit first looks for a possible duplicate here, after validation and
/// before `Store::apply`; a match asks for explicit consent instead of saving.
pub fn handle_quick_add(
    core: &mut AppCore,
    state: &mut QuickAddState,
    event: QuickAddEvent,
) -> (ModalAfter, Option<ToastKey>) {
    match event {
        QuickAddEvent::Close | QuickAddEvent::QuitConfirmed => (ModalAfter::Close, None),
        QuickAddEvent::Submit(draft) => {
            state.duplicate = None;
            let save = match state.mode {
                QuickAddMode::Create => state
                    .template_id
                    .map_or(PendingSave::Create, PendingSave::CreateFromTemplate),
                QuickAddMode::Edit(id) => PendingSave::Edit(id),
                // Template edits submit through `SubmitTemplate`, never as an entry.
                QuickAddMode::Template(_) => return (ModalAfter::Keep, None),
            };
            guarded_save(core, state, save, draft)
        }
        QuickAddEvent::ConfirmDuplicate => confirm_duplicate(core, state),
        QuickAddEvent::SaveAsTemplate(draft) => match core.apply(AppAction::SaveTemplate(draft)) {
            Ok(outcome) => {
                state.template_prompt = None;
                (ModalAfter::Keep, outcome.toast)
            }
            Err(err) => {
                tracing::warn!("could not save the template: {err}");
                if let Some(prompt) = state.template_prompt.as_mut() {
                    prompt.error = Some(match err {
                        AppError::TemplateInvalid(errors) => errors
                            .into_iter()
                            .next()
                            .map_or(PromptError::Failed, PromptError::Template),
                        _ => PromptError::Failed,
                    });
                }
                (ModalAfter::Keep, None)
            }
        },
        QuickAddEvent::SubmitTemplate(draft) => {
            let QuickAddMode::Template(id) = state.mode else {
                return (ModalAfter::Keep, None);
            };
            match core.apply(AppAction::UpdateTemplate(id, draft)) {
                Ok(outcome) => (ModalAfter::Close, outcome.toast),
                Err(err) => {
                    tracing::warn!("could not update the template: {err}");
                    state.errors = match err {
                        AppError::TemplateInvalid(errors) => {
                            errors.into_iter().map(QuickAddError::Template).collect()
                        }
                        _ => vec![QuickAddError::Save],
                    };
                    state.focus_first_error();
                    (ModalAfter::Keep, None)
                }
            }
        }
    }
}

fn duplicates_of(core: &AppCore, save: PendingSave, draft: &TxDraft) -> Result<Vec<Transaction>> {
    let exclude = match save {
        PendingSave::Edit(id) => Some(id),
        PendingSave::Create | PendingSave::CreateFromTemplate(_) => None,
    };
    transactions_repo::find_duplicates(
        core.store.db().conn(),
        draft.date,
        draft.amount.cents(),
        &draft.title,
        exclude,
    )
}

fn consent_for(
    core: &AppCore,
    save: PendingSave,
    draft: &TxDraft,
    matches: &[Transaction],
) -> DuplicateConsent {
    DuplicateConsent {
        save,
        draft: draft.clone(),
        match_ids: matches.iter().map(|t| t.id).collect(),
        first: DuplicateExample {
            date: matches[0].date,
            amount: matches[0].amount,
            title: matches[0].title.clone(),
        },
        count: matches.len(),
        store_version: core.store.data_version(),
    }
}

/// Look for duplicates, then save or ask. A lookup error blocks saving and keeps the form.
fn guarded_save(
    core: &mut AppCore,
    state: &mut QuickAddState,
    save: PendingSave,
    draft: TxDraft,
) -> (ModalAfter, Option<ToastKey>) {
    match duplicates_of(core, save, &draft) {
        Ok(matches) if matches.is_empty() => commit_save(core, state, save, draft),
        Ok(matches) => {
            state.duplicate = Some(consent_for(core, save, &draft, &matches));
            state.focus_prompt = true;
            (ModalAfter::Keep, None)
        }
        Err(err) => {
            tracing::warn!("duplicate check failed: {err}");
            state.errors = vec![QuickAddError::DuplicateLookup];
            (ModalAfter::Keep, None)
        }
    }
}

/// Save anyway: only for exactly the draft and match set the user was shown. If the store changed
/// since, look again and ask again when the set differs.
fn confirm_duplicate(
    core: &mut AppCore,
    state: &mut QuickAddState,
) -> (ModalAfter, Option<ToastKey>) {
    let Some(consent) = state.duplicate.take() else {
        return (ModalAfter::Keep, None);
    };
    if state.to_draft().ok().as_ref() != Some(&consent.draft) {
        return (ModalAfter::Keep, None);
    }
    if consent.store_version != core.store.data_version() {
        match duplicates_of(core, consent.save, &consent.draft) {
            Ok(matches) => {
                let ids: Vec<i64> = matches.iter().map(|t| t.id).collect();
                if ids != consent.match_ids {
                    if matches.is_empty() {
                        return commit_save(core, state, consent.save, consent.draft);
                    }
                    state.duplicate =
                        Some(consent_for(core, consent.save, &consent.draft, &matches));
                    state.focus_prompt = true;
                    return (ModalAfter::Keep, None);
                }
            }
            Err(err) => {
                tracing::warn!("duplicate re-check failed: {err}");
                state.errors = vec![QuickAddError::DuplicateLookup];
                return (ModalAfter::Keep, None);
            }
        }
    }
    commit_save(core, state, consent.save, consent.draft)
}

fn commit_save(
    core: &mut AppCore,
    state: &mut QuickAddState,
    save: PendingSave,
    draft: TxDraft,
) -> (ModalAfter, Option<ToastKey>) {
    let action = match save {
        PendingSave::Create => AppAction::SaveTransaction(draft),
        PendingSave::CreateFromTemplate(id) => AppAction::SaveTransactionFromTemplate(draft, id),
        PendingSave::Edit(id) => AppAction::UpdateTransaction(id, draft),
    };
    match core.apply(action) {
        Ok(outcome) => {
            state.duplicate = None;
            match save {
                PendingSave::Edit(_) => (ModalAfter::Close, outcome.toast),
                PendingSave::Create | PendingSave::CreateFromTemplate(_) => {
                    if state.from_line {
                        state.reset_for_line(core.vm.today);
                    } else {
                        state.reset_for_next();
                    }
                    (ModalAfter::Keep, outcome.toast)
                }
            }
        }
        Err(err) => {
            tracing::warn!("quick add could not save: {err}");
            state.duplicate = None;
            state.errors = vec![QuickAddError::Save];
            (ModalAfter::Keep, None)
        }
    }
}

/// Open the quick-add form on a stored transaction, prefilled in the mode it was saved with.
///
/// # Errors
/// [`crate::shared::error::AppError::NotFound`] when the transaction no longer exists.
pub fn begin_edit(core: &AppCore, id: i64) -> Result<QuickAddState> {
    let tx = core.store.transaction(id)?;
    let people: Vec<_> = core.vm.active_people().into_iter().cloned().collect();
    Ok(QuickAddState::from_transaction(
        &tx,
        &people,
        core.vm.language(),
    ))
}

/// Open the template editor: the Quick Add form on a stored template, without a date.
///
/// # Errors
/// [`crate::shared::error::AppError::NotFound`] when the template no longer exists.
pub fn begin_edit_template(core: &AppCore, id: i64) -> Result<QuickAddState> {
    let template = core
        .vm
        .templates
        .iter()
        .find(|t| t.id == id)
        .ok_or(AppError::NotFound("template"))?;
    let people: Vec<_> = core.vm.active_people().into_iter().cloned().collect();
    Ok(QuickAddState::for_template(
        template,
        &people,
        &core.vm.all_people,
        &core.vm.all_categories,
        core.vm.today,
        core.vm.language(),
    ))
}

/// The "delete this template?" dialog, naming it.
///
/// # Errors
/// [`crate::shared::error::AppError::NotFound`] when the template no longer exists.
pub fn delete_template_confirmation(core: &AppCore, id: i64) -> Result<ConfirmDialog> {
    let template = core
        .vm
        .templates
        .iter()
        .find(|t| t.id == id)
        .ok_or(AppError::NotFound("template"))?;
    let s = strings(core.vm.language());
    Ok(ConfirmDialog {
        title: s.settings_template_delete_title.to_string(),
        body: s
            .settings_template_delete_body
            .replace("{name}", &template.draft.name),
        confirm_label: s.delete.to_string(),
        danger: true,
        action: AppAction::DeleteTemplate(id),
    })
}

/// The "delete this?" dialog for a transaction, naming it and its amount.
///
/// # Errors
/// [`crate::shared::error::AppError::NotFound`] when the transaction no longer exists.
pub fn delete_confirmation(core: &AppCore, id: i64) -> Result<ConfirmDialog> {
    let tx = core.store.transaction(id)?;
    let s = strings(core.vm.language());
    Ok(ConfirmDialog {
        title: s.confirm_delete_title.to_string(),
        body: s
            .confirm_delete_body
            .replace("{title}", &tx.title)
            .replace("{amount}", &tx.amount.format_brl()),
        confirm_label: s.delete.to_string(),
        danger: true,
        action: AppAction::DeleteTransaction(id),
    })
}

/// The "delete forever?" dialog for a trashed transaction.
///
/// # Errors
/// [`crate::shared::error::AppError::NotFound`] when no deleted transaction matches `id`.
pub fn purge_confirmation(core: &AppCore, id: i64) -> Result<ConfirmDialog> {
    let item = crate::infrastructure::db::trash_repo::get_deleted(core.store.db().conn(), id)?;
    let s = strings(core.vm.language());
    Ok(ConfirmDialog {
        title: s.confirm_purge_title.to_string(),
        body: s
            .confirm_purge_body
            .replace("{title}", &item.tx.title)
            .replace("{amount}", &item.tx.amount.format_brl()),
        confirm_label: s.delete_forever.to_string(),
        danger: true,
        action: AppAction::PurgeTransaction(id),
    })
}

/// The "empty trash?" dialog naming the count that will be permanently deleted.
#[must_use]
pub fn empty_trash_confirmation(core: &AppCore) -> ConfirmDialog {
    let language = core.vm.language();
    let s = strings(language);
    let count = plural(PluralKey::Entries, core.vm.trash_count, language);
    ConfirmDialog {
        title: s.confirm_empty_trash_title.to_string(),
        body: s.confirm_empty_trash_body.replace("{count}", &count),
        confirm_label: s.empty_trash.to_string(),
        danger: true,
        action: AppAction::EmptyTrash,
    }
}

/// The "delete this backup?" dialog for a manual backup the user picked from the list.
#[must_use]
pub fn delete_backup_confirmation(core: &AppCore, path: &std::path::Path) -> ConfirmDialog {
    let s = strings(core.vm.language());
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    ConfirmDialog {
        title: s.confirm_delete_backup_title.to_string(),
        body: s.confirm_delete_backup_body.replace("{name}", &name),
        confirm_label: s.backup_delete.to_string(),
        danger: true,
        action: AppAction::DeleteBackup(path.to_path_buf()),
    }
}

/// The "restore this backup?" dialog naming the source, local backup date, full-replacement
/// consequence, and safety-snapshot folder (`§9.3`).
#[must_use]
pub fn restore_confirmation(
    core: &AppCore,
    source: RestoreSource,
    modified_utc: Option<DateTime<Utc>>,
) -> ConfirmDialog {
    let language = core.vm.language();
    let s = strings(language);
    let source_label = match &source {
        RestoreSource::Listed(path) => path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
        RestoreSource::External(path) => path.display().to_string(),
    };
    let date_label = modified_utc.map_or_else(
        || s.unknown_date.to_string(),
        |utc| {
            let local = utc.with_timezone(&Local);
            format!(
                "{} {}",
                format_date(local.date_naive(), language),
                local.format("%H:%M")
            )
        },
    );
    let snapshot_dir = if core.vm.backups_dir.is_empty() {
        core.store.dirs().backups_dir.display().to_string()
    } else {
        core.vm.backups_dir.clone()
    };
    ConfirmDialog {
        title: s.confirm_restore_title.to_string(),
        body: s
            .confirm_restore_body
            .replace("{source}", &source_label)
            .replace("{date}", &date_label)
            .replace("{snapshot_dir}", &snapshot_dir),
        confirm_label: s.restore.to_string(),
        danger: true,
        action: AppAction::RestoreBackup(source),
    }
}

/// Build the import preview modal comparing the validated `plan` against the current database
/// counts (including Trash that will be emptied).
#[must_use]
pub fn import_preview_dialog(core: &AppCore, plan: ImportPlan) -> ImportPreviewDialog {
    let conn = core.store.db().conn();
    let current_transactions = transactions_repo::count_live(conn).unwrap_or(0);
    let current_templates = templates_repo::count(conn).unwrap_or(0);
    let current_trash = trash_repo::count(conn).unwrap_or(core.vm.trash_count);
    let snapshot_dir = if core.vm.backups_dir.is_empty() {
        core.store.dirs().backups_dir.display().to_string()
    } else {
        core.vm.backups_dir.clone()
    };
    ImportPreviewDialog {
        plan: Box::new(plan),
        current_people: core.vm.all_people.len(),
        current_categories: core.vm.all_categories.len(),
        current_transactions,
        current_templates,
        current_trash,
        snapshot_dir,
    }
}

/// Apply an action the user confirmed, returning its toast text and an optional undo action
/// (offered for `DeleteTransaction` only). Restore and import failures surface localized error
/// messages.
pub fn apply_confirmed_outcome(
    core: &mut AppCore,
    action: AppAction,
) -> Option<(String, Option<ToastAction>)> {
    let is_delete = matches!(action, AppAction::DeleteTransaction(_));
    let is_restore = matches!(action, AppAction::RestoreBackup(_));
    let is_import = matches!(action, AppAction::ImportProfile(_));
    let is_backup_delete = matches!(action, AppAction::DeleteBackup(_));
    let deleted_title = match &action {
        AppAction::DeleteTransaction(id) => core.store.transaction(*id).ok().map(|tx| tx.title),
        _ => None,
    };
    match core.apply(action) {
        Ok(outcome) => {
            if is_backup_delete {
                core.ui.settings.backup_error = None;
            }
            let language = core.vm.language();
            let text = if let Some(title) = deleted_title {
                Some(
                    strings(language)
                        .toast_deleted_named
                        .replace("{title}", &title),
                )
            } else if let Some(notice) = &outcome.notice
                && notice.kind == OperationKind::Restored
                && let Some(snapshot) = &notice.snapshot
            {
                let snap_name = snapshot.file_name().map_or_else(
                    || snapshot.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                Some(
                    strings(language)
                        .toast_restored_named
                        .replace("{snapshot}", &snap_name),
                )
            } else if let Some(notice) = &outcome.notice
                && notice.kind == OperationKind::Imported
                && let Some(snapshot) = &notice.snapshot
            {
                let snap_name = snapshot.file_name().map_or_else(
                    || snapshot.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                let src_name = notice.source.as_deref().unwrap_or("");
                Some(
                    strings(language)
                        .toast_imported_named
                        .replace("{source}", src_name)
                        .replace("{snapshot}", &snap_name),
                )
            } else {
                outcome.toast.map(|key| key.text(language).to_string())
            }?;
            let toast_action = is_delete
                .then_some(outcome.inverse)
                .flatten()
                .map(ToastAction::Undo);
            Some((text, toast_action))
        }
        Err(err) => record_confirmed_error(core, is_restore, is_import, is_backup_delete, &err),
    }
}

fn record_confirmed_error(
    core: &mut AppCore,
    is_restore: bool,
    is_import: bool,
    is_backup_delete: bool,
    err: &AppError,
) -> Option<(String, Option<ToastAction>)> {
    use crate::shared::error::{ImportError, RestoreError};

    let language = core.vm.language();
    match err {
        AppError::Restore(restore_err) => {
            tracing::warn!("restore failed: {restore_err}");
            let message = restore_error_message(restore_err, language);
            core.ui.settings.restore_error = Some(message.clone());
            Some((message, None))
        }
        AppError::Import(errs) => {
            tracing::warn!("import failed: {} error(s)", errs.len());
            let message = format_import_errors(errs, language);
            core.ui.settings.import_error = Some(message.clone());
            Some((message, None))
        }
        other if is_restore => {
            tracing::warn!("restore failed: {other}");
            let message = restore_error_message(&RestoreError::Failed(other.to_string()), language);
            core.ui.settings.restore_error = Some(message.clone());
            Some((message, None))
        }
        other if is_import => {
            tracing::warn!("import failed: {other}");
            let message = format_import_errors(
                &[ImportError::VerificationFailed(other.to_string())],
                language,
            );
            core.ui.settings.import_error = Some(message.clone());
            Some((message, None))
        }
        other if is_backup_delete => {
            tracing::warn!("could not delete the backup: {other}");
            let message = backup_delete_error_message(other, language);
            core.ui.settings.backup_error = Some(message.clone());
            Some((message, None))
        }
        other => {
            tracing::warn!("confirmed action failed: {other}");
            None
        }
    }
}

/// Apply an action the user confirmed. Returns the toast text, which names a deleted entry.
/// A failure is logged and shows nothing.
pub fn apply_confirmed(core: &mut AppCore, action: AppAction) -> Option<String> {
    apply_confirmed_outcome(core, action).map(|(text, _)| text)
}

/// Apply an action raised on the settings screen. Success drops the screen's unsaved edits so it
/// reloads from what is stored; a refusal is remembered so the screen can say so.
/// Returns the toast text and whether the action succeeded.
pub fn apply_settings_action(core: &mut AppCore, action: AppAction) -> (Option<String>, bool) {
    match core.apply(action) {
        Ok(outcome) => {
            core.ui.settings.reset();
            let text = outcome
                .toast
                .map(|key| key.text(core.vm.language()).to_string());
            (text, true)
        }
        Err(err) => {
            tracing::warn!("settings change refused: {err}");
            core.ui.settings.error = Some(match err {
                AppError::CategoryScopeInUse => SettingsError::ScopeInUse,
                _ => SettingsError::SaveRejected,
            });
            (None, false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::db::Db;

    #[test]
    fn scope_in_use_is_routed_to_the_specific_settings_error() {
        use crate::domain::category::CategoryScope;
        use crate::shared::test_fixtures::{seed_month, seed_people};
        let db = Db::open_in_memory().unwrap();
        seed_people(db.conn());
        seed_month(&db);
        let mut core = AppCore::empty_for_test(db);
        let id = core
            .vm
            .transactions
            .iter()
            .find_map(|t| t.category_id)
            .unwrap();
        let mut category = core.vm.category(id).unwrap().clone();
        category.applies_to = CategoryScope::Income;
        let (_, ok) = apply_settings_action(&mut core, AppAction::SaveCategory(category));
        assert!(!ok);
        assert_eq!(core.ui.settings.error, Some(SettingsError::ScopeInUse));
    }

    #[test]
    fn untyped_restore_and_import_errors_still_surface_in_toast_and_settings() {
        let mut core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
        let io_err = AppError::Io(std::io::Error::other("disk full"));

        let restore_toast = record_confirmed_error(&mut core, true, false, false, &io_err);
        let (msg, _) = restore_toast.expect("restore error must produce toast");
        assert!(msg.contains("disk full"), "{msg}");
        assert_eq!(
            core.ui.settings.restore_error.as_deref(),
            Some(msg.as_str())
        );

        let import_toast = record_confirmed_error(&mut core, false, true, false, &io_err);
        let (msg, _) = import_toast.expect("import error must produce toast");
        assert!(msg.contains("disk full"), "{msg}");
        assert_eq!(core.ui.settings.import_error.as_deref(), Some(msg.as_str()));
    }
}
