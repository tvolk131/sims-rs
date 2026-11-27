//! Node-graph rewrites of one compiled tree, applied after the `.air` compiler has assembled
//! it (`docs/decisions/20261126-antics-v2-3-step2-node-order-and-shared-tails.md`):
//!
//! - `share_tails` merges the nodes that run alike: the same instruction, whose branches lead
//!   to nodes that run alike or return the same way. The tree runs the same instructions in
//!   the same order on every run; only the node numbers change.
//! - `node_order` renumbers the nodes: entry `i` names the compiled node that becomes node `i`.
//!
//! [`match_node_order`] finds the order that turns one tree into another with the same nodes,
//! for a decompiler that pins a lifted tree's node numbers to the original's. [`match_tree_pin`]
//! finds the whole pin, `dead_false = error` included
//! (`docs/decisions/20261126-antics-v2-4-dead-false-error.md`).
//!
//! An original's nodes that no branch reaches can be kept as bytes at their node numbers
//! ([`DeadNode`], `dead_nodes`): [`split_dead_nodes`] takes them out, and
//! [`match_tree_pin_quarantined`] pins the rest
//! (`docs/decisions/20261127-antics-v2-6-compiler.md`).

use sims_formats::behavior::Instruction;

use crate::lower::{MAX_BHAV_NODES, TARGET_ERROR, TARGET_FALSE, TARGET_TRUE};
use crate::verify::is_always_true_instruction;

/// A branch of a node before the tree is numbered: another node, or a return
/// (`TARGET_ERROR`, `TARGET_TRUE` or `TARGET_FALSE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Edge {
    Node(usize),
    Exit(u8),
}

/// One node with unnumbered branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WideNode {
    pub opcode: u16,
    pub operands: [u8; 8],
    pub on_true: Edge,
    pub on_false: Edge,
}

/// Reads branch byte `b` of a tree of `count` nodes.
fn edge_of(b: u8, count: usize) -> Option<Edge> {
    if b >= TARGET_ERROR {
        Some(Edge::Exit(b))
    } else if usize::from(b) < count.min(MAX_BHAV_NODES) {
        Some(Edge::Node(usize::from(b)))
    } else {
        None
    }
}

/// `instructions` as unnumbered nodes, or `None` when a branch names a node the tree does
/// not have or a reserved byte (`0xFB`, `0xFC`): running it faults, and renumbering could make
/// it name a real node.
pub(crate) fn widen(instructions: &[Instruction]) -> Option<Vec<WideNode>> {
    let count = instructions.len();
    instructions
        .iter()
        .map(|i| {
            Some(WideNode {
                opcode: i.opcode,
                operands: i.operands,
                on_true: edge_of(i.on_true, count)?,
                on_false: edge_of(i.on_false, count)?,
            })
        })
        .collect()
}

/// Numbers `nodes` as BHAV instructions. They must fit [`MAX_BHAV_NODES`].
pub(crate) fn narrow(nodes: &[WideNode]) -> Vec<Instruction> {
    debug_assert!(nodes.len() <= MAX_BHAV_NODES);
    let byte = |e: Edge| match e {
        Edge::Node(n) => n as u8,
        Edge::Exit(b) => b,
    };
    nodes
        .iter()
        .map(|n| Instruction {
            opcode: n.opcode,
            on_true: byte(n.on_true),
            on_false: byte(n.on_false),
            operands: n.operands,
        })
        .collect()
}

/// Merges every set of nodes that run alike into its first node (the coarsest bisimulation,
/// by partition refinement): two nodes run alike when they hold the same instruction and each
/// branch returns the same way or leads to nodes that run alike, loops included. The merged
/// nodes keep the order of their first nodes, so node 0 stays the entry.
///
/// `Err(n)` once the nodes cannot merge into `limit` or fewer: refinement only splits classes,
/// so `n` (more than `limit`) is a lower bound of the merged count.
pub(crate) fn share_tails(nodes: &[WideNode], limit: usize) -> Result<Vec<WideNode>, usize> {
    let count = nodes.len();
    // Class ids are given in order of each class's first node, so a node's final class id is
    // its new number.
    let mut class = vec![0usize; count];
    let mut classes = usize::from(count > 0);
    loop {
        let mut ids = std::collections::BTreeMap::new();
        let mut next = Vec::with_capacity(count);
        for (i, n) in nodes.iter().enumerate() {
            let edge = |e: Edge| match e {
                Edge::Node(t) => Edge::Node(class[t]),
                exit => exit,
            };
            let key = (
                class[i],
                n.opcode,
                n.operands,
                edge(n.on_true),
                edge(n.on_false),
            );
            let fresh = ids.len();
            next.push(*ids.entry(key).or_insert(fresh));
        }
        let refined = ids.len();
        if refined > limit {
            return Err(refined);
        }
        class = next;
        if refined == classes {
            break;
        }
        classes = refined;
    }
    let mut out: Vec<WideNode> = Vec::with_capacity(classes);
    for (i, n) in nodes.iter().enumerate() {
        if class[i] == out.len() {
            let edge = |e: Edge| match e {
                Edge::Node(t) => Edge::Node(class[t]),
                exit => exit,
            };
            out.push(WideNode {
                on_true: edge(n.on_true),
                on_false: edge(n.on_false),
                ..*n
            });
        }
    }
    Ok(out)
}

/// Checks `order` as a node order of a tree of `count` nodes: a permutation of `0..count`
/// that keeps node 0 first. The error says what is wrong.
pub(crate) fn check_node_order(order: &[u16], count: usize) -> Result<(), String> {
    if order.len() != count {
        return Err(format!(
            "node_order lists {} node(s), but the tree compiles to {count}",
            order.len()
        ));
    }
    let mut seen = vec![false; count];
    for (i, &n) in order.iter().enumerate() {
        let n = usize::from(n);
        if n >= count {
            return Err(format!(
                "node_order entry {i} names node {n}, but the tree compiles to {count} node(s) (0..={})",
                count - 1
            ));
        }
        if std::mem::replace(&mut seen[n], true) {
            return Err(format!("node_order names node {n} twice"));
        }
    }
    if order.first().is_some_and(|&n| n != 0) {
        return Err(format!(
            "node_order must start with 0: a tree runs from node 0, so the entry node cannot move (found {})",
            order[0]
        ));
    }
    Ok(())
}

/// Renumbers `nodes` by `order` (checked with [`check_node_order`]): node `i` of the result is
/// `nodes[order[i]]`, and every branch follows its node.
pub(crate) fn apply_node_order(nodes: &[WideNode], order: &[u16]) -> Vec<WideNode> {
    let mut number = vec![0usize; nodes.len()];
    for (i, &n) in order.iter().enumerate() {
        number[usize::from(n)] = i;
    }
    let edge = |e: Edge| match e {
        Edge::Node(t) => Edge::Node(number[t]),
        exit => exit,
    };
    order
        .iter()
        .map(|&n| {
            let node = nodes[usize::from(n)];
            WideNode {
                on_true: edge(node.on_true),
                on_false: edge(node.on_false),
                ..node
            }
        })
        .collect()
}

/// The `node_order` that renumbers `compiled` into `original`, when the two trees are the same
/// graph of the same instructions: walked together from node 0, each pair of nodes holds the
/// same opcode and operands, and each pair of branches returns the same way or leads to a
/// consistent pair of nodes. Every node of both trees must be reached from node 0.
///
/// `None` when no renumbering makes the trees identical. A tree with the result written as
/// `#[bhav(.., node_order = [..])]` compiles to `original`'s instructions exactly.
pub fn match_node_order(compiled: &[Instruction], original: &[Instruction]) -> Option<Vec<u16>> {
    let count = compiled.len();
    if original.len() != count {
        return None;
    }
    if count == 0 {
        return Some(Vec::new());
    }
    let (a, b) = (widen(compiled)?, widen(original)?);
    // to_original[c] = o and to_compiled[o] = c for each paired node.
    let mut to_original = vec![usize::MAX; count];
    let mut to_compiled = vec![usize::MAX; count];
    let mut pending = vec![(0usize, 0usize)];
    to_original[0] = 0;
    to_compiled[0] = 0;
    while let Some((c, o)) = pending.pop() {
        let (x, y) = (a[c], b[o]);
        if x.opcode != y.opcode || x.operands != y.operands {
            return None;
        }
        for (ec, eo) in [(x.on_true, y.on_true), (x.on_false, y.on_false)] {
            match (ec, eo) {
                (Edge::Exit(p), Edge::Exit(q)) if p == q => {}
                (Edge::Node(nc), Edge::Node(no)) => match (to_original[nc], to_compiled[no]) {
                    (usize::MAX, usize::MAX) => {
                        to_original[nc] = no;
                        to_compiled[no] = nc;
                        pending.push((nc, no));
                    }
                    (seen_o, seen_c) if seen_o == no && seen_c == nc => {}
                    _ => return None,
                },
                _ => return None,
            }
        }
    }
    to_compiled
        .iter()
        .map(|&c| (c != usize::MAX).then_some(c as u16))
        .collect()
}

/// Whether `original` is a tree the original compiler wrote with its dead false branches as
/// `false: error`: it has a node whose instruction always branches true
/// ([`is_always_true_instruction`]), and every such node's false branch is `error` (0xFD).
/// A tree with `#[bhav(.., dead_false = error)]` compiles its statements so.
#[must_use]
pub fn wants_dead_false_error(original: &[Instruction]) -> bool {
    let mut dead = original
        .iter()
        .filter(|i| is_always_true_instruction(i))
        .peekable();
    dead.peek().is_some() && dead.all(|i| i.on_false == TARGET_ERROR)
}

/// `compiled`, a tree built without `dead_false`, as `dead_false = error` builds it: each node
/// whose instruction always branches true and whose false branch goes where its true branch
/// does (a statement's `false: next`) gets `false: error`. A raw node written out with both
/// branches to one place, which the compiler leaves alone, is the one case this changes too.
#[must_use]
pub fn with_dead_false_error(compiled: &[Instruction]) -> Vec<Instruction> {
    compiled
        .iter()
        .map(|i| {
            let mut i = i.clone();
            if i.on_false == i.on_true && is_always_true_instruction(&i) {
                i.on_false = TARGET_ERROR;
            }
            i
        })
        .collect()
}

/// The `#[bhav(..)]` header fields that make a tree compile to another tree's nodes exactly
/// ([`match_tree_pin`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreePin {
    /// `dead_false = error`.
    pub dead_false_error: bool,
    /// `share_tails`.
    pub share_tails: bool,
    /// `node_order = [..]`; `None` when the nodes keep their order.
    pub order: Option<Vec<u16>>,
}

impl TreePin {
    /// The fields as written in `#[bhav(..)]`, each after `, `.
    #[must_use]
    pub fn bhav_fields(&self) -> String {
        let mut out = String::new();
        if self.dead_false_error {
            out.push_str(", dead_false = error");
        }
        if self.share_tails {
            out.push_str(", share_tails");
        }
        if let Some(order) = &self.order {
            let order: Vec<String> = order.iter().map(u16::to_string).collect();
            out.push_str(&format!(", node_order = [{}]", order.join(", ")));
        }
        out
    }
}

/// `compiled` as [`with_dead_false_error`] builds it, except that a node keeps its branches when
/// `original` has a node of the same instruction that always branches true and whose false
/// branch is not `error`: the shape a source gives such a node by writing it out in a `raw`
/// block, which `dead_false` leaves alone.
fn with_dead_false_error_keeping(
    compiled: &[Instruction],
    original: &[Instruction],
) -> Vec<Instruction> {
    let kept: Vec<(u16, [u8; 8])> = original
        .iter()
        .filter(|i| i.on_false != TARGET_ERROR && is_always_true_instruction(i))
        .map(|i| (i.opcode, i.operands))
        .collect();
    compiled
        .iter()
        .map(|i| {
            let mut i = i.clone();
            if i.on_false == i.on_true
                && is_always_true_instruction(&i)
                && !kept.contains(&(i.opcode, i.operands))
            {
                i.on_false = TARGET_ERROR;
            }
            i
        })
        .collect()
}

/// Whether two trees hold the same nodes in the same order.
fn same_nodes(a: &[Instruction], b: &[Instruction]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            (a.opcode, a.on_true, a.on_false, a.operands)
                == (b.opcode, b.on_true, b.on_false, b.operands)
        })
}

/// The pin that should make `compiled` (a tree built with none of the pin's fields) compile to
/// `original`'s nodes exactly, in the compiler's order: `dead_false = error` when `original`
/// writes any dead false branch `error`, then `share_tails` when the nodes do not match without
/// it, then a `node_order` when they match in another order. `None` when the nodes already
/// match or no pin makes them; the fewest fields win.
///
/// The pin is a prediction from the bytes, so a caller compiles the tree with it and keeps it
/// only when the nodes come out identical. The bytes cannot tell a statement from a `raw` node
/// written with both branches to one place, which `dead_false` leaves alone, so the flag is
/// tried two ways: every such node flipped ([`with_dead_false_error`]), then the nodes whose
/// instruction `original` leaves with a live false branch kept, which holds when the source
/// writes those out raw. A caller that can compile the source with the flag gets an exact
/// answer from [`match_tree_pin_compiled`].
#[must_use]
pub fn match_tree_pin(compiled: &[Instruction], original: &[Instruction]) -> Option<TreePin> {
    let mut flagged = Vec::new();
    if original
        .iter()
        .any(|i| i.on_false == TARGET_ERROR && is_always_true_instruction(i))
    {
        let flipped = with_dead_false_error(compiled);
        let kept = with_dead_false_error_keeping(compiled, original);
        let differs = !same_nodes(&kept, &flipped);
        flagged.push(flipped);
        if differs {
            flagged.push(kept);
        }
    }
    match_bases(compiled, &flagged, original)
}

/// [`match_tree_pin`] given `flagged`, the same source compiled with `dead_false = error`
/// (and none of the other pin fields): no prediction, so the pin it returns compiles to
/// `original` exactly.
#[must_use]
pub fn match_tree_pin_compiled(
    compiled: &[Instruction],
    flagged: &[Instruction],
    original: &[Instruction],
) -> Option<TreePin> {
    let flagged = if same_nodes(compiled, flagged) {
        Vec::new()
    } else {
        vec![flagged.to_vec()]
    };
    match_bases(compiled, &flagged, original)
}

/// The first pin that turns `compiled`, or one of `flagged` (with `dead_false = error`), into
/// `original`: as it stands, then renumbered, then with `share_tails` (as it stands or
/// renumbered).
fn match_bases(
    compiled: &[Instruction],
    flagged: &[Vec<Instruction>],
    original: &[Instruction],
) -> Option<TreePin> {
    if same_nodes(compiled, original) {
        return None;
    }
    let bases = std::iter::once((false, compiled)).chain(flagged.iter().map(|f| (true, &f[..])));
    for (dead_false_error, base) in bases {
        let pin = |share_tails, order: Vec<u16>| TreePin {
            dead_false_error,
            share_tails,
            order: (!order.iter().enumerate().all(|(i, &n)| usize::from(n) == i)).then_some(order),
        };
        // A tree with a node no branch reaches matches only as it stands.
        if same_nodes(base, original) {
            return Some(pin(false, Vec::new()));
        }
        if let Some(order) = match_node_order(base, original) {
            let pin = pin(false, order);
            // The plain tree in its own order is the same nodes, which `None` covers above.
            if pin.dead_false_error || pin.order.is_some() {
                return Some(pin);
            }
            continue;
        }
        let Some(wide) = widen(base) else {
            continue;
        };
        let Ok(shared) = share_tails(&wide, MAX_BHAV_NODES) else {
            continue;
        };
        if shared.len() < wide.len() {
            let shared = narrow(&shared);
            if same_nodes(&shared, original) {
                return Some(pin(true, Vec::new()));
            }
            if let Some(order) = match_node_order(&shared, original) {
                return Some(pin(true, order));
            }
        }
    }
    None
}

/// A node of an original tree that no branch reaches from node 0, kept as bytes at its node
/// number (`dead_nodes { .. }` in v2, the `.air` header's `dead_nodes: [..]`). Nothing runs it,
/// so its branch bytes are kept as they are: they may name any node, a reserved byte or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeadNode {
    /// Its node number in the finished tree.
    pub at: u8,
    pub opcode: u16,
    pub operands: [u8; 8],
    pub on_true: u8,
    pub on_false: u8,
}

impl DeadNode {
    /// The node as one `dead_nodes` entry, the same in v2 and `.air`:
    /// `5: op 0x0002(0x01, ..) -> true: 6, false: error;`. A branch byte prints as
    /// `error` / `true` / `false` for the three returns, else as its number.
    #[must_use]
    pub fn entry_text(&self) -> String {
        let target = |b: u8| match b {
            TARGET_ERROR => "error".to_owned(),
            TARGET_TRUE => "true".to_owned(),
            TARGET_FALSE => "false".to_owned(),
            n => n.to_string(),
        };
        let operands: Vec<String> = self.operands.iter().map(|b| format!("{b:#04x}")).collect();
        format!(
            "{}: op {:#06x}({}) -> true: {}, false: {};",
            self.at,
            self.opcode,
            operands.join(", "),
            target(self.on_true),
            target(self.on_false)
        )
    }
}

/// The v2 `dead_nodes { .. }` block for `dead`, one entry a line, indented by `indent` (the
/// block's own indent; its entries are 4 deeper), ending in a newline. Empty for no nodes.
#[must_use]
pub fn dead_nodes_block(dead: &[DeadNode], indent: &str) -> String {
    if dead.is_empty() {
        return String::new();
    }
    let mut out = format!("{indent}dead_nodes {{\n");
    for d in dead {
        out.push_str(&format!("{indent}    {}\n", d.entry_text()));
    }
    out.push_str(&format!("{indent}}}\n"));
    out
}

/// Splits `original` into the nodes that node 0 reaches, renumbered in their order, and the
/// rest ([`DeadNode`]s at their node numbers). [`insert_dead_nodes`] puts the tree back
/// together: given the reached nodes as they come back, it rebuilds `original` exactly.
///
/// `None` when a reached node branches to a node the tree does not have or to a reserved byte
/// (the tree is no tree to rebuild), or the tree is longer than a branch byte can number.
/// A tree that every node of is reached comes back whole, with no dead nodes.
#[must_use]
pub fn split_dead_nodes(original: &[Instruction]) -> Option<(Vec<Instruction>, Vec<DeadNode>)> {
    let count = original.len();
    if count > MAX_BHAV_NODES {
        return None;
    }
    if count == 0 {
        return Some((Vec::new(), Vec::new()));
    }
    let mut reached = vec![false; count];
    let mut pending = vec![0usize];
    reached[0] = true;
    while let Some(i) = pending.pop() {
        for b in [original[i].on_true, original[i].on_false] {
            match edge_of(b, count)? {
                Edge::Node(n) if !reached[n] => {
                    reached[n] = true;
                    pending.push(n);
                }
                _ => {}
            }
        }
    }
    let mut number = vec![0u8; count];
    let mut next = 0u8;
    for (i, &r) in reached.iter().enumerate() {
        if r {
            number[i] = next;
            next += 1;
        }
    }
    let renumber = |b: u8| {
        if b >= TARGET_ERROR {
            b
        } else {
            number[usize::from(b)]
        }
    };
    let mut live = Vec::with_capacity(usize::from(next));
    let mut dead = Vec::new();
    for (i, ins) in original.iter().enumerate() {
        if reached[i] {
            live.push(Instruction {
                opcode: ins.opcode,
                on_true: renumber(ins.on_true),
                on_false: renumber(ins.on_false),
                operands: ins.operands,
            });
        } else {
            dead.push(DeadNode {
                at: u8::try_from(i).expect("fits MAX_BHAV_NODES"),
                opcode: ins.opcode,
                operands: ins.operands,
                on_true: ins.on_true,
                on_false: ins.on_false,
            });
        }
    }
    Some((live, dead))
}

/// Checks `dead` as the dead nodes of a tree whose live (reached) part has `live` nodes: each
/// at its own node number below `live + dead.len()` (at most [`MAX_BHAV_NODES`]), and none at
/// node 0, where the tree is entered. The error says what is wrong.
pub(crate) fn check_dead_nodes(dead: &[DeadNode], live: usize) -> Result<(), String> {
    let total = live + dead.len();
    if total > MAX_BHAV_NODES {
        return Err(format!(
            "the tree has {live} node(s) and {} dead node(s); a tree holds at most {MAX_BHAV_NODES}",
            dead.len()
        ));
    }
    let mut seen = vec![false; total];
    for d in dead {
        let at = usize::from(d.at);
        if at == 0 {
            return Err(
                "dead node 0: a tree runs from node 0, so its entry cannot be a dead node"
                    .to_owned(),
            );
        }
        if at >= total {
            return Err(format!(
                "dead node {at} is past the tree's last node: it has {total} (0..={})",
                total - 1
            ));
        }
        if std::mem::replace(&mut seen[at], true) {
            return Err(format!("dead node {at} is listed twice"));
        }
    }
    Ok(())
}

/// The tree `live` (its nodes in order, checked with [`check_dead_nodes`]) with `dead` put at
/// their node numbers: the live nodes fill the other numbers in order, and each live branch
/// follows its node. A dead node keeps its bytes, so no live branch can reach it.
pub(crate) fn insert_dead_nodes(live: &[WideNode], dead: &[DeadNode]) -> Vec<Instruction> {
    let total = live.len() + dead.len();
    let mut slots: Vec<Option<&DeadNode>> = vec![None; total];
    for d in dead {
        slots[usize::from(d.at)] = Some(d);
    }
    let number: Vec<usize> = (0..total).filter(|&i| slots[i].is_none()).collect();
    let byte = |e: Edge| match e {
        Edge::Node(n) => number[n] as u8,
        Edge::Exit(b) => b,
    };
    let mut live = live.iter();
    slots
        .iter()
        .map(|slot| match slot {
            Some(d) => Instruction {
                opcode: d.opcode,
                on_true: d.on_true,
                on_false: d.on_false,
                operands: d.operands,
            },
            None => {
                let n = live.next().expect("one live node per free number");
                Instruction {
                    opcode: n.opcode,
                    on_true: byte(n.on_true),
                    on_false: byte(n.on_false),
                    operands: n.operands,
                }
            }
        })
        .collect()
}

/// A pin for a tree whose original keeps nodes no branch reaches: the header fields
/// ([`TreePin`]) for its reached nodes, and the [`DeadNode`]s that go back in after them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedPin {
    /// The `#[bhav(..)]` fields; `None` when the reached nodes already match as compiled.
    pub pin: Option<TreePin>,
    /// The original's unreached nodes, for a trailing `dead_nodes { .. }` block
    /// ([`dead_nodes_block`]); empty when every node is reached.
    pub dead_nodes: Vec<DeadNode>,
}

/// [`match_tree_pin`] for an original that may keep nodes no branch reaches: its reached
/// nodes ([`split_dead_nodes`]) are matched as `match_tree_pin` matches a whole tree, and
/// the rest come back as dead nodes. `compiled` is the tree built with none of the pin's
/// fields and no dead nodes. `None` when nothing needs pinning (the nodes match and none is
/// dead) or no pin makes them match. The pin is a prediction from the bytes, like
/// `match_tree_pin`'s: a caller builds the tree with it and keeps it only when the nodes come
/// out identical.
#[must_use]
pub fn match_tree_pin_quarantined(
    compiled: &[Instruction],
    original: &[Instruction],
) -> Option<QuarantinedPin> {
    let (live, dead_nodes) = split_dead_nodes(original)?;
    if dead_nodes.is_empty() {
        return match_tree_pin(compiled, original).map(|pin| QuarantinedPin {
            pin: Some(pin),
            dead_nodes,
        });
    }
    let pin = if same_nodes(compiled, &live) {
        None
    } else {
        Some(match_tree_pin(compiled, &live)?)
    };
    Some(QuarantinedPin { pin, dead_nodes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::{TARGET_FALSE, TARGET_TRUE};

    fn bytes(tree: &[Instruction]) -> Vec<(u16, u8, u8)> {
        tree.iter()
            .map(|i| (i.opcode, i.on_true, i.on_false))
            .collect()
    }

    fn ins(opcode: u16, on_true: u8, on_false: u8) -> Instruction {
        Instruction {
            opcode,
            on_true,
            on_false,
            operands: [0; 8],
        }
    }

    #[test]
    fn share_tails_merges_identical_tails_and_loops() {
        // 0 -> 1 | 3; 1 -> 2; 2 returns; 3 -> 4; 4 returns: 3/4 copy 1/2.
        let tree = [
            ins(9, 1, 3),
            ins(5, 2, 2),
            ins(6, TARGET_TRUE, TARGET_FALSE),
            ins(5, 4, 4),
            ins(6, TARGET_TRUE, TARGET_FALSE),
        ];
        let merged = narrow(&share_tails(&widen(&tree).unwrap(), MAX_BHAV_NODES).unwrap());
        assert_eq!(
            bytes(&merged),
            [(9, 1, 1), (5, 2, 2), (6, TARGET_TRUE, TARGET_FALSE)]
        );
        // Two copies of a two-node loop merge too.
        let loops = [
            ins(9, 1, 3),
            ins(5, 2, TARGET_TRUE),
            ins(6, 1, TARGET_FALSE),
            ins(5, 4, TARGET_TRUE),
            ins(6, 3, TARGET_FALSE),
        ];
        let merged = narrow(&share_tails(&widen(&loops).unwrap(), MAX_BHAV_NODES).unwrap());
        assert_eq!(
            bytes(&merged),
            [(9, 1, 1), (5, 2, TARGET_TRUE), (6, 1, TARGET_FALSE)]
        );
        // Different returns stay apart.
        let apart = [
            ins(9, 1, 2),
            ins(6, TARGET_TRUE, TARGET_TRUE),
            ins(6, TARGET_FALSE, TARGET_FALSE),
        ];
        assert_eq!(
            share_tails(&widen(&apart).unwrap(), MAX_BHAV_NODES)
                .unwrap()
                .len(),
            3
        );
        // Refinement stops once the classes pass the limit.
        assert_eq!(share_tails(&widen(&apart).unwrap(), 2), Err(3));
        // Reserved bytes are not nodes.
        assert!(widen(&[ins(9, 251, TARGET_TRUE)]).is_none());
    }

    #[test]
    fn node_order_round_trips_through_match() {
        let tree = [
            ins(9, 2, 1),
            ins(5, TARGET_TRUE, 0),
            ins(6, 1, TARGET_FALSE),
        ];
        let order = [0u16, 2, 1];
        check_node_order(&order, 3).unwrap();
        let moved = narrow(&apply_node_order(&widen(&tree).unwrap(), &order));
        assert_eq!(
            bytes(&moved),
            [(9, 1, 2), (6, 2, TARGET_FALSE), (5, TARGET_TRUE, 0)]
        );
        assert_eq!(match_node_order(&tree, &moved), Some(order.to_vec()));
        assert_eq!(match_node_order(&tree, &tree), Some(vec![0, 1, 2]));
        // A different instruction or return does not match.
        let mut other = moved.clone();
        other[1].on_false = TARGET_TRUE;
        assert_eq!(match_node_order(&tree, &other), None);
    }

    #[test]
    fn node_order_is_checked() {
        assert!(
            check_node_order(&[0, 1], 3)
                .unwrap_err()
                .contains("lists 2 node(s)")
        );
        assert!(
            check_node_order(&[0, 1, 3], 3)
                .unwrap_err()
                .contains("names node 3")
        );
        assert!(
            check_node_order(&[0, 1, 1], 3)
                .unwrap_err()
                .contains("twice")
        );
        assert!(
            check_node_order(&[1, 0, 2], 3)
                .unwrap_err()
                .contains("start with 0")
        );
    }
}
