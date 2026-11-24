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
//! `enum_wildcard_all_covered` cannot be suppressed. See
//! `docs/decisions/20261124-antics-v2-lint.md`.

use crate::error::{Error, Result, Span};
use crate::v2::ast::{
    CallArg, FnDecl, ForEachIter, GuardBody, MatchPattern, ObjectMember, ReturnAbi, StateType,
    V2AssignOp, V2BinOp, V2Expr, V2File, V2Item, V2Stmt, V2Type, V2UnaryOp,
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
pub const WHILE_TRUE: &str = "while_true";
pub const REDUNDANT_BOOL_RETURN: &str = "redundant_bool_return";
pub const COLLAPSIBLE_IF_CHAIN: &str = "collapsible_if_chain";
pub const MANUAL_RAW_HAS_BUILTIN: &str = "manual_raw_has_builtin";
pub const MAGIC_GUID_LITERAL: &str = "magic_guid_literal";
pub const NEGATED_COMPARISON: &str = "negated_comparison";
pub const UNKNOWN_LINT_RULE: &str = "unknown_lint_rule";
pub const UNINITIALIZED_LOCAL_READ: &str = "uninitialized_local_read";
pub const DIALOG_LOCAL_SLOT_MISMATCH: &str = "dialog_local_slot_mismatch";
pub const RAW_BARE_CALL_FALSE_EXIT: &str = "raw_bare_call_false_exit";
pub const MISSING_SIM_FRAME_ARGS: &str = "missing_sim_frame_args";

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
        UNKNOWN_LINT_RULE,
        "an `antics-lint: allow(...)` directive naming no rule",
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
        if !is_lintable_v2(&source) {
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

/// Whether `source` is linted as v2 rather than skipped as `.air` (Antics IR), which shares
/// the extension.
///
/// [`crate::v2::is_v2_source`] decides by the first keyword, which misses a v2 file that
/// starts with `const` (both languages have it). So a file is also v2 when it parses as
/// v2, or when it parses as neither, so that its syntax error is reported rather than the
/// file skipped.
#[must_use]
pub fn is_lintable_v2(source: &str) -> bool {
    crate::v2::is_v2_source(source)
        || parse_v2_file(source).is_ok()
        || crate::parser::parse_module(source).is_err()
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
    for (_, ast, tokens) in &parsed {
        symbols.add_file(ast, tokens);
    }

    let mut out = Vec::new();
    for (input, ast, tokens) in &parsed {
        let directives = Directives::scan(&input.source, tokens);
        let expanded = expand_for_lint(input);
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
        let frames = frames.map(|f| verified_frames(f, compiled.as_deref()));
        let mut cx = FileLinter {
            file: &input.name,
            lines: input.source.lines().collect(),
            symbols: &symbols,
            enums: file_enums(input, ast),
            states: file_states(ast),
            frames,
            object: None,
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
            diags: Vec::new(),
        };
        cx.lint_file(ast);
        cx.frame_headroom(ast, compiled.as_deref());
        let mut diags = cx.diags;
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
        for mut d in diags {
            if !directives.allows(d.rule, d.line) {
                out.push(d);
                continue;
            }
            if rule(d.rule).is_some_and(|r| r.suppressible) {
                continue;
            }
            d.note = Some(format!(
                "`{}` cannot be allowed: the `_` arm only runs for a value that is none of the enum's variants. To handle out-of-range raw values on purpose, type the value `i16` instead of the enum.",
                d.rule
            ));
            out.push(d);
        }
    }
    out.sort_by(|a, b| {
        (&a.file, a.line, a.column, a.rule).cmp(&(&b.file, b.line, b.column, b.rule))
    });
    out.dedup();
    Ok(out)
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
    /// Each `raw { }` block's body: the `raw` keyword's `(line, column)` -> the byte
    /// offset of the body's first byte (after `{`) in the source.
    raw_bodies: BTreeMap<(usize, usize), usize>,
    /// The byte offset each line starts at.
    line_starts: Vec<usize>,
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
            let last_line = match target {
                Some(line) if code[n] => block_end_line(tokens, line).unwrap_or(line),
                other => other.unwrap_or(0),
            };
            for name in names {
                if rule(&name).is_none() {
                    out.unknown.push((n + 1, *column, name));
                    continue;
                }
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
    /// How often each identifier token appears.
    ident_counts: BTreeMap<String, usize>,
    /// Literal `attr[N]` indices.
    attr_indices: BTreeSet<i64>,
    /// Identifier-like words in `raw { }` code (not its comments or strings), less the
    /// names a block declares as its own `.air` constants: lowering resolves every other
    /// v2 `const` a raw block names.
    raw_words: BTreeSet<String>,
    /// Literal indices `raw { }` bodies address (`attr[5]`, `data[0x10]`), outside the
    /// frame's own `local` / `temp` / `param` words: a constant written there as its value
    /// is used all the same.
    raw_index_literals: BTreeSet<i64>,
    /// Whether code outside the file set's names may read BCON entries by number: a `raw`
    /// `bcon[N]`, or an `extern` tree (globals and semi-globals can read an object's
    /// private tables).
    bcon_by_number: bool,
}

impl Symbols {
    fn add_file(&mut self, file: &V2File, tokens: &[crate::v2::lexer::Token]) {
        for (i, t) in tokens.iter().enumerate() {
            match &t.kind {
                TokenKind::Ident(name) => {
                    *self.ident_counts.entry(name.clone()).or_default() += 1;
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
                    for (word, n) in raw_index_words(&outside) {
                        if !matches!(word, "local" | "temp" | "param") {
                            self.raw_index_literals.insert(n);
                        }
                    }
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
                V2Item::Object(o) => add_members(&o.members, self),
                V2Item::Template(t) => add_members(&t.members, self),
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

/// A `tree` or `extern` fn's return kind. A `-> bool` tree is often run for what it does
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
        | V2Stmt::ForEach { body, .. }
        | V2Stmt::Animate { body, .. } => vec![body],
        V2Stmt::If {
            then_body,
            else_body,
            ..
        } => vec![then_body, else_body],
        V2Stmt::Match { arms, .. } => arms.iter().map(|a| a.body.as_slice()).collect(),
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
const NON_YIELDING_METHODS: &[&str] = &["is_type", "set", "clear", "contains", "is_empty"];

/// `.air` primitives a v2 builtin spells with the same operands, with that spelling.
///
/// Primitives whose v2 spelling takes different operands are left out, since a raw block
/// holding them is often the only way to say it: `play_sound` (v2 names a declared
/// `sound`), `stop_all_sounds` and `push_interaction` (v2 picks the target from the
/// tree's context), `test_object_type` as a statement, `set_to_next` outside a loop, and
/// `stack_obj :=` (not every source has a v2 spelling).
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
    /// Enums this file can name (see `file_enums`).
    enums: BTreeMap<String, Option<Vec<String>>>,
    /// The file's object states by name, with their type (`None` when two disagree).
    states: BTreeMap<String, Option<StateType>>,
    /// The lowered `.air` frames of each object the file expands to, by object name
    /// (`None` when the file does not expand on its own).
    frames: Option<BTreeMap<String, Vec<AirFrame>>>,
    /// The object being linted.
    object: Option<String>,
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
    let literal = |v: &str| -> Option<i64> {
        let (neg, digits) = v.strip_prefix('-').map_or((false, v), |d| (true, d));
        let n = match digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"))
        {
            Some(hex) => i64::from_str_radix(&hex.replace('_', ""), 16).ok()?,
            None => digits.replace('_', "").parse().ok()?,
        };
        if neg { n.checked_neg() } else { Some(n) }
    };
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
                let start = i;
                let mut depth = 0usize;
                while i < b.len() {
                    if b[i..].starts_with(b"/*") {
                        depth += 1;
                        i += 2;
                    } else if b[i..].starts_with(b"*/") {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                let end = i.min(b.len());
                blank(&mut out, start, end);
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
            match item {
                V2Item::Fn(f) => self.lint_fn(f),
                V2Item::Const(c) if !c.is_pub => self.unused_const(c),
                V2Item::Object(o) => {
                    self.object = Some(o.name.clone());
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
                    self.lint_members(&o.members, false);
                    self.object = None;
                    self.strings.clear();
                    self.dialog_strings.clear();
                    self.tables.clear();
                }
                V2Item::Template(t) => self.lint_members(&t.members, true),
                _ => {}
            }
        }
    }

    fn lint_fn(&mut self, f: &FnDecl) {
        let sim = f.params.iter().any(|p| p.ty == V2Type::Sim);
        self.lint_body(&f.body, BodyCx { sim });
    }

    fn lint_members(&mut self, members: &[ObjectMember], template: bool) {
        for m in members {
            match m {
                ObjectMember::Fn(f) => self.lint_fn(f),
                ObjectMember::Tree(t) => {
                    self.missing_sim_frame_args(t);
                    self.enter_tree(t.pinned_id, &t.label, &t.body);
                    self.lint_body(
                        &t.body,
                        BodyCx {
                            sim: t.sim_binding.is_some(),
                        },
                    );
                }
                ObjectMember::Init(b) | ObjectMember::Main(b) => {
                    let label = if matches!(m, ObjectMember::Init(_)) {
                        "Init"
                    } else {
                        "Main"
                    };
                    self.enter_tree(None, label, &b.body);
                    self.lint_body(&b.body, BodyCx { sim: false });
                }
                ObjectMember::Load(b) | ObjectMember::Reset(b) | ObjectMember::Cleanup(b) => {
                    self.enter_tree(None, "", &b.body);
                    self.lint_body(&b.body, BodyCx { sim: false });
                }
                ObjectMember::Functional(f) => {
                    self.lint_guard(f.guard.as_ref().map(|g| &g.kind));
                    self.lint_body(&f.body, BodyCx { sim: true });
                }
                ObjectMember::Interaction(i) => {
                    self.lint_guard(i.guard.as_ref().map(|g| &g.kind));
                    let label = crate::v2::ast::object_type_ident(&i.label);
                    self.enter_tree(i.pinned_id, &label, &i.run_body);
                    self.lint_body(&i.run_body, BodyCx { sim: true });
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
                    let used: Vec<bool> = b
                        .entries
                        .iter()
                        .map(|(_, name, _)| self.symbols.field_names.contains(name))
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
        let count = self.symbols.ident_counts.get(&c.name).copied().unwrap_or(0);
        if count > 1 || c.name.starts_with('_') {
            return;
        }
        // A `raw` block that names the constant uses it: lowering declares it for the
        // block as an `.air` constant.
        if !self.symbols.raw_words.contains(&c.name)
            && !self.symbols.raw_index_literals.contains(&c.value)
        {
            self.emit(
                UNUSED_DECL,
                c.span,
                format!("constant `{}` is never used", c.name),
                Some("remove it".to_owned()),
                None,
                None,
            );
        }
    }

    fn lint_guard(&mut self, guard: Option<&GuardBody>) {
        match guard {
            Some(GuardBody::Expr(e)) => self.lint_expr(e),
            Some(GuardBody::Block(b)) => self.lint_body(b, BodyCx { sim: true }),
            None => {}
        }
    }

    fn lint_body(&mut self, body: &[V2Stmt], cx: BodyCx) {
        self.sim_body = cx.sim;
        self.body_has_defer = false;
        walk_stmts(body, &mut |s| {
            self.body_has_defer |= matches!(s, V2Stmt::Defer { .. })
        });
        self.lint_list(body, false, false);
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
    /// read `param[n]` past its `@frame(args:)`.
    fn missing_sim_frame_args(&mut self, t: &crate::v2::ast::TreeMemberDecl) {
        if t.sim_binding.is_none() || !t.params.is_empty() {
            return;
        }
        // A pinned `@frame(args:)` declares the frame (a read past it does not compile).
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
                "declare its arguments: pin `@frame(args: {})` (or the original's count), or declare the parameters; `(sim: Sim)` is the frame's Sim, not an argument word",
                n + 1
            )),
            None,
            Some("the compiled header takes its argument count from the raw read alone, and only a caller that passes `temps` fills those words; a lifted tree keeps the original's `@frame(args:)` so the count matches".to_owned()),
        );
    }

    /// Lints one statement list. `elif` marks the `else if` tail of a chain checked above. `follows`: whether code runs after the list ends (it
    /// is nested in a list that goes on, or is a loop body).
    fn lint_list(&mut self, list: &[V2Stmt], elif: bool, follows: bool) {
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
                    for b in stmt_blocks(other) {
                        self.lint_list(b, false, after || looped);
                    }
                    self.in_block -= usize::from(block);
                }
            }
        }
    }

    fn lint_stmt(&mut self, s: &V2Stmt) {
        for e in stmt_exprs(s) {
            self.lint_expr(e);
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
            V2Stmt::Expr { span, expr } => self.dropped_bool_call(*span, expr),
            V2Stmt::RawAir { span, code } => self.raw_builtin(*span, code),
            _ => {}
        }
    }

    fn lint_expr(&mut self, e: &V2Expr) {
        let mut found: Vec<&V2Expr> = Vec::new();
        walk_expr(e, &mut |x| found.push(x));
        for x in found {
            match x {
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
                _ => {}
            }
        }
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
            V2Expr::MethodCall { method, .. } if method == "is_type" => {
                ("is_type".to_owned(), RetKind::Bool)
            }
            _ => return,
        };
        let help = match kind {
            RetKind::Result => format!(
                "propagate the failure with `{name}(..)?;`, or test it with `if !{name}(..) {{ ... }}`"
            ),
            RetKind::Bool => format!("test it: `if {name}(..) {{ ... }}`, or return it"),
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

    fn raw_builtin(&mut self, span: Span, code: &str) {
        // One `.air` statement per entry.
        let lines: Vec<&str> = code
            .lines()
            .flat_map(|l| {
                let l = l.split("//").next().unwrap_or("");
                l.split('#').next().unwrap_or("").split(';')
            })
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if lines.is_empty() {
            return;
        }
        let mut spellings = Vec::new();
        for l in &lines {
            if l.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(|w| w == "pad")
                || l.contains("->")
                || l.contains('{')
                || l.contains('}')
            {
                return;
            }
            let word: String = l
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let Some((_, v2)) = RAW_BUILTINS.iter().find(|(air, _)| *air == word) else {
                return;
            };
            if !native_operands(&word, l[word.len()..].trim()) {
                return;
            }
            if !spellings.contains(v2) {
                spellings.push(*v2);
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

    /// `let` bindings never read, and `let x: T;` read before any assignment.
    fn unused_let(&mut self, list: &[V2Stmt], i: usize) {
        let (span, name, uninit) = match &list[i] {
            V2Stmt::Let { span, name, .. } | V2Stmt::LetSome { span, name, .. } => {
                (*span, name, false)
            }
            V2Stmt::LetUninit { span, name, .. } => (*span, name, true),
            _ => return,
        };
        // `@slot(N) let`: the frame word is pinned so that code addressing it by number
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
                read |= mentions_ident(e, name).is_some();
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
        if uninit && let Some(at) = first_unassigned_read(rest, name) {
            self.emit(
                UNINITIALIZED_LOCAL_READ,
                at,
                format!("`{name}` is read before anything assigns it"),
                Some(format!("assign it before this read, or initialize it: `let {name} = 0;` (one more node)")),
                None,
                Some("the VM zeroes a tree's locals on entry, so this reads 0; say so with `= 0` if that is meant".to_owned()),
            );
        }
    }

    /// `frame_headroom`: compiles the file (with its pack, when it has one) and checks
    /// every tree's node and local counts.
    fn frame_headroom(&mut self, ast: &V2File, objects: Option<&[crate::compile::CompiledObject]>) {
        const NODE_WARN: usize = 200;
        const LOCAL_WARN: u16 = 12;
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
            for m in &o.members {
                let (id, label, span) = match m {
                    ObjectMember::Tree(t) => (t.pinned_id, t.label.as_str(), t.span),
                    ObjectMember::Interaction(i) => (i.pinned_id, i.label.as_str(), i.span),
                    ObjectMember::Init(b) => (None, "init", b.span),
                    ObjectMember::Main(b) => (None, "main", b.span),
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
                if nodes > NODE_WARN {
                    self.emit(
                        FRAME_HEADROOM,
                        at,
                        format!(
                            "tree {} \"{}\" compiles to {nodes} nodes; a tree holds at most {}",
                            b.id,
                            b.label,
                            crate::lower::MAX_BHAV_NODES
                        ),
                        Some("split it: move a self-contained part into its own `tree`".to_owned()),
                        None,
                        None,
                    );
                }
                if b.locals > LOCAL_WARN {
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
                        Some("reuse locals across disjoint parts, call a helper `tree` instead of inlining a `fn` many times (each inlined call gets its own locals), or split the tree".to_owned()),
                        None,
                        None,
                    );
                }
            }
        }
    }
}

/// The first read of `name` with no assignment before it on any path (`None` when every
/// read follows some assignment, or a `&mut name` / `raw` mention ends the search).
fn first_unassigned_read(list: &[V2Stmt], name: &str) -> Option<Span> {
    enum Event {
        Read(Span),
        Write,
    }
    /// A builtin call whose arguments may name a local it writes.
    fn call_events(callee: &str, args: &[CallArg], name: &str, out: &mut Vec<Event>) {
        let mut writes = false;
        for (k, a) in args.iter().enumerate() {
            if writes_arg(callee, args, k) && is_ident(&a.value, name) {
                writes = true;
            } else {
                expr_events(&a.value, name, out);
            }
        }
        if writes {
            out.push(Event::Write);
        }
    }
    fn expr_events(e: &V2Expr, name: &str, out: &mut Vec<Event>) {
        if let V2Expr::MutRef { inner, .. } = e
            && is_ident(inner, name)
        {
            out.push(Event::Write);
            return;
        }
        if let V2Expr::Call { callee, args, .. } = e
            && let V2Expr::Ident(_, f) = callee.as_ref()
            && LOCAL_WRITING_BUILTINS.contains(&f.as_str())
        {
            call_events(f, args, name, out);
            return;
        }
        if is_ident(e, name) {
            out.push(Event::Read(e.span()));
            return;
        }
        for c in subexprs(e) {
            expr_events(c, name, out);
        }
    }
    fn stmt_events(list: &[V2Stmt], name: &str, out: &mut Vec<Event>) {
        for s in list {
            match s {
                V2Stmt::Assign {
                    target,
                    op: V2AssignOp::Set,
                    value,
                    ..
                } if is_ident(target, name) => {
                    expr_events(value, name, out);
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
                    expr_events(init, name, out);
                    out.push(Event::Write);
                }
                V2Stmt::LetUninit { name: n, .. } if n == name => out.push(Event::Write),
                // `relationship` writes its `value:` local only when it returns true (a
                // missing row returns false first), and a call statement goes on either
                // way: no write. As `relationship(..)?` or a test it is one.
                V2Stmt::Expr {
                    expr: V2Expr::Call { callee, args, .. },
                    ..
                } if matches!(callee.as_ref(), V2Expr::Ident(_, f) if f == "relationship") => {
                    for (k, a) in args.iter().enumerate() {
                        if !(writes_arg("relationship", args, k) && is_ident(&a.value, name)) {
                            expr_events(&a.value, name, out);
                        }
                    }
                }
                V2Stmt::Animate {
                    args, event, body, ..
                } => {
                    call_events("animate", args, name, out);
                    // The handler's `|event|` binding shadows the name inside it.
                    if event.as_deref() != Some(name) {
                        stmt_events(body, name, out);
                    }
                }
                other => {
                    for e in stmt_exprs(other) {
                        expr_events(e, name, out);
                    }
                    for b in stmt_blocks(other) {
                        stmt_events(b, name, out);
                    }
                }
            }
        }
    }
    let mut events = Vec::new();
    stmt_events(list, name, &mut events);
    match events.first()? {
        Event::Read(at) => Some(*at),
        Event::Write => None,
    }
}

/// A statement's span (that of its first token).
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
const LOCAL_WRITING_BUILTINS: &[&str] = &["random_into", "animate", "relationship"];

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
    fn header(line: &str) -> Option<(u16, String)> {
        let mut parts = line.splitn(3, ' ');
        let word = parts.next()?;
        let id = parts.next()?.parse().ok()?;
        let rest = parts.next()?;
        if !word.bytes().all(|b| b.is_ascii_lowercase()) || !rest.trim_end().ends_with('{') {
            return None;
        }
        let label = rest.strip_prefix('"')?;
        let label = &label[..label.find('"')?];
        Some((id, label.to_owned()))
    }
    fn frame(id: u16, label: String, body: &str) -> Option<AirFrame> {
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
        })
    }
    let mut out = Vec::new();
    let mut current: Option<(u16, String, String)> = None;
    for line in air.lines() {
        if let Some((_, _, body)) = &mut current {
            if line == "}" {
                let (id, label, body) = current.take().expect("open tree");
                out.extend(frame(id, label, &body));
            } else {
                body.push_str(line);
                body.push('\n');
            }
        } else if let Some((id, label)) = header(line) {
            current = Some((id, label, String::new()));
        }
    }
    out
}

/// Keeps the frames whose slot count is the compiled tree's local count, so a slot
/// layout the scan got wrong (a reserved slot it does not see, `@frame(locals:)`) is
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
fn expand_for_lint(input: &LintInput) -> Option<Vec<crate::v2::ExpandedV2Object>> {
    match (&input.path, &input.pack_dir) {
        (Some(path), Some(pack)) => {
            let resolved = crate::v2::resolve_v2_file(path, Some(pack)).ok()?;
            crate::v2::lower_resolved_module(&resolved).ok()
        }
        _ => crate::v2::expand_v2_source_to_objects(
            &input.name,
            &input.source,
            &crate::v2::PackResolverConfig::default(),
        )
        .ok(),
    }
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
