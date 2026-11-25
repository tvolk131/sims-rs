//! Conditions and control flow: simple `.air` conditions, comparisons, bitflag predicates, booleans
//! into locals, `if`, blocks, short circuits and boolean returns.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// Returns `true` iff lowering `expr` emits zero setup statements and has zero side effects.
    pub(super) fn is_pure_single_operand(&self, expr: &V2Expr) -> bool {
        match expr {
            V2Expr::Int(_, _) | V2Expr::Bool(_, _) | V2Expr::Path(_, _) => true,
            V2Expr::Ident(_, name) => !matches!(
                self.lookup_var(name),
                Some(
                    Binding::AutonomousFlag
                        | Binding::MutRef {
                            target: LValue::ForeignAttr { .. } | LValue::ForeignCell { .. }
                        }
                )
            ),
            V2Expr::Unary {
                op: V2UnaryOp::Neg,
                inner,
                ..
            } => matches!(inner.as_ref(), V2Expr::Int(_, _)),
            V2Expr::Deref { inner, .. } => {
                if let V2Expr::Ident(_, pname) = inner.as_ref() {
                    matches!(
                        self.lookup_var(pname),
                        Some(Binding::MutRef {
                            target: LValue::Direct { .. }
                        })
                    )
                } else {
                    false
                }
            }
            V2Expr::Field { base, .. } => match base.as_ref() {
                // `stack_obj.<field>` is the stack object's own cell (no switch).
                V2Expr::Ident(_, bname) => {
                    (self.is_self_ident(bname) && self.self_stack_target() == "stack_obj")
                        || self.is_sim_ident(bname)
                        || self.is_bcon_table_ref(bname)
                        || bname == "stack_obj"
                }
                V2Expr::Field { base: root, .. } => {
                    matches!(root.as_ref(), V2Expr::Ident(_, rname) if self.is_sim_ident(rname))
                }
                _ => false,
            },
            V2Expr::Index { base, index, .. } => {
                if self.eval_const_i16(index).is_err() {
                    return false;
                }
                match base.as_ref() {
                    V2Expr::Ident(_, bname) => {
                        matches!(bname.as_str(), "temp" | "global")
                            || (bname == "param" && self.lookup_var("param").is_none())
                    }
                    V2Expr::Field {
                        base: root, field, ..
                    } => {
                        field != "sprite_flags"
                            && matches!(
                                root.as_ref(),
                                // In a Sim tree that moves the cursor, `self.f[i]` switches to
                                // the entry object first, like the named fields above.
                                V2Expr::Ident(_, rname) if (self.is_self_ident(rname)
                                        && self.self_stack_target() == "stack_obj")
                                    || self.is_sim_ident(rname)
                                    || self.is_frame_me(rname)
                                    || rname == "stack_obj"
                            )
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Returns `true` iff `expr` can be lowered directly into an `.air` `while <cond>` header
    /// without hoisting any setup statements before the loop!
    pub(super) fn is_pure_simple_air_cond(&self, expr: &V2Expr) -> bool {
        match expr {
            V2Expr::Bool(_, _) => true,
            V2Expr::Ident(_, name)
                if matches!(self.lookup_var(name), Some(Binding::AutonomousFlag)) =>
            {
                true
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.is_pure_simple_air_cond(inner),
            V2Expr::Binary { lhs, op, rhs, .. } => match op {
                V2BinOp::Eq
                | V2BinOp::Ne
                | V2BinOp::Lt
                | V2BinOp::Le
                | V2BinOp::Gt
                | V2BinOp::Ge
                | V2BinOp::FlagTest => {
                    self.is_pure_single_operand(lhs) && self.is_pure_single_operand(rhs)
                }
                _ => false,
            },
            V2Expr::MethodCall {
                receiver,
                method,
                args,
                ..
            } if method == "is_empty" && args.is_empty() => self.is_pure_single_operand(receiver),
            // These cursor operations are a single condition node with no hoisted setup.
            V2Expr::Call {
                span,
                callee,
                type_args,
                args,
            } if matches!(callee.as_ref(), V2Expr::Ident(_, name)
                    if (name == "set_to_next" && type_args.is_empty())
                        || (name == "create_object" && has_create_dest(args))) =>
            {
                !self.user_callable_named(callee, *span).unwrap_or(true)
            }
            // `inc_lt(&mut x, n)` / `dec_gt(&mut x, n)` on a direct slot is one node.
            V2Expr::Call {
                span,
                callee,
                type_args,
                args,
            } if type_args.is_empty()
                && matches!(callee.as_ref(), V2Expr::Ident(_, f) if matches!(f.as_str(), "inc_lt" | "dec_gt"))
                && !self.user_callable_named(callee, *span).unwrap_or(true) =>
            {
                matches!(
                    args.as_slice(),
                    [place, bound] if place.name.is_none()
                        && bound.name.is_none()
                        && matches!(&place.value, V2Expr::MutRef { inner, .. } if self.is_pure_single_operand(inner))
                        && self.is_pure_single_operand(&bound.value)
                )
            }
            V2Expr::Call { .. } | V2Expr::MethodCall { .. } => false,
            _ => self.is_pure_single_operand(expr),
        }
    }

    pub(super) fn lower_cmp_operands(
        &mut self,
        lhs: &V2Expr,
        op: V2BinOp,
        rhs: &V2Expr,
        span: Span,
    ) -> Result<(String, String)> {
        // `job_data(..) <op> k` with a constant `k`: the comparison node reads the career cell
        // right after its staging, so it needs no copy. The VM resolves the right operand
        // first, so `k` must be one that cannot fault (a temp or attribute can), or the
        // comparison would fault where the copy faulted on the career lookup.
        let job_cell = match self.builtin_job_data_call(lhs)? {
            Some(call) if self.eval_const_i16(rhs).is_ok() => Some(call),
            _ => None,
        };
        let (l, l_ty) = match job_cell {
            Some((args, jspan)) => self.lower_job_data_cell(args, jspan)?,
            None => self.lower_value_expr(lhs)?,
        };
        if op == V2BinOp::FlagTest
            && let ValueType::Bitflags(ref key) = l_ty
        {
            let Some((_, Some(bit))) = self.try_eval_const_bitflag(rhs, key)? else {
                return Err(Error::at(
                    rhs.span(),
                    format!(
                        "'flag_test' on bitflags '{}' requires a single flag constant (use '.contains(...)' for masks)",
                        self.env.display_value_type(&l_ty)
                    ),
                ));
            };
            return Ok((l, bit.to_string()));
        }
        let (l_eff, r, r_ty) = if self.is_pure_single_operand(rhs) {
            let (r, r_ty) = self.lower_value_expr(rhs)?;
            (l, r, r_ty)
        } else {
            let snap_l = self.fresh("cmp_l");
            self.emit_let(&snap_l, &l);
            let (r, r_ty) = self.lower_value_expr(rhs)?;
            (snap_l, r, r_ty)
        };
        match op {
            V2BinOp::Eq | V2BinOp::Ne => {
                if !types_compatible_for_assign(&l_ty, &r_ty)
                    && !types_compatible_for_assign(&r_ty, &l_ty)
                {
                    return Err(Error::at(
                        span,
                        format!(
                            "cannot compare '{}' with '{}'",
                            self.env.display_value_type(&l_ty),
                            self.env.display_value_type(&r_ty)
                        ),
                    ));
                }
            }
            V2BinOp::Lt | V2BinOp::Le | V2BinOp::Gt | V2BinOp::Ge => {
                if !matches!(l_ty, ValueType::I16 | ValueType::ObjectId)
                    || !matches!(r_ty, ValueType::I16 | ValueType::ObjectId)
                {
                    return Err(Error::at(
                        span,
                        format!(
                            "ordering comparison is not supported between '{}' and '{}'",
                            self.env.display_value_type(&l_ty),
                            self.env.display_value_type(&r_ty)
                        ),
                    ));
                }
            }
            V2BinOp::FlagTest if l_ty != ValueType::I16 || r_ty != ValueType::I16 => {
                return Err(Error::at(
                    span,
                    format!(
                        "'flag_test' requires matching bitflags or i16 operands, found '{}' and '{}'",
                        self.env.display_value_type(&l_ty),
                        self.env.display_value_type(&r_ty)
                    ),
                ));
            }
            _ => {}
        }
        Ok((l_eff, r))
    }

    pub(super) fn lower_bitflag_predicate(
        &mut self,
        receiver: &V2Expr,
        method: &str,
        args: &[CallArg],
        span: Span,
        negated: bool,
    ) -> Result<String> {
        let derefed = self.maybe_auto_deref_mut_ref(receiver);
        let (lhs_air, lhs_ty) = self.lower_value_expr(derefed.as_ref().unwrap_or(receiver))?;
        let ValueType::Bitflags(key) = lhs_ty else {
            return Err(Error::at(
                span,
                format!(
                    "'.{method}(...)' requires a bitflags receiver, found '{}'",
                    self.env.display_value_type(&lhs_ty)
                ),
            ));
        };
        if method == "is_empty" {
            if !args.is_empty() {
                return Err(Error::at(span, "'.is_empty()' takes no arguments"));
            }
            return Ok(if negated {
                format!("{lhs_air} != 0")
            } else {
                format!("{lhs_air} == 0")
            });
        }
        if args.len() != 1 || args[0].name.is_some() {
            return Err(Error::at(
                span,
                format!("'.{method}(flag)' expects exactly 1 positional argument"),
            ));
        }
        if let Some((mask, single_bit)) = self.try_eval_const_bitflag(&args[0].value, &key)? {
            if let Some(bit) = single_bit {
                return Ok(if negated {
                    format!("!{lhs_air} flag_test {bit}")
                } else {
                    format!("{lhs_air} flag_test {bit}")
                });
            }
            if mask == 0 {
                let res = method == "contains";
                let effective = if negated { !res } else { res };
                return Ok(if effective { "1 == 1" } else { "1 == 0" }.to_owned());
            }
            let tmp = self.fresh("bf_tst");
            self.emit_let(&tmp, &lhs_air);
            self.line(format!("{tmp} &= {mask};"));
            return Ok(if method == "contains" {
                if negated {
                    format!("{tmp} != {mask}")
                } else {
                    format!("{tmp} == {mask}")
                }
            } else if negated {
                format!("{tmp} == 0")
            } else {
                format!("{tmp} != 0")
            });
        }
        let lhs_snap = if self.is_pure_single_operand(&args[0].value) {
            lhs_air
        } else {
            let s = self.fresh("bf_lhs");
            self.emit_let(&s, &lhs_air);
            s
        };
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
        let tmp = self.fresh("bf_tst");
        self.emit_let(&tmp, &lhs_snap);
        self.line(format!("{tmp} &= {rhs_air};"));
        Ok(if method == "contains" {
            if negated {
                format!("{tmp} != {rhs_air}")
            } else {
                format!("{tmp} == {rhs_air}")
            }
        } else if negated {
            format!("{tmp} == 0")
        } else {
            format!("{tmp} != 0")
        })
    }

    /// Tries to lower a boolean condition into a single `.air` `Condition` string (no `&&`/`||`).
    pub(super) fn try_lower_simple_air_cond(&mut self, expr: &V2Expr) -> Result<Option<String>> {
        match expr {
            V2Expr::Bool(_, true) => Ok(Some("1 == 1".to_owned())),
            V2Expr::Bool(_, false) => Ok(Some("1 == 0".to_owned())),
            V2Expr::Ident(_, name)
                if matches!(self.lookup_var(name), Some(Binding::AutonomousFlag)) =>
            {
                Ok(Some("param[0] == 1".to_owned()))
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => {
                // A written `!(a > b)` keeps its compare: `.air` `!a > b` is the same compare
                // node with its exits swapped, so the node's own result (the thread's last
                // result) matches the source, where `a <= b` would invert it.
                if let V2Expr::Binary {
                    span: ispan,
                    lhs,
                    op:
                        op @ (V2BinOp::Eq
                        | V2BinOp::Ne
                        | V2BinOp::Lt
                        | V2BinOp::Le
                        | V2BinOp::Gt
                        | V2BinOp::Ge),
                    rhs,
                } = inner.as_ref()
                {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *ispan)?;
                    return Ok(Some(format!("!{l} {} {r}", cmp_op_str(*op))));
                }
                if let Some(neg) = self.try_lower_negated_simple_air_cond(inner)? {
                    return Ok(Some(neg));
                }
                Ok(None)
            }
            V2Expr::Binary { span, lhs, op, rhs } => match op {
                V2BinOp::And | V2BinOp::Or => Ok(None),
                V2BinOp::Eq
                | V2BinOp::Ne
                | V2BinOp::Lt
                | V2BinOp::Le
                | V2BinOp::Gt
                | V2BinOp::Ge
                | V2BinOp::FlagTest => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    let sym = cmp_op_str(*op);
                    Ok(Some(format!("{l} {sym} {r}")))
                }
                _ => Err(Error::at(
                    expr.span(),
                    "arithmetic expression cannot be used directly as a boolean condition",
                )),
            },
            V2Expr::Call {
                span,
                callee,
                type_args,
                args,
            } => {
                if matches!(callee.as_ref(), V2Expr::Ident(_, name) if name == "create_object")
                    && has_create_dest(args)
                    && !self.user_callable_named(callee, *span)?
                {
                    return Ok(Some(self.format_create_object(type_args, args, *span)?.0));
                }
                if !type_args.is_empty() {
                    return Ok(None);
                }
                if let Some(sig) = self.resolve_callable(callee, *span)? {
                    return match sig.ret {
                        ResolvedReturnAbi::Bool | ResolvedReturnAbi::Result => {
                            Ok(Some(self.prepare_callable_call(&sig, args, *span)?))
                        }
                        _ => Err(Error::at(
                            *span,
                            format!(
                                "{} cannot be used as a condition (only '-> bool' and '-> Result' calls can)",
                                sig.display
                            ),
                        )),
                    };
                }
                if let V2Expr::Ident(_, fname) = callee.as_ref() {
                    match fname.as_str() {
                        "can_afford" => {
                            return Ok(Some(self.format_alter_budget(args, *span, true, false)?));
                        }
                        "debit_funds" => {
                            return Ok(Some(self.format_alter_budget(args, *span, false, false)?));
                        }
                        "credit_funds" => {
                            return Ok(Some(self.format_alter_budget(args, *span, false, true)?));
                        }
                        "dialog" => return Ok(Some(self.format_dialog(args, *span)?)),
                        "route_to" => {
                            return Ok(Some(self.format_slot_prim("route_to", args, *span)?));
                        }
                        "snap" => return Ok(Some(self.format_slot_prim("snap", args, *span)?)),
                        _ => {}
                    }
                    if let Some((fn_mod, fn_decl, is_local)) =
                        self.env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                        && self.helper_returns_condition(&fn_mod, &fn_decl, is_local)
                        && self.has_single_inline_tail_return(&fn_decl)
                    {
                        let (text, _) = self
                            .inline_helper_call_mode(
                                &fn_mod,
                                &fn_decl,
                                is_local,
                                args,
                                *span,
                                InlineCallMode::TailCondition,
                            )?
                            .expect("tail bool helpers return a condition");
                        return Ok(Some(text));
                    }
                    if self
                        .env
                        .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                        .is_none()
                    {
                        return self.format_prim_builtin(fname, args, *span);
                    }
                    return Ok(None);
                }
                Ok(None)
            }
            V2Expr::MethodCall { method, .. } if method == "run_functional" => Ok(None),
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if method == "is_type" && type_args.is_empty() => {
                Ok(Some(self.format_is_type(receiver, args, *span)?))
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if matches!(method.as_str(), "contains" | "intersects" | "is_empty") => {
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        format!("'.{method}()' does not accept type arguments"),
                    ));
                }
                Ok(Some(self.lower_bitflag_predicate(
                    receiver, method, args, *span, false,
                )?))
            }
            _ => {
                let (val, ty) = self.lower_value_expr(expr)?;
                if ty != ValueType::Bool {
                    return Err(Error::at(
                        expr.span(),
                        "expected boolean condition (use an explicit comparison like 'x != 0')",
                    ));
                }
                Ok(Some(format!("{val} != 0")))
            }
        }
    }

    pub(super) fn try_lower_negated_simple_air_cond(
        &mut self,
        expr: &V2Expr,
    ) -> Result<Option<String>> {
        match expr {
            V2Expr::Bool(_, true) => Ok(Some("1 == 0".to_owned())),
            V2Expr::Bool(_, false) => Ok(Some("1 == 1".to_owned())),
            V2Expr::Ident(_, name)
                if matches!(self.lookup_var(name), Some(Binding::AutonomousFlag)) =>
            {
                Ok(Some("param[0] != 1".to_owned()))
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.try_lower_simple_air_cond(inner),
            V2Expr::Binary { span, lhs, op, rhs } => match op {
                V2BinOp::And | V2BinOp::Or => Ok(None),
                V2BinOp::Eq => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} != {r}")))
                }
                V2BinOp::Ne => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} == {r}")))
                }
                V2BinOp::Lt => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} >= {r}")))
                }
                V2BinOp::Le => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} > {r}")))
                }
                V2BinOp::Gt => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} <= {r}")))
                }
                V2BinOp::Ge => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("{l} < {r}")))
                }
                V2BinOp::FlagTest => {
                    let (l, r) = self.lower_cmp_operands(lhs, *op, rhs, *span)?;
                    Ok(Some(format!("!{l} flag_test {r}")))
                }
                _ => Err(Error::at(
                    expr.span(),
                    "cannot apply '!' to arithmetic expression",
                )),
            },
            V2Expr::Call { .. } => {
                if let Some(pos) = self.try_lower_simple_air_cond(expr)? {
                    return Ok(Some(format!("!{pos}")));
                }
                Ok(None)
            }
            V2Expr::MethodCall { method, .. } if method == "run_functional" => Ok(None),
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if method == "is_type" && type_args.is_empty() => Ok(Some(format!(
                "!{}",
                self.format_is_type(receiver, args, *span)?
            ))),
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if matches!(method.as_str(), "contains" | "intersects" | "is_empty") => {
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        format!("'.{method}()' does not accept type arguments"),
                    ));
                }
                Ok(Some(self.lower_bitflag_predicate(
                    receiver, method, args, *span, true,
                )?))
            }
            _ => {
                let (val, ty) = self.lower_value_expr(expr)?;
                if ty != ValueType::Bool {
                    return Err(Error::at(
                        expr.span(),
                        "cannot apply '!' to non-bool expression",
                    ));
                }
                Ok(Some(format!("{val} == 0")))
            }
        }
    }

    /// Evaluates an arbitrary boolean expression (including short-circuit `&&`, `||`, `!`,
    /// helper calls, and `run_functional`) into `dst_local` (`0` or `1`).
    pub(super) fn emit_bool_into_local(&mut self, expr: &V2Expr, dst_local: &str) -> Result<()> {
        if let Some(b) = self.try_eval_const_bool(expr) {
            self.emit_store(dst_local, ":=", if b { "1" } else { "0" });
            return Ok(());
        }
        match expr {
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => {
                // Short-circuit `lhs && rhs`: evaluate rhs ONLY when lhs is true!
                self.emit_bool_into_local(lhs, dst_local)?;
                self.line(format!("if {dst_local} != 0 {{"));
                self.em.indent();
                self.emit_bool_into_local(rhs, dst_local)?;
                self.em.dedent();
                self.line("}".to_owned());
                Ok(())
            }
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => {
                // Short-circuit `lhs || rhs`: evaluate rhs ONLY when lhs is false!
                self.emit_bool_into_local(lhs, dst_local)?;
                self.line(format!("if {dst_local} == 0 {{"));
                self.em.indent();
                self.emit_bool_into_local(rhs, dst_local)?;
                self.em.dedent();
                self.line("}".to_owned());
                Ok(())
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => {
                if let Some(neg) = self.try_lower_negated_simple_air_cond(inner)? {
                    self.line(format!("if {neg} {{"));
                    self.em.indent();
                    self.emit_store(dst_local, ":=", "1");
                    self.em.dedent();
                    self.line("} else {".to_owned());
                    self.em.indent();
                    self.emit_store(dst_local, ":=", "0");
                    self.em.dedent();
                    self.line("}".to_owned());
                } else {
                    self.emit_bool_into_local(inner, dst_local)?;
                    self.line(format!("if {dst_local} == 0 {{"));
                    self.em.indent();
                    self.emit_store(dst_local, ":=", "1");
                    self.em.dedent();
                    self.line("} else {".to_owned());
                    self.em.indent();
                    self.emit_store(dst_local, ":=", "0");
                    self.em.dedent();
                    self.line("}".to_owned());
                }
                Ok(())
            }
            V2Expr::MethodCall {
                span,
                receiver,
                method,
                type_args,
                args,
            } if method == "run_functional" => {
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        "type arguments on run_functional are only valid inside 'let Some(x) = obj.run_functional::<T>(...)'",
                    ));
                }
                self.emit_run_functional(receiver, args, *span, Some(dst_local))
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
                    // `if create_object(...)`: success flag only; `stack_obj` gets back its
                    // pre-call value on both edges (bind the new object with
                    // `let Some(x) = create_object(...)`).
                    let (prim, _) = self.format_create_object(type_args, args, *span)?;
                    let saved = if has_create_dest(args) {
                        None
                    } else {
                        let pre = self.fresh("pre_stack_obj");
                        self.stack_op(mir::StackObjOp::Save(&pre));
                        Some(pre)
                    };
                    self.line(format!("if {prim} {{"));
                    self.em.indent();
                    self.emit_store(dst_local, ":=", "1");
                    self.em.dedent();
                    self.line("} else {".to_owned());
                    self.em.indent();
                    self.emit_store(dst_local, ":=", "0");
                    self.em.dedent();
                    self.line("}".to_owned());
                    if let Some(saved) = saved {
                        self.stack_op(mir::StackObjOp::Set(&saved));
                    }
                    return Ok(());
                }
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        "type arguments are only supported on 'create_object::<T>', 'find_best_object::<T>', and 'objects_of::<T>'",
                    ));
                }
                if let Some(simple) = self.try_lower_simple_air_cond(expr)? {
                    self.line(format!("if {simple} {{"));
                    self.em.indent();
                    self.emit_store(dst_local, ":=", "1");
                    self.em.dedent();
                    self.line("} else {".to_owned());
                    self.em.indent();
                    self.emit_store(dst_local, ":=", "0");
                    self.em.dedent();
                    self.line("}".to_owned());
                    return Ok(());
                }
                if let V2Expr::Ident(_, fname) = callee.as_ref()
                    && let Some((fn_mod, fn_decl, is_local)) =
                        self.env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                {
                    let (rv, vty) = self
                        .inline_helper_call(&fn_mod, &fn_decl, is_local, args, *span)?
                        .ok_or_else(|| {
                            Error::at(
                                *span,
                                format!("void helper function '{fname}' cannot be used as bool"),
                            )
                        })?;
                    if vty != ValueType::Bool {
                        let actual = self.env.display_value_type(&vty);
                        return Err(Error::at(
                            *span,
                            format!(
                                "helper function '{fname}' returns '{actual}' and cannot be used directly as a bool condition (use an explicit comparison)"
                            ),
                        ));
                    }
                    self.emit_store(dst_local, ":=", &rv);
                    return Ok(());
                }
                Err(Error::at(*span, "unsupported boolean call expression"))
            }
            _ => {
                let simple = self
                    .try_lower_simple_air_cond(expr)?
                    .ok_or_else(|| Error::at(expr.span(), "expected boolean expression"))?;
                self.line(format!("if {simple} {{"));
                self.em.indent();
                self.emit_store(dst_local, ":=", "1");
                self.em.dedent();
                self.line("} else {".to_owned());
                self.em.indent();
                self.emit_store(dst_local, ":=", "0");
                self.em.dedent();
                self.line("}".to_owned());
                Ok(())
            }
        }
    }

    /// Emits direct, zero-temporary short-circuit `return true` / `return false` for
    /// `guard(sim) => <expr>` and cleanup-free `return <expr>;`.
    /// Lowers a block in its own scope.
    pub(super) fn lower_block(&mut self, stmts: &[V2Stmt]) -> Result<()> {
        self.push_scope(ScopeFrame::block());
        let r = self.lower_stmts(stmts);
        let p = self.pop_scope_and_run_cleanups();
        r.and(p)
    }

    /// One branch of an `if` statement (a block) or `if` expression (a value into `result`).
    pub(super) fn lower_branch(
        &mut self,
        body: ArmBody<'_>,
        result: Option<&mut MatchResult>,
    ) -> Result<()> {
        match body {
            ArmBody::Stmts(stmts) => self.lower_block(stmts),
            ArmBody::Value(_) => self.lower_match_arm(body, result),
        }
    }

    /// `if c { a } else { b }` as a value, written to `dest` (or a fresh local). The condition
    /// takes the `if` statement's paths: constant fold (the untaken branch is still checked),
    /// short-circuit `||` / `&&`, a simple test, or a `__cond` local.
    pub(super) fn lower_if_expr(
        &mut self,
        span: Span,
        cond: &V2Expr,
        then_value: &V2Expr,
        else_value: &V2Expr,
        dest: Option<&str>,
    ) -> Result<(String, ValueType)> {
        let dest = match dest {
            Some(d) => d.to_owned(),
            None => {
                let d = self.fresh("ifv");
                self.line(format!("let {d};"));
                d
            }
        };
        let mut result = MatchResult {
            dest: dest.clone(),
            ty: None,
            if_expr: true,
        };
        let (then_b, else_b) = (ArmBody::Value(then_value), ArmBody::Value(else_value));
        if let Some(const_b) = self
            .try_eval_const_bool(cond)
            .or_else(|| self.try_fold_literal_cond(cond))
        {
            self.check_folded_cond(cond)?;
            if const_b {
                self.lower_match_arm(then_b, Some(&mut result))?;
                self.check_match_arm(else_b, Some(&mut result))?;
            } else {
                self.check_match_arm(then_b, Some(&mut result))?;
                self.lower_match_arm(else_b, Some(&mut result))?;
            }
        } else if !self.try_lower_short_circuit_if(cond, then_b, else_b, Some(&mut result))? {
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
            let r1 = self.lower_match_arm(then_b, Some(&mut result));
            self.em.dedent();
            r1?;
            self.line("} else {".to_owned());
            self.em.indent();
            let r2 = self.lower_match_arm(else_b, Some(&mut result));
            self.em.dedent();
            r2?;
            self.line("}".to_owned());
        }
        let ty = result
            .ty
            .ok_or_else(|| Error::at(span, "'if' expression has no value"))?;
        Ok((dest, ty))
    }

    /// Whether `if` bodies may sit inside a condition-free single-pass loop: nothing in them,
    /// or in what lowering adds, may `break` / `continue` an enclosing loop.
    pub(super) fn single_pass_loop_ok(&self, bodies: &[&[V2Stmt]]) -> bool {
        self.active_inline_fn.is_none()
            && !self.in_defer()
            && self.enclosing_cancel_flag().is_none()
            && !bodies.iter().any(|b| stmts_exit_loop(b))
    }

    /// `if` on a flat `a || b || ..` or `a && b && ..` of simple conditions, without a `__cond`
    /// local. Each test keeps its place in evaluation order:
    /// - `a || b`: `while true { if !a { if !b { E; break; } } T; break; }`
    /// - `a && b` with `else`: `while true { if a { if b { T; break; } } E; break; }`
    /// - `a && b` without `else`: `if a { if b { T } }`
    ///
    /// Returns `false`, with nothing emitted, when the shape does not apply.
    pub(super) fn try_lower_short_circuit_if(
        &mut self,
        cond: &V2Expr,
        then_body: ArmBody<'_>,
        else_body: ArmBody<'_>,
        mut result: Option<&mut MatchResult>,
    ) -> Result<bool> {
        let V2Expr::Binary { op, .. } = cond else {
            return Ok(false);
        };
        let op = *op;
        if !matches!(op, V2BinOp::And | V2BinOp::Or) {
            return Ok(false);
        }
        let mut leaves = Vec::new();
        flatten_logic_chain(cond, op, &mut leaves);
        let has_else = !matches!(else_body, ArmBody::Stmts(s) if s.is_empty());
        let looped = op == V2BinOp::Or || has_else;
        if looped && !self.single_pass_loop_ok(&[arm_stmts(then_body), arm_stmts(else_body)]) {
            return Ok(false);
        }
        let (mark, indent) = (self.em.mark(), self.em.depth());
        if looped {
            self.line("while true {".to_owned());
            self.em.indent();
        }
        for leaf in &leaves {
            let Some(text) = self.try_lower_simple_air_cond(leaf)? else {
                self.em.truncate(mark);
                self.em.set_depth(indent);
                return Ok(false);
            };
            let text = if op == V2BinOp::Or {
                negate_air_cond(&text)
            } else {
                text
            };
            self.line(format!("if {text} {{"));
            self.em.indent();
        }
        let (inner, outer) = if op == V2BinOp::Or {
            (else_body, then_body)
        } else {
            (then_body, else_body)
        };
        self.lower_branch(inner, result.as_deref_mut())?;
        if looped {
            self.line("break;".to_owned());
        }
        for _ in &leaves {
            self.em.dedent();
            self.line("}".to_owned());
        }
        if looped {
            self.lower_branch(outer, result)?;
            self.line("break;".to_owned());
            self.em.dedent();
            self.line("}".to_owned());
        }
        Ok(true)
    }

    /// Evaluates a helper return before emitting the matching cleanup and caller edge.
    /// A comparison keeps its own polarity, and logical operators short-circuit.
    pub(super) fn emit_inline_bool_edges(
        &mut self,
        expr: &V2Expr,
        terminal: bool,
        edge: &mut dyn FnMut(&mut Self, bool) -> Result<()>,
    ) -> Result<()> {
        if let Some(value) = self.try_eval_const_bool(expr) {
            return edge(self, value);
        }
        if let Some(value) = self.try_fold_literal_cond(expr) {
            self.check_folded_cond(expr)?;
            return edge(self, value);
        }
        match expr {
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => {
                return self.emit_inline_bool_edges(inner, terminal, &mut |c, b| edge(c, !b));
            }
            V2Expr::Binary {
                lhs,
                op: op @ (V2BinOp::And | V2BinOp::Or),
                rhs,
                ..
            } => {
                if self.try_fold_literal_cond(lhs) == Some(*op == V2BinOp::Or) {
                    self.check_folded_cond(rhs)?;
                }
                return self.emit_inline_bool_edges(lhs, terminal, &mut |c, b| {
                    if b == (*op == V2BinOp::And) {
                        c.emit_inline_bool_edges(rhs, terminal, edge)
                    } else {
                        edge(c, b)
                    }
                });
            }
            _ => {}
        }

        // `Some(saved)` after a `run_functional` test: each edge first restores the stack object
        // (to `saved`, or to the entry object when `saved` is `None`).
        let (test, restore) = if let Some((_, receiver, args, span)) = run_functional_test(expr) {
            let PreparedFunctional {
                prim,
                saved_stack_obj,
                ..
            } = self.prepare_run_functional(receiver, args, span)?;
            (prim, Some(saved_stack_obj))
        } else if let Some(test) = self.try_lower_simple_air_cond(expr)? {
            (test, None)
        } else {
            let local = self.fresh("cond");
            self.line(format!("let {local};"));
            self.emit_bool_into_local(expr, &local)?;
            (format!("{local} != 0"), None)
        };
        self.line(format!("if {test} {{"));
        self.em.indent();
        if let Some(saved) = &restore {
            self.emit_restore_stack_obj(saved.as_deref());
        }
        edge(self, true)?;
        self.em.dedent();
        if terminal {
            self.line("}".to_owned());
        } else {
            let else_at = self.em.mark();
            self.line("} else {".to_owned());
            self.em.indent();
            let arm_at = self.em.mark();
            if let Some(saved) = &restore {
                self.emit_restore_stack_obj(saved.as_deref());
            }
            edge(self, false)?;
            self.em.dedent();
            if self.em.mark() == arm_at {
                self.em.truncate(else_at);
            }
            self.line("}".to_owned());
            return Ok(());
        }
        if let Some(saved) = &restore {
            self.emit_restore_stack_obj(saved.as_deref());
        }
        edge(self, false)?;
        Ok(())
    }

    pub(super) fn emit_return_bool_expr(&mut self, expr: &V2Expr) -> Result<()> {
        if let Some(b) = self.try_eval_const_bool(expr) {
            self.line(if b {
                "return true;".to_owned()
            } else {
                "return false;".to_owned()
            });
            return Ok(());
        }
        match expr {
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => {
                // `!` swaps the test's exits, so the false exit still ends on `lhs`'s own result.
                if let Some(pos_lhs) = self.try_lower_simple_air_cond(lhs)? {
                    self.line(format!("if {} {{", swap_exits(&pos_lhs)));
                    self.em.indent();
                    self.line("return false;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                } else {
                    let tmp = self.fresh("and");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(lhs, &tmp)?;
                    self.line(format!("if {tmp} == 0 {{"));
                    self.em.indent();
                    self.line("return false;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                }
            }
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => {
                if let Some(pos_lhs) = self.try_lower_simple_air_cond(lhs)? {
                    self.line(format!("if {pos_lhs} {{"));
                    self.em.indent();
                    self.line("return true;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                } else {
                    let tmp = self.fresh("or");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(lhs, &tmp)?;
                    self.line(format!("if {tmp} != 0 {{"));
                    self.em.indent();
                    self.line("return true;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                    self.emit_return_bool_expr(rhs)
                }
            }
            _ => {
                if let Some((negated, receiver, args, rspan)) = run_functional_test(expr) {
                    let PreparedFunctional {
                        prim,
                        saved_stack_obj,
                        ..
                    } = self.prepare_run_functional(receiver, args, rspan)?;
                    let bang = if negated { "!" } else { "" };
                    self.line(format!("if {bang}{prim} {{"));
                    self.em.indent();
                    self.emit_restore_stack_obj(saved_stack_obj.as_deref());
                    self.line("return true;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                    self.emit_restore_stack_obj(saved_stack_obj.as_deref());
                    self.line("return false;".to_owned());
                    return Ok(());
                }
                // `return a == b;` tests `a == b` itself and returns on its exits, so the last
                // result at each return is the test's own (a flipped `a != b` would invert it).
                if let Some(pos) = self.try_lower_simple_air_cond(expr)? {
                    self.line(format!("if {pos} {{"));
                    self.em.indent();
                    self.line("return true;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                    self.line("return false;".to_owned());
                    Ok(())
                } else {
                    let tmp = self.fresh("ret");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(expr, &tmp)?;
                    self.line(format!("if {tmp} != 0 {{"));
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
