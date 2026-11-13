//! Owned43fc94 tail after the imported objects' ordered visits. Application
//! registry/view context remains explicit; no F6 or household tick is invented.
use super::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{Simulation, lot_selection, pose::portrait::view::Input};
use sims_vm::{
    ObjectId, World,
    commander::{
        Registry,
        house::{
            Fields, Receiver, Request as Listener,
            dispatch::{Dispatch, Progress},
        },
    },
    load_finalize::{Request, Tail},
    selection,
    selection_indicators::Entry,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    tail: Tail,
    enabled: u8,
    registry: Registry<Receiver>,
    fields: Fields,
    started: bool,
    created: Option<(ObjectId, Dispatch)>,
}
struct People<'a>(&'a World);
impl selection::View for People<'_> {
    fn person_order(&self) -> &[ObjectId] {
        &self.0.person.as_ref().unwrap().person_order
    }
    fn flags(&self, id: ObjectId) -> Result<u32, String> {
        self.0
            .objects
            .get(&id)
            .and_then(|o| o.native_flags)
            .ok_or("post-load person flags absent".into())
    }
    fn script_id(&self, id: ObjectId) -> Result<i16, String> {
        self.0
            .objects
            .get(&id)
            .and_then(|o| o.data.get(&11))
            .copied()
            .ok_or("post-load script ID absent".into())
    }
}
impl State {
    pub fn complete(&self) -> bool {
        self.tail.complete() && !self.started && self.created.is_none()
    }
    pub fn pending(&self) -> Option<Request> {
        self.tail.pending()
    }
    pub fn context(&self) -> (&Registry<Receiver>, &Fields) {
        (&self.registry, &self.fields)
    }
    pub fn validate(&self, sim: &Simulation) -> Result<()> {
        self.tail.validate().map_err(anyhow::Error::msg)?;
        self.registry.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            !self.tail.complete() || (!self.started && self.created.is_none()),
            "returned post-load tail retains child work"
        );
        if self.complete() {
            return Ok(());
        }
        ensure!(
            sim.vm.world.person.is_some(),
            "post-load person context absent"
        );
        if self.started {
            let e = sim
                .native_selection
                .as_ref()
                .context("post-load indicator child absent")?;
            ensure!(
                match self.tail.pending() {
                    Some(Request::Initialize) => e.is_initialization(),
                    Some(Request::Selection(id)) =>
                        e.direct_indicator() == Some((id, Entry::Selection)),
                    Some(Request::Star(id)) =>
                        e.direct_indicator() == Some((Some(id), Entry::Star)),
                    None => false,
                },
                "post-load indicator caller differs"
            );
        } else {
            ensure!(
                sim.native_selection.is_none(),
                "post-load overlaps another indicator owner"
            );
        }
        if let Some((id, d)) = &self.created {
            d.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.started
                    && d.root_message() == Some((0xe2, *id as u32))
                    && d.registry == self.registry
                    && d.fields == self.fields
                    && sim.pending_native_selection()
                        == Some((lot_selection::Request::CreationNotification(*id), true)),
                "post-load factory lost its E2 child"
            );
        } else {
            ensure!(
                !matches!(
                    sim.pending_native_selection(),
                    Some((lot_selection::Request::CreationNotification(_), true))
                ),
                "post-load creation callback lost its dispatcher"
            );
        }
        Ok(())
    }
}
impl Session {
    pub fn postload_pending(&self) -> bool {
        self.host
            .presentation
            .postload
            .as_ref()
            .is_some_and(|p| !p.complete())
    }
    /// Enter the native tail at its actual post-visit position, before F6.
    pub fn begin_postload(&mut self, registry: Registry<Receiver>, fields: Fields) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(
            self.options.native_postload && self.host.presentation.awaiting_house_open,
            "session was not loaded for its owned post-load tail"
        );
        ensure!(
            self.host.presentation.postload.is_none()
                && !self.house_notification_pending()
                && !self.view_refresh_pending()
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending(),
            "post-load overlaps pending work"
        );
        let state = State {
            tail: Tail::default(),
            enabled: 0,
            registry,
            fields,
            started: false,
            created: None,
        };
        state.validate(&self.simulation)?;
        self.host.presentation.postload = Some(state);
        Ok(())
    }
    pub fn advance_postload(&mut self, camera: Input, budget: usize) -> Result<bool> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(budget > 0, "post-load requires a positive budget");
        camera.build().map_err(anyhow::Error::msg)?;
        ensure!(
            !self.house_notification_pending() && !self.view_refresh_pending(),
            "post-load overlaps presentation work"
        );
        let mut state = self
            .host
            .presentation
            .postload
            .take()
            .context("post-load tail absent")?;
        let result = self.advance_postload_inner(&mut state, camera, budget);
        if let Some((_, child)) = &state.created {
            state.registry = child.registry.clone();
            state.fields = child.fields.clone();
        }
        self.host.presentation.postload = Some(state);
        result
    }
    fn advance_postload_inner(
        &mut self,
        state: &mut State,
        camera: Input,
        budget: usize,
    ) -> Result<bool> {
        state.validate(&self.simulation)?;
        if state.complete() {
            return Ok(true);
        }
        for _ in 0..budget {
            if let Some((id, d)) = &mut state.created {
                let selected = self
                    .simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?
                    .is_some();
                let progress = d
                    .advance(
                        &mut self.simulation.vm.world.globals,
                        selected,
                        1,
                        |_, _, _, _| {},
                    )
                    .map_err(anyhow::Error::msg)?;
                match progress {
                    Progress::Pending => (),
                    Progress::Service(call)
                        if matches!(call.request, Listener::WorldCreated { .. }) =>
                    {
                        let Listener::WorldCreated { value } = call.request else {
                            unreachable!()
                        };
                        sims_vm::family::created(
                            &mut self.simulation.vm.world,
                            &self.program,
                            value,
                        )
                        .map_err(anyhow::Error::msg)?;
                        d.acknowledge(&call).map_err(anyhow::Error::msg)?;
                    }
                    Progress::Complete(_) => {
                        self.simulation
                            .complete_indicator_created(*id)
                            .map_err(anyhow::Error::msg)?;
                    }
                    other => {
                        anyhow::bail!("post-load E2 requires original listener service: {other:?}")
                    }
                }
                state.registry = d.registry.clone();
                state.fields = d.fields.clone();
                if d.returned().is_some() {
                    state.created = None;
                }
            } else if state.started {
                let ordinary = self.host.ordinary.graphics.keys().copied().collect();
                self.simulation
                    .advance_native_selection(&self.program, &ordinary, 1)
                    .map_err(anyhow::Error::msg)?;
                if self.simulation.native_selection.is_none() {
                    state
                        .tail
                        .acknowledge(state.tail.pending().context("post-load callback absent")?)
                        .map_err(anyhow::Error::msg)?;
                    state.started = false;
                } else if let Some((lot_selection::Request::CreationNotification(id), false)) =
                    self.simulation.pending_native_selection()
                {
                    let child = Dispatch::begin(
                        state.registry.clone(),
                        state.fields.clone(),
                        0xe2,
                        id as u32,
                    )
                    .map_err(anyhow::Error::msg)?;
                    self.simulation
                        .start_indicator_created(id)
                        .map_err(anyhow::Error::msg)?;
                    state.created = Some((id, child));
                } else {
                    self.service_selection_assets(camera, 1)?;
                }
            } else {
                let request = state
                    .tail
                    .advance(&mut state.enabled, &People(&self.simulation.vm.world))
                    .map_err(anyhow::Error::msg)?;
                match request {
                    None => return Ok(true),
                    Some(Request::Initialize) => self
                        .simulation
                        .begin_native_indicator_initialization()
                        .map_err(anyhow::Error::msg)?,
                    Some(Request::Selection(id)) => self
                        .simulation
                        .begin_native_indicator_update(id, Entry::Selection, state.enabled != 0)
                        .map_err(anyhow::Error::msg)?,
                    Some(Request::Star(id)) => self
                        .simulation
                        .begin_native_indicator_update(Some(id), Entry::Star, state.enabled != 0)
                        .map_err(anyhow::Error::msg)?,
                }
                state.started = true;
            }
        }
        Ok(false)
    }
}
