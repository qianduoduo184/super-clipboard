use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{SecondsFormat, Utc};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

pub fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        error(format!("panic: {panic_info}"));
        default_hook(panic_info);
    }));
}

pub fn init(app_data: &Path) -> anyhow::Result<PathBuf> {
    let log_dir = app_data.join("logs");
    fs::create_dir_all(&log_dir)?;
    let path = log_dir.join("super-clipboard.log");
    let _ = LOG_PATH.set(path.clone());

    info("============================================================");
    info(format!(
        "super-clipboard starting version={} pid={}",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));
    info(format!("app_data_dir={}", app_data.display()));
    info(format!("log_path={}", path.display()));

    Ok(path)
}

pub fn log_path() -> Option<PathBuf> {
    LOG_PATH.get().cloned()
}

pub fn info(message: impl AsRef<str>) {
    write_line("INFO", message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    write_line("WARN", message.as_ref());
}

pub fn error(message: impl AsRef<str>) {
    write_line("ERROR", message.as_ref());
}

fn write_line(level: &str, message: &str) {
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let line = format!("{timestamp} [{level}] {message}\n");

    let Some(path) = LOG_PATH.get() else {
        eprint!("{line}");
        return;
    };

    let Ok(_guard) = LOG_LOCK.lock() else {
        eprint!("{line}");
        return;
    };

    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut file) => {
            let _ = file.write_all(line.as_bytes());
        }
        Err(_) => {
            eprint!("{line}");
        }
    }
}
