//! Frame allocation: the pinned `push_interaction` slot (`pin_in_place`), `#[slot(N)]` placement
//! (`assign_frame_slots`), definite-definition flow, and local renaming.

use super::*;

/// Whether `text` (an `.air` statement or `if` condition) starts with a `push_interaction`
/// primitive, which does not read the last result.
pub(crate) fn is_push_interaction(text: &str) -> bool {
    let t = text.trim_start();
    let t = t.strip_prefix('!').unwrap_or(t).trim_start();
    t.starts_with("push_interaction(")
}

/// In a tree whose pinned pushes all copy the same plain local `s` into `local[0]`
/// (`local[0] := s;` right before each `push_interaction(.., object: 0, ..)`), makes `s` live in
/// `local[0]` itself and drops the copies. `.air` places named locals after `local[0]`, a pinned
/// tree has no `raw`, and only the push reads the slot, so the slot and `s` always agree.
///
/// `ranges` says which Random Number ranges define their destination ([`DrawRanges`]).
pub(crate) fn pin_in_place(body: &str, ranges: DrawRanges) -> String {
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
            .all(|name| defined_before_reads(&nodes, name, &mut false, ranges))
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

/// Places a tree's `#[slot(N)] let` locals (`pinned`, which lowering names and numbers) in
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
    let Some((_, slots)) = plan_frame_slots(body, pinned, raw_declared) else {
        return body.to_owned();
    };
    place_locals(body, &slots).unwrap_or_else(|| body.to_owned())
}

/// The `.air` compiler's local-slot scan of a tree body, or `None` if `.air` cannot parse it.
fn body_local_usage(body: &str) -> Option<crate::lower::AirLocalUsage> {
    let wrapped = format!("tree 4096 \"__slots\" {{\n{body}\n}}\n");
    crate::parser::parse_module(&wrapped)
        .ok()
        .and_then(|m| match m.items.first() {
            Some(crate::ast::Item::Tree(t)) => Some(crate::lower::air_local_usage(&t.body)),
            _ => None,
        })
}

/// The words [`assign_frame_slots`] gives the locals it places, with the body's local usage.
fn plan_frame_slots(
    body: &str,
    pinned: &BTreeMap<String, usize>,
    raw_declared: &BTreeSet<String>,
) -> Option<(crate::lower::AirLocalUsage, BTreeMap<String, usize>)> {
    let usage = body_local_usage(body)?;
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
    Some((usage, slots))
}

/// Which local occupies each frame word of the finished tree `body` (before `#[slot(N)]`
/// placement, which `has_slots` says runs), read the way `.air` reads the placed body: a word
/// the placed body names (`local[N]`) holds the local placed there, or itself, and every
/// remaining `let` takes the word `.air` gives it, after the highest named word and the
/// header's `min_locals` floor (0 when the header has none), in declaration order. A placed
/// local the body never names past its `let x;` is gone from the placed body and holds no
/// word. The word right after the named locals holds [`IDLE_SCRATCH`] when `.air` saves
/// `param[0]` there around an `idle`: when the tree uses both (the same scan `.air`'s allocation
/// makes). `None` if `.air` cannot parse the body.
pub(crate) fn frame_occupancy(
    body: &str,
    pinned: &BTreeMap<String, usize>,
    raw_declared: &BTreeSet<String>,
    has_slots: bool,
    min_locals: usize,
) -> Option<BTreeMap<usize, String>> {
    let (usage, slots) = if has_slots {
        let (_, slots) = plan_frame_slots(body, pinned, raw_declared)?;
        (body_local_usage(&place_locals(body, &slots)?)?, slots)
    } else {
        (body_local_usage(body)?, BTreeMap::new())
    };
    let placed: BTreeMap<usize, &String> = slots.iter().map(|(name, &w)| (w, name)).collect();
    let mut words: BTreeMap<usize, String> = usage
        .explicit
        .iter()
        .map(|&w| {
            let occupant = placed
                .get(&w)
                .map_or_else(|| format!("local[{w}]"), |n| (*n).clone());
            (w, occupant)
        })
        .collect();
    let first = words.keys().last().map_or(0, |w| w + 1).max(min_locals);
    let mut end = first;
    for (word, name) in (first..).zip(&usage.lets) {
        words.insert(word, name.clone());
        end = word + 1;
    }
    if usage.uses_idle_scratch {
        words.insert(end, IDLE_SCRATCH.to_owned());
    }
    Some(words)
}

/// The occupant [`frame_occupancy`] gives the word after the named locals, where `.air` keeps
/// its `param[0]` save around an `idle` (not a name any local can have).
pub(crate) const IDLE_SCRATCH: &str = "<idle scratch>";

/// `body` with each local of `slots` written as `local[K]`, found by the `.air` lexer: `let x;`
/// (or `local x;`) goes, `let x = v;` becomes `local[K] := v;`, and every other operand `x`
/// becomes `local[K]`. A name after `.` (a field), before `:` (a label or an argument key), a
/// called name (`call x(`, `x(`, `subroutine(x)`), a name after `const`, or among a node's
/// branch targets (`-> .. ;`) is not an operand and stays. `None` if the body does not lex.
pub(crate) fn place_locals(body: &str, slots: &BTreeMap<String, usize>) -> Option<String> {
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
                // A field, a label or argument key, or a called name (`call x(..)`,
                // `subroutine x(..)`, `subroutine(x)`).
                let prev2 = i.checked_sub(2).map(|p| &tokens[p].kind);
                if matches!(prev, Some(T::Dot))
                    || matches!(next, Some(T::Colon | T::LParen))
                    || matches!(prev, Some(T::Ident(kw)) if kw == "call" || kw == "subroutine")
                    || (matches!(prev, Some(T::LParen))
                        && matches!(prev2, Some(T::Ident(kw)) if kw == "subroutine"))
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
pub(crate) fn check_pins(nodes: &[Stmt], copied: &mut BTreeSet<String>, ok: &mut bool) {
    for (i, n) in nodes.iter().enumerate() {
        match n {
            Stmt::Inst(l) => {
                let stripped = strip_strings(l);
                if !stripped.contains("local[") {
                    continue;
                }
                let followed_by_push = match nodes[i + 1..].iter().find(|n| !is_decl(n)) {
                    Some(Stmt::Inst(next)) => is_push_interaction(next),
                    Some(Stmt::If { arms, .. }) => is_push_interaction(&arms[0].0),
                    _ => false,
                };
                match classify(l) {
                    InstKind::Assign {
                        lhs: "local[0]",
                        op: ":=",
                        rhs,
                    } if is_ident(rhs) && followed_by_push => {
                        copied.insert(rhs.to_owned());
                    }
                    _ => *ok = false,
                }
            }
            Stmt::If { arms, else_body } => {
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
            Stmt::While { cond, body } => {
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
pub(crate) fn defined_before_reads(
    nodes: &[Stmt],
    name: &str,
    defined: &mut bool,
    ranges: DrawRanges,
) -> bool {
    // A `break` outside any loop has no loop exit to reach; the parser never produces one.
    let mut breaks = None;
    flow_defined(nodes, name, defined, &mut breaks, ranges)
}

/// The `&&` of two path states where either may be absent (no path reaches the join that way).
pub(crate) fn meet(a: Option<bool>, b: Option<bool>) -> Option<bool> {
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
pub(crate) fn flow_defined(
    nodes: &[Stmt],
    name: &str,
    defined: &mut bool,
    breaks: &mut Option<bool>,
    ranges: DrawRanges,
) -> bool {
    let reads = |text: &str| idents(text).contains(&name);
    for n in nodes {
        match n {
            Stmt::Inst(l) => match classify(l) {
                InstKind::Let { name: n, init } if n == name => {
                    if let Some(init) = init {
                        if reads(init) {
                            return false;
                        }
                        *defined = true;
                    }
                }
                InstKind::Assign { lhs, op: ":=", rhs } if lhs == name => {
                    if reads(rhs) {
                        return false;
                    }
                    *defined = true;
                }
                kind @ (InstKind::Return | InstKind::Break | InstKind::Continue) => {
                    if !*defined && reads(l) {
                        return false;
                    }
                    if matches!(kind, InstKind::Break) {
                        *breaks = meet(*breaks, Some(*defined));
                    }
                    *defined = true;
                }
                _ => match out_operand_write_with(l, name, ranges) {
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
            Stmt::If { arms, else_body } => {
                if !if_chain_defined(arms, else_body.as_deref(), name, defined, breaks, ranges) {
                    return false;
                }
            }
            Stmt::While { cond, body } => {
                let (mut d, exit) = match cond {
                    Some(c) => match cond_defines(c, name, *defined, ranges) {
                        Some((on_true, on_false)) => (on_true, Some(on_false)),
                        None => return false,
                    },
                    None => (*defined, None),
                };
                let mut inner = None;
                if !flow_defined(body, name, &mut d, &mut inner, ranges) {
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
pub(crate) fn if_chain_defined(
    arms: &[(String, Vec<Stmt>)],
    else_body: Option<&[Stmt]>,
    name: &str,
    defined: &mut bool,
    breaks: &mut Option<bool>,
    ranges: DrawRanges,
) -> bool {
    let Some(((cond, body), rest)) = arms.split_first() else {
        return match else_body {
            // No `else`: control falls through with what the conditions defined.
            None => true,
            Some(body) => flow_defined(body, name, defined, breaks, ranges),
        };
    };
    let Some((on_true, on_false)) = cond_defines(cond, name, *defined, ranges) else {
        return false;
    };
    let mut d_then = on_true;
    if !flow_defined(body, name, &mut d_then, breaks, ranges) {
        return false;
    }
    let mut d_else = on_false;
    if !if_chain_defined(rest, else_body, name, &mut d_else, breaks, ranges) {
        return false;
    }
    *defined = d_then && d_else;
    true
}

/// What a branch condition leaves defined on its true and false exits, or `None` when it reads
/// `name` before anything writes it.
pub(crate) fn cond_defines(
    cond: &str,
    name: &str,
    defined: bool,
    ranges: DrawRanges,
) -> Option<(bool, bool)> {
    let reads = |text: &str| idents(text).contains(&name);
    let (negated, inner) = match cond.trim_start().strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, cond),
    };
    let Some((rest, exit)) = out_operand_write_with(inner, name, ranges) else {
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
pub(crate) enum WriteExit {
    Both,
    True,
    False,
}

/// When `text` (a statement or condition, no leading `!`) is a primitive that stores into the
/// cell `name` through an out-operand, returns the text with that operand blanked (for the reads
/// it still makes) and the exits on which the store happens, as the VM runs them:
///
/// - `random(x, n)` stores the draw, then takes the true exit, unless its range may resolve to a
///   word pointer ([`range_may_take_draw`]), which redirects the draw into the range cell;
/// - `animate(.., local: x)` stores an event in the local, then takes the false exit;
/// - `relationship(.., value: x)` without flag 4 reads the number into the cell and takes the
///   true exit; with flag 1 a missing number takes the false exit unwritten, and without it
///   the row is added, so the false exit never runs.
pub(crate) fn out_operand_write(text: &str, name: &str) -> Option<(String, WriteExit)> {
    out_operand_write_with(text, name, DrawRanges::NotGlobal)
}

/// Which Random Number ranges a pass counts as leaving the draw in the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DrawRanges {
    /// Every range not spelled with a global ([`range_may_take_draw`]).
    NotGlobal,
    /// Literal ranges only: main's rule. A tree that may read global 34 keeps it, since a
    /// range that defines the destination lets the entry save or a pin copy go, an Expression
    /// whose scratch-pointer write that read may depend on.
    Literal,
}

impl DrawRanges {
    /// The rule for a tree that may (`true`) or may not read global 34.
    pub(crate) fn for_tree(may_read_global_34: bool) -> Self {
        if may_read_global_34 {
            DrawRanges::Literal
        } else {
            DrawRanges::NotGlobal
        }
    }
}

/// [`out_operand_write`] with the Random Number range rule `ranges`.
pub(crate) fn out_operand_write_with(
    text: &str,
    name: &str,
    ranges: DrawRanges,
) -> Option<(String, WriteExit)> {
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
            let range = args.get(1)?.trim();
            let defines = match ranges {
                DrawRanges::NotGlobal => !range_may_take_draw(range),
                DrawRanges::Literal => parse_int(range).is_some(),
            };
            (first.trim() == name && defines).then_some((0, WriteExit::Both))?
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

/// The most locals a frame holds: the default format's (`0x8002`) limit, the largest there is.
pub(crate) const MAX_FRAME_LOCALS: usize = 255;

/// The frame locals `.air` allocates for a lowered tree body: one per `let` / `local` name,
/// after every explicit local (`local[N]`, `scope(25, N)`). Read from the body's tokens, so
/// comments and strings name nothing. It words the `animate(event_local:)` diagnostic and bounds
/// declaration grouping ([`FrameFacts::fixed_frame`]); the `.air` compiler still rejects a tree
/// whose locals outgrow its frame, so a slot it misses (a raw primitive's numeric local operand)
/// is caught there.
pub(crate) fn count_frame_locals(body: &str) -> usize {
    use crate::lexer::{TokenKind as T, tokenize};
    let Ok(tokens) = tokenize(body) else {
        // `.air` rejects the body anyway.
        return 0;
    };
    let kinds: Vec<&T> = tokens.iter().map(|t| &t.kind).collect();
    let mut names = BTreeSet::new();
    let mut explicit: Option<i64> = None;
    for (i, k) in kinds.iter().enumerate() {
        let next = |n: usize| kinds.get(i + n).copied();
        match k {
            T::Ident(w) if w == "let" || (w == "local" && matches!(next(1), Some(T::Ident(_)))) => {
                if let Some(T::Ident(name)) = next(1) {
                    names.insert(name.as_str());
                }
            }
            T::Ident(w) if w == "local" => {
                if let (Some(T::LBracket), Some(T::Int(n))) = (next(1), next(2)) {
                    explicit = explicit.max(Some(*n));
                }
            }
            T::Ident(w) if w == "scope" => {
                if let (Some(T::LParen), Some(T::Int(25)), Some(T::Comma), Some(T::Int(n))) =
                    (next(1), next(2), next(3), next(4))
                {
                    explicit = explicit.max(Some(*n));
                }
            }
            _ => {}
        }
    }
    let first_named = explicit.map_or(0, |m| usize::try_from(m).unwrap_or(0) + 1);
    first_named + names.len()
}

/// The frame words `.air` allocates for the finished (slot-placed) tree `body` under a header
/// whose `min_locals` floor is `min_locals`, by `.air`'s own scan: the named locals after every
/// explicit local and the floor, and the `idle` scratch word after them when the tree uses
/// both `idle` and `param[0]`. `None` when `.air` cannot parse the body, or raw `op` bytes may
/// name locals no scan reads.
pub(crate) fn frame_words(body: &str, min_locals: usize) -> Option<usize> {
    let usage = body_local_usage(body)?;
    if usage.has_raw_ops {
        return None;
    }
    let first = usage.explicit.last().map_or(0, |w| w + 1).max(min_locals);
    Some(first + usage.lets.len() + usize::from(usage.uses_idle_scratch))
}

/// Whether `body` may read global 34, whose value depends on the scratch pointer every
/// Expression sets (`sims-vm` `primitives/expression.rs`): a `global[34]` operand or one whose
/// index is not a known number, a `scope(6, ..)` or `scope(<not a known number>, ..)` cell, or
/// a raw `op` that may be an Expression or a UI Effect naming it ([`raw_op_may_read_global_34`]).
/// A body that does not lex counts as reading it.
///
/// An index may be a constant: one of `consts` (the raw blocks' source constants, which the
/// tree declares at its top) or a `const NAME = N;` in the body. It is the number it names when
/// every declaration of the name gives the same number and no `let` / `local` reuses the name;
/// otherwise it may be anything.
pub(crate) fn may_read_global_34(body: &str, consts: &BTreeMap<String, String>) -> bool {
    use crate::lexer::TokenKind as T;
    let Ok(tokens) = crate::lexer::tokenize(body) else {
        return true;
    };
    let kind = |i: usize| tokens.get(i).map(|t| &t.kind);
    let known = const_numbers(&tokens, consts);
    // The number an index token names, if it is a literal or a known constant.
    let number = |k: Option<&T>| match k {
        Some(T::Int(n)) => Some(*n),
        Some(T::Ident(name)) => known.get(name).copied().flatten(),
        _ => None,
    };
    tokens.iter().enumerate().any(|(i, t)| match &t.kind {
        T::Ident(id) if id == "global" => {
            matches!(kind(i + 1), Some(T::LBracket))
                && !(number(kind(i + 2)).is_some_and(|n| n != 34)
                    && matches!(kind(i + 3), Some(T::RBracket)))
        }
        T::Ident(id) if id == "scope" => {
            matches!(kind(i + 1), Some(T::LParen))
                && !(number(kind(i + 2)).is_some_and(|n| n != 6)
                    && matches!(kind(i + 3), Some(T::Comma)))
        }
        T::Ident(id) if id == "op" => raw_op_may_read_global_34(&tokens[i + 1..]),
        _ => false,
    })
}

/// The number each constant names: `Some(n)` when every declaration of the name (in `consts`, or
/// a `const NAME = N;` in `tokens`) gives the literal `n` and no `let` / `local` declares it, and
/// `None` when they disagree or one is not a literal.
fn const_numbers(
    tokens: &[crate::lexer::Token],
    consts: &BTreeMap<String, String>,
) -> BTreeMap<String, Option<i64>> {
    use crate::lexer::TokenKind as T;
    let mut known: BTreeMap<String, Option<i64>> = BTreeMap::new();
    let mut note = |name: &str, value: Option<i64>| {
        known
            .entry(name.to_owned())
            .and_modify(|v| {
                if *v != value {
                    *v = None;
                }
            })
            .or_insert(value);
    };
    for (name, value) in consts {
        let literal = crate::lexer::tokenize(value).ok().and_then(|t| {
            match t.iter().map(|t| &t.kind).collect::<Vec<_>>().as_slice() {
                [T::Int(n)] | [T::Int(n), T::Eof] => Some(*n),
                _ => None,
            }
        });
        note(name, literal);
    }
    let kind = |i: usize| tokens.get(i).map(|t| &t.kind);
    for (i, t) in tokens.iter().enumerate() {
        match (&t.kind, kind(i + 1)) {
            (T::Ident(kw), Some(T::Ident(name))) if kw == "const" => {
                let literal = match (kind(i + 2), kind(i + 3), kind(i + 4)) {
                    (Some(T::Assign), Some(T::Int(n)), Some(T::Semicolon)) => Some(*n),
                    _ => None,
                };
                note(name, literal);
            }
            (T::Ident(kw), Some(T::Ident(name))) if kw == "let" || kw == "local" => {
                note(name, None);
            }
            _ => {}
        }
    }
    known
}

/// Whether a raw `op` (`tokens` follow the keyword) may read global 34 in the way that depends
/// on the scratch pointer. Two primitives do (`sims-vm`): Expression (opcode 2), whose right
/// operand is global 34 when operand byte 7 (its scope) is 6 and the word at bytes 2..4 (its
/// index) is 34 (`primitives/expression.rs`), and UI Effect (opcode 34), whose variable is
/// global 34 when byte 1 is 6 and the word at bytes 2..4 is 34 (`primitives/ui_effect.rs`).
/// Any other literal opcode reads none: below 256 it is another primitive, and from 256 up it
/// calls a tree, which `ObjectEnv::reads_global_34` covers for the object's own trees. An
/// opcode or operand byte that is not a literal may be anything, so it counts as a read. A
/// `op` not followed by an opcode is not a raw op.
fn raw_op_may_read_global_34(tokens: &[crate::lexer::Token]) -> bool {
    use crate::lexer::TokenKind as T;
    let scope_byte = match tokens.first().map(|t| &t.kind) {
        Some(T::Ident(_)) => return true,
        Some(T::Int(2)) => 7,
        Some(T::Int(34)) => 1,
        _ => return false,
    };
    // The operand list: `(b0, .., b7)`, its bytes separated by commas or by spaces.
    if !matches!(tokens.get(1).map(|t| &t.kind), Some(T::LParen)) {
        return true;
    }
    let mut bytes = Vec::new();
    for t in &tokens[2..] {
        match &t.kind {
            T::Int(b) => bytes.push(*b),
            T::Comma => {}
            T::RParen => break,
            _ => return true,
        }
    }
    bytes.len() != 8 || (bytes[scope_byte] == 6 && bytes[2] | bytes[3] << 8 == 34)
}

/// Whether a Random Number range operand (`.air` text) may resolve to a word pointer, which
/// takes the draw in place of the destination. The VM resolves the range read-only, and the only
/// cell that then comes back with a word pointer is global 34 (`resolve` in `sims-vm`'s
/// `memory.rs`). `.air` names a global only as `global[N]` or `scope(6, N)`, so any range not
/// spelled with `global` or `scope` (a literal, a `const`/`bcon` constant or named constant, a
/// local, a temp, a parameter, an attribute) leaves the draw in the destination.
pub(crate) fn range_may_take_draw(range: &str) -> bool {
    idents(range)
        .iter()
        .any(|id| matches!(*id, "global" | "scope"))
}

pub(crate) fn drop_pins(nodes: &mut Vec<Stmt>) {
    nodes.retain(|n| {
        !matches!(n, Stmt::Inst(l) if matches!(classify(l), InstKind::Assign { lhs: "local[0]", op: ":=", .. }))
    });
    for n in nodes.iter_mut() {
        match n {
            Stmt::Inst(_) => {}
            Stmt::If { arms, else_body } => {
                for (_, body) in arms.iter_mut() {
                    drop_pins(body);
                }
                if let Some(body) = else_body {
                    drop_pins(body);
                }
            }
            Stmt::While { body, .. } => drop_pins(body),
        }
    }
}

/// Whether every named local in `nodes` is written before anything reads it.
pub(crate) fn locals_defined_before_reads(nodes: &[Stmt], ranges: DrawRanges) -> bool {
    let mut lets = Vec::new();
    collect_lets(nodes, &mut lets);
    lets.iter()
        .all(|name| defined_before_reads(nodes, name, &mut false, ranges))
}

/// The coalescing class of a local, or `None` when it must keep its own slot.
pub(crate) fn merge_group(name: &str) -> Option<String> {
    // `#[slot(N)]` locals (`__slot<N>_..`) are frame words of their own: never merged.
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

pub(crate) fn rewrite(nodes: &mut Vec<Stmt>, rename: &BTreeMap<String, String>) {
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes.drain(..) {
        match n {
            Stmt::Inst(l) => {
                if let InstKind::Let { name, init } = classify(&l)
                    && let Some(to) = rename.get(name)
                {
                    if let Some(init) = init {
                        out.push(Stmt::Inst(format!(
                            "{to} := {};",
                            rename_idents(init, rename)
                        )));
                    }
                    continue;
                }
                out.push(Stmt::Inst(rename_idents(&l, rename)));
            }
            Stmt::If {
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
                out.push(Stmt::If { arms, else_body });
            }
            Stmt::While { cond, mut body } => {
                rewrite(&mut body, rename);
                out.push(Stmt::While {
                    cond: cond.map(|c| rename_idents(&c, rename)),
                    body,
                });
            }
        }
    }
    *nodes = out;
}
