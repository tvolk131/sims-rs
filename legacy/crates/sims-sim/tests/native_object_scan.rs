use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::object_scan::{self, Entry, Host, Paths, PhaseHost, Scanner};
use std::collections::BTreeSet;

#[derive(Deserialize)]
struct Case<T> {
    input: T,
    original: Value,
}
#[derive(Deserialize)]
struct Phase {
    flags: i16,
    paths: Vec<Vec<u8>>,
    people: u8,
    loaded: u8,
    append: usize,
}
#[derive(Deserialize)]
struct Filter {
    path: Vec<u8>,
    category: u32,
}
#[derive(Deserialize)]
struct Phases {
    phases: Vec<Case<Phase>>,
    filters: Vec<Case<Filter>>,
}
struct Catalog {
    count: usize,
    append: usize,
    events: Vec<Value>,
}
impl PhaseHost for Catalog {
    fn count(&self) -> usize {
        self.count
    }
    fn scan(&mut self, path: &[u8], category: u32, people: u8) -> Result<(), String> {
        self.events.push(json!(["scan", path, category, people]));
        self.count += self.append;
        Ok(())
    }
    fn changed(&mut self) -> Result<(), String> {
        self.events.push(json!(["notify", 224, 0]));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_object_phases_and_callbacks_preserve_modes_categories_and_order() {
    let report: Phases = serde_json::from_str(include_str!("fixtures/object-scan.json")).unwrap();
    assert_eq!(report.phases.len(), 287);
    assert_eq!(report.filters.len(), 231);
    for (i, case) in report.phases.into_iter().enumerate() {
        let p = &case.input.paths;
        let paths = Paths {
            objects: p[0].clone(),
            user_objects: p[1].clone(),
            people: p[2].clone(),
            downloads: p[3].clone(),
            expansions: std::array::from_fn(|i| p[if i < 7 { i + 4 } else { 13 }].clone()),
            shared: p[11].clone(),
            npcs: p[12].clone(),
        };
        let mut host = Catalog {
            count: 2,
            append: case.input.append,
            events: vec![],
        };
        object_scan::phases(
            &paths,
            case.input.flags,
            case.input.people,
            case.input.loaded != 0,
            &mut host,
        )
        .unwrap();
        assert_eq!(
            json!({"events":host.events,"people":case.input.people}),
            case.original,
            "phase {i}"
        );
    }
    for (i, case) in report.filters.into_iter().enumerate() {
        let recurse = object_scan::recurse_directory(&case.input.path).unwrap();
        let events: Vec<Value> = if object_scan::object_file(&case.input.path).unwrap() {
            vec![json!(["file", case.input.path, case.input.category])]
        } else {
            vec![]
        };
        assert_eq!(
            json!({"recurse":recurse,"events":events}),
            case.original,
            "filter {i}"
        );
    }
}

#[derive(Clone, Deserialize)]
struct Row {
    name: Vec<u8>,
    directory: u8,
    writable: u8,
}
#[derive(Deserialize)]
struct Directory {
    path: Vec<u8>,
    entries: Vec<Row>,
}
#[derive(Deserialize)]
struct Input {
    path: Vec<u8>,
    registered: Vec<Vec<u8>>,
    directories: Vec<Directory>,
    category: u32,
    writable: u8,
}
#[derive(Deserialize)]
struct Traversals {
    cases: Vec<Case<Input>>,
}
struct Search {
    directory: Vec<u8>,
    entries: Vec<Row>,
    at: usize,
    manager: usize,
}
impl Search {
    fn entry(&self) -> Option<Entry> {
        self.entries.get(self.at).map(|e| Entry {
            directory: self.directory.clone(),
            name: e.name.clone(),
            directory_flag: e.directory,
            writable: e.writable,
        })
    }
}
struct Walker<'a> {
    input: &'a Input,
    events: Vec<Value>,
    next: usize,
    managers: BTreeSet<usize>,
    searches: usize,
}
impl Host for Walker<'_> {
    type Manager = usize;
    type Search = Search;
    fn global(&mut self) -> usize {
        0
    }
    fn registered(&mut self, path: &[u8]) -> Result<bool, String> {
        let found = self.input.registered.iter().any(|p| p == path);
        self.events.push(json!(["registered", path, found]));
        Ok(found)
    }
    fn temporary(&mut self) -> Result<usize, String> {
        self.next += 1;
        assert!(self.managers.insert(self.next));
        self.events.push(json!(["create", self.next]));
        Ok(self.next)
    }
    fn register(&mut self, manager: usize, path: &[u8]) -> Result<(), String> {
        assert!(self.managers.contains(&manager));
        self.events.push(json!(["register", manager, path, 0, 1]));
        Ok(())
    }
    fn clear(&mut self, manager: usize) -> Result<(), String> {
        assert!(self.managers.contains(&manager));
        self.events.push(json!(["clear", manager]));
        Ok(())
    }
    fn dispose(&mut self, manager: usize) -> Result<(), String> {
        assert!(self.managers.remove(&manager));
        self.events.push(json!(["dispose", manager]));
        Ok(())
    }
    fn first(&mut self, manager: usize, pattern: &[u8]) -> Result<(Search, Option<Entry>), String> {
        assert!(manager == 0 || self.managers.contains(&manager));
        self.events.push(json!(["first", manager, pattern]));
        let directory = pattern.strip_suffix(b"*").unwrap_or(pattern).to_vec();
        let entries = self
            .input
            .directories
            .iter()
            .find(|d| d.path == directory)
            .map(|d| d.entries.clone())
            .unwrap_or_default();
        let search = Search {
            directory,
            entries,
            at: 0,
            manager,
        };
        self.searches += 1;
        let entry = search.entry();
        Ok((search, entry))
    }
    fn next(&mut self, manager: usize, search: &mut Search) -> Result<Option<Entry>, String> {
        assert_eq!(manager, search.manager);
        search.at += 1;
        let entry = search.entry();
        self.events
            .push(json!(["next", manager, search.directory, entry.is_some()]));
        Ok(entry)
    }
    fn close(&mut self, _: Search) -> Result<(), String> {
        self.searches -= 1;
        Ok(())
    }
    fn directory(&mut self, path: &[u8]) -> Result<(), String> {
        self.events.push(json!(["directory", path]));
        Ok(())
    }
    fn file(&mut self, path: &[u8], category: u32, writable: u8) -> Result<(), String> {
        self.events.push(json!(["file", path, category, writable]));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_object_traversal_preserves_recursion_and_manager_lifetime() {
    let report: Traversals =
        serde_json::from_str(include_str!("fixtures/object-traversal.json")).unwrap();
    assert_eq!(report.cases.len(), 154);
    for (i, case) in report.cases.into_iter().enumerate() {
        let mut host = Walker {
            input: &case.input,
            events: vec![],
            next: 0,
            managers: BTreeSet::new(),
            searches: 0,
        };
        let mut scanner = Scanner {
            category: case.input.category,
            writable: case.input.writable,
        };
        let found = scanner.scan(&case.input.path, &mut host).unwrap();
        assert_eq!(
            json!({"result": if found {0} else {u32::MAX},"writable":scanner.writable,"events":host.events}),
            case.original,
            "traversal {i}"
        );
        assert!(host.managers.is_empty());
        assert_eq!(host.searches, 0);
    }
}

#[derive(Deserialize)]
struct File {
    name: Vec<u8>,
    words: [u32; 4],
    flags: [u8; 3],
}
#[derive(Deserialize)]
struct Map {
    path: Vec<u8>,
    files: Vec<File>,
}
#[derive(Deserialize)]
struct Queries {
    directories: Vec<Map>,
    patterns: Vec<Vec<u8>>,
}
#[derive(Deserialize)]
struct Iterators {
    cases: Vec<Case<Queries>>,
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_object_file_iterator_preserves_normalization_hash_order_and_metadata() {
    use sims_sim::pose::clothing::catalog_mount::{Manager, Metadata};
    let report: Iterators =
        serde_json::from_str(include_str!("fixtures/object-file-iterator.json")).unwrap();
    assert_eq!(report.cases.len(), 17);
    for (i, case) in report.cases.into_iter().enumerate() {
        let mut manager = Manager::default();
        for d in case.input.directories {
            let files = manager.directory_mut(&d.path).unwrap();
            for f in d.files {
                files
                    .insert(
                        &f.name,
                        Metadata {
                            archive: f.words[0],
                            offset: f.words[1],
                            reserved: f.words[2],
                            size: f.words[3],
                            archived: f.flags[0],
                            writable: f.flags[1],
                            directory: f.flags[2],
                        },
                    )
                    .unwrap();
            }
        }
        let mut answers = vec![];
        for pattern in case.input.patterns {
            let (mut cursor, mut entry) =
                object_scan::MountedSearch::first(&manager, &pattern).unwrap();
            let mut entries = vec![];
            while let Some(e) = entry {
                entries.push(json!({"directory":e.directory,"name":e.name,"flags":[e.directory_flag,e.writable]}));
                entry = cursor.next(&manager).unwrap();
            }
            assert_eq!(cursor.next(&manager).unwrap(), None);
            answers.push(entries);
        }
        assert_eq!(json!(answers), case.original, "iterator {i}");
    }
}
