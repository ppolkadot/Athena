# Prompt for Claude Opus 5.5 High

Act as an independent Athena reviewer and implementation planner. The user wants your own findings, a concrete implementation plan for the attached readiness and appearance requirements, and a reusable implementation prompt expressing your findings. Inspect this checkout and verify source before reaching conclusions.

Read these files:
- [Attached readiness and appearance review](../superpowers/specs/2026-11-16-athena-readiness-and-appearance-review.md)
- [Canonical contracts](../superpowers/specs/contracts.md)
- [Codex assessment and proposed implementation sequence](../superpowers/plans/2026-11-17-athena-native-appearance.md)
- [1.1 Safe Data plan](../superpowers/plans/2026-11-16-athena-1.1.0-safe-data.md) and [1.1 smoke record](../smoke-1.1.md)
- [Previous review remediation](../superpowers/plans/2026-11-16-athena-review-improvement-plan.md)

Evidence on 2026-11-17: GitHub main and v1.1.0 resolve to bc9f18f6557f51db31adf223e4d33591d842ad93. The release is public, branch CI passed (44089921766), tag checks and release compilation passed, and the tag installer job (44089925770) failed at Chocolatey because innosetup 6.7.3 was unavailable. GitHub's listed digests and sizes match the smoke record, which still leaves native manual verification open. Open issues query returned none. No v1.0.2 tag is in this clone or the queried remote, and the waiver is documented. Current checkout is based on that exact SHA with Codex's working-tree packaging fix, plan and prompt. Test result on the source baseline was 432 total passes and two ignored. The workflow now resolves the official Inno Setup asset and checks its SHA-256. Inspect the working diff and treat the hosted CI run as historical.

Independently assess whether the old gaps still apply, check adjacent correctness issues especially startup recovery, import/restore boundaries and ViewModel cache behavior, and cross-check the packaging fix. Give file/line evidence and distinguish test evidence from inspection and proposals. Read Cargo.lock and relevant native renderer/media APIs before endorsing the spike. Explicitly assess format 4, asset retention across backup/restore, Windows accessibility signals, safe animation/texture limits, DPI, draft handling, and preservation of Rust/egui. Explain disagreements with Codex and decide what gates the spike versus 1.3. D9 assigns 1.2 Daily Speed, 1.3 Appearance and 1.4 Goals/Closings. Preserve that scope unless explicitly presenting an alternative for user consideration.

Write one Markdown file at docs/reviews/2026-11-17-opus-athena-appearance-analysis-and-prompt.md. Include your own prioritized findings, proposed implementation plan with dependencies/files/tests/checkpoints, and a clearly headed standalone implementation prompt that cites your findings. Return the path and any important disagreements or confirmed bugs. You own only the requested review output file. Preserve all parent changes. Use fixtures, no live financial data, no commits, tags, pushes or production edits. No subdelegation.
