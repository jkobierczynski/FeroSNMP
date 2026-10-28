//! Window classes whose messages go to a Rust value.
//!
//! Win32 delivers every message for a window to one "window procedure"
//! function. [`create`] stores a Rust value (anything that implements
//! [`Window`]) with the window and routes each message to
//! [`Window::message`].
//!
//! # Re-entrancy
//!
//! Many Win32 calls send messages back to the window *before they return*.
//! For example, expanding a tree view item sends `WM_NOTIFY` to the tree's
//! parent from inside the expand call. So `Window::message` can be running
//! several times at once on the same thread, one call nested inside another.
//!
//! That is why the handler takes `&self`, not `&mut self`: Rust does not allow
//! two `&mut` to the same value at the same time. Keep the window's state in
//! [`Cell`](std::cell::Cell) and [`RefCell`](std::cell::RefCell) fields, and
//! do not hold a `RefCell` borrow across a Win32 call that may send messages
//! (the nested call would panic when it borrows the same field).

use std::rc::Rc;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_BTNFACE, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW,
    GWLP_USERDATA, GetWindowLongPtrW, HICON, HMENU, IDC_ARROW, LoadCursorW, RegisterClassExW,
    SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_NCCREATE, WM_NCDESTROY, WNDCLASSEXW,
};
use windows::core::{HSTRING, PCWSTR};

/// A Rust value that handles the messages of a window.
pub trait Window {
    /// Handle one message. Return `Some(result)` if it was handled, or `None`
    /// to let Windows do its default processing (`DefWindowProcW`).
    ///
    /// See the [module documentation](self) for why this takes `&self`.
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT>;
}

/// The module handle of the running program.
pub fn instance() -> HINSTANCE {
    // SAFETY: a null module name means the executable itself, which is
    // always loaded.
    unsafe { GetModuleHandleW(None).map(Into::into).unwrap_or_default() }
}

/// Register a window class whose windows are created with [`create`].
///
/// The background is the dialog colour, which also shows in the gaps between
/// panes, such as splitter bars.
pub fn register_class(name: &HSTRING, icon: Option<HICON>) -> windows::core::Result<()> {
    // SAFETY: the class structure is fully initialised and `name` outlives
    // the call (Windows copies the name).
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as usize as *mut _),
            lpszClassName: PCWSTR(name.as_ptr()),
            hIcon: icon.unwrap_or_default(),
            hIconSm: icon.unwrap_or_default(),
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
    }
    Ok(())
}

/// Options for [`create`]. Sizes are in physical pixels.
#[derive(Debug, Clone)]
pub struct CreateOptions {
    /// Window style, such as `WS_OVERLAPPEDWINDOW`.
    pub style: WINDOW_STYLE,
    /// Extended window style.
    pub ex_style: WINDOW_EX_STYLE,
    /// Position, or `None` to let Windows choose.
    pub position: Option<(i32, i32)>,
    /// Size, or `None` to let Windows choose.
    pub size: Option<(i32, i32)>,
    /// Owner (for a top-level window) or parent (for a child window).
    pub parent: Option<HWND>,
    /// Menu bar of a top-level window.
    pub menu: Option<HMENU>,
}

/// Create a window of a class registered with [`register_class`], with
/// `handler` receiving its messages from the first one (`WM_NCCREATE`) on.
///
/// The window keeps the handler alive until it is destroyed.
pub fn create(
    class: &HSTRING,
    title: &str,
    options: &CreateOptions,
    handler: Rc<dyn Window>,
) -> windows::core::Result<HWND> {
    let (x, y) = options.position.unwrap_or((CW_USEDEFAULT, CW_USEDEFAULT));
    let (width, height) = options.size.unwrap_or((CW_USEDEFAULT, CW_USEDEFAULT));
    // The handler travels to WM_NCCREATE as the creation parameter. An
    // `Rc<dyn Window>` is two pointers wide, so it is boxed to get a single
    // pointer that fits.
    //
    // From WM_NCCREATE on, the window owns the box and frees it in
    // WM_NCDESTROY, also when creation fails half-way. Only if Windows fails
    // before WM_NCCREATE (an unregistered class) does the box leak; that is a
    // programming error, and leaking a few bytes is the safe outcome.
    let boxed: *mut Rc<dyn Window> = Box::into_raw(Box::new(handler));
    // SAFETY: `window_proc` takes ownership of `boxed` in WM_NCCREATE.
    unsafe {
        CreateWindowExW(
            options.ex_style,
            class,
            &HSTRING::from(title),
            options.style,
            x,
            y,
            width,
            height,
            options.parent,
            options.menu,
            Some(instance()),
            Some(boxed as *const _),
        )
    }
}

/// The window procedure shared by every class from [`register_class`].
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: GWLP_USERDATA holds either 0 or the box created in `create`,
    // which stays valid until WM_NCDESTROY sets the slot back to 0.
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let slot = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Rc<dyn Window>;
        if slot.is_null() {
            // Messages sent before WM_NCCREATE (WM_GETMINMAXINFO) have no
            // handler yet.
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        // Take our own reference for the duration of this call, so the
        // handler stays alive even if a nested call reaches WM_NCDESTROY
        // and releases the window's reference.
        let handler: Rc<dyn Window> = Rc::clone(&*slot);
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(slot));
        }
        match handler.message(hwnd, msg, wparam, lparam) {
            Some(result) => result,
            None => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
