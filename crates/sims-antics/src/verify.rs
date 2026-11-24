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

mod iff;
mod live;
mod replay;
pub use iff::{
    IffEquivOptions, IffEquivReport, IffVerdict, Reach, ResourceReport, ResourceVerdict,
    check_iff_equivalent, check_tree_with_callees, library_argless_from_iff, library_arities_from_iff,
    not_run_from_file,
    runs_from_own_file, same_tree_bytes, single_file_problem, tree_reach,
};
pub use live::{GUARANTEED_ATTRIBUTES, LiveContext, LiveOptions, check_bhav_live_equivalent};

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
    InconclusiveProof,
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
    /// Zero turns silent steps off, so every node is matched with a node of the other tree.
    pub max_silent_steps: usize,
    /// Require the thread's last branch result to agree at exit and before primitives
    /// (see [`LiveOptions::observe_last_result`]). Off by default: a structured lift may
    /// word a final test the other way round. Needs `max_silent_steps == 0`, since a
    /// silent step changes the result on one side only.
    pub observe_last_result: bool,
}

impl Default for EquivOptions {
    fn default() -> Self {
        Self {
            max_states: 4096,
            max_depth: 512,
            max_silent_steps: 64,
            observe_last_result: false,
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
    string_entries: BTreeMap<u16, Vec<String>>,
    ttas: BTreeMap<u16, usize>,
    ttabs: BTreeMap<u16, InteractionTable>,
    slots: BTreeMap<u16, usize>,
    routing_slots: BTreeMap<u16, usize>,
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
        string_entries: BTreeMap::new(),
        ttas: BTreeMap::new(),
        ttabs: BTreeMap::new(),
        slots: BTreeMap::new(),
        routing_slots: BTreeMap::new(),
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
                    // Match `sims-vm` US English loading: language 0 (non-localized) or 1 (US English).
                    let en_entries: Vec<String> = st
                        .entries
                        .iter()
                        .filter(|e| e.language == 0 || e.language == 1)
                        .map(|e| e.text.clone())
                        .collect();
                    model.strings.insert(chunk.id, en_entries.len());
                    model.string_entries.insert(chunk.id, en_entries);
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
                    let routing_count = sl.entries.iter().filter(|s| s.kind == 3).count();
                    model.slots.insert(chunk.id, sl.entries.len());
                    model.routing_slots.insert(chunk.id, routing_count);
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
    // Match `sims-vm/src/program.rs:1019`: Route/Snap use the definition's linked SLOT table
    // filtered to kind-3 routing entries.
    let mut linked_routing_counts: Vec<usize> = Vec::new();
    for objd in model.objds.values() {
        let slots_id = objd.words.get(20).copied().unwrap_or(0);
        if slots_id != 0 {
            linked_routing_counts.push(model.routing_slots.get(&slots_id).copied().unwrap_or(0));
        }
    }
    let routing_slot_count = linked_routing_counts.into_iter().min().unwrap_or(0);
    let caller_param_consts = compute_private_helper_param_consts(&model);
    for (&tree_id, (tree_name, bhav)) in &model.behaviors {
        verify_bhav_static(
            &model,
            tree_id,
            tree_name,
            bhav,
            routing_slot_count,
            caller_param_consts.get(&tree_id),
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

/// Whether `ins` calls a tree in `argless`: one that takes no arguments and is not
/// indexed (kind 4). Such a call reads none of its operand bytes: it copies no
/// argument words, and only an indexed tree enters at the node its first word (or
/// `temp[0]`, for `[0xFF; 8]`) names (`interpreter.rs:1539`, `entry_node`). Every
/// object has eight temps, so the pass-temps form cannot fail where another cannot.
pub(crate) fn is_argless_call(ins: &Instruction, argless: Option<&BTreeSet<u16>>) -> bool {
    ins.opcode >= 256 && argless.is_some_and(|set| set.contains(&ins.opcode))
}

/// `ins`'s operand bytes as the VM uses them: `[0xFF; 8]` for any spelling of a call to
/// a tree that reads none of them ([`is_argless_call`]), else the bytes as they are.
/// Bytes a call with more than four inline arguments reads past its own node stay
/// compared raw (`collect_inline_call_arg_bytes`).
pub(crate) fn call_operands(ins: &Instruction, argless: Option<&BTreeSet<u16>>) -> [u8; 8] {
    if is_argless_call(ins, argless) {
        [0xFF; 8]
    } else {
        ins.operands
    }
}

/// Whether the VM calls `bhav` the same with any operand bytes ([`is_argless_call`]).
pub(crate) fn reads_no_call_operands(bhav: &Behavior) -> bool {
    bhav.kind != 4 && bhav_param_limit(bhav) == 0
}

pub(crate) fn bhav_param_limit(bhav: &Behavior) -> usize {
    match bhav.format {
        0x8000 => 4,
        0x8001 => usize::from(bhav.args).min(4),
        _ => usize::from(bhav.args),
    }
}

fn bhav_local_limit(bhav: &Behavior) -> usize {
    // Match `sims_vm::interpreter::dimensions`: the local count is truncated to a byte.
    let low = usize::from(bhav.locals as u8);
    match bhav.format {
        0x8000 => 0,
        0x8001 => low.min(4),
        _ => low,
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

/// Returns true iff `opcode` is implemented in `sims-vm/src/primitives.rs` (0..=37, 41..=51).
fn is_known_primitive(opcode: u16) -> bool {
    matches!(opcode, 0..=37 | 41..=51)
}

/// Returns true iff `scope` is a valid SimAntics memory scope implemented in `sims-vm/src/memory.rs`.
fn is_valid_memory_scope(scope: u8) -> bool {
    matches!(scope, 0 | 1 | 3 | 4 | 6..=38)
}

/// Returns true iff `scope` is read-only in `sims-vm/src/memory.rs` (writes trap with 0x13).
fn is_read_only_scope(scope: u8) -> bool {
    matches!(scope, 7 | 21 | 23 | 24 | 26 | 32 | 33 | 34 | 35 | 38)
}

/// Returns true iff `routine` is implemented in `sims-vm/src/primitives/generic.rs`.
fn is_known_generic_call(routine: i16) -> bool {
    matches!(
        routine,
        2 | 6 | 10 | 11 | 12 | 14 | 15 | 18 | 26 | 28 | 35 | 43
    )
}

/// Legacy OBJD callback word indices matching `sims-vm/src/program.rs:213` (`DEFINITION_CALLBACKS`).
const OBJD_CALLBACK_WORDS: [usize; 30] = [
    0x56 / 2,
    0x0a / 2,
    0x5e / 2,
    0x68 / 2,
    0x78 / 2,
    0x2a / 2,
    0x7c / 2,
    0x64 / 2,
    0x9c / 2,
    0x58 / 2,
    0x94 / 2,
    0x60 / 2,
    0x5a / 2,
    0x6a / 2,
    0x6e / 2,
    0x22 / 2,
    0x0c / 2,
    0x18 / 2,
    0x30 / 2,
    0x32 / 2,
    0x34 / 2,
    0x36 / 2,
    0x38 / 2,
    0x3a / 2,
    0x3c / 2,
    0x3e / 2,
    0x40 / 2,
    0x42 / 2,
    0x76 / 2,
    0xa0 / 2,
];

/// For private helper BHAVs that are NOT external entry points (OBJD/OBJf/TTAB) and are
/// invoked exclusively via inline constant calls (`operands != [0xFF; 8]`), computes the
/// exact finite set of constant values each parameter `0..4` can take across all callers.
fn compute_private_helper_param_consts(
    model: &ObjectModel,
) -> BTreeMap<u16, [Option<BTreeSet<i16>>; 4]> {
    let mut entry_points = BTreeSet::new();
    for objd in model.objds.values() {
        for &idx in &OBJD_CALLBACK_WORDS {
            let id = objd.words.get(idx).copied().unwrap_or(0);
            if id >= 4096 {
                entry_points.insert(id);
            }
        }
    }
    for objf in model.objfs.values() {
        for e in &objf.entries {
            if e.condition >= 4096 {
                entry_points.insert(e.condition);
            }
            if e.action >= 4096 {
                entry_points.insert(e.action);
            }
        }
    }
    for ttab in model.ttabs.values() {
        for inter in &ttab.interactions {
            if inter.action >= 4096 {
                entry_points.insert(inter.action);
            }
            if inter.test >= 4096 {
                entry_points.insert(inter.test);
            }
        }
    }

    let mut result: BTreeMap<u16, [Option<BTreeSet<i16>>; 4]> = BTreeMap::new();
    let mut poisoned: BTreeSet<u16> = entry_points;
    let mut called: BTreeSet<u16> = BTreeSet::new();
    let mut has_dynamic_tree_dispatch = false;

    for (_, bhav) in model.behaviors.values() {
        let reach = if bhav.kind == 4 {
            vec![true; bhav.instructions.len()]
        } else {
            reachable_nodes(&bhav.instructions)
        };
        for (i, ins) in bhav.instructions.iter().enumerate() {
            if !reach[i] {
                continue;
            }
            // `FindBestInteraction` (3), `RunFunctionalTree` (20), `RunTreeByName` (28),
            // `GosubFoundAction` (30), and `Reach` (47, which runs `"CT - Grab"` / `"CT - Put"`
            // by name) can invoke private trees dynamically, so no private helper has a closed
            // inline caller set.
            if matches!(ins.opcode, 3 | 20 | 28 | 30 | 47) {
                has_dynamic_tree_dispatch = true;
            }
            if !(4096..=8191).contains(&ins.opcode) {
                continue;
            }
            called.insert(ins.opcode);
            if ins.operands == [0xFF; 8] {
                poisoned.insert(ins.opcode);
                continue;
            }
            let entry = result
                .entry(ins.opcode)
                .or_insert_with(|| std::array::from_fn(|_| Some(BTreeSet::new())));
            for (p, slot) in entry.iter_mut().enumerate() {
                if let Some(set) = slot {
                    set.insert(word_i16(&ins.operands, p * 2));
                    if set.len() > 16 {
                        *slot = None;
                    }
                }
            }
        }
    }

    if has_dynamic_tree_dispatch {
        return BTreeMap::new();
    }
    result.retain(|id, _| called.contains(id) && !poisoned.contains(id));
    result
}

type SlotConstMap = BTreeMap<i16, Option<BTreeSet<i16>>>;

/// Forward constant-set dataflow over reachable CFG nodes for sound static trap checking.
fn compute_node_const_sets(
    model: &ObjectModel,
    bhav: &Behavior,
    reachable: &[bool],
    init_params: Option<&[Option<BTreeSet<i16>>; 4]>,
) -> Vec<(SlotConstMap, SlotConstMap)> {
    let n = bhav.instructions.len();
    let mut in_states: Vec<Option<(SlotConstMap, SlotConstMap)>> = vec![None; n];
    if n == 0 {
        return Vec::new();
    }

    let mut init_p = BTreeMap::new();
    if let Some(pconsts) = init_params {
        for (i, slot) in pconsts.iter().enumerate() {
            if let Some(set) = slot {
                init_p.insert(i as i16, Some(set.clone()));
            }
        }
    }
    let init_l = BTreeMap::new(); // Uninitialized locals are Top (None) — never assumed 0!
    let mut work = VecDeque::new();
    if bhav.kind == 4 {
        for (i, state) in in_states.iter_mut().enumerate() {
            *state = Some((init_p.clone(), init_l.clone()));
            work.push_back(i);
        }
    } else {
        in_states[0] = Some((init_p, init_l));
        work.push_back(0usize);
    }

    let join_slot = |dst: &mut BTreeMap<i16, Option<BTreeSet<i16>>>,
                     src: &BTreeMap<i16, Option<BTreeSet<i16>>>|
     -> bool {
        let mut changed = false;
        let keys: BTreeSet<i16> = dst.keys().chain(src.keys()).copied().collect();
        for k in keys {
            let a = dst.get(&k).and_then(|v| v.as_ref());
            let b = src.get(&k).and_then(|v| v.as_ref());
            let merged = match (a, b) {
                (Some(sa), Some(sb)) => {
                    let mut u = sa.clone();
                    u.extend(sb.iter().copied());
                    if u.len() <= 16 { Some(u) } else { None }
                }
                _ => None,
            };
            if dst.get(&k) != Some(&merged) {
                dst.insert(k, merged);
                changed = true;
            }
        }
        changed
    };

    while let Some(u) = work.pop_front() {
        if !reachable[u] {
            continue;
        }
        let Some((cur_p, cur_l)) = in_states[u].clone() else {
            continue;
        };
        let ins = &bhav.instructions[u];
        let ops = &ins.operands;
        // Compute branch-specific outgoing const sets:
        // - Relationship read (opcode 26) writes its destination ONLY on `on_true` (`relationship.rs:190`).
        // - Animate (opcode 44) writes its event destination ONLY on `on_false` (`animate.rs:197`).
        let mut true_p = cur_p.clone();
        let mut true_l = cur_l.clone();
        let mut false_p = cur_p;
        let mut false_l = cur_l;

        match ins.opcode {
            0 => {
                let p = word_i16(ops, 0);
                true_p.insert(p, Some(BTreeSet::from([0])));
                false_p.insert(p, Some(BTreeSet::from([0])));
            }
            2 => {
                let lhs_idx = word_i16(ops, 0);
                let rhs_idx = word_i16(ops, 2);
                let op = ops[5];
                let lhs_scope = ops[6];
                let rhs_scope = ops[7];
                if matches!(op, 3..=7 | 9..=13 | 17..=20) && matches!(lhs_scope, 9 | 25) {
                    let rhs_vals = if op == 5 {
                        match rhs_scope {
                            7 => Some(BTreeSet::from([rhs_idx])),
                            9 => true_p.get(&rhs_idx).and_then(|v| v.clone()),
                            25 => true_l.get(&rhs_idx).and_then(|v| v.clone()),
                            26 => lookup_bcon(model, rhs_idx).map(|v| BTreeSet::from([v])),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if lhs_scope == 9 {
                        true_p.insert(lhs_idx, rhs_vals.clone());
                        false_p.insert(lhs_idx, rhs_vals);
                    } else {
                        true_l.insert(lhs_idx, rhs_vals.clone());
                        false_l.insert(lhs_idx, rhs_vals);
                    }
                }
            }
            8 | 12 => {
                let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
                let dst_idx = word_i16(ops, 0);
                if dst_scope == 9 {
                    true_p.insert(dst_idx, None);
                    false_p.insert(dst_idx, None);
                } else if dst_scope == 25 {
                    true_l.insert(dst_idx, None);
                    false_l.insert(dst_idx, None);
                }
            }
            17 => {
                let p = word_i16(ops, 0);
                true_p.insert(p, None);
                false_p.insert(p, None);
            }
            24 if ops[0] == 0 => {
                // Old Relationship read writes `param[ops[2] as i8]` on `on_true` only (`relationship.rs:61`).
                true_p.insert(i16::from(ops[2] as i8), None);
            }
            26 if ops[2] & 4 == 0 => {
                // Relationship read writes into `(ops[4], word_i16(ops, 6))` on `on_true` only.
                if ops[4] == 9 {
                    true_p.insert(word_i16(ops, 6), None);
                } else if ops[4] == 25 {
                    true_l.insert(word_i16(ops, 6), None);
                }
            }
            31 if ops[4] & 0x80 != 0 => {
                let idx = i16::from(ops[7]);
                if ops[5] == 9 {
                    true_p.insert(idx, None);
                    false_p.insert(idx, None);
                } else if ops[5] == 25 {
                    true_l.insert(idx, None);
                    false_l.insert(idx, None);
                }
            }
            44 => {
                // Animate stores event into local/param[0] on `on_false` only (`animate.rs:197`).
                if ops[5] & 0x20 != 0 {
                    false_l.insert(i16::from(ops[2]), None);
                } else {
                    false_p.insert(0, None);
                }
            }
            _ => {}
        }

        let mut propagate =
            |t: u8,
             out_p: &BTreeMap<i16, Option<BTreeSet<i16>>>,
             out_l: &BTreeMap<i16, Option<BTreeSet<i16>>>| {
                if !matches!(t, TARGET_ERROR..=TARGET_FALSE) && usize::from(t) < n {
                    let v = usize::from(t);
                    match &mut in_states[v] {
                        None => {
                            in_states[v] = Some((out_p.clone(), out_l.clone()));
                            work.push_back(v);
                        }
                        Some((vp, vl)) => {
                            let cp = join_slot(vp, out_p);
                            let cl = join_slot(vl, out_l);
                            if cp || cl {
                                work.push_back(v);
                            }
                        }
                    }
                }
            };
        propagate(ins.on_true, &true_p, &true_l);
        if !is_always_true_instruction(ins) {
            propagate(ins.on_false, &false_p, &false_l);
        }
    }

    in_states
        .into_iter()
        .map(|s| s.unwrap_or_default())
        .collect()
}

fn verify_bhav_static(
    model: &ObjectModel,
    tree_id: u16,
    tree_name: &str,
    bhav: &Behavior,
    total_slots: usize,
    init_params: Option<&[Option<BTreeSet<i16>>; 4]>,
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

    // Kind-4 indexed trees can enter at any node (`interpreter.rs:1786`), so all nodes are reachable.
    let reachable = if bhav.kind == 4 {
        vec![true; n]
    } else {
        reachable_nodes(&bhav.instructions)
    };
    let node_consts = compute_node_const_sets(model, bhav, &reachable, init_params);
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

        // Reachable TARGET_ERROR (0xFD) exits fault the VM thread.
        if ins.on_true == TARGET_ERROR
            || (!is_always_true_instruction(ins) && ins.on_false == TARGET_ERROR)
        {
            add_err(
                violations,
                SafetyCategory::PotentialTrap,
                idx,
                "reachable instruction targets error exit (0xFD)".to_owned(),
            );
        }

        // Helper to validate a (scope, index) operand reference.
        let check_operand = |scope: u8,
                             index: i16,
                             is_write: bool,
                             violations: &mut Vec<SafetyViolation>| {
            if !is_valid_memory_scope(scope) {
                add_err(
                    violations,
                    SafetyCategory::PotentialTrap,
                    idx,
                    format!("invalid memory scope {scope} (index {index})"),
                );
                return;
            }
            if is_write && is_read_only_scope(scope) {
                add_err(
                    violations,
                    SafetyCategory::PotentialTrap,
                    idx,
                    format!("cannot write to read-only memory scope {scope}"),
                );
                return;
            }
            // In `sims-vm/src/memory.rs:669`, globals are read-only except global 34 and global 36
            // (and global 3 in coop mode). Writing e.g. `global[0]` traps with 0x13.
            if scope == 6 && is_write && !matches!(index, 34 | 36) {
                add_err(
                    violations,
                    SafetyCategory::PotentialTrap,
                    idx,
                    format!("cannot write to read-only global[{index}]"),
                );
                return;
            }
            match scope {
                7 => {}
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
                1 => {
                    let routine = word_i16(ops, 0);
                    if !is_known_generic_call(routine) {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("unimplemented Generic Sims Call routine {routine}"),
                        );
                    }
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
                        let (ref cur_p, ref cur_l) = node_consts[idx];
                        let static_rhs_vals: Option<BTreeSet<i16>> = match rhs_scope {
                            7 => Some(BTreeSet::from([rhs_idx])),
                            9 => cur_p.get(&rhs_idx).and_then(|v| v.clone()),
                            25 => cur_l.get(&rhs_idx).and_then(|v| v.clone()),
                            26 => lookup_bcon(model, rhs_idx).map(|v| BTreeSet::from([v])),
                            _ => None,
                        };
                        if matches!(operator, 7 | 12) {
                            match static_rhs_vals.as_ref() {
                                Some(vals) if !vals.is_empty() && vals.iter().all(|&v| v != 0) => {}
                                Some(vals) if vals.contains(&0) => add_err(
                                    violations,
                                    SafetyCategory::PotentialTrap,
                                    idx,
                                    "Expression division/modulo by constant 0".to_owned(),
                                ),
                                _ => add_err(
                                    violations,
                                    SafetyCategory::PotentialTrap,
                                    idx,
                                    "Expression division/modulo by unproved non-constant divisor"
                                        .to_owned(),
                                ),
                            }
                        }
                        if matches!(operator, 8..=10) {
                            match static_rhs_vals.as_ref() {
                                Some(vals)
                                    if !vals.is_empty()
                                        && vals.iter().all(|&b| (0..=16).contains(&b)) => {}
                                Some(vals) => {
                                    let bad = vals
                                        .iter()
                                        .copied()
                                        .find(|b| !(0..=16).contains(b))
                                        .unwrap_or(-1);
                                    add_err(
                                        violations,
                                        SafetyCategory::PotentialTrap,
                                        idx,
                                        format!("Expression flag bit {bad} out of 0..=16 range"),
                                    );
                                }
                                None => add_err(
                                    violations,
                                    SafetyCategory::PotentialTrap,
                                    idx,
                                    "Expression flag bit is non-constant and unproved in 0..=16"
                                        .to_owned(),
                                ),
                            }
                        }
                        if operator == 20 {
                            match static_rhs_vals.as_ref() {
                                Some(vals) if !vals.is_empty() && vals.iter().all(|&v| v >= 0) => {}
                                Some(vals) => {
                                    let bad = vals.iter().copied().find(|&v| v < 0).unwrap_or(-1);
                                    add_err(
                                        violations,
                                        SafetyCategory::PotentialTrap,
                                        idx,
                                        format!("Expression sqrt of negative constant {bad}"),
                                    );
                                }
                                None => add_err(
                                    violations,
                                    SafetyCategory::PotentialTrap,
                                    idx,
                                    "Expression sqrt of unproved non-constant operand".to_owned(),
                                ),
                            }
                        }
                    }
                }
                8 => {
                    let dst_idx = word_i16(ops, 0);
                    let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
                    let rng_idx = word_i16(ops, 4);
                    let rng_scope = u16::from_le_bytes([ops[6], ops[7]]);
                    if dst_scope > 255 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("Random destination scope {dst_scope} exceeds 255"),
                        );
                    } else {
                        check_operand(dst_scope as u8, dst_idx, true, violations);
                    }
                    if rng_scope > 255 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("Random range scope {rng_scope} exceeds 255"),
                        );
                    } else {
                        check_operand(rng_scope as u8, rng_idx, false, violations);
                        let (ref cur_p, ref cur_l) = node_consts[idx];
                        let static_rng_vals: Option<BTreeSet<i16>> = match rng_scope as u8 {
                            7 => Some(BTreeSet::from([rng_idx])),
                            9 => cur_p.get(&rng_idx).and_then(|v| v.clone()),
                            25 => cur_l.get(&rng_idx).and_then(|v| v.clone()),
                            26 => lookup_bcon(model, rng_idx).map(|v| BTreeSet::from([v])),
                            _ => None,
                        };
                        match static_rng_vals.as_ref() {
                            Some(vals) if !vals.is_empty() && vals.iter().all(|&v| v != 0) => {}
                            Some(vals) if vals.contains(&0) => add_err(
                                violations,
                                SafetyCategory::PotentialTrap,
                                idx,
                                "Random range is constant 0".to_owned(),
                            ),
                            _ => add_err(
                                violations,
                                SafetyCategory::PotentialTrap,
                                idx,
                                "Random range is unproved non-constant (could be 0)".to_owned(),
                            ),
                        }
                    }
                }
                6 => {
                    if ops[1] > 2 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("ChangeSuit invalid scope {} (must be 0..=2)", ops[1]),
                        );
                    }
                }
                7 => {
                    let selector = word_i16(ops, 0);
                    let mode = word_i16(ops, 2);
                    if !(0..=1).contains(&selector) {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("Refresh invalid object selector {selector} (must be 0 or 1)"),
                        );
                    }
                    if !(0..=2).contains(&mode) {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("Refresh invalid mode {mode} (must be 0..=2)"),
                        );
                    }
                }
                10 => {
                    if ops[0] > 1 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("Tutorial invalid mode {} (must be 0 or 1)", ops[0]),
                        );
                    }
                }
                11 => {
                    let temp = word_i16(ops, 0);
                    if !(0..=7).contains(&temp) {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("GetDistanceTo temp {temp} out of 0..=7 range"),
                        );
                    }
                    if ops[2] & 1 != 0 {
                        check_operand(ops[3], word_i16(ops, 4), false, violations);
                    }
                }
                12 => {
                    let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
                    if dst_scope > 255 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("GetDirectionTo destination scope {dst_scope} exceeds 255"),
                        );
                    } else {
                        check_operand(dst_scope as u8, word_i16(ops, 0), true, violations);
                        if matches!(dst_scope, 14 | 15 | 17) {
                            add_err(
                                violations,
                                SafetyCategory::PotentialTrap,
                                idx,
                                format!(
                                    "GetDirectionTo destination scope {dst_scope} is a float motive (no word pointer)"
                                ),
                            );
                        }
                    }
                    if ops[4] & 1 != 0 {
                        check_operand(ops[5], word_i16(ops, 6), false, violations);
                    }
                }
                13 => {
                    let obj_slot = i16::from(ops[1]);
                    if ops[3] & 2 != 0 {
                        check_operand(25, obj_slot, false, violations);
                    } else {
                        check_operand(9, obj_slot, false, violations);
                    }
                    if ops[3] & 1 != 0 {
                        check_operand(25, i16::from(ops[4]), false, violations);
                    }
                }
                14 => {
                    if ops[6] & 8 != 0 {
                        let scope = u16::from_le_bytes([ops[4], ops[5]]);
                        if scope <= 255 {
                            check_operand(scope as u8, word_i16(ops, 2), false, violations);
                        } else {
                            add_err(
                                violations,
                                SafetyCategory::PotentialTrap,
                                idx,
                                format!("FindBestObject origin scope {scope} exceeds u8 range"),
                            );
                        }
                    }
                }
                15 => {
                    let scope = word_i16(ops, 2);
                    let cond_idx = word_i16(ops, 0);
                    if !(0..=255).contains(&scope) {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("Breakpoint scope {scope} out of 0..=255 range"),
                        );
                    } else {
                        check_operand(scope as u8, cond_idx, false, violations);
                        let (ref cur_p, ref cur_l) = node_consts[idx];
                        let static_cond_vals: Option<BTreeSet<i16>> = match scope as u8 {
                            7 => Some(BTreeSet::from([cond_idx])),
                            9 => cur_p.get(&cond_idx).and_then(|v| v.clone()),
                            25 => cur_l.get(&cond_idx).and_then(|v| v.clone()),
                            26 => lookup_bcon(model, cond_idx).map(|v| BTreeSet::from([v])),
                            _ => None,
                        };
                        match static_cond_vals.as_ref() {
                            Some(vals) if !vals.is_empty() && vals.iter().all(|&v| v == 0) => {}
                            Some(vals) if vals.iter().any(|&v| v != 0) => {
                                let bad = vals.iter().copied().find(|&v| v != 0).unwrap_or(1);
                                add_err(
                                    violations,
                                    SafetyCategory::PotentialTrap,
                                    idx,
                                    format!(
                                        "Breakpoint condition can be nonzero ({bad}), raising native error 0x3e"
                                    ),
                                );
                            }
                            _ => add_err(
                                violations,
                                SafetyCategory::PotentialTrap,
                                idx,
                                "Breakpoint condition is unproved non-zero and may raise native error 0x3e"
                                    .to_owned(),
                            ),
                        }
                    }
                }
                16 => {
                    if matches!(ops[0], 3 | 4) && ops[2] & 1 == 0 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!(
                                "FindLocation mode {} requires flag bit 0 (reference local)",
                                ops[0]
                            ),
                        );
                    }
                    if ops[2] & 1 != 0 {
                        check_operand(25, i16::from(ops[1]), false, violations);
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
                19 => {
                    check_operand(25, i16::from(ops[0]), false, violations);
                    check_operand(25, i16::from(ops[1]), false, violations);
                    check_operand(25, i16::from(ops[2]), false, violations);
                }
                22 => {
                    let mode = word_i16(ops, 0);
                    if !(0..=3).contains(&mode) {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("LookTowards invalid mode {mode} (must be 0..=3)"),
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
                24 => {
                    if (ops[1] as i8) < 0 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("OldRelationship negative column {}", ops[1] as i8),
                        );
                    }
                    if ops[3] > 3 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!(
                                "OldRelationship invalid direction {} (must be 0..=3)",
                                ops[3]
                            ),
                        );
                    }
                    if matches!(ops[3], 2 | 3) {
                        check_operand(9, 0, false, violations);
                    }
                    let writes_param = ops[0] == 0;
                    check_operand(9, i16::from(ops[2] as i8), writes_param, violations);
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
                26 => {
                    if ops[1] > 3 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("Relationship invalid direction {} (must be 0..=3)", ops[1]),
                        );
                    }
                    if matches!(ops[1], 2 | 3) {
                        check_operand(25, i16::from(ops[3]), false, violations);
                    }
                    let writes_operand = ops[2] & 4 == 0;
                    check_operand(ops[4], word_i16(ops, 6), writes_operand, violations);
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
                32 => {
                    check_operand(ops[6], word_i16(ops, 4), false, violations);
                }
                33 => {
                    let target = u16::from_le_bytes([ops[4], ops[5]]);
                    let mode = u16::from_le_bytes([ops[6], ops[7]]);
                    if target == 2 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            "FindWorstMotives target 2 always names no person".to_owned(),
                        );
                    }
                    if mode > 2 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("FindWorstMotives invalid mode {mode} (must be 0..=2)"),
                        );
                    }
                }
                34 => {
                    if ops[0] <= 2 {
                        check_operand(ops[1], word_i16(ops, 2), false, violations);
                    }
                }
                35 => {
                    if ops[4] & 0x20 != 0 {
                        check_operand(25, word_i16(ops, 0), false, violations);
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
                37 => {
                    if ops[0] == 1 {
                        check_operand(ops[1], word_i16(ops, 2), false, violations);
                    }
                }
                41 => {
                    if ops[3] == 7 && ops[2] == 2 {
                        check_operand(25, word_i16(ops, 0) >> 1, false, violations);
                    }
                }
                42 => {
                    let mode = ops[4];
                    if mode == 7 {
                        check_operand(9, 0, false, violations);
                    } else if mode == 8 || mode == 9 {
                        // `create.rs:177` sign-extends `operands[6] as i8`.
                        check_operand(25, i16::from(ops[6] as i8), false, violations);
                    }
                }
                43 => {
                    if word_i16(ops, 0) != 0 {
                        check_operand(9, word_i16(ops, 2), false, violations);
                    }
                    if word_i16(ops, 4) != 0 {
                        check_operand(9, word_i16(ops, 6), false, violations);
                    }
                }
                44 => {
                    let clip_id = word_i16(ops, 0);
                    let flags = ops[5];
                    let can_emit_events =
                        matches!(flags & 0x11, 0x00 | 0x11) && (flags & 0x04 != 0 || clip_id != 0);
                    if flags & 0x20 != 0 {
                        check_operand(25, i16::from(ops[2]), true, violations);
                    } else if can_emit_events {
                        // Even with `events == 0`, foreground clips can emit event >= 100
                        // and call `store_event` into `param[0]` (`animate.rs:185-198`).
                        check_operand(9, 0, true, violations);
                    }
                    if flags & 0x04 != 0 {
                        check_operand(9, clip_id, false, violations);
                    }
                }
                45 => {
                    let slot = usize::from(u16::from_le_bytes([ops[0], ops[1]]));
                    let source = u16::from_le_bytes([ops[2], ops[3]]);
                    if source == 0 {
                        check_operand(9, word_i16(ops, 0), false, violations);
                    } else if source == 1 && slot >= total_slots {
                        add_err(
                            violations,
                            SafetyCategory::OutOfBoundsSlot,
                            idx,
                            format!("RouteTo slot {slot} out of bounds (SLOT count {total_slots})"),
                        );
                    } else if source > 2 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("RouteTo invalid source {source} (must be 0..=2)"),
                        );
                    }
                }
                46 => {
                    let slot = usize::from(u16::from_le_bytes([ops[0], ops[1]]));
                    let mode = u16::from_le_bytes([ops[2], ops[3]]);
                    if mode == 0 {
                        check_operand(9, word_i16(ops, 0), false, violations);
                    } else if mode == 3 && slot >= total_slots {
                        add_err(
                            violations,
                            SafetyCategory::OutOfBoundsSlot,
                            idx,
                            format!("Snap slot {slot} out of bounds (SLOT count {total_slots})"),
                        );
                    } else if mode > 4 {
                        add_err(
                            violations,
                            SafetyCategory::PotentialTrap,
                            idx,
                            format!("Snap invalid mode {mode} (must be 0..=4)"),
                        );
                    }
                }
                47 if word_i16(ops, 0) == 1 => {
                    check_operand(9, word_i16(ops, 4), false, violations);
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
                    if pass_temps {
                        if callee_args > 8 {
                            add_err(
                                violations,
                                SafetyCategory::ArityMismatch,
                                idx,
                                format!(
                                    "calls private BHAV #{} ('{callee_name}') requiring {callee_args} args via 8-temp convention",
                                    ins.opcode
                                ),
                            );
                        }
                    } else if callee_args > 4 {
                        // Extended inline calls read `callee_args * 2` bytes across this
                        // instruction's 8 operand bytes plus 12 bytes per following record
                        // (`sims-vm/src/interpreter.rs:1548`).
                        let avail_bytes = 8usize.saturating_add((n - 1 - idx).saturating_mul(12));
                        if avail_bytes < callee_args * 2 {
                            add_err(
                                violations,
                                SafetyCategory::ArityMismatch,
                                idx,
                                format!(
                                    "calls private BHAV #{} ('{callee_name}') requiring {callee_args} inline args ({} bytes), but only {avail_bytes} bytes remain in BHAV",
                                    ins.opcode,
                                    callee_args * 2
                                ),
                            );
                        }
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
    /// Freshly created object (`CreateObject`) or proven-distinct object (`x != InitialTarget`).
    OtherObject,
}

impl SymWord {
    /// Returns `true` whenever this symbolic object ID could alias `InitialTarget` at runtime.
    /// Any positive literal ID (`Const(v)` with `v > 0`) could equal the target's runtime ID;
    /// `Const(0)` / negative IDs and `OtherObject` can never alias `InitialTarget`.
    fn may_alias_initial_target(self) -> bool {
        match self {
            Self::InitialTarget | Self::Unknown => true,
            Self::Const(v) => v > 0,
            Self::OtherObject => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SymFrame {
    tree_id: u16,
    pc: u8,
    stack_obj: SymWord,
    params: Vec<SymWord>,
    locals: Vec<SymWord>,
    /// For `Run Tree By Name` mode 1 (which runs on the target's thread with target's temps,
    /// `named_tree.rs:102`), saves the caller's temps to restore/join on return.
    saved_caller_temps: Option<[SymWord; 8]>,
    /// True when executing inside a synchronous `Run Tree By Name` (mode 0 or 1), where
    /// yielding primitives trap (`named_tree.rs:33`).
    synchronous: bool,
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

/// Summary of lock operations discovered across reachable paths of an interaction.
#[derive(Default)]
struct TreeLockOps {
    acquires: BTreeSet<LockKey>,
    releases: BTreeSet<LockKey>,
    /// Attributes targeted by an arbitrary/unknown write (e.g. `Relationship` read, `Random`,
    /// `SetToNext`), which can set any scalar or bitmask lock on that attribute.
    written_attrs: BTreeSet<i16>,
    /// True if a reachable scope-22 write has an unknown index (can alias any tracked mutex).
    wildcard_attr_write: bool,
    /// True if a reachable `Run Tree By Name` (opcode 28) could not be resolved to a local tree.
    has_dynamic_call: bool,
    /// True if call-depth recursion cap was hit before completing discovery.
    truncated: bool,
}

/// Resolves a local `Run Tree By Name` (opcode 28) instruction against `model`'s `STR# 303`
/// and private `BHAV`s when `table == 303`, local table (`ops[2] & 1 == 0`), and `mode <= 2`.
enum NamedTreeResolution {
    /// Entry is missing or empty in local `STR# 303` -> VM takes `on_false` without calling.
    NoMatchFalse,
    /// Resolved to a private `BHAV` in this object (`4096..=8191`).
    Private(u16),
    /// Dynamic/external/global/invalid target that cannot be resolved from this object alone.
    Unresolved(String),
}

fn resolve_named_tree_local(model: &ObjectModel, ops: &[u8; 8]) -> NamedTreeResolution {
    let table = u16::from_le_bytes([ops[0], ops[1]]);
    if table != 303 {
        return NamedTreeResolution::Unresolved(format!(
            "Run Tree By Name uses non-303 string table {table}"
        ));
    }
    if ops[2] & 1 != 0 {
        return NamedTreeResolution::Unresolved(
            "Run Tree By Name reads global STR# 303".to_owned(),
        );
    }
    if ops[5] > 2 {
        return NamedTreeResolution::Unresolved(format!(
            "Run Tree By Name uses invalid mode {}",
            ops[5]
        ));
    }
    let Some(idx) = usize::from(ops[4]).checked_sub(1) else {
        return NamedTreeResolution::NoMatchFalse;
    };
    let Some(name) = model
        .string_entries
        .get(&303)
        .and_then(|t| t.get(idx))
        .filter(|s| !s.is_empty())
    else {
        return NamedTreeResolution::NoMatchFalse;
    };
    // Match `sims-vm/src/program.rs:1236` (`Program::named_tree`):
    // Exact name lookup checks the local library FIRST before semi-global/global, and traps
    // on duplicates. Only a UNIQUE exact match in this object is guaranteed not to be shadowed
    // by a semi-global or global exact match! A case-insensitive-only match may be shadowed.
    let mut exact_matches = model
        .behaviors
        .iter()
        .filter(|(_, (label, _))| label.as_str() == name.as_str());
    if let Some((&callee_id, _)) = exact_matches.next() {
        if exact_matches.next().is_some() {
            return NamedTreeResolution::Unresolved(format!(
                "duplicate local trees named '{name}' trap in Program::named_tree"
            ));
        }
        return NamedTreeResolution::Private(callee_id);
    }
    NamedTreeResolution::Unresolved(format!(
        "Run Tree By Name '{name}' has no unique exact local match and may resolve to a semi-global or global tree"
    ))
}

/// Collects parameter indices that may be mutated anywhere in the reachable instructions of `bhav`
/// so lock discovery never uses stale caller-passed `initial_params` values.
fn collect_mutated_params(bhav: &Behavior, reachable: &[bool]) -> BTreeSet<i16> {
    let mut mutated = BTreeSet::new();
    for (idx, ins) in bhav.instructions.iter().enumerate() {
        if !reachable[idx] {
            continue;
        }
        let ops = &ins.operands;
        match ins.opcode {
            0 | 17 => {
                mutated.insert(word_i16(ops, 0));
            }
            2 => {
                let op = ops[5];
                if ops[6] == 9 && matches!(op, 3..=7 | 9..=13 | 17..=20) {
                    mutated.insert(word_i16(ops, 0));
                }
            }
            8 | 12 => {
                if u16::from_le_bytes([ops[2], ops[3]]) == 9 {
                    mutated.insert(word_i16(ops, 0));
                }
            }
            24 if ops[0] == 0 => {
                mutated.insert(i16::from(ops[2] as i8));
            }
            26 if ops[2] & 4 == 0 && ops[4] == 9 => {
                mutated.insert(word_i16(ops, 6));
            }
            31 if ops[4] & 0x80 != 0 && ops[5] == 9 => {
                mutated.insert(i16::from(ops[7]));
            }
            44 if ops[5] & 0x20 == 0 => {
                mutated.insert(0);
            }
            _ => {}
        }
    }
    mutated
}

/// Collects scalar/bitmask lock acquires and releases reachable from `root_tree`
/// (inlining private BHAV callees with constant param propagation).
fn collect_tree_lock_ops(
    model: &ObjectModel,
    root_tree: u16,
    initial_params: &[SymWord],
    depth: usize,
    out: &mut TreeLockOps,
) {
    if depth > 6 {
        out.truncated = true;
        return;
    }
    let Some((_, bhav)) = model.behaviors.get(&root_tree) else {
        return;
    };
    let reachable = if bhav.kind == 4 {
        vec![true; bhav.instructions.len()]
    } else {
        reachable_nodes(&bhav.instructions)
    };
    let mutated_params = collect_mutated_params(bhav, &reachable);
    let resolve_val = |scope: u8, idx: i16| -> Option<i16> {
        match scope {
            7 => Some(idx),
            9 => {
                if mutated_params.contains(&idx) {
                    return None;
                }
                match initial_params.get(idx as usize).copied() {
                    Some(SymWord::Const(v)) => Some(v),
                    _ => None,
                }
            }
            26 => lookup_bcon(model, idx),
            _ => None,
        }
    };

    let record_unknown_dst_write = |scope: u8, idx: i16, out: &mut TreeLockOps| {
        if scope == 1 {
            out.acquires.insert(LockKey::Scalar(idx));
            out.written_attrs.insert(idx);
        } else if scope == 22 {
            if let Some(attr_idx) = resolve_val(9, idx) {
                out.acquires.insert(LockKey::Scalar(attr_idx));
                out.written_attrs.insert(attr_idx);
            } else {
                out.wildcard_attr_write = true;
            }
        }
    };

    for (idx, ins) in bhav.instructions.iter().enumerate() {
        if !reachable[idx] {
            continue;
        }
        if ins.opcode == 2 {
            let lhs_idx = word_i16(&ins.operands, 0);
            let rhs_idx = word_i16(&ins.operands, 2);
            let op = ins.operands[5];
            let lhs_scope = ins.operands[6];
            let rhs_scope = ins.operands[7];
            if lhs_scope == 1 {
                match op {
                    5 => {
                        if let Some(v) = resolve_val(rhs_scope, rhs_idx) {
                            if v == 0 {
                                out.releases.insert(LockKey::Scalar(lhs_idx));
                            } else {
                                out.acquires.insert(LockKey::Scalar(lhs_idx));
                            }
                        } else {
                            // Unknown/symbolic write to stack.attr[lhs_idx] can acquire it.
                            out.acquires.insert(LockKey::Scalar(lhs_idx));
                            out.written_attrs.insert(lhs_idx);
                        }
                    }
                    9 => {
                        if let Some(b) = resolve_val(rhs_scope, rhs_idx)
                            && (1..=16).contains(&b)
                        {
                            out.acquires.insert(LockKey::Bit(lhs_idx, b as u8));
                        } else {
                            out.acquires.insert(LockKey::Scalar(lhs_idx));
                            out.written_attrs.insert(lhs_idx);
                        }
                    }
                    10 => {
                        if let Some(b) = resolve_val(rhs_scope, rhs_idx)
                            && (1..=16).contains(&b)
                        {
                            out.releases.insert(LockKey::Bit(lhs_idx, b as u8));
                        }
                    }
                    3 | 4 | 6 | 7 | 12 | 13 | 18 | 19 | 20 => {
                        out.acquires.insert(LockKey::Scalar(lhs_idx));
                        out.written_attrs.insert(lhs_idx);
                    }
                    _ => {}
                }
            } else if lhs_scope == 22 && matches!(op, 3..=7 | 9..=13 | 17..=20) {
                // Scope 22 writes `stack.attr[param[lhs_idx]]`.
                if let Some(attr_idx) = resolve_val(9, lhs_idx) {
                    if op == 5 && resolve_val(rhs_scope, rhs_idx) == Some(0) {
                        out.releases.insert(LockKey::Scalar(attr_idx));
                    } else {
                        out.acquires.insert(LockKey::Scalar(attr_idx));
                        out.written_attrs.insert(attr_idx);
                    }
                } else {
                    out.wildcard_attr_write = true;
                }
            }
        } else if matches!(ins.opcode, 8 | 12) {
            let dst_scope = u16::from_le_bytes([ins.operands[2], ins.operands[3]]);
            if dst_scope <= 255 {
                record_unknown_dst_write(dst_scope as u8, word_i16(&ins.operands, 0), out);
            }
        } else if ins.opcode == 26 && ins.operands[2] & 4 == 0 {
            // Relationship read writes an arbitrary value into `(ops[4], word_i16(ops, 6))`
            // on the true branch (`relationship.rs:178-195`).
            record_unknown_dst_write(ins.operands[4], word_i16(&ins.operands, 6), out);
        } else if ins.opcode == 31 && ins.operands[4] & 0x80 != 0 {
            record_unknown_dst_write(ins.operands[5], i16::from(ins.operands[7]), out);
        } else if matches!(ins.opcode, 3 | 30 | 47) {
            // FindBestInteraction (3, TTAB check trees), GosubFoundAction (30), and
            // Reach (47, CT - Grab/Put) invoke dynamic trees.
            out.has_dynamic_call = true;
        } else if ins.opcode == 28 {
            match resolve_named_tree_local(model, &ins.operands) {
                NamedTreeResolution::NoMatchFalse => {}
                NamedTreeResolution::Private(callee_id) => {
                    let call_params = vec![SymWord::Unknown; 8];
                    collect_tree_lock_ops(model, callee_id, &call_params, depth + 1, out);
                }
                NamedTreeResolution::Unresolved(_) => {
                    out.has_dynamic_call = true;
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
            collect_tree_lock_ops(model, ins.opcode, &call_params, depth + 1, out);
        }
    }
}

enum LockProofFailure {
    Leak {
        lock: LockKey,
        exit_label: &'static str,
        trace: Vec<String>,
    },
    Inconclusive {
        reason: String,
        trace: Vec<String>,
    },
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

    let mut per_inter_ops: Vec<(TreeLockOps, BTreeSet<LockKey>)> =
        Vec::with_capacity(interactions.len());
    for &(action, test) in &interactions {
        let mut ops = TreeLockOps::default();
        collect_tree_lock_ops(model, action, &[], 0, &mut ops);
        let guard_mutexes = model
            .behaviors
            .get(&test)
            .map(|(_, b)| collect_guard_mutexes(b))
            .unwrap_or_default();
        per_inter_ops.push((ops, guard_mutexes));
    }

    // Identify attributes that act as scoped mutexes anywhere on this object (acquired AND released
    // within at least one interaction). A persistent multi-interaction toggle (like TV power or
    // lamp on/off) has separate TurnOn (only sets) and TurnOff (only clears) interactions.
    let mut scoped_mutexes: BTreeSet<LockKey> = BTreeSet::new();
    let mut all_guarded_mutexes: BTreeSet<LockKey> = BTreeSet::new();
    for (ops, guards) in &per_inter_ops {
        all_guarded_mutexes.extend(guards.iter().copied());
        for &lock in &ops.acquires {
            if ops.releases.contains(&lock) {
                scoped_mutexes.insert(lock);
            }
        }
    }

    let mut locks_verified = 0usize;
    let mut checked_actions = BTreeSet::new();

    for (idx, &(action, _test)) in interactions.iter().enumerate() {
        if !checked_actions.insert(action) {
            continue;
        }
        let (ops, guard_mutexes) = &per_inter_ops[idx];
        let (tree_name, _) = &model.behaviors[&action];
        if ops.truncated {
            violations.push(SafetyViolation {
                object: model.name.clone(),
                tree_id: Some(action),
                tree_name: tree_name.clone(),
                category: SafetyCategory::InconclusiveProof,
                node: None,
                message: "lock discovery exceeded private call-depth limit (6)".to_owned(),
                path: Vec::new(),
            });
            continue;
        }

        let mut tracked_locks = BTreeSet::new();
        for &lock in &ops.acquires {
            if ops.releases.contains(&lock) || scoped_mutexes.contains(&lock) {
                // Acquired and released in this interaction (or a known scoped mutex on this object).
                tracked_locks.insert(lock);
            } else if guard_mutexes.contains(&lock) {
                // Guarded as mutex and acquired here; only exempt if another interaction
                // acts strictly as a dedicated release toggle (releases without acquiring).
                let is_persistent_toggle =
                    per_inter_ops.iter().enumerate().any(|(j, (other_ops, _))| {
                        j != idx
                            && other_ops.releases.contains(&lock)
                            && !other_ops.acquires.contains(&lock)
                    });
                if !is_persistent_toggle {
                    tracked_locks.insert(lock);
                }
            }
        }
        // Any arbitrary write to `stack.attr[a]` (e.g. Relationship read, Random, SetToNext)
        // can set any scalar or bitmask mutex living on attribute `a`.
        if !ops.written_attrs.is_empty() {
            for &lock in scoped_mutexes.iter().chain(all_guarded_mutexes.iter()) {
                let attr = match lock {
                    LockKey::Scalar(a) | LockKey::Bit(a, _) => a,
                };
                if ops.written_attrs.contains(&attr) {
                    tracked_locks.insert(lock);
                }
            }
        }
        // If this interaction writes through scope 22 with an unknown index or invokes an
        // unresolved `Run Tree By Name`, it can alias any known scoped or guarded mutex.
        if ops.wildcard_attr_write || ops.has_dynamic_call {
            tracked_locks.extend(scoped_mutexes.iter().copied());
            tracked_locks.extend(all_guarded_mutexes.iter().copied());
        }

        if tracked_locks.is_empty() {
            if ops.has_dynamic_call {
                violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: Some(action),
                    tree_name: tree_name.clone(),
                    category: SafetyCategory::InconclusiveProof,
                    node: None,
                    message: "reachable dynamic tree call prevents complete lock safety proof"
                        .to_owned(),
                    path: Vec::new(),
                });
            }
            continue;
        }

        match prove_interaction_lock_safety(model, action, &tracked_locks) {
            Ok(verified_count) => {
                locks_verified += verified_count;
            }
            Err(LockProofFailure::Leak {
                lock: leaked,
                exit_label,
                trace,
            }) => {
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
            Err(LockProofFailure::Inconclusive { reason, trace }) => {
                violations.push(SafetyViolation {
                    object: model.name.clone(),
                    tree_id: Some(action),
                    tree_name: tree_name.clone(),
                    category: SafetyCategory::InconclusiveProof,
                    node: None,
                    message: format!("lock safety proof incomplete: {reason}"),
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
        9 => frame
            .params
            .get(idx as usize)
            .copied()
            .unwrap_or(SymWord::Unknown),
        10 => frame.stack_obj,
        11 => match state.temps.get(idx as usize).copied() {
            Some(SymWord::Const(t)) => state
                .temps
                .get(t as usize)
                .copied()
                .unwrap_or(SymWord::Unknown),
            _ => SymWord::Unknown,
        },
        22 => {
            // Scope 22 is `stack.attr[param[idx]]` (`sims-vm/src/memory.rs:461`).
            if frame.stack_obj == SymWord::InitialTarget
                && let Some(SymWord::Const(attr_idx)) = frame.params.get(idx as usize).copied()
            {
                match state.target_attrs.get(&attr_idx) {
                    Some(LockVal::Concrete(v)) => SymWord::Const(*v),
                    _ => SymWord::Unknown,
                }
            } else {
                SymWord::Unknown
            }
        }
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
        9 => {
            if let Some(slot) = frame.params.get_mut(idx as usize) {
                *slot = val;
            }
        }
        10 => {
            frame.stack_obj = val;
        }
        11 => {
            // Scope 11 writes `temp[temp[idx]]` (`sims-vm/src/memory.rs:446`).
            match state.temps.get(idx as usize).copied() {
                Some(SymWord::Const(t)) => {
                    if let Some(slot) = state.temps.get_mut(t as usize) {
                        *slot = val;
                    }
                }
                _ => {
                    state.temps.fill(SymWord::Unknown);
                }
            }
        }
        13 => {
            // Scope 13 writes `stack_obj.temp[idx]`; if `stack_obj == me`, this aliases
            // the actor's `temp[idx]` (`sims-vm/src/memory.rs:447`), so conservatively clear
            // any known constant in `state.temps[idx]`.
            if let Some(slot) = state.temps.get_mut(idx as usize) {
                *slot = SymWord::Unknown;
            }
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
) -> std::result::Result<usize, LockProofFailure> {
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
            // Frame locals can inherit nonzero values from prior frames (`interpreter.rs:1210`).
            locals: vec![SymWord::Unknown; bhav_local_limit(root_bhav)],
            saved_caller_temps: None,
            synchronous: false,
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
            return Err(LockProofFailure::Inconclusive {
                reason: format!("exceeded symbolic state limit ({MAX_EXPLORED_STATES})"),
                trace,
            });
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
         -> std::result::Result<(), LockProofFailure> {
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
                        let popped = next_state.frames.pop();
                        // Returning from `Run Tree By Name` mode 1 (which runs on the target's
                        // thread with target's temps, `named_tree.rs:102`): join saved caller
                        // temps with callee temps so distinct caller/target never falsely inherits
                        // callee temp writes, while same-thread aliasing is also preserved.
                        if let Some(saved) = popped.and_then(|f| f.saved_caller_temps) {
                            for (slot, orig) in next_state.temps.iter_mut().zip(saved) {
                                if *slot != orig {
                                    *slot = SymWord::Unknown;
                                }
                            }
                        }
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
                                    return Err(LockProofFailure::Leak {
                                        lock,
                                        exit_label,
                                        trace: next_trace,
                                    });
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
        let mark_attr_unknown_acquired = |next: &mut LockState, attr_idx: i16| {
            next.target_attrs.insert(attr_idx, LockVal::Unknown);
            for &lock in tracked_locks {
                match lock {
                    LockKey::Scalar(a) | LockKey::Bit(a, _) if a == attr_idx => {
                        next.acquired.insert(lock);
                    }
                    _ => {}
                }
            }
        };
        let mark_all_tracked_unknown_acquired = |next: &mut LockState| {
            let attrs: Vec<i16> = tracked_locks
                .iter()
                .map(|k| match *k {
                    LockKey::Scalar(a) | LockKey::Bit(a, _) => a,
                })
                .collect();
            for a in attrs {
                mark_attr_unknown_acquired(next, a);
            }
        };
        let apply_unknown_write = |next: &mut LockState, scope: u8, idx: i16| {
            let cur_stack_obj = next.frames.last().expect("frame").stack_obj;
            let may_alias_initial = cur_stack_obj.may_alias_initial_target();
            if scope == 1 {
                if may_alias_initial {
                    mark_attr_unknown_acquired(next, idx);
                }
            } else if scope == 22 {
                if may_alias_initial {
                    match next
                        .frames
                        .last()
                        .expect("frame")
                        .params
                        .get(idx as usize)
                        .copied()
                    {
                        Some(SymWord::Const(a)) => mark_attr_unknown_acquired(next, a),
                        _ => mark_all_tracked_unknown_acquired(next),
                    }
                }
            } else {
                write_sym_operand(next, scope, idx, SymWord::Unknown);
            }
        };

        match ins.opcode {
            2 => {
                let lhs_idx = word_i16(ops, 0);
                let rhs_idx = word_i16(ops, 2);
                let op = ops[5];
                let lhs_scope = ops[6];
                let rhs_scope = ops[7];
                let lhs_val = eval_sym_operand(model, &state, lhs_scope, lhs_idx);
                let rhs_val = eval_sym_operand(model, &state, rhs_scope, rhs_idx);

                // Model potential trap edges (div/mod by unproved/zero, invalid flag bit, negative sqrt)
                // so a trap while holding a lock is caught as a Leak on error exit!
                let may_trap = match op {
                    7 | 12 => !matches!(rhs_val, SymWord::Const(r) if r != 0),
                    8..=10 => !matches!(rhs_val, SymWord::Const(b) if (0..=16).contains(&b)),
                    20 => !matches!(rhs_val, SymWord::Const(v) if v >= 0),
                    _ => !is_valid_memory_scope(lhs_scope) || !is_valid_memory_scope(rhs_scope),
                };
                if may_trap {
                    advance(
                        state.clone(),
                        TARGET_ERROR,
                        format!("tree#{cur_tree}@n{cur_pc}:expr(op={op})->trap"),
                        &mut visited,
                        &mut queue,
                    )?;
                }

                let is_assign = matches!(op, 3..=7 | 9 | 10 | 12 | 13 | 18 | 19 | 20);
                if is_assign {
                    let mut next = state.clone();
                    let cur_stack_obj = next.frames.last().expect("frame").stack_obj;
                    let on_initial_target = cur_stack_obj == SymWord::InitialTarget;
                    let may_alias_initial = cur_stack_obj.may_alias_initial_target();

                    // Resolve direct `stack.attr[lhs_idx]` (scope 1) or indirect `stack.attr[param[lhs_idx]]` (scope 22).
                    let resolved_target_attr = if may_alias_initial && lhs_scope == 1 {
                        Some(Some(lhs_idx))
                    } else if may_alias_initial && lhs_scope == 22 {
                        match next
                            .frames
                            .last()
                            .expect("frame")
                            .params
                            .get(lhs_idx as usize)
                            .copied()
                        {
                            Some(SymWord::Const(a)) => Some(Some(a)),
                            _ => Some(None), // unknown target attribute index -> aliases all tracked attrs!
                        }
                    } else {
                        None
                    };

                    if let Some(maybe_attr) = resolved_target_attr {
                        if on_initial_target {
                            if let Some(attr_idx) = maybe_attr {
                                match op {
                                    5 => {
                                        if let SymWord::Const(v) = rhs_val {
                                            next.target_attrs
                                                .insert(attr_idx, LockVal::Concrete(v));
                                            if v != 0
                                                && tracked_locks
                                                    .contains(&LockKey::Scalar(attr_idx))
                                            {
                                                next.acquired.insert(LockKey::Scalar(attr_idx));
                                            }
                                        } else {
                                            mark_attr_unknown_acquired(&mut next, attr_idx);
                                        }
                                    }
                                    9 => {
                                        if let SymWord::Const(b) = rhs_val
                                            && (1..=16).contains(&b)
                                        {
                                            let b = b as u8;
                                            let cur = next
                                                .target_attrs
                                                .get(&attr_idx)
                                                .copied()
                                                .unwrap_or(LockVal::Unknown);
                                            next.target_attrs
                                                .insert(attr_idx, cur.with_flag_set(b));
                                            if tracked_locks.contains(&LockKey::Bit(attr_idx, b)) {
                                                next.acquired.insert(LockKey::Bit(attr_idx, b));
                                            }
                                        } else {
                                            mark_attr_unknown_acquired(&mut next, attr_idx);
                                        }
                                    }
                                    10 => {
                                        if let SymWord::Const(b) = rhs_val
                                            && (1..=16).contains(&b)
                                        {
                                            let b = b as u8;
                                            let cur = next
                                                .target_attrs
                                                .get(&attr_idx)
                                                .copied()
                                                .unwrap_or(LockVal::Unknown);
                                            next.target_attrs
                                                .insert(attr_idx, cur.with_flag_clear(b));
                                        } else {
                                            mark_attr_unknown_acquired(&mut next, attr_idx);
                                        }
                                    }
                                    _ => {
                                        mark_attr_unknown_acquired(&mut next, attr_idx);
                                    }
                                }
                            } else {
                                // Indirect write to unknown `stack.attr[?]` can alias any tracked attribute.
                                mark_all_tracked_unknown_acquired(&mut next);
                            }
                        } else {
                            // `stack_obj` is `Unknown` and may alias `InitialTarget`:
                            // never credit a release (might be a different object), but any write
                            // that can set a nonzero value/bit conservatively marks acquired!
                            let definitely_clearing =
                                (op == 5 && rhs_val == SymWord::Const(0)) || op == 10;
                            if !definitely_clearing {
                                if let Some(attr_idx) = maybe_attr {
                                    mark_attr_unknown_acquired(&mut next, attr_idx);
                                } else {
                                    mark_all_tracked_unknown_acquired(&mut next);
                                }
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
                        if matches!(lhs_scope, 1 | 22) {
                            apply_unknown_write(&mut next, lhs_scope, lhs_idx);
                        } else {
                            write_sym_operand(&mut next, lhs_scope, lhs_idx, updated);
                        }
                    }
                    let known_truth = match (pre_lhs, rhs_val) {
                        (SymWord::Const(l), SymWord::Const(r)) => eval_expr_compare(l, op, r),
                        (SymWord::InitialTarget, SymWord::InitialTarget) if op == 2 => Some(true),
                        (SymWord::InitialTarget, SymWord::InitialTarget) if op == 16 => Some(false),
                        (SymWord::InitialTarget, SymWord::OtherObject)
                        | (SymWord::OtherObject, SymWord::InitialTarget)
                            if op == 2 =>
                        {
                            Some(false)
                        }
                        (SymWord::InitialTarget, SymWord::OtherObject)
                        | (SymWord::OtherObject, SymWord::InitialTarget)
                            if op == 16 =>
                        {
                            Some(true)
                        }
                        _ => None,
                    };
                    // Path-sensitive refinement for `x == InitialTarget` / `x != InitialTarget`
                    // where `x` is a tracked variable (temp, param, stack_obj, or local):
                    // e.g. multi-tile `for part in self.parts()` checks `if part == __self_obj { break; }`
                    // so the fallthrough branch proves `part` is `OtherObject` (distinct from `InitialTarget`).
                    let refine_target = if matches!(op, 2 | 16) {
                        if rhs_val == SymWord::InitialTarget
                            && matches!(lhs_scope, 8 | 9 | 10 | 25)
                            && pre_lhs == SymWord::Unknown
                        {
                            Some((lhs_scope, lhs_idx))
                        } else if pre_lhs == SymWord::InitialTarget
                            && matches!(rhs_scope, 8 | 9 | 10 | 25)
                            && rhs_val == SymWord::Unknown
                        {
                            Some((rhs_scope, rhs_idx))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if known_truth != Some(false) {
                        let mut true_state = next.clone();
                        if let Some((r_scope, r_idx)) = refine_target {
                            let refined = if op == 2 {
                                SymWord::InitialTarget
                            } else {
                                SymWord::OtherObject
                            };
                            write_sym_operand(&mut true_state, r_scope, r_idx, refined);
                        }
                        advance(
                            true_state,
                            ins.on_true,
                            format!("tree#{cur_tree}@n{cur_pc}:cond(op={op})->true"),
                            &mut visited,
                            &mut queue,
                        )?;
                    }
                    if known_truth != Some(true) {
                        let mut false_state = next;
                        if let Some((r_scope, r_idx)) = refine_target {
                            let refined = if op == 2 {
                                SymWord::OtherObject
                            } else {
                                SymWord::InitialTarget
                            };
                            write_sym_operand(&mut false_state, r_scope, r_idx, refined);
                        }
                        advance(
                            false_state,
                            ins.on_false,
                            format!("tree#{cur_tree}@n{cur_pc}:cond(op={op})->false"),
                            &mut visited,
                            &mut queue,
                        )?;
                    }
                }
            }
            28 => {
                // Run Tree By Name (`named_tree.rs:57`): resolve private callee if target is
                // `InitialTarget` and name lives in local `STR# 303`; otherwise `Inconclusive`.
                let cur_stack_obj = state.frames.last().expect("frame").stack_obj;
                if cur_stack_obj != SymWord::InitialTarget {
                    return Err(LockProofFailure::Inconclusive {
                        reason: format!(
                            "reachable Run Tree By Name at tree#{cur_tree}@n{cur_pc} targets non-initial stack object"
                        ),
                        trace,
                    });
                }
                match resolve_named_tree_local(model, ops) {
                    NamedTreeResolution::NoMatchFalse => {
                        advance(
                            state,
                            ins.on_false,
                            format!("tree#{cur_tree}@n{cur_pc}:run_tree_by_name(nomatch)->false"),
                            &mut visited,
                            &mut queue,
                        )?;
                    }
                    NamedTreeResolution::Private(callee_id) => {
                        if state.frames.len() >= 8 {
                            return Err(LockProofFailure::Inconclusive {
                                reason: format!(
                                    "exceeded call depth limit (8) calling named BHAV #{callee_id}"
                                ),
                                trace,
                            });
                        }
                        let callee = &model.behaviors[&callee_id].1;
                        if callee.instructions.is_empty() {
                            advance(
                                state,
                                ins.on_true,
                                format!(
                                    "tree#{cur_tree}@n{cur_pc}:run_tree_by_name#{callee_id}(empty)->true"
                                ),
                                &mut visited,
                                &mut queue,
                            )?;
                        } else {
                            // Mode 1 runs on target's thread with target's own temps (unknown),
                            // preserving caller's temps on return; modes 0 & 1 are synchronous.
                            let is_mode_1 = ops[5] == 1;
                            let is_sync = ops[5] == 0
                                || is_mode_1
                                || state.frames.last().expect("frame").synchronous;
                            let mut params = if is_mode_1 {
                                vec![SymWord::Unknown; 8]
                            } else {
                                state.temps.to_vec()
                            };
                            let needed = bhav_param_limit(callee).max(4);
                            if params.len() < needed {
                                params.resize(needed, SymWord::Const(0));
                            }
                            let entry_pcs: Vec<u8> = if callee.kind == 4 {
                                (0..callee.instructions.len())
                                    .filter_map(|p| u8::try_from(p).ok())
                                    .collect()
                            } else {
                                vec![0]
                            };
                            for entry_pc in entry_pcs {
                                let mut next = state.clone();
                                let saved_caller_temps = if is_mode_1 {
                                    let saved = next.temps;
                                    next.temps = [SymWord::Unknown; 8];
                                    Some(saved)
                                } else {
                                    None
                                };
                                next.frames.push(SymFrame {
                                    tree_id: callee_id,
                                    pc: entry_pc,
                                    stack_obj: cur_stack_obj,
                                    params: params.clone(),
                                    locals: vec![SymWord::Unknown; bhav_local_limit(callee)],
                                    saved_caller_temps,
                                    synchronous: is_sync,
                                });
                                if visited.insert(next.clone()) {
                                    let mut next_trace = trace.clone();
                                    next_trace.push(format!(
                                        "tree#{cur_tree}@n{cur_pc}:run_tree_by_name#{callee_id}@{entry_pc}"
                                    ));
                                    queue.push_back((next, next_trace));
                                }
                            }
                        }
                    }
                    NamedTreeResolution::Unresolved(why) => {
                        return Err(LockProofFailure::Inconclusive {
                            reason: format!(
                                "reachable Run Tree By Name at tree#{cur_tree}@n{cur_pc}: {why}"
                            ),
                            trace,
                        });
                    }
                }
            }
            3 | 30 | 47 => {
                let name = match ins.opcode {
                    3 => "Find Best Interaction",
                    30 => "Gosub Found Action",
                    _ => "Reach",
                };
                return Err(LockProofFailure::Inconclusive {
                    reason: format!(
                        "reachable {name} at tree#{cur_tree}@n{cur_pc} can invoke dynamic trees"
                    ),
                    trace,
                });
            }
            opcode if (4096..=8191).contains(&opcode) && model.behaviors.contains_key(&opcode) => {
                if state.frames.len() >= 8 {
                    return Err(LockProofFailure::Inconclusive {
                        reason: format!(
                            "exceeded call depth limit (8) calling private BHAV #{opcode}"
                        ),
                        trace,
                    });
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
                        let caller_frame = state.frames.last().expect("frame");
                        let caller_stack_obj = caller_frame.stack_obj;
                        let caller_sync = caller_frame.synchronous;
                        let needed = bhav_param_limit(callee).max(4);
                        let mut params = if *ops == [0xFF; 8] {
                            state.temps.to_vec()
                        } else if needed <= 4 {
                            (0..4)
                                .map(|i| SymWord::Const(word_i16(ops, i * 2)))
                                .collect()
                        } else {
                            let mut raw = Vec::with_capacity(needed * 2 + 12);
                            raw.extend_from_slice(ops);
                            for next_ins in bhav.instructions.iter().skip(usize::from(cur_pc) + 1) {
                                if raw.len() >= needed * 2 {
                                    break;
                                }
                                raw.extend_from_slice(&next_ins.opcode.to_le_bytes());
                                raw.push(next_ins.on_true);
                                raw.push(next_ins.on_false);
                                raw.extend_from_slice(&next_ins.operands);
                            }
                            (0..needed)
                                .map(|i| {
                                    raw.get(i * 2..i * 2 + 2)
                                        .map(|b| SymWord::Const(i16::from_le_bytes([b[0], b[1]])))
                                        .unwrap_or(SymWord::Unknown)
                                })
                                .collect()
                        };
                        if params.len() < needed {
                            params.resize(needed, SymWord::Const(0));
                        }
                        // Kind-4 indexed trees can enter at any instruction index selected by
                        // param[0] (`interpreter.rs:1609`), so explore every valid entry PC.
                        let entry_pcs: Vec<u8> = if callee.kind == 4 {
                            (0..callee.instructions.len())
                                .filter_map(|p| u8::try_from(p).ok())
                                .collect()
                        } else {
                            vec![0]
                        };
                        for entry_pc in entry_pcs {
                            let mut next = state.clone();
                            next.frames.push(SymFrame {
                                tree_id: opcode,
                                pc: entry_pc,
                                stack_obj: caller_stack_obj,
                                params: params.clone(),
                                // Frame locals can inherit nonzero values from prior frames.
                                locals: vec![SymWord::Unknown; bhav_local_limit(callee)],
                                saved_caller_temps: None,
                                synchronous: caller_sync,
                            });
                            if visited.insert(next.clone()) {
                                let mut next_trace = trace.clone();
                                next_trace.push(format!(
                                    "tree#{cur_tree}@n{cur_pc}:enter#{opcode}@{entry_pc}"
                                ));
                                queue.push_back((next, next_trace));
                            }
                        }
                    }
                }
            }
            _ => {
                // Model potential trap exits so a fault between acquiring and releasing a lock
                // is caught as a Leak even when `is_always_true_instruction` holds (no false branch).
                if primitive_may_trap_in_lock_prover(model, &state, ins) {
                    advance(
                        state.clone(),
                        TARGET_ERROR,
                        format!("tree#{cur_tree}@n{cur_pc}:prim({})->trap", ins.opcode),
                        &mut visited,
                        &mut queue,
                    )?;
                }

                // World-interacting primitive: compute branch-specific mutations cleanly so
                // false branches never inherit true-only stack_obj/destination mutations.
                let mut next_true = state.clone();
                let mut next_false = state.clone();
                match ins.opcode {
                    0 => {
                        // Sleep zeroes its param slot.
                        let p = word_i16(ops, 0);
                        write_sym_operand(&mut next_true, 9, p, SymWord::Const(0));
                        write_sym_operand(&mut next_false, 9, p, SymWord::Const(0));
                    }
                    19 => {
                        // MakeNewCharacter (19) overwrites `stack_obj` on `true` (`make_character.rs:30`).
                        write_sym_operand(&mut next_true, 10, 0, SymWord::Unknown);
                    }
                    8 => {
                        let dst_idx = word_i16(ops, 0);
                        let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
                        if dst_scope <= 255 {
                            apply_unknown_write(&mut next_true, dst_scope as u8, dst_idx);
                            apply_unknown_write(&mut next_false, dst_scope as u8, dst_idx);
                        }
                    }
                    11 => {
                        // GetDistanceTo writes distance into `temps[word(ops, 0)]` (`distance.rs:68`).
                        if let Ok(t) = usize::try_from(word_i16(ops, 0))
                            && t < 8
                        {
                            next_true.temps[t] = SymWord::Unknown;
                        }
                    }
                    12 => {
                        // GetDirectionTo writes direction into `(word_u16(ops, 2), word_i16(ops, 0))`.
                        let dst_idx = word_i16(ops, 0);
                        let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
                        if dst_scope <= 255 {
                            apply_unknown_write(&mut next_true, dst_scope as u8, dst_idx);
                        }
                    }
                    14 => {
                        // FindBestObject updates `stack_obj` ONLY on `true` (`find_best_object.rs:280`);
                        // on `false`, `stack_obj` remains unchanged.
                        // On `true`, if the search looks for a specific OBJf callback (`find_best_object.rs:128`)
                        // that none of this object's OBJf tables implement, the result cannot be `InitialTarget`.
                        let func = word_i16(ops, 0);
                        let callback = match func {
                            0 => Some(0x12usize),
                            1 => Some(0x13),
                            2 => Some(0x14),
                            3 => Some(0x15),
                            6 => Some(0x18),
                            7 => Some(0x19),
                            8 => Some(0x1a),
                            10 => Some(0x0e),
                            11 => Some(0x1c),
                            12 => Some(0x10),
                            13 => Some(0x11),
                            14 => Some(0x1d),
                            _ => None,
                        };
                        let cannot_match_self = callback.is_some_and(|cb| {
                            !model.objds.is_empty()
                                && model.objds.keys().all(|objd_id| {
                                    model.objfs.get(objd_id).is_some_and(|objf| {
                                        objf.entries.get(cb).is_none_or(|e| e.action == 0)
                                    })
                                })
                        });
                        let found_obj = if cannot_match_self {
                            SymWord::OtherObject
                        } else {
                            SymWord::Unknown
                        };
                        write_sym_operand(&mut next_true, 10, 0, found_obj);
                    }
                    17 => {
                        // IdleForInput decrements/zeroes its param slot.
                        let p = word_i16(ops, 0);
                        write_sym_operand(&mut next_true, 9, p, SymWord::Unknown);
                        write_sym_operand(&mut next_false, 9, p, SymWord::Unknown);
                    }
                    24 if ops[0] == 0 => {
                        // OldRelationship read writes `param[ops[2] as i8]` on `true` only.
                        write_sym_operand(
                            &mut next_true,
                            9,
                            i16::from(ops[2] as i8),
                            SymWord::Unknown,
                        );
                    }
                    26 if ops[2] & 4 == 0 => {
                        // Relationship read writes into `(ops[4], word_i16(ops, 6))` on `true` only
                        // (`relationship.rs:178-195`); `false` exits before writing.
                        apply_unknown_write(&mut next_true, ops[4], word_i16(ops, 6));
                    }
                    33 => {
                        // FindWorstMotives overwrites temps 0..5 (`worst_motives.rs:93`).
                        for slot in &mut next_true.temps[..5] {
                            *slot = SymWord::Unknown;
                        }
                    }
                    31 => {
                        // SetToNext writes dest (or stack_obj if bit 7 clear) on `true`;
                        // on `false`:
                        // - modes 1/3/5/7 store `0` (`set_to_next.rs:14, 188, 203`)
                        // - mode 4 stores `0` when GUID exists or keeps cursor when missing (`set_to_next.rs:230`),
                        //   so `stack_obj` becomes `Unknown` (refuses release credit while still catching acquires!)
                        // - other modes preserve the cursor.
                        let mode = ops[4];
                        let base = mode & 0x7F;
                        if mode & 0x80 != 0 {
                            let dst_scope = ops[5];
                            let dst_idx = i16::from(ops[7]);
                            apply_unknown_write(&mut next_true, dst_scope, dst_idx);
                            if matches!(base, 1 | 3 | 4 | 5 | 7) {
                                apply_unknown_write(&mut next_false, dst_scope, dst_idx);
                            }
                        } else {
                            write_sym_operand(&mut next_true, 10, 0, SymWord::Unknown);
                            if matches!(base, 1 | 3 | 5 | 7) {
                                write_sym_operand(&mut next_false, 10, 0, SymWord::Const(0));
                            } else if base == 4 {
                                write_sym_operand(&mut next_false, 10, 0, SymWord::Unknown);
                            }
                        }
                    }
                    36 => {
                        // Dialog writes result into selected temp slot.
                        let t = usize::from((ops[7] >> 4) & 7);
                        next_true.temps[t] = SymWord::Unknown;
                        next_false.temps[t] = SymWord::Unknown;
                    }
                    42 => {
                        // CreateObject replaces `stack_obj` with the newly created object ONLY on
                        // `true` (`create.rs:581`); on `false`, `stack_obj` remains unchanged.
                        write_sym_operand(&mut next_true, 10, 0, SymWord::OtherObject);
                    }
                    44 => {
                        // Animate (`animate.rs:197`): firing an animation event takes `on_false`
                        // and writes the event ID into local[ops[2]] (flag 0x20) or param[0].
                        if ops[5] & 0x20 != 0 {
                            write_sym_operand(
                                &mut next_false,
                                25,
                                i16::from(ops[2]),
                                SymWord::Unknown,
                            );
                        } else {
                            write_sym_operand(&mut next_false, 9, 0, SymWord::Unknown);
                        }
                    }
                    51 if ops[0] == 3 => {
                        // ManageInventory op 3 (`inventory.rs:219-236`): writes count then index on
                        // `true`; leaves both temps unchanged on `false`.
                        let flags = u16::from_le_bytes([ops[2], ops[3]]);
                        next_true.temps[usize::from((flags >> 5) & 3)] = SymWord::Unknown;
                        next_true.temps[usize::from((flags >> 11) & 3)] = SymWord::Unknown;
                    }
                    51 if ops[0] == 4 => {
                        // ManageInventory op 4 (`inventory.rs:238-265`): writes count then index on
                        // both branches; on `false`, writes `count = 0` then `index = -1` in VM order.
                        let flags = u16::from_le_bytes([ops[2], ops[3]]);
                        let count_t = usize::from((flags >> 5) & 3);
                        let index_t = usize::from((flags >> 11) & 3);
                        next_true.temps[count_t] = SymWord::Unknown;
                        next_true.temps[index_t] = SymWord::Unknown;
                        next_false.temps[count_t] = SymWord::Const(0);
                        next_false.temps[index_t] = SymWord::Const(-1);
                    }
                    _ => {}
                }

                if is_always_true_instruction(ins) {
                    advance(
                        next_true,
                        ins.on_true,
                        format!("tree#{cur_tree}@n{cur_pc}:prim({})->true", ins.opcode),
                        &mut visited,
                        &mut queue,
                    )?;
                } else {
                    advance(
                        next_true,
                        ins.on_true,
                        format!("tree#{cur_tree}@n{cur_pc}:prim({})->true", ins.opcode),
                        &mut visited,
                        &mut queue,
                    )?;
                    advance(
                        next_false,
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

/// Returns whether a SimAntics instruction never takes `on_false` when it succeeds
/// without trapping (meaning its `on_false` branch is architecturally unreachable).
///
/// NOTE: This predicate does **not** mean the instruction is trap-free — see
/// [`primitive_may_trap_in_lock_prover`] for lock-safety trap modeling.
pub(crate) fn is_always_true_instruction(ins: &Instruction) -> bool {
    match ins.opcode {
        0 => true,                             // Sleep (yields until done, then branches true)
        6 => true,                             // ChangeSuit (`change_suit.rs:66`)
        7 => true,                             // Refresh (`refresh.rs:36,42,53,56`)
        11 => true,                            // GetDistanceTo (`distance.rs:69`)
        12 => true,                            // GetDirectionTo (`direction.rs:88`)
        15 => true,                            // Breakpoint (`debug.rs:37`)
        16 => ins.operands[0] == 1, // FindLocation mode 1 (take off lot, `find_location.rs:122`)
        21 => true,                 // ShowString (`debug.rs:41`)
        22 => word_i16(&ins.operands, 0) == 0, // LookTowards mode 0 (`look_towards.rs:84`)
        23 => true,                 // PlaySound (`sound.rs:61`)
        24 => ins.operands[4] & 1 == 0, // OldRelationship without fail-if-missing (`relationship.rs:62,161`)
        26 => ins.operands[2] & 1 == 0, // Relationship without fail-if-missing (`relationship.rs:161`)
        29 => true, // SetMotiveChange / ClearMotiveChanges (`motive_change.rs:54,70`)
        33 => true, // FindWorstMotives (`worst_motives.rs:97`)
        34 => true, // UiEffect (`ui_effect.rs:358`)
        35 => true, // SpecialEffect (`special_effect.rs:110,113`)
        41 => true, // SetHeadline (`headline.rs:123..176`)
        48 => true, // StopAllSounds (`sound.rs:71`)
        49 => true, // NotifyIdle (`notify.rs:36,64`)
        50 => true, // ActionString (`action_string.rs:102,105,120`)
        2 => matches!(ins.operands[5], 3..=7 | 9 | 10 | 12 | 13 | 18 | 19 | 20),
        _ => false,
    }
}

/// Conservative trap predicate for primitives inside [`prove_interaction_lock_safety`]:
/// Returns the attribute array length `sims-vm` allocates for an `OBJD` chunk:
/// word 58 read as signed `i16`, clamped to at least 8 (`sims-vm/src/factory.rs:314`).
fn objd_attribute_count(objd: &ObjdChunk) -> usize {
    let raw = objd.words.get(58).copied().unwrap_or(0) as i16;
    usize::try_from(raw).unwrap_or(0).max(8)
}

/// Returns `true` whenever the instruction can raise a VM trap under the current symbolic
/// state (even if `is_always_true_instruction` is true because `on_false` is unreachable).
fn primitive_may_trap_in_lock_prover(
    model: &ObjectModel,
    state: &LockState,
    ins: &Instruction,
) -> bool {
    let frame = state.frames.last().expect("non-empty frame stack");
    let ops = &ins.operands;
    let check_param = |p: i16| p < 0 || (p as usize) >= frame.params.len();
    let check_local = |l: i16| l < 0 || (l as usize) >= frame.locals.len();
    match ins.opcode {
        0 => frame.synchronous || check_param(word_i16(ops, 0)),
        6 => ops[1] != 1, // scope > 2 traps 0x2f; accessory lookup (0|2) can trap 0x2f/0x30
        7 => {
            let selector = word_i16(ops, 0);
            let mode = word_i16(ops, 2);
            !(0..=1).contains(&selector)
                || !(0..=2).contains(&mode)
                || (selector == 1 && frame.stack_obj != SymWord::InitialTarget)
        }
        9 => frame.stack_obj != SymWord::InitialTarget,
        10 => ops[0] > 1,
        11 => {
            !(0..=7).contains(&word_i16(ops, 0))
                || frame.stack_obj != SymWord::InitialTarget
                || (ops[2] & 1 != 0)
        }
        12 => {
            let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
            let dst_idx = word_i16(ops, 0);
            let dst_may_trap = if dst_scope > 255 || !is_valid_memory_scope(dst_scope as u8) {
                true
            } else {
                match dst_scope as u8 {
                    // Read-only, float motive, or object/room/world scopes that may trap on write.
                    0 | 3 | 4 | 7 | 14..=21 | 23 | 24 | 26..=38 => true,
                    1 => {
                        frame.stack_obj != SymWord::InitialTarget
                            || dst_idx < 0
                            || model.objds.is_empty()
                            || !model
                                .objds
                                .values()
                                .all(|o| usize::from(dst_idx as u16) < objd_attribute_count(o))
                    }
                    6 => !matches!(dst_idx, 34 | 36),
                    8 => !(0..8).contains(&dst_idx),
                    9 => check_param(dst_idx),
                    10 => dst_idx != 0,
                    11 => {
                        !(0..8).contains(&dst_idx)
                            || !matches!(
                                state.temps[dst_idx as usize],
                                SymWord::Const(t) if (0..8).contains(&t)
                            )
                    }
                    13 => !(0..8).contains(&dst_idx) || frame.stack_obj != SymWord::InitialTarget,
                    22 => {
                        check_param(dst_idx)
                            || frame.stack_obj != SymWord::InitialTarget
                            || model.objds.is_empty()
                            || !matches!(
                                frame.params[dst_idx as usize],
                                SymWord::Const(a)
                                    if a >= 0
                                        && model.objds.values().all(|o| {
                                            usize::from(a as u16) < objd_attribute_count(o)
                                        })
                            )
                    }
                    25 => check_local(dst_idx),
                    _ => true,
                }
            };
            dst_may_trap || frame.stack_obj != SymWord::InitialTarget || (ops[4] & 1 != 0)
        }
        15 => {
            let scope = word_i16(ops, 2);
            let cond_idx = word_i16(ops, 0);
            let safe_read = match scope {
                7 => true,
                8 => (0..8).contains(&cond_idx),
                9 => !check_param(cond_idx),
                25 => !check_local(cond_idx),
                26 => lookup_bcon(model, cond_idx).is_some(),
                _ => false,
            };
            !(safe_read
                && eval_sym_operand(model, state, scope as u8, cond_idx) == SymWord::Const(0))
        }
        16 => {
            frame.stack_obj != SymWord::InitialTarget
                || (ops[2] & 1 != 0)
                || matches!(ops[0], 3 | 4)
        }
        17 | 27 | 44 | 45 => frame.synchronous,
        19 | 24 | 26 | 30 => true,
        21 | 23 | 48 => false,
        22 => {
            let m = word_i16(ops, 0);
            !(0..=3).contains(&m)
                || (m != 0 && frame.synchronous)
                || (matches!(m, 2 | 3) && frame.stack_obj != SymWord::InitialTarget)
        }
        29 => {
            ops[3] & 1 == 0
                && (!matches!(ops[2], 5..=9 | 14..=15)
                    || !is_valid_memory_scope(ops[0])
                    || !is_valid_memory_scope(ops[1]))
        }
        33 => {
            let target = u16::from_le_bytes([ops[4], ops[5]]);
            let mode = u16::from_le_bytes([ops[6], ops[7]]);
            target == 2 || mode > 2 || (target == 1 && frame.stack_obj != SymWord::InitialTarget)
        }
        34 => {
            ops[0] <= 2
                && (!is_valid_memory_scope(ops[1])
                    || (ops[1] == 6 && word_i16(ops, 2) == 34)
                    || (ops[1] == 9 && check_param(word_i16(ops, 2)))
                    || (ops[1] == 25 && check_local(word_i16(ops, 2))))
        }
        35 => {
            frame.synchronous
                || frame.stack_obj != SymWord::InitialTarget
                || (ops[4] & 0x20 != 0 && check_local(word_i16(ops, 0)))
        }
        41 => {
            (ops[0] & 1 != 0 && frame.stack_obj != SymWord::InitialTarget)
                || (ops[3] == 7 && ops[2] == 2 && check_local(word_i16(ops, 0) >> 1))
        }
        47 => frame.synchronous || (word_i16(ops, 0) == 1 && check_param(word_i16(ops, 4))),
        49 => frame.stack_obj != SymWord::InitialTarget,
        50 => {
            // `action_string.rs:83-107`: traps if synchronous, if string is missing, or if
            // `format_text` encounters any `$`-token that can trap (`$Family`, `$Local`, etc.).
            if frame.synchronous {
                return true;
            }
            let table = u16::from_le_bytes([ops[0], ops[1]]);
            let Some(idx) = usize::from(ops[4]).checked_sub(1) else {
                return true;
            };
            match model.string_entries.get(&table).and_then(|t| t.get(idx)) {
                Some(text) => text.contains('$'),
                None => true,
            }
        }
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
    /// Initial frame parameter `param[i]`.
    InitParam(u16),
    /// Initial (possibly inherited/nonzero) frame local `local[i]`.
    InitLocal(u16),
    /// Interned symbolic expression or post-event value (disjoint from widened tokens).
    Sym(u32),
    /// Widened loop-header local `local[i]` (distinct namespace, never collides with `Sym`).
    WidenedLocal(u16),
    /// Widened loop-header parameter `param[i]` (distinct namespace, never collides with `Sym`).
    WidenedParam(u16),
    /// External/shared memory operand `(scope, index)`.
    Mem(u8, i16),
}

impl CanonOperand {
    fn is_pure(self) -> bool {
        !matches!(self, Self::Mem(..))
    }
}

/// Canonicalized pure comparison predicate `(lhs op rhs)` normalized so equivalent
/// formulations (`a != b` vs `!(a == b)`, `a >= b` vs `!(a < b)`, `a <= b` vs `!(a > b)`,
/// `a >= c` vs `a > c-1`) map to the exact same canonical predicate without reordering
/// external memory reads or erasing `LOCKOUT`/fault side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CanonPred {
    /// Always true or always false (constant-folded pure comparison).
    Const(bool),
    /// `lhs == rhs` (commuted only when both operands are pure/side-effect-free).
    Eq(CanonOperand, CanonOperand),
    /// `lhs < rhs`.
    Lt(CanonOperand, CanonOperand),
    /// `lhs > rhs` (preserved without operand swap when an external memory operand is involved).
    Gt(CanonOperand, CanonOperand),
    /// `lhs flag_test rhs`.
    FlagTest(CanonOperand, CanonOperand),
}

impl CanonPred {
    fn touches_mem(self) -> bool {
        match self {
            Self::Const(_) => false,
            Self::Eq(a, b) | Self::Lt(a, b) | Self::Gt(a, b) | Self::FlagTest(a, b) => {
                !a.is_pure() || !b.is_pure()
            }
        }
    }

    fn touches_scope_22(self) -> bool {
        let is_22 = |op: CanonOperand| matches!(op, CanonOperand::Mem(22, _));
        match self {
            Self::Const(_) => false,
            Self::Eq(a, b) | Self::Lt(a, b) | Self::Gt(a, b) | Self::FlagTest(a, b) => {
                is_22(a) || is_22(b)
            }
        }
    }
}

fn is_global_34(scope: u8, idx: i16) -> bool {
    scope == 6 && idx == 34
}

fn bhav_uses_global_34(bhav: &BhavChunk) -> bool {
    bhav.instructions.iter().any(|ins| match ins.opcode {
        2 => {
            is_global_34(ins.operands[6], word_i16(&ins.operands, 0))
                || is_global_34(ins.operands[7], word_i16(&ins.operands, 2))
        }
        8 => {
            (u16::from_le_bytes([ins.operands[2], ins.operands[3]]) == 6
                && word_i16(&ins.operands, 0) == 34)
                || (u16::from_le_bytes([ins.operands[6], ins.operands[7]]) == 6
                    && word_i16(&ins.operands, 4) == 34)
        }
        11 => ins.operands[2] & 1 != 0 && is_global_34(ins.operands[3], word_i16(&ins.operands, 4)),
        12 => {
            (u16::from_le_bytes([ins.operands[2], ins.operands[3]]) == 6
                && word_i16(&ins.operands, 0) == 34)
                || (ins.operands[4] & 1 != 0
                    && is_global_34(ins.operands[5], word_i16(&ins.operands, 6)))
        }
        15 => word_i16(&ins.operands, 2) == 6 && word_i16(&ins.operands, 0) == 34,
        // `UI Effect` (opcode 34) reads Global 34 / scratch pointers (`ui_effect.rs:371`), and
        // any dynamic tree callback (`FindBestInteraction`, `FindBestObject`, `RunFunctionalTree`,
        // `RunTreeByName`, `GosubFoundAction`, `Reach`) or subroutine call can invoke a callee on
        // the actor's stack that observes the scratch pointer.
        3 | 14 | 20 | 28 | 30 | 34 | 47 | 256..=u16::MAX => true,
        _ => false,
    })
}

fn canonicalize_compare(
    op: u8,
    lhs: CanonOperand,
    rhs: CanonOperand,
    on_true: u8,
    on_false: u8,
    can_commute: bool,
) -> Option<(CanonPred, u8, u8)> {
    let both_pure = can_commute && lhs.is_pure() && rhs.is_pure();
    // Constant-fold only when both sides are concrete pure constants.
    if let (CanonOperand::Const(l), CanonOperand::Const(r)) = (lhs, rhs)
        && both_pure
        && let Some(val) = eval_expr_compare(l, op, r)
    {
        return Some((CanonPred::Const(val), on_true, on_false));
    }

    match op {
        // lhs == rhs
        2 => {
            if lhs == rhs && both_pure {
                return Some((CanonPred::Const(true), on_true, on_false));
            }
            let (a, b) = if both_pure && rhs < lhs {
                (rhs, lhs)
            } else {
                (lhs, rhs)
            };
            Some((CanonPred::Eq(a, b), on_true, on_false))
        }
        // lhs != rhs  <=>  !(lhs == rhs)
        16 => {
            if lhs == rhs && both_pure {
                return Some((CanonPred::Const(false), on_true, on_false));
            }
            let (a, b) = if both_pure && rhs < lhs {
                (rhs, lhs)
            } else {
                (lhs, rhs)
            };
            Some((CanonPred::Eq(a, b), on_false, on_true))
        }
        // lhs < rhs
        1 => Some(canonicalize_lt(lhs, rhs, on_true, on_false, both_pure)),
        // lhs >= rhs  <=>  !(lhs < rhs)
        14 => Some(canonicalize_lt(lhs, rhs, on_false, on_true, both_pure)),
        // lhs > rhs
        0 => {
            if both_pure {
                Some(canonicalize_lt(rhs, lhs, on_true, on_false, true))
            } else {
                // Keep exact LHS/RHS evaluation order for external memory operands.
                match rhs {
                    CanonOperand::Const(c) if c < i16::MAX => Some((
                        CanonPred::Lt(lhs, CanonOperand::Const(c + 1)),
                        on_false,
                        on_true,
                    )),
                    _ => Some((CanonPred::Gt(lhs, rhs), on_true, on_false)),
                }
            }
        }
        // lhs <= rhs  <=>  !(lhs > rhs)
        15 => {
            if both_pure {
                Some(canonicalize_lt(rhs, lhs, on_false, on_true, true))
            } else {
                match rhs {
                    CanonOperand::Const(c) if c < i16::MAX => Some((
                        CanonPred::Lt(lhs, CanonOperand::Const(c + 1)),
                        on_true,
                        on_false,
                    )),
                    _ => Some((CanonPred::Gt(lhs, rhs), on_false, on_true)),
                }
            }
        }
        // flag_test
        8 => Some((CanonPred::FlagTest(lhs, rhs), on_true, on_false)),
        _ => None,
    }
}

/// Normalizes integer `<` inequalities with literal bounds without reordering external memory reads.
fn canonicalize_lt(
    lhs: CanonOperand,
    rhs: CanonOperand,
    on_true: u8,
    on_false: u8,
    both_pure: bool,
) -> (CanonPred, u8, u8) {
    if lhs == rhs && both_pure {
        return (CanonPred::Const(false), on_true, on_false);
    }
    // `Const(c) < x` <=> `x > c` <=> `!(x < c + 1)` when both are pure and `c < i16::MAX`.
    if both_pure
        && let CanonOperand::Const(c) = lhs
        && !matches!(rhs, CanonOperand::Const(_))
        && c < i16::MAX
    {
        return (
            CanonPred::Lt(rhs, CanonOperand::Const(c + 1)),
            on_false,
            on_true,
        );
    }
    // `x < Const(i16::MIN)` is always false (only fold when `lhs` is pure so faults/LOCKOUT aren't dropped).
    if both_pure && matches!(rhs, CanonOperand::Const(i16::MIN)) {
        return (CanonPred::Const(false), on_true, on_false);
    }
    (CanonPred::Lt(lhs, rhs), on_true, on_false)
}

/// Canonicalizes primitive operands by zeroing verified-ignored padding bytes.
pub(crate) fn canonicalize_primitive_operands(opcode: u16, ops: [u8; 8]) -> [u8; 8] {
    match opcode {
        // Sleep: word 0 (param_index) only (`sleep.rs:29`).
        0 => [ops[0], ops[1], 0, 0, 0, 0, 0, 0],
        // GenericCall: word 0 (routine) only.
        1 => [ops[0], ops[1], 0, 0, 0, 0, 0, 0],
        // FindBestInteraction / Grab / Drop / GosubFoundAction: operands ignored.
        3 | 4 | 5 | 30 => [0; 8],
        // ChangeSuit: index, scope, bit 0 of flags (`change_suit.rs:49-50`).
        6 => [ops[0], ops[1], ops[2] & 1, 0, 0, 0, 0, 0],
        // Refresh: word 0 (target) + word 1 (mode) (`refresh.rs:25`).
        7 => [ops[0], ops[1], ops[2], ops[3], 0, 0, 0, 0],
        // Burn: byte 0 == 1 (in_front), byte 1 bit 0 (any_state) (`burn.rs:136,172`).
        9 => [u8::from(ops[0] == 1), ops[1] & 1, 0, 0, 0, 0, 0, 0],
        // Tutorial: byte 0 (mode) only (`events.rs:22`).
        10 => [ops[0], 0, 0, 0, 0, 0, 0, 0],
        // GetDistanceTo: word 0 (temp), byte 2 bit 0 (has_from), optional from (3..6) (`distance.rs:47-55`).
        11 => {
            if ops[2] & 1 != 0 {
                [ops[0], ops[1], 1, ops[3], ops[4], ops[5], 0, 0]
            } else {
                [ops[0], ops[1], 0, 0, 0, 0, 0, 0]
            }
        }
        // GetDirectionTo: dst (0..4), byte 4 bit 0 (has_from), optional from (5..8) (`direction.rs:45-59`).
        12 => {
            if ops[4] & 1 != 0 {
                [ops[0], ops[1], ops[2], ops[3], 1, ops[5], ops[6], ops[7]]
            } else {
                [ops[0], ops[1], ops[2], ops[3], 0, 0, 0, 0]
            }
        }
        // PushInteraction: menu, object_slot, priority, flags & 0x0F, icon (when bit 0 set) (`push.rs:65-165`).
        13 => [
            ops[0],
            ops[1],
            ops[2],
            ops[3] & 0x0F,
            if ops[3] & 1 != 0 { ops[4] } else { 0 },
            0,
            0,
            0,
        ],
        // FindBestObject: word 0 (func), optional origin when flags & 8, byte 6 (flags) (`find_best_object.rs:153-168`).
        14 => {
            if ops[6] & 8 != 0 {
                [ops[0], ops[1], ops[2], ops[3], ops[4], ops[5], ops[6], 0]
            } else {
                [ops[0], ops[1], 0, 0, 0, 0, ops[6], 0]
            }
        }
        // Breakpoint: word 0 (index) + word 1 (scope) (`debug.rs:32-34`).
        15 => [ops[0], ops[1], ops[2], ops[3], 0, 0, 0, 0],
        // FindLocation: mode, local (when flags & 1), flags & 7 (`find_location.rs:70-128`).
        16 => [
            ops[0],
            if ops[2] & 1 != 0 { ops[1] } else { 0 },
            ops[2] & 7,
            0,
            0,
            0,
            0,
            0,
        ],
        // IdleForInput: word 0 (param_index) + word 1 (allow_push) (`idle.rs:7-9`).
        17 => [ops[0], ops[1], ops[2], ops[3], 0, 0, 0, 0],
        // RemoveObject: word 0 (target) + byte 2 (flags) (`remove.rs:47-53`).
        18 => [ops[0], ops[1], ops[2], 0, 0, 0, 0, 0],
        // MakeNewCharacter: bytes 0..4 (skin, age, sex, species) (`make_character.rs:8-16`).
        19 => [ops[0], ops[1], ops[2], ops[3], 0, 0, 0, 0],
        // RunFunctionalTree: word 0 (function) + byte 2 bit 0 (`run_functional_tree.rs:75-93`).
        20 => [ops[0], ops[1], ops[2] & 1, 0, 0, 0, 0, 0],
        // ShowString: word 0 (table) + word 1 (index); bytes 4..8 unused (`debug.rs:12`).
        21 => [ops[0], ops[1], ops[2], ops[3], 0, 0, 0, 0],
        // LookTowards: word 0 (mode) only (`look_towards.rs:66`).
        22 => [ops[0], ops[1], 0, 0, 0, 0, 0, 0],
        // PlaySound: word 0 (sound_id) + effective stack_obj flag (`sound.rs:41-47`).
        23 => {
            let use_stack = ops[4] != 0xFF && ops[4] & 2 != 0;
            [ops[0], ops[1], 0, 0, if use_stack { 2 } else { 0 }, 0, 0, 0]
        }
        // OldRelationship: byte 0 != 0 (write), col, param, dir, flags & 3 (`relationship.rs:45-64`).
        24 => [
            u8::from(ops[0] != 0),
            ops[1],
            ops[2],
            ops[3],
            ops[4] & 3,
            0,
            0,
            0,
        ],
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
        // Relationship: col, dir, flags & 7, local (when dir 2|3), value scope, pad 0, value idx (`relationship.rs:78-179`).
        26 => [
            ops[0],
            ops[1],
            ops[2] & 7,
            if matches!(ops[1], 2 | 3) { ops[3] } else { 0 },
            ops[4],
            0,
            ops[6],
            ops[7],
        ],
        // GoToRelative: byte 2 (location), byte 3 (facing), byte 6 bits 1..=2 (`go_to_slot.rs:278-302`).
        27 => [0, 0, ops[2], ops[3], 0, 0, ops[6] & 6, 0],
        // RunTreeByName: word 0 (table), byte 2 bit 0 (global), byte 4 (index), byte 5 (mode) (`named_tree.rs:60-85`).
        28 => [ops[0], ops[1], ops[2] & 1, 0, ops[4], ops[5], 0, 0],
        // SetMotiveChange: if flags & 1 != 0, clear all (other bytes ignored; `motive_change.rs:52`).
        29 => {
            if ops[3] & 1 != 0 {
                [0, 0, 0, 1, 0, 0, 0, 0]
            } else {
                [ops[0], ops[1], ops[2], 0, ops[4], ops[5], ops[6], ops[7]]
            }
        }
        // SetToNext: when bit 7 of mode is clear, dest scope/index bytes 5 and 7 are ignored (`set_to_next.rs:7-9`).
        31 => {
            if ops[4] & 0x80 != 0 {
                ops
            } else {
                [ops[0], ops[1], ops[2], ops[3], ops[4], 0, ops[6], 0]
            }
        }
        // TestObjectType: GUID (0..4), index (4..6), scope (6); byte 7 ignored (`object_type.rs:6-7`).
        32 => [ops[0], ops[1], ops[2], ops[3], ops[4], ops[5], ops[6], 0],
        // FindWorstMotives: words 2..4 (target, mode); bytes 0..4 unread (`worst_motives.rs:56-68`).
        33 => [0, 0, 0, 0, ops[4], ops[5], ops[6], ops[7]],
        // UiEffect: mode (0); for mode <= 2: scope (1), index (2..4), highlight bit 0 (4) (`ui_effect.rs:324-356`).
        34 => {
            if ops[0] <= 2 {
                [ops[0], ops[1], ops[2], ops[3], ops[4] & 1, 0, 0, 0]
            } else {
                [ops[0], 0, 0, 0, 0, 0, 0, 0]
            }
        }
        // SpecialEffect: bytes 0..6 (duration, kind, size, flags, str_index); bytes 6..8 unread (`special_effect.rs:5-7`).
        35 => [ops[0], ops[1], ops[2], ops[3], ops[4], ops[5], 0, 0],
        // Dialog: kind is low 7 bits of byte 5 (`dialog.rs:569`).
        36 => [
            ops[0],
            ops[1],
            ops[2],
            ops[3],
            ops[4],
            ops[5] & 0x7F,
            ops[6],
            ops[7],
        ],
        // TestSimInteractingWith: byte 0 == 1 reads scope/index (1..4); otherwise ignores all (`interacting.rs:27-39`).
        37 => {
            if ops[0] == 1 {
                [1, ops[1], ops[2], ops[3], 0, 0, 0, 0]
            } else {
                [0; 8]
            }
        }
        // SetHeadline: word 0 upper bits only read when kind == 7 && index == 2 (`headline.rs:111,198`).
        41 => {
            let (b0, b1) = if ops[3] == 7 && ops[2] == 2 {
                (ops[0], ops[1])
            } else {
                (ops[0] & 1, 0)
            };
            [b0, b1, ops[2], ops[3], ops[4], ops[5], ops[6], ops[7]]
        }
        // CreateObject: bytes 0..7; byte 7 unused (`create.rs:9-10`).
        42 => [ops[0], ops[1], ops[2], ops[3], ops[4], ops[5], ops[6], 0],
        // DropOnto: kind words are zero vs nonzero booleans (`drop_onto.rs:120`).
        43 => {
            let sk = u8::from(word_i16(&ops, 0) != 0);
            let dk = u8::from(word_i16(&ops, 4) != 0);
            [sk, 0, ops[2], ops[3], dk, 0, ops[6], ops[7]]
        }
        // Animate: bytes 3 and 7 unused (`animate.rs:8-10`).
        44 => [ops[0], ops[1], ops[2], 0, ops[4], ops[5], ops[6], 0],
        // RouteTo: word 0 (slot), word 1 (source), byte 4 bit 0 (`go_to_slot.rs:338-388`).
        45 => [ops[0], ops[1], ops[2], ops[3], ops[4] & 1, 0, 0, 0],
        // Snap: word 0 (slot; ignored in modes 1/2), word 1 (mode), byte 4 (flags) (`snap.rs:4-14`).
        46 => {
            let mode = word_i16(&ops, 2);
            if mode == 1 || mode == 2 {
                [0, 0, ops[2], ops[3], ops[4], 0, 0, 0]
            } else {
                [ops[0], ops[1], ops[2], ops[3], ops[4], 0, 0, 0]
            }
        }
        // Reach: word 0 (mode), word 1 != 0 (moves), word 2 (slot_param, mode 1 only) (`reach.rs:106-107,213`).
        47 => {
            let mode = word_i16(&ops, 0);
            let moves = u8::from(word_i16(&ops, 2) != 0);
            if mode == 1 {
                [ops[0], ops[1], moves, 0, ops[4], ops[5], 0, 0]
            } else {
                [ops[0], ops[1], moves, 0, 0, 0, 0, 0]
            }
        }
        // StopAllSounds: word 0 != 0 boolean (`sound.rs:65`).
        48 => {
            let flag = u8::from(i16::from_le_bytes([ops[0], ops[1]]) != 0);
            [flag, 0, 0, 0, 0, 0, 0, 0]
        }
        // NotifyIdle: operands ignored (`notify.rs:24`).
        49 => [0; 8],
        // ActionString: word 0 (table) + byte 4 (index) (`action_string.rs:126-127`).
        50 => [ops[0], ops[1], 0, 0, ops[4], 0, 0, 0],
        // ManageInventory: all 8 bytes used (`inventory.rs:98-121`).
        51 => ops,
        _ => ops,
    }
}

/// Frame-private symbolic environment for one BHAV during equivalence checking.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LocalEnv {
    arg_count: usize,
    local_count: usize,
    params: BTreeMap<i16, CanonOperand>,
    locals: BTreeMap<i16, CanonOperand>,
}

impl LocalEnv {
    fn new(arg_count: usize, local_count: usize) -> Self {
        let mut params = BTreeMap::new();
        for i in 0..arg_count {
            params.insert(i as i16, CanonOperand::InitParam(i as u16));
        }
        let mut locals = BTreeMap::new();
        for i in 0..local_count {
            // Frame locals can inherit nonzero values from prior frames (`interpreter.rs:1210`).
            locals.insert(i as i16, CanonOperand::InitLocal(i as u16));
        }
        Self {
            arg_count,
            local_count,
            params,
            locals,
        }
    }

    fn has_slot(&self, scope: u8, idx: i16) -> bool {
        match scope {
            9 => idx >= 0 && (idx as usize) < self.arg_count,
            25 => idx >= 0 && (idx as usize) < self.local_count,
            _ => true,
        }
    }

    fn read(&self, scope: u8, idx: i16) -> CanonOperand {
        match scope {
            7 => CanonOperand::Const(idx),
            9 => self
                .params
                .get(&idx)
                .copied()
                .unwrap_or(CanonOperand::Mem(scope, idx)),
            25 => self
                .locals
                .get(&idx)
                .copied()
                .unwrap_or(CanonOperand::Mem(scope, idx)),
            // Scope 22 (`stack.attr[param[idx]]`) is external memory, NOT frame-private.
            _ => CanonOperand::Mem(scope, idx),
        }
    }

    fn write(&mut self, scope: u8, idx: i16, val: CanonOperand) {
        if !self.has_slot(scope, idx) {
            return;
        }
        match scope {
            9 => {
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
            9 => self.has_slot(9, idx),
            25 => self.has_slot(25, idx),
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
///
/// Disabled when `uses_global_34` is true because every `Expression` initializes the
/// VM scratch pointer (`expression.rs:51`), which is observable to subsequent Global 34 reads.
fn try_silent_step(
    bhav: &BhavChunk,
    pc: u8,
    env: &mut LocalEnv,
    interner: &mut SymInterner,
    uses_global_34: bool,
) -> Option<u8> {
    if uses_global_34 || matches!(pc, TARGET_ERROR..=TARGET_FALSE) {
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

    // 1. Constant-folded pure comparison (e.g., `0 == 0` jump node emitted by `lower.rs`).
    //    Never fold a read of an uninitialized `InitLocal` (the VM may trap on unreadable
    //    pointer fragments inherited from prior frames, `interpreter.rs:1271`).
    if matches!(op, 0 | 1 | 2 | 8 | 14 | 15 | 16)
        && (lhs_scope == 7 || env.is_frame_private(lhs_scope, lhs_idx))
        && (rhs_scope == 7 || env.is_frame_private(rhs_scope, rhs_idx))
    {
        let lhs = env.read(lhs_scope, lhs_idx);
        let rhs = env.read(rhs_scope, rhs_idx);
        if matches!(lhs, CanonOperand::InitLocal(_)) || matches!(rhs, CanonOperand::InitLocal(_)) {
            return None;
        }
        if let Some((CanonPred::Const(b), t, f)) =
            canonicalize_compare(op, lhs, rhs, ins.on_true, ins.on_false, true)
        {
            return Some(if b { t } else { f });
        }
        // Non-trapping pure comparison with identical targets is a pure jump.
        if ins.on_true == ins.on_false
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
        // Plain overwrite (`=`, `sqrt`) only reads RHS; read-modify-write ops also read LHS.
        let reads_uninit_local = matches!(rhs, CanonOperand::InitLocal(_))
            || (!matches!(op, 5 | 20) && matches!(lhs, CanonOperand::InitLocal(_)));
        if reads_uninit_local {
            return None;
        }
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
    uses_global_34: bool,
) -> std::result::Result<u8, String> {
    if limit == 0 {
        return Ok(pc);
    }
    for _ in 0..limit {
        let Some(next) = try_silent_step(bhav, pc, env, interner, uses_global_34) else {
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
    constraints: BTreeMap<CanonPred, bool>,
    /// Whether the two threads' last branch results may differ (only tracked with
    /// [`EquivOptions::observe_last_result`]).
    last_differs: bool,
}

/// Checks whether two BHAV chunks `a` and `b` are behaviorally trace-equivalent
/// using CFG canonicalization + bounded symbolic bisimulation.
///
/// - Ignores dead/unreachable nodes, Edith node renumbering/reordering, unused
///   operand padding bytes, and unreachable `on_false` targets on always-true primitives.
/// - Models pure `Expression` (`0x0002`) arithmetic and comparisons over symbolic/concrete `i16`.
/// - Models world-interacting primitives and subroutines as canonical trace events.
/// - Never reports false positives: returns [`EquivResult::Inconclusive`] if
///   bounds are exceeded or a layout-dependent call's callee arity is unknown.
#[must_use]
pub fn check_bhav_equivalent(a: &BhavChunk, b: &BhavChunk, opts: &EquivOptions) -> EquivResult {
    check_bhav_equivalent_with_arities(a, b, opts, None)
}

/// Like [`check_bhav_equivalent`], additionally accepting a known `(tree_id -> arg_count)`
/// map so inline subroutine calls (`opcode >= 256`) can be verified soundly even when
/// callers are reordered or pass >4 inline arguments across adjacent instruction bytes.
#[must_use]
pub fn check_bhav_equivalent_with_arities(
    a: &BhavChunk,
    b: &BhavChunk,
    opts: &EquivOptions,
    callee_arities: Option<&BTreeMap<u16, usize>>,
) -> EquivResult {
    lockstep_traced(
        a,
        b,
        opts,
        callee_arities,
        None,
        &mut LockstepTrace::default(),
    )
}

/// Which node pairs a lockstep run visited.
#[derive(Debug, Default)]
struct LockstepTrace {
    /// Every `(node in A, node in B)` pair the product ran.
    pairs: BTreeSet<(u8, u8)>,
    /// Whether some pair ran while the two sides' last branch results may differ.
    last_differs_mid_run: bool,
}

fn lockstep_traced(
    a: &BhavChunk,
    b: &BhavChunk,
    opts: &EquivOptions,
    callee_arities: Option<&BTreeMap<u16, usize>>,
    argless: Option<&BTreeSet<u16>>,
    trace: &mut LockstepTrace,
) -> EquivResult {
    if a.kind != b.kind {
        return EquivResult::Counterexample {
            path: Vec::new(),
            divergence: format!(
                "BHAV kind mismatch: A has kind {} vs B has kind {}",
                a.kind, b.kind
            ),
        };
    }
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
    // Kind-4 indexed-entry trees can be entered at any node (`interpreter.rs:1786`),
    // so both trees must have identical entry-table length and match from every entry PC.
    if a.kind == 4 && a.instructions.len() != b.instructions.len() {
        return EquivResult::Counterexample {
            path: Vec::new(),
            divergence: format!(
                "kind-4 indexed tree length mismatch: A has {} entries vs B has {}",
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
        0x8001 => usize::from(a.locals as u8).min(4),
        _ => usize::from(a.locals as u8),
    };
    let locals_b = match b.format {
        0x8000 => 0,
        0x8001 => usize::from(b.locals as u8).min(4),
        _ => usize::from(b.locals as u8),
    };

    let uses_global_34 = bhav_uses_global_34(a) || bhav_uses_global_34(b);
    let mut interner = SymInterner::new();
    let mut visited: BTreeSet<ProductState> = BTreeSet::new();
    let mut pc_pair_visits: BTreeMap<(u8, u8), usize> = BTreeMap::new();
    let mut queue: VecDeque<(ProductState, Vec<TraceStep>)> = VecDeque::new();

    let entry_count = if a.kind == 4 { a.instructions.len() } else { 1 };
    for entry_pc in 0..entry_count {
        let init = ProductState {
            pc_a: entry_pc as u8,
            pc_b: entry_pc as u8,
            env_a: LocalEnv::new(args_a, locals_a),
            env_b: LocalEnv::new(args_b, locals_b),
            constraints: BTreeMap::new(),
            last_differs: false,
        };
        if visited.insert(init.clone()) {
            queue.push_back((init, Vec::new()));
        }
    }

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
            uses_global_34,
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
            uses_global_34,
        ) {
            Ok(pc) => pc,
            Err(reason) => return EquivResult::Inconclusive { reason },
        };
        if opts.observe_last_result && (pc_a != state.pc_a || pc_b != state.pc_b) {
            // A silent step sets its side's last result alone.
            state.last_differs = true;
        }
        state.pc_a = pc_a;
        state.pc_b = pc_b;

        let term_a = matches!(pc_a, TARGET_ERROR..=TARGET_FALSE);
        let term_b = matches!(pc_b, TARGET_ERROR..=TARGET_FALSE);

        if term_a || term_b {
            if pc_a == pc_b {
                if opts.observe_last_result && state.last_differs {
                    return EquivResult::Counterexample {
                        path,
                        divergence: format!(
                            "both trees reach {} but their last branch results may differ",
                            format_pc(pc_a)
                        ),
                    };
                }
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
        // frame-local environments on A and B, widen both sides symmetrically to distinct
        // `WidenedLocal`/`WidenedParam` tokens (never colliding with `Sym`) so unbounded
        // identical loops prove equivalent in O(N) states!
        trace.pairs.insert((pc_a, pc_b));
        trace.last_differs_mid_run |= state.last_differs;
        let pair_hits = pc_pair_visits.entry((pc_a, pc_b)).or_insert(0);
        *pair_hits += 1;
        if *pair_hits > 4 && state.env_a == state.env_b {
            let mut widened_a = state.env_a.clone();
            let mut widened_b = state.env_b.clone();
            for (&k, val_a) in &mut widened_a.locals {
                let sym = CanonOperand::WidenedLocal(k as u16);
                *val_a = sym;
                widened_b.locals.insert(k, sym);
            }
            for (&k, val_a) in &mut widened_a.params {
                let sym = CanonOperand::WidenedParam(k as u16);
                *val_a = sym;
                widened_b.params.insert(k, sym);
            }
            let widened = ProductState {
                pc_a,
                pc_b,
                env_a: widened_a,
                env_b: widened_b,
                constraints: BTreeMap::new(),
                last_differs: state.last_differs,
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
            // Verify indirect scope-22 parameter indices match on both sides.
            if let Err(msg) = check_expr_indirect_params(ins_a, ins_b, &state.env_a, &state.env_b) {
                return EquivResult::Counterexample {
                    path,
                    divergence: format!("{msg} at A@n{pc_a} vs B@n{pc_b}"),
                };
            }
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
            // Whether canonicalization swapped a side's targets, which makes its own
            // result the negation of the canonical predicate's value.
            let swapped = |op: u8, l: CanonOperand, r: CanonOperand, commute: bool| {
                canonicalize_compare(op, l, r, 0, 1, commute).is_some_and(|(_, t, _)| t == 1)
            };
            let last_differs = opts.observe_last_result
                && swapped(ins_a.operands[5], la, ra, can_commute_a)
                    != swapped(ins_b.operands[5], lb, rb, can_commute_b);
            match (norm_a, norm_b) {
                (Some((pred_a, ta, fa)), Some((pred_b, tb, fb))) if pred_a == pred_b => {
                    let mut enqueue = |na: u8, nb: u8, branch: bool, record_pred: Option<bool>| {
                        let mut next_constraints = state.constraints.clone();
                        // Never cache scope-22 indirect predicates (`stack.attr[param[i]]`) because
                        // a later change to `param[i]` changes the addressed attribute.
                        if let Some(outcome) = record_pred
                            && !matches!(pred_a, CanonPred::Const(_))
                            && !pred_a.touches_scope_22()
                            && next_constraints.len() < 16
                        {
                            next_constraints.insert(pred_a, outcome);
                        }
                        let next = ProductState {
                            pc_a: na,
                            pc_b: nb,
                            env_a: state.env_a.clone(),
                            env_b: state.env_b.clone(),
                            constraints: next_constraints,
                            last_differs,
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
                        CanonPred::Const(true) => enqueue(ta, tb, true, None),
                        CanonPred::Const(false) => enqueue(fa, fb, false, None),
                        _ => {
                            // Prune contradictory branch outcomes on unchanged predicates.
                            match state.constraints.get(&pred_a).copied() {
                                Some(true) => enqueue(ta, tb, true, None),
                                Some(false) => enqueue(fa, fb, false, None),
                                None => {
                                    enqueue(ta, tb, true, Some(true));
                                    enqueue(fa, fb, false, Some(false));
                                }
                            }
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
            if let Err(msg) = check_expr_indirect_params(ins_a, ins_b, &state.env_a, &state.env_b) {
                return EquivResult::Counterexample {
                    path,
                    divergence: format!("{msg} at A@n{pc_a} vs B@n{pc_b}"),
                };
            }
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

            // Apply identical symbolic update to frame-private targets and invalidate any
            // cached memory constraints when writing to external memory.
            let mut next_env_a = state.env_a.clone();
            let mut next_env_b = state.env_b.clone();
            let mut next_constraints = state.constraints.clone();
            if next_env_a.is_frame_private(lhs_scope_a, lhs_idx_a) {
                let sym = interner.fresh_event();
                next_env_a.write(lhs_scope_a, lhs_idx_a, sym);
                next_env_b.write(lhs_scope_b, lhs_idx_b, sym);
            } else {
                next_constraints.retain(|pred, _| !pred.touches_mem());
            }

            let mut enqueue = |na: u8, nb: u8, branch: Option<bool>| {
                let next = ProductState {
                    pc_a: na,
                    pc_b: nb,
                    env_a: next_env_a.clone(),
                    env_b: next_env_b.clone(),
                    constraints: next_constraints.clone(),
                    last_differs: false,
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
        let ops_a = canonicalize_primitive_operands(ins_a.opcode, call_operands(ins_a, argless));
        let ops_b = canonicalize_primitive_operands(ins_b.opcode, call_operands(ins_b, argless));
        let ev_a = CanonEvent::Primitive {
            opcode: ins_a.opcode,
            operands: ops_a,
        };
        let ev_b = CanonEvent::Primitive {
            opcode: ins_b.opcode,
            operands: ops_b,
        };
        if ev_a != ev_b {
            return EquivResult::Counterexample {
                path,
                divergence: format!(
                    "trace event divergence at A@n{pc_a} vs B@n{pc_b}: A=op {:#06x} {ops_a:?} vs B=op {:#06x} {ops_b:?}",
                    ins_a.opcode, ins_b.opcode,
                ),
            };
        }
        // A primitive may start a tree that reads the last result (`found_action.rs:102`).
        if opts.observe_last_result && state.last_differs {
            return EquivResult::Counterexample {
                path,
                divergence: format!(
                    "last branch results may differ before op {:#06x} at A@n{pc_a} vs B@n{pc_b}",
                    ins_a.opcode
                ),
            };
        }

        // Verify layout-dependent inline subroutine calls (`opcode >= 256` with non-0xFF operands).
        // Callees with >4 args read bytes from instructions following the call (`interpreter.rs:1542`).
        if ins_a.opcode >= 256 && ops_a != [0xFF; 8] {
            match callee_arities.and_then(|m| m.get(&ins_a.opcode).copied()) {
                Some(arity) if arity <= 4 => {}
                Some(arity) => {
                    let need = arity * 2;
                    let bytes_a = collect_inline_call_arg_bytes(a, usize::from(pc_a), need);
                    let bytes_b = collect_inline_call_arg_bytes(b, usize::from(pc_b), need);
                    if bytes_a != bytes_b {
                        return EquivResult::Counterexample {
                            path,
                            divergence: format!(
                                "subroutine call #{} ({arity} inline args) extended argument bytes differ at A@n{pc_a} vs B@n{pc_b}",
                                ins_a.opcode
                            ),
                        };
                    }
                }
                None => {
                    // Callee arity unknown: a callee with >4 inline args can read across multiple
                    // following instructions (`interpreter.rs:1542-1580`), so require the entire
                    // trailing instruction suffix after the call to match byte-for-byte.
                    let key =
                        |ins: &Instruction| (ins.opcode, ins.on_true, ins.on_false, ins.operands);
                    let tail_a = a.instructions.get(usize::from(pc_a) + 1..).unwrap_or(&[]);
                    let tail_b = b.instructions.get(usize::from(pc_b) + 1..).unwrap_or(&[]);
                    let suffix_matches = tail_a.len() == tail_b.len()
                        && tail_a
                            .iter()
                            .zip(tail_b.iter())
                            .all(|(x, y)| key(x) == key(y));
                    if !suffix_matches {
                        return EquivResult::Inconclusive {
                            reason: format!(
                                "subroutine call #{} at A@n{pc_a}/B@n{pc_b} has unknown callee arity and differing following instruction layout",
                                ins_a.opcode
                            ),
                        };
                    }
                }
            }
        }

        // Verify any frame-private operands read or written by this primitive agree on A and B.
        if let Err(err) =
            check_primitive_frame_reads(ins_a.opcode, &ins_a.operands, &state.env_a, &state.env_b)
        {
            return match err {
                FrameCheckErr::Divergence(msg) => EquivResult::Counterexample {
                    path,
                    divergence: format!("{msg} at A@n{pc_a} vs B@n{pc_b}"),
                },
                FrameCheckErr::Inconclusive(reason) => EquivResult::Inconclusive {
                    reason: format!("{reason} at A@n{pc_a} vs B@n{pc_b}"),
                },
            };
        }

        // Random (opcode 8) / GetDirectionTo (opcode 12): when the secondary read resolves to
        // Global 34, the VM redirects the store to Global 34 (`random.rs:38`, `direction.rs:65`),
        // so treat Global 34 / scope 22 aliasing as Inconclusive.
        if ins_a.opcode == 8 {
            let dst_scope = u16::from_le_bytes([ins_a.operands[2], ins_a.operands[3]]);
            let rng_scope = u16::from_le_bytes([ins_a.operands[6], ins_a.operands[7]]);
            let rng_idx = word_i16(&ins_a.operands, 4);
            if (rng_scope == 6 && rng_idx == 34) || dst_scope == 22 {
                return EquivResult::Inconclusive {
                    reason: format!(
                        "Random instruction at A@n{pc_a}/B@n{pc_b} uses unmodeled Global 34 / scope 22 aliasing"
                    ),
                };
            }
        } else if ins_a.opcode == 12 {
            let dst_scope = u16::from_le_bytes([ins_a.operands[2], ins_a.operands[3]]);
            let from_g34 = ins_a.operands[4] & 1 != 0
                && ins_a.operands[5] == 6
                && word_i16(&ins_a.operands, 6) == 34;
            if from_g34 || dst_scope == 22 {
                return EquivResult::Inconclusive {
                    reason: format!(
                        "GetDirectionTo instruction at A@n{pc_a}/B@n{pc_b} uses unmodeled Global 34 / scope 22 aliasing"
                    ),
                };
            }
        }

        // Any world-interacting step may mutate external memory; retain only pure frame constraints.
        let mut base_constraints = state.constraints.clone();
        base_constraints.retain(|pred, _| !pred.touches_mem());

        // Compute branch-specific frame-private updates (`Animate` only stores an event on `false`!).
        let mut env_true_a = state.env_a.clone();
        let mut env_true_b = state.env_b.clone();
        let mut env_false_a = state.env_a.clone();
        let mut env_false_b = state.env_b.clone();
        match ins_a.opcode {
            0 => {
                let p = word_i16(&ins_a.operands, 0);
                env_true_a.write(9, p, CanonOperand::Const(0));
                env_true_b.write(9, p, CanonOperand::Const(0));
                env_false_a.write(9, p, CanonOperand::Const(0));
                env_false_b.write(9, p, CanonOperand::Const(0));
            }
            17 => {
                let p = word_i16(&ins_a.operands, 0);
                let sym = interner.fresh_event();
                env_true_a.write(9, p, sym);
                env_true_b.write(9, p, sym);
                env_false_a.write(9, p, sym);
                env_false_b.write(9, p, sym);
            }
            8 | 12 => {
                let dst_scope = u16::from_le_bytes([ins_a.operands[2], ins_a.operands[3]]);
                let dst_idx = word_i16(&ins_a.operands, 0);
                if matches!(dst_scope, 9 | 25) {
                    let sym = interner.fresh_event();
                    env_true_a.write(dst_scope as u8, dst_idx, sym);
                    env_true_b.write(dst_scope as u8, dst_idx, sym);
                    env_false_a.write(dst_scope as u8, dst_idx, sym);
                    env_false_b.write(dst_scope as u8, dst_idx, sym);
                }
            }
            24 if ins_a.operands[0] == 0 => {
                // OldRelationship read writes `param[ops[2] as i8]` on `on_true` only.
                let sym = interner.fresh_event();
                env_true_a.write(9, i16::from(ins_a.operands[2] as i8), sym);
                env_true_b.write(9, i16::from(ins_b.operands[2] as i8), sym);
            }
            26 if ins_a.operands[2] & 4 == 0 => {
                // Relationship read writes into `(ops[4], word_i16(ops, 6))` on `on_true` only.
                let sym = interner.fresh_event();
                let idx_a = word_i16(&ins_a.operands, 6);
                let idx_b = word_i16(&ins_b.operands, 6);
                env_true_a.write(ins_a.operands[4], idx_a, sym);
                env_true_b.write(ins_b.operands[4], idx_b, sym);
            }
            31 if ins_a.operands[4] & 0x80 != 0 => {
                let sym = interner.fresh_event();
                env_true_a.write(ins_a.operands[5], i16::from(ins_a.operands[7]), sym);
                env_true_b.write(ins_b.operands[5], i16::from(ins_b.operands[7]), sym);
                env_false_a.write(ins_a.operands[5], i16::from(ins_a.operands[7]), sym);
                env_false_b.write(ins_b.operands[5], i16::from(ins_b.operands[7]), sym);
            }
            44 => {
                // `Animate` (`animate.rs:139-289`): `on_true` finishes without calling `store_event`;
                // `on_false` stores the event into `local[ops[2]]` (flag 0x20) or `param[0]`.
                let sym = interner.fresh_event();
                if ins_a.operands[5] & 0x20 != 0 {
                    env_false_a.write(25, i16::from(ins_a.operands[2]), sym);
                    env_false_b.write(25, i16::from(ins_b.operands[2]), sym);
                } else {
                    env_false_a.write(9, 0, sym);
                    env_false_b.write(9, 0, sym);
                }
            }
            _ => {}
        }

        let mut enqueue = |na: u8, nb: u8, ea: LocalEnv, eb: LocalEnv, branch: Option<bool>| {
            let next = ProductState {
                pc_a: na,
                pc_b: nb,
                env_a: ea,
                env_b: eb,
                constraints: base_constraints.clone(),
                last_differs: false,
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
            enqueue(ins_a.on_true, ins_b.on_true, env_true_a, env_true_b, None);
        } else {
            enqueue(
                ins_a.on_true,
                ins_b.on_true,
                env_true_a,
                env_true_b,
                Some(true),
            );
            enqueue(
                ins_a.on_false,
                ins_b.on_false,
                env_false_a,
                env_false_b,
                Some(false),
            );
        }
    }

    EquivResult::Equivalent {
        states_checked,
        paths_verified,
    }
}

fn check_expr_indirect_params(
    ins_a: &Instruction,
    ins_b: &Instruction,
    env_a: &LocalEnv,
    env_b: &LocalEnv,
) -> std::result::Result<(), String> {
    if ins_a.operands[6] == 22 || ins_b.operands[6] == 22 {
        let ia = word_i16(&ins_a.operands, 0);
        let ib = word_i16(&ins_b.operands, 0);
        if ins_a.operands[6] != ins_b.operands[6] || env_a.read(9, ia) != env_b.read(9, ib) {
            return Err("scope 22 LHS indirect parameter index divergence".to_owned());
        }
    }
    if ins_a.operands[7] == 22 || ins_b.operands[7] == 22 {
        let ia = word_i16(&ins_a.operands, 2);
        let ib = word_i16(&ins_b.operands, 2);
        if ins_a.operands[7] != ins_b.operands[7] || env_a.read(9, ia) != env_b.read(9, ib) {
            return Err("scope 22 RHS indirect parameter index divergence".to_owned());
        }
    }
    Ok(())
}

fn collect_inline_call_arg_bytes(bhav: &BhavChunk, call_pc: usize, need: usize) -> Option<Vec<u8>> {
    let call = bhav.instructions.get(call_pc)?;
    let mut buf = Vec::with_capacity(need.max(8));
    buf.extend_from_slice(&call.operands);
    for next in bhav.instructions.iter().skip(call_pc + 1) {
        if buf.len() >= need {
            break;
        }
        buf.extend_from_slice(&next.opcode.to_le_bytes());
        buf.push(next.on_true);
        buf.push(next.on_false);
        buf.extend_from_slice(&next.operands);
    }
    if buf.len() < need {
        None
    } else {
        buf.truncate(need);
        Some(buf)
    }
}

enum FrameCheckErr {
    Divergence(String),
    Inconclusive(String),
}

fn check_primitive_frame_reads(
    opcode: u16,
    ops: &[u8; 8],
    env_a: &LocalEnv,
    env_b: &LocalEnv,
) -> std::result::Result<(), FrameCheckErr> {
    let check_op = |scope: u8, idx: i16, label: &str| -> std::result::Result<(), FrameCheckErr> {
        let effective_scope = if scope == 22 { 9 } else { scope };
        if matches!(effective_scope, 9 | 25) {
            if env_a.has_slot(effective_scope, idx) != env_b.has_slot(effective_scope, idx) {
                return Err(FrameCheckErr::Divergence(format!(
                    "{label} frame-bounds divergence for scope {effective_scope}[{idx}]"
                )));
            }
            let va = env_a.read(effective_scope, idx);
            let vb = env_b.read(effective_scope, idx);
            if va != vb {
                return Err(FrameCheckErr::Divergence(format!(
                    "{label} value divergence: {va:?} != {vb:?}"
                )));
            }
        }
        Ok(())
    };
    let check_dst = |scope: u8, idx: i16, label: &str| -> std::result::Result<(), FrameCheckErr> {
        if scope == 22 {
            return check_op(9, idx, label);
        }
        if matches!(scope, 9 | 25) && env_a.has_slot(scope, idx) != env_b.has_slot(scope, idx) {
            return Err(FrameCheckErr::Divergence(format!(
                "{label} destination frame-bounds divergence for scope {scope}[{idx}]"
            )));
        }
        Ok(())
    };
    for frame_use in primitive_frame_uses(opcode, ops) {
        match frame_use {
            FrameUse::Read(scope, idx, label) => check_op(scope, idx, label)?,
            FrameUse::Dst(scope, idx, label) => check_dst(scope, idx, label)?,
            FrameUse::AllLocals(name) => {
                if env_a.local_count != env_b.local_count || env_a.locals != env_b.locals {
                    return Err(FrameCheckErr::Inconclusive(format!(
                        "{name} text formatting may read frame locals via $Local tokens and local state differs"
                    )));
                }
            }
            FrameUse::WholeFrame => {
                if env_a != env_b {
                    return Err(FrameCheckErr::Divergence(format!(
                        "frame-private state divergence before primitive opcode {opcode:#06x}"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// One frame-private operand a primitive touches (`sims-vm/src/primitives/*.rs`).
///
/// Shared by the lockstep checker ([`check_primitive_frame_reads`]) and the live-output
/// checker (`live`) so both model exactly the same per-primitive frame reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameUse {
    /// Reads `(scope, idx)`; scope 22 reads `param[idx]` as its indirection.
    Read(u8, i16, &'static str),
    /// Writes (or bounds-checks as a destination) `(scope, idx)`; scope 22 reads `param[idx]`.
    Dst(u8, i16, &'static str),
    /// Text formatting may read any frame local (`Dialog` / `ActionString` `$Local` tokens).
    AllLocals(&'static str),
    /// Unmodeled primitive: every frame-private slot may be read.
    WholeFrame,
}

/// Lists the frame-private operands primitive `opcode` reads or writes.
fn primitive_frame_uses(opcode: u16, ops: &[u8; 8]) -> Vec<FrameUse> {
    let mut uses = Vec::new();
    match opcode {
        0 => uses.push(FrameUse::Read(9, word_i16(ops, 0), "Sleep param")),
        1 | 3 | 4 | 5 | 6 | 7 | 9 | 10 | 18 | 20 | 21 | 22 | 23 | 27 | 28 | 30 | 33 | 48 | 49
        | 51 => {}
        8 => {
            let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
            if dst_scope <= 255 {
                uses.push(FrameUse::Dst(
                    dst_scope as u8,
                    word_i16(ops, 0),
                    "Random destination",
                ));
            }
            let rng_scope = u16::from_le_bytes([ops[6], ops[7]]);
            if rng_scope <= 255 {
                uses.push(FrameUse::Read(
                    rng_scope as u8,
                    word_i16(ops, 4),
                    "Random range",
                ));
            }
        }
        11 => {
            if ops[2] & 1 != 0 {
                uses.push(FrameUse::Read(
                    ops[3],
                    word_i16(ops, 4),
                    "GetDistanceTo from",
                ));
            }
        }
        12 => {
            let dst_scope = u16::from_le_bytes([ops[2], ops[3]]);
            if dst_scope <= 255 {
                uses.push(FrameUse::Dst(
                    dst_scope as u8,
                    word_i16(ops, 0),
                    "GetDirectionTo destination",
                ));
            }
            if ops[4] & 1 != 0 {
                uses.push(FrameUse::Read(
                    ops[5],
                    word_i16(ops, 6),
                    "GetDirectionTo from",
                ));
            }
        }
        13 => {
            if ops[3] & 2 != 0 {
                uses.push(FrameUse::Read(
                    25,
                    i16::from(ops[1]),
                    "PushInteraction object local",
                ));
            } else {
                uses.push(FrameUse::Read(
                    9,
                    i16::from(ops[1]),
                    "PushInteraction object param",
                ));
            }
            if ops[3] & 1 != 0 {
                uses.push(FrameUse::Read(
                    25,
                    i16::from(ops[4]),
                    "PushInteraction icon local",
                ));
            }
        }
        14 => {
            if ops[6] & 8 != 0 {
                let scope = u16::from_le_bytes([ops[4], ops[5]]);
                if scope <= 255 {
                    uses.push(FrameUse::Read(
                        scope as u8,
                        word_i16(ops, 2),
                        "FindBestObject origin",
                    ));
                }
            }
        }
        15 => {
            let scope = word_i16(ops, 2);
            if (0..=255).contains(&scope) {
                uses.push(FrameUse::Read(
                    scope as u8,
                    word_i16(ops, 0),
                    "Breakpoint cond",
                ));
            }
        }
        16 => {
            if ops[2] & 1 != 0 {
                uses.push(FrameUse::Read(25, i16::from(ops[1]), "FindLocation local"));
            }
        }
        17 => uses.push(FrameUse::Read(9, word_i16(ops, 0), "IdleForInput param")),
        19 => {
            uses.push(FrameUse::Read(
                25,
                i16::from(ops[0]),
                "MakeNewCharacter skin",
            ));
            uses.push(FrameUse::Read(
                25,
                i16::from(ops[1]),
                "MakeNewCharacter age",
            ));
            uses.push(FrameUse::Read(
                25,
                i16::from(ops[2]),
                "MakeNewCharacter sex",
            ));
        }
        24 => {
            if matches!(ops[3], 2 | 3) {
                uses.push(FrameUse::Read(9, 0, "OldRelationship param[0]"));
            }
            let p = i16::from(ops[2] as i8);
            uses.push(FrameUse::Dst(9, p, "OldRelationship param"));
            if ops[0] != 0 {
                uses.push(FrameUse::Read(9, p, "OldRelationship param"));
            }
        }
        25 => {
            let scope = match ops[0] {
                0 => 7,
                1 => 9,
                2 => 25,
                _ => ops[1],
            };
            uses.push(FrameUse::Read(
                scope,
                word_i16(ops, 2),
                "AlterBudget amount",
            ));
        }
        26 => {
            if matches!(ops[1], 2 | 3) {
                uses.push(FrameUse::Read(25, i16::from(ops[3]), "Relationship local"));
            }
            // `relationship.rs:179` resolves the operand in both read and write directions;
            // validate destination frame bounds even on read write-back (`ops[2] & 4 == 0`).
            uses.push(FrameUse::Dst(
                ops[4],
                word_i16(ops, 6),
                "Relationship value",
            ));
            if ops[2] & 4 != 0 || ops[4] == 22 {
                uses.push(FrameUse::Read(
                    ops[4],
                    word_i16(ops, 6),
                    "Relationship value",
                ));
            }
        }
        29 => {
            if ops[3] & 1 == 0 {
                uses.push(FrameUse::Read(
                    ops[0],
                    word_i16(ops, 4),
                    "SetMotiveChange delta",
                ));
                uses.push(FrameUse::Read(
                    ops[1],
                    word_i16(ops, 6),
                    "SetMotiveChange stop_at",
                ));
            }
        }
        31 => {
            if ops[4] & 0x80 != 0 {
                uses.push(FrameUse::Read(
                    ops[5],
                    i16::from(ops[7]),
                    "SetToNext destination cursor",
                ));
            }
            let base = ops[4] & 0x7F;
            if base == 6 {
                uses.push(FrameUse::Read(9, 0, "SetToNext param[0]"));
            } else if base == 9 {
                uses.push(FrameUse::Read(25, i16::from(ops[6]), "SetToNext local"));
            }
        }
        32 => uses.push(FrameUse::Read(
            ops[6],
            word_i16(ops, 4),
            "TestObjectType object",
        )),
        34 => {
            if ops[0] <= 2 {
                uses.push(FrameUse::Read(ops[1], word_i16(ops, 2), "UiEffect target"));
            }
        }
        35 => {
            if ops[4] & 0x20 != 0 {
                uses.push(FrameUse::Read(
                    25,
                    word_i16(ops, 0),
                    "SpecialEffect duration local",
                ));
            }
        }
        36 | 50 => {
            // Both `Dialog` (`dialog.rs:1498`) and `ActionString` (`action_string.rs:85`)
            // run `format_text`, which reads frame locals via `$Local` / `$NeighborLocal` /
            // `$TimeLocal` tokens (`dialog.rs:1605`). Without string tables loaded, require
            // identical local frame layout and values, otherwise return `Inconclusive`.
            uses.push(FrameUse::AllLocals(if opcode == 36 {
                "Dialog"
            } else {
                "ActionString"
            }));
        }
        37 => {
            if ops[0] == 1 {
                uses.push(FrameUse::Read(
                    ops[1],
                    word_i16(ops, 2),
                    "TestSimInteractingWith person",
                ));
            }
        }
        41 => {
            if ops[3] == 7 && ops[2] == 2 {
                uses.push(FrameUse::Read(
                    25,
                    word_i16(ops, 0) >> 1,
                    "SetHeadline local",
                ));
            }
        }
        42 => {
            if ops[4] == 7 {
                uses.push(FrameUse::Read(9, 0, "CreateObject param[0]"));
            } else if ops[4] == 8 || ops[4] == 9 {
                // `create.rs:177` sign-extends `operands[6] as i8`.
                uses.push(FrameUse::Read(
                    25,
                    i16::from(ops[6] as i8),
                    "CreateObject local",
                ));
            }
        }
        43 => {
            if word_i16(ops, 0) != 0 {
                uses.push(FrameUse::Read(9, word_i16(ops, 2), "DropOnto src param"));
            }
            if word_i16(ops, 4) != 0 {
                uses.push(FrameUse::Read(9, word_i16(ops, 6), "DropOnto dst param"));
            }
        }
        44 => {
            let clip_id = word_i16(ops, 0);
            let flags = ops[5];
            let can_emit_events =
                matches!(flags & 0x11, 0x00 | 0x11) && (flags & 0x04 != 0 || clip_id != 0);
            if flags & 0x04 != 0 {
                uses.push(FrameUse::Read(9, clip_id, "Animate clip param"));
            }
            if flags & 0x20 != 0 {
                uses.push(FrameUse::Dst(25, i16::from(ops[2]), "Animate event local"));
            } else if can_emit_events {
                uses.push(FrameUse::Dst(9, 0, "Animate event param[0]"));
            }
        }
        45 => {
            if u16::from_le_bytes([ops[2], ops[3]]) == 0 {
                uses.push(FrameUse::Read(9, word_i16(ops, 0), "RouteTo param"));
            }
        }
        46 => {
            if u16::from_le_bytes([ops[2], ops[3]]) == 0 {
                uses.push(FrameUse::Read(9, word_i16(ops, 0), "Snap param"));
            }
        }
        47 => {
            if word_i16(ops, 0) == 1 {
                uses.push(FrameUse::Read(9, word_i16(ops, 4), "Reach slot param"));
            }
        }
        id if id >= 256 => {}
        // Conservative soundness guard for any unmodeled primitive opcode.
        _ => uses.push(FrameUse::WholeFrame),
    }
    uses
}

fn format_pc(pc: u8) -> String {
    match pc {
        TARGET_ERROR => "error".to_owned(),
        TARGET_TRUE => "return true".to_owned(),
        TARGET_FALSE => "return false".to_owned(),
        n => format!("node {n}"),
    }
}

/// Convenience helper for CLI `sims-antics equiv`: parses a raw IFF file in memory,
/// extracts a single [`BhavChunk`] by tree ID, and also collects all BHAV arities present
/// in that IFF for sound inline-call checking.
pub fn extract_bhav_from_iff(iff_bytes: &[u8], tree_id: u16) -> Result<BhavChunk> {
    let iff = Iff::parse(iff_bytes.to_vec())?;
    let chunk = iff
        .get(b"BHAV", tree_id)
        .ok_or_else(|| Error::new(format!("IFF does not contain BHAV #{tree_id}")))?;
    crate::lift::lift_bhav(chunk.id, &chunk.label, iff.data(chunk))
}

/// Which checker proved (or refuted) equivalence in [`check_bhav_equivalent_combined`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquivMethod {
    /// The node-for-node lockstep bisimulation ([`check_bhav_equivalent_with_arities`]).
    Lockstep,
    /// The symbolic live-output checker ([`check_bhav_live_equivalent`]).
    LiveOutput,
}

impl std::fmt::Display for EquivMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Lockstep => "lockstep",
            Self::LiveOutput => "live-output",
        })
    }
}

/// Merges two files' callee arity maps, keeping only the trees both agree on (or that
/// only one of them knows).
fn merge_arities(
    a: Option<&BTreeMap<u16, usize>>,
    b: Option<&BTreeMap<u16, usize>>,
) -> Option<BTreeMap<u16, usize>> {
    match (a, b) {
        (None, None) => None,
        (Some(m), None) | (None, Some(m)) => Some(m.clone()),
        (Some(a), Some(b)) => {
            let mut merged = a.clone();
            for (&id, &arity) in b {
                if merged.get(&id).is_some_and(|&other| other != arity) {
                    merged.remove(&id);
                } else {
                    merged.insert(id, arity);
                }
            }
            Some(merged)
        }
    }
}

/// Why two trees cannot be compared as if they ran against the same game data: their
/// files give them different tuning constants (`BCON`), which neither checker can see
/// through when a primitive or an unfolded operand reads them. `None` when the files
/// agree on everything the trees may read.
#[must_use]
pub fn context_mismatch(
    a: &BhavChunk,
    ctx_a: &LiveContext,
    b: &BhavChunk,
    ctx_b: &LiveContext,
) -> Option<String> {
    let may_read_constants = |bhav: &BhavChunk| {
        bhav.instructions
            .iter()
            .any(|ins| ins.opcode != 2 || ins.operands[6] == 26 || ins.operands[7] == 26)
    };
    (ctx_a.local_bcons != ctx_b.local_bcons && (may_read_constants(a) || may_read_constants(b)))
        .then(|| "the files' constant tables (BCON) differ and the trees may read them".to_owned())
}

/// Whether some path through `bhav` may read a local before writing it.
///
/// A forward must-be-written analysis: only `:=` (op 5) and op 20, which mark the
/// local written before reading it (`expression.rs:75`), count as writes; other
/// operators read the left side first. Primitive writes are not counted, so the
/// answer errs towards "may". Every node of a kind-4 tree is an entry point.
fn may_read_unwritten_local(bhav: &BhavChunk) -> bool {
    let locals = live::frame_dims(bhav).1;
    if locals == 0 {
        return false;
    }
    let local = |idx: i16| usize::try_from(idx).ok().filter(|&i| i < locals);
    let len = bhav.instructions.len().min(251);
    let all = vec![true; locals];
    // `written[n]`: locals written on every path reaching node `n` (None: unreached).
    let mut written: Vec<Option<Vec<bool>>> = vec![None; len];
    let mut work: Vec<usize> = Vec::new();
    for (n, w) in written.iter_mut().enumerate() {
        if n == 0 || bhav.kind == 4 {
            *w = Some(vec![false; locals]);
            work.push(n);
        }
    }
    while let Some(n) = work.pop() {
        let ins = &bhav.instructions[n];
        let mut out = written[n].clone().unwrap_or_else(|| all.clone());
        if ins.opcode == 2 {
            let ops = &ins.operands;
            let (lhs, rhs) = (word_i16(ops, 0), word_i16(ops, 2));
            let op = ops[5];
            if ops[7] == 25
                && let Some(i) = local(rhs)
                && !out[i]
            {
                return true;
            }
            if ops[6] == 25
                && let Some(i) = local(lhs)
            {
                if matches!(op, 5 | 20) {
                    out[i] = true;
                } else if !out[i] {
                    return true;
                }
            }
        } else {
            for u in primitive_frame_uses(ins.opcode, &ins.operands) {
                match u {
                    FrameUse::Read(25, idx, _) => {
                        if local(idx).is_some_and(|i| !out[i]) {
                            return true;
                        }
                    }
                    FrameUse::AllLocals(_) | FrameUse::WholeFrame if out.iter().any(|w| !w) => {
                        return true;
                    }
                    _ => {}
                }
            }
        }
        for target in [ins.on_true, ins.on_false] {
            let t = usize::from(target);
            if t >= len {
                continue;
            }
            let merged = match &written[t] {
                None => out.clone(),
                Some(prev) => prev.iter().zip(&out).map(|(&p, &o)| p && o).collect(),
            };
            if written[t].as_ref() != Some(&merged) {
                written[t] = Some(merged);
                work.push(t);
            }
        }
    }
    false
}

/// Whether a lockstep proof (with silent steps off) also covers everything the
/// live-output checker observes under `live`, faults included.
fn lockstep_suffices(
    a: &BhavChunk,
    ctx_a: &LiveContext,
    b: &BhavChunk,
    ctx_b: &LiveContext,
    live: &LiveOptions,
    trace: &LockstepTrace,
) -> bool {
    // Any node may fault (a trap, the error target, a bad branch), and a fault leaves
    // the faulting frame on the stack at its node with the thread's last result
    // (`interpreter.rs:809`). So the proof stands only if every matched pair is the
    // same node on both sides, the frames have the same layout, and the last results
    // never differ while a node runs. With silent steps off, lockstep matches every
    // frame write (expression destinations and primitive operands), so equal layouts
    // also end with equal contents, which covers frame residue.
    if trace.pairs.iter().any(|&(pa, pb)| pa != pb) {
        return false;
    }
    // Lockstep matches what nodes do, not how: two nodes that compute the same thing
    // with different operands can still fault differently, or leave different partial
    // state when they fault (`expression.rs:69`). So every node it ran must be the same
    // bytes on both sides, and a call the same inline arguments after it too. Then the
    // two runs are the same run.
    // A call to a tree that reads none of its operand bytes is the same run with any of
    // them ([`call_operands`]), and has no inline arguments after it.
    let argless = shared_argless(ctx_a, ctx_b);
    let argless = argless.as_ref();
    let bytes = |ins: &Instruction| (ins.opcode, ins.on_true, ins.on_false, ins.operands);
    let used = |ins: &Instruction| {
        let ops = call_operands(ins, argless);
        (ins.opcode, ins.on_true, ins.on_false, ops)
    };
    for p in trace.pairs.iter().map(|&(p, _)| usize::from(p)) {
        let (Some(ia), Some(ib)) = (a.instructions.get(p), b.instructions.get(p)) else {
            return false;
        };
        if used(ia) != used(ib) {
            return false;
        }
        if ia.opcode >= 256
            && !is_argless_call(ia, argless)
            && !a.instructions[p..]
                .iter()
                .map(bytes)
                .eq(b.instructions[p..].iter().map(bytes))
        {
            return false;
        }
    }
    if live.observe_last_result && trace.last_differs_mid_run {
        return false;
    }
    if live::frame_dims(a) != live::frame_dims(b) {
        return false;
    }
    // A fault names the tree, a jump to a missing node carries the tree's length
    // (`interpreter.rs:1484`), and the VM enters a tree by its format and kind.
    if (a.id, a.format, a.kind, a.instructions.len())
        != (b.id, b.format, b.kind, b.instructions.len())
    {
        return false;
    }
    // Lockstep runs nodes past 250 and branches to 251 and 252, which the VM rejects
    // (`interpreter.rs:766`, `interpreter.rs:1695`).
    let past_last_node = |bhav: &BhavChunk| {
        bhav.instructions.len() > 251
            || bhav
                .instructions
                .iter()
                .any(|ins| matches!(ins.on_true, 251 | 252) || matches!(ins.on_false, 251 | 252))
    };
    if past_last_node(a) || past_last_node(b) {
        return false;
    }
    // Reading a local the tree has not written yet can fail in the VM
    // (`interpreter.rs:1277`), but lockstep folds such reads (`local[0] == local[0]`
    // becomes `true`). Leave trees that may do so to the live-output checker, which
    // records those reads.
    if may_read_unwritten_local(a) || may_read_unwritten_local(b) {
        return false;
    }
    // A call whose target has different argument counts in the two files runs
    // different trees; lockstep only sees the call.
    let arity =
        |ctx: &LiveContext, id: u16| ctx.callee_arities.as_ref().map(|m| m.get(&id).copied());
    !a.instructions
        .iter()
        .chain(&b.instructions)
        .filter(|ins| ins.opcode >= 256)
        .any(|ins| arity(ctx_a, ins.opcode) != arity(ctx_b, ins.opcode))
}

/// The callees both sides' files say read no call operand bytes
/// ([`LiveContext::argless_callees`]).
fn shared_argless(ctx_a: &LiveContext, ctx_b: &LiveContext) -> Option<BTreeSet<u16>> {
    let (a, b) = (ctx_a.argless_callees.as_ref()?, ctx_b.argless_callees.as_ref()?);
    Some(a.intersection(b).copied().collect())
}

/// Checks two trees with the lockstep checker first and, when it cannot prove them
/// equivalent, with the live-output checker, which also accepts trees that differ by
/// pure reads and dummy nodes (and dead frame writes, with frame residue ignored).
///
/// Lockstep runs with silent steps off (so every `Expression` is matched, which keeps
/// the scratch pointer and uninitialized-local reads in step) and, when `live` observes
/// it, with the last branch result compared. Its proof is taken only when it covers
/// what `live` observes ([`lockstep_suffices`]): among other things it must have paired
/// every node with the node of the same number, since a fault leaves the node number
/// behind. A lockstep counterexample is a divergence in the stricter node-for-node
/// trace and is never reported; only a live-output counterexample, which a `sims-vm`
/// replay has confirmed, is. Otherwise the result is `Inconclusive`.
#[must_use]
pub fn check_bhav_equivalent_combined(
    a: &BhavChunk,
    ctx_a: &LiveContext,
    b: &BhavChunk,
    ctx_b: &LiveContext,
    lockstep: &EquivOptions,
    live: &LiveOptions,
) -> (EquivMethod, EquivResult) {
    if let Some(reason) = context_mismatch(a, ctx_a, b, ctx_b) {
        return (
            EquivMethod::LiveOutput,
            EquivResult::Inconclusive { reason },
        );
    }
    let arities = merge_arities(ctx_a.callee_arities.as_ref(), ctx_b.callee_arities.as_ref());
    let strict = EquivOptions {
        max_silent_steps: 0,
        observe_last_result: live.observe_last_result,
        ..*lockstep
    };
    let mut trace = LockstepTrace::default();
    let argless = shared_argless(ctx_a, ctx_b);
    let first = lockstep_traced(
        a,
        b,
        &strict,
        arities.as_ref(),
        argless.as_ref(),
        &mut trace,
    );
    if matches!(first, EquivResult::Equivalent { .. })
        && lockstep_suffices(a, ctx_a, b, ctx_b, live, &trace)
    {
        return (EquivMethod::Lockstep, first);
    }
    match check_bhav_live_equivalent(a, ctx_a, b, ctx_b, live) {
        EquivResult::Inconclusive { reason } => {
            let lockstep_note = match first {
                EquivResult::Counterexample { divergence, .. } => {
                    format!("lockstep counterexample: {divergence}")
                }
                EquivResult::Inconclusive { reason } => format!("lockstep: {reason}"),
                EquivResult::Equivalent { .. } => "lockstep: equivalent node for node, but that \
                     does not cover renumbered nodes (where a fault would differ), frame layout, \
                     invalid nodes or differing callee arities here"
                    .to_owned(),
            };
            (
                EquivMethod::LiveOutput,
                EquivResult::Inconclusive {
                    reason: format!("{reason} ({lockstep_note})"),
                },
            )
        }
        other => (EquivMethod::LiveOutput, other),
    }
}

/// Extracts `(tree_id -> arg_count)` for every valid BHAV chunk inside a raw IFF.
pub fn extract_bhav_arities_from_iff(iff_bytes: &[u8]) -> Result<BTreeMap<u16, usize>> {
    let iff = Iff::parse(iff_bytes.to_vec())?;
    let mut arities = BTreeMap::new();
    for chunk in iff.all(b"BHAV") {
        if chunk.flags & DELETED != 0 {
            continue;
        }
        if let Ok(bhav) = Behavior::parse(iff.data(chunk)) {
            arities.insert(chunk.id, bhav_param_limit(&bhav));
        }
    }
    Ok(arities)
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

    #[test]
    fn sol_round1_verifier_soundness_regressions() {
        // 1. Opcode 38 (unimplemented in sims-vm), reachable TARGET_ERROR, invalid scope 255,
        //    and dynamic divisor `/` must all be rejected by the safety prover.
        let manifest = r#"
            [object]
            name = "TrapTester"
            guid = 0x7F00_9992
            init = "Init"
            main = "Main"
            scripts = ["behavior.antics"]
        "#;
        let trap_script = r#"
            tree 4096 "Init" { return true; }
            tree 4097 "Main" (args: 1) {
                op 0x0026 (0, 0, 0, 0, 0, 0, 0, 0) -> true: 1, false: error;
                op 0x0002 (0, 0, 0, 0, 0, 7, 0, 9) -> true: 2, false: true;
                op 0x0002 (0, 0, 0, 0, 0, 2, 255, 7) -> true: true, false: false;
            }
        "#;
        let compiled =
            compile_object_from_sources(manifest, &[("behavior.antics", trap_script)]).unwrap();
        let report = verify_object_safety(&compiled).unwrap();
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::InvalidOpcode),
            "opcode 38 must be rejected: {:?}",
            report.violations
        );
        let trap_count = report
            .violations
            .iter()
            .filter(|v| v.category == SafetyCategory::PotentialTrap)
            .count();
        assert!(
            trap_count >= 3,
            "expected >=3 PotentialTrap violations (TARGET_ERROR, dynamic div, scope 255), got {trap_count}: {:?}",
            report.violations
        );

        // 2. Unknown-value write to a guarded lock attribute followed by priority==0 cancel exit
        //    must be caught as LockLeak!
        let unknown_lock_manifest = r#"
            [object]
            name = "UnknownLockObj"
            guid = 0x7F00_9993
            init = "Init"
            main = "Main"
            scripts = ["behavior.antics"]

            [[interaction]]
            name = "Use"
            action = "Use"
            guard = "CanUse"
        "#;
        let unknown_lock_script = r#"
            tree 4096 "Init" { my.attr[0] := 0; return true; }
            tree 4097 "Main" { return true; }
            tree 4098 "CanUse" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            interaction 4099 "Use" (args: 1) {
                stack.attr[0] := param[0];
                if my.person.priority == 0 {
                    return false;
                }
                stack.attr[0] := 0;
                return true;
            }
        "#;
        let ul_obj = compile_object_from_sources(
            unknown_lock_manifest,
            &[("behavior.antics", unknown_lock_script)],
        )
        .unwrap();
        let ul_report = verify_object_safety(&ul_obj).unwrap();
        assert!(
            ul_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "unknown write to lock attr + cancel exit must trigger LockLeak: {:?}",
            ul_report.violations
        );

        // 3. `0 == 0` before a Global 34 RHS read must NOT be silently erased (scratch ptr init!).
        let g34_a = &compile_antics(
            r#"
            tree 4096 "WithInit" {
                op 0x0002 (0, 0, 0, 0, 0, 2, 7, 7) -> true: 1, false: 1;
                op 0x0002 (0, 0, 34, 0, 0, 5, 8, 6) -> true: true, false: true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let g34_b = &compile_antics(
            r#"
            tree 4096 "WithoutInit" {
                op 0x0002 (0, 0, 34, 0, 0, 5, 8, 6) -> true: true, false: true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(g34_a, g34_b, &EquivOptions::default()),
                EquivResult::Counterexample { .. }
            ),
            "0==0 before Global 34 read must not be folded away"
        );

        // 4. Scope 22 (`stack.attr[param[0]]`) write must NOT be silently erased as frame-private!
        let s22_a = &compile_antics(
            r#"
            tree 4096 "Scope22Write" (args: 1) {
                op 0x0002 (0, 0, 1, 0, 0, 5, 22, 7) -> true: true, false: true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let s22_b = &compile_antics(
            r#"
            tree 4096 "EmptyReturn" (args: 1) {
                return true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(s22_a, s22_b, &EquivOptions::default()),
                EquivResult::Counterexample { .. }
            ),
            "scope 22 write must not be erased as frame-private"
        );

        // 5. Animate `on_true` branch must NOT fabricate equality for pre-existing local values!
        let anim_a = &compile_antics(
            r#"
            tree 4096 "AnimA" (locals: 1) {
                op 0x0002 (0, 0, 1, 0, 0, 5, 25, 7) -> true: 1, false: 1;
                op 0x002c (0, 0, 0, 0, 0, 32, 0, 0) -> true: 2, false: false;
                op 0x0002 (0, 0, 0, 0, 0, 5, 8, 25) -> true: true, false: true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let anim_b = &compile_antics(
            r#"
            tree 4096 "AnimB" (locals: 1) {
                op 0x0002 (0, 0, 2, 0, 0, 5, 25, 7) -> true: 1, false: 1;
                op 0x002c (0, 0, 0, 0, 0, 32, 0, 0) -> true: 2, false: false;
                op 0x0002 (0, 0, 0, 0, 0, 5, 8, 25) -> true: true, false: true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(anim_a, anim_b, &EquivOptions::default()),
                EquivResult::Counterexample { .. }
            ),
            "Animate on_true does not overwrite local[0]; divergent values must be caught"
        );

        // 6. Kind-4 indexed tree entry mismatch must be caught!
        let k4_a = &compile_antics(
            r#"
            tree 4096 "Kind4A" (kind: 4) {
                op 0x0002 (0, 0, 1, 0, 0, 5, 8, 7) -> true: true, false: true;
                op 0x0002 (0, 0, 2, 0, 0, 5, 8, 7) -> true: true, false: true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let k4_b = &compile_antics(
            r#"
            tree 4096 "Kind4B" (kind: 4) {
                op 0x0002 (0, 0, 1, 0, 0, 5, 8, 7) -> true: true, false: true;
                op 0x0002 (0, 0, 9, 0, 0, 5, 8, 7) -> true: true, false: true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(k4_a, k4_b, &EquivOptions::default()),
                EquivResult::Counterexample { .. }
            ),
            "kind-4 indexed entry divergence at node 1 must be caught"
        );
    }

    #[test]
    fn sol_round2_verifier_soundness_regressions() {
        // 1. Scope 2, write to read-only global[0], and unsupported Generic Sims Call routine 99
        //    must all be flagged by the no-trap checker.
        let trap_manifest = r#"
            [object]
            name = "Round2TrapObj"
            guid = 0x7F00_9994
            init = "Init"
            main = "Main"
            scripts = ["behavior.antics"]
        "#;
        let trap_script = r#"
            tree 4096 "Init" { return true; }
            tree 4097 "Main" {
                op 0x0002 (0, 0, 0, 0, 0, 2, 2, 7) -> true: 1, false: 1;
                op 0x0002 (0, 0, 1, 0, 0, 5, 6, 7) -> true: 2, false: 2;
                op 0x0001 (99, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: false;
            }
        "#;
        let trap_obj =
            compile_object_from_sources(trap_manifest, &[("behavior.antics", trap_script)])
                .unwrap();
        let trap_report = verify_object_safety(&trap_obj).unwrap();
        let trap_violations: Vec<_> = trap_report
            .violations
            .iter()
            .filter(|v| v.category == SafetyCategory::PotentialTrap)
            .collect();
        assert!(
            trap_violations.len() >= 3,
            "expected >=3 PotentialTrap violations (scope 2, write global[0], generic call 99), got {:?}",
            trap_report.violations
        );

        // 2. Animate on_false event overwriting a cleanup-controlling local must be caught as LockLeak.
        let anim_lock_manifest = r#"
            [object]
            name = "AnimLeakObj"
            guid = 0x7F00_9995
            init = "Init"
            main = "Main"
            scripts = ["behavior.antics"]

            [[interaction]]
            name = "Use"
            action = "Use"
            guard = "CanUse"
        "#;
        let anim_lock_script = r#"
            tree 4096 "Init" { my.attr[0] := 0; return true; }
            tree 4097 "Main" { return true; }
            tree 4098 "CanUse" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            interaction 4099 "Use" (locals: 1) {
                op 0x0002 (0, 0, 1, 0, 0, 5, 1, 7) -> true: 1, false: 1;
                op 0x0002 (0, 0, 0, 0, 0, 5, 25, 7) -> true: 2, false: 2;
                op 0x002c (0, 0, 0, 0, 0, 32, 0, 0) -> true: 3, false: 3;
                op 0x0002 (0, 0, 0, 0, 0, 2, 25, 7) -> true: 4, false: false;
                op 0x0002 (0, 0, 0, 0, 0, 5, 1, 7) -> true: true, false: true;
            }
        "#;
        let anim_obj = compile_object_from_sources(
            anim_lock_manifest,
            &[("behavior.antics", anim_lock_script)],
        )
        .unwrap();
        let anim_report = verify_object_safety(&anim_obj).unwrap();
        assert!(
            anim_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "Animate on_false mutating cleanup-controlling local[0] must trigger LockLeak: {:?}",
            anim_report.violations
        );

        // 3. Run Tree By Name (opcode 28) disables closed-caller param inference so helper div is rejected.
        let named_manifest = r#"
            [object]
            name = "NamedDivObj"
            guid = 0x7F00_9996
            init = "Init"
            main = "Main"
            scripts = ["behavior.antics"]
        "#;
        let named_script = r#"
            tree 4096 "Init" { return true; }
            tree 4097 "Main" {
                op 0x1002 (1, 0, 0, 0, 0, 0, 0, 0) -> true: 1, false: 1;
                op 0x001c (0, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: false;
            }
            tree 4098 "Helper" (args: 1) {
                op 0x0002 (0, 0, 0, 0, 0, 7, 8, 9) -> true: true, false: false;
            }
        "#;
        let named_obj =
            compile_object_from_sources(named_manifest, &[("behavior.antics", named_script)])
                .unwrap();
        let named_report = verify_object_safety(&named_obj).unwrap();
        assert!(
            named_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::PotentialTrap && v.tree_id == Some(4098)),
            "RunTreeByName must disable closed-caller divisor inference on Helper #4098: {:?}",
            named_report.violations
        );

        // 4. `0 == 0` before UI Effect (opcode 34) must NOT be silently folded away!
        let ui_a = &compile_antics(
            r#"
            tree 4096 "WithUiInit" {
                op 0x0002 (0, 0, 0, 0, 0, 2, 7, 7) -> true: 1, false: 1;
                op 0x0022 (0, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let ui_b = &compile_antics(
            r#"
            tree 4096 "WithoutUiInit" {
                op 0x0022 (0, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(ui_a, ui_b, &EquivOptions::default()),
                EquivResult::Counterexample { .. }
            ),
            "0 == 0 before UI Effect (opcode 34) must not be silently folded"
        );

        // 5. Scope-22 indirect predicate must NOT be falsely pruned after param[0] mutates.
        let s22_pred_a = &compile_antics(
            r#"
            tree 4096 "S22PredA" (args: 1) {
                op 0x0002 (0, 0, 0, 0, 0, 2, 22, 7) -> true: 1, false: false;
                op 0x0002 (0, 0, 1, 0, 0, 5, 9, 7) -> true: 2, false: 2;
                op 0x0002 (0, 0, 0, 0, 0, 2, 22, 7) -> true: true, false: false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let s22_pred_b = &compile_antics(
            r#"
            tree 4096 "S22PredB" (args: 1) {
                op 0x0002 (0, 0, 0, 0, 0, 2, 22, 7) -> true: 1, false: false;
                op 0x0002 (0, 0, 1, 0, 0, 5, 9, 7) -> true: true, false: true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(s22_pred_a, s22_pred_b, &EquivOptions::default()),
                EquivResult::Counterexample { .. }
            ),
            "scope-22 predicate after param[0] write must not be cached as redundant"
        );

        // 6. Unknown-arity inline call with divergent pc+2 suffix must return Inconclusive.
        let wide_a = &compile_antics(
            r#"
            tree 4096 "WideA" {
                op 0x0100 (1, 0, 2, 0, 3, 0, 4, 0) -> true: true, false: false;
                op 0x0002 (0, 0, 0, 0, 0, 2, 7, 7) -> true: true, false: false;
                op 0x0002 (1, 0, 0, 0, 0, 2, 7, 7) -> true: true, false: false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let wide_b = &compile_antics(
            r#"
            tree 4096 "WideB" {
                op 0x0100 (1, 0, 2, 0, 3, 0, 4, 0) -> true: true, false: false;
                op 0x0002 (0, 0, 0, 0, 0, 2, 7, 7) -> true: true, false: false;
                op 0x0002 (9, 0, 0, 0, 0, 2, 7, 7) -> true: true, false: false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(wide_a, wide_b, &EquivOptions::default()),
                EquivResult::Inconclusive { .. }
            ),
            "unknown-arity inline call with divergent pc+2 suffix must return Inconclusive"
        );
    }

    #[test]
    fn sol_wave1_r1_soundness_regressions() {
        let base_manifest = r#"
[object]
name = "SolWave1Probe"
guid = 0x50110001
init = "Init"
main = "Init"
scripts = ["behavior.antics"]

[[interaction]]
name = "Use"
action = "Action"
guard = "Guard"
"#;

        // P1-2: Relationship read writing into `stack.attr[0]` after release must be caught as LockLeak!
        let rel_leak_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                stack.attr[0] := 0;
                relationship(column: 0, dir: 0, flags: 0, value: stack.attr[0]);
                return true;
            }
        "#;
        let rel_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", rel_leak_script)])
                .unwrap();
        let rel_report = verify_object_safety(&rel_obj).unwrap();
        assert!(
            rel_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "Relationship read into stack.attr[0] must be caught as LockLeak: {:?}",
            rel_report.violations
        );

        // P1-3: Run Tree By Name (opcode 28) calling a private helper that reacquires mutex
        // via local STR# 303 must be caught as LockLeak; unresolved global/external must be Inconclusive.
        let rtb_unresolved_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                stack.attr[0] := 0;
                run_tree_by_name(table: 303, index: 1, mode: 2, global: true);
                return true;
            }
        "#;
        let rtb_obj = compile_object_from_sources(
            base_manifest,
            &[("behavior.antics", rtb_unresolved_script)],
        )
        .unwrap();
        let rtb_report = verify_object_safety(&rtb_obj).unwrap();
        assert!(
            rtb_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::InconclusiveProof),
            "unresolved Run Tree By Name must return InconclusiveProof: {:?}",
            rtb_report.violations
        );

        // P1-4: FindBestObject false branch leaves `stack_obj` unchanged (InitialTarget),
        // so acquiring `stack.attr[0] := 1` on its false branch must be caught as LockLeak!
        let fbo_false_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                stack.attr[0] := 0;
                if find_best_object(0, 0) {
                    return true;
                } else {
                    stack.attr[0] := 1;
                    return false;
                }
            }
        "#;
        let fbo_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", fbo_false_script)])
                .unwrap();
        let fbo_report = verify_object_safety(&fbo_obj).unwrap();
        assert!(
            fbo_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "FindBestObject false branch keeps InitialTarget and must catch LockLeak: {:?}",
            fbo_report.violations
        );

        // P1-5: Refresh with invalid selector between acquire and release traps (`error` exit)
        // and must be caught as BOTH PotentialTrap AND LockLeak (not skipped as trap-free!).
        let trap_leak_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                refresh(target: 9, mode: 0);
                stack.attr[0] := 0;
                return true;
            }
        "#;
        let trap_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", trap_leak_script)])
                .unwrap();
        let trap_report = verify_object_safety(&trap_obj).unwrap();
        assert!(
            trap_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "trapping Refresh between acquire and release must be caught as LockLeak: {:?}",
            trap_report.violations
        );
        assert!(
            trap_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::PotentialTrap),
            "invalid Refresh selector must be caught as PotentialTrap: {:?}",
            trap_report.violations
        );

        // P1-6: Equivalence must reject identical Relationship read into `local[0]` when
        // one tree declares `locals: 0` (traps) and the other declares `locals: 1` (succeeds).
        let rel_oob_a = &compile_antics(
            r#"
            tree 4096 "RelOobA" (format: 0x8002, args: 0, locals: 0) {
                op 0x001a (0, 0, 0, 0, 25, 0, 0, 0) -> true: true, false: false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let rel_oob_b = &compile_antics(
            r#"
            tree 4096 "RelOobB" (format: 0x8002, args: 0, locals: 1) {
                op 0x001a (0, 0, 0, 0, 25, 0, 0, 0) -> true: true, false: false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(rel_oob_a, rel_oob_b, &EquivOptions::default()),
                EquivResult::Counterexample { .. }
            ),
            "out-of-frame Relationship destination (locals: 0 vs 1) must be Counterexample"
        );

        // P1-6 (Animate): Animate event destination `param[0]` with `events: 0` when args differ (0 vs 1).
        let anim_oob_a = &compile_antics(
            r#"
            tree 4096 "AnimA" (format: 0x8002, args: 0, locals: 0) {
                op 0x002c (1, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let anim_oob_b = &compile_antics(
            r#"
            tree 4096 "AnimB" (format: 0x8002, args: 1, locals: 0) {
                op 0x002c (1, 0, 0, 0, 0, 0, 0, 0) -> true: true, false: false;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(anim_oob_a, anim_oob_b, &EquivOptions::default()),
                EquivResult::Counterexample { .. }
            ),
            "foreground Animate with events: 0 and args: 0 vs 1 must be Counterexample"
        );

        // P1-7: Dialog (opcode 36) with differing local initialization must NOT report Equivalent!
        let dlg_a = &compile_antics(
            r#"
            tree 4096 "DlgInit" (format: 0x8002, args: 0, locals: 1) {
                local[0] := 42;
                dialog(message: 1, title: 0, kind: 0, flags: 0);
                return true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        let dlg_b = &compile_antics(
            r#"
            tree 4096 "DlgUninit" (format: 0x8002, args: 0, locals: 1) {
                dialog(message: 1, title: 0, kind: 0, flags: 0);
                return true;
            }
        "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(dlg_a, dlg_b, &EquivOptions::default()),
                EquivResult::Inconclusive { .. }
            ),
            "Dialog with divergent local[0] state must return Inconclusive, not Equivalent"
        );

        // P2-1: Static checks catch RouteTo source 0 OOB param, FindLocation mode 3 without flag 0,
        // FindBestObject origin scope > 255, and CreateObject mode 8 signed local byte 128 (`-128`).
        let p2_static_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" { return true; }
            tree 4097 "Action" (args: 0, locals: 200) {
                op 0x002d (0, 0, 0, 0, 0, 0, 0, 0) -> true: 1, false: 1;
                op 0x0010 (3, 0, 0, 0, 0, 0, 0, 0) -> true: 2, false: 2;
                op 0x000e (0, 0, 0, 0, 0, 1, 8, 0) -> true: 3, false: 3;
                op 0x002a (0, 0, 0, 0, 8, 0, 128, 0) -> true: true, false: false;
            }
        "#;
        let p2_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", p2_static_script)])
                .unwrap();
        let p2_report = verify_object_safety(&p2_obj).unwrap();
        assert!(
            p2_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::InvalidParam && v.node == Some(0)),
            "RouteTo source 0 OOB param must be caught: {:?}",
            p2_report.violations
        );
        assert!(
            p2_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::PotentialTrap && v.node == Some(1)),
            "FindLocation mode 3 without flag bit 0 must be caught: {:?}",
            p2_report.violations
        );
        assert!(
            p2_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::PotentialTrap && v.node == Some(2)),
            "FindBestObject origin scope > 255 must be caught: {:?}",
            p2_report.violations
        );
        assert!(
            p2_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::InvalidLocal && v.node == Some(3)),
            "CreateObject mode 8 local byte 128 (signed -128) must be caught: {:?}",
            p2_report.violations
        );

        // P2-2: Relationship false branch preserves const fact in `compute_node_const_sets`
        // (so `temp[0] /= local[0]` on false branch still knows `local[0] == 2`, zero trap!).
        let rel_false_const_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" { return true; }
            tree 4097 "Action" (locals: 1) {
                local[0] := 2;
                if relationship(column: 0, dir: 0, flags: 1, value: local[0]) {
                    return true;
                } else {
                    temp[0] /= local[0];
                    return false;
                }
            }
        "#;
        let rel_const_obj = compile_object_from_sources(
            base_manifest,
            &[("behavior.antics", rel_false_const_script)],
        )
        .unwrap();
        let rel_const_report = verify_object_safety(&rel_const_obj).unwrap();
        assert!(
            rel_const_report.is_ok(),
            "Relationship false branch should preserve local[0] == 2: {:?}",
            rel_const_report.violations
        );

        // P1-2 (scope 22): Relationship read into `scope(22, 0)` with unknown param[0]
        // must conservatively alias tracked `stack.attr[0]` and catch LockLeak!
        let rel_s22_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" (args: 1) {
                stack.attr[0] := 1;
                stack.attr[0] := 0;
                relationship(column: 0, dir: 0, flags: 0, value: scope(22, 0));
                return true;
            }
        "#;
        let rel_s22_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", rel_s22_script)])
                .unwrap();
        let rel_s22_report = verify_object_safety(&rel_s22_obj).unwrap();
        assert!(
            rel_s22_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "Relationship read into scope(22, 0) must be caught as LockLeak: {:?}",
            rel_s22_report.violations
        );

        // P1-3 (local STR# 303 private callee): Run Tree By Name resolving to a private helper
        // that reacquires `stack.attr[0]` must be caught as LockLeak!
        let rtb_private_script = r#"
            str 303 "TreeNames" { "RelockHelper" }
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4099 "RelockHelper" {
                stack.attr[0] := 1;
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                stack.attr[0] := 0;
                run_tree_by_name(table: 303, index: 1, mode: 2);
                return true;
            }
        "#;
        let rtb_priv_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", rtb_private_script)])
                .unwrap();
        let rtb_priv_report = verify_object_safety(&rtb_priv_obj).unwrap();
        assert!(
            rtb_priv_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "local Run Tree By Name calling RelockHelper must be caught as LockLeak: {:?}",
            rtb_priv_report.violations
        );

        // P1-4 (true branch unknown stack_obj aliasing): FindBestObject category search (func 15)
        // true branch sets `stack_obj = Unknown` (may alias `InitialTarget`), so `stack.attr[0] := 1`
        // must be caught as LockLeak, whereas `CreateObject` true branch (`OtherObject`) is distinct!
        let fbo_true_alias_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                stack.attr[0] := 0;
                if find_best_object(15, 0) {
                    stack.attr[0] := 1;
                    return true;
                }
                return false;
            }
        "#;
        let fbo_true_obj = compile_object_from_sources(
            base_manifest,
            &[("behavior.antics", fbo_true_alias_script)],
        )
        .unwrap();
        let fbo_true_report = verify_object_safety(&fbo_true_obj).unwrap();
        assert!(
            fbo_true_report
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "FindBestObject true branch unknown stack_obj aliasing must catch LockLeak: {:?}",
            fbo_true_report.violations
        );

        // P2-2: ManageInventory op 4 false branch writes count=0 THEN index=-1 (VM write order!),
        // so when both count and index select temp[0] (flags: 0), temp[0] becomes -1 (not 0),
        // making `if temp[0] == -1 { stack.attr[0] := 0; }` provably release on the false branch!
        let inv_order_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                if manage_inventory(op: 4, kind: 1, flags: 0, guid: 0) {
                    stack.attr[0] := 0;
                    return true;
                } else {
                    if temp[0] == -1 {
                        stack.attr[0] := 0;
                        return false;
                    }
                    return false;
                }
            }
        "#;
        let inv_order_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", inv_order_script)])
                .unwrap();
        let inv_order_report = verify_object_safety(&inv_order_obj).unwrap();
        assert!(
            inv_order_report.is_ok(),
            "ManageInventory op 4 false branch must write count=0 then index=-1 to temp[0]: {:?}",
            inv_order_report.violations
        );
    }

    #[test]
    fn sol_wave1_r2_soundness_regressions() {
        let base_manifest = r#"
[object]
name = "SolWave1R2Probe"
guid = 0x50110002
init = "Init"
main = "Init"
scripts = ["behavior.antics"]

[[interaction]]
name = "Use"
action = "Action"
guard = "Guard"
"#;

        // R2 P1-2: Mutated param[0] in private callee before scope(22, 0) Relationship read
        // must NOT use stale initial param[0] = 0 during lock discovery!
        let stale_param_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[1] != 0 { return false; }
                return true;
            }
            tree 4099 "Helper" (args: 1) {
                param[0] := 1;
                relationship(column: 0, dir: 0, flags: 0, value: scope(22, 0));
                return true;
            }
            tree 4097 "Action" {
                call 4099(0, 0, 0, 0);
                return true;
            }
        "#;
        let stale_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", stale_param_script)])
                .unwrap();
        let stale_rep = verify_object_safety(&stale_obj).unwrap();
        assert!(
            stale_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "mutated param[0] + scope(22, 0) Relationship read must catch LockLeak on attr[1]: {:?}",
            stale_rep.violations
        );

        // R2 P1-3 (case-insensitive shadowing & empty tracked lock set):
        // Case-insensitive-only local match `cleanup` for STR# 303 `"Cleanup"` must return
        // `InconclusiveProof` (could be shadowed by exact semi-global/global `"Cleanup"`!).
        let ci_shadow_script = r#"
            str 303 "TreeNames" { "Cleanup" }
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4099 "cleanup" {
                stack.attr[0] := 0;
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                run_tree_by_name(table: 303, index: 1, mode: 2);
                return true;
            }
        "#;
        let ci_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", ci_shadow_script)])
                .unwrap();
        let ci_rep = verify_object_safety(&ci_obj).unwrap();
        assert!(
            ci_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::InconclusiveProof),
            "case-insensitive-only local match must return InconclusiveProof: {:?}",
            ci_rep.violations
        );

        // Unresolved Run Tree By Name even when tracked_locks is empty must still report InconclusiveProof!
        let empty_locks_dyn_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" { return true; }
            tree 4097 "Action" {
                run_tree_by_name(table: 303, index: 1, mode: 2, global: true);
                return true;
            }
        "#;
        let empty_dyn_obj = compile_object_from_sources(
            base_manifest,
            &[("behavior.antics", empty_locks_dyn_script)],
        )
        .unwrap();
        let empty_dyn_rep = verify_object_safety(&empty_dyn_obj).unwrap();
        assert!(
            empty_dyn_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::InconclusiveProof),
            "unresolved Run Tree By Name with empty tracked_locks must still report InconclusiveProof: {:?}",
            empty_dyn_rep.violations
        );

        // R2 P1-4: Named-tree mode 1 runs on target's thread with target's own temps, so callee
        // writing `temp[0] := 1` must NOT make prover assume caller sees `temp[0] == 1`!
        let mode1_temps_script = r#"
            str 303 "TreeNames" { "SetTargetTemp" }
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4099 "SetTargetTemp" {
                temp[0] := 1;
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                temp[0] := 0;
                run_tree_by_name(table: 303, index: 1, mode: 1);
                if temp[0] == 1 {
                    stack.attr[0] := 0;
                }
                return true;
            }
        "#;
        let mode1_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", mode1_temps_script)])
                .unwrap();
        let mode1_rep = verify_object_safety(&mode1_obj).unwrap();
        assert!(
            mode1_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "Run Tree By Name mode 1 writing target temp[0] must not credit caller's temp[0] == 1 unlock: {:?}",
            mode1_rep.violations
        );

        // R2 P1-5: Literal positive `stack_obj := 1` may alias InitialTarget (runtime ID 1),
        // so writing `stack.attr[0] := 1` after `stack_obj := 1` must catch LockLeak!
        let lit_id_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                stack.attr[0] := 0;
                stack_obj := 1;
                stack.attr[0] := 1;
                return true;
            }
        "#;
        let lit_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", lit_id_script)])
                .unwrap();
        let lit_rep = verify_object_safety(&lit_obj).unwrap();
        assert!(
            lit_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "literal stack_obj := 1 may alias InitialTarget and must catch LockLeak: {:?}",
            lit_rep.violations
        );

        // R2 P1-6: SetToNext mode 4 false path can store `0` into `stack_obj`, so subsequent
        // `stack.attr[0] := 0` must NOT be credited as releasing InitialTarget's lock!
        let stn4_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                if set_to_next(4, guid: 0x12345678) {
                    return true;
                } else {
                    stack.attr[0] := 0;
                    return false;
                }
            }
        "#;
        let stn4_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", stn4_script)])
                .unwrap();
        let stn4_rep = verify_object_safety(&stn4_obj).unwrap();
        assert!(
            stn4_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "SetToNext mode 4 false path stores 0 into stack_obj and must catch LockLeak: {:?}",
            stn4_rep.violations
        );

        // R2 P1-7: ActionString with valid STR# index containing `$Family` traps during formatting
        // and must be caught as LockLeak when called while holding a lock!
        let act_str_script = r#"
            str 300 "ActionStrings" { "Visit $Family" }
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                action_string(table: 300, index: 1);
                stack.attr[0] := 0;
                return true;
            }
        "#;
        let act_str_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", act_str_script)])
                .unwrap();
        let act_str_rep = verify_object_safety(&act_str_obj).unwrap();
        assert!(
            act_str_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "ActionString with $Family token must be modeled as trapping and catch LockLeak: {:?}",
            act_str_rep.violations
        );

        // R2 P2-1: Valid 7-argument extended inline private call (with following record supplying
        // remaining bytes) must NOT emit ArityMismatch!
        let ext7_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" { return true; }
            tree 4099 "SevenArgs" (format: 0x8002, args: 7) { return true; }
            tree 4097 "Action" {
                @n0:
                call 4099(1, 2, 3, 4) -> true: @n1, false: false;
                @n1:
                notify_idle() -> true: true, false: true;
            }
        "#;
        let ext7_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", ext7_script)])
                .unwrap();
        let ext7_rep = verify_object_safety(&ext7_obj).unwrap();
        assert!(
            !ext7_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::ArityMismatch),
            "valid 7-arg extended inline call with following record must not fail ArityMismatch: {:?}",
            ext7_rep.violations
        );
    }

    #[test]
    fn wave2_primitives_verifier_static_lock_and_equiv_models() {
        use crate::compile::{compile_antics, compile_object_from_sources};

        let base_manifest = r#"
[object]
name = "Wave2VerifierProbe"
guid = 0x50110003
init = "Init"
main = "Init"
scripts = ["behavior.antics"]

[[interaction]]
name = "Probe"
action = "Action"
guard = "Guard"
"#;

        // 1. GetDirectionTo writing into `stack.attr[0]` must be tracked as acquiring the lock!
        let gdt_lock_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                get_direction_to(stack.attr[0]);
                return true;
            }
        "#;
        let gdt_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", gdt_lock_script)])
                .unwrap();
        let gdt_rep = verify_object_safety(&gdt_obj).unwrap();
        assert!(
            gdt_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "GetDirectionTo writing stack.attr[0] must be caught as LockLeak: {:?}",
            gdt_rep.violations
        );

        // 2. Static trap checks on Tutorial (mode > 1), FindWorstMotives (target 2 / mode > 2),
        //    LookTowards (mode > 3), and OldRelationship (dir > 3).
        let traps_script = r#"
            tree 4096 "Init" (args: 1) {
                tutorial(2);
                find_worst_motives(target: 2, mode: 3);
                look_towards(9);
                old_relationship(write: true, column: 0, param: 0, dir: 5, flags: 0);
                return true;
            }
            tree 4098 "Guard" { return true; }
            tree 4097 "Action" { return true; }
        "#;
        let traps_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", traps_script)])
                .unwrap();
        let traps_rep = verify_object_safety(&traps_obj).unwrap();
        let trap_msgs: Vec<&str> = traps_rep
            .violations
            .iter()
            .filter(|v| v.category == SafetyCategory::PotentialTrap)
            .map(|v| v.message.as_str())
            .collect();
        assert!(
            trap_msgs.iter().any(|m| m.contains("Tutorial"))
                && trap_msgs.iter().any(|m| m.contains("FindWorstMotives"))
                && trap_msgs.iter().any(|m| m.contains("LookTowards"))
                && trap_msgs.iter().any(|m| m.contains("OldRelationship")),
            "expected static PotentialTrap diagnostics for Wave 2 primitives, got: {trap_msgs:?}"
        );

        // 3. GetDirectionTo with Global 34 `from` redirects destination -> Equiv returns Inconclusive.
        let gdt_g34_a = &compile_antics(
            r#"tree 4096 "A" { get_direction_to(temp[0], from: global[34]); return true; }"#,
        )
        .unwrap()
        .behaviors[&4096];
        let gdt_g34_b = &compile_antics(
            r#"tree 4096 "B" { get_direction_to(temp[0], from: global[34]); return true; }"#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(gdt_g34_a, gdt_g34_b, &EquivOptions::default()),
                EquivResult::Inconclusive { .. }
            ),
            "GetDirectionTo with Global 34 from operand must return Inconclusive"
        );

        // 4. OldRelationship read (write: false) mutates param[0] on `true`; subsequent read must
        //    not reuse stale constant `param[0] == 7`.
        let or_a = &compile_antics(
            r#"
            tree 4096 "A" (args: 1) {
                param[0] := 7;
                old_relationship(write: false, column: 0, param: 0, dir: 0, flags: 0);
                if param[0] == 7 { return true; }
                return false;
            }
            "#,
        )
        .unwrap()
        .behaviors[&4096];
        let or_b = &compile_antics(
            r#"
            tree 4096 "B" (args: 1) {
                param[0] := 7;
                old_relationship(write: false, column: 0, param: 0, dir: 0, flags: 0);
                return true;
            }
            "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            matches!(
                check_bhav_equivalent(or_a, or_b, &EquivOptions::default()),
                EquivResult::Counterexample { .. }
            ),
            "OldRelationship read mutates param[0], so folding param[0] == 7 to true must be rejected"
        );
    }

    #[test]
    fn sol_wave2_r1_soundness_regressions() {
        use crate::compile::{compile_antics, compile_object_from_sources};

        let base_manifest = r#"
[object]
name = "SolWave2R1Probe"
guid = 0x50110004
init = "Init"
main = "Init"
scripts = ["behavior.antics"]

[[interaction]]
name = "Probe"
action = "Action"
guard = "Guard"
"#;

        // P1-2: Reach (47), GosubFoundAction (30), and FindBestInteraction (3) must return
        // InconclusiveProof even when a tracked lock was acquired and released before them!
        for (label, call_stmt) in [
            ("Reach", "reach(mode: 0, moves: true);"),
            ("GosubFoundAction", "gosub_found_action();"),
            ("FindBestInteraction", "find_best_interaction();"),
        ] {
            let script = format!(
                r#"
                tree 4096 "Init" {{ return true; }}
                tree 4098 "Guard" {{
                    if stack.attr[0] != 0 {{ return false; }}
                    return true;
                }}
                tree 4097 "Action" {{
                    stack.attr[0] := 1;
                    stack.attr[0] := 0;
                    {call_stmt}
                    return true;
                }}
                "#
            );
            let obj = compile_object_from_sources(base_manifest, &[("behavior.antics", &script)])
                .unwrap();
            let rep = verify_object_safety(&obj).unwrap();
            assert!(
                rep.violations
                    .iter()
                    .any(|v| v.category == SafetyCategory::InconclusiveProof),
                "{label} after balanced lock acquire/release must return InconclusiveProof: {:?}",
                rep.violations
            );
        }

        // P1-3a: GetDirectionTo writing scope(11, 0) (`temp[temp[0]]`) must invalidate temp[1]!
        let scope11_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                temp[0] := 1;
                temp[1] := 0;
                get_direction_to(scope(11, 0));
                if temp[1] == 0 {
                    stack.attr[0] := 0;
                }
                return true;
            }
        "#;
        let scope11_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", scope11_script)])
                .unwrap();
        let scope11_rep = verify_object_safety(&scope11_obj).unwrap();
        assert!(
            scope11_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "GetDirectionTo writing scope(11, 0) must invalidate temp[1] and catch LockLeak: {:?}",
            scope11_rep.violations
        );

        // P1-3b: GetDirectionTo writing scope(22, 0) with param[0] = -1 traps before lock release!
        let scope22_trap_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" (args: 1) {
                stack.attr[0] := 1;
                param[0] := -1;
                get_direction_to(scope(22, 0));
                stack.attr[0] := 0;
                return true;
            }
        "#;
        let scope22_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", scope22_trap_script)])
                .unwrap();
        let scope22_rep = verify_object_safety(&scope22_obj).unwrap();
        assert!(
            scope22_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "GetDirectionTo writing scope(22, 0) with param[0] = -1 must trap and catch LockLeak: {:?}",
            scope22_rep.violations
        );

        // P1-4: Equivalence must NOT erase `0 == 0` scratch-pointer init before `Reach`!
        let reach_with_init = &compile_antics(
            r#"
            tree 4096 "A" {
                if 0 == 0 {
                    reach(mode: 0, moves: true);
                }
                return true;
            }
            "#,
        )
        .unwrap()
        .behaviors[&4096];
        let reach_without_init = &compile_antics(
            r#"
            tree 4096 "B" {
                reach(mode: 0, moves: true);
                return true;
            }
            "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            !matches!(
                check_bhav_equivalent(
                    reach_with_init,
                    reach_without_init,
                    &EquivOptions::default()
                ),
                EquivResult::Equivalent { .. }
            ),
            "Equivalence must not erase `0 == 0` scratch-pointer init before Reach"
        );

        // P2-1: Breakpoint static trap checks (`breakpoint()` and nonzero local trap; `0` passes).
        let bp_trap_script = r#"
            tree 4096 "Init" (locals: 1) {
                breakpoint();
                local[0] := 1;
                breakpoint(local[0]);
                return true;
            }
            tree 4098 "Guard" { return true; }
            tree 4097 "Action" { return true; }
        "#;
        let bp_trap_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", bp_trap_script)])
                .unwrap();
        let bp_trap_rep = verify_object_safety(&bp_trap_obj).unwrap();
        let bp_traps: Vec<&str> = bp_trap_rep
            .violations
            .iter()
            .filter(|v| {
                v.category == SafetyCategory::PotentialTrap && v.message.contains("Breakpoint")
            })
            .map(|v| v.message.as_str())
            .collect();
        assert_eq!(
            bp_traps.len(),
            2,
            "expected 2 Breakpoint PotentialTrap diagnostics, got: {bp_traps:?}"
        );

        let bp_safe_script = r#"
            tree 4096 "Init" (locals: 1) {
                breakpoint(0);
                local[0] := 0;
                breakpoint(local[0]);
                return true;
            }
            tree 4098 "Guard" { return true; }
            tree 4097 "Action" { return true; }
        "#;
        let bp_safe_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", bp_safe_script)])
                .unwrap();
        let bp_safe_rep = verify_object_safety(&bp_safe_obj).unwrap();
        assert!(
            bp_safe_rep.violations.is_empty(),
            "zero-condition Breakpoint must pass static checks: {:?}",
            bp_safe_rep.violations
        );
    }

    #[test]
    fn sol_wave2_r2_soundness_regressions() {
        use crate::compile::{compile_antics, compile_object_from_sources};

        let base_manifest = r#"
[object]
name = "SolWave2R2Probe"
guid = 0x50110005
init = "Init"
main = "Init"
scripts = ["behavior.antics"]

[[interaction]]
name = "Probe"
action = "Action"
guard = "Guard"
"#;

        // R2 P1-2a: OBJD `attributes = 65535` is read as signed `-1`, so VM allocates 8 attributes
        // (0..8); writing `scope(22, 0)` with `param[0] = 8` traps before lock release!
        let neg_attrs_manifest = r#"
[object]
name = "NegAttrsProbe"
guid = 0x50110006
attributes = 65535
init = "Init"
main = "Init"
scripts = ["behavior.antics"]

[[interaction]]
name = "Probe"
action = "Action"
guard = "Guard"
"#;
        let attr8_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" (args: 1) {
                stack.attr[0] := 1;
                param[0] := 8;
                get_direction_to(scope(22, 0));
                stack.attr[0] := 0;
                return true;
            }
        "#;
        let neg_obj =
            compile_object_from_sources(neg_attrs_manifest, &[("behavior.antics", attr8_script)])
                .unwrap();
        let neg_rep = verify_object_safety(&neg_obj).unwrap();
        assert!(
            neg_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "attributes=65535 (signed -1 -> 8 slots) must trap on attr[8] and report LockLeak: {:?}",
            neg_rep.violations
        );

        // R2 P1-2b: OBJD `attributes = 0` still allocates 8 attributes (0..8) in VM, so writing
        // `scope(22, 0)` with `param[0] = 2` (distinct from lock `stack.attr[0]`) must not false-leak!
        let zero_attrs_manifest = r#"
[object]
name = "ZeroAttrsProbe"
guid = 0x50110007
attributes = 0
init = "Init"
main = "Init"
scripts = ["behavior.antics"]

[[interaction]]
name = "Probe"
action = "Action"
guard = "Guard"
"#;
        let attr2_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" (args: 1) {
                stack.attr[0] := 1;
                param[0] := 2;
                get_direction_to(scope(22, 0));
                stack.attr[0] := 0;
                return true;
            }
        "#;
        let zero_obj =
            compile_object_from_sources(zero_attrs_manifest, &[("behavior.antics", attr2_script)])
                .unwrap();
        let zero_rep = verify_object_safety(&zero_obj).unwrap();
        assert!(
            zero_rep.violations.is_empty(),
            "attributes=0 (VM floor 8 slots) must allow attr[2] without false LockLeak: {:?}",
            zero_rep.violations
        );

        // R2 P1-3: Scope 13 (`stack.temp[0]`) aliases actor `temp[0]` when actor == stack_obj,
        // so `get_direction_to(scope(13, 0))` must invalidate `temp[0]` and catch LockLeak!
        let scope13_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" {
                stack.attr[0] := 1;
                temp[0] := 0;
                get_direction_to(scope(13, 0));
                if temp[0] == 0 {
                    stack.attr[0] := 0;
                }
                return true;
            }
        "#;
        let scope13_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", scope13_script)])
                .unwrap();
        let scope13_rep = verify_object_safety(&scope13_obj).unwrap();
        assert!(
            scope13_rep
                .violations
                .iter()
                .any(|v| v.category == SafetyCategory::LockLeak),
            "GetDirectionTo writing scope(13, 0) must invalidate temp[0] and catch LockLeak: {:?}",
            scope13_rep.violations
        );

        // R2 P1-4: Equivalence must NOT erase `0 == 0` scratch-pointer init before FindBestObject!
        let fbo_with_init = &compile_antics(
            r#"
            tree 4096 "A" {
                if 0 == 0 {
                    find_best_object(0, 0);
                }
                return true;
            }
            "#,
        )
        .unwrap()
        .behaviors[&4096];
        let fbo_without_init = &compile_antics(
            r#"
            tree 4096 "B" {
                find_best_object(0, 0);
                return true;
            }
            "#,
        )
        .unwrap()
        .behaviors[&4096];
        assert!(
            !matches!(
                check_bhav_equivalent(fbo_with_init, fbo_without_init, &EquivOptions::default()),
                EquivResult::Equivalent { .. }
            ),
            "Equivalence must not erase `0 == 0` scratch-pointer init before FindBestObject"
        );

        // R2 P2-2: Proved-zero local Breakpoint while holding a lock must NOT false-leak!
        let bp_lock_safe_script = r#"
            tree 4096 "Init" { return true; }
            tree 4098 "Guard" {
                if stack.attr[0] != 0 { return false; }
                return true;
            }
            tree 4097 "Action" (locals: 1) {
                stack.attr[0] := 1;
                local[0] := 0;
                breakpoint(local[0]);
                stack.attr[0] := 0;
                return true;
            }
        "#;
        let bp_lock_obj =
            compile_object_from_sources(base_manifest, &[("behavior.antics", bp_lock_safe_script)])
                .unwrap();
        let bp_lock_rep = verify_object_safety(&bp_lock_obj).unwrap();
        assert!(
            bp_lock_rep.violations.is_empty(),
            "proved-zero local Breakpoint must not add false trap edge in lock prover: {:?}",
            bp_lock_rep.violations
        );
    }
}
