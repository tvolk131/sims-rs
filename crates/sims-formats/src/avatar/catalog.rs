//! Incremental ordinary BCF records for the native catalog controller. Earlier
//! records can be registered before a later decode fails. These bounded readers
//! retain the existing format support limits; an error is not, by itself, proof
//! that the original reader would reject the same input.
use super::{Appearance, Skeleton, index::Kind, read_appearance, read_skeleton};
use crate::{
    Result,
    animation::{Animation, read_animation},
    reader::Reader,
};

#[derive(Debug, Clone)]
pub enum Asset {
    Skeleton(Skeleton),
    Appearance(Appearance),
    Animation(Animation),
}
impl Asset {
    pub fn kind(&self) -> Kind {
        match self {
            Self::Skeleton(_) => Kind::Skeleton,
            Self::Appearance(_) => Kind::Appearance,
            Self::Animation(_) => Kind::Animation,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Skeleton(a) => &a.name,
            Self::Appearance(a) => &a.name,
            Self::Animation(a) => &a.name,
        }
    }
    pub fn set_name(&mut self, name: String) {
        match self {
            Self::Skeleton(a) => a.name = name,
            Self::Appearance(a) => a.name = name,
            Self::Animation(a) => a.name = name,
        }
    }
}

pub struct Binary<'a> {
    reader: Reader<'a>,
    group: usize,
    remaining: i32,
    failed: bool,
}
impl<'a> Binary<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            reader: Reader::new(data),
            group: 0,
            remaining: 0,
            failed: false,
        }
    }
    pub fn position(&self) -> usize {
        self.reader.pos
    }
    /// None represents the native count reader's short-read failure. Other
    /// errors are explicit Rust support/ordering limits, not native failures.
    pub fn count(&mut self, kind: Kind) -> Result<Option<i32>> {
        if self.failed || self.group != kind.index() || self.remaining > 0 {
            return self.reader.fail("catalog group out of sequence");
        }
        self.failed = true;
        if self.reader.remaining() < 4 {
            self.reader.pos = self.reader.data.len();
            return Ok(None);
        }
        let count = self.reader.i32()?;
        if count > [64, 256, 4096][kind.index()] {
            return self.reader.fail("catalog group exceeds supported limit");
        }
        self.remaining = count;
        self.group += 1;
        self.failed = false;
        Ok(Some(count))
    }
    pub fn read(&mut self, kind: Kind) -> Result<Asset> {
        if self.failed || self.group != kind.index() + 1 || self.remaining <= 0 {
            return self.reader.fail("catalog record out of sequence");
        }
        self.failed = true;
        let asset = match kind {
            Kind::Skeleton => Asset::Skeleton(read_skeleton(&mut self.reader)?),
            Kind::Appearance => {
                let mut a = read_appearance(&mut self.reader)?;
                // 5950f5 lowercases the decoded appearance's name before lookup.
                a.name.make_ascii_lowercase();
                Asset::Appearance(a)
            }
            Kind::Animation => Asset::Animation(read_animation(&mut self.reader)?),
        };
        self.remaining -= 1;
        self.failed = false;
        Ok(asset)
    }
}
