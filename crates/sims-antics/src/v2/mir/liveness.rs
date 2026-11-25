//! Local-variable liveness over the MIR (interference edges, dead definitions, live-out sets), used
//! by dead-store elimination, copy forwarding and local coalescing.

use super::*;

pub(crate) type Set = BTreeSet<String>;

pub(crate) struct Liveness<'a> {
    pub(crate) locals: &'a BTreeSet<String>,
    /// The tree formats text (a dialog or action string), which reads frame locals by number
    /// (`$Local:N`, `$TimeLocal:N`): every such node reads every source local.
    pub(crate) text_reads_locals: bool,
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
                    // A name outside `locals` (a pinned `@slot` word) is never a dead def.
                    InstKind::Let {
                        name,
                        init: Some(init),
                    } => (self.locals.contains(name).then_some(name), init),
                    InstKind::Assign { lhs, op: ":=", rhs } if self.locals.contains(lhs) => {
                        (Some(lhs), rhs)
                    }
                    _ => (None, l.as_str()),
                };
                self.live_out.insert(std::ptr::from_ref(n), out.clone());
                if let Some(d) = def
                    && d.starts_with("__")
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
                    let then_in = self.block(body, out.clone(), brk, cont);
                    let after_test: Set = then_in.union(&acc).cloned().collect();
                    let m = self.mentions(cond);
                    let mut live_in = after_test.clone();
                    live_in.extend(m.iter().cloned());
                    self.record(&m, &live_in);
                    acc = live_in;
                }
                acc
            }
            Stmt::While { cond, body } => {
                let m = cond
                    .as_deref()
                    .map(|c| self.mentions(c))
                    .unwrap_or_default();
                let mut head: Set = m.clone();
                if cond.is_some() {
                    head.extend(out.iter().cloned());
                }
                loop {
                    let body_in = self.block(body, head.clone(), &out, &head);
                    let mut new_head = body_in;
                    if cond.is_some() {
                        new_head.extend(out.iter().cloned());
                        new_head.extend(m.iter().cloned());
                    }
                    if new_head == head {
                        break;
                    }
                    head = new_head;
                }
                self.record(&m, &head);
                head
            }
        }
    }
}

/// A local the source names (`let x`, an inlined helper's `let x`, `@slot(N) let x`), as
/// opposed to a compiler temporary: a tree that formats text may read its word by number.
pub(crate) fn is_source_local(name: &str) -> bool {
    name.starts_with("__v_") || name.starts_with("__inline_v_") || name.starts_with("__slot")
}
