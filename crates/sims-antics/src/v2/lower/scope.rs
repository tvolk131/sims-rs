//! Lexical scopes and cleanups: `defer`, locks, loop exits, `continue` steps and cancel
//! propagation.

use super::*;

impl<'a> TreeLowerer<'a> {
    pub(super) fn push_scope(&mut self, frame: ScopeFrame) {
        self.var_scopes.push(BTreeMap::new());
        self.cleanup_scopes.push(frame);
    }

    pub(super) fn pop_scope_and_run_cleanups(&mut self) -> Result<()> {
        let frame = self.cleanup_scopes.pop().expect("cleanup_scopes balanced");
        let res = self.run_frame_cleanups(&frame);
        self.var_scopes.pop();
        res
    }

    pub(super) fn pop_scope_discard_cleanups(&mut self) {
        let frame = self.cleanup_scopes.pop().expect("cleanup_scopes balanced");
        for action in frame.cleanups.iter().rev() {
            if let CleanupAction::Unlock(lval) = action
                && let Some(pos) = self.held_locks.iter().rposition(|x| x == lval)
            {
                self.held_locks.remove(pos);
            }
        }
        self.var_scopes.pop();
    }

    pub(super) fn fn_scope_floor(&self) -> usize {
        self.cleanup_scopes
            .iter()
            .rposition(|f| f.is_fn_root)
            .unwrap_or(0)
    }

    pub(super) fn has_active_cleanups(&self) -> bool {
        let floor = self.fn_scope_floor();
        self.cleanup_scopes[floor..]
            .iter()
            .any(|f| !f.cleanups.is_empty())
    }

    /// Whether a `break` (when `brk`) or a `continue` (when `cont`) of the innermost loop emits
    /// code besides cleanups (which `has_any_active_cleanups` covers): a `continue`'s counter or
    /// range step, or a `break`'s stack-object restore in a `for ... else`. A raw block's own
    /// loop exit would skip it.
    pub(super) fn loop_exit_has_work(&self, brk: bool, cont: bool) -> bool {
        let floor = self.fn_scope_floor();
        let Some(frame) = self.cleanup_scopes[floor..]
            .iter()
            .rev()
            .find(|f| f.is_loop_body)
        else {
            return false;
        };
        (cont && frame.continue_step.is_some())
            || (brk
                && frame.restore_stack_on_break
                && (self.has_self_obj_local || matches!(self.tree_ctx, TreeContext::ObjectSelf)))
    }

    pub(super) fn has_any_active_cleanups(&self) -> bool {
        self.cleanup_scopes.iter().any(|f| !f.cleanups.is_empty())
    }

    pub(super) fn emit_cleanups_down_to(&mut self, start: usize) -> Result<()> {
        let saved = self.cleanup_scopes.split_off(start);
        let saved_locks = self.held_locks.clone();
        let mut res = Ok(());
        for frame in saved.iter().rev() {
            if let Err(e) = self.run_frame_cleanups(frame) {
                res = Err(e);
                break;
            }
        }
        self.held_locks = saved_locks;
        self.cleanup_scopes.extend(saved);
        res
    }

    pub(super) fn emit_all_cleanups(&mut self) -> Result<()> {
        let floor = self.fn_scope_floor();
        self.emit_cleanups_down_to(floor)
    }

    pub(super) fn emit_loop_cleanups(&mut self, span: Span, is_continue: bool) -> Result<()> {
        let floor = self.fn_scope_floor();
        let loop_idx = self.cleanup_scopes[floor..]
            .iter()
            .rposition(|f| f.is_loop_body)
            .map(|rel| floor + rel)
            .ok_or_else(|| {
                Error::at(
                    span,
                    if self.in_defer() {
                        if is_continue {
                            "'continue' cannot exit out of a 'defer' block"
                        } else {
                            "'break' cannot exit out of a 'defer' block"
                        }
                    } else if is_continue {
                        "'continue' outside of a loop"
                    } else {
                        "'break' outside of a loop"
                    },
                )
            })?;
        if self.cleanup_scopes[loop_idx].owner_defer_id != self.current_defer_id {
            return Err(Error::at(
                span,
                if is_continue {
                    "'continue' cannot exit out of a 'defer' block"
                } else {
                    "'break' cannot exit out of a 'defer' block"
                },
            ));
        }
        self.emit_cleanups_down_to(loop_idx)?;
        if is_continue {
            if let Some(step) = self.cleanup_scopes[loop_idx].continue_step.clone() {
                self.emit_continue_step(&step);
            }
        } else if self.cleanup_scopes[loop_idx].restore_stack_on_break {
            self.emit_entry_stack_restore();
        }
        Ok(())
    }

    pub(super) fn emit_continue_step(&mut self, step: &ContinueStep) {
        match step {
            ContinueStep::Counter(counter) => self.line(format!("{counter} += 1;")),
            ContinueStep::Range { var, last, step } => {
                self.line(format!("if {var} == {last} {{"));
                self.em.indent();
                self.line("break;".to_owned());
                self.em.dedent();
                self.line("}".to_owned());
                self.line(format!("{var} {step} 1;"));
            }
        }
    }

    pub(super) fn emit_cancel_propagation(&mut self, restore_stack: bool) -> Result<()> {
        if self.in_defer() {
            return Ok(());
        }
        let Some((_, flag)) = self.enclosing_cancel_flag() else {
            return Ok(());
        };
        let target_while = self
            .nearest_air_while_idx()
            .expect("enclosing cancel_flag implies an enclosing air while loop");
        self.line(format!("if {flag} != 0 {{"));
        self.em.indent();
        self.emit_cleanups_down_to(target_while)?;
        if restore_stack {
            self.emit_entry_stack_restore();
        }
        self.line("break;".to_owned());
        self.em.dedent();
        self.line("}".to_owned());
        Ok(())
    }

    pub(super) fn run_frame_cleanups(&mut self, frame: &ScopeFrame) -> Result<()> {
        // LIFO order within each lexical scope!
        for action in frame.cleanups.iter().rev() {
            match action {
                CleanupAction::Unlock(lval) => {
                    self.emit_lvalue_assign(lval, V2AssignOp::Set, "0")?;
                    if let Some(pos) = self.held_locks.iter().rposition(|x| x == lval) {
                        self.held_locks.remove(pos);
                    }
                }
                CleanupAction::Defer {
                    owner_module,
                    in_object_scope,
                    body,
                    saved_bindings,
                } => {
                    self.defer_generation += 1;
                    let prev_defer_id = self.current_defer_id.replace(self.defer_generation);
                    let prev_in_obj = self.in_object_scope;
                    let prev_vars = std::mem::replace(&mut self.var_scopes, saved_bindings.clone());
                    self.in_object_scope = *in_object_scope;
                    let res = self.with_owner_module(owner_module, |c| {
                        c.push_scope(ScopeFrame::block());
                        let r = c.lower_stmts(body);
                        let p = c.pop_scope_and_run_cleanups();
                        r.and(p)
                    });
                    self.current_defer_id = prev_defer_id;
                    self.in_object_scope = prev_in_obj;
                    self.var_scopes = prev_vars;
                    res?;
                }
            }
        }
        Ok(())
    }
}
