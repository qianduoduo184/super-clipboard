use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

pub fn register_shortcut(
    app: &AppHandle,
    shortcut_text: &str,
    current_shortcut: Arc<Mutex<Option<String>>>,
) -> anyhow::Result<()> {
    validate_shortcut(shortcut_text)?;
    crate::diagnostics::info(format!("shortcuts: registering {shortcut_text}"));
    let shortcut: Shortcut = shortcut_text.parse()?;
    let handle = app.clone();
    let shortcut_for_handler = shortcut_text.to_string();

    app.global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, _event| {
            if let Some(window) = handle.get_webview_window("main") {
                crate::diagnostics::info(format!(
                    "shortcuts: {shortcut_for_handler} pressed, showing main window"
                ));
                let _ = window.show();
                let _ = window.set_focus();
            }
        })?;
    if let Ok(mut guard) = current_shortcut.lock() {
        *guard = Some(shortcut_text.to_string());
    }

    Ok(())
}

pub fn replace_shortcut(
    app: &AppHandle,
    next_shortcut: &str,
    current_shortcut: Arc<Mutex<Option<String>>>,
) -> anyhow::Result<()> {
    validate_shortcut(next_shortcut)?;
    let previous_shortcut = current_shortcut
        .lock()
        .map_err(|error| anyhow::anyhow!("shortcut lock poisoned: {error}"))?
        .clone();

    if let Some(previous) = previous_shortcut.as_deref() {
        crate::diagnostics::info(format!("shortcuts: unregistering {previous}"));
        match previous.parse::<Shortcut>() {
            Ok(shortcut) => {
                if let Err(error) = app.global_shortcut().unregister(shortcut) {
                    crate::diagnostics::warn(format!(
                        "shortcuts: unregister failed for {previous}: {error}"
                    ));
                }
            }
            Err(error) => {
                crate::diagnostics::warn(format!(
                    "shortcuts: previous shortcut parse failed for {previous}: {error}"
                ));
            }
        }
        if let Ok(mut guard) = current_shortcut.lock() {
            *guard = None;
        }
    }

    match register_shortcut(app, next_shortcut, current_shortcut.clone()) {
        Ok(()) => Ok(()),
        Err(error) => {
            crate::diagnostics::error(format!(
                "shortcuts: failed to register {next_shortcut}: {error}"
            ));
            if let Some(previous) = previous_shortcut {
                let _ = register_shortcut(app, &previous, current_shortcut);
            }
            Err(error)
        }
    }
}

pub fn validate_shortcut(shortcut_text: &str) -> anyhow::Result<()> {
    let parts = shortcut_text
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();

    let has_modifier = parts.iter().any(|part| matches_modifier(part));
    let has_key = parts.iter().any(|part| !matches_modifier(part));

    if !has_modifier || !has_key {
        return Err(anyhow::anyhow!(
            "shortcut must include at least one modifier and one key"
        ));
    }

    let _: Shortcut = shortcut_text.parse()?;
    Ok(())
}

fn matches_modifier(value: &str) -> bool {
    value.eq_ignore_ascii_case("ctrl")
        || value.eq_ignore_ascii_case("control")
        || value.eq_ignore_ascii_case("alt")
        || value.eq_ignore_ascii_case("shift")
        || value.eq_ignore_ascii_case("meta")
        || value.eq_ignore_ascii_case("super")
        || value.eq_ignore_ascii_case("cmd")
        || value.eq_ignore_ascii_case("command")
}
