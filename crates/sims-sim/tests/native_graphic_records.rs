use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_sim::pose::{graphic_appearance::Model, graphic_records::*};
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Skin {
    geometry: u32,
    generation: u32,
    dirty: u32,
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_base_record_walks_retain_live_vectors_and_captured_skin() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/native-graphic-records.json")).unwrap();
    for (case, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut vectors: Vectors = serde_json::from_value(c["vectors"].clone()).unwrap();
        let mut skins: BTreeMap<u64, Skin> = serde_json::from_value(c["skins"].clone()).unwrap();
        let mut model = c["model"].as_u64().unwrap();
        let mut update = Update::new(serde_json::from_value(c["entry"].clone()).unwrap());
        let mut counts = BTreeMap::<&str, u64>::new();
        let mut events = vec![];
        let mut applied = vec![];
        let mut freed = vec![];
        loop {
            let Some(request) = update
                .advance(&mut vectors, Some(Model { model, skin: model }))
                .unwrap()
            else {
                break;
            };
            let (restored_u, restored_v) =
                serde_json::from_slice(&serde_json::to_vec(&(&update, &vectors)).unwrap()).unwrap();
            assert_eq!((&update, &vectors), (&restored_u, &restored_v));
            (update, vectors) = (restored_u, restored_v);
            let before = (update.clone(), vectors.clone());
            assert_eq!(
                update.advance(&mut vectors, None).unwrap(),
                Some(request.clone())
            );
            assert_eq!((update.clone(), vectors.clone()), before);
            let (name, mut event) = match request {
                Request::Clear(record) => ("clear", json!({"record":record})),
                Request::Destroy(record) => ("destroy", json!({"record":record})),
                Request::Bind { record, skin } => ("bind", json!({"record":record,"skin":skin})),
                Request::Invalidate { skin } => {
                    let s = skins.get_mut(&skin).unwrap();
                    s.geometry = s.geometry.wrapping_add(1);
                    s.generation = s.generation.wrapping_add(1);
                    s.dirty = 1;
                    update.acknowledge(&mut vectors, &request).unwrap();
                    continue;
                }
            };
            event["service"] = json!(name);
            events.push(event);
            let n = counts.entry(name).or_default();
            for (i, m) in c["mutations"].as_array().unwrap().iter().enumerate() {
                if m["after"] != name || m["occurrence"].as_u64().unwrap_or(0) != *n {
                    continue;
                }
                applied.push(i);
                if let Some(v) = m["vectors"].as_object() {
                    if let Some(g) = v.get("generic") {
                        vectors.generic = serde_json::from_value(g.clone()).unwrap();
                    }
                    if let Some(a) = v.get("accessories") {
                        vectors.accessories = serde_json::from_value(a.clone()).unwrap();
                    }
                }
                if let Some(v) = m["model"].as_u64() {
                    model = v;
                }
                if let Some(values) = m["skins"].as_object() {
                    for (id, value) in values {
                        skins.insert(
                            id.parse().unwrap(),
                            serde_json::from_value(value.clone()).unwrap(),
                        );
                    }
                }
            }
            *n += 1;
            if let Request::Destroy(id) = request {
                freed.push(id);
            }
            let restored: (Update, Vectors) =
                serde_json::from_slice(&serde_json::to_vec(&(&update, &vectors)).unwrap()).unwrap();
            assert_eq!((&update, &vectors), (&restored.0, &restored.1));
            (update, vectors) = restored;
            update.acknowledge(&mut vectors, &request).unwrap();
            let after = (update.clone(), vectors.clone());
            assert!(update.acknowledge(&mut vectors, &request).is_err());
            assert_eq!((update.clone(), vectors.clone()), after);
        }
        assert!(update.complete());
        for (key, actual) in [
            ("vectors", json!(vectors)),
            ("skins", json!(skins)),
            ("model", json!(model)),
            ("events", json!(events)),
            ("applied", json!(applied)),
            ("freed", json!(freed)),
        ] {
            assert_eq!(actual, row["original"][key], "case {case} {key}: {c}");
        }
    }
}

#[test]
fn base_record_walk_rejects_null_binding_and_missing_skin() {
    let mut v = Vectors {
        generic: vec![],
        accessories: vec![None],
    };
    let mut update = Update::new(Entry::BindAccessories);
    let initial = update.clone();
    assert!(update.advance(&mut v, None).is_err());
    assert_eq!(update, initial);
    assert!(
        update
            .advance(&mut v, Some(Model { model: 1, skin: 1 }))
            .is_err()
    );
    let mut destroy = Update::new(Entry::DestroyAccessories);
    assert!(destroy.advance(&mut v, None).unwrap().is_none());
    assert!(v.accessories.is_empty());
}
