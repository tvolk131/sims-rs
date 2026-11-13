//! Unchanged4b954a unit/cached palette selection. Refresh/rebuild are explicit
//! pending calls; no cache table or epoch is fabricated at those boundaries.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    UnitCompared,
    Unit,
    Cache,
    Epoch,
    CachedCompare,
    CachedCompared,
    Working,
    Return,
    Caller,
    Done,
    Refresh,
    Rebuild,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    owner: u32,
    light: u32,
    unit: u32,
    epsilon: u32,
    cached: u32,
    cache_generation: u32,
    generation: u32,
    stage: Stage,
    result: Option<u32>,
    write: Option<[u32; 2]>,
}
impl Selection {
    pub fn new(
        owner: u32,
        light: u32,
        unit: u32,
        epsilon: u32,
        cached: u32,
        cache_generation: u32,
        generation: u32,
    ) -> Result<Self, String> {
        let value = Self {
            owner,
            light,
            unit,
            epsilon,
            cached,
            cache_generation,
            generation,
            stage: Stage::Entry,
            result: None,
            write: None,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.owner.checked_add(0xe20).is_none()
            || self.owner == 0
            || !f32::from_bits(self.light).is_finite()
            || !f32::from_bits(self.unit).is_finite()
            || !f32::from_bits(self.epsilon).is_finite()
            || f32::from_bits(self.epsilon) < 0.0
        {
            return Err("palette selector numeric/owner profile unavailable".into());
        }
        let returned=matches!(self.stage,Stage::Return|Stage::Caller|Stage::Done);
        if returned!=self.result.is_some(){return Err("palette result lifetime differs".into());}
        if let Some(result)=self.result{
            let valid=if self.near(self.unit){result==self.owner+0x420}else{self.cache_generation>=self.generation&&self.near(self.cached)&&result==self.owner+0xc20};
            if !valid{return Err("palette result branch differs".into());}
        }
        if let Some([_,value])=self.write{
            if self.stage!=Stage::Refresh||value!=self.light||self.cached!=value||self.cache_generation>=self.generation{return Err("palette cache store lifetime differs".into());}
        }else if self.stage==Stage::Refresh{return Err("palette refresh lost its preceding cache store".into());}
        Ok(())
    }
    fn near(&self, other: u32) -> bool {
        let delta =
            (f64::from(f32::from_bits(self.light)) - f64::from(f32::from_bits(other))).abs();
        delta <= f64::from(f32::from_bits(self.epsilon))
    }
    pub fn advance(&mut self) -> Result<bool, String> {
        self.validate()?;
        use Stage::*;
        self.stage = match self.stage {
            Entry => UnitCompared,
            UnitCompared => {
                if self.near(self.unit) {
                    Unit
                } else {
                    Cache
                }
            }
            Unit => {
                self.result = Some(self.owner + 0x420);
                Return
            }
            Cache => Epoch,
            Epoch => {
                if self.cache_generation < self.generation {
                    self.write = Some([self.cached, self.light]);
                    self.cached = self.light;
                    Refresh
                } else {
                    CachedCompare
                }
            }
            CachedCompare => CachedCompared,
            CachedCompared => {
                if self.near(self.cached) {
                    Working
                } else {
                    Rebuild
                }
            }
            Working => {
                self.result = Some(self.owner + 0xc20);
                Return
            }
            Return => Caller,
            Caller => Done,
            Done | Refresh | Rebuild => return Ok(false),
        };
        Ok(true)
    }
    pub fn pc(&self) -> u32 {
        use Stage::*;
        match self.stage {
            Entry => 0x4b954a,
            UnitCompared => 0x4b9564,
            Unit => 0x4b9566,
            Cache => 0x4b956e,
            Epoch => 0x4b9577,
            CachedCompare => 0x4b9587,
            CachedCompared => 0x4b959b,
            Working => 0x4b95ac,
            Return => 0x4b95b2,
            Caller | Done => 0x4c1bc7,
            Refresh => 0x4b9582,
            Rebuild => 0x4b95a7,
        }
    }
    pub fn esp_delta(&self) -> i32 {
        use Stage::*;
        match self.stage {
            Entry => 0,
            Caller | Done => 8,
            Rebuild => -8,
            _ => -4,
        }
    }
    pub fn eax(&self) -> Option<u32> {
        match self.stage {
            Stage::Epoch => Some(self.cache_generation),
            Stage::Refresh => Some(self.light),
            _ => self.result,
        }
    }
    pub fn light_bits(&self) -> u32 {
        self.light
    }
    pub fn cache_write(&self) -> Option<[u32; 2]> {
        self.write
    }
    pub fn result(&self) -> Option<u32> {
        self.complete().then_some(self.result).flatten()
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn returned(&self) -> bool {
        matches!(self.stage, Stage::Caller | Stage::Done)
    }
    pub fn pending(&self) -> Option<u32> {
        match self.stage {
            Stage::Refresh => Some(0x4b9299),
            Stage::Rebuild => Some(0x4b95b6),
            _ => None,
        }
    }
}
