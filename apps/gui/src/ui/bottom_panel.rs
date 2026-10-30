//! The panel along the bottom of the window: a tab control with five tabs,
//! each holding a virtual list.
//!
//! - **Results**: the values of the last query.
//! - **Table**: the last table walked with Table, as rows and columns. Its
//!   columns change with the table.
//! - **Traps**: the traps and INFORMs received, with a row of filters and
//!   the Start listening button above the list.
//! - **Log**: every request sent, with timings, and how each query ended.
//! - **Diagnostics**: every problem found while loading the MIBs.
//!
//! The lists have `LVS_OWNERDATA`: they store no text of their own, only a
//! row count, and ask the main window for each visible cell
//! (`LVN_GETDISPINFOW`). So a walk of 100,000 rows only costs the rows on
//! screen when painting, and adding rows is one `LVM_SETITEMCOUNT`.

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::EM_SETCUEBANNER;
use windows::Win32::UI::Controls::{
    LIST_VIEW_ITEM_STATE_FLAGS, LVCF_FMT, LVCF_TEXT, LVCF_WIDTH, LVCFMT_LEFT, LVCFMT_RIGHT,
    LVCOLUMNW, LVIF_STATE, LVIF_TEXT, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_DELETECOLUMN,
    LVM_ENSUREVISIBLE, LVM_GETCOLUMNWIDTH, LVM_GETNEXTITEM, LVM_GETSELECTEDCOUNT,
    LVM_INSERTCOLUMNW, LVM_SETCOLUMNW, LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE,
    LVM_SETITEMCOUNT, LVM_SETITEMSTATE, LVNI_SELECTED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT,
    LVS_EX_LABELTIP, LVS_NOSORTHEADER, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL,
    LVSCW_AUTOSIZE, LVSICF_NOINVALIDATEALL, LVSICF_NOSCROLL, NMLVDISPINFOW, SetWindowTheme,
    TCIF_TEXT, TCITEMW, TCM_ADJUSTRECT, TCM_GETCURSEL, TCM_INSERTITEMW, TCM_SETCURSEL,
    TCM_SETITEMW, WC_LISTVIEWW, WC_TABCONTROLW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CreateWindowExW, ES_AUTOHSCROLL, HMENU, HWND_TOP, SW_HIDE, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW,
    SetWindowPos, SetWindowTextW, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD,
    WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_TABSTOP,
};
use windows::core::{HSTRING, PWSTR, w};

use fero_win32::dpi;
use fero_win32::splitter::Rect;
use fero_win32::text::{wide, window_text};
use fero_win32::window::instance;

use crate::diagnostics::{DiagnosticList, Field};
use crate::oplog::LogColumn;
use crate::results::Column;
use crate::traps::{TrapColumn, TrapFilter};

use super::{id, send};

/// A tab of the panel, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Results,
    Table,
    Traps,
    Log,
    Diagnostics,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Results,
        Tab::Table,
        Tab::Traps,
        Tab::Log,
        Tab::Diagnostics,
    ];

    fn index(self) -> usize {
        Tab::ALL.iter().position(|&tab| tab == self).unwrap_or(0)
    }

    /// The name in the settings file's `[layout]`.
    pub fn name(self) -> &'static str {
        match self {
            Tab::Results => "results",
            Tab::Table => "table",
            Tab::Traps => "traps",
            Tab::Log => "log",
            Tab::Diagnostics => "diagnostics",
        }
    }

    /// The tab with this [`name`](Self::name).
    pub fn from_name(name: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|tab| tab.name() == name)
    }
}

/// Sizes of the row above the Traps list, at 96 DPI.
const TRAP_BAR_HEIGHT: i32 = 23;
const TRAP_BAR_GAP: i32 = 4;
/// The Start listening button and the four filter boxes, from the left.
const TRAP_BAR_WIDTHS: [i32; 5] = [110, 140, 170, 100, 170];

/// A list column: heading, width at 96 DPI, right-aligned or not.
type ColumnSpec = (&'static str, i32, bool);

fn result_columns(raw: bool) -> Vec<ColumnSpec> {
    Column::ALL
        .iter()
        .map(|c| (c.title(raw), c.width(), false))
        .collect()
}

/// Width at 96 DPI of a Table column: enough for the heading, within limits.
fn table_column_width(title: &str) -> i32 {
    (title.chars().count() as i32 * 7 + 24).clamp(90, 220)
}

fn trap_columns() -> Vec<ColumnSpec> {
    TrapColumn::ALL
        .iter()
        .map(|c| (c.title(), c.width(), false))
        .collect()
}

fn log_columns() -> Vec<ColumnSpec> {
    LogColumn::ALL
        .iter()
        .map(|c| (c.title(), c.width(), false))
        .collect()
}

fn diagnostic_columns() -> Vec<ColumnSpec> {
    Field::ALL
        .iter()
        .map(|c| (c.title(), c.width(), c.is_number()))
        .collect()
}

/// The tab control and the lists on it.
#[derive(Debug, Clone, Copy, Default)]
pub struct BottomPanel {
    pub tabs: HWND,
    pub results: HWND,
    pub table: HWND,
    pub traps: HWND,
    pub log: HWND,
    pub diagnostics: HWND,
    /// The row above the Traps list.
    pub trap_bar: TrapBar,
    dpi: u32,
}

/// The Start listening button, the filter boxes, and a line saying where
/// the receiver listens, above the Traps list.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrapBar {
    pub listen: HWND,
    pub source: HWND,
    pub notification: HWND,
    pub severity: HWND,
    pub text: HWND,
    pub state: HWND,
}

impl TrapBar {
    /// Every control, from the left.
    pub fn controls(&self) -> [HWND; 6] {
        [
            self.listen,
            self.source,
            self.notification,
            self.severity,
            self.text,
            self.state,
        ]
    }

    /// True if `id` is one of the filter boxes.
    pub fn is_filter(id: u16) -> bool {
        matches!(
            id,
            id::TRAP_SOURCE | id::TRAP_NOTIFICATION | id::TRAP_SEVERITY | id::TRAP_TEXT
        )
    }

    /// What the filter boxes say.
    pub fn filter(&self) -> TrapFilter {
        TrapFilter {
            source: window_text(self.source),
            notification: window_text(self.notification),
            severity: window_text(self.severity),
            text: window_text(self.text),
        }
    }

    /// Show Start or Stop listening on the button, and where the receiver
    /// listens next to the boxes.
    pub fn set_listening(&self, listening: bool, state: &str) {
        let label = if listening {
            "Stop listening"
        } else {
            "Start listening"
        };
        // SAFETY: setting the text of our own controls.
        unsafe {
            let _ = SetWindowTextW(self.listen, &HSTRING::from(label));
            let _ = SetWindowTextW(self.state, &HSTRING::from(state));
        }
    }
}

impl BottomPanel {
    /// Create the panel's controls as children of `parent`, showing the
    /// Results tab.
    pub fn create(parent: HWND, dpi: u32) -> windows::core::Result<BottomPanel> {
        let child = |class, style: WINDOW_STYLE, ex_style, id: u16| {
            // SAFETY: creates a child control of our window.
            unsafe {
                CreateWindowExW(
                    ex_style,
                    class,
                    None,
                    WS_CHILD | style,
                    0,
                    0,
                    0,
                    0,
                    Some(parent),
                    Some(HMENU(usize::from(id) as *mut _)),
                    Some(instance()),
                    None,
                )
            }
        };
        // The lists are siblings of the tab control placed over its page
        // area, as tab controls do not host children well themselves.
        let list = |id: u16, single: bool| {
            let mut style = LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER;
            if single {
                style |= LVS_SINGLESEL;
            }
            child(
                WC_LISTVIEWW,
                WS_TABSTOP | WINDOW_STYLE(style),
                WS_EX_CLIENTEDGE,
                id,
            )
        };
        let results = list(id::RESULTS, false)?;
        let table = list(id::TABLE, false)?;
        let traps = list(id::TRAPS, false)?;
        let log = list(id::LOG, false)?;
        let diagnostics = list(id::DIAGNOSTICS, true)?;
        let filter_box = |id: u16, cue: &str| -> windows::core::Result<HWND> {
            let edit = child(
                w!("EDIT"),
                WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                WS_EX_CLIENTEDGE,
                id,
            )?;
            let cue = HSTRING::from(cue);
            // SAFETY: the cue text lives until the call returns; the box
            // copies it. wparam 1: show it also while the box has the focus.
            unsafe {
                SendMessageW(
                    edit,
                    EM_SETCUEBANNER,
                    Some(WPARAM(1)),
                    Some(LPARAM(cue.as_ptr() as isize)),
                );
            }
            Ok(edit)
        };
        let trap_bar = TrapBar {
            listen: child(
                w!("BUTTON"),
                WS_TABSTOP | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                WINDOW_EX_STYLE(0),
                id::LISTEN,
            )?,
            source: filter_box(id::TRAP_SOURCE, "From (address)")?,
            notification: filter_box(id::TRAP_NOTIFICATION, "Notification, such as link*")?,
            severity: filter_box(id::TRAP_SEVERITY, "Severity")?,
            text: filter_box(id::TRAP_TEXT, "Any text")?,
            state: child(w!("STATIC"), WINDOW_STYLE(0), WINDOW_EX_STYLE(0), 0)?,
        };
        trap_bar.set_listening(false, "Not listening");
        let tabs = child(
            WC_TABCONTROLW,
            WS_CLIPSIBLINGS,
            WINDOW_EX_STYLE(0),
            id::TABS,
        )?;
        // SAFETY: plain control messages; the tab texts live until each call
        // returns.
        unsafe {
            let _ = ShowWindow(tabs, SW_SHOW);
            for list in [results, table, traps, log, diagnostics] {
                // Keep the lists above the tab control; the tab control clips
                // itself against them (WS_CLIPSIBLINGS), so it never paints
                // over them.
                let _ = SetWindowPos(
                    list,
                    Some(HWND_TOP),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                let _ = SetWindowTheme(list, w!("Explorer"), None);
                let extended = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP;
                send(
                    list,
                    LVM_SETEXTENDEDLISTVIEWSTYLE,
                    extended as usize,
                    extended as isize,
                );
            }
            for (index, title) in ["Results", "Table", "Traps", "Log", "Diagnostics"]
                .into_iter()
                .enumerate()
            {
                let mut text = wide(title);
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
        let panel = BottomPanel {
            tabs,
            results,
            table,
            traps,
            log,
            diagnostics,
            trap_bar,
            dpi,
        };
        add_columns(results, &result_columns(false), dpi);
        add_columns(traps, &trap_columns(), dpi);
        add_columns(log, &log_columns(), dpi);
        add_columns(diagnostics, &diagnostic_columns(), dpi);
        panel.select_tab(Tab::Results);
        Ok(panel)
    }

    fn lists(&self) -> [(Tab, HWND); 5] {
        [
            (Tab::Results, self.results),
            (Tab::Table, self.table),
            (Tab::Traps, self.traps),
            (Tab::Log, self.log),
            (Tab::Diagnostics, self.diagnostics),
        ]
    }

    /// Every list, for the dark mode.
    pub fn all_lists(&self) -> [HWND; 5] {
        self.lists().map(|(_, list)| list)
    }

    /// Every control of the panel, for the font.
    pub fn controls(&self) -> Vec<HWND> {
        let mut controls = vec![self.tabs];
        controls.extend(self.all_lists());
        controls.extend(self.trap_bar.controls());
        controls
    }

    /// Use the sizes for a new DPI from the next [`place`](Self::place) on.
    pub fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi;
    }

    /// The tab shown.
    pub fn current_tab(&self) -> Tab {
        // SAFETY: TCM_GETCURSEL takes no parameters.
        let index = unsafe { send(self.tabs, TCM_GETCURSEL, 0, 0).0 };
        usize::try_from(index)
            .ok()
            .and_then(|i| Tab::ALL.get(i).copied())
            .unwrap_or(Tab::Results)
    }

    /// Show a tab and its list. Also call after the user clicked a tab
    /// (`TCN_SELCHANGE`), with [`current_tab`](Self::current_tab).
    pub fn select_tab(&self, tab: Tab) {
        // SAFETY: plain control messages on our own controls.
        unsafe {
            send(self.tabs, TCM_SETCURSEL, tab.index(), 0);
            for (list_tab, list) in self.lists() {
                let _ = ShowWindow(list, if list_tab == tab { SW_SHOW } else { SW_HIDE });
            }
            for control in self.trap_bar.controls() {
                let _ = ShowWindow(control, if tab == Tab::Traps { SW_SHOW } else { SW_HIDE });
            }
        }
    }

    /// Replace the Table columns with `headers`.
    pub fn set_table_columns(&self, headers: &[String], dpi: u32) {
        // SAFETY: plain list view messages. Deleting column 0 until it fails
        // empties the header; a report list keeps working without columns.
        unsafe { while send(self.table, LVM_DELETECOLUMN, 0, 0).0 != 0 {} }
        let columns: Vec<(&str, i32, bool)> = headers
            .iter()
            .map(|title| (title.as_str(), table_column_width(title), false))
            .collect();
        add_columns(self.table, &columns, dpi);
    }

    /// Size each Table column to its widest cell on screen, but never
    /// narrower than its heading needs, once the rows are in.
    pub fn fit_table_columns(&self, headers: &[String], dpi: u32) {
        for (index, title) in headers.iter().enumerate() {
            // SAFETY: LVM_SETCOLUMNWIDTH takes a column index and a width,
            // LVM_GETCOLUMNWIDTH a column index; LVSCW_AUTOSIZE measures the
            // cells.
            unsafe {
                send(
                    self.table,
                    LVM_SETCOLUMNWIDTH,
                    index,
                    LVSCW_AUTOSIZE as isize,
                );
                let cells = send(self.table, LVM_GETCOLUMNWIDTH, index, 0).0 as i32;
                let width = cells.max(dpi::scale(table_column_width(title), dpi));
                send(self.table, LVM_SETCOLUMNWIDTH, index, width as isize);
            }
        }
    }

    /// Set the column widths for a new DPI.
    pub fn size_columns(&self, dpi: u32, table_headers: &[String]) {
        for (index, title) in table_headers.iter().enumerate() {
            // SAFETY: LVM_SETCOLUMNWIDTH takes a column index and a width.
            unsafe {
                send(
                    self.table,
                    LVM_SETCOLUMNWIDTH,
                    index,
                    dpi::scale(table_column_width(title), dpi) as isize,
                );
            }
        }
        for (list, columns) in [
            (self.results, result_columns(false)),
            (self.traps, trap_columns()),
            (self.log, log_columns()),
            (self.diagnostics, diagnostic_columns()),
        ] {
            for (index, (_, width, _)) in columns.into_iter().enumerate() {
                // SAFETY: LVM_SETCOLUMNWIDTH takes a column index and a width.
                unsafe {
                    send(
                        list,
                        LVM_SETCOLUMNWIDTH,
                        index,
                        dpi::scale(width, dpi) as isize,
                    );
                }
            }
        }
    }

    /// Change the Results headings for the raw view or back.
    pub fn set_raw_headings(&self, raw: bool) {
        for (index, (title, _, _)) in result_columns(raw).into_iter().enumerate() {
            let mut text = wide(title);
            let column = LVCOLUMNW {
                mask: LVCF_TEXT,
                pszText: PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            // SAFETY: `column` and its text live until the call returns.
            unsafe {
                SendMessageW(
                    self.results,
                    LVM_SETCOLUMNW,
                    Some(WPARAM(index)),
                    Some(LPARAM(&column as *const _ as isize)),
                );
            }
        }
    }

    /// Put the panel in `area`: the tabs fill it, the lists fill the page.
    pub fn place(&self, area: Rect) {
        let place = |hwnd: HWND, rect: Rect| {
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
        place(self.tabs, area);
        let mut page = RECT {
            left: area.left,
            top: area.top,
            right: area.right,
            bottom: area.bottom,
        };
        // SAFETY: TCM_ADJUSTRECT turns the window rectangle into the page
        // rectangle in place.
        unsafe {
            send(self.tabs, TCM_ADJUSTRECT, 0, &mut page as *mut _ as isize);
        }
        let page = Rect {
            left: page.left,
            top: page.top,
            right: page.right.max(page.left),
            bottom: page.bottom.max(page.top),
        };
        for (tab, list) in self.lists() {
            if tab != Tab::Traps {
                place(list, page);
            }
        }
        // The Traps tab: the row of filters, then the list.
        let scale = |size| dpi::scale(size, self.dpi);
        let gap = scale(TRAP_BAR_GAP);
        let bar_bottom = (page.top + gap + scale(TRAP_BAR_HEIGHT)).min(page.bottom);
        let mut left = page.left;
        for (control, width) in self.trap_bar.controls().into_iter().zip(TRAP_BAR_WIDTHS) {
            let right = (left + scale(width)).min(page.right);
            place(
                control,
                Rect {
                    left,
                    top: page.top + gap,
                    right,
                    bottom: bar_bottom,
                },
            );
            left = (right + gap).min(page.right);
        }
        // The line saying where the receiver listens takes the rest; a
        // static control's text sits at its top, so it is moved down to the
        // middle of the boxes.
        let text_top = page.top + gap + scale(4);
        place(
            self.trap_bar.state,
            Rect {
                left,
                top: text_top.min(bar_bottom),
                right: page.right.max(left),
                bottom: bar_bottom,
            },
        );
        place(
            self.traps,
            Rect {
                left: page.left,
                top: (bar_bottom + gap).min(page.bottom),
                right: page.right,
                bottom: page.bottom,
            },
        );
    }

    /// Set a tab's title, such as `Results (1,234)`.
    pub fn set_title(&self, tab: Tab, title: &str) {
        let mut text = wide(title);
        let item = TCITEMW {
            mask: TCIF_TEXT,
            pszText: PWSTR(text.as_mut_ptr()),
            ..Default::default()
        };
        // SAFETY: `item` and its text live until the call returns.
        unsafe {
            SendMessageW(
                self.tabs,
                TCM_SETITEMW,
                Some(WPARAM(tab.index())),
                Some(LPARAM(&item as *const _ as isize)),
            );
            // A shorter title leaves the old end of the tab row painted.
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.tabs), None, true);
        }
    }

    /// Show a new diagnostics list: set the row count and the tab title.
    pub fn show_diagnostics(&self, list: &DiagnosticList) {
        set_count(self.diagnostics, list.len(), false);
        self.set_title(
            Tab::Diagnostics,
            &format!("Diagnostics ({})", list.summary()),
        );
    }
}

fn add_columns(list: HWND, columns: &[(&str, i32, bool)], dpi: u32) {
    for (index, &(title, width, right)) in columns.iter().enumerate() {
        let mut text = wide(title);
        let header = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT,
            fmt: if right { LVCFMT_RIGHT } else { LVCFMT_LEFT },
            cx: dpi::scale(width, dpi),
            pszText: PWSTR(text.as_mut_ptr()),
            ..Default::default()
        };
        // SAFETY: `header` and its text live until the call returns.
        unsafe {
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                Some(WPARAM(index)),
                Some(LPARAM(&header as *const _ as isize)),
            );
        }
    }
}

/// Set the number of rows of a virtual list. With `growing`, rows were only
/// added at the end: the list keeps its scroll position and repaints only
/// what changed, which keeps a streaming walk smooth.
pub fn set_count(list: HWND, count: usize, growing: bool) {
    let flags = if growing {
        LVSICF_NOINVALIDATEALL | LVSICF_NOSCROLL
    } else {
        0
    };
    // SAFETY: plain control messages.
    unsafe {
        send(list, LVM_SETITEMCOUNT, count, flags as isize);
        if !growing {
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(list), None, true);
        }
    }
}

/// Answer `LVN_GETDISPINFOW`: the column asked for, if the list wants text.
pub fn asked_cell(info: &NMLVDISPINFOW) -> Option<(usize, usize)> {
    let item = &info.item;
    if item.mask.0 & LVIF_TEXT.0 == 0 || item.pszText.is_null() || item.cchTextMax <= 0 {
        return None;
    }
    Some((
        usize::try_from(item.iItem).ok()?,
        usize::try_from(item.iSubItem).ok()?,
    ))
}

/// Copy `text` into the buffer `LVN_GETDISPINFOW` provides, on one line.
pub fn fill_cell(info: &mut NMLVDISPINFOW, text: &str) {
    let item = &mut info.item;
    if item.pszText.is_null() || item.cchTextMax <= 0 {
        return;
    }
    let capacity = item.cchTextMax as usize;
    let units: Vec<u16> = text
        .encode_utf16()
        // Line breaks and tabs show as boxes in a list cell.
        .map(|unit| {
            if matches!(unit, 9 | 10 | 13) {
                32
            } else {
                unit
            }
        })
        .take(capacity - 1)
        .collect();
    // SAFETY: the list provides a buffer of cchTextMax UTF-16 units; we
    // write at most cchTextMax - 1 of them and a terminating zero.
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), item.pszText.0, units.len());
        *item.pszText.0.add(units.len()) = 0;
    }
}

/// The first selected row of a list.
pub fn selected(list: HWND) -> Option<usize> {
    next_selected(list, None)
}

fn next_selected(list: HWND, after: Option<usize>) -> Option<usize> {
    let start = after.map_or(usize::MAX, |row| row); // -1: from the top
    // SAFETY: LVM_GETNEXTITEM takes a start index and flags.
    let row = unsafe { send(list, LVM_GETNEXTITEM, start, LVNI_SELECTED as isize) };
    usize::try_from(row.0).ok()
}

/// Every selected row of a list, in order.
pub fn all_selected(list: HWND) -> Vec<usize> {
    // SAFETY: LVM_GETSELECTEDCOUNT takes no parameters.
    let count = unsafe { send(list, LVM_GETSELECTEDCOUNT, 0, 0).0 };
    let mut rows = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    let mut row = None;
    while let Some(next) = next_selected(list, row) {
        rows.push(next);
        row = Some(next);
    }
    rows
}

/// Select only `row` (or with `row` = `None`, every row), and with a row,
/// focus it and scroll it into view.
pub fn select(list: HWND, row: Option<usize>) {
    let flags = LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0);
    let clear = LVITEMW {
        mask: LVIF_STATE,
        state: LIST_VIEW_ITEM_STATE_FLAGS(0),
        stateMask: LVIS_SELECTED,
        ..Default::default()
    };
    let set = LVITEMW {
        mask: LVIF_STATE,
        state: if row.is_some() { flags } else { LVIS_SELECTED },
        stateMask: if row.is_some() { flags } else { LVIS_SELECTED },
        ..Default::default()
    };
    // SAFETY: the items live until each call returns; index -1 means every
    // row.
    unsafe {
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(&clear as *const _ as isize)),
        );
        SendMessageW(
            list,
            LVM_SETITEMSTATE,
            Some(WPARAM(row.unwrap_or(usize::MAX))),
            Some(LPARAM(&set as *const _ as isize)),
        );
        if let Some(row) = row {
            send(list, LVM_ENSUREVISIBLE, row, 0);
        }
    }
}

/// Scroll the last row into view.
pub fn show_last(list: HWND, count: usize) {
    if let Some(last) = count.checked_sub(1) {
        // SAFETY: LVM_ENSUREVISIBLE takes a row and FALSE (fully visible).
        unsafe {
            send(list, LVM_ENSUREVISIBLE, last, 0);
        }
    }
}
