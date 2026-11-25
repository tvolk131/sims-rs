//! `TreeLowerer` core: construction, body/frame bookkeeping, the `.air` line writer, fresh names,
//! variable bindings and `finish` (per-tree post-passes).

use super::*;

impl<'a> TreeLowerer<'a> {
    pub(super) fn new(
        env: &'a ObjectEnv<'a>,
        tree_ctx: TreeContext,
        owner_module: &str,
        has_self_obj_local: bool,
    ) -> Self {
        let mut root_vars = BTreeMap::new();
        if let TreeContext::SimOnObject { ref sim_name } = tree_ctx {
            root_vars.insert(sim_name.clone(), Binding::SimHandle);
        }
        Self {
            env,
            tree_ctx,
            owner_module: owner_module.to_owned(),
            in_object_scope: true,
            has_self_obj_local,
            explicit_stack_obj: false,
            used_saved_stack_obj: false,
            value_return: None,
            em: mir::Emitter::new(1),
            next_id: 0,
            var_scopes: vec![root_vars],
            cleanup_scopes: vec![ScopeFrame::block()],
            held_locks: Vec::new(),
            defer_generation: 0,
            current_defer_id: None,
            inline_call_stack: Vec::new(),
            inline_expansions: 0,
            active_inline_fn: None,
            checking_discarded: 0,
            frame_args: 0,
            pin_span: None,
            raw_span: None,
            raw_declared: BTreeSet::new(),
            raw_consts: BTreeMap::new(),
            raw_const_owner: BTreeMap::new(),
            raw_names: BTreeMap::new(),
            raw_formats_text: false,
            slot_span: None,
            pinned_locals: BTreeMap::new(),
            seen_decls: BTreeSet::new(),
            inline_call_sites: Vec::new(),
            dup_decl: None,
            keep_layout: false,
            param_words_used: 0,
            frame_locals: None,
            event_local: None,
            sleep_span: None,
        }
    }

    pub(super) fn begin_body(&mut self, explicit_stack_obj: bool) {
        self.track_stack_obj(explicit_stack_obj);
        if self.has_self_obj_local {
            self.stack_op(mir::StackObjOp::Save(mir::SELF_OBJ));
        }
    }

    pub(super) fn in_defer(&self) -> bool {
        self.current_defer_id.is_some()
    }

    pub(super) fn finish(self) -> Result<String> {
        let formats_text = self.raw_formats_text || mir::body_formats_text(self.em.text());
        if let Some((span, name)) = &self.dup_decl
            && formats_text
        {
            return Err(Error::at(
                *span,
                format!(
                    "'{name}' is declared in a const binding's region, which is copied into each \
                     arm with its own frame word, and this tree formats text that can name frame \
                     words ('$Local:N'); declare '{name}' before the binding and assign it in the region"
                ),
            ));
        }
        let facts = mir::FrameFacts {
            formats_text,
            keep_layout: self.keep_layout,
            has_self_obj_local: self.has_self_obj_local,
            pinned: self.pin_span.is_some(),
            raw_consts: &self.raw_consts,
            raw_declared: &self.raw_declared,
            has_slots: self.slot_span.is_some(),
            pinned_locals: &self.pinned_locals,
        };
        let body = self.em.into_text();
        let mut out = if self.used_saved_stack_obj {
            let save = mir::StackObjOp::Save(mir::SAVED_STACK_OBJ).text();
            format!("    {save}\n{body}")
        } else {
            body
        };
        if self.pin_span.is_some() {
            // A tree with a pinned `push_interaction` has no raw block (that is an error), so
            // nothing else names an explicit local: the pin takes slot 0, and `.air` allocates
            // the named locals after it.
            out = replace_outside_strings(&out, PIN_SLOT, "0");
        }
        Ok(mir::finish_body(out, &facts))
    }

    /// A lifecycle tree's `@frame(...)`: `param[0..args]` become addressable and `locals` bounds
    /// the frame, as on a standalone `tree`.
    pub(super) fn set_lifecycle_frame(&mut self, frame: &TreeFrame) {
        self.frame_locals = frame.locals;
        self.frame_args = frame.args.map_or(0, usize::from);
    }

    /// Adds `min_locals: N` to a tree header when an `@slot(N - 1)` word must be in the frame:
    /// the tree may never name it again (an uninitialized slot a raw primitive or text reads by
    /// number), and `.air` sizes the frame by the words the body names.
    pub(super) fn push_frame_floor(&self, header: &mut Vec<String>) {
        if let Some(top) = self.pinned_locals.values().max() {
            header.push(format!("min_locals: {}", top + 1));
        }
    }

    pub(super) fn fresh(&mut self, prefix: &str) -> String {
        let id = self.next_id;
        self.next_id += 1;
        let scope = if prefix.starts_with("v_") && !self.inline_call_stack.is_empty() {
            "inline_"
        } else {
            ""
        };
        format!("__{scope}{prefix}_{id}")
    }

    pub(super) fn line(&mut self, text: String) {
        self.em.line(&text);
    }

    pub(super) fn with_owner_module<T>(
        &mut self,
        module: &str,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let prev = std::mem::replace(&mut self.owner_module, module.to_owned());
        let res = f(self);
        self.owner_module = prev;
        res
    }

    /// Resolves a `tree_ident(...)` / `extern_mod::fn_name(...)` call target, if `callee` is one.
    /// Whether `name` is read-only in `stmts`: assigned, borrowed `&mut`, or handed to an
    /// unshadowed builtin that writes a named local.
    pub(super) fn stmts_write_ident(&self, stmts: &[V2Stmt], name: &str) -> Result<bool> {
        let mut active = Vec::new();
        for b in LOCAL_WRITING_BUILTINS {
            let callee = V2Expr::Ident(Span::default(), (*b).to_owned());
            if !self.user_callable_named(&callee, Span::default())? {
                active.push(*b);
            }
        }
        Ok(stmts_mutate_or_borrow_ident_in(stmts, name, &active))
    }

    pub(super) fn bind_var(&mut self, name: String, binding: Binding) {
        if let Some(top) = self.var_scopes.last_mut()
            && let Some(old) = top.insert(name.clone(), binding)
            && matches!(&old, Binding::Local { air_name, .. } if air_name.starts_with("__slot"))
        {
            // A `defer` registered before the shadowing still uses the old `@slot` variable,
            // so its frame word stays taken until the scope ends. The key is no identifier, so
            // only the slot conflict check in `local_air_name` sees it.
            let mut k = 0;
            while top.contains_key(&format!("{name}#shadowed{k}")) {
                k += 1;
            }
            top.insert(format!("{name}#shadowed{k}"), old);
        }
    }

    pub(super) fn lookup_var(&self, name: &str) -> Option<&Binding> {
        for scope in self.var_scopes.iter().rev() {
            if let Some(b) = scope.get(name) {
                return Some(b);
            }
        }
        None
    }

    pub(super) fn bind_func_param(
        &mut self,
        name: &str,
        idx: usize,
        ty: &V2Type,
        span: Span,
    ) -> Result<()> {
        let vty = self
            .env
            .resolve_value_type(ty, &self.owner_module, self.in_object_scope, span)
            .map_err(|_| {
                Error::at(
                    span,
                    format!(
                        "functional parameter '{name}' has unsupported type '{}' (expected i16, bool, ObjectId, enum, bitflags, or object type)",
                        ty.display_name()
                    ),
                )
            })?;
        self.bind_var(
            name.to_owned(),
            Binding::BhavParam {
                index: idx,
                ty: vty,
            },
        );
        Ok(())
    }

    pub(super) fn is_sim_ident(&self, name: &str) -> bool {
        matches!(self.lookup_var(name), Some(Binding::SimHandle))
    }

    pub(super) fn is_frame_me(&self, name: &str) -> bool {
        name == "me" && self.lookup_var(name).is_none()
    }

    pub(super) fn is_self_ident(&self, name: &str) -> bool {
        if matches!(self.lookup_var(name), Some(Binding::SelfHandle)) {
            return true;
        }
        name == "self"
    }

    /// The `.air` operand holding an object-handle binding (`ObjectId`/typed locals, params, and
    /// `Object` helper params), plus whether it may be a Sim (only untyped handles may).
    pub(super) fn object_handle_air(&self, name: &str) -> Option<(String, bool)> {
        // `stack_obj` is the frame's stack object (it cannot be bound as a name).
        if name == "stack_obj" {
            return Some(("stack_obj".to_owned(), true));
        }
        match self.lookup_var(name)? {
            Binding::Local { air_name, ty } if ty.is_object_handle() => {
                Some((air_name.clone(), *ty == ValueType::ObjectId))
            }
            Binding::BhavParam { index, ty } if ty.is_object_handle() => {
                Some((format!("param[{index}]"), *ty == ValueType::ObjectId))
            }
            Binding::ForeignObject { air_name } => Some((air_name.clone(), true)),
            _ => None,
        }
    }

    /// The object or Sim cell behind `handle.<field>` that every handle shares: `hand`, `room`,
    /// `flags` (object data word 8), and (untyped handles only) the person words `age`, `priority`, `is_visitor`, and `kind`.
    pub(super) fn handle_field_cell(
        field: &str,
        maybe_sim: bool,
    ) -> Option<(&'static str, ValueType)> {
        match field {
            "hand" => Some(("stack.slot[0]", ValueType::ObjectId)),
            "room" => Some(("stack.room", ValueType::I16)),
            "flags" => Some((
                "stack.data[8]",
                ValueType::Bitflags(ANY_BITFLAGS.to_owned()),
            )),
            _ if !maybe_sim => None,
            "age" => Some(("stack.person[58]", ValueType::I16)),
            "priority" => Some(("stack.person.priority", ValueType::I16)),
            "is_visitor" => Some(("stack.person[32]", ValueType::Bool)),
            "visitor_kind" => Some(("stack.person[32]", ValueType::I16)),
            "kind" => Some(("stack.person[65]", ValueType::I16)),
            _ => None,
        }
    }

    /// `obj.sprite_flags[i]`: scope 27 reads the stack object's dynamic-sprite flag whose index
    /// is in `temp[0]`. Returns the setup line and the cell.
    pub(super) fn sprite_flag_cell(&mut self, index: &V2Expr) -> Result<(String, String)> {
        let idx_air = if let Ok(v) = self.eval_const_i16(index) {
            v.to_string()
        } else if self.is_pure_single_operand(index) {
            let (air, ty) = self.lower_value_expr(index)?;
            if ty != ValueType::I16 {
                return Err(Error::at(
                    index.span(),
                    format!(
                        "sprite_flags index must be i16 (found '{}')",
                        self.env.display_value_type(&ty)
                    ),
                ));
            }
            air
        } else {
            return Err(Error::at(
                index.span(),
                "sprite_flags index must be a constant or a plain variable",
            ));
        };
        Ok((format!("temp[0] := {idx_air};"), "scope(27, 0)".to_owned()))
    }

    pub(super) fn self_prefix(&self) -> &'static str {
        match self.tree_ctx {
            TreeContext::ObjectSelf => "my",
            TreeContext::SimOnObject { .. } => "stack",
        }
    }

    pub(super) fn enclosing_cancel_flag(&self) -> Option<(usize, String)> {
        for (idx, frame) in self.cleanup_scopes.iter().enumerate().rev() {
            if let Some(flag) = &frame.cancel_flag {
                return Some((idx, flag.clone()));
            }
        }
        None
    }

    pub(super) fn nearest_air_while_idx(&self) -> Option<usize> {
        self.cleanup_scopes.iter().rposition(|f| f.is_air_while)
    }

    /// Records a statement that declares a local (see `seen_decls`).
    pub(super) fn note_decl(&mut self, stmt: &V2Stmt) {
        let (span, name) = match stmt {
            V2Stmt::Let { span, name, .. }
            | V2Stmt::LetUninit { span, name, .. }
            | V2Stmt::LetSome { span, name, .. }
            | V2Stmt::ForEach {
                span,
                binding: name,
                ..
            } => (*span, name.as_str()),
            V2Stmt::Animate {
                span,
                event: Some(name),
                ..
            } => (*span, name.as_str()),
            _ => return,
        };
        if self.checking_discarded > 0 || self.dup_decl.is_some() {
            return;
        }
        let depth = self.inline_call_stack.len();
        self.inline_call_sites.truncate(depth);
        let key = (
            self.inline_call_sites.clone(),
            self.current_defer_id,
            (span.line, span.column),
        );
        if !self.seen_decls.insert(key) {
            self.dup_decl = Some((span, name.to_owned()));
        }
    }

    /// The `.air` name of a new `let` variable: `__v_<name>_<n>`, or `__slot<N>_<name>_<n>` for
    /// `@slot(N) let`, which the optimizer never merges and `opt::assign_frame_slots` turns into
    /// `local[N]`. A slot belongs to the tree's frame, so `@slot` is refused in an inline helper
    /// (each caller's frame differs), next to a pinned `push_interaction` object slot, and while
    /// another `@slot` variable of the same slot is in scope.
    pub(super) fn local_air_name(
        &mut self,
        name: &str,
        slot: Option<u8>,
        span: Span,
    ) -> Result<String> {
        let Some(n) = slot else {
            return Ok(self.fresh(&format!("v_{name}")));
        };
        if !self.inline_call_stack.is_empty() || self.active_inline_fn.is_some() {
            return Err(Error::at(
                span,
                "'@slot(N) let' names a frame word of the tree, so it is not allowed in an inlined helper",
            ));
        }
        if let Some(locals) = self.frame_locals
            && u16::from(n) >= locals
        {
            return Err(Error::at(
                span,
                format!(
                    "'@slot({n})' names local[{n}], but '@frame(locals: {locals})' gives the tree {locals} local word(s)"
                ),
            ));
        }
        let prefix = format!("__slot{n}_");
        if let Some(other) = self
            .var_scopes
            .iter()
            .flat_map(|s| s.iter())
            .find_map(|(k, b)| {
                matches!(b, Binding::Local { air_name, .. } if air_name.starts_with(&prefix))
                    .then_some(k)
            })
        {
            return Err(Error::at(
                span,
                format!(
                    "local slot {n} already holds '{}', which is still in scope",
                    other.split('#').next().unwrap_or(other)
                ),
            ));
        }
        if self.checking_discarded == 0 {
            self.slot_span.get_or_insert(span);
            if self.pin_span.is_some() {
                return Err(Error::at(span, SLOT_WITH_PIN));
            }
        }
        let air_name = self.fresh(&format!("slot{n}_{name}"));
        if self.checking_discarded == 0 {
            self.pinned_locals.insert(air_name.clone(), usize::from(n));
        }
        Ok(air_name)
    }
}
