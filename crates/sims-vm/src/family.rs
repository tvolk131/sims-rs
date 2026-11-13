//! Generic Sims Call 6 / 42601e and loaded-person refresh 42a2b4.
//! Membership uses GUIDs; the primitive operand is a neighbor slot, never a
//! physical object ID. Windows panel and broadcast subscribers are host services.
use crate::{ObjectId, World};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub flags: u32,
    pub dirty: bool,
    /// Native +120; absent historical input is unknown, never a zero cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friend_count: Option<i32>,
    /// Native +12c GUID owner, independent of the resolved neighbor-ID adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_guids: Option<Vec<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub family: i16,
    pub neighbor: i16,
    pub guid: u32,
}

/// Observation points correspond to native helper entries, before their writes.
#[derive(Debug, Clone, Copy)]
pub enum Stage {
    Member,
    Live,
    Erase,
    Visitors,
}

pub trait Host {
    fn close_panel(&mut self, world: &mut World) -> Result<(), String>;
    fn residential(&mut self, world: &World) -> Result<bool, String>;
    fn observe(&mut self, _stage: Stage, _world: &World) {}
    fn broadcast(&mut self, world: &mut World, change: Change) -> Result<(), String>;
}

fn member(world: &World, family: i16, guid: u32) -> Result<Option<usize>, String> {
    let people = world.person.as_ref().ok_or("family neighborhood absent")?;
    let members = people
        .families
        .get(&family)
        .ok_or("selected family absent")?;
    if let Some(guids) = people
        .family_metadata
        .get(&family)
        .and_then(|m| m.member_guids.as_ref())
    {
        if guids.len() != members.len() {
            return Err("native family GUID adapter length differs".into());
        }
        return Ok(guids.iter().position(|value| *value == guid));
    }
    for (i, id) in members.iter().enumerate() {
        let neighbor = people
            .neighbors
            .get(id)
            .ok_or("family member neighbor absent")?;
        if neighbor.guid == guid {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

fn live_guid(world: &World, id: ObjectId) -> Result<u32, String> {
    let object = world.objects.get(&id).ok_or("family live person absent")?;
    if object.person.is_none() {
        return Err("family live registry contains nonperson".into());
    }
    object
        .definition
        .as_ref()
        .map(|d| d.guid)
        .ok_or("family live person definition absent".into())
}

/// Preserve nonzero visitor modes for nonmembers; controlled membership wins
/// regardless of a stale per-person family word. Every person queries zoning.
pub fn refresh_visitors(world: &mut World, host: &mut impl Host) -> Result<(), String> {
    host.observe(Stage::Visitors, world);
    let people = world.person.as_ref().ok_or("family neighborhood absent")?;
    let selected = people.controlled_family;
    let order = people.person_order.clone();
    for id in order {
        let guid = live_guid(world, id)?;
        let current = *world.objects[&id]
            .person
            .as_ref()
            .unwrap()
            .get(&32)
            .ok_or("family visitor field absent")?;
        let mut visitor = if current == 0 { 1 } else { current };
        if member(world, selected, guid)?.is_some() {
            visitor = 0;
        }
        if !host.residential(world)? && visitor == 0 {
            visitor = 1;
        }
        world
            .objects
            .get_mut(&id)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(32, visitor);
    }
    Ok(())
}

pub fn remove(world: &mut World, neighbor: i16, host: &mut impl Host) -> Result<bool, String> {
    let people = world.person.as_ref().ok_or("family neighborhood absent")?;
    people.validate_neighbor_slots()?;
    let slots = people
        .neighbor_slots
        .as_ref()
        .ok_or("family neighborhood slots not supplied")?;
    let slot = usize::try_from(neighbor)
        .ok()
        .and_then(|n| n.checked_sub(1));
    let id = slot
        .and_then(|i| slots.get(i).copied().flatten())
        .ok_or("family removal neighbor absent (native error 0x37)")?;
    host.close_panel(world)?;
    remove_resolved(world, id, host)
}

fn remove_resolved(world: &mut World, id: i16, host: &mut impl Host) -> Result<bool, String> {
    // 42601e reads the retained neighbor after the panel callback has returned.
    let record = world
        .person
        .as_ref()
        .unwrap()
        .neighbors
        .get(&id)
        .ok_or("family neighbor slot record absent")?;
    let guid = record.guid;
    let family = *record
        .person
        .get(&61)
        .ok_or("family neighbor word absent")?;
    if family == 0
        || !world
            .person
            .as_ref()
            .unwrap()
            .families
            .contains_key(&family)
    {
        return Ok(false);
    }
    host.observe(Stage::Member, world);
    let Some(index) = member(world, family, guid)? else {
        return Ok(false);
    };
    if !world
        .person
        .as_ref()
        .unwrap()
        .family_metadata
        .contains_key(&family)
    {
        return Err("family loaded metadata absent".into());
    }
    host.observe(Stage::Live, world);
    let mut live = None;
    for &person in &world.person.as_ref().unwrap().person_order {
        if live_guid(world, person)? == guid {
            live = Some(person);
            break;
        }
    }
    if let Some(person) = live {
        world
            .objects
            .get_mut(&person)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(61, 0);
    }
    world
        .person
        .as_mut()
        .unwrap()
        .neighbors
        .get_mut(&id)
        .unwrap()
        .person
        .insert(61, 0);
    host.observe(Stage::Erase, world);
    // First matching GUID only; duplicate GUIDs and other neighbors survive.
    world
        .person
        .as_mut()
        .unwrap()
        .families
        .get_mut(&family)
        .unwrap()
        .remove(index);
    if let Some(guids) = world
        .person
        .as_mut()
        .unwrap()
        .family_metadata
        .get_mut(&family)
        .unwrap()
        .member_guids
        .as_mut()
    {
        guids.remove(index);
    }
    refresh_visitors(world, host)?;
    let metadata = world
        .person
        .as_mut()
        .unwrap()
        .family_metadata
        .get_mut(&family)
        .ok_or("family loaded metadata absent")?;
    metadata.flags &= !8;
    metadata.dirty = true;
    host.broadcast(
        world,
        Change {
            family,
            neighbor: id,
            guid,
        },
    )?;
    Ok(true)
}

pub fn add(world: &mut World, neighbor: i16, host: &mut impl Host) -> Result<bool, String> {
    let people = world.person.as_ref().ok_or("family neighborhood absent")?;
    people.validate_neighbor_slots()?;
    let slots = people
        .neighbor_slots
        .as_ref()
        .ok_or("family neighborhood slots not supplied")?;
    let id = usize::try_from(neighbor)
        .ok()
        .and_then(|value| value.checked_sub(1))
        .and_then(|index| slots.get(index).copied().flatten())
        .ok_or("family addition neighbor absent (native error 0x37)")?;
    let family = people.controlled_family;
    if !people.families.contains_key(&family) {
        return Ok(false);
    }
    let record = people
        .neighbors
        .get(&id)
        .ok_or("family neighbor slot record absent")?;
    let guid = record.guid;
    let previous = *record
        .person
        .get(&61)
        .ok_or("family neighbor word absent")?;
    if previous != 0 && !remove_resolved(world, id, host)? {
        return Ok(false);
    }
    host.observe(Stage::Member, world);
    if member(world, family, guid)?.is_some() {
        return Ok(false);
    }
    let people = world.person.as_mut().unwrap();
    let metadata = people
        .family_metadata
        .get_mut(&family)
        .ok_or("family loaded metadata absent")?;
    if let Some(guids) = &mut metadata.member_guids {
        guids.push(guid);
    }
    people.families.get_mut(&family).unwrap().push(id);
    refresh_visitors(world, host)?;
    if member(world, family, guid)?.is_none() {
        return Ok(false);
    }
    host.observe(Stage::Live, world);
    let mut live = None;
    for &person in &world.person.as_ref().unwrap().person_order {
        if live_guid(world, person)? == guid {
            live = Some(person);
            break;
        }
    }
    if let Some(person) = live {
        let fields = world
            .objects
            .get_mut(&person)
            .unwrap()
            .person
            .as_mut()
            .unwrap();
        fields.insert(61, family);
        fields.insert(67, 0);
    }
    let people = world.person.as_mut().unwrap();
    let fields = &mut people.neighbors.get_mut(&id).unwrap().person;
    fields.insert(61, family);
    fields.insert(67, 0);
    let metadata = people.family_metadata.get_mut(&family).unwrap();
    metadata.flags &= !8;
    metadata.dirty = true;
    host.broadcast(
        world,
        Change {
            family,
            neighbor: id,
            guid,
        },
    )?;
    Ok(true)
}

pub(crate) struct Live;

pub fn refresh_loaded_visitors(world: &mut World) -> Result<(), String> {
    refresh_visitors(world, &mut Live)
}

///41637f E2: resolve the full-width positive ID before inspecting OBJD kind.
/// The sparse live map contains only registered positive short IDs. Missing
/// slots and out-of-range values have the same no-op outcome as the native
/// registry bounds/null checks; do not narrow a high payload into a valid ID.
pub fn created(world: &mut World, program: &crate::Program, value: u32) -> Result<(), String> {
    let Some(object) = i16::try_from(value)
        .ok()
        .filter(|id| *id > 0)
        .and_then(|id| world.objects.get(&id))
    else {
        return Ok(());
    };
    let definition = object
        .definition
        .as_ref()
        .ok_or("created-object definition absent")?;
    let words = crate::construction::definition_words(program, definition)?;
    if words[9] == 2 {
        refresh_visitors(world, &mut Live)?;
    }
    Ok(())
}
impl Host for Live {
    fn close_panel(&mut self, _world: &mut World) -> Result<(), String> {
        // The continuous/headless presenter owns no native transient panel.
        Ok(())
    }
    fn residential(&mut self, world: &World) -> Result<bool, String> {
        let lot = *world.globals.get(&10).ok_or("family current lot absent")?;
        let people = world.person.as_ref().ok_or("family neighborhood absent")?;
        Ok(crate::zoning::is_residential(&people.lot_zoning, lot))
    }
    fn broadcast(&mut self, world: &mut World, change: Change) -> Result<(), String> {
        // 40760d(0xe8,0), after all state publication. Retain the presentation
        // notification; native Windows subscriber bodies are not implemented.
        world.person.as_mut().unwrap().family_changes.push(change);
        Ok(())
    }
}

#[cfg(test)]
#[path = "family_addition_tests.rs"]
mod addition_tests;
