//! One file that records what a playback session actually did.
//!
//! [`set_diagnostic_sink`](crate::set_diagnostic_sink) lets an application
//! forward the failures playback swallows, but a field report needs more than a
//! callback: it needs a file it can attach. This module installs that sink and
//! writes one line per event, stamped in UTC and tagged with the thread that
//! produced it. The lines a session writes answer, in order:
//!
//! - which source was opened, with which decoder, and whether it is live;
//! - for HLS, which playlist and rendition were chosen and when each window was
//!   joined, including whether its sound came muxed into the window or from a
//!   separate audio playlist;
//! - whether a video picture was delivered ([`player.video`]);
//! - whether the audio track was opened and whether PCM reached the device
//!   ([`mf_audio.setup`], [`mf_audio.first_sample`]);
//! - every failure, at the point it happened, with the reason the backend gave.
//!
//! Logging is opt-in and additive: nothing is written and no behaviour changes
//! unless [`start_log_file`] is called. Writing never panics and never blocks
//! playback on a slow disk for longer than one line.
//!
//! ```no_run
//! # fn run() -> std::io::Result<()> {
//! cranpose_video::start_log_file("playback.log")?;
//! // Open and play a source; the log now records what both tracks did.
//! cranpose_video::stop_log_file();
//! # Ok(())
//! # }
//! ```

use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;

/// The open log file, when one is recording.
static LOG: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();

struct Sink {
    file: File,
    path: PathBuf,
}

fn slot() -> &'static Mutex<Option<Sink>> {
    LOG.get_or_init(|| Mutex::new(None))
}

/// Starts recording every diagnostic into `path`, appending to an existing
/// file.
///
/// The file is created, together with its parent directory when one is named,
/// and a header line marks the moment recording began. Replacing a sink the
/// application installed itself is intentional: a session has one log, and this
/// call is the one that owns it. Calling this twice closes the previous file and
/// opens the new one.
///
/// # Errors
///
/// Returns the I/O error when the file or its directory cannot be created.
pub fn start_log_file(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(
        file,
        "# cranpose-video log opened {} platform={} decoder={}",
        timestamp(),
        std::env::consts::OS,
        crate::decoder_name()
    )?;
    file.flush()?;
    *slot().lock() = Some(Sink {
        file,
        path: path.clone(),
    });
    crate::diagnostics::set_diagnostic_sink(Some(Box::new(|level, target, message| {
        // The sink forwards here rather than to the file directly, so a line
        // written by the library and one written by the application share the
        // same stamp, thread tag and redaction.
        note(level, target, message);
    })));
    note(
        "INFO",
        "log.open",
        &format!("recording to {}", path.display()),
    );
    Ok(())
}

/// Stops recording and closes the file.
///
/// The diagnostic sink is removed as well, so a session that stops logging
/// goes back to swallowing its diagnostics exactly as it did before.
pub fn stop_log_file() {
    let mut guard = slot().lock();
    if let Some(sink) = guard.as_mut() {
        write_into(sink, "INFO", "log.close", "recording stopped");
    }
    *guard = None;
    drop(guard);
    crate::diagnostics::set_diagnostic_sink(None);
}

/// Whether a log file is currently recording.
pub fn is_logging() -> bool {
    slot().lock().is_some()
}

/// The path of the file being written, when one is recording.
pub fn log_path() -> Option<PathBuf> {
    slot().lock().as_ref().map(|sink| sink.path.clone())
}

/// Writes one line of the application's own into the log.
///
/// Use it to mark what the application did around playback — the screen it
/// opened, the gesture that switched clips — so the library's lines can be read
/// in context. A URL in `message` is redacted the same way the library's own
/// diagnostics are. A call with no file open writes nothing.
pub fn log_note(level: &str, target: &str, message: &str) {
    let level = match level {
        "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE" => level,
        _ => "INFO",
    };
    note(level, target, message);
}

/// Forwards one line, redacted and stamped, to the open file.
pub(crate) fn note(level: &str, target: &str, message: &str) {
    let mut guard = slot().lock();
    let Some(sink) = guard.as_mut() else {
        return;
    };
    write_into(sink, level, target, message);
}

/// Writes one line and flushes it, so a crash or a killed process still leaves
/// the last thing that happened in the file.
fn write_into(sink: &mut Sink, level: &str, target: &str, message: &str) {
    let thread = std::thread::current();
    let thread = thread.name().unwrap_or("unnamed");
    // Redact here rather than at every call site: the application's own notes go
    // through this path, and a live address can carry a token in its query.
    let message = crate::diagnostics::redact_for_report(message);
    // A full disk or a closed handle must not propagate into a decode thread; the
    // line is dropped and playback continues.
    let _ = writeln!(
        sink.file,
        "{} {:<5} {:<18} [{}] {}",
        timestamp(),
        level,
        target,
        thread,
        message
    );
    let _ = sink.file.flush();
}

/// The current UTC instant as `2026-11-20T14:22:01.123Z`.
fn timestamp() -> String {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    format_instant(since.as_secs(), since.subsec_millis())
}

/// Formats one instant counted from the Unix epoch.
fn format_instant(seconds: u64, millis: u32) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// The civil date of a count of days since 1970-01-01.
///
/// Howard Hinnant's `civil_from_days`, which is exact for every date the
/// proleptic Gregorian calendar defines and needs no table or dependency.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_instant_is_stamped_in_utc() {
        // 2023-11-14T22:13:20Z, the instant 1_700_000_000 counts.
        assert_eq!(format_instant(1_700_000_000, 0), "2023-11-14T22:13:20.000Z");
        assert_eq!(format_instant(1_700_000_000, 7), "2023-11-14T22:13:20.007Z");
        // The epoch itself, and a leap day.
        assert_eq!(format_instant(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_instant(1_709_164_800, 0), "2024-02-29T00:00:00.000Z");
    }

    #[test]
    fn a_session_writes_its_lines_to_the_file() {
        let directory = std::env::temp_dir().join(format!(
            "cranpose-video-log-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("playback.log");
        let _ = std::fs::remove_file(&path);
        start_log_file(&path).expect("the log file is created");
        assert!(is_logging());
        assert_eq!(log_path().as_deref(), Some(path.as_path()));
        log_note(
            "WARN",
            "test.event",
            "sound failed for https://h/x.m3u8?t=secret",
        );
        stop_log_file();
        assert!(!is_logging());
        // A note after the file is closed writes nothing.
        log_note("INFO", "test.event", "after close");

        let text = std::fs::read_to_string(&path).expect("the log is readable");
        assert!(text.contains("log.open"), "the header is recorded: {text}");
        assert!(
            text.contains("WARN  test.event"),
            "the note is recorded: {text}"
        );
        assert!(
            text.contains("https://h/x.m3u8?<redacted>"),
            "a token in the message is redacted: {text}"
        );
        assert!(
            !text.contains("after close"),
            "nothing is written after close"
        );
        assert!(
            text.lines().all(
                |line| line.starts_with('#') || (line.len() > 20 && line.as_bytes()[4] == b'-')
            ),
            "every line is stamped: {text}"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }
}
