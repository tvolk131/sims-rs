use sims_formats::lot::WallTile;
use sims_sim::lot_presenter::{terrain_shade::Map, terrain_walls};
use sims_vm::scheduler::clock::draw::ground::visible::{Frame, Reply, Request};
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_covered_ground_stores_and_suspended_wall_replies() {
    let all: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/native-terrain-wall-preparation.json"
    ))
    .unwrap();
    let rows = all["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 256);
    let mut stores = 0;
    for (n, row) in rows.iter().enumerate() {
        let c = &row["input"];
        let point = serde_json::from_value(c["point"].clone()).unwrap();
        let zoom = c["zoom"].as_u64().unwrap() as u32;
        let active = c["active"].as_i64().unwrap() as i32;
        let height = c["particle_height"].as_i64().unwrap() as i32;
        let rotation = c["rotation"].as_u64().unwrap() as u8;
        let seed = c["signed_seed"].as_i64().unwrap() as i32;
        let data: Vec<i8> = (0..64)
            .flat_map(|x| (0..64).map(move |y| (seed + 19 * x + 31 * y) as i8))
            .collect();
        let map = Map {
            width: 64,
            height: 64,
            rotation,
            cells: &data,
        };
        let cells = vec![
            WallTile {
                segments: c["segments"].as_u64().unwrap() as u8,
                ..WallTile::default()
            };
            4096
        ];
        let mut raw = hex(c["raw"].as_str().unwrap());
        let mut f = Frame::new(point, [0, 63, 0, 63], zoom);
        let mut events = vec![];
        let mut wall = false;
        for _ in 0..100 {
            let (mut cold, mut cold_raw): (Frame, Vec<u8>) =
                serde_json::from_slice(&serde_json::to_vec(&(&f, &raw)).unwrap()).unwrap();
            if let Some(r) = f.pending() {
                let reply = match r {
                    Request::Shade { point } => Reply::Shade(
                        map.corner(
                            [point[0] as i8, point[1] as i8, 1],
                            i32::from_le_bytes(raw[0x7c..0x80].try_into().unwrap()),
                        )
                        .unwrap(),
                    ),
                    Request::Terrain { point } => Reply::Terrain(map.sample(point).unwrap()),
                    Request::Variation { .. } => {
                        Reply::Variation(c["variation"].as_u64().unwrap() as u16)
                    }
                    Request::Covering { .. } => Reply::Covering(1),
                    Request::WallFlags { point } => {
                        let bits = terrain_walls::segments(64, &cells, point, rotation).unwrap();
                        assert_eq!(
                            u64::from(bits),
                            row["wall_reply"].as_u64().unwrap(),
                            "wall case{n}"
                        );
                        wall = true;
                        Reply::WallFlags(bits)
                    }
                    other => panic!("unexpected child{other:?}"),
                };
                f.reply(r, reply).unwrap();
                cold.reply(r, reply).unwrap();
                if let Reply::WallFlags(bits) = reply {
                    assert_eq!(f.wall_reply(), Some(bits));
                    assert!(f.at_native_cut(0x49d14b));
                }
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
        assert!(wall && f.complete(), "case{n}");
        assert_eq!(
            events,
            row["events"].as_array().unwrap().to_vec(),
            "stores case{n}"
        );
        assert_eq!(raw, hex(row["original"].as_str().unwrap()), "owner case{n}");
        assert_eq!(
            u64::from(f.result().unwrap()),
            row["result"].as_u64().unwrap(),
            "return case{n}"
        );
        stores += events.len();
    }
    assert_eq!(stores, 2009);
}
