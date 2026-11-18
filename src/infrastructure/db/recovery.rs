//! Startup recovery for an unreadable or missing live database (`§11`).
//!
//! Prepare a standalone replacement before moving the old files, persist a validated relative
//! intent (`recovery-intent.json`) with byte-identity checksums, move the original database and
//! its `SQLite` sidecars (`-wal`, `-shm`, `-journal`) aside without clobbering, and publish the
//! replacement without overwriting an occupied destination.
//!
//! # Lock boundary and durability limits
//!
//! - **Single-instance lock (`athena.lock`) vs. external writers:** `athena.lock` serializes
//!   concurrent Athena processes, including recovery mode, for the entire process lifetime. It
//!   does **not** serialize external non-Athena writers (indexers, backup/sync tools, or manual
//!   file edits). Recovery mitigates concurrent external changes by re-verifying file identities,
//!   checksums, and absent live/staged sidecar slots at mutation and verification boundaries,
//!   failing closed into [`AppError::RecoveryConflict`] without clobbering.
//! - **Process termination vs. power loss:** Staged database bytes and `recovery-intent.json`
//!   contents are flushed via `File::sync_all()` before rename/publish, so abrupt process
//!   termination (`TerminateProcess` / `SIGKILL`) under `athena.lock` reconciles deterministically
//!   on next startup (including Unix `hard_link` + `remove_file` alias reconciliation verified by
//!   physical file identity). Parent directory entries are not explicitly fsynced, so sudden
//!   power loss or storage write-cache loss can leave directory metadata out of sync with flushed
//!   file contents.

use std::collections::HashSet;
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Db, backup, restore};
use crate::infrastructure::config::AppDirs;
use crate::shared::error::{AppError, RestoreError, Result};

/// Prefix of the files a recovery restore moves aside.
pub const BROKEN_PREFIX: &str = "broken-";
const STAGED_PREFIX: &str = "recovery-";
const INTENT_FILE_NAME: &str = "recovery-intent.json";
const INTENT_VERSION: u32 = 1;
const SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];
const ALL_SUFFIXES: [&str; 4] = ["", "-wal", "-shm", "-journal"];

/// State of the live database path before any scan for recovery remnants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LivePathState {
    Missing,
    EmptyFile,
}

/// Result of probing the data folder at startup (`§11`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartupProbe {
    /// Confirmed absent `athena.db`, zero remnants/backups, and every folder scan succeeded.
    FirstLaunch,
    /// `athena.db` is confirmed absent or zero-byte, with recovery evidence or a scan error.
    MissingOrEmpty {
        live: LivePathState,
        scan_error: Option<String>,
    },
    /// `athena.db` metadata could not be read or points to a non-regular file (directory/link).
    IndeterminateLive(String),
    /// An interrupted recovery left a corrupt intent or conflicting files on disk.
    Conflict(String),
    /// `athena.db` is a non-empty regular file with no active recovery conflict.
    ReadyToOpen,
}

/// Outcome of reconciling any leftover `recovery-intent.json` from an interrupted recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReconcileOutcome {
    Clean,
    Published(PathBuf),
    RolledBack,
    Conflict(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IntentPhase {
    MovingOriginals,
    PublishingCandidate,
    Published,
    RollingBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginalEntry {
    suffix: String,
    len: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryIntent {
    version: u32,
    phase: IntentPhase,
    /// Relative filename inside `dirs.root` (e.g. `broken-2026-11-16_12-00-00.db`).
    broken_name: String,
    /// Relative filename inside `dirs.staging_dir` (e.g. `recovery-2026-11-16_12-00-00.db`).
    candidate_name: String,
    candidate_len: u64,
    candidate_sha256: String,
    originals: Vec<OriginalEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileSlot {
    Missing,
    Regular { len: u64, sha256: String },
    NonRegular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum HookStep {
    AfterWriteIntent,
    BeforeMoveOriginal(&'static str),
    AfterMoveOriginal(&'static str),
    BeforeMarkPublishing,
    BeforePublish,
    AfterPublishBeforeMark,
    AfterMarkPublished,
    BeforeRollbackPhase,
    BeforeRollbackSuffix(&'static str),
    AfterRollbackSuffix(&'static str),
}

#[allow(dead_code)]
enum HookAction {
    Continue,
    FailWithRollback(String),
    InterruptWithoutRollback,
}

#[allow(dead_code)]
enum HookPublish {
    Default,
    FailWithRollback(String),
    InterruptWithoutRollback,
}

#[cfg(not(test))]
struct RecoveryHooks;

#[cfg(not(test))]
impl RecoveryHooks {
    fn step(_step: HookStep) -> HookAction {
        HookAction::Continue
    }

    #[allow(clippy::unused_self)]
    fn publish(&mut self, _staged: &Path, _live: &Path) -> HookPublish {
        HookPublish::Default
    }
}

#[cfg(test)]
type StepHookFn = Box<dyn FnMut(&AppDirs, HookStep) -> HookAction>;
#[cfg(test)]
type PublishHookFn = Box<dyn FnMut(&Path, &Path) -> HookPublish>;

#[cfg(test)]
#[derive(Default)]
struct RecoveryHooks {
    step_cb: Option<StepHookFn>,
    publish_cb: Option<PublishHookFn>,
}

#[cfg(test)]
impl RecoveryHooks {
    fn with_step(f: impl FnMut(&AppDirs, HookStep) -> HookAction + 'static) -> Self {
        Self {
            step_cb: Some(Box::new(f)),
            publish_cb: None,
        }
    }

    fn with_publish(f: impl FnMut(&Path, &Path) -> HookPublish + 'static) -> Self {
        Self {
            step_cb: None,
            publish_cb: Some(Box::new(f)),
        }
    }

    fn step(&mut self, dirs: &AppDirs, step: HookStep) -> HookAction {
        self.step_cb
            .as_mut()
            .map_or(HookAction::Continue, |cb| cb(dirs, step))
    }

    fn publish(&mut self, staged: &Path, live: &Path) -> HookPublish {
        self.publish_cb
            .as_mut()
            .map_or(HookPublish::Default, |cb| cb(staged, live))
    }
}

fn default_hooks() -> RecoveryHooks {
    #[cfg(not(test))]
    {
        RecoveryHooks
    }
    #[cfg(test)]
    {
        RecoveryHooks::default()
    }
}

#[cfg(not(test))]
fn run_hook_step(_hooks: &mut RecoveryHooks, _dirs: &AppDirs, step: HookStep) -> HookAction {
    RecoveryHooks::step(step)
}

#[cfg(test)]
fn run_hook_step(hooks: &mut RecoveryHooks, dirs: &AppDirs, step: HookStep) -> HookAction {
    hooks.step(dirs, step)
}

/// Probe the data directory at startup, reconciling any leftover recovery intent first.
pub(crate) fn probe_startup(dirs: &AppDirs) -> StartupProbe {
    let mut hooks = default_hooks();
    match reconcile_with_hooks(dirs, &mut hooks) {
        Ok(ReconcileOutcome::Conflict(detail)) => return StartupProbe::Conflict(detail),
        Ok(
            ReconcileOutcome::Clean | ReconcileOutcome::Published(_) | ReconcileOutcome::RolledBack,
        ) => {}
        Err(err) => return StartupProbe::Conflict(err.to_string()),
    }

    let live = match std::fs::symlink_metadata(&dirs.db_path) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => return StartupProbe::ReadyToOpen,
        Ok(meta) if meta.is_file() => LivePathState::EmptyFile,
        Ok(_) => {
            return StartupProbe::IndeterminateLive(format!(
                "{} is not a regular file",
                dirs.db_path.display()
            ));
        }
        Err(err) if err.kind() == ErrorKind::NotFound => LivePathState::Missing,
        Err(err) => return StartupProbe::IndeterminateLive(err.to_string()),
    };

    let (has_evidence, scan_error) = scan_recovery_evidence(dirs, live);
    if has_evidence || scan_error.is_some() {
        StartupProbe::MissingOrEmpty { live, scan_error }
    } else {
        StartupProbe::FirstLaunch
    }
}

/// Reconcile any interrupted recovery recorded in `recovery-intent.json`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn reconcile_intent(dirs: &AppDirs) -> Result<ReconcileOutcome> {
    let mut hooks = default_hooks();
    reconcile_with_hooks(dirs, &mut hooks)
}

fn scan_recovery_evidence(dirs: &AppDirs, live: LivePathState) -> (bool, Option<String>) {
    let mut has_evidence = matches!(live, LivePathState::EmptyFile);
    let mut scan_error: Option<String> = None;
    let mut record_error = |err: String| {
        if scan_error.is_none() {
            scan_error = Some(err);
        }
    };

    for suffix in SIDECARS {
        match std::fs::symlink_metadata(with_suffix(&dirs.db_path, suffix)) {
            Ok(_) => has_evidence = true,
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => record_error(err.to_string()),
        }
    }

    match scan_dir_for_prefix(&dirs.root, BROKEN_PREFIX, true) {
        Ok(found) => has_evidence |= found,
        Err(err) => record_error(err),
    }

    match scan_dir_for_prefix(&dirs.staging_dir, STAGED_PREFIX, true) {
        Ok(found) => has_evidence |= found,
        Err(err) => record_error(err),
    }

    match backup::list(&dirs.backups_dir) {
        Ok(backups) => has_evidence |= !backups.is_empty(),
        Err(err) => record_error(err.to_string()),
    }

    (has_evidence, scan_error)
}

fn scan_dir_for_prefix(
    dir: &Path,
    prefix: &str,
    allow_sidecars: bool,
) -> std::result::Result<bool, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.to_string()),
    };
    let mut found = false;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let base = if allow_sidecars {
            SIDECARS
                .iter()
                .find_map(|suffix| name.strip_suffix(suffix))
                .unwrap_or(&name)
        } else {
            &name
        };
        if base.starts_with(prefix)
            && Path::new(base)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("db"))
        {
            found = true;
        }
    }
    Ok(found)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivePolicy {
    AnyRegularOrMissing,
    RequireMissingOrEmpty,
}

/// Restore `source` over an unreadable live database. If preparing, moving files aside, or
/// publishing fails, put originals back where verified safe and fail closed without overwriting on
/// any conflict.
///
/// If a prior interrupted recovery had already published and verified the replacement (e.g.
/// cleanup of `recovery-intent.json` was previously blocked), reconciliation finishes cleanup and
/// returns the recorded `broken-*` path without replacing the valid live database again.
///
/// # Errors
/// Backup validation, conflict, file, or database errors preparing or installing the replacement.
pub fn restore_over_broken(
    dirs: &AppDirs,
    source: &Path,
    now_utc: DateTime<Utc>,
) -> Result<PathBuf> {
    let mut hooks = default_hooks();
    restore_with_hooks(dirs, source, now_utc, &mut hooks)
}

fn restore_with_hooks(
    dirs: &AppDirs,
    source: &Path,
    now_utc: DateTime<Utc>,
    hooks: &mut RecoveryHooks,
) -> Result<PathBuf> {
    match reconcile_with_hooks(dirs, hooks)? {
        ReconcileOutcome::Conflict(detail) => return Err(AppError::RecoveryConflict(detail)),
        ReconcileOutcome::Published(broken) => return Ok(broken),
        ReconcileOutcome::Clean | ReconcileOutcome::RolledBack => {}
    }

    let staged = staged_recovery_path(dirs, now_utc)?;
    let mut standalone_ready = false;
    let result = (|| {
        stage_and_upgrade(source, &staged)?;
        prepare_standalone(&staged)?;
        standalone_ready = true;
        replace_live(
            dirs,
            now_utc,
            &staged,
            LivePolicy::AnyRegularOrMissing,
            hooks,
        )
    })();
    if should_clean_staged(&result) {
        if standalone_ready {
            let _ = std::fs::remove_file(&staged);
        } else {
            remove_sqlite_files(&staged);
        }
    }
    result
}

/// Create a new profile only after the user confirms starting fresh on an absent or empty
/// database.
///
/// If a prior recovery publish is reconciled on retry, this finishes cleanup and activates the
/// current validated database without replacing it again.
pub(crate) fn start_fresh(dirs: &AppDirs, now_utc: DateTime<Utc>) -> Result<()> {
    let mut hooks = default_hooks();
    start_fresh_with_hooks(dirs, now_utc, &mut hooks)
}

fn start_fresh_with_hooks(
    dirs: &AppDirs,
    now_utc: DateTime<Utc>,
    hooks: &mut RecoveryHooks,
) -> Result<()> {
    match reconcile_with_hooks(dirs, hooks)? {
        ReconcileOutcome::Conflict(detail) => return Err(AppError::RecoveryConflict(detail)),
        ReconcileOutcome::Published(_) => return Ok(()),
        ReconcileOutcome::Clean | ReconcileOutcome::RolledBack => {}
    }

    match probe_startup(dirs) {
        StartupProbe::Conflict(detail) => return Err(AppError::RecoveryConflict(detail)),
        StartupProbe::IndeterminateLive(detail) => {
            return Err(AppError::LiveMetadataIndeterminate(detail));
        }
        StartupProbe::ReadyToOpen => return Err(AppError::Restore(RestoreError::SameAsLive)),
        StartupProbe::FirstLaunch | StartupProbe::MissingOrEmpty { .. } => {}
    }

    let staged = staged_recovery_path(dirs, now_utc)?;
    let mut standalone_ready = false;
    let result = (|| {
        drop(Db::open(&staged)?);
        prepare_standalone(&staged)?;
        standalone_ready = true;
        replace_live(
            dirs,
            now_utc,
            &staged,
            LivePolicy::RequireMissingOrEmpty,
            hooks,
        )?;
        Ok(())
    })();
    if should_clean_staged(&result) {
        if standalone_ready {
            let _ = std::fs::remove_file(&staged);
        } else {
            remove_sqlite_files(&staged);
        }
    }
    result
}

fn should_clean_staged<T>(result: &Result<T>) -> bool {
    !matches!(result, Err(AppError::RecoveryConflict(_)))
        && !matches!(
            result,
            Err(AppError::Restore(RestoreError::Failed(msg))) if msg == INTERRUPT_MARKER
        )
}

const INTERRUPT_MARKER: &str = "simulated process interruption";

fn replace_live(
    dirs: &AppDirs,
    now_utc: DateTime<Utc>,
    staged: &Path,
    live_policy: LivePolicy,
    hooks: &mut RecoveryHooks,
) -> Result<PathBuf> {
    if live_policy == LivePolicy::RequireMissingOrEmpty {
        confirm_live_missing_or_empty(dirs)?;
    }

    let broken = unique_broken_path(dirs, now_utc)?;
    let broken_name = file_name_str(&broken)?;
    let candidate_name = file_name_str(staged)?;
    validate_relative_db_name(&broken_name, BROKEN_PREFIX).map_err(RestoreError::Failed)?;
    validate_relative_db_name(&candidate_name, STAGED_PREFIX).map_err(RestoreError::Failed)?;

    let (candidate_len, candidate_sha256) = file_len_and_sha256(staged)?;
    let originals = snapshot_live_originals(dirs, live_policy)?;

    let mut intent = RecoveryIntent {
        version: INTENT_VERSION,
        phase: IntentPhase::MovingOriginals,
        broken_name,
        candidate_name,
        candidate_len,
        candidate_sha256,
        originals,
    };
    validate_intent(&intent).map_err(AppError::RecoveryConflict)?;
    write_intent_noclobber(dirs, &intent)?;

    if let Some(early) = apply_step_hook(dirs, hooks, HookStep::AfterWriteIntent, &intent) {
        return Err(early);
    }

    move_originals_aside(dirs, &broken, &intent, live_policy, hooks)?;

    if let Some(early) = apply_step_hook(dirs, hooks, HookStep::BeforeMarkPublishing, &intent) {
        return Err(early);
    }

    intent.phase = IntentPhase::PublishingCandidate;
    update_intent(dirs, &intent)?;

    if let Some(early) = apply_step_hook(dirs, hooks, HookStep::BeforePublish, &intent) {
        return Err(early);
    }

    if let Err(detail) = verify_ready_to_publish(dirs, &intent, staged) {
        return Err(rollback_preserving_error(
            dirs,
            &intent,
            hooks,
            AppError::RecoveryConflict(detail),
        ));
    }

    match publish_staged_noclobber(staged, &dirs.db_path, hooks) {
        Ok(()) => {}
        Err(PublishFailure::Interrupt(err)) => return Err(err),
        Err(PublishFailure::Rollback(err)) => {
            return Err(rollback_preserving_error(dirs, &intent, hooks, err));
        }
    }

    if let Some(early) = apply_step_hook_no_rollback(hooks, dirs, HookStep::AfterPublishBeforeMark)
    {
        return Err(early);
    }

    intent.phase = IntentPhase::Published;
    update_intent(dirs, &intent)?;

    if let Some(early) = apply_step_hook_no_rollback(hooks, dirs, HookStep::AfterMarkPublished) {
        return Err(early);
    }

    verify_published_state(dirs, &intent, &broken, staged).map_err(AppError::RecoveryConflict)?;

    cleanup_finished_intent(dirs, staged)?;
    Ok(broken)
}

fn confirm_live_missing_or_empty(dirs: &AppDirs) -> Result<()> {
    match std::fs::symlink_metadata(&dirs.db_path) {
        Ok(meta) if meta.is_file() && meta.len() == 0 => Ok(()),
        Ok(meta) if meta.is_file() => Err(AppError::Restore(RestoreError::SameAsLive)),
        Ok(_) => Err(AppError::LiveMetadataIndeterminate(format!(
            "{} is not a regular file",
            dirs.db_path.display()
        ))),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn verify_ready_to_publish(
    dirs: &AppDirs,
    intent: &RecoveryIntent,
    staged: &Path,
) -> std::result::Result<(), String> {
    if !matches!(inspect_file_slot(&dirs.db_path)?, FileSlot::Missing) {
        return Err(format!(
            "live path {} is occupied before publish",
            dirs.db_path.display()
        ));
    }
    for suffix in SIDECARS {
        let sidecar = with_suffix(&dirs.db_path, suffix);
        if !matches!(inspect_file_slot(&sidecar)?, FileSlot::Missing) {
            return Err(format!(
                "unexpected live sidecar {} present before publish",
                sidecar.display()
            ));
        }
    }
    match inspect_file_slot(staged)? {
        FileSlot::Regular { len, sha256 }
            if len == intent.candidate_len && sha256 == intent.candidate_sha256 => {}
        _ => {
            return Err(format!(
                "staged candidate {} is missing or changed before publish",
                staged.display()
            ));
        }
    }
    for suffix in SIDECARS {
        let sidecar = with_suffix(staged, suffix);
        if !matches!(inspect_file_slot(&sidecar)?, FileSlot::Missing) {
            return Err(format!(
                "unexpected staged sidecar {} present before publish",
                sidecar.display()
            ));
        }
    }
    Ok(())
}

enum PublishFailure {
    Rollback(AppError),
    Interrupt(AppError),
}

fn publish_staged_noclobber(
    staged: &Path,
    live: &Path,
    hooks: &mut RecoveryHooks,
) -> std::result::Result<(), PublishFailure> {
    match hooks.publish(staged, live) {
        HookPublish::Default => move_noclobber(staged, live).map_err(PublishFailure::Rollback),
        HookPublish::FailWithRollback(msg) => {
            Err(PublishFailure::Rollback(RestoreError::Failed(msg).into()))
        }
        HookPublish::InterruptWithoutRollback => Err(PublishFailure::Interrupt(
            RestoreError::Failed(INTERRUPT_MARKER.into()).into(),
        )),
    }
}

fn apply_step_hook(
    dirs: &AppDirs,
    hooks: &mut RecoveryHooks,
    step: HookStep,
    intent: &RecoveryIntent,
) -> Option<AppError> {
    match run_hook_step(hooks, dirs, step) {
        HookAction::Continue => None,
        HookAction::FailWithRollback(msg) => {
            let err =
                rollback_preserving_error(dirs, intent, hooks, RestoreError::Failed(msg).into());
            Some(err)
        }
        HookAction::InterruptWithoutRollback => {
            Some(RestoreError::Failed(INTERRUPT_MARKER.into()).into())
        }
    }
}

fn apply_step_hook_no_rollback(
    hooks: &mut RecoveryHooks,
    dirs: &AppDirs,
    step: HookStep,
) -> Option<AppError> {
    match run_hook_step(hooks, dirs, step) {
        HookAction::Continue => None,
        HookAction::FailWithRollback(msg) => Some(RestoreError::Failed(msg).into()),
        HookAction::InterruptWithoutRollback => {
            Some(RestoreError::Failed(INTERRUPT_MARKER.into()).into())
        }
    }
}

fn rollback_preserving_error(
    dirs: &AppDirs,
    intent: &RecoveryIntent,
    hooks: &mut RecoveryHooks,
    cause: AppError,
) -> AppError {
    match roll_back_via_intent(dirs, intent, hooks) {
        Ok(()) => cause,
        Err(conflict) => conflict,
    }
}

fn move_originals_aside(
    dirs: &AppDirs,
    broken: &Path,
    intent: &RecoveryIntent,
    live_policy: LivePolicy,
    hooks: &mut RecoveryHooks,
) -> Result<()> {
    for entry in &intent.originals {
        let suffix = static_suffix(&entry.suffix).ok_or_else(|| {
            AppError::RecoveryConflict(format!("invalid tracked suffix {:?}", entry.suffix))
        })?;
        if let Some(early) =
            apply_step_hook(dirs, hooks, HookStep::BeforeMoveOriginal(suffix), intent)
        {
            return Err(early);
        }
        let src = with_suffix(&dirs.db_path, suffix);
        let dest = with_suffix(broken, suffix);

        // Recheck identity right at the move boundary so an unexpected live file (especially a
        // non-empty database appearing during Start Fresh) is never moved into `broken-*`.
        match inspect_file_slot(&src) {
            Ok(FileSlot::Regular { len, sha256 })
                if len == entry.len
                    && sha256 == entry.sha256
                    && (live_policy != LivePolicy::RequireMissingOrEmpty
                        || !suffix.is_empty()
                        || len == 0) => {}
            Ok(_) => {
                return Err(rollback_preserving_error(
                    dirs,
                    intent,
                    hooks,
                    AppError::RecoveryConflict(format!(
                        "live file {} changed before moving aside",
                        src.display()
                    )),
                ));
            }
            Err(detail) => {
                return Err(rollback_preserving_error(
                    dirs,
                    intent,
                    hooks,
                    AppError::RecoveryConflict(detail),
                ));
            }
        }

        if let Err(err) = move_noclobber(&src, &dest) {
            return Err(rollback_preserving_error(dirs, intent, hooks, err));
        }
        if let Some(early) =
            apply_step_hook(dirs, hooks, HookStep::AfterMoveOriginal(suffix), intent)
        {
            return Err(early);
        }
    }
    Ok(())
}

fn snapshot_live_originals(dirs: &AppDirs, live_policy: LivePolicy) -> Result<Vec<OriginalEntry>> {
    let mut entries = Vec::new();
    for suffix in ALL_SUFFIXES {
        let path = with_suffix(&dirs.db_path, suffix);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => {
                if live_policy == LivePolicy::RequireMissingOrEmpty
                    && suffix.is_empty()
                    && meta.len() > 0
                {
                    return Err(AppError::Restore(RestoreError::SameAsLive));
                }
                let (len, sha256) = file_len_and_sha256(&path)?;
                entries.push(OriginalEntry {
                    suffix: suffix.to_string(),
                    len,
                    sha256,
                });
            }
            Ok(_) => {
                return Err(AppError::LiveMetadataIndeterminate(format!(
                    "{} is not a regular file",
                    path.display()
                )));
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(entries)
}

fn reconcile_with_hooks(dirs: &AppDirs, hooks: &mut RecoveryHooks) -> Result<ReconcileOutcome> {
    let path = intent_path(dirs);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Ok(ReconcileOutcome::Conflict(format!(
                "{INTENT_FILE_NAME} is not a regular file"
            )));
        }
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(ReconcileOutcome::Clean),
        Err(err) => {
            return Ok(ReconcileOutcome::Conflict(format!(
                "could not stat {INTENT_FILE_NAME}: {err}"
            )));
        }
    }

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return Ok(ReconcileOutcome::Conflict(format!(
                "could not read {INTENT_FILE_NAME}: {err}"
            )));
        }
    };
    let intent: RecoveryIntent = match serde_json::from_slice(&bytes) {
        Ok(intent) => intent,
        Err(err) => {
            return Ok(ReconcileOutcome::Conflict(format!(
                "corrupt {INTENT_FILE_NAME}: {err}"
            )));
        }
    };
    if let Err(detail) = validate_intent(&intent) {
        return Ok(ReconcileOutcome::Conflict(detail));
    }

    let staged = dirs.staging_dir.join(&intent.candidate_name);
    let broken = dirs.root.join(&intent.broken_name);

    match intent.phase {
        IntentPhase::Published => match verify_published_state(dirs, &intent, &broken, &staged) {
            Ok(()) => {
                cleanup_finished_intent(dirs, &staged)?;
                Ok(ReconcileOutcome::Published(broken))
            }
            Err(detail) => Ok(ReconcileOutcome::Conflict(detail)),
        },
        IntentPhase::PublishingCandidate => {
            reconcile_publishing(dirs, &intent, &broken, &staged, hooks)
        }
        IntentPhase::MovingOriginals | IntentPhase::RollingBack => {
            match roll_back_via_intent(dirs, &intent, hooks) {
                Ok(()) => Ok(ReconcileOutcome::RolledBack),
                Err(AppError::RecoveryConflict(detail)) => Ok(ReconcileOutcome::Conflict(detail)),
                Err(err) => Ok(ReconcileOutcome::Conflict(err.to_string())),
            }
        }
    }
}

fn reconcile_publishing(
    dirs: &AppDirs,
    intent: &RecoveryIntent,
    broken: &Path,
    staged: &Path,
    hooks: &mut RecoveryHooks,
) -> Result<ReconcileOutcome> {
    match inspect_file_slot(&dirs.db_path) {
        Ok(FileSlot::Regular { len, sha256 }) => {
            if len == intent.candidate_len && sha256 == intent.candidate_sha256 {
                match verify_published_state(dirs, intent, broken, staged) {
                    Ok(()) => {
                        cleanup_finished_intent(dirs, staged)?;
                        Ok(ReconcileOutcome::Published(broken.to_path_buf()))
                    }
                    Err(detail) => Ok(ReconcileOutcome::Conflict(detail)),
                }
            } else {
                Ok(ReconcileOutcome::Conflict(format!(
                    "live path {} contains unexpected data",
                    dirs.db_path.display()
                )))
            }
        }
        Ok(FileSlot::NonRegular) => Ok(ReconcileOutcome::Conflict(format!(
            "live path {} is not a regular file",
            dirs.db_path.display()
        ))),
        Ok(FileSlot::Missing) => match roll_back_via_intent(dirs, intent, hooks) {
            Ok(()) => Ok(ReconcileOutcome::RolledBack),
            Err(AppError::RecoveryConflict(detail)) => Ok(ReconcileOutcome::Conflict(detail)),
            Err(err) => Ok(ReconcileOutcome::Conflict(err.to_string())),
        },
        Err(detail) => Ok(ReconcileOutcome::Conflict(detail)),
    }
}

fn verify_published_state(
    dirs: &AppDirs,
    intent: &RecoveryIntent,
    broken: &Path,
    staged: &Path,
) -> std::result::Result<(), String> {
    match inspect_file_slot(&dirs.db_path)? {
        FileSlot::Regular { len, sha256 }
            if len == intent.candidate_len && sha256 == intent.candidate_sha256 => {}
        FileSlot::Missing => {
            return Err("candidate database is missing at the live path".into());
        }
        FileSlot::NonRegular => {
            return Err("live database path is not a regular file".into());
        }
        FileSlot::Regular { .. } => {
            return Err("live database does not match the staged candidate".into());
        }
    }

    for suffix in SIDECARS {
        let sidecar = with_suffix(&dirs.db_path, suffix);
        if !matches!(inspect_file_slot(&sidecar)?, FileSlot::Missing) {
            return Err(format!(
                "unexpected live sidecar {} alongside published candidate",
                sidecar.display()
            ));
        }
    }

    for entry in &intent.originals {
        let saved = with_suffix(broken, &entry.suffix);
        match inspect_file_slot(&saved)? {
            FileSlot::Regular { len, sha256 } if len == entry.len && sha256 == entry.sha256 => {}
            _ => {
                return Err(format!(
                    "saved original {} is missing or does not match expected identity",
                    saved.display()
                ));
            }
        }
    }

    verify_staged_matches_or_missing(staged, intent)?;
    if matches!(inspect_file_slot(staged)?, FileSlot::Regular { .. })
        && !is_same_physical_file(&dirs.db_path, staged)?
    {
        return Err(format!(
            "staged candidate {} and live database {} are distinct files",
            staged.display(),
            dirs.db_path.display()
        ));
    }
    Ok(())
}

fn roll_back_via_intent(
    dirs: &AppDirs,
    intent: &RecoveryIntent,
    hooks: &mut RecoveryHooks,
) -> Result<()> {
    let broken = dirs.root.join(&intent.broken_name);
    let staged = dirs.staging_dir.join(&intent.candidate_name);

    verify_rollback_state(dirs, intent, &broken, &staged).map_err(AppError::RecoveryConflict)?;

    if let Some(early) = apply_step_hook_no_rollback(hooks, dirs, HookStep::BeforeRollbackPhase) {
        return Err(early);
    }

    if intent.phase != IntentPhase::RollingBack {
        let mut rolling = intent.clone();
        rolling.phase = IntentPhase::RollingBack;
        update_intent(dirs, &rolling)?;
    }

    // Move the main database back first so sidecars never restore beside a missing or alien live file.
    if let Some(main_entry) = intent
        .originals
        .iter()
        .find(|entry| entry.suffix.is_empty())
    {
        rollback_one_suffix(dirs, &broken, intent, main_entry, hooks)?;
    }

    for suffix in SIDECARS {
        if let Some(entry) = intent.originals.iter().find(|e| e.suffix == suffix) {
            // Re-confirm the live main path immediately before returning each sidecar.
            confirm_live_main_before_sidecar_rollback(dirs, intent)
                .map_err(AppError::RecoveryConflict)?;
            rollback_one_suffix(dirs, &broken, intent, entry, hooks)?;
        }
    }

    cleanup_finished_intent(dirs, &staged)?;
    Ok(())
}

fn rollback_one_suffix(
    dirs: &AppDirs,
    broken: &Path,
    intent: &RecoveryIntent,
    entry: &OriginalEntry,
    hooks: &mut RecoveryHooks,
) -> Result<()> {
    let suffix = static_suffix(&entry.suffix).ok_or_else(|| {
        AppError::RecoveryConflict(format!("invalid tracked suffix {:?}", entry.suffix))
    })?;
    let live = with_suffix(&dirs.db_path, suffix);
    let saved = with_suffix(broken, suffix);
    let live_slot = inspect_file_slot(&live).map_err(AppError::RecoveryConflict)?;
    let saved_slot = inspect_file_slot(&saved).map_err(AppError::RecoveryConflict)?;

    match (live_slot, saved_slot) {
        (FileSlot::Regular { len, sha256 }, FileSlot::Missing)
            if len == entry.len && sha256 == entry.sha256 =>
        {
            Ok(())
        }
        (FileSlot::Missing, FileSlot::Regular { len, sha256 })
            if len == entry.len && sha256 == entry.sha256 =>
        {
            if let Some(early) =
                apply_step_hook_no_rollback(hooks, dirs, HookStep::BeforeRollbackSuffix(suffix))
            {
                return Err(early);
            }
            if !entry.suffix.is_empty() {
                confirm_live_main_before_sidecar_rollback(dirs, intent)
                    .map_err(AppError::RecoveryConflict)?;
            }
            move_noclobber(&saved, &live)?;
            if let Some(early) =
                apply_step_hook_no_rollback(hooks, dirs, HookStep::AfterRollbackSuffix(suffix))
            {
                return Err(early);
            }
            Ok(())
        }
        (
            FileSlot::Regular {
                len: l_len,
                sha256: l_sha,
            },
            FileSlot::Regular {
                len: s_len,
                sha256: s_sha,
            },
        ) if l_len == entry.len
            && l_sha == entry.sha256
            && s_len == entry.len
            && s_sha == entry.sha256
            && is_same_physical_file(&live, &saved).map_err(AppError::RecoveryConflict)? =>
        {
            if let Some(early) =
                apply_step_hook_no_rollback(hooks, dirs, HookStep::BeforeRollbackSuffix(suffix))
            {
                return Err(early);
            }
            if !entry.suffix.is_empty() {
                confirm_live_main_before_sidecar_rollback(dirs, intent)
                    .map_err(AppError::RecoveryConflict)?;
            }
            std::fs::remove_file(&saved)?;
            if let Some(early) =
                apply_step_hook_no_rollback(hooks, dirs, HookStep::AfterRollbackSuffix(suffix))
            {
                return Err(early);
            }
            Ok(())
        }
        _ => Err(AppError::RecoveryConflict(format!(
            "cannot safely restore {} from {}",
            live.display(),
            saved.display()
        ))),
    }
}

fn confirm_live_main_before_sidecar_rollback(
    dirs: &AppDirs,
    intent: &RecoveryIntent,
) -> std::result::Result<(), String> {
    let orig_main = intent.originals.iter().find(|e| e.suffix.is_empty());
    match (orig_main, inspect_file_slot(&dirs.db_path)?) {
        (Some(main), FileSlot::Regular { len, sha256 })
            if len == main.len && sha256 == main.sha256 =>
        {
            Ok(())
        }
        (None, FileSlot::Missing) => Ok(()),
        _ => Err("live database path changed before sidecars could be restored".into()),
    }
}

fn verify_rollback_state(
    dirs: &AppDirs,
    intent: &RecoveryIntent,
    broken: &Path,
    staged: &Path,
) -> std::result::Result<(), String> {
    let orig_main = intent.originals.iter().find(|e| e.suffix.is_empty());
    if orig_main.is_none() && !matches!(inspect_file_slot(&dirs.db_path)?, FileSlot::Missing) {
        return Err(
            "unexpected database file exists at the live path; refusing to restore orphaned sidecars"
                .into(),
        );
    }

    for suffix in SIDECARS {
        if !intent.originals.iter().any(|e| e.suffix == suffix) {
            let live_sidecar = with_suffix(&dirs.db_path, suffix);
            if !matches!(inspect_file_slot(&live_sidecar)?, FileSlot::Missing) {
                return Err(format!(
                    "unexpected untracked live sidecar {}",
                    live_sidecar.display()
                ));
            }
        }
    }

    for entry in &intent.originals {
        let live = with_suffix(&dirs.db_path, &entry.suffix);
        let saved = with_suffix(broken, &entry.suffix);
        let live_slot = inspect_file_slot(&live)?;
        let saved_slot = inspect_file_slot(&saved)?;
        let live_matches = matches!(
            live_slot,
            FileSlot::Regular { len, ref sha256 } if len == entry.len && sha256 == &entry.sha256
        );
        let saved_matches = matches!(
            saved_slot,
            FileSlot::Regular { len, ref sha256 } if len == entry.len && sha256 == &entry.sha256
        );
        let at_live = live_matches && matches!(saved_slot, FileSlot::Missing);
        let at_saved = matches!(live_slot, FileSlot::Missing) && saved_matches;
        let alias = live_matches && saved_matches && is_same_physical_file(&live, &saved)?;
        if !at_live && !at_saved && !alias {
            return Err(format!(
                "file mismatch for suffix {:?} between {} and {}",
                entry.suffix,
                live.display(),
                saved.display()
            ));
        }
    }

    verify_staged_matches_or_missing(staged, intent)?;
    Ok(())
}

fn verify_staged_matches_or_missing(
    staged: &Path,
    intent: &RecoveryIntent,
) -> std::result::Result<(), String> {
    match inspect_file_slot(staged)? {
        FileSlot::Missing => {}
        FileSlot::Regular { len, sha256 }
            if len == intent.candidate_len && sha256 == intent.candidate_sha256 => {}
        FileSlot::NonRegular | FileSlot::Regular { .. } => {
            return Err(format!(
                "staged candidate {} does not match recorded intent",
                staged.display()
            ));
        }
    }
    for suffix in SIDECARS {
        let sidecar = with_suffix(staged, suffix);
        if !matches!(inspect_file_slot(&sidecar)?, FileSlot::Missing) {
            return Err(format!(
                "unexpected staged sidecar {} alongside recovery candidate",
                sidecar.display()
            ));
        }
    }
    Ok(())
}

fn cleanup_finished_intent(dirs: &AppDirs, staged: &Path) -> Result<()> {
    remove_file_if_present(staged)?;
    remove_file_if_present(&intent_path(dirs))
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn inspect_file_slot(path: &Path) -> std::result::Result<FileSlot, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => {
            let (len, sha256) = file_len_and_sha256(path).map_err(|err| err.to_string())?;
            Ok(FileSlot::Regular { len, sha256 })
        }
        Ok(_) => Ok(FileSlot::NonRegular),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(FileSlot::Missing),
        Err(err) => Err(format!("could not inspect {}: {err}", path.display())),
    }
}

#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
fn is_same_physical_file(a: &Path, b: &Path) -> std::result::Result<bool, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta_a = std::fs::symlink_metadata(a)
            .map_err(|err| format!("could not inspect {}: {err}", a.display()))?;
        let meta_b = std::fs::symlink_metadata(b)
            .map_err(|err| format!("could not inspect {}: {err}", b.display()))?;
        Ok(meta_a.is_file()
            && meta_b.is_file()
            && meta_a.dev() == meta_b.dev()
            && meta_a.ino() == meta_b.ino()
            && meta_a.nlink() >= 2)
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        Ok(false)
    }
}

fn validate_intent(intent: &RecoveryIntent) -> std::result::Result<(), String> {
    if intent.version != INTENT_VERSION {
        return Err(format!("unsupported intent version {}", intent.version));
    }
    validate_relative_db_name(&intent.broken_name, BROKEN_PREFIX)?;
    validate_relative_db_name(&intent.candidate_name, STAGED_PREFIX)?;
    if !is_valid_sha256_hex(&intent.candidate_sha256) {
        return Err("invalid candidate checksum in recovery intent".into());
    }
    let mut seen = HashSet::new();
    for entry in &intent.originals {
        if static_suffix(&entry.suffix).is_none() {
            return Err(format!(
                "invalid suffix {:?} in recovery intent",
                entry.suffix
            ));
        }
        if !seen.insert(entry.suffix.as_str()) {
            return Err(format!(
                "duplicate suffix {:?} in recovery intent",
                entry.suffix
            ));
        }
        if !is_valid_sha256_hex(&entry.sha256) {
            return Err(format!(
                "invalid original checksum for suffix {:?} in recovery intent",
                entry.suffix
            ));
        }
    }
    Ok(())
}

fn validate_relative_db_name(name: &str, prefix: &str) -> std::result::Result<(), String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains(':') {
        return Err(format!("invalid relative filename {name:?}"));
    }
    let mut comps = Path::new(name).components();
    if !matches!(comps.next(), Some(Component::Normal(_))) || comps.next().is_some() {
        return Err(format!("unsafe path component in {name:?}"));
    }
    if !name.starts_with(prefix)
        || !Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("db"))
    {
        return Err(format!("unexpected filename format {name:?}"));
    }
    Ok(())
}

fn is_valid_sha256_hex(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn static_suffix(suffix: &str) -> Option<&'static str> {
    ALL_SUFFIXES.into_iter().find(|&allowed| allowed == suffix)
}

fn file_name_str(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Restore(RestoreError::Failed("invalid recovery path name".into())))
}

fn intent_path(dirs: &AppDirs) -> PathBuf {
    dirs.root.join(INTENT_FILE_NAME)
}

fn write_intent_noclobber(dirs: &AppDirs, intent: &RecoveryIntent) -> Result<()> {
    let path = intent_path(dirs);
    let json = serde_json::to_vec_pretty(intent)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    let mut tmp = tempfile::NamedTempFile::new_in(&dirs.root)?;
    tmp.write_all(&json)?;
    tmp.as_file().sync_all()?;
    tmp.persist_noclobber(&path)
        .map_err(|err| AppError::Io(err.error))?;
    Ok(())
}

fn update_intent(dirs: &AppDirs, intent: &RecoveryIntent) -> Result<()> {
    let path = intent_path(dirs);
    let json = serde_json::to_vec_pretty(intent)
        .map_err(|err| AppError::Restore(RestoreError::Failed(err.to_string())))?;
    let mut tmp = tempfile::NamedTempFile::new_in(&dirs.root)?;
    tmp.write_all(&json)?;
    tmp.as_file().sync_all()?;
    tmp.persist(&path).map_err(|err| AppError::Io(err.error))?;
    Ok(())
}

/// Move `src` to `dest` without clobbering any existing destination file and with zero destructor
/// ownership over `src`, so any failure leaves `src` untouched on disk.
fn move_noclobber(src: &Path, dest: &Path) -> Result<()> {
    match std::fs::symlink_metadata(src) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => {
            return Err(AppError::Io(std::io::Error::new(
                ErrorKind::InvalidInput,
                format!("source is not a regular file: {}", src.display()),
            )));
        }
        Err(err) => return Err(err.into()),
    }
    match std::fs::symlink_metadata(dest) {
        Ok(_) => {
            return Err(AppError::Io(std::io::Error::new(
                ErrorKind::AlreadyExists,
                format!("destination already exists: {}", dest.display()),
            )));
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    #[cfg(windows)]
    {
        move_file_noclobber_windows(src, dest)?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::hard_link(src, dest)?;
        std::fs::remove_file(src)?;
        Ok(())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn move_file_noclobber_windows(src: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            lpExistingFileName: *const u16,
            lpNewFileName: *const u16,
            dwFlags: u32,
        ) -> i32;
    }

    let src_wide: Vec<u16> = src.as_os_str().encode_wide().chain(Some(0)).collect();
    let dest_wide: Vec<u16> = dest.as_os_str().encode_wide().chain(Some(0)).collect();

    // SAFETY: Both pointers reference null-terminated UTF-16 buffers valid for the call duration.
    // `dwFlags = 0` performs a non-clobbering move without modifying file attributes.
    let ok = unsafe { MoveFileExW(src_wide.as_ptr(), dest_wide.as_ptr(), 0) };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Steps 1–4 of `§9.3` against the staged copy. The page-size comparison has no live database to
/// match in recovery, so only identity, integrity, version, migrations, and the domain invariants
/// are enforced.
fn stage_and_upgrade(source: &Path, staged: &Path) -> Result<()> {
    match std::fs::symlink_metadata(source) {
        Ok(meta) if meta.is_file() && meta.len() > 0 => {}
        Ok(meta) if meta.is_file() => return Err(AppError::Restore(RestoreError::Corrupt)),
        _ => return Err(AppError::Restore(RestoreError::NotAthena)),
    }
    restore::step_stage(source, staged)?;
    let _snapshot = backup::inspect_athena_snapshot(staged).map_err(AppError::Restore)?;
    restore::step_upgrade(staged)
}

/// Close `SQLite` with a standalone database, then flush the file before publishing it.
fn prepare_standalone(staged: &Path) -> Result<()> {
    let conn = rusqlite::Connection::open(staged)?;
    let mode: String = conn.query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("delete") {
        return Err(AppError::Restore(RestoreError::Failed(
            "could not make the staged database standalone".into(),
        )));
    }
    drop(conn);
    std::fs::OpenOptions::new()
        .write(true)
        .open(staged)?
        .sync_all()?;
    Ok(())
}

fn staged_recovery_path(dirs: &AppDirs, now_utc: DateTime<Utc>) -> Result<PathBuf> {
    unique_path(
        &dirs.staging_dir,
        &format!("{STAGED_PREFIX}{}", now_utc.format("%Y-%m-%d_%H-%M-%S")),
    )
}

fn unique_broken_path(dirs: &AppDirs, now_utc: DateTime<Utc>) -> Result<PathBuf> {
    unique_path(
        &dirs.root,
        &format!("{BROKEN_PREFIX}{}", now_utc.format("%Y-%m-%d_%H-%M-%S")),
    )
}

/// `<dir>/<stem>.db`, or `<dir>/<stem>-<n>.db` when that already exists.
fn unique_path(dir: &Path, stem: &str) -> Result<PathBuf> {
    let first = dir.join(format!("{stem}.db"));
    if path_available(&first)? {
        return Ok(first);
    }
    for n in 1_u32.. {
        let candidate = dir.join(format!("{stem}-{n}.db"));
        if path_available(&candidate)? {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn path_available(path: &Path) -> Result<bool> {
    for suffix in ALL_SUFFIXES {
        match std::fs::symlink_metadata(with_suffix(path, suffix)) {
            Ok(_) => return Ok(false),
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(true)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    if suffix.is_empty() {
        return path.to_path_buf();
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return path.to_path_buf();
    };
    path.with_file_name(format!("{name}{suffix}"))
}

fn remove_sqlite_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in SIDECARS {
        let _ = std::fs::remove_file(with_suffix(path, suffix));
    }
}

fn file_len_and_sha256(path: &Path) -> Result<(u64, String)> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Err(AppError::Io(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("{} is not a regular file", path.display()),
        )));
    }
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total = total.saturating_add(n as u64);
    }
    Ok((total, hasher.finalize_hex()))
}

struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    bit_len: u64,
}

impl Sha256 {
    const H0: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    fn new() -> Self {
        Self {
            state: Self::H0,
            block: [0; 64],
            block_len: 0,
            bit_len: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.bit_len = self.bit_len.wrapping_add((data.len() as u64) * 8);
        while !data.is_empty() {
            let take = (64 - self.block_len).min(data.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&data[..take]);
            self.block_len += take;
            data = &data[take..];
            if self.block_len == 64 {
                let block = self.block;
                self.compress(&block);
                self.block_len = 0;
            }
        }
    }

    fn finalize_hex(mut self) -> String {
        self.block[self.block_len] = 0x80;
        self.block_len += 1;
        if self.block_len > 56 {
            self.block[self.block_len..].fill(0);
            let block = self.block;
            self.compress(&block);
            self.block_len = 0;
        }
        self.block[self.block_len..56].fill(0);
        self.block[56..64].copy_from_slice(&self.bit_len.to_be_bytes());
        let block = self.block;
        self.compress(&block);

        let mut out = String::with_capacity(64);
        for word in self.state {
            use std::fmt::Write as _;
            let _ = write!(out, "{word:08x}");
        }
        out
    }

    #[allow(clippy::many_single_char_names)]
    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0_u32; 64];
        for (slot, chunk) in w.iter_mut().zip(block.chunks_exact(4)) {
            *slot = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for (&k_i, &w_i) in Self::K.iter().zip(w.iter()) {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k_i)
                .wrapping_add(w_i);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (slot, delta) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(delta);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppSession;
    use crate::domain::money::Money;
    use crate::domain::settings::{Language, Profile, Theme};
    use crate::infrastructure::db::profile_repo;
    use crate::shared::test_fixtures::{seed_month, seed_people};

    const STAMP: &str = "2026-11-16T12:00:00Z";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(STAMP)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn sha256_bytes(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        h.finalize_hex()
    }

    /// A temp root with a valid Athena backup at `backups/manual-good.db` and garbage bytes plus
    /// WAL/SHM sidecars at the live path.
    fn broken_env() -> (tempfile::TempDir, AppDirs, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();

        let source = dirs.backups_dir.join("manual-good.db");
        {
            let db = Db::open(&source).unwrap();
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
            let (_, _) = seed_people(db.conn());
            seed_month(&db);
        }
        std::fs::write(&dirs.db_path, b"this is not a database").unwrap();
        std::fs::write(with_suffix(&dirs.db_path, "-wal"), b"stale wal").unwrap();
        std::fs::write(with_suffix(&dirs.db_path, "-shm"), b"stale shm").unwrap();
        (dir, dirs, source)
    }

    fn broken_files(dirs: &AppDirs) -> Vec<PathBuf> {
        std::fs::read_dir(&dirs.root)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(BROKEN_PREFIX))
            })
            .collect()
    }

    #[test]
    fn sha256_matches_fips_180_4_vectors() {
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_bytes(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn move_noclobber_preserves_existing_destination_and_source() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.db");
        let dest = dir.path().join("dest.db");
        std::fs::write(&src, b"source bytes").unwrap();
        std::fs::write(&dest, b"existing destination").unwrap();

        let err = move_noclobber(&src, &dest).unwrap_err();
        assert!(matches!(err, AppError::Io(_)));
        assert_eq!(std::fs::read(&dest).unwrap(), b"existing destination");
        assert_eq!(std::fs::read(&src).unwrap(), b"source bytes");
    }

    #[test]
    fn recovery_preserves_orphan_broken_sidecars_on_name_collision() {
        let (_dir, dirs, source) = broken_env();
        let orphan = dirs.root.join("broken-2026-11-16_12-00-00.db-wal");
        std::fs::write(&orphan, b"previous evidence").unwrap();
        let saved = restore_over_broken(&dirs, &source, now()).unwrap();
        assert_eq!(
            saved.file_name().unwrap(),
            "broken-2026-11-16_12-00-00-1.db"
        );
        assert_eq!(std::fs::read(orphan).unwrap(), b"previous evidence");
    }

    #[test]
    fn recovery_restore_moves_broken_files_aside_and_opens_the_live_path() {
        let (_dir, dirs, source) = broken_env();

        let broken = restore_over_broken(&dirs, &source, now()).unwrap();

        assert_eq!(
            broken.file_name().and_then(|n| n.to_str()),
            Some("broken-2026-11-16_12-00-00.db")
        );
        assert_eq!(std::fs::read(&broken).unwrap(), b"this is not a database");
        assert!(with_suffix(&broken, "-wal").is_file());
        assert!(with_suffix(&broken, "-shm").is_file());
        assert!(!with_suffix(&dirs.db_path, "-wal").exists());
        assert!(!with_suffix(&dirs.db_path, "-shm").exists());
        assert!(!intent_path(&dirs).exists());

        let db = Db::open(&dirs.db_path).unwrap();
        assert_eq!(
            profile_repo::load(db.conn())
                .unwrap()
                .unwrap()
                .start_balance,
            Money::from_cents(10_000)
        );
        assert!(
            std::fs::read_dir(&dirs.staging_dir)
                .unwrap()
                .next()
                .is_none(),
            "the staged copy must be removed"
        );
    }

    #[test]
    fn intent_persists_validated_relative_names_and_suffixes_only() {
        let (_dir, dirs, source) = broken_env();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured_hook = captured.clone();
        let mut hooks = RecoveryHooks::with_step(move |dirs, step| {
            if step == HookStep::AfterWriteIntent {
                *captured_hook.lock().unwrap() =
                    Some(std::fs::read_to_string(intent_path(dirs)).unwrap());
            }
            HookAction::Continue
        });
        restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap();
        let json = captured.lock().unwrap().take().unwrap();
        let intent: RecoveryIntent = serde_json::from_str(&json).unwrap();
        assert_eq!(intent.broken_name, "broken-2026-11-16_12-00-00.db");
        assert_eq!(intent.candidate_name, "recovery-2026-11-16_12-00-00.db");
        assert!(!json.contains('\\') && !json.contains('/'));
        assert_eq!(
            intent
                .originals
                .iter()
                .map(|o| o.suffix.as_str())
                .collect::<Vec<_>>(),
            vec!["", "-wal", "-shm"]
        );
    }

    #[test]
    fn recovery_restore_failure_stays_in_recovery_with_files_intact() {
        let (_dir, dirs, _source) = broken_env();
        let garbage = dirs.backups_dir.join("manual-garbage.db");
        std::fs::write(&garbage, b"also not a database").unwrap();

        let result = restore_over_broken(&dirs, &garbage, now());

        assert!(matches!(result, Err(AppError::Restore(_))), "{result:?}");
        assert_eq!(
            std::fs::read(&dirs.db_path).unwrap(),
            b"this is not a database"
        );
        assert!(with_suffix(&dirs.db_path, "-wal").is_file());
        assert!(broken_files(&dirs).is_empty(), "nothing moved aside");
        assert!(!intent_path(&dirs).exists());
        assert!(
            std::fs::read_dir(&dirs.staging_dir)
                .unwrap()
                .next()
                .is_none(),
            "the rejected staged copy must be removed"
        );
    }

    #[test]
    fn a_second_recovery_restore_uses_a_fresh_broken_name() {
        let (_dir, dirs, source) = broken_env();
        restore_over_broken(&dirs, &source, now()).unwrap();
        std::fs::write(&dirs.db_path, b"broken again").unwrap();

        let second = restore_over_broken(&dirs, &source, now()).unwrap();

        assert_eq!(
            second.file_name().and_then(|n| n.to_str()),
            Some("broken-2026-11-16_12-00-00-1.db")
        );
        assert!(Db::open(&dirs.db_path).is_ok());
    }

    #[test]
    fn failed_install_puts_back_the_original_database_and_sidecars() {
        let (_dir, dirs, source) = broken_env();
        let journal = with_suffix(&dirs.db_path, "-journal");
        std::fs::write(&journal, b"original journal").unwrap();
        let source_before = std::fs::read(&source).unwrap();

        let mut hooks = RecoveryHooks::with_publish(|candidate, live| {
            assert!(!live.exists(), "failure occurs after the original moves");
            assert!(backup::is_valid_athena_snapshot(candidate).unwrap());
            HookPublish::FailWithRollback("simulated publish failure".into())
        });
        let error = restore_with_hooks(&dirs, &source, now(), &mut hooks);
        assert!(error.is_err());
        assert_eq!(
            std::fs::read(&dirs.db_path).unwrap(),
            b"this is not a database"
        );
        assert_eq!(
            std::fs::read(with_suffix(&dirs.db_path, "-wal")).unwrap(),
            b"stale wal"
        );
        assert_eq!(
            std::fs::read(with_suffix(&dirs.db_path, "-shm")).unwrap(),
            b"stale shm"
        );
        assert_eq!(std::fs::read(journal).unwrap(), b"original journal");
        assert_eq!(std::fs::read(source).unwrap(), source_before);
        assert!(broken_files(&dirs).is_empty());
        assert!(!intent_path(&dirs).exists());
        assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
        assert!(matches!(
            crate::open_session(dirs).unwrap(),
            AppSession::Recovery(_)
        ));
    }

    #[test]
    fn fresh_start_refuses_to_replace_a_nonempty_live_database() {
        let (_dir, dirs, _source) = broken_env();
        let original = std::fs::read(&dirs.db_path).unwrap();
        assert!(start_fresh(&dirs, now()).is_err());
        assert_eq!(std::fs::read(&dirs.db_path).unwrap(), original);
        assert!(broken_files(&dirs).is_empty());
    }

    #[test]
    fn recovery_keeps_the_original_rollback_journal_with_its_database() {
        let (_dir, dirs, source) = broken_env();
        let journal = with_suffix(&dirs.db_path, "-journal");
        std::fs::write(&journal, b"original journal").unwrap();
        let broken = restore_over_broken(&dirs, &source, now()).unwrap();
        assert!(!journal.exists());
        assert_eq!(
            std::fs::read(with_suffix(&broken, "-journal")).unwrap(),
            b"original journal"
        );
    }

    #[test]
    fn process_interruption_at_every_move_and_before_publish_reconciles_to_original_state() {
        let steps = [
            HookStep::AfterWriteIntent,
            HookStep::BeforeMoveOriginal(""),
            HookStep::AfterMoveOriginal(""),
            HookStep::BeforeMoveOriginal("-wal"),
            HookStep::AfterMoveOriginal("-wal"),
            HookStep::BeforeMoveOriginal("-shm"),
            HookStep::AfterMoveOriginal("-shm"),
            HookStep::BeforeMoveOriginal("-journal"),
            HookStep::AfterMoveOriginal("-journal"),
            HookStep::BeforeMarkPublishing,
            HookStep::BeforePublish,
        ];

        for target_step in steps {
            let (_dir, dirs, source) = broken_env();
            let journal = with_suffix(&dirs.db_path, "-journal");
            std::fs::write(&journal, b"original journal").unwrap();

            let mut hooks = RecoveryHooks::with_step(move |_dirs, step| {
                if step == target_step {
                    HookAction::InterruptWithoutRollback
                } else {
                    HookAction::Continue
                }
            });
            let err = restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();
            assert!(
                err.to_string().contains(INTERRUPT_MARKER),
                "{target_step:?}: {err}"
            );
            assert!(intent_path(&dirs).exists(), "{target_step:?}");

            // First restart reconciles and keeps Recovery UI because originals are broken bytes.
            let session = crate::open_session(dirs.clone()).unwrap();
            let AppSession::Recovery(state) = session else {
                panic!("{target_step:?}: rolled-back broken DB must reopen in Recovery");
            };
            assert!(matches!(state.error, AppError::Corrupt { .. }));
            assert!(!state.can_start_fresh());

            // Original bytes and sidecars are back at live; broken/staging/intent are clean.
            assert_eq!(
                std::fs::read(&dirs.db_path).unwrap(),
                b"this is not a database"
            );
            assert_eq!(
                std::fs::read(with_suffix(&dirs.db_path, "-wal")).unwrap(),
                b"stale wal"
            );
            assert_eq!(
                std::fs::read(with_suffix(&dirs.db_path, "-shm")).unwrap(),
                b"stale shm"
            );
            assert_eq!(std::fs::read(&journal).unwrap(), b"original journal");
            assert!(broken_files(&dirs).is_empty());
            assert!(!intent_path(&dirs).exists());
            assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);

            // Idempotent re-reconcile is Clean.
            assert_eq!(reconcile_intent(&dirs).unwrap(), ReconcileOutcome::Clean);
        }
    }

    #[test]
    fn process_interruption_at_and_after_publish_completes_idempotently() {
        for target_step in [
            HookStep::AfterPublishBeforeMark,
            HookStep::AfterMarkPublished,
        ] {
            let (_dir, dirs, source) = broken_env();
            let mut hooks = RecoveryHooks::with_step(move |_dirs, step| {
                if step == target_step {
                    HookAction::InterruptWithoutRollback
                } else {
                    HookAction::Continue
                }
            });
            restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();
            assert!(intent_path(&dirs).exists(), "{target_step:?}");

            // Candidate checksum matches at live and originals are at broken-* -> Active!
            let session = crate::open_session(dirs.clone()).unwrap();
            assert!(
                matches!(session, AppSession::Active(_)),
                "{target_step:?}: verified publish must open Active"
            );
            assert!(!intent_path(&dirs).exists());
            assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
            assert_eq!(reconcile_intent(&dirs).unwrap(), ReconcileOutcome::Clean);

            // Broken originals are still preserved.
            let broken = dirs.root.join("broken-2026-11-16_12-00-00.db");
            assert_eq!(std::fs::read(&broken).unwrap(), b"this is not a database");
            assert_eq!(
                std::fs::read(with_suffix(&broken, "-wal")).unwrap(),
                b"stale wal"
            );
            assert_eq!(
                std::fs::read(with_suffix(&broken, "-shm")).unwrap(),
                b"stale shm"
            );
        }
    }

    #[test]
    fn process_interruption_mid_rollback_completes_on_next_restart() {
        for target_step in [
            HookStep::BeforeRollbackPhase,
            HookStep::BeforeRollbackSuffix(""),
            HookStep::AfterRollbackSuffix(""),
            HookStep::BeforeRollbackSuffix("-wal"),
            HookStep::AfterRollbackSuffix("-wal"),
            HookStep::BeforeRollbackSuffix("-shm"),
        ] {
            let (_dir, dirs, source) = broken_env();
            let mut moved_back_trigger = false;
            let mut hooks = RecoveryHooks::with_step(move |_dirs, step| {
                if step == HookStep::BeforePublish {
                    return HookAction::FailWithRollback("trigger rollback".into());
                }
                if step == target_step && !moved_back_trigger {
                    moved_back_trigger = true;
                    return HookAction::InterruptWithoutRollback;
                }
                HookAction::Continue
            });
            restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();
            assert!(intent_path(&dirs).exists(), "{target_step:?}");

            // Restart finishes rollback cleanly and opens Recovery for the corrupt original DB.
            let session = crate::open_session(dirs.clone()).unwrap();
            let AppSession::Recovery(state) = session else {
                panic!("{target_step:?}: interrupted rollback must reopen in Recovery");
            };
            assert!(matches!(state.error, AppError::Corrupt { .. }));
            assert_eq!(
                std::fs::read(&dirs.db_path).unwrap(),
                b"this is not a database"
            );
            assert_eq!(
                std::fs::read(with_suffix(&dirs.db_path, "-wal")).unwrap(),
                b"stale wal"
            );
            assert_eq!(
                std::fs::read(with_suffix(&dirs.db_path, "-shm")).unwrap(),
                b"stale shm"
            );
            assert!(broken_files(&dirs).is_empty());
            assert!(!intent_path(&dirs).exists());
        }
    }

    #[test]
    fn live_file_presence_alone_never_proves_publish_and_unexpected_live_fails_closed() {
        let (_dir, dirs, source) = broken_env();
        let mut hooks = RecoveryHooks::with_publish(|_staged, live| {
            // Simulate another file appearing at live instead of the candidate.
            std::fs::write(live, b"unexpected alien database").unwrap();
            HookPublish::InterruptWithoutRollback
        });
        restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();

        let session = crate::open_session(dirs.clone()).unwrap();
        let AppSession::Recovery(state) = session else {
            panic!("unexpected occupied live must fail closed into Recovery");
        };
        assert!(matches!(state.error, AppError::RecoveryConflict(_)));
        assert!(!state.can_start_fresh());

        // Every byte must be preserved without overwriting live or attaching old sidecars.
        assert_eq!(
            std::fs::read(&dirs.db_path).unwrap(),
            b"unexpected alien database"
        );
        assert!(!with_suffix(&dirs.db_path, "-wal").exists());
        assert!(!with_suffix(&dirs.db_path, "-shm").exists());
        let broken = dirs.root.join("broken-2026-11-16_12-00-00.db");
        assert_eq!(std::fs::read(&broken).unwrap(), b"this is not a database");
        assert_eq!(
            std::fs::read(with_suffix(&broken, "-wal")).unwrap(),
            b"stale wal"
        );
        assert!(intent_path(&dirs).exists());

        // Subsequent restore refuses to proceed while Conflict is unresolved.
        assert!(matches!(
            restore_over_broken(&dirs, &source, now()),
            Err(AppError::RecoveryConflict(_))
        ));
    }

    #[test]
    fn published_candidate_with_unexpected_live_sidecar_fails_closed() {
        let (_dir, dirs, source) = broken_env();
        let mut hooks = RecoveryHooks::with_step(|dirs, step| {
            if step == HookStep::AfterPublishBeforeMark {
                std::fs::write(with_suffix(&dirs.db_path, "-wal"), b"rogue wal").unwrap();
                return HookAction::InterruptWithoutRollback;
            }
            HookAction::Continue
        });
        restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();

        let session = crate::open_session(dirs.clone()).unwrap();
        let AppSession::Recovery(state) = session else {
            panic!("live sidecar alongside published candidate must fail closed");
        };
        assert!(matches!(state.error, AppError::RecoveryConflict(_)));
        assert!(!state.can_start_fresh());
        assert_eq!(
            std::fs::read(with_suffix(&dirs.db_path, "-wal")).unwrap(),
            b"rogue wal"
        );
    }

    #[test]
    fn missing_main_orphan_sidecar_restore_and_unexpected_occupied_live_preserve_all_bytes() {
        // Variant A: clean interruption & restart reconciliation with missing main + orphan sidecars.
        {
            let (_dir, dirs, source) = broken_env();
            std::fs::remove_file(&dirs.db_path).unwrap();
            std::fs::write(with_suffix(&dirs.db_path, "-journal"), b"orphan journal").unwrap();

            let mut hooks = RecoveryHooks::with_step(|_dirs, step| {
                if step == HookStep::BeforePublish {
                    HookAction::InterruptWithoutRollback
                } else {
                    HookAction::Continue
                }
            });
            restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();

            let session = crate::open_session(dirs.clone()).unwrap();
            let AppSession::Recovery(state) = session else {
                panic!("rolled back missing-main + orphan sidecars must stay in Recovery");
            };
            assert!(matches!(state.error, AppError::LiveDatabaseMissingOrEmpty));
            assert!(state.can_start_fresh());
            assert!(!dirs.db_path.exists());
            assert_eq!(
                std::fs::read(with_suffix(&dirs.db_path, "-wal")).unwrap(),
                b"stale wal"
            );
            assert_eq!(
                std::fs::read(with_suffix(&dirs.db_path, "-journal")).unwrap(),
                b"orphan journal"
            );
        }

        // Variant B: missing main + orphan sidecars, then unexpected file appears at live path.
        // Sidecars MUST NOT be moved back next to the unexpected DB!
        {
            let (_dir, dirs, source) = broken_env();
            std::fs::remove_file(&dirs.db_path).unwrap();

            let mut hooks = RecoveryHooks::with_publish(|_staged, live| {
                std::fs::write(live, b"unexpected new db").unwrap();
                HookPublish::FailWithRollback("dest occupied".into())
            });
            let err = restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();
            assert!(matches!(err, AppError::RecoveryConflict(_)), "{err:?}");

            assert_eq!(std::fs::read(&dirs.db_path).unwrap(), b"unexpected new db");
            assert!(!with_suffix(&dirs.db_path, "-wal").exists());
            assert!(!with_suffix(&dirs.db_path, "-shm").exists());
            let broken_wal = dirs.root.join("broken-2026-11-16_12-00-00.db-wal");
            assert_eq!(std::fs::read(&broken_wal).unwrap(), b"stale wal");

            let session = crate::open_session(dirs).unwrap();
            let AppSession::Recovery(state) = session else {
                panic!("conflict must reopen Recovery");
            };
            assert!(!state.can_start_fresh());
        }
    }

    #[test]
    fn corrupt_or_path_traversal_intent_fails_closed_without_touching_files() {
        let (_dir, dirs, _source) = broken_env();
        let bad_intents = [
            "not json",
            r#"{"version":1,"phase":"moving_originals","broken_name":"../broken-1.db","candidate_name":"recovery-1.db","candidate_len":0,"candidate_sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","originals":[]}"#,
            r#"{"version":1,"phase":"moving_originals","broken_name":"broken-1.db","candidate_name":"recovery-1.db","candidate_len":0,"candidate_sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855","originals":[{"suffix":"-tmp","len":1,"sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"}]}"#,
        ];

        for raw in bad_intents {
            std::fs::write(intent_path(&dirs), raw).unwrap();
            let session = crate::open_session(dirs.clone()).unwrap();
            let AppSession::Recovery(state) = session else {
                panic!("invalid intent must open Recovery conflict");
            };
            assert!(matches!(state.error, AppError::RecoveryConflict(_)));
            assert!(!state.can_start_fresh());
            assert_eq!(
                std::fs::read(&dirs.db_path).unwrap(),
                b"this is not a database"
            );
        }
    }

    #[test]
    fn startup_probe_distinguishes_first_launch_missing_with_evidence_scan_failure_and_indeterminate()
     {
        // 1. Clean first launch.
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            dirs.ensure().unwrap();
            assert_eq!(probe_startup(&dirs), StartupProbe::FirstLaunch);
        }

        // 2. Confirmed empty live database + orphan sidecar.
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            dirs.ensure().unwrap();
            std::fs::write(&dirs.db_path, []).unwrap();
            assert_eq!(
                probe_startup(&dirs),
                StartupProbe::MissingOrEmpty {
                    live: LivePathState::EmptyFile,
                    scan_error: None
                }
            );
        }

        // 3. Scan uncertainty (backups_dir replaced by regular file): Recovery opens and Start fresh stays enabled.
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            dirs.ensure().unwrap();
            std::fs::remove_dir(&dirs.backups_dir).unwrap();
            std::fs::write(&dirs.backups_dir, b"not a directory").unwrap();

            assert!(matches!(
                probe_startup(&dirs),
                StartupProbe::MissingOrEmpty {
                    live: LivePathState::Missing,
                    scan_error: Some(_)
                }
            ));
            let AppSession::Recovery(state) = crate::open_session(dirs).unwrap() else {
                panic!("scan uncertainty must keep Recovery UI available");
            };
            assert!(matches!(state.error, AppError::RecoveryScanUncertain(_)));
            assert!(state.can_start_fresh());
            assert!(!state.detail(Language::En).trim().is_empty());
            assert!(!state.detail(Language::PtBr).trim().is_empty());
        }

        // 4. Indeterminate live metadata (athena.db is a directory): Recovery opens, Start fresh disabled.
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            dirs.ensure().unwrap();
            std::fs::create_dir(&dirs.db_path).unwrap();

            assert!(matches!(
                probe_startup(&dirs),
                StartupProbe::IndeterminateLive(_)
            ));
            let AppSession::Recovery(state) = crate::open_session(dirs.clone()).unwrap() else {
                panic!("indeterminate live metadata must enter Recovery");
            };
            assert!(matches!(
                state.error,
                AppError::LiveMetadataIndeterminate(_)
            ));
            assert!(!state.can_start_fresh());
            assert!(start_fresh(&dirs, now()).is_err());
            assert!(dirs.db_path.is_dir());
        }
    }

    #[cfg(windows)]
    struct DenyWriteAttributesGuard {
        path: PathBuf,
    }

    #[cfg(windows)]
    impl DenyWriteAttributesGuard {
        fn apply(path: &Path) -> Self {
            let output = std::process::Command::new("icacls")
                .arg(path)
                .args(["/deny", "*S-1-1-0:(WA)"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "icacls /deny failed: stdout={} stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            Self {
                path: path.to_path_buf(),
            }
        }
    }

    #[cfg(windows)]
    impl Drop for DenyWriteAttributesGuard {
        fn drop(&mut self) {
            if self.path.exists() {
                let _ = std::process::Command::new("icacls")
                    .arg(&self.path)
                    .args(["/remove:d", "*S-1-1-0"])
                    .output();
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_write_attributes_denied_move_noclobber_and_recovery_preserve_original() {
        // 1. Primitive validation: WriteAttributes denied + Delete allowed never loses `src` on
        // failure and still moves cleanly when `dest` is free.
        {
            let dir = tempfile::tempdir().unwrap();
            let src = dir.path().join("src.db");
            let occupied = dir.path().join("occupied.db");
            let free = dir.path().join("free.db");
            std::fs::write(&src, b"precious original").unwrap();
            std::fs::write(&occupied, b"already here").unwrap();

            let _guard = DenyWriteAttributesGuard::apply(&src);
            let err = move_noclobber(&src, &occupied).unwrap_err();
            assert!(matches!(err, AppError::Io(_)));
            assert_eq!(std::fs::read(&src).unwrap(), b"precious original");
            assert_eq!(std::fs::read(&occupied).unwrap(), b"already here");

            move_noclobber(&src, &free).unwrap();
            let _free_guard = DenyWriteAttributesGuard { path: free.clone() };
            assert!(!src.exists());
            assert_eq!(std::fs::read(&free).unwrap(), b"precious original");
        }

        // 2. Recovery validation: failed publish keeps WriteAttributes-denied `athena.db` intact;
        // normal completion moves it to `broken-*.db` without attribute modification errors.
        {
            let (_dir, dirs, source) = broken_env();
            let _guard = DenyWriteAttributesGuard::apply(&dirs.db_path);

            let mut fail_hooks = RecoveryHooks::with_publish(|_staged, _live| {
                HookPublish::FailWithRollback("simulated failure".into())
            });
            let err = restore_with_hooks(&dirs, &source, now(), &mut fail_hooks).unwrap_err();
            assert!(err.to_string().contains("simulated failure"));
            assert_eq!(
                std::fs::read(&dirs.db_path).unwrap(),
                b"this is not a database"
            );

            let broken = restore_over_broken(&dirs, &source, now()).unwrap();
            let _broken_guard = DenyWriteAttributesGuard {
                path: broken.clone(),
            };
            assert_eq!(std::fs::read(&broken).unwrap(), b"this is not a database");
            assert!(Db::open(&dirs.db_path).is_ok());
        }
    }

    #[test]
    fn two_name_state_fails_closed_on_distinct_files_and_reconciles_same_file_on_unix() {
        // Distinct files with identical bytes at both `live` and `broken` MUST fail closed.
        {
            let (_dir, dirs, source) = broken_env();
            let mut hooks = RecoveryHooks::with_step(|dirs, step| {
                if step == HookStep::BeforePublish {
                    let broken = dirs.root.join("broken-2026-11-16_12-00-00.db");
                    std::fs::copy(&broken, &dirs.db_path).unwrap();
                    return HookAction::InterruptWithoutRollback;
                }
                HookAction::Continue
            });
            restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();

            let outcome = reconcile_intent(&dirs).unwrap();
            assert!(matches!(outcome, ReconcileOutcome::Conflict(_)));
            assert!(dirs.db_path.is_file());
            assert!(dirs.root.join("broken-2026-11-16_12-00-00.db").is_file());
            assert!(intent_path(&dirs).exists());
        }

        // Published intent with distinct regular file lingering at staged MUST fail closed.
        {
            let (_dir, dirs, source) = broken_env();
            let mut hooks = RecoveryHooks::with_step(|dirs, step| {
                if step == HookStep::AfterMarkPublished {
                    let staged = dirs.staging_dir.join("recovery-2026-11-16_12-00-00.db");
                    std::fs::copy(&dirs.db_path, staged).unwrap();
                    return HookAction::InterruptWithoutRollback;
                }
                HookAction::Continue
            });
            restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();

            let outcome = reconcile_intent(&dirs).unwrap();
            assert!(matches!(outcome, ReconcileOutcome::Conflict(_)));
            assert!(intent_path(&dirs).exists());
            assert!(
                dirs.staging_dir
                    .join("recovery-2026-11-16_12-00-00.db")
                    .is_file()
            );
        }

        #[cfg(unix)]
        {
            let (_dir, dirs, source) = broken_env();
            let mut hooks = RecoveryHooks::with_step(|dirs, step| {
                if step == HookStep::BeforeMoveOriginal("") {
                    let broken = dirs.root.join("broken-2026-11-16_12-00-00.db");
                    std::fs::hard_link(&dirs.db_path, broken).unwrap();
                    return HookAction::InterruptWithoutRollback;
                }
                HookAction::Continue
            });
            restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();

            assert_eq!(
                reconcile_intent(&dirs).unwrap(),
                ReconcileOutcome::RolledBack
            );
            assert_eq!(
                std::fs::read(&dirs.db_path).unwrap(),
                b"this is not a database"
            );
            assert!(broken_files(&dirs).is_empty());
            assert!(!intent_path(&dirs).exists());
        }
    }

    #[test]
    fn normal_publication_rejects_unexpected_live_sidecar_before_publish_and_after_mark_published()
    {
        // 1. Rogue live `-wal` appearing after originals moved aside fails closed before publish.
        {
            let (_dir, dirs, source) = broken_env();
            let mut hooks = RecoveryHooks::with_step(|dirs, step| {
                if step == HookStep::BeforePublish {
                    std::fs::write(with_suffix(&dirs.db_path, "-wal"), b"alien live wal").unwrap();
                }
                HookAction::Continue
            });
            let err = restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();
            assert!(matches!(err, AppError::RecoveryConflict(_)), "{err:?}");
            assert_eq!(
                std::fs::read(with_suffix(&dirs.db_path, "-wal")).unwrap(),
                b"alien live wal"
            );
            assert!(!dirs.db_path.exists());
            assert!(intent_path(&dirs).exists());
        }

        // 2. Rogue live `-wal` appearing after `AfterMarkPublished` fails verification before
        // clearing intent or opening `Active`.
        {
            let (_dir, dirs, source) = broken_env();
            let mut hooks = RecoveryHooks::with_step(|dirs, step| {
                if step == HookStep::AfterMarkPublished {
                    std::fs::write(with_suffix(&dirs.db_path, "-wal"), b"post publish wal")
                        .unwrap();
                }
                HookAction::Continue
            });
            let err = restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();
            assert!(matches!(err, AppError::RecoveryConflict(_)), "{err:?}");
            assert!(intent_path(&dirs).exists());
            assert!(matches!(
                crate::open_session(dirs).unwrap(),
                AppSession::Recovery(_)
            ));
        }
    }

    #[test]
    fn start_fresh_rechecks_missing_or_empty_at_move_boundary_without_clobbering_nonempty_live() {
        // 1. Live starts empty (0 bytes) and becomes non-empty before `move_originals_aside`.
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            dirs.ensure().unwrap();
            std::fs::write(&dirs.db_path, []).unwrap();
            std::fs::write(with_suffix(&dirs.db_path, "-wal"), b"orphan wal").unwrap();

            let mut hooks = RecoveryHooks::with_step(|dirs, step| {
                if step == HookStep::BeforeMoveOriginal("") {
                    std::fs::write(&dirs.db_path, b"unexpected nonempty db").unwrap();
                }
                HookAction::Continue
            });
            let err = start_fresh_with_hooks(&dirs, now(), &mut hooks).unwrap_err();
            assert!(matches!(err, AppError::RecoveryConflict(_)), "{err:?}");
            assert_eq!(
                std::fs::read(&dirs.db_path).unwrap(),
                b"unexpected nonempty db"
            );
            assert!(broken_files(&dirs).is_empty());
        }

        // 2. Live starts missing (with orphan `-wal`) and becomes non-empty before publish.
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            dirs.ensure().unwrap();
            std::fs::write(with_suffix(&dirs.db_path, "-wal"), b"orphan wal").unwrap();

            let mut hooks = RecoveryHooks::with_step(|dirs, step| {
                if step == HookStep::BeforePublish {
                    std::fs::write(&dirs.db_path, b"unexpected nonempty db").unwrap();
                }
                HookAction::Continue
            });
            let err = start_fresh_with_hooks(&dirs, now(), &mut hooks).unwrap_err();
            assert!(matches!(err, AppError::RecoveryConflict(_)), "{err:?}");
            assert_eq!(
                std::fs::read(&dirs.db_path).unwrap(),
                b"unexpected nonempty db"
            );
            assert!(!with_suffix(&dirs.db_path, "-wal").exists());
            let broken_wal = dirs.root.join("broken-2026-11-16_12-00-00.db-wal");
            assert_eq!(std::fs::read(broken_wal).unwrap(), b"orphan wal");
        }
    }

    #[test]
    fn sidecar_rollback_rechecks_live_main_immediately_before_each_sidecar() {
        let (_dir, dirs, source) = broken_env();
        let mut hooks = RecoveryHooks::with_step(|dirs, step| match step {
            HookStep::BeforePublish => HookAction::FailWithRollback("trigger rollback".into()),
            HookStep::BeforeRollbackSuffix("-shm") => {
                // Corrupt/replace the already-restored live main right before `-shm` rolls back.
                std::fs::write(&dirs.db_path, b"tampered main mid rollback").unwrap();
                HookAction::Continue
            }
            _ => HookAction::Continue,
        });

        let err = restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();
        assert!(matches!(err, AppError::RecoveryConflict(_)), "{err:?}");
        assert_eq!(
            std::fs::read(&dirs.db_path).unwrap(),
            b"tampered main mid rollback"
        );
        assert!(!with_suffix(&dirs.db_path, "-shm").exists());
        let broken_shm = dirs.root.join("broken-2026-11-16_12-00-00.db-shm");
        assert_eq!(std::fs::read(broken_shm).unwrap(), b"stale shm");
    }

    #[test]
    fn staged_sidecar_slots_are_startup_evidence_and_preserve_conflict_state() {
        // 1. Orphan staged sidecar with missing live DB probes as MissingOrEmpty (not FirstLaunch).
        {
            let dir = tempfile::tempdir().unwrap();
            let dirs = AppDirs::from_root(dir.path().to_path_buf());
            dirs.ensure().unwrap();
            std::fs::write(
                dirs.staging_dir.join("recovery-2026-11-16_12-00-00.db-wal"),
                b"orphan staged wal",
            )
            .unwrap();

            assert_eq!(
                probe_startup(&dirs),
                StartupProbe::MissingOrEmpty {
                    live: LivePathState::Missing,
                    scan_error: None
                }
            );
            assert!(matches!(
                crate::open_session(dirs).unwrap(),
                AppSession::Recovery(_)
            ));
        }

        // 2. Valid interrupted intent + unexpected staged `-wal` fails closed and preserves both
        // the staged `-wal` and `recovery-intent.json`.
        {
            let (_dir, dirs, source) = broken_env();
            let mut hooks = RecoveryHooks::with_step(|dirs, step| {
                if step == HookStep::BeforePublish {
                    std::fs::write(
                        dirs.staging_dir.join("recovery-2026-11-16_12-00-00.db-wal"),
                        b"unexpected staged wal",
                    )
                    .unwrap();
                    return HookAction::InterruptWithoutRollback;
                }
                HookAction::Continue
            });
            restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();

            let outcome = reconcile_intent(&dirs).unwrap();
            assert!(
                matches!(outcome, ReconcileOutcome::Conflict(_)),
                "{outcome:?}"
            );
            assert!(intent_path(&dirs).exists());
            assert_eq!(
                std::fs::read(dirs.staging_dir.join("recovery-2026-11-16_12-00-00.db-wal"))
                    .unwrap(),
                b"unexpected staged wal"
            );
        }

        // 3. Cleanup after rollback or error never deletes unrecognized staged sidecars once the
        // candidate was prepared standalone.
        {
            let (_dir, dirs, source) = broken_env();
            let mut hooks = RecoveryHooks::with_step(|dirs, step| match step {
                HookStep::BeforePublish => HookAction::FailWithRollback("trigger rollback".into()),
                HookStep::BeforeRollbackPhase => {
                    for suffix in SIDECARS {
                        std::fs::write(
                            dirs.staging_dir
                                .join(format!("recovery-2026-11-16_12-00-00.db{suffix}")),
                            format!("unrecognized staged {suffix}"),
                        )
                        .unwrap();
                    }
                    HookAction::Continue
                }
                _ => HookAction::Continue,
            });
            let err = restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();
            assert!(err.to_string().contains("trigger rollback"), "{err:?}");
            assert!(
                !dirs
                    .staging_dir
                    .join("recovery-2026-11-16_12-00-00.db")
                    .exists()
            );
            for suffix in SIDECARS {
                assert_eq!(
                    std::fs::read(
                        dirs.staging_dir
                            .join(format!("recovery-2026-11-16_12-00-00.db{suffix}"))
                    )
                    .unwrap(),
                    format!("unrecognized staged {suffix}").as_bytes()
                );
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn retry_after_windows_sharing_blocked_published_cleanup_activates_without_rerolling() {
        use std::os::windows::fs::OpenOptionsExt;

        // 1. Restore retry via `restore_over_broken` and `app::recovery::restore_and_activate`.
        {
            let (_dir, dirs, source) = broken_env();
            let lock_holder: std::sync::Arc<std::sync::Mutex<Option<std::fs::File>>> =
                std::sync::Arc::new(std::sync::Mutex::new(None));
            let hook_lock = lock_holder.clone();

            let mut hooks = RecoveryHooks::with_step(move |dirs, step| {
                if step == HookStep::AfterMarkPublished {
                    // Open `recovery-intent.json` without `FILE_SHARE_DELETE` so cleanup fails
                    // with a genuine Windows sharing violation (`os error 32`).
                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(intent_path(dirs))
                        .unwrap();
                    *hook_lock.lock().unwrap() = Some(file);
                }
                HookAction::Continue
            });
            let err = restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();
            assert!(matches!(err, AppError::Io(_)), "{err:?}");
            assert!(intent_path(&dirs).exists());

            // Release the sharing lock; retrying activates the validated published DB and keeps
            // the original single `broken-*` snapshot without creating a second replacement.
            drop(lock_holder.lock().unwrap().take());

            let expected_broken = dirs.root.join("broken-2026-11-16_12-00-00.db");
            let retried = restore_over_broken(&dirs, &source, now()).unwrap();
            assert_eq!(retried, expected_broken);
            let mut broken = broken_files(&dirs);
            broken.sort();
            assert_eq!(
                broken,
                vec![
                    expected_broken.clone(),
                    with_suffix(&expected_broken, "-shm"),
                    with_suffix(&expected_broken, "-wal"),
                ]
            );
            assert!(!intent_path(&dirs).exists());
            assert!(Db::open(&dirs.db_path).is_ok());
        }

        // 2. End-to-end `RecoveryState` / `restore_and_activate` retry while sharing lock clears.
        {
            let (_dir, dirs, source) = broken_env();
            let lock_holder: std::sync::Arc<std::sync::Mutex<Option<std::fs::File>>> =
                std::sync::Arc::new(std::sync::Mutex::new(None));
            let hook_lock = lock_holder.clone();

            let mut hooks = RecoveryHooks::with_step(move |dirs, step| {
                if step == HookStep::AfterMarkPublished {
                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(0)
                        .open(intent_path(dirs))
                        .unwrap();
                    *hook_lock.lock().unwrap() = Some(file);
                }
                HookAction::Continue
            });
            restore_with_hooks(&dirs, &source, now(), &mut hooks).unwrap_err();

            // Refresh UI state while lock is held, then release lock and retry activation.
            let mut state = crate::RecoveryState::new(
                dirs.clone(),
                AppError::Corrupt {
                    path: dirs.db_path.display().to_string(),
                },
            );
            state.refresh_after_error(&AppError::Io(std::io::Error::from_raw_os_error(32)));
            assert!(matches!(state.error, AppError::RecoveryConflict(_)));
            drop(lock_holder.lock().unwrap().take());

            let expected_broken = dirs.root.join("broken-2026-11-16_12-00-00.db");
            let core = crate::app::recovery::restore_and_activate(&mut state, &source);
            assert!(core.is_some(), "retry must activate published DB");
            assert!(!intent_path(&dirs).exists());
            let mut broken = broken_files(&dirs);
            broken.sort();
            assert_eq!(
                broken,
                vec![
                    expected_broken.clone(),
                    with_suffix(&expected_broken, "-shm"),
                    with_suffix(&expected_broken, "-wal"),
                ]
            );
        }
    }

    fn wal_backed_broken_env() -> (
        tempfile::TempDir,
        AppDirs,
        PathBuf,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();

        let source = dirs.backups_dir.join("manual-good.db");
        {
            let db = Db::open(&source).unwrap();
            profile_repo::save(
                db.conn(),
                &Profile {
                    start_balance: Money::from_cents(25_000),
                    theme: Theme::Dark,
                    language: Language::En,
                    onboarding_complete: true,
                },
            )
            .unwrap();
            let (_, _) = seed_people(db.conn());
            seed_month(&db);
        }

        // Build a real WAL-backed SQLite database with active uncheckpointed WAL & SHM frames,
        // snapshot those sidecar bytes while open, then corrupt the main database header.
        let (wal_bytes, shm_bytes) = {
            let db = Db::open(&dirs.db_path).unwrap();
            db.conn()
                .execute_batch("PRAGMA wal_autocheckpoint = 0;")
                .unwrap();
            profile_repo::save(
                db.conn(),
                &Profile {
                    start_balance: Money::from_cents(99_999),
                    theme: Theme::Light,
                    language: Language::PtBr,
                    onboarding_complete: true,
                },
            )
            .unwrap();
            let wal = std::fs::read(with_suffix(&dirs.db_path, "-wal")).unwrap();
            let shm = std::fs::read(with_suffix(&dirs.db_path, "-shm")).unwrap();
            assert!(!wal.is_empty(), "real WAL fixture must contain WAL frames");
            assert!(!shm.is_empty(), "real WAL fixture must have SHM state");
            (wal, shm)
        };
        let mut db_bytes = std::fs::read(&dirs.db_path).unwrap();
        db_bytes[..16].copy_from_slice(b"CORRUPT-WAL-HDR!");
        std::fs::write(&dirs.db_path, &db_bytes).unwrap();
        std::fs::write(with_suffix(&dirs.db_path, "-wal"), &wal_bytes).unwrap();
        std::fs::write(with_suffix(&dirs.db_path, "-shm"), &shm_bytes).unwrap();

        (dir, dirs, source, db_bytes, wal_bytes, shm_bytes)
    }

    fn run_child_recovery_worker(root: &str) {
        use std::io::Write;

        let dirs = AppDirs::from_root(PathBuf::from(root));
        let source = PathBuf::from(std::env::var("ATHENA_TEST_RECOVERY_SOURCE").unwrap());
        let case = std::env::var("ATHENA_TEST_RECOVERY_CASE").unwrap();
        let _lock = crate::app::instance::acquire(&dirs.root).unwrap();

        let mut trigger_rollback = case == "BeforeRollbackWal";
        let mut hooks = RecoveryHooks::with_step(move |_dirs, step| {
            if case == "BeforeRollbackWal" && trigger_rollback && step == HookStep::BeforePublish {
                trigger_rollback = false;
                return HookAction::FailWithRollback("trigger rollback".into());
            }
            let matched = match case.as_str() {
                "AfterWriteIntent" => step == HookStep::AfterWriteIntent,
                "AfterMoveOriginalMain" => step == HookStep::AfterMoveOriginal(""),
                "BeforePublish" => step == HookStep::BeforePublish,
                "AfterMarkPublished" => step == HookStep::AfterMarkPublished,
                "BeforeRollbackWal" => step == HookStep::BeforeRollbackSuffix("-wal"),
                _ => false,
            };
            if matched {
                println!("READY");
                let _ = std::io::stdout().flush();
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            HookAction::Continue
        });
        let _ = restore_with_hooks(&dirs, &source, now(), &mut hooks);
    }

    fn verify_child_interruption_case(exe: &Path, case: &str, expect_active: bool) {
        use std::io::BufRead;
        use std::process::Stdio;

        let (_dir, dirs, source, db_bytes, wal_bytes, shm_bytes) = wal_backed_broken_env();
        let mut child = std::process::Command::new(exe)
            .args([
                "--exact",
                "infrastructure::db::recovery::tests::child_process_recovery_interruption_with_wal_fixture",
                "--nocapture",
            ])
            .env("ATHENA_TEST_RECOVERY_ROOT", &dirs.root)
            .env("ATHENA_TEST_RECOVERY_SOURCE", &source)
            .env("ATHENA_TEST_RECOVERY_CASE", case)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        let stdout = child.stdout.take().unwrap();
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        let mut ready = false;
        while reader.read_line(&mut line).unwrap() > 0 {
            if line.trim() == "READY" {
                ready = true;
                break;
            }
            line.clear();
        }
        assert!(ready, "child failed to reach READY for {case}");

        // While child is alive, `athena.lock` blocks another instance cleanly.
        assert!(matches!(
            crate::app::instance::acquire(&dirs.root),
            Err(AppError::AlreadyRunning)
        ));

        child.kill().unwrap();
        let _ = child.wait();

        // OS releases `athena.lock` on child process termination.
        let _guard = crate::app::instance::acquire(&dirs.root)
            .expect("athena.lock must release after child kill");

        let session = crate::open_session(dirs.clone()).unwrap();
        if expect_active {
            assert!(
                matches!(session, AppSession::Active(_)),
                "{case}: post-publish kill must open Active"
            );
            assert!(!intent_path(&dirs).exists());
            assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
            let broken = dirs.root.join("broken-2026-11-16_12-00-00.db");
            assert_eq!(std::fs::read(&broken).unwrap(), db_bytes);
            assert_eq!(
                std::fs::read(with_suffix(&broken, "-wal")).unwrap(),
                wal_bytes
            );
            assert_eq!(
                std::fs::read(with_suffix(&broken, "-shm")).unwrap(),
                shm_bytes
            );
        } else {
            let AppSession::Recovery(state) = session else {
                panic!("{case}: interrupted recovery must roll back to Recovery");
            };
            assert!(matches!(state.error, AppError::Corrupt { .. }));
            assert_eq!(std::fs::read(&dirs.db_path).unwrap(), db_bytes);
            assert_eq!(
                std::fs::read(with_suffix(&dirs.db_path, "-wal")).unwrap(),
                wal_bytes
            );
            assert_eq!(
                std::fs::read(with_suffix(&dirs.db_path, "-shm")).unwrap(),
                shm_bytes
            );
            assert!(broken_files(&dirs).is_empty());
            assert!(!intent_path(&dirs).exists());
            assert_eq!(std::fs::read_dir(&dirs.staging_dir).unwrap().count(), 0);
        }
    }

    #[test]
    fn child_process_recovery_interruption_with_wal_fixture() {
        if let Ok(root) = std::env::var("ATHENA_TEST_RECOVERY_ROOT") {
            run_child_recovery_worker(&root);
            return;
        }

        let exe = std::env::current_exe().unwrap();
        for (case, expect_active) in [
            ("AfterWriteIntent", false),
            ("AfterMoveOriginalMain", false),
            ("BeforePublish", false),
            ("BeforeRollbackWal", false),
            ("AfterMarkPublished", true),
        ] {
            verify_child_interruption_case(&exe, case, expect_active);
        }
    }
}
