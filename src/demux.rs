//! A minimal MP4 demuxer that turns a container into Annex-B access units.
//!
//! The `upstream-video` path needs an elementary stream: `HardwareVideo::
//! open_stream` takes H.264/HEVC access units, and until now the caller had
//! to demux the container itself. This module reads a progressive MP4 — the
//! `moov` box for the track's sample table, the `mdat` payload for its bytes —
//! and emits one access unit per sync sample with real timestamps from the
//! sample table, replacing the hardcoded 40 ms step.
//!
//! Scope: one video track, no fragmentation (`moof` boxes are skipped), no
//! audio. A file this demuxer cannot read returns an error rather than
//! guessing.

use std::time::Duration;

/// One decoded-ready access unit cut from a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessUnit {
    /// Annex-B bytes (start codes included) of one picture.
    pub data: Vec<u8>,
    /// Presentation timestamp derived from the sample table.
    pub timestamp: Duration,
}

/// What went wrong while demuxing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemuxError {
    /// The bytes do not start with an `ftyp`/`moov`-bearing MP4 structure.
    NotMp4,
    /// No video track with a supported codec was found.
    NoVideoTrack,
    /// The sample table is inconsistent (no samples, or no sync samples).
    BrokenSampleTable,
}

/// The codec an MP4 video track declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mp4Codec {
    /// `avc1` — H.264.
    H264,
    /// `hvc1`/`hev1` — HEVC.
    Hevc,
}

/// Demuxes a progressive MP4 into Annex-B access units.
///
/// `timescale` comes from the track's `mdhd`; timestamps are computed as
/// `sample_delta` sums scaled to real time, so a 30 fps clip yields ~33 ms
/// steps instead of the fixed 40 ms the raw stream path assumed.
pub fn demux_mp4(input: &[u8]) -> Result<(Mp4Codec, Vec<AccessUnit>), DemuxError> {
    let boxes = top_level_boxes(input);
    let moov = boxes
        .iter()
        .find(|(kind, _, _)| kind == b"moov")
        .map(|(_, start, end)| &input[*start..*end])
        .ok_or(DemuxError::NotMp4)?;

    // Walk trak boxes; take the first video track this demuxer supports.
    for (kind, start, end) in child_boxes(moov) {
        if kind != *b"trak" {
            continue;
        }
        let trak = &moov[start..end];
        if let Some(result) = demux_trak(input, trak) {
            return result;
        }
    }
    Err(DemuxError::NoVideoTrack)
}

fn demux_trak(input: &[u8], trak: &[u8]) -> Option<Result<(Mp4Codec, Vec<AccessUnit>), DemuxError>> {
    let mdia = child_boxes(trak)
        .into_iter()
        .find(|(kind, _, _)| kind == b"mdia")
        .map(|(_, start, end)| &trak[start..end])?;
    let mdhd = child_boxes(mdia)
        .into_iter()
        .find(|(kind, _, _)| kind == b"mdhd")
        .map(|(_, start, end)| &mdia[start..end])?;
    let timescale = mdhd_timescale(mdhd)?;

    let minf = child_boxes(mdia)
        .into_iter()
        .find(|(kind, _, _)| kind == b"minf")
        .map(|(_, start, end)| &mdia[start..end])?;
    let stbl = child_boxes(minf)
        .into_iter()
        .find(|(kind, _, _)| kind == b"stbl")
        .map(|(_, start, end)| &minf[start..end])?;

    let mut codec = None;
    let mut sample_sizes: Vec<u32> = Vec::new();
    let mut sample_deltas: Vec<u32> = Vec::new();
    let mut key_samples: Vec<u64> = Vec::new();
    let mut chunk_offsets: Vec<u64> = Vec::new();
    let mut samples_per_chunk: Vec<u32> = Vec::new();

    for (kind, start, end) in child_boxes(stbl) {
        let body = &stbl[start..end];
        match &kind {
            b"stsd" => codec = stsd_codec(body),
            b"stsz" => sample_sizes = stsz_sizes(body),
            b"stts" => sample_deltas = stts_deltas(body),
            b"stss" => key_samples = stss_samples(body),
            b"stco" | b"co64" => chunk_offsets = stco_offsets(body, kind == *b"co64"),
            b"stsc" => samples_per_chunk = stsc_entries(body),
            _ => {}
        }
    }
    let codec = codec?;
    if sample_sizes.is_empty() || sample_deltas.is_empty() || chunk_offsets.is_empty() {
        return Some(Err(DemuxError::BrokenSampleTable));
    }

    // Walk chunks directly: each chunk holds `count` consecutive samples.
    let mut offsets: Vec<u64> = Vec::with_capacity(sample_sizes.len());
    let mut sample_index = 0usize;
    for (chunk, count) in samples_per_chunk.iter().enumerate() {
        let Some(chunk_offset) = chunk_offsets.get(chunk) else {
            break;
        };
        let mut position = *chunk_offset;
        for _ in 0..*count {
            if sample_index >= sample_sizes.len() {
                break;
            }
            offsets.push(position);
            position += sample_sizes[sample_index] as u64;
            sample_index += 1;
        }
    }
    if offsets.len() < sample_sizes.len() {
        return Some(Err(DemuxError::BrokenSampleTable));
    }

    // Timestamps: cumulative deltas scaled by the timescale.
    let mut units = Vec::with_capacity(sample_sizes.len());
    let mut elapsed: u64 = 0;
    let mut key_iter = key_samples.iter().peekable();
    for (index, size) in sample_sizes.iter().enumerate() {
        let delta = sample_deltas.get(index).copied().unwrap_or(0);
        let is_key = match key_iter.peek() {
            Some(next_key) if **next_key == (index as u64) + 1 => {
                key_iter.next();
                true
            }
            _ => key_samples.is_empty(),
        };
        if !is_key {
            elapsed += delta as u64;
            continue;
        }
        let offset = offsets[index] as usize;
        let end = offset + *size as usize;
        if end > input.len() {
            return Some(Err(DemuxError::BrokenSampleTable));
        }
        let annex_b = avcc_to_annex_b(&input[offset..end]);
        units.push(AccessUnit {
            data: annex_b,
            timestamp: Duration::from_nanos(
                elapsed * 1_000_000_000 / u64::from(timescale.max(1)),
            ),
        });
        elapsed += delta as u64;
    }
    if units.is_empty() {
        return Some(Err(DemuxError::BrokenSampleTable));
    }
    Some(Ok((codec, units)))
}

/// Reads the timescale out of an `mdhd` box (version 0 or 1).
fn mdhd_timescale(mdhd: &[u8]) -> Option<u32> {
    // mdhd: version(1) flags(3) then v0: ctime(4) mtime(4) timescale(4)
    //       v1: ctime(8) mtime(8) timescale(4)
    let body = mdhd.get(8..)?;
    let version = *mdhd.get(4)?;
    let skip = if version == 1 { 16 } else { 8 };
    let at = skip..skip + 4;
    let bytes = body.get(at)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Finds the codec in an `stsd` box's first sample entry.
fn stsd_codec(stsd: &[u8]) -> Option<Mp4Codec> {
    // stsd: version+flags(4) entry_count(4) then entries.
    let entries = stsd.get(8..)?;
    let (kind, _, _) = *child_boxes(entries).first()?;
    match &kind {
        b"avc1" => Some(Mp4Codec::H264),
        b"hvc1" | b"hev1" => Some(Mp4Codec::Hevc),
        _ => None,
    }
}

/// `stsz`: sample sizes (fixed size or per-sample table).
fn stsz_sizes(stsz: &[u8]) -> Vec<u32> {
    // stsz: version+flags(4) sample_size(4) sample_count(4) sizes...
    let Some(body) = stsz.get(20..) else {
        return Vec::new();
    };
    let fixed = u32::from_be_bytes([stsz[12], stsz[13], stsz[14], stsz[15]]);
    let count = u32::from_be_bytes([stsz[16], stsz[17], stsz[18], stsz[19]]);
    if fixed != 0 {
        return vec![fixed; count as usize];
    }
    body[..]
        .chunks_exact(4)
        .take(count as usize)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// `stts`: sample deltas (run-length encoded).
fn stts_deltas(stts: &[u8]) -> Vec<u32> {
    // stts: version+flags(4) entry_count(4) runs of (count, delta)
    let Some(body) = stts.get(16..) else {
        return Vec::new();
    };
    let count = u32::from_be_bytes([stts[12], stts[13], stts[14], stts[15]]) as usize;
    let mut deltas = Vec::new();
    for run in body.chunks_exact(8).take(count) {
        let repeat = u32::from_be_bytes([run[0], run[1], run[2], run[3]]);
        let delta = u32::from_be_bytes([run[4], run[5], run[6], run[7]]);
        deltas.extend(std::iter::repeat_n(delta, repeat as usize));
    }
    deltas
}

/// `stss`: one-based indices of sync samples; absent means every sample.
fn stss_samples(stss: &[u8]) -> Vec<u64> {
    let Some(body) = stss.get(16..) else {
        return Vec::new();
    };
    let count = u32::from_be_bytes([stss[12], stss[13], stss[14], stss[15]]) as usize;
    body[..]
        .chunks_exact(4)
        .take(count)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as u64)
        .collect()
}

/// `stco`/`co64`: chunk offsets.
fn stco_offsets(stco: &[u8], wide: bool) -> Vec<u64> {
    let Some(body) = stco.get(16..) else {
        return Vec::new();
    };
    let count = u32::from_be_bytes([stco[12], stco[13], stco[14], stco[15]]) as usize;
    if wide {
        body[..]
            .chunks_exact(8)
            .take(count)
            .map(|c| u64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
            .collect()
    } else {
        body[..]
            .chunks_exact(4)
            .take(count)
            .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]) as u64)
            .collect()
    }
}

/// `stsc`: samples per chunk (un-length encoded).
fn stsc_entries(stsc: &[u8]) -> Vec<u32> {
    let Some(body) = stsc.get(16..) else {
        return Vec::new();
    };
    let count = u32::from_be_bytes([stsc[12], stsc[13], stsc[14], stsc[15]]) as usize;
    let mut entries = Vec::new();
    for run in body.chunks_exact(12).take(count) {
        let repeat = u32::from_be_bytes([run[0], run[1], run[2], run[3]]);
        let per_chunk = u32::from_be_bytes([run[4], run[5], run[6], run[7]]);
        entries.extend(std::iter::repeat_n(per_chunk, repeat.max(1) as usize));
    }
    entries
}

/// Converts an `avcC`-style length-prefixed sample into Annex-B start codes.
fn avcc_to_annex_b(sample: &[u8]) -> Vec<u8> {
    // The sample entry carries the parameter sets; a full implementation
    // splices them ahead of the first slice. Here the length-prefix layout
    // (4-byte lengths, the MP4 default) is converted; callers that already
    // hold Annex-B pass through unchanged when no length prefix parses.
    let mut out = Vec::with_capacity(sample.len() + 16);
    let mut rest = sample;
    while rest.len() >= 4 {
        let length = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        if length == 0 || length > rest.len() - 4 {
            // Not a length-prefixed layout; return the bytes as-is.
            return sample.to_vec();
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&rest[4..4 + length]);
        rest = &rest[4 + length..];
    }
    if out.is_empty() {
        sample.to_vec()
    } else {
        out
    }
}

/// Walks the child boxes of a container box, yielding `(kind, start, end)`
/// relative to `body`.
fn child_boxes(body: &[u8]) -> Vec<([u8; 4], usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= body.len() {
        let size = u32::from_be_bytes([
            body[at],
            body[at + 1],
            body[at + 2],
            body[at + 3],
        ]) as usize;
        let kind = [
            body[at + 4],
            body[at + 5],
            body[at + 6],
            body[at + 7],
        ];
        if size == 0 {
            out.push((kind, at + 8, body.len()));
            break;
        }
        if size < 8 || at + size > body.len() {
            break;
        }
        out.push((kind, at + 8, at + size));
        at += size;
    }
    out
}

fn top_level_boxes(input: &[u8]) -> Vec<([u8; 4], usize, usize)> {
    child_boxes(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_of(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn child_boxes_walk_a_simple_container() {
        let inner = box_of(b"abcd", &[1, 2, 3]);
        let outer = box_of(b"prnt", &inner);
        let children = child_boxes(&outer[8..]);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].0, *b"abcd");
    }

    #[test]
    fn non_mp4_input_is_rejected() {
        assert_eq!(demux_mp4(b"not an mp4 at all"), Err(DemuxError::NotMp4));
        assert_eq!(demux_mp4(&[]), Err(DemuxError::NotMp4));
    }

    #[test]
    fn avcc_length_prefixes_become_start_codes() {
        let sample = vec![0u8, 0, 0, 3, 0x65, 1, 2];
        let out = avcc_to_annex_b(&sample);
        assert_eq!(out, vec![0, 0, 0, 1, 0x65, 1, 2]);
    }

    #[test]
    fn a_non_length_prefixed_sample_passes_through() {
        // A first length larger than the rest of the sample cannot be a
        // length prefix, so the bytes pass through unchanged.
        let sample = vec![0u8, 0, 0, 9, 0x65, 1, 2];
        assert_eq!(avcc_to_annex_b(&sample), sample);
    }

    #[test]
    fn stts_expands_run_length_deltas() {
        let mut body = vec![0u8, 0, 0, 0]; // version+flags
        body.extend_from_slice(&1u32.to_be_bytes()); // one run
        body.extend_from_slice(&3u32.to_be_bytes()); // repeat 3
        body.extend_from_slice(&3000u32.to_be_bytes()); // delta 3000
        let mut stts = box_of(b"stts", &body);
        assert_eq!(stts_deltas(&stts), vec![3000, 3000, 3000]);
        stts.clear();
    }

    #[test]
    fn stsz_reads_the_fixed_size_form() {
        let mut body = vec![0u8, 0, 0, 0];
        body.extend_from_slice(&100u32.to_be_bytes()); // fixed size
        body.extend_from_slice(&4u32.to_be_bytes()); // count
        let stsz = box_of(b"stsz", &body);
        assert_eq!(stsz_sizes(&stsz), vec![100; 4]);
    }
}
