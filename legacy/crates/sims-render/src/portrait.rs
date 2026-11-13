//! Software endpoint for the native portrait draw/restore controller. Projection,
//! background and light are explicit presenter inputs. This endpoint is not a
//! claim of pixel equivalence to the original Windows graphics backend.
use crate::{Draw, Scene, Texture, Vertex};
use sims_content::{
    avatar::Part, character_geometry::PublishedVertices, character_portrait::Renderer,
};
use sims_sim::pose::{
    model::Matrix,
    portrait::{Crop, CropInput, border},
};
use std::{collections::BTreeMap, sync::Arc};

pub mod blit;
pub mod bmp;
pub mod cache;
pub mod generate;
pub mod images;
pub mod job;
pub mod persistence;
pub mod resize;
pub mod retrieval;
pub mod speech;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drawable {
    pub matrix: Matrix,
    pub mode: u32,
    pub cached: bool,
}
#[derive(Clone)]
pub struct Surface {
    pub drawables: BTreeMap<usize, Drawable>,
    pub projection: Matrix,
    pub background: [u8; 4],
    pub light: [f32; 3],
    pub ambient: f32,
    pub frame: Option<image::RgbaImage>,
    pub uploads: Vec<usize>,
    /// Native draw return codes for controlled backend-failure checks. The
    /// normal software renderer leaves this empty and returns zero.
    pub draw_status: BTreeMap<usize, i32>,
    vertices: Vec<Vertex>,
    draws: Vec<Draw>,
    textures: Vec<Texture>,
    active: bool,
}
impl Surface {
    /// Dedicated portrait surface using the original view matrices and
    /// temporary white ambient/directional-light setup. Background is the
    /// generator's explicit input. No global scene lighting is modified.
    pub fn for_portrait(
        drawables: BTreeMap<usize, Drawable>,
        view: sims_sim::pose::portrait::view::View,
        background: [u8; 4],
    ) -> Result<Self, String> {
        Ok(Self::new(
            drawables,
            view.screen,
            background,
            view.portrait_light()?,
            1.,
        ))
    }
    pub fn new(
        drawables: BTreeMap<usize, Drawable>,
        projection: Matrix,
        background: [u8; 4],
        light: [f32; 3],
        ambient: f32,
    ) -> Self {
        Self {
            drawables,
            projection,
            background,
            light,
            ambient,
            frame: None,
            uploads: vec![],
            draw_status: BTreeMap::new(),
            vertices: vec![],
            draws: vec![],
            textures: vec![],
            active: false,
        }
    }
    fn drawable(&self, binding: usize) -> Result<&Drawable, String> {
        self.drawables
            .get(&binding)
            .ok_or_else(|| "portrait drawable state absent".into())
    }
    fn drawable_mut(&mut self, binding: usize) -> Result<&mut Drawable, String> {
        self.drawables
            .get_mut(&binding)
            .ok_or_else(|| "portrait drawable state absent".into())
    }
    /// Original border scan followed by 4e7cba's crop/aspect rules. Equality
    /// uses this backend's exact clear color; native u16 conversion and image
    /// resampling are separate operations, not inferred from RGBA pixels.
    pub fn crop(
        &self,
        target: [i32; 2],
        fixed: Option<[i32; 4]>,
        padding: [i32; 4],
    ) -> Result<Crop, String> {
        let frame = self.frame.as_ref().ok_or("portrait frame absent")?;
        if frame.dimensions() != (200, 200) {
            return Err("invalid portrait scratch dimensions".into());
        }
        let margins = if fixed.is_some() {
            None
        } else {
            let pixels = frame.pixels().copied().collect::<Vec<_>>();
            let scan = border::scan(
                Some(&pixels),
                200,
                [0, 0, 200, 200],
                &image::Rgba(self.background),
            )?;
            scan.success.then_some(scan.margins)
        };
        CropInput {
            target,
            fixed,
            margins,
            padding,
        }
        .crop()
    }

    /// Convert this software scratch frame to the selected native display
    /// format, scan its 16-bit background and execute 4ba79c's pixel path.
    /// The source surface's color-key flag is explicit; a background clear
    /// color alone does not imply that the original surface had that flag.
    pub fn image(
        &self,
        format: resize::Format,
        source_key: Option<u16>,
        target: [i32; 2],
        fixed: Option<[i32; 4]>,
        padding: [i32; 4],
    ) -> Result<Option<resize::Image>, String> {
        let (pixels, crop) = self.image_input(format, target, fixed, padding)?;
        resize::resize(
            format,
            resize::Source {
                pixels: &pixels,
                width: 200,
                height: 200,
                stride: 200,
                key: source_key,
            },
            crop.scale,
            Some(crop.rect),
            false,
        )
    }

    /// Prepared input is also useful for comparing the software-generated
    /// scratch pixels against the original resizer without a Windows GPU.
    pub fn image_input(
        &self,
        format: resize::Format,
        target: [i32; 2],
        fixed: Option<[i32; 4]>,
        padding: [i32; 4],
    ) -> Result<(Vec<u16>, Crop), String> {
        let frame = self.frame.as_ref().ok_or("portrait frame absent")?;
        if frame.dimensions() != (200, 200) {
            return Err("invalid portrait scratch dimensions".into());
        }
        let pixels = frame
            .pixels()
            .map(|p| format.encode([p[0], p[1], p[2]]))
            .collect::<Vec<_>>();
        let background =
            format.encode([self.background[0], self.background[1], self.background[2]]);
        let margins = if fixed.is_some() {
            None
        } else {
            let scan = border::scan(Some(&pixels), 200, [0, 0, 200, 200], &background)?;
            scan.success.then_some(scan.margins)
        };
        let crop = CropInput {
            target,
            fixed,
            margins,
            padding,
        }
        .crop()?;
        Ok((pixels, crop))
    }
}
impl Renderer for Surface {
    fn begin(&mut self) -> Result<i32, String> {
        if self.active {
            return Err("portrait surface already drawing".into());
        }
        if !self.ambient.is_finite() || self.light.iter().any(|v| !v.is_finite()) {
            return Err("invalid portrait light".into());
        }
        self.active = true;
        self.vertices.clear();
        self.draws.clear();
        self.textures.clear();
        self.uploads.clear();
        self.frame = None;
        Ok(0)
    }
    fn matrix(&self, binding: usize) -> Result<Matrix, String> {
        Ok(self.drawable(binding)?.matrix)
    }
    fn publish_matrix(&mut self, binding: usize, matrix: Matrix) -> Result<(), String> {
        self.drawable_mut(binding)?.matrix = matrix;
        Ok(())
    }
    fn mode(&self, binding: usize) -> Result<u32, String> {
        Ok(self.drawable(binding)?.mode)
    }
    fn set_mode(&mut self, binding: usize, mode: u32) -> Result<(), String> {
        self.drawable_mut(binding)?.mode = mode;
        Ok(())
    }
    fn cached_drawable(&self, binding: usize) -> Result<bool, String> {
        Ok(self.drawable(binding)?.cached)
    }
    fn upload(&mut self, binding: usize, _: &PublishedVertices) -> Result<(), String> {
        self.uploads.push(binding);
        Ok(())
    }
    fn draw(
        &mut self,
        binding: usize,
        part: &Part,
        vertices: &PublishedVertices,
    ) -> Result<i32, String> {
        if !self.active || self.drawable(binding)?.mode != 0 {
            return Err("portrait draw outside native mode".into());
        }
        let status = self.draw_status.get(&binding).copied().unwrap_or(0);
        if status != 0 {
            return Ok(status);
        }
        let matrix = self.drawable(binding)?.matrix;
        let light = glam::Vec3::from_array(self.light).normalize_or_zero();
        let start = self.vertices.len() as u32;
        for &index in part.mesh.triangles.iter().flatten() {
            let vertex = vertices
                .vertices
                .get(index as usize)
                .ok_or("portrait triangle vertex absent")?;
            let p = self.projection.point(matrix.point(vertex.position)?)?;
            let normal = glam::Vec3::from_array(std::array::from_fn(|axis| {
                (0..3)
                    .map(|row| vertex.normal[row] * matrix.values[row * 4 + axis])
                    .sum()
            }))
            .normalize_or_zero();
            let shade =
                (self.ambient + (1. - self.ambient) * normal.dot(light).max(0.)).clamp(0., 1.);
            self.vertices.push(Vertex {
                position: [p[0] / 100. - 1., 1. - p[1] / 100., p[2] / 65535.],
                uv: vertex.uv,
                tint: [shade, shade, shade, 1.],
                depth_scale: 0.,
            });
        }
        self.draws.push(Draw {
            object: None,
            vertices: start..self.vertices.len() as u32,
            texture: self.textures.len(),
        });
        self.textures.push(Texture {
            width: part.texture.width,
            height: part.texture.height,
            rgba: part.texture.rgba.clone().into(),
            depth: vec![255; (part.texture.width * part.texture.height) as usize].into(),
        });
        Ok(0)
    }
    fn end(&mut self) -> Result<i32, String> {
        if !self.active {
            return Err("portrait surface is not drawing".into());
        }
        let scene = Scene {
            width: 200,
            height: 200,
            censors: vec![],
            vertices: std::mem::take(&mut self.vertices),
            draws: std::mem::take(&mut self.draws),
            textures: Arc::new(std::mem::take(&mut self.textures)),
        };
        self.frame = Some(crate::software::render_with_background(
            &scene,
            self.background,
        ));
        self.active = false;
        Ok(0)
    }
}
