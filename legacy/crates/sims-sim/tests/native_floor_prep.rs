use sims_sim::lot_presenter::floor_prep;
#[cfg(feature = "private-fixtures")]
#[test]
fn original_floor_descriptors_rotations_and_sentinels() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-floor-prep.json")).unwrap();
    assert_eq!(all["queries"].as_array().unwrap().len(), 1024);
    for (n, r) in all["queries"].as_array().unwrap().iter().enumerate() {
        let c = &r["input"];
        let p: [i8; 3] = serde_json::from_value(c["point"].clone()).unwrap();
        let seed = c["seed"].as_i64().unwrap() as i32;
        let family = c["family"].as_u64().unwrap();
        let mut cells = vec![];
        for y in 0..64 {
            for x in 0..64 {
                let mut v = [
                    (seed + x * 11 + y * 3 + i32::from(p[2])) as u8,
                    (seed + x * 5 - y * 7) as u8,
                    (seed + x * 13 - y * 9) as u8,
                    (seed - x * 3 + y * 17) as u8,
                ];
                if matches!(family, 0 | 2) {
                    v[..2].copy_from_slice(&[127; 2]);
                }
                if matches!(family, 0 | 1) {
                    v[2..].copy_from_slice(&[127; 2]);
                }
                cells.extend(v);
            }
        }
        let result =
            floor_prep::descriptor(&cells, p, c["rotation"].as_u64().unwrap() as u8).unwrap();
        let actual = result
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(actual, r["descriptor"].as_str().unwrap(), "case{n}");
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_resident_floor_lookup_and_brightness() {
    let all: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-floor-prep.json")).unwrap();
    let entries = [
        (1, ([27, 28, 29], 0x10009000)),
        (7, ([30, 31, 32], 0x10009100)),
        (65535, ([33, 34, 35], 0x10009200)),
    ]
    .into_iter()
    .collect();
    assert_eq!(all["lookups"].as_array().unwrap().len(), 36);
    for r in all["lookups"].as_array().unwrap() {
        let got = floor_prep::lookup(
            &entries,
            r["key"].as_u64().unwrap() as u32,
            r["zoom"].as_u64().unwrap() as u32,
        )
        .unwrap();
        assert_eq!(got.is_some(), r["found"].as_bool().unwrap());
        if let Some((image, palette)) = got {
            assert_eq!(u64::from(image), r["image"].as_u64().unwrap());
            assert_eq!(u64::from(palette), r["palette"].as_u64().unwrap());
        } else {
            assert_eq!(r["image"], 0x11223344u32);
            assert_eq!(r["palette"], 0xaabbccddu32);
        }
    }
    assert_eq!(all["brightness"].as_array().unwrap().len(), 42);
    for r in all["brightness"].as_array().unwrap() {
        let mut c = [0; 29];
        c[0x34 / 4] = r["flag"].as_u64().unwrap() as u32;
        c[0x48 / 4] = r["bits"].as_u64().unwrap() as u32;
        assert_eq!(
            u64::from(floor_prep::brightness(&c)),
            r["result"].as_u64().unwrap()
        );
    }
}
