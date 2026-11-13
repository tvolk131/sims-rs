use super::*;

#[derive(Clone)]
struct Entry {
    bytes: Arc<[u8]>,
    retain_empty: bool,
    clips: Arc<[Animation]>,
}

#[derive(Clone)]
pub struct AnimationCatalogs {
    entries: BTreeMap<String, Entry>,
    retained_bytes: usize,
    limit: usize,
}

impl Default for AnimationCatalogs {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            retained_bytes: 0,
            limit: 64 * 1024 * 1024,
        }
    }
}

pub(super) fn parse(bytes: &[u8], retain_empty: bool) -> Result<Arc<[Animation]>> {
    Ok(if retain_empty {
        sims_formats::animation::read_bcf_native(bytes)?
    } else {
        read_bcf(bytes)?
    }
    .into())
}

impl AnimationCatalogs {
    pub fn load_native(
        &mut self,
        index: &Index,
        program: &mut Program,
        names: &BTreeSet<String>,
    ) -> Result<Report> {
        let mut scan = crate::resource_scan::Scan::default();
        self.load_read(index, program, names, true, |resource| scan.read(resource))
    }

    pub(super) fn load_read(
        &mut self,
        index: &Index,
        program: &mut Program,
        names: &BTreeSet<String>,
        retain_empty: bool,
        read: impl FnMut(&crate::Resource) -> Result<Vec<u8>>,
    ) -> Result<Report> {
        load_named_animations_using(
            index,
            program,
            names,
            retain_empty,
            read,
            |resource, bytes, native| {
                self.parse_with(&resource.locator, bytes, native, |bytes| {
                    parse(bytes, native)
                })
            },
        )
    }

    fn parse_with(
        &mut self,
        source: &str,
        bytes: Vec<u8>,
        retain_empty: bool,
        decode: impl FnOnce(&[u8]) -> Result<Arc<[Animation]>>,
    ) -> Result<Arc<[Animation]>> {
        if let Some(entry) = self.entries.get(source)
            && entry.retain_empty == retain_empty
            && entry.bytes.as_ref() == bytes.as_slice()
        {
            return Ok(Arc::clone(&entry.clips));
        }
        if let Some(old) = self.entries.remove(source) {
            self.retained_bytes -= old.bytes.len();
        }
        let clips = decode(&bytes)?;
        if bytes.len() <= self.limit.saturating_sub(self.retained_bytes)
            && self.entries.len() < 16_384
        {
            self.retained_bytes += bytes.len();
            self.entries.insert(
                source.to_owned(),
                Entry {
                    bytes: bytes.into(),
                    retain_empty,
                    clips: Arc::clone(&clips),
                },
            );
        }
        Ok(clips)
    }
}

#[cfg(test)]
mod tests;
