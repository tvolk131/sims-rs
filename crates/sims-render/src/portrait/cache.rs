//! Owned pixels behind the native seven-image cache's handles. This services
//! only image requests; catalog lookup, person construction/removal and resource
//! persistence remain explicit services for the session driver.
use super::{
    images::{self, Raster, Software},
    resize::{Format, Image},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::dialog::portrait::{
    Layout,
    generators::{self, Backend, Render, Rendered},
};
use sims_vm::image_cache::{
    Event, State,
    regeneration::{Regeneration, Request, Response},
};
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Images {
    pixels: BTreeMap<u32, Raster>,
}
impl Images {
    /// Simulation-side invalidations already update native reference counts.
    /// Reclaim their dead owned buffers at the presenter boundary; never alter
    /// cache/VM metadata while doing so.
    pub fn collect(&mut self, cache: &State) -> Result<()> {
        ensure!(
            self.pixels
                .keys()
                .all(|id| *id > 0 && (*id as usize) <= cache.refs.len()),
            "portrait pixels reference an unknown image"
        );
        self.pixels
            .retain(|id, _| cache.refs[*id as usize - 1] != 0);
        self.validate(cache)
    }
    /// Every live cache handle must have pixels, including host-owned template
    /// wrappers. Destroyed handles remain in State::refs but own no pixels.
    pub fn validate(&self, cache: &State) -> Result<()> {
        cache.validate().map_err(anyhow::Error::msg)?;
        for (&id, raster) in &self.pixels {
            ensure!(
                id > 0 && cache.refs.get(id as usize - 1).is_some_and(|n| *n > 0),
                "portrait pixels have no live image owner"
            );
            raster.validate()?;
        }
        for (index, &refs) in cache.refs.iter().enumerate() {
            ensure!(
                refs == 0 || self.pixels.contains_key(&(index as u32 + 1)),
                "live portrait image has no pixels"
            );
        }
        Ok(())
    }
    pub fn get(&self, id: u32) -> Result<&Raster> {
        self.pixels.get(&id).context("portrait pixels absent")
    }
    /// Creates one host-owned wrapper. A service response borrows this handle;
    /// release this reference after the continuation acquires its own wrapper.
    pub fn insert(&mut self, cache: &mut State, raster: Raster) -> Result<u32> {
        raster.validate()?;
        ensure!(
            cache.refs.len() < 1_000_000,
            "portrait image capacity exceeded"
        );
        let id = cache.refs.len() as u32 + 1;
        ensure!(
            !self.pixels.contains_key(&id),
            "portrait image identity collision"
        );
        cache.refs.push(1);
        self.pixels.insert(id, raster);
        Ok(id)
    }
    /// Use at native cache ownership boundaries, including invalidation. The
    /// cache invokes Destroy after setting the reference count to zero.
    pub fn event(&mut self, event: Event, _: &State) {
        if let Event::Destroy(id) = event {
            self.pixels.remove(&id);
        }
    }
    pub fn release(&mut self, cache: &mut State, owner: &mut Option<u32>) -> Result<()> {
        cache
            .assign_image(owner, None, &mut |e, s| self.event(e, s))
            .map_err(anyhow::Error::msg)
    }
    pub fn advance(
        &mut self,
        cache: &mut State,
        operation: &mut Regeneration,
    ) -> Result<Option<Request>> {
        self.validate(cache)?;
        operation
            .advance(cache, &mut |e, s| self.event(e, s))
            .map_err(anyhow::Error::msg)
    }
    pub fn respond(
        &mut self,
        cache: &mut State,
        operation: &mut Regeneration,
        response: Response,
    ) -> Result<()> {
        operation
            .respond(cache, response, &mut |e, s| self.event(e, s))
            .map_err(anyhow::Error::msg)?;
        self.validate(cache)
    }
    fn publish(
        &mut self,
        cache: &mut State,
        operation: &mut Regeneration,
        output: Rendered<Raster>,
    ) -> Result<()> {
        let mut owner = output.image.map(|r| self.insert(cache, r)).transpose()?;
        // The borrowed host owner keeps the result alive while respond retains
        // it, then releases the old local. Even failed renders can return pixels.
        let result = self.respond(
            cache,
            operation,
            Response::Image {
                image: owner,
                success: output.success,
            },
        );
        self.release(cache, &mut owner)?;
        result
    }
    /// Completes one pending image service without advancing the simulation.
    /// False leaves catalog/lifecycle/persistence requests pending unchanged.
    pub fn service(
        &mut self,
        operation: &mut Regeneration,
        settings: &Settings,
        host: &mut impl Host,
    ) -> Result<bool> {
        self.collect(host.cache())?;
        let Some(request) = self.advance(host.cache_mut(), operation)? else {
            return Ok(false);
        };
        match request {
            Request::Clone { image } => self.publish(
                host.cache_mut(),
                operation,
                Rendered {
                    image: Some(self.get(image)?.clone()),
                    success: true,
                },
            )?,
            Request::Render {
                slot,
                person,
                input,
            } => {
                let initial = input.map(|id| self.get(id).cloned()).transpose()?;
                let mut backend =
                    Software::new(settings.format, settings.templates.clone(), initial, |r| {
                        host.draw(person, settings.format, r)
                    })?;
                let success = generators::generate(
                    slot,
                    Layout(settings.layout),
                    match settings.format {
                        Format::Rgb555 => 1,
                        Format::Rgb565 => 2,
                    },
                    u32::from(settings.key),
                    person,
                    &mut backend,
                )
                .map_err(anyhow::Error::msg)?;
                // The thumbnail edits the caller's existing wrapper. Do not
                // replace its identity with a new cache image just because our
                // software endpoint used local copies while executing it.
                let retained = input.filter(|_| backend.output() == Some(0));
                let output = backend.finish(success)?;
                self.collect(host.cache())?;
                if let Some(id) = retained {
                    self.pixels
                        .insert(id, output.image.context("retained portrait lost pixels")?);
                    self.respond(
                        host.cache_mut(),
                        operation,
                        Response::Image {
                            image: Some(id),
                            success,
                        },
                    )?;
                } else {
                    self.publish(host.cache_mut(), operation, output)?;
                }
            }
            Request::Key { image, color } => {
                self.pixels
                    .get_mut(&image)
                    .context("keyed portrait absent")?
                    .key = Some(color as u16);
                self.respond(host.cache_mut(), operation, Response::Done)?;
            }
            Request::Compose { faces, background } => {
                let output = if let Some(faces) = faces {
                    images::compose(
                        Layout(settings.layout),
                        self.get(faces)?,
                        self.get(background)?,
                        u32::from(settings.key),
                    )?
                } else {
                    anyhow::bail!("portrait composition requires a face image");
                };
                self.publish(host.cache_mut(), operation, output)?;
            }
            _ => return Ok(false),
        }
        self.validate(host.cache())?;
        Ok(true)
    }
}

/// Rendering may run appearance notifications against this SAME cache. Keeping
/// the host intact avoids moving it out of the VM during native preparation.
pub trait Host {
    fn cache(&self) -> &State;
    fn cache_mut(&mut self) -> &mut State;
    fn draw(&mut self, person: u32, format: Format, request: Render) -> Result<Rendered<Image>>;
}

pub struct Session<'a> {
    pub session: &'a mut sims_runtime::original_session::Session,
    pub view: sims_sim::pose::portrait::view::Input,
    pub source_key: Option<u16>,
}
impl Host for Session<'_> {
    fn cache(&self) -> &State {
        &self.session.simulation.vm.world.image_cache
    }
    fn cache_mut(&mut self) -> &mut State {
        &mut self.session.simulation.vm.world.image_cache
    }
    fn draw(&mut self, person: u32, format: Format, request: Render) -> Result<Rendered<Image>> {
        super::generate::head(
            self.session,
            sims_vm::ObjectId::try_from(person).context("portrait person ID out of range")?,
            self.view,
            format,
            self.source_key,
            request,
        )
    }
}

/// Explicit native global layout, display packing/key and mounted UI templates.
/// These inputs are persisted with a suspended image job by its presenter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub layout: [i32; 6],
    pub format: Format,
    pub key: u16,
    pub templates: BTreeMap<u16, Raster>,
}
