use serde::Deserialize;
use serde_json::{Value, json};
use sims_content::dialog::portrait::{
    Layout, Rect,
    generators::{self, Backend, Render, Rendered},
};
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Input {
    slot: usize,
    format: u32,
    settings: [i32; 6],
    key: u32,
    allocate: bool,
    create: bool,
    template: bool,
    initial: bool,
    initial_rect: Rect,
    template_rect: Rect,
    render_rect: Rect,
    sample: u16,
    deny_lock: Vec<[u32; 2]>,
    fail_render: u32,
    null_render: u32,
    fail_blit: u32,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Value,
}
#[derive(Deserialize)]
struct Report {
    cases: Vec<Case>,
}
struct Host<'a> {
    input: &'a Input,
    events: Vec<Value>,
    refs: [u32; 9],
    rects: BTreeMap<u32, Rect>,
    renders: u32,
    blits: u32,
    output: Option<u32>,
}
impl<'a> Host<'a> {
    fn new(input: &'a Input) -> Self {
        let mut refs = [0; 9];
        refs[0] = u32::from(input.initial);
        Self {
            input,
            events: vec![],
            refs,
            rects: [(1, input.initial_rect), (3, input.template_rect)]
                .into_iter()
                .collect(),
            renders: 0,
            blits: 0,
            output: input.initial.then_some(1),
        }
    }
}
impl Backend for Host<'_> {
    type Image = u32;
    fn allocate(&mut self) -> Option<u32> {
        self.events.push(json!(["allocate", 100]));
        if self.input.allocate {
            self.events.push(json!(["construct", 2]));
            Some(2)
        } else {
            None
        }
    }
    fn create(&mut self, image: u32, width: i32, height: i32) -> bool {
        self.events.push(json!(["create", image, width, height]));
        self.rects.insert(image, [0, 0, width, height]);
        self.input.create
    }
    fn destroy(&mut self, image: u32) {
        self.events.push(json!(["destroy", image]));
    }
    fn template(&mut self, resource: u16) -> Option<u32> {
        self.events.push(json!(["template", resource]));
        self.input.template.then_some(3)
    }
    fn lock(&mut self, image: u32, mode: u32) -> bool {
        self.events.push(json!(["lock", image, mode]));
        !self.input.deny_lock.contains(&[image, mode])
    }
    fn unlock(&mut self, image: u32, mode: u32) {
        self.events.push(json!(["unlock", image, mode]));
    }
    fn color_key(&mut self, image: u32, color: u32) {
        self.events.push(json!(["key", image, color]));
    }
    fn first_pixel(&mut self, image: u32, query_stride: bool) -> u16 {
        if query_stride {
            self.events.push(json!(["stride", image]));
        }
        self.events.push(json!(["pixels", image]));
        self.input.sample
    }
    fn rect(&self, image: u32) -> Rect {
        self.rects[&image]
    }
    fn render(&mut self, request: Render) -> Rendered<u32> {
        self.events.push(json!(["render", request]));
        self.renders += 1;
        let image = (self.renders != self.input.null_render).then_some(self.renders + 3);
        if let Some(i) = image {
            self.refs[i as usize - 1] = 1;
            let [x, y, dx, dy] = self.input.render_rect;
            self.rects.insert(
                i,
                [x, y, x + request.target[0] + dx, y + request.target[1] + dy],
            );
        }
        Rendered {
            image,
            success: self.renders != self.input.fail_render,
        }
    }
    fn blit(&mut self, destination: u32, source: u32, from: Rect, to: Rect) -> bool {
        self.events
            .push(json!(["blit", destination, source, from, to, 0]));
        self.blits += 1;
        self.blits != self.input.fail_blit
    }
    fn release(&mut self, image: u32) {
        self.events.push(json!(["release", image]));
        self.refs[image as usize - 1] -= 1;
        if self.refs[image as usize - 1] == 0 {
            self.destroy(image);
        }
    }
    fn publish(&mut self, image: u32) {
        self.events.push(json!(["publish", image]));
        self.refs[image as usize - 1] += 1;
        if let Some(old) = self.output {
            self.release(old);
        }
        self.output = Some(image);
    }
    fn output(&self) -> Option<u32> {
        self.output
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn portrait_generators_match_original_layouts_failures_and_ownership() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/portrait-generators.json")).unwrap();
    assert_eq!(report.cases.len(), 650);
    for (n, mut case) in report.cases.into_iter().enumerate() {
        let c = &case.input;
        let mut host = Host::new(c);
        let returned = generators::generate(
            c.slot,
            Layout(c.settings),
            c.format,
            c.key,
            0x12345678,
            &mut host,
        )
        .unwrap();
        // Original x87 results were stored as f32. Normalize JSON's decimal
        // representation through that type, retaining exact f32 comparison.
        for event in case.original["events"].as_array_mut().unwrap() {
            if event[0] == "render" {
                event[1] = json!(serde_json::from_value::<Render>(event[1].clone()).unwrap());
            }
        }
        assert_eq!(
            json!({"returned":returned,"output":host.output,"refs":host.refs,"events":host.events}),
            case.original,
            "case {n}, slot {}",
            c.slot
        );
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn thumbnail_requires_the_callers_existing_image() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/portrait-generators.json")).unwrap();
    let mut c = report
        .cases
        .into_iter()
        .find(|c| c.input.slot == 3)
        .unwrap()
        .input;
    c.initial = false;
    let mut host = Host::new(&c);
    assert!(
        generators::generate(
            3,
            Layout(c.settings),
            c.format,
            c.key,
            0x12345678,
            &mut host
        )
        .is_err()
    );
    assert!(host.events.is_empty());
}
