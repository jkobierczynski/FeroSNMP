//! Small helpers for plain Win32 programs written with the [windows] crate.
//!
//! FeroSNMP talks to Win32 directly instead of through a GUI framework. Most
//! of that code is ordinary API calls, but a few pieces are needed by every
//! window and are easy to get subtly wrong; they live here, with nothing
//! specific to SNMP, so other Fero tools (such as FeroSSH) can share them:
//!
//! - `window`: register a window class and route its messages to a Rust
//!   value, without undefined behaviour when Win32 calls back into the window
//!   while it is already handling a message.
//! - `dpi` and `font`: per-monitor DPI scaling and the system UI font
//!   (Segoe UI on Windows 10 and 11) at the right size for each monitor.
//! - [`splitter`]: the arithmetic of a draggable bar between two panes.
//! - `theme`: a dark title bar when Windows is set to dark mode.
//! - `dialog`: modal dialogs built in memory, so no resource compiler is
//!   needed.
//! - `shell`: the standard "pick a folder" dialog.
//! - `clipboard`: putting text on the clipboard.
//!
//! Everything except [`splitter`] only exists on Windows. The splitter is
//! plain arithmetic, so it builds and is tested everywhere.
//!
//! [windows]: https://docs.rs/windows

pub mod splitter;

#[cfg(windows)]
pub mod clipboard;
#[cfg(windows)]
pub mod dialog;
#[cfg(windows)]
pub mod dpi;
#[cfg(windows)]
pub mod font;
#[cfg(windows)]
pub mod shell;
#[cfg(windows)]
pub mod text;
#[cfg(windows)]
pub mod theme;
#[cfg(windows)]
pub mod window;

/// Low 16 bits of a message parameter, such as the control id in the
/// `wparam` of `WM_COMMAND`.
pub fn loword(value: usize) -> u16 {
    (value & 0xFFFF) as u16
}

/// High 16 bits (of the low 32) of a message parameter, such as the
/// notification code in the `wparam` of `WM_COMMAND`.
pub fn hiword(value: usize) -> u16 {
    ((value >> 16) & 0xFFFF) as u16
}

/// The signed x and y coordinates packed into the `lparam` of mouse messages.
pub fn point_from_lparam(lparam: isize) -> (i32, i32) {
    // The coordinates are signed 16-bit values: on a multi-monitor desktop a
    // point left of or above the primary monitor is negative.
    let x = (lparam & 0xFFFF) as u16 as i16;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16;
    (i32::from(x), i32::from(y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_split() {
        assert_eq!(loword(0x0002_0001), 1);
        assert_eq!(hiword(0x0002_0001), 2);
    }

    #[test]
    fn negative_points_survive() {
        // x = -5 (0xFFFB), y = 10.
        assert_eq!(point_from_lparam(0x000A_FFFB), (-5, 10));
    }
}
