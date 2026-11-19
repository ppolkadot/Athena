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

## 1.3.0 acceptance references

Release 1.3.0 (schema 5, format 4; contract §14.2) is accepted when, in addition to the rules above:

- The named regressions of the [roadmap](superpowers/plans/2026-11-18-athena-stalled-ui-roadmap-plan.md#130-native-appearance) exist and pass, including the `0005` migration, bad-appearance-row, apply/cancel/reset, platform-preference, media decode/ingest/quota, playback, scrim-contrast, keyboard and format-4 tests.
- All shipped fixtures (`format1-1.0.2.json`, `format2-1.1.0.json`, `format3-1.2.0.json`, `format4-1.3.0.json`) import through explicit version adapters and keep their LF line endings; formats 1–3 clear the appearance row.
- Changed and new snapshots (`snapshot_settings_appearance` solid and bright-wallpaper frames, dark/light shell) were inspected in EN/Dark and PT-BR/Light for clipping and contrast.
- Production Glass is not selectable unless every separate hardware gate in the [glass spike](design/athena-glass-spike.md) has a recorded real measurement.
- `docs/smoke-1.3.md` separates automated checks performed from interactive and hardware Windows checks still pending, including open Glass gates; manual checks are never marked done unless executed against the recorded final artifacts.

## 1.4.0 acceptance references

Release 1.4.0 (schema 7, format 5; contract §14.3) is accepted when, in addition to the rules above:

- The named regressions of the [roadmap](superpowers/plans/2026-11-18-athena-stalled-ui-roadmap-plan.md#140-goals-and-closings) exist and pass, including the goal lifecycle, contribution overflow, progress, schedule, goal detail refresh, archive/restore focus, closing comparison, label-rename, atomic replacement, Close/Recalculate flow, format-5 and workbook tests.
- All shipped fixtures (`format1-1.0.2.json` through `format5-1.4.0.json`) import through explicit version adapters and keep their LF line endings; formats 1–4 yield empty goals and closings.
- Goals, Month closing card and Calendar marker snapshots were inspected in EN/Dark and PT-BR/Light for clipping, amounts and the saved-versus-live distinction.
- Goal and contribution writes leave ledger and settlement figures unchanged; reads still go through `live_transactions`.
- `docs/smoke-1.4.md` separates automated checks performed from interactive and hardware Windows checks still pending; manual checks are never marked done unless executed against the recorded final artifacts.
