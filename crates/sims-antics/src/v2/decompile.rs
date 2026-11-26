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
    LiftOptions, StructuredLiftOutcome, lift_bhav_structured_in_node_order,
    lift_bhav_structured_with_arities,
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
/// The Run Tree By Name primitive, and the string table its names are in.
const RUN_TREE_BY_NAME: u16 = 28;
const TREE_NAMES_TABLE: u16 = 303;
/// Primitives that look a tree up by a fixed name (the other `sims-vm`
/// `Program::named_tree` callers): Create Object Instance runs `init visitor`, Go To
/// Relative Position and Go To Routing Slot (one routing state machine) ask or wait by
/// name and run `Route failure feedback`, and Reach runs `CT - Grab`/`CT - Put`. Which
/// object's file is searched first varies, so the names are checked in this file
/// whichever it is.
const FIXED_NAME_LOOKUPS: [(u16, &[&str]); 4] = [
    // Only with `FROM_RECORD` (byte 5 bit 2): see `fixed_names`.
    (42, &["init visitor"]),
    (
        27,
        &[
            "route ask to move animation",
            "route waiting animation",
            "Route failure feedback",
        ],
    ),
    (
        45,
        &[
            "route ask to move animation",
            "route waiting animation",
            "Route failure feedback",
        ],
    ),
    (47, &["CT - Grab", "CT - Put"]),
];
/// Create Object Instance's operand byte and flag under which it runs `init visitor`.
const CREATE_OBJECT: u16 = 42;
const CREATE_FROM_RECORD: u8 = 0x04;
/// Run Functional Tree: the stack object runs the callback a function names.
const RUN_FUNCTIONAL_TREE: u16 = 20;
/// Primitives whose VM work may run object callbacks (`sims-vm` `.callback(`): of the
/// object they move, of its container, or of the target of a queued action they
/// cancel. Grab (4), Drop (5), Burn (9), Push Interaction (13), Find Location For (16),
/// Idle For Input (17), Remove Object Instance (18), Go To Relative Position (27), UI
/// Effect (34), Create Object Instance (42), Drop Onto (43), Go To Routing Slot (45),
/// Snap (46) and Reach (47); Dialog (36) too when it builds a portrait person
/// ([`runs_callbacks`]).
const CALLBACK_PRIMITIVES: [u16; 14] = [4, 5, 9, 13, 16, 17, 18, 27, 34, 42, 43, 45, 46, 47];
const DIALOG: u16 = 36;
/// Expression, and the operand scopes through which it reads object definitions
/// (`sims-vm` `memory.rs`): the stack object's words (21), the type-attribute record
/// that definition words 92 and 93 of me (36) or the stack object (37) may name, and
/// a neighbor's linked definition (38); and whether the stack object's callback table
/// names a tree (35).
const EXPRESSION: u16 = 2;
const DEFINITION_SCOPES: [u8; 4] = [21, 36, 37, 38];
const CALLBACK_SCOPE: u8 = 35;
/// When an OBJD has no OBJf table, the VM reads its callbacks from these OBJD byte
/// offsets, in callback order (`sims-vm` `program.rs` `DEFINITION_CALLBACKS`).
const DEFINITION_CALLBACKS: [usize; 30] = [
    0x56, 0x0a, 0x5e, 0x68, 0x78, 0x2a, 0x7c, 0x64, 0x9c, 0x58, 0x94, 0x60, 0x5a, 0x6a, 0x6e, 0x22,
    0x0c, 0x18, 0x30, 0x32, 0x34, 0x36, 0x38, 0x3a, 0x3c, 0x3e, 0x40, 0x42, 0x76, 0xa0,
];
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
    /// Check trees under the behavioral relation (`lift --v2 --behavioral`, the relation of
    /// `verify-equiv --behavioral`, `docs/decisions/20261123-verifier-behavioral-mode.md`)
    /// instead of the strict one (default `false`). Node numbers and frame storage are
    /// then not compared, so a proof holds only for runs that start and end under one
    /// build. Every other check (headers, sounds, strings, object gaps, unproven callees)
    /// is unchanged. The source says so in its header.
    pub behavioral: bool,
}

impl Default for DecompileV2Options {
    fn default() -> Self {
        Self {
            library_arities: BTreeMap::new(),
            library_names: BTreeMap::new(),
            validate: true,
            behavioral: false,
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
    /// An `init`/`main` tree written as a plain `#[bhav] fn`, because an `init`/`main` block
    /// (which takes no `#[frame]`) did not reproduce it. The compiler then adds its own
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
    // Every round lowers at least one tree or statement (or swaps a tree's branch order, at
    // most once per tree and level), so this bound is never reached by
    // a run that converges; it only guards against a bug turning into a hang.
    let max_rounds = 8 + model.trees.len() * 8 + model.statement_count() * 2;
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
                let unblamed = tree.is_none()
                    && (plan.drop_all_wave3(&generated)
                        || plan.drop_all_wave2(&generated)
                        || plan.drop_all_extras(&generated)
                        || plan.number_all_raw_bcons(&generated));
                if !unblamed
                    && !tree.is_some_and(|id| plan.add_frame(&model, id))
                    && !tree.is_some_and(|id| plan.keep_temps_raw(id, &generated))
                    && !tree.is_some_and(|id| plan.unmerge(id, &generated))
                    && !tree.is_some_and(|id| plan.drop_wave3(id, &generated))
                    && !tree.is_some_and(|id| plan.drop_wave2(id, &generated))
                    && !tree.is_some_and(|id| plan.name_builtin_locals_only(id, &generated))
                    && !tree.is_some_and(|id| plan.drop_inherit(&model, id))
                    && !tree.is_some_and(|id| plan.drop_extras(id, &generated))
                    && !tree.is_some_and(|id| plan.number_raw_bcons(id, &generated))
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
                        let handled = plan.air_fallback(&model, &generated, id, &message)
                            || model.omit(id, &message);
                        if handled && message.contains(LOCALS_SHORT) {
                            // The build stops at the first tree short of locals: one build with
                            // the counts left out finds every other one, so each takes its
                            // next step now instead of a round of its own. One left out
                            // (`omit`) still is, in its own round, with its own message.
                            for (other, (declared, needed)) in model.locals_needed(&generated.text)
                            {
                                if other != id && needed > declared {
                                    let message =
                                        format!("tree declares {declared}{LOCALS_SHORT}{needed}");
                                    plan.air_fallback(&model, &generated, other, &message);
                                }
                            }
                        }
                        handled
                    }
                    None => {
                        plan.drop_all_wave3(&generated)
                            || plan.drop_all_wave2(&generated)
                            || plan.drop_all_extras(&generated)
                            || plan.number_all_raw_bcons(&generated)
                            || plan.lower_all(&model)
                    }
                };
                if !handled {
                    return Err(no_compile(&message));
                }
            }
            Ok(recompiled) => {
                let (checks, unproven) = model.check_all(&recompiled, &mut cache);
                // A block given its header may be all its callers were missing (a caller is
                // proven only with its callees), so such a round lowers nothing else.
                let mut changed = false;
                for &id in &unproven {
                    changed |= plan.add_frame(&model, id);
                }
                if !changed {
                    for &id in &unproven {
                        changed |= plan.keep_temps_raw(id, &generated)
                            || plan.fix_locals(id)
                            || plan.unmerge(id, &generated)
                            || plan.drop_wave3(id, &generated)
                            || plan.drop_wave2(id, &generated)
                            || plan.name_builtin_locals_only(id, &generated)
                            || plan.drop_inherit(&model, id)
                            || plan.drop_extras(id, &generated)
                            || plan.other_order(&mut model, id)
                            || plan.lower_tree(&model, id)
                            || plan.number_raw_bcons(id, &generated);
                    }
                }
                if changed {
                    continue;
                }
                let (text, recompiled, checks) = model.type_engine_fields(
                    &generated.text,
                    (recompiled, checks, &unproven),
                    &mut cache,
                );
                let text = model.lint_spellings(text, &recompiled);
                let text = model.polish(text, &recompiled);
                let (text, recompiled, mut checks) =
                    model.flip_negations(text, (recompiled, checks), &mut cache);
                let text = model.dot_bcon_reads_the_lint_misses(text, &recompiled);
                let text = model.allow_kept_negations(text);
                let source = format_v2_source(&text)?;
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

/// Lint `untyped_engine_field`'s suggestion: `(line, column, typed name)`, 1-based, for the
/// cell spelled from the column through its index's `]` (`global[3]`, `sim.slot[0]`).
type EngineFieldFix = (usize, usize, String);

/// Every engine cell spelled by number that has a typed name, by lint
/// `untyped_engine_field`'s own table and rules. Empty when the text does not lint.
fn engine_field_fixes(text: &str) -> Vec<EngineFieldFix> {
    let Ok(diags) = crate::v2::lint::lint_v2_source(text, "decompiled.antics") else {
        return Vec::new();
    };
    diags
        .into_iter()
        .filter(|d| d.rule == crate::v2::lint::UNTYPED_ENGINE_FIELD)
        .filter_map(|d| Some((d.line, d.column, d.suggestion?)))
        .collect()
}

/// `text` with `fixes` applied. `None` when one does not fit the text.
fn apply_engine_field_fixes(text: &str, fixes: &[EngineFieldFix]) -> Option<String> {
    let mut sorted: Vec<&EngineFieldFix> = fixes.iter().collect();
    // Right to left, so earlier columns on a line stay put.
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    for (line, column, typed) in sorted {
        let l = lines.get_mut(line.checked_sub(1)?)?;
        let start = column.checked_sub(1)?;
        let end = start + l.get(start..)?.find(']')? + 1;
        l.replace_range(start..end, typed);
    }
    Some(lines.join("\n"))
}

/// `text` with an `allow(untyped_engine_field)` line above each line holding a cell of
/// `left` (whose typed name builds other nodes that are not proven the same).
fn allow_engine_fields(text: &str, left: &[EngineFieldFix]) -> String {
    let mut by_line: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for (line, _, typed) in left {
        let names = by_line.entry(*line).or_default();
        if !names.contains(&typed.as_str()) {
            names.push(typed);
        }
    }
    let mut out = String::with_capacity(text.len());
    for (n, l) in text.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        if let Some(names) = by_line.get(&(n + 1)) {
            let indent = &l[..l.len() - l.trim_start().len()];
            let names: Vec<String> = names.iter().map(|t| format!("`{t}`")).collect();
            out.push_str(&format!(
                "{indent}// antics-lint: allow(untyped_engine_field): {} builds other nodes, not proven the same here\n",
                names.join(", ")
            ));
        }
        out.push_str(l);
    }
    out
}

/// The fixes `ok` accepts, found by halving: a set `ok` refuses is split until each part is
/// accepted or is one fix (which is dropped).
fn keep_fixes(
    fixes: &[EngineFieldFix],
    ok: &dyn Fn(&[EngineFieldFix]) -> bool,
) -> Vec<EngineFieldFix> {
    if fixes.is_empty() || ok(fixes) {
        return fixes.to_vec();
    }
    if fixes.len() == 1 {
        return Vec::new();
    }
    let (a, b) = fixes.split_at(fixes.len() / 2);
    let mut kept = keep_fixes(a, ok);
    kept.extend(keep_fixes(b, ok));
    kept
}

/// A lint wave 3 suggestion (`untyped_engine_constant`, `prefer_method_syntax`,
/// `prefer_question_mark`): bytes `start..end` of the text become `text`, with the `use`
/// line it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LintSpelling {
    start: usize,
    end: usize,
    text: String,
    import: Option<String>,
}

/// The lint's suggestions for the three spelling rules in `text`, each with the source range
/// it replaces, read back from where the finding points. A finding whose range does not read
/// back as the rule's shape is left out. Empty when the text does not lint.
fn lint_spellings(text: &str) -> Vec<LintSpelling> {
    use crate::v2::lint::{PREFER_METHOD_SYNTAX, PREFER_QUESTION_MARK, UNTYPED_ENGINE_CONSTANT};
    let Ok(diags) = crate::v2::lint::lint_v2_source(text, "decompiled.antics") else {
        return Vec::new();
    };
    let mut line_starts = vec![0];
    line_starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let mut out = Vec::new();
    for d in diags {
        let Some(suggestion) = d.suggestion.clone() else {
            continue;
        };
        let Some(start) = d
            .line
            .checked_sub(1)
            .and_then(|l| line_starts.get(l))
            .and_then(|&l| Some(l + d.column.checked_sub(1)?))
            .filter(|&s| text.is_char_boundary(s))
        else {
            continue;
        };
        let rest = &text[start..];
        let end = match d.rule {
            UNTYPED_ENGINE_CONSTANT => {
                // The message quotes the literal: `3` is bit 3 of ..
                let quoted = d.message.split('`').nth(1).unwrap_or_default();
                let len = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                if quoted.is_empty()
                    || &rest[..len] != quoted
                    || !quoted.starts_with(|c: char| c.is_ascii_digit())
                {
                    continue;
                }
                start + len
            }
            PREFER_METHOD_SYNTAX => {
                let name = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                if name == 0 || !rest[name..].starts_with('(') {
                    continue;
                }
                let Some(end) = group_end(text, start + name) else {
                    continue;
                };
                end
            }
            PREFER_QUESTION_MARK => {
                let Some(end) = question_mark_end(text, start, &suggestion) else {
                    continue;
                };
                end
            }
            _ => continue,
        };
        // `.. and add `use std::flags::ObjectFlags;`; it builds ..`
        let import = d.help.as_deref().and_then(|h| {
            let (_, after) = h.split_once("add `")?;
            let line = after.split('`').next()?;
            line.starts_with("use std::").then(|| line.to_owned())
        });
        out.push(LintSpelling {
            start,
            end,
            text: suggestion,
            import,
        });
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    // Overlapping edits (a `refresh` constant inside the call's method form): the outer one
    // first; the inner one is found again on the next pass.
    let mut last_end = 0;
    out.retain(|f| {
        let keep = f.start >= last_end;
        if keep {
            last_end = f.end;
        }
        keep
    });
    out
}

/// The end of the `if` statement at `start` that lint `prefer_question_mark` rewrites to
/// `suggestion` (`f(..)?;`): `if !f(..) { return error; }`, or
/// `if f(..) {} else { return error; }`. `None` when the text is not that statement.
fn question_mark_end(text: &str, start: usize, suggestion: &str) -> Option<usize> {
    let call = suggestion.strip_suffix("?;")?;
    let rest = text.get(start..)?.strip_prefix("if ")?;
    let (negated, rest) = match rest.strip_prefix('!') {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let rest = rest.strip_prefix(call)?;
    let mut at = text.len() - rest.len();
    at += rest.len() - rest.trim_start().len();
    if !text[at..].starts_with('{') {
        return None;
    }
    let then_end = group_end(text, at)?;
    let then_body = text[at + 1..then_end - 1].trim();
    if negated {
        return (then_body == "return error;").then_some(then_end);
    }
    if !then_body.is_empty() {
        return None;
    }
    let after = &text[then_end..];
    let at = then_end + (after.len() - after.trim_start().len());
    let rest = text[at..].strip_prefix("else")?;
    let at = text.len() - rest.len();
    let at = at + (rest.len() - rest.trim_start().len());
    if !text[at..].starts_with('{') {
        return None;
    }
    let end = group_end(text, at)?;
    (text[at + 1..end - 1].trim() == "return error;").then_some(end)
}

/// The end of the bracketed group opening at `open` (`(`, `[` or `{`), past string literals.
/// `None` when it does not close, or a comment is inside.
fn group_end(text: &str, open: usize) -> Option<usize> {
    let b = text.as_bytes();
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += usize::from(b[i] == b'\\') + 1;
                }
            }
            b'/' if matches!(b.get(i + 1), Some(b'/' | b'*')) => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// `text` with `fixes` applied (right to left) and their `use` lines added at the top.
fn apply_lint_spellings(text: &str, fixes: &[LintSpelling]) -> String {
    let mut out = text.to_owned();
    let mut sorted: Vec<&LintSpelling> = fixes.iter().collect();
    sorted.sort_by_key(|f| std::cmp::Reverse(f.start));
    for f in sorted {
        out.replace_range(f.start..f.end, &f.text);
    }
    let imports: BTreeSet<&str> = fixes.iter().filter_map(|f| f.import.as_deref()).collect();
    if imports.is_empty() {
        return out;
    }
    insert_imports(&out, imports)
}

/// `text` with `imports` joined to its `use` block: the one after the header comments, made
/// when there is none. The block is sorted and holds each line once.
fn insert_imports(text: &str, imports: BTreeSet<&str>) -> String {
    let header: usize = text
        .split_inclusive('\n')
        .take_while(|l| l.starts_with("//"))
        .map(str::len)
        .sum();
    let rest = &text[header..];
    let block = header + rest.len() - rest.trim_start_matches('\n').len();
    let mut uses: BTreeSet<&str> = imports;
    let mut end = block;
    for line in text[block..].split_inclusive('\n') {
        let l = line.trim_end();
        if !(l.starts_with("use ") && l.ends_with(';')) {
            break;
        }
        uses.insert(l);
        end += line.len();
    }
    let mut out = text[..header].to_owned();
    if header > 0 {
        out.push('\n');
    }
    for line in uses {
        out.push_str(line);
        out.push('\n');
    }
    if !text[end..].starts_with('\n') {
        out.push('\n');
    }
    out.push_str(&text[end..]);
    out
}

/// The largest run of `fixes` found to pass `ok` together: each part joins what is kept only
/// if `ok` holds for both, and a part that does not is split in halves. At most `budget`
/// calls of `ok`.
fn keep_joint<T: Clone>(fixes: &[T], ok: &dyn Fn(&[T]) -> bool, budget: usize) -> Vec<T> {
    fn go<T: Clone>(kept: &mut Vec<T>, part: &[T], ok: &dyn Fn(&[T]) -> bool, budget: &mut usize) {
        if part.is_empty() || *budget == 0 {
            return;
        }
        *budget -= 1;
        let mut trial = kept.clone();
        trial.extend_from_slice(part);
        if ok(&trial) {
            *kept = trial;
            return;
        }
        if part.len() > 1 {
            let (a, b) = part.split_at(part.len() / 2);
            go(kept, a, ok, budget);
            go(kept, b, ok, budget);
        }
    }
    let mut kept = Vec::new();
    let mut budget = budget;
    go(&mut kept, fixes, ok, &mut budget);
    kept
}

/// The most rounds [`Model::flip_negations`] takes.
const NEGATION_ROUNDS: usize = 3;

/// The most builds [`Model::polish`] may take to find the tree names that keep the bytes.
const POLISH_NAME_BUILDS: usize = 8;

/// Names a tree is not given even when nothing in its file uses them, so the name it gets
/// reads as the tree alone: every identifier-like string the v2 lexer, parser and lowering
/// match on (keywords, builtin calls and their options, field names), `.air`'s reserved
/// words, and every callable and module `std` declares.
fn reserved_tree_names() -> &'static BTreeSet<&'static str> {
    const COMPILER: [&str; 27] = [
        include_str!("lexer.rs"),
        include_str!("parser.rs"),
        include_str!("lower/actions.rs"),
        include_str!("lower/air_text.rs"),
        include_str!("lower/ast_facts.rs"),
        include_str!("lower/builtins.rs"),
        include_str!("lower/callable.rs"),
        include_str!("lower/cond.rs"),
        include_str!("lower/consteval.rs"),
        include_str!("lower/dialog.rs"),
        include_str!("lower/env.rs"),
        include_str!("lower/expr.rs"),
        include_str!("lower/inline.rs"),
        include_str!("lower/lvalue.rs"),
        include_str!("lower/match_lower.rs"),
        include_str!("lower/matching.rs"),
        include_str!("lower/mod.rs"),
        include_str!("lower/object.rs"),
        include_str!("lower/prim_table.rs"),
        include_str!("lower/raw.rs"),
        include_str!("lower/raw_consts.rs"),
        include_str!("lower/schema.rs"),
        include_str!("lower/scope.rs"),
        include_str!("lower/stack.rs"),
        include_str!("lower/stmt.rs"),
        include_str!("lower/tree.rs"),
        include_str!("lower/trees.rs"),
    ];
    const STD: [&str; 6] = [
        include_str!("std/budget.antics"),
        include_str!("std/engine.antics"),
        include_str!("std/flags.antics"),
        include_str!("std/globals.antics"),
        include_str!("std/math.antics"),
        include_str!("std/motives.antics"),
    ];
    static NAMES: std::sync::OnceLock<BTreeSet<&'static str>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        let ident = |s: &str| {
            s.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        };
        let mut names = BTreeSet::new();
        for source in COMPILER {
            for line in source.lines() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                // The text between quotes, `\"` and `'"'` aside: a stray pair only adds
                // names, which costs a tree its short name and nothing else.
                let code = code.replace("\\\"", "").replace("'\"'", "");
                for lit in code.split('"').skip(1).step_by(2) {
                    if ident(lit) {
                        names.insert(&*lit.to_owned().leak());
                    }
                }
            }
        }
        for source in STD {
            for line in source.lines() {
                let Some(at) = line.find("fn ") else { continue };
                let rest = &line[at + 3..];
                let name = rest.split('(').next().unwrap_or_default().trim();
                if ident(name) {
                    names.insert(&*name.to_owned().leak());
                }
            }
        }
        names.extend([
            "std",
            "globals",
            "more_globals",
            "library_consts",
            "budget",
            "engine",
            "flags",
            "math",
            "motives",
            "tree",
        ]);
        names
    })
}

/// Every identifier `text`'s code holds: each identifier token, and each word of a
/// `raw { }` body (strings and comments aside). `None` when `text` does not lex.
fn code_words(text: &str) -> Option<BTreeSet<String>> {
    use crate::v2::lexer::{TokenKind as K, tokenize};
    let mut out = BTreeSet::new();
    for t in tokenize(text).ok()? {
        match t.kind {
            K::Ident(w) => {
                out.insert(w);
            }
            K::RawBlock(body) => out.extend(
                body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .filter(|w| !w.is_empty())
                    .map(str::to_owned),
            ),
            _ => {}
        }
    }
    Some(out)
}

/// Byte offsets of tokens in `text`. The lexer counts a column per byte, except a
/// multibyte character inside a string literal, which counts once; so a token on a line
/// with any non-ASCII byte gets no offset, and whatever needs it is left as it is.
struct TokenOffsets<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> TokenOffsets<'a> {
    fn new(text: &'a str) -> Self {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self { text, line_starts }
    }

    fn of(&self, t: &crate::v2::lexer::Token) -> Option<usize> {
        let start = *self.line_starts.get(t.span.line.checked_sub(1)?)?;
        let line = self.text[start..].split('\n').next()?;
        (line.is_ascii() && t.span.column >= 1 && t.span.column - 1 <= line.len())
            .then(|| start + t.span.column - 1)
    }
}

/// `text` with each `Table::entry` path in code (tokens, never a string or comment) whose
/// table is in `tables` and entry in `entries` written `Table.entry`.
fn dot_paths(text: &str, tables: &[&str], entries: &[String]) -> Option<String> {
    use crate::v2::lexer::{TokenKind as K, tokenize};
    let tokens = tokenize(text).ok()?;
    let offsets = TokenOffsets::new(text);
    let mut out = String::with_capacity(text.len());
    let mut done = 0;
    for w in tokens.windows(3) {
        if let (K::Ident(table), K::ColonColon, K::Ident(entry)) =
            (&w[0].kind, &w[1].kind, &w[2].kind)
            && tables.contains(&table.as_str())
            && entries.contains(entry)
        {
            // Only a path written whole (`Table::entry`, nothing between) is rewritten.
            let (Some(start), Some(colons)) = (offsets.of(&w[0]), offsets.of(&w[1])) else {
                continue;
            };
            if colons != start + table.len()
                || text.get(colons..colons + 2) != Some("::")
                || offsets.of(&w[2]) != Some(colons + 2)
            {
                continue;
            }
            out.push_str(&text[done..colons]);
            out.push('.');
            done = colons + 2;
        }
    }
    out.push_str(&text[done..]);
    Some(out)
}

/// `text` with each word that `renames` maps written as its new name, outside string
/// literals (comments included).
fn apply_renames(text: &str, renames: &[(String, String)]) -> String {
    let map: BTreeMap<&str, &str> = renames
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (mut in_string, mut escaped) = (false, false);
        let mut word_start: Option<usize> = None;
        let flush = |out: &mut String, word: &str| {
            out.push_str(map.get(word).copied().unwrap_or(word));
        };
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            let word_char = c.is_ascii_alphanumeric() || c == b'_';
            if word_char && !in_string {
                word_start.get_or_insert(i);
                i += 1;
                continue;
            }
            if let Some(start) = word_start.take() {
                flush(&mut out, &line[start..i]);
            }
            if !in_string && line[i..].starts_with("//") {
                // A comment: names in it are renamed too, quotes or not.
                let rest = &line[i..];
                let mut last = 0;
                for (at, w) in
                    rest.match_indices(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                {
                    flush(&mut out, &rest[last..at]);
                    out.push_str(w);
                    last = at + w.len();
                }
                flush(&mut out, &rest[last..]);
                break;
            }
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    in_string = false;
                }
            } else if c == b'"' {
                in_string = true;
            }
            let ch_len = line[i..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&line[i..i + ch_len]);
            i += ch_len;
        }
        if let Some(start) = word_start.take() {
            flush(&mut out, &line[start..]);
        }
    }
    out
}

/// `text` with every `else { if .. }` whose block holds the `if` chain alone (no other
/// statement, no comment) written `else if ..`, and `None` when there is none. Only an
/// `if`'s `else` is joined (not a `for` loop's), and `raw { }` bodies are one token, so
/// `.air` is never touched. The parser reads both forms as the same statement.
fn collapse_else_if(text: &str) -> Option<String> {
    use crate::v2::lexer::{TokenKind as K, tokenize};
    let tokens = tokenize(text).ok()?;
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let offset = |t: &crate::v2::lexer::Token| line_starts[t.span.line - 1] + t.span.column - 1;
    let is = |i: usize, word: &str| matches!(&tokens.get(i).map(|t| &t.kind), Some(K::Ident(w)) if w == word);
    // The matching brace of each brace.
    let mut pair = vec![usize::MAX; tokens.len()];
    let mut open = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        match t.kind {
            K::LBrace => open.push(i),
            K::RBrace => {
                let o = open.pop()?;
                pair[o] = i;
                pair[i] = o;
            }
            _ => {}
        }
    }
    // The `{` that opens the body of the `if` / `while` / `for` whose header starts at
    // token `start`: the first `{` outside parentheses and brackets.
    let body_open = |start: usize| {
        let mut depth = 0i32;
        for (k, t) in tokens.iter().enumerate().skip(start + 1) {
            match t.kind {
                K::LParen | K::LBracket => depth += 1,
                K::RParen | K::RBracket => depth -= 1,
                K::LBrace if depth == 0 => return Some(k),
                K::RBrace | K::Semicolon | K::Eof => return None,
                _ => {}
            }
        }
        None
    };
    // The keyword that heads the block whose `{` is token `open`.
    let head = |open: usize| {
        let mut depth = 0i32;
        for k in (0..open).rev() {
            match &tokens[k].kind {
                K::RParen | K::RBracket => depth += 1,
                K::LParen | K::LBracket => depth -= 1,
                K::LBrace | K::RBrace | K::Semicolon if depth == 0 => return None,
                K::Ident(w)
                    if depth == 0
                        && matches!(w.as_str(), "if" | "for" | "while" | "loop" | "match") =>
                {
                    return Some(w.as_str());
                }
                _ => {}
            }
        }
        None
    };
    let blank = |a: usize, b: usize| text[a..b].trim().is_empty();
    let mut cuts: Vec<std::ops::Range<usize>> = Vec::new();
    for i in 1..tokens.len() {
        if !(is(i, "else") && matches!(tokens[i - 1].kind, K::RBrace) && is(i + 2, "if")) {
            continue;
        }
        if !matches!(tokens[i + 1].kind, K::LBrace) || head(pair[i - 1]) != Some("if") {
            continue;
        }
        let close = pair[i + 1];
        // The chain inside: `if .. { } (else if .. { })* (else { })?`, then the `}`.
        let mut j = i + 2;
        let end = loop {
            let Some(body) = body_open(j) else { break None };
            let body_end = pair[body];
            if !is(body_end + 1, "else") {
                break Some(body_end);
            }
            if is(body_end + 2, "if") {
                j = body_end + 2;
            } else if matches!(tokens.get(body_end + 2).map(|t| &t.kind), Some(K::LBrace)) {
                break Some(pair[body_end + 2]);
            } else {
                break None;
            }
        };
        let Some(end) = end else { continue };
        if end + 1 != close {
            continue;
        }
        let (brace, first) = (offset(&tokens[i + 1]), offset(&tokens[i + 2]));
        let (last, closing) = (offset(&tokens[end]) + 1, offset(&tokens[close]));
        if !(blank(brace + 1, first) && blank(last, closing)) {
            continue;
        }
        cuts.push(brace..first);
        cuts.push(last..closing + 1);
    }
    if cuts.is_empty() {
        return None;
    }
    cuts.sort_by_key(|r| std::cmp::Reverse(r.start));
    let mut out = text.to_owned();
    for r in cuts {
        out.replace_range(r, "");
    }
    Some(out)
}

/// `text` with the `use` lines of its leading block that import single names from one
/// module joined into one braced, sorted line (`use std::flags::{Expansions, ObjectFlags};`).
/// A glob or already-braced line stays as it is.
fn group_imports(text: &str) -> String {
    let header: usize = text
        .split_inclusive('\n')
        .take_while(|l| l.starts_with("//"))
        .map(str::len)
        .sum();
    let rest = &text[header..];
    let block = header + rest.len() - rest.trim_start_matches('\n').len();
    let mut end = block;
    let mut lines: Vec<&str> = Vec::new();
    for line in text[block..].split_inclusive('\n') {
        let l = line.trim_end();
        if !(l.starts_with("use ") && l.ends_with(';')) {
            break;
        }
        lines.push(l);
        end += line.len();
    }
    let mut groups: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut kept: Vec<String> = Vec::new();
    for l in &lines {
        let path = &l["use ".len()..l.len() - 1];
        match path.rsplit_once("::") {
            Some((module, name))
                if !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                groups.entry(module).or_default().insert(name);
            }
            _ => kept.push((*l).to_owned()),
        }
    }
    if groups.values().all(|names| names.len() < 2) {
        return text.to_owned();
    }
    for (module, names) in groups {
        let names: Vec<&str> = names.into_iter().collect();
        kept.push(if names.len() == 1 {
            format!("use {module}::{};", names[0])
        } else {
            format!("use {module}::{{{}}};", names.join(", "))
        });
    }
    kept.sort();
    let mut out = text[..block].to_owned();
    for l in kept {
        out.push_str(&l);
        out.push('\n');
    }
    out.push_str(&text[end..]);
    out
}

/// One `if !(a op b) {` / `while !(a op b) {` test written `a flipped b`: the byte range
/// from `!` through `)`, its replacement, and the tree it is in.
#[derive(Clone)]
struct Flip {
    range: std::ops::Range<usize>,
    text: String,
    tree: u16,
}

/// The tree each token is in, read from tokens (a quoted label never counts). A tree is
/// the `fn`, `init`, `main`, `load`, `reset` or `cleanup` member right after a
/// `#[bhav(.., id = N, ..)]` or `#[id(N)]` attribute; any other member (an unpinned `fn`,
/// an interaction with its guard, a declaration) is in no tree. Members are an object's:
/// they sit one brace deep.
fn token_trees(tokens: &[crate::v2::lexer::Token]) -> Vec<Option<u16>> {
    use crate::v2::lexer::TokenKind as K;
    let kind = |k: usize| tokens.get(k).map(|t| &t.kind);
    // Braces and parentheses deep: members sit at one brace and no parenthesis.
    let (mut depth, mut parens) = (0usize, 0usize);
    let (mut pending, mut tree) = (None, None);
    let mut trees = Vec::with_capacity(tokens.len());
    let mut k = 0;
    while k < tokens.len() {
        match &tokens[k].kind {
            K::HashBracket => {
                // The attribute runs to its matching `]`.
                let mut end = k + 1;
                let mut nest = 0usize;
                while let Some(t) = kind(end) {
                    match t {
                        K::LBracket | K::HashBracket => nest += 1,
                        K::RBracket if nest == 0 => break,
                        K::RBracket => nest -= 1,
                        _ => {}
                    }
                    end += 1;
                }
                let attr = &tokens[k + 1..end.min(tokens.len())];
                let word = |j: usize, w: &str| matches!(attr.get(j).map(|t| &t.kind), Some(K::Ident(x)) if x == w);
                let int = |j: usize| match attr.get(j).map(|t| &t.kind) {
                    Some(K::Int(n)) => u16::try_from(*n).ok(),
                    _ => None,
                };
                // Only a member's own attribute counts (not a parameter's, nor one inside
                // an `extern mod`); one without an ID (`#[bhav]`) keeps the pending one.
                let id = if word(0, "bhav") {
                    (1..attr.len()).find_map(|j| {
                        (word(j, "id") && attr.get(j + 1).map(|t| &t.kind) == Some(&K::Assign))
                            .then(|| int(j + 2))
                            .flatten()
                    })
                } else if word(0, "id") {
                    int(2)
                } else {
                    None
                };
                if depth == 1 && parens == 0 && id.is_some() {
                    pending = id;
                }
                for _ in k..=end.min(tokens.len() - 1) {
                    trees.push(tree);
                }
                k = end + 1;
                continue;
            }
            K::LBrace => depth += 1,
            K::RBrace => depth = depth.saturating_sub(1),
            K::LParen => parens += 1,
            K::RParen => parens = parens.saturating_sub(1),
            K::Ident(w) if depth == 1 && parens == 0 => {
                // A member starts after `{`, `}`, `;`, an attribute's `]` or `pub`.
                let starts = k == 0
                    || matches!(
                        &tokens[k - 1].kind,
                        K::LBrace | K::RBrace | K::Semicolon | K::RBracket
                    )
                    || matches!(&tokens[k - 1].kind, K::Ident(p) if p == "pub");
                if starts {
                    tree = if ["fn", "init", "main", "load", "reset", "cleanup"]
                        .contains(&w.as_str())
                    {
                        pending.take()
                    } else {
                        if w != "pub" {
                            pending = None;
                        }
                        None
                    };
                }
            }
            _ => {}
        }
        trees.push(tree);
        k += 1;
    }
    trees
}

/// Every test `!(a op b)` that directly follows `if` or `while` and opens the block (`{`
/// next), with one comparison inside the parentheses and nothing else at their top level,
/// as the flipped comparison. `raw { }` bodies are one token, so `.air` is never touched.
fn negated_comparisons(text: &str) -> Vec<Flip> {
    use crate::v2::lexer::{TokenKind as K, tokenize};
    let Ok(tokens) = tokenize(text) else {
        return Vec::new();
    };
    let offsets = TokenOffsets::new(text);
    let trees = token_trees(&tokens);
    let mut out = Vec::new();
    for i in 0..tokens.len().saturating_sub(3) {
        let K::Ident(w) = &tokens[i].kind else {
            continue;
        };
        if !(w == "if" || w == "while")
            || tokens[i + 1].kind != K::Bang
            || tokens[i + 2].kind != K::LParen
        {
            continue;
        }
        let mut depth = 0i32;
        let mut op: Option<(usize, &'static str)> = None;
        let mut close = None;
        let mut plain = true;
        for (k, t) in tokens.iter().enumerate().skip(i + 3) {
            match &t.kind {
                K::LParen | K::LBracket => depth += 1,
                K::RParen if depth == 0 => {
                    close = Some(k);
                    break;
                }
                K::RParen | K::RBracket => depth -= 1,
                K::EqEq | K::BangEq | K::Lt | K::Gt | K::LtEq | K::GtEq if depth == 0 => {
                    let flipped = match t.kind {
                        K::EqEq => "!=",
                        K::BangEq => "==",
                        K::Lt => ">=",
                        K::Gt => "<=",
                        K::LtEq => ">",
                        _ => "<",
                    };
                    plain &= op.is_none();
                    op = Some((k, flipped));
                }
                K::AndAnd | K::OrOr | K::Bang | K::LBrace | K::RBrace | K::Semicolon | K::Eof
                    if depth == 0 =>
                {
                    plain = false;
                }
                _ => {}
            }
        }
        let (Some(close), Some((at, flipped)), Some(tree)) = (close, op, trees[i]) else {
            continue;
        };
        if !plain || tokens.get(close + 1).map(|t| &t.kind) != Some(&K::LBrace) {
            continue;
        }
        let at_of = |k: usize| offsets.of(&tokens[k]);
        let (Some(bang), Some(lhs_start), Some(op_start), Some(op_end), Some(close_at)) = (
            at_of(i + 1),
            at_of(i + 3),
            at_of(at),
            at_of(at + 1),
            at_of(close),
        ) else {
            continue;
        };
        let lhs = text[lhs_start..op_start].trim();
        let rhs = text[op_end..close_at].trim();
        out.push(Flip {
            range: bang..close_at + 1,
            text: format!("{lhs} {flipped} {rhs}"),
            tree,
        });
    }
    out
}

/// The most passes [`Model::lint_spellings`] makes over a file.
const LINT_SPELLING_PASSES: usize = 3;

/// The most builds one pass of [`Model::lint_spellings`] may take.
const LINT_SPELLING_BUILDS: usize = 24;

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
    /// The structured lift with the other branch order (true branch first rather than the
    /// lower-numbered one), when it differs: [`Plan::other_order`] swaps it in.
    other_order: Option<Box<OrderedLift>>,
    /// Whether the idiomatic body runs as a Sim acting on this object (`(sim: Sim)`).
    sim_ctx: bool,
    lifecycle: Option<Lifecycle>,
    /// Why the tree cannot be written at all (unsupported header).
    omitted: Option<String>,
}

/// Swaps `t`'s structured lift for its other branch order, if it has one.
fn swap_lift(t: &mut TreeModel) {
    if let Some(other) = t.other_order.as_mut() {
        std::mem::swap(&mut t.lifted, &mut other.lifted);
        std::mem::swap(&mut t.body_lines, &mut other.body_lines);
        std::mem::swap(&mut t.body, &mut other.body);
    }
}

/// One structured lift of a tree: its lines, header and closing-brace indices, and
/// parsed body.
struct OrderedLift {
    lifted: Vec<String>,
    body_lines: (usize, usize),
    body: Option<Vec<Stmt>>,
}

/// `text`'s tree as an [`OrderedLift`], or `None` when it holds no tree.
fn ordered_lift(text: &str) -> Option<OrderedLift> {
    let lifted: Vec<String> = text.lines().map(str::to_owned).collect();
    let head = lifted
        .iter()
        .position(|l| l.starts_with("tree ") && l.trim_end().ends_with('{'))?;
    let close = lifted
        .iter()
        .rposition(|l| l.trim_end() == "}")
        .filter(|&i| i > head)
        .unwrap_or(lifted.len());
    let body = crate::parser::parse_module(text)
        .ok()
        .and_then(|m| {
            m.items.into_iter().find_map(|item| match item {
                crate::ast::Item::Tree(t) => Some(t.body),
                _ => None,
            })
        })
        // Labeled-CFG bodies (`@n0:` labels) are only ever written raw.
        .filter(|b| !b.iter().any(|s| matches!(s, Stmt::Label { .. })));
    Some(OrderedLift {
        lifted,
        body_lines: (head, close),
        body,
    })
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
    /// Library trees called through `use std::globals::*;`.
    std_globals: BTreeSet<u16>,
    /// A local library declaration clashes with `std::globals` (see
    /// [`Model::globals_module`]).
    local_globals_clash: bool,
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
    original_resources: BTreeMap<([u8; 4], u16), Vec<Vec<u8>>>,
    /// The original's object definitions, as the VM reads them.
    original_definitions: Option<Vec<(u16, u16, Vec<u8>)>>,
    /// The original's string tables, as the VM reads their text.
    original_strings: BTreeMap<([u8; 4], u16), StringTexts>,
    /// How the original finds a tree by name.
    original_lookup: NameLookup,
    /// Where the original's object definitions register each tree as a callback.
    original_callbacks: Option<Callbacks>,
    library_arities: BTreeMap<u16, usize>,
    /// Trees are checked under the behavioral relation ([`DecompileV2Options::behavioral`]).
    behavioral: bool,
}

impl Model {
    fn build(iff: &Iff, options: &DecompileV2Options) -> Result<Self> {
        let behavioral = options.validate && options.behavioral;
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
                label: tree_label(&c.label),
                ident,
                lifted: Vec::new(),
                body_lines: (0, 0),
                labeled: Vec::new(),
                structured: false,
                body: None,
                other_order: None,
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
            let text =
                match lift_bhav_structured_in_node_order(&tree.original, opts, Some(&lift_arities))
                {
                    StructuredLiftOutcome::Lifted(text) => {
                        tree.structured = true;
                        text
                    }
                    _ => labeled,
                };
            let Some(lift) = ordered_lift(&text) else {
                tree.lifted = text.lines().map(str::to_owned).collect();
                tree.omitted = Some("the lifter produced no tree".to_owned());
                trees.push(tree);
                continue;
            };
            tree.lifted = lift.lifted;
            tree.body_lines = lift.body_lines;
            tree.body = lift.body;
            if tree.structured
                && let StructuredLiftOutcome::Lifted(other) =
                    lift_bhav_structured_with_arities(&tree.original, opts, Some(&lift_arities))
                && other != text
            {
                tree.other_order = ordered_lift(&other).map(Box::new);
                // Node numbers are not compared in behavioral mode: the true branch goes
                // first, as written, which needs no negated conditions.
                if behavioral {
                    swap_lift(&mut tree);
                }
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
        // Identical tables (one per OBJD, typically) are written once.
        let mut tables: Vec<(Vec<u16>, Vec<String>)> = Vec::new();
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
            let mut lines = Vec::new();
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
                lines.push(format!(
                    "{name}: action {}{guard}, autonomy {}",
                    tree_ident(it.action),
                    it.autonomy
                ));
            }
            match tables.iter_mut().find(|(_, l)| *l == lines) {
                Some((ids, _)) => ids.push(c.id),
                None => tables.push((vec![c.id], lines)),
            }
        }
        let interactions = interaction_comments(tables);

        // Sounds: v2 can pin an FWAV's ID and event name when the name is a plain file
        // name (`sound s = fwav(N, wav: "name")`).
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
        // A library tree `std::globals` declares with the arity the bodies call it with is
        // named from there; the rest stay in a local block.
        let std_globals: BTreeSet<u16> = extern_calls
            .iter()
            .copied()
            .filter(|id| {
                std_global(*id)
                    .is_some_and(|g| g.params == literal_calls.get(id).copied().unwrap_or(0))
            })
            .collect();
        let extern_calls: BTreeMap<u16, String> = extern_calls
            .into_iter()
            .map(|id| {
                if let Some(g) = std_global(id).filter(|_| std_globals.contains(&id)) {
                    return (id, g.name.to_owned());
                }
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
        // `use std::globals::*;` merges with a local `extern mod globals`, unless a local
        // declaration takes a tree id or a name `std::globals` declares (another arity, or
        // another tree): the merge refuses that, so those files keep `more_globals`.
        let local_globals_clash = extern_calls.iter().any(|(id, ident)| {
            *id < 8192
                && !std_globals.contains(id)
                && (std_global(*id).is_some() || std_global_names().contains(ident.as_str()))
        });

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
            std_globals,
            local_globals_clash,
            literal_calls,
            trees,
            interactions,
            sounds,
            sounds_skipped,
            ctx_original,
            original_sounds,
            original_resources: resource_chunks(iff),
            original_definitions: definition_data(iff),
            original_strings: string_texts(iff),
            original_lookup: NameLookup::of(iff),
            original_callbacks: callback_slots(iff),
            library_arities: options.library_arities.clone(),
            behavioral,
        })
    }

    /// The local block's name for library trees: `globals`, which merges with the
    /// `std::globals` import, unless a local declaration clashes with it.
    /// The v2 path of the object's own BCON entry at operand address `addr`
    /// (`Table.entry`). A raw block resolves the path to the same address.
    fn bcon_path(&self, addr: u16) -> Option<String> {
        let (table, entry) = (addr >> 7, addr & 0x7F);
        if table >= 64 {
            return None;
        }
        let bcon = self.bcons.iter().find(|b| b.id == 4096 + table)?;
        let (field, _) = bcon.entries.get(usize::from(entry))?;
        Some(format!("{}.{field}", bcon.label))
    }

    /// `line` (lifted `.air`) with each `bcon[addr]` of the object's own tables written as
    /// its v2 path, except under a label `.air` reserves.
    fn name_bcons(&self, line: &str) -> String {
        rewrite_bcons(line, |addr| {
            self.bcon_path(addr).filter(|path| {
                path.split('.')
                    .next()
                    .is_some_and(|label| !crate::parser::is_reserved_ident(label))
            })
        })
    }

    fn globals_module(&self) -> &'static str {
        if self.std_globals.is_empty() || !self.local_globals_clash {
            "globals"
        } else {
            "more_globals"
        }
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

    /// Swaps tree `id`'s structured lift for its other branch order.
    fn swap_order(&mut self, id: u16) {
        if let Some(t) = self.trees.iter_mut().find(|t| t.id == id) {
            swap_lift(t);
        }
    }

    /// For each tree whose `.air` header in `text` declares its locals, the count it
    /// declares and the count it requires, from one build with the headers' counts left out
    /// (the build then gives each tree what it needs). Empty when that build fails.
    fn locals_needed(&self, text: &str) -> BTreeMap<u16, (u16, u16)> {
        let Ok(mut expanded) =
            expand_v2_source_to_objects("decompiled.antics", text, &PackResolverConfig::default())
        else {
            return BTreeMap::new();
        };
        if expanded.len() != 1 {
            return BTreeMap::new();
        }
        let mut expanded = expanded.remove(0);
        let (air, declared) = without_locals(&expanded.air_source);
        expanded.air_source = air;
        let Ok(compiled) = compile_expanded_v2_object(expanded) else {
            return BTreeMap::new();
        };
        declared
            .into_iter()
            .filter_map(|(id, declared)| {
                Some((id, (declared, compiled.behaviors.get(&id)?.locals)))
            })
            .collect()
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
        let iff_bytes = compiled.iff_bytes.clone();
        let iff = Iff::parse(compiled.iff_bytes).map_err(|e| Failure::Air {
            tree: None,
            message: format!("the compiled IFF does not parse: {e}"),
        })?;
        let ctx = live_context(&iff, &self.library_arities);
        Ok(RecompiledFile {
            iff_bytes,
            trees: compiled.behaviors,
            ctx,
            sounds: fwav_sounds(&iff),
            resources: resource_chunks(&iff),
            definitions: definition_data(&iff),
            strings: string_texts(&iff),
            lookup: NameLookup::of(&iff),
            callbacks: callback_slots(&iff),
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
        if let Some(gap) = header_gap(&t.original, b) {
            return TreeCheck::Inconclusive(gap);
        }
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
            &lift_live_options(self.behavioral),
        );
        let check = match result {
            EquivResult::Equivalent { .. } => TreeCheck::Equivalent,
            EquivResult::Counterexample { divergence, .. } => TreeCheck::Different(divergence),
            EquivResult::Inconclusive { reason } => TreeCheck::Inconclusive(reason),
        };
        cache.insert(key, check.clone());
        check
    }

    /// Checks every tree that is written: each tree's check, and the trees not proven.
    fn check_all(
        &self,
        recompiled: &RecompiledFile,
        cache: &mut BTreeMap<(u16, String, String), TreeCheck>,
    ) -> (BTreeMap<u16, TreeCheck>, Vec<u16>) {
        let mut checks = BTreeMap::new();
        let mut unproven = Vec::new();
        for t in self.trees.iter().filter(|t| t.omitted.is_none()) {
            let mut check = self.check_tree(t, recompiled, cache);
            if !matches!(check, TreeCheck::Identical | TreeCheck::Equivalent) {
                unproven.push(t.id);
            } else if let Some(gap) = self.object_gap(t, recompiled) {
                // Settled: no other spelling of the tree closes it.
                check = TreeCheck::Inconclusive(gap);
            }
            checks.insert(t.id, check);
        }
        (checks, unproven)
    }

    /// The final `text` with engine cells that have a typed name written by it (lint
    /// `untyped_engine_field`'s suggestions), with what it compiles to and its checks.
    /// - Every suggestion is taken when the object builds to the same bytes, or when every
    ///   tree proven with the numbers is still proven (a typed name may lower to other
    ///   nodes: `stack_obj = self.id` is not a copy of data word 11).
    /// - Otherwise the suggestions that keep the bytes are taken, and each cell left by
    ///   number carries an `allow` saying why.
    fn type_engine_fields(
        &self,
        text: &str,
        (recompiled, checks, unproven): (RecompiledFile, BTreeMap<u16, TreeCheck>, &[u16]),
        cache: &mut BTreeMap<(u16, String, String), TreeCheck>,
    ) -> (String, RecompiledFile, BTreeMap<u16, TreeCheck>) {
        let fixes = engine_field_fixes(text);
        let typed = |fixes: &[EngineFieldFix]| {
            let t = apply_engine_field_fixes(text, fixes)?;
            let r = self.compile(&t).ok()?;
            Some((t, r))
        };
        let Some((all, all_compiled)) = (!fixes.is_empty()).then(|| typed(&fixes)).flatten() else {
            return (text.to_owned(), recompiled, checks);
        };
        if all_compiled.iff_bytes == recompiled.iff_bytes {
            return (all, recompiled, checks);
        }
        let (all_checks, all_unproven) = self.check_all(&all_compiled, cache);
        if all_unproven.iter().all(|id| unproven.contains(id)) {
            return (all, all_compiled, all_checks);
        }
        let same = |fixes: &[EngineFieldFix]| {
            typed(fixes).is_some_and(|(_, r)| r.iff_bytes == recompiled.iff_bytes)
        };
        let mut kept = keep_fixes(&fixes, &same);
        let base = match (!kept.is_empty()).then(|| typed(&kept)).flatten() {
            Some((t, r)) if r.iff_bytes == recompiled.iff_bytes => t,
            _ => {
                kept.clear();
                text.to_owned()
            }
        };
        let left: Vec<EngineFieldFix> = fixes.into_iter().filter(|f| !kept.contains(f)).collect();
        (allow_engine_fields(&base, &left), recompiled, checks)
    }

    /// The final `text` with lint wave 3's spellings written: typed `std` constants
    /// (`untyped_engine_constant`), method forms (`prefer_method_syntax`) and `?`
    /// (`prefer_question_mark`). The lint proposes each only when the file builds to the same
    /// bytes with it; they are taken together only when the object still builds to the
    /// bytes of `recompiled`, so every check stands. A pass may uncover more (a constant
    /// inside a call written as a method), so it runs again, a few times at most.
    fn lint_spellings(&self, mut text: String, recompiled: &RecompiledFile) -> String {
        for _ in 0..LINT_SPELLING_PASSES {
            let fixes = lint_spellings(&text);
            if fixes.is_empty() {
                break;
            }
            let same = |fixes: &[LintSpelling]| {
                self.compile(&apply_lint_spellings(&text, fixes))
                    .is_ok_and(|r| r.iff_bytes == recompiled.iff_bytes)
            };
            let kept = keep_joint(&fixes, &same, LINT_SPELLING_BUILDS);
            if kept.is_empty() {
                break;
            }
            text = apply_lint_spellings(&text, &kept);
        }
        text
    }

    /// The final `text` in plainer Rust, where the object still builds to the bytes of
    /// `recompiled` (so every check stands):
    /// - a tree whose short name (its label in `snake_case`) is its own is called by it,
    ///   without the `_{id}` suffix (see [`Self::tree_renames`]);
    /// - `else { if .. }` holding the chain alone is `else if ..`;
    /// - `use` lines importing from one module are one braced line.
    ///
    /// The names are tried together, then in halves; the other two never change the bytes,
    /// but are checked the same way.
    fn polish(&self, text: String, recompiled: &RecompiledFile) -> String {
        let same = |t: &str| {
            self.compile(t)
                .is_ok_and(|r| r.iff_bytes == recompiled.iff_bytes)
        };
        let mut text = text;
        let renames = self.tree_renames(&text);
        if !renames.is_empty() {
            let base = text.clone();
            let ok = |r: &[(String, String)]| same(&apply_renames(&base, r));
            let kept = keep_joint(&renames, &ok, POLISH_NAME_BUILDS);
            if !kept.is_empty() {
                text = apply_renames(&text, &kept);
                // A label the new name spells is the one the compiler gives it.
                for t in &self.trees {
                    if kept
                        .iter()
                        .any(|(old, new)| *old == t.ident && *new == t.label)
                    {
                        let attr = format!("#[bhav(id = {}, label = \"{}\", ", t.id, t.label);
                        let plain = format!("#[bhav(id = {}, ", t.id);
                        let dropped = text.replacen(&attr, &plain, 1);
                        if dropped != text && same(&dropped) {
                            text = dropped;
                        }
                    }
                }
            }
        }
        if let Some(joined) = collapse_else_if(&text).filter(|t| same(t)) {
            text = joined;
        }
        let grouped = group_imports(&text);
        if grouped != text && same(&grouped) {
            text = grouped;
        }
        text
    }

    /// In behavioral mode, the final `text` with `!(a op b)` tests written as the flipped
    /// comparison (`a <= b` for `!(a > b)`). The flip is the opposite compare node with its
    /// branches swapped back, so it changes the node's own result: it is taken only where
    /// the checker still proves the tree, so the result is never observed:
    /// - only in a tree proven equivalent (an identical one would stop being identical);
    /// - a tree whose check changes in any way gets back its negations, and the rest are
    ///   tried again (a few rounds at most);
    /// - no flip is kept unless every other tree's check is unchanged too.
    ///
    /// Strict mode keeps every `!(a op b)`: the flipped node is never the same bytes.
    fn flip_negations(
        &self,
        text: String,
        (recompiled, checks): (RecompiledFile, BTreeMap<u16, TreeCheck>),
        cache: &mut BTreeMap<(u16, String, String), TreeCheck>,
    ) -> (String, RecompiledFile, BTreeMap<u16, TreeCheck>) {
        if !self.behavioral {
            return (text, recompiled, checks);
        }
        let mut flips: Vec<Flip> = negated_comparisons(&text)
            .into_iter()
            .filter(|f| matches!(checks.get(&f.tree), Some(TreeCheck::Equivalent)))
            .collect();
        let kind = std::mem::discriminant::<TreeCheck>;
        for _ in 0..NEGATION_ROUNDS {
            if flips.is_empty() {
                break;
            }
            let mut flipped = text.clone();
            let mut sorted: Vec<&Flip> = flips.iter().collect();
            sorted.sort_by_key(|f| std::cmp::Reverse(f.range.start));
            for f in sorted {
                flipped.replace_range(f.range.clone(), &f.text);
            }
            let Ok(rebuilt) = self.compile(&flipped) else {
                break;
            };
            let (new_checks, _) = self.check_all(&rebuilt, cache);
            let changed: BTreeSet<u16> = checks
                .iter()
                .filter(|(id, c)| new_checks.get(id).is_none_or(|n| kind(n) != kind(c)))
                .map(|(id, _)| *id)
                .collect();
            if changed.is_empty() {
                return (flipped, rebuilt, new_checks);
            }
            let flipped_trees: BTreeSet<u16> = flips.iter().map(|f| f.tree).collect();
            if !changed.is_subset(&flipped_trees) {
                break;
            }
            flips.retain(|f| !changed.contains(&f.tree));
        }
        (text, recompiled, checks)
    }

    /// The final `text` with an `allow` above each `!(a op b)` lint `negated_comparison`
    /// still reports: one the lift keeps on purpose, as the original's compare node (strict
    /// mode keeps every node's bytes, and behavioral mode flips one only where the tree's
    /// check proves it). Unchanged when the result does not lint cleaner.
    /// `text` with a BCON entry that lint `unused_decl` reports as never read written
    /// `Table.entry` again in v2 code. The lint counts a read by a field's dot and by name in
    /// `raw { }`, but not by a `Table::entry` path, so a table that raw code reads by name
    /// would report its other entries. Only a file whose table is read both ways is linted,
    /// and the rewrite (code tokens only) is kept only if the object builds the same bytes.
    fn dot_bcon_reads_the_lint_misses(&self, text: String, recompiled: &RecompiledFile) -> String {
        use crate::v2::lint::{UNUSED_DECL, lint_v2_source};
        let both: Vec<&str> = self
            .bcons
            .iter()
            .map(|b| b.label.as_str())
            .filter(|l| text.contains(&format!("{l}::")) && text.contains(&format!("{l}.")))
            .collect();
        if both.is_empty() {
            return text;
        }
        let unread = |text: &str| -> Option<Vec<String>> {
            let lints = lint_v2_source(text, "decompiled.antics").ok()?;
            Some(
                lints
                    .iter()
                    .filter(|d| d.rule == UNUSED_DECL)
                    .filter_map(|d| {
                        let rest = d.message.strip_prefix("BCON entry `")?;
                        Some(rest[..rest.find('`')?].to_owned())
                    })
                    .collect(),
            )
        };
        let Some(names) = unread(&text) else {
            return text;
        };
        if names.is_empty() {
            return text;
        }
        let Some(out) = dot_paths(&text, &both, &names) else {
            return text;
        };
        let same = self
            .compile(&out)
            .is_ok_and(|r| r.iff_bytes == recompiled.iff_bytes);
        match unread(&out) {
            Some(after) if same && after.is_empty() => out,
            _ => text,
        }
    }

    fn allow_kept_negations(&self, text: String) -> String {
        use crate::v2::lint::{NEGATED_COMPARISON, lint_v2_source};
        // Cheap first: a file with no negated test needs no lint run.
        if !text.contains("!(") {
            return text;
        }
        let Ok(before) = lint_v2_source(&text, "decompiled.antics") else {
            return text;
        };
        let lines: BTreeSet<usize> = before
            .iter()
            .filter(|d| d.rule == NEGATED_COMPARISON)
            .map(|d| d.line)
            .collect();
        if lines.is_empty() {
            return text;
        }
        let mut out = String::with_capacity(text.len());
        for (n, line) in text.split_inclusive('\n').enumerate() {
            if lines.contains(&(n + 1)) {
                let indent = &line[..line.len() - line.trim_start().len()];
                out.push_str(&format!(
                    "{indent}// antics-lint: allow({NEGATED_COMPARISON}): the original's compare node, kept as it is\n"
                ));
            }
            out.push_str(line);
        }
        match lint_v2_source(&out, "decompiled.antics") {
            Ok(after)
                if after.len() + lines.len() <= before.len()
                    && after.iter().all(|d| d.rule != NEGATED_COMPARISON) =>
            {
                out
            }
            _ => text,
        }
    }

    /// `(ident, short name)` for each tree that may drop its `_{id}` suffix: its label's
    /// `snake_case` form is no other tree's, is no word of `text` (a local, state, sound,
    /// constant or declaration), and is not reserved ([`reserved_tree_names`]).
    fn tree_renames(&self, text: &str) -> Vec<(String, String)> {
        let short = |t: &TreeModel| {
            t.ident
                .strip_suffix(&format!("_{}", t.id))
                .filter(|base| !base.is_empty())
                .map(str::to_owned)
        };
        let mut count: BTreeMap<String, usize> = BTreeMap::new();
        for t in &self.trees {
            if let Some(base) = short(t) {
                *count.entry(base).or_default() += 1;
            }
        }
        let Some(used) = code_words(text) else {
            return Vec::new();
        };
        let reserved = reserved_tree_names();
        self.trees
            .iter()
            .filter_map(|t| {
                let base = short(t)?;
                (count[&base] == 1
                    && !used.contains(base.as_str())
                    && !reserved.contains(base.as_str())
                    && !crate::parser::is_reserved_ident(&base)
                    && used.contains(t.ident.as_str()))
                .then(|| (t.ident.clone(), base))
            })
            .collect()
    }

    /// Why tree `t`, proven by its code, still may not behave the same in the rebuilt
    /// object, where no other spelling of it helps:
    /// - it is labelled differently (a lifecycle block is named after its kind), which
    ///   Run Tree By Name may see;
    /// - it runs a tree by name, and the rebuilt file's tree labels differ;
    /// - it is the object's `init`/`main`, and the rebuilt object's entry runs another
    ///   tree (a plain tree written for it gets a compiler-made block);
    /// - the rebuilt object links another semi-global file, which any of its trees may
    ///   reach (semi-global calls and constants, and global trees, which run with this
    ///   object as their owner);
    /// - the rebuilt file drops this file's non-private trees or constant tables, which
    ///   its trees reach when it is another object's global or semi-global file;
    /// - it reads this file's definitions or callback tables, or creates an object (whose
    ///   callbacks run), and the rebuilt object differs there.
    fn object_gap(&self, t: &TreeModel, recompiled: &RecompiledFile) -> Option<String> {
        let b = recompiled.trees.get(&t.id)?;
        if b.label != t.original.label {
            return Some(format!(
                "the rebuilt tree is labelled {:?}, not {:?}, which Run Tree By Name may see",
                b.label, t.original.label
            ));
        }
        if self.original_lookup.glob != recompiled.lookup.glob {
            return Some(
                "the rebuilt object links another semi-global file, which its trees reach \
                 through semi-global calls and constants and the global trees they call"
                    .to_owned(),
            );
        }
        let foreign = |of: &BTreeMap<([u8; 4], u16), Vec<Vec<u8>>>| {
            of.iter()
                .filter(|(key, _)| foreign_code(key))
                .map(|(key, data)| (*key, data.clone()))
                .collect::<Vec<_>>()
        };
        let dropped = foreign(&self.original_resources);
        if dropped != foreign(&recompiled.resources) {
            let names: Vec<String> = dropped
                .iter()
                .map(|((tag, id), _)| format!("{} {id}", String::from_utf8_lossy(tag)))
                .collect();
            return Some(format!(
                "the rebuilt file does not keep this file's non-private code ({}), which its \
                 trees reach by number or by name when it is another object's global or \
                 semi-global file",
                names.join(", ")
            ));
        }
        let callbacks = || match (&self.original_callbacks, &recompiled.callbacks) {
            (Some(a), Some(b)) => Some(a.tables == b.tables),
            _ => None,
        };
        for i in &t.original.instructions {
            if i.opcode == EXPRESSION {
                let scopes = [i.operands[6], i.operands[7]];
                if scopes.iter().any(|s| DEFINITION_SCOPES.contains(s))
                    && (self.original_definitions.is_none()
                        || self.original_definitions != recompiled.definitions)
                {
                    return Some(
                        "reads an object definition's words, which the rebuilt object's \
                         definitions give differently"
                            .to_owned(),
                    );
                }
                if scopes.contains(&CALLBACK_SCOPE) && callbacks() != Some(true) {
                    return Some(
                        "reads an object's callback table, which the rebuilt object gives \
                         differently"
                            .to_owned(),
                    );
                }
            }
            if runs_callbacks(i) && callbacks() != Some(true) {
                return Some(
                    "may run object callbacks (creating, moving or removing an object, building a \
                     portrait person or cancelling an action), which the rebuilt object's \
                     tables give differently"
                        .to_owned(),
                );
            }
            let names = self.names_run(i, &self.original_strings);
            if names != self.names_run(i, &recompiled.strings) {
                return Some(
                    "runs a tree by a name the rebuilt object's names table gives differently"
                        .to_owned(),
                );
            }
            if let Some(name) = names
                .iter()
                .find(|n| !self.original_lookup.same_result(&recompiled.lookup, n))
            {
                return Some(match name {
                    TreeName::Known(n) => format!(
                        "runs the tree named {:?} by name, which the rebuilt object may \
                         resolve differently",
                        String::from_utf8_lossy(n)
                    ),
                    TreeName::Unknown | TreeName::Entry(_) => {
                        "runs a tree by a name not known here, which the rebuilt object's tree \
                         labels may resolve differently"
                            .to_owned()
                    }
                });
            }
            if let Some(slot) = functional_callback(i) {
                let (Some(a), Some(b)) = (&self.original_callbacks, &recompiled.callbacks) else {
                    return Some("an object's callback table does not parse".to_owned());
                };
                if a.actions_at(slot) != b.actions_at(slot) {
                    return Some(format!(
                        "runs callback {slot} of an object, which the rebuilt object's callback \
                         tables give another tree"
                    ));
                }
            }
            if self.runs_shared_code(i) && !self.same_owner_environment(recompiled) {
                return Some(
                    "may run code outside this file's private trees (a global or semi-global \
                     tree, a tree found by name, a callback), which may read this object's \
                     data the rebuilt object gives differently"
                        .to_owned(),
                );
            }
        }
        let slots = |c: &Option<Callbacks>| {
            c.as_ref()
                .map(|c| c.by_tree.get(&t.id).cloned().unwrap_or_default())
        };
        match (
            slots(&self.original_callbacks),
            slots(&recompiled.callbacks),
        ) {
            (Some(a), Some(b)) if a == b => None,
            (Some(_), Some(_)) => Some(
                "the rebuilt object's callback tables do not register this tree where the \
                 original's do"
                    .to_owned(),
            ),
            _ => Some("an object's callback table does not parse".to_owned()),
        }
    }

    /// The names instruction `i` may look a tree up by, reading Run Tree By Name's name
    /// from `strings`.
    fn names_run(
        &self,
        i: &sims_formats::behavior::Instruction,
        strings: &BTreeMap<([u8; 4], u16), StringTexts>,
    ) -> Vec<TreeName> {
        if let Some((_, names)) = FIXED_NAME_LOOKUPS.iter().find(|(op, _)| *op == i.opcode) {
            if i.opcode == CREATE_OBJECT && i.operands[5] & CREATE_FROM_RECORD == 0 {
                return Vec::new();
            }
            return names
                .iter()
                .map(|n| TreeName::Known(n.as_bytes().to_vec()))
                .collect();
        }
        if i.opcode != RUN_TREE_BY_NAME
            || u16::from_le_bytes([i.operands[0], i.operands[1]]) != TREE_NAMES_TABLE
        {
            // The VM refuses another table before it looks anything up.
            return Vec::new();
        }
        if i.operands[4] == 0 {
            // Entry 0 names nothing, in either table.
            return Vec::new();
        }
        let entry = match strings.get(&(*b"STR#", TREE_NAMES_TABLE)) {
            // No table: the VM looks nothing up.
            None => Some(None),
            Some(Ok(texts)) => Some(
                usize::from(i.operands[4])
                    .checked_sub(1)
                    .and_then(|i| texts.get(i))
                    .filter(|n| !n.is_empty())
                    .cloned(),
            ),
            Some(Err(_)) => None,
        };
        if i.operands[2] & 1 != 0 {
            // The global file's table. This file may be the global file, so its own entry
            // must not change either.
            return vec![TreeName::Unknown, TreeName::Entry(entry)];
        }
        match entry {
            // No name: the VM looks nothing up.
            Some(None) => Vec::new(),
            Some(Some(n)) => vec![TreeName::Known(n)],
            None => vec![TreeName::Unknown],
        }
    }

    /// The trees a Run Functional Tree instruction `i` may run in the original: the
    /// action each of this file's definitions has for its function.
    fn functional_callees(
        &self,
        i: &sims_formats::behavior::Instruction,
    ) -> impl Iterator<Item = u16> + '_ {
        functional_callback(i)
            .zip(self.original_callbacks.as_ref())
            .into_iter()
            .flat_map(|(slot, c)| c.actions_at(slot).into_values())
            .filter(|&id| id != 0)
    }

    /// Whether instruction `i` may run code outside this file's private trees, which
    /// the lift does not see:
    /// - a global or semi-global tree, called by number;
    /// - a tree found by name, which may be another file's when this one has no match;
    /// - a callback run by function, from whichever object is the stack object;
    /// - callbacks a primitive runs ([`runs_callbacks`]): a created, moved or
    ///   removed object's, its container's, or a cancelled action's target's (any of
    ///   which may be another object).
    ///
    /// Such code may run on this thread with this object as owner, so it may run any
    /// of this file's trees by name, create an object, or read any of its data.
    fn runs_shared_code(&self, i: &sims_formats::behavior::Instruction) -> bool {
        (i.opcode >= 0x100 && !PRIVATE_TREES.contains(&i.opcode))
            || i.opcode == RUN_FUNCTIONAL_TREE
            || runs_callbacks(i)
            || !self.names_run(i, &self.original_strings).is_empty()
            || self
                .direct_named_callees(i)
                .iter()
                .any(|id| !PRIVATE_TREES.contains(id))
    }

    /// Whether the rebuilt object gives code running with it as owner
    /// ([`Self::runs_shared_code`]) the same environment: every resource as the VM
    /// loads it, tree labels and `GLOB` link, text tables, sounds, definitions and
    /// callback tables.
    fn same_owner_environment(&self, recompiled: &RecompiledFile) -> bool {
        self.original_resources == recompiled.resources
            && self.original_lookup == recompiled.lookup
            && self.original_strings == recompiled.strings
            && self.original_sounds == recompiled.sounds
            && self.original_definitions.is_some()
            && self.original_definitions == recompiled.definitions
            && self.original_callbacks.is_some()
            && self.original_callbacks.as_ref().map(|c| &c.tables)
                == recompiled.callbacks.as_ref().map(|c| &c.tables)
    }

    /// The trees of this file instruction `i` may run in the original other than by
    /// number: [`Self::direct_named_callees`], and every private tree when it may run
    /// code outside them ([`Self::runs_shared_code`]).
    fn named_callees(&self, i: &sims_formats::behavior::Instruction) -> Vec<u16> {
        let mut out = self.direct_named_callees(i);
        if self.runs_shared_code(i) {
            out.extend(
                self.original_lookup
                    .names
                    .keys()
                    .copied()
                    .filter(|id| PRIVATE_TREES.contains(id)),
            );
        }
        out
    }

    /// The trees of this file instruction `i` may run by name in the original (the tree
    /// each name finds here, or every tree for a name not known here), by function, or
    /// or as callbacks a primitive runs ([`runs_callbacks`]).
    fn direct_named_callees(&self, i: &sims_formats::behavior::Instruction) -> Vec<u16> {
        let mut out = Vec::new();
        for name in self.names_run(i, &self.original_strings) {
            match name {
                TreeName::Known(n) => out.extend(self.original_lookup.local_hit(&n)),
                TreeName::Unknown => out.extend(self.original_lookup.names.keys().copied()),
                TreeName::Entry(_) => {}
            }
        }
        out.extend(self.functional_callees(i));
        if runs_callbacks(i)
            && let Some(c) = &self.original_callbacks
        {
            // The object whose callbacks run may be one of this file's definitions.
            out.extend(c.by_tree.keys().copied());
        }
        out
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
                    let direct = PRIVATE_TREES.contains(&ins.opcode).then_some(ins.opcode);
                    direct
                        .into_iter()
                        .chain(self.named_callees(ins))
                        .filter(|&callee| callee != t.id)
                        .find(|callee| !match status.get(callee) {
                            Some(&ok) => ok,
                            None => !recompiled.trees.contains_key(callee),
                        })
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

/// A name a primitive looks a tree up by.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TreeName {
    Known(Vec<u8>),
    /// A name from another file (the global file's or a running object's table).
    Unknown,
    /// This file's `STR#` 303 entry for a name read from another file's table (None
    /// when the table is not one the lift reads as plain text): compared only, since
    /// this file may be that other file.
    Entry(Option<Option<Vec<u8>>>),
}

/// What one file contributes to finding a tree by name (`sims-vm`
/// `Program::named_tree`): the VM searches the owner's file, its semi-global file and
/// the global file by exact name, then the three again ignoring ASCII case; two
/// matches in one file is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NameLookup {
    /// Every tree's label as the VM loads it: the last BHAV chunk of each ID, deleted
    /// ones included.
    names: BTreeMap<u16, Vec<u8>>,
    /// The first `GLOB` chunk: the semi-global file the catalog links (none for an
    /// empty name), or its bytes when it does not parse.
    glob: std::result::Result<Option<String>, Vec<u8>>,
}

impl NameLookup {
    fn of(iff: &Iff) -> Self {
        Self {
            names: vm_kept_bhavs(iff)
                .chunks
                .into_iter()
                .map(|c| (c.id, iff.label_bytes(c).to_vec()))
                .collect(),
            glob: match iff.all(b"GLOB").next() {
                None => Ok(None),
                Some(c) => sims_formats::execution::global_link(iff.data(c))
                    .map_err(|_| iff.data(c).to_vec()),
            },
        }
    }

    /// One pass over this file: by exact name or ignoring case.
    fn find(&self, name: &[u8], exact: bool) -> std::result::Result<Option<u16>, ()> {
        let mut found = self.names.iter().filter(|(_, label)| {
            if exact {
                label.as_slice() == name
            } else {
                label.eq_ignore_ascii_case(name)
            }
        });
        match (found.next(), found.next()) {
            (Some(_), Some(_)) => Err(()),
            (first, _) => Ok(first.map(|(&id, _)| id)),
        }
    }

    /// Whether every lookup of `name` that consults this file gets the same result with
    /// `other` in its place, whether this file is the owner's, the semi-global or
    /// neither. The exact pass must agree; when it finds nothing here, the passes after
    /// it (the semi-global this file links, then this file ignoring case) must too.
    fn same_result(&self, other: &Self, name: &TreeName) -> bool {
        let name = match name {
            TreeName::Known(name) => name,
            TreeName::Unknown => return self == other,
            TreeName::Entry(_) => return true,
        };
        let exact = self.find(name, true);
        exact == other.find(name, true)
            && (exact != Ok(None)
                || (self.glob == other.glob && self.find(name, false) == other.find(name, false)))
    }

    /// The tree of this file a lookup of `name` can run.
    fn local_hit(&self, name: &[u8]) -> Option<u16> {
        match self.find(name, true) {
            Ok(Some(id)) => Some(id),
            Ok(None) => self.find(name, false).ok().flatten(),
            Err(()) => None,
        }
    }
}

/// Each tree an object definition registers as a callback: the definitions naming it (by
/// GUID, and which of the file's definitions with that GUID in the catalog's resource
/// order) and the callback slots, as action (true) or condition.
type CallbackSlots = BTreeMap<u16, BTreeSet<(u32, usize, usize, bool)>>;

/// An object file's callbacks, as the VM finds them.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Callbacks {
    by_tree: CallbackSlots,
    /// Each definition's (GUID, nth with that GUID) callback table: (condition, action)
    /// per slot.
    tables: BTreeMap<(u32, usize), Vec<(u16, u16)>>,
}

impl Callbacks {
    /// The action each definition has in `slot` (0 past the table's end).
    fn actions_at(&self, slot: usize) -> BTreeMap<(u32, usize), u16> {
        self.tables
            .iter()
            .map(|(&key, table)| (key, table.get(slot).map_or(0, |&(_, action)| action)))
            .collect()
    }
}

/// The callback a Run Functional Tree instruction runs (`sims-vm`
/// `primitives/run_functional_tree.rs`); None when its function names none.
fn functional_callback(i: &sims_formats::behavior::Instruction) -> Option<usize> {
    if i.opcode != RUN_FUNCTIONAL_TREE {
        return None;
    }
    let function = i16::from_le_bytes([i.operands[0], i.operands[1]]);
    Some(match function {
        0..=7 => 0x12 + function as usize,
        8 => 0x1a,
        9 => 0x1b,
        10 => 0x0e,
        11 => 0x1c,
        12 => 0x10,
        13 => 0x11,
        14 => 0x1d,
        _ => return None,
    })
}

/// Where the VM finds each callback (`sims-vm` `program.rs`): the OBJf table of the
/// definition's chunk ID when there is one, else the definition's own words. The VM
/// keeps one table per chunk ID (the last definition of that ID in resource order), and
/// every definition with that ID uses it. None when the resource map, a definition or a
/// table does not parse.
fn callback_slots(iff: &Iff) -> Option<Callbacks> {
    let definitions = iff.resource_order(b"OBJD").ok()?;
    let mut by_chunk: BTreeMap<u16, Vec<(u16, u16)>> = BTreeMap::new();
    for chunk in &definitions {
        let data = iff.data(chunk);
        let functions = match iff.get(b"OBJf", chunk.id) {
            Some(table) => sims_formats::execution::ObjectFunctions::parse(iff.data(table))
                .ok()?
                .entries
                .iter()
                .map(|f| (f.condition, f.action))
                .collect(),
            None => DEFINITION_CALLBACKS
                .iter()
                .map(|&at| {
                    let b = |k: usize| data.get(at + k).copied().unwrap_or(0);
                    (0, u16::from_le_bytes([b(0), b(1)]))
                })
                .collect(),
        };
        by_chunk.insert(chunk.id, functions);
    }
    let mut out = Callbacks {
        by_tree: CallbackSlots::new(),
        tables: BTreeMap::new(),
    };
    let mut seen: BTreeMap<u32, usize> = BTreeMap::new();
    for chunk in &definitions {
        let guid = ObjectDefinition::parse(iff.data(chunk)).ok()?.guid;
        let nth = seen.entry(guid).or_default();
        let order = *nth;
        *nth += 1;
        let functions = by_chunk.get(&chunk.id)?.clone();
        for (slot, &(condition, action)) in functions.iter().enumerate() {
            for (tree, is_action) in [(action, true), (condition, false)] {
                if tree != 0 {
                    out.by_tree
                        .entry(tree)
                        .or_default()
                        .insert((guid, order, slot, is_action));
                }
            }
        }
        out.tables.insert((guid, order), functions);
    }
    Some(out)
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

/// The resources a tree can read through a primitive, keyed by tag and ID, as the VM
/// reads them (`sims-vm` `load_library_with_pool`), plus the trees and constants the
/// lift never writes ([`foreign_code`]):
/// - `BHAV`, `BCON`, `FCNS`, `TTAB`, `SLOT`: the last chunk of each ID in file order,
///   deleted ones included;
/// - `STR#`: every chunk of the ID in file order (the VM keeps the last one that
///   parses, deleted or not; comparing them all is stricter);
/// - the rest (`CTSS`, `CATS`, `CST`, `TTAs`, `GLOB`, `TPRP`, `OBJD`, `OBJf`, and
///   `BMP_` 2002): each resource of that key in resource order (live chunks; every
///   chunk of the tag in file order when the resource map does not parse).
fn resource_chunks(iff: &Iff) -> BTreeMap<([u8; 4], u16), Vec<Vec<u8>>> {
    const LAST_CHUNK: [&[u8; 4]; 5] = [b"BCON", b"FCNS", b"TTAB", b"SLOT", b"FWAV"];
    const EVERY_CHUNK: [&[u8; 4]; 1] = [b"STR#"];
    const RESOURCE_ORDER: [&[u8; 4]; 8] = [
        b"CTSS", b"CATS", b"\0CST", b"TTAs", b"GLOB", b"TPRP", b"OBJD", b"OBJf",
    ];
    let mut out: BTreeMap<([u8; 4], u16), Vec<Vec<u8>>> = BTreeMap::new();
    for c in &iff.chunks {
        if LAST_CHUNK.contains(&&c.tag) {
            if &c.tag == b"BCON" && PRIVATE_BCONS.contains(&c.id) {
                // Written by the lift and checked as the trees' constants.
                continue;
            }
            out.insert((c.tag, c.id), vec![iff.data(c).to_vec()]);
        } else if EVERY_CHUNK.contains(&&c.tag) {
            out.entry((c.tag, c.id))
                .or_default()
                .push(iff.data(c).to_vec());
        }
    }
    for tag in RESOURCE_ORDER {
        let chunks = iff
            .resource_order(tag)
            .unwrap_or_else(|_| iff.chunks.iter().filter(|c| &c.tag == tag).collect());
        for c in chunks {
            out.entry((c.tag, c.id))
                .or_default()
                .push(iff.data(c).to_vec());
        }
    }
    // The portrait bitmap the VM checks for (`load_library_with_pool`).
    let bitmaps = iff
        .resource_order(b"BMP_")
        .unwrap_or_else(|_| iff.chunks.iter().filter(|c| &c.tag == b"BMP_").collect());
    for c in bitmaps.into_iter().filter(|c| c.id == PORTRAIT_BITMAP) {
        out.entry((c.tag, c.id))
            .or_default()
            .push(iff.data(c).to_vec());
    }
    for c in vm_kept_bhavs(iff).chunks {
        if !PRIVATE_TREES.contains(&c.id) {
            out.insert((c.tag, c.id), vec![iff.data(c).to_vec()]);
        }
    }
    out
}

/// Whether instruction `i` may run object callbacks in the VM ([`CALLBACK_PRIMITIVES`]).
/// Dialog does when it builds a temporary person for a portrait (`sims-vm`
/// `dialog_portrait`, whose `init` runs): kinds 0 to 4 (operand 5) with icon mode 0
/// or 2 (operand 7 bits 1 to 3).
fn runs_callbacks(i: &sims_formats::behavior::Instruction) -> bool {
    if i.opcode == DIALOG {
        return i.operands[5] <= 4 && matches!((i.operands[7] >> 1) & 7, 0 | 2);
    }
    CALLBACK_PRIMITIVES.contains(&i.opcode)
}

/// The portrait bitmap ID the VM loads (`sims-vm` `program.rs` `PORTRAIT_BITMAP`).
const PORTRAIT_BITMAP: u16 = 2002;

/// Whether a resource key is code the lift never writes: a tree outside the private
/// range, or a constant table outside the private ones. This file's trees reach them
/// when it is another object's global or semi-global file.
fn foreign_code(key: &([u8; 4], u16)) -> bool {
    &key.0 == b"BHAV" || &key.0 == b"BCON"
}

/// Every object definition as the VM reads it, in resource order (None when the
/// resource map does not parse).
fn definition_data(iff: &Iff) -> Option<Vec<(u16, u16, Vec<u8>)>> {
    Some(
        iff.resource_order(b"OBJD")
            .ok()?
            .into_iter()
            .map(|c| (c.id, c.flags, iff.data(c).to_vec()))
            .collect(),
    )
}

struct RecompiledFile {
    /// The whole compiled object.
    iff_bytes: Vec<u8>,
    trees: BTreeMap<u16, BhavChunk>,
    ctx: LiveContext,
    sounds: BTreeMap<u16, String>,
    resources: BTreeMap<([u8; 4], u16), Vec<Vec<u8>>>,
    definitions: Option<Vec<(u16, u16, Vec<u8>)>>,
    strings: BTreeMap<([u8; 4], u16), StringTexts>,
    attributes: Option<u16>,
    lookup: NameLookup,
    callbacks: Option<Callbacks>,
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
        // Run Tree By Name reads its name from this file's table unless byte 2 bit 0
        // picks the global file's.
        RUN_TREE_BY_NAME
            if i.operands[2] & 1 == 0
                && u16::from_le_bytes([i.operands[0], i.operands[1]]) == TREE_NAMES_TABLE =>
        {
            Some(TREE_NAMES_TABLE)
        }
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

/// A tree's label as written (escaped by [`v2_string_literal`]): the whole label, since
/// Run Tree By Name compares all of it. One the compiler cannot store (a control
/// character, a character outside Windows-1252, more than 63 bytes) is cleaned and cut to
/// fit, and the tree is left unproven by its label.
fn tree_label(label: &str) -> String {
    let fits = !label.chars().any(char::is_control)
        && sims_formats::far::encode_name(label).is_some_and(|b| b.len() <= 63);
    if fits {
        return label.to_owned();
    }
    let mut cleaned = clean_label(label);
    cleaned.truncate(63);
    cleaned
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

/// Whether a body reads mostly the Sim (`my.motive`, `my.person`, `my.person[temp]`) and
/// the object as the stack object (`stack.attr`), as interaction trees do. Only a Sim tree
/// names `my.person[temp[k]]` (`sim.person[temp[k]]`).
fn prefers_sim_context(body: &[Stmt]) -> bool {
    let (mut sim, mut obj) = (0usize, 0usize);
    walk_stmts(body, &mut |s| {
        for op in stmt_operands(s) {
            if let OperandRef::Direct(o) = op {
                match (o.scope, o.index) {
                    // `my.id` is `sim` in one context and `self` in the other.
                    (3, 11) => {}
                    (1 | 14 | 16 | 18 | 30, _) => sim += 1,
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

/// Whether a body may assign `stack_obj` (`verify.rs` lists the primitives that write it).
fn assigns_stack_obj(body: &[Stmt]) -> bool {
    fn prim_moves(p: &PrimitiveCall) -> bool {
        match unpadded(p) {
            // Searches and creations select what they find or make.
            PrimitiveCall::FindBestInteraction
            | PrimitiveCall::FindBestObject { .. }
            | PrimitiveCall::MakeNewCharacter { .. }
            | PrimitiveCall::CreateObject { .. } => true,
            PrimitiveCall::ExprRaw { lhs, operator, .. } => {
                is_operand(lhs, 10, 0)
                    && (assign_op_of(*operator).is_some()
                        || matches!(
                            compare_op_of(*operator),
                            Some(CompareOp::IncLt | CompareOp::DecGt)
                        ))
            }
            PrimitiveCall::Random { dst, .. } | PrimitiveCall::GetDirectionTo { dst, .. } => {
                is_operand(dst, 10, 0)
            }
            PrimitiveCall::Relationship { value, .. } => is_operand(value, 10, 0),
            // Without bit 7 the step writes `stack_obj`, whatever `dest` says.
            PrimitiveCall::SetToNext { mode, dest, .. } => {
                mode & 0x80 == 0 || dest.as_ref().is_some_and(|d| is_operand(d, 10, 0))
            }
            _ => false,
        }
    }
    // An unparsed node of an opcode that can write `stack_obj` (expression, random,
    // direction, searches, new character, relationship, set-to-next, create) may.
    fn raw_moves(opcode: u16) -> bool {
        matches!(opcode, 2 | 8 | 12 | 14 | 19 | 26 | 31 | 42)
    }
    fn cond_moves(c: &Condition) -> bool {
        match c {
            Condition::Primitive { prim, .. } => prim_moves(prim),
            Condition::RawOp { opcode, .. } => raw_moves(*opcode),
            Condition::Compare {
                lhs,
                op: CompareOp::IncLt | CompareOp::DecGt,
                ..
            } => is_operand(lhs, 10, 0),
            Condition::Not(inner) => cond_moves(inner),
            _ => false,
        }
    }
    let mut found = false;
    walk_stmts(body, &mut |s| {
        found |= match s {
            Stmt::Assign { lhs, .. } => is_operand(lhs, 10, 0),
            Stmt::Primitive { prim, .. } => prim_moves(prim),
            Stmt::RawOp { opcode, .. } | Stmt::InlineRawOp { opcode, .. } => raw_moves(*opcode),
            Stmt::If { cond, .. } | Stmt::While { cond, .. } => cond_moves(cond),
            _ => false,
        };
    });
    found
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
        PrimitiveCall::SetToNext { guid, .. } | PrimitiveCall::CreateObject { guid, .. }
            if *guid != 0 =>
        {
            out.push(*guid);
        }
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

/// The interaction comment lines: with more than one distinct table, each starts with a
/// line naming its TTAB chunk(s).
fn interaction_comments(tables: Vec<(Vec<u16>, Vec<String>)>) -> Vec<String> {
    let headed = tables.len() > 1;
    let mut out = Vec::new();
    for (ids, lines) in tables {
        if headed {
            let ids: Vec<String> = ids.iter().map(u16::to_string).collect();
            out.push(format!("TTAB {}:", ids.join(", ")));
        }
        out.extend(lines);
    }
    out
}

/// The header lines of a source checked under the behavioral relation. They name the
/// relation only: which trees proved is in the lift's report, not the source.
const BEHAVIORAL_HEADER: [&str; 4] = [
    "// Checked in behavioral mode (`--behavioral`): node numbers and frame storage were not",
    "// compared, so a tree proven equivalent is equivalent only for runs that start and end",
    "// under one build (docs/decisions/20261123-verifier-behavioral-mode.md). Not every tree",
    "// is proven; the lift's report lists the ones that are not.",
];

/// The checker's bounds for `lift`: the defaults, but at most 50,000 pairs of paths with
/// different outputs (the default is 250,000). Running out leaves a tree unproven, so it
/// falls back to a lower level, and never proves one. It stops a file of many large trees
/// that each fail at the first levels from taking minutes. Over the full corpus no tree's
/// result or written text changed. `behavioral` selects the behavioral relation.
fn lift_live_options(behavioral: bool) -> LiveOptions {
    LiveOptions {
        max_pair_checks: 50_000,
        behavioral,
        ..LiveOptions::default()
    }
}

/// A tree `std::globals` declares.
#[derive(Clone, Copy)]
struct StdGlobal {
    name: &'static str,
    params: usize,
    /// Declared `-> Result` (else `-> bool`).
    result: bool,
}

/// The tree `std::globals` declares with this ID.
fn std_global(id: u16) -> Option<StdGlobal> {
    std_global_table().get(&id).copied()
}

/// Every function name `std::globals` declares.
fn std_global_names() -> &'static BTreeSet<&'static str> {
    static NAMES: std::sync::OnceLock<BTreeSet<&'static str>> = std::sync::OnceLock::new();
    NAMES.get_or_init(|| {
        include_str!("std/globals.antics")
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("#[id(")?;
                let (_, rest) = rest.split_once(")]")?;
                let (name, _) = rest.trim().strip_prefix("fn ")?.split_once('(')?;
                Some(name)
            })
            .collect()
    })
}

fn std_global_table() -> &'static BTreeMap<u16, StdGlobal> {
    static TABLE: std::sync::OnceLock<BTreeMap<u16, StdGlobal>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        include_str!("std/globals.antics")
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("#[id(")?;
                let (id, rest) = rest.split_once(")]")?;
                let rest = rest.trim().strip_prefix("fn ")?;
                let (name, rest) = rest.split_once('(')?;
                let (params, ret) = rest.split_once(')')?;
                let params = if params.trim().is_empty() {
                    0
                } else {
                    params.split(',').count()
                };
                let result = match ret.trim() {
                    "-> Result;" => true,
                    "-> bool;" => false,
                    _ => return None,
                };
                Some((
                    id.parse().ok()?,
                    StdGlobal {
                        name,
                        params,
                        result,
                    },
                ))
            })
            .collect()
    })
}

/// Whether `text` has a space outside brackets: a binary expression, which `!` must
/// parenthesize.
fn has_top_level_space(text: &str) -> bool {
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ' ' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

/// `line` with each standalone `bcon[addr]` that `path` names replaced by the name.
fn rewrite_bcons(line: &str, path: impl Fn(u16) -> Option<String>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("bcon[") {
        out.push_str(&rest[..at]);
        let ident_before = out
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
        let inner = &rest[at + "bcon[".len()..];
        let named = inner.find(']').filter(|_| !ident_before).and_then(|end| {
            let addr = inner[..end].parse::<u16>().ok()?;
            Some((path(addr)?, end))
        });
        match named {
            Some((name, end)) => {
                out.push_str(&name);
                rest = &inner[end + 1..];
            }
            None => {
                out.push_str("bcon[");
                rest = inner;
            }
        }
    }
    out.push_str(rest);
    out
}

/// An Expression primitive the lift wrote as `expr(..)` (padded or not): its operands and
/// operator byte.
fn expr_parts(p: &PrimitiveCall) -> Option<(&OperandRef, u8, &OperandRef)> {
    match unpadded(p) {
        PrimitiveCall::ExprRaw { lhs, operator, rhs } => Some((lhs, *operator, rhs)),
        _ => None,
    }
}

fn assign_op_of(code: u8) -> Option<AssignOp> {
    [
        AssignOp::Set,
        AssignOp::Add,
        AssignOp::Sub,
        AssignOp::Mul,
        AssignOp::Div,
        AssignOp::Mod,
        AssignOp::BitAnd,
        AssignOp::BitOr,
        AssignOp::BitXor,
        AssignOp::FlagSet,
        AssignOp::FlagClear,
        AssignOp::Sqrt,
    ]
    .into_iter()
    .find(|op| op.code() == code)
}

fn compare_op_of(code: u8) -> Option<CompareOp> {
    [
        CompareOp::Gt,
        CompareOp::Lt,
        CompareOp::Eq,
        CompareOp::FlagTest,
        CompareOp::IncLt,
        CompareOp::Ge,
        CompareOp::Le,
        CompareOp::Ne,
        CompareOp::DecGt,
    ]
    .into_iter()
    .find(|op| op.code() == code)
}

fn call_params(arity: usize) -> impl Iterator<Item = String> {
    (0..arity).map(|i| format!("arg_{i}: i16"))
}

/// The highest slot that needs a named declaration for a primitive's local arguments;
/// `wave2` adds the operands only the wave 2 spellings name (see [`Translator::wave2`]),
/// and `wave3` (which implies `wave2`) those only the wave 3 ones name.
fn builtin_local(s: &Stmt, wave2: bool, wave3: bool) -> Option<u16> {
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
        // Mode 9 reads a local; the `dest:` of an `if` or a statement may be one. A loop's
        // is not counted: as a `for`, v2 gives the cursor its own slot.
        PrimitiveCall::SetToNext {
            mode, local, dest, ..
        } => {
            let read = match local {
                LocalRef::Index(i) if mode & 0x7F == 9 => Some(u16::from(*i)),
                _ => None,
            };
            let dest = dest
                .as_ref()
                .filter(|_| wave2 && !matches!(s, Stmt::While { .. }))
                .and_then(direct_local)
                .and_then(|v| u16::try_from(v).ok());
            read.into_iter().chain(dest).max()
        }
        PrimitiveCall::Relationship {
            direction,
            local: LocalRef::Index(i),
            value,
            ..
        } => {
            let value = direct_local(value).and_then(|v| u16::try_from(v).ok());
            // Without wave 2 only a local value is spelled.
            if value.is_none() && !wave2 {
                return None;
            }
            let local = (matches!(direction, 2 | 3) || *i != 0).then_some(u16::from(*i));
            value.into_iter().chain(local).max()
        }
        // Modes 8 and 9 read a local.
        PrimitiveCall::CreateObject {
            mode: 8 | 9,
            local: LocalRef::Index(i),
            ..
        } if wave2 => Some(u16::from(*i)),
        // Flag 0x01: the icon is read from a local.
        PrimitiveCall::PushInteraction {
            flags,
            icon_local: LocalRef::Index(i),
            ..
        } if flags & 0x03 == 0x01 && wave2 => Some(u16::from(*i)),
        // Skin, age and sex are read from locals.
        PrimitiveCall::MakeNewCharacter {
            skin: LocalRef::Index(skin),
            age: LocalRef::Index(age),
            sex: LocalRef::Index(sex),
            species,
        } if *species <= 2 && wave3 => [skin, age, sex].into_iter().copied().max().map(u16::from),
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
    /// `init`/`main` trees written as plain `#[bhav] fn`s.
    standalone: BTreeSet<u16>,
    /// Lifecycle blocks written with their whole header as `#[frame(..)]`: those the default
    /// header does not reproduce.
    framed: BTreeSet<u16>,
    /// Why the catalog variants are left out (the compiler rejected them).
    variants_off: Option<String>,
    /// Why the `strings` members are left out (the compiler rejected them).
    strings_off: Option<String>,
    /// Trees written without the extra idioms (see [`Translator::extra`]), after a round
    /// that used them was not proven.
    plain: BTreeSet<u16>,
    /// Trees whose locals are named only where a builtin addresses them, after a round that
    /// named every local was not proven (a pinned word can take the frame word an idiom
    /// needs); tried before [`Self::plain`].
    builtin_locals_only: BTreeSet<u16>,
    /// Raw-level trees whose lines keep `bcon[addr]`, after a round naming the entries did
    /// not compile (see [`Model::name_bcons`]).
    raw_bcons_numbered: BTreeSet<u16>,
    /// Trees some caller runs in the other context: written `#[context(inherit)]` (an extra
    /// idiom) so that either context may call them.
    inherit: BTreeSet<u16>,
    /// Inherit trees [`Self::drop_inherit`] took out: a new order or context tries them
    /// again, as it does the extras.
    inherit_dropped: BTreeSet<u16>,
    /// Tree ID -> the level at which its branch order was last swapped (see
    /// [`Plan::other_order`]).
    swapped: BTreeMap<u16, Level>,
    /// Tree ID -> the locals its header declares, when more than the original's (see
    /// [`Plan::grow_locals`]).
    locals: BTreeMap<u16, u16>,
    /// Trees whose `if` chains are written arm by arm, after a round that merged arms (see
    /// [`Translator::merge_arms`]) did not compile or was not proven.
    unmerged: BTreeSet<u16>,
    /// Trees whose temps calls to trees with arguments stay raw, after a round that named
    /// them (see [`Translator::call`]) did not compile or was not proven: v2 copies the temps
    /// through frame locals, which a dialog's `$Local` tokens may read.
    temps_raw: BTreeSet<u16>,
    /// Trees written without the wave 2 spellings (see [`Translator::wave2`]), after a round
    /// that used them did not compile or was not proven.
    pre_wave2: BTreeSet<u16>,
    /// Trees written without the wave 3 spellings (see [`Translator::wave3`]), after a round
    /// that used them did not compile or was not proven.
    pre_wave3: BTreeSet<u16>,
    /// Trees that keep the original's locals, after a round with more (see
    /// [`Plan::grow_locals`]) was not proven: a dialog's `$Local` tokens read the frame, so
    /// the idiom that needed the words goes raw instead.
    fixed_locals: BTreeSet<u16>,
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
        // A lifecycle block carries any other header as `#[frame(..)]`, but never an empty
        // one (entering a 0-node tree faults; v2 refuses `#[frame(empty)]` there), so a
        // 0-node lifecycle tree starts plain.
        let standalone = model
            .trees
            .iter()
            .filter(|t| t.lifecycle.is_some() && t.original.instructions.is_empty())
            .map(|t| t.id)
            .collect();
        // Formats, kinds and revisions the compiler never gives a block by default start
        // with `#[frame]`; anything else gets one when the default header does not compile or
        // is not proven (see `add_frame`).
        let framed = model
            .trees
            .iter()
            .filter(|t| {
                let b = &t.original;
                t.lifecycle.is_some() && (b.format != 0x8002 || b.kind != 0 || b.revision != 0)
            })
            .map(|t| t.id)
            .collect();
        // A private tree called from a tree of the other context (by the contexts the
        // trees prefer), or from an inherit tree (which, lowered in both contexts, calls
        // only inherit trees); never a lifecycle tree or a variant's init/main. A tree whose
        // body depends on the context fails to compile as inherit and drops back to its own
        // context (`drop_inherit`), keeping its other extras.
        let mut inherit = BTreeSet::new();
        loop {
            let before = inherit.len();
            for caller in &model.trees {
                let Some(body) = &caller.body else {
                    continue;
                };
                let caller_inherits = inherit.contains(&caller.id);
                walk_stmts(body, &mut |s| {
                    if let Some((id, _)) = stmt_call(s)
                        && let Some(callee) = model.tree(id)
                        && (caller_inherits || callee.sim_ctx != caller.sim_ctx)
                        && callee.body.is_some()
                        && callee.lifecycle.is_none()
                        && !model.variants.iter().any(|v| v.init == id || v.main == id)
                    {
                        inherit.insert(id);
                    }
                });
            }
            if inherit.len() == before {
                break;
            }
        }
        Self {
            styles,
            raw_stmts: BTreeMap::new(),
            standalone,
            framed,
            variants_off: None,
            strings_off: None,
            plain: BTreeSet::new(),
            builtin_locals_only: BTreeSet::new(),
            raw_bcons_numbered: BTreeSet::new(),
            inherit,
            inherit_dropped: BTreeSet::new(),
            swapped: BTreeMap::new(),
            locals: BTreeMap::new(),
            unmerged: BTreeSet::new(),
            temps_raw: BTreeSet::new(),
            pre_wave2: BTreeSet::new(),
            pre_wave3: BTreeSet::new(),
            fixed_locals: BTreeSet::new(),
        }
    }

    /// Whether tree `t` is written `#[context(inherit)]` this round.
    fn inherits(&self, t: &TreeModel) -> bool {
        self.inherit.contains(&t.id) && !self.plain.contains(&t.id) && self.level(t).idiomatic()
    }

    /// In behavioral mode, declares the locals tree `id`'s idioms need when the compiler
    /// says the header has too few (`let` copies of call arguments, a builtin's saved
    /// cursor). Frame storage is not compared there; v2 needs the words. Not past the
    /// `frame_headroom` limit unless the original is already beyond it, and not for an
    /// argless tree without locals (the checker's frame-shape rule, `verify/live.rs`).
    fn grow_locals(&mut self, model: &Model, id: u16, message: &str) -> bool {
        let Some(needed) = message
            .split_once(LOCALS_SHORT)
            .and_then(|(_, n)| n.trim().parse::<u16>().ok())
        else {
            return false;
        };
        self.grow_locals_to(model, id, needed)
    }

    /// Gives tree `id`'s header `needed` locals, when that is more than it declares, within
    /// the cap. `false` when it did not.
    fn grow_locals_to(&mut self, model: &Model, id: u16, needed: u16) -> bool {
        let Some(t) = model
            .tree(id)
            .filter(|_| model.behavioral && !self.fixed_locals.contains(&id))
        else {
            return false;
        };
        // Create New Object Instance writes an argless tree's first local, if it has one,
        // so the checker refuses a frame that gains its first local there.
        if t.original.args == 0 && t.original.locals == 0 {
            return false;
        }
        let declared = self.locals.get(&id).copied().unwrap_or(t.original.locals);
        let limit = t.original.locals.max(crate::v2::lint::FRAME_LOCAL_WARN);
        if needed <= declared || needed > limit {
            return false;
        }
        self.locals.insert(id, needed);
        true
    }

    /// Gives tree `id` the original's locals from now on, when it had more. `false` when it
    /// did not.
    fn fix_locals(&mut self, id: u16) -> bool {
        self.fixed_locals.insert(id);
        self.locals.remove(&id).is_some()
    }

    /// The locals tree `t`'s header declares.
    fn locals(&self, t: &TreeModel) -> u16 {
        self.locals.get(&t.id).copied().unwrap_or(t.original.locals)
    }

    /// The next step for tree `id`, which a round's `.air` build failed on with `message`.
    /// `false` when it has none left but being left out.
    fn air_fallback(
        &mut self,
        model: &Model,
        generated: &Generated,
        id: u16,
        message: &str,
    ) -> bool {
        self.grow_locals(model, id, message)
            || self.add_frame(model, id)
            || self.keep_temps_raw(id, generated)
            || self.unmerge(id, generated)
            || self.drop_wave3(id, generated)
            || self.drop_wave2(id, generated)
            || self.name_builtin_locals_only(id, generated)
            || self.drop_inherit(model, id)
            || self.drop_extras(id, generated)
            || self.number_raw_bcons(id, generated)
            || self.lower_tree(model, id)
    }

    /// Writes lifecycle block `id` with its whole header as `#[frame(..)]` from now on, when
    /// it is still written with the default one. Tried before anything else: the header is
    /// the original's, so it only ever helps.
    fn add_frame(&mut self, model: &Model, id: u16) -> bool {
        model.tree(id).is_some_and(|t| t.lifecycle.is_some())
            && !self.standalone.contains(&id)
            && self.framed.insert(id)
    }

    /// Writes tree `id` in its own context from now on, when the last round wrote it
    /// inherit. Tried before its other extras: a callee the closure made inherit may depend
    /// on the context, and its other idioms may still hold in its own.
    fn drop_inherit(&mut self, model: &Model, id: u16) -> bool {
        model.tree(id).is_some_and(|t| self.inherits(t))
            && self.inherit.remove(&id)
            && self.inherit_dropped.insert(id)
    }

    /// Gives tree `id` its extras, inherit included, another try.
    fn retry_extras(&mut self, id: u16) {
        self.plain.remove(&id);
        if self.inherit_dropped.remove(&id) {
            self.inherit.insert(id);
        }
    }

    /// Writes tree `id`'s `if` chains arm by arm from now on, when the last round merged
    /// arms. `false` when it did not.
    fn unmerge(&mut self, id: u16, generated: &Generated) -> bool {
        generated.merged.contains(&id) && self.unmerged.insert(id)
    }

    /// Writes tree `id`'s temps calls to trees with arguments raw from now on, with the
    /// original's locals, when the last round named them. `false` when it did not.
    fn keep_temps_raw(&mut self, id: u16, generated: &Generated) -> bool {
        let changed = generated.temps_named.contains(&id) && self.temps_raw.insert(id);
        if changed {
            self.locals.remove(&id);
        }
        changed
    }

    /// Writes tree `id` without the wave 2 spellings from now on, with the original's
    /// locals, when the last round used them. `false` when it did not. Tried before the
    /// older extras are dropped, so a new spelling that breaks a tree costs it no more
    /// than the new spellings.
    fn drop_wave2(&mut self, id: u16, generated: &Generated) -> bool {
        let changed = generated.wave2.contains(&id) && self.pre_wave2.insert(id);
        if changed {
            self.locals.remove(&id);
            self.fixed_locals.insert(id);
        }
        changed
    }

    /// Writes tree `id` without the wave 3 spellings from now on, when the last round used
    /// them. `false` when it did not. Tried before the wave 2 spellings are dropped, so a
    /// wave 3 spelling that breaks a tree costs it no more than the wave 3 spellings.
    fn drop_wave3(&mut self, id: u16, generated: &Generated) -> bool {
        generated.wave3.contains(&id) && self.pre_wave3.insert(id)
    }

    /// [`Self::drop_wave3`] for every tree, for a failure no one tree is blamed for.
    fn drop_all_wave3(&mut self, generated: &Generated) -> bool {
        let mut changed = false;
        for &id in &generated.wave3 {
            changed |= self.drop_wave3(id, generated);
        }
        changed
    }

    /// [`Self::drop_wave2`] for every tree, for a failure no one tree is blamed for.
    fn drop_all_wave2(&mut self, generated: &Generated) -> bool {
        let mut changed = false;
        for &id in &generated.wave2 {
            changed |= self.drop_wave2(id, generated);
        }
        changed
    }

    /// Names tree `id`'s locals only where builtins address them from now on, when the last
    /// round named others. `false` when it did not.
    fn name_builtin_locals_only(&mut self, id: u16, generated: &Generated) -> bool {
        generated.named_other_locals.contains(&id) && self.builtin_locals_only.insert(id)
    }

    /// Writes raw-level tree `id`'s constants as `bcon[addr]` from now on, when the last
    /// round named them. `false` when it did not.
    fn number_raw_bcons(&mut self, id: u16, generated: &Generated) -> bool {
        generated.raw_bcons_named.contains(&id) && self.raw_bcons_numbered.insert(id)
    }

    /// [`Self::number_raw_bcons`] for every tree, for a failure no one tree is blamed for.
    fn number_all_raw_bcons(&mut self, generated: &Generated) -> bool {
        let before = self.raw_bcons_numbered.len();
        self.raw_bcons_numbered
            .extend(generated.raw_bcons_named.iter().copied());
        self.raw_bcons_numbered.len() != before
    }

    /// Turns off tree `id`'s extra idioms when the last round used them. `false` when it
    /// did not, so the tree has to move down a style instead.
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

    /// Tries tree `id`'s other branch order at its current level, once per level: node
    /// order usually keeps the original's node numbers, but a tree whose paths visit
    /// nodes out of order can prove only in the other. The extra idioms and the statements
    /// written raw start over, since the statements are numbered differently.
    fn other_order(&mut self, model: &mut Model, id: u16) -> bool {
        let Some(t) = model.tree(id) else {
            return false;
        };
        let level = self.level(t);
        // In behavioral mode node numbers are not compared, so the order cannot matter.
        if model.behavioral
            || t.other_order.is_none()
            || level == Level::RawLabeled
            || self.swapped.get(&id) == Some(&level)
        {
            return false;
        }
        model.swap_order(id);
        self.swapped.insert(id, level);
        self.retry_extras(id);
        self.raw_stmts.remove(&id);
        true
    }

    /// Moves tree `id` one style down. `false` when it is already at its last style.
    fn lower_tree(&mut self, model: &Model, id: u16) -> bool {
        let Some(t) = model.tree(id) else {
            return false;
        };
        let next = match self.level(t) {
            Level::Idiomatic if t.sim_ctx => {
                // The extra idioms get another try in the object's context.
                self.retry_extras(id);
                Level::IdiomaticObject
            }
            Level::Idiomatic | Level::IdiomaticObject if t.structured => Level::Raw,
            Level::Idiomatic | Level::IdiomaticObject | Level::Raw => Level::RawLabeled,
            Level::RawLabeled if t.lifecycle.is_some() && !self.standalone.contains(&id) => {
                // Start again as a plain tree, which keeps the header.
                self.standalone.insert(id);
                self.raw_stmts.remove(&id);
                self.swapped.remove(&id);
                if t.body.is_some() {
                    Level::Idiomatic
                } else {
                    Level::RawLabeled
                }
            }
            Level::RawLabeled => return false,
        };
        // Raw code declares the original's locals: only idioms needed more.
        if !next.idiomatic() {
            self.locals.remove(&id);
        }
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

/// What the `.air` build says of a tree header with too few locals, between the count it
/// declares and the count it requires.
const LOCALS_SHORT: &str = " locals but requires at least ";

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
    /// Trees that named a local no builtin addresses (see [`Plan::name_builtin_locals_only`]).
    named_other_locals: BTreeSet<u16>,
    /// Trees with merged `if` arms (see [`Translator::merge_arms`]).
    merged: BTreeSet<u16>,
    /// Trees that named a temps call to a tree with arguments (see [`Plan::keep_temps_raw`]).
    temps_named: BTreeSet<u16>,
    /// Trees that wrote a wave 2 spelling (see [`Plan::drop_wave2`]).
    wave2: BTreeSet<u16>,
    /// Trees that wrote a wave 3 spelling (see [`Plan::drop_wave3`]).
    wave3: BTreeSet<u16>,
    /// Raw-level trees whose lines name BCON entries.
    raw_bcons_named: BTreeSet<u16>,
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
        let mut named_other_locals = BTreeSet::new();
        let mut merged = BTreeSet::new();
        let mut temps_named = BTreeSet::new();
        let mut wave2_trees = BTreeSet::new();
        let mut wave3_trees = BTreeSet::new();
        let mut raw_bcons_named = BTreeSet::new();
        out.line("// Decompiled by `sims-antics lift --v2`.");
        if self.behavioral {
            for line in BEHAVIORAL_HEADER {
                out.line(line);
            }
        }
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
            } else if id >= 256 && !self.std_globals.contains(&id) {
                lib_globals.push(id);
            }
        }
        if !self.std_globals.is_empty() {
            out.line("use std::globals::*;");
            out.line("");
        }
        for (module, ids) in [
            (self.globals_module(), &lib_globals),
            ("semi_globals", &lib_semis),
        ] {
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
                out.line(format!(
                    "#[id({id})] fn {ident}({params}) -> bool;{comment}"
                ));
            }
            out.indent -= 1;
            out.line("}");
            out.line("");
        }
        if !self.extern_bcons.is_empty() {
            out.line("extern bcon library_consts {");
            out.indent += 1;
            for (addr, name) in &self.extern_bcons {
                out.line(format!("#[addr({addr})] {name};"));
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
            out.line(format!("#[slot({s})] state attr_{s}: i16;"));
        }
        if !self.sounds.is_empty() {
            out.line("");
        }
        for (id, sound) in &self.sounds {
            out.line(format!(
                "sound {} = fwav({id}, wav: \"{}\");",
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
                    t.id,
                    clean_label(&t.label)
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
                    out.line(format!("#[id({})]", t.id));
                    write_frame_allow(&mut out, &t.original);
                    out.line(format!("{} {{", kind.keyword()));
                }
                Some(kind) => {
                    // The block keeps the tree's own header, as a plain tree does.
                    let b = &t.original;
                    let frame_kind = if b.kind == 0 {
                        String::new()
                    } else {
                        format!(", kind = {}", b.kind)
                    };
                    out.line(format!(
                        "#[bhav(id = {}, args = {}, locals = {}, format = {:#06x}, revision = {}{frame_kind})]",
                        t.id,
                        b.args,
                        plan.locals(t),
                        b.format,
                        b.revision,
                    ));
                    write_frame_allow(&mut out, b);
                    out.line(format!("{} {{", kind.keyword()));
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
                    let mut kind = if b.kind == 0 {
                        String::new()
                    } else {
                        format!(", kind = {}", b.kind)
                    };
                    // A 0-node tree (entering it faults) is `empty` with a `{}` body.
                    if b.instructions.is_empty() {
                        kind.push_str(", empty");
                    }
                    let context = if plan.inherits(t) {
                        extras.insert(t.id);
                        ", context = inherit"
                    } else {
                        ""
                    };
                    let label = if t.label == t.ident {
                        String::new()
                    } else {
                        format!(", label = \"{}\"", v2_string_literal(&t.label))
                    };
                    out.line(format!(
                        "#[bhav(id = {}{label}, args = {}, locals = {}, format = {:#06x}, revision = {}{kind}{context})]",
                        t.id,
                        b.args,
                        plan.locals(t),
                        b.format,
                        b.revision,
                    ));
                    write_frame_allow(&mut out, b);
                    out.line(format!("fn {}({}) {{", t.ident, params.join(", ")));
                }
            }
            out.indent += 1;
            let mut raw_count = 0;
            match &t.body {
                Some(body) if level.idiomatic() => {
                    let mut named_locals = 0;
                    if as_lifecycle.is_none() && !plan.plain.contains(&t.id) {
                        let mut builtin_locals = 0;
                        let wave2 = !plan.pre_wave2.contains(&t.id);
                        let wave3 = wave2 && !plan.pre_wave3.contains(&t.id);
                        walk_stmts(body, &mut |s| {
                            let local = builtin_local(s, wave2, wave3);
                            // A local only a wave 2 (wave 3) spelling names is that
                            // spelling's, even when the spelling is not written:
                            // `drop_wave2` (`drop_wave3`) drops the pin.
                            if wave3 && local != builtin_local(s, wave2, false) {
                                wave3_trees.insert(t.id);
                            }
                            if wave2
                                && builtin_local(s, wave2, false) != builtin_local(s, false, false)
                            {
                                wave2_trees.insert(t.id);
                            }
                            if let Some(index) = local
                                && index < t.original.locals
                            {
                                builtin_locals = builtin_locals.max(index + 1);
                            }
                            // Every local the body reads or writes gets a pinned name.
                            for op in stmt_operands(s) {
                                if let OperandRef::Direct(o) = op
                                    && o.scope == 25
                                    && let Ok(index) = u16::try_from(o.index)
                                    && index < t.original.locals
                                {
                                    named_locals = named_locals.max(index + 1);
                                }
                            }
                        });
                        if plan.builtin_locals_only.contains(&t.id)
                            || named_locals <= builtin_locals
                        {
                            named_locals = builtin_locals;
                        } else {
                            named_other_locals.insert(t.id);
                        }
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
                        merged: false,
                        temps_named: std::cell::Cell::new(false),
                        wave2_used: std::cell::Cell::new(false),
                        wave3_used: std::cell::Cell::new(false),
                        stack_moves: sim && assigns_stack_obj(body),
                    };
                    if named_locals > 0 && tr.extra() {
                        // `#[slot]` pins each word the builtins address by number; the
                        // build keeps a pinned word in the frame even when nothing names it.
                        for i in 0..named_locals {
                            out.line(format!("#[slot({i})] let slot_{i}: i16;"));
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
                    if tr.merged {
                        merged.insert(t.id);
                    }
                    if tr.temps_named.get() {
                        temps_named.insert(t.id);
                    }
                    if tr.wave2_used.get() {
                        wave2_trees.insert(t.id);
                    }
                    if tr.wave3_used.get() {
                        wave3_trees.insert(t.id);
                    }
                }
                _ => {
                    let mut lines = if level == Level::Raw {
                        let (head, close) = t.body_lines;
                        let lines: Vec<&str> = t.lifted[head + 1..close.min(t.lifted.len())]
                            .iter()
                            .map(String::as_str)
                            .collect();
                        dedent(&lines)
                    } else {
                        t.labeled.clone()
                    };
                    // The names compile to the same addresses, so only a compile failure (or a
                    // tree with no lower style left) turns them off.
                    if !plan.raw_bcons_numbered.contains(&t.id)
                        && lines.iter().any(|l| l.contains("bcon["))
                    {
                        let named: Vec<String> = lines.iter().map(|l| self.name_bcons(l)).collect();
                        if named != lines {
                            raw_bcons_named.insert(t.id);
                            lines = named;
                        }
                    }
                    write_raw_block(&mut out, &lines);
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
            named_other_locals,
            merged,
            temps_named,
            wave2: wave2_trees,
            wave3: wave3_trees,
            raw_bcons_named,
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

/// Allows `frame_headroom` on a tree whose original already has more nodes or locals than
/// the lint asks for: a faithful lift keeps the original's size. Written between the
/// tree's attribute and its header line, which the lint reports.
fn write_frame_allow(out: &mut Out, original: &BhavChunk) {
    if original.instructions.len() > crate::v2::lint::FRAME_NODE_WARN
        || original.locals > crate::v2::lint::FRAME_LOCAL_WARN
    {
        out.line("// antics-lint: allow(frame_headroom): the original tree is this size");
    }
}

fn write_raw_block(out: &mut Out, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    out.line("raw {");
    out.indent += 1;
    for l in lines {
        out.line(explicit_false_exit(l));
    }
    out.indent -= 1;
    out.line("}");
}

/// The value a body that is just `return true;` or `return false;` returns.
/// Whether `text` is one call, `path(args)` (`globals::f(1)`, `sim.debit_funds(5)`), so
/// that `text?` applies `?` to the call. A bitflags test (`x.is_empty()`) reads oddly with
/// `?` and is left alone.
/// `.air` text with the `locals:` count left out of every tree header
/// (`tree ID "label" (.., locals: N, ..) {`), and the count each header declared.
fn without_locals(air: &str) -> (String, BTreeMap<u16, u16>) {
    let mut out = String::with_capacity(air.len());
    let mut declared = BTreeMap::new();
    for line in air.split_inclusive('\n') {
        let header = line
            .trim_start()
            .strip_prefix("tree ")
            .and_then(|rest| rest.split_once(' '))
            .and_then(|(id, _)| id.parse::<u16>().ok())
            .and_then(|id| Some((id, line.rfind(" (")?)))
            .and_then(|(id, open)| {
                let head = &line[open..];
                let start = head.find("locals: ")?;
                let digits = head[start + 8..]
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(head.len() - start - 8);
                let end = start + 8 + digits;
                declared.insert(id, head[start + 8..end].parse::<u16>().ok()?);
                // The field with its separator: `, locals: N` or `locals: N, `.
                let (cut_start, cut_end) = if head[..start].ends_with(", ") {
                    (start - 2, end)
                } else if head[end..].starts_with(", ") {
                    (start, end + 2)
                } else {
                    (start, end)
                };
                Some(format!(
                    "{}{}{}",
                    &line[..open],
                    &head[..cut_start],
                    &head[cut_end..]
                ))
            });
        out.push_str(header.as_deref().unwrap_or(line));
    }
    (out, declared)
}

fn is_call_text(text: &str) -> bool {
    let Some(open) = text.find('(') else {
        return false;
    };
    let path = &text[..open];
    if path.is_empty()
        || !path.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        || !path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '.'))
        || path.ends_with("is_empty")
        || !text.ends_with(')')
    {
        return false;
    }
    // The `(` after the path closes at the very end.
    let mut depth = 0usize;
    for (i, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return open + i + 1 == text.len();
                }
            }
            '"' => return false,
            _ => {}
        }
    }
    false
}

fn single_return(body: &[Stmt]) -> Option<bool> {
    match body {
        [Stmt::Return { target, .. }] => match target {
            Target::ReturnTrue => Some(true),
            Target::ReturnFalse => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// A bare `call N(..);` line in AIR exits the tree false when the call returns false; it
/// is written `if !call N(..) { return false; }`, the same node with the exit spelled out
/// (lint `raw_bare_call_false_exit` asks for it before v2 code).
fn explicit_false_exit(line: &str) -> Cow<'_, str> {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    match body.strip_suffix(';') {
        Some(call) if call.starts_with("call ") && call.ends_with(')') && !call.contains("->") => {
            Cow::Owned(format!("{indent}if !{call} {{ return false; }}"))
        }
        _ => Cow::Borrowed(line),
    }
}

struct Translator<'a> {
    model: &'a Model,
    plan: &'a Plan,
    tree: &'a TreeModel,
    sim: bool,
    /// `#[context(inherit)]`: the runner's cells are `me.*` and the stack object's
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
    /// Whether `if` arms were merged (see [`Translator::merge_arms`]).
    merged: bool,
    /// Whether a temps call to a tree with arguments was named (see [`Plan::keep_temps_raw`]).
    temps_named: std::cell::Cell<bool>,
    /// Whether a wave 2 spelling was written (see [`Translator::wave2`]).
    wave2_used: std::cell::Cell<bool>,
    /// Whether a wave 3 spelling was written (see [`Translator::wave3`]).
    wave3_used: std::cell::Cell<bool>,
    /// A Sim tree that assigns `stack_obj`: the stack object's cells are written
    /// `stack_obj.attr[i]` / `stack_obj.slot[i]` (the same operands), since `self` would
    /// name another object after the assignment (lint `stack_obj_clobber_after_self`).
    stack_moves: bool,
}

/// The person word `fame` names.
const FAME: i16 = 81;

/// Clear Motive Changes' operands (opcode 29), as the lowering writes them.
const CLEAR_MOTIVE_CHANGES: [u8; 8] = [0, 0, 0, 1, 0, 0, 0, 0];

/// PR I-4's `padN: 0xNN` options: one per operand byte that differs from what the call
/// writes without them, in byte order.
fn pad_options(operands: &[u8; 8], base: &[u8; 8]) -> Vec<String> {
    (0..8)
        .filter(|&i| operands[i] != base[i])
        .map(|i| format!("pad{i}: {:#04x}", operands[i]))
        .collect()
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

    /// [`Self::extra`] for the wave 2 spellings (`?`, joined arms, `.remove()`,
    /// `set_to_next` into a cell, `find_best_object` and dialogs as conditions, the
    /// definition and job cells, `create_object`, icon locals, relationship values, motive
    /// changes, named temps calls), noting that one is written. A tree one of them breaks is
    /// written without all of them ([`Plan::drop_wave2`]) before it loses the older extras.
    fn wave2(&self) -> bool {
        if self.plan.pre_wave2.contains(&self.tree.id) || !self.extra() {
            return false;
        }
        self.wave2_used.set(true);
        true
    }

    /// Whether this tree may use the wave 3 spellings (the temp-indexed, neighbor and
    /// advertisement cells PR I-4 named), noting that one is written. They sit under the
    /// wave 2 switch: a tree one of them breaks is written without them
    /// ([`Plan::drop_wave3`]) before it loses the wave 2 spellings.
    fn wave3(&self) -> bool {
        if self.plan.pre_wave3.contains(&self.tree.id) || !self.wave2() {
            return false;
        }
        self.wave3_used.set(true);
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
        let mut lines = self.lifted_lines(s);
        if lines.iter().any(|l| l.contains("bcon[")) && self.extra() {
            lines = lines.iter().map(|l| self.model.name_bcons(l)).collect();
        }
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
                // `if !f(..) { return error; }` is `f(..)?;`: v2 lowers both to the same test
                // with the same error exit. With the other arm a plain return,
                // `if f(..) { return X; } else { return error; }` is `f(..)?; return X;`.
                let is_error = |body: &[Stmt]| {
                    matches!(
                        body,
                        [Stmt::Return {
                            target: Target::Error,
                            ..
                        }]
                    )
                };
                let after = if else_body.is_empty() && is_error(then_body) {
                    Some((true, None))
                } else if is_error(then_body) {
                    single_return(else_body).map(|r| (true, Some(r)))
                } else if is_error(else_body) {
                    single_return(then_body).map(|r| (false, Some(r)))
                } else {
                    None
                };
                if let Some((negated, then)) = after
                    && !self.in_handler
                    && (id + 1..id + count_stmts(std::slice::from_ref(s)))
                        .all(|c| !self.forced_raw(c))
                    && let Some(c) = self.cond(cond)
                    && let Some(call) = (if negated {
                        c.strip_prefix('!')
                    } else {
                        Some(c.as_str())
                    })
                    .filter(|t| is_call_text(t))
                    && self.wave2()
                {
                    self.flush_raw(out);
                    let start = out.lines.len();
                    out.line(format!("{call}?;"));
                    if let Some(r) = then {
                        out.line(format!("return {r};"));
                    }
                    skip_children(self);
                    self.record(id, start, out);
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
                if !self.merge_arms(id, start, out) {
                    self.constant_ladder(id, start, out);
                }
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
                // A primitive whose true branch is itself: the original's own loop (a
                // breakpoint trap, for one), which runs until the VM errors the object out.
                // A comparison with both branches back to itself (`if dec_gt(..) {}`) loops
                // the same way; a comparison never yields.
                let self_compare = |c: &Condition| match c {
                    Condition::Compare { .. } => true,
                    Condition::Not(c) => matches!(c.as_ref(), Condition::Compare { .. }),
                    _ => false,
                };
                let allow = match body.as_slice() {
                    [
                        Stmt::Primitive {
                            prim,
                            branches: None,
                            ..
                        },
                    ] => {
                        out.line(if matches!(unpadded(prim), PrimitiveCall::Breakpoint { .. }) {
                            "// antics-lint: allow(unbounded_loop_without_yield): the original's breakpoint loops on itself"
                        } else {
                            "// antics-lint: allow(unbounded_loop_without_yield): the original's node loops on itself"
                        });
                        Some(out.lines.len() - 1)
                    }
                    [
                        Stmt::If {
                            cond,
                            then_body,
                            else_body,
                            ..
                        },
                    ] if then_body.is_empty() && else_body.is_empty() && self_compare(cond) => {
                        out.line(
                            "// antics-lint: allow(unbounded_loop_without_yield): the original's node loops on itself",
                        );
                        Some(out.lines.len() - 1)
                    }
                    _ => None,
                };
                let start = out.lines.len();
                out.line("loop {");
                out.indent += 1;
                self.loop_depth += 1;
                self.block(body, out);
                self.loop_depth -= 1;
                self.flush_raw(out);
                out.indent -= 1;
                out.line("}");
                // Lint does not look inside a raw body, so an allow over one suppresses
                // nothing (`stale_lint_allow`): it stays a plain comment, on the same line
                // (statement ranges count lines).
                if let Some(at) = allow
                    && out.lines[at + 1..].iter().any(|l| l.trim() == "raw {")
                {
                    out.lines[at] = out.lines[at].replacen(
                        "antics-lint: allow(unbounded_loop_without_yield): the",
                        "The",
                        1,
                    );
                }
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
    /// the same exits (so the same last result). Also a primitive test or a tree call
    /// returned whole (`return can_afford(..);`, `return !f();`), one node either way.
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
                    | CompareOp::Ne
                    | CompareOp::FlagTest,
                ..
            }
        );
        // A call returns its result as one node (`if call N { return true; } return false;`),
        // but a `-> Result` one is no `bool` to return.
        let call = match cond {
            Condition::Call {
                target: CallTarget::Id(callee),
                ..
            } => {
                !std_global(*callee).is_some_and(|g| g.result)
                    || !self.model.std_globals.contains(callee)
            }
            _ => false,
        };
        if !compare && !call && !matches!(cond, Condition::Primitive { .. }) {
            return false;
        }
        let (Some(on_true), Some(on_false)) = (single_return(then_body), single_return(else_body))
        else {
            return false;
        };
        if on_true == on_false || self.forced_raw(id + 1) || self.forced_raw(id + 2) {
            return false;
        }
        let Some(c) = self.cond(cond) else {
            return false;
        };
        self.flush_raw(out);
        let start = out.lines.len();
        if on_true != negated {
            out.line(format!("return {c};"));
        } else if compare {
            out.line(format!("return !({c});"));
        } else {
            out.line(format!("return !{c};"));
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
        let Condition::Primitive { prim, .. } = cond else {
            return false;
        };
        let Some(dialog) = self.dialog(prim) else {
            return false;
        };
        self.flush_raw(out);
        let start = out.lines.len();
        out.line(format!("{dialog}?;"));
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

    /// A dialog, `dialog(kind: .., message: .., title: ..)`: true for its first button.
    fn dialog(&self, prim: &PrimitiveCall) -> Option<String> {
        let PrimitiveCall::Dialog {
            kind,
            message,
            title,
            button1,
            button2,
            button3,
            mode4_str,
            flags,
        } = prim
        else {
            return None;
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
                return None;
            };
            if *index != 0 || matches!(name, "message" | "title") {
                fields.push(format!("{name}: {index}"));
            }
        }
        if *flags != 0 {
            fields.push(format!("flags: {flags}"));
        }
        self.extra()
            .then(|| format!("dialog({})", fields.join(", ")))
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
            // An `if` that only returns its condition is left to `else { return c; }`.
            && !(single_return(inner_then).is_some()
                && single_return(inner_else).is_some_and(|f| Some(!f) == single_return(inner_then)))
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

    /// Merges the arms of the `if` chain at lines `start..`, in place:
    /// - adjacent `if a { X } else if b { X }` arms with the same body become
    ///   `if a || b { X }` (`||` tests `b` only when `a` fails, as the chain does);
    /// - statements every arm ends with, when the chain has a final `else`, move after it:
    ///   each path runs them after its arm, as before.
    ///
    /// v2 lowers either to one copy where the lift wrote one per arm, so the tree is
    /// retried arm by arm ([`Plan::unmerge`]) when it is then not proven. Returns whether
    /// it rewrote the chain.
    fn merge_arms(&mut self, id: usize, start: usize, out: &mut Out) -> bool {
        if self.plan.unmerged.contains(&self.tree.id) {
            return false;
        }
        let lines = &out.lines[start..];
        let Some(head) = lines.first() else {
            return false;
        };
        let indent = head.len() - head.trim_start().len();
        let Some(first) = head
            .trim_start()
            .strip_prefix("if ")
            .and_then(|h| h.strip_suffix(" {"))
        else {
            return false;
        };
        // (condition, body lines) per arm; `None` is the final `else`.
        let mut arms: Vec<(Option<String>, Vec<String>)> =
            vec![(Some(first.to_owned()), Vec::new())];
        let mut closed = false;
        for l in &lines[1..] {
            if closed {
                return false;
            }
            if l.len() - l.trim_start().len() > indent || l.trim().is_empty() {
                arms.last_mut().expect("an arm").1.push(l.clone());
                continue;
            }
            let t = l.trim_start();
            if t == "}" {
                closed = true;
            } else if t == "} else {" {
                arms.push((None, Vec::new()));
            } else if let Some(c) = t
                .strip_prefix("} else if ")
                .and_then(|h| h.strip_suffix(" {"))
            {
                arms.push((Some(c.to_owned()), Vec::new()));
            } else {
                return false;
            }
        }
        if !closed
            || arms.iter().any(|(c, body)| {
                body.is_empty() || c.as_deref().is_some_and(|c| c.starts_with("let "))
            })
        {
            return false;
        }
        let mut changed = false;
        // `if a { X } else if b { X }` -> `if a || b { X }`.
        let mut k = 0;
        while k + 1 < arms.len() {
            if arms[k].1 == arms[k + 1].1 && arms[k + 1].0.is_some() {
                let (next, _) = arms.remove(k + 1);
                let cond = arms[k].0.as_mut().expect("a tested arm");
                *cond = format!("{cond} || {}", next.expect("a tested arm"));
                changed = true;
            } else {
                k += 1;
            }
        }
        // The statements every arm ends with move after the chain.
        let arm_indent = indent + 4;
        // A statement starts at its first line, or at the comments just before it (a lint
        // `allow` belongs to the statement it precedes); a comment is not a statement.
        let starts = |body: &[String]| -> Vec<usize> {
            let mut out = Vec::new();
            let mut comments: Option<usize> = None;
            for (i, l) in body.iter().enumerate() {
                let t = l.trim_start();
                if l.len() - t.len() != arm_indent || t.starts_with('}') || t.is_empty() {
                    continue;
                }
                if t.starts_with("//") {
                    comments.get_or_insert(i);
                } else {
                    out.push(comments.take().unwrap_or(i));
                }
            }
            out
        };
        let mut tail: Vec<String> = Vec::new();
        if arms.len() > 1
            && arms.last().is_some_and(|a| a.0.is_none())
            // A `let` an arm declares may be what its tail reads.
            && !arms.iter().any(|(_, body)| {
                body.iter().any(|l| {
                    l.len() - l.trim_start().len() == arm_indent
                        && (l.trim_start().starts_with("let ")
                            || l.trim_start().starts_with("const "))
                })
            })
        {
            let all_starts: Vec<Vec<usize>> = arms.iter().map(|(_, b)| starts(b)).collect();
            // Common trailing statements, compared as text.
            let mut n = 0;
            'grow: loop {
                let next = n + 1;
                let mut text: Option<&[String]> = None;
                for ((_, body), st) in arms.iter().zip(&all_starts) {
                    // Every arm keeps at least one statement of its own.
                    if st.len() <= next {
                        break 'grow;
                    }
                    let from = st[st.len() - next];
                    let slice = &body[from..];
                    match text {
                        None => text = Some(slice),
                        Some(t) if t == slice => {}
                        Some(_) => break 'grow,
                    }
                }
                n = next;
            }
            if n > 0 {
                let (_, body) = &arms[0];
                let from = all_starts[0][all_starts[0].len() - n];
                tail = body[from..]
                    .iter()
                    .map(|l| l.strip_prefix("    ").unwrap_or(l).to_owned())
                    .collect();
                for ((_, body), st) in arms.iter_mut().zip(&all_starts) {
                    body.truncate(st[st.len() - n]);
                }
                changed = true;
            }
        }
        if !changed || !self.wave2() {
            return false;
        }
        let pad = " ".repeat(indent);
        let mut rewritten = Vec::new();
        for (k, (cond, body)) in arms.into_iter().enumerate() {
            rewritten.push(match (k, cond) {
                (0, Some(c)) => format!("{pad}if {c} {{"),
                (_, Some(c)) => format!("{pad}}} else if {c} {{"),
                (_, None) => format!("{pad}}} else {{"),
            });
            rewritten.extend(body);
        }
        rewritten.push(format!("{pad}}}"));
        rewritten.extend(tail);
        out.lines.truncate(start);
        out.lines.extend(rewritten);
        // The statements inside are one statement now: a failure there writes the `if` raw.
        self.ranges
            .retain(|r| r.tree != self.tree.id || r.start < start);
        self.record(id, start, out);
        self.merged = true;
        true
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
                l.contains("raw {") || t.starts_with("#[") || t.starts_with("let ") || t.is_empty()
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
                let token = |p: Option<usize>| p.map(|p| &a[ta[p].clone()]);
                let mut before = token(n.checked_sub(1));
                let after = ta.get(n + 1).map(|r| &a[r.clone()]);
                // A sign (a `-` after no operand) belongs to the value, in both arms.
                let operand = |t: &str| {
                    t.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                        || matches!(t, ")" | "]" | "\"")
                };
                let signed = before == Some("-")
                    && token(n.checked_sub(2)).is_none_or(|t| !operand(t))
                    && token(n.checked_sub(2)) != Some("@");
                let (mut span, mut x, mut y) = (ra.clone(), x.to_owned(), y.to_owned());
                if signed {
                    span.start = ta[n - 1].start;
                    (x, y) = (format!("-{x}"), format!("-{y}"));
                    before = token(n.checked_sub(2));
                }
                // A value operand only: not an index, a field, a subtrahend, a pattern or
                // a named argument's key.
                if !value(&a[ra.clone()])
                    || !value(&b[rb.clone()])
                    || matches!(before, Some("[" | "." | "-" | "@"))
                    || matches!(after, Some("=" | ":" | "[" | "."))
                {
                    return;
                }
                holes.push((k, span, x, y));
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
    ///   `set_dynamic_sprite(index: i, value: v)`;
    /// - `param[0] := n; sleep(param[0])` as `sleep(n)`.
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
        } else if let Some(ticks) = set(first, 9, 0)
            && let Stmt::Primitive {
                prim: PrimitiveCall::Sleep { param_index: 0 },
                branches: None,
                ..
            } = second
        {
            let ticks = self.rvalue(ticks)?;
            if ticks.contains("param[") {
                return None;
            }
            format!("sleep({ticks});")
        } else if let Stmt::Call {
            target: CallTarget::Id(callee),
            args,
            pass_temps,
            branches: None,
            ..
        } = first
            && let Stmt::Return {
                target: Target::ReturnTrue,
                ..
            } = second
            && !self.in_handler
        {
            // A standalone call exits false on failure, so with `return true;` after it the
            // tree returns the call's result: the same node as `return f();`.
            format!("return {};", self.call(*callee, args, *pass_temps)?)
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
                    // On a bitflags place or any other `i16` cell (whose bit number v2
                    // passes through).
                    AssignOp::FlagSet | AssignOp::FlagClear if self.extra() => {
                        if *op == AssignOp::FlagSet {
                            "flag_set"
                        } else {
                            "flag_clear"
                        }
                    }
                    _ => assign_operator(op)?,
                };
                let (lhs, mut rhs) = (self.lvalue(lhs)?, self.rvalue(rhs)?);
                // `stack_obj` takes a handle: an object's data word 11 is its id.
                if lhs == "stack_obj"
                    && op == "="
                    && let Some(base) = rhs.strip_suffix(".data[11]")
                    && matches!(base, "self" | "sim" | "stack_obj" | "me")
                    && self.extra()
                {
                    rhs = format!("{base}.id");
                }
                Some(format!("{lhs} {op} {rhs};"))
            }
            // An `expr(..)` written for its operand padding is the plain assignment.
            Stmt::Primitive {
                span,
                prim,
                branches: None,
            } if let Some((lhs, code, rhs)) = expr_parts(prim)
                && let Some(op) = assign_op_of(code)
                && self.extra() =>
            {
                self.simple(&Stmt::Assign {
                    span: *span,
                    lhs: lhs.clone(),
                    op,
                    rhs: rhs.clone(),
                })
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
                // A standalone AIR call exits false on failure.
                Some(format!("if !{call} {{ return false; }}"))
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
            } else if self.model.std_globals.contains(&id) {
                "globals"
            } else {
                self.model.globals_module()
            };
            format!("{module}::{ident}")
        };
        if pass_temps {
            // In behavioral mode a callee with arguments takes the temps by name: v2 copies
            // them through locals into the same temps (frame storage is not compared there).
            if let Some(&arity) = self.model.literal_calls.get(&id)
                && self.model.behavioral
                && !self.plan.temps_raw.contains(&self.tree.id)
                && self.wave2()
            {
                self.temps_named.set(true);
                params.extend((0..arity).map(|i| format!("temp[{i}]")));
            } else if self.model.literal_calls.contains_key(&id) {
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
                // `x.has_function[N]` is a bool: it is the test against 0 (`!= 0` only; `> 0`
                // differs for a negative cell, which the checker cannot rule out). v2 names
                // functions 0..=255.
                if let OperandRef::Direct(Operand { scope: 35, index }) = lhs
                    && (0..=255).contains(index)
                    && is_operand(rhs, 7, 0)
                    && matches!(op, CompareOp::Eq | CompareOp::Ne)
                    && self.wave2()
                {
                    let not = if *op == CompareOp::Eq { "!" } else { "" };
                    let handle = self.definition_handle()?;
                    return Some(format!("{not}{handle}.has_function[{index}]"));
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
                    // Bitflags places, and any other `i16` cell (v2 types both sides as
                    // `i16` there).
                    CompareOp::FlagTest if self.extra() => "flag_test",
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
            // An `expr(..)` written for its operand padding is the plain comparison.
            Condition::Primitive { span, prim }
                if let Some((lhs, code, rhs)) = expr_parts(prim)
                    && let Some(op) = compare_op_of(code) =>
            {
                if !self.extra() {
                    return None;
                }
                self.cond(&Condition::Compare {
                    span: *span,
                    lhs: lhs.clone(),
                    op,
                    rhs: rhs.clone(),
                })
            }
            Condition::Primitive { prim, .. } => self.prim_cond(prim),
            Condition::Not(inner) => {
                let text = self.cond(inner)?;
                // Any binary condition (a compare, or a padded `expr` written as one) binds
                // looser than `!`. A call (`inc_lt(..)`, `x.is_empty()`) does not; a negation
                // keeps its parentheses (`!(!x)`).
                if has_top_level_space(&text) || text.starts_with('!') {
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
                let receiver = if self.sim && obj == "stack_obj" && !self.stack_moves {
                    "self".to_owned()
                } else if let Some(base) = obj.strip_suffix(".data[11]").filter(|b| {
                    *b == "self" || (self.sim && *b == "sim") || (self.inherit && *b == "me")
                }) {
                    // Object data word 11 is the object's id; `.is_type` needs it typed as a
                    // handle, which only the `.id` spelling gives (`me.id` in an inherit tree).
                    format!("{base}.id")
                } else if let Some(base) = obj
                    .strip_suffix(".data[2]")
                    .filter(|b| self.sim && *b == "sim")
                {
                    format!("{base}.container")
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
            PrimitiveCall::RemoveObject { target, flags } => self.remove_object(*target, *flags),
            PrimitiveCall::Dialog { .. } if self.wave2() => self.dialog(p),
            // Find Best Object's own node: true selects the object found.
            PrimitiveCall::FindBestObject {
                function,
                flags,
                origin: None,
            } if self.cursor.is_none() && self.wave2() => {
                let kind = crate::parser::FUNCTION_KINDS
                    .iter()
                    .find(|k| k.function == *function)?;
                if kind.search == crate::parser::FunctionSearch::None {
                    return None;
                }
                Some(if *flags == 0 {
                    format!("find_best_object({})", kind.name)
                } else {
                    format!("find_best_object({}, flags: {flags})", kind.name)
                })
            }
            // The pad covers only bytes the VM reads no differently.
            PrimitiveCall::Padded { call, .. } => self
                .builtin(p)
                .or_else(|| self.extra().then(|| self.prim_cond(call)).flatten()),
            other => self.builtin(other),
        }
    }

    /// Remove Object Instance as `x.remove(..)`. Its operand is a mode: 0 the running object
    /// (`self` in an object's own tree, `sim` in a Sim tree, `me` in an inherit tree), 1 the
    /// stack object (`stack_obj`, or the cursor binding that holds it).
    fn remove_object(&self, target: u16, flags: u8) -> Option<String> {
        if !self.wave2() {
            return None;
        }
        let receiver = match target {
            0 if self.inherit => "me",
            0 if self.sim => "sim",
            0 => "self",
            1 => self.cursor.as_deref().unwrap_or("stack_obj"),
            _ => return None,
        };
        let options = match flags {
            0 => String::new(),
            1 => "no_yield: true".to_owned(),
            2 => "skip_resets: true".to_owned(),
            3 => "no_yield: true, skip_resets: true".to_owned(),
            _ => format!("flags: {flags}"),
        };
        Some(format!("{receiver}.remove({options})"))
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
                    let base = [lo, hi, 0, 0, *flags, 0, 0, 0];
                    let operands = apply_operand_pad(23, base, pad).ok()?;
                    let text = self.play_sound(*sound_id, operands[4])?;
                    // PR I-4's `padN:` spells the bytes the call's arguments do not (0, 1
                    // and 4 are the sound and the flags).
                    let pads = pad_options(&operands, &[lo, hi, 0, 0, operands[4], 0, 0, 0]);
                    if operands[..2] != [lo, hi] {
                        return None;
                    }
                    // Without them, the pad covers only bytes the VM reads no differently.
                    if pads.is_empty() || !self.wave3() {
                        return Some(text);
                    }
                    let text = text.strip_suffix(')')?;
                    let sep = if text.ends_with('(') { "" } else { ", " };
                    Some(format!("{text}{sep}{})", pads.join(", ")))
                }
                PrimitiveCall::ClearMotiveChanges => {
                    let operands = apply_operand_pad(29, CLEAR_MOTIVE_CHANGES, pad).ok()?;
                    let pads = pad_options(&operands, &CLEAR_MOTIVE_CHANGES);
                    if !pads.is_empty() && self.wave2() && self.wave3() {
                        return Some(format!("clear_motive_changes({})", pads.join(", ")));
                    }
                    self.builtin(p)
                        .or_else(|| self.extra().then(|| self.prim_stmt(call)).flatten())
                }
                PrimitiveCall::StopAllSounds { .. } if self.extra() => self.prim_stmt(call),
                // The pad covers only bytes the VM reads no differently.
                _ => self
                    .builtin(p)
                    .or_else(|| self.extra().then(|| self.prim_stmt(call)).flatten()),
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
                    return self.remove_object(*target, *flags);
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
            // An inherit tree's default target depends on the caller; `me` does not.
            (false, false) if self.inherit => "me",
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
            PrimitiveCall::ClearMotiveChanges if self.wave2() => {
                "clear_motive_changes()".to_owned()
            }
            PrimitiveCall::SetMotiveChange {
                motive,
                delta_per_hour,
                stop_at,
            } if *motive < 16 && self.wave2() => {
                let motive = motive_index(i16::from(*motive));
                format!(
                    "set_motive_change({motive}, {}, {})",
                    self.rvalue(delta_per_hour)?,
                    self.rvalue(stop_at)?
                )
            }
            // `dest: stack_obj` is the primitive alone: the new object stays selected (v2
            // saves and restores `stack_obj` around it otherwise).
            PrimitiveCall::CreateObject {
                guid,
                mode,
                flags,
                local: LocalRef::Index(local),
            } if self.cursor.is_none() && self.wave2() => {
                let local = match (mode, local) {
                    (8 | 9, i) => format!(", local: {}", self.local_name(u16::from(*i))?),
                    (_, 0) => String::new(),
                    _ => return None,
                };
                format!(
                    "create_object({}, mode: {mode}, flags: {flags}{local}, dest: stack_obj)",
                    self.guid(*guid)
                )
            }
            PrimitiveCall::Breakpoint { cond } => format!("breakpoint({})", self.rvalue(cond)?),
            PrimitiveCall::Random { dst, range } => {
                let range = self.rvalue(range)?;
                if range.starts_with("global[") || range.starts_with("world.") {
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
            // Flag 0x01 reads the icon from a local, which `icon:` names (and implies).
            PrimitiveCall::PushInteraction {
                menu,
                object_slot,
                priority,
                flags,
                icon_local: LocalRef::Index(icon),
            } if flags & 0x03 == 0x01 && self.wave2() => format!(
                "push_interaction(menu: {menu}, object: {object_slot}, priority: {priority}, flags: {flags}, icon: {})",
                self.local_name(u16::from(*icon))?
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
            // The pad covers only bytes the VM reads no differently.
            PrimitiveCall::Padded { call, .. } if self.extra() => {
                return self.builtin(call).or_else(|| self.pilot_builtin(p));
            }
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
            // PR I-4: a slot read from a tree parameter.
            PrimitiveCall::DropOnto { src, dst } if self.wave3() => format!(
                "drop_onto(src: {}, dst: {})",
                self.slot_spec(src)?,
                self.slot_spec(dst)?
            ),
            // PR I-4: mode 1 reads the slot from a parameter; the others read none (an
            // omitted `slot:` encodes parameter 0).
            PrimitiveCall::Reach {
                mode,
                moves,
                slot_param,
            } if self.wave3() => match (*mode, *slot_param) {
                (1, p) => format!(
                    "reach(mode: 1, moves: {moves}, slot: {})",
                    self.param_slot(p)?
                ),
                (m, 0) => format!("reach(mode: {m}, moves: {moves})"),
                _ => return None,
            },
            // PR I-4: skin, age and sex are read from `i16` locals.
            PrimitiveCall::MakeNewCharacter {
                skin: LocalRef::Index(skin),
                age: LocalRef::Index(age),
                sex: LocalRef::Index(sex),
                species,
            } if *species <= 2 && self.wave3() => format!(
                "make_new_character(skin: {}, age: {}, sex: {}, species: {species})",
                self.local_name(u16::from(*skin))?,
                self.local_name(u16::from(*age))?,
                self.local_name(u16::from(*sex))?
            ),
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
                // A cell the primitive writes (or, with flag 4, reads): a local, a temp or
                // a parameter; with flag 4 also a literal.
                let value = match value {
                    OperandRef::Direct(Operand { scope: 25, index }) => {
                        self.local_name(u16::try_from(*index).ok()?)?
                    }
                    OperandRef::Direct(Operand { scope: 8 | 9, .. }) if self.wave2() => {
                        self.lvalue(value)?
                    }
                    OperandRef::Direct(Operand { scope: 7, index })
                        if flags & 4 != 0 && self.wave2() =>
                    {
                        index.to_string()
                    }
                    _ => return None,
                };
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
            None => String::new(),
            Some(d) if is_operand(d, 10, 0) => ", dest: stack_obj".to_owned(),
            // A local, a parameter or a temp (`temp[0]` to `temp[7]`).
            Some(OperandRef::Direct(Operand { scope, index }))
                if (*scope == 25 || *scope == 9 || (*scope == 8 && (0..8).contains(index)))
                    && self.wave2() =>
            {
                format!(", dest: {}", self.lvalue(dest.as_ref()?)?)
            }
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

    /// A slot operand of `drop_onto`: a number, or the tree parameter that holds it.
    fn slot_spec(&self, spec: &SlotSpec) -> Option<String> {
        match spec {
            SlotSpec::Literal(n) => Some(n.to_string()),
            SlotSpec::Param(p) => self.param_slot(*p),
        }
    }

    /// `param[p]`, which a slot argument reads by index. Inside an animation handler too:
    /// the argument names the parameter by index, not the handler's `event` binding.
    fn param_slot(&self, p: i16) -> Option<String> {
        (p >= 0).then(|| format!("param[{p}]"))
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
                let table = addr >> 7;
                // v2 code names an entry by path (`Table::entry`); raw `.air` keeps the dot.
                if table < 64 {
                    self.model.bcon_path(addr).map(|p| p.replacen('.', "::", 1))
                } else {
                    let name = self.model.extern_bcons.get(&addr)?;
                    Some(format!("library_consts::{name}"))
                }
            }
            10 if o.index == 0 && self.cursor.is_some() => self.cursor.clone(),
            9 if o.index == 0 && self.in_handler => Some("event".to_owned()),
            20 if !self.sim && !self.inherit && o.index >= 0 && self.extra() => {
                Some(format!("self.slot[{}]", o.index))
            }
            20 if self.sim && (0..=255).contains(&o.index) && self.extra() => {
                Some(format!("sim.slot[{}]", o.index))
            }
            16 if self.sim && o.index >= 0 && self.extra() => {
                let handle = if self.stack_moves {
                    "stack_obj"
                } else {
                    "self"
                };
                Some(format!("{handle}.slot[{}]", o.index))
            }
            3 if self.sim && o.index == 2 && self.extra() => Some("sim.container".to_owned()),
            // The stack object's definition words (read-only).
            21 if matches!(o.index, 9 | 11) && self.wave2() => {
                let field = if o.index == 9 {
                    "object_type"
                } else {
                    "sub_index"
                };
                Some(format!("{}.{field}", self.definition_handle()?))
            }
            // A field of the job named by `temp[0]` (career) and `temp[1]` (level).
            33 if o.index >= 0 && self.wave2() => Some(format!("job_field({})", o.index)),
            19 if o.index == 65 && self.person_binding().is_some() => {
                Some(format!("{}.kind", self.person_binding()?))
            }
            // The stack object's neighborhood record, read-only: its fields (24) and its
            // person's words (32).
            24 | 32
                if (0..=if o.scope == 24 { 9 } else { 255 }).contains(&o.index)
                    && self.neighbor_handle().is_some()
                    && self.wave3() =>
            {
                let handle = self.neighbor_handle()?;
                // The `i16` cells PR I-4 names (the `ObjectId`, `bool` and enum ones would
                // type their uses differently).
                let name = match (o.scope, o.index) {
                    (24, 2) => Some("age"),
                    (24, 5) => Some("friend_count"),
                    (24, 6) => Some("family_house"),
                    (24, 9) => Some("family_friend_count"),
                    (32, 0) => Some("posture"),
                    (32, 34) => Some("pets_outside"),
                    (32, 56) => Some("job_type"),
                    (32, 57) => Some("job_level"),
                    (32, 61) => Some("family"),
                    (32, 77) => Some("date_partner"),
                    (32, FAME) => Some("fame"),
                    _ => None,
                };
                Some(match name {
                    Some(name) => format!("{handle}.neighbor.{name}"),
                    None if o.scope == 24 => format!("{handle}.neighbor.field[{}]", o.index),
                    None => format!("{handle}.neighbor.person[{}]", o.index),
                })
            }
            _ => self.lvalue(r),
        }
    }

    /// The handle that reads the stack object's definition cells in place (`x.object_type`,
    /// `x.has_function[N]`): `stack_obj`, or `self` in a Sim tree whose stack object is the
    /// entry object. In a cursor loop they are left numbered: another handle moves the
    /// stack object.
    fn definition_handle(&self) -> Option<&'static str> {
        if self.cursor.is_some() {
            return None;
        }
        Some(if self.sim && !self.inherit && !self.stack_moves {
            "self"
        } else {
            "stack_obj"
        })
    }

    /// The handle the stack object's neighborhood record is read through: the cursor
    /// binding inside a `for` loop whose cursor is `stack_obj`, else [`Self::stack_handle`].
    fn neighbor_handle(&self) -> Option<&str> {
        match &self.cursor {
            Some(c) => Some(c),
            None => self.stack_handle(),
        }
    }

    /// The handle the stack object's cells are read through in place, outside cursor and
    /// Sim loops: `stack_obj`, or `self` in a Sim tree that never moves the stack object.
    fn stack_handle(&self) -> Option<&'static str> {
        if self.cursor.is_some() || self.sims_loop.is_some() {
            return None;
        }
        Some(if self.sim && !self.inherit && !self.stack_moves {
            "self"
        } else {
            "stack_obj"
        })
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
                19 if i == FAME && self.wave3() => return Some(format!("{c}.fame")),
                31 if i < 8 && self.wave3() => return Some(format!("{c}.person[temp[{i}]]")),
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
            // `me` is whatever runs the tree; its person words and the stack object's.
            match o.scope {
                18 if i == FAME && self.wave3() => return Some("me.fame".to_owned()),
                19 if i == FAME && self.person_binding().is_none() && self.wave3() => {
                    return Some("stack_obj.fame".to_owned());
                }
                30 if i < 8 && self.wave3() => return Some(format!("me.person[temp[{i}]]")),
                _ => {}
            }
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
                let index = if field == "motive" {
                    motive_index(i)
                } else {
                    i.to_string()
                };
                return (i <= 255).then(|| format!("{handle}.{field}[{index}]"));
            }
        }
        Some(match (o.scope, self.sim) {
            (8, _) => format!("temp[{i}]"),
            (6, _) => crate::v2::parser::WORLD_GLOBALS
                .iter()
                .find(|&&(_, index)| i16::from(index) == i)
                .map_or_else(
                    || format!("global[{i}]"),
                    |(name, _)| format!("world.{name}"),
                ),
            (10, _) if i == 0 => "stack_obj".to_owned(),
            (1, true) if self.stack_moves && i <= 255 => format!("stack_obj.attr[{i}]"),
            (0, false) | (1, true) if i <= self.model.state_limit => format!("self.attr_{i}"),
            (3, false) => format!("self.data[{i}]"),
            (0, true) if i <= 255 && self.extra() => format!("sim.attr[{i}]"),
            (3, true) if i <= 255 && self.extra() => format!("sim.data[{i}]"),
            (4, _) if i == 8 && self.extra() => "stack_obj.data[8]".to_owned(),
            // The stack object's cells: `stack_obj.*` in an object tree; in a Sim tree,
            // where the stack object is `self`, `self.*` unless the tree moves `stack_obj`.
            (0, false) if i <= 255 && self.extra() => format!("self.attr[{i}]"),
            (1, _) | (4, _) if i <= 255 && self.extra() => {
                let handle = if self.sim && !self.stack_moves {
                    "self"
                } else {
                    "stack_obj"
                };
                let field = if o.scope == 1 { "attr" } else { "data" };
                format!("{handle}.{field}[{i}]")
            }
            (15, _) if i <= 255 && self.extra() => format!("stack_obj.motive[{}]", motive_index(i)),
            (14, true) => format!("sim.motive[{}]", motive_index(i)),
            // PR I-4's typed person word (the lint's table does not name it).
            (18, true) if i == FAME && self.wave3() => "sim.fame".to_owned(),
            (19, _) if i == FAME && self.wave3() => {
                format!("{}.fame", self.person_binding().unwrap_or("stack_obj"))
            }
            (18, true) => self
                .personality("sim", i)
                .unwrap_or_else(|| format!("sim.person[{i}]")),
            (9, _) if !(self.in_handler && i == 0) && self.extra() => format!("param[{i}]"),
            (19, _) => {
                let binding = self.person_binding();
                self.personality(binding.unwrap_or("stack_obj"), i)
                    .or_else(|| binding.map(|p| format!("{p}.person[{i}]")))
                    .or_else(|| {
                        (i <= 255 && self.extra()).then(|| format!("stack_obj.person[{i}]"))
                    })?
            }
            (25, _) => self.local_name(i as u16)?,
            // The check tree's advertisement: its motive deltas (12), personality (28) and
            // minimums (29).
            (12 | 28 | 29, _) if i < 16 && self.wave3() => {
                let field = match o.scope {
                    12 => "delta",
                    28 => "personality",
                    _ => "minimum",
                };
                format!("advert.{field}[{i}]")
            }
            // The cells whose number is in `temp[i]`: the stack object's sprite flag (27),
            // the running Sim's person word (30) and the stack object's (31).
            (27, _) if i < 8 && self.stack_handle().is_some() && self.wave3() => {
                format!("{}.sprite_flags[temp[{i}]]", self.stack_handle()?)
            }
            (30, true) if i < 8 && !self.inherit && self.wave3() => {
                format!("sim.person[temp[{i}]]")
            }
            (31, _) if i < 8 && self.stack_handle().is_some() && self.wave3() => {
                format!("stack_obj.person[temp[{i}]]")
            }
            _ => return None,
        })
    }
}

/// A motive index as v2 writes it: `Motive::Hunger` for the motives `std` names (the
/// lowering's own table, `motive_by_name`), else the number.
fn motive_index(i: i16) -> String {
    const NAMES: [(i16, &str); 10] = [
        (3, "Mood"),
        (5, "Energy"),
        (6, "Comfort"),
        (7, "Hunger"),
        (8, "Hygiene"),
        (9, "Bladder"),
        (11, "Alertness"),
        (13, "Room"),
        (14, "Social"),
        (15, "Fun"),
    ];
    NAMES
        .iter()
        .find(|(m, _)| *m == i)
        .map_or_else(|| i.to_string(), |(_, n)| format!("Motive::{n}"))
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

/// The part of a tree's header the code checker does not compare but the game reads and
/// `#[frame]` writes: the revision (saved objects are matched against it). The label is
/// [`Model::object_gap`]'s.
fn header_gap(original: &BhavChunk, rebuilt: &BhavChunk) -> Option<String> {
    (original.revision != rebuilt.revision).then(|| {
        format!(
            "the rebuilt tree has revision {}, not {}",
            rebuilt.revision, original.revision
        )
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
    fn motive_names_are_the_lowerings() {
        for i in 0..16 {
            let name = motive_index(i);
            let expected = name
                .strip_prefix("Motive::")
                .and_then(|n| crate::parser::motive_by_name(&n.to_ascii_lowercase()));
            match expected {
                Some(m) => assert_eq!(i16::from(m), i),
                None => assert_eq!(name, i.to_string()),
            }
        }
    }

    #[test]
    fn an_else_joins_its_if_only_when_the_if_chain_is_all_it_holds() {
        let text = "fn f() {\n    if a {\n        x = 1;\n    } else {\n        if b {\n            x = 2;\n        } else {\n            if c {\n                x = 3;\n            }\n        }\n    }\n}\n";
        assert_eq!(
            collapse_else_if(text).as_deref(),
            Some(
                "fn f() {\n    if a {\n        x = 1;\n    } else if b {\n            x = 2;\n        } else if c {\n                x = 3;\n            }\n}\n"
            )
        );
        // A statement after the chain, a comment, or a `for` loop's `else` stays.
        for kept in [
            "fn f() {\n    if a {\n    } else {\n        if b {\n        }\n        x = 1;\n    }\n}\n",
            "fn f() {\n    if a {\n    } else {\n        // why\n        if b {\n        }\n    }\n}\n",
            "fn f() {\n    for p in lot.sims() {\n    } else {\n        if b {\n        }\n    }\n}\n",
            "fn f() {\n    raw {\n        if a {\n        } else {\n            if b {\n            }\n        }\n    }\n}\n",
        ] {
            assert_eq!(collapse_else_if(kept), None, "{kept}");
        }
    }

    #[test]
    fn single_name_imports_from_one_module_are_braced() {
        let text = "// header\n\nuse std::flags::ObjectFlags;\nuse std::engine::RefreshMode;\nuse std::flags::Expansions;\nuse std::globals::*;\n\nobject \"A\" {}\n";
        assert_eq!(
            group_imports(text),
            "// header\n\nuse std::engine::RefreshMode;\nuse std::flags::{Expansions, ObjectFlags};\nuse std::globals::*;\n\nobject \"A\" {}\n"
        );
        let single = "use std::flags::ObjectFlags;\nuse std::globals::*;\n\nobject \"A\" {}\n";
        assert_eq!(group_imports(single), single);
    }

    #[test]
    fn renames_touch_code_and_comments_but_not_strings() {
        let renames = [("open_4100".to_owned(), "open".to_owned())];
        assert_eq!(
            apply_renames(
                "// action open_4100\nfn open_4100() { reopen_4100(); show(\"open_4100\"); }\n",
                &renames
            ),
            "// action open\nfn open() { reopen_4100(); show(\"open_4100\"); }\n"
        );
    }

    #[test]
    fn negations_flip_only_as_a_whole_test() {
        let text = "object \"O\" {\n#[bhav(id = 4300)]\nfn f() {\n    if !(temp[0] > -1) {\n    }\n    while !(a.b[2] == X::y) {\n    }\n    if !(a > b && c) {\n    }\n    if !(inc_lt(&mut a, 2)) {\n    }\n    raw {\n        if !(a > b) {\n        }\n    }\n}\n}\n";
        let flips: Vec<String> = negated_comparisons(text)
            .into_iter()
            .map(|f| format!("{} => {}", &text[f.range], f.text))
            .collect();
        assert_eq!(
            flips,
            [
                "!(temp[0] > -1) => temp[0] <= -1",
                "!(a.b[2] == X::y) => a.b[2] != X::y"
            ]
        );
    }

    #[test]
    fn a_flip_belongs_to_the_tree_whose_attribute_comes_before_it_in_code() {
        // A quoted label that spells another tree's attribute does not count.
        let text = "object \"O\" {\n#[id(4300)]\nfn a() {\n}\n#[bhav(id = 4301, label = \"Other #[id(4300)]\")]\nfn b() {\n    if !(temp[0] > 1) {\n    }\n}\n}\n";
        let trees: Vec<u16> = negated_comparisons(text).iter().map(|f| f.tree).collect();
        assert_eq!(trees, [4301]);
        // `id` after other fields and in hex; an unpinned helper after a tree is in no tree;
        // a lifecycle block's `#[id(N)]` counts.
        let text = "#[bhav(id = 4300)]\nfn a() {\n    if !(temp[0] > 1) {\n    }\n}\n#[bhav(locals = 1, id = 0x10CD)]\nfn b() {\n    if !(temp[0] > 2) {\n    }\n}\nfn helper() {\n    if !(temp[0] > 3) {\n    }\n}\n#[id(4096)]\ninit {\n    if !(temp[0] > 4) {\n    }\n}\n";
        let wrapped = format!("object \"O\" {{\n{text}}}\n");
        let trees: Vec<u16> = negated_comparisons(&wrapped)
            .iter()
            .map(|f| f.tree)
            .collect();
        assert_eq!(trees, [4300, 0x10CD, 4096]);
        // A parameter's attribute is no member; a bare `#[bhav]` keeps the ID before it; an
        // `extern mod`'s attributes pin nothing after it.
        let text = "object \"O\" {\n#[bhav(id = 4300)]\nfn a(#[temp(0)] x: i16) {\n    if !(x > 1) {\n    }\n}\n#[id(4301)]\n#[bhav]\nfn b() {\n    if !(temp[0] > 2) {\n    }\n}\nextern mod m {\n    #[id(300)]\n    fn probe() -> bool;\n}\npub fn helper() {\n    if !(temp[0] > 3) {\n    }\n}\n}\n";
        let trees: Vec<u16> = negated_comparisons(text).iter().map(|f| f.tree).collect();
        assert_eq!(trees, [4300, 4301]);
        // A line with a multibyte character gets no offsets, so nothing on it is flipped.
        let text = "object \"O\" {\n#[bhav(id = 4300, label = \"é\")] fn a() { if !(temp[0] > 1) {\n    }\n}\n}\n";
        assert!(negated_comparisons(text).is_empty());
        // Before any tree attribute, a test has no tree and is never flipped.
        assert!(
            negated_comparisons("object \"O\" {\nfn f() {\n    if !(a > b) {\n    }\n}\n}\n")
                .is_empty()
        );
    }

    #[test]
    fn dotted_constant_paths_touch_code_only() {
        let text = "fn f() {\n    // Tuning::e0\n    temp[0] = Tuning::e0;\n    temp[1] = Tuning::e1;\n    show(\"Tuning::e0\");\n    temp[2] = Other::e0 + MyTuning::e0;\n}\n";
        assert_eq!(
            dot_paths(text, &["Tuning"], &["e0".to_owned()]).as_deref(),
            Some(
                "fn f() {\n    // Tuning::e0\n    temp[0] = Tuning.e0;\n    temp[1] = Tuning::e1;\n    show(\"Tuning::e0\");\n    temp[2] = Other::e0 + MyTuning::e0;\n}\n"
            )
        );
        // A path written with something between its parts, or on a line with a multibyte
        // character, is left alone; the others still change.
        let text = "fn f() {\n    temp[0] = Tuning /* x */ ::e0;\n    show(\"é\"); temp[1] = Tuning::e0;\n    temp[2] = Tuning::e0;\n}\n";
        assert_eq!(
            dot_paths(text, &["Tuning"], &["e0".to_owned()]).as_deref(),
            Some(
                "fn f() {\n    temp[0] = Tuning /* x */ ::e0;\n    show(\"é\"); temp[1] = Tuning::e0;\n    temp[2] = Tuning.e0;\n}\n"
            )
        );
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

    /// Lifts the object `source` compiles to: every tree proven, the text compiles again
    /// and lints clean.
    fn lift_source(source: &str) -> String {
        lift_source_with(source, &DecompileV2Options::default())
    }

    fn compile_source(text: &str) -> Iff {
        let object = crate::compile_v2_source("lift.antics", text, &PackResolverConfig::default())
            .unwrap_or_else(|e| panic!("{e}\n{text}"));
        Iff::parse(object.iff_bytes).expect("parse IFF")
    }

    /// Lifts `source`'s object with `options`, asserting every tree is proven and the
    /// result compiles and lints clean.
    fn lift_source_with(source: &str, options: &DecompileV2Options) -> String {
        let compile = compile_source;
        let report = decompile_iff_to_v2_report(&compile(source), options).expect("lift");
        assert!(report.compiled);
        assert!(
            report
                .trees
                .iter()
                .all(|t| matches!(t.check, TreeCheck::Identical | TreeCheck::Equivalent)),
            "{:?}\n{}",
            report.trees,
            report.source
        );
        compile(&report.source);
        let lints = crate::v2::lint_v2_source(&report.source, "lifted.antics").expect("lint");
        assert!(lints.is_empty(), "{lints:?}\n{}", report.source);
        report.source
    }

    fn has(lifted: &str, spellings: &[&str]) {
        for s in spellings {
            assert!(lifted.contains(s), "missing {s:?} in:\n{lifted}");
        }
    }

    #[test]
    fn std_globals_are_used_and_the_rest_merge_into_a_local_globals() {
        let lifted = lift_source(
            r#"use std::globals::*;

extern mod more_globals {
    #[id(300)] fn other() -> bool;
}

object "Fable Lamp" {
    guid: 0x7E00_0130;
    #[bhav(id = 4300, label = "Use", locals = 0)]
    fn use_it() {
        globals::standard_entry()?;
        if globals::am_i_a_child() {
            temp[0] = 1;
        }
        if more_globals::other() {
            temp[0] = 2;
        }
    }
}"#,
        );
        has(
            &lifted,
            &[
                "use std::globals::*;",
                "extern mod globals {",
                "#[id(300)] fn global_300() -> bool;",
                "globals::standard_entry()",
                "globals::am_i_a_child()",
                "globals::global_300()",
            ],
        );
        assert!(!lifted.contains("more_globals"), "{lifted}");
        assert!(!lifted.contains("#[id(272)]"), "{lifted}");
    }

    #[test]
    fn a_local_declaration_of_a_std_tree_keeps_more_globals() {
        // 273 is called passing temps, but `std::globals` declares it with one argument:
        // merging the two declarations would be refused.
        let lifted = lift_source(
            r#"use std::globals::*;

extern mod more_globals {
    #[id(273)] fn exit_now() -> bool;
}

object "Fable Lamp" {
    guid: 0x7E00_013A;
    #[bhav(id = 4300, label = "Use", locals = 0)]
    fn use_it() {
        globals::standard_entry()?;
        if more_globals::exit_now() {
            temp[0] = 1;
        }
    }
}"#,
        );
        has(
            &lifted,
            &[
                "use std::globals::*;",
                "extern mod more_globals {",
                "#[id(273)]",
                "globals::standard_entry()",
            ],
        );
        assert!(!lifted.contains("extern mod globals"), "{lifted}");
    }

    #[test]
    fn std_globals_need_the_called_arity() {
        // The bodies call 273 passing temps; std declares it with one argument.
        let lifted = lift_source(
            r#"extern mod globals {
    #[id(273)] fn exit_now() -> bool;
}

object "Fable Lamp" {
    guid: 0x7E00_0131;
    #[bhav(id = 4300, label = "Use", locals = 0)]
    fn use_it() {
        if globals::exit_now() {
            temp[0] = 1;
        }
    }
}"#,
        );
        has(&lifted, &["extern mod globals {", "#[id(273)]"]);
        assert!(!lifted.contains("use std::globals"), "{lifted}");
    }

    #[test]
    fn calls_and_flag_tests_are_returned_whole() {
        let lifted = lift_source(
            r#"object "Fable Clock" {
    guid: 0x7E00_0132;
    #[bhav(id = 4300, label = "Ready", locals = 0)]
    fn ready() -> bool {
        return self.data[3] == 1;
    }
    #[bhav(id = 4301, label = "Ask", locals = 0)]
    fn ask() -> bool {
        return ready();
    }
    #[bhav(id = 4302, label = "Ask Not", locals = 0)]
    fn ask_not() -> bool {
        return !ready();
    }
    #[bhav(id = 4303, label = "Lit", locals = 0)]
    fn lit() -> bool {
        return !(self.data[8] flag_test 6);
    }
    #[bhav(id = 4304, label = "Either", locals = 0)]
    fn either() -> bool {
        if self.data[4] == 0 {
            return self.data[5] == 2;
        } else if ready() {
            return true;
        } else {
            return false;
        }
    }
}"#,
        );
        has(
            &lifted,
            &[
                "return ready();",
                "return !ready();",
                "return !(self.flags flag_test ObjectFlags::InUse);",
                "} else {\n            return ready();",
            ],
        );
    }

    #[test]
    fn a_standalone_call_keeps_its_false_exit() {
        let lifted = lift_source(
            r#"object "Fable Clock" {
    guid: 0x7E00_0133;
    #[bhav(id = 4300, label = "Ready", locals = 0)]
    fn ready() -> bool {
        return self.data[3] == 1;
    }
    #[bhav(id = 4301, label = "Step", locals = 0)]
    fn step() -> bool {
        if !ready() {
            return false;
        }
        temp[0] = 1;
        return true;
    }
}"#,
        );
        has(
            &lifted,
            &["if !ready() {\n            return false;\n        }"],
        );
        assert!(!lifted.contains("raw"), "{lifted}");
    }

    #[test]
    fn raw_bare_calls_spell_out_the_false_exit() {
        assert_eq!(
            explicit_false_exit("    call 4099(temps);"),
            "    if !call 4099(temps) { return false; }"
        );
        assert_eq!(
            explicit_false_exit("call 4099(1, 2, 3, 4);"),
            "if !call 4099(1, 2, 3, 4) { return false; }"
        );
        for kept in [
            "call 4099(temps) -> true: @n2, false: error;",
            "temp[0] := 1;",
            "if call 4099(temps) {",
        ] {
            assert_eq!(explicit_false_exit(kept), kept);
        }
    }

    #[test]
    fn a_sim_tree_that_moves_stack_obj_names_its_cells_by_stack_obj() {
        let lifted = lift_source(
            r#"object "Fable Easel" {
    guid: 0x7E00_0134;
    state paint: i16;
    #[bhav(id = 4300, label = "Pick", locals = 0)]
    fn pick(sim: Sim) {
        stack_obj = temp[1];
        stack_obj.attr[0] = 2;
        sim.motive[1] = 3;
    }
}"#,
        );
        has(&lifted, &["stack_obj = temp[1];", "stack_obj.attr[0] = 2;"]);
    }

    #[test]
    fn every_local_a_tree_uses_is_named() {
        let lifted = lift_source(
            r#"object "Fable Kiln" {
    guid: 0x7E00_013B;
    #[bhav(id = 4300, label = "Fire", locals = 2)]
    fn fire() {
        raw {
            local[0] := temp[0];
            local[1] := local[0];
            temp[1] := local[1];
            my.attr[2] := local[0];
        }
    }
}"#,
        );
        has(
            &lifted,
            &[
                "#[slot(0)] let slot_0: i16;",
                "#[slot(1)] let slot_1: i16;",
                "slot_1 = slot_0;",
                "temp[1] = slot_1;",
            ],
        );
        assert!(!lifted.contains("raw {"), "{lifted}");
    }

    #[test]
    fn object_and_stack_cells_and_ids_leave_raw() {
        let lifted = lift_source(
            r#"object "Fable Loom" {
    guid: 0x7E00_013C;
    #[bhav(id = 4300, label = "Weave", locals = 0)]
    fn weave() {
        raw {
            my.attr[3] := 4;
            stack_obj := temp[0];
            stack.attr[1] := my.attr[3];
            stack.data[2] := 5;
            temp[1] := stack.person[4];
            stack_obj := my.data[11];
            stack.motive[2] := 1;
        }
    }
}"#,
        );
        has(
            &lifted,
            &[
                "self.attr_3 = 4;",
                "stack_obj.attr[1] = self.attr_3;",
                "stack_obj.data[2] = 5;",
                "temp[1] = stack_obj.person[4];",
                "stack_obj = self.id;",
                "stack_obj.motive[2] = 1;",
            ],
        );
        assert!(!lifted.contains("raw {"), "{lifted}");
    }

    #[test]
    fn padded_expressions_and_primitives_leave_raw() {
        let lifted = lift_source(
            r#"object "Fable Lathe" {
    guid: 0x7E00_013D;
    #[bhav(id = 4300, label = "Turn", locals = 0)]
    fn turn() {
        raw {
            expr(temp[0] := 3) pad 0xff;
            expr(temp[1] += temp[0]) pad 0xff;
            if expr(temp[1] > 2) pad 0xff {
                notify_idle() pad 0xff;
            }
        }
    }
}"#,
        );
        has(
            &lifted,
            &[
                "temp[0] = 3;",
                "temp[1] += temp[0];",
                "if !(temp[1] > 2) {",
                "notify_out_of_idle();",
            ],
        );
        assert!(!lifted.contains("raw {"), "{lifted}");
    }

    #[test]
    fn a_sim_tree_whose_search_moves_stack_obj_names_its_cells_by_stack_obj() {
        let lifted = lift_source(
            r#"object "Fable Quill" {
    guid: 0x7E00_013E;
    #[bhav(id = 4300, label = "Look", locals = 0)]
    fn look(sim: Sim) {
        raw {
            if find_best_interaction() {
                stack.data[25] := 100;
            }
        }
    }
}"#,
        );
        has(&lifted, &["stack_obj.data[25] = 100;"]);
        assert!(!lifted.contains("self.data[25]"), "{lifted}");
    }

    #[test]
    fn a_sim_tree_whose_step_search_or_creation_moves_stack_obj_names_its_cells_by_stack_obj() {
        // A step with no destination, a search, a creation and a direction read into
        // `stack_obj` all move it.
        let lifted = lift_source(
            r#"object "Fable Quill" {
    guid: 0x7E00_0143;
    #[bhav(id = 4300, label = "Look", locals = 0)]
    fn look(sim: Sim) {
        raw {
            my.motive[1] := 2;
            if set_to_next(person) {
                stack.data[25] := 100;
            }
        }
    }
    #[bhav(id = 4301, label = "Seek", locals = 0)]
    fn seek(sim: Sim) {
        raw {
            my.motive[1] := 2;
            if find_best_object(0, 0) {
                stack.data[26] := 100;
            }
        }
    }
    #[bhav(id = 4302, label = "Make", locals = 0)]
    fn make(sim: Sim) {
        raw {
            my.motive[1] := 2;
            if create_object(0x7E00_0143) {
                stack.data[27] := 100;
            }
        }
    }
    #[bhav(id = 4303, label = "Turn", locals = 0)]
    fn turn(sim: Sim) {
        raw {
            my.motive[1] := 2;
            get_direction_to(stack_obj);
            stack.data[28] := 100;
        }
    }
}"#,
        );
        for i in 25..=28 {
            assert!(!lifted.contains(&format!("self.data[{i}]")), "{lifted}");
        }
        has(
            &lifted,
            &[
                "stack_obj.data[25] = 100;",
                "stack_obj.support_strength = 100;",
            ],
        );
    }

    #[test]
    fn bit_operations_and_sim_slots_leave_raw() {
        let lifted = lift_source(
            r#"object "Fable Spool" {
    guid: 0x7E00_013F;
    #[bhav(id = 4300, label = "Tidy", locals = 0)]
    fn tidy(sim: Sim) {
        raw {
            my.motive[1] := 2;
            my.motive[2] := 3;
            my.data[5] flag_set 3;
            if temp[0] flag_test 2 {
                stack.data[4] flag_clear 1;
            }
            if my.slot[0] == 0 {
                temp[1] := 1;
            }
        }
    }
}"#,
        );
        has(
            &lifted,
            &[
                "sim.wall_adjacency flag_set WallAdjacency::WallFront;",
                "if temp[0] flag_test 2 {",
                "self.allowed_heights flag_clear Height::Floor;",
                "if !(sim.hand == 0) {",
            ],
        );
        assert!(!lifted.contains("raw {"), "{lifted}");
    }

    /// The lint pass on its own: both `?` shapes are rewritten, and the file builds to the
    /// same bytes with the rewrite.
    #[test]
    fn lint_spellings_rewrite_both_question_mark_shapes_to_the_same_bytes() {
        let text = "use std::globals::*;\n\nobject \"Fable Wave3 Query\" {\n    guid: 0x7E00_016B;\n    #[bhav(id = 4300)]\n    fn t() {\n        if !globals::init_object() {\n            return error;\n        }\n        if globals::init_object() {\n        } else {\n            return error;\n        }\n        return true;\n    }\n}\n";
        let fixes = lint_spellings(text);
        assert_eq!(fixes.len(), 2, "{text}");
        let rewritten = apply_lint_spellings(text, &fixes);
        assert_eq!(
            rewritten.matches("globals::init_object()?;").count(),
            2,
            "{rewritten}"
        );
        assert!(!rewritten.contains("if "), "{rewritten}");
        let bytes = |t: &str| {
            crate::compile_v2_source("q.antics", t, &PackResolverConfig::default())
                .unwrap_or_else(|e| panic!("{e}\n{t}"))
                .iff_bytes
        };
        assert_eq!(bytes(text), bytes(&rewritten));
    }

    #[test]
    fn an_inherit_trees_callees_inherit_and_its_sounds_name_me() {
        let lifted = lift_source(
            r#"object "Fable Bell" {
    guid: 0x7E00_0140;
    #[bhav(id = 4300, label = "Ring", locals = 0)]
    fn ring(sim: Sim) {
        raw {
            my.motive[1] := 2;
            if call 4301(temps) {
                temp[0] := 1;
            }
        }
    }
    #[bhav(id = 4301, label = "Chime", locals = 0)]
    fn chime() {
        raw {
            if call 4302(temps) {
                temp[1] := 1;
            }
        }
    }
    #[bhav(id = 4302, label = "Tone", locals = 0)]
    fn tone() {
        raw {
            play_sound(5);
        }
    }
}"#,
        );
        assert!(!lifted.contains("raw {"), "{lifted}");
        has(&lifted, &["me.play_sound(5);"]);
        assert_eq!(lifted.matches("context = inherit").count(), 2, "{lifted}");
    }

    #[test]
    fn a_callee_that_fails_as_inherit_keeps_its_other_idioms() {
        let lifted = lift_source(
            r#"object "Fable Gong" {
    guid: 0x7E00_0145;
    #[bhav(id = 4300, label = "Strike", locals = 0)]
    fn strike(sim: Sim) {
        raw {
            my.motive[1] := 2;
            if call 4301(temps) {
                temp[0] := 1;
            }
        }
    }
    #[bhav(id = 4301, label = "Swing", locals = 0)]
    fn swing() {
        raw {
            if call 4302(temps) {
                temp[1] := 1;
            }
        }
    }
    #[bhav(id = 4302, label = "Fade", locals = 0)]
    fn fade() {
        raw {
            my.attr[2] := 1;
            play_sound(5);
            remove_object(me);
        }
    }
}"#,
        );
        // The leaf drops back to the object's context and keeps its idioms.
        let leaf = &lifted[lifted.find("fn fade()").expect("leaf")..];
        assert!(!leaf.contains("raw {"), "{lifted}");
        has(leaf, &["self.attr_2 = 1;", "play_sound(5);"]);
    }

    #[test]
    fn engine_cells_with_a_typed_name_take_it() {
        let lifted = lift_source(
            r#"object "Fable Lamp" {
    guid: 0x7E00_0146;
    #[bhav(id = 4300, label = "Glow", locals = 0)]
    fn glow(sim: Sim) {
        raw {
            my.motive[1] := 2;
            my.person[0] := 1;
            stack.data[25] := 1;
            if global[0] > 20 {
                temp[0] := 1;
            }
        }
    }
}"#,
        );
        has(
            &lifted,
            &["sim.posture = 1;", "self.in_use = 1;", "world.hour > 20"],
        );
    }

    #[test]
    fn typed_engine_fields_keep_what_builds_the_same_and_allow_the_rest() {
        let text = "fn f() {\n    stack_obj = self.data[11];\n    x = sim.slot[0] + global[0];\n}";
        let fix = |line, column, typed: &str| (line, column, typed.to_owned());
        let fixes = vec![
            fix(2, 17, "self.id"),
            fix(3, 9, "sim.hand"),
            fix(3, 23, "world.hour"),
        ];
        assert_eq!(
            apply_engine_field_fixes(text, &fixes).as_deref(),
            Some("fn f() {\n    stack_obj = self.id;\n    x = sim.hand + world.hour;\n}")
        );
        // Halving keeps every fix the check accepts and drops only the one it refuses.
        let ok = |fs: &[EngineFieldFix]| fs.iter().all(|f| f.2 != "self.id");
        assert_eq!(keep_fixes(&fixes, &ok), fixes[1..].to_vec());
        assert_eq!(
            allow_engine_fields(text, &fixes[..1]),
            "fn f() {\n    // antics-lint: allow(untyped_engine_field): `self.id` builds other nodes, \
             not proven the same here\n    stack_obj = self.data[11];\n    x = sim.slot[0] + global[0];\n}"
        );
    }

    #[test]
    fn raw_lines_name_the_objects_own_bcon_entries() {
        let lifted = lift_source(
            r#"object "Fable Gauge" {
    guid: 0x7E00_0142;
    bcon 4096 "Tuning" {
        speed = 5,
        size = 7,
    }
    #[bhav(id = 4300, label = "Read", locals = 0)]
    fn read() {
        raw {
            scope(21, 3) := bcon[1];
        }
        temp[0] = Tuning.speed;
        temp[1] = Tuning.size;
    }
}"#,
        );
        has(
            &lifted,
            &[
                "scope(21, 3) := Tuning.e1;",
                "temp[1] = Tuning::e1;",
                // Lint `unused_decl` does not count a `Table::entry` read, and raw code reads
                // this table by name, so the entry only v2 code reads keeps its dot.
                "temp[0] = Tuning.e0;",
            ],
        );
        assert!(!lifted.contains("bcon["), "{lifted}");
        let path = |addr: u16| (addr == 1).then(|| "Tuning.size".to_owned());
        assert_eq!(
            rewrite_bcons(
                "x := bcon[1]; y := my_bcon[1]; z := bcon[300] + bcon[1];",
                path
            ),
            "x := Tuning.size; y := my_bcon[1]; z := bcon[300] + Tuning.size;"
        );
    }

    #[test]
    fn node_order_puts_the_lower_branch_first() {
        let lifted = lift_source(
            r#"object "Fable Clock" {
    guid: 0x7E00_0135;
    #[bhav(id = 4300, label = "Ready", locals = 0)]
    fn ready() -> bool {
        return self.data[3] == 1;
    }
    #[bhav(id = 4301, label = "Pick", locals = 0)]
    fn pick() -> bool {
        if !ready() {
            temp[0] = 2;
        } else {
            temp[0] = 3;
        }
        return true;
    }
}"#,
        );
        has(&lifted, &["if !ready() {\n            temp[0] = 2;"]);
    }

    #[test]
    fn a_tree_that_proves_only_in_the_other_branch_order_keeps_it() {
        // The true branch visits n4, n2, n3: written first, it keeps the primitive at n3;
        // node order would put it at n4.
        let lifted = lift_source(
            r#"object "Fable Clock" {
    guid: 0x7E00_0137;
    #[bhav(id = 4300, label = "Pick", locals = 0)]
    fn pick() {
        raw {
            @n0:
            expr(my.data[3] == 1) -> true: @n4, false: @n1;
            @n1:
            expr(temp[0] := 5) -> true: true, false: error;
            @n2:
            expr(temp[1] := 2) -> true: @n3, false: error;
            @n3:
            play_sound(2) -> true: true, false: error;
            @n4:
            expr(temp[0] := 1) -> true: @n2, false: error;
        }
    }
}"#,
        );
        has(
            &lifted,
            &["if self.data[3] == 1 {\n            temp[0] = 1;"],
        );
        assert!(!lifted.contains("@n"), "{lifted}");
    }

    /// n3 (a yield) is reached from n1 and n2. The structured lift writes it once after the
    /// `if`, as the shared tail of both arms.
    const SHARED_YIELD: &str = r#"object "Fable Clock" {
    guid: 0x7E00_0137;
    #[bhav(id = 4300, label = "Pick", locals = 0)]
    fn pick() {
        raw {
            @n0:
            expr(my.data[3] == 1) -> true: @n2, false: @n1;
            @n1:
            expr(my.data[4] == 1) -> true: @n3, false: true;
            @n2:
            expr(temp[1] := 2) -> true: @n3, false: error;
            @n3:
            sleep(1) -> true: true, false: error;
        }
    }
}"#;

    #[test]
    fn behavioral_mode_proves_a_renumbered_tree_and_says_so() {
        let strict = lift_source(SHARED_YIELD);
        assert!(!strict.contains("@n"), "{strict}");
        assert_eq!(strict.matches("sleep(param[1])").count(), 1, "{strict}");
        assert!(!strict.contains("behavioral"), "{strict}");

        let behavioral = lift_source_with(
            SHARED_YIELD,
            &DecompileV2Options {
                behavioral: true,
                ..DecompileV2Options::default()
            },
        );
        assert!(!behavioral.contains("@n"), "{behavioral}");
        has(
            &behavioral,
            &[&BEHAVIORAL_HEADER.join("\n"), "temp[1] = 2;"],
        );
        assert_eq!(
            behavioral.matches("sleep(param[1])").count(),
            1,
            "{behavioral}"
        );
    }

    #[test]
    fn a_breakpoint_trap_loop_allows_the_yield_lint() {
        let lifted = lift_source(
            r#"object "Fable Tower" {
    guid: 0x7E00_0138;
    #[bhav(id = 4300, label = "Trap", locals = 0)]
    fn trap() {
        if self.data[3] == 1 {
            loop {
                breakpoint(1);
            }
        }
        temp[0] = 2;
        return true;
    }
}"#,
        );
        has(
            &lifted,
            &[
                "// antics-lint: allow(unbounded_loop_without_yield): the original's breakpoint loops on itself\n            loop {\n                breakpoint(1);",
            ],
        );
    }

    #[test]
    fn an_unchecked_lift_never_claims_behavioral_mode() {
        let report = decompile_iff_to_v2_report(
            &compile_source(SHARED_YIELD),
            &DecompileV2Options {
                validate: false,
                behavioral: true,
                ..DecompileV2Options::default()
            },
        )
        .expect("lift");
        assert!(!report.source.contains("behavioral"), "{}", report.source);
    }

    #[test]
    fn identical_interaction_tables_are_written_once() {
        let line = |s: &str| vec![s.to_owned()];
        assert_eq!(
            interaction_comments(vec![(vec![128], line("a"))]),
            line("a")
        );
        assert_eq!(
            interaction_comments(vec![(vec![128, 129], line("a")), (vec![130], line("b"))]),
            vec![
                "TTAB 128, 129:".to_owned(),
                "a".to_owned(),
                "TTAB 130:".to_owned(),
                "b".to_owned()
            ]
        );
    }

    #[test]
    fn an_oversized_lifecycle_tree_allows_the_frame_lint() {
        let stores: String = (0..205)
            .map(|i| format!("        temp[0] = {i};\n"))
            .collect();
        for header in [
            "#[id(4096)]",
            "#[bhav(id = 4096, args = 0, locals = 0, format = 0x8002, revision = 7)]",
        ] {
            let lifted = lift_source(&format!(
                "object \"Fable Tower\" {{\n    guid: 0x7E00_0136;\n    {header}\n    init {{\n{stores}    }}\n}}"
            ));
            has(&lifted, &["// antics-lint: allow(frame_headroom)"]);
        }
    }

    #[test]
    fn a_superseded_lifecycle_pin_does_not_take_a_trees_frame_finding() {
        // The last pinned `init` names the lifecycle tree; the first pin (4300) is the
        // oversized fn's, so its finding stays on the fn, not under the init's allow.
        let stores: String = (0..205)
            .map(|i| format!("        temp[0] = {i};\n"))
            .collect();
        let source = format!(
            "object \"Fable Tower\" {{\n    guid: 0x7E00_0137;\n    #[bhav(id = 4300)]\n    fn big() {{\n{stores}    }}\n    #[id(4300)]\n    // antics-lint: allow(frame_headroom): test\n    init {{\n        temp[0] = 1;\n    }}\n    #[id(4301)]\n    init {{\n        temp[1] = 1;\n    }}\n}}\n"
        );
        let diags = crate::v2::lint::lint_v2_source(&source, "t.antics").expect("lint");
        let frame: Vec<_> = diags
            .iter()
            .filter(|d| d.rule == crate::v2::lint::FRAME_HEADROOM)
            .collect();
        let fn_line = source.lines().position(|l| l.contains("fn big()")).unwrap() + 1;
        assert_eq!(frame.len(), 1, "{diags:?}");
        assert_eq!(frame[0].line, fn_line, "{diags:?}");
    }

    #[test]
    fn a_tree_already_past_the_frame_lint_allows_it() {
        let allow = |nodes: usize, locals: u16| {
            let mut t = tree(&vec![(2, 0xfe, 0xfd, [0; 8]); nodes]);
            t.locals = locals;
            let mut out = Out {
                lines: Vec::new(),
                indent: 0,
            };
            write_frame_allow(&mut out, &t);
            out.lines.len()
        };
        assert_eq!(allow(crate::v2::lint::FRAME_NODE_WARN, 0), 0);
        assert_eq!(allow(crate::v2::lint::FRAME_NODE_WARN + 1, 0), 1);
        assert_eq!(allow(1, crate::v2::lint::FRAME_LOCAL_WARN), 0);
        assert_eq!(allow(1, crate::v2::lint::FRAME_LOCAL_WARN + 1), 1);
    }

    #[test]
    fn std_global_table_reads_the_std_source() {
        let entry = std_global(272).expect("standard_entry");
        assert_eq!(
            (entry.name, entry.params, entry.result),
            ("standard_entry", 0, true)
        );
        let exit = std_global(273).expect("standard_exit");
        assert_eq!((exit.params, exit.result), (1, true));
        let child = std_global(363).expect("am_i_a_child");
        assert_eq!((child.params, child.result), (0, false));
        assert!(std_global(4096).is_none());
    }

    #[test]
    fn air_lines_map_to_trees() {
        let air = "bcon 4097 \"T\" = [1];\ntree 4096 \"A\" {\n    x;\n}\ntree 4200 \"B\" (args: 1) {\n    y;\n}\n";
        assert_eq!(air_tree_at_line(air, 1), None);
        assert_eq!(air_tree_at_line(air, 3), Some(4096));
        assert_eq!(air_tree_at_line(air, 6), Some(4200));
    }
}
