use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::VideoError;

/// Bytes a decoder can only read from a file, written to one that is removed
/// when the file goes.
///
/// Media Foundation takes a stream and the browser takes a `Blob`, so the Apple
/// and Linux backends, Android's demuxer, and the HLS reader on every platform
/// need a file. The extension comes from the container's own bytes, because
/// AVFoundation chooses a parser by the type the file name states.
pub(crate) struct TempFile {
    path: PathBuf,
}

impl TempFile {
    pub(crate) fn write(bytes: &[u8]) -> Result<Self, VideoError> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let unique = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos())
            .unwrap_or_default();
        let name = format!(
            "cranpose-video-{}-{unique}-{nanos}.{}",
            std::process::id(),
            extension(bytes)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, bytes).map_err(|error| {
            VideoError::Source(format!(
                "cannot write the clip to a temporary file: {error}"
            ))
        })?;
        Ok(Self { path })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The file name extension of a container, from the marker its first bytes
/// carry.
fn extension(bytes: &[u8]) -> &'static str {
    let starts = |marker: &[u8]| bytes.len() >= marker.len() && &bytes[..marker.len()] == marker;
    if starts(b"\x1aE\xdf\xa3") {
        // Matroska and WebM share the header; the DocType states which.
        if bytes.windows(4).take(64).any(|window| window == b"webm") {
            "webm"
        } else {
            "mkv"
        }
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"AVI ") {
        "avi"
    } else if bytes.get(4..8) == Some(b"ftyp") {
        "mp4"
    } else if starts(b"FLV") {
        "flv"
    } else if starts(b"\x00\x00\x01\xba") || starts(b"\x00\x00\x01\xb3") {
        "mpg"
    } else if transport_stream(bytes) {
        // MPEG-TS is 188-byte packets, each starting with a sync byte. HLS
        // names such segments without an `#EXT-X-MAP` init section, so the
        // joined file is a plain transport stream.
        "ts"
    } else {
        "mp4"
    }
}

/// Whether the bytes are an MPEG transport stream.
///
/// Packets are 188 bytes and each starts with `0x47`. Two in a row rule out a
/// coincidental match. Some encoders put a few junk or alignment bytes before
/// the first packet, so the pair may start a little into the file rather than
/// at zero; a short lead-in search keeps those streams from being mistaken for
/// an MP4 and handed to the wrong parser on platforms that choose by file type.
fn transport_stream(bytes: &[u8]) -> bool {
    const PACKET: usize = 188;
    // Only a small lead-in is plausible; anything longer is not a transport
    // stream and should not hide an MP4's own bytes behind a false match.
    const LEAD: usize = 16;
    let limit = LEAD.min(bytes.len().saturating_sub(2 * PACKET + 1));
    (0..=limit).any(|start| {
        bytes.get(start) == Some(&0x47)
            && bytes.get(start + PACKET) == Some(&0x47)
            && bytes.get(start + 2 * PACKET) == Some(&0x47)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(fill: u8) -> [u8; 188] {
        let mut packet = [fill; 188];
        packet[0] = 0x47;
        packet
    }

    #[test]
    fn a_plain_transport_stream_is_named_ts() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&packet(0x11));
        bytes.extend_from_slice(&packet(0x22));
        bytes.extend_from_slice(&packet(0x33));
        assert_eq!(extension(&bytes), "ts");
    }

    #[test]
    fn a_transport_stream_with_a_lead_in_is_still_named_ts() {
        // A few alignment bytes before the first sync byte must not make the
        // joined file look like an MP4 to a platform that reads the extension.
        let mut bytes = vec![0x00; 4];
        bytes.extend_from_slice(&packet(0x11));
        bytes.extend_from_slice(&packet(0x22));
        bytes.extend_from_slice(&packet(0x33));
        assert_eq!(extension(&bytes), "ts");
    }

    #[test]
    fn an_mp4_is_not_mistaken_for_a_transport_stream() {
        let mut bytes = vec![0, 0, 0, 0x18];
        bytes.extend_from_slice(b"ftypisom");
        bytes.resize(1024, 0);
        assert_eq!(extension(&bytes), "mp4");
    }
}
