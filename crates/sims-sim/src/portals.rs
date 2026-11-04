//! Original portal distance cache and traversal history (0x46390f–0x463f4e).
//! Pairing and room membership are supplied by architecture; path costs and
//! attempt/failure sentinels follow the original engine, including stable ties.
use crate::Point;
use serde::{Deserialize, Serialize};
use sims_vm::ObjectId;
use std::collections::BTreeMap;

const MAX_COST: f32 = 1000.;
const FIRST_ATTEMPT: f32 = 1001.;
const LAST_ATTEMPT: f32 = 1100.;
const BLOCKED: f32 = 1101.;
const UNREACHABLE: f32 = 10000.;
const UNKNOWN: f32 = 10001.;

#[derive(Debug, Clone, Copy)]
pub struct Location {
    pub position: Point,
    pub level: i32,
    pub room: i16,
    /// Native kind 7 destinations have zero distance (0x434148).
    pub zero_distance: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct Portal {
    pub id: ObjectId,
    pub pair: ObjectId,
    pub location: Location,
}
/// Inputs read by original partner lookup 0x4636ae. Style one pairs matching
/// base-graphic definitions; other styles follow the multipart ring until a
/// part with function 15 is reached. A traversal bound rejects corrupt links.
#[derive(Debug, Clone, Copy)]
pub struct PairPart {
    pub id: ObjectId,
    pub head: Option<ObjectId>,
    pub next: Option<ObjectId>,
    pub style: i16,
    pub base_graphic: i16,
    pub portal: bool,
}
pub fn partner(parts: &[PairPart], start: ObjectId) -> Option<ObjectId> {
    let initial = parts.iter().find(|p| p.id == start)?;
    let mut current = initial;
    for _ in 0..parts.len() {
        let next = if initial.style == 1 {
            let mut candidate = parts
                .iter()
                .find(|p| p.id == current.head.unwrap_or(current.id))?;
            let mut found = None;
            for _ in 0..parts.len() {
                if candidate.id != current.id && candidate.base_graphic == current.base_graphic {
                    found = Some(candidate.id);
                    break;
                }
                let Some(id) = candidate.next else { break };
                if id == current.head.unwrap_or(current.id) {
                    break;
                }
                candidate = parts.iter().find(|p| p.id == id)?;
            }
            found
        } else {
            current.next.or(current.head)
        }?;
        if next == start {
            return None;
        }
        current = parts.iter().find(|p| p.id == next)?;
        if current.portal {
            return Some(next);
        }
    }
    None
}
/// Coordinates are sixteenths of a tile; levels remain separate from local
/// height, so a chair seat and an upstairs floor cannot alias each other.
pub fn distance(a: Location, b: Location) -> f64 {
    if b.zero_distance {
        return 0.;
    }
    let dx = f64::from(a.position.x - b.position.x);
    let dy = f64::from(a.position.y - b.position.y);
    let dz = 20. * f64::from(a.level - b.level);
    ((dx * dx + dy * dy) / 256. + dz * dz).sqrt()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Cache {
    /// Original per-person column in the portal objects' cost vectors. A new
    /// outer route resets this column; nested same-room routes leave it alone.
    pub costs: BTreeMap<ObjectId, f32>,
}
impl Cache {
    pub fn reset(&mut self) {
        self.costs.clear();
    }
    /// 46387c preserves the allocated columns while invalidating every cost.
    pub fn invalidate(&mut self) {
        for cost in self.costs.values_mut() {
            *cost = UNKNOWN;
        }
    }
    fn get(&self, id: ObjectId) -> f32 {
        self.costs.get(&id).copied().unwrap_or(UNKNOWN)
    }
    pub fn choose(
        &mut self,
        portals: &[Portal],
        actor: Location,
        target: Location,
    ) -> Option<ObjectId> {
        let mut best = None;
        for entry in portals.iter().filter(|p| p.location.room == actor.room) {
            if self.get(entry.id) == UNKNOWN {
                self.costs.insert(entry.id, UNREACHABLE);
                let approach = distance(actor, entry.location) as f32;
                let from_entry = distances(portals, entry.id);
                let from_pair = distances(portals, entry.pair);
                for exit in portals.iter().filter(|p| p.location.room == target.room) {
                    let through = from_entry[&exit.id];
                    if through != UNREACHABLE && from_pair[&exit.id] <= through {
                        let cost = ((distance(exit.location, target)
                            + f64::from(through)
                            + f64::from(approach)) as f32)
                            .min(MAX_COST);
                        if cost < self.get(entry.id) {
                            self.costs.insert(entry.id, cost);
                        }
                    }
                }
            }
            let cost = self.get(entry.id);
            if cost != UNREACHABLE && cost != BLOCKED && best.is_none_or(|id| cost < self.get(id)) {
                best = Some(entry.id);
            }
        }
        best
    }
    pub fn fail(&mut self, portals: &[Portal], id: ObjectId) {
        self.set_pair(portals, id, BLOCKED);
    }
    pub fn attempt(&mut self, portals: &[Portal], actor: Location, id: ObjectId) {
        let cost = self.get(id);
        if (FIRST_ATTEMPT..=LAST_ATTEMPT).contains(&cost) {
            self.set_pair(portals, id, BLOCKED);
        } else if cost != BLOCKED {
            let mut next = FIRST_ATTEMPT;
            for portal in portals.iter().filter(|p| p.location.room == actor.room) {
                let value = self.get(portal.id);
                if (FIRST_ATTEMPT..=LAST_ATTEMPT).contains(&value) && next <= value {
                    next = value + 1.;
                }
            }
            self.set_pair(portals, id, next);
        }
    }
    fn set_pair(&mut self, portals: &[Portal], id: ObjectId, cost: f32) {
        self.costs.insert(id, cost);
        self.costs
            .insert(portals.iter().find(|p| p.id == id).unwrap().pair, cost);
    }
    pub fn validate(&self, portals: &[Portal]) -> Result<(), String> {
        for (&id, &cost) in &self.costs {
            if !portals.iter().any(|p| p.id == id)
                || !cost.is_finite()
                || !((0. ..=BLOCKED).contains(&cost) || cost == UNREACHABLE || cost == UNKNOWN)
            {
                return Err("invalid native portal cache".into());
            }
        }
        Ok(())
    }
}

fn distances(portals: &[Portal], start: ObjectId) -> BTreeMap<ObjectId, f32> {
    let mut costs: BTreeMap<_, _> = portals.iter().map(|p| (p.id, UNREACHABLE)).collect();
    costs.insert(start, 0.);
    let mut pending = vec![start];
    while !pending.is_empty() {
        // The original scans backwards with a strict comparison: the last
        // queued item wins equal-cost ties, while portal selection prefers first.
        let mut next = pending.len() - 1;
        for i in (0..next).rev() {
            if costs[&pending[i]] < costs[&pending[next]] {
                next = i;
            }
        }
        let id = pending.remove(next);
        let from = portals.iter().find(|p| p.id == id).unwrap();
        for to in portals {
            if (to.location.room == from.location.room || to.id == from.pair)
                && costs[&id] < costs[&to.id]
            {
                let cost = distance(from.location, to.location) + f64::from(costs[&id]) + 1.;
                if cost < f64::from(costs[&to.id]) {
                    costs.insert(to.id, cost as f32);
                    pending.push(to.id);
                }
            }
        }
    }
    costs
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn location(v: &Value) -> Location {
        Location {
            position: Point::new(
                v["position"][0].as_i64().unwrap() as i32,
                v["position"][1].as_i64().unwrap() as i32,
            ),
            level: v["level"].as_i64().unwrap_or(1) as i32,
            room: v["room"].as_i64().unwrap() as i16,
            zero_distance: false,
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn matches_original_portal_graph_and_history() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/portals.json")).unwrap();
        for (case, row) in fixture["cases"].as_array().unwrap().iter().enumerate() {
            let c = &row["input"];
            let portals: Vec<_> = c["portals"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, p)| Portal {
                    id: i as ObjectId,
                    pair: p["pair"].as_i64().unwrap() as ObjectId,
                    location: location(p),
                })
                .collect();
            let mut cache = Cache::default();
            let mut actor = location(&c["actor"]);
            let target = location(&c["target"]);
            for (step, op) in c["operations"].as_array().unwrap().iter().enumerate() {
                let chosen = match op["kind"].as_str().unwrap() {
                    "choose" => cache.choose(&portals, actor, target),
                    "attempt" => {
                        cache.attempt(&portals, actor, op["portal"].as_i64().unwrap() as ObjectId);
                        None
                    }
                    "fail" => {
                        cache.fail(&portals, op["portal"].as_i64().unwrap() as ObjectId);
                        None
                    }
                    "reset" => {
                        cache.reset();
                        None
                    }
                    "move" => {
                        actor = location(op);
                        None
                    }
                    _ => unreachable!(),
                };
                let expected = &row["original"][step];
                assert_eq!(
                    chosen,
                    expected["chosen"].as_i64().map(|n| n as ObjectId),
                    "case {case} step {step}"
                );
                for (i, value) in expected["costs"].as_array().unwrap().iter().enumerate() {
                    assert_eq!(
                        cache.get(i as ObjectId),
                        value.as_f64().unwrap() as f32,
                        "case {case} step {step} portal {i}"
                    );
                }
                cache.validate(&portals).unwrap();
            }
        }
    }
}
