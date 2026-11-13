//! 4e9000: synchronous portrait replacement. Earlier deletion is not rolled
//! back when export fails, and release can overwrite earlier resource errors.
use super::State;

pub trait Host {
    fn resource(&mut self, guid: u32) -> Result<Option<u32>, String>;
    fn file(&mut self, resource: u32) -> Result<Option<u32>, String>;
    fn writable(&mut self, file: u32) -> Result<bool, String>;
    fn lookup(&mut self, file: u32, tag: u32, chunk: u32) -> Result<u32, String>;
    fn delete(&mut self, file: u32, handle: u32) -> Result<(), String>;
    fn encode(&mut self, image: u32) -> Result<Option<u32>, String>;
    fn add(
        &mut self,
        file: u32,
        buffer: Option<u32>,
        tag: u32,
        chunk: u32,
        name: &[u8],
    ) -> Result<(), String>;
    fn release(&mut self, file: u32, buffer: Option<u32>) -> Result<(), String>;
    fn error(&mut self, file: u32) -> Result<i32, String>;
}

pub fn persist(
    cache: &mut State,
    index: u32,
    image: Option<u32>,
    chunk: u32,
    name: &[u8],
    host: &mut impl Host,
) -> Result<bool, String> {
    if index as usize >= cache.entries.len() {
        return Ok(false);
    }
    let Some(image) = image else {
        return Ok(false);
    };
    let Some(resource) = cache.resolve_resource(index as usize, |guid, _| host.resource(guid))?
    else {
        return Ok(false);
    };
    let Some(file) = host.file(resource)? else {
        return Ok(false);
    };
    if !host.writable(file)? {
        return Ok(false);
    }
    let tag = u32::from_be_bytes(*b"BMP_");
    let old = host.lookup(file, tag, chunk)?;
    if old != 0 {
        host.delete(file, old)?;
    }
    let buffer = host.encode(image)?;
    let end = name
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name.len())
        .min(63);
    host.add(file, buffer, tag, chunk, &name[..end])?;
    host.release(file, buffer)?;
    Ok(host.error(file)? == 0)
}
