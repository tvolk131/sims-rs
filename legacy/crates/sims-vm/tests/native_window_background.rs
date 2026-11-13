use sims_vm::scheduler::clock::draw::{
    background::{Controls, Frame, Request},
    regions::List,
};
fn hex(v: &serde_json::Value) -> Vec<u8> {
    v.as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn list(p: u32, rows: &[[i32; 4]]) -> List {
    let mut h = vec![0; 24];
    for (i, v) in [p, p + 16 * rows.len() as u32, p + 16 * rows.len() as u32]
        .iter()
        .enumerate()
    {
        h[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    List {
        header: h,
        backing: rows
            .iter()
            .flatten()
            .flat_map(|x| x.to_le_bytes())
            .collect(),
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_window_background_preserves_order_unused_bytes_and_every_cold_cut() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-window-background.json")).unwrap();
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 1024);
    for (i, c) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        let v = &c["input"];
        let source: Vec<[i32; 4]> = serde_json::from_value(v["source"].clone()).unwrap();
        let damage: Vec<[i32; 4]> = serde_json::from_value(v["damage"].clone()).unwrap();
        let source = list(0x10002000, &source);
        let damage = list(0x10003000, &damage);
        let controls = Controls {
            enabled: v["enabled"].as_u64().unwrap() as u8,
            behind: v["behind"].as_u64().unwrap() as u8,
            full: v["full"].as_u64().unwrap() as u8,
            viewport: serde_json::from_value(v["viewport"].clone()).unwrap(),
            delta: serde_json::from_value(v["delta"].clone()).unwrap(),
            flags: v["flags"].as_u64().unwrap() as u32,
        };
        let mut temp = List {
            header: hex(&v["temporary_header"]),
            backing: hex(&v["temporary_backing"]),
        };
        let mut frame = Frame::new();
        let mut events = vec![];
        let mut offsets = 0;
        for _ in 0..8192 {
            let (mut cold, mut cold_temp): (Frame, List) =
                serde_json::from_slice(&serde_json::to_vec(&(&frame, &temp)).unwrap()).unwrap();
            if frame.complete() {
                break;
            }
            if frame.at_native_cut(0x4acff5) {
                offsets += 1;
            }
            if let Some(req) = frame.pending().cloned() {
                let (kind, rectangle) = match req {
                    Request::Color { rectangle } => ("Color", rectangle),
                    Request::Depth { rectangle } => ("Depth", rectangle),
                    _ => panic!("unsupported child in case{i}: {req:?}"),
                };
                events.push(serde_json::json!({"kind":kind,"rectangle":rectangle}));
                frame.acknowledge_clear(&req).unwrap();
                cold.acknowledge_clear(&req).unwrap();
            } else {
                assert!(
                    frame
                        .advance(controls, &source, &damage, &mut temp)
                        .unwrap(),
                    "case{i}"
                );
                assert!(
                    cold.advance(controls, &source, &damage, &mut cold_temp)
                        .unwrap()
                );
            }
            assert_eq!(frame, cold, "frame case{i}");
            assert_eq!(temp, cold_temp, "temporary case{i}");
        }
        assert!(frame.complete(), "case{i}");
        assert_eq!(
            temp.header,
            hex(&c["expected"]["temporary_header"]),
            "header case{i}"
        );
        let mut expected = hex(&v["temporary_backing"]);
        let used = hex(&c["expected"]["temporary_used_raw"]);
        expected[..used.len()].copy_from_slice(&used);
        assert_eq!(temp.backing, expected, "backing case{i}");
        assert_eq!(
            serde_json::json!(events),
            c["expected"]["events"],
            "calls case{i}"
        );
        assert_eq!(
            serde_json::json!(offsets),
            c["expected"]["offset_calls"],
            "offset calls case{i}"
        );
        assert_eq!(c["expected"]["world_unchanged"], true);
    }
}
#[test]
fn unsupported_initialization_and_growth_keep_their_saved_child() {
    let source = list(0x10002000, &[[0, 0, 8, 8]]);
    let damage = list(0x10003000, &[[0, 0, 8, 8]]);
    let controls = Controls {
        enabled: 1,
        behind: 0,
        full: 0,
        viewport: [0, 0, 16, 16],
        delta: [0, 0],
        flags: 3,
    };
    for initialize in [true, false] {
        let mut temp = list(0x10004000, &[]);
        if !initialize {
            temp.header[12] = 1;
        }
        let initial = temp.clone();
        let mut frame = Frame::new();
        for _ in 0..32 {
            if frame.pending().is_some() {
                break;
            }
            assert!(
                frame
                    .advance(controls, &source, &damage, &mut temp)
                    .unwrap()
            );
        }
        assert!(
            matches!(frame.pending(), Some(Request::InitializeTemporary)) && initialize
                || matches!(
                    frame.pending(),
                    Some(Request::GrowTemporary {
                        rectangle: [0, 0, 8, 8]
                    })
                ) && !initialize
        );
        assert_eq!(temp, initial);
        let saved = serde_json::to_vec(&(&frame, &temp)).unwrap();
        assert!(
            !frame
                .advance(controls, &source, &damage, &mut temp)
                .unwrap()
        );
        assert_eq!(saved, serde_json::to_vec(&(&frame, &temp)).unwrap());
        assert!(
            frame
                .acknowledge_clear(frame.pending().cloned().as_ref().unwrap())
                .is_err()
        );
    }
}
