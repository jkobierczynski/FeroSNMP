//! The Win32 user interface. Windows only.
//!
//! - [`main_window`]: the main window, which owns everything below.
//! - [`toolbar`]: the rebar at the top: buttons, the search box, and the
//!   Agent and Object boxes with the query buttons.
//! - [`tree`]: the MIB tree on the left, filled in as nodes are expanded.
//! - [`details`]: the properties list and description on the right.
//! - [`icons`]: the small pictures in the tree, one per kind of node.
//! - [`settings_dialog`]: the Settings dialog.
//! - [`agents_dialog`]: the Agents dialog, for saved agents.
//! - [`bottom_panel`]: the Results, Log and Diagnostics tabs along the bottom.
//! - [`text_window`]: the source viewer and the "Why this symbol?" report.
//!
//! All windows belong to the one UI thread that runs [`run`]. MIBs are loaded
//! on another thread (see [`crate::loading`]), which posts the result back,
//! and SNMP queries run on the engine's thread (see [`crate::query`]), which
//! posts a message whenever results are waiting.

mod agents_dialog;
mod bottom_panel;
mod credentials;
mod details;
mod icons;
mod main_window;
mod settings_dialog;
mod text_window;
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

/// Posted by the SNMP engine's thread when results of a query are waiting.
/// The `wparam` is the query's number; the window ignores older queries.
pub const WM_APP_QUERY: u32 = WM_APP + 2;

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
    /// The tab control along the bottom.
    pub const TABS: u16 = 107;
    /// The Diagnostics list.
    pub const DIAGNOSTICS: u16 = 108;
    /// The Results list.
    pub const RESULTS: u16 = 109;
    /// The Log list.
    pub const LOG: u16 = 110;
    /// The Agent box in the rebar.
    pub const AGENT: u16 = 111;
    /// The Object box in the rebar.
    pub const OBJECT: u16 = 112;
    /// The toolbar with the query buttons.
    pub const QUERY_TOOLBAR: u16 = 113;

    /// Enter, from the dialog keyboard handling (the predefined `IDOK`).
    pub const ENTER: u16 = 1;
    /// Escape, from the dialog keyboard handling (the predefined `IDCANCEL`).
    pub const ESCAPE: u16 = 2;
    /// File > Reload MIBs (Ctrl+R).
    pub const RELOAD: u16 = 200;
    /// File > Settings.
    pub const SETTINGS: u16 = 201;
    /// File > Export diagnostics.
    pub const EXPORT: u16 = 203;
    /// File > Exit.
    pub const EXIT: u16 = 202;
    /// File > Agents (also a toolbar button).
    pub const AGENTS: u16 = 204;
    /// File > Export results (Ctrl+S).
    pub const EXPORT_RESULTS: u16 = 205;
    /// Edit > Find (Ctrl+F): go to the search box.
    pub const FIND: u16 = 210;
    /// Edit > Find next (F3).
    pub const FIND_NEXT: u16 = 211;
    /// Edit > Find previous (Shift+F3).
    pub const FIND_PREVIOUS: u16 = 212;
    /// Edit > Why this symbol? (Ctrl+W), also the button in the source viewer.
    pub const WHY: u16 = 213;
    /// View > Source of the selected diagnostic (also Enter or a double click).
    pub const SHOW_SOURCE: u16 = 214;
    /// Edit > Copy (Ctrl+C).
    pub const COPY: u16 = 215;
    /// Query > Get.
    pub const GET: u16 = 230;
    /// Query > Get next.
    pub const NEXT: u16 = 231;
    /// Query > Get bulk.
    pub const BULK: u16 = 232;
    /// Query > Walk.
    pub const WALK: u16 = 233;
    /// Query > Repeat (F5).
    pub const REPEAT: u16 = 234;
    /// Query > Cancel (Esc, also a toolbar button).
    pub const CANCEL: u16 = 235;
    /// View > Raw values (Ctrl+Shift+R, also a toolbar button).
    pub const RAW: u16 = 240;
    /// Context menu of the Results list: copy the cell under the mouse.
    pub const COPY_CELL: u16 = 250;
    /// Context menu of the Results list: copy the numeric OIDs.
    pub const COPY_OID: u16 = 251;
    /// Context menu of the Results list: show the object in the MIB tree.
    pub const SHOW_IN_TREE: u16 = 252;
    /// Context menu of the Results list: forget the results.
    pub const CLEAR_RESULTS: u16 = 253;
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
        accelerator(FVIRTKEY, VK_F5.0, id::REPEAT),
        accelerator(FCONTROL, u16::from(b'R'), id::RELOAD),
        accelerator(FCONTROL | FSHIFT, u16::from(b'R'), id::RAW),
        accelerator(FCONTROL, u16::from(b'C'), id::COPY),
        accelerator(FCONTROL, u16::from(b'S'), id::EXPORT_RESULTS),
        accelerator(FCONTROL, u16::from(b'W'), id::WHY),
        accelerator(FCONTROL, u16::from(b'E'), id::EXPORT),
    ];
    // SAFETY: the standard Win32 message loop on the UI thread.
    unsafe {
        let table = CreateAcceleratorTableW(&accelerators).ok();
        let mut msg = MSG::default();
        // GetMessageW returns 0 for WM_QUIT and -1 on error; stop on both.
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            // Keyboard shortcuts first (Ctrl+F, F3, F5, Ctrl+C) ...
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
