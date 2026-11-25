//! `stack_obj` and last-result effects of instructions: which callees and scopes read or write the
//! stack object, whether a body may leave it moved, and whether the next node ignores the last
//! result.

use super::*;

/// `rest` is the block after the node; `let x;` declarations emit nothing and are skipped.
pub(crate) fn next_ignores_last_result(rest: &[Stmt]) -> bool {
    let next = rest.iter().find(|n| !is_decl(n));
    match next {
        Some(Stmt::Inst(l)) => {
            matches!(
                classify(l),
                InstKind::Assign { .. } | InstKind::Let { init: Some(_), .. } | InstKind::Return
            ) || is_scope_store(l)
        }
        Some(Stmt::If { arms, .. }) => !arms[0].0.contains('('),
        Some(Stmt::While { cond: Some(c), .. }) => !c.contains('('),
        _ => false,
    }
}

/// Primitives that never write the stack object, as v2 spells them.
pub(crate) const STACK_OBJ_KEEPING_PRIMS: &[&str] =
    &["test_object_type", "action_string", "alter_budget"];

/// Each call-like `name(` in a (string-stripped) text: `call` for a subroutine call (`call N(`,
/// `call name(`, `subroutine N(`), `subroutine` for `subroutine(N)`, the primitive's name,
/// `scope`, or `""` for a bare parenthesis.
pub(crate) fn callees(stripped: &str) -> impl Iterator<Item = &str> {
    stripped.match_indices('(').map(move |(at, _)| {
        let before = stripped[..at].trim_end();
        let start = before
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(0, |i| i + 1);
        let mut word = &before[start..];
        let mut start = start;
        // A quoted target (`call "name"(..)`): the word before the string.
        if word.is_empty()
            && before.ends_with('"')
            && let Some(open) = before[..before.len() - 1].rfind('"')
        {
            start = open;
            word = &before[open..];
        }
        // `call N(..)`, `call name(..)`, `subroutine N(..)`: the word before the target.
        let rest = before[..start].trim_end();
        let kw_at = rest
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(0, |i| i + 1);
        if !word.is_empty() && matches!(&rest[kw_at..], "call" | "subroutine") {
            return "call";
        }
        word
    })
}

/// Raw scopes whose cells never involve the stack object: my attributes, globals, literals,
/// temps, params, advertisements, my person data and slots, locals, tuning constants, and the
/// job and neighborhood tables. Every other scope may (scope 27, the sprite flags, reads the
/// stack object's; scope 10 is the stack object itself).
pub(crate) const STACK_FREE_SCOPES: &[u32] = &[
    0, 3, 6, 7, 8, 9, 11, 12, 14, 18, 20, 25, 26, 28, 29, 30, 33, 34, 36,
];

/// Whether every `scope(S, ..)` in a (string-stripped) text names a stack-free scope.
pub(crate) fn scopes_stack_free(stripped: &str) -> bool {
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
pub(crate) fn may_read_stack_obj(text: &str) -> bool {
    let stripped = strip_strings(text);
    stripped.contains("stack")
        || !scopes_stack_free(&stripped)
        || callees(&stripped).any(|c| c != "scope")
}

/// Whether a statement or condition may write the stack object. Subroutine calls cannot (the
/// callee's frame has its own copy).
pub(crate) fn may_write_stack_obj(text: &str) -> bool {
    if matches!(
        classify(text),
        InstKind::Assign {
            lhs: "stack_obj",
            op: ":=",
            rhs: "stack_obj"
        }
    ) {
        return false;
    }
    if matches!(
        classify(text),
        InstKind::Assign {
            lhs: "stack_obj",
            ..
        }
    ) {
        return true;
    }
    let stripped = strip_strings(text);
    names_stack_obj_as_target(&stripped)
        || stripped.contains("dest: stack_obj")
        || callees(&stripped).any(|c| {
            !matches!(c, "call" | "subroutine" | "scope") && !STACK_OBJ_KEEPING_PRIMS.contains(&c)
        })
}

/// Whether an `.air` expression has the stack object as its target, read from the tokens: the
/// stack object (`stack_obj`, or `scope(S, ..)` with `S` 10 or not a literal) followed by anything
/// but the end of an operand, a comparison or `flag_test` (`stack_obj:=9`,
/// `scope (10, 0) += 1`, `stack_obj sqrt 9`, the writing condition `stack_obj inc_lt 9`). Text
/// that does not lex may.
pub(crate) fn names_stack_obj_as_target(text: &str) -> bool {
    use crate::lexer::TokenKind as T;
    let Ok(tokens) = crate::lexer::tokenize(text) else {
        return true;
    };
    let kinds: Vec<&T> = tokens.iter().map(|t| &t.kind).collect();
    // The end of an operand, or a reading operator (a comparison, `flag_test`).
    let ends_operand = |k: Option<&&T>| {
        matches!(k, Some(T::Ident(w)) if w == "flag_test")
            || matches!(
                k,
                None | Some(
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
            )
    };
    (0..kinds.len()).any(|i| match kinds[i] {
        T::Ident(n) if n == "stack_obj" => !ends_operand(kinds.get(i + 1)),
        T::Ident(n) if n == "scope" && matches!(kinds.get(i + 1), Some(T::LParen)) => {
            let stack = !matches!(kinds.get(i + 2), Some(T::Int(s)) if *s != 10);
            // The token after the operand's closing parenthesis.
            let mut depth = 0usize;
            let close = (i + 1..kinds.len()).find(|&j| {
                match kinds[j] {
                    T::LParen => depth += 1,
                    T::RParen => depth -= 1,
                    _ => {}
                }
                depth == 0
            });
            stack && close.is_none_or(|j| !ends_operand(kinds.get(j + 1)))
        }
        _ => false,
    })
}

/// Whether lowered loop-body code (`body`) may reach a `continue` or its end with `stack_obj`
/// still moved away from its entry value: some line may write it and no later `restore` line
/// (`stack_obj := __self_obj;`) puts it back on that path. The paths follow the control flow:
/// a `break` leaves its own loop (a nested one's exit carries its state; the body's own `break`
/// skips the exhaustion block), and a `return` leaves the tree. A field read through another
/// object moves the stack object and restores it right after, so it leaves nothing displaced.
/// A subroutine runs in its own frame with its own stack object, so a call writes none here.
/// Code that does not parse may.
///
/// Precision matters both ways: a missing restore leaves the exhaustion block on the wrong
/// object, and a needless one replaces the iterator's false result, which the block may read.
pub(crate) fn body_may_leave_stack_obj_moved(body: &str, restore: &str) -> bool {
    /// A path's state: `None` when no path reaches the point, else whether `stack_obj` may be
    /// moved there.
    type State = Option<bool>;
    fn join(a: State, b: State) -> State {
        match (a, b) {
            (None, x) | (x, None) => x,
            (Some(a), Some(b)) => Some(a || b),
        }
    }
    /// The states the innermost loop's `break`s and `continue`s carry.
    #[derive(Default)]
    struct Exits {
        brk: State,
        cont: State,
    }
    /// The state after falling off the end of `nodes`, entered in state `st`.
    fn flow(nodes: &[Stmt], mut st: State, restore: &str, exits: &mut Exits) -> State {
        for n in nodes {
            match n {
                Stmt::Inst(l) if l.trim() == restore => st = st.map(|_| false),
                Stmt::Inst(l) => match classify(l) {
                    InstKind::Continue => exits.cont = join(exits.cont, st.take()),
                    InstKind::Break => exits.brk = join(exits.brk, st.take()),
                    InstKind::Return => st = None,
                    _ => st = st.map(|m| m || may_write_stack_obj(l)),
                },
                Stmt::If { arms, else_body } => {
                    let mut out = None;
                    for (cond, body) in arms {
                        st = st.map(|m| m || may_write_stack_obj(cond));
                        out = join(out, flow(body, st, restore, exits));
                    }
                    out = join(
                        out,
                        match else_body {
                            Some(body) => flow(body, st, restore, exits),
                            None => st,
                        },
                    );
                    st = out;
                }
                Stmt::While { cond, body } => {
                    // Iterate to the fixed point: the head joins the entry state with every
                    // path back to it (the body's end and its `continue`s).
                    let mut head = st;
                    let mut inner = Exits::default();
                    loop {
                        let tested =
                            head.map(|m| m || cond.as_deref().is_some_and(may_write_stack_obj));
                        let back = flow(body, tested, restore, &mut inner);
                        let next = join(join(head, back), inner.cont);
                        if next == head {
                            // `while true` leaves only through a `break`.
                            let falls = if cond.is_some() { tested } else { None };
                            st = join(falls, inner.brk);
                            break;
                        }
                        head = next;
                    }
                }
            }
        }
        st
    }
    let Some(nodes) = parse_body(body) else {
        return true;
    };
    let mut exits = Exits::default();
    let end = flow(&nodes, Some(false), restore, &mut exits);
    end == Some(true) || exits.cont == Some(true)
}

pub(crate) fn writes_stack_obj_anywhere(nodes: &[Stmt]) -> bool {
    nodes.iter().any(|n| match n {
        Stmt::Inst(l) => may_write_stack_obj(l),
        Stmt::If { arms, else_body } => {
            arms.iter()
                .any(|(c, b)| may_write_stack_obj(c) || writes_stack_obj_anywhere(b))
                || else_body.as_deref().is_some_and(writes_stack_obj_anywhere)
        }
        Stmt::While { cond, body } => {
            cond.as_deref().is_some_and(may_write_stack_obj) || writes_stack_obj_anywhere(body)
        }
    })
}

/// Primitive arguments `.air` parses as a local slot rather than a value.
/// (`value:` is Relationship's, which the primitive may write.)
pub(crate) const LOCAL_SLOT_ARGS: &[&str] =
    &["icon", "skin", "age", "sex", "local", "dest", "value"];

/// Whether every mention of a `map` key in `text` is a plain value read: the text runs no
/// primitive, or each mention is a labelled primitive argument (`label: name`) whose label
/// takes a value.
pub(crate) fn value_operands_only(text: &str, map: &BTreeMap<String, String>) -> bool {
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
