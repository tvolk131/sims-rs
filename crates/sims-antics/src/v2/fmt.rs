//! Canonical `.antics` v2 formatter (`sims-antics fmt`).
//!
//! The formatter reflows the *token stream* instead of pretty-printing the AST: the AST drops
//! comments and surface spellings (`loop`, `:=`, `0x7F00_0020`, meta-field aliases), while a
//! token reflow keeps every token's text exactly and every comment verbatim. Three guarantees
//! hold for every successful call:
//!
//! - **Semantic preservation:** the output lexes (with the real v2 lexer) to the same token
//!   sequence as the input, `raw { ... }` bodies compared by their trimmed lines (all lowering
//!   reads). This is checked on every call; a mismatch is an error, never a silent rewrite.
//! - **Comment preservation:** every `//`, `///` and `/* */` comment survives verbatim (trailing
//!   whitespace trimmed). Also checked on every call.
//! - **Idempotence:** every layout rule depends only on facts the output itself preserves.
//!
//! See `docs/decisions/20261123-antics-v2-fmt-and-decompiler.md`.

use super::lexer::{TokenKind, tokenize};
use super::parser::parse_v2_file;
use crate::error::{Error, Result};

/// Formats `.antics` v2 source text into its canonical layout.
///
/// The input must parse as v2 (`parse_v2_file`). Comments are preserved verbatim and the
/// output is checked to lex to the same token sequence as the input.
pub fn format_v2_source(source: &str) -> Result<String> {
    parse_v2_file(source)?;
    let scanned = scan(source)?;
    let out = Printer::new(&scanned).print();

    // Self-checks: identical tokens (modulo raw-body indentation) and identical comments.
    let before = tokenize(source)?;
    let after = tokenize(&out)
        .map_err(|e| Error::new(format!("formatter produced unlexable output: {e}")))?;
    let norm = |toks: Vec<super::lexer::Token>| -> Vec<TokenKind> {
        toks.into_iter()
            .map(|t| match t.kind {
                TokenKind::RawBlock(body) => TokenKind::RawBlock(normalize_raw_body(&body)),
                other => other,
            })
            .collect()
    };
    if norm(before) != norm(after) {
        return Err(Error::new(
            "internal formatter error: formatted output changes the token stream",
        ));
    }
    let rescanned = scan(&out)?;
    let comments = |s: &Scanned| -> Vec<String> {
        s.tokens
            .iter()
            .flat_map(|t| t.leading.iter())
            .chain(s.trailing.iter())
            .map(|c| c.text.clone())
            .collect()
    };
    if comments(&scanned) != comments(&rescanned) {
        return Err(Error::new(
            "internal formatter error: formatted output changes the comments",
        ));
    }
    Ok(out)
}

/// Returns `true` if `source` is already in canonical `.antics` v2 layout.
pub fn is_v2_formatted(source: &str) -> Result<bool> {
    Ok(format_v2_source(source)? == source)
}

/// Collects the `.antics` files named by `paths`: files as given, directories walked
/// recursively for `*.antics` (sorted, symlinks skipped).
pub fn collect_antics_files(paths: &[std::path::PathBuf]) -> Result<Vec<std::path::PathBuf>> {
    // The listing is sorted before use, so its order never depends on the filesystem.
    #[allow(clippy::disallowed_methods)]
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| Error::new(format!("failed to read {}: {e}", dir.display())))?
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                walk(&path, out)?;
            } else if meta.is_file() && path.extension().is_some_and(|e| e == "antics") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for path in paths {
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| Error::new(format!("failed to access {}: {e}", path.display())))?;
        if meta.file_type().is_symlink() {
            return Err(Error::new(format!("refusing symlink {}", path.display())));
        }
        if meta.is_dir() {
            walk(path, &mut out)?;
        } else if meta.is_file() {
            out.push(path.clone());
        } else {
            return Err(Error::new(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
    }
    Ok(out)
}

/// The lines lowering keeps from a `raw { ... }` body: trimmed, non-empty.
fn normalize_raw_body(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------------------------
// Scanner: tokens with exact source text plus comment / line-break trivia.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Comment {
    /// Verbatim comment text (trailing whitespace trimmed).
    text: String,
    is_line: bool,
    /// Line breaks between the previous item (token or comment) and this comment.
    newlines_before: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Ident,
    Number,
    Str,
    Color,
    Raw,
    Punct,
}

#[derive(Debug, Clone)]
struct Tok {
    /// Exact source text (for `Raw`: the body between the braces).
    text: String,
    class: Class,
    /// For `Raw`: trivia (comments) between `raw` and `{`, verbatim.
    raw_prefix: String,
    leading: Vec<Comment>,
    /// Line breaks between the previous item (token or comment) and this token.
    newlines_before: usize,
}

struct Scanned {
    tokens: Vec<Tok>,
    /// Comments after the last token.
    trailing: Vec<Comment>,
}

fn scan(source: &str) -> Result<Scanned> {
    let bytes = source.as_bytes();
    let mut pos = 0usize;
    let mut tokens: Vec<Tok> = Vec::new();
    loop {
        let (comments, newlines, next) = scan_trivia(source, pos)?;
        pos = next;
        if pos >= bytes.len() {
            let mut trailing = comments;
            // Line breaks before EOF are irrelevant.
            let _ = newlines;
            for c in &mut trailing {
                c.text = c.text.trim_end().to_owned();
            }
            return Ok(Scanned { tokens, trailing });
        }
        let start = pos;
        let b = bytes[pos];
        let (class, end) = match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => (Class::Ident, scan_ident_end(bytes, pos)),
            b'0'..=b'9' => (Class::Number, scan_number_end(bytes, pos)),
            b'"' => (Class::Str, scan_string_end(bytes, pos)?),
            b'#' => (Class::Color, scan_ident_end(bytes, pos + 1)),
            _ => (Class::Punct, pos + punct_len(bytes, pos)?),
        };
        let mut tok = Tok {
            text: source[start..end].to_owned(),
            class,
            raw_prefix: String::new(),
            leading: comments,
            newlines_before: newlines,
        };
        pos = end;
        if class == Class::Ident && tok.text == "raw" {
            let at_stmt_start = tokens.last().is_some_and(|t| {
                matches!(t.text.as_str(), "{" | "}" | ";") || t.class == Class::Raw
            });
            if at_stmt_start {
                let (pre_comments, _, brace) = scan_trivia(source, pos)?;
                if bytes.get(brace) == Some(&b'{') {
                    let body_end = scan_raw_body_end(bytes, brace)?;
                    tok.class = Class::Raw;
                    tok.raw_prefix = pre_comments
                        .iter()
                        .map(|c| c.text.trim_end().to_owned())
                        .collect::<Vec<_>>()
                        .join(" ");
                    if pre_comments.iter().any(|c| c.is_line) {
                        return Err(Error::new(
                            "unsupported layout: a line comment between 'raw' and '{'",
                        ));
                    }
                    tok.text = source[brace + 1..body_end].to_owned();
                    pos = body_end + 1;
                }
            }
        }
        for c in &mut tok.leading {
            c.text = c.text.trim_end().to_owned();
        }
        tokens.push(tok);
    }
}

/// Skips whitespace and comments from `pos`; returns the comments, the line breaks after the
/// last comment (or since `pos` when there is none), and the position of the next token.
fn scan_trivia(source: &str, mut pos: usize) -> Result<(Vec<Comment>, usize, usize)> {
    let bytes = source.as_bytes();
    let mut comments = Vec::new();
    let mut newlines = 0usize;
    loop {
        while let Some(&b) = bytes.get(pos) {
            if b == b'\n' {
                newlines += 1;
            } else if !matches!(b, b' ' | b'\t' | b'\r') {
                break;
            }
            pos += 1;
        }
        if bytes.get(pos) == Some(&b'/') && bytes.get(pos + 1) == Some(&b'/') {
            let start = pos;
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            comments.push(Comment {
                text: source[start..pos].to_owned(),
                is_line: true,
                newlines_before: newlines,
            });
            newlines = 0;
            continue;
        }
        if bytes.get(pos) == Some(&b'/') && bytes.get(pos + 1) == Some(&b'*') {
            let start = pos;
            pos += 2;
            let mut depth = 1usize;
            while depth > 0 {
                match (bytes.get(pos), bytes.get(pos + 1)) {
                    (Some(b'/'), Some(b'*')) => {
                        pos += 2;
                        depth += 1;
                    }
                    (Some(b'*'), Some(b'/')) => {
                        pos += 2;
                        depth -= 1;
                    }
                    (Some(_), _) => pos += 1,
                    (None, _) => return Err(Error::new("unterminated block comment")),
                }
            }
            comments.push(Comment {
                text: source[start..pos].to_owned(),
                is_line: false,
                newlines_before: newlines,
            });
            newlines = 0;
            continue;
        }
        return Ok((comments, newlines, pos));
    }
}

fn scan_ident_end(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
        pos += 1;
    }
    pos
}

/// Mirrors the v2 lexer's `lex_number`.
fn scan_number_end(bytes: &[u8], mut pos: usize) -> usize {
    if bytes[pos] == b'0' && matches!(bytes.get(pos + 1), Some(b'x' | b'X')) {
        pos += 2;
        while pos < bytes.len() && (bytes[pos].is_ascii_hexdigit() || bytes[pos] == b'_') {
            pos += 1;
        }
        return pos;
    }
    let digits = |mut p: usize| {
        while p < bytes.len() && (bytes[p].is_ascii_digit() || bytes[p] == b'_') {
            p += 1;
        }
        p
    };
    pos = digits(pos);
    if bytes.get(pos) == Some(&b'.') && bytes.get(pos + 1).is_some_and(u8::is_ascii_digit) {
        pos = digits(pos + 1);
    }
    pos
}

fn scan_string_end(bytes: &[u8], mut pos: usize) -> Result<usize> {
    pos += 1;
    while let Some(&b) = bytes.get(pos) {
        match b {
            b'"' => return Ok(pos + 1),
            b'\\' => pos += 2,
            b'\n' => break,
            _ => pos += 1,
        }
    }
    Err(Error::new("unterminated string literal"))
}

/// Mirrors the v2 lexer's `lex_raw_air_block`: returns the index of the closing `}`.
fn scan_raw_body_end(bytes: &[u8], open: usize) -> Result<usize> {
    let mut pos = open + 1;
    let mut depth = 1usize;
    while let Some(&b) = bytes.get(pos) {
        match b {
            b'"' => {
                pos += 1;
                let mut escaped = false;
                while let Some(&sb) = bytes.get(pos) {
                    pos += 1;
                    if escaped {
                        escaped = false;
                    } else if sb == b'\\' {
                        escaped = true;
                    } else if sb == b'"' {
                        break;
                    }
                }
            }
            b'#' => {
                while let Some(&cb) = bytes.get(pos) {
                    pos += 1;
                    if cb == b'\n' {
                        break;
                    }
                }
            }
            b'/' if bytes.get(pos + 1) == Some(&b'/') => {
                while let Some(&cb) = bytes.get(pos) {
                    pos += 1;
                    if cb == b'\n' {
                        break;
                    }
                }
            }
            b'/' if bytes.get(pos + 1) == Some(&b'*') => {
                pos += 2;
                let mut cdepth = 1usize;
                while cdepth > 0 {
                    match (bytes.get(pos), bytes.get(pos + 1)) {
                        (Some(b'/'), Some(b'*')) => {
                            pos += 2;
                            cdepth += 1;
                        }
                        (Some(b'*'), Some(b'/')) => {
                            pos += 2;
                            cdepth -= 1;
                        }
                        (Some(_), _) => pos += 1,
                        (None, _) => {
                            return Err(Error::new(
                                "unterminated block comment inside 'raw { ... }'",
                            ));
                        }
                    }
                }
            }
            b'{' => {
                pos += 1;
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(pos);
                }
                pos += 1;
            }
            _ => pos += 1,
        }
    }
    Err(Error::new("unterminated 'raw { ... }' block"))
}

/// Length of the punctuation token at `pos` (longest match, as the v2 lexer does).
fn punct_len(bytes: &[u8], pos: usize) -> Result<usize> {
    let b = bytes[pos];
    let n1 = bytes.get(pos + 1).copied();
    let n2 = bytes.get(pos + 2).copied();
    let len = match (b, n1) {
        (b'.', Some(b'.')) => {
            if n2 == Some(b'=') {
                3
            } else {
                2
            }
        }
        (b':', Some(b':' | b'=')) => 2,
        (b'=', Some(b'=' | b'>')) => 2,
        (b'!' | b'<' | b'>' | b'+' | b'*' | b'/' | b'%' | b'^', Some(b'=')) => 2,
        (b'-', Some(b'>' | b'=')) => 2,
        (b'&', Some(b'&' | b'=')) => 2,
        (b'|', Some(b'|' | b'=')) => 2,
        (
            b'{' | b'}' | b'(' | b')' | b'[' | b']' | b';' | b',' | b'.' | b'@' | b'?' | b':'
            | b'=' | b'!' | b'<' | b'>' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^',
            _,
        ) => 1,
        _ => {
            return Err(Error::new(format!(
                "unexpected character {:?}",
                char::from(b)
            )));
        }
    };
    Ok(len)
}

// ---------------------------------------------------------------------------------------------
// Structure analysis.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// `{ ... }` statement / member / entry block.
    Block,
    /// `use a::{B, C}` import list (inline).
    UseBrace,
    /// `( ... )` / `[ ... ]` laid out on one line.
    Group,
    /// `( ... )` / `[ ... ]` laid out vertically.
    BrokenGroup,
}

impl Frame {
    fn indents(self) -> bool {
        matches!(self, Self::Block | Self::BrokenGroup)
    }
}

/// Keywords after which `(`/`[` get a space and `-`/`*`/`&` are unary.
fn is_space_keyword(s: &str) -> bool {
    matches!(
        s,
        "if" | "while"
            | "match"
            | "return"
            | "in"
            | "else"
            | "for"
            | "repeat"
            | "strings"
            | "flag_set"
            | "flag_clear"
            | "flag_test"
    )
}

/// Declaration keywords that group top-level items and object members.
fn member_keyword(s: &str) -> Option<&'static str> {
    Some(match s {
        "use" => "use",
        "const" => "const",
        "fn" => "fn",
        "template" => "template",
        "object" => "object",
        "enum" => "enum",
        "bitflags" => "bitflags",
        "extern" => "extern",
        "param" => "param",
        "state" => "state",
        "sound" => "sound",
        "slot" => "slot",
        "strings" => "strings",
        "bcon" => "bcon",
        "tree" => "tree",
        "init" | "main" | "load" | "reset" | "cleanup" => "lifecycle",
        "functional" => "functional",
        "interaction" => "interaction",
        "multi_tile" => "multi_tile",
        _ => return None,
    })
}

struct Analysis {
    /// Matching closer for each opener (and opener for each closer).
    partner: Vec<Option<usize>>,
    /// Frame kind for each `{`/`(`/`[` opener and its closer.
    frame: Vec<Option<Frame>>,
    generic_open: Vec<bool>,
    generic_close: Vec<bool>,
    closure_open: Vec<bool>,
    closure_close: Vec<bool>,
    attr_at: Vec<bool>,
    attr_close: Vec<bool>,
    sign_op: Vec<bool>,
    unary: Vec<bool>,
    /// Innermost frame (by opener index) enclosing each token.
    enclosing: Vec<Option<usize>>,
    /// A forced blank line before the member starting at this token.
    forced_blank: Vec<bool>,
}

fn analyze(toks: &[Tok]) -> Analysis {
    let n = toks.len();
    let text = |i: usize| toks[i].text.as_str();
    let is_punct = |i: usize, s: &str| toks[i].class == Class::Punct && toks[i].text == s;

    let mut partner = vec![None; n];
    let mut enclosing = vec![None; n];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..n {
        enclosing[i] = stack.last().copied();
        if toks[i].class != Class::Punct {
            continue;
        }
        match text(i) {
            "{" | "(" | "[" => stack.push(i),
            "}" | ")" | "]" => {
                if let Some(o) = stack.pop() {
                    partner[o] = Some(i);
                    partner[i] = Some(o);
                    enclosing[i] = stack.last().copied();
                }
            }
            _ => {}
        }
    }

    // Frame kinds.
    let mut frame = vec![None; n];
    // Process innermost-first so `BrokenGroup` can see nested non-empty blocks: iterate openers
    // in order of decreasing index (inner openers come later).
    let mut openers: Vec<usize> = (0..n)
        .filter(|&i| toks[i].class == Class::Punct && matches!(text(i), "{" | "(" | "["))
        .filter(|&i| partner[i].is_some())
        .collect();
    openers.reverse();
    let non_empty = |o: usize, c: usize| c > o + 1 || !toks[c].leading.is_empty();
    for &o in &openers {
        let c = partner[o].expect("filtered");
        let kind = if text(o) == "{" {
            if o > 0 && is_punct(o - 1, "::") {
                Frame::UseBrace
            } else {
                Frame::Block
            }
        } else if !non_empty(o, c) {
            Frame::Group
        } else {
            let mut broken = false;
            for k in o + 1..=c {
                if toks[k].newlines_before > 0 || !toks[k].leading.is_empty() {
                    broken = true;
                    break;
                }
                if toks[k].class == Class::Raw && toks[k].text.contains('\n') {
                    broken = true;
                    break;
                }
                if k < c
                    && frame[k] == Some(Frame::Block)
                    && partner[k].is_some_and(|kc| non_empty(k, kc))
                {
                    broken = true;
                    break;
                }
                if k < c && frame[k] == Some(Frame::BrokenGroup) {
                    broken = true;
                    break;
                }
            }
            if broken {
                Frame::BrokenGroup
            } else {
                Frame::Group
            }
        };
        frame[o] = Some(kind);
        frame[c] = Some(kind);
    }

    // Turbofish / `Result<...>` generics.
    let mut generic_open = vec![false; n];
    let mut generic_close = vec![false; n];
    for i in 1..n {
        if is_punct(i, "<")
            && (is_punct(i - 1, "::")
                || (toks[i - 1].class == Class::Ident
                    && matches!(text(i - 1), "Result" | "Option")))
        {
            let mut depth = 0usize;
            for j in i + 1..n {
                if is_punct(j, "<") {
                    depth += 1;
                } else if is_punct(j, ">") {
                    if depth == 0 {
                        generic_open[i] = true;
                        generic_close[j] = true;
                        break;
                    }
                    depth -= 1;
                } else if toks[j].class == Class::Punct && matches!(text(j), ";" | "{" | "}" | "=")
                {
                    break;
                }
            }
        }
    }

    // `|event|` animation handlers.
    let mut closure_open = vec![false; n];
    let mut closure_close = vec![false; n];
    for i in 1..n.saturating_sub(3) {
        if is_punct(i, "|")
            && is_punct(i - 1, ")")
            && toks[i + 1].class == Class::Ident
            && is_punct(i + 2, "|")
            && is_punct(i + 3, "{")
        {
            closure_open[i] = true;
            closure_close[i + 2] = true;
        }
    }

    // `@id(...)` / `@frame(...)` / `@guard_id(...)` attributes.
    let mut attr_at = vec![false; n];
    let mut attr_close = vec![false; n];
    for i in 0..n.saturating_sub(2) {
        if is_punct(i, "@")
            && toks[i + 1].class == Class::Ident
            && matches!(text(i + 1), "id" | "frame" | "guard_id")
            && is_punct(i + 2, "(")
            && let Some(c) = partner[i + 2]
        {
            attr_at[i] = true;
            attr_close[c] = true;
        }
    }

    // Advertisement deltas `advertises: [hunger +75]`.
    let mut sign_op = vec![false; n];
    for i in 1..n {
        if toks[i].class == Class::Punct
            && matches!(text(i), "+" | "-")
            && toks[i - 1].class == Class::Ident
            && let Some(g) = enclosing[i]
            && is_punct(g, "[")
            && g >= 2
            && is_punct(g - 1, ":")
            && toks[g - 2].class == Class::Ident
            && text(g - 2) == "advertises"
        {
            sign_op[i] = true;
        }
    }

    // Unary operators.
    let mut unary = vec![false; n];
    for i in 0..n {
        if toks[i].class != Class::Punct || !matches!(text(i), "-" | "*" | "&" | "!") {
            continue;
        }
        if text(i) == "!" {
            unary[i] = true;
            continue;
        }
        let operand_end = i > 0 && {
            let p = &toks[i - 1];
            match p.class {
                Class::Ident => !is_space_keyword(&p.text),
                Class::Number | Class::Str | Class::Color => true,
                Class::Raw => false,
                Class::Punct => matches!(p.text.as_str(), ")" | "]" | "?") || generic_close[i - 1],
            }
        };
        unary[i] = !operand_end;
    }

    let mut a = Analysis {
        partner,
        frame,
        generic_open,
        generic_close,
        closure_open,
        closure_close,
        attr_at,
        attr_close,
        sign_op,
        unary,
        enclosing,
        forced_blank: vec![false; n],
    };
    let mut forced = vec![false; n];
    scan_members(toks, &a, 0, n, &mut forced);
    a.forced_blank = forced;
    a
}

/// Walks the members of one container (`start..end`, exclusive) and marks forced blank lines.
fn scan_members(toks: &[Tok], a: &Analysis, start: usize, end: usize, forced: &mut [bool]) {
    let is_punct = |i: usize, s: &str| toks[i].class == Class::Punct && toks[i].text == s;
    let mut prev: Option<(&'static str, bool)> = None;
    let mut i = start;
    while i < end {
        let member_start = i;
        let kind = member_kind(toks, a, i);
        let mut ends_block = false;
        let mut j = i;
        let mut seen_container_body = false;
        let member_end = loop {
            if j >= end {
                break end - 1;
            }
            let t = &toks[j];
            if t.class == Class::Raw {
                j += 1;
                continue;
            }
            if t.class == Class::Punct && matches!(t.text.as_str(), "{" | "(" | "[") {
                let Some(c) = a.partner[j] else {
                    j += 1;
                    continue;
                };
                let non_empty_block =
                    a.frame[j] == Some(Frame::Block) && (c > j + 1 || !toks[c].leading.is_empty());
                if a.frame[j] == Some(Frame::Block)
                    && matches!(kind, "object" | "template")
                    && !seen_container_body
                {
                    seen_container_body = true;
                    scan_members(toks, a, j + 1, c, forced);
                }
                j = c;
                ends_block = non_empty_block && is_punct(j, "}");
                if is_punct(j, "}") {
                    match toks.get(j + 1) {
                        Some(next)
                            if next.class == Class::Punct
                                && matches!(next.text.as_str(), ";" | ",")
                                && j + 1 < end =>
                        {
                            break j + 1;
                        }
                        Some(next) if next.class == Class::Ident && next.text == "else" => {
                            j += 1;
                            continue;
                        }
                        _ => break j,
                    }
                }
                j += 1;
                continue;
            }
            if t.class == Class::Punct && matches!(t.text.as_str(), ";" | ",") {
                break j;
            }
            ends_block = false;
            j += 1;
        };
        if let Some((pkind, pblock)) = prev {
            // Plain fields (`guid: ...;`, template bindings) group however the author chose.
            let kind_change = pkind != kind && pkind != "field" && kind != "field";
            forced[member_start] = pblock || ends_block || kind_change;
        }
        prev = Some((kind, ends_block));
        i = member_end + 1;
    }
}

fn member_kind(toks: &[Tok], a: &Analysis, mut i: usize) -> &'static str {
    loop {
        let Some(t) = toks.get(i) else {
            return "field";
        };
        if t.class == Class::Punct && t.text == "@" && a.attr_at[i] {
            // Skip `@name(...)`.
            match a.partner.get(i + 2).copied().flatten() {
                Some(c) => {
                    i = c + 1;
                    continue;
                }
                None => return "field",
            }
        }
        if t.class == Class::Ident && t.text == "pub" {
            i += 1;
            continue;
        }
        if t.class == Class::Ident {
            return member_keyword(&t.text).unwrap_or("field");
        }
        return "field";
    }
}

// ---------------------------------------------------------------------------------------------
// Printer.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sep {
    None,
    Space,
    Newline,
    /// A newline if the source had one here, otherwise a space.
    Preserve,
}

struct Printer<'a> {
    toks: &'a [Tok],
    trailing: &'a [Comment],
    a: Analysis,
    out: String,
    /// Open indenting frames.
    indent: usize,
    at_line_start: bool,
    /// The last emitted item opened an indenting frame (no blank line may follow).
    after_opener: bool,
}

impl<'a> Printer<'a> {
    fn new(s: &'a Scanned) -> Self {
        Self {
            toks: &s.tokens,
            trailing: &s.trailing,
            a: analyze(&s.tokens),
            out: String::new(),
            indent: 0,
            at_line_start: true,
            after_opener: false,
        }
    }

    fn text(&self, i: usize) -> &str {
        &self.toks[i].text
    }

    fn frame(&self, i: usize) -> Option<Frame> {
        self.a.frame[i]
    }

    fn is_opener(&self, i: usize) -> bool {
        self.toks[i].class == Class::Punct
            && matches!(self.text(i), "{" | "(" | "[")
            && self.a.partner[i].is_some()
    }

    fn is_closer(&self, i: usize) -> bool {
        self.toks[i].class == Class::Punct
            && matches!(self.text(i), "}" | ")" | "]")
            && self.a.partner[i].is_some()
    }

    /// Frame enclosing token `i` (not counting `i` itself if it is an opener).
    fn inner_frame(&self, i: usize) -> Option<Frame> {
        self.a.enclosing[i].and_then(|o| self.a.frame[o])
    }

    fn newline(&mut self, blank: bool) {
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        if self.out.is_empty() {
            self.at_line_start = true;
            return;
        }
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        if blank && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
        self.at_line_start = true;
    }

    fn write(&mut self, s: &str) {
        if self.at_line_start {
            for _ in 0..self.indent {
                self.out.push_str("    ");
            }
            self.at_line_start = false;
        }
        self.out.push_str(s);
        self.after_opener = false;
    }

    fn space(&mut self) {
        if !self.at_line_start && !self.out.ends_with(' ') {
            self.out.push(' ');
        }
    }

    /// Separator required between tokens `i - 1` and `i` by the layout rules.
    fn sep(&self, i: usize) -> Sep {
        let p = i - 1;
        let pt = self.text(p);
        let nt = self.text(i);
        let p_punct = self.toks[p].class == Class::Punct;
        let n_punct = self.toks[i].class == Class::Punct;

        // Raw blocks are whole statements.
        if self.toks[p].class == Class::Raw {
            return Sep::Newline;
        }

        // Block braces.
        if p_punct && pt == "{" && self.frame(p) == Some(Frame::Block) {
            return if self.a.partner[p] == Some(i) {
                Sep::None
            } else {
                Sep::Newline
            };
        }
        if n_punct && nt == "}" && self.frame(i) == Some(Frame::Block) {
            return Sep::Newline;
        }
        // Broken groups.
        if self.is_opener(p) && self.frame(p) == Some(Frame::BrokenGroup) {
            return Sep::Newline;
        }
        if self.is_closer(i) && self.frame(i) == Some(Frame::BrokenGroup) {
            return Sep::Newline;
        }
        if p_punct && pt == "}" && self.frame(p) == Some(Frame::Block) {
            if self.toks[i].class == Class::Ident && nt == "else" {
                return Sep::Space;
            }
            if n_punct && matches!(nt, ";" | "," | ")" | "]" | "." | "?") {
                return Sep::None;
            }
            return Sep::Newline;
        }
        if p_punct && pt == ";" {
            return match self.inner_frame(p) {
                None | Some(Frame::Block) => Sep::Newline,
                _ => Sep::Space,
            };
        }
        if p_punct && pt == "," {
            return match self.inner_frame(p) {
                Some(Frame::Block | Frame::BrokenGroup) => Sep::Newline,
                _ => Sep::Space,
            };
        }
        if self.toks[i].class == Class::Raw {
            return Sep::Newline;
        }
        if self.a.attr_close[p] {
            return Sep::Preserve;
        }
        // Use-import braces.
        if (self.frame(i) == Some(Frame::UseBrace) || self.frame(p) == Some(Frame::UseBrace))
            && ((self.is_opener(p) && self.frame(p) == Some(Frame::UseBrace))
                || (self.is_closer(i) && self.frame(i) == Some(Frame::UseBrace))
                || (self.is_opener(i) && self.frame(i) == Some(Frame::UseBrace)))
        {
            return Sep::None;
        }
        // Closures `|event|`.
        if self.a.closure_open[i] {
            return Sep::Space;
        }
        if self.a.closure_open[p] || self.a.closure_close[i] {
            return Sep::None;
        }
        if self.a.closure_close[p] {
            return Sep::Space;
        }
        // Generics.
        if self.a.generic_open[i] || self.a.generic_open[p] || self.a.generic_close[i] {
            return Sep::None;
        }
        // Attributes.
        if self.a.attr_at[p] {
            return Sep::None;
        }
        // Tight punctuation.
        if n_punct && matches!(nt, "," | ";" | "." | "?" | ":" | "::" | ".." | "..=") {
            return Sep::None;
        }
        if n_punct && matches!(nt, ")" | "]") {
            return Sep::None;
        }
        if p_punct && matches!(pt, "(" | "[" | "." | "::" | ".." | "..=") {
            return Sep::None;
        }
        if p_punct && (self.a.unary[p] || self.a.sign_op[p]) {
            return Sep::None;
        }
        if n_punct && matches!(nt, "(" | "[") {
            let tight = match self.toks[p].class {
                Class::Ident => !is_space_keyword(pt),
                Class::Number => nt == "[",
                Class::Punct => matches!(pt, ")" | "]" | "?") || self.a.generic_close[p],
                _ => false,
            };
            return if tight { Sep::None } else { Sep::Space };
        }
        Sep::Space
    }

    fn print(mut self) -> String {
        let n = self.toks.len();
        for i in 0..n {
            let tsep = if i == 0 { Sep::Newline } else { self.sep(i) };
            let forced_blank = self.a.forced_blank[i];
            let closer = self.is_closer(i);
            let tok_newlines = self.toks[i].newlines_before;
            let leading = self.toks[i].leading.clone();
            let emitted_comment =
                self.emit_comments(&leading, forced_blank && tsep == Sep::Newline);

            // Dedent before an indenting closer.
            if closer && self.frame(i).is_some_and(Frame::indents) {
                self.indent = self.indent.saturating_sub(1);
            }

            match emitted_comment {
                Some(last_is_line) => {
                    if last_is_line || tok_newlines > 0 || tsep == Sep::Newline {
                        let blank = tok_newlines >= 2 && !closer && !self.after_opener;
                        self.newline(blank);
                    } else if tsep != Sep::None || !matches!(self.text(i), ")" | "]" | "," | ";") {
                        self.space();
                    }
                }
                None => match tsep {
                    Sep::None => {}
                    Sep::Space => self.space(),
                    Sep::Preserve => {
                        if tok_newlines > 0 {
                            self.newline(false);
                        } else {
                            self.space();
                        }
                    }
                    Sep::Newline => {
                        let blank = i > 0
                            && (forced_blank || tok_newlines >= 2)
                            && !closer
                            && !self.after_opener;
                        self.newline(blank);
                    }
                },
            }

            if self.toks[i].class == Class::Raw {
                self.write_raw(i);
            } else {
                let t = self.toks[i].text.clone();
                self.write(&t);
            }

            if self.is_opener(i) && self.frame(i).is_some_and(Frame::indents) {
                self.indent += 1;
                self.after_opener = true;
            }
        }
        let trailing = self.trailing.to_vec();
        if !trailing.is_empty() {
            self.emit_comments(&trailing, false);
        }
        self.newline(false);
        if self.out.is_empty() {
            return String::new();
        }
        while self.out.ends_with("\n\n") {
            self.out.pop();
        }
        self.out
    }

    /// Emits comments preceding a token; returns `Some(last_is_line)` if any were written.
    fn emit_comments(&mut self, comments: &[Comment], forced_blank: bool) -> Option<bool> {
        let mut last_is_line = None;
        let mut first_own_line = true;
        for c in comments {
            let trailing = c.newlines_before == 0 && !self.at_line_start && !self.out.is_empty();
            let trailing = trailing && last_is_line != Some(true);
            if trailing {
                if !(self.out.ends_with('(') || self.out.ends_with('[')) {
                    self.space();
                }
                self.write(&c.text);
            } else if last_is_line == Some(false) && c.newlines_before == 0 {
                // Block comment followed by another comment on the same line.
                self.space();
                self.write(&c.text);
            } else {
                let blank = !self.out.is_empty()
                    && !self.after_opener
                    && (c.newlines_before >= 2 || (first_own_line && forced_blank));
                self.newline(blank);
                self.write(&c.text);
                first_own_line = false;
            }
            last_is_line = Some(c.is_line);
            if c.is_line {
                self.newline(false);
            }
        }
        last_is_line
    }

    fn write_raw(&mut self, i: usize) {
        let body = self.toks[i].text.clone();
        let prefix = self.toks[i].raw_prefix.clone();
        self.write("raw");
        if !prefix.is_empty() {
            self.write(" ");
            self.write(&prefix);
        }
        self.write(" {");
        if !body.contains('\n') {
            let t = body.trim();
            if t.is_empty() {
                self.write("}");
            } else {
                self.write(" ");
                self.write(t);
                self.write(" }");
            }
            return;
        }
        let lines: Vec<&str> = body.lines().map(str::trim_end).collect();
        // The first line follows `{` directly; the others keep their relative indentation.
        let min_indent = lines
            .iter()
            .skip(1)
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);
        let mut rendered: Vec<String> = Vec::new();
        for (k, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                rendered.push(String::new());
            } else if k == 0 {
                rendered.push(line.trim().to_owned());
            } else {
                let lead = line.len() - line.trim_start().len();
                let rel = lead.saturating_sub(min_indent);
                rendered.push(format!("{}{}", &line[lead - rel..lead], line.trim_start()));
            }
        }
        // Drop leading/trailing empty lines and collapse runs of empty lines.
        while rendered.first().is_some_and(String::is_empty) {
            rendered.remove(0);
        }
        while rendered.last().is_some_and(String::is_empty) {
            rendered.pop();
        }
        self.indent += 1;
        let mut prev_empty = false;
        for line in rendered {
            if line.is_empty() {
                if !prev_empty {
                    self.newline(true);
                }
                prev_empty = true;
                continue;
            }
            prev_empty = false;
            self.newline(false);
            self.write(&line);
        }
        self.indent -= 1;
        self.newline(false);
        self.write("}");
    }
}

#[cfg(test)]
mod tests {
    use super::format_v2_source;

    fn fmt(s: &str) -> String {
        let once = format_v2_source(s).unwrap_or_else(|e| panic!("format failed: {e}\n{s}"));
        let twice = format_v2_source(&once).expect("reformat");
        assert_eq!(once, twice, "not idempotent:\n{once}");
        once
    }

    #[test]
    fn reflows_object_members_and_statements() {
        let src = "object \"Synth\"{guid:0x7E00_0001;price:10;\nparam a:i16=-5;state b:bool=false;\ninteraction \"Go\"{autonomy:5,advertises:[fun +5],guard(sim)=>!self.b,run(sim){if self.b{return false;}else{self.b=true;}}}}";
        let out = fmt(src);
        let expected = "object \"Synth\" {\n    guid: 0x7E00_0001;\n    price: 10;\n    param a: i16 = -5;\n\n    state b: bool = false;\n\n    interaction \"Go\" {\n        autonomy: 5,\n        advertises: [fun +5],\n        guard(sim) => !self.b,\n        run(sim) {\n            if self.b {\n                return false;\n            } else {\n                self.b = true;\n            }\n        }\n    }\n}\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn keeps_comments_and_raw_bodies() {
        let src = "// header\n\n/// doc\nobject \"S\" { // trailing\n    guid: 1; /* inline */\n    init {\n        raw {\n              x := 1;\n                y := 2; # air comment\n        }\n        /* own line */\n    }\n}\n";
        let out = fmt(src);
        for c in [
            "// header",
            "/// doc",
            "// trailing",
            "/* inline */",
            "# air comment",
            "/* own line */",
        ] {
            assert!(out.contains(c), "lost {c}:\n{out}");
        }
        assert!(out.contains("        raw {\n            x := 1;\n              y := 2; # air comment\n        }\n"), "{out}");
    }
}
