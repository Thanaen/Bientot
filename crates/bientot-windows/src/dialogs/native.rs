//! The Win32 calls behind the file dialogs.

#![allow(unsafe_code)]

use std::path::PathBuf;

use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance,
    CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_OVERWRITEPROMPT, FileOpenDialog, FileSaveDialog,
    IFileDialog, IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH,
};
use windows::core::{HSTRING, Interface, PCWSTR};

/// Shows the save dialog, offering `suggested_name` and only files of type `file_type`, named
/// `*.extension`; a name typed without the extension gets it. `None` when the user cancels.
pub fn pick_save_file(
    suggested_name: &str,
    file_type: &str,
    extension: &str,
) -> windows::core::Result<Option<PathBuf>> {
    in_apartment(|| {
        // SAFETY: the apartment is initialized; the dialog is released before it is left.
        let dialog: IFileSaveDialog =
            unsafe { CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)? };
        // SAFETY: the string outlives the call, which copies it.
        unsafe {
            dialog.SetDefaultExtension(&HSTRING::from(extension))?;
            dialog.SetFileName(&HSTRING::from(suggested_name))?;
        }
        show(
            &dialog.cast()?,
            file_type,
            &[extension],
            FOS_OVERWRITEPROMPT,
        )
    })
}

/// Shows the open dialog, offering only existing files of type `file_type`, named with one of
/// `extensions`. `None` when the user cancels.
pub fn pick_open_file(
    file_type: &str,
    extensions: &[&str],
) -> windows::core::Result<Option<PathBuf>> {
    in_apartment(|| {
        // SAFETY: the apartment is initialized; the dialog is released before it is left.
        let dialog: IFileOpenDialog =
            unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? };
        show(&dialog.cast()?, file_type, extensions, FOS_FILEMUSTEXIST)
    })
}

/// Runs `dialog` in a single-threaded apartment joined for it, as the dialogs require.
fn in_apartment(
    dialog: impl FnOnce() -> windows::core::Result<Option<PathBuf>>,
) -> windows::core::Result<Option<PathBuf>> {
    // SAFETY: balanced by `CoUninitialize` below, once the dialog's objects are released.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.ok()?;
    let picked = dialog();
    // SAFETY: the `CoInitializeEx` above succeeded on this thread.
    unsafe { CoUninitialize() };
    picked
}

fn show(
    dialog: &IFileDialog,
    file_type: &str,
    extensions: &[&str],
    options: windows::Win32::UI::Shell::FILEOPENDIALOGOPTIONS,
) -> windows::core::Result<Option<PathBuf>> {
    let patterns: Vec<_> = extensions
        .iter()
        .map(|extension| format!("*.{extension}"))
        .collect();
    let (file_type, pattern) = (HSTRING::from(file_type), HSTRING::from(patterns.join(";")));
    // SAFETY: every string outlives the calls that read it; the dialog copies them.
    unsafe {
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
            pszName: PCWSTR(file_type.as_ptr()),
            pszSpec: PCWSTR(pattern.as_ptr()),
        }])?;
        dialog.SetOptions(dialog.GetOptions()? | options | FOS_FORCEFILESYSTEM)?;
        if let Err(error) = dialog.Show(None) {
            return if error.code() == ERROR_CANCELLED.to_hresult() {
                Ok(None)
            } else {
                Err(error)
            };
        }
        let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = name.to_string();
        CoTaskMemFree(Some(name.0.cast_const().cast()));
        Ok(Some(PathBuf::from(path?)))
    }
}
