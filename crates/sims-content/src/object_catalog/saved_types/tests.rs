use super::*;
use crate::{
    Index, avatar_catalog::Directories, avatar_installation::Installation,
    object_resources::Resources,
};
use sims_sim::{
    object_catalog::{Key, definition::Entry},
    saved_types::{Function, Gtab, Record, State},
};
use std::sync::Weak;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Source {
    cursor: usize,
    block_main: bool,
    records: Vec<Record>,
    events: Vec<String>,
    #[serde(skip)]
    owner: Option<Weak<()>>,
}
impl Host for Source {
    fn retire_declaration(&mut self, _: &saved_types::Declaration) -> Result<(), String> {
        unreachable!()
    }
    fn gtab(&mut self) -> Result<Gtab, String> {
        self.events.push("Gtab".into());
        Ok(Gtab::Absent)
    }
    fn begin_objt(&mut self) -> Result<Option<i32>, String> {
        self.events.push("open".into());
        Ok(Some(3))
    }
    fn next_objt(&mut self, _: i32) -> Result<Option<Record>, String> {
        let record = self.records.get(self.cursor).cloned();
        self.cursor += 1;
        self.events.push("record".into());
        Ok(record)
    }
    fn end_objt(&mut self) -> Result<(), String> {
        self.events.push("release".into());
        Ok(())
    }
    fn signature(
        &mut self,
        resources: &mut Resources<'_>,
        key: Key<u32>,
        function: Function,
    ) -> Result<u32, String> {
        if !self
            .owner
            .as_ref()
            .is_some_and(|owner| Weak::ptr_eq(owner, &resources.retained_identity()))
        {
            return Err("signature host requires actual restored resource capability".into());
        }
        assert_eq!(
            key,
            Key {
                resource: 7,
                chunk: 130
            }
        );
        if function == Function::Main && self.block_main {
            return Err("current Main source unavailable".into());
        }
        self.events.push(format!("{function:?}"));
        Ok(if function == Function::Init { 7 } else { 99 })
    }
}
fn index() -> Index {
    Index {
        root: "unused-saved-types".into(),
        resources: vec![],
        archives: 0,
        diagnostics: vec![],
    }
}
fn catalog(index: &Index, definitions: definition::Catalog) -> Catalog<'_> {
    let installation = Installation::new(index, vec![]).unwrap();
    let resources = Resources::new(
        index,
        Directories::default(),
        installation.directory_search(),
    );
    let mut catalog = Catalog::new(
        resources,
        super::super::Settings {
            filter_flagged: false,
            global_directory: vec![],
            global_people: 0,
            global: None,
        },
    );
    catalog.definitions = definitions;
    catalog
}
fn definitions() -> definition::Catalog {
    let entries = [(10u32, 2), (20, 2), (10, 0)]
        .into_iter()
        .enumerate()
        .map(|(i, (guid, flags))| {
            let mut data = vec![0; 216];
            data[28..32].copy_from_slice(&guid.to_le_bytes());
            Entry {
                key: Key {
                    resource: 7,
                    chunk: 128 + i as u16,
                },
                semiglobal: None,
                sprite_resource: 1,
                global: None,
                name: vec![],
                data,
                index: i as u32,
                flags: ((128 + i as u32) << 16) | flags,
            }
        })
        .collect();
    definition::Catalog {
        entries,
        guids: [(10, 2), (20, 1)].into(),
        attributes: vec![],
    }
}
fn source(catalog: &Catalog<'_>) -> Source {
    Source {
        cursor: 0,
        block_main: true,
        records: vec![Record {
            guid: 10,
            signatures: Some([7, 8]),
            saved_index: 3,
            kind: Some(2),
            name: vec![0x81],
            extra: Some(9),
        }],
        events: vec![],
        owner: Some(catalog.resources.retained_identity()),
    }
}
fn advance_to_pending(catalog: &mut Catalog<'_>, read: &mut Read<Source>) {
    for _ in 0..32 {
        match catalog.step_saved_type_read(read) {
            Ok(false) => {}
            Ok(true) => panic!("missing source was bypassed"),
            Err(e) => {
                assert_eq!(e, "current Main source unavailable");
                return;
            }
        }
    }
    panic!("pending source was not reached");
}
fn finish(catalog: &mut Catalog<'_>, read: &mut Read<Source>) {
    read.host.block_main = false;
    for _ in 0..32 {
        if catalog.step_saved_type_read(read).unwrap() {
            return;
        }
    }
    panic!("metadata did not complete");
}
#[test]
fn canonical_flags_and_owned_source_cursor_survive_missing_getter_and_cold_metadata() {
    let index = index();
    let mut warm = catalog(&index, definitions());
    let state = State::from_parts(&warm.definitions, vec![91, 92, 93], vec![], vec![77]).unwrap();
    let mut read = warm
        .begin_saved_type_read(state, 64, source(&warm))
        .unwrap();
    let mut vm = sims_vm::Vm::new(sims_vm::World::default(), 0x12345678);
    vm.tick = 42;
    let before = vm.clone();
    assert!(
        warm.publish_saved_type_changes(&read, &mut vm.world)
            .is_err()
    );
    assert_eq!(vm, before);
    advance_to_pending(&mut warm, &mut read);
    assert_eq!(read.state.saved_indices, [0, 0, 3]);
    assert_eq!(warm.definitions.entries[2].flags & 2, 0);
    assert_eq!(read.host.events.iter().filter(|e| *e == "Init").count(), 1);
    let cut = warm.saved_type_metadata(&read).unwrap();
    let raw = serde_json::to_vec(&cut).unwrap();
    let cut: Metadata<Source> = serde_json::from_slice(&raw).unwrap();
    let mut cold = catalog(&index, cut.definitions.clone());
    let mut restored = cold.resume_saved_type_read(cut).unwrap();
    assert!(
        cold.step_saved_type_read(&mut restored)
            .unwrap_err()
            .contains("resource capability")
    );
    restored.host.owner = Some(cold.resources.retained_identity());
    finish(&mut warm, &mut read);
    finish(&mut cold, &mut restored);
    assert_eq!(warm.definitions, cold.definitions);
    assert_eq!(
        serde_json::to_value(&read).unwrap(),
        serde_json::to_value(&restored).unwrap()
    );
    assert_eq!(read.host.events.iter().filter(|e| *e == "Init").count(), 1);
    assert_eq!(read.state.type_guids, [20, 0, 0, 10]);
    warm.publish_saved_type_changes(&read, &mut vm.world)
        .unwrap();
    let mut expected = before;
    expected.world.changed_resources = [10, 20].into();
    assert_eq!(vm, expected);
    assert_eq!(warm.definitions.entries[0].flags & 2, 2); // earlier duplicate retained
    assert_eq!(warm.definitions.entries[1].flags & 2, 2); // unlisted resource retained
}
#[test]
fn metadata_from_another_canonical_catalog_is_rejected_without_copying_flags() {
    let index = index();
    let mut catalog = catalog(&index, definitions());
    let state = State::from_parts(&catalog.definitions, vec![0; 3], vec![], vec![]).unwrap();
    let mut read = catalog
        .begin_saved_type_read(state, 64, source(&catalog))
        .unwrap();
    advance_to_pending(&mut catalog, &mut read);
    let mut cut = catalog.saved_type_metadata(&read).unwrap();
    cut.definitions.entries[0].flags ^= 2;
    let before = catalog.definitions.clone();
    assert!(catalog.resume_saved_type_read(cut).is_err());
    assert_eq!(catalog.definitions, before);
    let mut state = read.state;
    state.saved_indices.pop();
    assert!(
        catalog
            .begin_saved_type_read(state, 64, source(&catalog))
            .is_err()
    );
    assert_eq!(catalog.definitions, before);
}
#[test]
fn absent_objt_does_not_clear_retained_changed_flags() {
    #[derive(Clone)]
    struct Absent;
    impl Host for Absent {
        fn retire_declaration(&mut self, _: &saved_types::Declaration) -> Result<(), String> {
            unreachable!()
        }
        fn gtab(&mut self) -> Result<Gtab, String> {
            Ok(Gtab::Absent)
        }
        fn begin_objt(&mut self) -> Result<Option<i32>, String> {
            Ok(None)
        }
        fn next_objt(&mut self, _: i32) -> Result<Option<Record>, String> {
            unreachable!()
        }
        fn end_objt(&mut self) -> Result<(), String> {
            unreachable!()
        }
        fn signature(
            &mut self,
            _: &mut Resources<'_>,
            _: Key<u32>,
            _: Function,
        ) -> Result<u32, String> {
            unreachable!()
        }
    }
    let index = index();
    let mut catalog = catalog(&index, definitions());
    let expected = catalog.definitions.clone();
    let state = State::from_parts(&catalog.definitions, vec![99; 3], vec![], vec![7]).unwrap();
    let mut read = catalog.begin_saved_type_read(state, 64, Absent).unwrap();
    for _ in 0..32 {
        if catalog.step_saved_type_read(&mut read).unwrap() {
            break;
        }
    }
    assert!(read.execution.complete());
    assert_eq!(catalog.definitions, expected);
    assert_eq!(read.state.type_guids, [10]);
    let mut world = sims_vm::World::default();
    catalog
        .publish_saved_type_changes(&read, &mut world)
        .unwrap();
    assert_eq!(world.changed_resources, [20].into()); // last GUID10 row is unchanged
}
