//! Resumable native regeneration joined to real person construction, drawing
//! and synchronous removal. Catalog, camera and persistence are explicit host
//! requests; the job never manufactures a successful reply for missing work.
use super::cache::{self, Images, Settings};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_runtime::original_session::Session;
use sims_sim::{Step, portrait_person};
use sims_vm::image_cache::regeneration::{Regeneration, Request, Response};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Active {
    Create(u32),
    Remove(sims_vm::ObjectId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    operation: Regeneration,
    active: Option<Active>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Service(Request),
    Temporary(portrait_person::Service),
    Yielded,
    Complete(bool),
}
impl Job {
    pub fn binding(&self) -> (u32, u32, i32, Option<u32>, Option<u32>, u32) {
        self.operation.binding()
    }
    pub fn new(operation: Regeneration) -> Self {
        Self {
            operation,
            active: None,
        }
    }
    pub fn validate(&self, session: &Session, images: &Images) -> Result<()> {
        let s = &session.simulation;
        self.operation
            .validate(&s.vm.world.image_cache)
            .map_err(anyhow::Error::msg)?;
        images.validate(&s.vm.world.image_cache)?;
        let pending = self.operation.pending();
        ensure!(
            match self.active {
                None => s.portrait_person.is_none() && s.synchronous_removal.is_none(),
                Some(Active::Create(index)) =>
                    pending == Some(Request::Create { index })
                        && s.portrait_person
                            .as_ref()
                            .is_some_and(|e| e.index() == index)
                        && s.synchronous_removal.is_none(),
                Some(Active::Remove(person)) =>
                    pending
                        == Some(Request::Remove {
                            person: person as u32
                        })
                        && s.synchronous_removal
                            .as_ref()
                            .is_some_and(|e| e.target() == person)
                        && s.portrait_person.is_none(),
            },
            "portrait job lifecycle owner mismatch"
        );
        Ok(())
    }
    pub fn respond(
        &mut self,
        session: &mut Session,
        images: &mut Images,
        response: Response,
    ) -> Result<()> {
        self.validate(session, images)?;
        ensure!(
            self.active.is_none()
                && matches!(
                    self.operation.pending(),
                    Some(
                        Request::Resource { .. }
                            | Request::Find { .. }
                            | Request::Eligible { .. }
                            | Request::Persist { .. }
                            | Request::Flush { .. }
                    )
                ),
            "portrait service is owned by the image/lifecycle driver"
        );
        images.respond(
            &mut session.simulation.vm.world.image_cache,
            &mut self.operation,
            response,
        )
    }
    pub fn advance(
        &mut self,
        host: &mut cache::Session<'_>,
        images: &mut Images,
        settings: &Settings,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Progress> {
        images.collect(&host.session.simulation.vm.world.image_cache)?;
        self.validate(host.session, images)?;
        loop {
            if let Some(result) = self.operation.returned() {
                return Ok(Progress::Complete(result));
            }
            if *remaining == 0 {
                return Ok(Progress::Yielded);
            }
            let request = images.advance(
                &mut host.session.simulation.vm.world.image_cache,
                &mut self.operation,
            )?;
            let Some(request) = request else {
                continue;
            };
            let response = match request {
                Request::Create { index } => {
                    let s = &mut host.session.simulation;
                    if self.active.is_none() {
                        s.begin_temporary_portrait_person(index)
                            .map_err(anyhow::Error::msg)?;
                        self.active = Some(Active::Create(index));
                    }
                    match s
                        .drive_temporary_portrait_person(&host.session.program, remaining, step)
                        .map_err(anyhow::Error::msg)?
                    {
                        portrait_person::Progress::Service(service) => {
                            return Ok(Progress::Temporary(service));
                        }
                        portrait_person::Progress::Yielded => return Ok(Progress::Yielded),
                        portrait_person::Progress::Complete(_) => {
                            let person = s
                                .take_temporary_portrait_person()
                                .map_err(anyhow::Error::msg)?;
                            self.active = None;
                            Response::Person(person.map(|id| id as u32))
                        }
                    }
                }
                Request::Remove { person } => {
                    let id = sims_vm::ObjectId::try_from(person)
                        .context("portrait removal ID out of range")?;
                    let s = &mut host.session.simulation;
                    if self.active.is_none() {
                        s.begin_synchronous_removal(&host.session.program, id)
                            .map_err(anyhow::Error::msg)?;
                        self.active = Some(Active::Remove(id));
                    }
                    if !s
                        .drive_synchronous_removal(&host.session.program, remaining, step)
                        .map_err(anyhow::Error::msg)?
                    {
                        return Ok(Progress::Yielded);
                    }
                    ensure!(s.take_synchronous_removal().map_err(anyhow::Error::msg)? == id);
                    self.active = None;
                    Response::Done
                }
                Request::Clone { .. }
                | Request::Render { .. }
                | Request::Key { .. }
                | Request::Compose { .. } => {
                    ensure!(images.service(&mut self.operation, settings, host)?);
                    continue;
                }
                request => return Ok(Progress::Service(request)),
            };
            images.collect(&host.session.simulation.vm.world.image_cache)?;
            images.respond(
                &mut host.session.simulation.vm.world.image_cache,
                &mut self.operation,
                response,
            )?;
        }
    }
}
