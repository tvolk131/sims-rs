//!434e23/4ae844 shadow caller. B asset owners are retained; live flags, facing,
//! physical successors, lighting and ground depth come from their current owners.
use super::*;
use sims_sim::lot_presenter::{object_shadow, sprite_full, sprite_gradient, terrain_raster::Data};
use std::collections::BTreeMap;
const INPUT: &str = "386f319179a60a01a3532f97e680eaef45b9c72f246b565d46bd8ff490bc566a";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Object {
    pointer: u32,
    kind: i16,
    guid: u32,
    code: i32,
    strength: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Bitmap {
    node: u32,
    data: u32,
    pitch: i32,
    bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cache {
    pointer: u32,
    width: i32,
    height: i32,
    entries: BTreeMap<u8, Bitmap>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source {
    identity: String,
    objects: BTreeMap<i16, Object>,
    caches: BTreeMap<u32, Cache>,
    manager: u32,
    remap: Vec<u8>,
    tables: Vec<u8>,
    shift: u32,
    mode: u32,
    enabled: bool,
    origin_y: i32,
    half_width: i32,
    vertical: i32,
    row_start: i32,
    rows: BTreeMap<u32, (u32, Vec<i32>)>,
    levels: [i32; 3],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    FirstCall,
    FirstReturned,
    SecondCall,
    SecondReturned,
    QueryCall,
    QueryReturned,
    SurfaceCall,
    SurfaceReturned,
    SelectCall,
    SelectReturned,
    TokenCall,
    TokenReturned,
    BitmapCall,
    BitmapReturned,
    ZoomCall,
    ZoomReturned,
    DepthCall,
    DepthReturned,
    Zoom2Call,
    Zoom2Returned,
    Depth2Call,
    Depth2Returned,
    Zoom3Call,
    Zoom3Returned,
    BaseCall,
    BaseReturned,
    BrightnessCall,
    BrightnessReturned,
    IntensityCall,
    IntensityReturned,
    BlitCall,
    BlitEntry,
    Drawing,
    BlitReturned,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Job {
    stage: Stage,
    id: i16,
    point: [i8; 3],
    rotation: u8,
    code: Option<i32>,
    strength: Option<i32>,
    key: Option<u8>,
    origin: Option<[i32; 2]>,
    zoom: Option<u32>,
    cache: Option<u32>,
    bitmap: Option<(u32, i32)>,
    base: Option<i32>,
    row_pointer: Option<u32>,
    rows: Vec<i32>,
    offset: Option<i32>,
    light: Option<u32>,
    intensity: Option<i32>,
    bias: Option<i32>,
    writes: Vec<sprite_full::Write>,
    next: usize,
}
fn half(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
impl Source {
    pub fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == INPUT,
            "object-shadow B input differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            v["tick"] == 89073
                && v["executes_target_code"] == false
                && v["directory_sha256"]
                    == "f042f68b9f32d7fe96351320812492bc731df7ecee450af9d10c864781b813a9",
            "object-shadow epoch differs"
        );
        let m = Memory::new(&v)?;
        let mut objects = BTreeMap::new();
        for e in v["objects"].as_array().context("shadow instances absent")? {
            let pointer = e["pointer"].as_u64().context("shadow pointer absent")? as u32;
            let o = m.read(pointer, 0x18c)?;
            let id = half(&o, 0xf0);
            let def = m.read(word(&o, 0x118), 0xc0)?;
            let res = m.read(word(&o, 0x11c), 0xb8)?;
            let rd = m.read(word(&res, 0x60), 0xc0)?;
            ensure!(e["id"] == id, "shadow instance alias differs");
            ensure!(
                objects
                    .insert(
                        id,
                        Object {
                            pointer,
                            kind: half(&def, 0x12),
                            guid: word(&def, 0x1c),
                            code: word(&o, 0x184) as i32,
                            strength: i32::from(half(&rd, 0x9e))
                        }
                    )
                    .is_none(),
                "duplicate shadow instance"
            );
        }
        let manager = word(&m.read(0x6efcb4, 4)?, 0);
        let mgr = m.read(manager, 0x40)?;
        let mut caches = BTreeMap::new();
        for e in v["caches"].as_array().context("shadow caches absent")? {
            let zoom = e["zoom"].as_u64().context("shadow cache zoom absent")? as u32;
            ensure!((1..=3).contains(&zoom), "shadow zoom invalid");
            let pointer = word(&mgr, (15 - zoom) as usize * 4);
            let header = m.read(pointer, 0x44)?;
            let sentinel = word(&header, 4);
            let sent = m.read(sentinel, 28)?;
            let mut todo = vec![word(&sent, 4)];
            let mut seen = std::collections::BTreeSet::new();
            let mut entries = BTreeMap::new();
            let width = word(&header, 0x38) as i32;
            let height = word(&header, 0x3c) as i32;
            ensure!(
                width > 0 && width <= 512 && height > 0 && height <= 512,
                "shadow bitmap size invalid"
            );
            while let Some(node) = todo.pop() {
                if node == 0 || node == sentinel {
                    continue;
                }
                ensure!(seen.insert(node) && seen.len() <= 1024, "shadow tree cycle");
                let n = m.read(node, 28)?;
                todo.extend([word(&n, 8), word(&n, 12)]);
                let data = word(&n, 20);
                let pitch = word(&n, 24) as i32;
                ensure!(
                    pitch >= width && pitch <= 512,
                    "shadow bitmap pitch invalid"
                );
                ensure!(
                    entries
                        .insert(
                            n[16],
                            Bitmap {
                                node,
                                data,
                                pitch,
                                bytes: m.read(data, (pitch * height) as usize)?
                            }
                        )
                        .is_none(),
                    "duplicate shadow key"
                );
            }
            ensure!(
                e["pointer"] == pointer
                    && e["sentinel"] == sentinel
                    && e["width"] == width
                    && e["height"] == height,
                "shadow cache alias differs"
            );
            ensure!(
                caches
                    .insert(
                        zoom,
                        Cache {
                            pointer,
                            width,
                            height,
                            entries
                        }
                    )
                    .is_none(),
                "duplicate shadow zoom"
            );
        }
        let depth = m.read(0x09e88960 + 0xb8, 0xb0)?;
        let projection = m.read(0x6f00d0, 16)?;
        let view = m.read(0x09e88960, 0x520)?;
        let row_start = v["row_start"]
            .as_i64()
            .context("shadow row origin absent")? as i32;
        let count = v["row_count"].as_u64().context("shadow row count absent")? as usize;
        ensure!(
            row_start == -512 && count == 2560,
            "shadow row span differs"
        );
        let mut rows = BTreeMap::new();
        for zoom in 1..=3 {
            let pointer = word(&depth, (7 - zoom) as usize * 4);
            let raw = m.read(
                pointer.wrapping_add((row_start as u32).wrapping_mul(4)),
                count * 4,
            )?;
            rows.insert(
                zoom,
                (
                    pointer,
                    raw.chunks_exact(4).map(|b| word(b, 0) as i32).collect(),
                ),
            );
        }
        let display = word(&m.read(0x6e48bc, 4)?, 0);
        let display = m.read(display, 0x40)?;
        let flags = m.read(0x6f0180, 12)?;
        ensure!(
            m.read(0x6683b0, 8)? == 255f64.to_le_bytes()
                && m.read(0x667270, 8)? == 0.5f64.to_le_bytes(),
            "shadow intensity constants differ"
        );
        let s = Self {
            identity: INPUT.into(),
            objects,
            caches,
            manager,
            remap: m.read(0x6efcbc, 1024)?,
            tables: m.read(0x6edf48, 0x10e0)?,
            shift: word(&m.read(0x6c66fc, 4)?, 0),
            mode: word(&display, 0x34),
            enabled: flags[8] != 0,
            origin_y: word(&projection, 4) as i32,
            half_width: word(&view, 0x5c) as i32,
            vertical: word(&view, 0x64) as i32,
            row_start,
            rows,
            levels: std::array::from_fn(|i| word(&depth, 0x84 + i * 4) as i32),
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.identity == INPUT
                && self.caches.len() == 3
                && self.objects.len() == 346
                && self.remap.len() == 1024
                && self.tables.len() == 0x10e0
                && matches!(self.shift, 2 | 3)
                && self.rows.len() == 3,
            "shadow source owner differs"
        );
        Ok(())
    }
    fn rotate(&self, key: u8, rotation: i32) -> Result<u8> {
        if rotation <= 0 {
            return Ok(key);
        }
        self.remap
            .get(rotation as usize * 256 + usize::from(key))
            .copied()
            .context("shadow remap needs an unavailable direction")
    }
    fn query(&self, mut id: i16, sim: &sims_sim::Simulation) -> Result<Option<(i32, i32)>> {
        let mut seen = std::collections::BTreeSet::new();
        while id != 0 {
            ensure!(seen.insert(id), "shadow physical chain cycle");
            let Some(s) = self.objects.get(&id) else {
                return Ok(None);
            };
            let o = sim
                .vm
                .world
                .objects
                .get(&id)
                .context("shadow current object absent")?;
            if o.definition.as_ref().is_none_or(|d| d.guid != s.guid) {
                return Ok(None);
            }
            let data = |k| {
                o.data
                    .get(&k)
                    .copied()
                    .context("shadow live field unobserved")
            };
            if data(34)? == 0 && data(8)? & 4 == 0 && s.kind != 2 && s.kind != 9 && s.code != -1 {
                let key = self.rotate(s.code as u8, i32::from(data(1)?) / 2)?;
                return Ok(Some((i32::from(key), s.strength)));
            }
            let Some(next) = sim
                .native_lot
                .as_ref()
                .and_then(|l| l.placement.links.next.get(&id))
            else {
                return Ok(None);
            };
            id = *next;
        }
        Ok(Some((-1, 0)))
    }
}
impl Job {
    pub fn new(id: i16, point: [i8; 3], rotation: u8) -> Self {
        Self {
            stage: Stage::Entry,
            id,
            point,
            rotation,
            code: None,
            strength: None,
            key: None,
            origin: None,
            zoom: None,
            cache: None,
            bitmap: None,
            base: None,
            row_pointer: None,
            rows: vec![],
            offset: None,
            light: None,
            intensity: None,
            bias: None,
            writes: vec![],
            next: 0,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn store_progress(&self) -> Option<(usize, usize)> {
        (self.stage == Stage::Drawing).then_some((self.next, self.writes.len()))
    }
    pub fn advance(
        &mut self,
        source: &Source,
        placement: &placement::Source,
        sim: &sims_sim::Simulation,
        context: &Frame,
        data: &mut Data,
        clip: [i16; 4],
    ) -> Result<bool> {
        source.validate()?;
        use self::Stage::*;
        let w = context.state().words();
        self.stage = match self.stage {
            Entry => {
                if source.enabled {
                    FirstCall
                } else {
                    Complete
                }
            }
            FirstCall => FirstReturned,
            FirstReturned => {
                if w[22] & 1 != 0 {
                    SecondCall
                } else {
                    Complete
                }
            }
            SecondCall => SecondReturned,
            SecondReturned => {
                if w[22] & 2 != 0 {
                    QueryCall
                } else {
                    Complete
                }
            }
            QueryCall => {
                let Some((code, strength)) = source.query(self.id, sim)? else {
                    return Ok(false);
                };
                self.code = Some(code);
                self.strength = Some(strength);
                QueryReturned
            }
            QueryReturned => {
                if self.code == Some(-1) {
                    Complete
                } else {
                    self.key = Some(source.rotate(
                        self.code.context("shadow code absent")? as u8,
                        i32::from(self.rotation),
                    )?);
                    self.origin = Some([
                        (w[4] as i32).wrapping_sub(source.half_width),
                        (w[5] as i32).wrapping_sub(source.vertical),
                    ]);
                    SurfaceCall
                }
            }
            SurfaceCall => {
                self.zoom = Some(w[8]);
                SurfaceReturned
            }
            SurfaceReturned => SelectCall,
            SelectCall => {
                let Some(c) = source
                    .caches
                    .get(&self.zoom.context("shadow surface zoom absent")?)
                else {
                    return Ok(false);
                };
                self.cache = Some(c.pointer);
                SelectReturned
            }
            SelectReturned => TokenCall,
            TokenCall => TokenReturned,
            TokenReturned => BitmapCall,
            BitmapCall => {
                let c = &source.caches[&self.zoom.context("shadow zoom absent")?];
                let Some(b) = c.entries.get(&self.key.context("shadow key absent")?) else {
                    return Ok(false);
                };
                self.bitmap = Some((b.data, b.pitch));
                BitmapReturned
            }
            BitmapReturned => ZoomCall,
            ZoomCall => ZoomReturned,
            ZoomReturned => DepthCall,
            DepthCall => DepthReturned,
            DepthReturned => {
                let zoom = self.zoom.context("shadow zoom absent")?;
                let origin = self.origin.context("shadow origin absent")?;
                let c = &source.caches[&zoom];
                let (base, rows) = &source.rows[&zoom];
                let at = origin[1]
                    .wrapping_sub(source.origin_y)
                    .wrapping_sub(source.row_start);
                let at = usize::try_from(at)?;
                self.rows = rows
                    .get(at..at + c.height as usize)
                    .context("shadow rows outside B span")?
                    .to_vec();
                self.row_pointer = Some(base.wrapping_add(
                    (origin[1].wrapping_sub(source.origin_y) as u32).wrapping_mul(4),
                ));
                Zoom2Call
            }
            Zoom2Call => Zoom2Returned,
            Zoom2Returned => Depth2Call,
            Depth2Call => Depth2Returned,
            Depth2Returned => Zoom3Call,
            Zoom3Call => Zoom3Returned,
            Zoom3Returned => BaseCall,
            BaseCall => {
                let zoom = self.zoom.context("shadow zoom absent")?;
                let base = placement
                    .ground_depth_unclamped(sim, self.point, self.rotation)?
                    .wrapping_add(
                        source.levels[(3 - zoom) as usize]
                            .wrapping_mul(i32::from(self.point[2]) - 1),
                    );
                self.base = Some(base);
                BaseReturned
            }
            BaseReturned => BrightnessCall,
            BrightnessCall => {
                self.light = Some(sims_sim::lot_presenter::floor_prep::brightness(w));
                BrightnessReturned
            }
            BrightnessReturned => IntensityCall,
            IntensityCall => {
                self.intensity = Some(
                    sprite_gradient::intensity(self.light.context("shadow light absent")?, 0x037f)
                        .map_err(anyhow::Error::msg)?,
                );
                IntensityReturned
            }
            IntensityReturned => {
                self.offset = Some(
                    self.base
                        .context("shadow base absent")?
                        .wrapping_sub(*self.rows.first().context("shadow rows empty")?),
                );
                self.bias = Some(
                    self.strength
                        .context("shadow strength absent")?
                        .wrapping_sub(self.intensity.context("shadow intensity absent")?)
                        .wrapping_add(255),
                );
                BlitCall
            }
            BlitCall => BlitEntry,
            BlitEntry => {
                let c = &source.caches[&self.zoom.context("shadow zoom absent")?];
                let b = &c.entries[&self.key.context("shadow key absent")?];
                self.writes = object_shadow::plan(object_shadow::Input {
                    origin: self.origin.context("shadow origin absent")?,
                    size: [c.width, c.height],
                    mask: &b.bytes,
                    mask_pitch: b.pitch,
                    bias: self.bias.context("shadow bias absent")?,
                    rows: &self.rows,
                    depth_offset: self.offset.context("shadow depth offset absent")?,
                    clip: [
                        clip[1] as i32,
                        clip[0] as i32,
                        clip[3] as i32,
                        clip[2] as i32,
                    ],
                    color: &data.color,
                    depth: &data.depth,
                    pitches: [data.color_pitch, data.depth_pitch],
                    rgb555: source.mode == 1,
                    tables: sprite_gradient::Tables {
                        bytes: &source.tables,
                        green_shift: source.shift,
                    },
                })
                .map_err(anyhow::Error::msg)?;
                Drawing
            }
            Drawing => {
                if let Some(w) = self.writes.get(self.next) {
                    sprite_full::apply(w, &mut data.color, &mut data.depth)
                        .map_err(anyhow::Error::msg)?;
                    self.next += 1;
                    Drawing
                } else {
                    BlitReturned
                }
            }
            BlitReturned => Complete,
            Complete => return Ok(false),
        };
        Ok(true)
    }
    pub fn pc(&self) -> u32 {
        use self::Stage::*;
        match self.stage {
            Entry => 0x4ae83e,
            FirstCall => 0x4ae85f,
            FirstReturned => 0x4ae864,
            SecondCall => 0x4ae86f,
            SecondReturned => 0x4ae874,
            QueryCall => 0x4ae88c,
            QueryReturned => 0x4ae891,
            SurfaceCall => 0x4ae8e8,
            SurfaceReturned => 0x4ae8eb,
            SelectCall => 0x4ae8f2,
            SelectReturned => 0x4ae8f7,
            TokenCall => 0x4ae908,
            TokenReturned => 0x4ae90d,
            BitmapCall => 0x4ae916,
            BitmapReturned => 0x4ae91b,
            ZoomCall => 0x4ae91d,
            ZoomReturned => 0x4ae922,
            DepthCall => 0x4ae927,
            DepthReturned => 0x4ae92c,
            Zoom2Call => 0x4ae950,
            Zoom2Returned => 0x4ae955,
            Depth2Call => 0x4ae95b,
            Depth2Returned => 0x4ae960,
            Zoom3Call => 0x4ae971,
            Zoom3Returned => 0x4ae976,
            BaseCall => 0x4ae981,
            BaseReturned => 0x4ae986,
            BrightnessCall => 0x4ae995,
            BrightnessReturned => 0x4ae99a,
            IntensityCall => 0x4ae9a6,
            IntensityReturned => 0x4ae9ab,
            BlitCall => 0x4ae9d3,
            BlitEntry => 0x4b10c4,
            BlitReturned => 0x4ae9d8,
            Complete => 0x4ae9e4,
            Drawing => 0,
        }
    }
    pub fn compare(
        &self,
        source: &Source,
        q: &serde_json::Value,
        context: &Frame,
        data: &Data,
    ) -> Result<()> {
        ensure!(q["pc"] == self.pc(), "shadow instruction boundary differs");
        let f = &q["ordinary_frame"];
        let raw = crate::original_session::completed_animations::bytes(
            f["raw"].as_str().context("shadow caller frame absent")?,
        )?;
        if self.code.is_some() {
            ensure!(
                word(&raw, 0x230 - 0xb4) as i32 == self.code.unwrap()
                    && word(&raw, 0x230 - 0xc0) as i32 == self.strength.unwrap(),
                "shadow query/strength differs"
            );
        }
        if let Some(origin) = self.origin {
            if self.stage as u32 >= Stage::SurfaceCall as u32 {
                ensure!(
                    word(&raw, 0x230 - 0x44) as i32 == origin[0]
                        && word(&raw, 0x230 + 8) as i32 == origin[1],
                    "shadow origin differs"
                );
            }
        }
        if self.stage == Stage::SurfaceReturned {
            ensure!(
                q["registers"]["eax"] == self.zoom.unwrap(),
                "shadow surface selection differs"
            );
        }
        if self.stage == Stage::SelectReturned {
            ensure!(
                q["registers"]["eax"] == self.cache.unwrap(),
                "shadow cache selection differs"
            );
        }
        if let Some((ptr, pitch)) = self.bitmap {
            if self.stage as u32 >= Stage::BitmapReturned as u32 {
                ensure!(
                    word(&raw, 0x230 - 0xf0) == ptr && word(&raw, 0x230 - 0xec) as i32 == pitch,
                    "shadow bitmap descriptor differs"
                );
            }
        }
        if self.stage == Stage::BaseReturned {
            ensure!(
                q["registers"]["eax"] == self.base.unwrap() as u32,
                "shadow base depth differs"
            );
        }
        if self.stage == Stage::IntensityReturned {
            ensure!(
                q["registers"]["eax"] == self.intensity.unwrap() as u32,
                "shadow intensity differs"
            );
        }
        if matches!(self.stage, Stage::BlitCall | Stage::BlitEntry) {
            let c = &source.caches[&self.zoom.unwrap()];
            let b = &c.entries[&self.key.unwrap()];
            let n = &q["shadow_call"];
            let a = n["arguments"]
                .as_array()
                .context("native shadow arguments absent")?;
            let origin = self.origin.unwrap();
            ensure!(
                a[0] == origin[0] as u32
                    && a[1] == origin[1] as u32
                    && a[3] == self.bias.unwrap() as u32
                    && a[4] == c.width
                    && a[5] == c.height
                    && a[6] == self.row_pointer.unwrap()
                    && a[7] == self.offset.unwrap() as u32,
                "shadow scalar/pointer arguments differ"
            );
            ensure!(
                n["descriptor"] == serde_json::json!([b.data, b.pitch])
                    && crate::original_session::completed_animations::bytes(
                        n["bitmap"]
                            .as_str()
                            .context("native shadow pixels absent")?
                    )? == b.bytes,
                "shadow bitmap bytes differ"
            );
            let rows: Vec<u8> = self.rows.iter().flat_map(|n| n.to_le_bytes()).collect();
            ensure!(
                crate::original_session::completed_animations::bytes(
                    n["rows"].as_str().context("native shadow rows absent")?
                )? == rows
                    && n["display_mode"] == source.mode
                    && n["green_shift"] == source.shift,
                "shadow depth/configuration differs"
            );
            ensure!(
                crate::original_session::completed_animations::bytes(
                    n["tables"]
                        .as_str()
                        .context("native shadow tables absent")?
                )? == source.tables,
                "shadow RGB tables differ"
            );
        }
        let _ = (context, data);
        Ok(())
    }
}
