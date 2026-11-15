//! A person in the way of a route, and asking them to move (0x45ba28,
//! 0x45bf2e), which step 6 of a route ([`super::go_to_slot`]) does when
//! the planning found no way twice.
//!
//! **Who is in the way** (0x45ba28, for the routing person R and R's
//! last context):
//!
//! 1. The candidates are the lot's people but R, one the planner leaves
//!    out of its obstacles for R's context ([`crate::Interpreter::ignored`]:
//!    one farther than 3 tiles, among others), every one when R's
//!    posture (person word 0) is not 0, and one some person's route
//!    context already waits on (+0x80). None: nobody is in the way.
//! 2. A new context for R and the same slot (0x48163e) is planned once
//!    for each candidate, with the candidate as the person it leaves out
//!    (+0x7c). The first candidate with a way that agrees to move
//!    (step 4) is in the way.
//! 3. Otherwise the context is planned leaving nobody out, with byte
//!    +0x8e set (which nothing reads). With no way, or a path of fewer
//!    than two points, nobody is in the way. Else the path is walked in
//!    [`samples`]; the first candidate whose footprint holds a sample is
//!    asked to move.
//!
//! **Asking P to move** (0x45bf2e):
//!
//! 1. P is not asked when P's running or queued actions already include
//!    the marker's move interaction (0x45bec1), when the path has fewer
//!    than two points, or when P's last route context itself waits on
//!    someone.
//! 2. Where the path enters P's footprint and where it leaves it
//!    ([`samples`]): with no sample inside, from the path's start to P;
//!    entered and not left, to the path's end.
//! 3. A marker (type 0x7c4) is made (0x440b2d) and put at P's position
//!    and level (virtual function 0x58). With no movement between the two
//!    points the original stops there, leaving the marker on the lot.
//! 4. The marker's first slot is written over (0x45c2ab): standing,
//!    within 16 to 80 of the marker at a resolution of 8, facing anywhere
//!    (-3), flags 0x100 and 0x2000, and the two directions square to the
//!    way R goes ([`direction`] plus and minus 2). If P has no way to that
//!    slot, the way R goes is allowed too and the best distance becomes
//!    80. The record is kept in [`sims_sim::route::Routing::slots`].
//! 5. P gets an action on the marker (0x41f00e): its interaction with
//!    menu number 3, at the priority of R's running action (person word
//!    33), and it is queued (0x45d20b, [`Machine::queue_action`]). If it
//!    cannot be, the marker is removed at once and P is not asked.
//! 6. R's context then waits on P (+0x80) for that action (+0x90), and
//!    its byte +0x84 is cleared.
//!
//! The lot's list of people is not modeled; more than one candidate,
//! whose order would matter, fails loudly. So do an action that runs at
//! once (interaction flag 4) and one that pushes queued actions out.

use crate::autonomy::{attenuation, find, person};
use crate::fault::{Fault, Trap};
use crate::interpreter::Machine;
use crate::placement::{Place, on_lot};
use crate::routing::slot::{direction, new_context};
use sims_core::{Ext, F32};
use sims_formats::saved_objects::Interaction;
use sims_sim::route::{Context, RouteSlot};

/// The type of the marker a person asked to move routes to (0x45c0e0).
const MARKER: u32 = 0x7c4;
/// The marker's interaction that moves the person (0x45c2d6).
const MOVE: i32 = 3;
/// The person data word that holds the running action's priority.
const PRIORITY: usize = 33;
/// Person data word 0.
const POSTURE: usize = 0;
/// The interaction table entry flag that runs the action at once
/// (0x45d235).
const AT_ONCE: u32 = 4;
/// The interaction table entry flag that sets the action's flag bit 0
/// (0x41f09e).
const ENTRY_FLAG: u32 = 0x100;
/// The action flag that puts it at the queue's front (0x45d317).
const TO_FRONT: u32 = 2;
/// How many actions a person's queue holds (+0x820, 0x45f744).
const QUEUE: usize = 8;
/// The function an object runs when an action on it is cancelled.
const CANCELLED: usize = 4;

fn unsupported(at: &str, what: &str) -> Trap {
    Trap::Unsupported(format!("a route {what} ({at}), which is not ported"))
}

/// The points 0x45ba28 and 0x45bf2e test along one segment of a path,
/// in order. Points are in the path's order of coordinates. With `n` one
/// more than the larger of the two differences, `t` goes from 0 by
/// `1 / n` while it is below 1, and each point is `from + d * t + 0.5`,
/// truncated. The original computes this in the x87 unit at single
/// precision (each operation rounded to 24 bits), constants 1.0, 0.5
/// (0x666748, 0x666578) and the compare with 1.0 as a double (0x667280).
pub(crate) fn samples(from: [i32; 2], to: [i32; 2]) -> Vec<[i32; 2]> {
    let d = [to[0].wrapping_sub(from[0]), to[1].wrapping_sub(from[1])];
    let n = d[0].wrapping_abs().max(d[1].wrapping_abs()).wrapping_add(1);
    let one = Ext::from_i32(1);
    let half = Ext::from_f32(F32::from_bits(0x3f00_0000));
    let step = (one / Ext::from_i32(n)).single();
    let at = |t: Ext, i: usize| {
        let scaled = (Ext::from_i32(d[i]) * t).single();
        let moved = (scaled + Ext::from_i32(from[i])).single();
        (moved + half).single().to_i32_truncate()
    };
    let mut points = Vec::new();
    let mut t = Ext::from_i32(0);
    loop {
        points.push([at(t, 0), at(t, 1)]);
        t = (t + step).single();
        if !matches!(t.partial_cmp(&one), Some(std::cmp::Ordering::Less)) {
            break;
        }
    }
    points
}

/// Whether a footprint ([`sims_sim::Object::footprint`]: least first and
/// second coordinates, then greatest) holds a point in the path's order
/// (0x45bcbd): the point's first coordinate is the position's second.
pub(crate) fn holds(footprint: [i32; 4], point: [i32; 2]) -> bool {
    (footprint[1]..footprint[3]).contains(&point[0])
        && (footprint[0]..footprint[2]).contains(&point[1])
}

/// The slot a person asked to move routes to (0x45c14d to 0x45c27c),
/// for R going in direction `dir`. `wide` is the slot once the person
/// found no way to the first.
pub(crate) fn move_slot(dir: i32, wide: bool) -> RouteSlot {
    let bit = |d: i32| 1 << d.rem_euclid(8);
    let mut slot = RouteSlot {
        offset: [0; 3],
        number: -1,
        standing: 1,
        sitting: 0,
        ground: 0,
        flags: bit(dir + 2) | bit(dir + 6) | 0x100 | 0x2000,
        snap_target: -1,
        proximity: [16, 80, 16],
        gradient: 0x3e40_0000,
        facing: -3,
        resolution: 8,
    };
    if wide {
        slot.flags |= bit(dir);
        slot.proximity = [16, 80, 80];
    }
    slot
}

impl Machine<'_, '_> {
    /// Finds who is in the way of the running person's route and asks
    /// them to move (0x45ba28). See the module's documentation.
    pub(super) fn find_in_way(&mut self, ctx: &mut Context) -> Result<bool, Trap> {
        let me = self.me;
        let interpreter = self.interpreter();
        let wrap = |fault: Fault| {
            Trap::Unsupported(format!("finding a person in the way (0x45ba28): {fault}"))
        };
        let people: Vec<u16> = self
            .world
            .objects
            .iter()
            .filter(|(_, o)| o.person.is_some())
            .map(|(&id, _)| id)
            .collect();
        let posture = self.person_word(POSTURE)?;
        let mut candidates = Vec::new();
        for id in people {
            if id == me || interpreter.ignored(self.world, ctx, id).map_err(wrap)? {
                continue;
            }
            if posture != 0 {
                continue;
            }
            let waited_on = self
                .world
                .routing
                .routes
                .values()
                .any(|r| r.contexts.iter().any(|c| c.objects[1] == id));
            if !waited_on {
                candidates.push(id);
            }
        }
        match candidates.len() {
            0 => return Ok(false),
            1 => {}
            n => {
                return Err(unsupported(
                    "0x45bb04",
                    &format!(
                        "with {n} people in the way, whose order on the lot's list of people \
                         is not modeled"
                    ),
                ));
            }
        }
        let mut copy = new_context(self.world, me, ctx.target, ctx.slot);
        let mut path = Vec::new();
        for &id in &candidates {
            path.clear();
            copy.objects[0] = id;
            interpreter
                .find_candidates(self.world, &mut copy)
                .map_err(wrap)?;
            if interpreter
                .plan(self.world, &mut copy, &mut path)
                .map_err(wrap)?
                && self.ask_to_move(id, ctx, &path)?
            {
                return Ok(true);
            }
        }
        path.clear();
        copy.objects[0] = 0;
        copy.byte_8e = true;
        interpreter
            .find_candidates(self.world, &mut copy)
            .map_err(wrap)?;
        if !interpreter
            .plan(self.world, &mut copy, &mut path)
            .map_err(wrap)?
            || path.len() < 2
        {
            return Ok(false);
        }
        let mut found = None;
        'segments: for pair in path.windows(2) {
            for point in samples(pair[0], pair[1]) {
                found = candidates.iter().copied().find(|id| {
                    self.world
                        .objects
                        .get(id)
                        .is_some_and(|o| holds(o.footprint, point))
                });
                if found.is_some() {
                    break 'segments;
                }
            }
        }
        match found {
            Some(id) => self.ask_to_move(id, ctx, &path),
            None => Ok(false),
        }
    }

    /// Asks the person `asked` to move out of the way of the running
    /// person's `path` (0x45bf2e). See the module's documentation.
    fn ask_to_move(
        &mut self,
        asked: u16,
        ctx: &mut Context,
        path: &[[i32; 2]],
    ) -> Result<bool, Trap> {
        if self.asked_to_move(asked)? || path.len() < 2 {
            return Ok(false);
        }
        if self
            .world
            .routing
            .routes
            .get(&asked)
            .and_then(|r| r.contexts.last())
            .is_some_and(|c| c.objects[1] != 0)
        {
            return Ok(false);
        }
        let object = self
            .world
            .objects
            .get(&asked)
            .ok_or(Trap::native(10, "missing object"))?;
        let (footprint, position) = (object.footprint, object.position);
        let mut phase = 0;
        let (mut enter, mut exit) = ([0; 2], [0; 2]);
        'segments: for pair in path.windows(2) {
            for point in samples(pair[0], pair[1]) {
                if holds(footprint, point) {
                    if phase == 0 {
                        enter = point;
                        phase = 1;
                    }
                } else if phase == 1 {
                    exit = point;
                    phase = 2;
                    break 'segments;
                }
            }
        }
        match phase {
            0 => {
                enter = path[0];
                exit = [position[1], position[0]];
            }
            1 => exit = path[path.len() - 1],
            _ => {}
        }
        if self.catalog.object(MARKER).is_none() {
            return Ok(false);
        }
        let interpreter = self.interpreter();
        let wrap = |fault: Fault| {
            Trap::Unsupported(format!(
                "asking person {asked} to move out of the way (0x45bf2e): {fault}"
            ))
        };
        let marker = interpreter.create(self.world, MARKER).map_err(wrap)?;
        let at = [position[0], position[1]];
        interpreter
            .put(
                self.world,
                marker,
                Place {
                    on_lot: on_lot(at),
                    position: at,
                    container: None,
                    slot: -1,
                    level: position[2],
                },
                true,
            )
            .map_err(wrap)?;
        let d = [
            exit[0].wrapping_sub(enter[0]),
            exit[1].wrapping_sub(enter[1]),
        ];
        if d == [0, 0] {
            // 0x45c184: the marker stays on the lot.
            return Ok(false);
        }
        let dir = direction([0, 0], d);
        let mut asked_ctx = new_context(self.world, asked, marker, move_slot(dir, false));
        interpreter
            .find_candidates(self.world, &mut asked_ctx)
            .map_err(wrap)?;
        let mut scratch = Vec::new();
        let reachable = interpreter
            .plan(self.world, &mut asked_ctx, &mut scratch)
            .map_err(wrap)?;
        self.world
            .routing
            .slots
            .insert((marker, 0), move_slot(dir, !reachable));

        let priority = self
            .world
            .objects
            .get(&ctx.person)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.data.get(PRIORITY).copied())
            .ok_or_else(|| Trap::Invalid("the routing person has no data word 33".into()))?;
        let visitor = self
            .world
            .objects
            .get(&asked)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.data.get(person::VISITOR).copied())
            .ok_or_else(|| Trap::Invalid("the asked person has no data word 32".into()))?
            != 0;
        let entry = self.table(marker)?.and_then(|t| find(t, MOVE as u32));
        let constants = self.program.autonomy_constants();
        let action = Interaction {
            uid: 0,
            caller: asked as i16,
            target: marker as i16,
            icon: marker as i16,
            table_index: MOVE,
            args: [0; 4],
            priority: i32::from(priority),
            action: entry.map_or(0, |e| e.action),
            attenuation: entry.map_or(0.0, |e| attenuation(&constants, visitor, e).to_f32()),
            flags: entry.map_or(0, |e| u32::from(e.flags & ENTRY_FLAG != 0)),
        };
        let at_once = entry.map(|e| e.flags & AT_ONCE != 0);
        let Some(uid) = self.queue_action(asked, action, at_once)? else {
            interpreter.remove_now(self.world, marker).map_err(wrap)?;
            return Ok(false);
        };
        ctx.byte_84 = false;
        ctx.objects[1] = asked;
        ctx.action = uid;
        Ok(true)
    }

    /// Whether a person's running action or a queued one is a marker's
    /// move interaction (0x45bec1).
    fn asked_to_move(&self, id: u16) -> Result<bool, Trap> {
        let person = self
            .world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .ok_or(Trap::native(0x1c, "object is not a person"))?;
        let moving = |action: &Interaction| {
            u16::try_from(action.target)
                .ok()
                .filter(|&t| t != 0)
                .and_then(|t| self.world.objects.get(&t))
                .is_some_and(|o| o.guid == MARKER && action.table_index == MOVE)
        };
        Ok(moving(&person.active) || person.queue.iter().any(moving))
    }

    /// Queues an action on a person (0x45d20b) and returns its number, or
    /// None when it was not queued. `at_once` is the entry's flag 4, None
    /// with no entry (the action is not queued).
    ///
    /// The action takes the next number ([`sims_sim::World::action_serial`],
    /// 0x41f158) and goes after every queued action of at least its
    /// priority. A full queue takes nothing. An entry that runs at once,
    /// an action put at the front (flag 2), and one that goes before
    /// queued actions (which are dropped, each object told with its
    /// function 4) fail loudly. The original's branch for an action queued
    /// while one runs at once (0x6e5294) is never taken here, as running
    /// at once is not ported.
    pub(crate) fn queue_action(
        &mut self,
        id: u16,
        mut action: Interaction,
        at_once: Option<bool>,
    ) -> Result<Option<i32>, Trap> {
        match at_once {
            None => return Ok(None),
            Some(true) => {
                return Err(unsupported(
                    "0x45d240",
                    "queueing an action that runs at once",
                ));
            }
            Some(false) => {}
        }
        self.world.action_serial = self.world.action_serial.wrapping_add(1);
        action.uid = self.world.action_serial;
        if action.flags & TO_FRONT != 0 {
            return Err(unsupported(
                "0x45d322",
                "queueing an action at the queue's front",
            ));
        }
        let queue = &mut self
            .world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or(Trap::native(0x1c, "object is not a person"))?
            .queue;
        let mut at = queue.len();
        while at > 0 && queue[at - 1].priority < action.priority {
            at -= 1;
        }
        if at < queue.len() {
            return Err(unsupported(
                "0x45d4c8",
                "queueing an action before others, which are dropped",
            ));
        }
        if queue.len() >= QUEUE {
            return Ok(None);
        }
        let uid = action.uid;
        queue.push(action);
        Ok(Some(uid))
    }

    /// Takes the action numbered `uid` off a person's queue (0x45d5f8),
    /// as a route that asked the person to move ends. Each queued action
    /// with the number is dropped and its object, if any, runs its
    /// function 4 with the person as the stack object. Returns whether
    /// one was dropped.
    ///
    /// Every function 4 runs while the whole queue, the dropped actions
    /// included, is still in place: the original walks the live queue
    /// (0x45d697), calling function 4 for each match and copying the rest
    /// aside (0x45f744), and only swaps the copy in after the walk
    /// (0x45d70b). Other cancellations remove the action first; this one
    /// does not.
    ///
    /// When the action is the person's running one, the original instead
    /// lowers its priority and may queue the person's own interaction 0x21;
    /// that fails loudly. So does a function 4 that changes the queue.
    pub(crate) fn release_action(&mut self, id: u16, uid: i32) -> Result<bool, Trap> {
        let person = self
            .world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .ok_or(Trap::native(0x1c, "object is not a person"))?;
        if person.active.uid == uid {
            return Err(unsupported(
                "0x45d617",
                "ending while the person it asked to move is moving",
            ));
        }
        let queue = person.queue.clone();
        let interpreter = self.interpreter();
        for action in queue.iter().filter(|a| a.uid == uid) {
            if action.target != 0 {
                interpreter
                    .callback(self.world, action.target as u16, CANCELLED, id as i16)
                    .map_err(|fault| {
                        Trap::Unsupported(format!(
                            "cancelling action {uid} of person {id} (0x45d6c4): {fault}"
                        ))
                    })?;
            }
        }
        let person = self
            .world
            .objects
            .get_mut(&id)
            .and_then(|o| o.person.as_mut())
            .ok_or(Trap::native(0x1c, "object is not a person"))?;
        let key = |q: &[Interaction]| -> Vec<(i32, i16, i32)> {
            q.iter().map(|a| (a.uid, a.target, a.table_index)).collect()
        };
        if key(&person.queue) != key(&queue) {
            return Err(unsupported(
                "0x45d6c4",
                "whose cancelled action's object changed the queue",
            ));
        }
        let before = person.queue.len();
        person.queue.retain(|a| a.uid != uid);
        Ok(person.queue.len() != before)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_is_sampled_once_per_unit_of_its_longer_side() {
        // Three points for a side of 2, at t = 0, 1/3 and 2/3: the end
        // point is not reached.
        assert_eq!(samples([0, 0], [2, 0]), vec![[0, 0], [1, 0], [1, 0]]);
        assert_eq!(samples([5, 5], [5, 5]), vec![[5, 5]]);
        // A third: t is 0, 1/3, 2/3 (each rounded to single precision),
        // and the sum stops below 1.
        assert_eq!(samples([0, 0], [-2, 1]), vec![[0, 0], [0, 0], [0, 1]]);
    }

    #[test]
    fn a_footprint_holds_points_by_its_swapped_sides() {
        // Least first 10, least second 20, greatest first 16, greatest
        // second 26; a point's first coordinate is the second.
        let footprint = [10, 20, 16, 26];
        assert!(holds(footprint, [20, 10]));
        assert!(holds(footprint, [25, 15]));
        assert!(!holds(footprint, [26, 15]));
        assert!(!holds(footprint, [25, 16]));
        assert!(!holds(footprint, [10, 20]));
    }

    #[test]
    fn the_move_slot_lies_square_to_the_way() {
        let slot = move_slot(0, false);
        assert_eq!(slot.flags, (1 << 2) | (1 << 6) | 0x100 | 0x2000);
        assert_eq!(slot.proximity, [16, 80, 16]);
        assert_eq!((slot.standing, slot.facing, slot.resolution), (1, -3, 8));
        assert_eq!(slot.number, -1);
        let wide = move_slot(7, true);
        assert_eq!(wide.flags, (1 << 1) | (1 << 5) | (1 << 7) | 0x100 | 0x2000);
        assert_eq!(wide.proximity, [16, 80, 80]);
    }
}
