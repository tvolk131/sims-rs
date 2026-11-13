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

impl Catalog {
    pub fn build(index: &Index) -> Result<Self> {
        let global = index
            .resources
            .iter()
            .position(|r| r.mount == 0 && r.name().eq_ignore_ascii_case("Global.iff"))
            .ok_or_else(|| Error::Missing {
                root: index.root.clone(),
                path: "GameData/Global/Global.iff".into(),
            })?;
        let mut catalog = Self {
            global,
            objects: Vec::new(),
            by_guid: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        for (number, resource) in index.resources.iter().enumerate() {
            if !resource.name().to_ascii_lowercase().ends_with(".iff") {
                continue;
            }
            let location = resource.location(&index.root);
            let iff = match resource
                .read()
                .and_then(|bytes| Iff::parse(bytes).map_err(|e| Error::format(&location, e)))
            {
                Ok(iff) => iff,
                Err(error) => {
                    catalog.diagnostics.push(error.to_string());
                    continue;
                }
            };
            catalog.register(index, number, &location, &iff);
        }
        Ok(catalog)
    }

    /// The object a GUID resolves to.
    pub fn object(&self, guid: u32) -> Option<&Object> {
        self.by_guid.get(&guid).map(|&n| &self.objects[n])
    }

    fn register(&mut self, index: &Index, resource: usize, location: &str, iff: &Iff) {
        let definitions = match iff.resource_order(b"OBJD") {
            Ok(definitions) => definitions,
            Err(error) => {
                self.diagnostics.push(format!("{location}: {error}"));
                return;
            }
        };
        for chunk in definitions {
            match ObjectDefinition::parse(iff.data(chunk)) {
                Ok(definition) => {
                    let number = self.objects.len();
                    self.objects.push(Object {
                        guid: definition.guid,
                        resource,
                        chunk: chunk.id,
                        kind: definition.object_type,
                        master: definition.master_id,
                    });
                    if let Some(previous) = self.by_guid.insert(definition.guid, number) {
                        let previous = &self.objects[previous];
                        self.diagnostics.push(format!(
                            "GUID 0x{:08X} in {location} OBJD #{} replaces {} OBJD #{}",
                            definition.guid,
                            chunk.id,
                            index.resources[previous.resource].location(&index.root),
                            previous.chunk,
                        ));
                    }
                }
                Err(error) => self
                    .diagnostics
                    .push(format!("{location} OBJD #{}: {error}", chunk.id)),
            }
        }
        // An object file links at most one semi-global file.
        let Some(link) = iff.all(b"GLOB").next() else {
            return;
        };
        match global_link(iff.data(link)) {
            Ok(None) => {
                self.semi_globals.insert(resource, None);
            }
            Ok(Some(name)) => {
                let file = format!("{name}.iff");
                // The first file of that name in load order. Whether the
                // original picks the first is unverified.
                match index.named(&file).next() {
                    Some((target, _)) => {
                        self.semi_globals.insert(resource, Some(target));
                    }
                    None => self
                        .diagnostics
                        .push(format!("{location}: semi-global {file} not found")),
                }
            }
            Err(error) => self
                .diagnostics
                .push(format!("{location} GLOB #{}: {error}", link.id)),
        }
    }
}
