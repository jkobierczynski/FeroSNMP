//! The main window: menu, toolbar with search box, MIB tree, details pane,
//! Diagnostics tab and status bar.
//!
//! ```text
//! +-----------------------------------------------------------+
//! | File  Edit  Help                                          |
//! | [Reload] [Settings]   Search [_____________________]      |  rebar
//! +---------------------+-+-----------------------------------+
//! | v iso(1)            | | Property   | Value                |
//! |   v org(3)          | | OID        | 1.3.6.1.2.1.1.1      |  properties
//! |     > dod(6)        | | ...        |                      |
//! |                     | +-----------------------------------+  <- splitter
//! |                     | | A textual description of ...      |  description
//! +---------------------+-+-----------------------------------+  <- splitter
//! | [Diagnostics (12 errors, 3 warnings)]                     |
//! | Severity | Module      | File | Line | Message | Fix hint |  diagnostics
//! +-----------------------------------------------------------+
//! | Loaded in 1.2 s            | 214 modules, 48213 nodes     |  status bar
//! +-----------------------------------------------------------+
//!                        ^ splitter
//! ```
//!
//! Clicking a diagnostic shows its source in a [`TextWindow`], with the place
//! it points at highlighted. MIB files and folders can be dropped on the
//! window to load them for this session, and when the window becomes active
//! again after a loaded file was edited elsewhere, the MIBs are reloaded.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOW, GetSysColor, GetSysColorBrush, HDC, InvalidateRect, ScreenToClient, SetBkColor,
    UpdateWindow,
};
use windows::Win32::UI::Controls::{
    HTREEITEM, LVIS_SELECTED, LVM_SETEXTENDEDLISTVIEWSTYLE, LVN_GETDISPINFOW, LVN_ITEMACTIVATE,
    LVN_ITEMCHANGED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT, LVS_EX_LABELTIP, LVS_NOSORTHEADER,
    LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL, NM_CLICK, NM_SETFOCUS, NMHDR, NMLISTVIEW,
    NMLVDISPINFOW, NMTREEVIEWW, SB_SETPARTS, SB_SETTEXTW, SBARS_SIZEGRIP, STATUSCLASSNAMEW,
    SetWindowTheme, TVE_EXPAND, TVI_ROOT, TVIS_EXPANDEDONCE, TVM_SETEXTENDEDSTYLE,
    TVM_SETIMAGELIST, TVN_ITEMEXPANDINGW, TVN_SELCHANGEDW, TVS_EX_DOUBLEBUFFER, TVS_HASBUTTONS,
    TVS_LINESATROOT, TVS_SHOWSELALWAYS, TVSIL_NORMAL, WC_LISTVIEWW, WC_TREEVIEWW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateMenu, CreatePopupMenu, CreateWindowExW, DestroyWindow, ES_AUTOVSCROLL,
    ES_MULTILINE, ES_READONLY, GetClientRect, GetCursorPos, GetWindowRect, HMENU, HTCLIENT,
    IDC_SIZENS, IDC_SIZEWE, LoadCursorW, MB_ICONINFORMATION, MB_OK, MF_POPUP, MF_SEPARATOR,
    MF_STRING, MINMAXINFO, MessageBoxW, PostQuitMessage, SW_SHOW, SWP_NOACTIVATE, SWP_NOCOPYBITS,
    SWP_NOZORDER, SendMessageW, SetCursor, SetWindowPos, SetWindowTextW, ShowWindow,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATEAPP, WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DROPFILES, WM_GETMINMAXINFO,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NOTIFY, WM_SETCURSOR, WM_SETREDRAW,
    WM_SETTINGCHANGE, WM_SIZE, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, w};

use mib_core::{Diagnostic, MibSet, NodeId, SourceView};

use fero_win32::dpi;
use fero_win32::font::Font;
use fero_win32::shell::{self, FileType};
use fero_win32::splitter::{Orientation, Pane, Rect, Splitter};
use fero_win32::text::window_text;
use fero_win32::window::{self, CreateOptions, Window, instance};
use fero_win32::{hiword, loword, point_from_lparam, theme};

use crate::diagnostics::{DiagnosticList, ExportFormat};
use crate::file_stamps::FileStamps;
use crate::find::Matches;
use crate::loading::{self, Loaded};
use crate::settings::Settings;
use crate::{VERSION, diagnostics, node_info, rich_text};

use super::diagnostics_panel::DiagnosticsPanel;
use super::icons::{NodeIcons, image_for};
use super::text_window::{Content, TextWindow};
use super::toolbar::Toolbar;
use super::{WM_APP_LOADED, details, error_box, id, send, settings_dialog, tree};

/// Sizes at 96 DPI.
const WINDOW_SIZE: (i32, i32) = (1100, 720);
const MIN_SIZE: (i32, i32) = (520, 360);
const TREE_WIDTH: i32 = 380;
const PROPERTIES_HEIGHT: i32 = 280;
const DIAGNOSTICS_HEIGHT: i32 = 190;
const SPLITTER: i32 = 5;
const MIN_PANE: i32 = 80;
const COUNTS_WIDTH: i32 = 330;

/// The window handles of the controls in the window.
#[derive(Debug, Clone, Copy, Default)]
struct Controls {
    tree: HWND,
    properties: HWND,
    description: HWND,
    status: HWND,
    diagnostics: DiagnosticsPanel,
}

/// Which splitter is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Tree,
    Details,
    Bottom,
}

/// The state of the main window.
///
/// See [`fero_win32::window`] for why every field is a `Cell` or `RefCell`.
pub struct MainWindow {
    hwnd: Cell<HWND>,
    controls: Cell<Controls>,
    toolbar: Cell<Option<Toolbar>>,
    dpi: Cell<u32>,
    font: RefCell<Option<Font>>,
    icons: RefCell<Option<NodeIcons>>,
    /// Between the tree and the details.
    tree_split: RefCell<Splitter>,
    /// Between the properties and the description.
    details_split: RefCell<Splitter>,
    /// Between the tree and details above and the Diagnostics tab below.
    bottom_split: RefCell<Splitter>,
    dragging: Cell<Option<Drag>>,

    settings: RefCell<Settings>,
    settings_path: Option<PathBuf>,
    /// A problem reading the settings file at startup, shown once the window
    /// is open.
    settings_error: RefCell<Option<String>>,

    /// The loaded MIBs, once the first load has finished.
    mibs: RefCell<Option<MibSet>>,
    /// The node of each tree item: the item's `lParam` is an index here.
    items: RefCell<Vec<NodeId>>,
    /// The last search.
    matches: RefCell<Option<Matches>>,
    /// Number of the most recent load; older results are ignored.
    generation: Cell<u64>,
    /// True while the tree is being emptied, to ignore its notifications.
    clearing: Cell<bool>,
    /// True from starting a load until its result arrives.
    loading: Cell<bool>,

    /// The diagnostics of the loaded set, as the Diagnostics tab lists them.
    diagnostics: RefCell<DiagnosticList>,
    /// The source viewer, once opened, and the row it shows.
    source_window: RefCell<Option<Rc<TextWindow>>>,
    source_row: Cell<Option<usize>>,
    /// The "Why this symbol?" report, once opened.
    why_window: RefCell<Option<Rc<TextWindow>>>,
    /// True if the Diagnostics list, rather than the tree, last had the
    /// focus: "Why this symbol?" then explains the selected diagnostic.
    diagnostics_focused: Cell<bool>,
    /// Files and folders dropped on the window, searched before the ones in
    /// the settings, for this session only.
    dropped: RefCell<Vec<PathBuf>>,
    /// The loaded files and their modification times, to notice edits.
    stamps: RefCell<FileStamps>,
}

impl MainWindow {
    /// Read the settings and create the (still hidden) window.
    pub fn create() -> windows::core::Result<Rc<MainWindow>> {
        let settings_path = Settings::default_path();
        let (settings, settings_error) = match settings_path.as_deref().map(Settings::load) {
            Some(Ok(settings)) => (settings, None),
            Some(Err(error)) => (
                Settings::default(),
                Some(format!(
                    "Your settings could not be read, so the defaults are used. \
                     Saving settings will replace the file.\n\n{error}"
                )),
            ),
            None => (Settings::default(), None),
        };

        let dpi = dpi::for_system();
        let window = Rc::new(MainWindow {
            hwnd: Cell::new(HWND::default()),
            controls: Cell::new(Controls::default()),
            toolbar: Cell::new(None),
            dpi: Cell::new(dpi),
            font: RefCell::new(None),
            icons: RefCell::new(None),
            tree_split: RefCell::new(Splitter::new(
                Orientation::Vertical,
                dpi::scale(TREE_WIDTH, dpi),
                dpi::scale(SPLITTER, dpi),
                dpi::scale(MIN_PANE, dpi),
            )),
            details_split: RefCell::new(Splitter::new(
                Orientation::Horizontal,
                dpi::scale(PROPERTIES_HEIGHT, dpi),
                dpi::scale(SPLITTER, dpi),
                dpi::scale(MIN_PANE, dpi),
            )),
            bottom_split: RefCell::new(
                Splitter::new(
                    Orientation::Horizontal,
                    dpi::scale(DIAGNOSTICS_HEIGHT, dpi),
                    dpi::scale(SPLITTER, dpi),
                    dpi::scale(MIN_PANE, dpi),
                )
                .with_fixed_pane(Pane::Second),
            ),
            dragging: Cell::new(None),
            settings: RefCell::new(settings),
            settings_path,
            settings_error: RefCell::new(settings_error),
            mibs: RefCell::new(None),
            items: RefCell::new(Vec::new()),
            matches: RefCell::new(None),
            generation: Cell::new(0),
            clearing: Cell::new(false),
            loading: Cell::new(false),
            diagnostics: RefCell::new(DiagnosticList::default()),
            source_window: RefCell::new(None),
            source_row: Cell::new(None),
            why_window: RefCell::new(None),
            diagnostics_focused: Cell::new(false),
            dropped: RefCell::new(Vec::new()),
            stamps: RefCell::new(FileStamps::default()),
        });

        let class = HSTRING::from("FeroSNMP.MainWindow");
        window::register_class(&class, None)?;
        let options = CreateOptions {
            style: WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            ex_style: WS_EX_CONTROLPARENT,
            position: None,
            size: Some((
                dpi::scale(WINDOW_SIZE.0, dpi),
                dpi::scale(WINDOW_SIZE.1, dpi),
            )),
            parent: None,
            menu: Some(menu_bar()?),
        };
        let hwnd = window::create(&class, "FeroSNMP", &options, window.clone())?;
        window.hwnd.set(hwnd);
        Ok(window)
    }

    /// The window handle.
    pub fn hwnd(&self) -> HWND {
        self.hwnd.get()
    }

    /// Show the window and start loading MIBs.
    pub fn start(&self) {
        // SAFETY: showing our own window.
        unsafe {
            let _ = ShowWindow(self.hwnd(), SW_SHOW);
            let _ = UpdateWindow(self.hwnd());
        }
        // Take the message out first: the message box runs a message loop,
        // which may call back into this window.
        let settings_error = self.settings_error.borrow_mut().take();
        if let Some(message) = settings_error {
            error_box(Some(self.hwnd()), &message);
        }
        self.start_load();
    }

    // ----- creating and laying out the controls -----

    fn on_create(&self, hwnd: HWND) -> windows::core::Result<()> {
        self.hwnd.set(hwnd);
        let dpi = dpi::for_window(hwnd);
        if dpi != self.dpi.get() {
            // The window opened on a monitor other than the primary one.
            self.tree_split.borrow_mut().rescale(self.dpi.get(), dpi);
            self.details_split.borrow_mut().rescale(self.dpi.get(), dpi);
            self.bottom_split.borrow_mut().rescale(self.dpi.get(), dpi);
            self.dpi.set(dpi);
        }
        theme::apply_title_bar(hwnd);
        let font = Font::message(dpi)?;
        let child = |class, style: WINDOW_STYLE, ex_style, id: u16| {
            // SAFETY: creates a child control of our window.
            unsafe {
                CreateWindowExW(
                    ex_style,
                    class,
                    None,
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

        let tree = child(
            WC_TREEVIEWW,
            WS_TABSTOP | WINDOW_STYLE(TVS_HASBUTTONS | TVS_LINESATROOT | TVS_SHOWSELALWAYS),
            WS_EX_CLIENTEDGE,
            id::TREE,
        )?;
        let properties = child(
            WC_LISTVIEWW,
            WS_TABSTOP
                | WINDOW_STYLE(LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER),
            WS_EX_CLIENTEDGE,
            id::PROPERTIES,
        )?;
        let description = child(
            w!("EDIT"),
            WS_TABSTOP
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
            WS_EX_CLIENTEDGE,
            id::DESCRIPTION,
        )?;
        let status = child(
            STATUSCLASSNAMEW,
            WINDOW_STYLE(SBARS_SIZEGRIP),
            WINDOW_EX_STYLE(0),
            id::STATUS,
        )?;
        let diagnostics = DiagnosticsPanel::create(hwnd, dpi)?;

        // SAFETY: plain control messages and theme calls on our own controls.
        unsafe {
            // The "Explorer" theme gives the tree and list the Windows 10/11
            // look (no dotted lines, soft selection) instead of the classic one.
            let _ = SetWindowTheme(tree, w!("Explorer"), None);
            let _ = SetWindowTheme(properties, w!("Explorer"), None);
            send(
                tree,
                TVM_SETEXTENDEDSTYLE,
                TVS_EX_DOUBLEBUFFER as usize,
                TVS_EX_DOUBLEBUFFER as isize,
            );
            let extended = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP;
            send(
                properties,
                LVM_SETEXTENDEDLISTVIEWSTYLE,
                extended as usize,
                extended as isize,
            );
        }
        details::add_columns(properties, dpi);

        self.controls.set(Controls {
            tree,
            properties,
            description,
            status,
            diagnostics,
        });
        *self.font.borrow_mut() = Some(font);
        self.apply_dpi_resources()?;
        // SAFETY: plain control message, and accepting files dropped from
        // Explorer (they arrive as WM_DROPFILES).
        unsafe {
            let _ = SetFocus(Some(tree));
            DragAcceptFiles(hwnd, true);
        }
        Ok(())
    }

    /// Create the font, icons and toolbar for the current DPI and hand them
    /// to the controls. Used at creation and when the DPI changes.
    fn apply_dpi_resources(&self) -> windows::core::Result<()> {
        let dpi = self.dpi.get();
        let controls = self.controls.get();
        let font = Font::message(dpi)?;
        for control in [
            controls.tree,
            controls.properties,
            controls.description,
            controls.status,
            controls.diagnostics.tabs,
            controls.diagnostics.list,
        ] {
            font.apply(control);
        }

        // The rebar does not adapt its bands to a new DPI, so it is simply
        // made again; only the search text needs carrying over.
        let search_text = self.toolbar.get().map(|old| {
            let text = window_text(old.search);
            old.destroy();
            text
        });
        let toolbar = Toolbar::create(self.hwnd(), dpi, &font)?;
        if let Some(text) = search_text {
            // SAFETY: setting the text of our own control.
            unsafe {
                let _ = SetWindowTextW(toolbar.search, &HSTRING::from(text));
            }
        }
        self.toolbar.set(Some(toolbar));

        let icons = NodeIcons::new(dpi);
        // SAFETY: the tree gets the new list before the old one is dropped.
        unsafe {
            send(
                controls.tree,
                TVM_SETIMAGELIST,
                TVSIL_NORMAL as usize,
                icons.handle().0,
            );
        }
        // Replacing the values drops the old font and icons, which the
        // controls no longer use.
        *self.icons.borrow_mut() = Some(icons);
        *self.font.borrow_mut() = Some(font);
        details::size_columns(controls.properties, dpi);
        controls.diagnostics.size_columns(dpi);
        Ok(())
    }

    /// The area between the toolbar and the status bar.
    fn pane_area(&self) -> Rect {
        let controls = self.controls.get();
        let mut client = RECT::default();
        let height_of = |hwnd: HWND| {
            let mut rect = RECT::default();
            // SAFETY: reading the rectangle of our own control.
            unsafe {
                let _ = GetWindowRect(hwnd, &mut rect);
            }
            rect.bottom - rect.top
        };
        // SAFETY: reading our own client rectangle.
        unsafe {
            let _ = GetClientRect(self.hwnd(), &mut client);
        }
        let top = self
            .toolbar
            .get()
            .map_or(0, |toolbar| height_of(toolbar.rebar));
        let bottom = client.bottom - height_of(controls.status);
        Rect {
            left: 0,
            top,
            right: client.right,
            bottom: bottom.max(top),
        }
    }

    fn layout(&self) {
        let controls = self.controls.get();
        let dpi = self.dpi.get();
        // SAFETY: plain messages to our own controls. The rebar and status
        // bar size themselves when told the window size changed.
        unsafe {
            if let Some(toolbar) = self.toolbar.get() {
                send(toolbar.rebar, WM_SIZE, 0, 0);
            }
            send(controls.status, WM_SIZE, 0, 0);
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd(), &mut client);
            let parts = [client.right - dpi::scale(COUNTS_WIDTH, dpi), -1];
            SendMessageW(
                controls.status,
                SB_SETPARTS,
                Some(WPARAM(parts.len())),
                Some(LPARAM(parts.as_ptr() as isize)),
            );
        }

        let (top, _, bottom) = self.bottom_split.borrow().split(self.pane_area());
        let (tree, _, details) = self.tree_split.borrow().split(top);
        let (properties, _, description) = self.details_split.borrow().split(details);
        let place = |hwnd: HWND, rect: Rect| {
            // SWP_NOCOPYBITS: repaint the control instead of moving its old
            // pixels along, which left stale column text while dragging a
            // splitter.
            // SAFETY: moving our own control.
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.width(),
                    rect.height(),
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
                );
            }
        };
        place(controls.tree, tree);
        place(controls.properties, properties);
        place(controls.description, description);
        controls.diagnostics.place(bottom);
        details::fill_value_column(controls.properties);
    }

    fn on_dpi_changed(&self, new_dpi: u32, suggested: &RECT) {
        let old_dpi = self.dpi.replace(new_dpi);
        self.tree_split.borrow_mut().rescale(old_dpi, new_dpi);
        self.details_split.borrow_mut().rescale(old_dpi, new_dpi);
        self.bottom_split.borrow_mut().rescale(old_dpi, new_dpi);
        if let Err(error) = self.apply_dpi_resources() {
            error_box(
                Some(self.hwnd()),
                &format!("Could not adapt to the new display scaling.\n\n{error}"),
            );
        }
        // SAFETY: Windows suggests where the window should go at the new DPI.
        unsafe {
            let _ = SetWindowPos(
                self.hwnd(),
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.layout();
    }

    // ----- splitters -----

    /// The area above the Diagnostics tab, which the tree and details share.
    fn top_area(&self) -> Rect {
        self.bottom_split.borrow().split(self.pane_area()).0
    }

    /// The splitter under the mouse, if any.
    fn splitter_at(&self, x: i32, y: i32) -> Option<Drag> {
        if self.bottom_split.borrow().hit(self.pane_area(), x, y) {
            return Some(Drag::Bottom);
        }
        let area = self.top_area();
        let tree_split = self.tree_split.borrow();
        if tree_split.hit(area, x, y) {
            return Some(Drag::Tree);
        }
        let (_, _, details) = tree_split.split(area);
        self.details_split
            .borrow()
            .hit(details, x, y)
            .then_some(Drag::Details)
    }

    fn on_set_cursor(&self, lparam: LPARAM) -> bool {
        if u32::from(loword(lparam.0 as usize)) != HTCLIENT {
            return false;
        }
        let mut point = POINT::default();
        // SAFETY: reading the cursor position and converting it to our
        // client coordinates.
        unsafe {
            let _ = GetCursorPos(&mut point);
            let _ = ScreenToClient(self.hwnd(), &mut point);
        }
        let cursor = match self
            .dragging
            .get()
            .or_else(|| self.splitter_at(point.x, point.y))
        {
            Some(Drag::Tree) => IDC_SIZEWE,
            Some(Drag::Details | Drag::Bottom) => IDC_SIZENS,
            None => return false,
        };
        // SAFETY: loading a system cursor.
        unsafe {
            if let Ok(cursor) = LoadCursorW(None, cursor) {
                SetCursor(Some(cursor));
            }
        }
        true
    }

    fn on_mouse_down(&self, x: i32, y: i32) {
        let area = self.top_area();
        let drag = match self.splitter_at(x, y) {
            Some(Drag::Bottom) => self
                .bottom_split
                .borrow_mut()
                .begin_drag(self.pane_area(), x, y)
                .then_some(Drag::Bottom),
            Some(Drag::Tree) => self
                .tree_split
                .borrow_mut()
                .begin_drag(area, x, y)
                .then_some(Drag::Tree),
            Some(Drag::Details) => {
                let (_, _, details) = self.tree_split.borrow().split(area);
                self.details_split
                    .borrow_mut()
                    .begin_drag(details, x, y)
                    .then_some(Drag::Details)
            }
            None => None,
        };
        if drag.is_some() {
            self.dragging.set(drag);
            // SAFETY: capturing the mouse for our own window.
            unsafe {
                SetCapture(self.hwnd());
            }
        }
    }

    fn on_mouse_move(&self, x: i32, y: i32) {
        let area = self.top_area();
        let moved = match self.dragging.get() {
            Some(Drag::Bottom) => self.bottom_split.borrow_mut().drag(self.pane_area(), x, y),
            Some(Drag::Tree) => self.tree_split.borrow_mut().drag(area, x, y),
            Some(Drag::Details) => {
                let (_, _, details) = self.tree_split.borrow().split(area);
                self.details_split.borrow_mut().drag(details, x, y)
            }
            None => false,
        };
        if moved {
            self.layout();
        }
    }

    fn end_drag(&self) {
        if self.dragging.take().is_some() {
            self.tree_split.borrow_mut().end_drag();
            self.details_split.borrow_mut().end_drag();
            self.bottom_split.borrow_mut().end_drag();
            // SAFETY: releasing our own capture. This sends
            // WM_CAPTURECHANGED, which comes back here and finds no drag.
            unsafe {
                let _ = ReleaseCapture();
            }
        }
    }

    // ----- loading MIBs -----

    fn start_load(&self) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.loading.set(true);
        self.set_status("Loading MIBs\u{2026}");
        let library = self.settings.borrow().library_with(&self.dropped.borrow());
        // A window handle is not `Send`, but the number inside it is; the
        // loading thread only uses it to post one message back.
        let hwnd = self.hwnd().0 as isize;
        loading::spawn(library, generation, move |loaded| {
            let loaded = Box::into_raw(Box::new(loaded));
            // SAFETY: PostMessageW may be called from any thread. If the
            // window is gone, the post fails and we free the box here.
            unsafe {
                let posted = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(HWND(hwnd as *mut _)),
                    WM_APP_LOADED,
                    WPARAM(0),
                    LPARAM(loaded as isize),
                );
                if posted.is_err() {
                    drop(Box::from_raw(loaded));
                }
            }
        });
    }

    fn on_loaded(&self, loaded: Loaded) {
        if loaded.generation != self.generation.get() {
            return; // a newer load was started since
        }
        self.loading.set(false);
        match loaded.result {
            Ok(set) => {
                let seconds = loaded.elapsed.as_secs_f64();
                let list = DiagnosticList::new(set.diagnostics());
                let counts = format!(
                    "{} modules, {} nodes, {}",
                    set.modules().len(),
                    set.node_count(),
                    list.summary()
                );
                let only_built_in = set.modules().iter().all(|module| module.embedded);
                *self.stamps.borrow_mut() = FileStamps::of(&set);
                self.show_set(set);
                self.show_diagnostics(list);
                self.set_counts(&counts);
                if only_built_in {
                    self.set_status("Only the built-in modules are loaded. Add your MIB folders in File > Settings.");
                } else {
                    self.set_status(&format!("Loaded in {seconds:.1} s"));
                }
            }
            Err(error) => {
                self.set_status("Loading MIBs failed");
                error_box(
                    Some(self.hwnd()),
                    &format!(
                        "The MIBs could not be loaded.\n\n{}",
                        loading::error_text(&error)
                    ),
                );
            }
        }
    }

    /// Replace the tree with a newly loaded set, keeping the selected node
    /// selected if the new set has it.
    fn show_set(&self, set: MibSet) {
        let tree = self.controls.get().tree;
        let previous = self.selected_node().and_then(|id| {
            let mibs = self.mibs.borrow();
            mibs.as_ref().map(|old| old.node(id).oid())
        });

        // Emptying the tree sends a notification for every item; ignore them.
        self.clearing.set(true);
        // SAFETY: plain control messages.
        unsafe {
            send(tree, WM_SETREDRAW, 0, 0);
        }
        tree::clear(tree);
        self.clearing.set(false);

        // Node ids belong to one set, so everything that holds them is reset.
        self.items.borrow_mut().clear();
        *self.matches.borrow_mut() = None;
        let top_level = self.child_items(&set, set.root().id());
        let target = previous
            .and_then(|oid| set.node_at(&oid).map(|node| node.id()))
            .or_else(|| set.resolve("internet").map(|node| node.id()));
        *self.mibs.borrow_mut() = Some(set);

        for item in &top_level {
            tree::insert(tree, TVI_ROOT, item);
        }
        if let Some(item) = target.and_then(|target| self.reveal(target)) {
            tree::expand(tree, item);
        }
        // SAFETY: plain control messages.
        unsafe {
            send(tree, WM_SETREDRAW, 1, 0);
            let _ = InvalidateRect(Some(tree), None, true);
        }
        self.show_details();
    }

    /// The tree items for the children of `parent`, registering each child in
    /// the item table.
    fn child_items(&self, set: &MibSet, parent: NodeId) -> Vec<tree::NewItem> {
        let mut items = self.items.borrow_mut();
        set.node(parent)
            .children()
            .map(|child| {
                let name = child.name();
                let text = if name.is_empty() {
                    child.arc().to_string()
                } else {
                    format!("{name}({})", child.arc())
                };
                items.push(child.id());
                tree::NewItem {
                    text,
                    image: image_for(child.kind()),
                    has_children: child.has_children(),
                    param: (items.len() - 1) as isize,
                }
            })
            .collect()
    }

    /// The node shown by a tree item.
    fn node_of(&self, item: HTREEITEM) -> Option<NodeId> {
        let index = tree::param(self.controls.get().tree, item)?;
        self.items
            .borrow()
            .get(usize::try_from(index).ok()?)
            .copied()
    }

    fn selected_node(&self) -> Option<NodeId> {
        tree::selected(self.controls.get().tree).and_then(|item| self.node_of(item))
    }

    /// Expand the tree down to a node and select it. Returns its item.
    fn reveal(&self, target: NodeId) -> Option<HTREEITEM> {
        let tree = self.controls.get().tree;
        // The path from the top of the tree down to the target.
        let path: Vec<NodeId> = {
            let mibs = self.mibs.borrow();
            let set = mibs.as_ref()?;
            let mut path: Vec<NodeId> =
                std::iter::successors(Some(set.node(target)), |node| node.parent())
                    .filter(|node| node.parent().is_some()) // not the unnamed root
                    .map(|node| node.id())
                    .collect();
            path.reverse();
            path
        };
        let mut item = TVI_ROOT;
        for id in path {
            if item != TVI_ROOT {
                // Adds the children, through TVN_ITEMEXPANDING.
                tree::expand(tree, item);
            }
            item = tree::children(tree, item).find(|&child| self.node_of(child) == Some(id))?;
        }
        tree::select(tree, item);
        Some(item)
    }

    /// TVN_ITEMEXPANDING: add the children the first time an item opens.
    fn on_item_expanding(&self, info: &NMTREEVIEWW) {
        let first_time = info.itemNew.state.0 & TVIS_EXPANDEDONCE.0 == 0;
        if info.action.0 & TVE_EXPAND.0 == 0 || !first_time {
            return;
        }
        let Some(node) = self.node_of(info.itemNew.hItem) else {
            return;
        };
        let children = {
            let mibs = self.mibs.borrow();
            let Some(set) = mibs.as_ref() else {
                return;
            };
            self.child_items(set, node)
        };
        // The item is not open yet, so adding even thousands of children
        // (such as under ciscoProducts) causes no repainting.
        let tree = self.controls.get().tree;
        for child in &children {
            tree::insert(tree, info.itemNew.hItem, child);
        }
    }

    /// Show the selected node in the details pane.
    fn show_details(&self) {
        let controls = self.controls.get();
        let (rows, text) = {
            let mibs = self.mibs.borrow();
            match (mibs.as_ref(), self.selected_node()) {
                (Some(set), Some(id)) => {
                    let node = set.node(id);
                    (
                        node_info::properties(set, node),
                        node_info::description(node),
                    )
                }
                _ => (Vec::new(), String::new()),
            }
        };
        details::show(controls.properties, controls.description, &rows, &text);
    }

    // ----- diagnostics -----

    /// Show the diagnostics of a newly loaded set. If the source viewer is
    /// open, it moves to the diagnostic nearest to the one it showed, so
    /// after fixing a file and reloading, the user sees the same place.
    fn show_diagnostics(&self, list: DiagnosticList) {
        let previous = self.source_row.get().and_then(|row| {
            let old = self.diagnostics.borrow();
            let location = old.get(row)?.location.clone()?;
            Some((location.file, location.line))
        });
        let row = previous
            .as_ref()
            .and_then(|(file, line)| list.find(file, *line));
        let panel = self.controls.get().diagnostics;
        *self.diagnostics.borrow_mut() = list;
        self.source_row.set(None);
        panel.show(&self.diagnostics.borrow());
        let viewer_open = self
            .source_window
            .borrow()
            .as_ref()
            .is_some_and(|window| window.is_visible());
        if viewer_open {
            match row {
                Some(row) => {
                    panel.select(row);
                    self.show_source(row, false);
                }
                None => {
                    if let Some((file, _)) = previous {
                        self.show_file(&file);
                    }
                }
            }
        }
    }

    /// Show the source of a diagnostic in the viewer, opening it if needed.
    fn show_source(&self, row: usize, activate: bool) {
        let content = {
            let list = self.diagnostics.borrow();
            let Some(diagnostic) = list.get(row) else {
                return;
            };
            let mibs = self.mibs.borrow();
            source_content(mibs.as_ref(), diagnostic)
        };
        let has_symbol = self
            .diagnostics
            .borrow()
            .get(row)
            .is_some_and(|d| d.symbol.is_some());
        let Some(window) = self.text_window(&self.source_window, "FeroSNMP.SourceWindow", true)
        else {
            return;
        };
        self.source_row.set(Some(row));
        window.set_why_enabled(has_symbol);
        window.show(&content, activate);
    }

    /// Show a file without a diagnostic: after a reload fixed every problem
    /// the viewer pointed at.
    fn show_file(&self, file: &Path) {
        let view = {
            let mibs = self.mibs.borrow();
            mibs.as_ref()
                .and_then(|set| set.source_view(file))
                .or_else(|| SourceView::read(file).ok())
        };
        let Some(view) = view else {
            return;
        };
        let content = Content {
            title: format!("{} \u{2014} FeroSNMP", file_name(file)),
            header: format!(
                "{}\nNo problems left at the place shown before the reload.",
                file.display()
            ),
            rtf: rich_text::source_rtf(&view, None),
            caret: None,
        };
        if let Some(window) = self.source_window.borrow().as_ref() {
            window.set_why_enabled(false);
            window.show(&content, false);
        }
    }

    /// The text window in `slot`, created on first use.
    fn text_window(
        &self,
        slot: &RefCell<Option<Rc<TextWindow>>>,
        class: &str,
        with_why: bool,
    ) -> Option<Rc<TextWindow>> {
        if let Some(window) = slot.borrow().as_ref() {
            return Some(window.clone());
        }
        match TextWindow::create(self.hwnd(), class, with_why) {
            Ok(window) => {
                *slot.borrow_mut() = Some(window.clone());
                Some(window)
            }
            Err(error) => {
                error_box(
                    Some(self.hwnd()),
                    &format!("The window could not be opened.\n\n{error}"),
                );
                None
            }
        }
    }

    /// Edit > Why this symbol?: explain the symbol of the selected diagnostic
    /// if the Diagnostics list (or the viewer) was used last, otherwise the
    /// name of the node selected in the tree.
    fn explain_symbol(&self) {
        let query = {
            let list = self.diagnostics.borrow();
            let from_diagnostic = self
                .diagnostics_focused
                .get()
                .then(|| {
                    let row = self.controls.get().diagnostics.selected()?;
                    let diagnostic = list.get(row)?;
                    let symbol = diagnostic.symbol.as_ref()?;
                    Some(match &diagnostic.module {
                        Some(module) => format!("{module}::{symbol}"),
                        None => symbol.clone(),
                    })
                })
                .flatten();
            let mibs = self.mibs.borrow();
            from_diagnostic.or_else(|| {
                let set = mibs.as_ref()?;
                let node = set.node(self.selected_node()?);
                (!node.name().is_empty()).then(|| node.name().to_string())
            })
        };
        let Some(query) = query else {
            self.set_status(
                "Select a diagnostic about a symbol, or a node in the tree, to see why it resolves the way it does.",
            );
            return;
        };
        let report = {
            let mibs = self.mibs.borrow();
            let Some(set) = mibs.as_ref() else {
                return;
            };
            match set.trace(&query) {
                Ok(trace) => trace.to_string(),
                Err(error) => format!("{query}: {error}"),
            }
        };
        let content = Content {
            title: format!("Why {query}? \u{2014} FeroSNMP"),
            header: format!(
                "How {query} resolves with the rules the loader used: the \
                 module's own definitions, its IMPORTS, then the fallbacks the \
                 strictness allows. The same report as `ferosnmp mib why {query}`."
            ),
            rtf: rich_text::plain_rtf(&report),
            caret: None,
        };
        if let Some(window) = self.text_window(&self.why_window, "FeroSNMP.WhyWindow", false) {
            window.show(&content, true);
        }
    }

    /// File > Export diagnostics: save the list as text or JSON.
    fn export_diagnostics(&self) {
        let types = [
            FileType {
                name: "Text, as ferosnmp lint prints it (*.txt)",
                pattern: "*.txt",
                extension: "txt",
            },
            FileType {
                name: "JSON, as ferosnmp lint --format json (*.json)",
                pattern: "*.json",
                extension: "json",
            },
        ];
        let path = match shell::save_file(
            Some(self.hwnd()),
            "Export diagnostics",
            "diagnostics.txt",
            &types,
        ) {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                error_box(
                    Some(self.hwnd()),
                    &format!("The Save dialog could not be opened.\n\n{error}"),
                );
                return;
            }
        };
        let text = {
            let list = self.diagnostics.borrow();
            let mibs = self.mibs.borrow();
            let modules: Vec<String> = mibs
                .as_ref()
                .map(|set| {
                    set.modules()
                        .into_iter()
                        .filter(|module| !module.embedded)
                        .map(|module| module.name)
                        .collect()
                })
                .unwrap_or_default();
            diagnostics::export(list.rows(), &modules, ExportFormat::for_path(&path))
        };
        match std::fs::write(&path, text) {
            Ok(()) => self.set_status(&format!("Diagnostics saved to {}", path.display())),
            Err(error) => error_box(
                Some(self.hwnd()),
                &format!("{} could not be written.\n\n{error}", path.display()),
            ),
        }
    }

    /// WM_NOTIFY from the Diagnostics list.
    fn on_diagnostics_notify(&self, code: u32, lparam: LPARAM) {
        let panel = self.controls.get().diagnostics;
        match code {
            LVN_GETDISPINFOW => {
                // SAFETY: for LVN_GETDISPINFOW, lparam points to an
                // NMLVDISPINFOW whose text buffer we fill.
                let info = unsafe { &mut *(lparam.0 as *mut NMLVDISPINFOW) };
                DiagnosticsPanel::fill_cell(&self.diagnostics.borrow(), info);
            }
            NM_SETFOCUS => self.diagnostics_focused.set(true),
            // A click opens the source without taking the focus from the
            // list; a double click or Enter (LVN_ITEMACTIVATE) brings the
            // viewer to the front.
            NM_CLICK => {
                if let Some(row) = panel.selected() {
                    self.show_source(row, false);
                }
            }
            LVN_ITEMACTIVATE => {
                if let Some(row) = panel.selected() {
                    self.show_source(row, true);
                }
            }
            LVN_ITEMCHANGED => {
                // SAFETY: for LVN_ITEMCHANGED, lparam points to an NMLISTVIEW.
                let info = unsafe { &*(lparam.0 as *const NMLISTVIEW) };
                let selected =
                    info.uNewState & LVIS_SELECTED.0 != 0 && info.uOldState & LVIS_SELECTED.0 == 0;
                let viewer_open = self
                    .source_window
                    .borrow()
                    .as_ref()
                    .is_some_and(|window| window.is_visible());
                // Moving through the list with the keyboard follows along in
                // an open viewer.
                if selected
                    && viewer_open
                    && let Ok(row) = usize::try_from(info.iItem)
                    && self.source_row.get() != Some(row)
                {
                    self.show_source(row, false);
                }
            }
            _ => {}
        }
    }

    // ----- dropped files and edits elsewhere -----

    /// WM_DROPFILES: load the dropped files and folders, first, for this
    /// session.
    fn on_drop_files(&self, drop: HDROP) {
        let mut paths = Vec::new();
        // SAFETY: DragQueryFileW with index 0xFFFFFFFF returns the count;
        // each name is copied into a buffer of the length it reports. The
        // drop handle is released with DragFinish.
        unsafe {
            let count = DragQueryFileW(drop, u32::MAX, None);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, None) as usize;
                let mut buffer = vec![0u16; length + 1];
                let copied = DragQueryFileW(drop, index, Some(&mut buffer)) as usize;
                paths.push(PathBuf::from(String::from_utf16_lossy(&buffer[..copied])));
            }
            DragFinish(drop);
        }
        let added = {
            let mut dropped = self.dropped.borrow_mut();
            let before = dropped.len();
            for path in paths {
                if !dropped.contains(&path) {
                    dropped.push(path);
                }
            }
            dropped.len() - before
        };
        if added == 0 {
            self.set_status("Those files are loaded already.");
            return;
        }
        self.start_load();
        self.set_status(&format!(
            "Loading {added} dropped {} for this session (add folders in File > Settings to keep them)\u{2026}",
            if added == 1 { "item" } else { "items" }
        ));
    }

    /// WM_ACTIVATEAPP: when the user comes back from another program, reload
    /// if a loaded MIB file changed there.
    fn on_activated(&self) {
        if self.loading.get() {
            return;
        }
        let changed = self.stamps.borrow().changed();
        let Some(first) = changed.first() else {
            return;
        };
        let what = match changed.len() {
            1 => file_name(first),
            count => format!("{count} MIB files"),
        };
        self.start_load();
        self.set_status(&format!("{what} changed; reloading\u{2026}"));
    }

    // ----- searching -----

    fn find(&self, backwards: bool) {
        let Some(toolbar) = self.toolbar.get() else {
            return;
        };
        let query = window_text(toolbar.search);
        let (target, status) = {
            let mibs = self.mibs.borrow();
            let Some(set) = mibs.as_ref() else {
                return;
            };
            let mut matches = self.matches.borrow_mut();
            let target = match matches.as_mut() {
                // The same search again: go to the next (or previous) match.
                Some(current) if current.query() == query.trim() => {
                    if backwards {
                        current.previous()
                    } else {
                        current.next()
                    }
                }
                _ => {
                    let found = Matches::find(set, &query);
                    let target = found.current();
                    *matches = Some(found);
                    target
                }
            };
            let current = matches.as_ref().expect("set above");
            let status = match current.len() {
                _ if current.query().is_empty() => String::new(),
                _ if current.is_empty() => {
                    format!("Nothing matches \u{201c}{}\u{201d}", current.query())
                }
                1 => "1 match".to_string(),
                count => format!(
                    "Match {} of {count} (F3: next, Shift+F3: previous)",
                    current.position()
                ),
            };
            (target, status)
        };
        self.set_status(&status);
        if let Some(target) = target {
            self.reveal(target);
        }
    }

    // ----- commands -----

    fn on_command(&self, command: u16) {
        let controls = self.controls.get();
        // SAFETY: GetFocus has no preconditions.
        let focus = unsafe { GetFocus() };
        let search = self.toolbar.get().map(|toolbar| toolbar.search);
        match command {
            id::ENTER if Some(focus) == search => self.find(false),
            id::ENTER | id::SHOW_SOURCE if focus == controls.diagnostics.list => {
                if let Some(row) = controls.diagnostics.selected() {
                    self.show_source(row, true);
                }
            }
            id::SHOW_SOURCE => self.set_status("Select a diagnostic in the Diagnostics tab first."),
            id::WHY => self.explain_symbol(),
            id::EXPORT => self.export_diagnostics(),
            id::ESCAPE if Some(focus) == search => {
                // SAFETY: moving the focus between our own controls.
                unsafe {
                    let _ = SetFocus(Some(controls.tree));
                }
            }
            id::FIND => {
                if let Some(search) = search {
                    // SAFETY: plain control messages.
                    unsafe {
                        let _ = SetFocus(Some(search));
                        send(search, windows::Win32::UI::Controls::EM_SETSEL, 0, -1);
                    }
                }
            }
            id::FIND_NEXT => self.find(false),
            id::FIND_PREVIOUS => self.find(true),
            id::RELOAD => self.start_load(),
            id::SETTINGS => self.edit_settings(),
            id::EXIT => {
                // SAFETY: closing our own window.
                unsafe {
                    let _ = DestroyWindow(self.hwnd());
                }
            }
            id::ABOUT => {
                let text = format!(
                    "FeroSNMP {VERSION}\n\nA native Windows SNMP MIB tool.\n\n\
                     Licensed under the GNU General Public License, version 3 or later.\n\
                     https://github.com/jkobierczynski/ferosnmp"
                );
                // SAFETY: a message box owned by our window.
                unsafe {
                    MessageBoxW(
                        Some(self.hwnd()),
                        &HSTRING::from(text),
                        w!("About FeroSNMP"),
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
            }
            _ => {}
        }
    }

    fn edit_settings(&self) {
        let current = self.settings.borrow().clone();
        let Some(new) = settings_dialog::show(self.hwnd(), &current, self.settings_path.as_deref())
        else {
            return;
        };
        if new == current {
            return;
        }
        if let Some(path) = &self.settings_path
            && let Err(error) = new.save(path)
        {
            error_box(
                Some(self.hwnd()),
                &format!("The settings could not be saved.\n\n{error}"),
            );
        }
        *self.settings.borrow_mut() = new;
        self.start_load();
    }

    // ----- status bar -----

    fn set_status(&self, text: &str) {
        self.set_status_part(0, text);
    }

    fn set_counts(&self, text: &str) {
        self.set_status_part(1, text);
    }

    fn set_status_part(&self, part: usize, text: &str) {
        let text = HSTRING::from(text);
        // SAFETY: the text lives until the call returns; the status bar copies it.
        unsafe {
            SendMessageW(
                self.controls.get().status,
                SB_SETTEXTW,
                Some(WPARAM(part)),
                Some(LPARAM(text.as_ptr() as isize)),
            );
        }
    }
}

impl Window for MainWindow {
    fn message(&self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_CREATE => Some(LRESULT(match self.on_create(hwnd) {
                Ok(()) => 0,
                Err(error) => {
                    error_box(
                        None,
                        &format!("FeroSNMP could not create its window.\n\n{error}"),
                    );
                    -1
                }
            })),
            WM_SIZE => {
                self.layout();
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                // SAFETY: for this message, lparam points to a MINMAXINFO.
                let info = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                let dpi = self.dpi.get();
                info.ptMinTrackSize.x = dpi::scale(MIN_SIZE.0, dpi);
                info.ptMinTrackSize.y = dpi::scale(MIN_SIZE.1, dpi);
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                // SAFETY: for this message, lparam points to the suggested RECT.
                let suggested = unsafe { *(lparam.0 as *const RECT) };
                self.on_dpi_changed(u32::from(hiword(wparam.0)), &suggested);
                Some(LRESULT(0))
            }
            WM_SETCURSOR if wparam.0 == hwnd.0 as usize && self.on_set_cursor(lparam) => {
                Some(LRESULT(1))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point_from_lparam(lparam.0);
                self.on_mouse_down(x, y);
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let (x, y) = point_from_lparam(lparam.0);
                self.on_mouse_move(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP | WM_CAPTURECHANGED => {
                self.end_drag();
                Some(LRESULT(0))
            }
            WM_COMMAND => {
                self.on_command(loword(wparam.0));
                Some(LRESULT(0))
            }
            WM_NOTIFY => {
                // SAFETY: for WM_NOTIFY, lparam points to an NMHDR, and for
                // tree view notifications to the larger NMTREEVIEWW.
                let header = unsafe { &*(lparam.0 as *const NMHDR) };
                if header.hwndFrom == self.controls.get().tree && !self.clearing.get() {
                    match header.code {
                        TVN_ITEMEXPANDINGW => {
                            let info = unsafe { &*(lparam.0 as *const NMTREEVIEWW) };
                            self.on_item_expanding(info);
                        }
                        TVN_SELCHANGEDW => self.show_details(),
                        NM_SETFOCUS => self.diagnostics_focused.set(false),
                        _ => {}
                    }
                } else if header.hwndFrom == self.controls.get().diagnostics.list {
                    self.on_diagnostics_notify(header.code, lparam);
                }
                Some(LRESULT(0))
            }
            WM_CTLCOLORSTATIC if HWND(lparam.0 as *mut _) == self.controls.get().description => {
                // A read-only edit box is grey by default; the description
                // reads better on the normal window background.
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
            WM_DROPFILES => {
                self.on_drop_files(HDROP(wparam.0 as *mut _));
                Some(LRESULT(0))
            }
            WM_ACTIVATEAPP => {
                if wparam.0 != 0 {
                    self.on_activated();
                }
                None
            }
            WM_APP_LOADED => {
                // SAFETY: the loading thread posted a Box<Loaded> as a raw
                // pointer and gave up ownership; we take it back exactly once.
                let loaded = unsafe { Box::from_raw(lparam.0 as *mut Loaded) };
                self.on_loaded(*loaded);
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                // SAFETY: closing our own window.
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                Some(LRESULT(0))
            }
            WM_DESTROY => {
                // SAFETY: ends the message loop.
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

/// The menu bar: File, Edit and Help.
fn menu_bar() -> windows::core::Result<HMENU> {
    // SAFETY: building menus; the strings are copied by AppendMenuW.
    unsafe {
        let add = |menu: HMENU, id: u16, text: &str| {
            AppendMenuW(menu, MF_STRING, usize::from(id), &HSTRING::from(text))
        };
        let file = CreatePopupMenu()?;
        add(file, id::RELOAD, "&Reload MIBs\tF5")?;
        add(file, id::SETTINGS, "&Settings\u{2026}")?;
        add(file, id::EXPORT, "&Export diagnostics\u{2026}\tCtrl+E")?;
        AppendMenuW(file, MF_SEPARATOR, 0, None)?;
        add(file, id::EXIT, "E&xit")?;
        let edit = CreatePopupMenu()?;
        add(edit, id::FIND, "&Find\tCtrl+F")?;
        add(edit, id::FIND_NEXT, "Find &next\tF3")?;
        add(edit, id::FIND_PREVIOUS, "Find &previous\tShift+F3")?;
        AppendMenuW(edit, MF_SEPARATOR, 0, None)?;
        add(edit, id::SHOW_SOURCE, "Show &source of diagnostic\tEnter")?;
        add(edit, id::WHY, "&Why this symbol?\tCtrl+W")?;
        let help = CreatePopupMenu()?;
        add(help, id::ABOUT, "&About FeroSNMP")?;
        let bar = CreateMenu()?;
        for (menu, title) in [(file, "&File"), (edit, "&Edit"), (help, "&Help")] {
            AppendMenuW(bar, MF_POPUP, menu.0 as usize, &HSTRING::from(title))?;
        }
        Ok(bar)
    }
}

/// The file name of a path, for window titles.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What the source viewer shows for a diagnostic: the message and hint
/// above, the file below with the place highlighted.
fn source_content(set: Option<&MibSet>, diagnostic: &Diagnostic) -> Content {
    let mut header = format!("{}: ", diagnostic.severity);
    if let Some(module) = &diagnostic.module {
        header.push_str(module);
        header.push_str(": ");
    }
    header.push_str(&format!("{} [{}]", diagnostic.message, diagnostic.code));
    let Some(location) = &diagnostic.location else {
        if let Some(hint) = &diagnostic.hint {
            header.push_str(&format!("\nHint: {hint}"));
        }
        return Content {
            title: "Diagnostic \u{2014} FeroSNMP".to_string(),
            header,
            rtf: rich_text::plain_rtf("This problem does not point at a place in a file."),
            caret: None,
        };
    };
    header.push_str(&format!(
        "\n{}, line {}, column {}",
        location.file.display(),
        location.line,
        location.column
    ));
    if let Some(hint) = &diagnostic.hint {
        header.push_str(&format!("\nHint: {hint}"));
    }
    let title = format!("{} \u{2014} FeroSNMP", file_name(&location.file));

    // The text as it was loaded; a file outside the set (none so far) is
    // read from disk.
    let view = set
        .and_then(|set| set.source_view(&location.file))
        .or_else(|| SourceView::read(&location.file).ok());
    let Some(view) = view else {
        return Content {
            title,
            header,
            rtf: rich_text::plain_rtf(&format!(
                "The text of {} is not available.",
                location.file.display()
            )),
            caret: None,
        };
    };
    let start = view.offset(location.line, location.column);
    let end = view
        .offset(location.end_line, location.end_column)
        .filter(|&end| start.is_some_and(|start| end > start))
        // An empty range: mark the rest of the line.
        .or_else(|| {
            let start = start?;
            Some(
                start
                    + view.text[start..]
                        .find(['\r', '\n'])
                        .unwrap_or(view.text.len() - start),
            )
        });
    let highlight = start.zip(end).map(|(start, end)| start..end);
    let caret = start.map(|start| rich_text::position(&view.text, start));
    Content {
        title,
        header,
        rtf: rich_text::source_rtf(&view, highlight),
        caret,
    }
}
