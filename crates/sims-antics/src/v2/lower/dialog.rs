//! Dialogs: `match dialog(...)`, dialog string arguments, option parsing and `dialog(...)`
//! primitives.

use super::*;

impl<'a> TreeLowerer<'a> {
    pub(super) fn lower_match_dialog(
        &mut self,
        match_span: Span,
        call_span: Span,
        args: &[CallArg],
        arms: &[MatchArm],
    ) -> Result<()> {
        let opts = self.parse_dialog_options(args, call_span, true)?;
        if (opts.effective_flags & 1) != 0 {
            return Err(Error::at(
                call_span,
                "'match dialog(...)' cannot set the inform bit ('inform: true' or 'flags' bit 0), because informational dialogs do not wait for a button choice",
            ));
        }

        let arms_have_third = arms
            .iter()
            .flat_map(|a| pattern_alternatives(&a.pattern))
            .any(|p| {
                matches!(
                    p,
                    MatchPattern::Ident(_, n) if matches!(n.as_str(), "Button3" | "Cancel")
                ) || matches!(p, MatchPattern::Int(_, 2))
            });
        let is_three_way = match opts.kind_num {
            Some(1) => false,
            Some(2) => true,
            Some(other) => {
                return Err(Error::at(
                    call_span,
                    format!(
                        "'match dialog(...)' only supports 2-button ('yes_no') or 3-button ('yes_no_cancel') dialogs, found kind {other}"
                    ),
                ));
            }
            None => opts.button_count == 3 || arms_have_third,
        };
        if let Some(tuple_len) = opts.buttons_tuple_len {
            let expected_len = if is_three_way { 3 } else { 2 };
            if tuple_len != expected_len {
                return Err(Error::at(
                    call_span,
                    format!(
                        "dialog 'buttons:' tuple has {tuple_len} entries, but {} dialog requires exactly {expected_len}",
                        if is_three_way {
                            "'yes_no_cancel'"
                        } else {
                            "'yes_no'"
                        }
                    ),
                ));
            }
        }

        // Map arm patterns to button indices: 0 (Button1/Yes/0), 1 (Button2/No/1), 2 (Button3/Cancel/2).
        let mut arm_bodies: [Option<&[V2Stmt]>; 3] = [None, None, None];
        let mut wildcard_body: Option<&[V2Stmt]> = None;
        for arm in arms {
            if wildcard_body.is_some() {
                return Err(Error::at(
                    arm.span,
                    "unreachable arm after '_' wildcard in 'match dialog'",
                ));
            }
            for alt in pattern_alternatives(&arm.pattern) {
                let slot: Option<usize> = match alt {
                    MatchPattern::Wildcard(_) => {
                        wildcard_body = Some(arm.body.as_slice());
                        None
                    }
                    MatchPattern::Range(pspan, ..) => {
                        return Err(Error::at(
                            *pspan,
                            "range patterns cannot match a 'dialog(...)' result",
                        ));
                    }
                    MatchPattern::Ident(pspan, name) => match name.as_str() {
                        "Button1" | "Yes" => Some(0),
                        "Button2" | "No" => Some(1),
                        "Button3" | "Cancel" => {
                            if !is_three_way {
                                return Err(Error::at(
                                    *pspan,
                                    format!(
                                        "arm '{name}' is invalid for 2-button 'yes_no' dialog (expected Button1/Yes or Button2/No)"
                                    ),
                                ));
                            }
                            Some(2)
                        }
                        other => {
                            return Err(Error::at(
                                *pspan,
                                format!(
                                    "invalid 'match dialog' arm '{other}' (expected Button1/Yes, Button2/No{}{})",
                                    if is_three_way { ", Button3/Cancel" } else { "" },
                                    ", 0/1/2, or _"
                                ),
                            ));
                        }
                    },
                    MatchPattern::Int(pspan, v) => match *v {
                        0 => Some(0),
                        1 => Some(1),
                        2 if is_three_way => Some(2),
                        other => {
                            return Err(Error::at(
                                *pspan,
                                format!(
                                    "invalid 'match dialog' button index {other} (expected {})",
                                    if is_three_way { "0, 1, or 2" } else { "0 or 1" }
                                ),
                            ));
                        }
                    },
                    MatchPattern::Or(pspan, _) => {
                        return Err(Error::at(*pspan, "nested OR-pattern in 'match dialog'"));
                    }
                    MatchPattern::Path(pspan, segs) => {
                        return Err(Error::at(
                            *pspan,
                            format!(
                                "unexpected path pattern '{}' in 'match dialog' (expected Button1/Button2/Button3, Yes/No/Cancel, 0/1/2, or _)",
                                segs.join("::")
                            ),
                        ));
                    }
                };
                if let Some(idx) = slot {
                    if arm_bodies[idx].is_some() {
                        return Err(Error::at(
                            arm.span,
                            format!("duplicate arm for button {} in 'match dialog'", idx + 1),
                        ));
                    }
                    arm_bodies[idx] = Some(arm.body.as_slice());
                }
            }
        }

        let required = if is_three_way { 3 } else { 2 };
        for (i, slot) in arm_bodies.iter_mut().enumerate().take(required) {
            if slot.is_none() {
                if let Some(wc) = wildcard_body {
                    *slot = Some(wc);
                } else {
                    return Err(Error::at(
                        match_span,
                        format!(
                            "non-exhaustive 'match dialog': missing arm for Button{} (add the missing arm or a '_' fallback)",
                            i + 1
                        ),
                    ));
                }
            }
        }

        // Build effective dialog() args (injecting kind: and temp: 0 for 3-way dialogs if omitted).
        let mut effective_args: Vec<CallArg> = args.to_vec();
        if opts.kind_num.is_none() {
            effective_args.insert(
                0,
                CallArg {
                    span: call_span,
                    name: Some("kind".to_owned()),
                    value: V2Expr::Ident(
                        call_span,
                        if is_three_way {
                            "yes_no_cancel".to_owned()
                        } else {
                            "yes_no".to_owned()
                        },
                    ),
                },
            );
        }
        let temp_slot = opts.effective_temp_slot;
        let save_implicit_temp = is_three_way && !opts.explicit_temp_specified;
        if save_implicit_temp {
            effective_args.push(CallArg {
                span: call_span,
                name: Some("temp".to_owned()),
                value: V2Expr::Int(call_span, i64::from(temp_slot)),
            });
        }

        let saved_temp_local = if save_implicit_temp {
            let s = self.fresh("dlg_t0");
            self.line(format!("let {s} = temp[{temp_slot}];"));
            Some(s)
        } else {
            None
        };

        let dlg_air = self.format_dialog(&effective_args, call_span)?;
        let body0 = arm_bodies[0].unwrap();
        self.line(format!("if {dlg_air} {{"));
        self.indent += 1;
        self.push_scope(ScopeFrame::block());
        let r0 = self.lower_stmts(body0);
        let p0 = if stmts_always_return(body0, self.env, &self.owner_module, self.in_object_scope) {
            self.pop_scope_discard_cleanups();
            Ok(())
        } else {
            self.pop_scope_and_run_cleanups()
        };
        self.indent -= 1;
        r0.and(p0)?;

        let body1 = arm_bodies[1].unwrap();
        if !is_three_way {
            self.line("} else {".to_owned());
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r1 = self.lower_stmts(body1);
            let p1 =
                if stmts_always_return(body1, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            self.line("}".to_owned());
            r1.and(p1)
        } else if let Some(saved_t) = saved_temp_local {
            // Restore implicit temp[0] BEFORE executing Button2/Button3 user code or defers!
            self.line("} else {".to_owned());
            self.indent += 1;
            let choice = self.fresh("dlg_choice");
            self.line(format!("let {choice} = temp[{temp_slot}];"));
            self.line(format!("temp[{temp_slot}] := {saved_t};"));
            self.line(format!("if {choice} == 0 {{"));
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r1 = self.lower_stmts(body1);
            let p1 =
                if stmts_always_return(body1, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            r1.and(p1)?;
            let body2 = arm_bodies[2].unwrap();
            self.line("} else {".to_owned());
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r2 = self.lower_stmts(body2);
            let p2 =
                if stmts_always_return(body2, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            self.line("}".to_owned());
            self.indent -= 1;
            self.line("}".to_owned());
            r2.and(p2)
        } else {
            self.line(format!("}} else if temp[{temp_slot}] == 0 {{"));
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r1 = self.lower_stmts(body1);
            let p1 =
                if stmts_always_return(body1, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            r1.and(p1)?;
            let body2 = arm_bodies[2].unwrap();
            self.line("} else {".to_owned());
            self.indent += 1;
            self.push_scope(ScopeFrame::block());
            let r2 = self.lower_stmts(body2);
            let p2 =
                if stmts_always_return(body2, self.env, &self.owner_module, self.in_object_scope) {
                    self.pop_scope_discard_cleanups();
                    Ok(())
                } else {
                    self.pop_scope_and_run_cleanups()
                };
            self.indent -= 1;
            self.line("}".to_owned());
            r2.and(p2)
        }
    }

    pub(super) fn resolve_dialog_str_arg(
        &self,
        expr: &V2Expr,
        field: &str,
        span: Span,
    ) -> Result<String> {
        match expr {
            V2Expr::Str(sspan, text) => {
                let interned = self.env.dialog_strings.borrow_mut().intern(text, *sspan)?;
                Ok(format!("{field}: str.{interned}"))
            }
            V2Expr::Ident(ispan, sname) => {
                if !self.env.string_names.contains_key(sname) {
                    return Err(Error::at(
                        *ispan,
                        format!(
                            "unknown string entry '{sname}' (declare it in 'strings {{ ... }}' or pass an inline string literal \"...\")"
                        ),
                    ));
                }
                Ok(format!("{field}: str.{sname}"))
            }
            V2Expr::Int(_, n) => Ok(format!("{field}: {n}")),
            _ => Err(Error::at(span, format!("invalid dialog {field}"))),
        }
    }

    pub(super) fn parse_dialog_options(
        &self,
        args: &[CallArg],
        span: Span,
        for_match: bool,
    ) -> Result<ParsedDialogOptions> {
        let mut kind_num: Option<u8> = None;
        let mut effective_flags: u8 = 0;
        let mut explicit_temp_specified = false;
        let mut button_count: usize = 0;
        let mut buttons_tuple_len: Option<usize> = None;
        let mut saw_buttons_tuple = false;
        let mut saw_explicit_button = false;

        for arg in args {
            let Some(k) = arg.name.as_deref() else {
                return Err(Error::at(arg.span, "dialog() arguments must be named"));
            };
            match k {
                "kind" => match &arg.value {
                    V2Expr::Ident(_, s) => {
                        kind_num = Some(match s.as_str() {
                            "ok" | "message" => 0,
                            "yes_no" => 1,
                            "yes_no_cancel" | "three_choice" => 2,
                            "text_entry" => 3,
                            "icon" => 4,
                            other => {
                                return Err(Error::at(
                                    arg.span,
                                    format!("unknown dialog kind '{other}'"),
                                ));
                            }
                        });
                    }
                    V2Expr::Int(_, n) => {
                        let kn = u8::try_from(*n).map_err(|_| {
                            Error::at(arg.span, format!("dialog kind {n} out of u8 range"))
                        })?;
                        kind_num = Some(kn);
                    }
                    _ => return Err(Error::at(arg.span, "invalid dialog kind")),
                },
                "title" | "message" | "text" | "mode4_str" => {}
                "button1" | "button2" | "button3" => {
                    if saw_buttons_tuple {
                        return Err(Error::at(
                            arg.span,
                            "cannot combine 'buttons: (...)' with explicit 'button1'/'button2'/'button3'",
                        ));
                    }
                    saw_explicit_button = true;
                    let idx = match k {
                        "button1" => 1,
                        "button2" => 2,
                        _ => 3,
                    };
                    button_count = button_count.max(idx);
                }
                "buttons" => {
                    if saw_explicit_button || saw_buttons_tuple {
                        return Err(Error::at(
                            arg.span,
                            "duplicate 'buttons:' or conflict with 'button1'/'button2'/'button3'",
                        ));
                    }
                    saw_buttons_tuple = true;
                    let V2Expr::Tuple(tspan, elems) = &arg.value else {
                        return Err(Error::at(
                            arg.span,
                            "dialog 'buttons:' expects a tuple of 1 to 3 strings, e.g. buttons: (\"Yes\", \"No\")",
                        ));
                    };
                    if elems.is_empty() || elems.len() > 3 {
                        return Err(Error::at(
                            *tspan,
                            format!(
                                "dialog 'buttons:' tuple must have 1 to 3 entries, found {}",
                                elems.len()
                            ),
                        ));
                    }
                    buttons_tuple_len = Some(elems.len());
                    button_count = elems.len();
                }
                "flags" => {
                    let n = self.eval_const_i16(&arg.value)?;
                    let raw = u8::try_from(n).map_err(|_| {
                        Error::at(arg.span, format!("dialog flags {n} out of u8 range"))
                    })?;
                    effective_flags |= raw;
                    if (raw & 0x70) != 0 {
                        explicit_temp_specified = true;
                    }
                }
                "inform" => {
                    if self.eval_const_bool_required(&arg.value, arg.span)? {
                        effective_flags |= 1;
                    }
                }
                "non_modal" => {
                    if self.eval_const_bool_required(&arg.value, arg.span)? {
                        effective_flags |= 0x80;
                    }
                }
                "icon_mode" => {
                    let n = self.eval_const_i16(&arg.value)?;
                    let mode = u8::try_from(n)
                        .ok()
                        .filter(|&m| m <= 7)
                        .ok_or_else(|| Error::at(arg.span, "dialog icon_mode must be in 0..=7"))?;
                    effective_flags = (effective_flags & !(7 << 1)) | (mode << 1);
                }
                "temp" => {
                    let n = self.eval_const_i16(&arg.value)?;
                    let t = u8::try_from(n)
                        .ok()
                        .filter(|&m| m <= 7)
                        .ok_or_else(|| Error::at(arg.span, "dialog temp must be in 0..=7"))?;
                    effective_flags = (effective_flags & !(7 << 4)) | (t << 4);
                    explicit_temp_specified = true;
                }
                other => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown dialog() argument '{other}'"),
                    ));
                }
            }
        }

        if !for_match && let Some(tuple_len) = buttons_tuple_len {
            let eff_kind = kind_num.unwrap_or(0) & 0x7f;
            let expected = match eff_kind {
                0 => Some((1, "'ok'")),
                1 => Some((2, "'yes_no'")),
                2 => Some((3, "'yes_no_cancel'")),
                3 => Some((1, "'text_entry'")),
                4 => Some((1, "'icon'")),
                _ => None,
            };
            match expected {
                Some((exp, label)) if tuple_len != exp => {
                    return Err(Error::at(
                        span,
                        format!(
                            "dialog 'buttons:' tuple has {tuple_len} entries, but {label} dialog requires exactly {exp}"
                        ),
                    ));
                }
                None => {
                    return Err(Error::at(
                        span,
                        format!(
                            "dialog kind {eff_kind} does not accept custom 'buttons:' tuple labels"
                        ),
                    ));
                }
                _ => {}
            }
        }

        Ok(ParsedDialogOptions {
            kind_num,
            effective_flags,
            effective_temp_slot: (effective_flags >> 4) & 7,
            explicit_temp_specified,
            button_count,
            buttons_tuple_len,
        })
    }

    pub(super) fn format_dialog(&self, args: &[CallArg], span: Span) -> Result<String> {
        let _opts = self.parse_dialog_options(args, span, false)?;
        let mut parts = Vec::new();
        for arg in args {
            let Some(k) = arg.name.as_deref() else {
                return Err(Error::at(arg.span, "dialog() arguments must be named"));
            };
            match k {
                "kind" => match &arg.value {
                    V2Expr::Ident(_, s) => parts.push(format!("kind: {s}")),
                    V2Expr::Int(_, n) => parts.push(format!("kind: {n}")),
                    _ => return Err(Error::at(arg.span, "invalid dialog kind")),
                },
                "title" | "message" | "text" | "mode4_str" | "button1" | "button2" | "button3" => {
                    parts.push(self.resolve_dialog_str_arg(&arg.value, k, arg.span)?);
                }
                "buttons" => {
                    let V2Expr::Tuple(_, elems) = &arg.value else {
                        unreachable!("validated in parse_dialog_options");
                    };
                    let btn_fields = ["button1", "button2", "button3"];
                    for (idx, elem) in elems.iter().enumerate() {
                        parts.push(self.resolve_dialog_str_arg(
                            elem,
                            btn_fields[idx],
                            elem.span(),
                        )?);
                    }
                }
                "temp" | "icon_mode" | "flags" => {
                    let n = self.eval_const_i16(&arg.value)?;
                    parts.push(format!("{k}: {n}"));
                }
                "inform" | "non_modal" => {
                    let b = self.eval_const_bool_required(&arg.value, arg.span)?;
                    parts.push(format!("{k}: {b}"));
                }
                other => {
                    return Err(Error::at(
                        arg.span,
                        format!("unknown dialog() argument '{other}'"),
                    ));
                }
            }
        }
        Ok(format!("dialog({})", parts.join(", ")))
    }
}
