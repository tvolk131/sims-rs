use super::*;
use serde::Serialize;
fn table() -> Table {
    Table {
        rows: vec![
            constants::Row {
                name: b"A".to_vec(),
                value_bits: 0x80000000,
                comment: b"odd".to_vec(),
            },
            constants::Row {
                name: b"a".to_vec(),
                value_bits: 0x7fc12345,
                comment: vec![0xc4, 0x80],
            },
        ],
    }
}
pub(super) fn components(present: bool) -> Components {
    let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1    ".to_vec();
    assert_eq!(bytes.len(), 64);
    bytes[60..].fill(0);
    if present {
        let data = encode(&table()).unwrap();
        let mut header = vec![0; 76];
        header[..4].copy_from_slice(b"FCNS");
        header[4..8].copy_from_slice(&((data.len() + 76) as u32).to_be_bytes());
        header[8..10].copy_from_slice(&2u16.to_be_bytes());
        header[10..12].copy_from_slice(&16u16.to_be_bytes());
        bytes.extend(header);
        bytes.extend(data);
    }
    let mut stream = OwnedStream {
        bytes,
        position: 0,
        open: false,
        writable: false,
    };
    let mut iff = Iff::default();
    assert_eq!(iff.open(b"Global.iff", 0, &mut stream).unwrap(), 0);
    Components {
        entries: BTreeMap::from([(1, Component { iff, stream })]),
        handles: BTreeMap::new(),
        next_handle: 1,
        buffers: BTreeMap::new(),
    }
}
pub(super) fn aggregate() -> Composite {
    let mut a = Composite::default();
    a.slots[0] = Some(1);
    a
}
fn cold<T: Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
}
#[test]
fn raw_codec_preserves_order_duplicates_nonunicode_and_float_bits() {
    let table = table();
    let bytes = encode(&table).unwrap();
    assert_eq!(decode(&bytes).unwrap(), table);
    for n in 0..bytes.len() {
        assert!(decode(&bytes[..n]).is_err(), "{n}");
    }
    let mut unsupported = bytes.clone();
    unsupported[4] = 2;
    assert!(decode(&unsupported).is_err());
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(decode(&extra).is_err());
    let mut bad = table;
    bad.rows[0].name.push(0);
    assert!(encode(&bad).is_err());
}
#[test]
fn readonly_replacement_really_distinguishes_existing_from_missing() {
    for (present, expected) in [(true, -99), (false, -94)] {
        let mut components = components(present);
        let mut a = aggregate();
        let new = components.allocate(encode(&table()).unwrap()).unwrap();
        components.validate().unwrap();
        assert_eq!(cold::<Components>(&components), components);
        a.add(
            composite::Chunk {
                handle: new,
                tag: TAG,
                id: 2,
                name: b"",
                variant: 0,
                converted: 1,
            },
            &mut components,
        )
        .unwrap();
        assert_eq!(a.error, expected);
        assert_eq!(
            components.entries[&1].iff.error,
            if present { 0 } else { -98 }
        );
        components.free(new);
        components.validate().unwrap();
        let decoded: Components = cold(&components);
        assert_eq!(decoded, components);
        decoded.validate().unwrap();
        if present {
            assert_eq!(components.handles.len(), 1);
            assert_eq!(
                components.entries[&1]
                    .iff
                    .records()
                    .filter(|r| r.payload.is_some())
                    .count(),
                1
            );
        }
    }
}
#[test]
fn load_detach_free_and_resident_cold_cuts_keep_exact_graph() {
    let mut c = components(true);
    let mut a = aggregate();
    let h = a.by_id(TAG, 2, 0, CALLBACK, &mut c).unwrap();
    assert!(h != 0);
    assert_eq!(decode(c.data(h).unwrap()).unwrap(), table());
    c.validate().unwrap();
    let resident: Components = cold(&c);
    assert_eq!(resident, c);
    a.release_chunk(h, &mut c).unwrap();
    assert_eq!(a.error, 0);
    assert!(c.buffers.contains_key(&h));
    c.validate().unwrap();
    assert_eq!(cold::<Components>(&c), c);
    c.free(h);
    c.validate().unwrap();
    assert_eq!(cold::<Components>(&c), c);
    let second = a.by_id(TAG, 2, 0, CALLBACK, &mut c).unwrap();
    assert!(second > h);
    assert_eq!(decode(c.data(second).unwrap()).unwrap(), table());
}
#[test]
fn forged_full_iff_buckets_records_and_handle_aliases_fail_closed() {
    let c = components(true);
    let mut value = serde_json::to_value(&c).unwrap();
    value["entries"]["1"]["iff"]["table"]["len"] = 0.into();
    let bad: Components = serde_json::from_value(value).unwrap();
    assert!(bad.validate().is_err());
    let mut value = serde_json::to_value(&c).unwrap();
    let buckets = value["entries"]["1"]["iff"]["table"]["buckets"]
        .as_array_mut()
        .unwrap();
    let entry = buckets
        .iter_mut()
        .find(|v| !v.as_array().unwrap().is_empty())
        .unwrap();
    entry[0][1]["buckets"] = serde_json::json!([]);
    let bad: Components = serde_json::from_value(value).unwrap();
    assert!(bad.validate().is_err());
    let mut resident = c;
    let mut a = aggregate();
    let h = a.by_id(TAG, 2, 0, CALLBACK, &mut resident).unwrap();
    let mut bad = resident.clone();
    bad.handles.insert(h + 1, (1, 1));
    bad.next_handle += 1;
    assert!(bad.validate().is_err());
    let mut bad = resident.clone();
    bad.handles.clear();
    assert!(bad.validate().is_err());
    let mut bad = resident.clone();
    bad.buffers.insert(h, vec![]);
    assert!(bad.validate().is_err());
    let mut value = serde_json::to_value(&resident).unwrap();
    value["entries"]["1"]["iff"]["storage"][0]["payload"]["handle"] = 0.into();
    let bad: Components = serde_json::from_value(value).unwrap();
    assert!(bad.validate().is_err());
}
#[test]
fn unsupported_callback_and_failing_decode_keep_actual_partial_resident_state() {
    let mut c = components(true);
    let mut a = aggregate();
    // No callback is required for the actual bit16 source. An unknown callback
    // is rejected before loading and does not mutate the graph.
    let before = c.clone();
    assert!(a.by_id(TAG, 2, 0, 0x12345, &mut c).is_err());
    assert_eq!(c, before);
    let h = a.by_id(TAG, 2, 0, CALLBACK, &mut c).unwrap();
    let local = c.handles[&h].1;
    c.entries
        .get_mut(&1)
        .unwrap()
        .iff
        .payload_mut(local)
        .unwrap()[4] = 2;
    assert!(decode(c.data(h).unwrap()).is_err());
    assert!(c.entries[&1].iff.payload(local).is_some());
    c.validate().unwrap();
}
#[cfg(feature = "private-fixtures")]
#[test]
#[ignore = "private immutable Global.far; exact actual initial process/resource proof"]
fn actual_asset_producer_services_and_every_cold_cut() {
    let root = std::path::PathBuf::from(std::env::var_os("SIMS_FCNS_ASSETS").unwrap());
    let index = Index::scan(&root).unwrap();
    let resource = index
        .resources
        .iter()
        .find(|r| r.name == "Global.iff")
        .unwrap();
    let input = FreshInput::observed_initial(&index, &resource.locator).unwrap();
    let mut owner = input.owner;
    let mut p = input.provider;
    assert_eq!(owner.process.loaded, Some(0));
    assert_eq!(owner.friendship_threshold().unwrap(), 25);
    let mut call = owner.begin();
    let mut writes = vec![];
    let mut world = sims_vm::World::default();
    world.autonomy_constants = Some(owner.clone());
    for _ in 0..100 {
        p = cold(&p);
        p.validate(&index).unwrap();
        call = cold(&call);
        world = cold(&world);
        match constants::advance(&mut world, &mut call).unwrap() {
            constants::Progress::Store(store) => writes.push(store),
            constants::Progress::Service(request) => {
                if matches!(request, Request::Flush(_)) {
                    assert_eq!(hash(&encode(call.table().unwrap()).unwrap()), PAYLOAD);
                }
                let reply = p.service(&request, call.table()).unwrap();
                call.reply(reply).unwrap();
            }
            constants::Progress::Complete => break,
        }
    }
    owner = world.autonomy_constants.unwrap();
    assert!(call.complete());
    let observed: serde_json::Value =
        serde_json::from_str(include_str!("expected-final-process.json")).unwrap();
    assert_eq!(
        serde_json::to_value(&owner.process).unwrap(),
        observed["process"]
    );
    assert_eq!(writes.len(), 13);
    assert_eq!(owner.process.loaded, Some(1));
    assert_eq!(owner.friendship_threshold().unwrap(), 50);
    assert_eq!(owner.process.words[9], Some(4));
    assert_eq!(
        p.events
            .iter()
            .map(|e| e.operation.as_str())
            .collect::<Vec<_>>(),
        ["Load.Lookup", "Load.Detach", "Flush.Serialize", "Flush.Add"]
    );
    assert_eq!(p.events.last().unwrap().return_value, -99);
    assert_eq!(p.events.last().unwrap().aggregate_error, -99);
    assert_eq!(p.events.last().unwrap().child_error, 0);
    p.validate(&index).unwrap();
    assert_eq!(p.components.buffers.len(), 0);
    assert_eq!(p.components.handles.len(), 1);
    let count = p.events.len();
    owner.configuration = Default::default();
    let mut call = owner.begin();
    world = sims_vm::World::default();
    world.autonomy_constants = Some(owner);
    assert_eq!(
        constants::advance(&mut world, &mut call).unwrap(),
        constants::Progress::Complete
    );
    assert_eq!(p.events.len(), count);
}

#[path = "registered_child.rs"]
mod registered_child;
