use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};

const DEFAULT_SIZE_CAP: u64 = 10 * 1024 * 1024;

static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

pub fn setup(config_path: &str, log_name: &str) -> Result<()> {
    setup_with_size_cap(config_path, log_name, DEFAULT_SIZE_CAP)
}

pub fn setup_with_size_cap(config_path: &str, log_name: &str, size_cap: u64) -> Result<()> {
    let dir = Path::new(config_path)
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let path = dir.join(log_name);
    if std::fs::metadata(&path)
        .map(|metadata| metadata.len() > size_cap)
        .unwrap_or(false)
    {
        let rotated = rotated_path(&path);
        let _ = std::fs::rename(&path, rotated);
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open log {}", path.display()))?;
    *LOG_FILE
        .lock()
        .map_err(|_| anyhow::anyhow!("log file mutex poisoned"))? = Some(file);
    Ok(())
}

pub fn line(args: std::fmt::Arguments<'_>) {
    eprintln!("{args}");
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "{args}");
            let _ = file.flush();
        }
    }
}

fn rotated_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.1", path.to_string_lossy()))
}

#[macro_export]
macro_rules! mars_log {
    ($($arg:tt)*) => {{
        $crate::logsink::line(format_args!($($arg)*));
    }};
}
