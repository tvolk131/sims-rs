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
            reassigns_stack_obj: false,
            raw_moves_cursor: false,
            edge_node_result: None,
            borrowing_place: false,
            sets_stack_obj: false,
            used_saved_stack_obj: false,
            value_return: None,
            em: mir::Emitter::new(1),
            raw_blocks: 0,
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
            local_origins: BTreeMap::new(),
            stmt_span: None,
            seen_decls: BTreeSet::new(),
            inline_call_sites: Vec::new(),
            dup_decl: None,
            keep_layout: false,
            pinned_layout: false,
            narrow_callees: BTreeSet::new(),
            option_aliases: BTreeSet::new(),
            fit_aliases: BTreeSet::new(),
            wide_call: None,
            staged_in_place: false,
            temp_loops: 0,
            fixed_entry: false,
            param_words_used: 0,
            frame_locals: None,
            frame_format: None,
            event_local: None,
            sleep_span: None,
            stmt_marks: Vec::new(),
            inherited_entry_save: false,
            inherit_tree: false,
            tree_display: None,
            object_reachable: false,
            raw_reserved: Vec::new(),
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

    pub(super) fn finish(mut self) -> Result<String> {
        // `#[bhav(keep_layout)]` keeps the backend pass off as a `raw` block that ends the body
        // and keeps its layout would: lowering itself is unchanged.
        self.keep_layout |= self.pinned_layout;
        let formats_text = self.raw_formats_text || mir::body_formats_text(self.em.text());
        if formats_text {
            self.env.saw_text_tree.set(true);
        }
        // The frame words the tree's text may read: none unless it formats text.
        // A tree of an object any of whose trees may read global 34 keeps main's rule (every
        // text tree reads): dropping the entry save removes an Expression, whose scratch-pointer
        // write that read depends on, and a call carries the pointer between trees. (The pinned
        // slot's placeholder is not `.air` yet; it becomes word 0 below.)
        // So does a tree that may run as an object's bottom frame (an object-self tree with no
        // arguments, as an object's or a variant's `main` is, or the inherit check's Sim pass
        // over one): Create Object with flag 0x10 or 0x02 writes that frame's word 0, so which
        // local lives there is observable.
        // A tree with a pinned `push_interaction` slot takes main's rules too, so it pins exactly
        // where main does (`mir::finish_body`).
        let bottom_frame = (matches!(self.tree_ctx, TreeContext::ObjectSelf)
            || self.inherited_entry_save)
            && self.frame_args == 0;
        let body = replace_outside_strings(self.em.text(), PIN_SLOT, "0");
        let body_reads_global_34 = mir::may_read_global_34(&body, &self.raw_consts);
        if body_reads_global_34 {
            self.env.saw_global_34.set(true);
        }
        // A bottom frame with a local that may be read before it is written takes main's rules
        // as well: dropping a store, or the local it initializes, shifts which local lives in
        // word 0, and that read sees what Create Object wrote there.
        let bottom_reads_unwritten = bottom_frame
            && !mir::parse_body(&body).is_some_and(|nodes| {
                mir::locals_defined_before_reads(&nodes, mir::DrawRanges::Literal)
            });
        let main_rules = self.env.reads_global_34
            || body_reads_global_34
            || bottom_reads_unwritten
            || self.pin_span.is_some();
        let (text_reads, text_reads_besides_me) = if formats_text && (bottom_frame || main_rules) {
            (mir::TableLocals::Unknown, mir::TableLocals::Unknown)
        } else if formats_text {
            (
                self.env.text_reads.clone(),
                self.env.text_reads_besides_me.clone(),
            )
        } else {
            (
                mir::TableLocals::Known(BTreeSet::new()),
                mir::TableLocals::Known(BTreeSet::new()),
            )
        };
        if let Some((span, name)) = &self.dup_decl
            && mir::reads_a_word(&text_reads)
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
        // A call this tree passes inline arguments to without a proven narrow frame may read
        // the nodes after it (`wide_call`), so no option copy goes.
        if self.wide_call.is_some() {
            self.option_aliases.clear();
        }
        let aliased = std::cell::Cell::new(false);
        let facts = mir::FrameFacts {
            main_rules,
            fixed_frame: self.frame_locals.is_some() || self.frame_format.is_some(),
            text_reads,
            text_reads_besides_me,
            keeps_exit_stack_obj: self.sets_stack_obj,
            keep_layout: self.keep_layout,
            has_self_obj_local: self.has_self_obj_local,
            pinned: self.pin_span.is_some(),
            raw_consts: &self.raw_consts,
            raw_declared: &self.raw_declared,
            has_slots: self.slot_span.is_some(),
            pinned_locals: &self.pinned_locals,
            local_origins: &self.local_origins,
            min_locals: self.frame_floor().unwrap_or(0),
            option_aliases: &self.option_aliases,
            aliased: &aliased,
        };
        let body = self.em.into_text();
        let mut out = if self.used_saved_stack_obj {
            let save = mir::StackObjOp::Save(mir::SAVED_STACK_OBJ).text();
            format!("    {save}\n{body}")
        } else {
            body
        };
        // The body reads the entry object when an `.air` token names it (a comment does not). A
        // source constant of that name, which a raw block reads as a constant, is not the entry
        // object, and neither is a raw block's own local or label of that name. The inherit
        // check's Sim lowering emits no entry save of its own (`track_stack_obj`), only this one.
        let declares_self_obj = self.has_self_obj_local && !self.inherited_entry_save;
        if self.inherited_entry_save
            && !self.raw_consts.contains_key(mir::SELF_OBJ)
            && !self
                .raw_reserved
                .iter()
                .any(|(name, _)| *name == mir::SELF_OBJ)
            && names_token(&out, mir::SELF_OBJ)
        {
            let save = mir::StackObjOp::Save(mir::SELF_OBJ).text();
            out = format!("    {save}\n{out}");
        }
        // A raw block may declare an entry-object local of its own only where lowering declares
        // none of that name.
        for (name, span) in &self.raw_reserved {
            let declared = match *name {
                mir::SELF_OBJ => declares_self_obj,
                _ => self.used_saved_stack_obj,
            };
            if declared {
                return Err(Error::at(
                    *span,
                    format!(
                        "'{name}' is the compiler's entry object in this tree: a raw block may \
                         read it but not declare its own"
                    ),
                ));
            }
        }
        if self.pin_span.is_some() {
            // A tree with a pinned `push_interaction` has no raw block (that is an error), so
            // nothing else names an explicit local: the pin takes slot 0, and `.air` allocates
            // the named locals after it.
            out = replace_outside_strings(&out, PIN_SLOT, "0");
        }
        let mut finished = mir::finish_body(out.clone(), &facts);
        // A fixed frame (`locals = N`) too small for the option copies would fail to build:
        // where the copies' reads allow it (`mir::alias_option_temps`), every binding reads its
        // temp in place instead, past the object-wide guards `copy_staging` follows (another
        // tree's frame-word text, or a kept layout elsewhere in the object), since the form
        // that copies every binding does not build, so no bytes or frame residue can differ. Only when the copies overflow the frame (as `.air` counts its words)
        // and the result fits it, so a source that builds keeps its bytes. Not when this
        // tree's own text may name a frame word (a copy may fill the word it shows), with a
        // kept layout, a wide inline call (it reads the nodes after it), or main's rules (the
        // copy is an Expression, whose scratch-pointer write global 34 reads).
        let words = |body: &str| mir::frame_words(body, facts.min_locals);
        if let Some(n) = self.frame_locals
            && !self.fit_aliases.is_empty()
            && !self.keep_layout
            && self.wide_call.is_none()
            && !facts.main_rules
            && !mir::reads_a_word(&facts.text_reads)
            && words(&finished).is_some_and(|w| w > usize::from(n))
        {
            let fit = std::cell::Cell::new(false);
            let fit_facts = mir::FrameFacts {
                option_aliases: &self.fit_aliases,
                aliased: &fit,
                ..facts.clone()
            };
            let alt = mir::finish_body(out, &fit_facts);
            if fit.get() && words(&alt).is_some_and(|w| w <= usize::from(n)) {
                finished = alt;
            }
        }
        // An option binding read in place dropped its copy, which changes nodes and frame
        // words as a staging in place does (the pass reports whether it read any in place).
        let aliased = aliased.get();
        // A call staged in place has fewer nodes and frame words than the copying form. Where
        // either is observable, the object is lowered again with the copying form: a kept
        // layout (numbered raw branches), main's rules (global 34, a bottom frame read before
        // it is written, a pin), text that may name frame words (`$Local:N`), and a bottom
        // frame left with no word at all (Create Object with flag 0x10 writes word 0).
        if self.staged_in_place || aliased {
            self.env.saw_in_place.set(true);
            let empty_bottom = bottom_frame
                && self.frame_locals.is_none()
                && facts.min_locals == 0
                && !finished.lines().any(|l| l.trim_start().starts_with("let "));
            if self.keep_layout
                || facts.main_rules
                || mir::reads_a_word(&facts.text_reads)
                || empty_bottom
            {
                self.env.saw_layout_staging.set(true);
            }
        }
        if self.keep_layout {
            self.env.saw_keep_layout.set(true);
        }
        Ok(finished)
    }

    /// A lifecycle tree's `#[frame(...)]`: `param[0..args]` become addressable and `locals` bounds
    /// the frame, as on a standalone `#[bhav] fn`.
    pub(super) fn set_lifecycle_frame(&mut self, frame: &TreeFrame) {
        self.frame_locals = frame.locals;
        self.pinned_layout = frame.keep_layout;
        self.frame_format = frame.format;
        self.frame_args = frame.args.map_or(0, usize::from);
        self.fixed_entry = frame.kind.is_some();
    }

    /// Adds `min_locals: N` to a tree header when an `#[slot(N - 1)]` word must be in the frame:
    /// the tree may never name it again (an uninitialized slot a raw primitive or text reads by
    /// number), and `.air` sizes the frame by the words the body names.
    /// The header's `#[frame(locals = K)]` sets the frame size in its place, with no floor
    /// (`write_framed_tree`).
    pub(super) fn push_frame_floor(&self, header: &mut Vec<String>) {
        if let Some(floor) = self.frame_floor() {
            header.push(format!("min_locals: {floor}"));
        }
    }

    /// The `min_locals` floor [`Self::push_frame_floor`] puts in the header: the highest
    /// `#[slot]` word plus one, unless the tree has `#[frame(locals = K)]`.
    pub(super) fn frame_floor(&self) -> Option<usize> {
        if self.frame_locals.is_some() {
            return None;
        }
        self.pinned_locals.values().max().map(|top| top + 1)
    }

    pub(super) fn fresh(&mut self, prefix: &str) -> String {
        let id = self.next_id;
        self.next_id += 1;
        let scope = if prefix.starts_with("v_") && !self.inline_call_stack.is_empty() {
            "inline_"
        } else {
            ""
        };
        let name = format!("__{scope}{prefix}_{id}");
        // An inlined helper's source local: copies of one declaration (the same helper and
        // statement) get the same origin.
        if !scope.is_empty()
            && let (Some(helper), Some(at)) = (self.inline_call_stack.last(), self.stmt_span)
        {
            let origin = format!("{helper}::{prefix}@{}:{}", at.line, at.column);
            self.local_origins.insert(name.clone(), origin);
        }
        name
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

    /// [`LOCAL_WRITING_BUILTINS`] less the names a tree or helper shadows in `module` (object
    /// trees and local helpers only where `allow_local`), as a helper body there resolves them.
    pub(super) fn local_writing_builtins_in(
        &self,
        module: &str,
        allow_local: bool,
    ) -> Vec<&'static str> {
        LOCAL_WRITING_BUILTINS
            .iter()
            .copied()
            .filter(|b| {
                !(allow_local && self.env.trees.contains_key(*b))
                    && self.env.lookup_fn(module, b, allow_local).is_none()
            })
            .collect()
    }

    pub(super) fn bind_var(&mut self, name: String, binding: Binding) {
        if let Some(top) = self.var_scopes.last_mut()
            && let Some(old) = top.insert(name.clone(), binding)
            && matches!(&old, Binding::Local { air_name, .. } if air_name.starts_with("__slot"))
        {
            // A `defer` registered before the shadowing still uses the old `#[slot]` variable,
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

    /// The running object's handle for its own cells and fields: the Sim handle, or the frame's
    /// `me` where it may be a Sim (in an interaction the Sim is `me`; in a `context = inherit`
    /// tree `me` is whichever object runs it). Both lower to `my.` cells. In an object's own
    /// tree `me` is the object, which has no Sim fields: only its raw cells
    /// ([`Self::is_frame_me`]) apply there, and a `bcon me` table keeps its name.
    pub(super) fn is_my_ident(&self, name: &str) -> bool {
        self.is_sim_ident(name)
            || (self.is_frame_me(name)
                && (self.inherit_tree || matches!(self.tree_ctx, TreeContext::SimOnObject { .. }))
                && !self
                    .env
                    .is_bcon_table_name(&self.owner_module, name, self.in_object_scope))
    }

    /// `self` the running tree's object: [`Self::is_self_ident`], unless a local or parameter
    /// named `self` (a `for self in ..` binding) shadows it.
    pub(super) fn is_self_keyword(&self, name: &str) -> bool {
        self.is_self_ident(name)
            && matches!(self.lookup_var(name), None | Some(Binding::SelfHandle))
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
    /// the named object-data words (`flags`, `facing`, `value`, ...), and (untyped handles
    /// only) the person words `age`, `priority`, `is_visitor`, `kind`, and the named ones.
    pub(super) fn handle_field_cell(field: &str, maybe_sim: bool) -> Option<(String, ValueType)> {
        let cell = |c: &str, ty| Some((c.to_owned(), ty));
        match field {
            "hand" => cell("stack.slot[0]", ValueType::ObjectId),
            "room" => cell("stack.room", ValueType::I16),
            other if object_data_word(other).is_some() => object_data_word(other)
                .map(|w| (format!("stack.data[{}]", w.index), w.value_type())),
            _ if !maybe_sim => None,
            "age" => cell("stack.person[58]", ValueType::I16),
            "priority" => cell("stack.person.priority", ValueType::I16),
            "is_visitor" => cell("stack.person[32]", ValueType::Bool),
            "visitor_kind" => cell("stack.person[32]", ValueType::I16),
            "kind" => cell("stack.person[65]", ValueType::I16),
            other => sim_person_word(other)
                .map(|w| (format!("stack.person[{}]", w.index), w.value_type())),
        }
    }

    /// `temp[k]` for a constant `k` in 0..8, when `temp` is the frame's own temps: the temp a
    /// temp-indexed scope (27, 30, 31) names by number, so the cell needs no setup.
    pub(super) fn temp_word_index(&self, index: &V2Expr) -> Option<i16> {
        let V2Expr::Index { base, index, .. } = index else {
            return None;
        };
        if self.borrowing_place {
            return None;
        }
        if !matches!(base.as_ref(), V2Expr::Ident(_, n) if n == "temp")
            || self.lookup_var("temp").is_some()
        {
            return None;
        }
        self.eval_const_i16(index)
            .ok()
            .filter(|k| (0..8).contains(k))
    }

    /// `obj.sprite_flags[i]`: scope 27 reads the stack object's dynamic-sprite flag whose index
    /// is in the temp the operand names. Returns the setup line, if any, and the cell:
    /// `sprite_flags[temp[k]]` is `scope(27, k)` with no setup; any other index is set into
    /// `temp[0]` first.
    pub(super) fn sprite_flag_cell(&mut self, index: &V2Expr) -> Result<(Option<String>, String)> {
        if let Some(k) = self.temp_word_index(index) {
            return Ok((None, format!("scope(27, {k})")));
        }
        let idx_air = if let Ok(v) = self.eval_const_i16(index) {
            v.to_string()
        } else if self.is_pure_single_operand(index) {
            let (air, ty) = self.lower_value_expr(index)?;
            if !i16_like(&ty) {
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
        Ok((
            Some(format!("temp[0] := {idx_air};")),
            "scope(27, 0)".to_owned(),
        ))
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
            | V2Stmt::IfLetSome { span, name, .. }
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
        // `let _ = ..` declares only when it falls back to a hidden local
        // (`lower_hidden_discard`).
        if matches!(stmt, V2Stmt::Let { .. }) && name == "_" {
            return;
        }
        self.note_decl_named(span, name);
    }

    /// Records a declaration at `span` (see [`Self::note_decl`]).
    pub(super) fn note_decl_named(&mut self, span: Span, name: &str) {
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
    /// `#[slot(N)] let`, which the optimizer never merges and `opt::assign_frame_slots` turns into
    /// `local[N]`. A slot belongs to the tree's frame, so `#[slot]` is refused in an inline helper
    /// (each caller's frame differs), next to a pinned `push_interaction` object slot, and while
    /// another `#[slot]` variable of the same slot is in scope.
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
                "'#[slot(N)] let' names a frame word of the tree, so it is not allowed in an inlined helper",
            ));
        }
        if let Some(locals) = self.frame_locals
            && u16::from(n) >= locals
        {
            return Err(Error::at(
                span,
                format!(
                    "'#[slot({n})]' names local[{n}], but '#[frame(locals = {locals})]' gives the tree {locals} local word(s)"
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

/// Whether an `.air` identifier token of `body` is `name` (comments and strings are not tokens,
/// and neither is a part of a qualified constant's name such as `name.tuning`, since a local has
/// no fields, nor the tree a `call name` runs). A body that does not lex counts as naming it.
fn names_token(body: &str, name: &str) -> bool {
    use crate::lexer::TokenKind as T;
    crate::lexer::tokenize(body).map_or(true, |tokens| {
        let dot = |i: Option<usize>| {
            i.and_then(|i| tokens.get(i))
                .is_some_and(|t| matches!(t.kind, T::Dot))
        };
        let ident = |i: Option<usize>, word: &str| {
            i.and_then(|i| tokens.get(i))
                .is_some_and(|t| matches!(&t.kind, T::Ident(k) if k == word))
        };
        // The tree `call x`, `subroutine x` or `subroutine(x ..)` names.
        let after_call = |i: usize| {
            ident(i.checked_sub(1), "call")
                || ident(i.checked_sub(1), "subroutine")
                || (i
                    .checked_sub(1)
                    .and_then(|i| tokens.get(i))
                    .is_some_and(|t| matches!(t.kind, T::LParen))
                    && ident(i.checked_sub(2), "subroutine"))
        };
        tokens.iter().enumerate().any(|(i, t)| {
            matches!(&t.kind, T::Ident(n) if n == name)
                && !dot(i.checked_sub(1))
                && !dot(Some(i + 1))
                && !after_call(i)
        })
    })
}

/// The type of a named engine word.
#[derive(Clone, Copy)]
pub(super) enum EngineWordTy {
    I16,
    ObjectId,
    /// A bitflags type by key (`std::flags`, or the untyped engine flags word).
    Flags(&'static str),
    /// An engine enum by key (`std::engine`).
    Enum(&'static str),
}

/// A named object-data or person-data word.
#[derive(Clone, Copy)]
pub(super) struct EngineWord {
    pub(super) index: u8,
    pub(super) ty: EngineWordTy,
    /// Whether a script may store into it. The engine traps a store of a different value
    /// into object data words 2, 3 and 11, so those are read-only by name.
    pub(super) writable: bool,
}

impl EngineWord {
    pub(super) fn value_type(self) -> ValueType {
        match self.ty {
            EngineWordTy::I16 => ValueType::I16,
            EngineWordTy::ObjectId => ValueType::ObjectId,
            EngineWordTy::Flags(key) => ValueType::Bitflags(key.to_owned()),
            EngineWordTy::Enum(key) => ValueType::Enum(key.to_owned()),
        }
    }
}

const fn word(index: u8, ty: EngineWordTy) -> EngineWord {
    EngineWord {
        index,
        ty,
        writable: true,
    }
}

const DIRECTION_KEY: &str = "enum::std::engine::Direction";
const PERSON_KIND_KEY: &str = "enum::std::engine::PersonKind";

/// Object-data words every object (Sims included) exposes by name: `self.broken` is
/// `self.data[15]`, read and written exactly like it. The indices and meanings are the
/// engine's (sims-vm `placement.rs`, `placement/find.rs`, `buy.rs`, `daily.rs`).
const OBJECT_DATA_WORDS: &[(&str, EngineWord)] = &[
    // Facing, 0..7 in eighths of a turn clockwise from north.
    ("facing", word(1, EngineWordTy::Enum(DIRECTION_KEY))),
    // The object whose slot holds this one (read-only).
    (
        "container",
        EngineWord {
            index: 2,
            ty: EngineWordTy::ObjectId,
            writable: false,
        },
    ),
    // The levels the object may stand at.
    ("allowed_heights", word(4, EngineWordTy::Flags(HEIGHT_KEY))),
    // Which sides have a wall, relative to the facing (written by the engine).
    (
        "wall_adjacency",
        word(
            5,
            EngineWordTy::Flags("bitflags::std::flags::WallAdjacency"),
        ),
    ),
    // How much the object adds to (or, negative, takes from) the score of the room it stands
    // in; outdoors only a negative value counts (sims-sim `rooms.rs` `count_object`).
    ("room_impact", word(7, EngineWordTy::I16)),
    // Movement and intersection flags; any bitflags type converts (`std::flags::ObjectFlags`).
    ("flags", word(8, EngineWordTy::Flags(ANY_BITFLAGS))),
    // The wall-placement rules: a lenient engine `bitflags` word, like `wall_adjacency`. It
    // takes flag methods (`self.wall_placement.set(WallPlacement::NoCorner)`) and still every
    // numeric use (`self.wall_placement flag_set 5`, `self.wall_placement == 0`).
    (
        "wall_placement",
        word(
            13,
            EngineWordTy::Flags("bitflags::std::flags::WallPlacement"),
        ),
    ),
    // Broken (a broken object sells for half).
    ("broken", word(15, EngineWordTy::I16)),
    // How a light source lights the lot: 1 casts a local light, which the room count marks on
    // the tiles around the object (its floor pools) and turns into a light that falls on the
    // people in its room; picking the object up clears those marks. The original's light
    // sources set it to 1 in their init and never change it (switching a lamp off sets `light`
    // to 0). The engine compares it with 1 and hands it to its light-map services as their
    // mode (sims-render `lot.rs` `gives_light`; legacy `lot_environment/objects.rs`,
    // `lot_placement.rs`).
    ("light_mode", word(16, EngineWordTy::I16)),
    // Weight, and the weight a container may hold.
    ("weight", word(27, EngineWordTy::I16)),
    ("support_strength", word(28, EngineWordTy::I16)),
    // The second flags word.
    (
        "flags2",
        word(
            40,
            EngineWordTy::Flags("bitflags::std::flags::ObjectFlags2"),
        ),
    ),
    // The current (depreciated) value.
    ("value", word(41, EngineWordTy::I16)),
    // What the object may stand on.
    ("placement", word(42, EngineWordTy::Flags(PLACEMENT_KEY))),
    // Buy-mode flags.
    (
        "buy_flags",
        word(43, EngineWordTy::Flags("bitflags::std::flags::BuyFlags")),
    ),
    // The light the object gives. When `flags2` holds `LightSource` and the object stands on
    // the lot, the room count adds it to its room's lamp light (a window's to its window
    // light), and a lamp that gives light is drawn lit (sims-sim `rooms.rs` `count_object`,
    // sims-render `lot.rs` `gives_light`).
    ("light", word(51, EngineWordTy::I16)),
    ("category", word(59, EngineWordTy::I16)),
];

/// Person-data words a Sim exposes by name: `sim.job_level` is `sim.person[57]`, read and
/// written exactly like it. The indices are the engine's (sims-vm `outfits.rs`, `caf.rs`,
/// `autonomy.rs`, `animate.rs`, `people.rs`, `travel.rs`).
const SIM_PERSON_WORDS: &[(&str, EngineWord)] = &[
    ("posture", word(0, EngineWordTy::I16)),
    ("pets_outside", word(34, EngineWordTy::I16)),
    ("job_type", word(56, EngineWordTy::I16)),
    // A child's school grade.
    ("job_level", word(57, EngineWordTy::I16)),
    ("family", word(61, EngineWordTy::I16)),
    // Sex and species.
    ("gender_kind", word(65, EngineWordTy::Enum(PERSON_KIND_KEY))),
    // A date's link: the companion's word names the leader, the leader's names the companion
    // negated, 0 none. A store queues the date indicator.
    ("date_partner", word(77, EngineWordTy::I16)),
    // Fame, in half-stars.
    ("fame", word(81, EngineWordTy::I16)),
];

/// Read-only words of an object's definition, read through the stack object (`sims-vm`
/// `memory.rs` scope 21, the stack object's definition as words; the word meanings are the
/// engine's, `factory.rs` `word`): `object_type` is word 9 (2 for a person) and `sub_index`
/// word 11, a tile's place in its group (high byte, signed, its first offset; low byte its
/// second; -1 the group's catalog master).
pub(super) fn definition_field(name: &str) -> Option<u8> {
    match name {
        "object_type" => Some(9),
        "sub_index" => Some(11),
        _ => None,
    }
}

/// `h.neighbor.<name>`: a read-only cell of the neighborhood record the stack object names, as
/// `set_to_next(neighbor)` leaves it (`sims-vm` `memory.rs`).
///
/// - Scope 24, the record's fields, each 0 when the stack object names no record:
///   `person_on_lot` (0, the ID of the record's person on this lot, or 0), `in_selected_family`
///   (1), `age` (2, the record's person word 58), `friend_count` (5), `family_house` (6, the
///   house of the family that holds the record) and `family_friend_count` (9, the friend count
///   of the family the record's word 61 names). Fields 3 and 4 read 0, and 7 and 8 read
///   bits 0 and 1 of the family's flags; `neighbor.field[i]` reads any field.
/// - Scope 32, the record's person data, by the names a Sim's person words have
///   (`neighbor.family` is word 61) and as `neighbor.person[i]`. Naming no record is native
///   error 0x37.
pub(super) fn neighbor_cell(name: &str) -> Option<(String, ValueType)> {
    let field = |i: u8, ty| Some((format!("scope(24, {i})"), ty));
    match name {
        "person_on_lot" => field(0, ValueType::ObjectId),
        "in_selected_family" => field(1, ValueType::Bool),
        "age" => field(2, ValueType::I16),
        "friend_count" => field(5, ValueType::I16),
        "family_house" => field(6, ValueType::I16),
        "family_friend_count" => field(9, ValueType::I16),
        other => {
            sim_person_word(other).map(|w| (format!("scope(32, {})", w.index), w.value_type()))
        }
    }
}

/// `h.neighbor.field[i]` (scope 24, fields 0..=9) or `h.neighbor.person[i]` (scope 32, words
/// 0..=255).
pub(super) fn neighbor_indexed_scope(field: &str) -> Option<(u8, i64)> {
    match field {
        "field" => Some((24, 9)),
        "person" => Some((32, 255)),
        _ => None,
    }
}

/// The object-data word of a named object field (`broken` → 15).
pub(super) fn object_data_word(name: &str) -> Option<EngineWord> {
    OBJECT_DATA_WORDS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, w)| w)
}

/// The person-data word of a named Sim field (`job_level` → 57).
pub(super) fn sim_person_word(name: &str) -> Option<EngineWord> {
    SIM_PERSON_WORDS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, w)| w)
}
