//! The base media decode time a fragment states, and moving it back to zero.
//!
//! An HLS fragment states, in its `tfdt` box, the time its first sample plays
//! at, counted from the start of the media the stream was cut from. The
//! platform decoders expect the file they open to start at zero, so the crate
//! subtracts the first fragment's time from every fragment in the window it
//! writes. The relative timing, which is what paces playback, stays as it was.

/// One box's header, and the span of its body inside the buffer.
struct Header {
    /// The four-character box type.
    kind: [u8; 4],
    /// The offset of the first byte after the header.
    body: usize,
    /// The offset one past the last byte of the box.
    end: usize,
}

/// The boxes laid out between `start` and `end`.
///
/// A size of zero states that the box runs to the end of its parent, and a size
/// of one states a 64-bit size after the type, which is how a fragment too large
/// for 32 bits states its own.
fn boxes(bytes: &[u8], start: usize, end: usize) -> Vec<Header> {
    let mut out = Vec::new();
    let mut at = start;
    while at.saturating_add(8) <= end {
        let size = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        let kind = [bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]];
        let (header, size) = match size {
            0 => (8usize, u64::try_from(end - at).unwrap_or(u64::MAX)),
            1 if at.saturating_add(16) <= end => {
                let mut large = [0u8; 8];
                large.copy_from_slice(&bytes[at + 8..at + 16]);
                (16usize, u64::from_be_bytes(large))
            }
            size => (8usize, u64::from(size)),
        };
        let Ok(length) = usize::try_from(size) else {
            break;
        };
        if length < header {
            break;
        }
        let end = at.saturating_add(length).min(end);
        out.push(Header {
            kind,
            body: at + header,
            end,
        });
        at = end;
    }
    out
}

/// The offsets of every `tfdt` body a fragment carries.
///
/// A fragment holds one movie fragment, which holds one track fragment per
/// track. This crate reads video only, but a stream may carry other tracks, so
/// every track fragment is visited.
fn bodies(bytes: &[u8], start: usize, end: usize, depth: u8, out: &mut Vec<usize>) {
    if depth > 4 {
        return;
    }
    for header in boxes(bytes, start, end) {
        if &header.kind == b"tfdt" {
            out.push(header.body);
        } else if matches!(&header.kind, b"moof" | b"traf") {
            bodies(bytes, header.body, header.end, depth + 1, out);
        }
    }
}

/// The base media decode time of every track fragment in `fragment`, in box
/// order.
///
/// Each track carries its `tfdt` in its own timescale, so video (often 90000)
/// and audio (often 48000) state different numbers for the same instant. A
/// window must capture one base per track and subtract that track's own base,
/// or the two tracks start at different times and drift out of sync. An empty
/// slice states that the fragment carries no `tfdt`, which is the case for an
/// MPEG-TS segment rather than a fragmented MP4 one.
pub(crate) fn first_bases(fragment: &[u8]) -> Vec<u64> {
    let mut found = Vec::new();
    bodies(fragment, 0, fragment.len(), 0, &mut found);
    found
        .into_iter()
        .filter_map(|body| read(fragment, body))
        .collect()
}

/// The base media decode time of the first track fragment in `fragment`.
///
/// A convenience wrapper for single-track tests; production callers use
/// [`first_bases`] so tracks with different timescales stay aligned.
#[cfg(test)]
pub(crate) fn first_base(fragment: &[u8]) -> Option<u64> {
    first_bases(fragment).into_iter().next()
}

/// Moves each `tfdt` in `fragment` back by its own `delta`, paired by index.
///
/// `deltas` is captured from the window's first fragment with [`first_bases`],
/// so the nth `tfdt` of every later fragment is moved by the nth track's base.
/// A fragment with more tracks than the first leaves the extras untouched; a
/// shorter list simply rebases fewer tracks. A value that would go below zero
/// stops at zero, so a fragment from before the window's first one still opens
/// rather than stating a time no decoder can represent.
pub(crate) fn rebase_each(fragment: &mut [u8], deltas: &[u64]) {
    if deltas.is_empty() {
        return;
    }
    let mut found = Vec::new();
    bodies(fragment, 0, fragment.len(), 0, &mut found);
    for (body, delta) in found.into_iter().zip(deltas.iter().copied()) {
        let Some(version) = fragment.get(body) else {
            continue;
        };
        if *version == 1 {
            let Some(cell) = fragment.get_mut(body + 4..body + 12) else {
                continue;
            };
            let mut base = [0u8; 8];
            base.copy_from_slice(cell);
            let moved = u64::from_be_bytes(base).saturating_sub(delta);
            cell.copy_from_slice(&moved.to_be_bytes());
        } else {
            let Some(cell) = fragment.get_mut(body + 4..body + 8) else {
                continue;
            };
            let mut base = [0u8; 4];
            base.copy_from_slice(cell);
            let moved = u32::try_from(delta)
                .map(|delta| u32::from_be_bytes(base).saturating_sub(delta))
                .unwrap_or(0);
            cell.copy_from_slice(&moved.to_be_bytes());
        }
    }
}

/// Moves every `tfdt` in `fragment` back by the same `delta`.
///
/// A convenience wrapper for single-track tests; a muxed window must use
/// [`rebase_each`] so each track keeps its own timescale.
#[cfg(test)]
pub(crate) fn rebase(fragment: &mut [u8], delta: u64) {
    let bases = first_bases(fragment)
        .into_iter()
        .map(|_| delta)
        .collect::<Vec<_>>();
    rebase_each(fragment, &bases);
}

/// The time a `tfdt` body states.
fn read(fragment: &[u8], body: usize) -> Option<u64> {
    match *fragment.get(body)? {
        1 => {
            let mut base = [0u8; 8];
            base.copy_from_slice(fragment.get(body + 4..body + 12)?);
            Some(u64::from_be_bytes(base))
        }
        _ => {
            let mut base = [0u8; 4];
            base.copy_from_slice(fragment.get(body + 4..body + 8)?);
            Some(u64::from(u32::from_be_bytes(base)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box with a 32-bit size.
    fn small(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    /// A `tfdt` box of the stated version.
    fn tfdt(version: u8, base: u64) -> Vec<u8> {
        let mut body = vec![version, 0, 0, 0];
        if version == 1 {
            body.extend_from_slice(&base.to_be_bytes());
        } else {
            body.extend_from_slice(&(base as u32).to_be_bytes());
        }
        small(b"tfdt", &body)
    }

    /// A fragment that wraps one `tfdt` the way a stream lays it out.
    fn fragment(version: u8, base: u64) -> Vec<u8> {
        let mut traf = Vec::new();
        traf.extend_from_slice(&tfdt(version, base));
        let traf = small(b"traf", &traf);
        let mut moof = Vec::new();
        moof.extend_from_slice(&small(b"mfhd", &[0, 0, 0, 1]));
        moof.extend_from_slice(&traf);
        let mut out = Vec::new();
        out.extend_from_slice(&small(b"styp", b"msdh"));
        out.extend_from_slice(&small(b"moof", &moof));
        out.extend_from_slice(&small(b"mdat", &[0; 4]));
        out
    }

    #[test]
    fn reads_a_long_base() {
        let fragment = fragment(1, 161_532_966_619_740);
        assert_eq!(first_base(&fragment), Some(161_532_966_619_740));
    }

    #[test]
    fn reads_a_short_base() {
        assert_eq!(first_base(&fragment(0, 5_400)), Some(5_400));
    }

    #[test]
    fn a_segment_without_a_tfdt_has_no_base() {
        let plain = small(b"styp", b"msdh");
        assert_eq!(first_base(&plain), None);
    }

    #[test]
    fn a_truncated_tfdt_body_has_no_base() {
        // A box whose size claims a 64-bit base but whose body stops short must
        // be ignored, not read past its end into the next box.
        let mut fragment = Vec::new();
        let body = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        fragment.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
        fragment.extend_from_slice(b"tfdt");
        fragment.extend_from_slice(&body);
        assert_eq!(first_base(&fragment), None);
    }

    #[test]
    fn moves_a_long_base_to_zero() {
        let mut fragment = fragment(1, 161_532_966_619_740);
        rebase(&mut fragment, 161_532_966_619_740);
        assert_eq!(first_base(&fragment), Some(0));
    }

    #[test]
    fn keeps_the_distance_between_fragments() {
        let mut later = fragment(1, 161_532_966_619_740 + 540_000);
        rebase(&mut later, 161_532_966_619_740);
        assert_eq!(first_base(&later), Some(540_000));
    }

    #[test]
    fn a_moved_past_zero_short_base_stops_at_zero() {
        let mut fragment = fragment(0, 100);
        rebase(&mut fragment, 1_000);
        assert_eq!(first_base(&fragment), Some(0));
    }

    #[test]
    fn a_moved_past_zero_long_base_stops_at_zero() {
        let mut fragment = fragment(1, 100);
        rebase(&mut fragment, u64::MAX);
        assert_eq!(first_base(&fragment), Some(0));
    }

    #[test]
    fn moves_every_track_by_its_own_base() {
        // Video and audio state their times in different timescales. A single
        // delta cannot line both up: each track needs its own base.
        let mut traf = Vec::new();
        traf.extend_from_slice(&tfdt(1, 900_000));
        let video = small(b"traf", &traf);
        let mut traf = Vec::new();
        traf.extend_from_slice(&tfdt(1, 480_000));
        let audio = small(b"traf", &traf);
        let mut moof = Vec::new();
        moof.extend_from_slice(&video);
        moof.extend_from_slice(&audio);
        let mut fragment = Vec::new();
        fragment.extend_from_slice(&small(b"moof", &moof));

        let bases = first_bases(&fragment);
        assert_eq!(bases, vec![900_000, 480_000]);
        rebase_each(&mut fragment, &bases);

        let mut found = Vec::new();
        bodies(&fragment, 0, fragment.len(), 0, &mut found);
        assert_eq!(found.len(), 2);
        assert_eq!(read(&fragment, found[0]), Some(0));
        assert_eq!(read(&fragment, found[1]), Some(0));
    }

    #[test]
    fn keeps_each_tracks_distance_between_fragments() {
        let bases = vec![900_000, 480_000];
        let mut later = Vec::new();
        let mut traf = Vec::new();
        traf.extend_from_slice(&tfdt(1, 900_000 + 540_000));
        later.extend_from_slice(&small(b"traf", &traf));
        let mut traf = Vec::new();
        traf.extend_from_slice(&tfdt(1, 480_000 + 288_000));
        later.extend_from_slice(&small(b"traf", &traf));
        let mut fragment = Vec::new();
        fragment.extend_from_slice(&small(b"moof", &later));

        rebase_each(&mut fragment, &bases);

        let mut found = Vec::new();
        bodies(&fragment, 0, fragment.len(), 0, &mut found);
        assert_eq!(found.len(), 2);
        assert_eq!(read(&fragment, found[0]), Some(540_000));
        assert_eq!(read(&fragment, found[1]), Some(288_000));
    }
}
