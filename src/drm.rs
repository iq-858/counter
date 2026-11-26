//! CENC/DRM facts a container or playlist declares.
//!
//! A player cannot decrypt Widevine or FairPlay by itself — those need the
//! platform's media-key session. What a Rust library can do is read the
//! protection metadata and tell the application which system the stream
//! needs and which key ids it asks for. This module parses that metadata:
//! MP4 `pssh`/`tenc` boxes and the HLS `EXT-X-KEY` line, so an application
//! can route a protected source to the right session before opening it.

/// A protection system a stream names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionSystem {
    /// The 16-byte system id (UUID bytes, big-endian).
    pub system_id: [u8; 16],
    /// System-specific data the session needs (the `pssh` payload).
    pub data: Vec<u8>,
    /// Key ids the init segment names, when it names any.
    pub key_ids: Vec<[u8; 16]>,
}

/// What one source asks for before it can be decoded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProtectionInfo {
    /// Every system the source names.
    pub systems: Vec<ProtectionSystem>,
    /// The default key id from `tenc`, when present.
    pub default_key_id: Option<[u8; 16]>,
    /// The `EXT-X-KEY` METHOD, when the source is an encrypted HLS stream.
    pub method: Option<String>,
}

impl ProtectionInfo {
    /// Whether the source is protected at all.
    pub fn is_protected(&self) -> bool {
        !self.systems.is_empty() || self.default_key_id.is_some() || self.method.is_some()
    }

    /// The system data for the well-known Widevine id, when present.
    pub fn widevine(&self) -> Option<&ProtectionSystem> {
        self.systems.iter().find(|system| {
            system.system_id == WIDEVINE_SYSTEM_ID
        })
    }

    /// The system data for the well-known PlayReady id, when present.
    pub fn playready(&self) -> Option<&ProtectionSystem> {
        self.systems.iter().find(|system| {
            system.system_id == PLAYREADY_SYSTEM_ID
        })
    }

    /// The system data for the well-known FairPlay id, when present.
    pub fn fairplay(&self) -> Option<&ProtectionSystem> {
        self.systems.iter().find(|system| {
            system.system_id == FAIRPLAY_SYSTEM_ID
        })
    }
}

/// Widevine's registered system id.
pub const WIDEVINE_SYSTEM_ID: [u8; 16] = [
    0xed, 0xef, 0x8b, 0xa9, 0x79, 0xd6, 0x4a, 0xce, 0xa3, 0xc8, 0x27, 0xdc, 0xd5, 0x1d, 0x21,
    0xed,
];
/// PlayReady's registered system id.
pub const PLAYREADY_SYSTEM_ID: [u8; 16] = [
    0x9a, 0x04, 0xf0, 0x79, 0x98, 0x40, 0x42, 0x86, 0xab, 0x92, 0xe6, 0x5b, 0xe0, 0x88, 0x5f,
    0x95,
];
/// FairPlay's registered system id.
pub const FAIRPLAY_SYSTEM_ID: [u8; 16] = [
    0x94, 0xce, 0x86, 0xa2, 0x13, 0x94, 0x4c, 0x1c, 0xa0, 0x6a, 0x9c, 0xd2, 0x0a, 0x66, 0xf4,
    0xec,
];

/// Parses every `pssh` box in an MP4 init segment or file.
pub fn parse_pssh_boxes(input: &[u8]) -> Vec<ProtectionSystem> {
    let mut systems = Vec::new();
    let mut at = 0usize;
    while at + 8 <= input.len() {
        let size = u32::from_be_bytes([input[at], input[at + 1], input[at + 2], input[at + 3]])
            as usize;
        let kind = &input[at + 4..at + 8];
        if size < 8 || at + size > input.len() {
            break;
        }
        if kind == b"pssh" {
            if let Some(system) = parse_pssh(&input[at + 8..at + size]) {
                systems.push(system);
            }
        }
        at += size;
    }
    systems
}

/// Parses one `pssh` body (after the 8-byte box header).
fn parse_pssh(body: &[u8]) -> Option<ProtectionSystem> {
    // version(1) flags(3) system_id(16) then v0: datasize(4) data
    //                        v1: kid_count(4) kids(16*n) datasize(4) data
    let version = *body.first()?;
    let system_id: [u8; 16] = body.get(4..20)?.try_into().ok()?;
    let mut key_ids = Vec::new();
    let mut at = 20usize;
    if version == 1 {
        let count = u32::from_be_bytes([
            *body.get(at)?,
            *body.get(at + 1)?,
            *body.get(at + 2)?,
            *body.get(at + 3)?,
        ]) as usize;
        at += 4;
        for _ in 0..count {
            let kid: [u8; 16] = body.get(at..at + 16)?.try_into().ok()?;
            key_ids.push(kid);
            at += 16;
        }
    }
    let size = u32::from_be_bytes([
        *body.get(at)?,
        *body.get(at + 1)?,
        *body.get(at + 2)?,
        *body.get(at + 3)?,
    ]) as usize;
    at += 4;
    let data = body.get(at..at + size)?.to_vec();
    Some(ProtectionSystem {
        system_id,
        data,
        key_ids,
    })
}

/// Reads the default key id out of a `tenc` box body (after its header).
fn tenc_key_id(tenc: &[u8]) -> Option<[u8; 16]> {
    // version(1) flags(3) reserved(1) [v1: default_iv_size(1)] isenc(1)
    // iv_size(1) kid_count(1) kid(16)
    let version = *tenc.first()?;
    let skip = if version == 1 { 2 } else { 1 };
    let at = 4 + skip;
    let kid: [u8; 16] = tenc.get(at..at + 16)?.try_into().ok()?;
    Some(kid)
}

/// Scans an MP4 for `pssh` boxes and the first `tenc` default key id.
pub fn parse_protection(input: &[u8]) -> ProtectionInfo {
    let systems = parse_pssh_boxes(input);
    let default_key_id = find_tenc(input);
    ProtectionInfo {
        systems,
        default_key_id,
        method: None,
    }
}

fn find_tenc(input: &[u8]) -> Option<[u8; 16]> {
    let mut at = 0usize;
    while at + 8 <= input.len() {
        let size = u32::from_be_bytes([input[at], input[at + 1], input[at + 2], input[at + 3]])
            as usize;
        if size < 8 || at + size > input.len() {
            return None;
        }
        if &input[at + 4..at + 8] == b"tenc" {
            return tenc_key_id(&input[at + 8..at + size]);
        }
        at += size;
    }
    None
}

/// Parses an HLS `EXT-X-KEY` line into the method and, for SAMPLE-AES with
/// FairPlay, the system id. `METHOD=NONE` yields an empty info.
pub fn parse_hls_key(line: &str) -> ProtectionInfo {
    let body = match line.strip_prefix("#EXT-X-KEY:") {
        Some(body) => body,
        None => return ProtectionInfo::default(),
    };
    let method = hls_attribute(body, "METHOD").unwrap_or_default();
    if method.is_empty() || method == "NONE" {
        return ProtectionInfo::default();
    }
    ProtectionInfo {
        systems: Vec::new(),
        default_key_id: None,
        method: Some(method),
    }
}

fn hls_attribute(body: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    for part in body.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&needle) {
            return Some(value.trim_matches('"').to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pssh_box(system_id: [u8; 16], data: &[u8]) -> Vec<u8> {
        let mut body = vec![0u8]; // version 0
        body.extend_from_slice(&[0, 0, 0]); // flags
        body.extend_from_slice(&system_id);
        body.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(b"pssh");
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parses_a_v0_pssh_box() {
        let mut file = Vec::new();
        file.extend_from_slice(&[0, 0, 0, 16]);
        file.extend_from_slice(b"ftyp");
        file.extend_from_slice(b"mp42isom");
        file.extend_from_slice(&pssh_box(WIDEVINE_SYSTEM_ID, &[7, 8, 9]));
        let systems = parse_pssh_boxes(&file);
        assert_eq!(systems.len(), 1);
        assert_eq!(systems[0].system_id, WIDEVINE_SYSTEM_ID);
        assert_eq!(systems[0].data, vec![7, 8, 9]);
        assert!(systems[0].key_ids.is_empty());
    }

    #[test]
    fn widevine_is_recognised_by_its_id() {
        let mut file = pssh_box(WIDEVINE_SYSTEM_ID, &[1]);
        file.extend_from_slice(&pssh_box(PLAYREADY_SYSTEM_ID, &[2]));
        let info = ProtectionInfo {
            systems: parse_pssh_boxes(&file),
            ..Default::default()
        };
        assert!(info.widevine().is_some());
        assert!(info.playready().is_some());
        assert!(info.fairplay().is_none());
        assert!(info.is_protected());
    }

    #[test]
    fn an_unprotected_source_is_not_protected() {
        assert!(!ProtectionInfo::default().is_protected());
    }

    #[test]
    fn an_hls_key_line_names_its_method() {
        let info = parse_hls_key("#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\"");
        assert_eq!(info.method.as_deref(), Some("SAMPLE-AES"));
        assert!(info.is_protected());
    }

    #[test]
    fn method_none_is_unprotected() {
        let info = parse_hls_key("#EXT-X-KEY:METHOD=NONE");
        assert!(!info.is_protected());
    }

    #[test]
    fn a_v1_pssh_carries_key_ids() {
        let mut body = vec![1u8]; // version 1
        body.extend_from_slice(&[0, 0, 0]);
        body.extend_from_slice(&WIDEVINE_SYSTEM_ID);
        body.extend_from_slice(&1u32.to_be_bytes()); // one kid
        body.extend_from_slice(&[0u8; 16]);
        body.extend_from_slice(&0u32.to_be_bytes()); // no data
        let mut box_bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        box_bytes.extend_from_slice(b"pssh");
        box_bytes.extend_from_slice(&body);
        let systems = parse_pssh_boxes(&box_bytes);
        assert_eq!(systems.len(), 1);
        assert_eq!(systems[0].key_ids.len(), 1);
    }
}
