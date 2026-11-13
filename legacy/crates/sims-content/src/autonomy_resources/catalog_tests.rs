//! Authored protocol checks; public source binding is exercised by the private
//! same-cut receiver helper against the real sealed Global source.
use super::*;
use crate::object_resources::{
    RetainedComponent, RetainedIdentities, RetainedResources, RetainedStream,
};
use sims_sim::object_catalog::{access::Access, resources};
use std::sync::Arc;

fn input(present: bool) -> (Index, RetainedResources, Provider) {
    let mut components = super::super::tests::components(present);
    let component = components.entries[&1].clone();
    let bytes = component.stream.bytes.clone();
    let mut source = Source::observed("Global.iff");
    source.module_sha256 = hash(&bytes);
    source.offset = 0;
    source.size = bytes.len() as u64;
    let standalone = Provider {
        source,
        aggregate: super::super::tests::aggregate(),
        components: components.clone(),
        events: vec![],
    };
    let index = Index {
        root: "unused-shared-global".into(),
        resources: vec![crate::Resource {
            locator: "Global.iff".into(),
            name: "Global.iff".into(),
            source: "must-not-open".into(),
            offset: 0,
            size: bytes.len() as u64,
            unpacked_size: bytes.len() as u64,
            archived: false,
            owned: Some(Arc::from(bytes.clone())),
        }],
        archives: 0,
        diagnostics: vec![],
    };
    let mut buckets = vec![vec![]; 193];
    let key = b"Global.iff".to_vec();
    buckets[sims_sim::pose::clothing::name_map::hash(&key) as usize % 193].push(resources::Entry {
        key,
        references: 17,
        resource: 7,
    });
    let mut aggregate = standalone.aggregate.clone();
    aggregate.slots[0] = Some(11);
    let component = components.entries.remove(&1).unwrap();
    let retained = RetainedResources {
        cache_buckets: buckets,
        cache_len: 1,
        aggregates: BTreeMap::from([(7, aggregate)]),
        components: BTreeMap::from([(
            11,
            RetainedComponent {
                iff: component.iff,
                stream: RetainedStream {
                    locator: "Global.iff".into(),
                    source_sha256: Sha256::digest(&bytes).into(),
                    bytes,
                    filename: b"./Global.iff".to_vec(),
                    access: Access {
                        opened: true,
                        writable: 0,
                    },
                    position: component.stream.position,
                    pending_backend_write_bytes: 0,
                },
            },
        )]),
        handles: components
            .handles
            .into_iter()
            .map(|(h, (_, local))| (h, (11, local)))
            .collect(),
        buffers: components.buffers,
        identities: RetainedIdentities::NewUnpublished {
            next_aggregate: 8,
            next_component: 12,
            next_handle: components.next_handle,
        },
    };
    (index, retained, standalone)
}
fn mount(index: &Index, input: RetainedResources) -> Resources<'_> {
    let installation = crate::avatar_installation::Installation::new(index, vec![]).unwrap();
    Resources::from_retained(
        index,
        crate::avatar_catalog::Directories::default(),
        installation.directory_search(),
        input,
    )
    .unwrap()
}
fn selector(resources: &Resources<'_>, standalone: &Provider) -> CatalogProvider {
    // Test-only binding for the small authored source. The public constructor
    // accepts only the pinned complete original Global archive/module profile.
    CatalogProvider {
        source: standalone.source.clone(),
        global: 7,
        identity: resources.retained_identity(),
        events: vec![],
    }
}
fn equal(view: &CatalogProvider, resources: &Resources<'_>, standalone: &Provider) {
    let observed = view.observe(resources).unwrap();
    let original = &standalone.components.entries[&1];
    assert_eq!(observed.iff, &original.iff);
    assert_eq!(observed.stream_position, original.stream.position);
    assert_eq!(observed.aggregate.error, standalone.aggregate.error);
    assert_eq!(
        observed.aggregate.configurations,
        standalone.aggregate.configurations
    );
    assert_eq!(observed.aggregate.filters, standalone.aggregate.filters);
    assert_eq!(
        observed.external_handles,
        standalone
            .components
            .handles
            .iter()
            .map(|(&h, &(_, l))| (h, l))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        observed.next_external_handle,
        standalone.components.next_handle
    );
    assert_eq!(view.events, standalone.events);
    assert_eq!(resources.cache.entries().next().unwrap().references, 17);
}

#[test]
fn borrowed_service_matches_standalone_complete_state_and_readonly_errors() {
    for present in [false, true] {
        let (index, input, mut standalone) = input(present);
        let mut resources = mount(&index, input);
        let mut view = selector(&resources, &standalone);
        equal(&view, &resources, &standalone);
        let request = Request::Load(view.binding());
        assert_eq!(
            view.service_with_resources(&mut resources, &request, None)
                .unwrap(),
            standalone.service(&request, None).unwrap()
        );
        equal(&view, &resources, &standalone);
        let table = Table {
            rows: vec![constants::Row {
                name: b"changed".to_vec(),
                value_bits: 0x80000000,
                comment: vec![],
            }],
        };
        let request = Request::Flush(view.binding());
        assert_eq!(
            view.service_with_resources(&mut resources, &request, Some(&table))
                .unwrap(),
            standalone.service(&request, Some(&table)).unwrap()
        );
        equal(&view, &resources, &standalone);
        assert_eq!(
            view.events.last().unwrap().return_value,
            if present { -99 } else { -94 }
        );
    }
}

#[test]
fn same_bytes_in_a_different_owner_do_not_satisfy_the_alias() {
    let (index, input, standalone) = input(true);
    let original = mount(&index, input.clone());
    let mut other = mount(&index, input);
    let mut view = selector(&original, &standalone);
    let before = view.events.clone();
    let count = other.live_counts();
    assert!(
        view.service_with_resources(&mut other, &Request::Load(view.binding()), None)
            .is_err()
    );
    assert_eq!(view.events, before);
    assert_eq!(other.live_counts(), count);
    equal(&view, &original, &standalone);
}

#[test]
fn binding_and_unconverted_callback_reject_before_native_owner_changes() {
    let (index, input, standalone) = input(true);
    let mut resources = mount(&index, input.clone());
    let mut view = selector(&resources, &standalone);
    let mut wrong = view.binding();
    wrong.module_sha256 = "wrong".into();
    assert!(
        view.service_with_resources(&mut resources, &Request::Load(wrong), None)
            .is_err()
    );
    equal(&view, &resources, &standalone);
    let mut bad = input;
    let mut iff = serde_json::to_value(&bad.components[&11].iff).unwrap();
    iff["storage"][0]["disk_flags"] = 0.into();
    bad.components.get_mut(&11).unwrap().iff = serde_json::from_value(iff).unwrap();
    let mut unconverted = mount(&index, bad);
    let mut view = selector(&unconverted, &standalone);
    let before = view.observe(&unconverted).unwrap().iff.clone();
    let cursor = view.observe(&unconverted).unwrap().stream_position;
    assert!(
        view.service_with_resources(&mut unconverted, &Request::Load(view.binding()), None)
            .is_err()
    );
    assert!(view.events.is_empty());
    let after = view.observe(&unconverted).unwrap();
    assert_eq!(after.iff, &before);
    assert_eq!(after.stream_position, cursor);
}

#[test]
fn legacy_saved_reader_rejects_new_alias_path_without_running_old_initialization() {
    use crate::object_catalog::{Catalog, retained::*};
    use crate::save_storage::PersonReader;
    struct Legacy(usize);
    impl PersonReader for Legacy {
        fn initialize(
            &mut self,
            _: &mut sims_vm::Program,
            _: &mut sims_vm::World,
            _: sims_vm::ObjectId,
            object: sims_vm::Object,
        ) -> Result<sims_vm::Object> {
            self.0 += 1;
            Ok(object)
        }
        fn read_animator(
            &mut self,
            _: &mut sims_vm::Program,
            _: &mut sims_vm::World,
            _: sims_vm::ObjectId,
            object: sims_vm::Object,
            _: &sims_formats::saved_objects::Person,
        ) -> Result<sims_vm::Object> {
            Ok(object)
        }
    }
    let (index, input, standalone) = input(true);
    let resources = mount(&index, input);
    let mut view = selector(&resources, &standalone);
    let mut catalog = Catalog::from_retained(
        resources,
        RetainedCatalog {
            definitions: sims_sim::object_catalog::definition::Catalog {
                entries: vec![],
                guids: BTreeMap::new(),
                attributes: vec![],
            },
            sprite_entries: vec![],
            sprite_epoch: sims_sim::object_catalog::sprite_pool::IdentityEpoch::NewUnpublished {
                next: 1,
            },
            settings: RetainedSettings {
                filter_flagged: false,
                global_directory: b"./Global/".to_vec(),
                global_people: 0,
                global: Some(7),
            },
            observation_epoch: ObservationEpoch::NewUnpublished,
        },
    )
    .unwrap();
    view.validate_catalog(&catalog).unwrap();
    let mut legacy = Legacy(0);
    assert!(legacy.validate_catalog_resources(&catalog, &view).is_err());
    let mut world = sims_vm::World::default();
    let before = world.clone();
    assert!(
        legacy
            .initialize_with_catalog(
                &mut sims_vm::Program::default(),
                &mut world,
                7,
                sims_vm::Object::default(),
                &mut catalog,
                &mut view
            )
            .is_err()
    );
    assert_eq!(world, before);
    assert_eq!(legacy.0, 0);
    equal(&view, &catalog.resources, &standalone);
}
