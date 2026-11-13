//! Authored service-boundary cases for the source-derived saved-type reader.
//! These are not original whole-household captures or parity tick credit.
use serde::{Deserialize, Serialize};
use sims_sim::{
    object_catalog::{
        Key,
        definition::{Catalog, Entry},
    },
    saved_types::*,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Services {
    records: Vec<Record>,
    cursor: usize,
    version: Option<i32>,
    gtab_present: bool,
    block_init: bool,
    block_main: bool,
    block_retire: bool,
    block_retire_guid: Option<u32>,
    events: Vec<String>,
    signatures: [u32; 2],
}
impl Host for Services {
    fn retire_declaration(&mut self, d: &Declaration) -> Result<(), String> {
        if self.block_retire || self.block_retire_guid == Some(d.guid) {
            return Err("retirement pending".into());
        }
        self.events.push(format!("retire:{}", d.guid));
        Ok(())
    }
    fn gtab(&mut self) -> Result<Gtab, String> {
        self.events.push("Gtab#1".into());
        Ok(if self.gtab_present {
            Gtab::Present
        } else {
            Gtab::Absent
        })
    }
    fn begin_objt(&mut self) -> Result<Option<i32>, String> {
        self.events.push("objt#0".into());
        Ok(self.version)
    }
    fn next_objt(&mut self, _: i32) -> Result<Option<Record>, String> {
        let result = self.records.get(self.cursor).cloned();
        self.events
            .push(format!("read:{:?}", result.as_ref().map(|r| r.guid)));
        self.cursor += 1;
        Ok(result)
    }
    fn end_objt(&mut self) -> Result<(), String> {
        self.events.push("release:objt".into());
        Ok(())
    }
    fn signature(&mut self, key: Key<u32>, function: Function) -> Result<u32, String> {
        if function == Function::Init && self.block_init {
            return Err("Init source unavailable".into());
        }
        if function == Function::Main && self.block_main {
            return Err("Main source unavailable".into());
        }
        self.events.push(format!("{:?}:{}", function, key.chunk));
        Ok(self.signatures[usize::from(function == Function::Main)])
    }
}
fn catalog() -> Catalog {
    let mut result = Catalog::default();
    for (i, guid) in [10_u32, 11, 12].into_iter().enumerate() {
        let mut data = vec![0; 216];
        data[28..32].copy_from_slice(&guid.to_le_bytes());
        data[0x12..0x14].copy_from_slice(&(if i == 0 { 2_i16 } else { 1 }).to_le_bytes());
        result.entries.push(Entry {
            key: Key {
                resource: 1,
                chunk: 128 + i as u16,
            },
            semiglobal: None,
            sprite_resource: 0,
            global: None,
            name: vec![],
            data,
            index: i as u32,
            flags: 0x800005,
        });
        result.guids.insert(guid, i);
    }
    result
}
fn state(c: &Catalog) -> State {
    State::from_parts(c, vec![7, 8, 9], vec![], vec![99, 98]).unwrap()
}
fn services(version: Option<i32>, records: Vec<Record>) -> Services {
    Services {
        version,
        records,
        cursor: 0,
        gtab_present: false,
        block_init: false,
        block_main: false,
        block_retire: false,
        block_retire_guid: None,
        events: vec![],
        signatures: [20, 30],
    }
}
fn record(version: i32, guid: u32, id: i16) -> Record {
    Record {
        guid,
        saved_index: id,
        signatures: (version > 0).then_some([20, 30]),
        kind: (version >= 2).then_some(-3),
        name: b"raw\x80 name".to_vec(),
        extra: (version >= 3).then_some(123),
    }
}
fn finish(e: &mut Execution, s: &mut State, c: &mut Catalog, h: &mut Services) {
    for _ in 0..1000 {
        if e.step(s, c, h).unwrap() {
            return;
        }
    }
    panic!("saved-type reader exceeded authored budget")
}

#[test]
fn absent_source_resets_indices_preserves_flags_and_publishes_every_definition() {
    let mut c = catalog();
    let mut s = state(&c);
    let mut h = services(None, vec![]);
    let mut e = Execution::new(65536);
    finish(&mut e, &mut s, &mut c, &mut h);
    assert_eq!(s.saved_indices, [0, 0, 0]);
    assert_eq!(s.type_guids, [12]);
    assert!(c.entries.iter().all(|e| e.flags == 0x800005));
    assert_eq!(h.events, ["Gtab#1", "objt#0"]);
}

#[test]
fn legacy_kind_and_signature_short_circuit_preserve_other_flag_bits() {
    for version in [0, 1, 2, 3, 4] {
        let mut c = catalog();
        let mut s = state(&c);
        let mut h = services(Some(version), vec![record(version, 10, 4)]);
        h.signatures[0] = 21;
        let mut e = Execution::new(65536);
        finish(&mut e, &mut s, &mut c, &mut h);
        assert_eq!(c.entries[0].flags, 0x800007);
        assert_eq!(s.saved_indices, [4, 0, 0]);
        assert!(!h.events.iter().any(|e| e.starts_with("Main:")));
        assert_eq!(
            h.events.iter().filter(|e| e.starts_with("Init:")).count(),
            usize::from(version > 0)
        );
        assert_eq!(s.type_guids, [12, 0, 0, 0, 10]);
    }
}

#[test]
fn main_source_failure_keeps_prior_writes_and_cold_resume_does_not_repeat_init() {
    let mut c = catalog();
    let mut s = state(&c);
    let mut h = services(Some(3), vec![record(3, 10, 5)]);
    h.block_main = true;
    let mut e = Execution::new(65536);
    loop {
        match e.step(&mut s, &mut c, &mut h) {
            Ok(false) => {}
            Err(error) => {
                assert_eq!(error, "Main source unavailable");
                break;
            }
            Ok(true) => panic!(),
        }
    }
    assert_eq!(s.saved_indices[0], 5);
    assert_eq!(c.entries[0].flags, 0x800005);
    assert_eq!(
        h.events.iter().filter(|v| v.starts_with("Init:")).count(),
        1
    );
    let cold = serde_json::to_vec(&(e, s, c, h)).unwrap();
    let (mut e, mut s, mut c, mut h): (Execution, State, Catalog, Services) =
        serde_json::from_slice(&cold).unwrap();
    h.block_main = false;
    finish(&mut e, &mut s, &mut c, &mut h);
    assert_eq!(
        h.events.iter().filter(|v| v.starts_with("Init:")).count(),
        1
    );
    let mut reference_catalog = catalog();
    let mut reference_state = state(&reference_catalog);
    let mut reference_services = services(Some(3), vec![record(3, 10, 5)]);
    let mut reference = Execution::new(65536);
    finish(
        &mut reference,
        &mut reference_state,
        &mut reference_catalog,
        &mut reference_services,
    );
    assert_eq!(
        (e, s, c, h),
        (
            reference,
            reference_state,
            reference_catalog,
            reference_services
        )
    );
}

#[test]
fn declaration_retirement_precedes_lookup_and_unknown_versions_use_native_defaults() {
    for version in [0, 1, 2, 3] {
        let mut c = catalog();
        let mut s = state(&c);
        s.declarations = vec![
            Some(Declaration {
                name: vec![],
                guid: 77,
                extra: 8,
                kind: 9,
                saved_index: 3,
            }),
            None,
        ];
        let mut h = services(Some(version), vec![record(version, 99, 6)]);
        h.block_retire = true;
        let mut e = Execution::new(65536);
        assert!(!e.step(&mut s, &mut c, &mut h).unwrap());
        assert!(e.step(&mut s, &mut c, &mut h).is_err());
        assert_eq!(s.saved_indices, [0, 0, 0]);
        assert!(h.events.is_empty());
        h.block_retire = false;
        finish(&mut e, &mut s, &mut c, &mut h);
        assert_eq!(h.events[0], "retire:77");
        assert_eq!(h.events[1], "Gtab#1");
        let d = s.declarations[0].as_ref().unwrap();
        assert_eq!(d.guid, 99);
        assert_eq!(d.kind, if version >= 2 { -3 } else { -1 });
        assert_eq!(d.extra, if version >= 3 { 123 } else { 0 });
        assert_eq!(d.name, b"raw\x80 name");
        assert_eq!(s.type_guids, [12]);
        assert!(
            !h.events
                .iter()
                .any(|v| v.starts_with("Init:") || v.starts_with("Main:"))
        );
    }
}

#[test]
fn duplicate_indices_follow_catalog_publication_order_and_zero_guid_terminates() {
    let mut c = catalog();
    let mut s = state(&c);
    let mut h = services(
        Some(3),
        vec![
            record(3, 11, 4),
            record(3, 10, 4),
            record(3, 0, 2),
            record(3, 12, 7),
        ],
    );
    let mut e = Execution::new(65536);
    finish(&mut e, &mut s, &mut c, &mut h);
    assert_eq!(s.type_guids, [12, 0, 0, 0, 11]);
    assert_eq!(h.cursor, 3);
    assert_eq!(s.saved_indices, [4, 4, 0]);
}

#[test]
fn present_gtab_is_a_retained_unsupported_owner_not_a_successful_identity_map() {
    let mut c = catalog();
    let mut s = state(&c);
    let mut h = services(Some(3), vec![]);
    h.gtab_present = true;
    let mut e = Execution::new(65536);
    while e.step(&mut s, &mut c, &mut h).is_ok() {}
    assert!(
        e.step(&mut s, &mut c, &mut h)
            .unwrap_err()
            .contains("present-Gtab")
    );
    assert_eq!(h.events, ["Gtab#1"]);
    assert_eq!(s.saved_indices, [0, 0, 0]);
    assert!(!e.complete());
}

#[test]
fn negative_native_index_stops_at_allocation_extent_after_source_release() {
    let mut c = catalog();
    let mut s = state(&c);
    let mut h = services(Some(3), vec![record(3, 10, -1)]);
    let mut e = Execution::new(65536);
    let error = loop {
        if let Err(error) = e.step(&mut s, &mut c, &mut h) {
            break error;
        }
    };
    assert!(error.contains("4294967296"));
    assert_eq!(h.events.last().unwrap(), "release:objt");
    assert_eq!(s.saved_indices[0], -1);
    assert_eq!(s.type_guids, [0]);
    assert!(!e.complete());
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_43a516_authored_component_cases_match_versions_flags_declarations_and_call_order() {
    use serde_json::{Value, json};
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/saved-types-native.json")).unwrap();
    assert_eq!(fixture["original_household_ticks_added"], 0);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 24);
    for (case_number, case) in cases.iter().enumerate() {
        let input = &case["input"];
        let version = input["version"].as_i64().unwrap() as i32;
        let mut c = Catalog::default();
        let mut prior = vec![];
        let resources = input["resources"].as_array().unwrap();
        for (i, r) in resources.iter().enumerate() {
            let guid = r["guid"].as_u64().unwrap() as u32;
            let mut data = vec![0; 216];
            data[28..32].copy_from_slice(&guid.to_le_bytes());
            data[0x12..0x14].copy_from_slice(&(r["kind"].as_i64().unwrap() as i16).to_le_bytes());
            c.entries.push(Entry {
                key: Key {
                    resource: 1,
                    chunk: 128 + i as u16,
                },
                semiglobal: None,
                sprite_resource: 0,
                global: None,
                name: vec![],
                data,
                index: i as u32,
                flags: r["flags"].as_u64().unwrap() as u32,
            });
            c.guids.insert(guid, i);
            prior.push(r["saved_index"].as_i64().unwrap() as i16);
        }
        let records = input["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| Record {
                guid: r["guid"].as_u64().unwrap() as u32,
                saved_index: r["saved_index"].as_i64().unwrap() as i16,
                signatures: (version > 0)
                    .then(|| serde_json::from_value(r["signatures"].clone()).unwrap()),
                kind: (version >= 2).then(|| r["kind"].as_i64().unwrap() as i16),
                name: serde_json::from_value(r["name"].clone()).unwrap(),
                extra: (version >= 3).then(|| r["extra"].as_u64().unwrap() as u32),
            })
            .collect();
        let mut s = State::from_parts(&c, prior, vec![], vec![]).unwrap();
        let mut h = services(Some(version), records);
        h.signatures = serde_json::from_value(resources[0]["signatures"].clone()).unwrap();
        let mut e = Execution::new(65536);
        // Unknown declarations may have negative IDs; they do not participate
        // in the registered-resource GUID lookup's allocation extent.
        finish(&mut e, &mut s, &mut c, &mut h);
        let actual_resources: Vec<_> = c
            .entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                json!({
            "guid":u32::from_le_bytes(entry.data[28..32].try_into().unwrap()),
            "saved_index":s.saved_indices[i],"flags":entry.flags})
            })
            .collect();
        let calls: Vec<_> = h
            .events
            .iter()
            .filter_map(|event| {
                let (which, chunk) = event.split_once(':')?;
                if !matches!(which, "Init" | "Main") {
                    return None;
                }
                let chunk: u16 = chunk.parse().unwrap();
                let entry = c.entries.iter().find(|e| e.key.chunk == chunk).unwrap();
                Some(json!([
                    which,
                    u32::from_le_bytes(entry.data[28..32].try_into().unwrap())
                ]))
            })
            .collect();
        assert_eq!(
            json!({"resources":actual_resources,
            "declarations":s.declarations.iter().map(|d|d.as_ref().unwrap()).collect::<Vec<_>>(),
            "signature_calls":calls}),
            case["original"],
            "original component case {case_number}"
        );
    }
}

#[test]
fn missing_init_keeps_the_record_and_does_not_publish_a_changed_flag_early() {
    let mut c = catalog();
    let mut s = state(&c);
    let mut h = services(Some(3), vec![record(3, 10, 111)]);
    h.block_init = true;
    let mut e = Execution::new(65536);
    let error = loop {
        if let Err(error) = e.step(&mut s, &mut c, &mut h) {
            break error;
        }
    };
    assert_eq!(error, "Init source unavailable");
    assert_eq!(h.cursor, 1);
    assert_eq!(s.saved_indices[0], 111);
    assert_eq!(c.entries[0].flags, 0x800005);
    assert_eq!(s.type_guids, [99, 98]);
    let cold = serde_json::to_vec(&(e, s, c, h)).unwrap();
    let (mut e, mut s, mut c, mut h): (Execution, State, Catalog, Services) =
        serde_json::from_slice(&cold).unwrap();
    h.block_init = false;
    finish(&mut e, &mut s, &mut c, &mut h);
    assert_eq!(
        h.events
            .iter()
            .filter(|v| v.as_str() == "read:Some(10)")
            .count(),
        1
    );
    assert_eq!(c.entries[0].flags, 0x800005);
    assert_eq!(s.type_guids[111], 10);
}

#[test]
fn partial_declaration_teardown_does_not_retire_an_owner_twice_after_cold_restore() {
    let mut c = catalog();
    let mut s = state(&c);
    s.declarations = [77, 88]
        .into_iter()
        .map(|guid| {
            Some(Declaration {
                name: vec![],
                guid,
                extra: 0,
                kind: -1,
                saved_index: 0,
            })
        })
        .collect();
    let mut h = services(None, vec![]);
    h.block_retire_guid = Some(88);
    let mut e = Execution::new(65536);
    loop {
        if e.step(&mut s, &mut c, &mut h).is_err() {
            break;
        }
    }
    assert_eq!(h.events, ["retire:77"]);
    assert_eq!(s.declarations.len(), 2);
    let cold = serde_json::to_vec(&(e, s, c, h)).unwrap();
    let (mut e, mut s, mut c, mut h): (Execution, State, Catalog, Services) =
        serde_json::from_slice(&cold).unwrap();
    h.block_retire_guid = None;
    finish(&mut e, &mut s, &mut c, &mut h);
    assert_eq!(h.events, ["retire:77", "retire:88", "Gtab#1", "objt#0"]);
    assert!(s.declarations.is_empty());
}

#[test]
fn malformed_decoded_record_is_retained_instead_of_reading_past_the_failure() {
    let mut c = catalog();
    let mut s = state(&c);
    let mut wrong = record(3, 10, 4);
    wrong.extra = None;
    let mut h = services(Some(3), vec![wrong, record(3, 11, 7)]);
    let mut e = Execution::new(65536);
    let first = loop {
        if let Err(error) = e.step(&mut s, &mut c, &mut h) {
            break error;
        }
    };
    assert_eq!(h.cursor, 1);
    let calls = h.events.clone();
    assert_eq!(e.step(&mut s, &mut c, &mut h).unwrap_err(), first);
    assert_eq!(h.events, calls);
    assert_eq!(s.saved_indices, [0, 0, 0]);
    assert_eq!(s.type_guids, [99, 98]);
}
