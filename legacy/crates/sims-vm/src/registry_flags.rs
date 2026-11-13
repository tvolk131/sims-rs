//! Cached object-registry flags (upper half of the word saved by ObjM).
//! They are refreshed at native Reset/post-load tails, not at every query.
use crate::{ObjectId, Program, Vm, program::MenuEntry};

/// Predicate body of 0x4358fc. Target resolution and TTAB lookup are separate
/// resource services. The threshold comparison is signed, including malformed
/// high-bit values; bit 7 excludes an interaction from autonomous advertising.
pub fn advertises(self_target: bool, table: Option<&[MenuEntry]>) -> bool {
    self_target
        && table.is_some_and(|entries| {
            entries
                .iter()
                .any(|e| (e.autonomy.threshold as i32) < 100 && e.flags & 128 == 0)
        })
}

impl Vm {
    /// Shared tail of 0x431be1, 0x423175 and 0x431e2e. Call after their synchronous
    /// Init/optional update or Load/Reset/Init sequence, including callback writes
    /// to data 60. Preserve the countdown and every other registry flag.
    pub fn refresh_registry_flags(&mut self, p: &Program, id: ObjectId) -> Result<(), String> {
        let self_target = crate::multitile::interaction_target(&self.world, p, id)? == id;
        let o = self
            .world
            .objects
            .get(&id)
            .ok_or("registry object absent")?;
        let def = o.definition.as_ref().ok_or("registry definition absent")?;
        let module = p.modules.get(&def.source).ok_or("registry module absent")?;
        let advertisement = advertises(
            self_target,
            module
                .object_interactions
                .get(&def.object)
                .map(Vec::as_slice),
        );
        let queued = *o.data.get(&60).ok_or("registry data 60 absent")? != 0;
        let idle = self
            .world
            .objects
            .get_mut(&id)
            .unwrap()
            .idle
            .as_mut()
            .ok_or("registry flags absent")?;
        idle.set_flags(2, queued);
        idle.set_flags(8, advertisement);
        Ok(())
    }
}
