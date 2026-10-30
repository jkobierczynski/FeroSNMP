//! Modal dialogs described in code instead of a resource script.
//!
//! A Win32 dialog is normally drawn in a resource editor and compiled into
//! the program with a resource compiler. [`DialogTemplate`] builds the same
//! binary description (a `DLGTEMPLATEEX`) in memory, so no resource compiler
//! is needed and the layout sits next to the code that uses it.
//!
//! Positions and sizes are in *dialog units*, which follow the dialog's font:
//! a dialog unit is a quarter of the average character width horizontally and
//! an eighth of the character height vertically. Windows also rescales the
//! dialog when it moves between monitors with different DPI, so dialogs need
//! no DPI code of their own.
//!
//! [`tabbed_dialog`] shows several templates made with
//! [`DialogTemplate::page`] as the tabs of one Settings-style dialog.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    EnableThemeDialogTexture, NMHDR, PSN_APPLY, PSN_KILLACTIVE, TCIF_TEXT, TCITEMW, TCM_ADJUSTRECT,
    TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL, TCN_SELCHANGE, TCN_SELCHANGING, WC_TABCONTROLW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CreateDialogIndirectParamW, CreateWindowExW, DLGTEMPLATE,
    DS_MODALFRAME, DS_SETFONT, DWLP_MSGRESULT, DialogBoxIndirectParamW, EndDialog, GWLP_USERDATA,
    GetWindowLongPtrW, HMENU, MapDialogRect, SW_HIDE, SW_SHOW, SWP_NOZORDER, SendMessageW,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_LONG_PTR_INDEX,
    WINDOW_STYLE, WM_COMMAND, WM_GETFONT, WM_INITDIALOG, WM_NOTIFY, WM_SETFONT, WS_CAPTION,
    WS_CHILD, WS_CLIPSIBLINGS, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::PWSTR;

use crate::window::instance;

/// The predefined control classes, which a template can name by number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlClass {
    /// Push buttons, check boxes, radio buttons and group boxes.
    Button,
    /// Text boxes.
    Edit,
    /// Labels.
    Static,
    /// List boxes.
    ListBox,
    /// Combo boxes and drop-down lists.
    ComboBox,
}

impl ControlClass {
    fn ordinal(self) -> u16 {
        match self {
            ControlClass::Button => 0x0080,
            ControlClass::Edit => 0x0081,
            ControlClass::Static => 0x0082,
            ControlClass::ListBox => 0x0083,
            ControlClass::ComboBox => 0x0085,
        }
    }
}

/// A rectangle in dialog units: x, y, width, height.
pub type DialogRect = (i16, i16, i16, i16);

/// An in-memory dialog template. Build it with [`new`](Self::new) and
/// [`control`](Self::control), then show it with [`run_modal`](Self::run_modal).
#[derive(Debug, Clone)]
pub struct DialogTemplate {
    words: Vec<u16>,
    /// Where the control count is stored in `words`.
    count_at: usize,
}

impl DialogTemplate {
    /// A dialog with a title bar and close button, `width` by `height` dialog
    /// units, in the given font (`"Segoe UI"`, 9 points matches Windows).
    pub fn new(title: &str, width: i16, height: i16, font: &str, points: u16) -> Self {
        let style = WS_POPUP
            | WS_CAPTION
            | WS_SYSMENU
            | WINDOW_STYLE(DS_MODALFRAME as u32)
            | WINDOW_STYLE(DS_SETFONT as u32);
        Self::with_style(style, title, width, height, font, points)
    }

    /// A page of a [`tabbed_dialog`]: a child dialog without a frame.
    pub fn page(width: i16, height: i16, font: &str, points: u16) -> Self {
        let style = WS_CHILD | WINDOW_STYLE((DS_SETFONT | DS_CONTROL) as u32);
        Self::with_style(style, "", width, height, font, points)
    }

    /// Width and height in dialog units.
    fn size(&self) -> (i16, i16) {
        // After dlgVer, signature, helpID (2), exStyle (2), style (2) and
        // the control count come x, y, cx and cy.
        (
            self.words[self.count_at + 3] as i16,
            self.words[self.count_at + 4] as i16,
        )
    }

    fn with_style(
        style: WINDOW_STYLE,
        title: &str,
        width: i16,
        height: i16,
        font: &str,
        points: u16,
    ) -> Self {
        let mut template = DialogTemplate {
            words: Vec::new(),
            count_at: 0,
        };
        template.push(1); // dlgVer
        template.push(0xFFFF); // signature: this is a DLGTEMPLATEEX
        template.push_u32(0); // helpID
        template.push_u32(0); // exStyle
        template.push_u32(style.0);
        template.count_at = template.words.len();
        template.push(0); // number of controls, filled in by `control`
        for value in [0, 0, width, height] {
            template.push(value as u16);
        }
        template.push(0); // no menu
        template.push(0); // the standard dialog class
        template.push_str(title);
        template.push(points);
        template.push(400); // weight: normal
        template.push(0x0100); // italic: no; character set: DEFAULT_CHARSET
        template.push_str(font);
        template
    }

    /// Add a control. `WS_CHILD | WS_VISIBLE` are added to `style`.
    pub fn control(
        mut self,
        class: ControlClass,
        text: &str,
        id: u16,
        style: WINDOW_STYLE,
        ex_style: WINDOW_EX_STYLE,
        rect: DialogRect,
    ) -> Self {
        // Every control starts on a 4-byte boundary.
        if self.words.len() % 2 == 1 {
            self.push(0);
        }
        self.push_u32(0); // helpID
        self.push_u32(ex_style.0);
        self.push_u32((style | WS_CHILD | WS_VISIBLE).0);
        let (x, y, width, height) = rect;
        for value in [x, y, width, height] {
            self.push(value as u16);
        }
        self.push_u32(u32::from(id));
        self.push(0xFFFF); // the class is given by number
        self.push(class.ordinal());
        self.push_str(text);
        self.push(0); // no creation data
        self.words[self.count_at] += 1;
        self
    }

    /// Show the dialog and wait until it is closed with `EndDialog`. Returns
    /// the value passed to `EndDialog`.
    pub fn run_modal(
        &self,
        owner: Option<HWND>,
        handler: &dyn Dialog,
    ) -> windows::core::Result<isize> {
        let aligned = self.aligned();
        // The handler is a wide pointer (`&dyn`), so pass a pointer to it.
        let handler_ref: &dyn Dialog = handler;
        let handler_ptr = Cell::new(&handler_ref as *const &dyn Dialog);
        // SAFETY: the template is a valid DLGTEMPLATEEX, and `handler_ref`
        // outlives the modal loop, which only returns after the dialog is gone.
        let result = unsafe {
            DialogBoxIndirectParamW(
                Some(instance()),
                aligned.as_ptr() as *const DLGTEMPLATE,
                owner,
                Some(dialog_proc),
                LPARAM(handler_ptr.get() as isize),
            )
        };
        if result == -1 {
            Err(windows::core::Error::from_thread())
        } else {
            Ok(result)
        }
    }

    /// The template on a 4-byte boundary, as Windows needs it; a Vec<u16>
    /// only guarantees 2, so it is copied into u32s.
    fn aligned(&self) -> Vec<u32> {
        let mut aligned = vec![0u32; self.words.len().div_ceil(2)];
        for (i, word) in self.words.iter().enumerate() {
            aligned[i / 2] |= u32::from(*word) << (16 * (i % 2));
        }
        aligned
    }

    fn push(&mut self, word: u16) {
        self.words.push(word);
    }

    fn push_u32(&mut self, value: u32) {
        self.push((value & 0xFFFF) as u16);
        self.push((value >> 16) as u16);
    }

    fn push_str(&mut self, text: &str) {
        self.words.extend(text.encode_utf16());
        self.push(0);
    }
}

/// A Rust value that handles the messages of a modal dialog.
pub trait Dialog {
    /// Handle one message. Return `Some(value)` if it was handled: usually
    /// `Some(1)` (TRUE), or for `WM_INITDIALOG`, `Some(0)` if the handler set
    /// the focus itself. Return `None` to let the dialog manager handle it.
    ///
    /// Like [`Window::message`](crate::window::Window::message), this can be
    /// re-entered, so it takes `&self`.
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize>;
}

/// Set the value a dialog returns for the message being handled, for
/// notifications that want more than TRUE or FALSE: return `Some(1)` after
/// calling it. A page of a [`tabbed_dialog`] refuses to be left
/// (`PSN_KILLACTIVE`) with `set_result(page, 1)`.
pub fn set_result(hwnd: HWND, value: isize) {
    // SAFETY: DWLP_MSGRESULT is the dialog's own result slot.
    unsafe {
        SetWindowLongPtrW(hwnd, WINDOW_LONG_PTR_INDEX(DWLP_MSGRESULT as i32), value);
    }
}

/// `DS_CONTROL`: a dialog that lives inside another one, whose controls the
/// Tab key reaches (not in the windows crate).
const DS_CONTROL: i32 = 0x0400;
/// `ETDT_ENABLETAB`: give a page the background of the tab control under it.
const ETDT_ENABLETAB: u32 = 6;
/// Ids inside a tabbed dialog.
const TAB_CONTROL: u16 = 0x7F00;
const OK: u16 = 1;
const CANCEL: u16 = 2;

/// Show `pages` (made with [`DialogTemplate::page`], each with the title of
/// its tab) as the tabs of a modal dialog with OK and Cancel, opening on
/// page `start`. Returns true if OK was pressed.
///
/// It works like a property sheet, but every page keeps the font of its
/// template (a property sheet sizes its pages with its own, older font, so
/// pages in Segoe UI are cut off). As in a property sheet, each page's
/// handler gets `WM_NOTIFY` with `PSN_KILLACTIVE` when the user leaves it
/// for another tab or presses OK, which it refuses with [`set_result`]`(page,
/// 1)`, and `PSN_APPLY` when OK is pressed. Every page is created at the
/// start, so every page gets `PSN_APPLY`.
pub fn tabbed_dialog(
    owner: Option<HWND>,
    title: &str,
    pages: &[(&str, &DialogTemplate, &dyn Dialog)],
    start: usize,
) -> windows::core::Result<bool> {
    // Room for the largest page, the tabs above it and the buttons below.
    let (width, height) = pages.iter().fold((0, 0), |(w, h), (_, template, _)| {
        let (pw, ph) = template.size();
        (w.max(pw), h.max(ph))
    });
    let (dialog_width, dialog_height) = (width + 22, height + 56);
    let button = WINDOW_STYLE(BS_PUSHBUTTON as u32) | WS_TABSTOP;
    let template = DialogTemplate::new(title, dialog_width, dialog_height, "Segoe UI", 9)
        .control(
            ControlClass::Button,
            "OK",
            OK,
            WINDOW_STYLE(BS_DEFPUSHBUTTON as u32) | WS_TABSTOP,
            WINDOW_EX_STYLE(0),
            (dialog_width - 116, dialog_height - 21, 52, 14),
        )
        .control(
            ControlClass::Button,
            "Cancel",
            CANCEL,
            button,
            WINDOW_EX_STYLE(0),
            (dialog_width - 61, dialog_height - 21, 54, 14),
        );
    let dialog = Tabbed {
        pages: pages
            .iter()
            .map(|(title, template, handler)| (title.to_string(), template.aligned(), *handler))
            .collect(),
        windows: RefCell::new(Vec::new()),
        tabs: Cell::new(HWND::default()),
        start: start.min(pages.len().saturating_sub(1)),
        tabs_height: dialog_height - 34,
    };
    Ok(template.run_modal(owner, &dialog)? == 1)
}

/// The dialog around the pages of a [`tabbed_dialog`].
struct Tabbed<'a> {
    /// Each page's title, template and handler. The handler pointers given
    /// to the pages point into this Vec.
    pages: Vec<(String, Vec<u32>, &'a dyn Dialog)>,
    /// The pages' windows, once created.
    windows: RefCell<Vec<HWND>>,
    tabs: Cell<HWND>,
    start: usize,
    /// Height of the tab control, in dialog units.
    tabs_height: i16,
}

impl Dialog for Tabbed<'_> {
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
        match msg {
            WM_INITDIALOG => {
                self.init(hwnd);
                Some(1)
            }
            WM_NOTIFY => {
                // SAFETY: for WM_NOTIFY, lparam points to an NMHDR.
                let header = unsafe { &*(lparam.0 as *const NMHDR) };
                if header.hwndFrom != self.tabs.get() {
                    return None;
                }
                match header.code {
                    // TRUE stops the change when the page refuses.
                    TCN_SELCHANGING => {
                        let refused = !self.leave(self.current());
                        set_result(hwnd, isize::from(refused));
                        Some(1)
                    }
                    TCN_SELCHANGE => {
                        self.show(self.current());
                        Some(1)
                    }
                    _ => None,
                }
            }
            WM_COMMAND => match crate::loword(wparam.0) {
                OK => {
                    let current = self.current();
                    // The page shown first, as a property sheet does.
                    let order = std::iter::once(current)
                        .chain((0..self.pages.len()).filter(|&page| page != current));
                    for page in order {
                        if !self.leave(page) {
                            self.select(page);
                            return Some(1);
                        }
                    }
                    for page in 0..self.pages.len() {
                        self.notify(page, PSN_APPLY);
                    }
                    // SAFETY: ends the modal loop of our own dialog.
                    unsafe {
                        let _ = EndDialog(hwnd, 1);
                    }
                    Some(1)
                }
                CANCEL => {
                    // SAFETY: ends the modal loop of our own dialog.
                    unsafe {
                        let _ = EndDialog(hwnd, 0);
                    }
                    Some(1)
                }
                _ => None,
            },
            _ => None,
        }
    }
}

impl Tabbed<'_> {
    fn init(&self, hwnd: HWND) {
        // The tab control fills the dialog above the buttons.
        let mut area = RECT {
            left: 7,
            top: 7,
            right: 7,
            bottom: 7 + i32::from(self.tabs_height),
        };
        let mut client = RECT::default();
        // SAFETY: MapDialogRect and GetClientRect fill the RECTs given.
        unsafe {
            let _ = MapDialogRect(hwnd, &mut area);
            let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut client);
        }
        let right = client.right - area.left;
        // SAFETY: creates a child control of our dialog.
        let tabs = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                WC_TABCONTROLW,
                None,
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS,
                area.left,
                area.top,
                right - area.left,
                area.bottom - area.top,
                Some(hwnd),
                Some(HMENU(usize::from(TAB_CONTROL) as *mut _)),
                Some(instance()),
                None,
            )
        };
        let Ok(tabs) = tabs else {
            return;
        };
        self.tabs.set(tabs);
        // SAFETY: plain control messages; the tab texts live until each
        // call returns, and the font belongs to the dialog.
        unsafe {
            let font = SendMessageW(hwnd, WM_GETFONT, None, None);
            SendMessageW(
                tabs,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(0)),
            );
            for (index, (title, _, _)) in self.pages.iter().enumerate() {
                let mut text: Vec<u16> = title.encode_utf16().chain([0]).collect();
                let item = TCITEMW {
                    mask: TCIF_TEXT,
                    pszText: PWSTR(text.as_mut_ptr()),
                    ..Default::default()
                };
                SendMessageW(
                    tabs,
                    TCM_INSERTITEMW,
                    Some(WPARAM(index)),
                    Some(LPARAM(&item as *const _ as isize)),
                );
            }
        }
        // Where the pages go: the tab control's page area, in the dialog's
        // coordinates (the tab control sits at area.left, area.top).
        let mut page_area = RECT {
            left: 0,
            top: 0,
            right: right - area.left,
            bottom: area.bottom - area.top,
        };
        // SAFETY: TCM_ADJUSTRECT turns the window rectangle into the page
        // rectangle in place.
        unsafe {
            SendMessageW(
                tabs,
                TCM_ADJUSTRECT,
                Some(WPARAM(0)),
                Some(LPARAM(&mut page_area as *mut _ as isize)),
            );
        }
        let mut windows = Vec::new();
        for (_, template, handler) in &self.pages {
            // SAFETY: the template is a valid DLGTEMPLATEEX; the handler
            // pointer points into `self.pages`, which outlives the dialog
            // and so its pages.
            let page = unsafe {
                CreateDialogIndirectParamW(
                    Some(instance()),
                    template.as_ptr() as *const DLGTEMPLATE,
                    Some(hwnd),
                    Some(dialog_proc),
                    LPARAM(handler as *const &dyn Dialog as isize),
                )
            };
            let Ok(page) = page else {
                continue;
            };
            // SAFETY: placing our own page and giving it the tab's
            // background; without visual styles that call fails harmlessly.
            unsafe {
                let _ = EnableThemeDialogTexture(page, ETDT_ENABLETAB);
                let _ = SetWindowPos(
                    page,
                    None,
                    area.left + page_area.left,
                    area.top + page_area.top,
                    page_area.right - page_area.left,
                    page_area.bottom - page_area.top,
                    SWP_NOZORDER,
                );
            }
            windows.push(page);
        }
        *self.windows.borrow_mut() = windows;
        self.select(self.start);
    }

    /// The page whose tab is selected.
    fn current(&self) -> usize {
        // SAFETY: TCM_GETCURSEL takes no parameters.
        let index = unsafe { SendMessageW(self.tabs.get(), TCM_GETCURSEL, None, None).0 };
        usize::try_from(index).unwrap_or(0)
    }

    /// Select a tab and show its page.
    fn select(&self, page: usize) {
        // SAFETY: TCM_SETCURSEL takes an index.
        unsafe {
            SendMessageW(self.tabs.get(), TCM_SETCURSEL, Some(WPARAM(page)), None);
        }
        self.show(page);
    }

    /// Show one page and hide the others.
    fn show(&self, page: usize) {
        for (index, &window) in self.windows.borrow().iter().enumerate() {
            // SAFETY: showing or hiding our own page.
            unsafe {
                let _ = ShowWindow(window, if index == page { SW_SHOW } else { SW_HIDE });
            }
        }
    }

    /// Ask a page whether it may be left (its fields are valid).
    fn leave(&self, page: usize) -> bool {
        self.notify(page, PSN_KILLACTIVE) == 0
    }

    /// Send a page a `PSN_` notification and return what it set with
    /// [`set_result`].
    fn notify(&self, page: usize, code: u32) -> isize {
        let Some(&window) = self.windows.borrow().get(page) else {
            return 0;
        };
        let header = NMHDR {
            hwndFrom: self.tabs.get(),
            idFrom: usize::from(TAB_CONTROL),
            code,
        };
        set_result(window, 0);
        // SAFETY: `header` lives until the call returns; DWLP_MSGRESULT is
        // the page's own result slot.
        unsafe {
            SendMessageW(
                window,
                WM_NOTIFY,
                Some(WPARAM(usize::from(TAB_CONTROL))),
                Some(LPARAM(&header as *const _ as isize)),
            );
            GetWindowLongPtrW(window, WINDOW_LONG_PTR_INDEX(DWLP_MSGRESULT as i32))
        }
    }
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    // SAFETY: GWLP_USERDATA holds the pointer passed to
    // DialogBoxIndirectParamW, which lives until the modal loop ends.
    unsafe {
        if msg == WM_INITDIALOG {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, lparam.0);
        }
        dispatch(hwnd, msg, wparam, lparam)
    }
}

/// Hand a message to the handler stored in `GWLP_USERDATA`.
///
/// # Safety
///
/// `GWLP_USERDATA` is 0 or a pointer to a `&dyn Dialog` that is still alive.
unsafe fn dispatch(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    // SAFETY: passed on to the caller.
    unsafe {
        let slot = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const &dyn Dialog;
        if slot.is_null() {
            return 0;
        }
        match (*slot).message(hwnd, msg, wparam, lparam) {
            Some(result) => result,
            // TRUE from WM_INITDIALOG: put the focus on the first control.
            None if msg == WM_INITDIALOG => 1,
            None => 0,
        }
    }
}
