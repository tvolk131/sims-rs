//! Automatic right-arm carry layer. Foreground pickup temporarily suppresses it.
use crate::Simulation;
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program, animation::CarrySelector};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Layer {
    pub clip: String,
    pub started_tick: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Carry {
    pub selector: CarrySelector,
    pub layer: Option<Layer>,
}
fn automatic_name(p: &Program, index: u16) -> Result<String, String> {
    p.modules
        .get(&p.globals)
        .and_then(|m| m.strings.get(&150))
        .and_then(|s| s.get(index as usize))
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| "automatic carry animation string not loaded".into())
}
impl Simulation {
    pub(crate) fn pending_carry_assets(
        &self,
        p: &Program,
    ) -> Result<std::collections::BTreeSet<String>, String> {
        let mut names = std::collections::BTreeSet::new();
        for (&actor, o) in &self.vm.world.objects {
            if o.person.is_none() || self.characters.contains_key(&actor) {
                continue;
            }
            let held = o.slots.first().is_some_and(Option::is_some);
            let pickup = self.animations.get(&actor).is_some_and(|a| a.pickup);
            let mut selector = self
                .carry
                .get(&actor)
                .map(|c| c.selector.clone())
                .unwrap_or_default();
            if let Some(Some(index)) = selector.update(held, pickup) {
                let name = automatic_name(p, index)?;
                if !p.animations.contains_key(&name) {
                    names.insert(name);
                }
            }
        }
        Ok(names)
    }
    pub(crate) fn sync_carry(&mut self, p: &Program) -> Result<(), String> {
        for (&actor, object) in &self.vm.world.objects {
            if object.person.is_none() || self.characters.contains_key(&actor) {
                continue;
            }
            let held = object.slots.first().is_some_and(Option::is_some);
            if !held && !self.carry.contains_key(&actor) {
                continue;
            }
            let pickup = self.animations.get(&actor).is_some_and(|p| p.pickup);
            // Loading is a host suspension, not a native "resource absent"
            // result. Keep the old selector/layer intact until the content host
            // supplies the clip so that retry and save/resume remain equivalent.
            let mut state = self.carry.get(&actor).cloned().unwrap_or_default();
            if let Some(selection) = state.selector.update(held, pickup) {
                state.layer = None;
                if let Some(index) = selection {
                    let name = automatic_name(p, index)?;
                    let clip = p
                        .animations
                        .get(&name)
                        .ok_or_else(|| format!("automatic carry animation not loaded: {name}"))?;
                    if clip.clip.frames == 0 || !clip.clip.events.is_empty() {
                        return Err("timed carry-layer events not implemented".into());
                    }
                    state.layer = Some(Layer {
                        clip: name,
                        started_tick: self.vm.tick,
                    });
                }
            }
            self.carry.insert(actor, state);
        }
        Ok(())
    }
    pub(crate) fn validate_carry(&self, p: &Program) -> Result<(), String> {
        for (&id, c) in &self.carry {
            if self
                .vm
                .world
                .objects
                .get(&id)
                .is_none_or(|o| o.person.is_none())
                || ![0, 1, 6].contains(&c.selector.mode)
                || ![-1, 0].contains(&c.selector.selection)
                || (c.selector.mode != 6 && (c.selector.selection == 0) != c.layer.is_some())
                || (c.selector.mode == 6 && c.selector.selection != -1)
            {
                return Err("invalid saved automatic carry state".into());
            }
            if let Some(layer) = &c.layer {
                let name = p
                    .modules
                    .get(&p.globals)
                    .and_then(|m| m.strings.get(&150))
                    .and_then(|s| s.get(38))
                    .ok_or("missing saved carry resource")?;
                if layer.started_tick > self.vm.tick
                    || (c.selector.mode != 6 && layer.clip != name.to_ascii_lowercase())
                    || !p.animations.contains_key(&layer.clip)
                {
                    return Err("invalid saved carry layer".into());
                }
            }
        }
        Ok(())
    }
    pub fn carry_frame(&self, actor: ObjectId, fraction: f32) -> Option<(&str, f32)> {
        let layer = self.carry.get(&actor)?.layer.as_ref()?;
        Some((
            &layer.clip,
            ((self.vm.tick - layer.started_tick) as f32 + fraction) * 1.2,
        ))
    }
}
