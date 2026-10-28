//! The Settings dialog: MIB folders and strictness.

use std::cell::RefCell;
use std::path::Path;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CB_ADDSTRING,
    CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST, EndDialog, GetDlgItem,
    LB_ADDSTRING, LB_ERR, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, LBN_SELCHANGE,
    LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, SendMessageW, SetWindowTextW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_COMMAND, WM_INITDIALOG, WS_BORDER, WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VSCROLL,
};
use windows::core::HSTRING;

use fero_win32::dialog::{ControlClass, Dialog, DialogTemplate};
use fero_win32::{hiword, loword};

use crate::settings::{Settings, StrictnessSetting};

use super::{error_box, send};

const FOLDERS: u16 = 1001;
const ADD: u16 = 1002;
const REMOVE: u16 = 1003;
const UP: u16 = 1004;
const DOWN: u16 = 1005;
const SYSTEM_FOLDERS: u16 = 1006;
const STRICTNESS: u16 = 1007;
const EXPLANATION: u16 = 1008;
const FILE: u16 = 1009;
const OK: u16 = 1;
const CANCEL: u16 = 2;

// Static control styles (not in the parts of the windows crate we use).
const SS_NOPREFIX: u32 = 0x0080;
const SS_PATHELLIPSIS: u32 = 0x8000;

/// Show the Settings dialog. Returns the new settings if the user pressed OK.
pub fn show(owner: HWND, settings: &Settings, file: Option<&Path>) -> Option<Settings> {
    let none = WINDOW_EX_STYLE(0);
    let style = |bits: i32| WINDOW_STYLE(bits as u32);
    let button = style(BS_PUSHBUTTON) | WS_TABSTOP;
    let template = DialogTemplate::new("Settings", 330, 196, "Segoe UI", 9)
        .control(
            ControlClass::Static,
            "MIB &folders, searched from top to bottom:",
            0,
            WINDOW_STYLE(0),
            none,
            (7, 7, 256, 9),
        )
        .control(
            ControlClass::ListBox,
            "",
            FOLDERS,
            style(LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) | WS_VSCROLL | WS_TABSTOP | WS_BORDER,
            WS_EX_CLIENTEDGE,
            (7, 18, 256, 92),
        )
        .control(
            ControlClass::Button,
            "&Add...",
            ADD,
            button,
            none,
            (269, 18, 54, 14),
        )
        .control(
            ControlClass::Button,
            "&Remove",
            REMOVE,
            button,
            none,
            (269, 36, 54, 14),
        )
        .control(
            ControlClass::Button,
            "Move &up",
            UP,
            button,
            none,
            (269, 54, 54, 14),
        )
        .control(
            ControlClass::Button,
            "Move &down",
            DOWN,
            button,
            none,
            (269, 72, 54, 14),
        )
        .control(
            ControlClass::Button,
            "Also use the &net-snmp and libsmi MIB folders on this computer",
            SYSTEM_FOLDERS,
            style(BS_AUTOCHECKBOX) | WS_TABSTOP,
            none,
            (7, 116, 316, 10),
        )
        .control(
            ControlClass::Static,
            "S&trictness:",
            0,
            WINDOW_STYLE(0),
            none,
            (7, 136, 50, 9),
        )
        .control(
            ControlClass::ComboBox,
            "",
            STRICTNESS,
            style(CBS_DROPDOWNLIST) | WS_VSCROLL | WS_TABSTOP,
            none,
            (60, 134, 100, 80),
        )
        .control(
            ControlClass::Static,
            "",
            EXPLANATION,
            WINDOW_STYLE(SS_NOPREFIX),
            none,
            (7, 152, 316, 18),
        )
        .control(
            ControlClass::Static,
            "",
            FILE,
            WINDOW_STYLE(SS_NOPREFIX | SS_PATHELLIPSIS),
            none,
            (7, 178, 200, 9),
        )
        .control(
            ControlClass::Button,
            "OK",
            OK,
            style(BS_DEFPUSHBUTTON) | WS_TABSTOP,
            none,
            (213, 175, 52, 14),
        )
        .control(
            ControlClass::Button,
            "Cancel",
            CANCEL,
            button,
            none,
            (269, 175, 54, 14),
        );

    let dialog = SettingsDialog {
        settings: RefCell::new(settings.clone()),
        file: match file {
            Some(file) => format!("Saved in {}", file.display()),
            None => "Not saved: APPDATA is not set".to_string(),
        },
    };
    match template.run_modal(Some(owner), &dialog) {
        Ok(1) => Some(dialog.settings.into_inner()),
        Ok(_) => None,
        Err(error) => {
            error_box(
                Some(owner),
                &format!("The Settings dialog could not be opened.\n\n{error}"),
            );
            None
        }
    }
}

struct SettingsDialog {
    /// The settings as edited so far.
    settings: RefCell<Settings>,
    /// Where the settings file is, for the line at the bottom.
    file: String,
}

impl Dialog for SettingsDialog {
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<isize> {
        match msg {
            WM_INITDIALOG => {
                self.init(hwnd);
                Some(1)
            }
            WM_COMMAND => {
                self.command(hwnd, loword(wparam.0), u32::from(hiword(wparam.0)));
                Some(1)
            }
            _ => None,
        }
    }
}

impl SettingsDialog {
    fn init(&self, hwnd: HWND) {
        let settings = self.settings.borrow().clone();
        let check = if settings.use_system_folders {
            BST_CHECKED
        } else {
            BST_UNCHECKED
        };
        // SAFETY: plain control messages; the strings live until each call
        // returns, and the controls copy them.
        unsafe {
            send(item(hwnd, SYSTEM_FOLDERS), BM_SETCHECK, check.0 as usize, 0);
            let combo = item(hwnd, STRICTNESS);
            for strictness in StrictnessSetting::ALL {
                let label = HSTRING::from(strictness.label());
                SendMessageW(
                    combo,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(label.as_ptr() as isize)),
                );
            }
            let selected = StrictnessSetting::ALL
                .iter()
                .position(|&s| s == settings.strictness);
            send(combo, CB_SETCURSEL, selected.unwrap_or(0), 0);
            let _ = SetWindowTextW(item(hwnd, FILE), &HSTRING::from(self.file.as_str()));
        }
        self.fill_folders(hwnd, None);
        self.show_explanation(hwnd);
    }

    fn command(&self, hwnd: HWND, id: u16, code: u32) {
        match (id, code) {
            (ADD, _) => self.add_folders(hwnd),
            (REMOVE, _) => {
                if let Some(index) = self.selected(hwnd) {
                    let mut settings = self.settings.borrow_mut();
                    settings.mib_folders.remove(index);
                    let next = index.min(settings.mib_folders.len().saturating_sub(1));
                    drop(settings);
                    self.fill_folders(hwnd, Some(next));
                }
            }
            (UP, _) => self.move_selected(hwnd, -1),
            (DOWN, _) => self.move_selected(hwnd, 1),
            (FOLDERS, LBN_SELCHANGE) => self.update_buttons(hwnd),
            (STRICTNESS, CBN_SELCHANGE) => self.show_explanation(hwnd),
            (OK, _) => {
                let mut settings = self.settings.borrow_mut();
                // SAFETY: plain control messages.
                unsafe {
                    settings.use_system_folders =
                        send(item(hwnd, SYSTEM_FOLDERS), BM_GETCHECK, 0, 0).0
                            == BST_CHECKED.0 as isize;
                }
                settings.strictness = self.strictness(hwnd);
                drop(settings);
                // SAFETY: ends the modal loop of our own dialog.
                unsafe {
                    let _ = EndDialog(hwnd, 1);
                }
            }
            (CANCEL, _) => {
                // SAFETY: ends the modal loop of our own dialog.
                unsafe {
                    let _ = EndDialog(hwnd, 0);
                }
            }
            _ => {}
        }
    }

    fn add_folders(&self, hwnd: HWND) {
        let picked = match fero_win32::shell::pick_folders(Some(hwnd), "Add MIB folders") {
            Ok(picked) => picked,
            Err(error) => {
                error_box(
                    Some(hwnd),
                    &format!("The folder picker could not be opened.\n\n{error}"),
                );
                return;
            }
        };
        if picked.is_empty() {
            return;
        }
        let mut settings = self.settings.borrow_mut();
        for folder in picked {
            if !settings.mib_folders.contains(&folder) {
                settings.mib_folders.push(folder);
            }
        }
        let last = settings.mib_folders.len() - 1;
        drop(settings);
        self.fill_folders(hwnd, Some(last));
    }

    fn move_selected(&self, hwnd: HWND, offset: isize) {
        let Some(index) = self.selected(hwnd) else {
            return;
        };
        let mut settings = self.settings.borrow_mut();
        let Some(target) = index
            .checked_add_signed(offset)
            .filter(|&t| t < settings.mib_folders.len())
        else {
            return;
        };
        settings.mib_folders.swap(index, target);
        drop(settings);
        self.fill_folders(hwnd, Some(target));
    }

    /// Show the folder list again, selecting `select`.
    fn fill_folders(&self, hwnd: HWND, select: Option<usize>) {
        let list = item(hwnd, FOLDERS);
        let folders = self.settings.borrow().mib_folders.clone();
        // SAFETY: plain list box messages; each string lives until its call
        // returns, and the list box copies it.
        unsafe {
            send(list, LB_RESETCONTENT, 0, 0);
            for folder in &folders {
                let text = HSTRING::from(folder.display().to_string());
                SendMessageW(
                    list,
                    LB_ADDSTRING,
                    None,
                    Some(LPARAM(text.as_ptr() as isize)),
                );
            }
            if let Some(index) = select.filter(|&i| i < folders.len()) {
                send(list, LB_SETCURSEL, index, 0);
            }
        }
        self.update_buttons(hwnd);
    }

    /// Enable Remove, Move up and Move down only when they can do something.
    fn update_buttons(&self, hwnd: HWND) {
        let count = self.settings.borrow().mib_folders.len();
        let selected = self.selected(hwnd);
        let enable = |id: u16, on: bool| {
            // SAFETY: enabling one of our own controls.
            unsafe {
                let _ = EnableWindow(item(hwnd, id), on);
            }
        };
        enable(REMOVE, selected.is_some());
        enable(UP, selected.is_some_and(|i| i > 0));
        enable(DOWN, selected.is_some_and(|i| i + 1 < count));
    }

    fn selected(&self, hwnd: HWND) -> Option<usize> {
        // SAFETY: plain list box message.
        let index = unsafe { send(item(hwnd, FOLDERS), LB_GETCURSEL, 0, 0).0 };
        (index != LB_ERR as isize).then_some(index as usize)
    }

    fn strictness(&self, hwnd: HWND) -> StrictnessSetting {
        // SAFETY: plain combo box message.
        let index = unsafe { send(item(hwnd, STRICTNESS), CB_GETCURSEL, 0, 0).0 };
        usize::try_from(index)
            .ok()
            .and_then(|i| StrictnessSetting::ALL.get(i).copied())
            .unwrap_or_default()
    }

    fn show_explanation(&self, hwnd: HWND) {
        let text = HSTRING::from(self.strictness(hwnd).explanation());
        // SAFETY: setting the text of one of our own controls.
        unsafe {
            let _ = SetWindowTextW(item(hwnd, EXPLANATION), &text);
        }
    }
}

/// A control of the dialog by id.
fn item(dialog: HWND, id: u16) -> HWND {
    // SAFETY: looks up a child window; a missing one gives a null handle,
    // which later calls ignore.
    unsafe { GetDlgItem(Some(dialog), i32::from(id)).unwrap_or_default() }
}
