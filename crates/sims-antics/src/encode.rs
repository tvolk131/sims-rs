//! Checked binary encoders for Sims 1 IFF chunks, IFF 2.5 containers (reusing
//! `sims_formats::iff_write` for `rsmp` framing), and `FAR!byAZ` archives.

use crate::error::{Error, Result};
use crate::ir::{
    BconChunk, BhavChunk, DgrpChunk, FarEntry, FwavChunk, IFF_CONVERTED_FLAG, ObjdChunk, ObjfChunk,
    PaltChunk, SlotChunk, Spr2Chunk, Spr2Frame, Spr2Pixel, StringFormat, StringTableChunk,
    TtabChunk,
};
use sims_formats::iff::ChunkData;
use sims_formats::iff_write::{self, ChunkWrite};

/// Encodes a `BHAV` chunk (formats `0x8000`, `0x8001`, `0x8002`, `0x8003`).
pub fn encode_bhav(chunk: &BhavChunk) -> Result<Vec<u8>> {
    if !matches!(chunk.format, 0x8000..=0x8003) {
        return Err(Error::new(format!(
            "unsupported BHAV format {:#06x}",
            chunk.format
        )));
    }
    let count = u16::try_from(chunk.instructions.len())
        .map_err(|_| Error::new("BHAV instruction count exceeds u16::MAX"))?;
    let mut out = Vec::with_capacity(12 + chunk.instructions.len() * 12 + chunk.trailing.len());
    out.extend_from_slice(&chunk.format.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.push(chunk.kind);
    out.push(chunk.args);
    out.extend_from_slice(&chunk.locals.to_le_bytes());
    out.extend_from_slice(&chunk.revision.to_le_bytes());
    for ins in &chunk.instructions {
        out.extend_from_slice(&ins.opcode.to_le_bytes());
        out.push(ins.on_true);
        out.push(ins.on_false);
        out.extend_from_slice(&ins.operands);
    }
    out.extend_from_slice(&chunk.trailing);
    Ok(out)
}

/// Encodes an `OBJD` chunk.
pub fn encode_objd(chunk: &ObjdChunk) -> Result<Vec<u8>> {
    if !matches!(chunk.version, 136..=142) {
        return Err(Error::new(format!(
            "unsupported OBJD version {}",
            chunk.version
        )));
    }
    if chunk.words.len() < 2 {
        return Err(Error::new("OBJD must contain at least the 4-byte version"));
    }
    let mut out =
        Vec::with_capacity(chunk.words.len() * 2 + usize::from(chunk.trailing_byte.is_some()));
    for (i, &word) in chunk.words.iter().enumerate() {
        let value = match i {
            0 => (chunk.version & 0xFFFF) as u16,
            1 => (chunk.version >> 16) as u16,
            _ => word,
        };
        out.extend_from_slice(&value.to_le_bytes());
    }
    if let Some(byte) = chunk.trailing_byte {
        out.push(byte);
    }
    Ok(out)
}

/// Encodes an `OBJf` object function table chunk.
pub fn encode_objf(chunk: &ObjfChunk) -> Result<Vec<u8>> {
    if chunk.version != 0 {
        return Err(Error::new(format!(
            "unsupported OBJf version {}",
            chunk.version
        )));
    }
    let count = u32::try_from(chunk.entries.len())
        .map_err(|_| Error::new("OBJf entry count exceeds u32::MAX"))?;
    let mut out = Vec::with_capacity(16 + chunk.entries.len() * 4 + chunk.trailing.len());
    out.extend_from_slice(&chunk.reserved.to_le_bytes());
    out.extend_from_slice(&chunk.version.to_le_bytes());
    out.extend_from_slice(b"fJBO");
    out.extend_from_slice(&count.to_le_bytes());
    for entry in &chunk.entries {
        out.extend_from_slice(&entry.condition.to_le_bytes());
        out.extend_from_slice(&entry.action.to_le_bytes());
    }
    out.extend_from_slice(&chunk.trailing);
    Ok(out)
}

/// Encodes a `BCON` tuning constant table chunk.
pub fn encode_bcon(chunk: &BconChunk) -> Result<Vec<u8>> {
    let count = u8::try_from(chunk.values.len())
        .map_err(|_| Error::new("BCON constant count exceeds 255"))?;
    let mut out = Vec::with_capacity(2 + chunk.values.len() * 2 + chunk.trailing.len());
    out.push(count);
    out.push(chunk.flags);
    for &value in &chunk.values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&chunk.trailing);
    Ok(out)
}

fn encode_string_bytes(text: &str, language: u8) -> Result<Vec<u8>> {
    use encoding_rs::*;
    let enc = match language {
        12 => WINDOWS_1255,
        13 => WINDOWS_1251,
        15 => SHIFT_JIS,
        16 | 21 => WINDOWS_1250,
        17 => GBK,
        18 => BIG5,
        19 => WINDOWS_874,
        20 => EUC_KR,
        _ => WINDOWS_1252,
    };
    let (bytes, _, unmappable) = enc.encode(text);
    if unmappable {
        return Err(Error::new(format!(
            "string contains characters unmappable in language {language} codepage"
        )));
    }
    if bytes.contains(&0) {
        return Err(Error::new("string table entry contains embedded NUL"));
    }
    Ok(bytes.into_owned())
}

/// Encodes a `STR#`, `CTSS`, or `TTAs` string table chunk across all formats
/// (`0x0000`, `0xFFFD`, `0xFFFE`, `0xFFFF`, and classic `MacResource`).
pub fn encode_string_table(chunk: &StringTableChunk) -> Result<Vec<u8>> {
    let count = u16::try_from(chunk.entries.len())
        .map_err(|_| Error::new("string table count exceeds u16::MAX"))?;
    let mut out = Vec::new();
    match chunk.format {
        StringFormat::Pascal => {
            out.extend_from_slice(&0x0000u16.to_le_bytes());
            if chunk.version_only_empty && chunk.entries.is_empty() {
                out.extend_from_slice(&chunk.trailing);
                return Ok(out);
            }
            out.extend_from_slice(&count.to_le_bytes());
        }
        StringFormat::Terminated => {
            out.extend_from_slice(&0xFFFFu16.to_le_bytes());
            if chunk.version_only_empty && chunk.entries.is_empty() {
                out.extend_from_slice(&chunk.trailing);
                return Ok(out);
            }
            out.extend_from_slice(&count.to_le_bytes());
        }
        StringFormat::Commented => {
            out.extend_from_slice(&0xFFFEu16.to_le_bytes());
            if chunk.version_only_empty && chunk.entries.is_empty() {
                out.extend_from_slice(&chunk.trailing);
                return Ok(out);
            }
            out.extend_from_slice(&count.to_le_bytes());
        }
        StringFormat::Localized => {
            out.extend_from_slice(&0xFFFDu16.to_le_bytes());
            if chunk.version_only_empty && chunk.entries.is_empty() {
                out.extend_from_slice(&chunk.trailing);
                return Ok(out);
            }
            out.extend_from_slice(&count.to_le_bytes());
        }
        StringFormat::MacResource => {
            let word_le = count.swap_bytes();
            if matches!(word_le as i16, -3..=0) {
                return Err(Error::new(format!(
                    "MacResource count {count} collides with versioned STR# header word"
                )));
            }
            out.extend_from_slice(&count.to_be_bytes());
        }
    }

    for entry in &chunk.entries {
        let lang_for_encode = if chunk.format == StringFormat::Localized {
            entry.language.max(1)
        } else {
            1
        };
        let raw_text = match &entry.raw_text {
            Some(raw) => raw.clone(),
            None => encode_string_bytes(&entry.text, lang_for_encode)?,
        };
        let raw_comment = match &entry.raw_comment {
            Some(raw) => raw.clone(),
            None => encode_string_bytes(&entry.comment, lang_for_encode)?,
        };
        match chunk.format {
            StringFormat::Pascal | StringFormat::MacResource => {
                let len = u8::try_from(raw_text.len())
                    .map_err(|_| Error::new("Pascal string entry exceeds 255 bytes"))?;
                out.push(len);
                out.extend_from_slice(&raw_text);
            }
            StringFormat::Terminated => {
                if raw_text.contains(&0) {
                    return Err(Error::new("terminated string contains embedded NUL"));
                }
                out.extend_from_slice(&raw_text);
                out.push(0);
            }
            StringFormat::Commented => {
                if raw_text.contains(&0) || raw_comment.contains(&0) {
                    return Err(Error::new("commented string contains embedded NUL"));
                }
                out.extend_from_slice(&raw_text);
                out.push(0);
                out.extend_from_slice(&raw_comment);
                out.push(0);
            }
            StringFormat::Localized => {
                if raw_text.contains(&0) || raw_comment.contains(&0) {
                    return Err(Error::new("localized string contains embedded NUL"));
                }
                out.push(entry.language);
                out.extend_from_slice(&raw_text);
                out.push(0);
                out.extend_from_slice(&raw_comment);
                out.push(0);
            }
        }
    }
    out.extend_from_slice(&chunk.trailing);
    Ok(out)
}

/// MSB-first bit writer matching `sims_formats::behavior::Fields` for compressed `TTAB` chunks.
struct BitWriter {
    bytes: Vec<u8>,
    current: u8,
    bits_used: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            current: 0,
            bits_used: 0,
        }
    }

    fn push_bits(&mut self, value: u32, count: usize) {
        for shift in (0..count).rev() {
            let bit = ((value >> shift) & 1) as u8;
            self.current = (self.current << 1) | bit;
            self.bits_used += 1;
            if self.bits_used == 8 {
                self.bytes.push(self.current);
                self.current = 0;
                self.bits_used = 0;
            }
        }
    }

    fn write_field(&mut self, value: u32, size: usize, width_hint: Option<u8>) {
        let widths = if size == 16 {
            [5usize, 8, 13, 16]
        } else {
            [6usize, 11, 21, 32]
        };
        if let Some(hint) = width_hint {
            if hint == 255 && value == 0 {
                self.push_bits(0, 1);
                return;
            }
            if let Some(&width) = widths.get(usize::from(hint)) {
                self.push_bits(1, 1);
                self.push_bits(u32::from(hint), 2);
                let mask = if width == 32 {
                    u32::MAX
                } else {
                    (1u32 << width) - 1
                };
                self.push_bits(value & mask, width);
                return;
            }
        }
        if value == 0 {
            self.push_bits(0, 1);
            return;
        }
        let signed: i64 = if size == 16 {
            i64::from(value as u16 as i16)
        } else {
            i64::from(value as i32)
        };
        let mut chosen_code = 3u32;
        for (code, &width) in widths.iter().enumerate() {
            let min = -(1i64 << (width - 1));
            let max = (1i64 << (width - 1)) - 1;
            if signed >= min && signed <= max {
                chosen_code = code as u32;
                break;
            }
        }
        let width = widths[chosen_code as usize];
        self.push_bits(1, 1);
        self.push_bits(chosen_code, 2);
        let mask = if width == 32 {
            u32::MAX
        } else {
            (1u32 << width) - 1
        };
        self.push_bits(value & mask, width);
    }

    fn finish(mut self, trailing_bits: u8) -> Vec<u8> {
        if self.bits_used > 0 {
            let remaining = 8 - self.bits_used;
            let mask = (1u8 << remaining) - 1;
            self.bytes
                .push((self.current << remaining) | (trailing_bits & mask));
        }
        self.bytes
    }
}

/// Encodes a `TTAB` pie-menu interaction table chunk (versions `4..=10`, both
/// uncompressed and bit-packed compressed).
pub fn encode_ttab(chunk: &TtabChunk) -> Result<Vec<u8>> {
    let count = u16::try_from(chunk.interactions.len())
        .map_err(|_| Error::new("TTAB interaction count exceeds u16::MAX"))?;
    let mut out = Vec::new();
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&chunk.version.to_le_bytes());
    if count == 0 {
        out.extend_from_slice(&chunk.trailing);
        return Ok(out);
    }
    if !(4..=10).contains(&chunk.version) {
        return Err(Error::new(format!(
            "unsupported TTAB version {}",
            chunk.version
        )));
    }
    let compressed = chunk.version >= 9 && chunk.compressed;
    if chunk.version >= 9 {
        out.push(if compressed {
            1
        } else if chunk.compression_byte != 1 {
            chunk.compression_byte
        } else {
            0
        });
    }

    if !compressed {
        for inter in &chunk.interactions {
            if inter.motives.len() > 256 {
                return Err(Error::new("TTAB motive advertisement count exceeds 256"));
            }
            out.extend_from_slice(&inter.action.to_le_bytes());
            out.extend_from_slice(&inter.test.to_le_bytes());
            out.extend_from_slice(&(inter.motives.len() as u32).to_le_bytes());
            out.extend_from_slice(&inter.flags.to_le_bytes());
            out.extend_from_slice(&inter.string_index.to_le_bytes());
            if chunk.version > 6 {
                out.extend_from_slice(&inter.attenuation_code.to_le_bytes());
            }
            out.extend_from_slice(&inter.attenuation.to_bits().to_le_bytes());
            out.extend_from_slice(&inter.autonomy.to_le_bytes());
            out.extend_from_slice(&(inter.joining_index as u32).to_le_bytes());
            for motive in &inter.motives {
                if chunk.version > 6 {
                    out.extend_from_slice(&(motive.minimum as u16).to_le_bytes());
                }
                out.extend_from_slice(&(motive.delta as u16).to_le_bytes());
                if chunk.version > 6 {
                    out.extend_from_slice(&motive.personality.to_le_bytes());
                }
            }
            if chunk.version > 9 {
                out.extend_from_slice(&inter.flags2.to_le_bytes());
            }
        }
    } else {
        let mut bw = BitWriter::new();
        let mut hints = chunk.compressed_width_hints.iter().copied();
        for inter in &chunk.interactions {
            if inter.motives.len() > 256 {
                return Err(Error::new("TTAB motive advertisement count exceeds 256"));
            }
            bw.write_field(u32::from(inter.action), 16, hints.next());
            bw.write_field(u32::from(inter.test), 16, hints.next());
            bw.write_field(inter.motives.len() as u32, 32, hints.next());
            bw.write_field(inter.flags, 32, hints.next());
            bw.write_field(inter.string_index, 32, hints.next());
            if chunk.version > 6 {
                bw.write_field(inter.attenuation_code, 32, hints.next());
            }
            bw.write_field(inter.attenuation.to_bits(), 32, hints.next());
            bw.write_field(inter.autonomy, 32, hints.next());
            bw.write_field(inter.joining_index as u32, 32, hints.next());
            for motive in &inter.motives {
                if chunk.version > 6 {
                    bw.write_field(u32::from(motive.minimum as u16), 16, hints.next());
                }
                bw.write_field(u32::from(motive.delta as u16), 16, hints.next());
                if chunk.version > 6 {
                    bw.write_field(u32::from(motive.personality), 16, hints.next());
                }
            }
            if chunk.version > 9 {
                bw.write_field(inter.flags2, 32, hints.next());
            }
        }
        out.extend_from_slice(&bw.finish(chunk.trailing_bits));
    }

    out.extend_from_slice(&chunk.trailing);
    Ok(out)
}

/// Encodes a `SLOT` routing/container slot table chunk (versions `4..=10`).
pub fn encode_slot(chunk: &SlotChunk) -> Result<Vec<u8>> {
    if !(4..=10).contains(&chunk.version) {
        return Err(Error::new(format!(
            "unsupported SLOT version {}",
            chunk.version
        )));
    }
    let count = u32::try_from(chunk.entries.len())
        .map_err(|_| Error::new("SLOT entry count exceeds u32::MAX"))?;
    let mut out = Vec::new();
    out.extend_from_slice(&chunk.reserved.to_le_bytes());
    out.extend_from_slice(&chunk.version.to_le_bytes());
    out.extend_from_slice(b"TOLS");
    out.extend_from_slice(&count.to_le_bytes());
    for slot in &chunk.entries {
        if !slot.gradient.is_finite() || slot.offset.iter().any(|v| !v.is_finite()) {
            return Err(Error::new("SLOT contains non-finite float geometry"));
        }
        out.extend_from_slice(&slot.kind.to_le_bytes());
        for coord in slot.offset {
            out.extend_from_slice(&coord.to_bits().to_le_bytes());
        }
        out.extend_from_slice(&slot.standing.to_le_bytes());
        out.extend_from_slice(&slot.sitting.to_le_bytes());
        out.extend_from_slice(&slot.ground.to_le_bytes());
        out.extend_from_slice(&slot.flags.to_le_bytes());
        out.extend_from_slice(&slot.snap_target.to_le_bytes());
        if chunk.version >= 6 {
            for p in slot.proximity {
                out.extend_from_slice(&p.to_le_bytes());
            }
            out.extend_from_slice(&slot.max_size.to_le_bytes());
            out.extend_from_slice(&slot.unknown.to_le_bytes());
        }
        if chunk.version >= 7 {
            out.extend_from_slice(&slot.gradient.to_bits().to_le_bytes());
        }
        if chunk.version >= 8 {
            out.extend_from_slice(&slot.height.to_le_bytes());
        }
        if chunk.version >= 9 {
            out.extend_from_slice(&slot.facing.to_le_bytes());
        }
        if chunk.version >= 10 {
            out.extend_from_slice(&slot.resolution.to_le_bytes());
        }
    }
    out.extend_from_slice(&chunk.trailing);
    Ok(out)
}

/// Encodes a `PALT` 256-color palette chunk.
pub fn encode_palt(chunk: &PaltChunk) -> Result<Vec<u8>> {
    if chunk.colors.len() > 256 {
        return Err(Error::new("PALT palette exceeds 256 colors"));
    }
    let mut out = Vec::with_capacity(16 + chunk.colors.len() * 3);
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(chunk.colors.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 8]);
    for rgb in &chunk.colors {
        out.extend_from_slice(rgb);
    }
    Ok(out)
}

fn encode_spr2_frame(frame: &Spr2Frame) -> Result<Vec<u8>> {
    let width = usize::from(frame.width);
    let height = usize::from(frame.height);
    let expected = width
        .checked_mul(height)
        .ok_or_else(|| Error::new("SPR2 frame dimensions overflow"))?;
    if frame.pixels.len() != expected {
        return Err(Error::new(format!(
            "SPR2 frame expected {expected} pixels, got {}",
            frame.pixels.len()
        )));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&frame.width.to_le_bytes());
    out.extend_from_slice(&frame.height.to_le_bytes());
    out.extend_from_slice(&frame.flags.to_le_bytes());
    out.extend_from_slice(&frame.palette_id.to_le_bytes());
    out.extend_from_slice(&frame.transparent_index.to_le_bytes());
    // Note: Frame::decode reads `y` before `x`.
    out.extend_from_slice(&frame.y.to_le_bytes());
    out.extend_from_slice(&frame.x.to_le_bytes());

    let has_depth = frame.flags & 2 != 0;
    let mut row = 0usize;
    while row < height {
        // Group consecutive completely transparent rows using row command 4.
        let mut empty_rows = 0usize;
        while row + empty_rows < height && empty_rows < 0x1FFF {
            let start = (row + empty_rows) * width;
            if frame.pixels[start..start + width]
                .iter()
                .all(|p| p.alpha == 0)
            {
                empty_rows += 1;
            } else {
                break;
            }
        }
        if empty_rows > 0 {
            let marker = (4u16 << 13) | (empty_rows as u16);
            out.extend_from_slice(&marker.to_le_bytes());
            row += empty_rows;
            continue;
        }

        let row_pixels = &frame.pixels[row * width..(row + 1) * width];
        let row_bytes = encode_spr2_row(row_pixels, has_depth)?;
        let row_size = u16::try_from(row_bytes.len() + 2)
            .ok()
            .filter(|&n| n <= 0x1FFF)
            .ok_or_else(|| Error::new("SPR2 encoded row exceeds 8191-byte limit"))?;
        // Row command 0: pixel row of `row_size` bytes including the 2-byte marker.
        out.extend_from_slice(&row_size.to_le_bytes());
        out.extend_from_slice(&row_bytes);
        row += 1;
    }
    // End of sprite marker: command 5.
    out.extend_from_slice(&(5u16 << 13).to_le_bytes());
    Ok(out)
}

fn encode_spr2_row(pixels: &[Spr2Pixel], has_depth: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut col = 0usize;
    let width = pixels.len();
    while col < width {
        let p = pixels[col];
        if p.alpha > 31 {
            return Err(Error::new("SPR2 pixel alpha exceeds 5-bit range (0..=31)"));
        }
        if p.alpha == 0 {
            let start = col;
            while col < width && col - start < 0x1FFF && pixels[col].alpha == 0 {
                col += 1;
            }
            let n = (col - start) as u16;
            // Op 3: skip transparent pixels.
            out.extend_from_slice(&((3u16 << 13) | n).to_le_bytes());
        } else if p.alpha == 31 && !has_depth {
            let start = col;
            while col < width && col - start < 0x1FFF && pixels[col].alpha == 31 {
                col += 1;
            }
            let n = col - start;
            // Op 6: opaque pixels without z-buffer.
            out.extend_from_slice(&((6u16 << 13) | (n as u16)).to_le_bytes());
            for px in &pixels[start..col] {
                out.push(px.color);
            }
            if !n.is_multiple_of(2) {
                out.push(0);
            }
        } else if p.alpha == 31 {
            let start = col;
            while col < width && col - start < 0x1FFF && pixels[col].alpha == 31 {
                col += 1;
            }
            let n = col - start;
            // Op 1: opaque pixels with z-buffer (2 bytes per pixel, always even).
            out.extend_from_slice(&((1u16 << 13) | (n as u16)).to_le_bytes());
            for px in &pixels[start..col] {
                out.push(px.depth);
                out.push(px.color);
            }
        } else {
            let start = col;
            while col < width
                && col - start < 0x1FFF
                && pixels[col].alpha > 0
                && pixels[col].alpha < 31
            {
                col += 1;
            }
            let n = col - start;
            // Op 2: translucent pixels with z-buffer + color + 5-bit alpha.
            out.extend_from_slice(&((2u16 << 13) | (n as u16)).to_le_bytes());
            for px in &pixels[start..col] {
                out.push(px.depth);
                out.push(px.color);
                out.push(px.alpha);
            }
            if !n.is_multiple_of(2) {
                out.push(0);
            }
        }
    }
    Ok(out)
}

/// Encodes an `SPR2` sprite set chunk (version `1000` or `1001`).
pub fn encode_spr2(chunk: &Spr2Chunk) -> Result<Vec<u8>> {
    if !matches!(chunk.version, 1000 | 1001) {
        return Err(Error::new(format!(
            "unsupported SPR2 version {}",
            chunk.version
        )));
    }
    let count = u32::try_from(chunk.frames.len())
        .map_err(|_| Error::new("SPR2 frame count exceeds u32::MAX"))?;
    let mut encoded_frames = Vec::with_capacity(chunk.frames.len());
    for frame in &chunk.frames {
        encoded_frames.push(encode_spr2_frame(frame)?);
    }

    let mut out = Vec::new();
    out.extend_from_slice(&chunk.version.to_le_bytes());
    if chunk.version == 1000 {
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&chunk.default_palette.to_le_bytes());
        let mut offset = 12usize
            .checked_add(encoded_frames.len().saturating_mul(4))
            .ok_or_else(|| Error::new("SPR2 offset overflow"))?;
        for frame_bytes in &encoded_frames {
            let off_u32 =
                u32::try_from(offset).map_err(|_| Error::new("SPR2 frame offset exceeds u32"))?;
            out.extend_from_slice(&off_u32.to_le_bytes());
            offset = offset
                .checked_add(frame_bytes.len())
                .ok_or_else(|| Error::new("SPR2 total size overflow"))?;
        }
        for frame_bytes in encoded_frames {
            out.extend_from_slice(&frame_bytes);
        }
    } else {
        out.extend_from_slice(&chunk.default_palette.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        for frame_bytes in encoded_frames {
            let len_u32 = u32::try_from(frame_bytes.len())
                .map_err(|_| Error::new("SPR2 frame length exceeds u32"))?;
            out.extend_from_slice(&1001u32.to_le_bytes());
            out.extend_from_slice(&len_u32.to_le_bytes());
            out.extend_from_slice(&frame_bytes);
        }
    }
    Ok(out)
}

/// Encodes a `DGRP` isometric draw group chunk (versions `20000..=20004`).
pub fn encode_dgrp(chunk: &DgrpChunk) -> Result<Vec<u8>> {
    if !(20000..=20004).contains(&chunk.version) {
        return Err(Error::new(format!(
            "unsupported DGRP version {}",
            chunk.version
        )));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&chunk.version.to_le_bytes());
    if chunk.version < 20003 {
        let count = u16::try_from(chunk.views.len())
            .map_err(|_| Error::new("DGRP view count exceeds u16::MAX"))?;
        out.extend_from_slice(&count.to_le_bytes());
        for view in &chunk.views {
            let n = u16::try_from(view.sprites.len())
                .map_err(|_| Error::new("DGRP sprite count exceeds u16::MAX"))?;
            let dir = u8::try_from(view.direction)
                .map_err(|_| Error::new("DGRP v20000..20002 direction exceeds u8"))?;
            let zoom = u8::try_from(view.zoom)
                .map_err(|_| Error::new("DGRP v20000..20002 zoom exceeds u8"))?;
            out.extend_from_slice(&n.to_le_bytes());
            out.push(dir);
            out.push(zoom);
            for s in &view.sprites {
                if s.object.iter().any(|v| !v.is_finite()) {
                    return Err(Error::new("non-finite DGRP object offset"));
                }
                let id = u16::try_from(s.id)
                    .map_err(|_| Error::new("DGRP v20000..20002 sprite id exceeds u16"))?;
                let frame = u16::try_from(s.frame)
                    .map_err(|_| Error::new("DGRP v20000..20002 frame exceeds u16"))?;
                let flags = u16::try_from(s.flags)
                    .map_err(|_| Error::new("DGRP v20000..20002 flags exceed u16"))?;
                let x = i16::try_from(s.x)
                    .map_err(|_| Error::new("DGRP v20000..20002 x exceeds i16"))?;
                let y = i16::try_from(s.y)
                    .map_err(|_| Error::new("DGRP v20000..20002 y exceeds i16"))?;
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(&id.to_le_bytes());
                out.extend_from_slice(&frame.to_le_bytes());
                out.extend_from_slice(&flags.to_le_bytes());
                out.extend_from_slice(&x.to_le_bytes());
                out.extend_from_slice(&y.to_le_bytes());
                if chunk.version == 20001 {
                    out.extend_from_slice(&s.object[2].to_bits().to_le_bytes());
                }
            }
        }
    } else {
        let count = u32::try_from(chunk.views.len())
            .map_err(|_| Error::new("DGRP view count exceeds u32::MAX"))?;
        out.extend_from_slice(&count.to_le_bytes());
        for view in &chunk.views {
            let n = u32::try_from(view.sprites.len())
                .map_err(|_| Error::new("DGRP sprite count exceeds u32::MAX"))?;
            out.extend_from_slice(&view.direction.to_le_bytes());
            out.extend_from_slice(&view.zoom.to_le_bytes());
            out.extend_from_slice(&n.to_le_bytes());
            for s in &view.sprites {
                if s.object.iter().any(|v| !v.is_finite()) {
                    return Err(Error::new("non-finite DGRP object offset"));
                }
                out.extend_from_slice(&s.id.to_le_bytes());
                out.extend_from_slice(&s.frame.to_le_bytes());
                out.extend_from_slice(&s.x.to_le_bytes());
                out.extend_from_slice(&s.y.to_le_bytes());
                out.extend_from_slice(&s.object[2].to_bits().to_le_bytes());
                out.extend_from_slice(&s.flags.to_le_bytes());
                if chunk.version == 20004 {
                    out.extend_from_slice(&s.object[0].to_bits().to_le_bytes());
                    out.extend_from_slice(&s.object[1].to_bits().to_le_bytes());
                }
            }
        }
    }
    Ok(out)
}

/// Encodes an `FWAV` sound event name chunk (NUL-terminated Windows-1252).
pub fn encode_fwav(chunk: &FwavChunk) -> Result<Vec<u8>> {
    let mut bytes = encode_string_bytes(&chunk.sound_name, 1)?;
    bytes.push(0);
    Ok(bytes)
}

/// Encodes chunks into a Maxis IFF 2.5 container with an authentic version-0
/// `rsmp` resource map by reusing [`sims_formats::iff_write::write_chunk_with_residue`].
pub fn encode_iff_with_rsmp(chunks: &[ChunkData]) -> Result<Vec<u8>> {
    static ZERO_PAD: [u8; 76] = [0u8; 76];
    let mut bytes = sims_formats::iff::write(&[])?;
    for chunk in chunks {
        let label = sims_formats::far::encode_name(&chunk.label)
            .filter(|b| b.len() <= 63 && !b.contains(&0))
            .ok_or_else(|| Error::new(format!("invalid IFF chunk label '{}'", chunk.label)))?;
        let flags = if chunk.flags == 0 {
            IFF_CONVERTED_FLAG
        } else {
            chunk.flags
        };
        bytes = iff_write::write_chunk_with_residue(
            &bytes,
            ChunkWrite {
                tag: chunk.tag,
                id: chunk.id,
                flags,
                label: &label,
                payload: &chunk.data,
            },
            |range| ZERO_PAD.get(..range.len()),
        )?;
    }
    Ok(bytes)
}

/// Encodes multiple files into a `FAR!byAZ` version 1 archive compatible with
/// [`sims_formats::far`].
pub fn encode_far(entries: &[FarEntry<'_>]) -> Result<Vec<u8>> {
    const HEADER_LEN: usize = 16;
    let mut payload = Vec::new();
    let mut directory_records = Vec::with_capacity(entries.len());

    for entry in entries {
        let raw_name = sims_formats::far::encode_name(entry.name)
            .filter(|b| !b.is_empty() && b.len() <= 32768 && !b.contains(&0))
            .ok_or_else(|| Error::new(format!("invalid FAR member name '{}'", entry.name)))?;
        let offset = u32::try_from(
            HEADER_LEN
                .checked_add(payload.len())
                .ok_or_else(|| Error::new("FAR archive payload offset overflow"))?,
        )
        .map_err(|_| Error::new("FAR archive payload exceeds 4 GiB"))?;
        let size =
            u32::try_from(entry.data.len()).map_err(|_| Error::new("FAR member exceeds 4 GiB"))?;
        payload.extend_from_slice(entry.data);
        directory_records.push((size, offset, raw_name));
    }

    let manifest_offset = u32::try_from(
        HEADER_LEN
            .checked_add(payload.len())
            .ok_or_else(|| Error::new("FAR manifest offset overflow"))?,
    )
    .map_err(|_| Error::new("FAR manifest offset exceeds u32"))?;

    let mut out = Vec::with_capacity(HEADER_LEN + payload.len() + entries.len() * 32);
    out.extend_from_slice(b"FAR!byAZ");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&manifest_offset.to_le_bytes());
    out.extend_from_slice(&payload);

    let count = u32::try_from(directory_records.len())
        .map_err(|_| Error::new("FAR entry count exceeds u32"))?;
    out.extend_from_slice(&count.to_le_bytes());
    for (size, offset, raw_name) in directory_records {
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(raw_name.len() as u32).to_le_bytes());
        out.extend_from_slice(&raw_name);
    }
    Ok(out)
}
