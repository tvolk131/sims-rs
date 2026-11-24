//! Which frame locals `Dialog` (opcode 36) and `Add/Change Action String` (opcode 50)
//! may read when they format their text.
//!
//! Both run the shared string formatter (`sims-vm/src/primitives/dialog.rs`,
//! `format_text`, the original's `0x44bf1a`) over strings of the running code's own
//! file (`string_table`, `0x47028b`):
//!
//! - `Dialog` formats strings of `STR#` 301 (`dialog_strings`);
//! - `Add/Change Action String` formats string `ops[4]` of table `word 0`
//!   (`action_string.rs`, `action_name`).
//!
//! The formatter reads locals only through tokens:
//!
//! - `$Local:N` (`locals_written`), `$TimeLocal:N` (`time_locals`) and the career tokens
//!   `$JobOffer`, `$JobDesc`, `$Job` (`career_tokens`) read the locals their numbers
//!   name: `:N` and an optional second `:M` (`local_token`, `0x44caf6`);
//! - `$NeighborLocal:N` reads one too, but this module never relies on it, below.
//!
//! So when every string a table holds is known, the locals the formatter may read are
//! the numbers those tokens name, unless a pass may write text that holds a token of a
//! later pass. Then the token it makes can name any local, and the table counts as
//! unknown ([`TableLocals::Unknown`]). That is the case when a string holds:
//!
//! - `$Me`, `$Object`, `$Neighbor` (and `$NeighborLocal`) or `$Family`: these passes
//!   write a name, which is world data (a player names a person), before `$TimeLocal`
//!   and the career passes, and `$Me`/`$Object`/`$Family` before `$Local`'s;
//! - a token of a pass `sims-vm` does not port (`BETWEEN`, `AFTER`: `$Attrib`,
//!   `$NameAttrib`, `$NameLocal`, `$TokenNameLocal`, `$CatalogLocal`,
//!   `$FameTitleLocal`, `$Grade`). They may read locals too;
//! - a `$Local` or `$TimeLocal` token right after a `:` or a digit. The value or hour it
//!   writes (digits, or a fixed `format_hour` text) could extend the number of a token
//!   before it, which then names a local chosen by a value.
//!
//! The other passes write a local's value in decimal or an hour text, neither of which
//! holds a `$`, so no token of a later pass appears.
//!
//! Each `Dialog` or `Add/Change Action String` names the strings it formats by index in
//! its operands (`dialog_strings`, `action_name`: operand value `v` names string `v - 1`,
//! and 0 names none), so the locals it may read are those of the strings it names. The
//! string at an index is taken from every chunk of the ID a loader could read it from:
//!
//! - a `STR#` chunk the loader reads as ported (`unported_strings`: flag 0x10, format -1,
//!   -2 or -3, a count from 0 to 0x7fff; and at least one US English entry, else the
//!   loader's English fallback rereads it, `english_fallback`), deleted or not: the
//!   string at the index in both orders `sims-vm` may use, its US English entries in file
//!   order (`Library::strings`) and in the loader's order (`loaded_order`), from the text
//!   and the raw bytes;
//! - any other `STR#` chunk of the ID, every `CTSS` chunk of the ID and the raw bytes of
//!   every chunk whose tag names `CST` (legacy constant strings, which the original loads
//!   when the file has no `STR#` of the ID): every string they hold, at every index. One
//!   that does not parse makes every index of its table unknown.
//!
//! A string the tables do not hold at an index reads no local: `Dialog` then shows a
//! default label (`sims-vm` formats it only when the labels are known to hold no token,
//! and fails otherwise) and `Add/Change Action String` faults (`0x4704fa` returns null).
//! Neither reads a local.

use sims_formats::iff::Iff;
use sims_formats::strings::{StringTable, TextEntry};
use std::collections::{BTreeMap, BTreeSet};

/// The table `Dialog` reads its strings from (`dialog.rs`, `dialog_strings`).
const DIALOG_TABLE: u16 = 301;

/// Tokens whose pass writes text a later pass may read as a token (see the module
/// docs), or whose pass `sims-vm` does not port. A string holding one may read any
/// local.
const UNBOUNDED: [&str; 11] = [
    "$Me",
    "$Object",
    "$Neighbor",
    "$Family",
    "$Attrib",
    "$NameAttrib",
    "$NameLocal",
    "$TokenNameLocal",
    "$CatalogLocal",
    "$FameTitleLocal",
    "$Grade",
];

/// Tokens that read the locals their numbers name.
const READERS: [&str; 5] = ["$Local", "$TimeLocal", "$JobOffer", "$JobDesc", "$Job"];

/// Tokens whose pass writes text (a value or an hour) into the string before later
/// passes run.
const WRITERS: [&str; 2] = ["$Local", "$TimeLocal"];

/// What one string table's strings may read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableLocals {
    /// At most these locals.
    Known(BTreeSet<i16>),
    /// Any local.
    Unknown,
}

/// What the strings of one table may read: at every index, and at each index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Table {
    /// What a string at any index may read (from chunks not read by index).
    anywhere: Option<TableLocals>,
    /// What the string at each index may read.
    at: BTreeMap<usize, TableLocals>,
}

impl Table {
    /// What the string at `index` may read.
    fn at(&self, index: usize) -> TableLocals {
        let mut all = TableLocals::Known(BTreeSet::new());
        if let Some(anywhere) = &self.anywhere {
            merge(&mut all, anywhere.clone());
        }
        if let Some(found) = self.at.get(&index) {
            merge(&mut all, found.clone());
        }
        all
    }

    fn anywhere(&mut self, found: TableLocals) {
        merge(
            self.anywhere
                .get_or_insert_with(|| TableLocals::Known(BTreeSet::new())),
            found,
        );
    }
}

/// The locals each string of each table of a file may make the formatter read
/// ([`LiveContext::text_locals`](super::LiveContext::text_locals)).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextLocals {
    tables: BTreeMap<u16, Table>,
}

impl TextLocals {
    /// Reads every string table of `iff` (see the module docs).
    #[must_use]
    pub fn from_iff(iff: &Iff) -> Self {
        let mut tables: BTreeMap<u16, Table> = BTreeMap::new();
        for chunk in &iff.chunks {
            let data = iff.data(chunk);
            let flags = iff.resource_flags(chunk);
            let table = tables.entry(chunk.id).or_default();
            match &chunk.tag {
                b"STR#" if read_by_index(flags, data) => {
                    let Ok(parsed) = StringTable::parse(data) else {
                        table.anywhere(TableLocals::Unknown);
                        continue;
                    };
                    let english: Vec<&TextEntry> =
                        parsed.entries.iter().filter(|e| e.language == 1).collect();
                    for order in [english, loaded_order(&parsed.entries)] {
                        for (index, entry) in order.into_iter().enumerate() {
                            let slot = table
                                .at
                                .entry(index)
                                .or_insert_with(|| TableLocals::Known(BTreeSet::new()));
                            merge(slot, entry_locals(entry));
                        }
                    }
                }
                b"STR#" | b"CTSS" => table.anywhere(
                    StringTable::parse(data)
                        .or_else(|_| StringTable::parse_chunk(data, flags))
                        .ok()
                        .map_or(TableLocals::Unknown, |parsed| {
                            let mut all = TableLocals::Known(BTreeSet::new());
                            for entry in &parsed.entries {
                                merge(&mut all, entry_locals(entry));
                            }
                            all
                        }),
                ),
                tag if tag.windows(3).any(|w| w == b"CST") => {
                    table.anywhere(string_locals(&String::from_utf8_lossy(data)));
                }
                _ => {}
            }
        }
        tables.retain(|_, table| table.anywhere.is_some() || !table.at.is_empty());
        Self { tables }
    }

    /// The locals string `index` of table `id` may make the formatter read. A string
    /// the file does not hold reads none.
    fn string(&self, id: u16, index: usize) -> TableLocals {
        self.tables
            .get(&id)
            .map_or_else(|| TableLocals::Known(BTreeSet::new()), |t| t.at(index))
    }

    /// The locals the strings named by operand values `named` (string `v - 1` for each
    /// `v`, none for 0) of table `id` may make the formatter read.
    fn strings(&self, id: u16, named: impl IntoIterator<Item = u8>) -> TableLocals {
        let mut all = TableLocals::Known(BTreeSet::new());
        for value in named {
            if let Some(index) = usize::from(value).checked_sub(1) {
                merge(&mut all, self.string(id, index));
            }
        }
        all
    }

    /// The locals primitive `opcode` with `ops` may read through its text, when it is a
    /// `Dialog` or `Add/Change Action String`; `None` for any other primitive.
    #[must_use]
    pub fn primitive(&self, opcode: u16, ops: &[u8; 8]) -> Option<TableLocals> {
        match opcode {
            36 => Some(self.strings(
                DIALOG_TABLE,
                dialog_operands(ops).iter().map(|&operand| ops[operand]),
            )),
            50 => Some(self.strings(u16::from_le_bytes([ops[0], ops[1]]), [ops[4]])),
            _ => None,
        }
    }
}

/// The operands whose values name the strings a `Dialog` formats (`dialog_strings`):
/// its buttons' by its type (`ops[5] & 0x7f`), its icon's and message's (6 and 2), and in
/// mode 4 (`(ops[7] >> 1) & 7`) operand 1's.
fn dialog_operands(ops: &[u8; 8]) -> Vec<usize> {
    let mut operands: Vec<usize> = match ops[5] & 0x7f {
        0 | 3 | 4 => vec![3],
        1 => vec![3, 4],
        2 => vec![3, 4, 0],
        _ => vec![],
    };
    operands.extend([6, 2]);
    if (ops[7] >> 1) & 7 == 4 {
        operands.push(1);
    }
    operands
}

/// Whether the loader reads a `STR#` chunk with resource flags `flags` and bytes `data`
/// as `sims-vm` ports it (`unported_strings` finds nothing and `english_fallback` does
/// not apply), so its strings are read by index.
fn read_by_index(flags: u16, data: &[u8]) -> bool {
    let word = |at: usize| {
        data.get(at..at + 2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
    };
    flags & 0x10 != 0
        && matches!(word(0), Some(-3..=-1))
        && word(2).is_some_and(|count| count >= 0)
        && StringTable::parse(data)
            .is_ok_and(|t| t.entries.is_empty() || t.entries.iter().any(|e| e.language == 1))
}

/// The US English entries of a table in the order the original's loader puts them
/// (`sims-vm`'s `loaded_order`, `0x46feac`): it counts each run of one language from 0
/// and inserts each US English entry at its place in its run, clamped to the entries so
/// far, so a later run goes in front of earlier ones.
fn loaded_order(entries: &[TextEntry]) -> Vec<&TextEntry> {
    let mut loaded: Vec<&TextEntry> = Vec::new();
    let mut run = None;
    let mut place = 0;
    for entry in entries {
        if run != Some(entry.language) {
            run = Some(entry.language);
            place = 0;
        }
        if entry.language == 1 {
            loaded.insert(place.min(loaded.len()), entry);
            place += 1;
        }
    }
    loaded
}

/// What one entry may read, through its decoded text or its raw bytes.
fn entry_locals(entry: &TextEntry) -> TableLocals {
    let mut all = string_locals(&entry.text);
    merge(
        &mut all,
        string_locals(&String::from_utf8_lossy(&entry.raw_text)),
    );
    all
}

fn merge(into: &mut TableLocals, found: TableLocals) {
    match (into, found) {
        (TableLocals::Known(set), TableLocals::Known(more)) => set.extend(more),
        (into, TableLocals::Unknown) => *into = TableLocals::Unknown,
        (TableLocals::Unknown, TableLocals::Known(_)) => {}
    }
}

/// The locals one string may make the formatter read (see the module docs).
#[must_use]
pub fn string_locals(text: &str) -> TableLocals {
    if UNBOUNDED.iter().any(|token| text.contains(token)) {
        return TableLocals::Unknown;
    }
    let bytes = text.as_bytes();
    let mut read = BTreeSet::new();
    for token in READERS {
        let mut from = 0;
        while let Some(at) = text
            .get(from..)
            .and_then(|t| t.find(token))
            .map(|i| i + from)
        {
            if WRITERS.contains(&token)
                && at > 0
                && (bytes[at - 1] == b':' || bytes[at - 1].is_ascii_digit())
            {
                return TableLocals::Unknown;
            }
            for number in token_numbers(bytes, at + token.len()) {
                if let Ok(n) = i16::try_from(number)
                    && n >= 0
                {
                    read.insert(n);
                }
            }
            from = at + 1;
        }
    }
    TableLocals::Known(read)
}

/// The numbers a token ending at `end` names, read as `local_token` (`0x44caf6`) reads
/// them: a `:` and digits, and an optional second `:` and digits, each number its
/// first 15 digits converted with 32-bit wrapping.
fn token_numbers(bytes: &[u8], mut end: usize) -> Vec<i32> {
    let mut numbers = Vec::new();
    for _ in 0..2 {
        if bytes.get(end) != Some(&b':') {
            break;
        }
        end += 1;
        let digits = bytes[end..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits > 0 {
            numbers.push(bytes[end..end + digits].iter().take(15).fold(0i32, |n, d| {
                n.wrapping_mul(10).wrapping_add(i32::from(d - b'0'))
            }));
        }
        end += digits;
    }
    numbers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(slots: &[i16]) -> TableLocals {
        TableLocals::Known(slots.iter().copied().collect())
    }

    #[test]
    fn tokens_name_their_locals() {
        assert_eq!(string_locals("plain"), known(&[]));
        assert_eq!(string_locals("Costs $$Local:2."), known(&[2]));
        assert_eq!(
            string_locals("$Local:1:7 at $TimeLocal:3"),
            known(&[1, 3, 7])
        );
        assert_eq!(string_locals("$Job:4:5"), known(&[4, 5]));
        assert_eq!(string_locals("$JobOffer:0"), known(&[0]));
        assert_eq!(string_locals("$Local: and $Local"), known(&[]));
        assert_eq!(string_locals("$Local:99999"), known(&[]));
    }

    fn entry(language: u8, text: &str) -> TextEntry {
        TextEntry {
            language,
            text: text.to_owned(),
            comment: String::new(),
            raw_text: text.as_bytes().to_vec(),
            raw_comment: Vec::new(),
        }
    }

    /// A later run of US English goes in front of the earlier one, so a string's index
    /// may differ between the two orders: both count.
    #[test]
    fn strings_are_indexed_in_both_orders() {
        let entries = [
            entry(1, "$Local:1"),
            entry(2, "other"),
            entry(1, "$Local:2"),
        ];
        let loaded: Vec<&str> = loaded_order(&entries)
            .iter()
            .map(|e| e.text.as_str())
            .collect();
        assert_eq!(loaded, ["$Local:2", "$Local:1"]);

        let mut data = Vec::new();
        data.extend_from_slice(&(-3i16).to_le_bytes());
        data.extend_from_slice(&3u16.to_le_bytes());
        for e in &entries {
            data.push(e.language);
            data.extend_from_slice(e.text.as_bytes());
            data.extend_from_slice(b"\0\0");
        }
        let iff = Iff::parse(
            sims_formats::iff::write(&[sims_formats::iff::ChunkData {
                tag: *b"STR#",
                id: 7,
                flags: 0x10,
                label: String::new(),
                data,
            }])
            .unwrap(),
        )
        .unwrap();
        let text = TextLocals::from_iff(&iff);
        assert_eq!(text.string(7, 0), known(&[1, 2]));
        assert_eq!(text.string(7, 1), known(&[1, 2]));
        assert_eq!(text.string(7, 2), known(&[]));
        assert_eq!(text.string(8, 0), known(&[]));
        // Action String names string `ops[4] - 1`, and 0 names none.
        assert_eq!(
            text.primitive(50, &[7, 0, 0, 0, 0, 0, 0, 0]),
            Some(known(&[]))
        );
        assert_eq!(
            text.primitive(50, &[7, 0, 0, 0, 3, 0, 0, 0]),
            Some(known(&[]))
        );
    }

    #[test]
    fn passes_that_write_tokens_read_any_local() {
        for text in [
            "$Me is here",
            "$Object",
            "$NeighborLocal:1",
            "$Family",
            "$NameLocal:2",
            "$Grade",
            "$Job:$Local:1",
            "$Job:3$Local:1",
            "$TimeLocal:1$TimeLocal:2",
            "$Local:1$Local:2",
        ] {
            assert_eq!(string_locals(text), TableLocals::Unknown, "{text}");
        }
    }
}
