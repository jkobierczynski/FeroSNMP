//! Per-monitor DPI scaling.
//!
//! FeroSNMP declares itself "per-monitor DPI aware (v2)" in its application
//! manifest. Windows then stops stretching the window as a bitmap, and the
//! program draws at the real resolution of whichever monitor the window is on.
//! The price is that every size in pixels must be scaled: designs use sizes at
//! 96 DPI (100 %), and [`scale`] converts them for the window's monitor. When
//! the window moves to another monitor, Windows sends `WM_DPICHANGED`.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};

/// The DPI that sizes in the design are written for (100 % scaling).
pub const BASE_DPI: u32 = 96;

/// The DPI of the monitor the window is on.
pub fn for_window(hwnd: HWND) -> u32 {
    // SAFETY: any window handle may be passed; an invalid one gives 0.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 { BASE_DPI } else { dpi }
}

/// The DPI of the primary monitor, for sizes needed before a window exists.
pub fn for_system() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetDpiForSystem() }
}

/// Scale a size at 96 DPI to `dpi`, rounding to the nearest pixel.
pub fn scale(value: i32, dpi: u32) -> i32 {
    let dpi = i64::from(dpi);
    let base = i64::from(BASE_DPI);
    let scaled = (i64::from(value) * dpi + base / 2).div_euclid(base);
    scaled as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_and_rounds() {
        assert_eq!(scale(16, 96), 16);
        assert_eq!(scale(16, 144), 24);
        assert_eq!(scale(5, 120), 6); // 6.25
        assert_eq!(scale(-4, 144), -6);
    }
}
