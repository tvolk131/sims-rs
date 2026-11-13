//! 4eb0eb / 4e895e: GUID-keyed seven-image cache invalidation.
//! Handles identify host-owned images, never original pointers. The continuous
//! software presenter starts with an empty cache; Windows pixels/destructors are
//! not reconstructed by this metadata model.
use serde::{Deserialize, Serialize};
pub mod catalog;
pub mod object_speech;
pub mod persistence;
pub mod portrait;
pub mod regeneration;
pub mod speech;
pub mod temporary;

pub const CHANGED: u32 = 0x2000_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub guid: u32,
    /// Native cache-record word +4, independent of a live object ID.
    #[serde(default, skip_serializing_if = "zero_neighbor")]
    pub neighbor: i16,
    /// Borrowed resource handle (+8), resolved lazily and cleared on catalog
    /// refresh. Host identities, never original process pointers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<u32>,
    pub images: [Option<u32>; 7],
    pub dirty: [bool; 7],
}
fn zero_neighbor(value: &i16) -> bool {
    *value == 0
}
impl Entry {
    /// 4e890a / 4e88fa: seven empty, clean images and no resource/neighbor.
    pub fn new(guid: u32) -> Self {
        Self {
            guid,
            neighbor: 0,
            resource: None,
            images: [None; 7],
            dirty: [false; 7],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub entries: Vec<Entry>,
    /// One-based image handles; references may include owners outside this cache.
    /// A zero count is a destroyed image and must have no remaining cache owners.
    pub refs: Vec<u32>,
    /// Explicit presenter owner (+b0) and its flags (+c). None is permitted for
    /// standalone 4eb0eb; the memory resolver's final invalidation requires it.
    pub ui: Option<u32>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            entries: vec![],
            refs: vec![],
            ui: Some(0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Release(u32),
    Destroy(u32),
    Ui,
}

impl State {
    /// 5a5f79: retain the incoming image before releasing the previous owner,
    /// even when both handles name the same image. Destruction observes the old
    /// wrapper; the assignment happens afterwards.
    pub fn assign_image(
        &mut self,
        owner: &mut Option<u32>,
        image: Option<u32>,
        host: &mut impl FnMut(Event, &Self),
    ) -> Result<(), String> {
        for handle in [*owner, image].into_iter().flatten() {
            if self
                .refs
                .get(handle.wrapping_sub(1) as usize)
                .is_none_or(|n| *n == 0)
            {
                return Err("image assignment references absent/destroyed image".into());
            }
        }
        if let Some(handle) = image {
            let count = &mut self.refs[handle as usize - 1];
            *count = count.checked_add(1).ok_or("image reference overflow")?;
        }
        if let Some(handle) = *owner {
            host(Event::Release(handle), self);
            let count = &mut self.refs[handle as usize - 1];
            *count -= 1;
            if *count == 0 {
                host(Event::Destroy(handle), self);
            }
        }
        *owner = image;
        Ok(())
    }

    /// 4e899e: successful non-null storage clears dirty; assigning null leaves
    /// the dirty byte as it was. An invalid slot returns false without writes.
    pub fn store_image(
        &mut self,
        index: usize,
        slot: usize,
        image: Option<u32>,
        host: &mut impl FnMut(Event, &Self),
    ) -> Result<bool, String> {
        let entry = self.entries.get(index).ok_or("image cache entry absent")?;
        let Some(mut owner) = entry.images.get(slot).copied() else {
            return Ok(false);
        };
        self.assign_image(&mut owner, image, host)?;
        self.entries[index].images[slot] = owner;
        if owner.is_some() {
            self.entries[index].dirty[slot] = false;
        }
        Ok(true)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.entries.len() > 32767 || self.refs.len() > 1_000_000 {
            return Err("image cache exceeds supported capacity".into());
        }
        let mut counts = vec![0u32; self.refs.len()];
        for image in self.entries.iter().flat_map(|e| e.images).flatten() {
            let count = image
                .checked_sub(1)
                .and_then(|i| counts.get_mut(i as usize))
                .ok_or("image cache handle absent")?;
            *count += 1;
        }
        if counts
            .iter()
            .zip(&self.refs)
            .any(|(owners, refs)| owners > refs)
        {
            return Err("image cache ownership exceeds references".into());
        }
        Ok(())
    }

    pub fn invalidate(&mut self, guid: u32) -> Result<bool, String> {
        self.invalidate_with(guid, &mut |_, _| {})
    }

    pub fn notify(&mut self, guid: u32, first: bool) -> Result<bool, String> {
        self.notify_with(guid, first, &mut |_, _| {})
    }

    /// The observer is also the host image-release boundary. It sees the old
    /// slot before release, zero references before destruction, then each dirty
    /// write before the next slot. The invalidation itself cannot yield.
    pub fn invalidate_with(
        &mut self,
        guid: u32,
        host: &mut impl FnMut(Event, &Self),
    ) -> Result<bool, String> {
        self.notify_with(guid, false, host)
    }

    /// 4eb0cc / 4e897e clears each dirty byte BEFORE releasing its image and
    /// does not invalidate the UI. Replacement sets dirty AFTER release.
    pub fn notify_with(
        &mut self,
        guid: u32,
        first: bool,
        host: &mut impl FnMut(Event, &Self),
    ) -> Result<bool, String> {
        self.validate()?;
        let Some(index) = self.entries.iter().position(|e| e.guid == guid) else {
            return Ok(false);
        };
        for slot in 0..7 {
            if first {
                self.entries[index].dirty[slot] = false;
            }
            if let Some(image) = self.entries[index].images[slot] {
                host(Event::Release(image), self);
                self.refs[image as usize - 1] -= 1;
                if self.refs[image as usize - 1] == 0 {
                    host(Event::Destroy(image), self);
                }
                self.entries[index].images[slot] = None;
            }
            if !first {
                self.entries[index].dirty[slot] = true;
            }
        }
        if !first && self.ui.is_some() {
            host(Event::Ui, self);
            self.invalidate_ui()?;
        }
        Ok(true)
    }

    pub fn invalidate_ui(&mut self) -> Result<(), String> {
        *self
            .ui
            .as_mut()
            .ok_or("native image-cache UI owner absent")? |= CHANGED;
        Ok(())
    }
}
