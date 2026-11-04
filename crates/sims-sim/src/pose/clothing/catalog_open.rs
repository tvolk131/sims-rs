//! 5c1f62's named-resource opening branch. Wildcard OS results remain explicit;
//! the original retries the pattern itself, not the returned filename.
use super::generator::fixed;
pub trait Host {
    type Search;
    fn open(&mut self, path: &[u8]) -> Result<u32, String>;
    fn find_first(&mut self, pattern: &[u8]) -> Result<Option<Self::Search>, String>;
    fn close_search(&mut self, search: &Self::Search) -> Result<(), String>;
    fn found_identity(&mut self, search: &Self::Search) -> Result<u32, String>;
    fn set_kind(&mut self, kind: u32) -> Result<(), String>;
    fn set_name(&mut self, name: &[u8]) -> Result<(), String>;
    fn set_identity(&mut self, identity: u32) -> Result<(), String>;
}

/// Concrete-type, plain-file pass of 5c1c65 through 5c1ec8. The caller must
/// resolve type aliases first. Status 5 means this pass missed, not that the
/// complete search (later provider paths and the named pass) has failed.
pub fn plain_paths(
    directories: &[Vec<u8>],
    name: Option<&[u8]>,
    kind: u32,
    host: &mut impl Host,
) -> Result<u32, String> {
    let Some(name) = name else { return Ok(5) };
    let name = fixed(name, usize::MAX);
    let bytes = kind.to_be_bytes();
    let extension = if bytes == *b"****" {
        b"*"
    } else {
        fixed(&bytes[..3], 3)
    };
    for directory in directories {
        let path = [fixed(directory, usize::MAX), name, b".", extension].concat();
        // 5c1ec8 maps every nonzero open status to 5, including status 4.
        if host.open(&path)? == 0 {
            host.set_kind(kind)?;
            host.set_name(name)?;
            return Ok(0);
        }
    }
    Ok(5)
}

pub fn named(
    directory: &[u8],
    name: &[u8],
    kind: u32,
    identity: u32,
    host: &mut impl Host,
) -> Result<u32, String> {
    let directory = fixed(directory, usize::MAX);
    let name = fixed(name, usize::MAX);
    let bytes = kind.to_be_bytes();
    let extension = if bytes == *b"****" {
        b"*"
    } else {
        fixed(&bytes[..3], 3)
    };
    let suffix = [b".".as_slice(), extension].concat();
    let plain = [directory, name, &suffix].concat();
    if host.open(&plain)? == 0 {
        host.set_kind(kind)?;
        host.set_name(name)?;
        return Ok(0);
    }
    let prefix = if identity == 0x80000000 {
        vec![]
    } else {
        format!("_{}_", identity as i32).into_bytes()
    };
    let numbered = [directory, &prefix, name, &suffix].concat();
    if host.open(&numbered)? == 0 {
        host.set_kind(kind)?;
        host.set_name(name)?;
        host.set_identity(identity)?;
        return Ok(0);
    }
    let pattern = [b"./".as_slice(), directory, b"/_*_", name, &suffix].concat();
    let Some(search) = host.find_first(&pattern)? else {
        return Ok(5);
    };
    host.close_search(&search)?;
    let status = host.open(&pattern)?;
    if status == 0 {
        host.set_kind(kind)?;
        host.set_name(name)?;
        let identity = host.found_identity(&search)?;
        host.set_identity(identity)?;
    }
    Ok(status)
}
