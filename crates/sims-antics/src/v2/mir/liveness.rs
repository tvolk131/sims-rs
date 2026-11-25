//! Local-variable liveness over the MIR (interference edges, dead definitions, live-out sets), used
//! by dead-store elimination, copy forwarding and local coalescing.

use super::*;

pub(crate) type Set = BTreeSet<String>;

pub(crate) struct Liveness<'a> {
    pub(crate) locals: &'a BTreeSet<String>,
    /// The tree formats text (a dialog or action string), which reads frame locals by number
    /// (`$Local:N`, `$TimeLocal:N`): every such node reads every source local.
    pub(crate) text_reads_locals: bool,
    /// Primitive out-operands define their local ([`out_operand_write`]). Off in a tree that
    /// may read global 34 ([`may_read_global_34`]): a store they make dead is an Expression
    /// whose scratch-pointer write that read may depend on, so such a tree keeps main's rule.
    pub(crate) out_defs: bool,
    pub(crate) edges: BTreeSet<(String, String)>,
    /// For each plain store to a v2 local (`__...`): whether the local is dead after it. Loops
    /// revisit their bodies with growing live sets; the last visit, with the final sets, wins.
    pub(crate) dead_defs: BTreeMap<*const Stmt, bool>,
    /// The locals live after each simple statement (last visit wins, as above).
    pub(crate) live_out: BTreeMap<*const Stmt, Set>,
}

impl Liveness<'_> {
    pub(crate) fn mentions(&self, text: &str) -> Set {
        let mut out: Set = idents(text)
            .into_iter()
            .filter(|t| self.locals.contains(*t))
            .map(str::to_owned)
            .collect();
        if self.text_reads_locals && formats_text(text) {
            out.extend(self.locals.iter().filter(|l| is_source_local(l)).cloned());
        }
        out
    }

    /// Records that every mentioned local interferes with every local live around the point.
    pub(crate) fn record(&mut self, mentioned: &Set, around: &Set) {
        for a in mentioned {
            for b in around.iter().chain(mentioned.iter()) {
                if a != b {
                    let (x, y) = if a < b { (a, b) } else { (b, a) };
                    self.edges.insert((x.clone(), y.clone()));
                }
            }
        }
    }

    pub(crate) fn block(&mut self, nodes: &[Stmt], out: Set, brk: &Set, cont: &Set) -> Set {
        let mut live = out;
        for n in nodes.iter().rev() {
            live = self.node(n, live, brk, cont);
        }
        live
    }

    pub(crate) fn node(&mut self, n: &Stmt, out: Set, brk: &Set, cont: &Set) -> Set {
        match n {
            Stmt::Inst(l) => {
                let (def, uses_text): (Option<&str>, &str) = match classify(l) {
                    InstKind::Return => return Set::new(),
                    InstKind::Break => return brk.clone(),
                    InstKind::Continue => return cont.clone(),
                    InstKind::Let { init: None, .. } => return out,
                    // A name outside `locals` (a pinned `#[slot]` word) is never a dead def.
                    InstKind::Let {
                        name,
                        init: Some(init),
                    } => (self.locals.contains(name).then_some(name), init),
                    InstKind::Assign { lhs, op: ":=", rhs } if self.locals.contains(lhs) => {
                        (Some(lhs), rhs)
                    }
                    _ => (None, l.as_str()),
                };
                // A primitive that stores into a local through an out-operand on both exits
                // defines it like a plain store (it is not a removable store, so it is never a
                // dead def). One that stores on only one exit defines nothing on the other, so
                // as a bare statement it stays a read.
                let out_def = match def {
                    None => self.out_write(l).filter(|w| w.2 == WriteExit::Both),
                    Some(_) => None,
                };
                let (def, uses_text): (Option<&str>, &str) = match &out_def {
                    Some((name, rest, _)) => (Some(name.as_str()), rest.as_str()),
                    None => (def, uses_text),
                };
                self.live_out.insert(std::ptr::from_ref(n), out.clone());
                if let Some(d) = def
                    && d.starts_with("__")
                    && out_def.is_none()
                {
                    self.dead_defs
                        .insert(std::ptr::from_ref(n), !out.contains(d));
                }
                let uses = self.mentions(uses_text);
                let mut live_in = out.clone();
                if let Some(d) = def {
                    live_in.remove(d);
                }
                live_in.extend(uses.iter().cloned());
                let mut mentioned = uses;
                if let Some(d) = def {
                    mentioned.insert(d.to_owned());
                }
                let around: Set = out.union(&live_in).cloned().collect();
                self.record(&mentioned, &around);
                live_in
            }
            Stmt::If { arms, else_body } => {
                let mut acc = match else_body {
                    Some(body) => self.block(body, out.clone(), brk, cont),
                    None => out.clone(),
                };
                for (cond, body) in arms.iter().rev() {
                    let mut then_in = self.block(body, out.clone(), brk, cont);
                    let after_test: Set = then_in.union(&acc).cloned().collect();
                    let (m, kill) = self.cond_effect(cond);
                    if let Some((name, on_true, on_false)) = &kill {
                        if *on_true {
                            then_in.remove(name);
                        }
                        if *on_false {
                            acc.remove(name);
                        }
                    }
                    let mut live_in: Set = then_in.union(&acc).cloned().collect();
                    live_in.extend(m.iter().cloned());
                    let around: Set = after_test.union(&live_in).cloned().collect();
                    self.record(&stored_and(&m, &kill), &around);
                    acc = live_in;
                }
                acc
            }
            Stmt::While { cond, body } => {
                let (m, kill) = match cond.as_deref() {
                    Some(c) => self.cond_effect(c),
                    None => (Set::new(), None),
                };
                // The live set on the condition's false exit (the loop exit).
                let mut exit_live = out.clone();
                if let Some((name, _, true)) = &kill {
                    exit_live.remove(name);
                }
                let mut head: Set = m.clone();
                if cond.is_some() {
                    head.extend(exit_live.iter().cloned());
                }
                let mut around = head.clone();
                loop {
                    let mut body_in = self.block(body, head.clone(), &out, &head);
                    around.extend(body_in.iter().cloned());
                    if let Some((name, true, _)) = &kill {
                        body_in.remove(name);
                    }
                    let mut new_head = body_in;
                    if cond.is_some() {
                        new_head.extend(exit_live.iter().cloned());
                        new_head.extend(m.iter().cloned());
                    }
                    if new_head == head {
                        break;
                    }
                    head = new_head;
                }
                around.extend(head.iter().cloned());
                around.extend(out.iter().cloned());
                self.record(&stored_and(&m, &kill), &around);
                head
            }
        }
    }
}

impl Liveness<'_> {
    /// When `text` stores into one of the locals through a primitive out-operand
    /// ([`out_operand_write`]): that local, the text with the operand blanked, and the exits
    /// that store.
    fn out_write(&self, text: &str) -> Option<(String, String, WriteExit)> {
        if !self.out_defs {
            return None;
        }
        idents(text)
            .into_iter()
            .filter(|t| self.locals.contains(*t))
            .find_map(|name| {
                out_operand_write(text, name).map(|(rest, exit)| (name.to_owned(), rest, exit))
            })
    }

    /// A branch condition's reads, and the local its primitive stores into, if any, with
    /// whether the store happens on the condition's true and false exits (a leading `!` swaps
    /// the primitive's exits). On an exit that does not store, the local keeps its old value,
    /// so it stays live there exactly when the successor reads it.
    fn cond_effect(&self, cond: &str) -> (Set, Option<(String, bool, bool)>) {
        let (negated, inner) = match cond.trim_start().strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, cond),
        };
        let Some((name, rest, exit)) = self.out_write(inner) else {
            return (self.mentions(cond), None);
        };
        let (prim_true, prim_false) = (exit != WriteExit::False, exit != WriteExit::True);
        let (on_true, on_false) = if negated {
            (prim_false, prim_true)
        } else {
            (prim_true, prim_false)
        };
        (self.mentions(&rest), Some((name, on_true, on_false)))
    }
}

/// A condition's reads plus the local it stores into: every local the condition touches, which
/// interferes with every local live around it.
fn stored_and(reads: &Set, kill: &Option<(String, bool, bool)>) -> Set {
    let mut all = reads.clone();
    if let Some((name, _, _)) = kill {
        all.insert(name.clone());
    }
    all
}

/// A local the source names (`let x`, an inlined helper's `let x`, `#[slot(N)] let x`), as
/// opposed to a compiler temporary: a tree that formats text may read its word by number.
pub(crate) fn is_source_local(name: &str) -> bool {
    name.starts_with("__v_") || name.starts_with("__inline_v_") || name.starts_with("__slot")
}
