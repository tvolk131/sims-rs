use super::*;
use sims_sim::{ServiceCut, service_observation::ObjectLifetime};
use sims_vm::Status;

#[derive(Clone)]
pub(in crate::original_viewer) struct FrameContext {
    pub epoch: Instant,
    pub millis: u64,
    pub camera_pointer: viewport_drag::Pointer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpeningCut {
    Animation,
    Creation,
    Placement,
}

pub(super) struct Opening {
    pub cut: OpeningCut,
    pub actor: Identity,
    pub package: Identity,
    pub serial: u32,
    pub source: String,
    pub contents: Vec<u32>,
    pub continuation: Value,
}

pub(super) enum Selection {
    Opening(Opening),
    Service(ServiceCut),
}

pub(super) struct Captured {
    pub cut: ServiceCut,
    pub boundary: Value,
    pub continuation: Value,
    pub checkpoint: Vec<u8>,
}

pub(super) struct Outcome {
    pub execution: Result<()>,
    pub captured: Option<Captured>,
}

pub(in crate::original_viewer) struct Request {
    pub frame: FrameContext,
    selection: Selection,
    captured: Option<Captured>,
}

fn registered(simulation: &sims_sim::Simulation, lifetime: ObjectLifetime) -> bool {
    simulation.vm.world.objects.contains_key(&lifetime.object)
        && simulation
            .vm
            .world
            .scheduler_storage
            .as_ref()
            .and_then(|storage| storage.handle(lifetime.object))
            .is_some()
        && simulation.vm.object_threads.get(&lifetime.object).copied() == lifetime.object_thread
        && lifetime
            .object_thread
            .is_none_or(|thread| simulation.vm.threads.contains_key(&thread))
}

impl Opening {
    fn boundary(
        &self,
        cut: ServiceCut,
        simulation: &sims_sim::Simulation,
    ) -> Result<Option<Value>> {
        for identity in [&self.actor, &self.package] {
            ensure!(
                simulation
                    .vm
                    .world
                    .scheduler_storage
                    .as_ref()
                    .and_then(|storage| storage.handle(identity.object))
                    == Some(identity.handle),
                "stale opening object identity: {identity:?}"
            );
        }
        let world = &simulation.vm.world;
        let package = world
            .objects
            .get(&self.package.object)
            .context("opening package absent")?;
        if package.attributes.get(7) != Some(&0)
            || !package
                .definition
                .as_ref()
                .is_some_and(|definition| definition.source == self.source)
            || !self.source.ends_with("::MagicStarterKit.iff")
            || !world
                .actions
                .as_ref()
                .and_then(|actions| actions.actors.get(&self.actor.object))
                .and_then(|actions| actions.active.as_ref())
                .is_some_and(|action| {
                    action.id == self.serial
                        && action.target == self.package.object
                        && action.routine == 4097
                })
        {
            return Ok(None);
        }
        let (thread, request, object) = match cut {
            ServiceCut::AwaitingAnimation {
                thread,
                request,
                object,
            } if self.cut == OpeningCut::Animation => (thread, request, object),
            ServiceCut::Creation {
                thread,
                request,
                object,
                ..
            } if self.cut == OpeningCut::Creation => (thread, request, object),
            ServiceCut::FindLocation {
                thread,
                request,
                object,
                ..
            } if self.cut == OpeningCut::Placement => (thread, request, object),
            _ => return Ok(None),
        };
        if simulation.vm.object_threads.get(&self.actor.object) != Some(&thread)
            || !registered(simulation, object)
        {
            return Ok(None);
        }
        let Some(owner) = simulation.vm.threads.get(&thread) else {
            return Ok(None);
        };
        let origin_matches = |origin: &sims_vm::Location, caller| {
            caller == self.actor.object && origin.source == self.source
        };
        let content = |object| {
            world
                .objects
                .get(&object)
                .and_then(|object| object.definition.as_ref())
                .is_some_and(|definition| self.contents.contains(&definition.guid))
        };
        let boundary = match (&owner.status, self.cut) {
            (Status::AwaitingAnimation(wait), OpeningCut::Animation)
                if wait.id == request
                    && origin_matches(&wait.origin, wait.caller)
                    && object.object == self.actor.object
                    && simulation
                        .characters
                        .get(&self.actor.object)
                        .is_some_and(|resident| {
                            resident.animation_yield.is_some_and(|(owner, pending, _)| {
                                owner == thread && pending == request
                            })
                        }) =>
            {
                Some(json!({"request":wait,
                    "resident_animation_yield":simulation.characters[&self.actor.object].animation_yield}))
            }
            (Status::AwaitingCreation(wait), OpeningCut::Creation)
                if wait.id == request
                    && origin_matches(&wait.origin, wait.caller)
                    && wait.created == Some(object.object)
                    && self
                        .contents
                        .contains(&u32::from_le_bytes(wait.operands[..4].try_into().unwrap()))
                    && world
                        .objects
                        .get(&object.object)
                        .and_then(|object| object.definition.as_ref())
                        .is_some_and(|definition| {
                            definition.guid
                                == u32::from_le_bytes(wait.operands[..4].try_into().unwrap())
                        }) =>
            {
                Some(json!({"request":wait,"object":object,
                    "creation":simulation.native_creations.get(&request)}))
            }
            (Status::AwaitingPlacement(wait), OpeningCut::Placement)
                if wait.id == request
                    && origin_matches(&wait.origin, wait.caller)
                    && wait.target == object.object
                    && content(wait.target) =>
            {
                simulation
                    .native_lot
                    .as_ref()
                    .and_then(|native| native.find_location.as_ref())
                    .filter(|retained| {
                        let (owner, pending) = retained.binding();
                        owner == thread && pending == wait
                    })
                    .map(|retained| json!({"request":wait,"find_location":retained}))
            }
            _ => None,
        };
        Ok(boundary)
    }
}

impl Request {
    pub(super) fn new(frame: FrameContext, selection: Selection) -> Self {
        Self {
            frame,
            selection,
            captured: None,
        }
    }

    pub(in crate::original_viewer) fn observe(
        &mut self,
        cut: ServiceCut,
        session: &Session,
        client: &DialogClient,
        device: Option<&CameraDevice>,
        local: &Local,
    ) -> Result<()> {
        if self.captured.is_some() {
            return Ok(());
        }
        let (boundary, continuation) = match &self.selection {
            Selection::Opening(selection) => {
                let Some(boundary) = selection.boundary(cut, &session.simulation)? else {
                    return Ok(());
                };
                (boundary, selection.continuation.clone())
            }
            Selection::Service(expected) if *expected == cut => {
                (json!({"service":cut}), Value::Null)
            }
            Selection::Service(_) => return Ok(()),
        };
        self.retain(cut, boundary, continuation, || {
            Ok(serde_json::to_vec(&capture_envelope(
                session, client, device, local, true,
            )?)?)
        })
    }

    fn retain(
        &mut self,
        cut: ServiceCut,
        boundary: Value,
        continuation: Value,
        serialize: impl FnOnce() -> Result<Vec<u8>>,
    ) -> Result<()> {
        if self.captured.is_none() {
            let checkpoint = serialize()?;
            self.captured = Some(Captured {
                cut,
                boundary,
                continuation,
                checkpoint,
            });
        }
        Ok(())
    }
}

impl Application {
    pub(super) fn capture_frame(&self) -> FrameContext {
        FrameContext {
            epoch: self.epoch,
            millis: self.millis,
            camera_pointer: self.camera_pointer.clone(),
        }
    }

    pub(super) fn execute_capturing(
        &mut self,
        command: &Command,
        limits: &Limits,
        request: Request,
        observer: &mut impl FnMut(&mut Self) -> Result<()>,
    ) -> Result<Outcome> {
        ensure!(
            self.viewer.service_capture.is_none(),
            "service capture request already installed"
        );
        self.viewer.service_capture = Some(request);
        let execution = self.execute_observed(command, limits, observer);
        let request = self
            .viewer
            .service_capture
            .take()
            .context("service capture request disappeared")?;
        Ok(Outcome {
            execution,
            captured: request.captured,
        })
    }
}

#[cfg(test)]
#[path = "service_capture/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "service_capture/integration_tests.rs"]
mod integration_tests;
