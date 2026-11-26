//! `.antics` v2 linter (`sims-antics lint`).
//!
//! Rules walk the parsed AST of a set of files that share one symbol table (enums, callee
//! return ABIs, field and identifier uses), so a use in another file of the set counts.
//! Suppression directives come from line comments, scanned here because the AST holds no
//! comments:
//!
//! - `// antics-lint: allow(rule, ...)` on its own line covers the next line of code, and at
//!   the end of a line covers that line;
//! - `//! antics-lint: allow(rule)` covers the whole file.
//!
//! `enum_wildcard_all_covered`, `raw_syntax` and `object_tree_me_field` cannot be
//! suppressed. An `allow` that suppresses nothing is itself a finding (`stale_lint_allow`).
//!
//! Some findings are kept only if their suggestion builds: `untyped_engine_constant`,
//! `prefer_question_mark`, `prefer_method_syntax`, and `untyped_engine_field` past its
//! static tables. Each one carries a source edit, and the file is built again with it. The
//! finding stands only when every object comes out with the same `.iff` bytes.
//!
//! See `docs/decisions/20261124-antics-v2-lint.md`,
//! `docs/decisions/20261125-antics-v2-new-lints.md`,
//! `docs/decisions/20261125-antics-v2-lint-wave2.md`,
//! `docs/decisions/20261125-antics-v2-lint-wave3.md`,
//! `docs/decisions/20261125-antics-v2-lint-wave4.md` and
//! `docs/decisions/20261125-antics-v2-lint-collapsible-else-if.md`.

use crate::error::{Error, Result, Span};
use crate::v2::ast::{
    CallArg, ExternFnDecl, FnDecl, ForEachIter, GuardBody, GuardDecl, MatchPattern, ObjectDecl,
    ObjectMember, ReturnAbi, StateType, V2AssignOp, V2BinOp, V2Expr, V2File, V2Item, V2Stmt,
    V2Type, V2UnaryOp,
};
use crate::v2::lexer::{TokenKind, tokenize};
use crate::v2::parser::parse_v2_file;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// How serious a [`LintDiagnostic`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// One lint finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintDiagnostic {
    pub rule: &'static str,
    pub severity: Severity,
    pub file: String,
    /// 1-based line.
    pub line: usize,
    /// 1-based byte column.
    pub column: usize,
    pub message: String,
    /// What to do about it.
    pub help: Option<String>,
    /// Replacement source text, when the fix is mechanical.
    pub suggestion: Option<String>,
    pub note: Option<String>,
}

/// A lint rule and its default severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LintRule {
    pub name: &'static str,
    pub severity: Severity,
    /// Whether `antics-lint: allow(...)` silences it.
    pub suppressible: bool,
    pub summary: &'static str,
}

const fn warn(name: &'static str, summary: &'static str) -> LintRule {
    LintRule {
        name,
        severity: Severity::Warning,
        suppressible: true,
        summary,
    }
}

pub const ENUM_WILDCARD_SINGLE_VARIANT: &str = "enum_wildcard_single_variant";
pub const ENUM_WILDCARD_ALL_COVERED: &str = "enum_wildcard_all_covered";
pub const DROPPED_BOOL_CALL: &str = "dropped_bool_call";
pub const UNBOUNDED_LOOP_WITHOUT_YIELD: &str = "unbounded_loop_without_yield";
pub const STACK_OBJ_CLOBBER_AFTER_SELF: &str = "stack_obj_clobber_after_self";
pub const UNUSED_DECL: &str = "unused_decl";
pub const FRAME_HEADROOM: &str = "frame_headroom";
/// `frame_headroom` reports a compiled tree with more nodes than this.
pub(crate) const FRAME_NODE_WARN: usize = 200;
/// `frame_headroom` reports a compiled tree with more locals than this.
pub(crate) const FRAME_LOCAL_WARN: u16 = 12;
pub const WHILE_TRUE: &str = "while_true";
pub const REDUNDANT_BOOL_RETURN: &str = "redundant_bool_return";
pub const COLLAPSIBLE_IF_CHAIN: &str = "collapsible_if_chain";
pub const COLLAPSIBLE_ELSE_IF: &str = "collapsible_else_if";
pub const MANUAL_RAW_HAS_BUILTIN: &str = "manual_raw_has_builtin";
pub const MAGIC_GUID_LITERAL: &str = "magic_guid_literal";
pub const NEGATED_COMPARISON: &str = "negated_comparison";
pub const UNKNOWN_LINT_RULE: &str = "unknown_lint_rule";
pub const UNINITIALIZED_LOCAL_READ: &str = "uninitialized_local_read";
pub const DIALOG_LOCAL_SLOT_MISMATCH: &str = "dialog_local_slot_mismatch";
pub const RAW_BARE_CALL_FALSE_EXIT: &str = "raw_bare_call_false_exit";
pub const MISSING_SIM_FRAME_ARGS: &str = "missing_sim_frame_args";
pub const SHADOWED_CONST: &str = "shadowed_const";
pub const DUPLICATE_STD_GLOBAL: &str = "duplicate_std_global";
pub const UNTYPED_ENGINE_FIELD: &str = "untyped_engine_field";
pub const RAW_SYNTAX: &str = "raw_syntax";
pub const OBJECT_TREE_ME_FIELD: &str = "object_tree_me_field";
pub const STALE_LINT_ALLOW: &str = "stale_lint_allow";
pub const UNREACHABLE_CODE: &str = "unreachable_code";
pub const UNTYPED_ENGINE_CONSTANT: &str = "untyped_engine_constant";
pub const PREFER_QUESTION_MARK: &str = "prefer_question_mark";
pub const PREFER_METHOD_SYNTAX: &str = "prefer_method_syntax";

/// Every rule `lint` knows.
pub const LINT_RULES: &[LintRule] = &[
    warn(
        ENUM_WILDCARD_SINGLE_VARIANT,
        "`_` arm of an enum match that stands for exactly one variant",
    ),
    LintRule {
        name: ENUM_WILDCARD_ALL_COVERED,
        severity: Severity::Error,
        suppressible: false,
        summary: "`_` arm of an enum match after every variant is named",
    },
    warn(
        DROPPED_BOOL_CALL,
        "a `bool` / `Result` call used as a statement, its result dropped",
    ),
    warn(
        UNBOUNDED_LOOP_WITHOUT_YIELD,
        "`loop` with no exit and nothing that yields",
    ),
    warn(
        STACK_OBJ_CLOBBER_AFTER_SELF,
        "`self` in a Sim-context body after `stack_obj` was reassigned",
    ),
    warn(UNUSED_DECL, "unused local, state, constant or BCON entry"),
    warn(
        UNINITIALIZED_LOCAL_READ,
        "a read of a `let x: T;` local before anything assigns it",
    ),
    warn(
        FRAME_HEADROOM,
        "a compiled tree close to the node limit, or with many locals",
    ),
    warn(WHILE_TRUE, "`while true` instead of `loop`"),
    warn(
        REDUNDANT_BOOL_RETURN,
        "`if c { return true; } return false;` instead of `return c;`",
    ),
    warn(
        COLLAPSIBLE_IF_CHAIN,
        "an `if`/`else if` chain testing one place against 3+ values",
    ),
    warn(
        COLLAPSIBLE_ELSE_IF,
        "an `else { .. }` holding only an `if`, instead of `else if`",
    ),
    warn(
        MANUAL_RAW_HAS_BUILTIN,
        "a `raw { }` block that v2 can spell with a builtin",
    ),
    warn(
        MAGIC_GUID_LITERAL,
        "an inline 32-bit GUID literal in a body",
    ),
    warn(
        NEGATED_COMPARISON,
        "`!(a op b)` instead of the flipped comparison",
    ),
    warn(
        DIALOG_LOCAL_SLOT_MISMATCH,
        "a `$Local:N` / `$TimeLocal:N` string token whose slot holds no `let` of the tree",
    ),
    warn(
        RAW_BARE_CALL_FALSE_EXIT,
        "a `raw { }` block ending in a bare `call N(...);`, whose false edge exits the tree",
    ),
    warn(
        MISSING_SIM_FRAME_ARGS,
        "a `(sim: Sim)` tree whose `raw { }` reads `param[n]` beyond its argument words",
    ),
    warn(
        SHADOWED_CONST,
        "a block `const`, `const` parameter or object `const` named like an outer constant",
    ),
    warn(
        DUPLICATE_STD_GLOBAL,
        "an `extern mod` function `std::globals` declares with the same ID and ABI",
    ),
    warn(
        UNTYPED_ENGINE_FIELD,
        "`global[N]`, or a `data` / `person` / `motive` / `slot` cell of `self`, a Sim or a handle, that has a typed name (`sim.skill.cooking`)",
    ),
    warn(
        UNTYPED_ENGINE_CONSTANT,
        "a number standing for a named `std::flags` bit or `std::engine` value",
    ),
    warn(
        PREFER_QUESTION_MARK,
        "`if !f(..) { panic!(); }` on a `Result` call instead of `f(..)?;`",
    ),
    warn(
        PREFER_METHOD_SYNTAX,
        "a builtin call whose target has a method form (`x.play_sound(s)`), or a flag operator on a typed flag word (`x.flags.set(F)`)",
    ),
    LintRule {
        name: RAW_SYNTAX,
        severity: Severity::Error,
        suppressible: false,
        summary: "a `raw { }` body that is not valid `.air`",
    },
    LintRule {
        name: OBJECT_TREE_ME_FIELD,
        severity: Severity::Error,
        suppressible: false,
        summary: "`me.<field>` in an object's own tree, where `me` is the object and has no fields",
    },
    warn(
        UNREACHABLE_CODE,
        "a statement after a `return`, `break`, `continue` or an `if` / `match` whose every branch leaves",
    ),
    warn(
        UNKNOWN_LINT_RULE,
        "an `antics-lint: allow(...)` directive naming no rule",
    ),
    warn(
        STALE_LINT_ALLOW,
        "an `antics-lint: allow(...)` directive that suppresses nothing",
    ),
];

fn rule(name: &str) -> Option<&'static LintRule> {
    LINT_RULES.iter().find(|r| r.name == name)
}

/// One file to lint.
#[derive(Debug, Clone, Default)]
pub struct LintInput {
    /// Name shown in diagnostics.
    pub name: String,
    pub source: String,
    /// Path on disk, used with `pack_dir` to compile the file for `frame_headroom`.
    pub path: Option<PathBuf>,
    /// The pack the file belongs to (a directory holding `pack.toml`).
    pub pack_dir: Option<PathBuf>,
}

/// Lints one self-contained source file.
pub fn lint_v2_source(source: &str, filename: &str) -> Result<Vec<LintDiagnostic>> {
    lint_v2_sources(&[LintInput {
        name: filename.to_owned(),
        source: source.to_owned(),
        path: None,
        pack_dir: None,
    }])
}

/// Lints the `.antics` files named by `paths` (directories walked for `*.antics`)
/// together. Returns the diagnostics and each file's source, keyed by its shown name.
pub fn lint_v2_paths(paths: &[PathBuf]) -> Result<(Vec<LintDiagnostic>, BTreeMap<String, String>)> {
    let files = crate::v2::fmt::collect_antics_files(paths)?;
    let mut inputs = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path)
            .map_err(|e| Error::new(format!("failed to read {}: {e}", path.display())))?;
        // `.air` shares the extension; it is not v2.
        if !crate::v2::is_v2_source(&source) {
            continue;
        }
        inputs.push(LintInput {
            name: path.display().to_string(),
            pack_dir: pack_dir_of(&path),
            path: Some(path),
            source,
        });
    }
    let diagnostics = lint_v2_sources(&inputs)?;
    let sources = inputs.into_iter().map(|i| (i.name, i.source)).collect();
    Ok((diagnostics, sources))
}

/// The nearest ancestor directory of `path` holding a `pack.toml`.
#[must_use]
pub fn pack_dir_of(path: &Path) -> Option<PathBuf> {
    let abs = std::fs::canonicalize(path).ok()?;
    abs.ancestors()
        .skip(1)
        .find(|d| d.join("pack.toml").is_file())
        .map(Path::to_path_buf)
}

/// Lints a set of files sharing one symbol table.
pub fn lint_v2_sources(inputs: &[LintInput]) -> Result<Vec<LintDiagnostic>> {
    let mut parsed = Vec::with_capacity(inputs.len());
    for input in inputs {
        let ast = parse_v2_file(&input.source).map_err(|e| {
            let at = e
                .span
                .map(|s| format!(":{}:{}", s.line, s.column))
                .unwrap_or_default();
            Error::new(format!("{}{at}: {}", input.name, e.message))
        })?;
        let tokens = tokenize(&input.source)?;
        parsed.push((input, ast, tokens));
    }
    let mut symbols = Symbols::default();
    for (input, ast, tokens) in &parsed {
        symbols.add_file(ast, tokens);
        symbols
            .bcon_paths
            .extend(bcon_path_reads(input, ast, tokens));
    }

    let mut out = Vec::new();
    for (input, ast, tokens) in &parsed {
        let directives = Directives::scan(&input.source, tokens);
        let lowered = expand_for_lint(input);
        let lower_error = lowered.as_ref().err().copied();
        let expanded = lowered.ok();
        let lowers = expanded.is_some();
        let frames = expanded.as_ref().map(|objects| {
            objects
                .iter()
                .map(|e| (e.name.clone(), parse_air_frames(&e.air_source)))
                .collect()
        });
        let compiled: Option<Vec<crate::compile::CompiledObject>> = expanded.and_then(|objects| {
            objects
                .into_iter()
                .map(|e| crate::v2::compile_expanded_v2_object(e).ok())
                .collect()
        });
        let compiles = compiled.is_some();
        let frames = frames.map(|f| verified_frames(f, compiled.as_deref()));
        let mut cx = FileLinter {
            file: &input.name,
            lines: input.source.lines().collect(),
            symbols: &symbols,
            callables: file_callables(ast, &symbols),
            sure_callables: file_sure_callables(ast),
            enums: file_enums(input, ast),
            states: file_states(ast),
            body_scope: None,
            frames,
            object: None,
            object_template: None,
            strings: BTreeMap::new(),
            dialog_strings: Vec::new(),
            tables: BTreeMap::new(),
            tree: None,
            self_handles: BTreeSet::new(),
            sim_body: false,
            body_has_defer: false,
            in_block: 0,
            consts: file_consts(ast),
            raw_bodies: directives.raw_bodies.clone(),
            line_starts: directives.line_starts.clone(),
            sims: Vec::new(),
            sim_bindings: BTreeSet::new(),
            object_me: false,
            me_bcon: symbols.bcon_names.contains("me"),
            bound: BTreeSet::new(),
            raw_declared: BTreeSet::new(),
            raw_blocks: Vec::new(),
            raw_edges: false,
            written: BTreeSet::new(),
            assigned: None,
            air_consts: BTreeMap::new(),
            source: &input.source,
            tokens,
            else_named: else_named(tokens),
            uses: &ast.uses,
            item_names: ast
                .items
                .iter()
                .filter_map(item_name)
                .map(str::to_owned)
                .collect(),
            std_globals: ast.uses.iter().any(|u| {
                (u.module_path == ["std", "globals"] && u.items.iter().any(|i| i.name == "*"))
                    || (u.module_path == ["std"]
                        && u.items
                            .iter()
                            .any(|i| i.name == "globals" && i.alias.is_none()))
            }),
            pending: Vec::new(),
            secondary: false,
            stmt: None,
            stmt_flows: false,
            body: 0,
            unbuilt: false,
            in_template: false,
            const_names: const_bindings(tokens)
                .into_iter()
                .flat_map(|b| b.names.into_iter().map(|(_, n)| n))
                .collect(),
            wide: wide_strings(tokens),
            diags: Vec::new(),
        };
        cx.lint_file(ast);
        cx.const_scopes(ast, tokens);
        cx.duplicate_std_globals(ast);
        cx.frame_headroom(ast, compiled.as_deref());
        let pending = std::mem::take(&mut cx.pending);
        let mut diags = cx.diags;
        // The two errors that cannot be allowed stand only where `check` rejects the file
        // (the lint reads lowering's rules approximately): lowering stops on the line of one
        // of them, or the `.air` it emits does not compile.
        let stops_at = |rule: &str, diags: &[LintDiagnostic]| {
            lower_error
                .flatten()
                .is_some_and(|at| diags.iter().any(|d| d.rule == rule && d.line == at.line))
        };
        let me_stands = stops_at(OBJECT_TREE_ME_FIELD, &diags);
        let raw_stands = (lowers && !compiles) || stops_at(RAW_SYNTAX, &diags);
        diags.retain(|d| match d.rule {
            OBJECT_TREE_ME_FIELD => me_stands,
            RAW_SYNTAX => raw_stands,
            _ => true,
        });
        for (line, column, name) in &directives.unknown {
            diags.push(LintDiagnostic {
                rule: UNKNOWN_LINT_RULE,
                severity: Severity::Warning,
                file: input.name.clone(),
                line: *line,
                column: *column,
                message: format!("unknown lint rule `{name}`"),
                help: Some(format!(
                    "known rules: {}",
                    LINT_RULES
                        .iter()
                        .filter(|r| r.name != UNKNOWN_LINT_RULE)
                        .map(|r| r.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                suggestion: None,
                note: None,
            });
        }
        if let Some(original) = compiled.as_deref() {
            let (stand, untried) = verified(input, original, pending, &diags);
            diags.extend(stand);
            let seen: Vec<LintDiagnostic> = diags.iter().chain(&untried).cloned().collect();
            diags.extend(stale_allows(&directives, &seen, &input.name));
        }
        for mut d in diags {
            if !directives.allows(d.rule, d.line) {
                out.push(d);
                continue;
            }
            if rule(d.rule).is_some_and(|r| r.suppressible) {
                continue;
            }
            d.note = Some(unsuppressible_note(d.rule));
            out.push(d);
        }
    }
    out.sort_by(|a, b| {
        (&a.file, a.line, a.column, a.rule).cmp(&(&b.file, b.line, b.column, b.rule))
    });
    out.dedup();
    Ok(out)
}

/// `stale_lint_allow`: each rule an `allow` names with no finding of that rule on what it
/// covers. Checked against the findings before suppression, after the compiler gate, so
/// only on a file that builds (`frame_headroom` and the frame-reading rules need the
/// build). An `allow` of an unsuppressible rule that has a finding is not stale: the
/// finding stands, with a note that it cannot be allowed.
fn stale_allows(
    directives: &Directives,
    diags: &[LintDiagnostic],
    file: &str,
) -> Vec<LintDiagnostic> {
    let mut out = Vec::new();
    for a in &directives.allows {
        if a.rule == STALE_LINT_ALLOW {
            continue;
        }
        let used = diags.iter().any(|d| {
            d.rule == a.rule
                && a.covers
                    .as_ref()
                    .is_none_or(|lines| lines.contains(&d.line))
        });
        if used {
            continue;
        }
        let what = match &a.covers {
            None => "in this file",
            Some(lines) if lines.is_empty() => "(no code follows it)",
            Some(_) => "on the code it covers",
        };
        let unsuppressible = rule(a.rule).is_some_and(|r| !r.suppressible);
        out.push(LintDiagnostic {
            rule: STALE_LINT_ALLOW,
            severity: Severity::Warning,
            file: file.to_owned(),
            line: a.line,
            column: a.column,
            message: format!(
                "`allow({})` suppresses nothing: no `{}` finding {what}",
                a.rule, a.rule
            ),
            help: Some(format!("remove `{}` from the directive", a.rule)),
            suggestion: None,
            note: unsuppressible.then(|| unsuppressible_note(a.rule)),
        });
    }
    out
}

/// Why an `allow` of the unsuppressible rule `rule` is ignored.
fn unsuppressible_note(rule: &str) -> String {
    let why = match rule {
        ENUM_WILDCARD_ALL_COVERED => {
            "the `_` arm only runs for a value that is none of the enum's variants. To handle out-of-range raw values on purpose, type the value `i16` instead of the enum."
        }
        _ => "the file does not build while it is there.",
    };
    format!("`{rule}` cannot be allowed: {why}")
}

// ---------------------------------------------------------------------------------------
// Directives
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Directives {
    file: BTreeSet<String>,
    lines: BTreeMap<usize, BTreeSet<String>>,
    /// `(line, column, name)` of rule names no rule has.
    unknown: Vec<(usize, usize, String)>,
    /// Each known rule an `allow` names, with what it covers.
    allows: Vec<Allow>,
    /// Each `raw { }` block's body: the `raw` keyword's `(line, column)` -> the byte
    /// offset of the body's first byte (after `{`) in the source.
    raw_bodies: BTreeMap<(usize, usize), usize>,
    /// The byte offset each line starts at.
    line_starts: Vec<usize>,
}

/// One rule named by one `antics-lint: allow(..)` directive.
struct Allow {
    /// The directive's comment: 1-based line and column.
    line: usize,
    column: usize,
    rule: &'static str,
    /// The lines it covers (empty when no code follows it); `None` for the whole file.
    covers: Option<std::ops::RangeInclusive<usize>>,
}

impl Directives {
    fn allows(&self, rule: &str, line: usize) -> bool {
        self.file.contains(rule) || self.lines.get(&line).is_some_and(|s| s.contains(rule))
    }

    fn scan(source: &str, tokens: &[crate::v2::lexer::Token]) -> Self {
        let lines: Vec<&str> = source.lines().collect();
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        // `raw { }` bodies follow `.air` comment and string rules, so they are scanned
        // apart (`scan_raw`): byte offset of each body's `{` -> one past its closing `}`.
        let mut raw_bodies = BTreeMap::new();
        let raw_ends: BTreeMap<usize, usize> = raw_keywords(source, tokens)
            .into_iter()
            // Keyed by the `{`: comments between `raw` and `{` are v2 comments.
            .map(|(at, (_, open, len))| {
                raw_bodies.insert(at, open + 1);
                (open, open + 1 + len + 1)
            })
            .collect();
        let mut skip_until = 0usize;
        // Per line: whether it holds code, and its line comment (column, text after `//`).
        let mut code = vec![false; lines.len()];
        let mut comments: Vec<Option<(usize, String)>> = vec![None; lines.len()];
        let mut block_depth = 0usize;
        let mut in_str = false;
        for (n, line) in lines.iter().enumerate() {
            let b = line.as_bytes();
            let mut i = 0;
            while i < b.len() {
                let offset = line_starts[n] + i;
                if offset < skip_until {
                    i += 1;
                    continue;
                }
                if block_depth > 0 {
                    if b[i..].starts_with(b"*/") {
                        block_depth -= 1;
                        i += 2;
                    } else if b[i..].starts_with(b"/*") {
                        block_depth += 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if in_str {
                    match b[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            in_str = false;
                            i += 1;
                        }
                        _ => i += 1,
                    }
                    continue;
                }
                if b[i..].starts_with(b"//") {
                    comments[n] = Some((i + 1, line[i + 2..].to_owned()));
                    break;
                }
                if b[i..].starts_with(b"/*") {
                    block_depth += 1;
                    i += 2;
                    continue;
                }
                if let Some(&end) = raw_ends.get(&offset) {
                    skip_until = end;
                    scan_raw(source, offset..end, &line_starts, &mut code, &mut comments);
                    continue;
                }
                if b[i] == b'"' {
                    in_str = true;
                }
                if !b[i].is_ascii_whitespace() {
                    code[n] = true;
                }
                i += 1;
            }
        }
        let mut out = Self {
            raw_bodies,
            line_starts: line_starts.clone(),
            ..Self::default()
        };
        for (n, comment) in comments.iter().enumerate() {
            let Some((column, text)) = comment else {
                continue;
            };
            let (inner, text) = match text.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, text.strip_prefix('/').unwrap_or(text)),
            };
            let Some(names) = parse_allow(text) else {
                continue;
            };
            let target = if inner {
                None
            } else if code[n] {
                Some(n + 1)
            } else {
                // The next line holding code.
                (n + 1..lines.len()).find(|&m| code[m]).map(|m| m + 1)
            };
            // At the end of a line that opens a block (`if c { // antics-lint: allow(..)`,
            // where `fmt` puts the comment of a one-line `if c { .. } // ..`), it covers
            // the block through its closing `}`.
            // On its own line above an item's attributes (`#[bhav(..)]`), it covers them and
            // the item's line, as it does above the item itself.
            let last_line = match target {
                Some(line) if code[n] => block_end_line(tokens, line).unwrap_or(line),
                Some(line) => attributes_end_line(tokens, line).unwrap_or(line),
                None => 0,
            };
            for name in names {
                let Some(known) = rule(&name) else {
                    out.unknown.push((n + 1, *column, name));
                    continue;
                };
                out.allows.push(Allow {
                    line: n + 1,
                    column: *column,
                    rule: known.name,
                    covers: match target {
                        None if inner => None,
                        // Empty: it covers no line.
                        None => Some(std::ops::RangeInclusive::new(1, 0)),
                        Some(line) => Some(line..=last_line.max(line)),
                    },
                });
                match target {
                    None if inner => {
                        out.file.insert(name);
                    }
                    None => {}
                    Some(line) => {
                        for l in line..=last_line.max(line) {
                            out.lines.entry(l).or_default().insert(name.clone());
                        }
                    }
                }
            }
        }
        out
    }
}

/// Marks the lines of the `raw { }` block at `range` of `source` that hold `.air` code,
/// and records each of its line comments (`// ...` or `# ...`) as [`Directives::scan`]
/// does, so `// antics-lint: allow(..)` works on a `raw` line as on any other.
fn scan_raw(
    source: &str,
    range: std::ops::Range<usize>,
    line_starts: &[usize],
    code: &mut [bool],
    comments: &mut [Option<(usize, String)>],
) {
    let line_of = |offset: usize| line_starts.partition_point(|&s| s <= offset) - 1;
    let Some(text) = source.get(range.clone()) else {
        return;
    };
    let b = text.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    let mut block_depth = 0usize;
    while i < b.len() {
        let at = range.start + i;
        if block_depth > 0 {
            if b[i..].starts_with(b"*/") {
                block_depth -= 1;
                i += 2;
            } else if b[i..].starts_with(b"/*") {
                block_depth += 1;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_str {
            match b[i] {
                b'\\' => i += 2,
                b'"' => {
                    in_str = false;
                    i += 1;
                }
                _ => i += 1,
            }
            continue;
        }
        let comment = if b[i..].starts_with(b"//") {
            Some(2)
        } else if b[i] == b'#' {
            Some(1)
        } else {
            None
        };
        if let Some(skip) = comment {
            let end = text[i..].find('\n').map_or(text.len(), |e| i + e);
            let n = line_of(at);
            // Never a file-level `//!` directive: it covers its line, like any other.
            let body = text[i + skip..end].trim_start_matches(['!', '/']);
            if let Some(slot) = comments.get_mut(n) {
                *slot = Some((at - line_starts[n] + 1, body.to_owned()));
            }
            i = end;
            continue;
        }
        if b[i..].starts_with(b"/*") {
            block_depth += 1;
            i += 2;
            continue;
        }
        if b[i] == b'"' {
            in_str = true;
        }
        if !b[i].is_ascii_whitespace()
            && let Some(c) = code.get_mut(line_of(at))
        {
            *c = true;
        }
        i += 1;
    }
}

/// When the last token on 1-based `line` is a `{`, the line of its matching `}`.
/// When `line` starts with attributes (`#[..]`), the line of the first token after them (the
/// item they belong to).
fn attributes_end_line(tokens: &[crate::v2::lexer::Token], line: usize) -> Option<usize> {
    let mut i = tokens.iter().position(|t| t.span.line == line)?;
    if tokens[i].kind != TokenKind::HashBracket {
        return None;
    }
    while tokens.get(i)?.kind == TokenKind::HashBracket {
        let mut depth = 0usize;
        loop {
            match tokens.get(i)?.kind {
                TokenKind::HashBracket | TokenKind::LBracket => depth += 1,
                TokenKind::RBracket => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    Some(tokens[i].span.line)
}

fn block_end_line(tokens: &[crate::v2::lexer::Token], line: usize) -> Option<usize> {
    let open = tokens.iter().rposition(|t| t.span.line == line)?;
    if tokens[open].kind != TokenKind::LBrace {
        return None;
    }
    // Through the matching `}`, and on through an `else` / `else if` chain.
    let mut depth = 0usize;
    let mut i = open;
    while i < tokens.len() {
        match tokens[i].kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    let is_else = tokens
                        .get(i + 1)
                        .is_some_and(|t| matches!(&t.kind, TokenKind::Ident(w) if w == "else"));
                    if !is_else {
                        return Some(tokens[i].span.line);
                    }
                    // Skip to the next arm's `{`.
                    i += 1;
                    while i < tokens.len() && tokens[i].kind != TokenKind::LBrace {
                        i += 1;
                    }
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The byte offset of the first byte at or after `at` that is not whitespace or inside a
/// `//` or (nested) `/* */` comment.
/// Each `raw { }` token's `(line, column)` -> the byte offsets of its `raw` keyword and
/// its `{`, and its body's length. Found by walking the source as the lexer does: its
/// columns count bytes, except inside a v2 string, where they count characters.
fn raw_keywords(
    source: &str,
    tokens: &[crate::v2::lexer::Token],
) -> BTreeMap<(usize, usize), (usize, usize, usize)> {
    let mut wanted: BTreeMap<(usize, usize), usize> = tokens
        .iter()
        .filter_map(|t| match &t.kind {
            TokenKind::RawBlock(body) => Some(((t.span.line, t.span.column), body.len())),
            _ => None,
        })
        .collect();
    let mut out = BTreeMap::new();
    let b = source.as_bytes();
    let (mut at, mut line, mut col) = (0usize, 1usize, 1usize);
    // Moves on to byte `to`, counting lines and byte columns.
    let advance = |at: &mut usize, line: &mut usize, col: &mut usize, to: usize| {
        while *at < to.min(b.len()) {
            if b[*at] == b'\n' {
                *line += 1;
                *col = 1;
            } else {
                *col += 1;
            }
            *at += 1;
        }
    };
    while at < b.len() && !wanted.is_empty() {
        if let Some(len) = wanted.remove(&(line, col))
            && b[at..].starts_with(b"raw")
        {
            let open = skip_ws_and_comments(source, at + 3);
            if b.get(open) == Some(&b'{') {
                out.insert((line, col), (at, open, len));
                advance(&mut at, &mut line, &mut col, open + 1 + len + 1);
                continue;
            }
        }
        match b[at] {
            b'"' => {
                // A v2 string: one column per character.
                col += 1;
                at += 1;
                while at < b.len() && b[at] != b'"' && b[at] != b'\n' {
                    if b[at] == b'\\' {
                        col += 1;
                        at += 1;
                    }
                    let c = source
                        .get(at..)
                        .and_then(|r| r.chars().next())
                        .map_or(1, char::len_utf8);
                    col += 1;
                    at += c;
                }
                if b.get(at) == Some(&b'"') {
                    col += 1;
                    at += 1;
                }
            }
            b'/' if matches!(b.get(at + 1), Some(b'/' | b'*')) => {
                let to = skip_ws_and_comments(source, at).max(at + 1);
                advance(&mut at, &mut line, &mut col, to);
            }
            _ => {
                let to = at + 1;
                advance(&mut at, &mut line, &mut col, to);
            }
        }
    }
    out
}

fn skip_ws_and_comments(source: &str, mut at: usize) -> usize {
    let b = source.as_bytes();
    loop {
        while at < b.len() && b[at].is_ascii_whitespace() {
            at += 1;
        }
        if b[at..].starts_with(b"//") {
            while at < b.len() && b[at] != b'\n' {
                at += 1;
            }
        } else if b[at..].starts_with(b"/*") {
            let mut depth = 0usize;
            while at < b.len() {
                if b[at..].starts_with(b"/*") {
                    depth += 1;
                    at += 2;
                } else if b[at..].starts_with(b"*/") {
                    depth -= 1;
                    at += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    at += 1;
                }
            }
        } else {
            return at;
        }
    }
}

/// `antics-lint: allow(a, b)` → `[a, b]`.
fn parse_allow(text: &str) -> Option<Vec<String>> {
    let rest = text.trim().strip_prefix("antics-lint:")?.trim();
    let inner = rest.strip_prefix("allow")?.trim().strip_prefix('(')?;
    let inner = &inner[..inner.find(')')?];
    Some(
        inner
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------
// Symbols shared by the file set
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RetKind {
    Bool,
    Result,
    Other,
}

#[derive(Default)]
struct Symbols {
    /// Callee (`name` or `module::name`) -> return kind (`None` when definitions disagree).
    returns: BTreeMap<String, Option<RetKind>>,
    /// Every identifier written right after a `.` (fields, methods, BCON entries).
    field_names: BTreeSet<String>,
    /// Every two-segment path `A::b` that may read a BCON entry, as `(body, A, b)` with
    /// `body` `object:<file>:Name`, `template:Name`, or `module` (outside any body). Filled
    /// per file by [`bcon_path_reads`].
    bcon_paths: BTreeSet<(String, String, String)>,
    /// The names of BCON tables (`bcon` labels, `extern bcon` names), anywhere in the set.
    bcon_names: BTreeSet<String>,
    /// The templates the file set declares.
    template_names: BTreeSet<String>,
    /// The functions and trees templates declare: an object that extends one, in any file,
    /// calls them by their bare names.
    template_fns: BTreeSet<String>,
    /// Literal `attr[N]` indices.
    attr_indices: BTreeSet<i64>,
    /// Identifier-like words in `raw { }` code (not its comments or strings), less the
    /// names a block declares as its own `.air` constants: lowering resolves every other
    /// v2 `const` a raw block names.
    raw_words: BTreeSet<String>,
    /// How often each identifier is written outside a `const` declaration's name (the
    /// uses a constant can have; a `raw` block's are in `raw_words`).
    const_uses: BTreeMap<String, usize>,
    /// Whether code outside the file set's names may read BCON entries by number: a `raw`
    /// `bcon[N]`, or an `extern` tree (globals and semi-globals can read an object's
    /// private tables).
    bcon_by_number: bool,
}

impl Symbols {
    fn add_file(&mut self, file: &V2File, tokens: &[crate::v2::lexer::Token]) {
        // A `use` may import a BCON table named `me` (not resolved here): an imported `me`,
        // or a glob from outside `std`.
        for u in &file.uses {
            for item in &u.items {
                let glob = item.name == "*" && u.module_path.first().is_none_or(|m| m != "std");
                if glob || item.alias.as_deref().unwrap_or(&item.name) == "me" {
                    self.bcon_names.insert("me".to_owned());
                }
            }
        }
        let declared: BTreeSet<usize> = const_bindings(tokens)
            .into_iter()
            .flat_map(|b| b.names.into_iter().map(|(i, _)| i))
            .collect();
        for (i, t) in tokens.iter().enumerate() {
            if let TokenKind::Ident(name) = &t.kind
                && !declared.contains(&i)
            {
                *self.const_uses.entry(name.clone()).or_default() += 1;
            }
            match &t.kind {
                TokenKind::Ident(name) => {
                    if i > 0 && tokens[i - 1].kind == TokenKind::Dot {
                        self.field_names.insert(name.clone());
                    }
                    if name == "attr"
                        && let (Some(open), Some(idx)) = (tokens.get(i + 1), tokens.get(i + 2))
                        && open.kind == TokenKind::LBracket
                        && let TokenKind::Int(n) = idx.kind
                    {
                        self.attr_indices.insert(n);
                    }
                }
                TokenKind::RawBlock(code) => {
                    // `.air` reads a BCON entry as `bcon[N]` or `const[N]`.
                    self.bcon_by_number |= raw_indexes(code, "bcon") || raw_indexes(code, "const");
                    // Names and indices in `.air` code, not in its comments or strings
                    // (a quoted tree label may read `attr[7]`).
                    let outside = air_code_only(code);
                    self.raw_words
                        .extend(air_name_uses(&outside).into_iter().map(str::to_owned));
                    // A BCON entry read by name: `Table.entry` or `Table::entry`.
                    self.field_names
                        .extend(air_members(&outside).into_iter().map(str::to_owned));
                    for (pos, _) in code.match_indices("attr[") {
                        let digits: String = code[pos + 5..]
                            .chars()
                            .take_while(char::is_ascii_digit)
                            .collect();
                        if let Ok(n) = digits.parse() {
                            self.attr_indices.insert(n);
                        }
                    }
                }
                _ => {}
            }
        }
        let add_members = |members: &[ObjectMember], this: &mut Self| {
            for m in members {
                match m {
                    ObjectMember::Bcon(b) => {
                        this.bcon_names.extend(bcon_names(&b.label));
                    }
                    ObjectMember::ExternBcon(b) => {
                        this.bcon_names.insert(b.name.clone());
                    }
                    ObjectMember::Fn(f) => this.add_fn(f),
                    ObjectMember::Tree(t) => this.add_return(&t.ident, ret_kind(&t.ret)),
                    ObjectMember::ExternMod(m) => {
                        this.bcon_by_number = true;
                        for f in &m.fns {
                            this.add_return(&format!("{}::{}", m.name, f.name), ret_kind(&f.ret));
                        }
                    }
                    _ => {}
                }
            }
        };
        for item in &file.items {
            match item {
                V2Item::Fn(f) => self.add_fn(f),
                V2Item::ExternMod(m) => {
                    self.bcon_by_number = true;
                    for f in &m.fns {
                        self.add_return(&format!("{}::{}", m.name, f.name), ret_kind(&f.ret));
                    }
                }
                V2Item::ExternBcon(b) => {
                    self.bcon_names.insert(b.name.clone());
                }
                V2Item::Object(o) => add_members(&o.members, self),
                V2Item::Template(t) => {
                    self.template_names.insert(t.name.clone());
                    for m in &t.members {
                        match m {
                            ObjectMember::Fn(f) => {
                                self.template_fns.insert(f.name.clone());
                            }
                            ObjectMember::Tree(t) => {
                                self.template_fns.insert(t.ident.clone());
                            }
                            _ => {}
                        }
                    }
                    add_members(&t.members, self);
                }
                _ => {}
            }
        }
    }

    fn add_fn(&mut self, f: &FnDecl) {
        let kind = match f.return_type {
            Some(V2Type::Bool) => RetKind::Bool,
            _ => RetKind::Other,
        };
        self.add_return(&f.name, kind);
    }

    fn add_return(&mut self, name: &str, kind: RetKind) {
        self.returns
            .entry(name.to_owned())
            .and_modify(|k| {
                if *k != Some(kind) {
                    *k = None;
                }
            })
            .or_insert(Some(kind));
    }

    fn user_defined(&self, name: &str) -> bool {
        self.returns.contains_key(name)
    }

    /// The return kind of a call, when it is known and unambiguous.
    fn call_kind(&self, callee: &V2Expr) -> Option<(String, RetKind)> {
        let name = match callee {
            V2Expr::Ident(_, n) => n.clone(),
            V2Expr::Path(_, segs) => segs.join("::"),
            _ => return None,
        };
        if let Some(kind) = self.returns.get(&name) {
            return kind.map(|k| (name, k));
        }
        match name.as_str() {
            // Pure: a dropped result makes the call pointless.
            "can_afford" => Some((name, RetKind::Bool)),
            _ => None,
        }
    }
}

/// A `#[bhav] fn` or `extern` fn's return kind. A `-> bool` tree is often run for what it does
/// (a test that also acts), so only a dropped `Result` counts for these.
fn ret_kind(r: &ReturnAbi) -> RetKind {
    match r {
        ReturnAbi::Bool => RetKind::Other,
        ReturnAbi::Result | ReturnAbi::ResultValue { .. } => RetKind::Result,
        ReturnAbi::Void | ReturnAbi::Value { .. } => RetKind::Other,
    }
}

// ---------------------------------------------------------------------------------------
// Walking
// ---------------------------------------------------------------------------------------

/// The expressions a statement holds directly (not those of nested statements).
fn stmt_exprs(s: &V2Stmt) -> Vec<&V2Expr> {
    match s {
        V2Stmt::Let { init, .. } | V2Stmt::LetSome { init, .. } => vec![init],
        V2Stmt::Assign { target, value, .. } => vec![target, value],
        V2Stmt::WithLock { lock_expr, .. } => vec![lock_expr],
        V2Stmt::Repeat { count, .. } => vec![count],
        V2Stmt::While { cond, .. } | V2Stmt::If { cond, .. } => vec![cond],
        V2Stmt::ForEach { iter, .. } => match iter {
            ForEachIter::ObjectsByGuid(e) => vec![e],
            ForEachIter::Range { start, end, .. } => vec![start, end],
            _ => Vec::new(),
        },
        V2Stmt::Match { scrutinee, .. } => vec![scrutinee],
        V2Stmt::Return { value: Some(v), .. } => vec![v],
        V2Stmt::Expr { expr, .. } => vec![expr],
        V2Stmt::Animate { args, .. } => args.iter().map(|a| &a.value).collect(),
        _ => Vec::new(),
    }
}

/// The statement lists nested in a statement.
fn stmt_blocks(s: &V2Stmt) -> Vec<&[V2Stmt]> {
    match s {
        V2Stmt::LetSome { else_body, .. } => vec![else_body],
        V2Stmt::WithLock { body, .. }
        | V2Stmt::Defer { body, .. }
        | V2Stmt::Repeat { body, .. }
        | V2Stmt::While { body, .. }
        | V2Stmt::Animate { body, .. } => vec![body],
        V2Stmt::ForEach {
            body, else_body, ..
        } => std::iter::once(body.as_slice())
            .chain(else_body.as_deref())
            .collect(),
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => vec![then_body, else_body],
        V2Stmt::Match { arms, .. } => arms.iter().map(|a| a.body.as_slice()).collect(),
        // The diverging `{ ..; return ..; }` branches of `let x = if .. / match ..`.
        V2Stmt::Let { init, .. } => crate::v2::specialize::init_blocks(init),
        _ => Vec::new(),
    }
}

fn args_exprs(args: &[CallArg]) -> impl Iterator<Item = &V2Expr> {
    args.iter().map(|a| &a.value)
}

fn subexprs(e: &V2Expr) -> Vec<&V2Expr> {
    match e {
        V2Expr::Tuple(_, items) => items.iter().collect(),
        V2Expr::Field { base, .. } => vec![base],
        V2Expr::Index { base, index, .. } => vec![base, index],
        V2Expr::Deref { inner, .. }
        | V2Expr::MutRef { inner, .. }
        | V2Expr::Unary { inner, .. }
        | V2Expr::Try { inner, .. } => vec![inner],
        V2Expr::Binary { lhs, rhs, .. } => vec![lhs, rhs],
        V2Expr::Call { callee, args, .. } => std::iter::once(callee.as_ref())
            .chain(args_exprs(args))
            .collect(),
        V2Expr::MethodCall { receiver, args, .. } => std::iter::once(receiver.as_ref())
            .chain(args_exprs(args))
            .collect(),
        V2Expr::Match {
            scrutinee, arms, ..
        } => std::iter::once(scrutinee.as_ref())
            .chain(arms.iter().map(|a| &a.value))
            .collect(),
        V2Expr::If {
            cond,
            then_value,
            else_value,
            ..
        } => vec![cond, then_value, else_value],
        _ => Vec::new(),
    }
}

fn walk_expr<'a>(e: &'a V2Expr, f: &mut dyn FnMut(&'a V2Expr)) {
    f(e);
    for c in subexprs(e) {
        walk_expr(c, f);
    }
}

fn walk_stmts<'a>(list: &'a [V2Stmt], f: &mut dyn FnMut(&'a V2Stmt)) {
    for s in list {
        f(s);
        for b in stmt_blocks(s) {
            walk_stmts(b, f);
        }
    }
}

/// Whether any expression in `list` (nested statements included) satisfies `pred`.
fn any_expr(list: &[V2Stmt], pred: &mut dyn FnMut(&V2Expr) -> bool) -> bool {
    let mut found = false;
    walk_stmts(list, &mut |s| {
        for e in stmt_exprs(s) {
            walk_expr(e, &mut |x| found |= pred(x));
        }
    });
    found
}

fn is_ident(e: &V2Expr, name: &str) -> bool {
    matches!(e, V2Expr::Ident(_, n) if n == name)
}

fn mentions_ident(e: &V2Expr, name: &str) -> Option<Span> {
    let mut at = None;
    walk_expr(e, &mut |x| {
        if at.is_none() && is_ident(x, name) {
            at = Some(x.span());
        }
    });
    at
}

/// Renders an expression as source text (`None` for forms not worth spelling out).
/// `text` as a v2 string literal, or `None` when it holds a control character v2's
/// escapes (`\"`, `\\`, `\n`, `\r`, `\t`, `\0`) do not spell.
fn v2_string(text: &str) -> Option<String> {
    let mut out = String::from('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if c.is_control() => return None,
            c => out.push(c),
        }
    }
    out.push('"');
    Some(out)
}

fn print_expr(e: &V2Expr) -> Option<String> {
    let child = |c: &V2Expr| -> Option<String> {
        let s = print_expr(c)?;
        Some(if matches!(c, V2Expr::Binary { .. } | V2Expr::If { .. }) {
            format!("({s})")
        } else {
            s
        })
    };
    let args = |args: &[CallArg]| -> Option<String> {
        let mut out = Vec::new();
        for a in args {
            let v = print_expr(&a.value)?;
            out.push(match &a.name {
                Some(n) => format!("{n}: {v}"),
                None => v,
            });
        }
        Some(out.join(", "))
    };
    Some(match e {
        V2Expr::Int(_, v) => v.to_string(),
        V2Expr::Bool(_, b) => b.to_string(),
        V2Expr::Str(_, s) => v2_string(s)?,
        V2Expr::Ident(_, n) => n.clone(),
        V2Expr::Path(_, segs) => segs.join("::"),
        V2Expr::Field { base, field, .. } => format!("{}.{field}", child(base)?),
        V2Expr::Index { base, index, .. } => format!("{}[{}]", child(base)?, print_expr(index)?),
        V2Expr::Deref { inner, .. } => format!("*{}", child(inner)?),
        V2Expr::MutRef { inner, .. } => format!("&mut {}", child(inner)?),
        V2Expr::Unary { op, inner, .. } => {
            let op = match op {
                V2UnaryOp::Not => "!",
                V2UnaryOp::Neg => "-",
                V2UnaryOp::AsI16 => return Some(format!("{} as i16", child(inner)?)),
            };
            format!("{op}{}", child(inner)?)
        }
        V2Expr::Binary { lhs, op, rhs, .. } => {
            format!("{} {} {}", child(lhs)?, binop_str(*op), child(rhs)?)
        }
        V2Expr::Call {
            callee,
            args: a,
            type_args,
            ..
        } if type_args.is_empty() => {
            format!("{}({})", print_expr(callee)?, args(a)?)
        }
        V2Expr::MethodCall {
            receiver,
            method,
            args: a,
            type_args,
            ..
        } if type_args.is_empty() => format!("{}.{method}({})", child(receiver)?, args(a)?),
        V2Expr::Try { inner, .. } => format!("{}?", child(inner)?),
        V2Expr::If {
            cond,
            then_value,
            else_value,
            ..
        } => {
            let tail = match else_value.as_ref() {
                V2Expr::If { .. } => print_expr(else_value)?,
                other => format!("{{ {} }}", print_expr(other)?),
            };
            format!(
                "if {} {{ {} }} else {tail}",
                print_expr(cond)?,
                print_expr(then_value)?
            )
        }
        _ => return None,
    })
}

fn binop_str(op: V2BinOp) -> &'static str {
    match op {
        V2BinOp::Or => "||",
        V2BinOp::And => "&&",
        V2BinOp::Eq => "==",
        V2BinOp::Ne => "!=",
        V2BinOp::Lt => "<",
        V2BinOp::Le => "<=",
        V2BinOp::Gt => ">",
        V2BinOp::Ge => ">=",
        V2BinOp::FlagTest => "flag_test",
        V2BinOp::Add => "+",
        V2BinOp::Sub => "-",
        V2BinOp::Mul => "*",
        V2BinOp::Div => "/",
        V2BinOp::Mod => "%",
        V2BinOp::BitAnd => "&",
        V2BinOp::BitOr => "|",
        V2BinOp::BitXor => "^",
    }
}

/// The comparison that `!(a op b)` equals.
fn flipped(op: V2BinOp) -> Option<V2BinOp> {
    Some(match op {
        V2BinOp::Eq => V2BinOp::Ne,
        V2BinOp::Ne => V2BinOp::Eq,
        V2BinOp::Lt => V2BinOp::Ge,
        V2BinOp::Le => V2BinOp::Gt,
        V2BinOp::Gt => V2BinOp::Le,
        V2BinOp::Ge => V2BinOp::Lt,
        _ => return None,
    })
}

/// A side-effect-free place (`x`, `self.a`, `temp[2]`, `Table.e`) as text.
fn place_key(e: &V2Expr) -> Option<String> {
    match e {
        V2Expr::Ident(_, n) => Some(n.clone()),
        V2Expr::Path(_, segs) => Some(segs.join("::")),
        V2Expr::Field { base, field, .. } => Some(format!("{}.{field}", place_key(base)?)),
        V2Expr::Index { base, index, .. } => {
            let idx = match index.as_ref() {
                V2Expr::Int(_, v) => v.to_string(),
                other => place_key(other)?,
            };
            Some(format!("{}[{idx}]", place_key(base)?))
        }
        _ => None,
    }
}

/// A value a `match` arm can name: a literal, a const, or an enum path.
fn pattern_text(e: &V2Expr) -> Option<String> {
    match e {
        V2Expr::Int(_, v) => Some(v.to_string()),
        V2Expr::Unary {
            op: V2UnaryOp::Neg,
            inner,
            ..
        } => match inner.as_ref() {
            V2Expr::Int(_, v) => Some(format!("-{v}")),
            _ => None,
        },
        V2Expr::Path(_, segs) if segs.len() == 2 => Some(segs.join("::")),
        // A `match` arm takes no other identifier, not even a `const`.
        _ => None,
    }
}

/// Calls inside a `loop` that never yield (and are not shadowed by a user definition).
const NON_YIELDING_CALLS: &[&str] = &[
    "random",
    "random_into",
    "can_afford",
    "debit_funds",
    "credit_funds",
    "play_sound",
    "stop_all_sounds",
    "refresh",
    "set_graphic",
    "set_motive_change",
    "clear_motive_changes",
    "action_label",
    "set_dynamic_sprite",
    "inc_lt",
    "dec_gt",
    "breakpoint",
    "show_string",
    "set_headline",
    "special_effect",
    "Some",
];

/// Methods that never yield.
const NON_YIELDING_METHODS: &[&str] = &[
    "is_type",
    "set",
    "clear",
    "contains",
    "is_empty",
    "play_sound",
    "stop_all_sounds",
    "refresh",
    "debit_funds",
    "credit_funds",
];

/// `.air` primitives a v2 builtin spells with the same operands, with that spelling.
///
/// Primitives whose v2 spelling takes different operands are left out, since a raw block
/// holding them is often the only way to say it: an unpadded `play_sound` (v2 names a
/// declared `sound`), `stop_all_sounds` and `push_interaction`
/// (v2 picks the target from the tree's context), `test_object_type` as a statement,
/// `set_to_next` outside a loop, and `stack_obj :=` (not every source has a v2 spelling).
/// `play_sound` with `pad(..)` bytes, `clear_motive_changes` and `reach` are judged operand by
/// operand in `FileLinter::raw_native_form`.
const RAW_BUILTINS: &[(&str, &str)] = &[
    ("action_string", "action_label(...)"),
    ("refresh", "refresh(target: .., mode: ..)"),
    ("go_to_relative", "go_to_relative(...)"),
    ("run_tree_by_name", "run_tree_by_name(...)"),
    ("manage_inventory", "manage_inventory(...)"),
    ("show_string", "show_string(...)"),
    ("set_headline", "set_headline(...)"),
    ("special_effect", "special_effect(...)"),
    ("breakpoint", "breakpoint(...)"),
    ("notify_idle", "notify_out_of_idle()"),
];

// ---------------------------------------------------------------------------------------
// Per-file rules
// ---------------------------------------------------------------------------------------

struct FileLinter<'a> {
    file: &'a str,
    lines: Vec<&'a str>,
    symbols: &'a Symbols,
    /// The bare names a call in this file may resolve to a user function or tree by (see
    /// [`file_callables`]); `None` when a glob import may bring in any name.
    callables: Option<BTreeSet<String>>,
    /// The bare names a call in this file surely resolves to a user function or tree by:
    /// the file's own functions and trees, and the names it imports by name.
    sure_callables: BTreeSet<String>,
    /// Enums this file can name (see `file_enums`).
    enums: BTreeMap<String, Option<Vec<String>>>,
    /// The file's object states by name, with their type (`None` when two disagree).
    states: BTreeMap<String, Option<StateType>>,
    /// What the linted object's body declares, with its template's (`None` outside an
    /// object, or when the template is in another file).
    body_scope: Option<BodyScope>,
    /// The lowered `.air` frames of each object the file expands to, by object name
    /// (`None` when the file does not expand on its own).
    frames: Option<BTreeMap<String, Vec<AirFrame>>>,
    /// The object being linted.
    object: Option<String>,
    /// The template it is built `from`, if any.
    object_template: Option<String>,
    /// The linted object's `strings` entries: name -> text.
    strings: BTreeMap<String, String>,
    /// Its dialog strings (`STR#` 301), in order: a dialog's numeric string is 1-based.
    dialog_strings: Vec<String>,
    /// Its `strings` tables by ID, each entry's text in order.
    tables: BTreeMap<i64, Vec<String>>,
    /// The tree being linted.
    tree: Option<TreeCx>,
    /// Locals holding this object's own handle (`let saved = stack_obj;` before any
    /// reassignment), in the Sim-context body `clobber_walk` is in.
    self_handles: BTreeSet<String>,
    /// The body being linted is one a Sim runs (`self` follows `stack_obj`).
    sim_body: bool,
    /// The body being linted holds a `defer`.
    body_has_defer: bool,
    /// How many `with lock`, `defer` or `animate` blocks the list being linted is in:
    /// a `return` there lowers through the block's own exit.
    in_block: usize,
    /// The file's `const` names (top level and object members).
    consts: BTreeSet<String>,
    /// Where each `raw { }` body starts in the source ([`Directives::raw_bodies`]).
    raw_bodies: BTreeMap<(usize, usize), usize>,
    /// The byte offset each source line starts at.
    line_starts: Vec<usize>,
    /// The names that are the running Sim in the body being linted: its Sim binding, and
    /// `me` where the frame's `me` is the Sim (an interaction, a functional, a tree with a
    /// Sim parameter or `context = inherit`).
    sims: Vec<String>,
    /// The body's own Sim bindings (its Sim parameters or `|sim|` names), which shadow a
    /// builtin name such as `param` or `me` like any other binding.
    sim_bindings: BTreeSet<String>,
    /// The body being linted is an object's own tree (no Sim parameter, not `context =
    /// inherit`), where `me` is the object itself.
    object_me: bool,
    /// A file of the set names a BCON table `me`, so `me.x` may read it.
    me_bcon: bool,
    /// The names the body being linted binds itself (`let`s, `for` bindings, non-Sim
    /// parameters): such a `me`, `self` or Sim name is not the frame's.
    bound: BTreeSet<String>,
    /// The names the body's `raw` blocks declare as `.air` locals (`let` / `local`), which
    /// every raw block of the tree shares.
    raw_declared: BTreeSet<String>,
    /// A `raw` block of the body has an explicit edge (`->`), which may jump to any later
    /// statement (a label, or a node by number).
    raw_edges: bool,
    /// The body's `raw` blocks (span, code), in order, checked together when it ends.
    raw_blocks: Vec<(Span, String)>,
    /// Addresses of the expressions the statement being linted writes (an assignment's
    /// target, a `&mut` operand, a builtin's out argument).
    written: BTreeSet<usize>,
    /// Address of the value the statement being linted assigns as a whole.
    assigned: Option<usize>,
    /// The constants a `raw` block of the object being linted may name, as lowering
    /// declares them for it (`NAME` or `TABLE.entry` -> `.air` value).
    air_consts: BTreeMap<String, String>,
    /// The file's source.
    source: &'a str,
    /// Its tokens (comments are not tokens; a `raw { }` body is one).
    tokens: &'a [crate::v2::lexer::Token],
    /// Whether some `else` in the file is a name, not the keyword (see [`else_named`]).
    else_named: bool,
    /// Its `use` declarations.
    uses: &'a [crate::v2::ast::UseDecl],
    /// The names of its top-level items.
    item_names: BTreeSet<String>,
    /// It imports `std::globals` (`globals::f` calls a `std::globals` tree).
    std_globals: bool,
    /// Findings that stand only if their rewrite builds the same bytes ([`verified`]).
    pending: Vec<Pending>,
    /// What is proposed now is secondary (see [`Pending::secondary`]).
    secondary: bool,
    /// The statement being linted.
    stmt: Option<Span>,
    /// Whether that statement binds names (see [`Pending::flows`]).
    stmt_flows: bool,
    /// How many bodies have been linted: the current body's number.
    body: usize,
    /// The code being linted is not built as itself: a plain (inlined) `fn`, which other
    /// files may inline too, or a template's member, built only into the objects that
    /// extend it. Its rewrites could not be checked, so none is proposed.
    unbuilt: bool,
    /// The code being linted is a template's member: a bare call in it may resolve to a
    /// member of an object in another file that extends the template.
    in_template: bool,
    /// Every name a `const` binds anywhere in the file (file, member, block and `const`
    /// parameters). Parsing substitutes a block constant for its name, so a spelling rooted
    /// at one would read the constant.
    const_names: BTreeSet<String>,
    /// Per line, each string literal's column and how many more bytes than characters it
    /// holds: the lexer counts a string's characters, everything else by bytes.
    wide: BTreeMap<usize, Vec<(usize, usize)>>,
    diags: Vec<LintDiagnostic>,
}

/// One lowered `.air` tree's frame.
#[derive(Debug, Clone)]
struct AirFrame {
    id: u16,
    label: String,
    /// What each local slot holds: `local[N]` for a raw slot, else the `.air` name.
    slots: Vec<String>,
    /// Slots `raw` blocks address by number (`0..raw_slots`).
    raw_slots: usize,
    /// Its argument count, from the `(args: N, ..)` header.
    args: Option<usize>,
}

/// The tree a body belongs to, for the rules that read its compiled frame.
struct TreeCx {
    /// Its `.air` frame (`None` when the file does not expand, or the tree is not found).
    frame: Option<AirFrame>,
    /// `$Local:N` / `$TimeLocal:N` slots its strings read: slot -> (span of the string's
    /// use, the token, where the string comes from).
    tokens: BTreeMap<usize, Vec<(Span, String, String)>>,
    /// Names of the `let`s its body declares.
    lets: BTreeSet<String>,
    /// It shows strings by number from a table this file does not declare (a dialog's
    /// `STR#` 301 entry past the file's own, an `action_label` of another table): their
    /// `$Local:N` tokens are unknown, so any of its locals may be read by one.
    unknown_strings: bool,
}

/// The bare names a call in `file` may resolve to a user function or tree by, which then
/// takes the call over a builtin of the same name: the file's own functions and trees, the
/// names it imports (by alias when it has one, and a `std` module's `pub fn`s for its glob; see
/// [`std_glob_fns`]), and every template's members (an object may
/// extend a template of another file). `None` when a glob import other than `std::*` may bring
/// in any name, or an object extends a template the linted files do not declare (its members
/// are unknown). Another file's private methods are not visible here.
fn file_callables(file: &V2File, symbols: &Symbols) -> Option<BTreeSet<String>> {
    let unknown_template = file.items.iter().any(|item| {
        matches!(item, V2Item::Object(o)
            if o.from_template.as_ref().is_some_and(|(t, _)| !symbols.template_names.contains(t)))
    });
    if unknown_template {
        return None;
    }
    let mut out = symbols.template_fns.clone();
    for u in &file.uses {
        for item in &u.items {
            if item.name == "*" {
                if u.module_path.first().map(String::as_str) != Some("std") {
                    return None;
                }
                out.extend(std_glob_fns(&u.module_path).iter().cloned());
            } else {
                out.insert(item.alias.clone().unwrap_or_else(|| item.name.clone()));
            }
        }
    }
    let add_members = |members: &[ObjectMember], out: &mut BTreeSet<String>| {
        for m in members {
            match m {
                ObjectMember::Fn(f) => {
                    out.insert(f.name.clone());
                }
                ObjectMember::Tree(t) => {
                    out.insert(t.ident.clone());
                }
                _ => {}
            }
        }
    };
    for item in &file.items {
        match item {
            V2Item::Fn(f) => {
                out.insert(f.name.clone());
            }
            V2Item::Object(o) => add_members(&o.members, &mut out),
            V2Item::Template(t) => add_members(&t.members, &mut out),
            _ => {}
        }
    }
    Some(out)
}

/// The bare names a call in `file` surely resolves to a user function or tree by: its own
/// functions and trees, the names it imports by name (by alias when it has one), and the
/// `pub fn`s a `use std::<module>::*;` brings in.
fn file_sure_callables(file: &V2File) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for u in &file.uses {
        for item in &u.items {
            if item.name == "*" {
                out.extend(std_glob_fns(&u.module_path).iter().cloned());
            } else {
                out.insert(item.alias.clone().unwrap_or_else(|| item.name.clone()));
            }
        }
    }
    for item in &file.items {
        let members = match item {
            V2Item::Fn(f) => {
                out.insert(f.name.clone());
                continue;
            }
            V2Item::Object(o) => &o.members,
            V2Item::Template(t) => &t.members,
            _ => continue,
        };
        for m in members {
            match m {
                ObjectMember::Fn(f) => {
                    out.insert(f.name.clone());
                }
                ObjectMember::Tree(t) => {
                    out.insert(t.ident.clone());
                }
                _ => {}
            }
        }
    }
    out
}

/// A file's object and template states by name, with their type (`None` when two
/// declarations disagree).
fn file_consts(file: &V2File) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for item in &file.items {
        match item {
            V2Item::Const(c) => {
                out.insert(c.name.clone());
            }
            V2Item::Object(o) => {
                for m in &o.members {
                    if let ObjectMember::Const(c) = m {
                        out.insert(c.name.clone());
                    }
                }
            }
            V2Item::Template(t) => {
                for m in &t.members {
                    if let ObjectMember::Const(c) = m {
                        out.insert(c.name.clone());
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The enum and bitflags types and the states an object's body sees as members.
struct BodyScope {
    types: BTreeSet<String>,
    states: BTreeSet<String>,
}

/// The members object `o` sees: its own and its template's. `None` when the template is
/// not in `file`: its members are unknown here.
fn body_scope(file: &V2File, o: &ObjectDecl) -> Option<BodyScope> {
    let mut members = vec![o.members.as_slice()];
    if let Some((t, _)) = &o.from_template {
        let template = file.items.iter().find_map(|item| match item {
            V2Item::Template(d) if d.name == *t => Some(d),
            _ => None,
        })?;
        members.push(&template.members);
    }
    Some(BodyScope {
        types: members.iter().flat_map(|m| member_type_names(m)).collect(),
        states: members
            .iter()
            .flat_map(|m| m.iter())
            .filter_map(|m| match m {
                ObjectMember::State(st) => Some(st.name.clone()),
                _ => None,
            })
            .collect(),
    })
}

/// Whether `raw` is the cell `root.cell[..]` as written, spaces aside: not
/// `(root).cell[..]`, whose range would start inside the parentheses.
fn cell_text_is(raw: &str, root: &str, cell: &str) -> bool {
    let blank = |t: &str| t.trim_start_matches([' ', '\t', '\r']).to_owned();
    let Some(rest) = raw.strip_prefix(root).map(blank) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('.').map(blank) else {
        return false;
    };
    rest.strip_prefix(cell)
        .map(blank)
        .is_some_and(|rest| rest.starts_with('['))
}

fn file_states(file: &V2File) -> BTreeMap<String, Option<StateType>> {
    let mut out: BTreeMap<String, Option<StateType>> = BTreeMap::new();
    for item in &file.items {
        let members = match item {
            V2Item::Object(o) => &o.members,
            V2Item::Template(t) => &t.members,
            _ => continue,
        };
        for m in members {
            if let ObjectMember::State(st) = m {
                out.entry(st.name.clone())
                    .and_modify(|t| {
                        if t.as_ref() != Some(&st.ty) {
                            *t = None;
                        }
                    })
                    .or_insert_with(|| Some(st.ty.clone()));
            }
        }
    }
    out
}

/// The enums a file can name: its own declarations, then its `use` imports (under their
/// alias) as the module loader resolves them. A name declared twice with different
/// variants maps to `None`. Imports are left out when the file's imports do not resolve
/// (no pack on disk), since a same-named enum elsewhere may not be the one imported.
fn file_enums(input: &LintInput, file: &V2File) -> BTreeMap<String, Option<Vec<String>>> {
    let mut local = BTreeMap::new();
    let mut add = |e: &crate::v2::ast::EnumDecl| {
        let names = e.variants.iter().map(|v| v.name.clone()).collect();
        merge_enum(&mut local, &e.name, Some(names));
    };
    for item in &file.items {
        let members = match item {
            V2Item::Enum(e) => {
                add(e);
                continue;
            }
            V2Item::Object(o) => &o.members,
            V2Item::Template(t) => &t.members,
            _ => continue,
        };
        for m in members {
            if let ObjectMember::Enum(e) = m {
                add(e);
            }
        }
    }
    if file.uses.is_empty() {
        return local;
    }
    let resolved = match (&input.path, &input.pack_dir) {
        (Some(path), Some(pack)) => crate::v2::resolve_v2_file(path, Some(pack)).ok(),
        _ => None,
    };
    let Some(resolved) = resolved else {
        return local;
    };
    for u in &file.uses {
        for item in &u.items {
            let name = item.alias.as_deref().unwrap_or(&item.name);
            if local.contains_key(name) {
                continue;
            }
            if let Some((_, e)) = resolved.enums.get(name) {
                let names = e.variants.iter().map(|v| v.name.clone()).collect();
                local.insert(name.to_owned(), Some(names));
            }
        }
    }
    local
}

/// An `.air` integer literal: decimal or `0x` hex, maybe negative, with `_` separators.
fn air_literal(v: &str) -> Option<i64> {
    let v = v.trim();
    let (neg, digits) = v.strip_prefix('-').map_or((false, v), |d| (true, d));
    if digits.is_empty() || digits.starts_with(['_', '-', '+']) {
        return None;
    }
    // As the `.air` lexer reads them: digits (and `_` separators) only, so no sign after
    // `0x` or a leading `+`, which Rust's parsers would accept.
    let body = |text: &str, radix: u32| {
        text.starts_with(|c: char| c.is_digit(radix))
            && text.chars().all(|c| c == '_' || c.is_digit(radix))
    };
    let n = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(hex) if body(hex, 16) => i64::from_str_radix(&hex.replace('_', ""), 16).ok()?,
        Some(_) => return None,
        None if body(digits, 10) => digits.replace('_', "").parse().ok()?,
        None => return None,
    };
    if neg { n.checked_neg() } else { Some(n) }
}

/// Whether the operands of raw `.air` primitive `word` (`args` is the text after its name)
/// have a v2 spelling: `name()` for argless ones, otherwise only `key: <literal>` pairs, and
/// none of the flag bits that make the VM read an operand from a local slot.
fn native_operands(word: &str, args: &str) -> bool {
    let Some(inner) = args.strip_prefix('(').and_then(|a| a.strip_suffix(')')) else {
        return false;
    };
    let inner = inner.trim();
    if matches!(word, "notify_idle" | "breakpoint") {
        return inner.is_empty();
    }
    let literal = air_literal;
    !inner.is_empty()
        && inner.split(',').all(|arg| {
            let Some((key, value)) = arg.split_once(':') else {
                return false;
            };
            let Some(&(_, _, kind)) = PRIM_OPERANDS
                .iter()
                .find(|(w, k, _)| *w == word && *k == key.trim())
            else {
                return false;
            };
            let value = value.trim();
            match (kind, literal(value)) {
                (Operand::Range(lo, hi), Some(n)) => (lo..=hi).contains(&n),
                // `special_effect` flag 0x20 reads the duration from a local.
                (Operand::Flags(lo, hi), Some(n)) => (lo..=hi).contains(&n) && n & 0x20 == 0,
                (Operand::Bool, None) => matches!(value, "true" | "false"),
                _ => false,
            }
        })
}

/// `.air` text with its whitespace removed, or `None` when whitespace separates two word
/// characters (`1 2`, `scope(1 2, 0)`: two tokens, which removing the space would read as
/// one) or follows a `-` (`- 2`, which the `.air` lexer does not read as `-2`).
fn compact_air(text: &str) -> Option<String> {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::new();
    let mut gap = false;
    for c in text.chars() {
        if c.is_whitespace() {
            gap = true;
            continue;
        }
        // `- 2` is not `-2`: the `.air` lexer reads a sign only next to its digits.
        if gap
            && out
                .chars()
                .last()
                .is_some_and(|p| p == '-' || (word(c) && word(p)))
        {
            return None;
        }
        gap = false;
        out.push(c);
    }
    Some(out)
}

/// What a v2 builtin accepts for one named operand.
#[derive(Clone, Copy)]
enum Operand {
    Range(i64, i64),
    /// A flags operand of `special_effect` (bit `0x20` reads a local).
    Flags(i64, i64),
    Bool,
}

/// `(.air primitive, operand key, what the v2 builtin accepts)`, mirroring the v2 lowering's
/// one-to-one builtins. A key not listed (such as `set_headline`'s `local`) is not judged.
const PRIM_OPERANDS: &[(&str, &str, Operand)] = &[
    ("run_tree_by_name", "table", Operand::Range(0, 65535)),
    ("run_tree_by_name", "index", Operand::Range(0, 255)),
    ("run_tree_by_name", "mode", Operand::Range(0, 255)),
    ("run_tree_by_name", "global", Operand::Bool),
    ("go_to_relative", "location", Operand::Range(-128, 127)),
    ("go_to_relative", "facing", Operand::Range(0, 255)),
    ("go_to_relative", "options", Operand::Range(0, 255)),
    ("manage_inventory", "op", Operand::Range(0, 255)),
    ("manage_inventory", "kind", Operand::Range(0, 255)),
    ("manage_inventory", "flags", Operand::Range(0, 65535)),
    ("manage_inventory", "guid", Operand::Range(0, 0xFFFF_FFFF)),
    // v2 names the target `self` / `stack_obj`, or `0` / `1`.
    ("refresh", "target", Operand::Range(0, 1)),
    ("refresh", "mode", Operand::Range(0, 65535)),
    ("action_string", "table", Operand::Range(0, 65535)),
    ("action_string", "index", Operand::Range(0, 255)),
    ("show_string", "table", Operand::Range(0, 65535)),
    ("show_string", "index", Operand::Range(0, 65535)),
    ("set_headline", "kind", Operand::Range(0, 255)),
    ("set_headline", "index", Operand::Range(0, 255)),
    ("set_headline", "duration", Operand::Range(-32768, 32767)),
    ("set_headline", "options", Operand::Range(0, 65535)),
    ("set_headline", "stack_obj", Operand::Bool),
    ("special_effect", "duration", Operand::Range(-32768, 32767)),
    ("special_effect", "kind", Operand::Range(0, 255)),
    ("special_effect", "size", Operand::Range(0, 255)),
    ("special_effect", "flags", Operand::Flags(0, 255)),
    ("special_effect", "str_index", Operand::Range(0, 255)),
];

/// `.air` text with its comments (`//`, `#`, nested `/* */`) replaced by a space; strings
/// are kept as they are.
/// `code` with its `.air` comments and string literals blanked to spaces.
fn air_code_only(code: &str) -> String {
    let mut outside = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for c in blank_air_comments(code).chars() {
        if quoted {
            quoted = escaped || c != '"';
            escaped = !escaped && c == '\\';
            outside.push(if c == '\n' { c } else { ' ' });
        } else {
            quoted = c == '"';
            outside.push(if quoted { ' ' } else { c });
        }
    }
    outside
}

/// The identifier-like words of `.air` text.
/// The names `.air` code may read a v2 constant by: every word except a field (after
/// `.`) and the block's own constant declarations. `const K = ..;` declares the block's
/// own `K`, which every bare `K` in the block then means; `const K.e = ..` declares the
/// block's `K.e` (so `K.e` there is not the v2 `K`, but a bare `K` is); `const[K]` and
/// `x.const` declare nothing.
fn air_name_uses(code: &str) -> Vec<&str> {
    let bytes = code.as_bytes();
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let prev = |at: usize| code[..at].trim_end().bytes().last();
    let mut words: Vec<(usize, &str)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if is_word(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_word(bytes[i]) {
                i += 1;
            }
            words.push((start, &code[start..i]));
        } else {
            i += 1;
        }
    }
    // The member after `K.`, if any.
    let member = |at: usize, name: &str| {
        let rest = code[at + name.len()..]
            .trim_start()
            .strip_prefix('.')?
            .trim_start();
        let end = rest.bytes().position(|c| !is_word(c)).unwrap_or(rest.len());
        (end > 0).then(|| &rest[..end])
    };
    let mut own = BTreeSet::new();
    let mut own_members = BTreeSet::new();
    let mut declaring = BTreeSet::new();
    for pair in words.windows(2) {
        let [(keyword_at, keyword), (name_at, name)] = pair else {
            continue;
        };
        // `x.const` is a field, and `const[K]` reads `K`.
        if *keyword != "const" || prev(*keyword_at) == Some(b'.') || prev(*name_at) == Some(b'[') {
            continue;
        }
        declaring.insert(*name_at);
        if let Some(m) = member(*name_at, name) {
            own_members.insert((*name, m));
            continue;
        }
        let rest = code[name_at + name.len()..].trim_start();
        if rest.starts_with('=') && !rest.starts_with("==") {
            own.insert(*name);
        }
    }
    words
        .into_iter()
        .filter(|(at, w)| {
            prev(*at) != Some(b'.')
                && !declaring.contains(at)
                && !own.contains(w)
                && !member(*at, w).is_some_and(|m| own_members.contains(&(*w, m)))
        })
        .map(|(_, w)| w)
        .collect()
}

/// The words of `.air` code (comments and strings already blanked) that follow a `.` or a
/// `::`: fields, and the entries of `Table.entry` / `Table::entry` BCON paths.
fn air_members(code: &str) -> Vec<&str> {
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !is_word(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_word(bytes[i]) {
            i += 1;
        }
        let before = code[..start].trim_end();
        let path = before.ends_with("::") || (before.ends_with('.') && !before.ends_with(".."));
        if path && !bytes[start].is_ascii_digit() {
            out.push(&code[start..i]);
        }
    }
    out
}

fn air_words(code: &str) -> impl Iterator<Item = &str> {
    code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
}

/// `code` with its `.air` comments (`# ..`, `// ..`, `/* .. */`, nested) blanked to
/// spaces, newlines kept: the same length, so offsets into it are offsets into `code`.
fn blank_air_comments(code: &str) -> String {
    let b = code.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'#' => {
                let end = code[i..].find('\n').map_or(b.len(), |e| i + e);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = code[i..].find('\n').map_or(b.len(), |e| i + e);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                // `.air` block comments do not nest: the first `*/` ends one.
                let start = i;
                let end = code[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
                blank(&mut out, start, end);
                i = end;
            }
            _ => i += 1,
        }
    }
    // Only ASCII bytes were replaced, by ASCII spaces, so whole characters went.
    String::from_utf8(out).unwrap_or_else(|_| code.to_owned())
}

fn strip_air_comments(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push(c);
                let mut escaped = false;
                for s in chars.by_ref() {
                    out.push(s);
                    if escaped {
                        escaped = false;
                    } else if s == '\\' {
                        escaped = true;
                    } else if s == '"' {
                        break;
                    }
                }
            }
            '#' => {
                chars.by_ref().find(|&s| s == '\n');
                out.push('\n');
            }
            '/' if chars.peek() == Some(&'/') => {
                chars.by_ref().find(|&s| s == '\n');
                out.push('\n');
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut depth = 1usize;
                while depth > 0 {
                    match chars.next() {
                        Some('/') if chars.peek() == Some(&'*') => {
                            chars.next();
                            depth += 1;
                        }
                        Some('*') if chars.peek() == Some(&'/') => {
                            chars.next();
                            depth -= 1;
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Whether `.air` text indexes `word` by number (`word[`, spaces allowed before the `[`).
fn raw_indexes(code: &str, word: &str) -> bool {
    let code = &strip_air_comments(code);
    code.match_indices(word).any(|(at, _)| {
        let before = code[..at].chars().next_back();
        !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
            && code[at + word.len()..].trim_start().starts_with('[')
    })
}

/// Records `name`'s variants, or `None` once two declarations of it disagree.
fn merge_enum(
    map: &mut BTreeMap<String, Option<Vec<String>>>,
    name: &str,
    variants: Option<Vec<String>>,
) {
    map.entry(name.to_owned())
        .and_modify(|e| {
            if *e != variants {
                *e = None;
            }
        })
        .or_insert(variants);
}

/// What kind of body a statement list is.
#[derive(Clone, Copy)]
struct BodyCx {
    /// A Sim acts on this object: `self` follows `stack_obj`.
    sim: bool,
}

impl FileLinter<'_> {
    /// Whether a bare call to `name` in this file may resolve to a user function or tree,
    /// which takes it over a builtin of the same name.
    fn user_fn(&self, name: &str) -> bool {
        self.callables
            .as_ref()
            .is_none_or(|names| names.contains(name))
    }

    /// Whether a bare call to `name` in this file surely resolves to a user function or
    /// tree. A rule that reports more when a call is a user's (a read of an unassigned
    /// local) asks this, not [`Self::user_fn`].
    fn sure_user_fn(&self, name: &str) -> bool {
        self.sure_callables.contains(name)
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        rule_name: &'static str,
        at: Span,
        message: String,
        help: Option<String>,
        suggestion: Option<String>,
        note: Option<String>,
    ) {
        let severity = rule(rule_name).map_or(Severity::Warning, |r| r.severity);
        self.diags.push(LintDiagnostic {
            rule: rule_name,
            severity,
            file: self.file.to_owned(),
            line: at.line,
            column: at.column,
            message,
            help,
            suggestion,
            note,
        });
    }

    /// Source text from `at` to the end of its line.
    fn text_at(&self, at: Span) -> &str {
        self.lines
            .get(at.line.wrapping_sub(1))
            .and_then(|l| l.get(at.column.saturating_sub(1)..))
            .unwrap_or("")
    }

    fn lint_file(&mut self, file: &V2File) {
        for item in &file.items {
            self.air_consts = air_consts(file, &[]);
            self.me_bcon = self.symbols.bcon_names.contains("me");
            match item {
                V2Item::Fn(f) => self.lint_fn(f),
                V2Item::Const(c) if !c.is_pub => self.unused_const(c),
                V2Item::Object(o) => {
                    self.object = Some(o.name.clone());
                    self.object_template = o.from_template.as_ref().map(|(t, _)| t.clone());
                    self.body_scope = body_scope(file, o);
                    // A template from another module may declare a BCON table `me`.
                    self.me_bcon |= o
                        .from_template
                        .as_ref()
                        .is_some_and(|(t, _)| !self.symbols.template_names.contains(t));
                    self.strings = o
                        .members
                        .iter()
                        .filter_map(|m| match m {
                            ObjectMember::Strings(s) => Some(&s.entries),
                            _ => None,
                        })
                        .flatten()
                        .map(|(_, name, text)| (name.clone(), text.clone()))
                        .collect();
                    self.dialog_strings = o
                        .members
                        .iter()
                        .find_map(|m| match m {
                            ObjectMember::Strings(s) if s.id == 301 => {
                                Some(s.entries.iter().map(|(_, _, t)| t.clone()).collect())
                            }
                            _ => None,
                        })
                        .unwrap_or_default();
                    self.tables = o
                        .members
                        .iter()
                        .filter_map(|m| match m {
                            ObjectMember::Strings(s) => Some((
                                i64::from(s.id),
                                s.entries.iter().map(|(_, _, t)| t.clone()).collect(),
                            )),
                            _ => None,
                        })
                        .collect();
                    self.air_consts = air_consts(file, &o.members);
                    self.lint_members(&o.members, false);
                    self.object = None;
                    self.object_template = None;
                    self.body_scope = None;
                    self.strings.clear();
                    self.dialog_strings.clear();
                    self.tables.clear();
                }
                V2Item::Template(t) => {
                    self.air_consts = air_consts(file, &t.members);
                    self.lint_members(&t.members, true);
                }
                _ => {}
            }
        }
    }

    fn lint_fn(&mut self, f: &FnDecl) {
        let sim = f.params.iter().any(|p| p.ty == V2Type::Sim);
        // An inline helper runs in its caller's context: only its own Sim parameters are
        // known to be the Sim.
        self.enter_context(
            f.params
                .iter()
                .filter(|p| p.ty == V2Type::Sim)
                .map(|p| p.name.as_str()),
            false,
            false,
        );
        self.bind(
            f.params
                .iter()
                .filter(|p| p.ty != V2Type::Sim)
                .map(|p| p.name.as_str()),
        );
        let unbuilt = std::mem::replace(&mut self.unbuilt, true);
        self.lint_body(&f.body, BodyCx { sim });
        self.unbuilt = unbuilt;
        self.leave_context();
    }

    /// Sets the Sim names (`sims`, plus `me` when `me_is_sim`) and whether `me` is the
    /// object (`object_me`) for the body about to be linted.
    fn enter_context<'n>(
        &mut self,
        sims: impl Iterator<Item = &'n str>,
        me_is_sim: bool,
        object_me: bool,
    ) {
        self.sims = sims.map(str::to_owned).collect();
        self.sim_bindings = self.sims.iter().cloned().collect();
        if me_is_sim && !self.me_bcon {
            self.sims.push("me".to_owned());
        }
        self.object_me = object_me && !self.me_bcon;
    }

    /// Records names the body being linted binds itself.
    fn bind<'n>(&mut self, names: impl Iterator<Item = &'n str>) {
        self.bound.extend(names.map(str::to_owned));
    }

    fn leave_context(&mut self) {
        self.sims.clear();
        self.sim_bindings.clear();
        self.object_me = false;
        self.bound.clear();
    }

    fn lint_members(&mut self, members: &[ObjectMember], template: bool) {
        let (unbuilt, in_template) = (self.unbuilt, self.in_template);
        self.unbuilt |= template;
        self.in_template |= template;
        self.lint_members_inner(members, template);
        self.unbuilt = unbuilt;
        self.in_template = in_template;
    }

    fn lint_members_inner(&mut self, members: &[ObjectMember], template: bool) {
        for m in members {
            match m {
                ObjectMember::Fn(f) => self.lint_fn(f),
                ObjectMember::Tree(t) => {
                    self.missing_sim_frame_args(t);
                    self.enter_tree(t.pinned_id, &t.label, &t.body);
                    let sim = t.sim_binding.is_some();
                    self.enter_context(
                        t.sim_binding.iter().map(String::as_str),
                        sim || t.inherit_context,
                        !sim && !t.inherit_context,
                    );
                    self.bind(
                        t.params
                            .iter()
                            .filter(|p| p.ty != V2Type::Sim)
                            .map(|p| p.name.as_str()),
                    );
                    self.lint_body(&t.body, BodyCx { sim });
                    self.leave_context();
                }
                ObjectMember::Init(b) | ObjectMember::Main(b) => {
                    let label = if matches!(m, ObjectMember::Init(_)) {
                        "Init"
                    } else {
                        "Main"
                    };
                    self.enter_tree(None, label, &b.body);
                    self.enter_context(std::iter::empty(), false, true);
                    self.lint_body(&b.body, BodyCx { sim: false });
                    self.leave_context();
                }
                ObjectMember::Load(b) | ObjectMember::Reset(b) | ObjectMember::Cleanup(b) => {
                    self.enter_tree(None, "", &b.body);
                    self.enter_context(std::iter::empty(), false, true);
                    self.lint_body(&b.body, BodyCx { sim: false });
                    self.leave_context();
                }
                ObjectMember::Functional(f) => {
                    self.lint_guard(f.guard.as_ref(), &f.params);
                    self.enter_context(std::iter::once(f.sim_binding.as_str()), true, false);
                    self.bind(
                        f.params
                            .iter()
                            .filter(|p| p.ty != V2Type::Sim)
                            .map(|p| p.name.as_str()),
                    );
                    self.lint_body(&f.body, BodyCx { sim: true });
                    self.leave_context();
                }
                ObjectMember::Interaction(i) => {
                    self.lint_guard(i.guard.as_ref(), &[]);
                    let label = crate::v2::ast::object_type_ident(&i.label);
                    self.enter_tree(i.pinned_id, &label, &i.run_body);
                    self.enter_context(std::iter::once(i.sim_binding.as_str()), true, false);
                    self.lint_body(&i.run_body, BodyCx { sim: true });
                    self.leave_context();
                }
                ObjectMember::Const(c) if !template && !c.is_pub => self.unused_const(c),
                ObjectMember::State(s) if !template => {
                    let used = self.symbols.field_names.contains(&s.name)
                        || s.slot.is_some_and(|(_, n)| {
                            self.symbols.attr_indices.contains(&i64::from(n))
                        });
                    if !used && !s.name.starts_with('_') {
                        self.emit(
                            UNUSED_DECL,
                            s.span,
                            format!("state `{}` is never used", s.name),
                            Some(
                                "remove it, or allow the lint if another tool reads the attribute"
                                    .to_owned(),
                            ),
                            None,
                            None,
                        );
                    }
                }
                ObjectMember::Bcon(b) if !template && !self.symbols.bcon_by_number => {
                    // `Table::entry` reads the table too (see `bcon_path_reads`): in this
                    // object's body, or in its template's unless this object declares a type
                    // of the table's name, which wins the path in the merged scope.
                    let tables = bcon_names(&b.label);
                    let own_types = member_type_names(members);
                    let mut bodies = vec![("module".to_owned(), false)];
                    if let Some(o) = &self.object {
                        bodies.push((format!("object:{}:{o}", self.file), false));
                    }
                    if let Some(t) = &self.object_template {
                        bodies.push((format!("template:{t}"), true));
                    }
                    let used: Vec<bool> = b
                        .entries
                        .iter()
                        .map(|(_, name, _)| {
                            self.symbols.field_names.contains(name)
                                || tables.iter().any(|t| {
                                    bodies.iter().any(|(body, inherited)| {
                                        !(*inherited && own_types.contains(t))
                                            && self.symbols.bcon_paths.contains(&(
                                                body.clone(),
                                                t.clone(),
                                                name.clone(),
                                            ))
                                    })
                                })
                        })
                        .collect();
                    if used.iter().any(|u| *u) {
                        for ((span, name, _), used) in b.entries.iter().zip(used) {
                            if !used && !name.starts_with('_') {
                                self.emit(
                                    UNUSED_DECL,
                                    *span,
                                    format!("BCON entry `{name}` of table {} is never read", b.id),
                                    Some("remove it (keep the slot with a `_` name), or allow the lint if a global tree reads it by index".to_owned()),
                                    None,
                                    None,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn unused_const(&mut self, c: &crate::v2::ast::ConstDecl) {
        if self.const_used(&c.name) {
            return;
        }
        self.emit(
            UNUSED_DECL,
            c.span,
            format!("constant `{}` is never used", c.name),
            Some("remove it".to_owned()),
            None,
            None,
        );
    }

    /// Whether the constant `name` may be used: a `_` name, a name written anywhere in the
    /// file set other than as a `const` name (a `guid:`, an index, a BCON value, an
    /// attribute), or one a `raw` block names (lowering declares it for the block as an
    /// `.air` constant). A `raw` block that writes the value as a number does not use it.
    fn const_used(&self, name: &str) -> bool {
        name.starts_with('_')
            || self.symbols.const_uses.get(name).is_some_and(|&n| n > 0)
            || self.symbols.raw_words.contains(name)
    }

    /// `untyped_engine_field`: `x` is an engine cell by number that has a typed name.
    fn untyped_engine_field(&mut self, x: &V2Expr, parent: Option<&V2Expr>) {
        let V2Expr::Index { base, index, .. } = x else {
            return;
        };
        let V2Expr::Int(n_at, n) = index.as_ref() else {
            return;
        };
        // A literal, not a constant (a specialized `const` may stand for several cells).
        if !self
            .text_at(*n_at)
            .starts_with(|c: char| c.is_ascii_digit())
        {
            return;
        }
        let n = *n;
        let (at, typed, field) = match base.as_ref() {
            // `world.hour` parses to `global[0]` too: only the spelled-out form counts.
            V2Expr::Ident(at, name)
                if name == "global" && self.text_at(*at).starts_with("global") =>
            {
                let Some((name, _)) = crate::v2::parser::WORLD_GLOBALS
                    .iter()
                    .find(|(_, i)| i64::from(*i) == n)
                else {
                    return;
                };
                let field = TypedField {
                    cell: "",
                    index: n,
                    name,
                    writable: true,
                    object_id: false,
                };
                (*at, format!("world.{name}"), field)
            }
            V2Expr::Field {
                base: root, field, ..
            } => {
                let V2Expr::Ident(at, root) = root.as_ref() else {
                    return;
                };
                let kind = if self.bound.contains(root) || root == "stack_obj" {
                    Root::Handle
                } else if root == "self" {
                    if self.sims.iter().any(|s| s == "self") {
                        return;
                    }
                    Root::Object
                } else if self.sims.iter().any(|s| s == root) {
                    Root::Sim
                } else {
                    return;
                };
                let table = match kind {
                    Root::Object => SELF_FIELDS,
                    Root::Sim => SIM_FIELDS,
                    Root::Handle => &[],
                };
                let Some(&typed) = table
                    .iter()
                    .find(|f| f.cell == field.as_str() && f.index == n)
                else {
                    self.trial_engine_field(x, parent, *at, root, kind, field, n);
                    return;
                };
                (*at, format!("{root}.{}", typed.name), typed)
            }
            _ => return,
        };
        if self.written.contains(&addr(x)) && !field.writable {
            return;
        }
        // An `ObjectId` field stands in for the `i16` cell only where the two types behave
        // alike: compared for (in)equality, or assigned as a whole.
        if field.object_id {
            let compared = matches!(
                parent,
                Some(V2Expr::Binary {
                    op: V2BinOp::Eq | V2BinOp::Ne,
                    ..
                })
            );
            if !compared && self.assigned != Some(addr(x)) {
                return;
            }
        }
        let raw = self.text_at(at);
        let end = raw.find(']').map_or(raw.len(), |i| i + 1);
        let raw = raw[..end].to_owned();
        self.emit(
            UNTYPED_ENGINE_FIELD,
            at,
            format!("`{raw}` is `{typed}`"),
            Some(format!("write `{typed}`; it names the same cell")),
            Some(typed),
            None,
        );
    }

    /// `object_tree_me_field`: `me.<field>` where `me` is the object running its own tree.
    fn object_tree_me_field(&mut self, x: &V2Expr, parent: Option<&V2Expr>) {
        if !self.object_me {
            return;
        }
        let V2Expr::Field { base, field, .. } = x else {
            return;
        };
        let V2Expr::Ident(at, name) = base.as_ref() else {
            return;
        };
        if name != "me" || self.bound.contains("me") {
            return;
        }
        // `me.attr[N]`, `me.data[N]` and the like index the object's own cells.
        if matches!(
            field.as_str(),
            "attr" | "data" | "person" | "motive" | "slot" | "sprite_flags"
        ) && matches!(parent, Some(V2Expr::Index { base, .. }) if addr(base.as_ref()) == addr(x))
        {
            return;
        }
        let help = if SIM_FIELDS.iter().any(|f| f.name == field.as_str())
            || matches!(field.as_str(), "motive" | "skill")
        {
            format!(
                "`me` is the object in its own tree, not a Sim: give the tree a Sim parameter (`fn t(sim: Sim)`) and write `sim.{field}`, or mark it `context = inherit` if a Sim calls it"
            )
        } else {
            format!(
                "`me` is the object in its own tree and has no fields: write `self.{field}`, or index a cell (`me.data[N]`)"
            )
        };
        self.emit(
            OBJECT_TREE_ME_FIELD,
            *at,
            format!("`me.{field}` in an object's own tree"),
            Some(help),
            None,
            None,
        );
    }

    /// `raw_syntax` for the body's `raw` blocks (in order), which share one `.air` tree.
    ///
    /// A block is parsed the way lowering validates it, after the constants lowering would
    /// declare for it. The rule must not report code `check` accepts, so a block that fails
    /// is parsed again before anything is reported:
    /// - with no tree-wide `.air` locals (one may sit in discarded code), and every other
    ///   name it uses declared as a constant (a `use` may bring it in from another module),
    ///   once as a number and once as a `bcon[..]` operand;
    /// - when other raw blocks share the tree, with all of them, in order: one block may
    ///   finish another's statement.
    ///
    /// Only an error that stays in the block in every reading is reported, and only for a
    /// block whose brackets balance.
    fn raw_syntax_body(&mut self) {
        let blocks = std::mem::take(&mut self.raw_blocks);
        let mut lenient: Vec<String> = Vec::with_capacity(blocks.len());
        let mut failed: Vec<(usize, crate::error::Error, usize)> = Vec::new();
        for (k, (_, code)) in blocks.iter().enumerate() {
            let (prelude, _) = self.raw_prelude(code, true);
            let strict = raw_parse(&prelude, code);
            let (known, names) = self.raw_prelude(code, false);
            let guesses = raw_guesses(code, &names);
            let as_int = format!("{known}{}", guesses.replace("= GUESS;", "= 1;"));
            let as_bcon = format!("{known}{}", guesses.replace("= GUESS;", "= bcon[0];"));
            let fails = strict.is_err()
                && raw_parse(&as_int, code).is_err()
                && raw_parse(&as_bcon, code).is_err();
            lenient.push(as_int);
            if let (true, Err((e, first))) = (fails, strict)
                && raw_balanced(code)
            {
                failed.push((k, e, first));
            }
        }
        if failed.is_empty() {
            return;
        }
        // All blocks in order, under the union of their lenient preludes (a name one block
        // declares itself is left to it).
        let together = (blocks.len() > 1).then(|| {
            let own: BTreeSet<String> = blocks
                .iter()
                .flat_map(|(_, code)| raw_own_consts(code))
                .collect();
            let mut seen = BTreeSet::new();
            let mut prelude = String::new();
            for line in lenient.iter().flat_map(|p| p.lines()) {
                let name = line
                    .strip_prefix("const ")
                    .and_then(|l| l.split(" = ").next())
                    .unwrap_or("");
                if !own.contains(name) && seen.insert(name.to_owned()) {
                    prelude.push_str(line);
                    prelude.push('\n');
                }
            }
            let mut code = String::new();
            let mut starts = Vec::with_capacity(blocks.len());
            for (_, c) in &blocks {
                starts.push(code.lines().count());
                code.push_str(c);
                code.push('\n');
            }
            (raw_parse(&prelude, &code), starts)
        });
        for (k, e, first) in failed {
            match &together {
                None => {}
                Some((Ok(()), _)) => continue,
                Some((Err((te, tfirst)), starts)) => {
                    // The shared reading fails too: report only an error inside this block.
                    let Some(line) = te.span.and_then(|s| s.line.checked_sub(*tfirst)) else {
                        continue;
                    };
                    let end = starts.get(k + 1).copied().unwrap_or(usize::MAX);
                    if line < starts[k] || line >= end || te.message.contains("duplicate") {
                        continue;
                    }
                }
            }
            let (span, code) = &blocks[k];
            let pos = e.span.and_then(|s| {
                let line = s.line.checked_sub(first)?;
                let starts: Vec<usize> = std::iter::once(0)
                    .chain(code.match_indices('\n').map(|(i, _)| i + 1))
                    .collect();
                let start = *starts.get(line)?;
                Some((start + s.column.saturating_sub(1)).min(code.len()))
            });
            let at = pos.map_or(*span, |pos| self.raw_span(*span, pos));
            self.emit(
                RAW_SYNTAX,
                at,
                format!("invalid `.air` in `raw {{ }}`: {}", e.message),
                Some("`check` rejects the file until the `.air` parses".to_owned()),
                None,
                None,
            );
        }
    }

    /// The `const` lines lowering puts before a `raw` block's code, and their names: each
    /// constant of the object or file (`NAME`, `TABLE.entry`) the code names and does not
    /// declare itself, less (with `locals`) the `.air` locals the tree's raw blocks declare.
    fn raw_prelude(&self, code: &str, locals: bool) -> (String, BTreeSet<String>) {
        let mut prelude = String::new();
        let mut declared = BTreeSet::new();
        let Ok(tokens) = crate::lexer::tokenize(code) else {
            return (prelude, declared);
        };
        use crate::lexer::TokenKind as K;
        let kind = |i: usize| tokens.get(i).map(|t| &t.kind);
        let own = raw_own_consts(code);
        for (i, t) in tokens.iter().enumerate() {
            let K::Ident(word) = &t.kind else {
                continue;
            };
            if (i > 0 && tokens[i - 1].kind == K::Dot)
                || crate::parser::is_reserved_ident(word)
                || (locals && self.raw_declared.contains(word))
            {
                continue;
            }
            let dotted = match (kind(i + 1), kind(i + 2)) {
                (Some(K::Dot), Some(K::Ident(entry))) => Some(format!("{word}.{entry}")),
                _ => None,
            };
            let found = match dotted {
                Some(d) if own.contains(&d) => continue,
                Some(d) if self.air_consts.contains_key(&d) => self.air_consts.get_key_value(&d),
                _ if own.contains(word) => continue,
                _ => self.air_consts.get_key_value(word),
            };
            let Some((name, value)) = found else {
                continue;
            };
            if declared.insert(name.clone()) {
                prelude.push_str(&format!("const {name} = {value};\n"));
            }
        }
        (prelude, declared)
    }

    /// `shadowed_const`, and `unused_decl` for block `const` bindings (which parsing
    /// specializes away, so they are found in the tokens).
    fn const_scopes(&mut self, file: &V2File, tokens: &[crate::v2::lexer::Token]) {
        let member_consts = |members: &[ObjectMember]| -> Vec<(String, Span)> {
            members
                .iter()
                .filter_map(|m| match m {
                    ObjectMember::Const(c) => Some((c.name.clone(), c.span)),
                    _ => None,
                })
                .collect()
        };
        let mut top: BTreeMap<String, Span> = BTreeMap::new();
        // Each object, template and helper by where it starts, with its member constants.
        let mut scopes: Vec<(Span, Vec<(String, Span)>)> = Vec::new();
        for item in &file.items {
            match item {
                V2Item::Const(c) => {
                    top.insert(c.name.clone(), c.span);
                }
                V2Item::Object(o) => scopes.push((o.span, member_consts(&o.members))),
                V2Item::Template(t) => scopes.push((t.span, member_consts(&t.members))),
                V2Item::Fn(f) => scopes.push((f.span, Vec::new())),
                _ => {}
            }
        }
        scopes.sort_by_key(|(at, _)| (at.line, at.column));
        let declared: BTreeSet<(usize, usize)> = top
            .values()
            .chain(scopes.iter().flat_map(|(_, m)| m.iter().map(|(_, at)| at)))
            .map(|at| (at.line, at.column))
            .collect();
        for (_, members) in &scopes {
            for (name, at) in members {
                if let Some(outer) = top.get(name) {
                    self.shadowed(*at, name, "object constant", *outer);
                }
            }
        }
        for b in const_bindings(tokens) {
            if declared.contains(&(b.at.line, b.at.column)) {
                continue;
            }
            let scope = scopes
                .iter()
                .rev()
                .find(|(at, _)| (at.line, at.column) <= (b.at.line, b.at.column));
            for (i, name) in &b.names {
                if name == "_" {
                    continue;
                }
                let at = tokens[*i].span;
                let outer = scope
                    .and_then(|(_, m)| m.iter().find(|(n, _)| n == name).map(|(_, at)| *at))
                    .or_else(|| top.get(name).copied());
                if let Some(outer) = outer {
                    let what = if b.param {
                        "`const` parameter"
                    } else {
                        "block constant"
                    };
                    self.shadowed(at, name, what, outer);
                }
                if !b.param && !b.diverges && !self.const_used(name) {
                    self.emit(
                        UNUSED_DECL,
                        at,
                        format!("block constant `{name}` is never used"),
                        Some(if b.names.len() > 1 {
                            "write `_` in its place".to_owned()
                        } else if b.calls {
                            "remove it, keeping as statements any calls in its value that have effects".to_owned()
                        } else {
                            "remove it".to_owned()
                        }),
                        None,
                        None,
                    );
                }
            }
        }
    }

    fn shadowed(&mut self, at: Span, name: &str, what: &str, outer: Span) {
        self.emit(
            SHADOWED_CONST,
            at,
            format!(
                "{what} `{name}` shadows the constant `{name}` declared at line {}",
                outer.line
            ),
            Some(format!(
                "rename one of them: here `{name}` means the inner binding, not the outer constant"
            )),
            None,
            None,
        );
    }

    /// `duplicate_std_global`: an `extern mod` function `std::globals` already declares.
    fn duplicate_std_globals(&mut self, file: &V2File) {
        let mut mods = Vec::new();
        for item in &file.items {
            match item {
                V2Item::ExternMod(m) => mods.push(m),
                V2Item::Object(o) => mods.extend(o.members.iter().filter_map(|m| match m {
                    ObjectMember::ExternMod(m) => Some(m),
                    _ => None,
                })),
                V2Item::Template(t) => mods.extend(t.members.iter().filter_map(|m| match m {
                    ObjectMember::ExternMod(m) => Some(m),
                    _ => None,
                })),
                _ => {}
            }
        }
        for m in mods {
            for f in &m.fns {
                let Some(std) = std_globals().iter().find(|g| same_abi(g, f)) else {
                    continue;
                };
                self.emit(
                    DUPLICATE_STD_GLOBAL,
                    f.span,
                    format!(
                        "`{}::{}` is global {} as `std::globals` declares it (`globals::{}`)",
                        m.name, f.name, f.id, std.name
                    ),
                    Some(format!(
                        "drop it and call `globals::{}` after `use std::globals::*;` (if `globals` names something else here, `use std::globals::{{globals as std_globals}};` and `std_globals::{}`){}. A local `extern mod globals` hides all of `std::globals`: keep other globals in a module of another name",
                        std.name,
                        std.name,
                        if std.params.iter().zip(&f.params).any(|(a, b)| a.name != b.name) {
                            format!(
                                " (its parameters are named {}: rename named arguments)",
                                std.params
                                    .iter()
                                    .map(|p| format!("`{}`", p.name))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        } else {
                            String::new()
                        }
                    )),
                    None,
                    None,
                );
            }
        }
    }

    /// Lints a guard; `params` are those of the functional it guards, which it sees too.
    fn lint_guard(&mut self, guard: Option<&GuardDecl>, params: &[crate::v2::ast::FnParam]) {
        let Some(g) = guard else {
            return;
        };
        self.enter_context(
            std::iter::once(g.sim_binding.as_str()).chain(
                params
                    .iter()
                    .filter(|p| p.ty == V2Type::Sim)
                    .map(|p| p.name.as_str()),
            ),
            true,
            false,
        );
        self.bind(g.autonomous_binding.iter().map(String::as_str));
        self.bind(
            params
                .iter()
                .filter(|p| p.ty != V2Type::Sim)
                .map(|p| p.name.as_str()),
        );
        match &g.kind {
            GuardBody::Expr(e) => self.lint_expr(e),
            GuardBody::Block(b) => self.lint_body(b, BodyCx { sim: true }),
        }
        self.leave_context();
    }

    fn lint_body(&mut self, body: &[V2Stmt], cx: BodyCx) {
        self.body += 1;
        self.sim_body = cx.sim;
        self.body_has_defer = false;
        self.raw_declared.clear();
        self.raw_edges = false;
        walk_stmts(body, &mut |s| {
            self.body_has_defer |= matches!(s, V2Stmt::Defer { .. });
            if let V2Stmt::RawAir { code, .. } = s {
                self.raw_declared.extend(raw_locals(code));
                self.raw_edges |= air_code_only(code).contains("->");
            }
        });
        self.raw_blocks.clear();
        self.lint_list(body, false, false);
        self.raw_syntax_body();
        if cx.sim {
            self.self_handles.clear();
            self.clobber_walk(body, None);
        }
        self.dialog_local_slots();
        self.tree = None;
    }

    /// Sets up [`TreeCx`] for the tree `id` / `label` (its `.air` label) of the current
    /// object, whose body is `body`.
    fn enter_tree(&mut self, id: Option<u16>, label: &str, body: &[V2Stmt]) {
        let frame = self.object.as_ref().and_then(|o| {
            let frames = self.frames.as_ref()?;
            let list = frames
                .get(o)
                .or_else(|| frames.values().next().filter(|_| frames.len() == 1))?;
            id.and_then(|id| list.iter().find(|f| f.id == id))
                .or_else(|| {
                    let mut named = list
                        .iter()
                        .filter(|f| !label.is_empty() && f.label == label);
                    named.next().filter(|_| named.next().is_none())
                })
                .cloned()
        });
        let mut tokens: BTreeMap<usize, Vec<(Span, String, String)>> = BTreeMap::new();
        let mut lets = BTreeSet::new();
        let mut unknown_strings = false;
        let strings = &self.strings;
        let dialog_strings = &self.dialog_strings;
        let tables = &self.tables;
        let mut note = |span: Span, text: &str, from: String| {
            for (token, slot) in local_tokens(text) {
                tokens
                    .entry(slot)
                    .or_default()
                    .push((span, token, from.clone()));
            }
        };
        walk_stmts(body, &mut |s| {
            match s {
                V2Stmt::Let { name, .. }
                | V2Stmt::LetSome { name, .. }
                | V2Stmt::LetUninit { name, .. } => {
                    lets.insert(name.clone());
                }
                // A `raw` dialog, Set Action String or bare opcode shows strings this
                // linter does not follow.
                V2Stmt::RawAir { code, .. } => {
                    unknown_strings |= air_words(&air_code_only(code))
                        .any(|w| matches!(w, "dialog" | "action_string" | "op"));
                }
                _ => {}
            }
            // Strings reach a primitive as call arguments: a literal, a `strings`
            // entry's name, or (for a dialog) its 1-based number in `STR#` 301.
            let mut args: Vec<(&str, &V2Expr)> = Vec::new();
            if let V2Stmt::Animate { args: a, .. } = s {
                args.extend(a.iter().map(|a| ("animate", &a.value)));
            }
            for e in stmt_exprs(s) {
                walk_expr(e, &mut |x| match x {
                    V2Expr::Call {
                        callee, args: a, ..
                    } => {
                        let name = match callee.as_ref() {
                            V2Expr::Ident(_, n) => n.as_str(),
                            _ => "",
                        };
                        // Show String's text is presentation: the VM formats no token.
                        if name == "show_string" {
                            return;
                        }
                        // Set Action String formats string `index` (from 1) of `table`.
                        if matches!(name, "action_label" | "action_string") {
                            let int = |key: &str| {
                                a.iter().find_map(|a| match (a.name.as_deref(), &a.value) {
                                    (Some(k), V2Expr::Int(_, n)) if k == key => Some(*n),
                                    _ => None,
                                })
                            };
                            let text = int("table").zip(int("index")).and_then(|(t, i)| {
                                let text = tables.get(&t)?.get(usize::try_from(i - 1).ok()?)?;
                                Some((t, i, text))
                            });
                            match text {
                                Some((t, i, text)) => {
                                    note(x.span(), text, format!("string {i} of table {t}"));
                                }
                                // A table this file does not hold, or not by number.
                                None => unknown_strings = true,
                            }
                            return;
                        }
                        if name != "dialog" {
                            args.extend(a.iter().map(|a| (name, &a.value)));
                            return;
                        }
                        // A dialog's strings: the text arguments and the `buttons` tuple
                        // (`kind:`, `flags:` and the like are numbers, not strings).
                        for a in a {
                            match (a.name.as_deref(), &a.value) {
                                (Some("buttons"), V2Expr::Tuple(_, items)) => {
                                    args.extend(items.iter().map(|v| ("dialog", v)));
                                }
                                (
                                    Some(
                                        "title" | "message" | "text" | "mode4_str" | "button1"
                                        | "button2" | "button3",
                                    ),
                                    v,
                                ) => args.push(("dialog", v)),
                                _ => {}
                            }
                        }
                    }
                    V2Expr::MethodCall { args: a, .. } => {
                        args.extend(a.iter().map(|a| ("", &a.value)));
                    }
                    _ => {}
                });
            }
            for (callee, value) in args {
                match value {
                    V2Expr::Str(span, text) => note(*span, text, "this string".to_owned()),
                    V2Expr::Ident(span, name) if strings.contains_key(name) => {
                        note(*span, &strings[name], format!("string `{name}`"));
                    }
                    V2Expr::Path(span, segments) => {
                        if let Some(name) = segments.last()
                            && let Some(text) = strings.get(name)
                        {
                            note(*span, text, format!("string `{name}`"));
                        }
                    }
                    V2Expr::Int(span, n) if callee == "dialog" && *n > 0 => {
                        match usize::try_from(*n - 1)
                            .ok()
                            .and_then(|i| dialog_strings.get(i))
                        {
                            Some(text) => note(*span, text, format!("dialog string {n}")),
                            // The object's own `STR#` 301 (a spliced resource), not this
                            // file's: its tokens are unknown.
                            None => unknown_strings = true,
                        }
                    }
                    _ => {}
                }
            }
        });
        self.tree = Some(TreeCx {
            frame,
            tokens,
            lets,
            unknown_strings,
        });
    }

    /// Whether a `$Local:N` token of the current tree's strings reads `name`'s slot (or,
    /// with no frame to check, whether the tree's strings have any such token). Always,
    /// when the tree shows strings by number that this file does not hold.
    fn read_by_string_token(&self, name: &str) -> bool {
        let Some(t) = &self.tree else {
            return false;
        };
        if t.unknown_strings {
            return true;
        }
        if t.tokens.is_empty() {
            return false;
        }
        let Some(f) = &t.frame else {
            return true;
        };
        t.tokens.keys().any(|&n| {
            f.slots
                .get(n)
                .is_some_and(|s| let_source_name(s) == Some(name))
        })
    }

    /// For a local nothing reads in a tree whose strings have `$Local:N` tokens: which
    /// slots they read, and which slot the local is on.
    fn string_token_note(&self, name: &str) -> Option<String> {
        let t = self.tree.as_ref()?;
        let frame = t.frame.as_ref()?;
        if t.tokens.is_empty() {
            return None;
        }
        let read: Vec<String> = t
            .tokens
            .iter()
            .filter_map(|(slot, uses)| {
                uses.first()
                    .map(|(_, token, _)| format!("`{token}` (slot {slot})"))
            })
            .collect();
        let on = frame
            .slots
            .iter()
            .position(|s| let_source_name(s) == Some(name))
            .map_or_else(String::new, |n| format!("; `{name}` is on slot {n}"));
        Some(format!("this tree's strings read {}{on}", read.join(", ")))
    }

    /// `dialog_local_slot_mismatch`: every `$Local:N` token of the current tree's strings
    /// must read a slot that holds one of the tree's own `let`s (or a raw `local[N]`).
    fn dialog_local_slots(&mut self) {
        let Some(t) = self.tree.take() else {
            return;
        };
        let Some(frame) = &t.frame else {
            return;
        };
        let layout = || {
            let shown: Vec<String> = frame
                .slots
                .iter()
                .enumerate()
                .map(|(i, s)| match let_source_name(s) {
                    Some(name) => format!("{i} `{name}`"),
                    None if s.starts_with("local[") => format!("{i} raw"),
                    None => format!("{i} `{s}` (compiler)"),
                })
                .collect();
            if shown.is_empty() {
                "this tree has no locals".to_owned()
            } else {
                format!("this tree's local slots: {}", shown.join(", "))
            }
        };
        for (slot, uses) in &t.tokens {
            let Some((at, token, from)) = uses.first() else {
                continue;
            };
            let what = match frame.slots.get(*slot) {
                None => format!("no local: the frame has {} slot(s)", frame.slots.len()),
                Some(s) if s.starts_with("local[") => continue,
                Some(s) => match let_source_name(s) {
                    Some(name) if t.lets.contains(name) => continue,
                    Some(name) => format!(
                        "`{name}`, which is not a `let` of this tree (an inlined helper's local)"
                    ),
                    None => format!("the compiler temporary `{s}`"),
                },
            };
            self.emit(
                DIALOG_LOCAL_SLOT_MISMATCH,
                *at,
                format!("`{token}` in {from} reads local slot {slot}, which holds {what}"),
                Some(format!("declare the `let` the string shows so that it lands on slot {slot}: locals take slots in the order the lowered tree declares them, after any raw `local[N]`, compiler temporaries included (`sims-antics expand` shows the order)")),
                None,
                Some(format!("{}. The verifier compares the dialog primitive, not the slots its string tokens read, so a shifted slot proves and shows the wrong value", layout())),
            );
        }
    }

    /// `missing_sim_frame_args`: a `(sim: Sim)` tree with no parameters whose `raw` blocks
    /// read `param[n]` past its `#[frame(args = )]`.
    fn missing_sim_frame_args(&mut self, t: &crate::v2::ast::TreeMemberDecl) {
        if t.sim_binding.is_none() || !t.params.is_empty() {
            return;
        }
        // A pinned `#[frame(args = )]` declares the frame (a read past it does not compile).
        if t.frame.as_ref().is_some_and(|f| f.args.is_some()) {
            return;
        }
        let args = 0usize;
        let mut hit: Option<(Span, i64)> = None;
        walk_stmts(&t.body, &mut |s| {
            if let V2Stmt::RawAir { span, code } = s {
                for (word, n) in raw_index_words(&strip_air_comments(code)) {
                    // A frame has at most 255 argument words; a larger index is the
                    // compiler's to reject.
                    if word == "param"
                        && usize::try_from(n).is_ok_and(|n| n >= args && n < 256)
                        && hit.is_none_or(|(_, m)| n > m)
                    {
                        hit = Some((hit.map_or(*span, |(at, _)| at), n));
                    }
                }
            }
        });
        let Some((at, n)) = hit else {
            return;
        };
        self.emit(
            MISSING_SIM_FRAME_ARGS,
            at,
            format!(
                "this Sim tree's `raw` block reads `param[{n}]`, but the tree declares no argument words"
            ),
            Some(format!(
                "declare its arguments: pin `#[frame(args = {})]` (or the original's count), or declare the parameters; `(sim: Sim)` is the frame's Sim, not an argument word",
                n + 1
            )),
            None,
            Some("the compiled header takes its argument count from the raw read alone, and only a caller that passes `temps` fills those words; a lifted tree keeps the original's `#[frame(args = N)]` so the count matches".to_owned()),
        );
    }

    /// Lints one statement list. `elif` marks the `else if` tail of a chain checked above. `follows`: whether code runs after the list ends (it
    /// is nested in a list that goes on, or is a loop body).
    fn lint_list(&mut self, list: &[V2Stmt], elif: bool, follows: bool) {
        // Names a statement binds are in scope from the next statement to the list's end.
        let outer = self.bound.clone();
        self.unreachable_code(list);
        for (i, s) in list.iter().enumerate() {
            if !(elif && i == 0) {
                self.collapsible_chain(s);
            }
            self.lint_stmt(s);
            self.redundant_bool_return(list, i);
            self.negated_comparison(list, i);
            let after = i + 1 < list.len() || follows;
            if let V2Stmt::RawAir { span, code } = s
                && after
            {
                self.raw_bare_call(*span, code);
            }
            self.unused_let(list, i);
            match s {
                V2Stmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    self.lint_list(then_body, false, after);
                    let is_elif = matches!(else_body.as_slice(), [V2Stmt::If { .. }]);
                    self.lint_list(else_body, is_elif, after);
                }
                V2Stmt::ForEach {
                    binding,
                    body,
                    else_body,
                    ..
                } => {
                    let before = self.bound.clone();
                    self.bound.insert(binding.clone());
                    self.lint_list(body, false, true);
                    self.bound = before;
                    if let Some(else_body) = else_body {
                        self.lint_list(else_body, false, after);
                    }
                }
                other => {
                    // A loop body goes back to its test.
                    let looped = matches!(
                        other,
                        V2Stmt::While { .. } | V2Stmt::Repeat { .. } | V2Stmt::ForEach { .. }
                    );
                    let block = matches!(
                        other,
                        V2Stmt::WithLock { .. } | V2Stmt::Defer { .. } | V2Stmt::Animate { .. }
                    );
                    self.in_block += usize::from(block);
                    // An animation's event binding names the event in its handler.
                    let before = self.bound.clone();
                    if let V2Stmt::Animate {
                        event: Some(event), ..
                    } = other
                    {
                        self.bound.insert(event.clone());
                    }
                    for b in stmt_blocks(other) {
                        self.lint_list(b, false, after || looped);
                    }
                    self.bound = before;
                    self.in_block -= usize::from(block);
                }
            }
            if let V2Stmt::Let { name, .. }
            | V2Stmt::LetSome { name, .. }
            | V2Stmt::LetUninit { name, .. } = s
            {
                self.bound.insert(name.clone());
            }
        }
        self.bound = outer;
    }

    fn lint_stmt(&mut self, s: &V2Stmt) {
        self.stmt = Some(stmt_span(s));
        self.stmt_flows = matches!(
            s,
            V2Stmt::Let { .. }
                | V2Stmt::LetSome { .. }
                | V2Stmt::ForEach { .. }
                | V2Stmt::Match { .. }
        );
        self.written.clear();
        self.assigned = None;
        if let V2Stmt::Assign { target, value, .. } = s {
            self.written.insert(addr(target));
            self.assigned = Some(addr(value));
        }
        for e in stmt_exprs(s) {
            self.lint_expr(e);
        }
        if let V2Stmt::Assign {
            target,
            op: op @ (V2AssignOp::FlagSet | V2AssignOp::FlagClear),
            value,
            ..
        } = s
        {
            self.flag_constant(target, value);
            let (op, method) = if *op == V2AssignOp::FlagSet {
                ("flag_set", "set")
            } else {
                ("flag_clear", "clear")
            };
            self.flag_method(target, value, op, method);
        }
        self.written.clear();
        self.assigned = None;
        self.prefer_question_mark(s);
        if let V2Stmt::If { else_body, .. } = s
            && let [V2Stmt::If { span, .. }] = else_body.as_slice()
        {
            self.collapsible_else_if(*span);
        }
        match s {
            V2Stmt::While {
                span,
                cond: V2Expr::Bool(_, true),
                body,
                ..
            } => {
                if self.text_at(*span).starts_with("while") {
                    self.emit(
                        WHILE_TRUE,
                        *span,
                        "`while true` is spelled `loop` in v2".to_owned(),
                        Some("write `loop { ... }`; it lowers to the same tree".to_owned()),
                        Some("loop".to_owned()),
                        None,
                    );
                }
                if !self.loop_exits_or_yields(body) {
                    self.emit(
                        UNBOUNDED_LOOP_WITHOUT_YIELD,
                        *span,
                        "this loop never exits and never yields".to_owned(),
                        Some("add `idle();` / `sleep(n);` inside it, or a `break` / `return`; a loop that never yields runs until the VM's per-tick iteration limit errors the object out".to_owned()),
                        None,
                        None,
                    );
                }
            }
            V2Stmt::Match { arms, .. } => {
                let pats: Vec<&MatchPattern> = arms.iter().map(|a| &a.pattern).collect();
                self.enum_wildcard(&pats);
            }
            V2Stmt::Expr { span, expr } => {
                self.dropped_bool_call(*span, expr);
                self.prefer_method_syntax(expr);
            }
            // `let _ = f(..);` drops the call's result on purpose (no `dropped_bool_call`),
            // and the call may still have a method form.
            V2Stmt::Let { name, init, .. } if name == "_" => self.prefer_method_syntax(init),
            V2Stmt::RawAir { span, code } => {
                self.raw_builtin(*span, code);
                self.raw_blocks.push((*span, code.clone()));
            }
            _ => {}
        }
    }

    fn lint_expr(&mut self, e: &V2Expr) {
        let mut found: Vec<&V2Expr> = Vec::new();
        walk_expr(e, &mut |x| found.push(x));
        let mut parents: BTreeMap<usize, &V2Expr> = BTreeMap::new();
        for &x in &found {
            for c in subexprs(x) {
                parents.insert(addr(c), x);
            }
            match x {
                V2Expr::MutRef { inner, .. } => {
                    self.written.insert(addr(inner));
                }
                V2Expr::Call { callee, args, .. } => {
                    if let V2Expr::Ident(_, name) = callee.as_ref() {
                        for k in 0..args.len() {
                            if writes_arg(name, args, k) {
                                self.written.insert(addr(&args[k].value));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        for x in found {
            let parent = parents.get(&addr(x)).copied();
            self.untyped_engine_field(x, parent);
            self.object_tree_me_field(x, parent);
            match x {
                V2Expr::Binary {
                    lhs,
                    op: V2BinOp::FlagTest,
                    rhs,
                    ..
                } => {
                    self.flag_constant(lhs, rhs);
                    self.flag_method(lhs, rhs, "flag_test", "contains");
                }
                V2Expr::Call { .. } | V2Expr::MethodCall { .. } => self.call_constants(x),
                V2Expr::Int(span, v) if *v > 0xFFFF => {
                    let text = self.text_at(*span);
                    if text.starts_with("0x") || text.starts_with("0X") {
                        self.emit(
                            MAGIC_GUID_LITERAL,
                            *span,
                            format!("inline GUID literal `{}`", hex_guid(*v)),
                            Some(format!(
                                "name it: `const SOME_OBJECT = {};` and use the name here",
                                hex_guid(*v)
                            )),
                            None,
                            None,
                        );
                    }
                }
                V2Expr::Match { arms, .. } => {
                    let pats: Vec<&MatchPattern> = arms.iter().map(|a| &a.pattern).collect();
                    self.enum_wildcard(&pats);
                }
                V2Expr::If { else_value, .. } => {
                    if let V2Expr::If { span, .. } = else_value.as_ref() {
                        self.collapsible_else_if(*span);
                    }
                }
                _ => {}
            }
        }
    }

    /// `collapsible_else_if`: `else { if .. }`, an `else` block holding one `if` and
    /// nothing else, where `else if ..` says the same. `inner` is the `if` the `else` holds.
    ///
    /// The parser reads both spellings to the same tree, so the AST cannot tell them apart:
    /// the tokens around `inner` do. Reported only when the `if` and its `else` chain are
    /// the whole block, and no comment sits where the collapse drops text (between `else`,
    /// `{` and `if`, or between the chain's last `}` and the block's `}`).
    fn collapsible_else_if(&mut self, inner: Span) {
        let toks = self.tokens;
        if self.else_named {
            return;
        }
        let at =
            toks.partition_point(|t| (t.span.line, t.span.column) < (inner.line, inner.column));
        if at < 2 || !is_kw(toks.get(at), "if") {
            return;
        }
        let (open, else_at) = (at - 1, at - 2);
        if toks[open].kind != TokenKind::LBrace || !is_kw(toks.get(else_at), "else") {
            return;
        }
        let Some(end) = if_chain_end(toks, at) else {
            return;
        };
        let close = end + 1;
        if toks.get(close).map(|t| &t.kind) != Some(&TokenKind::RBrace) {
            return;
        }
        let offset = |i: usize| self.offset(toks[i].span);
        let (Some(e), Some(o), Some(f), Some(x), Some(c)) = (
            offset(else_at),
            offset(open),
            offset(at),
            offset(end),
            offset(close),
        ) else {
            return;
        };
        let blank = |from: usize, to: usize| {
            from <= to
                && self
                    .source
                    .get(from..to)
                    .is_some_and(|gap| gap.trim().is_empty())
        };
        if !(blank(e + 4, o) && blank(o + 1, f) && blank(x + 1, c)) {
            return;
        }
        self.emit(
            COLLAPSIBLE_ELSE_IF,
            toks[else_at].span,
            "this `else` block holds only an `if`".to_owned(),
            Some(
                "write `else if ..`: drop the `else` block's braces; it parses to the same tree"
                    .to_owned(),
            ),
            None,
            None,
        );
    }

    fn enum_wildcard(&mut self, pats: &[&MatchPattern]) {
        let Some(wild) = pats.iter().find_map(|p| match p {
            MatchPattern::Wildcard(s) => Some(*s),
            _ => None,
        }) else {
            return;
        };
        // Every alternative of an OR-pattern (`Mode::Low | Mode::High`) names its variant.
        fn alternatives<'p>(p: &'p MatchPattern, out: &mut Vec<&'p MatchPattern>) {
            match p {
                MatchPattern::Or(_, alts) => alts.iter().for_each(|a| alternatives(a, out)),
                other => out.push(other),
            }
        }
        let mut flat = Vec::new();
        for p in pats {
            match p {
                MatchPattern::Wildcard(_) => {}
                other => alternatives(other, &mut flat),
            }
        }
        let mut enum_name: Option<&str> = None;
        let mut named = BTreeSet::new();
        for p in flat {
            match p {
                MatchPattern::Path(_, segs) if segs.len() == 2 => {
                    if enum_name.is_some_and(|e| e != segs[0]) {
                        return;
                    }
                    enum_name = Some(&segs[0]);
                    named.insert(segs[1].as_str());
                }
                _ => return,
            }
        }
        let Some(name) = enum_name else {
            return;
        };
        let Some(Some(variants)) = self.enums.get(name) else {
            return;
        };
        if named.iter().any(|n| !variants.iter().any(|v| v == n)) {
            return;
        }
        let missing: Vec<&String> = variants
            .iter()
            .filter(|v| !named.contains(v.as_str()))
            .collect();
        match missing.as_slice() {
            [] => self.emit(
                ENUM_WILDCARD_ALL_COVERED,
                wild,
                format!("every variant of `{name}` is already matched, so this `_` arm only runs for an out-of-range value"),
                Some("remove the `_` arm".to_owned()),
                None,
                Some("to handle out-of-range raw values on purpose, type the value `i16` instead of the enum".to_owned()),
            ),
            [one] => self.emit(
                ENUM_WILDCARD_SINGLE_VARIANT,
                wild,
                format!("this `_` arm stands for exactly one variant, `{name}::{one}`"),
                Some(format!("name it, so a variant added later is not silently caught: `{name}::{one} => ...`")),
                Some(format!("{name}::{one}")),
                None,
            ),
            _ => {}
        }
    }

    fn dropped_bool_call(&mut self, span: Span, expr: &V2Expr) {
        let (name, kind) = match expr {
            // A call handing out `&mut` locals is made for those writes; its result is
            // often redundant with them.
            V2Expr::Call { args, .. }
                if args
                    .iter()
                    .any(|a| matches!(a.value, V2Expr::MutRef { .. })) =>
            {
                return;
            }
            V2Expr::Call { callee, .. } => match self.symbols.call_kind(callee) {
                Some(k) => k,
                None => return,
            },
            // The help spells the receiver: `is_type` alone is no callable.
            V2Expr::MethodCall {
                receiver, method, ..
            } if method == "is_type" => {
                fn dotted(e: &V2Expr) -> Option<String> {
                    match e {
                        V2Expr::Ident(_, r) => Some(r.clone()),
                        V2Expr::Field { base, field, .. } => {
                            Some(format!("{}.{field}", dotted(base)?))
                        }
                        _ => None,
                    }
                }
                let receiver = dotted(receiver).unwrap_or_else(|| "<object>".to_owned());
                (format!("{receiver}.is_type"), RetKind::Bool)
            }
            _ => return,
        };
        let help = match kind {
            RetKind::Result => format!(
                "propagate the failure with `{name}(..)?;`, test it with `if !{name}(..) {{ ... }}`, or drop it on purpose with `let _ = {name}(..);`"
            ),
            RetKind::Bool => format!(
                "test it: `if {name}(..) {{ ... }}`, return it, or drop it on purpose with `let _ = {name}(..);`"
            ),
            RetKind::Other => return,
        };
        self.emit(
            DROPPED_BOOL_CALL,
            span,
            format!(
                "the {} result of `{name}` is dropped",
                if kind == RetKind::Result {
                    "`Result`"
                } else {
                    "`bool`"
                }
            ),
            Some(help),
            None,
            None,
        );
    }

    fn loop_exits_or_yields(&self, body: &[V2Stmt]) -> bool {
        // A `break` of this loop (not of a nested one).
        fn breaks(list: &[V2Stmt]) -> bool {
            list.iter().any(|s| match s {
                V2Stmt::Break { .. } => true,
                V2Stmt::While { .. } | V2Stmt::Repeat { .. } | V2Stmt::ForEach { .. } => false,
                other => stmt_blocks(other).into_iter().any(breaks),
            })
        }
        if breaks(body) {
            return true;
        }
        let mut found = false;
        walk_stmts(body, &mut |s| {
            found |= matches!(
                s,
                V2Stmt::Return { .. }
                    | V2Stmt::ReturnError { .. }
                    | V2Stmt::RawAir { .. }
                    | V2Stmt::Animate { .. }
            );
        });
        found
            || any_expr(body, &mut |e| match e {
                V2Expr::Try { .. } => true,
                V2Expr::Call { callee, .. } => match callee.as_ref() {
                    V2Expr::Ident(_, n) => {
                        self.symbols.user_defined(n) || !NON_YIELDING_CALLS.contains(&n.as_str())
                    }
                    _ => true,
                },
                V2Expr::MethodCall { method, .. } => {
                    !NON_YIELDING_METHODS.contains(&method.as_str())
                }
                _ => false,
            })
    }

    /// Reports the first `self` after `stack_obj` is reassigned, once per reassignment.
    /// Returns whether `list` ends the clobbered state (a `stack_obj = self` restore, or a
    /// report) anywhere.
    fn clobber_walk(&mut self, list: &[V2Stmt], clobbered: Option<Span>) -> bool {
        // A `let` in this list ends with it: a shadowing binding's handle state does not
        // leak out (stores to outer bindings do).
        let declared: BTreeSet<String> = list
            .iter()
            .filter_map(|s| match s {
                V2Stmt::Let { name, .. }
                | V2Stmt::LetSome { name, .. }
                | V2Stmt::LetUninit { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        let outer: BTreeSet<String> = declared
            .iter()
            .filter(|n| self.self_handles.contains(*n))
            .cloned()
            .collect();
        let ended = self.clobber_walk_list(list, clobbered);
        for name in declared {
            if outer.contains(&name) {
                self.self_handles.insert(name);
            } else {
                self.self_handles.remove(&name);
            }
        }
        ended
    }

    fn clobber_walk_list(&mut self, list: &[V2Stmt], mut clobbered: Option<Span>) -> bool {
        let finds_best = |e: &V2Expr| {
            let mut found = false;
            walk_expr(e, &mut |x| {
                found |= matches!(x, V2Expr::Call { callee, .. }
                    if is_ident(callee, "find_best_interaction"));
            });
            found
        };
        let mut ended = false;
        for s in list {
            // A local that takes this object's handle before any reassignment holds it
            // (`let saved = stack_obj;`), and assigning it back restores `self`.
            let handle = |e: &V2Expr| is_ident(e, "stack_obj") || is_ident(e, "self");
            match s {
                V2Stmt::Let { name, init, .. } => {
                    if clobbered.is_none() && handle(init) {
                        self.self_handles.insert(name.clone());
                    } else {
                        self.self_handles.remove(name);
                    }
                }
                V2Stmt::Assign {
                    target: V2Expr::Ident(_, name),
                    value,
                    ..
                } if name != "stack_obj" => {
                    if clobbered.is_none() && handle(value) {
                        self.self_handles.insert(name.clone());
                    } else {
                        self.self_handles.remove(name);
                    }
                }
                _ => {}
            }
            let is_clobber = match s {
                V2Stmt::Assign {
                    target,
                    op: V2AssignOp::Set,
                    value,
                    ..
                } if is_ident(target, "stack_obj") => {
                    let restores = is_ident(value, "self")
                        || matches!(value, V2Expr::Ident(_, n) if self.self_handles.contains(n));
                    if restores {
                        clobbered = None;
                        ended = true;
                        continue;
                    }
                    // `stack_obj = stack_obj;` changes nothing.
                    !is_ident(value, "stack_obj")
                }
                // `find_best_interaction()` leaves its pick in `stack_obj`.
                other => {
                    !self.symbols.user_defined("find_best_interaction")
                        && stmt_exprs(other).into_iter().any(finds_best)
                }
            };
            if let Some(at) = clobbered
                && let Some(use_at) = stmt_exprs(s)
                    .into_iter()
                    .find_map(|e| mentions_ident(e, "self"))
            {
                self.report_clobber(use_at, at);
                clobbered = None;
                ended = true;
            }
            if is_clobber {
                clobbered = Some(stmt_span(s));
            }
            let mut inner_ended = false;
            for b in stmt_blocks(s) {
                inner_ended |= self.clobber_walk(b, clobbered);
            }
            if inner_ended {
                clobbered = None;
                ended = true;
            }
        }
        ended
    }

    fn report_clobber(&mut self, use_at: Span, clobber_at: Span) {
        self.emit(
            STACK_OBJ_CLOBBER_AFTER_SELF,
            use_at,
            format!(
                "`self` here is the object `stack_obj` was set to on line {}, not this object",
                clobber_at.line
            ),
            Some("in a Sim-context body `self` follows `stack_obj`. To act on the other object, write `stack_obj.<field>` (`stack_obj.attr[N]` for a state) — it compiles to the same operand and says so; to act on this object, restore it first with `stack_obj = self;` (or a handle saved before the swap)".to_owned()),
            None,
            None,
        );
    }

    fn redundant_bool_return(&mut self, list: &[V2Stmt], i: usize) {
        let ret_bool = |s: &[V2Stmt]| match s {
            [
                V2Stmt::Return {
                    value: Some(V2Expr::Bool(_, b)),
                    ..
                },
            ] => Some(*b),
            _ => None,
        };
        let V2Stmt::If {
            span,
            cond,
            then_body,
            else_body,
        } = &list[i]
        else {
            return;
        };
        let Some(t) = ret_bool(then_body) else {
            return;
        };
        let other = if else_body.is_empty() {
            list.get(i + 1)
                .and_then(|n| ret_bool(std::slice::from_ref(n)))
        } else {
            ret_bool(else_body)
        };
        // Only the same-polarity form: `return c;` ends on the node that tests `c`, with
        // the same exits, so the tree's last result is unchanged. The inverted form
        // (`if c { return false; } return true;`) would need `return !c;`, which ends
        // on the opposite last result, observable at the tree's exit.
        if !t || other != Some(false) {
            return;
        }
        // The `if` lowers to the node testing `c` only when `c` cannot fold: a constant
        // comparison (`1 < 2`) folds away, leaving the node before it as the last
        // result, where `return 1 < 2;` would run a compare. A literal or `&&` / `||`
        // with one may fold in part. Each comparison must read a place or call.
        // A `defer` runs its block before each `return`, so the two forms lower
        // differently.
        // Only one test, or `a && b` of two: a longer chain lowers `return` through a
        // local. Inside `with lock` (or `defer`, `animate`) `return` leaves through the
        // block's exit, after the test.
        if may_fold(cond) || self.body_has_defer || self.in_block > 0 || !one_or_two_tests(cond) {
            return;
        }
        let suggestion = print_expr(cond).map(|c| format!("return {c};"));
        self.emit(
            REDUNDANT_BOOL_RETURN,
            *span,
            "this `if` returns true when the condition holds and false otherwise".to_owned(),
            Some("return the condition directly: `return <condition>;`".to_owned()),
            suggestion,
            Some(
                "`if c { return false; } return true;` is not reported: `return !c;` ends the \
                 tree on the opposite last result"
                    .to_owned(),
            ),
        );
    }

    /// `negated_comparison`: an `if` / `while` testing `!(a op b)` whose both successors
    /// run a node before the tree can end, so the flipped compare's opposite result is
    /// never the tree's last result.
    fn negated_comparison(&mut self, list: &[V2Stmt], i: usize) {
        let next = list.get(i + 1);
        let (cond, on_true, on_false) = match &list[i] {
            V2Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => (cond, then_body.first().or(next), else_body.first().or(next)),
            // An empty body loops back to the test, which is a node.
            V2Stmt::While { cond, body, .. } => (cond, body.first().or(Some(&list[i])), next),
            _ => return,
        };
        let V2Expr::Unary {
            span,
            op: V2UnaryOp::Not,
            inner,
        } = cond
        else {
            return;
        };
        let V2Expr::Binary { lhs, op, rhs, .. } = inner.as_ref() else {
            return;
        };
        let Some(flip) = flipped(*op) else {
            return;
        };
        let sim = self.sim_body;
        let (states, consts) = (&self.states, &self.consts);
        let quiet = |s: Option<&V2Stmt>| {
            s.is_some_and(|s| {
                runs_a_quiet_node(s, sim, &|f| states.contains_key(f), &|n| consts.contains(n))
            })
        };
        if !(quiet(on_true) && quiet(on_false)) {
            return;
        }
        let operand = |e: &V2Expr| {
            print_expr(e).map(|t| {
                if matches!(e, V2Expr::Binary { .. } | V2Expr::If { .. }) {
                    format!("({t})")
                } else {
                    t
                }
            })
        };
        let suggestion = operand(lhs)
            .zip(operand(rhs))
            .map(|(l, r)| format!("{l} {} {r}", binop_str(flip)));
        self.emit(
            NEGATED_COMPARISON,
            *span,
            format!(
                "`!(a {} b)` is `a {} b`",
                binop_str(*op),
                binop_str(flip)
            ),
            Some(format!("write the comparison with `{}`", binop_str(flip))),
            suggestion,
            Some("v2 keeps `!(a op b)` as the negated compare node with its branches swapped (the opposite last-result polarity); this one is only reported because both branches go on to a plain store that cannot fault, so the node's own result is never the tree's last result. Before a call, a primitive, `sleep` or the tree's end the result is observable, and that is not reported. If you mirror an original tree's node bytes on purpose, allow this lint".to_owned()),
        );
    }

    fn collapsible_chain(&mut self, s: &V2Stmt) {
        let mut key: Option<String> = None;
        let mut values: Vec<String> = Vec::new();
        let mut scrutinee: Option<&V2Expr> = None;
        let mut patterns: Vec<&V2Expr> = Vec::new();
        let mut has_else = false;
        let mut cur = s;
        while let V2Stmt::If {
            cond, else_body, ..
        } = cur
        {
            let V2Expr::Binary {
                lhs,
                op: V2BinOp::Eq,
                rhs,
                ..
            } = cond
            else {
                return;
            };
            let (Some(k), Some(v)) = (place_key(lhs), pattern_text(rhs)) else {
                return;
            };
            scrutinee = Some(lhs);
            patterns.push(rhs);
            if key.as_ref().is_some_and(|x| *x != k) || values.contains(&v) {
                return;
            }
            key = Some(k);
            values.push(v);
            match else_body.as_slice() {
                [next @ V2Stmt::If { .. }] => cur = next,
                [] => break,
                _ => {
                    has_else = true;
                    break;
                }
            }
        }
        if values.len() < 3 {
            return;
        }
        let (Some(key), Some(scrutinee)) = (key, scrutinee) else {
            return;
        };
        if !self.matchable(scrutinee, &patterns) {
            return;
        }
        let arms: Vec<String> = values.iter().map(|v| format!("{v} => {{ ... }}")).collect();
        let fallback = if has_else { "_ => { ... }" } else { "_ => {}" };
        self.emit(
            COLLAPSIBLE_IF_CHAIN,
            stmt_span(s),
            format!("{} `if`s test `{key}` against one value each", values.len()),
            Some(format!(
                "write a `match`: `match {key} {{ {} {fallback} }}`",
                arms.join(" ")
            )),
            None,
            None,
        );
    }

    /// Whether `match scrutinee { patterns.. }` would compile: integer literals against an
    /// `i16` place (`temp[N]`, `param[N]`, an `i16` state), or variants of one resolved enum
    /// against a state of that enum. Places of unknown or other types are not judged.
    fn matchable(&self, scrutinee: &V2Expr, patterns: &[&V2Expr]) -> bool {
        let ty = match scrutinee {
            V2Expr::Index { base, .. } if is_ident(base, "temp") || is_ident(base, "param") => {
                StateType::I16
            }
            V2Expr::Field { base, field, .. } if is_ident(base, "self") => {
                match self.states.get(field) {
                    Some(Some(ty)) => ty.clone(),
                    _ => return false,
                }
            }
            _ => return false,
        };
        match ty {
            StateType::I16 => patterns
                .iter()
                .all(|p| pattern_text(p).is_some() && !matches!(p, V2Expr::Path(..))),
            StateType::Named(e) => patterns.iter().all(|p| match p {
                V2Expr::Path(_, segs) => {
                    segs.len() == 2
                        && segs[0] == e
                        && matches!(self.enums.get(&e), Some(Some(vs)) if vs.contains(&segs[1]))
                }
                _ => false,
            }),
            _ => false,
        }
    }

    /// `raw_bare_call_false_exit`: a `raw` block that v2 code follows, ending in a bare
    /// `call N(...);` statement, whose false edge is the tree's false exit rather than
    /// the code after the block. (Inside a block of `.air`, the false exit is visible to
    /// whoever writes `.air`; a lifted tree mirrors the original's.)
    fn raw_bare_call(&mut self, span: Span, code: &str) {
        // Comments blanked in place, so offsets into it are offsets into `code`.
        let stripped = blank_air_comments(code);
        let Some(last) = stripped
            .split([';', '{', '}'])
            .rev()
            .find(|f| !f.trim().is_empty())
        else {
            return;
        };
        // The call must be the block's last statement: nothing (not a `}`) after its `;`.
        if !stripped.trim_end().ends_with(';') {
            return;
        }
        {
            // A label (`@done:`) may start the statement.
            let mut t = last.trim();
            while let Some(rest) = t.strip_prefix('@')
                && let Some((label, after)) = rest.split_once(':')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                t = after.trim_start();
            }
            // Point at the call's own line (past any label): `t` is a slice of `stripped`.
            let pos = t.as_ptr() as usize - stripped.as_ptr() as usize;
            let Some(rest) = t.strip_prefix("call") else {
                return;
            };
            // `call N(..) -> true: @a, false: @b;` names its own edges.
            if !rest.starts_with(char::is_whitespace) || rest.contains("->") {
                return;
            }
            let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
            let at = self.raw_span(span, pos);
            self.emit(
                RAW_BARE_CALL_FALSE_EXIT,
                at,
                format!("`{t};` as a statement exits the tree false when the call returns false"),
                Some(format!("to go on after either result, write `if {t} {{}}`; allow this lint if the false exit is meant")),
                None,
                Some("in `.air` a statement's false edge defaults to the tree's false exit; a v2 call statement goes on either way".to_owned()),
            );
        }
    }

    /// The source position of byte `pos` of the body of the `raw` block at `span`.
    fn raw_span(&self, span: Span, pos: usize) -> Span {
        let Some(&body) = self.raw_bodies.get(&(span.line, span.column)) else {
            return span;
        };
        let at = body + pos;
        let line = self.line_starts.partition_point(|&s| s <= at);
        Span::new(line, at - self.line_starts[line - 1] + 1)
    }

    /// Whether a bare call to builtin `name` in the body being linted surely means the builtin:
    /// no user function the file can call takes the name, and the body is not a template's
    /// member (an object in another file that extends the template may declare one).
    fn call_free(&self, name: &str) -> bool {
        !self.in_template && !self.user_fn(name)
    }

    fn raw_builtin(&mut self, span: Span, code: &str) {
        let code: Vec<&str> = code
            .lines()
            .map(|l| {
                let l = l.split("//").next().unwrap_or("");
                l.split('#').next().unwrap_or("")
            })
            .collect();
        // One `.air` statement per entry: only `;` ends one, never a line break.
        let code = code.join("\n");
        let mut lines: Vec<&str> = code.split(';').map(str::trim).collect();
        // The block must end its last statement and hold no empty one: `.air` reads adjacent
        // raw blocks as one stream, so `raw { f() } raw { pad(3: 0x05); }` is one padded
        // primitive, and the leading `;` of `raw { f() } raw { ; g(); }` ends `f()`.
        if lines.pop() != Some("") || lines.is_empty() || lines.iter().any(|l| l.is_empty()) {
            return;
        }
        let mut spellings = Vec::new();
        for l in &lines {
            let word: String = l
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            // Only the two primitives v2 gives `padN:` bytes may carry a `pad`.
            let padded = matches!(word.as_str(), "play_sound" | "clear_motive_changes");
            if (!padded
                && l.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .any(|w| w == "pad"))
                || l.contains("->")
                || l.contains('{')
                || l.contains('}')
            {
                return;
            }
            let v2 = match RAW_BUILTINS.iter().find(|(air, _)| *air == word) {
                Some((_, v2))
                    if native_operands(&word, l[word.len()..].trim())
                        && self.call_free(v2.split('(').next().unwrap_or(v2)) =>
                {
                    (*v2).to_owned()
                }
                Some(_) => return,
                None => match self.raw_native_form(&word, l) {
                    Some(v2) => v2,
                    None => return,
                },
            };
            if !spellings.contains(&v2) {
                spellings.push(v2);
            }
        }
        self.emit(
            MANUAL_RAW_HAS_BUILTIN,
            span,
            "this `raw` block only holds primitives v2 spells natively".to_owned(),
            Some(format!("use {}", spellings.join(", "))),
            None,
            None,
        );
    }

    /// The v2 spelling of one raw `.air` statement `l` (leading word `word`) that PR I-3's
    /// forms spell with the same operands, or `None`:
    /// - `remove_object(me | stack_obj, flags: N)` (target 2 has no v2 spelling);
    /// - `find_best_object(kind, flags: N)` with no `origin:`, as an `if` / `while` test;
    /// - `set_to_next(mode, dest: X)` into `temp[0..7]`, or `param[N]` below the tree's
    ///   compiled argument count (not `beside`, which takes `local:`);
    /// - `temp[K] := scope(21, 9 | 11)` or `scope(33, F)`: the stack object's `object_type` /
    ///   `sub_index`, and `job_field(F)`.
    ///
    /// And PR I-4's (#469), each lowering to the statement's own `.air` text:
    /// - `play_sound(N [, false | true | stack_obj [: bool]]) pad(i: 0xNN, ..)` with a
    ///   literal FWAV id, as `play_sound(N, false | stack_obj, padI: 0xNN)` (an explicit target,
    ///   so the spelling does not depend on the tree's context); not `flags:`, not a pad of
    ///   bytes 0, 1 or 4, which the call spells, and not without a pad (wave 1 leaves that raw,
    ///   since v2 would rather name a declared `sound`);
    /// - `clear_motive_changes() [pad(i: 0xNN, ..)]`, as `clear_motive_changes(padI: 0xNN)`;
    /// - `reach(mode: M, moves: B [, param: P])`: mode 1 as `slot: param[P]` (P below the
    ///   tree's compiled argument count; `.air` defaults P to 0), any other mode only with P 0;
    /// - `temp[K] := scope(S, N)` and `scope(S, N) := X` (X a literal, `temp[J]` or a
    ///   `param[J]` as above) for the cells #469 spells: `advert.delta[N]` (12, N 0..16),
    ///   `stack_obj.sprite_flags[temp[N]]` (27), `me.person[temp[N]]` (30) and
    ///   `stack_obj.person[temp[N]]` (31) with N 0..=7; and the read-only neighbor cells
    ///   `stack_obj.neighbor.<field>` / `.field[N]` (24, N 0..=9) and
    ///   `stack_obj.neighbor.person[N]` (32, N 0..=255).
    ///
    /// `make_new_character` is not spelled: v2 reads skin, age and sex from named `i16` locals,
    /// and a raw block names only frame words, which a v2 local need not land on.
    ///
    /// Raw code cannot name a v2 local, and `has_function[N]` (`scope(35, N)`) is a `bool`
    /// no `temp` takes, so neither is a store here. Nothing is spelled when the body rebinds
    /// the name the spelling uses (`temp`, `param`, `me`, `stack_obj`), or a user function
    /// shadows the builtin.
    fn raw_native_form(&self, word: &str, l: &str) -> Option<String> {
        let rest = l[word.len()..].trim();
        let args = || -> Option<Vec<&str>> {
            let inner = rest.strip_prefix('(')?.strip_suffix(')')?.trim();
            (!inner.is_empty()).then(|| inner.split(',').map(str::trim).collect())
        };
        let in_range =
            |v: &str, lo: i64, hi: i64| air_literal(v).is_some_and(|n| (lo..=hi).contains(&n));
        // A builtin, or a name the body has not rebound.
        let free = |name: &str| {
            !self.bound.contains(name)
                && !self.sim_bindings.contains(name)
                && !self.const_names.contains(name)
                && !self.user_fn(name)
        };
        // A builtin's call name: in a template's member, an extending object's member of
        // that name would take the call.
        let free_call = |name: &str| free(name) && !self.in_template;
        let index = |cell: &str, base: &str| -> Option<i64> {
            let inner = cell
                .strip_prefix(base)?
                .trim_start()
                .strip_prefix('[')?
                .strip_suffix(']')?;
            air_literal(inner.trim())
        };
        let temp =
            |cell: &str| free("temp") && index(cell, "temp").is_some_and(|n| (0..=7).contains(&n));
        let args_count = self
            .tree
            .as_ref()
            .and_then(|t| t.frame.as_ref())
            .and_then(|f| f.args);
        let param = |cell: &str| {
            free("param")
                && index(cell, "param").is_some_and(|n| {
                    args_count.is_some_and(|args| usize::try_from(n).is_ok_and(|n| n < args))
                })
        };
        match word {
            "remove_object" if free_call("remove_object") => {
                let args = args()?;
                let (target, opts) = args.split_first()?;
                // `.air` reads `me` / `self` / `0` as the frame's object, as v2's `me` is.
                let target = match (*target, air_literal(target)) {
                    ("me" | "self", _) | (_, Some(0)) if free("me") && !self.me_bcon => "me",
                    ("stack_obj", _) | (_, Some(1)) if free("stack_obj") => "stack_obj",
                    _ => return None,
                };
                let mut flags = 0i64;
                for o in opts {
                    let (k, v) = o.split_once(':')?;
                    match (k.trim(), v.trim()) {
                        ("flags", v) => {
                            flags |= air_literal(v).filter(|n| (0..=255).contains(n))?
                        }
                        ("no_yield", "true") => flags |= 1,
                        ("skip_resets", "true") => flags |= 2,
                        ("no_yield" | "skip_resets", "false") => {}
                        _ => return None,
                    }
                }
                Some(if flags == 0 {
                    format!("remove_object({target})")
                } else {
                    format!("remove_object({target}, flags: {flags})")
                })
            }
            "find_best_object" if free_call("find_best_object") => {
                let args = args()?;
                let (kind, opts) = args.split_first()?;
                let searched = |k: &crate::parser::FunctionKind| {
                    k.search != crate::parser::FunctionSearch::None
                };
                let named = match air_literal(kind) {
                    Some(n) => crate::parser::FUNCTION_KINDS
                        .iter()
                        .any(|k| i64::from(k.function) == n && searched(k)),
                    None => crate::parser::function_kind(kind).is_some_and(searched),
                };
                let ok = named
                    && opts.len() <= 1
                    && opts.iter().all(|o| match o.split_once(':') {
                        Some((k, v)) => k.trim() == "flags" && in_range(v.trim(), 0, 255),
                        None => in_range(o, 0, 255),
                    });
                // Only where `self` is the object: in a Sim's body, v2 saves and restores the
                // entry stack object around the search for a later `self`, which the raw
                // block does not.
                (ok && self.object_me)
                    .then(|| "`find_best_object(..)` as an `if` / `while` test".to_owned())
            }
            "set_to_next" if free_call("set_to_next") => {
                const MODES: &[&str] = &[
                    "all",
                    "person",
                    "non_person",
                    "part",
                    "tile",
                    "guid",
                    "neighbor",
                    "category",
                    "neighbor_guid",
                    "on_tile",
                    "career",
                    "family",
                ];
                let args = args()?;
                let (mode, opts) = args.split_first()?;
                let mode_ok = MODES.contains(mode)
                    || air_literal(mode).is_some_and(|n| (0..=12).contains(&n) && n != 9);
                let mut dest = false;
                let ok = mode_ok
                    && opts.iter().all(|o| match o.split_once(':') {
                        Some((k, v)) => match k.trim() {
                            "guid" => in_range(v.trim(), 0, 0xFFFF_FFFF),
                            "dest" | "into" if !dest => {
                                dest = true;
                                temp(v.trim()) || param(v.trim())
                            }
                            _ => false,
                        },
                        // A positional GUID.
                        None => in_range(o, 0, 0xFFFF_FFFF),
                    });
                (ok && dest).then(|| "set_to_next(.., dest: ..)".to_owned())
            }
            "play_sound" | "clear_motive_changes" if free_call(word) => {
                // `name(args)`, then an optional `pad(i: 0xNN, ..)`.
                let close = rest.find(')')?;
                let inner = rest.strip_prefix('(')?[..close - 1].trim();
                let after = rest[close + 1..].trim();
                let mut pads: Vec<(u8, u8)> = Vec::new();
                if !after.is_empty() {
                    let bytes = after
                        .strip_prefix("pad")?
                        .trim_start()
                        .strip_prefix('(')?
                        .strip_suffix(')')?;
                    for b in bytes.split(',') {
                        let (i, v) = b.split_once(':')?;
                        let i = u8::try_from(air_literal(i.trim())?).ok()?;
                        let v = u8::try_from(air_literal(v.trim())?).ok()?;
                        pads.push((i, v));
                    }
                }
                // The call's own operands, as the `.air` encoder lays them out.
                let (opcode, base, spelled, target) = if word == "clear_motive_changes" {
                    if !inner.is_empty() {
                        return None;
                    }
                    (29, [0, 0, 0, 1, 0, 0, 0, 0], &[][..], None)
                } else {
                    // Wave 1 leaves an unpadded sound raw: v2 would rather name a declared
                    // sound.
                    if pads.is_empty() {
                        return None;
                    }
                    let mut args = inner.split(',').map(compact_air);
                    let sound = u16::try_from(air_literal(&args.next()??)?).ok()?;
                    let on_stack = match args.next().map(|a| a.unwrap_or_default()).as_deref() {
                        None | Some("false" | "stack_obj:false") => false,
                        Some("true" | "stack_obj" | "stack_obj:true") => true,
                        Some(_) => return None,
                    };
                    if args.next().is_some() {
                        return None;
                    }
                    let [s0, s1] = sound.to_le_bytes();
                    let flags = if on_stack { 2 } else { 0 };
                    let target = match (on_stack, free("stack_obj")) {
                        (false, _) => "false",
                        (true, true) => "stack_obj",
                        (true, false) => "true",
                    };
                    (
                        23,
                        [s0, s1, 0, 0, flags, 0, 0, 0],
                        &[0u8, 1, 4][..],
                        Some(target),
                    )
                };
                // v2 refuses a pad of a byte the call spells; the encoder refuses the rest
                // (order, repeats, a byte that already holds the value, a byte the VM reads).
                if pads.iter().any(|(i, _)| spelled.contains(i))
                    || (!pads.is_empty()
                        && crate::verify::apply_operand_pad(
                            opcode,
                            base,
                            &crate::ast::OperandPad::Bytes(pads.clone()),
                        )
                        .is_err())
                {
                    return None;
                }
                let pad = if pads.is_empty() { "" } else { "padN: 0xNN" };
                Some(match target {
                    None => format!("clear_motive_changes({pad})"),
                    Some(target) => format!("play_sound(.., {target}, {pad})"),
                })
            }
            "reach" if free_call("reach") => {
                let (mut mode, mut moves, mut slot) = (None, None, None);
                for a in args()? {
                    let (k, v) = a.split_once(':')?;
                    let v = v.trim();
                    let cell = match k.trim() {
                        "mode" => &mut mode,
                        "moves" => &mut moves,
                        "param" | "slot_param" => &mut slot,
                        _ => return None,
                    };
                    if cell.replace(v).is_some() {
                        return None;
                    }
                }
                // `.air` defaults the mode to 0.
                let mode = match mode {
                    None => 0,
                    Some(m) => air_literal(m).filter(|m| (-32768..=32767).contains(m))?,
                };
                if !moves.is_none_or(|b| matches!(b, "true" | "false")) {
                    return None;
                }
                // `.air` reads `param: param[P]` or `param: P`.
                let p = match slot {
                    None => 0,
                    Some(v) => match index(v, "param") {
                        Some(p) => p,
                        None => air_literal(v)?,
                    },
                };
                if mode == 1 {
                    param(&format!("param[{p}]"))
                        .then(|| "reach(mode: 1, moves: .., slot: param[..])".to_owned())
                } else {
                    (p == 0).then(|| "reach(mode: .., moves: ..)".to_owned())
                }
            }
            "remove_object"
            | "find_best_object"
            | "set_to_next"
            | "play_sound"
            | "clear_motive_changes"
            | "reach" => None,
            _ => {
                let (dst, src) = l.split_once(":=")?;
                let (dst, src) = (compact_air(dst)?, compact_air(src)?);
                let scope_cell = |cell: &str| -> Option<(i64, i64)> {
                    let inner = cell.strip_prefix("scope(")?.strip_suffix(')')?;
                    let (scope, n) = inner.split_once(',')?;
                    Some((air_literal(scope)?, air_literal(n)?))
                };
                // A cell #469 spells, readable and writable alike.
                let cell = |scope: i64, n: i64| -> Option<String> {
                    let temp_n = (0..=7).contains(&n) && free("temp");
                    match scope {
                        12 if (0..16).contains(&n) && free("advert") => {
                            Some(format!("advert.delta[{n}]"))
                        }
                        27 if temp_n && free("stack_obj") => {
                            Some("stack_obj.sprite_flags[temp[..]]".to_owned())
                        }
                        30 if temp_n && free("me") && !self.me_bcon => {
                            Some("me.person[temp[..]]".to_owned())
                        }
                        31 if temp_n && free("stack_obj") => {
                            Some("stack_obj.person[temp[..]]".to_owned())
                        }
                        _ => None,
                    }
                };
                if temp(&dst) {
                    let (scope, n) = scope_cell(&src)?;
                    match (scope, n) {
                        (21, 9) if free("stack_obj") => Some("stack_obj.object_type".to_owned()),
                        (21, 11) if free("stack_obj") => Some("stack_obj.sub_index".to_owned()),
                        (33, 0..=32767) if free_call("job_field") => {
                            Some(format!("job_field({n})"))
                        }
                        (24, 0..=9) if free("stack_obj") => Some(match n {
                            2 => "stack_obj.neighbor.age".to_owned(),
                            5 => "stack_obj.neighbor.friend_count".to_owned(),
                            6 => "stack_obj.neighbor.family_house".to_owned(),
                            9 => "stack_obj.neighbor.family_friend_count".to_owned(),
                            _ => format!("stack_obj.neighbor.field[{n}]"),
                        }),
                        (32, 0..=255) if free("stack_obj") => {
                            Some(format!("stack_obj.neighbor.person[{n}]"))
                        }
                        _ => cell(scope, n),
                    }
                } else {
                    let (scope, n) = scope_cell(&dst)?;
                    (temp(&src) || param(&src) || in_range(&src, -32768, 32767))
                        .then(|| cell(scope, n))
                        .flatten()
                }
            }
        }
    }

    /// `unreachable_code`: the first statement of `list` after one that always leaves it.
    /// Not in a body whose raw code has an explicit edge (`-> true: @a` / `there` / `2`):
    /// it may jump to any later statement. Not when a `raw { }` block from there on defines
    /// an `.air` label, and not for a lone `return` closing the list
    /// after an `if` / `match` whose every branch leaves (a trailing return written for the
    /// reader).
    fn unreachable_code(&mut self, list: &[V2Stmt]) {
        if self.raw_edges {
            return;
        }
        let Some(i) = list.iter().position(stmt_leaves) else {
            return;
        };
        let Some(next) = list.get(i + 1) else {
            return;
        };
        let direct = matches!(
            list[i],
            V2Stmt::Return { .. }
                | V2Stmt::ReturnError { .. }
                | V2Stmt::Break { .. }
                | V2Stmt::Continue { .. }
        );
        if !direct && i + 2 == list.len() && matches!(next, V2Stmt::Return { .. }) {
            return;
        }
        let mut labelled = false;
        walk_stmts(&list[i + 1..], &mut |s| {
            if let V2Stmt::RawAir { code, .. } = s {
                labelled |= raw_defines_label(code);
            }
        });
        if labelled {
            return;
        }
        let what = match &list[i] {
            V2Stmt::Return { .. } => "`return`",
            V2Stmt::ReturnError { .. } => "`panic!()`",
            V2Stmt::Break { .. } => "`break`",
            V2Stmt::Continue { .. } => "`continue`",
            V2Stmt::If { .. } => "every branch of the `if`",
            _ => "every arm of the `match`",
        };
        let at = stmt_span(&list[i]);
        self.emit(
            UNREACHABLE_CODE,
            stmt_span(next),
            "unreachable code: nothing can run this statement".to_owned(),
            Some(
                "remove it, or move it before the statement that leaves; the build drops it"
                    .to_owned(),
            ),
            None,
            Some(format!(
                "{what} on line {} always leaves this block",
                at.line
            )),
        );
    }

    /// `let` bindings never read, and `let x: T;` read before any assignment.
    fn unused_let(&mut self, list: &[V2Stmt], i: usize) {
        let (span, name, uninit) = match &list[i] {
            V2Stmt::Let { span, name, .. } | V2Stmt::LetSome { span, name, .. } => {
                (*span, name, None)
            }
            V2Stmt::LetUninit { span, name, ty, .. } => (*span, name, Some(ty)),
            _ => return,
        };
        // `#[slot(N)] let`: the frame word is pinned so that code addressing it by number
        // (raw `op` bytes, a primitive, another tree sharing the frame) can read or fill
        // it; the source does not say which, and the build keeps every store to it.
        if name.starts_with('_')
            || matches!(
                &list[i],
                V2Stmt::Let { slot: Some(_), .. } | V2Stmt::LetUninit { slot: Some(_), .. }
            )
        {
            return;
        }
        let rest = &list[i + 1..];
        let mut read = false;
        // A raw block that addresses `local[N]` may be reading or writing this binding's
        // slot, which the source does not say.
        let mut by_slot = false;
        walk_stmts(rest, &mut |s| {
            if let V2Stmt::RawAir { code, .. } = s {
                read |= code
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .any(|w| w == name);
                by_slot |= raw_indexes(code, "local");
            }
            for (k, e) in stmt_exprs(s).into_iter().enumerate() {
                // The target of a plain `x = ...` is a write, not a read.
                if k == 0
                    && matches!(
                        s,
                        V2Stmt::Assign {
                            op: V2AssignOp::Set,
                            ..
                        }
                    )
                    && is_ident(e, name)
                {
                    continue;
                }
                // `animate(.., local: x)` only writes `x`, unless its handler reads the
                // event binding, which is the same frame word.
                if let V2Stmt::Animate {
                    args, event, body, ..
                } = s
                    && !self.user_fn("animate")
                    && args
                        .get(k)
                        .is_some_and(|a| a.name.as_deref() == Some("local"))
                    && is_ident(e, name)
                {
                    if let Some(event) = event {
                        walk_stmts(body, &mut |t| {
                            if let V2Stmt::RawAir { code, .. } = t {
                                read |= code
                                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                                    .any(|w| w == event);
                            }
                            for e in stmt_exprs(t) {
                                read |= reads_ident(e, event, &|f| self.user_fn(f));
                            }
                        });
                    }
                    continue;
                }
                // `set_to_next(.., dest: x)` reads `x`; `random_into(x, ..)` only writes it.
                read |= reads_ident(e, name, &|f| self.user_fn(f));
            }
        });
        if by_slot {
            return;
        }
        // `else` bodies of `let Some` cannot see the binding; its own uses are in `rest`.
        if !read && !self.read_by_string_token(name) {
            let note = self.string_token_note(name);
            self.emit(
                UNUSED_DECL,
                span,
                format!("local `{name}` is never read"),
                Some(format!("remove it, or name it `_{name}`")),
                None,
                note,
            );
            return;
        }
        if !read {
            return;
        }
        if let Some(ty) = uninit
            && let Some(at) = first_unassigned_read(rest, name, &|f| self.sure_user_fn(f))
        {
            // The initialized `let` keeps the declared type (`let c: ObjectId = 0;`).
            let init = match ty {
                V2Type::I16 => format!(": `let {name} = 0;`"),
                V2Type::Bool => format!(": `let {name}: bool = false;`"),
                V2Type::ObjectId | V2Type::Guid => {
                    format!(": `let {name}: {} = 0;`", ty.display_name())
                }
                _ => " where it is declared".to_owned(),
            };
            self.emit(
                UNINITIALIZED_LOCAL_READ,
                at,
                format!("`{name}` is read before anything assigns it"),
                Some(format!("assign it before this read, or initialize it{init} (one more node)")),
                None,
                Some("the VM zeroes a tree's locals on entry, so this reads 0; say so with `= 0` if that is meant".to_owned()),
            );
        }
    }

    /// `frame_headroom`: compiles the file (with its pack, when it has one) and checks
    /// every tree's node and local counts.
    fn frame_headroom(&mut self, ast: &V2File, objects: Option<&[crate::compile::CompiledObject]>) {
        let Some(objects) = objects else {
            return;
        };
        // Per source object: its span, and the spans of its members that become trees, by
        // pinned id and by label (labels repeat across objects).
        struct Spans {
            name: String,
            span: Span,
            by_id: BTreeMap<u16, Span>,
            /// `None` for a label two members share.
            by_label: BTreeMap<String, Option<Span>>,
        }
        let mut sources = Vec::new();
        for item in &ast.items {
            let V2Item::Object(o) = item else {
                continue;
            };
            let mut spans = Spans {
                name: o.name.clone(),
                span: o.span,
                by_id: BTreeMap::new(),
                by_label: BTreeMap::new(),
            };
            // Like the lowering, the last pinned `init` / `main` block's pin is the one the
            // lifecycle tree takes; an earlier, superseded pin names no tree of its own.
            let mut lifecycle_pins: [Option<(u16, Span)>; 2] = [None, None];
            for m in &o.members {
                let (id, label, span) = match m {
                    ObjectMember::Tree(t) => (t.pinned_id, t.label.as_str(), t.span),
                    ObjectMember::Interaction(i) => (i.pinned_id, i.label.as_str(), i.span),
                    ObjectMember::Init(b) | ObjectMember::Main(b) => {
                        let (slot, label) = match m {
                            ObjectMember::Init(_) => (0, "init"),
                            _ => (1, "main"),
                        };
                        if let Some(id) = b.pinned_id {
                            lifecycle_pins[slot] = Some((id, b.span));
                        }
                        (None, label, b.span)
                    }
                    _ => continue,
                };
                if let Some(id) = id {
                    spans.by_id.insert(id, span);
                }
                spans
                    .by_label
                    .entry(label.to_owned())
                    .and_modify(|s| *s = None)
                    .or_insert(Some(span));
            }
            for (id, span) in lifecycle_pins.into_iter().flatten() {
                spans.by_id.entry(id).or_insert(span);
            }
            sources.push(spans);
        }
        let single = sources.len() == 1 && objects.len() == 1;
        for obj in objects {
            // An object the file does not declare (from elsewhere in its pack) is not
            // this file's to report.
            let Some(source) = sources
                .iter()
                .find(|s| s.name == obj.name)
                .or_else(|| sources.first().filter(|_| single))
            else {
                continue;
            };
            for b in obj.behaviors.values() {
                let nodes = b.instructions.len();
                let at = source
                    .by_id
                    .get(&b.id)
                    .copied()
                    .or_else(|| source.by_label.get(&b.label).copied().flatten())
                    .unwrap_or(source.span);
                if nodes > FRAME_NODE_WARN {
                    self.emit(
                        FRAME_HEADROOM,
                        at,
                        format!(
                            "tree {} \"{}\" compiles to {nodes} nodes; a tree holds at most {}",
                            b.id,
                            b.label,
                            crate::lower::MAX_BHAV_NODES
                        ),
                        Some(
                            "split it: move a self-contained part into its own `#[bhav] fn`"
                                .to_owned(),
                        ),
                        None,
                        None,
                    );
                }
                if b.locals > FRAME_LOCAL_WARN {
                    let frame = self.frames.as_ref().and_then(|f| {
                        f.get(&obj.name)
                            .or_else(|| f.values().next().filter(|_| single))?
                            .iter()
                            .find(|f| f.id == b.id)
                    });
                    let culprits = frame.and_then(local_culprits);
                    self.emit(
                        FRAME_HEADROOM,
                        at,
                        match &culprits {
                            Some(c) => format!(
                                "tree {} \"{}\" uses {} locals: {c}",
                                b.id, b.label, b.locals
                            ),
                            None => format!("tree {} \"{}\" uses {} locals", b.id, b.label, b.locals),
                        },
                        Some("reuse locals across disjoint parts, call a `#[bhav] fn` instead of inlining a plain `fn` many times (each inlined call gets its own locals), or split the tree".to_owned()),
                        None,
                        None,
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Rewrites checked by building them
// ---------------------------------------------------------------------------------------

/// One source edit: bytes `start..end` become `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fix {
    start: usize,
    end: usize,
    text: String,
}

/// A finding that stands only if its rewrite builds the file to the same bytes.
#[derive(Debug, Clone)]
struct Pending {
    diag: LintDiagnostic,
    fixes: Vec<Fix>,
    /// A `use` line the rewrite needs, inserted at the top of the file.
    import: Option<String>,
    /// The statement it is in: two findings of one statement are built apart, so a rewrite
    /// that builds only with another (two cells compared, typed alike only together) does
    /// not stand.
    stmt: Option<Span>,
    /// The body it is in (a count of linted bodies).
    body: usize,
    /// Its statement binds names (`let`, `match`, `for`) whose types its rewrite may change:
    /// it is built apart from every other `untyped_engine_field` finding of its body.
    flows: bool,
    /// It restates a typed spelling in a nicer form (a Sim's dot names, a flag method):
    /// builds go to the other findings first, so these never crowd one out of the budget.
    secondary: bool,
}

/// The most builds one file's pending findings may take.
const TRIAL_BUDGET: usize = 48;

/// The most builds one file's `untyped_engine_field` findings may take to build together.
const JOINT_BUDGET: usize = 16;

/// What a `x.data[N]` root is, for the typed names its cells have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Root {
    /// `self` in a body where it is an object.
    Object,
    /// A name of the running Sim.
    Sim,
    /// `stack_obj`, or a local holding an object handle.
    Handle,
}

/// The pending findings whose rewrite builds `input` to the bytes of `original`. A group of
/// findings is built with all their rewrites; a group that fails is split in halves until
/// each finding stands or falls alone, within [`TRIAL_BUDGET`] builds.
/// The findings of `pending` that stand, and those the budget left untried (they may
/// stand: an `allow` of them is not stale).
fn verified(
    input: &LintInput,
    original: &[crate::compile::CompiledObject],
    pending: Vec<Pending>,
    reported: &[LintDiagnostic],
) -> (Vec<LintDiagnostic>, Vec<LintDiagnostic>) {
    if pending.is_empty() {
        return (Vec::new(), Vec::new());
    }
    // A pack file builds against its pack, under its file name, as `expand_for_lint` does.
    let pack = match (&input.path, &input.pack_dir) {
        (Some(path), Some(pack)) => match crate::v2::loader::read_v2_checked_file(path, Some(pack))
        {
            Ok((_, config)) => {
                let label = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<input.antics>")
                    .to_owned();
                Some((label, config))
            }
            Err(_) => return (Vec::new(), Vec::new()),
        },
        _ => None,
    };
    let builds_same = |source: &str| {
        let lowered = match &pack {
            Some((label, config)) => crate::v2::resolve_v2_source(label, source, config)
                .and_then(|r| crate::v2::lower_resolved_module(&r)),
            None => crate::v2::expand_v2_source_to_objects(
                &input.name,
                source,
                &crate::v2::PackResolverConfig::default(),
            ),
        };
        let Ok(objects) = lowered else {
            return false;
        };
        objects.len() == original.len()
            && objects.into_iter().zip(original).all(|(e, o)| {
                crate::v2::compile_expanded_v2_object(e).is_ok_and(|c| c.iff_bytes == o.iff_bytes)
            })
    };
    let mut budget = TRIAL_BUDGET;
    let mut joint_budget = JOINT_BUDGET;
    let mut stands = Vec::new();
    let mut accepted_groups: Vec<Vec<usize>> = Vec::new();
    // A group builds its findings together, so two that might build only together go in
    // separate groups:
    // - findings of one statement;
    // - findings whose fixes overlap (a `refresh` constant and the call's method form);
    // - two `untyped_engine_field` findings of one body when either is in a statement that
    //   binds names: the binding takes the typed word's type on to other statements.
    // Rewrites of other statements, and of other bodies, do not meet: each group member
    // then stands or falls as it would alone.
    // Secondary findings go in groups of their own, built after every other group (and its
    // halves), so they never take a build another finding needs.
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let field = |p: &Pending| p.diag.rule == UNTYPED_ENGINE_FIELD;
    let order = (0..pending.len())
        .filter(|&i| !pending[i].secondary)
        .chain((0..pending.len()).filter(|&i| pending[i].secondary));
    for i in order {
        let p = &pending[i];
        let clashes = |g: &Vec<usize>| {
            g.iter().any(|&j| {
                let q = &pending[j];
                (q.stmt.is_some() && q.stmt == p.stmt)
                    || (field(p) && field(q) && p.body == q.body && (p.flows || q.flows))
                    || q.fixes
                        .iter()
                        .any(|a| p.fixes.iter().any(|b| a.start < b.end && b.start < a.end))
            })
        };
        match groups
            .iter_mut()
            .find(|g| !clashes(g) && pending[g[0]].secondary == p.secondary)
        {
            Some(g) => g.push(i),
            None => groups.push(vec![i]),
        }
    }
    groups.reverse();
    let mut untried: Vec<usize> = Vec::new();
    while let Some(group) = groups.pop() {
        if budget == 0 {
            untried.extend(group);
            continue;
        }
        budget -= 1;
        let rewritten = apply_fixes(&input.source, group.iter().map(|&i| &pending[i]));
        if rewritten.is_some_and(|s| builds_same(&s)) {
            stands.extend(&group);
            accepted_groups.push(group);
        } else if group.len() > 1 {
            let (a, b) = group.split_at(group.len() / 2);
            groups.push(b.to_vec());
            groups.push(a.to_vec());
        }
    }
    // The lift applies every `untyped_engine_field` suggestion at once, so those must also build
    // together: two typed words of one statement may not compare (`flags2 == buy_flags`).
    // The static table's suggestions (`self.id`, ...) are always applied, as the lift applies
    // them. Each half joins only if it builds with everything kept so far; what the budget does
    // not reach is dropped.
    let statics = lift_fixes(&input.source, reported);
    stands.sort_unstable();
    let (fields, mut kept): (Vec<usize>, Vec<usize>) = stands
        .into_iter()
        .partition(|&i: &usize| pending[i].diag.rule == UNTYPED_ENGINE_FIELD);
    // The other findings join first; the secondary ones (a Sim's dot names) then join only
    // what those kept, so they never crowd one out.
    let (secondary, fields): (Vec<usize>, Vec<usize>) =
        fields.into_iter().partition(|&i| pending[i].secondary);
    let mut joint: Vec<usize> = Vec::new();
    let mut todo = vec![secondary, fields];
    let mut first = true;
    while let Some(part) = todo.pop() {
        if part.is_empty() {
            continue;
        }
        // One passing group already built them all together.
        let tested = first && statics.is_empty() && groups_built_whole(&part, &accepted_groups);
        first = false;
        if tested {
            joint.extend(part);
            continue;
        }
        if joint_budget == 0 {
            untried.extend(part);
            continue;
        }
        joint_budget -= 1;
        let all = joint
            .iter()
            .chain(&part)
            .map(|&i| &pending[i])
            .chain(&statics);
        if apply_fixes(&input.source, all).is_some_and(|s| builds_same(&s)) {
            joint.extend(part);
        } else if part.len() > 1 {
            let (a, b) = part.split_at(part.len() / 2);
            todo.push(b.to_vec());
            todo.push(a.to_vec());
        }
    }
    kept.extend(joint);
    kept.sort_unstable();
    untried.sort_unstable();
    let diags = |list: Vec<usize>| list.into_iter().map(|i| pending[i].diag.clone()).collect();
    (diags(kept), diags(untried))
}

/// Per line, each string literal's column and how many more bytes than characters it
/// holds (the lexer counts a string's characters, all else by bytes).
fn wide_strings(tokens: &[crate::v2::lexer::Token]) -> BTreeMap<usize, Vec<(usize, usize)>> {
    let mut out: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
    for t in tokens {
        if let TokenKind::Str(s) = &t.kind {
            let extra = s.len() - s.chars().count();
            if extra > 0 {
                out.entry(t.span.line)
                    .or_default()
                    .push((t.span.column, extra));
            }
        }
    }
    out
}

/// The `untyped_engine_field` suggestions of `reported` as the lift applies them: from the
/// column (read as a byte offset) to the first `]` of the line.
fn lift_fixes(source: &str, reported: &[LintDiagnostic]) -> Vec<Pending> {
    let starts: Vec<usize> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    reported
        .iter()
        .filter(|d| d.rule == UNTYPED_ENGINE_FIELD)
        .filter_map(|d| {
            let line = *starts.get(d.line.checked_sub(1)?)?;
            let start = line + d.column.checked_sub(1)?;
            let rest = source.get(start..)?;
            let rest = &rest[..rest.find('\n').unwrap_or(rest.len())];
            let end = start + rest.find(']')? + 1;
            Some(Pending {
                diag: d.clone(),
                fixes: vec![Fix {
                    start,
                    end,
                    text: d.suggestion.clone()?,
                }],
                import: None,
                stmt: None,
                body: 0,
                flows: false,
                secondary: false,
            })
        })
        .collect()
}

/// Whether `part` lies inside one group that built whole.
fn groups_built_whole(part: &[usize], accepted: &[Vec<usize>]) -> bool {
    accepted.iter().any(|g| part.iter().all(|i| g.contains(i)))
}

/// `source` with the fixes and imports of `pending` applied, or `None` when two fixes overlap.
fn apply_fixes<'p>(source: &str, pending: impl Iterator<Item = &'p Pending>) -> Option<String> {
    let mut fixes: Vec<&Fix> = Vec::new();
    let mut imports = BTreeSet::new();
    for p in pending {
        fixes.extend(&p.fixes);
        imports.extend(p.import.as_deref());
    }
    fixes.sort_by_key(|f| (f.start, f.end));
    fixes.dedup();
    if fixes.windows(2).any(|w| w[1].start < w[0].end) {
        return None;
    }
    let mut out = source.to_owned();
    for f in fixes.iter().rev() {
        source.get(f.start..f.end)?;
        out.replace_range(f.start..f.end, &f.text);
    }
    // A `use` goes before every item; the lines it adds change no built byte.
    let mut head: String = imports.into_iter().map(|i| format!("{i}\n")).collect();
    head.push_str(&out);
    Some(head)
}

/// Where an expression's text starts (a call's, field's or index's span is its `(`, `.` or `[`).
fn expr_start(e: &V2Expr) -> Span {
    match e {
        V2Expr::Field { base, .. } | V2Expr::Index { base, .. } => expr_start(base),
        V2Expr::Call { callee, .. } => expr_start(callee),
        V2Expr::MethodCall { receiver, .. } => expr_start(receiver),
        V2Expr::Try { inner, .. }
        | V2Expr::Unary {
            op: V2UnaryOp::AsI16,
            inner,
            ..
        } => expr_start(inner),
        V2Expr::Binary { lhs, .. } => expr_start(lhs),
        other => other.span(),
    }
}

/// One past the bracket closing the group that opens at `open`, or `None` when the group
/// holds a comment (a rewrite would drop it) or does not close.
fn group_end(src: &str, open: usize) -> Option<usize> {
    let b = src.as_bytes();
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

/// The trimmed top-level comma-separated parts of `src[from..to]` (a call's arguments).
fn split_args(src: &str, from: usize, to: usize) -> Option<Vec<&str>> {
    let b = src.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = from;
    let mut i = from;
    while i < to {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
            b'"' => {
                i += 1;
                while i < to && b[i] != b'"' {
                    i += usize::from(b[i] == b'\\') + 1;
                }
            }
            b',' if depth == 0 => {
                parts.push(src.get(start..i)?.trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    let last = src.get(start..to)?.trim();
    if !last.is_empty() {
        parts.push(last);
    }
    Some(parts)
}

/// `at` past any whitespace in `src`.
fn skip_ws(src: &str, at: usize) -> usize {
    src.get(at..)
        .map_or(at, |rest| at + rest.len() - rest.trim_start().len())
}

/// Whether some `else` in `toks` names something (`let else = 0;`) instead of being the
/// keyword, which is always followed by `{` or `if`. `collapsible_else_if` is off in such a
/// file: the collapse moves the inner chain's end next to whatever follows the block, and a
/// statement `else = 3;` there would be read as the chain's `else`.
fn else_named(toks: &[crate::v2::lexer::Token]) -> bool {
    toks.windows(2).any(|w| {
        is_kw(Some(&w[0]), "else") && w[1].kind != TokenKind::LBrace && !is_kw(Some(&w[1]), "if")
    })
}

/// The enum and bitflags types `members` declare.
fn member_type_names(members: &[ObjectMember]) -> BTreeSet<String> {
    members
        .iter()
        .filter_map(|m| match m {
            ObjectMember::Bitflags(b) => Some(b.name.clone()),
            ObjectMember::Enum(e) => Some(e.name.clone()),
            _ => None,
        })
        .collect()
}

/// The two-segment paths `A::b` in a file that may read BCON entry `b` of a table named
/// `A`, as `(body, A, b)`. `body` is `object:<file>:Name` or `template:Name` (a BCON table
/// is in scope only in its object, and its template's body), or `module` for a path outside
/// any body: a module-level helper is inlined into whatever object calls it.
/// Lowering gives the path to an enum or bitflags type `A` in scope first. Not counted:
/// - a segment of a longer path (`m::A::b`);
/// - a called path (`globals::f()`, `((globals::f))()`);
/// - a path whose `A` names such a type: one the file's module sees (its own items,
///   imports, globs and aliases, from the resolver), a `std` prelude type, or one the body
///   declares or (for an object) its `from` template declares. An object's own types also
///   win in its template's body; the BCON check applies that, since templates are shared.
///   When the module does not resolve, every name the file imports is taken for a type.
///
/// Raw blocks are one token: `.air` reads are counted from their text instead.
fn bcon_path_reads(
    input: &LintInput,
    file: &V2File,
    tokens: &[crate::v2::lexer::Token],
) -> BTreeSet<(String, String, String)> {
    let kind = |k: usize| tokens.get(k).map(|t| &t.kind);
    // Candidate paths first: most files have none, and they need no resolving.
    let mut candidates = Vec::new();
    for i in 2..tokens.len() {
        let (TokenKind::Ident(table), Some(TokenKind::ColonColon), Some(TokenKind::Ident(entry))) =
            (&tokens[i - 2].kind, kind(i - 1), kind(i))
        else {
            continue;
        };
        let longer = (i >= 3 && kind(i - 3) == Some(&TokenKind::ColonColon))
            || kind(i + 1) == Some(&TokenKind::ColonColon);
        // `(( A::b ))(..)` calls it too.
        let mut groups = 0;
        while i >= 3 + groups
            && kind(i - 3 - groups) == Some(&TokenKind::LParen)
            && kind(i + 1 + groups) == Some(&TokenKind::RParen)
        {
            groups += 1;
        }
        let called = kind(i + 1 + groups) == Some(&TokenKind::LParen);
        if !longer && !called {
            candidates.push((i, table, entry));
        }
    }
    if candidates.is_empty() {
        return BTreeSet::new();
    }
    let resolved = match (&input.path, &input.pack_dir) {
        (Some(path), Some(pack)) => crate::v2::resolve_v2_file(path, Some(pack)),
        _ => crate::v2::resolve_v2_source(
            &input.name,
            &input.source,
            &crate::v2::PackResolverConfig::default(),
        ),
    }
    .ok();
    let mut types = BTreeSet::new();
    for item in &file.items {
        match item {
            V2Item::Bitflags(b) => {
                types.insert(b.name.clone());
            }
            V2Item::Enum(e) => {
                types.insert(e.name.clone());
            }
            _ => {}
        }
    }
    match &resolved {
        Some(m) => {
            types.extend(m.enums.keys().cloned());
            types.extend(m.bitflags.keys().cloned());
        }
        None => {
            for u in &file.uses {
                for item in &u.items {
                    if item.name != "*" {
                        types.insert(item.alias.clone().unwrap_or_else(|| item.name.clone()));
                    }
                }
            }
        }
    }
    let template_members = |name: &str| -> Option<&[ObjectMember]> {
        file.items
            .iter()
            .find_map(|i| match i {
                V2Item::Template(t) if t.name == name => Some(t.members.as_slice()),
                _ => None,
            })
            .or_else(|| {
                resolved
                    .as_ref()
                    .and_then(|m| m.templates.get(name))
                    .map(|(_, t)| t.members.as_slice())
            })
    };
    // Each object or template body's token range, its key, and the types in scope in it.
    let mut bodies: Vec<(usize, usize, String, BTreeSet<String>)> = Vec::new();
    for item in &file.items {
        let (span, key, members, from) = match item {
            V2Item::Object(o) => (
                o.span,
                format!("object:{}:{}", input.name, o.name),
                &o.members,
                o.from_template.as_ref().map(|(t, _)| t),
            ),
            V2Item::Template(t) => (t.span, format!("template:{}", t.name), &t.members, None),
            _ => continue,
        };
        let at = (span.line, span.column);
        let Some(start) = tokens
            .iter()
            .position(|t| (t.span.line, t.span.column) >= at)
            .and_then(|k| {
                tokens[k..]
                    .iter()
                    .position(|t| t.kind == TokenKind::LBrace)
                    .map(|o| k + o)
            })
        else {
            continue;
        };
        let mut depth = 0usize;
        let mut end = tokens.len();
        for (k, t) in tokens.iter().enumerate().skip(start) {
            match t.kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        end = k;
                        break;
                    }
                }
                _ => {}
            }
        }
        let mut own = member_type_names(members);
        if let Some(members) = from.and_then(|t| template_members(t)) {
            own.extend(member_type_names(members));
        }
        bodies.push((start, end, key, own));
    }
    let mut out = BTreeSet::new();
    for (i, table, entry) in candidates {
        // Outside any body (a module-level helper, inlined into whatever object calls it):
        // it counts for every object.
        let (key, own) = match bodies
            .iter()
            .find(|(start, end, ..)| (*start..*end).contains(&i))
        {
            Some((_, _, key, own)) => (key.clone(), Some(own)),
            None => ("module".to_owned(), None),
        };
        let typed = types.contains(table)
            || crate::v2::ast::is_std_prelude_type_name(table)
            || own.is_some_and(|own| own.contains(table));
        if !typed {
            out.insert((key, table.clone(), entry.clone()));
        }
    }
    out
}

/// Whether `t` is the keyword (identifier token) `kw`.
fn is_kw(t: Option<&crate::v2::lexer::Token>, kw: &str) -> bool {
    matches!(t.map(|t| &t.kind), Some(TokenKind::Ident(k)) if k == kw)
}

/// The index of the `}` that ends the `if` at `toks[at]` with its `else if` / `else`
/// chain, or `None` when the tokens are not plainly such a chain: a condition holding
/// braces or an `if` / `match` (whose braces would be misread), or an unclosed group.
fn if_chain_end(toks: &[crate::v2::lexer::Token], mut at: usize) -> Option<usize> {
    // The `}` matching the `{` at `open`.
    let group = |open: usize| {
        let mut depth = 0usize;
        for (i, t) in toks.iter().enumerate().skip(open) {
            match t.kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                TokenKind::Eof => return None,
                _ => {}
            }
        }
        None
    };
    loop {
        // The condition: up to the first `{` outside brackets.
        let mut depth = 0usize;
        let mut open = at + 1;
        loop {
            match &toks.get(open)?.kind {
                TokenKind::LParen | TokenKind::LBracket => depth += 1,
                TokenKind::RParen | TokenKind::RBracket => depth = depth.checked_sub(1)?,
                TokenKind::LBrace if depth == 0 => break,
                TokenKind::LBrace | TokenKind::RBrace | TokenKind::Eof => return None,
                TokenKind::Ident(k) if k == "if" || k == "match" => return None,
                _ => {}
            }
            open += 1;
        }
        let end = group(open)?;
        if !is_kw(toks.get(end + 1), "else") {
            return Some(end);
        }
        match toks.get(end + 2)?.kind {
            TokenKind::LBrace => return group(end + 2),
            TokenKind::Ident(ref k) if k == "if" => at = end + 2,
            _ => return None,
        }
    }
}

/// Whether `text` holds a `//` or `/*` comment outside its strings.
fn has_comment(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += usize::from(b[i] == b'\\') + 1;
                }
            }
            b'/' if matches!(b.get(i + 1), Some(b'/' | b'*')) => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// The types `std::flags` and `std::engine` declare: (module, type) -> each value or bit
/// and its name.
type StdEngineTypes = BTreeMap<(&'static str, String), Vec<(i64, String)>>;

/// The `pub fn`s `use <path>::*;` brings in when `path` is a built-in `std::<module>` (the
/// loader's embedded `std` sources: `std::math::min`, `std::motives::restore_motive_loop`,
/// `std::budget::try_charge`, ...); empty for any other path.
fn std_glob_fns(path: &[String]) -> &'static [String] {
    static FNS: std::sync::OnceLock<BTreeMap<&'static str, Vec<String>>> =
        std::sync::OnceLock::new();
    let fns = FNS.get_or_init(|| {
        let mut out = BTreeMap::new();
        for module in crate::v2::loader::STD_MODULES {
            let Some(src) = crate::v2::loader::std_module_source(&[module.to_owned()]) else {
                continue;
            };
            let Ok(file) = crate::v2::parser::parse_v2_std_file(src) else {
                continue;
            };
            let names = file
                .items
                .into_iter()
                .filter_map(|item| match item {
                    V2Item::Fn(f) if f.is_pub => Some(f.name),
                    _ => None,
                })
                .collect();
            out.insert(module, names);
        }
        out
    });
    match path {
        [std, module] if std == "std" => fns.get(module.as_str()).map_or(&[], Vec::as_slice),
        _ => &[],
    }
}

/// [`StdEngineTypes`], read from the `std` sources.
fn std_engine_types() -> &'static StdEngineTypes {
    static TYPES: std::sync::OnceLock<StdEngineTypes> = std::sync::OnceLock::new();
    TYPES.get_or_init(|| {
        let mut out = BTreeMap::new();
        for (module, src) in [
            ("flags", include_str!("std/flags.antics")),
            ("engine", include_str!("std/engine.antics")),
        ] {
            let Ok(file) = crate::v2::parser::parse_v2_std_file(src) else {
                continue;
            };
            for item in file.items {
                let (name, values) = match item {
                    V2Item::Bitflags(b) => (
                        b.name,
                        b.flags
                            .into_iter()
                            .map(|f| (i64::from(f.bit), f.name))
                            .collect(),
                    ),
                    V2Item::Enum(e) => (
                        e.name,
                        e.variants
                            .into_iter()
                            .map(|v| (i64::from(v.value), v.name))
                            .collect(),
                    ),
                    _ => continue,
                };
                out.insert((module, name), values);
            }
        }
        out
    })
}

/// The name `std::<module>::<ty>` gives `value` (a bit number for a bitflags type).
fn std_engine_name(module: &'static str, ty: &str, value: i64) -> Option<&'static str> {
    std_engine_types()
        .get(&(module, ty.to_owned()))?
        .iter()
        .find(|(v, _)| *v == value)
        .map(|(_, n)| n.as_str())
}

/// The `std::flags` type of an object's data word, by index.
fn data_flags(index: i64) -> Option<&'static str> {
    Some(match index {
        4 => "Height",
        5 => "WallAdjacency",
        8 => "ObjectFlags",
        13 => "WallPlacement",
        40 => "ObjectFlags2",
        42 => "Placement",
        43 => "BuyFlags",
        _ => return None,
    })
}

/// The `std::flags` type of an object's named flag word.
fn field_flags(field: &str) -> Option<&'static str> {
    Some(match field {
        "allowed_heights" => "Height",
        "wall_adjacency" => "WallAdjacency",
        "flags" => "ObjectFlags",
        "wall_placement" => "WallPlacement",
        "flags2" => "ObjectFlags2",
        "placement" => "Placement",
        "buy_flags" => "BuyFlags",
        _ => return None,
    })
}

impl FileLinter<'_> {
    /// The byte offset of `at` in the source.
    fn offset(&self, at: Span) -> Option<usize> {
        let line = *self.line_starts.get(at.line.checked_sub(1)?)?;
        let wide: usize = self.wide.get(&at.line).map_or(0, |strings| {
            strings
                .iter()
                .filter(|(column, _)| *column < at.column)
                .map(|(_, extra)| extra)
                .sum()
        });
        let at = line + at.column.checked_sub(1)? + wide;
        self.source.is_char_boundary(at).then_some(at)
    }

    /// Whether the column of `at` is its byte offset in its line, as the lift reads it.
    fn column_is_byte(&self, at: Span) -> bool {
        self.wide
            .get(&at.line)
            .is_none_or(|strings| strings.iter().all(|(column, _)| *column >= at.column))
    }

    /// Queues a finding that stands only if `fixes` (with `import`) build the file to the
    /// same bytes; its suggestion is `suggestion`.
    #[allow(clippy::too_many_arguments)]
    fn propose(
        &mut self,
        rule_name: &'static str,
        at: Span,
        message: String,
        help: String,
        suggestion: String,
        fixes: Vec<Fix>,
        import: Option<String>,
    ) {
        // A rewrite drops what it replaces: a comment there is lost.
        if fixes
            .iter()
            .any(|f| self.source.get(f.start..f.end).is_none_or(has_comment))
        {
            return;
        }
        if self.unbuilt {
            return;
        }
        let severity = rule(rule_name).map_or(Severity::Warning, |r| r.severity);
        self.pending.push(Pending {
            diag: LintDiagnostic {
                rule: rule_name,
                severity,
                file: self.file.to_owned(),
                line: at.line,
                column: at.column,
                message,
                help: Some(help),
                suggestion: Some(suggestion),
                note: None,
            },
            fixes,
            import,
            stmt: self.stmt,
            body: self.body,
            flows: self.stmt_flows,
            secondary: self.secondary,
        });
    }

    /// `untyped_engine_field` for a cell the static tables do not name (a `std` engine word,
    /// a Sim's new person words, `stack_obj` and handles): proposed, and kept when the
    /// typed spelling builds the same bytes.
    #[allow(clippy::too_many_arguments)]
    fn trial_engine_field(
        &mut self,
        x: &V2Expr,
        parent: Option<&V2Expr>,
        at: Span,
        root: &str,
        kind: Root,
        cell: &str,
        n: i64,
    ) {
        let tables: &[&[TypedField]] = match kind {
            Root::Object => &[OBJECT_WORDS],
            Root::Sim => &[SIM_WORDS, SIM_GROUP_WORDS, OBJECT_WORDS],
            Root::Handle => &[HANDLE_WORDS, SIM_GROUP_WORDS, OBJECT_WORDS],
        };
        let Some(&field) = tables
            .iter()
            .flat_map(|t| t.iter())
            .find(|f| f.cell == cell && f.index == n)
        else {
            return;
        };
        // A state of the same name wins over the engine word.
        if kind != Root::Sim && self.states.contains_key(field.name) {
            return;
        }
        // An engine enum (`Direction`, `PersonKind`) is no `i16`: it stands in for the cell
        // only cast back (`self.facing as i16`).
        if ENUM_WORDS.contains(&field.name)
            && !matches!(
                parent,
                Some(V2Expr::Unary {
                    op: V2UnaryOp::AsI16,
                    ..
                })
            )
        {
            return;
        }
        if self.written.contains(&addr(x)) && !field.writable {
            return;
        }
        if field.object_id {
            let compared = matches!(
                parent,
                Some(V2Expr::Binary {
                    op: V2BinOp::Eq | V2BinOp::Ne,
                    ..
                })
            );
            if !compared && self.assigned != Some(addr(x)) {
                return;
            }
        }
        let V2Expr::Index { span: open, .. } = x else {
            return;
        };
        let (Some(start), Some(open)) = (self.offset(at), self.offset(*open)) else {
            return;
        };
        let Some(end) = group_end(self.source, open) else {
            return;
        };
        // The lift replaces from the column (as a byte offset) to the first `]`: the cell
        // must be that text, on one line.
        let raw = self.source[start..end].to_owned();
        if !self.column_is_byte(at)
            || raw.contains('\n')
            || raw.find(']') != Some(raw.len() - 1)
            || !cell_text_is(&raw, root, cell)
        {
            return;
        }
        let typed = format!("{root}.{}", field.name);
        // A Sim's dot names (`skill.cooking`) are secondary to the typed words.
        self.secondary = field.name.contains('.');
        self.propose(
            UNTYPED_ENGINE_FIELD,
            at,
            format!("`{raw}` is `{typed}`"),
            format!("write `{typed}`; it names the same cell"),
            typed.clone(),
            vec![Fix {
                start,
                end,
                text: typed,
            }],
            None,
        );
        self.secondary = false;
    }

    /// How this file spells `std::<module>::<ty>`, and the `use` line it needs first:
    /// `None` when another item of the file or another import is named `ty`.
    fn engine_type_scope(&self, module: &str, ty: &str) -> Option<(String, Option<String>)> {
        // Prelude types: every module sees them.
        if matches!(ty, "Height" | "Placement") {
            return Some((ty.to_owned(), None));
        }
        if self.item_names.contains(ty) {
            return None;
        }
        let mut glob = false;
        for u in self.uses {
            let std_module =
                u.module_path.len() == 2 && u.module_path[0] == "std" && u.module_path[1] == module;
            for item in &u.items {
                let local = item.alias.as_deref().unwrap_or(&item.name);
                if std_module && item.name == "*" {
                    glob = true;
                } else if std_module && item.name == ty {
                    return Some((local.to_owned(), None));
                } else if local == ty {
                    return None;
                }
            }
        }
        if glob {
            return Some((ty.to_owned(), None));
        }
        Some((ty.to_owned(), Some(format!("use std::{module}::{ty};"))))
    }

    /// `untyped_engine_constant`: the literal `lit` where a `std::<module>::<ty>` value
    /// goes, when the type names it.
    fn engine_constant(&mut self, lit: &V2Expr, module: &'static str, ty: &str) {
        let V2Expr::Int(at, v) = lit else {
            return;
        };
        let Some(text) = self.offset(*at).and_then(|o| self.source.get(o..)) else {
            return;
        };
        // A literal, not a constant.
        if !text.starts_with(|c: char| c.is_ascii_digit()) {
            return;
        }
        let len = text
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(text.len());
        let raw = text[..len].to_owned();
        let Some(variant) = std_engine_name(module, ty, *v) else {
            return;
        };
        let Some((spelled, import)) = self.engine_type_scope(module, ty) else {
            return;
        };
        let Some(start) = self.offset(*at) else {
            return;
        };
        let typed = format!("{spelled}::{variant}");
        let what = if module == "flags" {
            format!("bit {v} of `{ty}`")
        } else {
            format!("`{ty}` value {v}")
        };
        let help = match &import {
            Some(line) => format!("write `{typed}` and add `{line}`; it builds the same nodes"),
            None => format!("write `{typed}`; it builds the same nodes"),
        };
        self.propose(
            UNTYPED_ENGINE_CONSTANT,
            *at,
            format!("`{raw}` is {what}, `{typed}`"),
            help,
            typed.clone(),
            vec![Fix {
                start,
                end: start + len,
                text: typed,
            }],
            import,
        );
    }

    /// The `std::flags` type of the engine word `place` (`global[20]`, an object's flag
    /// words by name or number).
    fn flag_word_type(&self, place: &V2Expr) -> Option<&'static str> {
        match place {
            V2Expr::Index { base, index, .. } => {
                let V2Expr::Int(_, n) = index.as_ref() else {
                    return None;
                };
                match base.as_ref() {
                    V2Expr::Ident(_, g) if g == "global" => (*n == 20).then_some("Expansions"),
                    V2Expr::Field {
                        base: root, field, ..
                    } if field == "data" && matches!(root.as_ref(), V2Expr::Ident(..)) => {
                        data_flags(*n)
                    }
                    _ => None,
                }
            }
            V2Expr::Field {
                base: root, field, ..
            } => {
                let V2Expr::Ident(_, root) = root.as_ref() else {
                    return None;
                };
                // A state of the same name wins over the engine word, and a typed handle's
                // object may have one this file does not show.
                if (root == "self" || root == "me") && self.states.contains_key(field) {
                    return None;
                }
                if self.bound.contains(root) && root != "stack_obj" {
                    return None;
                }
                field_flags(field)
            }
            _ => None,
        }
    }

    /// `untyped_engine_constant` on a flag operation: `place flag_set N` / `flag_clear N` /
    /// `flag_test N`.
    fn flag_constant(&mut self, place: &V2Expr, bit: &V2Expr) {
        if let Some(ty) = self.flag_word_type(place) {
            self.engine_constant(bit, "flags", ty);
        }
    }

    /// Whether `name` names `std::flags::<ty>` in this file: an import of it (`ObjectFlags`,
    /// `Of`), or `ty` itself from a `std::flags` glob or the prelude (`Placement`,
    /// `Height`). Not when a file item or another import takes the name, nor `ty` itself
    /// by a glob or the prelude when a glob of another module may bind it.
    fn std_flag_spelling(&self, ty: &str, name: &str) -> bool {
        if self.item_names.contains(name) {
            return false;
        }
        let std_flags = |u: &crate::v2::ast::UseDecl| u.module_path == ["std", "flags"];
        let mut imported = false;
        let mut glob = matches!(ty, "Height" | "Placement");
        let mut foreign_glob = false;
        for u in self.uses {
            for item in &u.items {
                let local = item.alias.as_deref().unwrap_or(&item.name);
                if item.name == "*" {
                    if std_flags(u) {
                        glob = true;
                    } else if u.module_path.first().is_none_or(|m| m != "std") {
                        foreign_glob = true;
                    }
                } else if local == name {
                    if std_flags(u) && item.name == ty {
                        imported = true;
                    } else {
                        return false;
                    }
                }
            }
        }
        // An explicit import wins over a glob; another module's glob may bind `ty` where
        // only a glob or the prelude brings `std`'s.
        imported || (glob && name == ty && !foreign_glob)
    }

    /// `prefer_method_syntax` on a flag operation: `place flag_set T::F` is
    /// `place.set(T::F)`, `flag_clear` is `.clear(..)`, `flag_test` is `.contains(..)`.
    ///
    /// Only in an object's body, for a named object flag word (`self.flags`,
    /// `stack_obj.wall_placement`; not `self.data[8]`, which lowering types `i16`) or
    /// `world.expansions` (not `global[20]` written out), whose `std::flags` type `T` the
    /// operand names: a raw bit
    /// (`flag_set 8`) or another constant has no method form. `op` is the operator's
    /// spelling, `method` the method's.
    fn flag_method(&mut self, place: &V2Expr, bit: &V2Expr, op: &str, method: &str) {
        // In an object's body, whose members (and its template's) are known: a template's
        // objects, or a module helper's callers, may declare a state or type of the name.
        let Some(scope) = self.body_scope.as_ref() else {
            return;
        };
        let src = self.source;
        // The receiver as written, where it starts, and its `std::flags` type.
        let (start, receiver, ty) = match place {
            V2Expr::Field {
                base: root, field, ..
            } => {
                let V2Expr::Ident(at, root) = root.as_ref() else {
                    return;
                };
                // A state of the same name wins over the engine word (`me` is the Sim in a
                // Sim's body), and a typed handle's object may have one this file does not
                // show.
                if (root == "self" || (root == "me" && !self.sim_body))
                    && scope.states.contains(field)
                {
                    return;
                }
                if self.bound.contains(root) && root != "stack_obj" {
                    return;
                }
                let Some(ty) = field_flags(field) else {
                    return;
                };
                let Some(start) = self.offset(*at) else {
                    return;
                };
                (start, format!("{root}.{field}"), ty)
            }
            // `world.expansions`, which the parser reads as `global[20]` with the span of
            // its dot. A `global[20]` written out has no `world.` before its span.
            V2Expr::Index { span, base, index }
                if matches!(base.as_ref(), V2Expr::Ident(_, g) if g == "global")
                    && matches!(index.as_ref(), V2Expr::Int(_, 20)) =>
            {
                let receiver = "world.expansions";
                let Some(dot) = self.offset(*span) else {
                    return;
                };
                let Some(start) = dot.checked_sub("world".len()) else {
                    return;
                };
                if src.get(start..start + receiver.len()) != Some(receiver)
                    || src[..start]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    return;
                }
                (start, receiver.to_owned(), "Expansions")
            }
            _ => return,
        };
        let V2Expr::Path(bit_at, segs) = bit else {
            return;
        };
        let [ty_name, _] = segs.as_slice() else {
            return;
        };
        if !self.std_flag_spelling(ty, ty_name) || scope.types.contains(ty_name) {
            return;
        }
        let at = place.span();
        let Some(bit_start) = self.offset(*bit_at) else {
            return;
        };
        let path = segs.join("::");
        let word_ends = |at: usize| {
            src[at..]
                .chars()
                .next()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        };
        // The receiver, the operator and the operand, as written, with nothing else.
        let receiver_end = start + receiver.len();
        let end = bit_start + path.len();
        if bit_start < receiver_end
            || src.get(start..receiver_end) != Some(receiver.as_str())
            || src.get(bit_start..end) != Some(path.as_str())
            || !word_ends(receiver_end)
            || !word_ends(end)
            || src[receiver_end..bit_start].trim() != op
        {
            return;
        }
        let text = format!("{receiver}.{method}({path})");
        self.secondary = true;
        self.propose(
            PREFER_METHOD_SYNTAX,
            at,
            format!("`{op}` on a typed flag word has a method form"),
            format!("write `{text}`; it builds the same nodes"),
            text.clone(),
            vec![Fix { start, end, text }],
            None,
        );
        self.secondary = false;
    }

    /// `untyped_engine_constant` on a builtin's arguments: the `category:` of the budget
    /// calls, and `refresh`'s target and mode.
    fn call_constants(&mut self, x: &V2Expr) {
        let (name, args, method) = match x {
            V2Expr::Call { callee, args, .. } => match callee.as_ref() {
                V2Expr::Ident(_, name) if !self.user_fn(name) => (name.as_str(), args, false),
                _ => return,
            },
            V2Expr::MethodCall { method, args, .. } => (method.as_str(), args, true),
            _ => return,
        };
        let named = |key: &str| args.iter().find(|a| a.name.as_deref() == Some(key));
        let positional = |k: usize| args.iter().filter(|a| a.name.is_none()).nth(k);
        match name {
            "can_afford" | "debit_funds" | "credit_funds" if !method || name != "can_afford" => {
                if let Some(a) = named("category") {
                    self.engine_constant(&a.value, "engine", "BudgetCategory");
                }
            }
            "refresh" if method => {
                if let Some(a) = named("mode").or_else(|| positional(0)) {
                    self.engine_constant(&a.value, "engine", "RefreshMode");
                }
            }
            "refresh" => {
                if let Some(a) = named("target").or_else(|| positional(0)) {
                    self.engine_constant(&a.value, "engine", "RefreshTarget");
                }
                let mode =
                    named("mode").or_else(|| positional(usize::from(named("target").is_none())));
                if let Some(a) = mode {
                    self.engine_constant(&a.value, "engine", "RefreshMode");
                }
            }
            _ => {}
        }
    }

    /// The name of `callee` when it surely returns `Result`: a tree or `extern` function of
    /// the file set, or a `std::globals` tree the file imports.
    fn result_callee(&self, callee: &V2Expr) -> Option<String> {
        if let Some((name, kind)) = self.symbols.call_kind(callee) {
            return (kind == RetKind::Result).then_some(name);
        }
        let V2Expr::Path(_, segs) = callee else {
            return None;
        };
        let [module, name] = segs.as_slice() else {
            return None;
        };
        if module != "globals"
            || !self.std_globals
            || self.symbols.returns.contains_key(&segs.join("::"))
        {
            return None;
        }
        std_globals()
            .iter()
            .find(|f| &f.name == name)
            .filter(|f| ret_kind(&f.ret) == RetKind::Result)
            .map(|_| segs.join("::"))
    }

    /// `prefer_question_mark`: `if !f(..) { panic!(); }` or
    /// `if f(..) {} else { panic!(); }` on a call that returns `Result`.
    fn prefer_question_mark(&mut self, s: &V2Stmt) {
        let V2Stmt::If {
            span,
            cond,
            then_body,
            else_body,
        } = s
        else {
            return;
        };
        let error = |b: &[V2Stmt]| matches!(b, [V2Stmt::ReturnError { panic: true, .. }]);
        let (call, else_form) = match cond {
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } if error(then_body) && else_body.is_empty() => (inner.as_ref(), false),
            c if then_body.is_empty() && error(else_body) => (c, true),
            _ => return,
        };
        let V2Expr::Call {
            span: open, callee, ..
        } = call
        else {
            return;
        };
        let Some(name) = self.result_callee(callee) else {
            return;
        };
        let src = self.source;
        let (Some(start), Some(call_start), Some(open)) = (
            self.offset(*span),
            self.offset(expr_start(call)),
            self.offset(*open),
        ) else {
            return;
        };
        // Not the `if` of an `else if`: `else f()?;` is no statement.
        if !src[start..].starts_with("if") || src[..start].trim_end().ends_with("else") {
            return;
        }
        let Some(call_end) = group_end(src, open) else {
            return;
        };
        let block = |at: usize| {
            let at = skip_ws(src, at);
            src[at..]
                .starts_with('{')
                .then(|| group_end(src, at))
                .flatten()
        };
        let Some(mut end) = block(call_end) else {
            return;
        };
        if else_form {
            let at = skip_ws(src, end);
            if !src[at..].starts_with("else") {
                return;
            }
            let Some(e) = block(at + 4) else {
                return;
            };
            end = e;
        }
        if has_comment(&src[start..end]) {
            return;
        }
        let text = format!("{}?;", &src[call_start..call_end]);
        self.propose(
            PREFER_QUESTION_MARK,
            *span,
            format!("this `if` only returns the error of `{name}`"),
            format!("write `{text}`; `?` returns the error the same way"),
            text.clone(),
            vec![Fix { start, end, text }],
            None,
        );
    }

    /// `prefer_method_syntax`: a builtin call statement whose target has a method form:
    /// `play_sound(s, x)` is `x.play_sound(s)`, `refresh(target: 0, mode: m)` is
    /// `self.refresh(m)`.
    fn prefer_method_syntax(&mut self, expr: &V2Expr) {
        let V2Expr::Call {
            span: open,
            callee,
            type_args,
            args,
        } = expr
        else {
            return;
        };
        let V2Expr::Ident(name_at, name) = callee.as_ref() else {
            return;
        };
        if !type_args.is_empty() || self.user_fn(name) {
            return;
        }
        let src = self.source;
        let (Some(start), Some(open)) = (self.offset(*name_at), self.offset(*open)) else {
            return;
        };
        let Some(end) = group_end(src, open) else {
            return;
        };
        let Some(parts) = split_args(src, open + 1, end - 1) else {
            return;
        };
        if parts.len() != args.len() {
            return;
        }
        let named = |key: &str| args.iter().position(|a| a.name.as_deref() == Some(key));
        let nth_positional = |k: usize| {
            args.iter()
                .enumerate()
                .filter(|(_, a)| a.name.is_none())
                .nth(k)
                .map(|(i, _)| i)
        };
        let ident = |i: usize| match &args[i].value {
            V2Expr::Ident(_, n) => Some(n.clone()),
            _ => None,
        };
        // The receiver, the method, and the argument texts it keeps.
        let (receiver, method, kept): (String, &str, Vec<String>) = match name.as_str() {
            "play_sound" => {
                let Some(t) = named("target").or_else(|| nth_positional(1)) else {
                    return;
                };
                let Some(receiver) = ident(t) else {
                    return;
                };
                let kept = (0..args.len())
                    .filter(|&i| i != t)
                    .map(|i| parts[i].to_owned());
                (receiver, "play_sound", kept.collect())
            }
            "stop_all_sounds" => {
                let Some(t) = named("target").or_else(|| nth_positional(0)) else {
                    return;
                };
                if args.len() != 1 {
                    return;
                }
                let Some(receiver) = ident(t) else {
                    return;
                };
                (receiver, "stop_all_sounds", Vec::new())
            }
            "refresh" => {
                let Some(t) = named("target").or_else(|| nth_positional(0)) else {
                    return;
                };
                let m = named("mode")
                    .or_else(|| nth_positional(usize::from(named("target").is_none())));
                let Some(m) = m.filter(|_| args.len() == 2) else {
                    return;
                };
                let target = match &args[t].value {
                    V2Expr::Path(_, segs) if segs.len() == 2 && segs[0] == "RefreshTarget" => {
                        match segs[1].as_str() {
                            "Me" => Some(true),
                            "StackObject" => Some(false),
                            _ => return,
                        }
                    }
                    V2Expr::Int(_, 0) => Some(true),
                    V2Expr::Int(_, 1) => Some(false),
                    V2Expr::Ident(..) => None,
                    _ => return,
                };
                let receiver = match target {
                    None => ident(t).unwrap_or_default(),
                    Some(false) => "stack_obj".to_owned(),
                    // `me` of the tree: the object in its own tree, the Sim in a Sim's
                    // body, else (an inline helper runs in its caller's frame) `me`.
                    Some(true) if self.object_me => "self".to_owned(),
                    Some(true) => match self.sims.first() {
                        Some(sim) if self.sim_body => sim.clone(),
                        _ => "me".to_owned(),
                    },
                };
                let mode_text = parts[m]
                    .strip_prefix("mode:")
                    .map_or(parts[m], str::trim_start);
                (receiver, "refresh", vec![mode_text.to_owned()])
            }
            "push_interaction" => {
                let Some(t) = named("object") else {
                    return;
                };
                if args.iter().any(|a| a.name.is_none()) {
                    return;
                }
                let Some(receiver) = ident(t) else {
                    return;
                };
                let kept = (0..args.len())
                    .filter(|&i| i != t)
                    .map(|i| parts[i].to_owned());
                (receiver, "push_interaction", kept.collect())
            }
            "remove_object" => {
                if args.first().is_none_or(|a| a.name.is_some()) {
                    return;
                }
                let Some(receiver) = ident(0) else {
                    return;
                };
                (
                    receiver,
                    "remove",
                    parts[1..].iter().map(|p| (*p).to_owned()).collect(),
                )
            }
            _ => return,
        };
        let text = format!("{receiver}.{method}({})", kept.join(", "));
        self.propose_method(name, *name_at, start, end, text);
    }

    fn propose_method(&mut self, name: &str, at: Span, start: usize, end: usize, text: String) {
        self.propose(
            PREFER_METHOD_SYNTAX,
            at,
            format!("`{name}(..)` has a method form on its target"),
            format!("write `{text}`; it builds the same primitive"),
            text.clone(),
            vec![Fix { start, end, text }],
            None,
        );
    }
}

/// The first read of `name` with no assignment before it on any path (`None` when every
/// read follows some assignment, or a `&mut name` / `raw` mention ends the search).
fn first_unassigned_read(list: &[V2Stmt], name: &str, user: &dyn Fn(&str) -> bool) -> Option<Span> {
    enum Event {
        Read(Span),
        Write,
    }
    /// A builtin call whose arguments may name a local it writes.
    fn call_events(
        callee: &str,
        args: &[CallArg],
        name: &str,
        user: &dyn Fn(&str) -> bool,
        out: &mut Vec<Event>,
    ) {
        let mut writes = false;
        for (k, a) in args.iter().enumerate() {
            if writes_arg(callee, args, k) && is_ident(&a.value, name) {
                writes = true;
                // `set_to_next(.., dest: x)` starts from the cursor in `x`.
                if reads_written_arg(callee, args, k) {
                    out.push(Event::Read(a.value.span()));
                }
            } else {
                expr_events(&a.value, name, user, out);
            }
        }
        if writes {
            out.push(Event::Write);
        }
    }
    fn expr_events(e: &V2Expr, name: &str, user: &dyn Fn(&str) -> bool, out: &mut Vec<Event>) {
        if let V2Expr::MutRef { inner, .. } = e
            && is_ident(inner, name)
        {
            out.push(Event::Write);
            return;
        }
        if let V2Expr::Call { callee, args, .. } = e
            && let V2Expr::Ident(_, f) = callee.as_ref()
            && LOCAL_WRITING_BUILTINS.contains(&f.as_str())
            && !user(f)
        {
            call_events(f, args, name, user, out);
            return;
        }
        if is_ident(e, name) {
            out.push(Event::Read(e.span()));
            return;
        }
        for c in subexprs(e) {
            expr_events(c, name, user, out);
        }
    }
    fn stmt_events(list: &[V2Stmt], name: &str, user: &dyn Fn(&str) -> bool, out: &mut Vec<Event>) {
        for s in list {
            match s {
                V2Stmt::Assign {
                    target,
                    op: V2AssignOp::Set,
                    value,
                    ..
                } if is_ident(target, name) => {
                    expr_events(value, name, user, out);
                    out.push(Event::Write);
                }
                V2Stmt::RawAir { code, .. }
                    if code
                        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .any(|w| w == name) =>
                {
                    out.push(Event::Write);
                }
                // A new binding of the name ends this one's scope for what follows.
                V2Stmt::Let { name: n, init, .. } | V2Stmt::LetSome { name: n, init, .. }
                    if n == name =>
                {
                    expr_events(init, name, user, out);
                    out.push(Event::Write);
                }
                V2Stmt::LetUninit { name: n, .. } if n == name => out.push(Event::Write),
                // A `defer` body runs when the tree leaves, after everything that follows it
                // here: its reads are not ordered against them, so it is not scanned.
                V2Stmt::Defer { .. } => {}
                // `relationship` writes its `value:` local only when it returns true (a
                // missing row returns false first), and a call statement goes on either
                // way: no write. As `relationship(..)?` or a test it is one.
                // `let _ = relationship(..);` builds the same as the call statement.
                V2Stmt::Expr {
                    expr: V2Expr::Call { callee, args, .. },
                    ..
                }
                | V2Stmt::Let {
                    init: V2Expr::Call { callee, args, .. },
                    ..
                } if matches!(callee.as_ref(), V2Expr::Ident(_, f) if f == "relationship")
                    && !user("relationship")
                    && !matches!(s, V2Stmt::Let { name: n, .. } if n != "_") =>
                {
                    for (k, a) in args.iter().enumerate() {
                        if !(writes_arg("relationship", args, k) && is_ident(&a.value, name)) {
                            expr_events(&a.value, name, user, out);
                        }
                    }
                }
                V2Stmt::Animate {
                    args, event, body, ..
                } => {
                    if user("animate") {
                        // A user tree named `animate` takes the call: its arguments are reads.
                        for a in args {
                            expr_events(&a.value, name, user, out);
                        }
                    } else {
                        call_events("animate", args, name, user, out);
                    }
                    // The handler's `|event|` binding shadows the name inside it.
                    if event.as_deref() != Some(name) {
                        stmt_events(body, name, user, out);
                    }
                }
                other => {
                    for e in stmt_exprs(other) {
                        expr_events(e, name, user, out);
                    }
                    for b in stmt_blocks(other) {
                        stmt_events(b, name, user, out);
                    }
                }
            }
        }
    }
    let mut events = Vec::new();
    stmt_events(list, name, user, &mut events);
    match events.first()? {
        Event::Read(at) => Some(*at),
        Event::Write => None,
    }
}

/// Whether control never goes on past `s` to the next statement of its block: a `return`,
/// `panic!()`, `break` or `continue`, or an `if` with an `else` / a `match` with a `_`
/// arm whose every branch leaves. (A `match` without `_` falls through for a value no arm
/// names; a loop, `with lock`, `defer` or `animate` may go on.)
fn stmt_leaves(s: &V2Stmt) -> bool {
    let leaves = |list: &[V2Stmt]| list.iter().any(stmt_leaves);
    match s {
        V2Stmt::Return { .. }
        | V2Stmt::ReturnError { .. }
        | V2Stmt::Break { .. }
        | V2Stmt::Continue { .. } => true,
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => !else_body.is_empty() && leaves(then_body) && leaves(else_body),
        V2Stmt::Match { arms, .. } => {
            arms.iter()
                .any(|a| matches!(a.pattern, MatchPattern::Wildcard(_)))
                && arms.iter().all(|a| leaves(&a.body))
        }
        _ => false,
    }
}

/// Whether `.air` code defines a label (`@name:`) outside its comments and strings.
fn raw_defines_label(code: &str) -> bool {
    let code = air_code_only(code);
    code.match_indices('@').any(|(at, _)| {
        let rest = &code[at + 1..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        end > 0 && rest[end..].trim_start().starts_with(':')
    })
}

/// A statement's span (that of its first token).
/// The address of an expression node, to recognise it again in a walk.
fn addr(e: &V2Expr) -> usize {
    std::ptr::from_ref(e) as usize
}

/// The `.air` locals (`let NAME` / `local NAME`) a `raw` block declares.
fn raw_locals(code: &str) -> Vec<String> {
    use crate::lexer::TokenKind as K;
    let Ok(tokens) = crate::lexer::tokenize(code) else {
        return Vec::new();
    };
    tokens
        .windows(2)
        .filter_map(|w| match (&w[0].kind, &w[1].kind) {
            (K::Ident(kw), K::Ident(name)) if kw == "let" || kw == "local" => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Parses a `raw` block's `code` after `prelude` the way lowering validates it. On failure,
/// the error and the wrapped text's line holding the code's first line.
fn raw_parse(prelude: &str, code: &str) -> std::result::Result<(), (crate::error::Error, usize)> {
    let wrapped = format!("tree 4096 \"__raw_check\" {{\n{prelude}{code}\n}}\n");
    match crate::parser::parse_module(&wrapped) {
        Ok(_) => Ok(()),
        Err(e) => Err((e, 2 + prelude.lines().count())),
    }
}

/// The constants a `raw` block declares itself, by full name (`K`, `T.e`).
fn raw_own_consts(code: &str) -> BTreeSet<String> {
    use crate::lexer::TokenKind as K;
    let Ok(tokens) = crate::lexer::tokenize(code) else {
        return BTreeSet::new();
    };
    let kind = |i: usize| tokens.get(i).map(|t| &t.kind);
    tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| matches!(&t.kind, K::Ident(k) if k == "const"))
        .filter_map(|(i, _)| match (kind(i + 1), kind(i + 2), kind(i + 3)) {
            (Some(K::Ident(n)), Some(K::Dot), Some(K::Ident(e))) => Some(format!("{n}.{e}")),
            (Some(K::Ident(n)), _, _) => Some(n.clone()),
            _ => None,
        })
        .collect()
}

/// `const NAME = GUESS;` for each name a `raw` block's code uses that another module may
/// declare: a word (`NAME`) or table entry (`T.e`) that is no keyword, call, label, field
/// or declaration, is not in `known`, and the block does not declare itself.
fn raw_guesses(code: &str, known: &BTreeSet<String>) -> String {
    use crate::lexer::TokenKind as K;
    let Ok(tokens) = crate::lexer::tokenize(code) else {
        return String::new();
    };
    let own = raw_own_consts(code);
    let kind = |i: usize| tokens.get(i).map(|t| &t.kind);
    let mut out = BTreeSet::new();
    for (i, t) in tokens.iter().enumerate() {
        let K::Ident(word) = &t.kind else {
            continue;
        };
        let declares = i > 0
            && matches!(&tokens[i - 1].kind, K::Ident(k) if matches!(k.as_str(), "let" | "local" | "const"));
        if (i > 0 && tokens[i - 1].kind == K::Dot)
            || declares
            || crate::parser::is_reserved_ident(word)
            || matches!(kind(i + 1), Some(K::LParen | K::Colon))
        {
            continue;
        }
        let name = match (kind(i + 1), kind(i + 2)) {
            (Some(K::Dot), Some(K::Ident(entry))) => format!("{word}.{entry}"),
            _ => word.clone(),
        };
        if !known.contains(&name) && !own.contains(&name) {
            out.insert(name);
        }
    }
    out.iter()
        .map(|n| format!("const {n} = GUESS;\n"))
        .collect()
}

/// Whether a `raw` block's brackets balance (an unbalanced block may continue another).
fn raw_balanced(code: &str) -> bool {
    use crate::lexer::TokenKind as K;
    let Ok(tokens) = crate::lexer::tokenize(code) else {
        return false;
    };
    let mut depth = 0i64;
    for t in &tokens {
        match t.kind {
            K::LBrace | K::LParen | K::LBracket => depth += 1,
            K::RBrace | K::RParen | K::RBracket => depth -= 1,
            _ => {}
        }
        if depth < 0 {
            return false;
        }
    }
    depth == 0
}

/// The names v2 code refers to a BCON table labelled `label` by.
/// The name a top-level item declares in its module (an object's is a display name).
fn item_name(item: &V2Item) -> Option<&str> {
    match item {
        V2Item::Const(c) => Some(&c.name),
        V2Item::Fn(f) => Some(&f.name),
        V2Item::Template(t) => Some(&t.name),
        V2Item::Enum(e) => Some(&e.name),
        V2Item::Bitflags(b) => Some(&b.name),
        V2Item::ExternMod(m) => Some(&m.name),
        V2Item::ExternBcon(b) => Some(&b.name),
        V2Item::Object(_) => None,
    }
}

fn bcon_names(label: &str) -> Vec<String> {
    let mut names = vec![
        crate::v2::ast::object_type_ident(label),
        crate::v2::parser::snake_case_ident(label),
    ];
    names.dedup();
    names
}

/// The constants a `raw` block in an item with `members` may name: the file's and the
/// members' `const`s, and the BCON entries (`TABLE.entry`) of the members' tables and of
/// the file's and members' `extern bcon`s.
fn air_consts(file: &V2File, members: &[ObjectMember]) -> BTreeMap<String, String> {
    let value = |v: i64| {
        if i16::try_from(v).is_ok() {
            v.to_string()
        } else {
            format!("{v:#010x}")
        }
    };
    let extern_bcon = |b: &crate::v2::ast::ExternBconDecl, out: &mut BTreeMap<String, String>| {
        for e in &b.entries {
            out.insert(
                format!("{}.{}", b.name, e.name),
                format!("bcon[{}]", e.address),
            );
        }
    };
    let mut out = BTreeMap::new();
    for item in &file.items {
        match item {
            V2Item::Const(c) => {
                out.insert(c.name.clone(), value(c.value));
            }
            V2Item::ExternBcon(b) => extern_bcon(b, &mut out),
            _ => {}
        }
    }
    for m in members {
        match m {
            ObjectMember::Const(c) => {
                out.insert(c.name.clone(), value(c.value));
            }
            ObjectMember::Bcon(b) => {
                // Lowering names the table both ways and passes `bcon[(table << 7) | entry]`.
                let Some(table) = b.id.checked_sub(4096) else {
                    continue;
                };
                for (i, (_, entry, _)) in b.entries.iter().enumerate() {
                    let at = (u32::from(table) << 7) | i as u32;
                    for name in bcon_names(&b.label) {
                        out.insert(format!("{name}.{entry}"), format!("bcon[{at}]"));
                    }
                }
            }
            ObjectMember::ExternBcon(b) => extern_bcon(b, &mut out),
            _ => {}
        }
    }
    out
}

/// A `const` in the token stream that binds names.
struct ConstBinding {
    /// The `const` keyword.
    at: Span,
    /// Each bound name's token index, and the name.
    names: Vec<(usize, String)>,
    /// A `const n: T` parameter, not a `const NAME = ..;` / `const (A, B) = ..;`
    /// declaration or binding.
    param: bool,
    /// An arm of its selector returns, breaks or continues: the binding is control flow.
    diverges: bool,
    /// Its selector calls something, which may have effects of its own.
    calls: bool,
}

/// Whether the selector after the `=` at token `eq` (up to its `;`) holds a `return`,
/// `break` or `continue`.
fn selector_diverges(tokens: &[crate::v2::lexer::Token], eq: usize) -> bool {
    selector_words(tokens, eq, false, |w, _| {
        matches!(w, "return" | "break" | "continue")
    })
}

/// Whether the selector after the `=` at token `eq` (up to its `;`) calls a function or
/// method.
fn selector_calls(tokens: &[crate::v2::lexer::Token], eq: usize) -> bool {
    selector_words(tokens, eq, true, |_, next| next == Some(&TokenKind::LParen))
}

/// Whether a word of the selector after the `=` at token `eq` (a field, method or entry
/// after a `.` only with `members`) satisfies `pred(word, next token)`.
fn selector_words(
    tokens: &[crate::v2::lexer::Token],
    eq: usize,
    members: bool,
    pred: impl Fn(&str, Option<&TokenKind>) -> bool,
) -> bool {
    let mut depth = 0i32;
    for i in eq + 1..tokens.len() {
        match &tokens[i].kind {
            TokenKind::LBrace | TokenKind::LParen | TokenKind::LBracket => depth += 1,
            TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket => depth -= 1,
            TokenKind::Semicolon if depth <= 0 => return false,
            TokenKind::Ident(w)
                if (members || tokens[i - 1].kind != TokenKind::Dot)
                    && pred(w, tokens.get(i + 1).map(|t| &t.kind)) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

/// Every `const` declaration, block binding and `const` parameter in `tokens`.
fn const_bindings(tokens: &[crate::v2::lexer::Token]) -> Vec<ConstBinding> {
    let kind = |i: usize| tokens.get(i).map(|t| &t.kind);
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if !matches!(&t.kind, TokenKind::Ident(k) if k == "const") {
            continue;
        }
        match kind(i + 1) {
            Some(TokenKind::Ident(name)) => {
                let param = matches!(kind(i + 2), Some(TokenKind::Colon));
                if param || matches!(kind(i + 2), Some(TokenKind::Assign)) {
                    out.push(ConstBinding {
                        at: t.span,
                        names: vec![(i + 1, name.clone())],
                        param,
                        diverges: !param && selector_diverges(tokens, i + 2),
                        calls: !param && selector_calls(tokens, i + 2),
                    });
                }
            }
            Some(TokenKind::LParen) => {
                let mut names = Vec::new();
                let mut j = i + 2;
                loop {
                    match kind(j) {
                        Some(TokenKind::Ident(n)) => names.push((j, n.clone())),
                        Some(TokenKind::Comma) => {}
                        _ => break,
                    }
                    j += 1;
                }
                if matches!(kind(j), Some(TokenKind::RParen))
                    && matches!(kind(j + 1), Some(TokenKind::Assign))
                {
                    out.push(ConstBinding {
                        at: t.span,
                        names,
                        param: false,
                        diverges: selector_diverges(tokens, j + 1),
                        calls: selector_calls(tokens, j + 1),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// The functions `std::globals` declares.
fn std_globals() -> &'static [ExternFnDecl] {
    static STD: std::sync::OnceLock<Vec<ExternFnDecl>> = std::sync::OnceLock::new();
    STD.get_or_init(|| {
        parse_v2_file(include_str!("std/globals.antics"))
            .map(|f| {
                f.items
                    .into_iter()
                    .filter_map(|i| match i {
                        V2Item::ExternMod(m) => Some(m.fns),
                        _ => None,
                    })
                    .flatten()
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Whether two `extern` functions call the same tree the same way: one ID, the same
/// parameter types and slots, and the same return ABI (names aside).
fn same_abi(a: &ExternFnDecl, b: &ExternFnDecl) -> bool {
    a.id == b.id
        && a.ret == b.ret
        && a.params.len() == b.params.len()
        && a.params
            .iter()
            .zip(&b.params)
            .all(|(p, q)| p.ty == q.ty && p.slot == q.slot)
}

/// An engine cell with a typed name (`untyped_engine_field`).
#[derive(Debug, Clone, Copy)]
struct TypedField {
    /// The indexed cell group (`data`, `person`, `slot`; empty for `global`).
    cell: &'static str,
    index: i64,
    name: &'static str,
    /// Whether the typed name may be assigned.
    writable: bool,
    /// The typed name is an `ObjectId`, the cell an `i16`.
    object_id: bool,
}

const fn typed(
    cell: &'static str,
    index: i64,
    name: &'static str,
    writable: bool,
    object_id: bool,
) -> TypedField {
    TypedField {
        cell,
        index,
        name,
        writable,
        object_id,
    }
}

/// `self.data[N]` cells with an `i16` or `ObjectId` name, as lowering maps them
/// (`lower/lvalue.rs`, `lower/expr.rs`, `lower/tree.rs` `OBJECT_DATA_FIELDS`). `flags`
/// (`data[8]`), `allowed_heights` (`data[4]`), `wall_placement` (`data[13]`) and
/// `placement` (`data[42]`) are typed bitflags, which an `i16` use of the cell does not
/// always type-check as: [`OBJECT_WORDS`] proposes them, byte-checked.
const SELF_FIELDS: &[TypedField] = &[
    typed("data", 0, "graphic", true, false),
    typed("data", 7, "room_impact", true, false),
    typed("data", 11, "id", false, true),
    typed("data", 15, "broken", true, false),
    typed("data", 25, "in_use", true, false),
    typed("data", 29, "room", false, false),
    typed("data", 31, "prep_score", true, false),
    typed("data", 32, "cook_score", true, false),
];

/// A Sim handle's cells with a name (`lower/expr.rs`, `lower/lvalue.rs`,
/// `lower/tree.rs` `SIM_PERSON_FIELDS`).
const SIM_FIELDS: &[TypedField] = &[
    typed("data", 2, "container", false, true),
    typed("data", 11, "id", false, true),
    typed("data", 29, "room", false, false),
    typed("slot", 0, "hand", false, false),
    typed("person", 0, "posture", true, false),
    typed("person", 32, "visitor_kind", false, false),
    typed("person", 33, "priority", true, false),
    typed("person", 34, "pets_outside", true, false),
    typed("person", 56, "job_type", true, false),
    typed("person", 57, "job_level", true, false),
    typed("person", 58, "age", true, false),
    typed("person", 61, "family", true, false),
];

/// Object-data words every object (Sims included) names (`lower/tree.rs`
/// `OBJECT_DATA_WORDS`), past [`SELF_FIELDS`]: `untyped_engine_field` proposes them and
/// keeps a proposal that builds the same bytes (the typed words are not all `i16`).
const OBJECT_WORDS: &[TypedField] = &[
    typed("data", 1, "facing", true, false),
    typed("data", 2, "container", false, true),
    typed("data", 4, "allowed_heights", true, false),
    typed("data", 5, "wall_adjacency", true, false),
    typed("data", 7, "room_impact", true, false),
    typed("data", 8, "flags", true, false),
    typed("data", 13, "wall_placement", true, false),
    typed("data", 15, "broken", true, false),
    typed("data", 27, "weight", true, false),
    typed("data", 28, "support_strength", true, false),
    typed("data", 40, "flags2", true, false),
    typed("data", 41, "value", true, false),
    typed("data", 42, "placement", true, false),
    typed("data", 43, "buy_flags", true, false),
    typed("data", 59, "category", true, false),
];

/// The typed words whose type is a `std::engine` enum.
const ENUM_WORDS: &[&str] = &["facing", "gender_kind"];

/// A Sim's person words past [`SIM_FIELDS`] (`lower/tree.rs` `SIM_PERSON_WORDS`).
const SIM_WORDS: &[TypedField] = &[
    typed("person", 65, "gender_kind", true, false),
    typed("person", 77, "date_partner", true, false),
    typed("person", 81, "fame", true, false),
];

/// A Sim's skills, personality traits and motives by number, with their dot names
/// (`parser.rs` `skill_by_name` and `motive_by_name`, `lower/prim_table.rs`
/// `personality_trait`). A Sim and an untyped handle name them; a typed handle's
/// object is no Sim, and lowering refuses the dot name there, so that proposal falls.
const SIM_GROUP_WORDS: &[TypedField] = &[
    typed("person", 2, "personality.nice", true, false),
    typed("person", 3, "personality.active", true, false),
    typed("person", 5, "personality.playful", true, false),
    typed("person", 6, "personality.outgoing", true, false),
    typed("person", 7, "personality.neat", true, false),
    typed("person", 10, "skill.cooking", true, false),
    typed("person", 11, "skill.charisma", true, false),
    typed("person", 12, "skill.mechanical", true, false),
    typed("person", 15, "skill.creativity", true, false),
    typed("person", 17, "skill.body", true, false),
    typed("person", 18, "skill.logic", true, false),
    typed("motive", 3, "motive.mood", true, false),
    typed("motive", 5, "motive.energy", true, false),
    typed("motive", 6, "motive.comfort", true, false),
    typed("motive", 7, "motive.hunger", true, false),
    typed("motive", 8, "motive.hygiene", true, false),
    typed("motive", 9, "motive.bladder", true, false),
    typed("motive", 11, "motive.alertness", true, false),
    typed("motive", 13, "motive.room", true, false),
    typed("motive", 14, "motive.social", true, false),
    typed("motive", 15, "motive.fun", true, false),
];

/// The cells `stack_obj` and handles name (`lower/tree.rs` `handle_field_cell`), besides
/// [`OBJECT_WORDS`]; the person words only through an untyped handle.
const HANDLE_WORDS: &[TypedField] = &[
    typed("slot", 0, "hand", false, true),
    typed("data", 29, "room", false, false),
    typed("person", 0, "posture", true, false),
    typed("person", 32, "visitor_kind", false, false),
    typed("person", 33, "priority", true, false),
    typed("person", 34, "pets_outside", true, false),
    typed("person", 56, "job_type", true, false),
    typed("person", 57, "job_level", true, false),
    typed("person", 58, "age", true, false),
    typed("person", 61, "family", true, false),
    typed("person", 65, "kind", true, false),
    typed("person", 77, "date_partner", true, false),
    typed("person", 81, "fame", true, false),
];

fn stmt_span(s: &V2Stmt) -> Span {
    {
        match s {
            V2Stmt::Let { span, .. }
            | V2Stmt::LetSome { span, .. }
            | V2Stmt::Assign { span, .. }
            | V2Stmt::WithLock { span, .. }
            | V2Stmt::Defer { span, .. }
            | V2Stmt::Repeat { span, .. }
            | V2Stmt::While { span, .. }
            | V2Stmt::ForEach { span, .. }
            | V2Stmt::If { span, .. }
            | V2Stmt::Match { span, .. }
            | V2Stmt::Break { span }
            | V2Stmt::Continue { span }
            | V2Stmt::Return { span, .. }
            | V2Stmt::ReturnError { span, .. }
            | V2Stmt::Expr { span, .. }
            | V2Stmt::RawAir { span, .. }
            | V2Stmt::LetUninit { span, .. }
            | V2Stmt::Animate { span, .. } => *span,
        }
    }
}

fn hex_guid(v: i64) -> String {
    format!("0x{:04X}_{:04X}", (v >> 16) & 0xFFFF, v & 0xFFFF)
}

/// The builtins with an argument naming a local the VM writes.
const LOCAL_WRITING_BUILTINS: &[&str] = &["random_into", "animate", "relationship", "set_to_next"];

/// Whether argument `k` of a call to the builtin `callee` is a local the VM writes:
/// `random_into`'s first positional argument, `animate`'s event `local:`, and
/// `relationship`'s `value:` unless its `flags:` are a literal with flag 4 (which
/// writes the relationship from the local instead). Flags that are not a literal
/// count as a write, so no read is reported that may not happen.
/// (`set_headline(local:)` and `push_interaction(icon:)` read theirs.)
fn writes_arg(callee: &str, args: &[CallArg], k: usize) -> bool {
    match callee {
        "random_into" => args.iter().position(|a| a.name.is_none()) == Some(k),
        "animate" => args[k].name.as_deref() == Some("local"),
        // Set To Next reads the cursor in `dest:` and writes the next object back
        // (`reads_written_arg`).
        "set_to_next" => args[k].name.as_deref() == Some("dest"),
        "relationship" => {
            args[k].name.as_deref() == Some("value")
                && !args.iter().any(|a| {
                    a.name.as_deref() == Some("flags")
                        && matches!(a.value, V2Expr::Int(_, f) if f & 4 != 0)
                })
        }
        _ => false,
    }
}

/// Whether the `k`th argument of `callee`, which [`writes_arg`], is read first: the cell
/// `set_to_next(.., dest: x)` steps holds the cursor it starts from.
fn reads_written_arg(callee: &str, args: &[CallArg], k: usize) -> bool {
    callee == "set_to_next" && args[k].name.as_deref() == Some("dest")
}

/// Whether `e` reads `name`: a mention that is not only an argument a builtin (not a user
/// function of that name: `user`) writes without reading it first (`random_into(x, ..)`,
/// `relationship(.., value: x)` with literal flags).
fn reads_ident(e: &V2Expr, name: &str, user: &dyn Fn(&str) -> bool) -> bool {
    if let V2Expr::Call { callee, args, .. } = e
        && let V2Expr::Ident(_, f) = callee.as_ref()
        && LOCAL_WRITING_BUILTINS.contains(&f.as_str())
        && !user(f)
    {
        return args.iter().enumerate().any(|(k, a)| {
            if writes_arg(f, args, k) && is_ident(&a.value, name) {
                reads_written_arg(f, args, k) || !write_only_known(f, args)
            } else {
                reads_ident(&a.value, name, user)
            }
        });
    }
    if is_ident(e, name) {
        return true;
    }
    subexprs(e).into_iter().any(|c| reads_ident(c, name, user))
}

/// Whether the call's write-only argument surely is one: `relationship` reads its
/// `value:` local instead when flag 4 is set, which a non-literal `flags:` may hold.
fn write_only_known(callee: &str, args: &[CallArg]) -> bool {
    callee != "relationship"
        || args
            .iter()
            .filter(|a| a.name.as_deref() == Some("flags"))
            .all(|a| matches!(a.value, V2Expr::Int(..)))
}

/// Whether a statement is a store that lowers to one expression node which cannot
/// fault and calls nothing: a store to `temp[0]` to `temp[7]` (or to a `state` of
/// `self` outside a Sim's body, where `self` is the object running the tree and cannot
/// be missing) of literals, `const` names, enum paths and such places, with no division and no
/// flag operation (a bit out of range faults). The result of the node before it is
/// then overwritten without being observed: the tree does not end, call, or fault in
/// between. Conservative: a store to a `let` may be folded or dropped, and a test may
/// fold away (`if 1 < 2 {}`), so neither counts. A local in the value counts only as a
/// `const` name (`is_const`): an unwritten local may fault.
fn runs_a_quiet_node(
    s: &V2Stmt,
    sim: bool,
    is_state: &dyn Fn(&str) -> bool,
    is_const: &dyn Fn(&str) -> bool,
) -> bool {
    let place = |e: &V2Expr| match e {
        V2Expr::Index { base, index, .. } => {
            matches!(base.as_ref(), V2Expr::Ident(_, b) if b == "temp")
                && matches!(index.as_ref(), V2Expr::Int(_, n) if (0..=7).contains(n))
        }
        // A declared `state`: an attribute of the object (another field may lower to
        // a primitive).
        V2Expr::Field { base, field, .. } => {
            !sim && matches!(base.as_ref(), V2Expr::Ident(_, b) if b == "self") && is_state(field)
        }
        _ => false,
    };
    fn value(e: &V2Expr, place: &dyn Fn(&V2Expr) -> bool, is_const: &dyn Fn(&str) -> bool) -> bool {
        match e {
            V2Expr::Int(..) | V2Expr::Path(..) => true,
            V2Expr::Ident(_, n) => is_const(n),
            V2Expr::Index { .. } | V2Expr::Field { .. } => place(e),
            V2Expr::Unary {
                op: V2UnaryOp::Neg,
                inner,
                ..
            } => value(inner, place, is_const),
            V2Expr::Binary { lhs, op, rhs, .. } => {
                matches!(
                    op,
                    V2BinOp::Add
                        | V2BinOp::Sub
                        | V2BinOp::Mul
                        | V2BinOp::BitAnd
                        | V2BinOp::BitOr
                        | V2BinOp::BitXor
                ) && value(lhs, place, is_const)
                    && value(rhs, place, is_const)
            }
            _ => false,
        }
    }
    let V2Stmt::Assign {
        target,
        op,
        value: v,
        ..
    } = s
    else {
        return false;
    };
    place(target)
        && matches!(
            op,
            V2AssignOp::Set
                | V2AssignOp::Add
                | V2AssignOp::Sub
                | V2AssignOp::Mul
                | V2AssignOp::BitAnd
                | V2AssignOp::BitOr
                | V2AssignOp::BitXor
        )
        && value(v, &place, is_const)
        && print_expr(target).is_none_or(|t| print_expr(v).is_none_or(|v| t != v))
}

/// Whether `e` reads a place (`self.x`, `temp[0]`, a local) or calls: a value that is
/// not a constant. A bare name may be a `const`, so it does not count.
/// Whether `e` is one test, or `a && b` of two tests (each maybe negated).
fn one_or_two_tests(e: &V2Expr) -> bool {
    fn test(e: &V2Expr) -> bool {
        match e {
            V2Expr::Unary {
                op: V2UnaryOp::Not,
                inner,
                ..
            } => test(inner),
            V2Expr::Binary { op, .. } => flipped(*op).is_some() || *op == V2BinOp::FlagTest,
            V2Expr::Ident(..)
            | V2Expr::Field { .. }
            | V2Expr::Index { .. }
            | V2Expr::Call { .. }
            | V2Expr::MethodCall { .. } => true,
            _ => false,
        }
    }
    match e {
        V2Expr::Binary {
            lhs,
            op: V2BinOp::And,
            rhs,
            ..
        } => test(lhs) && test(rhs),
        other => test(other),
    }
}

/// Whether a condition's tests may fold to a constant: a `bool` literal, or a comparison
/// or flag test reading no place, in its `!` / `&&` / `||` structure. (A literal inside a
/// call's arguments does not make the call's test fold.)
fn may_fold(e: &V2Expr) -> bool {
    match e {
        V2Expr::Bool(..) => true,
        V2Expr::Unary {
            op: V2UnaryOp::Not,
            inner,
            ..
        } => may_fold(inner),
        V2Expr::Binary {
            lhs,
            op: V2BinOp::And | V2BinOp::Or,
            rhs,
            ..
        } => may_fold(lhs) || may_fold(rhs),
        V2Expr::Binary { lhs, op, rhs, .. }
            if flipped(*op).is_some() || *op == V2BinOp::FlagTest =>
        {
            !(reads_a_place(lhs) || reads_a_place(rhs))
        }
        _ => false,
    }
}

fn reads_a_place(e: &V2Expr) -> bool {
    let mut found = false;
    walk_expr(e, &mut |x| {
        found |= matches!(
            x,
            V2Expr::Field { .. }
                | V2Expr::Index { .. }
                | V2Expr::Call { .. }
                | V2Expr::MethodCall { .. }
        );
    });
    found
}

/// `word[N]` literal indices in `.air` text: `(word, N)` for decimal and hex `N`.
fn raw_index_words(code: &str) -> Vec<(&str, i64)> {
    let mut out = Vec::new();
    for (pos, _) in code.match_indices('[') {
        // Identifier bytes are ASCII, so the byte after the last other byte starts a char.
        let word_start = code.as_bytes()[..pos]
            .iter()
            .rposition(|&b| !(b.is_ascii_alphanumeric() || b == b'_'))
            .map_or(0, |n| n + 1);
        let word = &code[word_start..pos];
        let Some(len) = code[pos + 1..].find(']') else {
            continue;
        };
        if let Some(n) = parse_air_int(code[pos + 1..pos + 1 + len].trim())
            && !word.is_empty()
        {
            out.push((word, n));
        }
    }
    out
}

/// An `.air` integer literal: decimal or `0x` hex, `_` separators allowed.
fn parse_air_int(text: &str) -> Option<i64> {
    let text = text.replace('_', "");
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// The `$Local:N` / `$TimeLocal:N` tokens of a string: `(token, N)`.
fn local_tokens(text: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for prefix in ["$Local:", "$TimeLocal:"] {
        for (pos, _) in text.match_indices(prefix) {
            let digits: String = text[pos + prefix.len()..]
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            if let Ok(n) = digits.parse() {
                out.push((format!("{prefix}{digits}"), n));
            }
        }
    }
    out
}

/// The v2 `let` an `.air` local name lowers from (`__v_count_3` -> `count`).
fn let_source_name(air: &str) -> Option<&str> {
    let rest = air.strip_prefix("__v_")?;
    let (name, id) = rest.rsplit_once('_')?;
    (!name.is_empty() && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(name)
}

/// What a crowded frame's locals are, largest groups first (`None` when no group
/// repeats), e.g. "12 × `ingredient`".
fn local_culprits(frame: &AirFrame) -> Option<String> {
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    for slot in &frame.slots[frame.raw_slots..] {
        let base = let_source_name(slot).map_or_else(
            || match slot.rsplit_once('_') {
                Some((head, id)) if id.bytes().all(|b| b.is_ascii_digit()) => {
                    format!("`{head}` temporaries")
                }
                _ => format!("`{slot}`"),
            },
            |name| format!("`{name}`"),
        );
        *groups.entry(base).or_default() += 1;
    }
    let mut parts: Vec<(usize, String)> = groups
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(name, n)| (n, format!("{n} × {name}")))
        .collect();
    parts.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut shown: Vec<String> = parts.into_iter().take(3).map(|(_, p)| p).collect();
    if frame.raw_slots > 0 {
        shown.insert(
            0,
            format!(
                "{} raw slot(s) up to `local[{}]` before the named locals",
                frame.raw_slots,
                frame.raw_slots - 1
            ),
        );
    }
    (!shown.is_empty()).then(|| shown.join(", "))
}

/// The frames of a lowered `.air` file's trees. Named locals take slots in declaration
/// order after the highest explicit `local[N]` (`.air` lowering's rule). Other forms can
/// reserve slots too (numeric primitive locals), so the caller keeps a frame only when
/// the compiled tree has exactly as many locals ([`verified_frames`]).
fn parse_air_frames(air: &str) -> Vec<AirFrame> {
    fn header(line: &str) -> Option<(u16, String, Option<usize>)> {
        let mut parts = line.splitn(3, ' ');
        let word = parts.next()?;
        let id = parts.next()?.parse().ok()?;
        let rest = parts.next()?;
        if !word.bytes().all(|b| b.is_ascii_lowercase()) || !rest.trim_end().ends_with('{') {
            return None;
        }
        let label = rest.strip_prefix('"')?;
        // The label's closing quote: a `\"` inside it is part of the label.
        let bytes = label.as_bytes();
        let mut end = 0;
        while end < bytes.len() && bytes[end] != b'"' {
            end += if bytes[end] == b'\\' { 2 } else { 1 };
        }
        if end >= bytes.len() {
            return None;
        }
        let args = label
            .get(end + 1..)?
            .split_once("args:")
            .and_then(|(_, r)| {
                let r = r.trim_start();
                r[..r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len())]
                    .parse()
                    .ok()
            });
        // The label as the source wrote it: `\x` is `x`.
        let mut decoded = String::new();
        let mut chars = label.get(..end)?.chars();
        while let Some(c) = chars.next() {
            decoded.push(if c == '\\' { chars.next()? } else { c });
        }
        Some((id, decoded, args))
    }
    fn frame(id: u16, label: String, args: Option<usize>, body: &str) -> Option<AirFrame> {
        let code = strip_air_comments(body);
        let mut lets = Vec::new();
        let mut explicit: Option<usize> = None;
        let mut prev: Option<&str> = None;
        let bytes = code.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'"' {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
                prev = None;
                continue;
            }
            if c.is_ascii_alphabetic() || c == b'_' {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let word = &code[start..i];
                if word == "local" && code[i..].trim_start().starts_with('[') {
                    let rest = &code[i..].trim_start()[1..];
                    let inner = rest.split(']').next().unwrap_or("").trim();
                    match parse_air_int(inner).map(usize::try_from) {
                        // A frame holds at most 255 locals; anything else is the
                        // compiler's to reject.
                        Some(Ok(n)) if n < 256 => explicit = Some(explicit.map_or(n, |m| m.max(n))),
                        _ => return None,
                    }
                    prev = None;
                    continue;
                }
                if matches!(prev, Some("let" | "local")) {
                    lets.push(word.to_owned());
                    prev = None;
                } else {
                    prev = Some(word);
                }
                continue;
            }
            if !c.is_ascii_whitespace() {
                prev = None;
            }
            i += 1;
        }
        let raw_slots = explicit.map_or(0, |m| m + 1);
        let slots = (0..raw_slots)
            .map(|n| format!("local[{n}]"))
            .chain(lets)
            .collect();
        Some(AirFrame {
            id,
            label,
            slots,
            raw_slots,
            args,
        })
    }
    let mut out = Vec::new();
    let mut current: Option<(u16, String, Option<usize>, String)> = None;
    for line in air.lines() {
        if let Some((_, _, _, body)) = &mut current {
            if line == "}" {
                let (id, label, args, body) = current.take().expect("open tree");
                out.extend(frame(id, label, args, &body));
            } else {
                body.push_str(line);
                body.push('\n');
            }
        } else if let Some((id, label, args)) = header(line) {
            current = Some((id, label, args, String::new()));
        }
    }
    out
}

/// Keeps the frames whose slot count is the compiled tree's local count, so a slot
/// layout the scan got wrong (a reserved slot it does not see, `#[frame(locals = )]`) is
/// dropped rather than trusted.
fn verified_frames(
    frames: BTreeMap<String, Vec<AirFrame>>,
    compiled: Option<&[crate::compile::CompiledObject]>,
) -> BTreeMap<String, Vec<AirFrame>> {
    let single = frames.len() == 1;
    frames
        .into_iter()
        .map(|(name, list)| {
            let obj = compiled.and_then(|objects| {
                objects
                    .iter()
                    .find(|o| o.name == name)
                    .or_else(|| objects.first().filter(|_| single && objects.len() == 1))
            });
            let kept = list
                .into_iter()
                .filter(|f| {
                    obj.is_some_and(|o| {
                        o.behaviors
                            .values()
                            .any(|b| b.id == f.id && usize::from(b.locals) == f.slots.len())
                    })
                })
                .collect();
            (name, kept)
        })
        .collect()
}

/// Expands a lint input's objects, or `None` when it does not expand on its own.
/// The file's objects as `check` lowers them, or where lowering stopped (`None` when the
/// error has no place).
fn expand_for_lint(
    input: &LintInput,
) -> std::result::Result<Vec<crate::v2::ExpandedV2Object>, Option<Span>> {
    let lowered = match (&input.path, &input.pack_dir) {
        (Some(path), Some(pack)) => crate::v2::resolve_v2_file(path, Some(pack))
            .and_then(|resolved| crate::v2::lower_resolved_module(&resolved)),
        _ => crate::v2::expand_v2_source_to_objects(
            &input.name,
            &input.source,
            &crate::v2::PackResolverConfig::default(),
        ),
    };
    lowered.map_err(|e| e.span)
}

// ---------------------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------------------

/// Renders a diagnostic the way `rustc` does, quoting its source line.
#[must_use]
pub fn render_diagnostic(d: &LintDiagnostic, source: Option<&str>) -> String {
    let mut out = format!("{}[{}]: {}\n", d.severity.as_str(), d.rule, d.message);
    let gutter = d.line.to_string().len();
    let pad = " ".repeat(gutter);
    out.push_str(&format!("{pad}--> {}:{}:{}\n", d.file, d.line, d.column));
    if let Some(line) = source.and_then(|s| s.lines().nth(d.line.wrapping_sub(1))) {
        let caret_pad: String = line
            .get(..d.column.saturating_sub(1))
            .unwrap_or("")
            .chars()
            .map(|c| if c == '\t' { '\t' } else { ' ' })
            .collect();
        out.push_str(&format!(
            "{pad} |\n{} | {line}\n{pad} | {caret_pad}^\n",
            d.line
        ));
    }
    if let Some(h) = &d.help {
        out.push_str(&format!("{pad} = help: {h}\n"));
    }
    if let Some(s) = &d.suggestion {
        out.push_str(&format!("{pad} = suggestion: `{s}`\n"));
    }
    if let Some(n) = &d.note {
        out.push_str(&format!("{pad} = note: {n}\n"));
    }
    out
}

/// The diagnostics as a JSON array.
#[must_use]
pub fn diagnostics_to_json(diags: &[LintDiagnostic]) -> String {
    let opt = |v: &Option<String>| v.as_deref().map_or("null".to_owned(), json_str);
    let items: Vec<String> = diags
        .iter()
        .map(|d| {
            format!(
                "{{\"rule\":{},\"severity\":{},\"file\":{},\"line\":{},\"column\":{},\"message\":{},\"help\":{},\"suggestion\":{},\"note\":{}}}",
                json_str(d.rule),
                json_str(d.severity.as_str()),
                json_str(&d.file),
                d.line,
                d.column,
                json_str(&d.message),
                opt(&d.help),
                opt(&d.suggestion),
                opt(&d.note),
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
