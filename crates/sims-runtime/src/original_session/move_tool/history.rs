//!Original Undoable composition inside487105/487191. Engine services without
//!a live binding remain suspended; a frontend cannot manufacture their return.
use super::{History, Service};
use crate::original_session::Session;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_sim::{
    edit_history::{
        self, command, manager,
        object::{self, driver},
        snapshot,
    },
    object_catalog::move_tool::cleanup,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Execution {
    call: edit_history::tool::Call,
    body: Option<driver::Execution>,
    notice: bool,
    retired: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pickup: Option<driver::ServiceCaller>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    placement: Option<driver::ServiceCaller>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget: Option<driver::BudgetCaller>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    factory: Option<Factory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deletion: Option<Deletion>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Factory {
    caller: driver::ServiceCaller,
    definition: sims_vm::Definition,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Deletion {
    caller: driver::ServiceCaller,
    id: sims_vm::ObjectId,
    handle: u32,
}
pub(super) enum Step {
    Pending(usize),
    Complete,
    Service(Service),
}
impl Execution {
    pub fn new(request: cleanup::Request) -> Result<Option<Self>> {
        Ok(edit_history::tool::Call::new(request)
            .map_err(anyhow::Error::msg)?
            .map(|call| Self {
                call,
                body: None,
                notice: false,
                retired: None,
                pickup: None,
                placement: None,
                budget: None,
                factory: None,
                deletion: None,
            }))
    }
    pub fn request(&self) -> cleanup::Request {
        self.call.request()
    }
    pub fn retired_payload(&self) -> Option<u32> {
        self.retired
    }
    pub fn owns_notification(&self) -> bool {
        self.notice || self.budget.is_some() || self.factory.is_some() || self.deletion.is_some()
    }
    pub fn owns_factory(&self) -> bool {
        self.factory.is_some()
    }
    pub fn owns_deletion(&self) -> bool {
        self.deletion.is_some()
    }
    pub(super) fn removal_target(&self) -> Option<sims_vm::ObjectId> {
        self.deletion.as_ref().map(|d| d.id)
    }
    pub fn owns_pickup(&self) -> bool {
        self.pickup.is_some()
    }
    pub fn owns_placement(&self) -> bool {
        self.placement.is_some()
    }
    pub fn budget_caller(&self) -> Option<driver::BudgetCaller> {
        self.budget
    }
    pub fn validate(&self, session: &Session, history: &History, registry: u32) -> Result<()> {
        self.call.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            [
                self.pickup.is_some(),
                self.placement.is_some(),
                self.budget.is_some(),
                self.factory.is_some(),
                self.deletion.is_some(),
                self.notice
            ]
            .into_iter()
            .filter(|b| *b)
            .count()
                <= 1,
            "history owns concurrent engine children"
        );
        ensure!(
            !(self.notice && self.body.is_some()),
            "history owns two concurrent children"
        );
        if let Some(body) = &self.body {
            body.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                body.root().map(manager::Service::Virtual) == self.call.service(),
                "history virtual child differs from caller"
            );
        }
        if let Some(caller) = self.pickup {
            ensure!(
                self.body
                    .as_ref()
                    .and_then(driver::Execution::service_caller)
                    == Some(caller)
                    && session
                        .simulation
                        .object_pickup
                        .as_ref()
                        .and_then(|p| p.command_caller())
                        == Some(caller),
                "history pickup lost its exact object-command caller"
            );
        }
        if let Some(caller) = self.placement {
            ensure!(
                self.body
                    .as_ref()
                    .and_then(driver::Execution::service_caller)
                    == Some(caller)
                    && session
                        .simulation
                        .object_placement
                        .as_ref()
                        .map(|e| e.caller())
                        == Some(caller),
                "history placement lost its exact object-command caller"
            );
        }
        if let Some(caller) = self.budget {
            ensure!(
                self.body
                    .as_ref()
                    .and_then(driver::Execution::budget_caller)
                    == Some(caller)
                    && session
                        .host
                        .presentation
                        .object_budget
                        .as_ref()
                        .map(|e| e.caller())
                        == Some(caller),
                "history budget lost its exact object-command caller"
            );
        }
        if let Some(f) = &self.factory {
            let driver::Service::Restore(snapshot::lifecycle::Request::Create {
                manager,
                definition: Some(definition),
            }) = f.caller.service
            else {
                anyhow::bail!("history factory has no concrete master");
            };
            ensure!(
                self.body
                    .as_ref()
                    .and_then(driver::Execution::service_caller)
                    == Some(f.caller)
                    && manager == registry
                    && history.definitions.get(&definition) == Some(&f.definition)
                    && session
                        .simulation
                        .native_selection
                        .as_ref()
                        .and_then(sims_sim::lot_selection::Execution::direct_factory)
                        == Some(&f.definition),
                "history factory lost its exact object-command caller"
            );
            if session.house_notification_pending() {
                let n = session
                    .host
                    .presentation
                    .notifications
                    .as_ref()
                    .context("history factory notification absent")?;
                ensure!(
                    n.direct_factory_object().is_some(),
                    "history factory owns an unrelated notification"
                );
                n.validate(&session.host.presentation, &session.simulation)?;
            }
        }
        if let Some(d) = &self.deletion {
            let storage = session
                .simulation
                .vm
                .world
                .scheduler_storage
                .as_ref()
                .context("history deletion registry absent")?;
            ensure!(
                self.body
                    .as_ref()
                    .and_then(driver::Execution::service_caller)
                    == Some(d.caller)
                    && d.caller.service
                        == driver::Service::Delete(snapshot::Delete { script_id: d.id })
                    && session
                        .simulation
                        .synchronous_removal
                        .as_ref()
                        .is_some_and(|r| r.target() == d.id)
                    && (storage.handle(d.id) == Some(d.handle)
                        || storage.handle(d.id).is_none() && storage.retired_handle(d.handle)),
                "history deletion lost its exact object-command caller or lifetime"
            );
        }
        if self.notice {
            ensure!(
                self.call.service() == Some(manager::Service::Notify),
                "history notice lost caller"
            );
            let n = session
                .host
                .presentation
                .notifications
                .as_ref()
                .context("history notice absent")?;
            ensure!(
                n.dispatch.root_message() == Some((0xee, 0)),
                "history notice differs"
            );
            n.validate(&session.host.presentation, &session.simulation)?;
        }
        if let Some(id) = self.retired {
            ensure!(
                self.call.request() == cleanup::Request::ReleasePayload(id)
                    && self.call.service().is_none()
                    && self.body.is_none(),
                "history lost released payload owner"
            );
        }
        Ok(())
    }
    pub fn advance(&mut self, s: &mut Session, h: &mut History, registry: u32) -> Result<Step> {
        if self.factory.is_some() || self.deletion.is_some() {
            if s.house_notification_pending() {
                let input = s.retained_camera_input()?;
                match s.advance_house_notification(input, 1)? {
                    crate::original_session::notifications::Progress::Pending => {
                        return Ok(Step::Pending(0));
                    }
                    crate::original_session::notifications::Progress::Service(r) => {
                        return Ok(Step::Service(Service::Notification(r)));
                    }
                    crate::original_session::notifications::Progress::Complete(_) => (),
                }
            }
            if let Some(f) = &self.factory {
                let ordinary = s.host.ordinary.graphics.keys().copied().collect();
                let (step, result) = s
                    .simulation
                    .advance_native_object_factory(&s.program, &ordinary, 1)
                    .map_err(anyhow::Error::msg)?;
                if let Some(id) = result {
                    //48c053 compares the actual return and rereads its reference.
                    //440b2d does not grant this caller permission to rebind it.
                    self.body
                        .as_mut()
                        .context("history factory caller absent")?
                        .acknowledge(
                            f.caller.service,
                            driver::Reply::Restore(snapshot::lifecycle::Reply::Created(
                                id.unwrap_or(0),
                            )),
                        )
                        .map_err(anyhow::Error::msg)?;
                    self.factory = None;
                } else if let Some((
                    sims_sim::lot_selection::Request::CreationNotification(id),
                    false,
                )) = s.simulation.pending_native_selection()
                {
                    s.begin_factory_broadcast(id)?;
                } else {
                    s.service_selection_assets(s.retained_camera_input()?, 1)?;
                }
                return Ok(Step::Pending(step.trace.len()));
            }
            let d = self.deletion.as_ref().unwrap();
            let mut step = sims_sim::Step {
                stop: sims_sim::StepStop::Budget,
                trace: vec![],
                events: vec![],
            };
            let done = s
                .simulation
                .drive_synchronous_removal(&s.program, &mut 1, &mut step)
                .map_err(anyhow::Error::msg)?;
            s.move_tool_assets()?;
            if done {
                ensure!(
                    s.simulation
                        .take_synchronous_removal()
                        .map_err(anyhow::Error::msg)?
                        == d.id,
                    "history deletion returned another object"
                );
                self.body
                    .as_mut()
                    .context("history deletion caller absent")?
                    .acknowledge(d.caller.service, driver::Reply::Done)
                    .map_err(anyhow::Error::msg)?;
                self.deletion = None;
            }
            return Ok(Step::Pending(step.trace.len()));
        }
        if self.placement.is_some() {
            let step = s
                .simulation
                .advance_object_command_placement(&s.program, 1)
                .map_err(anyhow::Error::msg)?;
            s.move_tool_assets()?;
            if s.simulation
                .object_placement
                .as_ref()
                .context("history placement absent")?
                .complete()
            {
                s.simulation
                    .finish_object_command_placement(
                        &s.program,
                        self.body
                            .as_mut()
                            .context("history placement caller absent")?,
                    )
                    .map_err(anyhow::Error::msg)?;
                self.placement = None;
            }
            return Ok(Step::Pending(step.trace.len()));
        }
        if self.budget.is_some() {
            if s.host
                .presentation
                .object_budget
                .as_ref()
                .context("history budget absent")?
                .notification_pending()
            {
                let input = s.retained_camera_input()?;
                match s.advance_house_notification(input, 1)? {
                    crate::original_session::notifications::Progress::Pending => {
                        return Ok(Step::Pending(0));
                    }
                    crate::original_session::notifications::Progress::Service(r) => {
                        return Ok(Step::Service(Service::Notification(r)));
                    }
                    crate::original_session::notifications::Progress::Complete(_) => (),
                }
            }
            s.finish_object_budget(self.body.as_mut().context("history budget caller absent")?)?;
            self.budget = None;
            return Ok(Step::Pending(0));
        }
        if self.pickup.is_some() {
            let step = s
                .simulation
                .advance_object_pickup(&s.program, 1)
                .map_err(anyhow::Error::msg)?;
            s.move_tool_assets()?;
            if s.simulation
                .object_pickup
                .as_ref()
                .context("history pickup absent")?
                .complete()
            {
                s.simulation
                    .finish_object_command_pickup(
                        &s.program,
                        self.body.as_mut().context("history pickup caller absent")?,
                    )
                    .map_err(anyhow::Error::msg)?;
                self.pickup = None;
            }
            return Ok(Step::Pending(step.trace.len()));
        }
        if self.notice {
            let input = s.retained_camera_input()?;
            return match s.advance_house_notification(input, 1)? {
                crate::original_session::notifications::Progress::Pending => Ok(Step::Pending(0)),
                crate::original_session::notifications::Progress::Service(r) => {
                    Ok(Step::Service(Service::Notification(r)))
                }
                crate::original_session::notifications::Progress::Complete(_) => {
                    self.call
                        .acknowledge(manager::Service::Notify, 0)
                        .map_err(anyhow::Error::msg)?;
                    self.notice = false;
                    Ok(Step::Pending(0))
                }
            };
        }
        if let Some(body) = &mut self.body {
            let request = body.root().context("history virtual root absent")?;
            let definitions = &h.definitions;
            let definition = |d: &sims_vm::Definition| {
                let mut matches = definitions
                    .iter()
                    .filter_map(|(&id, value)| (value == d).then_some(id));
                let id = matches
                    .next()
                    .ok_or("history snapshot definition was not retained")?;
                if matches.next().is_some() {
                    return Err(
                        "history snapshot definition has ambiguous retained identities".into(),
                    );
                }
                Ok(id)
            };
            let mut view = Live {
                inner: snapshot::live::View {
                    world: &mut s.simulation.vm.world,
                    program: &s.program,
                    links: &s
                        .simulation
                        .native_lot
                        .as_ref()
                        .context("history lot links absent")?
                        .placement
                        .links,
                    definition: &definition,
                },
                registry,
            };
            return match body
                .advance(
                    &mut h.store,
                    &mut h.objects,
                    &mut h.references,
                    &mut h.budgets,
                    &mut view,
                    1,
                )
                .map_err(anyhow::Error::msg)?
            {
                driver::Step::Budget => Ok(Step::Pending(0)),
                driver::Step::Service(r) => {
                    use snapshot::{lifecycle, placement};
                    match r {
                        driver::Service::Restore(lifecycle::Request::Master {
                            manager,
                            definition,
                        }) if h.catalog.is_some() => {
                            ensure!(manager == registry, "history master lookup manager differs");
                            let master =
                                sims_sim::object_catalog::move_tool::selected::lookup_master(
                                    h.catalog.as_ref().unwrap(),
                                    definition,
                                )
                                .map_err(anyhow::Error::msg)?;
                            body.acknowledge(
                                r,
                                driver::Reply::Restore(lifecycle::Reply::Master(master)),
                            )
                            .map_err(anyhow::Error::msg)?;
                            Ok(Step::Pending(0))
                        }
                        driver::Service::Placement(placement::Request::RemoveFromLot(_))
                        | driver::Service::Restore(lifecycle::Request::Placement(
                            placement::Request::RemoveFromLot(_),
                        ))
                        | driver::Service::Restore(lifecycle::Request::Remove(_))
                        | driver::Service::Remove(_) => {
                            s.simulation
                                .begin_object_command_pickup(&s.program, body)
                                .map_err(anyhow::Error::msg)?;
                            self.pickup = body.service_caller();
                            Ok(Step::Pending(0))
                        }
                        driver::Service::Placement(_)
                        | driver::Service::Restore(lifecycle::Request::Placement(_)) => {
                            s.simulation
                                .begin_object_command_placement(&s.program, body, h.placement_cheat)
                                .map_err(anyhow::Error::msg)?;
                            self.placement = body.service_caller();
                            Ok(Step::Pending(0))
                        }
                        driver::Service::Budget(_) => {
                            s.begin_object_budget(body)?;
                            self.budget = body.budget_caller();
                            Ok(Step::Pending(0))
                        }
                        driver::Service::Restore(lifecycle::Request::Create {
                            manager,
                            definition,
                        }) => {
                            ensure!(manager == registry, "history factory manager differs");
                            let definition = h
                                .definitions
                                .get(
                                    &definition
                                        .context("native440b2d cannot dereference a null master")?,
                                )
                                .context("history factory definition was not retained")?
                                .clone();
                            sims_content::assets::load_object_guids(
                                &s.index,
                                &mut s.program,
                                &[definition.guid],
                            )?;
                            s.simulation
                                .architecture
                                .as_mut()
                                .context("history factory architecture absent")?
                                .bind_footprints(&s.program)
                                .map_err(anyhow::Error::msg)?;
                            s.simulation
                                .begin_native_object_factory(&s.program, definition.clone())
                                .map_err(anyhow::Error::msg)?;
                            self.factory = Some(Factory {
                                caller: body
                                    .service_caller()
                                    .context("history factory caller absent")?,
                                definition,
                            });
                            Ok(Step::Pending(0))
                        }
                        driver::Service::Delete(request) => {
                            let handle = s
                                .simulation
                                .vm
                                .world
                                .scheduler_storage
                                .as_ref()
                                .context("history deletion registry absent")?
                                .handle(request.script_id);
                            if let Some(handle) = handle {
                                s.simulation
                                    .begin_synchronous_removal(&s.program, request.script_id)
                                    .map_err(anyhow::Error::msg)?;
                                self.deletion = Some(Deletion {
                                    caller: body
                                        .service_caller()
                                        .context("history deletion caller absent")?,
                                    id: request.script_id,
                                    handle,
                                });
                            } else {
                                //44123f returns immediately when40743b finds no object.
                                body.acknowledge(r, driver::Reply::Done)
                                    .map_err(anyhow::Error::msg)?;
                            }
                            Ok(Step::Pending(0))
                        }
                        _ => Ok(Step::Service(Service::Object(r))),
                    }
                }
                driver::Step::Complete(value) => {
                    self.call
                        .acknowledge(manager::Service::Virtual(request), value)
                        .map_err(anyhow::Error::msg)?;
                    if request.method == command::Method::Destroy
                        && self.call.request() == cleanup::Request::ReleasePayload(request.command)
                    {
                        ensure!(
                            !h.store.commands.contains_key(&request.command)
                                && !h.objects.0.contains_key(&request.command),
                            "payload destructor returned with live storage"
                        );
                        self.retired = Some(request.command);
                    }
                    self.body = None;
                    Ok(Step::Pending(0))
                }
            };
        }
        match self
            .call
            .advance(&mut h.store)
            .map_err(anyhow::Error::msg)?
        {
            Some(manager::Service::Virtual(r)) => {
                if h.objects
                    .0
                    .get(&r.command)
                    .is_some_and(|o| o.class == object::Class::Object)
                {
                    self.body = Some(driver::Execution::new(r).map_err(anyhow::Error::msg)?);
                    Ok(Step::Pending(0))
                } else {
                    Ok(Step::Service(Service::History(manager::Service::Virtual(
                        r,
                    ))))
                }
            }
            Some(manager::Service::Notify) => {
                s.check_house_notification(0xee)?;
                let n = s
                    .host
                    .presentation
                    .notifications
                    .as_ref()
                    .context("history requires registered house listeners")?;
                s.install_house_notification(
                    n.dispatch.registry.clone(),
                    n.dispatch.fields.clone(),
                    0xee,
                    0,
                )?;
                self.notice = true;
                Ok(Step::Pending(0))
            }
            None => {
                ensure!(self.call.complete(), "history did not return");
                Ok(Step::Complete)
            }
        }
    }
}
struct Live<'a> {
    inner: snapshot::live::View<'a>,
    registry: u32,
}
impl snapshot::Registry for Live<'_> {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
        self.inner.lookup(id)
    }
    fn on_lot(&mut self, h: u32) -> Result<bool, String> {
        self.inner.on_lot(h)
    }
}
impl snapshot::CaptureView for Live<'_> {
    fn normalized_master(&self, h: u32) -> Result<Option<u32>, String> {
        self.inner.normalized_master(h)
    }
    fn script_id(&self, h: u32) -> Result<i16, String> {
        self.inner.script_id(h)
    }
    fn capture(&mut self, h: u32) -> Result<snapshot::Position, String> {
        self.inner.capture(h)
    }
    fn definition(&self, h: u32) -> Result<u32, String> {
        self.inner.definition(h)
    }
    fn facing(&self, h: u32) -> Result<i16, String> {
        snapshot::CaptureView::facing(&self.inner, h)
    }
}
impl snapshot::placement::View for Live<'_> {
    fn facing(&self, h: u32) -> Result<i16, String> {
        snapshot::placement::View::facing(&self.inner, h)
    }
}
impl snapshot::lifecycle::View for Live<'_> {
    fn manager(&self) -> Result<u32, String> {
        Ok(self.registry)
    }
}
