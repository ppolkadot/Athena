//! Operating-system preferences that change how decorative effects behave (contract §14.2):
//! client-area animation, high contrast and transparency. Each value is `None` when unknown, and
//! unknown always resolves to the accessible choice.

#[cfg(windows)]
pub mod windows_prefs;

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
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
pub trait PlatformPreferences: Send + Sync {
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
///
/// The provider is never called while the state lock is held, so the UI thread's `current` and
/// `update_window` do not wait on Win32 or registry queries.
pub struct PlatformWatch {
    shared: Arc<SharedWatch>,
    worker: Option<JoinHandle<()>>,
}

struct SharedWatch {
    provider: Box<dyn PlatformPreferences>,
    state: Mutex<WatchState>,
    activity: Condvar,
}

struct WatchState {
    current: PlatformPrefs,
    active: bool,
    stopped: bool,
    /// Focus or resume returned: the worker reads once without waiting for the next poll.
    needs_read: bool,
}

impl SharedWatch {
    fn lock(&self) -> MutexGuard<'_, WatchState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Read the provider without the lock, then store the sample unless the watcher stopped.
    /// Returns whether the stored sample changed.
    fn read_and_store(&self) -> bool {
        let next = self.provider.read();
        let mut state = self.lock();
        if state.stopped {
            return false;
        }
        let changed = next != state.current;
        state.current = next;
        changed
    }

    fn watch(&self, interval: Duration, on_change: impl Fn()) {
        loop {
            let state = self.lock();
            let mut state = self
                .activity
                .wait_while(state, |s| !s.active && !s.stopped)
                .unwrap_or_else(PoisonError::into_inner);
            let mut timed_out = false;
            if !state.needs_read && !state.stopped {
                let (waited, elapsed) = self
                    .activity
                    .wait_timeout_while(state, interval, |s| {
                        s.active && !s.stopped && !s.needs_read
                    })
                    .unwrap_or_else(PoisonError::into_inner);
                state = waited;
                timed_out = elapsed.timed_out();
            }
            if state.stopped {
                break;
            }
            let due = state.needs_read || (timed_out && state.active);
            state.needs_read = false;
            // The next frame also locks the sample. Release it before querying the system and
            // before asking egui to wake.
            drop(state);
            if due && self.read_and_store() {
                on_change();
            }
        }
    }
}

impl PlatformWatch {
    #[must_use]
    pub fn new(provider: Box<dyn PlatformPreferences>) -> Self {
        let current = provider.read();
        Self {
            shared: Arc::new(SharedWatch {
                provider,
                state: Mutex::new(WatchState {
                    current,
                    active: false,
                    stopped: false,
                    needs_read: false,
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
        self.shared.lock().current
    }

    /// Apply a window-state change. Focus regain or resume asks the worker for an immediate read;
    /// without a worker the read happens here, outside the lock. The returned sample is the one
    /// stored so far, and the worker's wake callback reports a newer one.
    #[allow(clippy::must_use_candidate)] // called for its effect; the sample is a convenience
    pub fn update_window(&self, window: WindowState) -> PlatformPrefs {
        let active = window.focused && window.visible;
        let mut regained = false;
        let current = {
            let mut state = self.shared.lock();
            if state.active != active {
                state.active = active;
                if active && self.worker.is_some() {
                    state.needs_read = true;
                }
                regained = active && self.worker.is_none();
                self.shared.activity.notify_one();
            }
            state.current
        };
        if regained {
            self.shared.read_and_store();
            return self.current();
        }
        current
    }

    /// Stop and join the worker. Later reads are ignored; calling it again does nothing.
    pub fn stop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.shared.lock().stopped = true;
            self.shared.activity.notify_one();
            let _ = worker.join();
        }
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

impl Drop for PlatformWatch {
    fn drop(&mut self) {
        self.stop();
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
        // Focus regain is read by the worker, which reports the change that happened while hidden.
        watch.update_window(active());
        notifications.recv_timeout(timeout).unwrap();
        assert_eq!(watch.current(), PlatformPrefs::default());
        drop(watch);
        assert_eq!(
            notifications.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        );
    }

    /// A provider whose reads block until the test releases them.
    struct Gated {
        prefs: Arc<Mutex<PlatformPrefs>>,
        entered: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
        gate: Arc<std::sync::atomic::AtomicBool>,
    }

    impl PlatformPreferences for Gated {
        fn read(&self) -> PlatformPrefs {
            let prefs = *self.prefs.lock().unwrap();
            if self.gate.load(std::sync::atomic::Ordering::SeqCst) {
                self.entered.lock().unwrap().send(()).unwrap();
                let _ = self
                    .release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5));
            }
            prefs
        }
    }

    #[allow(clippy::type_complexity)]
    fn gated(
        initial: PlatformPrefs,
    ) -> (
        PlatformWatch,
        Arc<Mutex<PlatformPrefs>>,
        Arc<std::sync::atomic::AtomicBool>,
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
    ) {
        let prefs = Arc::new(Mutex::new(initial));
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (entered, entered_rx) = mpsc::channel();
        let (release_tx, release) = mpsc::channel();
        let watch = PlatformWatch::new(Box::new(Gated {
            prefs: Arc::clone(&prefs),
            entered: Mutex::new(entered),
            release: Mutex::new(release),
            gate: Arc::clone(&gate),
        }));
        (watch, prefs, gate, entered_rx, release_tx)
    }

    #[test]
    fn a_slow_provider_read_never_blocks_the_ui_thread() {
        let (mut watch, prefs, gate, entered, release) = gated(PlatformPrefs::default());
        let (on_change, notifications) = mpsc::channel();
        watch.start_worker(Duration::from_mins(1), move || {
            let _ = on_change.send(());
        });
        gate.store(true, std::sync::atomic::Ordering::SeqCst);
        *prefs.lock().unwrap() = PlatformPrefs {
            animations: Some(false),
            ..PlatformPrefs::default()
        };
        // Focus regain asks the worker; it blocks inside the provider.
        watch.update_window(active());
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        // While that read is in flight the UI thread still locks the sample promptly.
        let watch = Arc::new(watch);
        let (done, finished) = mpsc::channel();
        let ui = {
            let watch = Arc::clone(&watch);
            std::thread::spawn(move || {
                let before = watch.current();
                watch.update_window(WindowState {
                    focused: true,
                    visible: true,
                });
                done.send(before).unwrap();
            })
        };
        let before = finished
            .recv_timeout(Duration::from_secs(2))
            .expect("current() and update_window() must not wait on the provider");
        assert_eq!(before.animations, None);
        release.send(()).unwrap();
        notifications.recv_timeout(Duration::from_secs(5)).unwrap();
        ui.join().unwrap();
        assert_eq!(watch.current().animations, Some(false));
    }

    #[test]
    fn a_read_finishing_after_stop_is_discarded() {
        let (mut watch, prefs, gate, entered, release) = gated(PlatformPrefs::default());
        let (on_change, notifications) = mpsc::channel();
        watch.start_worker(Duration::from_mins(1), move || {
            let _ = on_change.send(());
        });
        gate.store(true, std::sync::atomic::Ordering::SeqCst);
        *prefs.lock().unwrap() = PlatformPrefs {
            transparency: Some(false),
            ..PlatformPrefs::default()
        };
        watch.update_window(active());
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let shared = Arc::clone(&watch.shared);
        let stopper = std::thread::spawn(move || {
            watch.stop();
            watch
        });
        // `stop` sets the flag before it joins, so wait for the flag, then let the read finish.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !shared.lock().stopped {
            assert!(std::time::Instant::now() < deadline, "stop never signalled");
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        let watch = stopper.join().unwrap();
        assert_eq!(watch.current(), PlatformPrefs::default());
        assert_eq!(
            notifications.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        );
        // A second stop and the drop are harmless.
        drop(watch);
    }

    #[test]
    fn focus_regain_is_read_by_the_worker_and_reported() {
        let (mut watch, prefs, reads) = fake(PlatformPrefs::default());
        reads.try_recv().unwrap();
        let (on_change, notifications) = mpsc::channel();
        watch.start_worker(Duration::from_mins(1), move || {
            let _ = on_change.send(());
        });
        *prefs.lock().unwrap() = PlatformPrefs {
            animations: Some(false),
            ..PlatformPrefs::default()
        };
        watch.update_window(active());
        notifications.recv_timeout(Duration::from_secs(5)).unwrap();
        reads.try_recv().unwrap();
        assert_eq!(watch.current().animations, Some(false));
    }

    #[test]
    fn unknown_platform_reports_nothing() {
        assert_eq!(UnknownPlatform.read(), PlatformPrefs::default());
    }
}
