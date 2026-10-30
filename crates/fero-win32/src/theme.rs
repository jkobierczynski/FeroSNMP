//! Dark mode: a dark title bar, and dark tree views and list views.
//!
//! The title bar follows the setting through a DWM attribute that Windows 10
//! (version 2004 and later) and Windows 11 support. On older Windows 10
//! builds the call fails and the title bar stays light.
//!
//! Classic Win32 controls have no documented dark mode. Tree views and list
//! views come close with the `DarkMode_Explorer` visual style that Explorer
//! itself uses (dark selection, expand buttons and scroll bars, Windows 10
//! version 1809 and later) and dark background and text colours; a list's
//! column header uses `DarkMode_ItemsView`. Where those styles do not exist,
//! [`set_tree_dark`] and [`set_list_dark`] still set the colours. Other
//! controls (buttons, edit boxes, tabs, the status bar) stay light.

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, COLOR_WINDOWTEXT, GetSysColor, InvalidateRect};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Controls::{
    LVM_GETHEADER, LVM_SETBKCOLOR, LVM_SETTEXTBKCOLOR, LVM_SETTEXTCOLOR, SetWindowTheme,
    TVM_SETBKCOLOR, TVM_SETTEXTCOLOR,
};
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
use windows::core::{BOOL, PCWSTR, w};

/// The background of dark trees and lists, as Explorer's.
pub const DARK_BACKGROUND: COLORREF = COLORREF(0x0019_1919);
/// The text in dark trees and lists.
pub const DARK_TEXT: COLORREF = COLORREF(0x00F0_F0F0);

/// True if "Choose your app mode" in Windows settings is set to Dark.
pub fn apps_use_dark_mode() -> bool {
    let mut value: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the value is a DWORD and the buffer is a u32 of that size.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    result.is_ok() && value == 0
}

/// Make the title bar of a top-level window match the app mode setting. Call
/// it once after creating the window and again on [`is_theme_change`].
pub fn apply_title_bar(hwnd: HWND) {
    set_title_bar_dark(hwnd, apps_use_dark_mode());
}

/// Make the title bar of a top-level window dark or light.
pub fn set_title_bar_dark(hwnd: HWND, dark: bool) {
    let dark = BOOL::from(dark);
    // SAFETY: the attribute takes a BOOL and we pass one.
    unsafe {
        let set = |attribute: i32| {
            DwmSetWindowAttribute(
                hwnd,
                windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(attribute),
                &dark as *const BOOL as *const _,
                size_of::<BOOL>() as u32,
            )
        };
        if set(DWMWA_USE_IMMERSIVE_DARK_MODE.0).is_err() {
            // Windows 10 before version 2004 used attribute 19 for the same.
            let _ = set(19);
        }
    }
}

/// True if a `WM_SETTINGCHANGE` message says the light/dark setting changed.
pub fn is_theme_change(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    // SAFETY: for WM_SETTINGCHANGE a non-zero lparam is a null-terminated
    // string naming the changed setting.
    let name = unsafe { PCWSTR(lparam.0 as *const u16).to_string() };
    name.is_ok_and(|name| name == "ImmersiveColorSet")
}

fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: only used below with messages that take plain numbers.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))).0 }
}

/// Make a tree view dark or light again.
pub fn set_tree_dark(tree: HWND, dark: bool) {
    let (theme, background, text) = if dark {
        (
            w!("DarkMode_Explorer"),
            DARK_BACKGROUND.0 as isize,
            DARK_TEXT.0 as isize,
        )
    } else {
        // -1: the system colours.
        (w!("Explorer"), -1, -1)
    };
    // SAFETY: plain control messages with colours; a missing visual style
    // only makes SetWindowTheme fail.
    unsafe {
        let _ = SetWindowTheme(tree, theme, None);
    }
    send(tree, TVM_SETBKCOLOR, 0, background);
    send(tree, TVM_SETTEXTCOLOR, 0, text);
    // SAFETY: repainting a window.
    unsafe {
        let _ = InvalidateRect(Some(tree), None, true);
    }
}

/// Make a list view dark or light again, with its column header.
pub fn set_list_dark(list: HWND, dark: bool) {
    let (theme, header_theme, background, text_background, text) = if dark {
        (
            w!("DarkMode_Explorer"),
            w!("DarkMode_ItemsView"),
            DARK_BACKGROUND.0,
            DARK_BACKGROUND.0,
            DARK_TEXT.0,
        )
    } else {
        // SAFETY: GetSysColor has no preconditions.
        let (window, text) = unsafe { (GetSysColor(COLOR_WINDOW), GetSysColor(COLOR_WINDOWTEXT)) };
        (w!("Explorer"), w!("Explorer"), window, window, text)
    };
    let header = HWND(send(list, LVM_GETHEADER, 0, 0) as *mut _);
    // SAFETY: plain control messages with colours; a missing visual style
    // only makes SetWindowTheme fail.
    unsafe {
        let _ = SetWindowTheme(list, theme, None);
        if !header.is_invalid() {
            let _ = SetWindowTheme(header, header_theme, None);
        }
    }
    send(list, LVM_SETBKCOLOR, 0, background as isize);
    send(list, LVM_SETTEXTBKCOLOR, 0, text_background as isize);
    send(list, LVM_SETTEXTCOLOR, 0, text as isize);
    // SAFETY: repainting a window.
    unsafe {
        let _ = InvalidateRect(Some(list), None, true);
    }
}
