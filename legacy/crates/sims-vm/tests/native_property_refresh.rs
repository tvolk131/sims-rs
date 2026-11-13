use serde_json::{Value, json};
use sims_vm::interface::{
    property_refresh::{Call, Fields, Inputs, Query, Reply, Request},
    state::State,
};

fn raw(c: &Value, key: &str) -> u32 {
    c[key].as_i64().unwrap() as u32
}
fn fields(s: &State, f: &Fields, walls: u32) -> Value {
    json!({"dirty":s.buy.dirty,"zoom":f.zoom,"rotation":f.rotation,"floor":f.floor,
        "selected":s.selected_person_index.unwrap(),"undo":f.undo_available,"speed":s.speed,"paused":s.paused,"walls":walls})
}
fn run(c: &Value, resume: bool) -> Value {
    let mut s = State::constructed(11);
    s.paused = 7;
    s.selected_person_index = Some(123);
    s.buy.dirty = raw(c, "dirty");
    let mut f = Fields {
        zoom: 77,
        rotation: 88,
        floor: 99,
        undo_available: 6,
    };
    let mut inputs = Inputs {
        undo_vector_bytes: raw(c, "undo"),
        clock: [
            c["clock"][0].as_i64().unwrap() as i16,
            c["clock"][1].as_i64().unwrap() as i16,
        ],
        view: 5,
        panel: c["panel"].as_u64().map(|n| n as u32),
    };
    let mut call = Call::new(1, c["with_panels"].as_bool().unwrap()).unwrap();
    let mut walls = 9;
    let mut events = vec![];
    for _ in 0..64 {
        let Some(request) = call.advance(&mut s, &mut f, inputs).unwrap() else {
            break;
        };
        let before = (s.clone(), f, call.clone());
        assert_eq!(call.advance(&mut s, &mut f, inputs).unwrap(), Some(request));
        assert_eq!(
            (s.clone(), f, call.clone()),
            before,
            "replaying pending property request changed fields"
        );
        if resume {
            let saved = serde_json::to_vec(&(&s, f, &call, inputs, walls)).unwrap();
            (s, f, call, inputs, walls) =
                serde_json::from_slice::<(State, Fields, Call, Inputs, u32)>(&saved).unwrap();
        }
        let reply = match request {
            Request::SetFunds(_) => {
                s.mark_dirty(0x4000);
                Reply::Done
            }
            Request::Renderer => Reply::Number(3),
            Request::Funds => Reply::Number(raw(c, "funds")),
            Request::Speed => Reply::Number(raw(c, "speed")),
            Request::Paused => Reply::Number(raw(c, "paused")),
            Request::SelectedPerson => Reply::Number(raw(c, "selected")),
            Request::Camera { query, .. } => Reply::Number(match query {
                Query::Zoom => raw(c, "zoom"),
                Query::Rotation => raw(c, "rotation"),
                Query::Floor => raw(c, "floor"),
                Query::WallsDown => c["walls"][0].as_u64().unwrap() as u32,
                Query::Cutaway => c["walls"][1].as_u64().unwrap() as u32,
                Query::Roof => c["walls"][2].as_u64().unwrap() as u32,
            }),
            Request::SetSpeed(n) => {
                s.speed = n;
                s.mark_dirty(0x40);
                Reply::Done
            }
            Request::SetPaused(n) => {
                s.paused = n;
                s.mark_dirty(0x800040);
                Reply::Done
            }
            Request::SetWalls { value, .. } => {
                walls = value;
                s.mark_dirty(0x80);
                Reply::Done
            }
            _ => Reply::Done,
        };
        events.push(json!({"request":request,"fields":fields(&s,&f,walls)}));
        if let Some(m) = c["mutations"].get(events.len().to_string()) {
            if m.get("dirty").is_some() {
                s.buy.dirty = raw(m, "dirty");
            }
            if m.get("view").is_some() {
                inputs.view = raw(m, "view");
            }
            if let Some(n) = m.get("panel") {
                inputs.panel = n.as_u64().map(|n| n as u32);
            }
            if let Some(n) = m.get("clock") {
                inputs.clock = [n[0].as_i64().unwrap() as i16, n[1].as_i64().unwrap() as i16];
            }
        }
        call.acknowledge(request, reply).unwrap();
        if resume {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
        }
    }
    assert!(call.complete());
    let mut result = fields(&s, &f, walls);
    result["events"] = json!(events);
    result
}

#[cfg(feature = "private-fixtures")]
#[test]
fn property_refresh_matches_original_every_mask_callback_fields_and_cold_continuation() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/native-property-refresh.json")).unwrap();
    assert_eq!(
        corpus["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 1057);
    for (i, case) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        for resume in [false, true] {
            assert_eq!(
                run(&case["input"], resume),
                case["original"],
                "case{i}, cold{resume}"
            );
        }
    }
}

#[test]
fn property_refresh_rejects_foreign_duplicate_and_wrong_kind_replies() {
    let mut c = Call::new(1, true).unwrap();
    let mut s = State::constructed(1);
    s.buy.dirty = 0x2000;
    let mut f = Fields {
        zoom: 1,
        rotation: 0,
        floor: 1,
        undo_available: 0,
    };
    let inputs = Inputs {
        undo_vector_bytes: 0,
        clock: [7, 30],
        view: 5,
        panel: Some(2),
    };
    let q = c.advance(&mut s, &mut f, inputs).unwrap().unwrap();
    assert_eq!(q, Request::Renderer);
    let before = c.clone();
    assert!(c.acknowledge(Request::Funds, Reply::Number(1)).is_err());
    assert_eq!(c, before);
    assert!(c.acknowledge(q, Reply::Done).is_err());
    assert_eq!(c, before);
    c.acknowledge(q, Reply::Number(3)).unwrap();
    let before = c.clone();
    assert!(c.acknowledge(q, Reply::Number(3)).is_err());
    assert_eq!(c, before);
    let mut missing = serde_json::to_value(&c).unwrap();
    missing["value"] = Value::Null;
    assert!(
        serde_json::from_value::<Call>(missing)
            .unwrap()
            .validate()
            .is_err()
    );
    let mut missing = serde_json::to_value(&c).unwrap();
    missing["clock"] = Value::Null;
    assert!(
        serde_json::from_value::<Call>(missing)
            .unwrap()
            .validate()
            .is_err()
    );
}
