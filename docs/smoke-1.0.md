# Athena 1.0.x smoke checklist

Run this on the release build (`target\release\athena.exe`), ideally on a clean Windows profile or
VM with no Rust installed. Tick each box. A failure blocks the release.

Automated tests cover the logic behind most of these and drive every screen in a test harness, but
they cannot show a real window, real fonts, the native file dialogs or a second process. Those are
what this list is for.

`[x]` means it was checked on the real release exe (portable mode, Windows 11, Portuguese system
locale) by driving the window with UI Automation and real keyboard and mouse input. The original
1.0.0 run was on 2026-11-12; the 1.0.1 checks below were run on 2026-11-13.
`[ ]` means it still needs a person. Notes after a box say what was observed.

## 1.0.1 type and fonts
- [x] Month, Calendar, Settings, and onboarding use the embedded Polaris letterforms.
- [x] Amounts such as `R$ 1.234,56` align digit-for-digit in the register and calendar.
  (Compared register amounts `1.111,11`, `9.876,54`, and `1.234,56`; inspected calendar totals.)
- [x] Portuguese accents (`ã õ ç á é í ó ú â ê ô à ü`) render without tofu at 13 px and 15 px.
  (Used the full sequence in transaction titles and category labels, plus the onboarding name.)
- [x] The refund arrow `↩` renders through the Segoe UI Symbol fallback.
- [x] The search empty-state icon `⌕` renders through the Segoe UI Symbol fallback.
  (Typed a query with no matches and waited for the search debounce; the icon is a magnifier.)
- [x] Dark and light themes use the same type hierarchy.
- [x] The sidebar or About text shows Version 1.0.1.
  (Also verified the executable's FileVersion and ProductVersion are both `1.0.1`.)
- [x] The installer places `LICENSE.txt` under the app directory at `licenses\Salted-Flutters-Polaris\LICENSE.txt`.
  (Recompiled with Inno Setup 6.7.3, silently installed into an isolated test directory, and
  compared the installed executable and license byte-for-byte with the build and source.
  Launched the installed executable, then silently uninstalled it. The installer and uninstaller
  exited 0, and the installed executable and uninstall registration were removed.)

Run conditions: copied the release executable and `LICENSE.txt` into a fresh portable directory
under `target/smoke-1.0.1-20261113-151609/`. Completed onboarding and saved an expense through the
UI, then added isolated SQLite fixtures for a refund, contrasting tabular digits, a long title,
and accent coverage. No normal Athena profile was used. The installed executable was launched
with a portable marker too; a separate clean Windows machine and default-path installation
remain untested.

Additional 1.0.1 checks: `Ctrl+N`, `Ctrl+1/2/3`, Tab, Enter, and Esc worked; a successful save
refocused the amount field, and a refused Enter kept focus on the invalid amount. After closing
and reopening, all four fixture entries and the Portuguese/light settings persisted without
onboarding. Portable and installed logs contained no ERROR or panic entries. The portable
license was present beside the executable. The test installation was removed and test windows
were closed.

Stress-test finding (fixed): a long user-defined category name previously overlapped its total in
the Month screen's category rail and clipped beside the amount in a transaction row (observed in
both themes with the full accent test string). Fixed in `src/ui/screens/month.rs::rail_row` and
`src/ui/components/transaction_row.rs::show` by truncating long category names with an ellipsis
and a hover tooltip, verified by `long_category_names_do_not_overlap_rail_or_row_amounts`.

Current release artifact SHA-256 (rebuilt after Semibold/Bold font update and category truncation fix):

- `athena.exe`: `22CFBCE029E2ED5149DE6F8129CC5A8F32724E67DBFD346E23675489907C6A52`
- `Athena-1.0.1-setup.exe`: `3561C6B478DF022249C46602044D08AD45C6F4BB8B10F9976E9EF2D3C310AB25`

## 1.0.2 verification notes (2026-11-14)
- Automated suite on `1.0.2`: `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` (265 unit tests + 1 `design_assets` test + 5 `fonts` tests), `cargo build --release --locked`, `docs/superpowers/doc-lint.sh`, and Inno Setup 6.7.3 compilation of `installer/athena.iss` all passed after the fallback-height fix.
- Contrast & selection verification: automated WCAG AA >= 4.5:1 checks passed across `bg`, `panel`, `card`, and `card_hover` in both Dark and Light themes (`#3F55C6` accent, `#147D52` positive, `#B83247` negative, `#7A5B00` warning; selection fill alpha `36`), and all 10 regenerated PNGs in `src/ui/snapshots/` were visually inspected.
- Font asset normalization: `tools/fonts/build.py` now sets `OS/2.usWeightClass = 700` on `AthenaPolaris-Bold.otf` (keeping distinct Bold glyph outlines and `macStyle = 1`), verified in `tests/fonts.rs` (`Weight::Normal/Medium/SemiBold/Bold`, `TABULAR` derived from `Semibold`).
- Adaptive layout checks: tested `month`, `transaction_row`, and `quick_add` layouts headlessly at `1100x760`, minimum window (`760x560`), `200%`-scale logical viewport (`640x410` / `372 px` row), and `280 px` ultra-narrow row in English and Portuguese with long titles/categories, multi-person long names, and `i64::MAX` cents (`−R$ 92.233.720.368.547.758,07`).
- Scrolling-register regression: reproduced fallback rows consuming the viewport height, then bounded all three standalone right-aligned fallbacks with horizontal rows. The regression test verifies compact height and a visible following row at `372 px` (long names and maximum amounts) and `140 px` in both languages. Independent headless probes confirmed row heights of `136 px`, `136 px`, and `212 px`, respectively.
- Packaging and same-machine launch checks:
  - [x] `target\release\athena.exe` FileVersion and ProductVersion are both `1.0.2`.
  - [x] Before the fallback-height fix: `Athena-1.0.2-setup.exe` silently installed to an isolated temp directory, placed byte-identical `athena.exe` and `licenses\Salted-Flutters-Polaris\LICENSE.txt`, launched cleanly, and silently uninstalled (`exit 0`, directory removed).
  - [x] Before the fallback-height fix: isolated portable release launch (`athena.portable`) created `data\athena.db`, `data\backups\auto-2026-11-14.db`, and `data\logs\athena.log.2026-11-14` logging `athena 1.0.2 starting` with no errors or panics.
  - [ ] Installer and portable launch checks have not been repeated for the rebuilt artifacts below.
  - [ ] Live interactive multi-screen UI walk-through, real `200%` Windows DPI scaling window check, and clean Windows machine testing remain untested for `1.0.2`.

1.0.2 release artifact SHA-256 after the fallback-height fix (superseded by 2026-11-16 final artifacts below):

- `athena.exe` (17,915,904 bytes): `9D870D1092CD7BE288C499316D23949F00A11BDEA3FA576C43A34E3EE995BEDD`
- `Athena-1.0.2-setup.exe` (7,499,213 bytes): `EF7D70D031CADC6A7336D744D00BF5208DB8DD47B05C004B2AC86FE1EE354A9F`
- `assets/fonts/LICENSE.txt`: `931886D07182110715CCEE3504A076B59929503E4EBACADBE0DA789184850A61`

### 1.0.2 final artifacts (2026-11-16)

Built from `25b5bb8` after Phase 0 Task P0-1 (`rail_rows_stay_compact_in_a_scrolling_rail` passed without reproducing the `ScrollArea` height expansion; `transaction_row.rs` bound parameterized by style; `266` unit + `1` `design_assets` + `5` `fonts` = `272` tests, `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo build --release --locked`, and Inno Setup 6.7.3 all passed).

Final release artifact SHA-256:

- `athena.exe` (`17,891,328` bytes): `98294141062842FB75D1844B1614D28658F67ED1CEA97A40282D6D8B4467D218`
- `Athena-1.0.2-setup.exe` (`7,492,215` bytes): `A02E7C5DA949C4516A80D8ABFEDCCDC1FCFD2ED8C2BE4B0E86D5A12C472C7E05`
- `assets/fonts/LICENSE.txt` (`3,274` bytes): `931886D07182110715CCEE3504A076B59929503E4EBACADBE0DA789184850A61`

Checks against these final hashes:

- [x] `target\release\athena.exe` (`98294141062842FB75D1844B1614D28658F67ED1CEA97A40282D6D8B4467D218`) FileVersion and ProductVersion are both `1.0.2`.
- [x] `Athena-1.0.2-setup.exe` (`A02E7C5DA949C4516A80D8ABFEDCCDC1FCFD2ED8C2BE4B0E86D5A12C472C7E05`) silently installed into an isolated temp directory, placed byte-identical `athena.exe` (`98294141062842FB75D1844B1614D28658F67ED1CEA97A40282D6D8B4467D218`) and `licenses\Salted-Flutters-Polaris\LICENSE.txt` (`931886D07182110715CCEE3504A076B59929503E4EBACADBE0DA789184850A61`), launched cleanly, and silently uninstalled (`exit 0`, installed `athena.exe` removed).
- [x] Isolated portable release launch of `athena.exe` (`98294141062842FB75D1844B1614D28658F67ED1CEA97A40282D6D8B4467D218`) with `athena.portable` stayed running and created `data\athena.db`, `data\backups\auto-2026-11-16.db`, and `data\logs\athena.log.2026-11-16` logging `athena 1.0.2 starting` and `daily snapshot written` with no errors or panics.
- [ ] Live interactive multi-screen UI walk-through (Month, Calendar, Settings in both languages and themes) on `98294141062842FB75D1844B1614D28658F67ED1CEA97A40282D6D8B4467D218`: not run interactively by a human in this session (automated headless kittest + snapshot suites passed).
- [ ] Real `200%` Windows DPI scaling window check on `98294141062842FB75D1844B1614D28658F67ED1CEA97A40282D6D8B4467D218`: not run in a live scaled desktop session (automated `640x410` logical scale layout test passed).
- [ ] Clean Windows machine check (no Rust installed): no separate clean machine available in this session; remains untested.

**Disposition (2026-11-16):** `1.0.2` changes are merged on `main`, and the final `1.0.2` artifact hashes above passed automated and packaging checks. By explicit user approval, the `v1.0.2` git tag and the remaining unchecked manual `1.0.2` smoke items above were waived and superseded by the `1.1.0` release verification (`smoke-1.1.md`).

## First run and persistence
- [x] First run onboarding: skip partner, valid balance, finish. The "Welcome" toast appears.
  (Entered a partner and a negative balance instead of skipping; the Skip path is covered by tests.)
- [x] Restart: no onboarding, data persists.
- [x] Data is created beside the exe in portable mode (`data\athena.db`). `%APPDATA%\Athena\` in the default mode is not re-checked.
- [x] Kill the app (Task Manager) mid-session. Relaunch. Last committed rows are present. (Done repeatedly while testing.)

## Empty states
- [x] No partner, no transactions. Month and calendar show guidance; no panic, no blank card, no `NaN`.
- [x] Negative starting balance accepted. Tracked balance correct (checked to the cent after several entries).

## Entering money
- [x] Money input accepts: `45`, `45.90`, `45,90`, `1.234,56`, `R$ 1.234,56`, `-R$ 10,00`, `R$ -10,00`, leading `-`.
  (Accepted in the real window: `45,90`, `200,00`, `0,01`, `-50,00`. The rest are covered by unit and property tests.)
- [x] Money input rejects with a clear message: `0,005` ("Use at most two decimal places."), blank ("Enter your name."/"Enter an amount."). Letters, `0.005`, `1,234.56` are covered by tests.
- [x] Add an expense for me, shared evenly, shared custom, shared three ways. (Partner-only was not separately tried.)
- [x] Selected person and payer are visible before saving ("For Shared · paid by Marta"). Payer defaults to the selected person, or the owner when shared.
- [x] Odd split: R$ 0,01 shared three ways saved as 1 + 0 + 0 cents.
- [x] Custom split with zero shares saves (50,00 / 0,00 / 0,00).
- [x] Add income and a refund. Month result is correct and the refund is not counted as income.
- [x] A shared purchase with a shared refund: person totals net correctly (Marta R$ 191,23 checked by hand).
- [x] Enter saves, Esc closes, and after a save focus is back on the amount field.
- [x] A refused Enter (bad amount, empty title) leaves the keyboard on the field that needs fixing.

## Browsing
- [x] Search, person filter and shared filter narrow the register. (Category filter not tried in the real window.)
- [x] Search ignores case and accents: `PAO` finds `Pão de queijo`.
- [x] Calendar: month cards, year stepping, the empty-year hint, and clicking a card opens that month. (December/January boundary is covered by tests.)
- [x] A very long title ends in an ellipsis and does not push the amount off screen.
  (Re-checked in the 1.0.1 release window in both themes; long category names now truncate with
  an ellipsis and hover tooltip as well, covered by layout tests.)
- [x] `Ctrl+N`, `Ctrl+1`, `Ctrl+2`, `Ctrl+3`, `Esc` work.

## Editing and history
- [x] Edit a transaction: every field and the payer are prefilled; Enter saves and closes. (Changing the payer itself was not tried.)
- [x] Delete asks first, names the entry; Cancel and Esc keep the row; confirming removes it and shows "Deleted “Centavo”."
- [x] Rename a category used by history: the old row shows the new name.
- [x] Rename the owner: sidebar and rows update.
- [x] Archive a person with history: the picker hides them, old rows keep their name, totals stay.
- [x] Archive a category with history: old rows and the breakdown still show it; the picker no longer offers it.

## Export
- [x] Export JSON and CSV from Settings. The save dialog opens on the exports folder with the standard name. Allocation amounts and archived names are preserved. The CSV has the BOM, `;`, decimal comma and localized headers.
- [ ] The CSV opens correctly in Excel.
- [x] The "Exported …" toast is visible after saving. (A bug made it vanish at once after a native dialog; fixed and re-checked in the real window. A very long path wraps; cosmetic.)
- [x] Export onto a locked file: Athena shows the failure toast, the old file is intact, no temp files left.
- [x] Cancelling the save dialog does nothing.

## Language and look
- [x] Switch language. Every screen, month name, the entry form and the export headers update without a restart.
- [x] Dark theme is the default with a near-black canvas; muted text is readable.
- [x] Light theme is readable in the real window. (Re-checked in the 1.0.1 release window.)
- [x] Signs and labels carry meaning, not color alone.
- [x] The sidebar icons, refund arrow and search glyph render as symbols, not empty boxes.
- [x] The window title bar shows the Athena icon; the exe's file properties show the name and version.
- [ ] The taskbar shows the Athena icon.

## Safety
- [x] Start a second copy: it shows "Athena is already running." and exits.
- [x] `athena.portable` next to the exe stores data in `data\` beside it.
- [x] `backups\auto-YYYY-MM-DD.db` is written at launch, once per day. (The cap of seven is covered by tests.)
- [x] Corrupt database (overwritten with text): a message box names the path and the recovery steps; the file is left alone.
- [x] Newer schema (`PRAGMA user_version = 99`): a message box explains; the file hash is unchanged.
- [ ] The release exe runs on a clean Windows machine without Rust. (A fresh portable launch and a silent install, launch and uninstall of `Athena-1.0.0-setup.exe` worked here; a separate machine is still untested.)

## Bugs this run found and fixed
- Enter in a text field made it lose the keyboard, so after a refused Enter the user had to click back in (onboarding and the entry form).
- The entry form grew taller than a small window with a shared split, pushing the heading and the Save and Cancel buttons off screen; its fields now scroll.
- A toast raised before a native dialog expired before it was drawn, so export confirmations never appeared.
- Negative figures mixed `-` and `−`; the all-time balance now uses the real minus sign like everything else.
- Long user-defined category names could overlap totals in the Month category rail and clip beside the amount in a transaction row; both now truncate with an ellipsis and keep the full name in a tooltip.
