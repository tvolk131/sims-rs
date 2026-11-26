//! Option bindings read in place: the value of `if let Some(v) = f(..)` and
//! `let Some(v) = f(..) else { .. };` is the temp the call's true exit left it in, so while
//! nothing can write that temp, `v` is the temp itself and needs no frame word.

use super::*;

/// For each binding in `aliases` (lowering's `let v = temp[N];` copies of an option call's
/// result), reads `temp[N]` in place of `v` and drops the copy, when every read of `v` comes
/// before anything that may write `temp[N]`:
///
/// - `v` is never written, and every statement or condition that reads it is an Expression
///   (no primitive or call), so the operand stays a plain value read;
/// - on each path from the copy to a read, no statement or condition may write `temp[N]`: a
///   primitive or call may, as may an Expression whose target is `temp[N]` (or a
///   `stack.temp`, or a `scope` cell that may be the temps);
/// - a loop that may write `temp[N]` reads no `v`, since a later turn would see the write;
/// - `N` is a temp the VM always has (0 to 7), so the dropped copy could not fault, and the
///   node after the copy does not read the last result, which the copy set.
///
/// All or nothing per binding: where a read is not covered, `v` keeps its local (the copy and
/// its frame word). Bodies outside the modelled subset are returned unchanged, and the caller
/// runs this only where dropping an Expression node is unobservable (no kept layout, main's
/// rules or text that may read frame words).
pub(crate) fn alias_option_temps(body: &str, aliases: &BTreeSet<String>) -> String {
    if aliases.is_empty() {
        return body.to_owned();
    }
    let Some(mut nodes) = parse_body(body) else {
        return body.to_owned();
    };
    let before = nodes.clone();
    for name in aliases {
        let total = count_mentions(&nodes, name);
        for_each_block(&mut nodes, &mut |block| alias_in_block(block, name, total));
    }
    if nodes == before {
        return body.to_owned();
    }
    let mut out = String::with_capacity(body.len());
    print_nodes(&nodes, 1, &mut out);
    out
}

/// The temp `let NAME = temp[N];` copies, for `N` in 0..=7.
fn copied_temp(line: &str, name: &str) -> Option<u8> {
    let InstKind::Let {
        name: n,
        init: Some(init),
    } = classify(line)
    else {
        return None;
    };
    let index = init.strip_prefix("temp[")?.strip_suffix(']')?;
    let index: u8 = index.trim().parse().ok()?;
    (n == name && index <= 7).then_some(index)
}

fn alias_in_block(block: &mut [Stmt], name: &str, total: usize) {
    let Some((at, temp)) = block.iter().enumerate().find_map(|(i, n)| match n {
        Stmt::Inst(l) => copied_temp(l, name).map(|t| (i, t)),
        _ => None,
    }) else {
        return;
    };
    let rest = &block[at + 1..];
    // Every other mention follows the copy in its own block (lowering scopes the binding so).
    if count_mentions(rest, name) + 1 != total
        || !next_ignores_last_result(rest)
        || scan(rest, name, temp, true).is_none()
    {
        return;
    }
    let map = BTreeMap::from([(name.to_owned(), format!("temp[{temp}]"))]);
    rename_all(&mut block[at + 1..], &map);
    block[at] = Stmt::Inst(format!("let {name};"));
}

/// Lines and conditions naming `name`.
fn count_mentions(nodes: &[Stmt], name: &str) -> usize {
    let named = |text: &str| usize::from(idents(text).contains(&name));
    nodes
        .iter()
        .map(|n| match n {
            Stmt::Inst(l) => named(l),
            Stmt::If { arms, else_body } => {
                arms.iter()
                    .map(|(c, b)| named(c) + count_mentions(b, name))
                    .sum::<usize>()
                    + else_body.as_deref().map_or(0, |b| count_mentions(b, name))
            }
            Stmt::While { cond, body } => {
                cond.as_deref().map_or(0, named) + count_mentions(body, name)
            }
        })
        .sum()
}

fn rename_all(nodes: &mut [Stmt], map: &BTreeMap<String, String>) {
    for n in nodes {
        match n {
            Stmt::Inst(l) => *l = rename_idents(l, map),
            Stmt::If { arms, else_body } => {
                for (c, b) in arms.iter_mut() {
                    *c = rename_idents(c, map);
                    rename_all(b, map);
                }
                if let Some(b) = else_body {
                    rename_all(b, map);
                }
            }
            Stmt::While { cond, body } => {
                if let Some(c) = cond {
                    *c = rename_idents(c, map);
                }
                rename_all(body, map);
            }
        }
    }
}

/// Walks `nodes` entered with `clean` (no write to `temp[temp]` yet on any path): `None` when a
/// read of `name` is not covered, else whether every path falling off the end is still clean.
fn scan(nodes: &[Stmt], name: &str, temp: u8, mut clean: bool) -> Option<bool> {
    for n in nodes {
        match n {
            Stmt::Inst(l) => {
                if !read_ok(l, name, clean) {
                    return None;
                }
                if !matches!(
                    classify(l),
                    InstKind::Return | InstKind::Break | InstKind::Continue
                ) && may_write_temp(l, temp)
                {
                    clean = false;
                }
            }
            Stmt::If { arms, else_body } => {
                let mut tested = clean;
                let mut out = true;
                for (cond, body) in arms {
                    if !read_ok(cond, name, tested) {
                        return None;
                    }
                    tested &= !may_write_temp(cond, temp);
                    out &= scan(body, name, temp, tested)?;
                }
                out &= match else_body {
                    Some(body) => scan(body, name, temp, tested)?,
                    None => tested,
                };
                clean = out;
            }
            Stmt::While { cond, body } => {
                let writes = cond.as_deref().is_some_and(|c| may_write_temp(c, temp))
                    || writes_temp_anywhere(body, temp);
                if writes {
                    let read = cond.as_deref().is_some_and(|c| idents(c).contains(&name))
                        || count_mentions(body, name) > 0;
                    if read {
                        return None;
                    }
                    clean = false;
                } else {
                    if let Some(c) = cond
                        && !read_ok(c, name, clean)
                    {
                        return None;
                    }
                    scan(body, name, temp, clean)?;
                }
            }
        }
    }
    Some(clean)
}

/// Whether `text` may mention `name` here: it does not, or it reads it as a plain operand of an
/// Expression while the temp still holds the copied value.
fn read_ok(text: &str, name: &str, clean: bool) -> bool {
    if !idents(text).contains(&name) {
        return true;
    }
    let stripped = strip_strings(text);
    clean
        && callees(&stripped).all(|c| c == "scope")
        && !matches!(classify(text), InstKind::Let { name: n, .. } | InstKind::Assign { lhs: n, .. } if n == name)
        && !names_as_target(text, &|kinds, i| {
            matches!(kinds[i], crate::lexer::TokenKind::Ident(w) if w == name).then_some(i)
        })
}

fn writes_temp_anywhere(nodes: &[Stmt], temp: u8) -> bool {
    nodes.iter().any(|n| match n {
        Stmt::Inst(l) => may_write_temp(l, temp),
        Stmt::If { arms, else_body } => {
            arms.iter()
                .any(|(c, b)| may_write_temp(c, temp) || writes_temp_anywhere(b, temp))
                || else_body
                    .as_deref()
                    .is_some_and(|b| writes_temp_anywhere(b, temp))
        }
        Stmt::While { cond, body } => {
            cond.as_deref().is_some_and(|c| may_write_temp(c, temp))
                || writes_temp_anywhere(body, temp)
        }
    })
}

/// Scopes whose cells are never a temp: the attributes and data of my, the stack and the
/// target object (0 to 5), globals, literals, parameters, the stack object, my and the stack
/// object's person data, and locals.
const TEMP_FREE_SCOPES: [i64; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 9, 10, 18, 19];

/// Whether a statement or condition may write `temp[temp]`: it runs a primitive or a call
/// (any may write the temps), or an Expression's target may be that temp: `temp[temp]`, a temp
/// whose index is not a literal, `stack.temp[..]` (the stack object may be this object), or
/// `scope(S, ..)` unless `S` is a literal scope that never names a temp
/// ([`TEMP_FREE_SCOPES`]; the temps by temp index, 11, do). Text that does not lex may.
pub(crate) fn may_write_temp(text: &str, temp: u8) -> bool {
    use crate::lexer::TokenKind as T;
    let stripped = strip_strings(text);
    if callees(&stripped).any(|c| c != "scope") {
        return true;
    }
    names_as_target(text, &|kinds, i| match kinds[i] {
        T::Ident(w) if w == "temp" => {
            // `temp[K]` with a literal `K` other than `temp` names another cell.
            let other = matches!(
                (kinds.get(i + 1), kinds.get(i + 2), kinds.get(i + 3)),
                (Some(T::LBracket), Some(T::Int(k)), Some(T::RBracket)) if *k != i64::from(temp)
            );
            match (other, kinds.get(i + 1)) {
                (true, _) => None,
                (false, Some(T::LBracket)) => Some(i + 3),
                (false, _) => Some(i),
            }
        }
        T::Ident(w) if w == "scope" && matches!(kinds.get(i + 1), Some(T::LParen)) => {
            let temps = !matches!(
                kinds.get(i + 2),
                Some(T::Int(s)) if TEMP_FREE_SCOPES.contains(s)
            );
            if !temps {
                return None;
            }
            let mut depth = 0usize;
            let close = (i + 1..kinds.len()).find(|&j| {
                match kinds[j] {
                    T::LParen => depth += 1,
                    T::RParen => depth = depth.saturating_sub(1),
                    _ => {}
                }
                depth == 0
            });
            Some(close.unwrap_or(kinds.len()))
        }
        _ => None,
    })
}

/// Whether some operand `operand(kinds, i)` picks (returning the index of its last token) is
/// followed by anything but the end of an operand, a comparison or `flag_test`: an Expression's
/// target. Text that does not lex is.
fn names_as_target(
    text: &str,
    operand: &dyn Fn(&[&crate::lexer::TokenKind], usize) -> Option<usize>,
) -> bool {
    use crate::lexer::TokenKind as T;
    let Ok(tokens) = crate::lexer::tokenize(text) else {
        return true;
    };
    let kinds: Vec<&T> = tokens.iter().map(|t| &t.kind).collect();
    let ends_operand = |k: Option<&&T>| {
        matches!(k, Some(T::Ident(w)) if w == "flag_test")
            || matches!(
                k,
                None | Some(
                    T::Semicolon
                        | T::RParen
                        | T::Comma
                        | T::RBracket
                        | T::LBrace
                        | T::Eof
                        | T::EqEq
                        | T::BangEq
                        | T::Lt
                        | T::LtEq
                        | T::Gt
                        | T::GtEq
                )
            )
    };
    (0..kinds.len())
        .any(|i| operand(&kinds, i).is_some_and(|last| !ends_operand(kinds.get(last + 1))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(body: &str) -> String {
        alias_option_temps(body, &BTreeSet::from(["__v_k_0".to_owned()]))
    }

    #[test]
    fn reads_before_any_write_use_the_temp() {
        let body = "    if !call 282(20) {\n        return true;\n    }\n    let __v_k_0 = temp[0];\n    my.person[28] -= __v_k_0;\n    if __v_k_0 > 3 {\n        my.attr[0] := __v_k_0;\n    }\n    return true;\n";
        assert_eq!(
            run(body),
            "    if !call 282(20) {\n        return true;\n    }\n    let __v_k_0;\n    my.person[28] -= temp[0];\n    if temp[0] > 3 {\n        my.attr[0] := temp[0];\n    }\n    return true;\n"
        );
    }

    #[test]
    fn a_read_after_a_possible_write_keeps_the_local() {
        for after in [
            "    call 300(temps);\n",
            "    idle(1);\n",
            "    temp[0] := 1;\n",
            "    temp[0] += 1;\n",
            "    stack.temp[0] := 1;\n",
            "    scope(8, 0) := 1;\n",
            "    scope(11, 1) := 1;\n",
            "    temp[my.attr[1]] := 1;\n",
            "    if temp[0] inc_lt 3 {\n        return true;\n    }\n",
            "    if call 300(temps) {\n        my.attr[1] := 2;\n    }\n",
            "    while my.attr[1] < 3 {\n        temp[0] := 1;\n    }\n",
        ] {
            let body = format!(
                "    let __v_k_0 = temp[0];\n    my.attr[0] := __v_k_0;\n{after}    my.attr[2] := __v_k_0;\n    return true;\n"
            );
            assert_eq!(run(&body), body, "{after}");
        }
        // A write to another temp, a comparison of the temp, or a write only after the last
        // read leaves the temp alone.
        for after in [
            "    temp[1] := 1;\n",
            "    if temp[0] > 3 {\n        my.attr[1] := 2;\n    }\n",
            "    if my.attr[3] > 3 {\n        my.attr[1] := 2;\n    }\n",
        ] {
            let body = format!(
                "    let __v_k_0 = temp[0];\n    my.attr[0] := __v_k_0;\n{after}    my.attr[2] := __v_k_0;\n    temp[0] := 4;\n    return true;\n"
            );
            assert!(!run(&body).contains("__v_k_0 ="), "{after}");
        }
    }

    #[test]
    fn writes_reads_in_primitives_and_loops_keep_the_local() {
        for use_ in [
            "    __v_k_0 += 1;\n",
            "    idle(ticks: __v_k_0);\n",
            "    call 300(__v_k_0, 0, 0, 0);\n",
            "    while my.attr[1] < __v_k_0 {\n        call 300(temps);\n    }\n",
            "    while my.attr[1] < 3 {\n        my.attr[0] := __v_k_0;\n        temp[0] := 1;\n    }\n",
        ] {
            let body = format!("    let __v_k_0 = temp[0];\n{use_}    return true;\n");
            assert_eq!(run(&body), body, "{use_}");
        }
        // A loop that writes no temp may read it.
        let body = "    let __v_k_0 = temp[0];\n    while my.attr[1] < __v_k_0 {\n        my.attr[1] += 1;\n    }\n    return true;\n";
        assert!(!run(body).contains("__v_k_0 ="));
        // A temp the VM may not have, or a next node that reads the copy's result, keeps it.
        let body = "    let __v_k_0 = temp[8];\n    my.attr[0] := __v_k_0;\n    return true;\n";
        assert_eq!(run(body), body);
        let body = "    let __v_k_0 = temp[0];\n    if idle(1) {\n        my.attr[0] := __v_k_0;\n    }\n    return true;\n";
        assert_eq!(run(body), body);
    }
}
