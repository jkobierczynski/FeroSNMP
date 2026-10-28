//! The right-hand side: the properties of the selected node in a list, and
//! its description below.

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Controls::{
    LVCF_TEXT, LVCF_WIDTH, LVCOLUMNW, LVIF_TEXT, LVITEMW, LVM_DELETEALLITEMS, LVM_INSERTCOLUMNW,
    LVM_INSERTITEMW, LVM_SETCOLUMNWIDTH, LVM_SETITEMTEXTW, LVSCW_AUTOSIZE_USEHEADER,
};
use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, SetWindowTextW, WM_SETREDRAW};
use windows::core::{HSTRING, PWSTR};

use fero_win32::dpi;
use fero_win32::text::{crlf, wide};

use super::send;

/// Width of the "Property" column at 96 DPI.
const LABEL_WIDTH: i32 = 110;

/// Add the two columns to a freshly created list view.
pub fn add_columns(list: HWND, dpi: u32) {
    for (index, title) in ["Property", "Value"].into_iter().enumerate() {
        let mut text = wide(title);
        let column = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH,
            cx: dpi::scale(LABEL_WIDTH, dpi),
            pszText: PWSTR(text.as_mut_ptr()),
            ..Default::default()
        };
        // SAFETY: `column` and its text live until the call returns.
        unsafe {
            SendMessageW(
                list,
                LVM_INSERTCOLUMNW,
                Some(WPARAM(index)),
                Some(LPARAM(&column as *const _ as isize)),
            );
        }
    }
}

/// Set the label column to its width at `dpi` and let the value column fill
/// the rest. Call after the DPI changes.
pub fn size_columns(list: HWND, dpi: u32) {
    // SAFETY: LVM_SETCOLUMNWIDTH takes a column index and a width.
    unsafe {
        send(
            list,
            LVM_SETCOLUMNWIDTH,
            0,
            dpi::scale(LABEL_WIDTH, dpi) as isize,
        );
    }
    fill_value_column(list);
}

/// Let the value column fill the width of the list. Call after resizing it.
pub fn fill_value_column(list: HWND) {
    // SAFETY: LVM_SETCOLUMNWIDTH takes a column index and a width.
    unsafe {
        send(
            list,
            LVM_SETCOLUMNWIDTH,
            1,
            LVSCW_AUTOSIZE_USEHEADER as isize,
        );
    }
}

/// Show `rows` in the list and `description` in the text box.
pub fn show(list: HWND, description: HWND, rows: &[(&str, String)], text: &str) {
    // SAFETY: each LVITEMW and its text live until its call returns; the list
    // copies the text.
    unsafe {
        send(list, WM_SETREDRAW, 0, 0);
        send(list, LVM_DELETEALLITEMS, 0, 0);
        for (index, (label, value)) in rows.iter().enumerate() {
            let mut label = wide(label);
            let mut item = LVITEMW {
                mask: LVIF_TEXT,
                iItem: index as i32,
                pszText: PWSTR(label.as_mut_ptr()),
                ..Default::default()
            };
            SendMessageW(
                list,
                LVM_INSERTITEMW,
                None,
                Some(LPARAM(&item as *const _ as isize)),
            );
            let mut value = wide(value);
            item.iSubItem = 1;
            item.pszText = PWSTR(value.as_mut_ptr());
            SendMessageW(
                list,
                LVM_SETITEMTEXTW,
                Some(WPARAM(index)),
                Some(LPARAM(&item as *const _ as isize)),
            );
        }
        send(list, WM_SETREDRAW, 1, 0);
        let _ = InvalidateRect(Some(list), None, true);
        let _ = SetWindowTextW(description, &HSTRING::from(crlf(text)));
    }
}
