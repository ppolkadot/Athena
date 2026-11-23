# Athena Implementation Status

Current implementation status on branch `main` (candidate `1.4.0`, schema version `7`, document format `5`).

For historical context, the most recent tagged GitHub release on `origin` prior to this implementation series is `v1.1.1` (`3101d73`). The [milestone smoke records](#milestone-overview) distinguish local automated and packaging evidence from hosted CI, final artifacts, native walkthroughs and hardware measurements.

## Milestone Overview

| Release | Schema | Format | Code & Automated Verification | Smoke Record & Open Gates |
|---|---:|---:|---|---|
| `1.0.0` / `1.0.1` | 1 | 1 | Shipped (`v1.0.0`, `v1.0.1`) | [smoke-1.0.md](smoke-1.0.md) |
| `1.1.0` / `1.1.1` | 3 | 2 | Shipped (`v1.1.0`, `v1.1.1`) | [smoke-1.1.md](smoke-1.1.md) (interactive checks carried forward) |
| `1.2.0` Daily Speed | 4 | 3 | Implemented (`aad8655`–`c84051e`) | [smoke-1.2.md](smoke-1.2.md) (interactive & release-artifact checks open) |
| `1.3.0` Native Appearance | 5 | 4 | Implemented (`622b589`–`a41a4eb`) | [smoke-1.3.md](smoke-1.3.md) (interactive, hardware & Glass gates open) |
| `1.4.0` Goals & Closings | 7 | 5 | Implemented (`833573a`–HEAD) | [smoke-1.4.md](smoke-1.4.md) (interactive, scaling, hardware & release checks open) |

## Feature Status by Milestone

### `1.2.0` Daily Speed (§14.1 — Implemented)
- **Design System & Icons:** Shared tokens (`src/ui/tokens.rs`), embedded Lucide `0.468.0` path icons (`src/ui/icons.rs`, `assets/icons/`), and `<480 pt` collapsible labeled icon rail. The native client-area minimum matches the 640 × 410 pt Month/Calendar harness.
- **Templates & Format 3:** Migration `0004_templates.sql`, `Template` / `TemplateDraft` / `ResponsibilityMode`, `templates_repo`, `integrity::check` coverage, `format_v3.rs` (`DocumentV3`), and LF fixture [format3-1.2.0.json](../tests/fixtures/format3-1.2.0.json).
- **Quick Add & Entry Flow:** Template picker, Save-as-template, Settings Templates management page, broken/archived reference repair, dirty-draft guard across close/Escape/outside-click/replacement/window-close, pure one-line parser (`domain::quick_entry::parse_line`), negative-amount Expense/Refund choice, and duplicate-entry consent in `handle_quick_add`.
- **Day Drill-Down & Insights:** `DayActivity` grid and shared day detail panel across Calendar (`Days` button) and Month (`Days` toggle) with keyboard navigation, plus localized EN/PT-BR monthly insights (`InsightInputs` / `i18n/insight.rs`) using `vm.today`.
- **XLSX Export:** `rust_xlsxwriter` writer (`src/infrastructure/export/xlsx.rs`) with `Entries` and `Allocations` sheets, formula-safe explicit string writes, `±9,999,999,999,999` cent numeric threshold with `"Exact text"` fallback beyond it, atomic publish, and `calamine` readback tests.

### `1.3.0` Native Appearance (§14.2 — Implemented, Solid/Tint + Wallpaper)
- **Preferences & Schema 5:** Single-row `0005_appearance.sql`, `AppearancePrefs`, `ApplyAppearance` / `ResetAppearance` Store actions, `RefreshScope::Appearance`, and resilient fallback to legacy `profile.theme` on missing/corrupt rows.
- **Windows Platform Preferences:** `src/infrastructure/platform/windows_prefs.rs` (`windows-sys`, scoped `#[allow(unsafe_code)]`) reads reduced motion, high contrast (plus system colors), and transparency. Startup and focus/resume refresh immediately. While focused and visible, a worker checks every two seconds and requests a frame only when preferences change.
- **Managed Media & Playback:** Bounded PNG/JPEG/GIF decoder (`image 0.25.10`; <=16 MiB source, <=2048 px per axis, <=300 frames, 128 MiB combined active+preview RGBA budget), managed `appearance/` directory (32-hex `<id>.media`, symlink rejection, 256 MiB / 32-file cap), 30 Hz playback clock, reduced-motion poster, pause/blur/minimize suspension, and `Order::Background` scrim compositor (`>=4.5:1` text / `>=3:1` focus/icon over black & white).
- **Settings → Appearance & Format 4:** Live shell preview with Apply/Cancel/Reset and dirty-leave guard, built-in Aurora gradient + local Fit/Fill wallpaper, `format_v4.rs` (`DocumentV4`) with portable `local -> none` export semantics, and LF fixture [format4-1.3.0.json](../tests/fixtures/format4-1.3.0.json).
- **Production Glass Status:** **Unselectable.** The preference timer no longer schedules a UI frame. Glass still requires hardware swapchain p95, a measured five-second focused-idle sample, native DPI, iGPU/dGPU and device-loss evidence, followed by a contract revision. The local spike run could not acquire a `wgpu` device. See the [spike](design/athena-glass-spike.md) and [1.3 gate record](smoke-1.3.md#open-glass-gates).

### `1.4.0` Goals & Closings (§14.3 — Implemented)
- **Goals & Schedule (`0006_goals.sql`):** Goal lifecycle (`Active`, `Paused`, `Completed`, `Archived` with stored `pre_archive_status`), checked contribution accumulation, derived `Funded` (`contributed >= target`), active-attribution rule for new/changed goals, `contribution_plan` schedule arithmetic, and full ledger/settlement independence.
- **Goals UI (`Ctrl+5`):** `Screen::Goals` (`src/ui/screens/goals.rs`), `GoalsState` / `GoalDetail` cached read models, two-pane >=720 pt layout and narrow drill-in/back stack, contribution history, validated modals/confirmations, and DST-aware local-midnight refresh.
- **Monthly Closings & Staleness (`0007_closings.sql`):** Single-transaction capture of completed months (`month < YearMonth::from_date(today)`), `sha2` version-1 canonical comparison fingerprints for both the closed month and its predecessor, `CloseMonth` / `RecalculateClosing` actions, distinct Month `Saved closing` card, and interactive Calendar saved/changed markers.
- **Format 5 & Full-History Workbook Sheets:** `format_v5.rs` (`DocumentV5`), LF fixture [format5-1.4.0.json](../tests/fixtures/format5-1.4.0.json), missing live largest-expense FK projected to `null` on export, and localized full-history `Goals` and `Closings` XLSX sheets.

## Automated Verification & Release Candidate State

Verified on Windows 11 Pro with Rust 1.95.0:
- `bash docs/superpowers/doc-lint.sh`: clean
- `cargo fmt --check`: clean
- `cargo clippy --all-targets --locked -- -D warnings`: clean
- `cargo test --locked --all-targets -- --skip real_wgpu_`: **772 passed, 0 failed, 4 ignored, 1 filtered out** (`753` library, `1` `design_assets`, `5` `fonts`, `2` `no_transactions_table`, `2` `packaging_metadata`, `9` `glass_spike`)
- `cargo build --release --locked` and local Inno Setup 6.7.3 compilation passed. The [1.4 smoke record](smoke-1.4.md#local-release-candidate) lists the local portable, installer and license hashes.
- The local portable build reached a responsive window and created a clean schema-7 database.
  Default renderer selection now excludes OpenGL unless `WGPU_BACKEND` asks for it, and exit stops
  the background workers and frees the wallpaper textures first. On the GeForce GT 340 machine,
  10 of 10 default-renderer `WM_CLOSE` trials exited 0 (264 to 1,704 ms). See the
  [portable smoke record](smoke-1.4.md#local-portable-smoke).
- `git diff --check`: clean

## Open Release & Manual Gates

The roadmap's named automated regressions are present. Its historical checklists include mixed
engineering and release tasks. The smoke records track completion for each release separately.

See [smoke-1.4.md](smoke-1.4.md) (and open carry-over items in [smoke-1.1.md](smoke-1.1.md), [smoke-1.2.md](smoke-1.2.md), and [smoke-1.3.md](smoke-1.3.md)):
1. Hosted Windows CI run on the candidate commit and CI-built portable/installer artifact hashes (`Athena-1.4.0-setup.exe`, `athena-1.4.0-windows-x64.exe`, license notices, `BUILD-INFO.txt`, `SHA256SUMS.txt`).
2. Real historical profile upgrades (`1.0.2` / `1.1.x` / `1.2.0` / `1.3.0` -> schema `7`), live backup/restore, and JSON/XLSX checks in external apps.
3. Native interactive window walkthroughs (`Ctrl+1`–`5`, `640x410` narrow viewport, `100/125/150/200%` Windows DPI scaling, EN & PT-BR, Dark & Light, live GIF across all main screens, reference-device memory/frame-p95 measurements).
4. Production Glass hardware/DPI/focused-idle gates before any future Glass opt-in contract.
