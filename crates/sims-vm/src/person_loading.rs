//! Services after a person's base post-load visit (0x459bf6). Native animator
//! speed reset and final graphics rebuild surround score-curve loading; the
//! presentation host owns those calls. Placeholder and pre-v50 conversion are
//! distinct paths, not handled by these ordinary-person services.
use crate::{ObjectId, Program, World, motives::Curve};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RoomEffect {
    Occupied {
        room: i16,
        occupied: bool,
    },
    /// Native signal 0xf7, payload zero.
    SelectedRoomChanged,
}

/// Native +0xafc is constructor state, not a saved person-data word. Keep it
/// distinct from object word 29: post-load visits update it one person at a time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomMembership(pub BTreeMap<ObjectId, i16>);
impl RoomMembership {
    pub fn constructed(world: &World) -> Self {
        Self(
            world
                .objects
                .iter()
                .filter(|(_, o)| o.person.is_some())
                .map(|(&id, _)| (id, -5))
                .collect(),
        )
    }
    /// 0x45e43d / 0x46886c. Set-occupied requests are synchronous room/light
    /// services; this does not claim to implement their lighting calculations.
    pub fn update(
        &mut self,
        world: &World,
        object: ObjectId,
        rooms: &BTreeSet<i16>,
        selected: bool,
    ) -> Result<Vec<RoomEffect>, String> {
        let o = world.objects.get(&object).ok_or("room person absent")?;
        let person = o.person.as_ref().ok_or("room update requires person")?;
        let old = *self
            .0
            .get(&object)
            .ok_or("native previous person room absent")?;
        let room = *o.data.get(&29).ok_or("native current person room absent")?;
        let species = *person.get(&65).ok_or("room person species absent")?;
        if old == room {
            return Ok(vec![]);
        }
        self.0.insert(object, room);
        let exists = |room: i16| (room as u16) < 0xfffb && rooms.contains(&room);
        let mut effects = vec![];
        if species < 2 {
            if exists(old) {
                let mut occupied = false;
                let order = &world
                    .person
                    .as_ref()
                    .ok_or("room person registry absent")?
                    .person_order;
                for id in order {
                    let person = world
                        .objects
                        .get(id)
                        .and_then(|o| o.person.as_ref())
                        .ok_or("room registry person absent")?;
                    let visible = (*person.get(&68).ok_or("room person field 68 absent")?
                        | *person.get(&84).ok_or("room person field 84 absent")?)
                        & 1
                        == 0;
                    let cached = *self.0.get(id).ok_or("room registry cached room absent")?;
                    if visible && cached == old {
                        occupied = true;
                    }
                }
                if !occupied {
                    effects.push(RoomEffect::Occupied {
                        room: old,
                        occupied: false,
                    });
                }
            }
            if exists(room) {
                effects.push(RoomEffect::Occupied {
                    room,
                    occupied: true,
                });
            }
        }
        if selected {
            effects.push(RoomEffect::SelectedRoomChanged);
        }
        Ok(effects)
    }
}

/// 0x459d23 uses OBJD +0x66 when nonzero, otherwise global STR#501/503.
/// Negative signed ages select the child table too; zero selects the adult.
pub fn score_table(age: i16, custom: u16) -> (bool, u16) {
    if custom != 0 {
        (false, custom)
    } else {
        (true, if age == 0 || age >= 18 { 501 } else { 503 })
    }
}

/// Default identity curves installed by score-object constructor 0x42138d,
/// before the person's reset loads its resource-specific curves.
pub fn initial_score_curves() -> Vec<Curve> {
    [5, 6, 7, 8, 9, 3, 13, 14, 15]
        .into_iter()
        .map(|motive| Curve {
            motive,
            points: vec![[-100., -100.], [100., 100.]],
        })
        .collect()
}

pub fn load_score_curves(
    world: &World,
    p: &Program,
    object: ObjectId,
) -> Result<Vec<Curve>, String> {
    let o = world.objects.get(&object).ok_or("score person absent")?;
    let age = *o
        .person
        .as_ref()
        .and_then(|v| v.get(&58))
        .ok_or("score person age absent")?;
    let d = o.definition.as_ref().ok_or("score definition absent")?;
    let m = p.modules.get(&d.source).ok_or("score module absent")?;
    let custom = *m
        .object_definition_words
        .get(&d.object)
        .and_then(|w| w.get(51))
        .ok_or("score OBJD table absent")? as u16;
    let (global, id) = score_table(age, custom);
    let source = if global { &p.globals } else { &d.source };
    let strings = p
        .modules
        .get(source)
        .and_then(|m| m.strings.get(&id))
        .ok_or("person score StringSet not loaded")?;
    [5, 6, 7, 8, 9, 3, 13, 14, 15]
        .into_iter()
        .enumerate()
        .map(|(i, motive)| Curve::parse(motive, strings.get(i).map_or("", String::as_str), 4))
        .collect()
}
