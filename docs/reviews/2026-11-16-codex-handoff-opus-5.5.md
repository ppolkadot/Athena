# Athena 1.1.0 review and handoff to Claude Opus 5.5

Date: 2026-11-16. Reviewer: Codex. Checkout: C:\Users\PC\athena.
Review head: 5ba7411 (main and cached origin/main). Initially clean tree.
Scope: current implementation through Safe data Task 6, foundation changes since f16e553; not a release approval. Do not implement fixes, tag, push, publish, or touch real user databases. You are not alone in this checkout; preserve other edits.

## Authority and current plan compliance

Read docs/superpowers/specs/contracts.md first, then plans/2026-11-16-athena-1.1.0-safe-data.md and plans/2026-11-16-athena-phase-0-foundation.md. Contracts D9 explicitly supersedes the broad old scope: 1.1 Safe data, 1.2 Daily speed, 1.3 Appearance, 1.4 Goals. Do not count deferred design, templates or XLSX as missing 1.1 requirements. The November 13 briefs remain rationale; their acceptance criteria still apply where contracts say so. The historical plan is not executable authority.

Commits fcaa837, 29054cb, e7a52d3, c95fed8, 778a7b6, 5ba7411 map to Tasks 1-6. Tasks 7-10 (manual backups, validated restore, format-2 export/adaptation, import) are not implemented; this is partial implementation, not proof of violating task order. Tasks 11-12 may slip only with disclosure. Task 13 release is outstanding. Cargo remains 1.0.2: appropriate during development, not a finished 1.1 release.

Prerequisite deviation: Phase 0 requires v1.0.2 tagged before module split / 1.1. Local tags only v1.0.0 and v1.0.1; git ls-remote --tags origin v1.0.2 succeeded with no result. No v1.0.2 tag exists on that queried remote. Do not create it: tagging is a user action. Live CI status and Task-6 copied-real-data screenshot/checkpoint have not been verified. All task checkboxes remain unchecked and plans/README status is stale. Do not infer checkpoint evidence from unit/snapshot tests.

## Findings (Codex)

1. P1, REPRODUCED: retention may permanently purge with an unusable existing daily backup. backup::daily_snapshot (src/infrastructure/db/backup.rs, path.exists branch) returns Ok(None) without validating the file. Store::run_startup_tasks treats this as permission to purge. Temporary integration probe wrote corrupt bytes at auto-2026-11-16.db, seeded one >30-day deleted row, ran startup, and observed trash count 1 -> 0. Corrupt backup failed integrity_ok. A directory at that name is another unvalidated case. Existing snapshot helper predated this feature, but retention introduces the destructive consequence. Require a usable Athena snapshot, and fail closed or create a fresh verified snapshot on rejection; test existing corrupt/empty/wrong-schema/directory cases.

2. P2, REPRODUCED state contradiction: UiState::set_repair_filter preserves shared_only and other narrowing filters. Choose Shared then Unassigned repair: repository requires >=2 allocations AND zero allocations, so the repair list cannot contain any row despite exclusion counts. Preserved search/category/person can similarly hide repairs. Test reproduced shared_only remaining true. Clear conflicting filters and pending search state on entering repair mode, or explicitly define and disclose intersection semantics. Current global exclusion links imply the former; verify that interpretation independently.

3. P2, code-inspection finding: src/ui/components/settlement_card.rs::show returns before rendering exclusions when balances.len() < 2. A household whose only historical expenses have no payer / no allocations gets no card and no repair route. The task explicitly says hide below two participants, so this is a contract/product defect as well as implementation behavior, not an unambiguous failure to follow the literal plan. Separate visibility of repair coverage from transfer eligibility and reconcile contract in the same future change. Invalid exclusions are logged but not displayed (contract explicitly permits logging); do not inflate that into an independent UI violation.

4. Documentation/readiness: restore and format-2 import/export are absent and must block 1.1 release, not be presented as existing features. README still describes 1.0, Ctrl+1..3, permanent deletion; update for current development/release with explicit distinction from shipped artifacts. Phase 0 status and missing tag need honest disposition, not rewriting historical evidence or backdating approval.

## Verification performed now

- cargo test --all-targets: PASS, 312 library tests + 1 design asset + 5 font + 2 query-boundary integration tests = 320. Snapshot tests passed against existing goldens; no interactive/native/visual inspection implied.
- cargo fmt --check: PASS.
- cargo clippy --all-targets -- -D warnings: PASS.
- Document lint with CRLF normalization: PASS (bash -c "cd docs/superpowers && sed 's/\r$//' doc-lint.sh | bash").
- Two temporary integration probes: PASS asserting the erroneous behavior (reproductions, not regression fixes). Source retained at docs/reviews/2026-11-16-review-probes.rs.txt. To reproduce copy into tests/review_probe.rs, run cargo test --test review_probe, then remove only that copy.
- No release build/installer verification, native scaling/accessibility testing, real-data demonstration, remote CI verification, or live user-data modification performed.

## Request for Opus

Independently inspect the project and diff, challenge these findings, look for adjacent regressions (migration safety, arithmetic, archived references, refresh/cache boundaries, keyboard/undo, export query boundaries, trash retention). Judge current architecture and plan readiness. Separate reproduced defects, inspection findings, specification issues, and unimplemented future tasks. Do not merely endorse Codex. Write docs/reviews/2026-11-16-opus-5.5-analysis.md, with prioritized file/line findings, evidence, disagreements, validation limits, and your overall view. You own only that analysis file. Do not change production code or plans. Avoid rerunning all checks unless needed for new evidence. Return the file path and concise synthesis for the next Google Hydrogen planning handoff.

## Next stage

After Opus completes, Codex must assess its findings and create a synthesis input for Google Hydrogen. Hydrogen should independently judge both reviews and write a concrete improvement plan with task dependencies, owners/files, acceptance tests, release gates, unresolved choices, and explicit attribution of agreements/disagreements. Analysis/planning only; no implementation is authorized by this request.
