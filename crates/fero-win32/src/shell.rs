//! Standard shell dialogs.

use std::path::PathBuf;

use windows::Win32::Foundation::{ERROR_CANCELLED, HWND};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT,
    FOS_PICKFOLDERS, FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog,
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

/// One choice in the "Save as type" list of [`save_file`]: a name such as
/// `"Text files (*.txt)"`, the pattern `"*.txt"`, and the extension added
/// when the user types a name without one (`"txt"`).
#[derive(Debug, Clone, Copy)]
pub struct FileType<'a> {
    pub name: &'a str,
    pub pattern: &'a str,
    pub extension: &'a str,
}

/// Show the "Save As" dialog and return the chosen file, or `None` if the
/// user cancelled. The dialog asks before replacing an existing file. When
/// the user types a name without an extension, the extension of the chosen
/// file type is added.
///
/// COM must be initialised on this thread, as for [`pick_folders`].
pub fn save_file(
    owner: Option<HWND>,
    title: &str,
    file_name: &str,
    types: &[FileType<'_>],
) -> windows::core::Result<Option<PathBuf>> {
    let names: Vec<HSTRING> = types.iter().map(|t| HSTRING::from(t.name)).collect();
    let patterns: Vec<HSTRING> = types.iter().map(|t| HSTRING::from(t.pattern)).collect();
    let specs: Vec<COMDLG_FILTERSPEC> = names
        .iter()
        .zip(&patterns)
        .map(|(name, pattern)| COMDLG_FILTERSPEC {
            pszName: windows::core::PCWSTR(name.as_ptr()),
            pszSpec: windows::core::PCWSTR(pattern.as_ptr()),
        })
        .collect();
    // SAFETY: ordinary COM calls; `specs` and the strings it points to live
    // until the end of the function, and the path string is freed with
    // CoTaskMemFree after copying.
    unsafe {
        let dialog: IFileSaveDialog =
            CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
        let options = dialog.GetOptions()?;
        dialog.SetOptions(options | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        dialog.SetFileName(&HSTRING::from(file_name))?;
        if !specs.is_empty() {
            dialog.SetFileTypes(&specs)?;
            dialog.SetDefaultExtension(&HSTRING::from(types[0].extension))?;
        }
        if let Err(error) = dialog.Show(owner) {
            if error.code() == ERROR_CANCELLED.to_hresult() {
                return Ok(None);
            }
            return Err(error);
        }
        let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string();
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(Some(PathBuf::from(path?)))
    }
}

/// Show the "Open" dialog for one existing file and return it, or `None` if
/// the user cancelled.
///
/// COM must be initialised on this thread, as for [`pick_folders`].
pub fn open_file(
    owner: Option<HWND>,
    title: &str,
    types: &[FileType<'_>],
) -> windows::core::Result<Option<PathBuf>> {
    let names: Vec<HSTRING> = types.iter().map(|t| HSTRING::from(t.name)).collect();
    let patterns: Vec<HSTRING> = types.iter().map(|t| HSTRING::from(t.pattern)).collect();
    let specs: Vec<COMDLG_FILTERSPEC> = names
        .iter()
        .zip(&patterns)
        .map(|(name, pattern)| COMDLG_FILTERSPEC {
            pszName: windows::core::PCWSTR(name.as_ptr()),
            pszSpec: windows::core::PCWSTR(pattern.as_ptr()),
        })
        .collect();
    // SAFETY: as in `save_file`.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let options = dialog.GetOptions()?;
        dialog.SetOptions(options | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)?;
        dialog.SetTitle(&HSTRING::from(title))?;
        if !specs.is_empty() {
            dialog.SetFileTypes(&specs)?;
        }
        if let Err(error) = dialog.Show(owner) {
            if error.code() == ERROR_CANCELLED.to_hresult() {
                return Ok(None);
            }
            return Err(error);
        }
        let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string();
        CoTaskMemFree(Some(name.0 as *const _));
        Ok(Some(PathBuf::from(path?)))
    }
}
