//! The Trap user dialog: one SNMPv3 user that may send traps and INFORMs,
//! opened from the Traps page of the Settings dialog.

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE,
    CBS_DROPDOWNLIST, ES_AUTOHSCROLL, ES_PASSWORD, EndDialog, GetDlgItem, SendMessageW,
    SetWindowTextW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_INITDIALOG, WS_BORDER,
    WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VSCROLL,
};
use windows::core::HSTRING;

use fero_win32::dialog::{ControlClass, Dialog, DialogTemplate};
use fero_win32::text::window_text;
use fero_win32::{hiword, loword};

use snmp_core::{AuthProtocol, PrivProtocol, SecurityLevel};

use crate::trap_users::SavedTrapUser;

use super::agents_dialog::priv_label;
use super::{error_box, send};

const NAME: u16 = 1001;
const ENGINE: u16 = 1002;
const LEVEL: u16 = 1003;
const AUTH_PROTOCOL: u16 = 1004;
const AUTH_PASSWORD: u16 = 1005;
const PRIV_PROTOCOL: u16 = 1006;
const PRIV_PASSWORD: u16 = 1007;
const OK: u16 = 1;
const CANCEL: u16 = 2;

/// The fields, in the order they are shown.
const FIELDS: [(u16, &str); 7] = [
    (NAME, "&User name:"),
    (ENGINE, "Sender's &engine ID:"),
    (LEVEL, "&Security level:"),
    (AUTH_PROTOCOL, "A&uth protocol:"),
    (AUTH_PASSWORD, "Auth pass&word:"),
    (PRIV_PROTOCOL, "Pri&vacy protocol:"),
    (PRIV_PASSWORD, "Privacy pa&ssword:"),
];

// Static control style (not in the parts of the windows crate we use).
const SS_NOPREFIX: u32 = 0x0080;

/// Show the dialog for `user` (a new one when its name is empty). `others`
/// are the other saved users, which it may not repeat. Returns the user if
/// OK was pressed.
pub fn show(owner: HWND, user: &SavedTrapUser, others: &[SavedTrapUser]) -> Option<SavedTrapUser> {
    let none = WINDOW_EX_STYLE(0);
    let style = |bits: i32| WINDOW_STYLE(bits as u32);
    let edit = style(ES_AUTOHSCROLL) | WS_TABSTOP | WS_BORDER;
    let title = if user.name.is_empty() {
        "New trap user"
    } else {
        "Trap user"
    };
    let mut template = DialogTemplate::new(title, 250, 196, "Segoe UI", 9);
    for (row, (id, label)) in FIELDS.into_iter().enumerate() {
        let y = 7 + 17 * row as i16;
        template = template.control(
            ControlClass::Static,
            label,
            0,
            WINDOW_STYLE(0),
            none,
            (7, y + 2, 76, 9),
        );
        template = match id {
            LEVEL | AUTH_PROTOCOL | PRIV_PROTOCOL => template.control(
                ControlClass::ComboBox,
                "",
                id,
                style(CBS_DROPDOWNLIST) | WS_VSCROLL | WS_TABSTOP,
                none,
                (86, y, 100, 100),
            ),
            AUTH_PASSWORD | PRIV_PASSWORD => template.control(
                ControlClass::Edit,
                "",
                id,
                edit | style(ES_PASSWORD),
                WS_EX_CLIENTEDGE,
                (86, y, 157, 13),
            ),
            _ => template.control(
                ControlClass::Edit,
                "",
                id,
                edit,
                WS_EX_CLIENTEDGE,
                (86, y, 157, 13),
            ),
        };
    }
    let template = template
        .control(
            ControlClass::Static,
            "Leave the engine ID empty to accept this user from any sender. A \
             sender's engine ID is in its configuration (net-snmp: the -e of \
             snmptrap, or oldEngineID in snmpd's persistent file). An empty \
             privacy password means the same as the authentication password. \
             Passwords are kept in the Windows Credential Manager.",
            0,
            WINDOW_STYLE(SS_NOPREFIX),
            none,
            (7, 128, 236, 44),
        )
        .control(
            ControlClass::Button,
            "OK",
            OK,
            style(BS_DEFPUSHBUTTON) | WS_TABSTOP,
            none,
            (133, 176, 52, 14),
        )
        .control(
            ControlClass::Button,
            "Cancel",
            CANCEL,
            style(BS_PUSHBUTTON) | WS_TABSTOP,
            none,
            (189, 176, 54, 14),
        );
    let dialog = TrapUserDialog {
        user: RefCell::new(user.clone()),
        others,
    };
    match template.run_modal(Some(owner), &dialog) {
        Ok(1) => Some(dialog.user.into_inner()),
        Ok(_) => None,
        Err(error) => {
            error_box(
                Some(owner),
                &format!("The Trap user dialog could not be opened.\n\n{error}"),
            );
            None
        }
    }
}

struct TrapUserDialog<'a> {
    user: RefCell<SavedTrapUser>,
    others: &'a [SavedTrapUser],
}

impl Dialog for TrapUserDialog<'_> {
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<isize> {
        match msg {
            WM_INITDIALOG => {
                self.init(hwnd);
                // 0: the focus was set here.
                Some(0)
            }
            WM_COMMAND => {
                match (loword(wparam.0), u32::from(hiword(wparam.0))) {
                    (LEVEL, CBN_SELCHANGE) => self.enable_fields(hwnd),
                    (OK, _) => {
                        self.store(hwnd);
                        let problem = self.user.borrow().problem(self.others.iter());
                        match problem {
                            Some(problem) => error_box(
                                Some(hwnd),
                                &format!("This trap user cannot be saved yet.\n\n{problem}"),
                            ),
                            // SAFETY: ends the modal loop of our own dialog.
                            None => unsafe {
                                let _ = EndDialog(hwnd, 1);
                            },
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
                Some(1)
            }
            _ => None,
        }
    }
}

impl TrapUserDialog<'_> {
    fn init(&self, hwnd: HWND) {
        let user = self.user.borrow().clone();
        let set = |id: u16, text: &str| {
            // SAFETY: setting the text of one of our own controls.
            unsafe {
                let _ = SetWindowTextW(item(hwnd, id), &HSTRING::from(text));
            }
        };
        set(NAME, &user.name);
        set(ENGINE, &user.engine_id);
        set(AUTH_PASSWORD, &user.auth_password);
        set(PRIV_PASSWORD, &user.priv_password);
        let fill = |id: u16, labels: &[&str], selected: Option<usize>| {
            let combo = item(hwnd, id);
            // SAFETY: plain combo box messages; the strings live until each
            // call returns, and the combo box copies them.
            unsafe {
                for label in labels {
                    let label = HSTRING::from(*label);
                    SendMessageW(
                        combo,
                        CB_ADDSTRING,
                        None,
                        Some(LPARAM(label.as_ptr() as isize)),
                    );
                }
                send(combo, CB_SETCURSEL, selected.unwrap_or(0), 0);
            }
        };
        fill(
            LEVEL,
            &SecurityLevel::ALL.map(SecurityLevel::name),
            SecurityLevel::ALL
                .iter()
                .position(|&l| l == user.security_level),
        );
        fill(
            AUTH_PROTOCOL,
            &AuthProtocol::ALL.map(AuthProtocol::name),
            AuthProtocol::ALL
                .iter()
                .position(|&p| p == user.auth_protocol),
        );
        fill(
            PRIV_PROTOCOL,
            &PrivProtocol::ALL.map(priv_label),
            PrivProtocol::ALL
                .iter()
                .position(|&p| p == user.priv_protocol),
        );
        self.enable_fields(hwnd);
        // SAFETY: moving the focus to our own control.
        unsafe {
            let _ = SetFocus(Some(item(hwnd, NAME)));
        }
    }

    fn level(&self, hwnd: HWND) -> SecurityLevel {
        selected(hwnd, LEVEL)
            .and_then(|i| SecurityLevel::ALL.get(i).copied())
            .unwrap_or_default()
    }

    /// Enable the protocol and password fields the level uses.
    fn enable_fields(&self, hwnd: HWND) {
        let level = self.level(hwnd);
        for (id, on) in [
            (AUTH_PROTOCOL, level.has_auth()),
            (AUTH_PASSWORD, level.has_auth()),
            (PRIV_PROTOCOL, level.has_priv()),
            (PRIV_PASSWORD, level.has_priv()),
        ] {
            // SAFETY: enabling one of our own controls.
            unsafe {
                let _ = EnableWindow(item(hwnd, id), on);
            }
        }
    }

    /// Copy the fields into the user.
    fn store(&self, hwnd: HWND) {
        let level = self.level(hwnd);
        let text = |id: u16| window_text(item(hwnd, id));
        *self.user.borrow_mut() = SavedTrapUser {
            name: text(NAME).trim().to_string(),
            engine_id: text(ENGINE).trim().to_string(),
            security_level: level,
            auth_protocol: selected(hwnd, AUTH_PROTOCOL)
                .and_then(|i| AuthProtocol::ALL.get(i).copied())
                .unwrap_or_default(),
            priv_protocol: selected(hwnd, PRIV_PROTOCOL)
                .and_then(|i| PrivProtocol::ALL.get(i).copied())
                .unwrap_or_default(),
            // Passwords are kept as typed, spaces included, and only for the
            // level that uses them.
            auth_password: if level.has_auth() {
                text(AUTH_PASSWORD)
            } else {
                String::new()
            },
            priv_password: if level.has_priv() {
                text(PRIV_PASSWORD)
            } else {
                String::new()
            },
        };
    }
}

fn selected(hwnd: HWND, id: u16) -> Option<usize> {
    // SAFETY: plain combo box message.
    let index = unsafe { send(item(hwnd, id), CB_GETCURSEL, 0, 0).0 };
    usize::try_from(index).ok()
}

/// A control of the dialog by id.
fn item(dialog: HWND, id: u16) -> HWND {
    // SAFETY: looks up a child window; a missing one gives a null handle,
    // which later calls ignore.
    unsafe { GetDlgItem(Some(dialog), i32::from(id)).unwrap_or_default() }
}
