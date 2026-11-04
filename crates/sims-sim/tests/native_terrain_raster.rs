use sha2::{Digest, Sha256};
use sims_sim::lot_presenter::terrain_raster::{Data, Frame, ParticleColumn, Spec, Write};
fn bytes(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
fn sha(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_ground_columns_and_particles_match_every_ordered_store() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-terrain-raster.json")).unwrap();
    let rows = v["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 64);
    for (n, r) in rows.iter().enumerate() {
        let input = &r["input"];
        let mut owner = bytes(input["raw"].as_str().unwrap());
        let mut data = Data {
            colors: 8,
            palette: (0..4)
                .map(|s| {
                    (0..8)
                        .flat_map(|t| {
                            (0..8).flat_map(move |i| {
                                ((0x1000 * s + 0x100 * t + i) as u16).to_le_bytes()
                            })
                        })
                        .collect()
                })
                .collect(),
            particles: input["particles"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| ParticleColumn {
                    begin: p["begin"].as_u64().unwrap() as u32,
                    records: bytes(p["records"].as_str().unwrap()),
                })
                .collect(),
            color: vec![0x5a; 512 * 256],
            depth: vec![0xa5; 512 * 256],
            color_pitch: 512,
            depth_pitch: 512,
        };
        let spec: Spec = serde_json::from_value(input["spec"].clone()).unwrap();
        let mut f = Frame::new(spec, &owner).unwrap();
        let expected: Vec<Write> = serde_json::from_value(r["events"].clone()).unwrap();
        let mut stores = vec![];
        for _ in 0..100000 {
            if f.complete() {
                break;
            }
            assert!(
                f.advance(&mut data, &mut owner)
                    .unwrap_or_else(|e| panic!("case{n}, store{}: {e}", stores.len()))
            );
            if let Some(w) = f.last_store() {
                assert_eq!(
                    Some(w),
                    expected.get(stores.len()),
                    "case{n}, store{}",
                    stores.len()
                );
                stores.push(w.clone());
                if stores.len() == 1 || stores.len() == expected.len() / 2 {
                    let cold: (Frame, Data, Vec<u8>) =
                        serde_json::from_slice(&serde_json::to_vec(&(&f, &data, &owner)).unwrap())
                            .unwrap();
                    assert_eq!((&f, &data, &owner), (&cold.0, &cold.1, &cold.2));
                    f = cold.0;
                    data = cold.1;
                    owner = cold.2;
                }
            }
        }
        assert!(f.complete(), "case{n}");
        assert_eq!(stores, expected, "case{n}");
        assert_eq!(owner, bytes(r["owner"].as_str().unwrap()), "case{n}");
        for (c, p) in data.particles.iter().enumerate() {
            assert_eq!(
                p.records,
                bytes(r["particles"][c].as_str().unwrap()),
                "case{n}, column{c}"
            );
        }
        assert_eq!(sha(&data.color), r["color_sha256"], "case{n}");
        assert_eq!(sha(&data.depth), r["depth_sha256"], "case{n}");
    }
}
