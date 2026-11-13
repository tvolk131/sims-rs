//! Asset services for ordinary Animator584898. The model is retained while
//! its native destructor is suspended; immutable catalog caches are separate.
pub mod observed;

use crate::character_loading::RetirementEvent;
use crate::{Index, character_geometry::MeshPool, graphic_loading::Graphic};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::pose::{
    animator::{LayerId, Settings},
    appearance, graphic_destruction as destruction, graphic_records as records, graphic_reset,
    ordinary,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub execution: destruction::Execution,
    pub state: destruction::State,
    clear: Option<records::Update>,
    /// Catalog identity retained for restore, after cXObject releases +11c.
    pub profile_resource: u64,
}
/// One asset-side retirement service shared by the load Reader and live Host.
/// Registry unlink and the destructor cursor stay owned by Simulation. This
/// completes real asset work before acknowledging the matching native request.
pub(crate) fn service<P: crate::character_loading::Presentation>(
    index: &Index,
    ordinary: &mut crate::graphic_loading::Registry,
    pool: &mut MeshPool,
    settings: Settings,
    presentation: &mut P,
    simulation: &mut sims_sim::Simulation,
    id: sims_vm::ObjectId,
) -> Result<()> {
    let retired = simulation
        .retired_graphics
        .get(&id)
        .context("ordinary retirement absent")?;
    simulation
        .validate_graphic_retirement_owner(id)
        .map_err(anyhow::Error::msg)?;
    ordinary.validate_with_retired(&simulation.vm, &simulation.retired_graphics)?;
    let refs = presentation.retirement_references(id)?;
    if !retired.asset_service_started {
        ordinary.begin_retirement(id, refs.base_focus)?;
        simulation
            .begin_graphic_retirement(id)
            .map_err(anyhow::Error::msg)?;
        return Ok(());
    }
    let tick = retired.tick as i32;
    if ordinary.advance_retirement(
        index,
        id,
        pool,
        tick,
        settings,
        refs.base_focus,
        |request| {
            use sims_sim::pose::graphic_destruction::Request;
            presentation.retire(
                id,
                match request {
                    Request::DestroyModel(_) => RetirementEvent::ModelRemoved,
                    Request::ClearFocus => RetirementEvent::ClearBaseFocus,
                    _ => anyhow::bail!("unexpected ordinary destruction notification"),
                },
            )
        },
    )? {
        simulation
            .complete_graphic_retirement(id)
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

impl Job {
    pub fn begin(graphic: &Graphic, focus: bool) -> Result<Self> {
        ensure!(
            !graphic.update_pending(),
            "ordinary destruction overlaps an update"
        );
        let r = &graphic.ready;
        ensure!(
            r.bound.skeleton.skin.animator.destruction_layers()[3].is_none(),
            "ordinary Animator owns a person carry layer"
        );
        ensure!(
            r.avatar.auxiliary().is_empty(),
            "ordinary Animator has unowned auxiliary bindings"
        );
        Ok(Self {
            execution: Default::default(),
            state: destruction::State {
                model: Some(r.object as u64),
                layers: layers(graphic),
                auxiliary: r
                    .avatar
                    .records
                    .accessories
                    .iter()
                    .map(|&i| Some(i as u64))
                    .collect(),
                generic: vec![Some(r.avatar.records.generic as u64)],
                focus,
            },
            clear: None,
            profile_resource: graphic.owner.resource,
        })
    }
    pub fn validate(&self, graphic: &Graphic) -> Result<()> {
        self.execution
            .validate(&self.state)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            self.profile_resource != 0
                && graphic.owner.resource == 0
                && graphic.owner.graphic == Some(graphic.ready.object as u64)
                && !graphic.update_pending(),
            "invalid retired ordinary graphic owner"
        );
        ensure!(
            self.state.layers == layers(graphic),
            "retired ordinary layer pointers changed"
        );
        ensure!(
            self.state
                .model
                .is_none_or(|id| id == graphic.ready.object as u64),
            "retired ordinary model identity changed"
        );
        let records = &graphic.ready.avatar.records;
        ensure!(
            self.state.generic.len() == 1
                && self.state.auxiliary.len() == records.accessories.len(),
            "retired ordinary record vector changed"
        );
        for (id, expected) in self.state.generic.iter().zip([records.generic]).chain(
            self.state
                .auxiliary
                .iter()
                .zip(records.accessories.iter().copied()),
        ) {
            ensure!(
                match id {
                    Some(id) => *id == expected as u64 && records.entries.get(expected).is_some(),
                    None => records.entries.get(expected) == Some(&appearance::Record::default()),
                },
                "retired ordinary record identity changed"
            );
        }
        if let Some(clear) = &self.clear {
            clear.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.execution.pending() == Some(destruction::Request::ClearAppearances),
                "ordinary clear lost destructor ownership"
            );
        }
        if self.state.model.is_none() {
            ensure!(
                graphic.ready.geometry.state.appearances.is_empty()
                    && graphic.ready.geometry.appearances.is_empty()
                    && graphic.ready.avatar.avatar.parts.is_empty()
                    && graphic
                        .ready
                        .bound
                        .skeleton
                        .skin
                        .animator
                        .layers()
                        .is_empty(),
                "destroyed ordinary model retains live storage"
            );
        }
        Ok(())
    }
    /// One original service (or one nested record clear). The host supplies
    /// actual focus/model notifications and commits this transaction on success.
    pub fn advance(
        &mut self,
        index: &Index,
        graphic: &mut Graphic,
        pool: &mut MeshPool,
        tick: i32,
        settings: Settings,
        mut notify: impl FnMut(destruction::Request) -> Result<()>,
    ) -> Result<bool> {
        self.validate(graphic)?;
        let request = self
            .execution
            .advance(&mut self.state)
            .map_err(anyhow::Error::msg)?;
        if self.state.auxiliary.is_empty() {
            graphic.ready.avatar.records.accessories.clear();
        }
        let Some(request) = request else {
            ensure!(
                self.execution.complete()
                    && self.state.model.is_none()
                    && graphic
                        .ready
                        .avatar
                        .records
                        .entries
                        .iter()
                        .all(|r| r == &appearance::Record::default()),
                "ordinary destructor retained record storage"
            );
            return Ok(true);
        };
        let r = &mut graphic.ready;
        match request {
            destruction::Request::Reset => {
                // The native reset writes all four flags and engaged=0. Those
                // aliases are dead after destruction; placement remains saved.
                graphic_reset::apply(
                    &mut r.bound,
                    &mut r.frame.placement,
                    &mut [0; 4],
                    &mut 0,
                    ordinary::clock(tick, r.frame.clock_mode),
                    settings,
                )
                .map_err(anyhow::Error::msg)?;
                r.appearance.dirty = r.frame.placement.dirty;
                r.frame.blend = None;
                self.state.layers = layers(graphic);
            }
            destruction::Request::ClearAppearances => {
                let clear = self
                    .clear
                    .get_or_insert_with(|| records::Update::new(records::Entry::Clear));
                let mut vectors = records::Vectors {
                    generic: vec![r.avatar.records.generic],
                    accessories: r
                        .avatar
                        .records
                        .accessories
                        .iter()
                        .copied()
                        .map(Some)
                        .collect(),
                };
                if let Some(next) = clear
                    .advance(&mut vectors, None)
                    .map_err(anyhow::Error::msg)?
                {
                    let records::Request::Clear(record) = next else {
                        anyhow::bail!("unexpected ordinary clear service");
                    };
                    r.geometry.apply_record(
                        index,
                        &mut r.avatar,
                        record,
                        appearance::Execution::clear(),
                        &mut r.bound,
                        pool,
                    )?;
                    clear
                        .acknowledge(&mut vectors, &next)
                        .map_err(anyhow::Error::msg)?;
                    return Ok(false);
                }
                self.clear = None;
            }
            destruction::Request::DetachLayer(id) => r
                .bound
                .skeleton
                .skin
                .animator
                .detach_for_destruction(LayerId(id))
                .map_err(anyhow::Error::msg)?,
            destruction::Request::DestroyModel(_) => {
                ensure!(
                    r.geometry.state.appearances.is_empty()
                        && r.geometry.appearances.is_empty()
                        && r.avatar.avatar.parts.is_empty(),
                    "ordinary model still has appearances"
                );
                r.bound
                    .skeleton
                    .skin
                    .animator
                    .release_model_layers()
                    .map_err(anyhow::Error::msg)?;
                graphic.materials = None;
                notify(request)?;
            }
            destruction::Request::ClearFocus => notify(request)?,
            destruction::Request::DestroyRecord { record, .. } => {
                let record = usize::try_from(record).context("ordinary record index overflow")?;
                r.geometry.apply_record(
                    index,
                    &mut r.avatar,
                    record,
                    appearance::Execution::clear(),
                    &mut r.bound,
                    pool,
                )?;
                r.avatar.records.entries[record] = Default::default();
            }
            destruction::Request::Storage(_) => (), // Rust owns these allocations.
        }
        self.execution
            .acknowledge(&mut self.state, request)
            .map_err(anyhow::Error::msg)?;
        self.validate(graphic)?;
        Ok(false)
    }
}
fn layers(graphic: &Graphic) -> [Option<u64>; 3] {
    let all = graphic
        .ready
        .bound
        .skeleton
        .skin
        .animator
        .destruction_layers();
    [all[0], all[1], all[2]].map(|id| id.map(|id| id.0))
}
