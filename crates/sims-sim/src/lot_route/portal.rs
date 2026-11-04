//! Imported-lot portal dispatch. Original 0x45c7c2 schedules function 15 before
//! recording an attempt; the yielding target-tree owns all actual door motion.
use crate::{Event, Simulation, contention, portals};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, RouteRequest, Status, ThreadId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Control {
    /// Context+78, cleared by house refresh and consumed on route re-entry.
    #[serde(
        default = "initial_cache_valid",
        skip_serializing_if = "cache_is_valid"
    )]
    pub cache_valid: bool,
    /// Route context +0x94 captures person data 72 before any nested traversal.
    pub ignored: i16,
    pub crossing: bool,
    /// Route context +0x8c; independent of the person linked by a shoo action.
    pub blocked_object: i16,
    pub seated: Option<ObjectId>,
    pub standing: Option<ObjectId>,
}
fn initial_cache_valid() -> bool {
    true
}
fn cache_is_valid(value: &bool) -> bool {
    *value
}

/// State 2 uses the actual room after the script, independently of person68.
/// A false script return is not penalized after a successful room crossing.
fn returned_portal(
    different_rooms: bool,
    success: bool,
    previous: Option<ObjectId>,
    saved_ignored: i16,
    current_ignored: i16,
) -> (Option<ObjectId>, i16) {
    if different_rooms {
        (previous.filter(|_| !success), current_ignored)
    } else {
        (None, saved_ignored)
    }
}

/// 45b140..45b186: only type-8 targets compare against saved route-context
/// +68 portal pointers. An active function-15 service alone is not recursion.
fn recursive_target(
    kind: i16,
    target: ObjectId,
    portals: impl Iterator<Item = Option<ObjectId>>,
) -> bool {
    kind == 8 && portals.into_iter().any(|p| p == Some(target))
}

pub(crate) trait Services {
    fn reset(&mut self) -> Result<bool, String>;
    fn choose(&mut self) -> Result<Option<ObjectId>, String>;
    fn function(&mut self, portal: ObjectId) -> Result<u16, String>;
    fn push(&mut self, portal: ObjectId, bhav: u16) -> Result<bool, String>;
    fn attempt(&mut self, portal: ObjectId) -> Result<(), String>;
}

/// None leaves the context's previous portal untouched. The reset service can
/// assign a person ordinal, even when no door can ultimately be selected.
pub(crate) fn dispatch(
    previous: Option<ObjectId>,
    services: &mut impl Services,
) -> Result<Option<ObjectId>, String> {
    if previous.is_none() && !services.reset()? {
        return Ok(None);
    }
    let Some(portal) = services.choose()? else {
        return Ok(None);
    };
    let bhav = services.function(portal)?;
    if bhav == 0 || !services.push(portal, bhav)? {
        return Ok(None);
    }
    services.attempt(portal)?;
    Ok(Some(portal))
}

struct Host<'a> {
    s: &'a mut Simulation,
    p: &'a Program,
    thread: ThreadId,
    request: &'a RouteRequest,
    portals: Vec<portals::Portal>,
}
impl Services for Host<'_> {
    fn reset(&mut self) -> Result<bool, String> {
        let actor = self.request.caller;
        if self.s.vm.world.objects[&actor].data.get(&6) == Some(&0) {
            // 0x46381a scans the person vector and registered portals for the
            // first unused positive ordinal, independently of object IDs.
            let registry = self
                .s
                .vm
                .world
                .person
                .as_ref()
                .ok_or("portal registry absent")?;
            let used: std::collections::BTreeSet<_> = registry
                .person_order
                .iter()
                .copied()
                .chain(self.portals.iter().map(|p| p.id))
                .map(|id| self.s.vm.world.objects[&id].data[&6])
                .collect();
            let ordinal = (1..=9999).find(|id| !used.contains(id)).unwrap_or(0);
            self.s
                .vm
                .world
                .objects
                .get_mut(&actor)
                .unwrap()
                .data
                .insert(6, ordinal);
        }
        self.s.portal_cache.entry(actor).or_default().reset();
        Ok(true)
    }
    fn choose(&mut self) -> Result<Option<ObjectId>, String> {
        let actor = self.request.caller;
        if self.s.vm.world.objects[&actor].data.get(&6) == Some(&0) {
            return Ok(None);
        }
        let from = self
            .s
            .portal_location(actor)
            .ok_or("portal actor location absent")?;
        let to = self
            .s
            .portal_location(self.request.target)
            .ok_or("portal target location absent")?;
        Ok(self
            .s
            .portal_cache
            .entry(actor)
            .or_default()
            .choose(&self.portals, from, to))
    }
    fn function(&mut self, portal: ObjectId) -> Result<u16, String> {
        let definition = self.s.vm.world.objects[&portal]
            .definition
            .as_ref()
            .ok_or("portal definition absent")?;
        Ok(self.p.modules[&definition.source]
            .object_functions
            .get(&definition.object)
            .and_then(|f| f.get(15))
            .map_or(0, |f| f.action))
    }
    fn push(&mut self, portal: ObjectId, bhav: u16) -> Result<bool, String> {
        let owner = self.s.vm.world.objects[&portal]
            .definition
            .as_ref()
            .unwrap()
            .source
            .clone();
        self.s
            .vm
            .push_native_portal_service(self.p, self.thread, owner, bhav, portal)?;
        Ok(true)
    }
    fn attempt(&mut self, portal: ObjectId) -> Result<(), String> {
        let from = self
            .s
            .portal_location(self.request.caller)
            .ok_or("portal actor location absent")?;
        self.s
            .portal_cache
            .entry(self.request.caller)
            .or_default()
            .attempt(&self.portals, from, portal);
        Ok(())
    }
}

impl Simulation {
    /// True means a script was scheduled or the route completed. False means
    /// same-room destination preparation may start. Nested requests get their
    /// own context while sharing the person's portal cache, as in the original.
    pub(crate) fn service_original_portals(
        &mut self,
        p: &Program,
        thread: ThreadId,
        r: &RouteRequest,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        if !self.route_control.contains_key(&r.id) {
            let actor = &self.vm.world.objects[&r.caller];
            let person = actor
                .person
                .as_ref()
                .ok_or("original route requires a person")?;
            if actor.data[&8] & 8 == 0 {
                return Err("original disabled-person route cleanup adapter required".into());
            }
            let original = Control {
                cache_valid: true,
                ignored: person[&72],
                crossing: false,
                blocked_object: 0,
                seated: None,
                standing: None,
            };
            self.route_control.insert(
                r.id,
                contention::State {
                    thread,
                    caller: r.caller,
                    controller: contention::Controller::new(self.body(r.caller)?.position),
                    link: None,
                    blocker_success: false,
                    callback: None,
                    deciding: false,
                    shoo_result: None,
                    last_portal: None,
                    original: Some(original),
                },
            );
        }
        // A returned synchronous service reaches the route wrapper again.
        // Only its portal state2 clears +68; Stand/Sit/wait callbacks retain it.
        let portal_state = {
            let c = &self.route_control[&r.id];
            let original = c.original.as_ref().unwrap();
            original.crossing
                && original.standing.is_none()
                && original.seated.is_none()
                && c.callback.is_none()
                && !c.deciding
        };
        let resume = super::refresh::resume(
            &mut self
                .route_control
                .get_mut(&r.id)
                .unwrap()
                .original
                .as_mut()
                .unwrap()
                .cache_valid,
            true,
            if portal_state { 2 } else { 1 },
        );
        if let Some(call) = self.vm.threads[&thread].service.as_ref()
            && matches!(&call.resume_status, Status::AwaitingRoute(q) if q.id == r.id)
        {
            let success = call
                .result
                .ok_or("original portal resumed before returning")?;
            if self
                .route_control
                .get_mut(&r.id)
                .unwrap()
                .callback
                .take()
                .is_some()
            {
                self.vm.pop_service(thread)?;
                return self.route_contention(p, thread, r, events);
            }
            if let Some(feedback) = self.route_feedback.remove(&r.id) {
                self.vm.pop_service(thread)?;
                self.person(r.caller, 62, feedback.code)?;
                self.begin_native_route_completion(thread, r, false, i32::from(feedback.code))?;
                return Ok(true);
            }
            if self.route_control[&r.id]
                .original
                .as_ref()
                .unwrap()
                .standing
                == Some(call.callee)
            {
                let seat = call.callee;
                let ignored = self.route_control[&r.id].original.as_ref().unwrap().ignored;
                self.vm.pop_service(thread)?;
                self.route_control
                    .get_mut(&r.id)
                    .unwrap()
                    .original
                    .as_mut()
                    .unwrap()
                    .standing = None;
                self.person(r.caller, 72, ignored)?;
                let posture = self.vm.world.objects[&r.caller].person.as_ref().unwrap()[&0];
                self.event(
                    events,
                    thread,
                    format!("original Stand #{seat} returned {success}; posture {posture}"),
                );
                if !super::departure::returned(success, posture) {
                    return self.fail_route(
                        p,
                        thread,
                        r,
                        6,
                        "original Stand failed to clear posture",
                        events,
                    );
                }
                // State 5 re-enters state 1 and rebuilds departure/candidates.
                return self.service_original_portals(p, thread, r, events);
            }
            if self.route_control[&r.id].original.as_ref().unwrap().seated == Some(call.callee) {
                self.vm.pop_service(thread)?;
                self.route_control
                    .get_mut(&r.id)
                    .unwrap()
                    .original
                    .as_mut()
                    .unwrap()
                    .seated = None;
                if success {
                    self.begin_native_route_completion(thread, r, true, 0)?;
                    return Ok(true);
                }
                let code = self.vm.world.objects[&r.caller].person.as_ref().unwrap()[&62];
                let blocker = self.route_control[&r.id]
                    .original
                    .as_ref()
                    .unwrap()
                    .blocked_object;
                return self.fail_route_blocked(
                    p,
                    thread,
                    r,
                    if code == 0 { 5 } else { code },
                    blocker,
                    "original Sit service returned false",
                    events,
                );
            }
            if self.route_control[&r.id].last_portal != Some(call.callee) {
                return Err("original route portal continuation mismatch".into());
            }
            let portal = call.callee;
            let previous = if resume == super::refresh::Resume::ForgetPortal {
                self.route_control.get_mut(&r.id).unwrap().last_portal = None;
                None
            } else {
                Some(portal)
            };
            // State 2 checks interruption before penalizing a failed portal.
            if self.route_interrupted(thread, r.caller)? {
                self.vm.pop_service(thread)?;
                self.begin_native_route_completion(thread, r, false, 4)?;
                return Ok(true);
            }
            self.vm.pop_service(thread)?;
            let different = self.vm.world.objects[&r.caller].data[&29]
                != self.vm.world.objects[&r.target].data[&29];
            let (failed, ignored) = returned_portal(
                different,
                success,
                previous,
                self.route_control[&r.id].original.as_ref().unwrap().ignored,
                self.vm.world.objects[&r.caller].person.as_ref().unwrap()[&72],
            );
            self.person(r.caller, 72, ignored)?;
            self.event(
                events,
                thread,
                format!(
                    "original portal #{portal} returned {success}; destination room reached: {}",
                    !different
                ),
            );
            if let Some(portal) = failed {
                let nodes = self.portal_nodes();
                if !nodes.iter().any(|p| p.id == portal) {
                    return Err("original returned portal disappeared".into());
                }
                self.portal_cache
                    .entry(r.caller)
                    .or_default()
                    .fail(&nodes, portal);
            }
        }
        if resume == super::refresh::Resume::ForgetPortal {
            self.route_control.get_mut(&r.id).unwrap().last_portal = None;
        }
        if self.route_control[&r.id].deciding {
            return self.route_contention(p, thread, r, events);
        }
        // State 2 checks cancellation before restoring person72 on room arrival,
        // as well as before selecting another portal. State 7 retains the current
        // person72 and completes directly, without failure feedback or placement.
        if self.route_control[&r.id]
            .original
            .as_ref()
            .unwrap()
            .crossing
            && self.route_interrupted(thread, r.caller)?
        {
            self.begin_native_route_completion(thread, r, false, 4)?;
            return Ok(true);
        }
        let actor = &self.vm.world.objects[&r.caller];
        let different = actor.data[&29] != self.vm.world.objects[&r.target].data[&29];
        let control = self.route_control.get_mut(&r.id).unwrap();
        let original = control
            .original
            .as_mut()
            .ok_or("original route context absent")?;
        if original.crossing && !different {
            if actor.data[&8] & 8 == 0 {
                return Err("original disabled-person route cleanup adapter required".into());
            }
            original.crossing = false;
            let ignored = original.ignored;
            self.person(r.caller, 72, ignored)?;
            return Ok(false);
        }
        if !original.crossing {
            if actor.data[&8] & 8 == 0 {
                return Err("original disabled-person route cleanup adapter required".into());
            }
            if !crate::routing::needs_portal(
                actor.data[&29],
                self.vm.world.objects[&r.target].data[&29],
                actor.person.as_ref().unwrap()[&68],
            ) {
                // State 1 has the same gate before same-room candidate search.
                if self.route_interrupted(thread, r.caller)? {
                    self.begin_native_route_completion(thread, r, false, 4)?;
                    return Ok(true);
                }
                return Ok(false);
            }
            original.crossing = true;
            // State 1 checks actual context +68 pointers before entering state
            // 2. It enters ordinary state-7 failure cleanup (not feedback), with
            // result 0 normalized to 1 by that shared completion controller.
            let definition = self.vm.world.objects[&r.target]
                .definition
                .as_ref()
                .ok_or("recursive route target definition absent")?;
            let kind = self
                .architecture
                .as_ref()
                .and_then(|a| a.footprints.get(&definition.guid))
                .ok_or("recursive route target kind absent")?
                .kind;
            if recursive_target(
                kind,
                r.target,
                self.route_control
                    .values()
                    .filter(|c| c.caller == r.caller && c.original.is_some())
                    .map(|c| c.last_portal),
            ) {
                self.begin_native_route_completion(thread, r, false, 0)?;
                return Ok(true);
            }
        }
        if self.route_interrupted(thread, r.caller)? {
            self.begin_native_route_completion(thread, r, false, 4)?;
            return Ok(true);
        }
        let previous = self.route_control[&r.id].last_portal;
        let nodes = self.portal_nodes();
        let accepted = dispatch(
            previous,
            &mut Host {
                s: self,
                p,
                thread,
                request: r,
                portals: nodes,
            },
        )?;
        if let Some(portal) = accepted {
            self.route_control.get_mut(&r.id).unwrap().last_portal = Some(portal);
            // 0x45b962 clears person data 72 before yielding into function 15.
            self.person(r.caller, 72, 0)?;
            self.event(
                events,
                thread,
                format!("original-lot route invokes portal #{portal} function 15"),
            );
            Ok(true)
        } else {
            self.fail_route(p, thread, r, 2, "no original portal traversal", events)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn returns_match_original_room_and_failure_decisions() {
        let report: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/portal-resume.json")).unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let different = c["different"].as_bool().unwrap();
            let (failed, ignored) = returned_portal(
                different,
                c["success"].as_bool().unwrap(),
                c["previous"].as_bool().unwrap().then_some(2),
                c["ignored"].as_i64().unwrap() as i16,
                99,
            );
            assert_eq!(
                json!({"portal":different,"failed":failed.map(|_| "previous").into_iter().collect::<Vec<_>>(), "person72":ignored}),
                row["original"],
                "{c}"
            );
        }
    }
    struct Fixture<'a> {
        input: &'a Value,
        events: Vec<&'static str>,
    }
    impl Services for Fixture<'_> {
        fn reset(&mut self) -> Result<bool, String> {
            self.events.push("reset");
            Ok(self.input["reset"].as_bool().unwrap())
        }
        fn choose(&mut self) -> Result<Option<i16>, String> {
            self.events.push("choose");
            Ok(self.input["chosen"].as_bool().unwrap().then_some(2))
        }
        fn function(&mut self, _: i16) -> Result<u16, String> {
            self.events.extend(["resource", "function15"]);
            Ok(self.input["bhav"].as_u64().unwrap() as u16)
        }
        fn push(&mut self, _: i16, _: u16) -> Result<bool, String> {
            self.events.push("push");
            Ok(self.input["push"].as_bool().unwrap())
        }
        fn attempt(&mut self, _: i16) -> Result<(), String> {
            self.events.push("attempt");
            Ok(())
        }
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn dispatch_matches_original_guards_and_side_effect_order() {
        let report: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/portal-dispatch.json"))
                .unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let previous = c["previous"].as_bool().unwrap().then_some(1);
            let mut host = Fixture {
                input: c,
                events: vec![],
            };
            let result = dispatch(previous, &mut host).unwrap();
            let last = result
                .or(previous)
                .map(|id| if id == 2 { "chosen" } else { "previous" });
            assert_eq!(
                json!({"events":host.events,"success":result.is_some(),"last":last}),
                row["original"],
                "{c}"
            );
        }
    }
}

#[cfg(test)]
mod recursive_tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn native_recursive_guard_is_only_a_state_one_type_eight_context_scan() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/recursive-portal-target.json"
        ))
        .unwrap();
        for row in fixture["cases"].as_array().unwrap() {
            let c = &row["input"];
            let contexts = (0..c["depth"].as_i64().unwrap())
                .map(|i| (i == c["match"].as_i64().unwrap()).then_some(52));
            let result = c["state"] == 1
                && recursive_target(c["kind"].as_i64().unwrap() as i16, 52, contexts);
            assert_eq!(
                result,
                row["original"]["recursive"].as_bool().unwrap(),
                "{c}"
            );
            if result {
                assert_eq!(row["original"]["interruption_queries"], 0);
                assert_eq!(row["original"]["effects"]["context_result"], 1);
                assert_eq!(
                    row["original"]["effects"]["events"],
                    serde_json::json!(["context_pop"])
                );
            }
        }
        // An active portal service whose route context has no +68 pointer
        // is not recursive. Only the passed context records are inspected.
        assert!(!recursive_target(8, 52, [None, Some(7)].into_iter()));
    }
}
