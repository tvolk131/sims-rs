//! Software draw service for the original portrait generators. The caller
//! supplies the actual view and source-surface key state. The native generator
//! supplies angles, clear color, crop and target size, without demo defaults.
use super::{
    Drawable, Surface,
    resize::{Format, Image},
};
use anyhow::{Context, Result};
use sims_content::dialog::portrait::generators::{Render, Rendered};
use sims_runtime::original_session::Session;
use sims_sim::pose::{
    model::Matrix,
    portrait::{Camera, view::Input},
};

pub fn head(
    session: &mut Session,
    person: sims_vm::ObjectId,
    view: Input,
    format: Format,
    source_key: Option<u16>,
    request: Render,
) -> Result<Rendered<Image>> {
    let selected =
        session
            .host
            .prepare_portrait(&mut session.simulation, &session.program, person)?;
    if selected.is_empty() {
        return Ok(Rendered {
            success: false,
            image: None,
        });
    }
    let view_state = view.build().map_err(anyhow::Error::msg)?;
    let camera = Camera {
        inverse: view_state.inverse,
        rect: [0, 0, 200, 200],
        zoom: view.zoom,
        rotation: view.rotation as i32,
        angles: request.angles,
    };
    let transform = camera.transform().map_err(anyhow::Error::msg)?.matrix;
    let resident = session
        .simulation
        .characters
        .get(&person)
        .context("portrait resident absent")?;
    let matrix = Matrix {
        values: resident.character.matrix,
        kind: resident.character.model.bounds_transform.kind,
    };
    let drawables = selected
        .iter()
        .map(|&(binding, _)| {
            (
                binding,
                Drawable {
                    matrix,
                    mode: 0,
                    cached: false,
                },
            )
        })
        .collect();
    let [r, g, b] = format.decode(request.background as u16);
    let mut surface =
        Surface::for_portrait(drawables, view_state, [r, g, b, 255]).map_err(anyhow::Error::msg)?;
    let assets = session
        .host
        .characters
        .get_mut(&person)
        .context("portrait assets absent")?;
    let outcome = sims_content::character_portrait::render(
        &mut assets.geometry,
        &assets.avatar,
        resident,
        transform,
        &mut surface,
    )?;
    let image = if outcome.copy {
        surface
            .image(
                format,
                source_key,
                request.target,
                request.fixed,
                request.padding,
            )
            .map_err(anyhow::Error::msg)?
    } else {
        None
    };
    Ok(Rendered {
        success: outcome.success,
        image,
    })
}
