//! What the original's Buy catalog lists, and the text it lists it under.
//! Addresses are `Sims.exe` 1.91.
//!
//! - [`BuyDefinition::eligibility`] and [`has_interactions`]: which
//!   definitions the catalog builder (0x4cd149) keeps.
//! - [`CatalogText::read`]: a definition's catalog name and description
//!   (0x457d3b, which fills them by 0x4061e3).
//! - [`BuyLabels`]: the category buttons' text (`UIText.iff` STR# 150,
//!   200–207 and 210, loaded at 0x5057a2–0x5058cb and shown by 0x506765).
//! - [`Catalog::buy_entries`]: all three together over an install.
//!
//! Languages are the game's codes, 1 for US English, as the string
//! resources store them.

use crate::{Catalog, Error, Index, Install, Result};
use sims_formats::behavior::InteractionTable;
use sims_formats::iff::Iff;
use std::collections::BTreeMap;

/// The definition words the Buy catalog reads, from an `OBJD` payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyDefinition {
    /// Word +8: how many graphics the definition draws.
    pub graphics: u16,
    /// Word +0xe: the interaction table (`TTAB`) ID; -1 means the master's.
    pub interactions: i16,
    /// Word +0x12: the definition's kind; 2 is a person.
    pub kind: u16,
    /// Word +0x14: the multi-tile group, 0 for a single-tile object.
    pub master: u16,
    /// Word +0x16: the tile's place in its group; -1 is the group's master.
    pub subindex: i16,
    /// Word +0x52: the ID of the `CATS` or `CTSS` resource holding its
    /// catalog text.
    pub strings: u16,
}

/// What the catalog builder makes of a definition (0x4cd83b–0x4cd86b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuyEligibility {
    /// Never listed: a person (0x4575d2) or a tile of a multi-tile object
    /// other than its master (0x4575b8).
    Excluded,
    /// Listed: a multi-tile master, or a definition with graphics
    /// (0x457588).
    Listed,
    /// Listed only when its interaction table has an entry (0x4575a1, see
    /// [`has_interactions`]).
    NeedsInteractions,
}

impl BuyDefinition {
    /// Reads the words from an `OBJD` payload, which must reach word
    /// +0x52.
    pub fn parse(objd: &[u8]) -> sims_formats::Result<Self> {
        let word = |offset: usize| {
            objd.get(offset..offset + 2)
                .map(|w| u16::from_le_bytes([w[0], w[1]]))
                .ok_or_else(|| sims_formats::Error::new(objd.len(), "OBJD ends before word 0x52"))
        };
        Ok(Self {
            graphics: word(8)?,
            interactions: word(0xe)? as i16,
            kind: word(0x12)?,
            master: word(0x14)?,
            subindex: word(0x16)? as i16,
            strings: word(0x52)?,
        })
    }

    /// Whether the definition is a multi-tile group's master (0x43adf7).
    pub fn is_master(&self) -> bool {
        self.master != 0 && self.subindex == -1
    }

    /// The catalog builder's tests, in its order (0x4cd840–0x4cd864).
    pub fn eligibility(&self) -> BuyEligibility {
        if (self.master != 0 && !self.is_master()) || self.kind == 2 {
            BuyEligibility::Excluded
        } else if self.master != 0 || self.graphics != 0 {
            BuyEligibility::Listed
        } else {
            BuyEligibility::NeedsInteractions
        }
    }

    /// The interaction table ID the definition loads (0x45726a), or None
    /// when it loads none: an ID of 0 or below is never looked up
    /// (0x4569c0).
    ///
    /// An ID of -1 takes the master's word as it is (0x45727c), so
    /// `master` must be the definition's master (see [`master_of`]); a
    /// single-tile definition is its own master (0x43ae20), and its -1
    /// loads nothing. Any other negative ID is 0 (0x457289). With no
    /// master the original reads through a null pointer; this gives None.
    pub fn interaction_table(&self, master: Option<&BuyDefinition>) -> Option<u16> {
        let id = match self.interactions {
            -1 => master?.interactions,
            id if id < 0 => 0,
            id => id,
        };
        u16::try_from(id).ok().filter(|&id| id > 0)
    }
}

/// The master a definition's -1 interaction table refers to (0x43ae0a):
/// a single-tile definition is its own; otherwise the first registered
/// definition from the same file with the same group that is a master.
/// `definitions` are the candidates in registration order, each with the
/// file it came from.
pub fn master_of<'a, F: PartialEq + 'a>(
    definition: &'a BuyDefinition,
    file: &F,
    definitions: impl IntoIterator<Item = (&'a F, &'a BuyDefinition)>,
) -> Option<&'a BuyDefinition> {
    if definition.master == 0 {
        return Some(definition);
    }
    definitions
        .into_iter()
        .find(|(f, d)| *f == file && d.master == definition.master && d.is_master())
        .map(|(_, d)| d)
}

/// Whether a definition's interaction table has an entry (0x4575a1).
///
/// 0x457cb0 loads the table from the definition's own file, and only when
/// that file has no `TTAB` of that ID from its semi-global file. A table
/// that loads with no entries therefore hides the semi-global's. It never
/// reads `Global.iff`, and it does not ask whether an interaction's test
/// would pass. `own` and `semi_global` are the `TTAB` payloads of the
/// table's ID in each file, None when the file has none (or there is no
/// semi-global file).
pub fn has_interactions(
    own: Option<&[u8]>,
    semi_global: Option<&[u8]>,
) -> sims_formats::Result<bool> {
    match own.or(semi_global) {
        Some(table) => Ok(!InteractionTable::parse(table)?.interactions.is_empty()),
        None => Ok(false),
    }
}

/// A string resource's strings filed by language, as the original's
/// loaders file them: 0x46feac for `STR#` and 0x470ca7 for `CTSS`.
///
/// Languages here are a code less one, as the loaders index them.
#[derive(Debug, Default)]
struct Filed {
    by_language: BTreeMap<u8, Vec<Vec<u8>>>,
}

impl Filed {
    /// Inserts at the 1-based `place`, clamped to the list (0x471343,
    /// 0x470482), moving later strings down.
    fn insert(&mut self, language: u8, place: usize, text: Vec<u8>) {
        let list = self.by_language.entry(language).or_default();
        let at = place.clamp(1, list.len() + 1) - 1;
        list.insert(at, text);
    }

    /// The 1-based `place` in `language` (0x471411, 0x4704fa), as the
    /// C string the callers copy (0x5d4cf9).
    fn get(&self, language: u8, place: usize) -> Option<&[u8]> {
        let text = self
            .by_language
            .get(&language)?
            .get(place.checked_sub(1)?)?;
        Some(c_string(text))
    }

    /// Files a resource's strings for game language `language` (a code).
    ///
    /// - A leading word (in the byte order `flags` give, see
    ///   [`sims_formats::strings::StringTable::parse_chunk`]) of -1 or -2:
    ///   every string, in order, in the current language and in US
    ///   English.
    /// - -3: each string under its own language, a stored 0 meaning the
    ///   current one. Its place counts from 1 again at each change of
    ///   language, so a language's later run goes in front of its earlier
    ///   one. Unless `all_languages`, only the current language's strings
    ///   are kept. If the current language then has none and
    ///   `english_fallback`, the US English strings are filed under it in
    ///   order (0x470ea6, 0x47006b).
    /// - Any other positive word: that many length-prefixed strings, in
    ///   which bytes 0xa1 become `o` and 0xd5 become `'` (0x470f45), filed
    ///   as for -1. A word of 0 or below is an empty table.
    ///
    /// The original reads past the end of a resource that ends early; that
    /// is an error here.
    fn load(
        data: &[u8],
        flags: u16,
        language: u8,
        all_languages: bool,
        english_fallback: bool,
    ) -> sims_formats::Result<Self> {
        let current = language.wrapping_sub(1);
        let mut filed = Self::default();
        let short = |at: usize| sims_formats::Error::new(at, "string resource ends early");
        let Some(&[a, b]) = data.first_chunk::<2>() else {
            return Err(short(0));
        };
        let swapped = flags & 0x10 == 0;
        let word = if swapped {
            i16::from_be_bytes([a, b])
        } else {
            i16::from_le_bytes([a, b])
        };
        let count_at = |data: &[u8], swap: bool| -> sims_formats::Result<i16> {
            let &[a, b] = data
                .get(2..4)
                .and_then(|c| c.first_chunk::<2>())
                .ok_or(short(2))?;
            Ok(if swap {
                i16::from_be_bytes([a, b])
            } else {
                i16::from_le_bytes([a, b])
            })
        };
        let c_string_at = |at: usize| -> sims_formats::Result<(&[u8], usize)> {
            let rest = data.get(at..).ok_or(short(at))?;
            let end = rest.iter().position(|&b| b == 0).ok_or(short(data.len()))?;
            Ok((&rest[..end], at + end + 1))
        };
        let both = |filed: &mut Self, place: usize, text: Vec<u8>| {
            filed.insert(current, place, text.clone());
            if current != 0 {
                filed.insert(0, place, text);
            }
        };
        match word {
            -1 | -2 => {
                let count = count_at(data, word == -1 && swapped)?;
                let mut at = 4;
                for place in 1..=usize::try_from(count).unwrap_or(0) {
                    let (text, next) = c_string_at(at)?;
                    both(&mut filed, place, text.to_vec());
                    at = next;
                    if word == -2 {
                        at = c_string_at(at)?.1;
                    }
                }
            }
            -3 => {
                let count = usize::try_from(count_at(data, false)?).unwrap_or(0);
                // Each entry: its stored language, its string's start, and
                // where the next entry starts.
                let mut entries = Vec::with_capacity(count.min(data.len()));
                let mut at = 4;
                for _ in 0..count {
                    let stored = *data.get(at).ok_or(short(at))?;
                    let (_, comment) = c_string_at(at + 1)?;
                    let (_, next) = c_string_at(comment)?;
                    entries.push((stored, at + 1));
                    at = next;
                }
                let mut run = None;
                let mut place = 0;
                for &(stored, text) in &entries {
                    let language = match stored.wrapping_sub(1) {
                        0xff => current,
                        language => language,
                    };
                    if run != Some(language) {
                        run = Some(language);
                        place = 0;
                    }
                    if language == current || all_languages {
                        place += 1;
                        filed.insert(language, place, c_string_at(text)?.0.to_vec());
                    }
                }
                if english_fallback && filed.by_language.get(&current).is_none_or(Vec::is_empty) {
                    let mut place = 1;
                    for &(stored, text) in &entries {
                        if stored == 1 {
                            filed.insert(current, place, c_string_at(text)?.0.to_vec());
                            place += 1;
                        }
                    }
                }
            }
            count if count > 0 => {
                let mut at = 2;
                for place in 1..=count as usize {
                    let length = usize::from(*data.get(at).ok_or(short(at))?);
                    let text = data.get(at + 1..at + 1 + length).ok_or(short(at))?;
                    let text = text
                        .iter()
                        .map(|&b| match b {
                            0xa1 => b'o',
                            0xd5 => b'\'',
                            b => b,
                        })
                        .collect();
                    both(&mut filed, place, text);
                    at += 1 + length;
                }
            }
            _ => {}
        }
        Ok(filed)
    }
}

/// The bytes before the first NUL.
fn c_string(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

/// A definition's catalog name and description, as bytes in the game's
/// code page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogText {
    pub name: Vec<u8>,
    pub description: Vec<u8>,
}

impl CatalogText {
    /// The text 0x457d3b gives a definition, from its `CATS` and `CTSS`
    /// resources of ID [`BuyDefinition::strings`] (0x4061e3):
    ///
    /// - With a `CATS`, its first C string is the description and, when
    ///   another starts before the resource ends, the second is the name
    ///   (0x40631f–0x406352). The `CTSS` is not read.
    /// - Otherwise the `CTSS` (payload and resource flags) is filed for
    ///   the game's language only, with no fallback to US English
    ///   (0x470ca7 with 0, 0), and its first and second strings are the
    ///   name and description (0x471411 with 1 and 2). A missing `CTSS`
    ///   leaves both empty.
    /// - Then, for a person (`person`) whose name is still empty, the
    ///   `CTSS` is filed in every language, and the name is the first
    ///   string of the first language code from 1 to 20 that has one,
    ///   even an empty one (0x4062c1–0x4062f8).
    /// - A name still empty is `*` and `label`, the definition's `OBJD`
    ///   label up to its NUL (0x457daf–0x457de2, `"*"` at 0x6c12fc).
    ///
    /// The original reads past the end of an unterminated `CATS` string;
    /// that is an error here.
    pub fn read(
        cats: Option<&[u8]>,
        ctss: Option<(&[u8], u16)>,
        label: &[u8],
        language: u8,
        person: bool,
    ) -> sims_formats::Result<Self> {
        let mut text = Self {
            name: Vec::new(),
            description: Vec::new(),
        };
        if let Some(cats) = cats {
            let end = |from: usize| {
                cats[from..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|n| from + n)
                    .ok_or_else(|| sims_formats::Error::new(cats.len(), "unterminated CATS string"))
            };
            let first = end(0)?;
            text.description = cats[..first].to_vec();
            if first + 1 < cats.len() {
                text.name = cats[first + 1..end(first + 1)?].to_vec();
            }
        } else {
            if let Some((data, flags)) = ctss {
                let filed = Filed::load(data, flags, language, false, false)?;
                let current = language.wrapping_sub(1);
                text.name = filed.get(current, 1).unwrap_or_default().to_vec();
                text.description = filed.get(current, 2).unwrap_or_default().to_vec();
            }
            if person && text.name.is_empty() {
                text.name = match ctss {
                    Some((data, flags)) => {
                        let filed = Filed::load(data, flags, language, true, false)?;
                        (0..20)
                            .find_map(|language| filed.get(language, 1))
                            .unwrap_or_default()
                            .to_vec()
                    }
                    None => Vec::new(),
                };
            }
        }
        if text.name.is_empty() {
            text.name.push(b'*');
            text.name.extend_from_slice(c_string(label));
        }
        Ok(text)
    }
}

/// The Buy catalog's category button text, from `UIText.iff`.
///
/// Each table is filed for the game's language, falling back to US English
/// only when the table has no string in that language (0x47028b →
/// 0x46feac with 0, 1); a string the table lacks is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyLabels {
    /// STR# 150's 56 strings, eight to a catalog (0x5057b5–0x5057da).
    roots: [[Vec<u8>; 8]; 7],
    /// STR# 200 to 207's first four strings (0x5057e7–0x505844).
    functions: [[Vec<u8>; 4]; 8],
    /// STR# 210's strings 2 to 5 (0x505858–0x5058cb).
    special: [Vec<u8>; 4],
}

impl BuyLabels {
    /// Reads the tables from the install's `GameData/UIText.iff` (file 9
    /// of 0x415336).
    pub fn load(install: &Install, language: u8) -> Result<Self> {
        let path = install.file("GameData/UIText.iff")?;
        let location = path.display().to_string();
        let bytes = std::fs::read(&path).map_err(|e| Error::Io {
            path: path.clone(),
            source: e,
        })?;
        let iff = Iff::parse(bytes).map_err(|e| Error::format(&location, e))?;
        Self::from_iff(&iff, language).map_err(|e| Error::format(&location, e))
    }

    /// Reads the tables from a parsed `UIText.iff`. A table that is
    /// missing fails: the original then keeps whatever text it had.
    pub fn from_iff(iff: &Iff, language: u8) -> sims_formats::Result<Self> {
        let current = language.wrapping_sub(1);
        let table = |id: u16| -> sims_formats::Result<Filed> {
            let chunk = iff.get(b"STR#", id).ok_or_else(|| {
                sims_formats::Error::new(0, format!("UIText.iff has no STR# {id}"))
            })?;
            Filed::load(
                iff.data(chunk),
                iff.resource_flags(chunk),
                language,
                false,
                true,
            )
        };
        let text =
            |filed: &Filed, place: usize| filed.get(current, place).unwrap_or_default().to_vec();
        let roots = table(150)?;
        let mut functions: [[Vec<u8>; 4]; 8] = Default::default();
        for (row, values) in functions.iter_mut().enumerate() {
            let filed = table(200 + row as u16)?;
            *values = std::array::from_fn(|column| text(&filed, column + 1));
        }
        let special = table(210)?;
        Ok(Self {
            roots: std::array::from_fn(|catalog| {
                std::array::from_fn(|column| text(&roots, catalog * 8 + column + 1))
            }),
            functions,
            special: [2, 3, 4, 5].map(|place| text(&special, place)),
        })
    }

    /// STR# 150's string for catalog row `catalog` (0–6) and `column`
    /// (0–7). Empty outside those ranges.
    pub fn root(&self, catalog: usize, column: usize) -> &[u8] {
        self.roots
            .get(catalog)
            .and_then(|row| row.get(column))
            .map_or(&[], Vec::as_slice)
    }

    /// The text of category button `column` (0–7) in catalog `catalog`
    /// (0–6, the mode 0x506765 switches on), with `subcatalog` the view's
    /// subcatalog (`+0x46c`) and `row` the selected function row (0–7).
    ///
    /// - Subcatalog 0: catalog 0 to 4 use their own row of STR# 150 and
    ///   catalogs 5 and 6 use row 5, except that catalog 6's buttons 2 and
    ///   3 read row 5 column 4 and row 4 column 2.
    /// - Otherwise, catalog 1 reads its function row's STR# 200 + `row`
    ///   for buttons 0 to 3, and STR# 210's strings 4, 5, 2 and 3 for
    ///   buttons 4 to 7; every other catalog reads STR# 150 row 1.
    ///
    /// Whether a button is shown is [`BuyLabels::shown`]. A catalog, row
    /// or column outside those ranges has no button, and an empty label.
    pub fn label(&self, catalog: usize, subcatalog: i32, row: usize, column: usize) -> &[u8] {
        if catalog > 6 || column > 7 {
            return &[];
        }
        if subcatalog != 0 {
            return match (catalog, column) {
                (1, 0..=3) => self
                    .functions
                    .get(row)
                    .map_or(&[], |functions| functions[column].as_slice()),
                (1, _) => &self.special[[2, 3, 0, 1][column - 4]],
                _ => self.root(1, column),
            };
        }
        match (catalog, column) {
            (6, 2) => self.root(5, 4),
            (6, 3) => self.root(4, 2),
            (5 | 6, _) => self.root(5, column),
            _ => self.root(catalog, column),
        }
    }

    /// Whether 0x506765 leaves button `column` showing on its own account:
    /// catalogs 2 to 6 hide buttons 4 to 6 in subcatalog 0
    /// (0x506944–0x506959 and the like, to 0x506ca1). Catalog 1's buttons
    /// 4 to 7 in a subcatalog have further conditions that are not ported.
    /// A catalog or column outside 0–6 and 0–7 has no button.
    pub fn shown(catalog: usize, subcatalog: i32, column: usize) -> bool {
        catalog <= 6
            && column <= 7
            && !(subcatalog == 0 && (2..=6).contains(&catalog) && (4..=6).contains(&column))
    }
}

/// One definition the Buy catalog lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuyEntry {
    /// Index into [`Catalog::objects`].
    pub object: usize,
    pub definition: BuyDefinition,
    pub text: CatalogText,
}

/// The definitions the Buy catalog lists, and what could not be read.
#[derive(Debug, Clone, Default)]
pub struct BuyCatalog {
    /// In registration order. The catalog's own sort is not ported here.
    pub entries: Vec<BuyEntry>,
    /// Definitions left out because their files could not be read.
    pub diagnostics: Vec<String>,
}

impl Catalog {
    /// The definitions the Buy catalog lists (0x4cd81a–0x4cd8ab) and their
    /// text, for game language `language`.
    ///
    /// It walks [`Catalog::objects`] in registration order. Whether the
    /// original's registry keeps a definition whose GUID a later file
    /// replaces is not traced; each is kept here. Catalog text and
    /// interaction tables are read from the definition's own file (and its
    /// semi-global, for a table), as `Object::resource` and
    /// [`Catalog::semi_globals`] give them.
    pub fn buy_entries(&self, index: &Index, language: u8) -> Result<BuyCatalog> {
        let mut catalog = BuyCatalog::default();
        let read = |number: usize| -> std::result::Result<Iff, String> {
            let resource = &index.resources[number];
            let location = resource.location(&index.root);
            let bytes = resource.read().map_err(|e| e.to_string())?;
            Iff::parse(bytes).map_err(|e| format!("{location}: {e}"))
        };
        // Objects are registered file by file, so one object file is open
        // at a time; semi-global files stay open once read.
        let mut current: Option<(usize, std::result::Result<Iff, String>)> = None;
        let mut semi_globals: BTreeMap<usize, Option<Iff>> = BTreeMap::new();
        let mut definitions: Vec<Option<BuyDefinition>> = Vec::with_capacity(self.objects.len());
        for (number, object) in self.objects.iter().enumerate() {
            let location = || index.resources[object.resource].location(&index.root);
            if current.as_ref().map(|(n, _)| *n) != Some(object.resource) {
                let file = read(object.resource);
                if let Err(e) = &file {
                    catalog.diagnostics.push(e.clone());
                }
                current = Some((object.resource, file));
            }
            let Some((_, Ok(iff))) = &current else {
                definitions.push(None);
                continue;
            };
            let Some(chunk) = iff.get(b"OBJD", object.chunk) else {
                catalog.diagnostics.push(format!(
                    "{}: OBJD #{} is not a resource",
                    location(),
                    object.chunk
                ));
                definitions.push(None);
                continue;
            };
            let definition = match BuyDefinition::parse(iff.data(chunk)) {
                Ok(definition) => definition,
                Err(e) => {
                    catalog
                        .diagnostics
                        .push(format!("{} OBJD #{}: {e}", location(), object.chunk));
                    definitions.push(None);
                    continue;
                }
            };
            let listed = match definition.eligibility() {
                BuyEligibility::Excluded => false,
                BuyEligibility::Listed => true,
                BuyEligibility::NeedsInteractions => {
                    let earlier = self.objects[..number]
                        .iter()
                        .zip(&definitions)
                        .filter_map(|(o, d)| Some((&o.resource, d.as_ref()?)));
                    let master = master_of(&definition, &object.resource, earlier)
                        .copied()
                        .or_else(|| self.later_master(iff, number, &definition));
                    match definition.interaction_table(master.as_ref()) {
                        None => false,
                        Some(id) => {
                            let own = iff.get(b"TTAB", id).map(|c| iff.data(c));
                            let semi = match self.semi_globals.get(&object.resource) {
                                Some(&Some(semi)) if own.is_none() => semi_globals
                                    .entry(semi)
                                    .or_insert_with(|| match read(semi) {
                                        Ok(iff) => Some(iff),
                                        Err(e) => {
                                            catalog.diagnostics.push(e);
                                            None
                                        }
                                    })
                                    .as_ref()
                                    .and_then(|semi| semi.get(b"TTAB", id).map(|c| semi.data(c))),
                                _ => None,
                            };
                            has_interactions(own, semi).unwrap_or_else(|e| {
                                catalog
                                    .diagnostics
                                    .push(format!("{} TTAB #{id}: {e}", location()));
                                false
                            })
                        }
                    }
                }
            };
            definitions.push(Some(definition));
            if !listed {
                continue;
            }
            let cats = iff.get(b"CATS", definition.strings).map(|c| iff.data(c));
            let ctss = iff
                .get(b"CTSS", definition.strings)
                .map(|c| (iff.data(c), iff.resource_flags(c)));
            match CatalogText::read(
                cats,
                ctss,
                iff.label_bytes(chunk),
                language,
                definition.kind == 2,
            ) {
                Ok(text) => catalog.entries.push(BuyEntry {
                    object: number,
                    definition,
                    text,
                }),
                Err(e) => catalog.diagnostics.push(format!(
                    "{} catalog text #{}: {e}",
                    location(),
                    definition.strings
                )),
            }
        }
        Ok(catalog)
    }

    /// A master for `definition` registered after it, from the same file.
    fn later_master(
        &self,
        iff: &Iff,
        number: usize,
        definition: &BuyDefinition,
    ) -> Option<BuyDefinition> {
        let resource = self.objects[number].resource;
        self.objects[number + 1..]
            .iter()
            .filter(|o| o.resource == resource)
            .filter_map(|o| BuyDefinition::parse(iff.data(iff.get(b"OBJD", o.chunk)?)).ok())
            .find(|d| d.master == definition.master && d.is_master())
    }
}
