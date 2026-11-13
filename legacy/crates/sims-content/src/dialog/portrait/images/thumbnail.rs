//! Software endpoint for4ccaa0 with an already resident cached object picture.
//! Source cache loading/generation and native DirectDraw pixels remain separate
//! qualifications; this runs the original caller over our existing CPU blitter.
use super::*;
use crate::dialog::portrait::generators::Backend as Images;
use crate::object_catalog::thumbnail::{self as controller, Backend as Thumbnail, Prepared};

type Storage = Software<fn(Render) -> Result<Rendered<Image>>>;
struct Composer {
    host: Storage,
    background: usize,
    cached: Option<usize>,
}
impl Thumbnail for Composer {
    type Image = usize;
    fn output(&self) -> Option<usize> {
        self.host.output
    }
    fn replace_output(&mut self, image: Option<usize>) {
        if let Some(image) = image {
            Images::publish(&mut self.host, image);
        } else if let Some(old) = self.host.output.take() {
            Images::release(&mut self.host, old);
        }
    }
    fn allocate(&mut self) -> Option<usize> {
        Images::allocate(&mut self.host)
    }
    fn create(&mut self, image: usize, size: [i32; 2], _: u32) {
        Images::create(&mut self.host, image, size[0], size[1]);
    }
    fn size(&self, image: usize) -> [i32; 2] {
        let r = Images::rect(&self.host, image);
        [r[2], r[3]]
    }
    fn rectangle(&self, image: usize) -> Rect {
        Images::rect(&self.host, image)
    }
    fn lock(&mut self, image: usize, mode: u32) -> bool {
        Images::lock(&mut self.host, image, mode)
    }
    fn unlock(&mut self, image: usize, mode: u32) {
        Images::unlock(&mut self.host, image, mode);
    }
    fn background(&self) -> Option<usize> {
        Some(self.background)
    }
    fn prepare_source(&mut self) -> Prepared<usize> {
        if let Some(image) = self.cached {
            self.host.owners[image].refs += 1;
        }
        Prepared {
            image: self.cached,
            success: self.cached.is_some(),
        }
    }
    fn cached_source(&self) -> Option<usize> {
        self.cached
    }
    fn replace_cached(&mut self, image: Option<usize>) {
        if let Some(image) = image {
            self.host.owners[image].refs += 1;
        }
        if let Some(old) = self.cached {
            Images::release(&mut self.host, old);
        }
        self.cached = image;
    }
    fn release(&mut self, image: usize) {
        Images::release(&mut self.host, image);
    }
    fn blit(&mut self, destination: usize, source: Option<usize>, from: Rect, to: Rect) -> bool {
        match source {
            Some(source) => Images::blit(&mut self.host, destination, source, from, to),
            None => {
                self.host.fail("action thumbnail source absent");
                false
            }
        }
    }
}
/// Compose native45×45 action artwork from explicitly supplied resident images.
/// A failed/missing source loader is not replaced with generic fallback artwork.
pub fn compose(
    background: &Raster,
    picture: &Raster,
    initial: Option<Raster>,
) -> Result<Rendered<Raster>> {
    background.validate()?;
    picture.validate()?;
    ensure!(
        picture.format == background.format,
        "thumbnail picture display format differs"
    );
    let draw: fn(Render) -> Result<Rendered<Image>> =
        |_| anyhow::bail!("thumbnail requested an avatar draw");
    let mut host = Software::new(background.format, BTreeMap::new(), initial, draw)?;
    let background = host.put(Some(background.clone()), 1);
    let cached = host.put(Some(picture.clone()), 1);
    let mut composer = Composer {
        host,
        background,
        cached: Some(cached),
    };
    let success = controller::generate(&mut composer).map_err(anyhow::Error::msg)?;
    if let Some(cached) = composer.cached.take() {
        Images::release(&mut composer.host, cached);
    }
    Images::release(&mut composer.host, background);
    composer.host.finish(success)
}
