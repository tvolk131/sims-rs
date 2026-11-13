//! Shared fresh animator host for original-file loading and live NPC creation.
//! One pending service and its actual bindings survive each host yield.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub(super) struct Fresh {
    pub prepared: Prepared,
    pub geometry: CharacterGeometry,
    pub avatar: BoundAvatar,
    reload: Option<character_appearance::Service>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    prepared: Prepared,
    geometry: CharacterGeometry,
    binding: crate::saved_avatar::BindingState,
    reload: Option<character_appearance::Service>,
}

pub(super) struct Environment<'a> {
    pub position: [i32; 2],
    pub level: i32,
    pub view: transform::View,
    pub terrain: transform::Terrain<'a>,
}

impl Fresh {
    pub fn clothing_used(&self) -> bool {
        self.reload.as_ref().is_some_and(|r| r.clothing_used())
    }
    pub fn begin(
        index: &Index,
        vm: &mut Vm,
        program: &Program,
        id: ObjectId,
        settings: Settings,
        crt: &mut u32,
        resources: &mut crate::skeleton_resources::Cache,
        mut startup: Option<&mut crate::startup_memo::Memo>,
    ) -> Result<Self> {
        let prepared = Prepared::begin_with_startup(
            index,
            vm,
            program,
            id,
            settings,
            crt,
            resources,
            startup.as_deref_mut(),
        )?;
        let avatar = BoundAvatar::empty_with_startup(
            index,
            &prepared.execution.source.name,
            id as u64,
            startup,
        )?;
        let geometry = CharacterGeometry::new(avatar.avatar.skeleton.bones.len());
        Ok(Self {
            prepared,
            geometry,
            avatar,
            reload: None,
        })
    }

    pub fn snapshot(&self) -> Saved {
        Saved {
            prepared: self.prepared.clone(),
            geometry: self.geometry.clone(),
            binding: self.avatar.snapshot(),
            reload: self.reload.clone(),
        }
    }

    pub fn restore(
        index: &Index,
        saved: Saved,
        startup: Option<&mut crate::startup_memo::Memo>,
    ) -> Result<Self> {
        let id = saved.prepared.object;
        ensure!(id > 0, "invalid fresh animator owner");
        if let Some(reload) = &saved.reload {
            ensure!(
                reload.object == id
                    && saved.prepared.execution.pending_request()
                        == Some(&initialize::Request::ReloadProfile),
                "invalid fresh profile continuation"
            );
        }
        let avatar = BoundAvatar::restore_with_startup(
            index,
            &saved.prepared.execution.source.name,
            id as u64,
            &saved.binding,
            startup,
        )?;
        ensure!(
            avatar.avatar.skeleton == saved.prepared.execution.source,
            "fresh animator skeleton changed"
        );
        Ok(Self {
            prepared: saved.prepared,
            geometry: saved.geometry,
            avatar,
            reload: saved.reload,
        })
    }

    /// Execute one returned service. Resource-only steps may run internally;
    /// completion means the full native initializer has finished.
    pub fn advance<P: Presentation>(
        &mut self,
        index: &Index,
        vm: &mut Vm,
        program: &mut Program,
        presentation: &mut P,
        pool: &mut MeshPool,
        clothing: &mut crate::clothing_catalog::Cache,
        environment: Environment<'_>,
    ) -> Result<bool> {
        let id = self.prepared.object;
        let context = presentation.context_at(
            id,
            &self.prepared.execution.scalars.placement,
            self.prepared.appearance_owner.selection.cached[0],
            &vm.world,
        )?;
        ensure!(
            context.actor == id,
            "initializer context belongs to another person"
        );
        let Some(request) = self.prepared.advance(index, vm, program, context)? else {
            ensure!(
                self.reload.is_none(),
                "completed initializer still owns a reload"
            );
            return Ok(true);
        };
        match request {
            initialize::Request::ReloadProfile => {
                if self.reload.is_none() {
                    self.reload = Some(self.prepared.appearance_reload()?);
                }
                let service = self.reload.as_mut().unwrap();
                if let Some(request) = service.advance_bound_with_clothing(
                    index,
                    vm,
                    program,
                    &mut self.avatar,
                    &mut self.geometry,
                    &mut self.prepared.execution.bound,
                    &mut self.prepared.execution.scalars.placement,
                    pool,
                    &mut crate::clothing_catalog::Installed {
                        index,
                        cache: clothing,
                    },
                )? {
                    character_appearance::notify_images(vm, id, &request)?;
                    presentation.appearance(id, &request)?;
                    service.acknowledge(&mut self.avatar.records)?;
                } else {
                    self.prepared
                        .appearance_reloaded(self.reload.take().unwrap())?;
                }
            }
            initialize::Request::Placement => {
                let object = &vm.world.objects[&id];
                let fields = object
                    .person
                    .as_ref()
                    .context("fresh person storage absent")?;
                self.prepared
                    .execution
                    .place(
                        transform::Source {
                            position: environment.position,
                            facing: object.data[&1],
                            level: environment.level,
                            flags: [fields[&84] as u8, fields[&68] as u8],
                        },
                        environment.view,
                        environment.terrain,
                    )
                    .map_err(anyhow::Error::msg)?;
            }
            initialize::Request::Geometry { .. } => {
                self.prepared.refresh_geometry(&mut self.geometry, pool)?;
            }
            initialize::Request::Property(request) => {
                let placement = &self.prepared.execution.scalars.placement;
                let response = if matches!(request, properties::Request::FootstepSurface { .. })
                    && properties::footstep_tile(
                        [placement.base[0], placement.base[1]],
                        environment.level,
                    )
                    .map_err(anyhow::Error::msg)?
                    .is_none()
                {
                    Response::PropertySurface(-1)
                } else {
                    presentation.property(id, &request)?
                };
                let value = match response {
                    Response::Done => None,
                    Response::PropertySurface(value) => Some(value),
                    other => anyhow::bail!("invalid initializer property response {other:?}"),
                };
                self.prepared
                    .execution
                    .property_done(value)
                    .map_err(anyhow::Error::msg)?;
            }
            initialize::Request::Viewport => self
                .prepared
                .execution
                .viewport(presentation.viewport()?)
                .map_err(anyhow::Error::msg)?,
            initialize::Request::Visibility { viewport } => {
                let visible = presentation.visibility_for_initialization(
                    id,
                    &self.prepared.execution,
                    viewport,
                )?;
                self.prepared
                    .execution
                    .visibility(visible)
                    .map_err(anyhow::Error::msg)?;
            }
            other => anyhow::bail!("unhandled initializer request {other:?}"),
        }
        Ok(false)
    }

    pub fn into_loaded<P: Presentation>(
        self,
        tick: u64,
        presentation: &mut P,
        world: &World,
    ) -> Result<Loaded> {
        ensure!(self.reload.is_none(), "fresh animator profile is pending");
        let owner = self.prepared.appearance_owner;
        let context = presentation.context_at(
            self.prepared.object,
            &self.prepared.execution.scalars.placement,
            owner.selection.cached[0],
            world,
        )?;
        ensure!(
            context.actor == self.prepared.object,
            "fresh animator handoff context belongs to another person"
        );
        // 58464f writes +a4 and +a0 to zero. No saved animator words exist for
        // a newly constructed NPC, and its unwritten head target stays absent.
        let (resident, model) = character_model::State::handoff(
            self.prepared.execution,
            Censor {
                binding: owner.attachment,
                mask: owner.attachment_metadata as i32,
            },
            tick,
            0,
            0,
            context,
        )?;
        Ok(Loaded {
            resident,
            model,
            geometry: self.geometry,
            avatar: self.avatar,
            appearance_owner: owner,
        })
    }
}
