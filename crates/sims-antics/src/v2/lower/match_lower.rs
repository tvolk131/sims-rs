//! `match` lowering: statement/value/expression matches, arm lowering, range alternatives and the
//! comparison chain.

use super::*;

impl<'a> TreeLowerer<'a> {
    pub(super) fn lower_match(
        &mut self,
        span: Span,
        scrutinee: &V2Expr,
        arms: &[MatchArm],
    ) -> Result<()> {
        if let V2Expr::Call {
            span: cspan,
            callee,
            type_args,
            args,
        } = scrutinee
            && type_args.is_empty()
            && matches!(callee.as_ref(), V2Expr::Ident(_, fname) if fname == "dialog")
        {
            return self.lower_match_dialog(span, *cspan, args, arms);
        }
        self.lower_match_value(span, scrutinee, arms)
    }

    pub(super) fn lower_match_value(
        &mut self,
        span: Span,
        scrutinee: &V2Expr,
        arms: &[MatchArm],
    ) -> Result<()> {
        let arms: Vec<(Span, &MatchPattern, ArmBody<'_>)> = arms
            .iter()
            .map(|a| (a.span, &a.pattern, ArmBody::Stmts(a.body.as_slice())))
            .collect();
        self.lower_match_core(span, scrutinee, &arms, None)
    }

    /// Expression `match` into `dest` (a declared local), or into a fresh local that is returned.
    pub(super) fn lower_match_expr(
        &mut self,
        span: Span,
        scrutinee: &V2Expr,
        arms: &[MatchExprArm],
        dest: Option<&str>,
        may_diverge: bool,
    ) -> Result<(String, ValueType)> {
        let arms = arms
            .iter()
            .map(|a| Ok((a.span, &a.pattern, self.value_arm(&a.value, may_diverge)?)))
            .collect::<Result<Vec<(Span, &MatchPattern, ArmBody<'_>)>>>()?;
        let dest = match dest {
            Some(d) => d.to_owned(),
            None => {
                let d = self.fresh("mval");
                self.line(format!("let {d};"));
                d
            }
        };
        let mut result = MatchResult {
            dest: dest.clone(),
            ty: None,
            if_expr: false,
            may_diverge,
        };
        self.lower_match_core(span, scrutinee, &arms, Some(&mut result))?;
        let ty = result
            .ty
            .ok_or_else(|| Error::at(span, "'match' expression has no arms"))?;
        Ok((dest, ty))
    }

    /// A compile-time scrutinee (literal, `const`, constant parameter, enum variant), if any.
    pub(super) fn const_match_scrutinee(&self, scrutinee: &V2Expr) -> Option<(i16, ValueType)> {
        match scrutinee {
            V2Expr::Path(_, segs) if segs.len() == 2 => {
                let e = self
                    .env
                    .lookup_enum(&self.owner_module, &segs[0], self.in_object_scope)?;
                let v = *e.variants.get(&segs[1])?;
                Some((v, ValueType::Enum(e.canonical_key.clone())))
            }
            V2Expr::Ident(_, name) => match self.lookup_var(name) {
                Some(Binding::ConstEnum { key, value }) => {
                    Some((*value, ValueType::Enum(key.clone())))
                }
                // A helper argument folds only when it lowered to a literal: one bound to a
                // tunable `param` (a BCON read) stays a run-time test.
                Some(Binding::ConstI16 { air, value }) if air.parse::<i16>().is_ok() => {
                    Some((*value, ValueType::I16))
                }
                Some(_) => None,
                // `const` items fold; a tunable `param` does not.
                None => self
                    .const_item(name)
                    .and_then(|v| i16::try_from(v).ok())
                    .map(|v| (v, ValueType::I16)),
            },
            V2Expr::Int(..) | V2Expr::Unary { .. } if is_int_literal_expr(scrutinee) => self
                .eval_const_i16(scrutinee)
                .ok()
                .map(|v| (v, ValueType::I16)),
            _ => None,
        }
    }

    /// Lowers one arm body in its own block scope.
    pub(super) fn lower_match_arm(
        &mut self,
        body: ArmBody<'_>,
        result: Option<&mut MatchResult>,
    ) -> Result<()> {
        match body {
            ArmBody::Stmts(stmts) => {
                self.push_scope(ScopeFrame::block());
                let r = self.lower_stmts(stmts);
                let p = if stmts_always_return(
                    stmts,
                    self.env,
                    &self.owner_module,
                    self.in_object_scope,
                ) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
                r.and(p)
            }
            ArmBody::Value(expr) => {
                let result = result.expect("value arms always have a result");
                let dest = result.dest.clone();
                let is_bool_shape = matches!(
                    expr,
                    V2Expr::Binary {
                        op: V2BinOp::And
                            | V2BinOp::Or
                            | V2BinOp::Eq
                            | V2BinOp::Ne
                            | V2BinOp::Lt
                            | V2BinOp::Le
                            | V2BinOp::Gt
                            | V2BinOp::Ge
                            | V2BinOp::FlagTest,
                        ..
                    } | V2Expr::Unary {
                        op: V2UnaryOp::Not,
                        ..
                    }
                );
                let ty = if is_bool_shape {
                    self.emit_bool_into_local(expr, &dest)?;
                    ValueType::Bool
                } else if is_compound_arith_or_neg(expr) {
                    self.lower_arith_into_dest(expr, &dest, false)?
                } else if let V2Expr::Match {
                    span,
                    scrutinee,
                    arms,
                } = expr
                {
                    self.lower_match_expr(*span, scrutinee, arms, Some(&dest), result.may_diverge)?
                        .1
                } else if let V2Expr::If {
                    span,
                    cond,
                    then_value,
                    else_value,
                } = expr
                {
                    self.lower_if_expr(
                        *span,
                        cond,
                        then_value,
                        else_value,
                        Some(&dest),
                        result.may_diverge,
                    )?
                    .1
                } else {
                    let (air, ty) = self.lower_value_expr(expr)?;
                    self.emit_store(&dest, ":=", &air);
                    ty
                };
                match &result.ty {
                    None => result.ty = Some(ty),
                    Some(first) => {
                        // The first arm's type is the match's type, so the others must equal it
                        // (assignment-compatible is not enough: a generic handle is not a typed
                        // one).
                        if *first != ty {
                            return Err(Error::at(
                                expr.span(),
                                format!(
                                    "{} have different types: '{}' and '{}'",
                                    if result.if_expr {
                                        "'if' expression branches"
                                    } else {
                                        "'match' arms"
                                    },
                                    self.env.display_value_type(first),
                                    self.env.display_value_type(&ty)
                                ),
                            ));
                        }
                    }
                }
                Ok(())
            }
        }
    }

    pub(super) fn lower_match_core(
        &mut self,
        span: Span,
        scrutinee: &V2Expr,
        arms: &[(Span, &MatchPattern, ArmBody<'_>)],
        mut result: Option<&mut MatchResult>,
    ) -> Result<()> {
        let is_expr = result.is_some();
        let folded = self.const_match_scrutinee(scrutinee);
        let (val_air, val_ty) = match &folded {
            Some((v, ty)) => (v.to_string(), ty.clone()),
            None => self.lower_value_expr(scrutinee)?,
        };
        if !matches!(val_ty, ValueType::I16 | ValueType::Enum(_)) {
            return Err(Error::at(
                span,
                format!(
                    "'match' scrutinee must be 'dialog(...)', an enum, or 'i16', found '{}'",
                    self.env.display_value_type(&val_ty)
                ),
            ));
        }

        // Each arm's alternatives as inclusive ranges (`lo == hi` for a single value).
        let mut cases: Vec<MatchCase<'_>> = Vec::new();
        let mut wildcard: Option<ArmBody<'_>> = None;
        let mut seen_vals = BTreeSet::new();
        let mut alts_scratch: Vec<(i16, i16)> = Vec::new();

        for &(arm_span, pattern, body) in arms {
            if wildcard.is_some() {
                return Err(Error::at(
                    arm_span,
                    "unreachable match arm after '_' wildcard",
                ));
            }
            if let MatchPattern::Wildcard(_) = pattern {
                wildcard = Some(body);
                continue;
            }
            alts_scratch.clear();
            for alt in pattern_alternatives(pattern) {
                let range =
                    self.match_alt_range(alt, &val_ty, &mut seen_vals, &cases, &alts_scratch)?;
                alts_scratch.push(range);
            }
            cases.push(MatchCase {
                alts: alts_scratch.clone(),
                body,
            });
        }

        // Enforce exhaustiveness for enum matches when no `_` fallback is provided.
        let mut exhaustive_without_wildcard = false;
        if let ValueType::Enum(ref key) = val_ty
            && wildcard.is_none()
            && let Some(e) = self.env.all_enums.get(key)
        {
            for (vname, &vval) in &e.variants {
                if !seen_vals.contains(&vval) {
                    return Err(Error::at(
                        span,
                        format!(
                            "non-exhaustive match on enum '{}': missing variant '{}' (add the missing arm or a '_' fallback)",
                            e.display_name, vname
                        ),
                    ));
                }
            }
            exhaustive_without_wildcard = true;
        }
        if val_ty == ValueType::I16 && wildcard.is_none() {
            let mut ranges: Vec<(i32, i32)> = cases
                .iter()
                .flat_map(|c| c.alts.iter())
                .map(|&(lo, hi)| (i32::from(lo), i32::from(hi)))
                .collect();
            ranges.sort_unstable();
            let mut next = i32::from(i16::MIN);
            for (lo, hi) in ranges {
                if lo > next {
                    break;
                }
                next = next.max(hi + 1);
            }
            if next > i32::from(i16::MAX) {
                exhaustive_without_wildcard = true;
            } else if is_expr {
                return Err(Error::at(
                    span,
                    format!(
                        "non-exhaustive 'match' expression on 'i16': {next} is not covered (add a '_' arm)"
                    ),
                ));
            }
        }

        // A constant scrutinee selects one arm; the others are only checked.
        if let Some((v, _)) = folded {
            let selected = cases.iter().position(|c| c.contains(v));
            for (idx, case) in cases.iter().enumerate() {
                if Some(idx) == selected {
                    self.lower_match_arm(case.body, result.as_deref_mut())?;
                } else {
                    self.check_match_arm(case.body, result.as_deref_mut())?;
                }
            }
            if let Some(wc) = wildcard {
                if selected.is_none() {
                    self.lower_match_arm(wc, result.as_deref_mut())?;
                } else {
                    self.check_match_arm(wc, result.as_deref_mut())?;
                }
            }
            return Ok(());
        }

        // One arm and no test: the scrutinee is still read once (a read can fault), unless it
        // is a local or a literal.
        let single_arm = match wildcard {
            Some(wc) if cases.is_empty() => Some(wc),
            None if exhaustive_without_wildcard && cases.len() == 1 => Some(cases[0].body),
            _ => None,
        };
        if let Some(arm) = single_arm {
            let plain = val_air.parse::<i16>().is_ok()
                || (val_air.starts_with("__")
                    && val_air
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_'));
            if !plain {
                let l = self.fresh("match");
                self.emit_let(&l, &val_air);
            }
            return self.lower_match_arm(arm, result);
        }

        // Every test runs before any arm body, and an operand read has no side effects, so a
        // plain operand scrutinee is tested in place instead of copied.
        let scrut_local = if self.is_pure_single_operand(scrutinee) {
            val_air
        } else {
            let l = self.fresh("match");
            self.emit_let(&l, &val_air);
            l
        };
        self.emit_match_chain(
            &scrut_local,
            &cases,
            wildcard,
            exhaustive_without_wildcard,
            result,
        )
    }

    /// One non-`_` match alternative as an inclusive range, after its type, duplicate and
    /// overlap checks against every earlier alternative (`prior` arms and this arm's `current`).
    pub(super) fn match_alt_range(
        &self,
        pattern: &MatchPattern,
        val_ty: &ValueType,
        seen_vals: &mut BTreeSet<i16>,
        prior: &[MatchCase<'_>],
        current: &[(i16, i16)],
    ) -> Result<(i16, i16)> {
        let overlaps = |lo: i16, hi: i16| {
            prior
                .iter()
                .flat_map(|c| c.alts.iter())
                .chain(current)
                .any(|&(a, b)| a <= hi && lo <= b)
        };
        match pattern {
            MatchPattern::Int(pspan, v) => {
                if *val_ty != ValueType::I16 {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "integer pattern {v} cannot match '{}'",
                            self.env.display_value_type(val_ty)
                        ),
                    ));
                }
                if !seen_vals.insert(*v) || overlaps(*v, *v) {
                    return Err(Error::at(
                        *pspan,
                        format!("duplicate match arm for value {v}"),
                    ));
                }
                Ok((*v, *v))
            }
            MatchPattern::Range(pspan, lo, hi) => {
                if *val_ty != ValueType::I16 {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "range pattern {lo}..={hi} cannot match '{}'",
                            self.env.display_value_type(val_ty)
                        ),
                    ));
                }
                if overlaps(*lo, *hi) {
                    return Err(Error::at(
                        *pspan,
                        format!("range pattern {lo}..={hi} overlaps an earlier match arm"),
                    ));
                }
                Ok((*lo, *hi))
            }
            MatchPattern::Path(pspan, segs) => {
                let ValueType::Enum(expected_key) = val_ty else {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "enum pattern '{}' cannot match '{}'",
                            segs.join("::"),
                            self.env.display_value_type(val_ty)
                        ),
                    ));
                };
                if segs.len() != 2 {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "expected 'Enum::Variant' pattern, found '{}'",
                            segs.join("::")
                        ),
                    ));
                }
                let e = self
                    .env
                    .lookup_enum(&self.owner_module, &segs[0], self.in_object_scope)
                    .ok_or_else(|| Error::at(*pspan, format!("unknown enum '{}'", segs[0])))?;
                if &e.canonical_key != expected_key {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "mismatched enum pattern '{}': expected '{}'",
                            e.display_name,
                            self.env.display_value_type(val_ty)
                        ),
                    ));
                }
                let Some(&v) = e.variants.get(&segs[1]) else {
                    return Err(Error::at(
                        *pspan,
                        format!("enum '{}' has no variant '{}'", e.display_name, segs[1]),
                    ));
                };
                if !seen_vals.insert(v) {
                    return Err(Error::at(
                        *pspan,
                        format!("duplicate match arm for '{}::{}'", e.display_name, segs[1]),
                    ));
                }
                Ok((v, v))
            }
            // A `const` item matches its value.
            MatchPattern::Ident(pspan, name) => {
                let Some(raw) = self.const_item(name) else {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "unexpected identifier pattern '{name}' in match (use 'Enum::Variant', an integer literal or const, or '_')"
                        ),
                    ));
                };
                let v = i16::try_from(raw).map_err(|_| {
                    Error::at(
                        *pspan,
                        format!("const pattern '{name}' = {raw} is outside the i16 range"),
                    )
                })?;
                if *val_ty != ValueType::I16 {
                    return Err(Error::at(
                        *pspan,
                        format!(
                            "const pattern '{name}' cannot match '{}'",
                            self.env.display_value_type(val_ty)
                        ),
                    ));
                }
                if !seen_vals.insert(v) || overlaps(v, v) {
                    return Err(Error::at(
                        *pspan,
                        format!("duplicate match arm for value {v} ('{name}')"),
                    ));
                }
                Ok((v, v))
            }
            // The parser flattens OR-patterns and keeps `_` out of them.
            MatchPattern::Wildcard(pspan) | MatchPattern::Or(pspan, _) => Err(Error::at(
                *pspan,
                "'_' and nested OR-patterns cannot be OR-pattern alternatives",
            )),
        }
    }

    /// Emits the test chain for `cases` (then `wildcard`) on `scrut`. A case whose
    /// alternatives are all single compares, and whose body and later arms cannot leave an
    /// enclosing loop, tests them in a condition-free single-pass loop:
    /// `while true { if !a { if !b { <later arms>; break; } } <body>; break; }`. Other
    /// multi-compare cases compute a flag first, inside the previous arm's `else`.
    pub(super) fn emit_match_chain(
        &mut self,
        scrut: &str,
        cases: &[MatchCase<'_>],
        wildcard: Option<ArmBody<'_>>,
        exhaustive: bool,
        mut result: Option<&mut MatchResult>,
    ) -> Result<()> {
        if cases.is_empty() {
            return match wildcard {
                Some(wc) => self.lower_match_arm(wc, result),
                None => Ok(()),
            };
        }
        if wildcard.is_none() && exhaustive && cases.len() == 1 {
            return self.lower_match_arm(cases[0].body, result);
        }
        let last_idx = cases.len() - 1;
        let mut extra_closes = 0usize;
        let mut tests: Vec<Option<String>> = Vec::new();
        for (idx, case) in cases.iter().enumerate() {
            tests.clear();
            tests.extend(
                case.alts
                    .iter()
                    .map(|&(lo, hi)| single_range_test(scrut, lo, hi)),
            );
            let is_last_exhaustive = exhaustive && wildcard.is_none() && idx == last_idx && idx > 0;
            if is_last_exhaustive {
                self.line("} else {".to_owned());
            } else if let [Some(cond)] = tests.as_slice() {
                if idx == 0 {
                    self.line(format!("if {cond} {{"));
                } else {
                    self.line(format!("}} else if {cond} {{"));
                }
            } else {
                if idx > 0 {
                    self.line("} else {".to_owned());
                    self.em.indent();
                    extra_closes += 1;
                }
                if tests.len() > 1
                    && tests.iter().all(Option::is_some)
                    && self.match_loop_form_ok(&cases[idx..], wildcard)
                {
                    self.line("while true {".to_owned());
                    self.em.indent();
                    for cond in tests.iter().flatten() {
                        self.line(format!("if {} {{", negate_air_cond(cond)));
                        self.em.indent();
                    }
                    let rest = self.emit_match_chain(
                        scrut,
                        &cases[idx + 1..],
                        wildcard,
                        exhaustive,
                        result.as_deref_mut(),
                    );
                    self.line("break;".to_owned());
                    for _ in &tests {
                        self.em.dedent();
                        self.line("}".to_owned());
                    }
                    rest?;
                    let r = self.lower_match_arm(case.body, result);
                    self.line("break;".to_owned());
                    self.em.dedent();
                    self.line("}".to_owned());
                    self.close_match_chain(extra_closes);
                    return r;
                }
                let flag = self.fresh("in_range");
                self.line(format!("let {flag} = 0;"));
                for (&(lo, hi), test) in case.alts.iter().zip(&tests) {
                    if let Some(cond) = test {
                        self.line(format!("if {cond} {{"));
                        self.em.indent();
                    } else {
                        self.line(format!("if {scrut} >= {lo} {{"));
                        self.em.indent();
                        self.line(format!("if {scrut} <= {hi} {{"));
                        self.em.indent();
                    }
                    self.emit_store(&flag, ":=", "1");
                    if test.is_none() {
                        self.em.dedent();
                        self.line("}".to_owned());
                    }
                    self.em.dedent();
                    self.line("}".to_owned());
                }
                self.line(format!("if {flag} != 0 {{"));
            }
            self.em.indent();
            let r = self.lower_match_arm(case.body, result.as_deref_mut());
            self.em.dedent();
            r?;
        }
        if let Some(wc) = wildcard {
            self.line("} else {".to_owned());
            self.em.indent();
            let r = self.lower_match_arm(wc, result);
            self.em.dedent();
            self.line("}".to_owned());
            self.close_match_chain(extra_closes);
            r
        } else {
            self.line("}".to_owned());
            self.close_match_chain(extra_closes);
            Ok(())
        }
    }

    /// Whether `cases` (and `wildcard`) may sit inside a single-pass loop (see
    /// [`Self::single_pass_loop_ok`]).
    pub(super) fn match_loop_form_ok(
        &self,
        cases: &[MatchCase<'_>],
        wildcard: Option<ArmBody<'_>>,
    ) -> bool {
        let bodies: Vec<&[V2Stmt]> = cases
            .iter()
            .map(|c| c.body)
            .chain(wildcard)
            .map(arm_stmts)
            .collect();
        self.single_pass_loop_ok(&bodies)
    }

    /// Checks an arm that a constant scrutinee never selects (lowered, then discarded).
    pub(super) fn check_match_arm(
        &mut self,
        body: ArmBody<'_>,
        result: Option<&mut MatchResult>,
    ) -> Result<()> {
        match body {
            ArmBody::Stmts(stmts) => self.check_dead_branch(stmts),
            ArmBody::Value(_) => {
                let result = result.expect("value arms always have a result");
                let mut scratch = MatchResult {
                    dest: result.dest.clone(),
                    ty: result.ty.clone(),
                    if_expr: result.if_expr,
                    may_diverge: result.may_diverge,
                };
                self.check_discarded(|c| c.lower_match_arm(body, Some(&mut scratch)))?;
                if result.ty.is_none() {
                    result.ty = scratch.ty;
                }
                Ok(())
            }
        }
    }

    pub(super) fn close_match_chain(&mut self, extra_closes: usize) {
        for _ in 0..extra_closes {
            self.em.dedent();
            self.line("}".to_owned());
        }
    }
}
