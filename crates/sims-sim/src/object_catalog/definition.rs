//! 43ba52 registration, with 431759 normalization and 455f09 shared attributes.
//! Windows resource I/O and the sprite-resource pool remain explicit services.
use super::{Key, cstring};
use std::collections::BTreeMap;

/// Normalize the mutable resource chunk before interpreting any OBJD fields.
/// Native 136/137 upgrades clear from the last complete word, including an odd
/// trailing byte. Other versions resize without that word rounding.
pub fn normalize(data: &mut Vec<u8>) -> Result<(), String> {
    let version = u32::from_le_bytes(
        data.get(..4)
            .ok_or("OBJD version absent")?
            .try_into()
            .unwrap(),
    );
    if matches!(version, 136 | 137) {
        data[..4].copy_from_slice(&138_u32.to_le_bytes());
        if data.len() < 216 && data.len() & 1 != 0 {
            *data.last_mut().unwrap() = 0;
        }
    }
    data.resize(216, 0);
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub key: Key<u32>,
    pub semiglobal: Option<u32>,
    pub sprite_resource: u32,
    pub global: Option<u32>,
    pub name: Vec<u8>,
    pub data: Vec<u8>,
    pub index: u32,
    pub flags: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attributes {
    pub guid: u32,
    pub values: Vec<i16>,
}
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    /// Native overwrites the GUID lookup with the newest entry, retaining both
    /// ordered catalog rows and any separate attribute stores.
    pub guids: BTreeMap<u32, usize>,
    pub attributes: Vec<Attributes>,
}
pub struct Settings<'a> {
    /// Native manager +1240 rejects definitions whose word at +20 is nonzero.
    /// This is separate from the multipart group/subindex fields at +14/+16.
    pub filter_flagged: bool,
    pub global_directory: &'a [u8],
    pub global_people: u8,
    pub global: Option<u32>,
}
pub trait Host {
    type Chunk: Copy;
    fn definition(&mut self, resource: u32, id: u32) -> Result<Self::Chunk, String>;
    /// Mutable resident chunk storage: normalization persists in the resource.
    fn data(&mut self, chunk: Self::Chunk) -> Result<&mut Vec<u8>, String>;
    /// The resource supplies a CFString64, potentially with embedded NUL.
    fn name(&mut self, resource: u32, chunk: Self::Chunk) -> Result<Vec<u8>, String>;
    fn release_chunk(&mut self, resource: u32, chunk: Self::Chunk) -> Result<(), String>;
    fn release_resource(&mut self, resource: u32) -> Result<(), String>;
    fn sprite_resource(&mut self, resource: u32) -> Result<u32, String>;
    /// First GLOB resource, without a callback. The subsequent 46fb64 call is
    /// a native no-op; the bytes do not enter the final entry.
    fn glob(&mut self, resource: u32) -> Result<Option<Vec<u8>>, String>;
    fn acquire(&mut self, path: &[u8], people: u8) -> Result<Option<u32>, String>;
}
impl Catalog {
    pub fn register(
        &mut self,
        resource: u32,
        id: u32,
        category: u32,
        settings: Settings<'_>,
        host: &mut impl Host,
    ) -> Result<(), String> {
        let chunk = host.definition(resource, id)?;
        normalize(host.data(chunk)?)?;
        let name = host.name(resource, chunk)?;
        if name.len() >= 64 {
            return Err("object definition name exceeds CFString64".into());
        }
        let data = host.data(chunk)?;
        if settings.filter_flagged && u16::from_le_bytes(data[32..34].try_into().unwrap()) != 0 {
            host.release_chunk(resource, chunk)?;
            return host.release_resource(resource);
        }
        let data = data.clone();
        host.release_chunk(resource, chunk)?;
        let sprite_resource = host.sprite_resource(resource)?;
        let semiglobal = if let Some(glob) = host.glob(resource)? {
            let first = *glob
                .first()
                .ok_or("empty native GLOB would read past its chunk")?;
            let name = if glob.last() == Some(&0) {
                cstring(&glob)
            } else {
                cstring(
                    glob.get(1..=usize::from(first))
                        .ok_or("GLOB Pascal length exceeds chunk")?,
                )
            };
            if settings.global_directory.len() >= 260 {
                return Err("global resource directory exceeds CFString260".into());
            }
            let name = &name[..name.len().min(259)];
            let mut path = [cstring(settings.global_directory), name].concat();
            // 5b8a01 uses the native C-locale ASCII lowercase path, followed
            // by a bounded CFString260 copy. Locale-specific Windows mapping
            // and debugger behavior-registration callbacks are not supplied.
            path.make_ascii_lowercase();
            path.truncate(259);
            host.acquire(&path, settings.global_people)?
        } else {
            None
        };
        let guid = u32::from_le_bytes(data[28..32].try_into().unwrap());
        let count = i16::from_le_bytes(data[180..182].try_into().unwrap());
        if count != 0 {
            self.attributes.push(Attributes {
                guid,
                values: vec![
                    0;
                    if (0..32000).contains(&count) {
                        count as usize
                    } else {
                        0
                    }
                ],
            });
        }
        let index = self.entries.len();
        self.entries.push(Entry {
            key: Key {
                resource,
                chunk: id as u16,
            },
            semiglobal,
            sprite_resource,
            global: settings.global,
            name: cstring(&name).to_vec(),
            data,
            index: u32::try_from(index).map_err(|_| "object catalog index overflow")?,
            flags: ((id as u16 as u32) << 16) | ((category & 3) << 2),
        });
        self.guids.insert(guid, index);
        Ok(())
    }
}
