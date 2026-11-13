#!/usr/bin/env python3
"""Build Athena's pinned Salted Flutters Polaris subsets.

This is a one-off asset step, not part of `cargo build`. Requires fontTools 4.68+.

    git -c core.autocrlf=false clone https://github.com/ppolkadot/Salted-Flutters-Polaris.git
    git -c core.autocrlf=false -C Salted-Flutters-Polaris checkout e4d7c2f56c0aff6c6ff1d0751f17cf146a5ecfde
    python tools/fonts/build.py --source-dir Salted-Flutters-Polaris --out-dir assets/fonts
"""

from __future__ import annotations

import argparse
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

STYLES = ("Regular", "Medium", "Semibold", "Bold")
TABULAR_SOURCE = "Semibold"
COPYRIGHT = "Salted Flutters Polaris - Created by ppoolar"

# Basic Latin, Latin-1, Latin Extended-A, general punctuation, currency, arrows and the
# symbols Athena uses. This keeps Portuguese, R$, minus, arrows and bullets.
CHARS = "".join(
    chr(cp)
    for cp in (
        list(range(0x20, 0x7F))
        + list(range(0xA0, 0x180))
        + list(range(0x2000, 0x2070))
        + list(range(0x20A0, 0x20D0))
        + list(range(0x2190, 0x2200))
        + [0x2212, 0x2260, 0x2264, 0x2265, 0x25CF, 0x25CB, 0x2713]
    )
)


def source_path(source_dir: Path, style: str) -> Path:
    return source_dir / f"SaltedFluttersPolaris-{style}.otf"


def subset_font(source: Path) -> TTFont:
    options = subset.Options()
    options.layout_features = ["*"]
    options.name_IDs = ["*"]
    options.name_legacy = True
    options.name_languages = ["*"]
    options.notdef_outline = True
    options.recalc_bounds = True
    options.recalc_timestamp = False
    options.drop_tables += ["MERG", "meta", "trak"]
    font = subset.load_font(source, options)
    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text=CHARS)
    subsetter.subset(font)
    font.recalcTimestamp = False
    return font


def rename(font: TTFont, family: str, style: str) -> None:
    values = {
        1: family,
        2: style,
        3: f"{family} {style}; 1.001; ATHENA; 2026-11-13",
        4: f"{family} {style}",
        6: f"{family.replace(' ', '')}-{style}",
        16: family,
        17: style,
    }
    for record in font["name"].names:
        if record.nameID in values:
            record.string = values[record.nameID]


def freeze_tabular_digits(font: TTFont) -> None:
    order = set(font.getGlyphOrder())
    for table in font["cmap"].tables:
        for codepoint in range(0x30, 0x3A):
            if codepoint not in table.cmap:
                continue
            alternate = table.cmap[codepoint] + ".tnum"
            if alternate in order:
                table.cmap[codepoint] = alternate

    cmap = font.getBestCmap()
    widths = {
        font["hmtx"][cmap[codepoint]][0]
        for codepoint in range(0x30, 0x3A)
    }
    if len(widths) != 1:
        raise SystemExit(f"tabular digits do not share one advance: {sorted(widths)}")


def save(font: TTFont, out_dir: Path, name: str) -> None:
    target = out_dir / name
    font.save(target)
    check = TTFont(target)
    copyright = check["name"].getDebugName(0)
    if copyright != COPYRIGHT:
        raise SystemExit(f"{name}: copyright line changed: {copyright!r}")
    print(f"{name}: {target.stat().st_size} bytes, {len(check.getGlyphOrder())} glyphs")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)

    for style in STYLES:
        font = subset_font(source_path(args.source_dir, style))
        rename(font, "Athena Polaris", style)
        save(font, args.out_dir, f"AthenaPolaris-{style}.otf")

    font = subset_font(source_path(args.source_dir, TABULAR_SOURCE))
    freeze_tabular_digits(font)
    rename(font, "Athena Polaris Tabular", TABULAR_SOURCE)
    save(font, args.out_dir, "AthenaPolaris-Tabular.otf")


if __name__ == "__main__":
    main()
