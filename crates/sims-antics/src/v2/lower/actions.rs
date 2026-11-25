//! Action builtins with object effects: functionals, sounds, budget, find-best-object, create/next
//! object, slots and motive changes.

use super::*;

impl<'a> TreeLowerer<'a> {
    pub(super) fn prepare_run_functional(
        &mut self,
        receiver: &V2Expr,
        args: &[CallArg],
        span: Span,
    ) -> Result<PreparedFunctional> {
        if args.is_empty() {
            return Err(Error::at(
                span,
                "run_functional(kind [, args...]) requires a functional kind",
            ));
        }
        // A kind name (`prep`, `cook`, ...) or a raw function-table index (`run_functional(4)`),
        // which is always untyped.
        let (kspan, kind_name, numeric) = match &args[0].value {
            V2Expr::Ident(kspan, kind_name) if args[0].name.is_none() => {
                let _ = functional_kind_info(kind_name, *kspan)?;
                (*kspan, kind_name.clone(), false)
            }
            V2Expr::Int(kspan, v) if args[0].name.is_none() => {
                if !(0..=255).contains(v) {
                    return Err(Error::at(
                        *kspan,
                        format!("run_functional() function index {v} is outside 0..=255"),
                    ));
                }
                (*kspan, v.to_string(), true)
            }
            _ => {
                return Err(Error::at(
                    args[0].span,
                    "first argument to run_functional() must be a functional kind identifier (e.g. prep, cook) or a function index (0..=255)",
                ));
            }
        };
        let kind_name = &kind_name;
        let kspan = &kspan;
        let (target_air, target_ty) = self.lower_value_expr(receiver)?;
        if !matches!(
            target_ty,
            ValueType::ObjectId | ValueType::ObjectRef(_) | ValueType::I16
        ) {
            return Err(Error::at(
                receiver.span(),
                format!(
                    "run_functional() receiver must be an ObjectId or typed object reference, found '{}'",
                    self.env.display_value_type(&target_ty)
                ),
            ));
        }
        let sig_opt = if numeric {
            None
        } else if let ValueType::ObjectRef(ref schema_key) = target_ty {
            let schema = self
                .env
                .object_schemas
                .get(schema_key)
                .expect("ObjectRef schema must exist");
            let sig = schema.functionals.get(kind_name).cloned().ok_or_else(|| {
                Error::at(
                    *kspan,
                    format!(
                        "object '{}' does not declare 'functional {kind_name}'",
                        schema.display_name
                    ),
                )
            })?;
            Some(sig)
        } else {
            None
        };
        // Operands that later argument code cannot change (literals, BCON entries, `my.id`)
        // are used in place; anything else is staged when argument code follows it.
        let is_fixed =
            |air: &str| air.parse::<i16>().is_ok() || air.starts_with("bcon.") || air == "my.id";
        let positional = args[1..].iter().filter(|a| a.name.is_none()).count();
        // The live cursor as receiver stays in place unless argument code moves it (below).
        let receiver_is_cursor = target_air == "stack_obj";
        let args_at = self.em.mark();
        let mut staged_target = if positional == 0 || is_fixed(&target_air) || receiver_is_cursor {
            target_air
        } else {
            let staged = self.fresh("rf_obj");
            self.emit_let(&staged, &target_air);
            staged
        };
        let mut change_icon = false;
        let mut staged_args: Vec<(String, ValueType, Span)> = Vec::new();
        let mut seen_positional = 0usize;
        for arg in &args[1..] {
            if let Some(aname) = &arg.name {
                if aname == "change_icon" {
                    change_icon = self.eval_const_bool_required(&arg.value, arg.span)?;
                } else {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown run_functional option '{aname}'"),
                    ));
                }
            } else {
                seen_positional += 1;
                let (val, val_ty) = self.lower_value_expr(&arg.value)?;
                // The last argument is followed only by the temp copies, so it stays in place
                // unless it reads a temp those copies overwrite.
                let last = seen_positional == positional;
                let staged = if is_fixed(&val) || (last && !val.starts_with("temp[")) {
                    val
                } else {
                    let staged = self.fresh("rf_arg");
                    self.emit_let(&staged, &val);
                    staged
                };
                staged_args.push((staged, val_ty, arg.span));
            }
        }
        if staged_args.len() > 8 {
            return Err(Error::at(
                span,
                "run_functional() supports at most 8 temp arguments (temp[0..7])",
            ));
        }
        if let Some(ref sig) = sig_opt {
            if staged_args.len() != sig.params.len() {
                return Err(Error::at(
                    span,
                    format!(
                        "functional '{kind_name}' expects {} argument(s), found {}",
                        sig.params.len(),
                        staged_args.len()
                    ),
                ));
            }
            for (idx, ((_, actual_ty, aspan), expected_ty)) in
                staged_args.iter().zip(sig.params.iter()).enumerate()
            {
                if !types_compatible_for_assign(expected_ty, actual_ty) {
                    return Err(Error::at(
                        *aspan,
                        format!(
                            "argument {} to functional '{kind_name}' expects '{}', found '{}'",
                            idx + 1,
                            self.env.display_value_type(expected_ty),
                            self.env.display_value_type(actual_ty)
                        ),
                    ));
                }
            }
        }
        // Argument code may move the cursor (reading another object's cells puts back the
        // entry object, and a primitive such as `set_to_next` steps it), so when there is any,
        // save the cursor before it and select it again for the call (which leaves the caller's
        // frame's cursor as it was).
        let mut cursor_saved = false;
        if receiver_is_cursor && !self.em.since(args_at).is_empty() {
            let args_text = self.em.split_off(args_at);
            let staged = self.fresh("rf_obj");
            self.emit_let(&staged, "stack_obj");
            self.em.append(&args_text);
            staged_target = staged;
            cursor_saved = true;
        }
        for (i, (val, _, _)) in staged_args.iter().enumerate() {
            self.line(format!("temp[{i}] := {val};"));
        }
        let saved_stack_obj = if cursor_saved {
            self.stack_op(mir::StackObjOp::Set(&staged_target));
            StackRestore::Unmoved
        } else if staged_target != "stack_obj" {
            let saved = self.save_stack_obj_for_move();
            self.stack_op(mir::StackObjOp::Set(&staged_target));
            StackRestore::Moved(saved)
        } else {
            StackRestore::Unmoved
        };
        let prim = if change_icon {
            format!("run_functional_tree({kind_name}, change_icon: true)")
        } else {
            format!("run_functional_tree({kind_name})")
        };
        Ok(PreparedFunctional {
            prim,
            sig: sig_opt,
            kind_name: kind_name.clone(),
            saved_stack_obj,
        })
    }

    pub(super) fn emit_run_functional(
        &mut self,
        receiver: &V2Expr,
        args: &[CallArg],
        span: Span,
        result_local: Option<&str>,
    ) -> Result<()> {
        let PreparedFunctional {
            prim,
            saved_stack_obj,
            ..
        } = self.prepare_run_functional(receiver, args, span)?;
        if let Some(dst) = result_local {
            self.line(format!("if {prim} {{"));
            self.em.indent();
            self.emit_store(dst, ":=", "1");
            self.em.dedent();
            self.line("} else {".to_owned());
            self.em.indent();
            self.emit_store(dst, ":=", "0");
            self.em.dedent();
            self.line("}".to_owned());
        } else {
            self.line(format!("{prim};"));
        }
        self.emit_stack_restore(&saved_stack_obj);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_run_functional_let_some(
        &mut self,
        receiver: &V2Expr,
        type_args: &[V2Type],
        args: &[CallArg],
        span: Span,
        binding_name: &str,
        ty_ann: Option<&V2Type>,
        else_body: &[V2Stmt],
    ) -> Result<(ValueType, StackRestore)> {
        if type_args.len() > 1 {
            return Err(Error::at(
                span,
                "run_functional::<T>() accepts at most 1 return type argument",
            ));
        }
        let PreparedFunctional {
            prim,
            sig: sig_opt,
            kind_name,
            saved_stack_obj,
        } = self.prepare_run_functional(receiver, args, span)?;
        let is_untyped_receiver = sig_opt.is_none();
        let inferred_ty = match sig_opt {
            Some(sig) => sig.return_type.ok_or_else(|| {
                Error::at(
                    span,
                    format!(
                        "functional '{kind_name}' does not declare a return type (use 'if !obj.run_functional({kind_name})' instead of 'let Some(...)')"
                    ),
                )
            })?,
            None => {
                if let Some(t) = type_args.first() {
                    self.env
                        .resolve_value_type(t, &self.owner_module, self.in_object_scope, span)?
                } else {
                    ValueType::I16
                }
            }
        };
        let after_generic_ty = if let Some(t) = type_args.first() {
            let expected =
                self.env
                    .resolve_value_type(t, &self.owner_module, self.in_object_scope, span)?;
            if !types_compatible_for_assign(&expected, &inferred_ty) {
                return Err(Error::at(
                    span,
                    format!(
                        "run_functional::<{}> does not match functional '{kind_name}' return type '{}'",
                        self.env.display_value_type(&expected),
                        self.env.display_value_type(&inferred_ty)
                    ),
                ));
            }
            expected
        } else {
            inferred_ty
        };
        let final_ty = if let Some(ann) = ty_ann {
            let expected =
                self.env
                    .resolve_value_type(ann, &self.owner_module, self.in_object_scope, span)?;
            if !types_compatible_for_assign(&expected, &after_generic_ty) {
                return Err(Error::at(
                    span,
                    format!(
                        "'let Some({binding_name}: {})' does not match functional '{kind_name}' return type '{}'",
                        self.env.display_value_type(&expected),
                        self.env.display_value_type(&after_generic_ty)
                    ),
                ));
            }
            expected
        } else {
            after_generic_ty
        };
        if is_untyped_receiver && !matches!(final_ty, ValueType::I16 | ValueType::ObjectId) {
            return Err(Error::at(
                span,
                format!(
                    "untyped ObjectId.run_functional({kind_name}) cannot safely manufacture nominal type '{}' (bind the target via a typed object reference such as 'find_best_object::<T>' or 'objects_of::<T>')",
                    self.env.display_value_type(&final_ty)
                ),
            ));
        }

        self.line(format!("if !{prim} {{"));
        self.em.indent();
        self.emit_stack_restore(&saved_stack_obj);
        self.push_scope(ScopeFrame::block());
        let r = self.lower_stmts(else_body);
        let p = self.pop_scope_and_run_cleanups();
        self.em.dedent();
        self.line("}".to_owned());
        r.and(p)?;
        Ok((final_ty, saved_stack_obj))
    }

    pub(super) fn resolve_motive_expr(&self, expr: &V2Expr) -> Result<u8> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    return match binding {
                        Binding::Motive(m) => Ok(*m),
                        _ => Err(Error::at(
                            *span,
                            format!("variable '{name}' is not a Motive"),
                        )),
                    };
                }
                if self.env.i16_params.contains_key(name) || self.env.bool_params.contains_key(name)
                {
                    return Err(Error::at(
                        *span,
                        format!(
                            "parameter '{name}' is not a Motive (use 'Motive::...' to refer to a motive explicitly)"
                        ),
                    ));
                }
                motive_by_name(name)
                    .ok_or_else(|| Error::at(*span, format!("unknown motive '{name}'")))
            }
            V2Expr::Path(span, segs) if segs.len() == 2 && segs[0] == "Motive" => {
                let lower = segs[1].to_ascii_lowercase();
                motive_by_name(&lower)
                    .ok_or_else(|| Error::at(*span, format!("unknown Motive::{}", segs[1])))
            }
            V2Expr::Int(span, v) => u8::try_from(*v)
                .ok()
                .filter(|&i| i < 16)
                .ok_or_else(|| Error::at(*span, format!("motive index {v} out of 0..16 range"))),
            other => Err(Error::at(
                other.span(),
                "expected motive name (e.g. energy, Motive::Energy) or index",
            )),
        }
    }

    pub(super) fn resolve_skill_expr(&self, expr: &V2Expr) -> Result<u8> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    return match binding {
                        Binding::Skill(sk) => Ok(*sk),
                        _ => Err(Error::at(
                            *span,
                            format!("variable '{name}' is not a Skill"),
                        )),
                    };
                }
                if self.env.i16_params.contains_key(name) || self.env.bool_params.contains_key(name)
                {
                    return Err(Error::at(
                        *span,
                        format!(
                            "parameter '{name}' is not a Skill (use 'Skill::...' to refer to a skill explicitly)"
                        ),
                    ));
                }
                skill_by_name(name)
                    .ok_or_else(|| Error::at(*span, format!("unknown skill '{name}'")))
            }
            V2Expr::Path(span, segs) if segs.len() == 2 && segs[0] == "Skill" => {
                let lower = segs[1].to_ascii_lowercase();
                skill_by_name(&lower)
                    .ok_or_else(|| Error::at(*span, format!("unknown Skill::{}", segs[1])))
            }
            other => Err(Error::at(
                other.span(),
                "expected skill (e.g. Skill::Cooking, Skill::Mechanical, Skill::Charisma, Skill::Body, Skill::Logic, Skill::Creativity)",
            )),
        }
    }

    pub(super) fn resolve_sound_expr(&self, expr: &V2Expr) -> Result<u16> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(Binding::Sound(id)) = self.lookup_var(name) {
                    return Ok(*id);
                }
                if let Some(s) = self.env.sounds.get(name) {
                    return Ok(s.id);
                }
                if self.const_item(name).is_some() {
                    return u16::try_from(self.eval_builtin_int(expr, "sound id", 0, 65535)?)
                        .map_err(|_| Error::at(*span, "sound id out of range"));
                }
                Err(Error::at(*span, format!("unknown sound '{name}'")))
            }
            // A library (global) sound by FWAV id, as a literal or `const`.
            V2Expr::Int(..) => u16::try_from(self.eval_builtin_int(expr, "sound id", 0, 65535)?)
                .map_err(|_| Error::at(expr.span(), "sound id out of range")),
            other => Err(Error::at(
                other.span(),
                "expected typed sound identifier (declared with 'sound name = preset(...);') or an FWAV id",
            )),
        }
    }

    pub(super) fn resolve_slot_expr(&self, expr: &V2Expr) -> Result<u16> {
        match expr {
            V2Expr::Ident(span, name) => {
                if let Some(Binding::Slot(idx)) = self.lookup_var(name) {
                    return Ok(*idx);
                }
                match self.env.slots.get(name) {
                    Some((Some(idx), _)) => Ok(*idx),
                    Some((None, _)) => Err(Error::at(
                        *span,
                        format!(
                            "slot '{name}' is not a routing slot (only 'kind: routing' slots can be used with route_to/snap/Slot)"
                        ),
                    )),
                    None => Err(Error::at(*span, format!("unknown slot '{name}'"))),
                }
            }
            V2Expr::Int(span, v) => u16::try_from(*v)
                .map_err(|_| Error::at(*span, format!("slot index {v} out of u16 range"))),
            other => Err(Error::at(other.span(), "expected slot identifier or index")),
        }
    }

    /// `play_sound(sound [, target] [, flags: N])`. The target is `self` (the object), `sim`,
    /// `stack_obj`, or a literal `bool` (`true` = the stack object, as in `.air`); by default the
    /// object. Flag 0x02 is the target's, so `flags:` may not set it.
    pub(super) fn format_play_sound(&self, args: &[CallArg], span: Span) -> Result<(String, bool)> {
        let bound = self.bind_builtin_args("play_sound", args, &["sound", "target"], &["flags"])?;
        let Some(sound) = bound.get("sound") else {
            return Err(Error::at(
                span,
                "play_sound(sound [, target] [, flags: N]) requires a sound",
            ));
        };
        let sound_id = self.resolve_sound_expr(&sound.value)?;
        let on_self = self.sound_targets_self(bound.get("target").copied());
        let on_stack = match bound.get("target") {
            Some(t) => self.sound_target_is_stack_obj("play_sound", &t.value)?,
            None => matches!(self.tree_ctx, TreeContext::SimOnObject { .. }),
        };
        let Some(f) = bound.get("flags") else {
            let target = if on_stack { "stack_obj" } else { "false" };
            return Ok((format!("play_sound({sound_id}, {target})"), on_self));
        };
        let flags = self.eval_builtin_int(&f.value, "play_sound flags", 0, 255)?;
        if flags & 0x02 != 0 {
            return Err(Error::at(
                f.span,
                "play_sound flags 0x02 selects the stack object; pass 'stack_obj' as the target instead",
            ));
        }
        let flags = flags | if on_stack { 0x02 } else { 0 };
        // The VM plays on `me` whenever the flags byte is 0xFF, whatever bit 0x02 says.
        if flags == 0xFF {
            return Err(Error::at(
                f.span,
                "play_sound flags 0xFD with the stack object as target encode as 0xFF, which the VM plays on the running object",
            ));
        }
        Ok((format!("play_sound({sound_id}, flags: {flags})"), on_self))
    }

    /// Whether a sound primitive's target is the stack object (its flag) rather than `me`.
    pub(super) fn sound_target_is_stack_obj(&self, prim: &str, target: &V2Expr) -> Result<bool> {
        match target {
            V2Expr::Bool(_, b) => Ok(*b),
            V2Expr::Ident(_, n) if n == "stack_obj" => Ok(true),
            V2Expr::Ident(_, n) if self.is_self_ident(n) => {
                Ok(matches!(self.tree_ctx, TreeContext::SimOnObject { .. }))
            }
            V2Expr::Ident(_, n) if self.is_my_ident(n) => Ok(false),
            other => Err(Error::at(
                other.span(),
                format!("{prim} target must be 'self', 'sim', 'stack_obj', or a literal bool"),
            )),
        }
    }

    pub(super) fn sound_targets_self(&self, target: Option<&CallArg>) -> bool {
        target.is_none_or(|arg| matches!(&arg.value, V2Expr::Ident(_, n) if self.is_self_ident(n)))
    }

    pub(super) fn format_stop_all_sounds(&self, args: &[CallArg]) -> Result<(String, bool)> {
        let bound = self.bind_builtin_args("stop_all_sounds", args, &["target"], &[])?;
        let use_stack = match bound.get("target") {
            Some(t) => self.sound_target_is_stack_obj("stop_all_sounds", &t.value)?,
            None => matches!(self.tree_ctx, TreeContext::SimOnObject { .. }),
        };
        Ok((
            format!("stop_all_sounds({use_stack})"),
            self.sound_targets_self(bound.get("target").copied()),
        ))
    }

    /// `sim.debit_funds(n)` / `sim.credit_funds(n)`: the Sim's household budget, the same
    /// primitive as the call form (a statement, a condition, or a bool value).
    pub(super) fn format_budget_method(
        &mut self,
        receiver: &V2Expr,
        method: &str,
        type_args: &[V2Type],
        args: &[CallArg],
        span: Span,
    ) -> Result<String> {
        if !type_args.is_empty() {
            return Err(Error::at(
                span,
                format!("'.{method}()' does not accept type arguments"),
            ));
        }
        if !matches!(receiver, V2Expr::Ident(_, n) if self.is_sim_ident(n)) {
            return Err(Error::at(
                receiver.span(),
                format!(
                    "'.{method}()' is called on the interaction's Sim (its household's budget)"
                ),
            ));
        }
        self.format_alter_budget(args, span, false, method == "credit_funds")
    }

    pub(super) fn format_alter_budget(
        &mut self,
        args: &[CallArg],
        span: Span,
        test_only: bool,
        income: bool,
    ) -> Result<String> {
        let prim = match (test_only, income) {
            (true, _) => "can_afford",
            (false, true) => "credit_funds",
            (false, false) => "debit_funds",
        };
        let bound = self.bind_builtin_args(prim, args, &["amount"], &["category"])?;
        let Some(amount) = bound.get("amount") else {
            return Err(Error::at(span, "budget call requires an amount argument"));
        };
        let (amount_air, _) = self.lower_value_expr(&amount.value)?;
        let category = match bound.get("category") {
            Some(a) => self.eval_builtin_int(&a.value, &format!("{prim} category"), 0, 255)?,
            None => 0,
        };
        let mut out = format!("alter_budget({amount_air}, category: {category}");
        if test_only {
            out.push_str(", test_only: true");
        }
        if income {
            out.push_str(", income: true");
        }
        out.push(')');
        Ok(out)
    }

    pub(super) fn parse_find_best_object(
        &self,
        type_args: &[V2Type],
        args: &[CallArg],
        span: Span,
    ) -> Result<FindBestObjectPlan> {
        let bound = self.bind_builtin_args("find_best_object", args, &["kind", "flags"], &[])?;
        let Some(kind_arg) = bound.get("kind") else {
            return Err(Error::at(
                span,
                "find_best_object(kind [, flags: N]) requires a functional kind",
            ));
        };
        let V2Expr::Ident(kspan, kind_name) = &kind_arg.value else {
            return Err(Error::at(
                kind_arg.span,
                "find_best_object() expects a functional kind identifier (e.g. prep, cook)",
            ));
        };
        // `category4` (function 15) searches category-4 objects by a fixed score and has no
        // callback, so only the untyped search takes it.
        if !(kind_name == "category4" && type_args.is_empty()) {
            functional_kind_info(kind_name, *kspan)?;
        }
        if crate::parser::function_kind(kind_name)
            .is_some_and(|k| k.search == crate::parser::FunctionSearch::None)
        {
            return Err(Error::at(
                *kspan,
                format!(
                    "find_best_object({kind_name}) always fails: Find Best Object never searches for '{kind_name}'"
                ),
            ));
        }
        let typed_schema = if type_args.is_empty() {
            None
        } else if type_args.len() == 1 {
            let V2Type::Named(ref obj_name) = type_args[0] else {
                return Err(Error::at(
                    span,
                    "find_best_object::<T>() expects an imported object type name",
                ));
            };
            let schema = self
                .env
                .lookup_object_schema(&self.owner_module, obj_name, self.in_object_scope)
                .ok_or_else(|| {
                    Error::at(
                        span,
                        format!(
                            "unknown object type '{obj_name}' in 'find_best_object::<{obj_name}>({kind_name})'"
                        ),
                    )
                })?;
            if !schema.functionals.contains_key(kind_name) {
                return Err(Error::at(
                    *kspan,
                    format!(
                        "object '{}' does not declare 'functional {kind_name}'",
                        schema.display_name
                    ),
                ));
            }
            Some(schema.clone())
        } else {
            return Err(Error::at(
                span,
                "find_best_object::<T>() accepts at most 1 type argument",
            ));
        };
        let flags_opt = match bound.get("flags") {
            Some(a) => Some(
                u8::try_from(self.eval_builtin_int(&a.value, "find_best_object flags", 0, 255)?)
                    .expect("checked range"),
            ),
            None => None,
        };
        let flags = flags_opt.unwrap_or(0);
        if typed_schema.is_some() && (flags & 0x0b) != 0 {
            return Err(Error::at(
                span,
                format!(
                    "typed find_best_object::<T>() does not support indoor/outdoor or custom-origin flags ({flags:#04x}); use default flags (0) or same-room (4)"
                ),
            ));
        }
        let prim_air = match flags_opt {
            Some(f) => format!("find_best_object({kind_name}, flags: {f})"),
            None => format!("find_best_object({kind_name})"),
        };
        Ok(FindBestObjectPlan {
            kind_name: kind_name.clone(),
            flags,
            prim_air,
            typed_schema,
        })
    }

    /// Remove Object Instance's flags from named options: `flags: N`, `no_yield: b` (bit 0: an
    /// object that removes itself does not yield) and `skip_resets: b` (bit 1).
    pub(super) fn remove_object_flags(&self, prim: &str, options: &[CallArg]) -> Result<u8> {
        let mut flags = 0u8;
        for a in options {
            let Some(aname) = &a.name else {
                return Err(Error::at(
                    a.span,
                    format!(
                        "{prim}() only takes named options ('flags:', 'no_yield:', 'skip_resets:')"
                    ),
                ));
            };
            match aname.as_str() {
                "flags" => {
                    let v = self.eval_const_i16(&a.value)?;
                    flags |= u8::try_from(v).map_err(|_| {
                        Error::at(a.span, format!("{prim} flags {v} out of range 0..=255"))
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
                            "unknown {prim} option '{other}' (expected 'flags', 'no_yield', or 'skip_resets')"
                        ),
                    ));
                }
            }
        }
        Ok(flags)
    }

    /// `remove_object(target, ..)` / `target.remove(..)`: Remove Object Instance (opcode 18),
    /// whose operand names the object by mode, not by value (`sims-vm` `primitives/remove.rs`):
    /// 0 is the running object (`me`), 1 the stack object. So the target is `me` (the Sim handle
    /// in a Sim tree), `stack_obj`, a handle that is the stack object itself (a cursor loop's
    /// binding), or `self` where `self` is one of those. Another handle would need the stack
    /// object moved to it first, which is a node the source should spell. The primitive takes
    /// its false branch when the object is already gone.
    pub(super) fn format_remove_object(
        &self,
        prim: &str,
        target: &V2Expr,
        options: &[CallArg],
    ) -> Result<String> {
        let flags = self.remove_object_flags(prim, options)?;
        let other = || {
            Error::at(
                target.span(),
                format!(
                    "{prim}() removes 'me', 'stack_obj', or a handle the stack object holds; \
                     select another object first ('stack_obj = x;') and remove 'stack_obj'"
                ),
            )
        };
        let V2Expr::Ident(_, name) = target else {
            return Err(other());
        };
        let air = if name == "stack_obj" {
            "stack_obj"
        } else if self.is_self_ident(name) {
            if self.inherit_tree {
                return Err(Error::at(
                    target.span(),
                    format!(
                        "{prim}(): a 'context = inherit' tree runs on whichever object calls it; \
                         name 'me' or 'stack_obj'"
                    ),
                ));
            }
            match self.tree_ctx {
                TreeContext::ObjectSelf => "me",
                TreeContext::SimOnObject { .. } if self.self_stack_target() == "stack_obj" => {
                    "stack_obj"
                }
                TreeContext::SimOnObject { .. } => {
                    return Err(Error::at(
                        target.span(),
                        format!(
                            "{prim}(self): this Sim tree moves 'stack_obj', so 'self' is no longer \
                             the stack object; use 'destroy_self()', which selects it"
                        ),
                    ));
                }
            }
        } else if self.is_my_ident(name) || self.is_frame_me(name) {
            "me"
        } else if self
            .object_handle_air(name)
            .is_some_and(|(air, _)| air == "stack_obj")
        {
            "stack_obj"
        } else {
            return Err(other());
        };
        Ok(if flags == 0 {
            format!("remove_object({air})")
        } else {
            format!("remove_object({air}, flags: {flags})")
        })
    }

    /// The method form: `target.remove(..)`.
    pub(super) fn format_remove_method(
        &self,
        receiver: &V2Expr,
        type_args: &[V2Type],
        options: &[CallArg],
        span: Span,
    ) -> Result<String> {
        if !type_args.is_empty() {
            return Err(Error::at(
                span,
                "'.remove()' does not accept type arguments",
            ));
        }
        self.format_remove_object("remove", receiver, options)
    }

    /// The call form: `remove_object(target, ..)`, its target the one positional argument.
    pub(super) fn format_remove_object_call(&self, args: &[CallArg], span: Span) -> Result<String> {
        let Some((target, options)) = args.split_first().filter(|(t, _)| t.name.is_none()) else {
            return Err(Error::at(
                span,
                "remove_object(target [, flags: N, no_yield: b, skip_resets: b]) takes its target first",
            ));
        };
        self.format_remove_object("remove_object", &target.value, options)
    }

    pub(super) fn require_stack_obj_dest(prim: &str, arg: &CallArg) -> Result<()> {
        if matches!(&arg.value, V2Expr::Ident(_, name) if name == "stack_obj") {
            Ok(())
        } else {
            Err(Error::at(
                arg.span,
                format!("{prim}(dest:) expects stack_obj"),
            ))
        }
    }

    /// Steps the current stack object without initializing or restoring it. The explicit
    /// destination spelling preserves the AIR primitive's destination flag in the operand.
    pub(super) fn format_set_to_next(&self, args: &[CallArg], span: Span) -> Result<String> {
        let bound =
            self.bind_builtin_args("set_to_next", args, &["mode", "guid"], &["dest", "local"])?;
        let mode_arg = bound
            .get("mode")
            .ok_or_else(|| Error::at(span, "set_to_next() requires a mode"))?;
        let mode = match &mode_arg.value {
            V2Expr::Ident(_, name) => match name.as_str() {
                "all" => 0,
                "person" => 1,
                "non_person" => 2,
                "part" | "tile" => 3,
                "guid" => 4,
                "neighbor" => 5,
                "category" => 6,
                "neighbor_guid" => 7,
                "on_tile" => 8,
                "beside" => 9,
                "career" => 10,
                "family" => 12,
                _ => self.eval_builtin_int(&mode_arg.value, "set_to_next mode", 0, 12)?,
            },
            expr => self.eval_builtin_int(expr, "set_to_next mode", 0, 12)?,
        };
        let mut air = format!("set_to_next({mode}");
        if let Some(arg) = bound.get("guid") {
            let guid = self.resolve_guid_expr(&arg.value)?;
            let _ = write!(air, ", guid: {guid:#010x}");
        }
        if let Some(arg) = bound.get("dest") {
            Self::require_stack_obj_dest("set_to_next", arg)?;
            air.push_str(", dest: stack_obj");
        }
        match (mode, bound.get("local")) {
            (9, Some(arg)) => {
                let local = match &arg.value {
                    V2Expr::Ident(_, name) => match self.lookup_var(name) {
                        Some(Binding::Local { air_name, ty }) if ty.is_object_handle() => {
                            air_name.clone()
                        }
                        _ => self.builtin_local_arg(&arg.value, "set_to_next(local:)")?,
                    },
                    _ => self.builtin_local_arg(&arg.value, "set_to_next(local:)")?,
                };
                let _ = write!(air, ", local: {local}");
            }
            (9, None) => return Err(Error::at(span, "set_to_next(beside) requires 'local:'")),
            (_, Some(arg)) => {
                return Err(Error::at(
                    arg.span,
                    "only set_to_next(beside) reads 'local:'",
                ));
            }
            _ => {}
        }
        air.push(')');
        Ok(air)
    }

    pub(super) fn format_create_object(
        &self,
        type_args: &[V2Type],
        args: &[CallArg],
        span: Span,
    ) -> Result<(String, ValueType)> {
        let is_typed = !type_args.is_empty();
        let (guid, result_ty, opt_args) = if is_typed {
            if type_args.len() != 1 {
                return Err(Error::at(
                    span,
                    "create_object::<T>() expects exactly 1 object type argument",
                ));
            }
            let V2Type::Named(ref obj_name) = type_args[0] else {
                return Err(Error::at(
                    span,
                    "create_object::<T>() expects an imported object type name",
                ));
            };
            let schema = self
                .env
                .lookup_object_schema(&self.owner_module, obj_name, self.in_object_scope)
                .ok_or_else(|| {
                    Error::at(
                        span,
                        format!(
                            "unknown object type '{obj_name}' in 'create_object::<{obj_name}>(...)'"
                        ),
                    )
                })?;
            (
                schema.master_guid,
                ValueType::ObjectRef(schema.canonical_key.clone()),
                args,
            )
        } else {
            if args.is_empty() {
                return Err(Error::at(
                    span,
                    "create_object(guid [, mode: ..., flags: ...]) requires a GUID or type argument 'create_object::<T>(...)'",
                ));
            }
            if args[0].name.is_some() {
                return Err(Error::at(
                    args[0].span,
                    "create_object() requires a GUID (literal or const) or 'create_object::<ObjectType>(...)'",
                ));
            }
            let guid = self.resolve_guid_expr(&args[0].value)?;
            (guid, ValueType::ObjectId, &args[1..])
        };
        let mut mode = "here".to_owned();
        let mut flags = 0u8;
        let mut local = None;
        let mut seen = BTreeSet::new();
        for arg in opt_args {
            if let Some(key) = &arg.name
                && !seen.insert(key.as_str())
            {
                return Err(Error::at(
                    arg.span,
                    format!("duplicate create_object() argument '{key}'"),
                ));
            }
            match arg.name.as_deref() {
                Some("dest") => Self::require_stack_obj_dest("create_object", arg)?,
                Some("local") => {
                    local = Some((
                        arg.span,
                        self.builtin_local_arg(&arg.value, "create_object(local:)")?,
                    ));
                }
                Some("mode") => match &arg.value {
                    V2Expr::Ident(_, m) => mode = m.clone(),
                    V2Expr::Int(_, n) => mode = n.to_string(),
                    other => {
                        return Err(Error::at(
                            other.span(),
                            "create_object mode must be an identifier (e.g. stack_ahead) or integer",
                        ));
                    }
                },
                Some("flags") => {
                    let f = self.eval_const_i16(&arg.value)?;
                    flags = u8::try_from(f)
                        .map_err(|_| Error::at(arg.span, format!("flags {f} out of u8 range")))?;
                }
                Some(other) => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown create_object option '{other}'"),
                    ));
                }
                None => {
                    return Err(Error::at(
                        arg.span,
                        "extra create_object arguments must be named ('mode:', 'flags:', 'local:', 'dest:')",
                    ));
                }
            }
        }
        // Modes `local_pos` (8) and `turned` (9) read a local slot: it must be named.
        let reads_local = matches!(mode.as_str(), "local_pos" | "turned" | "8" | "9");
        let local_text = match (reads_local, local) {
            (true, Some((_, air))) => format!(", local: {air}"),
            (true, None) => {
                return Err(Error::at(
                    span,
                    format!("create_object mode '{mode}' reads a local; name it with 'local:'"),
                ));
            }
            (false, Some((lspan, _))) => {
                return Err(Error::at(
                    lspan,
                    format!("create_object mode '{mode}' does not read 'local:'"),
                ));
            }
            (false, None) => String::new(),
        };
        if is_typed && (flags & 0x04) != 0 {
            return Err(Error::at(
                span,
                "create_object::<T>() cannot set FROM_RECORD flag (0x04), which overrides T's GUID from a neighbor record at runtime; use untyped create_object(0, ..., flags: 0x04)",
            ));
        }
        Ok((
            format!("create_object({guid:#010x}, mode: {mode}, flags: {flags:#04x}{local_text})"),
            result_ty,
        ))
    }

    /// `route_to(slot [, flags] [, source: ...])` / `snap(slot [, flags] [, mode: ...])`.
    /// Each operand is bound by name or position (`slot`, then `flags`); the routing source
    /// and snap mode are named only.
    pub(super) fn format_slot_prim(
        &self,
        prim: &str,
        args: &[CallArg],
        span: Span,
    ) -> Result<String> {
        let (extra_key, extra_default) = if prim == "snap" {
            ("mode", 3)
        } else {
            ("source", 1)
        };
        let bound = self.bind_builtin_args(prim, args, &["slot", "flags"], &[extra_key])?;
        let Some(slot_arg) = bound.get("slot") else {
            return Err(Error::at(
                span,
                format!("{prim}(slot [, flags: N] [, {extra_key}: ...]) requires a slot"),
            ));
        };
        let slot_idx = self.resolve_slot_expr(&slot_arg.value)?;
        let flags = match bound.get("flags") {
            Some(a) => self.eval_builtin_int(&a.value, &format!("{prim} flags"), 0, 255)?,
            None => 0,
        };
        let extra = match bound.get(extra_key) {
            Some(a) => self.slot_prim_mode(prim, extra_key, a)?,
            None => extra_default,
        };
        let mut out = format!("{prim}({slot_idx}");
        if flags != 0 {
            out.push_str(&format!(", flags: {flags}"));
        }
        if extra != extra_default {
            out.push_str(&format!(", {extra_key}: {extra}"));
        }
        out.push(')');
        Ok(out)
    }

    /// The routing source (`param`, `object`, `global`, or `0..=2`) or snap mode (`param`,
    /// `object`, `front`, `slot`, `global`, or `0..=4`) of a slot primitive.
    pub(super) fn slot_prim_mode(&self, prim: &str, key: &str, arg: &CallArg) -> Result<i64> {
        let names: &[(&str, i64)] = if key == "mode" {
            &[
                ("param", 0),
                ("object", 1),
                ("container", 1),
                ("front", 2),
                ("slot", 3),
                ("global", 4),
            ]
        } else {
            &[("param", 0), ("object", 1), ("slot", 1), ("global", 2)]
        };
        if let V2Expr::Ident(ispan, name) = &arg.value
            && self.lookup_var(name).is_none()
            && self.const_item(name).is_none()
        {
            return names
                .iter()
                .find(|(n, _)| *n == name.as_str())
                .map(|(_, v)| *v)
                .ok_or_else(|| {
                    Error::at(
                        *ispan,
                        format!(
                            "unknown {prim} {key} '{name}' (expected {} or an integer)",
                            names.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                        ),
                    )
                });
        }
        let hi = names.iter().map(|(_, v)| *v).max().unwrap_or(0);
        self.eval_builtin_int(&arg.value, &format!("{prim} {key}"), 0, hi)
    }

    pub(super) fn format_set_motive_change(
        &mut self,
        args: &[CallArg],
        span: Span,
    ) -> Result<String> {
        let bound = self.bind_builtin_args(
            "set_motive_change",
            args,
            &["motive", "delta_per_hour", "stop_at"],
            &[],
        )?;
        let (Some(motive), Some(delta), Some(stop_at)) = (
            bound.get("motive"),
            bound.get("delta_per_hour"),
            bound.get("stop_at"),
        ) else {
            return Err(Error::at(
                span,
                "set_motive_change(motive, delta_per_hour, stop_at) expects 3 arguments",
            ));
        };
        let m = self.resolve_motive_expr(&motive.value)?;
        let (delta, _) = self.lower_value_expr(&delta.value)?;
        let delta_arg = if self.is_pure_single_operand(&stop_at.value) {
            delta
        } else {
            let snap = self.fresh("mc_d");
            self.emit_let(&snap, &delta);
            snap
        };
        let (stop, _) = self.lower_value_expr(&stop_at.value)?;
        Ok(format!("set_motive_change({m}, {delta_arg}, {stop})"))
    }
}
