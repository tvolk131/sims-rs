//!4a8a32..4a8b01 selected-object wall-composition scan. Reads the live registry,
//! cached signed-byte tile, native flags and lot flags in original order.
use crate::ObjectId;
pub trait View {
    fn registered(&self) -> Result<&[ObjectId], String>;
    fn cached_tile(&self, id: ObjectId) -> Result<[i8; 3], String>;
    fn active_size(&self) -> Result<i32, String>;
    fn native_flags(&self, id: ObjectId) -> Result<u32, String>;
    fn tile_flags(&self, tile: [i8; 3]) -> Result<u8, String>;
}
/// Return the first matching object. The caller owns the optional4ab992 child.
/// Disabled scanning and rejected positions do not read flags or tile storage.
pub fn first(view: &impl View, enabled: u8) -> Result<Option<ObjectId>, String> {
    if enabled == 0 {
        return Ok(None);
    }
    for &id in view.registered()? {
        let tile @ [x, y, _] = view.cached_tile(id)?;
        if x < 1 {
            continue;
        }
        let limit = view.active_size()?.wrapping_sub(1);
        if i32::from(x) >= limit || y < 1 || i32::from(y) >= limit {
            continue;
        }
        if view.native_flags(id)? & 2 != 0 && view.tile_flags(tile)? & 4 != 0 {
            return Ok(Some(id));
        }
    }
    Ok(None)
}
