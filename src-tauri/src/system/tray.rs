use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, Emitter, Manager};

pub fn setup(app: &App) -> anyhow::Result<()> {
    crate::diagnostics::info("tray: building tray menu");
    let show = MenuItem::with_id(app, "show", "显示", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &settings, &quit])?;

    TrayIconBuilder::new()
        .icon(
            app.default_window_icon()
                .ok_or_else(|| anyhow!("default window icon not configured"))?
                .clone(),
        )
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                crate::diagnostics::info("tray: show menu selected");
                show_main_window(app);
            }
            "settings" => {
                crate::diagnostics::info("tray: settings menu selected");
                show_main_window(app);
                if let Err(error) = app.emit("open-settings", ()) {
                    crate::diagnostics::warn(format!("tray: emit open-settings failed: {error}"));
                }
            }
            "quit" => {
                crate::diagnostics::info("tray: quit menu selected");
                app.exit(0)
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::diagnostics::info("tray: left click, showing main window");
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
