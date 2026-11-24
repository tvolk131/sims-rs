//! In-memory structured lifter from raw `BHAV` bytecode into structured
//! `.air` (Antics IR) source text (`if`/`else`, `while`, `return`, named
//! `Expression` assignments/comparisons, readable primitives, `call`, and
//! per-instruction inline `op 0xNNNN (...)` fallbacks for unmodeled opcodes
//! or dirty Edith padding bytes).
//!
//! Strictly in-memory: never writes any lifted representation to disk.

use crate::ast::OperandPad;
use crate::ir::BhavChunk;
use crate::lower::{TARGET_ERROR, TARGET_FALSE, TARGET_TRUE};
use crate::verify::{canonicalize_primitive_operands, is_always_true_instruction, operand_pad_for};
use sims_formats::behavior::Instruction;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;

/// Configuration for lifting raw `BHAV` bytecode into `.air` (Antics IR).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiftOptions {
    /// When `true` (default), only decode an instruction into a named `.air`
    /// primitive when unused/reserved operand bytes are zero; otherwise fall
    /// back to an inline `op 0xNNNN (...)` for that single instruction so no
    /// operand bit is silently dropped.
    ///
    /// When `false` (`--relax-padding`), verified-ignored padding bytes are
    /// stripped during lifting while preserving bounded trace equivalence.
    pub strict_padding: bool,
    /// When `true`, emit trailing `// ...` readability comments annotating
    /// canonical VM field names (`my.motive[N]`, `my.person[N]`, `my.data[N]`),
    /// sibling private BHAV call labels, local `BCON` values/labels, and local
    /// `STR#` previews (when [`LiftSymbols`] is provided).
    ///
    /// Defaults to `false` so canonical programmatic callers stay noise-free;
    /// `sims-antics lift` enables it by default unless `--no-comments` is passed.
    pub annotate_comments: bool,
}

impl Default for LiftOptions {
    fn default() -> Self {
        Self {
            strict_padding: true,
            annotate_comments: false,
        }
    }
}

/// Optional in-memory `.iff` symbol tables used when `LiftOptions::annotate_comments` is enabled.
///
/// Strictly in-memory: never written to disk by library functions.
#[derive(Debug, Clone, Default)]
pub struct LiftSymbols {
    /// Private `BHAV` chunk labels (`4096..=8191`) from the same `.iff`.
    pub bhav_labels: BTreeMap<u16, String>,
    /// Local `BCON` tables (`4096..=4159`): `chunk_id -> (label, values)`.
    pub bcons: BTreeMap<u16, (String, Vec<i16>)>,
    /// Local `STR#` tables: `chunk_id -> entry texts` (0-indexed; SimAntics uses 1-based indices).
    pub strings: BTreeMap<u16, Vec<String>>,
}

impl LiftSymbols {
    /// The symbols that apply to calls made by tree `caller`. A private call runs the
    /// code owner's tree; only a private caller's owner is known to be this file, so a
    /// global or semi-global caller gets no private-callee labels.
    #[must_use]
    pub fn for_caller(&self, caller: u16) -> std::borrow::Cow<'_, Self> {
        if (4096..=8191).contains(&caller) || self.bhav_labels.is_empty() {
            std::borrow::Cow::Borrowed(self)
        } else {
            std::borrow::Cow::Owned(Self {
                bhav_labels: BTreeMap::new(),
                ..self.clone()
            })
        }
    }
}

fn collect_canonical_chunks<'a>(
    iff: &'a sims_formats::iff::Iff,
    tag: &'a [u8; 4],
) -> BTreeMap<u16, &'a sims_formats::iff::Chunk> {
    use sims_formats::iff::DELETED;

    let mut by_id: BTreeMap<u16, &'a sims_formats::iff::Chunk> = BTreeMap::new();
    for chunk in iff.all(tag).filter(|c| c.flags & DELETED == 0) {
        by_id
            .entry(chunk.id)
            .and_modify(|existing| {
                if (existing.flags >> 8) != 0 && (chunk.flags >> 8) == 0 {
                    *existing = chunk;
                }
            })
            .or_insert(chunk);
    }
    by_id
}

/// Extracts US English strings in the exact order the original Sims 1 loader
/// (`sims-vm/src/program.rs::loaded_order`) indexes them: each run of one language
/// restarts its insertion slot at 0, so a later English run is inserted in front of
/// earlier runs. Non-localized formats decode every entry with `language == 1` in
/// file order, so this helper handles all `STR#` formats uniformly and returns an
/// empty list for localized tables with no English entries (matching VM behavior).
fn loaded_str_order(entries: &[sims_formats::strings::TextEntry]) -> Vec<String> {
    let mut loaded: Vec<String> = Vec::new();
    let mut run = None;
    let mut place = 0usize;
    for entry in entries {
        if run != Some(entry.language) {
            run = Some(entry.language);
            place = 0;
        }
        if entry.language == 1 {
            loaded.insert(place.min(loaded.len()), entry.text.clone());
            place += 1;
        }
    }
    loaded
}

/// Extracts strictly in-memory [`LiftSymbols`] (private `BHAV` labels, local `BCON` tables,
/// and local `STR#` tables) from a parsed [`sims_formats::iff::Iff`]. A `BHAV` label is the
/// one on the chunk the VM keeps for that ID
/// ([`crate::arity_catalog::vm_kept_bhavs`]); `BCON` and `STR#` prefer canonical
/// high-flag-byte `0` chunks and skip deleted chunks.
#[must_use]
pub fn extract_lift_symbols_from_iff(iff: &sims_formats::iff::Iff) -> LiftSymbols {
    let mut symbols = LiftSymbols::default();

    for chunk in crate::arity_catalog::vm_kept_bhavs(iff).chunks {
        if (4096..=8191).contains(&chunk.id) && !chunk.label.is_empty() {
            symbols.bhav_labels.insert(chunk.id, chunk.label.clone());
        }
    }
    for (id, chunk) in collect_canonical_chunks(iff, b"BCON") {
        if (4096..=4159).contains(&id)
            && let Ok(values) = sims_formats::execution::constants(iff.data(chunk))
        {
            symbols.bcons.insert(id, (chunk.label.clone(), values));
        }
    }
    for (id, chunk) in collect_canonical_chunks(iff, b"STR#") {
        let data = iff.data(chunk);
        let parsed = sims_formats::strings::StringTable::parse(data).or_else(|_| {
            sims_formats::strings::StringTable::parse_chunk(data, iff.resource_flags(chunk))
        });
        if let Ok(table) = parsed {
            let texts = loaded_str_order(&table.entries);
            if !texts.is_empty() {
                symbols.strings.insert(id, texts);
            }
        }
    }

    symbols
}

const MAX_COMMENT_PREVIEW_CHARS: usize = 48;

fn sanitize_comment_preview(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut truncated = false;
    for (count, ch) in s.chars().enumerate() {
        if count >= max_chars {
            truncated = true;
            break;
        }
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            '"' => out.push_str("\\\""),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    if truncated {
        out.push_str("...");
    }
    out
}

fn canonical_motive_name(index: i16) -> Option<&'static str> {
    match index {
        3 => Some("Mood"),
        5 => Some("Energy"),
        6 => Some("Comfort"),
        7 => Some("Hunger"),
        8 => Some("Hygiene"),
        9 => Some("Bladder"),
        11 => Some("Alertness"),
        13 => Some("Room"),
        14 => Some("Social"),
        15 => Some("Fun"),
        _ => None,
    }
}

fn canonical_person_field_name(index: i16) -> Option<&'static str> {
    match index {
        0 => Some("Posture"),
        2 => Some("Nice"),
        3 => Some("Active"),
        5 => Some("Playful"),
        6 => Some("Outgoing"),
        7 => Some("Neat"),
        8 => Some("BodySuit"),
        10 => Some("Cooking"),
        11 => Some("Charisma"),
        12 => Some("Mechanical"),
        15 => Some("Creativity"),
        17 => Some("Body"),
        18 => Some("Logic"),
        21 => Some("SuitVariant"),
        22 => Some("LeftHand"),
        23 => Some("RightHand"),
        30 => Some("Censor"),
        31 => Some("NeighborId"),
        32 => Some("IsVisitor"),
        33 => Some("Priority"),
        34 => Some("PetsOutside"),
        36 => Some("Autonomy"),
        41 => Some("HeadTarget"),
        56 => Some("JobType"),
        57 => Some("JobLevel"),
        58 => Some("Age"),
        60 => Some("SkinColor"),
        61 => Some("FamilyId"),
        65 => Some("PersonKind"),
        67 => Some("HomeLot"),
        70 => Some("Zodiac"),
        77 => Some("DateFlag"),
        81 => Some("FameLevel"),
        _ => None,
    }
}

fn canonical_object_data_name(index: i16) -> Option<&'static str> {
    match index {
        0 => Some("Graphic"),
        1 => Some("Direction"),
        2 => Some("ContainerId"),
        3 => Some("SlotNumber"),
        8 => Some("Flags"),
        11 => Some("ObjectId"),
        20 => Some("ChosenAction"),
        25 => Some("Lockout"),
        29 => Some("Room"),
        31 => Some("PrepScore"),
        32 => Some("CookScore"),
        40 => Some("Flags2"),
        59 => Some("Category"),
        _ => None,
    }
}

fn push_unique_note(notes: &mut Vec<String>, note: String) {
    if !note.is_empty() && !notes.contains(&note) {
        notes.push(note);
    }
}

fn annotate_operand_note(
    scope: u8,
    index: i16,
    symbols: Option<&LiftSymbols>,
    notes: &mut Vec<String>,
) {
    match scope {
        14 | 15 => {
            if let Some(name) = canonical_motive_name(index) {
                push_unique_note(notes, name.to_owned());
            }
        }
        18 | 19 => {
            if let Some(name) = canonical_person_field_name(index) {
                push_unique_note(notes, name.to_owned());
            }
        }
        3 | 4 => {
            if let Some(name) = canonical_object_data_name(index) {
                push_unique_note(notes, name.to_owned());
            }
        }
        26 => {
            let Some(syms) = symbols else {
                return;
            };
            let raw = index as u16;
            let table = raw >> 7;
            let entry = usize::from(raw & 127);
            if table < 64
                && let Some((label, values)) = syms.bcons.get(&(4096 + table))
                && let Some(&val) = values.get(entry)
            {
                let note = if label.is_empty() {
                    format!("bcon[{table}:{entry}] = {val}")
                } else {
                    format!(
                        "bcon[{table}:{entry}] = {val} (\"{}\")",
                        sanitize_comment_preview(label, 32)
                    )
                };
                push_unique_note(notes, note);
            }
        }
        _ => {}
    }
}

fn lookup_str_preview(symbols: &LiftSymbols, table: u16, one_based: usize) -> Option<String> {
    let idx = one_based.checked_sub(1)?;
    let text = symbols.strings.get(&table)?.get(idx)?;
    if text.is_empty() {
        return None;
    }
    Some(format!(
        "\"{}\"",
        sanitize_comment_preview(text, MAX_COMMENT_PREVIEW_CHARS)
    ))
}

/// Builds an optional single-line trailing comment (without leading `// `) for `ins`.
pub(crate) fn instruction_comment(
    ins: &Instruction,
    symbols: Option<&LiftSymbols>,
) -> Option<String> {
    let ops = &ins.operands;
    let mut notes = Vec::new();
    match ins.opcode {
        2 => {
            annotate_operand_note(ops[6], word_i16(ops, 0), symbols, &mut notes);
            annotate_operand_note(ops[7], word_i16(ops, 2), symbols, &mut notes);
        }
        8 => {
            let dst_scope = word_u16(ops, 2);
            if dst_scope <= 255 {
                annotate_operand_note(dst_scope as u8, word_i16(ops, 0), symbols, &mut notes);
            }
            let rng_scope = word_u16(ops, 6);
            if rng_scope <= 255 {
                annotate_operand_note(rng_scope as u8, word_i16(ops, 4), symbols, &mut notes);
            }
        }
        11 => {
            if ops[2] & 1 != 0 {
                annotate_operand_note(ops[3], word_i16(ops, 4), symbols, &mut notes);
            }
        }
        12 => {
            let dst_scope = word_u16(ops, 2);
            if dst_scope <= 255 {
                annotate_operand_note(dst_scope as u8, word_i16(ops, 0), symbols, &mut notes);
            }
            if ops[4] & 1 != 0 {
                annotate_operand_note(ops[5], word_i16(ops, 6), symbols, &mut notes);
            }
        }
        14 => {
            if ops[6] & 8 != 0 {
                let scope = word_u16(ops, 4);
                if scope <= 255 {
                    annotate_operand_note(scope as u8, word_i16(ops, 2), symbols, &mut notes);
                }
            }
        }
        15 => {
            let scope = word_i16(ops, 2);
            if (0..=255).contains(&scope) {
                annotate_operand_note(scope as u8, word_i16(ops, 0), symbols, &mut notes);
            }
        }
        21 => {
            if let Some(syms) = symbols
                && let Some(preview) =
                    lookup_str_preview(syms, word_u16(ops, 0), usize::from(word_u16(ops, 2)))
            {
                push_unique_note(&mut notes, preview);
            }
        }
        25 => {
            let scope = match ops[0] {
                0 => 7,
                1 => 9,
                2 => 25,
                _ => ops[1],
            };
            annotate_operand_note(scope, word_i16(ops, 2), symbols, &mut notes);
        }
        26 => {
            annotate_operand_note(ops[4], word_i16(ops, 6), symbols, &mut notes);
        }
        28 => {
            if ops[2] & 1 == 0
                && let Some(syms) = symbols
                && let Some(preview) =
                    lookup_str_preview(syms, word_u16(ops, 0), usize::from(ops[4]))
            {
                push_unique_note(&mut notes, preview);
            }
        }
        29 => {
            if ops[3] & 1 == 0 {
                if let Some(name) = canonical_motive_name(i16::from(ops[2])) {
                    push_unique_note(&mut notes, name.to_owned());
                }
                annotate_operand_note(ops[0], word_i16(ops, 4), symbols, &mut notes);
                annotate_operand_note(ops[1], word_i16(ops, 6), symbols, &mut notes);
            }
        }
        31 => {
            if ops[4] & 0x80 != 0 {
                annotate_operand_note(ops[5], i16::from(ops[7]), symbols, &mut notes);
            }
        }
        32 => {
            annotate_operand_note(ops[6], word_i16(ops, 4), symbols, &mut notes);
        }
        34 => {
            if ops[0] <= 2 {
                annotate_operand_note(ops[1], word_i16(ops, 2), symbols, &mut notes);
            }
        }
        36 => {
            if let Some(syms) = symbols {
                if let Some(preview) = lookup_str_preview(syms, 301, usize::from(ops[2])) {
                    push_unique_note(&mut notes, preview);
                } else if let Some(preview) = lookup_str_preview(syms, 301, usize::from(ops[6])) {
                    push_unique_note(&mut notes, preview);
                }
            }
        }
        37 => {
            if ops[0] == 1 {
                annotate_operand_note(ops[1], word_i16(ops, 2), symbols, &mut notes);
            }
        }
        50 => {
            if let Some(syms) = symbols
                && let Some(preview) =
                    lookup_str_preview(syms, word_u16(ops, 0), usize::from(ops[4]))
            {
                push_unique_note(&mut notes, preview);
            }
        }
        id if (4096..=8191).contains(&id) => {
            if let Some(syms) = symbols
                && let Some(label) = syms.bhav_labels.get(&id)
                && !label.is_empty()
            {
                push_unique_note(
                    &mut notes,
                    format!("\"{}\"", sanitize_comment_preview(label, 48)),
                );
            }
        }
        _ => {}
    }
    if notes.is_empty() {
        None
    } else {
        Some(notes.join(", "))
    }
}

/// Result of attempting to lift a [`BhavChunk`] into structured `.air` (Antics IR).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredLiftOutcome {
    /// Cleanly lifted into structured `.air` (`if`/`else`/`while`, with per-instruction
    /// inline `op 0xNNNN (...)` only where an opcode is unmodeled or padding is dirty).
    Lifted(String),
    /// Reserved for callers matching all variants (no longer triggered for individual
    /// unmodeled primitives now that inline `op 0xNNNN (...)` is supported).
    UnsupportedOpcode(u16),
    /// Tree uses an unsupported BHAV format (e.g., `0x8003`), `kind == 4` indexed multi-entry,
    /// `> 251` instructions, or an out-of-bounds target.
    UnsupportedHeaderOrTarget,
    /// Tree's control flow graph is irreducible or exceeds the structured region
    /// duplication budget.
    Unstructured,
}

fn word_i16(ops: &[u8; 8], offset: usize) -> i16 {
    i16::from_le_bytes([ops[offset], ops[offset + 1]])
}

fn word_u16(ops: &[u8; 8], offset: usize) -> u16 {
    u16::from_le_bytes([ops[offset], ops[offset + 1]])
}

fn format_limits(format: u16) -> Option<(usize, usize)> {
    match format {
        0x8000 => Some((4, 0)),
        0x8001 => Some((4, 4)),
        0x8002 => Some((255, 255)),
        _ => None,
    }
}

/// Formats a SimAntics `(scope, index)` operand into high-level `.antics` syntax.
fn format_operand(scope: u8, index: i16, max_args: usize, max_locals: usize) -> Option<String> {
    match scope {
        0 => Some(format!("my.attr[{index}]")),
        1 => Some(format!("stack.attr[{index}]")),
        3 => Some(format!("my.data[{index}]")),
        4 => Some(format!("stack.data[{index}]")),
        6 => Some(format!("global[{index}]")),
        7 => Some(index.to_string()),
        8 => Some(format!("temp[{index}]")),
        9 => {
            if index < 0 || index as usize >= max_args {
                return None;
            }
            Some(format!("param[{index}]"))
        }
        10 if index == 0 => Some("stack_obj".to_owned()),
        13 => Some(format!("stack.temp[{index}]")),
        14 => Some(format!("my.motive[{index}]")),
        15 => Some(format!("stack.motive[{index}]")),
        16 => Some(format!("stack.slot[{index}]")),
        18 => Some(format!("my.person[{index}]")),
        19 => Some(format!("stack.person[{index}]")),
        20 => Some(format!("my.slot[{index}]")),
        22 => {
            if index < 0 || index as usize >= max_args {
                return None;
            }
            Some(format!("scope(22, {index})"))
        }
        25 => {
            if index < 0 || index as usize >= max_locals {
                return None;
            }
            Some(format!("local[{index}]"))
        }
        26 => Some(format!("bcon[{index}]")),
        other => Some(format!("scope({other}, {index})")),
    }
}

/// Lifted high-level representation of a single SimAntics instruction.
#[derive(Debug, Clone)]
enum LiftedIns {
    /// Unconditional linear statement (always takes `on_true` in `sims-vm`).
    Stmt {
        text: String,
        comment: Option<String>,
        next: u8,
    },
    /// Branching high-level condition (used in `if` / `while`).
    Cond {
        /// Condition text (e.g. `my.attr[0] == 0`, `call 4098(0, 0, 0, 0)`, `play_sound(...)`).
        text: String,
        /// Optional statement form when `on_true == on_false` (or `call` with `on_false == 0xFF`).
        stmt_form: Option<String>,
        /// Optional trailing readability comment (when `LiftOptions::annotate_comments` is enabled).
        comment: Option<String>,
        /// True when `stmt_form` is a `call` (whose standalone `Stmt::Call` branches `on_false = 0xFF`).
        is_call: bool,
        on_true: u8,
        on_false: u8,
    },
}

/// Decoded representation of one instruction shared between structured lifting
/// and decoded labeled-CFG formatting.
enum DecodedIns {
    /// Unconditional linear statement (`is_always_true_instruction` is true).
    AlwaysTrue {
        /// Text for structured mode (includes trailing `;`).
        structured_stmt: String,
        /// Text for labeled-CFG mode (precedes ` -> true: ..., false: ...;`).
        labeled_head: String,
    },
    /// Branching instruction (comparison, branching primitive, or subroutine call).
    Branching {
        /// Condition text for `if`/`while` (also used as labeled-CFG head except for `Expression`).
        cond_text: String,
        /// Optional statement form when `on_true == on_false` (or `call` with `on_false == 0xFF`).
        stmt_form: Option<String>,
        /// Text for labeled-CFG mode (precedes ` -> true: ..., false: ...;`).
        labeled_head: String,
        /// True when `stmt_form` is a `call` (whose standalone `Stmt::Call` branches `on_false = 0xFF`).
        is_call: bool,
    },
}

impl DecodedIns {
    /// A linear primitive written `head;` in every mode.
    fn always_true(head: String) -> Self {
        Self::AlwaysTrue {
            structured_stmt: format!("{head};"),
            labeled_head: head,
        }
    }

    /// A branching (non-call) primitive whose text is the same as statement, condition
    /// and labeled head.
    fn branching(expr: String) -> Self {
        Self::Branching {
            stmt_form: Some(format!("{expr};")),
            labeled_head: expr.clone(),
            cond_text: expr,
            is_call: false,
        }
    }

    /// The same primitive with a `pad` clause. The labeled head is always the
    /// primitive-call spelling (`expr(...)` for an Expression), which takes the clause.
    fn padded(self, pad: &OperandPad) -> Self {
        let pad = format_operand_pad(pad);
        match self {
            Self::AlwaysTrue { labeled_head, .. } => {
                Self::always_true(format!("{labeled_head} {pad}"))
            }
            Self::Branching {
                labeled_head,
                stmt_form,
                is_call,
                ..
            } => {
                let head = format!("{labeled_head} {pad}");
                Self::Branching {
                    stmt_form: stmt_form.map(|_| format!("{head};")),
                    labeled_head: head.clone(),
                    cond_text: head,
                    is_call,
                }
            }
        }
    }
}

fn format_raw_op(ins: &Instruction) -> String {
    let [b0, b1, b2, b3, b4, b5, b6, b7] = ins.operands;
    format!(
        "op {:#06x} ({b0:#04x}, {b1:#04x}, {b2:#04x}, {b3:#04x}, {b4:#04x}, {b5:#04x}, {b6:#04x}, {b7:#04x})",
        ins.opcode
    )
}

fn try_decode_instruction(
    ins: &Instruction,
    max_args: usize,
    max_locals: usize,
    opts: LiftOptions,
) -> Option<DecodedIns> {
    if let Some(decoded) = decode_named(ins, max_args, max_locals, opts) {
        return Some(decoded);
    }
    // Strict lifting spells bytes the VM ignores with a `pad` clause on the named form
    // of the canonical operands: `apply_operand_pad` (which the compiler runs) rebuilds
    // exactly these bytes, and only when they read the same as the canonical ones.
    if !opts.strict_padding || ins.opcode >= 256 {
        return None;
    }
    let canon = canonicalize_primitive_operands(ins.opcode, ins.operands);
    let pad = operand_pad_for(ins.opcode, canon, ins.operands)?;
    let named = Instruction {
        operands: canon,
        ..ins.clone()
    };
    Some(decode_named(&named, max_args, max_locals, opts)?.padded(&pad))
}

fn format_operand_pad(pad: &OperandPad) -> String {
    match pad {
        OperandPad::Fill(value) => format!("pad {value:#04x}"),
        OperandPad::Bytes(bytes) => {
            let list: Vec<String> = bytes
                .iter()
                .map(|(index, value)| format!("{index}: {value:#04x}"))
                .collect();
            format!("pad({})", list.join(", "))
        }
    }
}

fn decode_named(
    ins: &Instruction,
    max_args: usize,
    max_locals: usize,
    opts: LiftOptions,
) -> Option<DecodedIns> {
    let opcode = ins.opcode;
    let strict = opts.strict_padding;
    // Strict lifting keeps every operand bit, rejecting bytes `.air` cannot spell. Relaxed
    // lifting first drops the bits the VM ignores, with the same normalization the
    // equivalence checker uses, so the arms below never mask bits themselves.
    let ops = &if strict {
        ins.operands
    } else {
        canonicalize_primitive_operands(opcode, ins.operands)
    };
    match opcode {
        0 => {
            if strict && ops[2..8] != [0; 6] {
                return None;
            }
            let p = word_i16(ops, 0);
            if p < 0 || p as usize >= max_args {
                return None;
            }
            let head = format!("sleep(param[{p}])");
            Some(DecodedIns::always_true(head))
        }
        1 => {
            if strict && ops[2..8] != [0; 6] {
                return None;
            }
            let routine = word_u16(ops, 0);
            let expr = format!("generic_call({routine})");
            Some(DecodedIns::branching(expr))
        }
        2 => {
            if strict && ops[4] != 0 {
                return None;
            }
            let lhs_idx = word_i16(ops, 0);
            let rhs_idx = word_i16(ops, 2);
            let op = ops[5];
            let lhs_scope = ops[6];
            let rhs_scope = ops[7];
            let lhs = format_operand(lhs_scope, lhs_idx, max_args, max_locals)?;
            let rhs = format_operand(rhs_scope, rhs_idx, max_args, max_locals)?;
            match op {
                3 | 4 | 5 | 6 | 7 | 9 | 10 | 12 | 13 | 18 | 19 | 20 => {
                    let sym = match op {
                        3 => "+=",
                        4 => "-=",
                        5 => ":=",
                        6 => "*=",
                        7 => "/=",
                        9 => "flag_set",
                        10 => "flag_clear",
                        12 => "%=",
                        13 => "&=",
                        18 => "|=",
                        19 => "^=",
                        20 => "sqrt",
                        _ => unreachable!(),
                    };
                    Some(DecodedIns::AlwaysTrue {
                        structured_stmt: format!("{lhs} {sym} {rhs};"),
                        labeled_head: format!("expr({lhs} {sym} {rhs})"),
                    })
                }
                0 | 1 | 2 | 8 | 11 | 14 | 15 | 16 | 17 => {
                    let sym = match op {
                        0 => ">",
                        1 => "<",
                        2 => "==",
                        8 => "flag_test",
                        11 => "inc_lt",
                        14 => ">=",
                        15 => "<=",
                        16 => "!=",
                        17 => "dec_gt",
                        _ => unreachable!(),
                    };
                    Some(DecodedIns::Branching {
                        cond_text: format!("{lhs} {sym} {rhs}"),
                        stmt_form: None,
                        labeled_head: format!("expr({lhs} {sym} {rhs})"),
                        is_call: false,
                    })
                }
                _ => None,
            }
        }
        3 => {
            if strict && *ops != [0; 8] {
                return None;
            }
            let expr = "find_best_interaction()".to_owned();
            Some(DecodedIns::branching(expr))
        }
        4 => {
            if strict && *ops != [0; 8] {
                return None;
            }
            let expr = "grab()".to_owned();
            Some(DecodedIns::branching(expr))
        }
        5 => {
            if strict && *ops != [0; 8] {
                return None;
            }
            let expr = "drop()".to_owned();
            Some(DecodedIns::branching(expr))
        }
        6 => {
            if strict && (ops[2] > 1 || ops[3..8] != [0; 5]) {
                return None;
            }
            let index = ops[0];
            let scope = ops[1];
            let remove = ops[2] & 1 != 0;
            let head = if remove {
                format!("change_suit(index: {index}, scope: {scope}, remove: true)")
            } else {
                format!("change_suit(index: {index}, scope: {scope})")
            };
            Some(DecodedIns::always_true(head))
        }
        7 => {
            if strict && ops[4..8] != [0; 4] {
                return None;
            }
            let target = word_u16(ops, 0);
            let mode = word_u16(ops, 2);
            let head = format!("refresh(target: {target}, mode: {mode})");
            Some(DecodedIns::always_true(head))
        }
        8 => {
            let dst_idx = word_i16(ops, 0);
            let dst_scope = word_u16(ops, 2);
            let rng_idx = word_i16(ops, 4);
            let rng_scope = word_u16(ops, 6);
            if dst_scope > 255 || rng_scope > 255 {
                return None;
            }
            let dst = format_operand(dst_scope as u8, dst_idx, max_args, max_locals)?;
            let rng = format_operand(rng_scope as u8, rng_idx, max_args, max_locals)?;
            let expr = format!("random({dst}, {rng})");
            Some(DecodedIns::branching(expr))
        }
        9 => {
            if strict && ops[2..8] != [0; 6] {
                return None;
            }
            let expr = format!("burn(mode: {}, flags: {})", ops[0], ops[1]);
            Some(DecodedIns::branching(expr))
        }
        10 => {
            if strict && ops[1..8] != [0; 7] {
                return None;
            }
            let expr = format!("tutorial({})", ops[0]);
            Some(DecodedIns::branching(expr))
        }
        11 => {
            let temp = word_i16(ops, 0);
            if !(0..=7).contains(&temp) {
                return None;
            }
            if strict && ops[6..8] != [0; 2] {
                return None;
            }
            let flags = ops[2];
            let head = if flags & 1 != 0 {
                let from = format_operand(ops[3], word_i16(ops, 4), max_args, max_locals)?;
                if flags == 1 {
                    format!("get_distance_to(temp[{temp}], from: {from})")
                } else {
                    format!("get_distance_to(temp[{temp}], flags: {flags}, from: {from})")
                }
            } else {
                if strict && ops[3..6] != [0; 3] {
                    return None;
                }
                if flags == 0 {
                    format!("get_distance_to(temp[{temp}])")
                } else {
                    format!("get_distance_to(temp[{temp}], flags: {flags})")
                }
            };
            Some(DecodedIns::always_true(head))
        }
        12 => {
            let dst_scope = word_u16(ops, 2);
            if dst_scope > 255 {
                return None;
            }
            let dst = format_operand(dst_scope as u8, word_i16(ops, 0), max_args, max_locals)?;
            let flags = ops[4];
            let head = if flags & 1 != 0 {
                let from = format_operand(ops[5], word_i16(ops, 6), max_args, max_locals)?;
                if flags == 1 {
                    format!("get_direction_to({dst}, from: {from})")
                } else {
                    format!("get_direction_to({dst}, flags: {flags}, from: {from})")
                }
            } else {
                if strict && ops[5..8] != [0; 3] {
                    return None;
                }
                if flags == 0 {
                    format!("get_direction_to({dst})")
                } else {
                    format!("get_direction_to({dst}, flags: {flags})")
                }
            };
            Some(DecodedIns::always_true(head))
        }
        13 => {
            if strict && ops[5..8] != [0; 3] {
                return None;
            }
            let menu = ops[0];
            let object_slot = ops[1];
            let priority = ops[2];
            let flags = ops[3];
            let icon = ops[4];
            if flags & 2 != 0 {
                if usize::from(object_slot) >= max_locals {
                    return None;
                }
            } else if usize::from(object_slot) >= max_args {
                return None;
            }
            if flags & 1 != 0 && usize::from(icon) >= max_locals {
                return None;
            }
            let mut expr = format!(
                "push_interaction(menu: {menu}, object: {object_slot}, priority: {priority}, flags: {flags}"
            );
            if icon != 0 {
                let _ = write!(expr, ", icon: {icon}");
            }
            expr.push(')');
            Some(DecodedIns::branching(expr))
        }
        14 => {
            if strict && ops[7] != 0 {
                return None;
            }
            let func = word_u16(ops, 0);
            let flags = ops[6];
            let expr = if flags & 8 != 0 {
                let origin_scope = word_u16(ops, 4);
                if origin_scope > 255 {
                    return None;
                }
                let origin =
                    format_operand(origin_scope as u8, word_i16(ops, 2), max_args, max_locals)?;
                format!("find_best_object({func}, flags: {flags}, origin: {origin})")
            } else {
                if strict && ops[2..6] != [0; 4] {
                    return None;
                }
                format!("find_best_object({func}, flags: {flags})")
            };
            Some(DecodedIns::branching(expr))
        }
        15 => {
            if strict && ops[4..8] != [0; 4] {
                return None;
            }
            let scope = word_i16(ops, 2);
            let index = word_i16(ops, 0);
            let head = if (0..=255).contains(&scope) {
                let cond = format_operand(scope as u8, index, max_args, max_locals)?;
                format!("breakpoint({cond})")
            } else {
                // The VM rejects the scope word before reading (`debug.rs`).
                format!(
                    "breakpoint(invalid_scope: {:#06x}, index: {index})",
                    scope as u16
                )
            };
            Some(DecodedIns::always_true(head))
        }
        16 => {
            if strict && ops[3..8] != [0; 5] {
                return None;
            }
            let mode = ops[0];
            let flags = ops[2];
            let local = ops[1];
            if flags & 1 != 0 && usize::from(local) >= max_locals {
                return None;
            }
            let mut expr = format!("find_location(mode: {mode}, flags: {flags}");
            if local != 0 {
                let _ = write!(expr, ", local: {local}");
            }
            expr.push(')');
            if mode == 1 {
                Some(DecodedIns::always_true(expr))
            } else {
                Some(DecodedIns::branching(expr))
            }
        }
        17 => {
            let p = word_i16(ops, 0);
            if p < 0 || p as usize >= max_args {
                return None;
            }
            if strict && ops[4..8] != [0; 4] {
                return None;
            }
            let allow_push = word_u16(ops, 2);
            let expr = format!("idle_for_input(param[{p}], {allow_push})");
            Some(DecodedIns::branching(expr))
        }
        18 => {
            if strict && ops[3..8] != [0; 5] {
                return None;
            }
            let target = word_u16(ops, 0);
            if target > 2 {
                return None;
            }
            let flags = ops[2];
            let expr = format!("remove_object({target}, flags: {flags})");
            Some(DecodedIns::branching(expr))
        }
        19 => {
            if strict && ops[4..8] != [0; 4] {
                return None;
            }
            let skin = ops[0];
            let age = ops[1];
            let sex = ops[2];
            let species = ops[3];
            if usize::from(skin) >= max_locals
                || usize::from(age) >= max_locals
                || usize::from(sex) >= max_locals
            {
                return None;
            }
            let expr = format!(
                "make_new_character(skin: {skin}, age: {age}, sex: {sex}, species: {species})"
            );
            Some(DecodedIns::branching(expr))
        }
        20 => {
            if strict && (ops[2] > 1 || ops[3..8] != [0; 5]) {
                return None;
            }
            let func = word_u16(ops, 0);
            let change_icon = ops[2] & 1 != 0;
            let expr = format!("run_functional_tree({func}, change_icon: {change_icon})");
            Some(DecodedIns::branching(expr))
        }
        21 => {
            if strict && ops[4..8] != [0; 4] {
                return None;
            }
            let table = word_u16(ops, 0);
            let index = word_u16(ops, 2);
            let head = format!("show_string(table: {table}, index: {index})");
            Some(DecodedIns::always_true(head))
        }
        22 => {
            if strict && ops[2..8] != [0; 6] {
                return None;
            }
            let mode = word_i16(ops, 0);
            let expr = format!("look_towards({mode})");
            if mode == 0 {
                Some(DecodedIns::always_true(expr))
            } else {
                Some(DecodedIns::branching(expr))
            }
        }
        23 => {
            if strict && (ops[2..4] != [0; 2] || ops[5..8] != [0; 3]) {
                return None;
            }
            let sound_id = word_u16(ops, 0);
            let head = match ops[4] {
                0 => format!("play_sound({sound_id})"),
                2 => format!("play_sound({sound_id}, stack_obj)"),
                f => format!("play_sound({sound_id}, flags: {f})"),
            };
            Some(DecodedIns::always_true(head))
        }
        24 => {
            if strict && (ops[0] > 1 || ops[5..8] != [0; 3]) {
                return None;
            }
            let write = ops[0] != 0;
            let col = ops[1];
            let param = ops[2] as i8;
            let dir = ops[3];
            let flags = ops[4];
            if param < 0 || (param as usize) >= max_args {
                return None;
            }
            if matches!(dir, 2 | 3) && max_args == 0 {
                return None;
            }
            let expr = format!(
                "old_relationship(write: {write}, column: {col}, param: {param}, dir: {dir}, flags: {flags})"
            );
            if flags & 1 == 0 {
                Some(DecodedIns::always_true(expr))
            } else {
                Some(DecodedIns::branching(expr))
            }
        }
        25 => {
            let scope = match ops[0] {
                0 => 7,
                1 => 9,
                2 => 25,
                _ => ops[1],
            };
            let expected_sel = match scope {
                7 => 0u8,
                9 => 1u8,
                25 => 2u8,
                _ => 3u8,
            };
            if strict
                && (ops[0] != expected_sel
                    || ops[1] != scope
                    || ops[4] > 3
                    || ops[5] != 0
                    || ops[7] != 0)
            {
                return None;
            }
            let idx = word_i16(ops, 2);
            let flags = ops[4] & 3;
            let category = ops[6];
            if category > 8 {
                return None;
            }
            let amount = format_operand(scope, idx, max_args, max_locals)?;
            let expr = format!("alter_budget({amount}, category: {category}, flags: {flags})");
            Some(DecodedIns::branching(expr))
        }
        26 => {
            if strict && ops[5] != 0 {
                return None;
            }
            let col = ops[0];
            let dir = ops[1];
            let flags = ops[2];
            let local = ops[3];
            if matches!(dir, 2 | 3) && usize::from(local) >= max_locals {
                return None;
            }
            let value = format_operand(ops[4], word_i16(ops, 6), max_args, max_locals)?;
            let mut expr =
                format!("relationship(column: {col}, dir: {dir}, flags: {flags}, value: {value}");
            if local != 0 {
                let _ = write!(expr, ", local: {local}");
            }
            expr.push(')');
            if flags & 1 == 0 {
                Some(DecodedIns::always_true(expr))
            } else {
                Some(DecodedIns::branching(expr))
            }
        }
        27 => {
            if strict && (ops[0] != 0 || ops[1] != 0 || ops[4] != 0 || ops[5] != 0 || ops[7] != 0) {
                return None;
            }
            let location = ops[2] as i8;
            let facing = ops[3];
            let options = ops[6];
            let expr = format!(
                "go_to_relative(location: {location}, facing: {facing}, options: {options})"
            );
            Some(DecodedIns::branching(expr))
        }
        28 => {
            if strict && (ops[2] > 1 || ops[3] != 0 || ops[6] != 0 || ops[7] != 0) {
                return None;
            }
            let table = word_u16(ops, 0);
            let global = ops[2] & 1 != 0;
            let index = ops[4];
            let mode = ops[5];
            let expr = if global {
                format!(
                    "run_tree_by_name(table: {table}, index: {index}, mode: {mode}, global: true)"
                )
            } else {
                format!("run_tree_by_name(table: {table}, index: {index}, mode: {mode})")
            };
            Some(DecodedIns::branching(expr))
        }
        29 => {
            if ops[3] & 1 != 0 {
                if strict && *ops != [0, 0, 0, 1, 0, 0, 0, 0] {
                    return None;
                }
                let head = "clear_motive_changes()".to_owned();
                Some(DecodedIns::always_true(head))
            } else {
                if strict && ops[3] != 0 {
                    return None;
                }
                let motive = ops[2];
                if !matches!(motive, 5..=9 | 14..=15) {
                    return None;
                }
                let delta = format_operand(ops[0], word_i16(ops, 4), max_args, max_locals)?;
                let stop = format_operand(ops[1], word_i16(ops, 6), max_args, max_locals)?;
                let head = format!("set_motive_change({motive}, {delta}, {stop})");
                Some(DecodedIns::always_true(head))
            }
        }
        30 => {
            if strict && *ops != [0; 8] {
                return None;
            }
            let expr = "gosub_found_action()".to_owned();
            Some(DecodedIns::branching(expr))
        }
        31 => {
            let guid = u32::from_le_bytes([ops[0], ops[1], ops[2], ops[3]]);
            let mode = ops[4];
            let base_mode = mode & 0x7F;
            let has_dest = mode & 0x80 != 0;
            let dest_scope = ops[5];
            let local_byte = ops[6];
            let dest_idx = ops[7];
            if base_mode == 6 && max_args == 0 {
                return None;
            }
            if base_mode == 9 && usize::from(local_byte) >= max_locals {
                return None;
            }
            if strict && !has_dest && (dest_scope != 0 || dest_idx != 0) {
                return None;
            }
            let mut expr = format!("set_to_next({base_mode}, guid: {guid:#010x}");
            if has_dest {
                if dest_scope == 7 {
                    return None;
                }
                let dest_str =
                    format_operand(dest_scope, i16::from(dest_idx), max_args, max_locals)?;
                let _ = write!(expr, ", dest: {dest_str}");
            }
            if local_byte != 0 {
                let _ = write!(expr, ", local: {local_byte}");
            }
            expr.push(')');
            Some(DecodedIns::branching(expr))
        }
        32 => {
            if strict && ops[7] != 0 {
                return None;
            }
            let guid = u32::from_le_bytes([ops[0], ops[1], ops[2], ops[3]]);
            let object = format_operand(ops[6], word_i16(ops, 4), max_args, max_locals)?;
            let expr = format!("test_object_type({guid:#010x}, {object})");
            Some(DecodedIns::branching(expr))
        }
        33 => {
            if strict && ops[0..4] != [0; 4] {
                return None;
            }
            let target = word_u16(ops, 4);
            let mode = word_u16(ops, 6);
            let head = format!("find_worst_motives(target: {target}, mode: {mode})");
            Some(DecodedIns::always_true(head))
        }
        34 => {
            if strict && ops[5..8] != [0; 3] {
                return None;
            }
            let mode = ops[0];
            let head = if mode <= 2 {
                let flags = ops[4];
                let target = format_operand(ops[1], word_i16(ops, 2), max_args, max_locals)?;
                format!("ui_effect(mode: {mode}, target: {target}, flags: {flags})")
            } else {
                if strict && ops[1..5] != [0; 4] {
                    return None;
                }
                format!("ui_effect(mode: {mode})")
            };
            Some(DecodedIns::always_true(head))
        }
        35 => {
            if strict && ops[6..8] != [0; 2] {
                return None;
            }
            let duration = word_i16(ops, 0);
            let kind = ops[2];
            let size = ops[3];
            let flags = ops[4];
            let str_index = ops[5];
            if flags & 0x20 != 0 && (duration < 0 || duration as usize >= max_locals) {
                return None;
            }
            let head = format!(
                "special_effect(duration: {duration}, kind: {kind}, size: {size}, flags: {flags}, str_index: {str_index})"
            );
            Some(DecodedIns::always_true(head))
        }
        36 => {
            if strict && ops[5] & 0x80 != 0 {
                return None;
            }
            let kind = ops[5] & 0x7F;
            let expr = format!(
                "dialog(kind: {kind}, message: {}, title: {}, button1: {}, button2: {}, button3: {}, mode4_str: {}, flags: {})",
                ops[2], ops[6], ops[3], ops[4], ops[0], ops[1], ops[7]
            );
            Some(DecodedIns::branching(expr))
        }
        37 => {
            let expr = if ops[0] == 1 {
                if strict && ops[4..8] != [0; 4] {
                    return None;
                }
                let person = format_operand(ops[1], word_i16(ops, 2), max_args, max_locals)?;
                format!("test_sim_interacting_with({person})")
            } else {
                if strict && *ops != [0; 8] {
                    return None;
                }
                "test_sim_interacting_with()".to_owned()
            };
            Some(DecodedIns::branching(expr))
        }
        41 => {
            let w0 = word_i16(ops, 0);
            let use_stack = ops[0] & 1 != 0;
            let local_val = w0 >> 1;
            let index = ops[2];
            let kind = ops[3];
            let duration = word_i16(ops, 4);
            let options = word_u16(ops, 6);
            let local_byte = if kind == 7 && index == 2 {
                if !(0..=255).contains(&local_val) || (local_val as usize) >= max_locals {
                    return None;
                }
                local_val as u8
            } else {
                if strict && local_val != 0 {
                    return None;
                }
                0
            };
            let mut head = format!(
                "set_headline(kind: {kind}, index: {index}, duration: {duration}, options: {options}"
            );
            if use_stack {
                head.push_str(", stack_obj: true");
            }
            if local_byte != 0 {
                let _ = write!(head, ", local: {local_byte}");
            }
            head.push(')');
            Some(DecodedIns::always_true(head))
        }
        42 => {
            if strict && ops[7] != 0 {
                return None;
            }
            let guid = u32::from_le_bytes([ops[0], ops[1], ops[2], ops[3]]);
            let mode = ops[4];
            let flags = ops[5];
            let local = ops[6];
            if mode == 7 && max_args == 0 {
                return None;
            }
            if (mode == 8 || mode == 9) && ((local as i8) < 0 || usize::from(local) >= max_locals) {
                return None;
            }
            let expr = format!(
                "create_object({guid:#010x}, mode: {mode}, flags: {flags}, local: {local})"
            );
            Some(DecodedIns::branching(expr))
        }
        43 => {
            let sk = word_i16(ops, 0);
            let sv = word_i16(ops, 2);
            let dk = word_i16(ops, 4);
            let dv = word_i16(ops, 6);
            if strict && ((sk != 0 && sk != 1) || (dk != 0 && dk != 1)) {
                return None;
            }
            if sk != 0 && (sv < 0 || sv as usize >= max_args) {
                return None;
            }
            if dk != 0 && (dv < 0 || dv as usize >= max_args) {
                return None;
            }
            let src = if sk == 0 {
                sv.to_string()
            } else {
                format!("param[{sv}]")
            };
            let dst = if dk == 0 {
                dv.to_string()
            } else {
                format!("param[{dv}]")
            };
            let expr = format!("drop_onto({src}, {dst})");
            Some(DecodedIns::branching(expr))
        }
        44 => {
            if strict && (ops[3] != 0 || ops[7] != 0) {
                return None;
            }
            let clip = word_u16(ops, 0);
            let local = ops[2];
            let table = ops[4];
            let flags = ops[5];
            let events = ops[6] as i8;
            if flags & 0x20 != 0 && usize::from(local) >= max_locals {
                return None;
            }
            if flags & 0x20 == 0
                && events != 0
                && matches!(flags & 0x11, 0x00 | 0x11)
                && max_args == 0
            {
                return None;
            }
            if flags & 0x04 != 0 && ((clip as i16) < 0 || (clip as i16) as usize >= max_args) {
                return None;
            }
            let expr = format!(
                "animate(clip: {clip}, table: {table}, flags: {flags}, events: {events}, local: {local})"
            );
            Some(DecodedIns::branching(expr))
        }
        45 => {
            let slot = word_u16(ops, 0);
            let source = word_u16(ops, 2);
            if source > 2 {
                return None;
            }
            if source == 0 && ((slot as i16) < 0 || (slot as i16) as usize >= max_args) {
                return None;
            }
            if strict && ops[5..8] != [0; 3] {
                return None;
            }
            let flags = ops[4];
            let expr = if source == 1 {
                format!("route_to(slot: {slot}, flags: {flags})")
            } else {
                format!("route_to(slot: {slot}, source: {source}, flags: {flags})")
            };
            Some(DecodedIns::branching(expr))
        }
        46 => {
            let slot = word_u16(ops, 0);
            let mode = word_u16(ops, 2);
            if mode > 4 {
                return None;
            }
            if mode == 0 && ((slot as i16) < 0 || (slot as i16) as usize >= max_args) {
                return None;
            }
            if strict && ops[5..8] != [0; 3] {
                return None;
            }
            let flags = ops[4];
            let expr = if mode == 3 {
                format!("snap({slot}, {flags})")
            } else {
                format!("snap(slot: {slot}, mode: {mode}, flags: {flags})")
            };
            Some(DecodedIns::branching(expr))
        }
        47 => {
            if strict && ops[6..8] != [0; 2] {
                return None;
            }
            let mode = word_i16(ops, 0);
            let moves_raw = word_i16(ops, 2);
            if strict && moves_raw != 0 && moves_raw != 1 {
                return None;
            }
            let moves = moves_raw != 0;
            let expr = if mode == 1 {
                let p = word_i16(ops, 4);
                if p < 0 || p as usize >= max_args {
                    return None;
                }
                format!("reach(mode: {mode}, moves: {moves}, param: {p})")
            } else {
                if strict && ops[4..6] != [0; 2] {
                    return None;
                }
                format!("reach(mode: {mode}, moves: {moves})")
            };
            Some(DecodedIns::branching(expr))
        }
        48 => {
            let raw_target = word_u16(ops, 0);
            if strict && (raw_target > 1 || ops[2..8] != [0; 6]) {
                return None;
            }
            let use_stack = raw_target != 0;
            let head = format!("stop_all_sounds({use_stack})");
            Some(DecodedIns::always_true(head))
        }
        49 => {
            if strict && *ops != [0; 8] {
                return None;
            }
            let head = "notify_idle()".to_owned();
            Some(DecodedIns::always_true(head))
        }
        50 => {
            if strict && (ops[2] != 0 || ops[3] != 0 || ops[5..8] != [0; 3]) {
                return None;
            }
            let table = word_u16(ops, 0);
            let index = ops[4];
            let head = format!("action_string(table: {table}, index: {index})");
            Some(DecodedIns::always_true(head))
        }
        51 => {
            let op = ops[0];
            let kind = ops[1];
            let flags = word_u16(ops, 2);
            let guid = u32::from_le_bytes([ops[4], ops[5], ops[6], ops[7]]);
            let expr = format!(
                "manage_inventory(op: {op}, kind: {kind}, flags: {flags}, guid: {guid:#010x})"
            );
            Some(DecodedIns::branching(expr))
        }
        id if id >= 256 => {
            let expr = if *ops == [0xFF; 8] {
                format!("call {id}(temps)")
            } else {
                let a0 = word_i16(ops, 0);
                let a1 = word_i16(ops, 2);
                let a2 = word_i16(ops, 4);
                let a3 = word_i16(ops, 6);
                format!("call {id}({a0}, {a1}, {a2}, {a3})")
            };
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: true,
            })
        }
        _ => None,
    }
}

/// Decodes one instruction into its labeled-CFG `.air` statement head (preceding
/// ` -> true: ..., false: ...;`), returning `None` if it must fall back to `op 0xNNNN (...)`.
pub(crate) fn decode_instruction_for_labeled_air(
    ins: &Instruction,
    max_args: usize,
    max_locals: usize,
    opts: LiftOptions,
) -> Option<String> {
    match try_decode_instruction(ins, max_args, max_locals, opts)? {
        DecodedIns::AlwaysTrue { labeled_head, .. }
        | DecodedIns::Branching { labeled_head, .. } => Some(labeled_head),
    }
}

fn lift_instruction(
    ins: &Instruction,
    max_args: usize,
    max_locals: usize,
    opts: LiftOptions,
    symbols: Option<&LiftSymbols>,
) -> LiftedIns {
    let comment = if opts.annotate_comments {
        instruction_comment(ins, symbols)
    } else {
        None
    };
    if let Some(decoded) = try_decode_instruction(ins, max_args, max_locals, opts) {
        return match decoded {
            DecodedIns::AlwaysTrue {
                structured_stmt, ..
            } => LiftedIns::Stmt {
                text: structured_stmt,
                comment,
                next: ins.on_true,
            },
            DecodedIns::Branching {
                cond_text,
                stmt_form,
                is_call,
                ..
            } => LiftedIns::Cond {
                text: cond_text,
                stmt_form,
                comment,
                is_call,
                on_true: ins.on_true,
                on_false: ins.on_false,
            },
        };
    }

    // Per-instruction inline raw `op 0xNNNN (...)` fallback: preserves the surrounding
    // structured `if`/`while` control flow instead of abandoning the entire tree!
    let raw = format_raw_op(ins);
    if is_always_true_instruction(ins) {
        LiftedIns::Stmt {
            text: format!("{raw};"),
            comment,
            next: ins.on_true,
        }
    } else {
        LiftedIns::Cond {
            stmt_form: Some(format!("{raw};")),
            text: raw,
            comment,
            is_call: false,
            on_true: ins.on_true,
            on_false: ins.on_false,
        }
    }
}

/// Attempts to lift a [`BhavChunk`] strictly in memory into structured `.air` (Antics IR)
/// source text using [`LiftOptions::default`] (`strict_padding: true`).
#[must_use]
pub fn lift_bhav_air_structured(chunk: &BhavChunk) -> StructuredLiftOutcome {
    lift_bhav_structured_with_options(chunk, LiftOptions::default())
}

/// Attempts to lift a [`BhavChunk`] strictly in memory into structured `.air` (Antics IR)
/// source text using explicit [`LiftOptions`].
#[must_use]
pub fn lift_bhav_air_structured_with_options(
    chunk: &BhavChunk,
    opts: LiftOptions,
) -> StructuredLiftOutcome {
    lift_bhav_structured_with_options(chunk, opts)
}

/// Uses a metadata-only synthetic label `Tree_<id>` so no commercial strings
/// ever appear in the generated `.air` (Antics IR) source text.
#[must_use]
pub fn lift_bhav_structured(chunk: &BhavChunk) -> StructuredLiftOutcome {
    lift_bhav_structured_with_options(chunk, LiftOptions::default())
}

/// Structured `.air` lifter parameterized by [`LiftOptions`].
#[must_use]
pub fn lift_bhav_structured_with_options(
    chunk: &BhavChunk,
    opts: LiftOptions,
) -> StructuredLiftOutcome {
    lift_bhav_structured_with_arities(chunk, opts, None)
}

/// Structured `.air` lifter parameterized by [`LiftOptions`] and optional `callee_arities`.
///
/// Refuses structured reordering when a reachable inline subroutine call (`opcode >= 256` with
/// non-`0xFF` operands) targets a known `> 4`-argument callee, or when a callee's arity is
/// unknown and an unreachable instruction record immediately follows the call (where Edith
/// stores extended inline argument bytes, `sims-vm/src/interpreter.rs:1548`).
#[must_use]
pub fn lift_bhav_structured_with_arities(
    chunk: &BhavChunk,
    opts: LiftOptions,
    callee_arities: Option<&BTreeMap<u16, usize>>,
) -> StructuredLiftOutcome {
    lift_bhav_structured_with_context(chunk, opts, callee_arities, None)
}

/// Why [`lift_bhav_structured_with_context`] returned
/// [`StructuredLiftOutcome::UnsupportedHeaderOrTarget`] for a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsupportedHeaderReason {
    /// BHAV format other than `0x8000..=0x8002`.
    Format(u16),
    /// More than 251 instructions (no room for the reserved branch targets).
    TooManyInstructions(usize),
    /// `kind == 4` indexed multi-entry tree (callers may enter at any node).
    IndexedEntry,
    /// A reachable node branches past the end of the tree.
    TargetOutOfBounds { node: usize, target: u8 },
    /// A reachable inline call targets a tree whose known arity is above 4, so it reads
    /// extended arguments from the following instruction records.
    ExtendedArityCallee {
        node: usize,
        callee: u16,
        arity: usize,
    },
    /// A reachable inline call targets a tree with no known arity in a multi-instruction
    /// tree, so the following records might be extended arguments.
    UnknownArityCallee { node: usize, callee: u16 },
}

impl UnsupportedHeaderReason {
    /// The callee tree ID when the reason is an inline call, else `None`.
    #[must_use]
    pub fn callee(&self) -> Option<u16> {
        match self {
            Self::ExtendedArityCallee { callee, .. } | Self::UnknownArityCallee { callee, .. } => {
                Some(*callee)
            }
            _ => None,
        }
    }
}

/// `global`, `private`, or `semi-global` for a callee tree ID, following
/// `sims_vm::interpreter::behavior_from_owner`.
#[must_use]
pub fn callee_scope_name(id: u16) -> &'static str {
    match id {
        0..=255 => "primitive",
        256..=4095 => "global",
        4096..=8191 => "private",
        _ => "semi-global",
    }
}

impl std::fmt::Display for UnsupportedHeaderReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Format(format) => write!(f, "unsupported BHAV format {format:#06x}"),
            Self::TooManyInstructions(n) => write!(f, "{n} instructions (max 251)"),
            Self::IndexedEntry => write!(f, "kind 4 indexed multi-entry tree"),
            Self::TargetOutOfBounds { node, target } => {
                write!(f, "node {node} branches to out-of-bounds node {target}")
            }
            Self::ExtendedArityCallee {
                node,
                callee,
                arity,
            } => write!(
                f,
                "node {node} calls {} tree {callee} with inline operands and {arity} args (>4 reads following records)",
                callee_scope_name(*callee)
            ),
            Self::UnknownArityCallee { node, callee } => write!(
                f,
                "node {node} calls {} tree {callee} with inline operands but its arity is unknown",
                callee_scope_name(*callee)
            ),
        }
    }
}

struct Precheck {
    max_args: usize,
    max_locals: usize,
    reachable: Vec<bool>,
}

/// Explains why structured lifting would refuse `chunk` as an unsupported header or
/// target, or returns `Ok(())` when the header/target/callee-arity checks pass (the tree
/// may still be [`StructuredLiftOutcome::Unstructured`]).
pub fn structured_lift_header_check(
    chunk: &BhavChunk,
    callee_arities: Option<&BTreeMap<u16, usize>>,
) -> Result<(), UnsupportedHeaderReason> {
    structured_lift_precheck(chunk, callee_arities).map(|_| ())
}

fn structured_lift_precheck(
    chunk: &BhavChunk,
    callee_arities: Option<&BTreeMap<u16, usize>>,
) -> Result<Precheck, UnsupportedHeaderReason> {
    let Some((max_args, max_locals)) = format_limits(chunk.format) else {
        return Err(UnsupportedHeaderReason::Format(chunk.format));
    };
    let n = chunk.instructions.len();
    if n > 251 {
        return Err(UnsupportedHeaderReason::TooManyInstructions(n));
    }

    // Indexed-entry trees (kind 4) can enter at caller-selected PCs (`interpreter.rs:1786`),
    // so renumbering or pruning unreachable-from-0 nodes would alter semantics.
    if chunk.kind == 4 {
        return Err(UnsupportedHeaderReason::IndexedEntry);
    }
    if n == 0 {
        return Ok(Precheck {
            max_args,
            max_locals,
            reachable: Vec::new(),
        });
    }

    // 1. Discover reachable nodes from node 0 and validate all targets.
    let mut reachable = vec![false; n];
    let mut queue = VecDeque::new();
    reachable[0] = true;
    queue.push_back(0usize);
    while let Some(idx) = queue.pop_front() {
        let ins = &chunk.instructions[idx];
        let succs: &[u8] = if is_always_true_instruction(ins) {
            &[ins.on_true]
        } else {
            &[ins.on_true, ins.on_false]
        };
        for &t in succs {
            if !matches!(t, TARGET_ERROR..=TARGET_FALSE) {
                let next = usize::from(t);
                if next >= n {
                    return Err(UnsupportedHeaderReason::TargetOutOfBounds {
                        node: idx,
                        target: t,
                    });
                }
                if !reachable[next] {
                    reachable[next] = true;
                    queue.push_back(next);
                }
            }
        }
    }

    // Protect layout-dependent extended (>4-arg) inline subroutine calls before reordering
    // or pruning instruction records (`sims-vm/src/interpreter.rs:1548`).
    // Any call with known `arity > 4` or with unknown arity in a multi-instruction tree
    // can read raw bytes (opcode, branch bytes, operands) from following records even when
    // those records are reachable as code, so fall back to labeled-CFG mode.
    for (idx, ins) in chunk.instructions.iter().enumerate() {
        if !reachable[idx] {
            continue;
        }
        if ins.opcode >= 256 && ins.operands != [0xFF; 8] {
            match callee_arities.and_then(|m| m.get(&ins.opcode).copied()) {
                Some(arity) if arity <= 4 => {}
                Some(arity) => {
                    return Err(UnsupportedHeaderReason::ExtendedArityCallee {
                        node: idx,
                        callee: ins.opcode,
                        arity,
                    });
                }
                None if n > 1 => {
                    return Err(UnsupportedHeaderReason::UnknownArityCallee {
                        node: idx,
                        callee: ins.opcode,
                    });
                }
                None => {}
            }
        }
    }
    Ok(Precheck {
        max_args,
        max_locals,
        reachable,
    })
}

/// Structured `.air` lifter parameterized by [`LiftOptions`], optional `callee_arities`,
/// and optional in-memory [`LiftSymbols`] for trailing readability comments.
#[must_use]
pub fn lift_bhav_structured_with_context(
    chunk: &BhavChunk,
    opts: LiftOptions,
    callee_arities: Option<&BTreeMap<u16, usize>>,
    symbols: Option<&LiftSymbols>,
) -> StructuredLiftOutcome {
    let scoped = symbols.map(|s| s.for_caller(chunk.id));
    let symbols = scoped.as_deref();
    let precheck = match structured_lift_precheck(chunk, callee_arities) {
        Ok(p) => p,
        Err(_) => return StructuredLiftOutcome::UnsupportedHeaderOrTarget,
    };
    let Precheck {
        max_args,
        max_locals,
        reachable,
    } = precheck;
    let n = chunk.instructions.len();
    let eff_args = if chunk.format == 0x8000 {
        chunk.args
    } else {
        usize::from(chunk.args).min(max_args) as u8
    };
    // Match `sims_vm::interpreter::dimensions`: the local count is truncated to a byte (`locals as u8`).
    let eff_locals = usize::from(chunk.locals as u8).min(max_locals) as u16;

    // Header (preserves exact format, kind, effective args/locals, revision with clean placeholder label).
    let header = format!(
        "tree {} \"Tree_{}\" (format: {:#06x}, kind: {}, args: {}, locals: {}, revision: {:#010x}) {{\n",
        chunk.id, chunk.id, chunk.format, chunk.kind, eff_args, eff_locals, chunk.revision
    );

    if n == 0 {
        return StructuredLiftOutcome::Lifted(format!("{header}}}\n"));
    }

    // 2. Lift every reachable instruction (using inline `op 0xNNNN (...)` per instruction
    //    only when an opcode is unmodeled or padding is dirty under `strict_padding`).
    let mut lifted: Vec<Option<LiftedIns>> = vec![None; n];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut has_terminal_edge: Vec<bool> = vec![false; n];
    for (idx, ins) in chunk.instructions.iter().enumerate() {
        if !reachable[idx] {
            continue;
        }
        let arg_bound = if chunk.format == 0x8000 {
            4
        } else {
            usize::from(eff_args)
        };
        let local_bound = usize::from(eff_locals);
        let li = lift_instruction(ins, arg_bound, local_bound, opts, symbols);
        let mut edges = Vec::new();
        match &li {
            LiftedIns::Stmt { next, .. } => {
                if matches!(*next, TARGET_ERROR..=TARGET_FALSE) {
                    has_terminal_edge[idx] = true;
                } else {
                    edges.push(usize::from(*next));
                }
            }
            LiftedIns::Cond {
                on_true, on_false, ..
            } => {
                if matches!(*on_true, TARGET_ERROR..=TARGET_FALSE) {
                    has_terminal_edge[idx] = true;
                } else {
                    edges.push(usize::from(*on_true));
                }
                if matches!(*on_false, TARGET_ERROR..=TARGET_FALSE) {
                    has_terminal_edge[idx] = true;
                } else if *on_false != *on_true {
                    edges.push(usize::from(*on_false));
                }
            }
        }
        succs[idx] = edges;
        lifted[idx] = Some(li);
    }

    // 3. Compute dominators (to detect natural loops) and post-dominators (for if/else merges).
    let dom = compute_dominators(n, &reachable, &succs);
    let ipdom = compute_immediate_postdominators(n, &reachable, &succs, &has_terminal_edge);

    // Build the full predecessor graph BEFORE collecting natural loops so loop discovery
    // is completely independent of instruction storage order.
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for u in 0..n {
        if !reachable[u] {
            continue;
        }
        for &v in &succs[u] {
            preds[v].push(u);
        }
    }

    // Detect natural loop headers (nodes targeted by a back-edge `u -> h` where `h` dominates `u`).
    let mut loop_bodies: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for u in 0..n {
        if !reachable[u] {
            continue;
        }
        for &v in &succs[u] {
            if dom[u].contains(&v) {
                // Collect natural loop nodes for back-edge u -> v.
                let body = loop_bodies.entry(v).or_default();
                body.insert(v);
                let mut work = VecDeque::new();
                if body.insert(u) {
                    work.push_back(u);
                }
                while let Some(cur) = work.pop_front() {
                    for &p in &preds[cur] {
                        if body.insert(p) {
                            work.push_back(p);
                        }
                    }
                }
            }
        }
    }

    // Compute per-loop DAG post-dominators so `if`/`else` diamonds inside infinite loops
    // (such as object `Main` loops with no `return`) merge cleanly without node duplication.
    let loop_ipdoms = compute_loop_ipdoms(n, &succs, &has_terminal_edge, &loop_bodies);

    // 4. Recursively emit structured `.antics` statements.
    let mut emitter = StructEmitter {
        lifted: &lifted,
        succs: &succs,
        ipdom: &ipdom,
        loop_bodies: &loop_bodies,
        loop_ipdoms: &loop_ipdoms,
        out: header,
        emitted_nodes: 0,
        max_emitted_nodes: 245, // Stay safely under MAX_BHAV_NODES (251)
    };

    if emitter
        .emit_region(0, None, None, &mut BTreeSet::new(), 1, false)
        .is_err()
    {
        return StructuredLiftOutcome::Unstructured;
    }

    emitter.out.push_str("}\n");
    StructuredLiftOutcome::Lifted(emitter.out)
}

fn compute_dominators(n: usize, reachable: &[bool], succs: &[Vec<usize>]) -> Vec<BTreeSet<usize>> {
    let all: BTreeSet<usize> = (0..n).filter(|&i| reachable[i]).collect();
    let mut dom: Vec<BTreeSet<usize>> = (0..n)
        .map(|i| {
            if i == 0 {
                BTreeSet::from([0])
            } else if reachable[i] {
                all.clone()
            } else {
                BTreeSet::new()
            }
        })
        .collect();

    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for u in 0..n {
        if reachable[u] {
            for &v in &succs[u] {
                preds[v].push(u);
            }
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for u in 1..n {
            if !reachable[u] || preds[u].is_empty() {
                continue;
            }
            let mut iter = preds[u].iter().copied();
            let first = iter.next().unwrap();
            let mut inter = dom[first].clone();
            for p in iter {
                inter.retain(|x| dom[p].contains(x));
            }
            inter.insert(u);
            if inter != dom[u] {
                dom[u] = inter;
                changed = true;
            }
        }
    }
    dom
}

/// Computes immediate post-dominators using a virtual exit node `n`.
fn compute_immediate_postdominators(
    n: usize,
    reachable: &[bool],
    succs: &[Vec<usize>],
    has_terminal_edge: &[bool],
) -> Vec<Option<usize>> {
    let exit = n;
    let total = n + 1;
    // Reverse graph: virtual exit is successor of any node with an actual terminal edge (0xFD..=0xFF).
    let mut rsuccs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut rpreds: Vec<Vec<usize>> = vec![Vec::new(); total];
    for u in 0..n {
        if !reachable[u] {
            continue;
        }
        rsuccs[u] = succs[u].clone();
        if has_terminal_edge[u] {
            rsuccs[u].push(exit);
        }
        for &v in &rsuccs[u] {
            rpreds[v].push(u);
        }
    }

    // Identify nodes that can actually reach the virtual exit; leave exitless components
    // without synthetic post-dominator candidates.
    let mut can_reach_exit = vec![false; total];
    let mut back_q = VecDeque::new();
    can_reach_exit[exit] = true;
    back_q.push_back(exit);
    while let Some(v) = back_q.pop_front() {
        for &u in &rpreds[v] {
            if !can_reach_exit[u] {
                can_reach_exit[u] = true;
                back_q.push_back(u);
            }
        }
    }

    let all: BTreeSet<usize> = (0..n)
        .filter(|&i| reachable[i] && can_reach_exit[i])
        .chain([exit])
        .collect();
    let mut pdom: Vec<BTreeSet<usize>> = (0..total)
        .map(|i| {
            if i == exit {
                BTreeSet::from([exit])
            } else if reachable[i] && can_reach_exit[i] {
                all.clone()
            } else {
                BTreeSet::new()
            }
        })
        .collect();

    let mut changed = true;
    while changed {
        changed = false;
        for u in (0..n).rev() {
            if !reachable[u] || !can_reach_exit[u] || rsuccs[u].is_empty() {
                continue;
            }
            let mut iter = rsuccs[u].iter().copied().filter(|&v| can_reach_exit[v]);
            let Some(first) = iter.next() else {
                continue;
            };
            let mut inter = pdom[first].clone();
            for s in iter {
                inter.retain(|x| pdom[s].contains(x));
            }
            inter.insert(u);
            if inter != pdom[u] {
                pdom[u] = inter;
                changed = true;
            }
        }
    }

    let mut ipdom = vec![None; n];
    for u in 0..n {
        if !reachable[u] {
            continue;
        }
        // Immediate post-dominator is the strict post-dominator with the largest pdom set.
        let best = pdom[u]
            .iter()
            .copied()
            .filter(|&d| d != u && d != exit)
            .max_by_key(|&d| pdom[d].len());
        ipdom[u] = best;
    }
    ipdom
}

/// Computes immediate post-dominators within each natural loop's DAG (treating back-edges
/// to `header`, edges exiting `body_set`, and terminal returns as edges to a virtual exit)
/// so diamonds inside infinite loops merge cleanly without duplicating the loop tail.
fn compute_loop_ipdoms(
    n: usize,
    succs: &[Vec<usize>],
    has_terminal_edge: &[bool],
    loop_bodies: &BTreeMap<usize, BTreeSet<usize>>,
) -> BTreeMap<usize, BTreeMap<usize, usize>> {
    let exit = n;
    let total = n + 1;
    let mut out = BTreeMap::new();
    for (&header, body) in loop_bodies {
        let mut rsuccs: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut rpreds: Vec<Vec<usize>> = vec![Vec::new(); total];
        for &u in body {
            let mut reaches_exit_directly = has_terminal_edge[u];
            for &v in &succs[u] {
                if v == header || !body.contains(&v) {
                    reaches_exit_directly = true;
                } else {
                    rsuccs[u].push(v);
                }
            }
            if reaches_exit_directly {
                rsuccs[u].push(exit);
            }
            for &v in &rsuccs[u] {
                rpreds[v].push(u);
            }
        }
        let mut can_reach = vec![false; total];
        let mut q = VecDeque::new();
        can_reach[exit] = true;
        q.push_back(exit);
        while let Some(v) = q.pop_front() {
            for &u in &rpreds[v] {
                if !can_reach[u] {
                    can_reach[u] = true;
                    q.push_back(u);
                }
            }
        }
        let all: BTreeSet<usize> = body
            .iter()
            .copied()
            .filter(|&u| can_reach[u])
            .chain([exit])
            .collect();
        let mut pdom: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        pdom.insert(exit, BTreeSet::from([exit]));
        for &u in body {
            if can_reach[u] {
                pdom.insert(u, all.clone());
            }
        }
        let nodes: Vec<usize> = body.iter().copied().filter(|&u| can_reach[u]).collect();
        let mut changed = true;
        while changed {
            changed = false;
            for &u in nodes.iter().rev() {
                let mut iter = rsuccs[u].iter().copied().filter(|&v| can_reach[v]);
                let Some(first) = iter.next() else {
                    continue;
                };
                let mut inter = pdom[&first].clone();
                for s in iter {
                    inter.retain(|x| pdom[&s].contains(x));
                }
                inter.insert(u);
                if pdom.get(&u) != Some(&inter) {
                    pdom.insert(u, inter);
                    changed = true;
                }
            }
        }
        let mut map = BTreeMap::new();
        for &u in &nodes {
            if let Some(best) = pdom[&u]
                .iter()
                .copied()
                .filter(|&d| d != u && d != exit && d != header)
                .max_by_key(|&d| pdom[&d].len())
            {
                map.insert(u, best);
            }
        }
        out.insert(header, map);
    }
    out
}

#[derive(Debug, Clone, Copy)]
struct LoopCtx {
    header: u8,
    exit: Option<u8>,
}

struct StructEmitter<'a> {
    lifted: &'a [Option<LiftedIns>],
    succs: &'a [Vec<usize>],
    ipdom: &'a [Option<usize>],
    loop_bodies: &'a BTreeMap<usize, BTreeSet<usize>>,
    loop_ipdoms: &'a BTreeMap<usize, BTreeMap<usize, usize>>,
    out: String,
    emitted_nodes: usize,
    max_emitted_nodes: usize,
}

impl StructEmitter<'_> {
    fn indent(&mut self, depth: usize) {
        for _ in 0..depth {
            self.out.push_str("    ");
        }
    }

    fn push_comment(&mut self, comment: Option<&str>) {
        if let Some(c) = comment {
            self.out.push_str(" // ");
            self.out.push_str(c);
        }
        self.out.push('\n');
    }

    fn emit_terminal(&mut self, target: u8, depth: usize) -> std::result::Result<(), ()> {
        let kw = match target {
            TARGET_TRUE => "return true;",
            TARGET_FALSE => "return false;",
            TARGET_ERROR => "return error;",
            _ => return Err(()),
        };
        self.indent(depth);
        self.out.push_str(kw);
        self.out.push('\n');
        Ok(())
    }

    fn immediate_terminator(target: u8, loop_ctx: Option<LoopCtx>) -> Option<&'static str> {
        if let Some(ctx) = loop_ctx {
            if target == ctx.header {
                return Some("continue;");
            }
            if ctx.exit == Some(target) {
                return Some("break;");
            }
        }
        match target {
            TARGET_TRUE => Some("return true;"),
            TARGET_FALSE => Some("return false;"),
            TARGET_ERROR => Some("return error;"),
            _ => None,
        }
    }

    /// Selects a canonical `break` exit target for a `while true` natural loop:
    /// - If the loop has no non-terminal exits, returns `None` (all exits are `return` or infinite).
    /// - If the loop has a unique non-terminal exit `E`, returns `Some(E)`.
    /// - If multiple non-terminal exits exist, prefers the header's immediate post-dominator
    ///   (when outside the loop body), otherwise the most-targeted exit from `body_set`.
    fn pick_loop_exit(&self, header: usize, body_set: &BTreeSet<usize>) -> Option<u8> {
        let mut exit_counts: BTreeMap<usize, usize> = BTreeMap::new();
        for &u in body_set {
            for &v in &self.succs[u] {
                if !body_set.contains(&v) {
                    *exit_counts.entry(v).or_insert(0) += 1;
                }
            }
        }
        if exit_counts.is_empty() {
            return None;
        }
        if exit_counts.len() == 1 {
            return exit_counts.keys().next().map(|&v| v as u8);
        }
        if let Some(ipd) = self.ipdom.get(header).and_then(|&m| m)
            && !body_set.contains(&ipd)
        {
            return Some(ipd as u8);
        }
        exit_counts
            .into_iter()
            .max_by(|(v1, c1), (v2, c2)| c1.cmp(c2).then_with(|| v2.cmp(v1)))
            .map(|(v, _)| v as u8)
    }

    fn emit_region(
        &mut self,
        mut cur: u8,
        stop_at: Option<u8>,
        loop_ctx: Option<LoopCtx>,
        active: &mut BTreeSet<u8>,
        depth: usize,
        mut entering_loop_header: bool,
    ) -> std::result::Result<(), ()> {
        // Keep well below parser's `MAX_NESTING_DEPTH = 64` (each `if`/`while` + block adds 2).
        if depth > 28 {
            return Err(());
        }
        let mut local_seen = BTreeSet::new();
        loop {
            if !entering_loop_header {
                if Some(cur) == stop_at {
                    return Ok(());
                }
                if let Some(ctx) = loop_ctx {
                    if cur == ctx.header {
                        self.indent(depth);
                        self.out.push_str("continue;\n");
                        return Ok(());
                    }
                    if ctx.exit == Some(cur) {
                        self.indent(depth);
                        self.out.push_str("break;\n");
                        return Ok(());
                    }
                }
                if matches!(cur, TARGET_ERROR..=TARGET_FALSE) {
                    return self.emit_terminal(cur, depth);
                }
                if active.contains(&cur) || !local_seen.insert(cur) {
                    // Back-edge to an active non-loop-header node -> irreducible CFG.
                    return Err(());
                }
            }
            let is_entering = entering_loop_header;
            entering_loop_header = false;

            let idx = usize::from(cur);
            let Some(li) = self.lifted.get(idx).and_then(Option::as_ref) else {
                return Err(());
            };

            // Check if `cur` is a natural loop header (when entering fresh, not re-entering its own body).
            if !is_entering && let Some(body_set) = self.loop_bodies.get(&idx) {
                // Case 1: Clean `while cond { ... }` header where exactly one branch enters the loop.
                if let LiftedIns::Cond {
                    text,
                    comment,
                    on_true,
                    on_false,
                    ..
                } = li
                {
                    let true_in = !matches!(*on_true, TARGET_ERROR..=TARGET_FALSE)
                        && body_set.contains(&usize::from(*on_true));
                    let false_in = !matches!(*on_false, TARGET_ERROR..=TARGET_FALSE)
                        && body_set.contains(&usize::from(*on_false));

                    if true_in ^ false_in {
                        let (cond_str, body_entry, exit_target) = if true_in {
                            (text.clone(), *on_true, *on_false)
                        } else {
                            (format!("!{text}"), *on_false, *on_true)
                        };
                        // When the header's own exit is a terminal `return` while the loop body
                        // has a real non-terminal exit node `E`, prefer `while true` so `break;`
                        // targets `E` (avoiding duplicating `E` across multiple exits!).
                        let defer_to_loop = matches!(exit_target, TARGET_ERROR..=TARGET_FALSE)
                            && self.pick_loop_exit(idx, body_set).is_some();
                        if !defer_to_loop {
                            self.emitted_nodes += 1;
                            if self.emitted_nodes > self.max_emitted_nodes {
                                return Err(());
                            }
                            self.indent(depth);
                            let _ = write!(self.out, "while {cond_str} {{");
                            self.push_comment(comment.as_deref());
                            active.insert(cur);
                            let res = self.emit_region(
                                body_entry,
                                Some(cur),
                                Some(LoopCtx {
                                    header: cur,
                                    exit: Some(exit_target),
                                }),
                                active,
                                depth + 1,
                                false,
                            );
                            active.remove(&cur);
                            res?;
                            self.indent(depth);
                            self.out.push_str("}\n");
                            cur = exit_target;
                            continue;
                        }
                    }
                }

                // Case 2: Unconditional / statement-headed / both-branches-in-loop natural loop:
                // emit zero-overhead `while true { ... }` with `break;` / `continue;` support!
                let loop_exit = self.pick_loop_exit(idx, body_set);
                self.indent(depth);
                self.out.push_str("while true {\n");
                active.insert(cur);
                let res = self.emit_region(
                    cur,
                    Some(cur),
                    Some(LoopCtx {
                        header: cur,
                        exit: loop_exit,
                    }),
                    active,
                    depth + 1,
                    true,
                );
                active.remove(&cur);
                res?;
                self.indent(depth);
                self.out.push_str("}\n");
                if let Some(exit_node) = loop_exit {
                    cur = exit_node;
                    continue;
                }
                return Ok(());
            }

            self.emitted_nodes += 1;
            if self.emitted_nodes > self.max_emitted_nodes {
                return Err(());
            }

            match li {
                LiftedIns::Stmt {
                    text,
                    comment,
                    next,
                } => {
                    self.indent(depth);
                    self.out.push_str(text);
                    self.push_comment(comment.as_deref());
                    cur = *next;
                }
                LiftedIns::Cond {
                    text,
                    stmt_form,
                    comment,
                    is_call,
                    on_true,
                    on_false,
                } => {
                    // Case A: Identical targets and has a standalone non-call statement form.
                    if *on_true == *on_false
                        && !*is_call
                        && let Some(stmt) = stmt_form
                    {
                        self.indent(depth);
                        self.out.push_str(stmt);
                        self.push_comment(comment.as_deref());
                        cur = *on_true;
                        continue;
                    }

                    // Case B: Subroutine call whose false branch is `TARGET_FALSE` (matches `call ...;`).
                    if *is_call
                        && *on_false == TARGET_FALSE
                        && let Some(stmt) = stmt_form
                    {
                        self.indent(depth);
                        self.out.push_str(stmt);
                        self.push_comment(comment.as_deref());
                        cur = *on_true;
                        continue;
                    }

                    // Case C: General `if` / `else`, merging at loop-DAG ipdom or global ipdom.
                    let loop_merge = loop_ctx.and_then(|ctx| {
                        self.loop_ipdoms
                            .get(&usize::from(ctx.header))
                            .and_then(|m| m.get(&idx))
                            .copied()
                    });
                    let merge = loop_merge
                        .or_else(|| self.ipdom.get(idx).and_then(|&m| m))
                        .map(|m| m as u8)
                        .filter(|&m| {
                            Some(m) != stop_at
                                && !active.contains(&m)
                                && loop_ctx.is_none_or(|c| m != c.header && c.exit != Some(m))
                        });
                    let branch_stop = merge.or(stop_at);

                    let true_is_skip = Some(*on_true) == branch_stop;
                    let false_is_skip = Some(*on_false) == branch_stop;

                    // Immediate-terminator flattening: when one branch immediately exits
                    // (`return ...;` / `break;` / `continue;`) and the other continues, emit
                    // a flat `if cond { <term> }` guard and continue linearly at the same depth!
                    if merge.is_none() && !true_is_skip && !false_is_skip {
                        let term_t = Self::immediate_terminator(*on_true, loop_ctx);
                        let term_f = Self::immediate_terminator(*on_false, loop_ctx);
                        if let (Some(tt), None) = (term_t, term_f) {
                            self.indent(depth);
                            let _ = write!(self.out, "if {text} {{");
                            self.push_comment(comment.as_deref());
                            self.indent(depth + 1);
                            self.out.push_str(tt);
                            self.out.push('\n');
                            self.indent(depth);
                            self.out.push_str("}\n");
                            cur = *on_false;
                            continue;
                        }
                        if let (None, Some(tf)) = (term_t, term_f) {
                            self.indent(depth);
                            let _ = write!(self.out, "if !{text} {{");
                            self.push_comment(comment.as_deref());
                            self.indent(depth + 1);
                            self.out.push_str(tf);
                            self.out.push('\n');
                            self.indent(depth);
                            self.out.push_str("}\n");
                            cur = *on_true;
                            continue;
                        }
                    }

                    active.insert(cur);
                    let res = if true_is_skip && false_is_skip {
                        self.indent(depth);
                        let _ = write!(self.out, "if {text} {{");
                        self.push_comment(comment.as_deref());
                        self.indent(depth);
                        self.out.push_str("}\n");
                        Ok(())
                    } else if false_is_skip {
                        self.indent(depth);
                        let _ = write!(self.out, "if {text} {{");
                        self.push_comment(comment.as_deref());
                        let r = self.emit_region(
                            *on_true,
                            branch_stop,
                            loop_ctx,
                            active,
                            depth + 1,
                            false,
                        );
                        self.indent(depth);
                        self.out.push_str("}\n");
                        r
                    } else if true_is_skip {
                        self.indent(depth);
                        let _ = write!(self.out, "if !{text} {{");
                        self.push_comment(comment.as_deref());
                        let r = self.emit_region(
                            *on_false,
                            branch_stop,
                            loop_ctx,
                            active,
                            depth + 1,
                            false,
                        );
                        self.indent(depth);
                        self.out.push_str("}\n");
                        r
                    } else {
                        self.indent(depth);
                        let _ = write!(self.out, "if {text} {{");
                        self.push_comment(comment.as_deref());
                        let r1 = self.emit_region(
                            *on_true,
                            branch_stop,
                            loop_ctx,
                            active,
                            depth + 1,
                            false,
                        );
                        self.indent(depth);
                        self.out.push_str("} else {\n");
                        let r2 = r1.and_then(|()| {
                            self.emit_region(
                                *on_false,
                                branch_stop,
                                loop_ctx,
                                active,
                                depth + 1,
                                false,
                            )
                        });
                        self.indent(depth);
                        self.out.push_str("}\n");
                        r2
                    };
                    active.remove(&cur);
                    res?;

                    if let Some(m) = merge {
                        cur = m;
                        continue;
                    }
                    return Ok(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile_antics;
    use crate::verify::{EquivOptions, EquivResult, check_bhav_equivalent};

    #[test]
    fn structured_lifter_roundtrips_if_else_and_while_loops() {
        let src = r#"
            tree 4096 "SampleLoop" {
                stack.attr[0] := 1;
                play_sound(1, stack_obj);
                let i = 0;
                while i < 4 {
                    if my.person[33] == 0 {
                        break;
                    }
                    my.motive[5] += 15;
                    i += 1;
                }
                stack.attr[0] := 0;
                return true;
            }
        "#;
        let orig = &compile_antics(src).unwrap().behaviors[&4096];
        let lifted_src = match lift_bhav_structured(orig) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            !lifted_src.contains("op 0x"),
            "structured lifter must emit zero raw op instructions:\n{lifted_src}"
        );
        assert!(
            lifted_src.contains("while ") && lifted_src.contains("if "),
            "expected structured while + if in:\n{lifted_src}"
        );
        let recompiled = &compile_antics(&lifted_src).unwrap().behaviors[&4096];
        let equiv = check_bhav_equivalent(orig, recompiled, &EquivOptions::default());
        assert!(
            matches!(equiv, EquivResult::Equivalent { .. }),
            "expected Equivalent, got {equiv:?}\nLifted:\n{lifted_src}"
        );
    }

    #[test]
    fn wave1_all_16_primitives_and_submodes_roundtrip_strict_and_equiv() {
        use crate::encode::encode_bhav;
        use crate::lift::format_bhav_labeled_air;

        let src = r#"
            tree 4096 "Wave1AllPrimitives" (args: 2, locals: 3) {
                action_string(table: 300, index: 1);
                if test_object_type(0x12345678, stack_obj) {
                    if push_interaction(menu: 2, object: 1, priority: 3, flags: 3, icon: 2) {
                        if run_tree_by_name(table: 303, index: 4, mode: 2, global: true) {
                            refresh(target: 1, mode: 2);
                        }
                    }
                }
                relationship(column: 0, dir: 0, flags: 4, value: temp[0]);
                if relationship(column: 1, dir: 2, flags: 3, value: local[0], local: 1) {
                    if go_to_relative(location: -2, facing: 254, options: 2) {
                        set_headline(kind: 7, index: 2, duration: 60, options: 0x0800, stack_obj: true, local: 1);
                        notify_idle();
                    }
                }
                if manage_inventory(op: 3, kind: 1, flags: 0x2040, guid: 0xdeadbeef) {
                    if test_sim_interacting_with(temp[1]) {
                        if test_sim_interacting_with() {
                            if grab() {
                                if drop_onto(0, param[1]) {
                                    drop();
                                }
                            }
                        }
                    }
                }
                change_suit(index: 5, scope: 1);
                change_suit(index: 2, scope: 0, remove: true);
                find_location(mode: 1, flags: 0);
                if find_location(mode: 3, flags: 5, local: 2) {
                    if find_best_object(2, flags: 8, origin: stack_obj) {
                        if route_to(slot: 0, source: 0, flags: 1) {
                            if route_to(slot: 2, source: 2, flags: 0) {
                                if snap(slot: 0, mode: 0, flags: 0) {
                                    if snap(slot: 0, mode: 1, flags: 0) {
                                        if snap(slot: 0, mode: 2, flags: 0) {
                                            if snap(slot: 1, mode: 4, flags: 2) {
                                                play_sound(42, flags: 6);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                return true;
            }
        "#;
        let orig = &compile_antics(src).expect("compile wave1 src").behaviors[&4096];

        // 1. Structured lift under strict padding must emit ZERO raw ops and prove Equivalent.
        let structured = match lift_bhav_structured_with_options(orig, LiftOptions::default()) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            !structured.contains("op 0x"),
            "all Wave 1 primitives & submodes must lift with zero raw ops:\n{structured}"
        );
        for expected in [
            "action_string(table: 300, index: 1);",
            "test_object_type(0x12345678, stack_obj)",
            "push_interaction(menu: 2, object: 1, priority: 3, flags: 3, icon: 2)",
            "run_tree_by_name(table: 303, index: 4, mode: 2, global: true)",
            "refresh(target: 1, mode: 2);",
            "relationship(column: 0, dir: 0, flags: 4, value: temp[0]);",
            "relationship(column: 1, dir: 2, flags: 3, value: local[0], local: 1)",
            "go_to_relative(location: -2, facing: 254, options: 2)",
            "set_headline(kind: 7, index: 2, duration: 60, options: 2048, stack_obj: true, local: 1);",
            "notify_idle();",
            "manage_inventory(op: 3, kind: 1, flags: 8256, guid: 0xdeadbeef)",
            "test_sim_interacting_with(temp[1])",
            "test_sim_interacting_with()",
            "grab()",
            "drop_onto(0, param[1])",
            "drop();",
            "change_suit(index: 5, scope: 1);",
            "change_suit(index: 2, scope: 0, remove: true);",
            "find_location(mode: 1, flags: 0);",
            "find_location(mode: 3, flags: 5, local: 2)",
            "find_best_object(2, flags: 8, origin: stack_obj)",
            "route_to(slot: 0, source: 0, flags: 1)",
            "route_to(slot: 2, source: 2, flags: 0)",
            "snap(slot: 0, mode: 0, flags: 0)",
            "snap(slot: 0, mode: 1, flags: 0)",
            "snap(slot: 0, mode: 2, flags: 0)",
            "snap(slot: 1, mode: 4, flags: 2)",
            "play_sound(42, flags: 6);",
        ] {
            assert!(
                structured.contains(expected),
                "missing `{expected}` in structured lift:\n{structured}"
            );
        }
        let recompiled = &compile_antics(&structured)
            .expect("recompile structured")
            .behaviors[&4096];
        let equiv = check_bhav_equivalent(orig, recompiled, &EquivOptions::default());
        assert!(
            matches!(equiv, EquivResult::Equivalent { .. }),
            "expected Equivalent, got {equiv:?}\nStructured:\n{structured}"
        );

        // 2. Decoded labeled-CFG format must emit ZERO raw ops and round-trip BYTE-EXACTLY.
        let labeled = format_bhav_labeled_air(orig, LiftOptions::default());
        assert!(
            !labeled.contains("op 0x"),
            "decoded labeled CFG should decode all modeled primitives:\n{labeled}"
        );
        let re_labeled = &compile_antics(&labeled)
            .expect("recompile labeled")
            .behaviors[&4096];
        assert_eq!(
            encode_bhav(re_labeled).unwrap(),
            encode_bhav(orig).unwrap(),
            "decoded labeled-CFG must round-trip byte-for-byte"
        );
    }

    #[test]
    fn inline_raw_op_works_as_statement_and_if_while_condition() {
        let src = r#"
            tree 4096 "InlineRawOps" {
                op 0x0042 (0x01 0x02 0x03 0x04 0x05 0x06 0x07 0x08);
                while !op 0x0043 (0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80) {
                    if op 0x0044 (0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x00, 0x11) {
                        my.attr[0] += 1;
                    } else {
                        break;
                    }
                }
                return true;
            }
        "#;
        let orig = &compile_antics(src)
            .expect("compile inline raw op")
            .behaviors[&4096];
        let lifted = match lift_bhav_structured(orig) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(lifted.contains("op 0x0042"), "{lifted}");
        assert!(lifted.contains("while !op 0x0043"), "{lifted}");
        assert!(
            lifted.contains("if op 0x0044") || lifted.contains("if !op 0x0044"),
            "{lifted}"
        );
        let recompiled = &compile_antics(&lifted)
            .expect("recompile inline raw op")
            .behaviors[&4096];
        let equiv = check_bhav_equivalent(orig, recompiled, &EquivOptions::default());
        assert!(
            matches!(equiv, EquivResult::Equivalent { .. }),
            "expected Equivalent, got {equiv:?}\nLifted:\n{lifted}"
        );
    }

    #[test]
    fn strict_padding_vs_relax_padding_preserves_dirty_edith_bytes_or_cleans_them() {
        use crate::encode::encode_bhav;
        use crate::lift::format_bhav_labeled_air;

        // Grab (opcode 4) with dirty Edith padding bytes, followed by Refresh (opcode 7)
        // with dirty upper padding bytes, plus an Expression (opcode 2) with dirty byte 4.
        let orig = BhavChunk {
            id: 4096,
            label: "DirtyEdithPadding".to_owned(),
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            revision: 0,
            instructions: vec![
                Instruction {
                    opcode: 4, // Grab: ignores all 8 bytes in VM, dirty byte 7 = 0x99
                    on_true: 1,
                    on_false: 0xFF,
                    operands: [0, 0, 0, 0, 0, 0, 0, 0x99],
                },
                Instruction {
                    opcode: 7, // Refresh(target: 0, mode: 1) with dirty byte 5 = 0x55
                    on_true: 2,
                    on_false: 0xFD,
                    operands: [0, 0, 1, 0, 0, 0x55, 0, 0],
                },
                Instruction {
                    opcode: 2, // my.attr[0] := 3 with dirty byte 4 = 0x77
                    on_true: 0xFE,
                    on_false: 0xFD,
                    operands: [0, 0, 3, 0, 0x77, 5, 0, 7],
                },
            ],
            trailing: Vec::new(),
        };

        // Strict padding: preserves dirty bytes with `pad` clauses while keeping structured `if`.
        let strict_src = match lift_bhav_structured_with_options(
            &orig,
            LiftOptions {
                strict_padding: true,
                annotate_comments: false,
            },
        ) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            strict_src.contains("if grab() pad(7: 0x99)")
                || strict_src.contains("if !grab() pad(7: 0x99)"),
            "{strict_src}"
        );
        assert!(
            strict_src.contains("refresh(target: 0, mode: 1) pad(5: 0x55);"),
            "{strict_src}"
        );
        assert!(
            strict_src.contains("expr(my.attr[0] := 3) pad 0x77;"),
            "{strict_src}"
        );
        assert!(!strict_src.contains("op 0x"), "{strict_src}");
        let re_strict = &compile_antics(&strict_src).unwrap().behaviors[&4096];
        let equiv = check_bhav_equivalent(&orig, re_strict, &EquivOptions::default());
        assert!(
            matches!(equiv, EquivResult::Equivalent { .. }),
            "{equiv:?}\n{strict_src}"
        );

        // Labeled CFG under strict padding must round-trip 100% byte-for-byte!
        let strict_labeled = format_bhav_labeled_air(
            &orig,
            LiftOptions {
                strict_padding: true,
                annotate_comments: false,
            },
        );
        let re_strict_labeled = &compile_antics(&strict_labeled).unwrap().behaviors[&4096];
        assert_eq!(
            encode_bhav(re_strict_labeled).unwrap(),
            encode_bhav(&orig).unwrap()
        );

        // Relaxed padding: strips verified-ignored padding bytes, lifts to clean named primitives,
        // AND still verifies 100% trace-equivalent!
        let relaxed_src = match lift_bhav_structured_with_options(
            &orig,
            LiftOptions {
                strict_padding: false,
                annotate_comments: false,
            },
        ) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            !relaxed_src.contains("op 0x"),
            "relaxed padding should lift all 3 dirty instructions cleanly:\n{relaxed_src}"
        );
        assert!(
            relaxed_src.contains("if grab()") || relaxed_src.contains("if !grab()"),
            "{relaxed_src}"
        );
        assert!(
            relaxed_src.contains("refresh(target: 0, mode: 1);"),
            "{relaxed_src}"
        );
        assert!(relaxed_src.contains("my.attr[0] := 3;"), "{relaxed_src}");
        let re_relaxed = &compile_antics(&relaxed_src).unwrap().behaviors[&4096];
        let equiv = check_bhav_equivalent(&orig, re_relaxed, &EquivOptions::default());
        assert!(
            matches!(equiv, EquivResult::Equivalent { .. }),
            "expected Equivalent under --relax-padding, got {equiv:?}\nRelaxed:\n{relaxed_src}"
        );
    }

    #[test]
    fn extended_inline_call_args_protected_in_relaxed_labeled_and_structured_lift() {
        use crate::encode::encode_bhav;
        use crate::lift::format_bhav_labeled_air_with_arities;
        use crate::verify::check_bhav_equivalent_with_arities;

        // Node 0 calls private BHAV #4097 (7 args => needs 14 bytes: 8 from node 0 + 6 from node 1,
        // meaning bytes 0..2 of node 1's `notify_idle` operands carry argument 7!).
        // Node 1 is an unreachable `notify_idle` record with dirty byte 0 = 0x77 (arg 7 low byte).
        let orig = BhavChunk {
            id: 4096,
            label: "ExtendedArgs7".to_owned(),
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 0,
            revision: 0,
            instructions: vec![
                Instruction {
                    opcode: 4097,
                    on_true: 0xFE,
                    on_false: 0xFF,
                    operands: [1, 0, 2, 0, 3, 0, 4, 0],
                },
                Instruction {
                    opcode: 49, // NotifyIdle, unreachable as code, carries args 5..7 bytes!
                    on_true: 0xFE,
                    on_false: 0xFF,
                    operands: [0x77, 0x00, 0, 0, 0, 0, 0, 0],
                },
            ],
            trailing: Vec::new(),
        };
        let arities_7 = BTreeMap::from([(4097u16, 7usize)]);
        let arities_4 = BTreeMap::from([(4097u16, 4usize)]);
        let relaxed = LiftOptions {
            strict_padding: false,
            annotate_comments: false,
        };

        // 1. Structured lifter rejects reordering when callee takes >4 args OR when callee
        //    arity is unknown and an unreachable record immediately follows the call.
        assert_eq!(
            lift_bhav_structured_with_arities(&orig, relaxed, Some(&arities_7)),
            StructuredLiftOutcome::UnsupportedHeaderOrTarget
        );
        assert_eq!(
            lift_bhav_structured_with_arities(&orig, relaxed, None),
            StructuredLiftOutcome::UnsupportedHeaderOrTarget
        );

        // 1b. Even when node 1 IS reachable (node 0 branches directly to node 1!), unknown-arity
        //     structured lifting must still fall back so neither branch targets nor operands of
        //     node 1 (which supply args 5-7!) can be rewritten.
        let reachable_ext = BhavChunk {
            instructions: vec![
                Instruction {
                    opcode: 4097,
                    on_true: 1,
                    on_false: 0xFF,
                    operands: [1, 0, 2, 0, 3, 0, 4, 0],
                },
                Instruction {
                    opcode: 49, // NotifyIdle, REACHABLE as code AND carries args 5..7 bytes!
                    on_true: 0xFE,
                    on_false: 0xFD,
                    operands: [0x77, 0x00, 0, 0, 0, 0, 0, 0],
                },
            ],
            ..orig.clone()
        };
        assert_eq!(
            lift_bhav_structured_with_arities(&reachable_ext, LiftOptions::default(), None),
            StructuredLiftOutcome::UnsupportedHeaderOrTarget
        );
        assert_eq!(
            lift_bhav_structured_with_arities(&reachable_ext, relaxed, None),
            StructuredLiftOutcome::UnsupportedHeaderOrTarget
        );

        // 2. Relaxed labeled-CFG preserves node 1's byte 0 (`0x77`) BOTH when arity is 7
        //    AND when arity is unknown (`None`), so arg 7 is NEVER zeroed!
        for maybe_arities in [Some(&arities_7), None] {
            let labeled = format_bhav_labeled_air_with_arities(&orig, relaxed, maybe_arities);
            let re = &compile_antics(&labeled).unwrap().behaviors[&4096];
            assert_eq!(
                encode_bhav(re).unwrap(),
                encode_bhav(&orig).unwrap(),
                "extended call argument record must stay byte-exact under --relax-padding:\n{labeled}"
            );
            let equiv = check_bhav_equivalent_with_arities(
                &orig,
                re,
                &EquivOptions::default(),
                Some(&arities_7),
            );
            assert!(
                matches!(equiv, EquivResult::Equivalent { .. }),
                "expected Equivalent for extended-arg labeled CFG, got {equiv:?}"
            );
        }

        // 3. When callee is known to take <=4 args, relaxed labeled-CFG is free to clean node 1!
        let cleaned = format_bhav_labeled_air_with_arities(&orig, relaxed, Some(&arities_4));
        assert!(
            cleaned.contains("notify_idle()"),
            "4-arg callee should allow relaxing unreachable notify_idle:\n{cleaned}"
        );

        // 4. CreateObject mode 8 with local byte 128 (`-128` signed) must NOT decode to `local: 128`
        //    and lowerer must reject `create_object(..., mode: 8, local: 128)`.
        let co_signed = BhavChunk {
            id: 4096,
            label: "CreateObjSignedLocal".to_owned(),
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 200,
            revision: 0,
            instructions: vec![Instruction {
                opcode: 42,
                on_true: 0xFE,
                on_false: 0xFF,
                operands: [0x11, 0x22, 0x33, 0x44, 8, 0, 128, 0],
            }],
            trailing: Vec::new(),
        };
        let co_lifted = match lift_bhav_structured_with_options(&co_signed, relaxed) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            co_lifted.contains("op 0x002a"),
            "CreateObject mode 8 local byte 128 (signed -128) must stay raw op:\n{co_lifted}"
        );
        assert!(
            compile_antics(
                r#"tree 4096 "BadCo" (format: 0x8002, locals: 200) { create_object(0x11223344, mode: 8, flags: 0, local: 128); return true; }"#
            )
            .is_err(),
            "lowerer must reject create_object mode 8 with local >= 128"
        );
    }

    #[test]
    fn wave2_all_15_remaining_primitives_roundtrip_strict_relaxed_and_equiv() {
        use crate::encode::encode_bhav;
        use crate::lift::format_bhav_labeled_air;

        let src = r#"
            tree 4096 "Wave2All15Primitives" (args: 2, locals: 3) {
                if find_best_interaction() {
                    if gosub_found_action() {
                        if burn(mode: 1, flags: 1) {
                            if tutorial(0) {
                                tutorial(1);
                            }
                        }
                    }
                }
                get_distance_to(temp[2]);
                get_distance_to(temp[3], from: stack_obj);
                get_direction_to(temp[4]);
                get_direction_to(local[0], from: my.data[11]);
                breakpoint(0);
                breakpoint(my.attr[0]);
                if make_new_character(skin: 0, age: 1, sex: 2, species: 1) {
                    show_string(table: 300, index: 1);
                    look_towards(0);
                    if look_towards(2) {
                        old_relationship(write: true, column: 0, param: 0, dir: 0, flags: 0);
                        if old_relationship(write: false, column: 1, param: 1, dir: 2, flags: 3) {
                            find_worst_motives(target: 0, mode: 1);
                            ui_effect(mode: 0, target: 42, flags: 1);
                            ui_effect(mode: 3);
                            special_effect(duration: 30, kind: 1, size: 2, flags: 0, str_index: 1);
                            special_effect(duration: 1, kind: 0, size: 0, flags: 32, str_index: 0);
                            if reach(mode: 0, moves: true) {
                                reach(mode: 1, moves: true, param: 1);
                            }
                        }
                    }
                }
                return true;
            }
        "#;
        let orig = &compile_antics(src).expect("compile wave2 src").behaviors[&4096];

        // 1. Structured lift under strict padding must emit ZERO raw ops and prove Equivalent.
        let structured = match lift_bhav_structured_with_options(orig, LiftOptions::default()) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            !structured.contains("op 0x"),
            "all Wave 2 primitives must lift with zero raw ops:\n{structured}"
        );
        for expected in [
            "find_best_interaction()",
            "gosub_found_action()",
            "burn(mode: 1, flags: 1)",
            "tutorial(0)",
            "tutorial(1);",
            "get_distance_to(temp[2]);",
            "get_distance_to(temp[3], from: stack_obj);",
            "get_direction_to(temp[4]);",
            "get_direction_to(local[0], from: my.data[11]);",
            "breakpoint(0);",
            "breakpoint(my.attr[0]);",
            "make_new_character(skin: 0, age: 1, sex: 2, species: 1)",
            "show_string(table: 300, index: 1);",
            "look_towards(0);",
            "look_towards(2)",
            "old_relationship(write: true, column: 0, param: 0, dir: 0, flags: 0);",
            "old_relationship(write: false, column: 1, param: 1, dir: 2, flags: 3)",
            "find_worst_motives(target: 0, mode: 1);",
            "ui_effect(mode: 0, target: 42, flags: 1);",
            "ui_effect(mode: 3);",
            "special_effect(duration: 30, kind: 1, size: 2, flags: 0, str_index: 1);",
            "special_effect(duration: 1, kind: 0, size: 0, flags: 32, str_index: 0);",
            "reach(mode: 0, moves: true)",
            "reach(mode: 1, moves: true, param: 1);",
        ] {
            assert!(
                structured.contains(expected),
                "missing `{expected}` in Wave 2 structured lift:\n{structured}"
            );
        }
        let recompiled = &compile_antics(&structured)
            .expect("recompile Wave 2 structured")
            .behaviors[&4096];
        let equiv = check_bhav_equivalent(orig, recompiled, &EquivOptions::default());
        assert!(
            matches!(equiv, EquivResult::Equivalent { .. }),
            "expected Equivalent, got {equiv:?}\nStructured:\n{structured}"
        );

        // 2. Decoded labeled-CFG must emit ZERO raw ops and round-trip BYTE-EXACTLY.
        let labeled = format_bhav_labeled_air(orig, LiftOptions::default());
        assert!(
            !labeled.contains("op 0x"),
            "decoded labeled CFG should decode all 15 Wave 2 primitives:\n{labeled}"
        );
        let re_labeled = &compile_antics(&labeled)
            .expect("recompile Wave 2 labeled")
            .behaviors[&4096];
        assert_eq!(
            encode_bhav(re_labeled).unwrap(),
            encode_bhav(orig).unwrap(),
            "decoded labeled-CFG must round-trip byte-for-byte"
        );

        // 3. Dirty padding on Wave 2 primitives: strict preserves via inline raw op,
        //    relaxed strips verified-ignored bytes and still proves Equivalent!
        let dirty = BhavChunk {
            id: 4096,
            label: "DirtyWave2".to_owned(),
            format: 0x8002,
            kind: 0,
            args: 1,
            locals: 0,
            revision: 0,
            instructions: vec![
                Instruction {
                    // Burn with mode=2 (treated as 0) and dirty byte 7=0xAA
                    opcode: 9,
                    on_true: 1,
                    on_false: 0xFF,
                    operands: [2, 0xFE, 0, 0, 0, 0, 0, 0xAA],
                },
                Instruction {
                    // GetDistanceTo temp[0] with flags=0 and dirty bytes 3..8
                    opcode: 11,
                    on_true: 2,
                    on_false: 0xFD,
                    operands: [0, 0, 0, 0x11, 0x22, 0x33, 0x44, 0x55],
                },
                Instruction {
                    // FindWorstMotives with dirty bytes 0..4
                    opcode: 33,
                    on_true: 3,
                    on_false: 0xFD,
                    operands: [0xDE, 0xAD, 0xBE, 0xEF, 0, 0, 1, 0],
                },
                Instruction {
                    // Reach mode 0, moves=5 (nonzero), dirty slot_param & tail
                    opcode: 47,
                    on_true: 0xFE,
                    on_false: 0xFF,
                    operands: [0, 0, 5, 0, 0x77, 0x88, 0x99, 0xAA],
                },
            ],
            trailing: Vec::new(),
        };
        let strict_dirty = match lift_bhav_structured_with_options(&dirty, LiftOptions::default()) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        // Strict lifting spells every dirty byte with a `pad` clause, byte for byte.
        assert!(!strict_dirty.contains("op 0x"), "{strict_dirty}");
        assert_eq!(strict_dirty.matches(" pad").count(), 4, "{strict_dirty}");
        let labeled_dirty = crate::lift::format_bhav_labeled_air(&dirty, LiftOptions::default());
        assert!(!labeled_dirty.contains("op 0x"), "{labeled_dirty}");
        let re_labeled_dirty = &compile_antics(&labeled_dirty).unwrap().behaviors[&4096];
        assert_eq!(
            crate::encode::encode_bhav(re_labeled_dirty).unwrap(),
            crate::encode::encode_bhav(&dirty).unwrap(),
            "{labeled_dirty}"
        );

        let relaxed = LiftOptions {
            strict_padding: false,
            annotate_comments: false,
        };
        let relaxed_dirty = match lift_bhav_structured_with_options(&dirty, relaxed) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            !relaxed_dirty.contains("op 0x"),
            "relaxed Wave 2 dirty padding must decode cleanly:\n{relaxed_dirty}"
        );
        assert!(
            relaxed_dirty.contains("burn(mode: 0, flags: 0)"),
            "{relaxed_dirty}"
        );
        assert!(
            relaxed_dirty.contains("get_distance_to(temp[0]);"),
            "{relaxed_dirty}"
        );
        assert!(
            relaxed_dirty.contains("find_worst_motives(target: 0, mode: 1);"),
            "{relaxed_dirty}"
        );
        assert!(
            relaxed_dirty.contains("reach(mode: 0, moves: true)"),
            "{relaxed_dirty}"
        );
        let re_dirty = &compile_antics(&relaxed_dirty).unwrap().behaviors[&4096];
        let eq_dirty = check_bhav_equivalent(&dirty, re_dirty, &EquivOptions::default());
        assert!(
            matches!(eq_dirty, EquivResult::Equivalent { .. }),
            "expected Equivalent for relaxed dirty Wave 2, got {eq_dirty:?}\n{relaxed_dirty}"
        );
    }

    #[test]
    fn loop_structuring_handles_while_true_break_continue_infinite_main_and_global34() {
        // 1. Statement-headed natural loop with both `continue` and `break` + post-loop tail.
        //    Must lower with ZERO extra nodes (exact node count parity) and lift cleanly to `while true`.
        let stmt_loop_src = r#"
            tree 4096 "StmtLoop" (args: 1, locals: 0) {
                while true {
                    sleep(param[0]);
                    if my.attr[0] == 1 {
                        continue;
                    }
                    if my.attr[0] == 2 {
                        break;
                    }
                    my.attr[1] += 1;
                }
                my.attr[2] := 9;
                return true;
            }
        "#;
        let orig = &compile_antics(stmt_loop_src).unwrap().behaviors[&4096];
        assert_eq!(
            orig.instructions.len(),
            5,
            "while true must emit zero dummy header nodes (expected 5 instructions)"
        );
        let lifted = match lift_bhav_structured(orig) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(lifted.contains("while true {"), "{lifted}");
        assert!(lifted.contains("continue;"), "{lifted}");
        assert!(lifted.contains("break;"), "{lifted}");
        let recompiled = &compile_antics(&lifted).unwrap().behaviors[&4096];
        assert_eq!(recompiled.instructions.len(), orig.instructions.len());
        assert!(matches!(
            check_bhav_equivalent(orig, recompiled, &EquivOptions::default()),
            EquivResult::Equivalent { .. }
        ));

        // 2. Infinite `Main` loop with multiple sequential `if`/`else` diamonds (no `return`).
        let main_loop_src = r#"
            tree 4097 "InfiniteMain" (args: 1, locals: 0) {
                while true {
                    if my.attr[0] == 0 {
                        my.attr[1] := 1;
                    } else {
                        my.attr[1] := 2;
                    }
                    if my.attr[2] == 0 {
                        my.attr[3] := 1;
                    } else {
                        my.attr[3] := 2;
                    }
                    sleep(param[0]);
                }
            }
        "#;
        let main_orig = &compile_antics(main_loop_src).unwrap().behaviors[&4097];
        let main_lifted = match lift_bhav_structured(main_orig) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted for infinite Main loop, got {other:?}"),
        };
        assert!(main_lifted.contains("while true {"), "{main_lifted}");
        // Loop-DAG post-dominators must prevent duplicating `sleep(param[0])`!
        assert_eq!(
            main_lifted.matches("sleep(param[0]);").count(),
            1,
            "infinite Main loop diamonds must merge cleanly without duplicating tail:\n{main_lifted}"
        );
        let main_re = &compile_antics(&main_lifted).unwrap().behaviors[&4097];
        assert!(matches!(
            check_bhav_equivalent(main_orig, main_re, &EquivOptions::default()),
            EquivResult::Equivalent { .. }
        ));

        // 3. Global 34 preservation across `while true`: zero dummy `0 == 0` node means
        //    even a loop touching `ui_effect` / `global[34]` stays 100% Equivalent!
        let g34_src = r#"
            tree 4098 "Global34Loop" (args: 0, locals: 0) {
                while true {
                    ui_effect(mode: 0, target: global[34], flags: 1);
                    if my.attr[0] == 1 {
                        break;
                    }
                }
                return true;
            }
        "#;
        let g34_orig = &compile_antics(g34_src).unwrap().behaviors[&4098];
        let g34_lifted = match lift_bhav_structured(g34_orig) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        let g34_re = &compile_antics(&g34_lifted).unwrap().behaviors[&4098];
        assert!(matches!(
            check_bhav_equivalent(g34_orig, g34_re, &EquivOptions::default()),
            EquivResult::Equivalent { .. }
        ));
    }

    #[test]
    fn symbol_comments_annotate_calls_bcon_strings_and_vm_fields_and_roundtrip_equiv() {
        use crate::encode::{encode_bcon, encode_bhav, encode_string_table};
        use crate::ir::{BconChunk, StringEntry, StringFormat, StringTableChunk};
        use crate::lift::format_bhav_labeled_air_with_context;
        use sims_formats::iff::{ChunkData, Iff, write as write_iff};

        let caller_src = r#"
            tree 4096 "Caller" (args: 0, locals: 0) {
                my.motive[7] += bcon[1];
                if my.person[58] < 18 {
                    my.data[25] := 0;
                    action_string(table: 300, index: 1);
                    dialog(kind: 0, message: 1, title: 0, button1: 0, button2: 0, button3: 0, mode4_str: 0, flags: 0);
                    run_tree_by_name(table: 303, index: 1, mode: 2);
                    set_motive_change(5, 10, 100);
                    call 4097(0, 0, 0, 0);
                }
                return true;
            }
            tree 4097 "open" (args: 0, locals: 0) {
                return true;
            }
        "#;
        let compiled = compile_antics(caller_src).unwrap();
        let caller = &compiled.behaviors[&4096];
        let callee = &compiled.behaviors[&4097];

        let bcon = BconChunk {
            id: 4096,
            label: "Tuning".to_owned(),
            flags: 0,
            values: vec![10, 25, 50],
            trailing: Vec::new(),
        };
        let str300 = StringTableChunk::new(
            *b"STR#",
            300,
            "ActionStrings",
            StringFormat::Localized,
            vec![StringEntry::english("Grab Snack")],
        );
        let str301 = StringTableChunk::new(
            *b"STR#",
            301,
            "Dialogs",
            StringFormat::Localized,
            vec![StringEntry::english("Fridge empty!\nRefill soon.")],
        );
        let str303 = StringTableChunk::new(
            *b"STR#",
            303,
            "TreeNames",
            StringFormat::Localized,
            vec![StringEntry::english("CT - Open")],
        );

        let iff_bytes = write_iff(&[
            ChunkData {
                tag: *b"BHAV",
                id: 4096,
                flags: 0,
                label: caller.label.clone(),
                data: encode_bhav(caller).unwrap(),
            },
            ChunkData {
                tag: *b"BHAV",
                id: 4097,
                flags: 0,
                label: callee.label.clone(),
                data: encode_bhav(callee).unwrap(),
            },
            ChunkData {
                tag: *b"BCON",
                id: 4096,
                flags: 0,
                label: bcon.label.clone(),
                data: encode_bcon(&bcon).unwrap(),
            },
            ChunkData {
                tag: *b"STR#",
                id: 300,
                flags: 0,
                label: str300.label.clone(),
                data: encode_string_table(&str300).unwrap(),
            },
            ChunkData {
                tag: *b"STR#",
                id: 301,
                flags: 0,
                label: str301.label.clone(),
                data: encode_string_table(&str301).unwrap(),
            },
            ChunkData {
                tag: *b"STR#",
                id: 303,
                flags: 0,
                label: str303.label.clone(),
                data: encode_string_table(&str303).unwrap(),
            },
        ])
        .unwrap();
        let iff = Iff::parse(iff_bytes).unwrap();
        let symbols = extract_lift_symbols_from_iff(&iff);
        let arities = BTreeMap::from([(4096u16, 0usize), (4097u16, 0usize)]);

        // Annotated structured lift:
        let annotated_opts = LiftOptions {
            strict_padding: true,
            annotate_comments: true,
        };
        let annotated = match lift_bhav_structured_with_context(
            caller,
            annotated_opts,
            Some(&arities),
            Some(&symbols),
        ) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        for expected_comment in [
            "// Hunger, bcon[0:1] = 25 (\"Tuning\")",
            "// Age",
            "// Lockout",
            "// \"Grab Snack\"",
            "// \"Fridge empty! Refill soon.\"",
            "// \"CT - Open\"",
            "// Energy",
            "// \"open\"",
        ] {
            assert!(
                annotated.contains(expected_comment),
                "missing `{expected_comment}` in annotated structured lift:\n{annotated}"
            );
        }
        // Annotated `.air` must compile cleanly and prove Equivalent!
        let re_annotated = &compile_antics(&annotated)
            .expect("annotated structured .air must compile")
            .behaviors[&4096];
        assert!(matches!(
            check_bhav_equivalent(caller, re_annotated, &EquivOptions::default()),
            EquivResult::Equivalent { .. }
        ));

        // Annotated labeled-CFG lift must ALSO compile cleanly and round-trip byte-for-byte!
        let labeled_ann = format_bhav_labeled_air_with_context(
            caller,
            annotated_opts,
            Some(&arities),
            Some(&symbols),
        );
        assert!(labeled_ann.contains("// \"open\""), "{labeled_ann}");
        assert!(labeled_ann.contains("// Hunger"), "{labeled_ann}");
        let re_labeled = &compile_antics(&labeled_ann)
            .expect("annotated labeled-CFG .air must compile")
            .behaviors[&4096];
        assert_eq!(
            encode_bhav(re_labeled).unwrap(),
            encode_bhav(caller).unwrap()
        );

        // Default (`annotate_comments: false`) emits zero trailing comments!
        let plain = match lift_bhav_structured_with_context(
            caller,
            LiftOptions::default(),
            Some(&arities),
            Some(&symbols),
        ) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            !plain.contains("//"),
            "plain lift must emit zero comments:\n{plain}"
        );
    }

    #[test]
    fn sol_wave2_r1_zero_instruction_while_true_and_localized_str_order_regressions() {
        use crate::compile::compile_antics;
        use crate::encode::{encode_bhav, encode_string_table};
        use crate::ir::{StringEntry, StringFormat, StringTableChunk};
        use sims_formats::iff::{ChunkData, Iff, write as write_iff};

        // P1-1a: `refresh(0, 0); while true { return false; } refresh(0, 1);`
        // First refresh must jump directly to TARGET_FALSE and never emit/jump to refresh(0, 1).
        let t1 = &compile_antics(
            r#"
            tree 4096 "T1" {
                refresh(target: 0, mode: 0);
                while true { return false; }
                refresh(target: 0, mode: 1);
            }
            "#,
        )
        .expect("compile T1")
        .behaviors[&4096];
        assert_eq!(t1.instructions.len(), 1);
        assert_eq!(t1.instructions[0].opcode, 7);
        assert_eq!(t1.instructions[0].on_true, TARGET_FALSE);
        assert_eq!(t1.instructions[0].on_false, TARGET_FALSE);

        // P1-1b: `refresh(0, 0); while true { return false; }` (no trailing stmt) must not point OOB.
        let t2 = &compile_antics(
            r#"
            tree 4096 "T2" {
                refresh(target: 0, mode: 0);
                while true { return false; }
            }
            "#,
        )
        .expect("compile T2")
        .behaviors[&4096];
        assert_eq!(t2.instructions.len(), 1);
        assert_eq!(t2.instructions[0].on_true, TARGET_FALSE);
        assert_eq!(t2.instructions[0].on_false, TARGET_FALSE);

        // P1-1c: `while true { return false; } refresh(0, 0);` must return false, not run refresh.
        let t3 = &compile_antics(
            r#"
            tree 4096 "T3" {
                while true { return false; }
                refresh(target: 0, mode: 0);
            }
            "#,
        )
        .expect("compile T3")
        .behaviors[&4096];
        assert_eq!(t3.instructions.len(), 1);
        assert_eq!(t3.instructions[0].opcode, 2);
        assert_eq!(t3.instructions[0].on_true, TARGET_FALSE);
        assert_eq!(t3.instructions[0].on_false, TARGET_FALSE);

        // P1-1d: Nested `while true { while true { break; } refresh(0, 0); }` must patch outer
        // back-edge cleanly to node 0 with zero unpatched sentinels!
        let t4 = &compile_antics(
            r#"
            tree 4096 "T4" {
                while true {
                    while true { break; }
                    refresh(target: 0, mode: 0);
                }
            }
            "#,
        )
        .expect("compile T4")
        .behaviors[&4096];
        assert_eq!(t4.instructions.len(), 1);
        assert_eq!(t4.instructions[0].opcode, 7);
        assert_eq!(t4.instructions[0].on_true, 0);
        assert_eq!(t4.instructions[0].on_false, 0);

        // P1-1e: `while true { while true { break; } }` must emit a clean 1-node self-loop.
        let t5 = &compile_antics(
            r#"
            tree 4096 "T5" {
                while true {
                    while true { break; }
                }
            }
            "#,
        )
        .expect("compile T5")
        .behaviors[&4096];
        assert_eq!(t5.instructions.len(), 1);
        assert_eq!(t5.instructions[0].on_true, 0);
        assert_eq!(t5.instructions[0].on_false, 0);

        // P1-1f: `refresh(0, 0); while true { break; } return true;` jumps directly to TARGET_TRUE.
        let t6 = &compile_antics(
            r#"
            tree 4096 "T6" {
                refresh(target: 0, mode: 0);
                while true { break; }
                return true;
            }
            "#,
        )
        .expect("compile T6")
        .behaviors[&4096];
        assert_eq!(t6.instructions.len(), 1);
        assert_eq!(t6.instructions[0].on_true, TARGET_TRUE);

        // P2-2: Localized STR# with interleaved English/French/English matches VM `loaded_order`
        // (`Last` before `First`), and French-only localized STR# produces NO preview comment.
        let str_interleaved = StringTableChunk::new(
            *b"STR#",
            300,
            "Interleaved",
            StringFormat::Localized,
            vec![
                StringEntry::english("First"),
                StringEntry {
                    language: 2,
                    text: "Autre".to_owned(),
                    comment: String::new(),
                    raw_text: None,
                    raw_comment: None,
                },
                StringEntry::english("Last"),
            ],
        );
        let str_french_only = StringTableChunk::new(
            *b"STR#",
            301,
            "FrenchOnly",
            StringFormat::Localized,
            vec![StringEntry {
                language: 2,
                text: "Seulement".to_owned(),
                comment: String::new(),
                raw_text: None,
                raw_comment: None,
            }],
        );
        let probe = &compile_antics(
            r#"
            tree 4096 "Probe" {
                action_string(table: 300, index: 1);
                action_string(table: 300, index: 2);
                dialog(kind: 0, message: 1, title: 0, button1: 0, button2: 0, button3: 0, mode4_str: 0, flags: 0);
                return true;
            }
            "#,
        )
        .unwrap()
        .behaviors[&4096];
        let iff_bytes = write_iff(&[
            ChunkData {
                tag: *b"BHAV",
                id: 4096,
                flags: 0,
                label: "Probe".to_owned(),
                data: encode_bhav(probe).unwrap(),
            },
            ChunkData {
                tag: *b"STR#",
                id: 300,
                flags: 0,
                label: "Interleaved".to_owned(),
                data: encode_string_table(&str_interleaved).unwrap(),
            },
            ChunkData {
                tag: *b"STR#",
                id: 301,
                flags: 0,
                label: "FrenchOnly".to_owned(),
                data: encode_string_table(&str_french_only).unwrap(),
            },
        ])
        .unwrap();
        let iff = Iff::parse(iff_bytes).unwrap();
        let symbols = extract_lift_symbols_from_iff(&iff);
        let lifted = match lift_bhav_structured_with_context(
            probe,
            LiftOptions {
                strict_padding: true,
                annotate_comments: true,
            },
            None,
            Some(&symbols),
        ) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            lifted.contains("action_string(table: 300, index: 1); // \"Last\""),
            "interleaved localized STR# index 1 must preview \"Last\" matching VM loaded_order:\n{lifted}"
        );
        assert!(
            lifted.contains("action_string(table: 300, index: 2); // \"First\""),
            "interleaved localized STR# index 2 must preview \"First\" matching VM loaded_order:\n{lifted}"
        );
        assert!(
            !lifted.contains("Seulement"),
            "French-only localized STR# must not emit a misleading preview:\n{lifted}"
        );

        // Sol R2 P1-1: Trailing break-only loop in a labeled tree must fall through to TARGET_TRUE
        // without leaving a dangling edge past the end of the tree.
        let labeled_trail = &compile_antics(
            r#"
            tree 4096 "LabeledTrail" {
                @start:
                refresh(target: 0, mode: 0);
                while true { break; }
            }
            "#,
        )
        .expect("labeled trailing break-only loop must compile")
        .behaviors[&4096];
        assert_eq!(labeled_trail.instructions.len(), 1);
        assert_eq!(labeled_trail.instructions[0].opcode, 7);
        assert_eq!(labeled_trail.instructions[0].on_true, TARGET_TRUE);
        assert_eq!(labeled_trail.instructions[0].on_false, TARGET_TRUE);

        let labeled_mid = &compile_antics(
            r#"
            tree 4096 "LabeledMid" {
                @start:
                refresh(target: 0, mode: 0);
                while true { break; }
                refresh(target: 0, mode: 1);
            }
            "#,
        )
        .expect("labeled mid break-only loop must compile")
        .behaviors[&4096];
        assert_eq!(labeled_mid.instructions.len(), 2);
        assert_eq!(labeled_mid.instructions[0].on_true, 1);
        assert_eq!(labeled_mid.instructions[0].on_false, 1);
        assert_eq!(labeled_mid.instructions[1].on_true, TARGET_TRUE);

        // Sol R2 P2-1: Non-fallthrough cutoff must still report semantic diagnostics in dead code.
        let dead_err = compile_antics(
            r#"
            tree 4096 "DeadErr" {
                while true { return false; }
                call Missing();
            }
            "#,
        )
        .expect_err("dead `call Missing()` after `while true { return false; }` must still error");
        assert!(
            dead_err.message.contains("Missing"),
            "unexpected diagnostic: {}",
            dead_err.message
        );
    }
}
