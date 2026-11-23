# Athena 1.4.0 release smoke record

Scope includes Goals and contribution history, saved monthly closings and Calendar markers,
schema 7, document format 5, and full-history Goals and Closings workbook sheets. See the
[1.4 roadmap](superpowers/plans/2026-11-18-athena-stalled-ui-roadmap-plan.md#140-goals-and-closings),
[contracts](superpowers/specs/contracts.md) (§13 and §14.1–§14.3), and the [definition of done](dod.md).

Status **candidate**. The local artifacts below were built on the developer machine. Hosted CI
artifacts and final hashes are pending. Repeat native checks against the exact final artifacts.

## Local release candidate

Rebuilt on 2026-11-23 from `4365b78` plus the working-tree fixes for the six bug-review
findings (renderer backend selection and shutdown, narrow Goals focus, platform watcher locking,
stale closing link after purge, and superseded smoke notes), on Windows 11 Pro build 26200 with
Rust 1.95.0 and a local Inno Setup 6 `ISCC.exe`.

| Local artifact | Bytes | SHA-256 |
|---|---:|---|
| `athena-1.4.0-windows-x64.exe` (`target/release/athena.exe`) | 22,241,792 | `ea703e80694e5871fa95970a1a3dda27022ae4832a9ded884a4e63685cb0c54f` |
| `Athena-1.4.0-setup.exe` | 8,508,771 | `d4f0ae82b095a43b5186c823322ce3e4f9216bb4df4cea83a1c77846e52bcd16` |
| `LICENSE.txt` | 3,274 | `931886d07182110715ccee3504a076b59929503e4ebacadbe0da789184850a61` |
| `ICONS-LICENSE.txt` | 880 | `1e7290b35280a048667bbf0ebabac1c7fd52a75300e8b2946ac165715997f2bc` |

## Automated checks

- [x] `bash docs/superpowers/doc-lint.sh` exited 0 and printed `doc-lint: clean`.
- [x] `cargo fmt --check` exited 0 with no output.
- [x] `cargo clippy --all-targets --locked -- -D warnings` exited 0.
- [x] `cargo test --locked --all-targets -- --skip real_wgpu_` exited 0 with 772 passed, 0 failed,
  4 ignored, 0 measured, and 1 filtered out. The ignored cases are two fixture regenerators and
  two 50,000-row benchmarks. The filter excludes the example's real-GPU case.
- [x] `cargo build --release --locked` and Inno Setup compilation of `installer/athena.iss`
  exited 0. Both executable products report version 1.4.0. The two packaging metadata tests
  passed, covering installer version alignment and CI artifact names and provenance.
- [x] `git diff --check` exited 0 with no whitespace errors. Git printed LF-to-CRLF conversion
  notices for modified text files outside the fixture policy.

| Test target | Passed | Failed | Ignored | Measured | Filtered out | Time |
|---|---:|---:|---:|---:|---:|---:|
| Library | 753 | 0 | 4 | 0 | 0 | 50.32 s |
| Main executable | 0 | 0 | 0 | 0 | 0 | 0.00 s |
| `design_assets` | 1 | 0 | 0 | 0 | 0 | 0.00 s |
| `fonts` | 5 | 0 | 0 | 0 | 0 | 0.01 s |
| `no_transactions_table` | 2 | 0 | 0 | 0 | 0 | 0.09 s |
| `packaging_metadata` | 2 | 0 | 0 | 0 | 0 | 0.00 s |
| `glass_spike` example | 9 | 0 | 0 | 0 | 1 | 0.30 s |

- [x] The UI regression observed no preference repaint deadline in a settled focused frame.
  The worker tests verified changed-sample notification, suspension while hidden, focus-regain
  reads on the worker, a provider read that blocks without blocking `current()` or
  `update_window()`, and a read that finishes after stop being discarded. The native
  five-second idle gate remains open.
- [x] Synthetic upgrades from every schema 1–6 reached schema 7. The file-backed 5/6
  regressions preserved appearance and existing goal contributions. Restore upgraded a schema-1
  backup and refused a newer schema.
- [x] Explicit version dispatch and fixture round-trips covered formats 1–5. All fixtures kept
  their original bytes and use LF separators. Formats 1–4 loaded with empty goals and closings.
  Format-5 fault injection preserved the existing financial, template, appearance, goal and
  closing data.
- [x] The live-transaction read guard, goal lifecycle/overflow checks, ledger independence,
  closing fingerprint and atomic replacement checks, and both workbook scopes passed. Workbook
  readback covered localized history sheets and exact numeric and text amounts.
- [x] The UI harness exercised Goals at all four app text scales with Reduced motion. It also
  exercised keyboard activation of the Calendar day-grid marker and closing recalculation in
  Português at 150% app text scale with the built-in wallpaper.
- [x] Designer brief and Appendix A were byte-identical. `design_assets` passed at 3,993
  characters, including the final LF.

## Local portable smoke

- The listed portable executable opened a responsive Athena window from a disposable directory
  with `athena.portable` and both license notices. With `WGPU_BACKEND=dx12` it used the Microsoft
  Basic Render Driver and closed with exit code 0. The portable database reported schema 7,
  `PRAGMA integrity_check` returned `ok`, and `PRAGMA foreign_key_check` returned no rows.
- On the GeForce GT 340 machine, the previous record found shutdown intermittent with default
  renderer selection and with `WGPU_BACKEND=gl`: the window became unresponsive after `WM_CLOSE`
  and the harness stopped the process. It also timed out in a matched trial of a release build
  from `4bfa8bd`.
- Cause and fix. egui's default wgpu setup allows `PRIMARY | GL`. On this pre-feature-level-11
  GPU, DX12 resolves to the Microsoft software adapter (`DeviceType::Cpu`), which wgpu ranks below
  the WGL adapter, so the default window ran on WGL. Athena now builds the wgpu options with
  `WGPU_BACKEND` when set and `PRIMARY` (no GL) otherwise (`renderer_backends` in `src/lib.rs`),
  and `on_exit` stops and joins the platform watcher and the media worker and drops the wallpaper
  textures before the renderer is torn down (`AthenaApp::shutdown`). `run_and_return` was left at
  its default so the instance lock and log guard still drop on return.
- Re-measured on 2026-11-23 with the rebuilt `target/release/athena.exe` (SHA-256 above), copied
  into a disposable directory with `athena.portable`, on the GeForce GT 340 machine, with no
  `WGPU_BACKEND` set: 10 launches, each waited for a window and four more seconds, confirmed the
  window responding, then sent `WM_CLOSE` (`CloseMainWindow`). All 10 exited with code 0 in
  264 to 1,704 ms (nine in 264 to 440 ms). With `WGPU_BACKEND=gl` set, 3 of 3 launches also
  exited 0 in 178 to 378 ms. Ten and three trials are a measurement, not proof that the
  intermittent hang is gone; the supported-hardware shutdown check below stays open.

## Snapshot review

- [x] Regenerated all 48 goldens to a scratch directory and compared them pixel by pixel.
  Updated the 25 with stale 1.3.0 footers. The affected Portuguese light goldens also show
  the current Settings item without its overlapping shortcut hint. Reviewed both changes.
- [x] Inspected the Goals empty, split-pane, and narrow detail views in English/Dark and
  Português/Light for notes, exact amounts, Funded labels, and action placement.
- [x] Inspected the Month and Calendar goldens in both variants. The Month card keeps the
  saved comparison separate from the live insight and adds an icon and text when stale. The
  Calendar year view shows the marker on the month tile, and the day-grid header spells out its
  status with a button that opens Month. The year grid scrolls to the bottom row at its default
  snapshot height.

## Open Windows and release checks

- [ ] Windows CI green for the candidate commit. Record the CI run, commit/tag, dependency and
  license notices, bytes, and SHA-256 for `athena-1.4.0-windows-x64.exe`,
  `Athena-1.4.0-setup.exe`, `LICENSE.txt`, `ICONS-LICENSE.txt`, `BUILD-INFO.txt`, and
  `SHA256SUMS.txt`.
- [ ] Real historical upgrade through schema 7, including a real 1.0.2 profile and applicable
  1.1.x, 1.2.0 and 1.3.0 profiles. Restore an older backup and verify newer-schema refusal in the
  installed and portable applications. Import formats 1–5 into a disposable profile in the real app.
- [ ] Use Ctrl+1 through Ctrl+5 in the native window. Walk through the goal lifecycle, below-target
  completion and reopening, contribution entry and deletion, focus return, archived attribution,
  long notes, and amounts at the precision boundaries.
- [ ] Close a completed month, edit both comparison months, check Calendar stale markers and
  navigation to Month, and confirm saved figures stay fixed until Recalculate is accepted.
- [ ] Open both workbook scopes in a spreadsheet application and check the full-history sheets,
  archived names, and exact amounts.
- [ ] Exercise both languages and themes at 100%, 125%, 150%, and 200% Windows display scaling,
  including a narrow 640 × 410 window, maximum app text scale, reduced motion, and keyboard-only
  focus and confirmations.
- [ ] Verify native wallpaper continuity through Goals and the other screens, with pause, blur,
  minimize, and resume. Record repeated media operations, memory, texture writes, and frame p95
  on reference Windows hardware. The [1.3 hardware and Glass checks](smoke-1.3.md) remain open.
- [ ] Run installer install, launch, and uninstall on a clean Windows machine. Launch the portable
  executable with `athena.portable` and verify both distributions include the Lucide and Polaris
  notices.
- [ ] Verify normal default-renderer shutdown on the supported Windows hardware beyond the 10
  trials recorded above, and with the installed build, including a wallpaper and an open modal.
- [ ] Carry forward outstanding interactive checks from the [1.1 record](smoke-1.1.md), the
  [1.2 record](smoke-1.2.md), and the [1.3 record](smoke-1.3.md).
