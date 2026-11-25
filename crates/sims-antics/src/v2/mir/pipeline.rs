//! The per-tree backend pipeline: from the body lowering wrote (already carrying the lowering's
//! own bookkeeping, such as the `__saved_stack_obj` save and the resolved pin slot) to the final
//! `.air` body: the entry-save drop or the optimizer, the pinned slot, the raw blocks'
//! constants, and `@slot(N)` frame placement.

use super::*;

/// What the backend needs to know about a lowered tree besides its body text.
pub(crate) struct FrameFacts<'a> {
    /// The frame words the tree's text may read: what the object's strings read
    /// ([`text_reads`](mod@super::text_reads)) when the tree formats text (a raw block's, or a
    /// dialog or action string in the body), and none when it does not.
    pub(crate) text_reads: TableLocals,
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
    /// Each inlined helper local's declaration (`TreeLowerer::local_origins`).
    pub(crate) local_origins: &'a BTreeMap<String, String>,
    /// The `min_locals` floor the tree's header will carry (0 for none).
    pub(crate) min_locals: usize,
}

/// Runs the backend passes over one lowered tree body.
pub(crate) fn finish_body(out: String, facts: &FrameFacts<'_>) -> String {
    let reads_text = reads_a_word(&facts.text_reads);
    let unmoved = |before: &str, after: &str| text_words_unmoved(before, after, facts);
    // A tree that may read global 34 keeps main's rules: the entry save and the pin's copies are
    // Expressions, whose scratch-pointer writes that read may depend on, so the tree neither
    // keeps nor drops one where main would not. Its text reads are `Unknown` when it formats
    // text (`TreeLowerer::finish`), which keeps the save as main's text rule does.
    let may_read_global_34 = may_read_global_34(&out);
    let ranges = DrawRanges::for_tree(may_read_global_34);
    let out = if facts.keep_layout {
        if facts.has_self_obj_local
            && let Some(trimmed) = drop_unread_entry_save(&out, &unmoved, ranges)
        {
            trimmed
        } else {
            out
        }
    } else {
        let out = crate::v2::opt::optimize_tree_body(&out, reads_text, facts.local_origins);
        // Pinning moves every named local down a word and fills word 0 from the start, so a
        // tree whose text may read a frame word keeps its copies unless the text reads neither
        // word 0 nor a word that changes hands. A tree that may read global 34 pins as on main.
        let out = if facts.pinned {
            let pinned = pin_in_place(&out, ranges);
            let text_ok = may_read_global_34
                || match &facts.text_reads {
                    TableLocals::Known(words) if words.is_empty() => true,
                    TableLocals::Known(words) => !words.contains(&0) && unmoved(&out, &pinned),
                    TableLocals::Unknown => false,
                };
            if pinned == out || text_ok {
                pinned
            } else {
                out
            }
        } else {
            out
        };
        // The optimizer keeps the entry save of a tree whose text may read a frame word. The
        // frame's locals are final here, so the save still goes when no word the text reads
        // changes hands.
        if facts.has_self_obj_local
            && reads_text
            && let Some(trimmed) = drop_unread_entry_save(&out, &unmoved, ranges)
        {
            trimmed
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

/// Whether dropping the entry save from `before` (giving `after`) leaves every frame word the
/// tree's text may read ([`FrameFacts::text_reads`]) holding the same local
/// ([`frame_occupancy`]). The save's own word changes hands, so text that may read it keeps
/// the save; so does text that may read any word.
fn text_words_unmoved(before: &str, after: &str, facts: &FrameFacts<'_>) -> bool {
    let words = match &facts.text_reads {
        TableLocals::Unknown => return false,
        TableLocals::Known(words) if words.is_empty() => return true,
        TableLocals::Known(words) => words,
    };
    let occupancy = |body: &str| {
        frame_occupancy(
            body,
            facts.pinned_locals,
            facts.raw_declared,
            facts.has_slots,
            facts.min_locals,
        )
    };
    let (Some(was), Some(now)) = (occupancy(before), occupancy(after)) else {
        return false;
    };
    words
        .iter()
        .all(|&w| usize::try_from(w).is_ok_and(|w| was.get(&w) == now.get(&w)))
}
