use serde_json::{Value, json};
use sims_vm::commander::{Broadcast, Registry};

fn dispatch(
    registry: &mut Registry<u32>,
    c: &Value,
    event: u32,
    value: u32,
    depth: usize,
    calls: &mut usize,
    events: &mut Vec<Value>,
    freed: &mut Vec<u32>,
) -> u16 {
    events.push(json!({"enter":{"event":event,"value":value},"depth":depth,"state":registry}));
    let mut cursor = Broadcast::begin(registry, event, value).unwrap();
    loop {
        (*registry, cursor) =
            serde_json::from_slice(&serde_json::to_vec(&(&registry, &cursor)).unwrap()).unwrap();
        let Some(request) = cursor.advance(registry).unwrap().cloned() else {
            break;
        };
        let before = cursor.clone();
        assert_eq!(cursor.advance(registry).unwrap(), Some(&request));
        assert_eq!(cursor, before);
        let mut wrong = request.clone();
        wrong.value = wrong.value.wrapping_add(1);
        assert!(cursor.acknowledge(&wrong, 1).is_err());
        assert_eq!(cursor, before);
        events.push(json!({"invoke":request,"depth":depth,"state":registry}));
        if request.handler != 0 {
            let first = *calls == 0;
            *calls += 1;
            if first {
                for a in c["actions"].as_array().unwrap() {
                    let (kind, v) = a.as_object().unwrap().iter().next().unwrap();
                    let id = v["id"].as_u64().unwrap_or(0) as u32;
                    match kind.as_str() {
                        "Register" => {
                            registry.construct(id).unwrap();
                            registry
                                .bind(id, v["handler"].as_u64().unwrap() as u32)
                                .unwrap();
                        }
                        "Unlink" => registry.unlink(id).unwrap(),
                        "Bind" => registry
                            .bind(id, v["handler"].as_u64().unwrap() as u32)
                            .unwrap(),
                        "Destroy" => {
                            let flags = v["flags"].as_u64().unwrap() as u8;
                            registry.destroy(id, flags).unwrap();
                            if flags & 1 != 0 {
                                freed.push(id);
                            }
                        }
                        "Broadcast" => {
                            dispatch(
                                registry,
                                c,
                                v["event"].as_u64().unwrap() as u32,
                                v["value"].as_u64().unwrap() as u32,
                                depth + 1,
                                calls,
                                events,
                                freed,
                            );
                        }
                        _ => panic!("unknown action"),
                    }
                    // Preserve the outer captured link across every mutation
                    // and nested dispatch, not a fresh walk from registry head.
                    (*registry, cursor) =
                        serde_json::from_slice(&serde_json::to_vec(&(&registry, &cursor)).unwrap())
                            .unwrap();
                    cursor.validate(registry).unwrap();
                }
            }
        }
        cursor
            .acknowledge(
                &request,
                c["returns"][request.handler as usize].as_u64().unwrap() as u32,
            )
            .unwrap();
        assert!(cursor.acknowledge(&request, 1).is_err());
    }
    let returned = cursor.returned().unwrap();
    events.push(json!({"returned":returned,"depth":depth,"state":registry}));
    returned
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_commander_mutation_nested_dispatch_and_restore() {
    let report: Value = serde_json::from_str(include_str!("fixtures/commander.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 96);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut registry = Registry::default();
        registry.generation = c["generation"].as_u64().unwrap() as u32;
        for id in c["order"].as_array().unwrap() {
            let id = id.as_u64().unwrap() as u32;
            registry.construct(id).unwrap();
            registry.bind(id, id).unwrap();
        }
        let mut events = vec![];
        let mut freed = vec![];
        let returned = dispatch(
            &mut registry,
            c,
            c["event"].as_u64().unwrap() as u32,
            c["value"].as_u64().unwrap() as u32,
            1,
            &mut 0,
            &mut events,
            &mut freed,
        );
        assert_eq!(
            json!({"events":events,"state":registry,"freed":freed,"returned":returned}),
            row["original"],
            "Commander case {n}"
        );
    }
}

#[test]
fn released_captured_listener_and_registry_cycles_are_explicit_errors() {
    let mut r = Registry::<u32>::default();
    r.construct(1).unwrap();
    r.construct(2).unwrap();
    let mut c = Broadcast::begin(&r, 0xf6, 0).unwrap();
    let request = c.advance(&r).unwrap().unwrap().clone();
    r.destroy(1, 1).unwrap();
    assert!(c.validate(&r).is_err());
    c.acknowledge(&request, 1).unwrap();
    assert!(c.advance(&r).is_err());
    r.nodes.get_mut(&2).unwrap().next = Some(2);
    assert!(r.validate().is_err());
}
