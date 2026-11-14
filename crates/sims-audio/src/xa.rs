//! Maxis XA: 4-bit ADPCM (adaptive differential PCM) sound files.
//!
//! Nearly every sound effect and voice in the install is an XA file. The
//! layout follows public notes (FFmpeg's "Maxis XA" demuxer and its
//! `adpcm_ea_maxis_xa` decoder, and the Niotso wiki's XA page). It was not
//! traced in the original executable: **unverified** against the game's
//! own decoder.
//!
//! A file is a 24-byte header and then frames:
//!
//! | bytes | field |
//! | --- | --- |
//! | 0..2 | `XA` |
//! | 2..4 | a tag: 0, `J\0` or `0x12` in this install; not read |
//! | 4..8 | the decoded size in bytes |
//! | 8..24 | a `WAVEFORMATEX` without its size: format 1, channels, rate, byte rate, block align, 16 bits |
//!
//! A frame holds 28 samples per channel in `15 * channels` bytes: one
//! header byte per channel (high nibble: a predictor from [`FILTERS`]; low
//! nibble: a shift), then 14 groups of one byte per channel. Each byte
//! holds two samples, the high nibble first. Stereo samples alternate
//! left, right. A sample is the nibble, sign-extended and shifted left by
//! `20 - shift`, plus the two previous outputs weighted by the predictor,
//! rounded and divided by 256, then clamped to 16 bits.
//!
//! The last frame may be cut short: of 18,155 files in the install, one
//! stereo file ends half way through a frame, and its decoded size needs
//! the samples that half holds. So a partial frame decodes as many whole
//! groups as it has. The output is cut to the decoded size.

use crate::{Error, Pcm, Result};

/// The header's length in bytes.
pub const HEADER: usize = 24;

/// Predictor weights, in 1/256ths, for the current and previous output.
pub const FILTERS: [(i32, i32); 4] = [(0, 0), (240, 0), (460, -208), (392, -220)];

/// An XA file's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Bytes 2 and 3, which the decoder does not read.
    pub tag: [u8; 2],
    /// The decoded size in bytes.
    pub decoded_size: u32,
    pub format: u16,
    pub channels: u16,
    pub rate: u32,
    pub byte_rate: u32,
    pub block_align: u16,
    pub bits: u16,
}

impl Header {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let fail = |offset, message: &str| Err(Error::format("XA", offset, message));
        if bytes.len() < HEADER {
            return fail(bytes.len(), "header cut short");
        }
        if &bytes[..2] != b"XA" {
            return fail(0, "not an XA file");
        }
        let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().expect("4 bytes"));
        let header = Self {
            tag: [bytes[2], bytes[3]],
            decoded_size: u32_at(4),
            format: u16_at(8),
            channels: u16_at(10),
            rate: u32_at(12),
            byte_rate: u32_at(16),
            block_align: u16_at(20),
            bits: u16_at(22),
        };
        if header.format != 1 {
            return fail(8, "format is not PCM");
        }
        if !matches!(header.channels, 1 | 2) {
            return fail(10, "only mono and stereo are supported");
        }
        if header.bits != 16 {
            return fail(22, "output is not 16-bit");
        }
        if header.rate == 0 {
            return fail(12, "sample rate is 0");
        }
        if !header
            .decoded_size
            .is_multiple_of(2 * u32::from(header.channels))
        {
            return fail(4, "decoded size is not a whole number of frames");
        }
        Ok(header)
    }
}

/// How many samples a payload of `payload` bytes decodes to: 28 per
/// channel for each whole frame of `15 * channels` bytes, and two per
/// channel for each byte group of a partial last frame after its header.
fn most_samples(payload: usize, channels: usize) -> usize {
    let frame = 15 * channels;
    let whole = (payload / frame).saturating_mul(28 * channels);
    let rest = payload % frame;
    let partial = if rest >= channels {
        ((rest - channels) / channels).min(14) * 2 * channels
    } else {
        0
    };
    whole.saturating_add(partial)
}

/// Decodes an XA file into PCM.
///
/// The loop is written with plain indexing and no helper calls: the
/// gated census decodes about a billion samples, and unoptimized test
/// builds do not inline.
pub fn decode(bytes: &[u8]) -> Result<Pcm> {
    let header = Header::parse(bytes)?;
    let channels = usize::from(header.channels);
    let wanted = header.decoded_size as usize / 2;
    // Check the claimed size against what the payload can hold before
    // reserving memory for it: a header may claim up to 4 GiB.
    let available = most_samples(bytes.len() - HEADER, channels);
    if wanted > available {
        return Err(Error::format(
            "XA",
            bytes.len(),
            format!("data holds at most {available} of {wanted} samples"),
        ));
    }
    let mut samples: Vec<i16> = Vec::with_capacity(wanted.saturating_add(56));
    // Per channel: the last output, the one before, the two weights and
    // the shift.
    let mut current = [0i32; 2];
    let mut previous = [0i32; 2];
    let mut weight0 = [0i32; 2];
    let mut weight1 = [0i32; 2];
    let mut shift = [0u32; 2];
    let mut at = HEADER;
    let len = bytes.len();
    while samples.len() < wanted && at + channels <= len {
        for c in 0..channels {
            let byte = bytes[at + c];
            let filter = usize::from(byte >> 4);
            if filter > 3 {
                return Err(Error::format("XA", at + c, "predictor index above 3"));
            }
            weight0[c] = FILTERS[filter].0;
            weight1[c] = FILTERS[filter].1;
            shift[c] = 20 - u32::from(byte & 0x0f);
        }
        at += channels;
        let groups = ((len - at) / channels).min(14);
        for _ in 0..groups {
            for half in 0..2 {
                for c in 0..channels {
                    let byte = bytes[at + c];
                    let nibble = if half == 0 { byte >> 4 } else { byte & 0x0f };
                    // Sign-extend the nibble, then scale it.
                    let delta = ((i32::from(nibble) << 28) >> 28) << shift[c];
                    let value =
                        ((delta + current[c] * weight0[c] + previous[c] * weight1[c] + 0x80) >> 8)
                            .clamp(-32768, 32767);
                    previous[c] = current[c];
                    current[c] = value;
                    samples.push(value as i16);
                }
            }
            at += channels;
        }
    }
    if samples.len() < wanted {
        return Err(Error::format(
            "XA",
            bytes.len(),
            format!("data ends after {} of {} samples", samples.len(), wanted),
        ));
    }
    samples.truncate(wanted);
    Ok(Pcm {
        rate: header.rate,
        channels: header.channels,
        samples,
    })
}
