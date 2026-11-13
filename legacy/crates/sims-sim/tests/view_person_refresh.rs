use serde::Deserialize;
use sims_sim::lot_presenter::person_refresh::{Cursor, View};
use sims_vm::ObjectId;

#[derive(Deserialize)]
struct Person {
    tile: [i8; 3],
    word: i16,
}
#[derive(Deserialize)]
struct Input {
    level: i32,
    size: i32,
    people: Vec<Person>,
    order: Vec<ObjectId>,
    mutate: bool,
}
struct Live(Input);
impl View for Live {
    fn people(&self) -> &[ObjectId] {
        &self.0.order
    }
    fn level(&self) -> i32 {
        self.0.level
    }
    fn size(&self) -> i32 {
        self.0.size
    }
    fn tile(&self, id: ObjectId) -> Result<[i8; 3], String> {
        Ok(self.0.people[id as usize - 1].tile)
    }
    fn field40(&self, id: ObjectId) -> Result<i16, String> {
        Ok(self.0.people[id as usize - 1].word)
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn view_person_refresh_matches_original_vector_and_live_callback_reads() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/view-person-refresh.json")).unwrap();
    assert_eq!(
        fixture["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        fixture["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 160);
    for row in rows {
        let mut v = Live(serde_json::from_value(row["input"].clone()).unwrap());
        // Native fixture reports allocation indexes; Rust uses positive IDs.
        for id in &mut v.0.order {
            *id += 1;
        }
        let mut cursor = Cursor::default();
        let mut events = vec![];
        while let Some(id) = cursor.advance(&v).unwrap() {
            cursor = serde_json::from_slice(&serde_json::to_vec(&cursor).unwrap()).unwrap();
            assert_eq!(cursor.advance(&v).unwrap(), Some(id));
            events.push(id - 1);
            if events.len() == 1 && v.0.mutate {
                v.0.level = 2;
                v.0.people[5] = Person {
                    tile: [0, -1, 3],
                    word: -1,
                };
            }
            cursor.acknowledge(id).unwrap();
            let before = cursor.clone();
            assert!(cursor.acknowledge(id).is_err());
            assert_eq!(cursor, before);
        }
        assert_eq!(
            serde_json::json!(events),
            row["original"],
            "{}",
            row["input"]
        );
    }
}
