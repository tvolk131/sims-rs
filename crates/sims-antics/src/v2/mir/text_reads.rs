//! Which frame words an object's text may read.
//!
//! `Dialog` and `Add/Change Action String` format strings of the running tree's own file, and the
//! formatter reads a frame word only through a token that names it (`$Local:N` and the other
//! readers in `verify::text`). A compiled v2 object's file holds exactly the strings its source
//! gives it: its `strings` tables, the `STR#` 301 entries its dialogs' inline literals add, and
//! the catalog strings (`CTSS`: each catalog name and description). So the words any of its
//! text can read are the union over those strings, read by the verifier's own token rules
//! ([`crate::verify::string_locals`]). A string with a token whose number cannot be known makes
//! the whole union [`TableLocals::Unknown`].
//!
//! A numeric string index the object does not hold reads nothing in the compiled object. Text
//! from any other file (a host file a compiled tree is spliced into) is not the compiler's to
//! see: under the v2 frame contract only `@slot(N)` fixes the word an unpinned local lives in.

use super::*;

pub(crate) use crate::verify::TableLocals;

/// The frame words that any of `texts` may make the formatter read.
pub(crate) fn strings_read<'a>(texts: impl IntoIterator<Item = &'a str>) -> TableLocals {
    let mut all = TableLocals::Known(BTreeSet::new());
    for text in texts {
        match (&mut all, crate::verify::string_locals(text)) {
            (_, TableLocals::Unknown) => return TableLocals::Unknown,
            (TableLocals::Known(set), TableLocals::Known(more)) => set.extend(more),
            (TableLocals::Unknown, TableLocals::Known(_)) => unreachable!("returned above"),
        }
    }
    all
}

/// Whether text that reads `reads` may read any frame word at all.
pub(crate) fn reads_a_word(reads: &TableLocals) -> bool {
    !matches!(reads, TableLocals::Known(set) if set.is_empty())
}
