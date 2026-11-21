//! Runtime diagnostic log.
//!
//! This exists for field diagnosis: the app can freeze and recover on a device
//! we cannot run here, and the interesting facts are the ones printed in the
//! seconds before it happens. Every line is appended and flushed immediately,
//! so the tail survives a crash.
//!
//! The file is `diagnostics.log` in the process working directory (usually the
//! folder you launched the app from) and rotates once to `diagnostics.log.old`
//! after 2 MiB. The absolute path is printed to stderr at startup.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

/// Size at which the current log is moved aside.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

static PATH: OnceLock<PathBuf> = OnceLock::new();

/// A small in-memory ring of recent WARN/ERROR/PANIC lines, so the UI can show
/// background failures that a phone user would otherwise never see (the file
/// log lives in a sandbox the user cannot open). Bounded so a long session
/// cannot grow without limit.
const SCREEN_CAP: usize = 80;
static SCREEN: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn screen() -> &'static Mutex<Vec<String>> {
    SCREEN.get_or_init(|| Mutex::new(Vec::with_capacity(SCREEN_CAP)))
}

/// Returns a snapshot of the most recent lines (oldest first) for the UI.
pub(crate) fn recent_lines() -> Vec<String> {
    screen().lock().map(|lines| lines.clone()).unwrap_or_default()
}

/// Clears the on-screen history (the file log is never touched).
pub(crate) fn clear_screen() {
    if let Ok(mut lines) = screen().lock() {
        lines.clear();
    }
}

fn log_path() -> &'static PathBuf {
    PATH.get_or_init(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("diagnostics.log")
    })
}

/// Installs the panic hook and writes the startup banner.
///
/// Call once, before any composable runs. A panic is written with its thread,
/// source location and a forced backtrace before the default hook runs, so a
/// crash that restarts the app still leaves the reason on disk.
pub(crate) fn init() {
    install_panic_hook();
    info(
        "startup",
        &format!(
            "pid={} exe={} args={:?} log={}",
            std::process::id(),
            std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "<unknown>".to_owned()),
            std::env::args().collect::<Vec<_>>(),
            log_path().display()
        ),
    );
    eprintln!("[diagnostics] writing runtime log to {}", log_path().display());
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = if let Some(text) = payload.downcast_ref::<&str>() {
            (*text).to_owned()
        } else if let Some(text) = payload.downcast_ref::<String>() {
            text.clone()
        } else {
            "<non-string panic payload>".to_owned()
        };
        let location = info
            .location()
            .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()))
            .unwrap_or_else(|| "<unknown location>".to_owned());
        let thread = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_owned();
        let backtrace = std::backtrace::Backtrace::force_capture();
        log(
            "PANIC",
            &thread,
            &format!("{message} @ {location}\nbacktrace:\n{backtrace}"),
        );
        previous(info);
    }));
}

/// Appends one timestamped line and flushes it.
pub(crate) fn log(level: &str, target: &str, message: &str) {
    let message = redact_for_report(message);
    let path = log_path();
    // Open-append makes each line one atomic OS write, so no mutex is needed
    // and the panic hook can never deadlock against another logging thread.
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.len() > MAX_BYTES {
            let _ = fs::rename(path, path.with_extension("log.old"));
        }
    }
    let line = format!(
        "{} [{}] [{}] {}\n",
        timestamp(),
        level,
        target,
        message.replace('\n', "\n           | ")
    );
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
    }
    // Mirror every line to the in-memory ring so the on-screen view is a full
    // timeline, not just failures. The level prefix lets the UI colour errors.
    // The file log above keeps the complete, unabridged record.
    if let Ok(mut lines) = screen().lock() {
        if lines.len() >= SCREEN_CAP {
            lines.remove(0);
        }
        let short = format!(
            "{} {} [{}] {}",
            &timestamp()[11..23],
            level,
            target,
            message.replace('\n', " ")
        );
        lines.push(short);
    }
}

pub(crate) fn error(target: &str, message: impl AsRef<str>) {
    log("ERROR", target, message.as_ref());
}

pub(crate) fn warn(target: &str, message: impl AsRef<str>) {
    log("WARN", target, message.as_ref());
}

pub(crate) fn info(target: &str, message: impl AsRef<str>) {
    log("INFO", target, message.as_ref());
}

/// UTC timestamp with millisecond precision, without a date-time dependency.
fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as i64;
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        second_of_day / 3_600,
        (second_of_day % 3_600) / 60,
        second_of_day % 60,
        now.subsec_millis()
    )
}

/// Howard Hinnant's civil-from-days conversion; `days` is days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u32;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = i64::from(year_of_era) + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (year + i64::from(month <= 2), month, day)
}

/// Removes user-info and query/fragment material before a message is written.
///
/// The app logs source keys, which for Live are third-party URLs. Those URLs
/// can carry tokens; a field log must not become a credential leak. Host and
/// path survive so a failure is still attributable to the right channel.
fn redact_for_report(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(scheme_at) = rest.find("://") {
        let start = rest[..scheme_at]
            .rfind(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | '[' | '<'))
            .map_or(0, |at| at + 1);
        out.push_str(&rest[..start]);
        let from = &rest[start..];
        let end = from
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']' | '>'))
            .unwrap_or(from.len());
        out.push_str(&redact_url(&from[..end]));
        rest = &from[end..];
    }
    out.push_str(rest);
    out
}

fn redact_url(url: &str) -> String {
    let Some(scheme_at) = url.find("://") else {
        return url.to_owned();
    };
    let scheme = &url[..scheme_at + 3];
    let tail = &url[scheme_at + 3..];
    let (authority_path, sensitive) = match tail.find(['?', '#']) {
        Some(at) => (&tail[..at], Some(&tail[at..])),
        None => (tail, None),
    };
    let host_path = match authority_path.rfind('@') {
        Some(at) => format!("{scheme}***@{}", &authority_path[at + 1..]),
        None => format!("{scheme}{authority_path}"),
    };
    match sensitive {
        Some(value) if value.starts_with('?') => format!("{host_path}?<redacted>"),
        Some(_) => format!("{host_path}#<redacted>"),
        None => host_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_redaction_keeps_the_channel_but_drops_tokens() {
        assert_eq!(
            redact_for_report("https://user:token@host.example/live.m3u8?token=secret failed"),
            "https://***@host.example/live.m3u8?<redacted> failed"
        );
        assert_eq!(
            redact_for_report("local clip.mp4 failed"),
            "local clip.mp4 failed"
        );
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_680), (2023, 11, 19));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }
}
