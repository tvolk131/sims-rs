//!Private recovered-byte comparisons; generated only by the guarded harness.
use serde_json::{Value, json};
use sims_sim::lot_presenter::{
    hover::Clear,
    lifecycle::{Cursor, Reply, Request, State, roof},
};

fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
    serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
}

fn wall_event(events: &mut Vec<Value>, c: &Value, request: Value, state: &mut State) {
    let index = events.len();
    events.push(json!({"request":request,"state":state}));
    for p in c["changes"].as_array().unwrap() {
        if p["event"].as_u64().unwrap() as usize != index {
            continue;
        }
        if let Some(v) = p["level"].as_i64() {
            state.level = v as i32;
        }
        for (name, field) in [
            ("all_walls", &mut state.all_walls),
            ("enabled", &mut state.visibility.enabled),
            ("dirty", &mut state.visibility.dirty),
            ("include_visible", &mut state.visibility.include_visible),
        ] {
            if let Some(v) = p[name].as_u64() {
                *field = v as u8;
            }
        }
        for row in p["rows"].as_array().into_iter().flatten() {
            state.visibility.mask.rows[row[0].as_u64().unwrap() as usize] =
                row[1].as_u64().unwrap();
        }
    }
}

#[test]
#[ignore = "requires the private guarded wall-publication native output"]
fn recovered_walls_and_roof_match_saved_callbacks_and_live_mutations() {
    let path =
        std::env::var("SIMS_WALL_PUBLICATION_ORACLE").expect("explicit native output required");
    let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report["native_execution"], true);
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["walls"].as_array().unwrap().len(), 192);
    assert_eq!(report["roof"].as_array().unwrap().len(), 388);
    for (n, row) in report["walls"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut state: State = serde_json::from_value(c["state"].clone()).unwrap();
        let mut cursor = Cursor::walls_down(
            &mut state,
            c["operation"]["WallsDown"].as_u64().unwrap() as u8,
        )
        .unwrap();
        let mut events = vec![];
        while let Some(request) = cursor.request() {
            (state, cursor) = roundtrip(&(state, cursor));
            cursor.validate(&state).unwrap();
            wall_event(
                &mut events,
                c,
                serde_json::to_value(&request).unwrap(),
                &mut state,
            );
            if request == Request::ClearMask {
                let mut clear = Clear::begin(
                    c["size"].as_u64().unwrap() as usize,
                    serde_json::from_value(c["origin"].clone()).unwrap(),
                )
                .unwrap();
                while let Some(p) = clear.advance(&mut state.visibility, state.level).unwrap() {
                    (state, cursor, clear) = roundtrip(&(state, cursor, clear));
                    clear.validate().unwrap();
                    assert_eq!(
                        clear.advance(&mut state.visibility, state.level).unwrap(),
                        Some(p)
                    );
                    wall_event(&mut events, c, json!({"Tile":p}), &mut state);
                    clear.acknowledge(p).unwrap();
                }
                assert!(clear.complete());
            }
            cursor
                .acknowledge(&mut state, &request, Reply::Done)
                .unwrap();
        }
        assert!(cursor.complete());
        assert_eq!(
            json!({"events":events,"state":state}),
            row["original"],
            "walls {n}"
        );
    }
    for (n, row) in report["roof"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut fields = c["fields"].clone();
        let mut cursor = roof::Cursor::new(
            c["argument"].as_u64().unwrap() as u32,
            c["force"].as_u64().unwrap() as u32,
        );
        let mut events = vec![];
        loop {
            cursor = roundtrip(&cursor);
            let mut roof = fields["roof"].as_u64().unwrap() as u8;
            let mut dirty = fields["dirty"].as_u64().unwrap() as u8;
            let controls = fields["root"] == true && fields["controls"] == true;
            let request = cursor.advance(&mut roof, &mut dirty, controls).unwrap();
            fields["roof"] = json!(roof);
            fields["dirty"] = json!(dirty);
            let Some(request) = request else { break };
            events.push(json!({"request":request,"fields":fields}));
            match request {
                roof::Request::InvalidateView => {
                    if let Some(patch) = c["patch"].as_object() {
                        for (key, value) in patch {
                            fields[key] = value.clone();
                        }
                    }
                }
                roof::Request::UiFlags(bits) => {
                    fields["flags"] = json!(fields["flags"].as_u64().unwrap() as u32 | bits);
                }
            }
            cursor = roundtrip(&cursor);
            cursor.acknowledge(request).unwrap();
        }
        assert!(cursor.complete());
        assert_eq!(
            json!({"events":events,"fields":fields}),
            row["original"],
            "roof {n}"
        );
    }
}
