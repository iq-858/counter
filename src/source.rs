use std::{path::Path, sync::Arc};

/// Where a video comes from.
///
/// A plain string that carries a scheme (`http://`, `https://`, `file://`) is
/// read as a URL. Anything else is read as a path on this device. A decoder
/// that cannot reach a scheme reports [`VideoError::UnsupportedOperation`].
///
/// [`VideoError::UnsupportedOperation`]: crate::VideoError::UnsupportedOperation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VideoSource {
    /// A file on this device.
    Path(Arc<Path>),
    /// A URL the platform decoder fetches itself.
    Url(Arc<str>),
    /// Bytes the caller already holds, such as a clip inside an application
    /// package or a buffer that arrived over a channel.
    Bytes(Arc<[u8]>),
}

impl VideoSource {
    /// A file on this device.
    pub fn path(path: impl AsRef<Path>) -> Self {
        Self::Path(Arc::from(path.as_ref()))
    }

    /// A URL the platform decoder fetches itself.
    pub fn url(url: impl Into<Arc<str>>) -> Self {
        Self::Url(url.into())
    }

    /// Bytes the caller already holds.
    pub fn bytes(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self::Bytes(bytes.into())
    }

    /// The file path, when the source is one.
    pub fn as_path(&self) -> Option<&Path> {
        match self {
            Self::Path(path) => Some(path),
            Self::Url(_) | Self::Bytes(_) => None,
        }
    }

    /// The URL text, when the source is one.
    pub fn as_url(&self) -> Option<&str> {
        match self {
            Self::Url(url) => Some(url),
            Self::Path(_) | Self::Bytes(_) => None,
        }
    }

    /// The bytes, when the source is a byte buffer.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            Self::Path(_) | Self::Url(_) => None,
        }
    }

    /// Whether the source is an HLS playlist.
    ///
    /// A playlist is a list of fragments rather than one file, so a platform
    /// decoder that takes a file cannot open it on its own. The answer looks at
    /// the address only: the URL path must end in `.m3u8`, before any query or
    /// fragment it carries. Percent escapes in the path are decoded first. A
    /// caller that knows the source is HLS should use
    /// [`VideoPlayer::open_hls`](crate::VideoPlayer::open_hls) instead of
    /// relying on this suffix.
    pub fn is_hls(&self) -> bool {
        let Some(url) = self.as_url() else {
            return false;
        };
        let path = percent_decode(url_path(url).as_bytes());
        const SUFFIX: &[u8] = b".m3u8";
        path.len() >= SUFFIX.len() && path[path.len() - SUFFIX.len()..].eq_ignore_ascii_case(SUFFIX)
    }

    /// The source as the text a platform decoder takes, when it has one.
    ///
    /// A path becomes its own text and a URL stays as written. Bytes have no
    /// text form, and a decoder that needs one writes them to a temporary file
    /// first.
    pub fn as_text(&self) -> Option<String> {
        match self {
            Self::Path(path) => Some(path.to_string_lossy().into_owned()),
            Self::Url(url) => Some(url.to_string()),
            Self::Bytes(_) => None,
        }
    }
}

/// The path part of a URL, without its authority, query or fragment.
fn url_path(url: &str) -> &str {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = after_scheme
        .split_once('/')
        .map_or("", |(_, after_authority)| after_authority);
    path.split(['?', '#']).next().unwrap_or("")
}

/// Decodes the percent escapes a URL path may carry.
fn percent_decode(bytes: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Some((high, low)) = hex_byte(bytes[index + 1]).zip(hex_byte(bytes[index + 2])) {
                decoded.push(high << 4 | low);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    decoded
}

fn hex_byte(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Whether the text names a scheme a URL carries rather than a file path.
///
/// `C:\clips\a.mp4` and `/home/me/a.mp4` are paths. `https://host/a.mp4` and
/// `content://media/1` are URLs. A Windows drive letter is one character
/// followed by a colon, so it never reads as a scheme.
pub(crate) fn has_scheme(text: &str) -> bool {
    match text.find("://") {
        Some(index) => {
            let scheme = &text[..index];
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

impl From<&str> for VideoSource {
    fn from(text: &str) -> Self {
        if has_scheme(text) {
            Self::url(text)
        } else {
            Self::path(text)
        }
    }
}

impl From<String> for VideoSource {
    fn from(text: String) -> Self {
        if has_scheme(&text) {
            Self::Url(Arc::from(text))
        } else {
            Self::Path(Arc::from(Path::new(&text)))
        }
    }
}

impl From<&Path> for VideoSource {
    fn from(path: &Path) -> Self {
        Self::path(path)
    }
}

impl From<std::path::PathBuf> for VideoSource {
    fn from(path: std::path::PathBuf) -> Self {
        Self::Path(Arc::from(path))
    }
}

impl From<Arc<[u8]>> for VideoSource {
    fn from(bytes: Arc<[u8]>) -> Self {
        Self::Bytes(bytes)
    }
}

impl From<Vec<u8>> for VideoSource {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Bytes(Arc::from(bytes))
    }
}
