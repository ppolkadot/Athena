//! A short confirmation that appears bottom-right, optionally with an Undo button, and fades
//! away unless hovered or focused.

use eframe::egui::{self, Align2, Area, Frame, Id, Margin, Order, RichText, Stroke, Vec2};

use crate::app::actions::AppAction;
use crate::domain::settings::{Language, Theme};
use crate::i18n::strings;
use crate::ui::theme::palette;

/// How long a toast stays up, in seconds.
pub const LIFETIME: f64 = 3.0;

#[derive(Debug, Clone, PartialEq)]
pub enum ToastAction {
    Undo(AppAction),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub text: String,
    /// `ctx.input(|i| i.time)` when it was first drawn. `None` until then, so time spent blocked
    /// before the first frame (a native file dialog, say) never counts against the toast.
    pub created_at: Option<f64>,
    pub action: Option<ToastAction>,
    last_seen_at: Option<f64>,
    paused: bool,
}

impl Toast {
    #[must_use]
    pub fn new(text: impl Into<String>, now: f64) -> Self {
        Self {
            text: text.into(),
            created_at: Some(now),
            action: None,
            last_seen_at: Some(now),
            paused: false,
        }
    }

    /// A toast whose clock starts when it is first drawn.
    #[must_use]
    pub fn pending(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            created_at: None,
            action: None,
            last_seen_at: None,
            paused: false,
        }
    }

    #[must_use]
    pub fn with_action(mut self, action: Option<ToastAction>) -> Self {
        self.action = action;
        self
    }

    /// Start the clock if it has not started yet.
    pub fn stamp(&mut self, now: f64) {
        self.created_at.get_or_insert(now);
        self.last_seen_at.get_or_insert(now);
    }

    /// Advance frame time; while hovered or focused, shift `created_at` forward so the toast does
    /// not count down.
    pub fn tick(&mut self, now: f64) {
        self.stamp(now);
        let dt = (now - self.last_seen_at.unwrap_or(now)).max(0.0);
        if self.paused
            && let Some(at) = &mut self.created_at
        {
            *at += dt;
        }
        self.last_seen_at = Some(now);
    }

    #[must_use]
    pub fn is_expired(&self, now: f64) -> bool {
        self.created_at.is_some_and(|at| now - at >= LIFETIME)
    }
}

/// Draw the toast if there is one, pause expiry while hovered or focused, clear it once expired,
/// and return any triggered [`AppAction`] (such as clicking Undo).
#[allow(clippy::cast_possible_truncation)]
pub fn show(
    ctx: &egui::Context,
    toast: &mut Option<Toast>,
    theme: Theme,
    language: Language,
) -> Option<AppAction> {
    let now = ctx.input(|i| i.time);
    if let Some(t) = toast.as_mut() {
        t.tick(now);
    }
    if toast.as_ref().is_some_and(|t| t.is_expired(now)) {
        *toast = None;
    }
    let current = toast.as_mut()?;
    let p = palette(theme);
    let s = strings(language);
    let age = now - current.created_at.unwrap_or(now);
    let remaining = if current.paused {
        1.0
    } else {
        (LIFETIME - age).clamp(0.0, 0.3) / 0.3
    };

    let mut triggered = None;
    let mut undo_hovered = false;
    let mut undo_focused = false;
    let interactive = current.action.is_some();

    let area_inner = Area::new(Id::unique("toast"))
        .order(Order::Tooltip)
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-24.0, -24.0))
        .interactable(interactive)
        .show(ctx, |ui| {
            ui.set_opacity(remaining as f32);
            Frame::new()
                .fill(p.card_hover)
                .stroke(Stroke::new(1.0, p.accent))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&current.text).color(p.text));
                        if let Some(ToastAction::Undo(action)) = &current.action {
                            ui.add_space(8.0);
                            let button =
                                egui::Button::new(RichText::new(s.undo).color(p.accent).strong())
                                    .frame(false);
                            let resp = ui.add(button);
                            undo_hovered = resp.hovered();
                            undo_focused = resp.has_focus();
                            if resp.clicked() {
                                triggered = Some(action.clone());
                            }
                        }
                    });
                });
        });

    if triggered.is_some() {
        *toast = None;
        return triggered;
    }

    let rect = area_inner.response.rect;
    let pointer_in_rect = ctx.input(|i| {
        i.pointer
            .hover_pos()
            .or_else(|| i.pointer.latest_pos())
            .is_some_and(|pos| rect.contains(pos))
    });
    current.paused = pointer_in_rect || undo_hovered || undo_focused;
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toast_expires_after_its_lifetime() {
        let toast = Toast::new("Saved", 10.0);
        assert!(!toast.is_expired(10.0));
        assert!(!toast.is_expired(12.9));
        assert!(toast.is_expired(13.0));
    }

    /// Found by exporting from the real window: the native save dialog blocks the app for seconds,
    /// so a toast stamped before it was already expired by the next frame and never showed.
    #[test]
    fn a_pending_toast_starts_its_clock_when_first_drawn() {
        let mut toast = Toast::pending("Exported");
        assert!(
            !toast.is_expired(1_000.0),
            "no clock yet, so it cannot have expired"
        );
        toast.stamp(50.0);
        toast.stamp(60.0);
        assert_eq!(
            toast.created_at,
            Some(50.0),
            "only the first frame stamps it"
        );
        assert!(!toast.is_expired(52.0));
        assert!(toast.is_expired(53.0));
    }

    #[test]
    fn toast_lifetime_pauses_while_hovered() {
        use egui_kittest::kittest::Queryable;

        let toast = Some(
            Toast::pending("Deleted “Coffee”.")
                .with_action(Some(ToastAction::Undo(AppAction::RestoreTransaction(1)))),
        );
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(600.0, 200.0))
            .build_ui_state(
                |ui, toast: &mut Option<Toast>| {
                    crate::ui::theme::apply(ui.ctx(), Theme::Dark);
                    show(ui.ctx(), toast, Theme::Dark, Language::En);
                },
                toast,
            );
        harness.run_steps(2);
        assert!(harness.state().is_some());

        // Move the pointer over the Undo button and process hover before advancing past LIFETIME.
        harness.get_by_label("Undo").hover();
        harness.run_steps(2);
        harness.run_steps(16);
        assert!(
            harness.state().is_some(),
            "toast stays visible while hovered past lifetime"
        );

        // Move pointer away and let LIFETIME elapse: now it expires.
        harness.hover_at(egui::pos2(0.0, 0.0));
        harness.run_steps(2);
        assert!(harness.state().is_some());

        harness.run_steps(14);
        assert!(
            harness.state().is_none(),
            "toast expires once unhovered and lifetime elapses"
        );
    }
}
