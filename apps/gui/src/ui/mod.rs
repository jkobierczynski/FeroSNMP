//! The Win32 user interface. Windows only.
//!
//! - [`main_window`]: the main window, which owns everything below.
//! - [`toolbar`]: the rebar at the top with buttons and the search box.
//! - [`tree`]: the MIB tree on the left, filled in as nodes are expanded.
//! - [`details`]: the properties list and description on the right.
//! - [`icons`]: the small pictures in the tree, one per kind of node.
//! - [`settings_dialog`]: the Settings dialog.
//!
//! All windows belong to the one UI thread that runs [`run`]. MIBs are loaded
//! on another thread (see [`crate::loading`]), which posts the result back.

mod details;
mod icons;
mod main_window;
mod settings_dialog;
mod toolbar;
mod tree;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
};
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_COOL_CLASSES, ICC_LISTVIEW_CLASSES, ICC_STANDARD_CLASSES,
    ICC_TREEVIEW_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_F3, VK_F5};
use windows::Win32::UI::WindowsAndMessaging::{
    ACCEL, CreateAcceleratorTableW, DispatchMessageW, FCONTROL, FSHIFT, FVIRTKEY, GetMessageW,
    IsDialogMessageW, MB_ICONERROR, MB_OK, MSG, MessageBoxW, SendMessageW, TranslateAcceleratorW,
    TranslateMessage, WM_APP,
};
use windows::core::HSTRING;

use self::main_window::MainWindow;

/// Posted by the loading thread when a load has finished. The `lparam` is a
/// `Box<Loaded>` turned into a raw pointer; the window takes it back.
pub const WM_APP_LOADED: u32 = WM_APP + 1;

/// Control and command ids.
pub mod id {
    /// The MIB tree.
    pub const TREE: u16 = 100;
    /// The properties list.
    pub const PROPERTIES: u16 = 101;
    /// The description text.
    pub const DESCRIPTION: u16 = 102;
    /// The status bar.
    pub const STATUS: u16 = 103;
    /// The rebar at the top.
    pub const REBAR: u16 = 104;
    /// The toolbar in the rebar.
    pub const TOOLBAR: u16 = 105;
    /// The search box in the rebar.
    pub const SEARCH: u16 = 106;

    /// Enter, from the dialog keyboard handling (the predefined `IDOK`).
    pub const ENTER: u16 = 1;
    /// Escape, from the dialog keyboard handling (the predefined `IDCANCEL`).
    pub const ESCAPE: u16 = 2;
    /// File > Reload MIBs (F5).
    pub const RELOAD: u16 = 200;
    /// File > Settings.
    pub const SETTINGS: u16 = 201;
    /// File > Exit.
    pub const EXIT: u16 = 202;
    /// Edit > Find (Ctrl+F): go to the search box.
    pub const FIND: u16 = 210;
    /// Edit > Find next (F3).
    pub const FIND_NEXT: u16 = 211;
    /// Edit > Find previous (Shift+F3).
    pub const FIND_PREVIOUS: u16 = 212;
    /// Help > About.
    pub const ABOUT: u16 = 220;
}

/// Start the GUI and run until the main window closes.
pub fn run() {
    install_panic_hook();
    // SAFETY: called once, first thing, on the thread that runs the UI.
    unsafe {
        // COM is needed for the folder picker. Failure only affects that.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).ok();
        let classes = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES
                | ICC_BAR_CLASSES
                | ICC_COOL_CLASSES
                | ICC_LISTVIEW_CLASSES
                | ICC_TREEVIEW_CLASSES,
        };
        let _ = InitCommonControlsEx(&classes);
    }

    let window = match MainWindow::create() {
        Ok(window) => window,
        Err(error) => {
            error_box(
                None,
                &format!("FeroSNMP could not open its window.\n\n{error}"),
            );
            std::process::exit(1);
        }
    };
    window.start();
    message_loop(window.hwnd());
}

/// The message loop: fetch each message for this thread and dispatch it.
fn message_loop(main: HWND) {
    let accelerators = [
        accelerator(FCONTROL, u16::from(b'F'), id::FIND),
        accelerator(FVIRTKEY, VK_F3.0, id::FIND_NEXT),
        accelerator(FSHIFT, VK_F3.0, id::FIND_PREVIOUS),
        accelerator(FVIRTKEY, VK_F5.0, id::RELOAD),
    ];
    // SAFETY: the standard Win32 message loop on the UI thread.
    unsafe {
        let table = CreateAcceleratorTableW(&accelerators).ok();
        let mut msg = MSG::default();
        // GetMessageW returns 0 for WM_QUIT and -1 on error; stop on both.
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            // Keyboard shortcuts first (Ctrl+F, F3, F5) ...
            if let Some(table) = table
                && TranslateAcceleratorW(main, table, &msg) != 0
            {
                continue;
            }
            // ... then dialog-style keys: Tab moves between the panes, Enter
            // and Escape arrive as id::ENTER and id::ESCAPE commands.
            if IsDialogMessageW(main, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn accelerator(
    modifiers: windows::Win32::UI::WindowsAndMessaging::ACCEL_VIRT_FLAGS,
    key: u16,
    command: u16,
) -> ACCEL {
    ACCEL {
        fVirt: modifiers | FVIRTKEY,
        key,
        cmd: command,
    }
}

/// Show a panic as a message box before the program stops. A release build
/// has no console, so without this a bug would close the window silently.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        error_box(
            None,
            &format!(
                "FeroSNMP ran into a bug and has to close.\n\n{info}\n\n\
                 Please report this at https://github.com/jkobierczynski/ferosnmp/issues"
            ),
        );
    }));
}

/// Show an error message.
pub fn error_box(owner: Option<HWND>, text: &str) {
    // SAFETY: plain message box; the strings outlive the call.
    unsafe {
        MessageBoxW(
            owner,
            &HSTRING::from(text),
            &HSTRING::from("FeroSNMP"),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// `SendMessageW` with the parameters as plain integers, which is how most
/// control messages are written in the Win32 documentation.
///
/// # Safety
///
/// Whatever the message expects to find in `wparam` and `lparam` must be
/// there: when it expects a pointer, it must point to a live value of the
/// right type for the duration of the call.
pub unsafe fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize) -> LRESULT {
    // SAFETY: passed on to the caller.
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }
}
