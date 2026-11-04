//! Recovered native routing decisions. World queries are explicit boundaries.
use crate::{Body, Destination, Point};
use serde::{Deserialize, Serialize};

mod posture;
mod standing;
pub use posture::PostureScan;
pub use standing::{StandingCheck, StandingScan};

/// Native horizontal coordinates; the laboratory room swaps native X and Y.
/// Direction 0 moves toward negative native X, direction 2 toward positive Y.
pub const DIRECTION_OFFSETS: [(i32, i32); 8] = [
    (-16, 0),
    (-16, 16),
    (0, 16),
    (16, 16),
    (16, 0),
    (16, -16),
    (0, -16),
    (-16, -16),
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Departure {
    pub position: Point,
    /// Relative to the seat's facing. Zero means no departure was selected.
    pub flags: u16,
}

/// Sims 1.91 0x004832d4: preselect a seated departure toward the next target.
/// The caller supplies containment/function lookup and placement classification.
/// Classification 5 (person obstruction) and 6 (clear) are eligible here; later
/// routing/Stand services still have to resolve any obstruction.
pub fn select_departure(
    position: Point,
    target: Point,
    facing: i16,
    allowed: u16,
    mut classify: impl FnMut(Point, u8) -> i32,
) -> Departure {
    let mut selected = Departure { position, flags: 0 };
    let mut best = None;
    for relative in 0..8 {
        let flag = 1 << relative;
        if allowed & flag == 0 {
            continue;
        }
        let direction = (i32::from(facing) + relative).rem_euclid(8) as u8;
        let (dx, dy) = DIRECTION_OFFSETS[usize::from(direction)];
        let point = Point::new(position.x.wrapping_add(dx), position.y.wrapping_add(dy));
        if !matches!(classify(point, direction), 5 | 6) {
            continue;
        }
        let dx = target.x.wrapping_sub(point.x);
        let dy = target.y.wrapping_sub(point.y);
        let distance = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
        // Signed native comparison and strict inequality preserve scan-order ties.
        if best.is_none_or(|previous| distance < previous) {
            best = Some(distance);
            selected = Departure {
                position: point,
                flags: flag,
            };
        }
    }
    selected
}

/// Native routing context after SLOT version/unit conversion. Positions use
/// native horizontal coordinates; offset is the original SLOT X/Y pair.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandingRequest {
    pub target: [i32; 2],
    pub preferred: [i32; 2],
    pub rotation: u8,
    pub actor_facing: u8,
    pub offset: [f32; 2],
    pub flags: u32,
    pub standing: i32,
    pub ground: i32,
    pub proximity: [i32; 3],
    pub gradient: f32,
    pub facing: i32,
    pub resolution: i32,
    pub walls: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Candidate {
    pub position: [i32; 2],
    pub score: i32,
    pub seat: i16,
    pub flags: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateSet {
    pub candidates: Vec<Candidate>,
    pub maximum: i32,
    pub result: u8,
    pub actor_facing: u8,
}

/// The exact-point branch of native 0x483414 (SLOT flags low byte zero).
/// Clearance is a separate world query. The caller must publish actor_facing
/// before that query, even when clearance subsequently rejects the destination.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PointCandidate {
    pub position: [i32; 2],
    /// None is the native -1 sentinel, which suppresses the approach wall query.
    pub direction: Option<u8>,
    pub actor_facing: u8,
}

impl PointCandidate {
    pub fn prepare(
        target: [i32; 2],
        offset: [f32; 2],
        rotation: u8,
        flags: u32,
    ) -> Result<Self, String> {
        if rotation > 7
            || flags & !0xfe00 != 0
            || target.iter().any(|v| v.abs_diff(0) > 32768)
            || offset.iter().any(|v| !v.is_finite() || v.abs() > 512.0)
        {
            return Err("unsupported native exact-point candidate context".into());
        }
        let rotation = if flags & 0x200 != 0 { 0 } else { rotation };
        let position = transform(target, offset[0], offset[1], rotation);
        let direction = (target[0] >> 4 != position[0] >> 4 || target[1] >> 4 != position[1] >> 4)
            .then(|| sims_vm::routing::direction(target, position) as u8);
        Ok(Self {
            position,
            direction,
            // 0x483e63 ignores the requested SLOT facing in this branch.
            actor_facing: (rotation + 4) & 7,
        })
    }

    /// A successful candidate retains the context's previous result word;
    /// only the empty/rejected branch writes a new rejection code.
    pub fn finish(&self, classification: u8, retained_result: u8) -> Result<CandidateSet, String> {
        if classification > 6 {
            return Err("invalid placement classification".into());
        }
        Ok(CandidateSet {
            candidates: if classification == 6 {
                vec![Candidate {
                    position: self.position,
                    score: 100,
                    seat: 0,
                    flags: 0,
                }]
            } else {
                Vec::new()
            },
            maximum: if classification == 6 { 100 } else { -1 },
            result: if classification == 6 {
                retained_result
            } else {
                [0, 8, 11, 11, 12, 13][usize::from(classification)]
            },
            actor_facing: self.actor_facing,
        })
    }
}

/// Native 0x483414 retains seats first, then makes one forward scan from a
/// random index per remaining slot. A scan reaching the end is not retried.
/// The pre-subset maximum score is deliberately retained.
pub fn limit_candidates(set: &mut CandidateSet, limit: usize, rng: &mut u32) {
    if set.candidates.len() <= limit {
        return;
    }
    let mut source = std::mem::take(&mut set.candidates);
    for candidate in &mut source {
        if candidate.seat != 0 {
            set.candidates.push(candidate.clone());
            candidate.score = 0;
        }
    }
    for _ in set.candidates.len()..limit {
        let start = sims_vm::rng::next(rng) as usize % source.len();
        if let Some(candidate) = source[start..].iter_mut().find(|c| c.score != 0) {
            set.candidates.push(candidate.clone());
            candidate.score = 0;
        }
    }
}

/// Native 0x48235a restores the actor's pre-scan direction after the original
/// candidate subset step. Clearance-facing changes are temporary preparation.
pub fn finish_candidate_scan(
    mut set: CandidateSet,
    limit: usize,
    rng: &mut u32,
    saved_facing: u8,
    retained_result: u8,
) -> CandidateSet {
    limit_candidates(&mut set, limit, rng);
    // 0x483414 writes context +0x88 only when the resulting vector is empty.
    if !set.candidates.is_empty() {
        set.result = retained_result;
    }
    set.actor_facing = saved_facing;
    set
}

/// Original 0x45ad63 state 1 enters room/portal routing before preparation.
/// Person field 84 affects later obstacle construction, not this branch.
pub fn needs_portal(actor_room: i16, target_room: i16, person68: i16) -> bool {
    person68 & 1 == 0 && actor_room != target_room
}

/// Original non-seat tail of 0x45c38e: point submitted to the animator after
/// successful path selection. Unlike candidate-clearance facing, mode -3 follows
/// the final path segment and flag 0x100 can aim at the target's exact position.
pub fn endpoint_look(
    path: &[[i32; 2]],
    target: [i32; 2],
    rotation: u8,
    flags: u32,
    facing: i32,
    actor_facing: u8,
) -> Result<Option<[i32; 2]>, String> {
    if rotation > 7 || actor_facing > 7 || !(-3..=7).contains(&facing) {
        return Err("invalid native route endpoint context".into());
    }
    if path.len() < 2 {
        return Ok(None);
    }
    let goal = path[path.len() - 1];
    let previous = path[path.len() - 2];
    let direction = match facing {
        -3 => {
            let delta = [
                goal[0].wrapping_sub(previous[0]),
                goal[1].wrapping_sub(previous[1]),
            ];
            if delta != [0, 0] {
                return Ok(Some([
                    goal[0].wrapping_add(delta[0]),
                    goal[1].wrapping_add(delta[1]),
                ]));
            }
            actor_facing
        }
        -2 | -1 => {
            if flags & 255 != 0 && flags & 0x100 != 0 {
                return Ok((goal != target).then_some(target));
            }
            let direction = if flags & 255 == 0 && goal == target {
                rotation
            } else {
                sims_vm::routing::direction(goal, target) as u8
            };
            (direction + if facing == -1 { 4 } else { 0 }) & 7
        }
        n => (rotation + n as u8) & 7,
    };
    let (x, y) = DIRECTION_OFFSETS[usize::from(direction)];
    Ok(Some([goal[0].wrapping_add(x), goal[1].wrapping_add(y)]))
}

fn transform(center: [i32; 2], x: f32, y: f32, rotation: u8) -> [i32; 2] {
    // Cardinal branches in 0x00483d4b intentionally truncate X before a quarter
    // turn. Native Y addition is stored as f32; native X stays in x87 until ftol.
    let (x, y) = match rotation {
        0 => (x, f64::from(y)),
        2 => (-y, f64::from(x as i32)),
        4 => (-x, -f64::from(y)),
        6 => (y, -f64::from(x as i32)),
        n => {
            // The diagonal branch passes through the 3/16 Vitaboy coordinate
            // conversion, a single-precision rotation matrix, then rounds
            // back to integral subtiles (ties away from zero).
            let angle = (f64::from(n) * f64::from(std::f32::consts::PI) * 0.25) as f32;
            let (s, c) = (f64::from(angle).sin() as f32, f64::from(angle).cos() as f32);
            let (x, y) = (x * 0.1875, y * 0.1875);
            let ox = (f64::from(x) * f64::from(c) - f64::from(y) * f64::from(s)) as f32;
            let oy = (f64::from(x) * f64::from(s) + f64::from(y) * f64::from(c)) as f32;
            let scale = 16.0f32 / 3.0;
            (
                (f64::from(ox) * f64::from(scale)).round() as f32,
                f64::from((f64::from(oy) * f64::from(scale)).round() as f32),
            )
        }
    };
    [
        (f64::from(center[0]) + y) as i32,
        (center[1] as f32 + x) as i32,
    ]
}

/// Original 0x00483414 standing/ground candidate enumeration. Seating uses
/// `posture_candidates`; the wrapper applies `limit_candidates` afterward.
/// Wall adjacency and position classification are provided by the world service.
pub fn standing_candidates(
    request: &StandingRequest,
    rng: &mut u32,
    mut classify: impl FnMut(Point, u8) -> i32,
) -> Result<CandidateSet, String> {
    let mut scan = StandingScan::new(request.clone())?;
    while let Some(check) = scan.next(rng)? {
        let classification = classify(
            Point::new(check.position[0], check.position[1]),
            check.direction,
        );
        scan.resume(
            check,
            u8::try_from(classification).map_err(|_| "invalid placement classification")?,
        )?;
    }
    Ok(scan.outcome().unwrap().clone())
}

/// Seat metadata supplied by the native tile lookup and slot-zero lookup.
#[derive(Debug, Clone, Copy)]
pub struct Seat {
    pub id: i16,
    pub facing: i16,
    pub allowed: u16,
    pub sit: bool,
    /// An occupant other than the routing actor; zero also covers the actor itself.
    pub other_occupant: i16,
}

/// Original 0x483414 combined standing/seated scan. Each lattice point tries the
/// higher-weight posture first; a successful candidate ends that point's scan.
/// A seat's position is used for scoring, but its approach point is the route goal.
pub fn posture_candidates(
    r: &StandingRequest,
    sitting: i32,
    rng: &mut u32,
    mut lookup: impl FnMut(Point) -> Option<Seat>,
    mut classify: impl FnMut(Point, u8) -> i32,
) -> Result<(CandidateSet, i16), String> {
    if sitting == 0 {
        return Ok((standing_candidates(r, rng, classify)?, 0));
    }
    let mut scan = PostureScan::new(r.clone(), sitting, 0)?;
    while let Some(check) = scan.next(rng, |point, _| Ok(lookup(point)))? {
        let result = classify(
            Point::new(check.position[0], check.position[1]),
            check.direction,
        );
        let result = u8::try_from(result).map_err(|_| "invalid placement classification")?;
        scan.resume(check, result)?;
    }
    Ok((scan.outcome().unwrap().clone(), scan.blocked_object()))
}

/// Geometric goals for the shared host. Relative routes use original standing
/// candidate enumeration; collision classification and path search remain the
/// authored room's services. No random-score flag is produced by this wrapper.
pub fn destinations(
    request: &sims_vm::routing::Destination,
    target: &Body,
    caller: &Body,
) -> Result<Vec<Destination>, String> {
    use sims_vm::routing::Destination as Request;
    match request {
        Request::FirstSlot | Request::Front => {
            Err("direct Snap destination cannot be routed".into())
        }
        Request::Slot { slot, version, .. } => {
            if slot.snap_target >= 0 {
                return Err("route-to-contained/sitting goals are unsupported; use Snap".into());
            }
            if let Ok(goals) = crate::geometry::destinations(slot, *version, target, caller) {
                return Ok(goals);
            }
            let request = standing_slot(slot, *version, target, caller)?;
            let set = standing_candidates(&request, &mut 0, |_, _| 6)?;
            Ok(set
                .candidates
                .iter()
                .map(|c| candidate_destination(&request, c))
                .collect())
        }
        Request::Relative(r) => {
            if r.flags & 0x8000 != 0 {
                return Err("relative route flag 0x8000 requires native driver support".into());
            }
            if r.flags & 255 == 0 {
                let rotation = target.facing.trailing_zeros() as u8;
                let facing = match r.facing {
                    -3 => caller.facing,
                    -2 => caller.facing,
                    n => 1 << ((rotation + n as u8) & 7),
                };
                return Ok(vec![Destination {
                    position: target.position,
                    facing,
                    entry_flags: 0,
                }]);
            }
            let rotation = target.facing.trailing_zeros() as u8;
            let candidates = standing_candidates(
                &StandingRequest {
                    target: [target.position.y, target.position.x],
                    preferred: [caller.position.y, caller.position.x],
                    rotation,
                    actor_facing: caller.facing.trailing_zeros() as u8,
                    offset: [0.0; 2],
                    flags: r.flags,
                    standing: 1,
                    ground: 0,
                    proximity: r.proximity,
                    gradient: 0.1875,
                    facing: r.facing,
                    resolution: 16,
                    walls: 0,
                },
                &mut 0,
                |_, _| 6,
            )?;
            Ok(candidates
                .candidates
                .into_iter()
                .map(|candidate| {
                    let direction = (rotation + candidate.flags.trailing_zeros() as u8) & 7;
                    let facing = match r.facing {
                        -3 => caller.facing.trailing_zeros() as u8,
                        -2 => (direction + 4) & 7,
                        n => (rotation + n as u8) & 7,
                    };
                    Destination {
                        position: Point::new(candidate.position[1], candidate.position[0]),
                        facing: 1 << facing,
                        entry_flags: candidate.flags as i16,
                    }
                })
                .collect())
        }
    }
}

/// Convert the supported standing SLOT context to native candidate units.
/// Room/wall maps and sitting/ground-position searches remain separate services.
pub fn standing_slot(
    slot: &sims_formats::execution::Slot,
    version: u32,
    target: &Body,
    caller: &Body,
) -> Result<StandingRequest, String> {
    if !(4..=10).contains(&version)
        || slot.kind != 3
        || slot.snap_target >= 0
        || slot.height != 0
        || slot.unknown != 0
        || !matches!(slot.max_size, 0 | 100)
        || slot.sitting < 0
        || slot.ground != 0
        || slot.standing <= 0 && slot.sitting == 0
        || slot.offset[2] != 0.
    {
        return Err("unsupported native standing SLOT context".into());
    }
    let scale = if version <= 9 { 16 } else { 1 };
    let mut proximity = [0; 3];
    for (i, n) in slot.proximity.iter().enumerate() {
        proximity[i] = n.checked_mul(scale).ok_or("SLOT proximity overflow")?;
    }
    Ok(StandingRequest {
        target: [target.position.y, target.position.x],
        preferred: [caller.position.y, caller.position.x],
        rotation: target.facing.trailing_zeros() as u8,
        actor_facing: caller.facing.trailing_zeros() as u8,
        offset: [slot.offset[0], slot.offset[1]],
        flags: slot.flags,
        standing: slot.standing,
        ground: slot.ground,
        proximity,
        gradient: slot.gradient,
        facing: slot.facing,
        resolution: slot.resolution,
        walls: 0,
    })
}
pub fn candidate_destination(r: &StandingRequest, candidate: &Candidate) -> Destination {
    let rotation = if r.flags & 512 != 0 { 0 } else { r.rotation };
    let direction = (rotation + candidate.flags.trailing_zeros() as u8) & 7;
    let facing = match r.facing {
        -3 => r.actor_facing,
        -2 => (direction + 4) & 7,
        -1 => direction,
        n => (rotation + n as u8) & 7,
    };
    Destination {
        position: Point::new(candidate.position[1], candidate.position[0]),
        facing: 1 << facing,
        entry_flags: candidate.flags as i16,
    }
}

/// Native Snap modes 1/2 preserve the actor's facing. The first slot's actual
/// containment offset/height is applied by the placement service, not this call.
pub fn direct_snap(
    target: [i32; 2],
    rotation: u8,
    facing: u8,
    front: bool,
) -> Result<([i32; 2], u8), String> {
    if rotation > 7 || facing > 7 {
        return Err("invalid direct Snap direction".into());
    }
    let (dx, dy) = if front {
        DIRECTION_OFFSETS[usize::from(rotation)]
    } else {
        (0, 0)
    };
    Ok((
        [target[0].wrapping_add(dx), target[1].wrapping_add(dy)],
        facing,
    ))
}

/// Native 0x48177b reuses a current container only when a prepared candidate
/// names that same existing object and its Sit function is present.
pub fn keeps_current_seat(
    seats: impl IntoIterator<Item = i16>,
    current: i16,
    mut has_sit: impl FnMut(i16) -> bool,
) -> bool {
    seats
        .into_iter()
        .any(|id| id != 0 && has_sit(id) && id == current)
}
impl crate::Simulation {
    /// Native 0x43244d skips non-colliding objects in tile-chain order before
    /// checking Sit presence. Returning the first object matters on shared tiles.
    pub(crate) fn seat_at(&self, p: &sims_vm::Program, actor: i16, point: Point) -> Option<Seat> {
        if point.x >> 4 < 1
            || point.y >> 4 < 1
            || point.x >> 4 >= (self.room.height / 16) - 1
            || point.y >> 4 >= (self.room.width / 16) - 1
        {
            return None;
        }
        let world = &self.vm.world;
        let order = &world.person.as_ref()?.object_order;
        let id = *order.iter().find(|&&id| {
            let Some(body) = self.bodies.get(&id) else {
                return false;
            };
            let object = &world.objects[&id];
            let flags = object.data.get(&8).copied().unwrap_or(0) as u16;
            !self.unplaced.contains(&id)
                && object.container.is_none()
                && body.level == self.bodies[&actor].level
                && body.position.y >> 4 == point.x >> 4
                && body.position.x >> 4 == point.y >> 4
                && !(if flags & 4 == 0 {
                    flags & 16 != 0
                } else {
                    flags & 2 == 0
                })
        })?;
        let object = &world.objects[&id];
        let def = object.definition.as_ref()?;
        let m = &p.modules[&def.source];
        let occupant = object.slots.first().copied().flatten().unwrap_or(0);
        Some(Seat {
            id,
            facing: object.data[&1],
            allowed: m.object_definition_words[&def.object][0x84 / 2] as u16,
            sit: m.object_functions.get(&def.object)?.get(26)?.action != 0,
            other_occupant: if occupant == actor { 0 } else { occupant },
        })
    }
}

/// 0x433612 scans cardinal neighbors for a Sit function facing this tile.
/// The tile query supplies the first non-pass-through object in native order.
pub fn adjacent_seat(mut tile: impl FnMut(u8) -> Option<(bool, u8)>) -> bool {
    (0..8u8)
        .step_by(2)
        .any(|d| tile(d).is_some_and(|(sit, facing)| sit && facing == (d + 4) & 7))
}
pub fn adjacency_flags(old: u16, seat: bool, surface: bool) -> (u16, bool) {
    (
        if seat { old | 256 } else { old & !256 },
        surface && (old & 256 != 0) != seat,
    )
}
impl crate::Simulation {
    /// Authored lot construction refreshes native seat adjacency and the original
    /// surface placement callback after all furniture is present. Runtime build
    /// mode will need this same notification on every placement/removal.
    pub fn initialize_seat_adjacency(&mut self, p: &sims_vm::Program) -> Result<(), String> {
        let ids = self
            .vm
            .world
            .person
            .as_ref()
            .ok_or("object registry missing")?
            .object_order
            .clone();
        for id in ids.iter().copied() {
            let Some(body) = self.bodies.get(&id) else {
                continue;
            };
            let x = body.position.y >> 4;
            let y = body.position.x >> 4;
            let seat = adjacent_seat(|d| {
                let (dx, dy) = DIRECTION_OFFSETS[d as usize];
                let other = ids.iter().find(|&&other| {
                    self.bodies.get(&other).is_some_and(|b| {
                        b.level == body.level
                            && b.position.y >> 4 == x + dx / 16
                            && b.position.x >> 4 == y + dy / 16
                    }) && self.vm.world.objects[&other].container.is_none()
                        && self.vm.world.objects[&other].data[&8] & 4 == 0
                })?;
                let object = &self.vm.world.objects[other];
                let def = object.definition.as_ref()?;
                let sit = p.modules[&def.source]
                    .object_functions
                    .get(&def.object)?
                    .get(26)?
                    .action
                    != 0;
                Some((sit, object.data[&1] as u8))
            });
            let o = &self.vm.world.objects[&id];
            let def = o
                .definition
                .as_ref()
                .ok_or("adjacency definition missing")?;
            let functions = &p.modules[&def.source].object_functions[&def.object];
            let (flags, notify) = adjacency_flags(
                o.data[&8] as u16,
                seat,
                functions.get(25).is_some_and(|f| f.action != 0),
            );
            let callback = functions.get(9).map_or(0, |f| f.action);
            let owner = def.source.clone();
            let args = o.temps.to_vec();
            self.vm
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .data
                .insert(8, flags as i16);
            if notify && callback != 0 {
                let thread = self.vm.object_threads[&id];
                self.vm.begin_immediate_callback(
                    p,
                    thread,
                    sims_vm::Start {
                        owner: &owner,
                        bhav: callback,
                        caller: id,
                        callee: id,
                        stack_object: id,
                        args,
                        check: true,
                    },
                )?;
                let run = self.vm.run_slice(p, thread, 10000)?;
                if self.vm.threads[&thread].callback.is_some()
                    || matches!(run.stop, sims_vm::Stop::Faulted)
                {
                    return Err(format!(
                        "authored adjacency callback did not complete: {run:?}"
                    ));
                }
                self.vm
                    .threads
                    .get_mut(&thread)
                    .unwrap()
                    .callback_result
                    .take()
                    .ok_or("adjacency callback result missing")?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FailureFeedback {
    pub thread: sims_vm::ThreadId,
    pub code: i16,
    pub blocker: sims_vm::ObjectId,
}

/// Native route state 6 permits another plan after more than eight subtiles
/// of progress. At or below that threshold it advances to ask/failure handling.
pub fn made_route_progress(previous: Point, current: Point) -> bool {
    let dx = current.x.wrapping_sub(previous.x);
    let dy = current.y.wrapping_sub(previous.y);
    dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) >= 65
}
