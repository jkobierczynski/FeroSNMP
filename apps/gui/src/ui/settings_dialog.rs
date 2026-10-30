//! The Settings dialog: three pages on tabs.
//!
//! - **MIBs**: the MIB folders and how strictly they are read.
//! - **Traps**: the port and address the Traps tab listens on, and the
//!   SNMPv3 users traps and INFORMs are accepted from (see
//!   [`trap_user_dialog`](super::trap_user_dialog)).
//! - **Appearance**: light or dark tree and lists.
//!
//! The pages edit one copy of the settings; it is returned when OK is
//! pressed. A page that was never opened keeps its settings as they were.

use std::cell::RefCell;
use std::path::Path;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED, NMHDR, PSN_APPLY, PSN_KILLACTIVE};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_PUSHBUTTON, CB_ADDSTRING, CB_GETCURSEL,
    CB_SETCURSEL, CBN_SELCHANGE, CBS_DROPDOWNLIST, ES_AUTOHSCROLL, ES_NUMBER, GetDlgItem,
    LB_ADDSTRING, LB_ERR, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, LBN_DBLCLK, LBN_SELCHANGE,
    LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, SendMessageW, SetWindowTextW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_COMMAND, WM_INITDIALOG, WM_NOTIFY, WS_BORDER, WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VSCROLL,
};
use windows::core::HSTRING;

use fero_win32::dialog::{self, ControlClass, Dialog, DialogTemplate};
use fero_win32::shell::{self, FileType};
use fero_win32::text::window_text;
use fero_win32::{hiword, loword};

use crate::settings::{Settings, StrictnessSetting, ThemeSetting};
use crate::trap_users::SavedTrapUser;

use super::{error_box, send, trap_user_dialog};

// The MIBs page.
const FOLDERS: u16 = 1001;
const ADD: u16 = 1002;
const REMOVE: u16 = 1003;
const UP: u16 = 1004;
const DOWN: u16 = 1005;
const SYSTEM_FOLDERS: u16 = 1006;
const STRICTNESS: u16 = 1007;
const EXPLANATION: u16 = 1008;
const FILE: u16 = 1009;
// The Traps page.
const PORT: u16 = 1101;
const ADDRESS: u16 = 1102;
const AT_START: u16 = 1103;
const USERS: u16 = 1104;
const ADD_USER: u16 = 1105;
const EDIT_USER: u16 = 1106;
const REMOVE_USER: u16 = 1107;
const IMPORT_USERS: u16 = 1108;
// The Appearance page.
const THEME: u16 = 1201;

// Static control styles (not in the parts of the windows crate we use).
const SS_NOPREFIX: u32 = 0x0080;
const SS_PATHELLIPSIS: u32 = 0x8000;

/// Size of every page, in dialog units.
const PAGE_SIZE: (i16, i16) = (330, 186);

/// The page to open the dialog on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Mibs,
    Traps,
}

/// Show the Settings dialog. Returns the new settings if the user pressed OK.
pub fn show(
    owner: HWND,
    settings: &Settings,
    file: Option<&Path>,
    start: Page,
) -> Option<Settings> {
    let shared = Shared {
        settings: RefCell::new(settings.clone()),
    };
    let mibs = MibsPage {
        shared: &shared,
        file: match file {
            Some(file) => format!("Saved in {}", file.display()),
            None => "Not saved: APPDATA is not set".to_string(),
        },
    };
    let traps = TrapsPage { shared: &shared };
    let appearance = AppearancePage { shared: &shared };
    let (mibs_template, traps_template, appearance_template) =
        (mibs_page(), traps_page(), appearance_page());
    let pages: [(&str, &DialogTemplate, &dyn Dialog); 3] = [
        ("MIBs", &mibs_template, &mibs),
        ("Traps", &traps_template, &traps),
        ("Appearance", &appearance_template, &appearance),
    ];
    let start = match start {
        Page::Mibs => 0,
        Page::Traps => 1,
    };
    match dialog::tabbed_dialog(Some(owner), "Settings", &pages, start) {
        Ok(true) => Some(shared.settings.into_inner()),
        Ok(false) => None,
        Err(error) => {
            error_box(
                Some(owner),
                &format!("The Settings dialog could not be opened.\n\n{error}"),
            );
            None
        }
    }
}

/// What the pages share: the settings as edited so far.
struct Shared {
    settings: RefCell<Settings>,
}

/// The notification code of a `WM_NOTIFY`.
fn notify_code(lparam: LPARAM) -> u32 {
    // SAFETY: for WM_NOTIFY, lparam points to an NMHDR.
    unsafe { (*(lparam.0 as *const NMHDR)).code }
}

fn none() -> WINDOW_EX_STYLE {
    WINDOW_EX_STYLE(0)
}

fn style(bits: i32) -> WINDOW_STYLE {
    WINDOW_STYLE(bits as u32)
}

fn button() -> WINDOW_STYLE {
    style(BS_PUSHBUTTON) | WS_TABSTOP
}

// ----- the MIBs page -----

fn mibs_page() -> DialogTemplate {
    DialogTemplate::page(PAGE_SIZE.0, PAGE_SIZE.1, "Segoe UI", 9)
        .control(
            ControlClass::Static,
            "MIB &folders, searched from top to bottom:",
            0,
            WINDOW_STYLE(0),
            none(),
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
            button(),
            none(),
            (269, 18, 54, 14),
        )
        .control(
            ControlClass::Button,
            "&Remove",
            REMOVE,
            button(),
            none(),
            (269, 36, 54, 14),
        )
        .control(
            ControlClass::Button,
            "Move &up",
            UP,
            button(),
            none(),
            (269, 54, 54, 14),
        )
        .control(
            ControlClass::Button,
            "Move &down",
            DOWN,
            button(),
            none(),
            (269, 72, 54, 14),
        )
        .control(
            ControlClass::Button,
            "Also use the &net-snmp and libsmi MIB folders on this computer",
            SYSTEM_FOLDERS,
            style(BS_AUTOCHECKBOX) | WS_TABSTOP,
            none(),
            (7, 116, 316, 10),
        )
        .control(
            ControlClass::Static,
            "S&trictness:",
            0,
            WINDOW_STYLE(0),
            none(),
            (7, 136, 50, 9),
        )
        .control(
            ControlClass::ComboBox,
            "",
            STRICTNESS,
            style(CBS_DROPDOWNLIST) | WS_VSCROLL | WS_TABSTOP,
            none(),
            (60, 134, 100, 80),
        )
        .control(
            ControlClass::Static,
            "",
            EXPLANATION,
            WINDOW_STYLE(SS_NOPREFIX),
            none(),
            (7, 152, 316, 18),
        )
        .control(
            ControlClass::Static,
            "",
            FILE,
            WINDOW_STYLE(SS_NOPREFIX | SS_PATHELLIPSIS),
            none(),
            (7, 174, 316, 9),
        )
}

struct MibsPage<'a> {
    shared: &'a Shared,
    /// Where the settings file is, for the line at the bottom.
    file: String,
}

impl Dialog for MibsPage<'_> {
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
        match msg {
            WM_INITDIALOG => {
                self.init(hwnd);
                Some(1)
            }
            WM_COMMAND => {
                self.command(hwnd, loword(wparam.0), u32::from(hiword(wparam.0)));
                Some(1)
            }
            WM_NOTIFY if notify_code(lparam) == PSN_APPLY => {
                self.apply(hwnd);
                Some(1)
            }
            _ => None,
        }
    }
}

impl MibsPage<'_> {
    fn init(&self, hwnd: HWND) {
        let settings = self.shared.settings.borrow().clone();
        let check = if settings.use_system_folders {
            BST_CHECKED
        } else {
            BST_UNCHECKED
        };
        // SAFETY: plain control messages; the strings live until each call
        // returns, and the controls copy them.
        unsafe {
            send(item(hwnd, SYSTEM_FOLDERS), BM_SETCHECK, check.0 as usize, 0);
            let _ = SetWindowTextW(item(hwnd, FILE), &HSTRING::from(self.file.as_str()));
        }
        let selected = StrictnessSetting::ALL
            .iter()
            .position(|&s| s == settings.strictness);
        fill_combo(
            item(hwnd, STRICTNESS),
            &StrictnessSetting::ALL.map(StrictnessSetting::label),
            selected.unwrap_or(0),
        );
        self.fill_folders(hwnd, None);
        self.show_explanation(hwnd);
    }

    fn command(&self, hwnd: HWND, id: u16, code: u32) {
        match (id, code) {
            (ADD, _) => self.add_folders(hwnd),
            (REMOVE, _) => {
                if let Some(index) = selection(item(hwnd, FOLDERS)) {
                    let mut settings = self.shared.settings.borrow_mut();
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
            _ => {}
        }
    }

    /// OK was pressed: keep what the controls say.
    fn apply(&self, hwnd: HWND) {
        let mut settings = self.shared.settings.borrow_mut();
        // SAFETY: plain control messages.
        unsafe {
            settings.use_system_folders =
                send(item(hwnd, SYSTEM_FOLDERS), BM_GETCHECK, 0, 0).0 == BST_CHECKED.0 as isize;
        }
        settings.strictness = self.strictness(hwnd);
    }

    fn add_folders(&self, hwnd: HWND) {
        let picked = match shell::pick_folders(Some(hwnd), "Add MIB folders") {
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
        let mut settings = self.shared.settings.borrow_mut();
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
        let Some(index) = selection(item(hwnd, FOLDERS)) else {
            return;
        };
        let mut settings = self.shared.settings.borrow_mut();
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
        let folders: Vec<String> = self
            .shared
            .settings
            .borrow()
            .mib_folders
            .iter()
            .map(|folder| folder.display().to_string())
            .collect();
        fill_list(item(hwnd, FOLDERS), &folders, select);
        self.update_buttons(hwnd);
    }

    /// Enable Remove, Move up and Move down only when they can do something.
    fn update_buttons(&self, hwnd: HWND) {
        let count = self.shared.settings.borrow().mib_folders.len();
        let selected = selection(item(hwnd, FOLDERS));
        enable(hwnd, REMOVE, selected.is_some());
        enable(hwnd, UP, selected.is_some_and(|i| i > 0));
        enable(hwnd, DOWN, selected.is_some_and(|i| i + 1 < count));
    }

    fn strictness(&self, hwnd: HWND) -> StrictnessSetting {
        combo_selection(item(hwnd, STRICTNESS))
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

// ----- the Traps page -----

fn traps_page() -> DialogTemplate {
    let edit = style(ES_AUTOHSCROLL) | WS_TABSTOP | WS_BORDER;
    DialogTemplate::page(PAGE_SIZE.0, PAGE_SIZE.1, "Segoe UI", 9)
        .control(
            ControlClass::Static,
            "Listen on UDP &port:",
            0,
            WINDOW_STYLE(0),
            none(),
            (7, 9, 70, 9),
        )
        .control(
            ControlClass::Edit,
            "",
            PORT,
            edit | style(ES_NUMBER),
            WS_EX_CLIENTEDGE,
            (80, 7, 36, 13),
        )
        .control(
            ControlClass::Static,
            "A&ddress:",
            0,
            WINDOW_STYLE(0),
            none(),
            (128, 9, 34, 9),
        )
        .control(
            ControlClass::Edit,
            "",
            ADDRESS,
            edit,
            WS_EX_CLIENTEDGE,
            (164, 7, 100, 13),
        )
        .control(
            ControlClass::Static,
            "0.0.0.0 listens on every IPv4 address of this computer. Port 162 is \
             the standard; the Windows SNMP Trap service must be stopped to use it, \
             or use 1162 and send the traps there.",
            0,
            WINDOW_STYLE(SS_NOPREFIX),
            none(),
            (7, 24, 316, 18),
        )
        .control(
            ControlClass::Button,
            "&Start listening when FeroSNMP opens",
            AT_START,
            style(BS_AUTOCHECKBOX) | WS_TABSTOP,
            none(),
            (7, 46, 316, 10),
        )
        .control(
            ControlClass::Static,
            "SNMPv3 trap &users (SNMPv1 and v2c traps are accepted from everyone):",
            0,
            WINDOW_STYLE(0),
            none(),
            (7, 64, 316, 9),
        )
        .control(
            ControlClass::ListBox,
            "",
            USERS,
            style(LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) | WS_VSCROLL | WS_TABSTOP | WS_BORDER,
            WS_EX_CLIENTEDGE,
            (7, 75, 256, 82),
        )
        .control(
            ControlClass::Button,
            "&Add...",
            ADD_USER,
            button(),
            none(),
            (269, 75, 54, 14),
        )
        .control(
            ControlClass::Button,
            "&Edit...",
            EDIT_USER,
            button(),
            none(),
            (269, 93, 54, 14),
        )
        .control(
            ControlClass::Button,
            "&Remove",
            REMOVE_USER,
            button(),
            none(),
            (269, 111, 54, 14),
        )
        .control(
            ControlClass::Button,
            "&Import...",
            IMPORT_USERS,
            button(),
            none(),
            (269, 143, 54, 14),
        )
        .control(
            ControlClass::Static,
            "Passwords are kept in the Windows Credential Manager. Import reads the \
             createUser lines of a snmptrapd.conf.",
            0,
            WINDOW_STYLE(SS_NOPREFIX),
            none(),
            (7, 162, 316, 18),
        )
}

struct TrapsPage<'a> {
    shared: &'a Shared,
}

impl Dialog for TrapsPage<'_> {
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
        match msg {
            WM_INITDIALOG => {
                self.init(hwnd);
                Some(1)
            }
            WM_COMMAND => {
                self.command(hwnd, loword(wparam.0), u32::from(hiword(wparam.0)));
                Some(1)
            }
            WM_NOTIFY => match notify_code(lparam) {
                PSN_KILLACTIVE => {
                    // TRUE keeps the page open, to fix what is wrong.
                    dialog::set_result(hwnd, isize::from(!self.check(hwnd)));
                    Some(1)
                }
                PSN_APPLY => {
                    self.store(hwnd);
                    Some(1)
                }
                _ => None,
            },
            _ => None,
        }
    }
}

impl TrapsPage<'_> {
    fn init(&self, hwnd: HWND) {
        let settings = self.shared.settings.borrow().clone();
        let check = if settings.listen_at_start {
            BST_CHECKED
        } else {
            BST_UNCHECKED
        };
        // SAFETY: plain control messages; the strings live until each call
        // returns, and the controls copy them.
        unsafe {
            let _ = SetWindowTextW(
                item(hwnd, PORT),
                &HSTRING::from(settings.trap_port.to_string()),
            );
            let _ = SetWindowTextW(
                item(hwnd, ADDRESS),
                &HSTRING::from(settings.trap_address.as_str()),
            );
            send(item(hwnd, AT_START), BM_SETCHECK, check.0 as usize, 0);
        }
        self.fill_users(hwnd, None);
    }

    fn command(&self, hwnd: HWND, id: u16, code: u32) {
        match (id, code) {
            (ADD_USER, _) => self.edit_user(hwnd, None),
            (EDIT_USER, _) | (USERS, LBN_DBLCLK) => {
                if let Some(index) = selection(item(hwnd, USERS)) {
                    self.edit_user(hwnd, Some(index));
                }
            }
            (REMOVE_USER, _) => {
                if let Some(index) = selection(item(hwnd, USERS)) {
                    let mut settings = self.shared.settings.borrow_mut();
                    settings.trap_users.remove(index);
                    let next = index.min(settings.trap_users.len().saturating_sub(1));
                    drop(settings);
                    self.fill_users(hwnd, Some(next));
                }
            }
            (IMPORT_USERS, _) => self.import(hwnd),
            (USERS, LBN_SELCHANGE) => self.update_buttons(hwnd),
            _ => {}
        }
    }

    /// Copy the port, address and check box into the settings.
    fn store(&self, hwnd: HWND) {
        let mut settings = self.shared.settings.borrow_mut();
        settings.trap_port = window_text(item(hwnd, PORT)).trim().parse().unwrap_or(0);
        settings.trap_address = window_text(item(hwnd, ADDRESS)).trim().to_string();
        // SAFETY: plain control message.
        settings.listen_at_start =
            unsafe { send(item(hwnd, AT_START), BM_GETCHECK, 0, 0).0 } == BST_CHECKED.0 as isize;
    }

    /// Store the fields and say what is wrong with them, if anything.
    fn check(&self, hwnd: HWND) -> bool {
        self.store(hwnd);
        let problem = self.shared.settings.borrow().trap_listen_address().err();
        match problem {
            Some(problem) => {
                error_box(Some(hwnd), &problem);
                false
            }
            None => true,
        }
    }

    /// Add a user (`index` = `None`) or change one.
    fn edit_user(&self, hwnd: HWND, index: Option<usize>) {
        let (user, others) = {
            let settings = self.shared.settings.borrow();
            let user = index
                .and_then(|i| settings.trap_users.get(i).cloned())
                .unwrap_or_default();
            let others: Vec<SavedTrapUser> = settings
                .trap_users
                .iter()
                .enumerate()
                .filter(|&(i, _)| Some(i) != index)
                .map(|(_, user)| user.clone())
                .collect();
            (user, others)
        };
        let Some(edited) = trap_user_dialog::show(hwnd, &user, &others) else {
            return;
        };
        let mut settings = self.shared.settings.borrow_mut();
        let index = match index {
            Some(index) => {
                settings.trap_users[index] = edited;
                index
            }
            None => {
                settings.trap_users.push(edited);
                settings.trap_users.len() - 1
            }
        };
        drop(settings);
        self.fill_users(hwnd, Some(index));
    }

    /// Add the users of a snmptrapd.conf (or any file with `createUser`
    /// lines), replacing users with the same name and engine.
    fn import(&self, hwnd: HWND) {
        let types = [
            FileType {
                name: "net-snmp configuration (*.conf)",
                pattern: "*.conf",
                extension: "conf",
            },
            FileType {
                name: "All files (*.*)",
                pattern: "*.*",
                extension: "",
            },
        ];
        let path = match shell::open_file(Some(hwnd), "Import trap users", &types) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                error_box(
                    Some(hwnd),
                    &format!("The Open dialog could not be opened.\n\n{error}"),
                );
                return;
            }
        };
        let users = match trap_rx::read_users(&path) {
            Ok(users) if users.is_empty() => {
                error_box(
                    Some(hwnd),
                    &format!("{} has no createUser lines.", path.display()),
                );
                return;
            }
            Ok(users) => users,
            Err(error) => {
                error_box(
                    Some(hwnd),
                    &format!("The trap users could not be read.\n\n{error}"),
                );
                return;
            }
        };
        let mut settings = self.shared.settings.borrow_mut();
        for user in &users {
            let imported = SavedTrapUser::from_trap_user(user);
            let same = settings.trap_users.iter().position(|old| {
                old.name == imported.name
                    && old.engine_id.trim_start_matches("0x").to_lowercase() == imported.engine_id
            });
            match same {
                Some(index) => settings.trap_users[index] = imported,
                None => settings.trap_users.push(imported),
            }
        }
        let last = settings.trap_users.len().checked_sub(1);
        drop(settings);
        self.fill_users(hwnd, last);
    }

    fn fill_users(&self, hwnd: HWND, select: Option<usize>) {
        let labels: Vec<String> = self
            .shared
            .settings
            .borrow()
            .trap_users
            .iter()
            .map(SavedTrapUser::label)
            .collect();
        fill_list(item(hwnd, USERS), &labels, select);
        self.update_buttons(hwnd);
    }

    fn update_buttons(&self, hwnd: HWND) {
        let selected = selection(item(hwnd, USERS)).is_some();
        enable(hwnd, EDIT_USER, selected);
        enable(hwnd, REMOVE_USER, selected);
    }
}

// ----- the Appearance page -----

fn appearance_page() -> DialogTemplate {
    DialogTemplate::page(PAGE_SIZE.0, PAGE_SIZE.1, "Segoe UI", 9)
        .control(
            ControlClass::Static,
            "&Tree and lists:",
            0,
            WINDOW_STYLE(0),
            none(),
            (7, 9, 60, 9),
        )
        .control(
            ControlClass::ComboBox,
            "",
            THEME,
            style(CBS_DROPDOWNLIST) | WS_VSCROLL | WS_TABSTOP,
            none(),
            (70, 7, 100, 60),
        )
        .control(
            ControlClass::Static,
            "Dark makes the MIB tree and the lists dark, as in Explorer. Menus, \
             buttons and boxes stay light. \u{201c}Same as Windows\u{201d} follows \
             Settings > Personalization > Colors > Choose your app mode.",
            0,
            WINDOW_STYLE(SS_NOPREFIX),
            none(),
            (7, 26, 316, 27),
        )
}

struct AppearancePage<'a> {
    shared: &'a Shared,
}

impl Dialog for AppearancePage<'_> {
    fn message(&self, hwnd: HWND, msg: u32, _wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
        match msg {
            WM_INITDIALOG => {
                let theme = self.shared.settings.borrow().theme;
                let selected = ThemeSetting::ALL.iter().position(|&t| t == theme);
                fill_combo(
                    item(hwnd, THEME),
                    &ThemeSetting::ALL.map(ThemeSetting::label),
                    selected.unwrap_or(0),
                );
                Some(1)
            }
            WM_NOTIFY if notify_code(lparam) == PSN_APPLY => {
                self.shared.settings.borrow_mut().theme = combo_selection(item(hwnd, THEME))
                    .and_then(|i| ThemeSetting::ALL.get(i).copied())
                    .unwrap_or_default();
                Some(1)
            }
            _ => None,
        }
    }
}

// ----- helpers -----

/// A control of a dialog or page by id.
fn item(dialog: HWND, id: u16) -> HWND {
    // SAFETY: looks up a child window; a missing one gives a null handle,
    // which later calls ignore.
    unsafe { GetDlgItem(Some(dialog), i32::from(id)).unwrap_or_default() }
}

fn enable(dialog: HWND, id: u16, on: bool) {
    // SAFETY: enabling one of our own controls.
    unsafe {
        let _ = EnableWindow(item(dialog, id), on);
    }
}

/// Fill a list box with `lines`, selecting `select`.
fn fill_list(list: HWND, lines: &[String], select: Option<usize>) {
    // SAFETY: plain list box messages; each string lives until its call
    // returns, and the list box copies it.
    unsafe {
        send(list, LB_RESETCONTENT, 0, 0);
        for line in lines {
            let text = HSTRING::from(line.as_str());
            SendMessageW(
                list,
                LB_ADDSTRING,
                None,
                Some(LPARAM(text.as_ptr() as isize)),
            );
        }
        if let Some(index) = select.filter(|&i| i < lines.len()) {
            send(list, LB_SETCURSEL, index, 0);
        }
    }
}

/// Fill a drop-down list with `labels`, selecting `select`.
fn fill_combo(combo: HWND, labels: &[&str], select: usize) {
    // SAFETY: plain combo box messages; each string lives until its call
    // returns, and the combo box copies it.
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
        send(combo, CB_SETCURSEL, select, 0);
    }
}

fn selection(list: HWND) -> Option<usize> {
    // SAFETY: plain list box message.
    let index = unsafe { send(list, LB_GETCURSEL, 0, 0).0 };
    (index != LB_ERR as isize).then_some(index as usize)
}

fn combo_selection(combo: HWND) -> Option<usize> {
    // SAFETY: plain combo box message.
    let index = unsafe { send(combo, CB_GETCURSEL, 0, 0).0 };
    usize::try_from(index).ok()
}
