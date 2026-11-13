//! Retained ordinary graphic owners, shared by load-time and live factories.
//! This module handles fresh4376f6 ownership. Saved native ordinary Animators,
//! selection attachment, frame scheduling and destruction have separate owners.
use crate::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    graphic_initialization::{self, Environment, Prepared, Ready},
    graphic_materials::GraphicMaterials,
    graphic_profiles::Cached,
    graphic_update::{self, Command, Step, Update},
    saved_avatar::{BindingState, BoundAvatar},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::{
    animator::Settings, graphic_appearance, graphic_frame, graphic_owner as owner, model::Bound,
};
use sims_vm::{Definition, ObjectId, Program, Vm};
use std::collections::BTreeMap;

pub mod retained;

#[derive(Clone)]
pub struct Graphic {
    pub marker_value: Option<i32>,
    pub owner: owner::State,
    pub ready: Ready,
    pub materials: Option<GraphicMaterials>,
    update: Option<Update>,
}
impl Graphic {
    pub fn update_pending(&self) -> bool {
        self.update.as_ref().is_some_and(|u| !u.complete())
    }
    pub fn begin_update(&mut self, command: Command) -> Result<()> {
        ensure!(self.owner.resource != 0, "ordinary graphic is retiring");
        ensure!(
            self.update.as_ref().is_none_or(Update::complete),
            "ordinary graphic update already active"
        );
        let update = Update::new(self.ready.object, command);
        update.validate(self.ready.object)?;
        self.update = Some(update);
        Ok(())
    }
    pub fn advance_update(
        &mut self,
        index: &Index,
        program: &Program,
        pool: &mut MeshPool,
        env: graphic_update::Environment<'_>,
    ) -> Result<Step> {
        let mut update = self
            .update
            .take()
            .context("ordinary graphic update absent")?;
        let result = update.advance(index, program, self, pool, env);
        self.update = Some(update);
        result
    }
    pub fn acknowledge_update(&mut self, request: &graphic_update::Request) -> Result<()> {
        let mut update = self
            .update
            .take()
            .context("ordinary graphic update absent")?;
        let result = update.acknowledge(self, request);
        self.update = Some(update);
        result
    }
}
#[derive(Clone)]
struct Job {
    definition: Definition,
    profile: usize,
    owner: owner::State,
    update: owner::Update,
    prepared: Option<Prepared>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SavedJob {
    definition: Definition,
    profile: usize,
    owner: owner::State,
    update: owner::Update,
    prepared: Option<graphic_initialization::Saved>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedGraphic {
    #[serde(default)]
    marker_value: Option<i32>,
    owner: owner::State,
    definition: Definition,
    profile: Vec<String>,
    frame: graphic_frame::State,
    appearance: graphic_appearance::State,
    bound: Bound,
    geometry: CharacterGeometry,
    skeleton: String,
    binding: BindingState,
    #[serde(default)]
    materials: Option<GraphicMaterials>,
    #[serde(default)]
    update: Option<Update>,
}
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    profiles: Vec<Cached>,
    jobs: BTreeMap<ObjectId, SavedJob>,
    pub graphics: BTreeMap<ObjectId, SavedGraphic>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub retirements: BTreeMap<ObjectId, crate::graphic_retirement::Job>,
    #[serde(default, skip_serializing_if = "retained::Registry::is_empty")]
    pub retained: retained::Registry,
}
#[derive(Default, Clone)]
pub struct Registry {
    profiles: Vec<Cached>,
    jobs: BTreeMap<ObjectId, Job>,
    pub graphics: BTreeMap<ObjectId, Graphic>,
    pub retirements: BTreeMap<ObjectId, crate::graphic_retirement::Job>,
    pub retained: retained::Registry,
}
impl Saved {
    /// Borrow ready bytes from the saved owners without loading meshes or
    /// constructing a second graphics registry.
    pub fn presenter_readiness_with(
        &self,
        vm: &Vm,
        definitions: &dyn sims_sim::lot_presenter::Definitions,
    ) -> Result<BTreeMap<ObjectId, u8>> {
        presenter_readiness(vm, definitions, |id| {
            self.graphics.get(&id).map(|g| g.owner.ready)
                .or_else(|| self.retained.ready(id))
                .or_else(|| self.jobs.get(&id).map(|j| j.owner.ready))
        })
    }
    pub fn bind_observed_process(
        &mut self,
        control: u16,
        marker_values: &BTreeMap<ObjectId, i32>,
    ) -> Result<()> {
        ensure!(
            self.graphics.keys().eq(marker_values.keys()),
            "ordinary process owner inventory differs"
        );
        let mut next = self.clone();
        for (&id, graphic) in &mut next.graphics {
            graphic
                .bound
                .skeleton
                .skin
                .animator
                .bind_fp_control(control)
                .map_err(anyhow::Error::msg)?;
            graphic.marker_value = Some(marker_values[&id]);
        }
        *self = next;
        Ok(())
    }
}
impl Registry {
    /// Bind a translated native object identity to its existing allocations.
    /// The importer supplies the proven definition and actual +180 ready byte.
    pub fn install_retained(
        &mut self,
        id: ObjectId,
        definition: Definition,
        registry_lifetime: u32,
        ready: u8,
        input: crate::graphic_retirement::observed::Retained,
    ) -> Result<()> {
        ensure!(
            !self.jobs.contains_key(&id)
                && !self.graphics.contains_key(&id)
                && !self.retirements.contains_key(&id),
            "retained graphics overlap constructed storage"
        );
        self.retained
            .insert(id, definition, registry_lifetime, ready, input)
    }
    /// Install an existing raw frame owner and explicit shared process inputs.
    /// No constructor, transform, clock query or RNG draw is performed.
    pub fn install_retained_frame(
        &mut self,
        binding: crate::graphic_retirement::observed::frame::Binding,
        ready: u8,
        input: crate::graphic_retirement::observed::frame::Input,
        context: retained::RefreshContext,
    ) -> Result<()> {
        let id = binding.object;
        ensure!(
            !self.jobs.contains_key(&id)
                && !self.graphics.contains_key(&id)
                && !self.retirements.contains_key(&id),
            "retained graphics overlap constructed storage"
        );
        self.retained.insert_frame(binding, ready, input, context)
    }
    pub fn owned_objects(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.graphics.keys().copied().chain(self.retained.objects())
    }
    pub fn retirement_started(&self, id: ObjectId) -> bool {
        self.retirements.contains_key(&id) || self.retained.started(id)
    }
    /// Actual object +180 ownership for the native presenter. A pending Init
    /// retains its zero byte; definition flags alone cannot prove readiness.
    pub fn presenter_readiness(
        &self,
        vm: &Vm,
        program: &Program,
    ) -> Result<BTreeMap<ObjectId, u8>> {
        presenter_readiness(vm, program, |id| {
            self.graphics.get(&id).map(|g| g.owner.ready)
                .or_else(|| self.retained.ready(id))
                .or_else(|| self.jobs.get(&id).map(|j| j.owner.ready))
        })
    }
    pub fn begin_retirement(&mut self, id: ObjectId, focus: bool) -> Result<()> {
        ensure!(
            !self.retirements.contains_key(&id) && !self.jobs.contains_key(&id),
            "ordinary retirement already active"
        );
        if self.retained.contains(id) {
            return self.retained.begin(id, focus);
        }
        let graphic = self
            .graphics
            .get_mut(&id)
            .context("ordinary retirement graphic absent")?;
        let job = crate::graphic_retirement::Job::begin(graphic, focus)?;
        // cXObject432331 releases its resource reference before its Animator.
        // Shared immutable decoded catalogs have independent cache ownership.
        graphic.owner.resource = 0;
        job.validate(graphic)?;
        self.retirements.insert(id, job);
        Ok(())
    }
    pub fn advance_retirement(
        &mut self,
        index: &Index,
        id: ObjectId,
        pool: &mut MeshPool,
        tick: i32,
        settings: Settings,
        focus: bool,
        notify: impl FnMut(sims_sim::pose::graphic_destruction::Request) -> Result<()>,
    ) -> Result<bool> {
        if self.retained.contains(id) {
            //This profile executes direct owned584898 services. It does not
            //reconstruct a Bound rig, sample a clock or query person settings.
            return self.retained.advance(id, focus, notify);
        }
        let mut job = self
            .retirements
            .get(&id)
            .context("ordinary retirement absent")?
            .clone();
        let mut graphic = self
            .graphics
            .get(&id)
            .context("retired ordinary graphic absent")?
            .clone();
        let mut next_pool = pool.clone();
        if job.execution.pending().is_none() {
            job.state.focus = focus;
        }
        let complete = job.advance(index, &mut graphic, &mut next_pool, tick, settings, notify)?;
        if complete {
            self.retirements.remove(&id);
            self.graphics.remove(&id);
        } else {
            self.retirements.insert(id, job);
            self.graphics.insert(id, graphic);
        }
        *pool = next_pool;
        Ok(complete)
    }
    pub fn pending_objects(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.jobs.keys().copied()
    }
    pub fn updates_pending(&self) -> bool {
        self.pending_updates().next().is_some()
    }
    pub fn pending_updates(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.graphics
            .iter()
            .filter_map(|(&id, g)| {
                g.update
                    .as_ref()
                    .is_some_and(|u| !u.complete())
                    .then_some(id)
            })
            .chain(self.retained.pending_updates())
    }
    pub fn pending(&self, id: ObjectId) -> bool {
        self.jobs.contains_key(&id)
    }
    pub fn begin(&mut self, vm: &Vm, p: &Program, id: ObjectId) -> Result<()> {
        ensure!(
            id > 0
                && !self.jobs.contains_key(&id)
                && !self.graphics.contains_key(&id)
                && !self.retained.contains(id),
            "ordinary graphic owner already exists"
        );
        let object = vm
            .world
            .objects
            .get(&id)
            .context("ordinary graphic object absent")?;
        let definition = object
            .definition
            .as_ref()
            .context("ordinary graphic definition absent")?;
        let words =
            sims_vm::construction::definition_words(p, definition).map_err(anyhow::Error::msg)?;
        ensure!(
            words[9] != 2 && words.get(102).is_some_and(|w| w & 1 != 0),
            "ordinary 3D graphic definition required"
        );
        let profile = if let Some(i) = self
            .profiles
            .iter()
            .position(|v| v.definition() == definition)
        {
            i
        } else {
            let i = self.profiles.len();
            self.profiles.push(Cached::new(definition.clone(), words)?);
            i
        };
        self.jobs.insert(
            id,
            Job {
                definition: definition.clone(),
                profile,
                owner: owner::State {
                    definition_flags: words[102] as u8,
                    resource: profile as u64 + 1,
                    graphic: None,
                    ready: 0,
                },
                update: owner::Update::new(id as u64),
                prepared: None,
            },
        );
        Ok(())
    }
    /// At most one actual profile/initializer service per call. The caller must
    /// retain the object factory and must not stamp dates or run Init meanwhile.
    pub fn advance(
        &mut self,
        index: &Index,
        vm: &Vm,
        p: &Program,
        id: ObjectId,
        crt: &mut u32,
        settings: Settings,
        pool: &mut MeshPool,
        env: Environment<'_>,
    ) -> Result<bool> {
        let job = self
            .jobs
            .get_mut(&id)
            .context("ordinary graphic construction absent")?;
        ensure!(
            vm.world
                .objects
                .get(&id)
                .and_then(|o| o.definition.as_ref())
                == Some(&job.definition),
            "ordinary graphic definition changed during construction"
        );
        let request = job
            .update
            .advance(&job.owner)
            .map_err(anyhow::Error::msg)?
            .context("ordinary owner completed without handoff")?;
        let mut ready = None;
        let reply = match &request {
            owner::Request::Profile { resource } => {
                ensure!(*resource == job.profile as u64 + 1);
                let profile = self
                    .profiles
                    .get_mut(job.profile)
                    .context("ordinary profile owner absent")?;
                if !profile.advance(index, &mut |_, _, _| {
                    anyhow::bail!("ordinary profile requires mounted StringSet writeback")
                })? {
                    return Ok(false);
                }
                owner::Reply::Profile
            }
            owner::Request::Construct => {
                ensure!(job.prepared.is_none(), "ordinary Animator allocated twice");
                let words = sims_vm::construction::definition_words(p, &job.definition)
                    .map_err(anyhow::Error::msg)?;
                job.prepared = Some(Prepared::begin_cached(
                    id,
                    &self.profiles[job.profile],
                    words,
                    vm.tick as i32,
                    settings,
                )?);
                owner::Reply::Constructed(Some(id as u64))
            }
            owner::Request::Initialize { graphic, owner } => {
                ensure!(*graphic == id as u64 && *owner == id as u64);
                let prepared = job
                    .prepared
                    .as_mut()
                    .context("ordinary Animator lost initializer")?;
                if !prepared.advance(index, crt, pool, env)? {
                    return Ok(false);
                }
                // Validate the handoff before acknowledging the owner's Init.
                // Failure retains a retryable, serializable Initialize request.
                ready = Some(Prepared::restore(index, prepared.snapshot())?.into_ready()?);
                owner::Reply::Initialized(1)
            }
        };
        job.update
            .acknowledge(&mut job.owner, &request, reply)
            .map_err(anyhow::Error::msg)?;
        if job.update.complete() {
            let ready = ready.context("completed ordinary owner has no model")?;
            self.graphics.insert(
                id,
                Graphic {
                    marker_value: None,
                    owner: job.owner.clone(),
                    ready,
                    materials: None,
                    update: None,
                },
            );
            self.jobs.remove(&id);
            return Ok(true);
        }
        Ok(false)
    }
    pub fn snapshot(&self) -> Saved {
        Saved {
            retained: self.retained.clone(),
            retirements: self.retirements.clone(),
            profiles: self.profiles.clone(),
            jobs: self
                .jobs
                .iter()
                .map(|(&id, j)| {
                    (
                        id,
                        SavedJob {
                            definition: j.definition.clone(),
                            profile: j.profile,
                            owner: j.owner.clone(),
                            update: j.update.clone(),
                            prepared: j.prepared.as_ref().map(Prepared::snapshot),
                        },
                    )
                })
                .collect(),
            graphics: self
                .graphics
                .iter()
                .map(|(&id, g)| {
                    let r = &g.ready;
                    (
                        id,
                        SavedGraphic {
                            marker_value: g.marker_value,
                            owner: g.owner.clone(),
                            definition: r.definition.clone(),
                            profile: r.profile.clone(),
                            frame: r.frame.clone(),
                            appearance: r.appearance.clone(),
                            bound: r.bound.clone(),
                            geometry: r.geometry.clone(),
                            skeleton: r.avatar.avatar.skeleton.name.clone(),
                            binding: r.avatar.snapshot(),
                            materials: g.materials.clone(),
                            update: g.update.clone(),
                        },
                    )
                })
                .collect(),
        }
    }
    pub fn restore(index: &Index, saved: Saved) -> Result<Self> {
        saved.retained.validate_storage()?;
        ensure!(
            saved
                .retained
                .objects()
                .all(|id| !saved.graphics.contains_key(&id)
                    && !saved.jobs.contains_key(&id)
                    && !saved.retirements.contains_key(&id)),
            "retained graphics overlap constructed storage"
        );
        let profiles = saved
            .profiles
            .into_iter()
            .map(|v| Cached::restore(index, v))
            .collect::<Result<Vec<_>>>()?;
        for (i, p) in profiles.iter().enumerate() {
            ensure!(
                !profiles[..i]
                    .iter()
                    .any(|v| v.definition() == p.definition()),
                "duplicate ordinary definition cache"
            );
        }
        let mut jobs = BTreeMap::new();
        for (id, j) in saved.jobs {
            ensure!(
                id > 0 && !saved.graphics.contains_key(&id),
                "duplicate ordinary graphic ownership"
            );
            j.update.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                j.update.owner() == id as u64
                    && profiles
                        .get(j.profile)
                        .is_some_and(|v| v.definition() == &j.definition)
                    && j.owner.resource == j.profile as u64 + 1
                    && j.owner.graphic == j.prepared.as_ref().map(|_| id as u64)
                    && j.owner.ready == 0
                    && !j.update.complete(),
                "invalid pending ordinary graphic ownership"
            );
            let prepared = j
                .prepared
                .map(|v| Prepared::restore(index, v))
                .transpose()?;
            if let Some(prepared) = &prepared {
                prepared.validate_owner(
                    id,
                    &j.definition,
                    profiles[j.profile]
                        .strings()?
                        .context("allocated ordinary owner has no profile")?,
                )?;
            }
            jobs.insert(
                id,
                Job {
                    definition: j.definition,
                    profile: j.profile,
                    owner: j.owner,
                    update: j.update,
                    prepared,
                },
            );
        }
        let mut graphics = BTreeMap::new();
        for (id, g) in saved.graphics {
            if let Some(update) = &g.update {
                update.validate(id)?;
            }
            ensure!(
                g.appearance.dirty == g.frame.placement.dirty,
                "saved ordinary dirty aliases differ"
            );
            let resource = saved
                .retirements
                .get(&id)
                .map_or(g.owner.resource, |r| r.profile_resource);
            let profile = usize::try_from(resource)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| profiles.get(i))
                .context("ordinary graphic profile owner absent")?;
            ensure!(
                id > 0
                    && g.owner.graphic == Some(id as u64)
                    && g.owner.ready == 1
                    && profile.definition() == &g.definition
                    && profile.strings()? == Some(g.profile.as_slice()),
                "invalid completed ordinary graphic ownership"
            );
            let avatar = BoundAvatar::restore(index, &g.skeleton, id as u64, &g.binding)?;
            ensure!(
                avatar.avatar.skeleton.bones.len() == g.bound.skeleton.skin.bones.len(),
                "ordinary graphic rig changed"
            );
            graphics.insert(
                id,
                Graphic {
                    marker_value: g.marker_value,
                    owner: g.owner,
                    materials: g.materials,
                    update: g.update,
                    ready: Ready {
                        object: id,
                        definition: g.definition,
                        profile: g.profile,
                        frame: g.frame,
                        appearance: g.appearance,
                        bound: g.bound,
                        geometry: g.geometry,
                        avatar,
                    },
                },
            );
        }
        for (&id, job) in &saved.retirements {
            job.validate(
                graphics
                    .get(&id)
                    .context("retired ordinary storage absent")?,
            )?;
        }
        Ok(Self {
            retained: saved.retained,
            profiles,
            jobs,
            graphics,
            retirements: saved.retirements,
        })
    }
    pub fn validate(&self, vm: &Vm) -> Result<()> {
        self.validate_with_retired(vm, &Default::default())
    }
    pub fn validate_with_retired(
        &self,
        vm: &Vm,
        retired: &BTreeMap<ObjectId, sims_sim::removal::graphic::Retired>,
    ) -> Result<()> {
        self.retained.validate(vm, retired)?;
        ensure!(
            self.retained
                .objects()
                .all(|id| !self.graphics.contains_key(&id)
                    && !self.jobs.contains_key(&id)
                    && !self.retirements.contains_key(&id)),
            "retained graphics overlap constructed storage"
        );
        for (&id, g) in &self.graphics {
            ensure!(
                g.ready.object == id
                    && (vm
                        .world
                        .objects
                        .get(&id)
                        .and_then(|o| o.definition.as_ref())
                        == Some(&g.ready.definition)
                        || retired
                            .get(&id)
                            .is_some_and(|r| r.definition == g.ready.definition)
                            && !vm.world.objects.contains_key(&id)),
                "ordinary graphic outlived its object registration"
            );
        }
        for (&id, r) in retired {
            ensure!(
                (self.graphics.contains_key(&id) || self.retained.contains(id))
                    && r.asset_service_started == self.retirement_started(id),
                "retired ordinary graphic lost its continuation"
            );
        }
        for (&id, job) in &self.retirements {
            ensure!(
                retired.contains_key(&id),
                "ordinary destructor lost its deletion owner"
            );
            job.validate(&self.graphics[&id])?;
        }
        for (&id, j) in &self.jobs {
            ensure!(
                vm.world
                    .objects
                    .get(&id)
                    .and_then(|o| o.definition.as_ref())
                    == Some(&j.definition),
                "ordinary graphic factory lost object registration"
            );
        }
        Ok(())
    }
}

// Shared query order for resident and serialized graphic owners.
fn presenter_readiness(
    vm: &Vm,
    definitions: &dyn sims_sim::lot_presenter::Definitions,
    mut owned: impl FnMut(ObjectId) -> Option<u8>,
) -> Result<BTreeMap<ObjectId,u8>> {
    let mut result = BTreeMap::new();
    for (&id, object) in &vm.world.objects {
        if object.person.is_some() {
            continue;
        }
        let definition = object
            .definition
            .as_ref()
            .context("presenter definition absent")?;
        let words = definitions.words(definition)
            .map_err(anyhow::Error::msg)?;
        let ready = if let Some(ready) = owned(id) {
            ready
        } else if words
            .get(102)
            .context("presenter graphics ownership flags absent")?
            & 1
            == 0
        {
            // 43189d constructor zero; 4376f6's flag-clear branch keeps it.
            0
        } else {
            anyhow::bail!("presenter ordinary graphic ownership absent: {id}");
        };
        result.insert(id, ready);
    }
    Ok(result)
}
