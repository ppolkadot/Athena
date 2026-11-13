# Athena — Designer Brief

## Product
Athena is a local-first desktop finance app for one person or a household. It runs offline on Windows, stores data in one SQLite file, and focuses on fast expense logging and clear monthly review. Money is BRL, exact cents. UI is English and Brazilian Portuguese at runtime. Pre-build; redesign shapes 1.0–1.2.

## Users and feel
Individual tracking daily spending; couple sharing expenses with per-person responsibility; desktop, keyboard-first. Light, quiet, fast. Entry faster than a notes app. No dashboard clutter.

## Core jobs
1. Log an expense in under 10 seconds.
2. See where the month's money went.
3. Browse the year by month; drill into a month/day.
4. Know who owes whom (1.1).
5. Track goals without mixing them into expenses (1.2).
6. Close a completed month and compare later (1.2).
7. Trust local data, backups, exports.

## Screens
- Onboarding: 4 steps.
- Month: stepper, all-time balance, income/expenses/result cards, insight, filters, day-grouped register, right rail with category bars, person totals, settlement, closing card.
- Calendar: 12 month cards, closed dot, day drill-down (1.1).
- Goals (1.2), Trash (1.1), Settings.
- Quick add modal: amount first, then title/date/type/category/payer/person/split; Enter saves, Esc closes.
- Global: sidebar (Month, Calendar, Trash, Goals, Settings), toasts/undo, confirm dialogs, empty states.

## Type
Target: Salted Flutters Polaris (github.com/ppolkadot/Salted-Flutters-Polaris). Geometric sans, 18 styles Thin–Black plus italics.

- Use for wordmark, display, headings, month labels, onboarding, empty states. Validate body at 13–15 px in the dense register.
- Money/dates use tabular figures (`tnum`). Verified: `R$`, digits, punctuation, Portuguese diacritics. Right-align amounts.
- Fallback while under development: Segoe UI. Consolas only if mono needed.
- Sizes: 15 body, 13 secondary, 20 section, 28 heading, 34 display. Keep 8 pt grid.
- Ship 3–5 weights, Latin subset, not all 18. Each OTF ~3.4 MB. Pin a commit.
- License allows commercial use and app embedding. Keep `LICENSE.txt` with redistributed files; preserve metadata.

## Visual baseline — open to redesign
Dark default near-black; light is a full peer.
bg `#050608` / `#F6F7FB`; panel `#0A0C10` / `#FFFFFF`; card `#0F1218` / `#FFFFFF`; border `#232A36` / `#E2E5EE`; text `#ECEEF4` / `#141821`; muted `#8B95A7` / `#5C6476`; accent `#6C8CFF` / `#4F6BFF`; positive `#3DDC97` / `#1F9E6E`; negative `#FF6B7A` / `#D6455A`; warning `#F5C451` / `#B07B12`.

8 pt grid; cards 16 radius/1 px border/16 padding; buttons 10 radius/36 height; content 1100 max; sidebar 220; window 1280×820. Motion 120–180 ms ease, fade + 8 px slide. Hover changes border/background only.

Color never sole signal: expense `−`+label, income `+`+label, refund `↩`+label; selected month underline+bold; closed dot+tooltip.

## Must not break
- Windows desktop, egui 0.37, offline, no mandatory network.
- Exact BRL `R$ 1.234,56`; no implied rounding; right-aligned tabular amounts.
- Keyboard-first: `Ctrl+N`, `Enter`, `Esc`, visible focus.
- Shared expense shows responsibility vs payment; never double-count.
- English and pt-BR fit without truncation; runtime switch.
- Contrast holds for body, muted, accent, positive/negative in both themes.
- No accounts, transfers, bank sync, budgets, recurring rules, AI advice, attachments (1.3+).

## Scope and deliverables
Redesign visual language, type, spacing, components, iconography, and layout within egui feasibility; flows, keeping entry fast; dark/light pair; future states: settlement, one-line entry, trash, insights, templates, duplicate warnings, XLSX, goals, closings.

Deliver: flows/wireframes for Month, Calendar, Quick add, Settings, Goals, Trash; hi-fi dark/light for 1.0 and key 1.1/1.2 states; Polaris type specimen including dense tables and small sizes; component library with default/hover/focus/active/disabled/error/empty; motion and accessibility notes; handoff tokens.
