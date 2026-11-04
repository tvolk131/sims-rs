use sims_sim::lot_presenter::tile_depth::Inputs;
use sims_vm::scheduler::clock::draw::ground::{Frame, Request, Store};
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn edge_delegate_preserves_every_original_store_and_suspended_resume() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-ground-delegate.json")).unwrap();
    assert_eq!(corpus["cases"].as_array().unwrap().len(), 512);
    let mut count = 0;
    for (n, row) in corpus["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let point: [i32; 2] = serde_json::from_value(c["point"].clone()).unwrap();
        let z = c["zoom"].as_u64().unwrap() as u8;
        let mut f = Frame::new(point, u32::from(z));
        let mut raw = hex(c["raw"].as_str().unwrap());
        let flags = c["flags"].as_u64().map(|v| v as u8);
        let heights = vec![serde_json::from_value(c["corners"].clone()).unwrap(); 4096];
        let seed = c["row_seed"].as_i64().unwrap() as i32;
        let slope = c["row_slope"].as_i64().unwrap() as i32;
        let cs = c["curve_seed"].as_i64().unwrap() as i32;
        let rows: Vec<i32> = (0..8192).map(|i| seed + slope * i).collect();
        let curve: Vec<i16> = (0..256).map(|i| (i * 71 + cs) as i16).collect();
        let inputs = Inputs {
            size: 64,
            rotation: c["rotation"].as_u64().unwrap() as u8,
            heights: &heights,
            zoom: z,
            row_start: 0,
            rows: &rows,
            curve_start: 0,
            curve: &curve,
        };
        let depth = inputs
            .sample([point[0] as i8, point[1] as i8, 0], 1)
            .unwrap();
        let mut events = vec![];
        for _ in 0..32 {
            let (mut cold, mut cold_raw): (Frame, Vec<u8>) =
                serde_json::from_slice(&serde_json::to_vec(&(&f, &raw)).unwrap()).unwrap();
            if let Some(r) = f.pending() {
                assert_eq!(r, Request::Common);
                let cell = c["cell"].as_u64().unwrap() as u32;
                f.common(cell, depth).unwrap();
                cold.common(cell, depth).unwrap();
            } else {
                assert_eq!(
                    f.advance(&mut raw, flags, [0, 63, 0, 63]).unwrap(),
                    cold.advance(&mut cold_raw, flags, [0, 63, 0, 63]).unwrap()
                );
                if let Some(s) = f.last_store() {
                    events.push(match s {
                        Store::Owner {
                            offset,
                            size,
                            value,
                        } => serde_json::json!({"offset":offset,"size":size,"value":value}),
                        Store::ContextAlias => serde_json::json!({"context_alias":true}),
                    });
                }
            }
            assert_eq!(f, cold);
            assert_eq!(raw, cold_raw);
            if f.complete() {
                break;
            }
        }
        assert!(f.complete(), "case{n} incomplete");
        assert!(f.alias_written());
        assert_eq!(
            events,
            row["events"].as_array().unwrap().to_vec(),
            "ordered stores case{n}"
        );
        assert_eq!(raw, hex(row["original"].as_str().unwrap()), "owner case{n}");
        count += events.len();
    }
    assert_eq!(count, 4915);
}
#[test]
fn changed_zoom_and_visible_tiles_remain_children_and_corrupt_resumes_fail() {
    let mut raw = vec![0; 0x388];
    raw[0xd8..0xdc].copy_from_slice(&1u32.to_le_bytes());
    let mut f = Frame::new([1, 1], 2);
    f.advance(&mut raw, None, [0, 63, 0, 63]).unwrap();
    f.common(7, 0).unwrap();
    f.advance(&mut raw, None, [0, 63, 0, 63]).unwrap();
    let mut corrupt = raw.clone();
    corrupt[0xc] ^= 1;
    assert!(f.advance(&mut corrupt, None, [0, 63, 0, 63]).is_err());
    for _ in 0..16 {
        if f.pending().is_some() {
            break;
        }
        f.advance(&mut raw, None, [0, 63, 0, 63]).unwrap();
    }
    assert_eq!(f.pending(), Some(Request::Zoom { zoom: 2 }));
    assert!(!f.alias_written());
    assert!(!f.advance(&mut raw, None, [0, 63, 0, 63]).unwrap());
    let mut f = Frame::new([1, 1], 1);
    f.advance(&mut raw, None, [0, 63, 0, 63]).unwrap();
    f.common(7, 0).unwrap();
    for _ in 0..16 {
        if f.pending().is_some() {
            break;
        }
        f.advance(&mut raw, None, [0, 63, 0, 63]).unwrap();
    }
    assert_eq!(f.pending(), Some(Request::Visible));
    assert!(f.alias_written());
    assert!(!f.complete());
}
