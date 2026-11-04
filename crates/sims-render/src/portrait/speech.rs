//! Person headline image retrieval using 4e915d's cache policy, the mounted
//! resource catalog, and the existing mask-4 speech image generators. This
//! does not place or composite a balloon in the lot's scene.
use super::{
    cache::{self, Images, Settings},
    images::Raster,
    job::{Job, Progress as JobProgress},
    retrieval::{Progress, refresh_service},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::object_catalog::Catalog;
use sims_runtime::original_session::Session;
use sims_sim::Step;
use sims_vm::image_cache::{
    regeneration::Regeneration,
    speech::{Request, Response, Speech},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retrieval {
    tick: u64,
    operation: Speech,
    color: u32,
    regeneration: Option<Job>,
}

impl Retrieval {
    pub fn new(
        tick: u64,
        index: u32,
        zoom: i32,
        output: Option<u32>,
        allow_regeneration: bool,
        color: u32,
    ) -> Self {
        Self {
            tick,
            operation: Speech::new(index, zoom, output, allow_regeneration),
            color,
            regeneration: None,
        }
    }

    pub fn validate(&self, session: &Session, images: &Images) -> Result<()> {
        let cache = &session.simulation.vm.world.image_cache;
        ensure!(
            session.simulation.vm.tick == self.tick,
            "speech retrieval advanced the lot clock"
        );
        self.operation.validate(cache).map_err(anyhow::Error::msg)?;
        images.validate(cache)?;
        if let Some(job) = &self.regeneration {
            let Some(Request::Refresh { index, persist }) = self.operation.pending() else {
                anyhow::bail!("speech regeneration has no retrieval owner");
            };
            ensure!(
                job.binding() == (index, 4, i32::from(persist), None, None, self.color),
                "speech regeneration arguments differ from retrieval"
            );
            job.validate(session, images)?;
        } else {
            ensure!(
                session.simulation.portrait_person.is_none()
                    && session.simulation.synchronous_removal.is_none(),
                "unowned speech lifecycle operation"
            );
        }
        Ok(())
    }

    pub fn finish(self) -> Result<(bool, Option<u32>)> {
        ensure!(
            self.regeneration.is_none(),
            "speech regeneration still active"
        );
        self.operation.finish().map_err(anyhow::Error::msg)
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
            "speech retrieval color changed while suspended"
        );
        loop {
            if let Some(job) = &mut self.regeneration {
                match job.advance(host, images, settings, remaining, step)? {
                    JobProgress::Temporary(service) => return Ok(Progress::Temporary(service)),
                    JobProgress::Yielded => return Ok(Progress::Yielded),
                    JobProgress::Complete(_) => {
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
                        4,
                        i32::from(persist),
                        None,
                        None,
                        self.color,
                    )));
                }
                Some(Request::Load { index, chunk }) => {
                    let cache = &mut host.session.simulation.vm.world.image_cache;
                    let resource = cache
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
                    let mut owner = raster.map(|r| images.insert(cache, r)).transpose()?;
                    let result =
                        self.operation
                            .respond(cache, Response::Image(owner), &mut |e, s| {
                                images.event(e, s)
                            });
                    images.release(cache, &mut owner)?;
                    result.map_err(anyhow::Error::msg)?;
                }
            }
        }
    }
}
