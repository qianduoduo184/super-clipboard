use tauri::{App, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

pub fn register_default_shortcuts(app: &App) -> anyhow::Result<()> {
    let shortcut: Shortcut = "Ctrl+Shift+V".parse()?;
    let handle = app.handle().clone();

    app.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, _event| {
        if let Some(window) = handle.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
    })?;

    Ok(())
}
