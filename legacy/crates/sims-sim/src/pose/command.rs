//! Original 58950b Animate Sim decisions over the live native animator.
//! Asset lookup, the person interrupt predicate and frame writes are host
//! boundaries. No legacy frame cursor or separate event queue is constructed.
use super::{
    animator::{Animator, Slot},
    ordinary,
};
use crate::character::Controls;
use serde::{Deserialize, Serialize};
use sims_formats::{animation::Animation, avatar::Skeleton};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lookup {
    Operand,
    Posture(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Waiting,
    Completed,
    Event(i16),
}
#[derive(Debug, Clone, Copy)]
pub struct Input {
    /// Argument indirection has already been validated and resolved, after the
    /// native mode gate. The operand is a signed word, including negative IDs.
    pub number: i16,
    pub flags: u8,
    pub event_count: i8,
    pub tick: i32,
    pub definition_flags: u8,
}

fn field(person: &BTreeMap<i16, i16>, key: i16) -> Result<i16, String> {
    person
        .get(&key)
        .copied()
        .ok_or_else(|| format!("native animation person field {key} absent"))
}
fn restore_posture(control: &mut Controls, person: &BTreeMap<i16, i16>) -> Result<(), String> {
    control.ordinary.mode =
        sims_vm::animation::posture_mode(field(person, 0)?, control.ordinary.mode as u32) as i32;
    Ok(())
}
fn repeat(animator: &mut Animator, slot: Slot, priority: i32) -> Result<(), String> {
    if let Some(id) = animator.slot(slot) {
        animator.repeat(id)?;
        animator.set_priority(id, priority)?;
    }
    Ok(())
}

/// One primitive invocation. The host must distinguish an actual absent asset
/// (None) from an unresolved/decode-failed resource (Err). It can stage the
/// native state transactionally while asynchronously loading a requested asset.
pub fn invoke<'a>(
    animator: &mut Animator,
    control: &mut Controls,
    person: &mut BTreeMap<i16, i16>,
    skeleton: &Skeleton,
    input: Input,
    mut lookup: impl FnMut(Lookup) -> Result<Option<&'a Animation>, String>,
    mut missing: impl FnMut(Slot) -> Result<(), String>,
    mut interrupt: impl FnMut() -> Result<bool, String>,
    mut write: impl FnMut(i16) -> Result<(), String>,
) -> Result<Outcome, String> {
    if !(0..=3).contains(&control.ordinary.mode) {
        return Ok(Outcome::Waiting);
    }
    if animator.busy() || control.movement.busy() {
        return Err("Animate Sim invoked during a pending native animator update".into());
    }
    let now = |control: &Controls| ordinary::clock(input.tick, control.ordinary.clock_mode);
    let settings = control.settings;
    let mode = sims_vm::animation::mode(input.flags);
    let reverse = input.flags & 2 != 0;
    if matches!(mode, 1 | 2) {
        let slot = if mode == 1 { Slot::Base } else { Slot::Carry };
        if input.number == 0 {
            animator.clear(slot, now(control), settings)?;
            if mode == 2 {
                control.ordinary.carry.mode = 0;
            }
        } else {
            let clip = lookup(Lookup::Operand)?;
            control.ordinary.clock_mode = input.flags >> 7;
            if !animator.start(slot, skeleton, clip, reverse, now(control), settings)? {
                missing(slot)?;
            }
            if mode == 2 {
                // An absent carry layer returns before resetting +1ac.
                if animator.slot(slot).is_none() {
                    return Ok(Outcome::Completed);
                }
                control.ordinary.carry.mode = 6;
            }
            repeat(animator, slot, if mode == 1 { -10 } else { 10 })?;
        }
        if mode == 2 {
            control.ordinary.carry.selection = -1;
        }
        return Ok(Outcome::Completed);
    }
    if input.number == 0 {
        animator.clear_foreground(now(control), settings)?;
        restore_posture(control, person)?;
        person.insert(19, 0);
        control.ordinary.pickup = 0;
        // This branch does not clear +a0 or synchronously rebuild the base.
        return Ok(Outcome::Completed);
    }
    if control.engaged == 0 {
        let clip = lookup(Lookup::Operand)?;
        animator.clear_numbered_events()?;
        animator.clear_foreground(now(control), settings)?;
        control.ordinary.clock_mode = input.flags >> 7;
        if !animator.start(
            Slot::Foreground,
            skeleton,
            clip,
            reverse,
            now(control),
            settings,
        )? {
            missing(Slot::Foreground)?;
            let fallback = lookup(Lookup::Posture(37))?;
            animator.start(
                Slot::Foreground,
                skeleton,
                fallback,
                false,
                now(control),
                settings,
            )?;
        }
        let foreground = animator
            .slot(Slot::Foreground)
            .ok_or("native foreground and fallback could not start")?;
        animator.set_interruptible(foreground, i32::from(input.flags & 8 != 0))?;
        control.ordinary.clock_mode = input.flags >> 7;
        control.movement.state.apply_rate = (input.flags >> 6) & 1;
        if mode == 3 {
            animator.clear(Slot::Carry, now(control), settings)?;
            control.ordinary.carry.selection = -1;
            control.ordinary.pickup = 1;
        }
        control.animation_events = 0;
        control.ordinary.mode = 0;
        animator.set_selection(-1)?;
        control.engaged = 1;
    }
    if let Some(event) = animator.pop_numbered_event() {
        let event = if event < 100 {
            let expected = if reverse {
                i32::from(input.event_count)
                    .wrapping_sub(control.animation_events)
                    .wrapping_sub(1)
            } else {
                control.animation_events
            };
            if expected < 0
                || expected >= i32::from(input.event_count)
                || event != expected
                || event < 0
            {
                return Ok(Outcome::Waiting);
            }
            // For a real event, the native counter precedes the frame write.
            control.animation_events = control.animation_events.wrapping_add(1);
            expected
        } else {
            event
        };
        write(event as i16)?;
        return Ok(Outcome::Event(event as i16));
    }
    if !animator.finished(Slot::Foreground)? {
        let layer = animator
            .slot(Slot::Foreground)
            .and_then(|id| animator.layer(id))
            .ok_or("active foreground absent")?;
        if layer.interruptible == 0 || !interrupt()? {
            return Ok(Outcome::Waiting);
        }
    }
    if control.animation_events < i32::from(input.event_count) {
        let event = if reverse {
            i32::from(input.event_count)
                .wrapping_sub(control.animation_events)
                .wrapping_sub(1)
        } else {
            control.animation_events
        };
        write(event as i16)?;
        // Substitute events increment only AFTER a successful frame write.
        control.animation_events = control.animation_events.wrapping_add(1);
        return Ok(Outcome::Event(event as i16));
    }
    control.engaged = 0;
    restore_posture(control, person)?;
    // Native 590430 base rebuild; retain the foreground even when interrupted.
    super::posture::update(
        animator,
        &mut control.ordinary,
        super::posture::Input {
            posture: field(person, 0)?,
            species: field(person, 65)?,
            definition_flags: input.definition_flags,
            tick: input.tick,
        },
        skeleton,
        settings,
        |index| lookup(Lookup::Posture(index)),
    )?;
    control.ordinary.pickup = 0;
    person.insert(19, 0);
    Ok(Outcome::Completed)
}
