use sims_sim::object_catalog::{
    Key,
    definition::{Catalog, Entry},
    move_tool::{
        Fields, Groups,
        selected::{Masters, lookup_master},
    },
};

fn add(
    c: &mut Catalog,
    resource: u32,
    guid: u32,
    multipart: i16,
    subindex: i16,
    group: i16,
) -> u32 {
    let mut data = vec![0; 216];
    for (offset, word) in [(0x14, multipart), (0x16, subindex), (0x8a, group)] {
        data[offset..offset + 2].copy_from_slice(&word.to_le_bytes());
    }
    data[0x1c..0x20].copy_from_slice(&guid.to_le_bytes());
    let row = c.entries.len();
    c.entries.push(Entry {
        key: Key {
            resource,
            chunk: row as u16,
        },
        semiglobal: None,
        sprite_resource: resource,
        global: None,
        name: vec![],
        data,
        index: row as u32,
        flags: 0,
    });
    c.guids.insert(guid, row);
    row as u32 + 1
}

#[test]
fn master_uses_first_matching_resource_and_signed_group_with_lazy_nonnull_cache() {
    let mut c = Catalog::default();
    let part = add(&mut c, 10, 100, -2, 0, 1);
    add(&mut c, 11, 101, -2, -1, 1); // wrong resource
    add(&mut c, 10, 102, 2, -1, 1); // wrong signed group
    add(&mut c, 10, 103, -2, 1, 1); // not master
    let mut masters = Masters::default();
    assert_eq!(masters.resolve(&c, part).unwrap(), None);
    assert!(!masters.0.contains_key(&part));
    let first = add(&mut c, 10, 104, -2, -1, 1);
    let later = add(&mut c, 10, 104, -2, -1, 1);
    assert_eq!(c.guids[&104], later as usize - 1);
    assert_eq!(masters.resolve(&c, part).unwrap(), Some(first));
    masters = serde_json::from_slice(&serde_json::to_vec(&masters).unwrap()).unwrap();
    c.entries[first as usize - 1].data[0x16..0x18].copy_from_slice(&0_i16.to_le_bytes());
    assert_eq!(
        masters.resolve(&c, part).unwrap(),
        Some(first),
        "nonnull cache is retained"
    );
    masters.0.insert(part, 0);
    assert_eq!(
        masters.resolve(&c, part).unwrap(),
        Some(later),
        "zero cache retries"
    );
    let single = add(&mut c, 10, 105, 0, 9, 0);
    assert_eq!(masters.resolve(&c, single).unwrap(), Some(single));
}

#[test]
fn snapshot_master_lookup_rereads_ordered_catalog_without_selection_cache() {
    let mut c = Catalog::default();
    let part = add(&mut c, 10, 100, -2, 0, 1);
    add(&mut c, 11, 101, -2, -1, 1);
    add(&mut c, 10, 102, 2, -1, 1);
    let first = add(&mut c, 10, 103, -2, -1, 1);
    let later = add(&mut c, 10, 103, -2, -1, 1);
    let mut cache = Masters::default();
    assert_eq!(cache.resolve(&c, part).unwrap(), Some(first));
    c.entries[first as usize - 1].data[0x16..0x18].copy_from_slice(&0_i16.to_le_bytes());
    let cold: Catalog = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
    assert_eq!(cold, c);
    assert_eq!(lookup_master(&cold, part).unwrap(), Some(later));
    assert_eq!(cache.resolve(&cold, part).unwrap(), Some(first));
    c.entries[later as usize - 1].data[0x14..0x16].copy_from_slice(&2_i16.to_le_bytes());
    assert_eq!(lookup_master(&c, part).unwrap(), None);
    assert_eq!(cache.resolve(&c, part).unwrap(), Some(first));
    let single = add(&mut c, 10, 104, 0, -1, 0);
    assert_eq!(lookup_master(&c, single).unwrap(), Some(single));
    assert!(lookup_master(&c, 0).is_err());
}

#[test]
fn selected_definition_returns_special_index_and_preserves_argument_and_position() {
    let mut c = Catalog::default();
    for group in 1..=7 {
        add(&mut c, 1, group as u32, 0, 0, group);
    }
    let mut groups = Groups::default();
    groups.populate(&c).unwrap();
    for n in 0..7 {
        let mut f = Fields::constructor_fields();
        f.argument = 123;
        f.position = [9, -6];
        let answer = groups
            .select_definition(&mut f, &mut Masters::default(), &c, n + 1)
            .unwrap();
        assert_eq!(answer, -20000 + 2000 * n as i32);
        assert_eq!(f.definition, Some(n + 1));
        assert_eq!(f.mode, n as i32 + 3);
        assert_eq!(f.cursor, 7);
        assert_eq!(f.flags, [0, 0, u8::from(n != 6), u8::from(n != 3), 0]);
        assert_eq!(f.special, u8::from(n < 2));
        assert_eq!(f.subtype, u8::from(n == 3));
        assert_eq!((f.argument, f.position), (123, [9, -6]));
    }
    let single = add(&mut c, 1, 8, 0, 0, 0);
    let mut f = Fields::constructor_fields();
    assert_eq!(
        groups
            .select_definition(&mut f, &mut Masters::default(), &c, single)
            .unwrap(),
        0
    );
    assert_eq!((f.mode, f.cursor, f.flags), (1, 6, [0, 0, 0, 1, 0]));
    // Selection classifies the resolved master, not the supplied member.
    let member = add(&mut c, 2, 9, 7, 0, 0);
    let master = add(&mut c, 2, 3, 7, -1, 3);
    assert_eq!(
        groups
            .select_definition(&mut f, &mut Masters::default(), &c, member)
            .unwrap(),
        -16000
    );
    assert_eq!(f.definition, Some(master));
}

#[test]
fn reverse_group_lookup_uses_first_guid_and_wrapping_index_ignoring_nominal_range() {
    let mut c = Catalog::default();
    let id = add(&mut c, 1, 10, 0, 0, 0);
    let mut g = Groups::default();
    g.0[0].guids = vec![11, 10, 10];
    g.0[0].first = i32::MAX;
    g.0[0].last = 0;
    assert_eq!(g.index_of(0, &c, id).unwrap(), i32::MIN);
    g.0[0].first = -1;
    g.0[1].guids = vec![10];
    let mut f = Fields::constructor_fields();
    assert_eq!(
        g.select_definition(&mut f, &mut Masters::default(), &c, id)
            .unwrap(),
        -18000,
        "a matching GUID returning zero does not end the caller's search"
    );
    assert_eq!(f.mode, 4);
}

#[test]
fn absent_master_reports_mandatory_guid_access_after_original_prefix_writes() {
    let mut c = Catalog::default();
    let id = add(&mut c, 1, 10, 5, 0, 1);
    let mut f = Fields::constructor_fields();
    f.mode = 42;
    f.special = 255;
    f.subtype = 255;
    f.flags = [9; 5];
    assert!(
        Groups::default()
            .select_definition(&mut f, &mut Masters::default(), &c, id)
            .is_err()
    );
    assert_eq!(
        (f.definition, f.cursor, f.special, f.subtype),
        (None, 6, 0, 0)
    );
    assert_eq!((f.mode, f.flags), (42, [9; 5]));
}

fn master_corpus_case(c: &serde_json::Value, cold: bool) -> serde_json::Value {
    let mut catalog = Catalog::default();
    for (n, row) in c["catalog"].as_array().unwrap().iter().enumerate() {
        add(
            &mut catalog,
            row["resource"].as_u64().unwrap() as u32,
            n as u32,
            row["group"].as_i64().unwrap() as i16,
            row["subindex"].as_i64().unwrap() as i16,
            0,
        );
    }
    let source = c["source"].as_u64().unwrap() as u32;
    let mut cache = Masters::default();
    if let Some(value) = c["cache"].as_u64() {
        cache.0.insert(source, value as u32);
    }
    let mut results = vec![];
    for step in 0..4 {
        if step == 2 {
            let row = c["mutation"].as_u64().unwrap() as usize - 1;
            catalog.entries[row].data[0x16..0x18].copy_from_slice(&0_i16.to_le_bytes());
        }
        if cold {
            (catalog, cache) =
                serde_json::from_slice(&serde_json::to_vec(&(catalog, cache)).unwrap()).unwrap();
        }
        let result = if step % 2 == 0 {
            lookup_master(&catalog, source)
        } else {
            cache.resolve(&catalog, source)
        }
        .unwrap();
        results.push(serde_json::json!({"result":result,"cache":cache.0.get(&source).copied()}));
    }
    serde_json::json!(results)
}
#[test]
#[ignore = "private prepared master cases; no native execution"]
fn prepared_master_lookup_survives_catalog_cold_restore() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-object-undo-master-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["native_execution"], false);
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 75);
    for c in cases {
        assert_eq!(
            master_corpus_case(c, false),
            master_corpus_case(c, true),
            "{c}"
        );
    }
}
#[test]
#[ignore = "original master corpus; generate with all Cargo/rustc absent"]
fn uncached_and_cached_master_lookups_match_original_instructions() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/master-definition.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["native_execution"], true);
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 75);
    for c in cases {
        for cold in [false, true] {
            assert_eq!(
                master_corpus_case(&c["input"], cold),
                c["original"],
                "{c} cold={cold}"
            );
        }
    }
}
