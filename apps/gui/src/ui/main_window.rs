//! The main window: menu, toolbar with search box and query controls, MIB
//! tree, details pane, the Results, Table, Log and Diagnostics tabs, and
//! status bar.
//!
//! ```text
//! +-----------------------------------------------------------+
//! | File  Edit  Query  View  Help                             |
//! | [Reload] [Settings]   Search [_____________________]      |  rebar
//! | Agent [lab v]  Object [sysDescr.0__]  [Get] [Next] ...    |
//! +---------------------+-+-----------------------------------+
//! | v iso(1)            | | Property   | Value                |
//! |   v org(3)          | | OID        | 1.3.6.1.2.1.1.1      |  properties
//! |     > dod(6)        | | ...        |                      |
//! |                     | +-----------------------------------+  <- splitter
//! |                     | | A textual description of ...      |  description
//! +---------------------+-+-----------------------------------+  <- splitter
//! | [Results (42)] [Table] [Log] [Diagnostics (12 errors, ...)]|
//! | Name                   | Value            | Type          |  results
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
//!
//! Queries run on the SNMP engine's thread ([`QueryWorker`]); their results
//! arrive as `WM_APP_QUERY` messages and are appended to the Results list
//! (and for Table, to the Table list) while the window stays responsive.
//! Set value opens the [`set_dialog`], and sends what it builds the same
//! way.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOW, GetSysColor, GetSysColorBrush, HDC, InvalidateRect, ScreenToClient, SetBkColor,
    UpdateWindow,
};
use windows::Win32::UI::Controls::{
    HTREEITEM, LVHITTESTINFO, LVIS_SELECTED, LVM_SETEXTENDEDLISTVIEWSTYLE, LVM_SUBITEMHITTEST,
    LVN_GETDISPINFOW, LVN_ITEMACTIVATE, LVN_ITEMCHANGED, LVN_KEYDOWN, LVS_EX_DOUBLEBUFFER,
    LVS_EX_FULLROWSELECT, LVS_EX_LABELTIP, LVS_NOSORTHEADER, LVS_REPORT, LVS_SHOWSELALWAYS,
    LVS_SINGLESEL, NM_CLICK, NM_SETFOCUS, NMHDR, NMITEMACTIVATE, NMLISTVIEW, NMLVDISPINFOW,
    NMLVKEYDOWN, NMTREEVIEWW, SB_SETPARTS, SB_SETTEXTW, SBARS_SIZEGRIP, STATUSCLASSNAMEW,
    SetWindowTheme, TCN_SELCHANGE, TVE_EXPAND, TVI_ROOT, TVIS_EXPANDEDONCE, TVM_SETEXTENDEDSTYLE,
    TVM_SETIMAGELIST, TVN_ITEMEXPANDINGW, TVN_SELCHANGEDW, TVS_EX_DOUBLEBUFFER, TVS_HASBUTTONS,
    TVS_LINESATROOT, TVS_SHOWSELALWAYS, TVSIL_NORMAL, WC_LISTVIEWW, WC_TREEVIEWW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL,
};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuItem, CreateMenu, CreatePopupMenu, CreateWindowExW, DestroyMenu,
    DestroyWindow, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, GetClientRect, GetCursorPos, GetMenu,
    GetParent, GetWindowRect, HMENU, HTCLIENT, IDC_SIZENS, IDC_SIZEWE, KillTimer, LoadCursorW,
    MB_ICONINFORMATION, MB_OK, MF_BYCOMMAND, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, MF_UNCHECKED, MINMAXINFO, MessageBoxW, PostMessageW, PostQuitMessage, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOZORDER, SendMessageW, SetCursor, SetTimer, SetWindowPos,
    SetWindowTextW, ShowWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_ACTIVATEAPP, WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_COPY,
    WM_CREATE, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DROPFILES, WM_GETMINMAXINFO,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NOTIFY, WM_SETCURSOR, WM_SETREDRAW,
    WM_SETTINGCHANGE, WM_SIZE, WM_TIMER, WS_CHILD, WS_CLIPCHILDREN, WS_EX_CLIENTEDGE,
    WS_EX_CONTROLPARENT, WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, w};

use mib_core::{Diagnostic, MibSet, NodeId, NodeKind, Oid, SourceView};
use snmp_core::{Error, Event, ObjectError, Operation, Value};

use fero_win32::font::Font;
use fero_win32::shell::{self, FileType};
use fero_win32::splitter::{Orientation, Pane, Rect, Splitter};
use fero_win32::text::window_text;
use fero_win32::window::{self, CreateOptions, Window, instance};
use fero_win32::{clipboard, dpi};
use fero_win32::{hiword, loword, point_from_lparam, theme};

use crate::diagnostics::{DiagnosticList, ExportFormat, Field};
use crate::file_stamps::FileStamps;
use crate::find::Matches;
use crate::loading::{self, Loaded};
use crate::oplog::{self, LogColumn, OpLog};
use crate::query::{Query, QueryWorker};
use crate::results::{Column, ResultList, count_text};
use crate::settings::Settings;
use crate::table::{TableList, find_table};
use crate::targets::{self, Agent};
use crate::{VERSION, diagnostics, node_info, rich_text};

use super::bottom_panel::{self, BottomPanel, Tab};
use super::icons::{NodeIcons, image_for};
use super::text_window::{Content, TextWindow};
use super::toolbar::Toolbar;
use super::{
    WM_APP_LOADED, WM_APP_QUERY, agents_dialog, credentials, details, error_box, id, send,
    set_dialog, settings_dialog, tree,
};
use set_dialog::SetRequest;

/// Sizes at 96 DPI.
const WINDOW_SIZE: (i32, i32) = (1100, 720);
const MIN_SIZE: (i32, i32) = (520, 360);
const TREE_WIDTH: i32 = 380;
const PROPERTIES_HEIGHT: i32 = 280;
const BOTTOM_HEIGHT: i32 = 250;
const SPLITTER: i32 = 5;
const MIN_PANE: i32 = 80;
const COUNTS_WIDTH: i32 = 330;

/// How often a streaming query updates the window at most. Posted messages
/// come before painting in a Win32 message queue, so taking results as fast
/// as they arrive would leave no time to paint them.
const QUERY_REFRESH: Duration = Duration::from_millis(50);
/// The timer that takes the results held back by [`QUERY_REFRESH`].
const QUERY_TIMER: usize = 1;

/// The window handles of the controls in the window.
#[derive(Debug, Clone, Copy, Default)]
struct Controls {
    tree: HWND,
    properties: HWND,
    description: HWND,
    status: HWND,
    panel: BottomPanel,
}

/// The part of the window that last had the keyboard focus, which decides
/// what Ctrl+C copies, what Ctrl+F searches and what "Why this symbol?"
/// explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Tree,
    Properties,
    Results,
    Table,
    Log,
    Diagnostics,
}

/// The query buttons, and Set value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryKind {
    Get,
    Next,
    Bulk,
    Walk,
    /// A walk of a table's rows, shown in the Table tab.
    Table,
    /// A SET of one value, built by the Set value dialog.
    Set,
}

impl QueryKind {
    fn name(self) -> &'static str {
        match self {
            QueryKind::Get => "Get",
            QueryKind::Next => "Get next",
            QueryKind::Bulk => "Get bulk",
            QueryKind::Walk => "Walk",
            QueryKind::Table => "Table",
            QueryKind::Set => "Set",
        }
    }
}

/// A query as typed: what F5 repeats (except a Set).
#[derive(Debug, Clone)]
struct Request {
    kind: QueryKind,
    agent: String,
    object: String,
    /// For a Set: the value to send.
    set: Option<SetRequest>,
}

/// The query whose results the Results list shows.
#[derive(Debug)]
struct Running {
    /// Its number; results of older queries are ignored.
    number: u64,
    /// `None` once it has finished.
    query: Option<Query>,
    kind: QueryKind,
    /// What was queried, for the status bar: `Walk of IF-MIB::ifTable`.
    what: String,
    /// The agent, as `host:port`.
    agent: String,
    started: Instant,
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
    /// The part of the window that last had the focus.
    pane: Cell<Focus>,
    /// Files and folders dropped on the window, searched before the ones in
    /// the settings, for this session only.
    dropped: RefCell<Vec<PathBuf>>,
    /// The loaded files and their modification times, to notice edits.
    stamps: RefCell<FileStamps>,

    /// The SNMP engine's thread, started with the window.
    worker: RefCell<Option<QueryWorker>>,
    /// Why the engine could not be started, if it could not.
    worker_error: RefCell<Option<String>>,
    /// The current (or last) query.
    running: RefCell<Option<Running>>,
    /// Number of the most recent query.
    query_number: Cell<u64>,
    /// The last query started, for F5.
    last_request: RefCell<Option<Request>>,
    /// The Results tab.
    results: RefCell<ResultList>,
    /// The Table tab.
    table: RefCell<TableList>,
    /// The column of the Table cell last right-clicked or activated.
    table_column: Cell<usize>,
    /// The Log tab.
    log: RefCell<OpLog>,
    /// The column of the Results cell last right-clicked, for "Copy cell".
    clicked_column: Cell<usize>,
    /// When results were last taken from the query.
    last_take: Cell<Option<Instant>>,
    /// True while [`QUERY_TIMER`] runs.
    timer_set: Cell<bool>,
}

impl MainWindow {
    /// Read the settings and create the (still hidden) window.
    pub fn create() -> windows::core::Result<Rc<MainWindow>> {
        let settings_path = Settings::default_path();
        let (mut settings, settings_error) = match settings_path.as_deref().map(Settings::load) {
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
        credentials::load_passwords(&mut settings.targets);

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
                    dpi::scale(BOTTOM_HEIGHT, dpi),
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
            pane: Cell::new(Focus::Tree),
            dropped: RefCell::new(Vec::new()),
            stamps: RefCell::new(FileStamps::default()),
            worker: RefCell::new(None),
            worker_error: RefCell::new(None),
            running: RefCell::new(None),
            query_number: Cell::new(0),
            last_request: RefCell::new(None),
            results: RefCell::new(ResultList::default()),
            table: RefCell::new(TableList::default()),
            table_column: Cell::new(0),
            log: RefCell::new(OpLog::default()),
            clicked_column: Cell::new(0),
            last_take: Cell::new(None),
            timer_set: Cell::new(false),
        });
        match QueryWorker::start() {
            Ok(worker) => *window.worker.borrow_mut() = Some(worker),
            Err(error) => {
                *window.worker_error.borrow_mut() =
                    Some(format!("The SNMP engine could not be started: {error}"))
            }
        }

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
        let panel = BottomPanel::create(hwnd, dpi)?;

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
            panel,
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
            controls.panel.tabs,
            controls.panel.results,
            controls.panel.table,
            controls.panel.log,
            controls.panel.diagnostics,
        ] {
            font.apply(control);
        }

        // The rebar does not adapt its bands to a new DPI, so it is simply
        // made again; only the texts typed in it need carrying over.
        let texts = self.toolbar.get().map(|old| {
            let texts = (
                window_text(old.search),
                window_text(old.agent),
                window_text(old.object),
            );
            old.destroy();
            texts
        });
        let toolbar = Toolbar::create(self.hwnd(), dpi, &font)?;
        let (search, agent, object) = texts.unwrap_or_else(|| {
            (
                String::new(),
                self.settings.borrow().agent.clone(),
                String::new(),
            )
        });
        // SAFETY: setting the text of our own controls.
        unsafe {
            let _ = SetWindowTextW(toolbar.search, &HSTRING::from(search));
            let _ = SetWindowTextW(toolbar.object, &HSTRING::from(object));
        }
        self.toolbar.set(Some(toolbar));
        self.fill_agents(&agent);
        toolbar.set_raw(self.results.borrow().raw());
        toolbar.set_running(self.is_running());

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
        let headers = self.table.borrow().headers();
        controls.panel.size_columns(dpi, &headers);
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
        controls.panel.place(bottom);
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
        self.results.borrow_mut().set_mibs(Some(set.clone()));
        self.table.borrow_mut().set_mibs(Some(set.clone()));
        *self.mibs.borrow_mut() = Some(set);
        // The names in the Results and Table lists may have changed.
        // SAFETY: repainting our own controls.
        unsafe {
            let panel = self.controls.get().panel;
            let _ = InvalidateRect(Some(panel.results), None, true);
            let _ = InvalidateRect(Some(panel.table), None, true);
        }

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
        let panel = self.controls.get().panel;
        *self.diagnostics.borrow_mut() = list;
        self.source_row.set(None);
        panel.show_diagnostics(&self.diagnostics.borrow());
        let viewer_open = self
            .source_window
            .borrow()
            .as_ref()
            .is_some_and(|window| window.is_visible());
        if viewer_open {
            match row {
                Some(row) => {
                    bottom_panel::select(panel.diagnostics, Some(row));
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
            let from_diagnostic = (self.pane.get() == Focus::Diagnostics)
                .then(|| {
                    let row = bottom_panel::selected(self.controls.get().panel.diagnostics)?;
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
        let list = self.controls.get().panel.diagnostics;
        match code {
            LVN_GETDISPINFOW => {
                // SAFETY: for LVN_GETDISPINFOW, lparam points to an
                // NMLVDISPINFOW whose text buffer we fill.
                let info = unsafe { &mut *(lparam.0 as *mut NMLVDISPINFOW) };
                if let Some((row, column)) = bottom_panel::asked_cell(info)
                    && let Some(&field) = Field::ALL.get(column)
                {
                    let text = self.diagnostics.borrow().cell(row, field);
                    bottom_panel::fill_cell(info, &text);
                }
            }
            NM_SETFOCUS => self.pane.set(Focus::Diagnostics),
            // A click opens the source without taking the focus from the
            // list; a double click or Enter (LVN_ITEMACTIVATE) brings the
            // viewer to the front.
            NM_CLICK => {
                if let Some(row) = bottom_panel::selected(list) {
                    self.show_source(row, false);
                }
            }
            LVN_ITEMACTIVATE => {
                if let Some(row) = bottom_panel::selected(list) {
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
        if matches!(self.pane.get(), Focus::Results | Focus::Table) {
            self.find_in_results(&query, backwards);
            return;
        }
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
            id::ENTER if self.in_query_boxes(focus) => self.run_again(),
            id::ENTER | id::SHOW_SOURCE if focus == controls.panel.diagnostics => {
                if let Some(row) = bottom_panel::selected(controls.panel.diagnostics) {
                    self.show_source(row, true);
                }
            }
            id::SHOW_SOURCE => self.set_status("Select a diagnostic in the Diagnostics tab first."),
            id::WHY => self.explain_symbol(),
            id::EXPORT => self.export_diagnostics(),
            id::ESCAPE if Some(focus) == search => {
                let back = match self.pane.get() {
                    Focus::Results => controls.panel.results,
                    Focus::Table => controls.panel.table,
                    _ => controls.tree,
                };
                // SAFETY: moving the focus between our own controls.
                unsafe {
                    let _ = SetFocus(Some(back));
                }
            }
            id::ESCAPE | id::CANCEL => self.cancel_query(),
            id::GET => self.run(QueryKind::Get),
            id::NEXT => self.run(QueryKind::Next),
            id::BULK => self.run(QueryKind::Bulk),
            id::WALK => self.run(QueryKind::Walk),
            id::TABLE_QUERY => self.run(QueryKind::Table),
            id::SET_VALUE => self.set_value_of(focus),
            id::REPEAT => self.repeat(),
            id::AGENTS => self.edit_agents(),
            id::RAW => self.toggle_raw(),
            id::COPY => self.copy(focus, None),
            id::COPY_CELL => self.copy(controls.panel.results, Some(self.clicked_column.get())),
            id::COPY_OID => {
                let rows = bottom_panel::all_selected(controls.panel.results);
                let text = self.results.borrow().oids(rows);
                self.put_on_clipboard(&text);
            }
            id::SHOW_IN_TREE => self.show_result_in_tree(),
            id::CLEAR_RESULTS => self.clear_results(),
            id::EXPORT_RESULTS => self.export_results(),
            id::FIND => {
                if let Some(search) = search {
                    let hint = if matches!(self.pane.get(), Focus::Results | Focus::Table) {
                        "Text in the results (Ctrl+F)"
                    } else {
                        "Name, OID or pattern such as if*Octets (Ctrl+F)"
                    };
                    let hint = HSTRING::from(hint);
                    // SAFETY: plain control messages; the hint lives until
                    // the call returns.
                    unsafe {
                        SendMessageW(
                            search,
                            windows::Win32::UI::Controls::EM_SETCUEBANNER,
                            Some(WPARAM(1)),
                            Some(LPARAM(hint.as_ptr() as isize)),
                        );
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

    // ----- queries -----

    /// Fill the Agent box's list with the saved agents and set its text.
    fn fill_agents(&self, text: &str) {
        let Some(toolbar) = self.toolbar.get() else {
            return;
        };
        let names: Vec<String> = self
            .settings
            .borrow()
            .targets
            .iter()
            .map(|target| target.name.clone())
            .collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        toolbar.set_agents(&names, text);
    }

    fn is_running(&self) -> bool {
        self.running
            .borrow()
            .as_ref()
            .is_some_and(|running| running.query.is_some())
    }

    /// True if `focus` is the Agent or the Object box. The typing part of
    /// the Agent box is an edit control inside the combo box.
    fn in_query_boxes(&self, focus: HWND) -> bool {
        let Some(toolbar) = self.toolbar.get() else {
            return false;
        };
        // SAFETY: GetParent only reads the window tree.
        let parent = unsafe { GetParent(focus).unwrap_or_default() };
        focus == toolbar.object || focus == toolbar.agent || parent == toolbar.agent
    }

    /// Put the node selected in the tree in the Object box: a scalar with
    /// its instance (`sysDescr.0`), anything else by name, ready for a walk.
    fn fill_object(&self) {
        let Some(toolbar) = self.toolbar.get() else {
            return;
        };
        let text = {
            let mibs = self.mibs.borrow();
            let (Some(set), Some(id)) = (mibs.as_ref(), self.selected_node()) else {
                return;
            };
            let node = set.node(id);
            let name = set.format_oid(&node.oid());
            if node.kind() == NodeKind::Scalar {
                format!("{name}.0")
            } else {
                name
            }
        };
        // SAFETY: setting the text of our own control.
        unsafe {
            let _ = SetWindowTextW(toolbar.object, &HSTRING::from(text));
        }
    }

    /// A query button: run it with what the Agent and Object boxes say.
    fn run(&self, kind: QueryKind) {
        let Some(toolbar) = self.toolbar.get() else {
            return;
        };
        self.start_query(Request {
            kind,
            agent: window_text(toolbar.agent).trim().to_string(),
            object: window_text(toolbar.object).trim().to_string(),
            set: None,
        });
    }

    /// Enter in the Agent or Object box: the last kind of query (Get at
    /// first) with what the boxes say now.
    fn run_again(&self) {
        let kind = self
            .last_request
            .borrow()
            .as_ref()
            .map_or(QueryKind::Get, |request| request.kind);
        self.run(kind);
    }

    /// F5: the last query again, exactly as it was.
    fn repeat(&self) {
        let last = self.last_request.borrow().clone();
        match last {
            Some(request) => self.start_query(request),
            None => self.set_status(
                "Nothing to repeat yet: run a query with Get, Next, Bulk, Walk or Table first.",
            ),
        }
    }

    /// Tell the user what is wrong with the Agent or Object box, and put the
    /// cursor there.
    fn input_problem(&self, field: HWND, message: &str) {
        error_box(Some(self.hwnd()), message);
        // SAFETY: moving the focus to our own control.
        unsafe {
            let _ = SetFocus(Some(field));
        }
    }

    fn start_query(&self, request: Request) {
        let Some(toolbar) = self.toolbar.get() else {
            return;
        };
        let agent = {
            let settings = self.settings.borrow();
            targets::pick(&request.agent, &settings.targets)
        };
        let agent: Agent = match agent {
            Ok(agent) => agent,
            Err(message) => return self.input_problem(toolbar.agent, &message),
        };
        let resolved = {
            let mibs = self.mibs.borrow();
            match (request.kind, mibs.as_ref(), &request.set) {
                (QueryKind::Set, _, Some(set)) => {
                    let oid = set.varbind.oid.clone();
                    let name = mibs
                        .as_ref()
                        .map_or_else(|| oid.to_string(), |mibs| mibs.format_oid(&oid));
                    Ok((oid, name, Some(set.message.clone()), None))
                }
                (QueryKind::Table, Some(set), _) => find_table(set, &request.object).map(|table| {
                    let what = set.format_oid(&table.oid);
                    let note = table.augments.as_ref().map(|augments| {
                        format!(
                            "{} AUGMENTS {augments}: its rows have the same INDEX",
                            table.name
                        )
                    });
                    (table.row_oid.clone(), what, note, Some(table))
                }),
                (QueryKind::Table, None, _) => Err(
                    "The MIBs are still loading, so the table cannot be looked up yet. \
                     Wait a moment and try again."
                        .to_string(),
                ),
                (kind, mibs, _) => resolve(mibs, &request.object, kind)
                    .map(|(oid, what, note)| (oid, what, note, None)),
            }
        };
        let (oid, what, note, table) = match resolved {
            Ok(resolved) => resolved,
            Err(message) => return self.input_problem(toolbar.object, &message),
        };
        let operation = match request.kind {
            QueryKind::Get => Operation::Get(vec![oid.clone()]),
            QueryKind::Next => Operation::GetNext(vec![oid.clone()]),
            QueryKind::Bulk => Operation::GetBulk {
                oids: vec![oid.clone()],
                non_repeaters: 0,
                max_repetitions: agent.max_repetitions,
            },
            QueryKind::Walk | QueryKind::Table => Operation::Walk {
                root: oid.clone(),
                max_repetitions: agent.max_repetitions,
                bulk: true,
            },
            QueryKind::Set => match &request.set {
                Some(set) => Operation::Set(vec![set.varbind.clone()]),
                None => return,
            },
        };

        // A new query replaces the one running; dropping it cancels it.
        let previous = self.running.borrow_mut().take();
        if let Some(previous) = previous
            && previous.query.is_some()
        {
            self.log_line(
                &previous.agent,
                format!(
                    "Stopped {} of {} for a new query, after {} values",
                    previous.kind.name(),
                    previous.what,
                    count_text(self.results.borrow().len())
                ),
            );
        }

        let number = self.query_number.get() + 1;
        self.query_number.set(number);
        // A window handle is not `Send`, but the number inside it is; the
        // engine's thread only uses it to post messages back.
        let hwnd = self.hwnd().0 as isize;
        let notify = move || {
            // SAFETY: PostMessageW may be called from any thread; a post to
            // a window that is gone fails harmlessly.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd as *mut _)),
                    WM_APP_QUERY,
                    WPARAM(number as usize),
                    LPARAM(0),
                );
            }
        };
        let submitted = match self.worker.borrow().as_ref() {
            Some(worker) => worker
                .submit(agent.target.clone(), operation, notify)
                .map_err(|error| error.to_string()),
            None => Err(self
                .worker_error
                .borrow()
                .clone()
                .unwrap_or_else(|| "The SNMP engine is not running.".to_string())),
        };
        let query = match submitted {
            Ok(query) => query,
            Err(message) => {
                error_box(Some(self.hwnd()), &message);
                return;
            }
        };

        let target = agent.target.to_string();
        self.results.borrow_mut().clear();
        let panel = self.controls.get().panel;
        bottom_panel::set_count(panel.results, 0, false);
        panel.set_title(Tab::Results, "Results");
        let tab = if let Some(table) = table {
            let headers = {
                let mut list = self.table.borrow_mut();
                list.start(table);
                list.headers()
            };
            panel.set_table_columns(&headers, self.dpi.get());
            bottom_panel::set_count(panel.table, 0, false);
            panel.set_title(Tab::Table, &self.table.borrow().title());
            Tab::Table
        } else if request.kind == QueryKind::Set && panel.current_tab() == Tab::Table {
            // A cell of the table was set: its new value shows there.
            Tab::Table
        } else {
            Tab::Results
        };
        if panel.current_tab() != tab {
            panel.select_tab(tab);
        }
        let mut started = format!(
            "{} {what} ({oid}), {}",
            request.kind.name(),
            agent.target.version
        );
        if matches!(
            request.kind,
            QueryKind::Bulk | QueryKind::Walk | QueryKind::Table
        ) {
            started.push_str(&format!(", max-repetitions {}", agent.max_repetitions));
        }
        match &agent.saved {
            Some(name) => started.push_str(&format!(", saved agent \u{201c}{name}\u{201d}")),
            None => started.push_str(", community public (not a saved agent)"),
        }
        self.log_line(&target, started);
        if let Some(note) = note {
            let label = if request.kind == QueryKind::Set {
                "Value"
            } else {
                "Note"
            };
            self.log_line(&target, format!("{label}: {note}"));
        }
        self.refresh_log();
        *self.running.borrow_mut() = Some(Running {
            number,
            query: Some(query),
            kind: request.kind,
            what: what.clone(),
            agent: target.clone(),
            started: Instant::now(),
        });
        toolbar.set_running(true);
        self.set_status(&format!(
            "{} of {what} on {target}\u{2026} (Esc to cancel)",
            request.kind.name()
        ));

        // Remember the agent for next time.
        let changed = self.settings.borrow().agent != request.agent;
        if changed {
            self.settings.borrow_mut().agent = request.agent.clone();
            self.save_settings();
        }
        // A Set is not repeated by F5 or Enter: a value is sent only from
        // the dialog, where it is checked.
        if request.kind != QueryKind::Set {
            *self.last_request.borrow_mut() = Some(request);
        }
    }

    /// WM_APP_QUERY: events of a query are waiting. Take them at once,
    /// unless results were taken less than [`QUERY_REFRESH`] ago: then a
    /// timer takes them a little later, and in the meantime the window
    /// paints. The engine posts no more messages until they are taken.
    fn on_query_message(&self, number: u64) {
        if self.running.borrow().as_ref().map(|r| r.number) != Some(number) {
            return; // an older query: its results are no longer shown
        }
        let since = self.last_take.get().map(|at| at.elapsed());
        match since {
            Some(since) if since < QUERY_REFRESH => {
                if !self.timer_set.replace(true) {
                    let wait = (QUERY_REFRESH - since).as_millis().max(1) as u32;
                    // SAFETY: a timer on our own window; it arrives as
                    // WM_TIMER.
                    unsafe {
                        SetTimer(Some(self.hwnd()), QUERY_TIMER, wait, None);
                    }
                }
            }
            _ => self.on_query_events(number),
        }
    }

    /// WM_TIMER: take the results held back.
    fn on_query_timer(&self) {
        // SAFETY: stopping our own timer.
        unsafe {
            let _ = KillTimer(Some(self.hwnd()), QUERY_TIMER);
        }
        self.timer_set.set(false);
        let number = self.running.borrow().as_ref().map(|r| r.number);
        if let Some(number) = number {
            self.on_query_events(number);
        }
    }

    /// Take the events that are waiting and show them.
    fn on_query_events(&self, number: u64) {
        self.last_take.set(Some(Instant::now()));
        let (events, agent, kind) = {
            let running = self.running.borrow();
            match running.as_ref() {
                Some(running) if running.number == number => (
                    running
                        .query
                        .as_ref()
                        .map(|query| query.take())
                        .unwrap_or_default(),
                    running.agent.clone(),
                    running.kind,
                ),
                // An older query: its results are no longer shown.
                _ => return,
            }
        };
        if events.is_empty() {
            return;
        }
        let mut end = None;
        for event in events {
            match event {
                Event::Varbinds(varbinds) => {
                    if kind == QueryKind::Table {
                        self.table.borrow_mut().extend(varbinds.iter().cloned());
                    }
                    self.results.borrow_mut().extend(varbinds);
                }
                Event::Log(entry) => self.log_line(&agent, oplog::request_text(&entry)),
                Event::Notice(text) => self.log_line(&agent, format!("Note: {text}")),
                Event::Engine(info) => self.log_line(&agent, oplog::engine_text(&info)),
                Event::Finished(_) | Event::Failed(_) => end = Some(event),
            }
        }
        let count = self.results.borrow().len();
        let panel = self.controls.get().panel;
        bottom_panel::set_count(panel.results, count, true);
        panel.set_title(Tab::Results, &format!("Results ({})", count_text(count)));
        if kind == QueryKind::Table {
            // Rows of a table walk arrive column by column, so every row can
            // change: repaint them all.
            let (rows, title) = {
                let table = self.table.borrow();
                (table.len(), table.title())
            };
            bottom_panel::set_count(panel.table, rows, false);
            panel.set_title(Tab::Table, &title);
        }
        match end {
            Some(event) => self.finish(event),
            None => {
                let running = self.running.borrow();
                if let Some(running) = running.as_ref() {
                    self.set_status(&format!(
                        "{} of {} on {}\u{2026} {} values so far (Esc to cancel)",
                        running.kind.name(),
                        running.what,
                        running.agent,
                        count_text(count)
                    ));
                }
            }
        }
        self.refresh_log();
    }

    /// The query ended with `event` (`Finished` or `Failed`).
    fn finish(&self, event: Event) {
        let Some((kind, what, agent, elapsed)) = self.running.borrow_mut().as_mut().map(|r| {
            r.query = None;
            (r.kind, r.what.clone(), r.agent.clone(), r.started.elapsed())
        }) else {
            return;
        };
        if let Some(toolbar) = self.toolbar.get() {
            toolbar.set_running(false);
        }
        let count = self.results.borrow().len();
        let values = match count {
            1 => "1 value".to_string(),
            n => format!("{} values", count_text(n)),
        };
        let status = match event {
            Event::Finished(summary) if kind == QueryKind::Set => {
                self.log_line(&agent, oplog::summary_text(&summary));
                self.show_set_result();
                let value = {
                    let results = self.results.borrow();
                    (0..results.len())
                        .map(|row| results.cell(row, Column::Value))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                format!("Set {what} = {value} on {agent}: done")
            }
            Event::Finished(summary) if kind == QueryKind::Table => {
                self.log_line(&agent, oplog::summary_text(&summary));
                let (rows, bad) = {
                    let table = self.table.borrow();
                    (table.len(), table.index_errors())
                };
                let mut status = match rows {
                    0 => format!("Table: the agent has no rows in {what}"),
                    1 => format!("Table {what}: 1 row from {agent}"),
                    n => format!("Table {what}: {} rows from {agent}", count_text(n)),
                };
                if rows > 0 {
                    status.push_str(&format!(" in {}", oplog::seconds(summary.elapsed)));
                }
                if rows > 0 {
                    let panel = self.controls.get().panel;
                    panel.fit_table_columns(&self.table.borrow().headers(), self.dpi.get());
                }
                if bad > 0 {
                    let note = format!(
                        "{bad} row(s) have an instance that does not fit the INDEX of {what}; \
                         they are shown as ?.suffix"
                    );
                    self.log_line(&agent, format!("Note: {note}"));
                    status.push_str(&format!(" ({note})"));
                }
                status
            }
            Event::Finished(summary) => {
                self.log_line(&agent, oplog::summary_text(&summary));
                if kind == QueryKind::Next {
                    self.step_object();
                }
                if count == 0 {
                    format!(
                        "{}: no values; the agent has nothing at or below {what}",
                        kind.name()
                    )
                } else {
                    format!(
                        "{} of {what}: {values} from {agent} in {}",
                        kind.name(),
                        oplog::seconds(summary.elapsed)
                    )
                }
            }
            Event::Failed(Error::Cancelled) => {
                self.log_line(&agent, format!("Cancelled after {values}"));
                format!("{} of {what} cancelled after {values}", kind.name())
            }
            Event::Failed(error) => {
                let text = {
                    let mibs = self.mibs.borrow();
                    describe(&error, mibs.as_ref())
                };
                if kind == QueryKind::Set {
                    self.log_line(&agent, format!("Failed: {text}"));
                    self.set_status(&format!("Set {what} failed: {text}"));
                    // The dialog is closed by now; make sure this is seen.
                    error_box(
                        Some(self.hwnd()),
                        &format!("{what} was not set.\n\n{}.", text.trim_end_matches('.')),
                    );
                    return;
                }
                self.log_line(&agent, format!("Failed after {values}: {text}"));
                let got = if count > 0 {
                    format!(" ({values} received before)")
                } else {
                    String::new()
                };
                format!(
                    "{} failed after {}{got}: {text}",
                    kind.name(),
                    oplog::seconds(elapsed)
                )
            }
            _ => return,
        };
        self.set_status(&status);
    }

    /// After a Get next, put the instance it returned in the Object box, so
    /// the next press of Next steps on from there.
    fn step_object(&self) {
        let (Some(toolbar), Some(text)) = (self.toolbar.get(), {
            let results = self.results.borrow();
            let mibs = self.mibs.borrow();
            results
                .get(results.len().wrapping_sub(1))
                .map(|last| match mibs.as_ref() {
                    Some(set) => set.format_oid(&last.oid),
                    None => last.oid.to_string(),
                })
        }) else {
            return;
        };
        // SAFETY: setting the text of our own control.
        unsafe {
            let _ = SetWindowTextW(toolbar.object, &HSTRING::from(text));
        }
    }

    /// Query > Cancel, the Cancel button and Esc.
    fn cancel_query(&self) {
        let mut running = self.running.borrow_mut();
        match running.as_mut().and_then(|running| running.query.as_mut()) {
            // The engine stops at once; its last event says so.
            Some(query) => query.cancel(),
            None => {
                drop(running);
                self.set_status("No query is running.");
            }
        }
    }

    /// File > Agents.
    fn edit_agents(&self) {
        let Some(toolbar) = self.toolbar.get() else {
            return;
        };
        let current = window_text(toolbar.agent);
        let targets = self.settings.borrow().targets.clone();
        let Some(edited) = agents_dialog::show(self.hwnd(), &targets, &current) else {
            return;
        };
        let text = edited.selected.clone().unwrap_or(current);
        if let Err(error) = credentials::save_passwords(&targets, &edited.targets) {
            error_box(
                Some(self.hwnd()),
                &format!(
                    "An SNMPv3 password could not be stored in the Windows Credential \
                     Manager, so it will be asked for again after a restart.\n\n{error}"
                ),
            );
        }
        {
            let mut settings = self.settings.borrow_mut();
            settings.targets = edited.targets;
            settings.agent = text.clone();
        }
        self.save_settings();
        self.fill_agents(&text);
    }

    fn save_settings(&self) {
        let Some(path) = &self.settings_path else {
            return;
        };
        let result = self.settings.borrow().save(path);
        if let Err(error) = result {
            error_box(
                Some(self.hwnd()),
                &format!("The settings could not be saved.\n\n{error}"),
            );
        }
    }

    /// View > Raw values: numeric OIDs, tags and bytes, or names and values.
    fn toggle_raw(&self) {
        let raw = !self.results.borrow().raw();
        self.results.borrow_mut().set_raw(raw);
        let panel = self.controls.get().panel;
        panel.set_raw_headings(raw);
        if let Some(toolbar) = self.toolbar.get() {
            toolbar.set_raw(raw);
        }
        // SAFETY: plain menu and window calls on our own window.
        unsafe {
            let flags = MF_BYCOMMAND | if raw { MF_CHECKED } else { MF_UNCHECKED };
            CheckMenuItem(GetMenu(self.hwnd()), u32::from(id::RAW), flags.0);
            let _ = InvalidateRect(Some(panel.results), None, true);
        }
    }

    /// Edit > Copy (Ctrl+C): what is selected in the control with the
    /// focus. With `column`, only that column of the selected results.
    fn copy(&self, focus: HWND, column: Option<usize>) {
        let controls = self.controls.get();
        let text = if focus == controls.panel.results {
            let rows = bottom_panel::all_selected(focus);
            let results = self.results.borrow();
            match column.and_then(|c| Column::ALL.get(c)) {
                Some(&column) => rows
                    .into_iter()
                    .map(|row| format!("{}\r\n", results.cell(row, column)))
                    .collect(),
                None => results.tab_separated(rows),
            }
        } else if focus == controls.panel.table {
            let rows = bottom_panel::all_selected(focus);
            let table = self.table.borrow();
            match column {
                Some(column) => table.column_text(rows, column),
                None => table.tab_separated(rows),
            }
        } else if focus == controls.panel.log {
            self.log
                .borrow()
                .tab_separated(bottom_panel::all_selected(focus))
        } else if focus == controls.panel.diagnostics {
            let list = self.diagnostics.borrow();
            bottom_panel::selected(focus)
                .map(|row| {
                    let cells: Vec<String> =
                        Field::ALL.iter().map(|&f| list.cell(row, f)).collect();
                    format!("{}\r\n", cells.join("\t"))
                })
                .unwrap_or_default()
        } else if focus == controls.tree || focus == controls.properties {
            let mibs = self.mibs.borrow();
            match (mibs.as_ref(), self.selected_node()) {
                (Some(set), Some(id)) if focus == controls.tree => set.node(id).oid().to_string(),
                (Some(set), Some(id)) => node_info::properties(set, set.node(id))
                    .into_iter()
                    .map(|(label, value)| format!("{label}\t{value}\r\n"))
                    .collect(),
                _ => String::new(),
            }
        } else {
            // An edit box (search, Agent, Object, description): its own copy.
            // SAFETY: WM_COPY takes no parameters.
            unsafe {
                send(focus, WM_COPY, 0, 0);
            }
            return;
        };
        self.put_on_clipboard(&text);
    }

    fn put_on_clipboard(&self, text: &str) {
        if text.is_empty() {
            self.set_status("Nothing is selected to copy.");
            return;
        }
        match clipboard::set_text(self.hwnd(), text) {
            Ok(()) => {
                let lines = text.lines().count();
                self.set_status(&if lines > 1 {
                    format!("Copied {} lines", count_text(lines))
                } else {
                    "Copied".to_string()
                });
            }
            Err(error) => error_box(
                Some(self.hwnd()),
                &format!("The clipboard could not be used.\n\n{error}"),
            ),
        }
    }

    /// Show the object of the selected result in the MIB tree.
    fn show_result_in_tree(&self) {
        let list = self.controls.get().panel.results;
        let Some(oid) = bottom_panel::selected(list)
            .and_then(|row| self.results.borrow().get(row).map(|v| v.oid.clone()))
        else {
            return;
        };
        let target = {
            let mibs = self.mibs.borrow();
            mibs.as_ref().map(|set| {
                let node = set.lookup(&oid).node;
                (!node.oid().is_empty()).then(|| node.id())
            })
        };
        match target.flatten() {
            Some(id) => {
                self.reveal(id);
                // SAFETY: moving the focus to our own control.
                unsafe {
                    let _ = SetFocus(Some(self.controls.get().tree));
                }
            }
            None => self.set_status(&format!("No loaded MIB defines {oid}")),
        }
    }

    fn clear_results(&self) {
        self.running.borrow_mut().take();
        if let Some(toolbar) = self.toolbar.get() {
            toolbar.set_running(false);
        }
        self.results.borrow_mut().clear();
        let panel = self.controls.get().panel;
        bottom_panel::set_count(panel.results, 0, false);
        panel.set_title(Tab::Results, "Results");
        self.set_status("");
    }

    /// File > Export results: save every row of the Results list as CSV,
    /// or of the Table list when that tab is shown.
    fn export_results(&self) {
        let table = self.controls.get().panel.current_tab() == Tab::Table;
        let empty = if table {
            self.table.borrow().is_empty()
        } else {
            self.results.borrow().is_empty()
        };
        if empty {
            self.set_status(if table {
                "There is no table to export yet: walk one with Table (Ctrl+T)."
            } else {
                "There are no results to export yet."
            });
            return;
        }
        let file_name = match self.table.borrow().table() {
            Some(shown) if table => format!("{}.csv", shown.name),
            _ => "results.csv".to_string(),
        };
        let types = [FileType {
            name: "CSV, for spreadsheets (*.csv)",
            pattern: "*.csv",
            extension: "csv",
        }];
        let path = match shell::save_file(Some(self.hwnd()), "Export results", &file_name, &types) {
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
        let (text, count) = if table {
            let list = self.table.borrow();
            (list.csv(), list.len())
        } else {
            let results = self.results.borrow();
            (results.csv(), results.len())
        };
        match std::fs::write(&path, text) {
            Ok(()) => self.set_status(&format!(
                "{} rows saved to {}",
                count_text(count),
                path.display()
            )),
            Err(error) => error_box(
                Some(self.hwnd()),
                &format!("{} could not be written.\n\n{error}", path.display()),
            ),
        }
    }

    /// The search box while the Results or Table list was used last: find
    /// text in that list.
    fn find_in_results(&self, query: &str, backwards: bool) {
        let panel = self.controls.get().panel;
        let found = if self.pane.get() == Focus::Table {
            let table = self.table.borrow();
            table
                .find(query, bottom_panel::selected(panel.table), backwards)
                .map(|row| (panel.table, row, table.len()))
        } else {
            let results = self.results.borrow();
            results
                .find(query, bottom_panel::selected(panel.results), backwards)
                .map(|row| (panel.results, row, results.len()))
        };
        match found {
            Some((list, row, count)) => {
                bottom_panel::select(list, Some(row));
                self.set_status(&format!(
                    "Row {} of {} (F3: next, Shift+F3: previous; click the tree to search the MIBs)",
                    count_text(row + 1),
                    count_text(count)
                ));
            }
            None if query.trim().is_empty() => self.set_status(""),
            None => self.set_status(&format!(
                "Nothing in the list matches \u{201c}{}\u{201d} (click the tree to search the MIBs)",
                query.trim()
            )),
        }
    }

    /// WM_NOTIFY from the Results list.
    fn on_results_notify(&self, code: u32, lparam: LPARAM) {
        let list = self.controls.get().panel.results;
        match code {
            LVN_GETDISPINFOW => {
                // SAFETY: for LVN_GETDISPINFOW, lparam points to an
                // NMLVDISPINFOW whose text buffer we fill.
                let info = unsafe { &mut *(lparam.0 as *mut NMLVDISPINFOW) };
                if let Some((row, column)) = bottom_panel::asked_cell(info)
                    && let Some(&column) = Column::ALL.get(column)
                {
                    let text = self.results.borrow().cell(row, column);
                    bottom_panel::fill_cell(info, &text);
                }
            }
            NM_SETFOCUS => self.pane.set(Focus::Results),
            LVN_ITEMACTIVATE => self.show_result_in_tree(),
            LVN_KEYDOWN => select_all_on_ctrl_a(list, lparam),
            _ => {}
        }
    }

    /// WM_NOTIFY from the Log list.
    fn on_log_notify(&self, code: u32, lparam: LPARAM) {
        match code {
            LVN_GETDISPINFOW => {
                // SAFETY: as in on_results_notify.
                let info = unsafe { &mut *(lparam.0 as *mut NMLVDISPINFOW) };
                if let Some((row, column)) = bottom_panel::asked_cell(info)
                    && let Some(&column) = LogColumn::ALL.get(column)
                {
                    let log = self.log.borrow();
                    bottom_panel::fill_cell(info, log.cell(row, column));
                }
            }
            NM_SETFOCUS => self.pane.set(Focus::Log),
            LVN_KEYDOWN => select_all_on_ctrl_a(self.controls.get().panel.log, lparam),
            _ => {}
        }
    }

    /// WM_CONTEXTMENU on the Results list: the right-click menu.
    fn results_menu(&self, lparam: LPARAM) {
        let list = self.controls.get().panel.results;
        let (point, column) = clicked_cell(list, lparam);
        self.clicked_column.set(column);
        let selected = bottom_panel::selected(list).is_some();
        let any = !self.results.borrow().is_empty();
        let column_title = Column::ALL
            .get(self.clicked_column.get())
            .map_or("cell", |c| c.title(self.results.borrow().raw()));
        // SAFETY: a popup menu we create, show and destroy here; the strings
        // are copied by AppendMenuW.
        let command = unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let add = |id: u16, text: &str, enabled: bool| {
                let flags = if enabled {
                    MF_STRING
                } else {
                    MF_STRING | MF_GRAYED
                };
                let _ = AppendMenuW(menu, flags, usize::from(id), &HSTRING::from(text));
            };
            add(id::COPY, "&Copy\tCtrl+C", selected);
            add(id::COPY_CELL, &format!("Copy &{column_title}"), selected);
            add(id::COPY_OID, "Copy &OID", selected);
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            add(id::SHOW_IN_TREE, "Show in MIB &tree\tEnter", selected);
            add(id::SET_VALUE, "&Set value\u{2026}\tF2", selected);
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            add(id::EXPORT_RESULTS, "&Export as CSV\u{2026}\tCtrl+S", any);
            add(id::CLEAR_RESULTS, "C&lear results", any);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                self.hwnd(),
                None,
            );
            let _ = DestroyMenu(menu);
            command.0 as u16
        };
        match command {
            0 => {}
            id::COPY => self.copy(list, None),
            id::SET_VALUE => self.set_value_of(list),
            other => self.on_command(other),
        }
    }

    /// WM_NOTIFY from the Table list.
    fn on_table_notify(&self, code: u32, lparam: LPARAM) {
        let list = self.controls.get().panel.table;
        match code {
            LVN_GETDISPINFOW => {
                // SAFETY: as in on_results_notify.
                let info = unsafe { &mut *(lparam.0 as *mut NMLVDISPINFOW) };
                if let Some((row, column)) = bottom_panel::asked_cell(info) {
                    let text = self.table.borrow().cell(row, column);
                    bottom_panel::fill_cell(info, &text);
                }
            }
            NM_SETFOCUS => self.pane.set(Focus::Table),
            LVN_ITEMACTIVATE => {
                // SAFETY: for LVN_ITEMACTIVATE, lparam points to an
                // NMITEMACTIVATE.
                let info = unsafe { &*(lparam.0 as *const NMITEMACTIVATE) };
                let column = usize::try_from(info.iSubItem).unwrap_or(0);
                // Enter gives column 0, and INDEX columns hold no value of
                // their own: then the column used last, or the first value.
                let index_count = self.table.borrow().index_count();
                if column >= index_count {
                    self.table_column.set(column);
                } else if self.table_column.get() < index_count {
                    self.table_column.set(index_count);
                }
                self.set_value_of(list);
            }
            LVN_KEYDOWN => select_all_on_ctrl_a(list, lparam),
            _ => {}
        }
    }

    /// WM_CONTEXTMENU on the Table list: the right-click menu.
    fn table_menu(&self, lparam: LPARAM) {
        let list = self.controls.get().panel.table;
        let (point, column) = clicked_cell(list, lparam);
        self.table_column.set(column);
        let (selected, any, title, settable) = {
            let table = self.table.borrow();
            let row = bottom_panel::selected(list);
            (
                row.is_some(),
                !table.is_empty(),
                table.column_name(column),
                row.and_then(|row| table.cell_target(row, column)).is_some(),
            )
        };
        let title = title.unwrap_or_else(|| "cell".to_string());
        // SAFETY: a popup menu we create, show and destroy here; the strings
        // are copied by AppendMenuW.
        let command = unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let add = |id: u16, text: &str, enabled: bool| {
                let flags = if enabled {
                    MF_STRING
                } else {
                    MF_STRING | MF_GRAYED
                };
                let _ = AppendMenuW(menu, flags, usize::from(id), &HSTRING::from(text));
            };
            add(id::COPY, "&Copy rows\tCtrl+C", selected);
            add(id::COPY_CELL, &format!("Copy &{title}"), selected);
            add(id::COPY_OID, &format!("Copy OID of {title}"), settable);
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            add(
                id::SET_VALUE,
                &format!("&Set {title}\u{2026}\tEnter"),
                settable,
            );
            add(id::SHOW_IN_TREE, &format!("Show {title} in MIB &tree"), any);
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            add(
                id::EXPORT_RESULTS,
                "&Export table as CSV\u{2026}\tCtrl+S",
                any,
            );
            add(id::CLEAR_TABLE, "C&lear table", any);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON,
                point.x,
                point.y,
                None,
                self.hwnd(),
                None,
            );
            let _ = DestroyMenu(menu);
            command.0 as u16
        };
        match command {
            0 => {}
            id::COPY => self.copy(list, None),
            id::COPY_CELL => self.copy(list, Some(column)),
            id::COPY_OID => {
                let text = {
                    let table = self.table.borrow();
                    bottom_panel::all_selected(list)
                        .into_iter()
                        .filter_map(|row| table.cell_target(row, column))
                        .map(|cell| format!("{}\r\n", cell.oid))
                        .collect::<String>()
                };
                self.put_on_clipboard(&text);
            }
            id::SET_VALUE => self.set_value_of(list),
            id::SHOW_IN_TREE => {
                let name = self.table.borrow().column_name(column);
                let target = {
                    let mibs = self.mibs.borrow();
                    let module = self
                        .table
                        .borrow()
                        .table()
                        .and_then(|table| table.module.clone());
                    mibs.as_ref().zip(name).and_then(|(set, name)| {
                        // The column's own module first: names can repeat.
                        module
                            .and_then(|module| set.resolve(&format!("{module}::{name}")))
                            .or_else(|| set.resolve(&name))
                            .map(|node| node.id())
                    })
                };
                if let Some(target) = target {
                    self.reveal(target);
                    // SAFETY: moving the focus to our own control.
                    unsafe {
                        let _ = SetFocus(Some(self.controls.get().tree));
                    }
                }
            }
            id::CLEAR_TABLE => self.clear_table(),
            other => self.on_command(other),
        }
    }

    fn clear_table(&self) {
        let is_table = self
            .running
            .borrow()
            .as_ref()
            .is_some_and(|running| running.kind == QueryKind::Table);
        if is_table {
            self.clear_results();
        }
        self.table.borrow_mut().clear();
        let panel = self.controls.get().panel;
        panel.set_table_columns(&[], self.dpi.get());
        bottom_panel::set_count(panel.table, 0, false);
        panel.set_title(Tab::Table, "Table");
    }

    /// Set value (F2, the toolbar, the context menus, or Enter on a table
    /// cell): open the Set value dialog for what `focus` shows. In the
    /// Results list, the selected result; in the Table list, the cell in
    /// the selected row and the column last clicked; elsewhere, what the
    /// Object box says.
    fn set_value_of(&self, focus: HWND) {
        let panel = self.controls.get().panel;
        let start = if focus == panel.results {
            let results = self.results.borrow();
            bottom_panel::selected(focus)
                .and_then(|row| results.get(row))
                .map(|varbind| (varbind.oid.to_string(), Some(varbind.value.clone())))
        } else if focus == panel.table {
            let Some(cell) = bottom_panel::selected(focus).and_then(|row| {
                self.table
                    .borrow()
                    .cell_target(row, self.table_column.get())
            }) else {
                self.set_status(
                    "Pick a cell to set: right-click it or double-click it (the INDEX columns \
                     are part of the row's name, not values).",
                );
                return;
            };
            Some((cell.oid.to_string(), cell.value))
        } else {
            None
        };
        let (object, current) = match start {
            Some((oid, value)) => (oid, value),
            None => {
                let Some(toolbar) = self.toolbar.get() else {
                    return;
                };
                let object = window_text(toolbar.object).trim().to_string();
                // The value from the last query, when it read this instance.
                let current = {
                    let mibs = self.mibs.borrow();
                    let results = self.results.borrow();
                    crate::set_value::SetForm::new(mibs.as_ref(), &object)
                        .oid()
                        .and_then(|oid| {
                            (0..results.len())
                                .filter_map(|row| results.get(row))
                                .find(|varbind| varbind.oid == *oid)
                                .map(|varbind| varbind.value.clone())
                        })
                };
                (object, current)
            }
        };
        self.open_set_dialog(&object, current.as_ref());
    }

    fn open_set_dialog(&self, object: &str, current: Option<&Value>) {
        let Some(toolbar) = self.toolbar.get() else {
            return;
        };
        // The dialog runs a message loop; hold no borrow while it is open.
        let mibs = self.mibs.borrow().clone();
        let Some(request) = set_dialog::show(self.hwnd(), mibs.as_ref(), object, current) else {
            return;
        };
        let object = mibs.as_ref().map_or_else(
            || request.varbind.oid.to_string(),
            |set| set.format_oid(&request.varbind.oid),
        );
        self.start_query(Request {
            kind: QueryKind::Set,
            agent: window_text(toolbar.agent).trim().to_string(),
            object,
            set: Some(request),
        });
    }

    /// After a Set: the agent answers with the values it now has; show them
    /// in the table when they are cells of it.
    fn show_set_result(&self) {
        let changed = {
            let results = self.results.borrow();
            let mut table = self.table.borrow_mut();
            let mut changed = false;
            for varbind in (0..results.len()).filter_map(|row| results.get(row)) {
                changed |= table.update(varbind);
            }
            changed
        };
        if changed {
            // SAFETY: repainting our own control.
            unsafe {
                let _ = InvalidateRect(Some(self.controls.get().panel.table), None, true);
            }
        }
    }

    /// Add a line to the Log tab; [`refresh_log`](Self::refresh_log) shows it.
    fn log_line(&self, agent: &str, text: String) {
        self.log.borrow_mut().push(local_time(), agent, text);
    }

    /// Show the lines added to the log, keeping the newest in view.
    fn refresh_log(&self) {
        let list = self.controls.get().panel.log;
        let count = self.log.borrow().len();
        // At the limit, old lines are dropped from the top: every row moves.
        let growing = count < oplog::LIMIT;
        bottom_panel::set_count(list, count, growing);
        bottom_panel::show_last(list, count);
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
                        TVN_SELCHANGEDW => {
                            self.show_details();
                            self.fill_object();
                        }
                        NM_SETFOCUS => self.pane.set(Focus::Tree),
                        _ => {}
                    }
                } else if header.hwndFrom == self.controls.get().panel.diagnostics {
                    self.on_diagnostics_notify(header.code, lparam);
                } else if header.hwndFrom == self.controls.get().panel.results {
                    self.on_results_notify(header.code, lparam);
                } else if header.hwndFrom == self.controls.get().panel.table {
                    self.on_table_notify(header.code, lparam);
                } else if header.hwndFrom == self.controls.get().panel.log {
                    self.on_log_notify(header.code, lparam);
                } else if header.hwndFrom == self.controls.get().panel.tabs
                    && header.code == TCN_SELCHANGE
                {
                    let panel = self.controls.get().panel;
                    panel.select_tab(panel.current_tab());
                } else if header.hwndFrom == self.controls.get().properties
                    && header.code == NM_SETFOCUS
                {
                    self.pane.set(Focus::Properties);
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
            WM_APP_QUERY => {
                self.on_query_message(wparam.0 as u64);
                Some(LRESULT(0))
            }
            WM_TIMER if wparam.0 == QUERY_TIMER => {
                self.on_query_timer();
                Some(LRESULT(0))
            }
            WM_CONTEXTMENU if HWND(wparam.0 as *mut _) == self.controls.get().panel.results => {
                self.results_menu(lparam);
                Some(LRESULT(0))
            }
            WM_CONTEXTMENU if HWND(wparam.0 as *mut _) == self.controls.get().panel.table => {
                self.table_menu(lparam);
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
                // Stop the query and the engine's thread.
                self.running.borrow_mut().take();
                let worker = self.worker.borrow_mut().take();
                drop(worker);
                // SAFETY: ends the message loop.
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

/// The menu bar: File, Edit, Query, View and Help.
fn menu_bar() -> windows::core::Result<HMENU> {
    // SAFETY: building menus; the strings are copied by AppendMenuW.
    unsafe {
        let add = |menu: HMENU, id: u16, text: &str| {
            AppendMenuW(menu, MF_STRING, usize::from(id), &HSTRING::from(text))
        };
        let file = CreatePopupMenu()?;
        add(file, id::RELOAD, "&Reload MIBs\tCtrl+R")?;
        add(file, id::SETTINGS, "&Settings\u{2026}")?;
        add(file, id::AGENTS, "&Agents\u{2026}")?;
        AppendMenuW(file, MF_SEPARATOR, 0, None)?;
        add(
            file,
            id::EXPORT_RESULTS,
            "Export res&ults as CSV\u{2026}\tCtrl+S",
        )?;
        add(file, id::EXPORT, "&Export diagnostics\u{2026}\tCtrl+E")?;
        AppendMenuW(file, MF_SEPARATOR, 0, None)?;
        add(file, id::EXIT, "E&xit")?;
        let edit = CreatePopupMenu()?;
        add(edit, id::COPY, "&Copy\tCtrl+C")?;
        AppendMenuW(edit, MF_SEPARATOR, 0, None)?;
        add(edit, id::FIND, "&Find\tCtrl+F")?;
        add(edit, id::FIND_NEXT, "Find &next\tF3")?;
        add(edit, id::FIND_PREVIOUS, "Find &previous\tShift+F3")?;
        AppendMenuW(edit, MF_SEPARATOR, 0, None)?;
        add(edit, id::SHOW_SOURCE, "Show &source of diagnostic\tEnter")?;
        add(edit, id::WHY, "&Why this symbol?\tCtrl+W")?;
        let query = CreatePopupMenu()?;
        add(query, id::GET, "&Get")?;
        add(query, id::NEXT, "Get &next")?;
        add(query, id::BULK, "Get &bulk")?;
        add(query, id::WALK, "&Walk")?;
        add(query, id::TABLE_QUERY, "&Table\tCtrl+T")?;
        AppendMenuW(query, MF_SEPARATOR, 0, None)?;
        add(query, id::SET_VALUE, "&Set value\u{2026}\tF2")?;
        AppendMenuW(query, MF_SEPARATOR, 0, None)?;
        add(query, id::REPEAT, "&Repeat last query\tF5")?;
        add(query, id::CANCEL, "&Cancel\tEsc")?;
        let view = CreatePopupMenu()?;
        add(view, id::RAW, "&Raw values\tCtrl+Shift+R")?;
        let help = CreatePopupMenu()?;
        add(help, id::ABOUT, "&About FeroSNMP")?;
        let bar = CreateMenu()?;
        for (menu, title) in [
            (file, "&File"),
            (edit, "&Edit"),
            (query, "&Query"),
            (view, "&View"),
            (help, "&Help"),
        ] {
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

/// Where a list's context menu opens, and the column clicked. The row
/// clicked is selected if it was not yet.
fn clicked_cell(list: HWND, lparam: LPARAM) -> (POINT, usize) {
    let mut point = POINT::default();
    // SAFETY: reading the cursor position.
    unsafe {
        if lparam.0 == -1 {
            // From the keyboard (Shift+F10): at the mouse pointer.
            let _ = GetCursorPos(&mut point);
        } else {
            let (x, y) = point_from_lparam(lparam.0);
            point = POINT { x, y };
        }
    }
    let mut hit = LVHITTESTINFO {
        pt: point,
        ..Default::default()
    };
    // SAFETY: converting to the list's coordinates, and LVM_SUBITEMHITTEST
    // fills `hit`, which lives until the call returns.
    unsafe {
        let _ = ScreenToClient(list, &mut hit.pt);
        send(list, LVM_SUBITEMHITTEST, 0, &mut hit as *mut _ as isize);
    }
    if let Ok(row) = usize::try_from(hit.iItem)
        && !bottom_panel::all_selected(list).contains(&row)
    {
        bottom_panel::select(list, Some(row));
    }
    (point, usize::try_from(hit.iSubItem).unwrap_or(0))
}

/// Ctrl+A in a list: select every row.
fn select_all_on_ctrl_a(list: HWND, lparam: LPARAM) {
    // SAFETY: for LVN_KEYDOWN, lparam points to an NMLVKEYDOWN; GetKeyState
    // has no preconditions.
    let (key, control) = unsafe {
        let info = &*(lparam.0 as *const NMLVKEYDOWN);
        (info.wVKey, GetKeyState(i32::from(VK_CONTROL.0)) < 0)
    };
    if control && key == u16::from(b'A') {
        bottom_panel::select(list, None);
    }
}

/// The local time as `14:03:22.120`, for the log.
fn local_time() -> String {
    // SAFETY: GetLocalTime only fills the structure it returns.
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        now.wHour, now.wMinute, now.wSecond, now.wMilliseconds
    )
}

/// The OID to query for the Object box, its name for the status bar, and a
/// note for the log when the text was adjusted.
fn resolve(
    mibs: Option<&MibSet>,
    text: &str,
    kind: QueryKind,
) -> Result<(Oid, String, Option<String>), String> {
    let text = text.trim();
    let mut note = None;
    let text = if text.is_empty() && kind == QueryKind::Walk {
        note = Some("the Object box was empty, so all of mib-2 is walked".to_string());
        "1.3.6.1.2.1"
    } else {
        text
    };
    let name = |oid: &Oid| mibs.map_or_else(|| oid.to_string(), |set| set.format_oid(oid));
    let Some(set) = mibs else {
        // The MIBs are still loading: numbers only.
        let oid = text.trim_start_matches('.').parse::<Oid>().map_err(|_| {
            "The MIBs are still loading, so names cannot be looked up yet. \
             Wait a moment, or give a numeric OID such as 1.3.6.1.2.1.1.1.0."
                .to_string()
        })?;
        return Ok((oid.clone(), name(&oid), note));
    };
    let mut oid = snmp_core::resolve_object(set, text).map_err(|error| match error {
        ObjectError::Unknown { .. } => format!(
            "{error}.\n\nLoad the MIB that defines it (File > Settings, or drop the file \
             on the window), or give a numeric OID."
        ),
        other => format!("{other}."),
    })?;
    // A scalar's value is instance 0: `Get sysDescr` means `sysDescr.0`.
    if kind == QueryKind::Get
        && set
            .node_at(&oid)
            .is_some_and(|node| node.kind() == NodeKind::Scalar)
    {
        oid = oid.child(0);
        note = Some(format!(
            "{text} is a scalar, so its instance {} is asked for",
            name(&oid)
        ));
    }
    Ok((oid.clone(), name(&oid), note))
}

/// A query error for the status bar and the log, with OIDs by name.
fn describe(error: &Error, mibs: Option<&MibSet>) -> String {
    let name = |oid: &Oid| mibs.map_or_else(|| oid.to_string(), |set| set.format_oid(oid));
    match error {
        Error::Agent {
            target,
            status,
            oid: Some(oid),
            ..
        } => {
            let mut text = format!("{target} answered {status} for {}", name(oid));
            if let Some(hint) = error.hint() {
                text.push_str(&format!(". {hint}"));
            }
            text
        }
        Error::NotIncreasing { previous, next } => format!(
            "the agent returned {} after {}; OIDs must increase, so the walk stopped \
             here to avoid an endless loop (the agent is broken)",
            name(next),
            name(previous)
        ),
        Error::BulkNeedsV2 => {
            format!("{error}. Use Walk or Next, or switch the agent to SNMPv2c in File > Agents.")
        }
        other => match other.hint() {
            Some(hint) => format!("{other}. {hint}"),
            None => other.to_string(),
        },
    }
}
