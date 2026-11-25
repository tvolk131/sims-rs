//! Builtin calls: argument binding, primitive formatting (`PRIM_BUILTINS`), animation, dynamic
//! sprites, type tests and `sleep`.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// Compile-time integer argument for a primitive builtin (literal, enum, `const`, constant
    /// helper binding, or a BCON entry with a known value), checked against `lo..=hi`.
    pub(super) fn eval_builtin_int(
        &self,
        expr: &V2Expr,
        what: &str,
        lo: i64,
        hi: i64,
    ) -> Result<i64> {
        let v: i64 = match expr {
            V2Expr::Int(_, v) => *v,
            V2Expr::Ident(_, name)
                if self.lookup_var(name).is_none() && self.const_item(name).is_some() =>
            {
                // A `const` that does not fit in i16 is a GUID, not a number.
                let v = self.const_item(name).unwrap_or_default();
                if i16::try_from(v).is_err() {
                    return Err(Error::at(
                        expr.span(),
                        format!(
                            "const '{name}' ({v:#x}) does not fit in i16; use it where a GUID is expected"
                        ),
                    ));
                }
                v
            }
            _ => match self.literal_compare_operand(expr) {
                Some((_, v)) => i64::from(v),
                None => i64::from(self.eval_const_i16(expr).map_err(|_| {
                    Error::at(
                        expr.span(),
                        format!("{what} must be a compile-time constant"),
                    )
                })?),
            },
        };
        if v < lo || v > hi {
            return Err(Error::at(
                expr.span(),
                format!("{what} {v} out of range {lo}..={hi}"),
            ));
        }
        Ok(v)
    }

    /// `.air` local name of a v2 `i16` local variable used as a primitive's local slot.
    /// The local `push_interaction(icon:)` reads: an `i16` local, or an object handle. The handle
    /// given as `object:` too is the pinned slot itself; another handle is a local (`self` in an
    /// interaction is the entry save, or a copy of `stack_obj` when the tree never changes it).
    pub(super) fn push_icon_local(
        &mut self,
        icon: &V2Expr,
        object: Option<&CallArg>,
        pinned_object: bool,
    ) -> Result<String> {
        if let V2Expr::Ident(_, name) = icon {
            if pinned_object
                && matches!(object.map(|o| &o.value), Some(V2Expr::Ident(_, o)) if o == name)
            {
                return Ok(PIN_SLOT.to_owned());
            }
            if let Some(Binding::Local { air_name, ty }) = self.lookup_var(name)
                && ty.is_object_handle()
            {
                return Ok(air_name.clone());
            }
            if self.is_self_ident(name) {
                if self.has_self_obj_local {
                    return Ok("__self_obj".to_owned());
                }
                // The object is the stack object on entry to a Sim tree and `my` in its own.
                let self_air = match self.tree_ctx {
                    TreeContext::SimOnObject { .. } => "stack_obj",
                    TreeContext::ObjectSelf => "my.id",
                };
                let tmp = self.fresh("icon");
                self.emit_let(&tmp, self_air);
                return Ok(tmp);
            }
        }
        self.builtin_local_arg(icon, "push_interaction(icon:)")
            .map_err(|_| {
                Error::at(
                    icon.span(),
                    "push_interaction(icon:) must name an i16 local, an object-handle local, 'self', or the 'object:' handle",
                )
            })
    }

    /// Relationship's `value:` operand: a value the VM reads (`writes`), or the direct `i16`
    /// cell it stores the number into. A value that needs lowering lines is computed into a
    /// temporary before the primitive.
    pub(super) fn relationship_value_arg(&mut self, expr: &V2Expr, writes: bool) -> Result<String> {
        if writes {
            let (air, ty) = self.lower_value_expr(expr)?;
            if ty != ValueType::I16 {
                return Err(Error::at(
                    expr.span(),
                    format!(
                        "relationship(value:) must be an i16, found '{}'",
                        self.env.display_value_type(&ty)
                    ),
                ));
            }
            return Ok(air);
        }
        self.reject_read_only_sim_slot(expr)?;
        let lval = match self.resolve_lvalue(expr) {
            Ok(l) => l,
            Err(_) if self.eval_const_i16(expr).is_ok() => {
                return Err(Error::at(
                    expr.span(),
                    "relationship(value:) without flag 4 stores the number into a cell, so a constant is not allowed (flag 4 writes the value instead)",
                ));
            }
            Err(e) => return Err(e),
        };
        match lval {
            LValue::Direct {
                air,
                ty: ValueType::I16,
            } => Ok(air),
            LValue::Direct { ty, .. } => Err(Error::at(
                expr.span(),
                format!(
                    "relationship(value:) must be an i16 cell, found '{}'",
                    self.env.display_value_type(&ty)
                ),
            )),
            _ => Err(Error::at(
                expr.span(),
                "relationship(value:) must be a local, temp, attribute, or other direct i16 cell of this object or the stack object",
            )),
        }
    }

    pub(super) fn builtin_local_arg(&self, expr: &V2Expr, what: &str) -> Result<String> {
        self.builtin_local_arg_typed(expr, what, true)
    }

    /// [`Self::builtin_local_arg`]; an `enum` local is accepted only with `allow_enum` (a slot
    /// the VM writes arbitrary codes into must be a plain `i16`).
    pub(super) fn builtin_local_arg_typed(
        &self,
        expr: &V2Expr,
        what: &str,
        allow_enum: bool,
    ) -> Result<String> {
        if let V2Expr::Ident(_, name) = expr
            && let Some(Binding::Local { air_name, ty }) = self.lookup_var(name)
            && (*ty == ValueType::I16 || (allow_enum && matches!(ty, ValueType::Enum(_))))
        {
            return Ok(air_name.clone());
        }
        Err(Error::at(
            expr.span(),
            format!("{what} must name an i16 local variable (declare one with 'let x: i16;')"),
        ))
    }

    /// Binds a builtin's arguments to its parameter names. Positional arguments fill
    /// `positional` in order; a named argument must name one of `positional` or `named_only`.
    /// Unknown names, a parameter given twice, too many positional arguments, and a
    /// positional argument after a named one are errors.
    pub(super) fn bind_builtin_args<'e>(
        &self,
        prim: &str,
        args: &'e [CallArg],
        positional: &[&str],
        named_only: &[&str],
    ) -> Result<BTreeMap<String, &'e CallArg>> {
        let mut out = BTreeMap::new();
        let mut next = 0usize;
        let mut saw_named = false;
        for a in args {
            let key = match &a.name {
                Some(key) => {
                    saw_named = true;
                    if !positional.contains(&key.as_str()) && !named_only.contains(&key.as_str()) {
                        return Err(Error::at(
                            a.span,
                            format!(
                                "unknown {prim}() argument '{key}' (expected one of: {})",
                                positional
                                    .iter()
                                    .chain(named_only)
                                    .copied()
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        ));
                    }
                    key.clone()
                }
                None if saw_named => {
                    return Err(Error::at(
                        a.span,
                        format!("positional argument cannot follow a named argument in {prim}()"),
                    ));
                }
                None => {
                    let Some(&key) = positional.get(next) else {
                        return Err(Error::at(
                            a.span,
                            format!(
                                "too many positional arguments to {prim}() (it takes at most {})",
                                positional.len()
                            ),
                        ));
                    };
                    next += 1;
                    key.to_owned()
                }
            };
            if out.insert(key.clone(), a).is_some() {
                return Err(Error::at(
                    a.span,
                    format!("duplicate {prim}() argument '{key}'"),
                ));
            }
        }
        Ok(out)
    }

    /// Collects `name: value` arguments, rejecting positional, unknown, and repeated keys.
    pub(super) fn builtin_named_args<'e>(
        &self,
        prim: &str,
        args: &'e [CallArg],
        allowed: &[&str],
    ) -> Result<BTreeMap<String, &'e CallArg>> {
        let mut out = BTreeMap::new();
        for a in args {
            let Some(key) = &a.name else {
                return Err(Error::at(
                    a.span,
                    format!(
                        "{prim}() only takes named arguments ({})",
                        allowed
                            .iter()
                            .map(|k| format!("'{k}:'"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            };
            if !allowed.contains(&key.as_str()) {
                return Err(Error::at(
                    a.span,
                    format!(
                        "unknown {prim}() argument '{key}' (expected one of: {})",
                        allowed.join(", ")
                    ),
                ));
            }
            if out.insert(key.clone(), a).is_some() {
                return Err(Error::at(
                    a.span,
                    format!("duplicate {prim}() argument '{key}'"),
                ));
            }
        }
        Ok(out)
    }

    /// Formats `animate(clip:, table:, flags:, events:, local:)` and returns the `.air` primitive
    /// text plus the event local's `.air` name (`local:` sets flag `0x20`).
    pub(super) fn format_animate(&mut self, args: &[CallArg]) -> Result<(String, Option<String>)> {
        let named = self.builtin_named_args(
            "animate",
            args,
            &["clip", "table", "flags", "events", "local", "event_local"],
        )?;
        let int = |key: &str, lo: i64, hi: i64| -> Result<i64> {
            match named.get(key) {
                Some(a) => self.eval_builtin_int(&a.value, &format!("animate {key}"), lo, hi),
                None => Ok(0),
            }
        };
        let clip = int("clip", 0, 65535)?;
        let table = int("table", 0, 255)?;
        let mut flags = int("flags", 0, 255)?;
        let events = int("events", -128, 127)?;
        // `event_local: N`: events go to slot N outside the frame, unbound. `.air`'s
        // `animate(local: N)` would grow the frame to N + 1, so this is the primitive's raw form.
        if let Some(a) = named.get("event_local") {
            if let Some(l) = named.get("local") {
                return Err(Error::at(
                    l.span,
                    "animate takes 'local:' or 'event_local:', not both",
                ));
            }
            let slot = self.eval_builtin_int(&a.value, "animate event_local", 0, 255)?;
            let Some(locals) = self.frame_locals else {
                return Err(Error::at(
                    a.span,
                    "animate(event_local:) needs the tree's frame size: add @frame(locals: K) with K <= the slot",
                ));
            };
            if slot < i64::from(locals) {
                return Err(Error::at(
                    a.span,
                    format!(
                        "animate(event_local: {slot}) is inside the frame (@frame(locals: {locals})); use 'local:' with a variable instead"
                    ),
                ));
            }
            let slot = slot as u8;
            self.event_local.get_or_insert((a.span, slot));
            flags |= 0x20;
            let [c0, c1] = (clip as u16).to_le_bytes();
            let text = format!(
                "op 0x002c ({c0:#04x}, {c1:#04x}, {slot:#04x}, 0x00, {table:#04x}, {flags:#04x}, {:#04x}, 0x00)",
                events as u8
            );
            return Ok((text, None));
        }
        let local = match named.get("local") {
            Some(a) => {
                flags |= 0x20;
                Some(self.builtin_local_arg_typed(&a.value, "animate(local:)", false)?)
            }
            None if flags & 0x20 != 0 => {
                let a = named.get("flags").expect("flags were given");
                return Err(Error::at(
                    a.span,
                    "animate flags 0x20 store events in a local; name it with 'local:' instead",
                ));
            }
            None => None,
        };
        let mut text =
            format!("animate(clip: {clip}, table: {table}, flags: {flags:#04x}, events: {events}");
        if let Some(l) = &local {
            text.push_str(&format!(", local: {l}"));
        }
        text.push(')');
        Ok((text, local))
    }

    /// `animate(...)` as a statement (runs to completion) or `animate(...) |event| { ... }`
    /// (runs the body once per event). Both lower to `while !animate(...) { ... }`.
    pub(super) fn lower_animate_stmt(
        &mut self,
        span: Span,
        args: &[CallArg],
        event: Option<&str>,
        body: &[V2Stmt],
    ) -> Result<()> {
        if let Some(name) = event
            && self.stmts_write_ident(body, name)?
        {
            return Err(Error::at(
                span,
                format!("the animation event '{name}' is read-only"),
            ));
        }
        if let Some(a) = args
            .iter()
            .find(|a| a.name.as_deref() == Some("event_local"))
            && (event.is_some() || !body.is_empty())
        {
            return Err(Error::at(
                a.span,
                "animate(event_local:) leaves events unbound: it takes no '|event|' body",
            ));
        }
        let (prim, local) = self.format_animate(args)?;
        self.line(format!("while !{prim} {{"));
        self.em.indent();
        self.push_scope(ScopeFrame {
            cleanups: Vec::new(),
            is_loop_body: true,
            is_fn_root: false,
            is_air_while: true,
            owner_defer_id: self.current_defer_id,
            cancel_flag: None,
            restore_stack_on_break: false,
            continue_step: None,
        });
        if let Some(name) = event {
            let binding = match local {
                Some(air_name) => Binding::Local {
                    air_name,
                    ty: ValueType::I16,
                },
                None => Binding::BhavParam {
                    index: 0,
                    ty: ValueType::I16,
                },
            };
            self.bind_var(name.to_owned(), binding);
        }
        let res = self.lower_stmts(body);
        let pop_res = self.pop_scope_and_run_cleanups();
        self.em.dedent();
        self.line("}".to_owned());
        res.and(pop_res)?;
        self.emit_inline_fn_return_propagation(false)?;
        self.emit_cancel_propagation(false)?;
        Ok(())
    }

    /// `set_dynamic_sprite(index: i, value: v)`: `self.sprite_flags[i] = v`.
    pub(super) fn lower_set_dynamic_sprite(&mut self, args: &[CallArg], span: Span) -> Result<()> {
        let mut index = None;
        let mut value = None;
        for a in args {
            match a.name.as_deref() {
                Some("index") if index.is_none() => index = Some(&a.value),
                Some("value") if value.is_none() => value = Some(&a.value),
                _ => {
                    return Err(Error::at(
                        a.span,
                        "set_dynamic_sprite takes exactly 'index:' and 'value:'",
                    ));
                }
            }
        }
        let (Some(index), Some(value)) = (index, value) else {
            return Err(Error::at(
                span,
                "set_dynamic_sprite takes exactly 'index:' and 'value:'",
            ));
        };
        let target = V2Expr::Index {
            span,
            base: Box::new(V2Expr::Field {
                span,
                base: Box::new(V2Expr::Ident(span, "self".to_owned())),
                field: "sprite_flags".to_owned(),
            }),
            index: Box::new(index.clone()),
        };
        let (rhs, rty) = self.lower_value_expr(value)?;
        if !matches!(rty, ValueType::I16 | ValueType::Bool) {
            return Err(Error::at(
                value.span(),
                format!(
                    "set_dynamic_sprite value must be i16 or bool (found '{}')",
                    self.env.display_value_type(&rty)
                ),
            ));
        }
        let lval = self.resolve_lvalue(&target)?;
        self.emit_lvalue_assign(&lval, V2AssignOp::Set, &rhs)
    }

    /// Formats a primitive builtin (`PRIM_BUILTINS`, `OP_BUILTINS`, `animate`, `breakpoint`,
    /// `random_into`) as `.air` primitive text usable as a statement or a condition. Returns
    /// `None` for other names. Callers resolve user callables and helpers first.
    pub(super) fn format_prim_builtin(
        &mut self,
        fname: &str,
        args: &[CallArg],
        span: Span,
    ) -> Result<Option<String>> {
        if let Some(&(_, opcode)) = OP_BUILTINS.iter().find(|(n, _)| *n == fname) {
            if !args.is_empty() {
                return Err(Error::at(span, format!("{fname}() takes no arguments")));
            }
            // The VM ignores Notify Stack Object out of Idle's operand; the original fills it
            // with 0xFF, and a faulting primitive's operand bytes are observable.
            if opcode == 0x0031 {
                return Ok(Some("notify_idle() pad 0xff".to_owned()));
            }
            return Ok(Some(format!("op {opcode:#06x} (0, 0, 0, 0, 0, 0, 0, 0)")));
        }
        match fname {
            "animate" => return Ok(Some(self.format_animate(args)?.0)),
            "set_to_next" => return Ok(Some(self.format_set_to_next(args, span)?)),
            // `inc_lt(&mut x, n)` / `dec_gt(&mut x, n)`: the VM's increment-and-compare
            // operators (`x = x ± 1` wrapping at i16, then compare with `n` as read before the
            // store). A condition; as a statement it is an empty `if`.
            "inc_lt" | "dec_gt" => {
                let [place, bound] = args else {
                    return Err(Error::at(
                        span,
                        format!("{fname}(&mut x, n) expects 2 positional arguments"),
                    ));
                };
                if place.name.is_some() || bound.name.is_some() {
                    return Err(Error::at(
                        span,
                        format!("{fname}(&mut x, n) expects 2 positional arguments"),
                    ));
                }
                let V2Expr::MutRef { inner, .. } = &place.value else {
                    return Err(Error::at(
                        place.span,
                        format!("{fname}() steps its first argument in place; pass it as '&mut x'"),
                    ));
                };
                self.reject_read_only_sim_slot(inner)?;
                let lval = self.resolve_lvalue(inner)?;
                let LValue::Direct { air, ty } = lval else {
                    return Err(Error::at(
                        place.span,
                        format!(
                            "{fname}(&mut x, n): 'x' must be a local, field, or other direct i16 slot of this object"
                        ),
                    ));
                };
                if ty != ValueType::I16 {
                    return Err(Error::at(
                        place.span,
                        format!(
                            "{fname}(&mut x, n): 'x' must be an i16, found '{}'",
                            self.env.display_value_type(&ty)
                        ),
                    ));
                }
                let (n, n_ty) = self.lower_value_expr(&bound.value)?;
                if n_ty != ValueType::I16 {
                    return Err(Error::at(
                        bound.span,
                        format!("{fname}(&mut x, n): 'n' must be an i16"),
                    ));
                }
                return Ok(Some(format!("{air} {fname} {n}")));
            }
            "breakpoint" => {
                return match args {
                    [] => Ok(Some("breakpoint()".to_owned())),
                    [a] if a.name.is_none() => {
                        let (v, _) = self.lower_value_expr(&a.value)?;
                        Ok(Some(format!("breakpoint({v})")))
                    }
                    _ => Err(Error::at(
                        span,
                        "breakpoint() takes at most one positional argument",
                    )),
                };
            }
            "random_into" => {
                let [dst, range] = args else {
                    return Err(Error::at(
                        span,
                        "random_into(x, n) expects 2 positional arguments",
                    ));
                };
                if dst.name.is_some() || range.name.is_some() {
                    return Err(Error::at(
                        span,
                        "random_into(x, n) expects 2 positional arguments",
                    ));
                }
                let (mut range_air, range_ty) = self.lower_value_expr(&range.value)?;
                if range_ty != ValueType::I16 {
                    return Err(Error::at(
                        range.span,
                        "random_into(x, n): 'n' must be an i16",
                    ));
                }
                // The VM's Random Number writes the draw into a global range operand (the
                // original's pointer aliasing on global 34), so a global range is copied first.
                if range_air.starts_with("global[") {
                    let tmp = self.fresh("rnd_range");
                    self.emit_let(&tmp, &range_air);
                    range_air = tmp;
                }
                self.reject_read_only_sim_slot(&dst.value)?;
                let lval = self.resolve_lvalue(&dst.value)?;
                let LValue::Direct { air, ty } = lval else {
                    return Err(Error::at(
                        dst.span,
                        "random_into(x, n): 'x' must be a local, attribute, or other direct i16 slot of this object",
                    ));
                };
                if ty != ValueType::I16 {
                    return Err(Error::at(
                        dst.span,
                        format!(
                            "random_into(x, n): 'x' must be an i16, found '{}'",
                            self.env.display_value_type(&ty)
                        ),
                    ));
                }
                return Ok(Some(format!("random({air}, {range_air})")));
            }
            _ => {}
        }
        if fname == "generic_call" {
            // Generic Sims Call: the routine number is the whole operand.
            let bound = self.bind_builtin_args(fname, args, &["routine"], &[])?;
            let Some(r) = bound.get("routine") else {
                return Err(Error::at(
                    span,
                    "generic_call(routine) requires a routine number",
                ));
            };
            let routine = self.eval_builtin_int(&r.value, "generic_call routine", 0, 65535)?;
            return Ok(Some(format!("generic_call({routine})")));
        }
        let Some(spec) = PRIM_BUILTINS.iter().find(|b| b.v2 == fname) else {
            return Ok(None);
        };
        let allowed: Vec<&str> = spec.keys.iter().map(|(k, _, _)| *k).collect();
        let named = self.builtin_named_args(fname, args, &allowed)?;
        let required: &[&str] = match fname {
            "relationship" => &["value"],
            _ => &[],
        };
        if let Some(missing) = required.iter().find(|k| !named.contains_key(**k)) {
            return Err(Error::at(span, format!("{fname}() requires '{missing}:'")));
        }
        // `push_interaction(object: <@slot(N) handle>)`: the handle already lives in frame word
        // N, which `@slot` fixes, so the VM reads it from there (flag 0x02) with no copy.
        let slot_object = match named.get("object") {
            Some(CallArg {
                value: V2Expr::Ident(_, name),
                ..
            }) if fname == "push_interaction" => match self.lookup_var(name) {
                Some(Binding::Local { air_name, ty }) if ty.is_object_handle() => {
                    slot_word(air_name)
                }
                _ => None,
            },
            _ => None,
        };
        if slot_object.is_some()
            && let Some(f) = named.get("flags")
            && self.eval_builtin_int(&f.value, "push_interaction flags", 0, 255)? & 0x02 != 0
        {
            return Err(Error::at(
                f.span,
                "push_interaction flags 0x02 is implied by an object handle; leave it out",
            ));
        }
        // `push_interaction(object: <handle>)`: the VM reads the object from a local slot
        // (flag 0x02), so the handle is copied into the tree's pinned slot first.
        let pinned_object = match named.get("object") {
            Some(a)
                if fname == "push_interaction"
                    && slot_object.is_none()
                    && self
                        .eval_builtin_int(&a.value, "push_interaction object", 0, 255)
                        .is_err() =>
            {
                if self.checking_discarded == 0 {
                    self.pin_span.get_or_insert(a.span);
                    if self.raw_span.is_some() {
                        return Err(Error::at(a.span, RAW_WITH_PIN));
                    }
                    if self.slot_span.is_some() {
                        return Err(Error::at(a.span, SLOT_WITH_PIN));
                    }
                }
                let (obj, ty) = self.lower_value_expr(&a.value)?;
                if !ty.is_object_handle() {
                    return Err(Error::at(
                        a.span,
                        format!(
                            "push_interaction object: expects an object handle or a constant parameter index, found '{}'",
                            self.env.display_value_type(&ty)
                        ),
                    ));
                }
                if let Some(f) = named.get("flags") {
                    let flags =
                        self.eval_builtin_int(&f.value, "push_interaction flags", 0, 255)?;
                    if flags & 0x02 != 0 {
                        return Err(Error::at(
                            f.span,
                            "push_interaction flags 0x02 is implied by an object handle; leave it out",
                        ));
                    }
                }
                self.line(format!("local[{PIN_SLOT}] := {obj};"));
                true
            }
            _ => false,
        };
        // Flag bits that make the VM read an operand as a local slot index. `icon:` names its
        // local by variable and sets its bit; operand forms v2 cannot spell by name are rejected.
        let (local_bits, implied_bits): (&[(i64, &str)], i64) = match fname {
            "push_interaction" => (
                &[
                    (
                        0x01,
                        "reads the icon from a local; name it with 'icon:' instead",
                    ),
                    (
                        0x02,
                        "reads the object from a local slot, which v2 cannot name; 'object:' is a parameter index",
                    ),
                ],
                (if named.contains_key("icon") { 0x01 } else { 0 })
                    | (if pinned_object || slot_object.is_some() {
                        0x02
                    } else {
                        0
                    }),
            ),
            "special_effect" => (
                &[(
                    0x20,
                    "reads the duration from a local slot, which v2 cannot name",
                )],
                0,
            ),
            _ => (&[], 0),
        };
        if let Some(a) = named.get("flags") {
            let flags = self.eval_builtin_int(&a.value, &format!("{fname} flags"), 0, 65535)?;
            for &(bit, why) in local_bits {
                if flags & bit != 0 && implied_bits & bit == 0 {
                    return Err(Error::at(a.span, format!("{fname} flags {bit:#04x} {why}")));
                }
            }
        }
        let mut parts = Vec::new();
        for &(key, air_key, kind) in spec.keys {
            let Some(a) = named.get(key) else {
                if key == "flags" && implied_bits != 0 {
                    parts.push(format!("{air_key}: {implied_bits}"));
                }
                continue;
            };
            let what = format!("{fname} {key}");
            if key == "object" && pinned_object {
                parts.push(format!("{air_key}: {PIN_SLOT}"));
                continue;
            }
            if key == "object"
                && let Some(n) = slot_object
            {
                parts.push(format!("{air_key}: {n}"));
                continue;
            }
            let text = match kind {
                PrimArg::Int(lo, hi) if key == "flags" && implied_bits != 0 => {
                    (self.eval_builtin_int(&a.value, &what, lo, hi)? | implied_bits).to_string()
                }
                PrimArg::Int(lo, hi) => self.eval_builtin_int(&a.value, &what, lo, hi)?.to_string(),
                PrimArg::Bool => self.eval_const_bool_required(&a.value, a.span)?.to_string(),
                PrimArg::Guid => format!("{:#010x}", self.resolve_guid_expr(&a.value)?),
                PrimArg::Local if fname == "push_interaction" && key == "icon" => {
                    self.push_icon_local(&a.value, named.get("object").copied(), pinned_object)?
                }
                PrimArg::Local => self.builtin_local_arg(&a.value, &format!("{fname}({key}:)"))?,
                PrimArg::RelValue => {
                    let flags = match named.get("flags") {
                        Some(f) => self.eval_builtin_int(&f.value, "relationship flags", 0, 255)?,
                        None => 0,
                    };
                    self.relationship_value_arg(&a.value, flags & 4 != 0)?
                }
                // `refresh` names the running frame's object (`me`) or its stack object. In a
                // Sim-on-object tree `me` is the Sim and the object is the stack object.
                PrimArg::RefreshTarget => match (&a.value, &self.tree_ctx) {
                    (V2Expr::Ident(_, n), TreeContext::ObjectSelf) if self.is_self_ident(n) => {
                        "me".to_owned()
                    }
                    (V2Expr::Ident(_, n), TreeContext::SimOnObject { .. })
                        if self.is_self_ident(n) =>
                    {
                        if self.explicit_stack_obj {
                            return Err(Error::at(
                                a.span,
                                "'refresh(target: self)' cannot temporarily select self in a Sim tree using an explicit stack_obj cursor",
                            ));
                        }
                        "stack_obj".to_owned()
                    }
                    (V2Expr::Ident(_, n), TreeContext::SimOnObject { .. })
                        if self.is_sim_ident(n) =>
                    {
                        "me".to_owned()
                    }
                    (V2Expr::Ident(_, n), _) if n == "stack_obj" => "stack_obj".to_owned(),
                    (other, _) => self.eval_builtin_int(other, &what, 0, 1)?.to_string(),
                },
            };
            parts.push(format!("{air_key}: {text}"));
        }
        Ok(Some(format!("{}({})", spec.air, parts.join(", "))))
    }

    /// `x.is_type(GUID)` on an object handle: `test_object_type(guid, object: x)`.
    pub(super) fn format_is_type(
        &mut self,
        receiver: &V2Expr,
        args: &[CallArg],
        span: Span,
    ) -> Result<String> {
        let [a] = args else {
            return Err(Error::at(
                span,
                "'.is_type(GUID)' expects exactly 1 argument",
            ));
        };
        if a.name.is_some() {
            return Err(Error::at(
                a.span,
                "'.is_type(GUID)' takes a positional GUID",
            ));
        }
        let guid = self.resolve_guid_expr(&a.value)?;
        let (obj, ty) = self.lower_value_expr(receiver)?;
        if !ty.is_object_handle() {
            return Err(Error::at(
                receiver.span(),
                format!(
                    "'.is_type(...)' needs an object handle, found '{}'",
                    self.env.display_value_type(&ty)
                ),
            ));
        }
        Ok(format!("test_object_type({guid:#010x}, object: {obj})"))
    }

    pub(super) fn emit_sleep(&mut self, args: &[CallArg], span: Span) -> Result<()> {
        if args.len() != 1 || args[0].name.is_some() {
            return Err(Error::at(
                span,
                "sleep(ticks) expects 1 positional argument",
            ));
        }
        let (ticks_air, _) = self.lower_value_expr(&args[0].value)?;
        self.sleep_span.get_or_insert(span);
        let cancel = if self.in_defer() {
            None
        } else {
            self.enclosing_cancel_flag()
        };
        if let Some((_, ref flag)) = cancel {
            let target_while = self
                .nearest_air_while_idx()
                .expect("enclosing cancel_flag implies an enclosing air while loop");
            self.line("if my.person.priority == 0 {".to_owned());
            self.em.indent();
            self.emit_store(flag, ":=", "1");
            self.emit_cleanups_down_to(target_while)?;
            self.line("break;".to_owned());
            self.em.dedent();
            self.line("}".to_owned());
        }
        self.line(format!("idle({ticks_air});"));
        if let Some((_, ref flag)) = cancel {
            let target_while = self
                .nearest_air_while_idx()
                .expect("enclosing cancel_flag implies an enclosing air while loop");
            self.line("if my.person.priority == 0 {".to_owned());
            self.em.indent();
            self.emit_store(flag, ":=", "1");
            self.emit_cleanups_down_to(target_while)?;
            self.line("break;".to_owned());
            self.em.dedent();
            self.line("}".to_owned());
        }
        Ok(())
    }
}

/// The frame word an `@slot(N) let` local's `.air` name (`__slot<N>_...`) fixes.
fn slot_word(air_name: &str) -> Option<u8> {
    let rest = air_name.strip_prefix("__slot")?;
    let (n, _) = rest.split_once('_')?;
    n.parse().ok()
}
