//! Original appearance reload owned by Reset or a dirty skill lookup. Its
//! caller cannot advance without the saved asset job and matching continuation.
use super::*;
use sims_vm::person_factory::{ConstructionResetRequest, InitialResetRequest, registered};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    Factory,
    Deletion,
    Reset,
    Reach { request: u64 },
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub source: Source,
    pub service: character_appearance::Service,
}

fn factory_request() -> registered::Request {
    registered::Request::Reset(ConstructionResetRequest::Service(
        InitialResetRequest::ReloadAppearance,
    ))
}

impl<P: Presentation> Host<P> {
    pub(super) fn validate_reload_bindings(&self, simulation: &Simulation) -> Result<()> {
        let factories = simulation.pending_factory_character_services();
        let deletions = simulation.pending_deletion_character_services();
        let reaches = simulation.pending_reach_profiles();
        let skills = simulation.pending_skill_profiles();
        let resets = simulation.pending_reset_character_services();
        for &(id, started) in &resets {
            ensure!(
                started
                    == self
                        .reloads
                        .get(&id)
                        .is_some_and(|j| j.source == Source::Reset),
                "live Reset lost its appearance continuation"
            );
        }
        for &(id, started) in &skills {
            ensure!(
                started
                    == self
                        .reloads
                        .get(&id)
                        .is_some_and(|j| j.source == Source::Skill),
                "skill lost its profile continuation"
            );
        }
        for &(id, request, started) in &reaches {
            ensure!(
                started
                    == self
                        .reloads
                        .get(&id)
                        .is_some_and(|j| j.source == Source::Reach { request }),
                "Reach lost its profile continuation"
            );
        }
        for id in &deletions {
            let started = simulation
                .deletion
                .last()
                .unwrap()
                .person_reset
                .as_ref()
                .unwrap()
                .asset_service_started;
            ensure!(
                started
                    == self
                        .reloads
                        .get(id)
                        .is_some_and(|j| j.source == Source::Deletion),
                "deletion reset lost its appearance continuation"
            );
        }
        for (&id, job) in &self.reloads {
            ensure!(
                !job.service.clothing_used() || self.clothing.loaded(),
                "appearance reload lost its clothing cache"
            );
            job.service
                .validate_profile_cache(&simulation.vm.world.body_profiles)?;
            ensure!(
                job.service.object == id
                    && self.characters.contains_key(&id)
                    && simulation.characters.contains_key(&id),
                "appearance reload character binding changed"
            );
            ensure!(
                !self.fresh.contains_key(&id) && !self.accessories.contains_key(&id),
                "appearance reload overlaps another asset operation"
            );
            let factory = factories.contains(&(id, factory_request()));
            let deletion = deletions.contains(&id);
            let reset = resets.iter().any(|(object, _)| *object == id);
            ensure!(
                match job.source {
                    Source::Factory => factory && !deletion && !reset,
                    Source::Deletion => deletion && !factory && !reset,
                    Source::Reset => resets.contains(&(id, true)) && !factory && !deletion,
                    Source::Reach { request } =>
                        reaches.contains(&(id, request, true)) && !factory && !deletion && !reset,
                    Source::Skill =>
                        skills.contains(&(id, true)) && !factory && !deletion && !reset,
                },
                "appearance reload lost its owning continuation"
            );
        }
        Ok(())
    }

    pub(super) fn service_reload(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        id: ObjectId,
        source: Source,
    ) -> Result<()> {
        if !self.reloads.contains_key(&id) {
            ensure!(
                !self.accessories.contains_key(&id) && !self.fresh.contains_key(&id),
                "appearance reload overlaps an asset operation"
            );
            let assets = self
                .characters
                .get(&id)
                .context("appearance assets absent")?;
            let service = character_appearance::Service::begin(id, assets.appearance_owner.clone());
            match source {
                Source::Factory => {
                    ensure!(
                        simulation
                            .pending_factory_character_services()
                            .contains(&(id, factory_request())),
                        "appearance factory request changed"
                    );
                    simulation
                        .person_factories
                        .get_mut(&id)
                        .unwrap()
                        .asset_service_started = true;
                }
                Source::Deletion => simulation
                    .begin_deletion_character_service(id)
                    .map_err(anyhow::Error::msg)?,
                Source::Reset => simulation
                    .begin_reset_character_service(id)
                    .map_err(anyhow::Error::msg)?,
                Source::Reach { request } => simulation
                    .begin_reach_profile(id, request)
                    .map_err(anyhow::Error::msg)?,
                Source::Skill => simulation
                    .begin_skill_profile(id)
                    .map_err(anyhow::Error::msg)?,
            }
            self.reloads.insert(id, Job { source, service });
            // Publish ownership before asset operations, so this boundary can
            // itself be restored without re-running animator Reset.
            return Ok(());
        }
        self.validate_reload_bindings(simulation)?;
        let mut job = self.reloads[&id].clone();
        ensure!(job.source == source, "appearance reload owner changed");
        let mut assets = self.characters[&id].clone();
        let mut resident = simulation
            .characters
            .get(&id)
            .context("appearance character absent")?
            .clone();
        let mut vm = simulation.vm.clone_for_transaction();
        let mut pool = self.mesh_pool.clone();
        let mut placement = resident.character.placement.clone();
        let pending = assets.model.with_bound(&mut resident, |bound| {
            job.service.advance_bound_with_clothing(
                &self.index,
                &mut vm,
                program,
                &mut assets.avatar,
                &mut assets.geometry,
                bound,
                &mut placement,
                &mut pool,
                &mut crate::clothing_catalog::WithStartup {
                    index: &self.index,
                    cache: &mut self.clothing,
                    startup: Some(&mut self.startup),
                },
            )
        })?;
        resident.character.placement = placement;
        let finished = pending.is_none();
        if let Some(pending) = pending {
            character_appearance::notify_images(&mut vm, id, &pending)?;
            self.presentation.appearance(id, &pending)?;
            job.service.acknowledge(&mut assets.avatar.records)?;
        } else {
            assets.appearance_owner = job.service.owner.clone();
            resident.character.censor = Censor {
                binding: job.service.owner.attachment,
                mask: job.service.owner.attachment_metadata as i32,
            };
            vm.world.objects.get_mut(&id).unwrap().accessories.clear();
        }
        // Resource failures leave both the request and every geometry/VM owner
        // untouched. Retain the completed job until its caller accepts it.
        // The process clothing-name cache is independent: a successful lookup
        // may retain it even when a later binding in this transaction fails.
        self.characters.insert(id, assets);
        simulation.characters.insert(id, resident);
        simulation.vm = vm;
        self.mesh_pool = pool;
        self.reloads.insert(id, job);
        if finished {
            match source {
                Source::Factory => {
                    simulation.complete_factory_character_service(program, id, factory_request())
                }
                Source::Deletion => simulation.complete_deletion_character_service(program, id),
                Source::Reset => simulation.complete_reset_character_service(program, id),
                Source::Reach { request } => simulation.complete_reach_profile(id, request),
                Source::Skill => simulation.complete_skill_profile(id),
            }
            .map_err(anyhow::Error::msg)?;
            self.reloads.remove(&id);
        }
        Ok(())
    }
}
