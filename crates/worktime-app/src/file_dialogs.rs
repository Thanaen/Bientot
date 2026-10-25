//! The save and open dialogs of the diagnostic export and of « Paramètres › Données ». On
//! Windows the common item dialog blocks until closed, so it runs on a thread of its own; off
//! Windows, where only the screenshots and the tests run, GPUI's own dialogs keep the flow.

use std::path::PathBuf;

use gpui::AsyncApp;

/// Asks where to save a file named `suggested` by default, offering only `extension` files
/// described as `file_type`: `None` once the dialog is cancelled.
pub async fn pick_save(
    suggested: String,
    file_type: String,
    extension: &'static str,
    cx: &mut AsyncApp,
) -> anyhow::Result<Option<PathBuf>> {
    #[cfg(windows)]
    {
        let _ = cx;
        off_thread(move || {
            worktime_windows::dialogs::pick_save_file(&suggested, &file_type, extension)
        })
        .await
    }
    #[cfg(not(windows))]
    {
        let _ = (file_type, extension);
        let directory = std::env::home_dir().unwrap_or_default();
        let chosen = cx.update(|cx| cx.prompt_for_new_path(&directory, Some(&suggested)));
        chosen.await?
    }
}

/// Asks which file, named with one of `extensions` and described as `file_type`, to open:
/// `None` once the dialog is cancelled.
pub async fn pick_open(
    file_type: String,
    extensions: &'static [&'static str],
    cx: &mut AsyncApp,
) -> anyhow::Result<Option<PathBuf>> {
    #[cfg(windows)]
    {
        let _ = cx;
        off_thread(move || worktime_windows::dialogs::pick_open_file(&file_type, extensions)).await
    }
    #[cfg(not(windows))]
    {
        let _ = (file_type, extensions);
        let chosen = cx.update(|cx| {
            cx.prompt_for_paths(gpui::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: None,
            })
        });
        Ok(chosen.await??.and_then(|paths| paths.into_iter().next()))
    }
}

#[cfg(windows)]
async fn off_thread(
    dialog: impl FnOnce() -> windows::core::Result<Option<PathBuf>> + Send + 'static,
) -> anyhow::Result<Option<PathBuf>> {
    let (picked, chosen) = futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("worktime-file-dialog".to_owned())
        .spawn(move || {
            picked.send(dialog()).ok();
        })?;
    Ok(chosen.await??)
}
