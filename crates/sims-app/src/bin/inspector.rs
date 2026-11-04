use clap::Parser;
use iced::{
    Color, Element, Fill, Font, Size, Task, Theme,
    widget::{
        self, Space, button, column, container, image, pick_list, row, scrollable, slider, text,
        text_input,
    },
    window,
};
use sims_content::{Document, Index};
use sims_formats::sprite::{self, Channel, Preview};
use std::{path::PathBuf, sync::Arc};

#[derive(Parser, Clone)]
#[command(about = "Browse original Sims 1 assets", version)]
struct Args {
    #[arg(long, default_value = "game-data/cabinet-extracted")]
    root: PathBuf,
    #[arg(long, default_value = "DiningChairs.iff")]
    resource: String,
    /// Save a screenshot of this application's rendered window.
    #[arg(long)]
    screenshot: Option<PathBuf>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Preview,
    Interactions,
    Behaviors,
    Chunks,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct Choice {
    id: u16,
    label: String,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}
#[derive(Debug, Clone)]
enum Message {
    Indexed(Result<Arc<Index>, String>),
    Search(String),
    Select(usize),
    Loaded(usize, Result<Arc<Document>, String>),
    Object(Choice),
    Group(Choice),
    View(usize),
    Channel(Channel),
    Scale(f32),
    Tab(Tab),
    Behavior(Choice),
    Jump(u16),
    Frame,
    WindowCaptured(window::Screenshot),
}
struct Inspector {
    args: Args,
    index: Option<Arc<Index>>,
    selected: Option<usize>,
    doc: Option<Arc<Document>>,
    search: String,
    error: Option<String>,
    loading: bool,
    objects: Vec<Choice>,
    object: Option<Choice>,
    groups: Vec<Choice>,
    group: Option<Choice>,
    routines: Vec<Choice>,
    routine: Option<Choice>,
    view_index: usize,
    channel: Channel,
    scale: f32,
    tab: Tab,
    preview: Option<Preview>,
    handle: Option<image::Handle>,
    preview_error: Option<String>,
    frames: u8,
    screenshot_pending: bool,
}
impl Inspector {
    fn new(args: Args) -> (Self, Task<Message>) {
        let root = args.root.clone();
        (
            Self {
                args,
                index: None,
                selected: None,
                doc: None,
                search: "chair".into(),
                error: None,
                loading: true,
                objects: vec![],
                object: None,
                groups: vec![],
                group: None,
                routines: vec![],
                routine: None,
                view_index: 0,
                channel: Channel::Color,
                scale: 3.0,
                tab: Tab::Preview,
                preview: None,
                handle: None,
                preview_error: None,
                frames: 0,
                screenshot_pending: false,
            },
            Task::perform(
                async move {
                    Index::scan(&root)
                        .map(Arc::new)
                        .map_err(|e| format!("{e:#}"))
                },
                Message::Indexed,
            ),
        )
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Indexed(result) => {
                self.loading = false;
                match result {
                    Ok(index) => {
                        let initial = index.find(&self.args.resource);
                        self.index = Some(index);
                        match initial {
                            Ok(id) => return self.select(id),
                            Err(e) => self.error = Some(format!("{e:#}")),
                        }
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Message::Search(value) => self.search = value,
            Message::Select(id) => return self.select(id),
            Message::Loaded(id, result) => {
                if self.selected != Some(id) {
                    return Task::none();
                }
                self.loading = false;
                match result {
                    Ok(doc) => {
                        self.objects = doc
                            .objects
                            .iter()
                            .map(|o| Choice {
                                id: o.id,
                                label: format!("{} · #{}", doc.object_title(o), o.id),
                            })
                            .collect();
                        self.object = self.objects.first().cloned();
                        self.groups = doc
                            .groups
                            .iter()
                            .map(|g| Choice {
                                id: g.id,
                                label: format!("#{} · {}", g.id, g.name),
                            })
                            .collect();
                        self.routines = doc
                            .behaviors
                            .iter()
                            .map(|b| Choice {
                                id: b.id,
                                label: format!("{:#06x} · {}", b.id, b.name),
                            })
                            .collect();
                        self.routine = self
                            .routines
                            .iter()
                            .find(|r| r.id == 4097)
                            .or(self.routines.first())
                            .cloned();
                        self.doc = Some(doc);
                        self.choose_object_group();
                        self.render_preview();
                        self.frames = 0;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Message::Object(choice) => {
                self.object = Some(choice);
                self.choose_object_group();
                self.render_preview();
            }
            Message::Group(choice) => {
                self.group = Some(choice);
                self.view_index = 0;
                self.select_near_view();
                self.render_preview();
            }
            Message::View(index) => {
                self.view_index = index;
                self.render_preview();
            }
            Message::Channel(channel) => {
                self.channel = channel;
                self.refresh_handle();
            }
            Message::Scale(scale) => self.scale = scale,
            Message::Tab(tab) => self.tab = tab,
            Message::Behavior(choice) => self.routine = Some(choice),
            Message::Jump(id) => {
                if let Some(r) = self.routines.iter().find(|r| r.id == id) {
                    self.routine = Some(r.clone());
                    self.tab = Tab::Behaviors;
                } else {
                    self.error = Some(format!(
                        "Routine {id:#06x} is outside this IFF. Select the global/semiglobal resource to inspect it."
                    ));
                }
            }
            Message::Frame => {
                if self.doc.is_some() && !self.loading {
                    self.frames = self.frames.saturating_add(1);
                    if self.frames == 1
                        && self.args.screenshot.is_some()
                        && !self.screenshot_pending
                    {
                        self.screenshot_pending = true;
                        return window::latest()
                            .and_then(window::screenshot)
                            .map(Message::WindowCaptured);
                    }
                }
            }
            Message::WindowCaptured(capture) => {
                if let Some(path) = self.args.screenshot.take() {
                    if let Some(p) = path.parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    match ::image::save_buffer(
                        &path,
                        &capture.rgba,
                        capture.size.width,
                        capture.size.height,
                        ::image::ColorType::Rgba8,
                    ) {
                        Ok(()) => eprintln!("Saved window screenshot: {}", path.display()),
                        Err(e) => self.error = Some(format!("Screenshot: {e}")),
                    }
                }
            }
        }
        Task::none()
    }
    fn select(&mut self, id: usize) -> Task<Message> {
        let Some(index) = self.index.clone() else {
            return Task::none();
        };
        self.selected = Some(id);
        self.loading = true;
        self.doc = None;
        self.error = None;
        self.preview = None;
        self.handle = None;
        Task::perform(
            async move {
                index
                    .document(id)
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"))
            },
            move |doc| Message::Loaded(id, doc),
        )
    }
    fn choose_object_group(&mut self) {
        let id = self
            .doc
            .as_ref()
            .and_then(|d| {
                self.object
                    .as_ref()
                    .and_then(|o| d.objects.iter().find(|v| v.id == o.id))
            })
            .map(|o| o.value.graphic);
        self.group = self
            .groups
            .iter()
            .find(|g| Some(g.id) == id)
            .or(self.groups.first())
            .cloned();
        self.view_index = 0;
        self.select_near_view();
    }
    fn select_near_view(&mut self) {
        if let Some(g) = self.drawing() {
            self.view_index = g.views.iter().position(|v| v.zoom == 3).unwrap_or(0);
        }
    }
    fn drawing(&self) -> Option<&sprite::DrawGroup> {
        let id = self.group.as_ref()?.id;
        self.doc
            .as_ref()?
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| &g.value)
    }
    fn render_preview(&mut self) {
        self.preview = None;
        self.preview_error = None;
        if let (Some(doc), Some(view)) = (
            &self.doc,
            self.drawing().and_then(|g| g.views.get(self.view_index)),
        ) {
            match sprite::render(&doc.iff, view) {
                Ok(p) => self.preview = Some(p),
                Err(e) => self.preview_error = Some(e.to_string()),
            }
        }
        self.refresh_handle();
    }
    fn refresh_handle(&mut self) {
        self.handle = self
            .preview
            .as_ref()
            .map(|p| image::Handle::from_rgba(p.width, p.height, p.channel(self.channel, true)));
    }
    fn view(&self) -> Element<'_, Message> {
        let count = self
            .index
            .as_ref()
            .map_or(0, |i| i.resources.iter().filter(|r| r.is_iff()).count());
        let heading = row![
            column![
                text("SIMS / ARCHIVE INSPECTOR")
                    .size(15)
                    .color(Color::from_rgb8(92, 220, 195)),
                text("The original world, piece by piece.")
                    .size(12)
                    .color(muted())
            ]
            .spacing(5),
            Space::new().width(Fill),
            text(format!("{count} IFF resources  ·  Complete Collection"))
                .size(12)
                .color(muted())
        ]
        .align_y(iced::Alignment::Center);
        let mut list = column![].spacing(3);
        let mut matches = 0;
        if let Some(index) = &self.index {
            let query = self.search.to_ascii_lowercase();
            for (id, res) in index
                .resources
                .iter()
                .enumerate()
                .filter(|(_, r)| r.is_iff() && r.locator.to_ascii_lowercase().contains(&query))
            {
                matches += 1;
                if matches > 100 {
                    continue;
                }
                let label = res.name.rsplit(['\\', '/']).next().unwrap_or(&res.name);
                let origin = res.locator.split('/').next().unwrap_or("");
                let item = column![
                    text(label.to_owned()).size(14),
                    text(origin.to_owned()).size(11).color(muted())
                ]
                .spacing(3);
                list = list.push(
                    button(item)
                        .width(Fill)
                        .padding(10)
                        .style(if self.selected == Some(id) {
                            button::primary
                        } else {
                            button::text
                        })
                        .on_press(Message::Select(id)),
                );
            }
        }
        let sidebar = container(
            column![
                text("RESOURCES").size(11).color(muted()),
                text_input("Find an object or archive…", &self.search)
                    .on_input(Message::Search)
                    .padding(10),
                text(format!(
                    "{matches} matches{}",
                    if matches > 100 {
                        " · first 100 shown"
                    } else {
                        ""
                    }
                ))
                .size(11)
                .color(muted()),
                scrollable(list).height(Fill)
            ]
            .spacing(12),
        )
        .padding(18)
        .width(290)
        .height(Fill)
        .style(panel);
        let mut body = column![].spacing(14);
        if self.loading {
            body = body.push(text("Reading original game data…").size(22));
        }
        if let Some(e) = &self.error {
            body = body.push(
                container(text(e).size(13).color(Color::from_rgb8(255, 185, 126)))
                    .padding(12)
                    .style(panel),
            );
        }
        if let Some(doc) = &self.doc {
            let title = self
                .object
                .as_ref()
                .map(|o| o.label.split(" · #").next().unwrap_or(&o.label))
                .unwrap_or("Resource details");
            body = body
                .push(text(title).size(28))
                .push(text(&doc.source).size(11).color(muted()));
            if !self.objects.is_empty() {
                body = body.push(
                    pick_list(
                        self.objects.as_slice(),
                        self.object.as_ref(),
                        Message::Object,
                    )
                    .width(Fill),
                );
            }
            let mut tabs = row![].spacing(6);
            for (tab, label) in [
                (Tab::Preview, "Graphics"),
                (Tab::Interactions, "Interactions"),
                (Tab::Behaviors, "Behaviors"),
                (Tab::Chunks, "Chunks"),
            ] {
                tabs = tabs.push(
                    button(label)
                        .padding([8, 14])
                        .style(if self.tab == tab {
                            button::primary
                        } else {
                            button::secondary
                        })
                        .on_press(Message::Tab(tab)),
                );
            }
            body = body.push(tabs).push(match self.tab {
                Tab::Preview => self.preview_view(),
                Tab::Interactions => self.interactions_view(),
                Tab::Behaviors => self.behaviors_view(),
                Tab::Chunks => self.chunks_view(),
            });
        } else if !self.loading {
            body = body.push(text("Select an IFF resource to begin."));
        }
        let footer=text(if let Some(doc)=&self.doc{format!("{} chunks  ·  {} objects  ·  {} routines  ·  {} diagnostics  ·  Source assets are read-only",doc.iff.chunks.len(),doc.objects.len(),doc.behaviors.len(),doc.diagnostics.len())}else{format!("Content root: {}",self.args.root.display())}).size(11).color(muted());
        container(column![
            container(heading).padding([16, 22]).style(panel),
            row![
                sidebar,
                container(body).padding(24).width(Fill).height(Fill)
            ]
            .spacing(1)
            .height(Fill),
            container(footer).padding([10, 22]).style(panel)
        ])
        .width(Fill)
        .height(Fill)
        .into()
    }
    fn preview_view(&self) -> Element<'_, Message> {
        let mut content = column![].spacing(12);
        if !self.groups.is_empty() {
            content = content.push(
                row![
                    text("Drawgroup").size(12),
                    pick_list(self.groups.as_slice(), self.group.as_ref(), Message::Group)
                        .width(240),
                    Space::new().width(Fill),
                    text(format!("Display ×{:.1}", self.scale)).size(12),
                    slider(1.0..=5.0, self.scale, Message::Scale)
                        .step(0.5_f32)
                        .width(120)
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center),
            );
        }
        if let Some(group) = self.drawing()
            && let Some(current) = group.views.get(self.view_index)
        {
            let mut directions = row![text("Direction").size(12)]
                .spacing(6)
                .align_y(iced::Alignment::Center);
            for d in [1, 4, 16, 64] {
                if let Some(n) = group
                    .views
                    .iter()
                    .position(|v| v.direction == d && v.zoom == current.zoom)
                {
                    directions = directions.push(
                        button(text(format!("{d:02X}")).size(12))
                            .on_press(Message::View(n))
                            .style(if current.direction == d {
                                button::primary
                            } else {
                                button::secondary
                            }),
                    );
                }
            }
            directions = directions
                .push(Space::new().width(12))
                .push(text("Original zoom").size(12));
            for (z, name) in [(1, "Far"), (2, "Medium"), (3, "Near")] {
                if let Some(n) = group
                    .views
                    .iter()
                    .position(|v| v.zoom == z && v.direction == current.direction)
                {
                    directions = directions.push(button(name).on_press(Message::View(n)).style(
                        if current.zoom == z {
                            button::primary
                        } else {
                            button::secondary
                        },
                    ));
                }
            }
            content = content.push(directions);
        }
        let mut channels = row![].spacing(6);
        for c in [Channel::Color, Channel::Alpha, Channel::Depth] {
            channels = channels.push(
                button(text(c.to_string()).size(12))
                    .padding([6, 16])
                    .on_press(Message::Channel(c))
                    .style(if self.channel == c {
                        button::primary
                    } else {
                        button::secondary
                    }),
            );
        }
        content = content.push(channels);
        if let (Some(handle), Some(p)) = (&self.handle, &self.preview) {
            let picture = image(handle.clone())
                .filter_method(widget::image::FilterMethod::Nearest)
                .width(p.width as f32 * self.scale)
                .height(p.height as f32 * self.scale);
            content = content.push(
                container(
                    scrollable(container(picture).center_x(Fill).padding(24)).direction(
                        scrollable::Direction::Both {
                            vertical: scrollable::Scrollbar::default(),
                            horizontal: scrollable::Scrollbar::default(),
                        },
                    ),
                )
                .width(Fill)
                .height(Fill)
                .style(stage),
            );
            let description = match self.channel {
                Channel::Color => "Original palette colors over a checkerboard",
                Channel::Alpha => "White = opaque · black = transparent",
                Channel::Depth => "Raw depth bytes · white = 255 / background",
            };
            content = content.push(
                text(format!(
                    "{} × {} px  ·  origin ({}, {})  ·  {description}",
                    p.width, p.height, p.origin[0], p.origin[1]
                ))
                .size(12)
                .color(muted()),
            );
            for note in &p.notes {
                content = content.push(text(note).size(11).color(muted()));
            }
        } else {
            content = content.push(
                container(text(
                    self.preview_error
                        .as_deref()
                        .unwrap_or("This resource has no supported drawing group."),
                ))
                .padding(30)
                .height(Fill)
                .width(Fill)
                .style(stage),
            );
        }
        content.height(Fill).into()
    }
    fn interactions_view(&self) -> Element<'_, Message> {
        let doc = self.doc.as_ref().unwrap();
        let selected_table = self
            .object
            .as_ref()
            .and_then(|o| doc.objects.iter().find(|v| v.id == o.id))
            .map(|o| o.value.table);
        let mut items = column![
            text("Original interaction menu").size(18),
            text("Eligibility checks and autonomy values are displayed, not executed.")
                .size(12)
                .color(muted())
        ]
        .spacing(14);
        for table in doc
            .tables
            .iter()
            .filter(|t| selected_table.is_none_or(|id| t.id == id))
        {
            let strings = doc.strings(b"TTAs", table.id);
            for entry in &table.value.interactions {
                let name = strings
                    .as_ref()
                    .ok()
                    .and_then(|t| t.get(entry.string_index as usize, 1))
                    .unwrap_or("[missing string]");
                let action = doc
                    .behavior_name(entry.action)
                    .unwrap_or_else(|| "external/unresolved".into());
                let test = doc
                    .behavior_name(entry.test)
                    .unwrap_or_else(|| "external/unresolved".into());
                items = items.push(
                    container(
                        column![
                            text(name.to_owned()).size(24),
                            text(format!(
                                "TTAB #{} · version {} · flags {:#010x} · autonomy {}",
                                table.id, table.value.version, entry.flags, entry.autonomy
                            ))
                            .size(12)
                            .color(muted()),
                            button(
                                text(format!("Action {:#06x} — {action}", entry.action)).size(14)
                            )
                            .on_press(Message::Jump(entry.action))
                            .style(button::secondary),
                            button(text(format!("Check {:#06x} — {test}", entry.test)).size(14))
                                .on_press(Message::Jump(entry.test))
                                .style(button::secondary)
                        ]
                        .spacing(12),
                    )
                    .padding(20)
                    .width(Fill)
                    .style(panel),
                );
            }
        }
        if doc.tables.is_empty() {
            items = items.push(text("No interaction tables in this resource."));
        }
        scrollable(items).height(Fill).into()
    }
    fn behaviors_view(&self) -> Element<'_, Message> {
        let doc = self.doc.as_ref().unwrap();
        let mut content=column![text("SimAntics disassembly").size(18),text("Branch targets and eight operand bytes are preserved. Primitive semantics are not executed.").size(12).color(muted())].spacing(12);
        if !self.routines.is_empty() {
            content = content.push(
                pick_list(
                    self.routines.as_slice(),
                    self.routine.as_ref(),
                    Message::Behavior,
                )
                .width(Fill),
            );
        }
        if let Some(routine) = self
            .routine
            .as_ref()
            .and_then(|r| doc.behaviors.iter().find(|b| b.id == r.id))
        {
            content = content.push(
                scrollable(
                    container(
                        text(routine.value.disassemble(|id| doc.behavior_name(id)))
                            .font(Font::MONOSPACE)
                            .size(13),
                    )
                    .padding(18)
                    .width(Fill)
                    .style(stage),
                )
                .height(Fill),
            );
        } else {
            content = content.push(text("No behaviors in this resource."));
        }
        content.height(Fill).into()
    }
    fn chunks_view(&self) -> Element<'_, Message> {
        let doc = self.doc.as_ref().unwrap();
        let mut content = column![
            text("IFF resource directory").size(18),
            text("Chunk IDs are local to this file. Unknown chunk payloads remain intact.")
                .size(12)
                .color(muted())
        ]
        .spacing(8);
        for error in &doc.diagnostics {
            content = content.push(text(error).size(12).color(Color::from_rgb8(255, 185, 126)));
        }
        for c in &doc.iff.chunks {
            content = content.push(
                text(format!(
                    "{}  #{:<5}  {:>8} bytes  @ {:08X}  {}",
                    c.kind(),
                    c.id,
                    c.payload.len(),
                    c.offset,
                    c.label
                ))
                .font(Font::MONOSPACE)
                .size(12),
            );
        }
        scrollable(content).height(Fill).into()
    }
}
fn muted() -> Color {
    Color::from_rgb8(153, 171, 185)
}
fn panel(_: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(24, 32, 42))
        .border(iced::Border::default().rounded(6))
}
fn stage(_: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(13, 20, 28))
        .border(iced::Border::default().rounded(8))
}
fn main() -> iced::Result {
    let mut args = Args::parse();
    // A locally packaged app can be launched from Finder with a different cwd.
    if args.root == std::path::Path::new("game-data/cabinet-extracted")
        && !args.root.is_dir()
        && let Ok(exe) = std::env::current_exe()
        && let Some(root) = exe
            .ancestors()
            .map(|p| p.join(&args.root))
            .find(|p| p.is_dir())
    {
        args.root = root;
    }
    iced::application(
        move || Inspector::new(args.clone()),
        Inspector::update,
        Inspector::view,
    )
    .title("Sims Archive Inspector")
    .theme(|_: &Inspector| {
        Theme::custom(
            "Archive",
            iced::theme::Palette {
                background: Color::from_rgb8(17, 24, 33),
                text: Color::from_rgb8(230, 238, 241),
                primary: Color::from_rgb8(54, 166, 148),
                success: Color::from_rgb8(88, 185, 125),
                warning: Color::from_rgb8(236, 174, 95),
                danger: Color::from_rgb8(235, 112, 112),
            },
        )
    })
    .subscription(|s: &Inspector| {
        if s.args.screenshot.is_some() {
            window::frames().map(|_| Message::Frame)
        } else {
            iced::Subscription::none()
        }
    })
    .window_size(Size::new(1240.0, 850.0))
    .run()
}
