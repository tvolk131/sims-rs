//!437fdd positive slots: borrowed registry, recursive draw and ordered prefix copies.
use super::*;
// Full producer digest is kept separate from the human-readable function name.
const SLOT_INPUT: &str = "3e5f58fa9421b81bac51a5ce6406a43153469ce60264295769cbb451fa031068";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Slots {
    identity: String,
    records: BTreeMap<i16, Vec<[u32; 9]>>,
}
impl Slots {
    pub fn import(encoded: &[u8], directory: &[u8]) -> Result<Self> {
        let identity = sims_fixtures::digest(encoded);
        ensure!(
            identity == SLOT_INPUT && sims_fixtures::digest(directory) == DIRECTORY,
            "contained record source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        let d: serde_json::Value = serde_json::from_slice(directory)?;
        ensure!(
            v["tick"] == 89073
                && v["executes_target_code"] == false
                && v["directory_sha256"] == DIRECTORY,
            "contained record epoch differs"
        );
        let memory = Memory::new(&v)?;
        let mut records = BTreeMap::new();
        for obj in d["objects"].as_array().context("slot directory absent")? {
            let id = obj["id"].as_i64().context("slot ID absent")? as i16;
            let pointer = obj["pointer"].as_u64().context("slot pointer absent")? as u32;
            let raw = memory.read(pointer, 0x18c)?;
            ensure!(
                crate::original_session::completed_animations::bytes(
                    obj["raw"].as_str().context("slot original record absent")?
                )? == raw,
                "contained B object differs"
            );
            let begin = word(&raw, 0x130);
            let end = word(&raw, 0x134);
            ensure!(
                end >= begin && (end - begin) % 36 == 0 && end - begin <= 36 * 256,
                "contained vector extent differs"
            );
            let bytes = memory.read(begin, (end - begin) as usize)?;
            let values = bytes
                .chunks_exact(36)
                .map(|r| std::array::from_fn(|n| word(r, n * 4)))
                .collect();
            ensure!(
                records.insert(id, values).is_none(),
                "duplicate contained owner"
            );
        }
        let value = Self { identity, records };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.identity == SLOT_INPUT
                && self.records.len() <= 1024
                && self
                    .records
                    .iter()
                    .all(|(id, rows)| *id > 0 && rows.len() <= 256),
            "contained record ownership/extent differs"
        );
        Ok(())
    }
    pub fn get(&self, parent: i16, index: usize, child: i16) -> Result<[u32; 3]> {
        ensure!(
            self.identity == SLOT_INPUT,
            "contained record provenance differs"
        );
        let r = self
            .records
            .get(&parent)
            .and_then(|v| v.get(index + 1))
            .context("contained logical record absent")?;
        ensure!(
            r[4] as i16 == child,
            "contained logical ID changed without its record owner"
        );
        Ok([r[0], r[1], r[2]])
    }
    pub fn compare_child(&self, parent: i16, index: usize, child: i16) -> Result<()> {
        self.get(parent, index, child)?;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    LookupCall,
    LookupEntry,
    LookupReturned,
    Positive,
    Backup(u8),
    BackedUp,
    RotateCall,
    Rotated,
    Translate(u8),
    ProjectCall,
    Project(u8),
    Projected,
    CacheRead,
    CacheReadReturned,
    CacheClear,
    CacheCleared,
    DrawCall,
    Drawing,
    DrawReturned,
    Union,
    UnionReturned,
    Invalid,
    RestoreCache,
    RestoreCacheReturned,
    ClearCache,
    ClearCacheReturned,
    RestoreSetup,
    Restore(u8),
    Restored,
    Output,
    Copy(u8),
    Return,
    Returned,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Positive {
    id: i16,
    local: [u32; 3],
    stage: Stage,
    pointer: Option<u32>,
    backup: [Option<u32>; 6],
    rotated: Option<[u32; 2]>,
    projected: Option<[i32; 3]>,
    depth: Option<PlacementUpdate>,
    old_rect: Option<[i32; 4]>,
    draw_rect: Option<[i32; 4]>,
    rectangle: [i32; 4],
    output: [Option<i32>; 4],
    child_source: Option<Source>,
    child_cursor: Option<Box<Cursor>>,
}
impl Positive {
    pub fn new(id: i16, local: [u32; 3]) -> Self {
        Self {
            id,
            local,
            stage: Stage::Entry,
            pointer: None,
            backup: [None; 6],
            rotated: None,
            projected: None,
            depth: None,
            old_rect: None,
            draw_rect: None,
            rectangle: sims_sim::lot_presenter::contained_slot::EMPTY,
            output: [None; 4],
            child_source: None,
            child_cursor: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Returned
    }
    pub fn result(&self) -> Option<[i32; 4]> {
        self.complete().then_some(self.rectangle)
    }
    pub fn drawing(&self) -> Option<&Cursor> {
        (self.stage == Stage::Drawing)
            .then(|| self.child_cursor.as_deref())
            .flatten()
    }
    pub fn pc(&self) -> u32 {
        use self::Stage::*;
        match self.stage {
            Entry => 0x437fdd,
            LookupCall => 0x438008,
            LookupEntry => 0x40743b,
            LookupReturned => 0x43800d,
            Positive => 0x438018,
            Backup(_) => 0x438024,
            BackedUp => 0x438026,
            RotateCall => 0x438044,
            Rotated => 0x438049,
            Translate(_) => 0,
            ProjectCall => 0x438068,
            Project(_) => 0,
            Projected => 0x43806d,
            CacheRead => 0x43807a,
            CacheReadReturned => 0x43807f,
            CacheClear => 0x438082,
            CacheCleared => 0x438087,
            DrawCall => 0x438090,
            Drawing => self.child_cursor.as_ref().map_or(0, |c| c.pc()),
            DrawReturned => 0x438093,
            Union => 0x4380a9,
            UnionReturned => 0x4380ae,
            Invalid => 0x4380b0,
            RestoreCache => 0x4380c7,
            RestoreCacheReturned => 0x4380cc,
            ClearCache => 0x4380d1,
            ClearCacheReturned => 0x4380d6,
            RestoreSetup => 0x4380d6,
            Restore(_) => 0x4380de,
            Restored => 0x4380e0,
            Output => 0x4380e1,
            Copy(n) => 0x4380e9 + u32::from(n),
            Return => 0x4380f0,
            Returned => 0x437cc2,
        }
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.id > 0 && self.local.iter().all(|v| f32::from_bits(*v).is_finite()),
            "invalid positive slot input"
        );
        ensure!(
            !matches!(self.stage,Stage::Backup(n)|Stage::Restore(n)if n>=6)
                && !matches!(self.stage,Stage::Copy(n)if n>=4)
                && !matches!(self.stage,Stage::Translate(n)if n>=3)
                && !matches!(self.stage,Stage::Project(n)if n>4),
            "positive slot copy/project cursor outside storage"
        );
        ensure!(
            self.child_cursor.is_some() == self.child_source.is_some(),
            "positive draw lost source or cursor"
        );
        if self.stage == Stage::Drawing {
            ensure!(
                self.child_cursor.is_some(),
                "positive drawing cursor absent"
            );
        }
        let n = match self.stage {
            Stage::Copy(n) => n as usize,
            Stage::Return | Stage::Returned => 4,
            _ => 0,
        };
        ensure!(
            self.output
                .iter()
                .enumerate()
                .all(|(i, v)| *v == (i < n).then_some(self.rectangle[i])),
            "positive output word lifetime differs"
        );
        Ok(())
    }
    pub fn advance(
        &mut self,
        parent: &Source,
        sim: &sims_sim::Simulation,
        context: &mut Frame,
        classify: &mut classification::State,
        clip: [i16; 4],
        raster: Option<(
            &mut raster::Source,
            &mut sims_sim::lot_presenter::terrain_raster::Data,
        )>,
        catalog: Option<&Catalog>,
        directory: Option<&ordinary_objects::Source>,
        placement: Option<&mut placement::Source>,
        rotation: u8,
    ) -> Result<bool> {
        use self::Stage::*;
        self.validate()?;
        self.stage = match self.stage {
            Entry => LookupCall,
            LookupCall => LookupEntry,
            LookupEntry => {
                let Some(directory) = directory else {
                    return Ok(false);
                };
                let Some(pointer) = directory.lookup(self.id, sim)? else {
                    return Ok(false);
                };
                self.pointer = Some(pointer);
                LookupReturned
            }
            LookupReturned => {
                if self.pointer == Some(0) {
                    Output
                } else {
                    Positive
                }
            }
            Positive => Backup(0),
            Backup(n) => {
                self.backup[n as usize] = Some(context.state().words()[n as usize]);
                if n == 5 { BackedUp } else { Backup(n + 1) }
            }
            BackedUp => RotateCall,
            RotateCall => {
                let facing = sim
                    .vm
                    .world
                    .objects
                    .get(&parent.id)
                    .and_then(|o| o.data.get(&1))
                    .copied()
                    .context("contained parent facing absent")?
                    & 7;
                let [x, y] = [self.local[0], self.local[1]].map(f32::from_bits);
                self.rotated = Some(match facing {
                    0 => [x.to_bits(), y.to_bits()],
                    2 => [(-y).to_bits(), x.to_bits()],
                    4 => [(-x).to_bits(), (-y).to_bits()],
                    6 => [y.to_bits(), (-x).to_bits()],
                    _ => [0, 0],
                });
                Rotated
            }
            Rotated => Translate(0),
            Translate(n) => {
                let index = n as usize;
                let offset = if n == 2 {
                    self.local[2]
                } else {
                    self.rotated.context("rotated offset absent")?[index]
                };
                let value = f32::from_bits(context.state().words()[index]) + f32::from_bits(offset);
                ensure!(value.is_finite(), "contained translation indefinite");
                context
                    .store_prefix_word(index, value.to_bits())
                    .map_err(anyhow::Error::msg)?;
                if n == 2 {
                    ProjectCall
                } else {
                    Translate(n + 1)
                }
            }
            ProjectCall => Project(0),
            Project(n) => {
                let Some(placement) = placement else {
                    return Ok(false);
                };
                match n {
                    0 => {
                        let (p, adjust) = placement.project_contained(sim, context, rotation)?;
                        self.projected = Some([p[0], p[1], adjust]);
                        Project(1)
                    }
                    1 => {
                        context
                            .store_prefix_word(
                                4,
                                self.projected.context("contained projection absent")?[0] as u32,
                            )
                            .map_err(anyhow::Error::msg)?;
                        Project(2)
                    }
                    2 => {
                        context
                            .store_prefix_word(
                                5,
                                self.projected.context("contained projection absent")?[1] as u32,
                            )
                            .map_err(anyhow::Error::msg)?;
                        Project(3)
                    }
                    3 => {
                        let p = self.projected.context("contained projection absent")?;
                        context
                            .store_prefix_word(5, p[1].wrapping_sub(p[2]) as u32)
                            .map_err(anyhow::Error::msg)?;
                        self.depth = Some(PlacementUpdate::depth_from_context());
                        Project(4)
                    }
                    _ => {
                        let depth = self
                            .depth
                            .as_mut()
                            .context("contained depth cursor absent")?;
                        ensure!(
                            placement.advance(depth, sim, context)?,
                            "contained depth stopped"
                        );
                        if depth.complete() {
                            Projected
                        } else {
                            Project(4)
                        }
                    }
                }
            }
            Projected => CacheRead,
            CacheRead => {
                self.old_rect = Some(classify.cached_rectangles(self.id)?[0]);
                CacheReadReturned
            }
            CacheReadReturned => CacheClear,
            CacheClear => {
                classify.clear_cached_rectangle(self.id, 1)?;
                CacheCleared
            }
            CacheCleared => DrawCall,
            DrawCall => {
                let Some(source) =
                    catalog.and_then(|c| c.select(self.id, self.pointer.unwrap_or(0), sim))
                else {
                    return Ok(false);
                };
                let mut cursor = source.new_draw();
                cursor.return_pc = Some(0x438093);
                self.child_source = Some(source);
                self.child_cursor = Some(Box::new(cursor));
                Drawing
            }
            Drawing => {
                let cursor = self
                    .child_cursor
                    .as_mut()
                    .context("contained child cursor absent")?;
                if !cursor.advance(
                    self.child_source
                        .as_ref()
                        .context("contained drawing source absent")?,
                    sim,
                    context,
                    classify,
                    clip,
                    raster,
                    catalog,
                    directory,
                    placement,
                    rotation,
                )? {
                    return Ok(false);
                }
                if cursor.complete() {
                    self.draw_rect = cursor.result();
                    DrawReturned
                } else {
                    Drawing
                }
            }
            DrawReturned => {
                let r = self.draw_rect.context("contained result absent")?;
                if r[0] < r[2] && r[1] < r[3] {
                    Union
                } else {
                    Invalid
                }
            }
            Union => {
                let r = self.draw_rect.context("contained union result absent")?;
                self.rectangle = [
                    self.rectangle[0].min(r[0]),
                    self.rectangle[1].min(r[1]),
                    self.rectangle[2].max(r[2]),
                    self.rectangle[3].max(r[3]),
                ];
                UnionReturned
            }
            UnionReturned => RestoreSetup,
            Invalid => {
                let r = self.old_rect.context("contained old cache absent")?;
                if r[0] < r[2] && r[1] < r[3] {
                    RestoreCache
                } else {
                    ClearCache
                }
            }
            RestoreCache => {
                classify.store_cached_rectangle(
                    self.id,
                    1,
                    self.old_rect.context("contained old cache absent")?,
                )?;
                RestoreCacheReturned
            }
            RestoreCacheReturned => RestoreSetup,
            ClearCache => {
                classify.clear_cached_rectangle(self.id, 1)?;
                ClearCacheReturned
            }
            ClearCacheReturned => Restore(0),
            RestoreSetup => Restore(0),
            Restore(n) => {
                context
                    .store_prefix_word(
                        n as usize,
                        self.backup[n as usize].context("contained backup word absent")?,
                    )
                    .map_err(anyhow::Error::msg)?;
                if n == 5 { Restored } else { Restore(n + 1) }
            }
            Restored => Output,
            Output => Copy(0),
            Copy(n) => {
                self.output[n as usize] = Some(self.rectangle[n as usize]);
                if n == 3 { Return } else { Copy(n + 1) }
            }
            Return => Returned,
            Returned => return Ok(false),
        };
        self.validate()?;
        Ok(true)
    }
    pub fn compare_auxiliary(&self,q:&serde_json::Value,catalog:&Catalog)->Result<()>{if let Some(cursor)=self.drawing(){cursor.compare_auxiliary(q,self.child_source.as_ref().context("contained drawing source absent")?,catalog)?;}Ok(())}
    pub fn matches_observation(&self, q: &serde_json::Value) -> Result<bool> {
        if let Some(c) = self.drawing() {
            return c.matches_observation(q);
        }
        if self.pc() != q["pc"].as_u64().unwrap_or(0) as u32 {
            return Ok(false);
        }
        if let Stage::Backup(n) | Stage::Restore(n) = self.stage {
            return Ok(q["registers"]["ecx"] == 6 - u32::from(n));
        }
        Ok(true)
    }
    pub fn compare(&self, q: &serde_json::Value, parent: &Source) -> Result<()> {
        self.validate()?;
        if let Some(c) = self.drawing() {
            return c.compare(
                q,
                self.child_source
                    .as_ref()
                    .context("contained child source absent")?,
            );
        }
        ensure!(
            self.matches_observation(q)?,
            "positive contained boundary differs"
        );
        if self.stage == Stage::Entry || self.complete() {
            return Ok(());
        }
        if q["slot_lookup"].is_object() {
            let (_, _, owner) = parent
                .slot_vector
                .context("positive registry owner absent")?;
            let query = &q["slot_lookup"];
            ensure!(
                query["owner"] == owner && query["id"] == self.id && query["caller"] == 0x43800du32,
                "positive registry arguments differ"
            );
        }
        if self.stage == Stage::LookupReturned {
            ensure!(
                q["registers"]["eax"] == self.pointer.unwrap_or(0),
                "contained registry result differs"
            );
        }
        if let Some(sf) = q.get("slot_frame").filter(|v| v.is_object()) {
            let raw = crate::original_session::completed_animations::bytes(
                sf["raw"].as_str().context("positive frame absent")?,
            )?;
            let bp = sf["bp"].as_u64().context("positive BP absent")? as u32;
            ensure!(
                raw.len() == 0x70 && sf["address"] == bp - 0x58,
                "positive frame extent differs"
            );
            ensure!(
                std::array::from_fn::<_, 4, _>(|n| word(&raw, 0x20 + n * 4) as i32)
                    == self.rectangle,
                "contained local union rectangle differs"
            );
            for (n, value) in self.backup.iter().enumerate() {
                if let Some(value) = value {
                    ensure!(
                        word(&raw, 8 + n * 4) == *value,
                        "contained saved context prefix differs"
                    );
                }
            }
            if let Some(old) = self.old_rect {
                ensure!(
                    std::array::from_fn::<_, 4, _>(|n| word(&raw, 0x30 + n * 4) as i32) == old,
                    "contained saved child cache differs"
                );
            }
            if let Some(r) = self.draw_rect {
                ensure!(
                    std::array::from_fn::<_, 4, _>(|n| word(&raw, 0x40 + n * 4) as i32) == r,
                    "contained child draw result differs"
                );
            }
            if let Stage::Restore(n) = self.stage {
                let at = u32::from(n) * 4;
                ensure!(
                    q["registers"]["esi"] == bp - 0x50 + at
                        && q["registers"]["edi"]
                            == q["context"]["pointer"]
                                .as_u64()
                                .context("contained context absent")?
                                as u32
                                + at,
                    "contained restore cursors differ"
                );
            }
        }
        if let Some(raw) = q["draw_frame"]["raw"].as_str() {
            let raw = crate::original_session::completed_animations::bytes(raw)?;
            for (n, value) in self.output.iter().enumerate() {
                if let Some(value) = value {
                    ensure!(
                        word(&raw, 0xd0 + n * 4) as i32 == *value,
                        "positive contained output word differs"
                    );
                }
            }
        }
        Ok(())
    }
}
