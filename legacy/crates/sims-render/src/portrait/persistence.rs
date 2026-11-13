//! Concrete synchronous 4e9000 services. Encoded bytes are owned by the resource
//! host; its native detach writes them into the selected Rust-owned copy.
use super::cache::Images;
use sims_content::object_catalog::Catalog;
use sims_vm::image_cache::{
    State,
    persistence::{self, Host},
};

pub fn persist(
    cache: &mut State,
    catalog: &mut Catalog<'_>,
    images: &Images,
    index: u32,
    chunk: u16,
    image: Option<u32>,
) -> Result<bool, String> {
    let name: &[u8] = match chunk {
        2002 => b"faces",
        2003 => b"rel. images",
        2004 => b"newimage",
        2005 => b"speech_med",
        2006 => b"speech_large",
        2007 => b"web_image",
        _ => return Err("unknown native portrait chunk label".into()),
    };
    persistence::persist(
        cache,
        index,
        image,
        u32::from(chunk),
        name,
        &mut Services { catalog, images },
    )
}
struct Services<'a, 'b> {
    catalog: &'a mut Catalog<'b>,
    images: &'a Images,
}
impl Host for Services<'_, '_> {
    fn resource(&mut self, guid: u32) -> Result<Option<u32>, String> {
        self.catalog.portrait_resource(guid)
    }
    fn file(&mut self, resource: u32) -> Result<Option<u32>, String> {
        self.catalog.portrait_file(resource).map(Some)
    }
    fn writable(&mut self, file: u32) -> Result<bool, String> {
        self.catalog.resources.writable(file)
    }
    fn lookup(&mut self, file: u32, tag: u32, chunk: u32) -> Result<u32, String> {
        self.catalog.resources.by_id(file, tag, chunk, 0)
    }
    fn delete(&mut self, file: u32, handle: u32) -> Result<(), String> {
        self.catalog.resources.delete(file, handle)
    }
    fn encode(&mut self, image: u32) -> Result<Option<u32>, String> {
        let bytes = self
            .images
            .get(image)
            .and_then(|r| r.bmp())
            .map_err(|e| e.to_string())?;
        self.catalog.resources.allocate(bytes).map(Some)
    }
    fn add(
        &mut self,
        file: u32,
        buffer: Option<u32>,
        tag: u32,
        chunk: u32,
        name: &[u8],
    ) -> Result<(), String> {
        self.catalog.resources.add(
            file,
            sims_sim::object_catalog::composite::Chunk {
                handle: buffer.unwrap_or(0),
                tag,
                id: chunk,
                name,
                variant: 0,
                converted: 1,
            },
        )
    }
    fn release(&mut self, file: u32, buffer: Option<u32>) -> Result<(), String> {
        self.catalog
            .resources
            .release_chunk(file, buffer.unwrap_or(0))
    }
    fn error(&mut self, file: u32) -> Result<i32, String> {
        self.catalog.resources.error(file)
    }
}
