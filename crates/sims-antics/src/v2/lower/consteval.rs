//! Compile-time evaluation: constant enums/bitflags/bools/ints, literal condition folding and
//! dead-branch diagnostics.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// Resolves a compile-time `Enum` constant (`Enum::Variant` or `Binding::ConstEnum`).
    pub(super) fn try_eval_const_enum(
        &self,
        expr: &V2Expr,
        expected_key: &str,
    ) -> Result<Option<i16>> {
        match expr {
            V2Expr::Path(span, segs) if segs.len() == 2 => {
                if let Some(e) =
                    self.env
                        .lookup_enum(&self.owner_module, &segs[0], self.in_object_scope)
                {
                    if e.canonical_key != expected_key {
                        return Err(Error::at(
                            *span,
                            format!(
                                "type mismatch: expected '{}', found '{}'",
                                self.env
                                    .display_value_type(&ValueType::Enum(expected_key.to_owned())),
                                e.display_name
                            ),
                        ));
                    }
                    let &val = e.variants.get(&segs[1]).ok_or_else(|| {
                        Error::at(
                            *span,
                            format!("enum '{}' has no variant '{}'", e.display_name, segs[1]),
                        )
                    })?;
                    return Ok(Some(val));
                }
                Ok(None)
            }
            V2Expr::Ident(span, name) => {
                if let Some(Binding::ConstEnum { key, value }) = self.lookup_var(name) {
                    if key != expected_key {
                        return Err(Error::at(
                            *span,
                            format!(
                                "type mismatch: expected '{}', found '{}'",
                                self.env
                                    .display_value_type(&ValueType::Enum(expected_key.to_owned())),
                                self.env.display_value_type(&ValueType::Enum(key.clone()))
                            ),
                        ));
                    }
                    return Ok(Some(*value));
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Resolves a compile-time constant of any enum (`Enum::Variant` or `Binding::ConstEnum`).
    pub(super) fn try_eval_any_const_enum(&self, expr: &V2Expr) -> Result<Option<i16>> {
        Ok(self.try_eval_keyed_const_enum(expr)?.map(|(_, v)| v))
    }

    /// A constant of an engine type (a `std::engine` variant or a `std::flags` mask), which a
    /// primitive's numeric operand takes as its number.
    pub(super) fn try_eval_engine_const(&self, expr: &V2Expr) -> Result<Option<i16>> {
        if let Some((key, mask, _)) = self.try_eval_any_const_bitflag(expr)? {
            return Ok(is_engine_flags_key(&key).then_some(mask));
        }
        Ok(self
            .try_eval_keyed_const_enum(expr)?
            .and_then(|(key, v)| is_engine_enum_key(&key).then_some(v)))
    }

    /// [`Self::try_eval_any_const_enum`] with the enum's key.
    fn try_eval_keyed_const_enum(&self, expr: &V2Expr) -> Result<Option<(String, i16)>> {
        match expr {
            V2Expr::Path(span, segs) if segs.len() == 2 => {
                let Some(e) =
                    self.env
                        .lookup_enum(&self.owner_module, &segs[0], self.in_object_scope)
                else {
                    return Ok(None);
                };
                let &val = e.variants.get(&segs[1]).ok_or_else(|| {
                    Error::at(
                        *span,
                        format!("enum '{}' has no variant '{}'", e.display_name, segs[1]),
                    )
                })?;
                Ok(Some((e.canonical_key.clone(), val)))
            }
            V2Expr::Ident(_, name) => match self.lookup_var(name) {
                Some(Binding::ConstEnum { key, value }) => Ok(Some((key.clone(), *value))),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Resolves a compile-time `Bitflags` constant expression (single flag, `None`, or `|`/`&`/`^` of constants),
    /// returning `(mask, single_bit_1_based)` when constant!
    pub(super) fn try_eval_const_bitflag(
        &self,
        expr: &V2Expr,
        expected_key: &str,
    ) -> Result<Option<(i16, Option<u8>)>> {
        match expr {
            V2Expr::Path(span, segs) if segs.len() == 2 => {
                if let Some(bf) =
                    self.env
                        .lookup_bitflags(&self.owner_module, &segs[0], self.in_object_scope)
                {
                    if bf.canonical_key != expected_key && expected_key != ANY_BITFLAGS {
                        return Err(Error::at(
                            *span,
                            format!(
                                "type mismatch: expected bitflags '{}', found '{}'",
                                self.env.display_value_type(&ValueType::Bitflags(
                                    expected_key.to_owned()
                                )),
                                bf.display_name
                            ),
                        ));
                    }
                    if segs[1] == "None" {
                        return Ok(Some((0, None)));
                    }
                    let &bit = bf.flags.get(&segs[1]).ok_or_else(|| {
                        Error::at(
                            *span,
                            format!("bitflags '{}' has no flag '{}'", bf.display_name, segs[1]),
                        )
                    })?;
                    return Ok(Some((bit_to_mask(bit), Some(bit))));
                }
                Ok(None)
            }
            V2Expr::Ident(span, name) => {
                if let Some(Binding::ConstBitflag {
                    key,
                    mask,
                    single_bit,
                }) = self.lookup_var(name)
                {
                    if key != expected_key && expected_key != ANY_BITFLAGS {
                        return Err(Error::at(
                            *span,
                            format!(
                                "type mismatch: expected bitflags '{}', found '{}'",
                                self.env.display_value_type(&ValueType::Bitflags(
                                    expected_key.to_owned()
                                )),
                                self.env
                                    .display_value_type(&ValueType::Bitflags(key.clone()))
                            ),
                        ));
                    }
                    return Ok(Some((*mask, *single_bit)));
                }
                Ok(None)
            }
            V2Expr::Binary {
                lhs,
                op: op @ (V2BinOp::BitOr | V2BinOp::BitAnd | V2BinOp::BitXor),
                rhs,
                ..
            } => {
                let Some((lmask, _)) = self.try_eval_const_bitflag(lhs, expected_key)? else {
                    return Ok(None);
                };
                let Some((rmask, _)) = self.try_eval_const_bitflag(rhs, expected_key)? else {
                    return Ok(None);
                };
                let mask = match op {
                    V2BinOp::BitOr => lmask | rmask,
                    V2BinOp::BitAnd => lmask & rmask,
                    V2BinOp::BitXor => lmask ^ rmask,
                    _ => unreachable!(),
                };
                let single_bit = single_bit_of_mask(mask);
                Ok(Some((mask, single_bit)))
            }
            _ => Ok(None),
        }
    }

    /// Detects if `expr` is a compile-time constant `Bitflags` expression of ANY bitflags type.
    pub(super) fn try_eval_any_const_bitflag(
        &self,
        expr: &V2Expr,
    ) -> Result<Option<(String, i16, Option<u8>)>> {
        match expr {
            V2Expr::Path(span, segs) if segs.len() == 2 => {
                if let Some(bf) =
                    self.env
                        .lookup_bitflags(&self.owner_module, &segs[0], self.in_object_scope)
                {
                    let key = bf.canonical_key.clone();
                    if segs[1] == "None" {
                        return Ok(Some((key, 0, None)));
                    }
                    let &bit = bf.flags.get(&segs[1]).ok_or_else(|| {
                        Error::at(
                            *span,
                            format!("bitflags '{}' has no flag '{}'", bf.display_name, segs[1]),
                        )
                    })?;
                    return Ok(Some((key, bit_to_mask(bit), Some(bit))));
                }
                Ok(None)
            }
            V2Expr::Ident(_, name) => {
                if let Some(Binding::ConstBitflag {
                    key,
                    mask,
                    single_bit,
                }) = self.lookup_var(name)
                {
                    return Ok(Some((key.clone(), *mask, *single_bit)));
                }
                Ok(None)
            }
            V2Expr::Binary {
                lhs,
                op: op @ (V2BinOp::BitOr | V2BinOp::BitAnd | V2BinOp::BitXor),
                rhs,
                ..
            } => {
                let Some((key, lmask, _)) = self.try_eval_any_const_bitflag(lhs)? else {
                    return Ok(None);
                };
                let Some((rmask, _)) = self.try_eval_const_bitflag(rhs, &key)? else {
                    return Ok(None);
                };
                let mask = match op {
                    V2BinOp::BitOr => lmask | rmask,
                    V2BinOp::BitAnd => lmask & rmask,
                    V2BinOp::BitXor => lmask ^ rmask,
                    _ => unreachable!(),
                };
                Ok(Some((key, mask, single_bit_of_mask(mask))))
            }
            _ => Ok(None),
        }
    }

    /// A `const` item named `name`, unless a local binding shadows it.
    pub(super) fn const_item(&self, name: &str) -> Option<i64> {
        if self.lookup_var(name).is_some() {
            return None;
        }
        self.env
            .lookup_const(&self.owner_module, name, self.in_object_scope)
    }

    /// A 32-bit GUID operand: an integer literal, a `const` item, or a helper binding.
    pub(super) fn resolve_guid_expr(&self, expr: &V2Expr) -> Result<u32> {
        let (span, value) = match expr {
            V2Expr::Int(span, v) => (*span, *v),
            V2Expr::Ident(span, name) => {
                if let Some(Binding::Guid(guid)) = self.lookup_var(name) {
                    return Ok(*guid);
                }
                let value = self.const_item(name).ok_or_else(|| {
                    Error::at(
                        *span,
                        format!("expected a GUID literal or const, found '{name}'"),
                    )
                })?;
                (*span, value)
            }
            other => {
                return Err(Error::at(
                    other.span(),
                    "expected a GUID literal (0x...) or a const",
                ));
            }
        };
        u32::try_from(value).map_err(|_| Error::at(span, format!("GUID {value} out of u32 range")))
    }

    pub(super) fn try_const_i16_air_operand(&self, expr: &V2Expr) -> Result<Option<(String, i16)>> {
        match expr {
            V2Expr::Int(span, v) => {
                let n = i16::try_from(*v)
                    .map_err(|_| Error::at(*span, format!("integer {v} out of i16 range")))?;
                Ok(Some((n.to_string(), n)))
            }
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
                    Ok(Some((n.to_string(), n)))
                } else {
                    Ok(None)
                }
            }
            V2Expr::Ident(_, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    return match binding {
                        Binding::ConstI16 { air, value } => Ok(Some((air.clone(), *value))),
                        _ => Ok(None),
                    };
                }
                if self.is_self_ident(name)
                    || self.is_my_ident(name)
                    || (name == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf))
                {
                    return Ok(None);
                }
                if let Some(v) = self.const_item(name)
                    && let Ok(v) = i16::try_from(v)
                {
                    return Ok(Some((v.to_string(), v)));
                }
                if let Some(&val) = self.env.i16_params.get(name) {
                    return Ok(Some((format!("bcon.{name}"), val)));
                }
                Ok(None)
            }
            V2Expr::Field { .. } | V2Expr::Path(..) => match self.bcon_field_ref(expr) {
                Some(r) => {
                    let (air, val) = r?;
                    Ok(val.map(|v| (air, v)))
                }
                None => Ok(None),
            },
            _ => Ok(None),
        }
    }

    /// Resolves a compile-time bool for helper argument substitution and built-in compile-time
    /// options (`inform:`, `non_modal:`, `change_icon:`), including `Binding::ConstBool`.
    /// Unlike `try_eval_const_bool`, this is NEVER used to prune `if` branches inside helpers,
    /// so untaken branches remain type- and symbol-checked.
    pub(super) fn try_const_bool_arg(&self, expr: &V2Expr) -> Option<bool> {
        self.try_eval_const_bool_inner(expr, true)
    }

    /// Folds a condition built only from literal comparisons (integer literals, enum variants,
    /// constant helper bindings), bool literals, constant helper bools, and `!`/`&&`/`||`. Object
    /// `i16` params are excluded: they are runtime `bcon` reads.
    pub(super) fn try_fold_literal_cond(&self, expr: &V2Expr) -> Option<bool> {
        match expr {
            V2Expr::Bool(_, b) => Some(*b),
            V2Expr::Ident(_, name) => match self.lookup_var(name) {
                Some(Binding::ConstBool(b)) => Some(*b),
                _ => None,
            },
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.try_fold_literal_cond(inner).map(|b| !b),
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => match (
                self.try_fold_literal_cond(lhs),
                self.try_fold_literal_cond(rhs),
            ) {
                (Some(a), Some(b)) => Some(a && b),
                _ => None,
            },
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => match (
                self.try_fold_literal_cond(lhs),
                self.try_fold_literal_cond(rhs),
            ) {
                (Some(a), Some(b)) => Some(a || b),
                _ => None,
            },
            V2Expr::Binary { lhs, op, rhs, .. } => {
                let (lk, l) = self.literal_compare_operand(lhs)?;
                let (rk, r) = self.literal_compare_operand(rhs)?;
                // Mixed kinds (enum vs integer, two enums) take the normal, type-checked path.
                if lk != rk {
                    return None;
                }
                // Enums are only compared for equality; ordering takes the checked path (and
                // its error).
                if lk.is_some() && !matches!(op, V2BinOp::Eq | V2BinOp::Ne) {
                    return None;
                }
                match op {
                    V2BinOp::Eq => Some(l == r),
                    V2BinOp::Ne => Some(l != r),
                    V2BinOp::Lt => Some(l < r),
                    V2BinOp::Le => Some(l <= r),
                    V2BinOp::Gt => Some(l > r),
                    V2BinOp::Ge => Some(l >= r),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// `(enum key, value)` of a compile-time comparison operand (`None` key for plain integers).
    pub(super) fn literal_compare_operand(&self, expr: &V2Expr) -> Option<(Option<String>, i16)> {
        match expr {
            V2Expr::Path(_, segs) if segs.len() == 2 => {
                let e = self
                    .env
                    .lookup_enum(&self.owner_module, &segs[0], self.in_object_scope)?;
                let &v = e.variants.get(&segs[1])?;
                Some((Some(e.canonical_key.clone()), v))
            }
            V2Expr::Ident(_, name)
                if let Some(Binding::ConstEnum { key, value }) = self.lookup_var(name) =>
            {
                Some((Some(key.clone()), *value))
            }
            _ => {
                let (air, v) = self.try_const_i16_air_operand(expr).ok().flatten()?;
                air.parse::<i16>().ok().map(|_| (None, v))
            }
        }
    }

    /// Lowers a constant-folded-away branch into a discarded buffer so it is still type- and
    /// symbol-checked, then restores the lowering state.
    pub(super) fn check_dead_branch(&mut self, stmts: &[V2Stmt]) -> Result<()> {
        self.check_dead_branch_as(stmts, false)
    }

    /// [`Self::check_dead_branch`], optionally as a loop body (so `break`/`continue` resolve).
    pub(super) fn check_dead_branch_as(&mut self, stmts: &[V2Stmt], loop_body: bool) -> Result<()> {
        if stmts.is_empty() {
            return Ok(());
        }
        self.check_discarded(|c| {
            c.push_scope(if loop_body {
                ScopeFrame {
                    is_loop_body: true,
                    is_air_while: true,
                    owner_defer_id: c.current_defer_id,
                    ..ScopeFrame::block()
                }
            } else {
                ScopeFrame::block()
            });
            let r = c.lower_stmts(stmts);
            let p = c.pop_scope_and_run_cleanups();
            r.and(p)
        })
    }

    /// Checks a folded condition in full, including operands that short-circuiting skipped.
    pub(super) fn check_folded_cond(&mut self, cond: &V2Expr) -> Result<()> {
        self.check_discarded(|c| c.check_cond_operands(cond))
    }

    pub(super) fn check_cond_operands(&mut self, cond: &V2Expr) -> Result<()> {
        match cond {
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And | V2BinOp::Or,
                rhs,
                ..
            } => {
                self.check_cond_operands(lhs)?;
                self.check_cond_operands(rhs)
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self.check_cond_operands(inner),
            _ if self.try_eval_const_bool(cond).is_some() => Ok(()),
            _ => {
                let tmp = self.fresh("folded");
                self.emit_bool_into_local(cond, &tmp)
            }
        }
    }

    /// Runs `f`, then throws away everything it emitted or allocated (output, fresh names,
    /// dialog strings, defer ids), keeping only its result.
    pub(super) fn check_discarded(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        let out_len = self.em.mark();
        let indent = self.em.depth();
        let used_saved = self.used_saved_stack_obj;
        let sets_stack_obj = self.sets_stack_obj;
        let (event_local, sleep_span) = (self.event_local, self.sleep_span);
        let expansions = self.inline_expansions;
        let next_id = self.next_id;
        let defer_generation = self.defer_generation;
        let strings = self.env.dialog_strings.borrow().clone();
        self.checking_discarded += 1;
        let r = f(self);
        self.checking_discarded -= 1;
        self.em.truncate(out_len);
        self.em.set_depth(indent);
        self.used_saved_stack_obj = used_saved;
        self.sets_stack_obj = sets_stack_obj;
        (self.event_local, self.sleep_span) = (event_local, sleep_span);
        self.inline_expansions = expansions;
        self.next_id = next_id;
        self.defer_generation = defer_generation;
        *self.env.dialog_strings.borrow_mut() = strings;
        r
    }

    pub(super) fn try_eval_const_bool(&self, expr: &V2Expr) -> Option<bool> {
        self.try_eval_const_bool_inner(expr, false)
    }

    pub(super) fn try_eval_const_bool_inner(
        &self,
        expr: &V2Expr,
        allow_const_binding: bool,
    ) -> Option<bool> {
        match expr {
            V2Expr::Bool(_, b) => Some(*b),
            V2Expr::Ident(_, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    if allow_const_binding && let Binding::ConstBool(b) = binding {
                        return Some(*b);
                    }
                    return None;
                }
                if self.is_self_ident(name)
                    || self.is_my_ident(name)
                    || (name == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf))
                {
                    None
                } else {
                    self.env.bool_params.get(name).copied()
                }
            }
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => self
                .try_eval_const_bool_inner(inner, allow_const_binding)
                .map(|b| !b),
            V2Expr::Binary {
                lhs,
                op: V2BinOp::And,
                rhs,
                ..
            } => match self.try_eval_const_bool_inner(lhs, allow_const_binding) {
                Some(false) => Some(false),
                Some(true) => self.try_eval_const_bool_inner(rhs, allow_const_binding),
                None => None,
            },
            V2Expr::Binary {
                lhs,
                op: V2BinOp::Or,
                rhs,
                ..
            } => match self.try_eval_const_bool_inner(lhs, allow_const_binding) {
                Some(true) => Some(true),
                Some(false) => self.try_eval_const_bool_inner(rhs, allow_const_binding),
                None => None,
            },
            _ => None,
        }
    }

    pub(super) fn eval_const_bool_required(&self, expr: &V2Expr, span: Span) -> Result<bool> {
        self.try_const_bool_arg(expr).ok_or_else(|| {
            Error::at(
                span,
                "expected compile-time boolean ('true', 'false', or bool param)",
            )
        })
    }

    pub(super) fn eval_const_i16(&self, expr: &V2Expr) -> Result<i16> {
        match expr {
            V2Expr::Int(span, v) => i16::try_from(*v)
                .map_err(|_| Error::at(*span, format!("integer {v} out of i16 range"))),
            V2Expr::Unary {
                span,
                op: V2UnaryOp::Neg,
                inner,
            } => {
                let inner_v = self.eval_const_i16(inner)?;
                inner_v
                    .checked_neg()
                    .ok_or_else(|| Error::at(*span, "i16 negation overflow"))
            }
            // `Enum::Variant as i16`, `Flags::A | Flags::B as i16`, or an i16 constant.
            V2Expr::Unary {
                op: V2UnaryOp::AsI16,
                inner,
                ..
            } => {
                if let Some((_, mask, _)) = self.try_eval_any_const_bitflag(inner)? {
                    return Ok(mask);
                }
                if let Some(v) = self.try_eval_any_const_enum(inner)? {
                    return Ok(v);
                }
                self.eval_const_i16(inner)
            }
            V2Expr::Ident(span, name) => {
                if let Some(binding) = self.lookup_var(name) {
                    return match binding {
                        Binding::ConstI16 { value, .. } => Ok(*value),
                        _ => Err(Error::at(
                            *span,
                            format!("expected constant i16, found non-constant variable '{name}'"),
                        )),
                    };
                }
                if self.is_self_ident(name)
                    || self.is_my_ident(name)
                    || (name == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf))
                {
                    return Err(Error::at(
                        *span,
                        format!("expected constant i16, found '{name}'"),
                    ));
                }
                if let Some(v) = self.const_item(name) {
                    return i16::try_from(v).map_err(|_| {
                        Error::at(
                            *span,
                            format!("const '{name}' ({v:#x}) is a GUID, not an i16"),
                        )
                    });
                }
                self.env.i16_params.get(name).copied().ok_or_else(|| {
                    Error::at(*span, format!("expected constant i16, found '{name}'"))
                })
            }
            V2Expr::Field { span, .. } | V2Expr::Path(span, _) => match self.bcon_field_ref(expr) {
                Some(r) => r?.1.ok_or_else(|| {
                    Error::at(
                        *span,
                        "expected constant i16, found an 'extern bcon' entry (its value is only known at runtime)",
                    )
                }),
                None => Err(Error::at(*span, "expected constant i16 index")),
            },
            other => Err(Error::at(other.span(), "expected constant i16 index")),
        }
    }
}
