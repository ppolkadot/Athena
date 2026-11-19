# Athena — Designer Brief

## Product
Athena is a local-first desktop finance app for one person or a household. It runs offline on Windows, stores data in one SQLite file, and focuses on fast expense logging and clear monthly review. Money is BRL, exact cents. UI is English and Brazilian Portuguese at runtime. Shipped through 1.4 (Goals and closings).

## Users and feel
Individual tracking daily spending; couple sharing expenses by responsibility; desktop, keyboard-first. Light, quiet, fast. Entry faster than a notes app. No dashboard clutter.

## Core jobs
1. Log an expense in under 10 seconds.
2. See where the month's money went.
3. Browse the year by month; drill into a month/day.
4. Know who owes whom (1.1).
5. Track goals without mixing them into expenses (1.4).
6. Close a completed month and compare later (1.4).
7. Trust local data, backups, exports.

## Screens
- Onboarding: 4 steps.
- Month: stepper, all-time balance, income/expenses/result cards, insight, filters, day-grouped register, right rail with category bars, person totals, settlement, saved closing card (changed marker).
- Calendar: 12 month cards with saved/changed closing markers, day drill-down.
- Goals (Ctrl+5): lifecycle tabs, list and detail, contributions, Funded apart from Completed. Trash, Settings (Appearance: wallpaper, text size).
- Quick add modal: amount first, then title/date/type/category/payer/person/split; Enter saves, Esc closes.
- Global: sidebar (Month, Calendar, Goals, Trash, Settings; Ctrl+1–5 hints), toasts/undo, confirm dialogs, empty states.

## Type
Shipped: Salted Flutters Polaris (github.com/ppolkadot/Salted-Flutters-Polaris), embedded Latin subsets.

- `Semibold` for body and controls; real `Bold` (`usWeightClass = 700`) for wordmark, section, heading, display, and month titles.
- Money/dates use frozen `tnum` `Tabular Semibold`. Right-align amounts.
- Glyph fallbacks: `Segoe UI` and `Segoe UI Symbol` for symbols beyond the Latin subset.
- Sizes: 15 body, 13 secondary, 20 section, 28 heading, 34 display. Keep 8 pt grid.
- Keep `LICENSE.txt` beside redistributed fonts.

## Visual baseline — open to redesign
Dark default near-black; light is a full peer.
bg `#050608` / `#F6F7FB`; panel `#0A0C10` / `#FFFFFF`; card `#0F1218` / `#FFFFFF`; border `#232A36` / `#E2E5EE`; text `#ECEEF4` / `#141821`; muted `#8B95A7` / `#5C6476`; accent `#6C8CFF` / `#3F55C6`; positive `#3DDC97` / `#147D52`; negative `#FF6B7A` / `#B83247`; warning `#F5C451` / `#7A5B00`.

8 pt grid; cards 16 radius/1 px border/16 padding; buttons 10 radius/36 height; content 1100 max; sidebar 220; window 1280×820. Motion 120–180 ms ease, fade + 8 px slide. Hover changes border/background only.

Color never sole signal: expense `−`+label, income `+`+label, refund `↩`+label; selected month underline+bold; closing marker is icon+words.

## Must not break
- Windows desktop, egui 0.37, offline.
- Exact BRL `R$ 1.234,56`; no implied rounding; right-aligned tabular amounts.
- Keyboard-first: `Ctrl+N`, `Enter`, `Esc`, visible focus.
- Shared expense shows responsibility vs payment; never double-count.
- English and pt-BR fit without truncation; runtime switch.
- Contrast holds for body, muted, accent, positive/negative in both themes.
- Wallpaper keeps text 4.5:1, cards opaque; Reduced motion/High contrast win.
- No accounts, transfers, bank sync, budgets, recurring rules, AI advice, attachments.

## Scope and deliverables
Redesign visual language, type, spacing, components, iconography, and layout within egui feasibility; keep entry fast; dark/light pair; future states: settlement, one-line entry, trash, insights, templates, duplicate warnings, XLSX, goals, closings.

Deliver: flows/wireframes for Month, Calendar, Quick add, Settings, Goals, Trash; hi-fi dark/light for 1.0 and key 1.1/1.2 states; Polaris type specimen including dense tables and small sizes; component library with default/hover/focus/active/disabled/error/empty; motion and accessibility notes; handoff tokens.
