//! In-memory structured lifter from raw `BHAV` bytecode into structured
//! `.air` (Antics IR) source text (`if`/`else`, `while`, `return`, named
//! `Expression` assignments/comparisons, readable primitives, `call`, and
//! per-instruction inline `op 0xNNNN (...)` fallbacks for unmodeled opcodes
//! or dirty Edith padding bytes).
//!
//! Strictly in-memory: never writes any lifted representation to disk.

use crate::ir::BhavChunk;
use crate::lower::{TARGET_ERROR, TARGET_FALSE, TARGET_TRUE};
use crate::verify::is_always_true_instruction;
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
}

impl Default for LiftOptions {
    fn default() -> Self {
        Self {
            strict_padding: true,
        }
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
    Stmt { text: String, next: u8 },
    /// Branching high-level condition (used in `if` / `while`).
    Cond {
        /// Condition text (e.g. `my.attr[0] == 0`, `call 4098(0, 0, 0, 0)`, `play_sound(...)`).
        text: String,
        /// Optional statement form when `on_true == on_false` (or `call` with `on_false == 0xFF`).
        stmt_form: Option<String>,
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
    let ops = &ins.operands;
    let opcode = ins.opcode;
    let strict = opts.strict_padding;
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
            Some(DecodedIns::AlwaysTrue {
                structured_stmt: format!("{head};"),
                labeled_head: head,
            })
        }
        1 => {
            if strict && ops[2..8] != [0; 6] {
                return None;
            }
            let routine = word_u16(ops, 0);
            let expr = format!("generic_call({routine})");
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
        4 => {
            if strict && *ops != [0; 8] {
                return None;
            }
            let expr = "grab()".to_owned();
            Some(DecodedIns::Branching {
                stmt_form: Some("grab();".to_owned()),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        5 => {
            if strict && *ops != [0; 8] {
                return None;
            }
            let expr = "drop()".to_owned();
            Some(DecodedIns::Branching {
                stmt_form: Some("drop();".to_owned()),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            Some(DecodedIns::AlwaysTrue {
                structured_stmt: format!("{head};"),
                labeled_head: head,
            })
        }
        7 => {
            if strict && ops[4..8] != [0; 4] {
                return None;
            }
            let target = word_u16(ops, 0);
            let mode = word_u16(ops, 2);
            let head = format!("refresh(target: {target}, mode: {mode})");
            Some(DecodedIns::AlwaysTrue {
                structured_stmt: format!("{head};"),
                labeled_head: head,
            })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        13 => {
            if strict && ops[5..8] != [0; 3] {
                return None;
            }
            let menu = ops[0];
            let object_slot = ops[1];
            let priority = ops[2];
            let flags = if strict { ops[3] } else { ops[3] & 0x0F };
            let icon = if strict || flags & 1 != 0 { ops[4] } else { 0 };
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        16 => {
            if strict && ops[3..8] != [0; 5] {
                return None;
            }
            let mode = ops[0];
            let flags = if strict { ops[2] } else { ops[2] & 7 };
            let local = if strict || flags & 1 != 0 { ops[1] } else { 0 };
            if flags & 1 != 0 && usize::from(local) >= max_locals {
                return None;
            }
            let mut expr = format!("find_location(mode: {mode}, flags: {flags}");
            if local != 0 {
                let _ = write!(expr, ", local: {local}");
            }
            expr.push(')');
            if mode == 1 {
                Some(DecodedIns::AlwaysTrue {
                    structured_stmt: format!("{expr};"),
                    labeled_head: expr,
                })
            } else {
                Some(DecodedIns::Branching {
                    stmt_form: Some(format!("{expr};")),
                    labeled_head: expr.clone(),
                    cond_text: expr,
                    is_call: false,
                })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        20 => {
            if strict && (ops[2] > 1 || ops[3..8] != [0; 5]) {
                return None;
            }
            let func = word_u16(ops, 0);
            let change_icon = ops[2] & 1 != 0;
            let expr = format!("run_functional_tree({func}, change_icon: {change_icon})");
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        23 => {
            if strict && (ops[2..4] != [0; 2] || ops[5..8] != [0; 3]) {
                return None;
            }
            let sound_id = word_u16(ops, 0);
            let head = if strict {
                match ops[4] {
                    0 => format!("play_sound({sound_id})"),
                    2 => format!("play_sound({sound_id}, stack_obj)"),
                    f => format!("play_sound({sound_id}, flags: {f})"),
                }
            } else {
                let use_stack = ops[4] != 0xFF && ops[4] & 2 != 0;
                if use_stack {
                    format!("play_sound({sound_id}, stack_obj)")
                } else {
                    format!("play_sound({sound_id})")
                }
            };
            Some(DecodedIns::AlwaysTrue {
                structured_stmt: format!("{head};"),
                labeled_head: head,
            })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        26 => {
            if strict && ops[5] != 0 {
                return None;
            }
            let col = ops[0];
            let dir = ops[1];
            let flags = if strict { ops[2] } else { ops[2] & 7 };
            let local = if strict || matches!(dir, 2 | 3) {
                ops[3]
            } else {
                0
            };
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
                Some(DecodedIns::AlwaysTrue {
                    structured_stmt: format!("{expr};"),
                    labeled_head: expr,
                })
            } else {
                Some(DecodedIns::Branching {
                    stmt_form: Some(format!("{expr};")),
                    labeled_head: expr.clone(),
                    cond_text: expr,
                    is_call: false,
                })
            }
        }
        27 => {
            if strict && (ops[0] != 0 || ops[1] != 0 || ops[4] != 0 || ops[5] != 0 || ops[7] != 0) {
                return None;
            }
            let location = ops[2] as i8;
            let facing = ops[3];
            let options = if strict { ops[6] } else { ops[6] & 6 };
            let expr = format!(
                "go_to_relative(location: {location}, facing: {facing}, options: {options})"
            );
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        29 => {
            if ops[3] & 1 != 0 {
                if strict && *ops != [0, 0, 0, 1, 0, 0, 0, 0] {
                    return None;
                }
                let head = "clear_motive_changes()".to_owned();
                Some(DecodedIns::AlwaysTrue {
                    structured_stmt: "clear_motive_changes();".to_owned(),
                    labeled_head: head,
                })
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
                Some(DecodedIns::AlwaysTrue {
                    structured_stmt: format!("{head};"),
                    labeled_head: head,
                })
            }
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        32 => {
            if strict && ops[7] != 0 {
                return None;
            }
            let guid = u32::from_le_bytes([ops[0], ops[1], ops[2], ops[3]]);
            let object = format_operand(ops[6], word_i16(ops, 4), max_args, max_locals)?;
            let expr = format!("test_object_type({guid:#010x}, {object})");
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            Some(DecodedIns::AlwaysTrue {
                structured_stmt: format!("{head};"),
                labeled_head: head,
            })
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
            if (mode == 8 || mode == 9) && usize::from(local) >= max_locals {
                return None;
            }
            let expr = format!(
                "create_object({guid:#010x}, mode: {mode}, flags: {flags}, local: {local})"
            );
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
            let flags = if strict { ops[4] } else { ops[4] & 1 };
            let expr = if source == 1 {
                format!("route_to(slot: {slot}, flags: {flags})")
            } else {
                format!("route_to(slot: {slot}, source: {source}, flags: {flags})")
            };
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        46 => {
            let mut slot = word_u16(ops, 0);
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
            if !strict && (mode == 1 || mode == 2) {
                slot = 0;
            }
            let flags = ops[4];
            let expr = if mode == 3 {
                format!("snap({slot}, {flags})")
            } else {
                format!("snap(slot: {slot}, mode: {mode}, flags: {flags})")
            };
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
        }
        48 => {
            let raw_target = word_u16(ops, 0);
            if strict && (raw_target > 1 || ops[2..8] != [0; 6]) {
                return None;
            }
            let use_stack = raw_target != 0;
            let head = format!("stop_all_sounds({use_stack})");
            Some(DecodedIns::AlwaysTrue {
                structured_stmt: format!("{head};"),
                labeled_head: head,
            })
        }
        49 => {
            if strict && *ops != [0; 8] {
                return None;
            }
            let head = "notify_idle()".to_owned();
            Some(DecodedIns::AlwaysTrue {
                structured_stmt: "notify_idle();".to_owned(),
                labeled_head: head,
            })
        }
        50 => {
            if strict && (ops[2] != 0 || ops[3] != 0 || ops[5..8] != [0; 3]) {
                return None;
            }
            let table = word_u16(ops, 0);
            let index = ops[4];
            let head = format!("action_string(table: {table}, index: {index})");
            Some(DecodedIns::AlwaysTrue {
                structured_stmt: format!("{head};"),
                labeled_head: head,
            })
        }
        51 => {
            let op = ops[0];
            let kind = ops[1];
            let flags = word_u16(ops, 2);
            let guid = u32::from_le_bytes([ops[4], ops[5], ops[6], ops[7]]);
            let expr = format!(
                "manage_inventory(op: {op}, kind: {kind}, flags: {flags}, guid: {guid:#010x})"
            );
            Some(DecodedIns::Branching {
                stmt_form: Some(format!("{expr};")),
                labeled_head: expr.clone(),
                cond_text: expr,
                is_call: false,
            })
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
) -> LiftedIns {
    if let Some(decoded) = try_decode_instruction(ins, max_args, max_locals, opts) {
        return match decoded {
            DecodedIns::AlwaysTrue {
                structured_stmt, ..
            } => LiftedIns::Stmt {
                text: structured_stmt,
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
            next: ins.on_true,
        }
    } else {
        LiftedIns::Cond {
            stmt_form: Some(format!("{raw};")),
            text: raw,
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
    let Some((max_args, max_locals)) = format_limits(chunk.format) else {
        return StructuredLiftOutcome::UnsupportedHeaderOrTarget;
    };
    let n = chunk.instructions.len();
    if n > 251 {
        return StructuredLiftOutcome::UnsupportedHeaderOrTarget;
    }
    let eff_args = if chunk.format == 0x8000 {
        chunk.args
    } else {
        usize::from(chunk.args).min(max_args) as u8
    };
    // Match `sims_vm::interpreter::dimensions`: the local count is truncated to a byte (`locals as u8`).
    let eff_locals = usize::from(chunk.locals as u8).min(max_locals) as u16;

    // Indexed-entry trees (kind 4) can enter at caller-selected PCs (`interpreter.rs:1786`),
    // so renumbering or pruning unreachable-from-0 nodes would alter semantics.
    if chunk.kind == 4 {
        return StructuredLiftOutcome::UnsupportedHeaderOrTarget;
    }

    // Header (preserves exact format, kind, effective args/locals, revision with clean placeholder label).
    let header = format!(
        "tree {} \"Tree_{}\" (format: {:#06x}, kind: {}, args: {}, locals: {}, revision: {:#010x}) {{\n",
        chunk.id, chunk.id, chunk.format, chunk.kind, eff_args, eff_locals, chunk.revision
    );

    if n == 0 {
        return StructuredLiftOutcome::Lifted(format!("{header}}}\n"));
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
                    return StructuredLiftOutcome::UnsupportedHeaderOrTarget;
                }
                if !reachable[next] {
                    reachable[next] = true;
                    queue.push_back(next);
                }
            }
        }
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
        let li = lift_instruction(ins, arg_bound, local_bound, opts);
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

    // 4. Recursively emit structured `.antics` statements.
    let mut emitter = StructEmitter {
        lifted: &lifted,
        ipdom: &ipdom,
        loop_bodies: &loop_bodies,
        out: header,
        emitted_nodes: 0,
        max_emitted_nodes: 245, // Stay safely under MAX_BHAV_NODES (251)
    };

    if emitter
        .emit_region(0, None, None, &mut BTreeSet::new(), 1)
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

#[derive(Debug, Clone, Copy)]
struct LoopCtx {
    header: u8,
    exit: u8,
}

struct StructEmitter<'a> {
    lifted: &'a [Option<LiftedIns>],
    ipdom: &'a [Option<usize>],
    loop_bodies: &'a BTreeMap<usize, BTreeSet<usize>>,
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

    fn emit_region(
        &mut self,
        mut cur: u8,
        stop_at: Option<u8>,
        loop_ctx: Option<LoopCtx>,
        active: &mut BTreeSet<u8>,
        depth: usize,
    ) -> std::result::Result<(), ()> {
        if depth > 32 {
            return Err(());
        }
        loop {
            if Some(cur) == stop_at {
                return Ok(());
            }
            if let Some(ctx) = loop_ctx {
                if cur == ctx.header {
                    self.indent(depth);
                    self.out.push_str("continue;\n");
                    return Ok(());
                }
                if cur == ctx.exit {
                    self.indent(depth);
                    self.out.push_str("break;\n");
                    return Ok(());
                }
            }
            if matches!(cur, TARGET_ERROR..=TARGET_FALSE) {
                return self.emit_terminal(cur, depth);
            }

            let idx = usize::from(cur);
            if active.contains(&cur) {
                // Back-edge to an active non-loop-header node -> irreducible CFG.
                return Err(());
            }
            let Some(li) = self.lifted.get(idx).and_then(Option::as_ref) else {
                return Err(());
            };

            // Check if `cur` is a natural `while` loop header.
            if let Some(body_set) = self.loop_bodies.get(&idx)
                && let LiftedIns::Cond {
                    text,
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
                    self.emitted_nodes += 1;
                    if self.emitted_nodes > self.max_emitted_nodes {
                        return Err(());
                    }
                    let (cond_str, body_entry, exit_target) = if true_in {
                        (text.clone(), *on_true, *on_false)
                    } else {
                        (format!("!{text}"), *on_false, *on_true)
                    };
                    self.indent(depth);
                    let _ = writeln!(self.out, "while {cond_str} {{");
                    active.insert(cur);
                    let res = self.emit_region(
                        body_entry,
                        Some(cur),
                        Some(LoopCtx {
                            header: cur,
                            exit: exit_target,
                        }),
                        active,
                        depth + 1,
                    );
                    active.remove(&cur);
                    res?;
                    self.indent(depth);
                    self.out.push_str("}\n");
                    cur = exit_target;
                    continue;
                }
            }

            self.emitted_nodes += 1;
            if self.emitted_nodes > self.max_emitted_nodes {
                return Err(());
            }

            match li {
                LiftedIns::Stmt { text, next } => {
                    self.indent(depth);
                    self.out.push_str(text);
                    self.out.push('\n');
                    active.insert(cur);
                    cur = *next;
                    // Remove after stepping? Wait: keep active on current straight-line path
                    // to guard against unexpected self-loops! We remove on return.
                    // Actually, popping at function exit is cleanest: track local_active!
                    if active.contains(&cur)
                        && Some(cur) != stop_at
                        && loop_ctx.map(|c| c.header) != Some(cur)
                        && loop_ctx.map(|c| c.exit) != Some(cur)
                    {
                        active.remove(&(idx as u8));
                        return Err(());
                    }
                    // Continue sequentially; note we leave `cur` checked in loop.
                    // Clean up `idx` when we finish this region via recursion or guard:
                    // To avoid false cycle detection across sequential diamonds, only keep
                    // `active` for enclosing ancestors!
                    active.remove(&(idx as u8));
                    if cur == idx as u8 {
                        return Err(());
                    }
                }
                LiftedIns::Cond {
                    text,
                    stmt_form,
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
                        self.out.push('\n');
                        if *on_true == cur {
                            return Err(());
                        }
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
                        self.out.push('\n');
                        if *on_true == cur {
                            return Err(());
                        }
                        cur = *on_true;
                        continue;
                    }

                    // Case C: General `if` / `else`, merging at immediate post-dominator if valid.
                    let merge = self
                        .ipdom
                        .get(idx)
                        .and_then(|&m| m)
                        .map(|m| m as u8)
                        .filter(|&m| {
                            Some(m) != stop_at
                                && !active.contains(&m)
                                && loop_ctx.is_none_or(|c| m != c.header && m != c.exit)
                        });
                    let branch_stop = merge.or(stop_at);

                    let true_is_skip = Some(*on_true) == branch_stop;
                    let false_is_skip = Some(*on_false) == branch_stop;

                    active.insert(cur);
                    let res = if true_is_skip && false_is_skip {
                        self.indent(depth);
                        let _ = writeln!(self.out, "if {text} {{");
                        self.indent(depth);
                        self.out.push_str("}\n");
                        Ok(())
                    } else if false_is_skip {
                        self.indent(depth);
                        let _ = writeln!(self.out, "if {text} {{");
                        let r =
                            self.emit_region(*on_true, branch_stop, loop_ctx, active, depth + 1);
                        self.indent(depth);
                        self.out.push_str("}\n");
                        r
                    } else if true_is_skip {
                        self.indent(depth);
                        let _ = writeln!(self.out, "if !{text} {{");
                        let r =
                            self.emit_region(*on_false, branch_stop, loop_ctx, active, depth + 1);
                        self.indent(depth);
                        self.out.push_str("}\n");
                        r
                    } else {
                        self.indent(depth);
                        let _ = writeln!(self.out, "if {text} {{");
                        let r1 =
                            self.emit_region(*on_true, branch_stop, loop_ctx, active, depth + 1);
                        self.indent(depth);
                        self.out.push_str("} else {\n");
                        let r2 = r1.and_then(|()| {
                            self.emit_region(*on_false, branch_stop, loop_ctx, active, depth + 1)
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
        assert!(lifted.contains("if op 0x0044"), "{lifted}");
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

        // Strict padding: preserves dirty bytes via inline `op` while keeping structured `if`!
        let strict_src = match lift_bhav_structured_with_options(
            &orig,
            LiftOptions {
                strict_padding: true,
            },
        ) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(strict_src.contains("if op 0x0004"), "{strict_src}");
        assert!(strict_src.contains("op 0x0007"), "{strict_src}");
        assert!(strict_src.contains("op 0x0002"), "{strict_src}");

        // Labeled CFG under strict padding must round-trip 100% byte-for-byte!
        let strict_labeled = format_bhav_labeled_air(
            &orig,
            LiftOptions {
                strict_padding: true,
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
            },
        ) {
            StructuredLiftOutcome::Lifted(s) => s,
            other => panic!("expected Lifted, got {other:?}"),
        };
        assert!(
            !relaxed_src.contains("op 0x"),
            "relaxed padding should lift all 3 dirty instructions cleanly:\n{relaxed_src}"
        );
        assert!(relaxed_src.contains("if grab()"), "{relaxed_src}");
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
}
