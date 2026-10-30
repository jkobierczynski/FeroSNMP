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
//! [`property_sheet`] shows several templates made with
//! [`DialogTemplate::page`] as the tabs of one Settings-style dialog.

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{
    PROPSHEETHEADERW_V2, PROPSHEETHEADERW_V2_1, PROPSHEETHEADERW_V2_2, PROPSHEETPAGEW,
    PROPSHEETPAGEW_0, PSH_NOAPPLYNOW, PSH_NOCONTEXTHELP, PSH_PROPSHEETPAGE, PSP_DLGINDIRECT,
    PropertySheetW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DLGTEMPLATE, DS_MODALFRAME, DS_SETFONT, DWLP_MSGRESULT, DialogBoxIndirectParamW, GWLP_USERDATA,
    GetWindowLongPtrW, SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INITDIALOG, WS_CAPTION,
    WS_CHILD, WS_DISABLED, WS_POPUP, WS_SYSMENU, WS_VISIBLE,
};
use windows::core::HSTRING;

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

    /// A page of a [`property_sheet`]: `title` is the text of its tab.
    pub fn page(title: &str, width: i16, height: i16, font: &str, points: u16) -> Self {
        let style = WS_CHILD | WS_DISABLED | WS_CAPTION | WINDOW_STYLE(DS_SETFONT as u32);
        Self::with_style(style, title, width, height, font, points)
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
/// calling it. A property page refuses to be left (`PSN_KILLACTIVE`) with
/// `set_result(page, 1)`.
pub fn set_result(hwnd: HWND, value: isize) {
    // SAFETY: DWLP_MSGRESULT is the dialog's own result slot.
    unsafe {
        SetWindowLongPtrW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::WINDOW_LONG_PTR_INDEX(DWLP_MSGRESULT as i32),
            value,
        );
    }
}

/// `PSH_USESTARTPAGE`: open on the page `nStartPage` (not in the windows
/// crate).
const PSH_USESTARTPAGE: u32 = 0x0040;

/// Show `pages` (made with [`DialogTemplate::page`]) as the tabs of a
/// modal property sheet with OK and Cancel, opening on page `start`. Each
/// page's handler gets the usual dialog messages, and `WM_NOTIFY` with
/// `PSN_KILLACTIVE` when the user leaves it or presses OK (refuse with
/// [`set_result`]), and `PSN_APPLY` when OK is pressed, but only if the page
/// was shown: a page never opened never gets messages.
///
/// Returns once the sheet is closed. Whether OK was pressed is for the
/// handlers to note on `PSN_APPLY`.
pub fn property_sheet(
    owner: Option<HWND>,
    title: &str,
    pages: &[(&DialogTemplate, &dyn Dialog)],
    start: usize,
) -> windows::core::Result<()> {
    let templates: Vec<Vec<u32>> = pages
        .iter()
        .map(|(template, _)| template.aligned())
        .collect();
    // The handlers are wide pointers (`&dyn`); each page gets a pointer to
    // one of them, which lives in this Vec until the sheet is closed.
    let handlers: Vec<&dyn Dialog> = pages.iter().map(|(_, handler)| *handler).collect();
    let mut sheet_pages: Vec<PROPSHEETPAGEW> = templates
        .iter()
        .zip(&handlers)
        .map(|(template, handler)| PROPSHEETPAGEW {
            dwSize: size_of::<PROPSHEETPAGEW>() as u32,
            dwFlags: PSP_DLGINDIRECT,
            hInstance: instance(),
            Anonymous1: PROPSHEETPAGEW_0 {
                pResource: template.as_ptr() as *mut DLGTEMPLATE,
            },
            pfnDlgProc: Some(page_proc),
            lParam: LPARAM(handler as *const &dyn Dialog as isize),
            ..Default::default()
        })
        .collect();
    let caption = HSTRING::from(title);
    let mut header = PROPSHEETHEADERW_V2 {
        dwSize: size_of::<PROPSHEETHEADERW_V2>() as u32,
        dwFlags: PSH_PROPSHEETPAGE | PSH_NOAPPLYNOW | PSH_NOCONTEXTHELP | PSH_USESTARTPAGE,
        hwndParent: owner.unwrap_or_default(),
        hInstance: instance(),
        pszCaption: windows::core::PCWSTR(caption.as_ptr()),
        nPages: sheet_pages.len() as u32,
        Anonymous2: PROPSHEETHEADERW_V2_1 {
            nStartPage: start.min(pages.len().saturating_sub(1)) as u32,
        },
        Anonymous3: PROPSHEETHEADERW_V2_2 {
            ppsp: sheet_pages.as_mut_ptr(),
        },
        ..Default::default()
    };
    // SAFETY: every pointer in the header and pages points into `caption`,
    // `templates`, `handlers` or `sheet_pages`, which outlive the modal
    // loop; PropertySheetW only returns once the sheet is gone.
    let result = unsafe { PropertySheetW(&mut header) };
    if result < 0 {
        Err(windows::core::Error::from_thread())
    } else {
        Ok(())
    }
}

/// The dialog procedure of a property page: like [`dialog_proc`], but the
/// handler arrives inside the page's `PROPSHEETPAGEW`.
unsafe extern "system" fn page_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    // SAFETY: for WM_INITDIALOG, lparam points to the page's copy of its
    // PROPSHEETPAGEW, whose lParam is the handler pointer set up by
    // `property_sheet`; that pointer lives until the sheet is closed.
    unsafe {
        if msg == WM_INITDIALOG {
            let page = &*(lparam.0 as *const PROPSHEETPAGEW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, page.lParam.0);
        }
        dispatch(hwnd, msg, wparam, lparam)
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
