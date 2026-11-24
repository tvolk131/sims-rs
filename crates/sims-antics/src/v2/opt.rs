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
//! The pass works on the structured text. It leaves a tree untouched when it meets anything it
//! does not model exactly: labels, explicit branch targets (`->`), comments, or more than one
//! statement on a line (some `raw { }` blocks).

use std::collections::{BTreeMap, BTreeSet};

/// Optimizes one lowered tree body (the lines between the tree's braces, indented by 4 per
/// level from 4). Returns the input unchanged when the body is outside the modelled subset or
/// nothing applies.
pub(crate) fn optimize_tree_body(body: &str) -> String {
    let Some(mut nodes) = parse_body(body) else {
        return body.to_owned();
    };
    let before = nodes.clone();
    for _ in 0..8 {
        let snapshot = nodes.clone();
        drop_unreachable(&mut nodes);
        forward_entry_save(&mut nodes);
        drop_dead_stack_obj_stores(&mut nodes);
        drop_stack_obj_stores_before_exit(&mut nodes, false);
        unwrap_single_pass_loops(&mut nodes);
        drop_dead_local_stores(&mut nodes);
        forward_copies(&mut nodes);
        merge_duplicate_tails(&mut nodes);
        coalesce_locals(&mut nodes);
        drop_unused_decls(&mut nodes);
        drop_unread_entry_save_node(&mut nodes);
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

/// Drops the Sim-tree entry save `let __self_obj = stack_obj;` when nothing else names
/// `__self_obj`. This works line by line, so it also applies to trees the full pass leaves alone
/// (`raw` blocks): a raw block that reads the save names it.
///
/// The save is the lowering's own bookkeeping, not part of the program: an unread save is one the
/// lowering never needed, so it goes even before a node that observes the last result (which
/// then sees the frame's entry state, as the source says).
pub(crate) fn drop_unread_entry_save(body: &str, formats_text: bool) -> Option<String> {
    const SAVE: &str = "let __self_obj = stack_obj;";
    // Deleting the node would shift numeric branch targets (`-> true: 3`), and deleting the
    // local would move the frame words of the named locals after it, which a raw op or a
    // `scope(25, N)` cell may name by number. (`.air` places named locals after every explicit
    // `local[N]`, so those keep their words.)
    // A tree that formats text (`formats_text`, which the lowering reads from the parse of its
    // raw blocks) reads frame words by number (`$Local:N`), so it sees the shift.
    if formats_text
        || body.contains("/*")
        || has_positional_targets(body)
        || body.lines().any(|l| names_local_slot(&strip_strings(l)))
    {
        return None;
    }
    let mut lines = body
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let (save_at, first) = lines.next()?;
    if first.trim() != SAVE {
        return None;
    }
    let mentions: usize = body
        .lines()
        .map(|l| idents(l).iter().filter(|id| **id == "__self_obj").count())
        .sum();
    if mentions != 1 {
        return None;
    }
    // Every named local after the save moves down one frame word, which keeps whatever an
    // earlier frame left there: none may be read before it is written. The flow analysis runs on
    // the body with one statement per line, so raw blocks written on one line parse too. A body
    // it still cannot parse (a raw node graph with labels, whose jumps it does not model) keeps
    // the save unless it has no other named local to move, as does a raw `local x;`
    // declaration.
    let split = one_statement_per_line(body);
    let statements: Vec<&str> = split.lines().map(str::trim).collect();
    if statements.iter().any(|stmt| {
        stmt.strip_prefix("local")
            .is_some_and(|r| r.starts_with(char::is_whitespace))
    }) {
        return None;
    }
    let moves_a_local = statements
        .iter()
        .filter(|stmt| {
            stmt.strip_prefix("let")
                .is_some_and(|r| r.starts_with(char::is_whitespace))
        })
        .count()
        > 1;
    let defined = match parse_body(&split) {
        Some(nodes) => locals_defined_before_reads(&nodes),
        None => !moves_a_local,
    };
    if !defined {
        return None;
    }
    let mut out = String::with_capacity(body.len());
    for (i, l) in body.lines().enumerate() {
        if i != save_at {
            out.push_str(l);
            out.push('\n');
        }
    }
    Some(out)
}

/// Whether a node graph names a branch target by position (`-> true: 3`, `raw(1)`), which
/// deleting a node would shift. Labels and `true` / `false` / `error` do not move.
///
/// A target list runs from `->` to its `;`, which may be on a later line.
fn has_positional_targets(body: &str) -> bool {
    let code: String = body
        .lines()
        .map(|l| {
            let stripped = strip_strings(l);
            match stripped.find("//") {
                Some(at) => stripped[..at].to_owned(),
                None => stripped,
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    code.split([';', '{', '}']).any(|stmt| {
        let Some((_, targets)) = stmt.split_once("->") else {
            return false;
        };
        targets.split(',').any(|t| {
            let target = t.split_once(':').map_or(t, |(_, v)| v).trim();
            target.starts_with("raw") || target.starts_with(|c: char| c.is_ascii_digit())
        })
    })
}

/// The body with one statement per line, for analysis only: `a; b;` and `if c { a; }` written
/// on one line (as raw blocks often are) are split, `}` and a following `else` rejoin, and `//`
/// comments go.
fn one_statement_per_line(body: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for line in body.lines() {
        let mut cur = String::new();
        let (mut in_str, mut escaped) = (false, false);
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if in_str {
                cur.push(c);
                match c {
                    _ if escaped => escaped = false,
                    '\\' => escaped = true,
                    '"' => in_str = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => {
                    in_str = true;
                    cur.push(c);
                }
                '/' if chars.peek() == Some(&'/') => break,
                ';' => {
                    cur.push(';');
                    parts.push(std::mem::take(&mut cur));
                }
                '{' => {
                    let head = cur.trim().to_owned();
                    parts.push(if head.is_empty() {
                        "{".to_owned()
                    } else {
                        format!("{head} {{")
                    });
                    cur.clear();
                }
                '}' => {
                    parts.push(std::mem::take(&mut cur));
                    parts.push("}".to_owned());
                }
                _ => cur.push(c),
            }
        }
        parts.push(cur);
    }
    let mut out: Vec<String> = Vec::new();
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if p.starts_with("else") && out.last().is_some_and(|l| l == "}") {
            let last = out.last_mut().expect("checked");
            last.push(' ');
            last.push_str(p);
        } else {
            out.push(p.to_owned());
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Whether `text` (an `.air` statement or `if` condition) starts with a `push_interaction`
/// primitive, which does not read the last result.
fn is_push_interaction(text: &str) -> bool {
    let t = text.trim_start();
    let t = t.strip_prefix('!').unwrap_or(t).trim_start();
    t.starts_with("push_interaction(")
}

/// In a tree whose pinned pushes all copy the same plain local `s` into `local[0]`
/// (`local[0] := s;` right before each `push_interaction(.., object: 0, ..)`), makes `s` live in
/// `local[0]` itself and drops the copies. `.air` places named locals after `local[0]`, a pinned
/// tree has no `raw`, and only the push reads the slot, so the slot and `s` always agree.
pub(crate) fn pin_in_place(body: &str) -> String {
    const PIN: &str = "local[0]";
    let Some(mut nodes) = parse_body(body) else {
        return body.to_owned();
    };
    let mut copied: BTreeSet<String> = BTreeSet::new();
    let mut ok = true;
    check_pins(&nodes, &mut copied, &mut ok);
    let mut lets = Vec::new();
    collect_lets(&nodes, &mut lets);
    let source = match copied.iter().next() {
        Some(s) if ok && copied.len() == 1 => s.clone(),
        _ => return body.to_owned(),
    };
    if !lets.contains(&source)
        || merge_group(&source).is_none()
        || matches!(source.as_str(), "__self_obj" | "__saved_stack_obj")
        // Every named local moves down one word once `s` leaves the named list, so none may be
        // read before it is written (frame words keep what an earlier frame left there).
        || !lets
            .iter()
            .all(|name| defined_before_reads(&nodes, name, &mut false))
    {
        return body.to_owned();
    }
    drop_pins(&mut nodes);
    let rename = BTreeMap::from([(source, PIN.to_owned())]);
    rewrite(&mut nodes, &rename);
    let mut out = String::with_capacity(body.len());
    print_nodes(&nodes, 1, &mut out);
    out
}

/// Places a tree's `@slot(N) let` locals (`pinned`, which lowering names and numbers) in
/// `local[N]` and every other generated local (`__..`) in the lowest frame word nothing else
/// names, in declaration order. `.air` alone would put the named locals after the highest
/// explicit slot, so pinning a high word would grow the frame by that many words.
///
/// The explicit slots come from the `.air` compiler's own scan of the tree, so no word an
/// operand, a numeric local argument or another pinned local names is shared. Locals a `raw`
/// block declares under its own name keep `.air`'s placement (after every explicit slot), as do
/// all unpinned locals of a tree with raw `op` bytes, whose operands no scan can read. A body
/// `.air` cannot parse is left alone: compiling it reports the error.
pub(crate) fn assign_frame_slots(
    body: &str,
    pinned: &BTreeMap<String, usize>,
    raw_declared: &BTreeSet<String>,
) -> String {
    let wrapped = format!("tree 4096 \"__slots\" {{\n{body}\n}}\n");
    let Some(usage) =
        crate::parser::parse_module(&wrapped)
            .ok()
            .and_then(|m| match m.items.first() {
                Some(crate::ast::Item::Tree(t)) => Some(crate::lower::air_local_usage(&t.body)),
                _ => None,
            })
    else {
        return body.to_owned();
    };
    let mut used = usage.explicit.clone();
    let mut slots: BTreeMap<String, usize> = BTreeMap::new();
    for name in &usage.lets {
        // A name a raw block declares itself is that block's local, whatever lowering named.
        if let Some(&n) = pinned.get(name)
            && !raw_declared.contains(name)
        {
            used.insert(n);
            slots.insert(name.clone(), n);
        }
    }
    if !usage.has_raw_ops {
        for name in &usage.lets {
            if name.starts_with("__") && !slots.contains_key(name) && !raw_declared.contains(name) {
                let k = (0..).find(|k| !used.contains(k)).expect("a free slot");
                used.insert(k);
                slots.insert(name.clone(), k);
            }
        }
    }
    place_locals(body, &slots).unwrap_or_else(|| body.to_owned())
}

/// `body` with each local of `slots` written as `local[K]`, found by the `.air` lexer: `let x;`
/// (or `local x;`) goes, `let x = v;` becomes `local[K] := v;`, and every other operand `x`
/// becomes `local[K]`. A name after `.` (a field), before `:` (a label or an argument key), after
/// `const`, or among a node's branch targets (`-> .. ;`) is not an operand and stays. `None` if
/// the body does not lex.
fn place_locals(body: &str, slots: &BTreeMap<String, usize>) -> Option<String> {
    use crate::lexer::TokenKind as T;
    let tokens = crate::lexer::tokenize(body).ok()?;
    let mut line_starts = vec![0];
    line_starts.extend(body.match_indices('\n').map(|(i, _)| i + 1));
    // The lexer counts columns in bytes.
    let offset = |t: &crate::lexer::Token| line_starts[t.span.line - 1] + t.span.column - 1;
    let ident_end = |at: usize| {
        at + body[at..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(body.len() - at)
    };
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut in_targets = false;
    for (i, t) in tokens.iter().enumerate() {
        match &t.kind {
            T::Arrow => in_targets = true,
            T::Semicolon => in_targets = false,
            T::Ident(name) if !in_targets => {
                let Some(k) = slots.get(name) else {
                    continue;
                };
                let prev = i.checked_sub(1).map(|p| &tokens[p].kind);
                let next = tokens.get(i + 1).map(|t| &t.kind);
                // A field, a label or argument key, or a called name.
                if matches!(prev, Some(T::Dot))
                    || matches!(next, Some(T::Colon | T::LParen))
                    || matches!(prev, Some(T::Ident(kw)) if kw == "call" || kw == "subroutine")
                {
                    continue;
                }
                match prev {
                    Some(T::Ident(kw)) if kw == "const" => {}
                    Some(T::Ident(kw)) if kw == "let" || kw == "local" => {
                        let decl = offset(&tokens[i - 1]);
                        match next {
                            Some(T::Semicolon) => {
                                let end = offset(&tokens[i + 1]) + 1;
                                edits.push((decl, end, String::new()));
                            }
                            Some(T::Assign) => {
                                let op = offset(&tokens[i + 1]);
                                let end = op + if body[op..].starts_with(":=") { 2 } else { 1 };
                                edits.push((decl, end, format!("local[{k}] :=")));
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        let at = offset(t);
                        edits.push((at, ident_end(at), format!("local[{k}]")));
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = body.to_owned();
    for (start, end, mut text) in edits.into_iter().rev() {
        // Line breaks inside a removed declaration stay, so lines still pair up below.
        text.extend(body[start..end].matches('\n'));
        out.replace_range(start..end, &text);
    }
    // A declaration that was alone on its line leaves no blank line behind.
    let mut placed = String::with_capacity(out.len());
    for (line, orig) in out.lines().zip(body.lines()) {
        if line.trim().is_empty() && !orig.trim().is_empty() {
            continue;
        }
        placed.push_str(line);
        placed.push('\n');
    }
    Some(placed)
}

/// Collects the sources of `local[0] := s;` copies; clears `ok` if `local[0]` is named in any
/// other way, or if a copy is not directly followed by a `push_interaction`.
fn check_pins(nodes: &[Node], copied: &mut BTreeSet<String>, ok: &mut bool) {
    for (i, n) in nodes.iter().enumerate() {
        match n {
            Node::Line(l) => {
                let stripped = strip_strings(l);
                if !stripped.contains("local[") {
                    continue;
                }
                let followed_by_push = match nodes[i + 1..].iter().find(|n| !is_decl(n)) {
                    Some(Node::Line(next)) => is_push_interaction(next),
                    Some(Node::If { arms, .. }) => is_push_interaction(&arms[0].0),
                    _ => false,
                };
                match classify(l) {
                    Kind::Assign {
                        lhs: "local[0]",
                        op: ":=",
                        rhs,
                    } if is_ident(rhs) && followed_by_push => {
                        copied.insert(rhs.to_owned());
                    }
                    _ => *ok = false,
                }
            }
            Node::If { arms, else_body } => {
                for (cond, body) in arms {
                    if strip_strings(cond).contains("local[") {
                        *ok = false;
                    }
                    check_pins(body, copied, ok);
                }
                if let Some(body) = else_body {
                    check_pins(body, copied, ok);
                }
            }
            Node::While { cond, body } => {
                if cond
                    .as_deref()
                    .is_some_and(|c| strip_strings(c).contains("local["))
                {
                    *ok = false;
                }
                check_pins(body, copied, ok);
            }
        }
    }
}

/// Whether every read of `name` in `nodes` comes after a write of it on every path (`defined`
/// says it is written on entry). A local read before any write reads whatever its frame word
/// held, so moving it to another word would change the value read. Writes are plain stores and
/// primitives' out-operands ([`out_operand_write`]) on the exits that write them.
fn defined_before_reads(nodes: &[Node], name: &str, defined: &mut bool) -> bool {
    // A `break` outside any loop has no loop exit to reach; the parser never produces one.
    let mut breaks = None;
    flow_defined(nodes, name, defined, &mut breaks)
}

/// The `&&` of two path states where either may be absent (no path reaches the join that way).
fn meet(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a && b),
        (a, None) => a,
        (None, b) => b,
    }
}

/// [`defined_before_reads`] inside the innermost loop: `breaks` collects the state on every
/// `break` (the loop exit's state, with the condition's false exit), `None` until one is seen.
///
/// After `return`, `break` or `continue` nothing in the block runs, so the state that falls
/// through is vacuous (`true`, the identity of the join). `continue` returns to the loop head,
/// whose state is the one on entry: a back edge only carries more writes.
fn flow_defined(nodes: &[Node], name: &str, defined: &mut bool, breaks: &mut Option<bool>) -> bool {
    let reads = |text: &str| idents(text).contains(&name);
    for n in nodes {
        match n {
            Node::Line(l) => match classify(l) {
                Kind::Let { name: n, init } if n == name => {
                    if let Some(init) = init {
                        if reads(init) {
                            return false;
                        }
                        *defined = true;
                    }
                }
                Kind::Assign { lhs, op: ":=", rhs } if lhs == name => {
                    if reads(rhs) {
                        return false;
                    }
                    *defined = true;
                }
                kind @ (Kind::Return | Kind::Break | Kind::Continue) => {
                    if !*defined && reads(l) {
                        return false;
                    }
                    if matches!(kind, Kind::Break) {
                        *breaks = meet(*breaks, Some(*defined));
                    }
                    *defined = true;
                }
                _ => match out_operand_write(l, name) {
                    // Both exits of a bare statement reach the next line, so it defines the
                    // cell only when every exit writes it.
                    Some((rest, exit)) => {
                        if !*defined && reads(&rest) {
                            return false;
                        }
                        *defined |= exit == WriteExit::Both;
                    }
                    None => {
                        if !*defined && reads(l) {
                            return false;
                        }
                    }
                },
            },
            Node::If { arms, else_body } => {
                if !if_chain_defined(arms, else_body.as_deref(), name, defined, breaks) {
                    return false;
                }
            }
            Node::While { cond, body } => {
                let (mut d, exit) = match cond {
                    Some(c) => match cond_defines(c, name, *defined) {
                        Some((on_true, on_false)) => (on_true, Some(on_false)),
                        None => return false,
                    },
                    None => (*defined, None),
                };
                let mut inner = None;
                if !flow_defined(body, name, &mut d, &mut inner) {
                    return false;
                }
                // A loop nothing leaves never falls through.
                *defined = meet(exit, inner).unwrap_or(true);
            }
        }
    }
    true
}

/// `if`/`else if`/`else` from arm `0`: each condition runs only when the ones before it were
/// false, so a write on a condition's false exit reaches the arms after it.
fn if_chain_defined(
    arms: &[(String, Vec<Node>)],
    else_body: Option<&[Node]>,
    name: &str,
    defined: &mut bool,
    breaks: &mut Option<bool>,
) -> bool {
    let Some(((cond, body), rest)) = arms.split_first() else {
        return match else_body {
            // No `else`: control falls through with what the conditions defined.
            None => true,
            Some(body) => flow_defined(body, name, defined, breaks),
        };
    };
    let Some((on_true, on_false)) = cond_defines(cond, name, *defined) else {
        return false;
    };
    let mut d_then = on_true;
    if !flow_defined(body, name, &mut d_then, breaks) {
        return false;
    }
    let mut d_else = on_false;
    if !if_chain_defined(rest, else_body, name, &mut d_else, breaks) {
        return false;
    }
    *defined = d_then && d_else;
    true
}

/// What a branch condition leaves defined on its true and false exits, or `None` when it reads
/// `name` before anything writes it.
fn cond_defines(cond: &str, name: &str, defined: bool) -> Option<(bool, bool)> {
    let reads = |text: &str| idents(text).contains(&name);
    let (negated, inner) = match cond.trim_start().strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, cond),
    };
    let Some((rest, exit)) = out_operand_write(inner, name) else {
        return (defined || !reads(cond)).then_some((defined, defined));
    };
    if !defined && reads(&rest) {
        return None;
    }
    let (prim_true, prim_false) = (
        defined || exit != WriteExit::False,
        defined || exit != WriteExit::True,
    );
    Some(if negated {
        (prim_false, prim_true)
    } else {
        (prim_true, prim_false)
    })
}

/// Which exits of a primitive write its out-operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteExit {
    Both,
    True,
    False,
}

/// When `text` (a statement or condition, no leading `!`) is a primitive that stores into the
/// cell `name` through an out-operand, returns the text with that operand blanked (for the reads
/// it still makes) and the exits on which the store happens, as the VM runs them:
///
/// - `random(x, n)` with a literal range stores the draw, then takes the true exit (a range
///   that resolves to a word pointer, such as global 34, redirects the draw into the range
///   cell, so any other range is not a write of `x`);
/// - `animate(.., local: x)` stores an event in the local, then takes the false exit;
/// - `relationship(.., value: x)` without flag 4 reads the number into the cell and takes the
///   true exit; with flag 1 a missing number takes the false exit unwritten, and without it
///   the row is added, so the false exit never runs.
fn out_operand_write(text: &str, name: &str) -> Option<(String, WriteExit)> {
    let t = text.trim();
    let t = t.strip_suffix(';').unwrap_or(t).trim_end();
    let open = t.find('(')?;
    let prim = &t[..open];
    if !t.ends_with(')') || !is_ident(prim) {
        return None;
    }
    let inner = &t[open + 1..t.len() - 1];
    let args = split_top_level(inner);
    let named = |key: &str| {
        args.iter().position(|a| {
            a.split_once(':')
                .is_some_and(|(k, _)| k.trim() == key && !k.contains('('))
        })
    };
    let value_of = |i: usize| args[i].split_once(':').map_or("", |(_, v)| v.trim());
    let (at, exit) = match prim {
        "random" => {
            let first = args.first()?;
            let literal_range = args.get(1).and_then(|r| parse_int(r.trim())).is_some();
            (first.trim() == name && literal_range).then_some((0, WriteExit::Both))?
        }
        "animate" => {
            let i = named("local")?;
            (value_of(i) == name).then_some((i, WriteExit::False))?
        }
        "relationship" => {
            let i = named("value")?;
            let flags = match named("flags") {
                Some(f) => parse_int(value_of(f))?,
                None => 0,
            };
            // Without flag 1 a missing row is added, so the read always writes the cell and
            // takes the true exit: the false exit never runs.
            let exit = if flags & 1 == 0 {
                WriteExit::Both
            } else {
                WriteExit::True
            };
            (value_of(i) == name && flags & 4 == 0).then_some((i, exit))?
        }
        _ => return None,
    };
    let mut rest = format!("{prim}(");
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            rest.push_str(", ");
        }
        if i != at {
            rest.push_str(a.trim());
        }
    }
    rest.push(')');
    Some((rest, exit))
}

/// `a, b(c, d), e` → `["a", " b(c, d)", " e"]`: commas outside brackets and strings.
fn split_top_level(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut in_str, mut escaped) = (0i32, 0usize, false, false);
    for (i, c) in text.char_indices() {
        if in_str {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

/// A decimal or `0x` integer.
fn parse_int(text: &str) -> Option<i64> {
    let t = text.trim();
    match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => t.parse().ok(),
    }
}

fn drop_pins(nodes: &mut Vec<Node>) {
    nodes.retain(|n| {
        !matches!(n, Node::Line(l) if matches!(classify(l), Kind::Assign { lhs: "local[0]", op: ":=", .. }))
    });
    for n in nodes.iter_mut() {
        match n {
            Node::Line(_) => {}
            Node::If { arms, else_body } => {
                for (_, body) in arms.iter_mut() {
                    drop_pins(body);
                }
                if let Some(body) = else_body {
                    drop_pins(body);
                }
            }
            Node::While { body, .. } => drop_pins(body),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Line(String),
    If {
        /// `(condition, body)` for the `if` and each `else if`.
        arms: Vec<(String, Vec<Node>)>,
        else_body: Option<Vec<Node>>,
    },
    /// `while cond { }`, or `while true { }` (no condition node) when `cond` is `None`.
    While {
        cond: Option<String>,
        body: Vec<Node>,
    },
}

// ---------------------------------------------------------------------------
// Parsing and printing
// ---------------------------------------------------------------------------

/// The line with string literal contents blanked, for structural checks and token scans.
fn strip_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_str = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                out.push('"');
                continue;
            }
            // Same byte width, so offsets into the stripped text index the original.
            for _ in 0..c.len_utf8() {
                out.push(' ');
            }
        } else {
            if c == '"' {
                in_str = true;
            }
            out.push(c);
        }
    }
    out
}

fn parse_body(body: &str) -> Option<Vec<Node>> {
    let lines: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut pos = 0;
    let (nodes, end) = parse_block(&lines, &mut pos)?;
    if end.is_some() || pos != lines.len() {
        return None;
    }
    Some(nodes)
}

/// How a nested block ended.
enum BlockEnd {
    Close,
    ElseIf(String),
    Else,
}

/// Parses statements until a closing line; returns them with the closing kind (`None` at EOF).
fn parse_block(lines: &[&str], pos: &mut usize) -> Option<(Vec<Node>, Option<BlockEnd>)> {
    let mut nodes = Vec::new();
    while *pos < lines.len() {
        let line = lines[*pos];
        let s = strip_strings(line);
        *pos += 1;
        if s.contains("//")
            || s.contains("/*")
            || s.contains("*/")
            || s.contains("->")
            || s.contains('#')
            || names_local_slot(&s)
        {
            return None;
        }
        if s == "}" {
            return Some((nodes, Some(BlockEnd::Close)));
        }
        if s == "} else {" {
            return Some((nodes, Some(BlockEnd::Else)));
        }
        if let Some(cond) = header(&s, line, "} else if ") {
            return Some((nodes, Some(BlockEnd::ElseIf(cond))));
        }
        if let Some(cond) = header(&s, line, "if ") {
            let mut arms = Vec::new();
            let mut else_body = None;
            let mut cond = cond;
            loop {
                let (body, end) = parse_block(lines, pos)?;
                arms.push((cond, body));
                match end? {
                    BlockEnd::Close => break,
                    BlockEnd::ElseIf(c) => cond = c,
                    BlockEnd::Else => {
                        let (body, end) = parse_block(lines, pos)?;
                        if !matches!(end?, BlockEnd::Close) {
                            return None;
                        }
                        else_body = Some(body);
                        break;
                    }
                }
            }
            nodes.push(Node::If { arms, else_body });
            continue;
        }
        if let Some(cond) = header(&s, line, "while ") {
            let (body, end) = parse_block(lines, pos)?;
            if !matches!(end?, BlockEnd::Close) {
                return None;
            }
            // `while true` is `.air`'s condition-free loop.
            let cond = (cond != "true").then_some(cond);
            nodes.push(Node::While { cond, body });
            continue;
        }
        // One simple statement: exactly one `;`, at the end, and no braces.
        if !s.ends_with(';') || s.matches(';').count() != 1 || s.contains('{') || s.contains('}') {
            return None;
        }
        nodes.push(Node::Line(line.to_owned()));
    }
    Some((nodes, None))
}

/// `kw COND {` → `COND` (taken from the original line), checking the stripped form.
fn header(stripped: &str, line: &str, kw: &str) -> Option<String> {
    let rest = stripped.strip_prefix(kw)?.strip_suffix(" {")?;
    if rest.contains(';') || rest.contains('{') || rest.contains('}') {
        return None;
    }
    Some(line[kw.len()..line.len() - 2].to_owned())
}

fn print_nodes(nodes: &[Node], depth: usize, out: &mut String) {
    let pad = |out: &mut String, d: usize| {
        for _ in 0..d {
            out.push_str("    ");
        }
    };
    for n in nodes {
        match n {
            Node::Line(l) => {
                pad(out, depth);
                out.push_str(l);
                out.push('\n');
            }
            Node::If { arms, else_body } => {
                for (i, (cond, body)) in arms.iter().enumerate() {
                    pad(out, depth);
                    if i == 0 {
                        out.push_str(&format!("if {cond} {{\n"));
                    } else {
                        out.push_str(&format!("}} else if {cond} {{\n"));
                    }
                    print_nodes(body, depth + 1, out);
                }
                if let Some(body) = else_body {
                    pad(out, depth);
                    out.push_str("} else {\n");
                    print_nodes(body, depth + 1, out);
                }
                pad(out, depth);
                out.push_str("}\n");
            }
            Node::While { cond, body } => {
                pad(out, depth);
                match cond {
                    Some(c) => out.push_str(&format!("while {c} {{\n")),
                    None => out.push_str("while true {\n"),
                }
                print_nodes(body, depth + 1, out);
                pad(out, depth);
                out.push_str("}\n");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Line classification
// ---------------------------------------------------------------------------

const ASSIGN_OPS: &[&str] = &[
    ":=",
    "+=",
    "-=",
    "*=",
    "/=",
    "%=",
    "&=",
    "|=",
    "^=",
    "flag_set",
    "flag_clear",
];

#[derive(Debug, PartialEq, Eq)]
enum Kind<'a> {
    Let {
        name: &'a str,
        init: Option<&'a str>,
    },
    Assign {
        lhs: &'a str,
        op: &'a str,
        rhs: &'a str,
    },
    Return,
    Break,
    Continue,
    Other,
}

fn classify(line: &str) -> Kind<'_> {
    let body = line.strip_suffix(';').unwrap_or(line).trim();
    if body == "break" {
        return Kind::Break;
    }
    if body == "continue" {
        return Kind::Continue;
    }
    if body == "return" || body.starts_with("return ") {
        return Kind::Return;
    }
    if body.contains('"') {
        return Kind::Other;
    }
    if let Some(rest) = body.strip_prefix("let ") {
        return match rest.split_once(" = ") {
            Some((name, init)) if is_ident(name.trim()) => Kind::Let {
                name: name.trim(),
                init: Some(init.trim()),
            },
            None if is_ident(rest.trim()) => Kind::Let {
                name: rest.trim(),
                init: None,
            },
            _ => Kind::Other,
        };
    }
    let mut parts = body.splitn(3, ' ');
    if let (Some(lhs), Some(op), Some(rhs)) = (parts.next(), parts.next(), parts.next())
        && ASSIGN_OPS.contains(&op)
        && !lhs.contains('(')
    {
        return Kind::Assign {
            lhs,
            op,
            rhs: rhs.trim(),
        };
    }
    Kind::Other
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Identifier tokens outside string literals.
fn idents(text: &str) -> Vec<&str> {
    let stripped = strip_strings(text);
    let mut out = Vec::new();
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            // A field after `.` is not a variable. Same byte offsets in the original: stripping
            // only blanks string contents.
            if !follows_dot(bytes, start) {
                out.push(&text[start..i]);
            }
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Whether the token at `start` is a field (`.name`, spaces allowed before the name).
fn follows_dot(bytes: &[u8], start: usize) -> bool {
    bytes[..start]
        .iter()
        .rev()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|&b| b == b'.')
}

/// Opcodes whose VM handlers read no operand (find best interaction, grab, drop, gosub found
/// action), as v2 emits them.
const OPERAND_FREE_OPS: &[&str] = &["0x0003", "0x0004", "0x0005", "0x001e"];

/// Whether a (string-stripped) line can name a local slot by number, which renaming and
/// coalescing named locals would silently retarget: a raw `op` (its operands may encode a
/// local), or `scope(N, …)` whose scope is the locals scope (25) or not a plain literal.
fn names_local_slot(stripped: &str) -> bool {
    // The operand-free primitives v2 builtins spell as raw ops name no local.
    let mut cleaned = stripped.to_owned();
    for opcode in OPERAND_FREE_OPS {
        cleaned = cleaned.replace(&format!("op {opcode} (0, 0, 0, 0, 0, 0, 0, 0)"), "");
    }
    let stripped = cleaned.as_str();
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &stripped[start..i];
            // A raw op is `op <opcode> (...)`; `op:` is a primitive's named argument.
            if word == "op"
                && !follows_dot(bytes, start)
                && !stripped[i..].trim_start().starts_with(':')
            {
                return true;
            }
            if word == "scope" && stripped[i..].trim_start().starts_with('(') {
                let args = stripped[i..].trim_start()[1..].trim_start();
                let first = args.split([',', ')']).next().unwrap_or("").trim();
                let scope = first.parse::<i64>().ok().or_else(|| {
                    first
                        .strip_prefix("0x")
                        .and_then(|h| i64::from_str_radix(h, 16).ok())
                });
                if scope.is_none_or(|n| n == 25) {
                    return true;
                }
            }
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// Replaces identifier tokens (outside string literals) per `map`.
fn rename_idents(text: &str, map: &BTreeMap<String, String>) -> String {
    let stripped = strip_strings(text);
    let bytes = stripped.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if !follows_dot(bytes, start)
                && let Some(to) = map.get(&text[start..i])
            {
                out.push_str(&text[last..start]);
                out.push_str(to);
                last = i;
            }
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// An operand whose read cannot fault and has no side effects: a named local (v2 writes every
/// named local before reading it), an integer literal, `stack_obj`, or `my.id`. Cells of
/// objects, temps, params, and globals stay: their reads can fault (an index past the data, or
/// past the frame), and so can an explicit `local[N]` the tree never wrote.
fn is_safe_operand(s: &str) -> bool {
    is_ident(s) || s.parse::<i32>().is_ok() || s == "my.id"
}

/// Whether running `next` first cannot observe the previous node's result: assignments and
/// `let` initializers are expression nodes, `return` sets the result, and so do `if`/`while`
/// tests that are expression compares (no primitive call).
fn is_decl(n: &Node) -> bool {
    matches!(n, Node::Line(l) if matches!(classify(l), Kind::Let { init: None, .. }))
}

/// `scope(S, I) op rhs;` with a call-free `rhs`: an expression node on a raw scope cell.
fn is_scope_store(line: &str) -> bool {
    let stripped = strip_strings(line);
    let Some(rest) = stripped.strip_prefix("scope(") else {
        return false;
    };
    let Some((_, after)) = rest.split_once(')') else {
        return false;
    };
    let mut parts = after.trim_start().splitn(2, ' ');
    matches!((parts.next(), parts.next()), (Some(op), Some(rhs))
        if ASSIGN_OPS.contains(&op) && !rhs.contains('(') && rhs.trim_end().ends_with(';'))
}

/// `rest` is the block after the node; `let x;` declarations emit nothing and are skipped.
fn next_ignores_last_result(rest: &[Node]) -> bool {
    let next = rest.iter().find(|n| !is_decl(n));
    match next {
        Some(Node::Line(l)) => {
            matches!(
                classify(l),
                Kind::Assign { .. } | Kind::Let { init: Some(_), .. } | Kind::Return
            ) || is_scope_store(l)
        }
        Some(Node::If { arms, .. }) => !arms[0].0.contains('('),
        Some(Node::While { cond: Some(c), .. }) => !c.contains('('),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Passes
// ---------------------------------------------------------------------------

fn for_each_block(nodes: &mut Vec<Node>, f: &mut impl FnMut(&mut Vec<Node>)) {
    for n in nodes.iter_mut() {
        match n {
            Node::Line(_) => {}
            Node::If { arms, else_body } => {
                for (_, body) in arms.iter_mut() {
                    for_each_block(body, f);
                }
                if let Some(body) = else_body {
                    for_each_block(body, f);
                }
            }
            Node::While { body, .. } => for_each_block(body, f),
        }
    }
    f(nodes);
}

fn collect_lets(nodes: &[Node], out: &mut Vec<String>) {
    for n in nodes {
        match n {
            Node::Line(l) => {
                if let Kind::Let { name, .. } = classify(l) {
                    out.push(name.to_owned());
                }
            }
            Node::If { arms, else_body } => {
                for (_, body) in arms {
                    collect_lets(body, out);
                }
                if let Some(body) = else_body {
                    collect_lets(body, out);
                }
            }
            Node::While { body, .. } => collect_lets(body, out),
        }
    }
}

fn drop_unreachable(nodes: &mut Vec<Node>) {
    for_each_block(nodes, &mut |block| {
        let Some(cut) = block.iter().position(|n| {
            matches!(n, Node::Line(l) if matches!(classify(l), Kind::Return | Kind::Break | Kind::Continue))
        }) else {
            return;
        };
        if cut + 1 >= block.len() {
            return;
        }
        let dropped: Vec<Node> = block.drain(cut + 1..).collect();
        let mut names = Vec::new();
        collect_lets(&dropped, &mut names);
        let decls: Vec<Node> = names
            .into_iter()
            .map(|n| Node::Line(format!("let {n};")))
            .collect();
        // Keep the declarations (no nodes) ahead of the jump; unused ones are dropped later.
        block.splice(cut..cut, decls);
    });
}

fn stack_obj_store(line: &str) -> Option<&str> {
    match classify(line) {
        Kind::Assign {
            lhs: "stack_obj",
            op: ":=",
            rhs,
        } => Some(rhs),
        _ => None,
    }
}

fn drop_dead_stack_obj_stores(nodes: &mut Vec<Node>) {
    for_each_block(nodes, &mut |block| {
        let mut i = 0;
        // What `stack_obj` provably equals here (a local name or `my.id`), in straight-line code.
        let mut known: Option<String> = None;
        while i < block.len() {
            let next = block[i + 1..].iter().find(|n| !is_decl(n));
            let Node::Line(line) = &block[i] else {
                known = None;
                i += 1;
                continue;
            };
            if let Some(rhs) = stack_obj_store(line) {
                let safe = is_safe_operand(rhs) || rhs == "stack_obj";
                let next_is_return =
                    matches!(next, Some(Node::Line(l)) if classify(l) == Kind::Return);
                // The next store must not read the value this one writes.
                let next_is_store = matches!(next, Some(Node::Line(l))
                    if stack_obj_store(l).is_some_and(|r| r != "stack_obj" && is_safe_operand(r)));
                let redundant = known.as_deref() == Some(rhs) || rhs == "stack_obj";
                // Nothing reads the value before it is overwritten or the frame returns.
                let unread = stack_obj_fate(&block[i + 1..]) == StackObjFate::Dead;
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
                Kind::Let {
                    name,
                    init: Some("stack_obj"),
                } => known = Some(name.to_owned()),
                Kind::Assign {
                    lhs,
                    op: ":=",
                    rhs: "stack_obj",
                } if is_ident(lhs) => known = Some(lhs.to_owned()),
                // Any other write to `stack_obj` (`+=`, ...) leaves its value unknown.
                Kind::Assign {
                    lhs: "stack_obj", ..
                } => known = None,
                Kind::Let { name, .. } | Kind::Assign { lhs: name, .. } => {
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

/// Primitives that never write the stack object, as v2 spells them.
const STACK_OBJ_KEEPING_PRIMS: &[&str] = &["test_object_type", "action_string", "alter_budget"];

/// Each call-like `name(` in a (string-stripped) text: `call N` for a subroutine call, the
/// primitive's name, `scope`, or `""` for a bare parenthesis.
fn callees(stripped: &str) -> impl Iterator<Item = &str> {
    stripped.match_indices('(').map(move |(at, _)| {
        let before = stripped[..at].trim_end();
        let start = before
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(0, |i| i + 1);
        let word = &before[start..];
        if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
            let rest = before[..start].trim_end();
            if rest.ends_with("call") {
                return "call";
            }
        }
        word
    })
}

/// Raw scopes whose cells never involve the stack object: my attributes, globals, literals,
/// temps, params, advertisements, my person data and slots, locals, tuning constants, and the
/// job and neighborhood tables. Every other scope may (scope 27, the sprite flags, reads the
/// stack object's; scope 10 is the stack object itself).
const STACK_FREE_SCOPES: &[u32] = &[
    0, 3, 6, 7, 8, 9, 11, 12, 14, 18, 20, 25, 26, 28, 29, 30, 33, 34, 36,
];

/// Whether every `scope(S, ..)` in a (string-stripped) text names a stack-free scope.
fn scopes_stack_free(stripped: &str) -> bool {
    stripped.match_indices("scope(").all(|(at, _)| {
        let preceded = stripped[..at]
            .bytes()
            .next_back()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
        let rest = &stripped[at + "scope(".len()..];
        let num = rest.split(',').next().unwrap_or("").trim();
        preceded
            || num
                .parse::<u32>()
                .is_ok_and(|n| STACK_FREE_SCOPES.contains(&n))
    })
}

/// Whether a statement or condition may read the stack object: it names `stack_obj` or a
/// `stack.` cell or a raw scope that involves the stack object, or runs anything (a call
/// inherits the stack object; a primitive may read it).
fn may_read_stack_obj(text: &str) -> bool {
    let stripped = strip_strings(text);
    stripped.contains("stack")
        || !scopes_stack_free(&stripped)
        || callees(&stripped).any(|c| c != "scope")
}

/// Whether a statement or condition may write the stack object. Subroutine calls cannot (the
/// callee's frame has its own copy).
fn may_write_stack_obj(text: &str) -> bool {
    if matches!(
        classify(text),
        Kind::Assign {
            lhs: "stack_obj",
            op: ":=",
            rhs: "stack_obj"
        }
    ) {
        return false;
    }
    if matches!(
        classify(text),
        Kind::Assign {
            lhs: "stack_obj",
            ..
        }
    ) {
        return true;
    }
    let stripped = strip_strings(text);
    names_stack_obj_as_target(&stripped)
        || stripped.contains("dest: stack_obj")
        || scope_store_lhs(&stripped).is_some_and(|n| n == 10)
        || callees(&stripped)
            .any(|c| !matches!(c, "call" | "scope") && !STACK_OBJ_KEEPING_PRIMS.contains(&c))
}

/// `stack_obj` is the target of an `.air` expression, by token: followed by anything but the end
/// of an operand or a comparison (`stack_obj:=9`, `stack_obj += 1`, `stack_obj sqrt 9`, the
/// writing condition `stack_obj inc_lt 9`). Text that does not lex may.
fn names_stack_obj_as_target(text: &str) -> bool {
    use crate::lexer::TokenKind as T;
    let Ok(tokens) = crate::lexer::tokenize(text) else {
        return true;
    };
    tokens.windows(2).any(|w| {
        matches!(&w[0].kind, T::Ident(n) if n == "stack_obj")
            && !matches!(
                w[1].kind,
                T::Semicolon
                    | T::RParen
                    | T::Comma
                    | T::RBracket
                    | T::LBrace
                    | T::Eof
                    | T::EqEq
                    | T::BangEq
                    | T::Lt
                    | T::LtEq
                    | T::Gt
                    | T::GtEq
            )
    })
}

/// The scope number of a `scope(S, I) op ..` store's target.
fn scope_store_lhs(stripped: &str) -> Option<u32> {
    let rest = stripped.trim_start().strip_prefix("scope(")?;
    let (num, after) = rest.split_once(',')?;
    let (_, after) = after.split_once(')')?;
    let op = after.trim_start().split(' ').next()?;
    if !ASSIGN_OPS.contains(&op) {
        return None;
    }
    // A non-literal scope number counts as the stack object, to stay conservative.
    Some(num.trim().parse::<u32>().unwrap_or(10))
}

/// Whether lowered loop-body code (`body`) may reach a `continue` or its end with `stack_obj`
/// still moved away from its entry value: some line may write it and no later `restore` line
/// (`stack_obj := __self_obj;`) puts it back on every path. A field read through another
/// object moves the stack object and restores it right after, so it leaves nothing displaced.
/// A subroutine runs in its own frame with its own stack object, so a call writes none here.
/// Code that does not parse may.
///
/// Precision matters both ways: a missing restore leaves the exhaustion block on the wrong
/// object, and a needless one replaces the iterator's false result, which the block may read.
pub(crate) fn body_may_leave_stack_obj_moved(body: &str, restore: &str) -> bool {
    fn line_writes(l: &str) -> bool {
        may_write_stack_obj(l)
    }
    /// `moved` after `nodes`, setting `exits` if a `continue` may see it moved.
    fn flow(nodes: &[Node], mut moved: bool, restore: &str, exits: &mut bool) -> bool {
        for n in nodes {
            match n {
                Node::Line(l) if l.trim() == restore => moved = false,
                Node::Line(l) => {
                    if matches!(classify(l), Kind::Continue) {
                        *exits |= moved;
                    }
                    moved |= line_writes(l);
                }
                Node::If { arms, else_body } => {
                    let mut out = false;
                    for (cond, body) in arms {
                        moved |= line_writes(cond);
                        out |= flow(body, moved, restore, exits);
                    }
                    out |= match else_body {
                        Some(body) => flow(body, moved, restore, exits),
                        None => moved,
                    };
                    moved = out;
                }
                Node::While { cond, body } => {
                    // Twice: the second pass starts from whatever one iteration may leave.
                    for _ in 0..2 {
                        moved |= cond.as_deref().is_some_and(line_writes);
                        moved |= flow(body, moved, restore, exits);
                    }
                }
            }
        }
        moved
    }
    let Some(nodes) = parse_body(body) else {
        return true;
    };
    let mut exits = false;
    flow(&nodes, false, restore, &mut exits) || exits
}

fn writes_stack_obj_anywhere(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Line(l) => may_write_stack_obj(l),
        Node::If { arms, else_body } => {
            arms.iter()
                .any(|(c, b)| may_write_stack_obj(c) || writes_stack_obj_anywhere(b))
                || else_body.as_deref().is_some_and(writes_stack_obj_anywhere)
        }
        Node::While { cond, body } => {
            cond.as_deref().is_some_and(may_write_stack_obj) || writes_stack_obj_anywhere(body)
        }
    })
}

/// The entry save `let __self_obj = stack_obj;` (or `__saved_stack_obj`), as its name.
fn entry_save(nodes: &[Node]) -> Option<&str> {
    match nodes.first()? {
        Node::Line(l) => match classify(l) {
            Kind::Let {
                name: name @ ("__self_obj" | "__saved_stack_obj"),
                init: Some("stack_obj"),
            } => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// Reads of the entry save become `stack_obj` while nothing can have changed the stack object
/// since entry: one fewer use of the save, and a restore there becomes `stack_obj := stack_obj`.
fn forward_entry_save(nodes: &mut [Node]) {
    let Some(name) = entry_save(nodes) else {
        return;
    };
    let map = BTreeMap::from([(name.to_owned(), "stack_obj".to_owned())]);
    forward_save_block(&mut nodes[1..], &map);
}

/// Renames per `map` up to the first node that may write `stack_obj`; returns whether control
/// falling off the end of `nodes` still has the entry stack object.
fn forward_save_block(nodes: &mut [Node], map: &BTreeMap<String, String>) -> bool {
    // `Err` when a primitive names the save other than as a labelled value operand: it may be
    // a local-slot operand (`icon:`, `local:`, ...) or a destination, so forwarding stops.
    let renamed = |text: &str| -> Result<Option<String>, ()> {
        if !idents(text).iter().any(|id| map.contains_key(*id)) {
            return Ok(None);
        }
        if !value_operands_only(text, map) {
            return Err(());
        }
        Ok(Some(rename_idents(text, map)))
    };
    for n in nodes.iter_mut() {
        match n {
            Node::Line(l) => {
                let writes_save = match classify(l) {
                    Kind::Let { name, .. } | Kind::Assign { lhs: name, .. } => {
                        map.contains_key(name)
                    }
                    _ => false,
                };
                if writes_save {
                    return false;
                }
                match renamed(l) {
                    Ok(Some(r)) => *l = r,
                    Ok(None) => {}
                    Err(()) => return false,
                }
                if may_write_stack_obj(l) {
                    return false;
                }
            }
            Node::If { arms, else_body } => {
                let mut kept = true;
                for (cond, body) in arms.iter_mut() {
                    match renamed(cond) {
                        Ok(Some(r)) => *cond = r,
                        Ok(None) => {}
                        Err(()) => return false,
                    }
                    if may_write_stack_obj(cond) {
                        return false;
                    }
                    kept &= forward_save_block(body, map);
                }
                if let Some(body) = else_body {
                    kept &= forward_save_block(body, map);
                }
                if !kept {
                    return false;
                }
            }
            Node::While { cond, body } => {
                if cond.as_deref().is_some_and(may_write_stack_obj)
                    || writes_stack_obj_anywhere(body)
                {
                    return false;
                }
                if let Some(c) = cond {
                    match renamed(c) {
                        Ok(Some(r)) => *c = r,
                        Ok(None) => {}
                        Err(()) => return false,
                    }
                }
                forward_save_block(body, map);
            }
        }
    }
    true
}

/// Primitive arguments `.air` parses as a local slot rather than a value.
/// (`value:` is Relationship's, which the primitive may write.)
const LOCAL_SLOT_ARGS: &[&str] = &["icon", "skin", "age", "sex", "local", "dest", "value"];

/// Whether every mention of a `map` key in `text` is a plain value read: the text runs no
/// primitive, or each mention is a labelled primitive argument (`label: name`) whose label
/// takes a value.
fn value_operands_only(text: &str, map: &BTreeMap<String, String>) -> bool {
    let stripped = strip_strings(text);
    if !callees(&stripped).any(|c| !c.is_empty() && c != "scope") {
        return true;
    }
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if follows_dot(bytes, start) || !map.contains_key(&stripped[start..i]) {
                continue;
            }
            let before = stripped[..start].trim_end();
            let Some(label_end) = before.strip_suffix(':') else {
                return false;
            };
            let label_end = label_end.trim_end();
            let label_start = label_end
                .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .map_or(0, |p| p + 1);
            if LOCAL_SLOT_ARGS.contains(&&label_end[label_start..]) {
                return false;
            }
            let after = stripped[i..].trim_start();
            if !(after.starts_with(',') || after.starts_with(')')) {
                return false;
            }
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    true
}

/// Drops an entry save nothing else names (see `drop_unread_entry_save`), when no named local
/// that moves down a frame word is read before it is written.
fn drop_unread_entry_save_node(nodes: &mut Vec<Node>) {
    let Some(name) = entry_save(nodes) else {
        return;
    };
    if !mentions(&nodes[1..], name)
        && !nodes_format_text(&nodes[1..])
        && locals_defined_before_reads(&nodes[1..])
    {
        nodes.remove(0);
    }
}

/// Whether every named local in `nodes` is written before anything reads it.
fn locals_defined_before_reads(nodes: &[Node]) -> bool {
    let mut lets = Vec::new();
    collect_lets(nodes, &mut lets);
    lets.iter()
        .all(|name| defined_before_reads(nodes, name, &mut false))
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

fn stack_obj_fate(nodes: &[Node]) -> StackObjFate {
    for n in nodes {
        match n {
            Node::Line(l) => {
                if let Some(rhs) = stack_obj_store(l)
                    && !may_read_stack_obj(rhs)
                {
                    return StackObjFate::Dead;
                }
                match classify(l) {
                    Kind::Return if !may_read_stack_obj(l) => return StackObjFate::Dead,
                    Kind::Break | Kind::Continue => return StackObjFate::Live,
                    _ if may_read_stack_obj(l) => return StackObjFate::Live,
                    _ => {}
                }
            }
            Node::If { arms, else_body } => {
                let mut all_dead = else_body.is_some();
                for (cond, body) in arms {
                    if may_read_stack_obj(cond) {
                        return StackObjFate::Live;
                    }
                    match stack_obj_fate(body) {
                        StackObjFate::Live => return StackObjFate::Live,
                        StackObjFate::Through => all_dead = false,
                        StackObjFate::Dead => {}
                    }
                }
                if let Some(body) = else_body {
                    match stack_obj_fate(body) {
                        StackObjFate::Live => return StackObjFate::Live,
                        StackObjFate::Through => all_dead = false,
                        StackObjFate::Dead => {}
                    }
                }
                if all_dead {
                    return StackObjFate::Dead;
                }
            }
            Node::While { .. } => return StackObjFate::Live,
        }
    }
    StackObjFate::Through
}

/// A plain `stack_obj := x;` store that is dead when control next reaches a `return` (the frame,
/// and its stack object, goes away; `return` sets the result).
fn is_droppable_stack_obj_store(n: &Node) -> bool {
    matches!(n, Node::Line(l) if stack_obj_store(l).is_some_and(|r| is_safe_operand(r) || r == "stack_obj"))
}

/// Drops `stack_obj` stores that end an `if` arm when the `if` itself is followed by `return`
/// (`if c { ..; stack_obj := s; } return x;`), at any depth, and then an `else` left empty.
/// `tail_returns` says that falling off the end of `nodes` reaches a `return`.
fn drop_stack_obj_stores_before_exit(nodes: &mut Vec<Node>, tail_returns: bool) {
    for i in 0..nodes.len() {
        let next = nodes[i + 1..].iter().find(|n| !is_decl(n));
        let next_returns = match next {
            Some(Node::Line(l)) => classify(l) == Kind::Return,
            Some(_) => false,
            None => tail_returns,
        };
        match &mut nodes[i] {
            Node::If { arms, else_body } => {
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
            Node::While { body, .. } => drop_stack_obj_stores_before_exit(body, false),
            Node::Line(_) => {}
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

/// Whether any line or condition in `nodes` names `name`.
fn mentions(nodes: &[Node], name: &str) -> bool {
    let named = |text: &str| text.contains(name) && idents(text).contains(&name);
    nodes.iter().any(|n| match n {
        Node::Line(l) => named(l),
        Node::If { arms, else_body } => {
            arms.iter().any(|(c, b)| named(c) || mentions(b, name))
                || else_body.as_deref().is_some_and(|b| mentions(b, name))
        }
        Node::While { cond, body } => cond.as_deref().is_some_and(named) || mentions(body, name),
    })
}

/// Every mention of each identifier: `(reads, writes-only lines)`.
fn mention_counts<'a>(nodes: &'a [Node], counts: &mut BTreeMap<&'a str, (usize, usize)>) {
    let add_text = |text: &'a str, counts: &mut BTreeMap<&'a str, (usize, usize)>| {
        for id in idents(text) {
            counts.entry(id).or_default().0 += 1;
        }
    };
    for n in nodes {
        match n {
            Node::Line(l) => match classify(l) {
                Kind::Let { name, init } => {
                    counts.entry(name).or_default().1 += 1;
                    if let Some(init) = init {
                        add_text(init, counts);
                    }
                }
                Kind::Assign { lhs, op: ":=", rhs } if is_ident(lhs) => {
                    counts.entry(lhs).or_default().1 += 1;
                    add_text(rhs, counts);
                }
                _ => add_text(l, counts),
            },
            Node::If { arms, else_body } => {
                for (cond, body) in arms {
                    add_text(cond, counts);
                    mention_counts(body, counts);
                }
                if let Some(body) = else_body {
                    mention_counts(body, counts);
                }
            }
            Node::While { cond, body } => {
                if let Some(c) = cond {
                    add_text(c, counts);
                }
                mention_counts(body, counts);
            }
        }
    }
}

/// Whether a block can leave an enclosing loop other than by falling off its end: a `break` or
/// `continue` at this loop level (nested loops own theirs).
fn has_loop_exit(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Line(l) => matches!(classify(l), Kind::Break | Kind::Continue),
        Node::If { arms, else_body } => {
            arms.iter().any(|(_, b)| has_loop_exit(b))
                || else_body.as_deref().is_some_and(has_loop_exit)
        }
        Node::While { .. } => false,
    })
}

/// `while true { B; break; }` with no other exit from `B` runs `B` once: it becomes `B`. The
/// condition-free loop emits no node, and the `break` targets what follows the loop, which is
/// where `B` falls through to.
fn unwrap_single_pass_loops(nodes: &mut Vec<Node>) {
    for_each_block(nodes, &mut |block| {
        let mut i = 0;
        while i < block.len() {
            let unwrap = match &block[i] {
                Node::While { cond: None, body } => {
                    matches!(body.last(), Some(Node::Line(l)) if classify(l) == Kind::Break)
                        && !has_loop_exit(&body[..body.len() - 1])
                }
                _ => false,
            };
            if !unwrap {
                i += 1;
                continue;
            }
            let Node::While { mut body, .. } = block.remove(i) else {
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
fn drop_dead_local_stores(nodes: &mut Vec<Node>) {
    // An `@slot` local's stores stay: the word is pinned so a frame matches one that other
    // code (a raw block, a later tree reusing the frame) reads by number.
    let mut names = Vec::new();
    collect_lets(nodes, &mut names);
    let locals: BTreeSet<String> = names
        .into_iter()
        .filter(|n| !n.starts_with("__slot"))
        .collect();
    let mut lv = Liveness {
        locals: &locals,
        text_reads_locals: nodes_format_text(nodes),
        edges: BTreeSet::new(),
        dead_defs: BTreeMap::new(),
        live_out: BTreeMap::new(),
    };
    lv.block(nodes, Set::new(), &Set::new(), &Set::new());
    let dead: BTreeSet<*const Node> = lv
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
            let Node::Line(l) = &block[i] else {
                continue;
            };
            let (name, safe) = match classify(l) {
                Kind::Let {
                    name,
                    init: Some(init),
                } => (Some(name.to_owned()), is_safe_operand(init)),
                Kind::Assign { rhs, .. } => (None, is_safe_operand(rhs) || rhs == "stack_obj"),
                _ => continue,
            };
            if !safe {
                continue;
            }
            match name {
                Some(name) => block[i] = Node::Line(format!("let {name};")),
                None => {
                    block.remove(i);
                }
            }
        }
    });
}

/// A plain store `(let) t = e;` or `t := e;`, as `(t, e, is_let)`.
fn plain_store(line: &str) -> Option<(&str, &str, bool)> {
    match classify(line) {
        Kind::Let {
            name,
            init: Some(init),
        } => Some((name, init, true)),
        Kind::Assign { lhs, op: ":=", rhs } => Some((lhs, rhs, false)),
        _ => None,
    }
}

/// `t := e; d := t;` with `t` a v2 local that is dead afterwards becomes `d := e;`: one
/// expression node reading `e` where two read it in turn, with nothing in between.
fn forward_copies(nodes: &mut Vec<Node>) {
    // An `@slot` local's stores stay: the word is pinned so a frame matches one that other
    // code (a raw block, a later tree reusing the frame) reads by number.
    let mut names = Vec::new();
    collect_lets(nodes, &mut names);
    let locals: BTreeSet<String> = names
        .into_iter()
        .filter(|n| !n.starts_with("__slot"))
        .collect();
    let mut lv = Liveness {
        locals: &locals,
        text_reads_locals: nodes_format_text(nodes),
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
            let (Node::Line(first), Node::Line(second)) = (&block[i], &block[i + 1]) else {
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
            block[i + 1] = Node::Line(merged);
            skip = true;
            match decl {
                Some(decl) => block[i] = Node::Line(decl),
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
fn drop_identity_stores(nodes: &mut Vec<Node>) {
    for_each_block(nodes, &mut |block| {
        for i in (0..block.len()).rev() {
            let Node::Line(l) = &block[i] else {
                continue;
            };
            let Kind::Assign { lhs, op: ":=", rhs } = classify(l) else {
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
fn next_is_plain_expression(rest: &[Node]) -> bool {
    let Some(Node::Line(l)) = rest.iter().find(|n| !is_decl(n)) else {
        return false;
    };
    let text = strip_strings(l);
    matches!(
        classify(l),
        Kind::Assign { .. } | Kind::Let { init: Some(_), .. }
    ) && !text.contains('(')
        && !text.contains("global")
}

/// Drops `let x;` declarations of names nothing else mentions.
fn drop_unused_decls(nodes: &mut Vec<Node>) {
    // A tree that formats text may read a user local's frame word by number, so its user
    // locals keep their declarations (and so their words) even when nothing names them.
    let keep_user = nodes_format_text(nodes);
    // Only the names to drop are copied out of the borrowed counts.
    let unused: BTreeSet<String> = {
        let mut counts = BTreeMap::new();
        mention_counts(nodes, &mut counts);
        counts
            .into_iter()
            .filter(|&(name, _)| !(keep_user && is_source_local(name)))
            // A pinned `@slot` word stays declared: dropping it would free its number.
            .filter(|&(name, _)| !name.starts_with("__slot"))
            .filter(|&(_, (reads, writes))| reads == 0 && writes <= 1)
            .map(|(name, _)| name.to_owned())
            .collect()
    };
    for_each_block(nodes, &mut |block| {
        if !unused.is_empty() {
            block.retain(|n| match n {
                Node::Line(l) => match classify(l) {
                    Kind::Let { name, init: None } => !unused.contains(name),
                    _ => true,
                },
                _ => true,
            });
        }
        // `let x;` right before `x := e;` is spelled `let x = e;` (the same node).
        let mut i = 0;
        while i + 1 < block.len() {
            if let (Node::Line(decl), Node::Line(store)) = (&block[i], &block[i + 1])
                && let Kind::Let { name, init: None } = classify(decl)
                && let Kind::Assign { lhs, op: ":=", rhs } = classify(store)
                && lhs == name
                && !idents(rhs).contains(&name)
            {
                block[i] = Node::Line(format!("let {name} = {rhs};"));
                block.remove(i + 1);
            }
            i += 1;
        }
    });
}

fn merge_duplicate_tails(nodes: &mut Vec<Node>) {
    for_each_block(nodes, &mut |block| {
        let mut i = 0;
        while i < block.len() {
            let hoisted = match &mut block[i] {
                Node::If {
                    arms,
                    else_body: Some(else_body),
                } if arms.len() == 1 => {
                    let then_body = &mut arms[0].1;
                    let mut n = 0;
                    while n < then_body.len()
                        && n < else_body.len()
                        && matches!(&then_body[then_body.len() - 1 - n], Node::Line(_))
                        && then_body[then_body.len() - 1 - n] == else_body[else_body.len() - 1 - n]
                    {
                        n += 1;
                    }
                    if n == 0 {
                        Vec::new()
                    } else {
                        let tail: Vec<Node> = then_body.drain(then_body.len() - n..).collect();
                        else_body.truncate(else_body.len() - n);
                        tail
                    }
                }
                _ => Vec::new(),
            };
            if let Node::If { else_body, .. } = &mut block[i]
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

// ---------------------------------------------------------------------------
// Liveness and coalescing
// ---------------------------------------------------------------------------

/// Whether a tree body runs a primitive that formats text (see [`formats_text`]).
pub(crate) fn body_formats_text(body: &str) -> bool {
    body.lines().any(formats_text)
}

/// Whether `text` runs a primitive that formats text (Dialog, Action String), which reads frame
/// locals by number through `$Local:N`, `$TimeLocal:N` and similar tokens.
fn formats_text(text: &str) -> bool {
    let stripped = strip_strings(text);
    ["dialog(", "action_string("].iter().any(|prim| {
        stripped.match_indices(prim).any(|(at, _)| {
            !stripped[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        })
    })
}

fn nodes_format_text(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Line(l) => formats_text(l),
        Node::If { arms, else_body } => {
            arms.iter()
                .any(|(c, body)| formats_text(c) || nodes_format_text(body))
                || else_body.as_deref().is_some_and(nodes_format_text)
        }
        Node::While { cond, body } => {
            cond.as_deref().is_some_and(formats_text) || nodes_format_text(body)
        }
    })
}

type Set = BTreeSet<String>;

struct Liveness<'a> {
    locals: &'a BTreeSet<String>,
    /// The tree formats text (a dialog or action string), which reads frame locals by number
    /// (`$Local:N`, `$TimeLocal:N`): every such node reads every source local.
    text_reads_locals: bool,
    edges: BTreeSet<(String, String)>,
    /// For each plain store to a v2 local (`__...`): whether the local is dead after it. Loops
    /// revisit their bodies with growing live sets; the last visit, with the final sets, wins.
    dead_defs: BTreeMap<*const Node, bool>,
    /// The locals live after each simple statement (last visit wins, as above).
    live_out: BTreeMap<*const Node, Set>,
}

impl Liveness<'_> {
    fn mentions(&self, text: &str) -> Set {
        let mut out: Set = idents(text)
            .into_iter()
            .filter(|t| self.locals.contains(*t))
            .map(str::to_owned)
            .collect();
        if self.text_reads_locals && formats_text(text) {
            out.extend(self.locals.iter().filter(|l| is_source_local(l)).cloned());
        }
        out
    }

    /// Records that every mentioned local interferes with every local live around the point.
    fn record(&mut self, mentioned: &Set, around: &Set) {
        for a in mentioned {
            for b in around.iter().chain(mentioned.iter()) {
                if a != b {
                    let (x, y) = if a < b { (a, b) } else { (b, a) };
                    self.edges.insert((x.clone(), y.clone()));
                }
            }
        }
    }

    fn block(&mut self, nodes: &[Node], out: Set, brk: &Set, cont: &Set) -> Set {
        let mut live = out;
        for n in nodes.iter().rev() {
            live = self.node(n, live, brk, cont);
        }
        live
    }

    fn node(&mut self, n: &Node, out: Set, brk: &Set, cont: &Set) -> Set {
        match n {
            Node::Line(l) => {
                let (def, uses_text): (Option<&str>, &str) = match classify(l) {
                    Kind::Return => return Set::new(),
                    Kind::Break => return brk.clone(),
                    Kind::Continue => return cont.clone(),
                    Kind::Let { init: None, .. } => return out,
                    // A name outside `locals` (a pinned `@slot` word) is never a dead def.
                    Kind::Let {
                        name,
                        init: Some(init),
                    } => (self.locals.contains(name).then_some(name), init),
                    Kind::Assign { lhs, op: ":=", rhs } if self.locals.contains(lhs) => {
                        (Some(lhs), rhs)
                    }
                    _ => (None, l.as_str()),
                };
                self.live_out.insert(std::ptr::from_ref(n), out.clone());
                if let Some(d) = def
                    && d.starts_with("__")
                {
                    self.dead_defs
                        .insert(std::ptr::from_ref(n), !out.contains(d));
                }
                let uses = self.mentions(uses_text);
                let mut live_in = out.clone();
                if let Some(d) = def {
                    live_in.remove(d);
                }
                live_in.extend(uses.iter().cloned());
                let mut mentioned = uses;
                if let Some(d) = def {
                    mentioned.insert(d.to_owned());
                }
                let around: Set = out.union(&live_in).cloned().collect();
                self.record(&mentioned, &around);
                live_in
            }
            Node::If { arms, else_body } => {
                let mut acc = match else_body {
                    Some(body) => self.block(body, out.clone(), brk, cont),
                    None => out.clone(),
                };
                for (cond, body) in arms.iter().rev() {
                    let then_in = self.block(body, out.clone(), brk, cont);
                    let after_test: Set = then_in.union(&acc).cloned().collect();
                    let m = self.mentions(cond);
                    let mut live_in = after_test.clone();
                    live_in.extend(m.iter().cloned());
                    self.record(&m, &live_in);
                    acc = live_in;
                }
                acc
            }
            Node::While { cond, body } => {
                let m = cond
                    .as_deref()
                    .map(|c| self.mentions(c))
                    .unwrap_or_default();
                let mut head: Set = m.clone();
                if cond.is_some() {
                    head.extend(out.iter().cloned());
                }
                loop {
                    let body_in = self.block(body, head.clone(), &out, &head);
                    let mut new_head = body_in;
                    if cond.is_some() {
                        new_head.extend(out.iter().cloned());
                        new_head.extend(m.iter().cloned());
                    }
                    if new_head == head {
                        break;
                    }
                    head = new_head;
                }
                self.record(&m, &head);
                head
            }
        }
    }
}

/// A local the source names (`let x`, an inlined helper's `let x`, `@slot(N) let x`), as
/// opposed to a compiler temporary: a tree that formats text may read its word by number.
fn is_source_local(name: &str) -> bool {
    name.starts_with("__v_") || name.starts_with("__inline_v_") || name.starts_with("__slot")
}

/// The coalescing class of a local, or `None` when it must keep its own slot.
fn merge_group(name: &str) -> Option<String> {
    // `@slot(N)` locals (`__slot<N>_..`) are frame words of their own: never merged.
    if matches!(name, "__self_obj" | "__saved_stack_obj") || name.starts_with("__slot") {
        return None;
    }
    let (stem, digits) = name.rsplit_once('_')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if let Some(base) = stem
        .strip_prefix("__v_")
        .or_else(|| stem.strip_prefix("__inline_v_"))
    {
        return Some(format!("v:{base}"));
    }
    stem.starts_with("__").then(|| "temp".to_owned())
}

fn coalesce_locals(nodes: &mut Vec<Node>) {
    let mut order = Vec::new();
    collect_lets(nodes, &mut order);
    let mut seen = BTreeSet::new();
    order.retain(|n| seen.insert(n.clone()));
    let locals: BTreeSet<String> = order.iter().cloned().collect();
    let mut lv = Liveness {
        locals: &locals,
        text_reads_locals: nodes_format_text(nodes),
        edges: BTreeSet::new(),
        dead_defs: BTreeMap::new(),
        live_out: BTreeMap::new(),
    };
    let entry_live = lv.block(nodes, Set::new(), &Set::new(), &Set::new());
    let interferes = |a: &str, b: &str| {
        let (x, y) = if a < b { (a, b) } else { (b, a) };
        lv.edges.contains(&(x.to_owned(), y.to_owned()))
    };

    // Greedy, in declaration order: each local joins the first earlier class of its group that
    // it does not interfere with.
    let mut classes: Vec<(String, Vec<String>)> = Vec::new();
    let mut rename: BTreeMap<String, String> = BTreeMap::new();
    for name in &order {
        let Some(group) = merge_group(name) else {
            continue;
        };
        if entry_live.contains(name) {
            continue;
        }
        let fits = |members: &[String]| members.iter().all(|m| !interferes(m, name));
        let target = classes
            .iter()
            .position(|(g, members)| *g == group && fits(members))
            .or_else(|| {
                if lv.text_reads_locals || !name.starts_with("__inline_v_") {
                    return None;
                }
                classes.iter().position(|(_, members)| {
                    members.iter().all(|m| m.starts_with("__inline_v_")) && fits(members)
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

fn rewrite(nodes: &mut Vec<Node>, rename: &BTreeMap<String, String>) {
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes.drain(..) {
        match n {
            Node::Line(l) => {
                if let Kind::Let { name, init } = classify(&l)
                    && let Some(to) = rename.get(name)
                {
                    if let Some(init) = init {
                        out.push(Node::Line(format!(
                            "{to} := {};",
                            rename_idents(init, rename)
                        )));
                    }
                    continue;
                }
                out.push(Node::Line(rename_idents(&l, rename)));
            }
            Node::If {
                mut arms,
                mut else_body,
            } => {
                for (cond, body) in &mut arms {
                    *cond = rename_idents(cond, rename);
                    rewrite(body, rename);
                }
                if let Some(body) = &mut else_body {
                    rewrite(body, rename);
                }
                out.push(Node::If { arms, else_body });
            }
            Node::While { cond, mut body } => {
                rewrite(&mut body, rename);
                out.push(Node::While {
                    cond: cond.map(|c| rename_idents(&c, rename)),
                    body,
                });
            }
        }
    }
    *nodes = out;
}

#[cfg(test)]
mod tests {
    use super::{drop_unread_entry_save, optimize_tree_body, parse_body, pin_in_place};

    #[test]
    fn dead_stores_unread_self_obj_and_coalescing() {
        let body = "    let __self_obj = stack_obj;\n    let __a_1 = 0;\n    __a_1 := my.attr[0];\n    my.attr[1] := __a_1;\n    my.attr[4] := __a_1;\n    let __b_2 = my.attr[2];\n    my.attr[3] := __b_2;\n    my.attr[5] := __b_2;\n    stack_obj := __self_obj;\n    return true;\n";
        let out = optimize_tree_body(body);
        assert_eq!(
            out,
            "    let __a_1 = my.attr[0];\n    my.attr[1] := __a_1;\n    my.attr[4] := __a_1;\n    __a_1 := my.attr[2];\n    my.attr[3] := __a_1;\n    my.attr[5] := __a_1;\n    return true;\n"
        );
    }

    #[test]
    fn single_pass_loops_unwrap_and_dead_flags_drop() {
        let body = "    let __r_1;\n    let __ret_2 = 0;\n    while true {\n        __r_1 := my.attr[0];\n        __ret_2 := 1;\n        break;\n    }\n    my.attr[1] := __r_1;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body),
            "    my.attr[1] := my.attr[0];\n    return true;\n"
        );
        // A second exit keeps the loop (its `break` has nowhere else to go).
        let body = "    while true {\n        if my.attr[0] == 1 {\n            break;\n        }\n        my.attr[1] := 2;\n        break;\n    }\n    return true;\n";
        assert_eq!(optimize_tree_body(body), body);
    }

    #[test]
    fn copies_forward_only_when_the_temp_dies() {
        let body = "    let __t_1 = stack.attr[0];\n    my.attr[0] := __t_1;\n    my.attr[1] := __t_1;\n    let __u_2 = my.attr[2];\n    my.attr[3] := __u_2;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body),
            "    let __t_1 = stack.attr[0];\n    my.attr[0] := __t_1;\n    my.attr[1] := __t_1;\n    my.attr[3] := my.attr[2];\n    return true;\n"
        );
    }

    #[test]
    fn stores_before_primitives_stay() {
        // `idle` may observe the previous node's result, so the dead store before it stays.
        let body = "    let __a_1 = my.attr[0];\n    idle(1);\n    return true;\n";
        assert_eq!(optimize_tree_body(body), body);
    }

    #[test]
    fn leaves_labels_and_multi_statement_lines_alone() {
        for body in [
            "    let __a_1 = 0;\n    my.attr[0] := 1; call 256(temps);\n    return true;\n",
            "    let __a_1 = 0;\n    x:\n    return true;\n",
        ] {
            assert_eq!(optimize_tree_body(body), body);
        }
    }

    #[test]
    fn duplicate_tails_and_unreachable() {
        let body = "    if my.attr[0] == 1 {\n        my.attr[1] := 2;\n        my.attr[2] := 3;\n    } else {\n        my.attr[2] := 3;\n    }\n    return true;\n    my.attr[3] := 4;\n";
        assert_eq!(
            optimize_tree_body(body),
            "    if my.attr[0] == 1 {\n        my.attr[1] := 2;\n    }\n    my.attr[2] := 3;\n    return true;\n"
        );
    }

    #[test]
    fn loop_carried_locals_do_not_merge() {
        // `__a_1` is live around the loop back edge while `__b_2` is written in the body.
        let body = "    let __a_1 = 0;\n    while my.attr[0] < 5 {\n        let __b_2 = my.attr[1];\n        my.attr[2] := __b_2;\n        my.attr[4] := __b_2;\n        __a_1 += 1;\n    }\n    my.attr[3] := __a_1;\n    return true;\n";
        assert_eq!(optimize_tree_body(body), body);
    }

    #[test]
    fn stack_obj_store_read_by_the_next_store_stays() {
        // The second store reads the first one's value through `stack.`.
        let body = "    stack_obj := __o_1;\n    stack_obj := stack.attr[0];\n    my.attr[0] := 1;\n    return true;\n";
        let out = optimize_tree_body(body);
        assert!(out.contains("stack_obj := __o_1;"), "{out}");
        // An independent second store does kill the first.
        let body = "    let __o_1 = my.id;\n    stack_obj := __o_1;\n    stack_obj := 7;\n    my.attr[0] := 1;\n    return true;\n";
        assert!(!optimize_tree_body(body).contains("__o_1"));
    }

    #[test]
    fn compound_stack_obj_writes_forget_its_value() {
        let body = "    let __o_1 = my.id;\n    stack_obj := __o_1;\n    stack_obj += 1;\n    stack_obj := __o_1;\n    my.attr[0] := stack.attr[0];\n    return true;\n";
        let out = optimize_tree_body(body);
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
            assert_eq!(optimize_tree_body(&body), body, "{operand}");
        }
        let body = "    let __a_1 = my.id;\n    let __b_2 = 5;\n    return true;\n";
        assert_eq!(optimize_tree_body(body), "    return true;\n");
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
            assert_eq!(optimize_tree_body(&body), body, "{line}");
        }
        // A named `op:` argument and the operand-free builtin ops are not raw local references.
        for line in [
            "manage_inventory(op: 1, kind: 2, flags: 3, guid: 0x7f400011);",
            "op 0x0004 (0, 0, 0, 0, 0, 0, 0, 0);",
            "my.attr[1] := scope(12, 2);",
        ] {
            let body = format!("{dead}    {line}\n    return true;\n");
            assert_ne!(optimize_tree_body(&body), body, "{line}");
        }
    }

    #[test]
    fn review_regressions_keep_observers() {
        // An unread save goes even before a primitive that reads the last result: that
        // primitive then sees the frame's entry state, as in the source.
        let body =
            "    let __self_obj = stack_obj;\n    idle_for_input(param[0], 1);\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body),
            "    idle_for_input(param[0], 1);\n    return true;\n"
        );
        // Not when a local read before it is written would move to another frame word.
        let body = "    let __self_obj = stack_obj;\n    let __v_x_0;\n    temp[0] := __v_x_0;\n    call 300(temps);\n    return true;\n";
        assert_eq!(drop_unread_entry_save(body, false), None);
        // A raw line the structured pass cannot parse falls back to the declarations: written
        // where they come into scope, the save goes; declared without a value, it stays.
        let raw_line = "    if !call 281(0, 0) {\n    if push_interaction(menu: 2, object: 0, priority: 0, flags: 7) { return true; }\n    }\n";
        let body = format!(
            "    let __self_obj = stack_obj;\n    let __v_x_0 = 1;\n    local[0] := stack_obj;\n    stack_obj := my.id;\n{raw_line}    stack_obj := local[0];\n    temp[0] := __v_x_0;\n    return true;\n"
        );
        assert!(parse_body(&body).is_none());
        assert_eq!(
            drop_unread_entry_save(&body, false).as_deref(),
            Some(&body["    let __self_obj = stack_obj;\n".len()..])
        );
        let body = body.replace("let __v_x_0 = 1;", "let __v_x_0;");
        assert_eq!(drop_unread_entry_save(&body, false), None);
        // Scope 27 (sprite flags) reads the stack object, so the switch before it stays.
        let body = "    stack_obj := my.id;\n    temp[0] := 0;\n    scope(27, 0) := 1;\n    return true;\n";
        assert_eq!(optimize_tree_body(body), body);
        // A local-slot operand keeps naming the save.
        let body = "    let __self_obj = stack_obj;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 1, icon: __self_obj);\n    stack_obj := my.id;\n    return true;\n";
        assert!(optimize_tree_body(body).contains("icon: __self_obj"));
        // Raw numeric targets keep the line-based save drop away.
        let body = "    let __self_obj = stack_obj;\n    stack_obj := my.id;\n    random(temp[0], 1) -> true: 3, false: 3;\n    return true;\n";
        assert_eq!(drop_unread_entry_save(body, false), None);
        // A local read before any write is not moved into the pin slot.
        let body = "    let o;\n    local[0] := o;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 3, icon: 0);\n    return true;\n";
        assert_eq!(pin_in_place(body), body);
        // Nor when another local read before any write would move to a different word.
        let body = "    let __v_o_0 = stack_obj;\n    let __v_inherited_1;\n    temp[0] := __v_inherited_1;\n    local[0] := __v_o_0;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 3, icon: 0);\n    return true;\n";
        assert_eq!(pin_in_place(body), body);
        // With every local written first, the pin applies.
        let body = "    let __v_o_0 = stack_obj;\n    local[0] := __v_o_0;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 3, icon: 0);\n    return true;\n";
        assert_eq!(
            pin_in_place(body),
            "    local[0] := stack_obj;\n    push_interaction(menu: 0, object: 0, priority: 2, flags: 3, icon: 0);\n    return true;\n"
        );
    }

    #[test]
    fn entry_save_forwards_until_a_write_and_then_goes() {
        // Nothing changes the stack object before the restore, which becomes a no-op.
        let body = "    let __self_obj = stack_obj;\n    my.attr[0] := stack.attr[1];\n    stack_obj := __self_obj;\n    my.attr[2] := 1;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body),
            "    my.attr[0] := stack.attr[1];\n    my.attr[2] := 1;\n    return true;\n"
        );
        // After a write the restore is real, and so is the save.
        let body = "    let __self_obj = stack_obj;\n    stack_obj := my.id;\n    my.attr[0] := stack.attr[1];\n    stack_obj := __self_obj;\n    my.attr[1] := stack.attr[2];\n    return true;\n";
        assert_eq!(optimize_tree_body(body), body);
        // A primitive may write it too.
        let body = "    let __self_obj = stack_obj;\n    set_to_next(all);\n    stack_obj := __self_obj;\n    my.attr[1] := stack.attr[2];\n    return true;\n";
        assert_eq!(optimize_tree_body(body), body);
    }

    #[test]
    fn unread_stack_obj_stores_drop() {
        // Nothing reads the store before the frame returns.
        let body = "    let __o_1 = my.id;\n    stack_obj := __o_1;\n    my.attr[0] := 1;\n    return true;\n";
        assert_eq!(
            optimize_tree_body(body),
            "    my.attr[0] := 1;\n    return true;\n"
        );
        // A read on one path keeps it; so does a loop.
        for tail in [
            "    if my.attr[0] == 1 {\n        my.attr[1] := stack.attr[0];\n    }\n    return true;\n",
            "    while my.attr[0] < 3 {\n        my.attr[0] += 1;\n    }\n    return true;\n",
        ] {
            let body = format!("    let __o_1 = my.id;\n    stack_obj := __o_1;\n{tail}");
            let out = optimize_tree_body(&body);
            assert!(out.starts_with("    stack_obj := my.id;\n"), "{out}");
        }
    }

    #[test]
    fn strings_fields_and_wide_characters() {
        // Multi-byte string contents keep byte offsets aligned, and a `.field` named like a
        // local is not renamed.
        let body = "    let __a_1 = 1;\n    my.attr[0] := __a_1;\n    let __b_2 = 2;\n    dialog(\"caf\u{e9} \u{2603}\", bcon.__b_2);\n    my.attr[1] := __b_2;\n    return true;\n";
        let out = optimize_tree_body(body);
        assert!(
            out.contains("dialog(\"caf\u{e9} \u{2603}\", bcon.__b_2);"),
            "{out}"
        );
        assert!(out.contains("my.attr[1] := __a_1;"), "{out}");
    }
}
