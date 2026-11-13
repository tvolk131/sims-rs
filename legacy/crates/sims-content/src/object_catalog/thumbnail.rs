//! Original4ccaa0 action-thumbnail composition. Image storage/creation, cached
//! object-picture retrieval and blits are explicit services, not VM callbacks.
pub type Rect = [i32; 4];
pub struct Prepared<I> {
    /// One temporary wrapper reference, released after output unlock.
    pub image: Option<I>,
    pub success: bool,
}
pub trait Backend {
    type Image: Copy;
    fn output(&self) -> Option<Self::Image>;
    /// Retain new, release old, assign, including null and identical owners.
    fn replace_output(&mut self, image: Option<Self::Image>);
    /// Allocate/construct an image with no wrapper reference yet.
    fn allocate(&mut self) -> Option<Self::Image>;
    /// Native virtual0c's return is ignored by4ccaa0.
    fn create(&mut self, image: Self::Image, size: [i32; 2], format: u32);
    fn size(&self, image: Self::Image) -> [i32; 2];
    fn rectangle(&self, image: Self::Image) -> Rect;
    fn lock(&mut self, image: Self::Image, mode: u32) -> bool;
    fn unlock(&mut self, image: Self::Image, mode: u32);
    fn background(&self) -> Option<Self::Image>;
    fn prepare_source(&mut self) -> Prepared<Self::Image>;
    fn cached_source(&self) -> Option<Self::Image>;
    fn replace_cached(&mut self, image: Option<Self::Image>);
    fn release(&mut self, image: Self::Image);
    fn blit(
        &mut self,
        destination: Self::Image,
        source: Option<Self::Image>,
        from: Rect,
        to: Rect,
    ) -> bool;
}

/// The cached picture may contain two horizontal views. Native halves the
/// right coordinate when width>45, then centers with signed truncation to zero.
/// All integer additions/subtractions wrap at32 bits before division.
pub fn rectangles(width: i32, mut source: Rect) -> (Rect, Rect) {
    if width > 45 {
        source[2] /= 2;
    }
    let x = source[0].wrapping_sub(source[2]).wrapping_add(45) / 2;
    let y = source[1].wrapping_sub(source[3]).wrapping_add(45) / 2;
    (
        source,
        [
            source[0].wrapping_add(x),
            source[1].wrapping_add(y),
            source[2].wrapping_add(x),
            source[3].wrapping_add(y),
        ],
    )
}
fn output<H: Backend>(host: &H) -> Result<H::Image, String> {
    host.output()
        .ok_or_else(|| "thumbnail output wrapper lost its image".into())
}

/// Synchronous native UI call. The host owns every wrapper and bitmap; no
/// fallback artwork or inferred successful service result is supplied here.
pub fn generate(host: &mut impl Backend) -> Result<bool, String> {
    let mut create = host.output().is_none();
    if let Some(image) = host.output() {
        if host.size(image) != [45, 45] {
            host.replace_output(None);
            create = true;
        }
    }
    if create {
        let image = host.allocate();
        host.replace_output(image);
        let Some(image) = host.output() else {
            return Ok(false);
        };
        host.create(image, [45, 45], 4);
    }
    let image = output(host)?;
    if !host.lock(image, 16) {
        return Ok(false);
    }
    let destination = output(host)?;
    let background = host.background();
    // Native ignores this blit's status and still retrieves the object picture.
    host.blit(destination, background, [0, 0, 45, 45], [0, 0, 45, 45]);
    let prepared = host.prepare_source();
    let result: Result<bool, String> = (|| {
        let mut success = prepared.success;
        if success && host.cached_source().is_some() {
            host.replace_cached(prepared.image);
            let cached = host
                .cached_source()
                .ok_or("thumbnail cached wrapper lost its image")?;
            let (from, to) = rectangles(host.size(cached)[0], host.rectangle(cached));
            let destination = output(host)?;
            success = host.blit(destination, host.cached_source(), from, to);
        }
        Ok(success)
    })();
    // The caller rereads its output wrapper for unlock. A backend which clears
    // that wrapper during a callback must expose the missing owner explicitly.
    let destination = output(host);
    if let Ok(image) = destination {
        host.unlock(image, 16);
    }
    if let Some(image) = prepared.image {
        host.release(image);
    }
    destination?;
    result
}
