# Athena Phase 0 — finish 1.0.2 and prepare for 1.1

> **For agentic workers:** Execute tasks in priority order. Each task lists its contract references, owning files, non-goals, tests to write first, and the standard verification footer ([contracts §13](../specs/contracts.md#13-verification--frozen-for-11)). Steps use checkbox (`- [ ]`) syntax. Tagging, pushing, and publishing are **user actions**; stop and report when a step says so.

**Goal:** Close the 1.0.2 release with evidence bound to its final artifacts, add CI, retire the layered planning documents, and make the large UI modules safe to extend. None of this changes user-visible behavior, except a narrow-layout fix if Task P0-1's test reproduces the bug.

**Contracts:** [contracts.md](../specs/contracts.md). **Next plan:** [1.1.0 Safe data](2026-11-16-athena-1.1.0-safe-data.md).

## Priority list

| Priority | Task | Blocks |
|---|---|---|
| P0 | P0-1 Narrow-layout regression sweep | 1.0.2 artifacts |
| P0 | P0-2 Final 1.0.2 artifacts and smoke | v1.0.2 tag (user) |
| P1 | P0-3 Windows CI | every 1.1 task's "CI green" |
| P1 | P0-4 Documentation consolidation and doc-lint | 1.1 plan execution |
| P1 | P0-5 Module split (Settings pages, app handlers) | 1.1 Tasks 5, 8, 10 |

P0-1 → P0-2 are sequential and come first: any code change after P0-2 invalidates its hashes. P0-3 and P0-4 can run in parallel with them. P0-5 starts after the v1.0.2 tag.

---

### Task P0-1: Narrow-layout regression sweep

**Priority:** P0. **Files:** `src/ui/screens/month.rs`, `src/ui/components/transaction_row.rs`.
**Non-goals:** no visual redesign; no snapshot changes unless the fix requires them.

The 1.0.2 fix wrapped right-aligned blocks in `ui.horizontal` so they stop taking the scroll area's full height. `rail_row` (`month.rs`, narrow branch) still uses `Layout::right_to_left(Align::Min)` directly inside `ui.vertical`.

- [ ] **Step 1: Write the failing test.** Add `rail_rows_stay_compact_in_a_scrolling_rail` in `month.rs` tests, modeled on `narrow_fallback_rows_stay_compact_in_a_scrolling_register`:
  - render `rail_row` inside `ScrollArea::vertical` at widths 120 px and 200 px, with `i64::MAX` cents and a long category name, in both languages
  - follow it with a `"Following row"` label
  - assert the row height is bounded by `2 × line height + bar + spacing` (compute this from the style; don't hard-code it), and that the following label is visible
- [ ] **Step 2:** Run `cargo test rail_rows_stay_compact`.
  - If it **passes**, record "not reproduced" in the commit message and skip Step 3.
  - If it **fails**, continue.
- [ ] **Step 3: Fix** by wrapping the narrow-branch `with_layout(right_to_left…)` in `ui.horizontal(…)`, exactly as in `transaction_row.rs:271-281`. Re-run.
- [ ] **Step 4: Replace the fixed threshold.** In `transaction_row.rs`, change the `row.height() <= 220.0` assertion so it is derived from the style: `3 × interact_size.y + 2 × wrapped amount line height + spacing`, plus a 15 % margin. Keep the "following row visible" assertion as the primary check, and add a comment explaining the bound.
- [ ] **Step 5: Add a repo rule.** Add to the `transaction_row.rs` module docs: *"Right-aligned content inside a vertical layout must sit in `ui.horizontal`; a bare `right_to_left` layout takes all remaining height."*
- [ ] **Step 6:** Standard verification. If any snapshot changed, inspect it.
- [ ] **Step 7: Commit:** `fix: keep narrow rail rows compact in scrolling layouts` (or `test: …` if not reproduced).

### Task P0-2: Final 1.0.2 artifacts and smoke

**Priority:** P0. **Files:** `docs/smoke-1.0.md` only. **Non-goals:** no code changes. Any code change restarts this task.

- [ ] **Step 1:** `cargo build --release --locked`, then compile `installer/athena.iss` with Inno Setup 6.7.3.
- [ ] **Step 2:** Record the SHA-256 and byte size of `target\release\athena.exe` and `Athena-1.0.2-setup.exe` in a new subsection, "1.0.2 final artifacts (YYYY-MM-DD)". Mark the earlier hash block as superseded.
- [ ] **Step 3: Run the open checklist items against these hashes:**
  - silent install / launch / uninstall in an isolated directory
  - portable launch (`athena.portable`) with the log line `athena 1.0.2 starting`
  - interactive walk-through of Month, Calendar and Settings in both languages and themes
  - a real 200 % Windows scaling window check
  - a clean-machine check if one is available; otherwise leave it unchecked and say so
- [ ] **Step 4:** Tick only the items performed, each naming the hash it ran against. Leave unperformed items unchecked, with a reason.
- [ ] **Step 5: Commit:** `docs: record final 1.0.2 artifact verification`.
- [ ] **Step 6: Stop.** Report the hashes and the checklist state to the user. Tagging `v1.0.2` and pushing are user actions.

### Task P0-3: Windows CI

**Priority:** P1. **Files:** create `.github/workflows/ci.yml`; README "Development" section.
**Non-goals:** no release publishing and no secrets; tag builds attach artifacts to the workflow run only.

- [ ] **Step 1: Create a `check` job** on `windows-latest`, triggered by push and pull request:
  - toolchain from `rust-version` (1.95) with `rustfmt` and `clippy`
  - `Swatinem/rust-cache`
  - `cargo fmt --check`
  - `cargo clippy --all-targets --locked -- -D warnings`
  - `cargo test --locked`
  - `bash docs/superpowers/doc-lint.sh`
- [ ] **Step 2: Check snapshot stability on CI.** The kittest snapshots render with wgpu, and CI uses a software adapter.
  - If snapshot tests fail only because of renderer differences, gate them behind `ATHENA_SNAPSHOTS=1` (set locally and in the release checklist), and keep every non-snapshot UI test running on CI.
  - Record the decision and the reason in the README.
  - Do **not** regenerate goldens on CI.
- [ ] **Step 3: Create a `package` job** on `v*` tags:
  - `cargo build --release --locked`
  - install Inno Setup (`choco install innosetup --version 6.7.3 -y`) and compile the installer
  - write `SHA256SUMS.txt` (exe, installer, `assets/fonts/LICENSE.txt`)
  - upload all three as workflow artifacts
- [ ] **Step 4:** Push a branch (user approval) and confirm both paths are green, or document why snapshots are gated.
- [ ] **Step 5: Commit:** `ci: add Windows check and package workflows`.

### Task P0-4: Documentation consolidation and doc-lint

**Priority:** P1. **Files:**
- `docs/superpowers/plans/` (move the 2026-11-11 1.1/1.2 plans to `historical/`)
- the revision briefs and roadmap revision (relative links)
- `docs/superpowers/doc-lint.sh`
- new `docs/superpowers/plans/README.md`
- `docs/design/athena-designer-brief.md`
- `docs/dod.md`

**Non-goals:** no rewriting of historical content beyond a superseded banner and link fixes.

- [ ] **Step 1: Move the old plans.** `git mv` the 2026-11-11 1.1.0 and 1.2.0 plans into `docs/superpowers/plans/historical/`. Add a one-line banner to each: `> **Superseded by [contracts](../../specs/contracts.md) and the 2026-11-16 plans. Historical reference only; do not execute.**`
- [ ] **Step 2: Fix relative links** in the briefs and roadmap revision that point at moved files. Add the contracts document and the new plans to the roadmap revision's document list.
- [ ] **Step 3: Write `plans/README.md`:** a table of active plans (Phase 0, 1.1) and historical plans, with one line each, plus the rule *"Tasks reference contracts.md; they do not restate it."*
- [ ] **Step 4: Update `doc-lint.sh`.**
  - Exclude `plans/historical/` from every check.
  - Add these checks (each existing legitimate mention gets `lint:allow`):
    - `previous_net` <!-- lint:allow -->
    - `is_complete` <!-- lint:allow -->
    - `CategoryTotal\.name` <!-- lint:allow -->
    - `Additive changes keep this at 1` <!-- lint:allow -->
    - `Option<Db>` <!-- lint:allow -->
    - `person_id` on a line that also contains `template` <!-- lint:allow -->
    - `egui::Image` on a line that also contains `wallpaper` <!-- lint:allow -->
  - Add a check that every relative Markdown link under `docs/` resolves to an existing file.
- [ ] **Step 5: Refresh the designer brief.** Replace the pre-build and fallback-font language with the shipped Polaris contract (Semibold body, Bold titles, tabular Semibold money). Keep it under 4,000 characters (`wc -m`), and update its mirrored appendix in the design spec in the same commit.
- [ ] **Step 6: Update `dod.md`.** Add:
  - "Task cites its contracts.md section"
  - "CI green"
  - "release checks bound to artifact hashes"
- [ ] **Step 7:** `bash docs/superpowers/doc-lint.sh` is clean.
- [ ] **Step 8: Commit:** `docs: consolidate plans behind contracts.md and extend doc-lint`.

### Task P0-5: Module split (no behavior change)

**Priority:** P1, after the v1.0.2 tag. **Files:**
- `src/ui/screens/settings.rs` → `src/ui/screens/settings/{mod.rs, general.rs, people.rs, categories.rs, data.rs}`
- `src/app/mod.rs` → extract `src/app/files.rs` (export path picking, `run_export`, `open_folder`) and `src/app/modals.rs` (`delete_confirmation`, `apply_confirmed`, `ModalAfter` handling)

**Non-goals:** no new features and no copy changes. Snapshots must be byte-identical.

- [ ] **Step 1:** Run the full suite and note the test count (baseline 271 + P0-1).
- [ ] **Step 2: Split Settings into page modules.**
  - Move the cards (`profile_card`, `people_card`, `categories_card`, the export/data-folder section) into their modules unchanged.
  - `settings::show` keeps rendering all cards in the current single-scroll order.
  - Add the `SettingsPage { General, People, Categories, Data }` enum, but no page selector yet.
  - **Snapshot policy:** the selector and the page-based layout arrive in 1.1 Task 8, together with the Data & backups content, so snapshots change once, deliberately.
- [ ] **Step 3: Extract the app handlers** into `files.rs` and `modals.rs` with the same signatures, re-exported from `app/mod.rs` so tests compile unchanged.
- [ ] **Step 4:** Run the suite. The same test count passes, and `git diff --stat src/ui/snapshots` is empty.
- [ ] **Step 5: Commit:** `refactor: split settings and app handlers into modules`.

---

## Phase 0 exit criteria

- `v1.0.2` is tagged by the user against hashes recorded in `smoke-1.0.md`.
- CI is green on `main`.
- Old 1.1/1.2 plans are in `historical/`; doc-lint is clean and checks links.
- The Settings and app handlers are split, with identical snapshots.
