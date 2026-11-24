//! Whole-file equivalence: two IFF files that differ only in `BHAV` bodies.
//!
//! The comparison is strict about structure. Unless both files are the same bytes
//! apart from `BHAV` bodies (and the sizes and offsets those move; see
//! `structure_problem`), and neither file has two chunks of one type and ID, the result
//! is `Inconclusive`: the loaders (the original's and `sims-vm`'s) would otherwise
//! have to be modeled for every resource type, and each round of review found another
//! loader corner. Within that precondition only `BHAV` bodies differ, and each
//! differing pair goes through [`check_bhav_equivalent_combined`]. A tree counts as the
//! same only if every tree it may run does too ([`tree_reach`]). Reports name chunks
//! only, never their contents.

use super::domains::{ParamDomain, ParamDomains};
use super::{
    EquivMethod, EquivOptions, EquivResult, LiveContext, LiveOptions, bhav_param_limit,
    check_bhav_equivalent_combined, context_mismatch, reads_no_call_operands,
};
use crate::arity_catalog::vm_kept_bhavs;
use crate::error::{Error, Result};
use crate::ir::BhavChunk;
use crate::lift::lift_bhav;
use sims_formats::behavior::Behavior;
use sims_formats::execution::constants;
use sims_formats::iff::{Chunk, Iff};
use std::collections::{BTreeMap, BTreeSet};

/// Options for [`check_iff_equivalent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IffEquivOptions {
    /// Bounds for the lockstep checker.
    pub lockstep: EquivOptions,
    /// Bounds and observation policy for the live-output checker.
    pub live: LiveOptions,
    /// Attributes whatever runs these trees as `me` is known to have (see
    /// [`LiveContext::my_attribute_count`]).
    pub my_attribute_count: usize,
    /// Compare only this `BHAV`.
    pub tree: Option<u16>,
    /// Arities of the global (256 to 4095) and semi-global (8192 and up) libraries the
    /// game loads, by tree ID, so calls into them with more than four inline arguments
    /// compare exactly. Entries for private IDs are ignored: those come from the file.
    pub extra_arities: BTreeMap<u16, usize>,
    /// Trees of those libraries that take no arguments and are not indexed
    /// ([`library_argless_from_iff`], [`LiveContext::argless_callees`]). An ID counts
    /// only when `extra_arities` also gives it 0 arguments. Private IDs are ignored
    /// here too.
    pub extra_argless: BTreeSet<u16>,
    /// Values tree parameters are assumed to hold on entry (see
    /// [`LiveContext::param_domains`]). Empty by default, which assumes nothing. The
    /// files' own calls to a constrained tree are checked against it
    /// ([`IffEquivReport::assumptions`]).
    pub param_domains: ParamDomains,
}

impl Default for IffEquivOptions {
    fn default() -> Self {
        Self {
            lockstep: EquivOptions::default(),
            live: LiveOptions::default(),
            my_attribute_count: super::GUARANTEED_ATTRIBUTES,
            tree: None,
            extra_arities: BTreeMap::new(),
            extra_argless: BTreeSet::new(),
            param_domains: ParamDomains::new(),
        }
    }
}

/// Whether the VM runs tree `id` from the file of the object that runs it: only the
/// private trees, 4096 to 8191. A call to 256..=4095 runs the global library's tree and
/// one to 8192 and up the semi-global library's, whatever the object's own file holds
/// under that ID (`program.rs:1015`, `interpreter.rs:695`).
#[must_use]
pub fn runs_from_own_file(id: u16) -> bool {
    (4096..=8191).contains(&id)
}

/// Why `sims-antics` will not compare tree `id` of an object's file, if it will not.
#[must_use]
pub fn not_run_from_file(id: u16) -> Option<String> {
    (!runs_from_own_file(id)).then(|| {
        format!(
            "the VM never runs BHAV #{id} from an object's file: it takes {} from the {} \
             library",
            if id < 256 {
                "IDs below 256 as primitives, not trees, and 256 to 4095"
            } else if id < 4096 {
                "256 to 4095"
            } else {
                "8192 and up"
            },
            if id < 4096 { "global" } else { "semi-global" }
        )
    })
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
    /// Every parameter domain assumed ([`IffEquivOptions::param_domains`]): applied or
    /// not and why, and how the files' own calls to the tree stand against it.
    pub assumptions: Vec<String>,
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
    if let Some(tree) = opts.tree {
        if let Some(why) = not_run_from_file(tree) {
            return Err(Error::new(why));
        }
        if !has_bhav(&iff_a, tree) && !has_bhav(&iff_b, tree) {
            return Err(Error::new(format!("neither file contains BHAV #{tree}")));
        }
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
        argless_callees: Some(argless(iff, &opts.extra_argless, &opts.extra_arities)),
        param_domains: opts.param_domains.clone(),
    };
    let (ctx_a, ctx_b) = (ctx(&iff_a), ctx(&iff_b));
    // The structure check leaves the same chunks, in the same order, on both sides.
    let (trees_a, trees_b) = (lift_all(&iff_a), lift_all(&iff_b));
    // The trees a call can reach in the file: only private ones. A call to any other ID
    // runs a library tree, which [`tree_reach`] treats as opaque.
    let ids: BTreeSet<u16> = iff_a
        .chunks
        .iter()
        .filter(|c| &c.tag == b"BHAV" && runs_from_own_file(c.id))
        .map(|c| c.id)
        .collect();
    let needed: BTreeSet<u16> = match opts.tree {
        Some(tree) => tree_reach(tree, &ids, &trees_a, &trees_b).trees,
        None => ids.clone(),
    };
    let mut verdicts: BTreeMap<u16, (ResourceVerdict, Vec<String>)> = BTreeMap::new();
    for (ca, cb) in iff_a.chunks.iter().zip(&iff_b.chunks) {
        if &ca.tag == b"BHAV" && !runs_from_own_file(ca.id) && opts.tree.is_none() {
            // The VM never runs it from this file, so it is compared as bytes only.
            let verdict = if iff_a.data(ca) == iff_b.data(cb) {
                ResourceVerdict::Identical
            } else {
                ResourceVerdict::Inconclusive {
                    why: not_run_from_file(ca.id).unwrap_or_default(),
                }
            };
            verdicts.insert(ca.id, (verdict, Vec::new()));
        } else if &ca.tag == b"BHAV" && needed.contains(&ca.id) {
            let mut notes = Vec::new();
            let verdict = compare_bhav(
                ca.id, ca, cb, &iff_a, &iff_b, &ctx_a, &ctx_b, opts, &mut notes,
            );
            verdicts.insert(ca.id, (verdict, notes));
        }
    }
    // As a tree in another's reach, each tree must be proven under the stricter
    // policy (see [`dependency_options`]).
    let strict = IffEquivOptions {
        live: dependency_options(&opts.live),
        ..opts.clone()
    };
    let mut dependency_verdicts = verdicts.clone();
    let (assumptions, violations) =
        domain_calls(&opts.param_domains, &iff_a, &iff_b, &ctx_a, &ctx_b);
    report.assumptions = assumptions;
    if strict.live != opts.live {
        for (ca, cb) in iff_a.chunks.iter().zip(&iff_b.chunks) {
            if &ca.tag == b"BHAV"
                && runs_from_own_file(ca.id)
                && matches!(
                    verdicts.get(&ca.id),
                    Some((ResourceVerdict::Equivalent { .. }, _))
                )
            {
                let verdict = compare_bhav(
                    ca.id,
                    ca,
                    cb,
                    &iff_a,
                    &iff_b,
                    &ctx_a,
                    &ctx_b,
                    &strict,
                    &mut Vec::new(),
                );
                dependency_verdicts.insert(ca.id, (verdict, Vec::new()));
            }
        }
    }
    // A tree proven equivalent only for its assumed domain is not shown the same where
    // the file itself calls it outside the domain.
    for (id, why) in &violations {
        for map in [&mut verdicts, &mut dependency_verdicts] {
            if let Some((verdict @ ResourceVerdict::Equivalent { .. }, _)) = map.get_mut(id) {
                *verdict = ResourceVerdict::Inconclusive { why: why.clone() };
            }
        }
    }
    for ca in &iff_a.chunks {
        let is_bhav = &ca.tag == b"BHAV";
        if is_bhav && opts.tree.is_some_and(|tree| ca.id != tree) {
            continue;
        }
        let (verdict, notes) = if is_bhav && runs_from_own_file(ca.id) {
            let (verdict, notes) = verdicts[&ca.id].clone();
            (
                needs_callees(
                    ca.id,
                    verdict,
                    &ids,
                    &trees_a,
                    &trees_b,
                    &dependency_verdicts,
                ),
                notes,
            )
        } else if is_bhav {
            verdicts[&ca.id].clone()
        } else {
            (ResourceVerdict::Identical, Vec::new())
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

/// How each assumed parameter domain stands in the two files: whether it applies (to a
/// private tree both files define, not kind 4, below its argument count) and, for each
/// call to it from a tree of either file, whether the call is **discharged** (every
/// constrained argument is a literal inside the domain), **violates** the domain (a
/// literal outside it), or is **not discharged** (it passes temps, or its arguments run
/// past the tree's end). Returns the report lines and, per tree, why a violation makes
/// its proof not apply.
fn domain_calls(
    domains: &ParamDomains,
    iff_a: &Iff,
    iff_b: &Iff,
    ctx_a: &LiveContext,
    ctx_b: &LiveContext,
) -> (Vec<String>, BTreeMap<u16, String>) {
    const LISTED: usize = 8;
    let mut lines = Vec::new();
    let mut violations: BTreeMap<u16, String> = BTreeMap::new();
    let kept = |iff: &Iff| -> BTreeMap<u16, Behavior> {
        vm_kept_bhavs(iff)
            .chunks
            .into_iter()
            .filter_map(|c| Behavior::parse(iff.data(c)).ok().map(|b| (c.id, b)))
            .collect()
    };
    if domains.is_empty() {
        return (lines, violations);
    }
    let sides = [("A", kept(iff_a), ctx_a), ("B", kept(iff_b), ctx_b)];
    for (&tree, params) in domains {
        let shape = sides
            .iter()
            .map(|(_, trees, _)| trees.get(&tree).map(|t| (t.kind, bhav_param_limit(t))))
            .collect::<Vec<_>>();
        let not_applied = if !runs_from_own_file(tree) {
            Some(not_run_from_file(tree).unwrap_or_default())
        } else if shape.iter().any(Option::is_none) {
            Some("not both files define it".to_owned())
        } else if shape.iter().flatten().any(|&(kind, _)| kind == 4) {
            Some("it is kind 4, so its first parameter is the entry node".to_owned())
        } else {
            None
        };
        let args = shape.iter().flatten().map(|&(_, n)| n).min().unwrap_or(0);
        let mut applied: Vec<(u16, &ParamDomain)> = Vec::new();
        for (&index, domain) in params {
            let why = not_applied.clone().or_else(|| {
                (usize::from(index) >= args).then(|| format!("the tree takes {args} argument(s)"))
            });
            match why {
                Some(why) => lines.push(format!(
                    "BHAV #{tree} param[{index}] in {{{domain}}}: NOT APPLIED ({why})"
                )),
                None => applied.push((index, domain)),
            }
        }
        if applied.is_empty() {
            continue;
        }
        let (mut discharged, mut open, mut violated) = (0usize, Vec::new(), Vec::new());
        for (side, trees, ctx) in &sides {
            let arity = ctx
                .callee_arities
                .as_ref()
                .and_then(|m| m.get(&tree).copied());
            for (&caller, bhav) in trees.iter().filter(|(id, _)| runs_from_own_file(**id)) {
                for (node, ins) in bhav.instructions.iter().enumerate() {
                    if ins.opcode != tree {
                        continue;
                    }
                    let at = format!("{side} #{caller} node {node}");
                    let words = match arity {
                        _ if ins.operands == [0xFF; 8] => None,
                        Some(n) => inline_call_words(&bhav.instructions, node, n),
                        None => None,
                    };
                    let Some(words) = words else {
                        open.push(at);
                        continue;
                    };
                    let bad: Vec<String> = applied
                        .iter()
                        .filter_map(|&(i, d)| {
                            let v = *words.get(usize::from(i))?;
                            (!d.contains(v)).then(|| format!("param[{i}] = {v}"))
                        })
                        .collect();
                    if bad.is_empty() {
                        discharged += 1;
                    } else {
                        violated.push(format!("{at} passes {}", bad.join(", ")));
                    }
                }
            }
        }
        let list = |items: &[String]| {
            let mut text = items
                .iter()
                .take(LISTED)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ");
            if items.len() > LISTED {
                text.push_str(&format!("; and {} more", items.len() - LISTED));
            }
            text
        };
        for (index, domain) in &applied {
            lines.push(format!(
                "BHAV #{tree} param[{index}] in {{{domain}}}: applied"
            ));
        }
        lines.push(format!(
            "BHAV #{tree} callers in the files: {discharged} discharged, {} not discharged{}, {} violating{}",
            open.len(),
            if open.is_empty() {
                String::new()
            } else {
                format!(" (assumed: {})", list(&open))
            },
            violated.len(),
            if violated.is_empty() {
                String::new()
            } else {
                format!(" ({})", list(&violated))
            }
        ));
        if !violated.is_empty() {
            violations.insert(
                tree,
                format!(
                    "equivalent only for the assumed parameter domain, which a call in the \
                     files violates: {}",
                    list(&violated)
                ),
            );
        }
    }
    (lines, violations)
}

/// The `arity` argument words a call at `node` passes inline: its operand words, then
/// (past four) the bytes of the following nodes (`interpreter.rs:1539`). `None` when they
/// run past the tree's end.
fn inline_call_words(
    instructions: &[sims_formats::behavior::Instruction],
    node: usize,
    arity: usize,
) -> Option<Vec<i16>> {
    let mut bytes: Vec<u8> = instructions.get(node)?.operands.to_vec();
    for next in &instructions[node + 1..] {
        if bytes.len() >= 2 * arity {
            break;
        }
        bytes.extend_from_slice(&next.opcode.to_le_bytes());
        bytes.extend_from_slice(&[next.on_true, next.on_false]);
        bytes.extend_from_slice(&next.operands);
    }
    if bytes.len() < 2 * arity {
        return None;
    }
    Some(
        bytes
            .chunks_exact(2)
            .take(arity)
            .map(|w| i16::from_le_bytes([w[0], w[1]]))
            .collect(),
    )
}

/// Every `BHAV` of the file that lifts, by ID.
fn lift_all(iff: &Iff) -> BTreeMap<u16, BhavChunk> {
    iff.chunks
        .iter()
        .filter(|c| &c.tag == b"BHAV")
        .filter_map(|c| lift(iff, c).ok().map(|b| (c.id, b)))
        .collect()
}

/// The observation policy a tree in another tree's reach is proven under: the
/// caller's, except that the last result and the frame residue are always observed.
///
/// `--ignore-last-result` and `--ignore-frame-residue` describe how the checked tree is
/// run: by `run_tree`, whose frame records no object use, so its return releases none
/// and cannot fault once its frame is written (`interpreter.rs:1230`,
/// `object_uses.rs:128`). A tree in its reach may instead be run by a primitive as an
/// object use (Run Tree By Name in mode 2, `count_use`). Its return then releases the
/// use, may end the running action with the returning node's result
/// (`Machine::end_action`), and can fault after its frame is written to the frame
/// storage, so both are observable there.
fn dependency_options(live: &LiveOptions) -> LiveOptions {
    LiveOptions {
        observe_last_result: true,
        observe_frame_residue: true,
        ..*live
    }
}

/// `verdict` for tree `id`, unless a tree it may run was not shown to behave the same.
///
/// Both checkers treat a call (and a primitive that runs a tree) as an event: equal
/// events from equal states do the same thing only if the trees they run do. So a tree
/// is the same only if every tree in its reach ([`tree_reach`]) is the same bytes or was
/// itself proven equivalent. Cycles are fine: by induction on the length of a run
/// within the instruction budget, every tree of a reach whose trees were each proven
/// equivalent under that assumption behaves the same.
fn needs_callees(
    id: u16,
    verdict: ResourceVerdict,
    ids: &BTreeSet<u16>,
    trees_a: &BTreeMap<u16, BhavChunk>,
    trees_b: &BTreeMap<u16, BhavChunk>,
    verdicts: &BTreeMap<u16, (ResourceVerdict, Vec<String>)>,
) -> ResourceVerdict {
    if !matches!(
        verdict,
        ResourceVerdict::Identical | ResourceVerdict::Equivalent { .. }
    ) {
        return verdict;
    }
    let reach = tree_reach(id, ids, trees_a, trees_b);
    // With a primitive in reach, the tree itself may be run as an object use too.
    for dep in reach.trees.iter().filter(|&&t| t != id || reach.opaque) {
        match verdicts.get(dep).map(|(v, _)| v) {
            Some(ResourceVerdict::Identical | ResourceVerdict::Equivalent { .. }) => {}
            Some(ResourceVerdict::Different { .. } | ResourceVerdict::Inconclusive { .. })
            | None => {
                if *dep == id {
                    return ResourceVerdict::Inconclusive {
                        why: format!(
                            "a primitive in the reach of tree #{id} may run it as an object \
                             use, whose return records its last result and frame, and it was \
                             not shown to behave the same with those observed"
                        ),
                    };
                }
                return ResourceVerdict::Inconclusive {
                    why: format!(
                        "tree #{id} may run tree #{dep}{}, which was not shown to behave \
                         the same",
                        if reach.opaque {
                            " (a primitive or a call outside the file may run any tree of \
                             the object)"
                        } else {
                            ""
                        }
                    ),
                };
            }
        }
    }
    verdict
}

/// The trees a run of a tree may enter, over both sides' trees.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reach {
    /// The tree itself and every tree it may run.
    pub trees: BTreeSet<u16>,
    /// Whether one of them has a primitive or calls a tree neither side defines: those
    /// may run any tree of the object (Run Tree By Name, functional trees, found
    /// actions, Idle For Input, routing callbacks, Create, ...), and read any resource
    /// of its file.
    pub opaque: bool,
    /// Whether one of them reads a tuning constant (scope 26), a `BCON` of a file.
    pub reads_constants: bool,
}

impl Reach {
    /// Whether a run may read resources of the trees' files other than the trees.
    #[must_use]
    pub fn reads_file(&self) -> bool {
        self.opaque || self.reads_constants
    }
}

/// `root` and every tree a run of it may enter. `ids` is every tree either side
/// defines; `a` and `b` are the ones that lift. Every instruction counts, reachable or
/// not (inline call arguments included): a call enters its callee, and a primitive, a
/// call to a tree outside `ids` or a store to the selected Sim ([`writes_selected_sim`])
/// may enter every tree in `ids`.
#[must_use]
pub fn tree_reach(
    root: u16,
    ids: &BTreeSet<u16>,
    a: &BTreeMap<u16, BhavChunk>,
    b: &BTreeMap<u16, BhavChunk>,
) -> Reach {
    let mut reach = Reach::default();
    let mut work = vec![root];
    while let Some(id) = work.pop() {
        if !reach.trees.insert(id) {
            continue;
        }
        for bhav in [a.get(&id), b.get(&id)].into_iter().flatten() {
            for ins in &bhav.instructions {
                match ins.opcode {
                    2 if !writes_selected_sim(&ins.operands) => {
                        reach.reads_constants |= ins.operands[6] == 26 || ins.operands[7] == 26;
                    }
                    callee if callee >= 256 && ids.contains(&callee) => work.push(callee),
                    _ => {
                        if !reach.opaque {
                            reach.opaque = true;
                            work.extend(ids.iter().copied());
                        }
                    }
                }
            }
        }
    }
    reach
}

/// Whether an `Expression` with these operands stores to global 3, the selected Sim.
/// In co-op that store selects the Sim, which can run object callbacks (the selection
/// indicator's placement, `memory.rs:846`), so it runs trees like a primitive does.
/// Comparisons and the flag test only read (`expression.rs`).
fn writes_selected_sim(ops: &[u8; 8]) -> bool {
    let (op, lhs_scope, lhs) = (ops[5], ops[6], i16::from_le_bytes([ops[0], ops[1]]));
    lhs_scope == 6 && lhs == 3 && !matches!(op, 0 | 1 | 2 | 8 | 14 | 15 | 16)
}

/// Whether two trees are the same bytes where the VM reads them: header words and
/// every instruction.
#[must_use]
pub fn same_tree_bytes(a: &BhavChunk, b: &BhavChunk) -> bool {
    let words = |x: &BhavChunk| {
        (
            x.format,
            x.kind,
            x.args,
            x.locals,
            x.instructions
                .iter()
                .map(|i| (i.opcode, i.on_true, i.on_false, i.operands))
                .collect::<Vec<_>>(),
        )
    };
    words(a) == words(b)
}

/// Checks tree `root` of two sides that are not whole files compared chunk for chunk
/// (`sims-antics equiv`: a compiled source against an IFF). The tree must be
/// equivalent, and so must every tree it may run ([`tree_reach`]): each the same bytes
/// on both sides, or proven equivalent by [`check_bhav_equivalent_combined`]. A tree in
/// reach that only one side defines makes the result `Inconclusive`. When a run may
/// read other resources of its file (a primitive, a call into the game, a tuning
/// constant), both sides must be IFF files that differ in `BHAV` bodies alone.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn check_tree_with_callees(
    root: u16,
    a: &BTreeMap<u16, BhavChunk>,
    file_a: Option<&[u8]>,
    b: &BTreeMap<u16, BhavChunk>,
    file_b: Option<&[u8]>,
    arities: (&BTreeMap<u16, usize>, &BTreeMap<u16, usize>),
    lockstep: &EquivOptions,
    live: &LiveOptions,
) -> (EquivMethod, EquivResult) {
    let inconclusive = |reason: String| {
        (
            EquivMethod::LiveOutput,
            EquivResult::Inconclusive { reason },
        )
    };
    let parsed = |file: Option<&[u8]>| file.and_then(|f| Iff::parse(f.to_vec()).ok());
    let (iff_a, iff_b) = (parsed(file_a), parsed(file_b));
    // Constant tables only when both sides have a file; a run that may read them then
    // needs the two files to match outside BHAV bodies (below).
    let both = iff_a.is_some() && iff_b.is_some();
    // A private callee reads no operand bytes when this side's argument count for it
    // is 0 and this side's tree is not indexed. Library trees are not known here.
    let ctx = |arities: &BTreeMap<u16, usize>,
               trees: &BTreeMap<u16, BhavChunk>,
               iff: &Option<Iff>| LiveContext {
        callee_arities: Some(arities.clone()),
        local_bcons: iff.as_ref().filter(|_| both).map(bcons),
        argless_callees: Some(
            arities
                .iter()
                .filter(|&(id, &n)| {
                    n == 0 && runs_from_own_file(*id) && trees.get(id).is_some_and(|t| t.kind != 4)
                })
                .map(|(&id, _)| id)
                .collect(),
        ),
        ..LiveContext::default()
    };
    if let Some(why) = not_run_from_file(root) {
        return inconclusive(why);
    }
    // Only private arities come from the sides' trees (see [`arities`]).
    let private = |m: &BTreeMap<u16, usize>| -> BTreeMap<u16, usize> {
        m.iter()
            .filter(|&(&id, _)| runs_from_own_file(id))
            .map(|(&id, &n)| (id, n))
            .collect()
    };
    let (arities_a, arities_b) = (private(arities.0), private(arities.1));
    let (ctx_a, ctx_b) = (ctx(&arities_a, a, &iff_a), ctx(&arities_b, b, &iff_b));
    let (Some(ra), Some(rb)) = (a.get(&root), b.get(&root)) else {
        return inconclusive(format!("tree #{root} is not defined on both sides"));
    };
    let first = check_bhav_equivalent_combined(ra, &ctx_a, rb, &ctx_b, lockstep, live);
    if !matches!(first.1, EquivResult::Equivalent { .. }) {
        return first;
    }
    let ids: BTreeSet<u16> = a
        .keys()
        .chain(b.keys())
        .copied()
        .filter(|&id| runs_from_own_file(id))
        .collect();
    let reach = tree_reach(root, &ids, a, b);
    if reach.reads_file() {
        let problem = match (&iff_a, &iff_b) {
            (Some(x), Some(y)) => structure_problem(x, y),
            _ => Some("one side is not an IFF file".to_owned()),
        };
        if let Some(problem) = problem {
            return inconclusive(format!(
                "a run of tree #{root} may read other resources of its file ({}), and {problem}",
                if reach.opaque {
                    "a primitive or a call outside the file"
                } else {
                    "a tuning constant"
                }
            ));
        }
    }
    let strict = dependency_options(live);
    if reach.opaque
        && strict != *live
        && !matches!(
            check_bhav_equivalent_combined(ra, &ctx_a, rb, &ctx_b, lockstep, &strict).1,
            EquivResult::Equivalent { .. }
        )
    {
        return inconclusive(format!(
            "a primitive in the reach of tree #{root} may run it as an object use, whose \
             return records its last result and frame, and it was not shown to behave the \
             same with those observed"
        ));
    }
    for dep in reach.trees.iter().filter(|&&t| t != root) {
        let (Some(da), Some(db)) = (a.get(dep), b.get(dep)) else {
            return inconclusive(format!(
                "tree #{root} may run tree #{dep}, which only {} defines",
                if a.contains_key(dep) { "A" } else { "B" }
            ));
        };
        if same_tree_bytes(da, db) {
            continue;
        }
        let proven = super::live::frame_dims(da).0 == super::live::frame_dims(db).0
            && matches!(
                check_bhav_equivalent_combined(da, &ctx_a, db, &ctx_b, lockstep, &strict).1,
                EquivResult::Equivalent { .. }
            );
        if !proven {
            return inconclusive(format!(
                "tree #{root} may run tree #{dep}, which differs between the sides and was \
                 not proven equivalent"
            ));
        }
    }
    first
}

fn has_bhav(iff: &Iff, id: u16) -> bool {
    iff.chunks.iter().any(|c| &c.tag == b"BHAV" && c.id == id)
}

/// Callee argument counts as the VM resolves them for code of an object whose file is
/// `iff`: its private `BHAV`s (`sims-vm` loads deleted chunks too, `program.rs:601`),
/// and `extra` for the global and semi-global libraries. A tree the file holds in a
/// library's range is never the one a call runs, so it supplies no arity.
fn arities(iff: &Iff, extra: &BTreeMap<u16, usize>) -> BTreeMap<u16, usize> {
    let mut map: BTreeMap<u16, usize> = extra
        .iter()
        .filter(|&(&id, _)| !runs_from_own_file(id))
        .map(|(&id, &n)| (id, n))
        .collect();
    map.extend(
        loaded_arities(iff)
            .into_iter()
            .filter(|&(id, _)| runs_from_own_file(id))
            .filter_map(|(id, n)| Some((id, n?))),
    );
    map
}

/// The argument count of each tree of a file as the VM's loader keeps the trees
/// (`load_library_with_pool`, `program.rs:588`, selected by [`vm_kept_bhavs`]): every
/// `BHAV` chunk in physical order, deleted ones included, the last copy of an ID
/// replacing the earlier ones. `None` when that last copy does not parse (the loader
/// then refuses the file, so the count is unknown).
fn loaded_arities(iff: &Iff) -> BTreeMap<u16, Option<usize>> {
    vm_kept_bhavs(iff)
        .chunks
        .into_iter()
        .map(|c| {
            (
                c.id,
                Behavior::parse(iff.data(c))
                    .ok()
                    .map(|bhav| bhav_param_limit(&bhav)),
            )
        })
        .collect()
}

/// The trees of a file, as the VM's loader keeps them ([`loaded_arities`]), that a
/// call runs the same with any operand bytes: no arguments, not indexed
/// ([`LiveContext::argless_callees`]).
fn loaded_argless(iff: &Iff) -> BTreeSet<u16> {
    // The last copy of an ID is the one the loader keeps, as in [`loaded_arities`].
    let kept: BTreeMap<u16, bool> = vm_kept_bhavs(iff)
        .chunks
        .into_iter()
        .map(|c| {
            let reads_none = Behavior::parse(iff.data(c)).is_ok_and(|b| reads_no_call_operands(&b));
            (c.id, reads_none)
        })
        .collect();
    kept.into_iter()
        .filter_map(|(id, reads_none)| reads_none.then_some(id))
        .collect()
}

/// The callees of a file's trees that read no call operand bytes: its own private
/// trees, and the library trees in `extra` that `extra_arities` also gives no
/// arguments (a library tree the two disagree on, or one with no stated count, is
/// treated as reading its operands).
fn argless(
    iff: &Iff,
    extra: &BTreeSet<u16>,
    extra_arities: &BTreeMap<u16, usize>,
) -> BTreeSet<u16> {
    let mut set: BTreeSet<u16> = extra
        .iter()
        .copied()
        .filter(|&id| !runs_from_own_file(id) && extra_arities.get(&id) == Some(&0))
        .collect();
    set.extend(
        loaded_argless(iff)
            .into_iter()
            .filter(|&id| runs_from_own_file(id)),
    );
    set
}

/// [`loaded_argless`] of a library file, for [`IffEquivOptions::extra_argless`],
/// limited to the IDs the VM takes from that library (`program.rs:1015`): 256 to 4095
/// from the global file, 8192 and up from a semi-global one. The same ID in the other
/// library's range names a different tree.
///
/// # Errors
///
/// When the file is not an IFF.
pub fn library_argless_from_iff(iff_bytes: &[u8], semi_global: bool) -> Result<BTreeSet<u16>> {
    let in_range = |id: u16| {
        if semi_global {
            id >= 8192
        } else {
            (256..=4095).contains(&id)
        }
    };
    Ok(loaded_argless(&Iff::parse(iff_bytes.to_vec())?)
        .into_iter()
        .filter(|&id| in_range(id))
        .collect())
}

/// [`loaded_arities`] of a library file (`--global`, `--semi-global`, or the file a
/// source is compared against).
///
/// # Errors
///
/// When the file is not an IFF.
pub fn library_arities_from_iff(iff_bytes: &[u8]) -> Result<BTreeMap<u16, Option<usize>>> {
    Ok(loaded_arities(&Iff::parse(iff_bytes.to_vec())?))
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
///
/// Every byte of both files is accounted for: an IFF is its 64-byte header followed by
/// its chunks, back to back (`Iff::parse` rejects anything else). The header must be
/// the same bytes, except that its resource-map offset (bytes 60..64) may differ when
/// both name the same chunk. Each chunk's 76-byte header must be the same bytes (label
/// field included, past its terminator), except a `BHAV`'s size. Each chunk that is not
/// a `BHAV` must have the same payload, except that the offsets in a resource map may
/// differ when they name the same chunks; anything else in the map, its padding and
/// label bytes included, must be the same bytes.
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
    if a.bytes.get(..MAP_OFFSET) != b.bytes.get(..MAP_OFFSET)
        || file_map_target(a) != file_map_target(b)
    {
        differ.push("the file header".to_owned());
    }
    for (i, (ca, cb)) in a.chunks.iter().zip(&b.chunks).enumerate() {
        let is_bhav = &ca.tag == b"BHAV";
        let (ha, hb) = (chunk_header(a, ca), chunk_header(b, cb));
        let what = if (ca.tag, ca.id) != (cb.tag, cb.id) {
            Some(format!(
                "chunk {i} is {} in A but {} in B",
                name(ca),
                name(cb)
            ))
        } else if ha[12..] != hb[12..] {
            // The whole 64-byte label field, bytes after its terminator included.
            Some(format!("{} has different labels", name(ca)))
        } else if ha[10..12] != hb[10..12] {
            Some(format!("{} has different header flags", name(ca)))
        } else if a.resource_flags(ca) != b.resource_flags(cb) {
            Some(format!("{} has different resource-map flags", name(ca)))
        } else if &ca.tag == b"rsmp" {
            (ha != hb || map_bytes(a, ca) != map_bytes(b, cb))
                .then(|| format!("{} (the resource map) differs", name(ca)))
        } else if !is_bhav && (ha != hb || a.data(ca) != b.data(cb)) {
            Some(format!("{} has different contents", name(ca)))
        } else {
            // Tag, ID, flags and label are compared above; a BHAV's size may differ.
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

/// Where the file header's resource-map offset starts (an IFF 2.5 header keeps it in
/// bytes 60..64, big-endian; `sims-formats` `iff.rs`).
const MAP_OFFSET: usize = 60;

/// A file offset, as the chunk it names (by physical index) or, when it names none,
/// the raw value.
type Target = std::result::Result<usize, u32>;

fn target(iff: &Iff, offset: u32) -> Target {
    iff.chunks
        .iter()
        .position(|c| c.offset as u64 == u64::from(offset))
        .ok_or(offset)
}

/// The file header's resource-map offset, as the chunk it names.
fn file_map_target(iff: &Iff) -> Option<Target> {
    let raw: [u8; 4] = iff.bytes.get(MAP_OFFSET..MAP_OFFSET + 4)?.try_into().ok()?;
    Some(target(iff, u32::from_be_bytes(raw)))
}

/// A chunk's raw 76-byte header: tag, size, ID, flags and the whole label field.
fn chunk_header<'a>(iff: &'a Iff, chunk: &Chunk) -> &'a [u8] {
    &iff.bytes[chunk.offset..chunk.payload.start]
}

/// A resource map's bytes with each entry's offset replaced by the chunk it names. The
/// walk follows the `sims-formats` map parser (`parse_rsmp_raw`) field for field and
/// copies every other byte as it is; a map it cannot walk compares by its raw bytes.
fn map_bytes(iff: &Iff, chunk: &Chunk) -> std::result::Result<Vec<u8>, Vec<u8>> {
    let data = iff.data(chunk);
    walk_map(iff, data).ok_or_else(|| data.to_vec())
}

fn walk_map(iff: &Iff, data: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0usize;
    let mut out = Vec::with_capacity(data.len() + 16);
    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        let bytes = data.get(*pos..pos.checked_add(n)?)?;
        *pos += n;
        Some(bytes)
    };
    let le32 = |bytes: &[u8]| u32::from_le_bytes(bytes.try_into().expect("four bytes"));
    // Eight zero bytes, "pmsr", an unused word, and the type count.
    let head = take(&mut pos, 20)?;
    if head[..8] != [0; 8] || &head[8..12] != b"pmsr" {
        return None;
    }
    out.extend_from_slice(head);
    let types = le32(&head[16..20]);
    for _ in 0..types {
        // The type's tag and entry count.
        let ty = take(&mut pos, 8)?;
        out.extend_from_slice(ty);
        for _ in 0..le32(&ty[4..8]) {
            let offset = le32(take(&mut pos, 4)?);
            match target(iff, offset) {
                Ok(index) => {
                    out.push(0);
                    out.extend_from_slice(&(index as u32).to_le_bytes());
                }
                Err(raw) => {
                    out.push(1);
                    out.extend_from_slice(&raw.to_le_bytes());
                }
            }
            // ID, flags, the NUL-terminated label, and a pad byte to an even offset.
            out.extend_from_slice(take(&mut pos, 4)?);
            let len = data.get(pos..)?.iter().position(|&b| b == 0)? + 1;
            out.extend_from_slice(take(&mut pos, len)?);
            if pos & 1 != 0 {
                out.extend_from_slice(take(&mut pos, 1)?);
            }
        }
    }
    (pos == data.len()).then_some(out)
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
                    let assumed = ctx_a
                        .param_domains
                        .get(&id)
                        .filter(|_| a.kind != 4)
                        .map(|d| {
                            d.iter()
                                .filter(|&(&i, _)| usize::from(i) < args_a)
                                .map(|(i, d)| format!("param[{i}] in {{{d}}}"))
                                .collect::<Vec<_>>()
                        })
                        .filter(|d| !d.is_empty())
                        .map(|d| format!(", assuming {}", d.join(", ")))
                        .unwrap_or_default();
                    format!("live-output: {paths_verified} paths, {states_checked} steps{assumed}")
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
