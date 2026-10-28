//! A dark title bar when Windows is set to dark mode.
//!
//! Classic Win32 controls have no supported dark mode, so the inside of the
//! window stays light. The title bar does follow the setting, through a DWM
//! attribute that Windows 10 (version 2004 and later) and Windows 11 support.
//! On older Windows 10 builds the call fails and the title bar stays light.

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::core::{BOOL, PCWSTR, w};

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
    let dark = BOOL::from(apps_use_dark_mode());
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
