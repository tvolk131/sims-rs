//!4b4324 drawing-context construction. Virtual getters are ordered children;
//! their receiver and the partially constructed context survive a cold restore.
use serde::{Deserialize, Serialize};
mod light;
pub mod placement;
pub use light::LightUpdate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Getter {
    Layer,
    Renderer,
    #[serde(alias = "RowStep")]
    DisplayRotation,
    Zoom,
    Origin,
    Camera,
    Mode,
}
impl Getter {
    pub fn slot(self) -> u32 {
        match self {
            Self::Layer => 0x0c,
            Self::Renderer => 0x10,
            Self::DisplayRotation => 0x18,
            Self::Zoom => 0x14,
            Self::Origin => 0x20,
            Self::Camera => 0x24,
            Self::Mode => 0x34,
        }
    }
}
const GETTERS: [Getter; 7] = [
    Getter::Layer,
    Getter::Renderer,
    Getter::DisplayRotation,
    Getter::Zoom,
    Getter::Origin,
    Getter::Camera,
    Getter::Mode,
];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub owner: u32,
    pub getter: Getter,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Word(u32),
    Origin([u32; 2]),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    // The original 0x74-byte stack value. Unwritten words stay zero, including
    // its flags until the final getter has returned. No native stack pointer
    // is used to identify this owned value after restore.
    words: [u32; 29],
}
impl State {
    pub fn words(&self) -> &[u32; 29] {
        &self.words
    }
    pub fn flags(&self, group: u32) -> Result<u32, String> {
        let index = usize::try_from(group)
            .ok()
            .and_then(|n| n.checked_add(0x58 / 4));
        index
            .and_then(|n| self.words.get(n))
            .copied()
            .ok_or_else(|| "drawing-context flag storage is unobserved".into())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    owner: u32,
    next: u8,
    pending: Option<Request>,
    state: State,
    #[serde(default)]
    flags_updated: bool,
    #[serde(default)]
    live_writes: u32,
}
impl Frame {
    pub fn new(owner: u32) -> Self {
        let mut words = [0; 29];
        words[0x18 / 4] = owner;
        Self {
            owner,
            next: 0,
            pending: None,
            state: State { words },
            flags_updated: false,
            live_writes: 0,
        }
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    /// Ordered rendering flag-byte stores. Keep neighboring bytes and the
    /// constructor's unwritten-field validation intact.
    pub fn set_render_byte(&mut self,offset:usize,value:u8)->Result<(),String>{
        self.validate()?;if !self.complete()||!(0x35..=0x38).contains(&offset){return Err("invalid live rendering byte store".into());}
        let index=offset/4;let shift=(offset%4)*8;
        self.state.words[index]=(self.state.words[index]&!(255_u32<<shift))|(u32::from(value)<<shift);
        self.live_writes|=1<<index;self.validate()
    }
    ///437fdd temporarily translates and restores the first six context words.
    /// The shared depth fields remain live across this borrowed-prefix lifetime.
    pub fn store_prefix_word(&mut self, index:usize, value:u32)->Result<(),String>{
        self.validate()?;
        if !self.complete()||index>=6{return Err("contained context store outside completed prefix".into());}
        self.state.words[index]=value;self.live_writes|=1<<index;self.validate()
    }
    pub fn render_byte(&self,offset:usize)->Result<u8,String>{if !(0x34..=0x38).contains(&offset){return Err("invalid rendering byte read".into());}Ok((self.state.words[offset/4]>>((offset%4)*8))as u8)}
    pub fn advance_placement(
        &mut self,
        update: &mut placement::PlacementUpdate,
        caches: &mut placement::Caches,
        depth: &placement::Depth<'_>,
    ) -> Result<bool, String> {
        self.validate()?;
        if !self.complete() {
            return Err("placement precedes context construction".into());
        }
        let progress = update.advance_words(&mut self.state.words, caches, depth)?;
        self.live_writes |= update.written_mask();
        self.validate()?;
        Ok(progress)
    }
    ///4b4525 has two observable value stores. This marks only the light word
    /// as live; unrelated constructor/uninitialized-field checks stay active.
    pub fn advance_light(&mut self, update: &mut LightUpdate) -> Result<bool, String> {
        self.validate()?;
        if !self.complete() {
            return Err("drawing-context light setter precedes construction".into());
        }
        let progress = update.advance_words(&mut self.state.words)?;
        if progress {
            self.live_writes |= 1 << (0x48 / 4);
        }
        self.validate()?;
        Ok(progress)
    }
    pub fn complete(&self) -> bool {
        self.next == 7
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    ///4b4430 modifies one retained flag word after construction. The native
    /// condition tests the low byte of the third stack argument.
    pub fn update_flags(&mut self, group: u32, mask: u32, enabled: u8) -> Result<(), String> {
        self.validate()?;
        if !self.complete() {
            return Err("drawing-context flags precede construction".into());
        }
        let old = self.state.flags(group)?;
        let index = 0x58 / 4 + group as usize;
        self.state.words[index] = if enabled != 0 {
            old | mask
        } else {
            old & !mask
        };
        self.flags_updated = true;
        self.validate()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.owner == 0 || self.next > 7 || self.state.words[0x18 / 4] != self.owner {
            return Err("drawing context receiver/cursor differs".into());
        }
        if self.flags_updated && !self.complete() {
            return Err("drawing-context flag setter escaped constructor".into());
        }
        if self.live_writes & !((1 << (0x48 / 4)) | (1 << (0x34 / 4)) | (1 << (0x38 / 4)) | 0x63f) != 0
            || self.live_writes != 0 && !self.complete()
        {
            return Err("drawing-context live setter escaped its field/constructor".into());
        }
        if let Some(request) = self.pending {
            if self.complete()
                || request.owner != self.owner
                || request.getter != GETTERS[self.next as usize]
            {
                return Err("drawing context getter escaped constructor".into());
            }
        }
        for (index, &value) in self.state.words.iter().enumerate() {
            let written = match index * 4 {
                0x18 => true,
                0x2c => self.next >= 1,
                0x30 => self.next >= 2,
                0x1c => self.next >= 3,
                0x20 => self.next >= 4,
                0x3c | 0x40 => self.next >= 5,
                0x44 | 0x48 => self.next >= 6,
                0x4c | 0x58 | 0x5c => self.next >= 7,
                0x60..=0x70 => self.flags_updated,
                _ => false,
            };
            if !written && self.live_writes & (1 << index) == 0 && value != 0 {
                return Err("drawing context field precedes its getter".into());
            }
        }
        if self.next >= 6
            && self.live_writes & (1 << (0x48 / 4)) == 0
            && self.state.words[0x48 / 4] != 0x3f000000
        {
            return Err("drawing context half constant differs".into());
        }
        if self.complete()
            && !self.flags_updated
            && (self.state.words[0x58 / 4] != 15 || self.state.words[0x5c / 4] != 3)
        {
            return Err("drawing context default flags differ".into());
        }
        Ok(())
    }
    pub fn advance(&mut self) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.complete() {
            return Ok(None);
        }
        let request = Request {
            owner: self.owner,
            getter: GETTERS[self.next as usize],
        };
        self.pending = Some(request);
        Ok(Some(request))
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("drawing context getter is not pending".into());
        }
        let words = &mut self.state.words;
        match (request.getter, reply) {
            (Getter::Layer, Reply::Word(value)) => words[0x2c / 4] = value,
            (Getter::Renderer, Reply::Word(value)) => words[0x30 / 4] = value,
            (Getter::DisplayRotation, Reply::Word(value)) => {
                words[0x1c / 4] = value.wrapping_shl(1)
            }
            (Getter::Zoom, Reply::Word(value)) => words[0x20 / 4] = value,
            (Getter::Origin, Reply::Origin(value)) => {
                words[0x3c / 4..0x44 / 4].copy_from_slice(&value)
            }
            (Getter::Camera, Reply::Word(value)) => {
                words[0x44 / 4] = value;
                words[0x48 / 4] = 0x3f000000;
            }
            (Getter::Mode, Reply::Word(value)) => {
                words[0x4c / 4] = value;
                words[0x58 / 4] = 15;
                words[0x5c / 4] = 3;
            }
            _ => return Err("drawing context getter reply type differs".into()),
        }
        self.next += 1;
        self.pending = None;
        self.validate()
    }
}
