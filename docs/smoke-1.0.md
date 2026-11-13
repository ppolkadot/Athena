# Athena 1.0.x smoke checklist

Run this on the release build (`target\release\athena.exe`), ideally on a clean Windows profile or
VM with no Rust installed. Tick each box. A failure blocks the release.

Automated tests cover the logic behind most of these and drive every screen in a test harness, but
they cannot show a real window, real fonts, the native file dialogs or a second process. Those are
what this list is for.

`[x]` means it was checked on the real release exe (portable mode, Windows 11, Portuguese system
locale) by driving the window with UI Automation and real keyboard and mouse input on 2026-11-12.
`[ ]` means it still needs a person. Notes after a box say what was observed.

## 1.0.1 type and fonts
- [ ] Month, Calendar, Settings, and onboarding use the embedded Polaris letterforms.
- [ ] Amounts such as `R$ 1.234,56` align digit-for-digit in the register and calendar.
- [ ] Portuguese accents (`ã õ ç á é í ó ú â ê ô à ü`) render without tofu at 13 px and 15 px.
- [ ] The refund arrow `↩` renders through the Segoe UI Symbol fallback.
- [ ] The search empty-state icon `⌕` renders through the Segoe UI Symbol fallback; record it if it does not.
- [ ] Dark and light themes use the same type hierarchy.
- [ ] The sidebar or About text shows Version 1.0.1.
- [ ] The installer places `LICENSE.txt` under the app directory at `licenses\Salted-Flutters-Polaris\LICENSE.txt`. (Compiled `target\installer\Athena-1.0.1-setup.exe` with Inno Setup 6.7.3 bundling `assets\fonts\LICENSE.txt`; interactive install not run.)

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
- [ ] A very long title ends in an ellipsis and does not push the amount off screen. (Covered by a test only.)
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
- [ ] Light theme is readable in the real window. (Seen only in test snapshots.)
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
