# Athena Design

**Date:** 2026-11-11
**Status:** Proposed. Revised after specification review and architecture review (2026-11-12).
**Scope:** 1.0.0, 1.0.1, 1.1.0, 1.2.0
**Source:** `ATHENA_PROJECT_OVERVIEW.md` + design conversation + review

## 1. Intent

Athena is a local-first desktop finance app for one household: you and your partner.

Core loop:

1. Log an expense or income in seconds.
2. See the month register: every entry, totals, category split, person split.
3. Browse months in a calendar. Click a month. See that month's log.

Clean. Quiet. Fast. No cloud. No accounts. No ceremony.

## 2. Why

Existing finance apps are heavy. The wanted experience is simple: record spending as it happens, then review the month. The calendar is the browsing surface, not a date-picker ornament.

Success is emotional and practical: opening Athena feels light, logging takes seconds, the month view answers "where did the money go", and the data is still there next year.

## 3. Goals

- Log a transaction in under 10 seconds.
- One screen per job: month, calendar, trash, goals, settings.
- Exact money. No floating point in storage or reports.
- Shared expenses never double-count.
- Payer recorded from the first release so settlement stays accurate after upgrades.
- Full offline operation.
- Export and backup that the user can trust and verify.
- Dark theme default. Light theme available.
- English and Brazilian Portuguese UI, switchable at any time.
- Keyboard-first quick entry.

## 4. Non-goals for 1.0.0

- Multiple bank accounts.
- Transfers between accounts.
- Automatic bank import.
- Synchronization.
- Online accounts or auth.
- Mobile.
- Budgets or envelopes.
- Recurring rules.
- Savings goals. Planned 1.2.0.
- Attachments. Planned 1.3.0+.
- Multi-currency.
- AI advice.

## 5. Anti-goals

- Entry slower than a notes app.
- Dashboard clutter: charts that do not answer a question.
- Double-counted shared purchases.
- Floating-point money.
- Silent data loss.
- Any mandatory network call.
- Logging transaction titles or descriptions.
- Color-only meaning.

## 6. Constraints

- Rust, edition 2024, `rust-version = "1.95"` (eframe/egui 0.37 MSRV).
- `eframe` / `egui` 0.37 for UI.
- SQLite via `rusqlite` 0.42 with `bundled` feature.
- `rust_decimal` for parsing. Storage in `i64` cents. No rounding of user input.
- BRL only. Currency code `BRL`.
- Display format: `R$ 1.234,56`.
- Windows is the primary target. Linux/macOS should compile but are not tested in 1.0.
- Runtime works with no internet.
- No sensitive transaction text in logs.
- Portable release for 1.0: single `athena.exe`, no installer.

## 7. Decisions

### Product

- **One wallet.** No account entities in 1.0. Balance is derived: `start_balance + income - expenses + refunds`. Accounts and transfers deferred.
- **People.** `people` supports owner, partner, and other. Onboarding creates the owner and optional partner. Settings can add, rename, archive. Exactly one owner, enforced by a partial unique index. The owner cannot be archived.
- **Owner name is the source of truth.** The `profile` row does not store a display name. The owner `people` row holds it. Sidebar, settings, and transaction attribution read the same record.
- **Payer recorded in 1.0.** Every transaction has a nullable `payer_id`. Quick add defaults payer to the responsible person, or the owner when shared. The settlement card ships in 1.1, but 1.0 data is already complete.
- **Shared expense.** One transaction row, many allocation rows. Allocations are responsibility, not payment.
- **Person selector.** Quick add offers every active person, `Shared`, and `No one`. `Shared` splits across all active people, default evenly, with one editable amount per person. Two people gives the described 50/50 flow. Three or more gives an N-way split. Shared is hidden when fewer than two active people exist.
- **Split mode.** Transactions store `split_mode` (`even`, `custom`, or null) so editing a shared entry reopens in the mode it was saved with.
- **Rounding and splits.** `split_evenly` rejects empty weights and zero total weight. Proportional split floors each share. Remaining cents go to largest fractional remainders, ties by list order (owner first). Sum always equals total. Zero-cent allocations are allowed when explicitly entered.
- **Refunds.** Independent transaction type. No link to an original purchase in 1.0. Refunds use the same allocation validation as expenses. Reports subtract refunds from expenses. Refunds never count as income. UI labels the type with an icon and text, not color alone.
- **Transfers.** Deferred. Not a transaction type in 1.0.
- **Categories.** Seeded defaults. Each has `applies_to` (`expense`, `income`, `both`); pickers show only categories matching the selected type. Seeds: 10 expense/both categories plus `Salary` and `Other income` (`income`), so the income picker is never empty. Settings can set the scope when adding or editing a category. Rename allowed. Delete means archive. Picker hides archived. History keeps the current name.
- **Rename and archive semantics.** Names are resolved through the current row. Renaming a person or category changes the label on historical transactions. Archiving hides it from new entries and keeps it queryable for history and reports. This is deliberate: one source of truth, no duplicated snapshot strings.
- **Month.** Local calendar month, Monday-first. Stored dates are ISO `YYYY-MM-DD`.
- **Starting balance.** Separate from income. May be negative. Included in tracked balance. Never in month income.
- **Money input.** Accepts `45`, `45.90`, `45,90`, `1.234,56`, `R$ 1.234,56`, and a leading `-`. Rejects more than two decimal places. Never rounds silently.
- **Onboarding.** Four steps: welcome + language, name + partner, starting balance, ready. Optional fields skippable. No income requirement. No goal.
- **Language.** English and pt-BR. Persisted in `profile.language`. Default from system locale. Switchable in onboarding and settings.
- **Deletion in 1.0.** Hard delete with confirmation. 1.1 adds trash + undo.
- **Daily snapshot in 1.0.** On startup, at most once per day, `VACUUM INTO` `backups/auto-YYYY-MM-DD.db`, verify integrity, keep 7. Manual rotating backups and restore arrive in 1.1.
- **Payment method.** Not stored. Added later only with a feature that reads it, as a fixed list.
- **Portable mode.** If `athena.portable` sits next to the exe, data lives in `<exe dir>\data\`; otherwise `%APPDATA%\Athena\`.
- **JSON import.** Moved to 1.1 so the JSON export is a verifiable restore path.
- **Theme.** Dark default with a near-black canvas (`#050608`), light available as a full peer theme. Since 1.0.1, Athena embeds Salted Flutters Polaris subsets: Regular for text, Medium for buttons, Semibold for section/heading/display, and Tabular for money and dates. Segoe UI and Segoe UI Symbol remain glyph fallbacks; egui defaults cover other systems.
- **Export 1.0.** JSON profile and CSV transactions, manual and atomic. CSV scope is the selected month or all time. XLSX in 1.1. Rotating backups in 1.1.
- **Search.** Case- and accent-insensitive for Latin text, so `pao` finds `Pão`.
- **Category target.** `applies_to` is editable in settings; income categories are seeded.
- **Calendar 1.0.** Year grid of 12 compact month cards. Click a card opens the month register. Day drill-down in 1.1.

### Technical

- UI never issues SQL. App state is three parts: `Store` (owns the DB, exposes use cases, bumps `data_version` on every committed write), `ViewModel` (cached query results, reloaded only when `data_version` changes), `UiState` (screen, month, filters, dialogs). `AppCore` is a thin composition root that owns those three and exposes convenience methods for screens.
- `AppAction` is the only mutation path from the UI. `Store::apply(action)` returns an outcome and, from 1.1, the inverse action used by undo.
- Layering for reports: SQL aggregates only in `infrastructure/db`; pure result types and arithmetic in `domain`; composition in `application/services`.
- `TransactionsRepo::save_with_allocations` is the only write path for transactions and allocations. `infrastructure/db/integrity.rs::check` finds allocation sums that differ from the total; it runs in tests and at debug startup.
- From 1.1, all transaction reads go through the `live_transactions` view. Only trash/restore/purge code, the transaction write path, and JSON import touch `transactions` directly; an integration test enforces this.
- Single instance: an exclusive `fs4` lock (`FileExt::try_lock`) on `athena.lock` in the data dir (released by the OS on exit, no `unsafe`) blocks a second process from opening the same database.
- `ActionOutcome { inverse: Option<AppAction>, toast: Option<ToastKey>, refresh: RefreshScope }` is what `Store::apply` returns. Inverses: create -> delete, update -> update with the previous draft, delete -> restore (1.1).
- `ViewModel` holds all people and categories, archived included, for label lookup; only pickers filter to active rows. Screens receive `&ViewModel`, `&mut UiState`, and an action list, and never query the database from `show`; progress, staleness and resolved names are computed in `ViewModel::refresh`.
- Text search and duplicate-name checks are accent- and case-insensitive through one `fold` function (NFD, strip combining marks, lowercase), registered as a SQLite scalar function. SQLite `LIKE` and `COLLATE NOCASE` alone are ASCII-only.
- `VACUUM INTO` destinations are bound as parameters, never formatted into SQL.
- Exporters take an explicit `ExportScope { All, Month(YearMonth) }` and, where a date is needed, a `today` argument; none calls `Local::now()`.
- Timestamps (`created_at`, `updated_at`, `closed_at`, `deleted_at`) are UTC RFC 3339. `date` is a local calendar date with no zone.
- Every multi-row write runs in one SQLite transaction.
- Migrations use `PRAGMA user_version` plus embedded SQL files. No migration library.
- A database newer than the app supports fails with a clear error. It is never opened or mutated.
- Before applying a migration to an existing database, Athena writes `backups/pre-migration-vN-<timestamp>.db` with `VACUUM INTO` and verifies it with `PRAGMA integrity_check`. No raw file copies of a live WAL database.
- A failed migration rolls back and leaves the previous schema and version intact.
- Money is `Money(i64)` cents. No `f32`/`f64` for money.
- Database is one file: `%APPDATA%\Athena\athena.db`.
- Logs go to `%APPDATA%\Athena\logs\`. Transaction text never logged.
- Errors are typed with `thiserror`. No `unwrap()` in production paths.
- Export is atomic: write temp file, validate, rename into place. JSON, CSV, and XLSX.
- UI copy flows through one top-level `src/i18n` module, usable by UI, services, and exporters. Each language is a `const Strings` struct with one field per key, so a missing translation fails to compile. No string literals in screens, except debug-only text.

## 8. Release map

| Version | Deliverable |
|---|---|
| 1.0.0 | Daily auto-snapshot, single-instance guard, portable mode, onboarding, people, payer, categories, transactions, quick add, month register, year calendar to month log, settings, English + pt-BR, dark/light theme, SQLite persistence, JSON + CSV export, portable exe |
| 1.1.0 | Who-owes-whom, one-line entry, trash + undo, rotating backups + restore, JSON import, day drill-down, monthly insight, templates, duplicate detection, XLSX export |
| 1.2.0 | Savings goals + contributions, monthly closing snapshots, closing export |
| 1.3.0+ | Attachments, budgets, recurring rules, accounts/transfers, full-text search (FTS5), optional encryption at rest (SQLCipher) |

## 9. Data model

### 1.0.0 tables

```sql
CREATE TABLE profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    currency TEXT NOT NULL DEFAULT 'BRL' CHECK (currency = 'BRL'),
    start_balance_cents INTEGER NOT NULL DEFAULT 0,
    theme TEXT NOT NULL DEFAULT 'dark' CHECK (theme IN ('dark', 'light')),
    language TEXT NOT NULL DEFAULT 'en' CHECK (language IN ('en', 'pt-BR')),
    onboarding_complete INTEGER NOT NULL DEFAULT 0 CHECK (onboarding_complete IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE people (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    kind TEXT NOT NULL CHECK (kind IN ('owner', 'partner', 'other')),
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at TEXT NOT NULL,
    CHECK (kind <> 'owner' OR archived = 0)
);
CREATE UNIQUE INDEX idx_people_one_owner ON people(kind) WHERE kind = 'owner';

CREATE TABLE categories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL COLLATE NOCASE UNIQUE CHECK (length(trim(name)) > 0),
    applies_to TEXT NOT NULL DEFAULT 'expense' CHECK (applies_to IN ('expense', 'income', 'both')),
    color TEXT NOT NULL DEFAULT '#8F98A8'
        CHECK (color GLOB '#[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]'),
    icon TEXT,
    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE TABLE transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    date TEXT NOT NULL CHECK (date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income', 'refund')),
    category_id INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    payer_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
    description TEXT,
    split_mode TEXT CHECK (split_mode IN ('even', 'custom')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE allocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transaction_id INTEGER NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL REFERENCES people(id) ON DELETE RESTRICT,
    amount_cents INTEGER NOT NULL CHECK (amount_cents >= 0),
    UNIQUE (transaction_id, person_id)
);
```

Indexes: `transactions(date)`, `transactions(category_id)`, `transactions(payer_id)`, `allocations(person_id)`.

### Schema checklist

- Foreign keys: allocations → transaction `CASCADE`; allocations → person `RESTRICT`; transaction → category `SET NULL`; transaction → payer `SET NULL`.
- `profile` is a singleton: `CHECK (id = 1)`.
- Exactly one owner: partial unique index on `people(kind) WHERE kind = 'owner'`.
- Owner cannot be archived: table CHECK.
- Archived people and categories remain queryable.
- A transaction may have zero allocations (unassigned) or any number, one per person.
- A zero-cent allocation is valid when explicitly entered.
- Allocation sum must equal transaction total when allocations exist. Enforced in Rust; SQLite cannot express a cross-row CHECK.
- New and edited transactions may reference only active people. Historical allocations may reference archived people.
- `category_id` and `payer_id` are nullable.
- `people` deletion is blocked while allocations reference the person. Archive instead.
- Category deletion is not exposed. Archive instead.
- Money is positive on transactions; allocation amounts are non-negative.
- Names are resolved through current rows. No name snapshots.

### 1.1.0 additions

- `transactions.deleted_at` nullable UTC timestamp. Soft delete.
- `CREATE VIEW live_transactions AS SELECT * FROM transactions WHERE deleted_at IS NULL`.
- `templates`: name, title, amount cents, kind, category, person, description, timestamps.
- Index on `transactions(deleted_at)`.

### 1.2.0 additions

- `goals`: name, description, target cents, target date, frequency, `person_id` (`ON DELETE RESTRICT`), icon, status, created_at, updated_at. No person name snapshot.
- `goal_contributions`: goal id (`ON DELETE CASCADE`), amount cents, date, note, created_at.
- `month_closings`: month primary key, income, expenses, refunds, net, count, `top_category_id` (`ON DELETE SET NULL`), `largest_expense_id` (`ON DELETE SET NULL`), `largest_expense_title` fallback, largest expense cents, previous net and count, `fingerprint` (count, amount sum, max `updated_at` of live rows), closed_at. No category name, no rendered insight.
- Closing insight is rendered at read time from snapshot numbers, the current category name, and the active language. Language switches and category renames never leave stale closing copy. The largest-expense label resolves through `largest_expense_id`; `largest_expense_title` is the fallback after deletion.

## 10. Architecture

```text
athena/
├── Cargo.toml
├── build.rs                      # Windows icon + version metadata
├── assets/icon.ico
├── migrations/
│   ├── 0001_init.sql
│   ├── 0002_v1_1.sql
│   └── 0003_v1_2.sql
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── app/
│   │   ├── mod.rs
│   │   ├── store.rs              # Store: DB owner, AppAction + apply, data_version
│   │   ├── view_model.rs         # cached reads, refresh on data_version change
│   │   ├── ui_state.rs           # screen, month, filters, dialogs
│   │   ├── instance.rs           # single-instance guard
│   │   └── navigation.rs
│   ├── i18n/{mod.rs,en.rs,pt_br.rs}
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── shell.rs
│   │   ├── quick_add.rs
│   │   ├── theme/{mod.rs,palette.rs,typography.rs}
│   │   ├── components/{mod.rs,summary_card.rs,transaction_row.rs,money_input.rs,confirm_dialog.rs,toast.rs,empty_state.rs,person_pill.rs}
│   │   └── screens/{mod.rs,onboarding.rs,month.rs,calendar.rs,settings.rs,trash.rs,goals.rs}
│   ├── domain/
│   │   ├── mod.rs
│   │   ├── settings.rs            # Theme, Language, Profile
│   │   ├── money.rs
│   │   ├── date.rs
│   │   ├── transaction.rs
│   │   ├── category.rs
│   │   ├── person.rs
│   │   ├── goal.rs
│   │   ├── closing.rs
│   │   ├── template.rs
│   │   ├── export.rs
│   │   └── reports.rs
│   ├── application/
│   │   ├── mod.rs
│   │   └── services/{mod.rs,transactions.rs,reports.rs,onboarding.rs,export.rs,import.rs,insights.rs,settlement.rs,quick_parse.rs,duplicates.rs,closing.rs,goals.rs}
│   ├── infrastructure/
│   │   ├── mod.rs
│   │   ├── config.rs             # data dir, portable mode
│   │   ├── db/{mod.rs,migrations.rs,backup.rs,integrity.rs,profile_repo.rs,people_repo.rs,categories_repo.rs,transactions_repo.rs,trash_repo.rs,reports_repo.rs,templates_repo.rs,goals_repo.rs,closings_repo.rs,import_repo.rs}
│   │   └── export/{mod.rs,json.rs,csv.rs,xlsx.rs}
│   └── shared/{mod.rs,error.rs,test_fixtures.rs}   # test_fixtures behind #[cfg(test)]
└── docs/
```

## 11. Screens

### Onboarding

Full-window. Animated gradient glow, wordmark `ATHENA`, four steps, progress dots, skip link. Large type. No form chrome.

1. Welcome + language toggle.
2. Your name, optional partner name.
3. Starting balance, optional. May be negative.
4. Summary + "Start".

### Month (home)

- Month stepper with a large, unmistakable selected-month label and accent underline.
- Lifetime balance strip, explicitly labeled "All time".
- Three compact month cards, explicitly labeled "This month": Income, Expenses, Result.
- Insight line (1.1).
- Filter row: All / each person / Shared, category, search.
- Day groups. Row: type icon + label, title, category, person name, amount. Not initials alone.
- Right rail: spending by category bars, totals by person, settle-up (1.1).
- Empty state with primary CTA and one sentence of guidance.
- `+ New transaction` and `Ctrl+N`.
- Closed months (1.2) show a card with income, expenses, refunds, result, count, top category, largest expense, insight, and `Closed on`. `Recalculate closing` appears only for completed months and only after confirmation.

The register is the dominant element. Summary cards support it.

### Calendar

- Year stepper.
- 12 compact month cards, 3 columns. Card: month name, expense net, income, count. Current month accent border. Closed months show a dot (1.2).
- Click card opens Month for that month.
- 1.1 adds day heat + day filter inside month.

### Goals (1.2)

- Cards: name, owner, contributed/target, remaining, progress bar, target date, and per-period hint.
- Detail: contribution list with add and delete; pause, resume, complete, archive.
- Empty state with a localized create CTA. Contributions never create transactions or change month totals.

### Settings

- Profile: owner name, partner name, starting balance, theme, language.
- People: add, rename, archive. Archived people leave pickers, stay in history.
- Categories: add, rename, color, archive. Archived categories leave pickers, stay in history.
- Data: database path, open folder, export JSON, CSV, XLSX (1.1), backups (1.1).
- About: version.

### Quick add (modal)

- Amount first, focused on open.
- Title, date, type, category (filtered by type), payer, person selector, description.
- Optional details (description) behind a collapsible "More" section.
- Shared mode exposes one amount field per active person, default even.
- Selected person and payer are visible before saving.
- Enter saves. Esc closes. Inline errors.
- After save, focus returns to the amount field for repeated entry.
- 1.1 adds templates, one-line mode, duplicate warning.

## 12. Visual direction

### Palette

| Token | Dark | Light |
|---|---|---|
| bg | `#050608` | `#F6F7FB` |
| panel | `#0A0C10` | `#FFFFFF` |
| card | `#0F1218` | `#FFFFFF` |
| card hover | `#151A22` | `#F0F2F8` |
| border | `#232A36` | `#E2E5EE` |
| text | `#ECEEF4` | `#141821` |
| muted | `#8B95A7` | `#5C6476` |
| accent | `#6C8CFF` | `#4F6BFF` |
| positive | `#3DDC97` | `#1F9E6E` |
| negative | `#FF6B7A` | `#D6455A` |
| warning | `#F5C451` | `#B07B12` |

Dark is the default. Its canvas is near-black (`#050608`), not gray. Light is a full peer theme, not an inverted afterthought. Both palettes must keep body text, muted text, accent text, and amount colors readable; contrast is checked in review.

### Type

- Salted Flutters Polaris: 15 body, 13 secondary, 20 section, 28 heading, 34 display.
- Regular for body and small text; Medium for buttons; Semibold for section, heading, and display.
- Athena Polaris Tabular (frozen `tnum`) for monetary values and table dates.
- Tabular numbers. Right-aligned amounts.
- Fallback: Segoe UI, Segoe UI Symbol, then egui defaults.

### Shape and spacing

- 8 pt grid.
- Cards: 16 radius, 1 px border, 16 padding.
- Buttons: 10 radius, 36 height.
- Content max width 1100, centered.
- Sidebar 220.

### Motion

- 120–180 ms ease.
- Month and onboarding transitions fade + 8 px slide.
- Hover: border and background only. No bounce.

### Non-color cues

- Expense: `−` sign + "Expense" label.
- Income: `+` sign + "Income" label.
- Refund: `↩` icon + "Refund" label.
- Selected month: accent underline plus bold label.
- Closed month: dot plus tooltip "Closed".

## 13. Report semantics

- **Transaction view:** each purchase appears once. Sum of transaction amounts.
- **Responsibility view:** each person's allocation. Sum of allocation amounts.
- **Spending:** expenses minus refunds.
- **Month result:** income minus spending.
- **Tracked balance:** start balance + all income - all expenses + all refunds. Start balance may be negative.
- **Person total:** sum of that person's allocations for expenses minus refunds. Income excluded from spending totals.
- **Refund allocation:** independent. If a refund is shared, its allocations reduce each person's responsibility. No automatic link to the original purchase.
- **Settlement (1.1):** `paid - owed` per person. With exactly two active people, one sentence from the owner's perspective. With three or more, a list of every balance. Null payer rows are counted and surfaced, not assumed.
- **Closing (1.2):** stores a structured snapshot at close time for a completed month only; current and future months cannot be closed. Replacing an existing snapshot requires `force = true` behind a confirmation. The month screen keeps showing live summaries. `closing_is_stale` compares the stored fingerprint with the live month; when they differ, the closing card shows `Changed since closing` and offers `Recalculate`. The calendar dot means a snapshot exists, not that the month is frozen. The card's insight sentence is rendered from the snapshot on every draw, so switching language or renaming the top category updates the card without rewriting the snapshot. The largest-expense label resolves through `largest_expense_id`, falling back to the stored title if the transaction was deleted.
- Every report labels range and view. No mixed totals.

## 14. Export and backup

### JSON

Versioned profile export. Top-level `schema_version: 1`, `app_version`, `exported_at`, `profile`, `people`, `categories`, `transactions`, `templates` (1.1), `goals` and `goal_contributions` (1.2), `closings` (1.2). Each transaction contains its `allocations` with per-person amounts. Cents as integers. Dates ISO. Validate by parsing the written bytes before rename.

Exports contain live rows only. Trashed transactions are excluded from JSON, CSV, and XLSX. A database backup is the only artifact that preserves trashed rows, and the README says so. `schema_version` stays `1` while changes are additive.

### CSV

Transactions, one row per transaction. UTF-8 with BOM. Delimiter `;`. Decimal comma. Columns: `Date;Title;Type;Amount;Category;People;Allocations;Description`. Export takes an explicit scope: default names `athena-YYYY-MM.csv` (month) and `athena-all.csv`. The JSON export is always the full profile, `athena-full-YYYY-MM-DD.json`.

`People` is a readable name list. `Allocations` preserves the split as `Name:100,00|Name:80,00`. CSV is the readable report; JSON is the complete structured export. Text cells that begin with `=`, `+`, `-`, or `@` are prefixed with a single quote so spreadsheet software never executes them as formulas.

### XLSX (1.1)

Sheets: Summary, Transactions, Income, Expenses, Categories, People, Templates. Summary, Transactions, Income, Expenses and People follow the chosen `ExportScope` (month or all time); tracked balance on Summary is always all-time and labeled; Categories and Templates are scope-independent. Currency format `#,##0.00`. Headers follow the selected language.

### Daily snapshot (1.0)

On startup, once per day: `VACUUM INTO backups/auto-YYYY-MM-DD.db`, `integrity_check`, keep the newest 7. Pruning is per filename prefix: `auto-` keep 7, `athena-` (manual, 1.1) keep 30, `pre-migration-` / `pre-restore-` / `pre-import-` keep 5. One prefix's policy never deletes another prefix's files.

### JSON import (1.1)

Restore from a JSON export into an empty database, or replace the current one after a pre-import snapshot. Validates `schema_version`, references, and allocation sums before writing, in one SQL transaction. Every collection added after 1.0 is `#[serde(default)]`, so older exports import. Export order is deterministic and ids and timestamps are preserved, so export -> import -> export is byte-comparable apart from `exported_at`.

### Backups (1.1)

Same `VACUUM INTO` path as the daily snapshot. `VACUUM INTO` timestamped file in `%APPDATA%\Athena\backups`. Keep 30 (prefix `athena-` only). Verify `PRAGMA integrity_check`. Restore with confirmation and a pre-restore snapshot, swapped in atomically: copy to a temp file in the data dir, rename the live file to `athena.db.old`, rename the temp into place, reopen, and only then delete `.old`. On failure rename `.old` back; on launch, a missing `athena.db` with an `athena.db.old` present is recovered automatically.

### Atomic writes

All exports and backups write to a temporary file in the destination directory, validate, then persist. A failure leaves the previous file untouched.

## 15. Error handling and migration safety

- `AppError` with `thiserror`.
- DB errors surface as toast + log line with operation name, no transaction text.
- Validation errors inline on the field.
- Export failures leave the old file intact.
- No `unwrap()` outside tests.
- Startup failure uses the fatal-error path below: native message box, log line, non-zero exit.
- **Newer schema:** if `PRAGMA user_version` exceeds the supported migration count, return `SchemaTooNew { found, supported }` and do not open the database.
- **Migration rollback:** each migration runs in one SQLite transaction. On failure, roll back, keep `user_version`, and report the pre-migration backup path.
- **Pre-migration backup:** before applying a migration to an existing file, `VACUUM INTO backups/pre-migration-vN-<timestamp>.db` and verify integrity.
- **Second instance:** a second launch shows "Athena is already running" and exits without opening the database.
- **Fatal startup errors** (newer schema, corrupt database, already running, unwritable data dir) are shown in a native message box (`rfd::MessageDialog`) because release builds have no console, logged without transaction text, and exit non-zero.
- **Corrupt database:** opening fails with the database path and a recovery hint. README documents renaming the file and restoring a backup.

## 16. Testing

- Property tests (`proptest`): money parse/format round-trip on a bounded domain that includes `i64::MIN`; splits always sum to the total.
- UI snapshots (`egui_kittest`): first launch, populated month, both languages, both themes. Fonts are fixed and time is frozen, or the test asserts structure instead of pixels, so animation and host fonts do not make snapshots flaky.
- Boundaries: Dec/Jan, first and last day of adjacent months, leap day, empty month, and max-zero guards for category bars and day heat.
- Search: `pao` finds `Pão`; `ALIMENTAÇÃO` and `Alimentação` collide as the same category name.
- Import: 1.0 and 1.1 JSON import into 1.2 with defaulted new collections; deterministic export order and preserved ids/timestamps.
- Themes: both palettes render; dark canvas is near-black; contrast assertions cover text, muted text, accent, positive and negative.
- Integrity: `integrity::check` returns no issues after every repository and import test.
- Soft-delete guard (1.1): a test fails if transaction reads bypass `live_transactions`.
- Cross-doc lint: `docs/superpowers/doc-lint.sh` fails on stale terms (`AppState`, per-query `deleted_at IS NULL`, `payment_method`, raw file copies, old export names, `top_category` without `_id`). Run before each release. <!-- lint:allow -->
- Unit: money parse/format including `i64::MIN` and >2 decimals; split errors; date grid; validation; insight strings in both languages; one-line parser.
- Integration-style: migrations including rollback and newer-schema rejection; repositories; reports; export round-trip; CSV atomic failure; backup restore. These run as `#[cfg(test)]` modules inside the crate so they share the test fixtures; no separate `tests/` crate.
- UI: state transitions unit-tested. Rendering covered by snapshots; the manual smoke checklist keeps only what snapshots cannot check.
- Localization: every visible key exists in both dictionaries; pt-BR insight and settlement asserted.
- Every release ends with `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`.

### Review focus

1. Money input: pt-BR and plain formats, thousands separators, `R$`, empty, letters, negative, more than two decimals, and the single-dot three-digit ambiguity (`0.005` is rejected, `1.234` is thousands). Parse or reject clearly. Never round silently.
2. Odd-cent split: `R$ 0,01` across three people; zero-cent allocations; empty and all-zero weights rejected.
3. Month boundary: last and first day of adjacent months appear in the correct month in register, calendar, and reports.
4. Empty state: first launch, no profile, no transactions, no partner. No panic, no blank card, no `NaN`, no division by zero.
5. History after archive: archiving or renaming a category or person keeps old transactions readable, filters working, and totals stable.
6. Payer integrity: historical rows from 1.0 already carry payer; null payer rows are surfaced in 1.1, not assumed.
7. Soft delete leaks: deleted rows vanish from month, calendar, reports, settlement, search, and exports.
8. Backup restore: a failed restore leaves the app usable; a pre-restore snapshot always exists.
9. Localization: switching language updates every screen, month names, insight, settlement, and export headers without restart.
10. Migration safety: newer schema refuses to open; failed migration rolls back.
11. Theme contrast: dark uses a near-black canvas, muted text and amounts stay readable, and no meaning depends on color alone.
12. Closing integrity: fingerprint detects changes; the largest expense resolves through its id and falls back to the stored title only after deletion.

## 17. Success criteria mapping

| Criterion | Where |
|---|---|
| Works offline | No network code; all state local |
| Persists between sessions | SQLite file + WAL |
| Income, expense, refund | `TransactionType`; transfer deferred |
| Search and filter | Month screen filter row |
| Accurate totals | Reports service + tests |
| Goals | 1.2.0 |
| Shared without double count | Transaction vs responsibility views |
| XLSX and CSV | CSV 1.0; XLSX 1.1 |
| JSON profile | 1.0 export |
| Polished desktop | Theme + shell + onboarding + two languages |
| Payer history | `transactions.payer_id` in 1.0 |

## 18. Left to the builder

- Exact widget composition inside each card.
- Icon set (small inline Unicode/SVG set; no icon library required).
- Animation curves beyond the stated duration.
- Fixture variations beyond the Appendix defaults.
- pt-BR wording choices where the plan gives the English source string.

## 19. Settled after review

- Payer moves into the 1.0 schema and quick-add form.
- Owner name lives only in `people`.
- Names resolve through current rows; renaming changes historical labels.
- Money input rejects more than two decimals; no silent rounding.
- `split_evenly` returns an error for invalid weights.
- CSV is atomic and carries an `Allocations` column; JSON is the complete export.
- Refunds are independent, allocated, and never income.
- Theme and language are defined once in `domain/settings.rs`.
- `archive_person` exists; archived people leave pickers, keep history.
- Schema constraints and migration safety are explicit.
- Closings store ids and numbers plus one display-fallback title; insight text is rendered at read time in the active language.
- Migration tests assert `MIGRATIONS.len()`, never a hardcoded version, so each release stays green.
- Tests are in-module `#[cfg(test)]` modules sharing one fixture file; the only separate `tests/` crate is the source-scanning `no_transactions_table.rs` guard added in 1.1.
- Release is a portable exe with icon, version metadata, and recovery notes.
- UI ships in English and pt-BR.
- Architecture review (2026-11-12): `Store`/`ViewModel`/`UiState` are the three state parts; `AppCore` is the thin composition root; `AppAction` is the only mutation path.
- i18n is a top-level module with compile-checked keys.
- Soft delete is enforced by the `live_transactions` view, not per-query filters.
- Daily auto-snapshot ships in 1.0; all backups use `VACUUM INTO`.
- Single-instance guard and portable mode ship in 1.0.
- Categories carry `applies_to`; transactions carry `split_mode`; `payment_method` dropped. <!-- lint:allow -->
- JSON import moves to 1.1.
- Closings store a fingerprint for change detection.
- Review 2 (2026-11-12): income categories seeded; `ActionOutcome` defined; single-instance guard uses an `fs4` file lock; integrity check lives in infrastructure; largest expense stored by id with a title fallback; export scope is explicit; restore is a rename swap; backups prune per prefix; fatal startup errors use a message box; accent-folded search.
- Review 3 (2026-11-12): dark canvas deepened to near-black; light remains a full peer theme; CSV is formula-safe; export names and scope are fixed; category `applies_to` is editable.
- Review 4 (2026-11-12): 1.2 finalized with completed-month-only closings, explicit `force` for recalculation, a stored largest-expense title fallback, `YearMonth` ordering, and import/export of goals and closings with `#[serde(default)]`.
