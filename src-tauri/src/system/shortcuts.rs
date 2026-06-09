use tauri::{App, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

pub fn register_default_shortcuts(app: &App) -> anyhow::Result<()> {
    crate::diagnostics::info("shortcuts: registering Ctrl+Shift+V");
    let shortcut: Shortcut = "Ctrl+Shift+V".parse()?;
    let handle = app.handle().clone();

    app.global_shortcut().on_shortcut(shortcut, move |_app, _shortcut, _event| {
        if let Some(window) = handle.get_webview_window("main") {
            crate::diagnostics::info("shortcuts: Ctrl+Shift+V pressed, showing main window");
            let _ = window.show();
            let _ = window.set_focus();
        }
    })?;

    Ok(())
}
