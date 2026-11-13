//! Application support shared by original households and authored laboratories.
use iced::{Color, Element, Size, Theme, widget, window};
use sims_render::Scene;
use std::{path::Path, sync::Arc, time::Instant};

#[path = "window_session.rs"]
pub(super) mod window_session;

pub(super) fn validate_output(root: &Path, path: &Path) -> anyhow::Result<()> {
    let root = root.canonicalize()?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut ancestor = absolute.as_path();
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid output path"))?;
    }
    if ancestor.canonicalize()?.starts_with(root) {
        anyhow::bail!("output must be outside original game data");
    }
    // Do not follow a dangling symlink or allow parent traversal into a protected directory.
    if absolute
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        anyhow::bail!("output path cannot contain parent traversal");
    }
    if std::fs::symlink_metadata(&absolute).is_ok_and(|m| m.file_type().is_symlink()) {
        anyhow::bail!("output cannot be a symlink");
    }
    Ok(())
}
pub(super) fn atomic_save(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let tmp = parent.join(format!(".sims-room-{}-{stamp}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    let result = (|| -> anyhow::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
pub(super) fn scene_cursor(
    scene: &Scene,
    bounds: iced::Rectangle,
    cursor: iced::mouse::Cursor,
) -> Option<[f32; 2]> {
    let p = cursor.position_in(bounds)?;
    let scale = (bounds.width / scene.width as f32).min(bounds.height / scene.height as f32);
    if scale <= 0. {
        return None;
    }
    let x = (p.x - (bounds.width - scene.width as f32 * scale) / 2.) / scale;
    let y = (p.y - (bounds.height - scene.height as f32 * scale) / 2.) / scale;
    (x >= 0. && y >= 0. && x < scene.width as f32 && y < scene.height as f32).then_some([x, y])
}
#[derive(Debug)]
pub(super) struct Primitive {
    pub(super) scene: Arc<Scene>,
}
pub(super) struct Pipeline {
    renderer: sims_render::gpu::Renderer,
    viewport: [f32; 4],
}
impl widget::shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, _: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            renderer: sims_render::gpu::Renderer::new(device, format),
            viewport: [0.; 4],
        }
    }
}
impl widget::shader::Primitive for Primitive {
    type Pipeline = Pipeline;
    fn prepare(
        &self,
        p: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &iced::Rectangle,
        viewport: &widget::shader::Viewport,
    ) {
        let started = window_session::enabled().then(Instant::now);
        let dpi = viewport.scale_factor();
        let size = viewport.physical_size();
        let scale =
            (bounds.width / self.scene.width as f32).min(bounds.height / self.scene.height as f32);
        let w = self.scene.width as f32 * scale;
        let h = self.scene.height as f32 * scale;
        p.viewport = [
            (bounds.x + (bounds.width - w) / 2.) * dpi,
            (bounds.y + (bounds.height - h) / 2.) * dpi,
            w * dpi,
            h * dpi,
        ];
        let preparation = p
            .renderer
            .prepare(device, queue, &self.scene, [size.width, size.height]);
        if let Some(started) = started {
            window_session::prepared(started, preparation, &self.scene);
        }
    }
    fn render(
        &self,
        p: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: &iced::Rectangle<u32>,
    ) {
        let started = window_session::enabled().then(Instant::now);
        if started.is_some() {
            window_session::bind_presentation(target);
        }
        p.renderer.render(
            encoder,
            target,
            p.viewport,
            [clip.x, clip.y, clip.width, clip.height],
            None,
        );
        if let Some(started) = started {
            window_session::stage("scene_encode", started);
        }
    }
}
pub(super) fn room_theme() -> Theme {
    Theme::custom(
        "Room",
        iced::theme::Palette {
            background: Color::from_rgb8(17, 24, 29),
            text: Color::from_rgb8(230, 238, 239),
            primary: Color::from_rgb8(70, 175, 153),
            success: Color::from_rgb8(84, 187, 132),
            warning: Color::from_rgb8(238, 179, 93),
            danger: Color::from_rgb8(245, 131, 115),
        },
    )
}
pub(super) fn render_headless_element<M>(
    root: &Path,
    path: &Path,
    element: Element<'_, M>,
) -> anyhow::Result<()> {
    render_headless_element_at_size(root, path, element, [1320, 900])
}
pub(super) fn render_headless_element_at_size<M>(
    root: &Path,
    path: &Path,
    mut element: Element<'_, M>,
    display: [u32; 2],
) -> anyhow::Result<()> {
    use iced::advanced::{
        layout,
        renderer::{Headless, Renderer as _},
        widget::Tree,
    };
    validate_output(root, path)?;
    let mut renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
        iced::Font::DEFAULT,
        iced::Pixels(16.),
        Some("wgpu"),
    ))
    .ok_or_else(|| anyhow::anyhow!("no headless GPU adapter"))?;
    let size = Size::new(display[0] as f32, display[1] as f32);
    let bounds = iced::Rectangle::with_size(size);
    let mut tree = Tree::new(element.as_widget());
    let node = element.as_widget_mut().layout(
        &mut tree,
        &mut renderer,
        &layout::Limits::new(Size::ZERO, size),
    );
    // Widgets resolve enabled/hover styles during the redraw event, just as
    // they do in a real window. No input or simulation messages are delivered.
    let mut messages = vec![];
    element.as_widget_mut().update(
        &mut tree,
        &iced::Event::Window(window::Event::RedrawRequested(Instant::now())),
        iced::advanced::Layout::new(&node),
        iced::mouse::Cursor::Unavailable,
        &renderer,
        &mut iced::advanced::clipboard::Null,
        &mut iced::advanced::Shell::new(&mut messages),
        &bounds,
    );
    anyhow::ensure!(
        messages.is_empty(),
        "headless redraw unexpectedly emitted input"
    );
    renderer.reset(bounds);
    let theme = room_theme();
    element.as_widget().draw(
        &tree,
        &mut renderer,
        &theme,
        &iced::advanced::renderer::Style {
            text_color: theme.palette().text,
        },
        iced::advanced::Layout::new(&node),
        iced::mouse::Cursor::Unavailable,
        &bounds,
    );
    let rgba = Headless::screenshot(
        &mut renderer,
        Size::new(display[0], display[1]),
        1.,
        theme.palette().background,
    );
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    image::save_buffer(path, &rgba, display[0], display[1], image::ColorType::Rgba8)?;
    Ok(())
}

pub(super) fn entries<'a>(
    program: &'a sims_vm::Program,
    sim: &sims_sim::Simulation,
    target: i16,
) -> &'a [sims_vm::program::MenuEntry] {
    sim.vm
        .world
        .objects
        .get(&target)
        .and_then(|o| o.definition.as_ref())
        .and_then(|d| {
            program
                .modules
                .get(&d.source)?
                .object_interactions
                .get(&d.object)
        })
        .map_or(&[], Vec::as_slice)
}
