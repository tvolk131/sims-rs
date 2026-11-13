//! A read-only presentation of the loaded lot and explicit saved resident assets.
//! There is deliberately no tick/input method: native live activation, dynamic
//! appearance rebuilding, and saved animator continuations are separate work.
use crate::{Assets, Camera, Scene, room_scene};
use anyhow::{Context, Result, ensure};
use sims_content::{Index, house_loading::FrozenLot, saves::House};
use sims_sim::{Simulation, lot_runtime};
use sims_vm::Program;

pub struct FrozenHouse {
    program: Program,
    state: Simulation,
    assets: Assets,
    pub missing_graphics: Vec<i16>,
}
impl FrozenHouse {
    pub fn load(index: &Index, house: &House, frozen: &FrozenLot) -> Result<Self> {
        let world = &frozen.execution.vm.world;
        let state = Simulation::from_loaded_lot(
            frozen.execution.vm.clone(),
            frozen.architecture.clone(),
            lot_runtime::State {
                placement: frozen.placement.clone(),
                membership: frozen.membership.clone(),
                indicators: frozen.indicators.clone(),
                observers: frozen.observers.clone(),
                diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
                    error: 0,
                    blocker: None,
                },
                route: None,
                shoo: None,
                movement: None,
                snap: None,
                find_location: None,
                removal: None,
                snap_offsets: Default::default(),
                completion: None,
                route_paths: Default::default(),
                selected_person: None,
                room_effects: vec![],
                routing_policy: Default::default(),
            },
        )
        .map_err(anyhow::Error::msg)?;
        let mut program = frozen.program.clone();
        let objects: std::collections::BTreeSet<_> = state
            .bodies
            .keys()
            .filter_map(|id| {
                let o = &world.objects[id];
                (o.person.is_none() && o.data.get(&34).is_none_or(|v| *v == 0))
                    .then(|| o.definition.as_ref().map(|d| (d.source.as_str(), d.object)))
                    .flatten()
            })
            .collect();
        let accessories = world
            .objects
            .values()
            .flat_map(|o| o.accessories.iter().cloned())
            .collect();
        let definitions = objects.iter().map(|(s, id)| (s.to_string(), *id)).collect();
        let mut assets = Assets::load_objects_profile(
            index,
            &objects.into_iter().collect::<Vec<_>>(),
            Some(&accessories),
        )?;
        assets.bind_original_floors(index, house)?;
        assets.bind_original_walls(index, house, &program, &definitions)?;
        for resident in &house
            .household
            .as_ref()
            .context("household absent")?
            .residents
        {
            let id = resident.object as i16;
            let object = world
                .objects
                .get(&id)
                .context("resident is absent from loaded world")?;
            let definition = object
                .definition
                .as_ref()
                .context("resident definition absent")?;
            ensure!(
                definition.guid == resident.guid && definition.source == resident.character_source,
                "resident #{id} identity disagrees with the loaded household"
            );
            let skeleton_name = resident
                .body_strings
                .first()
                .context("resident skeleton profile absent")?;
            let saved = house.objects.instances[&resident.object]
                .person
                .as_ref()
                .context("saved person absent")?;
            let avatar = sims_content::saved_avatar::load(index, skeleton_name, saved)?;
            ensure!(
                state.vm.world.objects[&id].person.as_ref().unwrap()[&8] == saved.data[8],
                "resident #{id} outfit changed during load; saved presentation is stale"
            );
            // Use the saved base clip as a stationary preview, not its encoded
            // timing/continuation. A non-standing base requires actual animator import.
            let idle = saved.animations[2].split(';').next().unwrap();
            ensure!(
                matches!(idle, "a2o-standing-loop" | "c2o-standing-loop"),
                "saved resident #{id} needs a non-standing preview pose: {idle}"
            );
            if !program.animations.contains_key(idle) {
                sims_content::assets::load_named_animations(
                    index,
                    &mut program,
                    &[idle.to_owned()].into(),
                )?;
                ensure!(
                    program.animations.contains_key(idle),
                    "saved base clip {idle} not found"
                );
            }
            assets.bind_resident(id, avatar, saved.data[8], idle.to_owned())?;
        }
        ensure!(
            state
                .vm
                .world
                .objects
                .iter()
                .filter(|(_, o)| o.person.is_some())
                .all(|(id, _)| assets.residents.contains_key(id)),
            "a loaded person has no original saved appearance"
        );
        let missing_graphics = state
            .bodies
            .keys()
            .filter_map(|&id| {
                let o = &world.objects[&id];
                if o.person.is_some() || o.data.get(&34).is_some_and(|v| *v != 0) {
                    return None;
                }
                let d = o.definition.as_ref()?;
                let &(base, count) = assets.object_graphics.get(&(d.source.clone(), d.object))?;
                (base != 0 && crate::graphic_group(base, count, o.data[&0]).is_none()).then_some(id)
            })
            .collect();
        Ok(Self {
            missing_graphics,
            program,
            state,
            assets,
        })
    }
    pub fn camera(&self, width: u32, height: u32, rotation: u8, zoom: f32, level: i32) -> Camera {
        Camera::room(&self.state, width, height, rotation, zoom).floor(level)
    }
    pub fn scene(&self, camera: Camera) -> Result<Scene> {
        room_scene(&self.assets, &self.program, &self.state, camera, 0.)
    }
}
