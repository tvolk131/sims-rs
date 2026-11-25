//! Tree emission: standalone trees, guard trees, `.air` headers and the framed tree writer (frame
//! layout, slot spans).

use super::*;

/// Lowers and emits one standalone `#[bhav] fn` member as a real (non-inlined) BHAV.
pub(super) fn emit_standalone_tree(
    trees_air: &mut String,
    env: &ObjectEnv<'_>,
    t: &OwnedTree,
) -> Result<()> {
    let mut lowered = ContextProbe::default();
    let air = lower_standalone_tree(env, t, false, Some(&mut lowered))?;
    if t.decl.inherit_context {
        // A private call inherits the runner and the current stack object. Check that lowering
        // in either caller context selects the same physical cells, primitives, and callees.
        // Both passes share the names of interned strings. Only the emitted pass adds entries.
        let strings = env.dialog_strings.borrow().clone();
        let mut sim_lowered = ContextProbe::default();
        let sim_air = lower_standalone_tree(env, t, true, Some(&mut sim_lowered));
        *env.dialog_strings.borrow_mut() = strings;
        let context_error = || {
            format!(
                "'context = inherit' fn '{}' depends on the caller's context (use explicit 'me' cells or 'stack_obj')",
                t.decl.ident
            )
        };
        // The passes may name the same local differently (a cursor save is `__self_obj` in one
        // and `__saved_stack_obj` in the other), so the texts are compared with every declared
        // local renamed by declaration order, which keeps each local's frame word.
        let sim_air = sim_air.map_err(|e| e.with_context(context_error()))?;
        if canonical_locals(&sim_air) != canonical_locals(&air) {
            let span = differing_statement(&lowered, &sim_lowered).unwrap_or(t.decl.span);
            return Err(Error::at(span, context_error()));
        }
    }
    trees_air.push_str(&air);
    Ok(())
}

/// One lowering pass of a standalone tree, before the backend passes: its body text and where
/// each of the tree's own statements starts in it.
#[derive(Default)]
pub(super) struct ContextProbe {
    text: String,
    marks: Vec<(usize, Span)>,
}

/// `air` with every local a `let` or `local` declares renamed `__ctx_local_<N>` by declaration
/// order. `.air` gives named locals their frame words in that order, so two texts that agree
/// after this rename assemble to the same instructions. Text that does not lex is returned as
/// it is.
pub(super) fn canonical_locals(air: &str) -> String {
    use crate::lexer::TokenKind as T;
    let Ok(tokens) = crate::lexer::tokenize(air) else {
        return air.to_owned();
    };
    let mut rename: BTreeMap<String, String> = BTreeMap::new();
    for pair in tokens.windows(2) {
        if let (T::Ident(kw), T::Ident(name)) = (&pair[0].kind, &pair[1].kind)
            && (kw == "let" || kw == "local")
            && !rename.contains_key(name)
        {
            let canonical = format!("__ctx_local_{}", rename.len());
            rename.insert(name.clone(), canonical);
        }
    }
    mir::rename_idents(air, &rename)
}

/// The tree statement whose lowering first differs between the two passes: the last statement
/// to start at or before the first line where their canonical body texts differ. `None` when
/// the bodies agree (the difference is elsewhere) or no statement precedes it.
fn differing_statement(a: &ContextProbe, b: &ContextProbe) -> Option<Span> {
    // Before `finish` neither pass declares its entry save, which each names its own way.
    let entry = BTreeMap::from([
        (mir::SELF_OBJ.to_owned(), "__ctx_entry_obj".to_owned()),
        (
            mir::SAVED_STACK_OBJ.to_owned(),
            "__ctx_entry_obj".to_owned(),
        ),
    ]);
    let canonical = |text: &str| canonical_locals(&mir::rename_idents(text, &entry));
    let (ca, cb) = (canonical(&a.text), canonical(&b.text));
    let line = ca
        .lines()
        .zip(cb.lines())
        .position(|(x, y)| x != y)
        .or_else(|| {
            (ca.lines().count() != cb.lines().count())
                .then(|| ca.lines().count().min(cb.lines().count()))
        })?;
    // The pass whose text still has that line locates it.
    let probe = if a.text.lines().count() > line { a } else { b };
    let offset: usize = probe.text.lines().take(line).map(|l| l.len() + 1).sum();
    probe
        .marks
        .iter()
        .filter(|(mark, _)| *mark <= offset)
        .max_by_key(|(mark, _)| *mark)
        .map(|(_, span)| *span)
}

pub(super) fn lower_standalone_tree(
    env: &ObjectEnv<'_>,
    t: &OwnedTree,
    checking_inherited_sim: bool,
    probe: Option<&mut ContextProbe>,
) -> Result<String> {
    let d = &t.decl;
    let sig = &env.trees[&d.ident];
    let frame = d.frame.unwrap_or_default();
    if frame.empty && !d.body.is_empty() {
        return Err(Error::at(
            frame.span,
            format!(
                "'empty' fn '{}' has no nodes, so its body must be '{{}}'",
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
    ctx.inherit_tree = t.decl.inherit_context;
    ctx.tree_display = Some(format!("tree '{}'", d.ident));
    let explicit = ExplicitStackOps::default().in_stmts(&d.body, env, &t.owner_module, true);
    ctx.reassigns_stack_obj =
        ExplicitStackOps::for_tree().in_stmts(&d.body, env, &t.owner_module, true);
    if checking_inherited_sim {
        ctx.track_stack_obj(explicit);
        ctx.inherited_entry_save = true;
    } else {
        ctx.begin_body(explicit);
    }
    ctx.frame_locals = d.frame.and_then(|f| f.locals);
    ctx.frame_format = d.frame.and_then(|f| f.format);
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
                // `Result<T>` with `#[ret(temp = N)]` trees take the false exit with `return None;` (or
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
    if let Some(probe) = probe {
        probe.text = ctx.em.text().to_owned();
        probe.marks = std::mem::take(&mut ctx.stmt_marks);
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

/// Finishes `ctx` and writes it as tree `id` with the `.air` header of its `#[frame(...)]`
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
                    "'args = {args}' on fn '{name}' is below its {positional} positional parameter(s)"
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
                    "'format = {format:#06x}' on fn '{name}' is not a supported BHAV format (0x8000, 0x8001, or 0x8002)"
                ),
            ));
        }
        // Formats 0x8000 and 0x8001 give a call frame at most four arguments.
        let args = frame.args.map_or(positional, usize::from);
        if format != 0x8002 && args > 4 {
            return Err(Error::at(
                frame.span,
                format!(
                    "'format = {format:#06x}' on fn '{name}' allows at most 4 arguments, not {args}"
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
        let used = mir::count_frame_locals(&body);
        if used > usize::from(locals) || usize::from(slot) < used {
            return Err(Error::at(
                espan,
                format!(
                    "animate(event_local: {slot}) must lie outside the frame, but tree '{name}' needs {used} local(s) and declares 'locals = {locals}'"
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

pub(super) fn emit_guard_tree(
    air: &mut String,
    env: &ObjectEnv<'_>,
    owner_module: &str,
    tree_id: u16,
    label: &str,
    guard: &GuardDecl,
    func_params: &[crate::v2::ast::FnParam],
    object_reachable: bool,
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
    ctx.object_reachable = object_reachable;
    let mut stack_ops = ExplicitStackOps::default();
    ctx.reassigns_stack_obj = match &guard.kind {
        GuardBody::Expr(e) => ExplicitStackOps::for_tree().in_expr(e, env, owner_module, true),
        GuardBody::Block(stmts) => {
            ExplicitStackOps::for_tree().in_stmts(stmts, env, owner_module, true)
        }
    };
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
