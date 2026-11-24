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
//! - **Readers the renaming cannot follow.** `Dialog` and `ActionString` format locals
//!   their strings name (`$Local:N`, `$TimeLocal:N`): those keep their number when the
//!   tree's own file's strings are known ([`super::text`]), and otherwise any local may
//!   be read. An unmodeled primitive may read the whole frame. A tree with either of
//!   those keeps every number. A slot any other primitive names
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

/// Where a primitive's operands hold a local's number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pos {
    /// The little-endian word at this byte.
    Word(usize),
    /// This byte, unsigned.
    Byte(usize),
    /// This byte, signed (`create.rs:177`).
    SignedByte(usize),
}

impl Pos {
    pub(super) fn get(self, ops: &[u8; 8]) -> i16 {
        match self {
            Pos::Word(at) => word_i16(ops, at),
            Pos::Byte(at) => i16::from(ops[at]),
            Pos::SignedByte(at) => i16::from(ops[at] as i8),
        }
    }

    /// Writes `idx` here, or `None` if it does not fit.
    pub(super) fn set(self, ops: &mut [u8; 8], idx: i16) -> Option<()> {
        match self {
            Pos::Word(at) => ops[at..at + 2].copy_from_slice(&idx.to_le_bytes()),
            Pos::Byte(at) => ops[at] = u8::try_from(idx).ok()?,
            Pos::SignedByte(at) => ops[at] = i8::try_from(idx).ok()? as u8,
        }
        Some(())
    }
}

/// The operands of primitive `opcode` that name a local, each with whether the
/// primitive writes it without reading it first, mirroring the scope-25 uses
/// [`primitive_frame_uses`] and [`primitive_frame_writes`] list for these opcodes (the
/// renamed tree is checked against both, in [`renamed`]). Only Random Number's
/// destination counts as a write: `random.rs` stores it and takes the true branch.
/// Every other one counts as a read, which can only keep more slots dirty.
///
/// Opcodes not listed here (Animate's event local aside) keep every local they name.
pub(super) fn local_operands(opcode: u16, ops: &[u8; 8]) -> Vec<(Pos, bool)> {
    let word = |at: usize| u16::from_le_bytes([ops[at], ops[at + 1]]);
    let local = u16::from(LOCAL);
    let mut out = Vec::new();
    let mut read = |cond: bool, pos: Pos| {
        if cond {
            out.push((pos, false));
        }
    };
    match opcode {
        8 => {
            read(word(6) == local, Pos::Word(4));
            if word(2) == local {
                out.push((Pos::Word(0), true));
            }
        }
        11 => read(ops[2] & 1 != 0 && ops[3] == LOCAL, Pos::Word(4)),
        12 => {
            read(word(2) == local, Pos::Word(0));
            read(ops[4] & 1 != 0 && ops[5] == LOCAL, Pos::Word(6));
        }
        13 => {
            read(ops[3] & 2 != 0, Pos::Byte(1));
            read(ops[3] & 1 != 0, Pos::Byte(4));
        }
        14 => read(ops[6] & 8 != 0 && word(4) == local, Pos::Word(2)),
        15 => read(word(2) == local, Pos::Word(0)),
        16 => read(ops[2] & 1 != 0, Pos::Byte(1)),
        19 => {
            read(true, Pos::Byte(0));
            read(true, Pos::Byte(1));
            read(true, Pos::Byte(2));
        }
        25 => read(ops[0] == 2 || (ops[0] > 2 && ops[1] == LOCAL), Pos::Word(2)),
        26 => {
            read(matches!(ops[1], 2 | 3), Pos::Byte(3));
            read(ops[4] == LOCAL, Pos::Word(6));
        }
        29 => {
            read(ops[3] & 1 == 0 && ops[0] == LOCAL, Pos::Word(4));
            read(ops[3] & 1 == 0 && ops[1] == LOCAL, Pos::Word(6));
        }
        31 => {
            read(ops[4] & 0x80 != 0 && ops[5] == LOCAL, Pos::Byte(7));
            read(ops[4] & 0x7F == 9, Pos::Byte(6));
        }
        32 => read(ops[6] == LOCAL, Pos::Word(4)),
        34 => read(ops[0] <= 2 && ops[1] == LOCAL, Pos::Word(2)),
        35 => read(ops[4] & 0x20 != 0, Pos::Word(0)),
        37 => read(ops[0] == 1 && ops[1] == LOCAL, Pos::Word(2)),
        42 => read(matches!(ops[4], 8 | 9), Pos::SignedByte(6)),
        _ => {}
    }
    out
}

/// The local numbers node `ins` names where the renaming rewrites them, in operand
/// order, with what tells two nodes' operands apart beyond those numbers (for
/// [`lockstep_pairs`]).
fn named_slots(ins: &Instruction) -> (Vec<i16>, Vec<u8>) {
    let ops = &ins.operands;
    if ins.opcode == 2 {
        let mut slots = Vec::new();
        if ops[6] == LOCAL {
            slots.push(word_i16(ops, 0));
        }
        if ops[7] == LOCAL {
            slots.push(word_i16(ops, 2));
        }
        return (slots, vec![ops[5], ops[6], ops[7]]);
    }
    let mut positions: Vec<Pos> = local_operands(ins.opcode, ops)
        .into_iter()
        .map(|(pos, _)| pos)
        .collect();
    if ins.opcode == 44 && ops[5] & 0x20 != 0 {
        positions.push(Pos::Byte(2));
    }
    let mut shape = *ops;
    for pos in &positions {
        let _ = pos.set(&mut shape, 0);
    }
    (
        positions.iter().map(|pos| pos.get(ops)).collect(),
        shape.to_vec(),
    )
}

fn node_use(
    ins: &Instruction,
    tree: u16,
    locals: usize,
    ctx: &LiveContext,
) -> Result<NodeUse, &'static str> {
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
    // Local operands the renaming rewrites ([`local_operands`]); every other local a
    // primitive names keeps its number.
    let mut covered = Vec::new();
    for (pos, writes) in local_operands(ins.opcode, ops) {
        if let Some(s) = slot(pos.get(ops)) {
            if writes {
                out.writes_true.push(s);
            } else {
                out.reads.push(s);
            }
            out.touches.push(s);
            covered.push(s);
        }
    }
    let covered = |s: &usize| covered.contains(s);
    for frame_use in primitive_frame_uses(ins.opcode, ops) {
        match frame_use {
            FrameUse::AllLocals(_) => {
                // The locals its file's strings name keep their number
                // (`LiveContext::text_reads`); without them, any local may be read.
                let slots = ctx
                    .text_reads(tree, ins.opcode, ops)
                    .ok_or("text formatting may read any local")?;
                for s in slots.into_iter().filter_map(slot) {
                    out.reads.push(s);
                    out.pinned.push(s);
                    out.touches.push(s);
                }
            }
            FrameUse::WholeFrame => return Err("an unmodeled primitive may read any local"),
            FrameUse::Read(LOCAL, idx, _) => {
                if let Some(s) = slot(idx).filter(|s| !covered(s)) {
                    out.reads.push(s);
                    out.pinned.push(s);
                    out.touches.push(s);
                }
            }
            FrameUse::Dst(LOCAL, idx, _) => {
                if let Some(s) = slot(idx).filter(|&s| Some(s) != event_local && !covered(&s)) {
                    out.pinned.push(s);
                    out.touches.push(s);
                }
            }
            _ => {}
        }
    }
    for (scope, idx) in primitive_frame_writes(ins.opcode, ops) {
        if scope == LOCAL
            && let Some(s) = slot(idx).filter(|&s| Some(s) != event_local && !covered(&s))
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
        .map(|ins| node_use(ins, bhav.id, locals, ctx))
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
    permutation(
        from,
        from.order.iter().copied().zip(to.order.iter().copied()),
    )
}

/// A permutation of `from`'s clean slots that moves each slot of `pairs` (`(slot in
/// from, slot to move it to)`) where it says, as far as both are clean in `from` and
/// no earlier pair took either; the clean slots left over keep their order in the free
/// places. `None` when it changes nothing.
fn permutation(from: &LocalUse, pairs: impl Iterator<Item = (usize, usize)>) -> Option<Vec<usize>> {
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
    for (b, a) in pairs {
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

/// Slot pairs `(slot in from, slot in to)` found by walking both trees in step from
/// their entries: two nodes with the same opcode and the same operands apart from the
/// local numbers [`named_slots`] lists pair those numbers in order, and their true
/// successors and their false successors are walked in step next. The walk stops at
/// nodes that do not match. It only proposes a renaming, which is then proven like any
/// other ([`candidates`]).
fn lockstep_pairs(from: &BhavChunk, to: &BhavChunk) -> Vec<(usize, usize)> {
    let (len_from, len_to) = (from.instructions.len(), to.instructions.len());
    let mut pairs = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = std::collections::VecDeque::from([(0usize, 0usize)]);
    while let Some((f, t)) = queue.pop_front() {
        if f >= len_from || t >= len_to || !seen.insert((f, t)) {
            continue;
        }
        let (fi, ti) = (&from.instructions[f], &to.instructions[t]);
        if fi.opcode != ti.opcode {
            continue;
        }
        let ((fs, fshape), (ts, tshape)) = (named_slots(fi), named_slots(ti));
        if fshape != tshape || fs.len() != ts.len() {
            continue;
        }
        for (a, b) in fs.into_iter().zip(ts) {
            if let (Ok(a), Ok(b)) = (usize::try_from(a), usize::try_from(b)) {
                pairs.push((a, b));
            }
        }
        if let (Some(a), Some(b)) = (
            successor(fi.on_true, len_from),
            successor(ti.on_true, len_to),
        ) {
            queue.push_back((a, b));
        }
        if has_false_branch(fi)
            && let (Some(a), Some(b)) = (
                successor(fi.on_false, len_from),
                successor(ti.on_false, len_to),
            )
        {
            queue.push_back((a, b));
        }
    }
    pairs
}

/// Slot pairs `(slot in from, slot in to)` of slots the two trees name alike: each
/// slot's uses are the nodes naming it, as their opcode, operands apart from local
/// numbers ([`named_slots`]) and operand place, and two slots score the uses they share.
/// The pairs go best score first (ties by the order each tree first names its slots),
/// each slot once. Like [`lockstep_pairs`], this only proposes a renaming.
fn similar_pairs(
    from: &BhavChunk,
    from_use: &LocalUse,
    to: &BhavChunk,
    to_use: &LocalUse,
) -> Vec<(usize, usize)> {
    type Uses = Vec<(u16, Vec<u8>, usize)>;
    let uses = |tree: &BhavChunk, locals: usize| -> Vec<Uses> {
        let mut out: Vec<Uses> = vec![Vec::new(); locals];
        for ins in &tree.instructions {
            let (slots, shape) = named_slots(ins);
            for (k, idx) in slots.into_iter().enumerate() {
                if let Ok(s) = usize::try_from(idx)
                    && s < locals
                {
                    out[s].push((ins.opcode, shape.clone(), k));
                }
            }
        }
        for u in &mut out {
            u.sort();
        }
        out
    };
    // Shared uses of two sorted lists, counted with multiplicity.
    let shared = |x: &Uses, y: &Uses| {
        let (mut i, mut j, mut n) = (0, 0, 0);
        while i < x.len() && j < y.len() {
            match x[i].cmp(&y[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    n += 1;
                    i += 1;
                    j += 1;
                }
            }
        }
        n
    };
    let (uses_from, uses_to) = (uses(from, from_use.locals), uses(to, to_use.locals));
    let mut scored = Vec::new();
    for (rank_f, &f) in from_use.order.iter().enumerate() {
        for (rank_t, &t) in to_use.order.iter().enumerate() {
            let score = shared(&uses_from[f], &uses_to[t]);
            if score > 0 {
                scored.push((std::cmp::Reverse(score), rank_f, rank_t, f, t));
            }
        }
    }
    scored.sort();
    let (mut used_f, mut used_t) = (vec![false; from_use.locals], vec![false; to_use.locals]);
    let mut pairs = Vec::new();
    for (_, _, _, f, t) in scored {
        if !used_f[f] && !used_t[t] {
            used_f[f] = true;
            used_t[t] = true;
            pairs.push((f, t));
        }
    }
    pairs
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
        } else {
            for (pos, _) in local_operands(ins.opcode, ops) {
                let idx = map(pos.get(ops));
                pos.set(ops, idx)?;
            }
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

/// Renamings of `a` or `b` worth checking, each once: `b`'s locals moved to where `a`
/// keeps the same values, then `a`'s to where `b` does, first as a walk of both trees
/// in step pairs them ([`lockstep_pairs`]), then by the order each tree first names
/// them ([`alignment`]).
pub(super) fn candidates(
    a: &BhavChunk,
    ctx_a: &LiveContext,
    b: &BhavChunk,
    ctx_b: &LiveContext,
) -> Vec<(BhavChunk, BhavChunk)> {
    let (ua, ub) = (local_use(a, ctx_a), local_use(b, ctx_b));
    let (Ok(use_a), Ok(use_b)) = (ua, ub) else {
        return Vec::new();
    };
    let flip =
        |pairs: Vec<(usize, usize)>| pairs.into_iter().map(|(x, y)| (y, x)).collect::<Vec<_>>();
    // A walk in step first, then slots used alike.
    let walked = lockstep_pairs(a, b);
    let mut a_to_b = walked.clone();
    a_to_b.extend(similar_pairs(a, &use_a, b, &use_b));
    let mut b_to_a = flip(walked);
    b_to_a.extend(similar_pairs(b, &use_b, a, &use_a));
    let tries = [
        (false, permutation(&use_b, b_to_a.into_iter())),
        (true, permutation(&use_a, a_to_b.into_iter())),
        (false, alignment(&use_b, &use_a)),
        (true, alignment(&use_a, &use_b)),
    ];
    let mut out: Vec<(BhavChunk, BhavChunk)> = Vec::new();
    for (rename_a, perm) in tries {
        let Some(perm) = perm else {
            continue;
        };
        let pair = if rename_a {
            renamed(a, &perm).map(|ra| (ra, b.clone()))
        } else {
            renamed(b, &perm).map(|rb| (a.clone(), rb))
        };
        let same = |x: &BhavChunk, y: &BhavChunk| {
            x.instructions.len() == y.instructions.len()
                && x.instructions.iter().zip(&y.instructions).all(|(i, j)| {
                    (i.opcode, i.on_true, i.on_false, i.operands)
                        == (j.opcode, j.on_true, j.on_false, j.operands)
                })
        };
        if let Some(pair) = pair
            && !out
                .iter()
                .any(|(x, y)| same(x, &pair.0) && same(y, &pair.1))
        {
            out.push(pair);
        }
    }
    out
}
