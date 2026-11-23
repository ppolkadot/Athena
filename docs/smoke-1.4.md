# Athena 1.4.0 release smoke record

Scope includes Goals and contribution history, saved monthly closings and Calendar markers,
schema 7, document format 5, and full-history Goals and Closings workbook sheets. See the
[1.4 roadmap](superpowers/plans/2026-11-18-athena-stalled-ui-roadmap-plan.md#140-goals-and-closings),
[contracts](superpowers/specs/contracts.md) (§13 and §14.1–§14.3), and the [definition of done](dod.md).

Status **candidate**. The local artifacts below were built on the developer machine. Hosted CI
artifacts and final hashes are pending. Repeat native checks against the exact final artifacts.

## Local release candidate

Built from the 1.4 continuation on top of `4bfa8bd`, on Windows 11 Pro build 26200 with
Rust 1.95.0 and the CI-pinned, SHA-256-verified Inno Setup 6.7.3 compiler.

| Local artifact | Bytes | SHA-256 |
|---|---:|---|
| `athena-1.4.0-windows-x64.exe` | 22,237,696 | `0b7f89c931961ed8a62d6167b14f6edeee311f4e58af99a1f8a4b10c70b8c04d` |
| `Athena-1.4.0-setup.exe` | 8,506,765 | `7e74f7e9214ac2b3e4214981cab783ee192b067df8ea944b68fbe072736ca0b1` |
| `LICENSE.txt` | 3,274 | `931886d07182110715ccee3504a076b59929503e4ebacadbe0da789184850a61` |
| `ICONS-LICENSE.txt` | 880 | `1e7290b35280a048667bbf0ebabac1c7fd52a75300e8b2946ac165715997f2bc` |

## Automated checks

- [x] `bash docs/superpowers/doc-lint.sh` exited 0 and printed `doc-lint: clean`.
- [x] `cargo fmt --check` exited 0 with no output.
- [x] `cargo clippy --all-targets --locked -- -D warnings` exited 0.
- [x] `cargo test --locked --all-targets -- --skip real_wgpu_` exited 0 with 762 passed, 0 failed,
  4 ignored, 0 measured, and 1 filtered out. The ignored cases are two fixture regenerators and
  two 50,000-row benchmarks. The filter excludes the example's real-GPU case.
- [x] `cargo build --release --locked` and Inno Setup compilation of `installer/athena.iss`
  exited 0. Both executable products report version 1.4.0. The two packaging metadata tests
  passed, covering installer version alignment and CI artifact names and provenance.
- [x] `git diff --check` exited 0 with no whitespace errors. Git printed LF-to-CRLF conversion
  notices for modified text files outside the fixture policy.

| Test target | Passed | Failed | Ignored | Measured | Filtered out | Time |
|---|---:|---:|---:|---:|---:|---:|
| Library | 743 | 0 | 4 | 0 | 0 | 66.58 s |
| Main executable | 0 | 0 | 0 | 0 | 0 | 0.00 s |
| `design_assets` | 1 | 0 | 0 | 0 | 0 | 0.00 s |
| `fonts` | 5 | 0 | 0 | 0 | 0 | 0.01 s |
| `no_transactions_table` | 2 | 0 | 0 | 0 | 0 | 0.10 s |
| `packaging_metadata` | 2 | 0 | 0 | 0 | 0 | 0.00 s |
| `glass_spike` example | 9 | 0 | 0 | 0 | 1 | 0.28 s |

- [x] The UI regression observed no preference repaint deadline in a settled focused frame.
  The worker test verified changed-sample notification and suspension while hidden. The native
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
- On the GeForce GT 340 machine, shutdown was intermittent with default renderer selection
  and with `WGPU_BACKEND=gl`. In failed trials the window became unresponsive after `WM_CLOSE`
  and the harness stopped the process. The default selection also timed out in a matched trial
  of a release build from `4bfa8bd`. The cause remains open.

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
- [ ] Investigate the intermittent default-renderer shutdown timeout and verify normal shutdown
  on the supported Windows hardware.
- [ ] Carry forward outstanding interactive checks from the [1.1 record](smoke-1.1.md), the
  [1.2 record](smoke-1.2.md), and the [1.3 record](smoke-1.3.md).
