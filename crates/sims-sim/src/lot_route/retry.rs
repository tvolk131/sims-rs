//! Imported route failure handoff and the readable 0x45ba28 shoo prefix.
//! Temporary-path queries and GoHere creation use the imported physical host.
//! Protected geometry/search remains gated by the explicit inference policy.
use crate::{Point, Simulation, contention::ObstacleGate};
use sims_vm::{ObjectId, RouteRequest};
use std::collections::BTreeSet;

/// The obstacle gate executes before the posture test. Registry order is kept;
/// any registered person's route context can already own the prospective blocker.
fn eligible_people(
    caller: ObjectId,
    posture: i16,
    order: &[ObjectId],
    linked: &BTreeSet<ObjectId>,
    mut ignored: impl FnMut(ObjectId) -> Result<bool, String>,
) -> Result<Vec<ObjectId>, String> {
    let mut people = vec![];
    for &id in order {
        if id != caller && !ignored(id)? && posture == 0 && !linked.contains(&id) {
            people.push(id);
        }
    }
    Ok(people)
}

impl Simulation {
    pub(crate) fn start_original_shoo(&mut self, r: &RouteRequest) -> Result<bool, String> {
        let order = self
            .vm
            .world
            .person
            .as_ref()
            .ok_or("native shoo registry absent")?
            .person_order
            .clone();
        let linked = self
            .route_control
            .values()
            .filter(|s| order.contains(&s.caller))
            .filter_map(|s| s.link.as_ref().map(|l| l.blocker))
            .collect();
        let actor = self.vm.world.positions[&r.caller].clone();
        let posture = self.vm.world.objects[&r.caller]
            .person
            .as_ref()
            .ok_or("native shoo caller is not person")?[&0];
        let people = eligible_people(r.caller, posture, &order, &linked, |id| {
            let contained = self
                .native_lot
                .as_ref()
                .unwrap()
                .placement
                .links
                .capture(&mut self.vm.world, id)?
                .container
                .is_some();
            let o = &self.vm.world.objects[&id];
            let other = &self.vm.world.positions[&id];
            Ok(crate::contention::obstacle_gate(
                contained,
                o.data[&8] as u16,
                o.data[&10],
                true,
                false,
                Point::new(other.x - actor.x, other.y - actor.y),
                other.level - actor.level,
            ) == ObstacleGate::Ignore)
        })?;
        if people.is_empty() {
            return Ok(false);
        }
        let native = self.native_lot.as_mut().unwrap();
        if native.shoo.is_some() {
            return Err("overlapping native shoo query".into());
        }
        native.shoo = Some(super::shoo::Execution {
            thread: self.route_control[&r.id].thread,
            route: r.clone(),
            selection: super::shoo::Selection::new(people),
            ignored_word: self.vm.world.objects[&r.caller].person.as_ref().unwrap()[&72],
            blocked_object: 0,
            retained_result: 0,
            candidate_limit: Some(48),
            query: None,
            attempt: None,
        });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_person_scan_preserves_order_and_cross_context_exclusions() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/shoo-people.json")).unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let list = |v: &serde_json::Value| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_i64().unwrap() as i16)
                    .collect::<Vec<_>>()
            };
            let order = list(&c["order"]);
            let ignored = list(&c["ignored"]);
            let linked = order
                .iter()
                .flat_map(|&id| list(&c["links"][id as usize - 1]))
                .filter(|&id| id != 0)
                .collect();
            let mut checked = vec![];
            let eligible = eligible_people(
                1,
                c["posture"].as_i64().unwrap() as i16,
                &order,
                &linked,
                |id| {
                    checked.push(id);
                    Ok(ignored.contains(&id))
                },
            )
            .unwrap();
            assert_eq!(
                serde_json::json!(eligible),
                row["original"]["eligible"],
                "{row}"
            );
            assert_eq!(
                serde_json::json!(checked),
                row["original"]["checked"],
                "{row}"
            );
            assert_eq!(
                !eligible.is_empty(),
                row["original"]["path_context"] == true
            );
            if eligible.is_empty() {
                assert_eq!(row["original"]["returned"], false);
            }
        }
    }

    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_endpoint_failure_enters_retry_without_inventing_a_facing() {
        use crate::contention::{Controller, Decision, Phase};
        let report: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/route-retry-handoff.json"
        ))
        .unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            let xy = |v: &serde_json::Value| serde_json::from_value::<[i32; 2]>(v.clone()).unwrap();
            let path: Vec<[i32; 2]> = serde_json::from_value(c["path"].clone()).unwrap();
            let selected = c["room"] == true && c["search"] == true;
            let look = if selected {
                crate::routing::endpoint_look(
                    &path,
                    xy(&c["target"]),
                    c["rotation"].as_u64().unwrap() as u8,
                    c["flags"].as_u64().unwrap() as u32,
                    c["facing"].as_i64().unwrap() as i32,
                    c["actor_facing"].as_u64().unwrap() as u8,
                )
                .unwrap()
            } else {
                None
            };
            assert_eq!(serde_json::json!(look), n["look"], "{row}");
            assert_eq!(if look.is_some() { 2 } else { 0 }, n["endpoint"]);
            assert_eq!(if selected { 64 } else { 0 }, n["entry_flags"]);
            // A coincident look returns early; other failures clear candidates.
            assert_eq!(!selected || path.len() < 2, n["cleared"] == true);
            if look.is_some() {
                assert_eq!(n["decision"], "walk");
                continue;
            }
            let a = xy(&c["previous"]);
            let b = xy(&c["current"]);
            let mut controller = Controller::new(Point::new(a[1], a[0]));
            controller.stage = c["stage"].as_i64().unwrap() as i16;
            controller.result = c["result"].as_i64().unwrap() as i16;
            let mut asked = false;
            let decision = controller.decide(
                Point::new(b[1], b[0]),
                100,
                false,
                false,
                false,
                || {
                    asked = true;
                    false
                },
                |_| false,
            );
            assert_eq!(controller.phase, Phase::Retry);
            assert_eq!(
                controller.stage,
                n["stage"].as_i64().unwrap() as i16,
                "{row}"
            );
            assert_eq!(asked, n["asked"] == true);
            assert_eq!(
                match decision {
                    Decision::Replan => "replan",
                    Decision::Fail { .. } => "fail",
                    _ => "callback",
                },
                n["decision"]
            );
        }
    }
}
