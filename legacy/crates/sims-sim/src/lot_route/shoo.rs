//! Original 0x45ba28: per-person potential paths, then the all-person fallback.
//! Path results and GoHere attempts are synchronous services of the lot host.
use serde::{Deserialize, Serialize};
use sims_vm::ObjectId;
mod attempt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    Path {
        ignored: Option<ObjectId>,
        ignore_people: bool,
    },
    Attempt {
        blocker: ObjectId,
        points: Vec<[i32; 2]>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Selection {
    people: Vec<ObjectId>,
    cursor: usize,
    pending: Option<Request>,
    result: Option<bool>,
}
impl Selection {
    fn validate(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        if self.people.is_empty()
            || self.cursor > self.people.len()
            || self.result.is_some()
            || self.people.iter().any(|id| *id <= 0 || !seen.insert(*id))
            || match &self.pending {
                None => false,
                Some(Request::Path {
                    ignored,
                    ignore_people,
                }) => {
                    *ignored != self.people.get(self.cursor).copied()
                        || *ignore_people != (self.cursor == self.people.len())
                }
                Some(Request::Attempt { blocker, .. }) => {
                    !self.people.contains(blocker)
                        || self.cursor < self.people.len() && self.people[self.cursor] != *blocker
                }
            }
        {
            return Err("invalid native shoo selection".into());
        }
        Ok(())
    }
    pub fn new(people: Vec<ObjectId>) -> Self {
        Self {
            result: people.is_empty().then_some(false),
            people,
            cursor: 0,
            pending: None,
        }
    }
    pub fn outcome(&self) -> Option<bool> {
        self.result
    }
    pub fn next(&mut self) -> Option<Request> {
        if self.result.is_some() {
            return None;
        }
        Some(
            self.pending
                .get_or_insert_with(|| Request::Path {
                    ignored: self.people.get(self.cursor).copied(),
                    ignore_people: self.cursor == self.people.len(),
                })
                .clone(),
        )
    }
    fn advance(&mut self) {
        self.pending = None;
        if self.cursor == self.people.len() {
            self.result = Some(false);
        } else {
            self.cursor += 1;
        }
    }
    pub fn path_result(
        &mut self,
        points: Option<Vec<[i32; 2]>>,
        mut bounds: impl FnMut(ObjectId) -> Result<[i32; 4], String>,
    ) -> Result<(), String> {
        let Some(Request::Path {
            ignored,
            ignore_people,
        }) = self.pending
        else {
            return Err("shoo path result without pending query".into());
        };
        let Some(points) = points else {
            self.advance();
            return Ok(());
        };
        let blocker = if ignore_people {
            first_blocker(&points, &self.people, &mut bounds)?
        } else {
            ignored
        };
        match blocker {
            Some(blocker) => self.pending = Some(Request::Attempt { blocker, points }),
            None => self.advance(),
        }
        Ok(())
    }
    pub fn attempt_result(&mut self, accepted: bool) -> Result<(), String> {
        if !matches!(self.pending, Some(Request::Attempt { .. })) {
            return Err("shoo attempt result without pending target".into());
        }
        if accepted {
            self.pending = None;
            self.result = Some(true);
        } else {
            self.advance();
        }
        Ok(())
    }
}

/// Native fallback samples every segment with +0.5 followed by truncation;
/// the first containing rectangle wins in captured person-registry order.
/// Bounds are native max X, max Y, min X, min Y, with exclusive upper edges.
fn first_blocker(
    points: &[[i32; 2]],
    people: &[ObjectId],
    bounds: &mut impl FnMut(ObjectId) -> Result<[i32; 4], String>,
) -> Result<Option<ObjectId>, String> {
    for segment in points.windows(2) {
        let [x, y] = segment[0];
        let dx = i64::from(segment[1][0]) - i64::from(x);
        let dy = i64::from(segment[1][1]) - i64::from(y);
        let steps = dx.abs().max(dy.abs()) + 1;
        if steps > 1_000_000 {
            return Err("shoo path segment outside native lot range".into());
        }
        let mut fraction = 0.;
        while fraction < 1. {
            let xx = (f64::from(x) + dx as f64 * fraction + 0.5) as i32;
            let yy = (f64::from(y) + dy as f64 * fraction + 0.5) as i32;
            for &id in people {
                let [max_x, max_y, min_x, min_y] = bounds(id)?;
                if min_x <= xx && xx < max_x && min_y <= yy && yy < max_y {
                    return Ok(Some(id));
                }
            }
            fraction += 1. / steps as f64;
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn native_shoo_query_order_and_sampled_fallback_match() {
        let report: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/shoo-selection.json")).unwrap();
        for row in report["cases"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            let people = serde_json::from_value(n["eligible"].clone()).unwrap();
            let mut s = Selection::new(people);
            let mut calls = vec![];
            let mut attempts = 0;
            while let Some(request) = s.next() {
                s = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
                match request {
                    Request::Path {
                        ignored,
                        ignore_people,
                    } => {
                        let id = ignored.unwrap_or(0);
                        calls.push(json!(["prepare", id, u8::from(ignore_people)]));
                        let points = &c["paths"][id.to_string()];
                        calls.push(json!(["path", id, !points.is_null()]));
                        s.path_result(serde_json::from_value(points.clone()).unwrap(), |id| {
                            let [x0, y0, x1, y1]: [i32; 4] =
                                serde_json::from_value(c["bounds"][id as usize - 1].clone())
                                    .unwrap();
                            Ok([x1, y1, x0, y0])
                        })
                        .unwrap();
                    }
                    Request::Attempt { blocker, points } => {
                        let outcomes = c["accept"].as_array().unwrap();
                        let accepted = outcomes[attempts.min(outcomes.len() - 1)]
                            .as_bool()
                            .unwrap();
                        attempts += 1;
                        calls.push(json!(["attempt", blocker, points, accepted]));
                        s.attempt_result(accepted).unwrap();
                    }
                }
            }
            let expected: Vec<_> = n["calls"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|c| c[0] != "context" && c[0] != "destroy")
                .cloned()
                .collect();
            assert_eq!(json!(calls), json!(expected), "{c}");
            assert_eq!(json!(s.outcome()), n["returned"], "{c}");
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Execution {
    pub thread: sims_vm::ThreadId,
    pub route: sims_vm::RouteRequest,
    pub selection: Selection,
    /// Captured once by 0x481693, independent of each ignored-person pointer.
    pub ignored_word: i16,
    pub blocked_object: i16,
    pub retained_result: u8,
    /// Constructor-owned context+98 survives individual and fallback queries.
    /// Historical absence is unknown, never a fresh constructor request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_limit: Option<usize>,
    pub query: Option<super::live::Execution>,
    pub attempt: Option<attempt::Execution>,
}
impl Execution {
    fn retained_candidate_limit(&self) -> Result<usize, String> {
        let limit = self
            .candidate_limit
            .ok_or("shoo retained candidate limit absent")?;
        if ![8, 48].contains(&limit) {
            return Err("invalid shoo retained candidate limit".into());
        }
        if self
            .query
            .as_ref()
            .is_some_and(|q| q.potential_candidate_limit() != limit)
        {
            return Err("shoo temporary candidate limit mismatch".into());
        }
        Ok(limit)
    }
    pub fn owns_callback(
        &self,
        vm: &sims_vm::Vm,
        p: &sims_vm::Program,
        thread: sims_vm::ThreadId,
        depth: usize,
    ) -> bool {
        self.query
            .as_ref()
            .is_some_and(|q| q.owns_callback(vm, p, thread, depth))
            || self
                .attempt
                .as_ref()
                .is_some_and(|a| a.owns_callback(vm, p, thread, depth))
    }
    pub fn validate(&self, s: &crate::Simulation, p: &sims_vm::Program) -> Result<(), String> {
        self.retained_candidate_limit()?;
        self.selection.validate()?;
        let t =
            s.vm.threads
                .get(&self.thread)
                .ok_or("shoo owner thread absent")?;
        let control = s
            .route_control
            .get(&self.route.id)
            .ok_or("shoo route control absent")?;
        if crate::contention::pending_route(t, self.route.id) != Some(&self.route)
            || s.visit.map(|v| v.0) != Some(self.thread)
            || control.thread != self.thread
            || control.caller != self.route.caller
            || !control.deciding
            || control.shoo_result.is_some()
            || control.link.is_some()
            || self.selection.cursor > self.selection.people.len()
            || self.selection.result.is_some()
            || self.selection.people.iter().any(|id| {
                s.vm.world
                    .objects
                    .get(id)
                    .is_none_or(|o| o.person.is_none())
            })
        {
            return Err("invalid native shoo continuation".into());
        }
        if let Some(q) = &self.query {
            let Some(Request::Path {
                ignored,
                ignore_people,
            }) = self.selection.pending
            else {
                return Err("shoo query without path request".into());
            };
            if q.potential.as_ref()
                != Some(&super::live::Potential {
                    ignored_person: ignored,
                    ignore_people,
                    ignored_word: self.ignored_word,
                    blocked_object: self.blocked_object,
                    retained_result: self.retained_result,
                })
            {
                return Err("shoo temporary path context mismatch".into());
            }
            q.validate(s, p)?;
        }
        if let Some(a) = &self.attempt {
            if self.query.is_some()
                || !matches!(self.selection.pending, Some(Request::Attempt { blocker, .. }) if blocker == a.blocker)
            {
                return Err("unbound native shoo attempt".into());
            }
            a.validate(s, p, &self.route, self.thread)?;
        }
        Ok(())
    }
    pub(crate) fn owns_deletion(&self, id: ObjectId) -> bool {
        self.attempt.as_ref().is_some_and(|a| a.owns_deletion(id))
    }
}
impl crate::Simulation {
    pub(crate) fn drive_native_shoo(
        &mut self,
        p: &sims_vm::Program,
        remaining: &mut usize,
        step: &mut crate::Step,
    ) -> Result<bool, String> {
        let Some(mut e) = self.native_lot.as_mut().and_then(|n| n.shoo.take()) else {
            return Ok(true);
        };
        let result = self.drive_native_shoo_inner(p, &mut e, remaining, step);
        if result.as_ref().is_ok_and(|done| *done) {
            self.route_control.get_mut(&e.route.id).unwrap().shoo_result = e.selection.outcome();
        } else {
            self.native_lot.as_mut().unwrap().shoo = Some(e);
        }
        result
    }
    fn drive_native_shoo_inner(
        &mut self,
        p: &sims_vm::Program,
        e: &mut Execution,
        remaining: &mut usize,
        step: &mut crate::Step,
    ) -> Result<bool, String> {
        //Reject historical unknown ownership before mutating selection or World.
        e.retained_candidate_limit()?;
        loop {
            let Some(request) = e.selection.next() else {
                return Ok(true);
            };
            match request {
                Request::Path {
                    ignored,
                    ignore_people,
                } => {
                    if e.query.is_none() {
                        let retained_limit = e.retained_candidate_limit()?;
                        let q = super::live::Execution::begin_retained_potential(
                            self,
                            e.thread,
                            &e.route,
                            super::live::Potential {
                                ignored_person: ignored,
                                ignore_people,
                                ignored_word: e.ignored_word,
                                blocked_object: e.blocked_object,
                                retained_result: e.retained_result,
                            },
                            retained_limit,
                        )?;
                        e.candidate_limit = Some(q.potential_candidate_limit());
                        e.query = Some(q);
                    }
                    let q = e.query.as_mut().unwrap();
                    if !self.drive_point_clearance(p, q, remaining, step)? {
                        return Ok(false);
                    }
                    q.mark_potential_prepared();
                    let actor = &self.vm.world.objects[&q.caller];
                    let person = actor.person.as_ref().unwrap();
                    let same_room = actor.data[&29] == self.vm.world.objects[&q.target].data[&29]
                        || [68, 84].into_iter().any(|i| person[&i] & 1 != 0);
                    let empty = q.candidates.as_ref().unwrap().candidates.is_empty();
                    if !empty && same_room {
                        if !self.drive_native_path_prefix(p, q, remaining, step)? {
                            return Ok(false);
                        }
                        if q.policy == super::path::Policy::Strict {
                            return Err("original shoo potential path crosses UNVERIFIED full routing wrapper boundary".into());
                        }
                    }
                    (e.blocked_object, e.retained_result) = q.potential_result();
                    if empty && e.retained_result == 1 {
                        e.retained_result = 7;
                    }
                    if q.failed {
                        e.retained_result = 3;
                    }
                    let points = q
                        .path
                        .as_ref()
                        .and_then(|i| i.points.as_ref())
                        .map(|v| v.iter().map(|p| [p.y, p.x]).collect::<Vec<_>>());
                    self.event(&mut step.events,e.thread,format!("original shoo potential query: ignored #{} all-people {} path {} points",ignored.unwrap_or(0),ignore_people,points.as_ref().map_or(0,Vec::len)));
                    e.selection.path_result(points, |id| {
                        self.native_lot
                            .as_ref()
                            .unwrap()
                            .placement
                            .bounds
                            .get(&id)
                            .copied()
                            .ok_or("shoo person bounds absent".into())
                    })?;
                    e.query = None;
                }
                Request::Attempt { blocker, points } => {
                    if let Some(a) = e.attempt.as_mut() {
                        if let Some(accepted) = self
                            .drive_native_shoo_attempt(p, a, e.thread, &e.route, remaining, step)?
                        {
                            e.selection.attempt_result(accepted)?;
                            e.attempt = None;
                            continue;
                        }
                        return Ok(false);
                    }
                    if points.len() < 2 || self.has_move_action(blocker) {
                        e.selection.attempt_result(false)?;
                        continue;
                    }
                    let [max_x, max_y, min_x, min_y] =
                        self.native_lot.as_ref().unwrap().placement.bounds[&blocker];
                    let position = &self.vm.world.positions[&blocker];
                    let path: Vec<_> = points
                        .iter()
                        .map(|p| crate::Point::new(p[1], p[0]))
                        .collect();
                    let direction = crate::contention::shoo_direction(
                        &path,
                        crate::Point::new(position.y, position.x),
                        crate::Rect {
                            min: crate::Point::new(min_y, min_x),
                            max: crate::Point::new(max_y, max_x),
                        },
                    );
                    e.attempt = attempt::Execution::begin(self, p, blocker, direction)?;
                    if e.attempt.is_none() {
                        e.selection.attempt_result(false)?;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "shoo/retained_cap.rs"]
mod retained_cap;
