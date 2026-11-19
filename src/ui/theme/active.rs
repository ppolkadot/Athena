//! The resolved look the current frame paints with. `AthenaApp` sets it when the effective
//! appearance changes; [`super::palette`], the typography helpers and the layout tokens read it so
//! painters and widgets always agree. It is per thread because egui draws on one thread, and
//! unset means the stock palette at 100% scale with comfortable spacing.

use std::cell::Cell;

use super::Palette;
use crate::domain::settings::Theme;

/// Everything that changes how egui is styled, compared to decide whether to restyle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleKey {
    pub theme: Theme,
    pub palette: Palette,
    /// Text scale in percent (`100`, `115`, `130`, `150`).
    pub text_scale: u16,
    pub compact: bool,
}

thread_local! {
    static ACTIVE: Cell<Option<StyleKey>> = const { Cell::new(None) };
}

/// Make `key` the look for this thread, or restore the defaults with `None`.
pub fn set(key: Option<StyleKey>) {
    ACTIVE.with(|a| a.set(key));
}

#[must_use]
pub fn get() -> Option<StyleKey> {
    ACTIVE.with(Cell::get)
}

#[must_use]
pub fn text_scale_percent() -> u16 {
    get().map_or(100, |k| k.text_scale)
}

#[must_use]
pub fn compact() -> bool {
    get().is_some_and(|k| k.compact)
}

/// Restores the defaults when dropped, so a test cannot leak its look into the next one.
#[cfg(test)]
pub(crate) struct ResetOnDrop;

#[cfg(test)]
impl Drop for ResetOnDrop {
    fn drop(&mut self) {
        set(None);
    }
}
