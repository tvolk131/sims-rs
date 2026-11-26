//! Node-graph rewrites of one compiled tree, applied after the `.air` compiler has assembled
//! it (`docs/decisions/20261126-antics-v2-3-step2-node-order-and-shared-tails.md`):
//!
//! - `share_tails` merges the nodes that run alike: the same instruction, whose branches lead
//!   to nodes that run alike or return the same way. The tree runs the same instructions in
//!   the same order on every run; only the node numbers change.
//! - `node_order` renumbers the nodes: entry `i` names the compiled node that becomes node `i`.
//!
//! [`match_node_order`] finds the order that turns one tree into another with the same nodes,
//! for a decompiler that pins a lifted tree's node numbers to the original's.

use sims_formats::behavior::Instruction;

use crate::lower::{MAX_BHAV_NODES, TARGET_ERROR};

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
