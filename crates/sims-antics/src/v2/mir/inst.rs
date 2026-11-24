//! Typed view of one MIR instruction (`InstKind`: declaration, store, exit, other) and operand-level
//! facts: identifiers, local-slot names, scope stores, plain stores and text formatting.

use super::*;

// ---------------------------------------------------------------------------
// Line classification
// ---------------------------------------------------------------------------

pub(crate) const ASSIGN_OPS: &[&str] = &[
    ":=",
    "+=",
    "-=",
    "*=",
    "/=",
    "%=",
    "&=",
    "|=",
    "^=",
    "flag_set",
    "flag_clear",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InstKind<'a> {
    Let {
        name: &'a str,
        init: Option<&'a str>,
    },
    Assign {
        lhs: &'a str,
        op: &'a str,
        rhs: &'a str,
    },
    Return,
    Break,
    Continue,
    Other,
}

pub(crate) fn classify(line: &str) -> InstKind<'_> {
    let body = line.strip_suffix(';').unwrap_or(line).trim();
    if body == "break" {
        return InstKind::Break;
    }
    if body == "continue" {
        return InstKind::Continue;
    }
    if body == "return" || body.starts_with("return ") {
        return InstKind::Return;
    }
    if body.contains('"') {
        return InstKind::Other;
    }
    if let Some(rest) = body.strip_prefix("let ") {
        return match rest.split_once(" = ") {
            Some((name, init)) if is_ident(name.trim()) => InstKind::Let {
                name: name.trim(),
                init: Some(init.trim()),
            },
            None if is_ident(rest.trim()) => InstKind::Let {
                name: rest.trim(),
                init: None,
            },
            _ => InstKind::Other,
        };
    }
    let mut parts = body.splitn(3, ' ');
    if let (Some(lhs), Some(op), Some(rhs)) = (parts.next(), parts.next(), parts.next())
        && ASSIGN_OPS.contains(&op)
        && !lhs.contains('(')
    {
        return InstKind::Assign {
            lhs,
            op,
            rhs: rhs.trim(),
        };
    }
    InstKind::Other
}

pub(crate) fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Identifier tokens outside string literals.
pub(crate) fn idents(text: &str) -> Vec<&str> {
    let stripped = strip_strings(text);
    let mut out = Vec::new();
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            // A field after `.` is not a variable. Same byte offsets in the original: stripping
            // only blanks string contents.
            if !follows_dot(bytes, start) {
                out.push(&text[start..i]);
            }
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Whether the token at `start` is a field (`.name`, spaces allowed before the name).
pub(crate) fn follows_dot(bytes: &[u8], start: usize) -> bool {
    bytes[..start]
        .iter()
        .rev()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|&b| b == b'.')
}

/// Opcodes whose VM handlers read no operand (find best interaction, grab, drop, gosub found
/// action), as v2 emits them.
pub(crate) const OPERAND_FREE_OPS: &[&str] = &["0x0003", "0x0004", "0x0005", "0x001e"];

/// Whether a (string-stripped) line can name a local slot by number, which renaming and
/// coalescing named locals would silently retarget: a raw `op` (its operands may encode a
/// local), or `scope(N, …)` whose scope is the locals scope (25) or not a plain literal.
pub(crate) fn names_local_slot(stripped: &str) -> bool {
    // The operand-free primitives v2 builtins spell as raw ops name no local.
    let mut cleaned = stripped.to_owned();
    for opcode in OPERAND_FREE_OPS {
        cleaned = cleaned.replace(&format!("op {opcode} (0, 0, 0, 0, 0, 0, 0, 0)"), "");
    }
    let stripped = cleaned.as_str();
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &stripped[start..i];
            // A raw op is `op <opcode> (...)`; `op:` is a primitive's named argument.
            if word == "op"
                && !follows_dot(bytes, start)
                && !stripped[i..].trim_start().starts_with(':')
            {
                return true;
            }
            if word == "scope" && stripped[i..].trim_start().starts_with('(') {
                let args = stripped[i..].trim_start()[1..].trim_start();
                let first = args.split([',', ')']).next().unwrap_or("").trim();
                let scope = first.parse::<i64>().ok().or_else(|| {
                    first
                        .strip_prefix("0x")
                        .and_then(|h| i64::from_str_radix(h, 16).ok())
                });
                if scope.is_none_or(|n| n == 25) {
                    return true;
                }
            }
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// Replaces identifier tokens (outside string literals) per `map`.
pub(crate) fn rename_idents(text: &str, map: &BTreeMap<String, String>) -> String {
    let stripped = strip_strings(text);
    let bytes = stripped.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if !follows_dot(bytes, start)
                && let Some(to) = map.get(&text[start..i])
            {
                out.push_str(&text[last..start]);
                out.push_str(to);
                last = i;
            }
        } else if c.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// An operand whose read cannot fault and has no side effects: a named local (v2 writes every
/// named local before reading it), an integer literal, `stack_obj`, or `my.id`. Cells of
/// objects, temps, params, and globals stay: their reads can fault (an index past the data, or
/// past the frame), and so can an explicit `local[N]` the tree never wrote.
pub(crate) fn is_safe_operand(s: &str) -> bool {
    is_ident(s) || s.parse::<i32>().is_ok() || s == "my.id"
}

/// `scope(S, I) op rhs;` with a call-free `rhs`: an expression node on a raw scope cell.
pub(crate) fn is_scope_store(line: &str) -> bool {
    let stripped = strip_strings(line);
    let Some(rest) = stripped.strip_prefix("scope(") else {
        return false;
    };
    let Some((_, after)) = rest.split_once(')') else {
        return false;
    };
    let mut parts = after.trim_start().splitn(2, ' ');
    matches!((parts.next(), parts.next()), (Some(op), Some(rhs))
        if ASSIGN_OPS.contains(&op) && !rhs.contains('(') && rhs.trim_end().ends_with(';'))
}

pub(crate) fn stack_obj_store(line: &str) -> Option<&str> {
    match classify(line) {
        InstKind::Assign {
            lhs: "stack_obj",
            op: ":=",
            rhs,
        } => Some(rhs),
        _ => None,
    }
}

/// A plain store `(let) t = e;` or `t := e;`, as `(t, e, is_let)`.
pub(crate) fn plain_store(line: &str) -> Option<(&str, &str, bool)> {
    match classify(line) {
        InstKind::Let {
            name,
            init: Some(init),
        } => Some((name, init, true)),
        InstKind::Assign { lhs, op: ":=", rhs } => Some((lhs, rhs, false)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Liveness and coalescing
// ---------------------------------------------------------------------------

/// Whether a tree body runs a primitive that formats text (see [`formats_text`]).
pub(crate) fn body_formats_text(body: &str) -> bool {
    body.lines().any(formats_text)
}

/// Whether `text` runs a primitive that formats text (Dialog, Action String), which reads frame
/// locals by number through `$Local:N`, `$TimeLocal:N` and similar tokens.
pub(crate) fn formats_text(text: &str) -> bool {
    let stripped = strip_strings(text);
    ["dialog(", "action_string("].iter().any(|prim| {
        stripped.match_indices(prim).any(|(at, _)| {
            !stripped[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        })
    })
}

pub(crate) fn nodes_format_text(nodes: &[Stmt]) -> bool {
    nodes.iter().any(|n| match n {
        Stmt::Inst(l) => formats_text(l),
        Stmt::If { arms, else_body } => {
            arms.iter()
                .any(|(c, body)| formats_text(c) || nodes_format_text(body))
                || else_body.as_deref().is_some_and(nodes_format_text)
        }
        Stmt::While { cond, body } => {
            cond.as_deref().is_some_and(formats_text) || nodes_format_text(body)
        }
    })
}
