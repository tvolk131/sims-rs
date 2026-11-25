//! Syntactic facts about AST statements and expressions: mutation/borrow of an identifier,
//! divergence, loop exits, returns, helper-cell preservation and `stack_obj` clobbering.

use super::specialize::init_blocks;
use super::*;

/// `obj.run_functional(..)` or `!obj.run_functional(..)` without type arguments, as
/// `(negated, receiver, args, span)`.
pub(super) fn run_functional_test(expr: &V2Expr) -> Option<(bool, &V2Expr, &[CallArg], Span)> {
    let (negated, inner) = match expr {
        V2Expr::Unary {
            op: V2UnaryOp::Not,
            inner,
            ..
        } => (true, inner.as_ref()),
        e => (false, e),
    };
    match inner {
        V2Expr::MethodCall {
            span,
            receiver,
            method,
            type_args,
            args,
        } if method == "run_functional" && type_args.is_empty() => {
            Some((negated, receiver.as_ref(), args.as_slice(), *span))
        }
        _ => None,
    }
}

/// An integer literal, possibly under unary minus (no names, so nothing tunable).
pub(super) fn is_int_literal_expr(e: &V2Expr) -> bool {
    match e {
        V2Expr::Int(..) => true,
        V2Expr::Unary {
            op: V2UnaryOp::Neg,
            inner,
            ..
        } => is_int_literal_expr(inner),
        _ => false,
    }
}

/// The builtins whose arguments can name a local they write (see [`builtin_args_write_ident`]).
pub(super) const LOCAL_WRITING_BUILTINS: &[&str] = &[
    "random_into",
    "animate",
    "set_headline",
    "push_interaction",
    "relationship",
    "create_object",
    "find_location",
    "set_to_next",
];

pub(super) fn stmts_mutate_or_borrow_ident(stmts: &[V2Stmt], target_name: &str) -> bool {
    stmts_mutate_or_borrow_ident_in(stmts, target_name, LOCAL_WRITING_BUILTINS)
}

pub(super) fn stmts_mutate_or_borrow_ident_in(
    stmts: &[V2Stmt],
    target_name: &str,
    w: &[&str],
) -> bool {
    stmts.iter().any(|s| match s {
        V2Stmt::Let { name, init, .. } => {
            name == target_name || expr_borrows_ident_in(init, target_name, w)
        }
        V2Stmt::LetSome {
            name,
            init,
            else_body,
            ..
        } => {
            name == target_name
                || expr_borrows_ident_in(init, target_name, w)
                || stmts_mutate_or_borrow_ident_in(else_body, target_name, w)
        }
        V2Stmt::Assign { target, value, .. } => {
            matches!(target, V2Expr::Ident(_, n) if n == target_name)
                || expr_borrows_ident_in(target, target_name, w)
                || expr_borrows_ident_in(value, target_name, w)
        }
        V2Stmt::WithLock {
            lock_expr, body, ..
        } => {
            matches!(lock_expr, V2Expr::Ident(_, n) if n == target_name)
                || expr_borrows_ident_in(lock_expr, target_name, w)
                || stmts_mutate_or_borrow_ident_in(body, target_name, w)
        }
        V2Stmt::Defer { body, .. } => stmts_mutate_or_borrow_ident_in(body, target_name, w),
        V2Stmt::Repeat { count, body, .. } => {
            expr_borrows_ident_in(count, target_name, w)
                || stmts_mutate_or_borrow_ident_in(body, target_name, w)
        }
        V2Stmt::While { cond, body, .. } => {
            expr_borrows_ident_in(cond, target_name, w)
                || stmts_mutate_or_borrow_ident_in(body, target_name, w)
        }
        V2Stmt::ForEach {
            binding,
            iter,
            body,
            else_body,
            ..
        } => {
            binding == target_name
                || match iter {
                    ForEachIter::Range { start, end, .. } => {
                        expr_borrows_ident_in(start, target_name, w)
                            || expr_borrows_ident_in(end, target_name, w)
                    }
                    ForEachIter::ObjectsByGuid(e) => expr_borrows_ident_in(e, target_name, w),
                    _ => false,
                }
                || stmts_mutate_or_borrow_ident_in(body, target_name, w)
                || else_body
                    .as_deref()
                    .is_some_and(|body| stmts_mutate_or_borrow_ident_in(body, target_name, w))
        }
        V2Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            expr_borrows_ident_in(cond, target_name, w)
                || stmts_mutate_or_borrow_ident_in(then_body, target_name, w)
                || stmts_mutate_or_borrow_ident_in(else_body, target_name, w)
        }
        V2Stmt::Match {
            scrutinee, arms, ..
        } => {
            expr_borrows_ident_in(scrutinee, target_name, w)
                || arms
                    .iter()
                    .any(|a| stmts_mutate_or_borrow_ident_in(&a.body, target_name, w))
        }
        V2Stmt::Return { value: Some(e), .. } | V2Stmt::Expr { expr: e, .. } => {
            expr_borrows_ident_in(e, target_name, w)
        }
        V2Stmt::LetUninit { name, .. } => name == target_name,
        V2Stmt::Animate {
            args, event, body, ..
        } => {
            event.as_deref() == Some(target_name)
                || builtin_args_write_ident("animate", args, target_name, w)
                || args
                    .iter()
                    .any(|a| expr_borrows_ident_in(&a.value, target_name, w))
                || stmts_mutate_or_borrow_ident_in(body, target_name, w)
        }
        V2Stmt::Break { .. }
        | V2Stmt::Continue { .. }
        | V2Stmt::Return { value: None, .. }
        | V2Stmt::ReturnError { .. }
        | V2Stmt::RawAir { .. } => false,
    })
}

/// Whether a call to the builtin `fname` names `target_name` as a local it may write (or, for the
/// local-slot arguments of `set_headline`/`push_interaction`, hand to the VM by slot). Only
/// names in `builtins` (the ones no user callable shadows) count.
pub(super) fn builtin_args_write_ident(
    fname: &str,
    args: &[CallArg],
    target_name: &str,
    builtins: &[&str],
) -> bool {
    if !builtins.contains(&fname) {
        return false;
    }
    let is_target = |e: &V2Expr| matches!(e, V2Expr::Ident(_, n) if n == target_name);
    match fname {
        "random_into" => args
            .first()
            .is_some_and(|a| a.name.is_none() && is_target(&a.value)),
        "animate" | "set_headline" | "push_interaction" | "create_object" | "find_location" => args
            .iter()
            .any(|a| matches!(a.name.as_deref(), Some("local" | "icon")) && is_target(&a.value)),
        // Without flag 4 the VM stores the number into `value:`.
        "relationship" => args
            .iter()
            .any(|a| matches!(a.name.as_deref(), Some("value" | "local")) && is_target(&a.value)),
        // Set To Next writes the next object into its destination.
        "set_to_next" => args
            .iter()
            .any(|a| a.name.as_deref() == Some("dest") && is_target(&a.value)),
        _ => false,
    }
}

pub(super) fn expr_borrows_ident_in(expr: &V2Expr, target_name: &str, w: &[&str]) -> bool {
    match expr {
        V2Expr::MutRef { inner, .. } => {
            matches!(inner.as_ref(), V2Expr::Ident(_, n) if n == target_name)
                || expr_borrows_ident_in(inner, target_name, w)
        }
        V2Expr::Field { base, .. }
        | V2Expr::Deref { inner: base, .. }
        | V2Expr::Unary { inner: base, .. }
        | V2Expr::Try { inner: base, .. } => expr_borrows_ident_in(base, target_name, w),
        V2Expr::Index { base, index, .. } => {
            expr_borrows_ident_in(base, target_name, w)
                || expr_borrows_ident_in(index, target_name, w)
        }
        V2Expr::Binary { lhs, rhs, .. } => {
            expr_borrows_ident_in(lhs, target_name, w) || expr_borrows_ident_in(rhs, target_name, w)
        }
        V2Expr::Tuple(_, elems) => elems
            .iter()
            .any(|e| expr_borrows_ident_in(e, target_name, w)),
        V2Expr::Match {
            scrutinee, arms, ..
        } => {
            expr_borrows_ident_in(scrutinee, target_name, w)
                || arms
                    .iter()
                    .any(|a| expr_borrows_ident_in(&a.value, target_name, w))
        }
        V2Expr::If {
            cond,
            then_value,
            else_value,
            ..
        } => [cond, then_value, else_value]
            .iter()
            .any(|e| expr_borrows_ident_in(e, target_name, w)),
        V2Expr::Call { callee, args, .. } => {
            expr_borrows_ident_in(callee, target_name, w)
                || matches!(callee.as_ref(), V2Expr::Ident(_, f) if builtin_args_write_ident(f, args, target_name, w))
                || args
                    .iter()
                    .any(|a| expr_borrows_ident_in(&a.value, target_name, w))
        }
        V2Expr::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            (matches!(method.as_str(), "set" | "clear")
                && matches!(receiver.as_ref(), V2Expr::Ident(_, n) if n == target_name))
                || expr_borrows_ident_in(receiver, target_name, w)
                || args
                    .iter()
                    .any(|a| expr_borrows_ident_in(&a.value, target_name, w))
        }
        V2Expr::Block { body, .. } => stmts_mutate_or_borrow_ident_in(body, target_name, w),
        V2Expr::Int(..)
        | V2Expr::Bool(..)
        | V2Expr::Str(..)
        | V2Expr::Ident(..)
        | V2Expr::Path(..) => false,
    }
}

pub(super) fn stmts_unconditionally_diverge(
    stmts: &[V2Stmt],
    env: &ObjectEnv<'_>,
    owner_module: &str,
    allow_local: bool,
) -> bool {
    match stmts.last() {
        Some(
            V2Stmt::Return { .. }
            | V2Stmt::ReturnError { .. }
            | V2Stmt::Break { .. }
            | V2Stmt::Continue { .. },
        ) => true,
        Some(V2Stmt::If {
            then_body,
            else_body,
            ..
        }) => {
            !else_body.is_empty()
                && stmts_unconditionally_diverge(then_body, env, owner_module, allow_local)
                && stmts_unconditionally_diverge(else_body, env, owner_module, allow_local)
        }
        Some(V2Stmt::Match {
            scrutinee, arms, ..
        }) => {
            !arms.is_empty()
                && match_is_exhaustive(scrutinee, arms, env, owner_module, allow_local)
                && arms
                    .iter()
                    .all(|a| stmts_unconditionally_diverge(&a.body, env, owner_module, allow_local))
        }
        Some(V2Stmt::WithLock { body, .. }) => {
            stmts_unconditionally_diverge(body, env, owner_module, allow_local)
        }
        // A `cancel_on_abort` loop leaves (with a `break`) when the interaction is cancelled.
        Some(V2Stmt::While {
            cond: V2Expr::Bool(_, true),
            cancel_on_abort: false,
            body,
            ..
        }) => !loop_body_breaks(body),
        _ => false,
    }
}

/// Whether a `break` in `stmts` exits the loop whose body `stmts` is (breaks inside nested loops,
/// including an `animate` event handler, exit those instead).
pub(super) fn loop_body_breaks(stmts: &[V2Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        V2Stmt::Break { .. } => true,
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => loop_body_breaks(then_body) || loop_body_breaks(else_body),
        V2Stmt::Match { arms, .. } => arms.iter().any(|a| loop_body_breaks(&a.body)),
        V2Stmt::WithLock { body, .. } | V2Stmt::Defer { body, .. } => loop_body_breaks(body),
        V2Stmt::LetSome { else_body, .. } => loop_body_breaks(else_body),
        V2Stmt::ForEach { else_body, .. } => else_body.as_deref().is_some_and(loop_body_breaks),
        V2Stmt::Let { init, .. } => init_blocks(init).into_iter().any(loop_body_breaks),
        _ => false,
    })
}

/// Whether `stmts` may `break` or `continue` the loop they sit in (nested loops own theirs).
pub(super) fn stmts_exit_loop(stmts: &[V2Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        V2Stmt::Break { .. } | V2Stmt::Continue { .. } => true,
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => stmts_exit_loop(then_body) || stmts_exit_loop(else_body),
        V2Stmt::Match { arms, .. } => arms.iter().any(|a| stmts_exit_loop(&a.body)),
        V2Stmt::WithLock { body, .. } | V2Stmt::Defer { body, .. } => stmts_exit_loop(body),
        V2Stmt::LetSome { else_body, .. } => stmts_exit_loop(else_body),
        V2Stmt::ForEach { else_body, .. } => else_body.as_deref().is_some_and(stmts_exit_loop),
        V2Stmt::RawAir { code, .. } => {
            super::raw_facts::raw_facts(code, None).exits_enclosing_loop()
        }
        V2Stmt::Let { init, .. } => init_blocks(init).into_iter().any(stmts_exit_loop),
        _ => false,
    })
}

pub(super) fn stmts_have_return(stmts: &[V2Stmt]) -> bool {
    stmts.iter().any(|s| match s {
        // `return error;` may resolve to an ordinary `return <local error>;` (see lowering).
        V2Stmt::Return { .. } | V2Stmt::ReturnError { .. } => true,
        V2Stmt::WithLock { body, .. }
        | V2Stmt::Defer { body, .. }
        | V2Stmt::Repeat { body, .. }
        | V2Stmt::While { body, .. }
        | V2Stmt::Animate { body, .. } => stmts_have_return(body),
        V2Stmt::ForEach {
            body, else_body, ..
        } => stmts_have_return(body) || else_body.as_deref().is_some_and(stmts_have_return),
        V2Stmt::LetSome { else_body, .. } => stmts_have_return(else_body),
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => stmts_have_return(then_body) || stmts_have_return(else_body),
        V2Stmt::Match { arms, .. } => arms.iter().any(|a| stmts_have_return(&a.body)),
        V2Stmt::Let { init, .. } => init_blocks(init).into_iter().any(stmts_have_return),
        _ => false,
    })
}

/// A helper's relative `break` reaches its wrapper directly unless a return is nested
/// inside one of the helper's own loops, including the loop for animation events.
pub(super) fn stmts_return_through_loop(stmts: &[V2Stmt]) -> bool {
    stmts.iter().any(|stmt| match stmt {
        V2Stmt::Repeat { body, .. } | V2Stmt::While { body, .. } | V2Stmt::Animate { body, .. } => {
            stmts_have_return(body)
        }
        // The exhaustion block is lowered inside the iterator's loop, so a return there leaves
        // a loop too.
        V2Stmt::ForEach {
            body, else_body, ..
        } => stmts_have_return(body) || else_body.as_deref().is_some_and(stmts_have_return),
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => stmts_return_through_loop(then_body) || stmts_return_through_loop(else_body),
        V2Stmt::Match { arms, .. } => arms.iter().any(|a| stmts_return_through_loop(&a.body)),
        V2Stmt::WithLock { body, .. } | V2Stmt::Defer { body, .. } => {
            stmts_return_through_loop(body)
        }
        V2Stmt::LetSome { else_body, .. } => stmts_return_through_loop(else_body),
        V2Stmt::Let { init, .. } => init_blocks(init).into_iter().any(stmts_return_through_loop),
        _ => false,
    })
}

/// Expressions whose evaluation cannot modify a forwarded temp, parameter, or own attribute.
/// The names of the functions `stmts` call (with a plain name), nested statements included.
pub(super) fn collect_callee_names(stmts: &[V2Stmt], out: &mut Vec<String>) {
    for s in stmts {
        for e in super::specialize::stmt_exprs(s) {
            super::specialize::visit_expr(e, &mut |x| {
                if let V2Expr::Call { callee, .. } = x
                    && let V2Expr::Ident(_, name) = callee.as_ref()
                {
                    out.push(name.clone());
                }
            });
        }
        for body in super::specialize::stmt_bodies(s) {
            collect_callee_names(body, out);
        }
    }
}

pub(super) fn helper_expr_has_no_writes(expr: &V2Expr) -> bool {
    match expr {
        V2Expr::Int(..)
        | V2Expr::Bool(..)
        | V2Expr::Str(..)
        | V2Expr::Path(..)
        | V2Expr::Ident(..) => true,
        V2Expr::Unary { inner, .. } => helper_expr_has_no_writes(inner),
        V2Expr::Binary { lhs, rhs, .. } => {
            helper_expr_has_no_writes(lhs) && helper_expr_has_no_writes(rhs)
        }
        V2Expr::Field { base, .. } => matches!(base.as_ref(), V2Expr::Ident(..)),
        V2Expr::Index { base, index, .. } => {
            let simple_base = match base.as_ref() {
                V2Expr::Ident(..) => true,
                V2Expr::Field { base, field, .. } => {
                    matches!(base.as_ref(), V2Expr::Ident(..)) && field != "sprite_flags"
                }
                _ => false,
            };
            simple_base && helper_expr_has_no_writes(index)
        }
        _ => false,
    }
}

pub(super) fn stmts_preserve_helper_cell(
    stmts: &[V2Stmt],
    cell: HelperCell,
    self_states: Option<&BTreeMap<String, ResolvedState>>,
) -> bool {
    let preserves = |body: &[V2Stmt]| stmts_preserve_helper_cell(body, cell, self_states);
    let expr_ok = helper_expr_has_no_writes;
    stmts.iter().all(|stmt| match stmt {
        V2Stmt::Let { init, .. } => expr_ok(init),
        V2Stmt::Assign { target, value, .. } => {
            let disjoint = match target {
                // Bare identifiers in helper scope name local bindings or `stack_obj`.
                V2Expr::Ident(..) => true,
                V2Expr::Field { base, field, .. } => match cell {
                    HelperCell::OwnAttr(i) => {
                        matches!(base.as_ref(), V2Expr::Ident(_, name) if name == "self")
                            && self_states
                                .and_then(|s| s.get(field))
                                .is_some_and(|s| s.attr_index != i as usize)
                    }
                    _ => expr_ok(target),
                },
                V2Expr::Index { base, index, .. } => match base.as_ref() {
                    V2Expr::Ident(_, name) if name == "temp" || name == "param" => {
                        match (name.as_str(), cell) {
                            ("temp", HelperCell::Temp(i)) | ("param", HelperCell::Param(i)) => {
                                matches!(index.as_ref(), V2Expr::Int(_, n) if *n != i64::from(i))
                            }
                            _ => expr_ok(index),
                        }
                    }
                    V2Expr::Field { base, field, .. } => match cell {
                        HelperCell::OwnAttr(i) => {
                            self_states.is_some()
                                && field == "attr"
                                && matches!(base.as_ref(), V2Expr::Ident(_, name) if name == "self")
                                && matches!(index.as_ref(), V2Expr::Int(_, n) if *n != i64::from(i))
                        }
                        _ => expr_ok(target),
                    },
                    _ => false,
                },
                _ => false,
            };
            disjoint && expr_ok(value)
        }
        V2Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => expr_ok(cond) && preserves(then_body) && preserves(else_body),
        V2Stmt::While { cond, body, .. } => expr_ok(cond) && preserves(body),
        V2Stmt::Repeat { count, body, .. } => expr_ok(count) && preserves(body),
        V2Stmt::Match {
            scrutinee, arms, ..
        } => expr_ok(scrutinee) && arms.iter().all(|a| preserves(&a.body)),
        V2Stmt::Defer { body, .. } => preserves(body),
        V2Stmt::Return { value, .. } => value.as_ref().is_none_or(expr_ok),
        V2Stmt::Break { .. }
        | V2Stmt::Continue { .. }
        | V2Stmt::ReturnError { .. }
        | V2Stmt::LetUninit { .. } => true,
        _ => false,
    })
}

pub(super) fn stmts_always_return(
    stmts: &[V2Stmt],
    env: &ObjectEnv<'_>,
    owner_module: &str,
    allow_local: bool,
) -> bool {
    for stmt in stmts {
        match stmt {
            V2Stmt::Return { .. } | V2Stmt::ReturnError { .. } => return true,
            V2Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                if !else_body.is_empty()
                    && stmts_always_return(then_body, env, owner_module, allow_local)
                    && stmts_always_return(else_body, env, owner_module, allow_local)
                {
                    return true;
                }
            }
            V2Stmt::Match {
                scrutinee, arms, ..
            } => {
                if !arms.is_empty()
                    && match_is_exhaustive(scrutinee, arms, env, owner_module, allow_local)
                    && arms
                        .iter()
                        .all(|a| stmts_always_return(&a.body, env, owner_module, allow_local))
                {
                    return true;
                }
            }
            V2Stmt::WithLock { body, .. }
                if stmts_always_return(body, env, owner_module, allow_local) =>
            {
                return true;
            }
            V2Stmt::ForEach {
                body,
                else_body: Some(else_body),
                ..
            } if !stmts_exit_loop(body)
                && stmts_always_return(else_body, env, owner_module, allow_local) =>
            {
                return true;
            }
            // `loop { }` / `while true { }` without a `break` (or a cancel exit) never falls
            // through.
            V2Stmt::While {
                cond: V2Expr::Bool(_, true),
                cancel_on_abort: false,
                body,
                ..
            } if !loop_body_breaks(body) => return true,
            _ => {}
        }
    }
    false
}

pub(super) fn stmts_may_clobber_stack_obj(
    stmts: &[V2Stmt],
    env: &ObjectEnv<'_>,
    owner_mod: &str,
    allow_local_fns: bool,
    sim_name: Option<&str>,
    depth: usize,
) -> bool {
    if depth > MAX_INLINE_DEPTH {
        return true;
    }
    stmts.iter().any(|s| match s {
        // Rebinding the Sim name makes later `sim.x` reads foreign-object reads (stack_obj swaps)
        // that the field check below would otherwise treat as plain Sim reads.
        V2Stmt::Let { name, .. } | V2Stmt::LetSome { name, .. }
            if Some(name.as_str()) == sim_name =>
        {
            true
        }
        V2Stmt::LetSome {
            init, else_body, ..
        } => {
            expr_may_clobber_stack_obj(init, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    else_body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::Let { init, .. } => {
            expr_may_clobber_stack_obj(init, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        V2Stmt::Assign {
            target: V2Expr::Ident(_, name),
            ..
        } if name == "stack_obj" => true,
        V2Stmt::Assign { target, value, .. } => {
            expr_may_clobber_stack_obj(target, env, owner_mod, allow_local_fns, sim_name, depth)
                || expr_may_clobber_stack_obj(
                    value,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::WithLock {
            lock_expr, body, ..
        } => {
            expr_may_clobber_stack_obj(lock_expr, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::Defer { body, .. } => {
            stmts_may_clobber_stack_obj(body, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        V2Stmt::Repeat { count, body, .. } => {
            expr_may_clobber_stack_obj(count, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::While { cond, body, .. } => {
            expr_may_clobber_stack_obj(cond, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::ForEach {
            iter: ForEachIter::Range { start, end, .. },
            body,
            ..
        } => {
            expr_may_clobber_stack_obj(start, env, owner_mod, allow_local_fns, sim_name, depth)
                || expr_may_clobber_stack_obj(end, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::ForEach { .. } => true,
        V2Stmt::RawAir { code, .. } => raw_air_may_clobber_stack_obj(code),
        V2Stmt::Animate {
            span, args, body, ..
        } => {
            // Analysed as a call, so a user `fn animate` that shadows the builtin is followed.
            let call = V2Expr::Call {
                span: *span,
                callee: Box::new(V2Expr::Ident(*span, "animate".to_owned())),
                type_args: Vec::new(),
                args: args.clone(),
            };
            expr_may_clobber_stack_obj(&call, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            expr_may_clobber_stack_obj(cond, env, owner_mod, allow_local_fns, sim_name, depth)
                || stmts_may_clobber_stack_obj(
                    then_body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
                || stmts_may_clobber_stack_obj(
                    else_body,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Stmt::Match {
            scrutinee, arms, ..
        } => {
            expr_may_clobber_stack_obj(scrutinee, env, owner_mod, allow_local_fns, sim_name, depth)
                || arms.iter().any(|a| {
                    stmts_may_clobber_stack_obj(
                        &a.body,
                        env,
                        owner_mod,
                        allow_local_fns,
                        sim_name,
                        depth,
                    )
                })
        }
        V2Stmt::Return { value: Some(e), .. } | V2Stmt::Expr { expr: e, .. } => {
            expr_may_clobber_stack_obj(e, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        _ => false,
    })
}

pub(super) fn expr_may_clobber_stack_obj(
    expr: &V2Expr,
    env: &ObjectEnv<'_>,
    owner_mod: &str,
    allow_local_fns: bool,
    sim_name: Option<&str>,
    depth: usize,
) -> bool {
    if depth > MAX_INLINE_DEPTH {
        return true;
    }
    match expr {
        V2Expr::Call { callee, args, .. } => {
            if let V2Expr::Ident(_, fname) = callee.as_ref() {
                if matches!(fname.as_str(), "find_best_object" | "create_object")
                    || (fname == "set_to_next" && set_to_next_steps_stack_obj(args))
                {
                    return true;
                }
                // `distance_to` points `stack_obj` at its target unless that already is it.
                if fname == "distance_to"
                    && env.lookup_fn(owner_mod, fname, allow_local_fns).is_none()
                    && !args.first().is_some_and(|a| {
                        a.name.is_none()
                            && matches!(&a.value, V2Expr::Ident(_, n) if n == "stack_obj")
                    })
                {
                    return true;
                }
                if matches!(
                    fname.as_str(),
                    "find_best_interaction" | "gosub_found_action"
                ) && env.lookup_fn(owner_mod, fname, allow_local_fns).is_none()
                {
                    return true;
                }
                if let Some((fmod, fdecl, is_local)) =
                    env.lookup_fn(owner_mod, fname, allow_local_fns)
                {
                    let fn_sim = fdecl
                        .params
                        .iter()
                        .find(|p| p.ty == V2Type::Sim)
                        .map(|p| p.name.as_str());
                    if stmts_may_clobber_stack_obj(
                        &fdecl.body,
                        env,
                        &fmod,
                        is_local,
                        fn_sim,
                        depth + 1,
                    ) || fdecl
                        .params
                        .iter()
                        .filter_map(|p| p.default.as_ref())
                        .any(|def_expr| {
                            expr_may_clobber_stack_obj(
                                def_expr,
                                env,
                                &fmod,
                                is_local,
                                fn_sim,
                                depth + 1,
                            )
                        })
                    {
                        return true;
                    }
                }
            }
            args.iter().any(|a| {
                expr_may_clobber_stack_obj(
                    &a.value,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
            })
        }
        V2Expr::MethodCall {
            receiver,
            method,
            args,
            ..
        } => {
            method == "run_functional"
                || expr_may_clobber_stack_obj(
                    receiver,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
                || args.iter().any(|a| {
                    expr_may_clobber_stack_obj(
                        &a.value,
                        env,
                        owner_mod,
                        allow_local_fns,
                        sim_name,
                        depth,
                    )
                })
        }
        V2Expr::Index { base, index, .. } => {
            if let V2Expr::Field {
                base: root, field, ..
            } = base.as_ref()
                && let V2Expr::Ident(_, rname) = root.as_ref()
            {
                // Advertisement cells (scope 12 / 28 / 29) and the stack object's own cells
                // never switch `stack_obj` (`stack_obj` cannot be rebound as a name).
                let direct = (rname == "advert"
                    && matches!(field.as_str(), "delta" | "personality" | "minimum"))
                    || rname == "stack_obj";
                if direct {
                    return expr_may_clobber_stack_obj(
                        index,
                        env,
                        owner_mod,
                        allow_local_fns,
                        sim_name,
                        depth,
                    );
                }
                // A Sim tree's own Sim (`sim.attr[i]`, `sim.data[i]`) is `my.`, not a foreign
                // object reached through `stack_obj`.
                if matches!(field.as_str(), "attr" | "data")
                    && rname != "self"
                    && Some(rname.as_str()) != sim_name
                {
                    return true;
                }
            }
            expr_may_clobber_stack_obj(base, env, owner_mod, allow_local_fns, sim_name, depth)
                || expr_may_clobber_stack_obj(
                    index,
                    env,
                    owner_mod,
                    allow_local_fns,
                    sim_name,
                    depth,
                )
        }
        V2Expr::Field { base, .. } => {
            // Typed foreign object field access (`fridge.stock`, `plate.nutrition`, etc.) swaps `stack_obj`!
            // (Named BCON table entries such as `LampTuning.a` are plain constant reads.)
            if let V2Expr::Ident(_, rname) = base.as_ref()
                && rname != "self"
                && rname != "stack_obj"
                && Some(rname.as_str()) != sim_name
                && !env.is_bcon_table_name(owner_mod, rname, allow_local_fns)
            {
                return true;
            }
            expr_may_clobber_stack_obj(base, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        // A `&mut stack_obj` borrow lets the callee write the running frame's stack object.
        V2Expr::MutRef { inner, .. } if matches!(inner.as_ref(), V2Expr::Ident(_, n) if n == "stack_obj") => {
            true
        }
        V2Expr::Deref { inner: base, .. }
        | V2Expr::MutRef { inner: base, .. }
        | V2Expr::Unary { inner: base, .. }
        | V2Expr::Try { inner: base, .. } => {
            expr_may_clobber_stack_obj(base, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        V2Expr::Binary { lhs, rhs, .. } => {
            expr_may_clobber_stack_obj(lhs, env, owner_mod, allow_local_fns, sim_name, depth)
                || expr_may_clobber_stack_obj(rhs, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        V2Expr::Tuple(_, elems) => elems.iter().any(|e| {
            expr_may_clobber_stack_obj(e, env, owner_mod, allow_local_fns, sim_name, depth)
        }),
        V2Expr::Match {
            scrutinee, arms, ..
        } => std::iter::once(scrutinee.as_ref())
            .chain(arms.iter().map(|a| &a.value))
            .any(|e| {
                expr_may_clobber_stack_obj(e, env, owner_mod, allow_local_fns, sim_name, depth)
            }),
        V2Expr::If {
            cond,
            then_value,
            else_value,
            ..
        } => [cond, then_value, else_value].iter().any(|e| {
            expr_may_clobber_stack_obj(e, env, owner_mod, allow_local_fns, sim_name, depth)
        }),
        V2Expr::Block { body, .. } => {
            stmts_may_clobber_stack_obj(body, env, owner_mod, allow_local_fns, sim_name, depth)
        }
        _ => false,
    }
}

impl ExplicitStackOps {
    pub(super) fn in_stmts(
        &mut self,
        stmts: &[V2Stmt],
        env: &ObjectEnv<'_>,
        owner: &str,
        local: bool,
    ) -> bool {
        stmts.iter().any(|stmt| {
            // `let Some(x) = find_best_object(..) else {..}` puts the cursor back itself; only
            // its arguments can move it. Anywhere else (a condition, a bool value) Find Best
            // Object leaves the found object in `stack_obj`.
            let let_some_search = match stmt {
                V2Stmt::LetSome {
                    init: V2Expr::Call { callee, args, .. },
                    ..
                } if matches!(callee.as_ref(), V2Expr::Ident(_, n) if n == "find_best_object") => {
                    Some(args)
                }
                _ => None,
            };
            if let Some(args) = let_some_search {
                return args.iter().any(|a| self.in_expr(&a.value, env, owner, local))
                    || super::specialize::stmt_bodies(stmt)
                        .into_iter()
                        .any(|body| self.in_stmts(body, env, owner, local));
            }
            super::specialize::stmt_exprs(stmt)
                .into_iter()
                .any(|expr| self.in_expr(expr, env, owner, local))
                || matches!(stmt, V2Stmt::Animate { .. } if self.in_call("animate", &[], env, owner, local))
                || super::specialize::stmt_bodies(stmt)
                    .into_iter()
                    .any(|body| self.in_stmts(body, env, owner, local))
        })
    }

    pub(super) fn in_expr(
        &mut self,
        expr: &V2Expr,
        env: &ObjectEnv<'_>,
        owner: &str,
        local: bool,
    ) -> bool {
        let mut found = false;
        super::specialize::visit_expr(expr, &mut |expr| {
            if !found
                && let V2Expr::Call { callee, args, .. } = expr
                && let V2Expr::Ident(_, name) = callee.as_ref()
            {
                found = self.in_call(name, args, env, owner, local);
            }
        });
        found
    }

    pub(super) fn in_call(
        &mut self,
        name: &str,
        args: &[CallArg],
        env: &ObjectEnv<'_>,
        owner: &str,
        local: bool,
    ) -> bool {
        if local && env.trees.contains_key(name) {
            return false; // A separate BHAV frame has its own stack object.
        }
        if let Some((module, decl, is_local)) = env.lookup_fn(owner, name, local) {
            self.visited.insert((module.clone(), decl.name, is_local))
                && (self.in_stmts(&decl.body, env, &module, is_local)
                    || decl
                        .params
                        .iter()
                        .filter_map(|p| p.default.as_ref())
                        .any(|expr| self.in_expr(expr, env, &module, is_local)))
        } else {
            name == "find_best_object"
                || (name == "set_to_next" && set_to_next_steps_stack_obj(args))
                || (name == "create_object" && has_create_dest(args))
        }
    }
}

/// Whether a `set_to_next` call steps `stack_obj`: it does unless its `dest:` names another
/// cell (a local, `param[N]` or `temp[N]`), which is then the only cell it writes (`sims-vm`
/// `primitives/set_to_next.rs`).
pub(super) fn set_to_next_steps_stack_obj(args: &[CallArg]) -> bool {
    args.iter()
        .find(|a| a.name.as_deref() == Some("dest"))
        .is_none_or(|a| matches!(&a.value, V2Expr::Ident(_, n) if n == "stack_obj"))
}

pub(super) fn has_create_dest(args: &[CallArg]) -> bool {
    args.iter().any(|arg| arg.name.as_deref() == Some("dest"))
}
