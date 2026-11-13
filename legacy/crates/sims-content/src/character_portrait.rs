//! Actual asset bindings for the native portrait draw/restore sequence. The
//! renderer owns explicit drawable matrices/modes and graphics status codes;
//! the simulation retains its animation pose, physical placement and clocks.
use crate::{
    avatar::Part,
    character_geometry::{self, CharacterGeometry, PublishedVertices},
    saved_avatar::BoundAvatar,
};
use anyhow::{Context, Result};
use sims_sim::{
    character::Resident,
    pose::{model::Matrix, portrait::draw},
};

pub trait Renderer {
    fn begin(&mut self) -> Result<i32, String>;
    fn matrix(&self, binding: usize) -> Result<Matrix, String>;
    fn publish_matrix(&mut self, binding: usize, matrix: Matrix) -> Result<(), String>;
    fn mode(&self, binding: usize) -> Result<u32, String>;
    fn set_mode(&mut self, binding: usize, mode: u32) -> Result<(), String>;
    fn cached_drawable(&self, binding: usize) -> Result<bool, String>;
    fn upload(&mut self, binding: usize, vertices: &PublishedVertices) -> Result<(), String>;
    fn draw(
        &mut self,
        binding: usize,
        part: &Part,
        vertices: &PublishedVertices,
    ) -> Result<i32, String>;
    fn end(&mut self) -> Result<i32, String>;
}

/// The caller must first finish forced appearance selection. This call is
/// synchronous. Clone-owned software renderer state and geometry commit only
/// after supported execution, including native nonzero graphics statuses.
pub fn render<R: Renderer + Clone>(
    geometry: &mut CharacterGeometry,
    avatar: &BoundAvatar,
    resident: &Resident,
    matrix: Matrix,
    renderer: &mut R,
) -> Result<draw::Outcome> {
    let selected = geometry.portrait_bindings(avatar)?;
    // 4e77c6 rejects an empty HEAD vector before acquiring/clearing a surface.
    // The lower draw-loop segment separately allows an empty vector.
    if selected.is_empty() {
        return Ok(draw::Outcome {
            success: false,
            copy: false,
        });
    }
    let mut next = geometry.clone();
    let mut next_renderer = renderer.clone();
    let bindings = selected
        .iter()
        .map(|&(binding, _)| binding)
        .collect::<Vec<_>>();
    let mut host = Host {
        geometry: &mut next,
        avatar,
        resident,
        selected,
        renderer: &mut next_renderer,
    };
    let result = draw::render(&mut host, &bindings, matrix).map_err(anyhow::Error::msg)?;
    *geometry = next;
    *renderer = next_renderer;
    Ok(result)
}

struct Host<'a, R> {
    geometry: &'a mut CharacterGeometry,
    avatar: &'a BoundAvatar,
    resident: &'a Resident,
    selected: Vec<(usize, usize)>,
    renderer: &'a mut R,
}
impl<R: Renderer> Host<'_, R> {
    fn part(&self, binding: usize) -> Result<usize, String> {
        self.selected
            .iter()
            .find_map(|&(b, p)| (b == binding).then_some(p))
            .ok_or_else(|| "portrait binding absent".into())
    }
}
impl<R: Renderer> draw::Host for Host<'_, R> {
    fn begin(&mut self) -> Result<i32, String> {
        self.renderer.begin()
    }
    fn matrix(&mut self, binding: usize) -> Result<Matrix, String> {
        self.renderer.matrix(binding)
    }
    fn vertices(&mut self, binding: usize, rest: bool) -> Result<(), String> {
        let part = self.part(binding)?;
        let cached = self.renderer.cached_drawable(binding)?;
        let hierarchy = &self.resident.character.hierarchy;
        let notify = character_geometry::update_binding_vertices(
            self.geometry
                .state
                .bindings
                .get_mut(binding)
                .context("portrait geometry binding absent")
                .map_err(|e| e.to_string())?,
            self.geometry
                .vertices
                .get_mut(&binding)
                .context("portrait vertices absent")
                .map_err(|e| e.to_string())?,
            &self.avatar.avatar.parts[part],
            &hierarchy.absolute,
            &hierarchy.updates,
            rest,
            cached,
        )
        .map_err(|e| e.to_string())?;
        if notify {
            self.renderer
                .upload(binding, &self.geometry.vertices[&binding])?;
        }
        Ok(())
    }
    fn publish_matrix(&mut self, binding: usize, matrix: Matrix) -> Result<(), String> {
        self.renderer.publish_matrix(binding, matrix)?;
        self.geometry.state.bindings[binding].drawable.flags |= 2;
        Ok(())
    }
    fn mode(&mut self, binding: usize) -> Result<u32, String> {
        self.renderer.mode(binding)
    }
    fn set_mode(&mut self, binding: usize, mode: u32) -> Result<(), String> {
        self.renderer.set_mode(binding, mode)
    }
    fn draw(&mut self, binding: usize) -> Result<i32, String> {
        let part = self.part(binding)?;
        self.renderer.draw(
            binding,
            &self.avatar.avatar.parts[part],
            &self.geometry.vertices[&binding],
        )
    }
    fn end(&mut self) -> Result<i32, String> {
        self.renderer.end()
    }
}
