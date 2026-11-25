//! The per-tree backend pipeline: from the body lowering wrote (already carrying the lowering's
//! own bookkeeping, such as the `__saved_stack_obj` save and the resolved pin slot) to the final
//! `.air` body: the entry-save drop or the optimizer, the pinned slot, the raw blocks'
//! constants, and `@slot(N)` frame placement.

use super::*;

/// What the backend needs to know about a lowered tree besides its body text.
pub(crate) struct FrameFacts<'a> {
    /// The tree keeps main's rules: some tree of its object may read global 34
    /// (`ObjectEnv::reads_global_34`), or it pins a `push_interaction` slot, which must pin
    /// exactly where main's body lets it. Its text reads are then `Unknown` when it formats text.
    pub(crate) main_rules: bool,
    /// The tree's header sets its frame size (`@frame(locals: K)`), which main's coalescing
    /// fits: out-operand locals share words as on main.
    pub(crate) fixed_frame: bool,
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
    let body = finish_frame(out, facts);
    // The raw blocks' constants, which no pass before this one reads.
    let mut consts = String::new();
    for (name, value) in facts.raw_consts {
        if !facts.raw_declared.contains(name) {
            consts.push_str(&format!("    const {name} = {value};\n"));
        }
    }
    let out = consts + &body;
    if facts.has_slots {
        assign_frame_slots(&out, facts.pinned_locals, facts.raw_declared)
    } else {
        out
    }
}

/// The entry-save drop or the optimizer, and the pinned slot.
fn finish_frame(out: String, facts: &FrameFacts<'_>) -> String {
    let text_reads = &facts.text_reads;
    let reads_text = reads_a_word(text_reads);
    let unmoved = |before: &str, after: &str| text_words_unmoved(before, after, facts);
    // A tree that may read global 34 keeps main's rules: the entry save and the pin's copies are
    // Expressions, whose scratch-pointer writes that read depends on, so the tree neither
    // keeps nor drops one where main would not. So does every tree of its object, since a call
    // carries the scratch pointer between trees, and a tree that pins (`FrameFacts::main_rules`).
    let main_rules = facts.main_rules || may_read_global_34(&out);
    let ranges = DrawRanges::for_tree(main_rules);
    if facts.keep_layout {
        if facts.has_self_obj_local
            && let Some(trimmed) = drop_unread_entry_save(&out, &unmoved, ranges)
        {
            trimmed
        } else {
            out
        }
    } else {
        let optimize = |decl_groups| {
            crate::v2::opt::optimize_tree_body(
                &out,
                reads_text,
                facts.local_origins,
                decl_groups,
                main_rules,
            )
        };
        // Declaration groups keep some words main's coalescing shares, so a tree they leave
        // within a word of the largest frame (`MAX_FRAME_LOCALS`; the `idle` scratch word may
        // take one more) coalesces as main does.
        let grouped = optimize(!facts.fixed_frame);
        let out = if !facts.fixed_frame && count_frame_locals(&grouped) + 1 >= MAX_FRAME_LOCALS {
            optimize(false)
        } else {
            grouped
        };
        // Pinning moves every named local down a word and fills word 0 from the start, which a
        // primitive may read implicitly (`relationship` without `local:` reads the word-0 pair).
        // The tree pins exactly where main's literal-range analysis pins, on the body main's
        // rules give (`FrameFacts::main_rules`), never on this branch's wider range rule.
        let out = if facts.pinned {
            pin_in_place(&out, DrawRanges::Literal)
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
