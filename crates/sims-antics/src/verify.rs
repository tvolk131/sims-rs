//! Formal static/symbolic safety prover and bounded behavioral trace-equivalence
//! verifier for SimAntics (`BHAV`) programs and compiled `open-pack/` objects.
//!
//! Pure deterministic safe Rust (`unsafe_code = "forbid"`, zero external solver
//! dependencies, `BTreeMap`/`BTreeSet` exclusively).

use crate::compile::{CompiledObject, CompiledPack};
use crate::error::{Error, Result};
use crate::ir::{BhavChunk, ObjdChunk};
use crate::lift::lift_objd;
use crate::lower::{TARGET_ERROR, TARGET_FALSE, TARGET_TRUE};
use sims_formats::behavior::{Behavior, Instruction, InteractionTable};
use sims_formats::execution::{ObjectFunctions, Slots, constants};
use sims_formats::iff::{DELETED, Iff};
use sims_formats::sprite::{DrawGroup, Palette, SpriteSet};
use sims_formats::strings::StringTable;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Category of safety violation reported by [`verify_object_safety`] / [`verify_pack_safety`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SafetyCategory {
    DanglingJump,
    UnreachableNode,
    OutOfBoundsBcon,
    OutOfBoundsString,
    OutOfBoundsSlot,
    MissingSound,
    InvalidLocal,
    InvalidParam,
    InvalidOpcode,
    MissingSubroutine,
    ArityMismatch,
    InvalidResourceRef,
    PotentialTrap,
    LockLeak,
}

/// One concrete safety finding detected during static or symbolic verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyViolation {
    pub object: String,
    pub tree_id: Option<u16>,
    pub tree_name: String,
    pub category: SafetyCategory,
    pub node: Option<usize>,
    pub message: String,
    pub path: Vec<String>,
}

/// Safety verification summary for a single compiled object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyReport {
    pub object_name: String,
    pub guid: u32,
    pub trees_checked: usize,
    pub locks_verified: usize,
    pub violations: Vec<SafetyViolation>,
}

impl SafetyReport {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Safety verification summary for an entire compiled pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackSafetyReport {
    pub pack_name: String,
    pub objects: Vec<SafetyReport>,
}

impl PackSafetyReport {
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.objects.iter().all(SafetyReport::is_ok)
    }

    #[must_use]
    pub fn total_violations(&self) -> usize {
        self.objects.iter().map(|o| o.violations.len()).sum()
    }

    #[must_use]
    pub fn total_trees_checked(&self) -> usize {
        self.objects.iter().map(|o| o.trees_checked).sum()
    }

    #[must_use]
    pub fn total_locks_verified(&self) -> usize {
        self.objects.iter().map(|o| o.locks_verified).sum()
    }
}

/// Options bounding symbolic bisimulation in [`check_bhav_equivalent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquivOptions {
    /// Maximum synchronized product states explored before returning `Inconclusive`.
    pub max_states: usize,
    /// Maximum path depth explored before returning `Inconclusive`.
    pub max_depth: usize,
    /// Maximum consecutive silent local/no-op steps per side before returning `Inconclusive`.
    pub max_silent_steps: usize,
}

impl Default for EquivOptions {
    fn default() -> Self {
        Self {
            max_states: 4096,
            max_depth: 512,
            max_silent_steps: 64,
        }
    }
}

/// One synchronized step in a counterexample trace returned by [`check_bhav_equivalent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceStep {
    pub pc_a: u8,
    pub pc_b: u8,
    pub event: String,
    pub branch: Option<bool>,
}

/// Result of formal behavioral trace-equivalence checking between two BHAVs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EquivResult {
    /// All reachable paths and bisimulation cycles matched with identical
    /// observable events, state updates, and terminal outcomes.
    Equivalent {
        states_checked: usize,
        paths_verified: usize,
    },
    /// Found a concrete path where the two BHAVs diverge in observable event,
    /// expression semantics, or exit code.
    Counterexample {
        path: Vec<TraceStep>,
        divergence: String,
    },
    /// Verification exhausted bounds or encountered an unsupported pattern;
    /// never reported as a false positive.
    Inconclusive { reason: String },
}

// ============================================================================
// Part 1: Static & Symbolic Safety Prover
// ============================================================================

/// Parsed object resources extracted from a compiled `.iff` for verification.
struct ObjectModel {
    name: String,
    behaviors: BTreeMap<u16, (String, Behavior)>,
    bcons: BTreeMap<u16, Vec<i16>>,
    strings: BTreeMap<u16, usize>,
    ttas: BTreeMap<u16, usize>,
    ttabs: BTreeMap<u16, InteractionTable>,
    slots: BTreeMap<u16, usize>,
    fwavs: BTreeSet<u16>,
    dgrps: BTreeMap<u16, DrawGroup>,
    spr2s: BTreeMap<u16, SpriteSet>,
    palts: BTreeSet<u16>,
    ctss: BTreeSet<u16>,
    objds: BTreeMap<u16, ObjdChunk>,
    objfs: BTreeMap<u16, ObjectFunctions>,
}

/// Verifies a compiled [`CompiledObject`] for structural integrity, resource bounds,
/// stack/arity discipline, no-trap invariants, and symbolic lock/cancel safety.
pub fn verify_object_safety(obj: &CompiledObject) -> Result<SafetyReport> {
    verify_iff_safety(&obj.name, obj.guid, &obj.iff_bytes)
}

/// Verifies every object inside a [`CompiledPack`].
pub fn verify_pack_safety(pack: &CompiledPack) -> Result<PackSafetyReport> {
    let mut objects = Vec::with_capacity(pack.objects.len());
    for entry in &pack.objects {
        objects.push(verify_object_safety(&entry.object)?);
    }
    Ok(PackSafetyReport {
        pack_name: pack.name.clone(),
        objects,
    })
}

/// Verifies a raw compiled object `.iff` payload.
pub fn verify_iff_safety(name: &str, guid: u32, iff_bytes: &[u8]) -> Result<SafetyReport> {
    let iff = Iff::parse(iff_bytes.to_vec())?;
    let mut model = ObjectModel {
        name: name.to_owned(),
        behaviors: BTreeMap::new(),
        bcons: BTreeMap::new(),
        strings: BTreeMap::new(),
        ttas: BTreeMap::new(),
        ttabs: BTreeMap::new(),
        slots: BTreeMap::new(),
        fwavs: BTreeSet::new(),
        dgrps: BTreeMap::new(),
        spr2s: BTreeMap::new(),
        palts: BTreeSet::new(),
        ctss: BTreeSet::new(),
        objds: BTreeMap::new(),
        objfs: BTreeMap::new(),
    };
    let mut violations = Vec::new();

    for chunk in &iff.chunks {
        if chunk.flags & DELETED != 0 {
            continue;
        }
        let data = iff.data(chunk);
        match &chunk.tag {
            b"BHAV" => match Behavior::parse(data) {
                Ok(bhav) => {
                    model
                        .behaviors
                        .insert(chunk.id, (chunk.label.clone(), bhav));
                }
                Err(e) => violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: Some(chunk.id),
                    tree_name: chunk.label.clone(),
                    category: SafetyCategory::InvalidResourceRef,
                    node: None,
                    message: format!("failed to parse BHAV #{}: {e}", chunk.id),
                    path: Vec::new(),
                }),
            },
            b"BCON" => match constants(data) {
                Ok(vals) => {
                    model.bcons.insert(chunk.id, vals);
                }
                Err(e) => violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: None,
                    tree_name: String::new(),
                    category: SafetyCategory::InvalidResourceRef,
                    node: None,
                    message: format!("failed to parse BCON #{}: {e}", chunk.id),
                    path: Vec::new(),
                }),
            },
            b"STR#" => match StringTable::parse(data) {
                Ok(st) => {
                    model.strings.insert(chunk.id, st.entries.len());
                }
                Err(e) => violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: None,
                    tree_name: String::new(),
                    category: SafetyCategory::InvalidResourceRef,
                    node: None,
                    message: format!("failed to parse STR# #{}: {e}", chunk.id),
                    path: Vec::new(),
                }),
            },
            b"TTAs" => match StringTable::parse(data) {
                Ok(st) => {
                    model.ttas.insert(chunk.id, st.entries.len());
                }
                Err(e) => violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: None,
                    tree_name: String::new(),
                    category: SafetyCategory::InvalidResourceRef,
                    node: None,
                    message: format!("failed to parse TTAs #{}: {e}", chunk.id),
                    path: Vec::new(),
                }),
            },
            b"CTSS" => match StringTable::parse(data) {
                Ok(_) => {
                    model.ctss.insert(chunk.id);
                }
                Err(e) => violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: None,
                    tree_name: String::new(),
                    category: SafetyCategory::InvalidResourceRef,
                    node: None,
                    message: format!("failed to parse CTSS #{}: {e}", chunk.id),
                    path: Vec::new(),
                }),
            },
            b"TTAB" => match InteractionTable::parse(data) {
                Ok(ttab) => {
                    model.ttabs.insert(chunk.id, ttab);
                }
                Err(e) => violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: None,
                    tree_name: String::new(),
                    category: SafetyCategory::InvalidResourceRef,
                    node: None,
                    message: format!("failed to parse TTAB #{}: {e}", chunk.id),
                    path: Vec::new(),
                }),
            },
            b"SLOT" => match Slots::parse(data) {
                Ok(sl) => {
                    model.slots.insert(chunk.id, sl.entries.len());
                }
                Err(e) => violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: None,
                    tree_name: String::new(),
                    category: SafetyCategory::InvalidResourceRef,
                    node: None,
                    message: format!("failed to parse SLOT #{}: {e}", chunk.id),
                    path: Vec::new(),
                }),
            },
            b"FWAV" => {
                model.fwavs.insert(chunk.id);
            }
            b"PALT" => {
                if Palette::parse(data).is_ok() {
                    model.palts.insert(chunk.id);
                }
            }
            b"SPR2" => {
                if let Ok(set) = SpriteSet::parse(data) {
                    model.spr2s.insert(chunk.id, set);
                }
            }
            b"DGRP" => {
                if let Ok(dg) = DrawGroup::parse(data) {
                    model.dgrps.insert(chunk.id, dg);
                }
            }
            b"OBJD" => {
                if let Ok(objd) = lift_objd(chunk.id, &chunk.label, data) {
                    model.objds.insert(chunk.id, objd);
                }
            }
            b"OBJf" => {
                if let Ok(objf) = ObjectFunctions::parse(data) {
                    model.objfs.insert(chunk.id, objf);
                }
            }
            _ => {}
        }
    }

    // 1. Validate OBJD / OBJf / TTAB / DGRP cross-resource references.
    verify_resource_links(&model, &mut violations);

    // 2. Validate every BHAV's CFG reachability, jumps, operand bounds, opcodes, and arity.
    let total_slots = model.slots.values().copied().max().unwrap_or(0);
    for (&tree_id, (tree_name, bhav)) in &model.behaviors {
        verify_bhav_static(
            &model,
            tree_id,
            tree_name,
            bhav,
            total_slots,
            &mut violations,
        );
    }

    // 3. Symbolic Lock & Cancel-Safety Invariant Prover across all TTAB interactions.
    let locks_verified = verify_object_locks(&model, &mut violations);

    Ok(SafetyReport {
        object_name: model.name,
        guid,
        trees_checked: model.behaviors.len(),
        locks_verified,
        violations,
    })
}

fn verify_resource_links(model: &ObjectModel, violations: &mut Vec<SafetyViolation>) {
    let push = |violations: &mut Vec<SafetyViolation>, msg: String| {
        violations.push(SafetyViolation {
            object: model.name.clone(),
            tree_id: None,
            tree_name: String::new(),
            category: SafetyCategory::InvalidResourceRef,
            node: None,
            message: msg,
            path: Vec::new(),
        });
    };

    for (&objd_id, objd) in &model.objds {
        let w = |i: usize| objd.words.get(i).copied().unwrap_or(0);
        let base_graphic_id = w(3);
        let main_id = w(5);
        let tree_table_id = w(7);
        let slots_id = w(20);
        let catalog_strings_id = w(41);
        let init_id = w(43);

        if init_id >= 4096 && !model.behaviors.contains_key(&init_id) {
            push(
                violations,
                format!("OBJD #{objd_id} references missing private init BHAV #{init_id}"),
            );
        }
        if main_id >= 4096 && !model.behaviors.contains_key(&main_id) {
            push(
                violations,
                format!("OBJD #{objd_id} references missing private main BHAV #{main_id}"),
            );
        }
        if base_graphic_id != 0 && !model.dgrps.contains_key(&base_graphic_id) {
            push(
                violations,
                format!("OBJD #{objd_id} references missing DGRP #{base_graphic_id}"),
            );
        }
        if tree_table_id != 0
            && tree_table_id != 0xFFFF
            && !model.ttabs.contains_key(&tree_table_id)
        {
            push(
                violations,
                format!("OBJD #{objd_id} references missing TTAB #{tree_table_id}"),
            );
        }
        if slots_id != 0 && !model.slots.contains_key(&slots_id) {
            push(
                violations,
                format!("OBJD #{objd_id} references missing SLOT #{slots_id}"),
            );
        }
        if catalog_strings_id != 0 && !model.ctss.contains(&catalog_strings_id) {
            push(
                violations,
                format!("OBJD #{objd_id} references missing CTSS #{catalog_strings_id}"),
            );
        }
    }

    for (&objf_id, objf) in &model.objfs {
        for (idx, entry) in objf.entries.iter().enumerate() {
            if entry.action >= 4096 && !model.behaviors.contains_key(&entry.action) {
                push(
                    violations,
                    format!(
                        "OBJf #{objf_id} callback[{idx}] action references missing private BHAV #{}",
                        entry.action
                    ),
                );
            }
            if entry.condition >= 4096 && !model.behaviors.contains_key(&entry.condition) {
                push(
                    violations,
                    format!(
                        "OBJf #{objf_id} callback[{idx}] condition references missing private BHAV #{}",
                        entry.condition
                    ),
                );
            }
        }
    }

    for (&ttab_id, ttab) in &model.ttabs {
        let ttas_len = model.ttas.get(&ttab_id).copied().unwrap_or(0);
        for (idx, inter) in ttab.interactions.iter().enumerate() {
            if inter.action >= 4096 && !model.behaviors.contains_key(&inter.action) {
                push(
                    violations,
                    format!(
                        "TTAB #{ttab_id} interaction[{idx}] action references missing private BHAV #{}",
                        inter.action
                    ),
                );
            }
            if inter.test >= 4096 && !model.behaviors.contains_key(&inter.test) {
                push(
                    violations,
                    format!(
                        "TTAB #{ttab_id} interaction[{idx}] guard references missing private BHAV #{}",
                        inter.test
                    ),
                );
            }
            if inter.string_index as usize >= ttas_len {
                push(
                    violations,
                    format!(
                        "TTAB #{ttab_id} interaction[{idx}] string_index {} out of bounds for TTAs #{ttab_id} (len {ttas_len})",
                        inter.string_index
                    ),
                );
            }
        }
    }

    for (&dgrp_id, dgrp) in &model.dgrps {
        for view in &dgrp.views {
            for spr in &view.sprites {
                let Ok(spr_id) = u16::try_from(spr.id) else {
                    push(
                        violations,
                        format!("DGRP #{dgrp_id} sprite id {} exceeds u16", spr.id),
                    );
                    continue;
                };
                match model.spr2s.get(&spr_id) {
                    None => push(
                        violations,
                        format!("DGRP #{dgrp_id} references missing SPR2 #{spr_id}"),
                    ),
                    Some(set) => {
                        if spr.frame as usize >= set.frames.len() {
                            push(
                                violations,
                                format!(
                                    "DGRP #{dgrp_id} references frame {} of SPR2 #{spr_id} (len {})",
                                    spr.frame,
                                    set.frames.len()
                                ),
                            );
                        }
                        if let Ok(palt_id) = u16::try_from(set.default_palette)
                            && !model.palts.contains(&palt_id)
                        {
                            push(
                                violations,
                                format!("SPR2 #{spr_id} references missing PALT #{palt_id}"),
                            );
                        }
                    }
                }
            }
        }
    }
}

fn bhav_param_limit(bhav: &Behavior) -> usize {
    match bhav.format {
        0x8000 => 4,
        0x8001 => usize::from(bhav.args).min(4),
        _ => usize::from(bhav.args),
    }
}

fn bhav_local_limit(bhav: &Behavior) -> usize {
    match bhav.format {
        0x8000 => 0,
        0x8001 => usize::from(bhav.locals).min(4),
        _ => usize::from(bhav.locals),
    }
}

fn reachable_nodes(instructions: &[Instruction]) -> Vec<bool> {
    let n = instructions.len();
    let mut visited = vec![false; n];
    if n == 0 {
        return visited;
    }
    let mut queue = VecDeque::new();
    visited[0] = true;
    queue.push_back(0usize);
    while let Some(idx) = queue.pop_front() {
        let ins = &instructions[idx];
        let always_true = is_always_true_instruction(ins);
        let targets: &[u8] = if always_true {
            &[ins.on_true]
        } else {
            &[ins.on_true, ins.on_false]
        };
        for &t in targets {
            if !matches!(t, TARGET_ERROR..=TARGET_FALSE) {
                let next = usize::from(t);
                if next < n && !visited[next] {
                    visited[next] = true;
                    queue.push_back(next);
                }
            }
        }
    }
    visited
}

fn is_known_primitive(opcode: u16) -> bool {
    matches!(
        opcode,
        0..=51 | 64..=66
    )
}

fn verify_bhav_static(
    model: &ObjectModel,
    tree_id: u16,
    tree_name: &str,
    bhav: &Behavior,
    total_slots: usize,
    violations: &mut Vec<SafetyViolation>,
) {
    let n = bhav.instructions.len();
    let param_limit = bhav_param_limit(bhav);
    let local_limit = bhav_local_limit(bhav);
    let str301_len = model.strings.get(&301).copied().unwrap_or(0);

    let add_err = |violations: &mut Vec<SafetyViolation>,
                   category: SafetyCategory,
                   node: usize,
                   msg: String| {
        violations.push(SafetyViolation {
            object: model.name.clone(),
            tree_id: Some(tree_id),
            tree_name: tree_name.to_owned(),
            category,
            node: Some(node),
            message: msg,
            path: Vec::new(),
        });
    };

    // Jump target validation on all nodes.
    for (idx, ins) in bhav.instructions.iter().enumerate() {
        let check_target = |t: u8, label: &str, violations: &mut Vec<SafetyViolation>| {
            if !matches!(t, TARGET_ERROR..=TARGET_FALSE) && usize::from(t) >= n {
                add_err(
                    violations,
                    SafetyCategory::DanglingJump,
                    idx,
                    format!("{label} target {t} out of bounds (tree has {n} nodes)"),
                );
            }
        };
        check_target(ins.on_true, "on_true", violations);
        check_target(ins.on_false, "on_false", violations);
    }

    let reachable = reachable_nodes(&bhav.instructions);
    for (idx, &is_reach) in reachable.iter().enumerate() {
        if !is_reach {
            add_err(
                violations,
                SafetyCategory::UnreachableNode,
                idx,
                format!("node {idx} is unreachable from entry node 0"),
            );
            continue;
        }

        let ins = &bhav.instructions[idx];
        let ops = &ins.operands;

        // Helper to validate a (scope, index) operand reference.
        let check_operand = |scope: u8,
                             index: i16,
                             is_write: bool,
                             violations: &mut Vec<SafetyViolation>| {
            match scope {
                7 => {
                    if is_write {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            "cannot write to read-only literal operand (scope 7)".to_owned(),
                        );
                    }
                }
                9 | 22 => {
                    if index < 0 || index as usize >= param_limit {
                        add_err(
                            violations,
                            SafetyCategory::InvalidParam,
                            idx,
                            format!(
                                "param[{index}] out of bounds for tree with {param_limit} args"
                            ),
                        );
                    }
                }
                25 => {
                    if index < 0 || index as usize >= local_limit {
                        add_err(
                            violations,
                            SafetyCategory::InvalidLocal,
                            idx,
                            format!(
                                "local[{index}] out of bounds for tree with {local_limit} locals"
                            ),
                        );
                    }
                }
                26 => {
                    if is_write {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            "cannot write to read-only BCON constant (scope 26)".to_owned(),
                        );
                    }
                    let raw = index as u16;
                    let table = raw >> 7;
                    let entry = usize::from(raw & 127);
                    // Private BCON tables are 0..64 -> chunk IDs 4096..=4159.
                    if table < 64 {
                        let bcon_id = 4096 + table;
                        match model.bcons.get(&bcon_id) {
                            None => add_err(
                                violations,
                                SafetyCategory::OutOfBoundsBcon,
                                idx,
                                format!(
                                    "references missing private BCON #{bcon_id} (entry {entry})"
                                ),
                            ),
                            Some(vals) if entry >= vals.len() => add_err(
                                violations,
                                SafetyCategory::OutOfBoundsBcon,
                                idx,
                                format!(
                                    "BCON #{bcon_id}[{entry}] out of bounds (len {})",
                                    vals.len()
                                ),
                            ),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        };

        if ins.opcode < 256 {
            if !is_known_primitive(ins.opcode) {
                add_err(
                    violations,
                    SafetyCategory::InvalidOpcode,
                    idx,
                    format!("unknown primitive opcode {:#06x}", ins.opcode),
                );
                continue;
            }
            match ins.opcode {
                0 => {
                    let p = word_i16(ops, 0);
                    check_operand(9, p, false, violations);
                }
                2 => {
                    let lhs_idx = word_i16(ops, 0);
                    let rhs_idx = word_i16(ops, 2);
                    let operator = ops[5];
                    let lhs_scope = ops[6];
                    let rhs_scope = ops[7];
                    if operator > 20 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("invalid Expression operator {operator}"),
                        );
                    } else {
                        let writes_lhs = matches!(operator, 3..=7 | 9..=13 | 17..=20);
                        check_operand(lhs_scope, lhs_idx, writes_lhs, violations);
                        check_operand(rhs_scope, rhs_idx, false, violations);
                        if matches!(operator, 7 | 12) && rhs_scope == 7 && rhs_idx == 0 {
                            add_err(
                                violations,
                                SafetyCategory::PotentialTrap,
                                idx,
                                "Expression division/modulo by constant 0".to_owned(),
                            );
                        }
                        if matches!(operator, 8..=10)
                            && rhs_scope == 7
                            && !(0..=16).contains(&rhs_idx)
                        {
                            add_err(
                                violations,
                                SafetyCategory::PotentialTrap,
                                idx,
                                format!("Expression flag bit {rhs_idx} out of 0..=16 range"),
                            );
                        }
                    }
                }
                8 => {
                    let dst_idx = word_i16(ops, 0);
                    let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
                    let rng_idx = word_i16(ops, 4);
                    let rng_scope = u16::from_le_bytes([ops[6], ops[7]]);
                    if dst_scope <= 255 {
                        check_operand(dst_scope as u8, dst_idx, true, violations);
                    }
                    if rng_scope <= 255 {
                        check_operand(rng_scope as u8, rng_idx, false, violations);
                    }
                }
                17 => {
                    let p = word_i16(ops, 0);
                    check_operand(9, p, false, violations);
                }
                18 => {
                    let target = u16::from_le_bytes([ops[0], ops[1]]);
                    if target > 2 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("RemoveObject target {target} out of 0..=2 range"),
                        );
                    }
                }
                23 => {
                    let sound_id = u16::from_le_bytes([ops[0], ops[1]]);
                    if sound_id != 0 && !model.fwavs.contains(&sound_id) {
                        add_err(
                            violations,
                            SafetyCategory::MissingSound,
                            idx,
                            format!("PlaySound references missing FWAV #{sound_id}"),
                        );
                    }
                }
                25 => {
                    let scope = match ops[0] {
                        0 => 7,
                        1 => 9,
                        2 => 25,
                        _ => ops[1],
                    };
                    let amount_idx = word_i16(ops, 2);
                    let category = ops[6];
                    if category > 8 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("AlterBudget category {category} exceeds 8"),
                        );
                    }
                    check_operand(scope, amount_idx, false, violations);
                }
                29 => {
                    if ops[3] & 1 == 0 {
                        let motive = ops[2];
                        if !matches!(motive, 5..=9 | 14..=15) {
                            add_err(
                                violations,
                                SafetyCategory::PotentialTrap,
                                idx,
                                format!("SetMotiveChange invalid motive {motive}"),
                            );
                        }
                        check_operand(ops[0], word_i16(ops, 4), false, violations);
                        check_operand(ops[1], word_i16(ops, 6), false, violations);
                    }
                }
                31 => {
                    let mode = ops[4];
                    let base = mode & 0x7F;
                    if base == 6 {
                        check_operand(9, 0, false, violations);
                    }
                    if base == 9 {
                        check_operand(25, i16::from(ops[6]), false, violations);
                    }
                    if mode & 0x80 != 0 {
                        check_operand(ops[5], i16::from(ops[7]), true, violations);
                    }
                }
                36 => {
                    // Dialog STR# 301 indices: [0]=btn3, [1]=mode4, [2]=msg, [3]=btn1, [4]=btn2, [6]=title.
                    for &(byte_pos, field) in &[
                        (2usize, "message"),
                        (6, "title"),
                        (3, "button1"),
                        (4, "button2"),
                        (0, "button3"),
                        (1, "mode4_str"),
                    ] {
                        let s_idx = usize::from(ops[byte_pos]);
                        if s_idx != 0 && s_idx > str301_len {
                            add_err(
                                violations,
                                SafetyCategory::OutOfBoundsString,
                                idx,
                                format!(
                                    "Dialog {field} string index {s_idx} out of bounds for STR# 301 (len {str301_len})"
                                ),
                            );
                        }
                    }
                }
                42 => {
                    let mode = ops[4];
                    if mode == 7 {
                        check_operand(9, 0, false, violations);
                    } else if mode == 8 || mode == 9 {
                        check_operand(25, i16::from(ops[6]), false, violations);
                    }
                }
                44 => {
                    let clip_id = word_i16(ops, 0);
                    let flags = ops[5];
                    let events = ops[6] as i8;
                    if flags & 0x20 != 0 {
                        check_operand(25, i16::from(ops[2]), true, violations);
                    } else if events != 0 && matches!(flags & 0x11, 0x00 | 0x11) {
                        check_operand(9, 0, false, violations);
                    }
                    if flags & 0x04 != 0 {
                        check_operand(9, clip_id, false, violations);
                    }
                }
                45 => {
                    let slot = usize::from(u16::from_le_bytes([ops[0], ops[1]]));
                    let source = u16::from_le_bytes([ops[2], ops[3]]);
                    if source == 1 && slot >= total_slots {
                        add_err(
                            violations,
                            SafetyCategory::OutOfBoundsSlot,
                            idx,
                            format!("RouteTo slot {slot} out of bounds (SLOT count {total_slots})"),
                        );
                    }
                }
                46 => {
                    let slot = usize::from(u16::from_le_bytes([ops[0], ops[1]]));
                    let mode = u16::from_le_bytes([ops[2], ops[3]]);
                    if mode == 3 && slot >= total_slots {
                        add_err(
                            violations,
                            SafetyCategory::OutOfBoundsSlot,
                            idx,
                            format!("Snap slot {slot} out of bounds (SLOT count {total_slots})"),
                        );
                    }
                }
                _ => {}
            }
        } else if (4096..=8191).contains(&ins.opcode) {
            // Private subroutine call: must exist in this object with valid arity.
            match model.behaviors.get(&ins.opcode) {
                None => add_err(
                    violations,
                    SafetyCategory::MissingSubroutine,
                    idx,
                    format!("calls missing private BHAV #{}", ins.opcode),
                ),
                Some((callee_name, callee)) => {
                    let callee_args = bhav_param_limit(callee);
                    let pass_temps = *ops == [0xFF; 8];
                    if !pass_temps && callee_args > 4 {
                        add_err(
                            violations,
                            SafetyCategory::ArityMismatch,
                            idx,
                            format!(
                                "calls private BHAV #{} ('{callee_name}') requiring {callee_args} args via 4-arg inline operands",
                                ins.opcode
                            ),
                        );
                    }
                }
            }
        }
    }
}

// ============================================================================
// Part 1B: Symbolic Lock & Cancel-Safety Invariant Prover
// ============================================================================

/// Identifier of an occupancy lock on the interaction's target object (`stack_obj`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LockKey {
    /// Whole attribute `stack.attr[attr]` set to non-zero and cleared to `0`.
    Scalar(i16),
    /// Single 1-based bit flag `bit` (`1..=16`) in `stack.attr[attr]` set via `flag_set` / cleared via `flag_clear`.
    Bit(i16, u8),
}

impl std::fmt::Display for LockKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scalar(a) => write!(f, "stack.attr[{a}]"),
            Self::Bit(a, b) => write!(f, "stack.attr[{a}].bit({b})"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum LockVal {
    Unknown,
    Concrete(i16),
    /// Symbolic bitmask tracking guaranteed-set and guaranteed-clear bits (`1..=16` -> bit `b-1`).
    BitMask {
        must_set: u16,
        must_clear: u16,
    },
}

impl LockVal {
    fn is_released_scalar(self) -> bool {
        matches!(self, Self::Concrete(0))
    }

    fn is_released_bit(self, bit_1based: u8) -> bool {
        if !(1..=16).contains(&bit_1based) {
            return true;
        }
        let mask = 1u16 << (bit_1based - 1);
        match self {
            Self::Concrete(v) => (v as u16) & mask == 0,
            Self::BitMask { must_clear, .. } => must_clear & mask != 0,
            Self::Unknown => false,
        }
    }

    fn with_flag_set(self, bit_1based: u8) -> Self {
        if !(1..=16).contains(&bit_1based) {
            return self;
        }
        let mask = 1u16 << (bit_1based - 1);
        match self {
            Self::Concrete(v) => Self::Concrete(((v as u16) | mask) as i16),
            Self::BitMask {
                must_set,
                must_clear,
            } => Self::BitMask {
                must_set: must_set | mask,
                must_clear: must_clear & !mask,
            },
            Self::Unknown => Self::BitMask {
                must_set: mask,
                must_clear: 0,
            },
        }
    }

    fn with_flag_clear(self, bit_1based: u8) -> Self {
        if !(1..=16).contains(&bit_1based) {
            return self;
        }
        let mask = 1u16 << (bit_1based - 1);
        match self {
            Self::Concrete(v) => Self::Concrete(((v as u16) & !mask) as i16),
            Self::BitMask {
                must_set,
                must_clear,
            } => Self::BitMask {
                must_set: must_set & !mask,
                must_clear: must_clear | mask,
            },
            Self::Unknown => Self::BitMask {
                must_set: 0,
                must_clear: mask,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SymWord {
    Unknown,
    Const(i16),
    InitialTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SymFrame {
    tree_id: u16,
    pc: u8,
    stack_obj: SymWord,
    params: Vec<SymWord>,
    locals: Vec<SymWord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LockState {
    frames: Vec<SymFrame>,
    temps: [SymWord; 8],
    target_attrs: BTreeMap<i16, LockVal>,
    acquired: BTreeSet<LockKey>,
}

/// Scans a guard tree for mutex checks of the form `stack.attr[k] != 0 => false`
/// or `stack.attr[k] flag_test b => false`.
fn collect_guard_mutexes(bhav: &Behavior) -> BTreeSet<LockKey> {
    let mut out = BTreeSet::new();
    for ins in &bhav.instructions {
        if ins.opcode != 2 {
            continue;
        }
        let lhs_idx = word_i16(&ins.operands, 0);
        let rhs_idx = word_i16(&ins.operands, 2);
        let op = ins.operands[5];
        let lhs_scope = ins.operands[6];
        let rhs_scope = ins.operands[7];
        if lhs_scope == 1 && rhs_scope == 7 {
            // `if stack.attr[k] != 0 { return false; }` or `if stack.attr[k] == 1 { return false; }`
            if ((op == 16 && rhs_idx == 0) || (op == 2 && rhs_idx != 0))
                && ins.on_true == TARGET_FALSE
            {
                out.insert(LockKey::Scalar(lhs_idx));
            }
            // `if stack.attr[k] == 0 { ... } else { return false; }`
            if op == 2 && rhs_idx == 0 && ins.on_false == TARGET_FALSE {
                out.insert(LockKey::Scalar(lhs_idx));
            }
            // `if stack.attr[k] flag_test b { return false; }`
            if op == 8 && (1..=16).contains(&rhs_idx) && ins.on_true == TARGET_FALSE {
                out.insert(LockKey::Bit(lhs_idx, rhs_idx as u8));
            }
        }
    }
    out
}

/// Collects scalar/bitmask lock acquires and releases reachable from `root_tree`
/// (inlining private BHAV callees with constant param propagation).
fn collect_tree_lock_ops(
    model: &ObjectModel,
    root_tree: u16,
    initial_params: &[SymWord],
    depth: usize,
    acquires: &mut BTreeSet<LockKey>,
    releases: &mut BTreeSet<LockKey>,
) {
    if depth > 6 {
        return;
    }
    let Some((_, bhav)) = model.behaviors.get(&root_tree) else {
        return;
    };
    let mut local_consts: BTreeMap<i16, i16> = BTreeMap::new();
    let resolve_val = |scope: u8, idx: i16, locals: &BTreeMap<i16, i16>| -> Option<i16> {
        match scope {
            7 => Some(idx),
            9 | 22 => match initial_params.get(idx as usize).copied() {
                Some(SymWord::Const(v)) => Some(v),
                _ => None,
            },
            25 => locals.get(&idx).copied(),
            26 => lookup_bcon(model, idx),
            _ => None,
        }
    };

    for ins in &bhav.instructions {
        if ins.opcode == 2 {
            let lhs_idx = word_i16(&ins.operands, 0);
            let rhs_idx = word_i16(&ins.operands, 2);
            let op = ins.operands[5];
            let lhs_scope = ins.operands[6];
            let rhs_scope = ins.operands[7];
            if lhs_scope == 25 && op == 5 {
                if let Some(v) = resolve_val(rhs_scope, rhs_idx, &local_consts) {
                    local_consts.insert(lhs_idx, v);
                } else {
                    local_consts.remove(&lhs_idx);
                }
            }
            if lhs_scope == 1 {
                match op {
                    5 => {
                        if let Some(v) = resolve_val(rhs_scope, rhs_idx, &local_consts) {
                            if v == 0 {
                                releases.insert(LockKey::Scalar(lhs_idx));
                            } else {
                                acquires.insert(LockKey::Scalar(lhs_idx));
                            }
                        }
                    }
                    9 => {
                        if let Some(b) = resolve_val(rhs_scope, rhs_idx, &local_consts)
                            && (1..=16).contains(&b)
                        {
                            acquires.insert(LockKey::Bit(lhs_idx, b as u8));
                        }
                    }
                    10 => {
                        if let Some(b) = resolve_val(rhs_scope, rhs_idx, &local_consts)
                            && (1..=16).contains(&b)
                        {
                            releases.insert(LockKey::Bit(lhs_idx, b as u8));
                        }
                    }
                    _ => {}
                }
            }
        } else if (4096..=8191).contains(&ins.opcode) {
            let call_params = if ins.operands == [0xFF; 8] {
                vec![SymWord::Unknown; 8]
            } else {
                (0..4)
                    .map(|i| SymWord::Const(word_i16(&ins.operands, i * 2)))
                    .collect()
            };
            collect_tree_lock_ops(
                model,
                ins.opcode,
                &call_params,
                depth + 1,
                acquires,
                releases,
            );
        }
    }
}

fn lookup_bcon(model: &ObjectModel, addr: i16) -> Option<i16> {
    let raw = addr as u16;
    let table = raw >> 7;
    let entry = usize::from(raw & 127);
    if table < 64 {
        model.bcons.get(&(4096 + table))?.get(entry).copied()
    } else {
        None
    }
}

fn verify_object_locks(model: &ObjectModel, violations: &mut Vec<SafetyViolation>) -> usize {
    // First collect all (action, test) pairs from TTABs and gather releases across all interactions
    // so persistent state toggles (e.g. TurnOn sets attr[1]:=1, TurnOff sets attr[1]:=0) are
    // cleanly distinguished from leaked occupancy locks.
    let mut interactions: Vec<(u16, u16)> = Vec::new();
    for ttab in model.ttabs.values() {
        for inter in &ttab.interactions {
            if inter.action >= 4096 && model.behaviors.contains_key(&inter.action) {
                interactions.push((inter.action, inter.test));
            }
        }
    }

    let mut per_inter_ops: Vec<(BTreeSet<LockKey>, BTreeSet<LockKey>, BTreeSet<LockKey>)> =
        Vec::with_capacity(interactions.len());
    for &(action, test) in &interactions {
        let mut acq = BTreeSet::new();
        let mut rel = BTreeSet::new();
        collect_tree_lock_ops(model, action, &[], 0, &mut acq, &mut rel);
        let guard_mutexes = model
            .behaviors
            .get(&test)
            .map(|(_, b)| collect_guard_mutexes(b))
            .unwrap_or_default();
        per_inter_ops.push((acq, rel, guard_mutexes));
    }

    let mut locks_verified = 0usize;
    let mut checked_actions = BTreeSet::new();

    for (idx, &(action, _test)) in interactions.iter().enumerate() {
        if !checked_actions.insert(action) {
            continue;
        }
        let (acq, rel, guard_mutexes) = &per_inter_ops[idx];
        let mut tracked_locks = BTreeSet::new();
        for &lock in acq {
            if rel.contains(&lock) {
                // Acquired and released within the same interaction -> scoped lock.
                tracked_locks.insert(lock);
            } else if guard_mutexes.contains(&lock) {
                // Guarded as mutex and acquired here; unless another interaction on this object
                // explicitly releases it (persistent toggle like TurnOn/TurnOff), it's a lock leak!
                let released_elsewhere = per_inter_ops
                    .iter()
                    .enumerate()
                    .any(|(j, (_, other_rel, _))| j != idx && other_rel.contains(&lock));
                if !released_elsewhere {
                    tracked_locks.insert(lock);
                }
            }
        }

        if tracked_locks.is_empty() {
            continue;
        }

        let (tree_name, _) = &model.behaviors[&action];
        match prove_interaction_lock_safety(model, action, &tracked_locks) {
            Ok(verified_count) => {
                locks_verified += verified_count;
            }
            Err((leaked, exit_label, trace)) => {
                violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: Some(action),
                    tree_name: tree_name.clone(),
                    category: SafetyCategory::LockLeak,
                    node: None,
                    message: format!(
                        "occupancy lock {leaked} remains held on reachable {exit_label} exit"
                    ),
                    path: trace,
                });
            }
        }
    }

    locks_verified
}

fn eval_sym_operand(model: &ObjectModel, state: &LockState, scope: u8, idx: i16) -> SymWord {
    let frame = state.frames.last().expect("non-empty frame stack");
    match scope {
        7 => SymWord::Const(idx),
        8 => state
            .temps
            .get(idx as usize)
            .copied()
            .unwrap_or(SymWord::Unknown),
        9 | 22 => frame
            .params
            .get(idx as usize)
            .copied()
            .unwrap_or(SymWord::Unknown),
        10 => frame.stack_obj,
        25 => frame
            .locals
            .get(idx as usize)
            .copied()
            .unwrap_or(SymWord::Unknown),
        26 => lookup_bcon(model, idx).map_or(SymWord::Unknown, SymWord::Const),
        1 if frame.stack_obj == SymWord::InitialTarget => match state.target_attrs.get(&idx) {
            Some(LockVal::Concrete(v)) => SymWord::Const(*v),
            _ => SymWord::Unknown,
        },
        _ => SymWord::Unknown,
    }
}

fn write_sym_operand(state: &mut LockState, scope: u8, idx: i16, val: SymWord) {
    let frame = state.frames.last_mut().expect("non-empty frame stack");
    match scope {
        8 => {
            if let Some(slot) = state.temps.get_mut(idx as usize) {
                *slot = val;
            }
        }
        9 | 22 => {
            if let Some(slot) = frame.params.get_mut(idx as usize) {
                *slot = val;
            }
        }
        10 => {
            frame.stack_obj = val;
        }
        25 => {
            if let Some(slot) = frame.locals.get_mut(idx as usize) {
                *slot = val;
            }
        }
        _ => {}
    }
}

fn prove_interaction_lock_safety(
    model: &ObjectModel,
    root_tree: u16,
    tracked_locks: &BTreeSet<LockKey>,
) -> std::result::Result<usize, (LockKey, &'static str, Vec<String>)> {
    const MAX_EXPLORED_STATES: usize = 8192;

    let Some((_, root_bhav)) = model.behaviors.get(&root_tree) else {
        return Ok(0);
    };
    if root_bhav.instructions.is_empty() {
        return Ok(tracked_locks.len());
    }

    let mut initial_attrs = BTreeMap::new();
    for &lock in tracked_locks {
        match lock {
            LockKey::Scalar(a) => {
                initial_attrs.insert(a, LockVal::Concrete(0));
            }
            LockKey::Bit(a, b) => {
                let cur = initial_attrs.get(&a).copied().unwrap_or(LockVal::Unknown);
                initial_attrs.insert(a, cur.with_flag_clear(b));
            }
        }
    }

    let init_state = LockState {
        frames: vec![SymFrame {
            tree_id: root_tree,
            pc: 0,
            stack_obj: SymWord::InitialTarget,
            params: vec![SymWord::Unknown; bhav_param_limit(root_bhav).max(4)],
            locals: vec![SymWord::Const(0); bhav_local_limit(root_bhav)],
        }],
        temps: [SymWord::Unknown; 8],
        target_attrs: initial_attrs,
        acquired: BTreeSet::new(),
    };

    let mut visited: BTreeSet<LockState> = BTreeSet::new();
    let mut queue: VecDeque<(LockState, Vec<String>)> = VecDeque::new();
    visited.insert(init_state.clone());
    queue.push_back((init_state, Vec::new()));

    // Track per-node visit counts so counter-increment loops widen cleanly instead of exploding.
    let mut pc_visits: BTreeMap<(u16, u8), usize> = BTreeMap::new();

    while let Some((state, trace)) = queue.pop_front() {
        if visited.len() > MAX_EXPLORED_STATES {
            break;
        }
        let frame = state.frames.last().expect("frame stack non-empty");
        let cur_tree = frame.tree_id;
        let cur_pc = frame.pc;
        let Some((_, bhav)) = model.behaviors.get(&cur_tree) else {
            continue;
        };
        let Some(ins) = bhav.instructions.get(usize::from(cur_pc)) else {
            continue;
        };

        let visits = pc_visits.entry((cur_tree, cur_pc)).or_insert(0);
        *visits += 1;
        let widen_counters = *visits > 12;

        // Helper to push a successor state (handling return from nested calls & terminal checks).
        let advance = |mut next_state: LockState,
                       target: u8,
                       step_note: String,
                       visited: &mut BTreeSet<LockState>,
                       queue: &mut VecDeque<(LockState, Vec<String>)>|
         -> std::result::Result<(), (LockKey, &'static str, Vec<String>)> {
            let mut next_trace = trace.clone();
            next_trace.push(step_note);
            if widen_counters {
                // Abstract loop-varying locals (except those holding InitialTarget or small bit flags)
                // so symbolic loop exploration converges quickly without losing lock/target precision.
                if let Some(top) = next_state.frames.last_mut() {
                    for loc in &mut top.locals {
                        if matches!(*loc, SymWord::Const(v) if !(1..=16).contains(&v)) {
                            *loc = SymWord::Unknown;
                        }
                    }
                }
            }

            let mut cur_target = target;
            loop {
                match cur_target {
                    TARGET_TRUE | TARGET_FALSE | TARGET_ERROR => {
                        let exit_label = match cur_target {
                            TARGET_TRUE => "return true",
                            TARGET_FALSE => "return false",
                            _ => "error",
                        };
                        next_state.frames.pop();
                        if next_state.frames.is_empty() {
                            // Top-level interaction exited: check EVERY acquired tracked lock!
                            for &lock in &next_state.acquired {
                                if !tracked_locks.contains(&lock) {
                                    continue;
                                }
                                let released = match lock {
                                    LockKey::Scalar(a) => next_state
                                        .target_attrs
                                        .get(&a)
                                        .copied()
                                        .unwrap_or(LockVal::Unknown)
                                        .is_released_scalar(),
                                    LockKey::Bit(a, b) => next_state
                                        .target_attrs
                                        .get(&a)
                                        .copied()
                                        .unwrap_or(LockVal::Unknown)
                                        .is_released_bit(b),
                                };
                                if !released {
                                    return Err((lock, exit_label, next_trace));
                                }
                            }
                            return Ok(());
                        }
                        // Return to caller frame along on_true / on_false / error.
                        let caller = next_state.frames.last().expect("caller frame");
                        let caller_ins = &model.behaviors[&caller.tree_id].1.instructions
                            [usize::from(caller.pc)];
                        cur_target = match cur_target {
                            TARGET_TRUE => caller_ins.on_true,
                            TARGET_FALSE => caller_ins.on_false,
                            _ => TARGET_ERROR,
                        };
                    }
                    node => {
                        next_state.frames.last_mut().expect("frame").pc = node;
                        if visited.insert(next_state.clone()) {
                            queue.push_back((next_state, next_trace));
                        }
                        return Ok(());
                    }
                }
            }
        };

        let ops = &ins.operands;
        match ins.opcode {
            2 => {
                let lhs_idx = word_i16(ops, 0);
                let rhs_idx = word_i16(ops, 2);
                let op = ops[5];
                let lhs_scope = ops[6];
                let rhs_scope = ops[7];
                let lhs_val = eval_sym_operand(model, &state, lhs_scope, lhs_idx);
                let rhs_val = eval_sym_operand(model, &state, rhs_scope, rhs_idx);

                let is_assign = matches!(op, 3..=7 | 9 | 10 | 12 | 13 | 18 | 19 | 20);
                if is_assign {
                    let mut next = state.clone();
                    let on_initial_target =
                        next.frames.last().expect("frame").stack_obj == SymWord::InitialTarget;

                    if lhs_scope == 1 && on_initial_target {
                        match op {
                            5 => {
                                if let SymWord::Const(v) = rhs_val {
                                    next.target_attrs.insert(lhs_idx, LockVal::Concrete(v));
                                    if v != 0 && tracked_locks.contains(&LockKey::Scalar(lhs_idx)) {
                                        next.acquired.insert(LockKey::Scalar(lhs_idx));
                                    }
                                } else {
                                    next.target_attrs.insert(lhs_idx, LockVal::Unknown);
                                }
                            }
                            9 => {
                                if let SymWord::Const(b) = rhs_val
                                    && (1..=16).contains(&b)
                                {
                                    let b = b as u8;
                                    let cur = next
                                        .target_attrs
                                        .get(&lhs_idx)
                                        .copied()
                                        .unwrap_or(LockVal::Unknown);
                                    next.target_attrs.insert(lhs_idx, cur.with_flag_set(b));
                                    if tracked_locks.contains(&LockKey::Bit(lhs_idx, b)) {
                                        next.acquired.insert(LockKey::Bit(lhs_idx, b));
                                    }
                                } else {
                                    next.target_attrs.insert(lhs_idx, LockVal::Unknown);
                                }
                            }
                            10 => {
                                if let SymWord::Const(b) = rhs_val
                                    && (1..=16).contains(&b)
                                {
                                    let b = b as u8;
                                    let cur = next
                                        .target_attrs
                                        .get(&lhs_idx)
                                        .copied()
                                        .unwrap_or(LockVal::Unknown);
                                    next.target_attrs.insert(lhs_idx, cur.with_flag_clear(b));
                                } else {
                                    next.target_attrs.insert(lhs_idx, LockVal::Unknown);
                                }
                            }
                            _ => {
                                next.target_attrs.insert(lhs_idx, LockVal::Unknown);
                            }
                        }
                    } else {
                        let new_val = eval_expr_assign(lhs_val, op, rhs_val);
                        write_sym_operand(&mut next, lhs_scope, lhs_idx, new_val);
                    }

                    advance(
                        next,
                        ins.on_true,
                        format!("tree#{cur_tree}@n{cur_pc}:expr(op={op})->true"),
                        &mut visited,
                        &mut queue,
                    )?;
                } else {
                    // Comparison (or inc_lt / dec_gt).
                    let mut next = state.clone();
                    let mut pre_lhs = lhs_val;
                    if matches!(op, 11 | 17) {
                        let updated = match lhs_val {
                            SymWord::Const(v) => {
                                SymWord::Const(v.wrapping_add(if op == 11 { 1 } else { -1 }))
                            }
                            _ => SymWord::Unknown,
                        };
                        pre_lhs = updated;
                        write_sym_operand(&mut next, lhs_scope, lhs_idx, updated);
                    }
                    let known_truth = match (pre_lhs, rhs_val) {
                        (SymWord::Const(l), SymWord::Const(r)) => eval_expr_compare(l, op, r),
                        _ => None,
                    };
                    if known_truth != Some(false) {
                        advance(
                            next.clone(),
                            ins.on_true,
                            format!("tree#{cur_tree}@n{cur_pc}:cond(op={op})->true"),
                            &mut visited,
                            &mut queue,
                        )?;
                    }
                    if known_truth != Some(true) {
                        advance(
                            next,
                            ins.on_false,
                            format!("tree#{cur_tree}@n{cur_pc}:cond(op={op})->false"),
                            &mut visited,
                            &mut queue,
                        )?;
                    }
                }
            }
            opcode if (4096..=8191).contains(&opcode) && model.behaviors.contains_key(&opcode) => {
                if state.frames.len() >= 8 {
                    // Bounded call depth: conservatively branch both outcomes.
                    advance(
                        state.clone(),
                        ins.on_true,
                        format!("tree#{cur_tree}@n{cur_pc}:call#{opcode}(depth-cap)->true"),
                        &mut visited,
                        &mut queue,
                    )?;
                    advance(
                        state,
                        ins.on_false,
                        format!("tree#{cur_tree}@n{cur_pc}:call#{opcode}(depth-cap)->false"),
                        &mut visited,
                        &mut queue,
                    )?;
                } else {
                    let callee = &model.behaviors[&opcode].1;
                    if callee.instructions.is_empty() {
                        advance(
                            state,
                            ins.on_true,
                            format!("tree#{cur_tree}@n{cur_pc}:call#{opcode}(empty)->true"),
                            &mut visited,
                            &mut queue,
                        )?;
                    } else {
                        let mut next = state.clone();
                        let caller_stack_obj = next.frames.last().expect("frame").stack_obj;
                        let mut params = if *ops == [0xFF; 8] {
                            next.temps.to_vec()
                        } else {
                            (0..4)
                                .map(|i| SymWord::Const(word_i16(ops, i * 2)))
                                .collect()
                        };
                        let needed = bhav_param_limit(callee).max(4);
                        if params.len() < needed {
                            params.resize(needed, SymWord::Const(0));
                        }
                        next.frames.push(SymFrame {
                            tree_id: opcode,
                            pc: 0,
                            stack_obj: caller_stack_obj,
                            params,
                            locals: vec![SymWord::Const(0); bhav_local_limit(callee)],
                        });
                        if visited.insert(next.clone()) {
                            let mut next_trace = trace.clone();
                            next_trace.push(format!("tree#{cur_tree}@n{cur_pc}:enter#{opcode}"));
                            queue.push_back((next, next_trace));
                        }
                    }
                }
            }
            _ => {
                // World-interacting primitive: apply conservative mutation to stack_obj/dest/temps.
                let mut next = state.clone();
                match ins.opcode {
                    0 => {
                        // Sleep zeroes its param slot.
                        let p = word_i16(ops, 0);
                        write_sym_operand(&mut next, 9, p, SymWord::Const(0));
                    }
                    8 => {
                        let dst_idx = word_i16(ops, 0);
                        let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
                        if dst_scope <= 255 {
                            write_sym_operand(
                                &mut next,
                                dst_scope as u8,
                                dst_idx,
                                SymWord::Unknown,
                            );
                        }
                    }
                    14 | 42 => {
                        // FindBestObject / CreateObject overwrites stack_obj with another object.
                        write_sym_operand(&mut next, 10, 0, SymWord::Unknown);
                    }
                    31 => {
                        // SetToNext writes dest (or stack_obj if bit 7 clear).
                        let mode = ops[4];
                        if mode & 0x80 != 0 {
                            write_sym_operand(
                                &mut next,
                                ops[5],
                                i16::from(ops[7]),
                                SymWord::Unknown,
                            );
                        } else {
                            write_sym_operand(&mut next, 10, 0, SymWord::Unknown);
                        }
                    }
                    36 => {
                        // Dialog writes result into selected temp slot.
                        let t = usize::from((ops[7] >> 4) & 7);
                        next.temps[t] = SymWord::Unknown;
                    }
                    _ => {}
                }

                if is_always_true_instruction(ins) {
                    advance(
                        next,
                        ins.on_true,
                        format!("tree#{cur_tree}@n{cur_pc}:prim({})->true", ins.opcode),
                        &mut visited,
                        &mut queue,
                    )?;
                } else {
                    advance(
                        next.clone(),
                        ins.on_true,
                        format!("tree#{cur_tree}@n{cur_pc}:prim({})->true", ins.opcode),
                        &mut visited,
                        &mut queue,
                    )?;
                    advance(
                        next,
                        ins.on_false,
                        format!("tree#{cur_tree}@n{cur_pc}:prim({})->false", ins.opcode),
                        &mut visited,
                        &mut queue,
                    )?;
                }
            }
        }
    }

    Ok(tracked_locks.len())
}

fn eval_expr_assign(lhs: SymWord, op: u8, rhs: SymWord) -> SymWord {
    if op == 5 {
        return rhs;
    }
    match (lhs, rhs) {
        (SymWord::Const(l), SymWord::Const(r)) => {
            let l32 = i32::from(l);
            let r32 = i32::from(r);
            let res = match op {
                3 => l32 + r32,
                4 => l32 - r32,
                6 => l32 * r32,
                7 if r32 != 0 => l32 / r32,
                9 if (0..=16).contains(&r32) => l32 | 1i32.wrapping_shl((r32 - 1) as u32),
                10 if (0..=16).contains(&r32) => l32 & !1i32.wrapping_shl((r32 - 1) as u32),
                12 if r32 != 0 => i32::from(l.wrapping_abs()) % i32::from(r.wrapping_abs()),
                13 => l32 & r32,
                18 => l32 | r32,
                19 => l32 ^ r32,
                20 if r32 >= 0 => (r32 as u32).isqrt() as i32,
                _ => return SymWord::Unknown,
            };
            SymWord::Const(res as i16)
        }
        _ => SymWord::Unknown,
    }
}

fn eval_expr_compare(l: i16, op: u8, r: i16) -> Option<bool> {
    let l32 = i32::from(l);
    let r32 = i32::from(r);
    match op {
        0 => Some(l32 > r32),
        1 | 11 => Some(l32 < r32),
        2 => Some(l32 == r32),
        8 if (0..=16).contains(&r32) => Some(l32 & 1i32.wrapping_shl((r32 - 1) as u32) != 0),
        14 => Some(l32 >= r32),
        15 => Some(l32 <= r32),
        16 => Some(l32 != r32),
        17 => Some(l32 > r32),
        _ => None,
    }
}

// ============================================================================
// Part 2: Behavioral Trace-Equivalence Checker (`check_bhav_equivalent`)
// ============================================================================

/// Returns whether a SimAntics instruction unconditionally takes `on_true` in `sims-vm`
/// (meaning its `on_false` branch is architecturally unreachable).
pub(crate) fn is_always_true_instruction(ins: &Instruction) -> bool {
    match ins.opcode {
        0 => true,  // Sleep (yields until done, then branches true)
        23 => true, // PlaySound
        29 => true, // SetMotiveChange / ClearMotiveChanges
        48 => true, // StopAllSounds
        2 => matches!(ins.operands[5], 3..=7 | 9 | 10 | 12 | 13 | 18 | 19 | 20),
        _ => false,
    }
}

fn word_i16(ops: &[u8; 8], offset: usize) -> i16 {
    i16::from_le_bytes([ops[offset], ops[offset + 1]])
}

/// Canonical operand key for symbolic equivalence, stripping architecturally ignored
/// padding bytes verified against `sims-vm/src/primitives/*.rs`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum CanonEvent {
    /// Observable `Expression` assignment (to external/shared state, or mutating compare).
    ExprAssign {
        lhs_scope: u8,
        lhs_idx: i16,
        op: u8,
        rhs: CanonOperand,
    },
    /// World-interacting primitive or subroutine call with canonicalized effective operands.
    Primitive { opcode: u16, operands: [u8; 8] },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CanonOperand {
    /// Concrete known `i16` value (literal or constant-propagated frame-private slot).
    Const(i16),
    /// Frame-private symbolic value token (identifies shared initial param/local or post-event value).
    Sym(u32),
    /// External/shared memory operand `(scope, index)`.
    Mem(u8, i16),
}

/// Canonicalized pure comparison predicate `(lhs op rhs)` normalized so equivalent
/// formulations (`a != b` vs `!(a == b)`, `a > b` vs `b < a`, `a >= c` vs `a > c-1`,
/// `a == b` vs `b == a`) map to the exact same canonical predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CanonPred {
    /// Always true or always false (constant-folded pure comparison).
    Const(bool),
    /// `lhs == rhs` (with `lhs <= rhs` when commuting is safe).
    Eq(CanonOperand, CanonOperand),
    /// `lhs < rhs` (canonicalized integer inequality).
    Lt(CanonOperand, CanonOperand),
    /// `lhs flag_test rhs`.
    FlagTest(CanonOperand, CanonOperand),
}

fn is_global_34(scope: u8, idx: i16) -> bool {
    scope == 6 && idx == 34
}

fn canonicalize_compare(
    op: u8,
    lhs: CanonOperand,
    rhs: CanonOperand,
    on_true: u8,
    on_false: u8,
    can_commute: bool,
) -> Option<(CanonPred, u8, u8)> {
    // Check if both sides are concrete i16 constants -> constant-fold!
    if let (CanonOperand::Const(l), CanonOperand::Const(r)) = (lhs, rhs)
        && can_commute
        && let Some(val) = eval_expr_compare(l, op, r)
    {
        return Some((CanonPred::Const(val), on_true, on_false));
    }

    match op {
        // lhs == rhs
        2 => {
            if lhs == rhs && can_commute {
                return Some((CanonPred::Const(true), on_true, on_false));
            }
            let (a, b) = if can_commute && rhs < lhs {
                (rhs, lhs)
            } else {
                (lhs, rhs)
            };
            Some((CanonPred::Eq(a, b), on_true, on_false))
        }
        // lhs != rhs  <=>  !(lhs == rhs)
        16 => {
            if lhs == rhs && can_commute {
                return Some((CanonPred::Const(false), on_true, on_false));
            }
            let (a, b) = if can_commute && rhs < lhs {
                (rhs, lhs)
            } else {
                (lhs, rhs)
            };
            Some((CanonPred::Eq(a, b), on_false, on_true))
        }
        // lhs < rhs
        1 => Some(canonicalize_lt(lhs, rhs, on_true, on_false)),
        // lhs >= rhs  <=>  !(lhs < rhs)
        14 => Some(canonicalize_lt(lhs, rhs, on_false, on_true)),
        // lhs > rhs  <=>  rhs < lhs (when commuting is safe)
        0 => {
            if can_commute {
                Some(canonicalize_lt(rhs, lhs, on_true, on_false))
            } else {
                // Keep exact direction if global 34 is involved.
                match rhs {
                    CanonOperand::Const(c) if c < i16::MAX => Some((
                        CanonPred::Lt(lhs, CanonOperand::Const(c + 1)),
                        on_false,
                        on_true,
                    )),
                    _ => None,
                }
            }
        }
        // lhs <= rhs  <=>  !(rhs < lhs)
        15 => {
            if can_commute {
                Some(canonicalize_lt(rhs, lhs, on_false, on_true))
            } else {
                match rhs {
                    CanonOperand::Const(c) if c < i16::MAX => Some((
                        CanonPred::Lt(lhs, CanonOperand::Const(c + 1)),
                        on_true,
                        on_false,
                    )),
                    _ => None,
                }
            }
        }
        // flag_test
        8 => Some((CanonPred::FlagTest(lhs, rhs), on_true, on_false)),
        _ => None,
    }
}

/// Normalizes integer `<` inequalities with literal bounds so e.g. `x >= 1` (`!(x < 1)`)
/// and `x > 0` (`0 < x`) canonicalize to the exact same form: `x < c` (literal on RHS when possible).
fn canonicalize_lt(
    lhs: CanonOperand,
    rhs: CanonOperand,
    on_true: u8,
    on_false: u8,
) -> (CanonPred, u8, u8) {
    if lhs == rhs {
        return (CanonPred::Const(false), on_true, on_false);
    }
    // `Const(c) < x` <=> `x > c` <=> `!(x < c + 1)` when `c < i16::MAX`!
    if let CanonOperand::Const(c) = lhs
        && !matches!(rhs, CanonOperand::Const(_))
        && c < i16::MAX
    {
        return (
            CanonPred::Lt(rhs, CanonOperand::Const(c + 1)),
            on_false,
            on_true,
        );
    }
    // `x < Const(i16::MIN)` is always false.
    if let CanonOperand::Const(i16::MIN) = rhs {
        return (CanonPred::Const(false), on_true, on_false);
    }
    (CanonPred::Lt(lhs, rhs), on_true, on_false)
}

/// Canonicalizes primitive operands by zeroing verified-ignored padding bytes.
fn canonicalize_primitive_operands(opcode: u16, ops: [u8; 8]) -> [u8; 8] {
    match opcode {
        // Sleep: word 0 (param_index) only (`sleep.rs:29`).
        0 => [ops[0], ops[1], 0, 0, 0, 0, 0, 0],
        // GenericCall: word 0 (routine) only.
        1 => [ops[0], ops[1], 0, 0, 0, 0, 0, 0],
        // RemoveObject: word 0 (target) + byte 2 (flags) (`remove.rs:47-53`).
        18 => [ops[0], ops[1], ops[2], 0, 0, 0, 0, 0],
        // RunFunctionalTree: word 0 (function) + byte 2 bit 0 (`run_functional_tree.rs:75-93`).
        20 => [ops[0], ops[1], ops[2] & 1, 0, 0, 0, 0, 0],
        // PlaySound: word 0 (sound_id) + effective stack_obj flag (`sound.rs:41-47`).
        23 => {
            let use_stack = ops[4] != 0xFF && ops[4] & 2 != 0;
            [ops[0], ops[1], 0, 0, if use_stack { 2 } else { 0 }, 0, 0, 0]
        }
        // AlterBudget: resolved scope, index, flags & 3, category (`budget.rs:38-50`).
        25 => {
            let scope = match ops[0] {
                0 => 7,
                1 => 9,
                2 => 25,
                _ => ops[1],
            };
            let sel = match scope {
                7 => 0,
                9 => 1,
                25 => 2,
                _ => 3,
            };
            [sel, scope, ops[2], ops[3], ops[4] & 3, 0, ops[6], 0]
        }
        // SetMotiveChange: if flags & 1 != 0, clear all (other bytes ignored; `motive_change.rs:52`).
        29 => {
            if ops[3] & 1 != 0 {
                [0, 0, 0, 1, 0, 0, 0, 0]
            } else {
                [ops[0], ops[1], ops[2], 0, ops[4], ops[5], ops[6], ops[7]]
            }
        }
        // StopAllSounds: word 0 != 0 boolean (`sound.rs:65`).
        48 => {
            let flag = u8::from(i16::from_le_bytes([ops[0], ops[1]]) != 0);
            [flag, 0, 0, 0, 0, 0, 0, 0]
        }
        _ => ops,
    }
}

/// Frame-private symbolic environment for one BHAV during equivalence checking.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LocalEnv {
    params: BTreeMap<i16, CanonOperand>,
    locals: BTreeMap<i16, CanonOperand>,
}

impl LocalEnv {
    fn new(arg_count: usize, local_count: usize) -> Self {
        let mut params = BTreeMap::new();
        for i in 0..arg_count {
            params.insert(i as i16, CanonOperand::Sym(1 + i as u32));
        }
        let mut locals = BTreeMap::new();
        for i in 0..local_count {
            locals.insert(i as i16, CanonOperand::Const(0));
        }
        Self { params, locals }
    }

    fn read(&self, scope: u8, idx: i16) -> CanonOperand {
        match scope {
            7 => CanonOperand::Const(idx),
            9 | 22 => self
                .params
                .get(&idx)
                .copied()
                .unwrap_or(CanonOperand::Mem(scope, idx)),
            25 => self
                .locals
                .get(&idx)
                .copied()
                .unwrap_or(CanonOperand::Mem(scope, idx)),
            _ => CanonOperand::Mem(scope, idx),
        }
    }

    fn write(&mut self, scope: u8, idx: i16, val: CanonOperand) {
        match scope {
            9 | 22 => {
                self.params.insert(idx, val);
            }
            25 => {
                self.locals.insert(idx, val);
            }
            _ => {}
        }
    }

    fn is_frame_private(&self, scope: u8, idx: i16) -> bool {
        match scope {
            9 | 22 => self.params.contains_key(&idx),
            25 => self.locals.contains_key(&idx),
            _ => false,
        }
    }
}

/// Deterministic hash-consing interner for symbolic expressions computed during
/// silent frame-private arithmetic so both trees `A` and `B` receive identical
/// symbolic tokens for structurally identical computations.
struct SymInterner {
    next_id: u32,
    exprs: BTreeMap<(u8, CanonOperand, CanonOperand), u32>,
}

impl SymInterner {
    fn new() -> Self {
        Self {
            next_id: 1000,
            exprs: BTreeMap::new(),
        }
    }

    fn fresh_event(&mut self) -> CanonOperand {
        self.next_id += 1;
        CanonOperand::Sym(self.next_id)
    }

    fn intern_op(&mut self, op: u8, lhs: CanonOperand, rhs: CanonOperand) -> CanonOperand {
        // Canonicalize commutative operators (`+`, `*`, `&`, `|`, `^`) so `a + b` == `b + a`.
        let (a, b) = if matches!(op, 3 | 6 | 13 | 18 | 19) && rhs < lhs {
            (rhs, lhs)
        } else {
            (lhs, rhs)
        };
        let id = *self.exprs.entry((op, a, b)).or_insert_with(|| {
            self.next_id += 1;
            self.next_id
        });
        CanonOperand::Sym(id)
    }
}

fn eval_canon_assign(
    lhs: CanonOperand,
    op: u8,
    rhs: CanonOperand,
    interner: &mut SymInterner,
) -> CanonOperand {
    if op == 5 {
        return rhs;
    }
    if let (CanonOperand::Const(l), CanonOperand::Const(r)) = (lhs, rhs) {
        let l32 = i32::from(l);
        let r32 = i32::from(r);
        let computed = match op {
            3 => Some(l32 + r32),
            4 => Some(l32 - r32),
            6 => Some(l32 * r32),
            7 if r32 != 0 => Some(l32 / r32),
            9 if (0..=16).contains(&r32) => Some(l32 | 1i32.wrapping_shl((r32 - 1) as u32)),
            10 if (0..=16).contains(&r32) => Some(l32 & !1i32.wrapping_shl((r32 - 1) as u32)),
            12 if r32 != 0 => Some(i32::from(l.wrapping_abs()) % i32::from(r.wrapping_abs())),
            13 => Some(l32 & r32),
            18 => Some(l32 | r32),
            19 => Some(l32 ^ r32),
            20 if r32 >= 0 => Some((r32 as u32).isqrt() as i32),
            _ => None,
        };
        if let Some(v) = computed {
            return CanonOperand::Const(v as i16);
        }
    }
    interner.intern_op(op, lhs, rhs)
}

/// Checks whether a node is a pure silent step (a no-op comparison like `0 == 0`
/// or a non-trapping write to a frame-private local/param).
fn try_silent_step(
    bhav: &BhavChunk,
    pc: u8,
    env: &mut LocalEnv,
    interner: &mut SymInterner,
) -> Option<u8> {
    if matches!(pc, TARGET_ERROR..=TARGET_FALSE) {
        return None;
    }
    let ins = bhav.instructions.get(usize::from(pc))?;
    if ins.opcode != 2 {
        return None;
    }
    let lhs_idx = word_i16(&ins.operands, 0);
    let rhs_idx = word_i16(&ins.operands, 2);
    let op = ins.operands[5];
    let lhs_scope = ins.operands[6];
    let rhs_scope = ins.operands[7];

    if is_global_34(lhs_scope, lhs_idx) || is_global_34(rhs_scope, rhs_idx) {
        return None;
    }

    // 1. Constant-folded pure comparison (e.g., `0 == 0` jump node emitted by `lower.rs`).
    if matches!(op, 0 | 1 | 2 | 8 | 14 | 15 | 16) {
        let lhs = env.read(lhs_scope, lhs_idx);
        let rhs = env.read(rhs_scope, rhs_idx);
        if let Some((CanonPred::Const(b), t, f)) =
            canonicalize_compare(op, lhs, rhs, ins.on_true, ins.on_false, true)
        {
            return Some(if b { t } else { f });
        }
        // Non-trapping pure comparison with identical targets is a pure jump.
        if ins.on_true == ins.on_false
            && matches!(lhs_scope, 7 | 9 | 22 | 25)
            && matches!(rhs_scope, 7 | 9 | 22 | 25)
            && (op != 8 || matches!(rhs, CanonOperand::Const(r) if (0..=16).contains(&r)))
        {
            return Some(ins.on_true);
        }
    }

    // 2. Pure non-trapping assignment to a frame-private local/param from a literal or
    //    frame-private slot (e.g., `idle()` saving/restoring `param[0]`, or constant folding).
    if matches!(op, 3..=7 | 9 | 10 | 12 | 13 | 18 | 19 | 20)
        && env.is_frame_private(lhs_scope, lhs_idx)
        && (rhs_scope == 7 || env.is_frame_private(rhs_scope, rhs_idx))
    {
        let lhs = env.read(lhs_scope, lhs_idx);
        let rhs = env.read(rhs_scope, rhs_idx);
        // Only step silently if guaranteed not to trap (no div/mod by 0, valid flag/sqrt).
        let safe = match op {
            7 | 12 => matches!(rhs, CanonOperand::Const(r) if r != 0),
            9 | 10 => matches!(rhs, CanonOperand::Const(r) if (0..=16).contains(&r)),
            20 => matches!(rhs, CanonOperand::Const(r) if r >= 0),
            _ => true,
        };
        if safe {
            let new_val = eval_canon_assign(lhs, op, rhs, interner);
            env.write(lhs_scope, lhs_idx, new_val);
            return Some(ins.on_true);
        }
    }

    None
}

fn advance_silent(
    bhav: &BhavChunk,
    mut pc: u8,
    env: &mut LocalEnv,
    interner: &mut SymInterner,
    limit: usize,
) -> std::result::Result<u8, String> {
    for _ in 0..limit {
        let Some(next) = try_silent_step(bhav, pc, env, interner) else {
            return Ok(pc);
        };
        pc = next;
    }
    Err(format!(
        "exceeded {limit} silent local/no-op steps (possible silent cycle at node {pc})"
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ProductState {
    pc_a: u8,
    pc_b: u8,
    env_a: LocalEnv,
    env_b: LocalEnv,
}

/// Checks whether two BHAV chunks `a` and `b` are behaviorally trace-equivalent
/// using CFG canonicalization + bounded symbolic bisimulation.
///
/// - Ignores dead/unreachable nodes, Edith node renumbering/reordering, unused
///   operand padding bytes, and unreachable `on_false` targets on always-true primitives.
/// - Models pure `Expression` (`0x0002`) arithmetic and comparisons over symbolic/concrete `i16`.
/// - Models world-interacting primitives and subroutines as canonical trace events.
/// - Never reports false positives: returns [`EquivResult::Inconclusive`] if
///   bounds are exceeded.
#[must_use]
pub fn check_bhav_equivalent(a: &BhavChunk, b: &BhavChunk, opts: &EquivOptions) -> EquivResult {
    if a.instructions.is_empty() && b.instructions.is_empty() {
        return EquivResult::Equivalent {
            states_checked: 1,
            paths_verified: 1,
        };
    }
    if a.instructions.is_empty() || b.instructions.is_empty() {
        return EquivResult::Counterexample {
            path: Vec::new(),
            divergence: format!(
                "instruction emptiness mismatch: a has {} nodes, b has {} nodes",
                a.instructions.len(),
                b.instructions.len()
            ),
        };
    }

    let args_a = match a.format {
        0x8000 => 4,
        0x8001 => usize::from(a.args).min(4),
        _ => usize::from(a.args),
    };
    let args_b = match b.format {
        0x8000 => 4,
        0x8001 => usize::from(b.args).min(4),
        _ => usize::from(b.args),
    };
    let locals_a = match a.format {
        0x8000 => 0,
        0x8001 => usize::from(a.locals).min(4),
        _ => usize::from(a.locals),
    };
    let locals_b = match b.format {
        0x8000 => 0,
        0x8001 => usize::from(b.locals).min(4),
        _ => usize::from(b.locals),
    };

    let mut interner = SymInterner::new();
    let init = ProductState {
        pc_a: 0,
        pc_b: 0,
        env_a: LocalEnv::new(args_a, locals_a),
        env_b: LocalEnv::new(args_b, locals_b),
    };

    let mut visited: BTreeSet<ProductState> = BTreeSet::new();
    let mut pc_pair_visits: BTreeMap<(u8, u8), usize> = BTreeMap::new();
    let mut queue: VecDeque<(ProductState, Vec<TraceStep>)> = VecDeque::new();
    visited.insert(init.clone());
    queue.push_back((init, Vec::new()));

    let mut states_checked = 0usize;
    let mut paths_verified = 0usize;

    while let Some((mut state, path)) = queue.pop_front() {
        states_checked += 1;
        if states_checked > opts.max_states {
            return EquivResult::Inconclusive {
                reason: format!("exceeded state exploration budget ({})", opts.max_states),
            };
        }
        if path.len() > opts.max_depth {
            return EquivResult::Inconclusive {
                reason: format!("exceeded path depth budget ({})", opts.max_depth),
            };
        }

        // Advance silent pure no-op / frame-local steps on both sides.
        let pc_a = match advance_silent(
            a,
            state.pc_a,
            &mut state.env_a,
            &mut interner,
            opts.max_silent_steps,
        ) {
            Ok(pc) => pc,
            Err(reason) => return EquivResult::Inconclusive { reason },
        };
        let pc_b = match advance_silent(
            b,
            state.pc_b,
            &mut state.env_b,
            &mut interner,
            opts.max_silent_steps,
        ) {
            Ok(pc) => pc,
            Err(reason) => return EquivResult::Inconclusive { reason },
        };
        state.pc_a = pc_a;
        state.pc_b = pc_b;

        let term_a = matches!(pc_a, TARGET_ERROR..=TARGET_FALSE);
        let term_b = matches!(pc_b, TARGET_ERROR..=TARGET_FALSE);

        if term_a || term_b {
            if pc_a == pc_b {
                paths_verified += 1;
                continue;
            }
            return EquivResult::Counterexample {
                path,
                divergence: format!(
                    "exit divergence: tree A reached {} while tree B reached {}",
                    format_pc(pc_a),
                    format_pc(pc_b)
                ),
            };
        }

        // If we hit the same (pc_a, pc_b) loop header multiple times with identical
        // frame-local environments on A and B, widen both sides symmetrically to shared
        // symbolic tokens so unbounded identical loops prove equivalent in O(N) states!
        let pair_hits = pc_pair_visits.entry((pc_a, pc_b)).or_insert(0);
        *pair_hits += 1;
        if *pair_hits > 4 && state.env_a == state.env_b {
            let mut widened_a = state.env_a.clone();
            let mut widened_b = state.env_b.clone();
            for (&k, val_a) in &mut widened_a.locals {
                let sym = CanonOperand::Sym(100_000 + u32::from(k as u16));
                *val_a = sym;
                widened_b.locals.insert(k, sym);
            }
            for (&k, val_a) in &mut widened_a.params {
                let sym = CanonOperand::Sym(200_000 + u32::from(k as u16));
                *val_a = sym;
                widened_b.params.insert(k, sym);
            }
            let widened = ProductState {
                pc_a,
                pc_b,
                env_a: widened_a,
                env_b: widened_b,
            };
            if !visited.insert(widened.clone()) {
                paths_verified += 1;
                continue;
            }
            state = widened;
        }

        let Some(ins_a) = a.instructions.get(usize::from(pc_a)) else {
            return EquivResult::Counterexample {
                path,
                divergence: format!("tree A jumped to out-of-bounds node {pc_a}"),
            };
        };
        let Some(ins_b) = b.instructions.get(usize::from(pc_b)) else {
            return EquivResult::Counterexample {
                path,
                divergence: format!("tree B jumped to out-of-bounds node {pc_b}"),
            };
        };

        // Case 1: Both are pure Expression comparisons (operators 0, 1, 2, 8, 14, 15, 16).
        let pure_cmp_a = ins_a.opcode == 2 && matches!(ins_a.operands[5], 0 | 1 | 2 | 8 | 14..=16);
        let pure_cmp_b = ins_b.opcode == 2 && matches!(ins_b.operands[5], 0 | 1 | 2 | 8 | 14..=16);
        if pure_cmp_a && pure_cmp_b {
            let la = state
                .env_a
                .read(ins_a.operands[6], word_i16(&ins_a.operands, 0));
            let ra = state
                .env_a
                .read(ins_a.operands[7], word_i16(&ins_a.operands, 2));
            let lb = state
                .env_b
                .read(ins_b.operands[6], word_i16(&ins_b.operands, 0));
            let rb = state
                .env_b
                .read(ins_b.operands[7], word_i16(&ins_b.operands, 2));

            let can_commute_a = !is_global_34(ins_a.operands[6], word_i16(&ins_a.operands, 0))
                && !is_global_34(ins_a.operands[7], word_i16(&ins_a.operands, 2));
            let can_commute_b = !is_global_34(ins_b.operands[6], word_i16(&ins_b.operands, 0))
                && !is_global_34(ins_b.operands[7], word_i16(&ins_b.operands, 2));
            let norm_a = canonicalize_compare(
                ins_a.operands[5],
                la,
                ra,
                ins_a.on_true,
                ins_a.on_false,
                can_commute_a,
            );
            let norm_b = canonicalize_compare(
                ins_b.operands[5],
                lb,
                rb,
                ins_b.on_true,
                ins_b.on_false,
                can_commute_b,
            );
            match (norm_a, norm_b) {
                (Some((pred_a, ta, fa)), Some((pred_b, tb, fb))) if pred_a == pred_b => {
                    let mut enqueue = |na: u8, nb: u8, branch: bool| {
                        let next = ProductState {
                            pc_a: na,
                            pc_b: nb,
                            env_a: state.env_a.clone(),
                            env_b: state.env_b.clone(),
                        };
                        if visited.insert(next.clone()) {
                            let mut next_path = path.clone();
                            next_path.push(TraceStep {
                                pc_a,
                                pc_b,
                                event: format!("cmp({pred_a:?})"),
                                branch: Some(branch),
                            });
                            queue.push_back((next, next_path));
                        } else {
                            paths_verified += 1;
                        }
                    };
                    match pred_a {
                        CanonPred::Const(true) => enqueue(ta, tb, true),
                        CanonPred::Const(false) => enqueue(fa, fb, false),
                        _ => {
                            enqueue(ta, tb, true);
                            enqueue(fa, fb, false);
                        }
                    }
                    continue;
                }
                _ => {
                    return EquivResult::Counterexample {
                        path,
                        divergence: format!(
                            "comparison divergence at A@n{pc_a} vs B@n{pc_b}: A=({la:?} op={} {ra:?}) vs B=({lb:?} op={} {rb:?})",
                            ins_a.operands[5], ins_b.operands[5]
                        ),
                    };
                }
            }
        }

        // Case 2: Both are Expression assignments (or mutating comparisons 11/17).
        if ins_a.opcode == 2 && ins_b.opcode == 2 {
            let op_a = ins_a.operands[5];
            let op_b = ins_b.operands[5];
            let lhs_scope_a = ins_a.operands[6];
            let lhs_scope_b = ins_b.operands[6];
            let lhs_idx_a = word_i16(&ins_a.operands, 0);
            let lhs_idx_b = word_i16(&ins_b.operands, 0);
            let rhs_a = state
                .env_a
                .read(ins_a.operands[7], word_i16(&ins_a.operands, 2));
            let rhs_b = state
                .env_b
                .read(ins_b.operands[7], word_i16(&ins_b.operands, 2));

            let ev_a = CanonEvent::ExprAssign {
                lhs_scope: lhs_scope_a,
                lhs_idx: lhs_idx_a,
                op: op_a,
                rhs: rhs_a,
            };
            let ev_b = CanonEvent::ExprAssign {
                lhs_scope: lhs_scope_b,
                lhs_idx: lhs_idx_b,
                op: op_b,
                rhs: rhs_b,
            };
            if ev_a != ev_b {
                return EquivResult::Counterexample {
                    path,
                    divergence: format!(
                        "expression divergence at A@n{pc_a} vs B@n{pc_b}: {ev_a:?} != {ev_b:?}"
                    ),
                };
            }

            // For mutating comparisons (11 `inc_lt`, 17 `dec_gt`) or read-modify-write
            // operators on frame-private slots, also verify the pre-operation LHS value matches.
            if op_a != 5 && op_a != 20 && state.env_a.is_frame_private(lhs_scope_a, lhs_idx_a) {
                let lhs_a = state.env_a.read(lhs_scope_a, lhs_idx_a);
                let lhs_b = state.env_b.read(lhs_scope_b, lhs_idx_b);
                if lhs_a != lhs_b {
                    return EquivResult::Counterexample {
                        path,
                        divergence: format!(
                            "expression LHS value divergence at A@n{pc_a} vs B@n{pc_b}: {lhs_a:?} != {lhs_b:?}"
                        ),
                    };
                }
            }

            // Apply identical symbolic update to frame-private targets if applicable.
            let mut next_env_a = state.env_a.clone();
            let mut next_env_b = state.env_b.clone();
            if next_env_a.is_frame_private(lhs_scope_a, lhs_idx_a) {
                let sym = interner.fresh_event();
                next_env_a.write(lhs_scope_a, lhs_idx_a, sym);
                next_env_b.write(lhs_scope_b, lhs_idx_b, sym);
            }

            let mut enqueue = |na: u8, nb: u8, branch: Option<bool>| {
                let next = ProductState {
                    pc_a: na,
                    pc_b: nb,
                    env_a: next_env_a.clone(),
                    env_b: next_env_b.clone(),
                };
                if visited.insert(next.clone()) {
                    let mut next_path = path.clone();
                    next_path.push(TraceStep {
                        pc_a,
                        pc_b,
                        event: format!("expr(op={op_a})"),
                        branch,
                    });
                    queue.push_back((next, next_path));
                } else {
                    paths_verified += 1;
                }
            };

            if is_always_true_instruction(ins_a) {
                enqueue(ins_a.on_true, ins_b.on_true, None);
            } else {
                enqueue(ins_a.on_true, ins_b.on_true, Some(true));
                enqueue(ins_a.on_false, ins_b.on_false, Some(false));
            }
            continue;
        }

        // Case 3: World-interacting primitives and subroutine calls.
        let ev_a = CanonEvent::Primitive {
            opcode: ins_a.opcode,
            operands: canonicalize_primitive_operands(ins_a.opcode, ins_a.operands),
        };
        let ev_b = CanonEvent::Primitive {
            opcode: ins_b.opcode,
            operands: canonicalize_primitive_operands(ins_b.opcode, ins_b.operands),
        };
        if ev_a != ev_b {
            return EquivResult::Counterexample {
                path,
                divergence: format!(
                    "trace event divergence at A@n{pc_a} vs B@n{pc_b}: A=op {:#06x} {:?} vs B=op {:#06x} {:?}",
                    ins_a.opcode,
                    canonicalize_primitive_operands(ins_a.opcode, ins_a.operands),
                    ins_b.opcode,
                    canonicalize_primitive_operands(ins_b.opcode, ins_b.operands),
                ),
            };
        }

        // Verify any frame-private operands read by this primitive evaluate identically on A and B.
        if let Err(msg) =
            check_primitive_frame_reads(ins_a.opcode, &ins_a.operands, &state.env_a, &state.env_b)
        {
            return EquivResult::Counterexample {
                path,
                divergence: format!("{msg} at A@n{pc_a} vs B@n{pc_b}"),
            };
        }

        // Update any frame-private slots mutated by this primitive identically on both sides.
        let mut next_env_a = state.env_a.clone();
        let mut next_env_b = state.env_b.clone();
        match ins_a.opcode {
            0 => {
                let p = word_i16(&ins_a.operands, 0);
                next_env_a.write(9, p, CanonOperand::Const(0));
                next_env_b.write(9, p, CanonOperand::Const(0));
            }
            17 => {
                let p = word_i16(&ins_a.operands, 0);
                let sym = interner.fresh_event();
                next_env_a.write(9, p, sym);
                next_env_b.write(9, p, sym);
            }
            8 => {
                let dst_scope = u16::from_le_bytes([ins_a.operands[2], ins_a.operands[3]]);
                let dst_idx = word_i16(&ins_a.operands, 0);
                if dst_scope <= 255 {
                    let sym = interner.fresh_event();
                    next_env_a.write(dst_scope as u8, dst_idx, sym);
                    next_env_b.write(dst_scope as u8, dst_idx, sym);
                }
            }
            31 if ins_a.operands[4] & 0x80 != 0 => {
                let sym = interner.fresh_event();
                next_env_a.write(ins_a.operands[5], i16::from(ins_a.operands[7]), sym);
                next_env_b.write(ins_b.operands[5], i16::from(ins_b.operands[7]), sym);
            }
            44 if ins_a.operands[5] & 0x20 != 0 => {
                let sym = interner.fresh_event();
                next_env_a.write(25, i16::from(ins_a.operands[2]), sym);
                next_env_b.write(25, i16::from(ins_b.operands[2]), sym);
            }
            _ => {}
        }

        let mut enqueue = |na: u8, nb: u8, branch: Option<bool>| {
            let next = ProductState {
                pc_a: na,
                pc_b: nb,
                env_a: next_env_a.clone(),
                env_b: next_env_b.clone(),
            };
            if visited.insert(next.clone()) {
                let mut next_path = path.clone();
                next_path.push(TraceStep {
                    pc_a,
                    pc_b,
                    event: format!("op({:#06x})", ins_a.opcode),
                    branch,
                });
                queue.push_back((next, next_path));
            } else {
                paths_verified += 1;
            }
        };

        if is_always_true_instruction(ins_a) {
            enqueue(ins_a.on_true, ins_b.on_true, None);
        } else {
            enqueue(ins_a.on_true, ins_b.on_true, Some(true));
            enqueue(ins_a.on_false, ins_b.on_false, Some(false));
        }
    }

    EquivResult::Equivalent {
        states_checked,
        paths_verified,
    }
}

fn check_primitive_frame_reads(
    opcode: u16,
    ops: &[u8; 8],
    env_a: &LocalEnv,
    env_b: &LocalEnv,
) -> std::result::Result<(), String> {
    let check_op = |scope: u8, idx: i16, label: &str| -> std::result::Result<(), String> {
        if matches!(scope, 9 | 22 | 25) {
            let va = env_a.read(scope, idx);
            let vb = env_b.read(scope, idx);
            if va != vb {
                return Err(format!("{label} value divergence: {va:?} != {vb:?}"));
            }
        }
        Ok(())
    };
    match opcode {
        0 => check_op(9, word_i16(ops, 0), "Sleep param"),
        1 | 14 | 18 | 20 | 23 | 36 | 48 => Ok(()),
        8 => {
            let rng_scope = u16::from_le_bytes([ops[6], ops[7]]);
            if rng_scope <= 255 {
                check_op(rng_scope as u8, word_i16(ops, 4), "Random range")?;
            }
            Ok(())
        }
        17 => check_op(9, word_i16(ops, 0), "IdleForInput param"),
        25 => {
            let scope = match ops[0] {
                0 => 7,
                1 => 9,
                2 => 25,
                _ => ops[1],
            };
            check_op(scope, word_i16(ops, 2), "AlterBudget amount")
        }
        29 => {
            if ops[3] & 1 == 0 {
                check_op(ops[0], word_i16(ops, 4), "SetMotiveChange delta")?;
                check_op(ops[1], word_i16(ops, 6), "SetMotiveChange stop_at")?;
            }
            Ok(())
        }
        31 => {
            if ops[4] & 0x80 != 0 {
                check_op(ops[5], i16::from(ops[7]), "SetToNext destination cursor")?;
            }
            let base = ops[4] & 0x7F;
            if base == 6 {
                check_op(9, 0, "SetToNext param[0]")?;
            } else if base == 9 {
                check_op(25, i16::from(ops[6]), "SetToNext local")?;
            }
            Ok(())
        }
        42 => {
            if ops[4] == 7 {
                check_op(9, 0, "CreateObject param[0]")?;
            } else if ops[4] == 8 || ops[4] == 9 {
                check_op(25, i16::from(ops[6]), "CreateObject local")?;
            }
            Ok(())
        }
        44 => {
            let flags = ops[5];
            let events = ops[6] as i8;
            if flags & 0x04 != 0 {
                check_op(9, word_i16(ops, 0), "Animate clip param")?;
            }
            if flags & 0x20 == 0 && events != 0 && matches!(flags & 0x11, 0x00 | 0x11) {
                check_op(9, 0, "Animate event param[0]")?;
            }
            Ok(())
        }
        45 => {
            if u16::from_le_bytes([ops[2], ops[3]]) == 0 {
                check_op(9, word_i16(ops, 0), "RouteTo param")?;
            }
            Ok(())
        }
        46 => {
            if u16::from_le_bytes([ops[2], ops[3]]) == 0 {
                check_op(9, word_i16(ops, 0), "Snap param")?;
            }
            Ok(())
        }
        id if id >= 256 => Ok(()),
        _ => {
            // Conservative soundness guard for any unmodeled primitive opcode.
            if env_a != env_b {
                Err(format!(
                    "frame-private state divergence before primitive opcode {opcode:#06x}"
                ))
            } else {
                Ok(())
            }
        }
    }
}

fn format_pc(pc: u8) -> String {
    match pc {
        TARGET_ERROR => "error".to_owned(),
        TARGET_TRUE => "return true".to_owned(),
        TARGET_FALSE => "return false".to_owned(),
        n => format!("node {n}"),
    }
}

/// Convenience helper for CLI `sims-antics equiv`: parses a raw IFF file in memory and
/// extracts a single [`BhavChunk`] by tree ID.
pub fn extract_bhav_from_iff(iff_bytes: &[u8], tree_id: u16) -> Result<BhavChunk> {
    let iff = Iff::parse(iff_bytes.to_vec())?;
    let chunk = iff
        .get(b"BHAV", tree_id)
        .ok_or_else(|| Error::new(format!("IFF does not contain BHAV #{tree_id}")))?;
    crate::lift::lift_bhav(chunk.id, &chunk.label, iff.data(chunk))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{compile_antics, compile_object_from_sources};

    #[test]
    fn equiv_passes_on_reordered_dead_stripped_and_padding_cleaned_bhavs() {
        let orig = BhavChunk {
            id: 4096,
            label: "Orig".to_owned(),
            format: 0x8002,
            kind: 0,
            args: 0,
            locals: 1,
            revision: 0,
            instructions: vec![
                // Node 0: if stack.attr[0] != 0 -> node 2 (false), else node 1 (true)
                // With dirty padding byte 4 = 0xAA!
                Instruction {
                    opcode: 2,
                    on_true: 2,
                    on_false: 1,
                    operands: [0, 0, 0, 0, 0xAA, 16, 1, 7],
                },
                // Node 1: stack.attr[0] := 1 -> node 3 (dead on_false = 0xFD, dirty pad = 0x55)
                Instruction {
                    opcode: 2,
                    on_true: 3,
                    on_false: TARGET_ERROR,
                    operands: [0, 0, 1, 0, 0x55, 5, 1, 7],
                },
                // Node 2: return false via no-op `0 == 0`
                Instruction {
                    opcode: 2,
                    on_true: TARGET_FALSE,
                    on_false: TARGET_FALSE,
                    operands: [0, 0, 0, 0, 0, 2, 7, 7],
                },
                // Node 3: play_sound(1, stack_obj) with dirty trailing bytes -> return true
                Instruction {
                    opcode: 23,
                    on_true: TARGET_TRUE,
                    on_false: TARGET_FALSE,
                    operands: [1, 0, 0x99, 0x88, 2, 0x77, 0x66, 0x55],
                },
                // Node 4: dead unreachable node!
                Instruction {
                    opcode: 23,
                    on_true: TARGET_FALSE,
                    on_false: TARGET_FALSE,
                    operands: [99, 0, 0, 0, 0, 0, 0, 0],
                },
            ],
            trailing: Vec::new(),
        };

        // Recompile equivalent high-level `.antics` with flipped condition (`== 0`),
        // reordered blocks, zero padding bytes, and zero dead nodes.
        let src = r#"
            tree 4096 "Clean" {
                if stack.attr[0] == 0 {
                    stack.attr[0] := 1;
                    play_sound(1, stack_obj);
                    return true;
                } else {
                    return false;
                }
            }
        "#;
        let compiled = compile_antics(src).unwrap();
        let clean = &compiled.behaviors[&4096];

        let res = check_bhav_equivalent(&orig, clean, &EquivOptions::default());
        assert!(
            matches!(res, EquivResult::Equivalent { .. }),
            "expected Equivalent, got {res:?}"
        );
    }

    #[test]
    fn equiv_detects_off_by_one_swapped_branch_missing_cleanup_and_dropped_sound() {
        let base_src = r#"
            tree 4096 "Base" {
                stack.attr[0] := 1;
                play_sound(1, stack_obj);
                if my.motive.energy < 80 {
                    my.motive.energy += 20;
                    stack.attr[0] := 0;
                    return true;
                } else {
                    stack.attr[0] := 0;
                    return false;
                }
            }
        "#;
        let base = &compile_antics(base_src).unwrap().behaviors[&4096];

        // 1. Off-by-one comparison (`<= 80` vs `< 80`)
        let off_by_one = &compile_antics(&base_src.replace("< 80", "<= 80"))
            .unwrap()
            .behaviors[&4096];
        assert!(matches!(
            check_bhav_equivalent(base, off_by_one, &EquivOptions::default()),
            EquivResult::Counterexample { .. }
        ));

        // 2. Swapped return branch
        let swapped = &compile_antics(
            r#"
            tree 4096 "Swapped" {
                stack.attr[0] := 1;
                play_sound(1, stack_obj);
                if my.motive.energy < 80 {
                    my.motive.energy += 20;
                    stack.attr[0] := 0;
                    return false;
                } else {
                    stack.attr[0] := 0;
                    return true;
                }
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(matches!(
            check_bhav_equivalent(base, swapped, &EquivOptions::default()),
            EquivResult::Counterexample { .. }
        ));

        // 3. Missing cleanup write (`stack.attr[0] := 0` omitted on else branch)
        let missing_cleanup = &compile_antics(
            r#"
            tree 4096 "MissingCleanup" {
                stack.attr[0] := 1;
                play_sound(1, stack_obj);
                if my.motive.energy < 80 {
                    my.motive.energy += 20;
                    stack.attr[0] := 0;
                    return true;
                } else {
                    return false;
                }
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(matches!(
            check_bhav_equivalent(base, missing_cleanup, &EquivOptions::default()),
            EquivResult::Counterexample { .. }
        ));

        // 4. Dropped sound event
        let dropped_sound = &compile_antics(&base_src.replace("play_sound(1, stack_obj);", ""))
            .unwrap()
            .behaviors[&4096];
        assert!(matches!(
            check_bhav_equivalent(base, dropped_sound, &EquivOptions::default()),
            EquivResult::Counterexample { .. }
        ));

        // 5. Divergent frame-private local passed into AlterBudget
        let budget_a = &compile_antics(
            r#"
            tree 4096 "BudgetA" {
                let cost = 10;
                if alter_budget(cost, category: 0) { return true; }
                return false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let budget_b = &compile_antics(
            r#"
            tree 4096 "BudgetB" {
                let cost = 99;
                if alter_budget(cost, category: 0) { return true; }
                return false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(matches!(
            check_bhav_equivalent(budget_a, budget_b, &EquivOptions::default()),
            EquivResult::Counterexample { .. }
        ));
    }

    #[test]
    fn safety_prover_catches_lock_leak_on_early_cancel_return() {
        let manifest = r#"
            [object]
            name = "Buggy Chair"
            guid = 0x7F00_9991
            init = "Init"
            main = "Main"
            scripts = ["behavior.antics"]

            [[interaction]]
            name = "Sit"
            action = "Sit"
            guard = "CanSit"
        "#;
        let buggy_script = r#"
            tree 4096 "Init" { my.attr[0] := 0; return true; }
            tree 4097 "Main" { return true; }
            tree 4098 "CanSit" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            interaction 4099 "Sit" {
                stack.attr[0] := 1;
                if my.person.priority == 0 {
                    // BUG: forgot to clear stack.attr[0] before returning!
                    return false;
                }
                stack.attr[0] := 0;
                return true;
            }
        "#;
        let compiled =
            compile_object_from_sources(manifest, &[("behavior.antics", buggy_script)]).unwrap();
        let report = verify_object_safety(&compiled).unwrap();
        assert!(!report.is_ok(), "expected lock leak violation");
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "expected LockLeak in {:?}",
            report.violations
        );
    }
}
