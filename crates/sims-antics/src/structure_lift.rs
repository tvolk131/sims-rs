//! In-memory structured high-level lifter from raw `BHAV` bytecode into
//! structured high-level `.antics` source text (`if`/`else`, `while`, `return`,
//! named `Expression` assignments/comparisons, readable primitives, and `call`),
//! with ZERO raw `op 0xNNNN` instructions.
//!
//! Strictly in-memory: never writes any lifted representation to disk.

use crate::ir::BhavChunk;
use crate::lower::{TARGET_ERROR, TARGET_FALSE, TARGET_TRUE};
use crate::verify::is_always_true_instruction;
use sims_formats::behavior::Instruction;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write;

/// Result of attempting to lift a [`BhavChunk`] into structured high-level `.antics`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuredLiftOutcome {
    /// Cleanly lifted into pure high-level `.antics` (zero raw `op` instructions).
    Lifted(String),
    /// Tree contains a reachable primitive opcode or operand form not yet in the
    /// high-level `.antics` DSL.
    UnsupportedOpcode(u16),
    /// Tree uses an unsupported BHAV format (e.g., `0x8003`), out-of-bounds target,
    /// or negative local/param index.
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

fn lift_instruction(
    ins: &Instruction,
    max_args: usize,
    max_locals: usize,
) -> std::result::Result<LiftedIns, u16> {
    let ops = &ins.operands;
    let opcode = ins.opcode;
    match opcode {
        0 => {
            let p = word_i16(ops, 0);
            if p < 0 || p as usize >= max_args {
                return Err(opcode);
            }
            Ok(LiftedIns::Stmt {
                text: format!("sleep(param[{p}]);"),
                next: ins.on_true,
            })
        }
        1 => {
            let routine = word_u16(ops, 0);
            let expr = format!("generic_call({routine})");
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        2 => {
            let lhs_idx = word_i16(ops, 0);
            let rhs_idx = word_i16(ops, 2);
            let op = ops[5];
            let lhs_scope = ops[6];
            let rhs_scope = ops[7];
            let Some(lhs) = format_operand(lhs_scope, lhs_idx, max_args, max_locals) else {
                return Err(opcode);
            };
            let Some(rhs) = format_operand(rhs_scope, rhs_idx, max_args, max_locals) else {
                return Err(opcode);
            };
            match op {
                3 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} += {rhs};"),
                    next: ins.on_true,
                }),
                4 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} -= {rhs};"),
                    next: ins.on_true,
                }),
                5 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} := {rhs};"),
                    next: ins.on_true,
                }),
                6 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} *= {rhs};"),
                    next: ins.on_true,
                }),
                7 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} /= {rhs};"),
                    next: ins.on_true,
                }),
                9 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} flag_set {rhs};"),
                    next: ins.on_true,
                }),
                10 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} flag_clear {rhs};"),
                    next: ins.on_true,
                }),
                12 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} %= {rhs};"),
                    next: ins.on_true,
                }),
                13 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} &= {rhs};"),
                    next: ins.on_true,
                }),
                18 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} |= {rhs};"),
                    next: ins.on_true,
                }),
                19 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} ^= {rhs};"),
                    next: ins.on_true,
                }),
                20 => Ok(LiftedIns::Stmt {
                    text: format!("{lhs} sqrt {rhs};"),
                    next: ins.on_true,
                }),
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
                    Ok(LiftedIns::Cond {
                        text: format!("{lhs} {sym} {rhs}"),
                        stmt_form: None,
                        is_call: false,
                        on_true: ins.on_true,
                        on_false: ins.on_false,
                    })
                }
                _ => Err(opcode),
            }
        }
        8 => {
            let dst_idx = word_i16(ops, 0);
            let dst_scope = word_u16(ops, 2);
            let rng_idx = word_i16(ops, 4);
            let rng_scope = word_u16(ops, 6);
            if dst_scope > 255 || rng_scope > 255 {
                return Err(opcode);
            }
            let Some(dst) = format_operand(dst_scope as u8, dst_idx, max_args, max_locals) else {
                return Err(opcode);
            };
            let Some(rng) = format_operand(rng_scope as u8, rng_idx, max_args, max_locals) else {
                return Err(opcode);
            };
            let expr = format!("random({dst}, {rng})");
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        14 => {
            if ops[2..6] != [0; 4] || ops[7] != 0 {
                return Err(opcode);
            }
            let func = word_u16(ops, 0);
            let flags = ops[6];
            let expr = format!("find_best_object({func}, flags: {flags})");
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        17 => {
            let p = word_i16(ops, 0);
            if p < 0 || p as usize >= max_args || ops[4..8] != [0; 4] {
                return Err(opcode);
            }
            let allow_push = word_u16(ops, 2);
            let expr = format!("idle_for_input(param[{p}], {allow_push})");
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        18 => {
            let target = word_u16(ops, 0);
            if target > 2 {
                return Err(opcode);
            }
            let flags = ops[2];
            let expr = format!("remove_object({target}, flags: {flags})");
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        20 => {
            let func = word_u16(ops, 0);
            let change_icon = ops[2] & 1 != 0;
            let expr = format!("run_functional_tree({func}, change_icon: {change_icon})");
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        23 => {
            let sound_id = word_u16(ops, 0);
            let use_stack = ops[4] != 0xFF && ops[4] & 2 != 0;
            let text = if use_stack {
                format!("play_sound({sound_id}, stack_obj);")
            } else {
                format!("play_sound({sound_id});")
            };
            Ok(LiftedIns::Stmt {
                text,
                next: ins.on_true,
            })
        }
        25 => {
            let scope = match ops[0] {
                0 => 7,
                1 => 9,
                2 => 25,
                _ => ops[1],
            };
            let idx = word_i16(ops, 2);
            let flags = ops[4] & 3;
            let category = ops[6];
            if category > 8 {
                return Err(opcode);
            }
            let Some(amount) = format_operand(scope, idx, max_args, max_locals) else {
                return Err(opcode);
            };
            let expr = format!("alter_budget({amount}, category: {category}, flags: {flags})");
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        29 => {
            if ops[3] & 1 != 0 {
                Ok(LiftedIns::Stmt {
                    text: "clear_motive_changes();".to_owned(),
                    next: ins.on_true,
                })
            } else {
                let motive = ops[2];
                if !matches!(motive, 5..=9 | 14..=15) {
                    return Err(opcode);
                }
                let Some(delta) = format_operand(ops[0], word_i16(ops, 4), max_args, max_locals)
                else {
                    return Err(opcode);
                };
                let Some(stop) = format_operand(ops[1], word_i16(ops, 6), max_args, max_locals)
                else {
                    return Err(opcode);
                };
                Ok(LiftedIns::Stmt {
                    text: format!("set_motive_change({motive}, {delta}, {stop});"),
                    next: ins.on_true,
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
                return Err(opcode);
            }
            if base_mode == 9 && usize::from(local_byte) >= max_locals {
                return Err(opcode);
            }
            if !has_dest && (dest_scope != 0 || dest_idx != 0) {
                return Err(opcode);
            }
            let mut expr = format!("set_to_next({base_mode}, guid: {guid:#010x}");
            if has_dest {
                if dest_scope == 7 {
                    return Err(opcode);
                }
                let Some(dest_str) =
                    format_operand(dest_scope, i16::from(dest_idx), max_args, max_locals)
                else {
                    return Err(opcode);
                };
                let _ = write!(expr, ", dest: {dest_str}");
            }
            if local_byte != 0 {
                let _ = write!(expr, ", local: {local_byte}");
            }
            expr.push(')');
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        36 => {
            if ops[5] & 0x80 != 0 {
                return Err(opcode);
            }
            let expr = format!(
                "dialog(kind: {}, message: {}, title: {}, button1: {}, button2: {}, button3: {}, mode4_str: {}, flags: {})",
                ops[5], ops[2], ops[6], ops[3], ops[4], ops[0], ops[1], ops[7]
            );
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        42 => {
            if ops[7] != 0 {
                return Err(opcode);
            }
            let guid = u32::from_le_bytes([ops[0], ops[1], ops[2], ops[3]]);
            let mode = ops[4];
            let flags = ops[5];
            let local = ops[6];
            if mode == 7 && max_args == 0 {
                return Err(opcode);
            }
            if (mode == 8 || mode == 9) && usize::from(local) >= max_locals {
                return Err(opcode);
            }
            let expr = format!(
                "create_object({guid:#010x}, mode: {mode}, flags: {flags}, local: {local})"
            );
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        44 => {
            if ops[3] != 0 || ops[7] != 0 {
                return Err(opcode);
            }
            let clip = word_u16(ops, 0);
            let local = ops[2];
            let table = ops[4];
            let flags = ops[5];
            let events = ops[6] as i8;
            if flags & 0x20 != 0 && usize::from(local) >= max_locals {
                return Err(opcode);
            }
            if flags & 0x20 == 0
                && events != 0
                && matches!(flags & 0x11, 0x00 | 0x11)
                && max_args == 0
            {
                return Err(opcode);
            }
            if flags & 0x04 != 0 && ((clip as i16) < 0 || (clip as i16) as usize >= max_args) {
                return Err(opcode);
            }
            let expr = format!(
                "animate(clip: {clip}, table: {table}, flags: {flags}, events: {events}, local: {local})"
            );
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        45 => {
            if word_u16(ops, 2) != 1 || ops[5..8] != [0; 3] {
                return Err(opcode);
            }
            let slot = word_u16(ops, 0);
            let flags = ops[4];
            let expr = format!("route_to(slot: {slot}, flags: {flags})");
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        46 => {
            if word_u16(ops, 2) != 3 || ops[5..8] != [0; 3] {
                return Err(opcode);
            }
            let slot = word_u16(ops, 0);
            let flags = ops[4];
            let expr = format!("snap({slot}, {flags})");
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: false,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        48 => {
            let use_stack = word_i16(ops, 0) != 0;
            Ok(LiftedIns::Stmt {
                text: format!("stop_all_sounds({use_stack});"),
                next: ins.on_true,
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
            Ok(LiftedIns::Cond {
                stmt_form: Some(format!("{expr};")),
                text: expr,
                is_call: true,
                on_true: ins.on_true,
                on_false: ins.on_false,
            })
        }
        other => Err(other),
    }
}

/// Attempts to lift a [`BhavChunk`] strictly in memory into clean, high-level
/// structured `.antics` source text (recovering `if`/`else`, `while`, `return`,
/// named `Expression`s, readable primitives, and `call` — ZERO raw `op` lines!).
///
/// Uses a metadata-only synthetic label `Tree_<id>` so no commercial strings
/// ever appear in the generated source text.
#[must_use]
pub fn lift_bhav_structured(chunk: &BhavChunk) -> StructuredLiftOutcome {
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
    let eff_locals = usize::from(chunk.locals).min(max_locals) as u16;

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

    // 2. Lift every reachable instruction to high-level syntax (zero raw `op`).
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
        match lift_instruction(ins, arg_bound, local_bound) {
            Ok(li) => {
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
            Err(opcode) => return StructuredLiftOutcome::UnsupportedOpcode(opcode),
        }
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
}
