use serde_json::{Value, json};
use sims_sim::burn::{self, Execution, Input, Object, Request, Response, Wall, World};
use sims_vm::ObjectId;
use std::collections::BTreeMap;

struct Host {
    case: Value,
    objects: BTreeMap<ObjectId, Object>,
    order: Vec<ObjectId>,
    calls: Vec<Value>,
    allocations: usize,
}

fn number(c: &Value, key: &str, default: i64) -> i64 {
    c[key].as_i64().unwrap_or(default)
}
fn flag(c: &Value, key: &str, default: bool) -> bool {
    c[key].as_bool().unwrap_or(default)
}
fn position(xy: [i32; 2], level: i32) -> [i8; 3] {
    [(xy[1] >> 4) as i8, (xy[0] >> 4) as i8, level as i8]
}
fn obj(c: &Value) -> Object {
    let xy: [i32; 2] =
        serde_json::from_value(c.get("position").cloned().unwrap_or(json!([88, 88]))).unwrap();
    let level = number(c, "level", 1) as i32;
    Object {
        position: xy,
        level,
        tile: position(xy, level),
        direction: number(c, "direction", 0) as i16,
        flags: number(c, "flags", 0) as u16,
        flags2: number(c, "flags2", 0) as u16,
        uses: number(c, "uses", 0) as i16,
        kind: number(c, "type", 0) as i16,
        master: flag(c, "multipart", false),
        group_excluded: flag(c, "exclude_group", false),
        head: c["head"]
            .as_i64()
            .filter(|&v| v != 0)
            .map(|v| v as ObjectId),
        next: c["next"]
            .as_i64()
            .filter(|&v| v != 0)
            .map(|v| v as ObjectId),
        fire_resource: flag(c, "fire", false),
        bounds: serde_json::from_value(c.get("bounds").cloned().unwrap_or(json!([
            xy[0] + 8,
            xy[1] + 8,
            xy[0] - 8,
            xy[1] - 8
        ])))
        .unwrap(),
    }
}

impl Host {
    fn move_object(&mut self, id: ObjectId, xy: [i32; 2], level: i32) {
        let o = self.objects.get_mut(&id).unwrap();
        o.position = xy;
        o.level = level;
        o.tile = position(xy, level);
    }
    fn new(c: &Value) -> Self {
        let mut definitions: BTreeMap<ObjectId, Value> = [
            (1, json!({"position":[24,24]})),
            (2, json!({"position":[88,88]})),
        ]
        .into();
        if let Some(v) = c["objects"].as_object() {
            for (id, o) in v {
                definitions.insert(id.parse().unwrap(), o.clone());
            }
        }
        let order = c
            .get("order")
            .map(|o| serde_json::from_value(o.clone()).unwrap())
            .unwrap_or_else(|| definitions.keys().copied().collect());
        Self {
            case: c.clone(),
            objects: definitions.iter().map(|(&id, c)| (id, obj(c))).collect(),
            order,
            calls: vec![],
            allocations: 0,
        }
    }
    fn service(&mut self, r: Request) -> Response {
        match r {
            Request::Resource => {
                self.calls.push(json!(["resource", burn::FIRE_GUID]));
                Response::Resource(flag(&self.case, "resource", true))
            }
            Request::Reset(id) => {
                self.calls.push(json!(["reset", id]));
                if let Some(value) = self.case.get("move_after_reset") {
                    let [x, y, level]: [i32; 3] = serde_json::from_value(value.clone()).unwrap();
                    self.move_object(id, [x, y], level);
                }
                Response::Done
            }
            Request::Refresh(id) => {
                self.calls.push(json!(["refresh", id]));
                Response::Done
            }
            Request::Factory => {
                let i = self.allocations;
                self.allocations += 1;
                let ok = flag(&self.case, "resource", true)
                    && flag(&self.case, "allocate", true)
                    && number(&self.case, "fail_allocation", -1) != i as i64;
                let id = if ok { 32 + i as ObjectId } else { 0 };
                self.calls.push(json!(["factory", id]));
                if ok {
                    self.objects
                        .insert(id, obj(&json!({"fire":true,"position":[-16,-16]})));
                }
                if let Some(value) = self.case.get("move_after_factory") {
                    let [x, y, level]: [i32; 3] = serde_json::from_value(value.clone()).unwrap();
                    self.move_object(2, [x, y], level);
                }
                Response::Created(ok.then_some(id))
            }
            Request::Check(id, at) => {
                let ok = flag(&self.case, "accept", true)
                    && number(&self.case, "reject_allocation", -1) != i64::from(id - 32);
                self.calls.push(json!(["check", id, at.xy, at.level, ok]));
                if let Some(level) = self.case["level_after_check"].as_i64() {
                    self.move_object(2, self.objects[&2].position, level as i32);
                }
                Response::Accepted(ok)
            }
            Request::Place(id, at) => {
                self.calls.push(json!(["place", id, at.xy, at.level]));
                let o = self.objects.get_mut(&id).unwrap();
                o.position = at.xy;
                o.level = at.level;
                o.tile = position(at.xy, at.level);
                if !self.order.contains(&id) {
                    self.order.push(id);
                }
                Response::Done
            }
            Request::Delete(id) => {
                self.calls.push(json!(["delete", id]));
                self.objects.remove(&id);
                Response::Done
            }
        }
    }
}
impl World for Host {
    fn size(&self) -> usize {
        number(&self.case, "size", 16) as usize
    }
    fn object(&self, id: ObjectId) -> Result<Option<Object>, String> {
        Ok(self.objects.get(&id).cloned())
    }
    fn on_lot(&mut self, id: ObjectId) -> Result<bool, String> {
        self.calls.push(json!(["on_lot", id]));
        Ok(flag(&self.case, "on_lot", true))
    }
    fn tile(&mut self, tile: [i8; 3]) -> Result<Vec<ObjectId>, String> {
        let ids: Vec<_> = self
            .order
            .iter()
            .copied()
            .filter(|id| self.objects[id].tile == tile)
            .collect();
        self.calls.push(json!(["tile", tile, ids]));
        Ok(ids)
    }
    fn wall(&mut self, tile: [i8; 3]) -> Result<Wall, String> {
        self.calls.push(json!(["wall", tile]));
        Ok(Wall {
            mask: number(&self.case, "wall_mask", 0) as u32,
            styles: [number(&self.case, "wall_style", 3) as i16; 6],
        })
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn burn_matches_original_decisions_and_services_across_saved_continuations() {
    let f: Value = serde_json::from_str(include_str!("fixtures/burn.json")).unwrap();
    assert_eq!(f["front_masks"], json!([8, 1, 2, 4]));
    let rows = f["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 784);
    let mut checkpoints = 0;
    for row in rows {
        let c = &row["input"];
        let expected = &row["original"];
        for restore in [false, true] {
            let mut h = Host::new(c);
            let mut e = Execution::new(Input {
                target: number(c, "target", 2) as ObjectId,
                mode: number(c, "mode", 0) as u8,
                flags: number(c, "flags", 0) as u8,
            });
            let mut error = None;
            for i in 0..1000 {
                let r = match e.next(&mut h) {
                    Ok(None) => break,
                    Ok(Some(r)) => r,
                    Err(message) => {
                        assert!(message.contains("native error 0x15"), "{c}: {message}");
                        error = Some(21);
                        break;
                    }
                };
                if restore {
                    let bytes = serde_json::to_vec(&e).unwrap();
                    let saved: Execution = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(e, saved);
                    e = saved;
                    e.validate().unwrap();
                    let mut bad: Value = serde_json::from_slice(&bytes).unwrap();
                    bad["cursor"] = json!(e.affected().len() + 1);
                    assert!(
                        serde_json::from_value::<Execution>(bad)
                            .unwrap()
                            .validate()
                            .is_err()
                    );
                    let mut bad: Value = serde_json::from_slice(&bytes).unwrap();
                    bad["pending"] = json!({"Delete": -1});
                    assert!(
                        serde_json::from_value::<Execution>(bad)
                            .unwrap()
                            .validate()
                            .is_err()
                    );
                    assert_eq!(
                        e.next(&mut h).unwrap(),
                        Some(r.clone()),
                        "pending request must not repeat queries"
                    );
                    checkpoints += 1;
                }
                e.respond(h.service(r)).unwrap();
                e.validate().unwrap();
                assert!(i < 999, "Burn did not finish: {c}");
            }
            assert_eq!(json!(error), expected["error"], "{c}");
            assert_eq!(
                json!(e.result().map(u32::from)),
                expected["returned"],
                "{c}"
            );
            assert_eq!(json!(e.affected()), expected["selected"], "{c}");
            assert_eq!(json!(h.calls), expected["calls"], "{c}");
        }
    }
    eprintln!(
        "{} original Burn cases; {checkpoints} restored service boundaries",
        rows.len()
    );
}
