//! Deterministic lowering from `.antics` AST (both low-level exact and
//! high-level structured trees) to validated `BHAV`, `BCON`, and `STR#` chunks.

use crate::ast::{
    AssignOp, CallTarget, Condition, Item, LocalRef, Module, Operand, OperandRef, PrimitiveCall,
    Stmt, StringRef, Target, TreeDecl,
};
use crate::error::{Error, Result, Span};
use crate::ir::{BconChunk, BhavChunk, StringEntry, StringFormat, StringTableChunk};
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
    pub strings: BTreeMap<u16, StringTableChunk>,
}

/// Shared module-level symbol tables resolved before lowering individual trees.
struct ModuleSymbols {
    tree_ids_by_name: BTreeMap<String, u16>,
    bcon_by_name: BTreeMap<String, i16>,
    /// 1-based string indices in `STR# 301` (used by `dialog(...)`).
    dialog_strings_by_name: BTreeMap<String, u8>,
    /// 1-based string indices across all `STR#` declarations (None when ambiguous).
    strings_by_name: BTreeMap<String, Option<u8>>,
}

fn bcon_id_to_table(id: u16) -> Option<u16> {
    match id {
        4096..=4159 => Some(id - 4096),
        8192..=8255 => Some((id - 8192) + 64),
        256..=319 => Some((id - 256) + 128),
        _ => None,
    }
}

/// Lowers a parsed `.antics` [`Module`] into deterministic [`BhavChunk`],
/// [`BconChunk`], and [`StringTableChunk`] outputs with full jump target and
/// frame-dimension validation.
pub fn lower_module(module: &Module) -> Result<LoweredModule> {
    let mut symbols = ModuleSymbols {
        tree_ids_by_name: BTreeMap::new(),
        bcon_by_name: BTreeMap::new(),
        dialog_strings_by_name: BTreeMap::new(),
        strings_by_name: BTreeMap::new(),
    };

    let mut constants = BTreeMap::new();
    let mut strings = BTreeMap::new();

    for item in &module.items {
        match item {
            Item::Tree(tree) => {
                if let Some(prev) = symbols.tree_ids_by_name.insert(tree.label.clone(), tree.id)
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
            Item::Bcon(bcon) => {
                if bcon.values.len() > 255 {
                    return Err(Error::at(
                        bcon.span,
                        "BCON table cannot exceed 255 constants",
                    ));
                }
                let has_names = bcon.entry_names.iter().any(Option::is_some);
                if has_names {
                    let table = bcon_id_to_table(bcon.id).ok_or_else(|| {
                        Error::at(
                            bcon.span,
                            format!(
                                "named BCON id {} is outside addressable ranges \
                                 (256..=319 global, 4096..=4159 private, 8192..=8255 semi-global)",
                                bcon.id
                            ),
                        )
                    })?;
                    for (idx, maybe_name) in bcon.entry_names.iter().enumerate() {
                        let Some(name) = maybe_name else { continue };
                        if idx > 127 {
                            return Err(Error::at(
                                bcon.span,
                                format!(
                                    "named BCON entry '{name}' at index {idx} exceeds 7-bit entry limit (127)"
                                ),
                            ));
                        }
                        let addr = ((table << 7) | idx as u16) as i16;
                        if symbols.bcon_by_name.insert(name.clone(), addr).is_some() {
                            return Err(Error::at(
                                bcon.span,
                                format!("duplicate named BCON constant '{name}'"),
                            ));
                        }
                    }
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
            Item::Str(decl) => {
                let mut table_entries = Vec::with_capacity(decl.entries.len());
                let mut seen_in_table = std::collections::BTreeSet::new();
                for (idx, entry) in decl.entries.iter().enumerate() {
                    if let Some(name) = &entry.name {
                        let one_based = u8::try_from(idx + 1).map_err(|_| {
                            Error::at(
                                entry.span,
                                format!(
                                    "named string '{name}' at 1-based index {} exceeds u8 limit (255)",
                                    idx + 1
                                ),
                            )
                        })?;
                        if !seen_in_table.insert(name.clone()) {
                            return Err(Error::at(
                                entry.span,
                                format!("duplicate string name '{name}' in STR# {}", decl.id),
                            ));
                        }
                        if decl.id == 301 {
                            symbols
                                .dialog_strings_by_name
                                .insert(name.clone(), one_based);
                        }
                        symbols
                            .strings_by_name
                            .entry(name.clone())
                            .and_modify(|slot| *slot = None)
                            .or_insert(Some(one_based));
                    }
                    table_entries.push(StringEntry::with_comment(&entry.text, &entry.comment));
                }
                let chunk = StringTableChunk::new(
                    *b"STR#",
                    decl.id,
                    &decl.label,
                    StringFormat::Localized,
                    table_entries,
                );
                if strings.insert(chunk.id, chunk).is_some() {
                    return Err(Error::at(
                        decl.span,
                        format!("duplicate STR# chunk id {}", decl.id),
                    ));
                }
            }
        }
    }

    let mut behaviors = BTreeMap::new();
    for item in &module.items {
        if let Item::Tree(tree) = item {
            let bhav = lower_tree(tree, &symbols)?;
            if behaviors.insert(bhav.id, bhav).is_some() {
                return Err(Error::at(
                    tree.span,
                    format!("duplicate BHAV tree id {}", tree.id),
                ));
            }
        }
    }

    Ok(LoweredModule {
        behaviors,
        tree_ids_by_name: symbols.tree_ids_by_name,
        constants,
        strings,
    })
}

#[derive(Debug, Clone)]
enum PendingTarget {
    Resolved(u8),
    Label { name: String, span: Span },
    Node { index: usize, span: Span },
    Sentinel(u32),
}

#[derive(Debug, Clone)]
struct PendingNode {
    span: Span,
    opcode: u16,
    operands: [u8; 8],
    on_true: PendingTarget,
    on_false: PendingTarget,
}

#[derive(Debug, Default)]
struct TreeUsage {
    only_raw_ops: bool,
    has_raw_ops: bool,
    uses_idle: bool,
    uses_param0: bool,
    max_explicit_local: Option<usize>,
    let_decls: Vec<(Span, String)>,
}

fn scan_tree_usage(stmts: &[Stmt]) -> TreeUsage {
    let mut usage = TreeUsage {
        only_raw_ops: true,
        ..TreeUsage::default()
    };
    scan_stmts(stmts, &mut usage);
    usage
}

fn scan_operand(op: &OperandRef, usage: &mut TreeUsage) {
    if let OperandRef::Direct(direct) = op {
        if matches!(direct.scope, 9 | 22) && direct.index == 0 {
            usage.uses_param0 = true;
        } else if direct.scope == 25 && direct.index >= 0 {
            let idx = direct.index as usize;
            usage.max_explicit_local = Some(usage.max_explicit_local.map_or(idx, |m| m.max(idx)));
        }
    }
}

fn scan_local_ref(local: &LocalRef, usage: &mut TreeUsage) {
    if let LocalRef::Index(idx) = local {
        let idx = usize::from(*idx);
        usage.max_explicit_local = Some(usage.max_explicit_local.map_or(idx, |m| m.max(idx)));
    }
}

fn scan_primitive(prim: &PrimitiveCall, usage: &mut TreeUsage) {
    match prim {
        PrimitiveCall::Sleep { param_index } | PrimitiveCall::IdleForInput { param_index, .. } => {
            if *param_index == 0 {
                usage.uses_param0 = true;
            }
        }
        PrimitiveCall::ExprRaw { lhs, rhs, .. } => {
            scan_operand(lhs, usage);
            scan_operand(rhs, usage);
        }
        PrimitiveCall::Random { dst, range } => {
            scan_operand(dst, usage);
            scan_operand(range, usage);
        }
        PrimitiveCall::AlterBudget { amount, .. } => {
            scan_operand(amount, usage);
        }
        PrimitiveCall::SetMotiveChange {
            delta_per_hour,
            stop_at,
            ..
        } => {
            scan_operand(delta_per_hour, usage);
            scan_operand(stop_at, usage);
        }
        PrimitiveCall::SetToNext {
            mode, dest, local, ..
        } => {
            if mode & 0x7F == 6 {
                // Mode 6 matches objects whose data word 59 equals param[0] (`set_to_next.rs:28`).
                usage.uses_param0 = true;
            }
            if mode & 0x7F == 9 {
                scan_local_ref(local, usage);
            }
            if let Some(d) = dest {
                scan_operand(d, usage);
            }
        }
        PrimitiveCall::CreateObject { mode, local, .. } => {
            if *mode == 7 {
                usage.uses_param0 = true;
            } else if *mode == 8 || *mode == 9 {
                scan_local_ref(local, usage);
            }
        }
        PrimitiveCall::Animate {
            clip_id,
            flags,
            events,
            local_index,
            ..
        } => {
            // Flag bit 0x04 takes the clip number from param[clip_id].
            if flags & 0x04 != 0 && *clip_id == 0 {
                usage.uses_param0 = true;
            }
            // Flag bit 0x20 stores events in local[local_index]; otherwise foreground
            // animation events write to param[0] (`animate.rs:13`).
            if flags & 0x20 != 0 {
                scan_local_ref(local_index, usage);
            } else if *events != 0 && matches!(flags & 0x11, 0x00 | 0x11) {
                usage.uses_param0 = true;
            }
        }
        _ => {}
    }
}

fn scan_condition(cond: &Condition, usage: &mut TreeUsage) {
    match cond {
        Condition::Not(inner) => scan_condition(inner, usage),
        Condition::Compare { lhs, rhs, .. } => {
            scan_operand(lhs, usage);
            scan_operand(rhs, usage);
        }
        Condition::Call { .. } => {}
        Condition::Primitive { prim, .. } => scan_primitive(prim, usage),
    }
}

fn scan_stmts(stmts: &[Stmt], usage: &mut TreeUsage) {
    for stmt in stmts {
        match stmt {
            Stmt::Label { .. } => {}
            Stmt::Let { span, name, init } => {
                usage.only_raw_ops = false;
                usage.let_decls.push((*span, name.clone()));
                if let Some(op) = init {
                    scan_operand(op, usage);
                }
            }
            Stmt::RawOp { .. } => {
                usage.has_raw_ops = true;
            }
            Stmt::Primitive { prim, .. } => {
                usage.only_raw_ops = false;
                scan_primitive(prim, usage);
            }
            Stmt::Call { .. } => {
                usage.only_raw_ops = false;
            }
            Stmt::Assign { lhs, rhs, .. } => {
                usage.only_raw_ops = false;
                scan_operand(lhs, usage);
                scan_operand(rhs, usage);
            }
            Stmt::Idle { ticks, .. } => {
                usage.only_raw_ops = false;
                usage.uses_idle = true;
                scan_operand(ticks, usage);
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                usage.only_raw_ops = false;
                scan_condition(cond, usage);
                scan_stmts(then_body, usage);
                scan_stmts(else_body, usage);
            }
            Stmt::While { cond, body, .. } => {
                usage.only_raw_ops = false;
                scan_condition(cond, usage);
                scan_stmts(body, usage);
            }
            Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Return { .. } => {
                usage.only_raw_ops = false;
            }
        }
    }
}

struct TreeBuilder<'a> {
    symbols: &'a ModuleSymbols,
    named_locals: BTreeMap<String, u8>,
    nodes: Vec<PendingNode>,
    labels: BTreeMap<String, usize>,
    loop_stack: Vec<(PendingTarget, PendingTarget)>,
    next_sentinel_id: u32,
    max_local_needed: usize,
    max_param_needed: usize,
    idle_scratch_local: Option<usize>,
    max_allowed_args: usize,
    max_allowed_locals: usize,
}

impl<'a> TreeBuilder<'a> {
    fn new(
        symbols: &'a ModuleSymbols,
        named_locals: BTreeMap<String, u8>,
        min_locals_needed: usize,
        idle_scratch_local: Option<usize>,
        max_allowed_args: usize,
        max_allowed_locals: usize,
    ) -> Self {
        Self {
            symbols,
            named_locals,
            nodes: Vec::new(),
            labels: BTreeMap::new(),
            loop_stack: Vec::new(),
            next_sentinel_id: 1,
            max_local_needed: min_locals_needed,
            max_param_needed: 0,
            idle_scratch_local,
            max_allowed_args,
            max_allowed_locals,
        }
    }

    fn alloc_sentinel(&mut self) -> u32 {
        let id = self.next_sentinel_id;
        self.next_sentinel_id = self.next_sentinel_id.wrapping_add(1);
        id
    }

    fn resolve_operand(&mut self, op: &OperandRef, span: Span) -> Result<Operand> {
        let resolved = match op {
            OperandRef::Direct(direct) => *direct,
            OperandRef::NamedLocal(name) => {
                let &idx = self.named_locals.get(name).ok_or_else(|| {
                    Error::at(
                        span,
                        format!("unknown operand or undeclared local '{name}'"),
                    )
                })?;
                Operand::new(25, i16::from(idx))
            }
            OperandRef::NamedBcon(name) => {
                let &addr = self.symbols.bcon_by_name.get(name).ok_or_else(|| {
                    Error::at(span, format!("unknown named BCON constant 'bcon.{name}'"))
                })?;
                Operand::new(26, addr)
            }
            OperandRef::NamedStr(name) => match self.symbols.strings_by_name.get(name) {
                Some(Some(idx)) => Operand::literal(i16::from(*idx)),
                Some(None) => {
                    return Err(Error::at(
                        span,
                        format!(
                            "ambiguous string name 'str.{name}' declared in multiple STR# tables"
                        ),
                    ));
                }
                None => {
                    return Err(Error::at(
                        span,
                        format!("unknown string reference 'str.{name}'"),
                    ));
                }
            },
        };
        self.note_operand(resolved, span)?;
        Ok(resolved)
    }

    fn resolve_local_ref(
        &mut self,
        local: &LocalRef,
        span: Span,
        record_usage: bool,
    ) -> Result<u8> {
        let idx = match local {
            LocalRef::Index(i) => *i,
            LocalRef::Named(name) => *self
                .named_locals
                .get(name)
                .ok_or_else(|| Error::at(span, format!("undeclared local variable '{name}'")))?,
        };
        if record_usage {
            self.note_operand(Operand::new(25, i16::from(idx)), span)?;
        }
        Ok(idx)
    }

    fn resolve_dialog_string(&self, sref: &StringRef, span: Span) -> Result<u8> {
        match sref {
            StringRef::Index(idx) => Ok(*idx),
            StringRef::Named(name) => self
                .symbols
                .dialog_strings_by_name
                .get(name)
                .copied()
                .ok_or_else(|| {
                    Error::at(
                        span,
                        format!("dialog string 'str.{name}' not found in STR# 301"),
                    )
                }),
        }
    }

    fn note_operand(&mut self, op: Operand, span: Span) -> Result<()> {
        if op.scope == 25 {
            let idx = usize::try_from(op.index).map_err(|_| {
                Error::at(span, format!("local index {} cannot be negative", op.index))
            })?;
            let needed = idx + 1;
            if needed > self.max_allowed_locals {
                return Err(Error::at(
                    span,
                    format!(
                        "local[{idx}] exceeds format limit of {} locals",
                        self.max_allowed_locals
                    ),
                ));
            }
            self.max_local_needed = self.max_local_needed.max(needed);
        } else if matches!(op.scope, 9 | 22) {
            let idx = usize::try_from(op.index).map_err(|_| {
                Error::at(
                    span,
                    format!("parameter index {} cannot be negative", op.index),
                )
            })?;
            let needed = idx + 1;
            if needed > self.max_allowed_args {
                return Err(Error::at(
                    span,
                    format!(
                        "param[{idx}] exceeds format limit of {} arguments",
                        self.max_allowed_args
                    ),
                ));
            }
            self.max_param_needed = self.max_param_needed.max(needed);
        }
        Ok(())
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

    fn emit_return_node(&mut self, span: Span, ret: PendingTarget) -> Result<usize> {
        let noop_ops = encode_expr_operands(Operand::literal(0), 2, Operand::literal(0));
        self.emit(span, 2, noop_ops, ret.clone(), ret)
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
            Target::Raw(b) => PendingTarget::Resolved(*b),
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
                .symbols
                .tree_ids_by_name
                .get(name)
                .copied()
                .ok_or_else(|| Error::at(span, format!("unknown subroutine tree '{name}'"))),
        }
    }

    fn encode_primitive(&mut self, prim: &PrimitiveCall, span: Span) -> Result<(u16, [u8; 8])> {
        match prim {
            PrimitiveCall::Sleep { param_index } => {
                self.note_operand(Operand::new(9, *param_index), span)?;
                let [p0, p1] = param_index.to_le_bytes();
                Ok((0, [p0, p1, 0, 0, 0, 0, 0, 0]))
            }
            PrimitiveCall::GenericCall { routine } => {
                let [r0, r1] = routine.to_le_bytes();
                Ok((1, [r0, r1, 0, 0, 0, 0, 0, 0]))
            }
            PrimitiveCall::ExprRaw { lhs, operator, rhs } => {
                let lhs = self.resolve_operand(lhs, span)?;
                let rhs = self.resolve_operand(rhs, span)?;
                Ok((2, encode_expr_operands(lhs, *operator, rhs)))
            }
            PrimitiveCall::Random { dst, range } => {
                let dst = self.resolve_operand(dst, span)?;
                let range = self.resolve_operand(range, span)?;
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
                self.note_operand(Operand::new(9, *param_index), span)?;
                let [p0, p1] = param_index.to_le_bytes();
                let [a0, a1] = allow_push.to_le_bytes();
                Ok((17, [p0, p1, a0, a1, 0, 0, 0, 0]))
            }
            PrimitiveCall::RemoveObject { target, flags } => {
                if *target > 2 {
                    return Err(Error::at(
                        span,
                        format!(
                            "remove_object target {target} out of valid range (0=me, 1=stack_obj, 2=0)"
                        ),
                    ));
                }
                let [t0, t1] = target.to_le_bytes();
                Ok((18, [t0, t1, *flags, 0, 0, 0, 0, 0]))
            }
            PrimitiveCall::RunFunctionalTree {
                function,
                change_icon,
            } => {
                let [f0, f1] = function.to_le_bytes();
                Ok((20, [f0, f1, u8::from(*change_icon), 0, 0, 0, 0, 0]))
            }
            PrimitiveCall::PlaySound {
                sound_id,
                use_stack_object,
            } => {
                let [s0, s1] = sound_id.to_le_bytes();
                let flags = if *use_stack_object { 2 } else { 0 };
                Ok((23, [s0, s1, 0, 0, flags, 0, 0, 0]))
            }
            PrimitiveCall::AlterBudget {
                amount,
                category,
                flags,
            } => {
                if *category > 8 {
                    return Err(Error::at(
                        span,
                        format!("alter_budget category {category} out of 0..=8 range"),
                    ));
                }
                let amount = self.resolve_operand(amount, span)?;
                let scope_sel = match amount.scope {
                    7 => 0u8,
                    9 => 1u8,
                    25 => 2u8,
                    _ => 3u8,
                };
                let [i0, i1] = amount.index.to_le_bytes();
                Ok((
                    25,
                    [scope_sel, amount.scope, i0, i1, *flags, 0, *category, 0],
                ))
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
                let delta_per_hour = self.resolve_operand(delta_per_hour, span)?;
                let stop_at = self.resolve_operand(stop_at, span)?;
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
            PrimitiveCall::SetToNext {
                mode,
                guid,
                dest,
                local,
            } => {
                let base_mode = *mode & 0x7F;
                if base_mode == 6 {
                    self.note_operand(Operand::new(9, 0), span)?;
                }
                let local_byte = self.resolve_local_ref(local, span, base_mode == 9)?;
                let (mode_byte, dest_scope, dest_index) = match dest {
                    Some(d) => {
                        let resolved = self.resolve_operand(d, span)?;
                        if resolved.scope == 7 {
                            return Err(Error::at(
                                span,
                                "set_to_next destination cannot be a read-only literal",
                            ));
                        }
                        let idx = u8::try_from(resolved.index).map_err(|_| {
                            Error::at(
                                span,
                                format!(
                                    "set_to_next destination index {} out of 0..=255 range",
                                    resolved.index
                                ),
                            )
                        })?;
                        (base_mode | 0x80, resolved.scope, idx)
                    }
                    None => (base_mode, 0, 0),
                };
                let [g0, g1, g2, g3] = guid.to_le_bytes();
                Ok((
                    31,
                    [
                        g0, g1, g2, g3, mode_byte, dest_scope, local_byte, dest_index,
                    ],
                ))
            }
            PrimitiveCall::Dialog {
                kind,
                message,
                title,
                button1,
                button2,
                button3,
                mode4_str,
                flags,
            } => {
                // Dialog operand layout (`sims-vm/src/primitives/dialog.rs:712`):
                // [0]=button3, [1]=mode4_str, [2]=message, [3]=button1,
                // [4]=button2, [5]=kind, [6]=title, [7]=flags.
                let message = self.resolve_dialog_string(message, span)?;
                let title = self.resolve_dialog_string(title, span)?;
                let button1 = self.resolve_dialog_string(button1, span)?;
                let button2 = self.resolve_dialog_string(button2, span)?;
                let button3 = self.resolve_dialog_string(button3, span)?;
                let mode4_str = self.resolve_dialog_string(mode4_str, span)?;
                Ok((
                    36,
                    [
                        button3,
                        mode4_str,
                        message,
                        button1,
                        button2,
                        *kind & 0x7F,
                        title,
                        *flags,
                    ],
                ))
            }
            PrimitiveCall::CreateObject {
                guid,
                mode,
                flags,
                local,
            } => {
                if *mode == 7 {
                    self.note_operand(Operand::new(9, 0), span)?;
                }
                let local_byte = self.resolve_local_ref(local, span, *mode == 8 || *mode == 9)?;
                let [g0, g1, g2, g3] = guid.to_le_bytes();
                Ok((42, [g0, g1, g2, g3, *mode, *flags, local_byte, 0]))
            }
            PrimitiveCall::Animate {
                clip_id,
                table,
                flags,
                events,
                local_index,
            } => {
                let local_byte = self.resolve_local_ref(local_index, span, flags & 0x20 != 0)?;
                if flags & 0x20 == 0 && *events != 0 && matches!(flags & 0x11, 0x00 | 0x11) {
                    self.note_operand(Operand::new(9, 0), span)?;
                }
                if flags & 0x04 != 0 {
                    self.note_operand(Operand::new(9, *clip_id as i16), span)?;
                }
                let [c0, c1] = clip_id.to_le_bytes();
                Ok((
                    44,
                    [c0, c1, local_byte, 0, *table, *flags, *events as u8, 0],
                ))
            }
            PrimitiveCall::RouteTo { slot, flags } => {
                // Go To Routing Slot (opcode 45): word 0 = slot, word 1 = source
                // (1 = literal object routing slot; 0 = parameter index), byte 4 = flags.
                let [s0, s1] = slot.to_le_bytes();
                Ok((45, [s0, s1, 1, 0, *flags, 0, 0, 0]))
            }
            PrimitiveCall::Snap { slot, flags } => {
                // Snap (opcode 46): word 0 = slot, word 1 = mode
                // (3 = literal object routing slot; 0 = parameter index), byte 4 = flags.
                let [s0, s1] = slot.to_le_bytes();
                Ok((46, [s0, s1, 3, 0, *flags, 0, 0, 0]))
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
        if operands == [0xFF; 8] {
            return Err(Error::at(
                span,
                "four literal -1 call arguments collide with the VM's all-0xFF 'temps' sentinel; \
                 pass them via temps instead",
            ));
        }
        Ok(operands)
    }

    fn resolve_jump_stmt(&self, stmt: &Stmt) -> Result<Option<PendingTarget>> {
        match stmt {
            Stmt::Return { span, target } => Ok(Some(self.target_from_ast(target, *span))),
            Stmt::Break { span } => self
                .loop_stack
                .last()
                .map(|(_, brk)| Some(brk.clone()))
                .ok_or_else(|| Error::at(*span, "'break' cannot be used outside a 'while' loop")),
            Stmt::Continue { span } => self
                .loop_stack
                .last()
                .map(|(cont, _)| Some(cont.clone()))
                .ok_or_else(|| {
                    Error::at(*span, "'continue' cannot be used outside a 'while' loop")
                }),
            _ => Ok(None),
        }
    }

    /// Lowers a sequence of statements whose fallthrough exits to `next_target`.
    /// Returns the entry target of this statement sequence.
    fn lower_block(&mut self, stmts: &[Stmt], next_target: PendingTarget) -> Result<PendingTarget> {
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
        // Filter out zero-instruction uninitialized `let name;` declarations before
        // computing fallthrough targets so a trailing declaration never leaves a
        // dangling branch to `nodes.len()`.
        let active: Vec<&Stmt> = stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::Let { init: None, .. }))
            .collect();
        for (idx, &stmt) in active.iter().enumerate() {
            let is_last = idx + 1 == active.len();
            let fallthrough = if is_last {
                next_target.clone()
            } else {
                PendingTarget::Resolved(TARGET_TRUE)
            };
            let before = self.nodes.len();
            match stmt {
                Stmt::Label { span, name } => {
                    if self.labels.insert(name.clone(), self.nodes.len()).is_some() {
                        return Err(Error::at(*span, format!("duplicate label '{name}'")));
                    }
                }
                Stmt::Let { init: None, .. } => {}
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
                Stmt::Return { span, target } => {
                    // Emit a concrete terminating node so any preceding label points to
                    // this return and execution never falls through into later statements.
                    let ret = self.target_from_ast(target, *span);
                    self.emit_return_node(*span, ret)?;
                }
                Stmt::Break { span } | Stmt::Continue { span } => {
                    let target = self.resolve_jump_stmt(stmt)?.expect("jump stmt");
                    self.emit_return_node(*span, target)?;
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
                    let sentinel_id = self.alloc_sentinel();
                    let entry = self.lower_structured_stmt(
                        stmt,
                        if is_last {
                            next_target.clone()
                        } else {
                            PendingTarget::Sentinel(sentinel_id)
                        },
                    )?;
                    if self.nodes.len() == before
                        && let PendingTarget::Resolved(ret) = entry
                    {
                        self.emit_return_node(Span::default(), PendingTarget::Resolved(ret))?;
                    }
                    if !is_last {
                        let next_idx = self.nodes.len();
                        for node in &mut self.nodes[before..] {
                            patch_sentinel(&mut node.on_true, sentinel_id, next_idx);
                            patch_sentinel(&mut node.on_false, sentinel_id, next_idx);
                        }
                    }
                    continue;
                }
            }
            if !is_last
                && self.nodes.len() > before
                && matches!(
                    stmt,
                    Stmt::Primitive { branches: None, .. } | Stmt::Call { branches: None, .. }
                )
            {
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
        // Filter out zero-instruction `let name;` declarations (slots are already allocated).
        let active: Vec<&Stmt> = stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::Let { init: None, .. }))
            .collect();
        if active.is_empty() {
            return Ok(next_target);
        }
        let mut i = 0usize;
        let mut entry = None;
        while i < active.len() {
            let is_last = i + 1 == active.len();
            let stmt = active[i];
            if let Some(jump) = self.resolve_jump_stmt(stmt)? {
                if entry.is_none() {
                    return Ok(jump);
                }
                break;
            }
            let (after, sentinel_id) = if is_last {
                (next_target.clone(), None)
            } else if let Some(jump) = self.resolve_jump_stmt(active[i + 1])? {
                let stmt_entry = self.lower_structured_stmt(stmt, jump)?;
                if entry.is_none() {
                    entry = Some(stmt_entry);
                }
                break;
            } else {
                let sid = self.alloc_sentinel();
                (PendingTarget::Sentinel(sid), Some(sid))
            };
            let before = self.nodes.len();
            let stmt_entry = self.lower_structured_stmt(stmt, after)?;
            if entry.is_none() {
                entry = Some(stmt_entry);
            }
            if let Some(sid) = sentinel_id {
                let next_idx = self.nodes.len();
                for node in &mut self.nodes[before..] {
                    patch_sentinel(&mut node.on_true, sid, next_idx);
                    patch_sentinel(&mut node.on_false, sid, next_idx);
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
            Stmt::Return { .. } | Stmt::Break { .. } | Stmt::Continue { .. } => {
                Ok(self.resolve_jump_stmt(stmt)?.expect("jump stmt"))
            }
            Stmt::Let {
                span,
                name,
                init: Some(rhs),
            } => {
                let lhs = self.resolve_operand(&OperandRef::NamedLocal(name.clone()), *span)?;
                let rhs = self.resolve_operand(rhs, *span)?;
                let operands = encode_expr_operands(lhs, AssignOp::Set.code(), rhs);
                let idx = self.emit(*span, 2, operands, next_target.clone(), next_target)?;
                Ok(PendingTarget::Node {
                    index: idx,
                    span: *span,
                })
            }
            Stmt::Let { init: None, .. } => Ok(next_target),
            Stmt::Assign { span, lhs, op, rhs } => {
                let lhs = self.resolve_operand(lhs, *span)?;
                let rhs = self.resolve_operand(rhs, *span)?;
                let operands = encode_expr_operands(lhs, op.code(), rhs);
                let idx = self.emit(*span, 2, operands, next_target.clone(), next_target)?;
                Ok(PendingTarget::Node {
                    index: idx,
                    span: *span,
                })
            }
            Stmt::Idle { span, ticks } => {
                let ticks = self.resolve_operand(ticks, *span)?;
                self.note_operand(Operand::new(9, 0), *span)?;
                if let Some(scratch) = self.idle_scratch_local {
                    // Preserve caller's param[0] around Sleep (opcode 0 counts param[0] down to 0).
                    let scratch_op = Operand::new(25, scratch as i16);
                    self.note_operand(scratch_op, *span)?;
                    let save_idx = self.nodes.len();
                    let set_idx = save_idx + 1;
                    let sleep_idx = save_idx + 2;
                    let restore_idx = save_idx + 3;
                    let save_ops = encode_expr_operands(scratch_op, 5, Operand::new(9, 0));
                    let set_ops = encode_expr_operands(Operand::new(9, 0), 5, ticks);
                    let restore_ops = encode_expr_operands(Operand::new(9, 0), 5, scratch_op);
                    let to_set = PendingTarget::Node {
                        index: set_idx,
                        span: *span,
                    };
                    let to_sleep = PendingTarget::Node {
                        index: sleep_idx,
                        span: *span,
                    };
                    let to_restore = PendingTarget::Node {
                        index: restore_idx,
                        span: *span,
                    };
                    self.emit(*span, 2, save_ops, to_set.clone(), to_set)?;
                    self.emit(*span, 2, set_ops, to_sleep.clone(), to_sleep)?;
                    self.emit(*span, 0, [0; 8], to_restore.clone(), to_restore)?;
                    self.emit(*span, 2, restore_ops, next_target.clone(), next_target)?;
                    Ok(PendingTarget::Node {
                        index: save_idx,
                        span: *span,
                    })
                } else {
                    let sleep_idx_placeholder = PendingTarget::Node {
                        index: self.nodes.len() + 1,
                        span: *span,
                    };
                    let set_ops = encode_expr_operands(Operand::new(9, 0), 5, ticks);
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
                self.loop_stack
                    .push((loop_back.clone(), next_target.clone()));
                let body_result = self.lower_structured_slice(body, loop_back);
                self.loop_stack.pop();
                let body_target = body_result?;
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
                let lhs = self.resolve_operand(lhs, *span)?;
                let rhs = self.resolve_operand(rhs, *span)?;
                let operands = encode_expr_operands(lhs, op.code(), rhs);
                self.emit(*span, 2, operands, on_true, on_false)
            }
            Condition::Call {
                span,
                target,
                args,
                pass_temps,
            } => {
                let opcode = self.resolve_call_target(target, *span)?;
                let operands = self.encode_call_operands(args, *pass_temps, *span)?;
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
            PendingTarget::Sentinel(id) => Err(Error::new(format!(
                "internal compiler error: unpatched branch sentinel #{id}"
            ))),
        }
    }
}

fn cond_is_negated(cond: &Condition) -> bool {
    match cond {
        Condition::Not(inner) => !cond_is_negated(inner),
        _ => false,
    }
}

fn patch_sentinel(target: &mut PendingTarget, sentinel_id: u32, actual_next: usize) {
    if let PendingTarget::Sentinel(id) = target
        && *id == sentinel_id
    {
        *target = PendingTarget::Node {
            index: actual_next,
            span: Span::default(),
        };
    }
}

fn encode_expr_operands(lhs: Operand, operator: u8, rhs: Operand) -> [u8; 8] {
    let [l0, l1] = lhs.index.to_le_bytes();
    let [r0, r1] = rhs.index.to_le_bytes();
    [l0, l1, r0, r1, 0, operator, lhs.scope, rhs.scope]
}

fn format_limits(format: u16) -> Option<(usize, usize)> {
    // Matches `sims-vm/src/interpreter.rs:1340` (`dimensions`).
    match format {
        0x8000 => Some((4, 0)),
        0x8001 => Some((4, 4)),
        0x8002 => Some((255, 255)),
        _ => None,
    }
}

fn lower_tree(tree: &TreeDecl, symbols: &ModuleSymbols) -> Result<BhavChunk> {
    let (max_allowed_args, max_allowed_locals) =
        format_limits(tree.header.format).ok_or_else(|| {
            Error::at(
                tree.span,
                format!(
                    "unsupported BHAV format {:#06x} (supported: 0x8000, 0x8001, 0x8002)",
                    tree.header.format
                ),
            )
        })?;

    let usage = scan_tree_usage(&tree.body);
    if usage.has_raw_ops && usage.uses_idle {
        return Err(Error::at(
            tree.span,
            format!(
                "tree '{}' mixes raw 'op' instructions with high-level 'idle()'; \
                 raw operands may alias locals/params — use explicit 'sleep(param[N])' instead",
                tree.label
            ),
        ));
    }

    // Deterministically assign named `let`/`local` variables after any explicit `local[N]`.
    let mut named_locals = BTreeMap::new();
    let mut next_local = usage.max_explicit_local.map_or(0, |m| m + 1);
    for (span, name) in &usage.let_decls {
        if named_locals.contains_key(name) {
            return Err(Error::at(
                *span,
                format!("duplicate local variable '{name}' in tree '{}'", tree.label),
            ));
        }
        if next_local >= max_allowed_locals {
            return Err(Error::at(
                *span,
                format!(
                    "local variable '{name}' (slot {next_local}) exceeds format {:#06x}'s \
                     limit of {max_allowed_locals} locals",
                    tree.header.format
                ),
            ));
        }
        named_locals.insert(name.clone(), next_local as u8);
        next_local += 1;
    }

    let min_locals_needed = next_local;
    let idle_scratch_local = if usage.uses_idle && usage.uses_param0 {
        let scratch = min_locals_needed;
        if scratch >= max_allowed_locals {
            return Err(Error::at(
                tree.span,
                format!(
                    "tree '{}' uses both param[0] and idle(), requiring a scratch local[{scratch}] \
                     which exceeds format {:#06x}'s limit of {max_allowed_locals} locals",
                    tree.label, tree.header.format
                ),
            ));
        }
        Some(scratch)
    } else {
        None
    };

    let mut builder = TreeBuilder::new(
        symbols,
        named_locals,
        min_locals_needed,
        idle_scratch_local,
        max_allowed_args,
        max_allowed_locals,
    );
    let entry = builder.lower_block(&tree.body, PendingTarget::Resolved(TARGET_TRUE))?;

    // If a high-level tree consisted solely of `return true/false/error`, emit a
    // clean 1-node no-op comparison (`0 == 0`) that immediately returns that value.
    // An explicitly empty tree `{}` stays at 0 instructions for lossless lifting.
    if builder.nodes.is_empty() && !tree.body.is_empty() {
        let ret_code = match entry {
            PendingTarget::Resolved(b) => b,
            _ => TARGET_TRUE,
        };
        builder.emit_return_node(tree.span, PendingTarget::Resolved(ret_code))?;
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

    // Verbatim raw-op trees (such as lifted original BHAVs) preserve exact header args/locals.
    let (args, locals) = if usage.only_raw_ops {
        (
            tree.header.args.unwrap_or(0),
            tree.header.locals.unwrap_or(0),
        )
    } else {
        let locals = match tree.header.locals {
            Some(declared) => {
                if usize::from(declared) > max_allowed_locals {
                    return Err(Error::at(
                        tree.span,
                        format!(
                            "tree '{}' declares {declared} locals, exceeding format {:#06x}'s \
                             limit of {max_allowed_locals}",
                            tree.label, tree.header.format
                        ),
                    ));
                }
                if usize::from(declared) < builder.max_local_needed {
                    return Err(Error::at(
                        tree.span,
                        format!(
                            "tree '{}' declares {declared} locals but requires at least {}",
                            tree.label, builder.max_local_needed
                        ),
                    ));
                }
                declared
            }
            None => builder.max_local_needed as u16,
        };

        let args = match tree.header.args {
            Some(declared) => {
                if tree.header.format != 0x8000 {
                    if usize::from(declared) > max_allowed_args {
                        return Err(Error::at(
                            tree.span,
                            format!(
                                "tree '{}' declares {declared} args, exceeding format {:#06x}'s \
                                 limit of {max_allowed_args}",
                                tree.label, tree.header.format
                            ),
                        ));
                    }
                    if usize::from(declared) < builder.max_param_needed {
                        return Err(Error::at(
                            tree.span,
                            format!(
                                "tree '{}' declares {declared} args but requires at least {}",
                                tree.label, builder.max_param_needed
                            ),
                        ));
                    }
                }
                declared
            }
            None => {
                if tree.header.format == 0x8000 {
                    0
                } else {
                    builder.max_param_needed as u8
                }
            }
        };
        (args, locals)
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
        trailing: tree.header.trailing.clone(),
    })
}
