# Athena contracts

**Status:** Canonical. Sections marked **Frozen for 1.1** are the binding interface for the [1.1.0 Safe Data plan](../plans/2026-11-16-athena-1.1.0-safe-data.md); §14.1 is **Frozen for 1.2**, §14.2 is **Frozen for 1.3** and §14.3 is **Frozen for 1.4**. Sections marked **Draft** reserve names and rules for later releases; they become frozen when their release plan is written.
**Created:** 2026-11-16, from the [roadmap decisions](2026-11-13-athena-roadmap-revision.md), the [1.1](2026-11-13-athena-1.1.0-revision-brief.md) / [1.2](2026-11-13-athena-1.2.0-revision-brief.md) briefs, and the [readiness review](2026-11-16-athena-readiness-and-appearance-review.md), checked against source at `2093257`.

## §0 Authority and change rules

- Task prompts link to a section here instead of restating schemas, DTOs, actions, or read-model fields. When a task and this document disagree, this document wins and the task is corrected.
- A change to a frozen section is a deliberate edit to this file, made in the same commit as the code that needs it, with a line in §15.
- The revision briefs and the readiness review remain the rationale. Where this document resolves one of their open questions differently, §1 records the decision and the brief's *acceptance criteria* still apply.
- Shipped migrations are immutable. Reserved migration numbers for unreleased versions (§2) may be renumbered until that version ships.

## §1 Decisions

| ID | Decision | Source |
|---|---|---|
| D1–D7 | As recorded in the roadmap revision (visual scope, complete-entry settlement, template fidelity, Funded vs Completed, 1.0.2 patch scope, wallpaper on all main screens, Rust/egui only). | User, 2026-11-13 / 2026-11-16 |
| D8 | Paybacks are recorded as a new transaction kind, `settlement`. A settlement moves money between two people. It is excluded from income, spending, and tracked balance, and it clears debt in the settlement calculation. | User, 2026-11-16 |
| D9 | Releases are split: **1.1 Safe data**, **1.2 Daily speed**, **1.3 Appearance**, **1.4 Goals & closings**. | User, 2026-11-16 |
| T1 | Settlement balances are **cumulative as of the end of the selected month**, not per month. This follows from D8: per-month balances would show a December payback for a November debt as an overpayment. | Technical consequence of D8 |
| T2 | Backup restore uses SQLite's online backup API (`Connection::restore`, rusqlite `backup` feature) into the open connection. It does not rename database files. The copy commits as one write transaction, and an interruption rolls back to the original. This replaces the brief's rename state machine. The brief's failure-injection acceptance still applies, at the boundaries in §9.3. | Technical |
| T3 | JSON import replaces the profile inside one SQL transaction on the live connection, after validation and a pre-import snapshot. No file swap. | Technical |
| T4 | Document format and database schema versions are independent counters (§2). The JSON field `schema_version` from format 1 is legacy; format ≥ 2 uses `format_version`. | Review P1 |
| T5 | Wire DTOs for the JSON document are separate structs from domain types. Domain types may change; frozen DTOs may not. | Technical |

## §2 Release ledger

| Release | Theme | Migrations (schema version after) | Document format |
|---|---|---|---|
| 1.0.x | Shipped foundation | `0001_init` (1) | 1 (written, never read) |
| **1.1.0** | Safe data | `0002_settlement_kind`, `0003_trash` (3) | **2** |
| **1.2.0** | Daily speed | `0004_templates` (4) | **3** (frozen for 1.2, §14.1) |
| **1.3.0** | Appearance | `0005_appearance` (5) | **4** (frozen for 1.3, §14.2) |
| **1.4.0** | Goals & closings | `0006_goals`, `0007_closings` (7) | **5** (frozen for 1.4, §14.3) |

Rules:
- One migration file per feature. The schema version equals `MIGRATIONS.len()`; tests never hard-code it.
- A document format increments only when the document shape changes. A reader accepts every format ≤ its own and rejects any greater format before touching data.

## §3 Global invariants (all releases)

1. Money is `Money(i64)` cents. Aggregation uses checked arithmetic wherever a sum can come from imported data. No floating point in domain or storage.
2. Every persisted mutation goes through `AppAction` → `Store::apply`. Every successful action bumps `data_version`. A failed action leaves data and `data_version` unchanged.
3. No SQL outside `src/infrastructure`. Screens read only `ViewModel` and never query or read the clock (`vm.today` is the clock).
4. Every visible string lives in `src/i18n` in both `en` and `pt-BR`.
5. Offline: no network access and no remote assets.
6. After 1.1 Task 4, every read of transaction rows goes through `live_transactions` (§6), except the modules §6.4 lists.
7. `integrity::check` returns no issues at the end of every repository test and after every import/restore.
8. One process per data folder (`athena.lock`), held for the whole process lifetime, including recovery mode.

## §4 Schema — Frozen for 1.1

### §4.1 Migration runner: foreign-keys-off migrations

A migration whose first line is exactly `-- athena:foreign-keys-off` runs as follows:

1. Read the current `PRAGMA foreign_keys` value.
2. Set `PRAGMA foreign_keys = OFF` **outside** any transaction. SQLite ignores the pragma inside one.
3. `BEGIN`, execute the batch, then run `PRAGMA foreign_key_check`. Any row returned is `AppError::Migration("step N: foreign key check failed …")` and rolls back.
4. Set `user_version`, then `COMMIT`.
5. Restore the step-1 value on **every** path, success or error.

Other migrations keep the existing behavior. This is required because `DROP TABLE transactions` with foreign keys on would cascade-delete every allocation.

### §4.2 `0002_settlement_kind.sql`

The rebuild widens the `kind` CHECK, adds a settlement shape guard, preserves ids and the AUTOINCREMENT high-water mark, and sets Athena's application id (`0x41544E41`, "ATNA").

```sql
-- athena:foreign-keys-off
PRAGMA application_id = 1096044097;

CREATE TABLE transactions_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
    date TEXT NOT NULL CHECK (date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income', 'refund', 'settlement')),
    category_id INTEGER REFERENCES categories(id) ON DELETE SET NULL,
    payer_id INTEGER REFERENCES people(id) ON DELETE SET NULL,
    description TEXT,
    split_mode TEXT CHECK (split_mode IN ('even', 'custom')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (kind <> 'settlement' OR (payer_id IS NOT NULL AND category_id IS NULL AND split_mode IS NULL))
);

INSERT INTO transactions_new (id, title, amount_cents, date, kind, category_id, payer_id,
                              description, split_mode, created_at, updated_at)
SELECT id, title, amount_cents, date, kind, category_id, payer_id,
       description, split_mode, created_at, updated_at
FROM transactions;

-- Keep ids of hard-deleted 1.0 rows from being reused.
INSERT INTO sqlite_sequence (name, seq)
SELECT 'transactions_new', seq FROM sqlite_sequence
WHERE name = 'transactions'
  AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'transactions_new');
UPDATE sqlite_sequence
SET seq = max(seq, coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'transactions'), 0))
WHERE name = 'transactions_new';

DROP TABLE transactions;
ALTER TABLE transactions_new RENAME TO transactions;

CREATE INDEX idx_transactions_date ON transactions(date);
CREATE INDEX idx_transactions_category ON transactions(category_id);
CREATE INDEX idx_transactions_payer ON transactions(payer_id);
```

Required tests: rows, allocations and ids survive. After upgrading a v1 database where the highest-id row was hard-deleted, the next insert receives an id greater than the old sequence value. `foreign_key_check` is empty. `foreign_keys` is ON again afterwards. `application_id` is set.

### §4.3 `0003_trash.sql`

```sql
ALTER TABLE transactions ADD COLUMN deleted_at TEXT
    CHECK (deleted_at GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T*');
CREATE INDEX idx_transactions_deleted ON transactions(deleted_at);
CREATE VIEW live_transactions AS SELECT * FROM transactions WHERE deleted_at IS NULL;
```

`SELECT *` in a view freezes its column list. **Any later migration that adds a column to `transactions` must drop and recreate `live_transactions` in the same file.**

### §4.4 Application id

Databases at schema ≥ 2 carry `application_id = 0x41544E41`. Version-1 files have 0 and are recognized by table shape (§9.3, step 3).

## §5 Transaction kinds — Frozen for 1.1

### §5.1 `TransactionType::Settlement`

- Code and serde name: `"settlement"`. Label: en "Settlement", pt-BR "Acerto". Register glyph: `⇄`.
- Meaning: `payer_id` (*from*) handed `amount` to the single allocation's person (*to*).
- `CategoryScope::allows(Settlement)` is `false` for every scope.

### §5.2 Shape validation

`TxDraft::validate` adds these errors for `kind == Settlement`:

| Rule | `ValidationError` | `TxField` |
|---|---|---|
| `payer_id` is `Some` | `SettlementNeedsFrom` | `Payer` |
| exactly one allocation, and its amount equals `amount` | `SettlementNeedsOneRecipient` | `People` |
| recipient ≠ payer | `SettlementSamePerson` | `People` |
| `category_id` is `None` | `SettlementHasCategory` | `Category` |
| `split_mode` is `None` | `SettlementHasSplitMode` | `People` |

`integrity::check` gains `IntegrityIssue::SettlementShape { tx_id }` for stored rows violating these rules.

### §5.3 Effects matrix

| Consumer | expense | income | refund | settlement |
|---|---|---|---|---|
| Tracked balance | − | + | + | **excluded** |
| `MonthSummary` income/expenses/refunds/count | counted | counted | counted | **excluded, including `count`** |
| Category totals, person totals, day totals, calendar heat | spending | — | −spending | **excluded** |
| Settlement calculation (§7) | if complete | excluded | if complete (reversed) | **always** (shape guaranteed) |
| Register, search, person filter (payer or recipient) | shown | shown | shown | shown |
| "Shared only" filter | ≥ 2 allocations | — | ≥ 2 allocations | never |
| CSV export | row | row | row | row, kind column `settlement` |
| JSON format 2 | record | record | record | record |

CSV columns (9, `;`-delimited, localized headers): `Date;Title;Type;Amount;Category;Payer;People;Allocations;Description` (`Data;Título;Tipo;Valor;Categoria;Pagador;Pessoas;Divisão;Descrição`). Payer is empty on `None`, `"?"` if unresolved, and formula-escaped via `csv_safe`.

`SUMMARY_SELECT`, `summaries_for_year`, and every `CASE t.kind` aggregate must exclude settlement explicitly. A test seeds one row of each kind and asserts every row of this table.

## §6 Trash — Frozen for 1.1

### §6.1 Semantics

- `DeleteTransaction(id)` sets `deleted_at` to now (UTC RFC 3339, second precision) and leaves `updated_at` alone. It fails with `NotFound` for a missing or already-deleted id.
- `RestoreTransaction(id)` clears `deleted_at`. Restored rows may reference archived people or categories; that is history.
- `PurgeTransaction(id)` hard-deletes one *deleted* row; allocations cascade. It fails for a live row. `EmptyTrash` purges all deleted rows.
- `UpdateTransaction` on a deleted row returns `NotFound`. Rows must be restored before they can be edited.

### §6.2 Retention

- Deleted rows older than **30 days** are purged at startup, only after that launch's daily snapshot is verified as an Athena database AND—whenever expired rows exist—a fresh verified `pre-purge-YYYY-MM-DD_HH-MM-SS.db` snapshot of the current database succeeds. If any snapshot check or creation fails, skip purging and log it.
- Clock-rollback guard: skip purging when `now < max(deleted_at)`.
- A forward clock jump may purge early, but the launch's verified `pre-purge-` snapshot holds those rows. The Trash screen states both the 30-day rule and the snapshot fallback.

### §6.3 Undo

- Undo is offered for `DeleteTransaction` only, through a toast action that applies `outcome.inverse`.
- Purging a row (`PurgeTransaction(id)`) clears a matching `RestoreTransaction(id)` Undo toast (and `EmptyTrash` clears any `RestoreTransaction` Undo toast); if an undo fails anyway, an `UndoFailed` toast is shown.
- The toast lifetime starts at first display and pauses while hovered or focused.
- `Ctrl+Z` triggers the visible toast's undo, except while a text field has focus.

### §6.4 Query boundary

All reads use `live_transactions`. Only these may name `transactions` directly:
- `trash_repo.rs`: reads of deleted rows, restore, purge, empty
- `transactions_repo.rs`: `INSERT`/`UPDATE` in the write path and the soft-delete `UPDATE`
- `import_repo.rs`: `DELETE`/`INSERT` during replace
- `integrity.rs`: checks over all rows, deleted included

`tests/no_transactions_table.rs` is a supplementary lexical guard. It strips comments, normalizes whitespace, matches `FROM|JOIN transactions\b`, and allow-lists exactly the files above. The binding check is behavioral: delete a row, then assert it is absent from register, search, summaries, category/person/day totals, calendar, balance, settlement, CSV and JSON export; restore it and assert it is back.

## §7 Settlement calculation — Frozen for 1.1

### §7.1 Inputs

All live rows with `date ≤ as_of`, where `as_of` is the last day of the selected month. Income is ignored.

- **Complete expense/refund:** `payer_id` is set and allocations are non-empty (sums are guaranteed by validation/integrity).
- **Excluded expense/refund:** classified by the first matching reason, in order:
  1. `Unassigned` — zero allocations ("no one" responsible). Valid data; informational.
  2. `UnknownPayer` — allocations present, payer missing. A repair item.
  3. `Invalid` — fails integrity rules. Expected to be 0; logged.
- **Settlement:** always included.

### §7.2 Arithmetic (checked `i64`; overflow → `AppError::Validation`)

| Row | payer `paid` | each allocation person `owed` |
|---|---|---|
| expense | `+amount` | `+share` |
| refund | `−amount` | `−share` |
| settlement | `+amount` | recipient `+amount` |

`balance = paid − owed`. Positive means others owe this person. The sum of all balances is exactly 0, and tests assert this for every fixture.

### §7.3 Participants and output

Participants are people, including archived ones, with non-zero `paid` or `owed` from included rows.

```rust
pub struct SettlementView {
    pub as_of: NaiveDate,
    pub balances: Vec<PersonBalance>,       // participants, balance desc, then person id
    pub transfers: Vec<SuggestedTransfer>,  // empty when all settled
    pub excluded: ExclusionCounts,          // { unassigned, unknown_payer, invalid }
}
pub struct PersonBalance { pub person_id: i64, pub paid: Money, pub owed: Money, pub balance: Money }
pub struct SuggestedTransfer { pub from: i64, pub to: i64, pub amount: Money }
```

- **Transfers:** greedy matching of the largest debtor with the largest creditor. Ties go to the lower person id. This gives at most n−1 transfers whose amounts exactly zero every balance. The result is deterministic.
- **Wording**, from the owner's perspective when the owner is a party:

| Situation | en | pt-BR |
|---|---|---|
| owner is creditor | "Ana owes you R$ 80,00" | "Ana te deve R$ 80,00" |
| owner is debtor | "You owe Ana R$ 80,00" | "Você deve R$ 80,00 a Ana" |
| owner not a party | "Ana owes Bia R$ 80,00" | "Ana deve R$ 80,00 a Bia" |
| all settled | "All settled up as of 30 Nov" | "Tudo acertado até 30/11" |

  More than one transfer renders as a list. The card shows whenever `vm.settlement_unavailable` is true or when `vm.settlement` has at least two participants or any exclusions (`excluded.any()`); when fewer than two participants exist with exclusions, it renders a muted status line (`settlement_no_balances`) above the exclusion repair links instead of fake transfers.
- Archived parties get an "(archived)" suffix. Their transfer offers "Restore Ana to record this" instead of **Record**, because new rows may not reference archived people.

### §7.4 Recording and repair

- **Record** opens Quick Add in Settlement mode prefilled with from, to, amount, `date = vm.today`, and title "Ana → Bia" (editable). It never saves automatically and never auto-backdates; when the viewed month differs from `today`'s month, Quick Add displays a non-blocking warning noting the viewed month and that the settlement will be dated `today`.
- **Repair filter:** `TxFilter` gains `issue: Option<SettlementIssue>` (`UnknownPayer | Unassigned`) and `until: Option<NaiveDate>`. When `issue` is set, `month` is `None` and `until == Some(as_of)`. The register then lists matching live expense/refund rows across all months up to `as_of`, under a visible chip ("Missing payer · until Nov 2026 · ✕").
- Clearing the chip restores `month == Some(ui.month)`. The `UiState` invariant becomes: *`filter.month == Some(month)` unless `filter.issue` is set.*

## §8 Actions, outcomes and read models — Frozen for 1.1

### §8.1 Actions

| `AppAction` | Inverse | `RefreshScope` | Notes |
|---|---|---|---|
| `SaveTransaction(TxDraft)` | `DeleteTransaction(id)` | `Month` | unchanged |
| `UpdateTransaction(i64, TxDraft)` | `UpdateTransaction(id, previous)` | `Month` | `NotFound` on deleted rows |
| `DeleteTransaction(i64)` | `RestoreTransaction(id)` | `Month` | soft delete |
| `RestoreTransaction(i64)` | `DeleteTransaction(id)` | `Month` | |
| `PurgeTransaction(i64)` | `None` | `Trash` | confirm first |
| `EmptyTrash` | `None` | `Trash` | confirm first |
| `CreateBackup` | `None` | `Backups` | writes `manual-…db` |
| `RestoreBackup(RestoreSource)` | `None` | `All` | §9; notice carries the snapshot path |
| `ImportProfile(Box<ImportPlan>)` | `None` | `All` | §10; the plan was validated at preview |
| `RestorePerson(i64)` | `None` | `People` | checks active name uniqueness |
| existing profile/people/category actions | unchanged | unchanged | |
| `SaveTemplate` / `UpdateTemplate` / `DeleteTemplate` (1.2, §14.1) | `None` | `Templates` | Settings and Quick Add |
| `SaveTransactionFromTemplate(TxDraft, i64)` (1.2) | `DeleteTransaction(id)` | `Month` + `Templates` | one transaction, one timestamp |
| `ApplyAppearance(Box<AppearanceApply>)` / `ResetAppearance` (1.3, §14.2) | `None` | `Appearance` | Settings → Appearance; Apply may publish a staged managed wallpaper first |
| `CreateGoal(GoalDraft)` / `UpdateGoal(i64, GoalDraft)` / `ChangeGoalStatus(i64, GoalTransition)` / `AddContribution(i64, ContributionDraft)` / `DeleteContribution { goal_id, contribution_id }` (1.4, §14.3) | `None` | `Goals` | Goals screen; a create reports the new goal id in the outcome |
| `CloseMonth(YearMonth)` / `RecalculateClosing(YearMonth)` (1.4, §14.3) | `None` | `Closings` | Month card; completed local months only |

- `RestoreSource` is `Listed(PathBuf)` (inside `backups_dir`) or `External(PathBuf)` (picked file).
- `ImportPlan` derives `Clone` and `PartialEq` because `AppAction` does.
- `Store::run` becomes `&mut self`, because restore needs `Db::conn_mut`.

### §8.2 Outcome

```rust
pub struct ActionOutcome {
    pub inverse: Option<AppAction>,
    pub toast: Option<ToastKey>,
    pub refresh: RefreshScope,
    pub notice: Option<OperationNotice>, // new
}
pub struct OperationNotice { pub kind: OperationKind, pub snapshot: Option<PathBuf>, pub source: Option<String> }
pub enum OperationKind { BackupCreated, Restored, Imported, TrashPurged }
```

- `RefreshScope` gains `Trash` and `Backups`.
- `ToastKey` gains `TransactionRestored`, `TrashEmptied`, `PersonRestored`, `BackupCreated`, `Restored`, `Imported`.
- `vm.last_operation: Option<OperationNotice>` keeps the latest notice for the Data & backups page for the rest of the session. Across restarts, the `pre-restore-`/`pre-import-` entries in the backups list are the durable record.

### §8.3 Cache key and fields

```rust
struct LoadKey { version: u64, month: YearMonth, year: i32, filter: TxFilter,
                 today: NaiveDate, screen: Screen }
```

Adding `today` makes the cache reload at midnight and on resume; `AthenaApp` already assigns `vm.today` before `sync`. Adding `screen` lets screen-specific models load lazily.

New `ViewModel` fields:

| Field | Loaded when | Source |
|---|---|---|
| `settlement: Option<SettlementView>` | always (cheap, isolated) | §7 service |
| `settlement_unavailable: bool` | on settlement error | set `true` when optional settlement calculation fails |
| `refresh_failed: bool` | on required query error | set `true` when required reload fails; prior loaded fields remain coherent and same-key auto-retry is suppressed |
| `trash_count: usize` | always | `trash_repo::count` |
| `trash: Vec<TrashItem>` | `screen == Trash` | `trash_repo::list` (newest first) |
| `backups: Vec<BackupFile>` | `screen == Settings` | `backup::list` (file I/O; not per frame) |
| `last_operation: Option<OperationNotice>` | set from outcomes | in-session only |

`ViewModel::reload` stages all required queries before mutating `self`, and `AppCore::apply` preserves `Ok(outcome)` when a post-commit reload fails so committed mutations never lose `ActionOutcome` or trigger double submission.

`TrashItem { tx: Transaction, deleted_at: String }`.

### §8.4 Screens and shortcuts

- `Screen::Trash` is added. The sidebar order is `Screen::ALL = [Month, Calendar, Trash, Settings]`.
- Shortcuts come from an explicit table, **not** from `ALL` indices: `Ctrl+1` Month, `Ctrl+2` Calendar, `Ctrl+3` Settings, `Ctrl+4` Trash, and (1.4) `Ctrl+5` Goals. A test asserts the table. In 1.4 the sidebar order is `[Month, Calendar, Goals, Trash, Settings]`; the Ctrl+1–4 targets are unchanged.
- Screen titles and icons follow the existing `title()`/`icon()` pattern.

## §9 Backups and restore — Frozen for 1.1

### §9.1 Files

| Prefix | Created by | Retention |
|---|---|---|
| `auto-YYYY-MM-DD.db` | daily snapshot (existing) | newest 7 |
| `manual-YYYY-MM-DD_HH-MM-SS.db` | `CreateBackup` | never auto-pruned; the user deletes in Settings |
| `pre-purge-YYYY-MM-DD_HH-MM-SS.db` | trash retention | newest 5, **except** the one written by an in-progress operation |
| `pre-restore-YYYY-MM-DD_HH-MM-SS.db` | restore | newest 5, **except** the one written by an in-progress operation |
| `pre-import-YYYY-MM-DD_HH-MM-SS.db` | import | newest 5, same protection |
| `pre-migration-vN-…db` | `Db::open` (existing) | never auto-pruned |

- Timestamps in names are UTC, matching existing code; the UI shows local time.
- Pruning only touches `.db` files with a known prefix in `backups_dir`.
- `AppDirs` gains `staging_dir = root/staging`. Its contents are app-owned temporary files, deleted at startup and after each operation.

`BackupFile { path, kind: BackupKind, created_utc: DateTime<Utc>, suffix: u32, bytes: u64 }`. Unrecognized names are not listed. `suffix` is the `-N` a second backup written in the same second carries (`…_HH-MM-SS-1.db`), so two files in one second still order deterministically.

### §9.2 Manual backup

`backup::create_manual(conn, dirs, now_utc) -> Result<BackupFile>` reuses `vacuum_into`, so the copy is integrity-checked before it gets its final name.

`backup::delete_manual(dir, path) -> Result<()>` deletes one manual backup from the Data tab, after confirmation. It refuses any path outside `backups_dir`, any file that is not a regular file, and any kind other than `Manual`. A stale row (already deleted on disk) reports the I/O error instead of pretending it worked.

### §9.3 Restore pipeline

Each step is a named seam. Tests inject a failure after each one.

1. **Resolve:** reject the live database path (canonicalized), non-files, and empty files.
2. **Stage:** open the source read-only and copy it with the backup API to `staging_dir/restore-<ts>.db`. The user's source file is never modified.
3. **Validate the staged copy:**
   - `PRAGMA integrity_check == ok`
   - `user_version` in `1..=latest`
   - `application_id == 0x41544E41`, or `user_version == 1` with the exact 0001 table set — established **before** the version check, so a foreign database carrying a high `user_version` is `NotAthena`, not `TooNew`
   - `page_size` equal to the live database's
4. **Upgrade the staged copy:** run migrations on it, then `integrity::check` must be empty, a profile row must exist, and after onboarding exactly one non-archived owner must remain.
5. **Snapshot:** `vacuum_into(live) → pre-restore-<ts>.db`. A failure aborts the restore.
6. **Copy:** `conn_mut().restore(Main, staged, None)`. An interruption rolls back inside SQLite; the live data stays intact.
7. **Verify:** `integrity::check(live)` must be empty. If not, restore from the step-5 snapshot with the same API, then report the error with both paths.
8. **Cleanup:** delete the staged file; prune by §9.1.

Failure results:
- Steps 1–5: `AppError::Restore(RestoreError::…)`, with `NotAthena`, `TooNew`, `Corrupt`, `PageSize`, `Invalid(Vec<IntegrityIssue>)`, `SnapshotFailed`. Live data is untouched.
- Step 6: live data is untouched (test it).
- Step 7: rollback to the snapshot.

After success: `data_version` bumps, the view model fully reloads, `UiState` clears filters and returns to Month, and the toast plus `last_operation` name the snapshot.

Restore is synchronous on the UI thread in 1.1, with a measured budget of < 1 s for a 50,000-transaction file on the reference machine. The confirmation dialog prevents double submission. Moving restore to a worker is a 1.3 concern if the budget fails.

## §10 Document formats — Frozen for 1.1

### §10.1 Envelope and dispatch

1. Read at most **64 MiB**; parse to `serde_json::Value`.
2. If `"format": "athena-profile"` is present, the version is `format_version`.
3. Otherwise, if `schema_version` is present, it is format 1 (1.0.x exports).
4. Otherwise reject as `NotAthena`.
5. A version > `FORMAT_VERSION` → `TooNew { found, supported }`, before any further parsing or writing.
6. Each version deserializes into its own frozen DTO module, `export/format_v1.rs` and `export/format_v2.rs`, with `#[serde(deny_unknown_fields)]` on every struct. A field addition therefore requires a new format version. That is intended.

### §10.2 Format 2 (written by 1.1)

```json
{
  "format": "athena-profile",
  "format_version": 2,
  "app_version": "1.1.0",
  "exported_at": "2026-11-16T14:03:00Z",
  "profile": { "start_balance_cents": 0, "theme": "dark", "language": "pt-BR", "onboarding_complete": true },
  "people": [{ "id": 1, "name": "A3", "kind": "owner", "archived": false, "created_at": "…" }],
  "categories": [{ "id": 1, "name": "Food", "color": "#FF8A65", "icon": null, "applies_to": "expense",
                   "is_default": true, "archived": false, "created_at": "…" }],
  "transactions": [{ "id": 7, "title": "…", "amount_cents": 1000, "date": "2026-11-14", "kind": "settlement",
                     "category_id": null, "payer_id": 2, "description": null, "split_mode": null,
                     "created_at": "…", "updated_at": "…",
                     "allocations": [{ "person_id": 1, "amount_cents": 1000 }] }]
}
```

- Only live transactions are exported. The import preview and README state that trash is not part of the document; a database backup is the way to restore trash.
- Ordering follows the existing deterministic order (people and categories by id; transactions by the existing export order).
- The writer still re-reads and compares the written bytes, as today.

### §10.2.1 Format 3 (written by 1.2)

Format 3 is format 2 plus a required `templates` array (`id, name, title, amount_cents, kind, category_id, payer_id, responsibility, description, created_at, last_used_at, allocations: [{ person_id, amount_cents }]`), ordered by id. Formats 1 and 2 import with no templates. A version above 3 is `TooNew` before any body parsing. Import replaces templates in the same transaction (children and parents before people and categories, inserted after them). <!-- lint:allow -->

### §10.2.2 Format 4 (written by 1.3)

Format 4 is format 3 plus an optional `appearance` object (§14.2). Portable fields are theme mode, preset, accent, density, text scale, motion, reduce transparency, quality, background (`none|builtin`), fit, dim and paused; a local wallpaper exports as `none`. Formats 1–3 and a format-4 document without `appearance` clear the appearance row in the replacement transaction. A version above 4 is `TooNew` before any body parsing; invalid enums or ranges in `appearance` are an import validation error. <!-- lint:allow -->

### §10.2.3 Format 5 (written by 1.4)

Format 5 is format 4 plus required `goals` (each with nested `contributions`) and `closings` arrays (§14.3). Goals export in id order with contributions in id order, closings in month order. Formats 1–4 import with no goals or closings. A version above 5 is `TooNew` before any body parsing. Import replaces both collections in the same transaction (closings and contributions before goals, people and transactions are deleted; goals, contributions and closings are inserted after their parents), never regenerates a saved fingerprint, and rejects a dangling reference or an inconsistent snapshot. A closing whose largest-expense row is not in the live export is written with a null `largest_expense_id` and keeps its title and amount. <!-- lint:allow -->

### §10.3 Format 1 adapter

`format_v1.rs` freezes the 1.0 shapes (domain-derived serde as of `2093257`, `schema_version: 1`).

- `templates` must be empty. 1.0 never wrote templates, so a non-empty value is `Invalid`.
- Transactions get `created_at = updated_at = exported_at`, a documented fallback. The preview says "Entry times weren't recorded in this file."
- Format 1 cannot contain settlements.

### §10.4 Validation (`ImportError`, all collected, at most 50 reported)

- **Envelope:** valid version; `exported_at` is RFC 3339.
- **Profile:** `onboarding_complete` is true. An export from a setup that never finished is rejected (`OnboardingIncomplete`) rather than imported as an empty-looking profile.
- **Ids:** positive and unique per collection.
- **People:** exactly one owner; the owner is not archived; names are non-blank after trim; the names of **active** people are unique after a Unicode fold (NFD, diacritics stripped, lowercased), matching how the pickers compare them. An archived person's name may be reused.
- **Categories:** names non-blank and unique after the same Unicode fold; color `#RRGGBB`; valid scope.
- **Transactions:**
  - title non-blank; amount > 0; valid calendar date; known kind
  - category exists (archived allowed). A non-settlement entry whose category scope does not allow its kind is **not** rejected: the category is widened to `CategoryScope::Both`, the widened names are reported in `ImportPlan.widened_categories`, and the preview says so. This keeps history from 1.0.x exports whose categories were narrowed afterwards.
  - payer exists (archived allowed); a payer on income is accepted as stored and ignored by settlement
  - allocation people exist with no duplicates; non-empty allocations sum exactly (checked)
  - `split_mode` rules; settlement shape per §5.2
  - timestamps RFC 3339
- **Aggregates & settlement overflow:** per-month and lifetime `income`, `expenses`, `refunds`, `income + refunds`, `expenses - refunds`, `net`, and `start_balance + net` fit in `i64` (matching `MonthSummary::new` and `tracked_balance`); additionally, scanning valid settlement-eligible transactions in deterministic `date ASC, id ASC` order, cumulative per-person `paid` and `owed` fit in `i64` at every row, and `balance = paid - owed` plus debtor negation (`0 - balance`) fit in `i64` at every distinct month-end boundary and final totals.

### §10.5 Import pipeline

1. The user picks a file, then the app runs read + dispatch + adapt + validate into `ImportPlan`.
2. Preview shows:
   - source file name, `app_version`, `exported_at`, format
   - incoming counts against current counts, **including the trash count that will be emptied**
   - the widened categories, when validation had to widen any (§10.4)
   - the consequence ("Replaces everything in Athena")
   - the snapshot location
3. Confirm, then `ImportProfile(plan)`.
4. Snapshot `pre-import-<ts>.db`; a failure aborts.
5. One SQL transaction:
   - delete allocations, transactions (all, deleted included), people and categories
   - update the profile
   - insert with explicit ids and timestamps
   - run `foreign_key_check` and `integrity::check`
   - commit, or roll back on any issue
6. Refresh as for restore (§9.3, last paragraph).

Import is replace-only in 1.1. Merge is out of scope.

## §11 Startup recovery — Frozen for 1.1 (P2)

```rust
enum AppSession { Active(AppCore), Recovery(RecoveryState) }
```

`AthenaApp` holds the session. `Store.db` stays a non-optional `Db`.

- **Entered when** `Db::open` fails with `Corrupt`, `Migration`, `SchemaTooNew`, `Db`, or `Io`, when `recovery-intent.json` validation/reconciliation hits a conflict, when live database metadata is indeterminate (unreadable or non-regular file), or before opening when the live database is missing/empty alongside a recognized backup, SQLite sidecar, staged file, moved-aside `broken-*` file, or folder scan failure. Only a confirmed absent live database with zero remnants and clean scans is first launch. `AlreadyRunning` and `Dirs` keep the message-box path.
- **The recovery screen** (solid surface, no wallpaper in later releases) shows the localized error, the data folder, and the backups list (read-only listing). Actions: **Open data folder**, **Quit**, and **Restore this backup**. For `SchemaTooNew`, restore warns that newer data will be replaced.
- **Restore from recovery:**
  1. Reconcile any prior `recovery-intent.json` first; fail closed on any conflict. Validate as in §9.3 steps 1–4 (`page_size` skipped).
  2. Make the upgraded candidate a standalone database and flush it. Record a validated relative-filename `recovery-intent.json` containing the candidate length/SHA-256 and each live original's suffix (`""`, `"-wal"`, `"-shm"`, `"-journal"`) length/SHA-256.
  3. Move original live files aside without clobbering (`broken-<ts>.db…`), advance the durable intent phase, and publish the complete candidate at the live path without overwriting an occupied destination.
  4. Re-confirm published identity (never treating live path presence alone as proof of publication), clean staging/intent, `Db::open`, then switch to `Active`.
  - If moving the originals or publishing the candidate is interrupted or fails, verified reconciliation puts originals back (main database first, never attaching old sidecars to an unexpected DB) and fails closed while preserving all bytes on any conflict.
- **Start fresh** is offered only when the live database path is confirmed missing or empty (not when live metadata is indeterminate or a recovery conflict exists). Confirm the choice before creating an empty profile, recheck the live path at the actual move boundary (never moving an unexpected non-empty live database), and preserve existing backups.
- **Scope & durability boundary:** `athena.lock` serializes concurrent Athena instances. Concurrent external writers are outside lock guarantees and trigger fail-closed `RecoveryConflict` preservation if live/staged/backup files diverge. Publication/rollback protects against process interruption (`SIGKILL`/crash); surviving sudden OS/disk power loss additionally depends on Windows filesystem metadata/journal ordering because directory entries are not flushed directly via directory handles on Windows.

## §12 UI and copy — Frozen for 1.1

- **Quick Add** gains a Settlement mode (kind selector):
  - fields: From (payer), To, Amount, Date, Title, Description
  - category and split controls are hidden, and their values cleared when the mode is entered
  - switching kinds with unsaved changes keeps the shared fields (title, amount, date, description)
  - when opened via **Record** while viewing a month other than `today`'s month, shows a non-blocking warning stating the viewed month and that the settlement will be dated `today`
- **Month:** the settlement card sits in the rail. It shows the sentence or transfers (or `settlement_unavailable` / `settlement_no_balances` when applicable), an "as of" date, a **Record** button per transfer, and a coverage line when exclusions exist: "2 entries without payer · 5 with no one responsible", where each count opens its repair filter.
- **Trash screen:**
  - header explaining the 30-day rule and that daily backups keep the latest 7 daily snapshots
  - rows: deleted date, transaction date, title, amount, kind
  - actions: **Restore**, **Delete forever** (confirm), **Empty trash** (confirm, states the count)
  - focus moves to the nearest remaining row after removal
- **Settings is split into pages** (§13, Phase 0 refactor): General, People, Categories, **Data & backups**. Data & backups contains:
  - data folder
  - export JSON/CSV
  - Import…
  - Back up now; a failure shows inline on the card and as a toast, and does not drop the page's drafts
  - backups list with Restore… per row, Delete… on manual rows (confirmed), and Restore from file…
  - the latest operation notice
  - Changing a category's scope is refused with its own message (`SettingsError::ScopeInUse`) when entries already use it and the new scope would hide them; the generic save failure is reserved for everything else.
- Successful restore/import clears filters and opens Month.
- **1.4 additions (§14.3):** the sidebar gains Goals (`Ctrl+5`, Lucide `target`); Month gains a "Saved closing" card below the live totals and Calendar gains saved/stale markers on year cards and the month header; Settings → Data & backups workbooks gain full-history Goals and Closings sheets.
- **1.3 additions (§14.2):** Settings gains an Appearance page (theme and presets, accent/density/text scale, background, accessibility) with a live shell preview, Apply/Cancel/Reset and a dirty-leave guard; General links to it instead of switching the theme directly. Wallpaper spans Month, Calendar, Settings and Trash behind an opaque-card shell.
- **1.2 additions (§14.1):** Create Quick Add gains a Templates picker, Save as template and a one-line entry field with a Review button; Settings gains a Templates page; the sidebar becomes a labeled icon rail when content would be narrower than 480 pt; Calendar and Month expose a day grid with a shared day detail panel; Settings → Data & backups gains XLSX export for the viewed month and all history.
- **Dialogs** return keyboard focus to the control that opened them.
- **All new copy** is added to both languages in the same commit, including error variants for every `RestoreError` and `ImportError`.

## §13 Verification — Frozen for 1.1

- **Definition of done:** [`docs/dod.md`](../../dod.md) plus:
  - every task names its tests before implementation
  - CI is green
  - snapshots are regenerated only by deliberate tasks and visually inspected
- **Standard verification footer** (every task):
  - `cargo fmt --check`
  - `cargo clippy --all-targets --locked -- -D warnings`
  - `cargo test --locked`
  - `docs/superpowers/doc-lint.sh` (when docs change)
- **Release:**
  - `cargo build --release --locked`
  - Inno Setup build
  - `docs/smoke-1.1.md` executed **against the recorded SHA-256 of the final `athena.exe` and installer**. A rebuild invalidates earlier native checks.
  - v1→v3 upgrade of a real 1.0.2 database
  - failure-injection suites (§9.3, §10.5) pass
  - both themes and languages, 100/150/200 % scaling, keyboard-only pass on Month/Trash/Settings

## §14 Draft contracts for later releases

These reserve names and rules. They are not frozen.

### §14.1 1.2 Daily speed — Frozen for 1.2

Implementation design for 1.2.0 (schema 4, document format 3). Rationale and test lists live in the [Daily Speed plan](../plans/2026-11-18-athena-1.2.0-daily-speed.md) and the [roadmap](../plans/2026-11-18-athena-stalled-ui-roadmap-plan.md).

- **`0004_templates.sql`:**
  - `templates`: `id, name, title, amount_cents (> 0), kind (expense|income|refund), category_id → categories ON DELETE RESTRICT, payer_id → people ON DELETE RESTRICT, responsibility ('none'|'person'|'even'|'custom'), description, created_at, last_used_at (nullable)`
  - `template_allocations (template_id → templates ON DELETE CASCADE, person_id → people ON DELETE RESTRICT, amount_cents ≥ 0, UNIQUE(template_id, person_id))` <!-- lint:allow -->
  - Settlements are not templatable. Migrations `0001`–`0003` are untouched.
  - Format 3 adds a required `templates` array with nested allocations, and the `Template { id, draft, created_at, last_used_at }` domain type replaces the 1.0 placeholder's single `person_id`. <!-- lint:allow -->
- **Template rules.** Names and titles are trimmed. Amount is positive cents. Names are unique under the trimmed `domain::text::fold`, enforced on each save and across imported names. `None` has no allocations; `Person` has exactly one full share; `Even` and `Custom` have at least two distinct participants; allocation totals equal the amount. `Even` uses `split_evenly` in ascending person-id order, so remainder cents are stable; `Custom` keeps explicit zeros. Income uses `None` with no payer and a compatible category. Stored or imported archived references stay readable; a new or modified template must use active people and a compatible active category. A category that later changes scope keeps its templates portable; they are listed as needing repair, and import/integrity check only id validity and allocation shape.
- **Applying a template** fills the form only (no write). The date becomes `vm.today` (visible). Amount edits re-split even splits and require repair for custom ones. Archived or incompatible references must be reassigned before saving a new entry. `SaveTransactionFromTemplate(TxDraft, template_id)` inserts the entry and sets `last_used_at` in one transaction with one timestamp.
- **Template actions:** `SaveTemplate(TemplateDraft)`, `UpdateTemplate(i64, TemplateDraft)`, `DeleteTemplate(i64)`, `SaveTransactionFromTemplate(TxDraft, i64)`; `RefreshScope::Templates` (the last also refreshes `Month`).
- **Dirty-draft guard.** Quick Add compares raw field values and participant selections with the snapshot taken at open, confirmed replacement, or successful create. Close, Escape, outside click, and Apply Template / Replace With Parsed Entry route through a modal-owned typed pending intent with Keep Editing / Discard. Window close with a dirty form uses the same confirmation through viewport cancel-close.
- **One-line entry:** `<amount> <title> [@YYYY-MM-DD]`, parsed by the pure `domain::quick_entry::parse_line`. The first Enter (or the visible Review button) parses into a reviewable Create draft and never saves in the same frame. A negative amount asks for Expense or Refund; zero or unrepresentable amounts are errors.
- **Duplicate check** runs in `handle_quick_add` (app layer), never in `quick_add::show`, for every Create/Edit submit. It matches live rows (all kinds, independent of filters) by date, positive cents and normalized title (`fold`, trimmed, whitespace collapsed), excludes the edited row and Trash, and offers Keep Editing / Save Anyway bound to the exact pending action and match set. A lookup error blocks saving.
- **Day drill-down:** `DayActivity { date, spending, activity_count }`; every live kind counts toward activity; spending is expense minus refund only. `TxFilter` gains an exact `day` predicate. Heat reflects only positive net spending.
- **Insights:** `InsightInputs` in `domain/reports.rs` with the localized messages and half-up `i128` percentage rules of the roadmap.
- **XLSX:**
  - writer `rust_xlsxwriter`; `calamine` is a dev-dependency used only to read workbooks back in tests
  - sheets Entries and Allocations, scope row, ISO dates and ids as exact text; user strings are written as explicit strings
  - numeric money cells only for |cents| ≤ 9,999,999,999,999 (verified by cent round-trip); larger values become exact two-decimal text labeled "Exact text"
  - published atomically with `atomic_write`
- **Design tokens (`ui/tokens.rs`) and an embedded icon set (`ui/icons.rs`)**: Lucide 0.468.0 24-unit path data vendored under `assets/icons/` with the upstream license replace the Unicode glyphs. Content narrower than 480 pt collapses the sidebar to a labeled icon rail.

### §14.2 1.3 Appearance — Frozen for 1.3

Implementation design for 1.3.0 (schema 5, document format 4). Rationale and test lists live in the [Native Appearance plan](../plans/2026-11-17-athena-native-appearance.md) and the [roadmap](../plans/2026-11-18-athena-stalled-ui-roadmap-plan.md). Production Glass is **not** part of 1.3; it stays behind the separate gates recorded in the [glass spike](../../design/athena-glass-spike.md).

- **`0005_appearance.sql`:** a single-row `appearance` table (`id = 1`), leaving `profile.theme` untouched as the legacy fallback. A row exists only after Apply or a format-4 import with appearance. Closed CHECKs match the domain allowlist:
  - `theme_mode` (`system|dark|light`), `preset` (`onyx|pearl|aurora`), `accent` (`blue|teal|violet`), `density` (`comfortable|compact`), `text_scale` (`100|115|130|150`)
  - `motion` (`system|reduced|full`), `reduce_transparency` (0/1), `quality` (`auto|low|high`)
  - `wallpaper_kind` (`none|builtin|local`), `wallpaper_asset_id` (required only for `local`; opaque lowercase 32-hex), `fit` (`fit|fill`), `dim` (0–100), `paused` (0/1)
- **Defaults** (no row, or Reset): theme follows the legacy profile theme; Onyx is stock Dark, Pearl is stock Light; blue; comfortable; 100; motion System; Reduce transparency off; Auto; wallpaper None, Fill, dim 0, not paused. Presets set theme and accent in the draft only (Onyx: Dark + blue, Pearl: Light + blue, Aurora: Dark + teal); the Aurora background is the embedded static `builtin` gradient chosen in the Background control. "Customized" shows when settings differ from the preset recipe.
- **Accent colors** (Dark / Light): blue is stock; teal `#59DDCC` / `#006F66`; violet `#C2ACFF` / `#5939A6`. Composed states are checked with palette contrast helpers; role-safe stock colors are the fallback. Status colors stay semantic.
- **Density and scale:** Compact maps the 8/12/16/24 spacing steps to 6/8/12/18 and the 36 pt control height to a 32 pt minimum; controls never shrink below their measured wrapped text. Text scale multiplies every application `FontId` through shared typography helpers; OS DPI and hit testing are independent.
- **Motion and transparency:** High contrast forces Solid and Reduced. Motion System follows the OS client-area animation preference (unknown means Reduced); Full opts into motion regardless. User Reduce transparency, or OS transparency off or unknown, forces opaque chrome.
- **Actions:** `ApplyAppearance(Box<AppearanceApply>)` writes the full row in one transaction (publishing a staged managed asset first when one is attached); `ResetAppearance` deletes the row. Both use `RefreshScope::Appearance`, which stages only the optional appearance read. An unreadable or invalid row is logged without paths or personal data, surfaces a localized notice, and keeps the financial workspace available (solid legacy theme). Import validation rejects invalid appearance and aborts that import.
- **Platform preferences:** `src/infrastructure/platform/windows_prefs.rs` (direct `windows-sys`, scoped `#[allow(unsafe_code)]`) behind a portable `PlatformPreferences` provider returns optional animation, high-contrast (plus system colors) and transparency (`EnableTransparency` DWORD). Read at startup and on focus regain/resume, plus a two-second check only while focused and visible. Failure returns unknown.
- **Media** lives under `root/appearance/` as managed `<id>.media` files (opaque lowercase 32-hex ids). Sources are at most 16 MiB, 2048 px per axis and 300 frames; retained RGBA is at most 128 MiB (active + preview) after downscale, charged with checked `width * height * 4` before retention; quality tiers cap frames at 1920×1080 (Auto), 960×540 (Low) and 2048×2048 (High) and never upsample. The directory quota is 256 MiB and 32 files. Symlinks and reparse points are rejected. The file is published and synced before the DB transaction; a failed Apply leaves at most an unreferenced managed file. No automatic collection in 1.3. Database backups keep the reference and resolve it under the current root; a missing file falls back to solid with a localized notice.
- **Playback rules:** GIF delays of 10 ms or less become 100 ms; absent loop metadata is one play-through, finite repeats follow the first play, infinite loops repeat; the last frame holds when finished. Uploads are at least 33.34 ms apart. Reduced motion shows frame 0; user Pause freezes the frame; blur, hidden or minimized suspends elapsed time without catch-up; at most two GPU textures with same-size sub-updates. No decorative wake-ups while paused, reduced or minimized.
- **Rendering rules:**
  - Never use `egui::Image` or the `egui_extras` loaders for wallpaper. <!-- lint:allow -->
  - Paint the wallpaper on `Order::Background` behind the shell chrome; cards, inputs, rows and modals stay opaque.
  - Fit letterboxes on the palette canvas; Fill is a centered aspect-cover crop (one shared geometry helper).
  - The scrim is `p.bg` at the minimum alpha computed in 8-bit steps so ordinary, muted and accent text over pure black and pure white is at least 4.5:1 and focus or essential icons at least 3:1; Dim is clamped to that floor at render time. High contrast omits the wallpaper. Missing media or a failed decode or texture uses a solid canvas plus a localized notice.
- **Glass** is not selectable in 1.3. It needs a revised contract and every gate in the spike record (swapchain p95, focused idle, native DPI, iGPU/dGPU, device-loss fallback).
- **Format 4** (written by 1.3) is format 3 plus optional `appearance` with the portable typed scalars and a `none|builtin` background. A local wallpaper exports as `none` while keeping fit, dim and paused; asset ids, paths and bytes are never exported. Formats 1–3 and a format-4 document without `appearance` clear the appearance row on replacement (solid legacy theme). A version above 4 is `TooNew` before body parsing.

### §14.3 1.4 Goals & closings — Frozen for 1.4

Implementation design for 1.4.0 (schema 7, document format 5). Domain rationale lives in the [goals and closing brief](2026-11-13-athena-1.2.0-revision-brief.md) (its release, migration and format numbers are superseded by §2) and the [roadmap](../plans/2026-11-18-athena-stalled-ui-roadmap-plan.md). Goal money is independent of the ledger: goals and contributions never create or edit transactions and never change balances, settlements or summaries.

#### Goals (`0006_goals.sql`)

- `goals(id, name, target_cents, target_date, frequency, attribution_id, status, pre_archive_status, created_at, updated_at)`:
  - `name` trimmed and nonempty (duplicate names allowed; ids distinguish goals); `target_cents > 0`; `target_date` optional ISO date (future or past); `frequency` is `daily|weekly|monthly`; `attribution_id` optional `people(id)` with `ON DELETE RESTRICT`
  - `status` is `active|paused|completed|archived`; `pre_archive_status` (`active|paused|completed`) is set exactly when `status = 'archived'`
  - normalized `YYYY-MM-DD` dates and RFC 3339 UTC metadata
- `goal_contributions(id, goal_id, amount_cents, date, note, created_at)`: `goal_id` → `goals ON DELETE RESTRICT`, `amount_cents > 0`, optional trimmed note, index on `(goal_id, date, id)`.
- **Lifecycle** (`GoalTransition`): Active → Pause | Complete; Paused → Resume | Complete; Completed → Reopen (to Active); Archive from Active, Paused or Completed stores the current status in `pre_archive_status`; Restore returns to it. Any other transition is a validation error, checked inside the write transaction. Completing below target requires an explicit UI confirmation that shows the remaining money; the repository does not second-guess it. Edits (`UpdateGoal`) and contribution add/delete are allowed only in Active or Paused, including when already Funded. Pausing keeps history and still accepts deliberate contributions.
- **Progress.** `contributed` is the checked `i64` sum of contributions; an addition or import whose total would overflow fails inside its transaction. `remaining = max(target − contributed, 0)`, `excess = max(contributed − target, 0)`, the bar is capped at 100 % and the percentage is computed in `i128` (floor). `Funded` is derived as `contributed ≥ target`, independent of the manual status. Removing contributions or raising the target only recomputes `Funded`; it never rewrites the manual status.
- **Attribution.** A new or changed attribution must be an active person. An unchanged or imported attribution may be archived.
- **Dates.** A contribution written through an action must have a valid date no later than the app's local today (the Store's date provider; tests freeze it). Imports keep valid historical dates whatever the clock says.
- **Schedule** `contribution_plan(goal, contributed, today) -> ContributionPlan { Hidden | DueToday(remaining) | Overdue(remaining) | PerPeriod { periods, per_period } }`: hidden when funded, not Active, or without a target date; `DueToday` on the target date and `Overdue` after it, both with the full remaining amount. Otherwise periods are the day difference (daily), `ceil(days / 7)` (weekly), or the smallest positive count of month advances from today that reaches or passes the target (monthly). Each advance anchors on today's day of month and clamps to the destination month's last day, so Jan 31 → Feb 28/29 is one period and Jan 31 → Mar 1 is two. `per_period = ceil(remaining / periods)` in cents (101 cents over two periods is 51).
- **Actions** (`RefreshScope::Goals`): `CreateGoal(GoalDraft)`, `UpdateGoal(i64, GoalDraft)`, `ChangeGoalStatus(i64, GoalTransition)`, `AddContribution(i64, ContributionDraft)`, `DeleteContribution { goal_id, contribution_id }`. The outcome carries the created goal id so the screen can select it. A delete whose contribution belongs to another goal is `NotFound`. `integrity::check` gains goal shape, contribution ownership and checked-total issues.

#### Goals screen and cache

- `Screen::Goals` on `Ctrl+5` with the Lucide `target` icon and a visible shortcut hint; the sidebar order is `[Month, Calendar, Goals, Trash, Settings]` and `Ctrl+1`–`Ctrl+4` are unchanged.
- `GoalsState { selected, filter, pending_focus }` in `UiState`; `ViewModel` holds `goals: Vec<GoalListItem>` (summary with contributed cents) and `goal_detail: Option<GoalDetail>` (goal plus contributions, `date DESC, id DESC`). The selected id is part of the load key, so changing it reloads only the detail. `RefreshScope::Goals` stages both together. Selection clears on import/restore, when it leaves the filtered partition, or when its id is missing. `LoadKey.today` reloads schedules at midnight; `AthenaApp` schedules its own local-midnight repaint (DST- and clock-change-aware) and refreshes on resume, independent of wallpaper or focus polls.
- Layout: Active (default), Paused, Completed and Archived tabs and a primary New goal; two panes at ≥ 720 pt of content, a list → detail drill-in with Back below that. Detail has one primary action (Add contribution for Active/Paused, Reopen for Completed, Restore for Archived) and a labeled menu for the rest. Confirmations: contribution delete (date and amount), archive, and complete below target. Amounts are tabular and update on commit without animation.

#### Closings (`0007_closings.sql`)

- `closings` keyed by canonical `month TEXT` (`YYYY-MM`, month 01–12): `closed_at` (RFC 3339 UTC), `income_cents`, `expenses_cents`, `refunds_cents`, `result_cents` (`income − expenses + refunds`, checked), `entry_count ≥ 0`; `previous_spending_cents` and `previous_count ≥ 0` of the preceding month; `top_category_id` (→ `categories ON DELETE RESTRICT`, nullable) with `top_category_net_cents` (positive; NULL means no winner, a positive amount with a NULL id is Uncategorized); `largest_expense_id` (→ `transactions ON DELETE SET NULL`, nullable) with `largest_expense_title` and `largest_expense_cents` (positive) kept as historical fallbacks after a purge; `fingerprint_version` (`1`) and the two 64-char lower-case hex fingerprints `current_fingerprint` and `previous_fingerprint`. CHECKs and `integrity::check` require consistent optional groups.
- **Capture** reuses the 1.2 `InsightInputs` (full completed month and full predecessor) from one consistent SQL snapshot taken in the same transaction as the write. The live-only responsibility note is not stored. `CloseMonth(YearMonth)` fails if a closing exists; `RecalculateClosing(YearMonth)` requires one and replaces it completely. Both require a completed local month, `month < YearMonth::from_date(today)`, enforced at action execution against the Store's date provider. Saved values stay historical even when live totals differ; labels (category names) resolve live.
- **Comparison record (version 1).** For month `M` the canonical bytes are the compact UTF-8 `serde_json` serialization of `{"version":1,"month":"YYYY-MM","rows":[…]}` with `rows` sorted by `(date, id)` and each row `{"id","date","kind","amount_cents","category_id","title","created_at","updated_at"}` in that field order, covering every live **financial** row (`expense|income|refund`) dated in `M`; settlements, descriptions, allocations and display labels are excluded. The empty month hashes `rows: []`. The fingerprint is the lower-case hex SHA-256 of those bytes (explicit `sha2` dependency). `current_fingerprint` covers the closed month and `previous_fingerprint` its predecessor. Fingerprints are never derived from aggregates or wall-clock order.
- **Staleness.** A closing is stale when either recomputed fingerprint differs from the stored one. The check runs on data, month, year, import/restore and date invalidations, never per frame: Month checks the viewed month and predecessor; Calendar checks closed months of the displayed year with one bounded year-plus-predecessor scan reusing adjacent months. Filter changes do not recompute. A failed check shows an unavailable/retry state and never claims freshness. Recalculation clears the marker.
- **UI.** Month shows a distinct "Saved closing" card below the live totals (date saved, month and comparison month, figures, saved insight, category, largest expense), a `Close month` action for an unclosed completed month and `Recalculate closing` for a saved one, each behind a confirmation naming the month; recalculation states that the saved snapshot is replaced and ledger rows stay editable. Staleness reads "Changed since closing" with an icon and text. Category names resolve live; the expense shows its captured title and amount, with `Open entry` only while that row is live. Calendar year cards and the month header carry an accessible saved / changed / unavailable marker that opens Month.
- **Actions** (`RefreshScope::Closings`): `CloseMonth(YearMonth)`, `RecalculateClosing(YearMonth)`.

#### Format 5 and workbook

- `format_v5.rs::DocumentV5` is format 4 plus required `goals` (nested `contributions`) and `closings`, ordered as in §10.2.3. Validation covers ids, references, `YYYY-MM` months, lifecycle shape, snapshot consistency, fingerprint shape and overflow; nothing is regenerated. `tests/fixtures/format5-1.4.0.json` is LF.
- **Workbook.** Goals and Closings sheets are always labeled "Full history", even for a viewed-month export; `Entries` and `Allocations` keep their requested scope. All cells follow the 1.2 numeric-or-exact-text and explicit-string policy. The receipt count still equals the ledger rows.
- JSON exports live ledger rows and structured history; SQLite backups keep Trash and local media references.

## §15 Change log

- 2026-11-16 — Created. Froze 1.1 (§4–§13) after D8/D9 and technical decisions T1–T5.
- 2026-11-16 — Added `RestorePerson(i64)` and `ToastKey::PersonRestored` (§8.1, §8.2) so archived settlement parties can be restored in place (§7.3).
- 2026-11-16 — Added `pre-purge-YYYY-MM-DD_HH-MM-SS.db` verified Athena snapshot guard before automatic trash retention (§6.2, §9.1).
- 2026-11-16 — Froze atomic `ViewModel::reload` staging, `settlement_unavailable` / `refresh_failed` state with same-key retry suppression, and `AppCore::apply` committed outcome preservation (§8.2, §8.3).
- 2026-11-16 — Updated settlement card visibility (`balances.len() >= 2 || excluded.any() || settlement_unavailable`, §7.3, §12) and added non-blocking Quick Add viewed-month date warning on `Record` (§7.4, §12).
- 2026-11-16 — Cleared stale `RestoreTransaction` Undo toast on purge, added `UndoFailed` toast (§6.3), and clarified the 7-day rolling window for daily backups in Trash copy (§12).
- 2026-11-16 — Expanded `§10.4` `ImportError::Overflow` to cover per-month/lifetime report sums and chronological (`date ASC, id ASC`) cumulative settlement balances/negations (D-P5), and added the 9th `Payer`/`Pagador` CSV column to `§5.3` (D-P6).
- 2026-11-17 — Review pass. `§9.1`: added `BackupFile.suffix` for two backups in one second; `§9.2`: added `delete_manual`. `§9.3`: identity is established before the `TooNew` check, and upgrade also enforces the single non-archived owner. `§10.4`: names compared after a Unicode fold, active names unique, `OnboardingIncomplete`, and legacy scope mismatches widen the category instead of failing (removed `CategoryScopeMismatch`); preview names the widened categories. `§11`: recovery restore skips the page-size comparison. `§12`: manual-row Delete…, inline backup failure that keeps drafts, and `SettingsError::ScopeInUse`.

- 2026-11-17 - Final review: restore owner validation applies after onboarding; daily snapshot dates display as calendar dates even across timezone/DST changes; recovery cleans staging and rolls back partial broken-file moves. Successful restore/import opens Month with cleared filters.

- 2026-11-18 — §11 rejects implicit onboarding after an interrupted recovery, preserves the rollback journal, publishes a complete candidate, and requires explicit confirmation to start fresh.
- 2026-11-18 — §11 adds a startup recovery probe distinguishing first launch, missing/empty with evidence or scan failures, indeterminate live metadata, and intent conflicts, plus durable `recovery-intent.json` reconciliation with checksum-verified publish and fail-closed rollback.
- 2026-11-18 — §11 hardens non-clobbering file moves without destructor ownership of existing files, shares post-publish verification across normal and reconciled paths, rechecks live state at move/sidecar boundaries, keeps blocked recoverable directories non-fatal at startup, and documents `athena.lock` and process-kill vs power-loss boundaries.
- 2026-11-18 — Froze §14.1 for 1.2.0 (schema 4, format 3): templates, dirty-draft guard, one-line entry, duplicate consent, day drill-down, insights, XLSX and tokens/icons. Synced §2 (release ledger), §8 (template actions, `RefreshScope::Templates`), §10 (format 3 envelope: required `templates`, accepted formats 1–3) and §12 (Quick Add template/one-line controls, Settings Templates page, icon rail). Historical 1.1 behavior is unchanged.
- 2026-11-19 — Froze §14.2 for 1.3.0 (schema 5, format 4): single-row `appearance` preferences, Windows platform preferences, managed local wallpaper (static + GIF) with bounded decode/playback and a contrast-floor scrim, Settings → Appearance, and the portable local→none export rule. Synced §2 (release ledger), §8 (`ApplyAppearance`/`ResetAppearance`, `RefreshScope::Appearance`), §10 (format 4 envelope, accepted formats 1–4) and §12 (Appearance page). Glass stays unshipped behind the spike gates. Historical 1.1/1.2 behavior is unchanged.
- 2026-11-19 — Froze §14.3 for 1.4.0 (schema 7, format 5): goals and contributions (lifecycle with stored pre-archive status, derived Funded, checked totals, calendar-aware contribution plan), `Ctrl+5` Goals screen and cache, monthly closings with version-1 SHA-256 comparison fingerprints and staleness, Month/Calendar closing surfaces, and format 5 plus full-history workbook sheets. Synced §2 (release ledger), §8 (goal and closing actions, `RefreshScope::Goals`/`Closings`, sidebar order), §10 (format 5 envelope, accepted formats 1–5) and §12 (Goals, closing card, markers). §6.4 is unchanged: closings read live rows through `live_transactions`. Historical 1.1–1.3 behavior is unchanged.
