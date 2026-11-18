# Athena appearance readiness: independent analysis, plan and implementation prompt (Opus 5.5)

**Date:** 2026-11-17
**Requested by:** [Codex prompt for Opus](2026-11-17-codex-prompt-for-opus.md)
**Base:** `bc9f18f` (= `main` = `v1.1.0^{}`), with the parent's uncommitted `.github/workflows/ci.yml` and `docs/superpowers/plans/README.md` edits and the untracked Codex plan. This review wrote only this file.
**Inputs read in full:** [readiness and appearance review](../superpowers/specs/2026-11-16-athena-readiness-and-appearance-review.md), [contracts](../superpowers/specs/contracts.md) (§0–§3, §8–§15), [Codex 1.3 plan](../superpowers/plans/2026-11-17-athena-native-appearance.md) (current on-disk version), [1.1 smoke record](../smoke-1.1.md), the Task 11/release sections of the [1.1 Safe Data plan](../superpowers/plans/2026-11-16-athena-1.1.0-safe-data.md), the outline and open items of the [review remediation plan](../superpowers/plans/2026-11-16-athena-review-improvement-plan.md), the working diff, and the source named below.

**Verdict.** The D9 order stands: 1.2 Daily Speed, 1.3 Appearance, 1.4 Goals and Closings. The disposable glass spike can start now and needs **no** dependency change. Persisted 1.3 work should wait for three things: the 1.2 baseline, a fix for one confirmed startup-recovery defect (F1), and a multi-version import dispatcher (F4). The Inno Setup packaging fix is sound. Its remaining risks are operational rather than correctness problems (F7). Several of Codex's numeric media budgets contradict each other, and I propose a coherent replacement (§4.3).

## 1. Evidence classes and what was run

Each finding carries one tag:

- **[T]** Reproduced by a test that I ran.
- **[I]** Established by reading source, the lockfile or crate sources.
- **[P]** A proposal, not evidence.

| What | Result | Class |
|---|---|---|
| Recovery probes A–D (Appendix A). They ran in a **scratch copy** of `HEAD` (`git archive`) with a separate `CARGO_TARGET_DIR`, so the checkout and `target/` were untouched. | `cargo test --locked --offline --lib opus_probes`: **4 passed**. Each probe asserts the defective behaviour, so a pass confirms the defect. | T |
| `cargo tree --locked --offline` for `image`, `eframe`, `windows`, `egui-wgpu` | `image 0.25.10` in the normal graph has only `bmp`+`png` (via `arboard`/`eframe`). The `gif` crate is absent from `Cargo.lock`. eframe is built wgpu-only (no `glow` feature). `windows 0.62.2` is already in the graph. | I |
| Registry sources: `egui-wgpu 0.37.0`, `epaint 0.37.0`, `egui 0.37.0`, `image 0.25.10` | Read the callback API, framebuffer format, blend state, texture-update path, `ViewportInfo` and the GIF iterator (cited in §4). | I |
| `gh run view 44089925770`, `gh release view v1.1.0`, published `SHA256SUMS.txt` | The tag run failed in "Build Release & Installer". All four assets were created at 13:09:43Z, but the tag's check job ran 13:09:34Z–13:11:55Z and the package job ran 13:11:57Z–13:17:43Z. The published sums use the names `Athena-1.1.0-setup.exe` and `athena-1.1.0-windows-x64.exe`. | I |
| Parent-supplied, **not re-run here** | `cargo test --locked --all-targets` gave 432 passed and 2 ignored. A fresh locked release build passed. The new Inno download/install block ran locally against a temporary `RUNNER_TEMP`/`GITHUB_ENV`, the pinned SHA-256 matched, and ISCC 6.7.3 built the installer. | reported |
| **Unverified by anyone** | Native interactive smoke of v1.1.0. A hosted GitHub run of the changed workflow. Any glass/GIF performance. Windows preference APIs on a target device. The `gif` crate's default loop semantics (the crate is not in the local registry). | — |

## 2. Prioritized findings

| ID | Pri | Class | Finding | Evidence | Consequence | Direction |
|---|---|---|---|---|---|---|
| **F1** | **P1** | T | **If startup recovery fails or is interrupted after the broken files are moved, the next launch silently creates a fresh empty profile.** `restore_over_broken` renames `athena.db`/`-wal`/`-shm` to `broken-<ts>` first, then installs. If the install fails, the function deletes whatever is at the live path and returns `Err` without moving the broken files back. A crash between the two steps leaves the same state. On relaunch, `Db::open` sees `!path.exists()`, treats it as version 0, creates and migrates an empty database, and `open_session` returns `Active` with onboarding. | `src/infrastructure/db/recovery.rs:39-50`, `:70-74`. `src/infrastructure/db/mod.rs:50-54`. `src/lib.rs:64-76`. Probe A forced the install to fail (`Err(Restore(Failed("unable to open database file")))`) and the relaunch returned `Active, onboarding_complete=false`. Probe B simulated a crash between move and install with the same result. | The user's real data still exists as `broken-*.db` plus backups, but the app shows first-run onboarding and gives no hint. Contract §11 ("stays in recovery") holds only within the session, not across a restart. If the user onboards and keeps working, auto snapshots of the new profile begin to rotate out older daily snapshots after 7 days (`store.rs:70-84`, §9.1). | (a) On install failure, roll the broken files back to the live names, mirroring the partial-move rollback at `recovery.rs:89-97`. (b) Startup guard: if the live DB is missing and any `broken-*` file exists in `root`, enter `Recovery` with an "interrupted recovery" message. Offer **Restore this backup** and an explicit **Start fresh** action, never an implicit fresh profile. (c) Add a §11 contract edit and a §15 line. |
| **F2** | P2 | T | **The SQLite rollback journal is not part of the recovery file set.** `move_broken_aside` and `remove_sqlite_files` handle `""`, `-wal` and `-shm` only. `install_staged` copies with the backup API into a fresh DELETE-journal destination, so the copy itself creates `athena.db-journal`. | `recovery.rs:80`, `:153-158`, `restore.rs:207-221`. Probe C: a stale `athena.db-journal` stayed at the live name after the move. Probe A: a blocked journal name is enough to make the install fail. | A crash during the install can leave a hot journal next to a half-copied file. Old journals are never quarantined with the rest of the evidence. | Add `-journal` to the moved and cleaned suffix sets. Better, install by copying to `root/athena.db.installing`, opening and verifying that file, then renaming it into the free live path. The live path then only ever holds a complete file. |
| **F3** | P3 | T | **A zero-byte `athena.db` is treated as a new install, not as damage.** | Probe D: an empty file at `db_path` returned `Active` with onboarding. | This is rare (truncation by a crash, sync tool or antivirus), but it ends in the same silent-onboarding outcome as F1. | Use the same guard as F1(b): an empty live file while Athena backups or `broken-*` files exist routes to Recovery with **Start fresh** available. |
| **F4** | P1 for 1.2/1.3 | I | **The import dispatcher accepts exactly one current format.** `if ver != FORMAT_VERSION → InvalidVersion` means a plain bump of `FORMAT_VERSION` to 3 would reject every 1.1 (format 2) export as invalid. No byte-stable format-2 fixture exists; `tests/fixtures/` holds only `format1-1.0.2.json`. | `src/application/services/import.rs:204-216`. `tests/fixtures/`. | Format 3 (1.2), format 4 (1.3) and format 5 (1.4) would each break backward import unless the dispatcher becomes per-version. §2 requires "a reader accepts every format ≤ its own". | 1.2 Gate 0: per-version dispatch (`1 → v1 adapter`, `2 → v2 adapter`, …) with a frozen DTO module per version. Commit `tests/fixtures/format2-1.1.0.json` from a real 1.1 export of fixture data. Add a test that every shipped fixture imports under every later build. |
| **F5** | P2 | I | **Quick Add discards an edited draft on Escape or click-outside without confirmation.** `modal.should_close()` emits `Close`, and the test `close_discards_without_saving` codifies the behaviour. The readiness review listed "deliberate draft protection" as core work, but it is not implemented. | `src/ui/quick_add.rs:862-864`. `src/app/mod.rs:1025-1032`. | Under D9 this belongs to 1.2 Daily Speed. 1.3 must not add new ways to lose a draft (preview, system-signal changes, DPI or text-scale changes). | 1.2: Keep Editing / Discard when the draft is dirty. 1.3: a test that appearance changes and system-signal refreshes leave `QuickAddState` intact. |
| **F6** | P2 | I | **Read failures on the startup path become a full Recovery session.** `open_session` runs `AppCore::new`, which runs `sync` and then `ViewModel::reload`, inside the recoverable path. `is_recoverable` includes `AppError::Db`, and `reload` treats `profile_repo::load` as a required query. | `src/lib.rs:50-58`, `:65`. `src/app/view_model.rs:376`. | If 1.3 adds the appearance row as another required query, one malformed appearance value (an unknown enum string or an out-of-range value from a buggy write) would put the whole finance app into the recovery screen. | Load appearance **optionally**, like `settlement_unavailable`. Invalid or unreadable values give the solid default plus a logged warning, and never an `Err` from `reload`. Add a test with a corrupted appearance row that still opens `Active`. |
| **F7** | P2 | I | **The packaging fix is correct, but four release-process seams remain.** (1) The package job runs only on `refs/tags/v*` (`ci.yml:50`), so the changed workflow can only get a hosted check by pushing a tag. (2) `Invoke-WebRequest` runs once with no retry (`:74`), so a transient download failure fails a release. (3) CI hashes `target\release\athena.exe` (`:95`), while the release publishes `athena-1.1.0-windows-x64.exe`, so CI's `SHA256SUMS.txt` names never match the published sums. (4) `AppVersion` is hard-coded in `installer/athena.iss:4` and nothing checks it against `Cargo.toml`. In addition, the v1.1.0 assets were uploaded before the tag CI finished, so they are local builds that CI cannot reproduce byte-for-byte. | Working diff. `gh` timings in §1. | A future tag can fail again for non-code reasons. Hosted validation of the fix is impossible without a tag. Version drift between `Cargo.toml` and the installer would go unnoticed. | Add `workflow_dispatch:` and allow the package job on it (`if: startsWith(github.ref,'refs/tags/v') \|\| github.event_name == 'workflow_dispatch'`). Retry the download 3 times with backoff, keeping the hash check. Rename the CI output to the published name before hashing. Fail the job when `AppVersion != Cargo version`. Close the v1.1.0 smoke on the **downloaded** bytes. |
| **F8** | P3 | I | **The onboarding background animates forever at about 30 Hz,** with no reduced-motion or minimized path (`wobble(...)` driven by `i.time` and `request_repaint_after(33 ms)`). | `src/ui/screens/onboarding.rs:193`, `:219-220`. | This conflicts with the review's rule that onboarding is a simple solid surface and with 1.3's "no decorative repaint under reduced motion" acceptance. | 1.3 routes this through the same effective-motion signal: freeze it under Reduced or High Contrast and stop repainting while minimized or unfocused. |
| **F9** | P3 | I | **A theme change reloads every report.** `SaveProfile` maps to `RefreshScope::All`. | `src/app/store.rs:339-345`. | This is harmless today. If 1.3 copies the pattern for appearance, every Apply re-runs the report queries. | Add `RefreshScope::Appearance`, which loads only the appearance row (optionally, per F6). |
| **F10** | P3 | I | **Plan and record hygiene.** The 1.1 plan's release checkboxes (`safe-data.md:362-376`) and the checkpoint "demonstrate restore and import end to end on a copied real data folder" (`:317`) are unticked even though v1.1.0 is published. All five smoke-record items are open. | Files cited. | Readers cannot tell what was actually exercised natively. | Tick only what has evidence, and record the waiver or outcome for the rest beside the artifact hashes. |

**ViewModel cache (explicitly checked):** no defect found [I]. The narrow `People`/`Categories`/`Trash`/`Backups` scopes are sound because the report read models carry IDs only (`CategoryTotal { category_id, net }`, `PersonTotal { person_id, net }`) and names resolve at render. Settlement does not depend on the archived flag. `LoadKey.today` drives a midnight reload (`load_key_changes_at_midnight`), and `AthenaApp` assigns `vm.today` every frame (`app/mod.rs:571`). Failed keys suppress same-key retries as §8.3 specifies. The only 1.3-relevant risks are F6 and F9.

**Live restore and import boundaries (explicitly checked):** no defect found [I]. Live restore (§9.3, T2) uses the online backup API into the open connection. It takes a snapshot before the copy, verifies afterwards with rollback, protects the snapshot and source from pruning, and the 8-seam injection matrix exists (`restore.rs:677`). Import (§10.5, T3) takes a snapshot and then replaces everything in one transaction. Reads are capped at 64 MiB both by metadata and by `take` (`import.rs:172-188`). The remaining file-swap risk is confined to **startup recovery** (F1–F3).

## 3. Status of the readiness review's original gaps

| Original gap | Status at `bc9f18f` |
|---|---|
| P1: settlement complete-entry selection | Implemented under §7 (1.1). Tests exist, for example `archived_participants_are_included`. Not re-audited here. |
| P1: restore via rename and the recovery state machine | **Closed for live restore** (T2, online backup). **Still partly open for startup recovery**: F1–F3. |
| P1: explicit format version dispatch | Formats 1 and 2 dispatch explicitly and reject future versions. **The forward path is still open**: F4. |
| P1: template DTO and responsibility | 1.2 scope. The §14.1 draft defines `responsibility` and allocations. Still open. |
| P1: goals `is_funded` and closing fingerprints | 1.4 scope. The §14.3 draft addresses both. Still open. |
| P2: trash/goal detail read models; midnight refresh | Trash is done (`vm.trash`, loaded per screen). The midnight refresh is done. Goals belong to 1.4. |
| P2: XLSX precision; lexical SQL guard | 1.2 (§14.1). The guard test exists (`tests/no_transactions_table.rs`) alongside behavioural trash tests. |
| Draft protection (1.1 core addition) | **Not implemented** (F5). Move it to 1.2. |
| Persistent operation status | Done (`vm.last_operation`, §8.2). |

## 4. Native renderer and media assessment

### 4.1 Renderer facts that the spike must respect [I]

1. **No new dependency is needed for the spike.** `eframe 0.37.0` re-exports `egui_wgpu` and `wgpu` (`eframe/src/lib.rs:162`), and the lock already resolves `egui-wgpu 0.37.0` and `wgpu 30.0.1`. Use `eframe::egui_wgpu::{CallbackTrait, Callback, ScreenDescriptor}` and `eframe::wgpu`. Codex's "dependency ownership is Cargo.toml and Cargo.lock" is unnecessary for A1, and adding a direct `wgpu` risks a version split.
2. **Athena is wgpu-only.** eframe is compiled without `glow`, and `lib.rs:29-36` uses the default renderer, so `CreationContext::wgpu_render_state` is `Some` whenever the window opens at all. The contract gate `wgpu_render_state.is_some()` (§14.2) is therefore close to a tautology. The useful gates are adapter type (disable glass by default on `DeviceType::Cpu`, i.e. WARP or software), pipeline/texture creation succeeding, and measured frame time.
3. **The framebuffer is not sRGB and blending happens in gamma space.** `preferred_framebuffer_format` picks `Rgba8Unorm`/`Bgra8Unorm` (`egui-wgpu/src/lib.rs:421-437`). The egui pipeline blends premultiplied colour as `One, OneMinusSrcAlpha` (`renderer.rs:443-453`). A glass shader must **output gamma-encoded premultiplied colour**. It may sample a `*Srgb` view to blur in linear space, but it must re-encode before writing. Codex's "use sRGB-aware blending" is ambiguous on exactly this point. The contrast-floor maths should use the existing gamma-space `palette::composite` and `contrast_ratio` (`src/ui/theme/palette.rs:62`, `:100`), which match what egui actually blends.
4. **`paint` draws inside egui's main render pass** (`CallbackTrait::paint(&self, PaintCallbackInfo, &mut RenderPass<'static>, &CallbackResources)`), so the framebuffer cannot be sampled. Wallpaper-only glass that samples Athena's own wallpaper texture is valid. An offscreen low-resolution separable blur goes in `prepare`, using `egui_encoder` or returned command buffers, and is cached in `CallbackResources` keyed by `(asset generation, frame index, target size)`. `PaintCallbackInfo` provides `viewport`, `clip_rect`, `pixels_per_point` and `screen_size_px`. Derive UVs and scissor from those and nothing else.
5. **Wallpaper painting does not need wgpu at all.** A full `TextureHandle::set` allocates a **new** GPU texture (`renderer.rs:720`, the `pos == None` path). `set_partial([0,0], …)` writes in place and reuses the bind group (`renderer.rs:690-718`, `epaint/src/texture_handle.rs:78`). `ImageData::Color(Arc<ColorImage>)` (`epaint/src/image.rs:16-19`) lets the app retain frames as `Arc<ColorImage>`, already premultiplied, and upload them with an `Arc` clone and no per-frame conversion or allocation. This satisfies §14.2 ("one or two GPU textures updated in place", no `egui::Image`). Paint with `ctx.layer_painter(LayerId::background())` at the start of the frame. It creates no interactive `Area`, so it cannot capture clicks or scroll and adds no AccessKit node.
6. **The shell surfaces are opaque today.** The sidebar (`fill(p.panel)`, `shell.rs:70`), the central panel (`fill(p.bg)`, `:103`) and the cards (`fill(p.card)`, `:112`) all fill, across 39 fill/`Frame::` sites in `src/ui`. Showing wallpaper means making the sidebar and central fills transparent while keeping cards opaque. That is a bounded change in `shell.rs` and `theme`, not in every screen.
7. **Motion.** egui has no reduced-motion hook. Effective Reduced sets `style.animation_time = 0.0` (the egui default is `0.2`) and freezes the wallpaper on its poster frame. Use `ViewportInfo.{minimized, focused, occluded, monitor_size, native_pixels_per_point}` (`egui/src/data/input/viewport_info.rs`) to suspend or pause playback and to size decode targets.

### 4.2 Image and GIF facts [I]

- Turning on GIF decoding adds the `gif` crate, plus whatever it pulls in, to `Cargo.lock`. Neither is present now, so this is a reviewed dependency change that needs network access to update the lock. JPEG adds `zune-jpeg 0.5.15`, which is already locked through dev-dependencies.
- `image`'s GIF iterator applies `Limits` **per frame** (`gif.rs:321-330`, `local_limits`) and yields full composited canvases. Aggregate bounding is the application's job, which agrees with the review.
- **Loop metadata:** `image` maps `gif::Repeat::Finite(0) | Infinite → LoopCount::Infinite` (`gif.rs:432`). If the `gif` crate reports `Finite(0)` for a file without a NETSCAPE loop extension (which I could not verify, because the crate is not available locally), files meant to play once would loop forever. Pin this with fixture tests for: no extension, loop 0, and loop N.
- **Short delays:** browsers treat GIF delays of 0–1 centiseconds as about 100 ms. Codex's proposed 33 ms minimum would play many real-world GIFs about 3× too fast. I propose matching the browser convention (delay ≤ 10 ms becomes 100 ms) and capping presentation separately at 30 fps. This is decision D-A5.

### 4.3 Budget arithmetic: Codex's limits contradict each other [I + P]

Retained RGBA is `w × h × 4 × frames`, and 128 MiB is 134,217,728 bytes.

| Retained frame size | Bytes/frame | Max frames in 128 MiB |
|---|---|---|
| 2048 × 2048 (Codex's per-axis cap) | 16.8 MB | **8** |
| 1920 × 1080 | 8.3 MB | 16 |
| 1280 × 720 | 3.7 MB | 36 |
| 960 × 540 | 2.1 MB | 64 |
| 640 × 360 | 0.92 MB | 145 |
| about 446 × 251 | 0.45 MB | 300 (Codex's frame cap) |

"≤ 2048 per axis", "≤ 300 frames" and "≤ 128 MiB retained" cannot all hold for ordinary animated wallpapers. My proposal [P]:

- Static images: decode at up to 2048 on the long edge.
- Animated images: **downscale at decode** to at most 1280 on the long edge, then step down (1280 → 960 → 640) until `frames × bytes ≤ budget`. Reject above 640 with a "shorten or resize" message. Behind a dimming scrim at ≤30 fps the softness is acceptable, but the user should approve the expectation (decision D-A2).
- Charge each frame **before** retaining it, and count the scaler's scratch buffers separately.
- Streaming decode with a small ring buffer is the alternative if the user wants long or high-resolution GIFs. It costs continuous CPU and would need measuring.

### 4.4 Topic assessments requested by the prompt

- **Format 4.** Under `deny_unknown_fields` (§10.1), format 4 needs a new frozen `format_v4.rs`, and the F4 per-version dispatch must exist first. Its `appearance` section is optional and contains only portable values: `theme_mode, preset, accent, density, text_scale, motion, reduce_transparency, quality, fit, dim`, plus `wallpaper: none | builtin:<id>`. Built-in IDs are portable. Local media, asset IDs, paths and `paused` (session state) are never exported. Format 4 import writes the appearance row **inside the same SQL transaction** as the financial replacement. Importing formats 1–3 should **keep the device's current appearance** (decision D-A3). Codex's "adapt formats 1–3 to explicit solid appearance defaults" would wipe a user's wallpaper just for importing an old file.
- **Asset retention across backup and restore.** Store assets **content-addressed and immutable**: `root/appearance/<sha256>.<ext>`, with the ID being the hash of the validated, normalized bytes. Write through `staging_dir` (same volume, cleared at startup by `run_startup_tasks`), then rename. Apply publishes the file before the DB row references it, as Codex says. 1.3 does **no automatic garbage collection** of applied assets. An explicit "Remove unused wallpapers" action scans the live DB and every Athena backup listed in `backups_dir` read-only (reading the appearance row only if the table exists), then lists what it will delete before deleting. A per-library cap (proposed 16 files / 256 MiB) bounds disk use. Restoring a backup or importing from an external file whose asset is missing falls back to solid with a "choose the wallpaper again" note. Portable mode works unchanged because the assets live under `root`. This is decision D-A1.
- **Windows accessibility signals.** Read "Show animations" with `SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION)`, high contrast with `SPI_GETHIGHCONTRAST` (`HCF_HIGHCONTRASTON`), and transparency with `HKCU\…\Themes\Personalize\EnableTransparency`. The safe WinRT alternative is `windows` 0.62 `UISettings::{AnimationsEnabled, AdvancedEffectsEnabled}`, already in the graph but carrying apartment/activation risk. Either way, sit behind a `PlatformPrefs` provider trait with a fixed fallback (Reduced motion, no transparency). Use `windows-sys 0.61.2`, which is already locked, so §14.2's `windows-sys` plus a scoped `#[allow(unsafe_code)]` holds without a lock change. **Refresh is more than focus/resume:** the Windows high-contrast hotkey toggles while Athena is focused. Re-poll on focus gain, on a change of `raw.system_theme` (winit `ThemeChanged`), and on a cheap 5 s timer only while a decorative effect is active. Avoid `WM_SETTINGCHANGE` subclassing, which needs more `unsafe` than the gain justifies. Under High Contrast, use solid surfaces, no wallpaper and system-like colours. Never consume system input.
- **Safe animation and texture limits.** At most two live wallpaper textures (front plus preview), each `≤ min(2048, device.limits().max_texture_dimension_2d)`. eframe requests the default limits (8192) but uses the downlevel WebGL2 limits on the GL backend (2048), per `setup.rs:261-264`. Upload only when the frame index changes. Schedule `request_repaint_after(next_frame_due)` only while playing and visible. Do zero decorative repaints when paused, reduced, hidden or minimized, and fix F8 the same way.
- **DPI.** Do the Fit/Fill maths in points, do sampling in pixels, and keep one shared function for shell and preview. A scale-factor change re-lays out but **does not re-decode**. The glass blur target derives from `ScreenDescriptor.size_in_pixels / k` and is rebuilt on change. Run the 100/125/150/200 % checks from the review.
- **Draft handling.** Appearance Apply/Cancel lives in Settings and cannot run while the Quick Add modal is open, because the modal blocks it (`app/mod.rs:285`). System-signal refreshes and DPI or text-scale changes can still happen mid-draft, so test that `QuickAddState` survives them. Restore and import intentionally call `ui.settings.reset_all()`, which will also discard an unapplied Appearance draft. Keep that, and say so in the preview copy.
- **Preservation of Rust/egui.** Nothing in scope needs a web runtime, a new GUI framework or a direct `wgpu` dependency. The spike uses eframe's re-exports, and production wallpaper uses egui textures.

## 5. Disagreements with Codex

1. **Spike dependencies:** none are needed (§4.1-1). Remove "Dependency ownership is Cargo.toml and Cargo.lock" from A1. The only lockfile change in 1.3 is enabling `image/gif` and `image/jpeg` in A3.
2. **"sRGB-aware blending":** replace it with "match egui-wgpu: non-sRGB target, gamma-space premultiplied output (`One, OneMinusSrcAlpha`). Blur in linear only via an sRGB view and re-encode" (§4.1-3).
3. **Budgets:** the 2048 / 300 / 128 MiB triple is inconsistent (§4.3). Replace it with decode-time downscaling and fit-or-reject.
4. **Minimum delay:** 33 ms becomes the browser convention, 100 ms for ≤1 cs delays, plus a 30 fps presentation cap (D-A5).
5. **Formats 1–3 on import:** keep the current appearance instead of forcing solid defaults (D-A3).
6. **Asset retention:** Codex leaves it as "retain … or document". I propose a concrete policy: content-addressed, no automatic GC, and explicit scan-then-delete (D-A1).
7. **Windows signal refresh:** focus/resume alone misses hotkey toggles while focused. Add the `ThemeChanged` trigger and a bounded poll.
8. **Glass gate:** `wgpu_render_state.is_some()` is near-tautological in a wgpu-only build. Gate on adapter type, resource creation and measurement.
9. **Recovery before media:** Codex's §"Scope" paragraph calls for "an interruption test around move/install/reopen". The probes show this is a **confirmed defect** (F1), not a precaution. It must be fixed, not just tested, before 1.3 adds lifecycle state under `root`.
10. **Packaging validation:** Codex says to "run tag packaging on a future authorized tag". A `workflow_dispatch` path validates the fix **without** a tag (F7).

Agreements, unchanged: D9 placement, the disposable spike against 1.1, persisted work on a verified 1.2 base, an isolated worker with generations, publish-before-reference, opaque reading surfaces, a solid onboarding and recovery screen, no remote media, video, custom fonts, window transparency or full-UI compositing.

## 6. What gates what

| Item | Gated by | Not gated by |
|---|---|---|
| **Glass spike (S1)** | A named reference Windows device for the measurements. | 1.2, F1, F4, any Cargo change. |
| **1.2 Daily Speed** | F4 (per-version dispatch plus the format-2 fixture) as its Gate 0. F5 inside 1.2. | The spike. |
| **1.3 persisted work (A–F below)** | (1) 1.2 merged with migration 0004, format 3 and their fixtures. (2) **F1–F3 fixed** (in 1.1.1 or 1.2 Gate 0, decision D-A4). (3) F6 rule adopted. (4) Decisions D-A1–D-A3 and D-A5 recorded in contracts §14.2/§15. (5) v1.1.0 native smoke closed or explicitly waived on the downloaded hashes. | Glass. 1.3 ships with solid and tinted surfaces plus wallpaper even if the spike fails. |
| **1.3 release** | F7 workflow fixes exercised on GitHub through `workflow_dispatch`. Smoke on the final hashes. | — |
| **Glass rollout inside 1.3** | S1 meeting its acceptance on the reference device. | The rest of 1.3. |

**Decisions the user owns:**

- **D-A1:** asset retention policy (§4.4).
- **D-A2:** animated-wallpaper fidelity, choosing between downscale-and-reject and streaming decode (§4.3).
- **D-A3:** whether importing formats 1–3 keeps the current appearance.
- **D-A4:** whether the F1–F3 recovery fix ships as a 1.1.1 patch or as 1.2 Gate 0.
- **D-A5:** short-delay normalization.

## 7. Implementation plan

Every step ends with the §13 footer (`cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, doc-lint when docs change). Tests are named before code. Commits, tags and pushes stay user-directed.

### Phase R: recovery hardening (1.1.1 or 1.2 Gate 0; depends on nothing)

- **Files:** `src/infrastructure/db/recovery.rs`, `src/lib.rs` (`open_session`), `src/app/recovery.rs`, `src/ui/screens/recovery.rs`, `src/i18n/{en,pt_br,mod}.rs`, contracts §11 and §15, README recovery paragraph.
- **Changes:**
  - Include the suffix set `["", "-wal", "-shm", "-journal"]` everywhere.
  - Install by copying to `root/athena.db.installing`, opening and running `verify_live_db`, then renaming into the free live path.
  - On any install failure, delete the `.installing` file and move the broken set back.
  - Add a startup guard: if the live DB is missing or zero bytes, and `broken-*` files or Athena backups exist, return `Recovery(RecoveryReason::Interrupted | EmptyDatabase)`.
  - Add a **Start fresh** action with a confirmation that names the kept evidence.
- **Tests:**
  - `install_failure_moves_broken_files_back` (probe A setup; asserts the live name holds the original broken bytes).
  - `relaunch_after_interrupted_recovery_enters_recovery` (probe B setup).
  - `stale_journal_is_moved_with_broken_files` (probe C).
  - `zero_byte_db_with_backups_enters_recovery` (probe D).
  - `fresh_folder_without_evidence_still_onboards`.
  - `start_fresh_keeps_broken_files_and_backups`.
  - Snapshot `recovery_interrupted_en_dark`.
- **Checkpoint:** probes A–D invert to pass. Run a native smoke of recovery against a copied corrupted database.

### Phase P: packaging residuals (parallel with R)

- **Files:** `.github/workflows/ci.yml`, plus a tiny test or script that checks `installer/athena.iss` `AppVersion` against `Cargo.toml`.
- **Changes:**
  - Add a `workflow_dispatch` trigger and allow the package job on it.
  - Retry the Inno download 3 times while keeping the SHA-256 check.
  - Copy `athena.exe` to `athena-<ver>-windows-x64.exe` before hashing and uploading.
  - Assert the version match.
- **Checkpoint:** a hosted `workflow_dispatch` run is green and its artifact names match the release naming. Record the run ID in the smoke record. Separately, run the open `docs/smoke-1.1.md` items against the **downloaded** v1.1.0 bytes, whose hashes already match.

### Phase S1: disposable glass spike (now; depends only on a reference device)

- **Files:** `examples/glass_spike.rs`, `examples/shaders/wallpaper_glass.wgsl`, `docs/design/athena-glass-spike.md`. **No Cargo change.**
- **Contents:**
  - A procedural deterministic wallpaper (gradient plus checker, generated in code), uploaded once with `set_partial`.
  - Background painted with `layer_painter(LayerId::background())`.
  - One rounded panel through `eframe::egui_wgpu::Callback`.
  - `prepare` builds a quarter-resolution two-pass blur cached in `CallbackResources`, rebuilt on a size or generation change.
  - `paint` samples it inside a rounded mask, with gamma-space premultiplied output.
  - Text and a `TextEdit` drawn above the panel.
  - Modes Solid / Tint / Glass selected by keys, plus a "fail resources" toggle that proves the solid fallback.
- **Measure:** p95 frame time, GPU time if available, resident memory, and repaint count while idle, on a named device at 1920×1080 and a recorded DPI, at 100/150/200 %. Record the adapter and driver. Check that `DeviceType::Cpu` disables glass.
- **Exit:** p95 ≤ 16.7 ms, zero idle repaints, no text distortion, and a working fallback. Otherwise 1.3 ships Solid/Tint only.

### Phase A: preferences and persistence (1.3; after 1.2 and R)

- **Files:** `migrations/0005_appearance.sql`, `src/domain/appearance.rs`, `src/infrastructure/db/appearance_repo.rs`, `src/app/{actions,store,view_model}.rs`, `src/ui/screens/settings/{mod,appearance}.rs`, `src/ui/theme/{mod,palette}.rs`, i18n, contracts §14.2 and §15.
- **Changes:**
  - A single-row table with CHECKs that mirror the closed enums and ranges. `profile.theme` stays untouched (§14.2).
  - A domain `Appearance::validate` and a `ResolvedAppearance` (palette, density, text scale and effective motion/transparency after the platform signals).
  - `AppAction::ApplyAppearance(Appearance)` and `ResetAppearance` with `RefreshScope::Appearance` (F9). The optional load follows F6.
  - The palette becomes a runtime value built from curated presets and accents, each validated by `contrast_ratio` tests.
- **Tests:**
  - `v4_to_v5_migration_keeps_rows_people_templates_and_theme`.
  - `appearance_round_trip`.
  - `invalid_apply_leaves_row_and_data_version_unchanged`.
  - `corrupt_appearance_row_still_opens_active_with_solid_default` (F6).
  - `apply_appearance_does_not_reload_reports` (counts report loads, like `trash_loads`).
  - `every_preset_and_accent_meets_4_5_on_its_surfaces`.
  - `quick_add_draft_survives_apply_and_system_signal_change` (F5).
- **Checkpoint:** Settings › Appearance works with solid presets in both locales, both themes and by keyboard.

### Phase B: platform signals (1.3; parallel with A)

- **Files:** `src/infrastructure/platform/{mod.rs,windows_prefs.rs}`, `Cargo.toml` (direct `windows-sys = "0.61"` with the minimal features; already locked).
- **Changes:** a `PlatformPrefs` trait with Windows and fallback implementations, refreshed per §4.4, and applied by setting `style.animation_time` to zero under Reduced. Freeze the onboarding animation (F8).
- **Tests:** `provider_failure_falls_back_to_reduced`, `high_contrast_forces_solid`, `theme_changed_event_triggers_repoll`, `no_decorative_repaint_when_reduced` (onboarding included).

### Phase C: media decode and managed assets (1.3; after A; lockfile change)

- **Files:** `Cargo.toml` and `Cargo.lock` (`image` with `default-features = false, features = ["png","jpeg","gif"]`), `src/infrastructure/media/{mod,decode,assets}.rs`.
- **Changes:**
  - Decode per §4.2–4.3: a source cap of 16 MiB read with `take`, magic sniffing, the per-axis cap, downscale-to-fit or reject, per-frame charging, the delay rule (D-A5), and loop fixtures.
  - A worker with one in-flight job, a latest-wins queue and generation IDs.
  - Content-addressed assets and the explicit cleanup action (D-A1).
- **Tests:**
  - `gif_without_loop_extension_plays_once` (or whatever D-A5/§4.2 settles), plus loop 0 and loop N.
  - `zero_and_one_cs_delays_normalize`.
  - `truncated_gif_rejected`, `oversize_canvas_rejected_before_alloc`.
  - `budget_exceeded_steps_down_then_rejects`.
  - `stale_generation_dropped`.
  - `apply_publishes_file_before_db_reference` (failure injection).
  - `cleanup_keeps_assets_referenced_by_listed_backups`.
  - `missing_asset_falls_back_to_solid`.
  - `source_file_never_modified`.

### Phase D: playback and shell composition (1.3; after C)

- **Files:** `src/ui/appearance/{mod,background}.rs`, `src/ui/shell.rs`, `src/app/mod.rs`, `src/lib.rs`.
- **Changes:**
  - One runtime is owned by `AthenaApp`, not by Store.
  - Two textures are updated with `set_partial`.
  - The background layer painter, transparent sidebar and central fills, opaque cards, and the scrim floor from `composite`.
  - Suspend and pause using `ViewportInfo`.
  - Recovery and onboarding stay solid.
- **Tests:**
  - `navigation_keeps_frame_index_and_scroll`.
  - `no_upload_when_frame_unchanged`.
  - `zero_repaints_paused_minimized_reduced`.
  - `wallpaper_has_no_accesskit_node_and_consumes_no_clicks`.
  - Fixed-frame goldens for Month, Calendar, Settings and Trash in dark/light over a white, black and saturated frame, visually inspected.
  - `scrim_floor_keeps_4_5_over_white_and_black`.

### Phase E: glass integration (1.3; only if S1 passed)

- **Files:** `src/ui/appearance/{glass.rs,glass.wgsl}`.
- **Scope:** chrome only (the sidebar and the selected navigation pill). Disabled under High Contrast, Reduce Transparency, Low quality or a CPU adapter.
- **Tests:** `glass_falls_back_on_resource_error_without_losing_focus_or_draft` and the S1 metrics re-run in the real app.

### Phase F: Format 4 and lifecycle integration (1.3; after A and F4)

- **Files:** `src/infrastructure/export/{format_v4.rs,json.rs}`, `src/application/services/{export,import}.rs`, `src/infrastructure/db/import_repo.rs`, the import preview, README.
- **Changes:**
  - Format 4 per §4.4.
  - Import writes appearance inside the replacement transaction.
  - Formats 1–3 follow D-A3.
  - Restore, import and recovery bump the media generation and re-resolve the asset with solid fallback.
  - The preview copy separates financial completeness from media.
- **Tests:**
  - `every_shipped_fixture_imports` (formats 1, 2, 3, 4).
  - `format5_rejected_as_too_new`.
  - `format4_has_no_paths_or_asset_ids`.
  - `failed_import_keeps_prior_appearance`.
  - `restore_of_backup_with_missing_asset_opens_solid`.

### Phase G: validation and release (1.3)

- Run the §13 release footer.
- Measure S1-style on the reference device across disabled, static, GIF and glass.
- Record a 1.3 smoke against the final hashes, including the recovery-interruption smoke from Phase R.
- Finalize copy, licenses and the designer brief.
- Stop before tagging.

---

## 8. Standalone implementation prompt

> Copy everything below this heading into a fresh agent session. It does not depend on this conversation.

### Role and goal

You are implementing Athena's next safe steps in the Rust/egui Windows app at `C:\Users\PC\athena` (eframe/egui 0.37, wgpu 30.0.1 through eframe, rusqlite 0.42, edition 2024, MSRV 1.95). Follow `docs/superpowers/specs/contracts.md`; it wins over every plan. Release order is fixed by D9: 1.2 Daily Speed, 1.3 Appearance, 1.4 Goals and Closings. Do not migrate away from Rust/egui. Do not add a web runtime, remote assets, video, custom fonts, window transparency or full-UI compositing. Work offline with fixtures only and no real financial data. Do not commit, tag, push or publish unless the user asks in that session.

### Confirmed findings you must act on

These are cited from `docs/reviews/2026-11-17-opus-athena-appearance-analysis-and-prompt.md`; read its §2, §4 and §7 first.

- **F1 (P1, reproduced):** `src/infrastructure/db/recovery.rs:39-50`. When the install fails after `move_broken_aside`, the live path is left empty. When the app crashes between move and install, the same happens. The next launch, through `Db::open` (`src/infrastructure/db/mod.rs:50-54`) and `open_session` (`src/lib.rs:64-76`), silently creates a fresh profile and shows onboarding.
- **F2 (P2, reproduced):** `-journal` is missing from the recovery suffix sets (`recovery.rs:80`, `:153-158`). The install copies into a DELETE-journal destination.
- **F3 (P3, reproduced):** a zero-byte `athena.db` is treated as a new install.
- **F4 (P1 for 1.2/1.3):** `src/application/services/import.rs:204-216` accepts only `FORMAT_VERSION`, and no format-2 fixture exists.
- **F5:** Quick Add discards a dirty draft on Escape or click-outside (`src/ui/quick_add.rs:862-864`). This is 1.2 work, and 1.3 must not regress it.
- **F6:** startup read errors route to Recovery (`src/lib.rs:50-58`, `view_model.rs:376`). Appearance must load optionally.
- **F7:** the CI package job is tag-only, the download has no retry, artifact names differ from the release names, and `AppVersion` in `installer/athena.iss:4` is unchecked.
- **F8:** the onboarding background repaints at 30 Hz forever (`src/ui/screens/onboarding.rs:193`, `:219-220`).
- **F9:** `SaveProfile` → `RefreshScope::All` (`src/app/store.rs:339-345`).

### Order of work (stop and report at each checkpoint)

1. **Phase R, recovery hardening (do first).**
   - Write the failing tests from §7 Phase R. The probe code is in Appendix A of the review.
   - Then implement:
     - The suffix set `["", "-wal", "-shm", "-journal"]`.
     - Install into `root/athena.db.installing`, verify with `restore::verify_live_db`, then rename into the free live path.
     - Roll the broken set back on any install failure.
     - A startup guard that routes "live DB missing or zero bytes while `broken-*` files or Athena backups exist" to `AppSession::Recovery` with a new reason.
     - An explicit, confirmed **Start fresh** action that keeps the evidence.
   - Edit contracts §11 and add a §15 line in the same change. Add EN and PT-BR copy.
   - Checkpoint: all probes inverted and green, full suite green, one native smoke against a copied corrupted database.
2. **Phase P, packaging.**
   - Add `workflow_dispatch` to `.github/workflows/ci.yml` and let the package job run on it.
   - Add a 3-attempt download retry that keeps the SHA-256 check.
   - Publish the portable binary as `athena-<ver>-windows-x64.exe` before hashing.
   - Add a check that `installer/athena.iss` `AppVersion` equals the `Cargo.toml` version.
   - Do not modify the existing pinned hash or URL.
   - Checkpoint: ask the user to dispatch the workflow, then record the run ID.
3. **Phase S1, glass spike.** This may run in parallel and makes **no Cargo changes**.
   - Add `examples/glass_spike.rs` and `examples/shaders/wallpaper_glass.wgsl`.
   - Use `eframe::egui_wgpu::{Callback, CallbackTrait}` and `eframe::wgpu`.
   - Use a procedural wallpaper uploaded with `TextureHandle::set_partial([0,0], Arc<ColorImage>, …)`; a full `set` reallocates.
   - Paint the background through `ctx.layer_painter(LayerId::background())`.
   - Do the quarter-resolution separable blur in `prepare`, cached in `CallbackResources`.
   - Output **gamma-space premultiplied** colour to match egui's non-sRGB target and its `One, OneMinusSrcAlpha` blending.
   - Draw text and a `TextEdit` above the panel.
   - Provide Solid/Tint/Glass modes, a forced-failure fallback, and glass disabled on `DeviceType::Cpu`.
   - Record p95 frame time, idle repaints, memory, adapter and driver at 100/150/200 % in `docs/design/athena-glass-spike.md`.
   - Exit criteria: p95 ≤ 16.7 ms, zero idle repaints, no text distortion.
4. **Prerequisites that belong to 1.2.** Do not start 1.3 persistence without them.
   - Per-version import dispatch, with each version in its own frozen `deny_unknown_fields` DTO module.
   - A committed `tests/fixtures/format2-1.1.0.json` built from fixture data.
   - The test `every_shipped_fixture_imports`.
   - Quick Add Keep Editing / Discard (F5).
5. **1.3 Phases A → B → C → D → (E if S1 passed) → F → G** exactly as listed in §7 of the review, with the tests named there. Binding rules:
   - Appearance is a single-row table from migration `0005`. `profile.theme` stays untouched. Load appearance optionally (F6). Use `RefreshScope::Appearance` (F9).
   - Runtime and media state live in `AthenaApp`, never in `Store`. Animation never bumps `data_version` or reloads reports.
   - Keep at most two live wallpaper textures, updated in place. Never use `egui::Image` or the `egui_extras` loaders for wallpaper.
   - Make the sidebar and central fills transparent, keep cards, modals and destructive confirmations opaque, and enforce a scrim floor proven with `palette::composite` + `contrast_ratio` ≥ 4.5 over pure white and pure black.
   - Under effective Reduced motion, set `style.animation_time = 0`, show a poster frame and do no decorative repaint. High Contrast forces solid. Suspend when minimized or occluded and pause when unfocused.
   - Platform signals come through a `PlatformPrefs` trait (`windows-sys 0.61`, already locked, with a scoped `#[allow(unsafe_code)]`). Refresh on focus gain, on `raw.system_theme` changes and on a 5 s poll only while effects are active. A failed read means Reduced motion and no transparency. Freeze the onboarding animation too (F8).
   - Media: change `image` to `default-features = false, features = ["png","jpeg","gif"]`. This is the only lockfile change; get the user's approval for the network lock update. Cap sources at 16 MiB using `take`. Static images go up to 2048 on the long edge. Animated images are downscaled at decode (1280 → 960 → 640) until `frames × w × h × 4 ≤ 128 MiB`, otherwise rejected with a resize/shorten message. Charge each frame before retaining it. Pin loop semantics and short-delay normalization with fixtures per the user's D-A5 decision.
   - Assets: content-addressed `root/appearance/<sha256>.<ext>`, staged in `staging_dir` and renamed, published before the DB references them. No automatic GC of applied assets. An explicit "Remove unused wallpapers" scans the live DB and listed backups read-only before deleting. Missing assets fall back to solid. Never follow a stored path, and never touch the user's source file.
   - Format 4: an optional portable `appearance` section with no paths, asset IDs, media or `paused`. It is written inside the import replacement transaction. Formats 1–3 follow the user's D-A3 decision (recommended: keep the current appearance).
6. **Do not decide these yourself.** Ask the user for D-A1 (asset retention), D-A2 (GIF fidelity), D-A3 (format 1–3 import and appearance), D-A4 (ship R as 1.1.1 or as 1.2 Gate 0) and D-A5 (short-delay rule). Record the answers in contracts §14.2 and §15 before implementing the dependent phase.

### Verification for every step

Run `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` and `bash docs/superpowers/doc-lint.sh`. Regenerate snapshots only on purpose and inspect them. Releases use `cargo build --release --locked`, the Inno build and a smoke record against the **final** SHA-256 values. Report test evidence, inspection and proposals separately, and never claim native or GPU results you did not measure.

---

## Appendix A: probe code (scratch copy only; not committed)

This module was appended to `src/infrastructure/db/recovery.rs` in a `git archive HEAD` copy and run with `cargo test --locked --offline --lib opus_probes -- --nocapture --test-threads=1`. All four tests passed, each asserting the defective behaviour. Phase R should turn their key assertions into the opposite regression tests.

```rust
#[cfg(test)]
mod opus_probes {
    use super::*;
    use crate::domain::money::Money;
    use crate::domain::settings::{Language, Profile, Theme};
    use crate::infrastructure::db::profile_repo;
    use crate::shared::test_fixtures::{seed_month, seed_people};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn env() -> (tempfile::TempDir, AppDirs, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let source = dirs.backups_dir.join("manual-good.db");
        {
            let db = Db::open(&source).unwrap();
            profile_repo::save(db.conn(), &Profile {
                start_balance: Money::from_cents(10_000),
                theme: Theme::Dark, language: Language::En, onboarding_complete: true,
            }).unwrap();
            let _ = seed_people(db.conn());
            seed_month(&db);
        }
        std::fs::write(&dirs.db_path, b"this is not a database").unwrap();
        (dir, dirs, source)
    }

    fn broken_count(dirs: &AppDirs) -> usize {
        std::fs::read_dir(&dirs.root).unwrap().filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(BROKEN_PREFIX)).count()
    }

    #[test] // A: install failure, then relaunch, lands in onboarding
    fn probe_a_install_failure_then_relaunch_lands_in_onboarding() {
        let (_d, dirs, source) = env();
        std::fs::create_dir(with_suffix(&dirs.db_path, "-journal")).unwrap(); // blocks the copy's journal
        let result = restore_over_broken(&dirs, &source, now());
        assert!(result.is_err());                 // Err(Restore(Failed("unable to open database file")))
        assert!(!dirs.db_path.exists());          // live path left empty
        assert_eq!(broken_count(&dirs), 1);       // evidence kept aside, not moved back
        std::fs::remove_dir(with_suffix(&dirs.db_path, "-journal")).unwrap();
        match crate::open_session(dirs.clone()).unwrap() {
            crate::app::recovery::AppSession::Active(core) => assert!(!core.vm.onboarding_complete()),
            crate::app::recovery::AppSession::Recovery(_) => panic!("stayed in recovery"),
        }
    }

    #[test] // B: crash between move_broken_aside and install_staged
    fn probe_b_crash_between_move_and_install() {
        let (_d, dirs, _source) = env();
        let broken = move_broken_aside(&dirs, now()).unwrap();
        assert!(broken.exists() && !dirs.db_path.exists());
        let session = crate::open_session(dirs.clone()).unwrap();
        assert!(matches!(session, crate::app::recovery::AppSession::Active(ref c) if !c.vm.onboarding_complete()));
    }

    #[test] // C: stale rollback journal is not moved with the broken set
    fn probe_c_stale_journal_is_not_moved_with_broken_files() {
        let (_d, dirs, _source) = env();
        std::fs::write(with_suffix(&dirs.db_path, "-journal"), b"stale journal").unwrap();
        let _ = move_broken_aside(&dirs, now()).unwrap();
        assert!(with_suffix(&dirs.db_path, "-journal").exists());
    }

    #[test] // D: zero-byte live file is treated as a new install
    fn probe_d_zero_byte_live_file_is_onboarding_not_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        std::fs::write(&dirs.db_path, b"").unwrap();
        let session = crate::open_session(dirs).unwrap();
        assert!(matches!(session, crate::app::recovery::AppSession::Active(ref c) if !c.vm.onboarding_complete()));
    }
}
```

Observed output (excerpt):

```text
PROBE A restore result: Err(Restore(Failed("unable to open database file")))
PROBE A relaunch: Active, onboarding_complete=false
PROBE B relaunch: Active onboarding with 1 broken file(s) beside it
PROBE C: athena.db-journal remained at the live path after the move
PROBE D: zero-byte athena.db -> Active onboarding
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 426 filtered out
```
