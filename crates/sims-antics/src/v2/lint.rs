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
    CallArg, FnDecl, ForEachIter, GuardBody, MatchPattern, ObjectMember, ReturnAbi, V2AssignOp,
    V2BinOp, V2Expr, V2File, V2Item, V2Stmt, V2Type, V2UnaryOp,
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
    warn(
        UNUSED_DECL,
        "unused local, state, constant or BCON entry, or a read of an unassigned local",
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
    for (_, ast, tokens) in &parsed {
        symbols.add_file(ast, tokens);
    }

    let mut out = Vec::new();
    for (input, ast, _) in &parsed {
        let directives = Directives::scan(&input.source);
        let mut cx = FileLinter {
            file: &input.name,
            lines: input.source.lines().collect(),
            symbols: &symbols,
            diags: Vec::new(),
        };
        cx.lint_file(ast);
        cx.frame_headroom(input, ast);
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
}

impl Directives {
    fn allows(&self, rule: &str, line: usize) -> bool {
        self.file.contains(rule) || self.lines.get(&line).is_some_and(|s| s.contains(rule))
    }

    fn scan(source: &str) -> Self {
        let lines: Vec<&str> = source.lines().collect();
        // Per line: whether it holds code, and its line comment (column, text after `//`).
        let mut code = vec![false; lines.len()];
        let mut comments: Vec<Option<(usize, String)>> = vec![None; lines.len()];
        let mut block_depth = 0usize;
        let mut in_str = false;
        for (n, line) in lines.iter().enumerate() {
            let b = line.as_bytes();
            let mut i = 0;
            while i < b.len() {
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
                if b[i] == b'"' {
                    in_str = true;
                }
                if !b[i].is_ascii_whitespace() {
                    code[n] = true;
                }
                i += 1;
            }
        }
        let mut out = Self::default();
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
                        out.lines.entry(line).or_default().insert(name);
                    }
                }
            }
        }
        out
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
    /// Enum name -> variant names (`None` when two declarations disagree).
    enums: BTreeMap<String, Option<Vec<String>>>,
    /// Callee (`name` or `module::name`) -> return kind (`None` when definitions disagree).
    returns: BTreeMap<String, Option<RetKind>>,
    /// Every identifier written right after a `.` (fields, methods, BCON entries).
    field_names: BTreeSet<String>,
    /// How often each identifier token appears.
    ident_counts: BTreeMap<String, usize>,
    /// Literal `attr[N]` indices.
    attr_indices: BTreeSet<i64>,
    /// Identifier-like words inside `raw { }` bodies.
    raw_words: BTreeSet<String>,
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
                    self.bcon_by_number |= code.contains("bcon[");
                    for w in code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                        if !w.is_empty() {
                            self.raw_words.insert(w.to_owned());
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
                    ObjectMember::Enum(e) => this.add_enum(&e.name, &e.variants),
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
                V2Item::Enum(e) => self.add_enum(&e.name, &e.variants),
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

    fn add_enum(&mut self, name: &str, variants: &[crate::v2::ast::EnumVariant]) {
        let names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
        self.enums
            .entry(name.to_owned())
            .and_modify(|e| {
                if e.as_ref() != Some(&names) {
                    *e = None;
                }
            })
            .or_insert(Some(names));
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
fn print_expr(e: &V2Expr) -> Option<String> {
    let child = |c: &V2Expr| -> Option<String> {
        let s = print_expr(c)?;
        Some(if matches!(c, V2Expr::Binary { .. }) {
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
        V2Expr::Str(_, s) => format!("{s:?}"),
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
        V2Expr::Ident(_, n) if n != "self" && n != "true" && n != "false" => Some(n.clone()),
        V2Expr::Path(_, segs) => Some(segs.join("::")),
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
    diags: Vec<LintDiagnostic>,
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
                V2Item::Object(o) => self.lint_members(&o.members, false),
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
                ObjectMember::Tree(t) => self.lint_body(
                    &t.body,
                    BodyCx {
                        sim: t.sim_binding.is_some(),
                    },
                ),
                ObjectMember::Init(b)
                | ObjectMember::Main(b)
                | ObjectMember::Load(b)
                | ObjectMember::Reset(b)
                | ObjectMember::Cleanup(b) => self.lint_body(&b.body, BodyCx { sim: false }),
                ObjectMember::Functional(f) => {
                    self.lint_guard(f.guard.as_ref().map(|g| &g.kind));
                    self.lint_body(&f.body, BodyCx { sim: true });
                }
                ObjectMember::Interaction(i) => {
                    self.lint_guard(i.guard.as_ref().map(|g| &g.kind));
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
        if count <= 1 && !self.symbols.raw_words.contains(&c.name) && !c.name.starts_with('_') {
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
        self.lint_list(body, false);
        if cx.sim {
            self.clobber_walk(body, None);
        }
    }

    /// Lints one statement list. `elif` marks the `else if` tail of a chain checked above.
    fn lint_list(&mut self, list: &[V2Stmt], elif: bool) {
        for (i, s) in list.iter().enumerate() {
            if !(elif && i == 0) {
                self.collapsible_chain(s);
            }
            self.lint_stmt(s);
            self.redundant_bool_return(list, i);
            self.unused_let(list, i);
            match s {
                V2Stmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    self.lint_list(then_body, false);
                    let is_elif = matches!(else_body.as_slice(), [V2Stmt::If { .. }]);
                    self.lint_list(else_body, is_elif);
                }
                other => {
                    for b in stmt_blocks(other) {
                        self.lint_list(b, false);
                    }
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
                V2Expr::Unary {
                    span,
                    op: V2UnaryOp::Not,
                    inner,
                } => {
                    if let V2Expr::Binary { lhs, op, rhs, .. } = inner.as_ref()
                        && let Some(flip) = flipped(*op)
                    {
                        let suggestion = print_expr(lhs)
                            .zip(print_expr(rhs))
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
                            Some("v2 keeps `!(a op b)` as the negated compare node with its branches swapped (the original's last-result polarity); if you mirror an original tree's branch shape on purpose, allow this lint".to_owned()),
                        );
                    }
                }
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
        let mut enum_name: Option<&str> = None;
        let mut named = BTreeSet::new();
        for p in pats {
            match p {
                MatchPattern::Wildcard(_) => {}
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
        let Some(Some(variants)) = self.symbols.enums.get(name) else {
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
    fn clobber_walk(&mut self, list: &[V2Stmt], mut clobbered: Option<Span>) -> bool {
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
            let is_clobber = match s {
                V2Stmt::Assign {
                    target,
                    op: V2AssignOp::Set,
                    value,
                    ..
                } if is_ident(target, "stack_obj") => {
                    if is_ident(value, "self") {
                        clobbered = None;
                        ended = true;
                        continue;
                    }
                    true
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
            Some("in a Sim-context body `self` follows `stack_obj`: restore it with `stack_obj = self;` first, or reach the other object through an `ObjectId` local".to_owned()),
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
        if other != Some(!t) {
            return;
        }
        let shown = print_expr(cond);
        let suggestion = shown.map(|c| {
            if t {
                format!("return {c};")
            } else if matches!(cond, V2Expr::Binary { .. }) {
                format!("return !({c});")
            } else {
                format!("return !{c};")
            }
        });
        self.emit(
            REDUNDANT_BOOL_RETURN,
            *span,
            format!(
                "this `if` returns {} when the condition holds and {} otherwise",
                t, !t
            ),
            Some(format!(
                "return the condition directly: `return {}<condition>;`",
                if t { "" } else { "!" }
            )),
            suggestion,
            None,
        );
    }

    fn collapsible_chain(&mut self, s: &V2Stmt) {
        let mut key: Option<String> = None;
        let mut values: Vec<String> = Vec::new();
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
        let Some(key) = key else {
            return;
        };
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

    fn raw_builtin(&mut self, span: Span, code: &str) {
        // One `.air` statement per entry.
        let lines: Vec<&str> = code
            .lines()
            .flat_map(|l| l.split("//").next().unwrap_or("").split(';'))
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
        if name.starts_with('_') {
            return;
        }
        let rest = &list[i + 1..];
        let mut read = false;
        walk_stmts(rest, &mut |s| {
            if let V2Stmt::RawAir { code, .. } = s
                && code
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .any(|w| w == name)
            {
                read = true;
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
        // `else` bodies of `let Some` cannot see the binding; its own uses are in `rest`.
        if !read {
            self.emit(
                UNUSED_DECL,
                span,
                format!("local `{name}` is never read"),
                Some(format!("remove it, or name it `_{name}`")),
                None,
                None,
            );
            return;
        }
        if uninit && let Some(at) = first_unassigned_read(rest, name) {
            self.emit(
                UNUSED_DECL,
                at,
                format!("`{name}` is read before anything assigns it"),
                Some(format!("initialize it: `let {name} = 0;`, or assign it before this read")),
                None,
                Some("the VM zeroes a tree's locals on entry, so this reads 0; say so with `= 0` if that is meant".to_owned()),
            );
        }
    }

    /// `frame_headroom`: compiles the file (with its pack, when it has one) and checks
    /// every tree's node and local counts.
    fn frame_headroom(&mut self, input: &LintInput, ast: &V2File) {
        const NODE_WARN: usize = 200;
        const LOCAL_WARN: u16 = 12;
        let objects = match compile_for_lint(input) {
            Some(o) => o,
            None => return,
        };
        // Source spans by label, for every named member that becomes a tree.
        let mut by_label: BTreeMap<String, Span> = BTreeMap::new();
        let mut object_spans = Vec::new();
        for item in &ast.items {
            if let V2Item::Object(o) = item {
                object_spans.push(o.span);
                for m in &o.members {
                    match m {
                        ObjectMember::Tree(t) => {
                            by_label.insert(t.label.clone(), t.span);
                        }
                        ObjectMember::Interaction(i) => {
                            by_label.insert(i.label.clone(), i.span);
                        }
                        ObjectMember::Init(b) => {
                            by_label.insert("init".to_owned(), b.span);
                        }
                        ObjectMember::Main(b) => {
                            by_label.insert("main".to_owned(), b.span);
                        }
                        _ => {}
                    }
                }
            }
        }
        let fallback = object_spans.first().copied().unwrap_or(Span::new(1, 1));
        for obj in objects {
            for b in obj.behaviors.values() {
                let nodes = b.instructions.len();
                let at = by_label.get(&b.label).copied().unwrap_or(fallback);
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
                    self.emit(
                        FRAME_HEADROOM,
                        at,
                        format!("tree {} \"{}\" uses {} locals", b.id, b.label, b.locals),
                        Some("reuse locals across disjoint parts, or split the tree".to_owned()),
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
    fn expr_events(e: &V2Expr, name: &str, out: &mut Vec<Event>) {
        if let V2Expr::MutRef { inner, .. } = e
            && is_ident(inner, name)
        {
            out.push(Event::Write);
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

/// Compiles a lint input's objects, or `None` when it does not compile on its own.
fn compile_for_lint(input: &LintInput) -> Option<Vec<crate::compile::CompiledObject>> {
    let expanded = match (&input.path, &input.pack_dir) {
        (Some(path), Some(pack)) => {
            let resolved = crate::v2::resolve_v2_file(path, Some(pack)).ok()?;
            crate::v2::lower_resolved_module(&resolved).ok()?
        }
        _ => crate::v2::expand_v2_source_to_objects(
            &input.name,
            &input.source,
            &crate::v2::PackResolverConfig::default(),
        )
        .ok()?,
    };
    expanded
        .into_iter()
        .map(|e| crate::v2::compile_expanded_v2_object(e).ok())
        .collect()
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
