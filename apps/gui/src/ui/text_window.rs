//! A window that shows read-only text: the source viewer, which shows a MIB
//! file with the place of a diagnostic highlighted, and the "Why this
//! symbol?" report.
//!
//! ```text
//! +---------------------------------------------------+
//! | error: unresolved import ...      [Why this ...?] |  header
//! | Hint: change the IMPORTS to ...                   |
//! +---------------------------------------------------+
//! | IMPORTS                                           |  rich edit,
//! |     HintLevel                                     |  read-only
//! |         FROM SNMPv2-TC;                           |
//! +---------------------------------------------------+
//! ```
//!
//! The window belongs to the main window, so it stays in front of it and
//! closes with it. Closing it only hides it, so it can be shown again.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Once;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, GetSysColor, GetSysColorBrush, HDC, SetBkColor};
use windows::Win32::System::LibraryLoader::LoadLibraryW;
use windows::Win32::UI::Controls::RichEdit::{
    CHARRANGE, EM_EXLIMITTEXT, EM_EXLINEFROMCHAR, EM_EXSETSEL, EM_SETTARGETDEVICE, EM_SETTEXTEX,
    MSFTEDIT_CLASS, SETTEXTEX, ST_DEFAULT,
};
use windows::Win32::UI::Controls::{EM_GETFIRSTVISIBLELINE, EM_LINESCROLL};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CreateWindowExW, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_NOHIDESEL,
    ES_READONLY, GetClientRect, HMENU, IsWindowVisible, PostMessageW, SW_HIDE, SW_SHOW,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, SetWindowTextW, ShowWindow,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORSTATIC,
    WM_DPICHANGED, WM_SETTINGCHANGE, WM_SIZE, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE,
    WS_HSCROLL, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, w};

use fero_win32::dpi;
use fero_win32::font::Font;
use fero_win32::text::crlf;
use fero_win32::window::{self, CreateOptions, Window, instance};
use fero_win32::{hiword, loword, theme};

use super::{error_box, id, send};

/// Sizes at 96 DPI.
const WINDOW_SIZE: (i32, i32) = (860, 620);
const MARGIN: i32 = 6;
const BUTTON_SIZE: (i32, i32) = (130, 26);
/// Lines of header text shown.
const HEADER_LINES: i32 = 5;

/// Control ids inside the window.
const HEADER: u16 = 1;
const TEXT: u16 = 2;
const WHY: u16 = 3;

/// What to show in a [`TextWindow`].
#[derive(Debug, Clone, Default)]
pub struct Content {
    /// The window title.
    pub title: String,
    /// The text above the source: the message, location and hint.
    pub header: String,
    /// The body, as RTF (see [`crate::rich_text`]).
    pub rtf: String,
    /// Where to put the caret and scroll to, in rich edit positions.
    pub caret: Option<u32>,
}

/// A read-only text window.
pub struct TextWindow {
    hwnd: Cell<HWND>,
    owner: HWND,
    header: Cell<HWND>,
    text: Cell<HWND>,
    /// "Why this symbol?", or none for the report window itself.
    why: Cell<Option<HWND>>,
    with_why: bool,
    dpi: Cell<u32>,
    font: RefCell<Option<Font>>,
}

impl TextWindow {
    /// Create the (hidden) window. With `with_why`, it has a "Why this
    /// symbol?" button that sends `id::WHY` to the owner.
    pub fn create(
        owner: HWND,
        class: &str,
        with_why: bool,
    ) -> windows::core::Result<Rc<TextWindow>> {
        load_rich_edit();
        let dpi = dpi::for_window(owner);
        let window = Rc::new(TextWindow {
            hwnd: Cell::new(HWND::default()),
            owner,
            header: Cell::new(HWND::default()),
            text: Cell::new(HWND::default()),
            why: Cell::new(None),
            with_why,
            dpi: Cell::new(dpi),
            font: RefCell::new(None),
        });
        let class = HSTRING::from(class);
        window::register_class(&class, None)?;
        let options = CreateOptions {
            style: WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            ex_style: WINDOW_EX_STYLE(0),
            position: None,
            size: Some((
                dpi::scale(WINDOW_SIZE.0, dpi),
                dpi::scale(WINDOW_SIZE.1, dpi),
            )),
            parent: Some(owner),
            menu: None,
        };
        let hwnd = window::create(&class, "FeroSNMP", &options, window.clone())?;
        window.hwnd.set(hwnd);
        Ok(window)
    }

    /// True while the window is on screen.
    pub fn is_visible(&self) -> bool {
        // SAFETY: IsWindowVisible has no preconditions.
        unsafe { IsWindowVisible(self.hwnd.get()).as_bool() }
    }

    /// Show `content`. With `activate` false, the window is shown without
    /// taking the keyboard focus, so the user can go on moving through the
    /// diagnostics list.
    pub fn show(&self, content: &Content, activate: bool) {
        let text = self.text.get();
        // SAFETY: plain messages to our own controls; every structure and
        // string lives until its call returns.
        unsafe {
            let _ = SetWindowTextW(self.hwnd.get(), &HSTRING::from(content.title.as_str()));
            let _ = SetWindowTextW(self.header.get(), &HSTRING::from(crlf(&content.header)));
            let mut rtf = content.rtf.clone().into_bytes();
            rtf.push(0);
            let options = SETTEXTEX {
                flags: ST_DEFAULT,
                codepage: 0, // the RTF is plain ASCII
            };
            send(
                text,
                EM_SETTEXTEX,
                &options as *const _ as usize,
                rtf.as_ptr() as isize,
            );
            let caret = content.caret.unwrap_or(0) as i32;
            let range = CHARRANGE {
                cpMin: caret,
                cpMax: caret,
            };
            send(text, EM_EXSETSEL, 0, &range as *const _ as isize);
            // Scroll so the caret line is a few lines from the top.
            let line = send(text, EM_EXLINEFROMCHAR, 0, caret as isize).0;
            let first = send(text, EM_GETFIRSTVISIBLELINE, 0, 0).0;
            send(text, EM_LINESCROLL, 0, line - first - 5);

            let _ = ShowWindow(
                self.hwnd.get(),
                if activate { SW_SHOW } else { SW_SHOWNOACTIVATE },
            );
            if activate {
                let _ = SetFocus(Some(text));
            }
        }
    }

    /// Enable or disable the "Why this symbol?" button.
    pub fn set_why_enabled(&self, enabled: bool) {
        if let Some(why) = self.why.get() {
            // SAFETY: enabling our own control.
            unsafe {
                let _ = EnableWindow(why, enabled);
            }
        }
    }

    fn on_create(&self, hwnd: HWND) -> windows::core::Result<()> {
        self.hwnd.set(hwnd);
        theme::apply_title_bar(hwnd);
        let child = |class, text: Option<&str>, style: WINDOW_STYLE, ex_style, id: u16| {
            let text = text.map(HSTRING::from);
            // SAFETY: creates a child control of our window.
            unsafe {
                CreateWindowExW(
                    ex_style,
                    class,
                    text.as_ref().map_or(windows::core::PCWSTR::null(), |t| {
                        windows::core::PCWSTR(t.as_ptr())
                    }),
                    WS_CHILD | WS_VISIBLE | style,
                    0,
                    0,
                    0,
                    0,
                    Some(hwnd),
                    Some(HMENU(usize::from(id) as *mut _)),
                    Some(instance()),
                    None,
                )
            }
        };
        let header = child(
            w!("EDIT"),
            None,
            WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
            WINDOW_EX_STYLE(0),
            HEADER,
        )?;
        let text = child(
            MSFTEDIT_CLASS,
            None,
            WS_TABSTOP
                | WS_VSCROLL
                | WS_HSCROLL
                | WINDOW_STYLE(
                    (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_NOHIDESEL)
                        as u32,
                ),
            WS_EX_CLIENTEDGE,
            TEXT,
        )?;
        if self.with_why {
            let why = child(
                w!("BUTTON"),
                Some("&Why this symbol?"),
                WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                WINDOW_EX_STYLE(0),
                WHY,
            )?;
            self.why.set(Some(why));
        }
        // SAFETY: plain control messages.
        unsafe {
            // No limit on the text length (the default is 32K characters,
            // less than some vendor MIBs) and no word wrap.
            send(text, EM_EXLIMITTEXT, 0, 0x7FFF_FFFE);
            send(text, EM_SETTARGETDEVICE, 0, 1);
        }
        self.header.set(header);
        self.text.set(text);
        self.apply_font()
    }

    fn apply_font(&self) -> windows::core::Result<()> {
        let font = Font::message(self.dpi.get())?;
        font.apply(self.header.get());
        if let Some(why) = self.why.get() {
            font.apply(why);
        }
        *self.font.borrow_mut() = Some(font);
        Ok(())
    }

    fn layout(&self) {
        let dpi = self.dpi.get();
        let mut client = RECT::default();
        // SAFETY: reading our own client rectangle.
        unsafe {
            let _ = GetClientRect(self.hwnd.get(), &mut client);
        }
        let margin = dpi::scale(MARGIN, dpi);
        let line = self
            .font
            .borrow()
            .as_ref()
            .map_or(dpi::scale(16, dpi), |font| {
                font.line_height(self.header.get())
            });
        let header_height = line * HEADER_LINES + margin / 2;
        let (button_width, button_height) = (
            dpi::scale(BUTTON_SIZE.0, dpi),
            dpi::scale(BUTTON_SIZE.1, dpi),
        );
        let button_space = if self.why.get().is_some() {
            button_width + margin
        } else {
            0
        };
        let place = |hwnd: HWND, x: i32, y: i32, width: i32, height: i32| {
            // SAFETY: moving our own control.
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    width.max(0),
                    height.max(0),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        };
        place(
            self.header.get(),
            margin,
            margin,
            client.right - 2 * margin - button_space,
            header_height,
        );
        if let Some(why) = self.why.get() {
            place(
                why,
                client.right - margin - button_width,
                margin,
                button_width,
                button_height,
            );
        }
        let top = margin + header_height + margin;
        place(self.text.get(), 0, top, client.right, client.bottom - top);
    }
}

impl Window for TextWindow {
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_CREATE => Some(LRESULT(match self.on_create(hwnd) {
                Ok(()) => 0,
                Err(error) => {
                    error_box(
                        Some(self.owner),
                        &format!("FeroSNMP could not open the text window.\n\n{error}"),
                    );
                    -1
                }
            })),
            WM_SIZE => {
                self.layout();
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                self.dpi.set(u32::from(hiword(wparam.0)));
                let _ = self.apply_font();
                // SAFETY: for this message, lparam points to the suggested RECT.
                let suggested = unsafe { *(lparam.0 as *const RECT) };
                // SAFETY: moving our own window where Windows suggests.
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        suggested.left,
                        suggested.top,
                        suggested.right - suggested.left,
                        suggested.bottom - suggested.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                self.layout();
                Some(LRESULT(0))
            }
            WM_COMMAND if loword(wparam.0) == WHY => {
                // SAFETY: posting a command to the main window, which owns us.
                unsafe {
                    let _ = PostMessageW(
                        Some(self.owner),
                        WM_COMMAND,
                        WPARAM(usize::from(id::WHY)),
                        LPARAM(0),
                    );
                }
                Some(LRESULT(0))
            }
            WM_CTLCOLORSTATIC if HWND(lparam.0 as *mut _) == self.header.get() => {
                // The read-only header reads better on the window background.
                // SAFETY: wparam is the control's device context.
                unsafe {
                    SetBkColor(
                        HDC(wparam.0 as *mut _),
                        windows::Win32::Foundation::COLORREF(GetSysColor(COLOR_WINDOW)),
                    );
                    Some(LRESULT(GetSysColorBrush(COLOR_WINDOW).0 as isize))
                }
            }
            WM_SETTINGCHANGE => {
                if theme::is_theme_change(lparam) {
                    theme::apply_title_bar(hwnd);
                }
                None
            }
            WM_CLOSE => {
                // Only hide: the main window shows it again for the next
                // diagnostic.
                // SAFETY: hiding our own window.
                unsafe {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

/// The rich edit control lives in Msftedit.dll, which must be loaded before
/// its window class can be used.
fn load_rich_edit() {
    static LOAD: Once = Once::new();
    LOAD.call_once(|| {
        // SAFETY: loading a system DLL by name; it stays loaded for the life
        // of the program.
        unsafe {
            let _ = LoadLibraryW(w!("Msftedit.dll"));
        }
    });
}
