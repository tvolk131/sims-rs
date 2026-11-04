//! 5c1a7d / 5c1ab3 / 5c1af4 and 5c11d6: enumerate each registered type
//! across ordered search paths, retaining the first equal resource name/ID.
//! Directory mounting and the alias registry are explicit host inputs.
use super::{generator::fixed, name_map::Files, resource_name::Name};

pub trait Host {
    type Source: Clone;
    fn files(&mut self, directory: &[u8]) -> Result<Option<&Files<Self::Source>>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource<T> {
    pub directory: Vec<u8>,
    /// Counted lowercase filename with its last dot replaced by NUL. Retaining
    /// bytes after NUL is necessary for the original duplicate predicate.
    pub name: Name,
    pub kind: u32,
    pub source: T,
}

pub fn enumerate<T: Clone>(
    paths: &[Vec<u8>],
    kind: u32,
    aliases: Option<&[u32]>,
    host: &mut impl Host<Source = T>,
) -> Result<Vec<Resource<T>>, String> {
    let mut resources: Vec<Resource<T>> = vec![];
    enumerate_into(paths, kind, aliases, &mut resources, host)?;
    Ok(resources)
}

/// Repeated 5c1a7d calls append to one resource list, preserving its duplicate
/// predicate across types. The texture cache calls it for BMP and then TGA.
pub fn enumerate_into<T: Clone>(
    paths: &[Vec<u8>],
    kind: u32,
    aliases: Option<&[u32]>,
    resources: &mut Vec<Resource<T>>,
    host: &mut impl Host<Source = T>,
) -> Result<(), String> {
    let types = aliases.unwrap_or(std::slice::from_ref(&kind));
    for &kind in types {
        // The original's special **** type generates the literal suffix .*;
        // it is still compared as a suffix, not interpreted as a wildcard.
        let suffix = if kind == u32::from_be_bytes(*b"****") {
            b".*".to_vec()
        } else {
            [b".".as_slice(), fixed(&kind.to_be_bytes()[..3], 3)].concat()
        };
        for path in paths {
            let path = fixed(path, usize::MAX);
            let Some(files) = host.files(path)? else {
                continue;
            };
            for (filename, source) in files.iter() {
                if filename.len() < suffix.len()
                    || !filename[filename.len() - suffix.len()..].eq_ignore_ascii_case(&suffix)
                {
                    continue;
                }
                let name = Name::parse(filename)?;
                if resources.iter().any(|r| r.name == name) {
                    continue;
                }
                resources.push(Resource {
                    directory: path.to_vec(),
                    name,
                    kind,
                    source: source.clone(),
                });
            }
        }
    }
    Ok(())
}
