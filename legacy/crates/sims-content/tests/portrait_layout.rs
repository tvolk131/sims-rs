use serde::Deserialize;
use serde_json::{Value, json};
use sims_content::dialog::portrait::{Backend, Layout, Rect, Source, compose};

#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Value,
}
#[derive(Deserialize)]
struct Input {
    settings: [i32; 6],
    faces: Rect,
    background: Rect,
    key: u32,
    allocate: bool,
    create: bool,
    deny_lock: Vec<u32>,
    fill: bool,
    fail_blit: u32,
}
struct Host<'a> {
    input: &'a Input,
    events: Vec<Value>,
    blits: u32,
    output: bool,
}
impl Backend for Host<'_> {
    fn allocate(&mut self) -> bool {
        self.events.push(json!(["allocate", 100]));
        if self.input.allocate {
            self.events.push(json!(["construct"]));
        }
        self.input.allocate
    }
    fn create(&mut self, width: i32, height: i32) -> bool {
        self.events.push(json!(["create", width, height]));
        self.input.create
    }
    fn destroy(&mut self) {
        self.events.push(json!(["destroy", 1]));
    }
    fn lock(&mut self, mode: u32) -> bool {
        self.events.push(json!(["lock", mode]));
        !self.input.deny_lock.contains(&mode)
    }
    fn unlock(&mut self, mode: u32) {
        self.events.push(json!(["unlock", mode]));
    }
    fn fill(&mut self, key: u32) -> bool {
        self.events.push(json!(["fill", key, 0]));
        self.input.fill
    }
    fn color_key(&mut self, key: u32) {
        self.events.push(json!(["key", key]));
    }
    fn blit(&mut self, source: Source, from: Rect, to: Rect) -> bool {
        self.events.push(json!([
            "blit",
            match source {
                Source::Faces => "faces",
                Source::Background => "background",
            },
            from,
            to,
            0
        ]));
        self.blits += 1;
        self.blits != self.input.fail_blit
    }
    fn publish(&mut self) {
        self.output = true;
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn portrait_composition_matches_native_rectangles_order_and_failures() {
    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/portrait-layout.json")).unwrap();
    assert_eq!(fixture.cases.len(), 55);
    for row in fixture.cases {
        let mut host = Host {
            input: &row.input,
            events: vec![],
            blits: 0,
            output: false,
        };
        let returned = compose(
            Layout(row.input.settings),
            row.input.faces,
            row.input.background,
            row.input.key,
            &mut host,
        );
        assert_eq!(
            json!({"returned":returned, "output":host.output,
            "refs":u32::from(host.output), "events":host.events}),
            row.original
        );
    }
}

/// Diagnostic software backend: exact, unscaled source texel copies. This
/// checks asset identity and rectangle coverage, not Windows color-key/blit
/// implementation. In particular no regenerated face pixels are fabricated.
struct Pixels {
    faces: image::RgbaImage,
    background: image::RgbaImage,
    output: image::RgbaImage,
    published: bool,
}
impl Backend for Pixels {
    fn allocate(&mut self) -> bool {
        true
    }
    fn create(&mut self, width: i32, height: i32) -> bool {
        self.output = image::RgbaImage::new(width.try_into().unwrap(), height.try_into().unwrap());
        true
    }
    fn destroy(&mut self) {
        panic!("source composition must create its image")
    }
    fn lock(&mut self, _: u32) -> bool {
        true
    }
    fn unlock(&mut self, _: u32) {}
    fn fill(&mut self, _: u32) -> bool {
        true
    }
    fn color_key(&mut self, _: u32) {}
    fn blit(&mut self, source: Source, from: Rect, to: Rect) -> bool {
        let source = match source {
            Source::Faces => &self.faces,
            Source::Background => &self.background,
        };
        assert_eq!(
            [from[2] - from[0], from[3] - from[1]],
            [to[2] - to[0], to[3] - to[1]]
        );
        assert!(
            from[0] >= 0
                && from[1] >= 0
                && from[2] <= source.width() as i32
                && from[3] <= source.height() as i32
        );
        assert!(
            to[0] >= 0
                && to[1] >= 0
                && to[2] <= self.output.width() as i32
                && to[3] <= self.output.height() as i32
        );
        for y in 0..from[3] - from[1] {
            for x in 0..from[2] - from[0] {
                self.output.put_pixel(
                    (to[0] + x) as u32,
                    (to[1] + y) as u32,
                    *source.get_pixel((from[0] + x) as u32, (from[1] + y) as u32),
                );
            }
        }
        true
    }
    fn publish(&mut self) {
        self.published = true;
    }
}

#[test]
#[ignore = "requires private original character and UI assets"]
fn original_saved_face_assets_fit_native_composition_rectangles() -> anyhow::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = sims_content::Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let template = index
        .resources
        .iter()
        .filter(|r| {
            r.name
                .replace('\\', "/")
                .eq_ignore_ascii_case("CPanel/People/PeopleTemplate.bmp")
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(template.len() == 1);
    let background =
        image::load_from_memory_with_format(&template[0].read()?, image::ImageFormat::Bmp)?
            .to_rgba8();
    anyhow::ensure!(background.dimensions() == (180, 45));
    for (file, label) in [
        ("User00000.iff", "frankie"),
        ("User00001.iff", "sylviamarie"),
    ] {
        let source = format!("UserData2/Characters/{file}");
        let faces = sims_content::dialog::private_bitmap(&index, &source, 2002)?.unwrap();
        anyhow::ensure!(faces.dimensions() == (105, 41));
        let mut host = Pixels {
            faces,
            background: background.clone(),
            output: image::RgbaImage::new(0, 0),
            published: false,
        };
        anyhow::ensure!(compose(
            Layout([45, 45, 5, 2, 35, 41]),
            [0, 0, 105, 41],
            [0, 0, 180, 45],
            0,
            &mut host
        ));
        anyhow::ensure!(host.published && host.output.dimensions() == (180, 135));
        let path = root.join(format!("cache/portrait-source-{label}.png"));
        host.output.save(&path)?;
        println!(
            "{source} BMP_2002 + {} -> {}",
            template[0].locator,
            path.display()
        );
    }
    Ok(())
}
