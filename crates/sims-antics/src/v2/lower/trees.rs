//! Tree emission: standalone trees, guard trees, `.air` headers and the framed tree writer (frame
//! layout, slot spans).

use super::*;

/// Lowers and emits one standalone `tree` member as a real (non-inlined) BHAV.
pub(super) fn emit_standalone_tree(
    trees_air: &mut String,
    env: &ObjectEnv<'_>,
    t: &OwnedTree,
) -> Result<()> {
    let air = lower_standalone_tree(env, t, false)?;
    if t.decl.inherit_context {
        // A private call inherits the runner and the current stack object. Check that lowering
        // in either caller context selects the same physical cells, primitives, and callees.
        // Both passes share the names of interned strings. Only the emitted pass adds entries.
        let strings = env.dialog_strings.borrow().clone();
        let sim_air = lower_standalone_tree(env, t, true);
        *env.dialog_strings.borrow_mut() = strings;
        let context_error = || {
            format!(
                "@context(inherit) tree '{}' depends on the caller's context (use explicit 'me' cells or 'stack_obj')",
                t.decl.ident
            )
        };
        if sim_air.map_err(|e| e.with_context(context_error()))? != air {
            return Err(Error::at(t.decl.span, context_error()));
        }
    }
    trees_air.push_str(&air);
    Ok(())
}

pub(super) fn lower_standalone_tree(
    env: &ObjectEnv<'_>,
    t: &OwnedTree,
    checking_inherited_sim: bool,
) -> Result<String> {
    let d = &t.decl;
    let sig = &env.trees[&d.ident];
    let frame = d.frame.unwrap_or_default();
    if frame.empty && !d.body.is_empty() {
        return Err(Error::at(
            frame.span,
            format!(
                "@frame(empty) tree '{}' has no nodes, so its body must be '{{}}'",
                d.ident
            ),
        ));
    }
    let must_return = !frame.empty
        && matches!(
            d.ret,
            ReturnAbi::Bool | ReturnAbi::Value { .. } | ReturnAbi::ResultValue { .. }
        );
    if must_return && !stmts_always_return(&d.body, env, &t.owner_module, true) {
        return Err(Error::at(
            d.span,
            format!(
                "tree '{}' declares a return value and must return on all control-flow paths",
                d.ident
            ),
        ));
    }
    // The check has no source-visible Sim binding. Track any use of an implicit saved self
    // through the usual lowering, but omit the entry save itself. An inherited tree has no
    // implicit entry save and may use raw code with pinned locals or numeric branch targets.
    let sim_binding = if checking_inherited_sim {
        Some("")
    } else {
        d.sim_binding.as_deref()
    };
    let (tree_ctx, needs_self_save) = match sim_binding {
        Some(sim) => (
            TreeContext::SimOnObject {
                sim_name: sim.to_owned(),
            },
            stmts_may_clobber_stack_obj(&d.body, env, &t.owner_module, true, Some(sim), 0),
        ),
        None => (TreeContext::ObjectSelf, false),
    };
    let mut ctx = TreeLowerer::new(env, tree_ctx, &t.owner_module, needs_self_save);
    let explicit = ExplicitStackOps::default().in_stmts(&d.body, env, &t.owner_module, true);
    if checking_inherited_sim {
        ctx.track_stack_obj(explicit);
    } else {
        ctx.begin_body(explicit);
    }
    ctx.frame_locals = d.frame.and_then(|f| f.locals);
    ctx.frame_args = d.frame.and_then(|f| f.args).map_or_else(
        || {
            sig.params
                .iter()
                .filter(|p| matches!(p.slot, AbiParamSlot::Positional))
                .count()
        },
        usize::from,
    );
    match &sig.ret {
        ResolvedReturnAbi::Value { ty, temp } | ResolvedReturnAbi::ResultValue { ty, temp } => {
            ctx.value_return = Some(ValueReturnAbi {
                ty: ty.clone(),
                temp: *temp,
                // `Result<T @ temp[N]>` trees take the false exit with `return None;` (or
                // `return false;` when T is not bool), so a caller's `?` runs its own cleanups.
                allow_none: matches!(sig.ret, ResolvedReturnAbi::ResultValue { .. }),
                in_place: true,
                what: "tree",
            });
        }
        _ => {}
    }
    // Temp-bound params are snapshotted into locals at entry (temps are shared with callees).
    let mut positional = 0usize;
    for p in &sig.params {
        match p.slot {
            AbiParamSlot::Positional => {
                ctx.bind_var(
                    p.name.clone(),
                    Binding::BhavParam {
                        index: positional,
                        ty: p.ty.clone(),
                    },
                );
                positional += 1;
            }
            AbiParamSlot::Temp(n) => {
                let local = ctx.fresh(&format!("p_{}", p.name));
                ctx.line(format!("let {local} = temp[{n}];"));
                ctx.bind_var(
                    p.name.clone(),
                    Binding::Local {
                        air_name: local,
                        ty: p.ty.clone(),
                    },
                );
            }
        }
    }
    ctx.lower_stmts(&d.body)?;
    if !frame.empty && !stmts_always_return(&d.body, env, &t.owner_module, true) {
        ctx.emit_all_cleanups()?;
        ctx.line("return true;".to_owned());
    }
    let mut air = String::new();
    write_framed_tree(
        &mut air, ctx, sig.id, &d.label, &frame, positional, &d.ident,
    )?;
    Ok(air)
}

/// ` (a, b)` for the `.air` header fields `parts`, or nothing when there are none.
pub(super) fn air_header(parts: &[String]) -> String {
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// Finishes `ctx` and writes it as tree `id` with the `.air` header of its `@frame(...)`
/// (`frame` is the default when there is none), checked against its `positional` parameter
/// count. `name` is the tree's name in diagnostics.
pub(super) fn write_framed_tree(
    trees_air: &mut String,
    ctx: TreeLowerer<'_>,
    id: u16,
    label: &str,
    frame: &TreeFrame,
    positional: usize,
    name: &str,
) -> Result<()> {
    let label = escape_str(label);
    let mut header: Vec<String> = Vec::new();
    match frame.args {
        Some(args) if usize::from(args) < positional => {
            return Err(Error::at(
                frame.span,
                format!(
                    "@frame(args: {args}) on tree '{name}' is below its {positional} positional parameter(s)"
                ),
            ));
        }
        Some(args) => header.push(format!("args: {args}")),
        None if positional > 0 => header.push(format!("args: {positional}")),
        None => {}
    }
    if let Some(locals) = frame.locals {
        header.push(format!("locals: {locals}"));
    } else {
        ctx.push_frame_floor(&mut header);
    }
    if let Some(format) = frame.format {
        if !(0x8000..=0x8002).contains(&format) {
            return Err(Error::at(
                frame.span,
                format!(
                    "@frame(format: {format:#06x}) on tree '{name}' is not a supported BHAV format (0x8000, 0x8001, or 0x8002)"
                ),
            ));
        }
        // Formats 0x8000 and 0x8001 give a call frame at most four arguments.
        let args = frame.args.map_or(positional, usize::from);
        if format != 0x8002 && args > 4 {
            return Err(Error::at(
                frame.span,
                format!(
                    "@frame(format: {format:#06x}) on tree '{name}' allows at most 4 arguments, not {args}"
                ),
            ));
        }
        header.push(format!("format: {format:#06x}"));
    }
    if let Some(revision) = frame.revision {
        header.push(format!("revision: {revision}"));
    }
    if let Some(kind) = frame.kind {
        header.push(format!("kind: {kind}"));
    }
    let event_local = ctx.event_local;
    if let (Some((espan, _)), Some(sleep)) = (event_local, ctx.sleep_span) {
        return Err(Error::at(
            espan,
            format!(
                "animate(event_local:) is a raw animate op, and a tree cannot mix raw ops with sleep() (line {}): use 'local:' with a variable, or move the sleep to another tree",
                sleep.line
            ),
        ));
    }
    // A 0-node tree keeps no entry code either (temp-bound parameter snapshots).
    let body = if frame.empty {
        String::new()
    } else {
        ctx.finish()?
    };
    if let (Some((espan, slot)), Some(locals)) = (event_local, frame.locals) {
        // `.air` grows the frame to fit the named locals (after an explicit `local[0]` pin).
        let used = count_frame_locals(&body);
        if used > usize::from(locals) || usize::from(slot) < used {
            return Err(Error::at(
                espan,
                format!(
                    "animate(event_local: {slot}) must lie outside the frame, but tree '{name}' needs {used} local(s) and declares @frame(locals: {locals})"
                ),
            ));
        }
    }
    if header.is_empty() {
        let _ = writeln!(trees_air, "tree {id} \"{label}\" {{");
    } else {
        let _ = writeln!(
            trees_air,
            "tree {id} \"{label}\" ({}) {{",
            header.join(", ")
        );
    }
    trees_air.push_str(&body);
    trees_air.push_str("}\n\n");
    Ok(())
}

/// The frame locals `.air` allocates for a lowered tree body: one per `let` / `local` name,
/// after every explicit local (`local[N]`, `scope(25, N)`). Read from the body's tokens, so
/// comments and strings name nothing. This only words the diagnostic: the `.air` compiler
/// rejects a tree whose locals outgrow its `@frame(locals: K)`, and `K <= N` is checked when
/// lowering, so a slot it misses (a raw primitive's numeric local operand) is still caught.
pub(super) fn count_frame_locals(body: &str) -> usize {
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

pub(super) fn emit_guard_tree(
    air: &mut String,
    env: &ObjectEnv<'_>,
    owner_module: &str,
    tree_id: u16,
    label: &str,
    guard: &GuardDecl,
    func_params: &[crate::v2::ast::FnParam],
) -> Result<()> {
    let needs_self_save = match &guard.kind {
        GuardBody::Expr(e) => {
            expr_may_clobber_stack_obj(e, env, owner_module, true, Some(&guard.sim_binding), 0)
        }
        GuardBody::Block(stmts) => {
            stmts_may_clobber_stack_obj(stmts, env, owner_module, true, Some(&guard.sim_binding), 0)
        }
    };
    let mut ctx = TreeLowerer::new(
        env,
        TreeContext::SimOnObject {
            sim_name: guard.sim_binding.clone(),
        },
        owner_module,
        needs_self_save,
    );
    let mut stack_ops = ExplicitStackOps::default();
    ctx.begin_body(match &guard.kind {
        GuardBody::Expr(e) => stack_ops.in_expr(e, env, owner_module, true),
        GuardBody::Block(stmts) => stack_ops.in_stmts(stmts, env, owner_module, true),
    });
    if let Some(ref auto_name) = guard.autonomous_binding {
        ctx.bind_var(auto_name.clone(), Binding::AutonomousFlag);
    }
    ctx.frame_args = if guard.autonomous_binding.is_some() {
        1
    } else {
        func_params.len()
    };
    for (pidx, fp) in func_params.iter().enumerate() {
        ctx.bind_func_param(&fp.name, pidx, &fp.ty, fp.span)?;
    }
    match &guard.kind {
        GuardBody::Expr(expr) => {
            ctx.emit_return_bool_expr(expr)?;
        }
        GuardBody::Block(stmts) => {
            ctx.lower_stmts(stmts)?;
            if !stmts_always_return(stmts, env, owner_module, true) {
                ctx.emit_all_cleanups()?;
                ctx.line("return true;".to_owned());
            }
        }
    }
    let arg_count = if guard.autonomous_binding.is_some() {
        1
    } else {
        func_params.len()
    };
    let mut header = Vec::new();
    if arg_count > 0 {
        header.push(format!("args: {arg_count}"));
    }
    ctx.push_frame_floor(&mut header);
    let _ = writeln!(air, "tree {tree_id} \"{label}\"{} {{", air_header(&header));
    air.push_str(&ctx.finish()?);
    air.push_str("}\n\n");
    Ok(())
}
