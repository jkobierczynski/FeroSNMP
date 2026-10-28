//! The font Windows uses for message boxes and dialogs, at any DPI.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, GetDC, GetTextMetricsW, HFONT, HGDIOBJ, ReleaseDC,
    SelectObject, TEXTMETRICW,
};
use windows::Win32::UI::HiDpi::SystemParametersInfoForDpi;
use windows::Win32::UI::WindowsAndMessaging::{
    NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SendMessageW, WM_SETFONT,
};

/// An owned GDI font, deleted when dropped.
///
/// Controls do not copy the font they are given with `WM_SETFONT`; keep the
/// `Font` alive for as long as any control uses it.
#[derive(Debug)]
pub struct Font {
    handle: HFONT,
}

impl Font {
    /// The message font at `dpi`: Segoe UI 9 pt on Windows 10 and 11, unless
    /// the user changed it.
    pub fn message(dpi: u32) -> windows::core::Result<Font> {
        let mut metrics = NONCLIENTMETRICSW {
            cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
            ..Default::default()
        };
        // SAFETY: `metrics` is a properly sized NONCLIENTMETRICSW that the call
        // fills in; the LOGFONTW inside it is then only read.
        unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS.0,
                metrics.cbSize,
                Some(&mut metrics as *mut _ as *mut _),
                0,
                dpi,
            )?;
            let handle = CreateFontIndirectW(&metrics.lfMessageFont);
            if handle.is_invalid() {
                return Err(windows::core::Error::from_thread());
            }
            Ok(Font { handle })
        }
    }

    /// The raw handle, for Win32 calls.
    pub fn handle(&self) -> HFONT {
        self.handle
    }

    /// Give this font to a window or control.
    pub fn apply(&self, hwnd: HWND) {
        // SAFETY: WM_SETFONT takes a font handle and a redraw flag.
        unsafe {
            SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(self.handle.0 as usize)),
                Some(LPARAM(1)),
            );
        }
    }

    /// Height of a line of text in this font, in pixels.
    pub fn line_height(&self, hwnd: HWND) -> i32 {
        // SAFETY: the device context is released before returning and the
        // previously selected font is put back.
        unsafe {
            let dc = GetDC(Some(hwnd));
            let old = SelectObject(dc, HGDIOBJ(self.handle.0));
            let mut metrics = TEXTMETRICW::default();
            let _ = GetTextMetricsW(dc, &mut metrics);
            SelectObject(dc, old);
            ReleaseDC(Some(hwnd), dc);
            metrics.tmHeight
        }
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: we own the handle and nobody uses it after this.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.handle.0));
        }
    }
}
