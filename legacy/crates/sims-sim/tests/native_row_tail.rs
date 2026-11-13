use sims_sim::lot_presenter::row_tail::{Frame, Inputs, Request};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_pool_floor_gates_and_ordered_tail_stores_survive_each_cold_cut() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-row-tail.json")).unwrap();
    assert_eq!(v["cases"].as_array().unwrap().len(), 1024);
    let mut stores = 0;
    for (n, row) in v["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut pool = vec![255; 4096];
        for cell in c["cells"].as_array().unwrap() {
            let p: Vec<usize> = serde_json::from_value(cell[0].clone()).unwrap();
            pool[p[0] * 64 + p[1]] = cell[1].as_u64().unwrap() as u8;
        }
        let input = || Inputs {
            size: 64,
            active_size: c["active"].as_i64().unwrap() as i32,
            rotation: c["rotation"].as_u64().unwrap() as u8,
            pool: c["present"].as_bool().unwrap().then_some(pool.as_slice()),
            bounds: [0, 63, 0, 63],
            neighbors: [[0, -1, 0], [-1, 0, 0], [-1, -1, 0]],
            covering: c["covering"].as_u64().unwrap() as u16,
            floor_enabled: c["enabled"].as_bool().unwrap(),
            tile_width: c["width"].as_i64().unwrap() as i32,
        };
        let mut f = Frame::new(
            serde_json::from_value(c["point"].clone()).unwrap(),
            serde_json::from_value(c["rotated"].clone()).unwrap(),
            serde_json::from_value(c["screen"].clone()).unwrap(),
            serde_json::from_value(c["step"].clone()).unwrap(),
            c["count"].as_i64().unwrap() as i32,
            c["index"].as_i64().unwrap() as i32,
        );
        let mut events = vec![];
        for _ in 0..32 {
            if f.pending().is_some() || f.complete() {
                break;
            }
            let mut cold: Frame = serde_json::from_slice(&serde_json::to_vec(&f).unwrap()).unwrap();
            assert_eq!(f.advance(input()).unwrap(), cold.advance(input()).unwrap());
            assert_eq!(f, cold);
            if let Some((field, value)) = f.last_store() {
                events.push(serde_json::json!({"field":field,"value":value}));
            }
        }
        let pc = match f.pending() {
            Some(Request::WaterRaster | Request::OverlayRaster) => {
                panic!("old gate corpus unexpectedly advanced into contents")
            }
            Some(Request::PoolEdge) => 0x4af3a7,
            Some(Request::FloorRaster) => 0x4af453,
            Some(Request::OtherContents) => 0x4af9d3,
            Some(Request::NextTile) => 0x4aeebc,
            None => {
                assert!(f.complete());
                0x4b0147
            }
        };
        assert_eq!(row["target"], pc, "case{n}");
        assert_eq!(
            serde_json::json!({"point":f.point(),"screen":f.screen(),"index":f.index()}),
            row["original"],
            "locals case{n}"
        );
        assert_eq!(
            events,
            row["events"].as_array().unwrap().to_vec(),
            "stores case{n}"
        );
        stores += events.len();
    }
    assert_eq!(stores, 1396);
    println!("matched{stores} ordered tail stores");
}
