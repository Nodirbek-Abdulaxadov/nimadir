//! The shell's own log: a file in the data directory, for the questions nobody
//! can answer once the window is gone — what was opened, what failed, what
//! panicked. A guest's `log` lines are not in it; those are the guest's.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG: Mutex<Option<File>> = Mutex::new(None);

/// Past this at start, the log is moved aside to `nimadir.log.1` and begun
/// again: 1 MiB.
const MAX_LOG: u64 = 1 << 20;

/// Open the log and route panics into it, ahead of whatever hook was there.
/// Without a data directory there is no log, and nothing else changes.
pub fn init() {
    if let Some(dir) = crate::paths::data_dir() {
        let path = dir.join("nimadir.log");
        let _ = std::fs::create_dir_all(&dir);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG) {
            let _ = std::fs::rename(&path, dir.join("nimadir.log.1"));
        }
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
            *LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(file);
        }
    }

    let earlier = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // `try_lock`: a panic while the log is held must not deadlock on it.
        if let Ok(mut log) = LOG.try_lock() {
            let trace = std::backtrace::Backtrace::force_capture();
            write(&mut log, &format!("panic: {info}\n{trace}"));
        }
        earlier(info);
    }));

    line(&format!(
        "nimadir {} starting (WIT {})",
        env!("CARGO_PKG_VERSION"),
        crate::wit_package()
    ));
}

/// One line in the log, stamped with the time in UTC. Escaped: what is logged
/// often carries an address or a reason that came from a guest, and a newline
/// in it would be a forged entry.
pub fn line(msg: &str) {
    write(
        &mut LOG.lock().unwrap_or_else(|e| e.into_inner()),
        &crate::text::visible(msg),
    );
}

fn write(log: &mut Option<File>, msg: &str) {
    if let Some(file) = log.as_mut() {
        let _ = writeln!(file, "{} {msg}", utc_now());
    }
}

/// `2026-10-20T18:44:05Z`, without a date library: days since the epoch to a
/// civil date is Howard Hinnant's algorithm.
fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}
