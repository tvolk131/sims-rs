//! ResolveResFile open/valid/name/close (465b4e/465d1b/465cd4/465caa).
//! 5c305e checks every resolver path with FindFirstFileA(path + "*.*").
//! Resource enumeration and payload queries are separate from this lifetime.
use crate::pose::clothing::catalog_paths::resolver_paths;

pub trait Host {
    type Search;
    fn find_first(&mut self, pattern: &[u8]) -> Result<Option<Self::Search>, String>;
    fn find_close(&mut self, search: Self::Search) -> Result<(), String>;
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Directory {
    pub paths: Option<Vec<Vec<u8>>>,
    pub error: i32,
}
impl Directory {
    pub fn open(
        &mut self,
        path: &[u8],
        _writable: u32,
        host: &mut impl Host,
    ) -> Result<i32, String> {
        if path.len() >= 260 {
            return Err("directory resource path exceeds CFString260".into());
        }
        // Appending the counted input one byte at a time drops embedded NULs;
        // it does not truncate the entire input at its first NUL.
        let mut path: Vec<_> = path
            .iter()
            .copied()
            .filter(|&b| b != 0)
            .map(|b| if b == b'\\' { b'/' } else { b })
            .collect();
        while path.starts_with(b"./") {
            path.drain(..2);
        }
        self.paths = Some(resolver_paths(&[path])?);
        if self.valid(host)? {
            Ok(0)
        } else {
            self.paths = None;
            Ok(-47)
        }
    }
    pub fn valid(&self, host: &mut impl Host) -> Result<bool, String> {
        let Some(paths) = &self.paths else {
            return Ok(false);
        };
        for path in paths {
            let pattern = [path.as_slice(), b"*.*"].concat();
            let Some(search) = host.find_first(&pattern)? else {
                return Ok(false);
            };
            host.find_close(search)?;
        }
        Ok(true)
    }
    pub fn filename(&self) -> Result<&[u8], String> {
        self.paths
            .as_ref()
            .and_then(|paths| paths.first())
            .map(Vec::as_slice)
            .ok_or_else(|| "closed directory resource has no native filename".into())
    }
    pub fn close(&mut self) -> i32 {
        self.paths = None;
        0
    }
}
