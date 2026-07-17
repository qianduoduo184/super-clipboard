use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{SecondsFormat, Utc};

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const LOG_GENERATIONS: usize = 3;

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

    write_line_to_path(path, line.as_bytes(), MAX_LOG_BYTES, LOG_GENERATIONS);
}

fn write_line_to_path(path: &Path, line: &[u8], max_bytes: u64, generations: usize) {
    if let Err(error) = try_write_line_to_path(path, line, max_bytes, generations) {
        eprintln!("diagnostics: failed to write {}: {error}", path.display());
        eprint!("{}", String::from_utf8_lossy(line));
    }
}

fn try_write_line_to_path(
    path: &Path,
    line: &[u8],
    max_bytes: u64,
    generations: usize,
) -> std::io::Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len().saturating_add(line.len() as u64) > max_bytes => {
            rotate_logs(path, generations)?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line)
}

fn rotate_logs(path: &Path, generations: usize) -> std::io::Result<()> {
    if generations == 0 {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }

    remove_if_exists(&generation_path(path, generations))?;
    for generation in (1..generations).rev() {
        let source = generation_path(path, generation);
        let target = generation_path(path, generation + 1);
        rename_if_exists(&source, &target)?;
    }
    rename_if_exists(path, &generation_path(path, 1))
}

fn rename_if_exists(source: &Path, target: &Path) -> std::io::Result<()> {
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn generation_path(path: &Path, generation: usize) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".{generation}"));
    PathBuf::from(value)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{generation_path, write_line_to_path, MAX_LOG_BYTES};

    fn temp_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "super-clipboard-diagnostics-{label}-{}",
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn appends_below_rotation_threshold() {
        let root = temp_dir("append");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("app.log");

        write_line_to_path(&path, b"first\n", MAX_LOG_BYTES, 3);
        write_line_to_path(&path, b"second\n", MAX_LOG_BYTES, 3);

        assert_eq!(fs::read(&path).expect("log"), b"first\nsecond\n");
        assert!(!generation_path(&path, 1).exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rotates_a_log_at_ten_mib_before_append() {
        let root = temp_dir("threshold");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("app.log");
        fs::File::create(&path)
            .expect("log")
            .set_len(MAX_LOG_BYTES)
            .expect("sparse threshold log");

        write_line_to_path(&path, b"next\n", MAX_LOG_BYTES, 3);

        assert_eq!(fs::read(&path).expect("new log"), b"next\n");
        assert_eq!(
            fs::metadata(generation_path(&path, 1))
                .expect("rotated log")
                .len(),
            MAX_LOG_BYTES
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rotation_keeps_three_ordered_generations_and_deletes_the_oldest() {
        let root = temp_dir("generations");
        fs::create_dir_all(&root).expect("root");
        let path = root.join("app.log");
        fs::write(&path, b"current").expect("current");
        fs::write(generation_path(&path, 1), b"one").expect("one");
        fs::write(generation_path(&path, 2), b"two").expect("two");
        fs::write(generation_path(&path, 3), b"three").expect("three");

        write_line_to_path(&path, b"new", 1, 3);

        assert_eq!(fs::read(&path).expect("new current"), b"new");
        assert_eq!(
            fs::read(generation_path(&path, 1)).expect("one"),
            b"current"
        );
        assert_eq!(fs::read(generation_path(&path, 2)).expect("two"), b"one");
        assert_eq!(fs::read(generation_path(&path, 3)).expect("three"), b"two");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn write_or_rotation_failure_does_not_panic() {
        let root = temp_dir("failure");
        fs::create_dir_all(&root).expect("root");
        let invalid_path = root.join("missing-parent").join("app.log");

        write_line_to_path(&invalid_path, b"line\n", 1, 3);

        assert!(!invalid_path.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }
}
