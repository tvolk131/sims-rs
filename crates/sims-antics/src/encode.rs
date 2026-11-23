//! Checked binary encoders for Sims 1 IFF chunks, IFF 2.5 containers (with
//! version-0 `rsmp` resource maps), and `FAR!byAZ` archives.

use crate::error::{Error, Result};
use crate::ir::{
    BconChunk, BhavChunk, DgrpChunk, FarEntry, FwavChunk, LegacySprChunk, LegacySprFrame,
    ObjdChunk, ObjfChunk, PaltChunk, SlotChunk, Spr2Chunk, Spr2Frame, Spr2Pixel, StringFormat,
    StringTableChunk, TtabChunk,
};
use sims_formats::far::{MAX_DIRECTORY, MAX_MEMBER};
use sims_formats::iff::{ChunkData, DELETED};
use std::collections::BTreeSet;

/// Maximum sprite frame pixel count matching `sims_formats::sprite::MAX_PIXELS`.
const MAX_SPR2_PIXELS: usize = 16 * 1024 * 1024;
const IFF_FILE_HEADER_SIZE: usize = 64;
const IFF_CHUNK_HEADER_SIZE: usize = 76;
const IFF_SIGNATURE_25: &[u8; 60] =
    b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1";
const IFF_CONVERTED: u16 = 0x0010;

/// Encodes a `BHAV` chunk (formats `0x8000`, `0x8001`, `0x8002`).
pub fn encode_bhav(chunk: &BhavChunk) -> Result<Vec<u8>> {
    if !matches!(chunk.format, 0x8000..=0x8002) {
        return Err(Error::new(format!(
            "unsupported BHAV format {:#06x} (supported: 0x8000, 0x8001, 0x8002)",
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

/// Encodes an `OBJD` chunk (versions `136 | 138 | 139 | 140 | 141 | 142`,
/// matching `sims_formats::behavior::ObjectDefinition::parse`).
pub fn encode_objd(chunk: &ObjdChunk) -> Result<Vec<u8>> {
    if !matches!(chunk.version, 136 | 138 | 139 | 140 | 141 | 142) {
        return Err(Error::new(format!(
            "unsupported OBJD version {} (supported: 136, 138..=142)",
            chunk.version
        )));
    }
    if chunk.words.len() < 42 {
        return Err(Error::new(format!(
            "OBJD has {} words ({} bytes), below minimum 42 words (84 bytes) required by ObjectDefinition::parse",
            chunk.words.len(),
            chunk.words.len() * 2
        )));
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

    /// Reconstructs the decoded value produced by `sims_formats::behavior::Fields::read`
    /// for a given bit width and field size (16 or 32 bits).
    fn reconstruct_signed(raw_bits: u32, width: usize, size: usize) -> u32 {
        let signed = if width < 32 && raw_bits & (1u32 << (width - 1)) != 0 {
            raw_bits | (!0u32 << width)
        } else {
            raw_bits
        };
        if size == 16 { signed & 0xFFFF } else { signed }
    }

    fn write_field(&mut self, value: u32, size: usize, width_hint: Option<u8>) {
        let widths = if size == 16 {
            [5usize, 8, 13, 16]
        } else {
            [6usize, 11, 21, 32]
        };
        let target_val = if size == 16 { value & 0xFFFF } else { value };

        // Honor a preserved width hint ONLY when it losslessly reconstructs `target_val`.
        if let Some(hint) = width_hint {
            if hint == 255 && target_val == 0 {
                self.push_bits(0, 1);
                return;
            }
            if let Some(&width) = widths.get(usize::from(hint)) {
                let mask = if width == 32 {
                    u32::MAX
                } else {
                    (1u32 << width) - 1
                };
                let raw_bits = target_val & mask;
                if Self::reconstruct_signed(raw_bits, width, size) == target_val {
                    self.push_bits(1, 1);
                    self.push_bits(u32::from(hint), 2);
                    self.push_bits(raw_bits, width);
                    return;
                }
            }
        }

        if target_val == 0 {
            self.push_bits(0, 1);
            return;
        }
        let mut chosen_code = 3u32;
        for (code, &width) in widths.iter().enumerate() {
            let mask = if width == 32 {
                u32::MAX
            } else {
                (1u32 << width) - 1
            };
            if Self::reconstruct_signed(target_val & mask, width, size) == target_val {
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
        self.push_bits(target_val & mask, width);
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
    if expected > MAX_SPR2_PIXELS {
        return Err(Error::new(format!(
            "SPR2 frame pixel count {expected} exceeds 16 megapixel reader limit"
        )));
    }
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

fn encode_legacy_spr_frame(frame: &LegacySprFrame) -> Result<Vec<u8>> {
    let width = usize::from(frame.width);
    let height = usize::from(frame.height);
    let expected = width
        .checked_mul(height)
        .ok_or_else(|| Error::new("SPR# frame dimensions overflow"))?;
    if expected > MAX_SPR2_PIXELS {
        return Err(Error::new("SPR# frame exceeds 16 megapixel reader limit"));
    }
    if frame.pixels.len() != expected {
        return Err(Error::new(format!(
            "SPR# frame expected {expected} pixels, got {}",
            frame.pixels.len()
        )));
    }
    let bottom = i16::try_from(frame.height)
        .map_err(|_| Error::new("SPR# frame height exceeds i16::MAX"))?;
    let right =
        i16::try_from(frame.width).map_err(|_| Error::new("SPR# frame width exceeds i16::MAX"))?;

    let mut out = Vec::new();
    // Native Rect: top, left, bottom, right, plus 2 skipped bytes.
    out.extend_from_slice(&0i16.to_le_bytes());
    out.extend_from_slice(&0i16.to_le_bytes());
    out.extend_from_slice(&bottom.to_le_bytes());
    out.extend_from_slice(&right.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    let mut row = 0usize;
    while row < height {
        let mut empty = 0usize;
        while row + empty < height && empty < 255 {
            let start = (row + empty) * width;
            if frame.pixels[start..start + width]
                .iter()
                .all(Option::is_none)
            {
                empty += 1;
            } else {
                break;
            }
        }
        if empty > 0 {
            out.push(9);
            out.push(empty as u8);
            row += empty;
            continue;
        }

        let row_pixels = &frame.pixels[row * width..(row + 1) * width];
        let mut row_buf = Vec::new();
        let mut col = 0usize;
        while col < width {
            match row_pixels[col] {
                None => {
                    let start = col;
                    while col < width && col - start < 255 && row_pixels[col].is_none() {
                        col += 1;
                    }
                    row_buf.push(1);
                    row_buf.push((col - start) as u8);
                }
                Some(first) => {
                    let mut same = 1usize;
                    while col + same < width && same < 255 && row_pixels[col + same] == Some(first)
                    {
                        same += 1;
                    }
                    if same >= 3 {
                        row_buf.push(2);
                        row_buf.push(same as u8);
                        row_buf.push(first);
                        row_buf.push(first);
                        col += same;
                    } else {
                        let start = col;
                        let mut literal = Vec::new();
                        while col < width && col - start < 255 {
                            let Some(c) = row_pixels[col] else {
                                break;
                            };
                            literal.push(c);
                            col += 1;
                        }
                        let n = literal.len();
                        row_buf.push(3);
                        row_buf.push(n as u8);
                        row_buf.extend_from_slice(&literal);
                        if !n.is_multiple_of(2) {
                            row_buf.push(0);
                        }
                    }
                }
            }
        }
        if row_buf.len() + 2 <= 255 && row_buf.len() + 2 != 2 {
            out.push(4);
            out.push((row_buf.len() + 2) as u8);
            out.extend_from_slice(&row_buf);
        } else {
            let total = u16::try_from(row_buf.len() + 6)
                .map_err(|_| Error::new("SPR# encoded row exceeds u16::MAX"))?;
            out.push(4);
            out.push(2);
            out.push(5);
            out.push(255);
            out.extend_from_slice(&total.to_be_bytes());
            out.extend_from_slice(&row_buf);
        }
        row += 1;
    }
    out.push(5);
    out.push(0);
    Ok(out)
}

/// Encodes a legacy `SPR#` sprite set chunk (versions `502..=505`, little-endian)
/// compatible with `sims_formats::sprite::LegacySpriteSet::parse`.
pub fn encode_legacy_spr(chunk: &LegacySprChunk) -> Result<Vec<u8>> {
    if !(502..=505).contains(&chunk.version) {
        return Err(Error::new(format!(
            "unsupported SPR# version {} (supported: 502..=505)",
            chunk.version
        )));
    }
    if chunk.frames.len() > 4096 {
        return Err(Error::new("SPR# frame count exceeds 4096 limit"));
    }
    let mut encoded_frames = Vec::with_capacity(chunk.frames.len());
    for frame in &chunk.frames {
        encoded_frames.push(encode_legacy_spr_frame(frame)?);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&chunk.version.to_le_bytes());
    out.extend_from_slice(&(encoded_frames.len() as u32).to_le_bytes());
    out.extend_from_slice(&chunk.default_palette.to_le_bytes());
    let mut offset = 12usize
        .checked_add(encoded_frames.len().saturating_mul(4))
        .ok_or_else(|| Error::new("SPR# offset overflow"))?;
    for frame_bytes in &encoded_frames {
        let off_u32 =
            u32::try_from(offset).map_err(|_| Error::new("SPR# frame offset exceeds u32"))?;
        out.extend_from_slice(&off_u32.to_le_bytes());
        offset = offset
            .checked_add(frame_bytes.len())
            .ok_or_else(|| Error::new("SPR# total size overflow"))?;
    }
    for frame_bytes in encoded_frames {
        out.extend_from_slice(&frame_bytes);
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

/// Encodes an `FWAV` sound event name chunk (NUL-terminated Latin-1 bytes matching
/// `sims-vm/src/program.rs:416`, which decodes FWAV bytes via `char::from(b)`).
pub fn encode_fwav(chunk: &FwavChunk) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(chunk.sound_name.len() + 1);
    for ch in chunk.sound_name.chars() {
        let code = u32::from(ch);
        if code == 0 || code > 0xFF {
            return Err(Error::new(format!(
                "FWAV sound name '{}' contains unsupported character {:?}",
                chunk.sound_name, ch
            )));
        }
        bytes.push(code as u8);
    }
    bytes.push(0);
    Ok(bytes)
}

/// Encodes chunks into a fresh Maxis IFF 2.5 container with an authentic
/// version-0 `rsmp` resource map.
///
/// Preserves exact caller-supplied `flags` (including `0` for big-endian
/// resources), preserves distinct `(tag, id, flags >> 8)` variants, and
/// rejects duplicate resource identities instead of silently replacing them.
pub fn encode_iff_with_rsmp(chunks: &[ChunkData]) -> Result<Vec<u8>> {
    struct MapEntry {
        offset: u32,
        id: u16,
        flags: u16,
        label: Vec<u8>,
    }

    let mut seen_keys = BTreeSet::new();
    let mut type_order: Vec<[u8; 4]> = Vec::new();
    let mut groups: Vec<([u8; 4], Vec<MapEntry>)> = Vec::new();

    let mut bytes = Vec::with_capacity(IFF_FILE_HEADER_SIZE);
    bytes.extend_from_slice(IFF_SIGNATURE_25);
    bytes.resize(IFF_FILE_HEADER_SIZE, 0);

    for chunk in chunks {
        if chunk.tag == *b"rsmp" {
            return Err(Error::new("cannot explicitly supply an 'rsmp' chunk"));
        }
        if chunk.flags & DELETED != 0 {
            return Err(Error::new(
                "cannot encode a chunk marked DELETED into a fresh IFF",
            ));
        }
        if chunk.flags & !(0xFF00 | IFF_CONVERTED) != 0 {
            return Err(Error::new(format!(
                "unsupported IFF chunk flags {:#06x} on {} #{}",
                chunk.flags,
                String::from_utf8_lossy(&chunk.tag),
                chunk.id
            )));
        }
        let key = (chunk.tag, chunk.id, (chunk.flags >> 8) as u8);
        if !seen_keys.insert(key) {
            return Err(Error::new(format!(
                "duplicate IFF resource {} #{} (variant {})",
                String::from_utf8_lossy(&chunk.tag),
                chunk.id,
                key.2
            )));
        }
        let label = sims_formats::far::encode_name(&chunk.label)
            .filter(|b| b.len() <= 63 && !b.contains(&0))
            .ok_or_else(|| Error::new(format!("invalid IFF chunk label '{}'", chunk.label)))?;

        let offset = bytes.len();
        let offset_u32 = u32::try_from(offset)
            .ok()
            .filter(|&o| o <= i32::MAX as u32)
            .ok_or_else(|| Error::new("IFF file offset exceeds 2 GiB limit"))?;
        let chunk_size = u32::try_from(chunk.data.len())
            .ok()
            .and_then(|n| n.checked_add(IFF_CHUNK_HEADER_SIZE as u32))
            .filter(|&n| n <= i32::MAX as u32)
            .ok_or_else(|| Error::new("IFF chunk size exceeds 2 GiB limit"))?;

        bytes.extend_from_slice(&chunk.tag);
        bytes.extend_from_slice(&chunk_size.to_be_bytes());
        bytes.extend_from_slice(&chunk.id.to_be_bytes());
        bytes.extend_from_slice(&chunk.flags.to_be_bytes());
        bytes.extend_from_slice(&label);
        bytes.resize(offset + IFF_CHUNK_HEADER_SIZE, 0);
        bytes.extend_from_slice(&chunk.data);

        let entry = MapEntry {
            offset: offset_u32,
            id: chunk.id,
            flags: chunk.flags,
            label,
        };
        if let Some(pos) = type_order.iter().position(|&t| t == chunk.tag) {
            groups[pos].1.push(entry);
        } else {
            type_order.push(chunk.tag);
            groups.push((chunk.tag, vec![entry]));
        }
    }

    // Build version-0 rsmp payload matching `sims_formats::iff::ResourceMap::parse`.
    let mut map = Vec::from(*b"\0\0\0\0\0\0\0\0pmsr");
    map.extend_from_slice(&0u32.to_le_bytes()); // unused_bytes = 0
    map.extend_from_slice(&(groups.len() as u32).to_le_bytes());
    for (tag, entries) in &groups {
        map.extend_from_slice(&u32::from_be_bytes(*tag).to_le_bytes());
        map.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for entry in entries {
            map.extend_from_slice(&entry.offset.to_le_bytes());
            map.extend_from_slice(&entry.id.to_le_bytes());
            map.extend_from_slice(&entry.flags.to_le_bytes());
            map.extend_from_slice(&entry.label);
            map.push(0);
            if map.len() & 1 != 0 {
                map.push(0);
            }
        }
    }

    let map_offset = bytes.len();
    let map_offset_u32 = u32::try_from(map_offset)
        .ok()
        .filter(|&o| o <= i32::MAX as u32)
        .ok_or_else(|| Error::new("IFF rsmp offset exceeds 2 GiB limit"))?;
    let map_chunk_size = u32::try_from(map.len())
        .ok()
        .and_then(|n| n.checked_add(IFF_CHUNK_HEADER_SIZE as u32))
        .filter(|&n| n <= i32::MAX as u32)
        .ok_or_else(|| Error::new("IFF rsmp chunk exceeds 2 GiB limit"))?;

    bytes.extend_from_slice(b"rsmp");
    bytes.extend_from_slice(&map_chunk_size.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes());
    bytes.extend_from_slice(&IFF_CONVERTED.to_be_bytes());
    bytes.resize(map_offset + IFF_CHUNK_HEADER_SIZE, 0);
    bytes.extend_from_slice(&map);
    bytes[60..IFF_FILE_HEADER_SIZE].copy_from_slice(&map_offset_u32.to_be_bytes());
    Ok(bytes)
}

/// Encodes multiple files into a `FAR!byAZ` version 1 archive compatible with
/// [`sims_formats::far`], enforcing the reader's 128 MiB member limit and
/// 64 MiB directory limit prior to allocation.
pub fn encode_far(entries: &[FarEntry<'_>]) -> Result<Vec<u8>> {
    const HEADER_LEN: u64 = 16;

    // Pre-validate names, member sizes, payload offsets, and directory size before allocating.
    let mut encoded_names = Vec::with_capacity(entries.len());
    let mut total_payload: u64 = 0;
    let mut dir_bytes: u64 = 4; // u32 entry count

    for entry in entries {
        let raw_name = sims_formats::far::encode_name(entry.name)
            .filter(|b| !b.is_empty() && b.len() <= 32768 && !b.contains(&0))
            .ok_or_else(|| Error::new(format!("invalid FAR member name '{}'", entry.name)))?;
        let member_len = entry.data.len() as u64;
        if member_len > MAX_MEMBER {
            return Err(Error::new(format!(
                "FAR member '{}' ({member_len} bytes) exceeds 128 MiB limit",
                entry.name
            )));
        }
        total_payload = total_payload
            .checked_add(member_len)
            .ok_or_else(|| Error::new("FAR archive payload length overflow"))?;
        dir_bytes = dir_bytes
            .checked_add(16)
            .and_then(|n| n.checked_add(raw_name.len() as u64))
            .ok_or_else(|| Error::new("FAR directory size overflow"))?;
        if dir_bytes > MAX_DIRECTORY {
            return Err(Error::new("FAR directory exceeds 64 MiB reader limit"));
        }
        encoded_names.push(raw_name);
    }

    let manifest_offset_u64 = HEADER_LEN
        .checked_add(total_payload)
        .ok_or_else(|| Error::new("FAR manifest offset overflow"))?;
    let manifest_offset = u32::try_from(manifest_offset_u64)
        .map_err(|_| Error::new("FAR manifest offset exceeds u32"))?;
    let total_len = usize::try_from(
        manifest_offset_u64
            .checked_add(dir_bytes)
            .ok_or_else(|| Error::new("FAR total archive size overflow"))?,
    )
    .map_err(|_| Error::new("FAR archive exceeds addressable memory"))?;

    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(b"FAR!byAZ");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&manifest_offset.to_le_bytes());

    let mut offsets = Vec::with_capacity(entries.len());
    for entry in entries {
        let offset = out.len() as u32;
        offsets.push(offset);
        out.extend_from_slice(entry.data);
    }

    let count =
        u32::try_from(entries.len()).map_err(|_| Error::new("FAR entry count exceeds u32"))?;
    out.extend_from_slice(&count.to_le_bytes());
    for ((entry, raw_name), offset) in entries.iter().zip(encoded_names).zip(offsets) {
        let size = entry.data.len() as u32;
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(raw_name.len() as u32).to_le_bytes());
        out.extend_from_slice(&raw_name);
    }
    Ok(out)
}
