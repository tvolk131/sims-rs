use serde::{Deserialize, Serialize};
use sims_vm::scheduler::{Inputs, Pass, Registry, Reply, Request};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Table {
    objects: Vec<Option<u32>>,
    words: Vec<u32>,
    errors: BTreeMap<u32, i16>,
}
impl Table {
    fn new(words: &[u32]) -> Self {
        Self {
            objects: (1..=words.len()).map(|i| Some(i as u32)).collect(),
            words: words.to_vec(),
            errors: (1..=words.len()).map(|i| (i as u32, 0)).collect(),
        }
    }
    fn index(&self, slot: i32) -> Result<usize, String> {
        usize::try_from(slot.wrapping_sub(1))
            .ok()
            .filter(|&i| i < self.len())
            .ok_or_else(|| "registry slot absent".into())
    }
}
impl Registry for Table {
    fn len(&self) -> usize {
        self.objects.len()
    }
    fn object(&self, slot: i32) -> Result<Option<u32>, String> {
        Ok(self.objects[self.index(slot)?])
    }
    fn word(&self, slot: i32) -> Result<u32, String> {
        Ok(self.words[self.index(slot)?])
    }
    fn set_word(&mut self, slot: i32, word: u32) -> Result<(), String> {
        let index = self.index(slot)?;
        self.words[index] = word;
        Ok(())
    }
    fn error(&self, object: u32) -> Result<i16, String> {
        self.errors
            .get(&object)
            .copied()
            .ok_or_else(|| "object error word absent".into())
    }
}
fn inputs() -> Inputs {
    Inputs {
        paused: 0,
        mode: 0,
        lot_number: 1,
        tick: 194316,
    }
}
fn restore(pass: &mut Pass) {
    let text = serde_json::to_string(pass).unwrap();
    let restored = serde_json::from_str::<Pass>(&text).unwrap();
    restored.validate().unwrap();
    assert_eq!(*pass, restored);
    *pass = restored;
}
fn finish(pass: &mut Pass, table: &mut Table, cursor: &mut i32, input: Inputs) -> Vec<Request> {
    let mut events = vec![];
    while let Some(request) = pass.advance(table, cursor, input).unwrap() {
        let before = table.clone();
        let prior_cursor = *cursor;
        restore(pass);
        assert_eq!(
            pass.advance(table, cursor, input).unwrap(),
            Some(request.clone())
        );
        assert_eq!(*table, before);
        assert_eq!(*cursor, prior_cursor);
        pass.acknowledge(
            &request,
            match request {
                Request::Tick { .. } => Reply::Tick(1),
                Request::ObjectError { .. } => Reply::Done,
            },
        )
        .unwrap();
        events.push(request);
    }
    restore(pass);
    events
}

#[test]
fn pause_mode_and_lot_words_are_not_truncated_to_bytes() {
    for paused in [0, 1, 256, -256, i16::MIN] {
        for mode in [0, 1, 2, 256, -1] {
            for lot_number in [0, 1, 256, -256] {
                let input = Inputs {
                    paused,
                    mode,
                    lot_number,
                    ..inputs()
                };
                let mut table = Table::new(&[0x1ffff, 0x3ffff]);
                let mut cursor = 0;
                let mut pass = Pass::begin(input, &table, &mut cursor).unwrap();
                let partial = paused != 0 || (mode != 0 && lot_number != 0);
                let events = finish(&mut pass, &mut table, &mut cursor, input);
                assert_eq!(events.len(), if partial { 1 } else { 2 });
                assert_eq!(
                    events.last(),
                    Some(&Request::Tick {
                        object: 2,
                        tick: input.tick
                    })
                );
                assert_eq!(pass.result(), Some(!partial));
                assert_eq!(cursor, 0);
            }
        }
    }
}

#[test]
fn only_eligible_entries_decrement_the_signed_countdown() {
    for partial in [false, true] {
        for flags in 0..16u32 {
            for countdown in [i16::MIN, -2, -1, 0, 1, 2, i16::MAX] {
                let input = Inputs {
                    paused: i16::from(partial),
                    ..inputs()
                };
                let word = flags << 16 | countdown as u16 as u32;
                let mut table = Table::new(&[word]);
                let mut cursor = 0;
                let mut pass = Pass::begin(input, &table, &mut cursor).unwrap();
                let events = finish(&mut pass, &mut table, &mut cursor, input);
                let eligible = flags & 1 != 0 && (!partial || flags & 2 != 0);
                let expected_count = if eligible && countdown > 0 {
                    countdown - 1
                } else {
                    countdown
                };
                assert_eq!(
                    table.words[0],
                    ((flags & !4) << 16) | expected_count as u16 as u32
                );
                assert_eq!(events.len(), usize::from(eligible && countdown <= 0));
            }
        }
    }
}

#[test]
fn holes_preserve_words_and_disabled_errors_are_still_serviced() {
    let input = Inputs {
        paused: 1,
        ..inputs()
    };
    let mut table = Table::new(&[0x7ffff, 0x4ffff, 0x3ffff]);
    table.objects[0] = None;
    table.errors.insert(2, -7);
    let before = table.words.clone();
    let mut cursor = 0;
    let mut pass = Pass::begin(input, &table, &mut cursor).unwrap();
    assert_eq!(
        finish(&mut pass, &mut table, &mut cursor, input),
        vec![Request::ObjectError { object: 2 }]
    );
    assert_eq!(table.words, before);
    assert_eq!(cursor, 2);
    assert_eq!(pass.result(), Some(false));
}

#[test]
fn native_false_return_retains_cursor_and_reentry_rereads_inputs_and_length() {
    let mut table = Table::new(&[0x1ffff, 0x1ffff]);
    let mut cursor = 0;
    let mut pass = Pass::begin(inputs(), &table, &mut cursor).unwrap();
    let request = pass
        .advance(&mut table, &mut cursor, inputs())
        .unwrap()
        .unwrap();
    table.errors.insert(1, 12);
    pass.acknowledge(&request, Reply::Tick(0)).unwrap();
    assert!(finish(&mut pass, &mut table, &mut cursor, inputs()).is_empty());
    assert_eq!(cursor, 1); //False Tick bypasses even its now-nonzero error.
    assert_eq!(pass.result(), Some(false));
    let input = Inputs {
        mode: 2,
        ..inputs()
    };
    table.objects.push(Some(3));
    table.words.push(0x3ffff);
    table.errors.insert(3, 0);
    let mut next = Pass::begin(input, &table, &mut cursor).unwrap();
    assert_eq!(
        finish(&mut next, &mut table, &mut cursor, input),
        vec![Request::Tick {
            object: 3,
            tick: input.tick
        }]
    );
    assert_eq!(next.result(), Some(false));
    assert_eq!(cursor, 0);
}

#[test]
fn callback_growth_and_mode_changes_do_not_rewrite_captured_pass_gates() {
    let mut table = Table::new(&[0x1ffff, 0x1ffff]);
    let mut cursor = 0;
    let mut pass = Pass::begin(inputs(), &table, &mut cursor).unwrap();
    let first = pass
        .advance(&mut table, &mut cursor, inputs())
        .unwrap()
        .unwrap();
    table.objects.push(Some(3));
    table.words.push(0x1ffff);
    table.errors.insert(3, 0);
    pass.acknowledge(&first, Reply::Tick(255)).unwrap();
    restore(&mut pass);
    let changed = Inputs {
        paused: 1,
        tick: 55,
        ..inputs()
    };
    assert_eq!(
        finish(&mut pass, &mut table, &mut cursor, changed),
        vec![Request::Tick {
            object: 2,
            tick: 55
        }]
    );
    assert_eq!(pass.result(), Some(true));
}

#[test]
fn later_slots_are_live_and_post_tick_error_uses_retained_object() {
    let mut table = Table::new(&[0x1ffff, 0x1ffff]);
    let mut cursor = 0;
    let mut pass = Pass::begin(inputs(), &table, &mut cursor).unwrap();
    let first = pass
        .advance(&mut table, &mut cursor, inputs())
        .unwrap()
        .unwrap();
    table.objects[0] = Some(9);
    table.errors.insert(9, 0);
    table.errors.insert(1, -1);
    cursor = 2;
    pass.acknowledge(&first, Reply::Tick(1)).unwrap();
    assert_eq!(
        finish(&mut pass, &mut table, &mut cursor, inputs()),
        vec![Request::ObjectError { object: 1 }]
    );
    assert_eq!(cursor, 2);
    let mut next = Pass::begin(inputs(), &table, &mut cursor).unwrap();
    table.objects[1] = Some(7);
    table.errors.insert(7, 0);
    assert_eq!(
        finish(&mut next, &mut table, &mut cursor, inputs()),
        vec![Request::Tick {
            object: 7,
            tick: inputs().tick
        }]
    );
}

#[test]
fn callback_cursor_changes_control_the_next_slot() {
    let mut table = Table::new(&[0x1ffff, 0x1ffff, 0x1ffff]);
    let mut cursor = 0;
    let mut pass = Pass::begin(inputs(), &table, &mut cursor).unwrap();
    let request = pass
        .advance(&mut table, &mut cursor, inputs())
        .unwrap()
        .unwrap();
    cursor = 2;
    pass.acknowledge(&request, Reply::Tick(1)).unwrap();
    assert_eq!(
        finish(&mut pass, &mut table, &mut cursor, inputs()),
        vec![Request::Tick {
            object: 3,
            tick: inputs().tick
        }]
    );
}

#[test]
fn repeated_paused_passes_visit_controllers_at_the_same_lot_tick() {
    let input = Inputs {
        mode: 2,
        ..inputs()
    };
    let mut table = Table::new(&[0x1ffff, 0x30002]);
    let mut cursor = 0;
    for iteration in 0..6 {
        let mut pass = Pass::begin(input, &table, &mut cursor).unwrap();
        let events = finish(&mut pass, &mut table, &mut cursor, input);
        assert_eq!(
            events,
            if iteration < 2 {
                vec![]
            } else {
                vec![Request::Tick {
                    object: 2,
                    tick: input.tick,
                }]
            }
        );
        assert_eq!(table.words[0], 0x1ffff);
        assert_eq!(pass.result(), Some(false));
    }
}

#[test]
fn failed_host_word_write_can_be_retried_without_skipping_the_entry() {
    struct FailingWrite {
        table: Table,
        fail: bool,
    }
    impl Registry for FailingWrite {
        fn len(&self) -> usize {
            self.table.len()
        }
        fn object(&self, slot: i32) -> Result<Option<u32>, String> {
            self.table.object(slot)
        }
        fn word(&self, slot: i32) -> Result<u32, String> {
            self.table.word(slot)
        }
        fn error(&self, object: u32) -> Result<i16, String> {
            self.table.error(object)
        }
        fn set_word(&mut self, slot: i32, word: u32) -> Result<(), String> {
            if std::mem::take(&mut self.fail) {
                Err("temporary host write failure".into())
            } else {
                self.table.set_word(slot, word)
            }
        }
    }
    for word in [0x10002, 0x50002] {
        let mut registry = FailingWrite {
            table: Table::new(&[word]),
            fail: true,
        };
        let mut cursor = 0;
        let mut pass = Pass::begin(inputs(), &registry, &mut cursor).unwrap();
        assert!(pass.advance(&mut registry, &mut cursor, inputs()).is_err());
        restore(&mut pass);
        assert_eq!(cursor, 1);
        assert_eq!(registry.table.words, vec![word]);
        assert_eq!(
            pass.advance(&mut registry, &mut cursor, inputs()).unwrap(),
            None
        );
        assert_eq!(registry.table.words, vec![0x10001]);
        assert_eq!(pass.result(), Some(true));
    }
}

#[test]
fn errors_do_not_consume_an_acknowledgement_and_empty_pass_resets_cursor() {
    let mut table = Table::new(&[0x1ffff]);
    let mut cursor = 0;
    let mut pass = Pass::begin(inputs(), &table, &mut cursor).unwrap();
    let request = pass
        .advance(&mut table, &mut cursor, inputs())
        .unwrap()
        .unwrap();
    let before = pass.clone();
    assert!(pass.acknowledge(&request, Reply::Done).is_err());
    assert_eq!(pass, before);
    assert!(
        pass.acknowledge(&Request::ObjectError { object: 1 }, Reply::Done)
            .is_err()
    );
    assert_eq!(pass, before);
    let mut value = serde_json::to_value(&pass).unwrap();
    value["object"] = serde_json::json!(99);
    assert!(
        serde_json::from_value::<Pass>(value)
            .unwrap()
            .validate()
            .is_err()
    );
    for paused in [0, 1] {
        let mut empty = Table::new(&[]);
        let input = Inputs { paused, ..inputs() };
        let mut cursor = 5;
        let mut pass = Pass::begin(input, &empty, &mut cursor).unwrap();
        assert!(finish(&mut pass, &mut empty, &mut cursor, input).is_empty());
        assert_eq!(cursor, 0);
        assert_eq!(pass.result(), Some(paused == 0));
    }
}

#[derive(Debug, Deserialize)]
struct Case {
    input: Inputs,
    cursor: i32,
    table: Table,
    actions: Vec<serde_json::Value>,
}
fn replay(case: &Case, saving: bool) -> serde_json::Value {
    let mut table = case.table.clone();
    let mut input = case.input;
    let mut cursor = case.cursor;
    let mut pass = Pass::begin(input, &table, &mut cursor).unwrap();
    let mut events = vec![];
    while let Some(request) = pass.advance(&mut table, &mut cursor, input).unwrap() {
        assert!(
            events.len() < 100,
            "unexpected nonterminating callback sequence"
        );
        events.push(serde_json::json!({ "request": request, "cursor": cursor, "table": table }));
        if saving {
            restore(&mut pass);
            let saved = serde_json::to_string(&(input, cursor, &table)).unwrap();
            (input, cursor, table) = serde_json::from_str(&saved).unwrap();
            assert_eq!(
                pass.advance(&mut table, &mut cursor, input).unwrap(),
                Some(request.clone())
            );
        }
        let action = case
            .actions
            .get(events.len() - 1)
            .cloned()
            .unwrap_or_default();
        let m = &action["mutation"];
        if let Some(value) = m["cursor"].as_i64() {
            cursor = value as i32;
        }
        if let Some(changes) = m["input"].as_object() {
            let mut value = serde_json::to_value(input).unwrap();
            for (k, v) in changes {
                value[k] = v.clone();
            }
            input = serde_json::from_value(value).unwrap();
        }
        if let Some(changes) = m["words"].as_object() {
            for (k, v) in changes {
                table.words[k.parse::<usize>().unwrap()] = v.as_u64().unwrap() as u32;
            }
        }
        if let Some(changes) = m["objects"].as_object() {
            for (k, v) in changes {
                table.objects[k.parse::<usize>().unwrap()] = v.as_u64().map(|v| v as u32);
            }
        }
        for key in ["errors", "extra_errors"] {
            if let Some(changes) = m[key].as_object() {
                for (k, v) in changes {
                    table
                        .errors
                        .insert(k.parse().unwrap(), v.as_i64().unwrap() as i16);
                }
            }
        }
        if let Some(item) = m["append"].as_object() {
            let object = item["object"].as_u64().unwrap() as u32;
            table.objects.push(Some(object));
            table.words.push(item["word"].as_u64().unwrap() as u32);
            table
                .errors
                .insert(object, item["error"].as_i64().unwrap() as i16);
        }
        pass.acknowledge(
            &request,
            match request {
                Request::Tick { .. } => Reply::Tick(action["returned"].as_u64().unwrap_or(1) as u8),
                Request::ObjectError { .. } => Reply::Done,
            },
        )
        .unwrap();
    }
    serde_json::json!({ "events": events, "cursor": cursor, "table": table, "result": pass.result().unwrap() })
}

#[test]
#[ignore = "private standard-library-only input corpus; no native execution"]
fn prepared_scheduler_cases_restore_at_every_service() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../cache/native-partial-scheduler-inputs.json"
    );
    let corpus: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(corpus["original_execution"], false);
    let rows = corpus["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 506);
    for row in rows {
        let case: Case = serde_json::from_value(row.clone()).unwrap();
        assert_eq!(replay(&case, false), replay(&case, true), "{row}");
    }
}

#[test]
#[ignore = "requires guarded native_scheduler.py corpus; never synthesize native expectations"]
fn scheduler_matches_original_instructions() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../cache/research/sims-exe/oracle/scheduler.json"
    );
    let corpus: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        corpus["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        corpus["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = corpus["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 506);
    for row in rows {
        let case: Case = serde_json::from_value(row.clone()).unwrap();
        assert_eq!(replay(&case, true), row["original"], "{row}");
    }
}

#[test]
#[ignore = "reads the retained private native_refresh_idle.py corpus; no emulator execution"]
fn scheduler_matches_retained_original_countdown_vectors() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../cache/research/sims-exe/oracle/refresh-idle.json"
    );
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = report["scheduler"].as_array().unwrap();
    assert_eq!(rows.len(), 49);
    for row in rows {
        let words: Vec<_> = row["input"]["countdowns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| 0x10000 | v.as_i64().unwrap() as i16 as u16 as u32)
            .collect();
        let mut table = Table::new(&words);
        let mut cursor = 0;
        let mut pass = Pass::begin(inputs(), &table, &mut cursor).unwrap();
        let events = finish(&mut pass, &mut table, &mut cursor, inputs());
        let updated: Vec<_> = events
            .into_iter()
            .map(|r| match r {
                Request::Tick { object, .. } => object,
                _ => panic!("unexpected native error request"),
            })
            .collect();
        assert_eq!(serde_json::json!(updated), row["original"]["updated"]);
        let countdowns: Vec<_> = table.words.iter().map(|&w| w as i16).collect();
        assert_eq!(serde_json::json!(countdowns), row["original"]["countdowns"]);
        assert_eq!(row["original"]["error"], serde_json::Value::Null);
        assert_eq!(pass.result(), Some(true));

        // Exercise the production borrowed adapter against the same captured
        // original outputs. Native Tick/error children were intercepted in
        // this corpus; this is not an original whole-VM execution comparison.
        use sims_vm::{
            Object, Vm, World,
            scheduler::{registry::Borrowed, storage::Storage},
        };
        let mut world = World {
            scheduler_storage: Some(Storage::default()),
            ..Default::default()
        };
        for (i, &word) in words.iter().enumerate() {
            let id = (i + 1) as i16;
            let mut object = Object {
                native_tree_error: Some(0),
                ..Default::default()
            };
            world.register_scheduler_object(id, &mut object).unwrap();
            object.idle = Some(sims_vm::idle::IdleState::from_registry_word(word));
            world.objects.insert(id, object);
        }
        let mut vm = Vm::new(world, 1);
        let mut cursor = 0;
        let mut pass =
            Pass::begin(inputs(), &Borrowed::new(&mut vm).unwrap(), &mut cursor).unwrap();
        let mut updated = vec![];
        while let Some(request) = pass
            .advance(&mut Borrowed::new(&mut vm).unwrap(), &mut cursor, inputs())
            .unwrap()
        {
            let Request::Tick { object, .. } = request else {
                panic!("unexpected native error");
            };
            updated.push(Borrowed::new(&mut vm).unwrap().object_id(object).unwrap());
            let saved = serde_json::to_string(&(&vm, &pass, cursor)).unwrap();
            (vm, pass, cursor) = serde_json::from_str(&saved).unwrap();
            pass.acknowledge(&request, Reply::Tick(1)).unwrap();
        }
        assert_eq!(serde_json::json!(updated), row["original"]["updated"]);
        let counts: Vec<_> = vm
            .world
            .objects
            .values()
            .map(|o| o.idle.as_ref().unwrap().countdown)
            .collect();
        assert_eq!(serde_json::json!(counts), row["original"]["countdowns"]);
        assert_eq!(pass.result(), Some(true));
        assert_eq!(cursor, 0);
    }
}
