//! A short confirmation that appears bottom-right and fades away by itself.

use eframe::egui::{self, Align2, Area, Frame, Id, Margin, Order, RichText, Stroke, Vec2};

use crate::domain::settings::Theme;
use crate::ui::theme::palette;

/// How long a toast stays up, in seconds.
pub const LIFETIME: f64 = 3.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub text: String,
    /// `ctx.input(|i| i.time)` when it was first drawn. `None` until then, so time spent blocked
    /// before the first frame (a native file dialog, say) never counts against the toast.
    pub created_at: Option<f64>,
}

impl Toast {
    #[must_use]
    pub fn new(text: impl Into<String>, now: f64) -> Self {
        Self {
            text: text.into(),
            created_at: Some(now),
        }
    }

    /// A toast whose clock starts when it is first drawn.
    #[must_use]
    pub fn pending(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            created_at: None,
        }
    }

    /// Start the clock if it has not started yet.
    pub fn stamp(&mut self, now: f64) {
        self.created_at.get_or_insert(now);
    }

    #[must_use]
    pub fn is_expired(&self, now: f64) -> bool {
        self.created_at.is_some_and(|at| now - at >= LIFETIME)
    }
}

#[allow(clippy::cast_possible_truncation)]
/// Draw the toast if there is one, and clear it once it has expired.
pub fn show(ctx: &egui::Context, toast: &mut Option<Toast>, theme: Theme) {
    let now = ctx.input(|i| i.time);
    if let Some(t) = toast.as_mut() {
        t.stamp(now);
    }
    if toast.as_ref().is_some_and(|t| t.is_expired(now)) {
        *toast = None;
    }
    let Some(current) = toast else { return };
    let p = palette(theme);
    let age = now - current.created_at.unwrap_or(now);
    let remaining = (LIFETIME - age).clamp(0.0, 0.3) / 0.3;
    Area::new(Id::unique("toast"))
        .order(Order::Tooltip)
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-24.0, -24.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_opacity(remaining as f32);
            Frame::new()
                .fill(p.card_hover)
                .stroke(Stroke::new(1.0, p.accent))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.label(RichText::new(&current.text).color(p.text));
                });
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(100));
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
}
