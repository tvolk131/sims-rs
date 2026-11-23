//! Whole-file equivalence: compares every resource two IFF files expose to the game.
//!
//! Resources are matched by `(type, id)` as the original looks them up ([`Iff::get`]),
//! ignoring deleted chunks and the resource map itself. `BHAV` trees go through
//! [`check_bhav_equivalent_combined`]; string tables, tuning constants and other chunks
//! are compared by value, so encoding-only differences (a string table's layout version,
//! comments, chunk labels) are reported as notes rather than differences. Reports name
//! resources and entry indices only, never string contents.

use super::{
    EquivMethod, EquivOptions, EquivResult, LiveContext, LiveOptions, bhav_param_limit,
    check_bhav_equivalent_combined, context_mismatch,
};
use crate::error::{Error, Result};
use crate::ir::BhavChunk;
use crate::lift::lift_bhav;
use sims_formats::behavior::Behavior;
use sims_formats::execution::constants;
use sims_formats::iff::{DELETED, Iff};
use sims_formats::strings::{Format, StringTable};
use std::collections::{BTreeMap, BTreeSet};

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
    /// Byte-identical (ignoring notes such as labels).
    Identical,
    /// Different bytes, same behavior or value; `how` says what proved it.
    Equivalent { how: String },
    /// Proven to differ.
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
    /// Differences that do not change behavior (labels, encoding, editor metadata).
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
    /// File-level reasons the comparison cannot be trusted (an unusable resource map).
    pub file_problems: Vec<String>,
}

impl IffEquivReport {
    /// `Inconclusive` if a file problem means the compared resources may not be the ones
    /// the game loads, else `Different` if any resource differs, else `Inconclusive` if
    /// any could not be decided, else `Equivalent`.
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

/// Resources that carry only editor metadata the game never reads: tree and constant
/// labels (`TPRP`, `TRCN`) and the resource map itself.
const COSMETIC_TAGS: [&[u8; 4]; 2] = [b"TPRP", b"TRCN"];

/// The live resources of a file, keyed as the original looks them up.
fn resources(iff: &Iff) -> BTreeMap<([u8; 4], u16), usize> {
    let tags: BTreeSet<[u8; 4]> = iff.chunks.iter().map(|c| c.tag).collect();
    let mut out = BTreeMap::new();
    for tag in tags {
        if &tag == b"rsmp" {
            continue;
        }
        for chunk in iff.all(&tag) {
            if chunk.flags & DELETED != 0 {
                continue;
            }
            // `Iff::get` returns the chunk the original resolves for `(tag, id)`.
            let Some(resolved) = iff.get(&tag, chunk.id) else {
                continue;
            };
            let index = iff
                .chunks
                .iter()
                .position(|c| std::ptr::eq(c, resolved))
                .expect("resolved chunk belongs to the file");
            out.entry((tag, chunk.id)).or_insert(index);
        }
    }
    out
}

/// The chunks the game resolves for `tag`, by id.
fn resolved<'a>(
    iff: &'a Iff,
    res: &'a BTreeMap<([u8; 4], u16), usize>,
    tag: &'a [u8; 4],
) -> impl Iterator<Item = &'a sims_formats::iff::Chunk> + 'a {
    res.iter()
        .filter(move |((t, _), _)| t == tag)
        .map(|(_, &index)| &iff.chunks[index])
}

fn arities(
    iff: &Iff,
    res: &BTreeMap<([u8; 4], u16), usize>,
    extra: &BTreeMap<u16, usize>,
) -> BTreeMap<u16, usize> {
    let mut map = extra.clone();
    for chunk in resolved(iff, res, b"BHAV") {
        if let Ok(bhav) = Behavior::parse(iff.data(chunk)) {
            map.insert(chunk.id, bhav_param_limit(&bhav));
        }
    }
    map
}

fn bcons(iff: &Iff, res: &BTreeMap<([u8; 4], u16), usize>) -> BTreeMap<u16, Vec<i16>> {
    resolved(iff, res, b"BCON")
        .filter_map(|c| constants(iff.data(c)).ok().map(|v| (c.id, v)))
        .collect()
}

/// Compares two IFF files resource by resource.
///
/// # Errors
/// Returns an error when either file is not a parseable IFF.
pub fn check_iff_equivalent(a: &[u8], b: &[u8], opts: &IffEquivOptions) -> Result<IffEquivReport> {
    let iff_a = Iff::parse(a.to_vec()).map_err(|e| Error::new(format!("file A: {e}")))?;
    let iff_b = Iff::parse(b.to_vec()).map_err(|e| Error::new(format!("file B: {e}")))?;
    let mut report = IffEquivReport::default();
    for (side, iff) in [("A", &iff_a), ("B", &iff_b)] {
        if let Some(e) = iff.map_error() {
            report.file_problems.push(format!(
                "file {side}: resource map is unusable ({e}), so the resources compared may not be the ones the game loads"
            ));
        }
    }

    let res_a = resources(&iff_a);
    let res_b = resources(&iff_b);
    let ctx = |iff: &Iff, res: &BTreeMap<([u8; 4], u16), usize>| LiveContext {
        callee_arities: Some(arities(iff, res, &opts.extra_arities)),
        my_attribute_count: opts.my_attribute_count,
        local_bcons: Some(bcons(iff, res)),
    };
    let (ctx_a, ctx_b) = (ctx(&iff_a, &res_a), ctx(&iff_b, &res_b));
    let keys: BTreeSet<([u8; 4], u16)> = res_a.keys().chain(res_b.keys()).copied().collect();
    for (tag, id) in keys {
        if let Some(tree) = opts.tree
            && (&tag != b"BHAV" || id != tree)
        {
            continue;
        }
        let ca = res_a.get(&(tag, id)).map(|&i| &iff_a.chunks[i]);
        let cb = res_b.get(&(tag, id)).map(|&i| &iff_b.chunks[i]);
        let mut notes = Vec::new();
        let verdict = match (ca, cb) {
            (Some(_), None) | (None, Some(_)) if COSMETIC_TAGS.contains(&&tag) => {
                notes.push(format!(
                    "editor metadata only in file {}",
                    if ca.is_some() { "A" } else { "B" }
                ));
                ResourceVerdict::Identical
            }
            (Some(_), None) => ResourceVerdict::Different {
                why: "only in file A".to_owned(),
            },
            (None, Some(_)) => ResourceVerdict::Different {
                why: "only in file B".to_owned(),
            },
            (None, None) => unreachable!("key comes from one of the files"),
            // Run Tree By Name finds trees by their chunk label (`sims-vm/src/program.rs:607`).
            (Some(ca), Some(cb))
                if &tag == b"BHAV" && iff_a.label_bytes(ca) != iff_b.label_bytes(cb) =>
            {
                ResourceVerdict::Different {
                    why: "chunk label differs (Run Tree By Name looks trees up by label)"
                        .to_owned(),
                }
            }
            (Some(ca), Some(cb)) => {
                if iff_a.label_bytes(ca) != iff_b.label_bytes(cb) {
                    notes.push("chunk label differs".to_owned());
                }
                let (fa, fb) = (iff_a.resource_flags(ca), iff_b.resource_flags(cb));
                if fa != fb {
                    notes.push(format!("chunk flags differ ({fa:#06x} vs {fb:#06x})"));
                }
                let (da, db) = (iff_a.data(ca), iff_b.data(cb));
                if COSMETIC_TAGS.contains(&&tag) {
                    if da != db {
                        notes.push("editor metadata differs".to_owned());
                    }
                    ResourceVerdict::Identical
                } else if &tag == b"BHAV" {
                    // Even identical bytes behave differently against different constants
                    // or callees, so the tree's context is checked too.
                    compare_bhav(
                        id, ca, cb, &iff_a, &iff_b, &ctx_a, &ctx_b, opts, &mut notes,
                    )
                } else if da == db && (fa == fb || !matches!(&tag, b"STR#" | b"CTSS" | b"TTAs")) {
                    ResourceVerdict::Identical
                } else {
                    match &tag {
                        b"STR#" | b"CTSS" | b"TTAs" => compare_strings(da, fa, db, fb, &mut notes),
                        b"BCON" => compare_bcon(da, db, &mut notes),
                        _ => ResourceVerdict::Different {
                            why: first_byte_difference(da, db),
                        },
                    }
                }
            }
        };
        report.resources.push(ResourceReport {
            tag,
            id,
            verdict,
            notes,
        });
    }
    if let Some(tree) = opts.tree
        && report.resources.is_empty()
    {
        return Err(Error::new(format!("neither file contains BHAV #{tree}")));
    }
    Ok(report)
}

fn first_byte_difference(a: &[u8], b: &[u8]) -> String {
    match a.iter().zip(b).position(|(x, y)| x != y) {
        Some(i) => format!(
            "bytes differ from offset {i} ({} vs {} bytes)",
            a.len(),
            b.len()
        ),
        None => format!("lengths differ ({} vs {} bytes)", a.len(), b.len()),
    }
}

fn lift(iff: &Iff, chunk: &sims_formats::iff::Chunk) -> std::result::Result<BhavChunk, String> {
    lift_bhav(chunk.id, &chunk.label, iff.data(chunk)).map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn compare_bhav(
    id: u16,
    ca: &sims_formats::iff::Chunk,
    cb: &sims_formats::iff::Chunk,
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
        return ResourceVerdict::Different {
            why: format!("argument count differs ({args_a} vs {args_b})"),
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
                && ctx_a.callee_arities.as_ref().and_then(|m| m.get(&ins.opcode))
                    != ctx_b.callee_arities.as_ref().and_then(|m| m.get(&ins.opcode))
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
        EquivResult::Counterexample { divergence, .. } => ResourceVerdict::Different {
            why: format!("{method}: {divergence}"),
        },
        EquivResult::Inconclusive { reason } => ResourceVerdict::Inconclusive { why: reason },
    }
}

/// A string table's entries as the original's loader sees them (0x46feac): `None`
/// language for layouts without one.
fn effective_entries(table: &StringTable) -> Vec<(Option<u8>, Vec<u8>)> {
    match table.format {
        // The loader reads a leading word of 0 as a count of no strings (0x4700d7).
        Format::Pascal => Vec::new(),
        // Length-prefixed strings get bytes 0xa1 and 0xd5 rewritten (0x4700f5).
        Format::MacResource => table
            .entries
            .iter()
            .map(|e| {
                let text = e
                    .raw_text
                    .iter()
                    .map(|&c| match c {
                        0xA1 => b'o',
                        0xD5 => b'\'',
                        c => c,
                    })
                    .collect();
                (None, text)
            })
            .collect(),
        Format::Terminated | Format::Commented => table
            .entries
            .iter()
            .map(|e| (None, e.raw_text.clone()))
            .collect(),
        Format::Localized => table
            .entries
            .iter()
            .map(|e| (Some(e.language), e.raw_text.clone()))
            .collect(),
    }
}

fn compare_strings(
    da: &[u8],
    fa: u16,
    db: &[u8],
    fb: u16,
    notes: &mut Vec<String>,
) -> ResourceVerdict {
    let (a, b) = match (
        StringTable::parse_chunk(da, fa),
        StringTable::parse_chunk(db, fb),
    ) {
        (Ok(a), Ok(b)) => (a, b),
        _ => {
            return ResourceVerdict::Inconclusive {
                why: format!(
                    "a string table does not parse ({})",
                    first_byte_difference(da, db)
                ),
            };
        }
    };
    // A table with languages is filed by language runs (0x46ffcc); only an identical
    // sequence of (language, text) is known to load the same.
    if (a.format == Format::Localized) != (b.format == Format::Localized) {
        return ResourceVerdict::Inconclusive {
            why: format!(
                "string table layouts {:?} and {:?} load entries by language differently",
                a.format, b.format
            ),
        };
    }
    let (ea, eb) = (effective_entries(&a), effective_entries(&b));
    if ea.len() != eb.len() {
        return ResourceVerdict::Different {
            why: format!(
                "entry count as loaded differs ({} vs {})",
                ea.len(),
                eb.len()
            ),
        };
    }
    let differing: Vec<usize> = ea
        .iter()
        .zip(&eb)
        .enumerate()
        .filter(|(_, (x, y))| x != y)
        .map(|(i, _)| i)
        .collect();
    if !differing.is_empty() {
        let shown: Vec<String> = differing.iter().take(8).map(usize::to_string).collect();
        let more = if differing.len() > 8 { ", ..." } else { "" };
        return ResourceVerdict::Different {
            why: format!(
                "{} entr{} differ (indices {}{more})",
                differing.len(),
                if differing.len() == 1 { "y" } else { "ies" },
                shown.join(", ")
            ),
        };
    }
    if a.format != b.format {
        notes.push(format!(
            "string table layout differs ({:?} vs {:?})",
            a.format, b.format
        ));
    }
    let comments = a
        .entries
        .iter()
        .zip(&b.entries)
        .filter(|(x, y)| x.raw_comment != y.raw_comment)
        .count();
    if comments > 0 {
        notes.push(format!("{comments} entry comment(s) differ"));
    }
    if a.entries.len() != b.entries.len() {
        notes.push("entries the loader skips differ".to_owned());
    }
    ResourceVerdict::Equivalent {
        how: "same entries as loaded".to_owned(),
    }
}

fn compare_bcon(da: &[u8], db: &[u8], notes: &mut Vec<String>) -> ResourceVerdict {
    match (constants(da), constants(db)) {
        (Ok(a), Ok(b)) if a == b => {
            if da.get(1) != db.get(1) {
                notes.push("constant table flags byte differs".to_owned());
            }
            if da.len() != db.len() {
                notes.push("trailing bytes differ".to_owned());
            }
            ResourceVerdict::Equivalent {
                how: "same values".to_owned(),
            }
        }
        (Ok(a), Ok(b)) if a.len() != b.len() => ResourceVerdict::Different {
            why: format!("constant count differs ({} vs {})", a.len(), b.len()),
        },
        (Ok(a), Ok(b)) => {
            let idx: Vec<String> = a
                .iter()
                .zip(&b)
                .enumerate()
                .filter(|(_, (x, y))| x != y)
                .map(|(i, _)| i.to_string())
                .collect();
            ResourceVerdict::Different {
                why: format!("constants differ at indices {}", idx.join(", ")),
            }
        }
        _ => ResourceVerdict::Different {
            why: first_byte_difference(da, db),
        },
    }
}
