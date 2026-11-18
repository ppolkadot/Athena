# Definition of done

A task is done when all of these hold:

- Tests were written first and pass.
- Task cites its `contracts.md` section.
- CI is green.
- `cargo fmt --check` is clean.
- `cargo clippy --all-targets --locked -- -D warnings` is clean.
- No new visible string literal outside `src/i18n`.
- No SQL outside `src/infrastructure`.
- Every new write path to stored data goes through `AppAction` and `Store::apply`.
- `integrity::check` is clean at the end of every new repository test.
- A new screen has a UI test that drives it, and a snapshot if it is a top-level screen.
- One manual check is noted in the commit message.
- Release checks are bound to the recorded final artifact SHA-256 hashes.

Before a release, also run `docs/superpowers/doc-lint.sh` and the release smoke checklist (`docs/smoke-1.0.md` or the release's `docs/smoke-1.x.md`).

## 1.2.0 acceptance references

Release 1.2.0 (schema 4, format 3; contract §14.1) is accepted when, in addition to the rules above:

- The named regressions of the [roadmap](superpowers/plans/2026-11-18-athena-stalled-ui-roadmap-plan.md#120-daily-speed) exist and pass, including the contrast, icon, migration, format-3, template, dirty-draft, one-line, duplicate, day drill-down, insight and XLSX tests.
- All shipped fixtures (`format1-1.0.2.json`, `format2-1.1.0.json`, `format3-1.2.0.json`) import through explicit version adapters and keep their LF line endings.
- Changed EN/Dark and PT-BR/Light snapshots were inspected for clipping and amounts.
- `docs/smoke-1.2.md` separates automated checks performed from interactive Windows checks still pending; manual checks are never marked done unless executed against the recorded final artifacts.
