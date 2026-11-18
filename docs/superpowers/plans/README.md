# Athena Implementation Plans

> **Rule:** Tasks reference [contracts.md](../specs/contracts.md); they do not restate it.

## Active plans

| Plan | Scope | Status |
|---|---|---|
| [2026-11-16-athena-phase-0-foundation.md](2026-11-16-athena-phase-0-foundation.md) | Finish 1.0.2, Windows CI, doc consolidation, Settings/app module split | Code merged; tag and remaining 1.0.2 manual checks waived in the [smoke record](../../smoke-1.0.md) |
| [2026-11-16-athena-1.1.0-safe-data.md](2026-11-16-athena-1.1.0-safe-data.md) | 1.1.0 Safe data (trash/undo, settlement with paybacks, backups/restore, format-2 import/export) | Released as `v1.1.0` (manual smoke items pending; see `docs/smoke-1.1.md`) |
| [2026-11-16-athena-review-improvement-plan.md](2026-11-16-athena-review-improvement-plan.md) | Review remediation (`R-1..R-7`) and Safe Data (`Tasks 7–10`) integration | Active |
| [2026-11-18-athena-1.2.0-daily-speed.md](2026-11-18-athena-1.2.0-daily-speed.md) | 1.2.0 Daily speed (`0004_templates.sql`, format 3, one-line entry, XLSX, tokens/icons) | Draft executable plan (awaiting §14.1 freeze) |
| [2026-11-18-athena-glass-spike.md](2026-11-18-athena-glass-spike.md) | S1/A1 isolated native `wgpu` glass spike | Implemented (see [design record](../../design/athena-glass-spike.md)) |
| [2026-11-17-athena-native-appearance.md](2026-11-17-athena-native-appearance.md) | Native appearance (`0005_appearance.sql`, format 4) and 1.3 implementation | Proposed, after 1.2 for persistence |

## Shipped and historical plans

| Plan | Scope | Status |
|---|---|---|
| [2026-11-11-athena-1.0.0.md](2026-11-11-athena-1.0.0.md) | Athena 1.0.0 foundation | Shipped (`v1.0.0`) |
| [2026-11-13-athena-1.0.1.md](2026-11-13-athena-1.0.1.md) | Embedded Polaris fonts and overflow fixes | Shipped (`v1.0.1`) |
| [2026-11-13-athena-1.0.2.md](2026-11-13-athena-1.0.2.md) | Contrast tokens, Bold weight normalization, adaptive layouts | Code merged; tag and remaining 1.0.2 manual checks waived in the [smoke record](../../smoke-1.0.md) |
| [historical/2026-11-11-athena-1.1.0.md](historical/2026-11-11-athena-1.1.0.md) | Original 1.1.0 draft | Superseded |
| [historical/2026-11-11-athena-1.2.0.md](historical/2026-11-11-athena-1.2.0.md) | Original 1.2.0 goals/closings draft (`0003` / format 3) | Superseded — Daily Speed (`0004_templates`, format 3) is `1.2.0`; Goals & closings moved to `1.4.0` (`0006_goals`, `0007_closings`, format 5) per [contracts.md](../specs/contracts.md) D9 |
