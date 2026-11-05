//! Exercise the exact GPU renderer used by the Iced shader without a window.
use anyhow::{Context, Result};
use std::{
    future::Future,
    sync::Arc,
    task::{Poll, Wake, Waker},
};

fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}

pub fn render(scene: &sims_render::Scene) -> Result<image::RgbaImage> {
    render_at(
        scene,
        [scene.width, scene.height],
        [0., 0., scene.width as f32, scene.height as f32],
    )
}

pub fn render_at(
    scene: &sims_render::Scene,
    target: [u32; 2],
    viewport: [f32; 4],
) -> Result<image::RgbaImage> {
    Ok(render_sequence_at(&[scene], target, viewport)?.remove(0))
}

/// Retain one GPU renderer across successive scenes to exercise cache changes.
pub fn render_sequence_at(
    scenes: &[&sims_render::Scene],
    target: [u32; 2],
    viewport: [f32; 4],
) -> Result<Vec<image::RgbaImage>> {
    Ok(render_sequence_with_preparation(scenes, target, viewport)?
        .into_iter()
        .map(|(image, _)| image)
        .collect())
}

pub fn render_sequence_with_preparation(
    scenes: &[&sims_render::Scene],
    target: [u32; 2],
    viewport: [f32; 4],
) -> Result<Vec<(image::RgbaImage, sims_render::gpu::Preparation)>> {
    let instance = wgpu::Instance::default();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let size = wgpu::Extent3d {
        width: target[0],
        height: target[1],
        depth_or_array_layers: 1,
    };
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless room color"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let row_bytes = (target[0] * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("headless room readback"),
        size: u64::from(row_bytes) * u64::from(target[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut renderer = sims_render::gpu::Renderer::new(&device, format);
    let mut images = Vec::new();
    for scene in scenes {
        let preparation = renderer.prepare(&device, &queue, scene, target);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &mut encoder,
            &texture.create_view(&Default::default()),
            viewport,
            [0, 0, target[0], target[1]],
            Some(wgpu::Color::BLACK),
        );
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(scene.height),
                },
            },
            size,
        );
        queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(60)),
        })?;
        receive.recv_timeout(std::time::Duration::from_secs(1))??;
        let mapped = buffer.slice(..).get_mapped_range();
        let pixels = mapped
            .chunks_exact(row_bytes as usize)
            .flat_map(|row| row[..target[0] as usize * 4].iter().copied())
            .collect();
        images.push((
            image::RgbaImage::from_raw(target[0], target[1], pixels)
                .context("invalid GPU readback")?,
            preparation,
        ));
        drop(mapped);
        buffer.unmap();
    }
    Ok(images)
}
