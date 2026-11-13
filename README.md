# Athena

A quiet, local-first desktop app for one household: you and your partner.

Log an expense, income or refund in seconds. See the month register: every entry, the totals, the
category split and the person split. Browse the year in a calendar and click a month to open its
log. No cloud, no accounts, no network.

Version 1.0.0. English and Brazilian Portuguese. Windows is the primary target.

## Running it

Download or build `athena.exe` and run it. There is no installer.

```powershell
cargo run --release          # build and run from source
cargo build --release        # produces target\release\athena.exe
```

Building needs a current stable Rust (1.95 or newer).

## Where your data lives

| Mode | Data folder |
|---|---|
| Default | `%APPDATA%\Athena\` |
| Portable | `data\` next to `athena.exe`, enabled by an empty file named `athena.portable` beside the exe |

Inside the data folder:

- `athena.db`: everything, in one SQLite file (money is stored as whole cents).
- `backups\`: automatic safety copies (see below).
- `logs\`: daily log files. Transaction titles and descriptions are never logged.
- `exports\`: the suggested folder for exports.
- `athena.lock`: stops a second copy of Athena from opening the same data.

Settings, then Data, shows the exact path and opens the folder.

## Keyboard

| Keys | Action |
|---|---|
| `Ctrl+N` | New transaction, from any screen |
| `Enter` | Save in the open form |
| `Esc` | Close the open form or dialog |
| `Ctrl+1` / `Ctrl+2` / `Ctrl+3` | Month, Calendar, Settings |

After saving a new entry the form stays open with the amount field focused, ready for the next one.

## Backups and exports

- **Automatic snapshots.** At most once a day, when Athena starts, it writes
  `backups\auto-YYYY-MM-DD.db` and keeps the newest seven. Before any upgrade of an existing
  database it also writes `backups\pre-migration-vN-<time>.db`.
- **Export JSON** (Settings, Data) is the complete profile: settings, people, categories and every
  transaction with its per-person split. Money is in integer cents.
- **Export CSV** is the readable report for one month or all time. It uses `;` as the delimiter and
  a decimal comma, with a byte order mark, so it opens correctly in Excel in Brazil. Text that
  starts with `=`, `+`, `-` or `@` is prefixed with `'` so a spreadsheet never runs it as a formula.
- Exports are written to a temporary file, checked, then renamed into place. A failed export never
  damages an existing file.

## If something goes wrong

**"Athena is already running."** Another copy has the data open. Close it, or end the process in Task Manager.

**"The database file ... could not be read."** The file is damaged.

1. Close Athena.
2. Rename `athena.db` to `athena.db.bad`. Also remove `athena.db-wal` and `athena.db-shm` if they exist.
3. Copy the newest file from `backups\` (for example `auto-2026-11-12.db`) to `athena.db`.
4. Start Athena. You lose only what was entered since that backup.

**"This data was created by a newer version of Athena."** Install the newer version. Athena does not
open or change a database it does not understand.

A failed upgrade is rolled back and leaves the previous data intact; the message names the
`pre-migration` backup it made first.

## Privacy

Athena makes no network connections. Everything stays on this computer. Back up the data folder
(or the exports) yourself if you want a copy elsewhere.

## What 1.0 does and does not do

Included: onboarding, people (owner, partner, others), who paid and who is responsible, shared
expenses split exactly to the cent, refunds, categories, search and filters, month register, year
calendar, settings, JSON and CSV export, dark and light themes, English and Portuguese.

Not in 1.0: multiple accounts or transfers, bank import, sync, budgets, recurring entries, goals,
attachments, multi-currency. Deleting a transaction is permanent (a trash and undo arrive in 1.1).

## Development

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Design: `docs/superpowers/specs/`. Implementation plans: `docs/superpowers/plans/`. Definition of
done for a task: `docs/dod.md`. Release smoke checklist: `docs/smoke-1.0.md`.

Pixel snapshots of every screen live in `src/ui/snapshots`. After an intended visual change run
`UPDATE_SNAPSHOTS=1 cargo test snapshot_` and review the new images.
