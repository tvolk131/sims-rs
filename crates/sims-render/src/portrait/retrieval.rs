//! Synchronous 4e92a8 retrieval joined to the mounted catalog and regeneration
//! job. The caller retains this owner across pauses and must not advance the
//! household scheduler until completion. Camera/resource construction services
//! remain explicit; the native primitive has not returned at these boundaries.
use super::{
    cache::{self, Images, Settings},
    images::{self, Raster},
    job::{Job, Progress as JobProgress},
    persistence,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::object_catalog::Catalog;
use sims_runtime::original_session::Session;
use sims_sim::{Step, portrait_person};
use sims_vm::image_cache::{
    portrait::{Portrait, Request, Response},
    regeneration::{Regeneration, Request as RefreshRequest, Response as RefreshResponse},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retrieval {
    tick: u64,
    operation: Portrait,
    thumbnail: Option<u32>,
    color: u32,
    regeneration: Option<Job>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Temporary(portrait_person::Service),
    Yielded,
    Complete(bool),
}

impl Retrieval {
    /// Output ownership transfers into this continuation. Template wrappers are
    /// borrowed from the presenter, whose owners must survive every pause.
    pub fn new(
        tick: u64,
        index: u32,
        output: Option<u32>,
        background: Option<u32>,
        thumbnail: Option<u32>,
        color: u32,
    ) -> Self {
        Self {
            tick,
            operation: Portrait::new(index, output, background),
            thumbnail,
            color,
            regeneration: None,
        }
    }

    pub fn validate(&self, session: &Session, images: &Images) -> Result<()> {
        let cache = &session.simulation.vm.world.image_cache;
        ensure!(
            session.simulation.vm.tick == self.tick,
            "portrait retrieval advanced the lot clock"
        );
        self.operation.validate(cache).map_err(anyhow::Error::msg)?;
        images.validate(cache)?;
        if let Some(id) = self.thumbnail {
            images.get(id)?;
        }
        if let Some(job) = &self.regeneration {
            let Some(Request::Refresh { index, persist }) = self.operation.pending() else {
                anyhow::bail!("portrait regeneration has no retrieval owner");
            };
            ensure!(
                job.binding()
                    == (
                        index,
                        1,
                        i32::from(persist),
                        self.operation.background(),
                        self.thumbnail,
                        self.color
                    ),
                "portrait regeneration arguments differ from retrieval"
            );
            job.validate(session, images)?;
        } else {
            ensure!(
                session.simulation.portrait_person.is_none()
                    && session.simulation.synchronous_removal.is_none(),
                "unowned portrait lifecycle operation"
            );
        }
        Ok(())
    }

    pub fn finish(self) -> Result<(bool, Option<u32>)> {
        ensure!(
            self.regeneration.is_none(),
            "portrait regeneration still active"
        );
        self.operation.finish().map_err(anyhow::Error::msg)
    }

    fn respond_image(
        &mut self,
        session: &mut Session,
        images: &mut Images,
        raster: Option<Raster>,
    ) -> Result<()> {
        let cache = &mut session.simulation.vm.world.image_cache;
        let mut owner = raster.map(|r| images.insert(cache, r)).transpose()?;
        let response = self
            .operation
            .respond(cache, Response::Image(owner), &mut |e, s| {
                images.event(e, s)
            });
        images.release(cache, &mut owner)?;
        response.map_err(anyhow::Error::msg)
    }

    pub fn advance(
        &mut self,
        host: &mut cache::Session<'_>,
        catalog: &mut Catalog<'_>,
        images: &mut Images,
        settings: &Settings,
        remaining: &mut usize,
        step: &mut Step,
    ) -> Result<Progress> {
        images.collect(&host.session.simulation.vm.world.image_cache)?;
        self.validate(host.session, images)?;
        ensure!(
            u32::from(settings.key) == self.color,
            "portrait retrieval color changed while suspended"
        );
        loop {
            if let Some(job) = &mut self.regeneration {
                match job.advance(host, images, settings, remaining, step)? {
                    JobProgress::Temporary(service) => return Ok(Progress::Temporary(service)),
                    JobProgress::Yielded => return Ok(Progress::Yielded),
                    JobProgress::Complete(_) => {
                        // 4e92a8 intentionally ignores regeneration's Boolean.
                        self.regeneration = None;
                        self.operation
                            .respond(
                                &mut host.session.simulation.vm.world.image_cache,
                                Response::Refreshed,
                                &mut |e, s| images.event(e, s),
                            )
                            .map_err(anyhow::Error::msg)?;
                    }
                    JobProgress::Service(request) => {
                        let reply = refresh_service(host.session, catalog, images, request)?;
                        job.respond(host.session, images, reply)?;
                    }
                }
                continue;
            }
            if let Some(returned) = self.operation.returned() {
                return Ok(Progress::Complete(returned));
            }
            if *remaining == 0 {
                return Ok(Progress::Yielded);
            }
            let request = self
                .operation
                .advance(
                    &mut host.session.simulation.vm.world.image_cache,
                    &mut |e, s| images.event(e, s),
                )
                .map_err(anyhow::Error::msg)?;
            match request {
                None => continue,
                Some(Request::Refresh { index, persist }) => {
                    self.regeneration = Some(Job::new(Regeneration::new(
                        index,
                        1,
                        i32::from(persist),
                        self.operation.background(),
                        self.thumbnail,
                        self.color,
                    )));
                }
                Some(Request::Load { index, chunk }) => {
                    let resource = host
                        .session
                        .simulation
                        .vm
                        .world
                        .image_cache
                        .resolve_resource(index as usize, |guid, _| catalog.portrait_resource(guid))
                        .map_err(anyhow::Error::msg)?;
                    let bytes = resource
                        .map(|r| catalog.portrait_chunk(r, *b"BMP_", chunk))
                        .transpose()
                        .map_err(anyhow::Error::msg)?
                        .flatten();
                    let raster = bytes
                        .as_deref()
                        .map(|b| Raster::from_bmp(b, settings.format))
                        .transpose()?;
                    self.respond_image(host.session, images, raster)?;
                }
                Some(Request::Compose { faces, background }) => {
                    let output = images::compose(
                        sims_content::dialog::portrait::Layout(settings.layout),
                        images.get(faces)?,
                        images.get(background)?,
                        self.color,
                    )?;
                    // As in 4e92a8, the resulting wrapper decides success.
                    self.respond_image(host.session, images, output.image)?;
                }
            }
        }
    }
}

pub(super) fn refresh_service(
    session: &mut Session,
    catalog: &mut Catalog<'_>,
    images: &Images,
    request: RefreshRequest,
) -> Result<RefreshResponse> {
    Ok(match request {
        RefreshRequest::Resource { guid } => RefreshResponse::Resource(
            catalog
                .portrait_resource(guid)
                .map_err(anyhow::Error::msg)?,
        ),
        RefreshRequest::Find { index } => {
            // 4e9608: first matching GUID in native person registry order.
            let world = &session.simulation.vm.world;
            let guid = world
                .image_cache
                .entries
                .get(index as usize)
                .map_or(0, |e| e.guid);
            let mut found = None;
            if guid != 0 {
                for id in &world
                    .person
                    .as_ref()
                    .context("portrait person registry absent")?
                    .person_order
                {
                    let definition = world
                        .objects
                        .get(id)
                        .and_then(|o| o.definition.as_ref())
                        .context("portrait registry definition absent")?;
                    if definition.guid == guid {
                        found = Some(u32::try_from(*id).context("invalid portrait person ID")?);
                        break;
                    }
                }
            }
            RefreshResponse::Person(found)
        }
        RefreshRequest::Eligible { resource } => RefreshResponse::Eligible(
            catalog
                .portrait_writable(resource)
                .map_err(anyhow::Error::msg)?,
        ),
        RefreshRequest::Persist {
            index,
            chunk,
            image,
        } => {
            let _ = persistence::persist(
                &mut session.simulation.vm.world.image_cache,
                catalog,
                images,
                index,
                chunk,
                image,
            )
            .map_err(anyhow::Error::msg)?;
            RefreshResponse::Done
        }
        RefreshRequest::Flush { resource } => {
            let file = catalog
                .portrait_file(resource)
                .map_err(anyhow::Error::msg)?;
            catalog.resources.flush(file).map_err(anyhow::Error::msg)?;
            RefreshResponse::Done
        }
        other => anyhow::bail!("portrait image/lifecycle request escaped its owner: {other:?}"),
    })
}
