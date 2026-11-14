//! RIFF WAVE files with PCM samples.
//!
//! Every WAV file in the install is format 1 (PCM), 16-bit, mono or
//! stereo, at 22,050 Hz (checked by the census test). 8-bit unsigned PCM
//! is read too, since it costs nothing; no other format is.
//!
//! Chunks are walked in order: `fmt ` must come before `data`, unknown
//! chunks are skipped, and each chunk is padded to an even length. A
//! `data` chunk longer than the file is cut to what the file holds.

use crate::{Error, Pcm, Result};

fn fail<T>(offset: usize, message: &str) -> Result<T> {
    Err(Error::format("WAV", offset, message))
}

/// The `fmt ` chunk's fields that decoding needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub tag: u16,
    pub channels: u16,
    pub rate: u32,
    pub block_align: u16,
    pub bits: u16,
}

/// Reads the format without decoding the samples.
pub fn format(bytes: &[u8]) -> Result<Format> {
    walk(bytes).map(|(format, _)| format)
}

/// Decodes a WAV file into PCM.
pub fn decode(bytes: &[u8]) -> Result<Pcm> {
    let (format, data) = walk(bytes)?;
    let samples = match format.bits {
        16 => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect(),
        8 => data.iter().map(|&b| (i16::from(b) - 128) << 8).collect(),
        _ => unreachable!("checked in walk"),
    };
    let mut pcm = Pcm {
        rate: format.rate,
        channels: format.channels,
        samples,
    };
    // A data chunk that ends mid-frame drops the partial frame.
    let whole = pcm.frames() * usize::from(format.channels);
    pcm.samples.truncate(whole);
    Ok(pcm)
}

fn walk(bytes: &[u8]) -> Result<(Format, &[u8])> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return fail(0, "not a RIFF WAVE file");
    }
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("4 bytes")) as usize;
        let body = at + 8;
        let end = body.saturating_add(size).min(bytes.len());
        let chunk = &bytes[body..end];
        match id {
            b"fmt " => {
                if chunk.len() < 16 {
                    return fail(body, "fmt chunk cut short");
                }
                let u16_at = |i: usize| u16::from_le_bytes([chunk[i], chunk[i + 1]]);
                let parsed = Format {
                    tag: u16_at(0),
                    channels: u16_at(2),
                    rate: u32::from_le_bytes(chunk[4..8].try_into().expect("4 bytes")),
                    block_align: u16_at(12),
                    bits: u16_at(14),
                };
                if parsed.tag != 1 {
                    return fail(body, "only PCM (format 1) is supported");
                }
                if !matches!(parsed.channels, 1 | 2) {
                    return fail(body + 2, "only mono and stereo are supported");
                }
                if !matches!(parsed.bits, 8 | 16) {
                    return fail(body + 14, "only 8- and 16-bit samples are supported");
                }
                if parsed.rate == 0 {
                    return fail(body + 4, "sample rate is 0");
                }
                format = Some(parsed);
            }
            b"data" => {
                let Some(format) = format else {
                    return fail(at, "data chunk before fmt chunk");
                };
                return Ok((format, chunk));
            }
            _ => {}
        }
        at = body.saturating_add(size).saturating_add(size & 1);
    }
    fail(bytes.len(), "no data chunk")
}
