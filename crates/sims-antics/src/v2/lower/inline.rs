//! Helper inlining: specialization splits, helper-argument binding and cell forwarding, inline
//! branches and the inline call mode.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// A builtin call statement with a `match` / `if` operand whose arms are compile-time
    /// operands, as one statement per arm (see [`specialize::split_inline_operand`]): each copy
    /// passes its own constant, which an operand that must be compile-time requires. User
    /// functions and trees take their arguments as values, so their calls are left alone.
    pub(super) fn split_inline_specialization(&self, stmt: &V2Stmt) -> Result<Option<V2Stmt>> {
        match stmt {
            V2Stmt::Expr { span, expr } => {
                let (call, tried) = match expr {
                    V2Expr::Try { inner, .. } => (inner.as_ref(), true),
                    other => (other, false),
                };
                let V2Expr::Call {
                    span: cspan,
                    callee,
                    type_args,
                    args,
                } = call
                else {
                    return Ok(None);
                };
                if !matches!(callee.as_ref(), V2Expr::Ident(..))
                    || self.user_callable_named(callee, *cspan)?
                {
                    return Ok(None);
                }
                let rebuild = |args: Vec<CallArg>| {
                    let call = V2Expr::Call {
                        span: *cspan,
                        callee: callee.clone(),
                        type_args: type_args.clone(),
                        args,
                    };
                    V2Stmt::Expr {
                        span: *span,
                        expr: if tried {
                            V2Expr::Try {
                                span: expr.span(),
                                inner: Box::new(call),
                            }
                        } else {
                            call
                        },
                    }
                };
                Ok(specialize::split_inline_operand(*span, args, &rebuild))
            }
            V2Stmt::Animate {
                span,
                args,
                event,
                body,
            } => {
                let callee = V2Expr::Ident(*span, "animate".to_owned());
                if self.user_callable_named(&callee, *span)? {
                    return Ok(None);
                }
                let rebuild = |args: Vec<CallArg>| V2Stmt::Animate {
                    span: *span,
                    args,
                    event: event.clone(),
                    body: body.clone(),
                };
                Ok(specialize::split_inline_operand(*span, args, &rebuild))
            }
            _ => Ok(None),
        }
    }

    /// A by-value read may move from argument evaluation to its uses only for a valid,
    /// stable cell. Unknown effects (including primitives, calls, raw code, and indirect
    /// writes) force a snapshot. A Sim tree's `stack.attr` can address a different object
    /// even before the helper is entered, so only declared attributes on `my` qualify.
    /// Any other cell (an object's, the Sim's, a global) is forwarded only when reading it can
    /// neither fault nor wake an object ([`Self::safe_cell_air`]), so moving, repeating or
    /// dropping the read changes nothing, and the helper is pure ([`HelperCell::Pure`]).
    pub(super) fn can_forward_helper_cell(
        &self,
        air: &str,
        decl: &FnDecl,
        args: &[&V2Expr],
    ) -> bool {
        let index = |prefix| {
            air.strip_prefix(prefix)?
                .strip_suffix(']')?
                .parse::<i16>()
                .ok()
        };
        let cell = if let Some(i) = index("temp[").filter(|i| (0..8).contains(i)) {
            HelperCell::Temp(i)
        } else if let Some(i) = index("param[").filter(|&i| (i as usize) < self.frame_args) {
            HelperCell::Param(i)
        } else if let Some(i) = index("my.attr[")
            && self.env.states.values().any(|s| s.attr_index == i as usize)
        {
            HelperCell::OwnAttr(i)
        } else if self.safe_cell_air(air)
            // Resolving data word 25 wakes its object, so that read must happen as written.
            && !canonical_air_cell(air).ends_with(".data[25]")
        {
            HelperCell::Pure
        } else {
            return false;
        };
        let self_states = (!decl.params.iter().any(|p| p.name == "self")
            && !stmts_mutate_or_borrow_ident(&decl.body, "self"))
        .then_some(&self.env.states);
        args.iter().all(|e| helper_expr_has_no_writes(e))
            && decl
                .params
                .iter()
                .filter_map(|p| p.default.as_ref())
                .all(helper_expr_has_no_writes)
            && stmts_preserve_helper_cell(&decl.body, cell, self_states)
    }

    /// Whether inlining `decl` (declared in `module`) can write a caller's frame word without
    /// naming it: it, or a helper it calls (transitively), runs a `raw { }` block.
    pub(super) fn helper_may_write_frame_words(
        &self,
        decl: &FnDecl,
        module: &str,
        allow_local: bool,
        depth: usize,
    ) -> bool {
        if depth > 32
            || super::specialize::stmts_any(&decl.body, &|s| matches!(s, V2Stmt::RawAir { .. }))
        {
            return true;
        }
        let mut callees = Vec::new();
        collect_callee_names(&decl.body, &mut callees);
        callees.iter().any(|name| {
            self.env
                .lookup_fn(module, name, allow_local)
                .is_some_and(|(m, d, l)| self.helper_may_write_frame_words(&d, &m, l, depth + 1))
        })
    }

    /// Whether evaluating `expr` in this scope calls a helper that may write frame words (see
    /// [`Self::helper_may_write_frame_words`]).
    pub(super) fn expr_may_write_frame_words(&self, expr: &V2Expr, depth: usize) -> bool {
        let mut callees = Vec::new();
        super::specialize::visit_expr(expr, &mut |e| {
            if let V2Expr::Call { callee, .. } = e
                && let V2Expr::Ident(_, name) = callee.as_ref()
            {
                callees.push(name.clone());
            }
        });
        callees.iter().any(|name| {
            self.env
                .lookup_fn(&self.owner_module, name, self.in_object_scope)
                .is_some_and(|(m, d, l)| self.helper_may_write_frame_words(&d, &m, l, depth + 1))
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn bind_helper_arg(
        &mut self,
        fn_mod: &str,
        is_local_fn: bool,
        fn_decl: &FnDecl,
        param: &crate::v2::ast::FnParam,
        arg_expr: &V2Expr,
        arg_span: Span,
        call_args: &[&V2Expr],
    ) -> Result<Binding> {
        if !param.is_const {
            return self.bind_helper_arg_value(
                fn_mod,
                is_local_fn,
                fn_decl,
                param,
                arg_expr,
                arg_span,
                call_args,
            );
        }
        let active = self.local_writing_builtins_in(fn_mod, is_local_fn);
        if stmts_mutate_or_borrow_ident_in(&fn_decl.body, &param.name, &active) {
            return Err(Error::at(
                param.span,
                format!(
                    "const parameter '{}' cannot be assigned or borrowed",
                    param.name
                ),
            ));
        }
        let binding = self.bind_helper_arg_value(
            fn_mod,
            is_local_fn,
            fn_decl,
            param,
            arg_expr,
            arg_span,
            call_args,
        )?;
        match binding {
            // A tunable `param` binds as its BCON read: still a run-time value.
            Binding::ConstI16 { ref air, .. } if air.parse::<i16>().is_ok() => Ok(binding),
            Binding::ConstBool(_) | Binding::ConstEnum { .. } | Binding::ConstBitflag { .. } => {
                Ok(binding)
            }
            _ => Err(Error::at(
                arg_span,
                format!(
                    "const parameter '{}' of '{}' needs a compile-time constant argument",
                    param.name, fn_decl.name
                ),
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_helper_arg_value(
        &mut self,
        fn_mod: &str,
        is_local_fn: bool,
        fn_decl: &FnDecl,
        param: &crate::v2::ast::FnParam,
        arg_expr: &V2Expr,
        arg_span: Span,
        call_args: &[&V2Expr],
    ) -> Result<Binding> {
        let active = self.local_writing_builtins_in(fn_mod, is_local_fn);
        let read_only = !stmts_mutate_or_borrow_ident_in(&fn_decl.body, &param.name, &active);
        // A caller local passed by value needs no `__arg_*` copy when nothing can change it
        // while the helper runs: the helper never mutates or borrows the parameter, and no
        // argument of this call borrows the local (`&mut x`, or a local-writing builtin). An
        // `#[slot]` local lives in an explicit frame word, which helper raw code can write as
        // `local[N]`, so it is always copied. So is any local when the helper, or a helper an
        // argument calls, runs raw code (directly or through the helpers it calls): raw code
        // can write any frame word by number or by its `.air` name.
        let frame_writer = self.helper_may_write_frame_words(fn_decl, fn_mod, is_local_fn, 0)
            || call_args
                .iter()
                .any(|a| self.expr_may_write_frame_words(a, 0));
        let elide_local = match arg_expr {
            _ if frame_writer => None,
            V2Expr::Ident(_, n)
                if !matches!(param.ty, V2Type::MutRef(_))
                    && read_only
                    && !call_args
                        .iter()
                        .any(|a| expr_borrows_ident_in(a, n, LOCAL_WRITING_BUILTINS)) =>
            {
                match self.lookup_var(n) {
                    Some(Binding::Local { air_name, .. })
                        if !air_name.starts_with("__slot")
                            && !call_args.iter().any(|e| {
                            matches!(e, V2Expr::Ident(_, name)
                                if matches!(self.lookup_var(name), Some(Binding::MutRef { target: LValue::Direct { air, .. } }) if air == air_name))
                        }) => Some(air_name.clone()),
                    _ => None,
                }
            }
            _ => None,
        };
        let arg_local = |c: &mut Self, air: String| -> String {
            if elide_local.as_deref() == Some(air.as_str()) {
                return air;
            }
            let loc = c.fresh(&format!("arg_{}", param.name));
            c.emit_let(&loc, &air);
            loc
        };
        let bind_operand = |c: &mut Self, air: String, ty: ValueType| {
            if read_only && c.can_forward_helper_cell(&air, fn_decl, call_args) {
                Binding::Operand { air, ty }
            } else {
                Binding::Local {
                    air_name: arg_local(c, air),
                    ty,
                }
            }
        };
        let binding = match &param.ty {
            V2Type::I16 => {
                if read_only
                    && let Some((const_air, const_val)) =
                        self.try_const_i16_air_operand(arg_expr)?
                {
                    Binding::ConstI16 {
                        air: const_air,
                        value: const_val,
                    }
                } else {
                    let (air, ty) = self.lower_value_expr(arg_expr)?;
                    if !types_compatible_for_assign(&ValueType::I16, &ty) {
                        return Err(Error::at(
                            arg_span,
                            format!(
                                "parameter '{}' expects i16, found '{}'",
                                param.name,
                                self.env.display_value_type(&ty)
                            ),
                        ));
                    }
                    bind_operand(self, air, ValueType::I16)
                }
            }
            V2Type::Bool => {
                if read_only && let Some(const_b) = self.try_const_bool_arg(arg_expr) {
                    Binding::ConstBool(const_b)
                } else {
                    let (air, ty) = self.lower_value_expr(arg_expr)?;
                    if ty != ValueType::Bool {
                        return Err(Error::at(
                            arg_span,
                            format!("parameter '{}' expects bool", param.name),
                        ));
                    }
                    bind_operand(self, air, ValueType::Bool)
                }
            }
            V2Type::ObjectId => {
                let (air, ty) = self.lower_value_expr(arg_expr)?;
                let is_null_zero = matches!(arg_expr, V2Expr::Int(_, 0));
                if !ty.is_object_handle() && !is_null_zero {
                    return Err(Error::at(
                        arg_span,
                        format!(
                            "parameter '{}' expects ObjectId (or 0), found '{}' (convert numeric temps explicitly via 'let id: ObjectId = ...;')",
                            param.name,
                            self.env.display_value_type(&ty)
                        ),
                    ));
                }
                let loc = arg_local(self, air);
                Binding::Local {
                    air_name: loc,
                    ty: ValueType::ObjectId,
                }
            }
            V2Type::Named(_) => {
                let expected =
                    self.env
                        .resolve_value_type(&param.ty, fn_mod, is_local_fn, param.span)?;
                if read_only {
                    if let ValueType::Bitflags(ref bf_key) = expected
                        && let Some((mask, single_bit)) =
                            self.try_eval_const_bitflag(arg_expr, bf_key)?
                    {
                        return Ok(Binding::ConstBitflag {
                            key: bf_key.clone(),
                            mask,
                            single_bit,
                        });
                    }
                    if let ValueType::Enum(ref e_key) = expected
                        && let Some(val) = self.try_eval_const_enum(arg_expr, e_key)?
                    {
                        return Ok(Binding::ConstEnum {
                            key: e_key.clone(),
                            value: val,
                        });
                    }
                }
                let (air, ty) = self.lower_value_expr(arg_expr)?;
                if !types_compatible_for_assign(&expected, &ty) {
                    return Err(Error::at(
                        arg_span,
                        format!(
                            "parameter '{}' expects '{}', found '{}'",
                            param.name,
                            self.env.display_value_type(&expected),
                            self.env.display_value_type(&ty)
                        ),
                    ));
                }
                let loc = arg_local(self, air);
                Binding::Local {
                    air_name: loc,
                    ty: expected,
                }
            }
            V2Type::MutRef(inner) => {
                let mut lval = match arg_expr {
                    V2Expr::MutRef { inner: place, .. } => {
                        self.reject_read_only_sim_slot(place)?;
                        self.resolve_lvalue(place)?
                    }
                    V2Expr::Ident(_, pname) => match self.lookup_var(pname).cloned() {
                        Some(Binding::MutRef { target }) => target,
                        _ => {
                            return Err(Error::at(
                                arg_span,
                                format!(
                                    "parameter '{}' expects '&mut {}'",
                                    param.name,
                                    inner.display_name()
                                ),
                            ));
                        }
                    },
                    _ => {
                        return Err(Error::at(
                            arg_span,
                            format!(
                                "parameter '{}' expects '&mut {}'",
                                param.name,
                                inner.display_name()
                            ),
                        ));
                    }
                };
                let expected_inner =
                    self.env
                        .resolve_value_type(inner, fn_mod, is_local_fn, param.span)?;
                if lval.value_type() != &expected_inner {
                    return Err(Error::at(
                        arg_span,
                        format!(
                            "type mismatch for '&mut {}' parameter '{}' (found '{}')",
                            self.env.display_value_type(&expected_inner),
                            param.name,
                            self.env.display_value_type(lval.value_type())
                        ),
                    ));
                }
                // Snapshot foreign object handle so later reassignment of caller local cannot alias.
                match &mut lval {
                    LValue::ForeignAttr { obj_air, .. } | LValue::ForeignCell { obj_air, .. } => {
                        let snap = self.fresh(&format!("ref_obj_{}", param.name));
                        self.emit_let(&snap, obj_air);
                        *obj_air = snap;
                    }
                    LValue::Direct { .. } => {}
                }
                // Likewise a run-time dynamic-sprite index: the reference names the cell chosen
                // at the call, even if the helper reassigns the caller's index variable.
                if let LValue::ForeignCell {
                    setup: Some(setup), ..
                } = &mut lval
                    && let Some(idx) = setup
                        .strip_prefix("temp[0] := ")
                        .and_then(|r| r.strip_suffix(';'))
                    && idx.parse::<i16>().is_err()
                {
                    let snap = self.fresh(&format!("ref_idx_{}", param.name));
                    self.emit_let(&snap, idx);
                    *setup = format!("temp[0] := {snap};");
                }
                Binding::MutRef { target: lval }
            }
            V2Type::Sim => {
                let V2Expr::Ident(ispan, ident) = arg_expr else {
                    return Err(Error::at(
                        arg_span,
                        format!("parameter '{}' expects Sim handle", param.name),
                    ));
                };
                if !self.is_sim_ident(ident) {
                    return Err(Error::at(
                        *ispan,
                        format!(
                            "expected Sim handle for parameter '{}', found '{ident}'",
                            param.name
                        ),
                    ));
                }
                Binding::SimHandle
            }
            V2Type::Object => {
                if let V2Expr::Ident(_, ident) = arg_expr
                    && self.is_self_ident(ident)
                {
                    Binding::SelfHandle
                } else {
                    let (air, ty) = self.lower_value_expr(arg_expr)?;
                    if !ty.is_object_handle() {
                        return Err(Error::at(
                            arg_span,
                            format!(
                                "parameter '{}' expects 'self' or ObjectId, found '{}'",
                                param.name,
                                self.env.display_value_type(&ty)
                            ),
                        ));
                    }
                    let loc = self.fresh(&format!("obj_{}", param.name));
                    self.emit_let(&loc, &air);
                    Binding::ForeignObject { air_name: loc }
                }
            }
            V2Type::Motive => {
                let m = self.resolve_motive_expr(arg_expr)?;
                Binding::Motive(m)
            }
            V2Type::Skill => {
                let sk = self.resolve_skill_expr(arg_expr)?;
                Binding::Skill(sk)
            }
            V2Type::Sound => {
                let sid = self.resolve_sound_expr(arg_expr)?;
                Binding::Sound(sid)
            }
            V2Type::Slot => {
                let slot_idx = self.resolve_slot_expr(arg_expr)?;
                Binding::Slot(slot_idx)
            }
            V2Type::Guid => Binding::Guid(self.resolve_guid_expr(arg_expr)?),
        };
        Ok(binding)
    }

    /// Resolves the same direct helper calls as value lowering. A tree and the
    /// pre-existing builtins with priority over helpers keep their usual paths.
    pub(super) fn direct_helper<'e>(
        &self,
        expr: &'e V2Expr,
    ) -> Result<Option<InlineHelperCall<'e>>> {
        let V2Expr::Call {
            span,
            callee,
            type_args,
            args,
        } = expr
        else {
            return Ok(None);
        };
        let V2Expr::Ident(_, name) = callee.as_ref() else {
            return Ok(None);
        };
        if !type_args.is_empty()
            || self.resolve_callable(callee, *span)?.is_some()
            || matches!(
                name.as_str(),
                "random"
                    | "can_afford"
                    | "debit_funds"
                    | "credit_funds"
                    | "dialog"
                    | "route_to"
                    | "snap"
                    | "find_best_object"
            )
        {
            return Ok(None);
        }
        Ok(self
            .env
            .lookup_fn(&self.owner_module, name, self.in_object_scope)
            .map(|(module, decl, local)| (module, decl, local, args.as_slice(), *span)))
    }

    pub(super) fn try_lower_inline_into(
        &mut self,
        expr: &V2Expr,
        dest: &str,
    ) -> Result<Option<ValueType>> {
        if self.enclosing_cancel_flag().is_some() {
            return Ok(None);
        }
        let Some((module, decl, local, args, span)) = self.direct_helper(expr)? else {
            return Ok(None);
        };
        // Into mode stores the result in `dest` at the helper's `return`, before its defers
        // run, and an `#[slot]` word is visible to raw code by number: either could observe the
        // early store, so those keep the general path (store after the helper finishes).
        if decl.return_type.is_none()
            || dest.starts_with("__slot")
            || super::specialize::stmts_any(&decl.body, &|s| matches!(s, V2Stmt::Defer { .. }))
        {
            return Ok(None);
        }
        Ok(self
            .inline_helper_call_mode(
                &module,
                &decl,
                local,
                args,
                span,
                InlineCallMode::Into(dest),
            )?
            .map(|(_, ty)| ty))
    }

    /// A caller arm is lowered in the caller's lexical and cleanup scopes. Hoist its
    /// declarations once so that copies on multiple exits share the same frame slots.
    pub(super) fn capture_inline_arm(
        &mut self,
        body: &[V2Stmt],
        declarations: &mut Vec<String>,
    ) -> Result<String> {
        let old_indent = self.em.set_depth(0);
        let start = self.em.mark();
        let result = self.lower_block(body);
        let arm = self.em.split_off(start);
        self.em.set_depth(old_indent);
        result?;
        let mut code = String::new();
        for line in arm.lines() {
            let trimmed = line.trim_start();
            if let Some(decl) = trimmed.strip_prefix("let ") {
                let (name, init) = decl
                    .strip_suffix(';')
                    .unwrap_or(decl)
                    .split_once(" = ")
                    .map_or((decl.trim_end_matches(';'), None), |(n, v)| (n, Some(v)));
                declarations.push(format!("let {name};"));
                if let Some(init) = init {
                    let indent = &line[..line.len() - trimmed.len()];
                    writeln!(code, "{indent}{name} := {init};").unwrap();
                }
            } else {
                writeln!(code, "{line}").unwrap();
            }
        }
        Ok(code)
    }

    pub(super) fn try_lower_inline_branch(
        &mut self,
        cond: &V2Expr,
        then_body: &[V2Stmt],
        else_body: &[V2Stmt],
        terminal: bool,
    ) -> Result<bool> {
        // A synthetic loop cannot host caller arms that jump to an enclosing loop.
        // An enclosing inline return also uses a relative break target.
        if self.active_inline_fn.is_some()
            || self.enclosing_cancel_flag().is_some()
            || self.keep_layout
            || self.pin_span.is_some()
            || stmts_exit_loop(then_body)
            || stmts_exit_loop(else_body)
        {
            return Ok(false);
        }
        let (call, negated) = match cond {
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => (inner.as_ref(), true),
            _ => (cond, false),
        };
        let Some((module, decl, local, args, span)) = self.direct_helper(call)? else {
            return Ok(false);
        };
        // A helper's cleanup (`defer`, `with lock`'s unlock) runs after its `return` routes the
        // branch, so the caller's arm would see the cleanup's last result rather than the
        // condition's (which a primitive such as `gosub_found_action` reads): the general path
        // tests a flag after it.
        if !self.helper_returns_condition(&module, &decl, local)
            || stmts_return_through_loop(&decl.body)
            || super::specialize::stmts_any(&decl.body, &|s| {
                // Every statement that registers a cleanup (`CleanupAction`).
                matches!(s, V2Stmt::Defer { .. } | V2Stmt::WithLock { .. })
            })
        {
            return Ok(false);
        }

        // Check a trial emission for raw code, pinned slots and calls that depend on the
        // layout of adjacent nodes. These need the ordinary single-copy branch lowering.
        let prefix = self.em.take_text();
        let mut trial = self.clone();
        self.em.set_text(prefix);
        let strings = self.env.dialog_strings.borrow().clone();
        let mut declarations = Vec::new();
        let then_air = trial.capture_inline_arm(then_body, &mut declarations)?;
        let else_air = trial.capture_inline_arm(else_body, &mut declarations)?;
        // Raw code keeps the single-copy lowering: copying it to each return site could push a
        // tree past the node limit its author counted with one copy.
        if trial.raw_blocks != self.raw_blocks {
            *self.env.dialog_strings.borrow_mut() = strings;
            return Ok(false);
        }
        let (on_true, on_false) = if negated {
            (else_air, then_air)
        } else {
            (then_air, else_air)
        };
        let arms = Rc::new(InlineBranchArms {
            on_true,
            on_false,
            terminal,
        });
        if !trial.keep_layout && trial.pin_span.is_none() {
            trial.inline_helper_call_mode(
                &module,
                &decl,
                local,
                args,
                span,
                InlineCallMode::Branch(arms),
            )?;
        }
        if trial.keep_layout || trial.pin_span.is_some() {
            *self.env.dialog_strings.borrow_mut() = strings;
            return Ok(false);
        }
        for decl in declarations {
            trial.line(decl);
        }
        self.em.append(trial.em.text());
        trial.em.set_text(self.em.take_text());
        *self = trial;
        Ok(true)
    }

    pub(super) fn inline_helper_call(
        &mut self,
        fn_mod: &str,
        fn_decl: &FnDecl,
        is_local_fn: bool,
        args: &[CallArg],
        call_span: Span,
    ) -> Result<Option<(String, ValueType)>> {
        self.inline_helper_call_mode(
            fn_mod,
            fn_decl,
            is_local_fn,
            args,
            call_span,
            InlineCallMode::Materialize,
        )
    }

    /// An inline Result uses the same true/false edges as bool. An explicit
    /// `return error;` still exits the enclosing BHAV, as it does in other helpers.
    /// Preserve the meaning of existing user-defined types named Result.
    pub(super) fn helper_returns_condition(
        &self,
        module: &str,
        decl: &FnDecl,
        is_local: bool,
    ) -> bool {
        match decl.return_type.as_ref() {
            Some(V2Type::Bool) => true,
            Some(V2Type::Named(name)) if name == "Result" => {
                self.env.lookup_named_type(module, name, is_local).is_none()
            }
            _ => false,
        }
    }

    /// Whether a bool or Result helper ends in one `return <cond>;`, which
    /// inlines as a branch condition without the return-flag loop wrapper.
    pub(super) fn has_single_inline_tail_return(&self, fn_decl: &FnDecl) -> bool {
        let Some((V2Stmt::Return { value: Some(_), .. }, prefix)) = fn_decl.body.split_last()
        else {
            return false;
        };
        !stmts_have_return(prefix)
            && !prefix.iter().any(|s| matches!(s, V2Stmt::Defer { .. }))
            && (self.in_defer() || self.enclosing_cancel_flag().is_none())
    }

    /// Inlines a helper; with `TailCondition` (only for [`Self::has_single_inline_tail_return`]
    /// helpers) the prefix is emitted and the returned operand is the tail condition's `.air`
    /// text, to be used directly as the caller's branch condition.
    pub(super) fn inline_helper_call_mode(
        &mut self,
        fn_mod: &str,
        fn_decl: &FnDecl,
        is_local_fn: bool,
        args: &[CallArg],
        call_span: Span,
        mode: InlineCallMode<'_>,
    ) -> Result<Option<(String, ValueType)>> {
        let fqn = format!(
            "{}::{fn_mod}::{}",
            if is_local_fn { "local" } else { "mod" },
            fn_decl.name
        );
        if let Some(pos) = self.inline_call_stack.iter().position(|n| n == &fqn) {
            let mut chain: Vec<&str> = self.inline_call_stack[pos..]
                .iter()
                .map(String::as_str)
                .collect();
            chain.push(&fqn);
            return Err(Error::at(
                call_span,
                format!(
                    "recursive function call cycle detected: {}",
                    chain.join(" -> ")
                ),
            ));
        }
        if self.inline_call_stack.len() >= MAX_INLINE_DEPTH {
            return Err(Error::at(
                call_span,
                format!("helper function nesting exceeds maximum depth {MAX_INLINE_DEPTH}"),
            ));
        }
        self.inline_expansions += 1;
        if self.inline_expansions > MAX_INLINE_EXPANSIONS {
            return Err(Error::at(
                call_span,
                format!("helper function expansion budget ({MAX_INLINE_EXPANSIONS}) exceeded"),
            ));
        }

        // Resolve positional + named arguments against `fn_decl.params` (supporting default values).
        let mut supplied = vec![false; fn_decl.params.len()];
        let mut call_order: Vec<(usize, &CallArg)> = Vec::with_capacity(args.len());
        let mut seen_named = false;
        let mut positional_idx = 0usize;
        for arg in args {
            if let Some(ref arg_name) = arg.name {
                seen_named = true;
                let Some(param_idx) = fn_decl.params.iter().position(|p| &p.name == arg_name)
                else {
                    return Err(Error::at(
                        arg.span,
                        format!(
                            "function '{}' has no parameter named '{arg_name}'",
                            fn_decl.name
                        ),
                    ));
                };
                if supplied[param_idx] {
                    return Err(Error::at(
                        arg.span,
                        format!(
                            "duplicate argument for parameter '{arg_name}' in call to '{}'",
                            fn_decl.name
                        ),
                    ));
                }
                supplied[param_idx] = true;
                call_order.push((param_idx, arg));
            } else {
                if seen_named {
                    return Err(Error::at(
                        arg.span,
                        format!(
                            "positional argument cannot follow a named argument in call to '{}'",
                            fn_decl.name
                        ),
                    ));
                }
                if positional_idx >= fn_decl.params.len() {
                    return Err(Error::at(
                        call_span,
                        format!(
                            "function '{}' expects at most {} argument(s), found {}",
                            fn_decl.name,
                            fn_decl.params.len(),
                            args.len()
                        ),
                    ));
                }
                supplied[positional_idx] = true;
                call_order.push((positional_idx, arg));
                positional_idx += 1;
            }
        }
        for (i, param) in fn_decl.params.iter().enumerate() {
            if !supplied[i] && param.default.is_none() {
                return Err(Error::at(
                    call_span,
                    format!(
                        "missing required argument '{}' in call to '{}'",
                        param.name, fn_decl.name
                    ),
                ));
            }
        }

        // Evaluate caller-supplied arguments in call-site order BEFORE registering `fqn` on
        // `inline_call_stack` (so finite explicit nesting like `f(f(1))` succeeds).
        let mut bound_params: BTreeMap<String, Binding> = BTreeMap::new();
        let arg_values: Vec<&V2Expr> = args.iter().map(|a| &a.value).collect();
        for (param_idx, arg) in call_order {
            let param = &fn_decl.params[param_idx];
            let binding = self.bind_helper_arg(
                fn_mod,
                is_local_fn,
                fn_decl,
                param,
                &arg.value,
                arg.span,
                &arg_values,
            )?;
            bound_params.insert(param.name.clone(), binding);
        }

        // Register `fqn` on `inline_call_stack` BEFORE evaluating omitted default expressions
        // so recursive default parameters (e.g. `fn f(x: i16 = f())`) trigger cycle/depth detection.
        self.inline_call_sites
            .truncate(self.inline_call_stack.len());
        self.inline_call_sites
            .push((call_span.line, call_span.column));
        self.inline_call_stack.push(fqn);

        // Evaluate omitted defaulted parameters hygienically in the callee's module/object scope
        // (never capturing caller local variables or caller `run(actor)` Sim bindings).
        if supplied.iter().any(|&s| !s) {
            let prev_vars = std::mem::replace(&mut self.var_scopes, vec![BTreeMap::new()]);
            let prev_in_obj = self.in_object_scope;
            self.in_object_scope = is_local_fn;
            let default_res = self.with_owner_module(fn_mod, |c| {
                for (i, param) in fn_decl.params.iter().enumerate() {
                    if !supplied[i] {
                        let def_expr = param
                            .default
                            .as_ref()
                            .expect("missing required args rejected above");
                        let binding = c.bind_helper_arg(
                            fn_mod,
                            is_local_fn,
                            fn_decl,
                            param,
                            def_expr,
                            param.span,
                            &[],
                        )?;
                        bound_params.insert(param.name.clone(), binding);
                    }
                }
                Ok(())
            });
            self.in_object_scope = prev_in_obj;
            self.var_scopes = prev_vars;
            if let Err(e) = default_res {
                self.inline_call_stack.pop();
                return Err(e);
            }
        }

        let ret_kind = match &fn_decl.return_type {
            None => FnReturnKind::Void,
            Some(_) if self.helper_returns_condition(fn_mod, fn_decl, is_local_fn) => {
                FnReturnKind::Bool
            }
            Some(other) => {
                match self
                    .env
                    .resolve_value_type(other, fn_mod, is_local_fn, fn_decl.span)
                {
                    Ok(vty) => FnReturnKind::Value(vty),
                    Err(_) => {
                        self.inline_call_stack.pop();
                        return Err(Error::at(
                            fn_decl.span,
                            format!(
                                "helper function '{}' has unsupported return type '{}'",
                                fn_decl.name,
                                other.display_name()
                            ),
                        ));
                    }
                }
            }
        };
        if ret_kind != FnReturnKind::Void
            && !stmts_always_return(&fn_decl.body, self.env, fn_mod, is_local_fn)
        {
            self.inline_call_stack.pop();
            return Err(Error::at(
                fn_decl.span,
                format!(
                    "helper function '{}' declares '-> {}' but does not return on all control-flow paths",
                    fn_decl.name,
                    fn_decl
                        .return_type
                        .as_ref()
                        .expect("non-void has return_type")
                        .display_name()
                ),
            ));
        }

        if matches!(mode, InlineCallMode::TailCondition) {
            let prev_vars = std::mem::replace(&mut self.var_scopes, vec![bound_params]);
            let prev_active_fn = self.active_inline_fn.take();
            let prev_in_obj = self.in_object_scope;
            self.in_object_scope = is_local_fn;
            let res = self.with_owner_module(fn_mod, |c| {
                let Some((
                    V2Stmt::Return {
                        value: Some(tail), ..
                    },
                    prefix,
                )) = fn_decl.body.split_last()
                else {
                    unreachable!("checked by has_single_inline_tail_return");
                };
                c.push_scope(ScopeFrame {
                    cleanups: Vec::new(),
                    is_loop_body: false,
                    is_fn_root: true,
                    is_air_while: false,
                    owner_defer_id: c.current_defer_id,
                    cancel_flag: None,
                    restore_stack_on_break: false,
                    continue_step: None,
                });
                let r = c.lower_stmts(prefix).and_then(|()| {
                    if let Some(text) = c.try_lower_simple_air_cond(tail)? {
                        return Ok(text);
                    }
                    let cond = c.fresh("cond");
                    c.line(format!("let {cond} = 0;"));
                    c.emit_bool_into_local(tail, &cond)?;
                    Ok(format!("{cond} != 0"))
                });
                let p = c.pop_scope_and_run_cleanups();
                let text = r?;
                p?;
                Ok(text)
            });
            self.in_object_scope = prev_in_obj;
            self.active_inline_fn = prev_active_fn;
            self.var_scopes = prev_vars;
            self.inline_call_stack.pop();
            return Ok(Some((res?, ValueType::Bool)));
        }

        let has_returns = stmts_have_return(&fn_decl.body);
        let has_enclosing_cancel = !self.in_defer() && self.enclosing_cancel_flag().is_some();
        let branch = matches!(&mode, InlineCallMode::Branch(_));
        let needs_wrapper = if let InlineCallMode::Branch(arms) = &mode {
            !arms.terminal && !self.has_single_inline_tail_return(fn_decl)
        } else {
            has_returns || has_enclosing_cancel
        };
        let ret_val_local = match &mode {
            InlineCallMode::Into(dest) => {
                // Keep later source-local names stable when the result slot is elided.
                self.next_id += 1;
                self.line(format!("let {dest};"));
                Some((*dest).to_owned())
            }
            InlineCallMode::Materialize if ret_kind != FnReturnKind::Void => {
                let rv = self.fresh(&format!("ret_{}", fn_decl.name));
                self.line(format!("let {rv} = 0;"));
                Some(rv)
            }
            _ => None,
        };
        let ret_flag = if !branch && has_returns {
            let flag = self.fresh("fn_ret");
            if stmts_return_through_loop(&fn_decl.body) {
                self.line(format!("let {flag} = 0;"));
                Some(flag)
            } else {
                None
            }
        } else {
            None
        };
        let target = match mode {
            InlineCallMode::Branch(arms) => InlineReturnTarget::Branch {
                arms,
                break_after: needs_wrapper,
            },
            _ => ret_val_local
                .clone()
                .map_or(InlineReturnTarget::Void, InlineReturnTarget::Store),
        };

        let prev_vars = std::mem::replace(&mut self.var_scopes, vec![bound_params]);
        let prev_active_fn = self.active_inline_fn.take();
        let prev_in_obj = self.in_object_scope;
        self.in_object_scope = is_local_fn;
        if has_returns {
            self.active_inline_fn = Some(ActiveInlineFn {
                ret_flag,
                target,
                ret_kind: ret_kind.clone(),
                owner_defer_id: self.current_defer_id,
            });
        }

        let res = self.with_owner_module(fn_mod, |c| {
            if needs_wrapper {
                c.line("while true {".to_owned());
                c.em.indent();
            }
            c.push_scope(ScopeFrame {
                is_fn_root: true,
                is_air_while: needs_wrapper,
                owner_defer_id: c.current_defer_id,
                ..ScopeFrame::block()
            });
            let r = c.lower_stmts(&fn_decl.body);
            let p = c.pop_scope_and_run_cleanups();
            if needs_wrapper {
                c.line("break;".to_owned());
                c.em.dedent();
                c.line("}".to_owned());
            }
            r.and(p)
        });

        self.in_object_scope = prev_in_obj;
        self.active_inline_fn = prev_active_fn;
        self.var_scopes = prev_vars;
        self.inline_call_stack.pop();
        res?;
        if needs_wrapper {
            self.emit_cancel_propagation(false)?;
        }
        Ok(ret_val_local.map(|rv| {
            let vty = match ret_kind {
                FnReturnKind::Bool => ValueType::Bool,
                FnReturnKind::Value(v) => v,
                FnReturnKind::Void => unreachable!("ret_val_local only set for non-void"),
            };
            (rv, vty)
        }))
    }
}
