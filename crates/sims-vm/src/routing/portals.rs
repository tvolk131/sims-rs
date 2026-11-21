//! The routing between rooms: which door a person goes through to reach
//! an object in another room (0x45c7c2 and the routines it calls).
//!
//! A door (an object of type 8 with a tree for function 15, the door
//! function) is a portal: a way from its room to the room of its other
//! side, the next part of its group with the door function (0x4636ae,
//! `partner` in `rooms.rs`). The lot keeps its portals in a list in the
//! order they were added to it (0x4436ef pushes each one as the object
//! joins the lot). A memory dump of the running game at the entry of
//! update 233 of the Goth house shows the list in the order of
//! [`World::order`], which is what [`Interpreter::portals`] gives.
//!
//! Each portal holds a vector of costs (object +0x1ac, saved with the
//! house: [`sims_sim::Object::portal`]), one column per number handed out
//! from a pool shared by people and portals and kept in object data
//! word 6 (0x46381a: the smallest positive number that no person on the
//! lot and no portal holds, below 10000). A person's column holds the
//! cost of going to the target through each portal of the person's
//! room; a portal's column holds the shortest distance from that portal
//! to every other portal ([`Interpreter::portal_distance`]).
//!
//! The values in the columns:
//!
//! - 10001: not worked out (0x667478).
//! - 10000: no way through (0x667474).
//! - 1101: tried and failed (0x667470).
//! - 1001 to 1100: tried, in the order of the tries (0x667468, 0x66746c).
//! - up to 1000: the cost (0x667464 caps it).
//!
//! The arithmetic is the FPU's, which runs in single precision during a
//! lot update (`docs/decisions/`): every add and square root is rounded
//! to single precision, and every value is stored as a float.
//!
//! Ported from the original's code at the addresses given. The old
//! prototype (legacy/crates/sims-sim/src/portals.rs: `Cache::choose`,
//! `Cache::attempt`, `Cache::fail`, `distances`; lot_route/portal.rs:
//! `dispatch`, `Host::reset`) compared the same rules with the original's
//! code run alone, in double precision.

use crate::fault::Trap;
use crate::interpreter::Interpreter;
use crate::primitives::distance::distance;
use sims_core::{Ext, F32};
use sims_sim::World;

/// Object data words.
const ORDINAL: usize = 6;
const ROOM: usize = 29;

/// An object's type: a door.
const DOOR: i16 = 8;
/// The door function: definition callback 15.
pub(crate) const DOOR_FUNCTION: usize = 15;
/// The object type whose distance is always zero (0x434148).
const NO_DISTANCE: i16 = 7;

/// The values a column holds (see the module's documentation).
const MAX_COST: u32 = 0x447a_0000; // 1000.0
const FIRST_TRY: u32 = 0x447a_4000; // 1001.0
const LAST_TRY: u32 = 0x4489_8000; // 1100.0
const FAILED: u32 = 0x4489_a000; // 1101.0
const NO_WAY: u32 = 0x461c_4000; // 10000.0
const UNKNOWN: u32 = 0x461c_4400; // 10001.0

/// The last number the pool hands out, plus one (0x46386e).
const POOL_END: i16 = 10000;

fn value(bits: u32) -> Ext {
    F32::from_bits(bits).ext()
}

fn unsupported(at: &str, what: &str) -> Trap {
    Trap::Unsupported(format!("{what} ({at}), which is not ported"))
}

fn word(world: &World, id: u16, index: usize) -> Result<i16, Trap> {
    world
        .objects
        .get(&id)
        .ok_or(Trap::native(10, "missing object"))?
        .data
        .get(index)
        .copied()
        .ok_or_else(|| Trap::Invalid(format!("object data has no word {index}")))
}

fn set_word(world: &mut World, id: u16, index: usize, value: i16) -> Result<(), Trap> {
    let slot = world
        .objects
        .get_mut(&id)
        .ok_or(Trap::native(10, "missing object"))?
        .data
        .get_mut(index)
        .ok_or_else(|| Trap::Invalid(format!("object data has no word {index}")))?;
    *slot = value;
    Ok(())
}

/// A column's value as 0x463c50 reads it: 0.0 for a number below 1 or
/// past the end of the vector.
fn column(world: &World, portal: u16, ordinal: i16) -> Result<Ext, Trap> {
    let costs = world
        .objects
        .get(&portal)
        .ok_or(Trap::native(10, "missing object"))?
        .portal
        .as_deref()
        .unwrap_or(&[]);
    Ok(usize::try_from(ordinal)
        .ok()
        .filter(|&n| n >= 1)
        .and_then(|n| costs.get(n - 1))
        .map_or(Ext::ZERO, |&v| F32::from_f32(v).ext()))
}

/// Writes a column (0x462fff): the vector grows with 10001 until it has
/// the column, and the value is stored as a float.
///
/// Number 0 would write before the vector, which fails loudly. The
/// original also redraws the portal (0x434101) when the number equals the
/// word at 0x6e52d2, which a debug view sets; nothing here sets it, and it
/// is 0 in a memory dump of the running game.
fn set_column(world: &mut World, portal: u16, ordinal: i16, cost: Ext) -> Result<(), Trap> {
    let index = usize::try_from(ordinal)
        .ok()
        .filter(|&n| n >= 1)
        .ok_or_else(|| {
            unsupported(
                "0x46302f",
                "writing a portal cost for number 0, which writes before the vector",
            )
        })?;
    let object = world
        .objects
        .get_mut(&portal)
        .ok_or(Trap::native(10, "missing object"))?;
    let costs = object.portal.get_or_insert_with(Vec::new);
    while costs.len() < index {
        costs.push(F32::from_bits(UNKNOWN).to_f32());
    }
    costs[index - 1] = cost.to_f32().to_f32();
    Ok(())
}

impl Interpreter<'_> {
    /// Whether an object is a portal: a door with the door function.
    pub(crate) fn is_portal(&self, world: &World, id: u16) -> Result<bool, Trap> {
        Ok(self.kind(world, id)? == DOOR && self.has_function(world, id, DOOR_FUNCTION))
    }

    /// The tree a portal's definition names for the door function, or 0
    /// (0x457c38, 0x455fa9).
    pub(crate) fn door_tree(&self, world: &World, id: u16) -> u16 {
        world
            .objects
            .get(&id)
            .and_then(|object| {
                crate::interpreter::object_definition(self.catalog, world, object.guid)
            })
            .and_then(|definition| {
                self.program
                    .libraries
                    .get(&definition.resource)
                    .and_then(|l| l.callbacks.get(&definition.chunk))
            })
            .and_then(|table| table.get(DOOR_FUNCTION))
            .map_or(0, |f| f.action)
    }

    /// The lot's portals, in the order of the lot's list (see the
    /// module's documentation).
    pub(crate) fn portals(&self, world: &World) -> Result<Vec<u16>, Trap> {
        let mut list = Vec::new();
        for &id in &world.order {
            if world.objects.contains_key(&id) && self.is_portal(world, id)? {
                list.push(id);
            }
        }
        Ok(list)
    }

    /// The smallest positive number no person on the lot and no portal
    /// holds in data word 6, or 0 if every number below 10000 is taken
    /// (0x46381a).
    fn free_ordinal(&self, world: &World, portals: &[u16]) -> Result<i16, Trap> {
        let mut used = std::collections::BTreeSet::new();
        for (&id, object) in &world.objects {
            if object.person.is_some() {
                used.insert(word(world, id, ORDINAL)?);
            }
        }
        for &id in portals {
            used.insert(word(world, id, ORDINAL)?);
        }
        Ok((1..POOL_END).find(|n| !used.contains(n)).unwrap_or(0))
    }

    /// The person's number for the portal costs, given one first if it has
    /// none (0x4638b9): then every portal's column for the number becomes
    /// 10001.
    pub(crate) fn reset_portal_costs(&self, world: &mut World, person: u16) -> Result<(), Trap> {
        let portals = self.portals(world)?;
        let mut ordinal = word(world, person, ORDINAL)?;
        if ordinal == 0 {
            ordinal = self.free_ordinal(world, &portals)?;
            set_word(world, person, ORDINAL, ordinal)?;
        }
        for &portal in &portals {
            set_column(world, portal, ordinal, value(UNKNOWN))?;
        }
        Ok(())
    }

    /// The shortest distance from portal `start` to portal `to`, through
    /// rooms and doors (0x46390f).
    ///
    /// The start portal gets a number if it has none, and every portal's
    /// column for it becomes 10001. If the start's own column is 10001
    /// (or the number is past its vector), the distances are worked out
    /// again: every portal's column becomes 10000 and the start's 0. Then,
    /// from a list holding the start, the portal with the lowest cost is
    /// taken off the list (scanning from the end with a strict compare, so
    /// the last of equals wins), and each portal in its room, and its other
    /// side, whose cost is above its own gets the cost of going there
    /// (the distance, plus its own cost, plus 1) if that is lower, and
    /// joins the list. The result is `to`'s column.
    pub(crate) fn portal_distance(
        &self,
        world: &mut World,
        start: u16,
        to: u16,
    ) -> Result<Ext, Trap> {
        let portals = self.portals(world)?;
        let mut ordinal = word(world, start, ORDINAL)?;
        if ordinal == 0 {
            ordinal = self.free_ordinal(world, &portals)?;
            set_word(world, start, ORDINAL, ordinal)?;
            for &portal in &portals {
                set_column(world, portal, ordinal, value(UNKNOWN))?;
            }
        }
        let length = world.objects[&start].portal.as_ref().map_or(0, Vec::len);
        // 0x46398c compares the number with the length without sign.
        let stale = usize::try_from(ordinal).map_or(true, |n| n > length)
            || column(world, start, ordinal)? == value(UNKNOWN);
        if stale {
            for &portal in &portals {
                set_column(world, portal, ordinal, value(NO_WAY))?;
            }
            set_column(world, start, ordinal, Ext::ZERO)?;
            let mut pending = vec![start];
            while !pending.is_empty() {
                let mut best = pending.len() - 1;
                for i in (0..best).rev() {
                    if column(world, pending[i], ordinal)? < column(world, pending[best], ordinal)?
                    {
                        best = i;
                    }
                }
                let from = pending.remove(best);
                let partner = self.partner(world, from)?;
                let room = word(world, from, ROOM)?;
                for &next in &portals {
                    if word(world, next, ROOM)? != room && next != partner {
                        continue;
                    }
                    let own = column(world, from, ordinal)?;
                    if column(world, next, ordinal)? <= own {
                        continue;
                    }
                    let own = own.to_f32().ext();
                    let cost = ((self.portal_span(world, from, next)? + own).single()
                        + Ext::from_i32(1))
                    .single();
                    if cost < column(world, next, ordinal)? {
                        set_column(world, next, ordinal, cost)?;
                        pending.push(next);
                    }
                }
            }
        }
        column(world, to, ordinal)
    }

    /// The distance between two objects (0x434148), 0 to an object of
    /// type 7.
    fn portal_span(&self, world: &World, from: u16, to: u16) -> Result<Ext, Trap> {
        let a = world
            .objects
            .get(&from)
            .ok_or(Trap::native(10, "missing object"))?;
        let b = world
            .objects
            .get(&to)
            .ok_or(Trap::native(10, "missing object"))?;
        let zero = self.kind(world, to)? == NO_DISTANCE;
        Ok(distance(a.position, b.position, zero))
    }

    /// The portal of the person's room to go through to reach `target`, or
    /// None (0x463c83).
    ///
    /// A person without a number gets none. For each portal of the
    /// person's room, in the list's order: if its cost in the person's
    /// column is 10001, it becomes 10000, and for each portal of the
    /// target's room that can be reached from it, and is no nearer its
    /// other side than it, the cost becomes the lowest of: the distance
    /// from that portal to the target, plus the distance through, plus the
    /// person's distance to the portal, capped at 1000. Then the portal
    /// with the lowest cost that is neither 10000 nor 1101 is chosen; the
    /// first of equals wins.
    pub(crate) fn choose_portal(
        &self,
        world: &mut World,
        person: u16,
        target: u16,
    ) -> Result<Option<u16>, Trap> {
        let ordinal = word(world, person, ORDINAL)?;
        if ordinal == 0 {
            return Ok(None);
        }
        let portals = self.portals(world)?;
        let room = word(world, person, ROOM)?;
        let target_room = word(world, target, ROOM)?;
        let mut best: Option<u16> = None;
        for &portal in &portals {
            if word(world, portal, ROOM)? != room {
                continue;
            }
            if column(world, portal, ordinal)? == value(UNKNOWN) {
                set_column(world, portal, ordinal, value(NO_WAY))?;
                let approach = self.portal_span(world, person, portal)?.to_f32().ext();
                for &exit in &portals {
                    if word(world, exit, ROOM)? != target_room {
                        continue;
                    }
                    let through = self.portal_distance(world, portal, exit)?.to_f32().ext();
                    if through == value(NO_WAY) {
                        continue;
                    }
                    let partner = self.partner(world, portal)?;
                    if self.portal_distance(world, partner, exit)? > through {
                        continue;
                    }
                    let mut cost = ((self.portal_span(world, exit, target)? + through).single()
                        + approach)
                        .single()
                        .to_f32()
                        .ext();
                    if cost > value(MAX_COST) {
                        cost = value(MAX_COST);
                    }
                    if column(world, portal, ordinal)? > cost {
                        set_column(world, portal, ordinal, cost)?;
                    }
                }
            }
            let cost = column(world, portal, ordinal)?;
            if cost == value(NO_WAY) || cost == value(FAILED) {
                continue;
            }
            match best {
                Some(b) if column(world, b, ordinal)? <= cost => {}
                _ => best = Some(portal),
            }
        }
        Ok(best)
    }

    /// Notes that the person tries a portal (0x463e43). A portal already
    /// tried (1001 to 1100) is marked failed (1101), with its other side;
    /// a failed one stays failed; any other gets, with its other side, one
    /// more than the highest try among the portals of the person's room,
    /// or 1001.
    pub(crate) fn try_portal(
        &self,
        world: &mut World,
        person: u16,
        portal: u16,
    ) -> Result<(), Trap> {
        let ordinal = word(world, person, ORDINAL)?;
        let cost = column(world, portal, ordinal)?;
        let tried = |c: Ext| c >= value(FIRST_TRY) && c <= value(LAST_TRY);
        let mark = if tried(cost) {
            value(FAILED)
        } else if cost == value(FAILED) {
            return Ok(());
        } else {
            let room = word(world, person, ROOM)?;
            let mut next = value(FIRST_TRY);
            for other in self.portals(world)? {
                if word(world, other, ROOM)? != room {
                    continue;
                }
                let c = column(world, other, ordinal)?;
                if tried(c) && c >= next {
                    next = (c + Ext::from_i32(1)).single().to_f32().ext();
                }
            }
            next
        };
        set_column(world, portal, ordinal, mark)?;
        let partner = self.partner(world, portal)?;
        set_column(world, partner, ordinal, mark)
    }

    /// Marks a portal and its other side failed in the person's column
    /// (0x463e09).
    pub(crate) fn fail_portal(
        &self,
        world: &mut World,
        person: u16,
        portal: u16,
    ) -> Result<(), Trap> {
        let ordinal = word(world, person, ORDINAL)?;
        set_column(world, portal, ordinal, value(FAILED))?;
        let partner = self.partner(world, portal)?;
        set_column(world, partner, ordinal, value(FAILED))
    }
}

#[cfg(test)]
mod tests {
    //! A synthetic lot, built in memory: no game data. Three rooms, 1 to
    //! 3, joined by three doors of two tiles each: A (10 in room 1, 11 in
    //! room 2), B (12 in room 1, 13 in room 3) and C (14 in room 3, 15 in
    //! room 2). Person 2 stands in room 1 and its target, object 3, in
    //! room 2. Positions are in sixteenths of a tile.
    use super::*;
    use crate::{DEFINITION_WORDS, Library, Program, Stream};
    use sims_content::Catalog;
    use sims_formats::execution::ObjectFunction;
    use sims_formats::saved_objects::{Interaction, Multipart, Person};
    use sims_sim::Object;
    use std::collections::BTreeMap;

    const PERSON: u16 = 2;
    const TARGET: u16 = 3;
    const DOORS: [(u16, u16); 3] = [(10, 11), (12, 13), (14, 15)];
    const GUIDS: [(u32, i16); 3] = [(0x100, 2), (0x200, 4), (0x300, DOOR)];

    fn program() -> Program {
        let mut libraries = BTreeMap::new();
        for (resource, &(guid, kind)) in GUIDS.iter().enumerate() {
            let mut words = [0; DEFINITION_WORDS];
            words[9] = kind;
            words[14] = guid as u16 as i16;
            let mut callbacks = vec![
                ObjectFunction {
                    condition: 0,
                    action: 0
                };
                16
            ];
            if kind == DOOR {
                callbacks[DOOR_FUNCTION].action = 4096;
            }
            libraries.insert(
                resource,
                Library {
                    callbacks: BTreeMap::from([(128, callbacks)]),
                    definitions: BTreeMap::from([(128, words)]),
                    definition_order: vec![128],
                    ..Library::default()
                },
            );
        }
        Program {
            global: 0,
            semi_globals: (0..GUIDS.len()).map(|r| (r, None)).collect(),
            libraries,
            careers: None,
            career_texts_end: false,
            labels_hold_no_token: false,
            zoning: BTreeMap::new(),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: Default::default(),
        }
    }

    fn catalog() -> Catalog {
        Catalog {
            global: 0,
            objects: GUIDS
                .iter()
                .enumerate()
                .map(|(resource, &(guid, kind))| sims_content::Object {
                    guid,
                    resource,
                    chunk: 128,
                    kind: kind as u16,
                    master: 0,
                })
                .collect(),
            by_guid: GUIDS
                .iter()
                .enumerate()
                .map(|(i, &(g, _))| (g, i))
                .collect(),
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }

    fn interaction() -> Interaction {
        Interaction {
            join: 0,
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: 0,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: 0.0,
            flags: 0,
        }
    }

    fn object(id: u16, guid: u32, room: i16, x: i32, y: i32) -> Object {
        let mut data = vec![0; 72];
        data[ROOM] = room;
        Object {
            id,
            guid,
            footprint: [0; 4],
            position: [x, y, 1],
            unknown_short: 0,
            attributes: Vec::new(),
            temps: vec![0; 8],
            data,
            obsolete_word: 0,
            stack_capacity_bytes: 0,
            stack: Vec::new(),
            relationship_flag: -1,
            relationships: Vec::new(),
            slots: Vec::new(),
            dynamic_sprites: Vec::new(),
            multipart: None,
            portal: None,
            person: None,
            registry_word: None,
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        }
    }

    fn world() -> World {
        let mut world = World::empty();
        let mut person = object(PERSON, GUIDS[0].0, 1, 0, 0);
        person.person = Some(Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: [0.0; 16],
            motives: [0.0; 16],
            data: vec![0; 256],
            routing_frames: 0,
            active: interaction(),
            last: interaction(),
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        });
        world.objects.insert(PERSON, person);
        world
            .objects
            .insert(TARGET, object(TARGET, GUIDS[1].0, 2, 64, 0));
        world.order = vec![PERSON, TARGET];
        world.people = vec![PERSON];
        let tiles = [
            (10, 1, 32, 0),
            (11, 2, 48, 0),
            (12, 1, 0, 32),
            (13, 3, 0, 48),
            (14, 3, 0, 160),
            (15, 2, 160, 160),
        ];
        for (id, room, x, y) in tiles {
            world.objects.insert(id, object(id, GUIDS[2].0, room, x, y));
            world.order.push(id);
        }
        for (lead, other) in DOORS {
            for (id, parent) in [(lead, 0), (other, lead as i16)] {
                world.objects.get_mut(&id).unwrap().multipart = Some(Multipart {
                    offset: [0; 3],
                    group: [0; 3],
                    parent,
                });
            }
            world.next_parts.insert(lead, other);
        }
        world
    }

    fn cost(world: &World, portal: u16, ordinal: usize) -> f32 {
        world.objects[&portal].portal.as_ref().unwrap()[ordinal - 1]
    }

    #[test]
    fn a_person_gets_a_number_and_every_portal_a_column_for_it() {
        let (program, catalog) = (program(), catalog());
        let interpreter = Interpreter::new(&program, &catalog, Stream::Lot);
        let mut world = world();
        assert_eq!(
            interpreter.portals(&world).unwrap(),
            [10, 11, 12, 13, 14, 15]
        );
        // Portals hold numbers 1 and 3, so the person gets 2.
        world.objects.get_mut(&11).unwrap().data[ORDINAL] = 1;
        world.objects.get_mut(&TARGET).unwrap().data[ORDINAL] = 2;
        world.objects.get_mut(&14).unwrap().data[ORDINAL] = 3;
        // The target is not a person and does not hold its number.
        interpreter.reset_portal_costs(&mut world, PERSON).unwrap();
        assert_eq!(world.objects[&PERSON].data[ORDINAL], 2);
        for (lead, other) in DOORS {
            for id in [lead, other] {
                assert_eq!(world.objects[&id].portal, Some(vec![10001.0; 2]));
            }
        }
        // A person with a number keeps it; its column is reset.
        world.objects.get_mut(&10).unwrap().portal = Some(vec![5.0, 7.0, 9.0]);
        interpreter.reset_portal_costs(&mut world, PERSON).unwrap();
        assert_eq!(world.objects[&PERSON].data[ORDINAL], 2);
        assert_eq!(world.objects[&10].portal, Some(vec![5.0, 10001.0, 9.0]));
    }

    #[test]
    fn the_distance_between_portals_goes_through_rooms_and_doors() {
        let (program, catalog) = (program(), catalog());
        let interpreter = Interpreter::new(&program, &catalog, Stream::Lot);
        let mut world = world();
        // From A's near side: across the door (1 tile, plus 1).
        let d = interpreter.portal_distance(&mut world, 10, 11).unwrap();
        assert_eq!(d.to_f32().to_f32(), 2.0);
        // The start got number 1; everything reachable has a cost.
        assert_eq!(world.objects[&10].data[ORDINAL], 1);
        assert_eq!(cost(&world, 10, 1), 0.0);
        // To B's near side, in the same room: 2 times the square root of
        // 2, plus 1; then across B: 1 more, plus 1.
        let twelve = (8f32.sqrt() + 1.0) as f64;
        assert!((f64::from(cost(&world, 12, 1)) - twelve).abs() < 1e-6);
        assert!((f64::from(cost(&world, 13, 1)) - (twelve + 2.0)).abs() < 1e-6);
        // C's far side (15) is reached through room 2 more cheaply than
        // through room 3 (14 then across C).
        let fifteen = f64::from(38144f32.sqrt() / 16.0) + 3.0;
        assert!((f64::from(cost(&world, 15, 1)) - fifteen).abs() < 1e-5);
        assert!((f64::from(cost(&world, 14, 1)) - (twelve + 2.0 + 8.0)).abs() < 1e-5);
        // A second call reads the column without working it out again.
        world.objects.get_mut(&11).unwrap().portal.as_mut().unwrap()[0] = 77.0;
        let d = interpreter.portal_distance(&mut world, 10, 11).unwrap();
        assert_eq!(d.to_f32().to_f32(), 77.0);
    }

    #[test]
    fn a_person_goes_through_the_cheapest_door_and_then_the_next() {
        let (program, catalog) = (program(), catalog());
        let interpreter = Interpreter::new(&program, &catalog, Stream::Lot);
        let mut world = world();
        // Without a number, no door.
        assert_eq!(
            interpreter
                .choose_portal(&mut world, PERSON, TARGET)
                .unwrap(),
            None
        );
        interpreter.reset_portal_costs(&mut world, PERSON).unwrap();
        assert_eq!(world.objects[&PERSON].data[ORDINAL], 1);
        // Through A: 2 tiles to the door, 2 through it, 1 to the target.
        // Through B the far side (13) is farther from A's far side than B
        // is, so only the way round through C counts.
        assert_eq!(
            interpreter
                .choose_portal(&mut world, PERSON, TARGET)
                .unwrap(),
            Some(10)
        );
        assert_eq!(cost(&world, 10, 1), 5.0);
        let through_c = cost(&world, 12, 1);
        assert!(through_c > 32.0 && through_c < 33.0, "{through_c}");
        // Doors in other rooms keep the person's column unworked.
        assert_eq!(cost(&world, 14, 1), 10001.0);
        // The first try gets 1001 on both sides; a second marks it failed.
        interpreter.try_portal(&mut world, PERSON, 10).unwrap();
        assert_eq!((cost(&world, 10, 1), cost(&world, 11, 1)), (1001.0, 1001.0));
        interpreter.try_portal(&mut world, PERSON, 10).unwrap();
        assert_eq!((cost(&world, 10, 1), cost(&world, 11, 1)), (1101.0, 1101.0));
        // A failed door stays failed and is not chosen.
        interpreter.try_portal(&mut world, PERSON, 10).unwrap();
        assert_eq!(cost(&world, 10, 1), 1101.0);
        assert_eq!(
            interpreter
                .choose_portal(&mut world, PERSON, TARGET)
                .unwrap(),
            Some(12)
        );
        // A try counts up from the highest try in the person's room.
        world.objects.get_mut(&10).unwrap().portal.as_mut().unwrap()[0] = 1004.0;
        interpreter.try_portal(&mut world, PERSON, 12).unwrap();
        assert_eq!((cost(&world, 12, 1), cost(&world, 13, 1)), (1005.0, 1005.0));
        // Failing the last door leaves none.
        world.objects.get_mut(&10).unwrap().portal.as_mut().unwrap()[0] = 1101.0;
        interpreter.fail_portal(&mut world, PERSON, 12).unwrap();
        assert_eq!((cost(&world, 12, 1), cost(&world, 13, 1)), (1101.0, 1101.0));
        assert_eq!(
            interpreter
                .choose_portal(&mut world, PERSON, TARGET)
                .unwrap(),
            None
        );
    }
}
