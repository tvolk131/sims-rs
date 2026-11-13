//! The supported Tutorial object's picture-dialog presentation. Original
//! 4efabc binds PopupInfoTiles3008 as style5;523fe3/524257 size and place its
//! native-size image, wrapped bitmap-font body, buttons and dismiss widget.
//! No simulation, lesson progress or dialog result is synthesized.
use super::*;
use anyhow::{Result, ensure};
use iced::advanced::image::Renderer as _;
use iced::advanced::text::{self as advanced_text, Paragraph, Renderer as _};
use iced::{
    Color, Length, Point, Rectangle, Size,
    advanced::{Widget, layout, renderer, widget::Tree},
};
#[path = "tutorial_font.rs"]
mod bitmap_font;

const FONT: iced::Font = iced::Font {
    family: iced::font::Family::Name("Comic Sans MS"),
    ..iced::Font::DEFAULT
};
const FONT_SIZE: f32 = 18.;
const LINE_HEIGHT: f32 = 23.;
const INSET: f32 = 21.; //12px style5 edge plus native9px content inset.

#[derive(Debug, Clone)]
pub(super) struct Skin {
    border: Vec<iced::widget::image::Handle>,
    buttons: Vec<iced::widget::image::Handle>,
    minimize: Vec<iced::widget::image::Handle>,
    font: bitmap_font::Font,
}
fn named(index: &sims_content::Index, path: &str) -> Result<image::RgbaImage> {
    let loose = format!("UIGraphics/{path}");
    // Explicit installed files have the same priority as ui_bitmaps::resource.
    let found = index
        .resources
        .iter()
        .filter(|r| r.locator.replace('\\', "/").eq_ignore_ascii_case(&loose))
        .collect::<Vec<_>>();
    let found = if found.is_empty() {
        index
            .resources
            .iter()
            .filter(|r| r.name.replace('\\', "/").eq_ignore_ascii_case(path))
            .collect::<Vec<_>>()
    } else {
        found
    };
    ensure!(
        found.len() == 1,
        "tutorial artwork absent/ambiguous: {path}"
    );
    let mut bitmap =
        image::load_from_memory_with_format(&found[0].read()?, image::ImageFormat::Bmp)?.to_rgba8();
    for p in bitmap.pixels_mut() {
        if p.0[..3] == [255, 0, 255] {
            p.0[3] = 0;
        }
    }
    Ok(bitmap)
}
fn cell(image: &image::RgbaImage, x: u32, y: u32, w: u32, h: u32) -> iced::widget::image::Handle {
    let pixels = image::imageops::crop_imm(image, x, y, w, h).to_image();
    iced::widget::image::Handle::from_rgba(w, h, pixels.into_raw())
}
impl Skin {
    pub(super) fn load(index: &sims_content::Index) -> Result<Self> {
        let back = named(index, "CPanel/Backgrounds/PopupInfoTiles.bmp")?;
        let buttons = named(index, "Shared/Sys/WinBtn.bmp")?;
        let minimize = named(index, "Shared/Sys/MinimizeBox.bmp")?;
        ensure!(
            back.dimensions() == (36, 36)
                && buttons.dimensions() == (260, 33)
                && minimize.dimensions() == (48, 11),
            "tutorial skin dimensions changed"
        );
        Ok(Self {
            font: bitmap_font::Font::load(index)?,
            border: (0..9)
                .map(|n| cell(&back, (n % 3) * 12, (n / 3) * 12, 12, 12))
                .collect(),
            buttons: (0..4).map(|n| cell(&buttons, n * 65, 0, 65, 33)).collect(),
            minimize: (0..4).map(|n| cell(&minimize, n * 12, 0, 12, 11)).collect(),
        })
    }
}

pub(super) fn supported(record: &Record) -> bool {
    // Presentation::capture only hides titles for caller GUID0xc3249a1d.
    // Keep this bounded renderer on its verified private Tutorial pictures;
    // global images, other sources and other dialog families retain fallback.
    let source = record.source.replace('\\', "/");
    source
        .rsplit(['/', ':'])
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case("Tutorial.iff"))
        && record
            .presentation
            .is_some_and(|p| p.hide_title && p.anchored)
        && record.buttons.len() == 1
        && record
            .bitmap_icon
            .is_some_and(|id| (5000..=5024).contains(&id))
        && record.global_bitmap_icon.is_none()
        && record.job_icon.is_none()
        && record.portrait.is_none()
}

struct Line {
    text: String,
    x: f32,
    y: f32,
}
#[derive(Default)]
struct Plan {
    width: f32,
    lines: Vec<Line>,
    bitmap_text: bool,
}
fn description<'a>(content: &'a str) -> advanced_text::Text<&'a str, iced::Font> {
    advanced_text::Text {
        content,
        bounds: Size::new(f32::INFINITY, LINE_HEIGHT),
        size: iced::Pixels(FONT_SIZE),
        line_height: advanced_text::LineHeight::Absolute(iced::Pixels(LINE_HEIGHT)),
        font: FONT,
        align_x: advanced_text::Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: advanced_text::Shaping::Advanced,
        wrapping: advanced_text::Wrapping::None,
    }
}
fn measure(text: &str) -> f32 {
    <<iced::Renderer as advanced_text::Renderer>::Paragraph as Paragraph>::with_text(description(
        text,
    ))
    .min_width()
}
#[derive(Clone, Copy)]
struct Glyph {
    ch: char,
    advance: f32,
    ink: f32,
}
// The native60b620 breaks at spaces/hyphens, consumes explicit LF, and skips
// spaces after an automatic break. The supported body uses original FNTF
// metrics; a text outside its code page retains the prior host-font fallback.
fn fit(glyphs: &[Glyph], max: f32) -> usize {
    if glyphs.is_empty() {
        return 0;
    }
    let mut breakpoint = 0;
    let mut explicit = false;
    let mut width = glyphs[0].ink;
    let mut i = 0;
    if width <= max {
        loop {
            let next = i + 1;
            if glyphs.len() <= next {
                return next;
            }
            if glyphs[i].ch == '\n' {
                explicit = true;
                breakpoint = next;
                break;
            }
            if matches!(glyphs[i].ch, ' ' | '-') && width <= max {
                breakpoint = next;
            }
            width += glyphs[i].advance - glyphs[i].ink + glyphs[next].ink;
            i = next;
            if width > max {
                break;
            }
        }
        if breakpoint == 0 && i > 1 {
            breakpoint = i - 1;
        }
    }
    if breakpoint == 0 {
        while breakpoint < glyphs.len() && !matches!(glyphs[breakpoint].ch, '\0' | '\n' | ' ') {
            breakpoint += 1;
        }
    }
    if !explicit {
        while breakpoint < glyphs.len() && glyphs[breakpoint].ch == ' ' {
            breakpoint += 1;
        }
    }
    breakpoint.max(1)
}
fn count(glyphs: &[Glyph], width: f32) -> usize {
    if width == 0. {
        return 1;
    } //native60b793's explicit zero-width branch.
    let mut offset = 0;
    let mut n = 0;
    while offset < glyphs.len() {
        offset += fit(&glyphs[offset..], width);
        n += 1;
    }
    n
}
struct Block {
    start: usize,
    length: usize,
    x: f32,
    y: f32,
    width: f32,
    lines: usize,
}
struct FlowLayout {
    rounded_height: f32,
    consumed: usize,
    narrow_lines: usize,
    tail_lines: usize,
    height: f32,
    blocks: Vec<Block>,
}
//5351cc rounds the exclusion to the nearest complete line (ties down).
//535266 sizes two text regions;534fe5 can keep one short tail line indented.
fn flow_layout(glyphs: &[Glyph], width: f32, exclusion: Size, line_height: f32) -> FlowLayout {
    let quotient = (exclusion.height / line_height).trunc();
    let n = quotient
        + if (quotient + 1.) * line_height - exclusion.height
            < exclusion.height - quotient * line_height
        {
            1.
        } else {
            0.
        };
    let rounded_height = n * line_height;
    let narrow_width = width - exclusion.width;
    let narrow_count = count(glyphs, narrow_width);
    let (narrow_lines, consumed, tail_lines) = if n < narrow_count as f32 {
        let mut consumed = 0;
        for _ in 0..n as usize {
            consumed += fit(&glyphs[consumed..], narrow_width);
        }
        (n as usize, consumed, count(&glyphs[consumed..], width))
    } else {
        (narrow_count, glyphs.len(), 0)
    };
    let mut blocks = vec![];
    if narrow_lines > 0 {
        blocks.push(Block {
            start: 0,
            length: consumed,
            x: exclusion.width,
            y: 0.,
            width: narrow_width,
            lines: narrow_lines,
        });
    }
    if tail_lines > 0 {
        let short_tail = tail_lines == 1
            && 1. + glyphs[consumed..].iter().map(|g| g.advance).sum::<f32>() <= narrow_width;
        blocks.push(Block {
            start: consumed,
            length: glyphs.len() - consumed,
            x: if short_tail { exclusion.width } else { 0. },
            y: if short_tail {
                narrow_lines as f32 * line_height
            } else {
                rounded_height
            },
            width: if short_tail { narrow_width } else { width },
            lines: tail_lines,
        });
    }
    FlowLayout {
        rounded_height,
        consumed,
        narrow_lines,
        tail_lines,
        height: rounded_height + tail_lines as f32 * line_height,
        blocks,
    }
}
struct Flow {
    glyphs: Vec<Glyph>,
    image: Size,
}
impl Flow {
    fn new(body: &str, image: Size, font: Option<&bitmap_font::Font>) -> Self {
        let space = if font.is_none() {
            measure("a a") - measure("aa")
        } else {
            0.
        };
        let glyphs = body
            .chars()
            .map(|ch| {
                if let Some(metric) = font.and_then(|f| f.metric(ch)) {
                    return Glyph {
                        ch,
                        advance: metric.advance as f32,
                        ink: metric.ink() as f32,
                    };
                }
                let advance = match ch {
                    ' ' => space,
                    '\r' | '\n' => 0.,
                    _ => measure(&ch.to_string()),
                };
                Glyph {
                    ch,
                    advance,
                    ink: advance,
                }
            })
            .collect();
        Self { glyphs, image }
    }
    fn at(&self, width: f32) -> (Vec<Line>, f32) {
        let exclusion = if self.image == Size::ZERO {
            Size::ZERO
        } else {
            Size::new(self.image.width + 18., self.image.height + 18.)
        };
        let plan = flow_layout(&self.glyphs, width, exclusion, LINE_HEIGHT);
        let mut lines = vec![];
        for block in &plan.blocks {
            let mut offset = block.start;
            for row in 0..block.lines {
                let n = fit(
                    &self.glyphs[offset..block.start + block.length],
                    block.width,
                );
                let text = self.glyphs[offset..offset + n]
                    .iter()
                    .map(|g| g.ch)
                    .collect::<String>();
                lines.push(Line {
                    text: text.trim_end_matches(['\r', '\n', ' ']).to_owned(),
                    x: block.x,
                    y: block.y + row as f32 * LINE_HEIGHT,
                });
                offset += n;
            }
        }
        (lines, plan.height)
    }
}
fn extent(body_width: f32, body_height: f32, primary: bool) -> Size {
    Size::new(
        body_width + INSET * 2.,
        INSET + body_height + 18. + if primary { 33. + 9. } else { 0. } + 12.,
    )
}
fn choose_width(flow: &Flow, primary: bool, large: bool, max: f32) -> f32 {
    let target = if large { 1.2 } else { 2. };
    let min = flow.image.width.max(if primary { 130. } else { 100. });
    let max = max.max(min);
    let ratio = |w: f32| {
        let (_, h) = flow.at(w);
        let s = extent(w, h, primary);
        s.width / s.height
    };
    //The body begins at zero width, so524257's first pass applies its minimum.
    let start = min;
    if (ratio(start) - target).abs() <= 0.05 {
        return start;
    }
    let (mut high, mut low) = (start, start);
    for _ in 0..10 {
        if ratio(high) >= target || high >= max {
            break;
        }
        high = (high * 2.).min(max);
    }
    for _ in 0..9 {
        if ratio(low) <= target || low <= min {
            break;
        }
        low = (low / 2.).max(min);
    }
    for _ in 0..9 {
        if high - low <= 20. {
            break;
        }
        let mid = ((high + low) / 2.).trunc();
        if ratio(mid) >= target {
            high = mid;
        } else {
            low = mid;
        }
    }
    if (ratio(high) - target).abs() <= (ratio(low) - target).abs() {
        high
    } else {
        low
    }
}
fn draw_image(
    r: &mut iced::Renderer,
    handle: iced::widget::image::Handle,
    bounds: Rectangle,
    clip: Rectangle,
) {
    r.draw_image(
        iced::advanced::image::Image {
            handle,
            filter_method: iced::widget::image::FilterMethod::Nearest,
            rotation: iced::Radians(0.),
            opacity: 1.,
            border_radius: Default::default(),
            snap: true,
        },
        bounds,
        clip,
    );
}
fn draw_border(skin: &Skin, r: &mut iced::Renderer, b: Rectangle, clip: Rectangle) {
    let widths = [12., b.width - 24., 12.];
    let heights = [12., b.height - 24., 12.];
    let mut y = b.y;
    for (row, h) in heights.into_iter().enumerate() {
        let mut x = b.x;
        for (col, w) in widths.into_iter().enumerate() {
            draw_image(
                r,
                skin.border[row * 3 + col].clone(),
                Rectangle::new(Point::new(x, y), Size::new(w, h)),
                clip,
            );
            x += w;
        }
        y += h;
    }
}
struct ButtonArt<'a> {
    skin: &'a Skin,
    label: &'a str,
    minimize: bool,
}
impl<M> Widget<M, iced::Theme, iced::Renderer> for ButtonArt<'_> {
    fn size(&self) -> Size<Length> {
        let size = if self.minimize {
            Size::new(12., 11.)
        } else {
            Size::new(100., 33.)
        };
        Size::new(size.width.into(), size.height.into())
    }
    fn layout(
        &mut self,
        _: &mut Tree,
        _: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let s = if self.minimize {
            Size::new(12., 11.)
        } else {
            Size::new(100., 33.)
        };
        layout::Node::new(limits.resolve(s.width, s.height, s))
    }
    fn draw(
        &self,
        _: &Tree,
        r: &mut iced::Renderer,
        _: &iced::Theme,
        _: &renderer::Style,
        l: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        clip: &Rectangle,
    ) {
        let b = l.bounds();
        let frame = if cursor.is_over(b) { 2 } else { 0 };
        draw_image(
            r,
            if self.minimize {
                self.skin.minimize[frame].clone()
            } else {
                self.skin.buttons[frame].clone()
            },
            b,
            *clip,
        );
        if !self.minimize {
            let mut text = description(self.label).with_content(self.label.to_owned());
            text.bounds = b.size();
            text.align_x = advanced_text::Alignment::Center;
            text.align_y = iced::alignment::Vertical::Center;
            r.fill_text(text, b.center(), Color::from_rgb8(166, 180, 180), *clip);
        }
    }
}
struct Card<'a, M> {
    record: &'a Record,
    skin: &'a Skin,
    picture: Option<(iced::widget::image::Handle, Size)>,
    buttons: Vec<Element<'a, M>>,
}
impl<M> Widget<M, iced::Theme, iced::Renderer> for Card<'_, M> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<Plan>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(Plan::default())
    }
    fn children(&self) -> Vec<Tree> {
        self.buttons.iter().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.buttons);
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        r: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let image = self.picture.as_ref().map(|p| p.1).unwrap_or(Size::ZERO);
        let bitmap_text = self.skin.font.supports(&self.record.body);
        let body = if bitmap_text {
            self.record.body.clone()
        } else {
            self.record.body.replace("\r\n", "\n")
        };
        let flow = Flow::new(&body, image, bitmap_text.then_some(&self.skin.font));
        let primary = self.record.modal();
        let width = choose_width(
            &flow,
            primary,
            self.record.presentation.unwrap().large,
            limits.max().width - INSET * 2.,
        );
        let (lines, height) = flow.at(width);
        let size = extent(width, height, primary);
        let mut children = vec![];
        for (button, child_tree) in self.buttons.iter_mut().zip(&mut tree.children) {
            let node = button.as_widget_mut().layout(
                child_tree,
                r,
                &layout::Limits::new(Size::ZERO, size),
            );
            let pos = if primary {
                Point::new((size.width - node.size().width) / 2., INSET + height + 18.)
            } else {
                Point::new(size.width - node.size().width - 5., 5.)
            };
            children.push(node.move_to(pos));
        }
        *tree.state.downcast_mut::<Plan>() = Plan {
            width: size.width,
            lines,
            bitmap_text,
        };
        layout::Node::with_children(size, children)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        l: iced::advanced::Layout<'_>,
        r: &iced::Renderer,
        op: &mut dyn iced::advanced::widget::Operation,
    ) {
        for ((b, t), l) in self
            .buttons
            .iter_mut()
            .zip(&mut tree.children)
            .zip(l.children())
        {
            b.as_widget_mut().operate(t, l, r, op);
        }
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        l: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        r: &iced::Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        for ((b, t), l) in self
            .buttons
            .iter_mut()
            .zip(&mut tree.children)
            .zip(l.children())
        {
            b.as_widget_mut()
                .update(t, event, l, cursor, r, clipboard, shell, viewport);
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        l: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &Rectangle,
        r: &iced::Renderer,
    ) -> iced::mouse::Interaction {
        self.buttons
            .iter()
            .zip(&tree.children)
            .zip(l.children())
            .map(|((b, t), l)| b.as_widget().mouse_interaction(t, l, cursor, viewport, r))
            .find(|i| *i != iced::mouse::Interaction::None)
            .unwrap_or_default()
    }
    fn draw(
        &self,
        tree: &Tree,
        r: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        l: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        clip: &Rectangle,
    ) {
        let plan = tree.state.downcast_ref::<Plan>();
        let b = l.bounds();
        draw_border(self.skin, r, b, *clip);
        if let Some((image, size)) = &self.picture {
            draw_image(
                r,
                image.clone(),
                Rectangle::new(Point::new(b.x + INSET, b.y + INSET), *size),
                *clip,
            );
        }
        for line in &plan.lines {
            if plan.bitmap_text {
                let origin = Point::new(b.x + INSET + line.x, b.y + INSET + line.y);
                let bounds = Rectangle::new(
                    origin,
                    Size::new(plan.width - INSET * 2. - line.x, LINE_HEIGHT),
                );
                if let Some(clip) = clip.intersection(&bounds) {
                    self.skin.font.draw(&line.text, r, origin, clip);
                }
                continue;
            }
            let mut text = description(&line.text).with_content(line.text.clone());
            text.bounds = Size::new(plan.width - INSET * 2. - line.x, LINE_HEIGHT);
            r.fill_text(
                text,
                Point::new(b.x + INSET + line.x, b.y + INSET + line.y),
                Color::from_rgb8(189, 188, 188),
                *clip,
            );
        }
        for ((button, t), l) in self.buttons.iter().zip(&tree.children).zip(l.children()) {
            button.as_widget().draw(t, r, theme, style, l, cursor, clip);
        }
    }
}
pub(super) fn view<'a, M: Clone + 'a>(
    record: &'a Record,
    icons: &'a HudArt,
    response: impl Fn(u64, u32) -> M,
) -> Element<'a, M> {
    let skin = &icons.tutorial_skin;
    let picture = record
        .bitmap_icon
        .and_then(|id| {
            icons
                .bitmaps
                .get(&(record.source.clone(), id))
                .cloned()
                .flatten()
        })
        .map(|(w, h, handle)| (handle, Size::new(w as f32, h as f32)));
    let buttons = record
        .buttons
        .iter()
        .map(|b| {
            button(Element::new(ButtonArt {
                skin,
                label: &b.label,
                minimize: !record.modal(),
            }))
            .padding(0)
            .on_press(response(record.id, b.result))
            .style(|_, _| iced::widget::button::Style {
                background: None,
                ..Default::default()
            })
            .into()
        })
        .collect();
    Element::new(Card {
        record,
        skin,
        picture,
        buttons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn text_regions_match_original_instructions_with_synthetic_font() -> Result<()> {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/tutorial-text-flow.json"))?;
        for case in fixture["cases"].as_array().unwrap() {
            let input = &case["input"];
            let text = input["text"].as_str().unwrap();
            let glyphs = text
                .chars()
                .map(|ch| {
                    let advance = match ch {
                        ' ' => 4.,
                        '\n' | '\r' => 0.,
                        '-' => 6.,
                        _ => 8.,
                    };
                    Glyph {
                        ch,
                        advance,
                        ink: advance,
                    }
                })
                .collect::<Vec<_>>();
            let plan = flow_layout(
                &glyphs,
                input["width"].as_f64().unwrap() as f32,
                Size::new(
                    input["exclude_width"].as_f64().unwrap() as f32,
                    input["exclude_height"].as_f64().unwrap() as f32,
                ),
                input["line_height"].as_f64().unwrap() as f32,
            );
            let native = &case["original"];
            let fields = &native["fields"];
            assert_eq!(
                plan.rounded_height as u64,
                fields["0xd8"].as_u64().unwrap(),
                "{input}"
            );
            assert_eq!(
                plan.consumed as u64,
                fields["0xec"].as_u64().unwrap(),
                "{input}"
            );
            assert_eq!(
                plan.narrow_lines as u64,
                fields["0xf0"].as_u64().unwrap(),
                "{input}"
            );
            assert_eq!(
                plan.tail_lines as u64,
                fields["0x100"].as_u64().unwrap(),
                "{input}"
            );
            assert_eq!(
                plan.height as u64,
                native["body"][3].as_u64().unwrap(),
                "{input}"
            );
            assert_eq!(
                plan.blocks.len(),
                native["blocks"].as_array().unwrap().len(),
                "{input}"
            );
            for (actual, expected) in plan.blocks.iter().zip(native["blocks"].as_array().unwrap()) {
                assert_eq!(
                    actual.start as u64,
                    expected["start"].as_u64().unwrap(),
                    "{input}"
                );
                assert_eq!(
                    actual.length as u64,
                    expected["length"].as_u64().unwrap(),
                    "{input}"
                );
                let rect = [
                    actual.x as i32,
                    actual.y as i32,
                    (actual.x + actual.width) as i32,
                    (actual.y + actual.lines as f32 * input["line_height"].as_f64().unwrap() as f32)
                        as i32,
                ];
                assert_eq!(serde_json::json!(rect), expected["rect"], "{input}");
            }
        }
        Ok(())
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn outer_layout_matches_unchanged_native_extent_corpus() -> Result<()> {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/tutorial-dialog-extent.json"
        ))?;
        for row in data["cases"].as_array().unwrap() {
            let body: [i32; 4] = serde_json::from_value(row["original"]["body"].clone())?;
            let outer: [i32; 4] = serde_json::from_value(row["original"]["outer"].clone())?;
            let primary = row["input"]["primary"].as_bool().unwrap();
            assert_eq!(
                extent(
                    (body[2] - body[0]) as f32,
                    (body[3] - body[1]) as f32,
                    primary
                ),
                Size::new((outer[2] - outer[0]) as f32, (outer[3] - outer[1]) as f32)
            );
            if primary {
                let b: [i32; 4] = serde_json::from_value(row["original"]["button"].clone())?;
                assert_eq!(
                    b,
                    [
                        (outer[2] - outer[0] - 100) / 2,
                        body[3] + 18,
                        (outer[2] - outer[0] - 100) / 2 + 100,
                        body[3] + 51
                    ]
                );
            }
        }
        Ok(())
    }
}
