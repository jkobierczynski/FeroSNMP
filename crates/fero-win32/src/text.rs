//! Moving text between Rust strings and the UTF-16 that Win32 uses.
//!
//! To pass a string *to* Win32, use [`windows::core::HSTRING`]:
//! `&HSTRING::from("text")` can be given wherever a `PCWSTR` is expected. This
//! module covers the other direction and a few conveniences.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};

/// A null-terminated UTF-16 copy of `text`, for structures with a `PWSTR`
/// field (such as `TVITEMW::pszText`). Keep the vector alive while Win32
/// reads from it.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// The text of a window or control, such as what is typed in an edit box.
pub fn window_text(hwnd: HWND) -> String {
    // SAFETY: the buffer is as long as the length we pass, plus room for the
    // terminating null.
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}

/// Convert `\n` line endings to the `\r\n` that edit controls need.
pub fn crlf(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 32);
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push_str("\r\n");
        }
        out.push_str(line.strip_suffix('\r').unwrap_or(line));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_is_null_terminated() {
        assert_eq!(wide("ab"), [97, 98, 0]);
    }

    #[test]
    fn crlf_does_not_double_carriage_returns() {
        assert_eq!(crlf("a\nb\r\nc"), "a\r\nb\r\nc");
    }
}
