//! The per-tree backend pipeline: from the body lowering wrote (already carrying the lowering's
//! own bookkeeping, such as the `__saved_stack_obj` save and the resolved pin slot) to the final
//! `.air` body: the entry-save drop or the optimizer, the pinned slot, the raw blocks'
//! constants, and `@slot(N)` frame placement.

use super::*;

/// What the backend needs to know about a lowered tree besides its body text.
pub(crate) struct FrameFacts<'a> {
    /// The tree formats text (a raw block's, or a dialog or action string in the body), which
    /// reads frame words by number (`$Local:N`).
    pub(crate) formats_text: bool,
    /// Keep the body's node layout: a raw block, `local[N]` read, or `@slot` in the tree may
    /// depend on it, so only the unread entry save may go.
    pub(crate) keep_layout: bool,
    /// The tree starts with the Sim-tree entry save `let __self_obj = stack_obj;`.
    pub(crate) has_self_obj_local: bool,
    /// The tree has a pinned `push_interaction` object slot (local 0).
    pub(crate) pinned: bool,
    /// The raw blocks' constants (`name -> value`).
    pub(crate) raw_consts: &'a BTreeMap<String, String>,
    /// Names the raw blocks declare themselves.
    pub(crate) raw_declared: &'a BTreeSet<String>,
    /// The tree has `@slot(N) let` locals, placed by `assign_frame_slots`.
    pub(crate) has_slots: bool,
    /// The `@slot(N)` locals.
    pub(crate) pinned_locals: &'a BTreeMap<String, usize>,
}

/// Runs the backend passes over one lowered tree body.
pub(crate) fn finish_body(out: String, facts: &FrameFacts<'_>) -> String {
    let out = if facts.keep_layout {
        if facts.has_self_obj_local
            && let Some(trimmed) = drop_unread_entry_save(&out, facts.formats_text)
        {
            trimmed
        } else {
            out
        }
    } else {
        let out = crate::v2::opt::optimize_tree_body(&out);
        if facts.pinned {
            pin_in_place(&out)
        } else {
            out
        }
    };
    // The raw blocks' constants, which no pass before this one reads.
    let mut consts = String::new();
    for (name, value) in facts.raw_consts {
        if !facts.raw_declared.contains(name) {
            consts.push_str(&format!("    const {name} = {value};\n"));
        }
    }
    let out = consts + &out;
    if facts.has_slots {
        assign_frame_slots(&out, facts.pinned_locals, facts.raw_declared)
    } else {
        out
    }
}
