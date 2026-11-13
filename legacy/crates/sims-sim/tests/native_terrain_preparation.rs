use sims_sim::lot_presenter::terrain_shade::Map;
use sims_vm::scheduler::clock::draw::ground::visible::{Frame, Reply, Request};
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn snow_and_wall_children_are_explicit_and_corrupt_owner_stores_are_rejected() {
    let mut raw = vec![0; 0x388];
    raw[0x81] = 1;
    let mut f = Frame::new([2, 2], [0, 63, 0, 63], 1);
    for _ in 0..64 {
        if f.pending() == Some(Request::Snow) {
            break;
        }
        if let Some(r) = f.pending() {
            let reply = match r {
                Request::Shade { .. } => Reply::Shade(7),
                Request::Terrain { .. } => Reply::Terrain(20),
                Request::Variation { .. } => Reply::Variation(8),
                other => panic!("{other:?}"),
            };
            f.reply(r, reply).unwrap();
        } else {
            f.advance(&mut raw, 64, 1).unwrap();
        }
    }
    assert_eq!(f.pending(), Some(Request::Snow));
    assert!(!f.complete());
    assert!(!f.advance(&mut raw, 64, 1).unwrap());
    let mut corrupt = raw.clone();
    corrupt[0x360] ^= 1;
    assert!(f.advance(&mut corrupt, 64, 1).is_err());
    let mut raw = vec![0; 0x388];
    raw[0x80] = 1;
    raw[0x82] = 1;
    let mut f = Frame::new([2, 2], [0, 63, 0, 63], 1);
    for _ in 0..64 {
        if matches!(f.pending(), Some(Request::WallFlags { .. })) {
            break;
        }
        if let Some(r) = f.pending() {
            let reply = match r {
                Request::Shade { .. } => Reply::Shade(7),
                Request::Terrain { .. } => Reply::Terrain(20),
                Request::Variation { .. } => Reply::Variation(8),
                Request::Covering { .. } => Reply::Covering(1),
                other => panic!("{other:?}"),
            };
            f.reply(r, reply).unwrap();
        } else {
            f.advance(&mut raw, 64, 1).unwrap();
        }
    }
    assert!(matches!(f.pending(), Some(Request::WallFlags { .. })));
    assert!(!f.complete());
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_terrain_preparation_matches_each_store_and_cold_continuation() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-terrain-preparation.json")).unwrap();
    assert_eq!(v["cases"].as_array().unwrap().len(), 1024);
    let mut stores = 0;
    for (n, row) in v["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let point: [i32; 2] = serde_json::from_value(c["point"].clone()).unwrap();
        let zoom = c["zoom"].as_u64().unwrap() as u32;
        let active = c["active"].as_i64().unwrap() as i32;
        let height = c["particle_height"].as_i64().unwrap() as i32;
        let seed = c["signed_seed"].as_i64().unwrap() as i32;
        let data: Vec<i8> = (0..64)
            .flat_map(|x| (0..64).map(move |y| (seed + 19 * x + 31 * y) as i8))
            .collect();
        let map = Map {
            width: 64,
            height: 64,
            rotation: c["rotation"].as_u64().unwrap() as u8,
            cells: &data,
        };
        let mut raw = hex(c["raw"].as_str().unwrap());
        let mut f = Frame::new(point, [0, 63, 0, 63], zoom);
        let mut events = vec![];
        for _ in 0..100 {
            let (mut cold, mut cold_raw): (Frame, Vec<u8>) =
                serde_json::from_slice(&serde_json::to_vec(&(&f, &raw)).unwrap()).unwrap();
            if let Some(r) = f.pending() {
                let reply = match r {
                    Request::Shade { point } => {
                        let count = i32::from_le_bytes(raw[0x7c..0x80].try_into().unwrap());
                        Reply::Shade(
                            map.corner([point[0] as i8, point[1] as i8, 1], count)
                                .unwrap(),
                        )
                    }
                    Request::Terrain { point } => Reply::Terrain(map.sample(point).unwrap()),
                    Request::Variation { .. } => {
                        Reply::Variation(c["variation"].as_u64().unwrap() as u16)
                    }
                    Request::Covering { .. } => Reply::Covering(0),
                    Request::PoolInclusive { .. } => {
                        Reply::PoolInclusive(c["pool"].as_u64().unwrap() <= 16)
                    }
                    other => panic!("unexpected child{other:?}"),
                };
                f.reply(r, reply).unwrap();
                cold.reply(r, reply).unwrap();
            } else {
                assert_eq!(
                    f.advance(&mut raw, active, height).unwrap(),
                    cold.advance(&mut cold_raw, active, height).unwrap()
                );
                if let Some((offset, size, value)) = f.last_store() {
                    events.push(serde_json::json!({"offset":offset,"size":size,"value":value}));
                }
            }
            assert_eq!(f, cold);
            assert_eq!(raw, cold_raw);
            if f.complete() {
                break;
            }
        }
        assert!(f.complete(), "case{n}");
        assert_eq!(
            events,
            row["events"].as_array().unwrap().to_vec(),
            "stores case{n}"
        );
        assert_eq!(raw, hex(row["original"].as_str().unwrap()), "owner case{n}");
        assert_eq!(
            f.result().unwrap(),
            row["result"].as_u64().unwrap() as u32,
            "return case{n}"
        );
        stores += events.len();
    }
    assert_eq!(stores, 8608);
}
