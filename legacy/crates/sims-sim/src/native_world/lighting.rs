use super::fixed;
use crate::saved_matrix::Stop;
use serde::{Deserialize, Serialize};

const BYTES: usize = 4320;
const EXPANDED: usize = 1024;
const NORMAL: usize = 3072;
const BLUE: usize = 4096;
const GREEN: usize = 4128;
const RED: usize = 4256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub application_mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tables {
    bytes: Vec<u8>,
    pixel_shift: u8,
}

impl Tables {
    pub fn new(bytes: Vec<u8>, pixel_shift: u8) -> Result<Self, Stop> {
        let result = Self { bytes, pixel_shift };
        result.validate()?;
        Ok(result)
    }

    fn validate(&self) -> Result<(), Stop> {
        if self.bytes.len() != BYTES {
            return Err(Stop::Unavailable(
                "lighting continuation requires the complete owned original static table region"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn pixel_shift(&self) -> u8 {
        self.pixel_shift
    }

    fn set_mode(&mut self, context: Context) {
        if context.application_mode == 2 {
            self.pixel_shift = 2;
        }
    }

    fn base(&mut self) {
        let factor = f64::from(f32::from_bits(0x3d042108));
        for row in 0..32_usize {
            for column in 0..32_usize {
                let value = ((column * row) as f64 * factor + 0.5).trunc() as u8;
                self.bytes[NORMAL + row * 32 + column] = value;
                self.bytes[EXPANDED + row * 64 + column] = value;
                self.bytes[row * 32 + column] =
                    ((column * row.saturating_sub(1)) as f64 * factor + 1.5).trunc() as u8;
            }
        }
    }

    fn expanded(&mut self, context: Context) {
        if context.application_mode == 2 {
            let factor = f64::from(f32::from_bits(0x3c820821));
            for row in 0..32 {
                for column in 0..64 {
                    self.bytes[EXPANDED + row * 64 + column] =
                        ((column * (row * 2)) as f64 * factor + 0.5).trunc() as u8;
                }
            }
        }
    }

    fn channels(&mut self, context: Context) {
        let green_entries = if context.application_mode == 1 {
            32
        } else {
            64
        };
        for index in 0..green_entries {
            let input = (index as u32)
                .wrapping_shl(u32::from(self.pixel_shift))
                .wrapping_shl(8);
            let value = pack(input, context.application_mode) as u16;
            self.bytes[GREEN + index * 2..GREEN + index * 2 + 2]
                .copy_from_slice(&value.to_le_bytes());
        }
        for index in 0..32 {
            let red = pack((index as u32) << 19, context.application_mode) as u16;
            self.bytes[RED + index * 2..RED + index * 2 + 2].copy_from_slice(&red.to_le_bytes());
            self.bytes[BLUE + index] = pack((index as u32) << 3, context.application_mode) as u8;
        }
    }
}

fn pack(pixel: u32, mode: u32) -> u32 {
    if mode == 1 {
        (((((pixel >> 3) & 0x1f0000) | (pixel & 0xf800)) >> 3) | (pixel & 0xf8)) >> 3
    } else {
        (((((pixel >> 3) & 0x1f0000) | (pixel & 0xfc00)) >> 2) | (pixel & 0xf8)) >> 3
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    PixelMode,
    Base,
    Expanded,
    Channels,
    Reached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cut {
    ResourceEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construction {
    fixed: fixed::State,
    initial: Tables,
    tables: Tables,
    context: Context,
    phase: Phase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub fixed: fixed::State,
    initial: Tables,
    pub tables: Tables,
    pub context: Context,
}

fn expected(initial: &Tables, context: Context, phase: Phase) -> Result<Tables, Stop> {
    initial.validate()?;
    let mut result = initial.clone();
    if phase != Phase::PixelMode {
        result.set_mode(context);
    }
    if matches!(phase, Phase::Expanded | Phase::Channels | Phase::Reached) {
        result.base();
    }
    if matches!(phase, Phase::Channels | Phase::Reached) {
        result.expanded(context);
    }
    if phase == Phase::Reached {
        result.channels(context);
    }
    Ok(result)
}

impl State {
    pub fn validate(&self) -> Result<(), Stop> {
        self.fixed.validate()?;
        self.tables.validate()?;
        if self.tables != expected(&self.initial, self.context, Phase::Reached)? {
            return Err(Stop::Unsupported(
                "retained lighting tables differ from the actual original construction and untouched input bytes"
                    .into(),
            ));
        }
        Ok(())
    }
}

impl Construction {
    pub fn new(
        fixed: fixed::Construction,
        initial: Tables,
        context: Context,
    ) -> Result<Self, Stop> {
        let result = Self {
            fixed: fixed.into_state()?,
            tables: initial.clone(),
            initial,
            context,
            phase: Phase::PixelMode,
        };
        result.validate()?;
        Ok(result)
    }

    pub fn fixed(&self) -> &fixed::State {
        &self.fixed
    }

    pub fn tables(&self) -> &Tables {
        &self.tables
    }

    pub fn cut(&self) -> Option<Cut> {
        (self.phase == Phase::Reached).then_some(Cut::ResourceEntry)
    }

    fn validate(&self) -> Result<(), Stop> {
        self.fixed.validate()?;
        self.tables.validate()?;
        if self.tables != expected(&self.initial, self.context, self.phase)? {
            return Err(Stop::Unsupported(
                "restored lighting phase lost its actual initialized or untouched table bytes"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<Option<Cut>, Stop> {
        self.validate()?;
        match self.phase {
            Phase::PixelMode => {
                self.tables.set_mode(self.context);
                self.phase = Phase::Base;
            }
            Phase::Base => {
                self.tables.base();
                self.phase = Phase::Expanded;
            }
            Phase::Expanded => {
                self.tables.expanded(self.context);
                self.phase = Phase::Channels;
            }
            Phase::Channels => {
                self.tables.channels(self.context);
                self.phase = Phase::Reached;
            }
            Phase::Reached => {}
        }
        self.validate()?;
        Ok(self.cut())
    }

    pub fn into_state(self) -> Result<State, Stop> {
        self.validate()?;
        if self.cut().is_none() {
            return Err(Stop::Unavailable(
                "lighting has not reached actual49f0d5 resource construction entry".into(),
            ));
        }
        let result = State {
            fixed: self.fixed,
            initial: self.initial,
            tables: self.tables,
            context: self.context,
        };
        result.validate()?;
        Ok(result)
    }
}
