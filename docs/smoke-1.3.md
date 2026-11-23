# Athena 1.3.0 release smoke record

Scope: Native Appearance (schema 5, document format 4): preferences, Windows platform preferences,
managed local wallpaper (static, GIF and a built-in gradient), the wallpaper compositor with its
contrast-floor scrim, Settings → Appearance and portable format-4 appearance.
Contract: `docs/superpowers/specs/contracts.md` §14.2 and §13. Plans:
`docs/superpowers/plans/2026-11-17-athena-native-appearance.md` and the 1.3.0 section of
`docs/superpowers/plans/2026-11-18-athena-stalled-ui-roadmap-plan.md`.

Status: **candidate, not released.** Nothing below was run against a tagged, CI-built or installed
artifact. Per `docs/dod.md`, a manual check is never marked done unless it was executed against the
recorded final artifacts; any tag or rebuild that changes the hashes invalidates this record.

**Production Glass is not part of 1.3.** No selectable Glass ships, the isolated spike
(`examples/glass_spike.rs`, `docs/design/athena-glass-spike.md`) is unchanged and its record stays
open (see [Open Glass gates](#open-glass-gates)).

## Local release candidate

| Item | Value |
|---|---|
| Source | commit after `3e2036a` on `main` (1.3.0 version bump and this record) |
| Local build | `cargo build --release --locked`, `target/release/athena.exe` |
| Local build size | 21,528,064 bytes |
| Local build SHA-256 | `dd1f5ae6c5f45d97fc56ddbbae3f46b4f836232c7dc24a904538ab5751952f80` |

This is a developer-machine build, not a release artifact. The published artifacts are produced by
the tag CI (`athena-1.3.0-windows-x64.exe`, `Athena-1.3.0-setup.exe`, `LICENSE.txt`,
`ICONS-LICENSE.txt`, `BUILD-INFO.txt`, `SHA256SUMS.txt`); their bytes and SHA-256 hashes are **not
yet recorded** and must be added here before any manual check below is ticked.

## Performed (automated, local, Windows 11, Rust 1.95)

- [x] `bash docs/superpowers/doc-lint.sh`: clean.
- [x] `cargo fmt --check`: clean.
- [x] `cargo clippy --all-targets --locked -- -D warnings`: clean.
- [x] `cargo test --locked --all-targets -- --skip real_wgpu_`: 677 library tests passed (3
  ignored fixture regenerators), plus integration tests `design_assets` 1, `fonts` 5,
  `no_transactions_table` 2, `packaging_metadata` 2 and the `glass_spike` example 9 (the real-GPU
  case is skipped by the filter); 0 failed.
- [x] `cargo build --release --locked`: succeeds.
- [x] `git diff --check`: clean.
- [x] Release launch smoke only: the release `athena.exe` was copied beside an `athena.portable`
  marker in a temporary folder and started. It stayed running for 8 seconds with the window title
  "Athena" and created `data\athena.db` (migrations 1 to 5 on an empty profile) and a log file.
  No interaction was performed; this is not a substitute for the interactive checks below.
- [x] Named regressions of the roadmap exist and pass:
  - A2a: `migration_0005_preserves_profile_and_templates` (from every earlier schema level),
    `bad_appearance_row_leaves_financial_workspace_available`,
    `appearance_apply_cancel_reset_isolated`, every enum and range boundary
    (`sql_checks_match_the_domain_allowlist`), `language_switch_keeps_the_applied_appearance_and_defaults`.
  - A2b: injected supported, missing, changing, focus/resume and high-contrast platform states
    (`app::appearance_platform_tests`, `infrastructure::platform`), the 2-second focused poll that
    is silent while hidden, minimized or unfocused, and the Windows provider reading
    `SPI_GETCLIENTAREAANIMATION`, `SPI_GETHIGHCONTRAST` and `EnableTransparency`.
  - A3a: PNG/JPEG/GIF magic detection, corrupt and truncated input, 16 MiB source, 2048 px and 301
    frame ceilings, GIF delta/disposal/transparency compositing, finite and infinite loops, delay
    normalization, aggregate memory budget with release on failure, one running plus one replaceable
    request with stale-generation results dropped, quota (32 files, 256 MiB), failed Apply cleanup,
    reuse of matching bytes, link/reparse rejection (skipped silently when the test account may not
    create symlinks) and resolution after a full-root copy versus a database-only move.
  - A3b: pinned-time playback (cumulative delays, 30 Hz upload floor, pause, blur, hidden, reduced
    motion poster, no catch-up, no wake-ups when paused or finished), Fit/Fill geometry, the pure
    scrim floor over black, white and saturated colors, rendered-pixel contrast samples through the
    real renderer at two scales in both themes, one live GIF behind Month, Calendar, Settings and
    Trash without a second decode or texture, and same-size sub-texture updates.
  - A4: `snapshot_settings_appearance` (solid and a fixed bright wallpaper frame) and
    `snapshot_shell_with_wallpaper`; keyboard-driven Appearance flows with a failed file, Apply,
    Cancel, Reset, the dirty-leave prompt, an open ledger draft and a same-profile restore; the
    format-4 fixture, explicit 1 to 4 dispatch, malformed enum/range/unknown-field rejection,
    formats 1 to 3 clearing the appearance row, the portable local-to-none export and import
    rollback with an existing asset reference.
- [x] All shipped fixtures (`format1-1.0.2.json`, `format2-1.1.0.json`, `format3-1.2.0.json`,
  `format4-1.3.0.json`) import through explicit version adapters, keep their LF endings, and
  round-trip.
  > Superseded by 1.4.0: JSON export now emits `"format_version": 5` (format 4 remains an accepted
  > import), and the `.xlsx` export adds the full-history `Goals` and `Closings` sheets next to
  > `Entries` and `Allocations`. The format 4 round-trip above is the 1.3.0 record; see
  > `docs/smoke-1.4.md` for the current checks.
- [x] New and changed snapshots were inspected in English/dark and Português/light:
  new `settings_appearance_solid_*`, `settings_appearance_wallpaper_*`, `shell_wallpaper_*`;
  changed `settings_*`, `settings_data_*`, `settings_templates_*` (the Appearance tab, the moved
  theme control and the scope note). The wallpaper is deliberately faint at the minimum dimming:
  the scrim floor guarantees 4.5:1 text over pure white and pure black. Goldens still show the
  previous version number in the sidebar footer (within the pixel tolerance).

## Not performed (no result is claimed)

These need a live Windows window, real hardware or the final artifacts and remain **open**:

- [ ] Windows CI green on the release candidate commit; tag build; bytes and SHA-256 of the final
  portable executable, installer, `LICENSE.txt`, `ICONS-LICENSE.txt`, `BUILD-INFO.txt` and
  `SHA256SUMS.txt` recorded here.
- [ ] Native interactive walk-through on the recorded artifacts: Appearance page with mouse and
  keyboard only, long Português text, both densities, 150 % text scale, Windows high contrast on
  and off, and Windows animation and transparency toggles changed while the window is focused.
- [ ] One live animated GIF viewed across Month, Calendar, Settings and Trash in a real window,
  confirming playback does not restart on navigation; pause, blur and minimize.
- [ ] 100 / 125 / 150 / 200 % Windows display scaling of the composed app (wallpaper, scrim,
  sidebar and cards) and the narrow 640 x 410 window.
- [ ] Repeated Preview / Apply / Cancel / Reset / Remove on a named reference device with bounded
  memory; source and retained media, scratch and GPU memory, frame p95 (16.7 ms target), texture
  writes and decorative wake-ups for wallpaper off, static and GIF. If static or GIF exceeds
  16.7 ms p95 during the reference interaction, decode and render quality must be reduced and the
  run repeated before release.
- [ ] Missing wallpaper after moving only the database or a portable folder (solid canvas with a
  notice) and intact wallpaper after a full data-folder copy, in the real app.
- [ ] Real `1.0.2` or `1.1.x` or `1.2.0` data folder upgraded to schema 5 (installed and portable);
  import of the four shipped formats in the real window; restore of an older backup.
- [ ] Installer install, launch and uninstall of `Athena-1.3.0-setup.exe`; clean Windows machine;
  portable launch with `athena.portable`; the Lucide and Polaris license notices present in both.
- [ ] Carried over from 1.1.x and 1.2.0 and still open: the interactive items listed in
  `docs/smoke-1.1.md` and `docs/smoke-1.2.md`.

These remain open and are not waived by this record.

## Open Glass gates

Production Glass (wallpaper-sampling blur behind chrome) stays unshipped and unselectable in 1.3.
Every gate below must pass on real hardware, with the numbers recorded, before a contract revision
may add it; the spike's own measurements are not repeated here.

- [ ] Real swapchain presentation p95 at most 16.7 ms at 60 Hz for Solid, Tint and Glass.
- [ ] Zero decorative requests and zero frames in five seconds of focused idle. The 1.4 candidate
  moved the two-second check to a Windows worker and removed the periodic preference repaint
  deadline. The native zero-frame sample and a count of worker wake-ups still need measurement.
- [ ] Native 100 / 125 / 150 / 200 % DPI visual checks, interactive resize, typing continuity and
  bright/dark seam and edge inspection.
- [ ] Integrated-GPU and discrete-GPU baselines.
- [ ] Real Windows resource and device-failure behavior. Verify a readable Tint underlay on the
  failing frame and a Solid fallback without an idle retry loop.

The 1.4 candidate reran the full spike test command with `--nocapture` on a GeForce GT 340
(driver `9.18.13.4192`). The nine hardware-independent cases passed. The real-`wgpu` case
printed `SKIP` because a device could not be acquired. No Glass hardware gate was closed.
The [1.4 smoke record](smoke-1.4.md) also records the refreshed 1.4 goldens.
