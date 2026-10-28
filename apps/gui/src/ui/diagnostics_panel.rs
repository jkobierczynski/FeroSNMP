//! The panel along the bottom of the window: a tab control with the
//! Diagnostics tab, which holds a virtual list of every problem found while
//! loading.
//!
//! The list has `LVS_OWNERDATA`: it stores no text of its own, only a row
//! count, and asks the main window for each visible cell (`LVN_GETDISPINFOW`),
//! which answers from [`DiagnosticList`].

use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Controls::{
    LIST_VIEW_ITEM_STATE_FLAGS, LVCF_FMT, LVCF_TEXT, LVCF_WIDTH, LVCFMT_LEFT, LVCFMT_RIGHT,
    LVCOLUMNW, LVIF_STATE, LVIF_TEXT, LVIS_FOCUSED, LVIS_SELECTED, LVITEMW, LVM_ENSUREVISIBLE,
    LVM_GETNEXTITEM, LVM_INSERTCOLUMNW, LVM_SETCOLUMNWIDTH, LVM_SETEXTENDEDLISTVIEWSTYLE,
    LVM_SETITEMCOUNT, LVM_SETITEMSTATE, LVNI_SELECTED, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT,
    LVS_EX_LABELTIP, LVS_NOSORTHEADER, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS, LVS_SINGLESEL,
    NMLVDISPINFOW, SetWindowTheme, TCIF_TEXT, TCITEMW, TCM_ADJUSTRECT, TCM_INSERTITEMW,
    TCM_SETITEMW, WC_LISTVIEWW, WC_TABCONTROLW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, HMENU, HWND_TOP, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SendMessageW, SetWindowPos, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CHILD,
    WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PWSTR, w};

use fero_win32::dpi;
use fero_win32::splitter::Rect;
use fero_win32::text::wide;
use fero_win32::window::instance;

use crate::diagnostics::{DiagnosticList, Field};

use super::{id, send};

/// The tab control and the list on it.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiagnosticsPanel {
    pub tabs: HWND,
    pub list: HWND,
}

impl DiagnosticsPanel {
    /// Create the panel's controls as children of `parent`.
    pub fn create(parent: HWND, dpi: u32) -> windows::core::Result<DiagnosticsPanel> {
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
                    Some(parent),
                    Some(HMENU(usize::from(id) as *mut _)),
                    Some(instance()),
                    None,
                )
            }
        };
        // The list is a sibling of the tab control placed over its page area,
        // as tab controls do not host children well themselves.
        let list = child(
            WC_LISTVIEWW,
            WS_TABSTOP
                | WINDOW_STYLE(
                    LVS_REPORT
                        | LVS_OWNERDATA
                        | LVS_SINGLESEL
                        | LVS_SHOWSELALWAYS
                        | LVS_NOSORTHEADER,
                ),
            WS_EX_CLIENTEDGE,
            id::DIAGNOSTICS,
        )?;
        let tabs = child(
            WC_TABCONTROLW,
            WS_CLIPSIBLINGS,
            WINDOW_EX_STYLE(0),
            id::TABS,
        )?;
        // SAFETY: plain control messages; the tab text lives until the call
        // returns.
        unsafe {
            // Keep the list above the tab control; the tab control clips
            // itself against it (WS_CLIPSIBLINGS), so it never paints over it.
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
            let mut text = wide("Diagnostics");
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            SendMessageW(
                tabs,
                TCM_INSERTITEMW,
                Some(WPARAM(0)),
                Some(LPARAM(&item as *const _ as isize)),
            );
        }
        let panel = DiagnosticsPanel { tabs, list };
        panel.add_columns(dpi);
        Ok(panel)
    }

    fn add_columns(&self, dpi: u32) {
        for (index, column) in Field::ALL.into_iter().enumerate() {
            let mut text = wide(column.title());
            let format = if column.is_number() {
                LVCFMT_RIGHT
            } else {
                LVCFMT_LEFT
            };
            let header = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT,
                fmt: format,
                cx: dpi::scale(column.width(), dpi),
                pszText: PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            // SAFETY: `header` and its text live until the call returns.
            unsafe {
                SendMessageW(
                    self.list,
                    LVM_INSERTCOLUMNW,
                    Some(WPARAM(index)),
                    Some(LPARAM(&header as *const _ as isize)),
                );
            }
        }
    }

    /// Set the column widths for a new DPI.
    pub fn size_columns(&self, dpi: u32) {
        for (index, column) in Field::ALL.into_iter().enumerate() {
            // SAFETY: LVM_SETCOLUMNWIDTH takes a column index and a width.
            unsafe {
                send(
                    self.list,
                    LVM_SETCOLUMNWIDTH,
                    index,
                    dpi::scale(column.width(), dpi) as isize,
                );
            }
        }
    }

    /// Put the panel in `area`: the tabs fill it, the list fills the page.
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
        place(
            self.list,
            Rect {
                left: page.left,
                top: page.top,
                right: page.right.max(page.left),
                bottom: page.bottom.max(page.top),
            },
        );
    }

    /// Show a new list: set the row count and the tab title.
    pub fn show(&self, list: &DiagnosticList) {
        // SAFETY: plain control messages; the tab text lives until the call
        // returns.
        unsafe {
            send(self.list, LVM_SETITEMCOUNT, list.len(), 0);
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.list), None, true);
            let mut text = wide(&format!("Diagnostics ({})", list.summary()));
            let item = TCITEMW {
                mask: TCIF_TEXT,
                pszText: PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            SendMessageW(
                self.tabs,
                TCM_SETITEMW,
                Some(WPARAM(0)),
                Some(LPARAM(&item as *const _ as isize)),
            );
        }
    }

    /// Answer `LVN_GETDISPINFOW`: copy the text of the asked-for cell into the
    /// buffer the list provides.
    pub fn fill_cell(list: &DiagnosticList, info: &mut NMLVDISPINFOW) {
        let item = &mut info.item;
        if item.mask.0 & LVIF_TEXT.0 == 0 || item.pszText.is_null() || item.cchTextMax <= 0 {
            return;
        }
        let Some(&column) = usize::try_from(item.iSubItem)
            .ok()
            .and_then(|index| Field::ALL.get(index))
        else {
            return;
        };
        let row = usize::try_from(item.iItem).unwrap_or(usize::MAX);
        // One line per cell: messages and hints never contain line breaks
        // that should show, but be safe.
        let text = list.cell(row, column).replace(['\r', '\n'], " ");
        let capacity = item.cchTextMax as usize;
        let units: Vec<u16> = text.encode_utf16().take(capacity - 1).collect();
        // SAFETY: the list provides a buffer of cchTextMax UTF-16 units; we
        // write at most cchTextMax - 1 of them and a terminating zero.
        unsafe {
            std::ptr::copy_nonoverlapping(units.as_ptr(), item.pszText.0, units.len());
            *item.pszText.0.add(units.len()) = 0;
        }
    }

    /// The selected row.
    pub fn selected(&self) -> Option<usize> {
        // SAFETY: LVM_GETNEXTITEM takes a start index (-1: from the top) and
        // flags.
        let row = unsafe {
            send(
                self.list,
                LVM_GETNEXTITEM,
                usize::MAX,
                LVNI_SELECTED as isize,
            )
        };
        usize::try_from(row.0).ok()
    }

    /// Select a row and scroll it into view.
    pub fn select(&self, row: usize) {
        let state = LVITEMW {
            mask: LVIF_STATE,
            state: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
            stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0),
            ..Default::default()
        };
        // SAFETY: `state` lives until the call returns.
        unsafe {
            SendMessageW(
                self.list,
                LVM_SETITEMSTATE,
                Some(WPARAM(row)),
                Some(LPARAM(&state as *const _ as isize)),
            );
            send(self.list, LVM_ENSUREVISIBLE, row, 0);
        }
    }
}
