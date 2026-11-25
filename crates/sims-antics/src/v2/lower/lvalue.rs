//! Places: l-value resolution, read-only checks, assignment and reads of l-values and frame local
//! slots.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// Slots (`x.hand`, `x.slot[i]`) and rooms (`x.room`) of Sims, `self`, and object handles
    /// can be read but not written: the VM's slot and room scopes have no write path.
    pub(super) fn reject_read_only_sim_slot(&self, target: &V2Expr) -> Result<()> {
        // A read-only named engine word (`container`, object data word 2): the engine traps a
        // store of a different value. A state of the same name wins over the engine word.
        if let V2Expr::Field { span, base, field } = target
            && let V2Expr::Ident(_, name) = base.as_ref()
            && object_data_word(field).is_some_and(|w| !w.writable)
            && (self.is_my_ident(name)
                || self.is_self_ident(name)
                || self.object_handle_air(name).is_some())
            && !self.handle_has_state(name, field)
        {
            return Err(Error::at(
                *span,
                format!(
                    "cannot assign to '{name}.{field}': it is read-only (the engine refuses a script store into this word)"
                ),
            ));
        }
        let (span, name, what) = match target {
            V2Expr::Field { span, base, field } if field == "hand" || field == "room" => {
                match base.as_ref() {
                    V2Expr::Ident(_, n) => (*span, n, field.as_str()),
                    _ => return Ok(()),
                }
            }
            V2Expr::Index { span, base, .. } => match base.as_ref() {
                V2Expr::Field { base, field, .. } if field == "slot" => match base.as_ref() {
                    V2Expr::Ident(_, n) => (*span, n, "slot[i]"),
                    _ => return Ok(()),
                },
                _ => return Ok(()),
            },
            _ => return Ok(()),
        };
        if self.is_my_ident(name)
            || self.is_self_ident(name)
            || self.object_handle_air(name).is_some()
        {
            return Err(Error::at(
                span,
                format!("'{name}.{what}' is read-only (slots and rooms cannot be written)"),
            ));
        }
        Ok(())
    }

    /// Resolves an assignable place. A foreign cell of `stack_obj` itself is the direct `stack.`
    /// cell: nothing is switched around it.
    pub(super) fn resolve_lvalue(&mut self, expr: &V2Expr) -> Result<LValue> {
        Ok(match self.resolve_lvalue_inner(expr)? {
            LValue::ForeignAttr {
                obj_air,
                attr_index,
                ty,
            } if obj_air == "stack_obj" => LValue::Direct {
                air: format!("stack.attr[{attr_index}]"),
                ty,
            },
            LValue::ForeignCell {
                obj_air,
                cell,
                setup: None,
                ty,
            } if obj_air == "stack_obj" => LValue::Direct { air: cell, ty },
            other => other,
        })
    }

    pub(super) fn resolve_lvalue_inner(&mut self, expr: &V2Expr) -> Result<LValue> {
        match expr {
            V2Expr::Deref { span, inner } => {
                let V2Expr::Ident(_, pname) = inner.as_ref() else {
                    return Err(Error::at(
                        *span,
                        "only '*param' dereference on '&mut' parameters is supported",
                    ));
                };
                match self.lookup_var(pname).cloned() {
                    Some(Binding::MutRef { target, .. }) => Ok(target),
                    Some(_) => Err(Error::at(
                        *span,
                        format!("cannot dereference non-'&mut' variable '{pname}'"),
                    )),
                    None => Err(Error::at(*span, format!("unknown variable '{pname}'"))),
                }
            }
            V2Expr::Ident(span, name) => match self.lookup_var(name).cloned() {
                Some(Binding::Local { air_name, ty }) => Ok(LValue::Direct { air: air_name, ty }),
                Some(Binding::BhavParam { .. }) => Err(Error::at(
                    *span,
                    format!("cannot assign directly to immutable parameter '{name}'"),
                )),
                Some(Binding::MutRef { .. }) => Err(Error::at(
                    *span,
                    format!("use '*{name} = ...' to assign through '&mut' parameter '{name}'"),
                )),
                Some(_) => Err(Error::at(*span, format!("'{name}' is not assignable"))),
                None if name == "stack_obj" => Ok(LValue::Direct {
                    air: "stack_obj".to_owned(),
                    ty: ValueType::ObjectId,
                }),
                None => {
                    if self.env.i16_params.contains_key(name)
                        || self.env.bool_params.contains_key(name)
                    {
                        Err(Error::at(
                            *span,
                            format!("cannot assign to compile-time param '{name}'"),
                        ))
                    } else {
                        Err(Error::at(*span, format!("unknown variable '{name}'")))
                    }
                }
            },
            // A neighbor cell (`h.neighbor.x`, `h.neighbor.person[i]`) is a value with no word
            // behind it (`sims-vm` `memory.rs` scopes 24 and 32).
            V2Expr::Field { span, base, .. } | V2Expr::Index { span, base, .. }
                if neighbor_place_root(base).is_some() =>
            {
                Err(Error::at(
                    *span,
                    "a neighbor cell is read-only (the engine hands back its value, not a word)",
                ))
            }
            V2Expr::Field { span, base, field } => {
                if let Some(r) = self.bcon_field_ref(expr) {
                    r?;
                    return Err(Error::at(
                        *span,
                        "BCON constants are read-only (cannot assign to a BCON table entry)",
                    ));
                }
                if let V2Expr::Ident(_, bname) = base.as_ref()
                    && let Some(word) = definition_field(field)
                    && !self.handle_has_state(bname, field)
                {
                    return Err(Error::at(
                        *span,
                        format!(
                            "'{bname}.{field}' is read-only (word {word} of the object's definition)"
                        ),
                    ));
                }
                // `sim.motive.energy`, `sim.skill.cooking`, `self.<state>`, `self.graphic`, etc.
                if let V2Expr::Field {
                    base: root,
                    field: group,
                    ..
                } = base.as_ref()
                    && let V2Expr::Ident(rspan, rname) = root.as_ref()
                {
                    if rname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *rspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                    if self.is_my_ident(rname) {
                        return match group.as_str() {
                            "motive" => {
                                let _ = motive_by_name(field).ok_or_else(|| {
                                    Error::at(*span, format!("unknown motive '{field}'"))
                                })?;
                                Ok(LValue::Direct {
                                    air: format!("my.motive.{field}"),
                                    ty: ValueType::I16,
                                })
                            }
                            "skill" => {
                                let _ = skill_by_name(field).ok_or_else(|| {
                                    Error::at(*span, format!("unknown skill '{field}'"))
                                })?;
                                Ok(LValue::Direct {
                                    air: format!("my.person.{field}"),
                                    ty: ValueType::I16,
                                })
                            }
                            "personality" => Ok(LValue::Direct {
                                air: format!("my.person[{}]", personality_trait(*span, field)?),
                                ty: ValueType::I16,
                            }),
                            other => Err(Error::at(
                                *span,
                                format!("unknown Sim member '{rname}.{other}.{field}'"),
                            )),
                        };
                    }
                    if self.is_self_ident(rname) {
                        return Err(Error::at(
                            *span,
                            format!(
                                "'self' is an Object, not a Sim (cannot access 'self.{group}.{field}')"
                            ),
                        ));
                    }
                    if let Some((obj_air, maybe_sim)) = self.object_handle_air(rname) {
                        if !maybe_sim {
                            return Err(Error::at(
                                *span,
                                format!(
                                    "'{rname}' is a typed object handle, not a Sim (cannot access '{rname}.{group}.{field}')"
                                ),
                            ));
                        }
                        let cell = match group.as_str() {
                            "motive" => {
                                let _ = motive_by_name(field).ok_or_else(|| {
                                    Error::at(*span, format!("unknown motive '{field}'"))
                                })?;
                                format!("stack.motive.{field}")
                            }
                            "skill" => {
                                let _ = skill_by_name(field).ok_or_else(|| {
                                    Error::at(*span, format!("unknown skill '{field}'"))
                                })?;
                                format!("stack.skill.{field}")
                            }
                            "personality" => {
                                format!("stack.person[{}]", personality_trait(*span, field)?)
                            }
                            other => {
                                return Err(Error::at(
                                    *span,
                                    format!("unknown Sim member '{rname}.{other}.{field}'"),
                                ));
                            }
                        };
                        return Ok(LValue::ForeignCell {
                            obj_air,
                            cell,
                            setup: None,
                            ty: ValueType::I16,
                        });
                    }
                }

                if let V2Expr::Ident(bspan, bname) = base.as_ref() {
                    if self.is_self_ident(bname) {
                        if let Some(st) = self.env.states.get(field) {
                            return Ok(
                                self.self_cell(&format!("attr[{}]", st.attr_index), st.ty.clone())
                            );
                        }
                        return match field.as_str() {
                            "graphic" => Ok(self.self_cell("data[0]", ValueType::I16)),
                            "prep_score" => Ok(self.self_cell("prep_score", ValueType::I16)),
                            "cook_score" => Ok(self.self_cell("cook_score", ValueType::I16)),
                            "in_use" => Ok(self.self_cell("in_use", ValueType::I16)),
                            other if let Some(w) = object_data_word(other) => {
                                Ok(self.self_cell(&format!("data[{}]", w.index), w.value_type()))
                            }
                            "motive" | "skill" | "priority" => Err(Error::at(
                                *span,
                                format!(
                                    "'self' is an Object, not a Sim (cannot access 'self.{field}')"
                                ),
                            )),
                            other => Err(Error::at(
                                *span,
                                format!("unknown object state or field 'self.{other}'"),
                            )),
                        };
                    }
                    if self.is_my_ident(bname) {
                        return match field.as_str() {
                            "hand" => Ok(LValue::Direct {
                                air: "my.slot[0]".to_owned(),
                                ty: ValueType::ObjectId,
                            }),
                            "age" => Ok(LValue::Direct {
                                air: "my.person[58]".to_owned(),
                                ty: ValueType::I16,
                            }),
                            "priority" => Ok(LValue::Direct {
                                air: "my.person.priority".to_owned(),
                                ty: ValueType::I16,
                            }),
                            other if let Some(w) = sim_person_word(other) => Ok(LValue::Direct {
                                air: format!("my.person[{}]", w.index),
                                ty: w.value_type(),
                            }),
                            other if let Some(w) = object_data_word(other) => Ok(LValue::Direct {
                                air: format!("my.data[{}]", w.index),
                                ty: w.value_type(),
                            }),
                            _ => Err(Error::at(
                                *span,
                                format!(
                                    "cannot assign to '{bname}.{field}' (use '{bname}.motive.<name>', '{bname}.skill.<name>', '{bname}.personality.<trait>', '{bname}.age', '{bname}.priority', '{bname}.job_level' (and the other named person words), '{bname}.attr[i]', '{bname}.data[i]', or '{bname}.person[i]'; use 'self.<state>' for object state)"
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
                    // Typed foreign object field access (`fridge.stock`, `plate.nutrition`, `part.occupancy`, etc.)
                    if let Some(binding) = self.lookup_var(bname).cloned() {
                        let foreign_info = match binding {
                            Binding::Local {
                                air_name,
                                ty: ValueType::ObjectRef(schema_key),
                            } => Some((air_name, schema_key)),
                            Binding::BhavParam {
                                index,
                                ty: ValueType::ObjectRef(schema_key),
                            } => Some((format!("param[{index}]"), schema_key)),
                            _ => None,
                        };
                        if let Some((obj_air, schema_key)) = foreign_info {
                            let schema = self
                                .env
                                .object_schemas
                                .get(&schema_key)
                                .expect("ObjectRef schema must exist");
                            if let Some((attr_idx, state_ty)) = schema.states.get(field) {
                                return Ok(LValue::ForeignAttr {
                                    obj_air,
                                    attr_index: *attr_idx as i16,
                                    ty: state_ty.clone(),
                                });
                            }
                            return match field.as_str() {
                                "graphic" => Ok(LValue::ForeignCell {
                                    cell: "stack.data[0]".to_owned(),
                                    setup: None,
                                    obj_air,
                                    ty: ValueType::I16,
                                }),
                                other => match Self::handle_field_cell(other, false) {
                                    Some((cell, ty)) => Ok(LValue::ForeignCell {
                                        obj_air,
                                        cell,
                                        setup: None,
                                        ty,
                                    }),
                                    None => Err(Error::at(
                                        *span,
                                        format!(
                                            "object '{}' has no state field '{other}'",
                                            schema.display_name
                                        ),
                                    )),
                                },
                            };
                        }
                    }
                    if let Some((obj_air, maybe_sim)) = self.object_handle_air(bname) {
                        let (cell, ty) = match field.as_str() {
                            "graphic" => ("stack.data[0]".to_owned(), ValueType::I16),
                            other => Self::handle_field_cell(other, maybe_sim).ok_or_else(|| {
                                Error::at(
                                    *span,
                                    format!(
                                        "unknown field '{bname}.{other}' on an object handle (expected graphic, hand, room, a named object-data word (flags, facing, container, value, ...), age, priority, is_visitor, visitor_kind, kind, a named person word (job_level, gender_kind, ...), motive.<name>, skill.<name>, attr[i], data[i], person[i], motive[m], slot[i], or sprite_flags[i])"
                                    ),
                                )
                            })?,
                        };
                        return Ok(LValue::ForeignCell {
                            obj_air,
                            cell,
                            setup: None,
                            ty,
                        });
                    }
                }
                let path = match base.as_ref() {
                    V2Expr::Ident(_, b) => format!("'{b}.{field}'"),
                    V2Expr::Field {
                        base: root,
                        field: group,
                        ..
                    } => match root.as_ref() {
                        V2Expr::Ident(_, r) => format!("'{r}.{group}.{field}'"),
                        _ => format!("'.{field}'"),
                    },
                    _ => format!("'.{field}'"),
                };
                Err(Error::at(
                    *span,
                    format!("unknown field {path} (nothing in scope has it)"),
                ))
            }
            V2Expr::Index { span, base, .. } if matches!(base.as_ref(), V2Expr::Field { field, .. } if field == "has_function") => {
                Err(Error::at(
                    *span,
                    "'has_function[N]' is read-only (whether the object's definition names a tree for function N)",
                ))
            }
            V2Expr::Index { span, base, index } => {
                // `temp[i]`, `global[i]`, `self.attr[i]`, `self.data[i]`, `sim.motive[m]`, `foreign.attr[i]`
                if let V2Expr::Ident(_, bname) = base.as_ref() {
                    match bname.as_str() {
                        "temp" => {
                            let idx = self.eval_const_i16(index)?;
                            return Ok(LValue::Direct {
                                air: format!("temp[{idx}]"),
                                ty: ValueType::I16,
                            });
                        }
                        "global" => {
                            let idx = self.eval_const_i16(index)?;
                            return Ok(LValue::Direct {
                                air: format!("global[{idx}]"),
                                ty: ValueType::I16,
                            });
                        }
                        "local" if self.lookup_var("local").is_none() => {
                            return Err(Error::at(
                                *span,
                                "'local[N]' is read-only (a raw block writes a numbered local slot; name the local with 'let' instead)",
                            ));
                        }
                        // `param[n]`: the call frame's argument word, readable and assignable
                        // inside the frame (positional params, `#[frame(args = )]`, functionals).
                        "param" if self.lookup_var("param").is_none() => {
                            let idx = self.eval_builtin_int(index, "param index", 0, 255)?;
                            if idx as usize >= self.frame_args {
                                return Err(Error::at(
                                    *span,
                                    format!(
                                        "param[{idx}] is outside this tree's {} argument(s) (declare the parameter or pin '#[frame(args = N)]')",
                                        self.frame_args
                                    ),
                                ));
                            }
                            self.param_words_used = self.param_words_used.max(idx as usize + 1);
                            return Ok(LValue::Direct {
                                air: format!("param[{idx}]"),
                                ty: ValueType::I16,
                            });
                        }
                        _ => {}
                    }
                }
                if let V2Expr::Field {
                    base: root, field, ..
                } = base.as_ref()
                    && let V2Expr::Ident(rspan, rname) = root.as_ref()
                {
                    if rname == "sim" && matches!(self.tree_ctx, TreeContext::ObjectSelf) {
                        return Err(Error::at(
                            *rspan,
                            "'sim' is not available in init/main (object lifecycle context)",
                        ));
                    }
                    let is_my_cell = self.is_sim_ident(rname) || self.is_frame_me(rname);
                    if is_my_cell && field == "motive" {
                        let m = self.resolve_motive_expr(index)?;
                        return Ok(LValue::Direct {
                            air: format!("my.motive[{m}]"),
                            ty: ValueType::I16,
                        });
                    }
                    if self.is_my_ident(rname) && field == "skill" {
                        let sk = self.resolve_skill_expr(index)?;
                        return Ok(LValue::Direct {
                            air: format!("my.person[{sk}]"),
                            ty: ValueType::I16,
                        });
                    }
                    // `sim.person[temp[k]]`: scope 30 reads the word whose number is in `temp[k]`.
                    if is_my_cell
                        && field == "person"
                        && let Some(k) = self.temp_word_index(index)
                    {
                        return Ok(LValue::Direct {
                            air: format!("scope(30, {k})"),
                            ty: ValueType::I16,
                        });
                    }
                    if is_my_cell && matches!(field.as_str(), "slot" | "person" | "attr" | "data") {
                        let idx = self.eval_builtin_int(
                            index,
                            &format!("{rname}.{field} index"),
                            0,
                            255,
                        )?;
                        return Ok(LValue::Direct {
                            air: format!("my.{field}[{idx}]"),
                            ty: if field == "slot" {
                                ValueType::ObjectId
                            } else {
                                ValueType::I16
                            },
                        });
                    }
                    if rname == "advert"
                        && self.lookup_var(rname).is_none()
                        && let Some(scope) = match field.as_str() {
                            "delta" => Some(12),
                            "personality" => Some(28),
                            "minimum" => Some(29),
                            _ => None,
                        }
                    {
                        // Motive indices are 0..16, which is also the advertisement range.
                        let m = self.resolve_motive_expr(index)?;
                        return Ok(LValue::Direct {
                            air: format!("scope({scope}, {m})"),
                            ty: ValueType::I16,
                        });
                    }
                    if self.is_self_ident(rname) && field == "sprite_flags" {
                        let (setup, cell) = self.sprite_flag_cell(index)?;
                        let obj_air = match self.tree_ctx {
                            TreeContext::ObjectSelf => "my.id",
                            TreeContext::SimOnObject { .. } => self.self_stack_target(),
                        };
                        return Ok(LValue::ForeignCell {
                            obj_air: obj_air.to_owned(),
                            cell,
                            setup,
                            ty: ValueType::I16,
                        });
                    }
                    if self.is_self_ident(rname) && field == "slot" {
                        let idx = self.eval_builtin_int(index, "self.slot index", 0, 255)?;
                        return Ok(self.self_cell(&format!("slot[{idx}]"), ValueType::ObjectId));
                    }
                    if let Some((obj_air, maybe_sim)) = self.object_handle_air(rname)
                        && matches!(
                            field.as_str(),
                            "person" | "motive" | "skill" | "slot" | "sprite_flags"
                        )
                    {
                        if field == "sprite_flags" {
                            let (setup, cell) = self.sprite_flag_cell(index)?;
                            return Ok(LValue::ForeignCell {
                                obj_air,
                                cell,
                                setup,
                                ty: ValueType::I16,
                            });
                        }
                        if !maybe_sim && field != "slot" {
                            return Err(Error::at(
                                *span,
                                format!(
                                    "'{rname}' is a typed object handle, not a Sim (cannot index '{rname}.{field}[...]')"
                                ),
                            ));
                        }
                        let (cell, ty) = match field.as_str() {
                            "motive" => (
                                format!("stack.motive[{}]", self.resolve_motive_expr(index)?),
                                ValueType::I16,
                            ),
                            "skill" => (
                                format!("stack.person[{}]", self.resolve_skill_expr(index)?),
                                ValueType::I16,
                            ),
                            // Scope 31 reads the word whose number is in `temp[k]`.
                            "person" if let Some(k) = self.temp_word_index(index) => {
                                (format!("scope(31, {k})"), ValueType::I16)
                            }
                            other => {
                                let idx = self.eval_builtin_int(
                                    index,
                                    &format!("{rname}.{other} index"),
                                    0,
                                    255,
                                )?;
                                let ty = if other == "slot" {
                                    ValueType::ObjectId
                                } else {
                                    ValueType::I16
                                };
                                (format!("stack.{other}[{idx}]"), ty)
                            }
                        };
                        return Ok(LValue::ForeignCell {
                            obj_air,
                            cell,
                            setup: None,
                            ty,
                        });
                    }
                    if self.is_self_ident(rname) {
                        let idx = self.eval_const_i16(index)?;
                        return match field.as_str() {
                            "attr" => Ok(self.self_cell(&format!("attr[{idx}]"), ValueType::I16)),
                            "data" => Ok(self.self_cell(&format!("data[{idx}]"), ValueType::I16)),
                            other => Err(Error::at(
                                *span,
                                format!("cannot index 'self.{other}[...]'"),
                            )),
                        };
                    }
                    // The stack object's own cells: no switch.
                    if rname == "stack_obj" && matches!(field.as_str(), "attr" | "data") {
                        let idx = self.eval_const_i16(index)?;
                        return Ok(LValue::Direct {
                            air: format!("stack.{field}[{idx}]"),
                            ty: ValueType::I16,
                        });
                    }
                    // Foreign ObjectId / ObjectRef variable: `plate.attr[0] = nutrition;`
                    if let Some(binding) = self.lookup_var(rname).cloned() {
                        let obj_air = match binding {
                            Binding::Local { air_name, ty } if ty.is_object_handle() => air_name,
                            Binding::ForeignObject { air_name } => air_name,
                            Binding::BhavParam { index, ty } if ty.is_object_handle() => {
                                format!("param[{index}]")
                            }
                            Binding::Local { ty, .. } | Binding::BhavParam { ty, .. } => {
                                return Err(Error::at(
                                    *span,
                                    format!(
                                        "cannot index '{rname}.{field}[...]': '{rname}' has type '{}', expected ObjectId",
                                        self.env.display_value_type(&ty)
                                    ),
                                ));
                            }
                            _ => {
                                return Err(Error::at(
                                    *span,
                                    format!("cannot index '{rname}.{field}[...]'"),
                                ));
                            }
                        };
                        let idx = self.eval_const_i16(index)?;
                        return match field.as_str() {
                            "attr" => Ok(LValue::ForeignAttr {
                                obj_air,
                                attr_index: idx,
                                ty: ValueType::I16,
                            }),
                            "data" => Ok(LValue::ForeignCell {
                                cell: format!("stack.data[{idx}]"),
                                setup: None,
                                obj_air,
                                ty: ValueType::I16,
                            }),
                            other => Err(Error::at(
                                *span,
                                format!("cannot index '{rname}.{other}[...]'"),
                            )),
                        };
                    }
                }
                Err(Error::at(*span, "unsupported indexed assignment target"))
            }
            other => Err(Error::at(
                other.span(),
                "expression is not a valid assignment target",
            )),
        }
    }

    pub(super) fn emit_lvalue_assign(
        &mut self,
        lval: &LValue,
        op: V2AssignOp,
        rhs_air: &str,
    ) -> Result<()> {
        if matches!(lval, LValue::Direct { air, .. } if air == "stack_obj") {
            self.sets_stack_obj = true;
        }
        let op_str = match op {
            V2AssignOp::Set => ":=",
            V2AssignOp::Add => "+=",
            V2AssignOp::Sub => "-=",
            V2AssignOp::Mul => "*=",
            V2AssignOp::Div => "/=",
            V2AssignOp::Mod => "%=",
            V2AssignOp::BitAnd => "&=",
            V2AssignOp::BitOr => "|=",
            V2AssignOp::BitXor => "^=",
            V2AssignOp::FlagSet => "flag_set",
            V2AssignOp::FlagClear => "flag_clear",
        };
        match lval {
            LValue::Direct { air, .. } => {
                self.emit_store(air, op_str, rhs_air);
            }
            LValue::ForeignAttr {
                obj_air,
                attr_index,
                ..
            } => {
                let effective_rhs = if reads_stack_obj(rhs_air) {
                    let snap = self.fresh("frhs");
                    self.emit_let(&snap, rhs_air);
                    snap
                } else {
                    rhs_air.to_owned()
                };
                let saved = self.save_stack_obj_for_move();
                self.stack_op(mir::StackObjOp::Set(obj_air));
                self.line(format!(
                    "stack.attr[{attr_index}] {op_str} {effective_rhs};"
                ));
                self.emit_restore_stack_obj(saved.as_deref());
            }
            LValue::ForeignCell {
                obj_air,
                cell,
                setup,
                ..
            } => {
                let switches = obj_air != "stack_obj";
                let clobbers_temp0 = setup.is_some()
                    && (rhs_air.contains("temp[0]") || rhs_air.contains("scope(33,"));
                let effective_rhs = if (switches && reads_stack_obj(rhs_air)) || clobbers_temp0 {
                    let snap = self.fresh("frhs");
                    self.emit_let(&snap, rhs_air);
                    snap
                } else {
                    rhs_air.to_owned()
                };
                if let Some(setup) = setup {
                    self.line(setup.clone());
                }
                let saved = if switches {
                    let saved = self.save_stack_obj_for_move();
                    self.stack_op(mir::StackObjOp::Set(obj_air));
                    saved
                } else {
                    None
                };
                self.emit_store(cell, op_str, &effective_rhs);
                if switches {
                    self.emit_restore_stack_obj(saved.as_deref());
                }
            }
        }
        Ok(())
    }

    /// `local[N]` read: frame word N as a raw block wrote it. Like a raw block, naming a slot
    /// pins the tree's layout (`.air` puts the named locals after every explicit slot, and the
    /// optimizer leaves the tree alone), so it cannot share a tree with a pinned
    /// `push_interaction` object slot.
    pub(super) fn read_frame_local_slot(
        &mut self,
        span: Span,
        index: &V2Expr,
    ) -> Result<(String, ValueType)> {
        let idx = self.eval_builtin_int(index, "local slot", 0, 255)?;
        if self.checking_discarded == 0 {
            self.raw_span.get_or_insert(span);
            self.keep_layout = true;
            if self.pin_span.is_some() {
                return Err(Error::at(span, RAW_WITH_PIN));
            }
        }
        Ok((format!("local[{idx}]"), ValueType::I16))
    }

    pub(super) fn read_lvalue(&mut self, lval: &LValue) -> Result<(String, ValueType)> {
        match lval {
            // The id and room words read by name (`my.data[11]` is `my.id`), so a number
            // builds exactly the nodes its name does: `my.id` is a value the optimiser knows.
            LValue::Direct { air, ty } => Ok((named_word_read(air), ty.clone())),
            LValue::ForeignAttr {
                obj_air,
                attr_index,
                ty,
            } => {
                let tmp = self.fresh("fattr");
                let saved = self.save_stack_obj_for_move();
                self.stack_op(mir::StackObjOp::Set(obj_air));
                self.line(format!("let {tmp} = stack.attr[{attr_index}];"));
                self.emit_restore_stack_obj(saved.as_deref());
                Ok((tmp, ty.clone()))
            }
            LValue::ForeignCell {
                obj_air,
                cell,
                setup,
                ty,
            } => {
                let tmp = self.fresh("fdata");
                if let Some(setup) = setup {
                    self.line(setup.clone());
                }
                if obj_air == "stack_obj" {
                    self.emit_let(&tmp, cell);
                } else {
                    let saved = self.save_stack_obj_for_move();
                    self.stack_op(mir::StackObjOp::Set(obj_air));
                    self.emit_let(&tmp, cell);
                    self.emit_restore_stack_obj(saved.as_deref());
                }
                Ok((tmp, ty.clone()))
            }
        }
    }
}

/// The read spelling of a `my`/`stack` id or room word: `my.data[11]` → `my.id`,
/// `stack.data[29]` → `stack.room`. The two encode the same operand.
fn named_word_read(air: &str) -> String {
    match air.split_once('.') {
        Some((prefix @ ("my" | "stack"), "data[11]")) => format!("{prefix}.id"),
        Some((prefix @ ("my" | "stack"), "data[29]")) => format!("{prefix}.room"),
        _ => air.to_owned(),
    }
}

/// The handle of `h.neighbor` or `h.neighbor.field`/`h.neighbor.person` (the base of a neighbor
/// cell), if `e` is one.
fn neighbor_place_root(e: &V2Expr) -> Option<&str> {
    let V2Expr::Field { base, field, .. } = e else {
        return None;
    };
    match base.as_ref() {
        V2Expr::Ident(_, root) if field == "neighbor" => Some(root),
        inner @ V2Expr::Field { .. } if neighbor_indexed_scope(field).is_some() => {
            neighbor_place_root(inner)
                .filter(|_| matches!(inner, V2Expr::Field { field, .. } if field == "neighbor"))
        }
        _ => None,
    }
}
