//! `.air` body text <-> MIR: parsing a lowered tree body into `Stmt`s (`None` outside the modelled
//! subset), printing it back, and text-level helpers (string blanking, statement splitting,
//! positional branch targets).

use super::*;

/// Whether a node graph names a branch target by position (`-> true: 3`, `raw(1)`), which
/// deleting a node would shift. Labels and `true` / `false` / `error` do not move.
///
/// A target list runs from `->` to its `;`, which may be on a later line.
pub(crate) fn has_positional_targets(body: &str) -> bool {
    let code: String = body
        .lines()
        .map(|l| {
            let stripped = strip_strings(l);
            match stripped.find("//") {
                Some(at) => stripped[..at].to_owned(),
                None => stripped,
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    code.split([';', '{', '}']).any(|stmt| {
        let Some((_, targets)) = stmt.split_once("->") else {
            return false;
        };
        targets.split(',').any(|t| {
            let target = t.split_once(':').map_or(t, |(_, v)| v).trim();
            target.starts_with("raw") || target.starts_with(|c: char| c.is_ascii_digit())
        })
    })
}

/// The body with one statement per line, for analysis only: `a; b;` and `if c { a; }` written
/// on one line (as raw blocks often are) are split, `}` and a following `else` rejoin, and `//`
/// comments go.
pub(crate) fn one_statement_per_line(body: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    for line in body.lines() {
        let mut cur = String::new();
        let (mut in_str, mut escaped) = (false, false);
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if in_str {
                cur.push(c);
                match c {
                    _ if escaped => escaped = false,
                    '\\' => escaped = true,
                    '"' => in_str = false,
                    _ => {}
                }
                continue;
            }
            match c {
                '"' => {
                    in_str = true;
                    cur.push(c);
                }
                '/' if chars.peek() == Some(&'/') => break,
                ';' => {
                    cur.push(';');
                    parts.push(std::mem::take(&mut cur));
                }
                '{' => {
                    let head = cur.trim().to_owned();
                    parts.push(if head.is_empty() {
                        "{".to_owned()
                    } else {
                        format!("{head} {{")
                    });
                    cur.clear();
                }
                '}' => {
                    parts.push(std::mem::take(&mut cur));
                    parts.push("}".to_owned());
                }
                _ => cur.push(c),
            }
        }
        parts.push(cur);
    }
    let mut out: Vec<String> = Vec::new();
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if p.starts_with("else") && out.last().is_some_and(|l| l == "}") {
            let last = out.last_mut().expect("checked");
            last.push(' ');
            last.push_str(p);
        } else {
            out.push(p.to_owned());
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// `a, b(c, d), e` → `["a", " b(c, d)", " e"]`: commas outside brackets and strings.
pub(crate) fn split_top_level(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut in_str, mut escaped) = (0i32, 0usize, false, false);
    for (i, c) in text.char_indices() {
        if in_str {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&text[start..]);
    out
}

/// A decimal or `0x` integer.
pub(crate) fn parse_int(text: &str) -> Option<i64> {
    let t = text.trim();
    match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => t.parse().ok(),
    }
}

// ---------------------------------------------------------------------------
// Parsing and printing
// ---------------------------------------------------------------------------

/// The line with string literal contents blanked, for structural checks and token scans.
pub(crate) fn strip_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_str = false;
    let mut escaped = false;
    for c in line.chars() {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                out.push('"');
                continue;
            }
            // Same byte width, so offsets into the stripped text index the original.
            for _ in 0..c.len_utf8() {
                out.push(' ');
            }
        } else {
            if c == '"' {
                in_str = true;
            }
            out.push(c);
        }
    }
    out
}

pub(crate) fn parse_body(body: &str) -> Option<Vec<Stmt>> {
    let lines: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut pos = 0;
    let (nodes, end) = parse_block(&lines, &mut pos)?;
    if end.is_some() || pos != lines.len() {
        return None;
    }
    Some(nodes)
}

/// How a nested block ended.
pub(crate) enum BlockEnd {
    Close,
    ElseIf(String),
    Else,
}

/// Parses statements until a closing line; returns them with the closing kind (`None` at EOF).
pub(crate) fn parse_block(
    lines: &[&str],
    pos: &mut usize,
) -> Option<(Vec<Stmt>, Option<BlockEnd>)> {
    let mut nodes = Vec::new();
    while *pos < lines.len() {
        let line = lines[*pos];
        let s = strip_strings(line);
        *pos += 1;
        if s.contains("//")
            || s.contains("/*")
            || s.contains("*/")
            || s.contains("->")
            || s.contains('#')
            || names_local_slot(&s)
        {
            return None;
        }
        if s == "}" {
            return Some((nodes, Some(BlockEnd::Close)));
        }
        if s == "} else {" {
            return Some((nodes, Some(BlockEnd::Else)));
        }
        if let Some(cond) = header(&s, line, "} else if ") {
            return Some((nodes, Some(BlockEnd::ElseIf(cond))));
        }
        if let Some(cond) = header(&s, line, "if ") {
            let mut arms = Vec::new();
            let mut else_body = None;
            let mut cond = cond;
            loop {
                let (body, end) = parse_block(lines, pos)?;
                arms.push((cond, body));
                match end? {
                    BlockEnd::Close => break,
                    BlockEnd::ElseIf(c) => cond = c,
                    BlockEnd::Else => {
                        let (body, end) = parse_block(lines, pos)?;
                        if !matches!(end?, BlockEnd::Close) {
                            return None;
                        }
                        else_body = Some(body);
                        break;
                    }
                }
            }
            nodes.push(Stmt::If { arms, else_body });
            continue;
        }
        if let Some(cond) = header(&s, line, "while ") {
            let (body, end) = parse_block(lines, pos)?;
            if !matches!(end?, BlockEnd::Close) {
                return None;
            }
            // `while true` is `.air`'s condition-free loop.
            let cond = (cond != "true").then_some(cond);
            nodes.push(Stmt::While { cond, body });
            continue;
        }
        // One simple statement: exactly one `;`, at the end, and no braces.
        if !s.ends_with(';') || s.matches(';').count() != 1 || s.contains('{') || s.contains('}') {
            return None;
        }
        nodes.push(Stmt::Inst(line.to_owned()));
    }
    Some((nodes, None))
}

/// `kw COND {` → `COND` (taken from the original line), checking the stripped form.
pub(crate) fn header(stripped: &str, line: &str, kw: &str) -> Option<String> {
    let rest = stripped.strip_prefix(kw)?.strip_suffix(" {")?;
    if rest.contains(';') || rest.contains('{') || rest.contains('}') {
        return None;
    }
    Some(line[kw.len()..line.len() - 2].to_owned())
}

pub(crate) fn print_nodes(nodes: &[Stmt], depth: usize, out: &mut String) {
    let pad = |out: &mut String, d: usize| {
        for _ in 0..d {
            out.push_str("    ");
        }
    };
    for n in nodes {
        match n {
            Stmt::Inst(l) => {
                pad(out, depth);
                out.push_str(l);
                out.push('\n');
            }
            Stmt::If { arms, else_body } => {
                for (i, (cond, body)) in arms.iter().enumerate() {
                    pad(out, depth);
                    if i == 0 {
                        out.push_str(&format!("if {cond} {{\n"));
                    } else {
                        out.push_str(&format!("}} else if {cond} {{\n"));
                    }
                    print_nodes(body, depth + 1, out);
                }
                if let Some(body) = else_body {
                    pad(out, depth);
                    out.push_str("} else {\n");
                    print_nodes(body, depth + 1, out);
                }
                pad(out, depth);
                out.push_str("}\n");
            }
            Stmt::While { cond, body } => {
                pad(out, depth);
                match cond {
                    Some(c) => out.push_str(&format!("while {c} {{\n")),
                    None => out.push_str("while true {\n"),
                }
                print_nodes(body, depth + 1, out);
                pad(out, depth);
                out.push_str("}\n");
            }
        }
    }
}
