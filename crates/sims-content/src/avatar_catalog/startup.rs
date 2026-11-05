//! Source-backed services for the original ordinary/indexed startup loops.
use super::*;
use sims_sim::pose::clothing::catalog_load::{self, Format, Host};
use sims_sim::pose::clothing::catalog_open;

impl Catalog {
    /// 59f844 ordinary loading or 5a0212 indexed loading, with the native alias
    /// order and explicit search-path identity. File failures retain earlier
    /// effects and continue. Unsupported host/decoder errors stop this catalog.
    pub fn load_startup_paths(
        &mut self,
        index: &Index,
        directories: &mut Directories,
        paths: &[Vec<u8>],
        indexed: bool,
        context: u32,
        flags: &mut u32,
    ) -> Result<usize> {
        self.load_startup_paths_observed(index, directories, paths, indexed, context, flags, None)
    }

    pub(crate) fn load_startup_paths_observed(
        &mut self,
        index: &Index,
        directories: &mut Directories,
        paths: &[Vec<u8>],
        indexed: bool,
        context: u32,
        flags: &mut u32,
        inputs: Option<&crate::startup_inputs::Session>,
    ) -> Result<usize> {
        self.ready()?;
        let mut host = Startup {
            index,
            directories,
            paths,
            catalog: self,
            flags,
            visited: 0,
            scan: inputs
                .map(crate::resource_scan::Scan::observed)
                .transpose()?
                .unwrap_or_default(),
        };
        let result = catalog_load::run(indexed, context, &mut host);
        let visited = host.visited;
        self.completed(result)?;
        Ok(visited)
    }
}
struct Startup<'a> {
    index: &'a Index,
    directories: &'a mut Directories,
    paths: &'a [Vec<u8>],
    catalog: &'a mut Catalog,
    flags: &'a mut u32,
    visited: usize,
    scan: crate::resource_scan::Scan,
}
struct Stream {
    source: Option<usize>,
    bytes: Option<Vec<u8>>,
    binary: bool,
}
impl Host for Startup<'_> {
    type Source = Option<usize>;
    type Stream = Stream;
    fn enumerate(
        &mut self,
        kind: u32,
        _: u32,
    ) -> Result<Vec<catalog_files::Resource<Self::Source>>, String> {
        let aliases = match &kind.to_be_bytes() {
            b"ANIM" => vec![u32::from_be_bytes(*b"CMX\0"), u32::from_be_bytes(*b"BCF\0")],
            b"INDX" => vec![u32::from_be_bytes(*b"NDX\0")],
            _ => return Err("unexpected startup resource type".into()),
        };
        catalog_files::enumerate(self.paths, kind, Some(&aliases), self.directories)
    }
    fn flags(&self) -> u32 {
        *self.flags
    }
    fn set_flags(&mut self, flags: u32) -> Result<(), String> {
        *self.flags = flags;
        Ok(())
    }
    fn create(
        &mut self,
        resource: &catalog_files::Resource<Self::Source>,
    ) -> Result<Stream, String> {
        self.visited += 1;
        Ok(Stream {
            source: resource.source,
            bytes: None,
            binary: false,
        })
    }
    fn binary(&mut self, stream: &mut Stream) -> Result<(), String> {
        stream.binary = true;
        Ok(())
    }
    fn open(
        &mut self,
        stream: &mut Stream,
        resource: &catalog_files::Resource<Self::Source>,
    ) -> Result<bool, String> {
        let status = catalog_open::named(
            &resource.directory,
            resource.name.visible(),
            resource.kind,
            resource.name.identity,
            &mut Opening {
                index: self.index,
                directories: self.directories,
                stream,
                scan: &mut self.scan,
            },
        )?;
        if status == 0 {
            return Ok(true);
        }
        self.catalog.startup_open_failures.push(resource.clone());
        Ok(false)
    }
    fn load(
        &mut self,
        stream: &mut Stream,
        format: Format,
        location: &catalog_index::Lazy,
    ) -> Result<u32, String> {
        if stream.binary != (format != Format::Text) {
            return Err("startup decoder stream mode mismatch".into());
        }
        let (Some(source), Some(bytes)) = (stream.source, stream.bytes.as_deref()) else {
            // No readable stream can register records; retain its open failure
            // and let the original controller continue to the next resource.
            return Ok(1);
        };
        if format == Format::Index {
            let mut reader = IndexReader::new(bytes);
            let result = catalog_index::register(
                &mut reader,
                &location.directory,
                location.context,
                &mut Registration {
                    indexed: self.catalog,
                    source,
                },
            );
            if result.is_err() && reader.failed_short_read() {
                self.catalog.index_read_failures.push(source);
                return Ok(1);
            }
            result?;
            Ok(0)
        } else {
            decode_catalog_bytes(
                self.index,
                source,
                bytes,
                location,
                &mut self.catalog.registry,
                &mut self.catalog.assets,
                &mut self.catalog.read_failures,
            )
        }
    }
    fn close(&mut self, _: Stream) -> Result<(), String> {
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        Ok(())
    }
}

pub(super) fn open_plain_paths(
    index: &Index,
    directories: &Directories,
    paths: &[Vec<u8>],
    name: &[u8],
    kind: u32,
) -> Result<Option<usize>> {
    let mut stream = Stream {
        source: None,
        bytes: None,
        binary: true,
    };
    let mut scan = crate::resource_scan::Scan::default();
    let status = catalog_open::plain_paths(
        paths,
        Some(name),
        kind,
        &mut Opening {
            index,
            directories,
            stream: &mut stream,
            scan: &mut scan,
        },
    )
    .map_err(anyhow::Error::msg)?;
    Ok((status == 0).then_some(stream.source).flatten())
}

struct Opening<'a> {
    index: &'a Index,
    directories: &'a Directories,
    stream: &'a mut Stream,
    scan: &'a mut crate::resource_scan::Scan,
}
impl catalog_open::Host for Opening<'_> {
    type Search = ();
    fn open(&mut self, path: &[u8]) -> Result<u32, String> {
        let Some(id) = self
            .directories
            .path_source(path)
            .map_err(|e| e.to_string())?
        else {
            return Ok(5);
        };
        let source = self
            .index
            .resources
            .get(id)
            .ok_or("startup source ID absent")?;
        match self.scan.read(source) {
            Ok(bytes) => {
                self.stream.source = Some(id);
                self.stream.bytes = Some(bytes);
                Ok(0)
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                Ok(5)
            }
            Err(error) => Err(format!("{error:#}")),
        }
    }
    fn find_first(&mut self, pattern: &[u8]) -> Result<Option<()>, String> {
        let key = Files::<()>::key(pattern)?;
        if self.directories.no_matches.contains(&key) {
            return Ok(None);
        }
        Err(format!(
            "native wildcard resource opening needs an OS search result: {}",
            String::from_utf8_lossy(pattern)
        ))
    }
    fn close_search(&mut self, _: &()) -> Result<(), String> {
        Err("wildcard success is not supplied".into())
    }
    fn found_identity(&mut self, _: &()) -> Result<u32, String> {
        Err("wildcard identity is not supplied".into())
    }
    // The startup decoder only consumes the raw stream, never the stream's
    // descriptive name/type/ID fields. Publication order is checked separately.
    fn set_kind(&mut self, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn set_name(&mut self, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn set_identity(&mut self, _: u32) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
