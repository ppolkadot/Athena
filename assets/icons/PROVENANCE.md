# Embedded icon provenance

- **Source:** [Lucide](https://lucide.dev) `lucide-static@0.468.0` (tag `0.468.0`), fetched from
  `https://cdn.jsdelivr.net/npm/lucide-static@0.468.0/icons/<name>.svg` at development time only.
  Athena makes no network calls at runtime.
- **License:** ISC, with portions MIT (Feather). The full upstream text is in `LICENSE.txt`
  (`https://github.com/lucide-icons/lucide/blob/0.468.0/LICENSE`). Release packaging ships it next
  to the Polaris font license.
- **Vendored SVGs:** `lucide/*.svg`, unmodified: `list`, `calendar-days`, `trash-2`, `settings`,
  `chevron-left`, `chevron-right`, `plus`, `x`, `pencil`, `arrow-down-left` (income),
  `arrow-up-right` (expense), `undo-2` (refund), `arrow-left-right` (settlement), `target` (Goals,
  1.4), `search`, `bookmark` (saved closing marker, 1.4) and `triangle-alert` (changed or
  unavailable closing marker, 1.4).
- **Mapping:** `python tools/icons/gen_icons.py` flattens every `path`, `circle`, `rect` and `line`
  of those SVGs (24-unit viewBox, 2-unit stroke, round caps/joins) into deterministic polylines in
  `src/ui/icon_paths.rs`. `src/ui/icons.rs` paints them as egui line shapes, so rendering needs no
  system font, runtime SVG loader or image dependency.
