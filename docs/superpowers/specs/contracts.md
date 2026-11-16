# Athena contracts

**Status:** Canonical. Sections marked **Frozen for 1.1** are the binding interface for the [1.1.0 Safe Data plan](../plans/2026-11-16-athena-1.1.0-safe-data.md). Sections marked **Draft** reserve names and rules for later releases; they become frozen when their release plan is written.
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
| 1.2.0 | Daily speed | reserved `0004_templates` (4) | reserved 3 |
| 1.3.0 | Appearance | reserved `0005_appearance` (5) | reserved 4 |
| 1.4.0 | Goals & closings | reserved `0006_goals`, `0007_closings` (7) | reserved 5 |

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

`SUMMARY_SELECT`, `summaries_for_year`, and every `CASE t.kind` aggregate must exclude settlement explicitly. A test seeds one row of each kind and asserts every row of this table.

## §6 Trash — Frozen for 1.1

### §6.1 Semantics

- `DeleteTransaction(id)` sets `deleted_at` to now (UTC RFC 3339, second precision) and leaves `updated_at` alone. It fails with `NotFound` for a missing or already-deleted id.
- `RestoreTransaction(id)` clears `deleted_at`. Restored rows may reference archived people or categories; that is history.
- `PurgeTransaction(id)` hard-deletes one *deleted* row; allocations cascade. It fails for a live row. `EmptyTrash` purges all deleted rows.
- `UpdateTransaction` on a deleted row returns `NotFound`. Rows must be restored before they can be edited.

### §6.2 Retention

- Deleted rows older than **30 days** are purged at startup, only after that launch's daily snapshot succeeded or already existed. If the snapshot failed, skip purging and log it.
- Clock-rollback guard: skip purging when `now < max(deleted_at)`.
- A forward clock jump may purge early, but the same launch's snapshot holds those rows. The Trash screen states both the 30-day rule and the snapshot fallback.

### §6.3 Undo

- Undo is offered for `DeleteTransaction` only, through a toast action that applies `outcome.inverse`.
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

  More than one transfer renders as a list. Fewer than two participants hides the card.
- Archived parties get an "(archived)" suffix. Their transfer offers "Restore Ana to record this" instead of **Record**, because new rows may not reference archived people.

### §7.4 Recording and repair

- **Record** opens Quick Add in Settlement mode prefilled with from, to, amount, `date = vm.today`, and title "Ana → Bia" (editable). It never saves automatically.
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
| existing profile/people/category actions | unchanged | unchanged | |

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
- `ToastKey` gains `TransactionRestored`, `TrashEmptied`, `BackupCreated`, `Restored`, `Imported`.
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
| `settlement: Option<SettlementView>` | always (cheap) | §7 service |
| `trash_count: usize` | always | `trash_repo::count` |
| `trash: Vec<TrashItem>` | `screen == Trash` | `trash_repo::list` (newest first) |
| `backups: Vec<BackupFile>` | `screen == Settings` | `backup::list` (file I/O; not per frame) |
| `last_operation: Option<OperationNotice>` | set from outcomes | in-session only |

`TrashItem { tx: Transaction, deleted_at: String }`.

### §8.4 Screens and shortcuts

- `Screen::Trash` is added. The sidebar order is `Screen::ALL = [Month, Calendar, Trash, Settings]`.
- Shortcuts come from an explicit table, **not** from `ALL` indices: `Ctrl+1` Month, `Ctrl+2` Calendar, `Ctrl+3` Settings, `Ctrl+4` Trash, and later `Ctrl+5` Goals. A test asserts the table.
- Screen titles and icons follow the existing `title()`/`icon()` pattern.

## §9 Backups and restore — Frozen for 1.1

### §9.1 Files

| Prefix | Created by | Retention |
|---|---|---|
| `auto-YYYY-MM-DD.db` | daily snapshot (existing) | newest 7 |
| `manual-YYYY-MM-DD_HH-MM-SS.db` | `CreateBackup` | never auto-pruned; the user deletes in Settings |
| `pre-restore-YYYY-MM-DD_HH-MM-SS.db` | restore | newest 5, **except** the one written by an in-progress operation |
| `pre-import-YYYY-MM-DD_HH-MM-SS.db` | import | newest 5, same protection |
| `pre-migration-vN-…db` | `Db::open` (existing) | never auto-pruned |

- Timestamps in names are UTC, matching existing code; the UI shows local time.
- Pruning only touches `.db` files with a known prefix in `backups_dir`.
- `AppDirs` gains `staging_dir = root/staging`. Its contents are app-owned temporary files, deleted at startup and after each operation.

`BackupFile { path, kind: BackupKind, created_utc: DateTime<Utc>, bytes: u64 }`. Unrecognized names are not listed.

### §9.2 Manual backup

`backup::create_manual(conn, dirs, now_utc) -> Result<BackupFile>` reuses `vacuum_into`, so the copy is integrity-checked before it gets its final name.

### §9.3 Restore pipeline

Each step is a named seam. Tests inject a failure after each one.

1. **Resolve:** reject the live database path (canonicalized), non-files, and empty files.
2. **Stage:** open the source read-only and copy it with the backup API to `staging_dir/restore-<ts>.db`. The user's source file is never modified.
3. **Validate the staged copy:**
   - `PRAGMA integrity_check == ok`
   - `user_version` in `1..=latest`
   - `application_id == 0x41544E41`, or `user_version == 1` with the exact 0001 table set
   - `page_size` equal to the live database's
4. **Upgrade the staged copy:** run migrations on it, then `integrity::check` must be empty and a profile row must exist.
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

### §10.3 Format 1 adapter

`format_v1.rs` freezes the 1.0 shapes (domain-derived serde as of `2093257`, `schema_version: 1`).

- `templates` must be empty. 1.0 never wrote templates, so a non-empty value is `Invalid`.
- Transactions get `created_at = updated_at = exported_at`, a documented fallback. The preview says "Entry times weren't recorded in this file."
- Format 1 cannot contain settlements.

### §10.4 Validation (`ImportError`, all collected, at most 50 reported)

- **Envelope:** valid version; `exported_at` is RFC 3339.
- **Ids:** positive and unique per collection.
- **People:** exactly one owner; the owner is not archived; names are non-blank after trim.
- **Categories:** names non-blank and unique case-insensitively (matching `COLLATE NOCASE`); color `#RRGGBB`; valid scope.
- **Transactions:**
  - title non-blank; amount > 0; valid calendar date; known kind
  - category exists and allows the kind (archived allowed)
  - payer exists (archived allowed); a payer on income is accepted as stored and ignored by settlement
  - allocation people exist with no duplicates; non-empty allocations sum exactly (checked)
  - `split_mode` rules; settlement shape per §5.2
  - timestamps RFC 3339
- **Aggregates:** the lifetime sums of income, expenses, refunds, and `start_balance + net` fit in `i64` (checked). SQLite `SUM` would otherwise fail later in reports.

### §10.5 Import pipeline

1. The user picks a file, then the app runs read + dispatch + adapt + validate into `ImportPlan`.
2. Preview shows:
   - source file name, `app_version`, `exported_at`, format
   - incoming counts against current counts, **including the trash count that will be emptied**
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

- **Entered when** `Db::open` fails with `Corrupt`, `Migration`, `SchemaTooNew`, or `Db`. `AlreadyRunning` and `Dirs` keep the message-box path.
- **The recovery screen** (solid surface, no wallpaper in later releases) shows the localized error, the data folder, and the backups list (read-only listing). Actions: **Open data folder**, **Quit**, and **Restore this backup**. For `SchemaTooNew`, restore warns that newer data will be replaced.
- **Restore from recovery:**
  1. Validate as in §9.3 steps 1–4.
  2. Rename the broken `athena.db`, `-wal` and `-shm` to `broken-<ts>.db…`, which frees the live path.
  3. Write the staged, upgraded copy to the live path with the backup API.
  4. `Db::open`, then switch to `Active`.
  - A failure at any step leaves the broken files renamed but intact, and stays in recovery.

## §12 UI and copy — Frozen for 1.1

- **Quick Add** gains a Settlement mode (kind selector):
  - fields: From (payer), To, Amount, Date, Title, Description
  - category and split controls are hidden, and their values cleared when the mode is entered
  - switching kinds with unsaved changes keeps the shared fields (title, amount, date, description)
- **Month:** the settlement card sits in the rail. It shows the sentence or transfers, an "as of" date, a **Record** button per transfer, and a coverage line when exclusions exist: "2 entries without payer · 5 with no one responsible", where each count opens its repair filter.
- **Trash screen:**
  - header explaining the 30-day rule and that daily backups keep removed items
  - rows: deleted date, transaction date, title, amount, kind
  - actions: **Restore**, **Delete forever** (confirm), **Empty trash** (confirm, states the count)
  - focus moves to the nearest remaining row after removal
- **Settings is split into pages** (§13, Phase 0 refactor): General, People, Categories, **Data & backups**. Data & backups contains:
  - data folder
  - export JSON/CSV
  - Import…
  - Back up now
  - backups list with Restore… per row, and Restore from file…
  - the latest operation notice
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

### §14.1 1.2 Daily speed (Draft)

- **`0004_templates.sql`:**
  - `templates` table: `id, name, title, amount_cents, kind (expense|income|refund), category_id, payer_id, responsibility ('none'|'person'|'even'|'custom'), description, created_at, last_used_at`
  - `template_allocations (template_id → templates ON DELETE CASCADE, person_id → people ON DELETE RESTRICT, amount_cents ≥ 0, UNIQUE(template_id, person_id))` <!-- lint:allow -->
  - Settlements are not templatable.
  - Format 3 adds `templates`, and the `Template` domain type replaces the 1.0 placeholder's single `person_id`. <!-- lint:allow -->
- **Applying a template** fills the form only. The date becomes `vm.today` (visible). Amount edits re-split even splits and require repair for custom ones. Archived references must be reassigned.
- **One-line entry:** the first Enter parses into a reviewable draft and never saves in the same frame. A negative amount asks for a kind.
- **Duplicate check** runs in `handle_quick_add` (app layer), never in `quick_add::show`. It excludes the edited row and the trash.
- **XLSX:**
  - choose the writer crate in the plan (a dependency decision); add `calamine` as a dev-dependency to read workbooks back in tests
  - numeric money cells only for |cents| ≤ 9,999,999,999,999 (verified by cent round-trip); larger values become text cells labeled as exact
- **Design tokens and an embedded icon set** (licensed, provenance documented) replace the Unicode glyphs.

### §14.2 1.3 Appearance (Draft)

- **`0005_appearance.sql`:** a single-row `appearance` table, leaving `profile.theme` untouched. Its CHECK allows only `dark|light`, and a SQLite CHECK cannot be altered in place. Fields:
  - `theme_mode` (system|dark|light), `preset`, `accent`, `density`, `text_scale`
  - `motion` (system|reduced|full), `reduce_transparency`, `quality`
  - `wallpaper_asset_id`, `fit`, `dim`, `paused`
- **Media** lives under `root/appearance/` as managed copies. Database backups keep the reference only. Format 4 adds an optional `appearance` section without media bytes or paths.
- **Rendering rules:**
  - Never use `egui::Image` or the `egui_extras` loaders for wallpaper. <!-- lint:allow -->
  - Paint the wallpaper on `Order::Background` behind transparent panels.
  - Use one or two GPU textures updated in place.
  - Downscale frames at decode time to the largest display size.
  - Budgets (source ≤ 16 MiB; retained ≤ 128 MiB after downscale) are enforced before retention.
  - Minimum scrim opacity per theme is computed so text over pure white and pure black is ≥ 4.5:1; the dimming control cannot go below it.
  - Glass is gated on `wgpu_render_state.is_some()` and on `examples/glass_spike.rs` passing its criteria.
- **Windows preferences** (reduced motion, high contrast, transparency) are read in `src/infrastructure/platform/windows_prefs.rs` with a direct `windows-sys` dependency and a scoped `#[allow(unsafe_code)]`, and refreshed on focus and resume.

### §14.3 1.4 Goals & closings (Draft)

- `0006_goals.sql` and `0007_closings.sql` implement the 1.2 brief:
  - `is_funded` is derived; `Completed` is manual; `pre_archive_status` is stored
  - checked contribution totals
  - closings store `previous_spending_cents`, `previous_count`, `top_category_id`, `top_category_net_cents`, and a canonical comparison record covering both months
- Format 5 adds goals, contributions and closings.
- Date-sensitive hints rely on `LoadKey.today` (§8.3).
- `Ctrl+5` opens Goals.

## §15 Change log

- 2026-11-16 — Created. Froze 1.1 (§4–§13) after D8/D9 and technical decisions T1–T5.
