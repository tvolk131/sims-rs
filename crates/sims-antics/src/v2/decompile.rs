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
    AssignOp, CallTarget, CompareOp, Condition, LocalRef, Operand, OperandRef, PrimitiveCall,
    SlotSpec, Stmt, StringRef, Target,
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
    EquivOptions, EquivResult, LiveContext, LiveOptions, apply_operand_pad,
    canonicalize_primitive_operands, check_bhav_equivalent_combined, same_tree_bytes,
};
use sims_formats::behavior::{InteractionTable, ObjectDefinition};
use sims_formats::iff::{Chunk, Iff};
use sims_formats::strings::StringTable;
use std::borrow::Cow;
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
/// OBJD word (after the version) holding how many attributes an instance gets.
const OBJD_ATTRIBUTES_WORD: usize = 56;

/// Options for [`decompile_iff_to_v2`].
#[derive(Debug, Clone)]
pub struct DecompileV2Options {
    /// Argument counts of library trees the object calls (global `256..=4095`,
    /// semi-global `8192+`), e.g. from an install's arity catalog. Private trees' counts
    /// always come from the file itself.
    pub library_arities: BTreeMap<u16, usize>,
    /// Optional BHAV labels for the same global and semi-global libraries.
    pub library_names: BTreeMap<u16, String>,
    /// Compile the generated source and check every tree against the original, falling
    /// back to `raw` where needed (default `true`). Without it the output is the most
    /// readable translation, unchecked.
    pub validate: bool,
}

impl Default for DecompileV2Options {
    fn default() -> Self {
        Self {
            library_arities: BTreeMap::new(),
            library_names: BTreeMap::new(),
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
            Err(Failure::V2 { line, message })
                if plan.drop_strings(&model, &generated, line, &message)
                    || plan.drop_variants(&model, &generated, line, &message) => {}
            Err(Failure::Air {
                tree: None,
                message,
            }) if plan.drop_strings(&model, &generated, None, &message)
                || plan.drop_variants(&model, &generated, None, &message) => {}
            Err(Failure::V2 { line, message }) => {
                let tree = line.and_then(|l| generated.tree_at_line(l.saturating_sub(1)));
                // An extra idiom is turned off before anything is lowered: it may be what
                // clashes with the rest of the tree.
                let unblamed = tree.is_none() && plan.drop_all_extras(&generated);
                if !unblamed
                    && !tree.is_some_and(|id| plan.add_frame(&model, id))
                    && !tree.is_some_and(|id| plan.drop_extras(id, &generated))
                    && !plan.lower_at_line(&model, &generated, line)
                    // A tree that fails at its lowest level is left out.
                    && !tree.is_some_and(|id| model.omit(id, &message))
                {
                    return Err(no_compile(&message));
                }
            }
            Err(Failure::Air { tree, message }) => {
                let handled = match tree {
                    Some(id) => {
                        plan.add_frame(&model, id)
                            || plan.drop_extras(id, &generated)
                            || plan.lower_tree(&model, id)
                            || model.omit(id, &message)
                    }
                    None => plan.drop_all_extras(&generated) || plan.lower_all(&model),
                };
                if !handled {
                    return Err(no_compile(&message));
                }
            }
            Ok(recompiled) => {
                let mut checks = BTreeMap::new();
                let mut unproven = Vec::new();
                for t in model.trees.iter().filter(|t| t.omitted.is_none()) {
                    let check = model.check_tree(t, &recompiled, &mut cache);
                    if !matches!(check, TreeCheck::Identical | TreeCheck::Equivalent) {
                        unproven.push(t.id);
                    }
                    checks.insert(t.id, check);
                }
                // A block given its header may be all its callers were missing (a caller is
                // proven only with its callees), so such a round lowers nothing else.
                let mut changed = false;
                for &id in &unproven {
                    changed |= plan.add_frame(&model, id);
                }
                if !changed {
                    for &id in &unproven {
                        changed |= plan.drop_extras(id, &generated) || plan.lower_tree(&model, id);
                    }
                }
                if changed {
                    continue;
                }
                let source = format_v2_source(&generated.text)?;
                let mut trees: Vec<DecompiledTree> = model
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
                model.propagate_unproven_callees(&mut trees, &recompiled);
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

/// One catalog variant (`variant "Name" { ... }`).
struct VariantModel {
    objd_id: u16,
    name: String,
    guid: u32,
    price: u16,
    description: Option<String>,
    init: u16,
    main: u16,
}

struct BconModel {
    id: u16,
    label: String,
    entries: Vec<(String, i16)>,
}

/// A `STR#` table some tree shows text from, written as a `strings` member.
struct StringsModel {
    id: u16,
    label: String,
    /// Entry name -> text (the table's US English entries, in order).
    entries: Vec<(String, String)>,
}

struct SoundModel {
    ident: String,
    wav: String,
}

struct Model {
    name: String,
    guid: u32,
    price: u16,
    /// Further OBJDs that are neither catalog variants nor written (multi-tile parts).
    extra_objds: usize,
    /// Catalog variants: further OBJDs that differ from the first only in the words a v2
    /// `variant` sets.
    variants: Vec<VariantModel>,
    /// The first OBJD's init and main tree IDs.
    primary_init: u16,
    primary_main: u16,
    bcons: Vec<BconModel>,
    /// String tables the trees read, by ID.
    strings: Vec<StringsModel>,
    /// The tables trees show text from that are not written, and why.
    strings_skipped: Vec<(u16, String)>,
    /// Library constant operand address -> entry name in `extern bcon library_consts`.
    extern_bcons: BTreeMap<u16, String>,
    /// Attribute slots referenced as `my.attr` / `stack.attr`.
    states: BTreeSet<i16>,
    /// The OBJD's attribute count, pinned with `attributes:` so states never grow it.
    attributes: Option<u16>,
    /// Highest attribute slot spelled `self.attr_N` (others stay `raw`).
    state_limit: i16,
    /// GUIDs used more than once -> `const` name.
    guid_consts: BTreeMap<u32, String>,
    /// Library trees referenced by a structured call, by ID and source identifier.
    extern_calls: BTreeMap<u16, String>,
    /// Callable trees with a known arity and a structured inline call.
    literal_calls: BTreeMap<u16, usize>,
    trees: Vec<TreeModel>,
    interactions: Vec<String>,
    /// FWAV chunks written as `sound` declarations, by chunk ID.
    sounds: BTreeMap<u16, SoundModel>,
    /// FWAV chunks v2 cannot declare, and why.
    sounds_skipped: BTreeMap<u16, &'static str>,
    /// Original-side checker context.
    ctx_original: LiveContext,
    /// The sound event each original FWAV names, as the VM reads it (last chunk wins).
    original_sounds: BTreeMap<u16, String>,
    /// Behaviour-relevant non-code chunks of the original, as the VM reads them.
    original_resources: BTreeMap<([u8; 4], u16), Vec<u8>>,
    /// The original's string tables, as the VM reads their text.
    original_strings: BTreeMap<([u8; 4], u16), StringTexts>,
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
        let attributes = objd_word(OBJD_ATTRIBUTES_WORD);
        // Highest attribute a `state` may name without growing the object.
        let state_limit = attributes.map_or(MAX_STATE_SLOT, |a| {
            i16::try_from(a).map_or(MAX_STATE_SLOT, |a| (a - 1).min(MAX_STATE_SLOT))
        });
        let primary_name = primary_object_name(&objd_chunk.label);
        let mut variants: Vec<VariantModel> = Vec::new();
        for c in objds[1..].iter().filter(|_| objd.master_id == 0) {
            let data = iff.data(c);
            let name = clean_label(&c.label);
            let Some(v) = variant_words(objd_data, data) else {
                continue;
            };
            let guid = u32::from(v[1]) | (u32::from(v[2]) << 16);
            // What the v2 compiler requires of a variant: its own name and GUID, and a
            // name whose chunk labels (`{name} Functions`) fit.
            if name.is_empty()
                || name.len() > MAX_CHUNK_LABEL - " Functions".len()
                || name == primary_name
                || guid == objd.guid
                || variants.iter().any(|w| w.name == name || w.guid == guid)
            {
                continue;
            }
            let description = iff
                .get(b"CTSS", v[4])
                .and_then(|t| StringTable::parse_chunk(iff.data(t), t.flags).ok())
                .and_then(|t| t.entries.get(1).map(|e| clean_label(&e.text)));
            variants.push(VariantModel {
                objd_id: c.id,
                name,
                guid,
                price: v[3],
                description,
                init: v[5],
                main: v[0],
            });
        }
        let primary_init = objd_word(OBJD_INIT_WORD).unwrap_or(0);
        let primary_main = objd_word(OBJD_MAIN_WORD).unwrap_or(0);
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
        // `library_consts` is the extern table's name.
        let mut used_names = BTreeSet::from([name_key("library_consts")]);
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
            // Labels are identifiers (ASCII), and must fit a chunk label, leaving room for
            // a `{id}_{n}` suffix when one is needed.
            label.truncate(MAX_CHUNK_LABEL);
            let base: String = label.chars().take(MAX_CHUNK_LABEL - 12).collect();
            let mut n = 0;
            while !used_names.insert(name_key(&label)) {
                n += 1;
                label = if n == 1 {
                    format!("{base}{}", c.id)
                } else {
                    format!("{base}{}_{n}", c.id)
                };
            }
            bcons.push(BconModel {
                id: c.id,
                label,
                entries: bcon_entries(iff, c.id, values),
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
            // A variant runs its init/main tree as the object itself.
            let variant_tree = variants
                .iter()
                .any(|v| v.init == tree.id || v.main == tree.id);
            if let Some(body) = &tree.body {
                tree.sim_ctx =
                    tree.lifecycle.is_none() && !variant_tree && prefers_sim_context(body);
            }
            trees.push(tree);
        }

        // Names the bodies use.
        let mut states = BTreeSet::new();
        let mut guid_uses: BTreeMap<u32, usize> = BTreeMap::new();
        let mut extern_calls = BTreeSet::new();
        let mut literal_calls = BTreeMap::new();
        let mut extern_bcons = BTreeMap::new();
        for t in &trees {
            let Some(body) = &t.body else { continue };
            walk_stmts(body, &mut |s| {
                for op in stmt_operands(s) {
                    if let OperandRef::Direct(o) = op {
                        if matches!(o.scope, 0 | 1) && (0..=state_limit).contains(&o.index) {
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
                if let Some((id, pass_temps)) = stmt_call(s) {
                    if !pass_temps
                        && let Some(&arity @ 1..=4) = lift_arities.get(&id)
                        && (!PRIVATE_TREES.contains(&id)
                            || (!variants.iter().any(|v| v.init == id || v.main == id)
                                && trees.iter().any(|callee| {
                                    callee.id == id && usize::from(callee.original.args) >= arity
                                })))
                    {
                        literal_calls.insert(id, arity);
                    }
                    if id >= 256 && !PRIVATE_TREES.contains(&id) {
                        extern_calls.insert(id);
                    }
                }
            });
        }
        let guid_consts = guid_uses
            .into_iter()
            // `lint` asks for a name for any GUID-sized literal (over 0xFFFF).
            .filter(|&(g, n)| g > 0xFFFF || (n > 1 && g > 0x7FFF))
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
        for c in iff.all(b"TTAB") {
            let Ok(table) = InteractionTable::parse(iff.data(c)) else {
                continue;
            };
            let names = iff
                .get(b"TTAs", c.id)
                .and_then(|s| StringTable::parse_chunk(iff.data(s), s.flags).ok());
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
        // Several FWAVs may share one event name (v2 PR F), but only spelled the same way.
        let mut sound_names: BTreeMap<String, &str> = BTreeMap::new();
        let mut event_counts = BTreeMap::new();
        for name in original_sounds.values() {
            *event_counts.entry(name).or_insert(0) += 1;
        }
        let mut sound_idents: BTreeSet<String> = original_sounds
            .keys()
            .map(|id| format!("sound_{id}"))
            .collect();
        for (&id, name) in &original_sounds {
            let why = if crate::manifest::validate_relative_filename(name, "sound").is_err() {
                "v2 cannot spell its event name (a plain file name)"
            } else if name.len() > MAX_CHUNK_LABEL {
                "its event name is longer than a chunk label, which v2 uses it as"
            } else if *sound_names
                .entry(name.to_ascii_lowercase())
                .or_insert(name.as_str())
                != name.as_str()
            {
                "another FWAV names the same event with other letter case, which v2 does not allow"
            } else {
                let fallback = format!("sound_{id}");
                let label = name
                    .rsplit_once('.')
                    .filter(|(_, ext)| ext.eq_ignore_ascii_case("wav"))
                    .map_or(name.as_str(), |(stem, _)| stem);
                let ident = if event_counts[name] == 1 {
                    unique_metadata_ident(label, id, &mut sound_idents).unwrap_or(fallback)
                } else {
                    // An event shared by multiple chunks has no single source identifier.
                    fallback
                };
                sounds.insert(
                    id,
                    SoundModel {
                        ident,
                        wav: name.clone(),
                    },
                );
                continue;
            };
            sounds_skipped.insert(id, why);
        }

        let mut global_idents = BTreeSet::new();
        let mut semi_global_idents = BTreeSet::new();
        for &id in &extern_calls {
            if id >= 8192 {
                semi_global_idents.insert(format!("semi_global_{id}"));
            } else {
                global_idents.insert(format!("global_{id}"));
            }
        }
        let extern_calls = extern_calls
            .into_iter()
            .map(|id| {
                let (prefix, used) = if id >= 8192 {
                    ("semi_global", &mut semi_global_idents)
                } else {
                    ("global", &mut global_idents)
                };
                let fallback = format!("{prefix}_{id}");
                let ident = options
                    .library_names
                    .get(&id)
                    .and_then(|label| unique_metadata_ident(label, id, used))
                    .unwrap_or(fallback);
                (id, ident)
            })
            .collect();

        let ctx_original = live_context(iff, &options.library_arities);
        let strings = strings_models(iff, &trees);
        Ok(Self {
            name: objd_chunk.label.trim().to_owned(),
            guid: objd.guid,
            price: objd.price,
            extra_objds: objds.len() - 1 - variants.len(),
            variants,
            primary_init,
            primary_main,
            strings: strings.0,
            strings_skipped: strings.1,
            bcons,
            extern_bcons,
            states,
            attributes,
            state_limit,
            guid_consts,
            extern_calls,
            literal_calls,
            trees,
            interactions,
            sounds,
            sounds_skipped,
            ctx_original,
            original_sounds,
            original_resources: resource_chunks(iff),
            original_strings: string_texts(iff),
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
            strings: string_texts(&iff),
            attributes: objd_attributes(&iff),
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
        // Nor does it read string tables: a tree that shows one the decompiled object
        // does not reproduce byte for byte is not proven by its code alone.
        if let Some(gap) = self.string_gap(&t.original, recompiled) {
            return TreeCheck::Inconclusive(gap);
        }
        // Byte comparisons prove nothing when the tree runs against different constants
        // or callees; the checker takes both contexts into account.
        let same_context = self.same_context_for(&t.original, recompiled);
        if same_context && same_tree_bytes(&t.original, b) {
            return TreeCheck::Identical;
        }
        let original = without_unused_operands(&t.original, &self.ctx_original);
        let candidate = without_unused_operands(b, &recompiled.ctx);
        if same_context
            && (same_tree_bytes(&original, &candidate)
                || same_but_unreachable_tail(&original, &candidate, &self.ctx_original))
        {
            return TreeCheck::Equivalent;
        }
        let key = (t.id, format!("{b:?}"), format!("{:?}", recompiled.ctx));
        if let Some(hit) = cache.get(&key) {
            return hit.clone();
        }
        let (_, result) = check_bhav_equivalent_combined(
            &original,
            &self.ctx_original,
            &candidate,
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
            && self.attributes == recompiled.attributes
            && a.local_bcons == b.local_bcons
            && tree
                .instructions
                .iter()
                .filter(|i| i.opcode >= 0x100)
                .all(|i| callee(a, i.opcode) == callee(b, i.opcode))
    }

    /// Why `tree`'s strings may differ: a Show String (21) or Set Action String (50)
    /// naming a `STR#` table, or a private Dialog (36, `STR#` 301), whose table the
    /// recompiled object does not reproduce (its texts, per language, in order).
    fn string_gap(&self, tree: &BhavChunk, recompiled: &RecompiledFile) -> Option<String> {
        tree.instructions
            .iter()
            .filter_map(string_table_read)
            .find(|&id| {
                STRING_TAGS.iter().any(|tag| {
                    let key = (**tag, id);
                    self.original_strings.get(&key) != recompiled.strings.get(&key)
                })
            })
            .map(|id| format!("reads STR# {id}, which the decompiled object does not reproduce"))
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
            .map(|((tag, id), _)| {
                if tag == b"OBJD"
                    && let Some(v) = self.variants.iter().find(|v| v.objd_id == *id)
                {
                    return format!("OBJD {id} (variant \"{}\")", v.name);
                }
                format!("{} {id}", String::from_utf8_lossy(tag))
            })
            .collect()
    }

    /// The fields of a `variant` member, or why it cannot be written this round.
    fn variant_fields(
        &self,
        v: &VariantModel,
        plan: &Plan,
    ) -> std::result::Result<Vec<String>, String> {
        if let Some(why) = &plan.variants_off {
            return Err(why.clone());
        }
        let mut fields = vec![
            format!("guid: {}", hex_u32(v.guid)),
            format!("price: {}", v.price),
        ];
        if let Some(d) = &v.description {
            fields.push(format!("description: \"{d}\""));
        }
        for (role, id, primary) in [
            ("init", v.init, self.primary_init),
            ("main", v.main, self.primary_main),
        ] {
            let tree = self.tree(id).filter(|t| t.omitted.is_none());
            let block = tree.and_then(|t| t.lifecycle.filter(|_| !plan.standalone.contains(&id)));
            if id == primary && (tree.is_none() || block.is_some()) {
                // The variant inherits the object's own init/main.
                continue;
            }
            match (tree, block) {
                (Some(t), None) if t.original.args <= 4 => {
                    fields.push(format!("{role}: {}", t.ident));
                }
                (Some(t), None) => {
                    return Err(format!(
                        "its {role} tree {id} takes {} arguments",
                        t.original.args
                    ));
                }
                (Some(_), Some(kind)) => {
                    return Err(format!(
                        "its {role} tree {id} is written as the object's {} block",
                        kind.keyword()
                    ));
                }
                (None, _) => {
                    return Err(format!(
                        "its {role} tree {id} is not a written private tree"
                    ));
                }
            }
        }
        Ok(fields)
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

/// OBJD words (after the 32-bit version) a v2 `variant` sets: main, GUID low, GUID high,
/// price, catalog strings ID, init.
const VARIANT_WORDS: [usize; 6] = [3, 12, 13, 16, 39, OBJD_INIT_WORD];

/// The [`VARIANT_WORDS`] of `other` if it differs from `primary` in no other word.
fn variant_words(primary: &[u8], other: &[u8]) -> Option<[u16; 6]> {
    let words = |d: &[u8]| -> Vec<u16> {
        d.get(4..)
            .unwrap_or_default()
            .chunks(2)
            .map(|b| u16::from_le_bytes([b[0], b.get(1).copied().unwrap_or(0)]))
            .collect()
    };
    let (a, b) = (words(primary), words(other));
    let same_rest = primary.len() == other.len()
        && primary.get(..4) == other.get(..4)
        && a.len() > OBJD_INIT_WORD
        && a.iter()
            .zip(&b)
            .enumerate()
            .all(|(w, (x, y))| x == y || VARIANT_WORDS.contains(&w));
    same_rest.then(|| VARIANT_WORDS.map(|w| b[w]))
}

/// The `object "..."` name written for an OBJD label.
fn primary_object_name(label: &str) -> String {
    let name = clean_label(label);
    if name.is_empty() {
        "Decompiled Object".to_owned()
    } else {
        name
    }
}

/// The longest chunk label the IFF writer accepts.
const MAX_CHUNK_LABEL: usize = 63;

/// What two table names must not share: the compiler refers to a table by its label
/// and by aliases that differ in case and underscores.
fn name_key(label: &str) -> String {
    label
        .chars()
        .filter(|c| *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

impl Model {
    /// A tree is only as proven as the private trees it calls: a proven caller of a tree
    /// that is not proven (or that the original lacks but the recompiled object has, such
    /// as v2's own `init`) becomes inconclusive. Repeats until callers of callers settle.
    fn propagate_unproven_callees(
        &self,
        trees: &mut [DecompiledTree],
        recompiled: &RecompiledFile,
    ) {
        let proven = |c: &TreeCheck| matches!(c, TreeCheck::Identical | TreeCheck::Equivalent);
        loop {
            let status: BTreeMap<u16, bool> =
                trees.iter().map(|t| (t.id, proven(&t.check))).collect();
            let mut changed = false;
            for (t, model) in trees.iter_mut().zip(&self.trees) {
                if !proven(&t.check) {
                    continue;
                }
                let bad = model.original.instructions.iter().find_map(|ins| {
                    let callee = ins.opcode;
                    if !PRIVATE_TREES.contains(&callee) || callee == t.id {
                        return None;
                    }
                    let ok = match status.get(&callee) {
                        Some(&ok) => ok,
                        None => !recompiled.trees.contains_key(&callee),
                    };
                    (!ok).then_some(callee)
                });
                if let Some(callee) = bad {
                    t.check = TreeCheck::Inconclusive(format!(
                        "calls tree {callee}, which is not proven the same"
                    ));
                    changed = true;
                }
            }
            if !changed {
                return;
            }
        }
    }
}

/// The first OBJD's attribute count.
fn objd_attributes(iff: &Iff) -> Option<u16> {
    let objd = iff.chunks.iter().find(|c| &c.tag == b"OBJD")?;
    let at = 4 + OBJD_ATTRIBUTES_WORD * 2;
    let b = iff.data(objd).get(at..at + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
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
    const TAGS: [&[u8; 4]; 9] = [
        b"STR#", b"CTSS", b"TTAB", b"TTAs", b"SLOT", b"FCNS", b"GLOB", b"TPRP", b"OBJD",
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
    strings: BTreeMap<([u8; 4], u16), StringTexts>,
    attributes: Option<u16>,
}

/// A string table as the VM reads it: its US English texts in order when the VM's
/// reading is plain ([`plain_string_table`]), else every chunk of its tag and ID as
/// stored (flags and bytes), so that only an identical table compares equal. Comments
/// and the table's encoding are not observable.
type StringTexts = std::result::Result<Vec<Vec<u8>>, Vec<(u16, Vec<u8>)>>;

/// The string table instruction `i` shows text from: Show String (21) and Set Action
/// String (50) name one; a private Dialog (36) of an ordinary kind (0 to 4, operand 5's
/// low 7 bits) reads `STR#` 301 for its nonzero string operands (0 to 4, 6). One that
/// names none reads no table, and the other kinds (pickers and panels, `sims-vm`
/// `primitives/dialog.rs`) return before any text is formatted.
fn string_table_read(i: &sims_formats::behavior::Instruction) -> Option<u16> {
    match i.opcode {
        21 | 50 => Some(u16::from_le_bytes([i.operands[0], i.operands[1]])),
        36 if i.operands[5] & 0x7f <= 4
            && [0, 1, 2, 3, 4, 6].iter().any(|&k| i.operands[k] != 0) =>
        {
            Some(301)
        }
        _ => None,
    }
}

/// The `STR#` tables the trees show text from that a v2 `strings` member can write (the
/// table's US English entries, each text one a v2 string literal spells), and those it
/// cannot, with why.
fn strings_models(iff: &Iff, trees: &[TreeModel]) -> (Vec<StringsModel>, Vec<(u16, String)>) {
    let ids: BTreeSet<u16> = trees
        .iter()
        .flat_map(|t| t.original.instructions.iter().filter_map(string_table_read))
        .collect();
    let mut used = BTreeSet::new();
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for id in ids {
        if !iff.chunks.iter().any(|c| &c.tag == b"STR#" && c.id == id) {
            continue;
        }
        let texts = match plain_string_table(iff, b"STR#", id) {
            Ok((_, texts)) => texts,
            Err(why) => {
                skipped.push((id, why.to_owned()));
                continue;
            }
        };
        let texts: Vec<String> = texts
            .iter()
            .map(|t| sims_formats::strings::decode_verbatim(t, 1))
            .collect();
        // A v2 literal spells no other control character, and the table encoder takes no NUL.
        let spellable = |t: &String| {
            !t.chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        };
        // A table with no entries at all is written empty (one with entries and none
        // in US English is not plain).
        if !texts.iter().all(spellable) {
            skipped.push((
                id,
                "a text holds a control character a v2 string does not spell".to_owned(),
            ));
            continue;
        }
        let Some(chunk) = iff.get(b"STR#", id) else {
            continue;
        };
        let entries = texts
            .into_iter()
            .enumerate()
            .map(|(n, text)| (string_entry_name(&text, n + 1, &mut used), text))
            .collect();
        out.push(StringsModel {
            id,
            label: chunk.label.trim().to_owned(),
            entries,
        });
    }
    (out, skipped)
}

/// The one chunk the VM reads table `tag` `id` from and its US English texts in the
/// order it reads them, when that reading is plain; else why not. Plain means what
/// `sims-vm` (`program.rs`: `unported_strings`, `english_fallback`, `loaded_order`) and
/// the original's loader agree on, read the simple way:
/// - one chunk of the tag and ID, not deleted, the one the resource map gives;
/// - flag 0x10 (the loader byte-swaps the header of a resource without it);
/// - format -1, -2 or -3 with a count from 0 to 0x7fff;
/// - US English entries in one run, unless there are none at all (the loader puts a
///   later run of a language in front of the earlier ones, and reads a table with
///   entries and none in English again for English).
fn plain_string_table<'a>(
    iff: &'a Iff,
    tag: &[u8; 4],
    id: u16,
) -> std::result::Result<(&'a sims_formats::iff::Chunk, Vec<Vec<u8>>), &'static str> {
    let mut chunks = iff.chunks.iter().filter(|c| &c.tag == tag && c.id == id);
    let (Some(chunk), None) = (chunks.next(), chunks.next()) else {
        return Err("the file has more than one chunk of its ID");
    };
    if chunk.flags & sims_formats::iff::DELETED != 0 {
        return Err("its chunk is marked deleted");
    }
    if iff.map_error().is_some() || !iff.get(tag, id).is_some_and(|c| c.offset == chunk.offset) {
        return Err("the resource map does not name its chunk");
    }
    if iff.resource_flags(chunk) & 0x10 == 0 {
        return Err("its resource lacks flag 0x10, so the loader byte-swaps its header");
    }
    let data = iff.data(chunk);
    let word = |at: usize| {
        data.get(at..at + 2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
    };
    if !matches!(word(0), Some(-3..=-1)) || !word(2).is_some_and(|n| n >= 0) {
        return Err("its format or count is one the loader reads another way");
    }
    let table = StringTable::parse(data).map_err(|_| "it does not parse")?;
    let english: Vec<usize> = table
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.language == 1)
        .map(|(n, _)| n)
        .collect();
    if english.is_empty() && !table.entries.is_empty() {
        return Err("it has entries and none in US English, which the loader reads again");
    }
    if english.windows(2).any(|w| w[1] != w[0] + 1) {
        return Err("its US English entries are not in one run");
    }
    let texts = english
        .into_iter()
        .map(|n| table.entries[n].raw_text.clone())
        .collect();
    Ok((chunk, texts))
}

/// A `strings` entry name from its text: its first words in snake case, else `s<N>`
/// (`n` is the entry's number from 1), unique within the object.
fn string_entry_name(text: &str, n: usize, used: &mut BTreeSet<String>) -> String {
    const RESERVED: &[&str] = &[
        "if",
        "else",
        "let",
        "return",
        "true",
        "false",
        "self",
        "sim",
        "while",
        "loop",
        "match",
        "fn",
        "tree",
        "const",
        "state",
        "strings",
        "for",
        "in",
        "break",
        "continue",
        "raw",
        "defer",
        "repeat",
        "sound",
        "object",
        "mod",
        "use",
        "pub",
        "enum",
        "struct",
        "stack_obj",
        "temp",
        "param",
        "local",
        "none",
        "some",
    ];
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '$' && c != ':')
        .filter(|w| {
            !w.is_empty() && !w.contains('$') && w.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .take(4)
        .map(str::to_ascii_lowercase)
        .collect();
    let mut name = words.join("_");
    name.truncate(32);
    let name = name.trim_end_matches('_').to_owned();
    let base = if name.is_empty()
        || name.starts_with(|c: char| c.is_ascii_digit())
        || RESERVED.contains(&name.as_str())
    {
        format!("s{n}")
    } else {
        name
    };
    let mut candidate = base.clone();
    if used.contains(&candidate) {
        candidate = format!("{base}_{n}");
    }
    let mut k = 2;
    while used.contains(&candidate) {
        candidate = format!("{base}_{n}_{k}");
        k += 1;
    }
    used.insert(candidate.clone());
    candidate
}

/// The chunks a text read can resolve: `STR#`, the catalog's `CTSS`, and the legacy
/// `CST` the loader falls back on when a file has no `STR#` of the id.
const STRING_TAGS: [&[u8; 4]; 3] = [b"STR#", b"CTSS", b"\0CST"];

/// Each text table, by tag and ID ([`StringTexts`]).
fn string_texts(iff: &Iff) -> BTreeMap<([u8; 4], u16), StringTexts> {
    let keys: BTreeSet<([u8; 4], u16)> = iff
        .chunks
        .iter()
        .filter(|c| STRING_TAGS.contains(&&c.tag))
        .map(|c| (c.tag, c.id))
        .collect();
    keys.into_iter()
        .map(|(tag, id)| {
            let texts = plain_string_table(iff, &tag, id)
                .map(|(_, texts)| texts)
                .map_err(|_| {
                    iff.chunks
                        .iter()
                        .filter(|c| c.tag == tag && c.id == id)
                        .map(|c| (c.flags, iff.data(c).to_vec()))
                        .collect()
                });
            ((tag, id), texts)
        })
        .collect()
}

/// Removes bytes that these v2 spellings drop and the VM never reads. Leave the
/// tree unchanged when any record could hold the extended arguments to an inline call.
fn without_unused_operands<'a>(tree: &'a BhavChunk, ctx: &LiveContext) -> Cow<'a, BhavChunk> {
    let canonical = |i: &sims_formats::behavior::Instruction| {
        let mut operands = i.operands;
        match i.opcode {
            // Animate reads byte 2 only with flag 0x20 (the VM's store_event).
            44 if operands[5] & 0x20 == 0 => operands[2] = 0,
            50 => operands = canonicalize_primitive_operands(i.opcode, operands),
            _ => {}
        }
        operands
    };
    if !tree.instructions.iter().any(|i| canonical(i) != i.operands)
        || tree.instructions.iter().any(|i| {
            i.opcode >= 256
                && i.operands != [0xFF; 8]
                && ctx
                    .callee_arities
                    .as_ref()
                    .and_then(|arities| arities.get(&i.opcode))
                    .is_none_or(|&arity| arity > 4)
        })
    {
        return Cow::Borrowed(tree);
    }
    let mut tree = tree.clone();
    for i in &mut tree.instructions {
        i.operands = canonical(i);
    }
    Cow::Owned(tree)
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
/// no call's inline arguments could run on into the appended node
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
        && a.instructions.iter().enumerate().all(|(i, ins)| {
            // Bytes the original holds from this call's operands to its last node.
            let available = 8 + 12 * (n - 1 - i);
            ins.opcode < 0x100
                || ins.operands == [0xff; 8]
                || ctx
                    .callee_arities
                    .as_ref()
                    .and_then(|m| m.get(&ins.opcode))
                    .is_some_and(|&argc| argc * 2 <= available)
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

fn metadata_ident(label: &str) -> Option<String> {
    let ident = snake_case_ident(label.trim_end_matches('\0'));
    if ident == "tree_" {
        return None;
    }
    let ident = match ident.as_str() {
        "true" | "false" | "if" | "match" | "self" | "sim" | "temp" | "stack_obj" => {
            format!("{ident}_")
        }
        _ => ident,
    };
    Some(ident)
}

fn unique_metadata_ident(label: &str, id: u16, used: &mut BTreeSet<String>) -> Option<String> {
    let mut ident = metadata_ident(label)?;
    if !used.insert(ident.clone()) {
        ident.push_str(&format!("_{id}"));
        while !used.insert(ident.clone()) {
            ident.push('_');
        }
    }
    Some(ident)
}

fn bcon_entries(iff: &Iff, id: u16, values: Vec<i16>) -> Vec<(String, i16)> {
    let str_labels = iff
        .get(b"STR#", id)
        .and_then(|c| StringTable::parse_chunk(iff.data(c), c.flags).ok());
    let trcn_labels = iff
        .get(b"TRCN", id)
        .and_then(|c| trcn_labels(iff.data(c), values.len()))
        .unwrap_or_default();
    let mut used: BTreeSet<String> = (0..values.len()).map(|i| format!("e{i}")).collect();
    values
        .into_iter()
        .enumerate()
        .map(|(i, value)| {
            let trcn = trcn_labels.get(i).and_then(|s| metadata_ident(s));
            let name = trcn
                .or_else(|| metadata_ident(str_labels.as_ref()?.get(i, 1)?))
                .filter(|ident| used.insert(ident.clone()))
                .unwrap_or_else(|| format!("e{i}"));
            (name, value)
        })
        .collect()
}

/// TRCN stores an ordered label and comment per BCON entry. Version 2 uses
/// length-prefixed strings. Earlier versions use padded C strings.
fn trcn_labels(data: &[u8], limit: usize) -> Option<Vec<String>> {
    fn string<'a>(data: &mut &'a [u8], version: u32, pad_even: bool) -> Option<&'a [u8]> {
        if version < 2 {
            let len = data.iter().position(|&b| b == 0)?;
            let padded = (len % 2 == 0) == pad_even;
            let (field, rest) = data.split_at_checked(len + 1 + usize::from(padded))?;
            *data = rest;
            return field.get(..len);
        }
        let mut len = 0u32;
        for shift in (0..=28).step_by(7) {
            let (&byte, rest) = data.split_first()?;
            *data = rest;
            if shift == 28 && byte > 7 {
                return None;
            }
            len |= u32::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                let (field, rest) = data.split_at_checked(len as usize)?;
                *data = rest;
                return Some(field);
            }
        }
        None
    }

    let (header, mut data) = data.split_at_checked(16)?;
    let version = u32::from_le_bytes(header[4..8].try_into().ok()?);
    if header[..4] != [0; 4] || version > 2 {
        return None;
    }
    let count = u32::from_le_bytes(header[12..16].try_into().ok()?) as usize;
    let mut labels = Vec::new();
    for i in 0..count.min(limit) {
        data = data.get(8..)?; // flags, unknown
        let label = string(&mut data, version, i == 0 || version == 0)?;
        string(&mut data, version, true)?;
        if version > 0 {
            data = data.get(5..)?; // range flag and bounds
        }
        labels.push(sims_formats::strings::decode(label, 1));
    }
    Some(labels)
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
        128..=191 => Some(format!("g{}_{entry}", 256 + (table - 128))),
        // No table: the VM reads nothing there (`Program::constant`).
        _ => None,
    }
}

/// Whether a body reads mostly the Sim (`my.motive`, `my.person`) and the object as the
/// stack object (`stack.attr`), as interaction trees do.
fn prefers_sim_context(body: &[Stmt]) -> bool {
    let (mut sim, mut obj) = (0usize, 0usize);
    walk_stmts(body, &mut |s| {
        for op in stmt_operands(s) {
            if let OperandRef::Direct(o) = op {
                match (o.scope, o.index) {
                    // `my.id` is `sim` in one context and `self` in the other.
                    (3, 11) => {}
                    (1 | 14 | 16 | 18, _) => sim += 1,
                    (0 | 3, _) => obj += 1,
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
    /// Whether `stack_obj` itself is the cursor (no loop local).
    stack_cursor: bool,
    body: &'a [Stmt],
}

fn for_each_parts<'a>(init: &Stmt, w: &'a Stmt, restore: &Stmt) -> Option<ForEachParts<'a>> {
    let Stmt::Assign {
        lhs: rl,
        op: AssignOp::Set,
        rhs: rr,
        ..
    } = restore
    else {
        return None;
    };
    let saved = direct_local(rr)?;
    if !is_operand(rl, 10, 0) {
        return None;
    }
    for_each_core(init, w, Some(saved)).map(|(mode, stack_cursor, body)| ForEachParts {
        saved,
        mode,
        stack_cursor,
        body,
    })
}

fn direct_local(r: &OperandRef) -> Option<i16> {
    match r {
        OperandRef::Direct(Operand { scope: 25, index }) => Some(*index),
        _ => None,
    }
}

fn is_operand(r: &OperandRef, scope: u8, index: i16) -> bool {
    matches!(r, OperandRef::Direct(o) if o.scope == scope && o.index == index)
}

/// `local[v] := 0; while set_to_next(mode, dest: local[v]) { body }`: the mode, whether the
/// cursor is `stack_obj` itself (v2 PR F's zero-local form, `stack_obj := 0; while
/// set_to_next(mode, dest: stack_obj)`), and the body, when the body names neither `local[v]`
/// nor the local `stack_obj` is saved in.
fn for_each_core<'a>(
    init: &Stmt,
    w: &'a Stmt,
    saved: Option<i16>,
) -> Option<(u8, bool, &'a [Stmt])> {
    let local = direct_local;
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
    // `None`: v2 PR F's zero-local form, with `stack_obj` itself as the cursor.
    let var = if is_operand(il, 10, 0) {
        None
    } else {
        Some(local(il)?)
    };
    if !is_operand(ir, 7, 0) || (var.is_some() && saved == var) {
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
    let dest_ok = match var {
        Some(v) => local(dest) == Some(v),
        None => is_operand(dest, 10, 0),
    };
    if !dest_ok || !matches!(mode, 0 | 1) {
        return None;
    }
    // The body must not name either local: v2 gives the loop its own slots. A
    // `stack_obj` cursor must not be written by a plain store either.
    let mut names_local = false;
    walk_stmts(body, &mut |s| {
        names_local |= stmt_operands(s)
            .iter()
            .any(|r| local(r).is_some_and(|l| Some(l) == var || Some(l) == saved));
        if var.is_none()
            && let Stmt::Assign { lhs, .. } = s
        {
            names_local |= is_operand(lhs, 10, 0);
        }
    });
    (!names_local).then_some((*mode, var.is_none(), body.as_slice()))
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

/// The value of `s` when it is `scope(scope, index) := value`.
fn assigned_to(s: &Stmt, scope: u8, index: i16) -> Option<&OperandRef> {
    match s {
        Stmt::Assign {
            lhs: OperandRef::Direct(o),
            op: AssignOp::Set,
            rhs,
            ..
        } if o.scope == scope && o.index == index => Some(rhs),
        _ => None,
    }
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
        PrimitiveCall::SetToNext { guid, .. } if *guid != 0 => out.push(*guid),
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

/// The callee and argument encoding of a structured call.
fn stmt_call(s: &Stmt) -> Option<(u16, bool)> {
    let call = |target: &CallTarget, pass_temps: bool| match target {
        CallTarget::Id(id) => Some((*id, pass_temps)),
        _ => None,
    };
    match s {
        Stmt::Call {
            target, pass_temps, ..
        } => call(target, *pass_temps),
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => {
            let mut c = cond;
            while let Condition::Not(inner) = c {
                c = inner;
            }
            match c {
                Condition::Call {
                    target, pass_temps, ..
                } => call(target, *pass_temps),
                _ => None,
            }
        }
        _ => None,
    }
}

fn call_params(arity: usize) -> impl Iterator<Item = String> {
    (0..arity).map(|i| format!("arg_{i}: i16"))
}

/// The highest slot that needs a named declaration for a primitive's local arguments.
fn builtin_local(s: &Stmt) -> Option<u16> {
    let prim = match s {
        Stmt::Primitive {
            prim,
            branches: None,
            ..
        } => prim,
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => {
            let mut cond = cond;
            while let Condition::Not(inner) = cond {
                cond = inner;
            }
            let Condition::Primitive { prim, .. } = cond else {
                return None;
            };
            prim
        }
        _ => return None,
    };
    if let PrimitiveCall::Animate {
        flags,
        local_index: LocalRef::Index(i),
        ..
    } = prim
        && flags & 0x20 != 0
        && matches!(s, Stmt::If { .. } | Stmt::While { .. })
    {
        return Some(u16::from(*i));
    }
    match unpadded(prim) {
        PrimitiveCall::FindLocation {
            local: LocalRef::Index(i),
            flags,
            ..
        } if flags & 1 != 0 || *i != 0 => Some(u16::from(*i)),
        PrimitiveCall::SetToNext {
            mode,
            local: LocalRef::Index(i),
            ..
        } if mode & 0x7F == 9 => Some(u16::from(*i)),
        PrimitiveCall::Relationship {
            direction,
            local: LocalRef::Index(i),
            value,
            ..
        } => {
            let value = u16::try_from(direct_local(value)?).ok()?;
            Some(if matches!(direction, 2 | 3) || *i != 0 {
                value.max(u16::from(*i))
            } else {
                value
            })
        }
        _ => None,
    }
}

fn unpadded(prim: &PrimitiveCall) -> &PrimitiveCall {
    match prim {
        PrimitiveCall::Padded { call, .. } => call,
        _ => prim,
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
    /// Lifecycle blocks written with their whole header as `@frame(..)`: those the default
    /// header does not reproduce.
    framed: BTreeSet<u16>,
    /// Why the catalog variants are left out (the compiler rejected them).
    variants_off: Option<String>,
    /// Why the `strings` members are left out (the compiler rejected them).
    strings_off: Option<String>,
    /// Trees written without the extra idioms (see [`Translator::extra`]), after a round
    /// that used them was not proven.
    plain: BTreeSet<u16>,
    /// Trees some caller runs in the other context: written `@context(inherit)` (an extra
    /// idiom) so that either context may call them.
    inherit: BTreeSet<u16>,
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
        // A lifecycle block carries any other header as `@frame(..)`, but never an empty
        // one (entering a 0-node tree faults; v2 refuses `@frame(empty)` there), so a
        // 0-node lifecycle tree starts plain.
        let standalone = model
            .trees
            .iter()
            .filter(|t| t.lifecycle.is_some() && t.original.instructions.is_empty())
            .map(|t| t.id)
            .collect();
        // Formats and kinds the compiler never gives a block by default start with `@frame`;
        // anything else gets one when the default header does not compile or is not proven
        // (see `add_frame`).
        let framed = model
            .trees
            .iter()
            .filter(|t| {
                let b = &t.original;
                t.lifecycle.is_some() && (b.format != 0x8002 || b.kind != 0)
            })
            .map(|t| t.id)
            .collect();
        // A private tree called from a tree of the other context (by the contexts the
        // trees prefer); never a lifecycle tree or a variant's init/main.
        let mut inherit = BTreeSet::new();
        for caller in &model.trees {
            let Some(body) = &caller.body else {
                continue;
            };
            walk_stmts(body, &mut |s| {
                if let Some((id, _)) = stmt_call(s)
                    && let Some(callee) = model.tree(id)
                    && callee.sim_ctx != caller.sim_ctx
                    && callee.body.is_some()
                    && callee.lifecycle.is_none()
                    && !model.variants.iter().any(|v| v.init == id || v.main == id)
                {
                    inherit.insert(id);
                }
            });
        }
        Self {
            styles,
            raw_stmts: BTreeMap::new(),
            standalone,
            framed,
            variants_off: None,
            strings_off: None,
            plain: BTreeSet::new(),
            inherit,
        }
    }

    /// Whether tree `t` is written `@context(inherit)` this round.
    fn inherits(&self, t: &TreeModel) -> bool {
        self.inherit.contains(&t.id) && !self.plain.contains(&t.id) && self.level(t).idiomatic()
    }

    /// Turns off tree `id`'s extra idioms when the last round used them. `false` when it
    /// did not, so the tree has to move down a style instead.
    /// Writes lifecycle block `id` with its whole header as `@frame(..)` from now on, when
    /// it is still written with the default one. Tried before anything else: the header is
    /// the original's, so it only ever helps.
    fn add_frame(&mut self, model: &Model, id: u16) -> bool {
        model.tree(id).is_some_and(|t| t.lifecycle.is_some())
            && !self.standalone.contains(&id)
            && self.framed.insert(id)
    }

    fn drop_extras(&mut self, id: u16, generated: &Generated) -> bool {
        generated.extras.contains(&id) && self.plain.insert(id)
    }

    /// [`Self::drop_extras`] for every tree, for a failure no one tree is blamed for.
    fn drop_all_extras(&mut self, generated: &Generated) -> bool {
        let before = self.plain.len();
        self.plain.extend(generated.extras.iter().copied());
        self.plain.len() != before
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
        t.sim_ctx && self.level(t) == Level::Idiomatic && !self.inherits(t)
    }

    /// Moves tree `id` one style down. `false` when it is already at its last style.
    fn lower_tree(&mut self, model: &Model, id: u16) -> bool {
        let Some(t) = model.tree(id) else {
            return false;
        };
        let next = match self.level(t) {
            Level::Idiomatic if t.sim_ctx => {
                // The extra idioms get another try in the object's context.
                self.plain.remove(&id);
                Level::IdiomaticObject
            }
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
    /// Leaves the catalog variants out when the compiler rejects them, or rejects the file
    /// at no particular line while they are written. `false` when that cannot help.
    fn drop_variants(
        &mut self,
        model: &Model,
        generated: &Generated,
        line: Option<usize>,
        message: &str,
    ) -> bool {
        let at_variant =
            line.is_none_or(|l| generated.variant_lines.contains(&l.saturating_sub(1)));
        if self.variants_off.is_some() || model.variants.is_empty() || !at_variant {
            return false;
        }
        self.variants_off = Some(format!(
            "the compiler rejected the variants: {}",
            message.lines().next().unwrap_or_default()
        ));
        true
    }

    /// Leaves the `strings` members out when the compiler rejects one of them. `false` when
    /// that cannot help.
    fn drop_strings(
        &mut self,
        model: &Model,
        generated: &Generated,
        line: Option<usize>,
        message: &str,
    ) -> bool {
        let at_strings = line
            .is_some_and(|l| generated.strings_lines.contains(&l.saturating_sub(1)))
            || (line.is_none() && message.contains("STR#"));
        if self.strings_off.is_some() || model.strings.is_empty() || !at_strings {
            return false;
        }
        self.strings_off = Some(message.lines().next().unwrap_or_default().to_owned());
        true
    }

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
    /// Trees that used an extra idiom (see [`Translator::extra`]).
    extras: BTreeSet<u16>,
    /// 0-based lines holding the `variant` members.
    variant_lines: std::ops::Range<usize>,
    /// 0-based lines holding the `strings` members.
    strings_lines: std::ops::Range<usize>,
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
        let mut extras = BTreeSet::new();
        out.line("// Decompiled by `sims-antics lift --v2`.");
        if self.extra_objds > 0 {
            out.line(format!(
                "// The file has {} additional OBJD chunks that v2 cannot write as catalog variants.",
                self.extra_objds
            ));
        }
        out.line("");

        let mut lib_globals: Vec<u16> = Vec::new();
        let mut lib_semis: Vec<u16> = Vec::new();
        for &id in self.extern_calls.keys() {
            if id >= 8192 {
                lib_semis.push(id);
            } else if id >= 256 {
                lib_globals.push(id);
            }
        }
        for (module, ids) in [("globals", &lib_globals), ("semi_globals", &lib_semis)] {
            if ids.is_empty() {
                continue;
            }
            out.line(format!("extern mod {module} {{"));
            out.indent += 1;
            for id in ids {
                let (params, comment) =
                    match (self.literal_calls.get(id), self.library_arities.get(id)) {
                        (Some(&arity), _) => (
                            call_params(arity).collect::<Vec<_>>().join(", "),
                            String::new(),
                        ),
                        (None, Some(n)) => (
                            String::new(),
                            format!(" // {n} argument(s), passed in temps"),
                        ),
                        _ => (String::new(), String::new()),
                    };
                let ident = &self.extern_calls[id];
                out.line(format!("@id({id}) fn {ident}({params}) -> bool;{comment}"));
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

        let name = primary_object_name(&self.name);
        out.line(format!("object \"{name}\" {{"));
        out.indent += 1;
        out.line(format!("guid: {};", hex_u32(self.guid)));
        out.line(format!("price: {};", self.price));
        if let Some(n) = self.attributes {
            out.line(format!("attributes: {n};"));
        }
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
            for (name, value) in &b.entries {
                out.line(format!("{name} = {value},"));
            }
            out.indent -= 1;
            out.line("}");
            out.line("");
        }
        let strings_start = out.lines.len();
        if let Some(why) = &plan.strings_off {
            if !self.strings.is_empty() {
                out.line(format!("// strings are not written: {why}."));
                out.line("");
            }
        } else {
            for (id, why) in &self.strings_skipped {
                out.line(format!("// strings {id} is not written: {why}."));
            }
            if !self.strings_skipped.is_empty() {
                out.line("");
            }
            for st in &self.strings {
                out.line(format!(
                    "strings {} \"{}\" {{",
                    st.id,
                    v2_string_literal(&st.label)
                ));
                out.indent += 1;
                for (name, text) in &st.entries {
                    out.line(format!("{name} = \"{}\",", v2_string_literal(text)));
                }
                out.indent -= 1;
                out.line("}");
                out.line("");
            }
        }
        let strings_lines = strings_start..out.lines.len();
        for s in &self.states {
            out.line(format!("state attr_{s} @ {s}: i16;"));
        }
        if !self.sounds.is_empty() {
            out.line("");
        }
        for (id, sound) in &self.sounds {
            out.line(format!(
                "sound {} = preset(\"ui_click\", wav: \"{}\", id: {id});",
                sound.ident, sound.wav
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
                Some(kind) if !plan.framed.contains(&t.id) => {
                    out.line(format!("@id({}) {} {{", t.id, kind.keyword()));
                }
                Some(kind) => {
                    // The block keeps the tree's own header, as a plain tree does.
                    let b = &t.original;
                    let frame_kind = if b.kind == 0 {
                        String::new()
                    } else {
                        format!(", kind: {}", b.kind)
                    };
                    out.line(format!(
                        "@id({}) @frame(args: {}, locals: {}, format: {:#06x}, revision: {}{frame_kind}) {} {{",
                        t.id,
                        b.args,
                        b.locals,
                        b.format,
                        b.revision,
                        kind.keyword()
                    ));
                }
                None => {
                    if let Some(kind) = t.lifecycle {
                        out.line(format!(
                            "// The object's {0} tree, written as a plain tree: the build gives the object its own `{0}` tree at a new ID.",
                            kind.keyword()
                        ));
                    }
                    let b = &t.original;
                    let mut params = Vec::new();
                    if sim {
                        params.push("sim: Sim".to_owned());
                    }
                    if let Some(&arity) = self.literal_calls.get(&t.id) {
                        params.extend(call_params(arity));
                    }
                    let ctx = if params.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", params.join(", "))
                    };
                    let mut kind = if b.kind == 0 {
                        String::new()
                    } else {
                        format!(", kind: {}", b.kind)
                    };
                    // A 0-node tree (entering it faults) is `@frame(empty)` with a `{}` body.
                    if b.instructions.is_empty() {
                        kind.push_str(", empty");
                    }
                    let context = if plan.inherits(t) {
                        extras.insert(t.id);
                        " @context(inherit)"
                    } else {
                        ""
                    };
                    out.line(format!(
                        "@id({}) @frame(args: {}, locals: {}, format: {:#06x}, revision: {}{kind}){context} tree {} \"{}\"{ctx} {{",
                        t.id, b.args, b.locals, b.format, b.revision, t.ident, t.label
                    ));
                }
            }
            out.indent += 1;
            let mut raw_count = 0;
            match &t.body {
                Some(body) if level.idiomatic() => {
                    let mut named_locals = 0;
                    if as_lifecycle.is_none() && !plan.plain.contains(&t.id) {
                        walk_stmts(body, &mut |s| {
                            if let Some(index) = builtin_local(s)
                                && index < t.original.locals
                            {
                                named_locals = named_locals.max(index + 1);
                            }
                        });
                    }
                    let mut tr = Translator {
                        model: self,
                        plan,
                        tree: t,
                        sim,
                        inherit: as_lifecycle.is_none() && plan.inherits(t),
                        next_id: 0,
                        raw: Vec::new(),
                        raw_count: 0,
                        ranges: &mut stmt_ranges,
                        loop_depth: 0,
                        in_handler: false,
                        hoisted_save: hoisted_self_save(body, &t.lifted),
                        sims_loop: None,
                        cursor: None,
                        named_locals,
                        extras: std::cell::Cell::new(false),
                    };
                    if named_locals > 0 && tr.extra() {
                        // `@slot` pins each word the builtins address by number; the
                        // build keeps a pinned word in the frame even when nothing names it.
                        for i in 0..named_locals {
                            out.line(format!("@slot({i}) let slot_{i}: i16;"));
                        }
                    }
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
                    if tr.extras.get() {
                        extras.insert(t.id);
                    }
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
        let variant_start = out.lines.len();
        for v in &self.variants {
            match self.variant_fields(v, plan) {
                Ok(fields) => {
                    out.line(format!("variant \"{}\" {{", v.name));
                    out.indent += 1;
                    for f in fields {
                        out.line(format!("{f},"));
                    }
                    out.indent -= 1;
                    out.line("}");
                }
                Err(why) => out.line(format!(
                    "// variant \"{}\" ({}) is not written: {why}.",
                    v.name,
                    hex_u32(v.guid)
                )),
            }
            out.line("");
        }
        let variant_lines = variant_start..out.lines.len();
        while out.lines.last().is_some_and(String::is_empty) {
            out.lines.pop();
        }
        out.indent = 0;
        out.line("}");
        // A GUID constant only `raw` text needed (its trees went `raw`) is left out; the
        // blank line keeps the line numbers the statement ranges use.
        for name in self.guid_consts.values() {
            let decl = format!("const {name} = ");
            let is_decl = |l: &str| l.trim_start().starts_with(&decl);
            let used = out
                .lines
                .iter()
                .any(|l| !is_decl(l) && l.contains(name.as_str()));
            if !used && let Some(l) = out.lines.iter_mut().find(|l| is_decl(l)) {
                l.clear();
            }
        }
        let mut text = out.lines.join("\n");
        text.push('\n');
        Generated {
            text,
            stmt_ranges,
            tree_ranges,
            raw_counts,
            extras,
            variant_lines,
            strings_lines,
        }
    }
}

/// `text` as the inside of a v2 string literal.
fn v2_string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
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

/// Byte spans of one written v2 line's tokens (identifiers and numbers, strings, single
/// punctuation), or `None` for a line with a comment.
fn ladder_tokens(line: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let from = i;
        if c == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(b.len());
        } else if c == b'/' && b.get(i + 1).is_some_and(|n| matches!(n, b'/' | b'*')) {
            return None;
        } else if c.is_ascii_alphanumeric() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
        } else {
            i += line[i..].chars().next().map_or(1, char::len_utf8);
        }
        out.push(from..i);
    }
    Some(out)
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
    /// `@context(inherit)`: the runner's cells are `me.*` and the stack object's
    /// `stack_obj.*`; `self` and `sim` are not written.
    inherit: bool,
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
    /// The binding of the innermost `for p in lot.sims()` (`None` outside one, or inside
    /// a `lot.objects()` loop nested in it).
    sims_loop: Option<String>,
    /// The binding of the innermost `for` loop whose cursor is `stack_obj` itself: inside it,
    /// the stack object's cells are the binding's (`o.data[i]`, `p.person[i]`).
    cursor: Option<String>,
    /// Number of leading frame slots declared for builtins with named local operands.
    named_locals: u16,
    /// Whether an extra idiom was written (see [`Translator::extra`]).
    extras: std::cell::Cell<bool>,
}

impl Translator<'_> {
    /// Whether the extra idioms are on for this tree, noting that one is written. These are
    /// the PR E spellings (`match`, `inc_lt` / `dec_gt`, `param[n]`, `self.slot[i]`,
    /// `p.person[i]` / `p.kind`, `run_functional`, `set_dynamic_sprite`,
    /// `push_interaction(object: ..)`, the padded `notify_out_of_idle()`). Some lower
    /// differently from the original (`run_functional` and `p.person` restore `stack_obj`),
    /// and one the checker cannot prove would cost the whole tree its style, so a tree that
    /// is not proven with them is retried without them before it moves down a style.
    fn extra(&self) -> bool {
        if self.plan.plain.contains(&self.tree.id) {
            return false;
        }
        self.extras.set(true);
        true
    }

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
            if let Some(used) = self.value_builtin(&body[i..], out) {
                i += used;
                continue;
            }
            if let Some(used) = self.pair(&body[i..], out) {
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
                if self.try_dialog(id, cond, then_body, else_body, out) {
                    return;
                }
                if self.match_chain(id, s, out) {
                    return;
                }
                if self.return_compare(id, cond, then_body, else_body, out) {
                    return;
                }
                let Some(c) = self.cond(cond) else {
                    self.emit_raw(s);
                    skip_children(self);
                    return;
                };
                self.flush_raw(out);
                let start = out.lines.len();
                out.line(format!("if {c} {{"));
                self.if_rest(id, start, then_body, else_body, out);
                self.constant_ladder(id, start, out);
            }
            Stmt::While { cond, body, .. } => {
                let animate = self.animate_loop(cond, body.is_empty());
                let handler = animate.is_some();
                let header = match animate {
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

    /// `if a op b { return true; } else { return false; }` as `return a op b;`, and the
    /// other polarity as `return !(a op b);`: each lowers to the same compare node with
    /// the same exits (so the same last result). Also a primitive test returned whole
    /// (`return can_afford(..);`), in that polarity only. No tree calls.
    fn return_compare(
        &mut self,
        id: usize,
        cond: &Condition,
        then_body: &[Stmt],
        else_body: &[Stmt],
        out: &mut Out,
    ) -> bool {
        let (cond, negated) = match cond {
            Condition::Not(inner) => (inner.as_ref(), true),
            c => (c, false),
        };
        let compare = matches!(
            cond,
            Condition::Compare {
                op: CompareOp::Gt
                    | CompareOp::Lt
                    | CompareOp::Eq
                    | CompareOp::Ge
                    | CompareOp::Le
                    | CompareOp::Ne,
                ..
            }
        );
        if !compare && !matches!(cond, Condition::Primitive { .. }) {
            return false;
        }
        let returns = |body: &[Stmt]| match body {
            [Stmt::Return { target, .. }] => match target {
                Target::ReturnTrue => Some(true),
                Target::ReturnFalse => Some(false),
                _ => None,
            },
            _ => None,
        };
        let (Some(on_true), Some(on_false)) = (returns(then_body), returns(else_body)) else {
            return false;
        };
        if on_true == on_false
            || (!compare && on_true == negated)
            || self.forced_raw(id + 1)
            || self.forced_raw(id + 2)
        {
            return false;
        }
        let Some(c) = self.cond(cond) else {
            return false;
        };
        self.flush_raw(out);
        let start = out.lines.len();
        if on_true != negated {
            out.line(format!("return {c};"));
        } else {
            out.line(format!("return !({c});"));
        }
        self.record(id, start, out);
        self.next_id = id + 3;
        true
    }

    fn try_dialog(
        &mut self,
        id: usize,
        cond: &Condition,
        then_body: &[Stmt],
        else_body: &[Stmt],
        out: &mut Out,
    ) -> bool {
        let (cond, success, failure, failure_id, failure_first) =
            if let Condition::Not(inner) = cond {
                (inner.as_ref(), else_body, then_body, id + 1, true)
            } else {
                (
                    cond,
                    then_body,
                    else_body,
                    id + 1 + count_stmts(then_body),
                    false,
                )
            };
        if self.forced_raw(failure_id)
            || !matches!(
                failure,
                [Stmt::Return {
                    target: Target::Error,
                    ..
                }]
            )
        {
            return false;
        }
        let Condition::Primitive {
            prim:
                PrimitiveCall::Dialog {
                    kind,
                    message,
                    title,
                    button1,
                    button2,
                    button3,
                    mode4_str,
                    flags,
                },
            ..
        } = cond
        else {
            return false;
        };
        let mut fields = vec![format!("kind: {kind}")];
        for (name, value) in [
            ("message", message),
            ("title", title),
            ("button1", button1),
            ("button2", button2),
            ("button3", button3),
            ("mode4_str", mode4_str),
        ] {
            let StringRef::Index(index) = value else {
                return false;
            };
            if *index != 0 || matches!(name, "message" | "title") {
                fields.push(format!("{name}: {index}"));
            }
        }
        if *flags != 0 {
            fields.push(format!("flags: {flags}"));
        }
        if !self.extra() {
            return false;
        }
        self.flush_raw(out);
        let start = out.lines.len();
        out.line(format!("dialog({})?;", fields.join(", ")));
        self.record(id, start, out);
        if failure_first {
            self.next_id += 1;
            self.block(success, out);
        } else {
            self.block(success, out);
            self.next_id += 1;
        }
        true
    }

    /// The rest of an `if` after its header line: the body, an `else if` (when the `else`
    /// holds just a translatable `if`) or `else`, and the closing brace.
    fn if_rest(
        &mut self,
        id: usize,
        start: usize,
        then_body: &[Stmt],
        else_body: &[Stmt],
        out: &mut Out,
    ) {
        out.indent += 1;
        self.block(then_body, out);
        self.flush_raw(out);
        out.indent -= 1;
        if let [
            Stmt::If {
                cond,
                then_body: inner_then,
                else_body: inner_else,
                ..
            },
        ] = else_body
            && !self.forced_raw(self.next_id)
            && let Some(c) = self.cond(cond)
        {
            let inner_id = self.next_id;
            self.next_id += 1;
            let inner_start = out.lines.len();
            out.line(format!("}} else if {c} {{"));
            self.if_rest(inner_id, inner_start, inner_then, inner_else, out);
            self.record(id, start, out);
            return;
        }
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

    /// v2 PR H's branch specialization, `const K = if c { a } else { b };`, for an `if`
    /// whose arms are the same text but for integer literals in value positions: the arms
    /// become one copy reading `K`, which v2 copies per arm again. v2 copies the statements
    /// up to the last one naming `K` and joins after them, so the arm's last statement
    /// must name it; and the arm declares no `let`, which the copy would leave in scope
    /// after the join. Lines `start..` are the written `if`; rewritten in place when it
    /// applies.
    fn constant_ladder(&mut self, id: usize, start: usize, out: &mut Out) {
        let lines = &out.lines[start..];
        let Some(head) = lines.first() else {
            return;
        };
        let indent = head.len() - head.trim_start().len();
        let Some(cond) = head
            .trim_start()
            .strip_prefix("if ")
            .and_then(|h| h.strip_suffix(" {"))
        else {
            return;
        };
        // The `if`'s own lines: the header, `} else {` and the closing `}`.
        let own: Vec<usize> = (1..lines.len())
            .filter(|&k| lines[k].len() - lines[k].trim_start().len() <= indent)
            .collect();
        let [mid, end] = own[..] else {
            return;
        };
        if lines[mid].trim() != "} else {" || lines[end].trim() != "}" || end + 1 != lines.len() {
            return;
        }
        let (then_arm, else_arm) = (&lines[1..mid], &lines[mid + 1..end]);
        if then_arm.is_empty()
            || then_arm.len() != else_arm.len()
            || then_arm.iter().any(|l| {
                let t = l.trim_start();
                l.contains("raw {") || t.starts_with('@') || t.starts_with("let ") || t.is_empty()
            })
        {
            return;
        }
        // The first line of the arm's last statement.
        let arm_indent = indent + 4;
        let last_stmt = then_arm
            .iter()
            .rposition(|l| {
                l.len() - l.trim_start().len() == arm_indent && !l.trim_start().starts_with('}')
            })
            .unwrap_or(0);
        // (line, byte span, then value, else value) per differing literal.
        let mut holes: Vec<(usize, std::ops::Range<usize>, String, String)> = Vec::new();
        for (k, (a, b)) in then_arm.iter().zip(else_arm).enumerate() {
            let (Some(ta), Some(tb)) = (ladder_tokens(a), ladder_tokens(b)) else {
                return;
            };
            if ta.len() != tb.len() {
                return;
            }
            for (n, (ra, rb)) in ta.iter().zip(&tb).enumerate() {
                let (x, y) = (&a[ra.clone()], &b[rb.clone()]);
                if x == y {
                    continue;
                }
                let value = |t: &str| t.starts_with(|c: char| c.is_ascii_digit());
                let before = n.checked_sub(1).map(|p| &a[ta[p].clone()]);
                let after = ta.get(n + 1).map(|r| &a[r.clone()]);
                // A value operand only: not an index, a field, a negation, a pattern or
                // a named argument's key.
                if !value(x)
                    || !value(y)
                    || matches!(before, Some("[" | "." | "-" | "@"))
                    || matches!(after, Some("=" | ":" | "[" | "."))
                {
                    return;
                }
                holes.push((k, ra.clone(), x.to_owned(), y.to_owned()));
            }
        }
        if holes.is_empty()
            || holes.len() > 4
            || !holes.iter().any(|h| h.0 >= last_stmt)
            || !self.extra()
        {
            return;
        }
        let names: Vec<String> = if holes.len() == 1 {
            vec![format!("ARM_{id}")]
        } else {
            (0..holes.len())
                .map(|n| format!("ARM_{id}_{}", (b'A' + n as u8) as char))
                .collect()
        };
        let tuple = |values: Vec<&str>| {
            if values.len() == 1 {
                values[0].to_owned()
            } else {
                format!("({})", values.join(", "))
            }
        };
        let binding = tuple(names.iter().map(String::as_str).collect());
        let then_values = tuple(holes.iter().map(|h| h.2.as_str()).collect());
        let else_values = tuple(holes.iter().map(|h| h.3.as_str()).collect());
        let pad = " ".repeat(indent);
        let mut region: Vec<String> = then_arm
            .iter()
            .map(|l| l.strip_prefix("    ").unwrap_or(l).to_owned())
            .collect();
        for ((k, span, _, _), name) in holes.iter().zip(&names).rev() {
            // The arm lines lost one indent level: four bytes.
            let span = span.start - 4..span.end - 4;
            region[*k].replace_range(span, name);
        }
        let mut rewritten = vec![format!(
            "{pad}const {binding} = if {cond} {{ {then_values} }} else {{ {else_values} }};"
        )];
        rewritten.extend(region);
        out.lines.truncate(start);
        out.lines.extend(rewritten);
        // The statements inside are one statement now: a failure there writes the `if` raw.
        self.ranges
            .retain(|r| r.tree != self.tree.id || r.start < start);
        self.record(id, start, out);
    }

    /// `if x == a { } else if x == b { } else if x == c { } [else { }]`, three or more
    /// distinct literals against one operand, as `match x { a => { } ... _ => { } }`. v2
    /// lowers a `match` to that same chain. Returns `false` (writing nothing) otherwise.
    fn match_chain(&mut self, id: usize, s: &Stmt, out: &mut Out) -> bool {
        // (literal, arm body) per test, and the final `else` body.
        let mut arms: Vec<(i16, &[Stmt])> = Vec::new();
        let mut scrutinee: Option<&OperandRef> = None;
        let mut cur = s;
        let fallback: &[Stmt] = loop {
            let Stmt::If {
                cond:
                    Condition::Compare {
                        lhs,
                        op: CompareOp::Eq,
                        rhs: OperandRef::Direct(Operand { scope: 7, index }),
                        ..
                    },
                then_body,
                else_body,
                ..
            } = cur
            else {
                return false;
            };
            if scrutinee.is_some_and(|x| x != lhs) || arms.iter().any(|(v, _)| v == index) {
                return false;
            }
            scrutinee = Some(lhs);
            arms.push((*index, then_body));
            match else_body.as_slice() {
                [next @ Stmt::If { .. }] if arms.len() < 3 || self.chain_continues(next, lhs) => {
                    cur = next;
                }
                rest => break rest,
            }
        };
        if arms.len() < 3 {
            return false;
        }
        // Every `if` of the chain gets an arm; none may be forced raw.
        let mut ids = Vec::with_capacity(arms.len());
        let mut next = id + 1;
        for (_, body) in &arms {
            ids.push(next - 1);
            next += count_stmts(body) + 1;
        }
        if ids.iter().any(|&i| self.forced_raw(i)) {
            return false;
        }
        let Some(x) = scrutinee.and_then(|x| self.rvalue(x)) else {
            return false;
        };
        if !self.extra() {
            return false;
        }
        self.flush_raw(out);
        let start = out.lines.len();
        out.line(format!("match {x} {{"));
        out.indent += 1;
        let mut arm_starts = Vec::with_capacity(arms.len());
        for (k, (value, body)) in arms.iter().enumerate() {
            if k > 0 {
                // The `if` this arm comes from.
                self.next_id += 1;
            }
            arm_starts.push(out.lines.len());
            out.line(format!("{value} => {{"));
            out.indent += 1;
            self.block(body, out);
            self.flush_raw(out);
            out.indent -= 1;
            out.line("}");
        }
        if !fallback.is_empty() {
            out.line("_ => {");
            out.indent += 1;
            self.block(fallback, out);
            self.flush_raw(out);
            out.indent -= 1;
            out.line("}");
        }
        out.indent -= 1;
        out.line("}");
        for (k, arm_start) in arm_starts.into_iter().enumerate().skip(1) {
            self.record(ids[k], arm_start, out);
        }
        self.record(id, start, out);
        true
    }

    /// Whether `s` is another `x == literal` test of the chain on `lhs`.
    fn chain_continues(&self, s: &Stmt, lhs: &OperandRef) -> bool {
        matches!(s, Stmt::If {
            cond: Condition::Compare {
                lhs: l,
                op: CompareOp::Eq,
                rhs: OperandRef::Direct(Operand { scope: 7, .. }),
                ..
            },
            ..
        } if l == lhs)
    }

    /// Folds the setup and result store of a value builtin within one structured block.
    fn value_builtin(&mut self, rest: &[Stmt], out: &mut Out) -> Option<usize> {
        let id = self.next_id;
        if self.plan.plain.contains(&self.tree.id) || self.forced_raw(id) {
            return None;
        }
        let (used, text) = self.job_data(rest).or_else(|| self.distance_to(rest))?;
        if (id + 1..id + used).any(|i| self.forced_raw(i)) || !self.extra() {
            return None;
        }
        self.next_id += used;
        self.flush_raw(out);
        let start = out.lines.len();
        out.line(text);
        self.record(id, start, out);
        Some(used)
    }

    /// `temp[0] := career; temp[1] := level; dest := scope(33, field)`.
    fn job_data(&self, rest: &[Stmt]) -> Option<(usize, String)> {
        let (career, rest) = if let Some(career) = assigned_to(rest.first()?, 8, 0) {
            (Some(career), &rest[1..])
        } else {
            // The optimizer drops an identity career store before a plain level store.
            (None, rest)
        };
        let [level, read, ..] = rest else {
            return None;
        };
        let level = assigned_to(level, 8, 1)?;
        let Stmt::Assign {
            lhs,
            op: AssignOp::Set,
            rhs: OperandRef::Direct(Operand { scope: 33, index }),
            ..
        } = read
        else {
            return None;
        };
        if *index < 0 || is_operand(level, 8, 0) {
            return None;
        }
        let (lhs, level) = (self.lvalue(lhs)?, self.rvalue(level)?);
        let career_arg = match career {
            Some(operand) => self.rvalue(operand)?,
            None => "temp[0]".to_owned(),
        };
        Some((
            usize::from(career.is_some()) + 2,
            format!("{lhs} = job_data({career_arg}, {level}, {index});"),
        ))
    }

    /// Get Distance To writes a temp. An adjacent assignment can use the builtin's return
    /// value directly. A move can be folded when the distance is the only use of the entry save.
    fn distance_to(&self, rest: &[Stmt]) -> Option<(usize, String)> {
        if self.cursor.is_some() {
            return None;
        }
        let (target, get, tail, used) = match rest {
            [get @ Stmt::Primitive { prim, .. }, tail @ ..]
                if matches!(unpadded(prim), PrimitiveCall::GetDistanceTo { .. }) =>
            {
                ("stack_obj".to_owned(), get, tail, 1)
            }
            [save, move_to, get, restore, tail @ ..] if self.next_id == 0 => {
                let saved = self_save_local(save)?;
                let target = self.distance_handle(assigned_to(move_to, 10, 0)?)?;
                if target == "stack_obj" || !is_operand(assigned_to(restore, 10, 0)?, 25, saved) {
                    return None;
                }
                let needle = format!("local[{saved}]");
                if self
                    .tree
                    .lifted
                    .iter()
                    .map(|l| l.matches(&needle).count())
                    .sum::<usize>()
                    != 2
                {
                    return None;
                }
                (target, get, tail, 4)
            }
            _ => return None,
        };
        let Stmt::Primitive {
            prim,
            branches: None,
            ..
        } = get
        else {
            return None;
        };
        let PrimitiveCall::GetDistanceTo { temp, flags, from } = unpadded(prim) else {
            return None;
        };
        let from = match (flags, from) {
            (0, None) => String::new(),
            (1, Some(from)) => {
                if used > 1
                    && matches!(from, OperandRef::Direct(o) if matches!(o.scope, 4 | 10 | 16))
                {
                    // With a move, v2 reads a stack-relative 'from' before changing stack_obj.
                    return None;
                }
                format!(", from: {}", self.distance_handle(from)?)
            }
            _ => return None,
        };
        let temp_arg = if *temp == 0 {
            String::new()
        } else {
            format!(", temp: {temp}")
        };
        let call = format!("distance_to({target}{from}{temp_arg})");
        if let [
            Stmt::Assign {
                lhs,
                op: AssignOp::Set,
                rhs,
                ..
            },
            ..,
        ] = tail
            && is_operand(rhs, 8, i16::from(*temp))
            && let Some(lhs) = self.lvalue(lhs)
        {
            return Some((used + 1, format!("{lhs} = {call};")));
        }
        // Lowering can eliminate the resulting temp-to-itself store before an Expression.
        Some((used, format!("temp[{temp}] = {call};")))
    }

    fn distance_handle(&self, operand: &OperandRef) -> Option<String> {
        let OperandRef::Direct(o) = operand else {
            return None;
        };
        match (o.scope, o.index, self.sim) {
            (10, 0, _) => Some("stack_obj".to_owned()),
            (3, 11, true) => Some("sim".to_owned()),
            (3, 11, false) => Some("self.id".to_owned()),
            (4, 11, true) => Some("self.id".to_owned()),
            (3, 2, true) => Some("sim.container".to_owned()),
            (16, 0..=255, true) | (20, 0..=255, false) => self.rvalue(operand),
            _ => None,
        }
    }

    /// Two-statement idioms, each the exact lowering of its v2 spelling. Returns how many
    /// statements it consumed.
    ///
    /// - `local[0] := <object>; push_interaction(.., object: 0, flags: f | 2)` as
    ///   `push_interaction(.., object: <object>, flags: f)`;
    /// - in a Sim context, `temp[0] := i; scope(27, 0) := v` as
    ///   `set_dynamic_sprite(index: i, value: v)`.
    fn pair(&mut self, rest: &[Stmt], out: &mut Out) -> Option<usize> {
        let [first, second, ..] = rest else {
            return None;
        };
        let id = self.next_id;
        if self.forced_raw(id) || self.forced_raw(id + 1) {
            return None;
        }
        let set = assigned_to;
        let text = if let Some(object) = set(first, 25, 0)
            && let Stmt::Primitive {
                prim:
                    PrimitiveCall::PushInteraction {
                        menu,
                        object_slot: 0,
                        priority,
                        flags,
                        icon_local: LocalRef::Index(0),
                    },
                branches: None,
                ..
            } = second
            && flags & 0x03 == 0x02
        {
            let object = match object {
                OperandRef::Direct(Operand {
                    scope: 3,
                    index: 11,
                }) if self.sim => "sim",
                OperandRef::Direct(Operand {
                    scope: 3,
                    index: 11,
                }) => "self",
                OperandRef::Direct(Operand {
                    scope: 10,
                    index: 0,
                }) if self.sim => "self",
                OperandRef::Direct(Operand {
                    scope: 10,
                    index: 0,
                }) => "stack_obj",
                _ => return None,
            };
            format!(
                "push_interaction(menu: {menu}, object: {object}, priority: {priority}, flags: {});",
                flags & !0x02
            )
        } else if self.sim
            && let Some(index) = set(first, 8, 0)
            && let Some(value) = set(second, 27, 0)
        {
            let index = self.rvalue(index)?;
            let value = self.rvalue(value)?;
            if index.contains("temp[0]") || value.contains("temp[0]") {
                return None;
            }
            format!("set_dynamic_sprite(index: {index}, value: {value});")
        } else if self.cursor.is_none()
            && let Some(index) = set(first, 8, 0)
            && let Stmt::Assign { lhs, op, rhs, .. } = second
        {
            let index = self.rvalue(index)?;
            let sprite = format!("stack_obj.sprite_flags[{index}]");
            let op = assign_operator(op)?;
            let (dest, source) = if is_operand(lhs, 27, 0) {
                let source = self.rvalue(rhs)?;
                // The source is read after temp[0] is set. The compiler would
                // snapshot a temp[0] operand before setting it.
                if is_operand(rhs, 8, 0) {
                    return None;
                }
                (sprite, source)
            } else if is_operand(rhs, 27, 0) {
                (self.lvalue(lhs)?, sprite)
            } else {
                return None;
            };
            format!("{dest} {op} {source};")
        } else {
            return None;
        };
        if !self.extra() {
            return None;
        }
        self.next_id += 2;
        self.flush_raw(out);
        let start = out.lines.len();
        out.line(text);
        self.record(id, start, out);
        Some(2)
    }

    /// `local[a] := stack_obj; local[v] := 0; while set_to_next(person|all, dest: local[v])
    /// { .. } stack_obj := local[a];` as `for p in lot.sims() { .. }` (or `lot.objects()`).
    /// Without its own save, the loop restores from the tree's hoisted `local[a]` save
    /// (see [`hoisted_self_save`]). Returns how many statements it consumed.
    fn for_each_loop(&mut self, rest: &[Stmt], out: &mut Out) -> Option<usize> {
        // (statements before the body, whether a restore follows it, mode, body)
        let with_save = |save: &Stmt, init, w, restore| {
            let p = for_each_parts(init, w, restore)?;
            (self_save_local(save) == Some(p.saved)).then_some((
                3,
                true,
                p.mode,
                p.stack_cursor,
                p.body,
            ))
        };
        let hoisted = |init, w, restore| {
            let p = for_each_parts(init, w, restore)?;
            (self.hoisted_save == Some(p.saved)).then_some((
                2,
                true,
                p.mode,
                p.stack_cursor,
                p.body,
            ))
        };
        // v2's optimizer drops a save and restore nothing reads (a restore just before
        // `return`), leaving the bare loop.
        let bare = |init, w| for_each_core(init, w, None).map(|(m, c, b)| (2, false, m, c, b));
        let (head, restores, mode, stack_cursor, body) = match rest {
            [save, init, w, restore, ..] if self_save_local(save).is_some() => {
                with_save(save, init, w, restore)
            }
            [init, w, restore, ..] => hoisted(init, w, restore).or_else(|| bare(init, w)),
            [init, w, ..] => bare(init, w),
            _ => None,
        }?;
        // The zero-local form is v2 PR F's: a tree that does not prove keeps it raw.
        if stack_cursor && !self.extra() {
            return None;
        }
        let used = head + usize::from(restores);
        let id = self.next_id;
        // The save (if any), the init and the `while` come first in pre-order; the restore
        // follows the loop body.
        let restore = restores.then_some(id + head + count_stmts(body));
        if (id..id + head).chain(restore).any(|i| self.forced_raw(i)) {
            return None;
        }
        self.next_id += head;
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
        let outer_sims = std::mem::replace(
            &mut self.sims_loop,
            (mode == 1).then(|| format!("{var_name}{suffix}")),
        );
        let outer_cursor = std::mem::replace(
            &mut self.cursor,
            stack_cursor.then(|| format!("{var_name}{suffix}")),
        );
        self.block(body, out);
        self.cursor = outer_cursor;
        self.sims_loop = outer_sims;
        self.loop_depth -= 1;
        self.flush_raw(out);
        out.indent -= 1;
        out.line("}");
        if restores {
            self.next_id += 1;
        }
        self.record(id, start, out);
        Some(used)
    }

    /// `while !animate(...)` whose events can be exposed through a v2 handler.
    fn animate_loop(&self, cond: &Condition, empty_body: bool) -> Option<String> {
        let Condition::Not(inner) = cond else {
            return None;
        };
        let Condition::Primitive { prim, .. } = inner.as_ref() else {
            return None;
        };
        if !empty_body && matches!(prim, PrimitiveCall::Animate { flags, .. } if flags & 0x20 != 0)
        {
            return None;
        }
        self.animate(prim)
    }

    fn animate(&self, prim: &PrimitiveCall) -> Option<String> {
        let PrimitiveCall::Animate {
            clip_id,
            table,
            flags,
            events,
            local_index: LocalRef::Index(local),
        } = prim
        else {
            return None;
        };
        let flags_without_local = flags & !0x20;
        let mut text = format!(
            "animate(clip: {clip_id}, table: {table}, flags: {flags_without_local}, events: {events}"
        );
        if flags & 0x20 != 0 {
            text.push_str(&format!(", local: {}", self.local_name(u16::from(*local))?));
        } else if *local != 0 && !self.extra() {
            return None;
        }
        text.push(')');
        Some(text)
    }

    fn simple(&self, s: &Stmt) -> Option<String> {
        match s {
            Stmt::Assign { lhs, op, rhs, .. } => {
                if let Some(lhs) = self.flags_place(lhs)
                    && let Some(rhs) = self.flags_place(rhs)
                    && matches!(
                        op,
                        AssignOp::Set | AssignOp::BitAnd | AssignOp::BitOr | AssignOp::BitXor
                    )
                    && self.extra()
                {
                    return Some(format!("{lhs} {} {rhs};", assign_operator(op)?));
                }
                let op = match op {
                    AssignOp::FlagSet | AssignOp::FlagClear
                        if self.flags_place(lhs).is_some() && self.extra() =>
                    {
                        if *op == AssignOp::FlagSet {
                            "flag_set"
                        } else {
                            "flag_clear"
                        }
                    }
                    _ => assign_operator(op)?,
                };
                Some(format!("{} {op} {};", self.lvalue(lhs)?, self.rvalue(rhs)?))
            }
            Stmt::Return {
                target: Target::ReturnTrue,
                ..
            } => Some("return true;".to_owned()),
            // An animation event handler is a closure: `return` there is v2's to place.
            Stmt::Return {
                target: Target::Error,
                ..
            } if !self.in_handler => Some("return error;".to_owned()),
            Stmt::Return {
                target: Target::ReturnFalse,
                ..
            } => Some("return false;".to_owned()),
            Stmt::Break { .. } if self.loop_depth > 0 => Some("break;".to_owned()),
            Stmt::Continue { .. } if self.loop_depth > 0 => Some("continue;".to_owned()),
            Stmt::Call {
                target: CallTarget::Id(id),
                args,
                pass_temps,
                branches: None,
                ..
            } => {
                let call = self.call(*id, args, *pass_temps)?;
                if *pass_temps {
                    Some(format!("{call};"))
                } else {
                    // A standalone AIR call exits false on failure.
                    Some(format!("if !{call} {{ return false; }}"))
                }
            }
            Stmt::Primitive {
                prim,
                branches: None,
                ..
            } => self.prim_stmt(prim).map(|p| format!("{p};")),
            _ => None,
        }
    }

    fn call(&self, id: u16, args: &[i16], pass_temps: bool) -> Option<String> {
        let mut params = Vec::new();
        let name = if PRIVATE_TREES.contains(&id) {
            let callee = self.model.tree(id)?;
            let standalone = callee.lifecycle.is_none() || self.plan.standalone.contains(&id);
            let callee_sim = self.plan.sim(callee);
            let callee_inherit = self.plan.inherits(callee);
            // Either context calls an inherit tree; an inherit tree, lowered in both, calls
            // no other context's tree.
            let context_ok = callee_inherit || (!self.inherit && callee_sim == self.sim);
            if callee.omitted.is_some() || !standalone || !context_ok {
                return None;
            }
            // A Sim tree's calls name the Sim first, argless ones too (`f(sim)` is
            // `call ID(temps)`).
            if self.sim && !callee_inherit {
                params.push("sim".to_owned());
            }
            callee.ident.clone()
        } else {
            let ident = self.model.extern_calls.get(&id)?;
            let module = if id >= 8192 {
                "semi_globals"
            } else {
                "globals"
            };
            format!("{module}::{ident}")
        };
        if pass_temps {
            if self.model.literal_calls.contains_key(&id) {
                return None;
            }
        } else {
            let arity = *self.model.literal_calls.get(&id)?;
            let args = args.get(..arity)?;
            if !self.extra() {
                return None;
            }
            params.extend(args.iter().map(i16::to_string));
        }
        Some(format!("{name}({})", params.join(", ")))
    }

    fn cond(&self, c: &Condition) -> Option<String> {
        match c {
            Condition::Compare { lhs, op, rhs, .. } => {
                if matches!(op, CompareOp::Eq | CompareOp::Ne)
                    && is_operand(rhs, 7, 0)
                    && let Some(flags) = self.flags_place(lhs)
                    && self.extra()
                {
                    let not = if *op == CompareOp::Ne { "!" } else { "" };
                    return Some(format!("{not}{flags}.is_empty()"));
                }
                let op = match op {
                    CompareOp::Gt => ">",
                    CompareOp::Lt => "<",
                    CompareOp::Eq => "==",
                    CompareOp::Ge => ">=",
                    CompareOp::Le => "<=",
                    CompareOp::Ne => "!=",
                    CompareOp::IncLt | CompareOp::DecGt => {
                        let name = if *op == CompareOp::IncLt {
                            "inc_lt"
                        } else {
                            "dec_gt"
                        };
                        let (place, bound) = (self.lvalue(lhs)?, self.rvalue(rhs)?);
                        return self
                            .extra()
                            .then(|| format!("{name}(&mut {place}, {bound})"));
                    }
                    CompareOp::FlagTest if self.flags_place(lhs).is_some() && self.extra() => {
                        "flag_test"
                    }
                    CompareOp::FlagTest => return None,
                };
                Some(format!("{} {op} {}", self.rvalue(lhs)?, self.rvalue(rhs)?))
            }
            Condition::Call {
                target: CallTarget::Id(id),
                args,
                pass_temps,
                ..
            } => self.call(*id, args, *pass_temps),
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
            PrimitiveCall::PlaySound { sound_id, flags } => self.play_sound(*sound_id, *flags),
            PrimitiveCall::Padded { call, pad } => match call.as_ref() {
                PrimitiveCall::PlaySound { sound_id, flags } => {
                    let [lo, hi] = sound_id.to_le_bytes();
                    let operands =
                        apply_operand_pad(23, [lo, hi, 0, 0, *flags, 0, 0, 0], pad).ok()?;
                    self.play_sound(*sound_id, operands[4])
                }
                PrimitiveCall::StopAllSounds { .. } if self.extra() => self.prim_stmt(call),
                _ => self.builtin(p),
            },
            PrimitiveCall::StopAllSounds { use_stack_object } => {
                let target = self.sound_target(*use_stack_object);
                if !self.sim && *use_stack_object && !self.extra() {
                    return None;
                }
                Some(format!("stop_all_sounds({target})"))
            }
            PrimitiveCall::RemoveObject { target, flags } => {
                let mine = if self.sim { 1 } else { 0 };
                if *target != mine {
                    return None;
                }
                Some(match flags {
                    0 => "destroy_self()".to_owned(),
                    1 if *target == 0 && self.extra() => "destroy_self(no_yield: true)".to_owned(),
                    _ => format!("destroy_self(flags: {flags})"),
                })
            }
            other => self.builtin(other),
        }
    }

    /// The empty spelling selects the running object in an object-context tree and the
    /// stack object in a Sim-context tree.
    fn sound_target(&self, stack: bool) -> &'static str {
        match (self.sim, stack) {
            (false, false) | (true, true) => "",
            (true, false) => "sim",
            (false, true) => "stack_obj",
        }
    }

    fn play_sound(&self, sound_id: u16, flags: u8) -> Option<String> {
        // v2 cannot emit 0xff, whose target bit the VM ignores.
        if flags == 0xff || !self.extra() {
            return None;
        }
        let sound = self
            .model
            .sounds
            .get(&sound_id)
            .map_or_else(|| sound_id.to_string(), |sound| sound.ident.clone());
        let target = self.sound_target(flags & 2 != 0);
        let mut args = vec![sound];
        if !target.is_empty() {
            args.push(target.to_owned());
        }
        let flags = flags & !2;
        if flags != 0 {
            args.push(format!("flags: {flags}"));
        }
        Some(format!("play_sound({})", args.join(", ")))
    }

    /// Builtins that are the same primitive as a statement or a condition.
    fn builtin(&self, p: &PrimitiveCall) -> Option<String> {
        Some(match p {
            PrimitiveCall::FindBestInteraction => "find_best_interaction()".to_owned(),
            PrimitiveCall::Grab => "grab()".to_owned(),
            PrimitiveCall::Drop => "drop()".to_owned(),
            PrimitiveCall::GosubFoundAction => "gosub_found_action()".to_owned(),
            PrimitiveCall::NotifyIdle => "notify_out_of_idle()".to_owned(),
            // The spelling v2 writes for `notify_out_of_idle()`.
            PrimitiveCall::Padded {
                call,
                pad: crate::ast::OperandPad::Fill(0xff),
            } if **call == PrimitiveCall::NotifyIdle && self.extra() => {
                "notify_out_of_idle()".to_owned()
            }
            // v2 restores `stack_obj` after the call (and, as a condition, carries the
            // result through a local).
            PrimitiveCall::RunFunctionalTree {
                function,
                change_icon,
            } if *function <= 255 && self.extra() => {
                let icon = if *change_icon {
                    ", change_icon: true"
                } else {
                    ""
                };
                format!("stack_obj.run_functional({function}{icon})")
            }
            PrimitiveCall::SetToNext { .. } => return self.set_to_next(p),
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
            PrimitiveCall::Padded { call, .. }
                if matches!(call.as_ref(), PrimitiveCall::ActionString { .. }) && self.extra() =>
            {
                return self.builtin(call);
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
            _ => self.pilot_builtin(p)?,
        })
    }

    fn pilot_builtin(&self, p: &PrimitiveCall) -> Option<String> {
        let text = match unpadded(p) {
            PrimitiveCall::ChangeSuit {
                index,
                scope,
                remove,
            } => {
                let remove = if *remove { ", remove: true" } else { "" };
                format!("change_suit(index: {index}, scope: {scope}{remove})")
            }
            PrimitiveCall::Burn { mode, flags } => format!("burn(mode: {mode}, flags: {flags})"),
            PrimitiveCall::LookTowards { mode } => format!("look_towards(mode: {mode})"),
            PrimitiveCall::DropOnto {
                src: SlotSpec::Literal(src),
                dst: SlotSpec::Literal(dst),
            } => format!("drop_onto(src: {src}, dst: {dst})"),
            PrimitiveCall::FindLocation {
                mode,
                flags,
                local: LocalRef::Index(local),
            } => {
                let local = if flags & 1 != 0 || *local != 0 {
                    format!(", local: {}", self.local_name(u16::from(*local))?)
                } else {
                    String::new()
                };
                format!("find_location(mode: {mode}, flags: {flags}{local})")
            }
            PrimitiveCall::Relationship {
                column,
                direction,
                flags,
                local: LocalRef::Index(local),
                value,
            } => {
                let value = self.local_name(u16::try_from(direct_local(value)?).ok()?)?;
                let local = if matches!(direction, 2 | 3) || *local != 0 {
                    format!(", local: {}", self.local_name(u16::from(*local))?)
                } else {
                    String::new()
                };
                format!(
                    "relationship(column: {column}, dir: {direction}, flags: {flags}{local}, value: {value})"
                )
            }
            PrimitiveCall::OldRelationship {
                write,
                column,
                param,
                direction,
                flags,
            } => format!(
                "old_relationship(write: {write}, column: {column}, param: {param}, dir: {direction}, flags: {flags})"
            ),
            PrimitiveCall::GenericCall { routine } => format!("generic_call({routine})"),
            PrimitiveCall::RouteTo {
                slot,
                source,
                flags,
            } if *source <= 2 => {
                Self::slot_builtin("route_to", *slot, *flags, "source", *source, 1)
            }
            PrimitiveCall::Snap { slot, mode, flags } if *mode <= 4 => {
                Self::slot_builtin("snap", *slot, *flags, "mode", *mode, 3)
            }
            _ => return None,
        };
        self.extra().then_some(text)
    }

    /// v2 PR H's in-place step, `set_to_next(mode, guid: K, dest: stack_obj)`: one
    /// primitive that moves the stack object (or, without `dest`, the VM's default cursor).
    /// Not inside a loop whose cursor is the stack object, which the step would move.
    fn set_to_next(&self, p: &PrimitiveCall) -> Option<String> {
        let PrimitiveCall::SetToNext {
            mode,
            guid,
            dest,
            local,
        } = p
        else {
            return None;
        };
        if self.cursor.is_some() || !self.extra() {
            return None;
        }
        // `.air` writes the destination flag (mode bit 7) as the `dest:` operand.
        let dest = match dest {
            None => "",
            Some(d) if is_operand(d, 10, 0) => ", dest: stack_obj",
            Some(_) => return None,
        };
        let base = mode & 0x7F;
        let word = match base {
            0 => "all",
            1 => "person",
            2 => "non_person",
            3 => "part",
            4 => "guid",
            5 => "neighbor",
            6 => "category",
            7 => "neighbor_guid",
            8 => "on_tile",
            9 => "beside",
            10 => "career",
            12 => "family",
            _ => return None,
        };
        let local = match (base, local) {
            (9, LocalRef::Index(i)) => format!(", local: {}", self.local_name(u16::from(*i))?),
            (_, LocalRef::Index(0)) => String::new(),
            _ => return None,
        };
        let guid = if *guid == 0 {
            String::new()
        } else {
            format!(", {}", self.guid(*guid))
        };
        Some(format!("set_to_next({word}{guid}{dest}{local})"))
    }

    fn slot_builtin(
        prim: &str,
        slot: u16,
        flags: u8,
        key: &str,
        value: u16,
        default: u16,
    ) -> String {
        let flags = if flags == 0 {
            String::new()
        } else {
            format!(", flags: {flags}")
        };
        let extra = if value == default {
            String::new()
        } else {
            format!(", {key}: {value}")
        };
        format!("{prim}({slot}{flags}{extra})")
    }

    fn local_name(&self, slot: u16) -> Option<String> {
        (slot < self.named_locals && self.extra()).then(|| format!("slot_{slot}"))
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
                    let bcon = self.model.bcons.iter().find(|b| b.id == 4096 + table)?;
                    let (field, _) = bcon.entries.get(usize::from(entry))?;
                    Some(format!("{}.{field}", bcon.label))
                } else {
                    let name = self.model.extern_bcons.get(&addr)?;
                    Some(format!("library_consts.{name}"))
                }
            }
            10 if o.index == 0 && self.cursor.is_some() => self.cursor.clone(),
            9 if o.index == 0 && self.in_handler => Some("event".to_owned()),
            20 if !self.sim && !self.inherit && o.index >= 0 && self.extra() => {
                Some(format!("self.slot[{}]", o.index))
            }
            16 if self.sim && o.index >= 0 && self.extra() => {
                Some(format!("self.slot[{}]", o.index))
            }
            3 if self.sim && o.index == 2 && self.extra() => Some("sim.container".to_owned()),
            19 if o.index == 65 => {
                let p = self.person_binding()?;
                Some(format!("{p}.kind"))
            }
            _ => self.lvalue(r),
        }
    }

    /// The `for p in lot.sims()` binding that `stack.person[..]` reads through. v2 points
    /// `stack_obj` at `p` around each access and restores it after.
    fn person_binding(&self) -> Option<&str> {
        let p = self.sims_loop.as_deref()?;
        self.extra().then_some(p)
    }

    fn personality(&self, handle: &str, index: i16) -> Option<String> {
        let trait_name = match index {
            2 => "nice",
            3 => "active",
            5 => "playful",
            6 => "outgoing",
            7 => "neat",
            _ => return None,
        };
        self.extra()
            .then(|| format!("{handle}.personality.{trait_name}"))
    }

    /// Numeric uses of the same word keep the `.data[8]` spelling: `.flags` is typed.
    fn flags_place(&self, r: &OperandRef) -> Option<String> {
        let handle = if is_operand(r, 4, 8) {
            self.cursor.as_deref().unwrap_or("stack_obj")
        } else if !self.sim && !self.inherit && is_operand(r, 3, 8) {
            "self"
        } else {
            return None;
        };
        Some(format!("{handle}.flags"))
    }

    fn lvalue(&self, r: &OperandRef) -> Option<String> {
        let OperandRef::Direct(o) = r else {
            return None;
        };
        let i = o.index;
        if i < 0 {
            return None;
        }
        if let Some(c) = &self.cursor {
            // Inside a `stack_obj`-cursor loop the stack object is the binding, and
            // `stack_obj` itself may not be written.
            match o.scope {
                1 => return Some(format!("{c}.attr[{i}]")),
                4 => return Some(format!("{c}.data[{i}]")),
                16 => return Some(format!("{c}.slot[{i}]")),
                19 if i == 65 => return Some(format!("{c}.kind")),
                19 => {
                    return self
                        .personality(c, i)
                        .or_else(|| Some(format!("{c}.person[{i}]")));
                }
                10 | 13 | 15 | 27 => return None,
                _ => {}
            }
        }
        if self.inherit {
            let cell = match o.scope {
                0 => Some(("me", "attr")),
                3 => Some(("me", "data")),
                14 => Some(("me", "motive")),
                18 => Some(("me", "person")),
                20 => Some(("me", "slot")),
                1 => Some(("stack_obj", "attr")),
                4 => Some(("stack_obj", "data")),
                16 => Some(("stack_obj", "slot")),
                19 if self.person_binding().is_none() => Some(("stack_obj", "person")),
                _ => None,
            };
            if let Some((handle, field)) = cell {
                return (i <= 255).then(|| format!("{handle}.{field}[{i}]"));
            }
        }
        Some(match (o.scope, self.sim) {
            (8, _) => format!("temp[{i}]"),
            (6, _) => format!("global[{i}]"),
            (10, _) if i == 0 => "stack_obj".to_owned(),
            (0, false) | (1, true) if i <= self.model.state_limit => format!("self.attr_{i}"),
            (3, false) => format!("self.data[{i}]"),
            (0, true) if i <= 255 && self.extra() => format!("sim.attr[{i}]"),
            (3, true) if i <= 255 && self.extra() => format!("sim.data[{i}]"),
            (4, _) if i == 8 && self.extra() => "stack_obj.data[8]".to_owned(),
            (14, true) => format!("sim.motive[{i}]"),
            (18, true) => self
                .personality("sim", i)
                .unwrap_or_else(|| format!("sim.person[{i}]")),
            (9, _) if !(self.in_handler && i == 0) && self.extra() => format!("param[{i}]"),
            (19, _) => {
                let binding = self.person_binding();
                self.personality(binding.unwrap_or("stack_obj"), i)
                    .or_else(|| binding.map(|p| format!("{p}.person[{i}]")))?
            }
            (25, _) => self.local_name(i as u16)?,
            _ => return None,
        })
    }
}

fn assign_operator(op: &AssignOp) -> Option<&'static str> {
    Some(match op {
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
    })
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
        assert_eq!(library_bcon_name(192 << 7), None);
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
        // An earlier call's arguments can run across later nodes into the tail too:
        // 8 operand bytes plus one 12-byte node hold 10 arguments, not 11.
        let ctx = arities(&[(4210, 10), (4211, 11)]);
        let reaches = |callee| tree(&[(callee, 1, 0xfd, [0; 8]), (2, 0xfe, 0xfd, [0; 8])]);
        let near = reaches(4210);
        assert!(same_but_unreachable_tail(&near, &with_tail(&near), &ctx));
        let far = reaches(4211);
        assert!(!same_but_unreachable_tail(&far, &with_tail(&far), &ctx));
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
