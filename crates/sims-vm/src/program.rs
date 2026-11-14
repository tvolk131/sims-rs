use crate::{Error, Result};
use sims_content::{Catalog, Index, Object};
use sims_formats::behavior::{Behavior, InteractionTable};
use sims_formats::execution::{self, ObjectFunction, ObjectFunctions, Slots};
use sims_formats::iff::Iff;
use sims_sim::TypeAttributes;
use std::collections::{BTreeMap, BTreeSet};

/// The code and constants one IFF file provides.
#[derive(Debug, Clone, Default)]
pub struct Library {
    /// BHAV trees by chunk ID.
    pub behaviors: BTreeMap<u16, Behavior>,
    /// BCON constant tables by chunk ID.
    pub constants: BTreeMap<u16, Vec<i16>>,
    /// Each OBJD's callback table, by OBJD chunk ID.
    pub callbacks: BTreeMap<u16, Vec<ObjectFunction>>,
    /// Each OBJD as the original holds it once registered, as 16-bit words,
    /// by OBJD chunk ID. See [`definition_words`].
    pub definitions: BTreeMap<u16, [i16; DEFINITION_WORDS]>,
    /// OBJD chunk IDs in the file's native resource order.
    pub definition_order: Vec<u16>,
    /// Interaction tables (TTAB) by chunk ID. None is a table this reader
    /// cannot decode; using one is an error, having one is not.
    pub interactions: BTreeMap<u16, Option<InteractionTable>>,
    /// How many slots of each SLOT chunk can hold an object (kind 0), by
    /// chunk ID.
    pub containers: BTreeMap<u16, usize>,
}

/// The words in a registered object definition: the original keeps every
/// OBJD as a 216-byte record.
pub const DEFINITION_WORDS: usize = 108;

/// What a tree ID names, seen from an object's file.
#[derive(Debug, Clone, Copy)]
pub enum Tree<'a> {
    /// IDs below 256 are primitives, built into the interpreter.
    Primitive(u16),
    /// A BHAV, with the resource that holds it.
    Behavior(usize, &'a Behavior),
    /// No tree with this ID where the ID points.
    Missing,
}

/// The callbacks an object definition names, in table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Callback {
    Init = 0,
    Main = 1,
    /// Function 7: asked of an object when the lot's rooms have been
    /// worked out again (0x442659).
    RoomsChanged = 7,
}

/// When an OBJD has no OBJf table, the original reads its callbacks from
/// these OBJD byte offsets, in callback order (init, main, load, ...).
const DEFINITION_CALLBACKS: [usize; 30] = [
    0x56, 0x0a, 0x5e, 0x68, 0x78, 0x2a, 0x7c, 0x64, 0x9c, 0x58, 0x94, 0x60, 0x5a, 0x6a, 0x6e, 0x22,
    0x0c, 0x18, 0x30, 0x32, 0x34, 0x36, 0x38, 0x3a, 0x3c, 0x3e, 0x40, 0x42, 0x76, 0xa0,
];

/// The code a set of objects can run.
#[derive(Debug, Clone)]
pub struct Program {
    /// The resource holding `Global.iff`.
    pub global: usize,
    /// Libraries by resource number in the index.
    pub libraries: BTreeMap<usize, Library>,
    /// Each object file's semi-global file, from the catalog.
    pub semi_globals: BTreeMap<usize, Option<usize>>,
    /// Career IDs in the original's order: the CARR chunks of `work.iff`
    /// in resource order, each chunk ID sign-extended (legacy
    /// legacy/crates/sims-content/src/careers.rs). None when the install has no
    /// `work.iff`, or has a `Careers.txt`, which overrides the table
    /// through a loader that is not ported.
    pub careers: Option<Vec<i32>>,
    /// Each lot's zoning, by lot number: 1 residential, 2 community. It
    /// comes from the neighborhood ([`sims_content::Neighborhood::zoning`]),
    /// so [`Program::link`] leaves it empty for the caller to fill.
    pub zoning: BTreeMap<i16, i32>,
}

impl Program {
    /// Loads Global.iff and, for each GUID, its object's file and that
    /// file's semi-global file.
    pub fn link(
        index: &Index,
        catalog: &Catalog,
        guids: impl IntoIterator<Item = u32>,
    ) -> Result<Self> {
        let mut program = Self {
            global: catalog.global,
            libraries: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            careers: careers(index)?,
            zoning: BTreeMap::new(),
        };
        program.load(index, catalog.global)?;
        for guid in guids {
            let object = catalog.object(guid).ok_or(Error::MissingDefinition(guid))?;
            program.load(index, object.resource)?;
            let semi_global = catalog
                .semi_globals
                .get(&object.resource)
                .copied()
                .flatten();
            program.semi_globals.insert(object.resource, semi_global);
            if let Some(resource) = semi_global {
                program.load(index, resource)?;
            }
        }
        Ok(program)
    }

    fn load(&mut self, index: &Index, resource: usize) -> Result<()> {
        if self.libraries.contains_key(&resource) {
            return Ok(());
        }
        let file = &index.resources[resource];
        let location = file.location(&index.root);
        let format = |tag: &str, id: u16, e| {
            sims_content::Error::format(&format!("{location} {tag} #{id}"), e)
        };
        let iff =
            Iff::parse(file.read()?).map_err(|e| sims_content::Error::format(&location, e))?;
        let mut library = Library::default();
        for chunk in &iff.chunks {
            let data = iff.data(chunk);
            match &chunk.tag {
                b"BHAV" => {
                    let behavior =
                        Behavior::parse(data).map_err(|e| format("BHAV", chunk.id, e))?;
                    library.behaviors.insert(chunk.id, behavior);
                }
                b"BCON" => {
                    let constants =
                        execution::constants(data).map_err(|e| format("BCON", chunk.id, e))?;
                    library.constants.insert(chunk.id, constants);
                }
                b"TTAB" => {
                    let table = InteractionTable::parse(data).ok();
                    library.interactions.insert(chunk.id, table);
                }
                b"SLOT" => {
                    let slots = Slots::parse(data).map_err(|e| format("SLOT", chunk.id, e))?;
                    let count = slots.entries.iter().filter(|s| s.kind == 0).count();
                    library.containers.insert(chunk.id, count);
                }
                _ => {}
            }
        }
        // The native resource order (the rsmp map when there is one), as the
        // catalog registers definitions. The old prototype kept physical
        // file order instead (legacy/crates/sims-content/src/vm.rs:187).
        let definitions = iff
            .resource_order(b"OBJD")
            .map_err(|e| sims_content::Error::format(&location, e))?;
        for chunk in definitions {
            let words =
                definition_words(iff.data(chunk)).map_err(|e| format("OBJD", chunk.id, e))?;
            library.definition_order.push(chunk.id);
            library.definitions.insert(chunk.id, words);
            let callbacks = match iff.get(b"OBJf", chunk.id) {
                Some(table) => {
                    ObjectFunctions::parse(iff.data(table))
                        .map_err(|e| format("OBJf", table.id, e))?
                        .entries
                }
                // The fallback reads the normalized record
                // (legacy/crates/sims-content/src/vm.rs:351-370).
                None => DEFINITION_CALLBACKS
                    .iter()
                    .map(|&offset| ObjectFunction {
                        condition: 0,
                        action: words[offset / 2] as u16,
                    })
                    .collect(),
            };
            library.callbacks.insert(chunk.id, callbacks);
        }
        self.libraries.insert(resource, library);
        Ok(())
    }

    /// Resolves a tree ID for code running on behalf of the object whose
    /// file is `owner`: 256..4096 are global, 4096..8192 the object's own,
    /// and 8192 up its semi-global file's. Calls made from inside a global
    /// or semi-global tree still resolve from the object's file.
    pub fn tree(&self, owner: usize, id: u16) -> Tree<'_> {
        let resource = match id {
            0..=255 => return Tree::Primitive(id),
            256..=4095 => self.global,
            4096..=8191 => owner,
            _ => match self.semi_globals.get(&owner).copied().flatten() {
                Some(resource) => resource,
                None => return Tree::Missing,
            },
        };
        match self
            .libraries
            .get(&resource)
            .and_then(|l| l.behaviors.get(&id))
        {
            Some(behavior) => Tree::Behavior(resource, behavior),
            None => Tree::Missing,
        }
    }

    /// A tuning constant, addressed as BCON table (high 9 bits) and entry
    /// (low 7 bits): tables 0..64 are the object's own BCONs from 4096,
    /// 64..128 its semi-global's from 8192, and 128..192 the global ones
    /// from 256.
    pub fn constant(&self, owner: usize, address: u16) -> Option<i16> {
        let table = address >> 7;
        let entry = usize::from(address & 127);
        let (resource, id) = match table {
            0..=63 => (owner, table + 4096),
            64..=127 => (
                self.semi_globals.get(&owner).copied().flatten()?,
                table - 64 + 8192,
            ),
            128..=191 => (self.global, table - 128 + 256),
            _ => return None,
        };
        self.libraries
            .get(&resource)?
            .constants
            .get(&id)?
            .get(entry)
            .copied()
    }

    /// An object definition's words, by the resource and OBJD chunk ID that
    /// hold it.
    pub fn definition(&self, resource: usize, chunk: u16) -> Option<&[i16; DEFINITION_WORDS]> {
        self.libraries.get(&resource)?.definitions.get(&chunk)
    }

    /// The interaction table an object definition uses: the TTAB its word 7
    /// names, in its own file or else its semi-global file. A tile of a
    /// multi-tile object whose word 7 is -1 uses its group's catalog
    /// definition's table (0x45726a, 0x43ae0a). Ok(None) is no table.
    ///
    /// Ported from the old prototype (legacy/crates/sims-content/src/vm.rs:448-490).
    pub fn interactions(
        &self,
        resource: usize,
        chunk: u16,
    ) -> std::result::Result<Option<&InteractionTable>, String> {
        let Some(library) = self.libraries.get(&resource) else {
            return Err(format!("resource {resource} is not linked"));
        };
        let Some(words) = library.definitions.get(&chunk) else {
            return Err(format!("resource {resource} has no OBJD #{chunk}"));
        };
        let mut table = words[7];
        if table == -1 {
            table = library
                .definition_order
                .iter()
                .filter_map(|id| library.definitions.get(id))
                .find(|parent| words[10] != 0 && parent[10] == words[10] && parent[11] == -1)
                .map_or(0, |parent| parent[7]);
        }
        if table <= 0 {
            return Ok(None);
        }
        let id = table as u16;
        let semi_global = self.semi_globals.get(&resource).copied().flatten();
        let found = library.interactions.get(&id).or_else(|| {
            self.libraries
                .get(&semi_global?)
                .and_then(|l| l.interactions.get(&id))
        });
        match found {
            None => Ok(None),
            Some(Some(table)) => Ok(Some(table)),
            Some(None) => Err(format!(
                "TTAB #{id} for OBJD #{chunk} of resource {resource} has no reader"
            )),
        }
    }

    /// How many objects an object definition's slots can hold: the kind 0
    /// entries of the SLOT chunk its word 20 names, or none when the file
    /// has no such chunk (0x431aaf, 0x46e2d1).
    pub fn containers(&self, resource: usize, chunk: u16) -> usize {
        let Some(library) = self.libraries.get(&resource) else {
            return 0;
        };
        library
            .definitions
            .get(&chunk)
            .and_then(|words| library.containers.get(&(words[20] as u16)))
            .copied()
            .unwrap_or(0)
    }

    /// The type attribute records the catalog makes, all zero: one for
    /// every definition whose word 90 asks for a number of words, in
    /// catalog order, two definitions with one GUID included (0x43ba52,
    /// 0x455f09). A count outside 0..32000 makes a record with no words.
    /// Every catalog definition must be linked.
    ///
    /// Ported from the old prototype (`resource_storage` in
    /// legacy/crates/sims-content/src/object_catalog/household.rs). A definition
    /// whose word 91 has bit 1 set wants its words cleared for each lot.
    /// When the original does that is not known, so one is an error; the
    /// Complete Collection has none.
    pub fn type_attributes(&self, catalog: &Catalog) -> Result<Vec<TypeAttributes>> {
        let mut records = Vec::new();
        for object in &catalog.objects {
            let words = self
                .definition(object.resource, object.chunk)
                .ok_or(Error::MissingDefinition(object.guid))?;
            let count = words[90];
            if count == 0 {
                continue;
            }
            if words[91] & 2 != 0 {
                return Err(Error::Unsupported(format!(
                    "GUID 0x{:08X} resets its type attributes for each lot",
                    object.guid
                )));
            }
            let count = usize::try_from(count).ok().filter(|&n| n < 32000);
            records.push(TypeAttributes {
                guid: object.guid,
                values: vec![0; count.unwrap_or(0)],
            });
        }
        Ok(records)
    }

    /// The tree an object's callback names, or 0 when it names none.
    pub fn callback(&self, object: &Object, callback: Callback) -> u16 {
        self.libraries
            .get(&object.resource)
            .and_then(|l| l.callbacks.get(&object.chunk))
            .and_then(|table| table.get(callback as usize))
            .map_or(0, |f| f.action)
    }

    /// The original's behavior signature (0x4026ed): the root tree's
    /// revision shifted left 16, plus the revision of every distinct tree
    /// reachable from the root through calls, the root included. Missing
    /// trees add nothing, and the sum wraps.
    pub fn signature(&self, owner: usize, root: u16) -> u32 {
        let revision = |id| match self.tree(owner, id) {
            Tree::Behavior(_, behavior) => behavior.revision,
            _ => 0,
        };
        let mut value = revision(root).wrapping_shl(16);
        let mut seen = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            value = value.wrapping_add(revision(id));
            if let Tree::Behavior(_, behavior) = self.tree(owner, id) {
                for instruction in &behavior.instructions {
                    let target = instruction.opcode & 0x7fff;
                    if target >= 256 {
                        pending.push(target);
                    }
                }
            }
        }
        value
    }
}

/// The career IDs of `work.iff`, or None when the table cannot be had from
/// it. See [`Program::careers`].
fn careers(index: &Index) -> Result<Option<Vec<i32>>> {
    if index.named("Careers.txt").next().is_some() {
        return Ok(None);
    }
    let Some((_, file)) = index.named("work.iff").next() else {
        return Ok(None);
    };
    let location = file.location(&index.root);
    let iff = Iff::parse(file.read()?).map_err(|e| sims_content::Error::format(&location, e))?;
    let chunks = iff
        .resource_order(b"CARR")
        .map_err(|e| sims_content::Error::format(&location, e))?;
    Ok(Some(
        chunks
            .iter()
            .map(|chunk| i32::from(chunk.id as i16))
            .collect(),
    ))
}

/// An OBJD's words as the original holds them. Registration (0x43ba52)
/// normalizes the resource with 0x431759 before reading any field
/// (legacy/crates/sims-sim/src/object_catalog/definition.rs:9): versions
/// 136 and 137 become 138, and if such a record is shorter than 216 bytes
/// and odd, its last byte is cleared. Every version is then cut or
/// zero-padded to 216 bytes, so all 108 words read, whatever the payload's
/// length.
pub(crate) fn definition_words(data: &[u8]) -> sims_formats::Result<[i16; DEFINITION_WORDS]> {
    let version = data
        .first_chunk::<4>()
        .map(|&b| u32::from_le_bytes(b))
        .ok_or_else(|| sims_formats::Error::new(0, "OBJD version absent"))?;
    let mut bytes = data.to_vec();
    if matches!(version, 136 | 137) {
        bytes[..4].copy_from_slice(&138u32.to_le_bytes());
        if bytes.len() < DEFINITION_WORDS * 2
            && !bytes.len().is_multiple_of(2)
            && let Some(last) = bytes.last_mut()
        {
            *last = 0;
        }
    }
    bytes.resize(DEFINITION_WORDS * 2, 0);
    let mut words = [0; DEFINITION_WORDS];
    for (word, pair) in words.iter_mut().zip(bytes.as_chunks::<2>().0) {
        *word = i16::from_le_bytes(*pair);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::{DEFINITION_WORDS, definition_words};

    #[test]
    fn short_definitions_are_padded() {
        let mut data = 138u32.to_le_bytes().to_vec();
        data.extend_from_slice(&[0x34, 0x12, 0x78]);
        let words = definition_words(&data).unwrap();
        assert_eq!(words[..4], [138, 0, 0x1234, 0x78]);
        assert!(words[4..].iter().all(|&w| w == 0));
    }

    #[test]
    fn versions_136_and_137_are_upgraded() {
        for version in [136u32, 137] {
            let mut data = version.to_le_bytes().to_vec();
            data.extend_from_slice(&[0x34, 0x12, 0x78]);
            // The odd trailing byte is cleared.
            assert_eq!(definition_words(&data).unwrap()[..4], [138, 0, 0x1234, 0]);
        }
    }

    #[test]
    fn long_definitions_are_cut() {
        let mut data = 138u32.to_le_bytes().to_vec();
        data.resize(300, 0xff);
        let words = definition_words(&data).unwrap();
        assert_eq!(words.len(), DEFINITION_WORDS);
        assert_eq!(words[DEFINITION_WORDS - 1], -1);
    }

    #[test]
    fn a_definition_needs_a_version() {
        assert!(definition_words(&[1, 2, 3]).is_err());
    }
}
