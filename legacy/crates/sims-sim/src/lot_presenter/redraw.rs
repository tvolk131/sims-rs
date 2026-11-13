//! Read-only adapter over the actual retained World and lot; no object copies.
use sims_vm::{ObjectId, World};
pub struct Live<'a> {
    pub world: &'a World,
    pub placement: &'a crate::lot_placement::State,
}
impl sims_vm::presenter::redraw::View for Live<'_> {
    fn registered(&self) -> Result<&[ObjectId], String> {
        self.world
            .person
            .as_ref()
            .map(|p| p.person_order.as_slice())
            .ok_or("redraw person registry order absent".into())
    }
    fn cached_tile(&self, id: ObjectId) -> Result<[i8; 3], String> {
        let p = self
            .world
            .positions
            .get(&id)
            .ok_or("redraw object position absent")?;
        Ok([(p.y >> 4) as i8, (p.x >> 4) as i8, p.level as i8])
    }
    fn active_size(&self) -> Result<i32, String> {
        self.world
            .map_size
            .as_ref()
            .map(|m| m.active_size)
            .ok_or("redraw active extent absent".into())
    }
    fn native_flags(&self, id: ObjectId) -> Result<u32, String> {
        self.world
            .objects
            .get(&id)
            .and_then(|o| o.native_flags)
            .ok_or("redraw native object flags absent".into())
    }
    fn tile_flags(&self, [x, y, z]: [i8; 3]) -> Result<u8, String> {
        let level = usize::try_from(i32::from(z) - 1).map_err(|_| "redraw level is not placed")?;
        let flags = self
            .placement
            .flags
            .get(level)
            .ok_or("redraw floor flags absent")?;
        let x = usize::try_from(x).map_err(|_| "negative redraw column")?;
        let y = usize::try_from(y).map_err(|_| "negative redraw row")?;
        let stride = self.placement.links.size;
        if x >= stride || y >= stride {
            return Err("redraw tile outside lot".into());
        }
        flags
            .get(x * stride + y)
            .copied()
            .ok_or("redraw tile flags absent".into())
    }
}
