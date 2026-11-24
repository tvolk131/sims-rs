//! What a `raw { }` block does, read from its `.air` parse rather than its text: the passes
//! that must treat raw code specially (const-binding regions, `for … else`, text liveness) ask
//! here, so comments, strings, spacing and line breaks cannot mislead them.

use crate::ast::{Branches, Condition, Item, PrimitiveCall, Stmt, Target};

/// Opcodes of the primitives that format text, which reads frame locals by number
/// (`$Local:N`): Dialog (36) and Add/Change Action String (50).
const TEXT_OPCODES: [u16; 2] = [36, 50];

/// Facts about one raw block. Code that not even the shape-only parse reads gets every fact but
/// the loop exits, which its tokens decide (the `.air` compiler reports the error later).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RawFacts {
    /// It defines a label (`@x:` or bare `x:`).
    pub defines_label: bool,
    /// It declares a local (`let x`, `local x`).
    pub declares_local: bool,
    /// A branch target names a node by number (`4`, `raw(4)`), which depends on the layout.
    pub positional_target: bool,
    /// A `break` outside every loop of the block, which leaves the loop the block sits in.
    pub breaks_enclosing_loop: bool,
    /// A `continue` outside every loop of the block, which jumps to the head of the loop the
    /// block sits in.
    pub continues_enclosing_loop: bool,
    /// It runs a primitive that formats text.
    pub formats_text: bool,
}

impl RawFacts {
    /// A `break` or `continue` that exits the loop the block sits in.
    pub fn exits_enclosing_loop(&self) -> bool {
        self.breaks_enclosing_loop || self.continues_enclosing_loop
    }

    fn all() -> Self {
        Self {
            defines_label: true,
            declares_local: true,
            positional_target: true,
            breaks_enclosing_loop: true,
            continues_enclosing_loop: true,
            formats_text: true,
        }
    }
}

/// The facts of raw `code`. `prelude` holds the `const` declarations of the names it uses; with
/// none, names where integers belong parse as 0 (the shape is right, the values are not).
pub(crate) fn raw_facts(code: &str, prelude: Option<&str>) -> RawFacts {
    let wrapped = format!(
        "tree 4096 \"__raw_facts\" {{\n{}{code}\n}}\n",
        prelude.unwrap_or("")
    );
    // A strict parse needs every name declared; when that fails (a constant an earlier raw block
    // declared, which this block cannot see), the shape-only parse still reads the structure.
    let parsed = match prelude {
        Some(_) => crate::parser::parse_module(&wrapped)
            .or_else(|_| crate::parser::parse_module_lenient(&wrapped)),
        None => crate::parser::parse_module_lenient(&wrapped),
    };
    match parsed.ok().and_then(|m| m.items.into_iter().next()) {
        Some(Item::Tree(t)) => {
            let mut facts = RawFacts::default();
            scan(&t.body, 0, &mut facts);
            facts
        }
        _ => {
            // The `.air` compiler reports the error. Until then only a loop exit is read from
            // the tokens: without a `break`, a `continue` or a branch (`->`, which may name a
            // node by number) the block cannot leave the loop it sits in. A shape-only parse
            // fails on a constant in a named position (`refresh(target: K)`), so this keeps such
            // a block usable in a `for ... else`.
            let mut facts = RawFacts::all();
            if let Ok(tokens) = crate::lexer::tokenize(code) {
                use crate::lexer::TokenKind as T;
                let has = |pred: &dyn Fn(&T) -> bool| tokens.iter().any(|t| pred(&t.kind));
                facts.breaks_enclosing_loop =
                    has(&|k| matches!(k, T::Ident(w) if w == "break") || *k == T::Arrow);
                facts.continues_enclosing_loop =
                    has(&|k| matches!(k, T::Ident(w) if w == "continue") || *k == T::Arrow);
            }
            facts
        }
    }
}

fn scan(stmts: &[Stmt], loops: usize, facts: &mut RawFacts) {
    for s in stmts {
        match s {
            Stmt::Label { .. } => facts.defines_label = true,
            Stmt::Let { .. } => facts.declares_local = true,
            Stmt::RawOp {
                opcode, branches, ..
            } => {
                facts.formats_text |= TEXT_OPCODES.contains(opcode);
                note_branches(branches, facts);
            }
            Stmt::InlineRawOp { opcode, .. } => {
                facts.formats_text |= TEXT_OPCODES.contains(opcode);
            }
            Stmt::Primitive { prim, branches, .. } => {
                facts.formats_text |= prim_formats_text(prim);
                if let Some(b) = branches {
                    note_branches(b, facts);
                }
            }
            Stmt::Call { branches, .. } => {
                if let Some(b) = branches {
                    note_branches(b, facts);
                }
            }
            Stmt::Return { target, .. } => note_target(target, facts),
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                note_cond(cond, facts);
                scan(then_body, loops, facts);
                scan(else_body, loops, facts);
            }
            Stmt::While { cond, body, .. } => {
                note_cond(cond, facts);
                scan(body, loops + 1, facts);
            }
            Stmt::Loop { body, .. } => scan(body, loops + 1, facts),
            Stmt::Break { .. } => facts.breaks_enclosing_loop |= loops == 0,
            Stmt::Continue { .. } => facts.continues_enclosing_loop |= loops == 0,
            Stmt::Assign { .. } | Stmt::Idle { .. } => {}
        }
    }
}

fn note_cond(cond: &Condition, facts: &mut RawFacts) {
    match cond {
        Condition::Primitive { prim, .. } => facts.formats_text |= prim_formats_text(prim),
        Condition::RawOp { opcode, .. } => facts.formats_text |= TEXT_OPCODES.contains(opcode),
        Condition::Not(inner) => note_cond(inner, facts),
        Condition::Compare { .. } | Condition::Call { .. } => {}
    }
}

fn note_branches(b: &Branches, facts: &mut RawFacts) {
    note_target(&b.on_true, facts);
    note_target(&b.on_false, facts);
}

fn note_target(t: &Target, facts: &mut RawFacts) {
    facts.positional_target |= matches!(t, Target::Node(_) | Target::Raw(_));
}

fn prim_formats_text(prim: &PrimitiveCall) -> bool {
    matches!(
        prim,
        PrimitiveCall::Dialog { .. } | PrimitiveCall::ActionString { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::raw_facts;

    #[test]
    fn facts_follow_the_parse_not_the_text() {
        let f = raw_facts("// let x; break; dialog(\ntemp[0] := 1; /* @a: */", None);
        assert!(!f.declares_local && !f.exits_enclosing_loop() && !f.defines_label);
        assert!(!f.formats_text && !f.positional_target);
        let f = raw_facts("while true { temp[0] := 1; break; }", None);
        assert!(!f.exits_enclosing_loop());
        let f = raw_facts("if temp[0] == 1 { continue; }", None);
        assert!(f.exits_enclosing_loop());
        let f = raw_facts("action_string (301, 1);", None);
        assert!(f.formats_text);
        let f = raw_facts("op 50 (0, 0, 0, 0, 0, 0, 0, 0);", None);
        assert!(f.formats_text);
        let f = raw_facts(
            "random(temp[0], 2) ->\n true: 3, false: done;\n done: local x;",
            None,
        );
        assert!(f.positional_target && f.defines_label && f.declares_local);
        let f = raw_facts("temp[0] := K; animate(clip: K, table: 0, events: 0);", None);
        assert!(!f.positional_target, "a lenient parse reads constants as 0");
        assert!(
            raw_facts("temp[0] :=", None).defines_label,
            "unparsed code has every fact"
        );
    }
}
