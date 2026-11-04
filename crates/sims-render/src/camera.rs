//! Adapt the original retained view to the replacement scene renderer.
//! The orthographic projection is anchored by49b0de; the existing ideal basis
//! differs slightly from native float matrices (tested within a quarter pixel).
//! Sprite depth remains the renderer's existing approximation. This adapter
//! does not acknowledge any native device service.
use crate::Camera;
use anyhow::{Result, ensure};
use glam::Vec3;
use sims_sim::pose::portrait::view::Input;

impl Camera {
    pub fn from_native(input: Input, level: i32) -> Result<Self> {
        ensure!(
            input.viewport[0..2] == [0, 0],
            "native scene camera needs a local viewport"
        );
        ensure!(
            (1..=2).contains(&level),
            "native scene camera floor unavailable"
        );
        let view = input.build().map_err(anyhow::Error::msg)?;
        let zoom = (1u32 << input.zoom) as f32 / 8.;
        let anchor = view
            .screen
            .point([0., 0., 0.])
            .map_err(anyhow::Error::msg)?;
        let [_, _, width, height] = input.viewport;
        let dx = (width as f32 * 0.5 - anchor[0]) / (64. * zoom);
        let dy = (height as f32 * 0.57 - anchor[1]) / (32. * zoom);
        let rotated_center = Vec3::new((dx + dy) * 0.5, 0., (dy - dx) * 0.5);
        let mut camera = Self {
            level,
            width,
            height,
            rotation: (4 - input.rotation) as u8 % 4,
            zoom,
            center: Vec3::ZERO,
        };
        camera.center = camera.rotated(rotated_center);
        camera.rotation = input.rotation as u8;
        Ok(camera)
    }
}
