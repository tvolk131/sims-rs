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

/// The tokens of the formatter's passes between `$Object`'s and
/// `$NeighborLocal`'s (0x44bf1a) that write a name, in the order they run:
/// `$NameAttrib` (0x44c330), `$NameLocal` (0x44c3ff) and `$TokenNameLocal`
/// (0x44c490). They are not ported, and the names are not known.
const LATER_NAMES: [&str; 3] = ["$NameAttrib", "$NameLocal", "$TokenNameLocal"];

/// The tokens of the passes between `$Object`'s and `$NeighborLocal`'s
/// that write a number (a data word through 0x46f941, put in by
/// 0x4557c0): `$Local` (0x44c1f2) and `$Attrib` (0x44c280). A number has
/// no `$` and no letter, so it cannot write `$Neighbor` or `$Object`.
const LATER_NUMBERS: [&str; 2] = ["$Local", "$Attrib"];

/// The tokens of the passes before `$TokenNameLocal`'s that write a
/// number (0x46f941, put in by 0x4557c0): `$FamilyAssets` (0x44bf66),
/// `$Local` and `$Attrib`. One after `$TokenNameLocal:` gives that token
/// its local.
const NUMBERS: [&str; 3] = ["$FamilyAssets", "$Local", "$Attrib"];

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

/// The number of a `$NeighborLocal` or `$TokenNameLocal` token and where
/// it ends, read as 0x44caf6 reads it from `end`, just after the token's
/// name: a `:` and digits, and an optional second `:` and digits. The
/// number is the first one, by the C runtime's 32-bit conversion
/// (0x636029), which wraps; -1 when there is none.
fn local_token(bytes: &[u8], mut end: usize) -> (i32, usize) {
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
        if part == 0 && digits > 0 {
            number = bytes[end..end + digits].iter().fold(0i32, |n, d| {
                n.wrapping_mul(10).wrapping_add(i32::from(d - b'0'))
            });
        }
        end += digits;
    }
    (number, end)
}

/// One text a dialog's string may be after the passes so far.
#[derive(Clone)]
struct Text {
    text: String,
    /// Whether a name not known stands in it: it may hold any token.
    unknown: bool,
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
                "Dialog kind {kind} (operand 5) is {}, which is not ported",
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
    /// - mode 0 (0x436308): the type of the object the stack object names,
    ///   when it is on the lot, unless the dialog's text named a neighbor
    ///   whose type it shows instead. The record's strings are formatted
    ///   (0x44bf1a, from 0x436219) with a place for a type (`[ebp-0x24]`),
    ///   which mode 0 reads first ([`Machine::dialog_text_type`]);
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
                None => match u16::try_from(stack_object)
                    .ok()
                    .and_then(|id| self.world.objects.get(&id))
                {
                    Some(object) => object.guid,
                    None => return Ok(()),
                },
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
    /// number from 1. One the file lacks, and a button's default label
    /// (number 0, 0x435f4e), are left out: taken to hold no token
    /// (**unverified**). In mode 4, operand 1's string comes last.
    ///
    /// The table is read in the application's language: the table's
    /// language byte is the interface's +0x64 less one (0x4702b1 to
    /// 0x4702bb), and 0x4704fa reads that language's strings, with no
    /// other to fall back on. The port reads US English (language 1,
    /// [`crate::program::Library::strings`]), the recorded install's. An
    /// install in another language is not supported: its strings may hold
    /// other tokens.
    pub(crate) fn dialog_strings(&self, operands: &[u8; 8]) -> Result<Vec<String>, Trap> {
        let owner = self.owner(self.frame()?)?;
        let strings = self
            .program
            .libraries
            .get(&owner)
            .and_then(|library| library.strings.get(&301));
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
        Ok(buttons
            .iter()
            .chain(&[6, 2])
            .chain(mode_4)
            .filter_map(|&operand| {
                let index = usize::from(operands[operand]).checked_sub(1)?;
                strings?.get(index).cloned()
            })
            .collect())
    }

    /// The type the formatter (0x44bf1a) finds in a dialog record's
    /// strings ([`Machine::dialog_strings`]), each formatted in turn with
    /// the same place for it (`[ebp-0x24]`, cleared at 0x43620d), so the
    /// last one found is kept. Only two tokens write it:
    ///
    /// - `$Object` (0x6e5050, from 0x6c407c; 0x44c155 to 0x44c1af): the
    ///   type of the object the stack object names (frame +4), when there
    ///   is one;
    /// - `$Neighbor` (0x6e5090, from 0x6c4054; 0x44c65f to 0x44c69f): the
    ///   type of the neighbor record the stack object names (0x425afe),
    ///   when there is one. Its pass comes after `$Object`'s, so it wins
    ///   in a string that holds both. It finds `$Neighbor` anywhere in the
    ///   text as the passes before it left it, so only after the
    ///   `$NeighborLocal` pass ([`Machine::neighbor_locals_named`]), which
    ///   may have put a name in place of a `$NeighborLocal` token.
    ///
    /// The passes before them may put a name or a number in the text, and
    /// so decide what they find. The passes run in this order (0x44bf1a):
    /// `$FamilyAssets`, `$Family`, `$Me`, `$Object`, `$Local`, `$Attrib`,
    /// `$NameAttrib`, `$NameLocal`, `$TokenNameLocal`, `$NeighborLocal`,
    /// `$Neighbor`. `$Me` and `$Object` are ported ([`Machine::name_pass`]).
    /// `$Family` writes a name not known when the stack object names a
    /// neighbor record (0x44c064; the family, 0x425d69, is not looked up).
    /// The others are not ported, and a text where they may change what
    /// is found leaves it unknown (see [`LATER_NAMES`], [`LATER_NUMBERS`]);
    /// in mode 0 the dialog fails loudly where that decides the portrait.
    /// In every mode the passes that would never end, and the
    /// `$TokenNameLocal` pass's work ([`Machine::token_name_locals`]), fail
    /// loudly, after the portrait.
    ///
    /// The other tokens' text is presentation, and is not worked out. The
    /// type is returned for mode 0 only.
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
        let strings = self.dialog_strings(operands)?;
        // Mode 4's last string is formatted with no place for a type
        // (0x4362ba pushes 0), but only mode 0 reads the place.
        // The names `$Object` may be replaced by; none when not known.
        // With no object the token stays as `(name)`.
        let object_names = match object {
            Some(guid) => self.catalog_names(guid)?,
            None => Some(vec!["(name)".to_owned()]),
        };
        // The running object's names, for `$Me` (the formatter's `this`
        // is the record's object, 0x436200, the object running the tree);
        // read only when a text may hold the token.
        let me_names = if strings
            .iter()
            .any(|text| text.contains("$Me") || text.contains("$Family"))
        {
            self.catalog_names(self.me()?.guid)?
        } else {
            Some(Vec::new())
        };
        // What may have been found so far: more than one where a name not
        // known, or a pass not ported, may decide it.
        let mut found = BTreeSet::from([None]);
        // The `$TokenNameLocal` pass's fault: kept until the portrait is
        // worked out, so a portrait that is not known names that first.
        let mut deferred = None;
        for text in strings {
            // `$FamilyAssets` (0x44bf66) writes a number. `$Family`
            // (0x44c034) writes the name of the family of the neighbor
            // record the stack object names (0x4256f4, record +0xee,
            // 0x425d69): not known. It finds `$Family` in a
            // `$FamilyAssets` token left too.
            let family = text.contains("$Family") && neighbor.is_some();
            let texts = vec![Text {
                text,
                unknown: family,
            }];
            let texts = self.name_pass(texts, "$Me", me_names.as_deref(), "0x44c13e")?;
            // What this string may do to what was found: None keeps it,
            // Some(type) puts that in its place.
            let mut outcomes = BTreeSet::new();
            for text in texts {
                // `$Object`'s pass writes the stack object's type when it
                // finds the token (0x44c1af); a name not known written
                // before it may hold one.
                let mut kept = BTreeSet::new();
                if object.is_some() && text.text.contains("$Object") {
                    kept.insert(Some(object));
                } else {
                    kept.insert(None);
                    if object.is_some() && text.unknown {
                        kept.insert(Some(object));
                    }
                }
                let texts =
                    self.name_pass(vec![text], "$Object", object_names.as_deref(), "0x44c1ca")?;
                for text in &texts {
                    if deferred.is_none() {
                        deferred = self.token_name_locals(&text.text, text.unknown)?;
                    }
                    outcomes.extend(self.text_outcomes(text, &kept, neighbor)?);
                }
            }
            found = outcomes
                .into_iter()
                .flat_map(|outcome| match outcome {
                    None => found.clone(),
                    Some(set) => BTreeSet::from([set]),
                })
                .collect();
        }
        if mode != 0 {
            return deferred.map_or(Ok(None), Err);
        }
        // Mode 0 shows the stack object's own type when nothing was found.
        let shown: BTreeSet<_> = found.iter().map(|f| f.or(object)).collect();
        if shown.len() > 1 {
            return Err(Trap::Unsupported(
                "a dialog's text has a name in place of `$Me` (0x44c13e), `$Object` (0x44c1ca), \
                 `$Family` (0x44c0cf) or `$NeighborLocal` (0x4e9c46, from the interface's \
                 table, 0x4e893e), or a token of a pass before `$Neighbor`'s that is not ported \
                 (0x44bf66 to 0x44c4a2), and whether the text then holds `$Neighbor` or \
                 `$Object` decides the portrait (0x44c65f): the names are not ported"
                    .into(),
            ));
        }
        if let Some(trap) = deferred {
            return Err(trap);
        }
        Ok(found.into_iter().next().flatten())
    }

    /// The texts a pass that replaces `token` with a name (`$Me`'s,
    /// `$Object`'s, at `at`) may leave from each of `texts`: one for each
    /// of the `names` it may write (none when they are not known). Each
    /// token is replaced from the start, then from one past each place
    /// replaced ([`replace_each`]). A name that holds the token past its
    /// first character never ends ([`never_ends`]), and fails loudly
    /// where the text holds the token, or a name not known that may hold
    /// it; so do names not known there.
    fn name_pass(
        &self,
        texts: Vec<Text>,
        token: &str,
        names: Option<&[String]>,
        at: &str,
    ) -> Result<Vec<Text>, Trap> {
        let mut out = Vec::new();
        for text in texts {
            if !text.text.contains(token) && !text.unknown {
                out.push(text);
                continue;
            }
            let Some(names) = names else {
                return Err(Trap::Unsupported(format!(
                    "a dialog's `{token}` is replaced by a name that is not known (0x4061e3, \
                     {at}), and a name that holds `{token}` past its first character would \
                     never end (0x4558a0, 0x455750): not ported"
                )));
            };
            if let Some(name) = names.iter().find(|name| never_ends(token, name)) {
                return Err(Trap::Unsupported(format!(
                    "a dialog's `{token}` is replaced by the name {name:?}, which holds it past \
                     its first character, so the pass never ends ({at}, 0x4558a0, 0x455750)"
                )));
            }
            out.extend(names.iter().map(|name| Text {
                text: replace_each(&text.text, token, name),
                unknown: text.unknown,
            }));
        }
        Ok(out)
    }

    /// What one string, as the passes up to `$Object`'s left it, may do
    /// to the type the formatter has found: None keeps it, Some(type) puts
    /// that in its place. `kept` is what `$Object`'s pass may have done.
    fn text_outcomes(
        &self,
        text: &Text,
        kept: &BTreeSet<Option<Option<u32>>>,
        neighbor: Option<u32>,
    ) -> Result<BTreeSet<Option<Option<u32>>>, Trap> {
        let Some(neighbor) = neighbor else {
            self.neighbor_locals_named(&text.text)?;
            return Ok(kept.clone());
        };
        let mut outcomes = kept.clone();
        // The passes between `$Object`'s and `$NeighborLocal`'s (0x44c1f2
        // to 0x44c4a2) are not ported. One that writes a name may write
        // `$Neighbor`; one that writes a number may make a `$NeighborLocal`
        // token out of one that already holds `$Neighbor`
        // (`$NeighborLocal:$Local:0`), and that token's pass may then take
        // `$Neighbor` out. Either way what the `$Neighbor` pass finds is
        // not known.
        if LATER_NAMES.iter().any(|token| text.text.contains(token))
            || (text.text.contains("$Neighbor")
                && LATER_NUMBERS.iter().any(|token| text.text.contains(token)))
        {
            outcomes.insert(Some(Some(neighbor)));
            return Ok(outcomes);
        }
        // A name not known may also finish a `$NeighborLocal` token
        // (`$NeighborLocal:$Object` and a name `0`), whose pass may then
        // take `$Neighbor` out, so what is found is not known.
        let (named_text, named) = self.neighbor_locals_named(&text.text)?;
        if named_text.contains("$Neighbor") && !text.unknown {
            return Ok(BTreeSet::from([Some(Some(neighbor))]));
        }
        if named || text.unknown {
            outcomes.insert(Some(Some(neighbor)));
        }
        Ok(outcomes)
    }

    /// The `$TokenNameLocal` pass (0x44c490 to 0x44c590), none of whose
    /// work is ported, or the fault it gives. Each token (0x6e4f80) is
    /// read as `$NeighborLocal`'s are ([`local_token`]); one whose number
    /// is not one of the running tree's locals (from 0, below frame +6,
    /// 0x44c4b5 to 0x44c4d3) is skipped, and the search goes on from one
    /// past it. For one that is, the pass casts the running object to a
    /// person (0x44c4f6) and throws when it is not one (0x44c57b,
    /// 0x634e08); for a person it looks up the inventory of the neighbor
    /// its +0x5ca names, adding an empty one when there is none (0x42a8a1,
    /// 0x42a8e1 to 0x42a91a), and replaces the token with the name of the
    /// token the local picks in it (0x41f6fb, 0x43ac10, 0x4573d9). That
    /// fails loudly. So does a text where a pass before it may have made
    /// such a token: `unknown` (a name not known stands in it),
    /// `$NameAttrib` or `$NameLocal`, or a number pass ([`NUMBERS`]) with
    /// `$TokenNameLocal` (`$TokenNameLocal:$Local`). With no locals no
    /// token is one of them.
    fn token_name_locals(&self, text: &str, unknown: bool) -> Result<Option<Trap>, Trap> {
        const TOKEN: &str = "$TokenNameLocal";
        let count = self.frame()?.locals.len();
        if count == 0 {
            return Ok(None);
        }
        let mut certain = false;
        let mut from = 0;
        while let Some(at) = text
            .get(from..)
            .and_then(|t| t.find(TOKEN))
            .map(|i| i + from)
        {
            let (number, _) = local_token(text.as_bytes(), at + TOKEN.len());
            if usize::try_from(number).is_ok_and(|n| n < count) {
                certain = true;
                break;
            }
            from = at + 1;
        }
        let may = unknown
            || ["$NameAttrib", "$NameLocal"]
                .iter()
                .any(|token| text.contains(token))
            || (text.contains(TOKEN) && NUMBERS.iter().any(|token| text.contains(token)));
        if !certain && !may {
            return Ok(None);
        }
        let has = if certain { "has" } else { "may have" };
        let work = if self.me()?.person.is_none() {
            "the object running the tree is not a person: the pass casts it to one (0x44c4f6) \
             and throws (0x44c57b, 0x634e08)"
        } else {
            "the pass looks up the inventory of the neighbor the person's +0x5ca names, adding \
             an empty one when there is none (0x42a8a1, 0x42a8e1), and names the token the \
             local picks in it (0x41f6fb, 0x43ac10)"
        };
        Ok(Some(Trap::Unsupported(format!(
            "a dialog's text {has} a `$TokenNameLocal` token whose number is one of the tree's \
             locals (0x44c4b5 to 0x44c4d3), and {work}: not ported"
        ))))
    }

    /// The names the formatter may write for `$Object` of type `guid`
    /// (0x44c1ca), or none when they are not known. The name is an entry
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

    /// `text` after the formatter's `$NeighborLocal` pass (0x44c590 to
    /// 0x44c65a), as far as it matters to the `$Neighbor` pass after it,
    /// and whether a type's name took the place of a token. Each token
    /// (0x6e4fc0), searched for from one past the last (0x44c643), is read
    /// with its `:` and number, and an optional second `:` and number
    /// (0x44caf6; the number is the C runtime's 32-bit conversion,
    /// 0x636029, which wraps). When the number is one of the running
    /// tree's locals (from 0, below the count the tree has, frame +6,
    /// compared signed), the local names a neighbor record (0x44c5f4,
    /// 0x4256f4) and the record's type is in the interface's table of
    /// person types (0x4572af, 0x4e9753), the token is replaced by the
    /// type's name (0x4e9c46, 0x455750); a record whose type is not in the
    /// table ends the pass (0x44c616 to 0x44c653). Otherwise the token
    /// stays, and the `$Neighbor` pass finds it. The name is not known: it
    /// stands here as text without a token, and
    /// [`Machine::dialog_text_type`] fails where that could matter.
    pub(crate) fn neighbor_locals_named(&self, text: &str) -> Result<(String, bool), Trap> {
        const TOKEN: &str = "$NeighborLocal";
        let locals = &self.frame()?.locals;
        let mut text = text.to_owned();
        let mut named = false;
        let mut from = 0;
        while let Some(at) = text
            .get(from..)
            .and_then(|t| t.find(TOKEN))
            .map(|i| i + from)
        {
            let (number, end) = local_token(text.as_bytes(), at + TOKEN.len());
            let record = match usize::try_from(number) {
                Ok(n) if n < locals.len() => {
                    self.check_local(n).map_err(|trap| {
                        Trap::Unsupported(format!(
                            "a dialog's `$NeighborLocal:{n}` (0x44c5f4): {trap}"
                        ))
                    })?;
                    self.world.neighbors.get(locals[n])
                }
                _ => None,
            };
            if let Some(record) = record {
                if !self.listed_person_type(record.guid, "a dialog's $NeighborLocal")? {
                    break;
                }
                text.replace_range(at..end, "(name)");
                named = true;
            }
            from = at + 1;
        }
        Ok((text, named))
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
