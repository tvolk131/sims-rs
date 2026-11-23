//! Whole-file equivalence: two IFF files that differ only in `BHAV` bodies.
//!
//! The comparison is strict about structure. Unless both files have the same chunks in
//! the same physical order (type, ID, label, header flags and effective resource-map
//! flags alike), every chunk that is not a `BHAV` is byte-identical, neither file has
//! two chunks of one type and ID, and both have the same resource map (or neither has
//! one), the result is `Inconclusive`: the loaders (the original's and `sims-vm`'s)
//! would otherwise have to be modeled for every resource type, and each round of
//! review found another loader corner. Within that precondition only `BHAV` bodies
//! differ, and each differing pair goes through [`check_bhav_equivalent_combined`].
//! Reports name chunks only, never their contents.

use super::{
    EquivMethod, EquivOptions, EquivResult, LiveContext, LiveOptions, bhav_param_limit,
    check_bhav_equivalent_combined, context_mismatch,
};
use crate::error::{Error, Result};
use crate::ir::BhavChunk;
use crate::lift::lift_bhav;
use sims_formats::behavior::Behavior;
use sims_formats::execution::constants;
use sims_formats::iff::{Chunk, Iff, ResourceMap};
use std::collections::BTreeMap;

/// Options for [`check_iff_equivalent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IffEquivOptions {
    /// Bounds for the lockstep checker.
    pub lockstep: EquivOptions,
    /// Bounds and observation policy for the live-output checker.
    pub live: LiveOptions,
    /// Attributes the objects running these trees are known to have (see
    /// [`LiveContext::my_attribute_count`]).
    pub my_attribute_count: usize,
    /// Compare only this `BHAV`.
    pub tree: Option<u16>,
    /// Arities of trees outside both files (`Global.iff`, semi-globals) that calls may
    /// target, so calls with more than four inline arguments compare exactly.
    pub extra_arities: BTreeMap<u16, usize>,
}

impl Default for IffEquivOptions {
    fn default() -> Self {
        Self {
            lockstep: EquivOptions::default(),
            live: LiveOptions::default(),
            my_attribute_count: super::GUARANTEED_ATTRIBUTES,
            tree: None,
            extra_arities: BTreeMap::new(),
        }
    }
}

/// How one resource compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceVerdict {
    /// Byte-identical.
    Identical,
    /// Different bytes, same behavior or value; `how` says what proved it.
    Equivalent { how: String },
    /// Shown to differ by running both trees in `sims-vm`.
    Different { why: String },
    /// Could not be decided.
    Inconclusive { why: String },
}

/// One resource's comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceReport {
    pub tag: [u8; 4],
    pub id: u16,
    pub verdict: ResourceVerdict,
    /// Differences that do not change behavior (a tree's format or revision word).
    pub notes: Vec<String>,
}

impl ResourceReport {
    /// `"BHAV #4143"`.
    #[must_use]
    pub fn name(&self) -> String {
        format!("{} #{}", String::from_utf8_lossy(&self.tag), self.id)
    }
}

/// Overall result of [`check_iff_equivalent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IffVerdict {
    Equivalent,
    Different,
    Inconclusive,
}

impl IffVerdict {
    /// Process exit code for the CLI: 0, 1, 2.
    #[must_use]
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Equivalent => 0,
            Self::Different => 1,
            Self::Inconclusive => 2,
        }
    }
}

/// Every resource's comparison, plus file-level problems.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IffEquivReport {
    pub resources: Vec<ResourceReport>,
    /// Why the files fall outside the structural precondition (see the module docs).
    pub file_problems: Vec<String>,
}

impl IffEquivReport {
    /// `Inconclusive` if the files fall outside the structural precondition, else
    /// `Different` if a tree pair was shown to differ, else `Inconclusive` if any could
    /// not be decided, else `Equivalent`.
    #[must_use]
    pub fn verdict(&self) -> IffVerdict {
        if !self.file_problems.is_empty() {
            IffVerdict::Inconclusive
        } else if self
            .resources
            .iter()
            .any(|r| matches!(r.verdict, ResourceVerdict::Different { .. }))
        {
            IffVerdict::Different
        } else if self
            .resources
            .iter()
            .any(|r| matches!(r.verdict, ResourceVerdict::Inconclusive { .. }))
        {
            IffVerdict::Inconclusive
        } else {
            IffVerdict::Equivalent
        }
    }
}

/// Compares two IFF files that may differ only in `BHAV` bodies.
///
/// # Errors
/// Returns an error when either file is not a parseable IFF, or `opts.tree` names a
/// tree neither file has.
pub fn check_iff_equivalent(a: &[u8], b: &[u8], opts: &IffEquivOptions) -> Result<IffEquivReport> {
    let iff_a = Iff::parse(a.to_vec()).map_err(|e| Error::new(format!("file A: {e}")))?;
    let iff_b = Iff::parse(b.to_vec()).map_err(|e| Error::new(format!("file B: {e}")))?;
    if let Some(tree) = opts.tree
        && !has_bhav(&iff_a, tree)
        && !has_bhav(&iff_b, tree)
    {
        return Err(Error::new(format!("neither file contains BHAV #{tree}")));
    }
    let mut report = IffEquivReport::default();
    for (side, iff) in [("A", &iff_a), ("B", &iff_b)] {
        if let Some(problem) = single_file_problem(iff) {
            report.file_problems.push(format!("file {side}: {problem}"));
        }
    }
    report
        .file_problems
        .extend(structure_problem(&iff_a, &iff_b));
    if !report.file_problems.is_empty() {
        return Ok(report);
    }
    let ctx = |iff: &Iff| LiveContext {
        callee_arities: Some(arities(iff, &opts.extra_arities)),
        my_attribute_count: opts.my_attribute_count,
        local_bcons: Some(bcons(iff)),
    };
    let (ctx_a, ctx_b) = (ctx(&iff_a), ctx(&iff_b));
    for (ca, cb) in iff_a.chunks.iter().zip(&iff_b.chunks) {
        let is_bhav = &ca.tag == b"BHAV";
        if let Some(tree) = opts.tree
            && is_bhav
            && ca.id != tree
        {
            continue;
        }
        let mut notes = Vec::new();
        let verdict = if is_bhav {
            compare_bhav(
                ca.id, ca, cb, &iff_a, &iff_b, &ctx_a, &ctx_b, opts, &mut notes,
            )
        } else {
            ResourceVerdict::Identical
        };
        report.resources.push(ResourceReport {
            tag: ca.tag,
            id: ca.id,
            verdict,
            notes,
        });
    }
    Ok(report)
}

fn has_bhav(iff: &Iff, id: u16) -> bool {
    iff.chunks.iter().any(|c| &c.tag == b"BHAV" && c.id == id)
}

/// Callee argument counts: every `BHAV` in the file (`sims-vm` loads deleted chunks
/// too, `program.rs:601`), plus `extra`.
fn arities(iff: &Iff, extra: &BTreeMap<u16, usize>) -> BTreeMap<u16, usize> {
    let mut map = extra.clone();
    for chunk in iff.chunks.iter().filter(|c| &c.tag == b"BHAV") {
        if let Ok(bhav) = Behavior::parse(iff.data(chunk)) {
            map.insert(chunk.id, bhav_param_limit(&bhav));
        }
    }
    map
}

fn bcons(iff: &Iff) -> BTreeMap<u16, Vec<i16>> {
    iff.chunks
        .iter()
        .filter(|c| &c.tag == b"BCON")
        .filter_map(|c| constants(iff.data(c)).ok().map(|v| (c.id, v)))
        .collect()
}

/// Why one file's resources may depend on which loader reads them: an unusable
/// resource map, two chunks of one type and ID (the original keeps the one its map or
/// first live copy names, `sims-vm` parses every copy and keeps the last,
/// `program.rs:601`), or a map that selects a different chunk than the physical order
/// does. `None` when every resource has exactly one chunk both loaders agree on.
///
/// `sims-antics equiv` checks this on its IFF before extracting the tree.
#[must_use]
pub fn single_file_problem(iff: &Iff) -> Option<String> {
    if let Some(e) = iff.map_error() {
        return Some(format!("the resource map is unusable ({e})"));
    }
    let mut seen: BTreeMap<([u8; 4], u16), usize> = BTreeMap::new();
    for chunk in &iff.chunks {
        *seen.entry((chunk.tag, chunk.id)).or_default() += 1;
    }
    let dups: Vec<String> = seen
        .iter()
        .filter(|&(_, &n)| n > 1)
        .map(|((tag, id), _)| format!("{} #{id}", String::from_utf8_lossy(tag)))
        .collect();
    if !dups.is_empty() {
        return Some(format!(
            "more than one chunk of the same type and ID ({}), which the loaders resolve \
             differently",
            shown(&dups)
        ));
    }
    let disagree: Vec<String> = iff
        .chunks
        .iter()
        .filter(|c| &c.tag != b"rsmp")
        .filter(|c| !iff.get(&c.tag, c.id).is_some_and(|r| std::ptr::eq(r, *c)))
        .map(|c| format!("{} #{}", String::from_utf8_lossy(&c.tag), c.id))
        .collect();
    (!disagree.is_empty()).then(|| {
        format!(
            "the resource map does not select the only chunk of {} (deleted or unmapped \
             chunks), so the result depends on the loader",
            shown(&disagree)
        )
    })
}

fn shown(items: &[String]) -> String {
    let first: Vec<&str> = items.iter().take(4).map(String::as_str).collect();
    format!(
        "{}{}",
        first.join(", "),
        if items.len() > 4 { ", ..." } else { "" }
    )
}

/// Why the two files are not the same apart from `BHAV` bodies, if they are not.
fn structure_problem(a: &Iff, b: &Iff) -> Option<String> {
    if a.chunks.len() != b.chunks.len() {
        return Some(format!(
            "the files have different chunk layouts ({} vs {} chunks); only files that \
             differ in BHAV bodies alone are compared",
            a.chunks.len(),
            b.chunks.len()
        ));
    }
    let name = |c: &Chunk| format!("{} #{}", String::from_utf8_lossy(&c.tag), c.id);
    let mut differ: Vec<String> = Vec::new();
    for (i, (ca, cb)) in a.chunks.iter().zip(&b.chunks).enumerate() {
        let what = if (ca.tag, ca.id) != (cb.tag, cb.id) {
            Some(format!(
                "chunk {i} is {} in A but {} in B",
                name(ca),
                name(cb)
            ))
        } else if a.label_bytes(ca) != b.label_bytes(cb) {
            Some(format!("{} has different labels", name(ca)))
        } else if ca.flags != cb.flags {
            Some(format!("{} has different header flags", name(ca)))
        } else if a.resource_flags(ca) != b.resource_flags(cb) {
            Some(format!("{} has different resource-map flags", name(ca)))
        } else if &ca.tag == b"rsmp" {
            (map_view(a, ca) != map_view(b, cb))
                .then(|| format!("{} (the resource map) differs", name(ca)))
        } else if &ca.tag != b"BHAV" && a.data(ca) != b.data(cb) {
            Some(format!("{} has different contents", name(ca)))
        } else {
            None
        };
        differ.extend(what);
    }
    (!differ.is_empty()).then(|| {
        format!(
            "the files differ in more than BHAV bodies: {}; only files that differ in BHAV \
             bodies alone are compared",
            shown(&differ)
        )
    })
}

/// One map entry: the chunk it points at (by physical index, or the raw offset when it
/// points at none), ID, flags and label.
type MapEntry = (std::result::Result<usize, u32>, u16, u16, String);

/// A parsed resource map: its unused-bytes field and its entries by type.
type MapView = (u32, BTreeMap<[u8; 4], Vec<MapEntry>>);

/// A resource map with its offsets replaced by the physical chunk they point at, so
/// two files whose `BHAV` bodies differ in size compare equal when their maps select
/// the same chunks. A map that does not parse compares by its bytes.
fn map_view(iff: &Iff, chunk: &Chunk) -> std::result::Result<MapView, Vec<u8>> {
    let map = ResourceMap::parse(iff.data(chunk)).map_err(|_| iff.data(chunk).to_vec())?;
    let types = map
        .types
        .into_iter()
        .map(|(tag, entries)| {
            let entries = entries
                .into_iter()
                .map(|e| {
                    let target = iff
                        .chunks
                        .iter()
                        .position(|c| c.offset as u64 == u64::from(e.offset))
                        .ok_or(e.offset);
                    (target, e.id, e.flags, e.label)
                })
                .collect();
            (tag, entries)
        })
        .collect();
    Ok((map.unused_bytes, types))
}

fn lift(iff: &Iff, chunk: &Chunk) -> std::result::Result<BhavChunk, String> {
    lift_bhav(chunk.id, &chunk.label, iff.data(chunk)).map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn compare_bhav(
    id: u16,
    ca: &Chunk,
    cb: &Chunk,
    iff_a: &Iff,
    iff_b: &Iff,
    ctx_a: &LiveContext,
    ctx_b: &LiveContext,
    opts: &IffEquivOptions,
    notes: &mut Vec<String>,
) -> ResourceVerdict {
    let (a, b) = match (lift(iff_a, ca), lift(iff_b, cb)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            return ResourceVerdict::Inconclusive {
                why: format!("tree #{id} does not parse: {e}"),
            };
        }
    };
    // The argument count is the caller-facing signature (`interpreter.rs:1182`).
    let (args_a, args_b) = (super::live::frame_dims(&a).0, super::live::frame_dims(&b).0);
    if args_a != args_b {
        return ResourceVerdict::Inconclusive {
            why: format!(
                "argument count differs ({args_a} vs {args_b}); callers see different trees"
            ),
        };
    }
    if a.locals != b.locals {
        notes.push(format!(
            "local count differs ({} vs {})",
            a.locals, b.locals
        ));
    }
    if a.format != b.format {
        notes.push(format!(
            "format differs ({:#06x} vs {:#06x})",
            a.format, b.format
        ));
    }
    if a.revision != b.revision {
        notes.push("revision differs".to_owned());
    }
    if a.trailing != b.trailing {
        notes.push("trailing bytes differ".to_owned());
    }
    if iff_a.data(ca) == iff_b.data(cb) {
        if let Some(reason) = context_mismatch(&a, ctx_a, &b, ctx_b) {
            return ResourceVerdict::Inconclusive {
                why: format!("same tree bytes, but {reason}"),
            };
        }
        let callee_arity_differs = a.instructions.iter().any(|ins| {
            ins.opcode >= 256
                && ctx_a
                    .callee_arities
                    .as_ref()
                    .and_then(|m| m.get(&ins.opcode))
                    != ctx_b
                        .callee_arities
                        .as_ref()
                        .and_then(|m| m.get(&ins.opcode))
        });
        if !callee_arity_differs {
            return ResourceVerdict::Identical;
        }
    }
    let (method, result) =
        check_bhav_equivalent_combined(&a, ctx_a, &b, ctx_b, &opts.lockstep, &opts.live);
    match result {
        EquivResult::Equivalent {
            states_checked,
            paths_verified,
        } => {
            let how = match method {
                EquivMethod::Lockstep => {
                    format!("lockstep: {states_checked} states")
                }
                EquivMethod::LiveOutput => {
                    format!("live-output: {paths_verified} paths, {states_checked} steps")
                }
            };
            ResourceVerdict::Equivalent { how }
        }
        // The live-output checker reports a counterexample only after replaying it.
        EquivResult::Counterexample { divergence, .. } => ResourceVerdict::Different {
            why: format!("{method}: {divergence}"),
        },
        EquivResult::Inconclusive { reason } => ResourceVerdict::Inconclusive { why: reason },
    }
}
