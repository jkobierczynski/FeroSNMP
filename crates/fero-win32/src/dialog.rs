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

use std::cell::Cell;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DLGTEMPLATE, DS_MODALFRAME, DS_SETFONT, DialogBoxIndirectParamW, GWLP_USERDATA,
    GetWindowLongPtrW, SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INITDIALOG, WS_CAPTION,
    WS_CHILD, WS_POPUP, WS_SYSMENU, WS_VISIBLE,
};

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
        let mut template = DialogTemplate {
            words: Vec::new(),
            count_at: 0,
        };
        let style = WS_POPUP
            | WS_CAPTION
            | WS_SYSMENU
            | WINDOW_STYLE(DS_MODALFRAME as u32)
            | WINDOW_STYLE(DS_SETFONT as u32);
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
        // DialogBoxIndirectParamW needs the template on a 4-byte boundary; a
        // Vec<u16> only guarantees 2, so copy it into u32s.
        let mut aligned = vec![0u32; self.words.len().div_ceil(2)];
        for (i, word) in self.words.iter().enumerate() {
            aligned[i / 2] |= u32::from(*word) << (16 * (i % 2));
        }
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
