use sims_sim::{
    lot_presenter::room_masks::{Camera, View},
    pose::transform::Terrain,
};
use sims_vm::scheduler::clock::draw::context::placement::{
    Caches, Depth, PlacementUpdate, Reply, Request,
};
#[cfg(feature = "private-fixtures")]
#[test]
fn original_placement_projection_and_depth_match_each_store_and_suspended_restore() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-draw-placement.json")).unwrap();
    assert_eq!(v["cases"].as_array().unwrap().len(), 512);
    let mut total = 0;
    for (n, row) in v["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut words: [u32; 29] = serde_json::from_value(c["raw"].clone()).unwrap();
        let mut caches: Caches = serde_json::from_value(
            serde_json::json!({"alpha":c["caches"][0],"beta":c["caches"][1]}),
        )
        .unwrap();
        let z = c["zoom"].as_u64().unwrap() as u8;
        let rotation = c["rotation"].as_u64().unwrap() as u8;
        let physical = serde_json::from_value(c["physical"].clone()).unwrap();
        let mut f = PlacementUpdate::new(
            physical,
            c["level"].as_i64().unwrap() as i32,
            c["override"].as_i64().unwrap() as i32,
            rotation,
        );
        let heights = vec![serde_json::from_value(c["corners"].clone()).unwrap(); 4096];
        let walls = vec![sims_formats::lot::WallTile::default(); 4096];
        let splits = Default::default();
        let camera = Camera {
            origin: serde_json::from_value(c["origin"].clone()).unwrap(),
            zoom: z,
            tile_width: 16 << z,
            half_width: 8 << z,
            bottom: 8 << z,
            floor_heights: vec![23, 58, 116, 232],
        };
        let view = View {
            size: 64,
            rotation,
            bounds: [0, 63, 0, 63],
            camera: &camera,
            heights: &heights,
            walls: &walls,
            splits: &splits,
        };
        let seed = c["row_seed"].as_i64().unwrap() as i32;
        let slope = c["row_slope"].as_i64().unwrap() as i32;
        let curve_seed = c["curve_seed"].as_i64().unwrap() as i32;
        let rows: Vec<i32> = (-32768i32..32768)
            .map(|i| seed.wrapping_add(slope.wrapping_mul(i)))
            .collect();
        let curve: Vec<i16> = (-1024i32..1024)
            .map(|i| (i * i * 3 + 7 * i + curve_seed) as i16)
            .collect();
        let d = Depth {
            origin_y: camera.origin[1],
            clip: serde_json::from_value(c["clip"].clone()).unwrap(),
            heights: [232, 116, 58],
            extents: [c["extent"].as_i64().unwrap() as i32; 3],
            limit: c["limit"].as_i64().unwrap() as i32,
            level_depth: [-967; 3],
            row_start: -32768,
            rows: &rows,
            curve_start: -1024,
            curve: &curve,
        };
        let mut events = vec![];
        for _ in 0..100 {
            let (mut cold, mut cold_words, mut cold_caches): (PlacementUpdate, [u32; 29], Caches) =
                serde_json::from_slice(&serde_json::to_vec(&(&f, &words, &caches)).unwrap())
                    .unwrap();
            if let Some(request) = f.pending() {
                let reply = match request {
                    Request::Height { physical } => Reply::Height(
                        (Terrain {
                            size: 64,
                            corners: &heights,
                        }
                        .height_with_control(physical, 0x003f)
                        .unwrap()
                        .unwrap() as f32)
                            .to_bits(),
                    ),
                    Request::Screen {
                        physical,
                        level,
                        height,
                    } => Reply::Screen(
                        view.fine_screen_with_height(physical, level, Some(height))
                            .unwrap(),
                    ),
                    Request::OffsetRect { rect, delta } => {
                        Reply::Rect(std::array::from_fn(|i| rect[i].wrapping_add(delta[i % 2])))
                    }
                };
                f.acknowledge(request, reply).unwrap();
                cold.acknowledge(request, reply).unwrap();
            } else {
                let progress = f
                    .advance_words(&mut words, &mut caches, &d)
                    .unwrap_or_else(|e| panic!("case{n}: {e}"));
                assert_eq!(
                    progress,
                    cold.advance_words(&mut cold_words, &mut cold_caches, &d)
                        .unwrap()
                );
            }
            assert_eq!(f, cold);
            assert_eq!(words, cold_words);
            assert_eq!(caches, cold_caches);
            // The cursor's last store is exposed separately from its cumulative mask.
            if let Some((offset, value)) = f.last_store() {
                events.push(serde_json::json!({"offset":offset,"value":value}));
            }
            if f.complete() {
                break;
            }
        }
        assert!(f.complete(), "case{n} incomplete");
        let expected: Vec<_> = row["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| serde_json::json!({"offset":e["offset"],"value":e["value"]}))
            .collect();
        assert_eq!(events, expected, "stores case{n}");
        assert_eq!(
            serde_json::to_value(words).unwrap(),
            row["original"],
            "final case{n}"
        );
        assert_eq!(
            serde_json::to_value([caches.alpha, caches.beta]).unwrap(),
            row["caches"],
            "cache case{n}"
        );
        total += events.len();
    }
    assert_eq!(total, 4973);
}
