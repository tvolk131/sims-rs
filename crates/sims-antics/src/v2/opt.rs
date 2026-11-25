//! Backend pass over the `.air` text v2 lowering emits for one tree body.
//!
//! The lowering stays simple and this pass removes what it leaves behind. Every rewrite here
//! preserves the tree's behaviour exactly:
//! - **Unreachable statements** after `return` / `break` / `continue` in the same block are
//!   dropped. Their `let` declarations stay, as `let x;`, which emits no node.
//! - **Dead `stack_obj` stores.** A store directly before `return` is dead: the frame's stack
//!   object is popped with the frame, and `return` sets the result. A store directly before
//!   another store is dead too. A store that straight-line code proves redundant (`stack_obj`
//!   already holds that value) is dropped when the next statement cannot read the last result.
//! - **Unread temps.** A compiler temp that is only ever written from a plain, non-faulting
//!   operand loses those writes. Its `let` stays only while something else names it.
//! - **Duplicate tails.** `if c { A; T } else { B; T }` becomes `if c { A } else { B } T`.
//! - **Local coalescing.** Locals whose live ranges never overlap share one `.air` local:
//!   compiler temps with each other, source bindings of the same name, and bindings of different
//!   names that belong to inline helpers. A merged `let y = e;` becomes `x := e;`, the same node.
//!
//! The passes work on the MIR (`v2::mir`: the body parsed into structured `Stmt`s, with the
//! typed instruction view, effects and liveness analyses there too). These passes leave a tree
//! the MIR does not model exactly untouched: labels on their own line, explicit branch targets (`->`), comments,
//! or more than one statement on a line (some `raw { }` blocks). The entry-save and frame-allocation passes
//! live in `v2::mir` beside the backend pipeline that runs them.

use std::collections::{BTreeMap, BTreeSet};

use super::mir::*;

/// Optimizes one lowered tree body (the lines between the tree's braces, indented by 4 per
/// level from 4). Returns the input unchanged when the body is outside the modelled subset or
/// nothing applies.
///
/// `text_reads` says the tree's text may read frame words (`mir::FrameFacts::text_reads`); when it
/// does not, a dialog or action string reads no local and the passes treat it like any other
/// primitive. `origins` names the declaration each inlined helper local copies
/// (`mir::FrameFacts::local_origins`). `decl_groups` lets out-operand locals share words only
/// with copies of their own declaration; without it they coalesce as on main. `main_rules`
/// keeps main's rules (`mir::FrameFacts::main_rules`), as a body that may read global 34 does.
/// `keep_exit_stack_obj` keeps every `stack_obj` store that only the exit makes dead
/// (`mir::FrameFacts::keeps_exit_stack_obj`).
pub(crate) fn optimize_tree_body(
    body: &str,
    text_reads: bool,
    origins: &BTreeMap<String, String>,
    decl_groups: bool,
    main_rules: bool,
    keep_exit_stack_obj: bool,
) -> String {
    let Some(mut nodes) = parse_body(body) else {
        return body.to_owned();
    };
    let before = nodes.clone();
    // A tree that may read global 34 keeps main's rules (`out_defs` off, literal draw ranges):
    // the read depends on the scratch pointer an Expression sets, so the tree keeps or drops
    // an Expression exactly where main does.
    let out_defs = !(main_rules || may_read_global_34(body));
    let ranges = DrawRanges::for_tree(!out_defs);
    for _ in 0..8 {
        let snapshot = nodes.clone();
        drop_unreachable(&mut nodes);
        forward_entry_save(&mut nodes);
        drop_dead_stack_obj_stores(&mut nodes, keep_exit_stack_obj);
        if !keep_exit_stack_obj {
            drop_stack_obj_stores_before_exit(&mut nodes, false);
        }
        unwrap_single_pass_loops(&mut nodes);
        drop_dead_local_stores(&mut nodes, text_reads, out_defs);
        forward_copies(&mut nodes, text_reads, out_defs);
        merge_duplicate_tails(&mut nodes);
        coalesce_locals(&mut nodes, text_reads, origins, out_defs, decl_groups);
        drop_unused_decls(&mut nodes, text_reads);
        drop_unread_entry_save_node(&mut nodes, text_reads, ranges);
        if nodes == snapshot {
            break;
        }
    }
    // Last, so no later pass rewrites the node an identity store was checked against.
    drop_identity_stores(&mut nodes);
    if nodes == before {
        return body.to_owned();
    }
    let mut out = String::with_capacity(body.len());
    print_nodes(&nodes, 1, &mut out);
    out
}

fn drop_unreachable(nodes: &mut Vec<Stmt>) {
    for_each_block(nodes, &mut |block| {
        let Some(cut) = block.iter().position(|n| {
            matches!(n, Stmt::Inst(l) if matches!(classify(l), InstKind::Return | InstKind::Break | InstKind::Continue))
        }) else {
            return;
        };
        if cut + 1 >= block.len() {
            return;
        }
        let dropped: Vec<Stmt> = block.drain(cut + 1..).collect();
        let mut names = Vec::new();
        collect_lets(&dropped, &mut names);
        let decls: Vec<Stmt> = names
            .into_iter()
            .map(|n| Stmt::Inst(format!("let {n};")))
            .collect();
        // Keep the declarations (no nodes) ahead of the jump; unused ones are dropped later.
        block.splice(cut..cut, decls);
    });
}

/// With `keep_exit`, a `return` reads the stack object (the source's exit value), so only a store
/// overwritten first, or one that changes nothing, goes.
fn drop_dead_stack_obj_stores(nodes: &mut Vec<Stmt>, keep_exit: bool) {
    for_each_block(nodes, &mut |block| {
        let mut i = 0;
        // What `stack_obj` provably equals here (a local name or `my.id`), in straight-line code.
        let mut known: Option<String> = None;
        while i < block.len() {
            let next = block[i + 1..].iter().find(|n| !is_decl(n));
            let Stmt::Inst(line) = &block[i] else {
                known = None;
                i += 1;
                continue;
            };
            if let Some(rhs) = stack_obj_store(line) {
                let safe = is_safe_operand(rhs) || rhs == "stack_obj";
                let next_is_return = !keep_exit
                    && matches!(next, Some(Stmt::Inst(l)) if classify(l) == InstKind::Return);
                // The next store must not read the value this one writes.
                let next_is_store = matches!(next, Some(Stmt::Inst(l))
                    if stack_obj_store(l).is_some_and(|r| r != "stack_obj" && is_safe_operand(r)));
                let redundant = known.as_deref() == Some(rhs) || rhs == "stack_obj";
                // Nothing reads the value before it is overwritten or the frame returns.
                let unread = stack_obj_fate(&block[i + 1..], keep_exit) == StackObjFate::Dead;
                if safe
                    && (next_is_return
                        || next_is_store
                        || ((redundant || unread) && next_ignores_last_result(&block[i + 1..])))
                {
                    block.remove(i);
                    continue;
                }
                known = (is_ident(rhs) || rhs == "my.id").then(|| rhs.to_owned());
                i += 1;
                continue;
            }
            match classify(line) {
                InstKind::Let {
                    name,
                    init: Some("stack_obj"),
                } => known = Some(name.to_owned()),
                InstKind::Assign {
                    lhs,
                    op: ":=",
                    rhs: "stack_obj",
                } if is_ident(lhs) => known = Some(lhs.to_owned()),
                // Any other write to `stack_obj` (`+=`, ...) leaves its value unknown.
                InstKind::Assign {
                    lhs: "stack_obj", ..
                } => known = None,
                InstKind::Let { name, .. } | InstKind::Assign { lhs: name, .. } => {
                    if known.as_deref() == Some(name) {
                        known = None;
                    }
                }
                _ => known = None,
            }
            i += 1;
        }
    });
}

/// What happens to the stack object after a point, scanning forward in one block.
#[derive(PartialEq, Eq)]
enum StackObjFate {
    /// Overwritten or the frame returns before anything can read it, on every path.
    Dead,
    /// Something may read it.
    Live,
    /// Control falls off the end of the block without reading it.
    Through,
}

/// With `keep_exit`, a `return` reads it.
fn stack_obj_fate(nodes: &[Stmt], keep_exit: bool) -> StackObjFate {
    for n in nodes {
        match n {
            Stmt::Inst(l) => {
                if let Some(rhs) = stack_obj_store(l)
                    && !may_read_stack_obj(rhs)
                {
                    return StackObjFate::Dead;
                }
                match classify(l) {
                    InstKind::Return if !keep_exit && !may_read_stack_obj(l) => {
                        return StackObjFate::Dead;
                    }
                    InstKind::Return => return StackObjFate::Live,
                    InstKind::Break | InstKind::Continue => return StackObjFate::Live,
                    _ if may_read_stack_obj(l) => return StackObjFate::Live,
                    _ => {}
                }
            }
            Stmt::If { arms, else_body } => {
                let mut all_dead = else_body.is_some();
                for (cond, body) in arms {
                    if may_read_stack_obj(cond) {
                        return StackObjFate::Live;
                    }
                    match stack_obj_fate(body, keep_exit) {
                        StackObjFate::Live => return StackObjFate::Live,
                        StackObjFate::Through => all_dead = false,
                        StackObjFate::Dead => {}
                    }
                }
                if let Some(body) = else_body {
                    match stack_obj_fate(body, keep_exit) {
                        StackObjFate::Live => return StackObjFate::Live,
                        StackObjFate::Through => all_dead = false,
                        StackObjFate::Dead => {}
                    }
                }
                if all_dead {
                    return StackObjFate::Dead;
                }
            }
            Stmt::While { .. } => return StackObjFate::Live,
        }
    }
    StackObjFate::Through
}

/// A plain `stack_obj := x;` store that is dead when control next reaches a `return` (the frame,
/// and its stack object, goes away; `return` sets the result).
fn is_droppable_stack_obj_store(n: &Stmt) -> bool {
    matches!(n, Stmt::Inst(l) if stack_obj_store(l).is_some_and(|r| is_safe_operand(r) || r == "stack_obj"))
}

/// Drops `stack_obj` stores that end an `if` arm when the `if` itself is followed by `return`
/// (`if c { ..; stack_obj := s; } return x;`), at any depth, and then an `else` left empty.
/// `tail_returns` says that falling off the end of `nodes` reaches a `return`.
fn drop_stack_obj_stores_before_exit(nodes: &mut Vec<Stmt>, tail_returns: bool) {
    for i in 0..nodes.len() {
        let next = nodes[i + 1..].iter().find(|n| !is_decl(n));
        let next_returns = match next {
            Some(Stmt::Inst(l)) => classify(l) == InstKind::Return,
            Some(_) => false,
            None => tail_returns,
        };
        match &mut nodes[i] {
            Stmt::If { arms, else_body } => {
                for (_, body) in arms.iter_mut() {
                    drop_stack_obj_stores_before_exit(body, next_returns);
                }
                if let Some(body) = else_body {
                    drop_stack_obj_stores_before_exit(body, next_returns);
                    if body.is_empty() {
                        *else_body = None;
                    }
                }
            }
            Stmt::While { body, .. } => drop_stack_obj_stores_before_exit(body, false),
            Stmt::Inst(_) => {}
        }
    }
    if tail_returns {
        while let Some(pos) = nodes.iter().rposition(|n| !is_decl(n)) {
            if !is_droppable_stack_obj_store(&nodes[pos]) {
                break;
            }
            nodes.remove(pos);
        }
    }
}

/// `while true { B; break; }` with no other exit from `B` runs `B` once: it becomes `B`. The
/// condition-free loop emits no node, and the `break` targets what follows the loop, which is
/// where `B` falls through to.
fn unwrap_single_pass_loops(nodes: &mut Vec<Stmt>) {
    for_each_block(nodes, &mut |block| {
        let mut i = 0;
        while i < block.len() {
            let unwrap = match &block[i] {
                Stmt::While { cond: None, body } => {
                    matches!(body.last(), Some(Stmt::Inst(l)) if classify(l) == InstKind::Break)
                        && !has_loop_exit(&body[..body.len() - 1])
                }
                _ => false,
            };
            if !unwrap {
                i += 1;
                continue;
            }
            let Stmt::While { mut body, .. } = block.remove(i) else {
                unreachable!("checked above");
            };
            body.pop();
            let count = body.len();
            block.splice(i..i, body);
            i += count;
        }
    });
}

/// Drops plain stores (`let x = e;`, `x := e;` from a non-faulting operand) to v2 locals that
/// are dead after the store. A dropped `let` keeps its declaration, which emits no node.
fn drop_dead_local_stores(nodes: &mut Vec<Stmt>, text_reads: bool, out_defs: bool) {
    // An `#[slot]` local's stores stay: the word is pinned so a frame matches one that other
    // code (a raw block, a later tree reusing the frame) reads by number.
    let mut names = Vec::new();
    collect_lets(nodes, &mut names);
    let locals: BTreeSet<String> = names
        .into_iter()
        .filter(|n| !n.starts_with("__slot"))
        .collect();
    let mut lv = Liveness {
        locals: &locals,
        text_reads_locals: text_reads && nodes_format_text(nodes),
        out_defs,
        edges: BTreeSet::new(),
        dead_defs: BTreeMap::new(),
        live_out: BTreeMap::new(),
    };
    lv.block(nodes, Set::new(), &Set::new(), &Set::new());
    let dead: BTreeSet<*const Stmt> = lv
        .dead_defs
        .into_iter()
        .filter_map(|(p, dead)| dead.then_some(p))
        .collect();
    if dead.is_empty() {
        return;
    }
    for_each_block(nodes, &mut |block| {
        // Back to front, so the addresses of the nodes not yet visited stay put.
        for i in (0..block.len()).rev() {
            if !dead.contains(&std::ptr::from_ref(&block[i]))
                || !next_ignores_last_result(&block[i + 1..])
            {
                continue;
            }
            let Stmt::Inst(l) = &block[i] else {
                continue;
            };
            let (name, safe) = match classify(l) {
                InstKind::Let {
                    name,
                    init: Some(init),
                } => (Some(name.to_owned()), is_safe_operand(init)),
                InstKind::Assign { rhs, .. } => (None, is_safe_operand(rhs) || rhs == "stack_obj"),
                _ => continue,
            };
            if !safe {
                continue;
            }
            match name {
                Some(name) => block[i] = Stmt::Inst(format!("let {name};")),
                None => {
                    block.remove(i);
                }
            }
        }
    });
}

/// `t := e; d := t;` with `t` a v2 local that is dead afterwards becomes `d := e;`: one
/// expression node reading `e` where two read it in turn, with nothing in between.
fn forward_copies(nodes: &mut Vec<Stmt>, text_reads: bool, out_defs: bool) {
    // An `#[slot]` local's stores stay: the word is pinned so a frame matches one that other
    // code (a raw block, a later tree reusing the frame) reads by number.
    let mut names = Vec::new();
    collect_lets(nodes, &mut names);
    let locals: BTreeSet<String> = names
        .into_iter()
        .filter(|n| !n.starts_with("__slot"))
        .collect();
    let mut lv = Liveness {
        locals: &locals,
        text_reads_locals: text_reads && nodes_format_text(nodes),
        out_defs,
        edges: BTreeSet::new(),
        dead_defs: BTreeMap::new(),
        live_out: BTreeMap::new(),
    };
    lv.block(nodes, Set::new(), &Set::new(), &Set::new());
    let live_out = lv.live_out;
    for_each_block(nodes, &mut |block| {
        // Back to front, so the addresses of the nodes not yet visited stay put. A forwarded
        // pair moves the merged line onto an address whose recorded live set is stale, so the
        // pair before it waits for the next round.
        let mut skip = false;
        for i in (0..block.len().saturating_sub(1)).rev() {
            if std::mem::take(&mut skip) {
                continue;
            }
            let (Stmt::Inst(first), Stmt::Inst(second)) = (&block[i], &block[i + 1]) else {
                continue;
            };
            let Some((t, e, t_is_let)) = plain_store(first) else {
                continue;
            };
            let Some((d, rhs, d_is_let)) = plain_store(second) else {
                continue;
            };
            let forward = t.starts_with("__")
                && locals.contains(t)
                && rhs == t
                && d != t
                && (!e.contains(' ') || e.starts_with("scope("))
                && !idents(d).contains(&t)
                && live_out
                    .get(&std::ptr::from_ref(&block[i + 1]))
                    .is_some_and(|live| !live.contains(t));
            if !forward {
                continue;
            }
            let merged = if d_is_let {
                format!("let {d} = {e};")
            } else {
                format!("{d} := {e};")
            };
            let decl = t_is_let.then(|| format!("let {t};"));
            block[i + 1] = Stmt::Inst(merged);
            skip = true;
            match decl {
                Some(decl) => block[i] = Stmt::Inst(decl),
                None => {
                    block.remove(i);
                }
            }
        }
    });
}

/// Drops `temp[i] := temp[i];` for a temp the VM always has (0 to 7), which forwarding a staged
/// argument copy leaves behind: the store changes no cell and cannot fault. Other cells stay: an
/// out-of-range temp, a parameter past the frame's arguments, or a local holding half of a stale
/// pointer faults on the read.
///
/// Every Expression node also sets the object's scratch pointer (`[object + 0x128]`), which a
/// later Expression reading global 34 depends on (`sims-vm` `primitives/expression.rs`). So the
/// store is only dropped when the next node is itself a call-free Expression that does not touch
/// a global or raw scope: that node ignores the last result and leaves the scratch pointer
/// exactly as the dropped store would have.
fn drop_identity_stores(nodes: &mut Vec<Stmt>) {
    for_each_block(nodes, &mut |block| {
        for i in (0..block.len()).rev() {
            let Stmt::Inst(l) = &block[i] else {
                continue;
            };
            let InstKind::Assign { lhs, op: ":=", rhs } = classify(l) else {
                continue;
            };
            let in_range_temp = lhs
                .strip_prefix("temp[")
                .and_then(|r| r.strip_suffix(']'))
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| n < 8);
            if lhs == rhs && in_range_temp && next_is_plain_expression(&block[i + 1..]) {
                block.remove(i);
            }
        }
    });
}

/// The next node (skipping declarations) is an Expression assignment or initializer whose text
/// names no call, raw scope, or global.
fn next_is_plain_expression(rest: &[Stmt]) -> bool {
    let Some(Stmt::Inst(l)) = rest.iter().find(|n| !is_decl(n)) else {
        return false;
    };
    let text = strip_strings(l);
    matches!(
        classify(l),
        InstKind::Assign { .. } | InstKind::Let { init: Some(_), .. }
    ) && !text.contains('(')
        && !text.contains("global")
}

/// Drops `let x;` declarations of names nothing else mentions.
fn drop_unused_decls(nodes: &mut Vec<Stmt>, text_reads: bool) {
    // A tree that formats text may read a user local's frame word by number, so its user
    // locals keep their declarations (and so their words) even when nothing names them.
    let keep_user = text_reads && nodes_format_text(nodes);
    // Only the names to drop are copied out of the borrowed counts.
    let unused: BTreeSet<String> = {
        let mut counts = BTreeMap::new();
        mention_counts(nodes, &mut counts);
        counts
            .into_iter()
            .filter(|&(name, _)| !(keep_user && is_source_local(name)))
            // A pinned `#[slot]` word stays declared: dropping it would free its number.
            .filter(|&(name, _)| !name.starts_with("__slot"))
            .filter(|&(_, (reads, writes))| reads == 0 && writes <= 1)
            .map(|(name, _)| name.to_owned())
            .collect()
    };
    for_each_block(nodes, &mut |block| {
        if !unused.is_empty() {
            block.retain(|n| match n {
                Stmt::Inst(l) => match classify(l) {
                    InstKind::Let { name, init: None } => !unused.contains(name),
                    _ => true,
                },
                _ => true,
            });
        }
        // `let x;` right before `x := e;` is spelled `let x = e;` (the same node).
        let mut i = 0;
        while i + 1 < block.len() {
            if let (Stmt::Inst(decl), Stmt::Inst(store)) = (&block[i], &block[i + 1])
                && let InstKind::Let { name, init: None } = classify(decl)
                && let InstKind::Assign { lhs, op: ":=", rhs } = classify(store)
                && lhs == name
                && !idents(rhs).contains(&name)
            {
                block[i] = Stmt::Inst(format!("let {name} = {rhs};"));
                block.remove(i + 1);
            }
            i += 1;
        }
    });
}

fn merge_duplicate_tails(nodes: &mut Vec<Stmt>) {
    for_each_block(nodes, &mut |block| {
        let mut i = 0;
        while i < block.len() {
            let hoisted = match &mut block[i] {
                Stmt::If {
                    arms,
                    else_body: Some(else_body),
                } if arms.len() == 1 => {
                    let then_body = &mut arms[0].1;
                    let mut n = 0;
                    while n < then_body.len()
                        && n < else_body.len()
                        && matches!(&then_body[then_body.len() - 1 - n], Stmt::Inst(_))
                        && then_body[then_body.len() - 1 - n] == else_body[else_body.len() - 1 - n]
                    {
                        n += 1;
                    }
                    if n == 0 {
                        Vec::new()
                    } else {
                        let tail: Vec<Stmt> = then_body.drain(then_body.len() - n..).collect();
                        else_body.truncate(else_body.len() - n);
                        tail
                    }
                }
                _ => Vec::new(),
            };
            if let Stmt::If { else_body, .. } = &mut block[i]
                && else_body.as_ref().is_some_and(Vec::is_empty)
            {
                // An empty `else` emits nothing; drop it so the text stays tidy.
                *else_body = None;
            }
            let count = hoisted.len();
            block.splice(i + 1..i + 1, hoisted);
            i += 1 + count;
        }
    });
}

fn coalesce_locals(
    nodes: &mut Vec<Stmt>,
    text_reads: bool,
    origins: &BTreeMap<String, String>,
    out_defs: bool,
    decl_groups: bool,
) {
    let mut order = Vec::new();
    collect_lets(nodes, &mut order);
    let mut seen = BTreeSet::new();
    order.retain(|n| seen.insert(n.clone()));
    let locals: BTreeSet<String> = order.iter().cloned().collect();
    let mut lv = Liveness {
        locals: &locals,
        text_reads_locals: text_reads && nodes_format_text(nodes),
        out_defs,
        edges: BTreeSet::new(),
        dead_defs: BTreeMap::new(),
        live_out: BTreeMap::new(),
    };
    let entry_live = lv.block(nodes, Set::new(), &Set::new(), &Set::new());
    // A tree that may read global 34 (`out_defs` off) or fixes its frame size shares
    // out-operand locals as main does.
    let mut out_written = BTreeSet::new();
    if out_defs && decl_groups {
        collect_out_written(nodes, &locals, &mut out_written);
    }
    // A copy of a declaration that some copy writes through an out-operand counts as one too
    // (a specialization may drop the write from one copy): copies of a declaration stay one
    // group.
    let written_origins: BTreeSet<&String> =
        out_written.iter().filter_map(|n| origins.get(n)).collect();
    out_written.extend(
        locals
            .iter()
            .filter(|n| origins.get(*n).is_some_and(|o| written_origins.contains(o)))
            .cloned(),
    );
    let interferes = |a: &str, b: &str| {
        let (x, y) = if a < b { (a, b) } else { (b, a) };
        lv.edges.contains(&(x.to_owned(), y.to_owned()))
    };

    // Greedy, in declaration order: each local joins the first earlier class of its group that
    // it does not interfere with. Inlined helper locals may also share a class across groups,
    // except a local a primitive writes through an out-operand (`animate(local:)`,
    // `random`, `relationship(value:)`): it shares only with copies of the same source local
    // (another inlined copy of the helper), as distinct named locals keep distinct words. Its
    // group is its declaration, so another helper's local of the same spelling is not a copy.
    let mut classes: Vec<(String, Vec<String>)> = Vec::new();
    let mut rename: BTreeMap<String, String> = BTreeMap::new();
    for name in &order {
        let Some(group) = merge_group(name) else {
            continue;
        };
        let group = match origins.get(name) {
            Some(origin) if out_written.contains(name) => format!("decl:{origin}"),
            _ => group,
        };
        if entry_live.contains(name) {
            continue;
        }
        let fits = |members: &[String]| members.iter().all(|m| !interferes(m, name));
        let target = classes
            .iter()
            .position(|(g, members)| *g == group && fits(members))
            .or_else(|| {
                if lv.text_reads_locals
                    || !name.starts_with("__inline_v_")
                    || out_written.contains(name)
                {
                    return None;
                }
                classes.iter().position(|(_, members)| {
                    members
                        .iter()
                        .all(|m| m.starts_with("__inline_v_") && !out_written.contains(m))
                        && fits(members)
                })
            });
        match target {
            Some(index) => {
                let members = &mut classes[index].1;
                rename.insert(name.clone(), members[0].clone());
                members.push(name.clone());
            }
            None => classes.push((group, vec![name.clone()])),
        }
    }
    if rename.is_empty() {
        return;
    }
    rewrite(nodes, &rename);
}

/// The locals among `locals` that some primitive in `nodes` (a statement or a branch condition)
/// writes through an out-operand.
fn collect_out_written(nodes: &[Stmt], locals: &BTreeSet<String>, out: &mut BTreeSet<String>) {
    fn note(text: &str, locals: &BTreeSet<String>, out: &mut BTreeSet<String>) {
        let text = text.trim_start();
        let text = text.strip_prefix('!').unwrap_or(text);
        for name in idents(text) {
            if locals.contains(name) && out_operand_write(text, name).is_some() {
                out.insert(name.to_owned());
            }
        }
    }
    for n in nodes {
        match n {
            Stmt::Inst(l) => note(l, locals, out),
            Stmt::If { arms, else_body } => {
                for (cond, body) in arms {
                    note(cond, locals, out);
                    collect_out_written(body, locals, out);
                }
                if let Some(body) = else_body {
                    collect_out_written(body, locals, out);
                }
            }
            Stmt::While { cond, body } => {
                if let Some(c) = cond {
                    note(c, locals, out);
                }
                collect_out_written(body, locals, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DrawRanges, drop_unread_entry_save, optimize_tree_body, parse_body, pin_in_place};
    use std::collections::BTreeMap;

    #[test]
    fn dead_stores_unread_self_obj_and_coalescing() {
        let body = "    let __self_obj = stack_obj;\n    let __a_1 = 0;\n    __a_1 := my.attr[0];\n    my.attr[1] := __a_1;\n    my.attr[4] := __a_1;\n    let __b_2 = my.attr[2];\n    my.attr[3] := __b_2;\n    my.attr[5] := __b_2;\n    stack_obj := __self_obj;\n    return true;\n";
        let out = optimize_tree_body(body, true, &BTreeMap::new(), true, false, false);
        assert_eq!(
            out,
            "    let __a_1 = my.attr[0];\n    my.attr[1] := __a_1;\n    my.attr[4] := __a_1;\n    __a_1 := my.attr[2];\n    my.attr[3] := __a_1;\n    my.attr[5] := __a_1;\n    return true;\n"
        );
    }

    #[test]
    fn single_pass_loops_unwrap_and_dead_flags_drop() {
        let body = "    let __r_1;\n    let __ret_2 = 0;\n    while true {\n        __r_1 := my.attr[0];\n        __ret_2 := 1;\n        break;\n    }\n    my.attr[1] := __r_1;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            "    my.attr[1] := my.attr[0];\n    return true;\n"
        );
        // A second exit keeps the loop (its `break` has nowhere else to go).
        let body = "    while true {\n        if my.attr[0] == 1 {\n            break;\n        }\n        my.attr[1] := 2;\n        break;\n    }\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            body
        );
    }

    #[test]
    fn copies_forward_only_when_the_temp_dies() {
        let body = "    let __t_1 = stack.attr[0];\n    my.attr[0] := __t_1;\n    my.attr[1] := __t_1;\n    let __u_2 = my.attr[2];\n    my.attr[3] := __u_2;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            "    let __t_1 = stack.attr[0];\n    my.attr[0] := __t_1;\n    my.attr[1] := __t_1;\n    my.attr[3] := my.attr[2];\n    return true;\n"
        );
    }

    #[test]
    fn stores_before_primitives_stay() {
        // `idle` may observe the previous node's result, so the dead store before it stays.
        let body = "    let __a_1 = my.attr[0];\n    idle(1);\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            body
        );
    }

    #[test]
    fn leaves_labels_and_multi_statement_lines_alone() {
        for body in [
            "    let __a_1 = 0;\n    my.attr[0] := 1; call 256(temps);\n    return true;\n",
            "    let __a_1 = 0;\n    x:\n    return true;\n",
        ] {
            assert_eq!(
                optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
                body
            );
        }
    }

    #[test]
    fn duplicate_tails_and_unreachable() {
        let body = "    if my.attr[0] == 1 {\n        my.attr[1] := 2;\n        my.attr[2] := 3;\n    } else {\n        my.attr[2] := 3;\n    }\n    return true;\n    my.attr[3] := 4;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            "    if my.attr[0] == 1 {\n        my.attr[1] := 2;\n    }\n    my.attr[2] := 3;\n    return true;\n"
        );
    }

    #[test]
    fn loop_carried_locals_do_not_merge() {
        // `__a_1` is live around the loop back edge while `__b_2` is written in the body.
        let body = "    let __a_1 = 0;\n    while my.attr[0] < 5 {\n        let __b_2 = my.attr[1];\n        my.attr[2] := __b_2;\n        my.attr[4] := __b_2;\n        __a_1 += 1;\n    }\n    my.attr[3] := __a_1;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            body
        );
    }

    #[test]
    fn stack_obj_store_read_by_the_next_store_stays() {
        // The second store reads the first one's value through `stack.`.
        let body = "    stack_obj := __o_1;\n    stack_obj := stack.attr[0];\n    my.attr[0] := 1;\n    return true;\n";
        let out = optimize_tree_body(body, true, &BTreeMap::new(), true, false, false);
        assert!(out.contains("stack_obj := __o_1;"), "{out}");
        // An independent second store does kill the first.
        let body = "    let __o_1 = my.id;\n    stack_obj := __o_1;\n    stack_obj := 7;\n    my.attr[0] := 1;\n    return true;\n";
        assert!(
            !optimize_tree_body(body, true, &BTreeMap::new(), true, false, false).contains("__o_1")
        );
    }

    #[test]
    fn compound_stack_obj_writes_forget_its_value() {
        let body = "    let __o_1 = my.id;\n    stack_obj := __o_1;\n    stack_obj += 1;\n    stack_obj := __o_1;\n    my.attr[0] := stack.attr[0];\n    return true;\n";
        let out = optimize_tree_body(body, true, &BTreeMap::new(), true, false, false);
        assert_eq!(out.matches("stack_obj := __o_1;").count(), 2, "{out}");
    }

    #[test]
    fn faulting_reads_stay() {
        // An object cell, temp, param, or global read can fault, so its unread store stays.
        for operand in [
            "my.attr[99]",
            "temp[0]",
            "param[3]",
            "global[2]",
            "stack.attr[0]",
            "local[0]",
        ] {
            let body = format!("    let __a_1 = {operand};\n    return true;\n");
            assert_eq!(
                optimize_tree_body(&body, true, &BTreeMap::new(), true, false, false),
                body,
                "{operand}"
            );
        }
        let body = "    let __a_1 = my.id;\n    let __b_2 = 5;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            "    return true;\n"
        );
    }

    #[test]
    fn local_slot_numbers_leave_the_tree_alone() {
        let dead = "    let __a_1 = 0;\n    let __b_2 = 1;\n    my.attr[0] := __b_2;\n";
        for line in [
            "op 0x0002 (0x05, 0x00, 0x00, 0x00, 0x19, 0x07, 0x00, 0x00);",
            "my.attr[1] := scope(25, 1);",
            "my.attr[1] := scope(0x19, 0);",
            "my.attr[1] := scope(__k_3, 0);",
            "/* note */ my.attr[1] := 0;",
        ] {
            let body = format!("{dead}    {line}\n    return true;\n");
            assert_eq!(
                optimize_tree_body(&body, true, &BTreeMap::new(), true, false, false),
                body,
                "{line}"
            );
        }
        // A named `op:` argument and the operand-free builtin ops are not raw local references.
        for line in [
            "manage_inventory(op: 1, kind: 2, flags: 3, guid: 0x7f400011);",
            "op 0x0004 (0, 0, 0, 0, 0, 0, 0, 0);",
            "my.attr[1] := scope(12, 2);",
        ] {
            let body = format!("{dead}    {line}\n    return true;\n");
            assert_ne!(
                optimize_tree_body(&body, true, &BTreeMap::new(), true, false, false),
                body,
                "{line}"
            );
        }
    }

    #[test]
    fn review_regressions_keep_observers() {
        // An unread save goes even before a primitive that reads the last result: that
        // primitive then sees the frame's entry state, as in the source.
        let body =
            "    let __self_obj = stack_obj;\n    idle_for_input(param[0], 1);\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            "    idle_for_input(param[0], 1);\n    return true;\n"
        );
        // Not when a local read before it is written would move to another frame word.
        let body = "    let __self_obj = stack_obj;\n    let __v_x_0;\n    temp[0] := __v_x_0;\n    call 300(temps);\n    return true;\n";
        assert_eq!(
            drop_unread_entry_save(body, &|_, _| true, DrawRanges::NotGlobal),
            None
        );
        // A raw line the structured pass cannot parse falls back to the declarations: written
        // where they come into scope, the save goes; declared without a value, it stays.
        let raw_line = "    if !call 281(0, 0) {\n    if push_interaction(menu: 2, object: 0, priority: 0, flags: 7) { return true; }\n    }\n";
        let body = format!(
            "    let __self_obj = stack_obj;\n    let __v_x_0 = 1;\n    local[0] := stack_obj;\n    stack_obj := my.id;\n{raw_line}    stack_obj := local[0];\n    temp[0] := __v_x_0;\n    return true;\n"
        );
        assert!(parse_body(&body).is_none());
        assert_eq!(
            drop_unread_entry_save(&body, &|_, _| true, DrawRanges::NotGlobal).as_deref(),
            Some(&body["    let __self_obj = stack_obj;\n".len()..])
        );
        // The pipeline's text check vetoes the drop when a word the text reads changes hands.
        assert_eq!(
            drop_unread_entry_save(&body, &|_, _| false, DrawRanges::NotGlobal),
            None
        );
        let body = body.replace("let __v_x_0 = 1;", "let __v_x_0;");
        assert_eq!(
            drop_unread_entry_save(&body, &|_, _| true, DrawRanges::NotGlobal),
            None
        );
        // Scope 27 (sprite flags) reads the stack object, so the switch before it stays.
        let body = "    stack_obj := my.id;\n    temp[0] := 0;\n    scope(27, 0) := 1;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            body
        );
        // A local-slot operand keeps naming the save.
        let body = "    let __self_obj = stack_obj;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 1, icon: __self_obj);\n    stack_obj := my.id;\n    return true;\n";
        assert!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false)
                .contains("icon: __self_obj")
        );
        // Raw numeric targets keep the line-based save drop away.
        let body = "    let __self_obj = stack_obj;\n    stack_obj := my.id;\n    random(temp[0], 1) -> true: 3, false: 3;\n    return true;\n";
        assert_eq!(
            drop_unread_entry_save(body, &|_, _| true, DrawRanges::NotGlobal),
            None
        );
        // A local read before any write is not moved into the pin slot.
        let body = "    let o;\n    local[0] := o;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 3, icon: 0);\n    return true;\n";
        assert_eq!(pin_in_place(body, DrawRanges::NotGlobal), body);
        // Nor when another local read before any write would move to a different word.
        let body = "    let __v_o_0 = stack_obj;\n    let __v_inherited_1;\n    temp[0] := __v_inherited_1;\n    local[0] := __v_o_0;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 3, icon: 0);\n    return true;\n";
        assert_eq!(pin_in_place(body, DrawRanges::NotGlobal), body);
        // With every local written first, the pin applies.
        let body = "    let __v_o_0 = stack_obj;\n    local[0] := __v_o_0;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 3, icon: 0);\n    return true;\n";
        assert_eq!(
            pin_in_place(body, DrawRanges::NotGlobal),
            "    local[0] := stack_obj;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 3, icon: 0);\n    return true;\n"
        );
    }

    #[test]
    fn entry_save_forwards_until_a_write_and_then_goes() {
        // Nothing changes the stack object before the restore, which becomes a no-op.
        let body = "    let __self_obj = stack_obj;\n    my.attr[0] := stack.attr[1];\n    stack_obj := __self_obj;\n    my.attr[2] := 1;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            "    my.attr[0] := stack.attr[1];\n    my.attr[2] := 1;\n    return true;\n"
        );
        // After a write the restore is real, and so is the save.
        let body = "    let __self_obj = stack_obj;\n    stack_obj := my.id;\n    my.attr[0] := stack.attr[1];\n    stack_obj := __self_obj;\n    my.attr[1] := stack.attr[2];\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            body
        );
        // A primitive may write it too.
        let body = "    let __self_obj = stack_obj;\n    set_to_next(all);\n    stack_obj := __self_obj;\n    my.attr[1] := stack.attr[2];\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            body
        );
    }

    #[test]
    fn unread_stack_obj_stores_drop() {
        // Nothing reads the store before the frame returns.
        let body = "    let __o_1 = my.id;\n    stack_obj := __o_1;\n    my.attr[0] := 1;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body, true, &BTreeMap::new(), true, false, false),
            "    my.attr[0] := 1;\n    return true;\n"
        );
        // A read on one path keeps it; so does a loop.
        for tail in [
            "    if my.attr[0] == 1 {\n        my.attr[1] := stack.attr[0];\n    }\n    return true;\n",
            "    while my.attr[0] < 3 {\n        my.attr[0] += 1;\n    }\n    return true;\n",
        ] {
            let body = format!("    let __o_1 = my.id;\n    stack_obj := __o_1;\n{tail}");
            let out = optimize_tree_body(&body, true, &BTreeMap::new(), true, false, false);
            assert!(out.starts_with("    stack_obj := my.id;\n"), "{out}");
        }
    }

    #[test]
    fn strings_fields_and_wide_characters() {
        // Multi-byte string contents keep byte offsets aligned, and a `.field` named like a
        // local is not renamed.
        let body = "    let __a_1 = 1;\n    my.attr[0] := __a_1;\n    let __b_2 = 2;\n    dialog(\"caf\u{e9} \u{2603}\", bcon.__b_2);\n    my.attr[1] := __b_2;\n    return true;\n";
        let out = optimize_tree_body(body, true, &BTreeMap::new(), true, false, false);
        assert!(
            out.contains("dialog(\"caf\u{e9} \u{2603}\", bcon.__b_2);"),
            "{out}"
        );
        assert!(out.contains("my.attr[1] := __a_1;"), "{out}");
    }
}
