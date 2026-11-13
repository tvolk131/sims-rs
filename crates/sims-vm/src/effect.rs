//! 44aaa6's effect continuation and ordered presentation/resource services.
use crate::ObjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// Explicit host presentation setting (6f2628+4), not a saved lot field
    /// or per-object visibility test. Our active/headless view enables it.
    pub active: bool,
}
impl Default for Context {
    fn default() -> Self {
        Self { active: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emission {
    pub object: ObjectId,
    pub duration_ms: i32,
    pub kind: u8,
    pub size: u8,
    pub flags: u8,
    pub text: String,
}

pub trait Host {
    fn redraw(&mut self) -> Result<(), String>;
    fn target(&mut self) -> Result<ObjectId, String>;
    fn duration(&mut self, local: bool, index: i16) -> Result<i16, String>;
    fn active(&mut self) -> Result<bool, String>;
    fn residential(&mut self) -> Result<bool, String>;
    fn actor_family_permits(&mut self) -> Result<bool, String>;
    fn notify(&mut self) -> Result<(), String>;
    fn text(&mut self, entry: u8) -> Result<String, String>;
    fn format(&mut self, text: String) -> Result<String, String>;
    fn emit(&mut self, effect: Emission) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Yield,
    Complete,
}

pub fn step(state: &mut u32, operands: [u8; 8], host: &mut impl Host) -> Result<Outcome, String> {
    match *state {
        0 => {
            host.redraw()?;
            *state = 1;
            Ok(Outcome::Yield)
        }
        1 => {
            // Native publishes this before target, duration or resource lookup.
            // Errors must retain it; suppressed effects complete immediately.
            *state = 2;
            let object = host.target()?;
            let flags = operands[4];
            let duration = host.duration(
                flags & 32 != 0,
                i16::from_le_bytes([operands[0], operands[1]]),
            )?;
            if !host.active()? && flags & 8 == 0 {
                return Ok(Outcome::Complete);
            }
            if !host.residential()? && flags & 8 == 0 && !host.actor_family_permits()? {
                return Ok(Outcome::Complete);
            }
            if flags & 64 == 0 {
                host.notify()?;
            }
            let mut text = if operands[5] == 0 {
                String::new()
            } else {
                host.text(operands[5])?
            };
            if !text.is_empty() {
                text = host.format(text)?;
            }
            host.emit(Emission {
                object,
                duration_ms: i32::from(duration) * 1000,
                kind: operands[3],
                size: operands[2],
                flags,
                text,
            })?;
            Ok(Outcome::Yield)
        }
        _ => Ok(Outcome::Complete),
    }
}

struct Live<'a> {
    vm: &'a mut crate::Vm,
    program: &'a crate::Program,
    thread: &'a mut crate::Thread,
}
impl Host for Live<'_> {
    fn redraw(&mut self) -> Result<(), String> {
        // The continuous/headless view has no persistent Windows redraw state.
        Ok(())
    }
    fn target(&mut self) -> Result<ObjectId, String> {
        let target = self.thread.stack.last().unwrap().stack_object;
        self.vm
            .world
            .objects
            .contains_key(&target)
            .then_some(target)
            .ok_or("Special Effect missing target (native error 0x17)".into())
    }
    fn duration(&mut self, local: bool, index: i16) -> Result<i16, String> {
        self.vm
            .read(self.program, self.thread, if local { 25 } else { 7 }, index)
    }
    fn active(&mut self) -> Result<bool, String> {
        Ok(self.vm.world.special_effects.active)
    }
    fn residential(&mut self) -> Result<bool, String> {
        if let (Some(lot), Some(person)) = (self.vm.world.globals.get(&10), &self.vm.world.person) {
            return Ok(crate::zoning::is_residential(&person.lot_zoning, *lot));
        }
        // Authored worlds already supply this same 40745f input for actions.
        self.vm
            .world
            .actions
            .as_ref()
            .map(|a| a.live_family)
            .ok_or("Special Effect residential context absent".into())
    }
    fn actor_family_permits(&mut self) -> Result<bool, String> {
        let actor = &self.vm.world.objects[&self.thread.stack.last().unwrap().caller];
        let definition = actor
            .definition
            .as_ref()
            .ok_or("Special Effect caller definition absent")?;
        let kind = self
            .program
            .modules
            .get(&definition.source)
            .and_then(|m| m.object_definition_words.get(&definition.object))
            .and_then(|w| w.get(9))
            .ok_or("Special Effect caller kind absent")?;
        if *kind != 2 {
            return Ok(false);
        }
        let Some(person) = &actor.person else {
            return Ok(false);
        };
        let controlled = self
            .vm
            .world
            .person
            .as_ref()
            .ok_or("Special Effect family context absent")?
            .controlled_family;
        let family = *person
            .get(&61)
            .ok_or("Special Effect caller family absent")?;
        Ok(family == controlled
            || controlled == 0
                && *person
                    .get(&77)
                    .ok_or("Special Effect caller family flag absent")?
                    != 0)
    }
    fn notify(&mut self) -> Result<(), String> {
        // 40760d(0x10c,0) targets the native window presenter. There is no
        // Windows widget owner in this host; the emission retains its flags.
        Ok(())
    }
    fn text(&mut self, entry: u8) -> Result<String, String> {
        // The resource manager resolves the frame's BHAV, including global and
        // semiglobal code. Its resource is distinct from private code_owner.
        let frame = self.thread.stack.last().unwrap();
        Ok(self
            .program
            .modules
            .get(&frame.location.source)
            .and_then(|m| m.strings.get(&305))
            .and_then(|s| s.get(usize::from(entry) - 1))
            .cloned()
            .unwrap_or_default())
    }
    fn format(&mut self, text: String) -> Result<String, String> {
        crate::dialog::format_text(
            self.program,
            &self.vm.world,
            self.thread.stack.last().unwrap(),
            text,
            &mut None,
        )
    }
    fn emit(&mut self, e: Emission) -> Result<(), String> {
        if self.vm.effects.len() >= 4096 {
            return Err("unconsumed Special Effect presentation queue".into());
        }
        self.vm.effects.push(crate::SpecialEffect {
            tick: self.vm.tick,
            object: e.object,
            duration_ms: e.duration_ms,
            kind: e.kind,
            size: e.size,
            flags: e.flags,
            text: e.text,
        });
        Ok(())
    }
}

impl crate::Vm {
    pub(crate) fn special_effect(
        &mut self,
        program: &crate::Program,
        thread: &mut crate::Thread,
        instruction: &sims_formats::behavior::Instruction,
    ) -> Result<String, String> {
        if thread.check {
            return Err("Special Effect yielded in synchronous callback".into());
        }
        let mut state = thread.stack.last().unwrap().continuation;
        let result = step(
            &mut state,
            instruction.operands,
            &mut Live {
                vm: self,
                program,
                thread,
            },
        );
        // No script executes inside these synchronous host calls. Publish on
        // failure too, preserving the native pre-lookup phase write.
        thread.stack.last_mut().unwrap().continuation = state;
        match result? {
            Outcome::Complete => {
                thread.stack.last_mut().unwrap().continuation = 0;
                self.branch(program, thread, instruction.on_true)?;
                Ok("Special Effect completed; branch true".into())
            }
            Outcome::Yield => {
                thread.status = crate::Status::Sleeping {
                    wake_tick: self.tick.checked_add(1).ok_or("effect tick overflow")?,
                };
                Ok("Special Effect yielded".into())
            }
        }
    }
}
