# Athena next-step readiness and appearance review

**Date:** 2026-11-16
**Scope:** Review the 1.0.2 foundation, 1.1 workflow/design plan, 1.2 goals/closings plan, and the supplied JavaScript/WebGL and CSS references. This is a review and requirements supplement, not a replacement executable implementation plan.
**Verdict:** Proceed with foundation verification, plan reconciliation, and a bounded native rendering experiment. Do not execute the existing 1.1/1.2 task examples unchanged or describe those releases as ready to ship.

## Evidence and authority

- Local HEAD is `f16e553`, version 1.0.2, with the transaction-row height fix and smoke-note changes still uncommitted at review start.
- A fresh `git ls-remote` found remote `main` and `v1.0.1` at `3b934e3`; no remote `v1.0.2` ref was returned. These are observations, not release instructions.
- Source inspection covers the Rust/egui shell, settings, template/export contracts, native options, renderer dependency, and the relevant future-plan task contracts. Committed Month/Dark and Settings/Light screenshots were inspected as visual evidence; no new interactive app walkthrough or shader prototype was performed.
- The existing smoke record reports automated verification of the height fix but explicitly leaves final-artifact launch, real Windows scaling, and other native checks incomplete. This review does not reattribute older launch results to rebuilt artifacts.
- Fresh baseline checks during this review passed: 271 tests (265 unit, 1 design-assets, 5 fonts), formatting, Clippy with warnings denied, document lint, whitespace checks, and all 21 relative document links in the reviewed/updated planning documents. These do not validate the unimplemented media/glass features or close the native release gaps.
- The [roadmap decisions](2026-11-13-athena-roadmap-revision.md) D1–D5 and the [1.1](2026-11-13-athena-1.1.0-revision-brief.md) / [1.2](2026-11-13-athena-1.2.0-revision-brief.md) briefs remain authoritative. Preserve Semibold main text, real Bold titles, tabular Semibold amounts, complete-entry settlement, full-state templates, and separate Funded/Completed semantics.
- The user explicitly requests Rust as the main language, no Tauri/Electron migration, more customization, and GIF wallpaper **across all main screens**. Exact glass composition, performance targets, presets, and additional features below are recommendations for planning, not approval of unseen artwork or proven performance.

## Problems to resolve before executing feature plans

These are plan defects found by inspection, not claims that unimplemented features are already broken in the current app.

| Priority | Contract gap | Required correction and regression target |
|---|---|---|
| P1 | 1.1 Task 2 independently sums paid and owed, counts only missing payers, and selects wording using active people. | Select complete expense/refund entries once for both sides. Track exclusion reasons and unique excluded entries; show a repair filter. Historical participants determine two-person wording. Assert zero-sum balances for unknown payer, missing/invalid splits, refunds, and archived participants. |
| P1 | 1.1 Task 5 restores by renaming files but startup only recovers when the live path is missing. Both live and `.old` can exist after an interrupted replacement. | Define a durable recovery state machine, candidate/schema validation, WAL/handle ownership, and rollback without overwriting an occupied Windows path. Test every failure/restart boundary with existing data. Keep recovery snapshots protected from retention until recovery completes. |
| P1 | 1.1/1.2 examples keep JSON format 1 and use additive defaults; a previous reader can silently omit new collections during replacement. | Explicit version dispatch: format 1 adapter, format 2 for 1.1, format 3 for 1.2. Reject future formats before replacement. Keep database and document versions separate; preserve available metadata and document defaults for metadata absent in old exports. |
| P1 | 1.1 migration and Task 8 template DTO have only `person_id`; payer and custom/even responsibility cannot round-trip. | Define responsibility mode, participant instructions, payer, amount-edit behavior, archived-reference repair, and today's date together in schema, DTOs, form state, and import tests. No automatic save or silent draft overwrite. | <!-- lint:allow -->
| P1 | 1.2 Task 2 calls funding `is_complete`, lacks pre-archive workflow state, and widens percentage multiplication without bounding aggregate contributions. | Use derived `is_funded`; manual completion remains a workflow action. Persist pre-archive status. Reject contribution-total overflow inside the write transaction; verify overfunding, reopen, and restore from every workflow state. | <!-- lint:allow -->
| P1 | 1.2 Tasks 1/5/7 retain `previous_net` and aggregate/max-timestamp fingerprints. Equal net values can hide different spending, and edits to nonmaximum-timestamp rows or the prior month can be missed. | Store previous spending, category winning amount, and canonical relevant inputs for both months. Capture consistently; cache stale checks on invalidation. Test equal-net/different-spending months, offsetting edits, clock rollback, and previous-month edits. | <!-- lint:allow -->
| P2 | Trash and Goals screens promise item/detail lists, but their named read models supply counts/progress only. Data-version-only refresh also misses midnight changes. | Add selected trash/goal detail data, explicit selection/write/restore invalidation, label refresh after renames, and injected local-date invalidation on midnight/resume. No SQL from rendering. |
| P2 | 1.1 Task 7 uses an undefined `CategoryTotal.name` fixture, all-entry counts for spending wording, and an ambiguous zero/negative-spending percentage. | Resolve names by ID. Distinguish activity from spending; specify income-only/refund-only/zero/negative comparison sentences and checked percentage arithmetic. | <!-- lint:allow -->
| P2 | XLSX acceptance checks only ZIP magic/size. Arbitrary `i64` cents converted to floating point cannot promise cent-exact numeric cells. | Read workbook contents in tests. Verify scope, localized headers, cell types and values. Use a documented conservative numeric range with cent-round-trip checks; export larger values as exact decimal text, clearly labeled. Never silently round supported app money or use numeric totals over text fallbacks. |
| P2 | A lexical SQL guard can match its own view definition or miss multiline/qualified SQL; it cannot prove trash isolation. | Allow the intentional migration view definition; cover query forms deliberately. Add behavioral delete/restore checks for reports, settlement, duplicates, search, calendar, and every exporter. Treat scanning as a supplementary guard. |

Additional plan hygiene: at review start, 1.1 named shipped 1.0.0 as its base despite its 1.0.2 revision header, and 1.2 assumed an already shipped 1.1. The plan headers now state the actual prerequisites. Still reconcile every action, DTO, fixture, signature, schema field, and release document before removing the execution warning. The compact designer brief retains pre-build/type-exploration language; refresh it and its historical appendix mirror together when the visual contract is finalized, preserving its character limit.

## Requested-feature feasibility

| Request | Feasibility in this Rust app | Recommended placement |
|---|---|---|
| GIF background | Feasible with bounded Rust decoding, egui textures, and scheduled repaints. Current direct dependencies do not enable GIF decoding. | 1.1 appearance workstream, across Month, Calendar, Settings, Trash, and later Goals. |
| More customization | Feasible through typed persisted appearance preferences and shared tokens. Existing profile settings only cover theme/language/basic profile values. | 1.1 appearance foundation; 1.2 inherits it. |
| Apple/Onyx hierarchy and animations | Feasible using native Rust components, egui animation state, embedded icons, and restrained materials. | 1.1 shared system before individual feature styling. |
| Reference-like refraction/glass | Feasible in principle through a wgpu paint callback and a WGSL port. Visual parity, driver behavior, and cost require a native prototype. | Prototype before committing the effect to the release gate; optional enhanced path with a solid fallback. |
| CSS backdrop blur over arbitrary UI | CSS does not run in egui. Blurring all already-rendered widgets needs additional render-target/compositing design. Wallpaper-only glass is a simpler first scope. | Sample the wallpaper texture for the first glass implementation; keep foreground widgets crisp. |

The references are design/algorithm examples, not a drop-in component. The JavaScript contains a full-screen tiled shape/refraction shader, RGB sampling offsets, cursor lighting, ripples, zoom/pan, and a remote MP4 video element. CSS separately supplies translucent gradients, borders, shadows, and backdrop blur. It does not implement a native window material, and copying the full-screen distortion would distort a finance workspace rather than establish useful hierarchy.

`eframe` 0.37 defaults to wgpu, and Athena does not override that renderer. `egui-wgpu` 0.37 exposes `CallbackTrait` preparation/paint stages for native GPU work. Keep those versions aligned; do not independently add the newest incompatible wgpu. The `image` crate supports GIF animation decoding and dimension/allocation limits, but decoder limits alone do not bound an application that collects every decoded frame. See the [renderer API](https://docs.rs/crate/egui-wgpu/0.37.0/source/src/renderer.rs), [eframe backend documentation](https://docs.rs/crate/eframe/0.37.0/source/README.md), [GIF decoder source](https://docs.rs/crate/image/0.25.10/source/src/codecs/gif.rs), and [decode limits](https://docs.rs/image/0.25.10/image/struct.Limits.html).

No browser, DOM, JavaScript runtime, Tauri, or Electron is required. No remote image/video loading is proposed: use embedded presets or explicitly selected local files, keeping Athena offline. MP4 playback, arbitrary URL loading, and OS desktop transparency are outside this first scope.

## Design direction and alternatives

1. **Recommended: layered Onyx workspace.** Media spans the main shell; a scrim stabilizes contrast; solid financial cards sit above it. Subtle wallpaper-sampling glass belongs on navigation, selected controls, and lightweight chrome. This gives the requested character while keeping forms and financial reading dependable.
2. **Simpler: static wallpaper and painted materials.** Gradients, edge highlights, and solid tinted surfaces need less GPU work. Useful as the first checkpoint and permanent fallback, but it does not reproduce animated refraction.
3. **More expensive: live compositing of the entire interface.** Arbitrary backdrop blur/refraction can reflect other panels, but needs additional render passes, clipping/overlap rules, and more driver testing. Defer this scope until wallpaper-only rendering proves valuable and fast enough.

Treat financial screens as an operating workspace. The current goldens already provide clear amounts, consistent spacing, and a complete light theme. Improve hierarchy and interaction before increasing decoration: make the balance header more compact, replace repeated fallback navigation glyphs with distinct embedded icons, keep labels, align controls, and retain a stable amount column. Avoid adding nested glass panels to every existing card.

Preserve Windows title-bar behavior, resize/snap affordances, and keyboard conventions. Apple influence means hierarchy, coherent materials, and precise feedback; the first scope does not need replacement window chrome. Do not bundle Apple platform assets as if they were unrestricted cross-platform icons; use a suitably licensed embedded family with documented provenance.

Apple's materials guidance separates controls from underlying content and emphasizes thicker surfaces when text needs contrast. Its motion guidance supports purposeful feedback and accessibility. Athena's specific colors, timing values, and GPU design remain project recommendations: [materials](https://developer.apple.com/design/human-interface-guidelines/materials.md), [motion](https://developer.apple.com/design/human-interface-guidelines/motion.md).

## Proposed 1.1 appearance contract

### Shared composition

- One wallpaper instance belongs to the shell and persists across navigation; switching screens does not restart playback or load a separate copy. Onboarding and recovery/error startup remain simple solid surfaces unless separately designed.
- Layer order: wallpaper -> optional wallpaper blur/refraction -> contrast scrim -> stable reading surfaces -> crisp widgets/text -> dialogs/toasts. Main screens all share it, including future Goals. A form or destructive confirmation gets a solid or sufficiently opaque surface.
- Wallpaper is decorative and never consumes clicks, scroll, drag, keyboard focus, or accessibility navigation. The reference's global wheel/drag/ripple handlers do not belong in the finance shell.
- Keep Semibold body/controls, Bold titles, and frozen-tabular Semibold amounts/dates. Customization changes scale and density, not the selected font family or financial precision.
- Default appearance remains usable without importing media. Background animation requires user selection; Reset Appearance restores the solid default without changing financial data.

### Useful customization

- Full dark/light modes; optionally System theme through an explicit preference distinct from the resolved palette.
- Curated Onyx, light Pearl, and restrained Aurora appearance presets. Presets are proposed names/directions, not approved final palettes.
- Curated accent choices whose text, focus, selection, chart and primary-button roles are validated independently; arbitrary unvalidated text colors are excluded from the first version.
- Comfortable/Compact density and a persisted app text-scale control with Reset. Preserve readable amounts and the supported window widths; layout changes must not restart or lose an open draft.
- Background selection: None, built-in static, local PNG/JPEG, local GIF. Provide fit mode, preview, replace/remove, pause/resume, and constrained dimming/material strength. Preserve aspect ratio; the crop preview must match the main viewport.
- Motion preference: System, Reduced, Full, plus Reduce Transparency and quality Auto/Low/High. Effective reduced motion freezes GIFs to a poster and removes spatial effects. System mode follows Windows where available and falls back to reduced motion if preference detection fails. High-contrast mode forces solid readable surfaces. The Pause action stops decorative shader loops too.
- A clear Preview / Apply / Cancel workflow for multi-setting edits. Cancel restores the previous applied settings and asset; invalid media leaves the current appearance intact. Reset only affects appearance, with its scope stated.

### Media lifecycle, storage, and resource limits

- Decode on a worker with cancellation and generation IDs; stale results cannot replace a newer selection or update a closed preview. UI rendering performs no disk reads or decode work.
- First implementation uses bounded decoding before applying an asset, with explicit iteration/frame accounting rather than unbounded `collect_frames`. Validate magic/format, canvas dimensions, frame rectangles, disposal/transparency handling, frame count, and checked allocation sizes. Reject malformed/truncated media with a localizable error.
- Initial engineering budgets: source <= 16 MiB, dimensions <= 2048 per axis, <= 300 decoded frames, aggregate retained RGBA <= 128 MiB. Account separately for decoder working buffers and GPU render targets; these numbers are validation targets, not an assertion that total process memory stays below 128 MiB. Reject over-budget input before retaining the excess frame. Display a useful resize/shorter-animation suggestion. Static decode uses the same dimension/source limits.
- Preserve frame timing and compositing, normalize zero/very short delays to a documented minimum, and cap decorative presentation at 30 fps by elapsed-time frame selection. Honor finite GIF loop metadata. Do not spin a busy loop or upload textures on unchanged frames.
- Stop animation work when minimized/hidden; pause when unfocused by default. Resume from the paused position without replaying a backlog. Schedule only the next animation or interaction repaint; static, paused, and settled states do not continuously repaint.
- Use bounded live texture ownership; replacing/removing an asset releases old CPU/GPU buffers after outstanding work is finished. Preview plus main view must share/cache data within a stated combined budget. Handle resize, DPI changes, and missing GPU resources with a solid fallback.
- Recommended storage policy: managed files under the resolved app data root (including portable mode), with an opaque internal asset ID in preferences. Explicit Store-owned actions authorize staged writes, apply, reset, and cleanup; the decoder only produces results. Copy/import atomically and never delete the user's original file. Cancel removes only staged, unreferenced app-owned assets. Avoid absolute path disclosure in logs and JSON.
- This changes the existing "everything in one SQLite file" description for appearance media: financial data remains in SQLite, while wallpaper bytes are optional local assets. Database backups preserve the preferences/reference but do not implicitly contain external media. JSON exports carry portable appearance values, without machine paths or raw GIF data; imported settings use a solid fallback until media is reselected. Preview must disclose this, and README/export/backup copy must distinguish financial completeness from optional appearance assets. A later appearance package can include media; do not silently promise it now.
- Import/restore resets worker generations, view caches, and invalid asset references before presenting the new profile. Missing/unreadable files recover to the solid theme without blocking startup or financial operations. Asset cleanup never touches user-selected source paths or financial backups.

### Native glass implementation boundary

- Prototype one panel using the existing wgpu renderer and a version-matched callback. Port the relevant shader mathematics to WGSL; do not embed the JavaScript program.
- Share a decoded wallpaper texture. Apply restrained refraction/chromatic edge light inside a rounded panel mask, then draw ordinary text and inputs above it. Never refract numbers, labels, focus rings, or accessibility hit regions.
- For real wallpaper blur, use a bounded lower-resolution render target and separable passes in preparation; reuse it across panels for a frame. Do not read the framebuffer back to the CPU, blur every panel separately, or sample the same texture currently used as a render attachment.
- Translate logical rectangles, physical pixel coordinates, crop UVs, and clipping consistently. Handle premultiplied alpha and color-space conversions deliberately. Verify opaque, transparent and bright wallpapers in both themes rather than copying CSS alpha values.
- Limit pointer lighting to subtle optional chrome feedback. Ripples, continuous zoom, tiled geometry, and full-screen parallax are excluded from the initial finance UI.
- Provide Solid / Tinted fallback paths when reduced transparency, high contrast, low quality, or resource creation failure requires them. Switching paths preserves navigation, input focus, scroll, draft state, and all financial data.
- Keep renderer resources in a UI-owned appearance runtime, independent from Store financial state. Cache shaders/pipelines/textures; appearance animation must not invalidate report queries or trigger database refreshes every frame.

## Other valuable additions

| Priority | Addition | Reason and acceptance |
|---|---|---|
| 1.1 core | Persistent operation/recovery status | Import/restore/backup previews show source, target, counts and recovery location. Busy state prevents double submit; cancellation is offered only before an irreversible stage. Recovery details remain in Settings after a toast disappears. |
| 1.1 core | Deliberate draft protection | Navigation, template application, appearance preview, and modal dismissal preserve an edited transaction draft or present Keep Editing / Discard. Do not silently clear a form. An in-session retained draft does not imply crash recovery. |
| 1.1 core | Keyboard help, search focus, visible filter scope | Keep Ctrl+1/2/3 fixed, Trash Ctrl+4, Goals later Ctrl+5. Add discoverable search focus and shortcut help; consume keys only in the correct context. Active filter chips and Clear Filters explain empty registers and the settlement repair subset. |
| 1.1 optional | Hide amounts / presentation privacy | A session toggle masks balances, money fields outside active entry, and financial summaries across main screens. Accessible output must follow the intended mask. Clearly disclose that exports and the database remain unchanged; this is not encryption or an app lock. |
| 1.2 core | Clear goal and closing distinctions | Show Funded beside actual contributed/target, remaining or excess; manual lifecycle status stays separate. Label saved closing values versus current values, comparison month, and stale state. Preserve these rules under every appearance preset. |

Defer bank integration, budgets, recurring automation, custom font loading, arbitrary remote themes/media, a full command palette, and replacement native window chrome. They add separate contracts without resolving the current workflow and design gaps.

## Recommended delivery order and ownership

| Stage | Files/modules | Deliverable and gate |
|---|---|---|
| Foundation | Existing 1.0.2 row fix, smoke record, release artifacts | Review the exact dirty diff; complete outstanding native verification on the final artifacts before claiming 1.0.2 release acceptance. Preserve v1.0.1. |
| Plan reconciliation | Existing 1.1/1.2 plans, revision briefs, design spec, designer brief/mirror | Rewrite conflicting snippets, schema/DTO contracts, and regression fixtures. Resolve the P1 gaps above. Do not use a header link as a substitute. |
| Appearance foundation | Proposed `src/domain/appearance.rs`, `src/infrastructure/db/appearance_repo.rs`, migration 0002, `src/app/actions.rs`, `store.rs`, `view_model.rs`, theme and Settings | Typed defaults/validation, persisted preferences, preview/apply/cancel, curated solid presets, reduced-motion/transparency behavior. Extend format 2 deliberately. |
| Media | Proposed `src/infrastructure/media/{mod.rs,decode.rs,assets.rs}`, `src/ui/appearance/{mod.rs,background.rs}`, `src/app/mod.rs`, shell | Bounded local image/GIF ingestion, cancellation, lifecycle/cache ownership, managed paths and truthful backup/export behavior. Static fallback works first. |
| Glass experiment | Proposed `src/ui/appearance/{glass.rs,glass.wgsl}` and version-matched renderer dependency in Cargo | Isolated native panel proves masking, resize/DPI, compositing, fallback, and measured frame cost before broad rollout. No finance migrations or framework replacement. |
| 1.1 workflows | Existing 1.1 tasks, reconciled migration 0002 and format 2 | Recovery/import/trash first, then settlement and fast entry; integrate shared components and the rest of 1.1 features in reviewable checkpoints. A failed glass experiment falls back without blocking financial correctness. |
| 1.2 | Existing 1.2 tasks, migration 0003 and format 3 | Goals and closings consume the established appearance runtime; do not create a second renderer, media library, or animation system. |

The proposed paths are ownership boundaries for the next detailed plan, not claims those modules already exist. Define that plan's exact API/test contracts before implementation. Keep financial writes and preference changes within the existing action boundary. Keep media decode and animation independent of database mutation. Publish/commit/tag remain separate user-directed actions.

## Acceptance before enabling the enhanced appearance

- Both themes and languages; Month/Calendar/Settings/Trash/Goals where available; long labels, maximum amounts, empty and populated states; minimum window and 100/125/150/200% Windows scaling.
- Snapshot stable solid and glass states with an injected clock and fixed wallpaper frame. Inspect the composed pixels; a token-only contrast test cannot prove contrast over changing media. Normal text remains >= 4.5:1 over worst-case bright/dark frames and all focus/selection states; enforce surface opacity/scrim limits rather than trusting user wallpaper color.
- Decode tests: disposal methods, transparent frames, short/zero delays, finite loops, corrupt/truncated files, oversized dimensions, excessive frame counts, aggregate-memory rejection, cancellation, replacement races, and missing assets after restore/portable moves.
- Input tests: keyboard-only navigation, focus return, screen-reader labels, no wallpaper event capture, navigation during animation, open-draft preservation, and repeated Save/Enter without duplicate financial writes.
- Lifecycle/performance: benchmark a named reference Windows device at its declared resolution/DPI. Target p95 interactive frames <= 16.7 ms at 60 Hz; wallpaper-only updates <= 30 Hz. Report measured CPU/GPU time, allocations and resident memory, and compare disabled/static/GIF/glass paths. Budgets are proposed targets; no measurements have been made in this review.
- Verify zero decorative timer-driven repaints after pause/reduced-motion settling and while minimized. No media decoding or report refresh on each paint; no growth after repeated preview/apply/remove cycles. If targets fail, reduce resolution/effects or select the solid fallback.
- Run formatting, Clippy, full tests, release build, document lint, migration/format compatibility, and final-artifact native smoke. Add explicit appearance controls and managed-media/backup notes to localized copy, README, and smoke checklists.

## Readiness decision

The product direction is coherent and the requested effects are technically plausible within Rust/egui. The release plans remain **conditional**, because their old examples conflict with already-approved correctness contracts and their new renderer/media requirements have not been prototyped. The next safe work is the reconciliation and a small native rendering experiment, followed by staged implementation of the agreed 1.1 workstreams. Keep 1.2 dependent on verified 1.1 contracts.
