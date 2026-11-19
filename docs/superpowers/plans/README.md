# Athena Implementation Plans

> **Rule:** The [contracts](../specs/contracts.md) govern implementation. The roadmap supplies decisions to record in the future release sections before feature work.

## Start here

[Athena implementation roadmap for 1.2.0–1.4.0](2026-11-18-athena-stalled-ui-roadmap-plan.md) is the ordered handoff from `v1.1.1` (`3101d73`). It identifies the code seams, freeze decisions, UI behavior, and validation for Daily Speed, Native Appearance, and Goals and Closings.

## Active plans

| Plan | Scope | Status |
|---|---|---|
| [2026-11-18-athena-1.2.0-daily-speed.md](2026-11-18-athena-1.2.0-daily-speed.md) | Daily Speed (`0004_templates.sql`, format 3, Quick Add, day drill-down, XLSX, tokens/icons) | Tasks 1–8 pending. Start with the roadmap's §14.1 freeze task. |
| [2026-11-17-athena-native-appearance.md](2026-11-17-athena-native-appearance.md) | Native Appearance (`0005_appearance.sql`, format 4, Settings Appearance, wallpaper) | A2–A5 pending after verified 1.2. Production Glass has separate open gates. |
| [Roadmap 1.4.0 milestone](2026-11-18-athena-stalled-ui-roadmap-plan.md#140-goals-and-closings) | Goals and Closings (`0006_goals`, `0007_closings`, format 5) | Tasks G1–G6 pending after verified 1.3. |

## Shipped and historical plans

| Plan | Scope | Status |
|---|---|---|
| [2026-11-11-athena-1.0.0.md](2026-11-11-athena-1.0.0.md) | Athena 1.0.0 foundation | Shipped (`v1.0.0`) |
| [2026-11-13-athena-1.0.1.md](2026-11-13-athena-1.0.1.md) | Embedded Polaris fonts and overflow fixes | Shipped (`v1.0.1`) |
| [2026-11-13-athena-1.0.2.md](2026-11-13-athena-1.0.2.md) | Contrast tokens, Bold weight normalization, adaptive layouts | Code merged; tag and remaining 1.0.2 manual checks waived in the [smoke record](../../smoke-1.0.md) |
| [2026-11-16-athena-phase-0-foundation.md](2026-11-16-athena-phase-0-foundation.md) | Finish 1.0.2, Windows CI, doc consolidation, Settings/app module split | Code merged. See [1.0 smoke record](../../smoke-1.0.md) for waiver. |
| [2026-11-16-athena-1.1.0-safe-data.md](2026-11-16-athena-1.1.0-safe-data.md) | Trash/undo, settlement and paybacks, backups/restore, format 2 | `v1.1.0` and patch `v1.1.1` published. Interactive [1.1 smoke](../../smoke-1.1.md) remains open. |
| [2026-11-16-athena-review-improvement-plan.md](2026-11-16-athena-review-improvement-plan.md) | Review remediation (`R-1..R-7`) and Safe Data integration | Implemented on main by `v1.1.1`. |
| [2026-11-18-athena-glass-spike.md](2026-11-18-athena-glass-spike.md) | S1/A1 isolated native `wgpu` glass spike | Spike implemented. See [measured results and open production gates](../../design/athena-glass-spike.md). |
| [historical/2026-11-11-athena-1.1.0.md](historical/2026-11-11-athena-1.1.0.md) | Original 1.1.0 draft | Superseded |
| [historical/2026-11-11-athena-1.2.0.md](historical/2026-11-11-athena-1.2.0.md) | Original 1.2.0 goals/closings draft (`0003` / format 3) | Superseded — Daily Speed (`0004_templates`, format 3) is `1.2.0`; Goals & closings moved to `1.4.0` (`0006_goals`, `0007_closings`, format 5) per [contracts.md](../specs/contracts.md) D9 |
