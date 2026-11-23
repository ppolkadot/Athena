//! Operating-system preferences that change how decorative effects behave (contract §14.2):
//! client-area animation, high contrast and transparency. Each value is `None` when unknown, and
//! unknown always resolves to the accessible choice.

#[cfg(windows)]
pub mod windows_prefs;

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

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
pub trait PlatformPreferences: Send {
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

/// The window state the watcher needs each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    pub focused: bool,
    pub visible: bool,
}

/// Reads at startup and resume. On Windows, a worker checks while focused and visible
/// and notifies the app only when a sample changes.
pub struct PlatformWatch {
    shared: Arc<SharedWatch>,
    worker: Option<JoinHandle<()>>,
}

struct SharedWatch {
    state: Mutex<WatchState>,
    activity: Condvar,
}

struct WatchState {
    provider: Box<dyn PlatformPreferences>,
    current: PlatformPrefs,
    active: bool,
    stopped: bool,
}

impl WatchState {
    fn read(&mut self) -> bool {
        let next = self.provider.read();
        let changed = next != self.current;
        self.current = next;
        changed
    }
}

impl PlatformWatch {
    #[must_use]
    pub fn new(provider: Box<dyn PlatformPreferences>) -> Self {
        let current = provider.read();
        Self {
            shared: Arc::new(SharedWatch {
                state: Mutex::new(WatchState {
                    provider,
                    current,
                    active: false,
                    stopped: false,
                }),
                activity: Condvar::new(),
            }),
            worker: None,
        }
    }

    /// Start the system watcher. The callback may run on the Windows worker thread.
    #[must_use]
    pub fn system(on_change: impl Fn() + Send + 'static) -> Self {
        #[cfg(windows)]
        let provider = windows_prefs::WindowsPrefs;
        #[cfg(not(windows))]
        let provider = UnknownPlatform;
        let mut watch = Self::new(Box::new(provider));
        if cfg!(windows) {
            watch.start_worker(Duration::from_secs(2), on_change);
        }
        watch
    }

    #[must_use]
    pub fn current(&self) -> PlatformPrefs {
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .current
    }

    /// Apply a window-state change and read immediately on focus regain or resume.
    pub fn update_window(&self, window: WindowState) -> PlatformPrefs {
        let active = window.focused && window.visible;
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state.active != active {
            state.active = active;
            if active {
                state.read();
            }
            self.shared.activity.notify_one();
        }
        state.current
    }

    fn start_worker(&mut self, interval: Duration, on_change: impl Fn() + Send + 'static) {
        let shared = Arc::clone(&self.shared);
        let worker = std::thread::Builder::new()
            .name("athena-platform-prefs".into())
            .spawn(move || shared.watch(interval, on_change));
        match worker {
            Ok(worker) => self.worker = Some(worker),
            Err(err) => tracing::warn!("could not start the platform preference watcher: {err}"),
        }
    }
}

impl SharedWatch {
    fn watch(&self, interval: Duration, on_change: impl Fn()) {
        loop {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let state = self
                .activity
                .wait_while(state, |s| !s.active && !s.stopped)
                .unwrap_or_else(PoisonError::into_inner);
            let (mut state, elapsed) = self
                .activity
                .wait_timeout_while(state, interval, |s| s.active && !s.stopped)
                .unwrap_or_else(PoisonError::into_inner);
            if state.stopped {
                break;
            }
            let changed = elapsed.timed_out() && state.active && state.read();
            // The next frame also locks the sample. Release it before asking egui to wake.
            drop(state);
            if changed {
                on_change();
            }
        }
    }
}

impl Drop for PlatformWatch {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stopped = true;
            self.shared.activity.notify_one();
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    struct Fake {
        prefs: Arc<Mutex<PlatformPrefs>>,
        reads: mpsc::Sender<()>,
    }

    impl PlatformPreferences for Fake {
        fn read(&self) -> PlatformPrefs {
            let prefs = *self.prefs.lock().unwrap();
            self.reads.send(()).unwrap();
            prefs
        }
    }

    fn active() -> WindowState {
        WindowState {
            focused: true,
            visible: true,
        }
    }

    fn fake(
        initial: PlatformPrefs,
    ) -> (PlatformWatch, Arc<Mutex<PlatformPrefs>>, mpsc::Receiver<()>) {
        let prefs = Arc::new(Mutex::new(initial));
        let (reads, samples) = mpsc::channel();
        let watch = PlatformWatch::new(Box::new(Fake {
            prefs: Arc::clone(&prefs),
            reads,
        }));
        (watch, prefs, samples)
    }

    #[test]
    fn reads_at_startup_and_on_focus_regain() {
        let (watch, prefs, reads) = fake(PlatformPrefs {
            animations: Some(true),
            ..PlatformPrefs::default()
        });
        reads.try_recv().unwrap();
        assert_eq!(watch.current().animations, Some(true));
        watch.update_window(active());
        reads.try_recv().unwrap();
        // A change while unfocused is noticed the moment focus returns.
        *prefs.lock().unwrap() = PlatformPrefs {
            animations: Some(false),
            ..PlatformPrefs::default()
        };
        watch.update_window(WindowState {
            focused: false,
            visible: true,
        });
        assert_eq!(reads.try_recv(), Err(mpsc::TryRecvError::Empty));
        watch.update_window(active());
        reads.try_recv().unwrap();
        assert_eq!(watch.current().animations, Some(false));
    }

    #[test]
    fn worker_notifies_only_on_changes_and_suspends_while_hidden() {
        let (mut watch, prefs, reads) = fake(PlatformPrefs::default());
        let (on_change, notifications) = mpsc::channel();
        let interval = Duration::from_millis(15);
        let timeout = Duration::from_secs(5);
        watch.start_worker(interval, move || on_change.send(()).unwrap());
        watch.update_window(active());
        // Read at startup, on first focus and twice on the worker with no change.
        for _ in 0..4 {
            reads.recv_timeout(timeout).unwrap();
        }
        assert_eq!(notifications.try_recv(), Err(mpsc::TryRecvError::Empty));
        *prefs.lock().unwrap() = PlatformPrefs {
            transparency: Some(false),
            ..PlatformPrefs::default()
        };
        notifications.recv_timeout(timeout).unwrap();
        assert_eq!(watch.current().transparency, Some(false));
        watch.update_window(WindowState {
            focused: true,
            visible: false,
        });
        for () in reads.try_iter() {}
        *prefs.lock().unwrap() = PlatformPrefs::default();
        assert_eq!(
            reads.recv_timeout(3 * interval),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        watch.update_window(active());
        assert_eq!(watch.current(), PlatformPrefs::default());
        drop(watch);
        assert_eq!(
            notifications.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        );
    }

    #[test]
    fn unknown_platform_reports_nothing() {
        assert_eq!(UnknownPlatform.read(), PlatformPrefs::default());
    }
}
