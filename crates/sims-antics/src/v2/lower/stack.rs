//! `stack_obj` management: tracking, saves around moves, entry restore, `self` targeting and
//! `cursor` statement checks.

use super::*;

impl<'a> TreeLowerer<'a> {
    /// Emits one typed `stack_obj` operation.
    pub(super) fn stack_op(&mut self, op: mir::StackObjOp<'_>) {
        self.line(op.text());
    }

    /// `let NAME = VALUE;`, typed as a [`mir::StackObjOp::Save`] when `VALUE` is the stack
    /// object.
    pub(super) fn emit_let(&mut self, name: &str, value: &str) {
        if value == "stack_obj" {
            self.stack_op(mir::StackObjOp::Save(name));
        } else {
            self.line(format!("let {name} = {value};"));
        }
    }

    /// `LHS OP RHS;`, typed as a [`mir::StackObjOp::Set`] when it sets the stack object.
    pub(super) fn emit_store(&mut self, lhs: &str, op: &str, rhs: &str) {
        if lhs == "stack_obj" && op == ":=" {
            self.stack_op(mir::StackObjOp::Set(rhs));
        } else {
            self.line(format!("{lhs} {op} {rhs};"));
        }
    }

    pub(super) fn track_stack_obj(&mut self, explicit_stack_obj: bool) {
        self.explicit_stack_obj = explicit_stack_obj;
        self.has_self_obj_local |=
            explicit_stack_obj && matches!(self.tree_ctx, TreeContext::SimOnObject { .. });
    }

    pub(super) fn save_stack_obj_for_move(&mut self) -> Option<String> {
        self.explicit_stack_obj.then(|| {
            let saved = self.fresh("pre_stack_obj");
            self.stack_op(mir::StackObjOp::Save(&saved));
            saved
        })
    }

    pub(super) fn emit_restore_stack_obj(&mut self, saved: Option<&str>) {
        if let Some(saved) = saved {
            self.stack_op(mir::StackObjOp::Set(saved));
        } else if !self.explicit_stack_obj {
            self.emit_entry_stack_restore();
        }
    }

    /// Restores `stack_obj` after a `run_functional` call (see [`StackRestore`]).
    pub(super) fn emit_stack_restore(&mut self, restore: &StackRestore) {
        if let StackRestore::Moved(saved) = restore {
            self.emit_restore_stack_obj(saved.as_deref());
        }
    }

    pub(super) fn emit_entry_stack_restore(&mut self) {
        if self.has_self_obj_local {
            self.stack_op(mir::StackObjOp::Set(mir::SELF_OBJ));
        } else if matches!(self.tree_ctx, TreeContext::ObjectSelf) {
            self.used_saved_stack_obj = true;
            self.stack_op(mir::StackObjOp::Set(mir::SAVED_STACK_OBJ));
        }
    }

    /// A Sim tree's `self` is always its entry object, even when the user moves the cursor.
    pub(super) fn self_stack_target(&self) -> &'static str {
        if self.explicit_stack_obj && matches!(self.tree_ctx, TreeContext::SimOnObject { .. }) {
            "__self_obj"
        } else {
            "stack_obj"
        }
    }

    pub(super) fn self_cell(&self, field: &str, ty: ValueType) -> LValue {
        if self.self_stack_target() == "__self_obj" {
            LValue::ForeignCell {
                cell: format!("stack.{field}"),
                setup: None,
                obj_air: "__self_obj".to_owned(),
                ty,
            }
        } else {
            LValue::Direct {
                air: format!("{}.{field}", self.self_prefix()),
                ty,
            }
        }
    }

    /// Emits a statement whose target selector is the entry object (`self`) rather than the
    /// current cursor. In a Sim tree the primitive can only select it through `stack_obj`.
    pub(super) fn emit_on_self(&mut self, prim: String) {
        let target = self.self_stack_target();
        let saved = if target != "stack_obj" {
            let saved = self.save_stack_obj_for_move();
            self.stack_op(mir::StackObjOp::Set(target));
            saved
        } else {
            None
        };
        self.line(format!("{prim};"));
        if let Some(saved) = saved {
            self.emit_restore_stack_obj(Some(&saved));
        }
    }

    /// Whether a `for` body statement keeps `stack_obj` free to be the loop cursor `cursor`: it
    /// neither writes `stack_obj` nor reads the stack object other than through `cursor`'s own
    /// cells. A conservative syntactic whitelist (no calls, no `raw`, no other handles).
    pub(super) fn cursor_stmt_ok(&self, stmt: &V2Stmt, cursor: &str) -> bool {
        match stmt {
            V2Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                self.cursor_expr_ok(cond, cursor)
                    && then_body
                        .iter()
                        .chain(else_body)
                        .all(|s| self.cursor_stmt_ok(s, cursor))
            }
            V2Stmt::Return { value, .. } => value
                .as_ref()
                .is_none_or(|v| self.cursor_expr_ok(v, cursor)),
            V2Stmt::Break { .. } | V2Stmt::Continue { .. } | V2Stmt::ReturnError { .. } => true,
            V2Stmt::Let { name, init, .. } => {
                self.cursor_fresh_name(name, cursor) && self.cursor_expr_ok(init, cursor)
            }
            V2Stmt::LetUninit { name, .. } => self.cursor_fresh_name(name, cursor),
            V2Stmt::Assign { target, value, .. } => {
                let target_ok = match target {
                    V2Expr::Ident(_, name) => {
                        name != cursor
                            && matches!(self.lookup_var(name), Some(Binding::Local { .. }))
                    }
                    other => self.cursor_place_ok(other, cursor),
                };
                target_ok && self.cursor_expr_ok(value, cursor)
            }
            _ => false,
        }
    }

    /// Whether a body `let` may bind `name`: it shadows nothing. The whitelist resolves names as
    /// they stand outside the body, so a shadowing binding (`let sim = o;`) would be checked as
    /// the outer one.
    pub(super) fn cursor_fresh_name(&self, name: &str, cursor: &str) -> bool {
        name != cursor && name != "self" && name != "stack_obj" && self.lookup_var(name).is_none()
    }

    /// Whether `base.<field>` / `base.<group>[i]` names a cell the cursor loop may touch: the
    /// cursor's own (`stack.`), the Sim's (`my.` in an interaction), or `self`'s in an object's
    /// own tree (`my.`).
    pub(super) fn cursor_cell_root_ok(&self, root: &str, cursor: &str) -> bool {
        root == cursor
            || self.is_my_ident(root)
            || (self.is_self_ident(root) && matches!(self.tree_ctx, TreeContext::ObjectSelf))
    }

    /// `sprite_flags[i]` is rejected on every root: it lowers through scope 27, which reads the
    /// current stack object, so it switches `stack_obj` and restores it.
    pub(super) fn cursor_place_ok(&self, place: &V2Expr, cursor: &str) -> bool {
        match place {
            V2Expr::Field { field, .. } if field == "sprite_flags" => false,
            V2Expr::Field { base, .. } => match base.as_ref() {
                V2Expr::Ident(_, root) => self.cursor_cell_root_ok(root, cursor),
                V2Expr::Field { base: inner, .. } => {
                    matches!(inner.as_ref(), V2Expr::Ident(_, root) if self.cursor_cell_root_ok(root, cursor))
                }
                _ => false,
            },
            V2Expr::Index { base, index, .. } => {
                let base_ok = match base.as_ref() {
                    V2Expr::Ident(_, name) => {
                        matches!(name.as_str(), "temp" | "param") && self.lookup_var(name).is_none()
                    }
                    V2Expr::Field {
                        base: root, field, ..
                    } => {
                        field != "sprite_flags"
                            && matches!(root.as_ref(), V2Expr::Ident(_, r) if self.cursor_cell_root_ok(r, cursor))
                    }
                    _ => false,
                };
                base_ok && self.cursor_expr_ok(index, cursor)
            }
            _ => false,
        }
    }

    pub(super) fn cursor_expr_ok(&self, expr: &V2Expr, cursor: &str) -> bool {
        match expr {
            V2Expr::Int(..) | V2Expr::Bool(..) | V2Expr::Path(..) => true,
            V2Expr::Ident(_, name) => {
                if name == "stack_obj" {
                    return false;
                }
                match self.lookup_var(name) {
                    Some(Binding::MutRef { .. } | Binding::SelfHandle) => false,
                    Some(_) => true,
                    None => name != "self",
                }
            }
            V2Expr::Unary { inner, .. } => self.cursor_expr_ok(inner, cursor),
            V2Expr::Binary { lhs, rhs, .. } => {
                self.cursor_expr_ok(lhs, cursor) && self.cursor_expr_ok(rhs, cursor)
            }
            V2Expr::Field { .. } if self.bcon_field_ref(expr).is_some() => true,
            V2Expr::Field { .. } | V2Expr::Index { .. } => self.cursor_place_ok(expr, cursor),
            V2Expr::MethodCall {
                receiver,
                method,
                type_args,
                args,
                ..
            } => {
                method == "is_type"
                    && type_args.is_empty()
                    && matches!(receiver.as_ref(), V2Expr::Ident(_, r) if r == cursor)
                    && args.iter().all(|a| {
                        matches!(
                            a.value,
                            V2Expr::Int(..) | V2Expr::Ident(..) | V2Expr::Path(..)
                        )
                    })
            }
            _ => false,
        }
    }
}
