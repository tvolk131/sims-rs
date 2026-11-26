//! Compile-time branch specialization.
//!
//! `const NAME = match s { P => v, .., _ => break };` (or `const (A, B) = if c { (a, b) } else
//! { .. };`) binds names to compile-time operands chosen by a runtime test. The statements after
//! the binding that depend on it (its *region*) are duplicated into each arm with the arm's
//! values substituted, so every specialized copy names its own constants: no local holds the
//! choice, and an operand that must be a compile-time constant (a GUID, a string index, an
//! animation) can still vary by branch. This is how SimAntics trees written by hand spell such
//! code (one test ladder, the dependent nodes copied under each test).
//!
//! The parser expands bindings when it closes a block ([`expand_block`]), so later passes see
//! ordinary `match` / `if` statements. A `match` / `if` expression used directly as a builtin's
//! operand is split the same way by the lowering ([`split_inline_operand`]): its region is that
//! one statement.
//!
//! Soundness rules, checked here:
//! - an arm yields compile-time operands (literals, `const` names, enum paths, tuning fields) or
//!   diverges (`break`, `continue`, `return ..`, or a block ending in one), so control never
//!   reaches the region without the names bound;
//! - the `match` has a `_` arm and the `if` an `else`, so every path binds or diverges;
//! - the region is the statements up to the last one naming a bound name, extended to cover every
//!   later use of a `let` it declares (each copy declares its own);
//! - the region may not redeclare a bound name, write a name an arm yields, contain a `defer`
//!   (which would run at the end of the arm instead of the block), or name a bound name inside
//!   `raw { }`.

use crate::error::{Error, Result, Span};
use crate::v2::ast::{CallArg, ForEachIter, MatchArm, MatchPattern, V2Expr, V2Stmt, V2UnaryOp};

/// A parsed `const` binding, expanded against the statements after it by [`expand_block`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConstBinding {
    pub span: Span,
    /// One name, or the names of a `(A, B, ..)` pattern.
    pub names: Vec<(Span, String)>,
    pub tuple: bool,
    pub selector: Selector,
}

/// How a binding chooses its arm.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Selector {
    /// `match scrutinee { P => arm, .. }`.
    Match {
        span: Span,
        scrutinee: V2Expr,
        arms: Vec<(Span, MatchPattern, Arm)>,
    },
    /// `if c { arm } else if c2 { arm } .. else { arm }`.
    If {
        arms: Vec<(Span, V2Expr, Arm)>,
        else_arm: Box<Arm>,
    },
}

/// One arm: the values it binds, or statements that leave the block.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Arm {
    Values(Span, Vec<V2Expr>),
    Diverge(Vec<V2Stmt>),
}

/// A statement of a block as the parser collects it, before bindings are expanded.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BlockItem {
    Stmt(V2Stmt),
    Bind(ConstBinding),
}

/// Expands every binding in `items` (last first, so an earlier binding's region holds the later
/// ones already expanded).
pub(crate) fn expand_block(items: Vec<BlockItem>) -> Result<Vec<V2Stmt>> {
    let mut out: Vec<V2Stmt> = Vec::new();
    for item in items.into_iter().rev() {
        match item {
            BlockItem::Stmt(s) => out.push(s),
            BlockItem::Bind(b) => {
                out.reverse();
                let expanded = expand(b, out)?;
                out = expanded;
                out.reverse();
            }
        }
    }
    out.reverse();
    Ok(out)
}

/// Whether the statements of a diverging arm never fall through.
pub(crate) fn diverges(stmts: &[V2Stmt]) -> bool {
    matches!(
        stmts.last(),
        Some(
            V2Stmt::Break { .. }
                | V2Stmt::Continue { .. }
                | V2Stmt::Return { .. }
                | V2Stmt::ReturnError { .. }
        )
    )
}

/// `binding` followed by `rest`: the selector statement holding the region in each arm,
/// followed by the statements after the region.
fn expand(binding: ConstBinding, mut rest: Vec<V2Stmt>) -> Result<Vec<V2Stmt>> {
    let names: Vec<String> = binding.names.iter().map(|(_, n)| n.clone()).collect();
    for (i, (span, n)) in binding.names.iter().enumerate() {
        if binding.names[..i].iter().any(|(_, m)| m == n) {
            return Err(Error::at(*span, format!("'{n}' is bound twice")));
        }
    }
    check_arms(&binding, &names)?;
    let end = region_end(&rest, &names);
    let tail = rest.split_off(end);
    let region = rest;
    check_region(&region, &binding, &names)?;
    let arm_body = |arm: Arm| -> Result<Vec<V2Stmt>> {
        match arm {
            Arm::Diverge(stmts) => Ok(stmts),
            Arm::Values(_, values) => {
                let pairs: Vec<(String, V2Expr)> = names.iter().cloned().zip(values).collect();
                let mut body = region.clone();
                subst_stmts(&mut body, &pairs)?;
                Ok(body)
            }
        }
    };
    let stmt = match binding.selector {
        Selector::Match {
            span,
            scrutinee,
            arms,
        } => V2Stmt::Match {
            span,
            scrutinee,
            arms: arms
                .into_iter()
                .map(|(span, pattern, arm)| {
                    Ok(MatchArm {
                        span,
                        pattern,
                        body: arm_body(arm)?,
                    })
                })
                .collect::<Result<_>>()?,
        },
        Selector::If { arms, else_arm } => {
            let mut else_body = arm_body(*else_arm)?;
            for (span, cond, arm) in arms.into_iter().rev() {
                else_body = vec![V2Stmt::If {
                    span,
                    cond,
                    then_body: arm_body(arm)?,
                    else_body,
                }];
            }
            else_body
                .pop()
                .expect("an 'if' binding has at least one condition")
        }
    };
    let mut out = vec![stmt];
    out.extend(tail);
    Ok(out)
}

/// Every arm binds as many values as there are names (each a compile-time operand) or
/// diverges; a `match` ends in `_`.
fn check_arms(binding: &ConstBinding, names: &[String]) -> Result<()> {
    let check = |arm: &Arm| -> Result<()> {
        match arm {
            Arm::Diverge(stmts) if diverges(stmts) => Ok(()),
            Arm::Diverge(stmts) => Err(Error::at(
                stmts.first().map_or(binding.span, stmt_span),
                "a const binding arm that binds no value must end in 'break', 'continue', or 'return'",
            )),
            Arm::Values(span, values) => {
                if values.len() != names.len() {
                    return Err(Error::at(
                        *span,
                        format!(
                            "this arm yields {} value(s) but the binding names {}",
                            values.len(),
                            names.len()
                        ),
                    ));
                }
                for v in values {
                    if matches!(v, V2Expr::Ident(_, n) if n == "stack_obj") {
                        return Err(Error::at(
                            v.span(),
                            "a const binding cannot yield 'stack_obj', which primitives change; copy it into a local first",
                        ));
                    }
                    if !is_const_operand(v) {
                        return Err(Error::at(
                            v.span(),
                            "a const binding arm yields compile-time operands: a literal, a 'const' or other name, an enum path, or a tuning field ('Tuning.name')",
                        ));
                    }
                }
                Ok(())
            }
        }
    };
    match &binding.selector {
        Selector::Match { span, arms, .. } => {
            for (_, _, arm) in arms {
                check(arm)?;
            }
            if !matches!(arms.last(), Some((_, MatchPattern::Wildcard(_), _))) {
                return Err(Error::at(
                    *span,
                    "a const binding 'match' must end with a '_' arm (every path binds the names or leaves the block)",
                ));
            }
        }
        Selector::If { arms, else_arm } => {
            for (_, _, arm) in arms {
                check(arm)?;
            }
            check(else_arm)?;
        }
    }
    Ok(())
}

/// A value that means the same thing wherever it is substituted: a literal, a name (checked
/// against writes in the region), an enum path, or a field of a capitalized name (a tuning
/// table, `Tuning.name`).
fn is_const_operand(e: &V2Expr) -> bool {
    match e {
        V2Expr::Int(..) | V2Expr::Bool(..) | V2Expr::Str(..) | V2Expr::Ident(..) => true,
        V2Expr::Path(..) => true,
        V2Expr::Unary {
            op: V2UnaryOp::Neg,
            inner,
            ..
        } => matches!(inner.as_ref(), V2Expr::Int(..)),
        V2Expr::Field { base, .. } => {
            matches!(base.as_ref(), V2Expr::Ident(_, n) if n.starts_with(|c: char| c.is_ascii_uppercase()))
        }
        _ => false,
    }
}

/// The names an arm yields (a bare name, or the table of `Table.entry`), with their spans.
pub(crate) fn yielded_idents_at(binding: &ConstBinding) -> Vec<(Span, String)> {
    let mut out = Vec::new();
    let mut add = |arm: &Arm| {
        if let Arm::Values(_, values) = arm {
            for v in values {
                let root = match v {
                    V2Expr::Field { base, .. } => base.as_ref(),
                    other => other,
                };
                if let V2Expr::Ident(span, n) = root {
                    out.push((*span, n.clone()));
                }
            }
        }
    };
    match &binding.selector {
        Selector::Match { arms, .. } => arms.iter().for_each(|(_, _, a)| add(a)),
        Selector::If { arms, else_arm } => {
            arms.iter().for_each(|(_, _, a)| add(a));
            add(else_arm);
        }
    }
    out
}

/// One past the last statement of the region: the last statement naming a bound name, extended
/// until no statement after it names a `let` the region declares.
fn region_end(rest: &[V2Stmt], names: &[String]) -> usize {
    let mut watched: Vec<String> = names.to_vec();
    let mut end = 0;
    loop {
        let last = rest
            .iter()
            .rposition(|s| watched.iter().any(|n| stmt_mentions(s, n)))
            .map_or(0, |i| i + 1);
        let new_end = last.max(end);
        let mut declared = Vec::new();
        for s in &rest[..new_end] {
            if let Some(n) = declared_name(s) {
                declared.push(n.to_owned());
            }
        }
        let grew = new_end != end || declared.iter().any(|d| !watched.contains(d));
        end = new_end;
        for d in declared {
            if !watched.contains(&d) {
                watched.push(d);
            }
        }
        if !grew {
            return end;
        }
    }
}

fn declared_name(s: &V2Stmt) -> Option<&str> {
    match s {
        V2Stmt::Let { name, .. }
        | V2Stmt::LetSome { name, .. }
        | V2Stmt::LetUninit { name, .. } => Some(name),
        _ => None,
    }
}

fn check_region(region: &[V2Stmt], binding: &ConstBinding, names: &[String]) -> Result<()> {
    for s in region {
        if let V2Stmt::Defer { span, .. } = s {
            return Err(Error::at(
                *span,
                "a 'defer' cannot be in a const binding's region (each arm would run it at its own end)",
            ));
        }
    }
    if let Some((span, why)) = find_uncopyable_raw(region) {
        return Err(Error::at(
            span,
            format!("a 'raw {{ }}' block that {why} cannot be in a const binding's region"),
        ));
    }
    for n in names {
        if let Some(span) = find_binding_of(region, n) {
            return Err(Error::at(
                span,
                format!(
                    "'{n}' is bound by a const binding; it cannot be redeclared in the statements that use it"
                ),
            ));
        }
        if let Some(span) = find_write_through(region, n) {
            return Err(Error::at(
                span,
                format!("'{n}' is bound by a const binding, so it cannot be assigned or borrowed"),
            ));
        }
        if let Some(span) = find_raw_mention(region, n) {
            return Err(Error::at(
                span,
                format!("'{n}' is bound by a const binding, which 'raw {{ }}' cannot name"),
            ));
        }
    }
    // A yielded name is never a variable (the parser refuses that), so nothing can write it,
    // but a declaration in the region would capture the substituted name.
    for (_, n) in yielded_idents_at(binding) {
        if let Some(span) = find_binding_of(region, &n) {
            return Err(Error::at(
                span,
                format!(
                    "a const binding yields '{n}', so the statements that use it cannot declare '{n}'"
                ),
            ));
        }
    }
    Ok(())
}

pub(super) fn stmt_span(s: &V2Stmt) -> Span {
    match s {
        V2Stmt::Let { span, .. }
        | V2Stmt::LetSome { span, .. }
        | V2Stmt::IfLetSome { span, .. }
        | V2Stmt::Assign { span, .. }
        | V2Stmt::WithLock { span, .. }
        | V2Stmt::Defer { span, .. }
        | V2Stmt::Repeat { span, .. }
        | V2Stmt::While { span, .. }
        | V2Stmt::ForEach { span, .. }
        | V2Stmt::If { span, .. }
        | V2Stmt::Match { span, .. }
        | V2Stmt::Break { span }
        | V2Stmt::Continue { span }
        | V2Stmt::Return { span, .. }
        | V2Stmt::ReturnError { span, .. }
        | V2Stmt::Expr { span, .. }
        | V2Stmt::RawAir { span, .. }
        | V2Stmt::LetUninit { span, .. }
        | V2Stmt::Animate { span, .. } => *span,
    }
}

// ---------------------------------------------------------------------------------------------
// Walkers
// ---------------------------------------------------------------------------------------------

/// The expressions a statement evaluates itself (not those of nested statements).
pub(super) fn stmt_exprs(s: &V2Stmt) -> Vec<&V2Expr> {
    match s {
        V2Stmt::Let { init, .. }
        | V2Stmt::LetSome { init, .. }
        | V2Stmt::IfLetSome { init, .. } => vec![init],
        V2Stmt::Assign { target, value, .. } => vec![target, value],
        V2Stmt::WithLock { lock_expr, .. } => vec![lock_expr],
        V2Stmt::Repeat { count, .. } => vec![count],
        V2Stmt::While { cond, .. } | V2Stmt::If { cond, .. } => vec![cond],
        V2Stmt::ForEach { iter, .. } => match iter {
            ForEachIter::ObjectsByGuid(e) => vec![e],
            ForEachIter::Range { start, end, .. } => vec![start, end],
            ForEachIter::ObjectsOf { .. }
            | ForEachIter::SelfParts
            | ForEachIter::LotSims
            | ForEachIter::LotObjects => vec![],
        },
        V2Stmt::Match { scrutinee, .. } => vec![scrutinee],
        V2Stmt::Return { value, .. } => value.iter().collect(),
        V2Stmt::Expr { expr, .. } => vec![expr],
        V2Stmt::Animate { args, .. } => args.iter().map(|a| &a.value).collect(),
        V2Stmt::Defer { .. }
        | V2Stmt::Break { .. }
        | V2Stmt::Continue { .. }
        | V2Stmt::ReturnError { .. }
        | V2Stmt::RawAir { .. }
        | V2Stmt::LetUninit { .. } => vec![],
    }
}

/// The statement lists nested in a statement.
pub(super) fn stmt_bodies(s: &V2Stmt) -> Vec<&[V2Stmt]> {
    match s {
        V2Stmt::LetSome { else_body, .. } => vec![else_body],
        V2Stmt::ForEach {
            body, else_body, ..
        } => std::iter::once(body.as_slice())
            .chain(else_body.as_deref())
            .collect(),
        V2Stmt::WithLock { body, .. }
        | V2Stmt::Defer { body, .. }
        | V2Stmt::Repeat { body, .. }
        | V2Stmt::While { body, .. }
        | V2Stmt::Animate { body, .. } => vec![body],
        V2Stmt::If {
            then_body,
            else_body,
            ..
        }
        | V2Stmt::IfLetSome {
            then_body,
            else_body,
            ..
        } => vec![then_body, else_body],
        V2Stmt::Match { arms, .. } => arms.iter().map(|a| a.body.as_slice()).collect(),
        V2Stmt::Let { init, .. } => init_blocks(init),
        _ => vec![],
    }
}

/// The bodies of the diverging `{ ..; return ..; }` branches of a `let` initializer
/// (`let x = if c { v } else { return false; };`). The parser allows such a branch only as an
/// arm of the initializer's `if` / `match` chain, so this follows only that chain.
pub(super) fn init_blocks(init: &V2Expr) -> Vec<&[V2Stmt]> {
    fn walk<'e>(e: &'e V2Expr, out: &mut Vec<&'e [V2Stmt]>) {
        match e {
            V2Expr::Block { body, .. } => out.push(body),
            V2Expr::If {
                then_value,
                else_value,
                ..
            } => {
                walk(then_value, out);
                walk(else_value, out);
            }
            V2Expr::Match { arms, .. } => arms.iter().for_each(|a| walk(&a.value, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(init, &mut out);
    out
}

/// Whether `pred` holds for any statement of `stmts`, nested bodies included.
pub(super) fn stmts_any(stmts: &[V2Stmt], pred: &dyn Fn(&V2Stmt) -> bool) -> bool {
    stmts
        .iter()
        .any(|s| pred(s) || stmt_bodies(s).into_iter().any(|b| stmts_any(b, pred)))
}

fn stmt_mentions(s: &V2Stmt, name: &str) -> bool {
    if let V2Stmt::RawAir { code, .. } = s {
        return raw_names(code, name);
    }
    if let V2Stmt::Match { arms, .. } = s
        && arms.iter().any(|a| pattern_names(&a.pattern, name))
    {
        return true;
    }
    stmt_exprs(s).into_iter().any(|e| expr_mentions(e, name))
        || stmt_bodies(s)
            .into_iter()
            .any(|b| b.iter().any(|s| stmt_mentions(s, name)))
}

fn pattern_names(p: &MatchPattern, name: &str) -> bool {
    match p {
        MatchPattern::Ident(_, n) => n == name,
        MatchPattern::Or(_, alts) => alts.iter().any(|a| pattern_names(a, name)),
        _ => false,
    }
}

/// Whether `e` reads `name` (a callee name is not a read).
fn expr_mentions(e: &V2Expr, name: &str) -> bool {
    let mut found = false;
    visit_expr(e, &mut |x| {
        if let V2Expr::Ident(_, n) = x
            && n == name
        {
            found = true;
        }
        if let V2Expr::Match { arms, .. } = x
            && arms.iter().any(|a| pattern_names(&a.pattern, name))
        {
            found = true;
        }
    });
    found
}

/// Calls `f` on `e` and every expression inside it, except a call's callee name. The
/// statements of a diverging branch (`V2Expr::Block`) are not visited: they are a nested body
/// (see [`stmt_bodies`]).
pub(super) fn visit_expr(e: &V2Expr, f: &mut dyn FnMut(&V2Expr)) {
    f(e);
    match e {
        V2Expr::Block { .. }
        | V2Expr::Int(..)
        | V2Expr::Bool(..)
        | V2Expr::Str(..)
        | V2Expr::Ident(..)
        | V2Expr::Path(..) => {}
        V2Expr::Tuple(_, items) => items.iter().for_each(|i| visit_expr(i, f)),
        V2Expr::Field { base, .. } => visit_expr(base, f),
        V2Expr::Index { base, index, .. } => {
            visit_expr(base, f);
            visit_expr(index, f);
        }
        V2Expr::Deref { inner, .. }
        | V2Expr::MutRef { inner, .. }
        | V2Expr::Unary { inner, .. }
        | V2Expr::Try { inner, .. } => visit_expr(inner, f),
        V2Expr::Binary { lhs, rhs, .. } => {
            visit_expr(lhs, f);
            visit_expr(rhs, f);
        }
        V2Expr::Call { callee, args, .. } => {
            if !matches!(callee.as_ref(), V2Expr::Ident(..)) {
                visit_expr(callee, f);
            }
            args.iter().for_each(|a| visit_expr(&a.value, f));
        }
        V2Expr::MethodCall { receiver, args, .. } => {
            visit_expr(receiver, f);
            args.iter().for_each(|a| visit_expr(&a.value, f));
        }
        V2Expr::Match {
            scrutinee, arms, ..
        } => {
            visit_expr(scrutinee, f);
            arms.iter().for_each(|a| visit_expr(&a.value, f));
        }
        V2Expr::If {
            cond,
            then_value,
            else_value,
            ..
        } => {
            visit_expr(cond, f);
            visit_expr(then_value, f);
            visit_expr(else_value, f);
        }
    }
}

/// Whether `raw { }` code names the identifier `name`, read from its `.air` tokens (so strings
/// and comments do not count). Code that does not lex is assumed to.
fn raw_names(code: &str, name: &str) -> bool {
    crate::lexer::tokenize(code).map_or(true, |tokens| {
        tokens
            .iter()
            .any(|t| matches!(&t.kind, crate::lexer::TokenKind::Ident(n) if n == name))
    })
}

fn find_raw_mention(stmts: &[V2Stmt], name: &str) -> Option<Span> {
    for s in stmts {
        if let V2Stmt::RawAir { span, code } = s
            && raw_names(code, name)
        {
            return Some(*span);
        }
        for b in stmt_bodies(s) {
            if let Some(span) = find_raw_mention(b, name) {
                return Some(span);
            }
        }
    }
    None
}

/// A declaration of `name` anywhere in `stmts` (a `let`, a loop binding, an animation event).
fn find_binding_of(stmts: &[V2Stmt], name: &str) -> Option<Span> {
    for s in stmts {
        let binds = match s {
            V2Stmt::ForEach { binding, .. } => binding == name,
            V2Stmt::Animate { event, .. } => event.as_deref() == Some(name),
            V2Stmt::IfLetSome { name: n, .. } => n == name,
            other => declared_name(other) == Some(name),
        };
        if binds {
            return Some(stmt_span(s));
        }
        for b in stmt_bodies(s) {
            if let Some(span) = find_binding_of(b, name) {
                return Some(span);
            }
        }
    }
    None
}

/// The root variable an assignment target or `&mut` names (`x`, `x.f`, `x[i]`, `*x`).
fn root_ident(e: &V2Expr) -> Option<&str> {
    match e {
        V2Expr::Ident(_, n) => Some(n),
        V2Expr::Field { base, .. } | V2Expr::Index { base, .. } => root_ident(base),
        V2Expr::Deref { inner, .. } => root_ident(inner),
        _ => None,
    }
}

/// A statement that may change `name`: a declaration of it, an assignment to it, `&mut name`,
/// or `name` passed bare to any call (builtins write out-operands this way).
fn find_write(stmts: &[V2Stmt], name: &str) -> Option<Span> {
    if let Some(span) = find_binding_of(stmts, name) {
        return Some(span);
    }
    for s in stmts {
        let assigns =
            matches!(s, V2Stmt::Assign { target, .. } if root_ident(target) == Some(name));
        let animate_arg = matches!(s, V2Stmt::Animate { args, .. } if args_bare(args, name));
        let mut passes = false;
        for e in stmt_exprs(s) {
            visit_expr(e, &mut |x| match x {
                V2Expr::MutRef { inner, .. } if root_ident(inner) == Some(name) => passes = true,
                V2Expr::Call { args, .. } | V2Expr::MethodCall { args, .. }
                    if args_bare(args, name) =>
                {
                    passes = true;
                }
                _ => {}
            });
        }
        if assigns || animate_arg || passes {
            return Some(stmt_span(s));
        }
        for b in stmt_bodies(s) {
            if let Some(span) = find_write(b, name) {
                return Some(span);
            }
        }
    }
    None
}

/// A statement that assigns the bound name `name` itself (`X = v`, `X += v`) or borrows it
/// (`&mut X`): substitution would turn it into a write of the yielded operand.
fn find_write_through(stmts: &[V2Stmt], name: &str) -> Option<Span> {
    for s in stmts {
        let assigns = matches!(s, V2Stmt::Assign { target: V2Expr::Ident(_, n), .. } if n == name);
        let mut borrows = false;
        for e in stmt_exprs(s) {
            visit_expr(e, &mut |x| {
                if matches!(x, V2Expr::MutRef { inner, .. } if matches!(inner.as_ref(), V2Expr::Ident(_, n) if n == name))
                {
                    borrows = true;
                }
            });
        }
        if assigns || borrows {
            return Some(stmt_span(s));
        }
        for b in stmt_bodies(s) {
            if let Some(span) = find_write_through(b, name) {
                return Some(span);
            }
        }
    }
    None
}

/// A `raw { }` block in `stmts` whose code cannot be copied into each arm, with the reason:
/// a label or a local would be defined twice, and a numbered branch target would jump into the
/// first copy.
fn find_uncopyable_raw(stmts: &[V2Stmt]) -> Option<(Span, &'static str)> {
    for s in stmts {
        if let V2Stmt::RawAir { span, code } = s {
            let facts = super::raw_facts::raw_facts(code, None);
            let why = if facts.defines_label {
                Some("defines a label (each arm would define it again)")
            } else if facts.declares_local {
                Some("declares a local (each arm would declare it again)")
            } else if facts.positional_target {
                Some("branches to a numbered node (each arm's copy would jump into the first)")
            } else {
                None
            };
            if let Some(why) = why {
                return Some((*span, why));
            }
        }
        for b in stmt_bodies(s) {
            if let Some(found) = find_uncopyable_raw(b) {
                return Some(found);
            }
        }
    }
    None
}

fn args_bare(args: &[CallArg], name: &str) -> bool {
    args.iter()
        .any(|a| matches!(&a.value, V2Expr::Ident(_, n) if n == name))
}

// ---------------------------------------------------------------------------------------------
// Substitution
// ---------------------------------------------------------------------------------------------

fn subst_stmts(stmts: &mut [V2Stmt], pairs: &[(String, V2Expr)]) -> Result<()> {
    for s in stmts {
        subst_stmt(s, pairs)?;
    }
    Ok(())
}

fn subst_stmt(s: &mut V2Stmt, pairs: &[(String, V2Expr)]) -> Result<()> {
    match s {
        V2Stmt::Let { init, .. } => subst_expr(init, pairs)?,
        V2Stmt::LetSome {
            init, else_body, ..
        } => {
            subst_expr(init, pairs)?;
            subst_stmts(else_body, pairs)?;
        }
        V2Stmt::Assign { target, value, .. } => {
            subst_expr(target, pairs)?;
            subst_expr(value, pairs)?;
        }
        V2Stmt::WithLock {
            lock_expr, body, ..
        } => {
            subst_expr(lock_expr, pairs)?;
            subst_stmts(body, pairs)?;
        }
        V2Stmt::Defer { body, .. } => subst_stmts(body, pairs)?,
        V2Stmt::Repeat { count, body, .. } => {
            subst_expr(count, pairs)?;
            subst_stmts(body, pairs)?;
        }
        V2Stmt::While { cond, body, .. } => {
            subst_expr(cond, pairs)?;
            subst_stmts(body, pairs)?;
        }
        V2Stmt::ForEach {
            iter,
            body,
            else_body,
            ..
        } => {
            match iter {
                ForEachIter::ObjectsByGuid(e) => subst_expr(e, pairs)?,
                ForEachIter::Range { start, end, .. } => {
                    subst_expr(start, pairs)?;
                    subst_expr(end, pairs)?;
                }
                ForEachIter::ObjectsOf { .. }
                | ForEachIter::SelfParts
                | ForEachIter::LotSims
                | ForEachIter::LotObjects => {}
            }
            subst_stmts(body, pairs)?;
            if let Some(else_body) = else_body {
                subst_stmts(else_body, pairs)?;
            }
        }
        V2Stmt::If {
            cond: init,
            then_body,
            else_body,
            ..
        }
        | V2Stmt::IfLetSome {
            init,
            then_body,
            else_body,
            ..
        } => {
            subst_expr(init, pairs)?;
            subst_stmts(then_body, pairs)?;
            subst_stmts(else_body, pairs)?;
        }
        V2Stmt::Match {
            scrutinee, arms, ..
        } => {
            subst_expr(scrutinee, pairs)?;
            for arm in arms {
                subst_pattern(&mut arm.pattern, pairs)?;
                subst_stmts(&mut arm.body, pairs)?;
            }
        }
        V2Stmt::Return { value, .. } => {
            if let Some(v) = value {
                subst_expr(v, pairs)?;
            }
        }
        V2Stmt::Expr { expr, .. } => subst_expr(expr, pairs)?,
        V2Stmt::Animate { args, body, .. } => {
            for a in args {
                subst_expr(&mut a.value, pairs)?;
            }
            subst_stmts(body, pairs)?;
        }
        V2Stmt::Break { .. }
        | V2Stmt::Continue { .. }
        | V2Stmt::ReturnError { .. }
        | V2Stmt::RawAir { .. }
        | V2Stmt::LetUninit { .. } => {}
    }
    Ok(())
}

/// A pattern naming a bound name takes its value: a number, a name, or a path.
fn subst_pattern(p: &mut MatchPattern, pairs: &[(String, V2Expr)]) -> Result<()> {
    match p {
        MatchPattern::Ident(span, n) => {
            let Some((_, v)) = pairs.iter().find(|(k, _)| k == n) else {
                return Ok(());
            };
            *p = match v {
                V2Expr::Int(_, i) => MatchPattern::Int(*span, pattern_int(*span, *i)?),
                V2Expr::Unary {
                    op: V2UnaryOp::Neg,
                    inner,
                    ..
                } => match inner.as_ref() {
                    V2Expr::Int(_, i) => MatchPattern::Int(*span, pattern_int(*span, -*i)?),
                    _ => return Err(not_a_pattern(*span, n)),
                },
                V2Expr::Ident(_, m) => MatchPattern::Ident(*span, m.clone()),
                V2Expr::Path(_, segs) => MatchPattern::Path(*span, segs.clone()),
                _ => return Err(not_a_pattern(*span, n)),
            };
        }
        MatchPattern::Or(_, alts) => {
            for a in alts {
                subst_pattern(a, pairs)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn pattern_int(span: Span, v: i64) -> Result<i16> {
    i16::try_from(v).map_err(|_| Error::at(span, format!("{v} is out of i16 range for a pattern")))
}

fn not_a_pattern(span: Span, name: &str) -> Error {
    Error::at(
        span,
        format!("'{name}' is bound to a value that cannot be a match pattern here"),
    )
}

fn subst_expr(e: &mut V2Expr, pairs: &[(String, V2Expr)]) -> Result<()> {
    if let V2Expr::Ident(_, n) = e
        && let Some((_, v)) = pairs.iter().find(|(k, _)| k == n)
    {
        *e = v.clone();
        return Ok(());
    }
    match e {
        V2Expr::Int(..)
        | V2Expr::Bool(..)
        | V2Expr::Str(..)
        | V2Expr::Ident(..)
        | V2Expr::Path(..) => {}
        V2Expr::Tuple(_, items) => {
            for i in items {
                subst_expr(i, pairs)?;
            }
        }
        V2Expr::Field { base, .. } => subst_expr(base, pairs)?,
        V2Expr::Index { base, index, .. } => {
            subst_expr(base, pairs)?;
            subst_expr(index, pairs)?;
        }
        V2Expr::Deref { inner, .. }
        | V2Expr::MutRef { inner, .. }
        | V2Expr::Unary { inner, .. }
        | V2Expr::Try { inner, .. } => subst_expr(inner, pairs)?,
        V2Expr::Binary { lhs, rhs, .. } => {
            subst_expr(lhs, pairs)?;
            subst_expr(rhs, pairs)?;
        }
        V2Expr::Call { callee, args, .. } => {
            if !matches!(callee.as_ref(), V2Expr::Ident(..)) {
                subst_expr(callee, pairs)?;
            }
            for a in args {
                subst_expr(&mut a.value, pairs)?;
            }
        }
        V2Expr::MethodCall { receiver, args, .. } => {
            subst_expr(receiver, pairs)?;
            for a in args {
                subst_expr(&mut a.value, pairs)?;
            }
        }
        V2Expr::Match {
            scrutinee, arms, ..
        } => {
            subst_expr(scrutinee, pairs)?;
            for arm in arms {
                subst_pattern(&mut arm.pattern, pairs)?;
                subst_expr(&mut arm.value, pairs)?;
            }
        }
        V2Expr::If {
            cond,
            then_value,
            else_value,
            ..
        } => {
            subst_expr(cond, pairs)?;
            subst_expr(then_value, pairs)?;
            subst_expr(else_value, pairs)?;
        }
        V2Expr::Block { body, .. } => subst_stmts(body, pairs)?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Inline operands
// ---------------------------------------------------------------------------------------------

/// `f(.., key: match s { P => v, .. }, ..);` → `match s { P => f(.., key: v, ..); .. }` when
/// every arm of the first such operand is a compile-time operand, and neither its tests nor the
/// other operands make calls or borrow (so testing first does not reorder anything observable:
/// a test that wrote a cell would run before an earlier operand reads it). `args` are the statement's
/// call arguments; `rebuild` makes the statement again from rewritten arguments. `None` when no
/// operand qualifies.
pub(crate) fn split_inline_operand(
    span: Span,
    args: &[CallArg],
    rebuild: &dyn Fn(Vec<CallArg>) -> V2Stmt,
) -> Option<V2Stmt> {
    let at = args.iter().position(|a| inline_selector(&a.value))?;
    let others_pure = args
        .iter()
        .enumerate()
        .all(|(i, a)| i == at || !makes_call(&a.value));
    if !others_pure || !selector_tests_pure(&args[at].value) {
        return None;
    }
    // A name an arm yields must hold the same value where the statement reads it.
    let stmt = [rebuild(args.to_vec())];
    let mut leaves = Vec::new();
    selector_leaves(&args[at].value, &mut leaves);
    if leaves.iter().any(|v| match v {
        V2Expr::Ident(_, n) => find_write(&stmt, n).is_some(),
        _ => false,
    }) {
        return None;
    }
    let with = |value: &V2Expr| {
        let mut new_args = args.to_vec();
        new_args[at].value = value.clone();
        vec![rebuild(new_args)]
    };
    Some(match &args[at].value {
        V2Expr::Match {
            scrutinee, arms, ..
        } => V2Stmt::Match {
            span,
            scrutinee: scrutinee.as_ref().clone(),
            arms: arms
                .iter()
                .map(|a| MatchArm {
                    span: a.span,
                    pattern: a.pattern.clone(),
                    body: with(&a.value),
                })
                .collect(),
        },
        V2Expr::If { .. } => if_chain(span, &args[at].value, &with),
        _ => unreachable!("inline_selector accepts only match and if"),
    })
}

fn if_chain(span: Span, e: &V2Expr, with: &dyn Fn(&V2Expr) -> Vec<V2Stmt>) -> V2Stmt {
    let V2Expr::If {
        cond,
        then_value,
        else_value,
        ..
    } = e
    else {
        unreachable!("if_chain is called on an 'if' expression");
    };
    let else_body = match else_value.as_ref() {
        nested @ V2Expr::If { .. } => vec![if_chain(span, nested, with)],
        v => with(v),
    };
    V2Stmt::If {
        span,
        cond: cond.as_ref().clone(),
        then_body: with(then_value),
        else_body,
    }
}

/// A `match` ending in `_` or an `if` chain whose every arm is a compile-time operand.
fn inline_selector(e: &V2Expr) -> bool {
    let leaf = |v: &V2Expr| is_const_operand(v);
    match e {
        V2Expr::Match { arms, .. } => {
            matches!(
                arms.last().map(|a| &a.pattern),
                Some(MatchPattern::Wildcard(_))
            ) && arms.iter().all(|a| leaf(&a.value))
        }
        V2Expr::If {
            then_value,
            else_value,
            ..
        } => {
            leaf(then_value)
                && match else_value.as_ref() {
                    nested @ V2Expr::If { .. } => inline_selector(nested),
                    v => leaf(v),
                }
        }
        _ => false,
    }
}

/// The arm values of an inline selector.
fn selector_leaves<'e>(e: &'e V2Expr, out: &mut Vec<&'e V2Expr>) {
    match e {
        V2Expr::Match { arms, .. } => out.extend(arms.iter().map(|a| &a.value)),
        V2Expr::If {
            then_value,
            else_value,
            ..
        } => {
            out.push(then_value);
            match else_value.as_ref() {
                nested @ V2Expr::If { .. } => selector_leaves(nested, out),
                v => out.push(v),
            }
        }
        _ => {}
    }
}

/// Whether the tests of an inline selector (its scrutinee, or each condition of an `if`
/// chain) make no call and borrow nothing.
fn selector_tests_pure(e: &V2Expr) -> bool {
    match e {
        V2Expr::Match { scrutinee, .. } => !makes_call(scrutinee),
        V2Expr::If {
            cond, else_value, ..
        } => !makes_call(cond) && selector_tests_pure(else_value),
        _ => true,
    }
}

/// Whether `e` makes a call or borrows (`&mut`), either of which may write.
fn makes_call(e: &V2Expr) -> bool {
    let mut found = false;
    visit_expr(e, &mut |x| {
        if matches!(
            x,
            V2Expr::Call { .. } | V2Expr::MethodCall { .. } | V2Expr::MutRef { .. }
        ) {
            found = true;
        }
    });
    found
}
