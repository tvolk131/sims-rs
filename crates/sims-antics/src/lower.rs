//! Deterministic lowering from `.antics` AST (both low-level exact and
//! high-level structured trees) to validated `BHAV` instruction graphs.

use crate::ast::{
    CallTarget, Condition, Item, Module, Operand, PrimitiveCall, Stmt, Target, TreeDecl,
};
use crate::error::{Error, Result, Span};
use crate::ir::{BconChunk, BhavChunk};
use sims_formats::behavior::Instruction;
use std::collections::BTreeMap;

/// Maximum instruction count in a single SimAntics BHAV tree (`0..=250`).
pub const MAX_BHAV_NODES: usize = 251;
pub const TARGET_ERROR: u8 = 0xFD;
pub const TARGET_TRUE: u8 = 0xFE;
pub const TARGET_FALSE: u8 = 0xFF;

/// Result of lowering a `.antics` module.
#[derive(Debug, Clone, Default)]
pub struct LoweredModule {
    pub behaviors: BTreeMap<u16, BhavChunk>,
    pub tree_ids_by_name: BTreeMap<String, u16>,
    pub constants: BTreeMap<u16, BconChunk>,
}

/// Lowers a parsed `.antics` [`Module`] into deterministic [`BhavChunk`]s and
/// [`BconChunk`]s with full jump target validation.
pub fn lower_module(module: &Module) -> Result<LoweredModule> {
    let mut tree_ids_by_name = BTreeMap::new();
    for item in &module.items {
        if let Item::Tree(tree) = item
            && let Some(prev) = tree_ids_by_name.insert(tree.label.clone(), tree.id)
            && prev != tree.id
        {
            return Err(Error::at(
                tree.span,
                format!(
                    "duplicate tree name '{}' (ids {prev} and {})",
                    tree.label, tree.id
                ),
            ));
        }
    }

    let mut behaviors = BTreeMap::new();
    let mut constants = BTreeMap::new();
    for item in &module.items {
        match item {
            Item::Tree(tree) => {
                let bhav = lower_tree(tree, &tree_ids_by_name)?;
                if behaviors.insert(bhav.id, bhav).is_some() {
                    return Err(Error::at(
                        tree.span,
                        format!("duplicate BHAV tree id {}", tree.id),
                    ));
                }
            }
            Item::Bcon(bcon) => {
                if bcon.values.len() > 255 {
                    return Err(Error::at(
                        bcon.span,
                        "BCON table cannot exceed 255 constants",
                    ));
                }
                let chunk = BconChunk {
                    id: bcon.id,
                    label: bcon.label.clone(),
                    flags: bcon.flags,
                    values: bcon.values.clone(),
                    trailing: Vec::new(),
                };
                if constants.insert(chunk.id, chunk).is_some() {
                    return Err(Error::at(
                        bcon.span,
                        format!("duplicate BCON chunk id {}", bcon.id),
                    ));
                }
            }
        }
    }

    Ok(LoweredModule {
        behaviors,
        tree_ids_by_name,
        constants,
    })
}

#[derive(Debug, Clone)]
enum PendingTarget {
    Resolved(u8),
    Label { name: String, span: Span },
    Node { index: usize, span: Span },
}

#[derive(Debug, Clone)]
struct PendingNode {
    span: Span,
    opcode: u16,
    operands: [u8; 8],
    on_true: PendingTarget,
    on_false: PendingTarget,
}

struct TreeBuilder<'a> {
    tree_ids: &'a BTreeMap<String, u16>,
    nodes: Vec<PendingNode>,
    labels: BTreeMap<String, usize>,
    max_local_used: Option<u16>,
    max_param_used: Option<u8>,
    uses_idle: bool,
}

impl<'a> TreeBuilder<'a> {
    fn new(tree_ids: &'a BTreeMap<String, u16>) -> Self {
        Self {
            tree_ids,
            nodes: Vec::new(),
            labels: BTreeMap::new(),
            max_local_used: None,
            max_param_used: None,
            uses_idle: false,
        }
    }

    fn note_operand(&mut self, op: Operand) {
        if op.scope == 25 && op.index >= 0 {
            let idx = op.index as u16;
            self.max_local_used = Some(self.max_local_used.map_or(idx, |m| m.max(idx)));
        } else if op.scope == 9 && op.index >= 0 && op.index <= 255 {
            let idx = op.index as u8;
            self.max_param_used = Some(self.max_param_used.map_or(idx, |m| m.max(idx)));
        }
    }

    fn emit(
        &mut self,
        span: Span,
        opcode: u16,
        operands: [u8; 8],
        on_true: PendingTarget,
        on_false: PendingTarget,
    ) -> Result<usize> {
        let idx = self.nodes.len();
        if idx >= MAX_BHAV_NODES {
            return Err(Error::at(
                span,
                format!("BHAV tree exceeds maximum of {MAX_BHAV_NODES} instructions"),
            ));
        }
        self.nodes.push(PendingNode {
            span,
            opcode,
            operands,
            on_true,
            on_false,
        });
        Ok(idx)
    }

    fn target_from_ast(&self, target: &Target, span: Span) -> PendingTarget {
        match target {
            Target::Label(name) => PendingTarget::Label {
                name: name.clone(),
                span,
            },
            Target::Node(n) => PendingTarget::Node {
                index: usize::from(*n),
                span,
            },
            Target::ReturnTrue => PendingTarget::Resolved(TARGET_TRUE),
            Target::ReturnFalse => PendingTarget::Resolved(TARGET_FALSE),
            Target::Error => PendingTarget::Resolved(TARGET_ERROR),
        }
    }

    fn resolve_call_target(&self, target: &CallTarget, span: Span) -> Result<u16> {
        match target {
            CallTarget::Id(id) => {
                if *id < 256 {
                    return Err(Error::at(
                        span,
                        format!("subroutine id {id} must be >= 256 (0..255 are primitives)"),
                    ));
                }
                Ok(*id)
            }
            CallTarget::Name(name) => self
                .tree_ids
                .get(name)
                .copied()
                .ok_or_else(|| Error::at(span, format!("unknown subroutine tree '{name}'"))),
        }
    }

    fn encode_primitive(&mut self, prim: &PrimitiveCall, span: Span) -> Result<(u16, [u8; 8])> {
        match prim {
            PrimitiveCall::Sleep { param_index } => {
                self.note_operand(Operand::new(9, *param_index));
                let [p0, p1] = param_index.to_le_bytes();
                Ok((0, [p0, p1, 0, 0, 0, 0, 0, 0]))
            }
            PrimitiveCall::GenericCall { routine } => {
                let [r0, r1] = routine.to_le_bytes();
                Ok((1, [r0, r1, 0, 0, 0, 0, 0, 0]))
            }
            PrimitiveCall::ExprRaw { lhs, operator, rhs } => {
                self.note_operand(*lhs);
                self.note_operand(*rhs);
                Ok((2, encode_expr_operands(*lhs, *operator, *rhs)))
            }
            PrimitiveCall::Random { dst, range } => {
                self.note_operand(*dst);
                self.note_operand(*range);
                let [d0, d1] = dst.index.to_le_bytes();
                let [ds0, ds1] = u16::from(dst.scope).to_le_bytes();
                let [r0, r1] = range.index.to_le_bytes();
                let [rs0, rs1] = u16::from(range.scope).to_le_bytes();
                Ok((8, [d0, d1, ds0, ds1, r0, r1, rs0, rs1]))
            }
            PrimitiveCall::FindBestObject { function, flags } => {
                let [f0, f1] = function.to_le_bytes();
                Ok((14, [f0, f1, 0, 0, 0, 0, *flags, 0]))
            }
            PrimitiveCall::IdleForInput {
                param_index,
                allow_push,
            } => {
                self.note_operand(Operand::new(9, *param_index));
                let [p0, p1] = param_index.to_le_bytes();
                let [a0, a1] = allow_push.to_le_bytes();
                Ok((17, [p0, p1, a0, a1, 0, 0, 0, 0]))
            }
            PrimitiveCall::PlaySound {
                sound_id,
                use_stack_object,
            } => {
                let [s0, s1] = sound_id.to_le_bytes();
                let flags = if *use_stack_object { 2 } else { 0 };
                Ok((23, [s0, s1, 0, 0, flags, 0, 0, 0]))
            }
            PrimitiveCall::StopAllSounds { use_stack_object } => {
                let target: u16 = u16::from(*use_stack_object);
                let [t0, t1] = target.to_le_bytes();
                Ok((48, [t0, t1, 0, 0, 0, 0, 0, 0]))
            }
            PrimitiveCall::SetMotiveChange {
                motive,
                delta_per_hour,
                stop_at,
            } => {
                if !matches!(motive, 5..=9 | 14..=15) {
                    return Err(Error::at(
                        span,
                        format!("motive {motive} cannot be modified by set_motive_change"),
                    ));
                }
                self.note_operand(*delta_per_hour);
                self.note_operand(*stop_at);
                let [a0, a1] = delta_per_hour.index.to_le_bytes();
                let [s0, s1] = stop_at.index.to_le_bytes();
                Ok((
                    29,
                    [
                        delta_per_hour.scope,
                        stop_at.scope,
                        *motive,
                        0,
                        a0,
                        a1,
                        s0,
                        s1,
                    ],
                ))
            }
            PrimitiveCall::ClearMotiveChanges => Ok((29, [0, 0, 0, 1, 0, 0, 0, 0])),
            PrimitiveCall::SetToNext { mode, guid } => {
                let [g0, g1, g2, g3] = guid.to_le_bytes();
                Ok((31, [g0, g1, g2, g3, *mode & 0x7F, 0, 0, 0]))
            }
            PrimitiveCall::Dialog {
                kind,
                text_id,
                flags,
            } => {
                let [t0, t1] = text_id.to_le_bytes();
                Ok((36, [t0, t1, 0, 0, 0, *kind & 0x7F, 0, *flags]))
            }
            PrimitiveCall::Animate {
                clip_id,
                table,
                flags,
                events,
                local_index,
            } => {
                let [c0, c1] = clip_id.to_le_bytes();
                Ok((
                    44,
                    [c0, c1, *local_index, 0, *table, *flags, *events as u8, 0],
                ))
            }
            PrimitiveCall::RouteTo { slot, flags } => {
                let [s0, s1] = slot.to_le_bytes();
                Ok((45, [s0, s1, 0, 0, *flags, 0, 0, 0]))
            }
            PrimitiveCall::Snap { slot, flags } => {
                let [s0, s1] = slot.to_le_bytes();
                Ok((46, [s0, s1, 0, 0, *flags, 0, 0, 0]))
            }
        }
    }

    fn encode_call_operands(&self, args: &[i16], pass_temps: bool, span: Span) -> Result<[u8; 8]> {
        if pass_temps {
            return Ok([0xFF; 8]);
        }
        if args.len() > 4 {
            return Err(Error::at(
                span,
                "single-instruction subroutine call accepts at most 4 i16 arguments",
            ));
        }
        let mut operands = [0u8; 8];
        for (i, &arg) in args.iter().enumerate() {
            let [lo, hi] = arg.to_le_bytes();
            operands[i * 2] = lo;
            operands[i * 2 + 1] = hi;
        }
        Ok(operands)
    }

    /// Lowers a sequence of statements whose fallthrough exits to `next_target`.
    /// Returns the entry target of this statement sequence.
    fn lower_block(&mut self, stmts: &[Stmt], next_target: PendingTarget) -> Result<PendingTarget> {
        // Check if this block uses explicit labels (low-level mode) vs structured control flow.
        let has_labels_or_raw = stmts
            .iter()
            .any(|s| matches!(s, Stmt::Label { .. } | Stmt::RawOp { .. }));

        if has_labels_or_raw {
            return self.lower_linear_block(stmts, next_target);
        }

        self.lower_structured_slice(stmts, next_target)
    }

    fn lower_linear_block(
        &mut self,
        stmts: &[Stmt],
        next_target: PendingTarget,
    ) -> Result<PendingTarget> {
        let start_node = self.nodes.len();
        for (idx, stmt) in stmts.iter().enumerate() {
            let is_last = idx + 1 == stmts.len();
            let fallthrough = if is_last {
                next_target.clone()
            } else {
                // Placeholder patched right after emitting this statement.
                PendingTarget::Resolved(TARGET_TRUE)
            };
            let before = self.nodes.len();
            match stmt {
                Stmt::Label { span, name } => {
                    if self.labels.insert(name.clone(), self.nodes.len()).is_some() {
                        return Err(Error::at(*span, format!("duplicate label '{name}'")));
                    }
                }
                Stmt::RawOp {
                    span,
                    opcode,
                    operands,
                    branches,
                } => {
                    let on_true = self.target_from_ast(&branches.on_true, *span);
                    let on_false = self.target_from_ast(&branches.on_false, *span);
                    self.emit(*span, *opcode, *operands, on_true, on_false)?;
                }
                Stmt::Primitive {
                    span,
                    prim,
                    branches,
                } => {
                    let (opcode, operands) = self.encode_primitive(prim, *span)?;
                    let (on_true, on_false) = match branches {
                        Some(b) => (
                            self.target_from_ast(&b.on_true, *span),
                            self.target_from_ast(&b.on_false, *span),
                        ),
                        None => (fallthrough.clone(), fallthrough.clone()),
                    };
                    self.emit(*span, opcode, operands, on_true, on_false)?;
                }
                Stmt::Call {
                    span,
                    target,
                    args,
                    pass_temps,
                    branches,
                } => {
                    let opcode = self.resolve_call_target(target, *span)?;
                    let operands = self.encode_call_operands(args, *pass_temps, *span)?;
                    let (on_true, on_false) = match branches {
                        Some(b) => (
                            self.target_from_ast(&b.on_true, *span),
                            self.target_from_ast(&b.on_false, *span),
                        ),
                        None => (fallthrough.clone(), PendingTarget::Resolved(TARGET_FALSE)),
                    };
                    self.emit(*span, opcode, operands, on_true, on_false)?;
                }
                _ => {
                    // Structured statement nested inside a labeled tree.
                    let after_placeholder = PendingTarget::Node {
                        index: usize::MAX,
                        span: Span::default(),
                    };
                    self.lower_structured_stmt(
                        stmt,
                        if is_last {
                            next_target.clone()
                        } else {
                            after_placeholder
                        },
                    )?;
                    if !is_last {
                        let next_idx = self.nodes.len();
                        for node in &mut self.nodes[before..] {
                            patch_sentinel(&mut node.on_true, next_idx);
                            patch_sentinel(&mut node.on_false, next_idx);
                        }
                    }
                    continue;
                }
            }
            if !is_last && self.nodes.len() > before {
                // Only patch implicit fallthrough when no explicit branches were given.
                if matches!(
                    stmt,
                    Stmt::Primitive { branches: None, .. } | Stmt::Call { branches: None, .. }
                ) {
                    let next_idx = self.nodes.len();
                    let last_node = self.nodes.last_mut().unwrap();
                    last_node.on_true = PendingTarget::Node {
                        index: next_idx,
                        span: last_node.span,
                    };
                    if matches!(stmt, Stmt::Primitive { branches: None, .. }) {
                        last_node.on_false = PendingTarget::Node {
                            index: next_idx,
                            span: last_node.span,
                        };
                    }
                }
            }
        }
        if self.nodes.len() == start_node {
            Ok(next_target)
        } else {
            Ok(PendingTarget::Node {
                index: start_node,
                span: self.nodes[start_node].span,
            })
        }
    }

    fn lower_structured_slice(
        &mut self,
        stmts: &[Stmt],
        next_target: PendingTarget,
    ) -> Result<PendingTarget> {
        if stmts.is_empty() {
            return Ok(next_target);
        }
        let mut i = 0usize;
        let mut entry = None;
        while i < stmts.len() {
            let is_last = i + 1 == stmts.len();
            let stmt = &stmts[i];
            if let Stmt::Return { span, target } = stmt {
                let resolved = self.target_from_ast(target, *span);
                if entry.is_none() {
                    return Ok(resolved);
                }
                break;
            }
            let after = if is_last {
                next_target.clone()
            } else if let Stmt::Return { span, target } = &stmts[i + 1] {
                // Directly route this statement's fallthrough to the return target!
                let ret = self.target_from_ast(target, *span);
                let stmt_entry = self.lower_structured_stmt(stmt, ret)?;
                if entry.is_none() {
                    entry = Some(stmt_entry);
                }
                // Stop after the return statement.
                break;
            } else {
                PendingTarget::Node {
                    index: usize::MAX,
                    span: Span::default(),
                }
            };
            let before = self.nodes.len();
            let stmt_entry = self.lower_structured_stmt(stmt, after)?;
            if entry.is_none() {
                entry = Some(stmt_entry);
            }
            if !is_last {
                let next_idx = self.nodes.len();
                for node in &mut self.nodes[before..] {
                    patch_sentinel(&mut node.on_true, next_idx);
                    patch_sentinel(&mut node.on_false, next_idx);
                }
            }
            i += 1;
        }
        Ok(entry.unwrap_or(next_target))
    }

    fn lower_structured_stmt(
        &mut self,
        stmt: &Stmt,
        next_target: PendingTarget,
    ) -> Result<PendingTarget> {
        match stmt {
            Stmt::Return { span, target } => Ok(self.target_from_ast(target, *span)),
            Stmt::Assign { span, lhs, op, rhs } => {
                self.note_operand(*lhs);
                self.note_operand(*rhs);
                let operands = encode_expr_operands(*lhs, op.code(), *rhs);
                let idx = self.emit(*span, 2, operands, next_target.clone(), next_target)?;
                Ok(PendingTarget::Node {
                    index: idx,
                    span: *span,
                })
            }
            Stmt::Idle { span, ticks } => {
                self.uses_idle = true;
                self.note_operand(*ticks);
                self.note_operand(Operand::new(9, 0));
                // 1. param[0] := ticks
                // 2. sleep(param[0]) -> next_target
                let sleep_idx_placeholder = PendingTarget::Node {
                    index: self.nodes.len() + 1,
                    span: *span,
                };
                let set_ops = encode_expr_operands(Operand::new(9, 0), 5, *ticks);
                let first = self.emit(
                    *span,
                    2,
                    set_ops,
                    sleep_idx_placeholder.clone(),
                    sleep_idx_placeholder,
                )?;
                self.emit(*span, 0, [0; 8], next_target.clone(), next_target)?;
                Ok(PendingTarget::Node {
                    index: first,
                    span: *span,
                })
            }
            Stmt::Primitive {
                span,
                prim,
                branches,
            } => {
                let (opcode, operands) = self.encode_primitive(prim, *span)?;
                let (on_true, on_false) = match branches {
                    Some(b) => (
                        self.target_from_ast(&b.on_true, *span),
                        self.target_from_ast(&b.on_false, *span),
                    ),
                    None => (next_target.clone(), next_target),
                };
                let idx = self.emit(*span, opcode, operands, on_true, on_false)?;
                Ok(PendingTarget::Node {
                    index: idx,
                    span: *span,
                })
            }
            Stmt::Call {
                span,
                target,
                args,
                pass_temps,
                branches,
            } => {
                let opcode = self.resolve_call_target(target, *span)?;
                let operands = self.encode_call_operands(args, *pass_temps, *span)?;
                let (on_true, on_false) = match branches {
                    Some(b) => (
                        self.target_from_ast(&b.on_true, *span),
                        self.target_from_ast(&b.on_false, *span),
                    ),
                    None => (next_target.clone(), PendingTarget::Resolved(TARGET_FALSE)),
                };
                let idx = self.emit(*span, opcode, operands, on_true, on_false)?;
                Ok(PendingTarget::Node {
                    index: idx,
                    span: *span,
                })
            }
            Stmt::If {
                span,
                cond,
                then_body,
                else_body,
            } => {
                // Emit condition first with placeholders, then bodies, then patch.
                let cond_idx = self.emit_condition(
                    cond,
                    PendingTarget::Resolved(TARGET_TRUE),
                    PendingTarget::Resolved(TARGET_FALSE),
                )?;
                let then_target = self.lower_structured_slice(then_body, next_target.clone())?;
                let else_target = self.lower_structured_slice(else_body, next_target)?;
                let node = &mut self.nodes[cond_idx];
                if cond_is_negated(cond) {
                    node.on_true = else_target;
                    node.on_false = then_target;
                } else {
                    node.on_true = then_target;
                    node.on_false = else_target;
                }
                Ok(PendingTarget::Node {
                    index: cond_idx,
                    span: *span,
                })
            }
            Stmt::While { span, cond, body } => {
                let cond_idx = self.emit_condition(
                    cond,
                    PendingTarget::Resolved(TARGET_TRUE),
                    next_target.clone(),
                )?;
                let loop_back = PendingTarget::Node {
                    index: cond_idx,
                    span: *span,
                };
                let body_target = self.lower_structured_slice(body, loop_back)?;
                let node = &mut self.nodes[cond_idx];
                if cond_is_negated(cond) {
                    node.on_true = next_target;
                    node.on_false = body_target;
                } else {
                    node.on_true = body_target;
                    node.on_false = next_target;
                }
                Ok(PendingTarget::Node {
                    index: cond_idx,
                    span: *span,
                })
            }
            Stmt::Label { span, .. } | Stmt::RawOp { span, .. } => Err(Error::at(
                *span,
                "labels and raw ops cannot appear inside nested structured blocks",
            )),
        }
    }

    fn emit_condition(
        &mut self,
        cond: &Condition,
        on_true: PendingTarget,
        on_false: PendingTarget,
    ) -> Result<usize> {
        match cond {
            Condition::Not(inner) => self.emit_condition(inner, on_false, on_true),
            Condition::Compare { span, lhs, op, rhs } => {
                self.note_operand(*lhs);
                self.note_operand(*rhs);
                let operands = encode_expr_operands(*lhs, op.code(), *rhs);
                self.emit(*span, 2, operands, on_true, on_false)
            }
            Condition::Call { span, target, args } => {
                let opcode = self.resolve_call_target(target, *span)?;
                let operands = self.encode_call_operands(args, false, *span)?;
                self.emit(*span, opcode, operands, on_true, on_false)
            }
            Condition::Primitive { span, prim } => {
                let (opcode, operands) = self.encode_primitive(prim, *span)?;
                self.emit(*span, opcode, operands, on_true, on_false)
            }
        }
    }

    fn resolve_target(&self, target: &PendingTarget, count: usize) -> Result<u8> {
        match target {
            PendingTarget::Resolved(b) => Ok(*b),
            PendingTarget::Label { name, span } => {
                let &idx = self
                    .labels
                    .get(name)
                    .ok_or_else(|| Error::at(*span, format!("unresolved label '@{name}'")))?;
                if idx >= count {
                    return Err(Error::at(
                        *span,
                        format!(
                            "label '@{name}' points past end of tree (node {idx}, count {count})"
                        ),
                    ));
                }
                Ok(idx as u8)
            }
            PendingTarget::Node { index, span } => {
                if *index >= count {
                    return Err(Error::at(
                        *span,
                        format!(
                            "branch target node {index} out of bounds (tree has {count} nodes)"
                        ),
                    ));
                }
                Ok(*index as u8)
            }
        }
    }
}

fn cond_is_negated(cond: &Condition) -> bool {
    match cond {
        Condition::Not(inner) => !cond_is_negated(inner),
        _ => false,
    }
}

fn patch_sentinel(target: &mut PendingTarget, actual_next: usize) {
    if let PendingTarget::Node { index, .. } = target
        && *index == usize::MAX
    {
        *index = actual_next;
    }
}

fn encode_expr_operands(lhs: Operand, operator: u8, rhs: Operand) -> [u8; 8] {
    let [l0, l1] = lhs.index.to_le_bytes();
    let [r0, r1] = rhs.index.to_le_bytes();
    [l0, l1, r0, r1, 0, operator, lhs.scope, rhs.scope]
}

fn lower_tree(tree: &TreeDecl, tree_ids: &BTreeMap<String, u16>) -> Result<BhavChunk> {
    if !matches!(tree.header.format, 0x8000..=0x8003) {
        return Err(Error::at(
            tree.span,
            format!("unsupported BHAV format {:#06x}", tree.header.format),
        ));
    }

    let mut builder = TreeBuilder::new(tree_ids);
    let entry = builder.lower_block(&tree.body, PendingTarget::Resolved(TARGET_TRUE))?;

    // If a high-level tree consisted solely of `return true/false/error`, emit a
    // clean 1-node no-op comparison (`0 == 0`) that immediately returns that value.
    // An explicitly empty tree `{}` stays at 0 instructions for lossless lifting.
    if builder.nodes.is_empty() && !tree.body.is_empty() {
        let ret_code = match entry {
            PendingTarget::Resolved(b) => b,
            _ => TARGET_TRUE,
        };
        let noop_ops = encode_expr_operands(Operand::literal(0), 2, Operand::literal(0));
        builder.emit(
            tree.span,
            2,
            noop_ops,
            PendingTarget::Resolved(ret_code),
            PendingTarget::Resolved(ret_code),
        )?;
    }

    let count = builder.nodes.len();
    let mut instructions = Vec::with_capacity(count);
    for node in &builder.nodes {
        let on_true = builder.resolve_target(&node.on_true, count)?;
        let on_false = builder.resolve_target(&node.on_false, count)?;
        instructions.push(Instruction {
            opcode: node.opcode,
            on_true,
            on_false,
            operands: node.operands,
        });
    }

    let inferred_locals = builder.max_local_used.map_or(0, |m| m + 1);
    let locals = match tree.header.locals {
        Some(declared) => {
            if declared < inferred_locals {
                return Err(Error::at(
                    tree.span,
                    format!(
                        "tree '{}' declares {declared} locals but references local[{}]",
                        tree.label,
                        inferred_locals - 1
                    ),
                ));
            }
            declared
        }
        None => inferred_locals,
    };

    let inferred_args = match (builder.max_param_used, builder.uses_idle) {
        (Some(m), _) => m.saturating_add(1),
        (None, true) => 1,
        (None, false) => 0,
    };
    let args = match tree.header.args {
        Some(declared) => {
            if declared < inferred_args && tree.header.format != 0x8000 {
                return Err(Error::at(
                    tree.span,
                    format!(
                        "tree '{}' declares {declared} args but requires at least {inferred_args}",
                        tree.label
                    ),
                ));
            }
            declared
        }
        None => inferred_args,
    };

    Ok(BhavChunk {
        id: tree.id,
        label: tree.label.clone(),
        format: tree.header.format,
        kind: tree.header.kind,
        args,
        locals,
        revision: tree.header.revision,
        instructions,
        trailing: Vec::new(),
    })
}
