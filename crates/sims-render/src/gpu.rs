//! GPU composition shares scene geometry/projection with the diagnostic rasterizer.
use crate::{Scene, Vertex};
use std::sync::Arc;
use wgpu::util::DeviceExt;

#[derive(Debug, Default, Clone, Copy)]
pub struct Preparation {
    pub uploaded_textures: usize,
    pub uploaded_bytes: usize,
    pub software_composited: bool,
}

// A scene can replace its texture-vector owner when a single appearance is
// attached. Slots are still addressed by the scene's texture indices. Compare
// complete immutable payloads, not pointer addresses, hashes or filenames.
fn changed_textures(
    previous: &[Option<crate::Texture>],
    current: &[crate::Texture],
    draws: &[crate::Draw],
) -> Vec<usize> {
    draws
        .iter()
        .map(|draw| draw.texture)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|&index| previous.get(index).and_then(Option::as_ref) != Some(&current[index]))
        .collect()
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    fn texture(color: u8) -> crate::Texture {
        crate::Texture {
            width: 1,
            height: 1,
            rgba: vec![color, 0, 0, 255].into(),
            depth: vec![255].into(),
        }
    }
    fn draws(slots: &[usize]) -> Vec<crate::Draw> {
        slots
            .iter()
            .map(|&texture| crate::Draw {
                vertices: 0..0,
                texture,
                object: None,
            })
            .collect()
    }

    #[test]
    fn texture_diff_checks_payload_and_dimensions_and_preserves_slot_identity() {
        let old = vec![texture(1), texture(2)];
        let cached = old.iter().cloned().map(Some).collect::<Vec<_>>();
        let visible = draws(&[0, 1, 0]);
        assert_eq!(changed_textures(&[], &old, &visible), [0, 1]);
        assert!(changed_textures(&cached, &old.clone(), &visible).is_empty());
        assert!(changed_textures(&cached, &old[..1], &draws(&[0])).is_empty());
        let mut new = old.clone();
        new.push(texture(3));
        assert_eq!(changed_textures(&cached, &new, &draws(&[2, 0])), [2]);
        new.swap(0, 1);
        assert_eq!(
            changed_textures(&cached, &new, &draws(&[2, 0, 1])),
            [0, 1, 2]
        );
        for field in 0..4 {
            let mut new = old.clone();
            match field {
                0 => new[1].width = 2,
                1 => new[1].height = 2,
                2 => Arc::make_mut(&mut new[1].rgba)[0] = 17,
                _ => Arc::make_mut(&mut new[1].depth)[0] = 17,
            }
            assert_eq!(changed_textures(&cached, &new, &visible), [1]);
        }
    }

    #[test]
    fn unseen_slots_and_changes_remain_pending_until_first_visible_use() {
        let original = vec![texture(1), texture(2), texture(3)];
        let mut cached = vec![Some(original[0].clone()), None, None];
        assert!(changed_textures(&cached, &original, &draws(&[0])).is_empty());
        let mut current = original.clone();
        current[1] = texture(4);
        assert!(changed_textures(&cached, &current, &draws(&[0])).is_empty());
        assert_eq!(changed_textures(&cached, &current, &draws(&[2, 2])), [2]);
        cached[2] = Some(current[2].clone());
        assert_eq!(changed_textures(&cached, &current, &draws(&[1, 0])), [1]);
        cached[1] = Some(current[1].clone());
        current[1] = texture(5);
        assert!(changed_textures(&cached, &current, &draws(&[0])).is_empty());
        assert_eq!(changed_textures(&cached, &current, &draws(&[1])), [1]);
        cached.truncate(1);
        assert_eq!(changed_textures(&cached, &current, &draws(&[2])), [2]);
        assert!(changed_textures(&cached, &current, &[]).is_empty());
    }
}
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: Option<Arc<Vec<crate::Texture>>>,
    cached_textures: Vec<Option<crate::Texture>>,
    validated_textures: Vec<bool>,
    groups: Vec<Option<wgpu::BindGroup>>,
    vertices: Option<wgpu::Buffer>,
    draws: Vec<crate::Draw>,
    depth: Option<wgpu::TextureView>,
    size: [u32; 2],
}
impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("room textures"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sims sprite/mesh depth"),
            source: wgpu::ShaderSource::Wgsl(include_str!("world.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("room"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline=device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
   label:Some("room sprite/mesh"),layout:Some(&pipeline_layout),
   vertex:wgpu::VertexState {module:&shader,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[wgpu::VertexBufferLayout {array_stride:std::mem::size_of::<Vertex>() as u64,step_mode:wgpu::VertexStepMode::Vertex,attributes:&wgpu::vertex_attr_array![0=>Float32x3,1=>Float32x2,2=>Float32x4,3=>Float32]}]},
   fragment:Some(wgpu::FragmentState {module:&shader,entry_point:Some("fs"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState {format,blend:Some(wgpu::BlendState::ALPHA_BLENDING),write_mask:wgpu::ColorWrites::ALL})]}),
   primitive:wgpu::PrimitiveState {cull_mode:None,..Default::default()},
   depth_stencil:Some(wgpu::DepthStencilState {format:wgpu::TextureFormat::Depth32Float,depth_write_enabled:true,depth_compare:wgpu::CompareFunction::LessEqual,stencil:Default::default(),bias:Default::default()}),
   multisample:Default::default(),multiview:None,cache:None,
  });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("original pixels"),
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            textures: None,
            cached_textures: vec![],
            validated_textures: vec![],
            groups: vec![],
            vertices: None,
            draws: vec![],
            depth: None,
            size: [0, 0],
        }
    }
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        target_size: [u32; 2],
    ) -> Preparation {
        if !scene.censors.is_empty() {
            // The recovered 16-bit mosaic is currently a CPU compositor. Keep
            // one implementation for Iced and headless verification; upload
            // the completed frame instead of drawing kind-1 boxes as clothing.
            // A GPU implementation can replace this bounded slow path once it
            // matches the native pixel/depth/RNG corpus.
            let frame = crate::software::composited_scene(scene);
            let mut preparation = self.prepare(device, queue, &frame, target_size);
            preparation.software_composited = true;
            return preparation;
        }
        let mut preparation = Preparation::default();
        let textures_changed = self
            .textures
            .as_ref()
            .is_none_or(|previous| !Arc::ptr_eq(previous, &scene.textures));
        if textures_changed {
            self.validated_textures.clear();
            self.validated_textures.resize(scene.textures.len(), false);
        }
        if textures_changed
            || scene.draws.iter().any(|draw| {
                !self
                    .validated_textures
                    .get(draw.texture)
                    .copied()
                    .unwrap_or(false)
            })
        {
            self.groups.resize_with(scene.textures.len(), || None);
            self.cached_textures
                .resize_with(scene.textures.len(), || None);
            for i in changed_textures(&self.cached_textures, &scene.textures, &scene.draws) {
                let t = &scene.textures[i];
                preparation.uploaded_textures += 1;
                preparation.uploaded_bytes += t.rgba.len() + t.depth.len();
                let size = wgpu::Extent3d {
                    width: t.width,
                    height: t.height,
                    depth_or_array_layers: 1,
                };
                let mut views = Vec::new();
                for (format, data, bpp) in [
                    (wgpu::TextureFormat::Rgba8UnormSrgb, t.rgba.as_ref(), 4),
                    (wgpu::TextureFormat::R8Unorm, t.depth.as_ref(), 1),
                ] {
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("original asset"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    queue.write_texture(
                        texture.as_image_copy(),
                        data,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(t.width * bpp),
                            rows_per_image: Some(t.height),
                        },
                        size,
                    );
                    views.push(texture.create_view(&Default::default()));
                }
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("room material"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&views[0]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&views[1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                });
                self.groups[i] = Some(group);
                self.cached_textures[i] = Some(t.clone());
            }
            for draw in &scene.draws {
                self.validated_textures[draw.texture] = true;
            }
            self.textures = Some(scene.textures.clone());
        }
        self.vertices = Some(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("room posed vertices"),
                contents: bytemuck::cast_slice(&scene.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        self.draws = scene.draws.clone();
        if self.size != target_size {
            let depth = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("room shared depth"),
                size: wgpu::Extent3d {
                    width: target_size[0],
                    height: target_size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.depth = Some(depth.create_view(&Default::default()));
            self.size = target_size;
        }
        preparation
    }
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        viewport: [f32; 4],
        clip: [u32; 4],
        clear: Option<wgpu::Color>,
    ) {
        if clip[2] == 0 || clip[3] == 0 {
            return;
        }
        let Some(vertices) = &self.vertices else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Sims room"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: clear.map(wgpu::LoadOp::Clear).unwrap_or(wgpu::LoadOp::Load),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: self.depth.as_ref().unwrap(),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_viewport(viewport[0], viewport[1], viewport[2], viewport[3], 0., 1.);
        pass.set_scissor_rect(clip[0], clip[1], clip[2], clip[3]);
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, vertices.slice(..));
        for draw in &self.draws {
            pass.set_bind_group(
                0,
                self.groups[draw.texture]
                    .as_ref()
                    .expect("drawn texture was prepared"),
                &[],
            );
            pass.draw(draw.vertices.clone(), 0..1);
        }
    }
}
