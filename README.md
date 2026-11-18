# Athena

A quiet, local-first desktop app for one household: you and your partner.

Log an expense, income or refund in seconds. See the month register: every entry, the totals, the
category split and the person split. Browse the year in a calendar and click a month to open its
log. No cloud, no accounts, no network.

Version 1.1.1 (Safe Data): settlement balances and paybacks, Trash with undo, automatic, manual and
safety backups with in-app recovery and restore, and Format 2 JSON export and import. English and
Brazilian Portuguese. Windows is the primary target.

Type: Salted Flutters Polaris, embedded as Latin subsets with a tabular money face. The font
license and provenance live in `assets/fonts/`.

Portable builds: keep `assets/fonts/LICENSE.txt` beside `athena.exe` when redistributing; the
font license requires the license file to travel with the fonts.

## Running it

Download or build `athena.exe` and run it. The Inno Setup script is `installer/athena.iss`.

```powershell
cargo run --release          # build and run from source
cargo build --release        # produces target\release\athena.exe
```

Building needs a Rust 1.95 (pinned in `rust-toolchain.toml`).

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
| `Ctrl+1` / `Ctrl+2` / `Ctrl+3` / `Ctrl+4` | Month, Calendar, Settings, Trash |
| `Ctrl+Z` | Undo a visible deletion, except while typing |

After saving a new entry the form stays open with the amount field focused, ready for the next one.

## Backups and exports

- **Automatic snapshots.** After setup, each launch saves at most one verified
  `backups\auto-YYYY-MM-DD.db` for the local day and keeps the newest seven.
- **Manual backups.** Settings > Data & backups > Back up now creates a verified
  `manual-<time>.db`. Manual backups can be deleted individually after confirmation.
- **Safety snapshots.** Before restore, import, or automatic Trash purging, Athena saves a
  `pre-restore-`, `pre-import-`, or `pre-purge-` snapshot and keeps the newest five of each kind.
  `pre-migration-vN-<time>.db` snapshots protect upgrades. Manual and migration backups are
  never automatically pruned. All backups live in `backups\` inside the data folder.
- **Restore** accepts a listed backup or an external Athena database. It replaces everything,
  including Trash, after validation and a safety snapshot, then opens Month with cleared filters.
- **Export JSON** writes Format 2: settings, people, categories, live transactions, settlements,
  record timestamps, and per-person splits. Money is in integer cents. JSON excludes Trash.
- **Import JSON** accepts Format 1 (1.0.2) and Format 2 exports. Preview shows counts and any
  category scopes widened for legacy history. Import replaces all data and empties Trash after
  saving a safety snapshot; use a database backup when you need Trash restored too.
- **Export CSV** is the readable report for one month or all time. It uses `;` as the delimiter and
  a decimal comma, with a byte order mark, so it opens correctly in Excel in Brazil. Text that
  starts with `=`, `+`, `-` or `@` is prefixed with `'` so a spreadsheet never runs it as a formula.
- Exports are written to a temporary file, checked, then renamed into place. A failed export never
  damages an existing file.

## If something goes wrong

**"Athena is already running."** Another copy has the data open. Close it, or end the process in Task Manager.

**"The database file ... could not be read."** The file is damaged.

Athena opens a recovery screen with the available backups, the data folder, and Quit.
Restoring validates the backup first and preserves the damaged database and its sidecars as
`broken-<time>.db`, `broken-<time>.db-wal`, and `broken-<time>.db-shm` before reopening the app.

**"This data was created by a newer version of Athena."** Install the newer version. Athena does not
open or change a database it does not understand.

A failed upgrade is rolled back and leaves the previous data intact; the message names the
`pre-migration` backup it made first.

## Privacy

Athena makes no network connections. Everything stays on this computer. Back up the data folder
(or the exports) yourself if you want a copy elsewhere.

## Settlement and recoverable deletion

The Month settlement card shows cumulative shared balances through that month's end. Record a
suggested payback to prefill a Settlement entry with From/To people; saving it reduces the debt.
Settlements do not count as spending, income, or refunds.

Deleting an entry moves it to Trash. Undo restores a recent deletion; Trash lets you restore or
permanently delete individual entries, or empty Trash after confirmation. Entries older than
30 days are purged at startup only after a verified safety snapshot. Backups can retain older data.

Onboarding, people, exact shared splits, categories, search, the year calendar, CSV export,
themes, and both languages remain available. Bank import, sync, budgets, recurring entries,
goals, attachments, and multi-currency remain future work.

Release verification: automated checks (format, Clippy, tests, document lint) and the installer
build pass. The live interactive GUI walk-through, 100/150/200 % Windows scaling checks, and a clean
Windows machine test remain open; see `docs/smoke-1.1.md`.

## Development

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Design: `docs/superpowers/specs/`. Designer brief: `docs/design/athena-designer-brief.md`.
Implementation plans: `docs/superpowers/plans/`. Definition of done: `docs/dod.md`. Release smoke
checklist: `docs/smoke-1.0.md`.

Pixel snapshots of every screen live in `src/ui/snapshots`. After an intended visual change run
`UPDATE_SNAPSHOTS=1 cargo test snapshot_` and review the new images.

CI runs `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`,
`cargo test --locked`, and `docs/superpowers/doc-lint.sh` on `windows-latest`; snapshot goldens
render against Windows' WARP (`Microsoft Basic Render Driver`) DX12 adapter with a 150-pixel
tolerance.
