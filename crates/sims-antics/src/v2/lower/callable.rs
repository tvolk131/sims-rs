//! Calls to user callables (trees, functions with an ABI): resolution, argument preparation,
//! statement/value calls and value returns.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// Whether a user `fn`, `tree`, or `extern` answers to `callee` (it then wins over a builtin
    /// of the same name).
    pub(super) fn user_callable_named(&self, callee: &V2Expr, span: Span) -> Result<bool> {
        if self.resolve_callable(callee, span)?.is_some() {
            return Ok(true);
        }
        Ok(matches!(callee, V2Expr::Ident(_, name)
            if self.env.lookup_fn(&self.owner_module, name, self.in_object_scope).is_some()))
    }

    pub(super) fn resolve_callable(
        &self,
        callee: &V2Expr,
        span: Span,
    ) -> Result<Option<CallableSig>> {
        match callee {
            V2Expr::Ident(_, name) => {
                if self.in_object_scope
                    && let Some(sig) = self.env.trees.get(name)
                {
                    return Ok(Some(sig.clone()));
                }
                Ok(None)
            }
            V2Expr::Path(_, segs) if segs.len() == 2 => {
                let Some((decl_mod, is_local, decl)) =
                    self.env
                        .lookup_extern_mod(&self.owner_module, &segs[0], self.in_object_scope)
                else {
                    return Ok(None);
                };
                let f = decl.fns.iter().find(|f| f.name == segs[1]).ok_or_else(|| {
                    Error::at(
                        span,
                        format!("extern mod '{}' has no fn '{}'", segs[0], segs[1]),
                    )
                })?;
                self.env
                    .resolve_callable_sig(
                        format!("{}::{}", segs[0], segs[1]),
                        f.id,
                        CallContext::Extern,
                        &f.params,
                        &f.ret,
                        &decl_mod,
                        is_local,
                        f.span,
                    )
                    .map(Some)
            }
            _ => Ok(None),
        }
    }

    /// Evaluates call arguments, writes `@ temp[N]` / `call(temps)` operands, and returns the
    /// `.air` call text (`call ID`, `call ID(1, 2)`, or `call ID(temps)`).
    pub(super) fn prepare_callable_call(
        &mut self,
        sig: &CallableSig,
        args: &[CallArg],
        span: Span,
    ) -> Result<String> {
        let mut args = args;
        match sig.context {
            CallContext::SimOnObject => {
                if !matches!(self.tree_ctx, TreeContext::SimOnObject { .. }) {
                    return Err(Error::at(
                        span,
                        format!(
                            "{} runs in the Sim-on-object context and can only be called from interactions, functionals, or other Sim trees",
                            sig.display
                        ),
                    ));
                }
                let first_is_sim = args.first().is_some_and(|a| {
                    a.name.is_none()
                        && matches!(&a.value, V2Expr::Ident(_, n) if self.is_sim_ident(n))
                });
                if !first_is_sim {
                    return Err(Error::at(
                        span,
                        format!(
                            "{} expects the Sim handle as its first argument",
                            sig.display
                        ),
                    ));
                }
                args = &args[1..];
            }
            CallContext::ObjectSelf if !matches!(self.tree_ctx, TreeContext::ObjectSelf) => {
                return Err(Error::at(
                    span,
                    format!(
                        "{} runs in the object's own context (it has no Sim parameter) and can only be called from init/main/load/reset/cleanup or other object trees",
                        sig.display
                    ),
                ));
            }
            CallContext::ObjectSelf | CallContext::Inherit | CallContext::Extern => {}
        }

        // Match arguments to parameters (positional first, then by name).
        let n = sig.params.len();
        let mut slots: Vec<Option<&V2Expr>> = vec![None; n];
        // Parameter indices in source (evaluation) order.
        let mut source_order: Vec<usize> = Vec::with_capacity(n);
        let mut next_pos = 0usize;
        let mut seen_named = false;
        for a in args {
            match &a.name {
                None => {
                    if seen_named {
                        return Err(Error::at(
                            a.span,
                            format!(
                                "positional argument after named argument in call to {}",
                                sig.display
                            ),
                        ));
                    }
                    if next_pos >= n {
                        return Err(Error::at(
                            a.span,
                            format!("{} takes {n} argument(s), but more were given", sig.display),
                        ));
                    }
                    slots[next_pos] = Some(&a.value);
                    source_order.push(next_pos);
                    next_pos += 1;
                }
                Some(name) => {
                    seen_named = true;
                    let idx = sig
                        .params
                        .iter()
                        .position(|p| &p.name == name)
                        .ok_or_else(|| {
                            Error::at(
                                a.span,
                                format!("{} has no parameter named '{name}'", sig.display),
                            )
                        })?;
                    if slots[idx].is_some() {
                        return Err(Error::at(
                            a.span,
                            format!(
                                "duplicate argument for parameter '{name}' of {}",
                                sig.display
                            ),
                        ));
                    }
                    slots[idx] = Some(&a.value);
                    source_order.push(idx);
                }
            }
        }
        let mut values: Vec<&V2Expr> = Vec::with_capacity(n);
        for (i, slot) in slots.into_iter().enumerate() {
            values.push(slot.ok_or_else(|| {
                Error::at(
                    span,
                    format!(
                        "missing argument '{}' in call to {}",
                        sig.params[i].name, sig.display
                    ),
                )
            })?);
        }

        // Evaluate operands left to right in source order (named arguments may be reordered). An
        // operand is snapshotted into a fresh local when it reads a temp (about to be overwritten)
        // or a later-evaluated argument may run code (calls, foreign reads) that could change it.
        let mut operands: Vec<(String, Option<i16>)> = vec![(String::new(), None); n];
        for (k, &i) in source_order.iter().enumerate() {
            let expr = values[i];
            let (air, ty) = self.lower_value_expr(expr)?;
            let pty = &sig.params[i].ty;
            if !types_compatible_for_assign(pty, &ty) {
                return Err(Error::at(
                    expr.span(),
                    format!(
                        "argument '{}' of {} expects '{}', found '{}'",
                        sig.params[i].name,
                        sig.display,
                        self.env.display_value_type(pty),
                        self.env.display_value_type(&ty)
                    ),
                ));
            }
            let const_val = air.parse::<i16>().ok().or_else(|| {
                self.try_const_i16_air_operand(expr)
                    .ok()
                    .flatten()
                    .map(|(_, v)| v)
            });
            let later_impure = source_order[k + 1..]
                .iter()
                .any(|&j| !self.is_pure_single_operand(values[j]));
            // A runtime (`extern bcon`) read can fault on a missing chunk: read it now, in source
            // order, so a fault cannot follow a partial temp staging or an out-of-order read.
            let may_fault = air.starts_with("bcon[");
            operands[i] =
                if const_val.is_none() && (air.contains("temp[") || later_impure || may_fault) {
                    let snap = self.fresh("arg");
                    self.emit_let(&snap, &air);
                    (snap, None)
                } else {
                    (air, const_val)
                };
        }

        let positional: Vec<usize> = (0..n)
            .filter(|&i| sig.params[i].slot == AbiParamSlot::Positional)
            .collect();
        let literals: Option<Vec<i16>> = if positional.len() <= 4 {
            positional.iter().map(|&i| operands[i].1).collect()
        } else {
            None
        };
        let literals = literals.filter(|l| !(l.len() == 4 && l.iter().all(|&v| v == -1)));
        for (i, p) in sig.params.iter().enumerate() {
            let dst = match p.slot {
                AbiParamSlot::Temp(t) => usize::from(t),
                AbiParamSlot::Positional if literals.is_some() => continue,
                AbiParamSlot::Positional => positional
                    .iter()
                    .position(|&x| x == i)
                    .expect("positional index"),
            };
            let op = operands[i].0.clone();
            self.line(format!("temp[{dst}] := {op};"));
        }
        Ok(match literals {
            // An argless call is spelled `(temps)` (`[0xFF; 8]`), the authoring tools' encoding:
            // the callee sees the caller's temps as its arguments (the temps pass-through ABI).
            Some(l) if l.is_empty() => format!("call {}(temps)", sig.id),
            Some(l) => {
                // A callee frame wider than the call's four operand words reads the rest from
                // the instructions after the call, so the tree's layout must stay as lowered.
                if sig.frame_args.is_none_or(|n| n > 4) {
                    self.keep_layout = true;
                }
                let parts: Vec<String> = l.iter().map(i16::to_string).collect();
                format!("call {}({})", sig.id, parts.join(", "))
            }
            None => format!("call {}(temps)", sig.id),
        })
    }

    /// Emits `if !<call> { <all cleanups> return error; }` for `?` propagation.
    pub(super) fn emit_try_fail_branch(&mut self, call: &str, span: Span) -> Result<()> {
        if self.in_defer() {
            return Err(Error::at(span, "'?' cannot exit out of a 'defer' block"));
        }
        self.line(format!("if !{call} {{"));
        self.em.indent();
        self.emit_cleanups_down_to(0)?;
        self.line("return error;".to_owned());
        self.em.dedent();
        self.line("}".to_owned());
        Ok(())
    }

    /// Lowers a callable used as a statement (`f(..);` or `f(..)?;`).
    pub(super) fn emit_callable_stmt(
        &mut self,
        sig: &CallableSig,
        args: &[CallArg],
        span: Span,
        is_try: bool,
    ) -> Result<()> {
        match (&sig.ret, is_try) {
            (ResolvedReturnAbi::Result | ResolvedReturnAbi::ResultValue { .. }, false) => {
                Err(Error::at(
                    span,
                    format!(
                        "unused Result from {}: propagate failure with '?' or test it with 'if'",
                        sig.display
                    ),
                ))
            }
            (ResolvedReturnAbi::Void | ResolvedReturnAbi::Value { .. }, true) => Err(Error::at(
                span,
                format!(
                    "'?' needs a call returning 'bool' or 'Result', but {} does not",
                    sig.display
                ),
            )),
            (_, false) => {
                // Both exits continue: the result (if any) is ignored.
                let call = self.prepare_callable_call(sig, args, span)?;
                self.line(format!("if {call} {{"));
                self.line("}".to_owned());
                Ok(())
            }
            (_, true) => {
                let call = self.prepare_callable_call(sig, args, span)?;
                self.emit_try_fail_branch(&call, span)
            }
        }
    }

    /// Lowers a callable in value position. With `raw`, a `temp[N]` result is returned as-is
    /// (the caller must consume it before anything else can run).
    pub(super) fn lower_callable_value(
        &mut self,
        sig: &CallableSig,
        args: &[CallArg],
        span: Span,
        is_try: bool,
        raw: bool,
    ) -> Result<(String, ValueType)> {
        match (&sig.ret, is_try) {
            (ResolvedReturnAbi::Value { ty, temp }, false) => {
                let (ty, temp) = (ty.clone(), *temp);
                let call = self.prepare_callable_call(sig, args, span)?;
                self.line(format!("if {call} {{"));
                self.line("}".to_owned());
                Ok((self.capture_temp(temp, raw), ty))
            }
            (ResolvedReturnAbi::ResultValue { ty, temp }, true) => {
                let (ty, temp) = (ty.clone(), *temp);
                let call = self.prepare_callable_call(sig, args, span)?;
                self.emit_try_fail_branch(&call, span)?;
                Ok((self.capture_temp(temp, raw), ty))
            }
            (ResolvedReturnAbi::Bool, false) => {
                let call = self.prepare_callable_call(sig, args, span)?;
                let tmp = self.fresh("call");
                self.line(format!("let {tmp} = 0;"));
                self.line(format!("if {call} {{"));
                self.em.indent();
                self.emit_store(&tmp, ":=", "1");
                self.em.dedent();
                self.line("}".to_owned());
                Ok((tmp, ValueType::Bool))
            }
            (ResolvedReturnAbi::Result | ResolvedReturnAbi::ResultValue { .. }, false) => {
                Err(Error::at(
                    span,
                    format!(
                        "{} returns a Result: unwrap it with '?' or test it with 'if'",
                        sig.display
                    ),
                ))
            }
            (ResolvedReturnAbi::Void, false) => Err(Error::at(
                span,
                format!("{} does not return a value", sig.display),
            )),
            (_, true) => Err(Error::at(
                span,
                format!(
                    "'?' in value position needs a call returning 'Result<T @ temp[N]>', but {} does not",
                    sig.display
                ),
            )),
        }
    }

    pub(super) fn capture_temp(&mut self, temp: u8, raw: bool) -> String {
        if raw {
            return format!("temp[{temp}]");
        }
        let tmp = self.fresh("xret");
        self.line(format!("let {tmp} = temp[{temp}];"));
        tmp
    }

    /// If `expr` is a value-returning callable (`f(..)` with `-> T @ temp[N]`, or `f(..)?` with
    /// `-> Result<T @ temp[N]>`), lowers the call and returns the raw `temp[N]` operand.
    pub(super) fn try_lower_callable_value_raw(
        &mut self,
        expr: &V2Expr,
    ) -> Result<Option<(String, ValueType)>> {
        let (call_expr, is_try) = match expr {
            V2Expr::Try { inner, .. } => (inner.as_ref(), true),
            other => (other, false),
        };
        let V2Expr::Call {
            span,
            callee,
            type_args,
            args,
        } = call_expr
        else {
            return Ok(None);
        };
        if !type_args.is_empty() {
            return Ok(None);
        }
        let Some(sig) = self.resolve_callable(callee, *span)? else {
            return Ok(None);
        };
        if !matches!(
            (&sig.ret, is_try),
            (ResolvedReturnAbi::Value { .. }, false)
                | (ResolvedReturnAbi::ResultValue { .. }, true)
        ) {
            return Ok(None);
        }
        self.lower_callable_value(&sig, args, *span, is_try, true)
            .map(Some)
    }

    /// `true` if `name` names a BCON table / `extern bcon` block in scope. Table names take
    /// priority over same-named variables (matching the `stack_obj` clobber analysis); only the
    /// `self` / Sim handles are never treated as tables.
    pub(super) fn is_bcon_table_ref(&self, name: &str) -> bool {
        !self.is_self_ident(name)
            && !self.is_sim_ident(name)
            && self
                .env
                .is_bcon_table_name(&self.owner_module, name, self.in_object_scope)
    }

    /// Resolves `Table.entry` to its `bcon[addr]` operand (and compile-time value for private
    /// tables). `None` if `expr` is not a BCON table entry reference.
    pub(super) fn bcon_field_ref(&self, expr: &V2Expr) -> Option<Result<(String, Option<i16>)>> {
        let V2Expr::Field { span, base, field } = expr else {
            return None;
        };
        let V2Expr::Ident(_, bname) = base.as_ref() else {
            return None;
        };
        if !self.is_bcon_table_ref(bname) {
            return None;
        }
        self.env.lookup_bcon_entry(
            &self.owner_module,
            bname,
            field,
            self.in_object_scope,
            *span,
        )
    }

    /// Lowers `return [expr];` inside a body that returns its value in `temp[N]`.
    pub(super) fn lower_value_return(
        &mut self,
        span: Span,
        value: Option<&V2Expr>,
        abi: &ValueReturnAbi,
    ) -> Result<()> {
        let expected_ty = abi.ty.clone();
        let dest = format!("temp[{}]", abi.temp);
        let Some(raw_expr) = value else {
            return Err(Error::at(
                span,
                if abi.allow_none {
                    format!(
                        "{} declaring '-> {}' must return a value (or 'return false;' / 'return None;' on failure)",
                        abi.what,
                        self.env.display_value_type(&expected_ty)
                    )
                } else {
                    format!(
                        "{} declaring '-> {} @ {dest}' must return a value",
                        abi.what,
                        self.env.display_value_type(&expected_ty)
                    )
                },
            ));
        };
        if abi.allow_none
            && (matches!(raw_expr, V2Expr::Ident(_, n) if n == "None" && self.lookup_var(n).is_none())
                || (matches!(raw_expr, V2Expr::Bool(_, false)) && expected_ty != ValueType::Bool))
        {
            self.emit_all_cleanups()?;
            self.line("return false;".to_owned());
            return Ok(());
        }
        let inner_expr = match raw_expr {
            V2Expr::Call {
                callee,
                type_args,
                args,
                ..
            } if abi.allow_none
                && type_args.is_empty()
                && args.len() == 1
                && args[0].name.is_none()
                && matches!(callee.as_ref(), V2Expr::Ident(_, s) if s == "Some") =>
            {
                &args[0].value
            }
            other => other,
        };
        let type_err = |this: &Self, val_ty: &ValueType| {
            Error::at(
                span,
                format!(
                    "{} returning '{}' cannot return '{}' expression",
                    abi.what,
                    this.env.display_value_type(&expected_ty),
                    this.env.display_value_type(val_ty)
                ),
            )
        };
        if abi.in_place
            && !self.has_active_cleanups()
            && is_compound_arith_or_neg(inner_expr)
            && self.can_lower_arith_in_place(&dest, inner_expr, false)
        {
            let val_ty = self.lower_arith_into_dest(inner_expr, &dest, false)?;
            if !types_compatible_for_assign(&expected_ty, &val_ty) {
                return Err(type_err(self, &val_ty));
            }
            self.line("return true;".to_owned());
            return Ok(());
        }
        let (val_air, val_ty) = self.lower_value_expr(inner_expr)?;
        if !types_compatible_for_assign(&expected_ty, &val_ty) {
            return Err(type_err(self, &val_ty));
        }
        if self.has_active_cleanups() {
            let ret_snap = self.fresh("fret");
            self.emit_let(&ret_snap, &val_air);
            self.emit_all_cleanups()?;
            self.emit_store(&dest, ":=", &ret_snap);
        } else {
            self.emit_store(&dest, ":=", &val_air);
        }
        self.line("return true;".to_owned());
        Ok(())
    }
}
