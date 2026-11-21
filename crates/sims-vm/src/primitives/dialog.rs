//! Dialog (opcode 36): ask the player something and read back the answer.
//!
//! Read from the original's 0x451552. The instruction runs again every
//! tick until it has an answer:
//!
//! 1. It looks up the first dialog record the object owns (0x44435c). A
//!    record that was shown and not answered (status 1) is closed
//!    (0x437058 sets status 5), and a closed one counts as none.
//! 2. With operand 7 bit 0 set the dialog only informs: a record is made
//!    (0x4443ab) and the true branch taken at once.
//! 3. Otherwise, with no record, a record is made and the instruction
//!    yields. The registry shows the record at the end of the lot update
//!    (`tick.rs`), and the player's answer goes into its status.
//! 4. With a record, the record is acknowledged (0x43718f: status 2 to 4
//!    becomes 5) and the status read: not answered yet (0 or 1) yields
//!    again; the first button (2) takes the true branch; the second (3)
//!    takes the false branch, clears the temp operand 7 names in a
//!    three-button dialog, and sets registry word 0x100 to -1; the third
//!    (4) takes the false branch and sets that temp to 1.
//!
//! Operand 5 (bits 0 to 6) is the kind of dialog. Kinds 0 and 4 have one
//! button, kind 1 two and kind 2 three; these are ported. Kind 3 asks for
//! text, and kinds 5 to 15 are the game's special panels (careers,
//! neighbors, the phone and others); they fail loudly. What a dialog
//! says and shows (its text, pictures and buttons' labels) is
//! presentation and is not modeled, except where showing a person's
//! portrait builds it ([`Machine::dialog_portrait`]): building one for a
//! person type with nobody on the lot makes a person and removes it at
//! once. Making a record is otherwise assumed not to run any object's
//! code or change any simulation state: **unverified**.
//!
//! In the original, step 3 makes no record while the interface has a
//! modal dialog open (interface word 0x7c, 0x451a8e). Here modal dialogs
//! are shown and answered inside the end of the lot update, so none is
//! open while scripts run.
//!
//! The evidence (`docs/decisions/`): on the Goth house five controllers
//! reach a one-button dialog in the first tick and yield; the run of the
//! original answered each, and in the next tick each took the true
//! branch. Object 15 then made a two-button dialog, answered with the
//! second button, and took the false branch the tick after. The rest of
//! the protocol follows the original's code and the old prototype's
//! `Protocol::step`.

use crate::fault::Trap;
use crate::interpreter::{Flow, Machine};
use sims_sim::Dialog;
use std::collections::BTreeSet;

/// A definition's type word (+0x12), and its value for a person.
const TYPE: usize = 9;
const PERSON_TYPE: i16 = 2;
/// Definition word 41 (+0x52): the catalog string table (0x457d5f).
const CATALOG_STRINGS: usize = 41;

/// The tokens of the formatter's passes between `$Local`'s and
/// `$NeighborLocal`'s (0x44bf1a), in the order they run: `$Attrib`
/// (0x44c280), `$NameAttrib` (0x44c330), `$NameLocal` (0x44c3ff) and
/// `$TokenNameLocal` (0x44c490). None is ported, and a text that holds one
/// when they run fails loudly: they read the running tree's locals and
/// the stack object's attributes, and fault or throw for some (0x44c2dd,
/// 0x44c381, 0x44c4f6), and `$TokenNameLocal`'s may add an inventory
/// (0x42a8a1).
const BETWEEN: [&str; 4] = ["$Attrib", "$NameAttrib", "$NameLocal", "$TokenNameLocal"];

/// The tokens of the formatter's passes after `$Neighbor`'s and before
/// `$TimeLocal`'s, in the order they run: `$CatalogLocal` (0x6e4fa0, from
/// 0x6c40f0; 0x44c6ea), `$FameTitleLocal` (0x6e4f90, from 0x6c4100;
/// 0x44c797) and `$Grade` (0x6e5000; 0x44c861). None is ported, and a text
/// that holds one when they run fails loudly: the text they write is not
/// known to end their passes or to make no later token. `$CatalogLocal`
/// writes a catalog name (0x4573de, 0x44c771), which may hold its token
/// again, and `$FameTitleLocal` (0x405241, 0x44c83d) and `$Grade`
/// (0x4034ce; empty for some numbers, 0x44c8ae) may join the text around
/// them into a career token, which faults for an object that is not a
/// person (0x44ca27).
const AFTER: [&str; 3] = ["$CatalogLocal", "$FameTitleLocal", "$Grade"];

/// The career tokens, in the order their loop (0x44c966) runs them:
/// `$JobOffer` (0x6e5010), `$JobDesc` (0x6e4ff0), `$Job` (0x6e5040).
const CAREER: [&str; 3] = ["$JobOffer", "$JobDesc", "$Job"];

/// Whether `text`, written for a career token, may hold one past its
/// first character, or end in the start of one: a pass searches again
/// from one past the place it wrote (0x44ca9f to 0x44cad3), so it would
/// find the token again and might never end. [`crate::Program::career_texts_end`]
/// is whether no text the career passes may write does.
pub(crate) fn may_hold_career_token(text: &str) -> bool {
    text.char_indices().skip(1).any(|(i, _)| {
        let rest = &text[i..];
        rest.starts_with('$')
            && CAREER
                .iter()
                .any(|token| rest.starts_with(token) || token.starts_with(rest))
    })
}

/// `text` with each `token` replaced by `name` as the formatter's passes
/// do it (0x4558a0, 0x455750): from the start, then from one past each
/// place replaced. Never called with a name for which [`never_ends`].
fn replace_each(text: &str, token: &str, name: &str) -> String {
    let mut text = text.to_owned();
    let mut from = 0;
    while let Some(at) = text
        .get(from..)
        .and_then(|t| t.find(token))
        .map(|i| i + from)
    {
        text.replace_range(at..at + token.len(), name);
        from = at + text[at..].chars().next().map_or(1, char::len_utf8);
    }
    text
}

/// Whether a name holds `token` past its first character. Replacing the
/// token with it ([`replace_each`]) never ends in the original: each
/// search goes on from one past the place replaced (0x4558a0, 0x455750),
/// and finds the token the name put back. A token made with the text
/// after the name uses some of that text up, so it ends.
fn never_ends(token: &str, name: &str) -> bool {
    name.char_indices()
        .skip(1)
        .any(|(i, _)| name[i..].starts_with(token))
}

/// The texts a pass that replaces `token` with a name (`$Me`'s,
/// `$Object`'s, `$Neighbor`'s, at `at`) may leave from each of `texts`
/// that holds it: one for each of the `names` it may write. Each token is
/// replaced from the start, then from one past each place replaced
/// ([`replace_each`]). The pass fails loudly when the names are not known
/// (none), and when one holds the token past its first character, so the
/// pass would never end ([`never_ends`]).
fn name_pass(
    texts: BTreeSet<String>,
    token: &str,
    names: Option<&[String]>,
    at: &str,
) -> Result<BTreeSet<String>, Trap> {
    let mut out = BTreeSet::new();
    for text in texts {
        if !text.contains(token) {
            out.insert(text);
            continue;
        }
        let Some(names) = names else {
            return Err(Trap::Unsupported(format!(
                "a dialog's `{token}` is replaced by a name that is not known (0x4573d9, {at}), \
                 and a name that holds `{token}` past its first character would never end \
                 (0x4558a0, 0x455750): not ported"
            )));
        };
        if let Some(name) = names.iter().find(|name| never_ends(token, name)) {
            return Err(Trap::Unsupported(format!(
                "a dialog's `{token}` is replaced by the name {name:?}, which holds it past its \
                 first character, so the pass never ends ({at}, 0x4558a0, 0x455750)"
            )));
        }
        out.extend(names.iter().map(|name| replace_each(&text, token, name)));
    }
    Ok(out)
}

/// Fails loudly, naming `token` and its pass, when one of `texts` holds
/// one of `tokens`, whose passes are not ported.
fn not_ported(texts: &BTreeSet<String>, tokens: &[&str], passes: &str) -> Result<(), Trap> {
    for token in tokens {
        if let Some(text) = texts.iter().find(|text| text.contains(*token)) {
            return Err(Trap::Unsupported(format!(
                "a dialog's text holds `{token}` when its pass runs ({passes}): not ported \
                 ({text:?})"
            )));
        }
    }
    Ok(())
}

/// The number of a `$Local` or `$NeighborLocal` token and where it ends, read as
/// 0x44caf6 reads it from `end`, just after the token's name: a `:` and
/// digits, and an optional second `:` and digits. The number is the first
/// one's first 15 digits (0x44cb52 makes a string of 16 bytes for them,
/// and 0x46f925 adds no more than fit, 0x46f78d to 0x46f79d), by the C
/// runtime's 32-bit conversion (0x636029), which wraps; -1 when there is
/// none (0x44cb34).
///
/// The digit test (0x636993) is handed each byte sign-extended (0x44cb70),
/// so a byte above 0x7f is looked up below the C runtime's table, and
/// some pass as digits that the conversion reads as none: where the scan
/// meets one it fails loudly, naming `token`.
fn local_token(bytes: &[u8], mut end: usize, token: &str) -> Result<(i32, usize), Trap> {
    let mut number = -1i32;
    for part in 0..2 {
        if bytes.get(end) != Some(&b':') {
            break;
        }
        end += 1;
        let digits = bytes[end..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if bytes.get(end + digits).is_some_and(|b| !b.is_ascii()) {
            return Err(Trap::Unsupported(format!(
                "a dialog's `{token}` is read with digits up to a byte above 0x7f, which the \
                 original's digit test (0x636993) reads sign-extended (0x44cb70): not ported"
            )));
        }
        if part == 0 && digits > 0 {
            number = bytes[end..end + digits].iter().take(15).fold(0i32, |n, d| {
                n.wrapping_mul(10).wrapping_add(i32::from(d - b'0'))
            });
        }
        end += digits;
    }
    Ok((number, end))
}

/// Record status: made and not shown yet.
pub const NEW: u8 = 0;
/// Shown, not answered.
pub const SHOWN: u8 = 1;
/// Answered with the first, second or third button.
pub const FIRST: u8 = 2;
pub const SECOND: u8 = 3;
pub const THIRD: u8 = 4;
/// Done: the registry destroys it at the end of the lot update.
pub const DONE: u8 = 5;

/// The answers a dialog of this kind (operand 5) offers, as statuses, or
/// None for a kind that is not ported. From the original's button setup
/// (0x43601c, 0x436a52), as the old prototype read it
/// (legacy/crates/sims-vm/src/dialog/presentation.rs, `buttons`).
pub fn answers(kind: u8) -> Option<&'static [u8]> {
    match kind & 0x7f {
        0 | 4 => Some(&[FIRST]),
        1 => Some(&[FIRST, SECOND]),
        2 => Some(&[FIRST, SECOND, THIRD]),
        _ => None,
    }
}

/// Whether a record's dialog stops the game until it is answered (operand
/// 7 bit 7 clear; record byte 0x69).
pub fn modal(operands: &[u8; 8]) -> bool {
    operands[7] & 0x80 == 0
}

/// Whether a record only informs: it is answered once and is done (operand
/// 7 bit 0).
pub fn informs(operands: &[u8; 8]) -> bool {
    operands[7] & 1 != 0
}

impl Machine<'_, '_> {
    pub(crate) fn dialog(&mut self, operands: &[u8; 8]) -> Result<Flow, Trap> {
        let kind = operands[5] & 0x7f;
        if answers(kind).is_none() {
            return Err(Trap::Unsupported(format!(
                "Dialog kind {kind} (operand 5) is {}, which is not ported (0x451552)",
                if kind == 3 {
                    "a dialog that asks for text"
                } else {
                    "one of the game's special panels"
                }
            )));
        }
        let me = self.me;
        let records = &mut self.world.dialogs.records;
        let mut found = records.iter().position(|record| record.caller == me);
        if let Some(index) = found {
            match records[index].status {
                SHOWN => {
                    records[index].status = DONE;
                    found = None;
                }
                DONE => found = None,
                _ => {}
            }
        }
        let record = Dialog {
            caller: me,
            operands: *operands,
            status: NEW,
        };
        if informs(operands) {
            self.dialog_portrait(operands)?;
            self.world.dialogs.records.push(record);
            return Ok(Flow::Branch(true));
        }
        let Some(index) = found else {
            self.dialog_portrait(operands)?;
            self.world.dialogs.records.push(record);
            return Ok(Flow::Yield);
        };
        let records = &mut self.world.dialogs.records;
        let status = records[index].status;
        if (FIRST..=THIRD).contains(&status) {
            records[index].status = DONE;
        }
        let temp = usize::from((operands[7] >> 4) & 7);
        Ok(match status {
            NEW | SHOWN => Flow::Yield,
            FIRST => Flow::Branch(true),
            SECOND => {
                if kind == 2 {
                    self.set_temp(temp, 0)?;
                }
                self.world.dialogs.declined = -1;
                Flow::Branch(false)
            }
            THIRD => {
                if kind == 2 {
                    self.set_temp(temp, 1)?;
                }
                Flow::Branch(false)
            }
            _ => Flow::Branch(true),
        })
    }

    /// The portrait an ordinary dialog's record shows, built as the
    /// record is made (0x4443ab, 0x435d8a, then 0x43601c), before it joins
    /// the registry. The icon mode is operand 7's bits 1 to 3 (0x436279):
    ///
    /// - mode 0 (0x436308): the type the dialog's text found, or else the
    ///   type the definition of the object the stack object names stands
    ///   for, when it is on the lot (a tile's group master's, 0x45728d).
    ///   The record's strings are formatted (0x44bf1a, from 0x436219) with
    ///   a place for a type (`[ebp-0x24]`), which mode 0 reads first
    ///   ([`Machine::dialog_text_type`]);
    /// - mode 2 (0x4362f4): the type of the neighbor record the stack
    ///   object names (0x425afe: 0x4256f4, record +8);
    /// - modes 1 and 4 to 7 show no person (the type stays 0 from
    ///   0x436271), and mode 3 shows a picture of the type's
    ///   (0x4362ca, 0x436362).
    ///
    /// A type that is a person (definition +0x12 is 2, 0x4575d2) has its
    /// entry in the interface's table of person types looked up by GUID
    /// (0x4572af, 0x4e9753), and 0x4e92a8 builds its first image when the
    /// entry has none: 0x4e8c5d draws the first person of that type on
    /// the lot (0x4e9608 walks the lot's people, +0x84). With none, it
    /// makes one (0x4e9652: the factory 0x440b2d, [`crate::Interpreter::create_person`]),
    /// finds it a tile from the house view's centre (0x4347f0 from
    /// 0x4a677c) and puts it there (virtual function 0x88), draws it,
    /// and removes it at once (0x44123f, [`crate::Interpreter::remove_now`]).
    /// The table holds every person type the catalog had when the game
    /// started (0x4e8b3d); a type made later ([`sims_sim::World::characters`]) may
    /// not be in it, and fails loudly.
    ///
    /// Which entries have their image is kept in [`sims_sim::World::portraits`].
    /// Only the dialog's builds are counted. The household's panel draws
    /// the family's portraits too, so a family member's type with nobody
    /// on the lot fails loudly.
    ///
    /// The put is [`crate::Interpreter::portrait_put`]: the search from the
    /// house view's centre (an input, [`crate::Interpreter::view_centre`])
    /// and as much of the put as is the same on every tile it may end on.
    /// It fails loudly where the tile would matter, including when no tile
    /// is found: the original then removes the person without drawing it
    /// (0x4e9706), the entry stays without its image, and 0x4e938c builds
    /// it again in the same dialog unless the type has image 2002, which is
    /// not ported. The rest of the put (the rooms, 0x440385, and the tiles,
    /// 0x4330d7) is taken to be undone by the removal: **unverified**.
    pub(crate) fn dialog_portrait(&mut self, operands: &[u8; 8]) -> Result<(), Trap> {
        let stack_object = self.frame()?.stack_object;
        let mode = (operands[7] >> 1) & 7;
        // The record's strings are formatted in every mode, before the
        // mode is read (0x436219 to 0x436265; mode 4 one more, 0x4362c0).
        let text_type = self.dialog_text_type(operands, mode)?;
        let guid = match mode {
            0 => match text_type {
                Some(guid) => guid,
                None => return Ok(()),
            },
            2 => match self.world.neighbors.get(stack_object) {
                Some(record) => record.guid,
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
        if self.world.portraits.contains(&guid) {
            return Ok(());
        }
        if !self.listed_person_type(guid, "a dialog's portrait")? {
            return Ok(());
        }
        let on_lot = self.world.people.iter().any(|id| {
            self.world
                .objects
                .get(id)
                .is_some_and(|object| object.guid == guid)
        });
        if !on_lot {
            let family = i32::from(self.world.globals.get(9).copied().unwrap_or(0));
            if self
                .world
                .families
                .iter()
                .any(|f| f.id == family && f.members.contains(&guid))
            {
                return Err(Trap::Unsupported(format!(
                    "a dialog's portrait of the household's type 0x{guid:08X} with nobody of \
                     it on the lot (0x4e9652): whether the household's panel built it before \
                     is not kept"
                )));
            }
            let interpreter = self.interpreter();
            let unsupported = |fault| {
                Trap::Unsupported(format!(
                    "making a person of type 0x{guid:08X} for a dialog's portrait (0x4e9652): \
                     {fault}"
                ))
            };
            // On a copy of the lot and of the machine's cells (the sounds
            // the person's Init plays, the objects deleted), kept only when
            // the whole build is done: a build that fails leaves nothing
            // behind (the original removes the person on every path,
            // 0x4e8fdb, 0x4e9706).
            let mut copy = self.world.clone();
            let cells = crate::both_ways::Cells::of(&interpreter);
            let built = cells.on(&interpreter);
            let id = built.create_person(&mut copy, guid).map_err(unsupported)?;
            built
                .portrait_put(&mut copy, id)
                .map_err(|fault| fault.trap)?;
            built.remove_now(&mut copy, id).map_err(unsupported)?;
            // The refresh pending is the caller's to keep, and a build
            // that changed it is not ported.
            if !cells.same_refresh(&crate::both_ways::Cells::of(&interpreter)) {
                return Err(Trap::Unsupported(format!(
                    "making a person of type 0x{guid:08X} for a dialog's portrait (0x4e9652) \
                     changed the refresh pending, which is not ported"
                )));
            }
            cells.keep(&interpreter, self.world, copy);
        }
        self.world.portraits.insert(guid);
        Ok(())
    }

    /// Whether type `guid` is in the interface's table of person types
    /// (0x4e9753, which gives its place, or -1): the table holds every
    /// person type the catalog had when the game started (0x4e8b3d). A
    /// type made since ([`sims_sim::World::characters`]) may not be in it,
    /// and fails loudly, naming `what` asked.
    fn listed_person_type(&self, guid: u32, what: &str) -> Result<bool, Trap> {
        if self.world.characters.made.contains_key(&guid) {
            return Err(Trap::Unsupported(format!(
                "{what} of type 0x{guid:08X}, made while the game ran: whether the interface's \
                 table of person types (0x6f2698) has it is not known"
            )));
        }
        Ok(
            crate::interpreter::object_definition(self.catalog, self.world, guid).is_some()
                && self.definition_words(guid)?[TYPE] == PERSON_TYPE,
        )
    }

    /// The strings a dialog's record loads, from STR# 301 of the running
    /// code's file (0x4360b8 then 0x416920, 0x47028b, then 0x4369de), in
    /// the order they are formatted (0x44bf1a, 0x436219 to 0x436265): the
    /// buttons' labels its kind (operand 5) loads (0x436175: kinds 0, 3
    /// and 4 operand 3, kind 1 operands 4 and 3, kind 2 operands 0, 4 and
    /// 3), formatted in the order operand 3, 4, 0; then the title
    /// (operand 6) and the message (operand 2). A string is named by its
    /// number from 1. In mode 4, operand 1's string comes last.
    ///
    /// A string that is number 0, that the file lacks or that is empty is
    /// left out: 0x4369de puts in its place the default the caller gives
    /// (0x436a21 to 0x436a35), a button's label from STR# 142 of
    /// `GameData/UIText.iff` (0x435fee, 0x43600a), or else "Missing
    /// string!!!" (0x6c29bc). The formatter reads them too, so they are
    /// left out only when the labels are known to hold no `$`
    /// ([`crate::Program::labels_hold_no_token`]); otherwise a dialog with
    /// one fails loudly.
    ///
    /// The table is read in the application's language: the table's
    /// language byte is the interface's +0x64 less one (0x4702b1 to
    /// 0x4702bb), and 0x4704fa reads that language's strings, with no
    /// other to fall back on. The port reads US English (language 1,
    /// [`crate::program::Library::strings`]), the recorded install's (its
    /// registry has `SIMS_LANGUAGE` USEnglish, which 0x4f2ddc to 0x4f2df8
    /// read after `SIMS_OTHERLANGUAGE`; the interface's +0x64 for it is
    /// **unverified**). An install in another language is not supported:
    /// its strings may hold other tokens.
    ///
    /// The table is the one the original's loader finds
    /// ([`crate::program::Library::loaded_strings`]): the resource of the
    /// ID, its entries in the loader's order. The loader's other ways are
    /// not ported, and fail loudly when the dialog names a string: a
    /// table it reads otherwise, such as one whose entries have a language
    /// and none is US English, which it reads again for English (0x47006b
    /// to 0x4700d0; [`crate::program::Library::unported_strings`]), and a
    /// file with no STR# 301 but a legacy CST 301, which it reads instead
    /// (0x4702c7; [`crate::program::Library::legacy_strings`]).
    pub(crate) fn dialog_strings(&self, operands: &[u8; 8]) -> Result<Vec<String>, Trap> {
        const TABLE: u16 = 301;
        let owner = self.owner(self.frame()?)?;
        let library = self.program.libraries.get(&owner);
        let strings = library.and_then(|library| match library.loaded_strings.get(&TABLE) {
            Some(loaded) => loaded.as_ref(),
            None => library.strings.get(&TABLE),
        });
        // Mode 4 formats operand 1's string too, after the others
        // (0x436297 to 0x4362c0, through 0x4369de as they are).
        let mode_4: &[usize] = if (operands[7] >> 1) & 7 == 4 {
            &[1]
        } else {
            &[]
        };
        let buttons: &[usize] = match operands[5] & 0x7f {
            0 | 3 | 4 => &[3],
            1 => &[3, 4],
            2 => &[3, 4, 0],
            _ => &[],
        };
        let named = buttons
            .iter()
            .chain(&[6, 2])
            .chain(mode_4)
            .any(|&operand| operands[operand] != 0);
        if named && let Some(library) = library {
            if let Some(why) = library.unported_strings.get(&TABLE) {
                return Err(Trap::Unsupported(format!(
                    "a dialog's STR# 301 {why}: not ported"
                )));
            }
            if strings.is_none() && library.legacy_strings.contains(&TABLE) {
                return Err(Trap::Unsupported(
                    "a dialog's file has no STR# 301 but a legacy CST 301, which the \
                     original's loader reads instead (0x4702c7, 0x47018e to 0x4701e6): not \
                     ported"
                        .into(),
                ));
            }
        }
        let mut texts = Vec::new();
        for &operand in buttons.iter().chain(&[6, 2]).chain(mode_4) {
            let text = usize::from(operands[operand])
                .checked_sub(1)
                .and_then(|index| strings?.get(index))
                .filter(|text| !text.is_empty());
            match text {
                Some(text) => texts.push(text.clone()),
                None if !self.program.labels_hold_no_token => {
                    return Err(Trap::Unsupported(
                        "a dialog's string is its default (0x4369de, 0x436a21 to 0x436a35), \
                         a button's label from STR# 142 of `GameData/UIText.iff` (0x435fee, \
                         0x43600a), and the labels are not known to hold no token: not ported"
                            .into(),
                    ));
                }
                None => {}
            }
        }
        Ok(texts)
    }

    /// The type a dialog of icon mode 0 shows (None for the other modes):
    /// the type the formatter (0x44bf1a) finds in the record's strings
    /// ([`Machine::dialog_strings`]), each formatted in turn with the same
    /// place for it (`[ebp-0x24]`, cleared at 0x43620d), so the last one
    /// found is kept; or, with none found, the type the stack object's
    /// definition stands for ([`Machine::master_type`], 0x436308 to
    /// 0x43634d). The passes run in this order:
    ///
    /// 1. `$FamilyAssets` (0x44bf66) and `$Family` (0x44c034): not ported,
    ///    and a text that holds `$Family` fails loudly;
    /// 2. `$Me` (0x6e5060; 0x44c0fc): the running object's name (the
    ///    formatter's `this` is the record's object, 0x436200);
    /// 3. `$Object` (0x6e5050, from 0x6c407c; 0x44c155 to 0x44c1af): the
    ///    name and type of the object the stack object names (frame +4),
    ///    when there is one; otherwise the token stays (0x44c183 to
    ///    0x44c1a6);
    /// 4. `$Local` ([`Machine::locals_written`]);
    /// 5. the passes of [`BETWEEN`]: not ported, and a text that holds one
    ///    of their tokens fails loudly;
    /// 6. `$NeighborLocal` ([`Machine::neighbor_locals`]);
    /// 7. `$Neighbor` (0x6e5090, from 0x6c4054; 0x44c65f to 0x44c6e8): the
    ///    name and type of the neighbor record the stack object names
    ///    (0x425afe), when there is one; otherwise the token stays. It
    ///    comes after `$Object`'s, so it wins in a string that holds both;
    /// 8. the passes of [`AFTER`]: not ported, and a text that holds one
    ///    of their tokens fails loudly;
    /// 9. `$TimeLocal` (0x6e5020, 0x44c8c4): an hour local in [0, 24)
    ///    written in one of fixed formats (0x4a6ab1), which hold no token;
    /// 10. the career's: `$JobOffer`, `$JobDesc` and `$Job` ([`CAREER`];
    ///     0x44c966). Each token found casts the running object to a
    ///     person (0x44c9bc) and reads it (0x44ca27, also with the numbers
    ///     not locals), which faults for an object that is not one: that
    ///     fails loudly. For a person they write career texts, which end
    ///     the passes only when they hold no career token past their first
    ///     character: that fails loudly unless
    ///     [`crate::Program::career_texts_end`].
    ///
    /// The passes after `$Neighbor`'s write text and read the running
    /// object, the locals and the career tables, and are taken to change
    /// nothing else (**unverified**).
    ///
    /// The names are the type's catalog names (0x4573d9,
    /// [`Machine::catalog_names`]), each replaced as [`name_pass`] does
    /// it. Which catalog entry is the name is not ported, so the text is
    /// worked out for each; mode 0 fails loudly where that decides the
    /// portrait. The checks run in every mode: the strings are formatted
    /// before the mode is read.
    pub(crate) fn dialog_text_type(
        &self,
        operands: &[u8; 8],
        mode: u8,
    ) -> Result<Option<u32>, Trap> {
        let stack_object = self.frame()?.stack_object;
        let object = u16::try_from(stack_object)
            .ok()
            .and_then(|id| self.world.objects.get(&id))
            .map(|object| object.guid);
        let neighbor = self.world.neighbors.get(stack_object).map(|r| r.guid);
        // Mode 4's last string is formatted with no place for a type
        // (0x4362ba pushes 0), but only mode 0 reads the place.
        // What may have been found so far: more than one where the name a
        // pass writes may be one of several.
        let mut found = BTreeSet::from([None]);
        for text in self.dialog_strings(operands)? {
            if text.contains("$Family") {
                return Err(Trap::Unsupported(
                    "a dialog's text holds `$FamilyAssets` or `$Family`, whose passes (0x44bf66, \
                     0x44c034) are not ported"
                        .into(),
                ));
            }
            let mut texts = BTreeSet::from([text]);
            if texts.iter().any(|text| text.contains("$Me")) {
                let names = self.catalog_names(self.me()?.guid)?;
                texts = name_pass(texts, "$Me", names.as_deref(), "0x44c13e")?;
            }
            // What this string may do to what was found: None keeps it,
            // Some(type) puts that in its place.
            let mut outcomes = BTreeSet::new();
            for text in texts {
                let mut texts = BTreeSet::from([text]);
                let mut written = None;
                if let Some(guid) = object.filter(|_| texts.iter().any(|t| t.contains("$Object"))) {
                    written = Some(guid);
                    let names = self.catalog_names(guid)?;
                    texts = name_pass(texts, "$Object", names.as_deref(), "0x44c1ca")?;
                }
                let texts = texts
                    .iter()
                    .map(|text| self.locals_written(text))
                    .collect::<Result<BTreeSet<_>, _>>()?;
                not_ported(&texts, &BETWEEN, "0x44c280 to 0x44c590")?;
                for text in texts {
                    self.neighbor_locals(&text)?;
                    let mut texts = BTreeSet::from([text]);
                    let mut written = written;
                    if let Some(guid) =
                        neighbor.filter(|_| texts.iter().any(|t| t.contains("$Neighbor")))
                    {
                        written = Some(guid);
                        let names = self.catalog_names(guid)?;
                        texts = name_pass(texts, "$Neighbor", names.as_deref(), "0x44c6ba")?;
                    }
                    not_ported(&texts, &AFTER, "0x44c6ea to 0x44c8c4")?;
                    if let Some(text) = texts.iter().find(|text| text.contains("$Job")) {
                        if self.me()?.person.is_none() {
                            return Err(Trap::Unsupported(format!(
                                "a dialog's text holds a career token (`$JobOffer`, `$JobDesc`, \
                                 `$Job`), whose pass casts the object running the tree to a \
                                 person (0x44c9bc) and reads it (0x44ca27): it is not one, so \
                                 the original faults ({text:?})"
                            )));
                        }
                        if !self.program.career_texts_end {
                            return Err(Trap::Unsupported(format!(
                                "a dialog's text holds a career token, and the career texts \
                                 are not known to hold none past their first character, which \
                                 would never end its pass (0x44ca9f to 0x44cad3): not ported \
                                 ({text:?})"
                            )));
                        }
                    }
                    outcomes.insert(written);
                }
            }
            found = outcomes
                .into_iter()
                .flat_map(|outcome| match outcome {
                    None => found.clone(),
                    Some(guid) => BTreeSet::from([Some(guid)]),
                })
                .collect();
        }
        if mode != 0 {
            return Ok(None);
        }
        // Mode 0 shows, when nothing was found, the type the stack object's
        // definition stands for (0x436308 to 0x43634d).
        let fallback = match object {
            Some(guid) => self.master_type(guid)?,
            None => None,
        };
        let shown: BTreeSet<_> = found.iter().map(|f| f.or(fallback)).collect();
        if shown.len() > 1 {
            return Err(Trap::Unsupported(
                "a dialog's `$Me`, `$Object` or `$Neighbor` is replaced by one of a type's \
                 catalog names (0x4573d9, 0x4061e3, which picks one, is not ported), and which \
                 one decides whether a later pass finds `$Object` or `$Neighbor` (0x44c1af, \
                 0x44c69f), so the portrait"
                    .into(),
            ));
        }
        Ok(shown.into_iter().next().flatten())
    }

    /// The type of the definition type `guid`'s definition stands for
    /// ([`crate::Program::master_definition`], 0x45728d through 0x43ae0a):
    /// its own for a definition alone, its group's master's for a tile.
    /// None for a tile whose group has no master: mode 0 then has no type
    /// and shows no portrait (0x436345 to 0x43634d leave the place null).
    fn master_type(&self, guid: u32) -> Result<Option<u32>, Trap> {
        let definition = self.definition(guid)?;
        let master = self
            .program
            .master_definition(definition.resource, definition.chunk)
            .map_err(Trap::Invalid)?;
        Ok(master.map(|words| {
            u32::from(words[crate::factory::word::GUID] as u16)
                | u32::from(words[crate::factory::word::GUID + 1] as u16) << 16
        }))
    }

    /// The names the formatter may write for a token of type `guid`
    /// (0x4573d9), or none when they are not known. The name is an entry
    /// of the catalog string table definition word 41 names (0x457d3b,
    /// 0x4061e3), or the definition's own name if that entry is empty
    /// (0x457db4). Which entry 0x4061e3 picks is not ported, so every
    /// entry may be the one; they are known only when the table is there,
    /// has an entry in the first language (the one every string table is
    /// read in), and every entry is not empty, and when the file has no
    /// legacy catalog string table (CATS), which 0x4061e3 checks for first
    /// ([`crate::program::Library::legacy_catalog_strings`]).
    fn catalog_names(&self, guid: u32) -> Result<Option<Vec<String>>, Trap> {
        let table = self.definition_words(guid)?[CATALOG_STRINGS] as u16;
        let definition = self.definition(guid)?;
        let Some(library) = self.program.libraries.get(&definition.resource) else {
            return Ok(None);
        };
        if !library.legacy_catalog_strings.is_empty() {
            return Ok(None);
        }
        Ok(library.catalog_strings.get(&table).and_then(|entries| {
            (entries.iter().any(|(language, _)| *language == 1)
                && entries.iter().all(|(_, text)| !text.is_empty()))
            .then(|| entries.iter().map(|(_, text)| text.clone()).collect())
        }))
    }

    /// `text` after the formatter's `$Local` pass (0x44c1f2 to 0x44c27d).
    /// Each token (0x6e5030), searched for from one past the last
    /// (0x44c25f), is read with its `:` and number, and an optional second
    /// `:` and number ([`local_token`], 0x44caf6). When the number is one
    /// of the running tree's locals (from 0, below the count the tree has,
    /// frame +6, compared signed), the token and what was read with it are
    /// replaced (0x4557c0) by the local's value in decimal (0x46f941,
    /// `%d` of the word sign-extended). Otherwise the token stays.
    fn locals_written(&self, text: &str) -> Result<String, Trap> {
        const TOKEN: &str = "$Local";
        let locals = &self.frame()?.locals;
        let mut text = text.to_owned();
        let mut from = 0;
        while let Some(at) = text
            .get(from..)
            .and_then(|t| t.find(TOKEN))
            .map(|i| i + from)
        {
            let (number, end) = local_token(text.as_bytes(), at + TOKEN.len(), TOKEN)?;
            if let Ok(n) = usize::try_from(number)
                && n < locals.len()
            {
                self.check_local(n).map_err(|trap| {
                    Trap::Unsupported(format!("a dialog's `$Local:{n}` (0x44c240): {trap}"))
                })?;
                text.replace_range(at..end, &locals[n].to_string());
            }
            from = at + 1;
        }
        Ok(text)
    }

    /// The formatter's `$NeighborLocal` pass (0x44c590 to 0x44c65a) on
    /// `text`. Each token (0x6e4fc0), searched for from one past the last
    /// (0x44c643), is read with its `:` and number, and an optional second
    /// `:` and number ([`local_token`]). When the number is one of the
    /// running tree's locals (from 0, below the count the tree has, frame
    /// +6, compared signed), the local names a neighbor record (0x44c5f4,
    /// 0x4256f4) and the record's type is in the interface's table of
    /// person types (0x4572af, 0x4e9753), the token is replaced by the
    /// type's name from the interface's table (0x4e9c46, 0x4e893e, 0x455750),
    /// which is not ported, and fails loudly; a record whose type is not
    /// in the table ends the pass (0x44c616 to 0x44c653). Otherwise the
    /// token stays, and the `$Neighbor` pass finds it.
    pub(crate) fn neighbor_locals(&self, text: &str) -> Result<(), Trap> {
        const TOKEN: &str = "$NeighborLocal";
        let locals = &self.frame()?.locals;
        let mut from = 0;
        while let Some(at) = text
            .get(from..)
            .and_then(|t| t.find(TOKEN))
            .map(|i| i + from)
        {
            let (number, _) = local_token(text.as_bytes(), at + TOKEN.len(), TOKEN)?;
            if let Ok(n) = usize::try_from(number)
                && n < locals.len()
            {
                self.check_local(n).map_err(|trap| {
                    Trap::Unsupported(format!(
                        "a dialog's `$NeighborLocal:{n}` (0x44c5f4): {trap}"
                    ))
                })?;
                if let Some(record) = self.world.neighbors.get(locals[n]) {
                    if !self.listed_person_type(record.guid, "a dialog's $NeighborLocal")? {
                        break;
                    }
                    return Err(Trap::Unsupported(format!(
                        "a dialog's `$NeighborLocal:{n}` is replaced by the name of type \
                         0x{:08X} from the interface's table (0x4e9c46, 0x4e893e): not ported",
                        record.guid
                    )));
                }
            }
            from = at + 1;
        }
        Ok(())
    }

    fn set_temp(&mut self, temp: usize, value: i16) -> Result<(), Trap> {
        let object = self.me_mut()?;
        let slot = object
            .temps
            .get_mut(temp)
            .ok_or(Trap::native(2, "temp out of range"))?;
        *slot = value;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::may_hold_career_token;

    #[test]
    fn a_career_token_past_the_first_character_or_the_start_of_one_at_the_end() {
        for text in ["a $Job", "x$JobDesc:0", "Clerk $Jo", "B$", "$$Job"] {
            assert!(may_hold_career_token(text), "{text}");
        }
        for text in [
            "$Job",
            "B004MA$b_01",
            "business$g$c_01",
            "Clerk",
            "",
            "$JobOffer",
        ] {
            assert!(!may_hold_career_token(text), "{text}");
        }
    }
}
