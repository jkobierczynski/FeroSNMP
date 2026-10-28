//! Standard shell dialogs.

use std::path::PathBuf;

use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog,
    SIGDN_FILESYSPATH,
};
use windows::core::HSTRING;

/// Show the "Select Folder" dialog and return the chosen folders, or an empty
/// list if the user cancelled.
///
/// COM must be initialised on this thread (`CoInitializeEx` with
/// `COINIT_APARTMENTTHREADED`), as for any program with a user interface.
pub fn pick_folders(owner: Option<HWND>, title: &str) -> windows::core::Result<Vec<PathBuf>> {
    // SAFETY: ordinary COM calls on interfaces we own; the strings returned by
    // GetDisplayName are freed with CoTaskMemFree after copying.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let options = dialog.GetOptions()?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_ALLOWMULTISELECT)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        if let Err(error) = dialog.Show(owner) {
            if error.code() == ERROR_CANCELLED.to_hresult() {
                return Ok(Vec::new());
            }
            return Err(error);
        }
        let items = dialog.GetResults()?;
        let mut folders = Vec::new();
        for i in 0..items.GetCount()? {
            let name = items.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = name.to_string();
            CoTaskMemFree(Some(name.0 as *const _));
            folders.push(PathBuf::from(path?));
        }
        Ok(folders)
    }
}
