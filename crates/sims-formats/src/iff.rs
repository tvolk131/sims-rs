use crate::Result;
use crate::reader::{Reader, western};
use std::collections::BTreeMap;
use std::ops::Range;

/// A chunk flag: the chunk is deleted, free space to be reused.
pub const DELETED: u16 = 4;

#[derive(Debug, Clone)]
pub struct Chunk {
    pub tag: [u8; 4],
    pub id: u16,
    pub flags: u16,
    pub label: String,
    pub offset: usize,
    pub payload: Range<usize>,
}
impl Chunk {
    pub fn kind(&self) -> String {
        String::from_utf8_lossy(&self.tag).into_owned()
    }
}
#[derive(Debug, Clone)]
pub struct Iff {
    pub bytes: Vec<u8>,
    /// Every chunk in file order, deleted and replaced ones included. The
    /// game's resources are [`Iff::get`], [`Iff::all`] and
    /// [`Iff::resource_order`].
    pub chunks: Vec<Chunk>,
    resources: Resources,
}

/// The resources an IFF file holds, as the original's IFFResFile3 table
/// keeps them (`legacy/crates/sims-sim/src/object_catalog/iff.rs`, from
/// 0x41c252 and 0x41be90). A resource's key is its type, its ID and the
/// high byte of its flags.
///
/// - Without a resource map, the live chunks in file order. Deleted chunks
///   are skipped. A chunk whose key an earlier chunk has replaces it and
///   takes its place in the order.
/// - With a version-0 resource map, the map's entries in its order. Of two
///   entries with one key, the first is kept. A live chunk the map does
///   not list is not a resource. A map that lists a resource map with ID 0
///   is ignored, and the chunks are read as if there were none.
///
/// **Unverified** against the original: deleted chunks with a live twin,
/// replaced chunks and maps listing a map. These rules follow the frozen
/// prototype's reader. The game's own files hold no deleted chunk sharing
/// a key with a live one, and their only chunks sharing a type and ID are
/// `BMP_` chunks told apart by the flags' high byte.
#[derive(Debug, Clone, Default)]
struct Resources {
    /// Indices into [`Iff::chunks`] by type, in resource order.
    order: BTreeMap<[u8; 4], Vec<usize>>,
    /// Indices into [`Iff::chunks`] by key.
    by_key: BTreeMap<([u8; 4], u16, u8), usize>,
    /// The flags the resource map lists for a resource, by index into
    /// [`Iff::chunks`]; see [`Iff::resource_flags`].
    map_flags: BTreeMap<usize, u16>,
    /// Why the resource map could not be used. The table is then the
    /// chunks read as if there were no map, so lookups still work, but
    /// [`Iff::resource_order`] fails.
    map_error: Option<crate::Error>,
}

impl Resources {
    /// Adds a resource, or replaces the one with its key where it stands.
    /// With `first_wins`, a resource whose key is taken is dropped instead.
    fn add(&mut self, chunks: &[Chunk], index: usize, first_wins: bool) {
        let chunk = &chunks[index];
        let key = (chunk.tag, chunk.id, (chunk.flags >> 8) as u8);
        let order = self.order.entry(chunk.tag).or_default();
        match self.by_key.insert(key, index) {
            None => order.push(index),
            Some(old) if first_wins => {
                self.by_key.insert(key, old);
            }
            Some(old) => {
                let at = order.iter().position(|&i| i == old).expect("listed");
                order[at] = index;
            }
        }
    }

    fn physical(chunks: &[Chunk]) -> Self {
        let mut resources = Self::default();
        for (index, chunk) in chunks.iter().enumerate() {
            if chunk.flags & DELETED == 0 {
                resources.add(chunks, index, false);
            }
        }
        resources
    }

    /// The table for `bytes`: from its resource map if it has one. A
    /// resource map is never a resource itself: the open removes its type.
    fn build(bytes: &[u8], chunks: &[Chunk]) -> Self {
        let mut resources = Self::with_map(bytes, chunks);
        resources.order.remove(b"rsmp");
        resources.by_key.retain(|key, _| key.0 != *b"rsmp");
        resources
    }

    fn with_map(bytes: &[u8], chunks: &[Chunk]) -> Self {
        // Only 2.5 headers carry a map offset, in bytes 60..64. In 2.0
        // headers those bytes are the end of the signature text
        // ("...MAXIS 1996\0").
        let offset = if bytes.starts_with(b"IFF FILE 2.5:") {
            u32::from_be_bytes(bytes[60..64].try_into().unwrap()) as usize
        } else {
            0
        };
        if offset == 0 {
            return Self::physical(chunks);
        }
        match Self::mapped(bytes, chunks, offset) {
            Ok(Some(resources)) => resources,
            Ok(None) => Self::physical(chunks),
            Err(error) => Self {
                map_error: Some(error),
                ..Self::physical(chunks)
            },
        }
    }

    /// The table from the resource map at `offset`, or None when the map
    /// lists a map with ID 0.
    fn mapped(bytes: &[u8], chunks: &[Chunk], offset: usize) -> Result<Option<Self>> {
        // Chunks are in file order, so by offset.
        let at = |offset: usize| {
            chunks
                .binary_search_by_key(&offset, |c| c.offset)
                .ok()
                .map(|i| (i, &chunks[i]))
        };
        let map = at(offset)
            .filter(|(_, c)| c.tag == *b"rsmp" && c.flags & DELETED == 0)
            .ok_or_else(|| crate::Error::new(offset, "IFF resource map not found"))?
            .1;
        let map = ResourceMap::parse(&bytes[map.payload.clone()])?;
        if map
            .types
            .get(b"rsmp")
            .is_some_and(|entries| entries.iter().any(|e| e.id == 0 && e.flags >> 8 == 0))
        {
            return Ok(None);
        }
        let mut resources = Self::default();
        for (tag, entries) in &map.types {
            for entry in entries {
                // A later entry with a key already listed is dropped
                // unread, as the original's table insert drops it.
                let key = (*tag, entry.id, (entry.flags >> 8) as u8);
                if resources.by_key.contains_key(&key) {
                    continue;
                }
                // The entry's chunk must be there with the entry's key. Its
                // low flag bits are the map's to state, as in the
                // prototype, which reads the payload wherever the map
                // points and checks only the type there. **Unverified:**
                // the original looks the resource up by the map's ID
                // (0x41bbc5) and checks only the chunk's type (0x41d33b),
                // so a chunk whose ID or high flag byte differs would be
                // read there under the map's; here the map is refused. No
                // file of the install has one.
                let index = at(entry.offset as usize)
                    .filter(|(_, c)| {
                        c.tag == *tag && c.id == entry.id && c.flags >> 8 == entry.flags >> 8
                    })
                    .ok_or_else(|| {
                        crate::Error::new(
                            entry.offset as usize,
                            "resource map entry disagrees with chunk header",
                        )
                    })?
                    .0;
                resources.add(chunks, index, true);
                resources.map_flags.insert(index, entry.flags);
            }
        }
        Ok(Some(resources))
    }
}
impl Iff {
    pub fn parse(bytes: Vec<u8>) -> Result<Self> {
        let mut r = Reader::new(&bytes);
        let header = r.take(64)?;
        if !header.starts_with(b"IFF FILE 2.5:") && !header.starts_with(b"IFF FILE 2.0:") {
            return r.fail("invalid Maxis IFF signature");
        }
        let mut chunks = Vec::new();
        while r.remaining() > 0 {
            let offset = r.pos;
            let tag = r.take(4)?.try_into().unwrap();
            let length = r.be32()? as usize;
            if length < 76 {
                return r.fail("IFF chunk shorter than its 76-byte header");
            }
            let id = r.be16()?;
            let flags = r.be16()?;
            let label_bytes = r.take(64)?;
            let end = label_bytes.iter().position(|b| *b == 0).unwrap_or(64);
            let label = western(&label_bytes[..end]);
            let start = r.pos;
            r.skip(length - 76)?;
            chunks.push(Chunk {
                tag,
                id,
                flags,
                label,
                offset,
                payload: start..r.pos,
            });
        }
        let resources = Resources::build(&bytes, &chunks);
        Ok(Self {
            bytes,
            chunks,
            resources,
        })
    }
    /// The flags the original reads a resource's byte order from
    /// (0x41d3a7): its resource map entry's when the file's map is used,
    /// else the chunk header's.
    pub fn resource_flags(&self, chunk: &Chunk) -> u16 {
        self.chunks
            .binary_search_by_key(&chunk.offset, |c| c.offset)
            .ok()
            .and_then(|index| self.resources.map_flags.get(&index))
            .copied()
            .unwrap_or(chunk.flags)
    }
    pub fn data(&self, chunk: &Chunk) -> &[u8] {
        &self.bytes[chunk.payload.clone()]
    }
    /// The resource of type `tag` with ID `id` (and flags whose high byte
    /// is 0), as the original looks one up (see [`Resources`]).
    pub fn get(&self, tag: &[u8; 4], id: u16) -> Option<&Chunk> {
        self.resources
            .by_key
            .get(&(*tag, id, 0))
            .map(|&i| &self.chunks[i])
    }
    /// The resources of type `tag` in the original's order. When the
    /// resource map cannot be used, the chunks as if there were no map.
    pub fn all<'a>(&'a self, tag: &'a [u8; 4]) -> impl Iterator<Item = &'a Chunk> {
        self.resources
            .order
            .get(tag)
            .into_iter()
            .flatten()
            .map(|&i| &self.chunks[i])
    }
    /// Native IFFResFile3 enumeration. A version-0 resource map supplies its
    /// own per-type order (0x41be90/0x41bbc5); without a map, 0x41c252 appends
    /// live chunks in physical file order. Resource IDs are not sort keys.
    /// Fails when the file names a resource map that cannot be used.
    pub fn resource_order(&self, tag: &[u8; 4]) -> Result<Vec<&Chunk>> {
        match &self.resources.map_error {
            Some(error) => Err(error.clone()),
            None => Ok(self
                .resources
                .order
                .get(tag)
                .into_iter()
                .flatten()
                .map(|&i| &self.chunks[i])
                .collect()),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ResourceMapEntry {
    pub offset: u32,
    pub id: u16,
    pub flags: u16,
    pub label: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceMap {
    pub unused_bytes: u32,
    pub types: std::collections::BTreeMap<[u8; 4], Vec<ResourceMapEntry>>,
}
impl ResourceMap {
    /// Raw version-0 rsmp stream, 0x41be90 and 0x41b7cb. String alignment is
    /// relative to the enclosing payload, including its twelve-byte header.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        if r.take(8)? != [0; 8] || r.take(4)? != b"pmsr" {
            return r.fail("unsupported resource-map stream header");
        }
        let unused_bytes = r.u32()?;
        let count = r.u32()? as usize;
        r.count(count, 8)?;
        let mut types = std::collections::BTreeMap::new();
        for _ in 0..count {
            let tag = r.u32()?.to_be_bytes();
            let n = r.u32()? as usize;
            r.count(n, 10)?;
            let mut entries = vec![];
            for _ in 0..n {
                let offset = r.u32()?;
                let id = r.u16()?;
                let flags = r.u16()?;
                let label = western(r.cstr()?);
                if r.pos & 1 != 0 {
                    r.skip(1)?;
                }
                entries.push(ResourceMapEntry {
                    offset,
                    id,
                    flags,
                    label,
                });
            }
            if types.insert(tag, entries).is_some() {
                return r.fail("duplicate resource-map type");
            }
        }
        if r.remaining() != 0 {
            return r.fail("unconsumed resource-map bytes");
        }
        Ok(Self {
            unused_bytes,
            types,
        })
    }
}
