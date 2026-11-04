//! 4e8b3d initial population and 4e8a3a resource refresh. The ordered native
//! object-definition vector is a host input; it is not inferred from live Sims.
use super::{Entry, Event, State};

#[derive(Debug, Clone, Copy)]
pub struct Resource {
    pub guid: u32,
    pub kind: u16,
}

pub trait Host {
    /// 43ac3a: a missing row ends traversal, including a hole before the end.
    fn resource(&mut self, index: u32, cache: &State) -> Result<Option<Resource>, String>;
    fn image_event(&mut self, event: Event, cache: &State);
}

impl State {
    /// 4e893e: a missing GUID lookup is retried next time; a cached resource is
    /// returned without consulting the manager. Resource handles are borrowed.
    pub fn resolve_resource(
        &mut self,
        index: usize,
        mut lookup: impl FnMut(u32, &State) -> Result<Option<u32>, String>,
    ) -> Result<Option<u32>, String> {
        let entry = self.entries.get(index).ok_or("image cache entry absent")?;
        if let Some(resource) = entry.resource {
            return Ok(Some(resource));
        }
        let resource = lookup(entry.guid, self)?;
        if resource == Some(0) {
            return Err("null resource handle must be absent".into());
        }
        self.entries[index].resource = resource;
        Ok(resource)
    }

    /// These routines are synchronous in the original. Unsupported host I/O
    /// stops at its actual boundary; this is not an all-or-nothing transaction.
    /// `initialized` is native global 6f26a4; initial population ignores it.
    pub fn populate(
        &mut self,
        reset: bool,
        initialized: bool,
        host: &mut impl Host,
    ) -> Result<(), String> {
        self.validate()?;
        if reset {
            // Vector erase destroys entries forwards, but each entry's seven
            // image wrappers backwards. The vector end changes only afterwards.
            for index in 0..self.entries.len() {
                for slot in (0..7).rev() {
                    let mut image = self.entries[index].images[slot];
                    self.assign_image(&mut image, None, &mut |event, cache| {
                        host.image_event(event, cache);
                    })?;
                    self.entries[index].images[slot] = image;
                }
            }
            self.entries.clear();
        } else if initialized {
            for entry in &mut self.entries {
                entry.resource = None;
            }
        } else {
            return Ok(());
        }
        let mut index = 0;
        while let Some(resource) = host.resource(index, self)? {
            if resource.kind == 2
                && (reset || !self.entries.iter().any(|e| e.guid == resource.guid))
            {
                if self.entries.len() >= 32767 {
                    return Err("image cache exceeds supported capacity".into());
                }
                self.entries.push(Entry::new(resource.guid));
            }
            index = index
                .checked_add(1)
                .ok_or("resource catalog index overflow")?;
        }
        Ok(())
    }
}
