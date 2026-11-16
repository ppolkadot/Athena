//! Modal, confirmation, and settings-action orchestration around [`AppCore`].

use crate::i18n::{PluralKey, ToastKey, plural, strings};
use crate::shared::error::Result;
use crate::ui::components::confirm_dialog::ConfirmDialog;
use crate::ui::components::toast::ToastAction;
use crate::ui::quick_add::{QuickAddError, QuickAddEvent, QuickAddMode, QuickAddState};
use crate::ui::screens::settings::SettingsError;

use super::AppCore;
use super::actions::AppAction;

/// Whether the quick-add modal stays up after handling an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalAfter {
    Keep,
    Close,
}

/// Apply what the quick-add modal asked for. A new entry saves and leaves the modal open, reset
/// for the next one; an edit saves and closes it. A refused save keeps the modal and its input.
pub fn handle_quick_add(
    core: &mut AppCore,
    state: &mut QuickAddState,
    event: QuickAddEvent,
) -> (ModalAfter, Option<ToastKey>) {
    let draft = match event {
        QuickAddEvent::Close => return (ModalAfter::Close, None),
        QuickAddEvent::Submit(draft) => draft,
    };
    let action = match state.mode {
        QuickAddMode::Create => AppAction::SaveTransaction(draft),
        QuickAddMode::Edit(id) => AppAction::UpdateTransaction(id, draft),
    };
    match core.apply(action) {
        Ok(outcome) => match state.mode {
            QuickAddMode::Create => {
                state.reset_for_next();
                (ModalAfter::Keep, outcome.toast)
            }
            QuickAddMode::Edit(_) => (ModalAfter::Close, outcome.toast),
        },
        Err(err) => {
            tracing::warn!("quick add could not save: {err}");
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

/// Apply an action the user confirmed, returning its toast text and an optional undo action
/// (offered for `DeleteTransaction` only). A failure is logged and shows nothing.
pub fn apply_confirmed_outcome(
    core: &mut AppCore,
    action: AppAction,
) -> Option<(String, Option<ToastAction>)> {
    let is_delete = matches!(action, AppAction::DeleteTransaction(_));
    let deleted_title = match &action {
        AppAction::DeleteTransaction(id) => core.store.transaction(*id).ok().map(|tx| tx.title),
        _ => None,
    };
    match core.apply(action) {
        Ok(outcome) => {
            let language = core.vm.language();
            let text = match deleted_title {
                Some(title) => Some(
                    strings(language)
                        .toast_deleted_named
                        .replace("{title}", &title),
                ),
                None => outcome.toast.map(|key| key.text(language).to_string()),
            }?;
            let toast_action = is_delete
                .then_some(outcome.inverse)
                .flatten()
                .map(ToastAction::Undo);
            Some((text, toast_action))
        }
        Err(err) => {
            tracing::warn!("confirmed action failed: {err}");
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
            core.ui.settings.error = Some(SettingsError::SaveRejected);
            (None, false)
        }
    }
}
