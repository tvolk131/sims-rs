//! Asset-side services for the live simulation. The simulation owns residents;
//! this host retains only their original model/binding/geometry resources.
use super::*;
use crate::saved_avatar::BindingState;
use serde::{Deserialize, Serialize};
use sims_sim::Simulation;
mod accessory;
mod appearance;
pub mod dependent;
mod factory;
pub mod graphic;
mod notification;
mod portrait;
mod reload;
pub mod retirement;

#[derive(Clone)]
pub struct Assets {
    pub model: character_model::State,
    pub geometry: CharacterGeometry,
    pub avatar: BoundAvatar,
    pub appearance_owner: Owner,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedAssets {
    pub skeleton: String,
    pub model: character_model::State,
    pub geometry: CharacterGeometry,
    pub binding: BindingState,
    pub appearance_owner: Owner,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub floors: BTreeMap<u16, floors::Binding>,
    pub neighborhood_source: String,
    pub initializer_settings: Settings,
    pub fresh: BTreeMap<ObjectId, fresh::Saved>,
    pub reloads: BTreeMap<ObjectId, reload::Job>,
    #[serde(default)]
    pub dependent_jobs: BTreeMap<ObjectId, dependent::Job>,
    #[serde(default)]
    pub accessories: BTreeMap<ObjectId, accessory::Job>,
    #[serde(default)]
    pub retirements: BTreeMap<ObjectId, retirement::Job>,
    #[serde(default)]
    pub notification_jobs: BTreeMap<ObjectId, notification::Job>,
    #[serde(default)]
    pub notifications: Option<crate::notifications::Saved>,
    pub characters: BTreeMap<ObjectId, SavedAssets>,
    pub mesh_pool: MeshPool,
    #[serde(default)]
    pub clothing: crate::clothing_catalog::Cache,
    /// None is an older snapshot with unknown resource history, not a cold cache.
    #[serde(default)]
    pub skeletons: Option<crate::skeleton_resources::Cache>,
    #[serde(default)]
    pub ordinary: crate::graphic_loading::Saved,
    /// None preserves unknown historical Global resource history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy_resources: Option<crate::autonomy_resources::Provider>,
}

pub struct Host<P> {
    floors: BTreeMap<u16, floors::Binding>,
    index: std::sync::Arc<Index>,
    startup: crate::startup_memo::Memo,
    animation_catalogs: crate::assets::AnimationCatalogs,
    neighborhood_source: String,
    initializer_settings: Settings,
    fresh: BTreeMap<ObjectId, Fresh>,
    reloads: BTreeMap<ObjectId, reload::Job>,
    dependent_jobs: BTreeMap<ObjectId, dependent::Job>,
    accessories: BTreeMap<ObjectId, accessory::Job>,
    retirements: BTreeMap<ObjectId, retirement::Job>,
    notification_jobs: BTreeMap<ObjectId, notification::Job>,
    pub notifications: Option<crate::notifications::Software>,
    pub presentation: P,
    pub mesh_pool: MeshPool,
    pub characters: BTreeMap<ObjectId, Assets>,
    clothing: crate::clothing_catalog::Cache,
    pub skeletons: Option<crate::skeleton_resources::Cache>,
    pub ordinary: crate::graphic_loading::Registry,
    pub autonomy_resources: Option<crate::autonomy_resources::Provider>,
}

impl<'a, P: Presentation> Reader<'a, P> {
    /// Move each loaded animator into its single live owner. This does not
    /// finalize selection, attach indicators, or certify playable activation.
    pub fn into_live(
        self,
        simulation: &mut Simulation,
        settings: sims_sim::character_update::Settings,
    ) -> Result<Host<P>> {
        ensure!(self.is_complete(), "character reads remain pending");
        ensure!(
            simulation.characters.is_empty(),
            "native characters already installed"
        );
        ensure!(
            simulation.native_lot.is_some(),
            "loaded native lot required"
        );
        ensure!(
            simulation
                .household
                .as_ref()
                .context("live household context absent")?
                .house_statistics
                .is_none(),
            "live House statistics already installed"
        );
        ensure!(
            settings.floor_vector == self.view.floor_vector,
            "character floor view changed during handoff"
        );
        for (&id, loaded) in &self.characters {
            ensure!(
                simulation
                    .vm
                    .world
                    .objects
                    .get(&id)
                    .is_some_and(|o| o.person.is_some()),
                "loaded character {id} missing from live world"
            );
            ensure!(
                loaded.resident.evaluated_tick == simulation.vm.tick,
                "loaded character clock differs from live world"
            );
            ensure!(
                !loaded.resident.character.busy(),
                "loaded character service remains pending"
            );
        }
        let people = simulation
            .vm
            .world
            .objects
            .iter()
            .filter(|(_, o)| o.person.is_some())
            .count();
        ensure!(
            people == self.characters.len(),
            "live person lacks an initialized character"
        );
        let mut characters = BTreeMap::new();
        for (id, loaded) in self.characters {
            simulation.characters.insert(id, loaded.resident);
            characters.insert(
                id,
                Assets {
                    model: loaded.model,
                    geometry: loaded.geometry,
                    avatar: loaded.avatar,
                    appearance_owner: loaded.appearance_owner,
                },
            );
        }
        simulation.native_character_settings = Some(settings);
        simulation.vm.native_accessory_host = true;
        let household = simulation.household.as_mut().unwrap();
        household.house_statistics = Some(sims_sim::house_statistics::State::from_saved_pairs(
            self.house.house.pairs,
        ));
        simulation.vm.crt_rng_state = Some(self.crt_state);
        if let Some(error) = self.route_diagnostic {
            simulation.native_lot.as_mut().unwrap().diagnostics.error = error;
        }
        let index = std::sync::Arc::new(self.index.clone());
        let startup = crate::startup_memo::Memo::new(index.clone());
        Ok(Host {
            floors: floors::load(self.index, self.house)?,
            index,
            startup,
            animation_catalogs: crate::assets::AnimationCatalogs::default(),
            neighborhood_source: self.house.neighborhood_source.clone(),
            initializer_settings: self.settings,
            fresh: BTreeMap::new(),
            reloads: BTreeMap::new(),
            dependent_jobs: BTreeMap::new(),
            accessories: BTreeMap::new(),
            retirements: BTreeMap::new(),
            notification_jobs: BTreeMap::new(),
            notifications: None,
            presentation: self.presentation,
            mesh_pool: self.mesh_pool,
            characters,
            clothing: self.clothing,
            skeletons: Some(self.skeletons),
            ordinary: self.ordinary,
            autonomy_resources: self.autonomy_resources,
        })
    }
}

impl<P: Presentation> Host<P> {
    /// Fork trusted live state without re-reading the installation. All mutable
    /// owners/bindings are copied. Only the read-only resource index is shared;
    /// the derived font raster cache is rebuilt from its retained font bytes.
    pub fn fork_for_exploration(&self) -> Result<Self>
    where
        P: Clone,
    {
        Ok(Self {
            floors: self.floors.clone(),
            index: self.index.clone(),
            startup: crate::startup_memo::Memo::new(self.index.clone()),
            animation_catalogs: self.animation_catalogs.clone(),
            neighborhood_source: self.neighborhood_source.clone(),
            initializer_settings: self.initializer_settings,
            fresh: self.fresh.clone(),
            reloads: self.reloads.clone(),
            dependent_jobs: self.dependent_jobs.clone(),
            accessories: self.accessories.clone(),
            retirements: self.retirements.clone(),
            notification_jobs: self.notification_jobs.clone(),
            notifications: self
                .notifications
                .as_ref()
                .map(|n| crate::notifications::Software::restore(n.saved.clone()))
                .transpose()?,
            presentation: self.presentation.clone(),
            mesh_pool: self.mesh_pool.clone(),
            characters: self.characters.clone(),
            clothing: self.clothing.clone(),
            skeletons: self.skeletons.clone(),
            ordinary: self.ordinary.clone(),
            autonomy_resources: self.autonomy_resources.clone(),
        })
    }

    pub fn pending_accessories(&self) -> &BTreeMap<ObjectId, accessory::Job> {
        &self.accessories
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            floors: self.floors.clone(),
            neighborhood_source: self.neighborhood_source.clone(),
            initializer_settings: self.initializer_settings,
            fresh: self
                .fresh
                .iter()
                .map(|(&id, f)| (id, f.snapshot()))
                .collect(),
            reloads: self.reloads.clone(),
            dependent_jobs: self.dependent_jobs.clone(),
            accessories: self.accessories.clone(),
            retirements: self.retirements.clone(),
            notification_jobs: self.notification_jobs.clone(),
            notifications: self.notifications.as_ref().map(|n| n.saved.clone()),
            mesh_pool: self.mesh_pool.clone(),
            clothing: self.clothing.clone(),
            skeletons: self.skeletons.clone(),
            ordinary: self.ordinary.snapshot(),
            autonomy_resources: self.autonomy_resources.clone(),
            characters: self
                .characters
                .iter()
                .map(|(&id, c)| {
                    (
                        id,
                        SavedAssets {
                            skeleton: c.avatar.avatar.skeleton.name.clone(),
                            model: c.model.clone(),
                            geometry: c.geometry.clone(),
                            binding: c.avatar.snapshot(),
                            appearance_owner: c.appearance_owner.clone(),
                        },
                    )
                })
                .collect(),
        }
    }
    pub fn restore(index: &Index, presentation: P, snapshot: Snapshot) -> Result<Self> {
        Self::restore_with_animation_catalogs(
            index,
            presentation,
            snapshot,
            crate::assets::AnimationCatalogs::default(),
        )
    }

    pub fn restore_with_animation_catalogs(
        index: &Index,
        presentation: P,
        snapshot: Snapshot,
        animation_catalogs: crate::assets::AnimationCatalogs,
    ) -> Result<Self> {
        snapshot.clothing.validate(index)?;
        if let Some(provider) = &snapshot.autonomy_resources {
            provider.validate(index)?;
        }
        if let Some(cache) = &snapshot.skeletons {
            cache.validate(index)?;
        }
        floors::restore(index, &snapshot.floors)?;
        index.find(&snapshot.neighborhood_source)?;
        ensure!(
            snapshot.notification_jobs.len() <= 1,
            "overlapping notification font owners"
        );
        let notifications = snapshot
            .notifications
            .map(crate::notifications::Software::restore)
            .transpose()?;
        for (&id, job) in &snapshot.notification_jobs {
            job.validate(
                id,
                notifications
                    .as_ref()
                    .context("notification presenter absent")?,
            )?;
            ensure!(
                snapshot.characters.contains_key(&id),
                "notification character absent"
            );
        }
        let index_owner = std::sync::Arc::new(index.clone());
        let mut startup = crate::startup_memo::Memo::new(index_owner.clone());
        let mut fresh = BTreeMap::new();
        for (id, saved) in snapshot.fresh {
            let state = Fresh::restore(&index_owner, saved, Some(&mut startup))?;
            ensure!(
                !state.clothing_used() || snapshot.clothing.loaded(),
                "fresh appearance lost its clothing cache"
            );
            ensure!(
                state.prepared.object == id && !snapshot.characters.contains_key(&id),
                "invalid fresh animator registry binding"
            );
            fresh.insert(id, state);
        }
        for (&id, reload) in &snapshot.reloads {
            ensure!(
                !reload.service.clothing_used() || snapshot.clothing.loaded(),
                "appearance reload lost its clothing cache"
            );
            ensure!(
                reload.service.object == id && snapshot.characters.contains_key(&id),
                "invalid live appearance registry binding"
            );
        }
        for (&id, job) in &snapshot.accessories {
            ensure!(
                snapshot.characters.contains_key(&id),
                "saved accessory character absent"
            );
            job.validate(id)?;
            job.execution
                .validate(&snapshot.characters[&id].binding.records)
                .map_err(anyhow::Error::msg)?;
        }
        for (&id, job) in &snapshot.retirements {
            let assets = snapshot
                .characters
                .get(&id)
                .context("retired character assets absent")?;
            job.validate(&assets.binding.records, &assets.appearance_owner)?;
            ensure!(
                !snapshot.reloads.contains_key(&id) && !snapshot.accessories.contains_key(&id),
                "retired assets overlap a live service"
            );
        }
        let mut characters = BTreeMap::new();
        for (id, c) in snapshot.characters {
            ensure!(id > 0, "invalid saved character asset owner");
            characters.insert(
                id,
                Assets {
                    model: c.model,
                    geometry: c.geometry,
                    avatar: BoundAvatar::restore_with_startup(
                        &index_owner,
                        &c.skeleton,
                        id as u64,
                        &c.binding,
                        Some(&mut startup),
                    )?,
                    appearance_owner: c.appearance_owner,
                },
            );
        }
        Ok(Self {
            floors: snapshot.floors,
            index: index_owner,
            startup,
            animation_catalogs,
            neighborhood_source: snapshot.neighborhood_source,
            initializer_settings: snapshot.initializer_settings,
            fresh,
            reloads: snapshot.reloads,
            dependent_jobs: snapshot.dependent_jobs,
            accessories: snapshot.accessories,
            retirements: snapshot.retirements,
            notification_jobs: snapshot.notification_jobs,
            notifications,
            presentation,
            mesh_pool: snapshot.mesh_pool,
            characters,
            clothing: snapshot.clothing,
            skeletons: snapshot.skeletons,
            ordinary: crate::graphic_loading::Registry::restore(index, snapshot.ordinary)?,
            autonomy_resources: snapshot.autonomy_resources,
        })
    }

    /// Refresh live view/audio inputs before entering the scheduler. No gameplay
    /// state, clock or script executes in this service.
    pub fn update_contexts(&mut self, simulation: &mut Simulation) -> Result<()> {
        self.validate_dependent(simulation)?;
        ensure!(
            self.ordinary.retained.objects().next().is_none(),
            "hot ordinary graphics require retained frame/placement services before live presentation"
        );
        self.ordinary
            .validate_with_retired(&simulation.vm, &simulation.retired_graphics)?;
        if let Some((id, started)) = simulation.pending_removal_graphic() {
            let graphic = self
                .ordinary
                .graphics
                .get(&id)
                .context("removal graphic absent")?;
            ensure!(
                started == graphic.update_pending(),
                "removal graphic lost its saved service"
            );
        }
        self.validate_selection_factory_graphics(simulation)?;
        self.validate_factory_bindings(simulation)?;
        self.validate_reload_bindings(simulation)?;
        self.validate_accessory_bindings(simulation)?;
        self.validate_retirement_bindings(simulation)?;
        self.validate_notification_bindings(simulation)?;
        ensure!(
            self.characters.len()
                == simulation.characters.len() + simulation.retired_characters.len(),
            "character asset registry mismatch"
        );
        for (&id, resident) in &mut simulation.characters {
            let assets = self
                .characters
                .get(&id)
                .context("native character assets absent")?;
            ensure!(
                resident.source.as_ref() == Some(&assets.avatar.avatar.skeleton),
                "native character rig binding changed"
            );
            if let Some(job) = self.reloads.get(&id) {
                crate::character_censor::validate_assets(
                    &job.service.owner,
                    &assets.avatar,
                    &assets.geometry,
                )?;
            } else {
                crate::character_censor::validate(
                    resident,
                    &assets.appearance_owner,
                    &assets.avatar,
                    &assets.geometry,
                )?;
            }
            let context = self
                .presentation
                .context_in_world(id, resident, &simulation.vm.world)?;
            ensure!(
                context.actor == id,
                "live character context belongs to another person"
            );
            resident.context = context;
        }
        Ok(())
    }

    /// Complete available asset/view/geometry requests. Unsupported requests
    /// remain pending for their actual owner; the caller must not advance time.
    pub fn service(&mut self, simulation: &mut Simulation, program: &mut Program) -> Result<usize> {
        let mut completed = 0;
        for (id, request) in simulation.pending_factory_autonomy_services() {
            let provider=self.autonomy_resources.as_mut().with_context(||format!(
                "person factory {id} requires source-bound autonomy resource service {request:?}"))?;
            let reply = provider.service(&request, simulation.factory_autonomy_table(id))?;
            simulation
                .reply_factory_autonomy_constants(id, reply)
                .map_err(anyhow::Error::msg)?;
            completed += 1;
        }
        self.update_contexts(simulation)?;
        for id in simulation
            .retired_graphics
            .keys()
            .copied()
            .collect::<Vec<_>>()
        {
            self.service_graphic_retirement(simulation, id)?;
            completed += 1;
        }
        for id in simulation
            .retired_characters
            .keys()
            .copied()
            .collect::<Vec<_>>()
        {
            self.service_retirement(simulation, program, id)?;
            completed += 1;
        }
        completed += self.service_factories(simulation, program)?;
        for id in simulation.pending_deletion_character_services() {
            self.service_reload(simulation, program, id, reload::Source::Deletion)?;
            completed += 1;
        }
        for (id, _) in simulation.pending_reset_character_services() {
            self.service_reload(simulation, program, id, reload::Source::Reset)?;
            completed += 1;
        }
        for (id, request, _) in simulation.pending_reach_profiles() {
            self.service_reload(simulation, program, id, reload::Source::Reach { request })?;
            completed += 1;
        }
        for (id, _) in simulation.pending_skill_profiles() {
            self.service_reload(simulation, program, id, reload::Source::Skill)?;
            completed += 1;
        }
        for (thread, command) in simulation
            .vm
            .native_accessory_commands(program)
            .map_err(anyhow::Error::msg)?
        {
            let id = command.caller;
            self.service_accessory(
                simulation,
                id,
                accessory::Source::Script { thread, command },
            )?;
            if self.accessories[&id].execution.is_complete() {
                let accessory::Source::Script { thread, command } = &self.accessories[&id].source
                else {
                    unreachable!()
                };
                simulation
                    .vm
                    .finish_native_accessory(program, *thread, command)
                    .map_err(anyhow::Error::msg)?;
                self.accessories.remove(&id);
            }
            completed += 1;
        }
        let mut guids: std::collections::BTreeSet<_> = simulation
            .vm
            .threads
            .values()
            .filter_map(|thread| {
                if let sims_vm::Status::AwaitingCreation(request) = &thread.status {
                    // Invalid or not-yet-resolvable operands stay with the VM's
                    // original request/error path, not the resource resolver.
                    simulation.vm.creation_guid(request).ok().flatten()
                } else {
                    None
                }
            })
            .filter(|guid| {
                !program.modules.values().any(|module| {
                    module.object_definition_words.values().any(|words| {
                        words.len() > 15
                            && (u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16))
                                == *guid
                    })
                })
            })
            .collect();
        if simulation.pending_burn_resource() {
            guids.insert(sims_sim::burn::FIRE_GUID);
        }
        if !guids.is_empty() {
            crate::saves::link_neighborhood_creation_resources(
                &self.index,
                program,
                &self.neighborhood_source,
                &guids,
            )?;
            simulation
                .architecture
                .as_mut()
                .context("live creation architecture absent")?
                .bind_footprints(program)
                .map_err(anyhow::Error::msg)?;
            completed += guids.len();
        }
        if simulation
            .pending_snap_shoo_resource(program)
            .map_err(anyhow::Error::msg)?
        {
            let present = crate::assets::load_object_guid_if_present(&self.index, program, 0x7c4)?;
            if present {
                simulation
                    .architecture
                    .as_mut()
                    .context("Snap shoo architecture absent")?
                    .bind_footprints(program)
                    .map_err(anyhow::Error::msg)?;
            } else {
                simulation
                    .complete_missing_snap_shoo_resource(program)
                    .map_err(anyhow::Error::msg)?;
            }
            completed += 1;
        }
        let names = simulation
            .pending_animation_assets(program)
            .map_err(anyhow::Error::msg)?;
        if !names.is_empty() {
            let report = self
                .animation_catalogs
                .load_native(&self.index, program, &names)?;
            ensure!(
                report.unresolved.is_empty(),
                "unresolved live animation assets: {:?}",
                report.unresolved
            );
            completed += names.len();
        }
        for (id, request) in simulation.pending_character_services() {
            completed += usize::from(self.service_character_request(simulation, id, request)?);
        }
        completed += self.service_notifications(simulation)?;
        Ok(completed)
    }
    /// Camera refresh must service its captured person only. General service()
    /// also probes waiting SimAntics animation/creation commands; those cannot
    /// overtake a suspended native graphic call.
    pub fn service_view_person(
        &mut self,
        simulation: &mut Simulation,
        id: ObjectId,
    ) -> Result<usize> {
        self.update_contexts(simulation)?;
        let Some((_, request)) = simulation
            .pending_character_services()
            .into_iter()
            .find(|(owner, _)| *owner == id)
        else {
            return Ok(0);
        };
        ensure!(
            self.service_character_request(simulation, id, request.clone())?,
            "view person {id} requires its character service owner: {request:?}"
        );
        Ok(1)
    }
    fn service_character_request(
        &mut self,
        simulation: &mut Simulation,
        id: ObjectId,
        request: Request,
    ) -> Result<bool> {
        if let Request::Property(
            property @ (properties::Request::Dress { .. } | properties::Request::Undress { .. }),
        ) = &request
        {
            self.service_accessory(
                simulation,
                id,
                accessory::Source::Property(property.clone()),
            )?;
            if self.accessories[&id].execution.is_complete() {
                simulation
                    .complete_character_service(id, &request, Response::Done)
                    .map_err(anyhow::Error::msg)?;
                self.accessories.remove(&id);
            }
            return Ok(true);
        }
        let assets = self
            .characters
            .get_mut(&id)
            .context("live character assets absent")?;
        let resident = simulation
            .characters
            .get_mut(&id)
            .context("live character absent")?;
        if crate::character_censor::service(
            &self.index,
            resident,
            &mut assets.model,
            &mut assets.geometry,
            &mut assets.avatar,
            &mut assets.appearance_owner,
            &mut self.mesh_pool,
        )? {
            return Ok(true);
        }
        if assets
            .geometry
            .service_resident(
                &mut assets.model,
                resident,
                &assets.avatar,
                &mut self.mesh_pool,
            )?
            .is_some()
        {
            return Ok(true);
        }
        let response = match &request {
            Request::Viewport => self.presentation.viewport().map(Response::Viewport)?,
            Request::Visibility { viewport } => self
                .presentation
                .visibility_for_model(id, resident, &assets.model, *viewport)
                .map(Response::Visible)?,
            Request::Property(property) => {
                if matches!(property, properties::Request::FootstepSurface { .. }) {
                    Response::PropertySurface(
                        simulation
                            .original_footstep_surface(id, &floors::categories(&self.floors))
                            .map_err(anyhow::Error::msg)?,
                    )
                } else {
                    self.presentation
                        .property(id, property)
                        .with_context(|| format!("live character {id} property {property:?}"))?
                }
            }
            _ => return Ok(false),
        };
        simulation
            .complete_character_service(id, &request, response)
            .map_err(anyhow::Error::msg)?;
        Ok(true)
    }
}
