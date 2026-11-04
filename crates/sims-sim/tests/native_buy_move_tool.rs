use sims_sim::object_catalog::{
    Key,
    definition::{Catalog, Entry},
    move_tool::{Activation, Fields, Groups},
};

fn add(c: &mut Catalog, guid: u32, group: i16, excluded: i16, flag: i16) -> u32 {
    let mut data = vec![0; 216];
    data[0x1c..0x20].copy_from_slice(&guid.to_le_bytes());
    data[0x4e..0x50].copy_from_slice(&excluded.to_le_bytes());
    data[0x8a..0x8c].copy_from_slice(&group.to_le_bytes());
    data[0xc0..0xc2].copy_from_slice(&flag.to_le_bytes());
    let row = c.entries.len();
    c.entries.push(Entry {
        key: Key {
            resource: 1,
            chunk: row as u16,
        },
        semiglobal: None,
        sprite_resource: 1,
        global: None,
        name: vec![],
        data,
        index: row as u32,
        flags: 0,
    });
    c.guids.insert(guid, row);
    row as u32 + 1
}
fn activate(f: &mut Fields, g: &Groups, c: &Catalog, arg: u32, lot: i16) {
    let mut call = Activation::new(arg);
    assert!(call.advance(f, g, c, lot).unwrap());
    assert_eq!(f.argument, i32::from(arg as i16));
    let before = f.clone();
    call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
    assert!(call.advance(f, g, c, lot).unwrap());
    assert_eq!(f, &before);
    // Concrete487191 resets+38; activation must not restore its old write.
    // The rest of this callback is deliberately supplied, not claimed tested.
    f.argument = -1;
    call.acknowledge_deactivation().unwrap();
    call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
    assert!(!call.advance(f, g, c, lot).unwrap());
    assert!(call.complete());
    assert_eq!(f.argument, -1);
    let after = f.clone();
    assert!(!call.advance(f, g, c, lot).unwrap());
    assert_eq!(f, &after);
}

#[test]
fn groups_keep_registration_order_and_resolve_latest_guid_binding() {
    let mut c = Catalog::default();
    add(&mut c, 10, 1, 0, 0);
    add(&mut c, 20, 1, 0, 0);
    add(&mut c, 30, 2, 1, 0);
    let latest = add(&mut c, 10, 7, 0, 0);
    let mut g = Groups::default();
    g.populate(&c).unwrap();
    assert_eq!(g.0[0].guids, [10, 20]);
    assert!(g.0[1].guids.is_empty());
    assert_eq!(g.0[6].guids, [10]);
    assert_eq!(g.lookup(0, -20000, &c).unwrap(), Some(latest));
    let original = g.clone();
    g.populate(&c).unwrap();
    assert_eq!(g, original);
    c.guids.remove(&10);
    assert_eq!(g.lookup(0, -20000, &c).unwrap(), None);
}

#[test]
fn empty_first_group_repeats_append_and_lookup_ignores_nominal_upper_endpoint() {
    let mut c = Catalog::default();
    let id = add(&mut c, 20, 2, 0, 0);
    let mut g = Groups::default();
    g.populate(&c).unwrap();
    g.populate(&c).unwrap();
    assert_eq!(g.0[1].guids, [20, 20]);
    g.0[1].last = -18000;
    assert_eq!(g.lookup(1, -17999, &c).unwrap(), Some(id));
    g.0[1].first = i32::MAX;
    assert_eq!(g.lookup(1, i32::MIN, &c).unwrap(), Some(id));
    assert_eq!(g.lookup(1, i32::MAX - 1, &c).unwrap(), None);
}

#[test]
fn each_original_group_drives_its_distinct_move_mode_and_flags() {
    let mut c = Catalog::default();
    for group in 1..=7 {
        add(&mut c, group as u32, group, 0, 0);
    }
    let mut g = Groups::default();
    g.populate(&c).unwrap();
    for group in 0..7 {
        let mut f = Fields::constructor_fields();
        f.position = [9, -4];
        activate(&mut f, &g, &c, g.0[group].first as u32, 0);
        assert_eq!(f.definition, Some(group as u32 + 1));
        assert_eq!(f.mode, group as i32 + 3);
        assert_eq!(f.cursor, 7);
        assert_eq!(f.special, u8::from(group < 2));
        assert_eq!(f.subtype, u8::from(group == 3));
        assert_eq!(f.flags, [1, 1, 1, u8::from(group != 3), 0]);
        assert_eq!(f.position, [0; 2]);
    }
}

#[test]
fn normal_indices_equal_identity_and_clear_have_different_cursor_writes() {
    let mut c = Catalog::default();
    add(&mut c, 10, 0, 0, 1);
    let g = Groups::default();
    let mut f = Fields::constructor_fields();
    activate(&mut f, &g, &c, 65536, 0); // original low word is zero
    assert_eq!((f.mode, f.cursor, f.definition), (2, 6, Some(1)));
    assert_eq!(f.flags, [1, 1, 0, 1, 0]);
    f.cursor = 123;
    activate(&mut f, &g, &c, 0, 0); // equal identity skips OBJD+ c0 / +8a
    assert_eq!(f.cursor, 123);
    assert_eq!(f.flags, [1, 0, 0, 1, 0]);
    activate(&mut f, &g, &c, 0x1234ffff, 0);
    assert_eq!((f.cursor, f.definition, f.mode), (123, None, 1));
    f.definition = Some(1);
    activate(&mut f, &g, &c, 50, 0); // missing normal row clears cursor
    assert_eq!((f.cursor, f.definition, f.mode), (6, None, 1));
    activate(&mut f, &g, &c, 0, 1);
    assert_eq!(f.flags, [1, 0, 0, 1, 0]);
}

#[test]
fn malformed_definition_fails_at_access_without_replacing_a_partial_list() {
    let mut c = Catalog::default();
    add(&mut c, 10, 1, 0, 0);
    add(&mut c, 20, 2, 0, 0);
    c.entries[1].data.truncate(10);
    let mut g = Groups::default();
    assert!(g.populate(&c).is_err());
    assert_eq!(g.0[0].guids, [10]);
    assert!(g.0[1].guids.is_empty());
}

fn case(c: &serde_json::Value, restore: bool) -> serde_json::Value {
    use serde_json::json;
    use sims_vm::interface::{
        buy::tool::{Reply, Request, Selection},
        state::State,
        tool,
    };
    match c["kind"].as_str().unwrap() {
        "lookup" => {
            let mut slots = std::array::from_fn(|n| Some(n as u32 + 100));
            if c["empty"] == true {
                slots[7] = None;
            }
            json!(tool::lookup(&slots, c["index"].as_i64().unwrap() as i32))
        }
        "select" => {
            let mut s = State::constructed(760);
            s.buy_tool = 91;
            s.buy.selected_object = c["selected"].as_u64().unwrap() as u32;
            let mut call = Selection::new(c["selector"].as_i64().unwrap() as i32);
            let mut events = vec![];
            while let Some(request) = call.advance(&mut s).unwrap() {
                let f = json!({"buy_tool":s.buy_tool,"selected":s.buy.selected_object});
                events.push(json!({"request":request,"fields":f}));
                if restore {
                    (call, s) =
                        serde_json::from_slice(&serde_json::to_vec(&(&call, &s)).unwrap()).unwrap();
                    let before = s.clone();
                    assert_eq!(call.advance(&mut s).unwrap(), Some(request));
                    assert_eq!(s, before);
                }
                let reply = if matches!(request, Request::Guid(_)) {
                    Reply::Guid(c["guid"].as_u64().unwrap() as u32)
                } else {
                    Reply::Done
                };
                call.acknowledge(request, reply).unwrap();
                if restore {
                    call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
                }
                assert!(events.len() <= 2);
            }
            assert!(call.complete());
            json!({"events":events,"fields":{"buy_tool":s.buy_tool,"selected":s.buy.selected_object}})
        }
        "activate" => {
            let mut catalog = Catalog::default();
            for row in c["catalog"].as_array().unwrap() {
                add(
                    &mut catalog,
                    row["guid"].as_u64().unwrap() as u32,
                    row["group"].as_i64().unwrap() as i16,
                    0,
                    row["flag"].as_i64().unwrap() as i16,
                );
            }
            let mut groups = Groups::default();
            for (n, guids) in c["groups"].as_array().unwrap().iter().enumerate() {
                groups.0[n].guids = serde_json::from_value(guids.clone()).unwrap();
            }
            let mut f = Fields {
                cursor: 91,
                special: 255,
                mode: 88,
                argument: 77,
                definition: serde_json::from_value(c["definition"].clone()).unwrap(),
                position: [42, -5],
                flags: [9, 10, 11, 12, 13],
                subtype: 254,
            };
            let mut call = Activation::new(c["argument"].as_u64().unwrap() as u32);
            let lot = c["lot"].as_i64().unwrap() as i16;
            assert!(call.advance(&mut f, &groups, &catalog, lot).unwrap());
            let events = vec![f.clone()];
            if restore {
                (call, f, groups) =
                    serde_json::from_slice(&serde_json::to_vec(&(&call, &f, &groups)).unwrap())
                        .unwrap();
                let before = f.clone();
                assert!(call.advance(&mut f, &groups, &catalog, lot).unwrap());
                assert_eq!(f, before);
            }
            f.argument = -1;
            let mut fields = serde_json::to_value(&f).unwrap();
            for (k, v) in c["patch"].as_object().unwrap() {
                fields[k] = v.clone();
            }
            f = serde_json::from_value(fields).unwrap();
            call.acknowledge_deactivation().unwrap();
            if restore {
                (call, f, groups) =
                    serde_json::from_slice(&serde_json::to_vec(&(&call, &f, &groups)).unwrap())
                        .unwrap();
            }
            assert!(!call.advance(&mut f, &groups, &catalog, lot).unwrap());
            assert!(call.complete());
            json!({"events":events,"fields":f})
        }
        _ => panic!("unknown case"),
    }
}

#[test]
#[ignore = "prepared private input corpus; not an original-instruction comparison"]
fn prepared_buy_tool_cases_restore_at_each_child_boundary() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/native-buy-tool-inputs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(corpus["native_execution"], false);
    for (n, c) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(case(c, false), case(c, true), "case{n}");
    }
}

#[test]
#[ignore = "original x86 comparison; run native generator only without any Cargo/rustc processes"]
fn buy_tools_match_original_instruction_corpus() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("cache/research/sims-exe/oracle/buy-tool.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(corpus["native_execution"], true);
    for (n, c) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        assert_eq!(case(&c["input"], false), c["original"], "case{n}");
        assert_eq!(case(&c["input"], true), c["original"], "restored case{n}");
    }
}
