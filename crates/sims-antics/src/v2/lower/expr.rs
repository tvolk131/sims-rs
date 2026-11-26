//! Value expressions: arithmetic in place, the value-expression lowerer, job data, operand
//! snapshots and distances.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// Returns `true` iff `expr` contains a division or remainder whose divisor is not a known
    /// non-zero compile-time constant (and therefore could trap partway through evaluation).
    pub(super) fn expr_may_trap_arith(&self, expr: &V2Expr) -> bool {
        match expr {
            V2Expr::Binary { lhs, op, rhs, .. } => {
                if matches!(op, V2BinOp::Div | V2BinOp::Mod)
                    && !self.eval_const_i16(rhs).is_ok_and(|v| v != 0)
                {
                    return true;
                }
                self.expr_may_trap_arith(lhs) || self.expr_may_trap_arith(rhs)
            }
            V2Expr::Unary { inner, .. }
            | V2Expr::Deref { inner, .. }
            | V2Expr::MutRef { inner, .. } => self.expr_may_trap_arith(inner),
            V2Expr::Index { base, index, .. } => {
                self.expr_may_trap_arith(base) || self.expr_may_trap_arith(index)
            }
            V2Expr::Field { base, .. } => match self.bcon_field_ref(expr) {
                // `extern bcon` entries are read at runtime and trap when the chunk/entry is
                // missing; private tables are compile-time constants.
                Some(Ok((_, Some(_)))) => false,
                Some(_) => true,
                None => self.expr_may_trap_arith(base),
            },
            V2Expr::Tuple(_, elems) => elems.iter().any(|e| self.expr_may_trap_arith(e)),
            V2Expr::Call { .. }
            | V2Expr::MethodCall { .. }
            | V2Expr::Try { .. }
            | V2Expr::Match { .. }
            | V2Expr::If { .. }
            | V2Expr::Block { .. } => true,
            V2Expr::Int(..)
            | V2Expr::Bool(..)
            | V2Expr::Str(..)
            | V2Expr::Ident(..)
            | V2Expr::Path(..) => {
                // `Table::entry` reads like `Table.entry` (an `extern bcon` read may trap).
                matches!(self.bcon_field_ref(expr), Some(r) if !matches!(r, Ok((_, Some(_)))))
            }
        }
    }

    /// Returns `true` iff reading `expr` may fault at runtime (`sims-vm` `memory.rs`): a read
    /// through the stack object, a foreign handle, a neighbor or a definition, `self` through a
    /// cursor the tree may move, an index outside what every such object has, or a person word
    /// of a runner that may not be a Sim. Locals, temps, constants and the cells
    /// [`Self::safe_cell_air`] accepts never fault.
    pub(super) fn expr_may_fault_read(&self, expr: &V2Expr) -> bool {
        if self.expr_may_trap_arith(expr) {
            return true;
        }
        match expr {
            V2Expr::Int(..)
            | V2Expr::Bool(..)
            | V2Expr::Str(..)
            | V2Expr::Path(..)
            | V2Expr::Ident(..) => false,
            V2Expr::Binary { lhs, rhs, .. } => {
                self.expr_may_fault_read(lhs) || self.expr_may_fault_read(rhs)
            }
            V2Expr::Unary { inner, .. } => self.expr_may_fault_read(inner),
            V2Expr::Tuple(_, elems) => elems.iter().any(|e| self.expr_may_fault_read(e)),
            V2Expr::Field { .. } | V2Expr::Index { .. } => {
                if matches!(self.bcon_field_ref(expr), Some(Ok((_, Some(_))))) {
                    return false;
                }
                !self
                    .known_cell_air(expr)
                    .is_some_and(|air| self.safe_cell_air(&air))
            }
            _ => true,
        }
    }

    /// The `.air` cell a plain field or constant-index read names, when it is one of the running
    /// object's, the Sim's, a temp, a parameter or a global (`None` for anything else).
    fn known_cell_air(&self, expr: &V2Expr) -> Option<String> {
        match expr {
            V2Expr::Field { base, field, .. } => match base.as_ref() {
                V2Expr::Ident(_, b) if self.is_self_ident(b) => {
                    let p = self.self_prefix();
                    if let Some(st) = self.env.states.get(field) {
                        return Some(format!("{p}.attr[{}]", st.attr_index));
                    }
                    let w = match field.as_str() {
                        "graphic" => 0,
                        "prep_score" => 31,
                        "cook_score" => 32,
                        "in_use" => 25,
                        "id" => 11,
                        "room" => 29,
                        other => object_data_word(other)?.index,
                    };
                    Some(format!("{p}.data[{w}]"))
                }
                V2Expr::Ident(_, b) if self.is_my_ident(b) => Some(match field.as_str() {
                    "id" => "my.data[11]".to_owned(),
                    "priority" => "my.person[33]".to_owned(),
                    "room" => "my.data[29]".to_owned(),
                    "age" => "my.person[58]".to_owned(),
                    "visitor_kind" => "my.person[32]".to_owned(),
                    other => match (sim_person_word(other), object_data_word(other)) {
                        (Some(w), _) => format!("my.person[{}]", w.index),
                        (None, Some(w)) => format!("my.data[{}]", w.index),
                        (None, None) => return None,
                    },
                }),
                V2Expr::Field {
                    base: root,
                    field: group,
                    ..
                } if matches!(root.as_ref(), V2Expr::Ident(_, r) if self.is_my_ident(r))
                    && matches!(group.as_str(), "motive" | "skill") =>
                {
                    let air = canonical_air_cell(&format!("my.{group}.{field}"));
                    (air != format!("my.{group}.{field}")).then_some(air)
                }
                _ => None,
            },
            V2Expr::Index { base, index, .. } => {
                let i = self.eval_const_i16(index).ok()?;
                match base.as_ref() {
                    V2Expr::Ident(_, b)
                        if matches!(b.as_str(), "temp" | "global" | "param")
                            && self.lookup_var(b).is_none() =>
                    {
                        Some(format!("{b}[{i}]"))
                    }
                    V2Expr::Field {
                        base: root, field, ..
                    } if matches!(field.as_str(), "attr" | "data" | "person") => {
                        let V2Expr::Ident(_, r) = root.as_ref() else {
                            return None;
                        };
                        let p = if self.is_self_ident(r) {
                            self.self_prefix()
                        } else if self.is_my_ident(r) {
                            "my"
                        } else {
                            return None;
                        };
                        Some(format!("{p}.{field}[{i}]"))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Whether reading the `.air` cell `air` can never fault (`sims-vm` `memory.rs`):
    /// - `temp[0..8]`, `param[i]` within the frame's arguments, `global[0..32]` (older saves
    ///   keep 32);
    /// - the running object's attributes it declares, and its data words below 64 (the
    ///   smallest object record); through `stack.` only in a Sim tree that never moves the
    ///   cursor, where `stack` is that object;
    /// - the runner's data words below 64;
    /// - the Sim's person words below 80 (older saves keep 80) and motives, only in a Sim tree,
    ///   whose runner is a Sim.
    pub(super) fn safe_cell_air(&self, air: &str) -> bool {
        let air = canonical_air_cell(air);
        let Some((head, rest)) = air.split_once('[') else {
            return false;
        };
        let Some(i) = rest.strip_suffix(']').and_then(|n| n.parse::<usize>().ok()) else {
            return false;
        };
        // An inherit tree's or a functional's runner may be an object.
        let sim_runner = matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
            && !self.inherit_tree
            && !self.object_reachable;
        let cursor_is_self = sim_runner && !(self.explicit_stack_obj || self.cursor_may_be_moved());
        let own = |prefix: &str| prefix == self.self_prefix() && (prefix == "my" || cursor_is_self);
        let declared_attr = self.env.states.values().any(|s| s.attr_index == i);
        match head {
            "temp" => i < 8,
            "param" => i < self.frame_args,
            "global" => i < 32,
            "my.attr" => own("my") && declared_attr,
            "stack.attr" => own("stack") && declared_attr,
            "my.data" => i < 64,
            "stack.data" => own("stack") && i < 64,
            "my.person" => sim_runner && i < 80,
            "my.motive" => sim_runner && i < 16,
            _ => false,
        }
    }

    /// Returns `true` iff `expr` can be lowered directly in-place into `dst_air` without aliasing,
    /// partial writes on trap, observable VM write side effects, or premature motive/skill clamping!
    pub(super) fn can_lower_arith_in_place(
        &self,
        dst_air: &str,
        expr: &V2Expr,
        is_fresh_local: bool,
    ) -> bool {
        let canon_dst = canonical_air_cell(dst_air);
        if canon_dst.starts_with("my.motive")
            || canon_dst.starts_with("stack.motive")
            || canon_dst.starts_with("my.person")
            || canon_dst.starts_with("stack.person")
            || canon_dst.starts_with("my.data[")
            || canon_dst.starts_with("stack.data[")
            || canon_dst.starts_with("global[")
            // A temp-indexed or other scope cell: its word is not known here, so it may alias
            // an operand or be a word with write side effects.
            || canon_dst.starts_with("scope(")
        {
            return false;
        }
        if !is_fresh_local && self.expr_may_trap_arith(expr) {
            return false;
        }
        let (base, ops) = flatten_arith_chain(expr);
        // In place, `dst := base; dst op= rhs; ...` has already written `dst` when a later operand
        // read faults (an invalid stack object, say), leaving the observable cell half-updated.
        if !is_fresh_local {
            let neg_inner = match base {
                V2Expr::Unary {
                    op: V2UnaryOp::Neg,
                    inner,
                    ..
                } => Some(inner.as_ref()),
                _ => None,
            };
            if neg_inner.is_some_and(|e| self.expr_may_fault_read(e))
                || ops.iter().any(|(_, rhs)| self.expr_may_fault_read(rhs))
            {
                return false;
            }
        }
        if ops.is_empty() {
            if let V2Expr::Unary {
                op: V2UnaryOp::Neg,
                inner,
                ..
            } = base
            {
                return !self.expr_may_read_or_clobber_air(inner, &canon_dst);
            }
            return true;
        }
        if ops
            .iter()
            .any(|(_, rhs)| self.expr_may_read_or_clobber_air(rhs, &canon_dst))
        {
            return false;
        }
        // If `base` itself is unary `-inner`, emitting `dst := 0; dst -= inner;` would overwrite
        // `dst` before reading `inner` if `inner` reads `dst`!
        if let V2Expr::Unary {
            op: V2UnaryOp::Neg,
            inner,
            ..
        } = base
        {
            return !self.expr_may_read_or_clobber_air(inner, &canon_dst);
        }
        true
    }

    /// Conservatively checks whether evaluating `expr` could read or mutate `canon_target`
    /// (already canonicalized via [`canonical_air_cell`]).
    pub(super) fn expr_may_read_or_clobber_air(&self, expr: &V2Expr, canon_target: &str) -> bool {
        match expr {
            V2Expr::Int(_, _) | V2Expr::Bool(_, _) | V2Expr::Str(_, _) | V2Expr::Path(_, _) => {
                false
            }
            V2Expr::Ident(_, name) => match self.lookup_var(name) {
                Some(Binding::Local { air_name, .. }) => {
                    canonical_air_cell(air_name) == canon_target
                }
                Some(Binding::BhavParam { index, .. }) => format!("param[{index}]") == canon_target,
                Some(Binding::MutRef {
                    target: LValue::Direct { air, .. },
                }) => canonical_air_cell(air) == canon_target,
                Some(Binding::MutRef { .. }) => true,
                Some(Binding::ConstI16 { air, .. } | Binding::Operand { air, .. }) => {
                    canonical_air_cell(air) == canon_target
                }
                _ => false,
            },
            V2Expr::Deref { inner, .. } => {
                if let V2Expr::Ident(_, pname) = inner.as_ref() {
                    match self.lookup_var(pname) {
                        Some(Binding::MutRef {
                            target: LValue::Direct { air, .. },
                        }) => canonical_air_cell(air) == canon_target,
                        _ => true,
                    }
                } else {
                    true
                }
            }
            V2Expr::Field { base, field, .. } => {
                if let V2Expr::Ident(_, bname) = base.as_ref() {
                    if self.is_bcon_table_ref(bname) {
                        // Read-only BCON constant: never aliases a writable cell.
                        return false;
                    }
                    if self.is_self_ident(bname) {
                        if let Some(st) = self.env.states.get(field) {
                            return format!("{}.attr[{}]", self.self_prefix(), st.attr_index)
                                == canon_target;
                        }
                        let prop_air = match field.as_str() {
                            "graphic" => format!("{}.data[0]", self.self_prefix()),
                            "prep_score" => format!("{}.data[31]", self.self_prefix()),
                            "cook_score" => format!("{}.data[32]", self.self_prefix()),
                            "in_use" => format!("{}.data[25]", self.self_prefix()),
                            "id" => format!("{}.data[11]", self.self_prefix()),
                            "room" => format!("{}.data[29]", self.self_prefix()),
                            other => match object_data_word(other) {
                                Some(w) => format!("{}.data[{}]", self.self_prefix(), w.index),
                                None => return true,
                            },
                        };
                        return prop_air == canon_target;
                    }
                    if self.is_my_ident(bname) {
                        let sim_air = match field.as_str() {
                            "id" => "my.data[11]".to_owned(),
                            "priority" => "my.person[33]".to_owned(),
                            "room" => "my.data[29]".to_owned(),
                            "hand" => "my.slot[0]".to_owned(),
                            "age" => "my.person[58]".to_owned(),
                            "visitor_kind" => "my.person[32]".to_owned(),
                            other => match (sim_person_word(other), object_data_word(other)) {
                                (Some(w), _) => format!("my.person[{}]", w.index),
                                (None, Some(w)) => format!("my.data[{}]", w.index),
                                (None, None) => return true,
                            },
                        };
                        return sim_air == canon_target;
                    }
                    // Foreign object field access swaps `stack_obj` AND the foreign handle may
                    // alias the current object (`stack.*` in interactions OR `my.attr`/`my.data`
                    // in `ObjectSelf` lifecycle hooks such as `for p in objects_of::<Self>()`)!
                    return canon_target.starts_with("stack.")
                        || canon_target == "stack_obj"
                        || canon_target.starts_with("my.attr[")
                        || canon_target.starts_with("my.data[");
                }
                if let V2Expr::Field {
                    base: root,
                    field: group,
                    ..
                } = base.as_ref()
                    && let V2Expr::Ident(_, rname) = root.as_ref()
                    && self.is_my_ident(rname)
                {
                    let sim_air = match group.as_str() {
                        "motive" => canonical_air_cell(&format!("my.motive.{field}")),
                        "skill" => canonical_air_cell(&format!("my.skill.{field}")),
                        _ => return true,
                    };
                    return sim_air == canon_target;
                }
                true
            }
            V2Expr::Index { base, index, .. } => {
                if self.expr_may_read_or_clobber_air(index, canon_target) {
                    return true;
                }
                if let V2Expr::Ident(_, bname) = base.as_ref()
                    && matches!(bname.as_str(), "temp" | "global")
                {
                    return if let Ok(idx) = self.eval_const_i16(index) {
                        format!("{bname}[{idx}]") == canon_target
                    } else {
                        true
                    };
                }
                if let V2Expr::Field {
                    base: root, field, ..
                } = base.as_ref()
                    && let V2Expr::Ident(_, rname) = root.as_ref()
                    && self.is_self_ident(rname)
                    && matches!(field.as_str(), "attr" | "data")
                {
                    return if let Ok(idx) = self.eval_const_i16(index) {
                        format!("{}.{field}[{idx}]", self.self_prefix()) == canon_target
                    } else {
                        true
                    };
                }
                true
            }
            V2Expr::Unary { inner, .. } | V2Expr::MutRef { inner, .. } => {
                self.expr_may_read_or_clobber_air(inner, canon_target)
            }
            V2Expr::Binary { lhs, rhs, .. } => {
                self.expr_may_read_or_clobber_air(lhs, canon_target)
                    || self.expr_may_read_or_clobber_air(rhs, canon_target)
            }
            V2Expr::Tuple(_, elems) => elems
                .iter()
                .any(|e| self.expr_may_read_or_clobber_air(e, canon_target)),
            // Calls/method calls (and `?`-propagated calls) may have arbitrary side effects.
            V2Expr::Call { .. }
            | V2Expr::MethodCall { .. }
            | V2Expr::Try { .. }
            | V2Expr::Match { .. }
            | V2Expr::If { .. }
            | V2Expr::Block { .. } => true,
        }
    }

    /// Lowers a compound arithmetic/bitwise/unary-negation expression directly into `dst_air`,
    /// collapsing left-associative chains `(a * 24) / 5 + 1` into zero extra scratch locals!
    pub(super) fn lower_arith_into_dest(
        &mut self,
        expr: &V2Expr,
        dst_air: &str,
        declare_let: bool,
    ) -> Result<ValueType> {
        // Constant-fold compile-time `Bitflags` expressions (e.g. `Placement::Floor | Placement::Terrain` -> `3`).
        if let Some((key, mask, _)) = self.try_eval_any_const_bitflag(expr)? {
            if declare_let {
                self.line(format!("let {dst_air} = {mask};"));
            } else {
                self.emit_store(dst_air, ":=", &mask.to_string());
            }
            return Ok(ValueType::Bitflags(key));
        }

        let (base, ops) = flatten_arith_chain(expr);

        // Emit base into `dst_air`.
        let base_ty = match base {
            V2Expr::Unary {
                span,
                op: V2UnaryOp::Neg,
                inner,
            } => {
                if let V2Expr::Int(_, v) = inner.as_ref() {
                    let n = v
                        .checked_neg()
                        .and_then(|x| i16::try_from(x).ok())
                        .ok_or_else(|| {
                            Error::at(*span, format!("negated integer -{v} out of i16 range"))
                        })?;
                    if declare_let {
                        self.line(format!("let {dst_air} = {n};"));
                    } else {
                        self.emit_store(dst_air, ":=", &n.to_string());
                    }
                    ValueType::I16
                } else {
                    let (v_air, v_ty) = self.lower_value_expr(inner)?;
                    if v_ty != ValueType::I16 && !is_engine_flags(&v_ty) {
                        return Err(Error::at(
                            *span,
                            format!(
                                "cannot apply unary '-' to '{}' expression",
                                self.env.display_value_type(&v_ty)
                            ),
                        ));
                    }
                    if declare_let {
                        self.line(format!("let {dst_air} = 0;"));
                    } else {
                        self.emit_store(dst_air, ":=", "0");
                    }
                    self.line(format!("{dst_air} -= {v_air};"));
                    ValueType::I16
                }
            }
            other => {
                let (b_air, b_ty) = self.lower_value_expr(other)?;
                if !ops.is_empty() && !matches!(b_ty, ValueType::I16 | ValueType::Bitflags(_)) {
                    return Err(Error::at(
                        other.span(),
                        format!(
                            "arithmetic/bitwise operator cannot be applied to '{}' operand",
                            self.env.display_value_type(&b_ty)
                        ),
                    ));
                }
                if declare_let {
                    self.emit_let(dst_air, &b_air);
                } else if b_air != dst_air {
                    self.emit_store(dst_air, ":=", &b_air);
                }
                b_ty
            }
        };

        let mut base_ty = base_ty;
        for (op, rhs) in ops {
            let is_bit_op = matches!(op, V2BinOp::BitAnd | V2BinOp::BitOr | V2BinOp::BitXor);
            if matches!(base_ty, ValueType::Bitflags(_)) && !is_bit_op && !is_engine_flags(&base_ty)
            {
                return Err(Error::at(
                    rhs.span(),
                    format!(
                        "arithmetic operator cannot be applied to bitflags '{}' (only |, &, ^ are allowed)",
                        self.env.display_value_type(&base_ty)
                    ),
                ));
            }
            let (r_air, r_ty) = self.lower_value_expr(rhs)?;
            // An engine word in arithmetic, or mixed with a raw word, is a plain `i16`.
            if (is_engine_flags(&base_ty) && (r_ty == ValueType::I16 || !is_bit_op))
                || (base_ty == ValueType::I16 && is_engine_flags(&r_ty))
            {
                base_ty = ValueType::I16;
            }
            let r_ty = if base_ty == ValueType::I16 && is_engine_flags(&r_ty) {
                ValueType::I16
            } else {
                r_ty
            };
            if r_ty != base_ty {
                return Err(Error::at(
                    rhs.span(),
                    format!(
                        "mismatched operand types in arithmetic/bitwise expression: '{}' vs '{}'",
                        self.env.display_value_type(&base_ty),
                        self.env.display_value_type(&r_ty)
                    ),
                ));
            }
            let aop = match op {
                V2BinOp::Add => "+=",
                V2BinOp::Sub => "-=",
                V2BinOp::Mul => "*=",
                V2BinOp::Div => "/=",
                V2BinOp::Mod => "%=",
                V2BinOp::BitAnd => "&=",
                V2BinOp::BitOr => "|=",
                V2BinOp::BitXor => "^=",
                _ => unreachable!(),
            };
            self.line(format!("{dst_air} {aop} {r_air};"));
        }

        Ok(base_ty)
    }

    /// Lowers a value-producing expression into an `.air` operand string + its [`ValueType`].
    pub(super) fn lower_value_expr(&mut self, expr: &V2Expr) -> Result<(String, ValueType)> {
        match expr {
            V2Expr::Int(span, v) => {
                let n = i16::try_from(*v)
                    .map_err(|_| Error::at(*span, format!("integer {v} out of i16 range")))?;
                Ok((n.to_string(), ValueType::I16))
            }
            V2Expr::Bool(_, b) => Ok((if *b { "1" } else { "0" }.to_owned(), ValueType::Bool)),
            V2Expr::Path(span, segs) => {
                if let Some(r) = self.bcon_field_ref(expr) {
                    return Ok((r?.0, ValueType::I16));
                }
                if segs.len() != 2 {
                    return Err(Error::at(
                        *span,
                        format!("unsupported path expression '{}'", segs.join("::")),
                    ));
                }
                let (tname, vname) = (&segs[0], &segs[1]);
                if let Some(e) =
                    self.env
                        .lookup_enum(&self.owner_module, tname, self.in_object_scope)
                {
                    let &val = e.variants.get(vname).ok_or_else(|| {
                        Error::at(
                            *span,
                            format!("enum '{}' has no variant '{vname}'", e.display_name),
                        )
                    })?;
                    return Ok((val.to_string(), ValueType::Enum(e.canonical_key.clone())));
                }
                if let Some(bf) =
                    self.env
                        .lookup_bitflags(&self.owner_module, tname, self.in_object_scope)
                {
                    let mask = if vname == "None" {
                        0i16
                    } else if let Some(&bit) = bf.flags.get(vname) {
                        bit_to_mask(bit)
                    } else {
                        return Err(Error::at(
                            *span,
                            format!("bitflags '{}' has no flag '{vname}'", bf.display_name),
                        ));
                    };
                    return Ok((
                        mask.to_string(),
                        ValueType::Bitflags(bf.canonical_key.clone()),
                    ));
                }
                Err(Error::at(
                    *span,
                    format!("unknown enum or bitflags type '{tname}'"),
                ))
            }
            V2Expr::Ident(span, name) => {
                if let Some(binding) = self.lookup_var(name).cloned() {
                    return match binding {
                        Binding::Local { air_name, ty } => Ok((air_name, ty)),
                        Binding::Operand { air, ty } => Ok((air, ty)),
                        Binding::ConstI16 { air, .. } => Ok((air, ValueType::I16)),
                        Binding::ConstBool(b) => {
                            Ok((if b { "1" } else { "0" }.to_owned(), ValueType::Bool))
                        }
                        Binding::ConstEnum { key, value } => {
                            Ok((value.to_string(), ValueType::Enum(key)))
                        }
                        Binding::ConstBitflag { key, mask, .. } => {
                            Ok((mask.to_string(), ValueType::Bitflags(key)))
                        }
                        Binding::AutonomousFlag => {
                            let tmp = self.fresh("auto");
                            self.line(format!("let {tmp} = 0;"));
                            self.line("if param[0] == 1 {".to_owned());
                            self.em.indent();
                            self.emit_store(&tmp, ":=", "1");
                            self.em.dedent();
                            self.line("}".to_owned());
                            Ok((tmp, ValueType::Bool))
                        }
                        Binding::BhavParam { index, ty } => Ok((format!("param[{index}]"), ty)),
                        Binding::ForeignObject { air_name } => Ok((air_name, ValueType::ObjectId)),
                        Binding::MutRef { .. } => Err(Error::at(
                            *span,
                            format!("use '*{name}' to read through '&mut' parameter '{name}'"),
                        )),
                        Binding::SelfHandle => match self.tree_ctx {
                            TreeContext::ObjectSelf => {
                                Ok(("my.id".to_owned(), ValueType::ObjectId))
                            }
                            TreeContext::SimOnObject { .. } => Ok((
                                if self.has_self_obj_local {
                                    "__self_obj".to_owned()
                                } else {
                                    "stack_obj".to_owned()
                                },
                                ValueType::ObjectId,
                            )),
                        },
                        Binding::SimHandle => Ok(("my.id".to_owned(), ValueType::ObjectId)),
                        Binding::Motive(_)
                        | Binding::Guid(_)
                        | Binding::Skill(_)
                        | Binding::Sound(_)
                        | Binding::Slot(_) => Err(Error::at(
                            *span,
                            format!("'{name}' cannot be used directly as a numeric value"),
                        )),
                    };
                }
                if self.is_self_ident(name) {
                    return match self.tree_ctx {
                        TreeContext::ObjectSelf => Ok(("my.id".to_owned(), ValueType::ObjectId)),
                        TreeContext::SimOnObject { .. } => Ok((
                            if self.has_self_obj_local {
                                "__self_obj".to_owned()
                            } else {
                                "stack_obj".to_owned()
                            },
                            ValueType::ObjectId,
                        )),
                    };
                }
                if name == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                    return Err(Error::at(
                        *span,
                        "'sim' is not available in init/main (object lifecycle context)",
                    ));
                }
                if let Some(v) = self.const_item(name) {
                    let n = i16::try_from(v).map_err(|_| {
                        Error::at(
                            *span,
                            format!(
                                "const '{name}' ({v:#x}) does not fit in i16; use it where a GUID is expected"
                            ),
                        )
                    })?;
                    return Ok((n.to_string(), ValueType::I16));
                }
                if name == "stack_obj" {
                    return Ok(("stack_obj".to_owned(), ValueType::ObjectId));
                }
                if self.env.i16_params.contains_key(name) {
                    return Ok((format!("bcon.{name}"), ValueType::I16));
                }
                if let Some(&b) = self.env.bool_params.get(name) {
                    return Ok((if b { "1" } else { "0" }.to_owned(), ValueType::Bool));
                }
                if name == "error" {
                    return Err(Error::at(
                        *span,
                        "unknown identifier 'error' (the error exit is 'panic!();')",
                    ));
                }
                Err(Error::at(*span, format!("unknown identifier '{name}'")))
            }
            V2Expr::Deref { span, inner } => {
                let lval = self.resolve_lvalue(&V2Expr::Deref {
                    span: *span,
                    inner: inner.clone(),
                })?;
                self.read_lvalue(&lval)
            }
            V2Expr::Field { span, base, field } => {
                if let Some(r) = self.bcon_field_ref(expr) {
                    return Ok((r?.0, ValueType::I16));
                }
                // Handle read-only properties like `sim.id`, `sim.priority`, `self.id`, `self.room`
                if let V2Expr::Ident(bspan, bname) = base.as_ref() {
                    if self.is_my_ident(bname) {
                        return match field.as_str() {
                            "id" => Ok(("my.id".to_owned(), ValueType::ObjectId)),
                            "priority" => Ok(("my.person.priority".to_owned(), ValueType::I16)),
                            "room" => Ok(("my.room".to_owned(), ValueType::I16)),
                            "hand" => Ok(("my.slot[0]".to_owned(), ValueType::ObjectId)),
                            "age" => Ok(("my.person[58]".to_owned(), ValueType::I16)),
                            "visitor_kind" => Ok(("my.person[32]".to_owned(), ValueType::I16)),
                            other if let Some(w) = sim_person_word(other) => {
                                Ok((format!("my.person[{}]", w.index), w.value_type()))
                            }
                            // The named object-data words (`container` is word 2, the object
                            // whose slot holds the Sim).
                            other if let Some(w) = object_data_word(other) => {
                                Ok((format!("my.data[{}]", w.index), w.value_type()))
                            }
                            other => Err(Error::at(
                                *span,
                                format!(
                                    "unknown Sim property '{bname}.{other}' (expected motive, skill, personality, id, priority, room, hand, age, visitor_kind, a named person word (posture, job_level, gender_kind, date_partner, ...), a named object-data word (container, facing, flags, value, ...), attr[i], data[i], slot[i], or person[i]; use 'self.<state>' for object state)"
                                ),
                            )),
                        };
                    }
                    if bname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *bspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                    // `self.id` and `self.room` read their words (data 11 and 29) by name.
                    if self.is_self_ident(bname) {
                        match field.as_str() {
                            "id" => {
                                return self
                                    .read_lvalue(&self.self_cell("id", ValueType::ObjectId));
                            }
                            "room" => {
                                return self.read_lvalue(&self.self_cell("room", ValueType::I16));
                            }
                            _ => {}
                        }
                    }
                }
                // `h.neighbor.<name>`: the neighborhood record the handle names, read through
                // the stack object.
                if let V2Expr::Field {
                    base: root,
                    field: middle,
                    ..
                } = base.as_ref()
                    && middle == "neighbor"
                    && let V2Expr::Ident(_, rname) = root.as_ref()
                    && !self.handle_has_state(rname, "neighbor")
                {
                    let Some((cell, ty)) = neighbor_cell(field) else {
                        return Err(Error::at(
                            *span,
                            format!(
                                "unknown neighbor field '{field}' (expected person_on_lot, in_selected_family, age, friend_count, family_house, family_friend_count, a named person word (family, gender_kind, job_type, fame, ...), field[i], or person[i])"
                            ),
                        ));
                    };
                    return self.read_neighbor_cell(rname, *span, &cell, ty);
                }
                if let V2Expr::Ident(_, bname) = base.as_ref()
                    && let Some(word) = definition_field(field)
                    && !self.handle_has_state(bname, field)
                    && let Some(read) = self.read_stack_object_cell(
                        bname,
                        *span,
                        &format!("scope(21, {word})"),
                        ValueType::I16,
                    )?
                {
                    return Ok(read);
                }
                let lval = self.resolve_lvalue(expr)?;
                self.read_lvalue(&lval)
            }
            // `x.has_function[N]`: whether the stack object's definition names a tree for
            // function N (`sims-vm` `memory.rs` scope 35), read through the stack object.
            V2Expr::Index { span, base, index }
                if matches!(base.as_ref(), V2Expr::Field { base: root, field, .. }
                    if field == "has_function" && matches!(root.as_ref(), V2Expr::Ident(..))) =>
            {
                let V2Expr::Field { base: root, .. } = base.as_ref() else {
                    unreachable!("matched above");
                };
                let V2Expr::Ident(_, rname) = root.as_ref() else {
                    unreachable!("matched above");
                };
                let n = self.eval_builtin_int(index, "has_function index", 0, 255)?;
                match self.read_stack_object_cell(
                    rname,
                    *span,
                    &format!("scope(35, {n})"),
                    ValueType::Bool,
                )? {
                    Some(read) => Ok(read),
                    None => Err(Error::at(
                        *span,
                        format!(
                            "'{rname}.has_function[..]' needs an object handle ('self', 'stack_obj', or an object variable)"
                        ),
                    )),
                }
            }
            // `h.neighbor.field[i]` and `h.neighbor.person[i]`.
            V2Expr::Index { span, base, index }
                if matches!(base.as_ref(), V2Expr::Field { base: mid, field, .. }
                    if neighbor_indexed_scope(field).is_some()
                        && matches!(mid.as_ref(), V2Expr::Field { base: root, field: m, .. }
                            if m == "neighbor"
                                && matches!(root.as_ref(), V2Expr::Ident(_, r) if !self.handle_has_state(r, "neighbor")))) =>
            {
                let V2Expr::Field {
                    base: mid, field, ..
                } = base.as_ref()
                else {
                    unreachable!("matched above");
                };
                let V2Expr::Field { base: root, .. } = mid.as_ref() else {
                    unreachable!("matched above");
                };
                let V2Expr::Ident(_, rname) = root.as_ref() else {
                    unreachable!("matched above");
                };
                let (scope, hi) = neighbor_indexed_scope(field).expect("matched above");
                let i = self.eval_builtin_int(index, &format!("neighbor.{field} index"), 0, hi)?;
                self.read_neighbor_cell(
                    rname,
                    *span,
                    &format!("scope({scope}, {i})"),
                    ValueType::I16,
                )
            }
            V2Expr::Index { span, base, index }
                if matches!(base.as_ref(), V2Expr::Ident(_, n) if n == "local")
                    && self.lookup_var("local").is_none() =>
            {
                self.read_frame_local_slot(*span, index)
            }
            V2Expr::Index { .. } => {
                let lval = self.resolve_lvalue(expr)?;
                self.read_lvalue(&lval)
            }
            V2Expr::Match {
                span,
                scrutinee,
                arms,
            } => self.lower_match_expr(*span, scrutinee, arms, None, false),
            V2Expr::If {
                span,
                cond,
                then_value,
                else_value,
            } => self.lower_if_expr(*span, cond, then_value, else_value, None, false),
            V2Expr::Block { span, .. } => Err(Error::at(
                *span,
                "a diverging '{ ..; return ..; }' branch can only be a branch of 'let x = if ..' or 'let x = match ..'",
            )),
            V2Expr::Unary { span, op, inner } => match op {
                V2UnaryOp::Not => {
                    let tmp = self.fresh("not");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(expr, &tmp)?;
                    Ok((tmp, ValueType::Bool))
                }
                V2UnaryOp::Neg => {
                    if let V2Expr::Int(_, v) = inner.as_ref() {
                        let n = v
                            .checked_neg()
                            .and_then(|x| i16::try_from(x).ok())
                            .ok_or_else(|| {
                                Error::at(*span, format!("negated integer -{v} out of i16 range"))
                            })?;
                        return Ok((n.to_string(), ValueType::I16));
                    }
                    let tmp = self.fresh("neg");
                    let ty = self.lower_arith_into_dest(expr, &tmp, true)?;
                    Ok((tmp, ty))
                }
                // `e as i16`: the operand of `e` itself, retyped (no `.air` nodes).
                V2UnaryOp::AsI16 => {
                    if let Some(b) = self.try_eval_cast_bool(inner) {
                        return Ok((i16::from(b).to_string(), ValueType::I16));
                    }
                    let (air, ty) = self.lower_value_expr(inner)?;
                    self.check_as_i16_source(*span, &ty)?;
                    // A bool word may hold any nonzero value (a visitor's person word 32 is
                    // 2 for some), so only a constant bool converts in place.
                    if ty == ValueType::Bool && air != "0" && air != "1" {
                        return Err(Error::at(
                            *span,
                            "'as i16' converts only a constant bool (a bool word may hold any nonzero value); write 'if b { 1 } else { 0 }'",
                        ));
                    }
                    Ok((air, ValueType::I16))
                }
            },
            V2Expr::Binary { op, .. } => match op {
                V2BinOp::And
                | V2BinOp::Or
                | V2BinOp::Eq
                | V2BinOp::Ne
                | V2BinOp::Lt
                | V2BinOp::Le
                | V2BinOp::Gt
                | V2BinOp::Ge
                | V2BinOp::FlagTest => {
                    let tmp = self.fresh("bool");
                    self.line(format!("let {tmp} = 0;"));
                    self.emit_bool_into_local(expr, &tmp)?;
                    Ok((tmp, ValueType::Bool))
                }
                V2BinOp::Add
                | V2BinOp::Sub
                | V2BinOp::Mul
                | V2BinOp::Div
                | V2BinOp::Mod
                | V2BinOp::BitAnd
                | V2BinOp::BitOr
                | V2BinOp::BitXor => {
                    // Constant-fold pure Bitflags expressions into an immediate literal!
                    if let Some((key, mask, _)) = self.try_eval_any_const_bitflag(expr)? {
                        return Ok((mask.to_string(), ValueType::Bitflags(key)));
                    }
                    let tmp = self.fresh("arith");
                    let ty = self.lower_arith_into_dest(expr, &tmp, true)?;
                    Ok((tmp, ty))
                }
            },
            V2Expr::Call {
                span,
                callee,
                type_args,
                args,
            } => {
                if let Some(target) = self.raw_conversion_target(callee) {
                    return self.lower_from_raw(*span, target, type_args, args);
                }
                if matches!(callee.as_ref(), V2Expr::Ident(_, name) if name == "create_object")
                    && has_create_dest(args)
                    && !self.user_callable_named(callee, *span)?
                {
                    let temp = self.fresh("call");
                    self.line(format!("let {temp} = 0;"));
                    self.emit_bool_into_local(expr, &temp)?;
                    return Ok((temp, ValueType::Bool));
                }
                if !type_args.is_empty() {
                    return Err(Error::at(
                        *span,
                        "type arguments are only supported on 'create_object::<T>', 'find_best_object::<T>', and 'objects_of::<T>'",
                    ));
                }
                if let Some(sig) = self.resolve_callable(callee, *span)? {
                    return self.lower_callable_value(&sig, args, *span, false, false);
                }
                if let V2Expr::Ident(_, fname) = callee.as_ref() {
                    if fname == "random" {
                        if args.len() != 1 || args[0].name.is_some() {
                            return Err(Error::at(*span, "random(range) expects 1 argument"));
                        }
                        let (mut range_air, _) = self.lower_value_expr(&args[0].value)?;
                        // See `random_into`: a global range would receive the draw.
                        if range_air.starts_with("global[") {
                            let copy = self.fresh("rnd_range");
                            self.emit_let(&copy, &range_air);
                            range_air = copy;
                        }
                        let tmp = self.fresh("rnd");
                        self.line(format!("let {tmp} = 0;"));
                        self.line(format!("random({tmp}, {range_air});"));
                        return Ok((tmp, ValueType::I16));
                    }
                    // `job_field(F)`: field F of the job `temp[0]` (career) and `temp[1]` (level)
                    // name as they are where it is read (`sims-vm` `memory.rs` scope 33), as a
                    // plain operand a compare or store reads in place.
                    if fname == "job_field"
                        && self
                            .env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                            .is_none()
                    {
                        let bound = self.bind_builtin_args("job_field", args, &["field"], &[])?;
                        let field = bound.get("field").ok_or_else(|| {
                            Error::at(*span, "job_field(field) requires a field number")
                        })?;
                        let f = self.eval_builtin_int(&field.value, "job_field field", 0, 32767)?;
                        return Ok((format!("scope(33, {f})"), ValueType::I16));
                    }
                    if fname == "job_data"
                        && self
                            .env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                            .is_none()
                    {
                        return self.lower_job_data(args, *span);
                    }
                    if fname == "distance_to"
                        && self
                            .env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                            .is_none()
                    {
                        return self.lower_distance_to(args, *span);
                    }
                    if matches!(
                        fname.as_str(),
                        "can_afford"
                            | "debit_funds"
                            | "credit_funds"
                            | "dialog"
                            | "route_to"
                            | "snap"
                    ) || (matches!(fname.as_str(), "create_object" | "find_best_object")
                        && !self.user_callable_named(callee, *span)?)
                    {
                        let tmp = self.fresh("call");
                        self.line(format!("let {tmp} = 0;"));
                        self.emit_bool_into_local(expr, &tmp)?;
                        return Ok((tmp, ValueType::Bool));
                    }
                    if let Some((fn_mod, fn_decl, is_local)) =
                        self.env
                            .lookup_fn(&self.owner_module, fname, self.in_object_scope)
                    {
                        let ret =
                            self.inline_helper_call(&fn_mod, &fn_decl, is_local, args, *span)?;
                        let (rv, vty) = ret.ok_or_else(|| {
                            Error::at(
                                *span,
                                format!("void helper function '{fname}' does not return a value"),
                            )
                        })?;
                        return Ok((rv, vty));
                    }
                    if is_prim_builtin_name(fname) {
                        let tmp = self.fresh("call");
                        self.line(format!("let {tmp} = 0;"));
                        self.emit_bool_into_local(expr, &tmp)?;
                        return Ok((tmp, ValueType::Bool));
                    }
                }
                Err(Error::at(*span, "unsupported value call expression"))
            }
            V2Expr::MethodCall {
                method, type_args, ..
            } if (method == "is_type" && type_args.is_empty())
                || matches!(method.as_str(), "debit_funds" | "credit_funds" | "remove") =>
            {
                let tmp = self.fresh("call");
                self.line(format!("let {tmp} = 0;"));
                self.emit_bool_into_local(expr, &tmp)?;
                Ok((tmp, ValueType::Bool))
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
                let tmp = self.fresh("rf");
                self.line(format!("let {tmp} = 0;"));
                self.emit_run_functional(receiver, args, *span, Some(&tmp))?;
                Ok((tmp, ValueType::Bool))
            }
            V2Expr::MethodCall { method, .. } if is_bitflag_predicate(method) => {
                let tmp = self.fresh("bf");
                self.line(format!("let {tmp} = 0;"));
                self.emit_bool_into_local(expr, &tmp)?;
                Ok((tmp, ValueType::Bool))
            }
            V2Expr::Try { span, inner } => {
                if let V2Expr::Call {
                    span: cspan,
                    callee,
                    type_args,
                    args,
                } = inner.as_ref()
                    && type_args.is_empty()
                    && let Some(sig) = self.resolve_callable(callee, *cspan)?
                {
                    return self.lower_callable_value(&sig, args, *cspan, true, false);
                }
                Err(Error::at(
                    *span,
                    "'?' in an expression needs a call to an 'extern mod' fn or '#[bhav] fn' (use 'cond?;' as a statement for plain bool checks)",
                ))
            }
            other => Err(Error::at(
                other.span(),
                "expression cannot be evaluated as a value",
            )),
        }
    }

    /// `job_data(career, level, field)`: scope 33 reads a field of a career level, the career
    /// taken from `temp[0]` and the level from `temp[1]`. Both temps are overwritten.
    pub(super) fn lower_job_data(
        &mut self,
        args: &[CallArg],
        span: Span,
    ) -> Result<(String, ValueType)> {
        let bound = self.bind_builtin_args("job_data", args, &["career", "level", "field"], &[])?;
        let get = |key: &str| {
            bound
                .get(key)
                .copied()
                .ok_or_else(|| Error::at(span, format!("job_data() requires '{key}'")))
        };
        let (career_arg, level_arg, field_arg) = (get("career")?, get("level")?, get("field")?);
        // Operands are evaluated in source order (named arguments may be reordered), as for a
        // call. Each is read when it is staged, so one is copied first when an operand evaluated
        // after it may write anything (a call, a nested `job_data`), or when it is the level and
        // reads `temp[0]`, which the career's staging overwrites.
        let level_first = args.iter().position(|a| std::ptr::eq(a, level_arg))
            < args.iter().position(|a| std::ptr::eq(a, career_arg));
        let (first, second) = if level_first {
            (level_arg, career_arg)
        } else {
            (career_arg, level_arg)
        };
        let first_key = if level_first { "level" } else { "career" };
        let second_key = if level_first { "career" } else { "level" };
        let mut a = self.lower_job_operand(first_key, first)?;
        if !self.is_pure_single_operand(&second.value) {
            a = self.snapshot_operand(a, &format!("job_{first_key}"));
        }
        let b = self.lower_job_operand(second_key, second)?;
        let (career, mut level) = if level_first { (b, a) } else { (a, b) };
        let field = self.eval_builtin_int(&field_arg.value, "job_data field", 0, 32767)?;
        // The career's staging writes `temp[0]` first: a level that reads it (job data reads
        // both temps) is copied before.
        if reads_temp0(&level) {
            level = self.snapshot_operand(level, "job_level");
        }
        self.line(format!("temp[0] := {career};"));
        self.line(format!("temp[1] := {level};"));
        let tmp = self.fresh("job");
        self.line(format!("let {tmp} = scope(33, {field});"));
        Ok((tmp, ValueType::I16))
    }

    /// Whether `handle.<field>` names a state of the object `handle` is (which then wins over an
    /// engine field of the same name).
    pub(super) fn handle_has_state(&self, handle: &str, field: &str) -> bool {
        if self.is_self_ident(handle) {
            return self.env.states.contains_key(field);
        }
        match self.lookup_var(handle) {
            Some(
                Binding::Local {
                    ty: ValueType::ObjectRef(key),
                    ..
                }
                | Binding::BhavParam {
                    ty: ValueType::ObjectRef(key),
                    ..
                },
            ) => self
                .env
                .object_schemas
                .get(key)
                .is_some_and(|s| s.states.contains_key(field)),
            _ => false,
        }
    }

    /// Whether a definition cell (`scope(21, ..)`, `scope(35, ..)`) of `handle` is read in place,
    /// with no cursor switch (see [`Self::read_stack_object_cell`]).
    pub(super) fn definition_cell_in_place(&self, handle: &str) -> bool {
        handle == "stack_obj"
            || (self.is_self_keyword(handle)
                && !self.inherit_tree
                && matches!(self.tree_ctx, TreeContext::SimOnObject { .. })
                && self.self_stack_target() == "stack_obj")
    }

    /// Reads `cell`, a cell of the stack object (`scope(21, ..)`, `scope(35, ..)`), for the object
    /// `handle` names: in place when that is the stack object, else with the stack object moved
    /// to it and put back. `None` when `handle` is no object handle. A Sim handle (`sim`, `me`
    /// in a Sim tree) is refused: it is `my.`, not reached through the stack object, so a tree
    /// that never moves the cursor keeps no entry save to put it back with.
    pub(super) fn read_stack_object_cell(
        &mut self,
        handle: &str,
        span: Span,
        cell: &str,
        ty: ValueType,
    ) -> Result<Option<(String, ValueType)>> {
        let direct = |ty| LValue::Direct {
            air: cell.to_owned(),
            ty,
        };
        let foreign = |obj_air: &str, ty| LValue::ForeignCell {
            obj_air: obj_air.to_owned(),
            cell: cell.to_owned(),
            setup: None,
            ty,
        };
        let lval = if handle == "stack_obj" {
            direct(ty)
        } else if self.is_self_keyword(handle) {
            if self.inherit_tree {
                return Err(Error::at(
                    span,
                    "a 'context = inherit' tree runs on whichever object calls it; read the field through 'stack_obj'",
                ));
            }
            match self.tree_ctx {
                TreeContext::ObjectSelf => foreign("my.id", ty),
                TreeContext::SimOnObject { .. } => match self.self_stack_target() {
                    "stack_obj" => direct(ty),
                    target => foreign(target, ty),
                },
            }
        } else if self.is_frame_me(handle)
            && !self.is_my_ident(handle)
            && matches!(self.tree_ctx, TreeContext::ObjectSelf)
        {
            // In an object's own tree `me` is the running object, as `self` is.
            foreign("my.id", ty)
        } else if self.is_my_ident(handle) || self.is_frame_me(handle) {
            return Err(Error::at(
                span,
                format!(
                    "'{handle}' is read through 'my.', not the stack object: select it first ('stack_obj = {handle};') and read the field on 'stack_obj'"
                ),
            ));
        } else if let Some((obj_air, _)) = self.object_handle_air(handle) {
            foreign(&obj_air, ty)
        } else {
            return Ok(None);
        };
        self.read_lvalue(&lval).map(Some)
    }

    /// A neighbor cell (`scope(24, ..)`, `scope(32, ..)`), read through the stack object like a
    /// definition cell.
    fn read_neighbor_cell(
        &mut self,
        handle: &str,
        span: Span,
        cell: &str,
        ty: ValueType,
    ) -> Result<(String, ValueType)> {
        self.read_stack_object_cell(handle, span, cell, ty)?
            .ok_or_else(|| {
                Error::at(
                    span,
                    format!(
                        "'{handle}.neighbor' needs an object handle holding a neighbor ('stack_obj' after 'set_to_next(neighbor)', or an object variable)"
                    ),
                )
            })
    }

    pub(super) fn lower_job_operand(&mut self, key: &str, arg: &CallArg) -> Result<String> {
        let (air, ty) = self.lower_value_expr(&arg.value)?;
        if !i16_like(&ty) {
            return Err(Error::at(
                arg.span,
                format!("job_data {key} must be an i16"),
            ));
        }
        // A runtime (`extern bcon`) read faults on a missing chunk: as for a call argument, it is
        // read now, so the fault cannot follow a partial staging of the temps.
        Ok(if air.starts_with("bcon[") {
            self.snapshot_operand(air, &format!("job_{key}"))
        } else {
            air
        })
    }

    /// `air` copied into a fresh local, unless it is a literal (which nothing can change).
    pub(super) fn snapshot_operand(&mut self, air: String, prefix: &str) -> String {
        if air.parse::<i64>().is_ok() {
            return air;
        }
        let copy = self.fresh(prefix);
        self.emit_let(&copy, &air);
        copy
    }

    /// `distance_to(target, from: obj, temp: n)`: Get Distance To measures from `from` (the
    /// running object by default) to the stack object, so `stack_obj` points at `target` around
    /// it. The result lands in `temp[n]` (0 by default).
    pub(super) fn lower_distance_to(
        &mut self,
        args: &[CallArg],
        span: Span,
    ) -> Result<(String, ValueType)> {
        let bound = self.bind_builtin_args("distance_to", args, &["target"], &["from", "temp"])?;
        let target_arg = *bound
            .get("target")
            .ok_or_else(|| Error::at(span, "distance_to() requires a target object"))?;
        let from_arg = bound.get("from").copied();
        let mut target = self.lower_distance_handle("target", target_arg)?;
        // The target is read when `stack_obj` moves, after `from` is lowered: a `from` that may
        // write anything gets a copy of the target first.
        // That includes `stack_obj` itself, which a `from` that moves and restores the stack
        // object (a nested `distance_to`) leaves pointing elsewhere.
        if from_arg.is_some_and(|a| !self.is_pure_single_operand(&a.value)) {
            target = self.snapshot_operand(target, "dist_target");
        }
        let mut from = match from_arg {
            Some(arg) => Some(self.lower_distance_handle("from", arg)?),
            None => None,
        };
        let temp = match bound.get("temp") {
            Some(arg) => self.eval_builtin_int(&arg.value, "distance_to temp", 0, 7)?,
            None => 0,
        };
        let moves = target != "stack_obj";
        // `from` is read after `stack_obj` moves to the target: one that names the stack object
        // (`stack_obj`, or a `stack.` cell such as `stack.id`) is copied before the move.
        if moves && let Some(f) = from.take() {
            let stack_relative = f.contains("stack.") || reads_stack_obj(&f);
            from = Some(if stack_relative {
                self.snapshot_operand(f, "dist_from")
            } else {
                f
            });
        }
        let saved = if moves {
            let saved = self.save_stack_obj_for_move();
            self.stack_op(mir::StackObjOp::Set(&target));
            saved
        } else {
            None
        };
        self.line(match &from {
            Some(f) => format!("get_distance_to(temp[{temp}], from: {f});"),
            None => format!("get_distance_to(temp[{temp}]);"),
        });
        if moves {
            self.emit_restore_stack_obj(saved.as_deref());
        }
        let tmp = self.fresh("dist");
        self.line(format!("let {tmp} = temp[{temp}];"));
        Ok((tmp, ValueType::I16))
    }

    pub(super) fn lower_distance_handle(&mut self, key: &str, arg: &CallArg) -> Result<String> {
        let (air, ty) = self.lower_value_expr(&arg.value)?;
        if !ty.is_object_handle() {
            return Err(Error::at(
                arg.span,
                format!(
                    "distance_to {key} must be an object, found '{}'",
                    self.env.display_value_type(&ty)
                ),
            ));
        }
        Ok(air)
    }

    /// The type `T` of a `T::from_raw(..)` call, when `T` names an enum or bitflags type in
    /// scope (`from_raw` is a built-in conversion, not a user function).
    pub(super) fn raw_conversion_target(&self, callee: &V2Expr) -> Option<ValueType> {
        let V2Expr::Path(_, segs) = callee else {
            return None;
        };
        let [tname, method] = segs.as_slice() else {
            return None;
        };
        if method != "from_raw" {
            return None;
        }
        // An `extern mod` named like the type owns the path, in every position: the call
        // then resolves (or fails) as that module's fn.
        if self
            .env
            .lookup_extern_mod(&self.owner_module, tname, self.in_object_scope)
            .is_some()
        {
            return None;
        }
        match self
            .env
            .lookup_named_type(&self.owner_module, tname, self.in_object_scope)?
        {
            ResolvedNamedType::Enum(k) if self.env.all_enums.contains_key(&k) => {
                Some(ValueType::Enum(k))
            }
            ResolvedNamedType::Bitflags(k) if self.env.all_bitflags.contains_key(&k) => {
                Some(ValueType::Bitflags(k))
            }
            _ => None,
        }
    }

    /// `T::from_raw(e)`: the operand of the `i16` expression `e`, typed `T` (no `.air` nodes),
    /// for a bitflags type or a `std::engine` enum. Any word is accepted: engine enums stay
    /// open, so a `match` on one needs a `_` arm.
    pub(super) fn lower_from_raw(
        &mut self,
        span: Span,
        target: ValueType,
        type_args: &[V2Type],
        args: &[CallArg],
    ) -> Result<(String, ValueType)> {
        self.check_from_raw_call(span, &target, type_args, args)?;
        let (air, ty) = self.lower_value_expr(&args[0].value)?;
        self.check_from_raw_arg(&args[0].value, &target, &ty)?;
        Ok((air, target))
    }

    /// The shape rules of `T::from_raw(..)`, shared by value lowering and constant evaluation.
    pub(super) fn check_from_raw_call(
        &self,
        span: Span,
        target: &ValueType,
        type_args: &[V2Type],
        args: &[CallArg],
    ) -> Result<()> {
        let name = self.env.display_value_type(target);
        if !type_args.is_empty() || args.len() != 1 || args[0].name.is_some() {
            return Err(Error::at(
                span,
                format!("'{name}::from_raw' takes exactly one positional i16 argument"),
            ));
        }
        // A user enum is closed: a `match` lists its variants with no `_`, so a word with no
        // variant must not reach one.
        if let ValueType::Enum(key) = target
            && !is_engine_enum_key(key)
        {
            return Err(Error::at(
                span,
                format!(
                    "'{name}::from_raw' is only for 'std::engine' enums and bitflags types: '{name}' is a closed enum, so a word with no variant would match a listed arm"
                ),
            ));
        }
        Ok(())
    }

    /// `T::from_raw(e)` takes an `i16` or a `T`.
    pub(super) fn check_from_raw_arg(
        &self,
        arg: &V2Expr,
        target: &ValueType,
        ty: &ValueType,
    ) -> Result<()> {
        if !i16_like(ty) && ty != target {
            return Err(Error::at(
                arg.span(),
                format!(
                    "'{}::from_raw' expects an i16, found '{}' (use 'as i16' first)",
                    self.env.display_value_type(target),
                    self.env.display_value_type(ty)
                ),
            ));
        }
        Ok(())
    }

    /// `e as i16` accepts integers, bools, enums and bitflags; object handles already
    /// convert to and from `i16` implicitly.
    pub(super) fn check_as_i16_source(&self, span: Span, ty: &ValueType) -> Result<()> {
        match ty {
            ValueType::I16 | ValueType::Bool | ValueType::Enum(_) | ValueType::Bitflags(_) => {
                Ok(())
            }
            ValueType::ObjectId | ValueType::ObjectRef(_) => Err(Error::at(
                span,
                format!(
                    "'as i16' does not apply to '{}' (object ids already convert to i16)",
                    self.env.display_value_type(ty)
                ),
            )),
        }
    }
}
