//! Calls to user callables (trees, functions with an ABI): resolution, argument preparation,
//! statement/value calls and value returns.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// Whether a user `fn`, `#[bhav] fn`, or `extern` answers to `callee` (it then wins over a builtin
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
                self.env.check_extern_library(decl, span)?;
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

    /// Evaluates call arguments, writes `#[temp(N)]` / `call(temps)` operands, and returns the
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
                if !self.inherit_tree && !matches!(self.tree_ctx, TreeContext::SimOnObject { .. }) {
                    return Err(Error::at(
                        span,
                        format!(
                            "{} runs in the Sim-on-object context and can only be called from interactions or other Sim trees",
                            sig.display
                        ),
                    ));
                }
                // A functional's runner is whoever runs it: `run_functional` and Find Best Object
                // run its action and guard on whichever object asks, so its `sim` binding may be
                // an object, and a Sim tree would run with the object as its Sim. (The same rule
                // refuses reaching a Sim tree through a `context = inherit` tree.)
                if self.object_reachable && !self.inherit_tree {
                    return Err(Error::at(
                        span,
                        format!(
                            "{} runs in the Sim-on-object context, and a functional's action or guard may run on an object ('run_functional' and Find Best Object run it on whichever object asks), so it cannot call a Sim tree",
                            sig.display
                        ),
                    ));
                }
                // In an inherit tree the Sim is the runner, `me`, in both passes.
                let first_is_sim = args.first().is_some_and(|a| {
                    a.name.is_none()
                        && matches!(&a.value, V2Expr::Ident(_, n) if if self.inherit_tree {
                            self.is_frame_me(n)
                        } else {
                            self.is_sim_ident(n)
                        })
                });
                if !first_is_sim {
                    return Err(Error::at(
                        span,
                        if self.inherit_tree {
                            format!(
                                "{} expects 'me' (the inherit tree's runner) as its first argument",
                                sig.display
                            )
                        } else {
                            format!(
                                "{} expects the Sim handle as its first argument",
                                sig.display
                            )
                        },
                    ));
                }
                // Discarded code emits no call.
                if self.checking_discarded == 0 {
                    let mut facts = self.env.inherit_calls.borrow_mut();
                    if self.inherit_tree {
                        if let Some(caller) = &self.tree_display {
                            facts.calls_sim.entry(caller.clone()).or_insert(span);
                        }
                    } else {
                        // A Sim tree reaching a Sim-only inherit tree passes the restriction on.
                        facts.edges.push((
                            self.tree_display.clone(),
                            self.object_reachable,
                            sig.display.clone(),
                            false,
                            span,
                        ));
                    }
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
            CallContext::Inherit if self.checking_discarded == 0 => {
                let named =
                    self.inherit_tree || matches!(self.tree_ctx, TreeContext::SimOnObject { .. });
                let caller = named.then(|| self.tree_display.clone()).flatten();
                let from_object = self.object_reachable
                    || (!self.inherit_tree && matches!(self.tree_ctx, TreeContext::ObjectSelf));
                self.env.inherit_calls.borrow_mut().edges.push((
                    caller,
                    from_object,
                    sig.display.clone(),
                    true,
                    span,
                ));
            }
            CallContext::Inherit | CallContext::ObjectSelf | CallContext::Extern => {}
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
        // operand is snapshotted into a fresh local when it reads a temp (about to be
        // overwritten), a later-evaluated argument may run code (calls, foreign reads) that could
        // change it, or it may fault. That is the copying form. A call whose every argument is a
        // bare `temp[k]` or a literal stages in place instead ([`Self::emit_temp_moves`]): no
        // operand can fault or run code, so each temp is read where it is stored.
        let in_place = !self.copy_staging() && values.iter().all(|v| self.stages_in_place(v));
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
            let read_at_store = in_place && bare_temp(&air).is_some();
            operands[i] = if const_val.is_none()
                && ((reads_temps(&air) && !read_at_store) || later_impure || may_fault)
            {
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
        let mut moves: Vec<(usize, String)> = Vec::with_capacity(n);
        for (i, p) in sig.params.iter().enumerate() {
            let dst = match p.slot {
                AbiParamSlot::Temp(t) => usize::from(t),
                AbiParamSlot::Positional if literals.is_some() => continue,
                AbiParamSlot::Positional => positional
                    .iter()
                    .position(|&x| x == i)
                    .expect("positional index"),
            };
            moves.push((dst, operands[i].0.clone()));
        }
        self.emit_temp_moves(moves, in_place);
        Ok(match literals {
            // An argless call is spelled `(temps)` (`[0xFF; 8]`), the authoring tools' encoding:
            // the callee sees the caller's temps as its arguments (the temps pass-through ABI).
            Some(l) if l.is_empty() => format!("call {}(temps)", sig.id),
            Some(l) => {
                // A callee frame wider than the call's four operand words reads the rest from
                // the instructions after the call, so the tree's layout must stay as lowered.
                match sig.frame_args {
                    Some(n) if n <= 4 => {
                        // Listed only when no other declaration of the id disagrees.
                        if self.env.frame_is_narrow(sig.id) {
                            self.narrow_callees.insert(sig.id);
                        } else if self.checking_discarded == 0 && self.wide_call.is_none() {
                            let why = if sig.id >= 8192 && !self.env.library_declares(sig.id) {
                                "a semi-global tree, whose library (and frame) depends on the \
                                 object that runs the call (its 'semi_global: \"Name\";' \
                                 link), and no 'extern \"Name\" mod' of that library declares \
                                 it"
                            } else {
                                "whose tree id another declaration gives a wider frame"
                            };
                            self.wide_call = Some((span, format!("to {}, {why}", sig.display)));
                        }
                    }
                    frame => {
                        self.keep_layout = true;
                        if self.checking_discarded == 0 && self.wide_call.is_none() {
                            let words =
                                frame.map_or("an unknown number of".to_owned(), |n| n.to_string());
                            self.wide_call = Some((
                                span,
                                format!("to {}, whose frame holds {words} arguments", sig.display),
                            ));
                        }
                    }
                }
                let parts: Vec<String> = l.iter().map(i16::to_string).collect();
                format!("call {}({})", sig.id, parts.join(", "))
            }
            None => format!("call {}(temps)", sig.id),
        })
    }

    /// Whether call arguments are staged in the copying form ([`Self::emit_temp_moves`]): in an
    /// object whose trees may read global 34 (each store is an Expression, which sets the
    /// scratch pointer that read depends on, and a call carries the pointer into the callee),
    /// in an object with a tree that keeps its layout (`ObjectEnv::copy_staging`: numbered raw
    /// branch targets count nodes, or where `finish` finds frame words observable), inside a
    /// `#[temp]` loop, whose counter check reads the stores, and in a tree with an explicit
    /// `kind` (an indexed tree is entered at a numbered node).
    pub(super) fn copy_staging(&self) -> bool {
        self.env.copy_staging || self.env.reads_global_34 || self.temp_loops > 0 || self.fixed_entry
    }

    /// Records an option binding's copy of its temp for [`crate::v2::mir::alias_option_temps`],
    /// which drops the copy and so, like an in-place staging, changes the tree's nodes and
    /// frame words: only where those are not observable ([`Self::copy_staging`]). `finish`
    /// treats an applied alias as a staging, so the object is lowered again without it where
    /// the result turns out observable. Nor in an object whose text may name frame words
    /// (`$Local:N`): a tree that formats it may read a word this tree's frame left behind.
    pub(super) fn record_option_alias(&mut self, air_name: &str) {
        // A discarded check's trial lowering leaves no code, and its names may come again.
        if self.checking_discarded == 0
            && !self.copy_staging()
            && !mir::reads_a_word(&self.env.text_reads)
        {
            self.option_aliases.insert(air_name.to_owned());
        }
    }

    /// Whether `arg` can be staged in place: a literal, or a bare `temp[k]` (`k < 8`). Neither
    /// can fault or run code, and the second is read where it is stored.
    fn stages_in_place(&self, arg: &V2Expr) -> bool {
        match arg {
            V2Expr::Int(..) | V2Expr::Bool(..) => true,
            V2Expr::Unary {
                op: V2UnaryOp::Neg,
                inner,
                ..
            } => matches!(inner.as_ref(), V2Expr::Int(..)),
            V2Expr::Index { base, index, .. } => {
                matches!(base.as_ref(), V2Expr::Ident(_, b) if b == "temp" && self.lookup_var(b).is_none())
                    && matches!(index.as_ref(), V2Expr::Int(_, k) if (0..8).contains(k))
            }
            _ => false,
        }
    }

    /// Stores call operands into their temps, each `(dst, src)` as `temp[dst] := src`, in
    /// parameter order.
    ///
    /// The copying form (`in_place` false) emits every store as written; its operands were
    /// snapshotted already. In place, every `src` is a literal or a bare `temp[k]`, which reads
    /// the value it had before any of the stores: before a store overwrites a temp a later store
    /// still reads (`f(temp[1], temp[0])`), that temp is saved into a fresh local, which the
    /// later store reads instead. A store of a temp into itself is dropped, except the last
    /// store: it is the staging's final Expression, whose result and scratch pointer the callee
    /// sees, so it stays exactly as the copying form leaves it. No operand can fault, so no
    /// other store is observable once the next one runs. The destinations are distinct (the
    /// signature check refuses a `#[temp(N)]` inside the positional window).
    pub(super) fn emit_temp_moves(&mut self, mut moves: Vec<(usize, String)>, in_place: bool) {
        if !in_place {
            for (dst, src) in moves {
                self.line(format!("temp[{dst}] := {src};"));
            }
            return;
        }
        self.staged_in_place = true;
        let last = moves.len().saturating_sub(1);
        for k in 0..moves.len() {
            let (dst, src) = moves[k].clone();
            if k < last && bare_temp(&src) == Some(dst) {
                continue;
            }
            if moves[k + 1..]
                .iter()
                .any(|(_, later)| bare_temp(later) == Some(dst))
            {
                let saved = self.fresh("arg");
                self.emit_let(&saved, &format!("temp[{dst}]"));
                for (_, later) in &mut moves[k + 1..] {
                    if bare_temp(later) == Some(dst) {
                        later.clone_from(&saved);
                    }
                }
            }
            self.line(format!("temp[{dst}] := {src};"));
        }
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
            (
                ResolvedReturnAbi::ResultValue { ty, temp }
                | ResolvedReturnAbi::OptionValue { ty, temp },
                true,
            ) => {
                let (ty, temp) = (ty.clone(), *temp);
                let call = self.prepare_callable_call(sig, args, span)?;
                self.emit_try_fail_branch(&call, span)?;
                Ok((self.capture_temp(temp, raw), ty))
            }
            (ResolvedReturnAbi::OptionValue { .. }, false) => Err(Error::at(
                span,
                format!(
                    "{} returns an Option: bind its value with 'let Some(v) = .. else {{ .. }};', unwrap it with '?', or test it with 'if'",
                    sig.display
                ),
            )),
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
                    "'?' in value position needs a call returning 'Result<T>' or 'Option<T>' with '#[ret(temp = N)]', but {} does not",
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

    /// If `expr` is a value-returning callable (`f(..)` with `#[ret(temp = N)] .. -> T`, or `f(..)?` with
    /// `#[ret(temp = N)] .. -> Result<T>` / `-> Option<T>`), lowers the call and returns the raw `temp[N]` operand.
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
                | (
                    ResolvedReturnAbi::ResultValue { .. } | ResolvedReturnAbi::OptionValue { .. },
                    true
                )
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
            && !self.is_my_ident(name)
            && self
                .env
                .is_bcon_table_name(&self.owner_module, name, self.in_object_scope)
    }

    /// Resolves `Table.entry` (or the path form `Table::entry`) to its `bcon[addr]` operand (and
    /// compile-time value for private tables). `None` if `expr` is not a BCON table entry reference.
    /// An enum or bitflags type of the same name wins the path form.
    pub(super) fn bcon_field_ref(&self, expr: &V2Expr) -> Option<Result<(String, Option<i16>)>> {
        let (span, bname, field) = match expr {
            V2Expr::Field { span, base, field } => {
                let V2Expr::Ident(_, bname) = base.as_ref() else {
                    return None;
                };
                (span, bname, field)
            }
            V2Expr::Path(span, segs) if segs.len() == 2 => {
                let bname = &segs[0];
                if self
                    .env
                    .lookup_enum(&self.owner_module, bname, self.in_object_scope)
                    .is_some()
                    || self
                        .env
                        .lookup_bitflags(&self.owner_module, bname, self.in_object_scope)
                        .is_some()
                {
                    return None;
                }
                (span, bname, &segs[1])
            }
            _ => return None,
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
