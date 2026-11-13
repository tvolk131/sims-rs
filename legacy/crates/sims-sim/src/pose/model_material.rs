//! Original model material controls (5a1a8f). These bytes select effects; they
//! are not RGB. Render groups and material vectors retain stable identities so
//! callbacks can replace their storage without changing captured loop counts.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vector {
    /// Allocated/readable slots, including a tail retained after logical shrink.
    pub storage: Vec<Option<u64>>,
    pub len: usize,
}
impl Vector {
    fn count(&self) -> Result<usize, String> {
        if self.len > self.storage.len() {
            return Err("material vector end exceeds readable storage".into());
        }
        Ok(self.len)
    }
    fn at(&self, index: usize) -> Result<Option<u64>, String> {
        self.storage
            .get(index)
            .copied()
            .ok_or_else(|| "captured material traversal exceeds live readable storage".into())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drawable {
    pub mode: u32,
    /// Native mesh +c4, distinct from geometry's bounds flags at +14.
    pub flags: u32,
    pub materials: Vector,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Material {
    /// D3DMATERIAL's 20 words, including exact float bits and device fields.
    pub words: [u32; 20],
    pub references: u32,
    pub texture: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub controls: [u8; 3],
    pub group: Option<u64>,
    pub groups: BTreeMap<u64, Vector>,
    pub drawables: BTreeMap<u64, Drawable>,
    pub materials: BTreeMap<u64, Material>,
}
impl State {
    /// 5d487b. Nonzero effect traversal deliberately retains this reference.
    pub fn acquire(&mut self, id: u64) -> Result<(), String> {
        let m = self
            .materials
            .get_mut(&id)
            .ok_or("missing acquired material")?;
        m.references = m.references.wrapping_add(1);
        Ok(())
    }
    /// 5d4882's non-destructive branch. Owners with a final reference must
    /// provide the real destructor/device cleanup service, not silently drop it.
    pub fn release_retained(&mut self, id: u64) -> Result<(), String> {
        let m = self
            .materials
            .get_mut(&id)
            .ok_or("missing released material")?;
        if m.references == 1 {
            return Err("material release requires destructor service".into());
        }
        m.references = m.references.wrapping_sub(1);
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Cast { object: Option<u64> },
    Acquire { material: u64 },
    Publish { material: u64, words: [u32; 20] },
    Release { material: u64 },
}
pub enum Reply {
    Cast(Option<u64>),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Child,
    Drawable,
    Material,
    Read,
    Release,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    requested: [u8; 3],
    stage: Stage,
    children: usize,
    child: usize,
    drawable: Option<u64>,
    materials: usize,
    index: usize,
    material: Option<u64>,
    pending: Option<Request>,
}
impl Update {
    pub fn new(requested: [u8; 3]) -> Self {
        Self {
            requested,
            stage: Stage::Start,
            children: 0,
            child: 0,
            drawable: None,
            materials: 0,
            index: 0,
            material: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.child > self.children
            || self.index > self.materials
            || matches!(
                self.stage,
                Stage::Drawable | Stage::Material | Stage::Read | Stage::Release
            ) && self.drawable.is_none()
            || self.pending.as_ref().is_some_and(|r| {
                !matches!(
                    (self.stage, r),
                    (Stage::Child, Request::Cast { .. })
                        | (Stage::Material, Request::Acquire { .. })
                        | (Stage::Read, Request::Publish { .. })
                        | (Stage::Release, Request::Release { .. })
                )
            })
        {
            return Err("invalid material continuation".into());
        }
        Ok(())
    }
    fn clear(&self) -> bool {
        self.requested == [0; 3]
    }
    fn slot(&self, state: &State) -> Result<Option<u64>, String> {
        state
            .drawables
            .get(&self.drawable.ok_or("missing material drawable identity")?)
            .ok_or("material drawable disappeared")?
            .materials
            .at(self.index)
    }
    fn next_material(&mut self) {
        self.index += 1;
        self.material = None;
        self.stage = Stage::Material;
    }
    pub fn advance(
        &mut self,
        state: &mut State,
        normal_mode: u32,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            let request = match self.stage {
                Stage::Start => {
                    if !self.clear() {
                        state.controls = self.requested;
                    }
                    self.children = match state.group {
                        Some(id) => state
                            .groups
                            .get(&id)
                            .ok_or("missing material render group")?
                            .count()?,
                        None => 0,
                    };
                    self.stage = Stage::Child;
                    continue;
                }
                Stage::Child => {
                    if self.child == self.children {
                        if self.clear() {
                            state.controls = [0; 3];
                        }
                        self.stage = Stage::Done;
                        continue;
                    }
                    let group = state
                        .group
                        .and_then(|id| state.groups.get(&id))
                        .ok_or("material render group disappeared")?;
                    Request::Cast {
                        object: group.at(self.child)?,
                    }
                }
                Stage::Drawable => {
                    let drawable = state
                        .drawables
                        .get_mut(&self.drawable.unwrap())
                        .ok_or("missing cast material drawable")?;
                    if self.clear() {
                        drawable.mode = normal_mode;
                        drawable.flags &= !2;
                    } else {
                        if drawable.mode == 0 {
                            drawable.mode = 1;
                        }
                        if drawable.mode == 2 && (state.controls[0] != 0 || state.controls[1] != 0)
                        {
                            drawable.flags |= 2;
                        } else {
                            drawable.flags &= !2;
                        }
                    }
                    self.materials = drawable.materials.count()?;
                    self.index = 0;
                    self.stage = Stage::Material;
                    continue;
                }
                Stage::Material => {
                    if self.index == self.materials {
                        self.child += 1;
                        self.drawable = None;
                        self.stage = Stage::Child;
                        continue;
                    }
                    if let Some(material) = self.slot(state)? {
                        Request::Acquire { material }
                    } else {
                        self.next_material();
                        continue;
                    }
                }
                Stage::Read => {
                    // 5a1dc0 rereads live vector storage after virtual AddRef.
                    let Some(id) = self.slot(state)? else {
                        self.next_material();
                        continue;
                    };
                    let material = state
                        .materials
                        .get_mut(&id)
                        .ok_or("missing live material")?;
                    let one = 1_f32.to_bits();
                    if self.clear() {
                        material.words[1..9].fill(one);
                        material.words[9..17].fill(0);
                    } else {
                        let color = if state.controls[0] != 0 {
                            [0x3d888889, one, 0x3f71f1f2, 0x3f008081]
                        } else if state.controls[2] != 0 {
                            [0x3d888889, one, 0x3f008081, 0x3f008081]
                        } else if state.controls[1] != 0 {
                            [0x3f78f8f9, 0x3f78f8f9, one, 0x3f008081]
                        } else {
                            [one; 4]
                        };
                        for block in material.words[1..17].chunks_exact_mut(4) {
                            block.copy_from_slice(&color);
                        }
                    }
                    // 5d484a replaces the copied texture handle before calling
                    // the backend. Its HRESULT is ignored by the original.
                    if let Some(handle) = material.texture {
                        material.words[18] = handle;
                    }
                    self.material = Some(id);
                    Request::Publish {
                        material: id,
                        words: material.words,
                    }
                }
                Stage::Release => Request::Release {
                    material: self.material.ok_or("missing release identity")?,
                },
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request);
            return Ok(self.pending.clone());
        }
    }
    pub fn acknowledge(&mut self, request: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("stale material acknowledgment".into());
        }
        match (request, reply) {
            (Request::Cast { .. }, Reply::Cast(id)) => {
                self.drawable = id;
                if id.is_some() {
                    self.stage = Stage::Drawable;
                } else {
                    self.child += 1;
                    self.stage = Stage::Child;
                }
            }
            (Request::Acquire { .. }, Reply::Done) => self.stage = Stage::Read,
            (Request::Publish { .. }, Reply::Done) if self.clear() => self.stage = Stage::Release,
            (Request::Publish { .. } | Request::Release { .. }, Reply::Done) => {
                self.next_material()
            }
            _ => return Err("mismatched material reply".into()),
        }
        self.pending = None;
        Ok(())
    }
}
