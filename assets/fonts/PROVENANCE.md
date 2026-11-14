# Font provenance

Athena embeds modified subsets of Salted Flutters Polaris.

- Upstream: https://github.com/ppolkadot/Salted-Flutters-Polaris
- Commit: e4d7c2f56c0aff6c6ff1d0751f17cf146a5ecfde (2026-11-04)
- Source files: SaltedFluttersPolaris-Regular.otf, SaltedFluttersPolaris-Medium.otf, SaltedFluttersPolaris-Semibold.otf, SaltedFluttersPolaris-Bold.otf
- Output files: AthenaPolaris-Regular.otf, AthenaPolaris-Medium.otf, AthenaPolaris-Semibold.otf, AthenaPolaris-Bold.otf, AthenaPolaris-Tabular.otf
- Generator: tools/fonts/build.py with fontTools 4.68.0
- Changes: Latin subset; family renamed to "Athena Polaris"; required copyright line preserved; `Bold` normalizes `OS/2.usWeightClass` from `600` to `700` while preserving upstream Bold outlines; tabular variant (`TABULAR_SOURCE = "Semibold"`) is derived from `Semibold` and remaps U+0030 through U+0039 to the source `.tnum` glyphs.
- License: assets/fonts/LICENSE.txt is the upstream license, byte-for-byte (3,274 bytes, SHA-256 931886d07182110715ccee3504a076b59929503e4ebacadbe0da789184850a61).

The upstream family is under development. Athena pins this commit and ships its own renamed subsets; it does not follow upstream updates automatically.
