//! Saved428041 completion with actual44123f removals and440b2d construction.
//! Process mode and terrain backend remain explicit host inputs.
use super::{Session, notifications};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{lot_selection, pose::portrait::view::Input};
use sims_vm::{
    Definition, ObjectId, World,
    commander::{
        Registry,
        house::{
            Fields, Receiver, Request as Listener,
            dispatch::{Dispatch, Progress as DispatchProgress},
        },
    },
    load_completion::{self, Completion, Inputs, Reply, Request},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub controller_suppressed: u32,
    pub mode: i16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    call: Completion,
    family: i16,
    manager_lot: i32,
    /// Native winning resource registration category, not an OBJD-kind guess.
    classes: BTreeMap<u32, (u16, u32)>,
    config: Option<Config>,
    registry: Option<Registry<Receiver>>,
    fields: Option<Fields>,
    child: Option<Request>,
    resource: Option<Definition>,
    created: Option<(ObjectId, Dispatch)>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    Pending,
    Complete,
    /// ColorSwatch/terrain renderer service. A headless host may explicitly
    /// report that the native terrain-renderer pointer is absent.
    Environment,
    Notification(notifications::Service),
}
struct Live<'a> {
    world: &'a mut World,
    state: &'a State,
}
impl load_completion::View for Live<'_> {
    fn people(&self) -> &[ObjectId] {
        &self.world.person.as_ref().unwrap().person_order
    }
    fn resource_flags(&self, id: ObjectId) -> Result<u32, String> {
        let d = self
            .world
            .objects
            .get(&id)
            .and_then(|o| o.definition.as_ref())
            .ok_or("cleanup object definition absent")?;
        let (chunk, flags) = self
            .state
            .classes
            .get(&d.guid)
            .ok_or("cleanup resource registration absent")?;
        if *chunk != d.object {
            return Err("cleanup native resource binding differs".into());
        }
        Ok(*flags)
    }
    fn person(&self, id: ObjectId, field: i16) -> Result<i16, String> {
        self.world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.get(&field))
            .copied()
            .ok_or("cleanup person field absent".into())
    }
    fn manager_lot(&self) -> i32 {
        self.state.manager_lot
    }
    fn controller_suppressed(&self) -> u32 {
        self.state.config.unwrap().controller_suppressed
    }
    fn mode(&self) -> i16 {
        self.state.config.unwrap().mode
    }
    fn controller_gate(&self) -> Result<i16, String> {
        self.world
            .globals
            .get(&26)
            .copied()
            .ok_or("cleanup controller gate absent".into())
    }
    fn family_id(&self) -> i32 {
        i32::from(self.state.family)
    }
    fn family_flags(&self) -> Result<u32, String> {
        self.world
            .person
            .as_ref()
            .unwrap()
            .family_metadata
            .get(&self.state.family)
            .map(|m| m.flags)
            .ok_or("cleanup retained family absent".into())
    }
    fn write_family_flags(&mut self, value: u32) -> Result<(), String> {
        self.world
            .person
            .as_mut()
            .unwrap()
            .family_metadata
            .get_mut(&self.state.family)
            .ok_or("cleanup retained family absent")?
            .flags = value;
        Ok(())
    }
    fn family_home(&self, family: i32) -> Result<Option<i16>, String> {
        Ok(i16::try_from(family).ok().and_then(|id| {
            self.world
                .person
                .as_ref()
                .unwrap()
                .family_homes
                .get(&id)
                .copied()
        }))
    }
}
impl State {
    pub(super) fn prepare(
        index: &sims_content::Index,
        house: &sims_content::saves::House,
        loaded: &sims_content::house_loading::FrozenLot,
    ) -> Result<Self> {
        let catalog = sims_content::object_catalog::Catalog::household(
            std::sync::Arc::new(index.clone()),
            &house.neighborhood_source,
            Default::default(),
        )?;
        let classes = catalog
            .definitions
            .guids
            .iter()
            .map(|(&guid, &i)| {
                let e = &catalog.definitions.entries[i];
                (guid, (e.key.chunk, e.flags))
            })
            .collect();
        let raw =
            sims_formats::iff::Iff::parse(index.resources[index.find(&house.source)?].read()?)?;
        let imported = u8::from(raw.all(b"EXPi").count() as u16 as i16 > 0);
        Ok(Self {
            call: Completion::new(Inputs {
                lot_argument: i32::from(house.number),
                reset_requested: loaded.family_reset_requested,
                imported,
                residents_before_visits: loaded.residents_before_visits.clone(),
            })
            .map_err(anyhow::Error::msg)?,
            family: loaded.selected_family,
            manager_lot: i32::from(house.number),
            classes,
            config: None,
            registry: None,
            fields: None,
            child: None,
            resource: None,
            created: None,
        })
    }
    pub fn pending(&self) -> Option<Request> {
        self.call.pending()
    }
    pub fn complete(&self) -> bool {
        self.call.complete() && self.child.is_none() && self.created.is_none()
    }
    pub fn inputs(&self) -> &Inputs {
        self.call.inputs()
    }
    pub(super) fn validate_source(&self, loaded: &Self) -> Result<()> {
        ensure!(
            self.classes == loaded.classes
                && self.family == loaded.family
                && self.manager_lot == loaded.manager_lot
                && self.call.inputs() == loaded.call.inputs(),
            "saved cleanup source bindings differ"
        );
        Ok(())
    }
    pub fn validate(&self, s: &Session) -> Result<()> {
        self.call.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            self.config.is_some() == self.registry.is_some()
                && self.config.is_some() == self.fields.is_some(),
            "cleanup context incomplete"
        );
        if let Some(r) = &self.registry {
            r.validate().map_err(anyhow::Error::msg)?;
        }
        ensure!(
            !self.call.complete() || self.complete(),
            "returned cleanup retains child"
        );
        if self.complete() {
            return Ok(());
        }
        if self.config.is_none() {
            ensure!(
                self.call.pending().is_none() && self.child.is_none() && self.created.is_none(),
                "unstarted cleanup retains callback"
            );
            return Ok(());
        }
        ensure!(
            s.host
                .presentation
                .postload
                .as_ref()
                .is_some_and(super::postload::State::complete),
            "cleanup overtook post-load"
        );
        if let Some(child) = self.child {
            ensure!(
                self.call.pending() == Some(child),
                "cleanup child lost its caller"
            );
            ensure!(
                match child {
                    Request::Delete(id) => s
                        .simulation
                        .synchronous_removal
                        .as_ref()
                        .is_some_and(|e| e.target() == id),
                    Request::CreateController =>
                        s.simulation
                            .native_selection
                            .as_ref()
                            .and_then(lot_selection::Execution::direct_factory)
                            == self.resource.as_ref()
                            && self.resource.is_some(),
                    Request::HouseOpen => s
                        .host
                        .presentation
                        .notifications
                        .as_ref()
                        .is_some_and(|n| n.dispatch.root_message() == Some((0xf6, 0))),
                    _ => false,
                },
                "cleanup retained service differs"
            );
        } else {
            ensure!(
                s.simulation.synchronous_removal.is_none()
                    && s.simulation.native_selection.is_none(),
                "cleanup orphan native child"
            );
        }
        if let Some((id, d)) = &self.created {
            d.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                self.child == Some(Request::CreateController)
                    && d.root_message() == Some((0xe2, *id as u32))
                    && Some(&d.registry) == self.registry.as_ref()
                    && Some(&d.fields) == self.fields.as_ref()
                    && s.simulation.pending_native_selection()
                        == Some((lot_selection::Request::CreationNotification(*id), true)),
                "cleanup E2 owner differs"
            );
        } else {
            ensure!(
                !matches!(
                    s.simulation.pending_native_selection(),
                    Some((lot_selection::Request::CreationNotification(_), true))
                ),
                "cleanup lost creation dispatcher"
            );
        }
        Ok(())
    }
}
impl Session {
    pub fn load_completion_pending(&self) -> bool {
        self.host
            .presentation
            .loading
            .as_ref()
            .is_some_and(|s| !s.complete())
    }
    pub fn load_completion_active(&self) -> bool {
        self.host
            .presentation
            .loading
            .as_ref()
            .is_some_and(|s| s.config.is_some() && !s.complete())
    }
    pub fn begin_load_completion(&mut self, config: Config) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        let postload = self
            .host
            .presentation
            .postload
            .as_ref()
            .context("post-load owner absent")?;
        ensure!(
            postload.complete()
                && !self.house_notification_pending()
                && !self.view_refresh_pending(),
            "post-load must return before cleanup"
        );
        let state = self
            .host
            .presentation
            .loading
            .as_mut()
            .context("load completion profile absent")?;
        ensure!(state.config.is_none(), "load completion already entered");
        let (registry, fields) = postload.context();
        state.registry = Some(registry.clone());
        state.fields = Some(fields.clone());
        state.config = Some(config);
        let registry = state.registry.clone().unwrap();
        let fields = state.fields.clone().unwrap();
        self.install_house_context(registry, fields)?;
        self.host
            .presentation
            .loading
            .as_ref()
            .unwrap()
            .validate(self)
    }
    pub fn acknowledge_absent_terrain_renderer(&mut self) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        let state = self
            .host
            .presentation
            .loading
            .as_mut()
            .context("load completion absent")?;
        ensure!(
            state.config.is_some() && state.child.is_none(),
            "cleanup environment is not pending"
        );
        state
            .call
            .acknowledge(Request::Environment, Reply::Done)
            .map_err(anyhow::Error::msg)
    }
    pub fn advance_load_completion(&mut self, camera: Input, budget: usize) -> Result<Progress> {
        ensure!(
            !self.camera_command_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(budget > 0, "load completion requires a positive budget");
        camera.build().map_err(anyhow::Error::msg)?;
        let mut state = self
            .host
            .presentation
            .loading
            .take()
            .context("load completion absent")?;
        let result = self.advance_load_completion_inner(&mut state, camera, budget);
        if let Some((_, d)) = &state.created {
            state.registry = Some(d.registry.clone());
            state.fields = Some(d.fields.clone());
        }
        self.host.presentation.loading = Some(state);
        result
    }
    fn advance_load_completion_inner(
        &mut self,
        state: &mut State,
        camera: Input,
        budget: usize,
    ) -> Result<Progress> {
        state.validate(self)?;
        ensure!(state.config.is_some(), "load completion has not begun");
        if state.complete() {
            return Ok(Progress::Complete);
        }
        for _ in 0..budget {
            if let Some((id, d)) = &mut state.created {
                let selected = self
                    .simulation
                    .native_selected_person()
                    .map_err(anyhow::Error::msg)?
                    .is_some();
                match d
                    .advance(
                        &mut self.simulation.vm.world.globals,
                        selected,
                        1,
                        |_, _, _, _| {},
                    )
                    .map_err(anyhow::Error::msg)?
                {
                    DispatchProgress::Pending => (),
                    DispatchProgress::Service(call)
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
                    DispatchProgress::Complete(_) => self
                        .simulation
                        .complete_indicator_created(*id)
                        .map_err(anyhow::Error::msg)?,
                    other => anyhow::bail!("cleanup E2 requires original service: {other:?}"),
                }
                state.registry = Some(d.registry.clone());
                state.fields = Some(d.fields.clone());
                if d.returned().is_some() {
                    state.created = None;
                    self.install_house_context(
                        state.registry.clone().unwrap(),
                        state.fields.clone().unwrap(),
                    )?;
                }
                continue;
            }
            if let Some(child) = state.child {
                let complete = match child {
                    Request::Delete(id) => {
                        if self.house_notification_pending() {
                            match self.advance_house_notification(camera, 1)? {
                                notifications::Progress::Pending => (),
                                notifications::Progress::Service(service) => {
                                    return Ok(Progress::Notification(service));
                                }
                                notifications::Progress::Complete(_) => {
                                    let d = &self
                                        .host
                                        .presentation
                                        .notifications
                                        .as_ref()
                                        .unwrap()
                                        .dispatch;
                                    state.registry = Some(d.registry.clone());
                                    state.fields = Some(d.fields.clone());
                                }
                            }
                            continue;
                        }
                        let mut step = sims_sim::Step {
                            stop: sims_sim::StepStop::Budget,
                            trace: vec![],
                            events: vec![],
                        };
                        let mut remaining = 1;
                        let done = self
                            .simulation
                            .drive_synchronous_removal(&self.program, &mut remaining, &mut step)
                            .map_err(anyhow::Error::msg)?;
                        self.service_selection_assets(camera, 1)?;
                        if !self.house_notification_pending() {
                            self.host.service(&mut self.simulation, &mut self.program)?;
                        }
                        if done {
                            ensure!(
                                self.simulation
                                    .take_synchronous_removal()
                                    .map_err(anyhow::Error::msg)?
                                    == id
                            );
                        }
                        done
                    }
                    Request::CreateController => {
                        let ordinary = self.host.ordinary.graphics.keys().copied().collect();
                        self.simulation
                            .advance_native_selection(&self.program, &ordinary, 1)
                            .map_err(anyhow::Error::msg)?;
                        if let Some((lot_selection::Request::CreationNotification(id), false)) =
                            self.simulation.pending_native_selection()
                        {
                            let dispatch = Dispatch::begin(
                                state.registry.clone().unwrap(),
                                state.fields.clone().unwrap(),
                                0xe2,
                                id as u32,
                            )
                            .map_err(anyhow::Error::msg)?;
                            self.simulation
                                .start_indicator_created(id)
                                .map_err(anyhow::Error::msg)?;
                            state.created = Some((id, dispatch));
                        } else {
                            self.service_selection_assets(camera, 1)?;
                        }
                        self.simulation.native_selection.is_none()
                    }
                    Request::HouseOpen => match self.advance_house_notification(camera, 1)? {
                        notifications::Progress::Pending => false,
                        notifications::Progress::Service(service) => {
                            return Ok(Progress::Notification(service));
                        }
                        notifications::Progress::Complete(_) => {
                            let d = &self
                                .host
                                .presentation
                                .notifications
                                .as_ref()
                                .unwrap()
                                .dispatch;
                            state.registry = Some(d.registry.clone());
                            state.fields = Some(d.fields.clone());
                            true
                        }
                    },
                    _ => unreachable!(),
                };
                if complete {
                    state
                        .call
                        .acknowledge(child, Reply::Done)
                        .map_err(anyhow::Error::msg)?;
                    state.child = None;
                }
                continue;
            }
            // Split borrowing without copying World or losing the saved caller.
            let mut call = state.call.clone();
            let result = call.advance(&mut Live {
                world: &mut self.simulation.vm.world,
                state,
            });
            state.call = call;
            let Some(request) = result.map_err(anyhow::Error::msg)? else {
                return Ok(Progress::Complete);
            };
            match request {
                Request::Delete(id) => {
                    self.simulation
                        .begin_synchronous_removal(&self.program, id)
                        .map_err(anyhow::Error::msg)?;
                    state.child = Some(request);
                }
                Request::FindController => {
                    let found = self.simulation.vm.world.objects.values().any(|o| {
                        o.definition
                            .as_ref()
                            .is_some_and(|d| d.guid == load_completion::CONTROLLER)
                    });
                    state
                        .call
                        .acknowledge(request, Reply::Found(found))
                        .map_err(anyhow::Error::msg)?;
                }
                Request::LookupController => {
                    if state.classes.contains_key(&load_completion::CONTROLLER) {
                        ensure!(
                            sims_content::assets::load_object_guid_if_present(
                                &self.index,
                                &mut self.program,
                                load_completion::CONTROLLER
                            )?,
                            "registered controller resource absent"
                        );
                        self.simulation
                            .architecture
                            .as_mut()
                            .unwrap()
                            .bind_footprints(&self.program)
                            .map_err(anyhow::Error::msg)?;
                        let mut definitions = self.program.modules.values().flat_map(|m| {
                            m.object_definition_words
                                .iter()
                                .filter_map(move |(&object, w)| {
                                    (w.len() > 20
                                        && (u32::from(w[14] as u16)
                                            | (u32::from(w[15] as u16) << 16))
                                            == load_completion::CONTROLLER)
                                        .then(|| Definition {
                                            source: m.source.clone(),
                                            object,
                                            guid: load_completion::CONTROLLER,
                                            slots: w[20] as u16,
                                        })
                                })
                        });
                        state.resource = definitions.next();
                        ensure!(
                            state.resource.is_some() && definitions.next().is_none(),
                            "controller definition ambiguous"
                        );
                    }
                    state
                        .call
                        .acknowledge(request, Reply::Found(state.resource.is_some()))
                        .map_err(anyhow::Error::msg)?;
                }
                Request::CreateController => {
                    self.simulation
                        .begin_native_object_factory(
                            &self.program,
                            state
                                .resource
                                .clone()
                                .context("controller resource absent")?,
                        )
                        .map_err(anyhow::Error::msg)?;
                    state.child = Some(request);
                }
                Request::RefreshVisitors => {
                    sims_vm::family::refresh_loaded_visitors(&mut self.simulation.vm.world)
                        .map_err(anyhow::Error::msg)?;
                    state
                        .call
                        .acknowledge(request, Reply::Done)
                        .map_err(anyhow::Error::msg)?;
                }
                Request::Environment => return Ok(Progress::Environment),
                Request::HouseOpen => {
                    self.begin_house_notification(
                        state.registry.clone().unwrap(),
                        state.fields.clone().unwrap(),
                        0xf6,
                        0,
                    )?;
                    state.child = Some(request);
                }
                Request::ReleaseSnapshot => state
                    .call
                    .acknowledge(request, Reply::Done)
                    .map_err(anyhow::Error::msg)?,
                other => anyhow::bail!(
                    "load completion requires original reset/import service: {other:?}"
                ),
            }
        }
        Ok(Progress::Pending)
    }
}
