//! The MIR body: a structured control-flow tree of statements (`Stmt`): simple instructions, `if` /
//! `else if` / `else` chains and `while` loops, plus block walkers and name-mention queries.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stmt {
    /// One simple instruction: a single `;`-terminated `.air` statement, typed by
    /// [`classify`] as an [`InstKind`].
    Inst(String),
    If {
        /// `(condition, body)` for the `if` and each `else if`.
        arms: Vec<(String, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
    },
    /// `while cond { }`, or `while true { }` (no condition node) when `cond` is `None`.
    While {
        cond: Option<String>,
        body: Vec<Stmt>,
    },
}

/// Whether running `next` first cannot observe the previous node's result: assignments and
/// `let` initializers are expression nodes, `return` sets the result, and so do `if`/`while`
/// tests that are expression compares (no primitive call).
pub(crate) fn is_decl(n: &Stmt) -> bool {
    matches!(n, Stmt::Inst(l) if matches!(classify(l), InstKind::Let { init: None, .. }))
}

// ---------------------------------------------------------------------------
// Passes
// ---------------------------------------------------------------------------

pub(crate) fn for_each_block(nodes: &mut Vec<Stmt>, f: &mut impl FnMut(&mut Vec<Stmt>)) {
    for n in nodes.iter_mut() {
        match n {
            Stmt::Inst(_) => {}
            Stmt::If { arms, else_body } => {
                for (_, body) in arms.iter_mut() {
                    for_each_block(body, f);
                }
                if let Some(body) = else_body {
                    for_each_block(body, f);
                }
            }
            Stmt::While { body, .. } => for_each_block(body, f),
        }
    }
    f(nodes);
}

pub(crate) fn collect_lets(nodes: &[Stmt], out: &mut Vec<String>) {
    for n in nodes {
        match n {
            Stmt::Inst(l) => {
                if let InstKind::Let { name, .. } = classify(l) {
                    out.push(name.to_owned());
                }
            }
            Stmt::If { arms, else_body } => {
                for (_, body) in arms {
                    collect_lets(body, out);
                }
                if let Some(body) = else_body {
                    collect_lets(body, out);
                }
            }
            Stmt::While { body, .. } => collect_lets(body, out),
        }
    }
}

/// Whether any line or condition in `nodes` names `name`.
pub(crate) fn mentions(nodes: &[Stmt], name: &str) -> bool {
    let named = |text: &str| text.contains(name) && idents(text).contains(&name);
    nodes.iter().any(|n| match n {
        Stmt::Inst(l) => named(l),
        Stmt::If { arms, else_body } => {
            arms.iter().any(|(c, b)| named(c) || mentions(b, name))
                || else_body.as_deref().is_some_and(|b| mentions(b, name))
        }
        Stmt::While { cond, body } => cond.as_deref().is_some_and(named) || mentions(body, name),
    })
}

/// Every mention of each identifier: `(reads, writes-only lines)`.
pub(crate) fn mention_counts<'a>(
    nodes: &'a [Stmt],
    counts: &mut BTreeMap<&'a str, (usize, usize)>,
) {
    let add_text = |text: &'a str, counts: &mut BTreeMap<&'a str, (usize, usize)>| {
        for id in idents(text) {
            counts.entry(id).or_default().0 += 1;
        }
    };
    for n in nodes {
        match n {
            Stmt::Inst(l) => match classify(l) {
                InstKind::Let { name, init } => {
                    counts.entry(name).or_default().1 += 1;
                    if let Some(init) = init {
                        add_text(init, counts);
                    }
                }
                InstKind::Assign { lhs, op: ":=", rhs } if is_ident(lhs) => {
                    counts.entry(lhs).or_default().1 += 1;
                    add_text(rhs, counts);
                }
                _ => add_text(l, counts),
            },
            Stmt::If { arms, else_body } => {
                for (cond, body) in arms {
                    add_text(cond, counts);
                    mention_counts(body, counts);
                }
                if let Some(body) = else_body {
                    mention_counts(body, counts);
                }
            }
            Stmt::While { cond, body } => {
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
pub(crate) fn has_loop_exit(nodes: &[Stmt]) -> bool {
    nodes.iter().any(|n| match n {
        Stmt::Inst(l) => matches!(classify(l), InstKind::Break | InstKind::Continue),
        Stmt::If { arms, else_body } => {
            arms.iter().any(|(_, b)| has_loop_exit(b))
                || else_body.as_deref().is_some_and(has_loop_exit)
        }
        Stmt::While { .. } => false,
    })
}
