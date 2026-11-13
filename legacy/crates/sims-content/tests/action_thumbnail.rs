use serde::Deserialize;
use serde_json::{Value, json};
use sims_content::object_catalog::thumbnail::{self, Backend, Prepared, Rect};

#[derive(Clone, Deserialize)]
struct Case {
    output: Option<[i32; 2]>,
    allocate: bool,
    lock: bool,
    prepared: bool,
    cached: bool,
    temporary: bool,
    background_result: bool,
    foreground_result: bool,
    source: Rect,
    rebind: bool,
    #[serde(default)]
    create_failure: bool,
}
struct Host {
    c: Case,
    events: Vec<Value>,
    output: Option<usize>,
    cached: Option<usize>,
    refs: [i32; 4],
    rects: [Rect; 4],
}
impl Host {
    fn new(c: Case) -> Self {
        let output = c.output.map(|_| 3);
        let cached = c.cached.then_some(2);
        let mut rects = [[0; 4]; 4];
        rects[0] = [0, 0, 45, 45];
        rects[1] = c.source;
        let [w, h] = c.output.unwrap_or([45, 45]);
        rects[2] = [0, 0, w, h];
        Self {
            refs: [
                1,
                1 + i32::from(c.cached),
                1 + i32::from(output.is_some()),
                0,
            ],
            c,
            events: vec![],
            output,
            cached,
            rects,
        }
    }
    fn reference(&mut self, image: Option<usize>, change: i32) {
        if let Some(i) = image {
            self.refs[i - 1] += change;
            assert!(self.refs[i - 1] >= 0);
        }
    }
    fn result(self, success: bool) -> Value {
        json!({"success":success,"events":self.events,"output":self.output,"cached":self.cached,"refs":self.refs})
    }
}
impl Backend for Host {
    type Image = usize;
    fn output(&self) -> Option<usize> {
        self.output
    }
    fn replace_output(&mut self, image: Option<usize>) {
        self.reference(image, 1);
        self.reference(self.output, -1);
        self.output = image;
        self.events.push(json!({"kind":"output","image":image}));
    }
    fn allocate(&mut self) -> Option<usize> {
        self.events.push(json!({"kind":"allocate"}));
        self.c.allocate.then_some(4)
    }
    fn create(&mut self, image: usize, size: [i32; 2], format: u32) {
        self.events
            .push(json!({"kind":"create","image":image,"size":size,"format":format}));
        if !self.c.create_failure {
            self.rects[image - 1] = [0, 0, size[0], size[1]];
        }
    }
    fn size(&self, image: usize) -> [i32; 2] {
        let r = self.rects[image - 1];
        [r[2], r[3]]
    }
    fn rectangle(&self, image: usize) -> Rect {
        self.rects[image - 1]
    }
    fn lock(&mut self, image: usize, mode: u32) -> bool {
        self.events
            .push(json!({"kind":"lock","image":image,"mode":mode}));
        self.c.lock
    }
    fn unlock(&mut self, image: usize, mode: u32) {
        self.events
            .push(json!({"kind":"unlock","image":image,"mode":mode}));
    }
    fn background(&self) -> Option<usize> {
        Some(1)
    }
    fn prepare_source(&mut self) -> Prepared<usize> {
        self.events.push(json!({"kind":"prepare"}));
        let image = self.c.temporary.then_some(2);
        self.reference(image, 1);
        Prepared {
            image,
            success: self.c.prepared,
        }
    }
    fn cached_source(&self) -> Option<usize> {
        self.cached
    }
    fn replace_cached(&mut self, image: Option<usize>) {
        self.reference(image, 1);
        self.reference(self.cached, -1);
        self.cached = image;
        self.events.push(json!({"kind":"cache","image":image}));
    }
    fn release(&mut self, image: usize) {
        self.reference(Some(image), -1);
        self.events.push(json!({"kind":"release","image":image}));
    }
    fn blit(&mut self, destination: usize, source: Option<usize>, from: Rect, to: Rect) -> bool {
        self.events.push(
            json!({"kind":"blit","destination":destination,"source":source,"from":from,"to":to}),
        );
        let first = self.events.iter().filter(|e| e["kind"] == "blit").count() == 1;
        if first && self.c.rebind {
            self.reference(Some(3), 1);
            self.reference(self.output, -1);
            self.output = Some(3);
        }
        if first {
            self.c.background_result
        } else {
            self.c.foreground_result
        }
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_caller_over_explicit_image_services() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/native-action-thumbnail.json")).unwrap();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 225);
    for (n, row) in cases.iter().enumerate() {
        let mut host = Host::new(serde_json::from_value(row["input"].clone()).unwrap());
        let success = thumbnail::generate(&mut host).unwrap();
        assert_eq!(host.result(success), row["expected"], "case {n}");
    }
}
