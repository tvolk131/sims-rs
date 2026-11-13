//! Ordered component resources behind the native ObjResFile. This owns native
//! aggregate selection/error semantics; concrete IFF/DIR I/O is a host service.
use super::{cstring, parts};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Configuration {
    pub kind: u8,
    pub extension: Vec<u8>,
}
#[derive(Debug, Clone, Copy)]
pub struct Reply {
    pub value: u32,
    pub error: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Filter {
    pub tags: Vec<u32>,
    pub exclude: u16,
}
impl Default for Filter {
    fn default() -> Self {
        Self {
            tags: vec![],
            exclude: 1,
        }
    }
}
#[derive(Clone, Copy)]
pub struct Chunk<'a> {
    pub handle: u32,
    pub tag: u32,
    pub id: u32,
    pub name: &'a [u8],
    pub variant: u32,
    pub converted: u32,
}
pub trait Host {
    fn construct(&mut self, kind: u8) -> Result<u32, String>;
    fn open(&mut self, resource: u32, path: &[u8], writable: u32) -> Result<i32, String>;
    fn valid(&mut self, resource: u32) -> Result<bool, String>;
    fn writable(&mut self, _resource: u32) -> Result<bool, String> {
        Err("component writable query is not implemented".into())
    }
    fn flush(&mut self, _resource: u32) -> Result<i32, String> {
        Err("component flush is not implemented".into())
    }
    fn write(&mut self, _resource: u32, _handle: u32) -> Result<i32, String> {
        Err("component write is not implemented".into())
    }
    fn delete(&mut self, _resource: u32, _handle: u32) -> Result<i32, String> {
        Err("component deletion is not implemented".into())
    }
    fn add(&mut self, _resource: u32, _chunk: Chunk<'_>) -> Result<i32, String> {
        Err("component addition is not implemented".into())
    }
    /// May leave the supplied fixed-string value unchanged.
    fn filename(&mut self, resource: u32, output: &mut Vec<u8>) -> Result<(), String>;
    fn count(&mut self, resource: u32, tag: u32) -> Result<Reply, String>;
    fn by_id(
        &mut self,
        resource: u32,
        tag: u32,
        id: u32,
        options: u32,
        callback: u32,
    ) -> Result<Reply, String>;
    fn ordinal(
        &mut self,
        resource: u32,
        tag: u32,
        ordinal: u32,
        callback: u32,
    ) -> Result<Reply, String>;
    /// Writes made by a failed component remain visible to later components.
    fn name(&mut self, resource: u32, handle: u32, output: &mut Vec<u8>) -> Result<i32, String>;
    fn id(&mut self, resource: u32, handle: u32, output: &mut u32) -> Result<i32, String>;
    fn release_chunk(&mut self, resource: u32, handle: u32) -> Result<i32, String>;
    fn close(&mut self, resource: u32) -> Result<i32, String>;
    fn destroy(&mut self, resource: u32) -> Result<(), String>;
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Composite {
    pub configurations: Vec<Configuration>,
    /// Native queries stop at the first empty slot; removal still shifts all
    /// eight slots, including any later entries. Ordinarily this is contiguous.
    pub slots: [Option<u32>; 8],
    pub filters: [Filter; 8],
    pub error: i32,
}
impl Default for Composite {
    fn default() -> Self {
        let mut result = Self::empty();
        for (kind, suffix) in [(2, b".iff"), (2, b".spf"), (2, b".stx"), (3, b".dir")] {
            result.configure(kind, suffix);
        }
        result
    }
}
impl Composite {
    pub fn empty() -> Self {
        Self {
            configurations: vec![],
            slots: [None; 8],
            filters: std::array::from_fn(|_| Filter::default()),
            error: 0,
        }
    }
    /// 46b152: a zero type is the list terminator; invalid types are ignored.
    pub fn configure(&mut self, kind: i32, extension: &[u8]) {
        if self.configurations.len() < 16 && (1..=3).contains(&kind) {
            let extension = cstring(extension);
            self.configurations.push(Configuration {
                kind: kind as u8,
                extension: extension[..extension.len().min(7)].to_vec(),
            });
        }
    }
    fn append(&mut self, resource: u32) {
        if let Some(i) = self.slots.iter().position(|slot| slot.is_none()) {
            self.slots[i] = Some(resource);
            self.filters[i] = Filter::default();
        }
    }
    fn remove(&mut self, resource: u32) {
        while let Some(i) = self.slots.iter().position(|&slot| slot == Some(resource)) {
            self.slots.copy_within(i + 1..8, i);
            self.slots[7] = None;
            for n in i..7 {
                self.filters[n] = self.filters[n + 1].clone();
            }
            self.filters[7] = Filter::default();
        }
    }
    /// 46f61d/46b577. A successful component makes the aggregate open succeed
    /// even when other components fail. There is no rollback of earlier opens.
    pub fn open(
        &mut self,
        path: &[u8],
        writable: u32,
        host: &mut impl Host,
    ) -> Result<i32, String> {
        if path.len() >= 260 {
            return Err("composite path exceeds CFString260".into());
        }
        let _ = self.close(host)?;
        let mut result = -250;
        let mut opened = false;
        for configuration in self.configurations.clone() {
            let resource = host.construct(configuration.kind)?;
            let code = if let Some(path) = component_path(path, &configuration)? {
                host.open(resource, &path, writable)?
            } else {
                -250
            };
            if code == 0 {
                self.append(resource);
                opened = true;
            } else {
                result = code;
                host.destroy(resource)?;
            }
        }
        if opened {
            // 46f61d reads names in reverse and compares extensions, without
            // changing selection. Keep the calls and order observable.
            let count = self.slots.iter().take_while(|slot| slot.is_some()).count();
            for i in (0..count).rev() {
                let mut path = vec![];
                host.filename(self.slots[i].unwrap(), &mut path)?;
                let _ = parts(&path)?;
            }
            result = 0;
        }
        Ok(result)
    }
    /// 4067b3. The vtable +20 capability is independent of +28 validity.
    pub fn writable(&mut self, host: &mut impl Host) -> Result<bool, String> {
        self.query(|resource| host.writable(resource))
    }
    /// 406763: flush every writable component, retaining the first error.
    pub fn flush(&mut self, host: &mut impl Host) -> Result<(), String> {
        self.error = 0;
        for resource in self.slots {
            let Some(resource) = resource else { break };
            if host.writable(resource)? {
                let code = host.flush(resource)?;
                if self.error == 0 {
                    self.error = code;
                }
            }
        }
        Ok(())
    }
    pub fn write(&mut self, handle: u32, host: &mut impl Host) -> Result<(), String> {
        self.dispatch_write(handle, false, host)
    }
    pub fn delete(&mut self, handle: u32, host: &mut impl Host) -> Result<(), String> {
        self.dispatch_write(handle, true, host)
    }
    fn dispatch_write(
        &mut self,
        handle: u32,
        delete: bool,
        host: &mut impl Host,
    ) -> Result<(), String> {
        self.error = -99;
        for resource in self.slots {
            let Some(resource) = resource else { break };
            if host.writable(resource)? {
                self.error = if delete {
                    host.delete(resource, handle)?
                } else {
                    host.write(resource, handle)?
                };
                if self.error == 0 {
                    break;
                }
            }
        }
        Ok(())
    }
    /// 406e7e. Resolve/delete an existing chunk before selecting the first
    /// permitted writable component. No rollback occurs after a deletion.
    pub fn add(&mut self, chunk: Chunk<'_>, host: &mut impl Host) -> Result<(), String> {
        let mut old = self.by_id(chunk.tag, chunk.id, chunk.variant, 0, host)?;
        if old != 0 {
            if old == chunk.handle {
                self.release_chunk(old, host)?;
                old = self.by_id(chunk.tag, chunk.id, chunk.variant, 0, host)?;
            }
            self.delete(old, host)?;
            if self.error != 0 {
                return Ok(());
            }
        }
        self.error = -94;
        for (i, resource) in self.slots.into_iter().enumerate() {
            let Some(resource) = resource else { break };
            let filter = &self.filters[i];
            let included = filter.tags.contains(&chunk.tag);
            if included != (filter.exclude == 0) {
                self.error = -92;
                continue;
            }
            if host.writable(resource)? {
                self.error = host.add(resource, chunk)?;
                return Ok(());
            }
        }
        Ok(())
    }
    pub fn valid(&mut self, host: &mut impl Host) -> Result<bool, String> {
        self.query(|resource| host.valid(resource))
    }
    fn query(
        &mut self,
        mut query: impl FnMut(u32) -> Result<bool, String>,
    ) -> Result<bool, String> {
        self.error = 0;
        for resource in self.slots {
            let Some(resource) = resource else { break };
            if query(resource)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn filename(&mut self, output: &mut Vec<u8>, host: &mut impl Host) -> Result<(), String> {
        if self.valid(host)? {
            let mut path = vec![];
            host.filename(self.slots[0].unwrap(), &mut path)?;
            let (directory, basename, _) = parts(&path)?;
            *output = [directory, basename].concat();
        }
        Ok(())
    }
    pub fn count(&mut self, tag: u32, host: &mut impl Host) -> Result<u16, String> {
        self.error = 0;
        let mut count = 0_u32;
        for resource in self.slots {
            let Some(resource) = resource else { break };
            let reply = host.count(resource, tag)?;
            count = count.wrapping_add(reply.value);
            if reply.error != 0 {
                self.error = reply.error;
                return Ok(0);
            }
        }
        Ok(count as u16)
    }
    pub fn by_id(
        &mut self,
        tag: u32,
        id: u32,
        options: u32,
        callback: u32,
        host: &mut impl Host,
    ) -> Result<u32, String> {
        self.error = -98;
        for resource in self.slots {
            let Some(resource) = resource else { break };
            let reply = host.by_id(resource, tag, id, options, callback)?;
            if reply.value != 0 {
                self.error = 0;
                return Ok(reply.value);
            }
            self.error = reply.error;
        }
        Ok(0)
    }
    pub fn ordinal(
        &mut self,
        tag: u32,
        mut ordinal: u32,
        callback: u32,
        host: &mut impl Host,
    ) -> Result<u32, String> {
        self.error = -98;
        for resource in self.slots {
            let Some(resource) = resource else { break };
            let count = host.count(resource, tag)?;
            if count.error != 0 {
                self.error = count.error;
                return Ok(0);
            }
            if ordinal as i16 <= count.value as i16 {
                let reply = host.ordinal(resource, tag, ordinal, callback)?;
                self.error = if reply.value != 0 { 0 } else { reply.error };
                return Ok(reply.value);
            }
            ordinal = ordinal.wrapping_sub(count.value);
            if (ordinal as i16) < 1 {
                return Ok(0);
            }
        }
        Ok(0)
    }
    pub fn name(
        &mut self,
        handle: u32,
        output: &mut Vec<u8>,
        host: &mut impl Host,
    ) -> Result<(), String> {
        self.error = -99;
        for resource in self.slots {
            let Some(resource) = resource else { break };
            self.error = host.name(resource, handle, output)?;
            if self.error == 0 {
                break;
            }
        }
        Ok(())
    }
    pub fn id(
        &mut self,
        handle: u32,
        output: &mut u32,
        host: &mut impl Host,
    ) -> Result<(), String> {
        self.error = -99;
        for resource in self.slots {
            let Some(resource) = resource else { break };
            self.error = host.id(resource, handle, output)?;
            if self.error == 0 {
                break;
            }
        }
        Ok(())
    }
    /// 406db0: find the component owning the handle, retaining each error
    /// until the first successful detach. 465918 frees the handle afterward.
    pub fn release_chunk(&mut self, handle: u32, host: &mut impl Host) -> Result<(), String> {
        self.error = -99;
        for resource in self.slots {
            let Some(resource) = resource else { break };
            self.error = host.release_chunk(resource, handle)?;
            if self.error == 0 {
                break;
            }
        }
        Ok(())
    }
    pub fn close(&mut self, host: &mut impl Host) -> Result<i32, String> {
        self.error = 0;
        let mut result = 0;
        // Close every leading component before removing or destroying any.
        for resource in self.slots {
            let Some(resource) = resource else { break };
            let code = host.close(resource)?;
            if code != 0 {
                result = code;
            }
        }
        while let Some(resource) = self.slots[0] {
            self.remove(resource);
            host.destroy(resource)?;
        }
        Ok(result)
    }
    /// 46b1ca: destruction removes components in reverse order. Unlike close,
    /// it does not first invoke every component's close virtual method.
    pub fn destroy(&mut self, host: &mut impl Host) -> Result<(), String> {
        let count = self.slots.iter().take_while(|slot| slot.is_some()).count();
        for i in (0..count).rev() {
            let resource = self.slots[i]
                .ok_or("duplicate component removal invalidated native destruction index")?;
            self.remove(resource);
            host.destroy(resource)?;
        }
        Ok(())
    }
}

/// 46aff9 after the caller's initial CFString copy. An explicit extension must
/// match; otherwise append this provider's extension with native truncation.
fn component_path(path: &[u8], configuration: &Configuration) -> Result<Option<Vec<u8>>, String> {
    let suffix = &configuration.extension;
    if suffix.len() < 2 || suffix[0] != b'.' {
        return Ok(None);
    }
    let path = cstring(path);
    let (directory, basename, extension) = parts(path)?;
    if !extension.is_empty() {
        return Ok(extension
            .eq_ignore_ascii_case(suffix)
            .then(|| path.to_vec()));
    }
    let base = [directory, basename].concat();
    let (_, name, extension) = parts(&base)?;
    if name.is_empty() && extension.is_empty() {
        return Ok(None);
    }
    let mut path = [base.as_slice(), suffix].concat();
    path.truncate(259);
    Ok(Some(path))
}
