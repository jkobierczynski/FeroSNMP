//! Putting text on the clipboard.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;

/// Replace the clipboard contents with `text`. `owner` is the window that
/// owns the clipboard data (any window of the program).
pub fn set_text(owner: HWND, text: &str) -> windows::core::Result<()> {
    let units: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    // SAFETY: the memory block is allocated with the size of `units`, filled
    // while locked, and handed to the clipboard, which then owns it. If the
    // hand-over fails, we still own the block and free it.
    unsafe {
        OpenClipboard(Some(owner))?;
        let result = (|| {
            EmptyClipboard()?;
            let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * size_of::<u16>())?;
            let target = GlobalLock(memory) as *mut u16;
            if target.is_null() {
                let error = windows::core::Error::from_thread();
                let _ = GlobalFree(Some(memory));
                return Err(error);
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
            let _ = GlobalUnlock(memory);
            if let Err(error) =
                SetClipboardData(u32::from(CF_UNICODETEXT.0), Some(HANDLE(memory.0)))
            {
                let _ = GlobalFree(Some(HGLOBAL(memory.0)));
                return Err(error);
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}
