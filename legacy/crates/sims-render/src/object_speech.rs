//! Mounted bitmap and resource services for 4cc3b9. Object drawing remains an
//! explicit Generate request; cached original images need no generated proxy.
use crate::portrait::{
    cache::Images,
    images::{self, Raster},
    resize::Format,
};
use anyhow::{Context, Result, ensure};
use sims_content::{Index, object_catalog::Catalog};
use sims_vm::image_cache::{
    State,
    object_speech::{Definition, Graphic, Request, Response, Retrieval},
};

/// Complete one pending non-drawing service. False retains a Generate request
/// unchanged for the object renderer, or means the getter has no pending work.
/// Writes go through the catalog's owned overlay, never original game files.
pub fn service(
    index: &Index,
    catalog: &mut Catalog<'_>,
    images: &mut Images,
    state: &mut State,
    graphic: &mut Graphic,
    operation: &mut Retrieval,
    format: Format,
) -> Result<bool> {
    operation
        .validate(state, graphic)
        .map_err(anyhow::Error::msg)?;
    images.collect(state)?;
    let Some(request) = operation.pending() else {
        return Ok(false);
    };
    let mut image_owner = None;
    let response = match request {
        Request::Definition { resource } => {
            let entry = catalog
                .definitions
                .entries
                .get(resource.wrapping_sub(1) as usize)
                .context("object speech definition absent")?;
            ensure!(
                entry.data.len() == 216,
                "object speech definition is not normalized"
            );
            let word =
                |n: usize| i16::from_le_bytes(entry.data[n * 2..n * 2 + 2].try_into().unwrap());
            Response::Definition(Definition {
                group: word(10),
                base_graphic: word(4),
                icon_base: word(41),
            })
        }
        Request::Load { resource, chunk } => {
            let file = catalog
                .portrait_file(resource)
                .map_err(anyhow::Error::msg)?;
            let handle = catalog
                .resources
                .by_id(file, u32::from_be_bytes(*b"BMP_"), chunk as u32, 0)
                .map_err(anyhow::Error::msg)?;
            let raster = if handle == 0 {
                None
            } else {
                let decoded = Raster::from_bmp(
                    catalog.resources.data(handle).map_err(anyhow::Error::msg)?,
                    format,
                );
                catalog
                    .resources
                    .release_chunk(file, handle)
                    .map_err(anyhow::Error::msg)?;
                Some(decoded?)
            };
            image_owner = raster.map(|r| images.insert(state, r)).transpose()?;
            Response::Image(image_owner)
        }
        Request::Template { chunk } => {
            image_owner = images::template(index, chunk, format)?
                .map(|r| images.insert(state, r))
                .transpose()?;
            Response::Image(image_owner)
        }
        Request::Generate { .. } => return Ok(false),
        Request::Encode { image } => {
            let bytes = images.get(image)?.bmp()?;
            Response::Buffer(Some(
                catalog
                    .resources
                    .allocate(bytes)
                    .map_err(anyhow::Error::msg)?,
            ))
        }
        Request::Add {
            resource,
            chunk,
            buffer,
        } => {
            let file = catalog
                .portrait_file(resource)
                .map_err(anyhow::Error::msg)?;
            catalog
                .resources
                .add(
                    file,
                    sims_sim::object_catalog::composite::Chunk {
                        handle: buffer,
                        tag: u32::from_be_bytes(*b"BMP_"),
                        id: chunk as u32,
                        name: b"Speech",
                        variant: 0,
                        converted: 1,
                    },
                )
                .map_err(anyhow::Error::msg)?;
            Response::Done
        }
        Request::Error { resource } => {
            let file = catalog
                .portrait_file(resource)
                .map_err(anyhow::Error::msg)?;
            let _ = catalog.resources.error(file).map_err(anyhow::Error::msg)?;
            Response::Done
        }
        Request::Flush { resource } => {
            let file = catalog
                .portrait_file(resource)
                .map_err(anyhow::Error::msg)?;
            catalog.resources.flush(file).map_err(anyhow::Error::msg)?;
            Response::Done
        }
        Request::Release { resource, buffer } => {
            let file = catalog
                .portrait_file(resource)
                .map_err(anyhow::Error::msg)?;
            catalog
                .resources
                .release_chunk(file, buffer)
                .map_err(anyhow::Error::msg)?;
            Response::Done
        }
    };
    let result = operation.respond(state, graphic, response, &mut |e, s| images.event(e, s));
    images.release(state, &mut image_owner)?;
    result.map_err(anyhow::Error::msg)?;
    images.validate(state)?;
    Ok(true)
}
