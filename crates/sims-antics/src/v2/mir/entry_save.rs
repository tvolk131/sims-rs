//! The Sim-tree `stack_obj` entry save (`let __self_obj = stack_obj;`): forwarding it into reads,
//! and dropping it when nothing reads it.

use super::*;

/// Drops the Sim-tree entry save `let __self_obj = stack_obj;` when nothing else names
/// `__self_obj`. This works line by line, so it also applies to trees the full pass leaves alone
/// (`raw` blocks): a raw block that reads the save names it.
///
/// The save is the lowering's own bookkeeping, not part of the program: an unread save is one the
/// lowering never needed, so it goes even before a node that observes the last result (which
/// then sees the frame's entry state, as the source says).
///
/// Deleting the local moves the frame words of the named locals after it. Text the tree formats
/// reads frame words by number (`$Local:N`), so `text_unmoved(body, trimmed)` must hold: the
/// words that text may read keep their locals.
pub(crate) fn drop_unread_entry_save(
    body: &str,
    text_unmoved: &dyn Fn(&str, &str) -> bool,
) -> Option<String> {
    const SAVE: &str = "let __self_obj = stack_obj;";
    // Deleting the node would shift numeric branch targets (`-> true: 3`), and deleting the
    // local would move the frame words of the named locals after it, which a raw op or a
    // `scope(25, N)` cell may name by number. (`.air` places named locals after every explicit
    // `local[N]`, so those keep their words.)
    if body.contains("/*")
        || has_positional_targets(body)
        || body.lines().any(|l| names_local_slot(&strip_strings(l)))
    {
        return None;
    }
    let mut lines = body
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let (save_at, first) = lines.next()?;
    if first.trim() != SAVE {
        return None;
    }
    let mentions: usize = body
        .lines()
        .map(|l| idents(l).iter().filter(|id| **id == "__self_obj").count())
        .sum();
    if mentions != 1 {
        return None;
    }
    // Every named local after the save moves down one frame word, which keeps whatever an
    // earlier frame left there: none may be read before it is written. The flow analysis runs on
    // the body with one statement per line, so raw blocks written on one line parse too. A body
    // it still cannot parse (a raw node graph with labels, whose jumps it does not model) keeps
    // the save unless it has no other named local to move, as does a raw `local x;`
    // declaration.
    let split = one_statement_per_line(body);
    let statements: Vec<&str> = split.lines().map(str::trim).collect();
    if statements.iter().any(|stmt| {
        stmt.strip_prefix("local")
            .is_some_and(|r| r.starts_with(char::is_whitespace))
    }) {
        return None;
    }
    let moves_a_local = statements
        .iter()
        .filter(|stmt| {
            stmt.strip_prefix("let")
                .is_some_and(|r| r.starts_with(char::is_whitespace))
        })
        .count()
        > 1;
    let defined = match parse_body(&split) {
        Some(nodes) => locals_defined_before_reads(&nodes),
        None => !moves_a_local,
    };
    if !defined {
        return None;
    }
    let mut out = String::with_capacity(body.len());
    for (i, l) in body.lines().enumerate() {
        if i != save_at {
            out.push_str(l);
            out.push('\n');
        }
    }
    text_unmoved(body, &out).then_some(out)
}

/// The entry save `let __self_obj = stack_obj;` (or `__saved_stack_obj`), as its name.
pub(crate) fn entry_save(nodes: &[Stmt]) -> Option<&str> {
    match nodes.first()? {
        Stmt::Inst(l) => match classify(l) {
            InstKind::Let {
                name: name @ ("__self_obj" | "__saved_stack_obj"),
                init: Some("stack_obj"),
            } => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// Reads of the entry save become `stack_obj` while nothing can have changed the stack object
/// since entry: one fewer use of the save, and a restore there becomes `stack_obj := stack_obj`.
pub(crate) fn forward_entry_save(nodes: &mut [Stmt]) {
    let Some(name) = entry_save(nodes) else {
        return;
    };
    let map = BTreeMap::from([(name.to_owned(), "stack_obj".to_owned())]);
    forward_save_block(&mut nodes[1..], &map);
}

/// Renames per `map` up to the first node that may write `stack_obj`; returns whether control
/// falling off the end of `nodes` still has the entry stack object.
pub(crate) fn forward_save_block(nodes: &mut [Stmt], map: &BTreeMap<String, String>) -> bool {
    // `Err` when a primitive names the save other than as a labelled value operand: it may be
    // a local-slot operand (`icon:`, `local:`, ...) or a destination, so forwarding stops.
    let renamed = |text: &str| -> Result<Option<String>, ()> {
        if !idents(text).iter().any(|id| map.contains_key(*id)) {
            return Ok(None);
        }
        if !value_operands_only(text, map) {
            return Err(());
        }
        Ok(Some(rename_idents(text, map)))
    };
    for n in nodes.iter_mut() {
        match n {
            Stmt::Inst(l) => {
                let writes_save = match classify(l) {
                    InstKind::Let { name, .. } | InstKind::Assign { lhs: name, .. } => {
                        map.contains_key(name)
                    }
                    _ => false,
                };
                if writes_save {
                    return false;
                }
                match renamed(l) {
                    Ok(Some(r)) => *l = r,
                    Ok(None) => {}
                    Err(()) => return false,
                }
                if may_write_stack_obj(l) {
                    return false;
                }
            }
            Stmt::If { arms, else_body } => {
                let mut kept = true;
                for (cond, body) in arms.iter_mut() {
                    match renamed(cond) {
                        Ok(Some(r)) => *cond = r,
                        Ok(None) => {}
                        Err(()) => return false,
                    }
                    if may_write_stack_obj(cond) {
                        return false;
                    }
                    kept &= forward_save_block(body, map);
                }
                if let Some(body) = else_body {
                    kept &= forward_save_block(body, map);
                }
                if !kept {
                    return false;
                }
            }
            Stmt::While { cond, body } => {
                if cond.as_deref().is_some_and(may_write_stack_obj)
                    || writes_stack_obj_anywhere(body)
                {
                    return false;
                }
                if let Some(c) = cond {
                    match renamed(c) {
                        Ok(Some(r)) => *c = r,
                        Ok(None) => {}
                        Err(()) => return false,
                    }
                }
                forward_save_block(body, map);
            }
        }
    }
    true
}

/// Drops an entry save nothing else names (see `drop_unread_entry_save`), when no named local
/// that moves down a frame word is read before it is written. `text_reads` says the tree's text
/// may read a frame word (`FrameFacts::text_reads`): then a body that formats text keeps the
/// save here, and the pipeline decides once the frame is final.
pub(crate) fn drop_unread_entry_save_node(nodes: &mut Vec<Stmt>, text_reads: bool) {
    let Some(name) = entry_save(nodes) else {
        return;
    };
    if !mentions(&nodes[1..], name)
        && !(text_reads && nodes_format_text(&nodes[1..]))
        && locals_defined_before_reads(&nodes[1..])
    {
        nodes.remove(0);
    }
}
