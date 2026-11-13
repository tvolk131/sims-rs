//! Native person construction and animator reads inside original lot loading.
//! Presentation/query inputs are explicit host services. Unsupported services
//! abort loading; no character reconstructed from a legacy playback is installed.
use crate::{
    Index, character_appearance,
    character_geometry::{CharacterGeometry, MeshPool},
    character_initialization::Prepared,
    character_model,
    save_storage::PersonReader,
    saved_avatar::BoundAvatar,
    saved_layers,
    saves::House,
};
use anyhow::{Context, Result, ensure};
use sims_sim::{
    character::Resident,
    pose::{
        animator::Settings,
        appearance_read,
        appearance_reload::Owner,
        initialize, properties,
        refresh::{Censor, Request, Response},
        transform,
    },
};
use sims_vm::{Object, ObjectId, Program, Vm, World};
use std::collections::BTreeMap;

mod fresh;
mod startup;
#[cfg(test)]
mod startup_tests;
pub use startup::RetainedGraphics;
pub mod live;
use fresh::Fresh;

/// The loader supplies actual view/audio inputs without inventing them from
/// saved person words, which are not installed until the animator read ends.
pub trait Presentation {
    fn retirement_references(&mut self, object: ObjectId) -> Result<RetirementReferences>;
    fn retire(&mut self, object: ObjectId, event: RetirementEvent) -> Result<()>;
    fn context(
        &mut self,
        object: ObjectId,
        placement: &transform::Placement,
    ) -> Result<properties::Context>;
    /// The animator exists before its Resident handoff. Native callbacks still
    /// read the live World and the actual +1b0 appearance selector at this cut.
    fn context_at(
        &mut self,
        object: ObjectId,
        placement: &transform::Placement,
        _appearance_mode: i32,
        _world: &World,
    ) -> Result<properties::Context> {
        self.context(object, placement)
    }
    /// Native property callbacks read the current object/world, independently
    /// of render selection and visibility. Existing authored presenters keep
    /// their explicitly supplied context implementation.
    fn context_in_world(
        &mut self,
        object: ObjectId,
        resident: &sims_sim::character::Resident,
        _world: &sims_vm::World,
    ) -> Result<properties::Context> {
        self.context(object, &resident.character.placement)
    }
    fn viewport(&mut self) -> Result<[i32; 4]>;
    fn visibility(&mut self, object: ObjectId, viewport: [i32; 4]) -> Result<bool>;
    fn visibility_for_initialization(
        &mut self,
        object: ObjectId,
        _initialized: &initialize::Initialization,
        viewport: [i32; 4],
    ) -> Result<bool> {
        self.visibility(object, viewport)
    }
    /// Native visibility projects the live model's bounds using the actual
    /// camera. The authored presentation hook remains available separately.
    fn visibility_for_model(
        &mut self,
        object: ObjectId,
        _resident: &sims_sim::character::Resident,
        _model: &crate::character_model::State,
        viewport: [i32; 4],
    ) -> Result<bool> {
        self.visibility(object, viewport)
    }
    fn appearance(
        &mut self,
        object: ObjectId,
        request: &character_appearance::Request,
    ) -> Result<()>;
    fn saved_texture(&mut self, object: ObjectId) -> Result<()>;
    fn invalidate(&mut self, object: ObjectId) -> Result<()>;
    fn property(&mut self, object: ObjectId, request: &properties::Request) -> Result<Response>;
}

/// Explicit presentation inputs for native animator destruction. A headless
/// host has no selected animator or global preview/focus owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetirementReferences {
    pub selected: bool,
    pub focus: bool,
    pub base_focus: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetirementEvent {
    ClearSelected,
    ClearFocus,
    ClearBaseFocus,
    ModelRemoved,
}

pub struct Loaded {
    pub resident: Resident,
    pub model: character_model::State,
    pub geometry: CharacterGeometry,
    pub avatar: BoundAvatar,
    pub appearance_owner: Owner,
}

pub mod floors;

pub struct Reader<'a, P> {
    index: &'a Index,
    house: &'a House,
    settings: Settings,
    view: transform::View,
    terrain: Vec<[u8; 4]>,
    pub presentation: P,
    /// Process CRT randomness is separate from both saved simulator streams.
    pub crt_state: u32,
    pub mesh_pool: MeshPool,
    pub route_diagnostic: Option<i32>,
    pub characters: BTreeMap<ObjectId, Loaded>,
    ///Ordinary graphics created by synchronous factories during post-load.
    pub ordinary: crate::graphic_loading::Registry,
    fresh: BTreeMap<ObjectId, Fresh>,
    pub autonomy_resources: Option<crate::autonomy_resources::Provider>,
    initial_process: Option<sims_vm::autonomy_constants::Owner>,
    /// Synchronous saved-person child; a failed import is not a managed loader continuation.
    constants: BTreeMap<ObjectId, sims_vm::autonomy_constants::Call>,
    clothing: crate::clothing_catalog::Cache,
    pub skeletons: crate::skeleton_resources::Cache,
}
impl<'a, P: Presentation> Reader<'a, P> {
    pub fn new(
        index: &'a Index,
        house: &'a House,
        settings: Settings,
        floor_vector: [f32; 3],
        crt_state: u32,
        presentation: P,
    ) -> Result<Self> {
        ensure!(
            house.objects.version > 60,
            "native character reader requires the version >60 animator tail"
        );
        Ok(Self {
            index,
            house,
            settings,
            view: transform::View {
                rotation: house.view.camera_direction.try_into()?,
                floor_vector,
            },
            terrain: house.corner_heights()?,
            presentation,
            crt_state,
            mesh_pool: MeshPool::default(),
            route_diagnostic: None,
            characters: BTreeMap::new(),
            ordinary: Default::default(),
            fresh: BTreeMap::new(),
            autonomy_resources: None,
            initial_process: None,
            constants: BTreeMap::new(),
            clothing: Default::default(),
            skeletons: Default::default(),
        })
    }
    pub fn install_process_input(
        &mut self,
        input: crate::autonomy_resources::FreshInput,
    ) -> Result<()> {
        ensure!(
            self.autonomy_resources.is_none()
                && self.initial_process.is_none()
                && self.fresh.is_empty()
                && self.characters.is_empty(),
            "process input installed after person construction"
        );
        input.owner.validate().map_err(anyhow::Error::msg)?;
        input.provider.validate(self.index)?;
        ensure!(
            input.owner.configuration.source.as_ref() == Some(&input.provider.binding()),
            "process/provider source differs"
        );
        self.initial_process = Some(input.owner);
        self.autonomy_resources = Some(input.provider);
        Ok(())
    }
    /// Reuse an explicitly supplied existing resource owner without importing
    /// process defaults. Unknown historical World owners remain unknown.
    pub fn install_retained_process_resources(
        &mut self,
        provider: crate::autonomy_resources::Provider,
    ) -> Result<()> {
        ensure!(
            self.autonomy_resources.is_none()
                && self.initial_process.is_none()
                && self.fresh.is_empty()
                && self.characters.is_empty(),
            "retained provider overlaps person/resource ownership"
        );
        provider.validate(self.index)?;
        self.autonomy_resources = Some(provider);
        Ok(())
    }
    pub fn initialize_retained_graphic(
        &mut self,
        vm: &Vm,
        program: &Program,
        id: ObjectId,
    ) -> Result<()> {
        let mut ordinary = std::mem::take(&mut self.ordinary);
        let result = self.initialize_graphic(&mut ordinary, vm, program, id);
        self.ordinary = ordinary;
        result
    }
    pub fn is_complete(&self) -> bool {
        self.fresh.is_empty() && self.constants.is_empty() && self.initial_process.is_none()
    }

    /// Ordinary4376f6 at the factory's post-construction boundary. Keep the
    /// same CRT stream and shared mesh ownership as the loaded residents.
    pub fn initialize_graphic(
        &mut self,
        graphics: &mut crate::graphic_loading::Registry,
        vm: &Vm,
        program: &Program,
        id: ObjectId,
    ) -> Result<()> {
        graphics.begin(vm, program, id)?;
        for _ in 0..10_000 {
            let object = vm
                .world
                .objects
                .get(&id)
                .context("ordinary graphic owner absent")?;
            let position = vm
                .world
                .positions
                .get(&id)
                .context("ordinary graphic position absent")?;
            if graphics.advance(
                self.index,
                vm,
                program,
                id,
                &mut self.crt_state,
                self.settings,
                &mut self.mesh_pool,
                crate::graphic_initialization::Environment {
                    owner_appearance_index: *object
                        .data
                        .get(&68)
                        .context("ordinary appearance index absent")?,
                    source: sims_sim::pose::graphic_transform::Source {
                        position: [position.x, position.y],
                        facing: object.data[&1],
                        level: position.level,
                    },
                    view: self.view,
                    terrain: transform::Terrain {
                        size: self.house.width,
                        corners: &self.terrain,
                    },
                    viewport: self.presentation.viewport()?,
                },
            )? {
                return Ok(());
            }
        }
        anyhow::bail!("ordinary graphic initializer service budget exceeded")
    }

    fn initialize_character(
        &mut self,
        program: &mut Program,
        vm: &mut Vm,
        id: ObjectId,
    ) -> Result<Fresh> {
        ensure!(
            !vm.world.positions.contains_key(&id),
            "saved position read before fresh initialization"
        );
        //45962b prepares the built-in slots before binding the animator. The
        //saved-object allocator already reserved this layout; occupancy is read
        //later. Keep its initialized geometry too, as the live factory does.
        let object = vm.world.objects.get_mut(&id).context("saved person absent")?;
        ensure!(object.person.is_some()
            && object.slots.len() == sims_vm::person::BUILTIN_SLOT_HEIGHTS.len()
            && object.native_slot_geometry.is_none(),
            "saved person slot construction requires the native built-in layout");
        object.native_slot_geometry = Some(sims_vm::person::builtin_slot_geometry());
        let mut fresh = Fresh::begin(
            self.index,
            vm,
            program,
            id,
            self.settings,
            &mut self.crt_state,
            &mut self.skeletons,
            None,
        )?;
        for _ in 0..10_000 {
            if fresh.advance(
                self.index,
                vm,
                program,
                &mut self.presentation,
                &mut self.mesh_pool,
                &mut self.clothing,
                None,
                fresh::Environment {
                    position: [-16, -16],
                    level: 1,
                    view: self.view,
                    terrain: transform::Terrain {
                        size: self.house.width,
                        corners: &self.terrain,
                    },
                },
            )? {
                return Ok(fresh);
            }
        }
        anyhow::bail!("initializer service budget exceeded")
    }

    fn read_character(
        &mut self,
        program: &mut Program,
        vm: &mut Vm,
        id: ObjectId,
        saved: &sims_formats::saved_objects::Person,
        mut fresh: Fresh,
    ) -> Result<Loaded> {
        let mut owner = fresh.prepared.appearance_owner;
        let mut initialized = fresh.prepared.execution;
        let mut read = appearance_read::Read::from_saved(&fresh.avatar.records, saved, false)
            .map_err(anyhow::Error::msg)?;
        while let Some(request) = fresh.geometry.advance_saved_appearance(
            self.index,
            &mut fresh.avatar,
            &mut read,
            &mut owner,
            &mut initialized.bound,
            &mut initialized.scalars.placement,
            &mut self.mesh_pool,
        )? {
            ensure!(
                request == appearance_read::Request::TextureNotification,
                "unsupported saved appearance service {request:?}"
            );
            self.presentation.saved_texture(id)?;
            read.acknowledge(&mut owner, &mut fresh.avatar.records)
                .map_err(anyhow::Error::msg)?;
        }
        let context = self
            .presentation
            .context_at(id, &initialized.scalars.placement, owner.selection.cached[0], &vm.world)?;
        let now = initialized.scalars.clock;
        let (resident, model) = saved_layers::restore_initialized(
            self.index,
            program,
            saved,
            initialized,
            Censor {
                binding: owner.attachment,
                mask: owner.attachment_metadata as i32,
            },
            vm.tick,
            context,
            now,
        )?;
        let mut loaded = Loaded {
            resident,
            model,
            geometry: fresh.geometry,
            avatar: fresh.avatar,
            appearance_owner: owner,
        };
        self.refresh(program, vm, id, &mut loaded)?;
        Ok(loaded)
    }
    fn refresh(
        &mut self,
        program: &Program,
        vm: &mut Vm,
        id: ObjectId,
        loaded: &mut Loaded,
    ) -> Result<()> {
        let Loaded {
            resident,
            model,
            geometry,
            avatar,
            appearance_owner,
        } = loaded;
        let mut services = 0;
        loop {
            let context = self
                .presentation
                .context_at(id, &resident.character.placement, appearance_owner.selection.cached[0], &vm.world)?;
            ensure!(
                context.actor == id,
                "saved animator context belongs to another person"
            );
            resident.context = context;
            let position = *vm
                .world
                .positions
                .get(&id)
                .context("generic saved position absent")?;
            let object = vm
                .world
                .objects
                .get_mut(&id)
                .context("saved person absent")?;
            let fields = object
                .person
                .as_mut()
                .context("saved person fields absent")?;
            let Some(request) = resident
                .character
                .next(program, fields, context, position.level)
                .map_err(anyhow::Error::msg)?
            else {
                break;
            };
            services += 1;
            ensure!(services <= 10_000, "saved animator service budget exceeded");
            if crate::character_censor::service(
                self.index,
                resident,
                model,
                geometry,
                avatar,
                appearance_owner,
                &mut self.mesh_pool,
            )? {
                continue;
            }
            if geometry
                .service_resident(model, resident, avatar, &mut self.mesh_pool)?
                .is_some()
            {
                continue;
            }
            match request {
                Request::Placement => {
                    resident
                        .character
                        .place(
                            transform::Source {
                                position: [position.x, position.y],
                                facing: object.data[&1],
                                level: position.level,
                                flags: [fields[&84] as u8, fields[&68] as u8],
                            },
                            self.view,
                            transform::Terrain {
                                size: self.house.width,
                                corners: &self.terrain,
                            },
                        )
                        .map_err(anyhow::Error::msg)?;
                }
                Request::Invalidate => {
                    self.presentation.invalidate(id)?;
                    resident
                        .character
                        .acknowledge(Response::Done)
                        .map_err(anyhow::Error::msg)?;
                }
                Request::Property(request) => {
                    let response =
                        self.property(id, &request, &resident.character.placement, position.level)?;
                    resident
                        .character
                        .acknowledge(response)
                        .map_err(anyhow::Error::msg)?;
                }
                Request::RefreshClock => {
                    resident
                        .character
                        .acknowledge(Response::Clock(sims_sim::pose::ordinary::clock(
                            vm.tick as i32,
                            resident.controls.ordinary.clock_mode,
                        )))
                        .map_err(anyhow::Error::msg)?;
                }
                Request::HeadTarget { object: target } => {
                    let resolved = if let Some(object) = vm.world.objects.get(&target) {
                        let position = vm
                            .world
                            .positions
                            .get(&target)
                            .context("head target position absent")?;
                        let point = if object.person.is_some() {
                            let character = if target == id {
                                &resident.character
                            } else {
                                &self
                                    .characters
                                    .get(&target)
                                    .context("head target person not yet initialized")?
                                    .resident
                                    .character
                            };
                            character
                                .rig
                                .head
                                .map(|anchor| {
                                    character
                                        .hierarchy
                                        .absolute
                                        .get(anchor.bone)
                                        .context("head target bone absent")?
                                        .anchor_point(anchor.kind, character.matrix)
                                        .map_err(anyhow::Error::msg)
                                })
                                .transpose()?
                        } else {
                            Some(sims_sim::head_look::ordinary_target_point([
                                position.x, position.y,
                            ]))
                        };
                        Some(sims_sim::head_look::Target {
                            level: position.level,
                            point,
                        })
                    } else {
                        None
                    };
                    resident
                        .character
                        .acknowledge(Response::HeadTarget(resolved))
                        .map_err(anyhow::Error::msg)?;
                }
                Request::CarriedObject => {
                    ensure!(
                        // Native +130 points at a physical sentinel followed
                        // by logical slots. The dependent +48 record is slot1.
                        object.slots.get(1).is_none_or(Option::is_none),
                        "saved animator needs carried-object dependency refresh"
                    );
                    resident
                        .character
                        .acknowledge(Response::Done)
                        .map_err(anyhow::Error::msg)?;
                }
                other => anyhow::bail!("unhandled original animator read service {other:?}"),
            }
        }
        resident
            .character
            .validate(program)
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }
    fn property(
        &mut self,
        id: ObjectId,
        request: &properties::Request,
        placement: &transform::Placement,
        level: i32,
    ) -> Result<Response> {
        if matches!(request, properties::Request::FootstepSurface { .. })
            && properties::footstep_tile([placement.base[0], placement.base[1]], level)
                .map_err(anyhow::Error::msg)?
                .is_none()
        {
            return Ok(Response::PropertySurface(-1));
        }
        self.presentation.property(id, request)
    }
}

/// The profile adapter needs VM-shaped storage, but no script or clock runs
/// inside the reader. Preserve world mutations and verify both saved RNG streams.
fn with_object<T>(
    house: &House,
    world: &mut World,
    id: ObjectId,
    object: Object,
    run: impl FnOnce(&mut Vm) -> Result<T>,
) -> Result<(Object, T)> {
    ensure!(
        !world.objects.contains_key(&id),
        "reader object already registered"
    );
    let mut vm = Vm::new(std::mem::take(world), house.simulator.global_rng as u64);
    vm.tick = house.simulator.ticks as u64;
    vm.lot_rng_state = Some(house.simulator.lot_rng);
    vm.world.objects.insert(id, object);
    let result = run(&mut vm);
    let unchanged = vm.tick == house.simulator.ticks as u64
        && vm.rng_state == house.simulator.global_rng
        && vm.lot_rng_state == Some(house.simulator.lot_rng)
        && vm.threads.is_empty();
    let object = vm.world.objects.remove(&id);
    *world = vm.world;
    let result = result?;
    ensure!(
        unchanged,
        "animator reader advanced simulation or installed script frames"
    );
    Ok((object.context("animator reader removed person")?, result))
}

/// Saved45962b runs this same child before neighbor attachment. Load/Flush
/// remain source-bound owning-host requirements, not successful no-ops. The
/// Call and partial shared process survive errors in Reader for diagnostics;
/// PersonReader/import has no whole pending-loader serialization contract.
fn drive_saved_person_constants(
    world: &mut World,
    call: &mut sims_vm::autonomy_constants::Call,
) -> Result<()> {
    drive_saved_person_constants_with_resources(world, call, None)
}
fn drive_saved_person_constants_with_resources(
    world: &mut World,
    call: &mut sims_vm::autonomy_constants::Call,
    mut resources: Option<&mut crate::autonomy_resources::Provider>,
) -> Result<()> {
    drive_saved_person_constants_with_service(world, call, |request, table| {
        resources
            .as_deref_mut()
            .with_context(|| {
                format!(
                    "saved constructor requires source-bound autonomy resource service {request:?}"
                )
            })?
            .service(request, table)
    })
}

fn drive_saved_person_constants_with_service(
    world: &mut World,
    call: &mut sims_vm::autonomy_constants::Call,
    mut service: impl FnMut(
        &sims_vm::autonomy_constants::Request,
        Option<&sims_vm::autonomy_constants::Table>,
    ) -> Result<sims_vm::autonomy_constants::Reply>,
) -> Result<()> {
    loop {
        match sims_vm::autonomy_constants::advance(world, call).map_err(anyhow::Error::msg)? {
            sims_vm::autonomy_constants::Progress::Store(_) => {}
            sims_vm::autonomy_constants::Progress::Complete => return Ok(()),
            sims_vm::autonomy_constants::Progress::Service(request) => {
                let reply = service(&request, call.table())?;
                call.reply(reply).map_err(anyhow::Error::msg)?;
            }
        }
    }
}

fn apply_initial_process(
    world: &mut World,
    input: &mut Option<sims_vm::autonomy_constants::Owner>,
) -> Result<()> {
    if let Some(owner) = input.as_ref() {
        if let Some(current) = &world.autonomy_constants {
            ensure!(
                current == owner,
                "observed initial process cannot overwrite retained process history"
            );
        }
        if world.autonomy_constants.is_none() {
            world.autonomy_constants = Some(owner.clone());
        }
        *input = None;
    }
    Ok(())
}

enum ConstantsSource<'s, 'c> {
    Standalone,
    Catalog(
        &'s mut crate::object_catalog::Catalog<'c>,
        &'s mut crate::autonomy_resources::CatalogProvider,
    ),
}
impl<P: Presentation> Reader<'_, P> {
    fn initialize_with_constants(
        &mut self,
        program: &mut Program,
        world: &mut World,
        id: ObjectId,
        object: Object,
        source: ConstantsSource<'_, '_>,
    ) -> Result<Object> {
        ensure!(
            !self.fresh.contains_key(&id) && !self.characters.contains_key(&id),
            "person initialized twice"
        );
        let (mut object, fresh) = with_object(self.house, world, id, object, |vm| {
            self.initialize_character(program, vm, id)
        })?;
        self.fresh.insert(id, fresh);
        let call = match self.constants.entry(id) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert(
                world
                    .autonomy_constants
                    .as_ref()
                    .context("autonomy constants process owner is unknown")?
                    .begin(),
            ),
        };
        match source {
            ConstantsSource::Standalone => drive_saved_person_constants_with_resources(
                world,
                call,
                self.autonomy_resources.as_mut(),
            )?,
            ConstantsSource::Catalog(catalog, provider) => {
                drive_saved_person_constants_with_service(world, call, |request, table| {
                    provider.service(catalog, request, table)
                })?
            }
        }
        self.constants.remove(&id);
        // 45962b's conditional default generation follows animator binding.
        // These words are subsequently overwritten by 45cf2d's saved person
        // data, but the shared CRT stream must retain the constructor's draws.
        let person = object
            .person
            .as_mut()
            .context("saved constructor person absent")?;
        world
            .person
            .as_mut()
            .context("saved constructor neighborhood absent")?
            .attach_person_neighbor(
                person,
                object.definition.as_ref().unwrap().guid,
                sims_vm::person_factory::NeighborAttachment::Persistent,
            )
            .map_err(anyhow::Error::msg)?;
        if (46..54).all(|field| person[&field] < 11) {
            sims_vm::person_factory::default_traits(person, &mut self.crt_state)
                .map_err(anyhow::Error::msg)?;
        }
        Ok(object)
    }
}

impl<P: Presentation> PersonReader for Reader<'_, P> {
    fn process_input(&mut self, world: &mut World) -> Result<()> {
        apply_initial_process(world, &mut self.initial_process)
    }
    fn validate_catalog_resources(
        &self,
        catalog: &crate::object_catalog::Catalog<'_>,
        provider: &crate::autonomy_resources::CatalogProvider,
    ) -> Result<()> {
        ensure!(
            self.autonomy_resources.is_none() && self.initial_process.is_none(),
            "retained catalog overlaps a separate process/provider owner"
        );
        provider.validate_catalog(catalog)
    }
    fn initialize(
        &mut self,
        program: &mut Program,
        world: &mut World,
        id: ObjectId,
        object: Object,
    ) -> Result<Object> {
        self.initialize_with_constants(program, world, id, object, ConstantsSource::Standalone)
    }
    fn initialize_with_catalog(
        &mut self,
        program: &mut Program,
        world: &mut World,
        id: ObjectId,
        object: Object,
        catalog: &mut crate::object_catalog::Catalog<'_>,
        provider: &mut crate::autonomy_resources::CatalogProvider,
    ) -> Result<Object> {
        self.validate_catalog_resources(catalog, provider)?;
        self.initialize_with_constants(
            program,
            world,
            id,
            object,
            ConstantsSource::Catalog(catalog, provider),
        )
    }
    fn read_animator(
        &mut self,
        program: &mut Program,
        world: &mut World,
        id: ObjectId,
        object: Object,
        saved: &sims_formats::saved_objects::Person,
    ) -> Result<Object> {
        let fresh = self
            .fresh
            .remove(&id)
            .context("saved animator read before initialization")?;
        let (object, loaded) = with_object(self.house, world, id, object, |vm| {
            self.read_character(program, vm, id, saved, fresh)
        })?;
        self.characters.insert(id, loaded);
        Ok(object)
    }
}

impl<P: Presentation> crate::house_loading::NativeAnimatorHost for Reader<'_, P> {
    fn initialize_graphic(&mut self, vm: &Vm, program: &Program, id: ObjectId) -> Result<()> {
        self.initialize_retained_graphic(vm, program, id)
    }
    fn post_load(
        &mut self,
        effect: crate::house_loading::AnimatorEffect,
        execution: &mut crate::save_storage::Execution,
        program: &mut Program,
        _architecture: &sims_sim::architecture::Architecture,
        _placement: &mut sims_sim::lot_placement::State,
    ) -> Result<()> {
        use crate::house_loading::AnimatorEffect;
        let id = match effect {
            AnimatorEffect::Reset(id)
            | AnimatorEffect::ReloadAppearance(id)
            | AnimatorEffect::ResetPlacementCache(id)
            | AnimatorEffect::Rebuild(id) => id,
        };
        let mut loaded = self
            .characters
            .remove(&id)
            .context("post-load animator was not read")?;
        let result = (|| -> Result<()> {
            match effect {
                AnimatorEffect::Reset(_) => {
                    let now = sims_sim::pose::ordinary::clock(
                        execution.vm.tick as i32,
                        loaded.resident.controls.ordinary.clock_mode,
                    );
                    if let Some(error) = loaded.resident.reset(now).map_err(anyhow::Error::msg)? {
                        self.route_diagnostic = Some(error);
                    }
                }
                AnimatorEffect::ReloadAppearance(_) => {
                    let mut service =
                        character_appearance::Service::begin(id, loaded.appearance_owner.clone());
                    let mut placement = loaded.resident.character.placement.clone();
                    loop {
                        let request = loaded.model.with_bound(&mut loaded.resident, |bound| {
                            service.advance_bound(
                                self.index,
                                &mut execution.vm,
                                program,
                                &mut loaded.avatar,
                                &mut loaded.geometry,
                                bound,
                                &mut placement,
                                &mut self.mesh_pool,
                            )
                        })?;
                        loaded.resident.character.placement = placement.clone();
                        let Some(request) = request else { break };
                        self.presentation.appearance(id, &request)?;
                        service.acknowledge(&mut loaded.avatar.records)?;
                    }
                    loaded.appearance_owner = service.owner;
                    loaded.resident.character.censor = Censor {
                        binding: loaded.appearance_owner.attachment,
                        mask: loaded.appearance_owner.attachment_metadata as i32,
                    };
                }
                AnimatorEffect::ResetPlacementCache(_) => {
                    loaded.resident.character.placement.invalidate()
                }
                AnimatorEffect::Rebuild(_) => {
                    let now = sims_sim::pose::ordinary::clock(
                        execution.vm.tick as i32,
                        loaded.resident.controls.ordinary.clock_mode,
                    );
                    loaded
                        .resident
                        .character
                        .begin(now, false, loaded.resident.character.visible, true)
                        .map_err(anyhow::Error::msg)?;
                    self.refresh(program, &mut execution.vm, id, &mut loaded)?;
                }
            }
            Ok(())
        })();
        self.characters.insert(id, loaded);
        result
    }
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[path = "character_loading/constants_tests.rs"]
mod constants_tests;
