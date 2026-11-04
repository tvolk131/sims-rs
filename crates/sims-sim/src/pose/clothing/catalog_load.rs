//! Ordinary startup source controllers 59f844 (conversion disabled) and 5a0212.
//! Enumeration, opening and individual decoders have separate native evidence.
use super::{catalog_files::Resource, catalog_index::Lazy, generator::fixed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Binary,
    Index,
}
pub trait Host {
    type Source: Clone;
    type Stream;
    fn enumerate(&mut self, kind: u32, context: u32)
    -> Result<Vec<Resource<Self::Source>>, String>;
    fn flags(&self) -> u32;
    fn set_flags(&mut self, flags: u32) -> Result<(), String>;
    fn create(&mut self, source: &Resource<Self::Source>) -> Result<Self::Stream, String>;
    fn binary(&mut self, stream: &mut Self::Stream) -> Result<(), String>;
    fn open(
        &mut self,
        stream: &mut Self::Stream,
        source: &Resource<Self::Source>,
    ) -> Result<bool, String>;
    fn load(
        &mut self,
        stream: &mut Self::Stream,
        format: Format,
        location: &Lazy,
    ) -> Result<u32, String>;
    fn close(&mut self, stream: Self::Stream) -> Result<(), String>;
    fn finish(&mut self) -> Result<(), String>;
}

/// Native open/read return statuses are intentionally ignored, preserving work
/// from a partial file and visiting subsequent files. A Rust host/support error
/// is different and must stop the caller; it cannot become a native failure.
pub fn run(indexed: bool, context: u32, host: &mut impl Host) -> Result<(), String> {
    let kind = u32::from_be_bytes(if indexed { *b"INDX" } else { *b"ANIM" });
    let sources = host.enumerate(kind, context)?;
    for source in sources {
        let format = match (indexed, source.kind.to_be_bytes()) {
            (false, [b'C', b'M', b'X', 0]) => Format::Text,
            (false, [b'B', b'C', b'F', 0]) => Format::Binary,
            (true, [b'N', b'D', b'X', 0]) => Format::Index,
            _ => continue,
        };
        let old_flags = host.flags();
        if format == Format::Text {
            host.set_flags(1)?;
        }
        let mut stream = host.create(&source)?;
        if format != Format::Text {
            host.binary(&mut stream)?;
        }
        let opened = host.open(&mut stream, &source);
        if format == Format::Text {
            host.set_flags(old_flags)?;
        }
        let _ = opened?;
        let catalog = if format == Format::Index {
            vec![]
        } else {
            let stem = source.name.visible();
            if stem.len() > 1019 {
                return Err("startup catalog name exceeds native buffer".into());
            }
            [
                stem,
                if format == Format::Text {
                    b".cmx"
                } else {
                    b".bcf"
                },
            ]
            .concat()
        };
        let _ = host.load(
            &mut stream,
            format,
            &Lazy {
                name: vec![],
                directory: fixed(&source.directory, usize::MAX).to_vec(),
                catalog,
                context,
            },
        )?;
        host.close(stream)?;
    }
    host.finish()
}
