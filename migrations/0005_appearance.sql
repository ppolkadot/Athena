-- Athena 1.3.0 appearance preferences (contract §14.2). One optional row; `profile.theme` stays as
-- the legacy fallback. A row exists only after Apply or a format-4 import with appearance.

CREATE TABLE appearance (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    theme_mode TEXT NOT NULL CHECK (theme_mode IN ('system', 'dark', 'light')),
    preset TEXT NOT NULL CHECK (preset IN ('onyx', 'pearl', 'aurora')),
    accent TEXT NOT NULL CHECK (accent IN ('blue', 'teal', 'violet')),
    density TEXT NOT NULL CHECK (density IN ('comfortable', 'compact')),
    text_scale INTEGER NOT NULL CHECK (text_scale IN (100, 115, 130, 150)),
    motion TEXT NOT NULL CHECK (motion IN ('system', 'reduced', 'full')),
    reduce_transparency INTEGER NOT NULL CHECK (reduce_transparency IN (0, 1)),
    quality TEXT NOT NULL CHECK (quality IN ('auto', 'low', 'high')),
    wallpaper_kind TEXT NOT NULL CHECK (wallpaper_kind IN ('none', 'builtin', 'local')),
    wallpaper_asset_id TEXT CHECK (
        wallpaper_asset_id IS NULL
        OR (length(wallpaper_asset_id) = 32 AND wallpaper_asset_id NOT GLOB '*[^0-9a-f]*')
    ),
    fit TEXT NOT NULL CHECK (fit IN ('fit', 'fill')),
    dim INTEGER NOT NULL CHECK (dim BETWEEN 0 AND 100),
    paused INTEGER NOT NULL CHECK (paused IN (0, 1)),
    updated_at TEXT NOT NULL,
    CHECK ((wallpaper_kind = 'local') = (wallpaper_asset_id IS NOT NULL))
);
