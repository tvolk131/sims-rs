//! Statement lowering: the statement dispatcher, range `for`, `let Some`, expression statements and
//! bitflag mutation.

use super::*;

impl<'a> TreeLowerer<'a> {
    pub(super) fn lower_stmts(&mut self, stmts: &[V2Stmt]) -> Result<()> {
        for stmt in stmts {
            self.lower_stmt(stmt)?;
        }
        Ok(())
    }

    pub(super) fn lower_stmt(&mut self, stmt: &V2Stmt) -> Result<()> {
        if let Some(split) = self.split_inline_specialization(stmt)? {
            return self.lower_stmt(&split);
        }
        // A binding allocated after its statement's nested bodies (`let Some`'s `else`) keeps its
        // own statement as its declaration origin, so the span is restored on the way out.
        let outer = self
            .stmt_span
            .replace(crate::v2::specialize::stmt_span(stmt));
        let lowered = self.lower_stmt_at(stmt);
        self.stmt_span = outer;
        lowered
    }

    fn lower_stmt_at(&mut self, stmt: &V2Stmt) -> Result<()> {
        self.note_decl(stmt);
        if self.inline_call_stack.is_empty() {
            self.stmt_marks
                .push((self.em.mark(), crate::v2::specialize::stmt_span(stmt)));
        }
        match stmt {
            V2Stmt::Let {
                span,
                name,
                ty,
                init,
                slot,
            } => {
                let air_name = self.local_air_name(name, *slot, *span)?;
                let inferred_ty = if let V2Expr::Match {
                    span: mspan,
                    scrutinee,
                    arms,
                } = init
                {
                    // The arms write the binding directly (`let x;` emits no node).
                    self.line(format!("let {air_name};"));
                    self.lower_match_expr(*mspan, scrutinee, arms, Some(&air_name), true)?
                        .1
                } else if let V2Expr::If {
                    span: ispan,
                    cond,
                    then_value,
                    else_value,
                } = init
                {
                    // The branches write the binding directly.
                    self.line(format!("let {air_name};"));
                    self.lower_if_expr(*ispan, cond, then_value, else_value, Some(&air_name), true)?
                        .1
                } else if let Some(ty) = self.try_lower_inline_into(init, &air_name)? {
                    ty
                } else if is_compound_arith_or_neg(init) {
                    self.lower_arith_into_dest(init, &air_name, true)?
                } else if let Some((val_air, inferred)) = self.try_lower_callable_value_raw(init)? {
                    self.emit_let(&air_name, &val_air);
                    inferred
                } else {
                    let (val_air, inferred) = self.lower_value_expr(init)?;
                    self.emit_let(&air_name, &val_air);
                    inferred
                };
                let final_ty = if let Some(ann) = ty {
                    let expected = self.env.resolve_value_type(
                        ann,
                        &self.owner_module,
                        self.in_object_scope,
                        *span,
                    )?;
                    if !types_compatible_for_assign(&expected, &inferred_ty) {
                        return Err(Error::at(
                            *span,
                            format!(
                                "cannot initialize '{}' variable '{name}' from {} expression",
                                self.env.display_value_type(&expected),
                                self.env.display_value_type(&inferred_ty)
                            ),
                        ));
                    }
                    expected
                } else {
                    inferred_ty
                };
                self.bind_var(
                    name.clone(),
                    Binding::Local {
                        air_name,
                        ty: final_ty,
                    },
                );
                Ok(())
            }
            V2Stmt::LetSome {
                span,
                name,
                ty,
                init,
                else_body,
            } => self.lower_let_some(*span, name, ty.as_ref(), init, else_body),
            V2Stmt::LetUninit {
                span,
                name,
                ty,
                slot,
            } => {
                let resolved = self.env.resolve_value_type(
                    ty,
                    &self.owner_module,
                    self.in_object_scope,
                    *span,
                )?;
                let air_name = self.local_air_name(name, *slot, *span)?;
                self.line(format!("let {air_name};"));
                self.bind_var(
                    name.clone(),
                    Binding::Local {
                        air_name,
                        ty: resolved,
                    },
                );
                Ok(())
            }
            V2Stmt::Animate {
                span,
                args,
                event,
                body,
            } => {
                // A user `fn`/`#[bhav] fn`/`extern` named `animate` wins over the builtin.
                let callee = V2Expr::Ident(*span, "animate".to_owned());
                if self.user_callable_named(&callee, *span)? {
                    if event.is_some() {
                        return Err(Error::at(
                            *span,
                            "'animate(...) |event| { ... }' is the builtin form, but a user function named 'animate' shadows it",
                        ));
                    }
                    return self.lower_stmt(&V2Stmt::Expr {
                        span: *span,
                        expr: V2Expr::Call {
                            span: *span,
                            callee: Box::new(callee),
                            type_args: Vec::new(),
                            args: args.clone(),
                        },
                    });
                }
                self.lower_animate_stmt(*span, args, event.as_deref(), body)
            }
            V2Stmt::Assign {
                span,
                target,
                op,
                value,
            } => {
                self.reject_read_only_sim_slot(target)?;
                let lval = self.resolve_lvalue(target)?;
                let target_ty = lval.value_type().clone();
                // Handle bitflags `flag_set` / `flag_clear` explicitly (1-based bit index!).
                if matches!(op, V2AssignOp::FlagSet | V2AssignOp::FlagClear)
                    && let ValueType::Bitflags(ref key) = target_ty
                {
                    let Some((_, Some(bit))) = self.try_eval_const_bitflag(value, key)? else {
                        return Err(Error::at(
                            value.span(),
                            format!(
                                "'flag_set'/'flag_clear' on bitflags '{}' requires a single flag constant (use '.set(...)' / '.clear(...)' for masks)",
                                self.env.display_value_type(&target_ty)
                            ),
                        ));
                    };
                    return self.emit_lvalue_assign(&lval, *op, &bit.to_string());
                }
                // Compact in-place compound arithmetic lowering when target is Direct, alias-free,
                // side-effect-free, and cannot trap partway through!
                if *op == V2AssignOp::Set
                    && let LValue::Direct { ref air, .. } = lval
                    && is_compound_arith_or_neg(value)
                    && self.can_lower_arith_in_place(air, value, false)
                {
                    let val_ty = self.lower_arith_into_dest(value, air, false)?;
                    if !types_compatible_for_assign(&target_ty, &val_ty) {
                        return Err(Error::at(
                            *span,
                            format!(
                                "cannot assign '{}' expression to '{}' target",
                                self.env.display_value_type(&val_ty),
                                self.env.display_value_type(&target_ty)
                            ),
                        ));
                    }
                    return Ok(());
                }
                // A value-returning call into a plain (non-foreign) target reads `temp[N]` directly.
                let raw_call = if matches!(lval, LValue::Direct { .. }) {
                    self.try_lower_callable_value_raw(value)?
                } else {
                    None
                };
                let (val_air, val_ty) = match raw_call {
                    Some(r) => r,
                    None => self.lower_value_expr(value)?,
                };
                if *op != V2AssignOp::Set {
                    match &target_ty {
                        ValueType::Bool => {
                            return Err(Error::at(
                                *span,
                                "compound assignment is not allowed on a bool target",
                            ));
                        }
                        ValueType::Enum(_) | ValueType::ObjectRef(_) => {
                            return Err(Error::at(
                                *span,
                                format!(
                                    "compound assignment is not allowed on '{}' target",
                                    self.env.display_value_type(&target_ty)
                                ),
                            ));
                        }
                        ValueType::Bitflags(_) => {
                            if !matches!(
                                op,
                                V2AssignOp::BitAnd | V2AssignOp::BitOr | V2AssignOp::BitXor
                            ) {
                                return Err(Error::at(
                                    *span,
                                    format!(
                                        "arithmetic assignment is not allowed on bitflags '{}' (use |=, &=, ^=, .set(), or .clear())",
                                        self.env.display_value_type(&target_ty)
                                    ),
                                ));
                            }
                        }
                        ValueType::I16 | ValueType::ObjectId => {}
                    }
                }
                if !types_compatible_for_assign(&target_ty, &val_ty) {
                    return Err(Error::at(
                        *span,
                        format!(
                            "cannot assign '{}' expression to '{}' target",
                            self.env.display_value_type(&val_ty),
                            self.env.display_value_type(&target_ty)
                        ),
                    ));
                }
                self.emit_lvalue_assign(&lval, *op, &val_air)
            }
            V2Stmt::WithLock {
                span,
                lock_expr,
                body,
            } => {
                let lval = self.resolve_lvalue(lock_expr)?;
                if *lval.value_type() != ValueType::Bool {
                    return Err(Error::at(
                        *span,
                        "with lock(...) requires a bool state or &mut bool target",
                    ));
                }
                if self.held_locks.contains(&lval) {
                    return Err(Error::at(
                        *span,
                        "re-entrant 'with lock(...)' on an already-locked target in the same scope chain",
                    ));
                }
                self.emit_lvalue_assign(&lval, V2AssignOp::Set, "1")?;
                self.held_locks.push(lval.clone());
                self.push_scope(ScopeFrame {
                    cleanups: vec![CleanupAction::Unlock(lval)],
                    is_loop_body: false,
                    is_fn_root: false,
                    is_air_while: false,
                    owner_defer_id: self.current_defer_id,
                    cancel_flag: None,
                    restore_stack_on_break: false,
                    continue_step: None,
                });
                let res = self.lower_stmts(body);
                let pop_res = if stmts_always_return(
                    body,
                    self.env,
                    &self.owner_module,
                    self.in_object_scope,
                ) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
                res.and(pop_res)
            }
            V2Stmt::Defer { span, body } => {
                if self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "nested 'defer' inside another 'defer' block is not allowed",
                    ));
                }
                let saved_bindings = self.var_scopes.clone();
                let owner_module = self.owner_module.clone();
                let in_object_scope = self.in_object_scope;
                if let Some(frame) = self.cleanup_scopes.last_mut() {
                    frame.cleanups.push(CleanupAction::Defer {
                        owner_module,
                        in_object_scope,
                        body: body.clone(),
                        saved_bindings,
                    });
                }
                Ok(())
            }
            V2Stmt::Repeat {
                span,
                count,
                cancel_on_abort,
                body,
            } => {
                if *cancel_on_abort && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' requires a Sim interaction context (not available in init/main)",
                    ));
                }
                if *cancel_on_abort && self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' is not allowed inside a 'defer' block",
                    ));
                }
                let (limit_air, _) = self.lower_value_expr(count)?;
                let cancel_flag = if *cancel_on_abort {
                    let f = self.fresh("abort");
                    self.line(format!("let {f} = 0;"));
                    Some(f)
                } else {
                    None
                };
                let counter = self.fresh("rep");
                self.line(format!("let {counter} = 0;"));
                self.line(format!("while {counter} < {limit_air} {{"));
                self.em.indent();
                self.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: true,
                    is_fn_root: false,
                    is_air_while: true,
                    owner_defer_id: self.current_defer_id,
                    cancel_flag: cancel_flag.clone(),
                    restore_stack_on_break: false,
                    continue_step: Some(ContinueStep::Counter(counter.clone())),
                });
                if let Some(flag) = &cancel_flag {
                    self.line("if my.person.priority == 0 {".to_owned());
                    self.em.indent();
                    self.emit_store(flag, ":=", "1");
                    self.line("break;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                }
                let res = self.lower_stmts(body);
                let pop_res = self.pop_scope_and_run_cleanups();
                self.line(format!("{counter} += 1;"));
                self.em.dedent();
                self.line("}".to_owned());
                res.and(pop_res)?;
                if let Some(inner_flag) = &cancel_flag
                    && let Some((_, outer_flag)) = self.enclosing_cancel_flag()
                {
                    self.line(format!("if {inner_flag} != 0 {{"));
                    self.em.indent();
                    self.emit_store(&outer_flag, ":=", "1");
                    self.em.dedent();
                    self.line("}".to_owned());
                }
                self.emit_inline_fn_return_propagation(false)?;
                self.emit_cancel_propagation(false)?;
                Ok(())
            }
            V2Stmt::While {
                span,
                cond,
                cancel_on_abort,
                body,
            } => {
                if *cancel_on_abort && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' requires a Sim interaction context (not available in init/main)",
                    ));
                }
                if *cancel_on_abort && self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'cancel_on_abort' is not allowed inside a 'defer' block",
                    ));
                }
                let folded = self
                    .try_eval_const_bool_inner(cond, true)
                    .or_else(|| self.try_fold_literal_cond(cond));
                if folded.is_some() {
                    self.check_folded_cond(cond)?;
                }
                // A constant-false loop never runs: its body is only checked.
                if folded == Some(false) {
                    return self.check_dead_branch_as(body, true);
                }
                let cancel_flag = if *cancel_on_abort {
                    let f = self.fresh("abort");
                    self.line(format!("let {f} = 0;"));
                    Some(f)
                } else {
                    None
                };
                // `loop { }` / `while true { }` (or a literal-true comparison) lower to `.air`
                // `while true`, which has no condition node at all.
                let header = if folded == Some(true) {
                    Some("true".to_owned())
                } else if self.is_pure_simple_air_cond(cond) {
                    self.try_lower_simple_air_cond(cond)?
                } else {
                    None
                };
                if let Some(simple_cond) = header {
                    self.line(format!("while {simple_cond} {{"));
                    self.em.indent();
                    self.push_scope(ScopeFrame {
                        cleanups: Vec::new(),
                        is_loop_body: true,
                        is_fn_root: false,
                        is_air_while: true,
                        owner_defer_id: self.current_defer_id,
                        cancel_flag: cancel_flag.clone(),
                        restore_stack_on_break: false,
                        continue_step: None,
                    });
                    if let Some(flag) = &cancel_flag {
                        self.line("if my.person.priority == 0 {".to_owned());
                        self.em.indent();
                        self.emit_store(flag, ":=", "1");
                        self.line("break;".to_owned());
                        self.em.dedent();
                        self.line("}".to_owned());
                    }
                    let res = self.lower_stmts(body);
                    let pop_res = self.pop_scope_and_run_cleanups();
                    self.em.dedent();
                    self.line("}".to_owned());
                    res.and(pop_res)?;
                } else {
                    self.line("while true {".to_owned());
                    self.em.indent();
                    self.push_scope(ScopeFrame {
                        cleanups: Vec::new(),
                        is_loop_body: true,
                        is_fn_root: false,
                        is_air_while: true,
                        owner_defer_id: self.current_defer_id,
                        cancel_flag: cancel_flag.clone(),
                        restore_stack_on_break: false,
                        continue_step: None,
                    });
                    if let Some(flag) = &cancel_flag {
                        self.line("if my.person.priority == 0 {".to_owned());
                        self.em.indent();
                        self.emit_store(flag, ":=", "1");
                        self.line("break;".to_owned());
                        self.em.dedent();
                        self.line("}".to_owned());
                    }
                    // The condition is tested at the top of every pass. One that lowers to a
                    // single `.air` condition (a call, `!f()`) needs no local to hold it. The
                    // exit swaps the test's exits rather than inverting it, so the body starts
                    // after the test came out as the source wrote it.
                    let cond_res = match self.try_lower_simple_air_cond(cond) {
                        Ok(Some(test)) => {
                            self.line(format!("if {} {{", swap_exits(&test)));
                            Ok(())
                        }
                        Ok(None) => {
                            let cond_loc = self.fresh("wcond");
                            self.line(format!("let {cond_loc} = 0;"));
                            let r = self.emit_bool_into_local(cond, &cond_loc);
                            self.line(format!("if {cond_loc} == 0 {{"));
                            r
                        }
                        Err(e) => Err(e),
                    };
                    self.em.indent();
                    self.line("break;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                    let body_res = cond_res.and_then(|()| self.lower_stmts(body));
                    let pop_res = self.pop_scope_and_run_cleanups();
                    self.em.dedent();
                    self.line("}".to_owned());
                    body_res.and(pop_res)?;
                }
                if let Some(inner_flag) = &cancel_flag
                    && let Some((_, outer_flag)) = self.enclosing_cancel_flag()
                {
                    self.line(format!("if {inner_flag} != 0 {{"));
                    self.em.indent();
                    self.emit_store(&outer_flag, ":=", "1");
                    self.em.dedent();
                    self.line("}".to_owned());
                }
                self.emit_inline_fn_return_propagation(false)?;
                self.emit_cancel_propagation(false)?;
                Ok(())
            }
            V2Stmt::ForEach {
                span,
                binding,
                iter,
                body,
                else_body,
            } => {
                if let ForEachIter::Range {
                    start,
                    end,
                    inclusive,
                    rev,
                    temp,
                } = iter
                {
                    return self
                        .lower_range_for(binding, start, end, *inclusive, *rev, *temp, body);
                }
                if else_body.as_deref().is_some_and(stmts_exit_loop) {
                    return Err(Error::at(
                        *span,
                        "'break' or 'continue' in 'for ... else' cannot target an enclosing loop",
                    ));
                }
                let else_diverges = else_body.as_deref().is_some_and(|stmts| {
                    stmts_always_return(stmts, self.env, &self.owner_module, self.in_object_scope)
                });
                // `stack_obj` itself is the cursor when the body can use it without
                // accidentally observing or replacing the incoming frame value.
                let stack_cursor = matches!(
                    iter,
                    ForEachIter::ObjectsByGuid(_) | ForEachIter::LotSims | ForEachIter::LotObjects
                ) && !self.explicit_stack_obj
                    && self.active_inline_fn.is_none()
                    && !self.in_defer()
                    && !self.has_active_cleanups()
                    && (else_body.is_none() || else_diverges)
                    && body.iter().all(|s| self.cursor_stmt_ok(s, binding));
                // Only a loop that moves the stack object restores it: a redundant store would
                // replace the iterator's false result, which the exhaustion block may read
                // (`gosub_found_action` in raw code).
                let sim_name = match &self.tree_ctx {
                    TreeContext::SimOnObject { sim_name } => Some(sim_name.clone()),
                    TreeContext::ObjectSelf => None,
                };
                let restore_on_exhaustion = else_body.is_some()
                    && !self.explicit_stack_obj
                    && (stack_cursor
                        || stmts_may_clobber_stack_obj(
                            body,
                            self.env,
                            &self.owner_module,
                            self.in_object_scope,
                            sim_name.as_deref(),
                            0,
                        ));
                let restore_on_break = restore_on_exhaustion && !else_diverges;
                let iter_var = if stack_cursor {
                    "stack_obj".to_owned()
                } else {
                    self.fresh(&format!("v_{binding}"))
                };
                let init_line = if stack_cursor {
                    mir::StackObjOp::Set("0").text()
                } else {
                    format!("let {iter_var} = 0;")
                };
                let mut parts_root_opt = None;
                let (step, binding_ty) = match iter {
                    ForEachIter::Range { .. } => unreachable!("handled above"),
                    ForEachIter::ObjectsByGuid(guid_expr) => {
                        let guid = self.resolve_guid_expr(guid_expr)?;
                        self.line(init_line);
                        (format!("guid, {guid:#010x}"), ValueType::ObjectId)
                    }
                    ForEachIter::LotSims | ForEachIter::LotObjects => {
                        let mode = if matches!(iter, ForEachIter::LotSims) {
                            "person"
                        } else {
                            "all"
                        };
                        self.line(init_line);
                        (mode.to_owned(), ValueType::ObjectId)
                    }
                    ForEachIter::ObjectsOf { span, object_type } => {
                        let schema = self
                            .env
                            .lookup_object_schema(
                                &self.owner_module,
                                object_type,
                                self.in_object_scope,
                            )
                            .ok_or_else(|| {
                                Error::at(
                                    *span,
                                    format!(
                                        "unknown object type '{object_type}' in 'objects_of::<{object_type}>()' (import it with 'use ...::{object_type};')"
                                    ),
                                )
                            })?;
                        let guid = schema.lead_guid;
                        let key = schema.canonical_key.clone();
                        self.line(format!("let {iter_var} = 0;"));
                        (format!("guid, {guid:#010x}"), ValueType::ObjectRef(key))
                    }
                    ForEachIter::SelfParts => {
                        let root_expr = match self.tree_ctx {
                            TreeContext::ObjectSelf => {
                                let root_loc = self.fresh("parts_root");
                                self.line(format!("let {root_loc} = my.id;"));
                                root_loc
                            }
                            TreeContext::SimOnObject { .. } => {
                                if self.has_self_obj_local {
                                    "__self_obj".to_owned()
                                } else {
                                    let root_loc = self.fresh("parts_root");
                                    self.stack_op(mir::StackObjOp::Save(&root_loc));
                                    root_loc
                                }
                            }
                        };
                        self.emit_let(&iter_var, &root_expr);
                        parts_root_opt = Some(root_expr);
                        (
                            "part".to_owned(),
                            ValueType::ObjectRef(self.env.self_object_key.clone()),
                        )
                    }
                };
                let step = format!("set_to_next({step}, dest: {iter_var})");
                if else_body.is_some() {
                    self.line("while true {".to_owned());
                } else {
                    self.line(format!("while {step} {{"));
                }
                self.em.indent();
                if let Some(root_expr) = &parts_root_opt
                    && else_body.is_none()
                {
                    self.line(format!("if {iter_var} == {root_expr} {{"));
                    self.em.indent();
                    self.line("break;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                }
                self.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: true,
                    is_fn_root: false,
                    is_air_while: true,
                    owner_defer_id: self.current_defer_id,
                    cancel_flag: None,
                    restore_stack_on_break: restore_on_break,
                    continue_step: None,
                });
                if else_body.is_some() {
                    // A successful iteration goes back to the test. Falling through the test
                    // (or wrapping to the root part) reaches the single exhaustion block.
                    self.line(format!("if {step} {{"));
                    self.em.indent();
                    if let Some(root_expr) = &parts_root_opt {
                        self.line(format!("if !{iter_var} == {root_expr} {{"));
                        self.em.indent();
                    }
                }
                self.bind_var(
                    binding.clone(),
                    Binding::Local {
                        air_name: iter_var,
                        ty: binding_ty,
                    },
                );
                let body_start = self.em.mark();
                let res = self.lower_stmts(body);
                let pop_res = self.pop_scope_and_run_cleanups();
                res.and(pop_res)?;
                // The exhaustion block sees the iterator's false result, which a restore store
                // would replace: restore only if an iteration may leave the stack object moved
                // (a field read through `item` moves it back itself).
                let restore_line = mir::StackObjOp::Set(if self.has_self_obj_local {
                    mir::SELF_OBJ
                } else {
                    mir::SAVED_STACK_OBJ
                })
                .text();
                let restore_on_exhaustion = restore_on_exhaustion
                    && (stack_cursor
                        || super::mir::body_may_leave_stack_obj_moved(
                            self.em.since(body_start),
                            &restore_line,
                        ));
                if let Some(else_body) = else_body {
                    self.line("continue;".to_owned());
                    if parts_root_opt.is_some() {
                        self.em.dedent();
                        self.line("}".to_owned());
                    }
                    self.em.dedent();
                    self.line("}".to_owned());
                    if restore_on_exhaustion {
                        self.emit_entry_stack_restore();
                    }
                    // Inline returns and cancellation still use the emitted AIR loop to
                    // propagate outwards. Source loop exits here were rejected above.
                    self.push_scope(ScopeFrame {
                        is_air_while: true,
                        ..ScopeFrame::block()
                    });
                    let res = self.lower_stmts(else_body);
                    let pop_res = self.pop_scope_and_run_cleanups();
                    res.and(pop_res)?;
                    self.line("break;".to_owned());
                }
                self.em.dedent();
                self.line("}".to_owned());
                if else_body.is_none() || (restore_on_exhaustion && else_diverges) {
                    self.emit_restore_stack_obj(None);
                }
                self.emit_inline_fn_return_propagation(restore_on_break)?;
                self.emit_cancel_propagation(restore_on_break)?;
                Ok(())
            }
            V2Stmt::If {
                span: _,
                cond,
                then_body,
                else_body,
            } => {
                // Constant-fold literal boolean conditions (e.g. from `bool` params) and literal
                // comparisons (`kind == Menu::Meal` with a constant helper argument). The
                // untaken branch is still lowered (and discarded) so it stays checked.
                if let Some(const_b) = self
                    .try_eval_const_bool(cond)
                    .or_else(|| self.try_fold_literal_cond(cond))
                {
                    self.check_folded_cond(cond)?;
                    let (chosen, dead) = if const_b {
                        (then_body, else_body)
                    } else {
                        (else_body, then_body)
                    };
                    self.check_dead_branch(dead)?;
                    self.push_scope(ScopeFrame::block());
                    let r = self.lower_stmts(chosen);
                    let p = self.pop_scope_and_run_cleanups();
                    return r.and(p);
                }

                // `if [!]obj.run_functional(..)` branches on the primitive itself; each arm
                // restores `stack_obj` first.
                if let Some((negated, receiver, args, rspan)) = run_functional_test(cond) {
                    let PreparedFunctional {
                        prim,
                        saved_stack_obj,
                        ..
                    } = self.prepare_run_functional(receiver, args, rspan)?;
                    let bang = if negated { "!" } else { "" };
                    self.line(format!("if {bang}{prim} {{"));
                    self.em.indent();
                    self.emit_restore_stack_obj(saved_stack_obj.as_deref());
                    self.push_scope(ScopeFrame::block());
                    let r1 = self.lower_stmts(then_body);
                    let p1 = self.pop_scope_and_run_cleanups();
                    self.em.dedent();
                    let mark = self.em.mark();
                    self.line("} else {".to_owned());
                    self.em.indent();
                    let restore_at = self.em.mark();
                    self.emit_restore_stack_obj(saved_stack_obj.as_deref());
                    let restored = self.em.mark() != restore_at;
                    self.push_scope(ScopeFrame::block());
                    let r2 = self.lower_stmts(else_body);
                    let p2 = self.pop_scope_and_run_cleanups();
                    self.em.dedent();
                    if else_body.is_empty() && !restored {
                        self.em.truncate(mark);
                    }
                    self.line("}".to_owned());
                    return r1.and(p1).and(r2).and(p2);
                }

                if self.try_lower_inline_branch(cond, then_body, else_body, false)? {
                    return Ok(());
                }

                if self.try_lower_short_circuit_if(
                    cond,
                    ArmBody::Stmts(then_body),
                    ArmBody::Stmts(else_body),
                    None,
                )? {
                    return Ok(());
                }

                let cond_text = if let Some(simple) = self.try_lower_simple_air_cond(cond)? {
                    simple
                } else {
                    let c = self.fresh("cond");
                    self.line(format!("let {c} = 0;"));
                    self.emit_bool_into_local(cond, &c)?;
                    format!("{c} != 0")
                };

                self.line(format!("if {cond_text} {{"));
                self.em.indent();
                self.push_scope(ScopeFrame::block());
                let r1 = self.lower_stmts(then_body);
                let p1 = self.pop_scope_and_run_cleanups();
                self.em.dedent();
                if else_body.is_empty() {
                    self.line("}".to_owned());
                    r1.and(p1)
                } else {
                    self.line("} else {".to_owned());
                    self.em.indent();
                    self.push_scope(ScopeFrame::block());
                    let r2 = self.lower_stmts(else_body);
                    let p2 = self.pop_scope_and_run_cleanups();
                    self.em.dedent();
                    self.line("}".to_owned());
                    r1.and(p1).and(r2).and(p2)
                }
            }
            V2Stmt::Match {
                span,
                scrutinee,
                arms,
            } => self.lower_match(*span, scrutinee, arms),
            V2Stmt::Break { span } => {
                self.emit_loop_cleanups(*span, false)?;
                self.line("break;".to_owned());
                Ok(())
            }
            V2Stmt::Continue { span } => {
                self.emit_loop_cleanups(*span, true)?;
                self.line("continue;".to_owned());
                Ok(())
            }
            V2Stmt::Return { span, value } => {
                if let Some(active_fn) = self.active_inline_fn.clone() {
                    if active_fn.owner_defer_id != self.current_defer_id {
                        return Err(Error::at(
                            *span,
                            "'return' cannot exit out of a 'defer' block",
                        ));
                    }
                    let ret_val = match active_fn.target {
                        InlineReturnTarget::Void => String::new(),
                        InlineReturnTarget::Store(dest) => dest,
                        InlineReturnTarget::Branch { arms, break_after } => {
                            let expr = value.as_ref().ok_or_else(|| {
                                Error::at(
                                    *span,
                                    "helper function with return type must return a value",
                                )
                            })?;
                            return self.emit_inline_bool_edges(
                                expr,
                                arms.terminal,
                                &mut |c, edge| {
                                    c.emit_all_cleanups()?;
                                    let arm = if edge { &arms.on_true } else { &arms.on_false };
                                    for line in arm.lines() {
                                        c.line(line.to_owned());
                                    }
                                    if break_after {
                                        c.line("break;".to_owned());
                                    }
                                    Ok(())
                                },
                            );
                        }
                    };
                    match (active_fn.ret_kind, value) {
                        (FnReturnKind::Void, Some(_)) => {
                            return Err(Error::at(
                                *span,
                                "void helper function cannot return a value",
                            ));
                        }
                        (FnReturnKind::Bool | FnReturnKind::Value(_), None) => {
                            return Err(Error::at(
                                *span,
                                "helper function with return type must return a value",
                            ));
                        }
                        (FnReturnKind::Void, None) => {}
                        (FnReturnKind::Bool, Some(expr)) => {
                            self.emit_bool_into_local(expr, &ret_val)?;
                        }
                        (FnReturnKind::Value(expected_ty), Some(expr)) => {
                            if is_compound_arith_or_neg(expr)
                                && self.can_lower_arith_in_place(&ret_val, expr, true)
                            {
                                let val_ty = self.lower_arith_into_dest(expr, &ret_val, false)?;
                                if !types_compatible_for_assign(&expected_ty, &val_ty) {
                                    return Err(Error::at(
                                        *span,
                                        format!(
                                            "helper function returning '{}' cannot return '{}' expression",
                                            self.env.display_value_type(&expected_ty),
                                            self.env.display_value_type(&val_ty)
                                        ),
                                    ));
                                }
                            } else {
                                let (val_air, val_ty) = self.lower_value_expr(expr)?;
                                if !types_compatible_for_assign(&expected_ty, &val_ty) {
                                    return Err(Error::at(
                                        *span,
                                        format!(
                                            "helper function returning '{}' cannot return '{}' expression",
                                            self.env.display_value_type(&expected_ty),
                                            self.env.display_value_type(&val_ty)
                                        ),
                                    ));
                                }
                                self.emit_store(&ret_val, ":=", &val_air);
                            }
                        }
                    }
                    self.emit_all_cleanups()?;
                    if let Some(ret_flag) = active_fn.ret_flag {
                        self.emit_store(&ret_flag, ":=", "1");
                    }
                    self.line("break;".to_owned());
                    return Ok(());
                }
                if self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'return' cannot exit out of a 'defer' block",
                    ));
                }

                // Value-returning `functional kind(...) -> T` tree / `#[bhav] fn .. -> T` with `#[ret(temp = N)]`:
                // `return None;` / `return false;` (functionals, non-bool T) exits with `return false;`
                // `return Some(expr);` / `return expr;` writes `temp[N] := val;` and exits `return true;`
                if let Some(abi) = self.value_return.clone() {
                    return self.lower_value_return(*span, value.as_ref(), &abi);
                }
                match value {
                    None => {
                        self.emit_all_cleanups()?;
                        self.line("return true;".to_owned());
                        Ok(())
                    }
                    Some(expr) => {
                        if self.try_lower_inline_branch(
                            expr,
                            &[V2Stmt::Return {
                                span: *span,
                                value: Some(V2Expr::Bool(*span, true)),
                            }],
                            &[V2Stmt::Return {
                                span: *span,
                                value: Some(V2Expr::Bool(*span, false)),
                            }],
                            true,
                        )? {
                            return Ok(());
                        }
                        if !self.has_active_cleanups() {
                            self.emit_return_bool_expr(expr)
                        } else {
                            // Evaluate return expression BEFORE running cleanups (so cleanups cannot
                            // clobber values read by the return expression), then run LIFO cleanups!
                            if let Some(const_b) = self.try_eval_const_bool(expr) {
                                self.emit_all_cleanups()?;
                                self.line(if const_b {
                                    "return true;".to_owned()
                                } else {
                                    "return false;".to_owned()
                                });
                                Ok(())
                            } else {
                                let ret_loc = self.fresh("ret");
                                self.line(format!("let {ret_loc} = 0;"));
                                self.emit_bool_into_local(expr, &ret_loc)?;
                                self.emit_all_cleanups()?;
                                self.line(format!("if {ret_loc} != 0 {{"));
                                self.em.indent();
                                self.line("return true;".to_owned());
                                self.em.dedent();
                                self.line("} else {".to_owned());
                                self.em.indent();
                                self.line("return false;".to_owned());
                                self.em.dedent();
                                self.line("}".to_owned());
                                Ok(())
                            }
                        }
                    }
                }
            }
            V2Stmt::ReturnError { span, from_try } => {
                // A written `return error;` with `error` bound as a local/param/i16 or bool param
                // keeps its pre-v2-interop meaning (returns that value). `cond?;` always errors.
                if !*from_try
                    && (self.lookup_var("error").is_some()
                        || self.env.i16_params.contains_key("error")
                        || self.env.bool_params.contains_key("error"))
                {
                    return self.lower_stmt(&V2Stmt::Return {
                        span: *span,
                        value: Some(V2Expr::Ident(*span, "error".to_owned())),
                    });
                }
                if self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'return error;' cannot exit out of a 'defer' block",
                    ));
                }
                // Exits the whole BHAV (even from an inlined helper), so every active cleanup runs.
                self.emit_cleanups_down_to(0)?;
                self.line("return error;".to_owned());
                Ok(())
            }
            V2Stmt::Expr { span: _, expr } => self.lower_expr_stmt(expr),
            V2Stmt::RawAir { span, code } => {
                self.raw_blocks += 1;
                // Discarded code emits nothing, so only emitted raw blocks count.
                if self.checking_discarded == 0 {
                    self.raw_span.get_or_insert(*span);
                    self.keep_layout = true;
                    if self.pin_span.is_some() {
                        return Err(Error::at(*span, RAW_WITH_PIN));
                    }
                }
                if self.in_defer() {
                    return Err(Error::at(
                        *span,
                        "'raw { ... }' is not allowed inside a 'defer' block",
                    ));
                }
                if self.checking_discarded == 0 {
                    for name in raw_reserved_declarations(code) {
                        self.raw_reserved.push((name, *span));
                    }
                }
                let prelude = self.collect_raw_consts(code, *span)?;
                let facts = super::raw_facts::raw_facts(code, Some(&prelude));
                if self.checking_discarded == 0 && facts.formats_text {
                    self.raw_formats_text = true;
                }
                if self.checking_discarded == 0
                    && self.loop_exit_has_work(
                        facts.breaks_enclosing_loop,
                        facts.continues_enclosing_loop,
                    )
                {
                    return Err(Error::at(
                        *span,
                        "'raw { ... }' cannot break/continue out of a loop whose exits do work (a counter step, or restoring the stack object a 'for' moved): use 'break'/'continue' outside the raw block",
                    ));
                }
                if self.has_any_active_cleanups()
                    || !self.inline_call_stack.is_empty()
                    || self.enclosing_cancel_flag().is_some()
                {
                    let wrapped = format!("tree 4096 \"__raw_check\" {{\n{prelude}{code}\n}}\n");
                    let parsed = crate::parser::parse_module(&wrapped).map_err(|e| {
                        Error::at(
                            *span,
                            format!("invalid .air syntax in raw block: {}", e.message),
                        )
                    })?;
                    let stmts = match parsed.items.first() {
                        Some(crate::ast::Item::Tree(t)) => &t.body,
                        _ => unreachable!("synthetic wrapper always has one tree"),
                    };
                    let mut defined_labels = BTreeSet::new();
                    collect_raw_air_labels(stmts, &mut defined_labels);
                    validate_raw_air_stmts(stmts, false, &defined_labels, *span)?;
                } else if self.checking_discarded > 0 {
                    // Discarded code is never parsed by the `.air` compiler, so check it here.
                    let wrapped = format!("tree 4096 \"__raw_check\" {{\n{prelude}{code}\n}}\n");
                    crate::parser::parse_module(&wrapped).map_err(|e| {
                        Error::at(
                            *span,
                            format!("invalid .air syntax in raw block: {}", e.message),
                        )
                    })?;
                }
                for raw_line in code.lines() {
                    let trimmed = raw_line.trim();
                    if !trimmed.is_empty() {
                        self.line(trimmed.to_owned());
                    }
                }
                Ok(())
            }
        }
    }

    /// Counted `for` loops. The VM's `inc_lt` / `dec_gt` step the cursor with i16 wrap-around and
    /// then compare, so a range whose adjusted bounds stay inside i16 lowers to one node per
    /// iteration:
    /// - `a..b` → `i := a - 1; while i inc_lt b { ... }` (`a - 1` may wrap; still exact);
    /// - `a..=b` → the same with `b + 1` (literal `b < 32767` only);
    /// - `(a..b).rev()` → `i := b; while i dec_gt a - 1 { ... }` (literal bounds above -32768);
    /// - `(a..=b).rev()` → the same with `b + 1` (literal `b < 32767` too).
    ///
    /// Every other range (a run-time inclusive or reversed bound, or a literal one at the i16
    /// edge) uses the edge-safe form `if a < b { i := first; while true { ...; if i == last {
    /// break; } i ±= 1; } }`, which never steps past the last value. Non-literal bounds are read
    /// once, start first; the loop variable is read-only.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_range_for(
        &mut self,
        binding: &str,
        start: &V2Expr,
        end: &V2Expr,
        inclusive: bool,
        rev: bool,
        temp: Option<u8>,
        body: &[V2Stmt],
    ) -> Result<()> {
        if self.stmts_write_ident(body, binding)? {
            return Err(Error::at(
                start.span(),
                format!("the range loop variable '{binding}' is read-only"),
            ));
        }
        enum Bound {
            Lit(i16),
            Local(String),
        }
        let snapshot = |c: &mut Self, expr: &V2Expr| -> Result<Bound> {
            if let Some((air, v)) = c.try_const_i16_air_operand(expr)?
                && air.parse::<i16>().is_ok()
            {
                return Ok(Bound::Lit(v));
            }
            let (air, ty) = c.lower_value_expr(expr)?;
            if ty != ValueType::I16 {
                return Err(Error::at(
                    expr.span(),
                    format!(
                        "range bounds must be i16, found '{}'",
                        c.env.display_value_type(&ty)
                    ),
                ));
            }
            let local = c.fresh("range");
            c.emit_let(&local, &air);
            Ok(Bound::Local(local))
        };
        let lo = snapshot(self, start)?;
        let hi = snapshot(self, end)?;
        // `#[temp(N)]`: the counter is `temp[N]` itself, set by a plain store.
        let iter_var = match temp {
            Some(t) => format!("temp[{t}]"),
            None => self.fresh(&format!("v_{binding}")),
        };
        let set_iter = |c: &mut Self, value: &str| {
            if temp.is_some() {
                c.emit_store(&iter_var, ":=", value);
            } else {
                c.emit_let(&iter_var, value);
            }
        };
        let one_node = match (&lo, &hi, rev) {
            (_, _, false) if !inclusive => true,
            (_, Bound::Lit(h), false) => *h < i16::MAX,
            (Bound::Lit(l), Bound::Lit(h), true) => {
                *l > i16::MIN
                    && (if inclusive {
                        *h < i16::MAX
                    } else {
                        *h > i16::MIN
                    })
            }
            _ => false,
        };
        // Adjusts a bound by ±1 for the one-node form (wrapping, like the VM's own step).
        let adjust = |c: &mut Self, b: &Bound, delta: i16| -> String {
            match b {
                Bound::Lit(v) => v.wrapping_add(delta).to_string(),
                Bound::Local(local) => {
                    match delta {
                        0 => {}
                        1 => c.line(format!("{local} += 1;")),
                        _ => c.line(format!("{local} -= 1;")),
                    }
                    local.clone()
                }
            }
        };
        let air = |b: &Bound| match b {
            Bound::Lit(v) => v.to_string(),
            Bound::Local(l) => l.clone(),
        };
        let mut edge_step = None;
        if one_node {
            let lo_adj = adjust(self, &lo, -1);
            let hi_adj = adjust(self, &hi, i16::from(inclusive));
            if rev {
                set_iter(self, &hi_adj);
                self.line(format!("while {iter_var} dec_gt {lo_adj} {{"));
            } else {
                set_iter(self, &lo_adj);
                self.line(format!("while {iter_var} inc_lt {hi_adj} {{"));
            }
        } else {
            let (lo_air, hi_air) = (air(&lo), air(&hi));
            let cmp = if inclusive { "<=" } else { "<" };
            self.line(format!("if {lo_air} {cmp} {hi_air} {{"));
            self.em.indent();
            // Inside the guard the range is non-empty, so `b - 1` cannot wrap.
            let last_hi = if inclusive {
                hi_air
            } else {
                match &hi {
                    Bound::Lit(v) => v.wrapping_sub(1).to_string(),
                    Bound::Local(local) => {
                        self.line(format!("{local} -= 1;"));
                        local.clone()
                    }
                }
            };
            let (first, last, step) = if rev {
                (last_hi, lo_air, "-=")
            } else {
                (lo_air, last_hi, "+=")
            };
            set_iter(self, &first);
            self.line("while true {".to_owned());
            edge_step = Some(ContinueStep::Range {
                var: iter_var.clone(),
                last,
                step,
            });
        }
        self.em.indent();
        self.push_scope(ScopeFrame {
            cleanups: Vec::new(),
            is_loop_body: true,
            is_fn_root: false,
            is_air_while: true,
            owner_defer_id: self.current_defer_id,
            cancel_flag: None,
            restore_stack_on_break: false,
            continue_step: edge_step.clone(),
        });
        // A `#[temp]` counter is a read-only operand, not a frame local: a helper it is passed
        // to gets its own copy.
        let counter = if temp.is_some() {
            Binding::Operand {
                air: iter_var.clone(),
                ty: ValueType::I16,
            }
        } else {
            Binding::Local {
                air_name: iter_var.clone(),
                ty: ValueType::I16,
            }
        };
        self.bind_var(binding.to_owned(), counter);
        let (body_mark, raw_before) = (self.em.mark(), self.raw_blocks);
        let res = self.lower_stmts(body);
        let pop_res = self.pop_scope_and_run_cleanups();
        if let Some(step) = &edge_step {
            self.emit_continue_step(step);
        }
        self.em.dedent();
        self.line("}".to_owned());
        if edge_step.is_some() {
            self.em.dedent();
            self.line("}".to_owned());
        }
        res.and(pop_res)?;
        if temp.is_some() {
            self.check_temp_counter_untouched(start.span(), &iter_var, body_mark, raw_before)?;
        }
        self.emit_inline_fn_return_propagation(false)?;
        self.emit_cancel_propagation(false)?;
        Ok(())
    }

    /// A `#[temp(N)]` loop's body (its `defer` cleanups and the helpers it inlines included)
    /// may not write the counter. The loop's own steps are spelled `scope(8, N)` (see
    /// [`Self::emit_continue_step`]), which nothing else emits outside raw code, so every
    /// `temp[N]` write left in the body text is the body's. Raw code, which could spell either,
    /// is rejected.
    fn check_temp_counter_untouched(
        &self,
        span: Span,
        counter: &str,
        body_mark: usize,
        raw_before: usize,
    ) -> Result<()> {
        if self.raw_blocks > raw_before {
            return Err(Error::at(
                span,
                format!(
                    "the body of this '#[temp]' loop (or a helper it calls) contains 'raw {{ .. }}', which could write its counter '{counter}'; use an ordinary range loop"
                ),
            ));
        }
        if air_writes_cell(self.em.since(body_mark), counter) {
            return Err(Error::at(
                span,
                format!(
                    "the body of this '#[temp]' loop writes its counter '{counter}' (directly, in a 'defer', or through a nested '#[temp]' loop)"
                ),
            ));
        }
        Ok(())
    }

    pub(super) fn emit_inline_fn_return_propagation(&mut self, restore_stack: bool) -> Result<()> {
        if let Some(ret_flag) = self
            .active_inline_fn
            .as_ref()
            .and_then(|active| active.ret_flag.clone())
        {
            // If we're still inside an outer loop inside the inline fn (Wait: the inline fn's
            // own wrapper loop is at `fn_scope_depth`!), breaking out of an inner loop needs to
            // propagate upward until we exit the wrapper loop.
            self.line(format!("if {ret_flag} != 0 {{"));
            self.em.indent();
            if restore_stack {
                self.emit_entry_stack_restore();
            }
            self.line("break;".to_owned());
            self.em.dedent();
            self.line("}".to_owned());
        }
        Ok(())
    }

    pub(super) fn lower_let_some(
        &mut self,
        span: Span,
        name: &str,
        ty_ann: Option<&V2Type>,
        init: &V2Expr,
        else_body: &[V2Stmt],
    ) -> Result<()> {
        if !stmts_unconditionally_diverge(
            else_body,
            self.env,
            &self.owner_module,
            self.in_object_scope,
        ) {
            return Err(Error::at(
                span,
                format!(
                    "'let Some({name}) = ... else {{ ... }}' else-block must diverge (end with return, break, or continue)"
                ),
            ));
        }
        // Form 1: `let Some(val) = obj.run_functional(kind [, args...]) else { ... };`
        if let V2Expr::MethodCall {
            span: mspan,
            receiver,
            method,
            type_args,
            args,
        } = init
            && method == "run_functional"
        {
            let (ret_ty, saved_stack_obj) = self.emit_run_functional_let_some(
                receiver, type_args, args, *mspan, name, ty_ann, else_body,
            )?;
            let air_name = self.fresh(&format!("v_{name}"));
            self.line(format!("let {air_name} = temp[0];"));
            self.emit_restore_stack_obj(saved_stack_obj.as_deref());
            self.bind_var(
                name.to_owned(),
                Binding::Local {
                    air_name,
                    ty: ret_ty,
                },
            );
            return Ok(());
        }

        // Form 2: `let Some(obj) = find_best_object[::<T>](...) | create_object[::<T>](...) else { ... };`
        let V2Expr::Call {
            callee,
            type_args,
            args,
            ..
        } = init
        else {
            return Err(Error::at(
                span,
                "'let Some(x) = ... else { ... }' requires find_best_object(...), create_object(...), or obj.run_functional(...)",
            ));
        };
        let V2Expr::Ident(_, fname) = callee.as_ref() else {
            return Err(Error::at(
                span,
                "'let Some(x) = ... else { ... }' requires find_best_object(...), create_object(...), or obj.run_functional(...)",
            ));
        };

        // Typed `find_best_object::<T>(kind [, flags: f])`: runtime-checked against T's GUIDs so
        // a higher-scoring foreign object can NEVER be bound as T, AND a lower-scoring T is still
        // found via fallback `set_to_next(guid, T::lead_guid)` scan!
        if fname == "find_best_object" && !type_args.is_empty() {
            let plan = self.parse_find_best_object(type_args, args, span)?;
            let schema = plan
                .typed_schema
                .expect("non-empty type_args yields schema");
            let inferred_ty = ValueType::ObjectRef(schema.canonical_key.clone());
            let final_ty = if let Some(ann) = ty_ann {
                let expected = self.env.resolve_value_type(
                    ann,
                    &self.owner_module,
                    self.in_object_scope,
                    span,
                )?;
                if !types_compatible_for_assign(&expected, &inferred_ty) {
                    return Err(Error::at(
                        span,
                        format!(
                            "'let Some({name}: {})' does not match '{}' result",
                            self.env.display_value_type(&expected),
                            self.env.display_value_type(&inferred_ty)
                        ),
                    ));
                }
                expected
            } else {
                inferred_ty
            };

            let found_var = self.fresh(&format!("v_{name}"));
            let raw_var = self.fresh("fbo_raw");
            let cand_var = self.fresh("fbo_cand");
            let score_field = match plan.kind_name.as_str() {
                "prep" => Some("stack.prep_score"),
                "cook" => Some("stack.cook_score"),
                _ => None,
            };
            let guard_summary = schema
                .functionals
                .get(&plan.kind_name)
                .map_or(FunctionalGuardSummary::Complex, |s| s.guard);
            let allow_fallback = !matches!(guard_summary, FunctionalGuardSummary::Complex);
            self.line(format!("let {found_var} = 0;"));
            let saved_stack_obj = self.save_stack_obj_for_move();
            self.line(format!("if {} {{", plan.prim_air));
            self.em.indent();
            self.stack_op(mir::StackObjOp::Save(&raw_var));
            self.line(format!("let {cand_var} = 0;"));
            let best_var = if allow_fallback && score_field.is_some() {
                let b = self.fresh("fbo_best");
                self.line(format!("let {b} = 0;"));
                Some(b)
            } else {
                None
            };
            self.line(format!(
                "while set_to_next(guid, {:#010x}, dest: {cand_var}) {{",
                schema.lead_guid
            ));
            self.em.indent();
            self.line(format!("if {cand_var} == {raw_var} {{"));
            self.em.indent();
            self.emit_store(&found_var, ":=", &cand_var);
            self.line("break;".to_owned());
            self.em.dedent();
            self.line("}".to_owned());
            if allow_fallback {
                self.stack_op(mir::StackObjOp::Set(&cand_var));
                self.line("if stack.in_use == 0 {".to_owned());
                self.em.indent();
                self.line("if !stack.data[8] flag_test 10 {".to_owned());
                self.em.indent();
                let mut extra_blocks = 0usize;
                if (plan.flags & 4) != 0 {
                    self.line("if stack.room == my.room {".to_owned());
                    self.em.indent();
                    extra_blocks += 1;
                }
                if let FunctionalGuardSummary::AttrBool {
                    attr_index,
                    expect_true,
                } = guard_summary
                {
                    let cond = if expect_true {
                        format!("stack.attr[{attr_index}] != 0")
                    } else {
                        format!("stack.attr[{attr_index}] == 0")
                    };
                    self.line(format!("if {cond} {{"));
                    self.em.indent();
                    extra_blocks += 1;
                }
                if let (Some(sf), Some(bvar)) = (score_field, &best_var) {
                    self.line(format!("if {sf} > 0 {{"));
                    self.em.indent();
                    self.line(format!("if {found_var} == 0 {{"));
                    self.em.indent();
                    self.emit_store(&found_var, ":=", &cand_var);
                    self.emit_store(bvar, ":=", sf);
                    self.em.dedent();
                    self.line(format!("}} else if {sf} > {bvar} {{"));
                    self.em.indent();
                    self.emit_store(&found_var, ":=", &cand_var);
                    self.emit_store(bvar, ":=", sf);
                    self.em.dedent();
                    self.line("}".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                } else {
                    self.line(format!("if {found_var} == 0 {{"));
                    self.em.indent();
                    self.emit_store(&found_var, ":=", &cand_var);
                    self.em.dedent();
                    self.line("}".to_owned());
                }
                for _ in 0..extra_blocks {
                    self.em.dedent();
                    self.line("}".to_owned());
                }
                self.em.dedent();
                self.line("}".to_owned());
                self.em.dedent();
                self.line("}".to_owned());
            }
            self.em.dedent();
            self.line("}".to_owned());

            // Multi-tile object support: if the VM's winner was a non-lead tile of T, accept it too.
            for &tile_guid in &schema.tile_guids {
                if tile_guid == schema.lead_guid {
                    continue;
                }
                let tchk = self.fresh("fbo_tile");
                self.line(format!("if {found_var} != {raw_var} {{"));
                self.em.indent();
                self.line(format!("let {tchk} = 0;"));
                self.line(format!(
                    "while set_to_next(guid, {tile_guid:#010x}, dest: {tchk}) {{"
                ));
                self.em.indent();
                self.line(format!("if {tchk} == {raw_var} {{"));
                self.em.indent();
                self.emit_store(&found_var, ":=", &raw_var);
                self.line("break;".to_owned());
                self.em.dedent();
                self.line("}".to_owned());
                self.em.dedent();
                self.line("}".to_owned());
                self.em.dedent();
                self.line("}".to_owned());
            }
            self.em.dedent();
            self.line("}".to_owned());

            self.line(format!("if {found_var} == 0 {{"));
            self.em.indent();
            self.emit_restore_stack_obj(saved_stack_obj.as_deref());
            self.push_scope(ScopeFrame::block());
            let r = self.lower_stmts(else_body);
            let p = self.pop_scope_and_run_cleanups();
            self.em.dedent();
            self.line("}".to_owned());
            r.and(p)?;

            self.emit_restore_stack_obj(saved_stack_obj.as_deref());
            self.bind_var(
                name.to_owned(),
                Binding::Local {
                    air_name: found_var,
                    ty: final_ty,
                },
            );
            return Ok(());
        }

        let (prim_air, inferred_ty) = match fname.as_str() {
            "find_best_object" => {
                let plan = self.parse_find_best_object(type_args, args, span)?;
                (plan.prim_air, ValueType::ObjectId)
            }
            "create_object" if has_create_dest(args) => {
                return Err(Error::at(
                    span,
                    "test create_object(dest: stack_obj) as a condition to keep its result in stack_obj",
                ));
            }
            "create_object" => self.format_create_object(type_args, args, span)?,
            other => {
                return Err(Error::at(
                    span,
                    format!(
                        "'let Some({name}) = {other}(...)' is not supported (expected 'find_best_object', 'create_object', or 'obj.run_functional')"
                    ),
                ));
            }
        };
        let final_ty = if let Some(ann) = ty_ann {
            let expected =
                self.env
                    .resolve_value_type(ann, &self.owner_module, self.in_object_scope, span)?;
            if !types_compatible_for_assign(&expected, &inferred_ty) {
                return Err(Error::at(
                    span,
                    format!(
                        "'let Some({name}: {})' does not match '{}' result",
                        self.env.display_value_type(&expected),
                        self.env.display_value_type(&inferred_ty)
                    ),
                ));
            }
            expected
        } else {
            inferred_ty
        };

        let saved_stack_obj = self.save_stack_obj_for_move();
        self.line(format!("if !{prim_air} {{"));
        self.em.indent();
        self.emit_restore_stack_obj(saved_stack_obj.as_deref());
        self.push_scope(ScopeFrame::block());
        let r = self.lower_stmts(else_body);
        let p = self.pop_scope_and_run_cleanups();
        self.em.dedent();
        self.line("}".to_owned());
        r.and(p)?;

        let air_name = self.fresh(&format!("v_{name}"));
        self.stack_op(mir::StackObjOp::Save(&air_name));
        self.emit_restore_stack_obj(saved_stack_obj.as_deref());
        self.bind_var(
            name.to_owned(),
            Binding::Local {
                air_name,
                ty: final_ty,
            },
        );
        Ok(())
    }

    pub(super) fn lower_expr_stmt(&mut self, expr: &V2Expr) -> Result<()> {
        match expr {
            V2Expr::Try { span, inner } => {
                // Checked up front: the generic desugaring below may constant-fold its error
                // branch away before `return error;` would report it.
                if self.in_defer() {
                    return Err(Error::at(*span, "'?' cannot exit out of a 'defer' block"));
                }
                if let V2Expr::Call {
                    span: cspan,
                    callee,
                    type_args,
                    args,
                } = inner.as_ref()
                    && type_args.is_empty()
                    && let Some(sig) = self.resolve_callable(callee, *cspan)?
                {
                    return self.emit_callable_stmt(&sig, args, *cspan, true);
                }
                // Generic `cond?;` on a bool expression: `if !cond { return error; }`.
                let desugared = V2Stmt::If {
                    span: *span,
                    cond: V2Expr::Unary {
                        span: *span,
                        op: V2UnaryOp::Not,
                        inner: inner.clone(),
                    },
                    then_body: vec![V2Stmt::ReturnError {
                        span: *span,
                        from_try: true,
                    }],
                    else_body: Vec::new(),
                };
                self.lower_stmt(&desugared)
            }
            V2Expr::Call {
                span,
                callee,
                type_args,
                args,
            } => {
                if let V2Expr::Ident(_, fname) = callee.as_ref()
                    && fname == "create_object"
                    && !self.user_callable_named(callee, *span)?
                {
                    let (prim, _) = self.format_create_object(type_args, args, *span)?;
                    if has_create_dest(args) {
                        self.line(format!("{prim};"));
                        return Ok(());
                    }
                    // The legacy spelling restores the pre-call stack object on both edges.
                    let pre = self.fresh("pre_stack_obj");
                    self.stack_op(mir::StackObjOp::Save(&pre));
                    self.line(format!("{prim};"));
                    self.stack_op(mir::StackObjOp::Set(&pre));
                    return Ok(());
                }
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        "type arguments are only supported on 'create_object::<T>', 'find_best_object::<T>', and 'objects_of::<T>'",
                    ));
                }
                if let Some(sig) = self.resolve_callable(callee, *span)? {
                    return self.emit_callable_stmt(&sig, args, *span, false);
                }
                if let V2Expr::Ident(_, fname) = callee.as_ref() {
                    match fname.as_str() {
                        "sleep" | "idle" => return self.emit_sleep(args, *span),
                        "play_sound" | "stop_all_sounds" => {
                            let (prim, on_self) = if fname == "play_sound" {
                                self.format_play_sound(args, *span)?
                            } else {
                                self.format_stop_all_sounds(args)?
                            };
                            if on_self {
                                self.emit_on_self(prim);
                            } else {
                                self.line(format!("{prim};"));
                            }
                            return Ok(());
                        }
                        "debit_funds" => {
                            let s = self.format_alter_budget(args, *span, false, false)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "credit_funds" => {
                            let s = self.format_alter_budget(args, *span, false, true)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "destroy_self" => {
                            let mut flags = 0u8;
                            for a in args {
                                let Some(aname) = &a.name else {
                                    return Err(Error::at(
                                        a.span,
                                        "destroy_self() only takes named options ('flags:', 'no_yield:', 'skip_resets:')",
                                    ));
                                };
                                match aname.as_str() {
                                    "flags" => {
                                        let v = self.eval_const_i16(&a.value)?;
                                        flags |= u8::try_from(v).map_err(|_| {
                                            Error::at(
                                                a.span,
                                                format!(
                                                    "destroy_self flags {v} out of range 0..=255"
                                                ),
                                            )
                                        })?;
                                    }
                                    "no_yield" => {
                                        if self.eval_const_bool_required(&a.value, a.span)? {
                                            flags |= 1;
                                        }
                                    }
                                    "skip_resets" => {
                                        if self.eval_const_bool_required(&a.value, a.span)? {
                                            flags |= 2;
                                        }
                                    }
                                    other => {
                                        return Err(Error::at(
                                            a.span,
                                            format!(
                                                "unknown destroy_self option '{other}' (expected 'flags', 'no_yield', or 'skip_resets')"
                                            ),
                                        ));
                                    }
                                }
                            }
                            let target = match self.tree_ctx {
                                TreeContext::ObjectSelf => "me",
                                TreeContext::SimOnObject { .. } => "stack_obj",
                            };
                            let prim = if flags == 0 {
                                format!("remove_object({target})")
                            } else {
                                format!("remove_object({target}, flags: {flags})")
                            };
                            self.emit_on_self(prim);
                            return Ok(());
                        }
                        "set_graphic" => {
                            if args.len() != 1 || args[0].name.is_some() {
                                return Err(Error::at(
                                    *span,
                                    "set_graphic(index) expects exactly one positional argument",
                                ));
                            }
                            let (val, _) = self.lower_value_expr(&args[0].value)?;
                            let cell = self.self_cell("data[0]", ValueType::I16);
                            return self.emit_lvalue_assign(&cell, V2AssignOp::Set, &val);
                        }
                        "route_to" => {
                            let s = self.format_slot_prim("route_to", args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "snap" => {
                            let s = self.format_slot_prim("snap", args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "set_motive_change" => {
                            let s = self.format_set_motive_change(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        "clear_motive_changes" => {
                            if !args.is_empty() {
                                return Err(Error::at(
                                    *span,
                                    "clear_motive_changes() takes no arguments",
                                ));
                            }
                            self.line("clear_motive_changes();".to_owned());
                            return Ok(());
                        }
                        "dialog" => {
                            let s = self.format_dialog(args, *span)?;
                            self.line(format!("{s};"));
                            return Ok(());
                        }
                        _ => {
                            // User helper function call (discarding optional return value).
                            if let Some((fn_mod, fn_decl, is_local)) =
                                self.env
                                    .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                            {
                                let _ = self
                                    .inline_helper_call(&fn_mod, &fn_decl, is_local, args, *span)?;
                                return Ok(());
                            }
                            if fname == "set_dynamic_sprite" {
                                return self.lower_set_dynamic_sprite(args, *span);
                            }
                            if let Some(prim) = self.format_prim_builtin(fname, args, *span)? {
                                if matches!(fname.as_str(), "inc_lt" | "dec_gt") {
                                    self.line(format!("if {prim} {{"));
                                    self.line("}".to_owned());
                                } else {
                                    self.line(format!("{prim};"));
                                }
                                return Ok(());
                            }
                            return Err(Error::at(*span, format!("unknown function '{fname}'")));
                        }
                    }
                }
                Err(Error::at(*span, "unsupported call target expression"))
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } => match method.as_str() {
                "run_functional" => {
                    if !type_args.is_empty() {
                        return Err(Error::at(
                            *span,
                            "type arguments on run_functional are only valid inside 'let Some(x) = obj.run_functional::<T>(...)'",
                        ));
                    }
                    self.emit_run_functional(receiver, args, *span, None)?;
                    Ok(())
                }
                // `x.play_sound(s)`, `x.stop_all_sounds()`, `x.refresh(mode)`: the builtin with
                // `target: x` (the same primitive as the call form).
                "play_sound" | "stop_all_sounds" | "refresh" => {
                    if !type_args.is_empty() {
                        return Err(Error::at(
                            *span,
                            format!("'.{method}()' does not accept type arguments"),
                        ));
                    }
                    if let Some(a) = args.iter().find(|a| a.name.as_deref() == Some("target")) {
                        return Err(Error::at(
                            a.span,
                            format!("'.{method}()' takes no 'target:'; the receiver is the target"),
                        ));
                    }
                    let mut full = args.clone();
                    if method == "refresh" {
                        // `x.refresh(mode)`: its one positional argument is the mode.
                        for a in &mut full {
                            if a.name.is_none() {
                                a.name = Some("mode".to_owned());
                            }
                        }
                    }
                    full.push(CallArg {
                        span: receiver.span(),
                        name: Some("target".to_owned()),
                        value: (**receiver).clone(),
                    });
                    match method.as_str() {
                        "refresh" => {
                            let prim = self
                                .format_prim_builtin("refresh", &full, *span)?
                                .expect("refresh is a primitive builtin");
                            self.line(format!("{prim};"));
                        }
                        _ => {
                            let (prim, on_self) = if method == "play_sound" {
                                self.format_play_sound(&full, *span)?
                            } else {
                                self.format_stop_all_sounds(&full)?
                            };
                            if on_self {
                                self.emit_on_self(prim);
                            } else {
                                self.line(format!("{prim};"));
                            }
                        }
                    }
                    Ok(())
                }
                // `sim.debit_funds(n)` / `sim.credit_funds(n)`: the Sim's household budget (the
                // lot's family; the same primitive as `debit_funds(n)`).
                "debit_funds" | "credit_funds" => {
                    if !type_args.is_empty() {
                        return Err(Error::at(
                            *span,
                            format!("'.{method}()' does not accept type arguments"),
                        ));
                    }
                    if !matches!(receiver.as_ref(), V2Expr::Ident(_, n) if self.is_sim_ident(n)) {
                        return Err(Error::at(
                            receiver.span(),
                            format!(
                                "'.{method}()' is called on the interaction's Sim (its household's budget)"
                            ),
                        ));
                    }
                    let s =
                        self.format_alter_budget(args, *span, false, method == "credit_funds")?;
                    self.line(format!("{s};"));
                    Ok(())
                }
                "set" => {
                    if !type_args.is_empty() {
                        return Err(Error::at(*span, "'.set()' does not accept type arguments"));
                    }
                    self.emit_bitflag_mutate(receiver, args, *span, true)
                }
                "clear" => {
                    if !type_args.is_empty() {
                        return Err(Error::at(
                            *span,
                            "'.clear()' does not accept type arguments",
                        ));
                    }
                    self.emit_bitflag_mutate(receiver, args, *span, false)
                }
                // `x.push_interaction(menu: ...)`: `push_interaction(object: x, menu: ...)`.
                "push_interaction" => {
                    if !type_args.is_empty() {
                        return Err(Error::at(
                            *span,
                            "'.push_interaction()' does not accept type arguments",
                        ));
                    }
                    if let Some(a) = args
                        .iter()
                        .find(|a| a.name.is_none() || a.name.as_deref() == Some("object"))
                    {
                        return Err(Error::at(
                            a.span,
                            "'.push_interaction()' takes named arguments; the receiver is the object",
                        ));
                    }
                    let mut full = vec![CallArg {
                        span: receiver.span(),
                        name: Some("object".to_owned()),
                        value: (**receiver).clone(),
                    }];
                    full.extend(args.iter().cloned());
                    let prim = self
                        .format_prim_builtin("push_interaction", &full, *span)?
                        .expect("push_interaction is a primitive builtin");
                    self.line(format!("{prim};"));
                    Ok(())
                }
                _ => Err(Error::at(*span, format!("unknown method '.{method}(...)'"))),
            },
            other => Err(Error::at(
                other.span(),
                "expression has no effect as a statement",
            )),
        }
    }

    pub(super) fn maybe_auto_deref_mut_ref(&self, expr: &V2Expr) -> Option<V2Expr> {
        if let V2Expr::Ident(span, name) = expr
            && matches!(self.lookup_var(name), Some(Binding::MutRef { .. }))
        {
            return Some(V2Expr::Deref {
                span: *span,
                inner: Box::new(expr.clone()),
            });
        }
        None
    }

    pub(super) fn emit_bitflag_mutate(
        &mut self,
        receiver: &V2Expr,
        args: &[CallArg],
        span: Span,
        is_set: bool,
    ) -> Result<()> {
        let method = if is_set { "set" } else { "clear" };
        if args.len() != 1 || args[0].name.is_some() {
            return Err(Error::at(
                span,
                format!("'.{method}(flag)' expects exactly 1 positional argument"),
            ));
        }
        let derefed = self.maybe_auto_deref_mut_ref(receiver);
        let lval = self.resolve_lvalue(derefed.as_ref().unwrap_or(receiver))?;
        let ValueType::Bitflags(key) = lval.value_type().clone() else {
            return Err(Error::at(
                span,
                format!(
                    "'.{method}(...)' requires a bitflags target, found '{}'",
                    self.env.display_value_type(lval.value_type())
                ),
            ));
        };
        if let Some((mask, single_bit)) = self.try_eval_const_bitflag(&args[0].value, &key)? {
            if let Some(bit) = single_bit {
                let op = if is_set {
                    V2AssignOp::FlagSet
                } else {
                    V2AssignOp::FlagClear
                };
                return self.emit_lvalue_assign(&lval, op, &bit.to_string());
            }
            if is_set {
                return self.emit_lvalue_assign(&lval, V2AssignOp::BitOr, &mask.to_string());
            }
            return self.emit_lvalue_assign(&lval, V2AssignOp::BitAnd, &(!mask).to_string());
        }
        let (rhs_air, rhs_ty) = self.lower_value_expr(&args[0].value)?;
        if !types_compatible_for_assign(&ValueType::Bitflags(key.clone()), &rhs_ty)
            || !matches!(rhs_ty, ValueType::Bitflags(_))
        {
            return Err(Error::at(
                args[0].span,
                format!(
                    "type mismatch in '.{method}(...)': expected '{}', found '{}'",
                    self.env.display_value_type(&ValueType::Bitflags(key)),
                    self.env.display_value_type(&rhs_ty)
                ),
            ));
        }
        if is_set {
            self.emit_lvalue_assign(&lval, V2AssignOp::BitOr, &rhs_air)
        } else {
            let inv = self.fresh("bf_inv");
            self.line(format!("let {inv} = -1;"));
            self.line(format!("{inv} ^= {rhs_air};"));
            self.emit_lvalue_assign(&lval, V2AssignOp::BitAnd, &inv)
        }
    }
}

/// Whether the `.air` text stores into `cell` (an assignment, compound assignment, or one of the
/// in-place operators `inc_lt` / `dec_gt` / `flag_set` / `flag_clear` / `sqrt` with `cell` on the
/// left). Calls and primitives that write temps as a side effect are not visible here.
fn air_writes_cell(air: &str, cell: &str) -> bool {
    const WRITES: &[&str] = &[
        ":=",
        "=",
        "+=",
        "-=",
        "*=",
        "/=",
        "%=",
        "&=",
        "|=",
        "^=",
        "<<=",
        ">>=",
        "inc_lt",
        "dec_gt",
        "flag_set",
        "flag_clear",
        "sqrt",
    ];
    air.match_indices(cell).any(|(at, _)| {
        let before_ok = air[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'));
        let op = air[at + cell.len()..]
            .split_whitespace()
            .next()
            .unwrap_or("");
        // A primitive's output argument (`dest: temp[0]`) writes it too.
        let is_dest = air[..at].trim_end().ends_with("dest:");
        before_ok && (WRITES.contains(&op) || is_dest)
    })
}
