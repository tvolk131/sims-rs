//! MPEG-1 Layer III (MP3): the radio stations' music.
//!
//! The install's 141 MP3 files are all 44,100 Hz MPEG-1 Layer III
//! streams (checked by the census test). Decoding uses the `symphonia`
//! MP3 decoder, a pure-Rust implementation. Its output is floating point
//! and converted to 16-bit with symphonia's own conversion, so a digest
//! of decoded MP3 PCM is only pinned on the machine that took it. Music is
//! presentation: nothing in the simulation reads it.

use crate::{Error, Pcm, Result};
use symphonia_bundle_mp3::{MpaDecoder, MpaReader};
use symphonia_core::audio::SampleBuffer;
use symphonia_core::codecs::{Decoder, DecoderOptions};
use symphonia_core::errors::Error as SymphoniaError;
use symphonia_core::formats::{FormatOptions, FormatReader};
use symphonia_core::io::{MediaSourceStream, MediaSourceStreamOptions};

fn error(error: SymphoniaError) -> Error {
    Error::format("MP3", 0, error.to_string())
}

/// What a frame scan finds without decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info {
    pub rate: u32,
    pub channels: u16,
    /// MPEG frames in the stream.
    pub packets: u64,
    /// Decoded frames (one sample per channel) the stream holds, as the
    /// frame headers count them.
    pub frames: u64,
}

fn open(bytes: &[u8]) -> Result<MpaReader> {
    let source = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(bytes.to_vec())),
        MediaSourceStreamOptions::default(),
    );
    MpaReader::try_new(source, &FormatOptions::default()).map_err(error)
}

fn is_end(error: &SymphoniaError) -> bool {
    matches!(error, SymphoniaError::IoError(e) if e.kind() == std::io::ErrorKind::UnexpectedEof)
}

/// Walks the stream's frame headers without decoding.
pub fn scan(bytes: &[u8]) -> Result<Info> {
    let mut reader = open(bytes)?;
    let params = reader
        .default_track()
        .ok_or_else(|| Error::format("MP3", 0, "no audio track"))?
        .codec_params
        .clone();
    let mut info = Info {
        rate: params.sample_rate.unwrap_or(0),
        channels: params.channels.map_or(0, |c| c.count() as u16),
        packets: 0,
        frames: 0,
    };
    loop {
        match reader.next_packet() {
            Ok(packet) => {
                info.packets += 1;
                info.frames += packet.dur;
            }
            Err(e) if is_end(&e) => break,
            Err(e) => return Err(error(e)),
        }
    }
    Ok(info)
}

/// Decodes the whole stream into PCM.
pub fn decode(bytes: &[u8]) -> Result<Pcm> {
    decode_packets(bytes, u64::MAX)
}

/// Decodes at most the first `limit` MPEG frames.
pub fn decode_packets(bytes: &[u8], limit: u64) -> Result<Pcm> {
    let mut reader = open(bytes)?;
    let params = reader
        .default_track()
        .ok_or_else(|| Error::format("MP3", 0, "no audio track"))?
        .codec_params
        .clone();
    let mut decoder = MpaDecoder::try_new(&params, &DecoderOptions::default()).map_err(error)?;
    let mut pcm = Pcm {
        rate: params.sample_rate.unwrap_or(0),
        channels: params.channels.map_or(0, |c| c.count() as u16),
        samples: Vec::new(),
    };
    let mut packets = 0;
    while packets < limit {
        let packet = match reader.next_packet() {
            Ok(packet) => packet,
            Err(e) if is_end(&e) => break,
            Err(e) => return Err(error(e)),
        };
        packets += 1;
        let decoded = decoder.decode(&packet).map_err(error)?;
        let spec = *decoded.spec();
        let mut buffer = SampleBuffer::<i16>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        pcm.rate = spec.rate;
        pcm.channels = spec.channels.count() as u16;
        pcm.samples.extend_from_slice(buffer.samples());
    }
    Ok(pcm)
}
