//! Managed wallpaper media (contract §14.2): bounded decoding, the retained-memory budget, the
//! single decode worker and the app-owned copies under `root/appearance/`.

pub mod assets;
pub mod decode;
#[cfg(test)]
pub(crate) mod test_gif;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use crate::domain::appearance::{AssetId, Quality};
use assets::MediaStore;

/// A source file may be at most this large; reads are bounded by it.
pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
/// Canvas width and height ceiling in pixels.
pub const MAX_AXIS: u32 = 2048;
/// Frames in one animation.
pub const MAX_FRAMES: usize = 300;
/// Retained RGBA across the active wallpaper and the preview.
pub const RETAINED_BUDGET_BYTES: usize = 128 * 1024 * 1024;

/// Why media could not be used. Carries no paths, so it is safe to log and to localize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    #[error("not a supported PNG, JPEG or GIF image")]
    Unsupported,
    #[error("the image data is damaged or incomplete")]
    Corrupt,
    #[error("the file is larger than 16 MiB")]
    SourceTooLarge,
    #[error("the image is wider or taller than 2048 pixels")]
    DimensionsTooLarge,
    #[error("the animation has more than 300 frames")]
    TooManyFrames,
    #[error("the image needs more memory than wallpaper may use")]
    OverBudget,
    #[error("the request was superseded")]
    Cancelled,
    #[error("the file could not be read or written")]
    Io,
    #[error("the file is missing")]
    Missing,
    #[error("the wallpaper folder is full")]
    QuotaExceeded,
    #[error("the wallpaper folder or file is not a plain app-owned file")]
    Unsafe,
}

impl MediaError {
    /// Whether the failure is the user's file (as opposed to the app's own storage).
    #[must_use]
    pub fn is_input_problem(self) -> bool {
        matches!(
            self,
            Self::Unsupported
                | Self::Corrupt
                | Self::SourceTooLarge
                | Self::DimensionsTooLarge
                | Self::TooManyFrames
                | Self::OverBudget
        )
    }
}

/// Retained decoded bytes charged against one shared cap (active plus preview).
#[derive(Debug)]
pub struct MediaBudget {
    cap: usize,
    used: AtomicUsize,
}

impl MediaBudget {
    #[must_use]
    pub fn new(cap: usize) -> Arc<Self> {
        Arc::new(Self {
            cap,
            used: AtomicUsize::new(0),
        })
    }

    /// The production 128 MiB cap.
    #[must_use]
    pub fn standard() -> Arc<Self> {
        Self::new(RETAINED_BUDGET_BYTES)
    }

    #[must_use]
    pub fn used(&self) -> usize {
        self.used.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// A lease that charges and releases against this budget.
    #[must_use]
    pub fn lease(self: &Arc<Self>) -> BudgetLease {
        BudgetLease {
            budget: Arc::clone(self),
            charged: 0,
        }
    }

    fn try_charge(&self, bytes: usize) -> Result<(), MediaError> {
        let mut current = self.used.load(Ordering::SeqCst);
        loop {
            let next = current
                .checked_add(bytes)
                .filter(|n| *n <= self.cap)
                .ok_or(MediaError::OverBudget)?;
            match self
                .used
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Ok(()),
                Err(seen) => current = seen,
            }
        }
    }
}

/// Bytes charged to a [`MediaBudget`]; released when dropped.
#[derive(Debug)]
pub struct BudgetLease {
    budget: Arc<MediaBudget>,
    charged: usize,
}

impl BudgetLease {
    /// Charge `bytes` before the memory is allocated.
    ///
    /// # Errors
    /// [`MediaError::OverBudget`] when the shared cap would be exceeded or the sum overflows.
    pub fn charge(&mut self, bytes: usize) -> Result<(), MediaError> {
        self.budget.try_charge(bytes)?;
        self.charged += bytes;
        Ok(())
    }

    #[must_use]
    pub fn charged(&self) -> usize {
        self.charged
    }
}

impl Drop for BudgetLease {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.charged, Ordering::SeqCst);
    }
}

/// How many times an animation plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    /// This many complete play-throughs (absent loop metadata is one).
    Plays(u32),
    Infinite,
}

/// One composited frame at the prepared size.
#[derive(Debug, Clone)]
pub struct MediaFrame {
    /// `width * height * 4` straight-alpha RGBA bytes.
    pub rgba: Arc<[u8]>,
    /// Normalized presentation time in milliseconds (at least 20 for animations).
    pub delay_ms: u32,
}

/// Decoded, composited and downscaled media. Dropping it releases its memory charge.
#[derive(Debug)]
pub struct DecodedMedia {
    pub width: u32,
    pub height: u32,
    pub frames: Vec<MediaFrame>,
    pub repeat: Repeat,
    /// A hash of the source bytes: the same source and tier produce the same identity.
    pub identity: u64,
    lease: BudgetLease,
}

impl DecodedMedia {
    pub(crate) fn new(
        width: u32,
        height: u32,
        frames: Vec<MediaFrame>,
        repeat: Repeat,
        identity: u64,
        lease: BudgetLease,
    ) -> Self {
        Self {
            width,
            height,
            frames,
            repeat,
            identity,
            lease,
        }
    }

    #[must_use]
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// Bytes retained for the frames.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.lease.charged()
    }
}

/// Which consumer asked for a decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Active,
    Preview,
}

/// Where a decode gets its bytes.
#[derive(Debug, Clone)]
pub enum Source {
    /// Bytes already in memory (a freshly picked file, or tests).
    Bytes(Arc<Vec<u8>>),
    /// A managed copy, read on the worker so the UI thread never waits on the disk.
    Managed(MediaStore, AssetId),
}

/// One decode. `latest` is the worker's current generation; a request whose `generation` no longer
/// matches is stale and aborts between frames.
#[derive(Debug, Clone)]
pub struct DecodeRequest {
    pub generation: u64,
    pub slot: Slot,
    pub source: Source,
    /// The largest display extent in physical pixels, when known.
    pub display: Option<(u32, u32)>,
    pub quality: Quality,
    pub budget: Arc<MediaBudget>,
    latest: Arc<AtomicU64>,
}

impl DecodeRequest {
    /// A standalone request that is never superseded (tests and synchronous callers).
    #[must_use]
    pub fn standalone(
        bytes: Arc<Vec<u8>>,
        display: Option<(u32, u32)>,
        quality: Quality,
        budget: Arc<MediaBudget>,
    ) -> Self {
        Self {
            generation: 1,
            slot: Slot::Active,
            source: Source::Bytes(bytes),
            display,
            quality,
            budget,
            latest: Arc::new(AtomicU64::new(1)),
        }
    }

    #[must_use]
    pub fn is_current(&self) -> bool {
        self.latest.load(Ordering::SeqCst) == self.generation
    }
}

#[derive(Debug)]
pub enum DecodeOutcome {
    Done(Arc<DecodedMedia>),
    Failed(MediaError),
    /// A newer request replaced this one before it started.
    Superseded,
}

#[derive(Debug)]
pub struct DecodeResult {
    pub generation: u64,
    pub slot: Slot,
    pub outcome: DecodeOutcome,
}

type Runner = dyn Fn(&DecodeRequest) -> Result<DecodedMedia, MediaError> + Send + Sync;
type Wake = dyn Fn() + Send + Sync;

struct State {
    pending: Option<DecodeRequest>,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    signal: Condvar,
    latest: Arc<AtomicU64>,
}

/// One worker thread with one running and one replaceable pending request. Results arrive on a
/// channel; the UI is woken once per result.
pub struct MediaWorker {
    shared: Arc<Shared>,
    results: Receiver<DecodeResult>,
    sender: Sender<DecodeResult>,
    wake: Arc<Wake>,
    next_generation: u64,
    handle: Option<JoinHandle<()>>,
}

impl MediaWorker {
    /// The production worker: [`decode::decode`] with `wake` called after every result.
    #[must_use]
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_runner(wake, decode::decode)
    }

    /// A worker with an injected decoder, so tests can control timing.
    pub fn with_runner(
        wake: impl Fn() + Send + Sync + 'static,
        runner: impl Fn(&DecodeRequest) -> Result<DecodedMedia, MediaError> + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                pending: None,
                shutdown: false,
            }),
            signal: Condvar::new(),
            latest: Arc::new(AtomicU64::new(0)),
        });
        let (sender, results) = mpsc::channel();
        let wake: Arc<Wake> = Arc::new(wake);
        let runner: Arc<Runner> = Arc::new(runner);
        let handle = {
            let (shared, sender, wake) = (Arc::clone(&shared), sender.clone(), Arc::clone(&wake));
            std::thread::Builder::new()
                .name("athena-media".into())
                .spawn(move || worker_loop(&shared, &sender, &*wake, &*runner))
                .ok()
        };
        Self {
            shared,
            results,
            sender,
            wake,
            next_generation: 0,
            handle,
        }
    }

    /// Queue a decode, replacing any request that has not started. Returns its generation.
    pub fn submit(
        &mut self,
        slot: Slot,
        source: Source,
        display: Option<(u32, u32)>,
        quality: Quality,
        budget: &Arc<MediaBudget>,
    ) -> u64 {
        self.next_generation += 1;
        let generation = self.next_generation;
        self.shared.latest.store(generation, Ordering::SeqCst);
        let request = DecodeRequest {
            generation,
            slot,
            source,
            display,
            quality,
            budget: Arc::clone(budget),
            latest: Arc::clone(&self.shared.latest),
        };
        let replaced = {
            let mut state = lock(&self.shared.state);
            state.pending.replace(request)
        };
        self.notify_superseded(replaced);
        self.shared.signal.notify_one();
        generation
    }

    /// Invalidate everything queued or running (closing a preview, Reset, import, restore). Late
    /// results then carry a stale generation.
    pub fn cancel_all(&mut self) {
        self.next_generation += 1;
        self.shared
            .latest
            .store(self.next_generation, Ordering::SeqCst);
        let replaced = lock(&self.shared.state).pending.take();
        self.notify_superseded(replaced);
    }

    /// The generation a result must carry to still be wanted.
    #[must_use]
    pub fn current_generation(&self) -> u64 {
        self.shared.latest.load(Ordering::SeqCst)
    }

    /// The next finished result, if any.
    #[must_use]
    pub fn try_recv(&self) -> Option<DecodeResult> {
        self.results.try_recv().ok()
    }

    fn notify_superseded(&self, replaced: Option<DecodeRequest>) {
        if let Some(old) = replaced {
            let _ = self.sender.send(DecodeResult {
                generation: old.generation,
                slot: old.slot,
                outcome: DecodeOutcome::Superseded,
            });
            (self.wake)();
        }
    }
}

impl Drop for MediaWorker {
    fn drop(&mut self) {
        lock(&self.shared.state).shutdown = true;
        self.shared.latest.fetch_add(1, Ordering::SeqCst);
        self.shared.signal.notify_one();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn worker_loop(shared: &Shared, sender: &Sender<DecodeResult>, wake: &Wake, runner: &Runner) {
    loop {
        let request = {
            let mut state = lock(&shared.state);
            loop {
                if state.shutdown {
                    return;
                }
                if let Some(request) = state.pending.take() {
                    break request;
                }
                state = shared
                    .signal
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let outcome = if request.is_current() {
            match runner(&request) {
                Ok(media) => DecodeOutcome::Done(Arc::new(media)),
                Err(err) => DecodeOutcome::Failed(err),
            }
        } else {
            DecodeOutcome::Failed(MediaError::Cancelled)
        };
        let _ = sender.send(DecodeResult {
            generation: request.generation,
            slot: request.slot,
            outcome,
        });
        wake();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    use super::*;

    fn media(budget: &Arc<MediaBudget>, bytes: usize) -> Result<DecodedMedia, MediaError> {
        let mut lease = budget.lease();
        lease.charge(bytes)?;
        Ok(DecodedMedia::new(
            1,
            1,
            Vec::new(),
            Repeat::Plays(1),
            0,
            lease,
        ))
    }

    #[test]
    fn budget_charges_checked_and_releases_on_drop() {
        let budget = MediaBudget::new(100);
        let mut a = budget.lease();
        a.charge(60).unwrap();
        let mut b = budget.lease();
        assert_eq!(b.charge(41), Err(MediaError::OverBudget));
        assert_eq!(b.charge(usize::MAX), Err(MediaError::OverBudget));
        b.charge(40).unwrap();
        assert_eq!(budget.used(), 100);
        drop(a);
        assert_eq!(budget.used(), 40);
        drop(b);
        assert_eq!(budget.used(), 0);
        assert_eq!(MediaBudget::standard().cap(), 128 * 1024 * 1024);
    }

    #[test]
    fn a_queued_request_is_replaced_and_a_running_one_is_cancelled() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = sync_channel::<u64>(4);
        let (release_tx, release_rx) = sync_channel::<()>(4);
        let release_rx = Mutex::new(release_rx);
        let budget = MediaBudget::new(1_000);
        let runner_budget = Arc::clone(&budget);
        let mut worker = {
            let wakes = Arc::clone(&wakes);
            MediaWorker::with_runner(
                move || {
                    wakes.fetch_add(1, Ordering::SeqCst);
                },
                move |request| {
                    started_tx.send(request.generation).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                    if !request.is_current() {
                        return Err(MediaError::Cancelled);
                    }
                    media(&runner_budget, 10)
                },
            )
        };
        let bytes = Source::Bytes(Arc::new(vec![0_u8]));
        let one = worker.submit(Slot::Active, bytes.clone(), None, Quality::Auto, &budget);
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            one
        );
        // While request one runs, two queues and three replaces it.
        let two = worker.submit(Slot::Preview, bytes.clone(), None, Quality::Auto, &budget);
        let three = worker.submit(Slot::Preview, bytes, None, Quality::Low, &budget);
        assert!(two < three);
        release_tx.send(()).unwrap();
        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            three,
            "the replaced request never starts"
        );
        release_tx.send(()).unwrap();

        let mut seen = Vec::new();
        while seen.len() < 3 {
            if let Some(result) = worker.try_recv() {
                seen.push(result);
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        let kind = |generation: u64| {
            let r = seen.iter().find(|r| r.generation == generation).unwrap();
            match &r.outcome {
                DecodeOutcome::Done(_) => "done",
                DecodeOutcome::Failed(MediaError::Cancelled) => "cancelled",
                DecodeOutcome::Failed(_) => "failed",
                DecodeOutcome::Superseded => "superseded",
            }
        };
        assert_eq!(
            kind(one),
            "cancelled",
            "the running request saw a newer generation"
        );
        assert_eq!(kind(two), "superseded");
        assert_eq!(kind(three), "done");
        // The UI is woken once per delivered result.
        assert_eq!(wakes.load(Ordering::SeqCst), 3);
        assert_eq!(worker.current_generation(), three);
    }

    #[test]
    fn cancel_all_invalidates_a_late_result() {
        let (started_tx, started_rx) = sync_channel::<u64>(1);
        let (release_tx, release_rx) = sync_channel::<()>(1);
        let release_rx = Mutex::new(release_rx);
        let budget = MediaBudget::new(1_000);
        let runner_budget = Arc::clone(&budget);
        let mut worker = MediaWorker::with_runner(
            || {},
            move |request| {
                started_tx.send(request.generation).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
                media(&runner_budget, 10)
            },
        );
        let generation = worker.submit(
            Slot::Preview,
            Source::Bytes(Arc::new(vec![0])),
            None,
            Quality::Auto,
            &budget,
        );
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // The preview is closed (or Reset/import/restore happens) while the decode is running.
        worker.cancel_all();
        release_tx.send(()).unwrap();
        let result = loop {
            if let Some(r) = worker.try_recv() {
                break r;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(result.generation, generation);
        assert_ne!(
            result.generation,
            worker.current_generation(),
            "the consumer drops it as stale"
        );
        // Dropping the late result releases its retained bytes.
        drop(result);
        assert_eq!(budget.used(), 0);
    }
}
