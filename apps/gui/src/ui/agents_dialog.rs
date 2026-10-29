//! The Agents dialog: add, change and delete saved agents.
//!
//! The list on the left shows the saved agents; the fields on the right
//! show the selected one: the common settings in the middle, the SNMPv3
//! user on the right. Changes are kept when another agent is selected, and
//! checked when OK is pressed. SNMPv3 passwords are stored in the Windows
//! Credential Manager by the main window, never in the settings file.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE,
    CBS_DROPDOWNLIST, ES_AUTOHSCROLL, ES_NUMBER, ES_PASSWORD, EndDialog, GetDlgItem, LB_ADDSTRING,
    LB_DELETESTRING, LB_ERR, LB_GETCURSEL, LB_INSERTSTRING, LB_RESETCONTENT, LB_SETCURSEL,
    LBN_SELCHANGE, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, SendMessageW, SetWindowTextW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_COMMAND, WM_INITDIALOG, WS_BORDER, WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VSCROLL,
};
use windows::core::HSTRING;

use fero_win32::dialog::{ControlClass, Dialog, DialogTemplate};
use fero_win32::text::window_text;
use fero_win32::{hiword, loword};

use snmp_core::{AuthProtocol, PrivProtocol, SecurityLevel};

use crate::targets::{SavedTarget, VersionSetting};

use super::{error_box, send};

const LIST: u16 = 1001;
const NEW: u16 = 1002;
const DELETE: u16 = 1003;
const NAME: u16 = 1010;
const HOST: u16 = 1011;
const PORT: u16 = 1012;
const VERSION: u16 = 1013;
const COMMUNITY: u16 = 1014;
const TIMEOUT: u16 = 1015;
const RETRIES: u16 = 1016;
const REPETITIONS: u16 = 1017;
const USER: u16 = 1020;
const LEVEL: u16 = 1021;
const AUTH_PROTOCOL: u16 = 1022;
const AUTH_PASSWORD: u16 = 1023;
const PRIV_PROTOCOL: u16 = 1024;
const PRIV_PASSWORD: u16 = 1025;
const CONTEXT: u16 = 1026;
const OK: u16 = 1;
const CANCEL: u16 = 2;

/// The fields of an agent, in the order they are shown.
const FIELDS: [(u16, &str); 8] = [
    (NAME, "&Name:"),
    (HOST, "&Host or address:"),
    (PORT, "&Port:"),
    (VERSION, "&Version:"),
    (COMMUNITY, "&Community:"),
    (TIMEOUT, "&Timeout (seconds):"),
    (RETRIES, "&Retries:"),
    (REPETITIONS, "&Max-repetitions:"),
];

/// The SNMPv3 fields, in the right-hand column.
const V3_FIELDS: [(u16, &str); 7] = [
    (USER, "&User:"),
    (LEVEL, "&Security level:"),
    (AUTH_PROTOCOL, "A&uth protocol:"),
    (AUTH_PASSWORD, "Auth pass&word:"),
    (PRIV_PROTOCOL, "Pri&vacy protocol:"),
    (PRIV_PASSWORD, "Privacy pa&ssword:"),
    (CONTEXT, "Conte&xt name:"),
];

// Static control style (not in the parts of the windows crate we use).
const SS_NOPREFIX: u32 = 0x0080;

/// What the dialog returns when OK is pressed.
#[derive(Debug)]
pub struct Edited {
    /// The saved agents.
    pub targets: Vec<SavedTarget>,
    /// The agent selected when OK was pressed, to put in the Agent box.
    pub selected: Option<String>,
}

/// Show the Agents dialog. `current` is the text of the Agent box: that
/// agent is selected, or, if it is an address, offered for a new agent.
pub fn show(owner: HWND, targets: &[SavedTarget], current: &str) -> Option<Edited> {
    let none = WINDOW_EX_STYLE(0);
    let style = |bits: i32| WINDOW_STYLE(bits as u32);
    let button = style(BS_PUSHBUTTON) | WS_TABSTOP;
    let edit = style(ES_AUTOHSCROLL) | WS_TABSTOP | WS_BORDER;
    let mut template = DialogTemplate::new("Agents", 520, 184, "Segoe UI", 9)
        .control(
            ControlClass::Static,
            "Saved &agents:",
            0,
            WINDOW_STYLE(0),
            none,
            (7, 7, 110, 9),
        )
        .control(
            ControlClass::ListBox,
            "",
            LIST,
            style(LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) | WS_VSCROLL | WS_TABSTOP | WS_BORDER,
            WS_EX_CLIENTEDGE,
            (7, 18, 110, 120),
        )
        .control(
            ControlClass::Button,
            "Ne&w",
            NEW,
            button,
            none,
            (7, 142, 53, 14),
        )
        .control(
            ControlClass::Button,
            "&Delete",
            DELETE,
            button,
            none,
            (64, 142, 53, 14),
        );
    for (row, (id, label)) in FIELDS.into_iter().enumerate() {
        let y = 18 + 17 * row as i16;
        template = template.control(
            ControlClass::Static,
            label,
            0,
            WINDOW_STYLE(0),
            none,
            (125, y + 2, 68, 9),
        );
        template = match id {
            VERSION => template.control(
                ControlClass::ComboBox,
                "",
                id,
                style(CBS_DROPDOWNLIST) | WS_VSCROLL | WS_TABSTOP,
                none,
                (196, y, 70, 60),
            ),
            PORT | RETRIES | REPETITIONS => template.control(
                ControlClass::Edit,
                "",
                id,
                edit | style(ES_NUMBER),
                WS_EX_CLIENTEDGE,
                (196, y, 40, 13),
            ),
            TIMEOUT => template.control(
                ControlClass::Edit,
                "",
                id,
                edit,
                WS_EX_CLIENTEDGE,
                (196, y, 40, 13),
            ),
            _ => template.control(
                ControlClass::Edit,
                "",
                id,
                edit,
                WS_EX_CLIENTEDGE,
                (196, y, 127, 13),
            ),
        };
    }
    template = template.control(
        ControlClass::Static,
        "SNMPv3",
        0,
        WINDOW_STYLE(0),
        none,
        (335, 7, 120, 9),
    );
    for (row, (id, label)) in V3_FIELDS.into_iter().enumerate() {
        let y = 18 + 17 * row as i16;
        template = template.control(
            ControlClass::Static,
            label,
            0,
            WINDOW_STYLE(0),
            none,
            (335, y + 2, 68, 9),
        );
        template = match id {
            LEVEL | AUTH_PROTOCOL | PRIV_PROTOCOL => template.control(
                ControlClass::ComboBox,
                "",
                id,
                style(CBS_DROPDOWNLIST) | WS_VSCROLL | WS_TABSTOP,
                none,
                (406, y, 90, 100),
            ),
            AUTH_PASSWORD | PRIV_PASSWORD => template.control(
                ControlClass::Edit,
                "",
                id,
                edit | style(ES_PASSWORD),
                WS_EX_CLIENTEDGE,
                (406, y, 107, 13),
            ),
            _ => template.control(
                ControlClass::Edit,
                "",
                id,
                edit,
                WS_EX_CLIENTEDGE,
                (406, y, 107, 13),
            ),
        };
    }
    let template = template
        .control(
            ControlClass::Static,
            "Passwords are kept in the Windows Credential Manager, not in the \
             settings file. An empty privacy password means the same as the \
             authentication password.",
            0,
            WINDOW_STYLE(SS_NOPREFIX),
            none,
            (335, 137, 178, 26),
        )
        .control(
            ControlClass::Static,
            "An address typed in the Agent box without saving it uses SNMPv2c, \
             community public, 2 seconds and 2 retries.",
            0,
            WINDOW_STYLE(SS_NOPREFIX),
            none,
            (7, 161, 300, 18),
        )
        .control(
            ControlClass::Button,
            "OK",
            OK,
            style(BS_DEFPUSHBUTTON) | WS_TABSTOP,
            none,
            (403, 165, 52, 14),
        )
        .control(
            ControlClass::Button,
            "Cancel",
            CANCEL,
            button,
            none,
            (459, 165, 54, 14),
        );

    let current = current.trim();
    let selected = targets
        .iter()
        .position(|t| t.name.trim().eq_ignore_ascii_case(current));
    let dialog = AgentsDialog {
        targets: RefCell::new(targets.to_vec()),
        shown: Cell::new(None),
        initial: selected,
        // An address typed in the Agent box becomes a new agent at once.
        new_host: (selected.is_none() && !current.is_empty()).then(|| current.to_string()),
    };
    match template.run_modal(Some(owner), &dialog) {
        Ok(1) => {
            let selected = dialog.shown.get().and_then(|i| {
                dialog
                    .targets
                    .borrow()
                    .get(i)
                    .map(|t| t.name.trim().to_string())
            });
            let mut targets = dialog.targets.into_inner();
            for target in &mut targets {
                target.name = target.name.trim().to_string();
                target.host = target.host.trim().to_string();
            }
            Some(Edited { targets, selected })
        }
        Ok(_) => None,
        Err(error) => {
            error_box(
                Some(owner),
                &format!("The Agents dialog could not be opened.\n\n{error}"),
            );
            None
        }
    }
}

struct AgentsDialog {
    /// The agents as edited so far.
    targets: RefCell<Vec<SavedTarget>>,
    /// The agent the fields show.
    shown: Cell<Option<usize>>,
    /// The agent to select at the start.
    initial: Option<usize>,
    /// A host to start a new agent with.
    new_host: Option<String>,
}

impl Dialog for AgentsDialog {
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

impl AgentsDialog {
    fn init(&self, hwnd: HWND) {
        let fill = |id: u16, labels: &[&str]| {
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
            }
        };
        fill(VERSION, &VersionSetting::ALL.map(VersionSetting::label));
        fill(LEVEL, &SecurityLevel::ALL.map(SecurityLevel::name));
        fill(AUTH_PROTOCOL, &AuthProtocol::ALL.map(AuthProtocol::name));
        fill(PRIV_PROTOCOL, &PrivProtocol::ALL.map(priv_label));
        self.fill_list(hwnd);
        if let Some(host) = &self.new_host {
            self.add(hwnd, host);
        } else {
            let first = self
                .initial
                .or((!self.targets.borrow().is_empty()).then_some(0));
            self.show_target(hwnd, first);
            // SAFETY: moving the focus inside our own dialog.
            unsafe {
                let _ = SetFocus(Some(item(hwnd, LIST)));
            }
        }
    }

    fn command(&self, hwnd: HWND, id: u16, code: u32) {
        match (id, code) {
            (LIST, LBN_SELCHANGE) => {
                self.store(hwnd);
                self.show_target(hwnd, list_selection(hwnd));
            }
            (VERSION | LEVEL, CBN_SELCHANGE) => self.enable_fields(hwnd),
            (NEW, _) => {
                self.store(hwnd);
                self.add(hwnd, "");
            }
            (DELETE, _) => {
                if let Some(index) = self.shown.get() {
                    self.targets.borrow_mut().remove(index);
                    // SAFETY: plain list box message.
                    unsafe {
                        send(item(hwnd, LIST), LB_DELETESTRING, index, 0);
                    }
                    let count = self.targets.borrow().len();
                    self.shown.set(None);
                    self.show_target(hwnd, (count > 0).then(|| index.min(count - 1)));
                }
            }
            (OK, _) => {
                self.store(hwnd);
                if self.check(hwnd) {
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

    /// Add a new agent with `host` and show it, with the name selected for
    /// typing.
    fn add(&self, hwnd: HWND, host: &str) {
        let name = {
            let targets = self.targets.borrow();
            let base = if host.is_empty() { "New agent" } else { host };
            let taken = |name: &str| targets.iter().any(|t| t.name.eq_ignore_ascii_case(name));
            (1..)
                .map(|n| {
                    if n == 1 {
                        base.to_string()
                    } else {
                        format!("{base} ({n})")
                    }
                })
                .find(|name| !taken(name))
                .expect("some name is free")
        };
        let target = SavedTarget {
            name,
            host: host.to_string(),
            ..SavedTarget::default()
        };
        let label = HSTRING::from(target.name.as_str());
        self.targets.borrow_mut().push(target);
        let index = self.targets.borrow().len() - 1;
        // SAFETY: plain list box message; the string lives until it returns.
        unsafe {
            SendMessageW(
                item(hwnd, LIST),
                LB_ADDSTRING,
                None,
                Some(LPARAM(label.as_ptr() as isize)),
            );
        }
        self.show_target(hwnd, Some(index));
        let name = item(hwnd, NAME);
        // SAFETY: moving the focus inside our own dialog.
        unsafe {
            let _ = SetFocus(Some(name));
            send(name, EM_SETSEL, 0, -1);
        }
    }

    fn fill_list(&self, hwnd: HWND) {
        let list = item(hwnd, LIST);
        // SAFETY: plain list box messages; each string lives until its call
        // returns.
        unsafe {
            send(list, LB_RESETCONTENT, 0, 0);
            for target in self.targets.borrow().iter() {
                let label = HSTRING::from(target.name.as_str());
                SendMessageW(
                    list,
                    LB_ADDSTRING,
                    None,
                    Some(LPARAM(label.as_ptr() as isize)),
                );
            }
        }
    }

    /// Show an agent in the fields, or empty and disabled fields.
    fn show_target(&self, hwnd: HWND, index: Option<usize>) {
        let target = index.and_then(|i| self.targets.borrow().get(i).cloned());
        self.shown.set(target.as_ref().and(index));
        let set = |id: u16, text: &str| {
            // SAFETY: setting the text of one of our own controls.
            unsafe {
                let _ = SetWindowTextW(item(hwnd, id), &HSTRING::from(text));
            }
        };
        let enabled = target.is_some();
        let target = target.unwrap_or_default();
        set(NAME, &target.name);
        set(HOST, &target.host);
        set(PORT, &target.port.to_string());
        set(COMMUNITY, &target.community);
        set(TIMEOUT, &target.timeout.to_string());
        set(RETRIES, &target.retries.to_string());
        set(REPETITIONS, &target.max_repetitions.to_string());
        set(USER, &target.user);
        set(AUTH_PASSWORD, &target.auth_password);
        set(PRIV_PASSWORD, &target.priv_password);
        set(CONTEXT, &target.context);
        let position = |found: Option<usize>| found.unwrap_or(0);
        // SAFETY: plain control messages on our own controls.
        unsafe {
            let select = |id: u16, index: usize| send(item(hwnd, id), CB_SETCURSEL, index, 0);
            select(
                VERSION,
                position(
                    VersionSetting::ALL
                        .iter()
                        .position(|&v| v == target.version),
                ),
            );
            select(
                LEVEL,
                position(
                    SecurityLevel::ALL
                        .iter()
                        .position(|&v| v == target.security_level),
                ),
            );
            select(
                AUTH_PROTOCOL,
                position(
                    AuthProtocol::ALL
                        .iter()
                        .position(|&v| v == target.auth_protocol),
                ),
            );
            select(
                PRIV_PROTOCOL,
                position(
                    PrivProtocol::ALL
                        .iter()
                        .position(|&v| v == target.priv_protocol),
                ),
            );
            send(
                item(hwnd, LIST),
                LB_SETCURSEL,
                index.unwrap_or(usize::MAX),
                0,
            );
            let _ = EnableWindow(item(hwnd, DELETE), enabled);
        }
        self.enable_fields(hwnd);
    }

    /// Enable the fields that apply to the selected version and security
    /// level; all are disabled when no agent is shown.
    fn enable_fields(&self, hwnd: HWND) {
        let shown = self.shown.get().is_some();
        let selected = |id: u16| {
            // SAFETY: plain combo box message.
            let index = unsafe { send(item(hwnd, id), CB_GETCURSEL, 0, 0).0 };
            usize::try_from(index).ok()
        };
        let v3 = selected(VERSION).and_then(|i| VersionSetting::ALL.get(i).copied())
            == Some(VersionSetting::V3);
        let level = selected(LEVEL)
            .and_then(|i| SecurityLevel::ALL.get(i).copied())
            .unwrap_or_default();
        let enable = |id: u16, on: bool| {
            // SAFETY: enabling one of our own controls.
            unsafe {
                let _ = EnableWindow(item(hwnd, id), on);
            }
        };
        for (id, _) in FIELDS {
            enable(id, shown && (id != COMMUNITY || !v3));
        }
        for (id, _) in V3_FIELDS {
            let on = match id {
                AUTH_PROTOCOL | AUTH_PASSWORD => level.has_auth(),
                PRIV_PROTOCOL | PRIV_PASSWORD => level.has_priv(),
                _ => true,
            };
            enable(id, shown && v3 && on);
        }
    }

    /// Copy the fields into the agent they show. Text that is not a number
    /// where one is needed becomes a value that [`check`](Self::check)
    /// rejects with an explanation.
    fn store(&self, hwnd: HWND) {
        let Some(index) = self.shown.get() else {
            return;
        };
        let text = |id: u16| window_text(item(hwnd, id)).trim().to_string();
        let selected = |id: u16| {
            // SAFETY: plain combo box message.
            let index = unsafe { send(item(hwnd, id), CB_GETCURSEL, 0, 0).0 };
            usize::try_from(index).ok()
        };
        let name = text(NAME);
        {
            let mut targets = self.targets.borrow_mut();
            let Some(target) = targets.get_mut(index) else {
                return;
            };
            let changed_name = target.name != name;
            *target = SavedTarget {
                name: name.clone(),
                host: text(HOST),
                port: text(PORT).parse().unwrap_or(0),
                version: selected(VERSION)
                    .and_then(|i| VersionSetting::ALL.get(i).copied())
                    .unwrap_or_default(),
                community: window_text(item(hwnd, COMMUNITY)),
                user: text(USER),
                security_level: selected(LEVEL)
                    .and_then(|i| SecurityLevel::ALL.get(i).copied())
                    .unwrap_or_default(),
                auth_protocol: selected(AUTH_PROTOCOL)
                    .and_then(|i| AuthProtocol::ALL.get(i).copied())
                    .unwrap_or_default(),
                // Passwords are kept as typed, spaces included.
                auth_password: window_text(item(hwnd, AUTH_PASSWORD)),
                priv_protocol: selected(PRIV_PROTOCOL)
                    .and_then(|i| PrivProtocol::ALL.get(i).copied())
                    .unwrap_or_default(),
                priv_password: window_text(item(hwnd, PRIV_PASSWORD)),
                context: text(CONTEXT),
                timeout: text(TIMEOUT).replace(',', ".").parse().unwrap_or(f64::NAN),
                retries: text(RETRIES).parse().unwrap_or(u32::MAX),
                max_repetitions: text(REPETITIONS).parse().unwrap_or(0),
            };
            if !changed_name {
                return;
            }
        }
        // Show the new name in the list.
        let list = item(hwnd, LIST);
        let label = HSTRING::from(name.as_str());
        // SAFETY: plain list box messages; the string lives until the call
        // returns.
        unsafe {
            send(list, LB_DELETESTRING, index, 0);
            SendMessageW(
                list,
                LB_INSERTSTRING,
                Some(WPARAM(index)),
                Some(LPARAM(label.as_ptr() as isize)),
            );
            send(list, LB_SETCURSEL, index, 0);
        }
    }

    /// Check every agent; show the first problem and return false.
    fn check(&self, hwnd: HWND) -> bool {
        let problem = {
            let targets = self.targets.borrow();
            targets.iter().enumerate().find_map(|(i, target)| {
                let others = targets
                    .iter()
                    .enumerate()
                    .filter(move |&(j, _)| j != i)
                    .map(|(_, t)| t.name.as_str());
                target.problem(others).map(|problem| (i, problem))
            })
        };
        let Some((index, problem)) = problem else {
            return true;
        };
        self.show_target(hwnd, Some(index));
        let name = self.targets.borrow()[index].name.clone();
        let title = if name.trim().is_empty() {
            "This agent".to_string()
        } else {
            format!("Agent \u{201c}{}\u{201d}", name.trim())
        };
        error_box(
            Some(hwnd),
            &format!("{title} cannot be saved yet.\n\n{problem}"),
        );
        false
    }
}

/// What the privacy list shows: the name, and which AES key extension.
fn priv_label(protocol: PrivProtocol) -> &'static str {
    match protocol {
        PrivProtocol::Des => "DES",
        PrivProtocol::Aes128 => "AES-128",
        PrivProtocol::Aes192 => "AES-192",
        PrivProtocol::Aes256 => "AES-256",
        PrivProtocol::Aes192Cisco => "AES-192 (Cisco)",
        PrivProtocol::Aes256Cisco => "AES-256 (Cisco)",
    }
}

fn list_selection(hwnd: HWND) -> Option<usize> {
    // SAFETY: plain list box message.
    let index = unsafe { send(item(hwnd, LIST), LB_GETCURSEL, 0, 0).0 };
    (index != LB_ERR as isize).then_some(index as usize)
}

/// A control of the dialog by id.
fn item(dialog: HWND, id: u16) -> HWND {
    // SAFETY: looks up a child window; a missing one gives a null handle,
    // which later calls ignore.
    unsafe { GetDlgItem(Some(dialog), i32::from(id)).unwrap_or_default() }
}
