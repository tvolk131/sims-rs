use sims_sim::lot_environment::lighting::point::{Sample, map_sample};
#[cfg(feature = "private-fixtures")]
#[test]
fn ordinary_point_light_matches_original_rotations_bytes_floors_and_wrapping() {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/native-point-light.json")).unwrap();
    assert_eq!(v["scale_bits"], 0x3b808081u32);
    assert_eq!(v["cases"].as_array().unwrap().len(), 512);
    for row in v["cases"].as_array().unwrap() {
        let c = &row["input"];
        let p: [i32; 2] = serde_json::from_value(c["physical"].clone()).unwrap();
        let rotation = c["rotation"].as_u64().unwrap() as u8;
        let level = c["level"].as_i64().unwrap() as i32;
        let [x, y] = [(p[1] >> 4) as u8, (p[0] >> 4) as u8];
        let r = match (4 - rotation) & 3 {
            1 => [63 - y, x],
            2 => [63 - x, 63 - y],
            3 => [y, 63 - x],
            _ => [x, y],
        };
        let index = r[0] as usize * 64 + r[1] as usize;
        let mut primary = vec![1; 4096];
        primary[index] = c["room"].as_i64().unwrap() as i16;
        let mut light = vec![0; 4096];
        light[index] = c["light"].as_u64().unwrap() as u8;
        assert_eq!(
            map_sample(p, level, rotation, 64, &primary, &light).unwrap(),
            Sample::Value(row["original"].as_u64().unwrap() as u32)
        );
        primary[index] = -5;
        assert_eq!(
            map_sample(p, level, rotation, 64, &primary, &light).unwrap(),
            Sample::Split {
                point: [x as i8, y as i8, level as i8]
            }
        );
    }
    assert!(map_sample([0, 0], 1, 4, 64, &vec![0; 4096], &vec![0; 4096]).is_err());
    assert!(map_sample([0, 0], 1, 0, 64, &[], &vec![0; 4096]).is_err());
}
