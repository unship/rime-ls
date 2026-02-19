//! File-based logging for rime-ls. Writes to log file instead of stdout/stderr.

use once_cell::sync::OnceCell;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

static LOGGER: OnceCell<Mutex<std::fs::File>> = OnceCell::new();

/// Initialize logger to write to the given directory. Creates rime-ls.log in that directory.
/// No-op if already initialized or if path is invalid.
pub fn init(log_dir: impl AsRef<Path>) {
    if LOGGER.get().is_some() {
        return;
    }
    let log_dir = log_dir.as_ref();
    if let Err(_) = std::fs::create_dir_all(log_dir) {
        return;
    }
    let log_path = log_dir.join("rime-ls.log");
    let file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(_) => return,
    };
    let _ = LOGGER.set(Mutex::new(file));
}

fn write_log(level: &str, msg: &str) {
    if let Some(guard) = LOGGER.get() {
        if let Ok(mut file) = guard.lock() {
            let _ = writeln!(
                file,
                "{} [{}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                level,
                msg
            );
            let _ = file.flush();
        }
    }
}

/// Log an info message to file.
pub fn info(msg: impl AsRef<str>) {
    write_log("INFO", msg.as_ref());
}

/// Log an error message to file.
pub fn error(msg: impl AsRef<str>) {
    write_log("ERROR", msg.as_ref());
}
