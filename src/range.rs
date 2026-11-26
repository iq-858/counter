//! HTTP byte-range fetching with generation-safe cancellation.
//!
//! The preload coordinator decides *how far* to prepare an item; this module
//! is the byte layer that actually performs the partial downloads. It speaks
//! HTTP/1.1 `Range` requests over the same blocking `ureq` client the HLS
//! module already uses, so no new dependency is introduced.
//!
//! Every request carries a generation id. When the feed selects a different
//! item, the caller bumps the generation and late responses for the old one
//! are dropped instead of landing in the wrong slot — the same discipline
//! `PreloadManager` and `VideoFeedController` already enforce.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How a range fetch ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeOutcome {
    /// The bytes were fetched and stored.
    Fetched {
        /// First byte offset of the returned window.
        start: u64,
        /// The bytes themselves.
        bytes: Arc<[u8]>,
    },
    /// The server ignored the Range header and returned the whole body.
    FullBody(Arc<[u8]>),
    /// A newer generation superseded this request before it completed.
    Superseded,
    /// The server answered but the fetch failed.
    Failed(String),
}

/// One partial fetch of a URL.
#[derive(Debug, Clone)]
pub struct RangeRequest {
    /// Absolute URL to fetch.
    pub url: String,
    /// First byte offset wanted.
    pub start: u64,
    /// Number of bytes wanted from `start`.
    pub length: u64,
    /// Generation the caller assigned; a stale generation is dropped.
    pub generation: u64,
}

impl RangeRequest {
    /// A request for the first `length` bytes of `url`.
    pub fn head(url: impl Into<String>, length: u64, generation: u64) -> Self {
        Self {
            url: url.into(),
            start: 0,
            length,
            generation,
        }
    }
}

/// Blocking byte-range client over `ureq`.
pub struct RangeFetcher {
    agent: ureq::Agent,
    timeout: Duration,
    /// Monotonic counter the caller can bump to supersede in-flight work.
    generation: AtomicU64,
}

impl Default for RangeFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl RangeFetcher {
    /// A fetcher with a 10 second timeout.
    pub fn new() -> Self {
        Self {
            agent: ureq::AgentBuilder::new().build(),
            timeout: Duration::from_secs(10),
            generation: AtomicU64::new(0),
        }
    }

    /// The current generation.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Bumps the generation, superseding every in-flight request.
    pub fn supersede(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Performs one range fetch. Blocking; call from a worker thread.
    pub fn fetch(&self, request: &RangeRequest) -> RangeOutcome {
        if request.generation != self.generation() {
            return RangeOutcome::Superseded;
        }
        let range = format!("bytes={}-{}", request.start, request.start + request.length - 1);
        let response = match self
            .agent
            .get(&request.url)
            .timeout(self.timeout)
            .set("Range", &range)
            .call()
        {
            Ok(response) => response,
            Err(error) => return RangeOutcome::Failed(error.to_string()),
        };
        // A server that ignores Range answers 200 with the whole body.
        if response.status() != 206 {
            let mut bytes = Vec::new();
            if response.into_reader().take(8 << 20).read_to_end(&mut bytes).is_err() {
                return RangeOutcome::Failed("body read failed".to_owned());
            }
            return RangeOutcome::FullBody(bytes.into());
        }
        let mut bytes = Vec::new();
        if response
            .into_reader()
            .take(request.length as u64 + 1)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return RangeOutcome::Failed("body read failed".to_owned());
        }
        RangeOutcome::Fetched {
            start: request.start,
            bytes: bytes.into(),
        }
    }
}

use std::io::Read;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superseding_bumps_the_generation() {
        let fetcher = RangeFetcher::new();
        let first = fetcher.generation();
        assert_eq!(fetcher.supersede(), first + 1);
        assert_eq!(fetcher.generation(), first + 1);
    }

    #[test]
    fn a_stale_generation_is_dropped_without_network() {
        let fetcher = RangeFetcher::new();
        let request = RangeRequest::head("http://127.0.0.1:1/clip.mp4", 1024, 99);
        assert_eq!(fetcher.fetch(&request), RangeOutcome::Superseded);
    }
}
