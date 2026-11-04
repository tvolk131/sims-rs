//! Synchronous object-folder services over the source-bound virtual installation.
//! The visitor performs IFF registration at each callback, before enumeration
//! proceeds. Collecting paths alone does not construct the native object catalog.
use super::*;
use sims_sim::object_scan::{self, Entry, MountedSearch, Paths};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObjectVisit {
    pub path: Vec<u8>,
    pub category: u32,
    pub people: u8,
    pub writable: u8,
}
pub trait ObjectVisitor {
    /// Number of registered object definitions, not visited files.
    fn count(&self) -> usize;
    fn file(&mut self, installation: &mut Installation<'_>, visit: ObjectVisit) -> Result<()>;
    /// Native catalog-changed notification (e0, 0).
    fn changed(&mut self) -> Result<()>;
}
impl Installation<'_> {
    pub fn scan_objects(
        &mut self,
        paths: &Paths,
        expansions: i16,
        people: u8,
        loaded: bool,
        visitor: &mut impl ObjectVisitor,
    ) -> Result<()> {
        let mut host = ObjectHost {
            installation: self,
            visitor,
            temporary: BTreeMap::new(),
            next: 0,
            people,
        };
        object_scan::phases(paths, expansions, people, loaded, &mut host)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            host.temporary.is_empty(),
            "object scan retained a temporary file manager"
        );
        Ok(())
    }
}
struct ObjectHost<'a, 'b, V> {
    installation: &'a mut Installation<'b>,
    visitor: &'a mut V,
    temporary: BTreeMap<usize, Manager>,
    next: usize,
    people: u8,
}
impl<V> ObjectHost<'_, '_, V> {
    fn manager(&self, id: usize) -> Result<&Manager, String> {
        if id == 0 {
            Ok(&self.installation.manager)
        } else {
            self.temporary
                .get(&id)
                .ok_or_else(|| "object scan manager absent".into())
        }
    }
}
struct Search {
    manager: usize,
    cursor: MountedSearch,
}
impl<V: ObjectVisitor> object_scan::PhaseHost for ObjectHost<'_, '_, V> {
    fn count(&self) -> usize {
        self.visitor.count()
    }
    fn scan(&mut self, path: &[u8], category: u32, people: u8) -> Result<(), String> {
        self.people = people;
        let _ = object_scan::Scanner {
            category,
            writable: 0,
        }
        .scan(path, self)?;
        Ok(())
    }
    fn changed(&mut self) -> Result<(), String> {
        self.visitor.changed().map_err(|e| e.to_string())
    }
}
impl<V: ObjectVisitor> object_scan::Host for ObjectHost<'_, '_, V> {
    type Manager = usize;
    type Search = Search;
    fn global(&mut self) -> usize {
        0
    }
    fn registered(&mut self, path: &[u8]) -> Result<bool, String> {
        self.installation.manager.registered(path)
    }
    fn temporary(&mut self) -> Result<usize, String> {
        self.next += 1;
        self.temporary.insert(self.next, Manager::default());
        Ok(self.next)
    }
    fn register(&mut self, manager: usize, path: &[u8]) -> Result<(), String> {
        let installation = &mut self.installation;
        installation.prepare(path).map_err(|e| e.to_string())?;
        let manager = self
            .temporary
            .get_mut(&manager)
            .ok_or("object scan temporary manager absent")?;
        installation
            .sources
            .scan(manager, path, false, true)
            .map_err(|e| e.to_string())
    }
    fn clear(&mut self, manager: usize) -> Result<(), String> {
        *self
            .temporary
            .get_mut(&manager)
            .ok_or("object scan temporary manager absent")? = Manager::default();
        Ok(())
    }
    fn dispose(&mut self, manager: usize) -> Result<(), String> {
        self.temporary
            .remove(&manager)
            .ok_or("object scan temporary manager absent")?;
        Ok(())
    }
    fn first(&mut self, manager: usize, pattern: &[u8]) -> Result<(Search, Option<Entry>), String> {
        let (cursor, entry) = MountedSearch::first(self.manager(manager)?, pattern)?;
        Ok((Search { manager, cursor }, entry))
    }
    fn next(&mut self, manager: usize, search: &mut Search) -> Result<Option<Entry>, String> {
        if manager != search.manager {
            return Err("object scan iterator changed managers".into());
        }
        search.cursor.next(self.manager(manager)?)
    }
    fn close(&mut self, _: Search) -> Result<(), String> {
        Ok(())
    }
    fn file(&mut self, path: &[u8], category: u32, writable: u8) -> Result<(), String> {
        self.visitor
            .file(
                self.installation,
                ObjectVisit {
                    path: path.to_vec(),
                    category,
                    people: self.people,
                    writable,
                },
            )
            .map_err(|e| e.to_string())
    }
}
