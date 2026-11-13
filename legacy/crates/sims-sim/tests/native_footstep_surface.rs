use serde_json::Value;
use sims_sim::pose::properties::surface::{self, World};

struct Input<'a> {
    c: &'a Value,
    category_reads: usize,
}
impl World for Input<'_> {
    fn level(&self) -> i8 {
        self.c["level"].as_i64().unwrap() as i8
    }
    fn water(&mut self) -> Result<bool, String> {
        Ok(self.level() == 1 && self.c["water"] != 255)
    }
    fn objects(&mut self) -> Result<Vec<u32>, String> {
        Ok(self.c["objects"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["contained"] == false)
            .map(|o| o["guid"].as_u64().unwrap() as u32)
            .collect())
    }
    fn pool(&mut self) -> Result<u8, String> {
        Ok(self.c["pool"].as_u64().unwrap() as u8)
    }
    fn covering(&mut self) -> Result<u16, String> {
        Ok(self.c["covering"].as_u64().unwrap() as u16)
    }
    fn category(&mut self, _: u16) -> Result<u8, String> {
        self.category_reads += 1;
        // 4a1463 returns category 2 when the runtime catalog has no entry.
        Ok(self.c["category"].as_u64().unwrap_or(2) as u8)
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_surface_precedence_and_floor_label_categories() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/native-footstep-surface.json")).unwrap();
    let rows = report["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 968);
    for row in rows {
        let mut input = Input {
            c: &row["input"],
            category_reads: 0,
        };
        assert_eq!(
            surface::query(&mut input).unwrap(),
            row["original"]["surface"].as_i64().unwrap() as i32,
            "{row}"
        );
        assert_eq!(
            input.category_reads,
            row["original"]["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| **c == "category")
                .count()
        );
    }
    for row in report["labels"].as_array().unwrap() {
        assert_eq!(
            surface::category(row["label"].as_str().unwrap()),
            row["original"].as_u64().unwrap() as u8
        );
    }
}
