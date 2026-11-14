# Athena roadmap revision briefs

**Date:** 2026-11-13
**Status:** Product decisions confirmed by the user on 2026-11-13. These revision briefs define the approved direction; detailed task/example integration is still required before execution.
**Observed baseline:** `3b934e3291bb572813ba5c91850d67ae285169e0`, local `main`, remote `main`, and remote `v1.0.1` matched when inspected.

## Purpose and document authority

Capture the repository review as one brief per version, including correctness fixes, UX requirements, visual direction, verification, and the decisions needed before rewriting the implementation plans.

- [1.0.1: shipped foundation and remaining patch work](2026-11-13-athena-1.0.1-revision-brief.md)
- [1.0.2: contrast and foundation follow-up](2026-11-13-athena-1.0.2-revision-brief.md)
- [1.1.0: reliable daily workflows and Apple/Onyx design](2026-11-13-athena-1.1.0-revision-brief.md)
- [1.2.0: goals and trustworthy closing snapshots](2026-11-13-athena-1.2.0-revision-brief.md)

The original [1.0.0 plan](../plans/2026-11-11-athena-1.0.0.md) remains historical baseline documentation. The existing 1.0.1, 1.1.0, and 1.2.0 implementation plans link to these briefs and distinguish shipped history from required revisions. Older task snippets still need reconciliation; a reference to this brief alone does not make them executable.

Current source is evidence of implementation, not proof that every release check passed. In particular, the smoke record includes checks performed before the most recent font/layout rebuild; those observations must not silently be attributed to a different executable hash.

## Confirmed product direction

- Windows-first, offline Rust/egui application; retain the existing architecture and SQLite storage.
- Main UI and controls use Salted Flutters Polaris Semibold. Titles use the real Bold face. Amounts and table dates use frozen-tabular Semibold.
- Onyx means a dark, polished material aesthetic, not a particular external product.
- Explore Apple-inspired hierarchy, restrained materials, and purposeful animation while keeping financial content clear and quick entry fast.
- Keep English and Brazilian Portuguese, exact integer cents, and a complete light theme.
- Goal contributions remain independent from transactions and tracked balance.

## Confirmed decisions

### D1 — Scope of the visual work

**User choice:** keep the patch focused. The broader Apple/Onyx redesign and animations belong in 1.1. Current typography and category truncation are already implemented; do not schedule them again as new features.

### D2 — Settlement with incomplete entries

**User choice:** show complete-entry balances, clearly label exclusions, and offer a repair filter. Unknown payers and missing allocations must never be silently assigned to a person or included on only one side of the calculation.

### D3 — Template fidelity

**User choice:** preserve the full supported form state, including payer and shared/custom responsibility splits. The 1.1 brief gives the required schema and amount-edit constraints.

### D4 — Goal completion semantics

**User choice:** keep `Funded` calculated from contributions and `Completed` as a deliberate user action. Automatic funding does not rewrite the workflow status.

### D5 — Version for remaining patch corrections

**User choice:** put remaining contrast and foundation corrections in 1.0.2, preserving the published 1.0.1 tag. Base 1.1 on the completed 1.0.2 foundation. The separate 1.0.2 brief carries that work rather than rewriting shipped history.

No product questions remain open from this review. Exact material rendering, icon assets, and visual compositions still require normal implementation-time design validation; this is not approval of an unseen mockup or a measured performance claim.

## Technical corrections that do not require a product choice

- Make backup replacement recoverable across every swap/reopen state.
- Validate imported domain invariants, format compatibility, references, and aggregate bounds before committing.
- Give closing snapshots sufficient data to reproduce their insight and detect changes to both comparison months.
- Supply selected-detail read models and invalidate cached labels, dates, and aggregates correctly.
- Update release/version/installer documents together and verify the actual artifact being distributed.

## Incorporation order

1. Use the recorded D1–D5 answers and the concrete behavior in the version briefs as the requirements for task expansion.
2. Reconcile the main design spec and the designer brief with the confirmed typography and approved design scope. The brief must remain under 4,000 characters; if its canonical text changes, update its mirrored appendix as well.
3. Create the 1.0.2 implementation plan from its brief; preserve the shipped 1.0.1 history.
4. Rewrite 1.1 task contracts, code examples, tests, migrations, and release checks around the resolved decisions. Include the full-fidelity template schema and document-format-2 adapter in that pass.
5. Rewrite 1.2 against the resulting 1.1 contracts, including its document-format compatibility rules.
6. Cross-check every named action, DTO, cache field, migration, test fixture, and exporter signature before marking a plan ready to execute.

Creating or incorporating these documents does not execute the feature plans or move release tags. Git commits and publication are separate actions.

## Design references

- [Apple: materials](https://developer.apple.com/design/human-interface-guidelines/materials)
- [Apple: motion](https://developer.apple.com/design/human-interface-guidelines/motion)
- [Apple: typography](https://developer.apple.com/design/human-interface-guidelines/typography)
- [W3C: text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)

The timing values and visual treatments in these briefs are Athena proposals, not requirements quoted from Apple.
