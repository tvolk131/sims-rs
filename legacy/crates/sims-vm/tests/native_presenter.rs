use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_vm::{ObjectId, presenter::*, refresh::DrawState};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Object {
    person: bool,
    graphics: i16,
    group: i16,
    flags: u16,
    parent: i16,
    word34: i16,
    level: i32,
    tile: [i8; 3],
    ready: u8,
    keep: u8,
    draw: [u32; 3],
    slots: Vec<Option<ObjectId>>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Host {
    visible: i32,
    force: u8,
    clock: u32,
    order: Vec<ObjectId>,
    objects: Vec<Object>,
}
impl Host {
    fn object(&self, id: ObjectId) -> &Object {
        &self.objects[id as usize - 1]
    }
    fn object_mut(&mut self, id: ObjectId) -> &mut Object {
        &mut self.objects[id as usize - 1]
    }
    fn draws(&self) -> Vec<[u32; 3]> {
        self.objects.iter().map(|o| o.draw).collect()
    }
}
impl DrawView for Host {
    fn slots(&self, id: ObjectId) -> Result<&[Option<ObjectId>], String> {
        Ok(&self.object(id).slots)
    }
    fn exists(&self, id: ObjectId) -> bool {
        id > 0 && id as usize <= self.objects.len()
    }
    fn definition_flags(&self, id: ObjectId) -> Result<u16, String> {
        Ok(self.object(id).flags)
    }
    fn force_dynamic(&self) -> bool {
        self.force != 0
    }
    fn visible_level(&self) -> i32 {
        self.visible
    }
    fn clock(&self) -> u32 {
        self.clock
    }
    fn draw(&self, id: ObjectId) -> Result<DrawState, String> {
        let o = self.object(id);
        Ok(DrawState {
            level: o.level,
            dynamic: o.draw[0] != 0,
            expiry: o.draw[1] as i32,
            refreshed_at: o.draw[2],
        })
    }
    fn write_draw(&mut self, id: ObjectId, d: DrawState) -> Result<(), String> {
        self.object_mut(id).draw = [u32::from(d.dynamic), d.expiry as u32, d.refreshed_at];
        Ok(())
    }
}
impl DurationView for Host {
    fn is_person(&self, id: ObjectId) -> Result<bool, String> {
        Ok(self.object(id).person)
    }
    fn keep_dynamic(&self, id: ObjectId) -> Result<bool, String> {
        Ok(self.object(id).keep != 0)
    }
}
impl ClassificationView for Host {
    fn order(&self) -> &[ObjectId] {
        &self.order
    }
    fn word34(&self, id: ObjectId) -> Result<i16, String> {
        Ok(self.object(id).word34)
    }
    fn graphic_ready(&self, id: ObjectId) -> Result<bool, String> {
        let o = self.object(id);
        Ok(o.person || o.ready != 0)
    }
    fn has_drawings(&self, id: ObjectId) -> Result<bool, String> {
        let o = self.object(id);
        Ok(o.graphics != 0 || o.group != 0)
    }
    fn parent(&mut self, id: ObjectId) -> Result<Option<ObjectId>, String> {
        let p = self.object(id).parent;
        Ok((p != 0).then_some(p))
    }
    fn duration(&self, id: ObjectId) -> Result<i32, String> {
        duration(self, id)
    }
    fn outside_interior(&self, id: ObjectId) -> Result<bool, String> {
        Ok(outside(self.object(id).tile, 64))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Run {
    host: Host,
    classify: Option<Classify>,
    setter: Option<DrawUpdate>,
    lists: Lists,
    events: Vec<Value>,
    applied: Vec<usize>,
}
impl Run {
    fn restore(&mut self) {
        let saved = serde_json::to_vec(self).unwrap();
        let restored: Self = serde_json::from_slice(&saved).unwrap();
        if let Some(c) = &restored.classify {
            c.validate().unwrap();
        }
        if let Some(d) = &restored.setter {
            d.validate().unwrap();
        }
        assert_eq!(restored, *self);
        *self = restored;
    }
    fn invalidation(&mut self, id: ObjectId, c: &Value) {
        self.events
            .push(json!({"object":id,"draw":self.host.draws()}));
        for (n, change) in c["changes"].as_array().into_iter().flatten().enumerate() {
            if self.applied.contains(&n) || change["at"] != json!(self.events.len()) {
                continue;
            }
            self.applied.push(n);
            if let Some(clock) = change["clock"].as_u64() {
                self.host.clock = clock as u32;
            }
            if let Some(id) = change["object"].as_i64() {
                let o = self.host.object_mut(id as i16);
                if let Some(slots) = change["slots"].as_array() {
                    o.slots = slots
                        .iter()
                        .map(|v| v.as_i64().filter(|v| *v != 0).map(|v| v as i16))
                        .collect();
                }
                if let Some(v) = change["word34"].as_i64() {
                    o.word34 = v as i16;
                }
                if let Some(v) = change["flags"].as_u64() {
                    o.flags = v as u16;
                }
            }
        }
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_classification_callbacks_and_resumption() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/presenter-classification.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(report["cases"].as_array().unwrap().len(), 252);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut input = c.clone();
        for object in input["objects"].as_array_mut().unwrap() {
            for id in object["slots"].as_array_mut().unwrap() {
                if *id == 0 {
                    *id = Value::Null;
                }
            }
        }
        let mut run = Run {
            host: serde_json::from_value(input).unwrap(),
            classify: (c["mode"] == "classify").then(Classify::default),
            setter: (c["mode"] == "setter").then(|| {
                DrawUpdate::new(
                    c["root"].as_i64().unwrap() as i16,
                    c["dynamic"] != 0,
                    c["no_children"] != 0,
                )
            }),
            lists: Lists {
                dynamic: vec![4],
                ordinary: vec![4],
                special: vec![4],
            },
            events: vec![],
            applied: vec![],
        };
        loop {
            run.restore();
            if let Some(setter) = &mut run.setter {
                if let Some(id) = setter.advance(&mut run.host).unwrap() {
                    run.restore();
                    let before = run.clone();
                    assert_eq!(
                        run.setter.as_mut().unwrap().advance(&mut run.host).unwrap(),
                        Some(id)
                    );
                    assert!(run.setter.as_mut().unwrap().acknowledge(99).is_err());
                    assert_eq!(run, before);
                    run.invalidation(id, c);
                    run.restore();
                    run.setter.as_mut().unwrap().acknowledge(id).unwrap();
                    let before = run.clone();
                    assert!(run.setter.as_mut().unwrap().acknowledge(id).is_err());
                    assert_eq!(run, before);
                    continue;
                }
                assert!(setter.complete());
                run.setter = None;
                if let Some(classify) = &mut run.classify {
                    let request = classify.pending().unwrap();
                    classify.acknowledge(request, &mut run.lists).unwrap();
                }
            }
            let Some(classify) = &mut run.classify else {
                break;
            };
            let Some(request) = classify.advance(&mut run.host, &mut run.lists).unwrap() else {
                assert!(classify.complete());
                break;
            };
            run.restore();
            let before = run.clone();
            assert_eq!(
                run.classify
                    .as_mut()
                    .unwrap()
                    .advance(&mut run.host, &mut run.lists)
                    .unwrap(),
                Some(request)
            );
            assert!(
                run.classify
                    .as_mut()
                    .unwrap()
                    .acknowledge(
                        Dynamic {
                            object: request.object,
                            enabled: !request.enabled
                        },
                        &mut run.lists
                    )
                    .is_err()
            );
            assert_eq!(run, before);
            run.setter = Some(DrawUpdate::new(request.object, request.enabled, false));
        }
        run.restore();
        assert_eq!(
            json!({"draw":run.host.draws(),"lists":[run.lists.dynamic,run.lists.ordinary,run.lists.special],"events":run.events,"applied":run.applied}),
            row["original"],
            "case {n}: {c}"
        );
    }
}

#[test]
fn malformed_presenter_continuations_are_rejected() {
    for (path, value) in [("root", json!(0)), ("frames", json!([]))] {
        let mut state = serde_json::to_value(DrawUpdate::new(1, true, false)).unwrap();
        state[path] = value;
        let draw: DrawUpdate = serde_json::from_value(state).unwrap();
        if path == "root" {
            assert!(draw.validate().is_err());
        } else {
            assert!(draw.complete());
        }
    }
    let mut state = serde_json::to_value(Classify::default()).unwrap();
    state["pending"] = json!({"object":1,"enabled":true});
    assert!(
        serde_json::from_value::<Classify>(state)
            .unwrap()
            .validate()
            .is_err()
    );
    let mut state = serde_json::to_value(DrawUpdate::new(1, true, false)).unwrap();
    let frame = state["frames"][0].clone();
    state["frames"].as_array_mut().unwrap().push(frame);
    assert!(
        serde_json::from_value::<DrawUpdate>(state)
            .unwrap()
            .validate()
            .is_err()
    );
}

#[cfg(feature = "private-fixtures")]
#[test]
fn classification_owner_matches_native_and_rejects_orphan_children() {
    let report: Value =
        serde_json::from_str(include_str!("fixtures/presenter-classification.json")).unwrap();
    let mut boundaries = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        if c["mode"] != "classify" {
            continue;
        }
        let mut input = c.clone();
        for object in input["objects"].as_array_mut().unwrap() {
            for id in object["slots"].as_array_mut().unwrap() {
                if *id == 0 {
                    *id = Value::Null;
                }
            }
        }
        let mut run = Run {
            host: serde_json::from_value(input).unwrap(),
            classify: None,
            setter: None,
            lists: Lists::default(),
            events: vec![],
            applied: vec![],
        };
        let mut owner = Classification::default();
        while let Some(id) = owner.advance(&mut run.host, &mut run.lists).unwrap() {
            boundaries += 1;
            owner = serde_json::from_slice(&serde_json::to_vec(&owner).unwrap()).unwrap();
            owner.validate().unwrap();
            run.restore();
            let saved = owner.clone();
            let before = run.clone();
            assert_eq!(
                owner.advance(&mut run.host, &mut run.lists).unwrap(),
                Some(id)
            );
            assert_eq!(owner, saved);
            assert_eq!(run, before);
            for change in 0..4 {
                let mut corrupt = serde_json::to_value(&owner).unwrap();
                match change {
                    0 => corrupt["draw"] = Value::Null,
                    1 => corrupt["classify"]["pending"] = Value::Null,
                    2 => {
                        corrupt["draw"]["requested"] =
                            json!(!corrupt["draw"]["requested"].as_bool().unwrap())
                    }
                    _ => corrupt["draw"]["frames"] = json!([]),
                }
                assert!(
                    serde_json::from_value::<Classification>(corrupt)
                        .unwrap()
                        .validate()
                        .is_err()
                );
            }
            run.invalidation(id, c);
            owner.acknowledge(id).unwrap();
            owner = serde_json::from_slice(&serde_json::to_vec(&owner).unwrap()).unwrap();
            owner.validate().unwrap();
        }
        assert!(owner.complete());
        assert_eq!(
            json!({"draw":run.host.draws(),"lists":[run.lists.dynamic,run.lists.ordinary,run.lists.special],"events":run.events,"applied":run.applied}),
            row["original"]
        );
    }
    assert!(boundaries > 0);
}
