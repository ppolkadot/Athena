# Assessed review synthesis and Google Hydrogen handoff

Date: 2026-11-16. Head: 5ba7411. Author: Codex, assessing its review plus Claude Opus 5.5's independent analysis.

## Inputs and requested output

Read these files in order:
1. docs/superpowers/specs/contracts.md (binding authority; D9 scope split).
2. docs/reviews/2026-11-16-codex-handoff-opus-5.5.md.
3. docs/reviews/2026-11-16-opus-5.5-analysis.md.
4. This assessment, plus active Phase 0 and Safe data plans.

User requested the sequence Codex analysis -> Opus independent review -> Google Hydrogen improvement plan based on all three judgments. This is analysis/planning only. No implementation, commits, tags, pushes, release changes, real-data access, or edits to existing plans/contracts are authorized by this handoff. You own only docs/superpowers/plans/2026-11-16-athena-review-improvement-plan.md. You are not alone; preserve other edits. No subdelegation.

## Agreements after assessing Opus

- Retention safety (Codex 1 / O-1) is the first priority. Codex reproduced corrupt-file acceptance in this checkout; Opus separately reproduced valid foreign SQLite acceptance in a scratch copy. Structural SQLite integrity alone is insufficient. Both findings share one root cause and should be one task.
- Repair filter reset (Codex 2) is a confirmed defect. Clear person, shared, category, kind and text filters plus search input and pending debounce before applying issue/until. Add actual query and UI tests for Shared/Person + Unassigned and pending search reintroduction.
- O-2 is a substantive new reliability defect. Codex inspected app/mod.rs::apply and view_model.rs::reload and agrees: mutation commits before reload; reload updates fields incrementally and propagates settlement error. This assessment is code inspection corroborating Opus's runtime probe, not another independent reproduction. A committed mutation must retain its success outcome/inverse even if subsequent reads fail. Initial screen load must not depend on optional settlement succeeding. Avoid repeated full-history reload/logging on every rendered frame.
- No 1.1 release until required Tasks 7-10 and 13 pass. Missing backup/restore/import mid-plan is expected unfinished scope, not itself proof that Tasks 1-6 were wrongly implemented.
- Plan README claims v1.0.2 was shipped with a tag that does not exist locally or on the queried remote. Record what is actually known and leave tag creation to user. Phase 0 prerequisite and Task 6 copied-real-data checkpoint need explicit disposition/evidence. Do not manufacture historical approval or CI results.
- D9 puts daily speed in 1.2, appearance in 1.3, goals in 1.4; preserve that split.

## Corrections and qualifications to Opus recommendations

1. O-1 proposed snapshot rule is not sufficient or minimal as written. Row IDs alone cannot prove identical/recoverable data and allocations. A same-day snapshot can have the row live rather than deleted and still be recoverable. Requiring deleted_at NOT NULL in the backup is unnecessarily strict. Exact latest schema is also a proposed policy, not intrinsically required if an older supported Athena snapshot is validated and restorable. Prefer a fresh verified pre-purge snapshot for a nonempty purge batch, before destruction, with explicit naming/retention/recovery support. If reusing snapshots, specify provenance and canonical row/allocation coverage, not merely IDs. Use fixture tests. Fail closed on any backup failure.
2. O-2: setting settlement=None with only a log would silently hide financial coverage and conflate unavailable with absent. Define a localized explicit unavailable/error state, and a retry policy that runs on relevant data/month change or deliberate retry. Stage read fields before swapping coherent state. Distinguish write failure from committed-but-refresh-failed outcomes generally, not only settlement overflow. Do not duplicate-save on retry.
3. Import overflow validation must agree with actual consumers and cumulative month-end cutoffs. A final lifetime total alone can miss an earlier prefix overflow that later refunds cancel. Check relevant per-person paid/owed, net balances, debtor negation and existing aggregate budgets, including trash rows that might later be restored; define the policy rather than inventing a new arbitrary money limit.
4. O-3 is chiefly an interim-build/release sequencing gate. Existing plan already requires restore before release. Do not call that alone a new shipped regression. Recommend disabling destructive automatic retention until recovery exists if intermediate builds will be used; keep proposal status explicit.
5. O-4 (historical/future Record): today-prefill follows the frozen contract. This is a product semantics issue, not literal noncompliance. Current-month-only Record still includes later-in-current-month transactions because cutoff is month end; it does not fully solve future-dated entries. Propose a clear reviewable policy, preserve historical as-of display and explicit draft date. Do not silently backdate or auto-record. User has not approved a new policy.
6. O-5 narrowed: single-row PurgeTransaction has toast=None and can leave stale Undo. EmptyTrash has Some(TrashEmptied), so it already replaces the toast. Correct any broader claim. Test actual reachable interaction and localized feedback on undo failure.
7. O-6 interim format-1 settlement exports are a compatibility hazard to close in Task 9; do not claim already-distributed user exports exist without evidence. Do not weaken genuine format-1 validation speculatively. A future adapter for interim files would require an explicit product/compatibility decision.
8. O-7 CSV direction is a useful improvement; adding payer/from changes an external report schema. Define compatibility and localized header/formula-safety tests. Code versus localized kind is a contract ambiguity; current tests intentionally expect localized labels.
9. O-8 performance is unmeasured. Benchmark with a generated large fixture first and optimize only if needed. Do not add broad cache refactors just because of code inspection. O-9 targeted restore-name error copy can be a small usability task.
10. Opus's blanket statement that no internal build may touch real data is its risk recommendation, not an approved project policy. Use copies for checkpoint work and say what is destructive. Keep release gates and verification limits precise.

## Desired Hydrogen output

Provide your own judgment: accept/reject/qualify each major finding with reasons; a consensus/disagreement ledger; a plan that supplements rather than silently replaces active Safe data tasks; exact owning files, dependencies, compact commit units, test-first acceptance cases, concrete commands, checkpoints, non-goals and release gates. Link existing Task 7-10 requirements rather than inventing conflicting DTOs or pipelines. Freeze changes must be proposed edits with a section-15 log in the eventual implementation commit. Distinguish recommendations from choices requiring user approval. Include an ordered execution path from current head through completed 1.1 and evidence required to mark each phase done. No implementation in this run.

Use the local writing-plans skill C:/Users/PC/.codex/skills/writing-plans/SKILL.md for plan structure. Keep it actionable, with code/test examples for new regression tasks; canonical existing tasks can be referenced by exact section. Self-check cross-references, undefined types, scope, unsupported consensus, and contradictory acceptance criteria before returning.

## Validation provenance

Codex in current checkout: 320 existing tests passed; cargo fmt --check; strict Clippy; doc lint passed. Two bug probes passed asserting erroneous behavior, source retained in docs/reviews/2026-11-16-review-probes.rs.txt. Opus ran three separate scratch-copy probes; two passed reproducing faults, third returned expected settlement errors and failed its final empty-register assertion, demonstrating partial update. Opus did not rerun full checks. No native UI/scaling/accessibility, release artifacts, copied-real-data checkpoint, remote CI, or performance measurements were verified. Since review, only docs/reviews files exist as untracked changes.
