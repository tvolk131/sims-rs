//! Deterministic lowering from `.air` (Antics IR) AST (both raw exact and
//! structured IR trees) to validated `BHAV`, `BCON`, and `STR#` chunks.

use crate::ast::{
    AssignOp, CallTarget, Condition, Item, LocalRef, Module, Operand, OperandRef, PrimitiveCall,
    SlotSpec, Stmt, StringRef, Target, TreeDecl,
};
use crate::error::{Error, Result, Span};
use crate::ir::{BconChunk, BhavChunk, StringEntry, StringFormat, StringTableChunk};
use crate::verify::apply_operand_pad;
use sims_formats::behavior::Instruction;
use std::collections::{BTreeMap, BTreeSet};

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
    /// Tree labels shared by multiple distinct BHAV IDs (valid for numeric calls/raw lifts,
    /// rejected if referenced ambiguously by name).
    ambiguous_tree_names: BTreeSet<String>,
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
        ambiguous_tree_names: BTreeSet::new(),
        bcon_by_name: BTreeMap::new(),
        dialog_strings_by_name: BTreeMap::new(),
        strings_by_name: BTreeMap::new(),
    };

    let mut constants = BTreeMap::new();
    let mut strings = BTreeMap::new();

    for item in &module.items {
        match item {
            Item::Tree(tree) => {
                if symbols.ambiguous_tree_names.contains(&tree.label) {
                    continue;
                }
                if let Some(prev) = symbols.tree_ids_by_name.insert(tree.label.clone(), tree.id)
                    && prev != tree.id
                {
                    symbols.tree_ids_by_name.remove(&tree.label);
                    symbols.ambiguous_tree_names.insert(tree.label.clone());
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
    is_verbatim_explicit: bool,
    has_raw_ops: bool,
    uses_idle: bool,
    uses_param0: bool,
    /// Every local slot the tree names explicitly (`local[N]`, numeric local arguments).
    explicit_locals: BTreeSet<usize>,
    let_decls: Vec<(Span, String)>,
}

/// The local-slot facts `.air` allocation works from: the explicitly named slots, the `let`
/// names in declaration order, and whether raw `op` bytes may name slots no scan can see.
#[derive(Debug)]
pub(crate) struct AirLocalUsage {
    pub(crate) explicit: BTreeSet<usize>,
    pub(crate) lets: Vec<String>,
    pub(crate) has_raw_ops: bool,
    /// The tree uses both `param[0]` and `idle()`, so `.air` keeps `param[0]` in the word after
    /// the named locals around each `idle`.
    pub(crate) uses_idle_scratch: bool,
}

/// [`AirLocalUsage`] of a parsed tree body.
pub(crate) fn air_local_usage(stmts: &[Stmt]) -> AirLocalUsage {
    let usage = scan_tree_usage(stmts);
    AirLocalUsage {
        explicit: usage.explicit_locals,
        lets: usage.let_decls.into_iter().map(|(_, name)| name).collect(),
        has_raw_ops: usage.has_raw_ops,
        uses_idle_scratch: usage.uses_idle && usage.uses_param0,
    }
}

fn scan_tree_usage(stmts: &[Stmt]) -> TreeUsage {
    let mut usage = TreeUsage {
        is_verbatim_explicit: true,
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
            usage.explicit_locals.insert(idx);
        }
    }
}

fn scan_local_ref(local: &LocalRef, usage: &mut TreeUsage) {
    if let LocalRef::Index(idx) = local {
        let idx = usize::from(*idx);
        usage.explicit_locals.insert(idx);
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
        PrimitiveCall::GetDistanceTo { flags, from, .. } => {
            if flags & 1 != 0
                && let Some(op) = from
            {
                scan_operand(op, usage);
            }
        }
        PrimitiveCall::GetDirectionTo { dst, flags, from } => {
            scan_operand(dst, usage);
            if flags & 1 != 0
                && let Some(op) = from
            {
                scan_operand(op, usage);
            }
        }
        PrimitiveCall::Breakpoint { cond } => {
            scan_operand(cond, usage);
        }
        PrimitiveCall::Padded { call, .. } => scan_primitive(call, usage),
        PrimitiveCall::MakeNewCharacter { skin, age, sex, .. } => {
            scan_local_ref(skin, usage);
            scan_local_ref(age, usage);
            scan_local_ref(sex, usage);
        }
        PrimitiveCall::OldRelationship {
            param, direction, ..
        } => {
            if *param == 0 || matches!(direction, 2 | 3) {
                usage.uses_param0 = true;
            }
        }
        PrimitiveCall::UiEffect { mode, target, .. } => {
            if *mode <= 2
                && let Some(op) = target
            {
                scan_operand(op, usage);
            }
        }
        PrimitiveCall::SpecialEffect {
            duration, flags, ..
        } => {
            if flags & 0x20 != 0 && *duration >= 0 {
                let idx = *duration as usize;
                usage.explicit_locals.insert(idx);
            }
        }
        PrimitiveCall::Reach {
            mode, slot_param, ..
        } => {
            if *mode == 1 && *slot_param == 0 {
                usage.uses_param0 = true;
            }
        }
        PrimitiveCall::PushInteraction {
            object_slot,
            flags,
            icon_local,
            ..
        } => {
            if flags & 2 != 0 {
                let idx = usize::from(*object_slot);
                usage.explicit_locals.insert(idx);
            } else if *object_slot == 0 {
                usage.uses_param0 = true;
            }
            if flags & 1 != 0 {
                scan_local_ref(icon_local, usage);
            }
        }
        PrimitiveCall::FindBestObject { flags, origin, .. } => {
            if flags & 8 != 0
                && let Some(op) = origin
            {
                scan_operand(op, usage);
            }
        }
        PrimitiveCall::FindLocation { local, flags, .. } => {
            if flags & 1 != 0 {
                scan_local_ref(local, usage);
            }
        }
        PrimitiveCall::AlterBudget { amount, .. } => {
            scan_operand(amount, usage);
        }
        PrimitiveCall::Relationship {
            direction,
            local,
            value,
            ..
        } => {
            if matches!(direction, 2 | 3) {
                scan_local_ref(local, usage);
            }
            scan_operand(value, usage);
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
        PrimitiveCall::TestObjectType { object, .. } => {
            scan_operand(object, usage);
        }
        PrimitiveCall::TestSimInteractingWith { person: Some(p) } => {
            scan_operand(p, usage);
        }
        PrimitiveCall::SetHeadline {
            kind, index, local, ..
        } => {
            if *kind == 7 && *index == 2 {
                scan_local_ref(local, usage);
            }
        }
        PrimitiveCall::CreateObject { mode, local, .. } => {
            if *mode == 7 {
                usage.uses_param0 = true;
            } else if *mode == 8 || *mode == 9 {
                scan_local_ref(local, usage);
            }
        }
        PrimitiveCall::DropOnto { src, dst } => {
            if matches!(src, SlotSpec::Param(0)) || matches!(dst, SlotSpec::Param(0)) {
                usage.uses_param0 = true;
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
        PrimitiveCall::RouteTo { slot, source, .. } if *source == 0 && *slot == 0 => {
            usage.uses_param0 = true;
        }
        PrimitiveCall::Snap { slot, mode, .. } if *mode == 0 && *slot == 0 => {
            usage.uses_param0 = true;
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
        Condition::RawOp { .. } => {
            usage.has_raw_ops = true;
        }
    }
}

fn scan_stmts(stmts: &[Stmt], usage: &mut TreeUsage) {
    for stmt in stmts {
        match stmt {
            Stmt::Label { .. } => {}
            Stmt::Let { span, name, init } => {
                usage.is_verbatim_explicit = false;
                usage.let_decls.push((*span, name.clone()));
                if let Some(op) = init {
                    scan_operand(op, usage);
                }
            }
            Stmt::RawOp { .. } => {
                usage.has_raw_ops = true;
            }
            Stmt::InlineRawOp { .. } => {
                usage.is_verbatim_explicit = false;
                usage.has_raw_ops = true;
            }
            Stmt::Primitive { prim, branches, .. } => {
                if branches.is_none() {
                    usage.is_verbatim_explicit = false;
                }
                scan_primitive(prim, usage);
            }
            Stmt::Call { branches, .. } => {
                if branches.is_none() {
                    usage.is_verbatim_explicit = false;
                }
            }
            Stmt::Assign { lhs, rhs, .. } => {
                usage.is_verbatim_explicit = false;
                scan_operand(lhs, usage);
                scan_operand(rhs, usage);
            }
            Stmt::Idle { ticks, .. } => {
                usage.is_verbatim_explicit = false;
                usage.uses_idle = true;
                scan_operand(ticks, usage);
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                usage.is_verbatim_explicit = false;
                scan_condition(cond, usage);
                scan_stmts(then_body, usage);
                scan_stmts(else_body, usage);
            }
            Stmt::While { cond, body, .. } => {
                usage.is_verbatim_explicit = false;
                scan_condition(cond, usage);
                scan_stmts(body, usage);
            }
            Stmt::Loop { body, .. } => {
                usage.is_verbatim_explicit = false;
                scan_stmts(body, usage);
            }
            Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Return { .. } => {
                usage.is_verbatim_explicit = false;
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
    is_verbatim_explicit: bool,
}

impl<'a> TreeBuilder<'a> {
    fn new(
        symbols: &'a ModuleSymbols,
        named_locals: BTreeMap<String, u8>,
        min_locals_needed: usize,
        idle_scratch_local: Option<usize>,
        max_allowed_args: usize,
        max_allowed_locals: usize,
        is_verbatim_explicit: bool,
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
            is_verbatim_explicit,
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
        let max_nodes = if self.is_verbatim_explicit {
            usize::from(u16::MAX)
        } else {
            MAX_BHAV_NODES
        };
        if idx >= max_nodes {
            return Err(Error::at(
                span,
                format!("BHAV tree exceeds maximum of {max_nodes} instructions"),
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
            CallTarget::Name(name) => {
                if self.symbols.ambiguous_tree_names.contains(name) {
                    return Err(Error::at(
                        span,
                        format!("ambiguous subroutine tree name '{name}'"),
                    ));
                }
                self.symbols
                    .tree_ids_by_name
                    .get(name)
                    .copied()
                    .ok_or_else(|| Error::at(span, format!("unknown subroutine tree '{name}'")))
            }
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
            PrimitiveCall::FindBestInteraction => Ok((3, [0; 8])),
            PrimitiveCall::Grab => Ok((4, [0; 8])),
            PrimitiveCall::Drop => Ok((5, [0; 8])),
            PrimitiveCall::ChangeSuit {
                index,
                scope,
                remove,
            } => Ok((6, [*index, *scope, u8::from(*remove), 0, 0, 0, 0, 0])),
            PrimitiveCall::Refresh { target, mode } => {
                let [t0, t1] = target.to_le_bytes();
                let [m0, m1] = mode.to_le_bytes();
                Ok((7, [t0, t1, m0, m1, 0, 0, 0, 0]))
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
            PrimitiveCall::Burn { mode, flags } => Ok((9, [*mode, *flags, 0, 0, 0, 0, 0, 0])),
            PrimitiveCall::Tutorial { mode } => Ok((10, [*mode, 0, 0, 0, 0, 0, 0, 0])),
            PrimitiveCall::GetDistanceTo { temp, flags, from } => {
                if *temp >= 8 {
                    return Err(Error::at(
                        span,
                        format!("get_distance_to temp {temp} out of 0..=7 range"),
                    ));
                }
                let [t0, t1] = u16::from(*temp).to_le_bytes();
                if flags & 1 != 0 {
                    let op = from.as_ref().ok_or_else(|| {
                        Error::at(
                            span,
                            "get_distance_to with flag bit 0 requires a 'from' operand",
                        )
                    })?;
                    let resolved = self.resolve_operand(op, span)?;
                    let [i0, i1] = resolved.index.to_le_bytes();
                    Ok((11, [t0, t1, *flags, resolved.scope, i0, i1, 0, 0]))
                } else {
                    Ok((11, [t0, t1, *flags, 0, 0, 0, 0, 0]))
                }
            }
            PrimitiveCall::GetDirectionTo { dst, flags, from } => {
                let dst = self.resolve_operand(dst, span)?;
                let [d0, d1] = dst.index.to_le_bytes();
                let [ds0, ds1] = u16::from(dst.scope).to_le_bytes();
                if flags & 1 != 0 {
                    let op = from.as_ref().ok_or_else(|| {
                        Error::at(
                            span,
                            "get_direction_to with flag bit 0 requires a 'from' operand",
                        )
                    })?;
                    let resolved = self.resolve_operand(op, span)?;
                    let [i0, i1] = resolved.index.to_le_bytes();
                    Ok((12, [d0, d1, ds0, ds1, *flags, resolved.scope, i0, i1]))
                } else {
                    Ok((12, [d0, d1, ds0, ds1, *flags, 0, 0, 0]))
                }
            }
            PrimitiveCall::Breakpoint { cond } => {
                let cond = self.resolve_operand(cond, span)?;
                let [i0, i1] = cond.index.to_le_bytes();
                let [s0, s1] = u16::from(cond.scope).to_le_bytes();
                Ok((15, [i0, i1, s0, s1, 0, 0, 0, 0]))
            }
            PrimitiveCall::BreakpointInvalidScope { scope, index } => {
                // Scopes 0..=255 are operands, spelled `breakpoint(scope(N, I))`.
                if *scope <= 255 {
                    return Err(Error::at(
                        span,
                        format!(
                            "breakpoint scope word {scope} is a valid scope; write breakpoint(scope({scope}, {index}))"
                        ),
                    ));
                }
                let [i0, i1] = index.to_le_bytes();
                let [s0, s1] = scope.to_le_bytes();
                Ok((15, [i0, i1, s0, s1, 0, 0, 0, 0]))
            }
            PrimitiveCall::Padded { call, pad } => {
                if matches!(**call, PrimitiveCall::Padded { .. }) {
                    return Err(Error::at(span, "a primitive takes one pad clause"));
                }
                let (opcode, base) = self.encode_primitive(call, span)?;
                let operands = apply_operand_pad(opcode, base, pad)
                    .map_err(|message| Error::at(span, message))?;
                Ok((opcode, operands))
            }
            PrimitiveCall::MakeNewCharacter {
                skin,
                age,
                sex,
                species,
            } => {
                let skin = self.resolve_local_ref(skin, span, true)?;
                let age = self.resolve_local_ref(age, span, true)?;
                let sex = self.resolve_local_ref(sex, span, true)?;
                Ok((19, [skin, age, sex, *species, 0, 0, 0, 0]))
            }
            PrimitiveCall::ShowString { table, index } => {
                let [t0, t1] = table.to_le_bytes();
                let [i0, i1] = index.to_le_bytes();
                Ok((21, [t0, t1, i0, i1, 0, 0, 0, 0]))
            }
            PrimitiveCall::LookTowards { mode } => {
                let [m0, m1] = mode.to_le_bytes();
                Ok((22, [m0, m1, 0, 0, 0, 0, 0, 0]))
            }
            PrimitiveCall::OldRelationship {
                write,
                column,
                param,
                direction,
                flags,
            } => {
                self.note_operand(Operand::new(9, i16::from(*param)), span)?;
                if matches!(direction, 2 | 3) {
                    self.note_operand(Operand::new(9, 0), span)?;
                }
                Ok((
                    24,
                    [
                        u8::from(*write),
                        *column,
                        *param as u8,
                        *direction,
                        *flags,
                        0,
                        0,
                        0,
                    ],
                ))
            }
            PrimitiveCall::GosubFoundAction => Ok((30, [0; 8])),
            PrimitiveCall::FindWorstMotives { target, mode } => {
                let [t0, t1] = target.to_le_bytes();
                let [m0, m1] = mode.to_le_bytes();
                Ok((33, [0, 0, 0, 0, t0, t1, m0, m1]))
            }
            PrimitiveCall::UiEffect {
                mode,
                target,
                flags,
            } => {
                if *mode <= 2 {
                    let op = target.as_ref().ok_or_else(|| {
                        Error::at(
                            span,
                            format!("ui_effect mode {mode} requires a 'target' operand"),
                        )
                    })?;
                    let resolved = self.resolve_operand(op, span)?;
                    let [i0, i1] = resolved.index.to_le_bytes();
                    Ok((34, [*mode, resolved.scope, i0, i1, *flags, 0, 0, 0]))
                } else if let Some(op) = target {
                    let resolved = self.resolve_operand(op, span)?;
                    let [i0, i1] = resolved.index.to_le_bytes();
                    Ok((34, [*mode, resolved.scope, i0, i1, *flags, 0, 0, 0]))
                } else {
                    Ok((34, [*mode, 0, 0, 0, *flags, 0, 0, 0]))
                }
            }
            PrimitiveCall::SpecialEffect {
                duration,
                kind,
                size,
                flags,
                str_index,
            } => {
                if flags & 0x20 != 0 {
                    self.note_operand(Operand::new(25, *duration), span)?;
                }
                let [d0, d1] = duration.to_le_bytes();
                Ok((35, [d0, d1, *kind, *size, *flags, *str_index, 0, 0]))
            }
            PrimitiveCall::Reach {
                mode,
                moves,
                slot_param,
            } => {
                if *mode == 1 {
                    self.note_operand(Operand::new(9, *slot_param), span)?;
                }
                let [m0, m1] = mode.to_le_bytes();
                let [p0, p1] = slot_param.to_le_bytes();
                Ok((47, [m0, m1, u8::from(*moves), 0, p0, p1, 0, 0]))
            }
            PrimitiveCall::PushInteraction {
                menu,
                object_slot,
                priority,
                flags,
                icon_local,
            } => {
                if flags & 2 != 0 {
                    self.note_operand(Operand::new(25, i16::from(*object_slot)), span)?;
                } else {
                    self.note_operand(Operand::new(9, i16::from(*object_slot)), span)?;
                }
                let icon_byte = self.resolve_local_ref(icon_local, span, flags & 1 != 0)?;
                Ok((
                    13,
                    [*menu, *object_slot, *priority, *flags, icon_byte, 0, 0, 0],
                ))
            }
            PrimitiveCall::FindBestObject {
                function,
                flags,
                origin,
            } => {
                let [f0, f1] = function.to_le_bytes();
                if flags & 8 != 0 {
                    let op = origin.as_ref().ok_or_else(|| {
                        Error::at(
                            span,
                            "find_best_object with flag bit 8 requires an 'origin' operand",
                        )
                    })?;
                    let resolved = self.resolve_operand(op, span)?;
                    let [i0, i1] = resolved.index.to_le_bytes();
                    Ok((14, [f0, f1, i0, i1, resolved.scope, 0, *flags, 0]))
                } else {
                    Ok((14, [f0, f1, 0, 0, 0, 0, *flags, 0]))
                }
            }
            PrimitiveCall::FindLocation { mode, local, flags } => {
                let local_byte = self.resolve_local_ref(local, span, flags & 1 != 0)?;
                Ok((16, [*mode, local_byte, *flags, 0, 0, 0, 0, 0]))
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
            PrimitiveCall::PlaySound { sound_id, flags } => {
                let [s0, s1] = sound_id.to_le_bytes();
                Ok((23, [s0, s1, 0, 0, *flags, 0, 0, 0]))
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
            PrimitiveCall::Relationship {
                column,
                direction,
                flags,
                local,
                value,
            } => {
                let local_byte = self.resolve_local_ref(local, span, matches!(direction, 2 | 3))?;
                let resolved = self.resolve_operand(value, span)?;
                let [v0, v1] = resolved.index.to_le_bytes();
                Ok((
                    26,
                    [
                        *column,
                        *direction,
                        *flags,
                        local_byte,
                        resolved.scope,
                        0,
                        v0,
                        v1,
                    ],
                ))
            }
            PrimitiveCall::GoToRelative {
                location,
                facing,
                options,
            } => Ok((27, [0, 0, *location as u8, *facing, 0, 0, *options, 0])),
            PrimitiveCall::RunTreeByName {
                table,
                index,
                mode,
                global,
            } => {
                let [t0, t1] = table.to_le_bytes();
                Ok((28, [t0, t1, u8::from(*global), 0, *index, *mode, 0, 0]))
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
            PrimitiveCall::TestObjectType { guid, object } => {
                let resolved = self.resolve_operand(object, span)?;
                let [g0, g1, g2, g3] = guid.to_le_bytes();
                let [i0, i1] = resolved.index.to_le_bytes();
                Ok((32, [g0, g1, g2, g3, i0, i1, resolved.scope, 0]))
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
                        button3, mode4_str, message, button1, button2,
                        // The VM shows kind `& 0x7F` but keeps the whole byte in the
                        // dialog record (`dialog.rs`), so it is written as given.
                        *kind, title, *flags,
                    ],
                ))
            }
            PrimitiveCall::TestSimInteractingWith { person } => match person {
                None => Ok((37, [0; 8])),
                Some(op) => {
                    let resolved = self.resolve_operand(op, span)?;
                    let [i0, i1] = resolved.index.to_le_bytes();
                    Ok((37, [1, resolved.scope, i0, i1, 0, 0, 0, 0]))
                }
            },
            PrimitiveCall::SetHeadline {
                use_stack_object,
                local,
                index,
                kind,
                duration,
                options,
            } => {
                let local_byte = self.resolve_local_ref(local, span, *kind == 7 && *index == 2)?;
                let w0 = (u16::from(local_byte) << 1) | u16::from(*use_stack_object);
                let [w0_lo, w0_hi] = w0.to_le_bytes();
                let [d0, d1] = duration.to_le_bytes();
                let [o0, o1] = options.to_le_bytes();
                Ok((41, [w0_lo, w0_hi, *index, *kind, d0, d1, o0, o1]))
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
                if (*mode == 8 || *mode == 9) && local_byte >= 128 {
                    return Err(Error::at(
                        span,
                        format!(
                            "create_object mode {mode} local index {local_byte} exceeds signed i8 range 0..=127"
                        ),
                    ));
                }
                let [g0, g1, g2, g3] = guid.to_le_bytes();
                Ok((42, [g0, g1, g2, g3, *mode, *flags, local_byte, 0]))
            }
            PrimitiveCall::DropOnto { src, dst } => {
                let (sk, sv) = match *src {
                    SlotSpec::Literal(v) => (0i16, v),
                    SlotSpec::Param(p) => {
                        self.note_operand(Operand::new(9, p), span)?;
                        (1i16, p)
                    }
                };
                let (dk, dv) = match *dst {
                    SlotSpec::Literal(v) => (0i16, v),
                    SlotSpec::Param(p) => {
                        self.note_operand(Operand::new(9, p), span)?;
                        (1i16, p)
                    }
                };
                let [sk0, sk1] = sk.to_le_bytes();
                let [sv0, sv1] = sv.to_le_bytes();
                let [dk0, dk1] = dk.to_le_bytes();
                let [dv0, dv1] = dv.to_le_bytes();
                Ok((43, [sk0, sk1, sv0, sv1, dk0, dk1, dv0, dv1]))
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
            PrimitiveCall::RouteTo {
                slot,
                source,
                flags,
            } => {
                // Go To Routing Slot (opcode 45): word 0 = slot/param, word 1 = source
                // (0 = param, 1 = literal object slot, 2 = global slot), byte 4 = flags.
                if *source == 0 {
                    self.note_operand(Operand::new(9, *slot as i16), span)?;
                }
                let [s0, s1] = slot.to_le_bytes();
                let [src0, src1] = source.to_le_bytes();
                Ok((45, [s0, s1, src0, src1, *flags, 0, 0, 0]))
            }
            PrimitiveCall::Snap { slot, mode, flags } => {
                // Snap (opcode 46): word 0 = slot/param, word 1 = mode
                // (0 = param, 1 = object container 0, 2 = front, 3 = object slot, 4 = global slot),
                // byte 4 = flags.
                if *mode == 0 {
                    self.note_operand(Operand::new(9, *slot as i16), span)?;
                }
                let [s0, s1] = slot.to_le_bytes();
                let [m0, m1] = mode.to_le_bytes();
                Ok((46, [s0, s1, m0, m1, *flags, 0, 0, 0]))
            }
            PrimitiveCall::NotifyIdle => Ok((49, [0; 8])),
            PrimitiveCall::ActionString { table, index } => {
                let [t0, t1] = table.to_le_bytes();
                Ok((50, [t0, t1, 0, 0, *index, 0, 0, 0]))
            }
            PrimitiveCall::ManageInventory {
                operation,
                kind,
                flags,
                guid,
            } => {
                let [f0, f1] = flags.to_le_bytes();
                let [g0, g1, g2, g3] = guid.to_le_bytes();
                Ok((51, [*operation, *kind, f0, f1, g0, g1, g2, g3]))
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
        let has_explicit_cfg = stmts.iter().any(|s| {
            matches!(
                s,
                Stmt::Label { .. }
                    | Stmt::RawOp { .. }
                    | Stmt::Primitive {
                        branches: Some(_),
                        ..
                    }
                    | Stmt::Call {
                        branches: Some(_),
                        ..
                    }
            )
        });

        if has_explicit_cfg {
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
        let mut pending_sids: Vec<u32> = Vec::new();
        for (idx, &stmt) in active.iter().enumerate() {
            let is_last = idx + 1 == active.len();
            let before = self.nodes.len();
            let fallthrough = if is_last {
                next_target.clone()
            } else {
                let sid = self.alloc_sentinel();
                PendingTarget::Sentinel(sid)
            };
            let fallthrough_sid = match fallthrough {
                PendingTarget::Sentinel(sid) if !is_last => Some(sid),
                _ => None,
            };
            match stmt {
                Stmt::Label { span, name } => {
                    if self.labels.insert(name.clone(), self.nodes.len()).is_some() {
                        return Err(Error::at(*span, format!("duplicate label '{name}'")));
                    }
                    continue;
                }
                Stmt::Let { init: None, .. } => continue,
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
                Stmt::InlineRawOp {
                    span,
                    opcode,
                    operands,
                } => {
                    self.emit(
                        *span,
                        *opcode,
                        *operands,
                        fallthrough.clone(),
                        fallthrough.clone(),
                    )?;
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
                    let mut entry = self.lower_structured_stmt(stmt, fallthrough)?;
                    if self.nodes.len() == before
                        && let PendingTarget::Resolved(ret) = entry
                        && self.labels.values().any(|&pos| pos == before)
                    {
                        self.emit_return_node(Span::default(), PendingTarget::Resolved(ret))?;
                        entry = PendingTarget::Node {
                            index: before,
                            span: Span::default(),
                        };
                    }
                    for psid in pending_sids.drain(..) {
                        for node in &mut self.nodes[start_node..] {
                            patch_sentinel_target(&mut node.on_true, psid, &entry);
                            patch_sentinel_target(&mut node.on_false, psid, &entry);
                        }
                    }
                    if let Some(sid) = fallthrough_sid
                        && slice_uses_sentinel(&self.nodes[start_node..], &entry, sid)
                    {
                        pending_sids.push(sid);
                    }
                    continue;
                }
            }
            if self.nodes.len() > before {
                let target = PendingTarget::Node {
                    index: before,
                    span: self.nodes[before].span,
                };
                for psid in pending_sids.drain(..) {
                    for node in &mut self.nodes[start_node..] {
                        patch_sentinel_target(&mut node.on_true, psid, &target);
                        patch_sentinel_target(&mut node.on_false, psid, &target);
                    }
                }
                if let Some(sid) = fallthrough_sid
                    && matches!(
                        stmt,
                        Stmt::InlineRawOp { .. }
                            | Stmt::Primitive { branches: None, .. }
                            | Stmt::Call { branches: None, .. }
                    )
                {
                    pending_sids.push(sid);
                }
            }
        }
        for psid in pending_sids {
            for node in &mut self.nodes[start_node..] {
                patch_sentinel_target(&mut node.on_true, psid, &next_target);
                patch_sentinel_target(&mut node.on_false, psid, &next_target);
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
        let slice_start = self.nodes.len();
        let mut i = 0usize;
        let mut entry: Option<PendingTarget> = None;
        let mut prev_sid: Option<u32> = None;
        while i < active.len() {
            let is_last = i + 1 == active.len();
            let stmt = active[i];
            if let Some(jump) = self.resolve_jump_stmt(stmt)? {
                if let Some(psid) = prev_sid.take() {
                    for node in &mut self.nodes[slice_start..] {
                        patch_sentinel_target(&mut node.on_true, psid, &jump);
                        patch_sentinel_target(&mut node.on_false, psid, &jump);
                    }
                    if let Some(ent) = &mut entry {
                        patch_sentinel_target(ent, psid, &jump);
                    }
                }
                if entry.is_none() {
                    return Ok(jump);
                }
                break;
            }
            let (after, next_sid) = if is_last {
                (next_target.clone(), None)
            } else if let Some(jump) = self.resolve_jump_stmt(active[i + 1])? {
                let stmt_entry = self.lower_structured_stmt(stmt, jump)?;
                if let Some(psid) = prev_sid.take() {
                    for node in &mut self.nodes[slice_start..] {
                        patch_sentinel_target(&mut node.on_true, psid, &stmt_entry);
                        patch_sentinel_target(&mut node.on_false, psid, &stmt_entry);
                    }
                    if let Some(ent) = &mut entry {
                        patch_sentinel_target(ent, psid, &stmt_entry);
                    }
                }
                if entry.is_none() {
                    entry = Some(stmt_entry);
                }
                break;
            } else {
                let sid = self.alloc_sentinel();
                (PendingTarget::Sentinel(sid), Some(sid))
            };
            let stmt_entry = self.lower_structured_stmt(stmt, after)?;
            if let Some(psid) = prev_sid.take() {
                for node in &mut self.nodes[slice_start..] {
                    patch_sentinel_target(&mut node.on_true, psid, &stmt_entry);
                    patch_sentinel_target(&mut node.on_false, psid, &stmt_entry);
                }
                if let Some(ent) = &mut entry {
                    patch_sentinel_target(ent, psid, &stmt_entry);
                }
            }
            if entry.is_none() {
                entry = Some(stmt_entry.clone());
            }
            if let Some(nsid) = next_sid {
                // If this statement never falls through to `nsid` (for example,
                // `while true { return false; }`), still lower the remaining dead
                // statements into throwaway nodes so semantic errors (unknown trees,
                // bad operands, invalid break/continue) are still diagnosed, then
                // truncate those unreachable nodes so dead code never occupies node 0.
                if !slice_uses_sentinel(&self.nodes[slice_start..], &stmt_entry, nsid) {
                    let saved_nodes = self.nodes.len();
                    for dead_stmt in &active[i + 1..] {
                        let _ = self.lower_structured_stmt(dead_stmt, next_target.clone())?;
                    }
                    self.nodes.truncate(saved_nodes);
                    break;
                }
                prev_sid = Some(nsid);
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
            Stmt::Loop { span, body } => {
                let header_sid = self.alloc_sentinel();
                let loop_back = PendingTarget::Sentinel(header_sid);
                let before = self.nodes.len();
                self.loop_stack
                    .push((loop_back.clone(), next_target.clone()));
                let body_result = self.lower_structured_slice(body, loop_back);
                self.loop_stack.pop();
                let body_target = body_result?;
                let resolved_entry = match body_target {
                    PendingTarget::Sentinel(id) if id == header_sid => {
                        let idx = self.nodes.len();
                        let self_target = PendingTarget::Node {
                            index: idx,
                            span: *span,
                        };
                        self.emit_return_node(*span, self_target.clone())?;
                        self_target
                    }
                    other => other,
                };
                for node in &mut self.nodes[before..] {
                    patch_sentinel_target(&mut node.on_true, header_sid, &resolved_entry);
                    patch_sentinel_target(&mut node.on_false, header_sid, &resolved_entry);
                }
                Ok(resolved_entry)
            }
            Stmt::InlineRawOp {
                span,
                opcode,
                operands,
            } => {
                let idx = self.emit(*span, *opcode, *operands, next_target.clone(), next_target)?;
                Ok(PendingTarget::Node {
                    index: idx,
                    span: *span,
                })
            }
            Stmt::Label { span, .. } | Stmt::RawOp { span, .. } => Err(Error::at(
                *span,
                "labels and explicit-branch raw ops cannot appear inside nested structured blocks",
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
            Condition::RawOp {
                span,
                opcode,
                operands,
            } => self.emit(*span, *opcode, *operands, on_true, on_false),
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
                if idx >= count || idx >= MAX_BHAV_NODES {
                    return Err(Error::at(
                        *span,
                        format!(
                            "label '@{name}' points past addressable node limit (node {idx}, count {count})"
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

fn patch_sentinel_target(
    target: &mut PendingTarget,
    sentinel_id: u32,
    replacement: &PendingTarget,
) {
    if let PendingTarget::Sentinel(id) = target
        && *id == sentinel_id
    {
        *target = replacement.clone();
    }
}

fn slice_uses_sentinel(nodes: &[PendingNode], entry: &PendingTarget, sentinel_id: u32) -> bool {
    matches!(entry, PendingTarget::Sentinel(id) if *id == sentinel_id)
        || nodes.iter().any(|n| {
            matches!(n.on_true, PendingTarget::Sentinel(id) if id == sentinel_id)
                || matches!(n.on_false, PendingTarget::Sentinel(id) if id == sentinel_id)
        })
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

    // Deterministically assign named `let`/`local` variables after any explicit `local[N]` and
    // the header's `min_locals`.
    let mut named_locals = BTreeMap::new();
    let min_locals = usize::from(tree.header.min_locals);
    if min_locals > max_allowed_locals {
        return Err(Error::at(
            tree.span,
            format!(
                "tree '{}' asks for min_locals {min_locals}, exceeding format {:#06x}'s limit of \
                 {max_allowed_locals}",
                tree.label, tree.header.format
            ),
        ));
    }
    let mut next_local = usage
        .explicit_locals
        .last()
        .map_or(0, |m| m + 1)
        .max(min_locals);
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
        usage.is_verbatim_explicit,
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

    // Verbatim explicit-branch trees (such as lifted original BHAVs) preserve exact header
    // `args`/`locals` when explicitly declared (including non-standard high-byte local counts
    // or 0x8000 header bytes), while still enforcing bounds for any decoded operand accesses.
    let locals = match tree.header.locals {
        Some(declared) => {
            let eff_declared = usize::from(declared as u8).min(max_allowed_locals);
            if usage.is_verbatim_explicit
                && (builder.max_local_needed == 0 || eff_declared >= builder.max_local_needed)
            {
                declared
            } else {
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
        }
        None => builder.max_local_needed as u16,
    };

    let args = match tree.header.args {
        Some(declared) => {
            if tree.header.format == 0x8000
                || (usage.is_verbatim_explicit
                    && (builder.max_param_needed == 0
                        || usize::from(declared).min(max_allowed_args) >= builder.max_param_needed))
            {
                declared
            } else {
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
                declared
            }
        }
        None => {
            if tree.header.format == 0x8000 {
                0
            } else {
                builder.max_param_needed as u8
            }
        }
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
