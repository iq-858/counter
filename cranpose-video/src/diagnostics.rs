//! Optional runtime diagnostic sink.
//!
//! Playback degrades deliberately: a busy audio device, a rejected PCM layout
//! or a source that has no audio track all leave the picture running and are
//! otherwise silent. That is the right behaviour for a player, but it makes a
//! field failure impossible to explain after the fact. This module adds an
//! opt-in hook so an application can forward those swallowed failures to its
//! own log.
//!
//! Nothing is written unless the application registers a sink, and registering
//! one never changes playback behaviour. The callback runs on whichever thread
//! detected the condition — usually the decode thread — so it must be quick
//! and thread-safe.

use std::sync::OnceLock;

use parking_lot::Mutex;

type Sink = Box<dyn Fn(&str, &str, &str) + Send + Sync>;

static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<Sink>> {
    SINK.get_or_init(|| Mutex::new(None))
}

/// Sets, replaces, or removes the process-wide diagnostic sink.
///
/// The callback receives three borrowed strings: a level (`"ERROR"`, `"WARN"`
/// or `"INFO"`), a short subsystem tag such as `"mf_audio.setup"`, and the
/// human-readable detail. Pass `None` to stop forwarding. Replacing the sink
/// while events are being produced is safe; a callback already in progress
/// finishes with the sink it captured.
///
/// This is purely additive: the player behaves identically whether or not a
/// sink is installed.
pub fn set_diagnostic_sink(sink: Option<Box<dyn Fn(&str, &str, &str) + Send + Sync>>) {
    *slot().lock() = sink;
}

/// Forwards one diagnostic to the registered sink, if any.
pub(crate) fn diag(level: &'static str, target: &'static str, message: &str) {
    let guard = slot().lock();
    let Some(sink) = guard.as_ref() else {
        return;
    };
    // A diagnostic can name a live URL whose query carries a token. Sanitize at
    // the single choke point so every backend, including ones added later, gets
    // the same protection without changing its call sites.
    let redacted = redact_for_report(message);
    sink(level, target, &redacted);
}

/// Removes credentials and query material from a message before it is recorded.
///
/// Diagnostic events and session histories outlive the request that produced
/// them. A live URL can carry a token in its query or user-info; keeping those
/// characters would turn a decode failure into a leaked credential. Host and
/// path are left intact so the failure is still attributable to a source.
pub(crate) fn redact_for_report(message: &str) -> String {
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn a_sink_can_be_installed_and_removed() {
        static SEEN: AtomicUsize = AtomicUsize::new(0);
        set_diagnostic_sink(Some(Box::new(|level, target, message| {
            assert_eq!(level, "INFO");
            assert_eq!(target, "diagnostics.test");
            assert_eq!(message, "hello");
            SEEN.fetch_add(1, Ordering::SeqCst);
        })));
        diag("INFO", "diagnostics.test", "hello");
        set_diagnostic_sink(None);
        // Must be a no-op after removal.
        diag("INFO", "diagnostics.test", "never forwarded");
        assert_eq!(SEEN.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn forwarded_diagnostics_are_redacted() {
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        set_diagnostic_sink(Some(Box::new(|_level, _target, message| {
            SEEN.lock().push(message.to_owned());
        })));
        diag(
            "WARN",
            "diagnostics.test",
            "open https://host.example/live.m3u8?token=secret failed",
        );
        set_diagnostic_sink(None);
        let seen = SEEN.lock().clone();
        assert_eq!(
            seen,
            vec!["open https://host.example/live.m3u8?<redacted> failed".to_owned()]
        );
    }

    #[test]
    fn report_redaction_removes_credentials_without_losing_the_source() {
        assert_eq!(
            redact_for_report(
                "cannot read https://user:secret@host.example/live.m3u8?token=abc: HTTP 403"
            ),
            "cannot read https://***@host.example/live.m3u8?<redacted> HTTP 403"
        );
        assert_eq!(
            redact_for_report("playlist https://host.example/a.m3u8#fragment failed"),
            "playlist https://host.example/a.m3u8#<redacted> failed"
        );
        assert_eq!(
            redact_for_report("local clip.mp4 failed"),
            "local clip.mp4 failed"
        );
        assert_eq!(
            redact_for_report("https://one.example/a?x=1 then https://two.example/b?y=2"),
            "https://one.example/a?<redacted> then https://two.example/b?<redacted>"
        );
    }
}
