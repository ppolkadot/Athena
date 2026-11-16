# Athena 1.1.0 — Review Assessment & Improvement Plan (Google Hydrogen)

> **For agentic workers:** Execute Gate 0 and Finish-Task-6 remediation tasks in order before running on any copy of real user data or starting Task 7. Each remediation task names its contract sections, owning files, non-goals, tests to write **before** implementation, and ends with the standard verification footer ([contracts §13](../specs/contracts.md#13-verification--frozen-for-11)) and a commit. Canonical Tasks 7–13 remain owned by [2026-11-16-athena-1.1.0-safe-data.md](2026-11-16-athena-1.1.0-safe-data.md); this plan supplements them without creating conflicting DTOs or pipelines. Frozen contract edits marked **Proposed** require user approval and must be applied to [contracts.md](../specs/contracts.md) with a `§15` change-log entry in the implementing commit.

**Goal:** Resolve safety, reliability, filter-logic, UX, and documentation defects identified across the Codex review, Claude Opus 5.5 independent analysis, synthesis assessment, and Hydrogen source verification on `5ba7411`, and integrate them cleanly into the remaining Athena 1.1.0 execution path (Tasks 7–13).

**Architecture:** Preserve Athena's layered architecture (`domain` pure logic, `infrastructure` SQLite/file I/O, `application` use cases, `app` store/view-model/session state, `ui` pure egui views) and the [D9 release split](../specs/contracts.md) (1.1 Safe Data, 1.2 Daily Speed, 1.3 Appearance, 1.4 Goals & Closings).

**Tech Stack:** Rust (`edition = 2024`, `rust-version = 1.95`), `rusqlite` (bundled SQLite + `backup` in Task 8), `eframe`/`egui` + `egui_kittest`, `chrono`, `proptest`, Windows CI.

**Plan status:** Review proposal only; implementation has not been authorized or started. D-P1 through D-P7 are recommendations, not approved decisions. Codex checked and corrected the execution details listed in section 6.

**Baseline:** Commit `5ba7411` (Phase 0 commits `c9876f1..25b5bb8`; Safe Data 1.1 Tasks 1–6 commits `fcaa837..5ba7411`). Local tags present: `v1.0.0`, `v1.0.1` (`v1.0.2` is not tagged locally or on `origin`). Untracked review inputs: [2026-11-16-codex-handoff-opus-5.5.md](../../reviews/2026-11-16-codex-handoff-opus-5.5.md), [2026-11-16-opus-5.5-analysis.md](../../reviews/2026-11-16-opus-5.5-analysis.md), [2026-11-16-synthesis-hydrogen-handoff.md](../../reviews/2026-11-16-synthesis-hydrogen-handoff.md), [2026-11-16-review-probes.rs.txt](../../reviews/2026-11-16-review-probes.rs.txt).

---

## 1. Independent Judgment & Consensus/Disagreement Ledger

After inspecting the source at `5ba7411` (`backup.rs`, `store.rs`, `view_model.rs`, `ui_state.rs`, `app/mod.rs`, `app/modals.rs`, `month.rs`, `settlement_card.rs`, `settlement.rs`, `settlement_repo.rs`, `reports.rs`, `reports_repo.rs`, `transactions_repo.rs`, `trash_repo.rs`, `export/json.rs`, `export/csv.rs`, `i18n/*`, `migrations.rs`, and `docs/superpowers/*`), Hydrogen's verdict is:

### 1.1 Findings Ledger (Codex vs. Opus vs. Hydrogen)

| ID | Topic | Codex Verdict | Opus Verdict | Hydrogen Verdict | Rationale & Source Evidence |
|---|---|---|---|---|---|
| **C-1 / O-1** | Startup trash retention trusts unvalidated `auto-YYYY-MM-DD.db` | **Confirm P1** (reproduced 0-byte/corrupt acceptance) | **Confirm P1 & Expand** (reproduced valid foreign SQLite acceptance) | **Accept P1, Qualify Fix** | `src/infrastructure/db/backup.rs:85-87` returns `Ok(None)` on `if path.exists()` (not even `is_file()`, so a directory also returns `Ok(None)`), and `src/app/store.rs:54-60` purges expired trash on `Ok(None)`. **Qualification of Opus's fix:** `integrity_ok` alone accepts foreign SQLite DBs; however, Opus's requirement that the snapshot contain exact deleted row IDs with `deleted_at IS NOT NULL` at exact `latest_version()` is over-strict (a same-day snapshot written earlier can hold those rows still live, older supported Athena schemas are valid, and IDs alone do not verify allocations). Require a verified Athena snapshot (`is_file()`, readable SQLite, `PRAGMA integrity_check == "ok"`, Athena identity, supported `user_version`) OR write a fresh verified pre-purge snapshot before purging any non-empty expired set; fail closed on any error. |
| **C-2** | `UiState::set_repair_filter` keeps conflicting filters & search debounce | **Confirm P2** (reproduced with `Shared` + `Unassigned`) | **Confirm & Expand** (noted pending `search_edited_at` debounce) | **Accept P2** | `src/app/ui_state.rs:71-75` sets only `month`, `until`, `issue`. `transactions_repo.rs:130-163` ANDs `person_id` / `shared_only` (`COUNT(alloc) >= 2`), which makes `SettlementIssue::Unassigned` (`NOT EXISTS alloc`) mathematically empty, while `category_id`, `kind`, `text`, and `search_edited_at` (committed 0.2s later by `month.rs:238`) silently hide repair rows. Must reset all conflicting `TxFilter` fields plus `search_input` and `search_edited_at`. |
| **C-3** | Settlement card hidden when `<2` participants even if exclusions exist | **Confirm P2/P3** (contract conflict `§7.3` vs `§12`) | **Confirm** | **Accept & Add Source Catch** | **New Hydrogen catch:** Both Codex and Opus cited only `src/ui/components/settlement_card.rs:213-215`. In reality, `src/ui/screens/month.rs:396-405` **also** gates calling `settlement_card::show` on `.is_some_and(\|s\| s.balances.len() >= 2)`! Fixing only `settlement_card.rs` leaves the bug alive. Requires coordinated change in `month.rs` + `settlement_card.rs` and explicit `§7.3`/`§12` reconciliation. |
| **O-2** | `ViewModel::reload` partial state corruption, `apply` outcome loss, startup/frame-loop hazard, import overflow gaps | **Corroborated via inspection; qualified** | **New P2** (reproduced partial update & frame error) | **Accept P2, Qualify Fix & Expand Overflow Analysis** | `src/app/view_model.rs:196-221` mutates `profile`, `transactions`, `summary`, etc. before `settlement::for_month` (`line 210`) and `self.loaded` (`line 221`). `AppCore::new` (`app/mod.rs:65`) fails launch if `sync` errors; `AthenaApp::show` (`app/mod.rs:429-431`) retries full `sync()` every frame on error; `AppCore::apply` (`app/mod.rs:100-104`) runs `self.store.apply(action)?` then `self.vm.reload(...)`, discarding `ActionOutcome` (including `inverse` and `toast`) when reload fails—causing `handle_quick_add` (`modals.rs:35-48`) to keep the modal open with `SaveFailed` so a retry double-saves! **Hydrogen checks:** (1) Do **not** collapse settlement error into `None` (conflates absent with broken); expose an explicit unavailable state. (2) Note `reports::tracked_balance` (`reports.rs:58`) and `MonthSummary::new` also use unchecked arithmetic/SQL `SUM`. (3) Import overflow validation (`§10.4`) must check prefix/monthly cutoffs, per-person `paid`/`owed`/`balance`, debtor `-balance` negation (`i64::MIN`), and trash-restoration safety, not just final lifetime aggregates. |
| **O-3** | Retention deletes >30d rows before Task 8 restore UI exists | **Qualify** (release-sequencing gate, not new regression) | **Flag P2** (do not run on real data) | **Qualify as Interim Gate** | `1.1.0-safe-data.md` already requires Tasks 7, 8, and 10 before release. Running `5ba7411` against real data is unsafe right now only if destructive purge executes before Task 8 exists. Enforce Gate 0 (Task R-1) + copy-only checkpoint policy; optionally pause auto-purge behind a tracked gate until Task 8 lands if developers run intermediate builds on copied real profiles. |
| **O-4** | `Record` pre-fills `vm.today` on historical/future months | **Qualify** (obeys `§7.4`; UX ambiguity) | **Flag P3** | **Qualify as Product Decision** | `src/app/mod.rs:295-302` obeys frozen `§7.4` (`today`). Even on the current month, `as_of = month.last_day()` includes future-dated entries within the same month (`settlement_repo.rs:20`). Needs an explicit user decision between keeping `§7.4` vs restricting/warning on non-current months or clamping `as_of`. Never auto-save or silently backdate. |
| **O-5** | Permanent Trash delete leaves stale Undo toast; undo failure silent | **Qualify & Narrow** (`PurgeTransaction` only; `EmptyTrash` replaces toast) | **Flag P3** | **Accept Narrowed Scope** | Confirmed in `store.rs:176-193` + `modals.rs:138-144` + `app/mod.rs:401-416`: `PurgeTransaction` has `toast: None`, so `raise_confirmed_toast` skips clearing an existing `DeleteTransaction` Undo toast (which `toast.rs` hover pause can keep alive across `Ctrl+4`). `EmptyTrash` sets `Some(ToastKey::TrashEmptied)`, which replaces it. `apply_undo` (`app/mod.rs:412-416`) logs `warn!` on error with zero user feedback. Clear any Undo toast pointing to purged row(s) (or any active Undo on destructive Trash actions) and show localized toast on undo failure. |
| **O-6** | Interim format-1 JSON writes `kind: "settlement"` | **Qualify** (Task 9 closure hazard, no proof of shipped files) | **Flag P3** | **Accept as Task 9 Guard** | `src/infrastructure/export/json.rs:22-36` still writes `schema_version: 1` with `Transaction`. Fold cleanly into Task 9 (`format_v1.rs` / `format_v2.rs`, remove `SCHEMA_VERSION` and stale comments). Do not relax real v1 validation speculatively. |
| **O-7** | CSV lacks `Payer`/`From` column; `§5.3` kind wording | **Qualify** (schema change needs decision; tests assert localized label) | **Flag P3** | **Accept as Product Decision** | `src/infrastructure/export/csv.rs:31-72,95-104` writes 8 columns and localized `type_label` (`Settlement`/`Acerto`), asserted by `services/export.rs:247-260`. Changing column count or label vs code breaks existing CSV consumers/tests; route through explicit user approval. |
| **O-8** | Unscoped `LoadKey` reloads all history + settlement per screen/keystroke | **Qualify** (unmeasured; measure first) | **Flag P3** | **Defer Optimization Pending Benchmark** | `view_model.rs:25-32` puts `screen` and `filter` in `LoadKey`, and `sync` calls `reload(..., RefreshScope::All)` every time. Real performance cost is unmeasured. Benchmark 50,000 rows first (Task R-6); optimize in 1.1 only if budget fails, otherwise leave broad refactor for 1.2 Daily Speed per D9. |
| **O-9** | `SettlementEvent::RestorePerson` error maps to generic `settings_save_failed` | **Accept P3** | **Flag P3** | **Accept P3** | `src/app/mod.rs:315-322` shows `"Could not save that. The name may already be in use."` (`en.rs:110`) on any `RestorePerson` error even outside Settings. Add targeted localized copy naming the person on duplicate active name. |
| **Docs / Tags** | `plans/README.md` says `v1.0.2` shipped; Phase 0 / Task 1–6 checkboxes open; `v1.0.2` missing | **Confirm** (verified `origin` + local tags) | **Confirm** | **Accept** | `git tag` shows only `v1.0.0`, `v1.0.1`. `docs/smoke-1.0.md:92-94` and Phase 0 checkboxes are incomplete. Do not invent tags, CI runs, or approvals; reconcile records honestly in Task R-7. |

---

## 2. Proposed Contract & Product Decisions

> **Rule:** Items in **§2.1** are strictly within existing contracts or defensive hardening and can proceed directly under user authorization of this plan. Items in **§2.2** modify frozen `contracts.md` behavior or external formats/copy and **require explicit user approval** before editing `contracts.md` during implementation.

### 2.1 Implementation Hardening (No Frozen Contract Change Required)
- **R-1 Snapshot integrity (`§6.2`):** Require verified Athena snapshot provenance/schema/SQLite integrity (or a fresh verified backup) before any non-empty retention purge; fail closed.
- **R-2 Coherent view state & committed outcome preservation (`§8`):** Stage `ViewModel::reload` fields before swapping `self`; preserve committed `ActionOutcome` in `AppCore::apply` even when refresh fails; never fail startup or spin per-frame reloads on optional settlement errors.
- **R-3 Repair filter hygiene (`§7.4`):** Reset `person_id`, `shared_only`, `category_id`, `kind`, `text`, `search_input`, and `search_edited_at` inside `set_repair_filter`.
- **R-5 Toast & restore-person copy hygiene (`§6.3`, `§12`):** Clear stale `Undo` toast when purged; notify user on undo failure; add specific localized duplicate-name error for `SettlementEvent::RestorePerson`.
- **R-6 50k benchmark before cache refactor (`§9.3`, D9):** Measure before changing `LoadKey` / `RefreshScope`.

### 2.2 Proposed Contract & Product Decisions (Require User Approval)

| # | Decision Area | Contract Section | Options | Hydrogen Recommendation & Proposed `contracts.md §15` Entry |
|---|---|---|---|---|
| **D-P1** | Pre-purge snapshot reuse vs dedicated `pre-purge-` snapshot | `§6.2`, `§9.1` | **Option A (Minimal):** Reuse valid same-day `auto-YYYY-MM-DD.db` after checking `is_file()`, readable SQLite, `PRAGMA integrity_check == "ok"`, Athena identity (`application_id == 0x41544E41` or v1 table set), `user_version in 1..=latest`, and overwriting/replacing if corrupt/invalid (if not written today) OR refusing purge if existing same-day file is invalid/untrusted.<br>**Option B (Stricter):** When `trash_repo::purge_older_than` would delete `> 0` rows, write a fresh `pre-purge-YYYY-MM-DD_HH-MM-SS.db` (or refresh `auto-` if corrupt) so a same-day run after creating/modifying expired rows or a corrupt `auto-` cannot leave unbacked-up data. | **Recommend Option B (with existing-`auto-` validation):** Write/refresh verified `auto-YYYY-MM-DD.db` if absent or invalid (never silently treat a foreign/corrupt `auto-` file as valid, and never destroy an existing valid `auto-` silently—instead write `pre-purge-` if `auto-` exists but lacks provenance or if expired rows exist and today's `auto-` is untrusted). Proposed `§15` note (if `pre-purge-` prefix added): *"Added `pre-purge-YYYY-MM-DD_HH-MM-SS.db` / verified Athena snapshot guard before automatic trash retention (§6.2, §9.1)."* |
| **D-P2** | Settlement card visibility with `<2` participants + coverage / error state | `§7.3`, `§8.3`, `§12` | **Option A:** Show the Settlement card whenever `settlement.excluded.any()` OR settlement computation failed (with explicit localized unavailable notice + Retry), even if `balances.len() < 2` (showing `"As of {date}"` + localized unavailable/no-transfers + exclusion links). Hide only when `balances.len() < 2 && !excluded.any()` and no error.<br>**Option B:** Keep card hidden for `<2` participants and show exclusions as a register banner instead. | **Recommend Option A:** Directly fulfills `§12` ("shows ... a coverage line when exclusions exist") without inventing a second UI banner, and fixes O-2's silent-error gap via explicit `settlement_unavailable: bool` (or `Option<SettlementLoadError>`) on `ViewModel`. Proposed `§15` note: *"Show the settlement card whenever participants ≥ 2, exclusions exist, or settlement is unavailable, and surface explicit localized unavailable/error state (§7.3, §8.3, §12)."* |
| **D-P3** | `Record` button date semantics on historical/future months | `§7.4`, `§12` | **Option A (Status quo):** Keep `§7.4` prefill = `vm.today` regardless of viewed month.<br>**Option B:** Allow **Record** only when viewing the current month (`ui.month == YearMonth::from(vm.today)`); on past/future months show localized hint instead (still note: future-dated entries later in current month are included in `last_day()`).<br>**Option C:** Keep enabled on all months with `vm.today` default plus a clear non-blocking warning in Quick Add when `ui.month != current_month` (and never auto-backdate). | **Recommend Option C (or Option B if user prefers stricter guard):** Preserves ability to record a real payback today while inspecting prior month's settled-up balance without risking silent backdating or dead buttons. Proposed `§15` note (if Option B or C chosen): *"Clarified `Record` prefill/warning when viewed month differs from `today` (§7.4)."* |
| **D-P4** | Interim destructive retention before Task 8 restore UI | `§6.2`, Plans | **Option A:** Keep retention active once R-1 verified-snapshot guard lands, enforcing copy-only policy until Task 8.<br>**Option B:** Temporarily gate automatic purge behind Task 8 completion (or a feature/env guard) in intermediate commits between `5ba7411` and Task 8, then re-enable in Task 8. | **Recommend Option B** if developers test intermediate builds on any non-throwaway copies; **Option A** is safe on fresh/copied test DBs once R-1 lands because R-1 guarantees a verified SQLite snapshot exists on disk before any purge. |
| **D-P5** | Import overflow validation boundary | `§10.4`, `§7.2`, `§5.3` | **Option A (Narrow):** Keep `§10.4` lifetime sums + add settlement dry-run over live rows.<br>**Option B (Complete):** Validate in `ImportError::Overflow` that (a) lifetime income/expenses/refunds, `income + refunds`, and `start_balance + net` fit in `i64`; (b) cumulative settlement balances (`paid`, `owed`, `balance`, debtor `-balance`) fit in `i64` across chronological month-end cutoffs; and (c) checking all imported rows (and for DB-level safety, keeping `settlement::calculate` non-fatal per R-2) prevents post-import overflow. | **Recommend Option B:** Directly aligns validator with every consumer (`reports.rs`, `settlement.rs`, `reports_repo.rs`). Proposed `§15` note: *"Expanded §10.4 aggregate overflow validation to cover cumulative settlement paid/owed/net/debtor bounds across chronological month cutoffs."* |
| **D-P6** | CSV `Payer` column & `type_label` vs `§5.3` wording | `§5.3` | **Option A (Keep 8 cols + localized label):** Update `§5.3` wording to match `csv.rs` (`Settlement`/`Acerto` localized label, matching existing tests `csv_writes_settlement_kind`).<br>**Option B (9 cols):** Add `Payer` / `Pagador` column (`csv_payer` i18n key) for settlements/expenses/refunds so CSV exports show settlement direction (`From -> To`), and reconcile `§5.3` wording. | **Recommend Option B** for 1.1 CSV completeness (with localized `type_label` kept per existing tests), subject to user sign-off on changing CSV columns from 8 to 9. Proposed `§15` note: *"Added localized Payer column to CSV export and clarified localized kind label in §5.3."* |
| **D-P7** | `Trash` screen retention copy regarding backups | `§12` | Keep existing `"Daily backups in your data folder also keep earlier snapshots"` (`en.rs:101`) vs explicitly clarify the 7-day rolling window of `auto-` backups so users do not assume deleted items stay backed up forever. | **Recommend clarifying copy** alongside Task 8/13 (`"Recent daily backups (last 7 days) also keep earlier snapshots"`). Proposed `§15` note if edited. |

---

## 3. File Map & Architectural Boundaries

- **Remediation Tasks (R-1 .. R-7):**
  - **R-1:** `src/infrastructure/db/backup.rs`, `src/app/store.rs`
  - **R-2:** `src/app/view_model.rs`, `src/app/mod.rs`, `src/app/modals.rs`, `src/ui/screens/month.rs`, `src/ui/components/settlement_card.rs`, `src/i18n/{mod.rs,en.rs,pt_br.rs}`
  - **R-3:** `src/app/ui_state.rs`, `src/ui/screens/month.rs`
  - **R-4:** `src/ui/screens/month.rs`, `src/ui/components/settlement_card.rs`, `src/ui/quick_add.rs` (only if D-P3 Option C adds warning), `src/i18n/*`, `docs/superpowers/specs/contracts.md` (if D-P2/D-P3 approved)
  - **R-5:** `src/app/mod.rs`, `src/app/modals.rs`, `src/i18n/{mod.rs,en.rs,pt_br.rs}`
  - **R-6:** `src/app/view_model.rs` (benchmark test; refactor only if measured > budget)
  - **R-7:** `docs/superpowers/plans/README.md`, `docs/smoke-1.0.md`, `docs/superpowers/specs/contracts.md` (with approval)
- **Canonical 1.1 Tasks (Unchanged Ownership in [1.1.0-safe-data.md](2026-11-16-athena-1.1.0-safe-data.md)):**
  - Task 7: `backup.rs`, `config.rs`, `store.rs`, `view_model.rs`, `i18n/*`
  - Task 8: `Cargo.toml`, `restore.rs`, `db/mod.rs`, `error.rs`, `store.rs`, `app/mod.rs`, `settings/{mod.rs,data.rs}`, `i18n/*`
  - Task 9: `export/{json.rs,format_v1.rs,format_v2.rs,csv.rs}`, `services/{export.rs,import.rs}`, `people_repo.rs`, `categories_repo.rs`, `transactions_repo.rs`
  - Task 10: `import_repo.rs`, `store.rs`, `app/mod.rs`, `settings/data.rs`, `import_preview.rs`, `i18n/*`
  - Tasks 11–13: per [1.1.0-safe-data.md](2026-11-16-athena-1.1.0-safe-data.md)

---

## 4. Prioritized Remediation & Integration Tasks

### Gate 0 (P1) — Immediate Data-Safety & State-Integrity Fixes

> Must complete before running any checkpoint or touching any copied user database.

---

#### Task R-1: Retention Snapshot Validation & Fail-Closed Guard

**Contract:** `§6.2`, `§9.1` (and **D-P1** if `pre-purge-` prefix chosen).
**Owning files:** `src/infrastructure/db/backup.rs`, `src/app/store.rs`.
**Dependencies:** None (first task from `5ba7411`).
**Non-goals:** Manual backup listing/pruning UI (Task 7); full restore pipeline (Task 8).

**Root cause:** `backup::daily_snapshot` (`src/infrastructure/db/backup.rs:85-87`) returns `Ok(None)` whenever `auto-YYYY-MM-DD.db` matches `.exists()`, without verifying it is a regular file, readable SQLite, integrity-clean, or an Athena schema. `Store::run_startup_tasks` (`src/app/store.rs:54-60`) then executes `run_trash_retention` on `Ok(None)`.

**Step 1 — Write failing tests first** in `src/infrastructure/db/backup.rs` and `src/app/store.rs`:

```rust
// In src/infrastructure/db/backup.rs #[cfg(test)] mod tests:
#[test]
fn verify_athena_snapshot_rejects_empty_corrupt_dir_and_foreign_sqlite() {
    let dir = tempfile::tempdir().unwrap();

    // 1. Directory at path
    let as_dir = dir.path().join("dir.db");
    std::fs::create_dir_all(&as_dir).unwrap();
    assert!(!is_valid_athena_snapshot(&as_dir).unwrap_or(false));

    // 2. Empty file
    let empty = dir.path().join("empty.db");
    std::fs::write(&empty, b"").unwrap();
    assert!(!is_valid_athena_snapshot(&empty).unwrap_or(false));

    // 3. Corrupt garbage
    let corrupt = dir.path().join("corrupt.db");
    std::fs::write(&corrupt, b"not sqlite at all").unwrap();
    assert!(!is_valid_athena_snapshot(&corrupt).unwrap_or(false));

    // 4. Valid non-Athena SQLite database
    let foreign = dir.path().join("foreign.db");
    {
        let conn = Connection::open(&foreign).unwrap();
        conn.execute_batch("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);").unwrap();
    }
    assert!(!is_valid_athena_snapshot(&foreign).unwrap_or(false));

    // 5. Valid Athena v1 database (without application_id set)
    let v1 = dir.path().join("v1.db");
    {
        let mut conn = Connection::open(&v1).unwrap();
        crate::infrastructure::db::run_migrations_with(
            &mut conn,
            &crate::infrastructure::db::MIGRATIONS[..1],
        )
        .unwrap();
    }
    assert!(is_valid_athena_snapshot(&v1).unwrap());
}
```

```rust
// In src/app/store.rs #[cfg(test)] mod tests:
#[test]
fn retention_skips_purge_when_existing_daily_snapshot_is_untrusted() {
    let now_utc = DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();

    for case in ["empty", "corrupt", "foreign_sqlite", "directory"] {
        let dir = tempfile::tempdir().unwrap();
        let dirs = AppDirs::from_root(dir.path().to_path_buf());
        dirs.ensure().unwrap();
        let store = Store::new(Db::open_in_memory().unwrap(), dirs.clone());
        let expired = transactions_repo::create(store.db().conn(), &draft_with(vec![])).unwrap();
        store
            .db()
            .conn()
            .execute(
                "UPDATE transactions SET deleted_at = '2026-10-01T00:00:00Z' WHERE id = ?1",
                [expired],
            )
            .unwrap();

        let snap = dirs.backups_dir.join("auto-2026-11-16.db");
        match case {
            "empty" => std::fs::write(&snap, b"").unwrap(),
            "corrupt" => std::fs::write(&snap, b"broken").unwrap(),
            "foreign_sqlite" => {
                let conn = rusqlite::Connection::open(&snap).unwrap();
                conn.execute_batch("CREATE TABLE other (x INTEGER);").unwrap();
            }
            "directory" => std::fs::create_dir_all(&snap).unwrap(),
            _ => unreachable!(),
        }

        store.run_startup_tasks(today, now_utc);
        assert_eq!(
            trash_repo::count(store.db().conn()).unwrap(),
            1,
            "expired row must survive when auto snapshot is {case}"
        );
    }
}
```

**Step 2 — Run failing tests:**
`cargo test --locked verify_athena_snapshot_rejects_empty_corrupt_dir_and_foreign_sqlite` followed by `cargo test --locked retention_skips_purge_when_existing_daily_snapshot_is_untrusted`
Expected: FAIL (no `is_valid_athena_snapshot`; existing `store.rs` purges the expired row).

**Step 3 — Implement minimal fix:**
**Mandatory coverage invariant (both D-P1 options):** Snapshot identity/schema/integrity alone does not authorize purge. Before deleting a nonempty batch, prove the snapshot preserves the current canonical contents and allocations of each eligible row, or create a fresh verified snapshot of the current live connection. A row may be live in the snapshot and still be recoverable. Compare content and allocations, not IDs or deleted_at alone. If no proof exists, skip purge. A supported v1 snapshot cannot cover settlement rows it cannot represent. Never overwrite an existing unknown or valid backup to make validation pass. Add regression cases for a valid Athena snapshot missing an eligible row and one holding older contents/allocations under the same ID; purge must be skipped or preceded by a new verified snapshot. Include a snapshot with an identical live row to avoid incorrectly requiring its deleted flag. The helper below is an identity check only, not coverage proof.

1. In `src/infrastructure/db/backup.rs`:
   - Add `pub fn is_valid_athena_snapshot(path: &Path) -> Result<bool>` (shared helper also reusable by Task 8's restore validation):
     - Check `std::fs::symlink_metadata(path)` -> `is_file()` and `len() > 0`; if false return `Ok(false)`.
     - Open read-only with `OpenFlags::SQLITE_OPEN_READ_ONLY`, check `PRAGMA integrity_check == "ok"`, `PRAGMA user_version` in `1..=latest_version()`.
     - Check Athena identity: `PRAGMA application_id == 0x41544E41`, **or** (`user_version == 1` and exact v1 table set `{"profile", "people", "categories", "transactions", "allocations"}` via `sqlite_master`—not a bare `notes` DB).
     - Return `Ok(false)` on any malformed SQLite / validation mismatch (do not panic).
   - In `daily_snapshot(conn, dirs, today)`:
     - Check `if path.exists()`: if `is_valid_athena_snapshot(&path)` is `Ok(true)`, return `Ok(None)`; otherwise return `Err(AppError::Backup(format!("existing daily snapshot {} failed verification", path.display())))` (fail closed—never silently overwrite an unknown directory/foreign file nor silently return `Ok(None)`!).
   - Add `pub fn verify_pre_purge_snapshot(conn: &Connection, dirs: &AppDirs, today: NaiveDate, now_utc: DateTime<Utc>) -> Result<()>` (if **D-P1 Option B** approved, writes `pre-purge-YYYY-MM-DD_HH-MM-SS.db` via `vacuum_into` + verifies `is_valid_athena_snapshot` whenever expired rows exist; under **D-P1 Option A**, verifies snapshot identity AND mandatory canonical row/allocation coverage above before calling `purge_older_than`).
2. In `src/app/store.rs`:
   - In `run_trash_retention`, check clock rollback and whether any rows are actually eligible for purge (`cutoff`). Only when count to purge `> 0`, call `backup::verify_pre_purge_snapshot(...)` (or re-verify `auto-YYYY-MM-DD.db`); on `Err(err)`, log `tracing::warn!` and return without calling `trash_repo::purge_older_than`.

**Step 4 — Standard verification & commit:**
- Run `cargo test --locked verify_athena_snapshot` and `cargo test --locked retention_skips_purge` then `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`.
- Commit: `fix(backup): verify Athena snapshot provenance and integrity before trash retention`

---

#### Task R-2: Atomic `ViewModel::reload`, Committed Outcome Preservation & Explicit Settlement Availability

**Contract:** `§8.2`, `§8.3`, `§10.4` (see **D-P2** for `ViewModel.settlement_error` / card notice).
**Owning files:** `src/app/view_model.rs`, `src/app/mod.rs`, `src/app/modals.rs`, `src/ui/screens/month.rs`, `src/ui/components/settlement_card.rs`, `src/i18n/{mod.rs,en.rs,pt_br.rs}`.
**Dependencies:** None (can run immediately after R-1).
**Non-goals:** Broad query caching across unrelated screens (R-6 measures first); full import overflow validator (Task 9).

**Root cause:**
1. `ViewModel::reload` (`src/app/view_model.rs:196-222`) writes directly to `self.*` before calling `settlement::for_month` (`line 210`) and updating `self.loaded` (`line 221`). Any error (such as `AppError::Validation("settlement calculation overflow")`) leaves half-updated fields (`transactions` from new month/version while `loaded` points to old key).
2. `AppCore::new` (`src/app/mod.rs:65`) calls `core.sync()?` (blocking startup completely on settlement error); `AthenaApp::show` (`src/app/mod.rs:429-431`) retries `sync()` every rendered frame while `loaded` remains stale.
3. `AppCore::apply` (`src/app/mod.rs:100-105`) commits `self.store.apply(action)?` and then runs `self.vm.reload(...)?`. If `reload` fails, `apply` returns `Err`, losing `ActionOutcome` (toast and undo inverse) and causing `handle_quick_add` (`modals.rs:35-48`) to keep the modal open with `QuickAddError::Save`, tempting double submission.

**Step 1 — Write failing tests first:**

```rust
// In src/app/view_model.rs #[cfg(test)] mod tests:
#[test]
fn settlement_overflow_keeps_app_loadable_and_other_fields_coherent() {
    use crate::domain::transaction::{Allocation, TransactionType};
    use crate::shared::test_fixtures::{date, draft_with, seed_people};

    let mut store = Store::new(
        Db::open_in_memory().unwrap(),
        AppDirs::from_root(std::env::temp_dir().join("athena-vm-overflow-test")),
    );
    let (owner, partner) = seed_people(store.db().conn());
    let mut expense = draft_with(vec![Allocation {
        person_id: partner,
        amount: Money::from_cents(i64::MAX / 2 + 100),
    }]);
    expense.title = "Huge expense".into();
    expense.kind = TransactionType::Expense;
    expense.payer_id = Some(owner);
    expense.date = date(2026, 11, 10);
    store.apply(AppAction::SaveTransaction(expense)).unwrap();

    // Settlement is excluded from reports_repo SUM, so only settlement::for_month overflows.
    let mut settlement_tx = draft_with(vec![Allocation {
        person_id: partner,
        amount: Money::from_cents(i64::MAX / 2 + 100),
    }]);
    settlement_tx.title = "Huge settlement".into();
    settlement_tx.kind = TransactionType::Settlement;
    settlement_tx.payer_id = Some(owner);
    settlement_tx.category_id = None;
    settlement_tx.split_mode = None;
    settlement_tx.date = date(2026, 11, 11);
    store.apply(AppAction::SaveTransaction(settlement_tx)).unwrap();

    let ui = UiState::new(YearMonth::new(2026, 11).unwrap());
    let mut vm = ViewModel::new(Language::En);
    vm.today = date(2026, 11, 16);

    // Sync succeeds, loaded key is recorded (no per-frame retry loop), transactions & profile load,
    // and settlement unavailability is explicitly recorded rather than crashing or half-mutating.
    assert!(vm.sync(&store, &ui).unwrap());
    assert!(!vm.sync(&store, &ui).unwrap(), "must not retry every frame for same key");
    assert_eq!(vm.transactions.len(), 2);
    assert!(vm.settlement.is_none());
    assert!(vm.settlement_unavailable);
}
```

```rust
// In src/app/mod.rs #[cfg(test)] mod tests:
#[test]
fn apply_preserves_committed_outcome_when_settlement_refresh_errors() {
    use crate::domain::transaction::{Allocation, TransactionType};
    use crate::ui::quick_add::QuickAddEvent;
    let mut core = AppCore::empty_for_test(Db::open_in_memory().unwrap());
    let (owner, partner) = crate::shared::test_fixtures::seed_people(core.store.db().conn());

    let mut d1 = draft_with(vec![Allocation {
        person_id: partner,
        amount: Money::from_cents(i64::MAX / 2 + 100),
    }]);
    d1.payer_id = Some(owner);
    core.apply(AppAction::SaveTransaction(d1)).unwrap();

    // Second large entry (Settlement) triggers settlement paid overflow while itself being valid.
    let mut d2 = draft_with(vec![Allocation {
        person_id: partner,
        amount: Money::from_cents(i64::MAX / 2 + 100),
    }]);
    d2.kind = TransactionType::Settlement;
    d2.payer_id = Some(owner);
    d2.category_id = None;
    d2.split_mode = None;

    let mut qa = QuickAddState::new(
        &core.vm.active_people().into_iter().cloned().collect::<Vec<_>>(),
        core.vm.today,
        Language::En,
    );
    let (after, toast) = handle_quick_add(&mut core, &mut qa, QuickAddEvent::Submit(d2));
    assert_eq!(after, ModalAfter::Keep); // Create resets for next, does NOT stay stuck with SaveFailed
    assert!(qa.errors.is_empty(), "committed write must not report SaveFailed");
    assert_eq!(toast, Some(ToastKey::TransactionSaved));
    assert_eq!(core.vm.transactions.len(), 2);
    assert!(core.vm.settlement_unavailable);
}
```

**Step 2 — Run failing tests:**
`cargo test --locked settlement_overflow_keeps_app_loadable` followed by `cargo test --locked apply_preserves_committed_outcome`
Expected: FAIL (`sync` returns `Err(Validation("settlement calculation overflow"))` leaving partial fields, `handle_quick_add` reports `QuickAddError::Save`).

**Step 3 — Implement minimal fix:**
1. In `src/app/view_model.rs`:
   - Add `pub settlement_unavailable: bool` (default `false`) to `ViewModel`.
   - Build all reloaded fields into local variables inside `reload` **before** mutating `self` (staging pattern). Only after all non-optional queries succeed, swap `self.*` atomically and update `self.loaded = Some(key)`.
   - Isolate `settlement::for_month(conn, ui.month)`:
     - On `Ok(view)` -> `settlement = Some(view); settlement_unavailable = false;`
     - On `Err(err)` -> `tracing::warn!("settlement unavailable for {}: {err}", ui.month); settlement = None; settlement_unavailable = true;`
   - Because `self.loaded = Some(key)` is committed when optional settlement is unavailable, `sync()` does **not** re-run every frame until `data_version`, `month`, `year`, `filter`, `today`, or `screen` changes (or a manual Retry bumps/clears `loaded`).
2. In `src/app/mod.rs`:
   - For hard read failures during `AppCore::apply` (e.g., if a required `reports`/`transactions_repo` query ever fails after `self.store.apply(action)?` already bumped `data_version`): log `tracing::error!`, do **not** discard `Ok(outcome)` (or attach a non-fatal refresh error status to `ViewModel` + toast) so callers like `handle_quick_add` never mistake a committed mutation for a failed write and never lose `outcome.inverse`. Keep `AppCore::apply`'s signature or return `Result<ActionOutcome>` returning `Ok(outcome)` when `store.apply` committed while marking `vm.refresh_failed = true` and keeping `self.loaded` unchanged so an explicit or bounded retry can recover cleanly. Record the failed read key separately and suppress automatic retries for that same key; do not leave required-query failures retrying every frame. Display a localized stale/unavailable-data notice while preserving coherent prior data.
3. In `src/ui/screens/month.rs`, `src/ui/components/settlement_card.rs`, `src/i18n/{mod.rs,en.rs,pt_br.rs}` (coordinate with Task R-4 / **D-P2**):
   - Render the settlement card when `vm.settlement_unavailable` is true, showing localized unavailable message (`settlement_unavailable` in EN/PT-BR) instead of silently hiding the card.
4. Also audit `src/application/services/reports.rs:58` (`tracked_balance`) and `src/domain/reports.rs` (`MonthSummary::new`): replace unchecked `+` / `-` with checked arithmetic returning `AppError::Validation` so integer overflow never panics in debug or wraps in release.

**Step 4 — Standard verification & commit:**
- `cargo fmt --check; cargo clippy --all-targets --locked -- -D warnings; cargo test --locked`
- Commit: `fix(app): stage ViewModel reload atomically and isolate settlement failures`

---

### Finish Task 6 Cleanly (P2) — Pre-Task-7 & Checkpoint

---

#### Task R-3: Repair Filter Full Reset & Pending Search Debounce Clear

**Contract:** `§7.4`.
**Owning files:** `src/app/ui_state.rs`, `src/ui/screens/month.rs`.
**Dependencies:** Gate 0 (R-1, R-2).
**Non-goals:** Adding cross-month search toggle (1.2 Daily Speed).

**Root cause:** `UiState::set_repair_filter` (`src/app/ui_state.rs:71-75`) leaves `person_id`, `shared_only`, `category_id`, `kind`, `text`, `search_input`, and `search_edited_at` intact.

**Step 1 — Write failing tests first** in `src/app/ui_state.rs` and `src/ui/screens/month.rs`:

```rust
// In src/app/ui_state.rs #[cfg(test)] mod tests:
#[test]
fn repair_filter_clears_conflicting_filters_and_pending_search() {
    use crate::domain::transaction::{Allocation, TransactionType};
    use crate::infrastructure::db::{Db, transactions_repo};
    use crate::shared::test_fixtures::{date, draft_with, seed_people};

    let nov = YearMonth::new(2026, 11).unwrap();
    let mut ui = UiState::new(nov);
    let db = Db::open_in_memory().unwrap();
    let (_owner, partner) = seed_people(db.conn());

    let mut unassigned = draft_with(vec![]);
    unassigned.title = "Unassigned item".into();
    unassigned.date = date(2026, 10, 5);
    transactions_repo::create(db.conn(), &unassigned).unwrap();

    // Activate every conflicting filter + pending search debounce
    ui.filter_shared();
    ui.set_category_filter(Some(1));
    ui.filter.kind = Some(TransactionType::Income);
    ui.filter.text = "nomatch".into();
    ui.edit_search("still typing".into(), 10.0);

    ui.set_repair_filter(SettlementIssue::Unassigned, nov.last_day());
    assert_eq!(ui.filter.person_id, None);
    assert!(!ui.filter.shared_only);
    assert_eq!(ui.filter.category_id, None);
    assert_eq!(ui.filter.kind, None);
    assert!(ui.filter.text.is_empty());
    assert!(ui.search_input.is_empty());
    assert!(!ui.search_pending());
    assert!(!ui.commit_search_if_due(11.0), "pending search must not re-apply");

    let rows = transactions_repo::list(db.conn(), &ui.filter).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Unassigned item");

    // Also verify Person filter + Unassigned yields the item after repair filter activation
    ui.filter_person(partner);
    ui.set_repair_filter(SettlementIssue::Unassigned, nov.last_day());
    let rows2 = transactions_repo::list(db.conn(), &ui.filter).unwrap();
    assert_eq!(rows2.len(), 1);
}
```

Add an `egui_kittest` UI test in `src/ui/screens/month.rs` (or `src/app/mod.rs`) where the user clicks `Shared` (or a person filter), types in the search box (leaving `search_pending() == true`), clicks the settlement `Unassigned` exclusion link, steps UI frames beyond `SEARCH_DEBOUNCE` (`0.2s`), and verifies the unassigned row remains visible in the register.

**Step 2 — Run failing tests:**
`cargo test repair_filter_clears_conflicting_filters_and_pending_search`
Expected: FAIL (`shared_only` stays `true`, `rows.len() == 0`, `search_pending()` stays `true`).

**Step 3 — Implement minimal fix** in `src/app/ui_state.rs`:

```rust
pub fn set_repair_filter(&mut self, issue: SettlementIssue, as_of: NaiveDate) {
    self.filter = TxFilter {
        month: None,
        until: Some(as_of),
        issue: Some(issue),
        ..TxFilter::default()
    };
    self.search_input.clear();
    self.search_edited_at = None;
}
```

**Step 4 — Standard verification & commit:**
- `cargo fmt --check; cargo clippy --all-targets --locked -- -D warnings; cargo test --locked`
- Commit: `fix(ui): clear conflicting register filters and pending search on repair filter`

---

#### Task R-4: Settlement Card Coverage Visibility (<2 Participants) & Record Date Alignment

**Contract:** `§7.3`, `§7.4`, `§12` (apply **D-P2** and **D-P3** once user approves).
**Owning files:** `src/ui/screens/month.rs`, `src/ui/components/settlement_card.rs`, `src/i18n/{mod.rs,en.rs,pt_br.rs}`, `docs/superpowers/specs/contracts.md` (with approval).
**Dependencies:** R-2, R-3.

**Root cause:** Both `src/ui/screens/month.rs:396-405` and `src/ui/components/settlement_card.rs:213-215` check `settlement.balances.len() < 2`, suppressing the card when 1 (or 0) participants have valid activity but 1+ exclusions (`unknown_payer` or `unassigned`) exist—leaving users with no clickable repair path on single-payer/unassigned ledgers.

**Step 1 — Write failing tests first** in `src/ui/components/settlement_card.rs` and `src/ui/screens/month.rs`:

```rust
#[test]
fn rail_shows_settlement_card_for_single_participant_when_exclusions_exist() {
    // Seed 1 unassigned expense (0 or 1 active participant in balances, excluded.unassigned == 1)
    // Render month::show / settlement_card::show via egui_kittest
    // Assert card renders "Settlement", "As of ...", and clickable "1 without anyone responsible" link
    // Assert clicking emits MonthEvent::Settlement(SettlementEvent::FilterIssue { .. })
}

#[test]
fn rail_hides_settlement_card_when_fewer_than_two_participants_and_no_exclusions_or_error() {
    // Assert settlement_card::show returns None without drawing when balances.len() < 2 && !excluded.any() && !settlement_unavailable
}
```

Plus, if **D-P3 Option B or C** is approved, add a test in `src/app/mod.rs` / `src/ui/quick_add.rs` asserting historical/future month `Record` behavior (never backdating automatically; showing hint/warning or gating per approved option).

**Step 2 — Implement fix:**
1. Add helper `pub fn should_show_settlement_card(vm: &ViewModel) -> bool`:
   - `true` if `vm.settlement_unavailable` is `true`, OR `vm.settlement.as_ref().is_some_and(|s| s.balances.len() >= 2 || s.excluded.any())`.
2. Update `src/ui/screens/month.rs:396-405` to call `should_show_settlement_card(vm)` instead of duplicating `.balances.len() >= 2`.
3. Update `src/ui/components/settlement_card.rs:211-265`:
   - Use the single predicate `should_show_settlement_card(vm)`.
   - When `vm.settlement_unavailable` is `true`, show title + localized unavailable notice (per R-2).
   - When `balances.len() < 2` and `excluded.any()`, show title + `as_of` + muted/status note (no fake transfer rows) + `exclusions_line`.
4. Snapshots: inspect whether any `month_*` golden fixture is affected (if all populated fixtures have `>= 2` participants and no exclusions/errors, snapshots stay identical).

**Step 3 — Standard verification & commit:**
- `cargo fmt --check; cargo clippy --all-targets --locked -- -D warnings; cargo test --locked`
- Commit: `fix(settlement): show exclusion repair links and unavailable state in rail card`

---

#### Task R-5: Stale Undo Toast Clearing, Undo Failure Toast & Specific `RestorePerson` Copy

**Contract:** `§6.3`, `§8.1`, `§12`.
**Owning files:** `src/app/mod.rs`, `src/app/modals.rs`, `src/i18n/{mod.rs,en.rs,pt_br.rs}`.
**Dependencies:** R-2.

**Root cause:**
1. `PurgeTransaction(id)` sets `toast: None` (`src/app/store.rs:178-183`); `raise_confirmed_toast` (`src/app/mod.rs:401-410`) skips updating `self.toast` when `toast` is `None`, leaving a prior `DeleteTransaction` Undo toast alive (`toast.rs` hover pause can preserve it across `Ctrl+4` into Trash). Clicking Undo or pressing `Ctrl+Z` then invokes `RestoreTransaction(id)` on a hard-deleted row, which fails silently because `apply_undo` (`src/app/mod.rs:412-416`) only logs `warn!`.
2. `SettlementEvent::RestorePerson(id)` (`src/app/mod.rs:315-322`) falls back to `settings_save_failed` instead of a targeted message naming the restored person.

**Step 1 — Write failing tests first** in `src/app/mod.rs`:
- `purge_clears_matching_undo_toast_and_undo_failure_shows_toast` (kittest):
  1. Soft-delete row `id` -> Undo toast (`ToastAction::Undo(AppAction::RestoreTransaction(id))`) is visible.
  2. Switch to `Screen::Trash` (`Ctrl+4`), confirm `PurgeTransaction(id)`.
  3. Assert `app.toast` no longer contains `RestoreTransaction(id)` (and pressing `Ctrl+Z` does not trigger a failed restore).
  4. Also invoke `apply_undo` directly with a stale `RestoreTransaction(999_999)` and assert `app.toast` displays localized undo-failed text.
- `restore_person_duplicate_name_shows_specific_localized_error`:
  1. Archive `"Ana"`, create active `"Ana"`, click `RestorePerson` from Settlement card.
  2. Assert `handle_screen_event` returns `false` and `app.toast` text names `"Ana"` using the dedicated duplicate-name key rather than `settings_save_failed`.

**Step 2 — Implement fix:**
1. Add `i18n` keys (EN + PT-BR):
   - `toast_undo_failed`: `"Could not undo that change."` / `"Não foi possível desfazer essa alteração."`
   - `settlement_restore_name_taken`: `"Cannot restore {name}: an active person already uses that name."` / `"Não foi possível restaurar {name}: já existe uma pessoa ativa com esse nome."`
2. In `src/app/mod.rs`:
   - In `ConfirmChoice::Confirmed(action)` handling (`lines 527-532`): on `PurgeTransaction(purged_id)`, clear `self.toast` if `matches!(&t.action, Some(ToastAction::Undo(AppAction::RestoreTransaction(id))) if *id == purged_id)` (and on `EmptyTrash`, clear any `ToastAction::Undo(AppAction::RestoreTransaction(_))`).
   - In `apply_undo(&mut self, ctx: &egui::Context, action: AppAction)`: on `Err(err)`, log `warn!` and `self.raise_toast(&ctx, Some(ToastKey::UndoFailed))` (or `raise_toast_text`).
   - In `ScreenEvent::Settlement(SettlementEvent::RestorePerson(id))`: on `Err(AppError::Validation(_))`, look up `self.core.vm.person(id)` to format `settlement_restore_name_taken` with `{name}`, falling back to `settings_save_failed` on non-validation errors.

**Step 3 — Standard verification & commit:**
- `cargo fmt --check; cargo clippy --all-targets --locked -- -D warnings; cargo test --locked`
- Commit: `fix(ui): clear stale undo toast after purge and improve undo/restore errors`

---

#### Task 6 Checkpoint (Hard Stop & Report)

- Verify on a **copy** of the user's real data directory (never the live directory):
  1. Launch built binary against copied directory; verify R-1 snapshot validation passes cleanly.
  2. Inspect Month screen settlement card, repair filter links (`UnknownPayer`, `Unassigned`), and `Record` flow.
  3. Capture screenshot of the settlement card on the copied database.
  4. Report checkpoint results to user before proceeding to Task 7.

---

### Performance Triage (P3 — Measure Before Refactoring)

#### Task R-6: 50,000-Row Settlement & `ViewModel::sync` Benchmark

**Contract:** `§8.3`, `§9.3`, D9.
**Owning files:** `src/app/view_model.rs` (or `tests/perf_settlement.rs`).
**Dependencies:** Finish Task 6 (or run alongside Task 8's 50k restore benchmark).
**Non-goals:** Changing `LoadKey` structure unless measured latency violates interactive budgets.

**Steps:**
1. Add a deterministic `#[ignore]` (or debug-timed) benchmark test generating 50,000 transactions (mixed expenses/refunds/settlements across 3 years, 2+ people) in a temp WAL DB, measuring:
   - `settlement::for_month` runtime (`settlement_repo::list_inputs` + `calculate`)
   - `ViewModel::sync` on screen switch (`Month -> Trash -> Settings -> Month`)
   - `ViewModel::sync` on filter/search change (`edit_search` -> `commit_search_if_due`)
2. **Decision rule (per Synthesis §9 & D9):**
   - If screen/filter reloads stay responsive (e.g., `< 50 ms` on reference hardware), record the numbers and leave broad read-model splitting for **1.2 Daily Speed**.
   - If screen/filter reloads measurably exceed interactive budget (`> 100 ms`), split screen-only reloads (`RefreshScope::Trash`, `Backups`) out of `LoadKey.screen` triggering `RefreshScope::All` and/or only recompute `settlement`/`calendar` when `data_version`, `month`, `year`, or `today` changed (not `filter.text` / `screen`), keeping all `§8.3` behavior intact.

---

### Integration with Canonical Safe Data Tasks 7–13

> Canonical definitions live in [2026-11-16-athena-1.1.0-safe-data.md](2026-11-16-athena-1.1.0-safe-data.md). Below are the exact additions/constraints from this review cycle that hook into each task—no duplicate DTOs or parallel pipelines.

#### Task 7 Integration: Backup Files (`§9.1`, `§9.2`)
- Reuse R-1's `backup::is_valid_athena_snapshot` (do not write a duplicate file-validation routine).
- If **D-P1 Option B** (`pre-purge-YYYY-MM-DD_HH-MM-SS.db`) is approved, add `BackupKind::PrePurge`, list it in `backup::list`, and retain newest 5 via `backup::prune_prefix(..., "pre-purge-", 5)` alongside `pre-restore-` and `pre-import-`.
- Ensure `clear_staging` runs at startup in `Store::run_startup_tasks` after `dirs.ensure()` and never touches `backups_dir`.

#### Task 8 Integration: Restore Pipeline & Data Page (`§9.3`, `§12`)
- **Step 3 validation:** Share `backup::is_valid_athena_snapshot` + enforce `page_size` match with live DB (`§9.3` step 3).
- **Atomic UI refresh (from R-2):** After `Store::apply(AppAction::RestoreBackup(_))` commits SQLite restore (`§9.3` step 6–7), `AppCore::apply` must clear `UiState` filters, return to `Screen::Month`, preserve `last_operation` (`OperationKind::Restored` with snapshot path), and keep `ActionOutcome` even if an optional `settlement` calculation on the restored data produces `settlement_unavailable = true`.
- **Gate lift (from O-3 / D-P4):** If automatic retention was temporarily paused pending restore UI, re-enable it in Task 8 and confirm `§6.2` recovery end-to-end.
- **50k measurement:** Record Task 8's `< 1 s` 50,000-row restore timing alongside R-6's `ViewModel::sync` measurement in the Task 8 commit body.

#### Task 9 Integration: Format 2 Export, Format 1 Adapter, Validator (`§10.1–§10.4`)
- **Resolves O-6:** Remove `SCHEMA_VERSION` and the stale `"Templates arrive in 1.1"` comment (`<!-- lint:allow -->`) from `src/infrastructure/export/json.rs`; freeze v1 shapes in `format_v1.rs` and write `FORMAT_VERSION = 2` via `format_v2.rs` (`§10.2`). Keep genuine Format 1 validation strict (`templates` empty, no `Settlement`).
- **Resolves O-2 / D-P5 Import Overflow Bounds:** Extend the Task 9 validator (`§10.4`) with `ImportError::Overflow` checking:
  1. Per-month and lifetime `income`, `expenses`, `refunds`, `income + refunds`, and `start_balance + net` fit in `i64` (matching `reports_repo` SQL `SUM`, `MonthSummary::new`, and `reports::tracked_balance`).
  2. Chronological cumulative settlement prefix checks at every distinct transaction date / month-end cutoff: every person's `paid`, `owed`, net `balance = paid - owed`, and debtor `-balance` (`0i64.checked_sub(balance)`) fit in `i64` without overflow.
  3. Add unit tests for prefix overflow (an earlier expense batch exceeding `i64::MAX` later offset by refunds) and `i64::MIN` debtor negation.
- **Resolves O-7 / D-P6 (if approved):** If adding a localized `Payer` / `Pagador` column to `csv.rs` is approved, update `csv.rs` (9 columns, `csv_safe` on payer name, empty when `payer_id` is `None`), `en.rs`/`pt_br.rs`, `services/export.rs` tests, and `contracts.md §5.3`.

#### Task 10 Integration: Import Pipeline & Preview (`§10.5`)
- Verify `import_repo::replace` wipes trash (`transactions` rows with `deleted_at IS NOT NULL`) atomically inside the single SQLite transaction after writing `pre-import-<ts>.db`, and runs `R-2`'s coherent `AppCore::apply` refresh (`UiState` reset + `Screen::Month` + `last_operation` naming snapshot).
- **Task 10 Checkpoint:** Execute restore and import E2E on a copied real data folder (1.0.2 Format-1 JSON import, 1.1 Format-2 JSON round-trip, manual/listed/external backup restore, Trash replacement preview count) and report to user.

#### Tasks 11–12 & 13 Integration
- **Tasks 11–12 (P2):** Execute Startup Recovery (`§11`) and Keyboard/Focus polish (`§6.3`, `§12`) unless explicitly deferred under the cut line (with status documented in `README.md` and `docs/smoke-1.1.md`).
- **Task 13 (P0):** Execute Release 1.1.0 (`Cargo.toml`, `installer/athena.iss`, `README.md`, `docs/smoke-1.1.md`, `contracts.md §15`, release SHA-256 verification).

---

### Documentation & Records Alignment

#### Task R-7: Fix Plan Status, Phase 0 Records & Contracts Change Log

**Contract:** `§13`, `§15`.
**Owning files:** `docs/superpowers/plans/README.md`, `docs/smoke-1.0.md`, `docs/superpowers/specs/contracts.md` (with approval of **D-P1..D-P7**).
**Non-goals:** Fabricating tags, CI runs, or historical human approvals; modifying `README.md` 1.1 user docs before Task 13.

**Steps:**
1. Update `docs/superpowers/plans/README.md`:
   - Change `Shipped (v1.0.2)` on line 18 to accurately reflect repo state (e.g., `Code merged; v1.0.2 tag/final smoke pending` or whatever user decides for 1.0.2 disposition) and link this improvement plan under **Active plans**.
2. Clarify Phase 0 exit disposition in `docs/smoke-1.0.md` / `2026-11-16-athena-phase-0-foundation.md`:
   - Either execute remaining Phase 0 human/installer checks (`smoke-1.0.md:92-94`) and have user tag `v1.0.2`, or record an explicit user-approved waiver superseding `v1.0.2` into `1.1.0` (never silently leave contradictory statuses).
3. Record user approvals for **§2.2 (D-P1..D-P7)** in `contracts.md` and append dated `§15` change-log lines in the same commit as the corresponding implementation.
4. Run `sed 's/\r$//' docs/superpowers/doc-lint.sh | bash` (or `bash docs/superpowers/doc-lint.sh`) — must exit `0` (`doc-lint: clean`).

---

## 5. Ordered Execution Path & Release Gates

```text
5ba7411 (current HEAD)
  │
  ├─► [Gate 0] Task R-1 (Snapshot validation & fail-closed retention)
  │        └─► Task R-2 (Atomic ViewModel reload + outcome preservation + unavailable state)
  │
  ├─► [Finish Task 6] Task R-3 (Repair filter reset & debounce clear)
  │        └─► Task R-4 (Card <2-participant coverage visibility + Record policy)
  │        └─► Task R-5 (Stale Undo clear, undo error toast, RestorePerson copy)
  │        └─► Task R-7 (Docs/README plan-status & Phase 0 disposition) [parallelizable before R-6/T7]
  │
  ├─► ★ TASK 6 CHECKPOINT (Stop: verify on COPIED real data + card screenshot)
  │
  ├─► [Perf Triage] Task R-6 (50k row settlement + sync benchmark)
  │
  ├─► [Safe Data 7–10] Task 7 (Backups & staging)
  │        └─► Task 8 (Restore pipeline + Data page + 50k restore benchmark)
  │        └─► Task 9 (Format 2, v1 adapter, overflow-complete validator, optional CSV Payer)
  │        └─► Task 10 (Import pipeline + preview)
  │
  ├─► ★ TASK 10 CHECKPOINT (Stop: restore & import E2E on COPIED real data)
  │
  ├─► [P2 Optional] Task 11 (Startup recovery) ─► Task 12 (Keyboard & focus)
  │
  └─► ★ TASK 13 RELEASE GATE (1.1.0 release build, Inno installer, SHA-256 smoke-1.1.md, README)
```

### Release Gates & Required Exit Evidence

| Gate | Required Evidence Before Advancing |
|---|---|
| **Gate 0 (Safety)** | `R-1` and `R-2` tests pass (`corrupt/empty/dir/foreign` snapshot rejected, retention skipped; settlement overflow does not block startup, loop per frame, half-mutate `ViewModel`, or lose `ActionOutcome`); `fmt`, `clippy -D warnings`, `cargo test --locked` green. |
| **Task 6 Checkpoint** | `R-3..R-5` + `R-7` tests & `doc-lint.sh` green; manual verification on a **copied** real database with screenshot of settlement card and working repair filter links; zero real-data directories modified. |
| **Task 8 Gate** | All `§9.3` failure-injection seams (steps 1–8) + rejection matrix pass in WAL mode; measured `< 1 s` restore time on 50,000-row fixture recorded in commit body. |
| **Task 10 Checkpoint** | All `§10.1–§10.5` tests (Format 1 fixture, Format 2 round-trip, `ImportError::Overflow` prefix/per-person/debtor checks, rollback on late SQLite failure) pass; E2E restore & import demonstrated on copied real data. |
| **Release Gate (`1.1.0`)** | Cut-line Tasks 1–10 + `R-1..R-7` + 13 complete (plus Tasks 11–12 or explicit cut-line waiver in `README.md` & `smoke-1.1.md`); Windows CI green; `cargo build --release --locked` and Inno installer built; `docs/smoke-1.1.md` fully executed and checked off against the **recorded SHA-256** of final `athena.exe` and setup `.exe`; user performs `v1.1.0` tag and release publish. |


## 6. Final Codex Assessment of the Hydrogen Plan

Codex agrees with the priority order, double visibility-gate finding, coherent reload and preserved committed outcomes, and required Safe Data completion before release. This plan is a proposed supplement, not an amendment to canonical contracts or implementation authorization.

Corrections made after Hydrogen delivery:
- Corrected the v1 user-table list: migration 0001 has five tables and no templates table. Exclude SQLite internal tables when comparing user-table sets.
- Split multi-filter Cargo test commands into separate invocations; Cargo accepts one positional filter.
- Added missing imports to the AppCore regression example.
- Made canonical row/allocation coverage mandatory for existing same-day backups; identity alone leaves the safety hole open. Fresh verified pre-purge snapshots are the recommended simpler proof.
- Applied same-key retry suppression to required-query failures as well as optional settlement failures. Preserving committed outcomes must not leave a per-frame query/log loop.

Acceptance caveats for implementation:
- The invalid-existing-snapshot test exercises the fail-closed path. If the selected policy creates a new snapshot instead, separately assert the new file preserves every purged row and allocation, plus inject snapshot failure and prove no purge. Preserve the original invalid file.
- Code snippets are proposed examples referencing future fields and have not been compiled. Expand comment-only kittest examples into real interactions before claiming the tests exist. Use the actual ScreenEvent variant rather than introducing an undefined MonthEvent.
- R-2's arithmetic audit also owns src/application/services/reports.rs and src/domain/reports.rs plus affected callers. A checked MonthSummary constructor requires consumer changes and focused overflow tests. Distinguish pre-existing report arithmetic from the newly reproduced settlement refresh defect.
- Task 7 must use the canonical protected-path backup pruning API for any new prefix. Never prune a required in-progress recovery snapshot through an unprotected prune_prefix call.
- At Task 9, test temporary arithmetic prefixes in the same deterministic date/id order used by settlement, not only date-group endpoints or final totals. Specify trash validation under backup restore/reactivation separately: format-2 JSON excludes trash and cannot validate rows absent from its payload.
- D-P4 and stricter copied-data gates are proposed workflow recommendations, not retroactively approved policy. Product/format changes remain unapproved until selected by the user.

Artifact verification: Codex inspected the plan in sections against source and reran document lint after these corrections. No production code, tests, canonical contracts, release refs or user data changed. Runtime verification remains as attributed in the reviews; planning checks are not new runtime tests.
