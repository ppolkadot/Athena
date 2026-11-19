//! Windows preference reads: client-area animation, high contrast and system colors, and the
//! per-user transparency switch. Every read returns `None` on failure.
//!
//! This is the one module that calls Win32 directly, so it opts out of the workspace-wide
//! `unsafe_code` warning; each `unsafe` block states why it is sound.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::Graphics::Gdi::{
    COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor,
};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SystemParametersInfoW,
};

use super::{HighContrast, PlatformPreferences, PlatformPrefs, Rgb, SystemColors};

/// The Windows implementation of [`PlatformPreferences`].
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsPrefs;

impl PlatformPreferences for WindowsPrefs {
    fn read(&self) -> PlatformPrefs {
        PlatformPrefs {
            animations: client_area_animation(),
            transparency: transparency_enabled(),
            high_contrast: high_contrast(),
        }
    }
}

fn client_area_animation() -> Option<bool> {
    let mut enabled: i32 = 0;
    // SAFETY: `SPI_GETCLIENTAREAANIMATION` writes one BOOL through `pvParam`; `enabled` is a live
    // local of that size, and the other arguments are unused for this action.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&raw mut enabled).cast::<c_void>(),
            0,
        )
    };
    (ok != 0).then_some(enabled != 0)
}

fn colorref(value: u32) -> Rgb {
    // COLORREF is 0x00BBGGRR.
    let [r, g, b, _] = value.to_le_bytes();
    [r, g, b]
}

fn system_colors() -> SystemColors {
    // SAFETY: `GetSysColor` takes a color index and returns a value; it has no preconditions.
    let get = |index| colorref(unsafe { GetSysColor(index) });
    SystemColors {
        window: get(COLOR_WINDOW),
        window_text: get(COLOR_WINDOWTEXT),
        highlight: get(COLOR_HIGHLIGHT),
        highlight_text: get(COLOR_HIGHLIGHTTEXT),
    }
}

fn high_contrast() -> Option<HighContrast> {
    let mut info = HIGHCONTRASTW {
        cbSize: u32::try_from(size_of::<HIGHCONTRASTW>()).ok()?,
        dwFlags: 0,
        lpszDefaultScheme: null_mut(),
    };
    // SAFETY: `SPI_GETHIGHCONTRAST` fills the HIGHCONTRASTW that `pvParam` points to, whose
    // `cbSize` is set and passed as `uiParam`. `info` is a live local; the scheme-name pointer it
    // may receive is never dereferenced.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            (&raw mut info).cast::<c_void>(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let on = info.dwFlags & HCF_HIGHCONTRASTON != 0;
    Some(HighContrast {
        on,
        colors: on.then(system_colors),
    })
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The current user's "Transparency effects" switch.
fn transparency_enabled() -> Option<bool> {
    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("EnableTransparency");
    let mut data: u32 = 0;
    let mut size = u32::try_from(size_of::<u32>()).ok()?;
    // SAFETY: both names are NUL-terminated UTF-16 buffers that outlive the call; the type filter
    // restricts the result to a DWORD, and `data`/`size` describe a live 4-byte output buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&raw mut data).cast::<c_void>(),
            &raw mut size,
        )
    };
    (status == ERROR_SUCCESS).then_some(data != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorref_is_blue_green_red() {
        assert_eq!(colorref(0x00_33_22_11), [0x11, 0x22, 0x33]);
    }

    #[test]
    fn reading_never_panics_and_animation_is_known_on_a_desktop() {
        let prefs = WindowsPrefs.read();
        // Windows always answers the animation query; other values depend on the account.
        assert!(prefs.animations.is_some());
        if let Some(hc) = prefs.high_contrast {
            assert_eq!(hc.colors.is_some(), hc.on);
        }
    }
}
