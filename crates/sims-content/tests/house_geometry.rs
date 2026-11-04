use serde_json::json;
use std::{fs, path::PathBuf};
fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
#[ignore = "requires original extracted residential houses"]
fn original_house_wall_geometry_and_partition_preview() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let mut rows = vec![];
    let mut preview = image::RgbImage::from_pixel(1536, 1024, image::Rgb([20, 27, 30]));
    for (house_col, source) in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ]
    .into_iter()
    .enumerate()
    {
        let h = sims_content::saves::House::load(&index, source).unwrap();
        for level in 1..=2 {
            let p = h.room_partition(level).unwrap();
            let g = h.wall_grid(level).unwrap();
            let f = &h.floors[(level - 1) as usize];
            let xoff = house_col as u32 * 512;
            let yoff = (level as u32 - 1) * 512;
            let split: std::collections::BTreeMap<_, _> =
                p.halves.iter().map(|s| (s.tile, &s.rooms)).collect();
            for x in 0..64 {
                for y in 0..64 {
                    let cell = x * 64 + y;
                    let expanded = g.expanded(x, y).unwrap();
                    assert_eq!(expanded.segments, f.walls[cell].segments);
                    for px in 0..8 {
                        for py in 0..8 {
                            let mut room = p.primary[cell];
                            if let Some(halves) = split.get(&[x, y]) {
                                let triangle = if f.walls[cell].segments & 16 != 0 {
                                    if px + py < 8 { 4 } else { 2 }
                                } else if px >= py {
                                    3
                                } else {
                                    1
                                };
                                room = halves[&triangle];
                            }
                            let local = room - ((level - 1) * 1024) as i16;
                            let color = if room < 0 {
                                [160, 55, 150]
                            } else if local == 0 && f.coverings[cell] == 0 {
                                [30, 42, 46]
                            } else {
                                let n = local as u32;
                                [
                                    (70 + (n * 43) % 130) as u8,
                                    (75 + (n * 79) % 130) as u8,
                                    (85 + (n * 31) % 120) as u8,
                                ]
                            };
                            preview.put_pixel(
                                xoff + x as u32 * 8 + px,
                                yoff + y as u32 * 8 + py,
                                image::Rgb(color),
                            );
                        }
                    }
                }
            }
            let segments: Vec<_> = g.owned_segments().collect();
            for (a, b, style) in &segments {
                for step in 0..=8 {
                    let x = (a.x + (b.x - a.x) * step / 8) / 2;
                    let y = (a.y + (b.y - a.y) * step / 8) / 2;
                    let color = if matches!(style, 3 | 5 | 6 | 15 | 23) {
                        [226, 153, 67]
                    } else {
                        [225, 235, 231]
                    };
                    if (0..512).contains(&x) && (0..512).contains(&y) {
                        preview.put_pixel(xoff + x as u32, yoff + y as u32, image::Rgb(color));
                    }
                }
            }
            println!(
                "{source} floor {level}: {} owned segments, {} split-room cells",
                segments.len(),
                p.halves.len()
            );
            rows.push(json!({"source":source,"input":{"size":h.width,"level":level,"walls":f.walls.iter().map(|w|w.segments).collect::<Vec<_>>(),"floors":f.coverings,"ground_regions":h.pool_cells().unwrap()},"rust":p}));
        }
    }
    fs::write(
        repo().join("cache/original-house-geometry.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    preview
        .save(repo().join("cache/original-house-room-partitions.png"))
        .unwrap();
}

#[test]
#[ignore = "requires original houses and private original-code partition oracle"]
fn original_house_partition_corpus_matches_native() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let native: serde_json::Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/house-room-partitions.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        native["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(native["cases"].as_array().unwrap().len(), 6);
    for b in native["cases"].as_array().unwrap() {
        let house =
            sims_content::saves::House::load(&index, b["source"].as_str().unwrap()).unwrap();
        let level = b["input"]["level"].as_i64().unwrap() as i32;
        let floor = &house.floors[(level - 1) as usize];
        assert_eq!(
            b["input"]["walls"],
            json!(floor.walls.iter().map(|w| w.segments).collect::<Vec<_>>())
        );
        assert_eq!(b["input"]["floors"], json!(floor.coverings));
        assert_eq!(
            b["input"]["ground_regions"],
            json!(house.pool_cells().unwrap())
        );
        let expected: sims_sim::lot_rooms::Partition =
            serde_json::from_value(b["original"].clone()).unwrap();
        let actual = house.room_partition(level).unwrap();
        assert_eq!(actual, expected, "{} level {}", b["source"], level);
    }
}

#[test]
#[ignore = "requires original houses and private original-code exterior/support oracle"]
fn original_house_geometry_corpus_matches_native() {
    let index = sims_content::Index::scan(&repo().join("game-data/cabinet-extracted")).unwrap();
    let report: serde_json::Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/house-lot-flags.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    let metrics_report: serde_json::Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/house-room-metrics.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(metrics_report["capture_sha256"], report["capture_sha256"]);
    let queries_report: serde_json::Value = serde_json::from_slice(
        &fs::read(repo().join("cache/research/sims-exe/oracle/room-query.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(queries_report["capture_sha256"], report["capture_sha256"]);
    for pair in cases.chunks_exact(2) {
        assert_eq!(pair[0]["source"], pair[1]["source"]);
        let house =
            sims_content::saves::House::load(&index, pair[0]["source"].as_str().unwrap()).unwrap();
        // The native probe ran after grid repair. Fresh house geometry is
        // rebuilt here; cached Rust outputs are never the expected result.
        let heads: Vec<Vec<u16>> = pair
            .iter()
            .map(|r| serde_json::from_value(r["input"]["object_heads"].clone()).unwrap())
            .collect();
        let actual = house.lot_geometry(&heads).unwrap();
        let architecture = sims_sim::architecture::Architecture::from_original(
            actual.clone(),
            &sims_vm::Program::default(),
        )
        .unwrap();
        for (i, case) in pair.iter().enumerate() {
            let input = &case["input"];
            let floor = &house.floors[i];
            assert_eq!(input["level"], json!(i + 1));
            assert_eq!(input["size"], json!(house.width));
            assert_eq!(
                input["walls"],
                json!(floor.walls.iter().map(|w| w.segments).collect::<Vec<_>>())
            );
            assert_eq!(
                input["styles"],
                json!(floor.walls.iter().map(|w| w.styles).collect::<Vec<_>>())
            );
            assert_eq!(input["floors"], json!(floor.coverings));
            assert_eq!(input["flags"], json!(floor.flags));
            assert_eq!(input["heights"], json!(actual.heights));
            assert_eq!(input["ground_regions"], json!(actual.pool));
            if i > 0 {
                assert_eq!(input["below"], json!(actual.floors[i - 1].flags.flags));
            }
            let flags: sims_sim::lot_flags::Rebuilt =
                serde_json::from_value(case["original"].clone()).unwrap();
            let rooms: sims_sim::lot_rooms::Partition =
                serde_json::from_value(case["original"]["rooms"].clone()).unwrap();
            assert_eq!(
                actual.floors[i].flags,
                flags,
                "{} floor {} flags",
                case["source"],
                i + 1
            );
            assert_eq!(
                actual.floors[i].rooms,
                rooms,
                "{} floor {} rooms",
                case["source"],
                i + 1
            );
            let metric = metrics_report["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["source"] == case["source"] && m["level"] == input["level"])
                .unwrap();
            assert_eq!(json!(actual.floors[i].metrics), metric["original"]);
            let queries = queries_report["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|m| m["source"] == case["source"] && m["input"]["level"] == input["level"])
                .unwrap();
            for q in queries["queries"].as_array().unwrap() {
                let p: [i32; 2] = serde_json::from_value(q["point"].clone()).unwrap();
                assert_eq!(
                    architecture.room_at(i as i32 + 1, sims_sim::Point::new(p[0], p[1])),
                    q["room"].as_i64().unwrap() as i16
                );
            }
            for x in 0..house.width {
                for y in 0..house.height {
                    let point = sims_sim::Point::new(x as i32 * 16 + 8, y as i32 * 16 + 8);
                    let cell = x * house.height + y;
                    assert_eq!(
                        architecture.primary_room_at(i as i32 + 1, point),
                        actual.floors[i].rooms.primary[cell]
                    );
                    assert_eq!(
                        architecture.supported(i as i32 + 1, point),
                        i == 0 || floor.coverings[cell] != 0
                    );
                }
            }
        }
    }
}
