//! The Settings → Appearance edit session (contract §14.2): a draft that previews live in the shell
//! until the user applies, cancels or resets it. It never touches storage; Apply builds an
//! [`AppAction`] and the store commits it.

use super::actions::{AppAction, AppearanceApply};
use super::view_model::ViewModel;
use crate::domain::appearance::{AppearancePrefs, Wallpaper};
use crate::infrastructure::media::MediaError;
use crate::infrastructure::media::assets::StagedMedia;

/// What the live preview reports back to the page: decode progress and which effective fallbacks
/// are in force. Filled by the app each frame from the resolved look and the wallpaper runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct PreviewStatus {
    /// The previewed wallpaper is still decoding.
    pub loading: bool,
    /// The previewed wallpaper cannot be used.
    pub failure: Option<MediaError>,
    pub high_contrast: bool,
    pub reduced_motion: bool,
    /// Reduced motion comes from the OS, not from the user's choice.
    pub motion_forced_by_os: bool,
    /// Solid surfaces come from the OS or high contrast, not from the user's choice.
    pub transparency_forced: bool,
}

#[derive(Debug, Clone)]
pub struct AppearanceEdit {
    /// What was applied when the session began (or the profile-derived defaults).
    pub baseline: AppearancePrefs,
    pub draft: AppearancePrefs,
    /// The picked-but-unapplied file behind a new local wallpaper.
    pub staged: Option<StagedMedia>,
    /// The picked file was rejected before it could be previewed.
    pub pick_error: Option<MediaError>,
    /// The last Apply failed; the draft and staged file are kept.
    pub apply_failed: bool,
    pub apply_error: Option<MediaError>,
    pub status: PreviewStatus,
}

impl AppearanceEdit {
    /// Start from the applied preferences, or the defaults derived from the profile theme.
    #[must_use]
    pub fn start(vm: &ViewModel) -> Self {
        let baseline = vm
            .appearance
            .clone()
            .unwrap_or_else(|| AppearancePrefs::defaults_for(vm.profile_theme()));
        Self {
            draft: baseline.clone(),
            baseline,
            staged: None,
            pick_error: None,
            apply_failed: false,
            apply_error: None,
            status: PreviewStatus::default(),
        }
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.draft != self.baseline
    }

    /// Apply needs a change, a finished decode and no rejected file.
    #[must_use]
    pub fn can_apply(&self) -> bool {
        self.is_dirty()
            && !self.status.loading
            && self.status.failure.is_none()
            && self.pick_error.is_none()
    }

    /// Drop every edit and the staged file; the shell re-resolves the applied look.
    pub fn cancel(&mut self) {
        self.draft = self.baseline.clone();
        self.staged = None;
        self.pick_error = None;
        self.apply_failed = false;
        self.apply_error = None;
    }

    /// Choose a wallpaper that needs no file (or restores the applied one).
    pub fn choose(&mut self, wallpaper: Wallpaper) {
        self.draft.wallpaper = wallpaper;
        self.staged = None;
        self.pick_error = None;
        self.apply_failed = false;
        self.apply_error = None;
    }

    /// Choose a freshly picked file.
    pub fn choose_file(&mut self, staged: StagedMedia) {
        self.draft.wallpaper = Wallpaper::Local(staged.id.clone());
        self.staged = Some(staged);
        self.pick_error = None;
        self.apply_failed = false;
        self.apply_error = None;
    }

    /// The Apply action for the draft, carrying the staged file when the wallpaper is new.
    #[must_use]
    pub fn apply_action(&self) -> AppAction {
        let media = self
            .staged
            .clone()
            .filter(|s| self.draft.wallpaper.asset_id() == Some(&s.id));
        AppAction::ApplyAppearance(Box::new(AppearanceApply {
            prefs: self.draft.clone(),
            media,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::appearance::{Accent, AssetId, Preset};
    use crate::domain::settings::Theme;

    fn vm() -> ViewModel {
        let mut vm = ViewModel::new(crate::domain::settings::Language::En);
        vm.profile = Some(crate::shared::test_fixtures::profile());
        vm
    }

    fn staged(id: &str) -> StagedMedia {
        StagedMedia {
            id: AssetId::parse(id).unwrap(),
            bytes: Arc::new(vec![1, 2, 3]),
        }
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn a_session_starts_clean_from_the_profile_derived_defaults() {
        let vm = vm();
        let edit = AppearanceEdit::start(&vm);
        assert_eq!(edit.draft, AppearancePrefs::defaults_for(Theme::Dark));
        assert!(!edit.is_dirty() && !edit.can_apply());
    }

    #[test]
    fn apply_needs_a_change_a_finished_decode_and_no_rejected_file() {
        let mut edit = AppearanceEdit::start(&vm());
        edit.draft = edit.draft.clone().with_preset(Preset::Aurora);
        assert!(edit.is_dirty() && edit.can_apply());
        edit.status.loading = true;
        assert!(!edit.can_apply(), "disabled while loading");
        edit.status.loading = false;
        edit.status.failure = Some(MediaError::Corrupt);
        assert!(!edit.can_apply(), "disabled for an invalid image");
        edit.status.failure = None;
        edit.pick_error = Some(MediaError::Unsupported);
        assert!(!edit.can_apply(), "disabled after a rejected pick");
        edit.pick_error = None;
        assert!(edit.can_apply());
    }

    #[test]
    fn cancel_restores_the_baseline_and_drops_the_staged_file() {
        let mut edit = AppearanceEdit::start(&vm());
        edit.draft.accent = Accent::Violet;
        edit.choose_file(staged(A));
        edit.apply_failed = true;
        assert!(edit.is_dirty() && edit.staged.is_some());
        edit.cancel();
        assert_eq!(edit.draft, edit.baseline);
        assert!(edit.staged.is_none() && !edit.apply_failed && !edit.is_dirty());
    }

    #[test]
    fn choosing_a_file_then_removing_it_leaves_no_staged_bytes() {
        let mut edit = AppearanceEdit::start(&vm());
        edit.choose_file(staged(A));
        assert!(matches!(edit.draft.wallpaper, Wallpaper::Local(_)));
        // Replace with another file, then remove.
        edit.choose_file(staged(B));
        assert_eq!(edit.staged.as_ref().unwrap().id.as_str(), B);
        edit.choose(Wallpaper::None);
        assert!(edit.staged.is_none());
        assert!(!edit.is_dirty(), "back to the baseline wallpaper");
    }

    #[test]
    fn the_apply_action_carries_only_the_staged_file_that_is_selected() {
        let mut edit = AppearanceEdit::start(&vm());
        edit.choose_file(staged(A));
        let AppAction::ApplyAppearance(apply) = edit.apply_action() else {
            panic!("expected Apply")
        };
        assert_eq!(apply.media.as_ref().unwrap().id.as_str(), A);
        assert_eq!(apply.prefs, edit.draft);
        // A stale staged file (the wallpaper moved on) is not attached.
        edit.draft.wallpaper = Wallpaper::Builtin;
        let AppAction::ApplyAppearance(apply) = edit.apply_action() else {
            panic!("expected Apply")
        };
        assert!(apply.media.is_none());
    }
}
