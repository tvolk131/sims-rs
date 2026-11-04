//! Sleep/Notify contracts recovered from Sims 1.91. Native object scheduler
//! counters are separate from BHAV parameters and may suspend passive objects.
use crate::{ObjectId, Program, Status, Thread, ThreadId, Vm};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdleState {
    pub countdown: i16,
    /// Upper half of the native registry word; countdown occupies the lower half.
    pub flags: u16,
    pub last_run: Option<u64>,
}
impl Default for IdleState {
    fn default() -> Self {
        Self {
            countdown: -1,
            flags: 0,
            last_run: None,
        }
    }
}
impl IdleState {
    pub fn from_registry_word(word: u32) -> Self {
        Self {
            countdown: word as i16,
            flags: (word >> 16) as u16,
            last_run: None,
        }
    }
    pub fn registry_word(&self) -> u32 {
        (u32::from(self.flags) << 16) | u32::from(self.countdown as u16)
    }
    /// Original 0x4457ef: replace only the selected upper-half bits.
    pub fn set_flags(&mut self, mask: u16, enabled: bool) {
        self.flags &= !mask;
        if enabled {
            self.flags |= mask;
        }
    }

    /// One native scheduler visit: a positive counter is decremented without
    /// invoking the object; zero and negative values allow it to run.
    pub fn visit(&mut self, tick: u64) -> bool {
        self.last_run = Some(tick);
        if self.countdown > 0 {
            self.countdown -= 1;
            false
        } else {
            true
        }
    }
}
impl Vm {
    /// Original 0x0044574b: undo a passive Sleep optimization and schedule the
    /// object every tick. Unlike Notify, preserve its remaining Sleep duration.
    pub fn wake_object(
        &mut self,
        p: &Program,
        current: &mut Thread,
        object: ObjectId,
    ) -> Result<(), String> {
        let Some(&thread) = self.object_threads.get(&object) else {
            return Ok(());
        };
        let Some(idle) = self
            .world
            .objects
            .get_mut(&object)
            .and_then(|o| o.idle.as_mut())
        else {
            return Ok(());
        };
        let countdown = idle.countdown;
        let already_ran = idle.last_run == Some(self.tick);
        idle.countdown = -2;
        let target = if let Some(target) = self.threads.get_mut(&thread) {
            target
        } else {
            if current.stack.first().is_none_or(|f| f.caller != object) {
                return Err("native wake has no registered object thread".into());
            }
            current
        };
        let depth = target
            .callback
            .as_ref()
            .map_or(target.stack.len(), |c| c.base_depth);
        if let Some(frame) = depth.checked_sub(1).and_then(|i| target.stack.get_mut(i)) {
            let (_, routine) = p.resolve(&frame.code_owner, frame.location.bhav)?;
            let ins = routine
                .body
                .instructions
                .get(frame.location.instruction)
                .ok_or("invalid wake instruction")?;
            if ins.opcode & 0x7fff == 0 && countdown >= 0 {
                *frame
                    .args
                    .first_mut()
                    .ok_or("native wake Sleep has no argument zero")? = countdown;
            }
        }
        if matches!(target.status, Status::Sleeping { .. }) {
            target.status = if already_ran {
                Status::Sleeping {
                    wake_tick: self.tick.checked_add(1).ok_or("tick exhausted")?,
                }
            } else {
                Status::Ready
            };
        }
        Ok(())
    }

    /// 0x004521f7: complete exactly one native object visit, after VM yield.
    /// A diagnostic instruction-budget pause is not a completed visit.
    pub fn finish_object_update(
        &mut self,
        object: ObjectId,
        completed: bool,
    ) -> Result<(), String> {
        if self
            .scheduled_object_visit
            .as_ref()
            .is_some_and(|v| v.object != object && v.chain().any(|parent| parent.object == object))
        {
            return Err("nested native update must return before its parent tail".into());
        }
        if completed
            && let Some(visit) = self
                .scheduled_object_visit
                .as_ref()
                .filter(|v| v.object == object)
        {
            visit.validate(self)?;
            let thread = &self.threads[&visit.thread()];
            if !visit.finished()
                && (matches!(thread.status, Status::Ready | Status::Faulted(_))
                    || thread.check
                    || thread.callback.is_some()
                    || thread.service.as_ref().is_some_and(|s| s.result.is_some()))
            {
                return Err("native object update has not yielded to its tail".into());
            }
        }
        let state = self
            .world
            .objects
            .get_mut(&object)
            .ok_or("missing updated object")?;
        let explicit_visit = self
            .scheduled_object_visit
            .as_mut()
            .filter(|v| v.object == object);
        if !completed
            || explicit_visit
                .as_ref()
                .map_or(state.last_update == Some(self.tick), |v| v.finished)
        {
            return Ok(());
        }
        if let Some(visit) = explicit_visit {
            visit.finished = true;
        }
        state.last_update = Some(self.tick);
        if let Some(cooldown) = state.data.get_mut(&25) {
            if *cooldown != 0 {
                *cooldown = cooldown.wrapping_sub(1);
            }
        }
        self.update_headline(object)
    }

    pub(crate) fn sleep(&mut self, t: &mut Thread, index: i16) -> Result<bool, String> {
        let f = t.stack.last_mut().ok_or("no sleep frame")?;
        let counter = f
            .args
            .get_mut(usize::try_from(index).unwrap_or(usize::MAX))
            .ok_or("native error 0x08: invalid sleep parameter")?;
        let obj = self
            .world
            .objects
            .get_mut(&f.caller)
            .ok_or("missing sleep caller")?;
        if t.interrupted {
            // Explicit host cancellation API, separate from primitive Notify.
            *counter = 0;
            t.interrupted = false;
            if let Some(s) = obj.idle.as_mut() {
                s.countdown = -1;
            }
        }
        if let Some(s) = obj.idle.as_mut() {
            if s.countdown == 0 {
                *counter = 0;
                s.countdown = -1;
            }
        }
        *counter = (*counter).max(0);
        if *counter == 0 {
            return Ok(true);
        }
        let mut wait = 1;
        if let Some(s) = obj.idle.as_mut() {
            if obj.person.is_none()
                && obj.data.get(&25) == Some(&0)
                && obj.headline.as_ref().is_none_or(|h| h.duration == 0)
            {
                //44aa57 tests kind, busy counter and headline duration. The
                //registry's run-while-paused flag does not prevent passive sleep.
                s.countdown = *counter - 1;
                wait = *counter as u64;
            } else {
                *counter -= 1;
            }
        } else {
            *counter -= 1;
        }
        t.status = Status::Sleeping {
            wake_tick: self.tick.checked_add(wait).ok_or("tick exhausted")?,
        };
        Ok(false)
    }
    pub(crate) fn notify_idle(
        &mut self,
        p: &Program,
        id: ThreadId,
        t: &mut Thread,
        object: ObjectId,
    ) -> Result<(), String> {
        if !self.world.objects.contains_key(&object) {
            return Err("native error 0x15: missing Notify stack object".into());
        }
        let Some(&target) = self.object_threads.get(&object) else {
            return Ok(());
        };
        let wake = |other: &mut Thread, world: &mut crate::World| -> Result<(), String> {
            if other.stack.is_empty() {
                return Ok(());
            }
            let index = other
                .callback
                .as_ref()
                .map_or(other.stack.len(), |b| b.base_depth)
                .checked_sub(1);
            let Some(index) = index else {
                return Ok(());
            };
            let f = &other.stack[index];
            let (_, routine) = p.resolve(&f.code_owner, f.location.bhav)?;
            let opcode = routine
                .body
                .instructions
                .get(f.location.instruction)
                .ok_or("invalid Notify frame PC")?
                .opcode
                & 0x7fff;
            if opcode == 0 || opcode == 17 {
                *other
                    .stack
                    .get_mut(index)
                    .unwrap()
                    .args
                    .first_mut()
                    .ok_or("Notify idle frame has no argument zero")? = 0;
                if opcode == 0 {
                    let already_ran =
                        if let Some(s) = world.objects.get_mut(&object).unwrap().idle.as_mut() {
                            s.countdown = 0;
                            s.last_run == Some(self.tick)
                        } else {
                            false
                        };
                    if matches!(other.status, Status::Sleeping { .. }) {
                        other.status = if already_ran {
                            Status::Sleeping {
                                wake_tick: self.tick.checked_add(1).ok_or("tick exhausted")?,
                            }
                        } else {
                            Status::Ready
                        };
                    }
                }
            }
            Ok(())
        };
        if target == id {
            wake(t, &mut self.world)
        } else {
            let other = self
                .threads
                .get_mut(&target)
                .ok_or("missing registered idle thread")?;
            wake(other, &mut self.world)
        }
    }
}
