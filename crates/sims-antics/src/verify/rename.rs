//! Local-slot renaming for the behavioral relation (`verify-equiv --behavioral`).
//!
//! Two builds of a tree often keep the same values in different local slots: a compiler
//! numbers its `let`s in its own order, and Animate's event local is wherever the source
//! put it. The behavioral relation does not observe frame storage
//! (`docs/decisions/20261123-verifier-behavioral-mode.md`), so a local's slot number
//! matters only through what reads the slot. That makes renaming a local safe exactly
//! when every read of it sees a value the tree itself wrote:
//!
//! - **Clean slots.** A slot is clean when no read of it can see its value on entry: on
//!   every path from the entry, an `Expression` `:=` (op 5) or op 20, which write without
//!   reading (`expression.rs:75`), or Animate's event store, writes it before anything
//!   reads it. Animate (opcode 44) with flag 0x20 writes `local[ops[2]]` on every false
//!   branch and only there (`animate.rs:197`, `animate.rs:209`, `animate.rs:269`), and
//!   faults on an object that is not a person (`animate.rs:99`). A local's value on entry
//!   is whatever an earlier frame or Create New Object Instance (flag 0x02 writes the
//!   bottom frame's first local of a tree with no argument, `create.rs:537`) left there,
//!   so a slot that may be read first is *dirty* and keeps its number.
//! - **Readers the renaming cannot follow.** `Dialog` and `ActionString` may format any
//!   local (`$Local:N`, `$TimeLocal:N`), and an unmodeled primitive may read the whole
//!   frame: a tree with either keeps every number. A slot any other primitive names
//!   (Random's destination, Set To Next's, Relationship's, ...) keeps its number too; only
//!   `Expression` operands and Animate's event local are renamed. A call that takes its
//!   arguments from the following nodes' bytes (more than four, or an unknown count)
//!   reads those nodes as data, so a tree with one keeps every number.
//!
//! A renaming is a permutation of the clean slots (dirty and pinned slots stay put), so
//! the renamed tree reads and writes the same values in the same order; only the names
//! of clean slots change. Which permutation is tried decides only whether a proof is
//! found, never whether it is sound: the checker then proves the renamed tree equivalent
//! to the other one as usual. Kind-4 trees, which every node may enter, are not renamed.

use super::live::{LiveContext, frame_dims, primitive_frame_writes};
use super::{FrameUse, is_always_true_instruction, primitive_frame_uses, word_i16};
use crate::ir::BhavChunk;
use sims_formats::behavior::Instruction;

/// The local scope of `Expression` operands and primitive frame uses (`memory.rs:545`).
const LOCAL: u8 = 25;
/// The last node index the VM runs (`interpreter.rs:766`).
const LAST_NODE: usize = 250;

/// A set of local slots (a frame has at most 255 locals).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slots([u64; 4]);

impl Slots {
    const NONE: Self = Self([0; 4]);

    fn has(self, slot: usize) -> bool {
        self.0[slot / 64] & (1 << (slot % 64)) != 0
    }

    fn with(mut self, slot: usize) -> Self {
        self.0[slot / 64] |= 1 << (slot % 64);
        self
    }

    fn meet(self, other: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] & other.0[i]))
    }
}

/// What one node does with the locals.
#[derive(Debug, Default)]
struct NodeUse {
    /// Slots it may read, in the order it reads them.
    reads: Vec<usize>,
    /// Slots it writes on its true branch, and on its false branch, without reading them.
    writes_true: Vec<usize>,
    writes_false: Vec<usize>,
    /// Slots it names in a way the renaming does not rewrite.
    pinned: Vec<usize>,
    /// Every slot it names, in operand order (for aligning two trees).
    touches: Vec<usize>,
}

/// How a tree uses its local slots.
#[derive(Debug)]
pub(super) struct LocalUse {
    locals: usize,
    /// Slots that keep their number: dirty or pinned.
    fixed: Vec<bool>,
    /// In-range slots in the order a walk from the entry first names them.
    order: Vec<usize>,
}

fn node_use(ins: &Instruction, locals: usize, ctx: &LiveContext) -> Result<NodeUse, &'static str> {
    let slot = |idx: i16| usize::try_from(idx).ok().filter(|&i| i < locals);
    let mut out = NodeUse::default();
    let ops = &ins.operands;
    if ins.opcode == 2 {
        let (op, lhs_scope, rhs_scope) = (ops[5], ops[6], ops[7]);
        // The right operand is read first (`expression.rs:43`).
        if rhs_scope == LOCAL
            && let Some(s) = slot(word_i16(ops, 2))
        {
            out.reads.push(s);
            out.touches.push(s);
        }
        if lhs_scope == LOCAL
            && let Some(s) = slot(word_i16(ops, 0))
        {
            if matches!(op, 5 | 20) {
                out.writes_true.push(s);
                out.writes_false.push(s);
            } else {
                out.reads.push(s);
            }
            out.touches.push(s);
        }
        return Ok(out);
    }
    if ins.opcode >= 256 {
        let argless = *ops == [0xFF; 8] || ctx.call_reads_no_operands(ins.opcode);
        let arity = ctx
            .callee_arities
            .as_ref()
            .and_then(|m| m.get(&ins.opcode).copied());
        if !argless && !arity.is_some_and(|n| n <= 4) {
            return Err("a call may read its arguments from the following nodes");
        }
    }
    // Animate's event local is the one primitive operand renamed.
    let event_local = (ins.opcode == 44 && ops[5] & 0x20 != 0)
        .then(|| slot(i16::from(ops[2])))
        .flatten();
    if let Some(s) = event_local {
        out.writes_false.push(s);
        out.touches.push(s);
    }
    for frame_use in primitive_frame_uses(ins.opcode, ops) {
        match frame_use {
            FrameUse::AllLocals(_) => return Err("text formatting may read any local"),
            FrameUse::WholeFrame => return Err("an unmodeled primitive may read any local"),
            FrameUse::Read(LOCAL, idx, _) => {
                if let Some(s) = slot(idx) {
                    out.reads.push(s);
                    out.pinned.push(s);
                    out.touches.push(s);
                }
            }
            FrameUse::Dst(LOCAL, idx, _) => {
                if let Some(s) = slot(idx).filter(|&s| Some(s) != event_local) {
                    out.pinned.push(s);
                    out.touches.push(s);
                }
            }
            _ => {}
        }
    }
    for (scope, idx) in primitive_frame_writes(ins.opcode, ops) {
        if scope == LOCAL
            && let Some(s) = slot(idx).filter(|&s| Some(s) != event_local)
        {
            out.pinned.push(s);
            out.touches.push(s);
        }
    }
    Ok(out)
}

/// The node a branch target runs next, if it is a node.
fn successor(target: u8, len: usize) -> Option<usize> {
    let t = usize::from(target);
    (t < len && t <= LAST_NODE).then_some(t)
}

/// Whether node `ins` can take its false branch (`iff::runnable_nodes`).
fn has_false_branch(ins: &Instruction) -> bool {
    !(ins.opcode == 8 || is_always_true_instruction(ins))
}

/// How `bhav` uses its locals, or why it cannot be renamed.
pub(super) fn local_use(bhav: &BhavChunk, ctx: &LiveContext) -> Result<LocalUse, &'static str> {
    if bhav.kind == 4 {
        return Err("every node of a kind-4 tree is an entry");
    }
    let locals = frame_dims(bhav).1;
    let len = bhav.instructions.len();
    if len == 0 {
        return Err("an empty tree");
    }
    let uses: Vec<NodeUse> = bhav
        .instructions
        .iter()
        .map(|ins| node_use(ins, locals, ctx))
        .collect::<Result<_, _>>()?;
    // Forward must-be-written analysis: the slots written on every path to a node.
    let mut written: Vec<Option<Slots>> = vec![None; len];
    written[0] = Some(Slots::NONE);
    let mut work = vec![0usize];
    while let Some(node) = work.pop() {
        let Some(before) = written[node] else {
            continue;
        };
        let ins = &bhav.instructions[node];
        let u = &uses[node];
        let mut edges = vec![(ins.on_true, &u.writes_true)];
        if has_false_branch(ins) {
            edges.push((ins.on_false, &u.writes_false));
        }
        for (target, writes) in edges {
            let Some(next) = successor(target, len) else {
                continue;
            };
            let after = writes.iter().fold(before, |s, &w| s.with(w));
            let merged = written[next].map_or(after, |old| old.meet(after));
            if written[next] != Some(merged) {
                written[next] = Some(merged);
                work.push(next);
            }
        }
    }
    let mut fixed = vec![false; locals];
    for (node, u) in uses.iter().enumerate() {
        let Some(before) = written[node] else {
            continue;
        };
        for &r in &u.reads {
            if !before.has(r) {
                fixed[r] = true;
            }
        }
        for &p in &u.pinned {
            fixed[p] = true;
        }
    }
    // First-touch order: depth first from the entry, true branch first.
    let mut order = Vec::new();
    let mut seen_slot = vec![false; locals];
    let mut seen_node = vec![false; len];
    let mut stack = vec![0usize];
    while let Some(node) = stack.pop() {
        if seen_node[node] {
            continue;
        }
        seen_node[node] = true;
        for &s in &uses[node].touches {
            if !seen_slot[s] {
                seen_slot[s] = true;
                order.push(s);
            }
        }
        let ins = &bhav.instructions[node];
        if has_false_branch(ins)
            && let Some(next) = successor(ins.on_false, len)
        {
            stack.push(next);
        }
        if let Some(next) = successor(ins.on_true, len) {
            stack.push(next);
        }
    }
    Ok(LocalUse {
        locals,
        fixed,
        order,
    })
}

/// A permutation of `from`'s clean slots that puts the `k`-th slot `from` names where
/// `to` names its `k`-th, as far as both are clean in `from`; the clean slots left over
/// keep their order in the free places. `None` when it changes nothing.
pub(super) fn alignment(from: &LocalUse, to: &LocalUse) -> Option<Vec<usize>> {
    let n = from.locals;
    let clean = |s: usize| s < n && !from.fixed[s];
    let mut perm: Vec<Option<usize>> = vec![None; n];
    let mut taken = vec![false; n];
    for (s, f) in from.fixed.iter().enumerate() {
        if *f {
            perm[s] = Some(s);
            taken[s] = true;
        }
    }
    for (&b, &a) in from.order.iter().zip(&to.order) {
        if clean(b) && clean(a) && perm[b].is_none() && !taken[a] {
            perm[b] = Some(a);
            taken[a] = true;
        }
    }
    let mut free = (0..n).filter(|&s| !taken[s]);
    let perm: Vec<usize> = perm
        .into_iter()
        .map(|p| p.or_else(|| free.next()))
        .collect::<Option<_>>()?;
    perm.iter()
        .enumerate()
        .any(|(i, &p)| i != p)
        .then_some(perm)
}

/// `bhav` with its local slots renamed by `perm`, or `None` if the result does not
/// name exactly the renamed slots everywhere the checker looks.
pub(super) fn renamed(bhav: &BhavChunk, perm: &[usize]) -> Option<BhavChunk> {
    let n = perm.len();
    let map = |idx: i16| -> i16 {
        usize::try_from(idx)
            .ok()
            .filter(|&i| i < n)
            .map_or(idx, |i| perm[i] as i16)
    };
    let mut out = bhav.clone();
    for ins in &mut out.instructions {
        let ops = &mut ins.operands;
        if ins.opcode == 2 {
            if ops[6] == LOCAL {
                let idx = map(word_i16(ops, 0)).to_le_bytes();
                ops[0..2].copy_from_slice(&idx);
            }
            if ops[7] == LOCAL {
                let idx = map(word_i16(ops, 2)).to_le_bytes();
                ops[2..4].copy_from_slice(&idx);
            }
        } else if ins.opcode == 44 && ops[5] & 0x20 != 0 {
            ops[2] = u8::try_from(map(i16::from(ops[2]))).ok()?;
        }
    }
    // Every frame use and write of every node is the original's, renamed.
    let map_use = |u: FrameUse| match u {
        FrameUse::Read(LOCAL, idx, w) => FrameUse::Read(LOCAL, map(idx), w),
        FrameUse::Dst(LOCAL, idx, w) => FrameUse::Dst(LOCAL, map(idx), w),
        other => other,
    };
    for (old, new) in bhav.instructions.iter().zip(&out.instructions) {
        if old.opcode == 2 {
            continue;
        }
        let expected: Vec<FrameUse> = primitive_frame_uses(old.opcode, &old.operands)
            .into_iter()
            .map(map_use)
            .collect();
        if primitive_frame_uses(new.opcode, &new.operands) != expected {
            return None;
        }
        let expected: Vec<(u8, i16)> = primitive_frame_writes(old.opcode, &old.operands)
            .into_iter()
            .map(|(s, i)| if s == LOCAL { (s, map(i)) } else { (s, i) })
            .collect();
        if primitive_frame_writes(new.opcode, &new.operands) != expected {
            return None;
        }
    }
    Some(out)
}

/// Renamings of `a` or `b` worth checking: `b`'s locals moved to where `a` keeps the
/// same values, then `a`'s to where `b` does.
pub(super) fn candidates(
    a: &BhavChunk,
    ctx_a: &LiveContext,
    b: &BhavChunk,
    ctx_b: &LiveContext,
) -> Vec<(BhavChunk, BhavChunk)> {
    let (Ok(use_a), Ok(use_b)) = (local_use(a, ctx_a), local_use(b, ctx_b)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(renamed_b) = alignment(&use_b, &use_a).and_then(|p| renamed(b, &p)) {
        out.push((a.clone(), renamed_b));
    }
    if let Some(renamed_a) = alignment(&use_a, &use_b).and_then(|p| renamed(a, &p)) {
        out.push((renamed_a, b.clone()));
    }
    out
}
