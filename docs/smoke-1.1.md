# Athena 1.1.0 release smoke record

Date: 2026-11-17. Scope: Safe Data (settlement and paybacks, Trash and undo, backups and restore,
Format 2 JSON export and import). Contract: `docs/superpowers/specs/contracts.md` §13.

## Release artifacts

| File | SHA-256 |
|---|---|
| `Athena-1.1.0-setup.exe` (7,657,927 bytes) | `955D2EB202D40B698A6DD77214321B822AE6EC8DC5E5342F9C19C95A14469088` |
| `athena-1.1.0-windows-x64.exe` (18,738,688 bytes) | `0BA5FF211A52BEBC339E1BE2186A3055A6B0180CB8C6692857D49B6E5319DEBE` |
| `LICENSE.txt` (3,274 bytes) | `931886D07182110715CCEE3504A076B59929503E4EBACADBE0DA789184850A61` |

## Performed (against the artifacts above)

- [x] `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`
  (424 unit tests plus integration tests, 2 ignored), and `docs/superpowers/doc-lint.sh` pass on `1.1.0`.
- [x] `cargo build --release --locked` and the Inno Setup 6 compile of `installer/athena.iss` succeed.
- [x] Windows CI passed on the feature commit `d07a260` before the version bump.

## Not performed (no result is claimed)

- [ ] v1 to v3 upgrade of a real 1.0.2 data folder (installed and portable).
- [ ] Interactive settlement record/clear, delete/undo/Trash/purge, manual backup and restore
  (including from an external file), and 1.0.2/1.1 JSON import walk-throughs in the real window.
- [ ] Recovery screen with a corrupted copy of a real database.
- [ ] Both languages and themes; 100/150/200 % Windows scaling; keyboard-only pass in the real window.
- [ ] Installer install, launch and uninstall of `Athena-1.1.0-setup.exe`; clean Windows machine test.

These remain open and are not waived by this record.
