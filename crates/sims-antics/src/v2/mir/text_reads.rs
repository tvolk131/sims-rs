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
//! see: under the v2 frame contract only `#[slot(N)]` fixes the word an unpinned local lives in.
//!
//! One string outside the object is formatted too: a dialog button whose label string is
//! missing gets the game's default label (`STR#` 142 of `GameData/UIText.iff`). The fact
//! assumes those labels hold no token, as `sims-vm` does: it formats a dialog with a default
//! label only after checking the install's labels hold no `$`
//! (`sims_vm::labels_hold_no_token`), and fails loudly otherwise.

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

/// [`strings_read`] apart from what a `$Me` name may name. `$Me` writes the running object's
/// name into the text before every other pass, which then read it as text: a name holding a
/// token (`$Local:N`) may read any word, and that is what this fact leaves out (each `$Me`
/// counts as a space). A name without `$` can still complete a token the source starts right
/// before it (`$Local:$Me` with the name `0` reads word 0, `$Loc$Me` with `al:0` too), so a
/// `$Me` right after a letter, digit, `_`, `-`, `:` or `$` leaves the fact unknown.
pub(crate) fn strings_read_besides_me<'a>(texts: impl IntoIterator<Item = &'a str>) -> TableLocals {
    let mut neutral = Vec::new();
    for text in texts {
        let completes_a_token = text.match_indices("$Me").any(|(at, _)| {
            text[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ':' | '$'))
        });
        if completes_a_token {
            return TableLocals::Unknown;
        }
        neutral.push(text.replace("$Me", " "));
    }
    strings_read(neutral.iter().map(String::as_str))
}

/// Whether text that reads `reads` may read any frame word at all.
pub(crate) fn reads_a_word(reads: &TableLocals) -> bool {
    !matches!(reads, TableLocals::Known(set) if set.is_empty())
}
