//! High-level `.antics` v2 decompiler: compiled object `.iff` -> readable v2 source.
//!
//! Every private tree is lifted with the structured `.air` lifter, parsed with the `.air`
//! parser and translated statement by statement into v2 idioms. Anything without a v2
//! spelling stays a minimal `raw { ... }` statement holding the lifted `.air`.
//!
//! The decompiler checks what it writes. It compiles the generated object with the real
//! v2 compiler and compares every original private tree with its recompiled twin using
//! [`check_bhav_equivalent_combined`]. A statement the compiler rejects falls back to
//! `raw`, and a tree that is not shown equivalent falls back to an all-`raw` body (see
//! `docs/decisions/20261123-antics-v2-fmt-and-decompiler.md`). The finished text always
//! goes through [`format_v2_source`].

use crate::arity_catalog::{bhav_arities_in_range, vm_kept_bhavs};
use crate::ast::{
    AssignOp, CallTarget, CompareOp, Condition, LocalRef, Operand, OperandRef, PrimitiveCall, Stmt,
    Target,
};
use crate::error::{Error, Result};
use crate::ir::BhavChunk;
use crate::lift::{format_bhav_labeled_air_with_arities, lift_bhav};
use crate::structure_lift::{
    LiftOptions, StructuredLiftOutcome, lift_bhav_structured_with_arities,
};
use crate::v2::ast::{is_reserved_type_name, object_type_ident};
use crate::v2::fmt::format_v2_source;
use crate::v2::parser::snake_case_ident;
use crate::v2::{PackResolverConfig, compile_expanded_v2_object, expand_v2_source_to_objects};
use crate::verify::{
    EquivOptions, EquivResult, LiveContext, LiveOptions, check_bhav_equivalent_combined,
    same_tree_bytes,
};
use sims_formats::behavior::{InteractionTable, ObjectDefinition};
use sims_formats::iff::{Chunk, Iff};
use sims_formats::strings::StringTable;
use std::collections::{BTreeMap, BTreeSet};

/// Private BHAV IDs a decompiled object declares as its own trees.
const PRIVATE_TREES: std::ops::RangeInclusive<u16> = 4096..=8191;
/// Private BCON chunk IDs a v2 `bcon` table can declare.
const PRIVATE_BCONS: std::ops::RangeInclusive<u16> = 4096..=4159;
/// Highest attribute slot a v2 `state` may bind (`parser::MAX_STATE_SLOT`).
const MAX_STATE_SLOT: i16 = 255;
/// OBJD words holding the main and init tree IDs (after the 32-bit version).
const OBJD_MAIN_WORD: usize = 3;
const OBJD_INIT_WORD: usize = 41;

/// Options for [`decompile_iff_to_v2`].
#[derive(Debug, Clone)]
pub struct DecompileV2Options {
    /// Argument counts of library trees the object calls (global `256..=4095`,
    /// semi-global `8192+`), e.g. from an install's arity catalog. Private trees' counts
    /// always come from the file itself.
    pub library_arities: BTreeMap<u16, usize>,
    /// Compile the generated source and check every tree against the original, falling
    /// back to `raw` where needed (default `true`). Without it the output is the most
    /// readable translation, unchecked.
    pub validate: bool,
}

impl Default for DecompileV2Options {
    fn default() -> Self {
        Self {
            library_arities: BTreeMap::new(),
            validate: true,
        }
    }
}

/// How a tree's body was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TreeStyle {
    /// v2 idioms, with `raw { ... }` only for statements without a v2 spelling.
    Idiomatic,
    /// The whole structured lifted body in one `raw { ... }` block.
    Raw,
    /// The labeled-CFG lift (`@n0:` nodes) in one `raw { ... }` block: the original
    /// nodes, one for one.
    RawLabeled,
}

/// What the validation found for one original tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeCheck {
    /// The recompiled tree has the original's exact bytes.
    Identical,
    /// The equivalence checker proved the recompiled tree equivalent.
    Equivalent,
    /// The checker could not decide.
    Inconclusive(String),
    /// The checker found a counterexample even for the all-`raw` body.
    Different(String),
    /// The tree is not in the output (unsupported header, see the reason).
    Omitted(String),
    /// Validation was off.
    NotChecked,
}

/// One original private tree in a [`DecompileV2Report`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecompiledTree {
    pub id: u16,
    pub style: TreeStyle,
    /// An `init`/`main` tree written as a plain `tree`, because an `init`/`main` block
    /// (which takes no `@frame`) did not reproduce it. The compiler then adds its own
    /// lifecycle tree at a fresh ID.
    pub standalone_lifecycle: bool,
    /// Statements left as `raw` inside an idiomatic body.
    pub raw_statements: usize,
    pub check: TreeCheck,
}

/// Decompiled source plus per-tree validation results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecompileV2Report {
    pub source: String,
    pub trees: Vec<DecompiledTree>,
    /// Whether the final source compiled (always `false` when validation is off).
    pub compiled: bool,
    /// Behaviour-relevant resource chunks of the original (`"STR# 300"`) that the
    /// recompiled object does not carry byte for byte. Tree checks cover code,
    /// constants, callee shapes and sounds; trees that read these resources (strings,
    /// interaction tables, slots) may still behave differently.
    pub resources_not_reproduced: Vec<String>,
}

impl DecompileV2Report {
    /// Trees whose check found a counterexample.
    #[must_use]
    pub fn counterexamples(&self) -> usize {
        self.trees
            .iter()
            .filter(|t| matches!(t.check, TreeCheck::Different(_)))
            .count()
    }
}

/// Decompiles a compiled object `.iff` into canonical `.antics` v2 source.
///
/// See [`decompile_iff_to_v2_report`] for the per-tree validation results.
pub fn decompile_iff_to_v2(iff: &Iff, options: &DecompileV2Options) -> Result<String> {
    Ok(decompile_iff_to_v2_report(iff, options)?.source)
}

/// Decompiles a compiled object `.iff` and reports how each private tree was written and
/// what validation found.
pub fn decompile_iff_to_v2_report(
    iff: &Iff,
    options: &DecompileV2Options,
) -> Result<DecompileV2Report> {
    let mut model = Model::build(iff, options)?;
    let mut plan = Plan::initial(&model);
    if !options.validate {
        let source = format_v2_source(&model.generate(&plan).text)?;
        let trees = model
            .trees
            .iter()
            .map(|t| DecompiledTree {
                id: t.id,
                style: plan.style(t),
                standalone_lifecycle: t.lifecycle.is_some() && plan.standalone.contains(&t.id),
                raw_statements: 0,
                check: t
                    .omitted
                    .clone()
                    .map_or(TreeCheck::NotChecked, TreeCheck::Omitted),
            })
            .collect();
        return Ok(DecompileV2Report {
            source,
            trees,
            compiled: false,
            resources_not_reproduced: Vec::new(),
        });
    }

    let mut cache: BTreeMap<(u16, String, String), TreeCheck> = BTreeMap::new();
    // Every round lowers at least one tree or statement, so this bound is never reached by
    // a run that converges; it only guards against a bug turning into a hang.
    let max_rounds = 8 + model.trees.len() * 4 + model.statement_count() * 2;
    for _ in 0..max_rounds {
        let generated = model.generate(&plan);
        match model.compile(&generated.text) {
            Err(Failure::V2 { line, message }) => {
                if !plan.lower_at_line(&model, &generated, line) {
                    // A tree that fails at its lowest level is left out.
                    let tree = line.and_then(|l| generated.tree_at_line(l.saturating_sub(1)));
                    if !tree.is_some_and(|id| model.omit(id, &message)) {
                        return Err(no_compile(&message));
                    }
                }
            }
            Err(Failure::Air { tree, message }) => {
                let handled = match tree {
                    Some(id) => plan.lower_tree(&model, id) || model.omit(id, &message),
                    None => plan.lower_all(&model),
                };
                if !handled {
                    return Err(no_compile(&message));
                }
            }
            Ok(recompiled) => {
                let mut changed = false;
                let mut checks = BTreeMap::new();
                for t in model.trees.iter().filter(|t| t.omitted.is_none()) {
                    let check = model.check_tree(t, &recompiled, &mut cache);
                    let proven = matches!(check, TreeCheck::Identical | TreeCheck::Equivalent);
                    if !proven && plan.lower_tree(&model, t.id) {
                        changed = true;
                    }
                    checks.insert(t.id, check);
                }
                if changed {
                    continue;
                }
                let source = format_v2_source(&generated.text)?;
                let trees = model
                    .trees
                    .iter()
                    .map(|t| DecompiledTree {
                        id: t.id,
                        style: plan.style(t),
                        standalone_lifecycle: t.lifecycle.is_some()
                            && plan.standalone.contains(&t.id),
                        raw_statements: generated.raw_counts.get(&t.id).copied().unwrap_or(0),
                        check: match &t.omitted {
                            Some(why) => TreeCheck::Omitted(why.clone()),
                            None => checks.remove(&t.id).unwrap_or(TreeCheck::NotChecked),
                        },
                    })
                    .collect();
                return Ok(DecompileV2Report {
                    source,
                    trees,
                    compiled: true,
                    resources_not_reproduced: model.resources_not_reproduced(&recompiled),
                });
            }
        }
    }
    Err(Error::new("decompiler validation did not converge"))
}

// ---------------------------------------------------------------------------------------
// Model: what the file holds
// ---------------------------------------------------------------------------------------

/// Which lifecycle block a tree is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    Init,
    Main,
}

impl Lifecycle {
    fn keyword(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Main => "main",
        }
    }
}

struct TreeModel {
    id: u16,
    label: String,
    ident: String,
    original: BhavChunk,
    /// Lifted `.air` lines (the whole tree, header included).
    lifted: Vec<String>,
    /// Index of the header line in `lifted`, and of the closing `}`.
    body_lines: (usize, usize),
    /// Body of the labeled-CFG lift, dedented.
    labeled: Vec<String>,
    /// Whether `lifted` is the structured lift (otherwise it is the labeled one).
    structured: bool,
    /// Parsed structured body (`None` for a labeled-CFG lift).
    body: Option<Vec<Stmt>>,
    /// Whether the idiomatic body runs as a Sim acting on this object (`(sim: Sim)`).
    sim_ctx: bool,
    lifecycle: Option<Lifecycle>,
    /// Why the tree cannot be written at all (unsupported header).
    omitted: Option<String>,
}

struct BconModel {
    id: u16,
    label: String,
    values: Vec<i16>,
}

struct Model {
    name: String,
    guid: u32,
    price: u16,
    extra_objds: usize,
    bcons: Vec<BconModel>,
    /// Private BCON chunk id -> reference name.
    bcon_names: BTreeMap<u16, String>,
    /// Library constant operand address -> entry name in `extern bcon library_consts`.
    extern_bcons: BTreeMap<u16, String>,
    /// Attribute slots referenced as `my.attr` / `stack.attr`.
    states: BTreeSet<i16>,
    /// GUIDs used more than once -> `const` name.
    guid_consts: BTreeMap<u32, String>,
    /// Library trees called with the caller's temps (`call N(temps)`).
    extern_calls: BTreeSet<u16>,
    trees: Vec<TreeModel>,
    interactions: Vec<String>,
    /// FWAV chunks written as `sound` declarations: chunk id -> sound event name.
    sounds: BTreeMap<u16, String>,
    /// FWAV chunks v2 cannot declare, and why.
    sounds_skipped: BTreeMap<u16, &'static str>,
    /// Original-side checker context.
    ctx_original: LiveContext,
    /// The sound event each original FWAV names, as the VM reads it (last chunk wins).
    original_sounds: BTreeMap<u16, String>,
    /// Behaviour-relevant non-code chunks of the original, as the VM reads them.
    original_resources: BTreeMap<([u8; 4], u16), Vec<u8>>,
    library_arities: BTreeMap<u16, usize>,
}

impl Model {
    fn build(iff: &Iff, options: &DecompileV2Options) -> Result<Self> {
        let objds: Vec<&Chunk> = iff.chunks.iter().filter(|c| &c.tag == b"OBJD").collect();
        let objd_chunk = objds
            .first()
            .ok_or_else(|| Error::new("the file has no OBJD chunk (not an object file)"))?;
        let objd_data = iff.data(objd_chunk);
        let objd = ObjectDefinition::parse(objd_data)?;
        let objd_word = |w: usize| -> Option<u16> {
            let at = 4 + w * 2;
            objd_data
                .get(at..at + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
        };
        let mut lifecycle: BTreeMap<u16, Lifecycle> = BTreeMap::new();
        for (word, kind) in [
            (OBJD_INIT_WORD, Lifecycle::Init),
            (OBJD_MAIN_WORD, Lifecycle::Main),
        ] {
            if let Some(id) = objd_word(word).filter(|id| PRIVATE_TREES.contains(id)) {
                lifecycle.entry(id).or_insert(kind);
            }
        }

        let private_arities = bhav_arities_in_range(iff, PRIVATE_TREES);
        let mut lift_arities = options.library_arities.clone();
        lift_arities.extend(private_arities.iter().map(|(&k, &v)| (k, v)));

        // BCON tables.
        let mut bcons = Vec::new();
        let mut bcon_names = BTreeMap::new();
        // `library_consts` is the extern table's name.
        let mut used_names = BTreeSet::from(["library_consts".to_owned()]);
        // The VM keeps the last chunk of an ID.
        let mut bcon_chunks: BTreeMap<u16, &Chunk> = BTreeMap::new();
        for c in iff.chunks.iter().filter(|c| &c.tag == b"BCON") {
            if PRIVATE_BCONS.contains(&c.id) {
                bcon_chunks.insert(c.id, c);
            }
        }
        for c in bcon_chunks.into_values() {
            let Ok(mut values) = sims_formats::execution::constants(iff.data(c)) else {
                continue;
            };
            // An operand addresses entries 0..128 only.
            values.truncate(BCON_ADDRESSABLE);
            if values.is_empty() {
                continue;
            }
            let mut label = object_type_ident(&c.label);
            if label == "Object"
                || label.starts_with(|ch: char| ch.is_ascii_digit())
                || is_reserved_type_name(&label)
            {
                label = format!("Bcon{label}");
            }
            let base = label.clone();
            let mut n = 0;
            while !used_names.insert(snake_case_ident(&label)) {
                n += 1;
                label = if n == 1 {
                    format!("{base}{}", c.id)
                } else {
                    format!("{base}{}_{n}", c.id)
                };
            }
            bcon_names.insert(c.id, label.clone());
            bcons.push(BconModel {
                id: c.id,
                label,
                values,
            });
        }

        // Trees.
        let kept = vm_kept_bhavs(iff);
        let mut trees = Vec::new();
        let mut used_idents = BTreeSet::new();
        for c in kept.chunks {
            if !PRIVATE_TREES.contains(&c.id) {
                continue;
            }
            let original = lift_bhav(c.id, &c.label, iff.data(c))?;
            let ident = unique_ident(&c.label, c.id, &mut used_idents);
            let mut tree = TreeModel {
                id: c.id,
                label: clean_label(&c.label),
                ident,
                lifted: Vec::new(),
                body_lines: (0, 0),
                labeled: Vec::new(),
                structured: false,
                body: None,
                sim_ctx: false,
                lifecycle: lifecycle.get(&c.id).copied(),
                omitted: None,
                original,
            };
            if !(0x8000..=0x8002).contains(&tree.original.format) {
                tree.omitted = Some(format!(
                    "BHAV format {:#06x} has no v2 spelling",
                    tree.original.format
                ));
                trees.push(tree);
                continue;
            }
            // v2 refuses a 0x8000/0x8001 frame with more than four arguments, which is the
            // VM's own limit for those formats; such a header can only come from a damaged
            // or hand-patched chunk.
            if tree.original.format != 0x8002 && tree.original.args > 4 {
                tree.omitted = Some(format!(
                    "BHAV format {:#06x} with {} arguments has no v2 spelling",
                    tree.original.format, tree.original.args
                ));
                trees.push(tree);
                continue;
            }
            let opts = LiftOptions {
                strict_padding: true,
                annotate_comments: false,
            };
            let labeled =
                format_bhav_labeled_air_with_arities(&tree.original, opts, Some(&lift_arities));
            tree.labeled = tree_body(&labeled);
            let text = match lift_bhav_structured_with_arities(
                &tree.original,
                opts,
                Some(&lift_arities),
            ) {
                StructuredLiftOutcome::Lifted(text) => {
                    tree.structured = true;
                    text
                }
                _ => labeled,
            };
            tree.lifted = text.lines().map(str::to_owned).collect();
            let Some(head) = tree
                .lifted
                .iter()
                .position(|l| l.starts_with("tree ") && l.trim_end().ends_with('{'))
            else {
                tree.omitted = Some("the lifter produced no tree".to_owned());
                trees.push(tree);
                continue;
            };
            let close = tree
                .lifted
                .iter()
                .rposition(|l| l.trim_end() == "}")
                .filter(|&i| i > head)
                .unwrap_or(tree.lifted.len());
            tree.body_lines = (head, close);
            tree.body = crate::parser::parse_module(&text).ok().and_then(|m| {
                m.items.into_iter().find_map(|item| match item {
                    crate::ast::Item::Tree(t) => Some(t.body),
                    _ => None,
                })
            });
            // Labeled-CFG bodies (`@n0:` labels) are only ever written raw.
            if tree
                .body
                .as_ref()
                .is_some_and(|b| b.iter().any(|s| matches!(s, Stmt::Label { .. })))
            {
                tree.body = None;
            }
            if let Some(body) = &tree.body {
                tree.sim_ctx = tree.lifecycle.is_none() && prefers_sim_context(body);
            }
            trees.push(tree);
        }

        // Names the bodies use.
        let mut states = BTreeSet::new();
        let mut guid_uses: BTreeMap<u32, usize> = BTreeMap::new();
        let mut extern_calls = BTreeSet::new();
        let mut extern_bcons = BTreeMap::new();
        for t in &trees {
            let Some(body) = &t.body else { continue };
            walk_stmts(body, &mut |s| {
                for op in stmt_operands(s) {
                    if let OperandRef::Direct(o) = op {
                        if matches!(o.scope, 0 | 1) && (0..=MAX_STATE_SLOT).contains(&o.index) {
                            states.insert(o.index);
                        }
                        if o.scope == 26 {
                            let addr = o.index as u16;
                            if let Some(name) = library_bcon_name(addr) {
                                extern_bcons.insert(addr, name);
                            }
                        }
                    }
                }
                for g in stmt_guids(s) {
                    *guid_uses.entry(g).or_default() += 1;
                }
                if let Some(id) = stmt_temps_call(s)
                    && !PRIVATE_TREES.contains(&id)
                {
                    extern_calls.insert(id);
                }
            });
        }
        let guid_consts = guid_uses
            .into_iter()
            .filter(|&(g, n)| n > 1 && g > 0x7FFF)
            .map(|(g, _)| (g, format!("GUID_{g:08X}")))
            .collect();

        // Interactions, as comments.
        let mut interactions = Vec::new();
        let tree_ident = |id: u16| -> String {
            trees
                .iter()
                .find(|t| t.id == id)
                .map_or_else(|| format!("tree {id}"), |t| t.ident.clone())
        };
        for c in iff.chunks.iter().filter(|c| &c.tag == b"TTAB") {
            let Ok(table) = InteractionTable::parse(iff.data(c)) else {
                continue;
            };
            let names = iff
                .chunks
                .iter()
                .find(|s| &s.tag == b"TTAs" && s.id == c.id)
                .and_then(|s| StringTable::parse(iff.data(s)).ok());
            for (i, it) in table.interactions.iter().enumerate() {
                let name = names
                    .as_ref()
                    .and_then(|n| n.entries.get(it.string_index as usize))
                    .map_or_else(
                        || format!("#{i}"),
                        |e| format!("{:?}", clean_label(&e.text)),
                    );
                let guard = if it.test == 0 {
                    String::new()
                } else {
                    format!(", guard {}", tree_ident(it.test))
                };
                interactions.push(format!(
                    "{name}: action {}{guard}, autonomy {}",
                    tree_ident(it.action),
                    it.autonomy
                ));
            }
        }

        // Sounds: v2 can pin an FWAV's ID and event name when the name is a plain file
        // name (`sound s = preset(.., wav: "name", id: N)`).
        let original_sounds = fwav_sounds(iff);
        let mut sounds = BTreeMap::new();
        let mut sounds_skipped = BTreeMap::new();
        let mut sound_names = BTreeSet::new();
        for (&id, name) in &original_sounds {
            let why = if id == 0 {
                "v2 cannot declare it (FWAV IDs start at 1)"
            } else if crate::manifest::validate_relative_filename(name, "sound").is_err() {
                "v2 cannot spell its event name (a plain file name)"
            } else if !sound_names.insert(name.to_ascii_lowercase()) {
                "another FWAV names the same event, which v2 does not allow"
            } else {
                sounds.insert(id, name.clone());
                continue;
            };
            sounds_skipped.insert(id, why);
        }

        let ctx_original = live_context(iff, &options.library_arities);
        Ok(Self {
            name: objd_chunk.label.trim().to_owned(),
            guid: objd.guid,
            price: objd.price,
            extra_objds: objds.len() - 1,
            bcons,
            bcon_names,
            extern_bcons,
            states,
            guid_consts,
            extern_calls,
            trees,
            interactions,
            sounds,
            sounds_skipped,
            ctx_original,
            original_sounds,
            original_resources: resource_chunks(iff),
            library_arities: options.library_arities.clone(),
        })
    }

    fn statement_count(&self) -> usize {
        self.trees
            .iter()
            .filter_map(|t| t.body.as_ref())
            .map(|b| count_stmts(b))
            .sum()
    }

    fn tree(&self, id: u16) -> Option<&TreeModel> {
        self.trees.iter().find(|t| t.id == id)
    }

    fn compile(&self, text: &str) -> std::result::Result<RecompiledFile, Failure> {
        let mut expanded =
            expand_v2_source_to_objects("decompiled.antics", text, &PackResolverConfig::default())
                .map_err(|e| Failure::V2 {
                    line: e.span.map(|s| s.line),
                    message: e.message.clone(),
                })?;
        if expanded.len() != 1 {
            return Err(Failure::V2 {
                line: None,
                message: format!("the source declares {} objects", expanded.len()),
            });
        }
        let expanded = expanded.remove(0);
        let air = expanded.air_source.clone();
        let compiled = compile_expanded_v2_object(expanded).map_err(|e| Failure::Air {
            tree: e.span.and_then(|s| air_tree_at_line(&air, s.line)),
            message: e.message.clone(),
        })?;
        let iff = Iff::parse(compiled.iff_bytes).map_err(|e| Failure::Air {
            tree: None,
            message: format!("the compiled IFF does not parse: {e}"),
        })?;
        let ctx = live_context(&iff, &self.library_arities);
        Ok(RecompiledFile {
            trees: compiled.behaviors,
            ctx,
            sounds: fwav_sounds(&iff),
            resources: resource_chunks(&iff),
        })
    }

    fn check_tree(
        &self,
        t: &TreeModel,
        recompiled: &RecompiledFile,
        cache: &mut BTreeMap<(u16, String, String), TreeCheck>,
    ) -> TreeCheck {
        let Some(b) = recompiled.trees.get(&t.id) else {
            return TreeCheck::Inconclusive(
                "the recompiled object has no tree with this ID".into(),
            );
        };
        // The checker does not model sounds: a tree that plays one whose event the
        // decompiled object names differently cannot be proven.
        if let Some(gap) = self.sound_gap(&t.original, recompiled) {
            return TreeCheck::Inconclusive(gap);
        }
        // Byte comparisons prove nothing when the tree runs against different constants
        // or callees; the checker takes both contexts into account.
        if self.same_context_for(&t.original, recompiled) {
            if same_tree_bytes(&t.original, b) {
                return TreeCheck::Identical;
            }
            if same_but_unreachable_tail(&t.original, b, &self.ctx_original) {
                return TreeCheck::Equivalent;
            }
        }
        let key = (t.id, format!("{b:?}"), format!("{:?}", recompiled.ctx));
        if let Some(hit) = cache.get(&key) {
            return hit.clone();
        }
        let (_, result) = check_bhav_equivalent_combined(
            &t.original,
            &self.ctx_original,
            b,
            &recompiled.ctx,
            &EquivOptions::default(),
            &LiveOptions::default(),
        );
        let check = match result {
            EquivResult::Equivalent { .. } => TreeCheck::Equivalent,
            EquivResult::Counterexample { divergence, .. } => TreeCheck::Different(divergence),
            EquivResult::Inconclusive { reason } => TreeCheck::Inconclusive(reason),
        };
        cache.insert(key, check.clone());
        check
    }

    /// Whether `tree` sees the same constants, attribute count and callee shapes in the
    /// recompiled file as in the original (constants as [`live_context`] normalizes
    /// them).
    fn same_context_for(&self, tree: &BhavChunk, recompiled: &RecompiledFile) -> bool {
        let (a, b) = (&self.ctx_original, &recompiled.ctx);
        let callee = |c: &LiveContext, id: u16| {
            (
                c.callee_arities.as_ref().and_then(|m| m.get(&id).copied()),
                c.argless_callees.as_ref().map(|s| s.contains(&id)),
            )
        };
        a.my_attribute_count == b.my_attribute_count
            && a.local_bcons == b.local_bcons
            && tree
                .instructions
                .iter()
                .filter(|i| i.opcode >= 0x100)
                .all(|i| callee(a, i.opcode) == callee(b, i.opcode))
    }

    /// Why `tree`'s sounds may differ: a Play Sound Event (primitive 23) naming an FWAV
    /// whose event the recompiled object does not reproduce.
    fn sound_gap(&self, tree: &BhavChunk, recompiled: &RecompiledFile) -> Option<String> {
        tree.instructions
            .iter()
            .filter(|i| i.opcode == PLAY_SOUND)
            .map(|i| u16::from_le_bytes([i.operands[0], i.operands[1]]))
            .find(|id| self.original_sounds.get(id) != recompiled.sounds.get(id))
            .map(|id| match self.sounds_skipped.get(&id) {
                Some(why) => format!("plays sound {id}: {why}"),
                None => format!("plays sound {id}, which the decompiled object does not reproduce"),
            })
    }

    fn resources_not_reproduced(&self, recompiled: &RecompiledFile) -> Vec<String> {
        self.original_resources
            .iter()
            .filter(|(key, data)| recompiled.resources.get(key) != Some(*data))
            .map(|((tag, id), _)| format!("{} {id}", String::from_utf8_lossy(tag)))
            .collect()
    }

    /// Marks tree `id` as not writable at any level. Returns `false` if it already was.
    fn omit(&mut self, id: u16, message: &str) -> bool {
        match self.trees.iter_mut().find(|t| t.id == id) {
            Some(t) if t.omitted.is_none() => {
                t.omitted = Some(format!(
                    "it does not compile at any level: {}",
                    message.lines().next().unwrap_or_default()
                ));
                true
            }
            _ => false,
        }
    }
}

/// Entries an operand can address in one BCON table (7 bits of entry index).
const BCON_ADDRESSABLE: usize = 128;

/// Play Sound Event.
const PLAY_SOUND: u16 = 23;

/// The sound event each FWAV chunk names, as the VM reads it: the bytes up to the first
/// zero as Latin-1, the last chunk of an ID winning (`sims-vm/src/program.rs`).
fn fwav_sounds(iff: &Iff) -> BTreeMap<u16, String> {
    let mut out = BTreeMap::new();
    for c in iff.chunks.iter().filter(|c| &c.tag == b"FWAV") {
        let data = iff.data(c);
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        out.insert(c.id, data[..end].iter().map(|&b| char::from(b)).collect());
    }
    out
}

/// Non-code chunks a tree can read through a primitive, keyed by tag and ID (last
/// chunk of a key wins).
fn resource_chunks(iff: &Iff) -> BTreeMap<([u8; 4], u16), Vec<u8>> {
    const TAGS: [&[u8; 4]; 8] = [
        b"STR#", b"CTSS", b"TTAB", b"TTAs", b"SLOT", b"FCNS", b"GLOB", b"TPRP",
    ];
    let mut out = BTreeMap::new();
    for c in iff.chunks.iter().filter(|c| TAGS.contains(&&c.tag)) {
        out.insert((c.tag, c.id), iff.data(c).to_vec());
    }
    out
}

struct RecompiledFile {
    trees: BTreeMap<u16, BhavChunk>,
    ctx: LiveContext,
    sounds: BTreeMap<u16, String>,
    resources: BTreeMap<([u8; 4], u16), Vec<u8>>,
}

enum Failure {
    /// The v2 front end rejected the source (line in the generated text).
    V2 {
        line: Option<usize>,
        message: String,
    },
    /// The `.air` compiler rejected the lowered source (the tree it was in).
    Air { tree: Option<u16>, message: String },
}

/// Whether `b` is `a` plus one trailing node nothing can reach: the `return true;` v2
/// adds after a `raw` body, which it cannot see always returns. Only node 0 is an entry
/// unless the tree is indexed (kind 4), so a node no edge names never runs.
///
/// The node must also be unread and must not change a fault: every edge of `a` stays
/// in range (an out-of-range target faults with the tree's node count), and the last
/// node is not a call whose inline arguments could run on into the next node
/// (`sims-vm/src/interpreter.rs`, the argument copy past the operand bytes).
fn same_but_unreachable_tail(a: &BhavChunk, b: &BhavChunk, ctx: &LiveContext) -> bool {
    let n = a.instructions.len();
    let header = |x: &BhavChunk| (x.format, x.kind, x.args, x.locals);
    let Ok(tail) = u8::try_from(n) else {
        return false;
    };
    n > 0
        && a.kind != 4
        && header(a) == header(b)
        && b.instructions.len() == n + 1
        && a.instructions.iter().zip(&b.instructions).all(|(x, y)| {
            (x.opcode, x.on_true, x.on_false, x.operands)
                == (y.opcode, y.on_true, y.on_false, y.operands)
        })
        && a.instructions.iter().all(|i| {
            [i.on_true, i.on_false]
                .iter()
                .all(|&t| t < tail || t >= 0xfd)
        })
        && a.instructions.last().is_some_and(|last| {
            last.opcode < 0x100
                || last.operands == [0xff; 8]
                || ctx
                    .callee_arities
                    .as_ref()
                    .and_then(|m| m.get(&last.opcode))
                    .is_some_and(|&argc| argc <= 4)
        })
}

/// The checker context for a file: its private arities plus the library's, its private
/// constant tables, and its argument-free private trees.
fn live_context(iff: &Iff, library: &BTreeMap<u16, usize>) -> LiveContext {
    let private = bhav_arities_in_range(iff, PRIVATE_TREES);
    let mut arities = library.clone();
    arities.extend(private.iter().map(|(&k, &v)| (k, v)));
    let mut bcons = BTreeMap::new();
    for c in iff.chunks.iter().filter(|c| &c.tag == b"BCON") {
        if PRIVATE_BCONS.contains(&c.id)
            && let Ok(values) = sims_formats::execution::constants(iff.data(c))
        {
            bcons.insert(c.id, values);
        }
    }
    // What an operand can read: entries past 127 have no address, and an empty table
    // reads like a missing one (`sims-vm/src/program.rs`, `Program::constant`).
    let bcons: BTreeMap<u16, Vec<i16>> = bcons
        .into_iter()
        .filter_map(|(id, mut values)| {
            values.truncate(BCON_ADDRESSABLE);
            (!values.is_empty()).then_some((id, values))
        })
        .collect();
    let mut argless = BTreeSet::new();
    for c in vm_kept_bhavs(iff).chunks {
        if PRIVATE_TREES.contains(&c.id)
            && private.get(&c.id) == Some(&0)
            && let Ok(t) = lift_bhav(c.id, &c.label, iff.data(c))
            && t.kind != 4
        {
            argless.insert(c.id);
        }
    }
    LiveContext {
        callee_arities: Some(arities),
        local_bcons: Some(bcons),
        argless_callees: Some(argless),
        ..LiveContext::default()
    }
}

/// The dedented body lines of the one `tree ... { ... }` in lifted `.air` text.
fn tree_body(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(head) = lines
        .iter()
        .position(|l| l.starts_with("tree ") && l.trim_end().ends_with('{'))
    else {
        return Vec::new();
    };
    let close = lines
        .iter()
        .rposition(|l| l.trim_end() == "}")
        .filter(|&i| i > head)
        .unwrap_or(lines.len());
    dedent(&lines[head + 1..close])
}

/// The tree whose `tree N "..."` header most recently precedes 1-based `line` of `air`.
fn air_tree_at_line(air: &str, line: usize) -> Option<u16> {
    air.lines()
        .take(line)
        .filter_map(|l| l.strip_prefix("tree "))
        .filter_map(|rest| rest.split_whitespace().next()?.parse().ok())
        .last()
}

/// A tree identifier: the label in `snake_case` plus the ID, so it never collides with
/// a builtin, a helper, or another tree.
fn unique_ident(label: &str, id: u16, used: &mut BTreeSet<String>) -> String {
    let base = snake_case_ident(label);
    let base = if base.starts_with("tree_") || base.len() > 40 {
        "tree".to_owned()
    } else {
        base
    };
    let mut ident = format!("{base}_{id}");
    while !used.insert(ident.clone()) {
        ident.push('_');
    }
    ident
}

/// A chunk label as a v2 string body: printable ASCII only, quotes and backslashes dropped.
fn clean_label(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                ' '
            }
        })
        .filter(|&c| c != '"' && c != '\\')
        .collect();
    cleaned.trim().to_owned()
}

/// The `extern bcon library_consts` entry name for a non-private constant address.
fn library_bcon_name(addr: u16) -> Option<String> {
    let (table, entry) = (addr >> 7, addr & 0x7F);
    match table {
        0..=63 => None,
        64..=127 => Some(format!("s{}_{entry}", 8192 + (table - 64))),
        _ => Some(format!("g{}_{entry}", 256 + (table - 128))),
    }
}

/// Whether a body reads mostly the Sim (`my.motive`, `my.person`) and the object as the
/// stack object (`stack.attr`), as interaction trees do.
fn prefers_sim_context(body: &[Stmt]) -> bool {
    let (mut sim, mut obj) = (0usize, 0usize);
    walk_stmts(body, &mut |s| {
        for op in stmt_operands(s) {
            if let OperandRef::Direct(o) = op {
                match o.scope {
                    1 | 14 | 18 => sim += 1,
                    0 | 3 => obj += 1,
                    _ => {}
                }
            }
        }
    });
    sim > obj
}

// ---------------------------------------------------------------------------------------
// AST walking
// ---------------------------------------------------------------------------------------

/// The local `s` saves `stack_obj` into (`local[a] := stack_obj`).
fn self_save_local(s: &Stmt) -> Option<i16> {
    match s {
        Stmt::Assign {
            lhs: OperandRef::Direct(Operand { scope: 25, index }),
            op: AssignOp::Set,
            rhs:
                OperandRef::Direct(Operand {
                    scope: 10,
                    index: 0,
                }),
            ..
        } => Some(*index),
        _ => None,
    }
}

/// `local[v] := 0; while set_to_next(person|all, dest: local[v]) { .. } stack_obj :=
/// local[a];`, the loop v2 writes for `for x in lot.sims()` / `lot.objects()`.
struct ForEachParts<'a> {
    /// The local `stack_obj` is restored from.
    saved: i16,
    /// `set_to_next` mode: 1 = people, 0 = all objects.
    mode: u8,
    body: &'a [Stmt],
}

fn for_each_parts<'a>(init: &Stmt, w: &'a Stmt, restore: &Stmt) -> Option<ForEachParts<'a>> {
    let local = |r: &OperandRef| match r {
        OperandRef::Direct(Operand { scope: 25, index }) => Some(*index),
        _ => None,
    };
    let is = |r: &OperandRef, scope: u8, index: i16| matches!(r, OperandRef::Direct(o) if o.scope == scope && o.index == index);
    let Stmt::Assign {
        lhs: il,
        op: AssignOp::Set,
        rhs: ir,
        ..
    } = init
    else {
        return None;
    };
    let Stmt::While { cond, body, .. } = w else {
        return None;
    };
    let Stmt::Assign {
        lhs: rl,
        op: AssignOp::Set,
        rhs: rr,
        ..
    } = restore
    else {
        return None;
    };
    let (saved, var) = (local(rr)?, local(il)?);
    if !is(ir, 7, 0) || !is(rl, 10, 0) || saved == var {
        return None;
    }
    let Condition::Primitive {
        prim:
            PrimitiveCall::SetToNext {
                mode,
                guid: 0,
                dest: Some(dest),
                local: LocalRef::Index(0),
            },
        ..
    } = cond
    else {
        return None;
    };
    if local(dest)? != var || !matches!(mode, 0 | 1) {
        return None;
    }
    // The body must not name either local: v2 gives the loop its own slots.
    let mut names_local = false;
    walk_stmts(body, &mut |s| {
        names_local |= stmt_operands(s)
            .iter()
            .any(|r| local(r).is_some_and(|l| l == saved || l == var));
    });
    (!names_local).then_some(ForEachParts {
        saved,
        mode: *mode,
        body,
    })
}

/// The local a tree's first statement saves `stack_obj` into, when nothing but the
/// restores closing `for`-shaped loops reads it again. v2 hoists that save to the top of a
/// tree whose loops clobber `stack_obj`, so the decompiled tree leaves it to v2.
///
/// The `.air` text is searched for the local as well, so a use the statement walker does
/// not see (a primitive operand, a raw op) keeps the save.
fn hoisted_self_save(body: &[Stmt], lifted: &[String]) -> Option<i16> {
    let saved = self_save_local(body.first()?)?;
    fn count(block: &[Stmt], saved: i16, n: &mut usize) {
        for (i, s) in block.iter().enumerate() {
            if i >= 2
                && for_each_parts(&block[i - 2], &block[i - 1], s).is_some_and(|p| p.saved == saved)
            {
                *n += 1;
            }
            match s {
                Stmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    count(then_body, saved, n);
                    count(else_body, saved, n);
                }
                Stmt::While { body, .. } | Stmt::Loop { body, .. } => count(body, saved, n),
                _ => {}
            }
        }
    }
    let mut loops = 0;
    count(&body[1..], saved, &mut loops);
    let needle = format!("local[{saved}]");
    let mentions: usize = lifted.iter().map(|l| l.matches(&needle).count()).sum();
    (loops > 0 && mentions == 1 + loops).then_some(saved)
}

fn walk_stmts(body: &[Stmt], f: &mut impl FnMut(&Stmt)) {
    for s in body {
        f(s);
        match s {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                walk_stmts(then_body, f);
                walk_stmts(else_body, f);
            }
            Stmt::While { body, .. } | Stmt::Loop { body, .. } => walk_stmts(body, f),
            _ => {}
        }
    }
}

fn count_stmts(body: &[Stmt]) -> usize {
    let mut n = 0;
    walk_stmts(body, &mut |_| n += 1);
    n
}

fn stmt_span_line(s: &Stmt) -> usize {
    match s {
        Stmt::Label { span, .. }
        | Stmt::Let { span, .. }
        | Stmt::RawOp { span, .. }
        | Stmt::InlineRawOp { span, .. }
        | Stmt::Primitive { span, .. }
        | Stmt::Call { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::Idle { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::Loop { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Return { span, .. } => span.line,
    }
}

fn cond_operands<'a>(c: &'a Condition, out: &mut Vec<&'a OperandRef>) {
    match c {
        Condition::Compare { lhs, rhs, .. } => {
            out.push(lhs);
            out.push(rhs);
        }
        Condition::Primitive { prim, .. } => prim_operands(prim, out),
        Condition::Not(inner) => cond_operands(inner, out),
        Condition::Call { .. } | Condition::RawOp { .. } => {}
    }
}

fn prim_operands<'a>(p: &'a PrimitiveCall, out: &mut Vec<&'a OperandRef>) {
    match p {
        PrimitiveCall::ExprRaw { lhs, rhs, .. } => {
            out.push(lhs);
            out.push(rhs);
        }
        PrimitiveCall::Random { dst, range } => {
            out.push(dst);
            out.push(range);
        }
        PrimitiveCall::AlterBudget { amount, .. } => out.push(amount),
        PrimitiveCall::TestObjectType { object, .. } => out.push(object),
        PrimitiveCall::Breakpoint { cond } => out.push(cond),
        PrimitiveCall::Padded { call, .. } => prim_operands(call, out),
        _ => {}
    }
}

/// Operands a statement itself reads or writes (not its nested statements).
fn stmt_operands(s: &Stmt) -> Vec<&OperandRef> {
    let mut out = Vec::new();
    match s {
        Stmt::Assign { lhs, rhs, .. } => {
            out.push(lhs);
            out.push(rhs);
        }
        Stmt::Primitive { prim, .. } => prim_operands(prim, &mut out),
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => cond_operands(cond, &mut out),
        _ => {}
    }
    out
}

fn prim_guids(p: &PrimitiveCall, out: &mut Vec<u32>) {
    match p {
        PrimitiveCall::TestObjectType { guid, .. }
        | PrimitiveCall::ManageInventory { guid, .. } => out.push(*guid),
        PrimitiveCall::Padded { call, .. } => prim_guids(call, out),
        _ => {}
    }
}

fn stmt_guids(s: &Stmt) -> Vec<u32> {
    let mut out = Vec::new();
    let cond = |c: &Condition, out: &mut Vec<u32>| {
        let mut c = c;
        while let Condition::Not(inner) = c {
            c = inner;
        }
        if let Condition::Primitive { prim, .. } = c {
            prim_guids(prim, out);
        }
    };
    match s {
        Stmt::Primitive { prim, .. } => prim_guids(prim, &mut out),
        Stmt::If { cond: c, .. } | Stmt::While { cond: c, .. } => cond(c, &mut out),
        _ => {}
    }
    out
}

/// The callee of a `call N(temps)` statement or condition.
fn stmt_temps_call(s: &Stmt) -> Option<u16> {
    let temps_call = |target: &CallTarget, args: &[i16], pass_temps: bool| match target {
        CallTarget::Id(id) if pass_temps && args.is_empty() => Some(*id),
        _ => None,
    };
    match s {
        Stmt::Call {
            target,
            args,
            pass_temps,
            ..
        } => temps_call(target, args, *pass_temps),
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => {
            let mut c = cond;
            while let Condition::Not(inner) = c {
                c = inner;
            }
            match c {
                Condition::Call {
                    target,
                    args,
                    pass_temps,
                    ..
                } => temps_call(target, args, *pass_temps),
                _ => None,
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------
// Plan: how each tree is written this round
// ---------------------------------------------------------------------------------------

/// How a tree is written this round, most readable first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    /// Idioms, in the tree's preferred context.
    Idiomatic,
    /// Idioms in the object's own context (for a tree that prefers `(sim: Sim)`, whose
    /// lowering may add a `stack_obj` save).
    IdiomaticObject,
    Raw,
    RawLabeled,
}

impl Level {
    fn idiomatic(self) -> bool {
        matches!(self, Self::Idiomatic | Self::IdiomaticObject)
    }
}

struct Plan {
    /// Tree ID -> level.
    styles: BTreeMap<u16, Level>,
    /// Tree ID -> pre-order statement indices written `raw`.
    raw_stmts: BTreeMap<u16, BTreeSet<usize>>,
    /// `init`/`main` trees written as plain `tree`s.
    standalone: BTreeSet<u16>,
}

impl Plan {
    fn initial(model: &Model) -> Self {
        let styles = model
            .trees
            .iter()
            .map(|t| {
                let level = if t.body.is_some() {
                    Level::Idiomatic
                } else {
                    Level::RawLabeled
                };
                (t.id, level)
            })
            .collect();
        // A lifecycle block always gets the default header, so other formats start plain.
        let standalone = model
            .trees
            .iter()
            .filter(|t| t.lifecycle.is_some() && t.original.format != 0x8002)
            .map(|t| t.id)
            .collect();
        Self {
            styles,
            raw_stmts: BTreeMap::new(),
            standalone,
        }
    }

    fn level(&self, t: &TreeModel) -> Level {
        self.styles.get(&t.id).copied().unwrap_or(Level::Raw)
    }

    fn style(&self, t: &TreeModel) -> TreeStyle {
        match self.level(t) {
            Level::Idiomatic | Level::IdiomaticObject => TreeStyle::Idiomatic,
            Level::Raw => TreeStyle::Raw,
            Level::RawLabeled => TreeStyle::RawLabeled,
        }
    }

    /// Whether the tree is written with `(sim: Sim)` this round.
    fn sim(&self, t: &TreeModel) -> bool {
        t.sim_ctx && self.level(t) == Level::Idiomatic
    }

    /// Moves tree `id` one style down. `false` when it is already at its last style.
    fn lower_tree(&mut self, model: &Model, id: u16) -> bool {
        let Some(t) = model.tree(id) else {
            return false;
        };
        let next = match self.level(t) {
            Level::Idiomatic if t.sim_ctx => Level::IdiomaticObject,
            Level::Idiomatic | Level::IdiomaticObject if t.structured => Level::Raw,
            Level::Idiomatic | Level::IdiomaticObject | Level::Raw => Level::RawLabeled,
            Level::RawLabeled if t.lifecycle.is_some() && !self.standalone.contains(&id) => {
                // Start again as a plain tree, which keeps the header.
                self.standalone.insert(id);
                self.raw_stmts.remove(&id);
                if t.body.is_some() {
                    Level::Idiomatic
                } else {
                    Level::RawLabeled
                }
            }
            Level::RawLabeled => return false,
        };
        self.styles.insert(id, next);
        true
    }

    /// Moves every tree to its last style. `false` when nothing changed.
    fn lower_all(&mut self, model: &Model) -> bool {
        let mut changed = false;
        for t in &model.trees {
            while self.lower_tree(model, t.id) {
                changed = true;
            }
        }
        changed
    }

    /// Lowers whatever generated line `line` (1-based) belongs to: the innermost idiomatic
    /// statement there goes `raw`, otherwise the enclosing tree moves down a style, and
    /// with no line or no tree every tree does.
    fn lower_at_line(&mut self, model: &Model, generated: &Generated, line: Option<usize>) -> bool {
        let Some(line) = line else {
            return self.lower_all(model);
        };
        let line = line.saturating_sub(1);
        let innermost = generated
            .stmt_ranges
            .iter()
            .filter(|r| r.start <= line && line <= r.end)
            .min_by_key(|r| r.end - r.start);
        if let Some(r) = innermost {
            return self.raw_stmts.entry(r.tree).or_default().insert(r.stmt)
                || self.lower_tree(model, r.tree);
        }
        match generated.tree_at_line(line) {
            // The caller leaves out a tree that cannot go lower.
            Some(id) => self.lower_tree(model, id),
            None => self.lower_all(model),
        }
    }
}

fn no_compile(message: &str) -> Error {
    Error::new(format!(
        "decompiled source does not compile even with every tree in 'raw': {}",
        message.lines().next().unwrap_or_default()
    ))
}

// ---------------------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------------------

struct StmtRange {
    start: usize,
    end: usize,
    tree: u16,
    stmt: usize,
}

impl Generated {
    /// The tree whose lines include 0-based line `line`.
    fn tree_at_line(&self, line: usize) -> Option<u16> {
        self.tree_ranges
            .iter()
            .find(|(_, start, end)| *start <= line && line <= *end)
            .map(|&(id, _, _)| id)
    }
}

struct Generated {
    text: String,
    stmt_ranges: Vec<StmtRange>,
    /// `(tree id, first line, last line)`, 0-based.
    tree_ranges: Vec<(u16, usize, usize)>,
    raw_counts: BTreeMap<u16, usize>,
}

struct Out {
    lines: Vec<String>,
    indent: usize,
}

impl Out {
    fn line(&mut self, text: impl AsRef<str>) {
        let text = text.as_ref();
        if text.is_empty() {
            self.lines.push(String::new());
        } else {
            self.lines
                .push(format!("{}{text}", "    ".repeat(self.indent)));
        }
    }
}

impl Model {
    fn generate(&self, plan: &Plan) -> Generated {
        let mut out = Out {
            lines: Vec::new(),
            indent: 0,
        };
        let mut stmt_ranges = Vec::new();
        let mut tree_ranges = Vec::new();
        let mut raw_counts = BTreeMap::new();
        out.line("// Decompiled by `sims-antics lift --v2`.");
        if self.extra_objds > 0 {
            out.line(format!(
                "// The file has {} more OBJD chunks (a multi-tile object); only the first is written.",
                self.extra_objds
            ));
        }
        out.line("");

        let mut lib_globals: Vec<u16> = Vec::new();
        let mut lib_semis: Vec<u16> = Vec::new();
        for &id in &self.extern_calls {
            if id >= 8192 {
                lib_semis.push(id);
            } else if id >= 256 {
                lib_globals.push(id);
            }
        }
        for (module, ids, prefix) in [
            ("globals", &lib_globals, "global"),
            ("semi_globals", &lib_semis, "semi_global"),
        ] {
            if ids.is_empty() {
                continue;
            }
            out.line(format!("extern mod {module} {{"));
            out.indent += 1;
            for id in ids {
                let args = match self.library_arities.get(id) {
                    Some(n) => format!(" // {n} argument(s), passed in temps"),
                    None => String::new(),
                };
                out.line(format!("@id({id}) fn {prefix}_{id}() -> bool;{args}"));
            }
            out.indent -= 1;
            out.line("}");
            out.line("");
        }
        if !self.extern_bcons.is_empty() {
            out.line("extern bcon library_consts {");
            out.indent += 1;
            for (addr, name) in &self.extern_bcons {
                out.line(format!("{name} @ {addr};"));
            }
            out.indent -= 1;
            out.line("}");
            out.line("");
        }

        let name = if self.name.is_empty() {
            "Decompiled Object".to_owned()
        } else {
            clean_label(&self.name)
        };
        out.line(format!("object \"{name}\" {{"));
        out.indent += 1;
        out.line(format!("guid: {};", hex_u32(self.guid)));
        out.line(format!("price: {};", self.price));
        out.line("");
        if !self.interactions.is_empty() {
            out.line("// Interactions (TTAB), kept by ID:");
            for i in &self.interactions {
                out.line(format!("// - {i}"));
            }
            out.line("");
        }
        for (g, name) in &self.guid_consts {
            out.line(format!("const {name} = {};", hex_u32(*g)));
        }
        if !self.guid_consts.is_empty() {
            out.line("");
        }
        for b in &self.bcons {
            out.line(format!("bcon {} \"{}\" {{", b.id, b.label));
            out.indent += 1;
            for (i, v) in b.values.iter().enumerate() {
                out.line(format!("e{i} = {v},"));
            }
            out.indent -= 1;
            out.line("}");
            out.line("");
        }
        for s in &self.states {
            out.line(format!("state attr_{s} @ {s}: i16;"));
        }
        if !self.sounds.is_empty() {
            out.line("");
        }
        for (id, name) in &self.sounds {
            out.line(format!(
                "sound sound_{id} = preset(\"ui_click\", wav: \"{name}\", id: {id});"
            ));
        }
        if !self.states.is_empty() {
            out.line("");
        }

        for t in &self.trees {
            if let Some(why) = &t.omitted {
                out.line(format!(
                    "// tree {} \"{}\" is not written: {why}.",
                    t.id, t.label
                ));
                out.line("");
                continue;
            }
            let level = plan.level(t);
            let start = out.lines.len();
            let sim = plan.sim(t);
            let as_lifecycle = t.lifecycle.filter(|_| !plan.standalone.contains(&t.id));
            match as_lifecycle {
                Some(kind) => {
                    out.line(format!("@id({}) {} {{", t.id, kind.keyword()));
                }
                None => {
                    let b = &t.original;
                    let ctx = if sim { " (sim: Sim)" } else { "" };
                    out.line(format!(
                        "@id({}) @frame(args: {}, locals: {}, format: {:#06x}, revision: {}) tree {} \"{}\"{ctx} {{",
                        t.id, b.args, b.locals, b.format, b.revision, t.ident, t.label
                    ));
                }
            }
            out.indent += 1;
            let mut raw_count = 0;
            match &t.body {
                Some(body) if level.idiomatic() => {
                    let mut tr = Translator {
                        model: self,
                        plan,
                        tree: t,
                        sim,
                        next_id: 0,
                        raw: Vec::new(),
                        raw_count: 0,
                        ranges: &mut stmt_ranges,
                        loop_depth: 0,
                        in_handler: false,
                        hoisted_save: hoisted_self_save(body, &t.lifted),
                    };
                    let body = match tr.hoisted_save {
                        Some(_) => {
                            tr.next_id += 1;
                            &body[1..]
                        }
                        None => &body[..],
                    };
                    tr.block(body, &mut out);
                    tr.flush_raw(&mut out);
                    raw_count = tr.raw_count;
                }
                _ => {
                    if level == Level::Raw {
                        let (head, close) = t.body_lines;
                        let lines: Vec<&str> = t.lifted[head + 1..close.min(t.lifted.len())]
                            .iter()
                            .map(String::as_str)
                            .collect();
                        write_raw_block(&mut out, &dedent(&lines));
                    } else {
                        write_raw_block(&mut out, &t.labeled);
                    }
                }
            }
            out.indent -= 1;
            out.line("}");
            tree_ranges.push((t.id, start, out.lines.len() - 1));
            raw_counts.insert(t.id, raw_count);
            out.line("");
        }
        while out.lines.last().is_some_and(String::is_empty) {
            out.lines.pop();
        }
        out.indent = 0;
        out.line("}");
        let mut text = out.lines.join("\n");
        text.push('\n');
        Generated {
            text,
            stmt_ranges,
            tree_ranges,
            raw_counts,
        }
    }
}

fn hex_u32(v: u32) -> String {
    format!("0x{:04X}_{:04X}", v >> 16, v & 0xFFFF)
}

/// Removes the common leading indentation of `lines` (blank lines ignored).
fn dedent(lines: &[&str]) -> Vec<String> {
    let indent_of = |l: &str| l.chars().take_while(|c| c.is_whitespace()).count();
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| indent_of(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            l.chars()
                .skip(indent)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn write_raw_block(out: &mut Out, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    out.line("raw {");
    out.indent += 1;
    for l in lines {
        out.line(l);
    }
    out.indent -= 1;
    out.line("}");
}

struct Translator<'a> {
    model: &'a Model,
    plan: &'a Plan,
    tree: &'a TreeModel,
    sim: bool,
    /// Pre-order index of the next statement.
    next_id: usize,
    /// Pending raw lines, merged into one `raw { ... }` block.
    raw: Vec<String>,
    raw_count: usize,
    ranges: &'a mut Vec<StmtRange>,
    loop_depth: usize,
    /// Inside an `animate(...) |event| { ... }` handler, where `param[0]` is `event`.
    in_handler: bool,
    /// The local the tree's first statement saves `stack_obj` into, when that save exists
    /// only for `for` loops to restore from (v2 writes its own save for them).
    hoisted_save: Option<i16>,
}

impl Translator<'_> {
    fn flush_raw(&mut self, out: &mut Out) {
        let lines = std::mem::take(&mut self.raw);
        write_raw_block(out, &lines);
    }

    fn forced_raw(&self, id: usize) -> bool {
        self.plan
            .raw_stmts
            .get(&self.tree.id)
            .is_some_and(|s| s.contains(&id))
    }

    /// The lifted `.air` lines of statement `s` (a compound statement up to its `}`).
    fn lifted_lines(&self, s: &Stmt) -> Vec<String> {
        let lifted = &self.tree.lifted;
        let first = stmt_span_line(s).saturating_sub(1);
        let Some(head) = lifted.get(first) else {
            return Vec::new();
        };
        let indent = head.len() - head.trim_start().len();
        let mut end = first;
        if head.trim_end().ends_with('{') {
            for (i, l) in lifted.iter().enumerate().skip(first + 1) {
                let li = l.len() - l.trim_start().len();
                let t = l.trim();
                if li == indent && t.starts_with('}') && !t.ends_with('{') {
                    end = i;
                    break;
                }
            }
        }
        let lines: Vec<&str> = lifted[first..=end].iter().map(String::as_str).collect();
        dedent(&lines)
    }

    fn emit_raw(&mut self, s: &Stmt) {
        let lines = self.lifted_lines(s);
        self.raw.extend(lines);
        self.raw_count += 1;
    }

    fn block(&mut self, body: &[Stmt], out: &mut Out) {
        let mut i = 0;
        while i < body.len() {
            if let Some(used) = self.for_each_loop(&body[i..], out) {
                i += used;
                continue;
            }
            self.stmt(&body[i], out);
            i += 1;
        }
    }

    /// Records the output lines `start..` as idiomatic statement `id`.
    fn record(&mut self, id: usize, start: usize, out: &Out) {
        self.ranges.push(StmtRange {
            start,
            end: out.lines.len().saturating_sub(1),
            tree: self.tree.id,
            stmt: id,
        });
    }

    fn stmt(&mut self, s: &Stmt, out: &mut Out) {
        let id = self.next_id;
        self.next_id += 1;
        let skip_children = |me: &mut Self| me.next_id = id + count_stmts(std::slice::from_ref(s));
        if self.forced_raw(id) {
            self.emit_raw(s);
            skip_children(self);
            return;
        }
        match s {
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                let Some(c) = self.cond(cond) else {
                    self.emit_raw(s);
                    skip_children(self);
                    return;
                };
                self.flush_raw(out);
                let start = out.lines.len();
                out.line(format!("if {c} {{"));
                out.indent += 1;
                self.block(then_body, out);
                self.flush_raw(out);
                out.indent -= 1;
                if !else_body.is_empty() {
                    out.line("} else {");
                    out.indent += 1;
                    self.block(else_body, out);
                    self.flush_raw(out);
                    out.indent -= 1;
                }
                out.line("}");
                self.record(id, start, out);
            }
            Stmt::While { cond, body, .. } => {
                let handler = self.animate_loop(cond).is_some();
                let header = match self.animate_loop(cond) {
                    Some(prim) if body.is_empty() => {
                        self.flush_raw(out);
                        let start = out.lines.len();
                        out.line(format!("{prim};"));
                        self.record(id, start, out);
                        return;
                    }
                    Some(prim) => format!("{prim} |event| {{"),
                    None => match self.cond(cond) {
                        Some(c) => format!("while {c} {{"),
                        None => {
                            self.emit_raw(s);
                            skip_children(self);
                            return;
                        }
                    },
                };
                self.flush_raw(out);
                let start = out.lines.len();
                out.line(header);
                out.indent += 1;
                self.loop_depth += 1;
                let outer_handler = self.in_handler;
                self.in_handler |= handler;
                self.block(body, out);
                self.in_handler = outer_handler;
                self.loop_depth -= 1;
                self.flush_raw(out);
                out.indent -= 1;
                out.line("}");
                self.record(id, start, out);
            }
            Stmt::Loop { body, .. } => {
                self.flush_raw(out);
                let start = out.lines.len();
                out.line("loop {");
                out.indent += 1;
                self.loop_depth += 1;
                self.block(body, out);
                self.loop_depth -= 1;
                self.flush_raw(out);
                out.indent -= 1;
                out.line("}");
                self.record(id, start, out);
            }
            _ => match self.simple(s) {
                Some(text) => {
                    self.flush_raw(out);
                    let start = out.lines.len();
                    out.line(text);
                    self.record(id, start, out);
                }
                None => self.emit_raw(s),
            },
        }
    }

    /// `local[a] := stack_obj; local[v] := 0; while set_to_next(person|all, dest: local[v])
    /// { .. } stack_obj := local[a];` as `for p in lot.sims() { .. }` (or `lot.objects()`).
    /// Without its own save, the loop restores from the tree's hoisted `local[a]` save
    /// (see [`hoisted_self_save`]). Returns how many statements it consumed.
    fn for_each_loop(&mut self, rest: &[Stmt], out: &mut Out) -> Option<usize> {
        let (saved_here, parts) = match rest {
            [save, init, w, restore, ..] if self_save_local(save).is_some() => {
                let parts = for_each_parts(init, w, restore)?;
                (
                    true,
                    (self_save_local(save) == Some(parts.saved)).then_some(parts)?,
                )
            }
            [init, w, restore, ..] => {
                let parts = for_each_parts(init, w, restore)?;
                (
                    false,
                    (self.hoisted_save == Some(parts.saved)).then_some(parts)?,
                )
            }
            _ => return None,
        };
        let ForEachParts { mode, body, .. } = parts;
        let used = if saved_here { 4 } else { 3 };
        let id = self.next_id;
        // The save (if any), the init and the `while` come first in pre-order; the restore
        // follows the loop body.
        let restore = id + used - 1 + count_stmts(body);
        if (id..id + used - 1)
            .chain([restore])
            .any(|i| self.forced_raw(i))
        {
            return None;
        }
        self.next_id += used - 1;
        self.flush_raw(out);
        let start = out.lines.len();
        let (var_name, source) = if mode == 1 {
            ("p", "lot.sims()")
        } else {
            ("o", "lot.objects()")
        };
        let suffix = if self.loop_depth > 0 {
            self.loop_depth.to_string()
        } else {
            String::new()
        };
        out.line(format!("for {var_name}{suffix} in {source} {{"));
        out.indent += 1;
        self.loop_depth += 1;
        self.block(body, out);
        self.loop_depth -= 1;
        self.flush_raw(out);
        out.indent -= 1;
        out.line("}");
        self.next_id += 1; // the restore
        self.record(id, start, out);
        Some(used)
    }

    /// `while !animate(...)` with the events left in `param[0]`.
    fn animate_loop(&self, cond: &Condition) -> Option<String> {
        let Condition::Not(inner) = cond else {
            return None;
        };
        let Condition::Primitive { prim, .. } = inner.as_ref() else {
            return None;
        };
        self.animate(prim)
    }

    fn animate(&self, prim: &PrimitiveCall) -> Option<String> {
        let PrimitiveCall::Animate {
            clip_id,
            table,
            flags,
            events,
            local_index: LocalRef::Index(0),
        } = prim
        else {
            return None;
        };
        (flags & 0x20 == 0).then(|| {
            format!("animate(clip: {clip_id}, table: {table}, flags: {flags}, events: {events})")
        })
    }

    fn simple(&self, s: &Stmt) -> Option<String> {
        match s {
            Stmt::Assign { lhs, op, rhs, .. } => {
                let op = match op {
                    AssignOp::Set => "=",
                    AssignOp::Add => "+=",
                    AssignOp::Sub => "-=",
                    AssignOp::Mul => "*=",
                    AssignOp::Div => "/=",
                    AssignOp::Mod => "%=",
                    AssignOp::BitAnd => "&=",
                    AssignOp::BitOr => "|=",
                    AssignOp::BitXor => "^=",
                    AssignOp::FlagSet | AssignOp::FlagClear | AssignOp::Sqrt => return None,
                };
                Some(format!("{} {op} {};", self.lvalue(lhs)?, self.rvalue(rhs)?))
            }
            Stmt::Return {
                target: Target::ReturnTrue,
                ..
            } => Some("return true;".to_owned()),
            Stmt::Return {
                target: Target::ReturnFalse,
                ..
            } => Some("return false;".to_owned()),
            Stmt::Break { .. } if self.loop_depth > 0 => Some("break;".to_owned()),
            Stmt::Continue { .. } if self.loop_depth > 0 => Some("continue;".to_owned()),
            Stmt::Call {
                target: CallTarget::Id(id),
                args,
                pass_temps: true,
                branches: None,
                ..
            } if args.is_empty() => Some(format!("{};", self.call(*id)?)),
            Stmt::Primitive {
                prim,
                branches: None,
                ..
            } => self.prim_stmt(prim).map(|p| format!("{p};")),
            _ => None,
        }
    }

    fn call(&self, id: u16) -> Option<String> {
        if PRIVATE_TREES.contains(&id) {
            let callee = self.model.tree(id)?;
            let standalone = callee.lifecycle.is_none() || self.plan.standalone.contains(&id);
            let callee_sim = self.plan.sim(callee);
            if callee.omitted.is_some() || !standalone || callee_sim != self.sim {
                return None;
            }
            Some(format!("{}()", callee.ident))
        } else if id >= 8192 {
            Some(format!("semi_globals::semi_global_{id}()"))
        } else if id >= 256 {
            Some(format!("globals::global_{id}()"))
        } else {
            None
        }
    }

    fn cond(&self, c: &Condition) -> Option<String> {
        match c {
            Condition::Compare { lhs, op, rhs, .. } => {
                let op = match op {
                    CompareOp::Gt => ">",
                    CompareOp::Lt => "<",
                    CompareOp::Eq => "==",
                    CompareOp::Ge => ">=",
                    CompareOp::Le => "<=",
                    CompareOp::Ne => "!=",
                    CompareOp::FlagTest | CompareOp::IncLt | CompareOp::DecGt => return None,
                };
                Some(format!("{} {op} {}", self.rvalue(lhs)?, self.rvalue(rhs)?))
            }
            Condition::Call {
                target: CallTarget::Id(id),
                args,
                pass_temps: true,
                ..
            } if args.is_empty() => self.call(*id),
            Condition::Primitive { prim, .. } => self.prim_cond(prim),
            Condition::Not(inner) => {
                let text = self.cond(inner)?;
                if matches!(inner.as_ref(), Condition::Compare { .. }) {
                    Some(format!("!({text})"))
                } else {
                    Some(format!("!{text}"))
                }
            }
            Condition::Call { .. } | Condition::RawOp { .. } => None,
        }
    }

    fn guid(&self, g: u32) -> String {
        self.model
            .guid_consts
            .get(&g)
            .cloned()
            .unwrap_or_else(|| hex_u32(g))
    }

    /// A primitive usable as a condition.
    fn prim_cond(&self, p: &PrimitiveCall) -> Option<String> {
        match p {
            PrimitiveCall::TestObjectType { guid, object } => {
                let obj = self.rvalue(object)?;
                let receiver = if self.sim && obj == "stack_obj" {
                    "self".to_owned()
                } else {
                    obj
                };
                Some(format!("{receiver}.is_type({})", self.guid(*guid)))
            }
            PrimitiveCall::AlterBudget {
                amount,
                category,
                flags,
            } => {
                let name = match flags {
                    0 => "debit_funds",
                    1 => "can_afford",
                    2 => "credit_funds",
                    _ => return None,
                };
                Some(format!(
                    "{name}({}, category: {category})",
                    self.rvalue(amount)?
                ))
            }
            PrimitiveCall::Animate { .. } => self.animate(p),
            other => self.builtin(other),
        }
    }

    /// A primitive as a statement.
    fn prim_stmt(&self, p: &PrimitiveCall) -> Option<String> {
        match p {
            PrimitiveCall::AlterBudget {
                amount,
                category,
                flags: flags @ (0 | 2),
            } => {
                let name = if *flags == 0 {
                    "debit_funds"
                } else {
                    "credit_funds"
                };
                Some(format!(
                    "{name}({}, category: {category})",
                    self.rvalue(amount)?
                ))
            }
            PrimitiveCall::StopAllSounds { use_stack_object } => {
                match (self.sim, use_stack_object) {
                    (false, false) | (true, true) => Some("stop_all_sounds()".to_owned()),
                    (true, false) => Some("stop_all_sounds(sim)".to_owned()),
                    (false, true) => None,
                }
            }
            PrimitiveCall::RemoveObject { target, flags } => {
                let mine = if self.sim { 1 } else { 0 };
                if *target != mine {
                    return None;
                }
                if *flags == 0 {
                    Some("destroy_self()".to_owned())
                } else {
                    Some(format!("destroy_self(flags: {flags})"))
                }
            }
            other => self.builtin(other),
        }
    }

    /// Builtins that are the same primitive as a statement or a condition.
    fn builtin(&self, p: &PrimitiveCall) -> Option<String> {
        Some(match p {
            PrimitiveCall::FindBestInteraction => "find_best_interaction()".to_owned(),
            PrimitiveCall::Grab => "grab()".to_owned(),
            PrimitiveCall::Drop => "drop()".to_owned(),
            PrimitiveCall::GosubFoundAction => "gosub_found_action()".to_owned(),
            PrimitiveCall::NotifyIdle => "notify_out_of_idle()".to_owned(),
            PrimitiveCall::Breakpoint { cond } => format!("breakpoint({})", self.rvalue(cond)?),
            PrimitiveCall::Random { dst, range } => {
                let range = self.rvalue(range)?;
                if range.starts_with("global[") {
                    return None;
                }
                format!("random_into({}, {range})", self.lvalue(dst)?)
            }
            PrimitiveCall::ActionString { table, index } => {
                format!("action_label(table: {table}, index: {index})")
            }
            PrimitiveCall::ShowString { table, index } => {
                format!("show_string(table: {table}, index: {index})")
            }
            PrimitiveCall::PushInteraction {
                menu,
                object_slot,
                priority,
                flags,
                icon_local: LocalRef::Index(0),
            } if flags & 0x03 == 0 => format!(
                "push_interaction(menu: {menu}, object: {object_slot}, priority: {priority}, flags: {flags})"
            ),
            PrimitiveCall::RunTreeByName {
                table,
                index,
                mode,
                global,
            } => format!(
                "run_tree_by_name(table: {table}, index: {index}, mode: {mode}, global: {global})"
            ),
            PrimitiveCall::GoToRelative {
                location,
                facing,
                options,
            } => format!(
                "go_to_relative(location: {location}, facing: {facing}, options: {options})"
            ),
            PrimitiveCall::ManageInventory {
                operation,
                kind,
                flags,
                guid,
            } => format!(
                "manage_inventory(op: {operation}, kind: {kind}, flags: {flags}, guid: {})",
                self.guid(*guid)
            ),
            PrimitiveCall::Refresh { target, mode } if *target <= 1 => {
                format!("refresh(target: {target}, mode: {mode})")
            }
            PrimitiveCall::SetHeadline {
                use_stack_object,
                local: LocalRef::Index(0),
                index,
                kind,
                duration,
                options,
            } => format!(
                "set_headline(kind: {kind}, index: {index}, duration: {duration}, options: {options}, stack_obj: {use_stack_object})"
            ),
            PrimitiveCall::SpecialEffect {
                duration,
                kind,
                size,
                flags,
                str_index,
            } if flags & 0x20 == 0 => format!(
                "special_effect(duration: {duration}, kind: {kind}, size: {size}, flags: {flags}, str_index: {str_index})"
            ),
            _ => return None,
        })
    }

    fn rvalue(&self, r: &OperandRef) -> Option<String> {
        let OperandRef::Direct(o) = r else {
            return None;
        };
        match o.scope {
            7 => Some(o.index.to_string()),
            26 => {
                let addr = o.index as u16;
                let (table, entry) = (addr >> 7, addr & 0x7F);
                if table < 64 {
                    let name = self.model.bcon_names.get(&(4096 + table))?;
                    let len = self
                        .model
                        .bcons
                        .iter()
                        .find(|b| b.id == 4096 + table)?
                        .values
                        .len();
                    (usize::from(entry) < len).then(|| format!("{name}.e{entry}"))
                } else {
                    let name = self.model.extern_bcons.get(&addr)?;
                    Some(format!("library_consts.{name}"))
                }
            }
            9 if o.index == 0 && self.in_handler => Some("event".to_owned()),
            _ => self.lvalue(r),
        }
    }

    fn lvalue(&self, r: &OperandRef) -> Option<String> {
        let OperandRef::Direct(o) = r else {
            return None;
        };
        let i = o.index;
        if i < 0 {
            return None;
        }
        Some(match (o.scope, self.sim) {
            (8, _) => format!("temp[{i}]"),
            (6, _) => format!("global[{i}]"),
            (10, _) if i == 0 => "stack_obj".to_owned(),
            (0, false) | (1, true) if i <= MAX_STATE_SLOT => format!("self.attr_{i}"),
            (3, false) => format!("self.data[{i}]"),
            (14, true) => format!("sim.motive[{i}]"),
            (18, true) => format!("sim.person[{i}]"),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::behavior::Instruction;

    #[test]
    fn library_constant_names() {
        assert_eq!(library_bcon_name((129 << 7) | 3).as_deref(), Some("g257_3"));
        assert_eq!(library_bcon_name((64 << 7) | 1).as_deref(), Some("s8192_1"));
        assert_eq!(library_bcon_name(5), None);
    }

    #[test]
    fn tree_idents_are_unique_and_valid() {
        let mut used = BTreeSet::new();
        assert_eq!(
            unique_ident("Helper Tree", 4200, &mut used),
            "helper_tree_4200"
        );
        assert_eq!(unique_ident("", 4201, &mut used), "tree_4201");
        assert_eq!(unique_ident("42 things", 4202, &mut used), "tree_4202");
    }

    fn tree(instructions: &[(u16, u8, u8, [u8; 8])]) -> BhavChunk {
        let mut t = BhavChunk::new(
            4200,
            "T",
            instructions
                .iter()
                .map(|&(opcode, on_true, on_false, operands)| Instruction {
                    opcode,
                    on_true,
                    on_false,
                    operands,
                })
                .collect(),
        );
        t.format = 0x8002;
        t
    }

    fn with_tail(a: &BhavChunk) -> BhavChunk {
        let mut b = a.clone();
        b.instructions.push(Instruction {
            opcode: 2,
            on_true: 0xfe,
            on_false: 0xfe,
            operands: [0; 8],
        });
        b
    }

    fn arities(pairs: &[(u16, usize)]) -> LiveContext {
        LiveContext {
            callee_arities: Some(pairs.iter().copied().collect()),
            ..LiveContext::default()
        }
    }

    #[test]
    fn unreachable_tail_needs_an_unread_node() {
        let ctx = arities(&[(4201, 4), (4202, 5)]);
        let plain = tree(&[(2, 0xfe, 0xfd, [0; 8])]);
        assert!(same_but_unreachable_tail(&plain, &with_tail(&plain), &ctx));
        // An edge to the appended node, or past the original's end, rules it out.
        let to_tail = tree(&[(2, 1, 0xfd, [0; 8])]);
        assert!(!same_but_unreachable_tail(
            &to_tail,
            &with_tail(&to_tail),
            &ctx
        ));
        let out_of_range = tree(&[(2, 0xfe, 9, [0; 8])]);
        assert!(!same_but_unreachable_tail(
            &out_of_range,
            &with_tail(&out_of_range),
            &ctx
        ));
        // A last-node call whose inline arguments run past its operands reads the tail.
        let spill = tree(&[(4202, 0xfe, 0xfd, [0; 8])]);
        assert!(!same_but_unreachable_tail(&spill, &with_tail(&spill), &ctx));
        let unknown = tree(&[(4203, 0xfe, 0xfd, [0; 8])]);
        assert!(!same_but_unreachable_tail(
            &unknown,
            &with_tail(&unknown),
            &ctx
        ));
        let fits = tree(&[(4201, 0xfe, 0xfd, [0; 8])]);
        assert!(same_but_unreachable_tail(&fits, &with_tail(&fits), &ctx));
        let temps = tree(&[(4202, 0xfe, 0xfd, [0xff; 8])]);
        assert!(same_but_unreachable_tail(&temps, &with_tail(&temps), &ctx));
    }

    #[test]
    fn dedent_counts_characters() {
        assert_eq!(
            dedent(&["  a", "\u{a0}  b"]),
            vec!["a".to_owned(), " b".to_owned()]
        );
    }

    #[test]
    fn air_lines_map_to_trees() {
        let air = "bcon 4097 \"T\" = [1];\ntree 4096 \"A\" {\n    x;\n}\ntree 4200 \"B\" (args: 1) {\n    y;\n}\n";
        assert_eq!(air_tree_at_line(air, 1), None);
        assert_eq!(air_tree_at_line(air, 3), Some(4096));
        assert_eq!(air_tree_at_line(air, 6), Some(4200));
    }
}
