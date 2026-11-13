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
