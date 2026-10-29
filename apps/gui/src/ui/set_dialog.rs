//! The Set value dialog: write one value to the agent.
//!
//! ```text
//! Object:   [FEROSNMP-TEST-MIB::fstMode.0                 ]
//!           INTEGER (off(1), on(2), auto(3)), read-write
//! Value:    [auto                                       v]
//! Send as:  [The MIB type (INTEGER)                     v]
//!           [ ] Send even if the MIB does not allow it
//!           Sends INTEGER auto(3).
//!                                           [Set] [Cancel]
//! ```
//!
//! The value is checked against the MIB as it is typed (see
//! [`crate::set_value`]), and Set is only enabled for a value that can be
//! sent. The dialog only builds the value; the main window sends it.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{BST_CHECKED, EM_SETSEL};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL,
    CB_RESETCONTENT, CB_SETCURSEL, CBN_EDITCHANGE, CBN_SELCHANGE, CBS_AUTOHSCROLL, CBS_DROPDOWN,
    CBS_DROPDOWNLIST, EN_CHANGE, ES_AUTOHSCROLL, EndDialog, GetDlgItem, SendMessageW,
    SetWindowTextW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_COMMAND, WM_INITDIALOG, WS_BORDER,
    WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VSCROLL,
};
use windows::core::HSTRING;

use fero_win32::dialog::{ControlClass, Dialog, DialogTemplate};
use fero_win32::text::window_text;
use fero_win32::{hiword, loword};

use mib_core::MibSet;
use snmp_core::{Value, Varbind};

use crate::set_value::{Check, SetForm};

use super::{error_box, send};

const OBJECT: u16 = 1001;
const ABOUT: u16 = 1002;
const VALUE: u16 = 1003;
const SEND_AS: u16 = 1004;
const FORCE: u16 = 1005;
const STATUS: u16 = 1006;
const OK: u16 = 1;
const CANCEL: u16 = 2;

// Static control style (not in the parts of the windows crate we use).
const SS_NOPREFIX: u32 = 0x0080;

/// What to send, when Set was pressed.
#[derive(Debug, Clone)]
pub struct SetRequest {
    /// The instance and the value.
    pub varbind: Varbind,
    /// The status line when Set was pressed: what goes out, and why the
    /// MIB would refuse it if the check was turned off.
    pub message: String,
}

/// Show the dialog for `object` (as in the Object box, or the instance of a
/// result or table cell), filled in with its `current` value if known.
pub fn show(
    owner: HWND,
    mibs: Option<&MibSet>,
    object: &str,
    current: Option<&Value>,
) -> Option<SetRequest> {
    let none = WINDOW_EX_STYLE(0);
    let style = |bits: i32| WINDOW_STYLE(bits as u32);
    let label = |template: DialogTemplate, text: &str, y: i16| {
        template.control(
            ControlClass::Static,
            text,
            0,
            WINDOW_STYLE(0),
            none,
            (7, y + 2, 44, 9),
        )
    };
    let mut template = DialogTemplate::new("Set value", 330, 150, "Segoe UI", 9);
    template = label(template, "&Object:", 7);
    template = template
        .control(
            ControlClass::Edit,
            "",
            OBJECT,
            style(ES_AUTOHSCROLL) | WS_TABSTOP | WS_BORDER,
            WS_EX_CLIENTEDGE,
            (54, 7, 269, 13),
        )
        .control(
            ControlClass::Static,
            "",
            ABOUT,
            WINDOW_STYLE(SS_NOPREFIX),
            none,
            (54, 24, 269, 18),
        );
    template = label(template, "&Value:", 46);
    template = template.control(
        ControlClass::ComboBox,
        "",
        VALUE,
        style(CBS_DROPDOWN | CBS_AUTOHSCROLL) | WS_VSCROLL | WS_TABSTOP,
        none,
        (54, 46, 269, 120),
    );
    template = label(template, "Send &as:", 64);
    template = template
        .control(
            ControlClass::ComboBox,
            "",
            SEND_AS,
            style(CBS_DROPDOWNLIST) | WS_VSCROLL | WS_TABSTOP,
            none,
            (54, 64, 140, 160),
        )
        .control(
            ControlClass::Button,
            "Send even if the &MIB does not allow it (to test the agent)",
            FORCE,
            style(BS_AUTOCHECKBOX) | WS_TABSTOP,
            none,
            (54, 82, 269, 10),
        )
        .control(
            ControlClass::Static,
            "",
            STATUS,
            WINDOW_STYLE(SS_NOPREFIX),
            none,
            (54, 98, 269, 27),
        )
        .control(
            ControlClass::Button,
            "&Set",
            OK,
            style(BS_DEFPUSHBUTTON) | WS_TABSTOP,
            none,
            (213, 129, 52, 14),
        )
        .control(
            ControlClass::Button,
            "Cancel",
            CANCEL,
            style(BS_PUSHBUTTON) | WS_TABSTOP,
            none,
            (269, 129, 54, 14),
        );

    let dialog = SetDialog {
        mibs: mibs.cloned(),
        object: object.trim().to_string(),
        current: current.cloned(),
        form: RefCell::new(SetForm::new(mibs, object)),
        filling: Cell::new(false),
        result: RefCell::new(None),
    };
    match template.run_modal(Some(owner), &dialog) {
        Ok(1) => dialog.result.into_inner(),
        Ok(_) => None,
        Err(error) => {
            error_box(
                Some(owner),
                &format!("The Set value dialog could not be opened.\n\n{error}"),
            );
            None
        }
    }
}

struct SetDialog {
    mibs: Option<MibSet>,
    /// The object to start with.
    object: String,
    /// Its current value, to start with.
    current: Option<Value>,
    /// The object as typed now.
    form: RefCell<SetForm>,
    /// True while the dialog fills in its own controls, whose change
    /// notifications are then ignored.
    filling: Cell<bool>,
    /// What Set sends.
    result: RefCell<Option<SetRequest>>,
}

impl Dialog for SetDialog {
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM) -> Option<isize> {
        match msg {
            WM_INITDIALOG => {
                self.init(hwnd);
                // 0: the focus was set here.
                Some(0)
            }
            WM_COMMAND => {
                self.command(hwnd, loword(wparam.0), u32::from(hiword(wparam.0)));
                Some(1)
            }
            _ => None,
        }
    }
}

impl SetDialog {
    fn init(&self, hwnd: HWND) {
        self.filling.set(true);
        let name = self
            .form
            .borrow()
            .name()
            .map_or_else(|| self.object.clone(), str::to_string);
        set_text(hwnd, OBJECT, &name);
        self.fill_lists(hwnd);
        let start = self
            .current
            .as_ref()
            .map(|value| self.form.borrow().edit_text(value));
        if let Some((position, text)) = start {
            set_text(hwnd, VALUE, &text);
            // SAFETY: plain combo box message.
            unsafe {
                send(item(hwnd, SEND_AS), CB_SETCURSEL, position, 0);
            }
        }
        self.filling.set(false);
        self.update(hwnd);
        // Start in the Value box with its text selected, ready to type over;
        // in the Object box when there is no object yet.
        let first = if self.form.borrow().oid().is_some() {
            VALUE
        } else {
            OBJECT
        };
        // SAFETY: moving the focus inside our own dialog.
        unsafe {
            let _ = SetFocus(Some(item(hwnd, first)));
            if first == VALUE {
                // CB_SETEDITSEL: select all of the text.
                send(item(hwnd, VALUE), 0x0142, 0, 0xFFFF_0000u32 as isize);
            } else {
                send(item(hwnd, OBJECT), EM_SETSEL, 0, -1);
            }
        }
    }

    /// Fill the Value and Send as lists for the object.
    fn fill_lists(&self, hwnd: HWND) {
        let (choices, types) = {
            let form = self.form.borrow();
            (form.choices(), form.type_names())
        };
        let value = item(hwnd, VALUE);
        let send_as = item(hwnd, SEND_AS);
        // Resetting the Value list empties its text too; keep it.
        let text = window_text(value);
        // SAFETY: plain combo box messages; each string lives until its call
        // returns, and the combo box copies it.
        unsafe {
            for (combo, entries) in [(value, &choices), (send_as, &types)] {
                send(combo, CB_RESETCONTENT, 0, 0);
                for entry in entries {
                    let entry = HSTRING::from(entry.as_str());
                    SendMessageW(
                        combo,
                        CB_ADDSTRING,
                        None,
                        Some(LPARAM(entry.as_ptr() as isize)),
                    );
                }
            }
            send(send_as, CB_SETCURSEL, 0, 0);
            let _ = SetWindowTextW(value, &HSTRING::from(text));
        }
    }

    fn command(&self, hwnd: HWND, id: u16, code: u32) {
        if self.filling.get() {
            return;
        }
        match (id, code) {
            (OBJECT, EN_CHANGE) => {
                let text = window_text(item(hwnd, OBJECT));
                *self.form.borrow_mut() = SetForm::new(self.mibs.as_ref(), &text);
                self.filling.set(true);
                self.fill_lists(hwnd);
                self.filling.set(false);
                self.update(hwnd);
            }
            (VALUE, CBN_SELCHANGE) => {
                // The edit part still has the old text at this point: take
                // the chosen entry from the list.
                // SAFETY: plain combo box message.
                let index = unsafe { send(item(hwnd, VALUE), CB_GETCURSEL, 0, 0).0 };
                let choice = usize::try_from(index)
                    .ok()
                    .and_then(|i| self.form.borrow().choices().get(i).cloned());
                if let Some(choice) = choice {
                    self.update_with(hwnd, &choice);
                }
            }
            (VALUE, CBN_EDITCHANGE) | (SEND_AS, CBN_SELCHANGE) | (FORCE, _) => self.update(hwnd),
            (OK, _) => {
                if self.result.borrow().is_some() {
                    // SAFETY: ends the modal loop of our own dialog.
                    unsafe {
                        let _ = EndDialog(hwnd, 1);
                    }
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

    fn update(&self, hwnd: HWND) {
        let text = window_text(item(hwnd, VALUE));
        self.update_with(hwnd, &text);
    }

    /// Check `text` as the value, and show the result.
    fn update_with(&self, hwnd: HWND, text: &str) {
        // SAFETY: plain control messages.
        let (position, force) = unsafe {
            (
                send(item(hwnd, SEND_AS), CB_GETCURSEL, 0, 0).0,
                send(item(hwnd, FORCE), BM_GETCHECK, 0, 0).0 == BST_CHECKED.0 as isize,
            )
        };
        let value_type = usize::try_from(position).ok().and_then(SetForm::value_type);
        let (about, check) = {
            let form = self.form.borrow();
            (form.about(), form.check(text, value_type, force))
        };
        let (status, ready) = match check {
            Check::Ready { varbind, message } => {
                *self.result.borrow_mut() = Some(SetRequest {
                    varbind,
                    message: message.clone(),
                });
                (message, true)
            }
            Check::Refused { message, can_force } => {
                *self.result.borrow_mut() = None;
                let status = if can_force && !force {
                    format!(
                        "{message} Tick \u{201c}Send even if\u{2026}\u{201d} to send it anyway."
                    )
                } else {
                    message
                };
                (status, false)
            }
        };
        set_text(hwnd, ABOUT, &about);
        set_text(hwnd, STATUS, &status);
        // SAFETY: enabling a button of our own dialog.
        unsafe {
            let _ = EnableWindow(item(hwnd, OK), ready);
        }
    }
}

fn set_text(dialog: HWND, id: u16, text: &str) {
    // SAFETY: setting the text of a control of our own dialog.
    unsafe {
        let _ = SetWindowTextW(item(dialog, id), &HSTRING::from(text));
    }
}

/// A control of the dialog by id.
fn item(dialog: HWND, id: u16) -> HWND {
    // SAFETY: looks up a child window; a missing one gives a null handle,
    // which later calls ignore.
    unsafe { GetDlgItem(Some(dialog), i32::from(id)).unwrap_or_default() }
}
