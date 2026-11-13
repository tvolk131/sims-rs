//! Original object catalog construction. Resource opening, IFF chunk access and
//! definition construction supply services to 43b89c's ordered registration.
pub mod access;
pub mod composite;
pub mod definition;
pub mod directory;
pub mod iff;
pub mod move_tool;
pub mod products;
pub mod resources;
pub mod sprite_pool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Key<R> {
    pub resource: R,
    pub chunk: u16,
}

pub trait Host {
    type Resource: Copy + Eq;
    type Chunk: Copy;
    fn acquire(&mut self, path: &[u8], people: u8) -> Result<Option<Self::Resource>, String>;
    fn valid(&mut self, resource: Self::Resource) -> Result<bool, String>;
    fn count(&mut self, resource: Self::Resource) -> Result<u32, String>;
    /// One-based OBJD ordinal, with the original 46fb64 no-op callback.
    fn ordinal(&mut self, resource: Self::Resource, ordinal: u32) -> Result<Self::Chunk, String>;
    fn id(&mut self, resource: Self::Resource, chunk: Self::Chunk) -> Result<u32, String>;
    fn release_chunk(&mut self, resource: Self::Resource, chunk: Self::Chunk)
    -> Result<(), String>;
    fn definitions(&self) -> &[Key<Self::Resource>];
    /// Consumes the reacquired resource reference, including when registration
    /// skips a definition. Changes are visible before the next ordinal.
    fn register(&mut self, resource: Self::Resource, id: u32, category: u32) -> Result<(), String>;
    fn release(&mut self, resource: Self::Resource) -> Result<(), String>;
}

pub(crate) fn cstring(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len())]
}

/// 41200b locates separators/dots in counted storage. Its directory, basename
/// and extension copies then terminate at NUL independently (not once up front).
pub(crate) fn parts(path: &[u8]) -> Result<(&[u8], &[u8], &[u8]), String> {
    if path.len() >= 260 {
        return Err("object resource path exceeds CFString260".into());
    }
    let start = path
        .iter()
        .rposition(|b| matches!(b, b'/' | b'\\'))
        .map_or(0, |i| i + 1);
    let end = path[start..]
        .iter()
        .rposition(|&b| b == b'.')
        .map_or(path.len(), |i| start + i);
    Ok((
        cstring(&path[..start]),
        cstring(&path[start..end]),
        cstring(&path[end..]),
    ))
}

/// 43b89c. Lowercase the extensionless path, preserving slash spelling. Duplicate
/// identity is the initial resource instance plus low 16 bits of the OBJD ID.
pub fn register_file(
    path: &[u8],
    category: u32,
    people: u8,
    host: &mut impl Host,
) -> Result<(), String> {
    let (directory, basename, extension) = parts(path)?;
    if !extension.eq_ignore_ascii_case(b".iff") {
        return Ok(());
    }
    let path = [directory, basename].concat().to_ascii_lowercase();
    let Some(resource) = host.acquire(&path, people)? else {
        return Ok(());
    };
    let result = (|| {
        if host.valid(resource)? {
            let count = host.count(resource)? as i16;
            if count == i16::MAX {
                // Native's signed-short loop never terminates for this value.
                // Diagnose unsupported input rather than silently skipping it.
                return Err("native OBJD ordinal loop would wrap".into());
            }
            for ordinal in 1..=i32::from(count) {
                let chunk = host.ordinal(resource, ordinal as u32)?;
                let id = host.id(resource, chunk)?;
                host.release_chunk(resource, chunk)?;
                // Native widens a signed-short catalog index to unsigned. At
                // 32768 it exceeds a valid 32-bit vector count and exits.
                if !host
                    .definitions()
                    .iter()
                    .take(32768)
                    .any(|entry| entry.resource == resource && entry.chunk == id as u16)
                {
                    let acquired = host
                        .acquire(&path, people)?
                        .ok_or("object resource vanished during OBJD registration")?;
                    host.register(acquired, id, category)?;
                }
            }
        }
        Ok(())
    })();
    let released = host.release(resource);
    result.and(released)
}
