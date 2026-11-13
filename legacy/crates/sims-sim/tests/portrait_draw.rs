use serde_json::{Value, json};
use sims_formats::avatar::{Blend, Mesh, MeshBinding, Vertex};
use sims_sim::pose::{
    mesh::Pose,
    model::{Bounds, Matrix},
    portrait::draw,
    vertices::{Buffer, Vertex as Published},
};

// Native floating fields are f32. Decode them at that precision, as typed
// fixtures do; comparing Value's intermediate f64 decimal parse adds artifacts.
fn native_floats(value: &Value) -> Value {
    match value {
        Value::Number(n) if n.is_f64() => json!(n.as_f64().unwrap() as f32),
        Value::Array(a) => Value::Array(a.iter().map(native_floats).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), native_floats(v)))
                .collect(),
        ),
        v => v.clone(),
    }
}

struct Host {
    mesh: Mesh,
    poses: Vec<Option<Pose>>,
    buffers: Vec<Buffer>,
    matrices: Vec<Matrix>,
    modes: Vec<u32>,
    cached: bool,
    begin: i32,
    end: i32,
    draws: Vec<i32>,
    draw_count: usize,
    events: Vec<Value>,
}
impl Host {
    fn state(&self, i: usize) -> Value {
        let b = &self.buffers[i];
        json!({"matrix":self.matrices[i],"mode":self.modes[i],"flags":b.flags,"buffer_flags":b.buffer_flags,
            "vertices":b.vertices.iter().map(|v| v.position.iter().chain(&v.normal).chain(&v.uv).copied().collect::<Vec<_>>()).collect::<Vec<_>>(),
            "bounds":b.bounds.minimum.iter().chain(&b.bounds.maximum).copied().collect::<Vec<_>>()})
    }
}
impl draw::Host for Host {
    fn begin(&mut self) -> Result<i32, String> {
        self.events.push(json!({"op":"begin"}));
        Ok(self.begin)
    }
    fn matrix(&mut self, i: usize) -> Result<Matrix, String> {
        Ok(self.matrices[i])
    }
    fn vertices(&mut self, i: usize, rest: bool) -> Result<(), String> {
        self.events
            .push(json!({"op":"vertices","binding":i,"rest":rest}));
        if self.buffers[i].update(
            &self.mesh,
            &self.poses,
            rest,
            self.cached,
            Bounds {
                minimum: [-4., -5., -6.],
                maximum: [7., 8., 9.],
            },
        )? {
            self.events.push(json!({"op":"upload","binding":i}));
        }
        Ok(())
    }
    fn publish_matrix(&mut self, i: usize, m: Matrix) -> Result<(), String> {
        self.matrices[i] = m;
        self.buffers[i].flags |= 2;
        self.events
            .push(json!({"op":"matrix","binding":i,"matrix":m}));
        Ok(())
    }
    fn mode(&mut self, i: usize) -> Result<u32, String> {
        Ok(self.modes[i])
    }
    fn set_mode(&mut self, i: usize, mode: u32) -> Result<(), String> {
        self.modes[i] = mode;
        self.events
            .push(json!({"op":"mode","binding":i,"mode":mode}));
        Ok(())
    }
    fn draw(&mut self, i: usize) -> Result<i32, String> {
        self.events
            .push(json!({"op":"draw","binding":i,"state":self.state(i)}));
        let status = self.draws[self.draw_count];
        self.draw_count += 1;
        Ok(status)
    }
    fn end(&mut self) -> Result<i32, String> {
        self.events.push(json!({"op":"end"}));
        Ok(self.end)
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn native_portrait_draw_restores_each_binding_even_when_draw_or_end_fails() {
    let data: Value = serde_json::from_str(include_str!("fixtures/portrait-draw.json")).unwrap();
    let cases = data["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 36);
    for (case, row) in cases.iter().enumerate() {
        let c = &row["input"];
        let v = &c["vertices"];
        let want = &row["original"];
        let samples: Vec<[f32; 6]> = serde_json::from_value(v["samples"].clone()).unwrap();
        let blends: Vec<[i32; 2]> = serde_json::from_value(v["blends"].clone()).unwrap();
        let primary = samples.len() - blends.len();
        let rows: Vec<[usize; 6]> = serde_json::from_value(v["bindings"].clone()).unwrap();
        let mesh = Mesh {
            name: "native portrait mesh".into(),
            texture: String::new(),
            triangles: vec![],
            bones: vec!["".into(); rows.len()],
            bindings: rows
                .iter()
                .map(|r| MeshBinding {
                    bone: r[0],
                    first_vertex: r[1],
                    vertex_count: r[2],
                    first_blend: r[3],
                    blend_count: r[4],
                })
                .collect(),
            vertices: samples[..primary]
                .iter()
                .map(|v| Vertex {
                    position: v[..3].try_into().unwrap(),
                    normal: v[3..].try_into().unwrap(),
                    uv: [0.125, 0.875],
                    bone: 0,
                })
                .collect(),
            blends: blends
                .iter()
                .zip(&samples[primary..])
                .map(|(b, v)| Blend {
                    vertex: b[1] as usize,
                    weight: b[0] as f32 / 32768.,
                    position: v[..3].try_into().unwrap(),
                    normal: v[3..].try_into().unwrap(),
                    bone: 0,
                })
                .collect(),
        };
        let poses = v["bones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                if p.is_null() {
                    None
                } else {
                    Some(Pose {
                        position: serde_json::from_value(p["position"].clone()).unwrap(),
                        rotation: serde_json::from_value(p["rotation"].clone()).unwrap(),
                        stamp: 0,
                    })
                }
            })
            .collect();
        let mut h = Host {
            mesh,
            poses,
            buffers: vec![],
            matrices: vec![],
            modes: vec![],
            cached: v["cached_drawable"].as_bool().unwrap(),
            begin: c["begin"].as_i64().unwrap() as i32,
            end: c["end"].as_i64().unwrap() as i32,
            draws: serde_json::from_value(c["draws"].clone()).unwrap(),
            draw_count: 0,
            events: vec![],
        };
        for initial in want["initial"].as_array().unwrap() {
            let vertices: Vec<[f32; 8]> =
                serde_json::from_value(initial["vertices"].clone()).unwrap();
            let bounds: [f32; 6] = serde_json::from_value(initial["bounds"].clone()).unwrap();
            h.buffers.push(Buffer {
                vertices: vertices
                    .iter()
                    .map(|v| Published {
                        position: v[..3].try_into().unwrap(),
                        normal: v[3..6].try_into().unwrap(),
                        uv: v[6..].try_into().unwrap(),
                    })
                    .collect(),
                flags: initial["flags"].as_u64().unwrap() as u32,
                buffer_flags: initial["buffer_flags"].as_u64().unwrap() as u32,
                range: [0, primary as i32 - 1],
                bounds: Bounds {
                    minimum: bounds[..3].try_into().unwrap(),
                    maximum: bounds[3..].try_into().unwrap(),
                },
            });
            h.matrices
                .push(serde_json::from_value(initial["matrix"].clone()).unwrap());
            h.modes.push(initial["mode"].as_u64().unwrap() as u32);
        }
        let order: Vec<usize> = serde_json::from_value(c["order"].clone()).unwrap();
        let outcome = draw::render(
            &mut h,
            &order,
            serde_json::from_value(c["matrix"].clone()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            outcome.success,
            want["success"].as_bool().unwrap(),
            "case {case}"
        );
        assert_eq!(outcome.copy, want["copy"].as_bool().unwrap(), "case {case}");
        assert_eq!(
            json!(h.events),
            native_floats(&want["events"]),
            "events case {case}"
        );
        assert_eq!(
            json!((0..h.buffers.len()).map(|i| h.state(i)).collect::<Vec<_>>()),
            native_floats(&want["final"]),
            "state case {case}"
        );
    }
}
