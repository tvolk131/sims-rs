//! Raw `.air` blocks: parsing, validation, label collection, and `stack_obj` clobber facts of raw
//! statements.

use super::*;

/// Parses a `raw { ... }` block's `.air` text the same way the lowering validates it.
pub(super) fn parse_raw_air_stmts(code: &str) -> Option<Vec<crate::ast::Stmt>> {
    let wrapped = format!("tree 4096 \"__raw_check\" {{\n{code}\n}}\n");
    let parsed = crate::parser::parse_module(&wrapped).ok()?;
    match parsed.items.into_iter().next() {
        Some(crate::ast::Item::Tree(t)) => Some(t.body),
        _ => None,
    }
}

/// Whether a `raw { ... }` block may overwrite the frame's `stack_obj` (or names `__self_obj`
/// itself), which forces the `__self_obj` save in a Sim-on-object tree. Text that does not parse
/// is assumed to clobber.
pub(super) fn raw_air_may_clobber_stack_obj(code: &str) -> bool {
    if code.contains("__self_obj") {
        return true;
    }
    match parse_raw_air_stmts(code) {
        Some(stmts) => raw_air_stmts_clobber_stack_obj(&stmts),
        None => true,
    }
}

pub(super) fn is_stack_obj_operand(op: &crate::ast::OperandRef) -> bool {
    matches!(
        op,
        crate::ast::OperandRef::Direct(crate::ast::Operand {
            scope: 10,
            index: 0
        })
    )
}

/// Whether a raw `op <opcode> (bytes)` may write the running frame's `stack_obj`, decoded the
/// way `sims-vm` runs it. BHAV calls never do (`stack_obj` is per frame and never copied back).
/// Of the primitives, only these write it: the ones that store into an operand the bytes name
/// (Expression, Random Number, Get Direction To, Relationship, Set To Next) when that operand is
/// the stack object (scope 10), and the ones that set it outright (Find Best Interaction, Find
/// Best Object, Make New Character, Gosub Found Action, Create Object). Unknown opcodes may.
pub(super) fn raw_op_may_write_stack_obj(opcode: u16, ops: &[u8; 8]) -> bool {
    const STACK_OBJ_SCOPE: u16 = 10;
    let word = |i: usize| u16::from_le_bytes([ops[i], ops[i + 1]]);
    match opcode {
        0x0100.. => false,
        // Expression: left operand scope byte 6.
        0x0002 => u16::from(ops[6]) == STACK_OBJ_SCOPE,
        // Random Number / Get Direction To: destination scope word 2.
        0x0008 | 0x000C => word(2) == STACK_OBJ_SCOPE,
        // Relationship: value operand scope byte 4 (it is stored when reading).
        0x001A => u16::from(ops[4]) == STACK_OBJ_SCOPE,
        // Set To Next: the stack object unless flag 0x80 names a destination (scope byte 5).
        0x001F => ops[4] & 0x80 == 0 || u16::from(ops[5]) == STACK_OBJ_SCOPE,
        0x0003 | 0x000E | 0x0013 | 0x001E | 0x002A => true,
        0x0000..=0x0033 => false,
        _ => true,
    }
}

pub(super) fn raw_condition_clobbers_stack_obj(cond: &crate::ast::Condition) -> bool {
    match cond {
        // `inc_lt` / `dec_gt` store the stepped value back into their left operand.
        crate::ast::Condition::Compare { lhs, op, .. } => {
            matches!(
                op,
                crate::ast::CompareOp::IncLt | crate::ast::CompareOp::DecGt
            ) && is_stack_obj_operand(lhs)
        }
        crate::ast::Condition::Call { .. } => false,
        crate::ast::Condition::Primitive { prim, .. } => raw_primitive_clobbers_stack_obj(prim),
        crate::ast::Condition::RawOp {
            opcode, operands, ..
        } => raw_op_may_write_stack_obj(*opcode, operands),
        crate::ast::Condition::Not(inner) => raw_condition_clobbers_stack_obj(inner),
    }
}

pub(super) fn raw_primitive_clobbers_stack_obj(prim: &crate::ast::PrimitiveCall) -> bool {
    use crate::ast::PrimitiveCall as P;
    match prim {
        // These set the running frame's stack object (or may, through an autonomy choice).
        P::FindBestInteraction
        | P::FindBestObject { .. }
        | P::CreateObject { .. }
        | P::MakeNewCharacter { .. }
        | P::GosubFoundAction => true,
        P::SetToNext { dest, .. } => dest.as_ref().is_none_or(is_stack_obj_operand),
        P::ExprRaw { lhs, .. } => is_stack_obj_operand(lhs),
        P::Random { dst, .. } | P::GetDirectionTo { dst, .. } => is_stack_obj_operand(dst),
        P::Relationship { value, .. } => is_stack_obj_operand(value),
        _ => false,
    }
}

pub(super) fn raw_air_stmts_clobber_stack_obj(stmts: &[crate::ast::Stmt]) -> bool {
    use crate::ast::Stmt;
    stmts.iter().any(|s| match s {
        Stmt::Assign { lhs, .. } => is_stack_obj_operand(lhs),
        Stmt::RawOp {
            opcode, operands, ..
        }
        | Stmt::InlineRawOp {
            opcode, operands, ..
        } => raw_op_may_write_stack_obj(*opcode, operands),
        Stmt::Primitive { prim, .. } => raw_primitive_clobbers_stack_obj(prim),
        Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            raw_condition_clobbers_stack_obj(cond)
                || raw_air_stmts_clobber_stack_obj(then_body)
                || raw_air_stmts_clobber_stack_obj(else_body)
        }
        Stmt::While { cond, body, .. } => {
            raw_condition_clobbers_stack_obj(cond) || raw_air_stmts_clobber_stack_obj(body)
        }
        Stmt::Loop { body, .. } => raw_air_stmts_clobber_stack_obj(body),
        Stmt::Label { .. }
        | Stmt::Let { .. }
        | Stmt::Call { .. }
        | Stmt::Idle { .. }
        | Stmt::Break { .. }
        | Stmt::Continue { .. }
        | Stmt::Return { .. } => false,
    })
}

/// The compiler's entry-object locals (`__self_obj`, `__saved_stack_obj`) that a raw block
/// declares as a local, a constant or a label. A raw block may read them, and may declare them
/// in a tree whose lowering does not declare them itself (`TreeLowerer::finish` checks that).
pub(super) fn raw_reserved_declarations(code: &str) -> Vec<&'static str> {
    use crate::lexer::TokenKind as T;
    const RESERVED: [&str; 2] = [mir::SELF_OBJ, mir::SAVED_STACK_OBJ];
    let mut found = Vec::new();
    let Ok(tokens) = crate::lexer::tokenize(code) else {
        return found;
    };
    for (i, pair) in tokens.windows(2).enumerate() {
        // `const __self_obj.tuning = ..` declares a qualified constant, not the entry object.
        let qualified = matches!(tokens.get(i + 2).map(|t| &t.kind), Some(T::Dot));
        if let (T::Ident(kw), T::Ident(name)) = (&pair[0].kind, &pair[1].kind)
            && matches!(kw.as_str(), "let" | "local" | "const")
            && !qualified
            && let Some(reserved) = RESERVED.iter().find(|r| **r == name.as_str())
        {
            found.push(*reserved);
        }
    }
    let wrapped = format!("tree 4096 \"__raw_names\" {{\n{code}\n}}\n");
    if let Ok(module) = crate::parser::parse_module_lenient(&wrapped)
        && let Some(crate::ast::Item::Tree(tree)) = module.items.first()
    {
        let mut labels = BTreeSet::new();
        collect_raw_air_labels(&tree.body, &mut labels);
        found.extend(RESERVED.iter().filter(|n| labels.contains(**n)));
    }
    found
}

pub(super) fn collect_raw_air_labels<'a>(
    stmts: &'a [crate::ast::Stmt],
    labels: &mut BTreeSet<&'a str>,
) {
    for s in stmts {
        match s {
            crate::ast::Stmt::Label { name, .. } => {
                labels.insert(name.as_str());
            }
            crate::ast::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_raw_air_labels(then_body, labels);
                collect_raw_air_labels(else_body, labels);
            }
            crate::ast::Stmt::While { body, .. } | crate::ast::Stmt::Loop { body, .. } => {
                collect_raw_air_labels(body, labels);
            }
            _ => {}
        }
    }
}

pub(super) fn validate_raw_air_target(
    target: &crate::ast::Target,
    defined_labels: &BTreeSet<&str>,
    span: Span,
) -> Result<()> {
    match target {
        crate::ast::Target::Label(name) if defined_labels.contains(name.as_str()) => Ok(()),
        other => Err(Error::at(
            span,
            format!(
                "'raw {{ ... }}' cannot branch to '{other:?}' while cleanups, inline helpers, or cancel_on_abort loops are active"
            ),
        )),
    }
}

pub(super) fn validate_raw_air_stmts(
    stmts: &[crate::ast::Stmt],
    in_raw_loop: bool,
    defined_labels: &BTreeSet<&str>,
    span: Span,
) -> Result<()> {
    for s in stmts {
        match s {
            crate::ast::Stmt::Return { .. } => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot contain 'return' while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::Break { .. } | crate::ast::Stmt::Continue { .. } if !in_raw_loop => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot break/continue out of its block while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::RawOp { branches, .. } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Primitive {
                branches: Some(branches),
                ..
            } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Call {
                branches: Some(branches),
                ..
            } => {
                validate_raw_air_target(&branches.on_true, defined_labels, span)?;
                validate_raw_air_target(&branches.on_false, defined_labels, span)?;
            }
            crate::ast::Stmt::Call { branches: None, .. } => {
                return Err(Error::at(
                    span,
                    "'raw { ... }' cannot invoke an unbranched subroutine call (whose false branch exits the tree) while cleanups, inline helpers, or cancel_on_abort loops are active",
                ));
            }
            crate::ast::Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                validate_raw_air_stmts(then_body, in_raw_loop, defined_labels, span)?;
                validate_raw_air_stmts(else_body, in_raw_loop, defined_labels, span)?;
            }
            crate::ast::Stmt::While { body, .. } | crate::ast::Stmt::Loop { body, .. } => {
                validate_raw_air_stmts(body, true, defined_labels, span)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether a raw block leaves the tree's node layout free for the backend optimizer
/// (`TreeLowerer::keep_layout` stays off for it). The block must be built only from text the
/// lowering itself emits, touching no frame word and reading no instruction bytes beyond its own:
///
/// - calls are `call N(temps)` (the temps pass-through ABI: the callee reads no literal words, so
///   none from the next instruction, and has its own frame);
/// - the only primitives are ones that read neither the caller's frame words nor bytes past their
///   own operands (`find_best_object`, `run_functional_tree`, `play_sound`; checked in `sims-vm`);
/// - every other word is control flow (`if`, `else`, `while`, `break`, `continue`, `return`) or a
///   temp, parameter, global, own, Sim or stack-object cell, or a primitive's named argument: no
///   `local[N]` or named local (so no frame word, and no name the optimizer could rename or
///   coalesce), no `let`/`local`/`const` declaration, no string, no node label or `->` graph,
///   no `scope(..)`/`expr(..)`/`pad` spelling.
///
/// Such a block's text is one the lowering could have produced, so whatever the optimizer
/// proves of lowered text holds for it. Anything else keeps the whole tree's layout.
/// The trees a raw block calls with inline arguments: `call N(..)` other than `call N(temps)`,
/// and `op N (..)` for an opcode of 256 and up. Empty when the block does not tokenize (the
/// `.air` compiler then reports it).
pub(super) fn raw_inline_callees(code: &str) -> Vec<u16> {
    use crate::lexer::TokenKind as T;
    let Ok(tokens) = crate::lexer::tokenize(code) else {
        return Vec::new();
    };
    let kind = |i: usize| tokens.get(i).map(|t| &t.kind);
    let mut ids = Vec::new();
    for i in 0..tokens.len() {
        let (Some(T::Ident(w)), Some(&T::Int(n))) = (kind(i), kind(i + 1)) else {
            continue;
        };
        let Ok(id) = u16::try_from(n) else {
            continue;
        };
        let temps = matches!(kind(i + 3), Some(T::Ident(t)) if t == "temps");
        if (w == "call" && !temps) || (w == "op" && id >= 256) {
            ids.push(id);
        }
    }
    ids
}

pub(super) fn raw_block_is_layout_free(code: &str) -> bool {
    use crate::lexer::TokenKind as T;
    const WORDS: &[&str] = &[
        "if",
        "else",
        "while",
        "break",
        "continue",
        "return",
        "true",
        "false",
        "error",
        "temp",
        "param",
        "global",
        "my",
        "stack",
        "attr",
        "data",
        "person",
        "stack_obj",
        "flag_set",
        "flag_clear",
        "flag_test",
    ];
    const PRIMS: &[&str] = &["find_best_object", "run_functional_tree", "play_sound"];
    // The line printer strips only `//` comments.
    if code.contains("/*") || code.contains('#') {
        return false;
    }
    let Ok(tokens) = crate::lexer::tokenize(code) else {
        return false;
    };
    let kind = |i: usize| tokens.get(i).map(|t| &t.kind);
    let mut depth = 0i32;
    let mut i = 0;
    while let Some(k) = kind(i) {
        match k {
            T::Eof => break,
            T::Label(_) | T::Arrow | T::Str(_) => return false,
            T::LParen => depth += 1,
            T::RParen => depth -= 1,
            T::Colon if depth == 0 => return false,
            T::Ident(w) if w == "call" => {
                let temps_call = matches!(kind(i + 1), Some(T::Int(_)))
                    && kind(i + 2) == Some(&T::LParen)
                    && matches!(kind(i + 3), Some(T::Ident(t)) if t == "temps")
                    && kind(i + 4) == Some(&T::RParen);
                if !temps_call {
                    return false;
                }
                i += 5;
                continue;
            }
            T::Ident(w) => {
                let ok = if kind(i + 1) == Some(&T::LParen) {
                    PRIMS.contains(&w.as_str())
                } else if depth > 0 && kind(i + 1) == Some(&T::Colon) {
                    // A primitive's named argument (`flags: 0`).
                    true
                } else {
                    WORDS.contains(&w.as_str())
                };
                if !ok {
                    return false;
                }
            }
            _ => {}
        }
        i += 1;
    }
    depth == 0 && !raw_arms_end_alike(code)
}

/// Whether the block has an `if c { .. T } else { .. T }` whose arms may end in the same
/// statement once the backend's other passes have run (any arm ending in a loop or an `if`
/// counts).
/// The backend joins such tails after the `if` (`opt::merge_duplicate_tails`), which changes the
/// node layout the block spells out, so the block keeps the tree's layout instead: its arms stay
/// as written. (A raw block is written to fix its nodes; structured code still gets the join.)
fn raw_arms_end_alike(code: &str) -> bool {
    fn any_alike(nodes: &[mir::Stmt]) -> bool {
        nodes.iter().any(|n| match n {
            mir::Stmt::Inst(_) => false,
            mir::Stmt::While { body, .. } => any_alike(body),
            mir::Stmt::If { arms, else_body } => {
                // Other passes run first and may reshape an arm's end (a single-pass loop is
                // unwrapped, a nested `if` folded), so only arms that all end with an
                // instruction, no two the same, are known never to join.
                let joinable = else_body.as_ref().is_some_and(|else_body| {
                    let lasts: Option<Vec<&mir::Stmt>> = arms
                        .iter()
                        .map(|(_, body)| body.last())
                        .chain(std::iter::once(else_body.last()))
                        .collect();
                    lasts.is_some_and(|lasts| {
                        !lasts.iter().all(|l| matches!(l, mir::Stmt::Inst(_)))
                            || lasts
                                .iter()
                                .enumerate()
                                .any(|(i, a)| lasts[i + 1..].contains(a))
                    })
                });
                joinable
                    || arms.iter().any(|(_, body)| any_alike(body))
                    || else_body.as_deref().is_some_and(any_alike)
            }
        })
    }
    // A block the backend cannot parse is left alone by it.
    mir::parse_body(&layout_free_raw_lines(code).join("\n")).is_some_and(|nodes| any_alike(&nodes))
}

/// A layout-free raw block ([`raw_block_is_layout_free`]) as the lowering prints its own code:
/// one statement per line, `{` ending its line, `}` alone (or `} else {`), comments dropped, so
/// the backend passes parse it like lowered text. The block has no strings, so every `{`, `}`
/// and `;` is punctuation.
pub(super) fn layout_free_raw_lines(code: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, lines: &mut Vec<String>| {
        let t = cur.split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() {
            lines.push(t);
        }
        cur.clear();
    };
    for raw_line in code.lines() {
        let line = raw_line.split("//").next().unwrap_or("");
        for c in line.chars() {
            match c {
                '{' | ';' => {
                    cur.push(c);
                    flush(&mut cur, &mut lines);
                }
                '}' => {
                    flush(&mut cur, &mut lines);
                    lines.push("}".to_owned());
                }
                _ => cur.push(c),
            }
        }
        cur.push(' ');
    }
    flush(&mut cur, &mut lines);
    let mut joined: Vec<String> = Vec::with_capacity(lines.len());
    for l in lines {
        if l.starts_with("else")
            && let Some(prev) = joined.last_mut()
            && prev == "}"
        {
            *prev = format!("}} {l}");
        } else {
            joined.push(l);
        }
    }
    // Nested lines indent like lowered code (the backend passes ignore indentation).
    let mut depth = 0usize;
    joined
        .into_iter()
        .map(|l| {
            if l.starts_with('}') {
                depth = depth.saturating_sub(1);
            }
            let out = format!("{}{l}", "    ".repeat(depth));
            if l.ends_with('{') {
                depth += 1;
            }
            out
        })
        .collect()
}
