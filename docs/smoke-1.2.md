# Athena 1.2.0 release smoke record

Scope: Daily Speed (schema 4, document format 3): templates, one-line quick entry, duplicate
consent, day drill-down, monthly insight, XLSX export, design tokens and the embedded icon set.
Contract: `docs/superpowers/specs/contracts.md` §14.1 and §13. Plan:
`docs/superpowers/plans/2026-11-18-athena-1.2.0-daily-speed.md`.

Status: **candidate, not released.** Nothing below was run against a tagged, CI-built or installed
artifact. Per `docs/dod.md`, a manual check is never marked done unless it was executed against the
recorded final artifacts; any tag or rebuild that changes the hashes invalidates this record.

## Local release candidate

| Item | Value |
|---|---|
| Source | commit after `f480930` on `main` (version bump commit) |
| Local build | `cargo build --release --locked`, `target/release/athena.exe` |
| Local build size | 20,717,568 bytes |
| Local build SHA-256 | `20eaec5750ca22a3152e8b92a36b4d98db0eaec45591aaeca9793496fa19784d` |

This is a developer-machine build, not a release artifact. The published artifacts are produced by
the tag CI (`athena-1.2.0-windows-x64.exe`, `Athena-1.2.0-setup.exe`, `LICENSE.txt`,
`ICONS-LICENSE.txt`, `BUILD-INFO.txt`, `SHA256SUMS.txt`); their bytes and SHA-256 hashes are **not
yet recorded** and must be added here before any manual check below is ticked.

## Performed (automated, local, Windows 11, Rust 1.95)

- [x] `bash docs/superpowers/doc-lint.sh`: clean.
- [x] `cargo fmt --check`: clean.
- [x] `cargo clippy --all-targets --locked -- -D warnings`: clean.
- [x] `cargo test --locked --all-targets`: 562 library tests passed (3 ignored), plus integration
  tests `design_assets` 1, `fonts` 5, `no_transactions_table` 2, `packaging_metadata` 2 and the
  `glass_spike` example 10; 0 failed.
- [x] `cargo build --release --locked`: succeeds.
- [x] `git diff --check`: clean.
- [x] Named regressions of the roadmap exist and pass: contrast and icon tests, migration
  `0004_templates`, format 3 round-trip and explicit dispatch (`format1-1.0.2.json`,
  `format2-1.1.0.json`, `format3-1.2.0.json`), template, dirty-draft, one-line, duplicate-consent,
  day drill-down (`app::day_flow_tests`, view-model day-cache tests), insight and XLSX tests
  (`infrastructure::export::xlsx::tests`, read back with `calamine`).
- [x] Changed or new EN/Dark and PT-BR/Light snapshots were inspected for clipping and amounts:
  Month with day grid and detail (wide and 640 px), Calendar with Days actions, Calendar day grid,
  and Settings Data with the Excel buttons.

## Not performed (no result is claimed)

These need a live Windows window or the final artifacts and remain **open**:

- [ ] Windows CI green on the release candidate commit; tag build; bytes and SHA-256 of the final
  portable executable, installer, `LICENSE.txt`, `ICONS-LICENSE.txt`, `BUILD-INFO.txt` and
  `SHA256SUMS.txt` recorded here.
- [ ] Real `1.0.2` data folder upgraded v1 to v4 (installed and portable); restore of an older
  backup; import of the three shipped formats in the real window.
- [ ] Real template-assisted repeated entry, dirty-draft discard and duplicate consent walk-through.
- [ ] Salary-only and payback-only days discoverable in the real Calendar and Month grids with a
  mouse and keyboard only (Tab, arrows, Enter/Space, Escape focus return).
- [ ] Both XLSX scopes (viewed month, all history) opened in a spreadsheet application, including
  text starting with `=`, `+`, `-` or `@`, and an amount beyond 13 digits of cents.
- [ ] Both languages and themes; 100/125/150/200 % Windows scaling; narrow (640 x 410) window;
  long text and maximum amounts; keyboard-only pass in the real window.
- [ ] Installer install, launch and uninstall of `Athena-1.2.0-setup.exe`; clean Windows machine;
  portable launch with `athena.portable`; the Lucide and Polaris license notices present in both.
- [ ] Carried over from 1.1.x and still open: the interactive items listed in `docs/smoke-1.1.md`.

These remain open and are not waived by this record.

The 1.4 candidate aligns the native client-area minimum with the 640 × 410 logical-point
Month and Calendar harness. The native walkthrough on final artifacts remains open.
