//! The rebar at the top of the window, in two rows:
//!
//! ```text
//! [Reload] [Settings]  Search [__________________________]
//! Agent [lab switch  v]  Object [IF-MIB::ifTable______]  [Get] [Next] [Bulk] [Walk] [Table] [Set...] [Cancel] [Agents...] [Raw]
//! ```
//!
//! A rebar is the strip of movable "bands" at the top of classic Windows
//! programs. Each band holds one child window: a toolbar, the search box, the
//! Agent box or the Object box.

use windows::Win32::Foundation::SIZE;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    BTNS_AUTOSIZE, BTNS_BUTTON, BTNS_CHECK, BTNS_SEP, BTNS_SHOWTEXT, CCS_NODIVIDER,
    CCS_NOPARENTALIGN, CCS_NORESIZE, EM_SETCUEBANNER, I_IMAGENONE, RB_INSERTBANDW, RBBIM_CHILD,
    RBBIM_CHILDSIZE, RBBIM_SIZE, RBBIM_STYLE, RBBIM_TEXT, RBBS_BREAK, RBBS_CHILDEDGE,
    RBBS_NOGRIPPER, RBS_BANDBORDERS, RBS_VARHEIGHT, REBARBANDINFOW, REBARCLASSNAMEW,
    TB_ADDBUTTONSW, TB_AUTOSIZE, TB_BUTTONSTRUCTSIZE, TB_CHECKBUTTON, TB_ENABLEBUTTON,
    TB_GETMAXSIZE, TB_SETEXTENDEDSTYLE, TB_SETIMAGELIST, TB_SETPARENT, TBBUTTON, TBSTATE_ENABLED,
    TBSTYLE_EX_MIXEDBUTTONS, TBSTYLE_FLAT, TBSTYLE_LIST, TBSTYLE_TRANSPARENT, TOOLBARCLASSNAMEW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CB_ADDSTRING, CB_RESETCONTENT, CBS_AUTOHSCROLL, CBS_DROPDOWN, CreateWindowExW, DestroyWindow,
    ES_AUTOHSCROLL, GetWindowRect, HMENU, SendMessageW, SetWindowTextW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE,
    WS_EX_CONTROLPARENT, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PWSTR};

use fero_win32::dpi;
use fero_win32::font::Font;
use fero_win32::text::wide;
use fero_win32::window::instance;

use super::{id, send};

/// The rebar and the controls in it.
#[derive(Debug, Clone, Copy)]
pub struct Toolbar {
    /// The rebar itself; the window lays it out.
    pub rebar: HWND,
    /// The search box.
    pub search: HWND,
    /// The Agent box: a combo box listing the saved agents, in which an
    /// address can be typed too.
    pub agent: HWND,
    /// The Object box: what to query, such as `sysDescr.0`.
    pub object: HWND,
    /// The toolbar with Get, Next, Bulk, Walk, Table, Set, Cancel, Agents
    /// and Raw.
    pub queries: HWND,
}

impl Toolbar {
    /// Create the rebar with its bands at `dpi`. Toolbar buttons send
    /// `WM_COMMAND` to `parent`.
    pub fn create(parent: HWND, dpi: u32, font: &Font) -> windows::core::Result<Toolbar> {
        // SAFETY: plain window creation; every structure passed to
        // SendMessageW lives on the stack until the call returns, and the
        // strings they point to live until the end of this function.
        unsafe {
            let rebar = CreateWindowExW(
                // Lets Tab move into the search box inside the rebar.
                WS_EX_CONTROLPARENT,
                REBARCLASSNAMEW,
                None,
                WS_CHILD
                    | WS_VISIBLE
                    | WS_CLIPSIBLINGS
                    | WS_CLIPCHILDREN
                    | WINDOW_STYLE(RBS_VARHEIGHT | RBS_BANDBORDERS)
                    | WINDOW_STYLE(CCS_NODIVIDER as u32),
                0,
                0,
                0,
                0,
                Some(parent),
                Some(HMENU(usize::from(id::REBAR) as *mut _)),
                Some(instance()),
                None,
            )?;
            font.apply(rebar);

            // The toolbar: text-only buttons.
            let toolbar = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                TOOLBARCLASSNAMEW,
                None,
                WS_CHILD
                    | WS_VISIBLE
                    | WINDOW_STYLE(TBSTYLE_FLAT | TBSTYLE_LIST | TBSTYLE_TRANSPARENT)
                    | WINDOW_STYLE((CCS_NODIVIDER | CCS_NORESIZE | CCS_NOPARENTALIGN) as u32),
                0,
                0,
                0,
                0,
                Some(rebar),
                Some(HMENU(usize::from(id::TOOLBAR) as *mut _)),
                Some(instance()),
                None,
            )?;
            font.apply(toolbar);
            send(toolbar, TB_BUTTONSTRUCTSIZE, size_of::<TBBUTTON>(), 0);
            // Send button clicks to the main window, not to the rebar.
            send(toolbar, TB_SETPARENT, parent.0 as usize, 0);
            send(
                toolbar,
                TB_SETEXTENDEDSTYLE,
                0,
                TBSTYLE_EX_MIXEDBUTTONS as isize,
            );
            // No image list: the buttons are text only.
            send(toolbar, TB_SETIMAGELIST, 0, 0);
            let labels = [wide("Reload"), wide("Settings")];
            let buttons = [
                button(id::RELOAD, &labels[0]),
                button(id::SETTINGS, &labels[1]),
            ];
            SendMessageW(
                toolbar,
                TB_ADDBUTTONSW,
                Some(WPARAM(buttons.len())),
                Some(LPARAM(buttons.as_ptr() as isize)),
            );
            send(toolbar, TB_AUTOSIZE, 0, 0);
            let mut size = SIZE::default();
            SendMessageW(
                toolbar,
                TB_GETMAXSIZE,
                None,
                Some(LPARAM(&mut size as *mut SIZE as isize)),
            );
            let mut band = REBARBANDINFOW {
                cbSize: size_of::<REBARBANDINFOW>() as u32,
                fMask: RBBIM_STYLE | RBBIM_CHILD | RBBIM_CHILDSIZE | RBBIM_SIZE,
                fStyle: RBBS_NOGRIPPER,
                hwndChild: toolbar,
                cxMinChild: size.cx as u32,
                cyMinChild: size.cy as u32,
                cx: size.cx as u32 + dpi::scale(8, dpi) as u32,
                ..Default::default()
            };
            SendMessageW(
                rebar,
                RB_INSERTBANDW,
                Some(WPARAM(usize::MAX)), // -1: at the end
                Some(LPARAM(&band as *const _ as isize)),
            );

            // The search box, with a hint shown while it is empty.
            let search = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                windows::core::w!("EDIT"),
                None,
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
                0,
                0,
                0,
                Some(rebar),
                Some(HMENU(usize::from(id::SEARCH) as *mut _)),
                Some(instance()),
                None,
            )?;
            font.apply(search);
            let hint = HSTRING::from("Name, OID or pattern such as if*Octets (Ctrl+F)");
            // TRUE: keep showing the hint while the box has the focus.
            SendMessageW(
                search,
                EM_SETCUEBANNER,
                Some(WPARAM(1)),
                Some(LPARAM(hint.as_ptr() as isize)),
            );
            let mut label = wide("Search");
            band = REBARBANDINFOW {
                cbSize: size_of::<REBARBANDINFOW>() as u32,
                fMask: RBBIM_STYLE | RBBIM_CHILD | RBBIM_CHILDSIZE | RBBIM_SIZE | RBBIM_TEXT,
                fStyle: RBBS_CHILDEDGE | RBBS_NOGRIPPER,
                lpText: PWSTR(label.as_mut_ptr()),
                hwndChild: search,
                cxMinChild: dpi::scale(160, dpi) as u32,
                cyMinChild: (font.line_height(search) + dpi::scale(8, dpi)) as u32,
                cx: dpi::scale(420, dpi) as u32,
                ..Default::default()
            };
            SendMessageW(
                rebar,
                RB_INSERTBANDW,
                Some(WPARAM(usize::MAX)),
                Some(LPARAM(&band as *const _ as isize)),
            );

            // Second row: the Agent box. Its height at creation is the height
            // of the open drop-down list; the closed box sizes itself to the
            // font.
            let agent = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("COMBOBOX"),
                None,
                WS_CHILD
                    | WS_VISIBLE
                    | WS_TABSTOP
                    | WS_VSCROLL
                    | WINDOW_STYLE((CBS_DROPDOWN | CBS_AUTOHSCROLL) as u32),
                0,
                0,
                dpi::scale(200, dpi),
                dpi::scale(240, dpi),
                Some(rebar),
                Some(HMENU(usize::from(id::AGENT) as *mut _)),
                Some(instance()),
                None,
            )?;
            font.apply(agent);
            let mut closed = RECT::default();
            let _ = GetWindowRect(agent, &mut closed);
            let row_height = (closed.bottom - closed.top).max(font.line_height(agent));
            let mut label = wide("Agent");
            band = REBARBANDINFOW {
                cbSize: size_of::<REBARBANDINFOW>() as u32,
                fMask: RBBIM_STYLE | RBBIM_CHILD | RBBIM_CHILDSIZE | RBBIM_SIZE | RBBIM_TEXT,
                fStyle: RBBS_BREAK | RBBS_CHILDEDGE | RBBS_NOGRIPPER,
                lpText: PWSTR(label.as_mut_ptr()),
                hwndChild: agent,
                cxMinChild: dpi::scale(120, dpi) as u32,
                cyMinChild: row_height as u32,
                cx: dpi::scale(230, dpi) as u32,
                ..Default::default()
            };
            SendMessageW(
                rebar,
                RB_INSERTBANDW,
                Some(WPARAM(usize::MAX)),
                Some(LPARAM(&band as *const _ as isize)),
            );

            // The Object box.
            let object = CreateWindowExW(
                WS_EX_CLIENTEDGE,
                windows::core::w!("EDIT"),
                None,
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
                0,
                0,
                0,
                Some(rebar),
                Some(HMENU(usize::from(id::OBJECT) as *mut _)),
                Some(instance()),
                None,
            )?;
            font.apply(object);
            let hint = HSTRING::from("sysDescr.0, IF-MIB::ifTable or 1.3.6.1.2.1");
            SendMessageW(
                object,
                EM_SETCUEBANNER,
                Some(WPARAM(1)),
                Some(LPARAM(hint.as_ptr() as isize)),
            );
            let mut label = wide("Object");
            band = REBARBANDINFOW {
                cbSize: size_of::<REBARBANDINFOW>() as u32,
                fMask: RBBIM_STYLE | RBBIM_CHILD | RBBIM_CHILDSIZE | RBBIM_SIZE | RBBIM_TEXT,
                fStyle: RBBS_CHILDEDGE | RBBS_NOGRIPPER,
                lpText: PWSTR(label.as_mut_ptr()),
                hwndChild: object,
                cxMinChild: dpi::scale(140, dpi) as u32,
                cyMinChild: row_height as u32,
                cx: dpi::scale(300, dpi) as u32,
                ..Default::default()
            };
            SendMessageW(
                rebar,
                RB_INSERTBANDW,
                Some(WPARAM(usize::MAX)),
                Some(LPARAM(&band as *const _ as isize)),
            );

            // The query buttons.
            let queries = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                TOOLBARCLASSNAMEW,
                None,
                WS_CHILD
                    | WS_VISIBLE
                    | WINDOW_STYLE(TBSTYLE_FLAT | TBSTYLE_LIST | TBSTYLE_TRANSPARENT)
                    | WINDOW_STYLE((CCS_NODIVIDER | CCS_NORESIZE | CCS_NOPARENTALIGN) as u32),
                0,
                0,
                0,
                0,
                Some(rebar),
                Some(HMENU(usize::from(id::QUERY_TOOLBAR) as *mut _)),
                Some(instance()),
                None,
            )?;
            font.apply(queries);
            send(queries, TB_BUTTONSTRUCTSIZE, size_of::<TBBUTTON>(), 0);
            send(queries, TB_SETPARENT, parent.0 as usize, 0);
            send(
                queries,
                TB_SETEXTENDEDSTYLE,
                0,
                TBSTYLE_EX_MIXEDBUTTONS as isize,
            );
            send(queries, TB_SETIMAGELIST, 0, 0);
            let labels = [
                wide("Get"),
                wide("Next"),
                wide("Bulk"),
                wide("Walk"),
                wide("Table"),
                wide("Set\u{2026}"),
                wide("Cancel"),
                wide("Agents\u{2026}"),
                wide("Raw"),
            ];
            let mut cancel = button(id::CANCEL, &labels[6]);
            cancel.fsState = 0; // disabled until a query runs
            let mut raw = button(id::RAW, &labels[8]);
            raw.fsStyle |= BTNS_CHECK as u8;
            let buttons = [
                button(id::GET, &labels[0]),
                button(id::NEXT, &labels[1]),
                button(id::BULK, &labels[2]),
                button(id::WALK, &labels[3]),
                button(id::TABLE_QUERY, &labels[4]),
                separator(),
                button(id::SET_VALUE, &labels[5]),
                cancel,
                separator(),
                button(id::AGENTS, &labels[7]),
                raw,
            ];
            SendMessageW(
                queries,
                TB_ADDBUTTONSW,
                Some(WPARAM(buttons.len())),
                Some(LPARAM(buttons.as_ptr() as isize)),
            );
            send(queries, TB_AUTOSIZE, 0, 0);
            let mut size = SIZE::default();
            SendMessageW(
                queries,
                TB_GETMAXSIZE,
                None,
                Some(LPARAM(&mut size as *mut SIZE as isize)),
            );
            band = REBARBANDINFOW {
                cbSize: size_of::<REBARBANDINFOW>() as u32,
                fMask: RBBIM_STYLE | RBBIM_CHILD | RBBIM_CHILDSIZE | RBBIM_SIZE,
                fStyle: RBBS_NOGRIPPER,
                hwndChild: queries,
                cxMinChild: size.cx as u32,
                cyMinChild: size.cy.max(row_height) as u32,
                cx: size.cx as u32 + dpi::scale(8, dpi) as u32,
                ..Default::default()
            };
            SendMessageW(
                rebar,
                RB_INSERTBANDW,
                Some(WPARAM(usize::MAX)),
                Some(LPARAM(&band as *const _ as isize)),
            );
            Ok(Toolbar {
                rebar,
                search,
                agent,
                object,
                queries,
            })
        }
    }

    /// Fill the Agent box's list with the saved agents' names and set its
    /// text.
    pub fn set_agents(&self, names: &[&str], text: &str) {
        // SAFETY: plain combo box messages; each string lives until its call
        // returns, and the combo box copies it.
        unsafe {
            send(self.agent, CB_RESETCONTENT, 0, 0);
            for name in names {
                let name = HSTRING::from(*name);
                SendMessageW(
                    self.agent,
                    CB_ADDSTRING,
                    None,
                    Some(LPARAM(name.as_ptr() as isize)),
                );
            }
            let _ = SetWindowTextW(self.agent, &HSTRING::from(text));
        }
    }

    /// Enable Cancel while a query runs.
    pub fn set_running(&self, running: bool) {
        // SAFETY: TB_ENABLEBUTTON takes a command id and TRUE or FALSE.
        unsafe {
            send(
                self.queries,
                TB_ENABLEBUTTON,
                usize::from(id::CANCEL),
                isize::from(running),
            );
        }
    }

    /// Show the Raw button pressed in the raw view.
    pub fn set_raw(&self, raw: bool) {
        // SAFETY: TB_CHECKBUTTON takes a command id and TRUE or FALSE.
        unsafe {
            send(
                self.queries,
                TB_CHECKBUTTON,
                usize::from(id::RAW),
                isize::from(raw),
            );
        }
    }

    /// Destroy the rebar and everything in it.
    pub fn destroy(self) {
        // SAFETY: the rebar is our own child window.
        unsafe {
            let _ = DestroyWindow(self.rebar);
        }
    }
}

fn separator() -> TBBUTTON {
    TBBUTTON {
        iBitmap: 6, // width in pixels
        fsStyle: BTNS_SEP as u8,
        ..Default::default()
    }
}

fn button(command: u16, label: &[u16]) -> TBBUTTON {
    TBBUTTON {
        iBitmap: I_IMAGENONE,
        idCommand: i32::from(command),
        fsState: TBSTATE_ENABLED as u8,
        fsStyle: (BTNS_BUTTON | BTNS_AUTOSIZE | BTNS_SHOWTEXT) as u8,
        iString: label.as_ptr() as isize,
        ..Default::default()
    }
}
