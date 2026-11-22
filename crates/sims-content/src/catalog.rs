use crate::pool::{ArchivePool, worker_count};
use crate::{Error, Index, Result};
use sims_formats::{behavior::ObjectDefinition, execution::global_link, iff::Iff};
use std::collections::BTreeMap;

/// One registered object definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub guid: u32,
    /// Index into [`Index::resources`] of the IFF file defining it.
    pub resource: usize,
    /// The OBJD chunk ID within that file.
    pub chunk: u16,
    /// The OBJD type: 2 is a person.
    pub kind: u16,
    /// Nonzero for each tile of a multi-tile object.
    pub master: u16,
}

/// Every object definition an install provides, and the files objects need.
#[derive(Debug, Clone)]
pub struct Catalog {
    /// The resource holding the global behaviors (`Global.iff`).
    pub global: usize,
    /// Definitions in registration order: the index's order of files, then
    /// each file's OBJD resource order.
    pub objects: Vec<Object>,
    /// The definition each GUID resolves to. Later registrations win, as in
    /// the original's lookup, so a download can replace a built-in object.
    pub by_guid: BTreeMap<u32, usize>,
    /// For each IFF resource with a GLOB link, the semi-global resource it
    /// names, or None when the link is empty.
    pub semi_globals: BTreeMap<usize, Option<usize>>,
    /// Problems that do not stop the catalog: unreadable files, bad
    /// definitions, GUIDs registered more than once, broken links.
    pub diagnostics: Vec<String>,
}

/// Per-resource extracted registration data computed independently on worker
/// threads before deterministic sequential registration.
enum ScannedResource {
    Error(String),
    Parsed {
        location: String,
        definitions: Vec<std::result::Result<(u16, ObjectDefinition), (u16, sims_formats::Error)>>,
        glob: Option<std::result::Result<Option<String>, (u16, sims_formats::Error)>>,
    },
}

fn scan_iff_resource(index: &Index, number: usize, pool: &ArchivePool) -> (usize, ScannedResource) {
    let resource = &index.resources[number];
    let location = resource.location(&index.root);
    let iff = match resource
        .read_with(pool)
        .and_then(|bytes| Iff::parse(bytes).map_err(|e| Error::format(&location, e)))
    {
        Ok(iff) => iff,
        Err(error) => return (number, ScannedResource::Error(error.to_string())),
    };
    let chunks = match iff.resource_order(b"OBJD") {
        Ok(chunks) => chunks,
        Err(error) => {
            return (
                number,
                ScannedResource::Error(format!("{location}: {error}")),
            );
        }
    };
    let definitions = chunks
        .into_iter()
        .map(|chunk| {
            ObjectDefinition::parse(iff.data(chunk))
                .map(|def| (chunk.id, def))
                .map_err(|e| (chunk.id, e))
        })
        .collect();
    let glob = iff
        .all(b"GLOB")
        .next()
        .map(|link| global_link(iff.data(link)).map_err(|e| (link.id, e)));
    (
        number,
        ScannedResource::Parsed {
            location,
            definitions,
            glob,
        },
    )
}

impl Catalog {
    pub fn build(index: &Index) -> Result<Self> {
        Self::build_with_pool(index, &ArchivePool::new())
    }

    /// Builds the catalog using a shared [`ArchivePool`] for archive reads and
    /// parallelizing IFF reads/parses across worker threads while registering
    /// every definition and diagnostic in exact [`Index::resources`] order.
    pub fn build_with_pool(index: &Index, pool: &ArchivePool) -> Result<Self> {
        let global = index
            .resources
            .iter()
            .position(|r| r.mount == 0 && r.name().eq_ignore_ascii_case("Global.iff"))
            .ok_or_else(|| Error::Missing {
                root: index.root.clone(),
                path: "GameData/Global/Global.iff".into(),
            })?;
        let iff_indices: Vec<usize> = index
            .resources
            .iter()
            .enumerate()
            .filter_map(|(number, resource)| {
                let name = resource.name();
                let bytes = name.as_bytes();
                (bytes.len() >= 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".iff"))
                    .then_some(number)
            })
            .collect();
        let workers = worker_count(iff_indices.len());
        let scanned: Vec<(usize, ScannedResource)> = if workers <= 1 {
            iff_indices
                .iter()
                .map(|&number| scan_iff_resource(index, number, pool))
                .collect()
        } else {
            let chunk_size = iff_indices.len().div_ceil(workers);
            std::thread::scope(|scope| {
                let handles: Vec<_> = iff_indices
                    .chunks(chunk_size)
                    .map(|chunk| {
                        scope.spawn(move || {
                            chunk
                                .iter()
                                .map(|&number| scan_iff_resource(index, number, pool))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                let mut out = Vec::with_capacity(iff_indices.len());
                for handle in handles {
                    out.extend(handle.join().expect("catalog worker panicked"));
                }
                out
            })
        };
        let first_by_name = index.first_by_file_name();
        let mut catalog = Self {
            global,
            objects: Vec::new(),
            by_guid: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        for (number, entry) in scanned {
            catalog.register_scanned(index, &first_by_name, number, entry);
        }
        Ok(catalog)
    }

    /// The object a GUID resolves to.
    #[inline]
    pub fn object(&self, guid: u32) -> Option<&Object> {
        self.by_guid.get(&guid).map(|&n| &self.objects[n])
    }

    fn register_scanned(
        &mut self,
        index: &Index,
        first_by_name: &BTreeMap<String, usize>,
        resource: usize,
        entry: ScannedResource,
    ) {
        let (location, definitions, glob) = match entry {
            ScannedResource::Error(message) => {
                self.diagnostics.push(message);
                return;
            }
            ScannedResource::Parsed {
                location,
                definitions,
                glob,
            } => (location, definitions, glob),
        };
        for item in definitions {
            match item {
                Ok((chunk_id, definition)) => {
                    let number = self.objects.len();
                    self.objects.push(Object {
                        guid: definition.guid,
                        resource,
                        chunk: chunk_id,
                        kind: definition.object_type,
                        master: definition.master_id,
                    });
                    if let Some(previous) = self.by_guid.insert(definition.guid, number) {
                        let previous = &self.objects[previous];
                        self.diagnostics.push(format!(
                            "GUID 0x{:08X} in {location} OBJD #{chunk_id} replaces {} OBJD #{}",
                            definition.guid,
                            index.resources[previous.resource].location(&index.root),
                            previous.chunk,
                        ));
                    }
                }
                Err((chunk_id, error)) => self
                    .diagnostics
                    .push(format!("{location} OBJD #{chunk_id}: {error}")),
            }
        }
        // An object file links at most one semi-global file.
        let Some(glob) = glob else {
            return;
        };
        match glob {
            Ok(None) => {
                self.semi_globals.insert(resource, None);
            }
            Ok(Some(name)) => {
                let file = format!("{name}.iff");
                // The first file of that name in load order. Whether the
                // original picks the first is unverified.
                let found = if file.contains(['\\', '/']) {
                    index.named(&file).next().map(|(target, _)| target)
                } else {
                    first_by_name.get(&file.to_ascii_lowercase()).copied()
                };
                match found {
                    Some(target) => {
                        self.semi_globals.insert(resource, Some(target));
                    }
                    None => self
                        .diagnostics
                        .push(format!("{location}: semi-global {file} not found")),
                }
            }
            Err((link_id, error)) => self
                .diagnostics
                .push(format!("{location} GLOB #{link_id}: {error}")),
        }
    }
}
