//! Render an actual imported-house request through the same Iced prompt widget.
//! No window, input delivery or simulator execution.
#[path = "../src/dialog_view.rs"]
mod dialog_view;
use iced::advanced::{
    layout,
    renderer::{Headless, Renderer as _},
    widget::Tree,
};
use iced::{
    Element, Fill, Size, Theme,
    widget::{column, container, text},
};

fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let report_path = args
        .first()
        .map(String::as_str)
        .unwrap_or("cache/original-house-inferred-walking.json");
    let title = args.get(1).map(String::as_str);
    let report: serde_json::Value = serde_json::from_slice(&std::fs::read(report_path)?)?;
    let rows = report
        .as_array()
        .or_else(|| report.get("houses").and_then(|r| r.as_array()))
        .ok_or_else(|| anyhow::anyhow!("house report array absent"))?;
    let request = rows
        .iter()
        .flat_map(|row| {
            row["dialogs"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_else(|| std::slice::from_ref(row))
        })
        .find(|request| title.is_none_or(|title| request["record"]["title"] == title))
        .ok_or_else(|| anyhow::anyhow!("no original dialog captured"))?;
    let record: sims_vm::dialog::Record = serde_json::from_value(request["record"].clone())?;
    let index = sims_content::Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let icons = dialog_view::Icons::load(&index)?;
    icons.prepare(std::slice::from_ref(&record))?;
    if record.portrait.is_some() {
        let path = args
            .get(3)
            .ok_or_else(|| anyhow::anyhow!("portrait pixel-pool snapshot argument required"))?;
        let pixels: sims_render::portrait::cache::Images =
            serde_json::from_slice(&std::fs::read(path)?)?;
        icons.prepare_portraits(std::slice::from_ref(&record), &pixels)?;
    }
    let art = icons.publish_art();
    let mut element: Element<'_, (u64, u32)> = container(
        column![
            text("Original household · waiting for your response").size(17),
            dialog_view::view(&record, &art, |id, result| (id, result))
        ]
        .spacing(24),
    )
    .center(Fill)
    .into();
    let size = Size::new(900., 500.);
    let bounds = iced::Rectangle::with_size(size);
    let mut renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
        iced::Font::DEFAULT,
        iced::Pixels(16.),
        Some("wgpu"),
    ))
    .ok_or_else(|| anyhow::anyhow!("headless adapter unavailable"))?;
    let mut tree = Tree::new(element.as_widget());
    let node = element.as_widget_mut().layout(
        &mut tree,
        &mut renderer,
        &layout::Limits::new(Size::ZERO, size),
    );
    let mut messages = vec![];
    element.as_widget_mut().update(
        &mut tree,
        &iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        )),
        iced::advanced::Layout::new(&node),
        iced::mouse::Cursor::Unavailable,
        &renderer,
        &mut iced::advanced::clipboard::Null,
        &mut iced::advanced::Shell::new(&mut messages),
        &bounds,
    );
    anyhow::ensure!(messages.is_empty(), "headless redraw delivered input");
    let theme = Theme::Dark;
    renderer.reset(bounds);
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
        Size::new(900, 500),
        1.,
        theme.palette().background,
    );
    image::save_buffer(
        args.get(2)
            .map(String::as_str)
            .unwrap_or("cache/original-dialog-iced.png"),
        &rgba,
        900,
        500,
        image::ColorType::Rgba8,
    )?;
    println!(
        "Rendered actual {} request #{} headlessly",
        record.title, record.id
    );
    Ok(())
}
