//! In-memory chunk lifter from parsed `sims_formats` / raw IFF chunks into
//! `sims-antics` IR and canonical low-level `.air` (Antics IR) source text.
//!
//! Strictly in-memory: never writes any lifted representation to disk.

use crate::error::{Error, Result};
use crate::ir::{
    BconChunk, BhavChunk, ObjdChunk, ObjfChunk, SlotChunk, StringEntry, StringFormat,
    StringTableChunk, TtabChunk,
};
use crate::structure_lift::{LiftOptions, decode_instruction_for_labeled_air};
use sims_formats::behavior::{Behavior, Interaction, InteractionTable, MotiveAdvertisement};
use sims_formats::execution::{ObjectFunctions, Slots};
use sims_formats::strings::{Format, StringTable};
use std::collections::BTreeMap;
use std::fmt::Write;

/// Lifts a raw `BHAV` chunk payload into a [`BhavChunk`] IR.
pub fn lift_bhav(id: u16, label: &str, data: &[u8]) -> Result<BhavChunk> {
    let behavior = Behavior::parse(data)?;
    let consumed = 12 + behavior.instructions.len() * 12;
    let trailing = data.get(consumed..).unwrap_or(&[]).to_vec();
    Ok(BhavChunk {
        id,
        label: label.to_owned(),
        format: behavior.format,
        kind: behavior.kind,
        args: behavior.args,
        locals: behavior.locals,
        revision: behavior.revision,
        instructions: behavior.instructions,
        trailing,
    })
}

/// Formats a [`BhavChunk`] into canonical decoded labeled-CFG `.air` (Antics IR) source text
/// using [`LiftOptions::default`] (`strict_padding: true`), preserving `@n0:` labels and
/// `-> true: ..., false: ...` edges while decoding every clean modeled instruction so it
/// parses and lowers back to the exact same instruction stream and payload bytes.
#[must_use]
pub fn format_bhav_air(chunk: &BhavChunk) -> String {
    format_bhav_labeled_air(chunk, LiftOptions::default())
}

/// Alias for [`format_bhav_air`].
#[must_use]
pub fn format_bhav_antics(chunk: &BhavChunk) -> String {
    format_bhav_air(chunk)
}

/// Formats a [`BhavChunk`] into labeled-CFG `.air` (Antics IR) source text, keeping
/// `@n0:` labels and `-> true: ..., false: ...` edges while decoding every modeled
/// instruction according to `opts`.
///
/// With `opts.strict_padding == true`, every decoded instruction encodes back to the
/// exact original 8 operand bytes (falling back to `op 0xNNNN (...)` per instruction on
/// unmodeled opcodes, dirty padding bytes, or out-of-range operands), guaranteeing 100%
/// byte-exact BHAV round-tripping.
#[must_use]
pub fn format_bhav_labeled_air(chunk: &BhavChunk, opts: LiftOptions) -> String {
    format_bhav_labeled_air_with_arities(chunk, opts, None)
}

/// Formats a [`BhavChunk`] into labeled-CFG `.air` (Antics IR) source text using optional
/// `callee_arities` to protect trailing instruction records that may carry extended (>4-arg)
/// inline subroutine arguments when `opts.strict_padding == false` (`--relax-padding`).
///
/// Any instruction record consumed by an extended inline call (or any record following an
/// inline call whose callee arity is unknown) is decoded with `strict_padding: true` so its
/// 12 raw bytes are preserved identically.
#[must_use]
pub fn format_bhav_labeled_air_with_arities(
    chunk: &BhavChunk,
    opts: LiftOptions,
    callee_arities: Option<&BTreeMap<u16, usize>>,
) -> String {
    let mut out = format_bhav_header(chunk);
    let bounds = match chunk.format {
        0x8000 => Some((4usize, 0usize)),
        0x8001 => Some((
            usize::from(chunk.args).min(4),
            usize::from(chunk.locals as u8).min(4),
        )),
        0x8002 => Some((usize::from(chunk.args), usize::from(chunk.locals as u8))),
        _ => None,
    };

    // Protect instruction records whose raw bytes can be read as extended (>4) inline
    // subroutine arguments (`sims-vm/src/interpreter.rs:1548`) when relaxing padding.
    let n = chunk.instructions.len();
    let mut protected = vec![false; n];
    if !opts.strict_padding {
        for (i, ins) in chunk.instructions.iter().enumerate() {
            if ins.opcode >= 256 && ins.operands != [0xFF; 8] {
                match callee_arities.and_then(|m| m.get(&ins.opcode).copied()) {
                    Some(arity) if arity <= 4 => {}
                    Some(arity) => {
                        let extra_records = (arity * 2 - 8).div_ceil(12);
                        let end = (i + 1 + extra_records).min(n);
                        for slot in &mut protected[i + 1..end] {
                            *slot = true;
                        }
                    }
                    None => {
                        for slot in &mut protected[i + 1..n] {
                            *slot = true;
                        }
                        break;
                    }
                }
            }
        }
    }

    for (idx, ins) in chunk.instructions.iter().enumerate() {
        let _ = writeln!(out, "    @n{idx}:");
        let on_true = format_target(ins.on_true, n);
        let on_false = format_target(ins.on_false, n);
        let ins_opts = if protected[idx] {
            LiftOptions {
                strict_padding: true,
            }
        } else {
            opts
        };
        if let Some((arg_bound, local_bound)) = bounds
            && let Some(decoded) =
                decode_instruction_for_labeled_air(ins, arg_bound, local_bound, ins_opts)
        {
            let _ = writeln!(out, "    {decoded} -> true: {on_true}, false: {on_false};");
        } else {
            let [b0, b1, b2, b3, b4, b5, b6, b7] = ins.operands;
            let _ = writeln!(
                out,
                "    op {:#06x} ({b0:#04x}, {b1:#04x}, {b2:#04x}, {b3:#04x}, {b4:#04x}, {b5:#04x}, {b6:#04x}, {b7:#04x}) -> true: {on_true}, false: {on_false};",
                ins.opcode
            );
        }
    }
    out.push_str("}\n");
    out
}

/// Formats a [`BhavChunk`] into raw hex-only `.air` (Antics IR) source text (`op 0xNNNN (...)`
/// for every instruction) for `--raw` inspection.
#[must_use]
pub fn format_bhav_raw_air(chunk: &BhavChunk) -> String {
    let mut out = format_bhav_header(chunk);
    for (idx, ins) in chunk.instructions.iter().enumerate() {
        let _ = writeln!(out, "    @n{idx}:");
        let on_true = format_target(ins.on_true, chunk.instructions.len());
        let on_false = format_target(ins.on_false, chunk.instructions.len());
        let [b0, b1, b2, b3, b4, b5, b6, b7] = ins.operands;
        let _ = writeln!(
            out,
            "    op {:#06x} ({b0:#04x}, {b1:#04x}, {b2:#04x}, {b3:#04x}, {b4:#04x}, {b5:#04x}, {b6:#04x}, {b7:#04x}) -> true: {on_true}, false: {on_false};",
            ins.opcode
        );
    }
    out.push_str("}\n");
    out
}

fn format_bhav_header(chunk: &BhavChunk) -> String {
    let mut out = String::new();
    let escaped_label = escape_antics_string(&chunk.label);
    let _ = write!(
        out,
        "tree {} \"{}\" (format: {:#06x}, kind: {}, args: {}, locals: {}, revision: {:#010x}",
        chunk.id, escaped_label, chunk.format, chunk.kind, chunk.args, chunk.locals, chunk.revision
    );
    if !chunk.trailing.is_empty() {
        out.push_str(", trailing: [");
        for (i, b) in chunk.trailing.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let _ = write!(out, "{b:#04x}");
        }
        out.push(']');
    }
    out.push_str(") {\n");
    out
}

fn format_target(target: u8, count: usize) -> String {
    match target {
        0xFD => "error".to_owned(),
        0xFE => "true".to_owned(),
        0xFF => "false".to_owned(),
        n if n <= 250 && usize::from(n) < count => format!("@n{n}"),
        n => format!("raw({n:#04x})"),
    }
}

fn escape_antics_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    let _ = write!(out, "\\x{b:02x}");
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Lifts a raw `BCON` chunk payload into a [`BconChunk`] IR.
pub fn lift_bcon(id: u16, label: &str, data: &[u8]) -> Result<BconChunk> {
    let values = sims_formats::execution::constants(data)?;
    let flags = *data
        .get(1)
        .ok_or_else(|| Error::new("BCON chunk missing flags byte"))?;
    let consumed = 2 + values.len() * 2;
    let trailing = data.get(consumed..).unwrap_or(&[]).to_vec();
    Ok(BconChunk {
        id,
        label: label.to_owned(),
        flags,
        values,
        trailing,
    })
}

/// Formats a [`BconChunk`] into `.antics` `bcon` syntax.
#[must_use]
pub fn format_bcon_antics(chunk: &BconChunk) -> String {
    let vals: Vec<String> = chunk.values.iter().map(ToString::to_string).collect();
    format!(
        "bcon {} \"{}\" (flags: {}) = [{}];\n",
        chunk.id,
        escape_antics_string(&chunk.label),
        chunk.flags,
        vals.join(", ")
    )
}

/// Lifts a raw `OBJD` chunk payload into an [`ObjdChunk`] IR.
pub fn lift_objd(id: u16, label: &str, data: &[u8]) -> Result<ObjdChunk> {
    let parsed = sims_formats::behavior::ObjectDefinition::parse(data)?;
    let word_count = data.len() / 2;
    let mut words = Vec::with_capacity(word_count);
    for pair in data[..word_count * 2].as_chunks::<2>().0 {
        words.push(u16::from_le_bytes(*pair));
    }
    let trailing_byte = if data.len() % 2 == 1 {
        Some(*data.last().unwrap())
    } else {
        None
    };
    Ok(ObjdChunk {
        id,
        label: label.to_owned(),
        version: parsed.version,
        words,
        trailing_byte,
    })
}

/// Lifts a raw `OBJf` chunk payload into an [`ObjfChunk`] IR.
pub fn lift_objf(id: u16, label: &str, data: &[u8]) -> Result<ObjfChunk> {
    let parsed = ObjectFunctions::parse(data)?;
    let reserved = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let consumed = 16 + parsed.entries.len() * 4;
    let trailing = data.get(consumed..).unwrap_or(&[]).to_vec();
    Ok(ObjfChunk {
        id,
        label: label.to_owned(),
        reserved,
        version: parsed.version,
        entries: parsed.entries,
        trailing,
    })
}

/// Lifts a raw `STR#`, `CTSS`, or `TTAs` chunk payload into a [`StringTableChunk`] IR,
/// preserving exact format, raw language bytes, and raw encoded text/comment bytes.
pub fn lift_string_table(
    tag: [u8; 4],
    id: u16,
    label: &str,
    data: &[u8],
) -> Result<StringTableChunk> {
    let parsed = StringTable::parse(data)?;
    let format = match parsed.format {
        Format::Pascal => StringFormat::Pascal,
        Format::Terminated => StringFormat::Terminated,
        Format::Commented => StringFormat::Commented,
        Format::Localized => StringFormat::Localized,
        Format::MacResource => StringFormat::MacResource,
    };

    let version_only_empty = format != StringFormat::MacResource && data.len() == 2;
    // Walk exact byte offsets to capture raw language bytes (e.g. 0 vs 1) and trailing bytes.
    let mut pos = match format {
        StringFormat::MacResource => 2usize,
        _ if version_only_empty => 2usize,
        _ => 4usize,
    };
    let mut entries = Vec::with_capacity(parsed.entries.len());
    for entry in &parsed.entries {
        let raw_lang = if format == StringFormat::Localized {
            let b = *data
                .get(pos)
                .ok_or_else(|| Error::new("truncated localized STR# language byte"))?;
            pos += 1;
            b
        } else {
            entry.language
        };
        match format {
            StringFormat::Pascal | StringFormat::MacResource => {
                pos += 1 + entry.raw_text.len();
            }
            StringFormat::Terminated => {
                pos += entry.raw_text.len() + 1;
            }
            StringFormat::Commented | StringFormat::Localized => {
                pos += entry.raw_text.len() + 1 + entry.raw_comment.len() + 1;
            }
        }
        entries.push(StringEntry {
            language: raw_lang,
            text: entry.text.clone(),
            comment: entry.comment.clone(),
            raw_text: Some(entry.raw_text.clone()),
            raw_comment: Some(entry.raw_comment.clone()),
        });
    }
    let trailing = data.get(pos..).unwrap_or(&[]).to_vec();

    Ok(StringTableChunk {
        tag,
        id,
        label: label.to_owned(),
        format,
        version_only_empty,
        entries,
        trailing,
    })
}

/// Lifts a raw `SLOT` chunk payload into a [`SlotChunk`] IR.
pub fn lift_slot(id: u16, label: &str, data: &[u8]) -> Result<SlotChunk> {
    let parsed = Slots::parse(data)?;
    let reserved = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let stride = match parsed.version {
        4 | 5 => 34usize,
        6 => 54,
        7 => 58,
        8 => 62,
        9 => 66,
        10 => 70,
        v => return Err(Error::new(format!("unsupported SLOT version {v}"))),
    };
    let consumed = 16 + parsed.entries.len() * stride;
    let trailing = data.get(consumed..).unwrap_or(&[]).to_vec();
    Ok(SlotChunk {
        id,
        label: label.to_owned(),
        reserved,
        version: parsed.version,
        entries: parsed.entries,
        trailing,
    })
}

/// Lifts a raw `TTAB` chunk payload into a [`TtabChunk`] IR, capturing exact
/// bit-width hints for compressed v9/v10 tables so re-encoding is byte-for-byte identical.
pub fn lift_ttab(id: u16, label: &str, data: &[u8]) -> Result<TtabChunk> {
    // Validate with sims_formats::behavior::InteractionTable first.
    let parsed = InteractionTable::parse(data)?;
    if parsed.interactions.is_empty() {
        let trailing = data.get(4..).unwrap_or(&[]).to_vec();
        return Ok(TtabChunk {
            id,
            label: label.to_owned(),
            version: parsed.version,
            compressed: false,
            compression_byte: 0,
            interactions: Vec::new(),
            compressed_width_hints: Vec::new(),
            trailing_bits: 0,
            trailing,
        });
    }

    let version = parsed.version;
    let compression_byte = if version >= 9 { data[4] } else { 0 };
    let compressed = version >= 9 && compression_byte == 1;
    let payload_start = if version >= 9 { 5usize } else { 4usize };

    if !compressed {
        // Compute exact consumed byte count for uncompressed stream.
        let mut pos = payload_start;
        for inter in &parsed.interactions {
            pos += 2 + 2 + 4 + 4 + 4; // action, test, motives_len, flags, string_index
            if version > 6 {
                pos += 4; // attenuation_code
            }
            pos += 4 + 4 + 4; // attenuation, autonomy, joining_index
            let motive_stride = if version > 6 { 6 } else { 2 };
            pos += inter.motives.len() * motive_stride;
            if version > 9 {
                pos += 4; // flags2
            }
        }
        let trailing = data.get(pos..).unwrap_or(&[]).to_vec();
        return Ok(TtabChunk {
            id,
            label: label.to_owned(),
            version,
            compressed: false,
            compression_byte,
            interactions: parsed.interactions,
            compressed_width_hints: Vec::new(),
            trailing_bits: 0,
            trailing,
        });
    }

    // Walk the compressed bitstream to record exact bit-width codes and trailing bits/bytes.
    let stream = &data[payload_start..];
    let mut br = BitInspector {
        data: stream,
        bit: 0,
        hints: Vec::new(),
    };
    let mut interactions = Vec::with_capacity(parsed.interactions.len());
    for _ in 0..parsed.interactions.len() {
        let action = br.read(16)? as u16;
        let test = br.read(16)? as u16;
        let motives_len = br.read(32)? as usize;
        let flags = br.read(32)?;
        let string_index = br.read(32)?;
        let attenuation_code = if version > 6 { br.read(32)? } else { 0 };
        let attenuation = f32::from_bits(br.read(32)?);
        let autonomy = br.read(32)?;
        let joining_index = br.read(32)? as i32;
        let mut motives = Vec::with_capacity(motives_len);
        for _ in 0..motives_len {
            let minimum = if version > 6 { br.read(16)? as i16 } else { 0 };
            let delta = br.read(16)? as i16;
            let personality = if version > 6 { br.read(16)? as u16 } else { 0 };
            motives.push(MotiveAdvertisement {
                minimum,
                delta,
                personality,
            });
        }
        let flags2 = if version > 9 { br.read(32)? } else { 0 };
        interactions.push(Interaction {
            action,
            test,
            string_index,
            flags,
            autonomy,
            attenuation_code,
            attenuation,
            joining_index,
            motives,
            flags2,
        });
    }
    let rem_bits = br.bit % 8;
    let trailing_bits = if rem_bits != 0 {
        let remaining = 8 - rem_bits;
        let byte = stream[br.bit / 8];
        byte & ((1u8 << remaining) - 1)
    } else {
        0
    };
    let consumed_bytes = br.bit.div_ceil(8);
    let trailing = stream.get(consumed_bytes..).unwrap_or(&[]).to_vec();

    Ok(TtabChunk {
        id,
        label: label.to_owned(),
        version,
        compressed: true,
        compression_byte,
        interactions,
        compressed_width_hints: br.hints,
        trailing_bits,
        trailing,
    })
}

struct BitInspector<'a> {
    data: &'a [u8],
    bit: usize,
    hints: Vec<u8>,
}

impl BitInspector<'_> {
    fn bits(&mut self, n: usize) -> Result<u32> {
        if n > self.data.len().saturating_mul(8).saturating_sub(self.bit) {
            return Err(Error::new("truncated compressed TTAB bitstream"));
        }
        let mut value = 0u32;
        for _ in 0..n {
            value = (value << 1) | u32::from((self.data[self.bit / 8] >> (7 - self.bit % 8)) & 1);
            self.bit += 1;
        }
        Ok(value)
    }

    fn read(&mut self, size: usize) -> Result<u32> {
        if self.bits(1)? == 0 {
            self.hints.push(255);
            return Ok(0);
        }
        let code = self.bits(2)? as usize;
        self.hints.push(code as u8);
        let width = if size == 16 {
            [5, 8, 13, 16][code]
        } else {
            [6, 11, 21, 32][code]
        };
        let value = self.bits(width)?;
        let signed = if width < 32 && value & (1 << (width - 1)) != 0 {
            value | (!0u32 << width)
        } else {
            value
        };
        Ok(if size == 16 { signed & 0xFFFF } else { signed })
    }
}
