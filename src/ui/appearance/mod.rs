//! Wallpaper runtime (contract §14.2): the playback clock, the managed media lifecycle and the (at
//! most two) GPU textures. `AthenaApp` owns one [`WallpaperRuntime`], so every main screen shares
//! the same live frame and navigation never restarts playback or re-decodes.

pub mod background;

use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions, Vec2};

use self::background::Backdrop;
use crate::domain::appearance::{AssetId, Quality, Wallpaper};
use crate::infrastructure::media::assets::{MediaStore, StagedMedia};
use crate::infrastructure::media::{
    BudgetLease, DecodeOutcome, DecodedMedia, MediaBudget, MediaError, MediaFrame, MediaWorker,
    Repeat, Slot, Source,
};
use crate::ui::theme::effective::{EffectiveAppearance, Material};

/// Uploads are at least this far apart: a 30 Hz ceiling on wallpaper texture writes.
pub const UPLOAD_FLOOR: Duration = Duration::from_micros(33_340);

/// Cumulative frame times for one animation, in microseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeline {
    ends_us: Vec<u64>,
    repeat: Repeat,
}

impl Timeline {
    #[must_use]
    pub fn new(delays_ms: &[u32], repeat: Repeat) -> Self {
        let mut total = 0_u64;
        let ends_us = delays_ms
            .iter()
            .map(|d| {
                total += u64::from((*d).max(1)) * 1000;
                total
            })
            .collect();
        Self { ends_us, repeat }
    }

    #[must_use]
    pub fn from_media(media: &DecodedMedia) -> Self {
        let delays: Vec<u32> = media.frames.iter().map(|f| f.delay_ms).collect();
        Self::new(&delays, media.repeat)
    }

    fn total_us(&self) -> u64 {
        self.ends_us.last().copied().unwrap_or(1).max(1)
    }

    /// The frame shown `elapsed_us` into playback, whether playback has finished (the last frame
    /// then holds), and how long until the frame changes.
    #[must_use]
    pub fn frame_at(&self, elapsed_us: u64) -> (usize, bool, Option<u64>) {
        let last = self.ends_us.len().saturating_sub(1);
        if self.ends_us.len() <= 1 {
            return (0, true, None);
        }
        let total = self.total_us();
        if let Repeat::Plays(n) = self.repeat
            && elapsed_us >= total.saturating_mul(u64::from(n.max(1)))
        {
            return (last, true, None);
        }
        let position = elapsed_us % total;
        let index = self
            .ends_us
            .partition_point(|end| *end <= position)
            .min(last);
        (index, false, Some(self.ends_us[index] - position))
    }
}

/// How the wallpaper clock behaves this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Elapsed time advances.
    Run,
    /// User pause, blur, hidden or minimized: keep the frame, never catch up.
    Frozen,
    /// Reduced motion or opaque chrome: show frame zero and restart from zero later.
    Poster,
}

impl Mode {
    /// Reduced motion and opaque chrome show a poster; a user pause or an inactive window freezes;
    /// everything else runs.
    #[must_use]
    pub fn for_look(look: &EffectiveAppearance, focused: bool, visible: bool) -> Self {
        if look.reduced_motion || look.material == Material::Solid {
            Self::Poster
        } else if look.paused || !focused || !visible {
            Self::Frozen
        } else {
            Self::Run
        }
    }
}

/// One frame's decision: which frame to show, whether to upload it, and when to wake next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub frame: usize,
    pub upload: bool,
    /// The next decorative deadline; `None` means no repaint is needed for this wallpaper.
    pub wake: Option<Duration>,
}

/// The playback clock for one decoded animation. It owns elapsed time and the upload floor and
/// reads time only from the `now` it is given.
#[derive(Debug, Clone)]
pub struct Playback {
    timeline: Timeline,
    elapsed_us: u64,
    anchor: Option<Instant>,
    shown: Option<usize>,
    last_upload: Option<Instant>,
}

impl Playback {
    #[must_use]
    pub fn new(timeline: Timeline) -> Self {
        Self {
            timeline,
            elapsed_us: 0,
            anchor: None,
            shown: None,
            last_upload: None,
        }
    }

    #[must_use]
    pub fn elapsed(&self) -> Duration {
        Duration::from_micros(self.elapsed_us)
    }

    /// Forget what is on the GPU (after a texture replacement) so the current frame uploads again.
    pub fn invalidate_texture(&mut self) {
        self.shown = None;
    }

    pub fn step(&mut self, now: Instant, mode: Mode) -> Step {
        match mode {
            Mode::Poster => {
                self.elapsed_us = 0;
                self.anchor = None;
            }
            Mode::Frozen => self.anchor = None,
            Mode::Run => {
                if let Some(anchor) = self.anchor {
                    let advance = now.saturating_duration_since(anchor).as_micros();
                    self.elapsed_us = self
                        .elapsed_us
                        .saturating_add(u64::try_from(advance).unwrap_or(u64::MAX));
                }
                self.anchor = Some(now);
            }
        }
        let (frame, finished, until_next) = self.timeline.frame_at(self.elapsed_us);
        let floor_left = self
            .last_upload
            .map(|t| UPLOAD_FLOOR.saturating_sub(now.saturating_duration_since(t)))
            .filter(|left| !left.is_zero());
        let changed = self.shown != Some(frame);
        let upload = changed && floor_left.is_none();
        if upload {
            self.shown = Some(frame);
            self.last_upload = Some(now);
        }
        let running = mode == Mode::Run && !finished;
        let wake = if changed && !upload {
            // The frame on screen is stale (for example the poster): upload as soon as the floor
            // allows, never sooner.
            floor_left
        } else if running {
            let next = Duration::from_micros(until_next.unwrap_or(0));
            let gap = if upload {
                UPLOAD_FLOOR
            } else {
                floor_left.unwrap_or(Duration::ZERO)
            };
            Some(next.max(gap))
        } else {
            None
        };
        Step {
            frame,
            upload,
            wake,
        }
    }
}

/// Counters for measuring wallpaper cost (smoke records and tests).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub texture_allocations: usize,
    pub texture_writes: usize,
    pub decodes_submitted: usize,
}

struct Texture {
    handle: TextureHandle,
    size: [usize; 2],
}

/// What a slot wants decoded. Equal wants share one decode.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Want {
    source: WantSource,
    quality: Quality,
    display: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WantSource {
    Builtin,
    Managed(AssetId),
    /// Bytes of a file picked in this session and not yet applied.
    Staged(AssetId),
}

enum Load {
    Idle,
    Loading,
    Ready(Arc<DecodedMedia>),
    Failed(MediaError),
}

struct SlotState {
    want: Option<Want>,
    load: Load,
    playback: Option<Playback>,
    texture: Option<Texture>,
}

impl SlotState {
    fn new() -> Self {
        Self {
            want: None,
            load: Load::Idle,
            playback: None,
            texture: None,
        }
    }

    fn reset(&mut self) {
        self.want = None;
        self.load = Load::Idle;
        self.playback = None;
        // The texture is dropped with the handle, freeing its GPU memory.
        self.texture = None;
    }
}

/// A wallpaper the shell may show: what to decode and where to find the bytes.
#[derive(Debug, Clone, Copy)]
pub struct Wanted<'a> {
    pub wallpaper: &'a Wallpaper,
    /// The bytes of a freshly picked file for `wallpaper`, when it is not managed yet.
    pub staged: Option<&'a StagedMedia>,
}

/// The built-in Aurora wallpaper: a subtle static slate-to-teal gradient generated in code, so it
/// needs no asset storage.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn builtin_aurora(budget: &Arc<MediaBudget>) -> Option<DecodedMedia> {
    const W: usize = 256;
    const H: usize = 144;
    let mut lease: BudgetLease = budget.lease();
    lease.charge(W * H * 4).ok()?;
    let mut rgba = Vec::with_capacity(W * H * 4);
    for y in 0..H {
        for x in 0..W {
            let (fx, fy) = (x as f32 / W as f32, y as f32 / H as f32);
            let t = (fx * 0.55 + fy * 0.45).clamp(0.0, 1.0);
            let glow =
                (1.0 - ((fx - 0.78).powi(2) + (fy - 0.25).powi(2)).sqrt() * 2.2).clamp(0.0, 1.0);
            let mix = |a: f32, b: f32, g: f32| (a + (b - a) * t + g * glow).clamp(0.0, 255.0);
            rgba.extend_from_slice(&[
                mix(24.0, 10.0, 6.0) as u8,
                mix(36.0, 92.0, 52.0) as u8,
                mix(52.0, 96.0, 46.0) as u8,
                255,
            ]);
        }
    }
    Some(DecodedMedia::new(
        W as u32,
        H as u32,
        vec![MediaFrame {
            rgba: Arc::from(rgba),
            delay_ms: 100,
        }],
        Repeat::Plays(1),
        0xA0_70_4A,
        lease,
    ))
}

/// Owns the decode worker, both wallpaper slots and their textures.
pub struct WallpaperRuntime {
    worker: MediaWorker,
    budget: Arc<MediaBudget>,
    active: SlotState,
    preview: SlotState,
    /// The slot with a request in flight and its generation. One request runs at a time.
    in_flight: Option<(Slot, u64)>,
    stats: Stats,
}

impl WallpaperRuntime {
    /// A runtime whose decode completions wake the UI.
    #[must_use]
    pub fn new(ctx: &egui::Context) -> Self {
        let ctx = ctx.clone();
        Self::with_worker(MediaWorker::new(move || ctx.request_repaint()))
    }

    #[must_use]
    pub fn with_worker(worker: MediaWorker) -> Self {
        Self {
            worker,
            budget: MediaBudget::standard(),
            active: SlotState::new(),
            preview: SlotState::new(),
            in_flight: None,
            stats: Stats::default(),
        }
    }

    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    #[must_use]
    pub fn budget(&self) -> &Arc<MediaBudget> {
        &self.budget
    }

    /// Live GPU textures (at most two).
    #[must_use]
    pub fn live_textures(&self) -> usize {
        usize::from(self.active.texture.is_some()) + usize::from(self.preview.texture.is_some())
    }

    fn slot_mut(&mut self, slot: Slot) -> &mut SlotState {
        match slot {
            Slot::Active => &mut self.active,
            Slot::Preview => &mut self.preview,
        }
    }

    /// Drop everything bound to a wallpaper and invalidate in-flight results: import, restore and
    /// Reset rebind even when the new preferences compare equal to the old.
    pub fn invalidate_all(&mut self) {
        self.worker.cancel_all();
        self.in_flight = None;
        self.active.reset();
        self.preview.reset();
    }

    /// Close the preview: a late result is ignored and its memory is released.
    pub fn clear_preview(&mut self) {
        if matches!(self.in_flight, Some((Slot::Preview, _))) {
            self.worker.cancel_all();
            self.in_flight = None;
        }
        self.preview.reset();
    }

    /// Why the wallpaper currently shown cannot be used, if it cannot.
    #[must_use]
    pub fn failure(&self, preview: bool) -> Option<MediaError> {
        let slot = if preview { &self.preview } else { &self.active };
        match slot.load {
            Load::Failed(err) => Some(err),
            _ => None,
        }
    }

    /// Whether the slot's wallpaper is still decoding.
    #[must_use]
    pub fn is_loading(&self, preview: bool) -> bool {
        let slot = if preview { &self.preview } else { &self.active };
        matches!(slot.load, Load::Loading)
    }

    /// Whether the slot has a usable decoded wallpaper.
    #[must_use]
    pub fn is_ready(&self, preview: bool) -> bool {
        let slot = if preview { &self.preview } else { &self.active };
        matches!(slot.load, Load::Ready(_))
    }

    fn want_for(wanted: Wanted<'_>, quality: Quality, display: Option<(u32, u32)>) -> Option<Want> {
        let source = match wanted.wallpaper {
            Wallpaper::None => return None,
            Wallpaper::Builtin => WantSource::Builtin,
            Wallpaper::Local(id) => match wanted.staged {
                Some(staged) if staged.id == *id => WantSource::Staged(id.clone()),
                _ => WantSource::Managed(id.clone()),
            },
        };
        Some(Want {
            source,
            quality,
            display,
        })
    }

    fn receive(&mut self) {
        while let Some(result) = self.worker.try_recv() {
            if self.in_flight != Some((result.slot, result.generation)) {
                // Stale: a closed preview, Reset, import or restore invalidated it. Dropping the
                // outcome releases its retained bytes.
                continue;
            }
            self.in_flight = None;
            let slot = self.slot_mut(result.slot);
            match result.outcome {
                DecodeOutcome::Done(media) => {
                    slot.playback = Some(Playback::new(Timeline::from_media(&media)));
                    slot.texture = None;
                    slot.load = Load::Ready(media);
                }
                DecodeOutcome::Failed(MediaError::Cancelled) | DecodeOutcome::Superseded => {
                    slot.load = Load::Idle;
                }
                DecodeOutcome::Failed(err) => slot.load = Load::Failed(err),
            }
        }
    }

    fn request(
        &mut self,
        slot_id: Slot,
        want: &Want,
        store: &MediaStore,
        staged: Option<&StagedMedia>,
    ) {
        let source = match &want.source {
            WantSource::Builtin => None,
            WantSource::Managed(id) => Some(Source::Managed(store.clone(), id.clone())),
            WantSource::Staged(_) => staged.map(|s| Source::Bytes(Arc::clone(&s.bytes))),
        };
        if let Some(source) = source {
            let generation =
                self.worker
                    .submit(slot_id, source, want.display, want.quality, &self.budget);
            self.in_flight = Some((slot_id, generation));
            self.stats.decodes_submitted += 1;
            self.slot_mut(slot_id).load = Load::Loading;
        } else {
            let media = builtin_aurora(&self.budget);
            let slot = self.slot_mut(slot_id);
            match media {
                Some(media) => {
                    let media = Arc::new(media);
                    slot.playback = Some(Playback::new(Timeline::from_media(&media)));
                    slot.texture = None;
                    slot.load = Load::Ready(media);
                }
                None => slot.load = Load::Failed(MediaError::OverBudget),
            }
        }
    }

    /// Follow the wanted wallpapers, start decodes, apply results and advance the displayed slot's
    /// clock. Returns the backdrop to paint and the next decorative wake-up, if any.
    #[allow(clippy::too_many_arguments)]
    pub fn sync(
        &mut self,
        ctx: &egui::Context,
        store: &MediaStore,
        active: Wanted<'_>,
        preview: Option<Wanted<'_>>,
        look: &EffectiveAppearance,
        window: (bool, bool),
        now: Instant,
    ) -> (Backdrop, Option<Duration>) {
        let display = ctx.input(|i| i.viewport().monitor_size).map(|s| {
            let ppp = ctx.pixels_per_point();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let px = |v: f32| (v * ppp).round().clamp(1.0, 16_384.0) as u32;
            (px(s.x), px(s.y))
        });
        // High contrast omits the wallpaper entirely, so nothing is decoded for it either.
        let allowed = !look.high_contrast;
        let active_want = Self::want_for(active, look.quality, display).filter(|_| allowed);
        let preview_want = preview
            .and_then(|p| Self::want_for(p, look.quality, display))
            .filter(|_| allowed);
        let previewing = preview.is_some();
        // A preview of the same wallpaper shares the active slot (one decode, one texture).
        let shares = previewing && preview_want == active_want;
        self.receive();

        for slot_id in [Slot::Active, Slot::Preview] {
            let (want, wanted) = match slot_id {
                Slot::Active => (active_want.clone(), Some(active)),
                Slot::Preview if shares => (None, None),
                Slot::Preview => (preview_want.clone(), preview),
            };
            if self.slot_mut(slot_id).want == want {
                // Reuse decoded frames; only retry an idle slot whose request was superseded.
                let idle = matches!(self.slot_mut(slot_id).load, Load::Idle);
                if !(idle && want.is_some()) {
                    continue;
                }
            } else {
                if matches!(self.in_flight, Some((busy, _)) if busy == slot_id) {
                    self.worker.cancel_all();
                    self.in_flight = None;
                }
                let slot = self.slot_mut(slot_id);
                slot.reset();
                slot.want.clone_from(&want);
            }
            let Some(want) = want else { continue };
            if matches!(self.in_flight, Some((busy, _)) if busy != slot_id) {
                continue; // one request at a time; this slot starts when the other finishes
            }
            self.request(slot_id, &want, store, wanted.and_then(|w| w.staged));
        }

        let displayed = if previewing && !shares {
            Slot::Preview
        } else {
            Slot::Active
        };
        let mode = Mode::for_look(look, window.0, window.1);
        let wake = self.advance(ctx, displayed, mode, now);
        let slot = self.slot_mut(displayed);
        // Texture sides are at most 2048, exactly representable as `f32`.
        #[allow(clippy::cast_precision_loss)]
        let texture = slot
            .texture
            .as_ref()
            .map(|t| (t.handle.id(), Vec2::new(t.size[0] as f32, t.size[1] as f32)));
        (
            Backdrop {
                texture: if look.wallpaper == Wallpaper::None {
                    None
                } else {
                    texture
                },
                fit: look.fit,
                scrim: background::scrim_alpha(&look.palette, look.dim),
                material: look.material,
            },
            wake,
        )
    }

    fn advance(
        &mut self,
        ctx: &egui::Context,
        slot_id: Slot,
        mode: Mode,
        now: Instant,
    ) -> Option<Duration> {
        let writes = &mut self.stats;
        let slot = match slot_id {
            Slot::Active => &mut self.active,
            Slot::Preview => &mut self.preview,
        };
        let Load::Ready(media) = &slot.load else {
            return None;
        };
        let media = Arc::clone(media);
        let playback = slot
            .playback
            .get_or_insert_with(|| Playback::new(Timeline::from_media(&media)));
        if slot.texture.is_none() {
            playback.invalidate_texture();
        }
        let step = playback.step(now, mode);
        if step.upload || slot.texture.is_none() {
            let frame = &media.frames[step.frame.min(media.frames.len() - 1)];
            let size = [media.width as usize, media.height as usize];
            let image = ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
            match &mut slot.texture {
                Some(texture) if texture.size == size => {
                    texture
                        .handle
                        .set_partial([0, 0], image, TextureOptions::LINEAR);
                }
                other => {
                    // Allocated once per wallpaper; later frames are same-size sub-updates.
                    *other = Some(Texture {
                        handle: ctx.load_texture("athena-wallpaper", image, TextureOptions::LINEAR),
                        size,
                    });
                    writes.texture_allocations += 1;
                }
            }
            writes.texture_writes += 1;
        }
        step.wake
    }

    /// Playback time elapsed for a slot's wallpaper, if it has a clock.
    #[must_use]
    pub fn elapsed(&self, preview: bool) -> Option<Duration> {
        let slot = if preview { &self.preview } else { &self.active };
        slot.playback.as_ref().map(Playback::elapsed)
    }

    /// The texture currently bound for a slot, for tests and diagnostics.
    #[must_use]
    pub fn texture_id(&self, preview: bool) -> Option<TextureId> {
        let slot = if preview { &self.preview } else { &self.active };
        slot.texture.as_ref().map(|t| t.handle.id())
    }
}

#[cfg(test)]
mod tests;
