use super::*;

fn fixture(writable: bool) -> (Index, RetainedResources) {
    let baseline = vec![0; 256];
    let source_sha256 = Sha256::digest(&baseline).into();
    let index = Index {
        root: std::path::PathBuf::from("unopened-retained-test"),
        resources: vec![crate::Resource {
            locator: "fixture.iff".into(),
            name: "fixture.iff".into(),
            source: "must-not-open-this-source".into(),
            offset: 0,
            size: 256,
            unpacked_size: 256,
            archived: false,
            owned: Some(Arc::from(baseline)),
        }],
        archives: 0,
        diagnostics: vec![],
    };
    let tag = u32::from_be_bytes(*b"BCON");
    let mut inner = vec![vec![]; 193];
    inner[1] = vec![(1u32, 0usize)];
    let mut outer = vec![vec![]; 193];
    outer[tag as usize % 193] = vec![(
        tag,
        serde_json::json!({"buckets": inner, "len":1, "short":true}),
    )];
    let iff: Iff = serde_json::from_value(serde_json::json!({
        "valid":true, "error":-7,"map_offset":176,"unused":0,
        "write_cursor":256,"modified":false,
        "table":{"buckets":outer,"len":1,"short":false},
        "storage":[{"tag":tag,"id":1,"flags":16,"disk_flags":16,"ordinal":1,
        "offset":64,"name":[65],"payload":{"handle":2,"bytes":[8,9]}}],
        "next_handle":3
    }))
    .unwrap();
    let mut cache_buckets = vec![vec![]; 193];
    let key = b"fixture.iff".to_vec();
    let hash = sims_sim::pose::clothing::name_map::hash(&key) as usize % 193;
    cache_buckets[hash].push(resources::Entry {
        key,
        references: 17,
        resource: 4,
    });
    let mut aggregate = Composite::empty();
    aggregate.configure(2, b".iff");
    aggregate.slots[0] = Some(7);
    aggregate.error = -13;
    let mut bytes = vec![0; 256];
    if writable {
        bytes[70] = 3;
    }
    let input = RetainedResources {
        cache_buckets,
        cache_len: 1,
        aggregates: BTreeMap::from([(4, aggregate)]),
        components: BTreeMap::from([(
            7,
            RetainedComponent {
                iff,
                stream: RetainedStream {
                    locator: "fixture.iff".into(),
                    source_sha256,
                    bytes,
                    filename: b"./Characters/fixture.iff".to_vec(),
                    access: Access {
                        opened: true,
                        writable: u8::from(writable),
                    },
                    position: 123,
                    pending_backend_write_bytes: 0,
                },
            },
        )]),
        handles: BTreeMap::from([(9, (7, 2))]),
        buffers: BTreeMap::from([(8, vec![1, 2])]),
        identities: RetainedIdentities::NewUnpublished {
            next_aggregate: 5,
            next_component: 8,
            next_handle: 10,
        },
    };
    (index, input)
}

fn import(index: &Index, input: RetainedResources) -> Result<Resources<'_>, String> {
    let installation =
        crate::avatar_installation::Installation::new(index, vec![]).map_err(|e| e.to_string())?;
    Resources::from_retained(
        index,
        Directories::default(),
        installation.directory_search(),
        input,
    )
}

#[test]
fn retained_resource_import_keeps_complete_state_and_resident_lookup_without_io() {
    let (index, input) = fixture(false);
    let expected = input.clone();
    let encoded = serde_json::to_vec(&input).unwrap();
    let input = serde_json::from_slice(&encoded).unwrap();
    let mut resources = import(&index, input).unwrap();
    assert_eq!(resources.owner.aggregates, expected.aggregates);
    assert_eq!(resources.owner.components.handles, expected.handles);
    assert_eq!(resources.owner.components.buffers, expected.buffers);
    assert_eq!(resources.cache.entries().next().unwrap().references, 17);
    assert_eq!(resources.data(8).unwrap(), [1, 2]);
    assert_eq!(resources.data(9).unwrap(), [8, 9]);
    assert!(resources.component_opens().is_empty());
    let Component::Iff { state, stream } = &resources.owner.components.entries[&7] else {
        panic!()
    };
    assert_eq!(state, &expected.components[&7].iff);
    assert_eq!(stream.file.position, 123);
    assert_eq!(stream.access, expected.components[&7].stream.access);
    assert_eq!(
        stream.file.bytes.as_ref(),
        Some(&expected.components[&7].stream.bytes)
    );
    assert_eq!(
        resources
            .by_id(4, u32::from_be_bytes(*b"BCON"), 1, 0)
            .unwrap(),
        9
    );
    let Component::Iff { stream, .. } = &resources.owner.components.entries[&7] else {
        panic!()
    };
    assert_eq!(stream.file.position, 123);
}

#[test]
fn writable_import_uses_explicit_current_bytes_and_preserves_original_source() {
    let (index, input) = fixture(true);
    let current = input.components[&7].stream.bytes.clone();
    let resources = import(&index, input).unwrap();
    let overlay = resources.overlay().unwrap();
    assert_eq!(overlay.read("fixture.iff"), Some(current.as_slice()));
    assert_eq!(index.resources[0].read().unwrap(), vec![0; 256]);
}

#[test]
fn retained_resource_import_rejects_missing_aliases_sources_and_unsupported_owners() {
    let (index, input) = fixture(false);
    let mut bad = input.clone();
    bad.handles.clear();
    assert!(import(&index, bad).is_err());
    let mut bad = input.clone();
    bad.handles.insert(7, (7, 2));
    assert!(import(&index, bad).is_err());
    let mut bad = input.clone();
    bad.buffers.insert(9, vec![]);
    assert!(import(&index, bad).is_err());
    let mut bad = input.clone();
    bad.components.clear();
    assert!(import(&index, bad).is_err());
    let mut bad = input.clone();
    bad.components
        .get_mut(&7)
        .unwrap()
        .stream
        .pending_backend_write_bytes = 1;
    assert!(import(&index, bad).is_err());
    let mut bad = input.clone();
    bad.components.get_mut(&7).unwrap().stream.access.opened = false;
    assert!(import(&index, bad).is_err());
    let mut bad = input.clone();
    bad.components.get_mut(&7).unwrap().stream.source_sha256[0] ^= 1;
    assert!(import(&index, bad).is_err());
    let mut bad = input;
    bad.components.get_mut(&7).unwrap().stream.bytes[0] ^= 1;
    assert!(import(&index, bad).is_err());
}

#[test]
fn retained_resource_fields_cannot_be_omitted_or_misspelled() {
    let (_, input) = fixture(false);
    let value = serde_json::to_value(input).unwrap();
    for key in value.as_object().unwrap().keys() {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<RetainedResources>(missing).is_err(),
            "{key}"
        );
    }
    let mut bad = value.clone();
    bad["next_handle"] = 1.into();
    assert!(serde_json::from_value::<RetainedResources>(bad).is_err());
    for key in value["components"]["7"]["stream"]
        .as_object()
        .unwrap()
        .keys()
    {
        let mut missing = value.clone();
        missing["components"]["7"]["stream"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(
            serde_json::from_value::<RetainedResources>(missing).is_err(),
            "{key}"
        );
    }
}

#[test]
fn captured_payload_absence_requires_explicit_null_without_changing_saved_readers() {
    let (_, input) = fixture(false);
    let value = serde_json::to_value(&input.components[&7]).unwrap();
    let mut missing = value.clone();
    missing["iff"]["storage"][0]
        .as_object_mut()
        .unwrap()
        .remove("payload");
    assert!(serde_json::from_value::<RetainedComponent>(missing.clone()).is_err());
    // Existing managed snapshots retain their accepted omission semantics.
    let old: Iff = serde_json::from_value(missing["iff"].clone()).unwrap();
    assert!(old.records().next().unwrap().payload.is_none());
    let mut empty = value;
    empty["iff"]["storage"][0]["payload"] = serde_json::Value::Null;
    let captured: RetainedComponent = serde_json::from_value(empty).unwrap();
    assert!(captured.iff.records().next().unwrap().payload.is_none());
}
