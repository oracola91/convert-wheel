//! A small log file, so there is a trace when a notification can't be shown or
//! something fails with no window to say so. File names only, never full paths.
//! macOS: ~/Library/Logs/local.convertwheel; Windows: %LOCALAPPDATA%\local.convertwheel\logs;
//! Linux: ~/.local/share/local.convertwheel/logs.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// The log is started over once it passes this size; the old one is kept as .old.
const MAX_BYTES: u64 = 512 * 1024;

static FILE: OnceLock<PathBuf> = OnceLock::new();
static WRITE: Mutex<()> = Mutex::new(());

pub fn init(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    let _ = FILE.set(dir.join("convert-wheel.log"));
}

pub fn line(message: &str) {
    eprintln!("convert-wheel: {message}");
    let Some(file) = FILE.get() else {
        return;
    };
    let _guard = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    if fs::metadata(file).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = fs::rename(file, file.with_extension("log.old"));
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "{} {message}", timestamp());
    }
}

/// UTC time as 2026-10-23T12:34:56Z, without a date library.
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Days since 1970-01-01 to a calendar date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_come_out_right() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_384), (2025, 10, 23));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }
}
