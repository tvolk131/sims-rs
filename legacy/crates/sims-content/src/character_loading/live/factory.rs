use super::*;
use sims_vm::person_factory::{
    ConstructionResetRequest as Reset, InitialResetRequest as Service,
    PostConstructRequest as Post, registered::Request as FactoryRequest,
};

#[cfg(test)]
#[path = "factory/startup_tests.rs"]
mod startup_tests;

impl<P: Presentation> Host<P> {
    pub(super) fn validate_factory_bindings(&self, simulation: &Simulation) -> Result<()> {
        let pending = simulation.pending_factory_character_services();
        for (id, request) in &pending {
            let retained = match request {
                FactoryRequest::Post(Post::BindAnimator) => self.fresh.contains_key(id),
                _ => self
                    .reloads
                    .get(id)
                    .is_some_and(|j| j.source == reload::Source::Factory),
            };
            ensure!(
                simulation.person_factories[id].asset_service_started == retained,
                "person factory lost its asset continuation"
            );
        }
        for &id in self.fresh.keys() {
            ensure!(
                pending.contains(&(id, FactoryRequest::Post(Post::BindAnimator))),
                "fresh animator has lost its factory continuation"
            );
        }
        Ok(())
    }

    pub(super) fn service_factories(
        &mut self,
        simulation: &mut Simulation,
        program: &mut Program,
    ) -> Result<usize> {
        let mut completed = 0;
        for (id, request) in simulation.pending_factory_character_services() {
            match request {
                FactoryRequest::Post(Post::BindAnimator) => {
                    ensure!(
                        !self.characters.contains_key(&id)
                            && !simulation.characters.contains_key(&id),
                        "person factory animator already installed"
                    );
                    if !self.fresh.contains_key(&id) {
                        // Construction may load resources before it yields. Keep
                        // the old VM/CRT state if that initial allocation fails.
                        let mut vm = simulation.vm.clone();
                        let mut crt = vm.crt_rng_state.context("factory CRT state absent")?;
                        let mut skeletons = self
                            .skeletons
                            .clone()
                            .context("factory needs the process skeleton resource history")?;
                        let fresh = Fresh::begin(
                            &self.index,
                            &mut vm,
                            program,
                            id,
                            self.initializer_settings,
                            &mut crt,
                            &mut skeletons,
                            Some(&mut self.startup),
                        )?;
                        vm.crt_rng_state = Some(crt);
                        simulation.vm = vm;
                        self.skeletons = Some(skeletons);
                        self.fresh.insert(id, fresh);
                        simulation
                            .person_factories
                            .get_mut(&id)
                            .unwrap()
                            .asset_service_started = true;
                    }
                    let position = *simulation
                        .vm
                        .world
                        .positions
                        .get(&id)
                        .context("fresh person physical position absent")?;
                    ensure!(
                        [position.x, position.y] == [-16, -16] && position.level == 1,
                        "new person placed before animator construction"
                    );
                    let lot = simulation
                        .architecture
                        .as_ref()
                        .and_then(|a| a.original.as_ref())
                        .context("fresh animator original terrain absent")?;
                    let floor_vector = simulation
                        .native_character_settings
                        .context("fresh animator settings absent")?
                        .floor_vector;
                    let view = transform::View {
                        rotation: simulation.vm.world.camera_rotation,
                        floor_vector,
                    };
                    let finished = self.fresh.get_mut(&id).unwrap().advance(
                        &self.index,
                        &mut simulation.vm,
                        program,
                        &mut self.presentation,
                        &mut self.mesh_pool,
                        &mut self.clothing,
                        fresh::Environment {
                            position: [position.x, position.y],
                            level: position.level,
                            view,
                            terrain: transform::Terrain {
                                size: lot.size,
                                corners: &lot.heights,
                            },
                        },
                    )?;
                    if finished {
                        // Retain the complete fresh owner until handoff succeeds.
                        let loaded = Fresh::restore(
                            &self.index,
                            self.fresh[&id].snapshot(),
                            Some(&mut self.startup),
                        )?
                        .into_loaded(
                            simulation.vm.tick,
                            &mut self.presentation,
                            &simulation.vm.world,
                        )?;
                        loaded
                            .resident
                            .character
                            .validate(program)
                            .map_err(anyhow::Error::msg)?;
                        simulation.characters.insert(id, loaded.resident);
                        self.characters.insert(
                            id,
                            Assets {
                                model: loaded.model,
                                geometry: loaded.geometry,
                                avatar: loaded.avatar,
                                appearance_owner: loaded.appearance_owner,
                            },
                        );
                        simulation
                            .complete_factory_character_service(program, id, request)
                            .map_err(anyhow::Error::msg)?;
                        self.fresh.remove(&id);
                    }
                }
                FactoryRequest::Reset(Reset::Service(Service::ReloadAppearance)) => {
                    self.service_reload(simulation, program, id, reload::Source::Factory)?;
                }
                other => anyhow::bail!("unsupported factory asset service {other:?}"),
            }
            completed += 1;
        }
        Ok(completed)
    }
}
