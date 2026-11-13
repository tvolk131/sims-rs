//! 0x422937 plus Create Object's four-attempt wrapper, for off-lot groups.
//! The selected member is the factory head. Real virtual detach runs before
//! reading Rotation Notches. On-lot acceptance
//! and rollback remain explicit boundaries; no placement result is invented.
use super::*;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Rotation {
    pub(super) object: ObjectId,
    desired: i16,
    attempts: u8,
    phase: u8,
    old: Placement,
    delta: i16,
    current: Option<ObjectId>,
    seen: BTreeSet<ObjectId>,
    remove: Option<Box<VirtualOperation>>,
}
impl Rotation {
    pub(super) fn new(w: &World, object: ObjectId, desired: i16) -> Result<Self, String> {
        if ![0, 2, 4, 6].contains(&desired) {
            return Err("invalid group desired direction".into());
        }
        if sims_vm::multitile::chain(w, object)?[0] != object {
            return Err("off-lot creation rotation requires the group head".into());
        }
        Ok(Self {
            object,
            desired,
            attempts: 0,
            phase: 0,
            old: position(w, object)?,
            delta: 0,
            current: None,
            seen: BTreeSet::new(),
            remove: None,
        })
    }
    pub(super) fn validate(&self, w: &World) -> Result<bool, String> {
        if ![0, 2, 4, 6].contains(&self.desired)
            || self.attempts > 4
            || self.phase > 7
            || !(0..8).contains(&self.delta)
            || self.remove.is_some() != (self.phase == 1)
            || self.current.is_some_and(|id| !w.objects.contains_key(&id))
        {
            return Err("invalid off-lot group rotation continuation".into());
        }
        sims_vm::multitile::chain(w, self.object)?;
        if matches!(self.phase, 3 | 4) && self.current.is_none() {
            return Err("missing multipart rotation cursor".into());
        }
        if let Some(remove) = &self.remove {
            remove.validate_progress(w, false)?;
            if remove.binding() != (self.object, None) {
                return Err("misbound multipart rotation removal".into());
            }
        }
        Ok(self.phase == 7)
    }
    pub(super) fn next(
        &mut self,
        w: &mut World,
        p: &Program,
        a: &Architecture,
        state: &mut State,
    ) -> Result<Option<Request>, String> {
        loop {
            match self.phase {
                0 => {
                    if self.attempts == 4 || data(w, self.object, 1)? == self.desired {
                        self.phase = 7;
                        return Ok(None);
                    }
                    self.old = position(w, self.object)?;
                    if state.links.on_lot([self.old.x, self.old.y]) {
                        return Err(
                            "on-lot multipart rotation acceptance/rollback adapter required".into(),
                        );
                    }
                    self.remove = Some(Box::new(VirtualOperation::remove(
                        w,
                        &state.links,
                        self.object,
                        true,
                    )?));
                    self.phase = 1;
                }
                1 => {
                    if let Some(r) = self.remove.as_mut().unwrap().next(w, p, a, state)? {
                        return Ok(Some(r));
                    }
                    self.remove = None;
                    self.attempts += 1;
                    self.delta = data(w, self.object, 23)? & 7;
                    // Unlike base Rotate, even zero notches detached the group.
                    if self.delta == 0 {
                        self.phase = 0;
                        continue;
                    }
                    self.current = Some(sims_vm::multitile::chain(w, self.object)?[0]);
                    self.seen.clear();
                    self.phase = 2;
                }
                2 => {
                    let Some(id) = self.current else {
                        sims_vm::multitile::update_offsets(w, self.object)?;
                        self.phase = 5;
                        continue;
                    };
                    if !self.seen.insert(id) {
                        return Err("cyclic multipart rotation chain".into());
                    }
                    let d = data(w, id, 1)?.wrapping_add(self.delta) & 7;
                    w.objects.get_mut(&id).unwrap().data.insert(1, d);
                    self.phase = 3;
                    return Ok(Some(Request::Invalidate {
                        object: id,
                        children_only: false,
                    }));
                }
                3 => {
                    self.phase = 4;
                    return Ok(Some(Request::Shadow(self.current.unwrap())));
                }
                4 => {
                    self.current = w
                        .objects
                        .get(&self.current.unwrap())
                        .and_then(|o| o.multitile.as_ref())
                        .ok_or("multipart rotation member removed")?
                        .next;
                    self.phase = 2;
                }
                5 | 6 => {
                    // 0x422abb checks the group's origin before visiting any
                    // base floor check. Read the selected offset for EACH call.
                    let off = w.objects[&self.object].multitile.as_ref().unwrap().offset;
                    let root = [
                        self.old.x.wrapping_sub(off[1].wrapping_mul(16)),
                        self.old.y.wrapping_sub(off[0].wrapping_mul(16)),
                    ];
                    if state.links.on_lot(root) {
                        return Err("multipart rotation needs on-lot group acceptance".into());
                    }
                    // Both acceptance calls reject with error 1. Because the
                    // saved selected point is outside, native does not roll back.
                    self.phase = if self.phase == 5 { 6 } else { 0 };
                    return Ok(Some(Request::PlacementError(1)));
                }
                _ => return Ok(None),
            }
        }
    }
}
