//! Thin wrappers around the tree view messages used by the main window.
//!
//! The tree view does not know about MIB nodes. Each item stores a number
//! (its `lParam`), and the main window keeps a table from that number to the
//! [`NodeId`](mib_core::NodeId).

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Controls::{
    HTREEITEM, TVE_EXPAND, TVGN_CARET, TVGN_CHILD, TVGN_NEXT, TVI_LAST, TVI_ROOT, TVIF_CHILDREN,
    TVIF_HANDLE, TVIF_IMAGE, TVIF_PARAM, TVIF_SELECTEDIMAGE, TVIF_TEXT, TVINSERTSTRUCTW,
    TVINSERTSTRUCTW_0, TVITEMEXW_CHILDREN, TVITEMW, TVM_DELETEITEM, TVM_ENSUREVISIBLE, TVM_EXPAND,
    TVM_GETITEMW, TVM_GETNEXTITEM, TVM_INSERTITEMW, TVM_SELECTITEM,
};
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
use windows::core::PWSTR;

use fero_win32::text::wide;

use super::send;

/// What to show for one node.
#[derive(Debug, Clone)]
pub struct NewItem {
    /// The label, such as `ifTable(2)`.
    pub text: String,
    /// Image list position.
    pub image: i32,
    /// Show an expand button.
    pub has_children: bool,
    /// The number stored with the item.
    pub param: isize,
}

/// Add an item under `parent` (or [`TVI_ROOT`]), after its last child.
pub fn insert(tree: HWND, parent: HTREEITEM, item: &NewItem) -> HTREEITEM {
    let mut text = wide(&item.text);
    let insert = TVINSERTSTRUCTW {
        hParent: parent,
        hInsertAfter: TVI_LAST,
        Anonymous: TVINSERTSTRUCTW_0 {
            item: TVITEMW {
                mask: TVIF_TEXT | TVIF_IMAGE | TVIF_SELECTEDIMAGE | TVIF_CHILDREN | TVIF_PARAM,
                pszText: PWSTR(text.as_mut_ptr()),
                iImage: item.image,
                iSelectedImage: item.image,
                cChildren: TVITEMEXW_CHILDREN(i32::from(item.has_children)),
                lParam: LPARAM(item.param),
                ..Default::default()
            },
        },
    };
    // SAFETY: `insert` and the text it points to live until the call returns;
    // the tree copies the text.
    let result = unsafe {
        SendMessageW(
            tree,
            TVM_INSERTITEMW,
            None,
            Some(LPARAM(&insert as *const _ as isize)),
        )
    };
    HTREEITEM(result.0)
}

/// The number stored with an item.
pub fn param(tree: HWND, item: HTREEITEM) -> Option<isize> {
    let mut query = TVITEMW {
        mask: TVIF_HANDLE | TVIF_PARAM,
        hItem: item,
        ..Default::default()
    };
    // SAFETY: `query` lives until the call returns.
    let found = unsafe {
        SendMessageW(
            tree,
            TVM_GETITEMW,
            None,
            Some(LPARAM(&mut query as *mut _ as isize)),
        )
    };
    (found.0 != 0).then_some(query.lParam.0)
}

/// The children of an item (or of [`TVI_ROOT`]) that are in the tree.
pub fn children(tree: HWND, parent: HTREEITEM) -> impl Iterator<Item = HTREEITEM> {
    let next = move |item: HTREEITEM, relation: u32| {
        // SAFETY: TVM_GETNEXTITEM takes a relation and an item handle.
        let found = unsafe { send(tree, TVM_GETNEXTITEM, relation as usize, item.0) };
        (found.0 != 0).then_some(HTREEITEM(found.0))
    };
    let first = if parent == TVI_ROOT {
        // TVGN_ROOT: the first top-level item.
        next(HTREEITEM(0), 0)
    } else {
        next(parent, TVGN_CHILD)
    };
    std::iter::successors(first, move |&item| next(item, TVGN_NEXT))
}

/// Expand an item. The tree sends `TVN_ITEMEXPANDING` to its parent first,
/// which is where the children are added.
pub fn expand(tree: HWND, item: HTREEITEM) {
    // SAFETY: TVM_EXPAND takes a flag and an item handle.
    unsafe {
        send(tree, TVM_EXPAND, TVE_EXPAND.0 as usize, item.0);
    }
}

/// Select an item and scroll it into view.
pub fn select(tree: HWND, item: HTREEITEM) {
    // SAFETY: both messages take a flag and an item handle.
    unsafe {
        send(tree, TVM_SELECTITEM, TVGN_CARET as usize, item.0);
        send(tree, TVM_ENSUREVISIBLE, 0, item.0);
    }
}

/// The selected item.
pub fn selected(tree: HWND) -> Option<HTREEITEM> {
    // SAFETY: TVM_GETNEXTITEM takes a relation and an item handle.
    let found = unsafe { send(tree, TVM_GETNEXTITEM, TVGN_CARET as usize, 0) };
    (found.0 != 0).then_some(HTREEITEM(found.0))
}

/// Remove every item.
pub fn clear(tree: HWND) {
    // SAFETY: TVI_ROOT means "all items".
    unsafe {
        SendMessageW(
            tree,
            TVM_DELETEITEM,
            Some(WPARAM(0)),
            Some(LPARAM(TVI_ROOT.0)),
        );
    }
}
