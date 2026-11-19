//! Operating-system preferences that change how decorative effects behave (contract §14.2):
//! client-area animation, high contrast and transparency. Each value is `None` when unknown, and
//! unknown always resolves to the accessible choice.

#[cfg(windows)]
pub mod windows_prefs;

use std::time::{Duration, Instant};

/// An opaque `[r, g, b]` color.
pub type Rgb = [u8; 3];

/// The Windows system colors used when high contrast is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemColors {
    pub window: Rgb,
    pub window_text: Rgb,
    pub highlight: Rgb,
    pub highlight_text: Rgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighContrast {
    pub on: bool,
    pub colors: Option<SystemColors>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlatformPrefs {
    /// Client-area animation is enabled.
    pub animations: Option<bool>,
    /// Window transparency effects are enabled.
    pub transparency: Option<bool>,
    pub high_contrast: Option<HighContrast>,
}

/// A source of [`PlatformPrefs`]; tests inject their own.
pub trait PlatformPreferences {
    fn read(&self) -> PlatformPrefs;
}

/// A platform that reports nothing (every value unknown).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnknownPlatform;

impl PlatformPreferences for UnknownPlatform {
    fn read(&self) -> PlatformPrefs {
        PlatformPrefs::default()
    }
}

/// A provider that always reports the same state (tests).
#[cfg(test)]
pub struct FixedPlatform(pub PlatformPrefs);

#[cfg(test)]
impl PlatformPreferences for FixedPlatform {
    fn read(&self) -> PlatformPrefs {
        self.0
    }
}

/// The provider for the running OS.
#[must_use]
pub fn system_provider() -> Box<dyn PlatformPreferences> {
    #[cfg(windows)]
    {
        Box::new(windows_prefs::WindowsPrefs)
    }
    #[cfg(not(windows))]
    {
        Box::new(UnknownPlatform)
    }
}

/// How often a focused, visible window re-reads the preferences.
pub const FOCUSED_POLL: Duration = Duration::from_secs(2);

/// The window state the watcher needs each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    pub focused: bool,
    pub visible: bool,
}

/// Holds the latest preferences. Reads once at creation and on focus regain or resume, then every
/// [`FOCUSED_POLL`] while the window is focused and visible. Unfocused or hidden windows schedule
/// nothing, so this never wakes an idle background window.
pub struct PlatformWatch {
    provider: Box<dyn PlatformPreferences>,
    current: PlatformPrefs,
    last_read: Instant,
    was_active: bool,
}

impl PlatformWatch {
    #[must_use]
    pub fn new(provider: Box<dyn PlatformPreferences>, now: Instant) -> Self {
        let current = provider.read();
        Self {
            provider,
            current,
            last_read: now,
            was_active: false,
        }
    }

    #[must_use]
    pub fn current(&self) -> PlatformPrefs {
        self.current
    }

    fn read(&mut self, now: Instant) -> bool {
        let next = self.provider.read();
        self.last_read = now;
        let changed = next != self.current;
        self.current = next;
        changed
    }

    /// Advance one frame. Returns whether the preferences changed and when to wake next (`None`
    /// when no check is due until the window becomes active again).
    pub fn poll(&mut self, now: Instant, window: WindowState) -> (bool, Option<Duration>) {
        let active = window.focused && window.visible;
        if !active {
            self.was_active = false;
            return (false, None);
        }
        let resumed = !self.was_active;
        self.was_active = true;
        let due = now.saturating_duration_since(self.last_read) >= FOCUSED_POLL;
        let changed = if resumed || due {
            self.read(now)
        } else {
            false
        };
        let wait = FOCUSED_POLL.saturating_sub(now.saturating_duration_since(self.last_read));
        (changed, Some(wait))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[derive(Clone)]
    struct Fake(Rc<Cell<PlatformPrefs>>, Rc<Cell<u32>>);

    impl PlatformPreferences for Fake {
        fn read(&self) -> PlatformPrefs {
            self.1.set(self.1.get() + 1);
            self.0.get()
        }
    }

    fn active() -> WindowState {
        WindowState {
            focused: true,
            visible: true,
        }
    }

    fn fake(initial: PlatformPrefs) -> (Fake, Rc<Cell<PlatformPrefs>>, Rc<Cell<u32>>) {
        let cell = Rc::new(Cell::new(initial));
        let reads = Rc::new(Cell::new(0));
        (Fake(cell.clone(), reads.clone()), cell, reads)
    }

    #[test]
    fn reads_at_startup_and_on_focus_regain() {
        let t0 = Instant::now();
        let (provider, cell, reads) = fake(PlatformPrefs {
            animations: Some(true),
            ..PlatformPrefs::default()
        });
        let mut watch = PlatformWatch::new(Box::new(provider), t0);
        assert_eq!(reads.get(), 1);
        assert_eq!(watch.current().animations, Some(true));
        // The first active frame counts as a resume and reads again.
        assert!(!watch.poll(t0, active()).0);
        assert_eq!(reads.get(), 2);
        // A change while unfocused is noticed the moment focus returns.
        cell.set(PlatformPrefs {
            animations: Some(false),
            ..PlatformPrefs::default()
        });
        let blurred = WindowState {
            focused: false,
            visible: true,
        };
        assert_eq!(watch.poll(t0, blurred), (false, None));
        let (changed, _) = watch.poll(t0, active());
        assert!(changed);
        assert_eq!(watch.current().animations, Some(false));
    }

    #[test]
    fn focused_polling_runs_every_two_seconds_and_hidden_never() {
        let t0 = Instant::now();
        let (provider, cell, reads) = fake(PlatformPrefs::default());
        let mut watch = PlatformWatch::new(Box::new(provider), t0);
        watch.poll(t0, active());
        let after_resume = reads.get();
        let (changed, wait) = watch.poll(t0 + Duration::from_millis(500), active());
        assert!(!changed);
        assert_eq!(reads.get(), after_resume, "no read before the interval");
        assert_eq!(wait, Some(Duration::from_millis(1500)));
        cell.set(PlatformPrefs {
            transparency: Some(false),
            ..PlatformPrefs::default()
        });
        let (changed, wait) = watch.poll(t0 + Duration::from_secs(2), active());
        assert!(changed);
        assert_eq!(wait, Some(FOCUSED_POLL));
        // Minimized or hidden: no read, no wake-up.
        let hidden = WindowState {
            focused: true,
            visible: false,
        };
        let before = reads.get();
        assert_eq!(
            watch.poll(t0 + Duration::from_mins(1), hidden),
            (false, None)
        );
        assert_eq!(reads.get(), before);
    }

    #[test]
    fn unknown_platform_reports_nothing() {
        assert_eq!(UnknownPlatform.read(), PlatformPrefs::default());
    }
}
