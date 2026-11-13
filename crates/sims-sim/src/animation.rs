//! Deterministic 30-fps animation cursors advanced at 25 simulation ticks/second.
//! A fifth-frame integer represents the TS1 reference speed 30/25 without drift.
use crate::{Event, Simulation};
use serde::{Deserialize, Serialize};
use sims_formats::animation::Animation;
use sims_vm::{AnimationOutcome, AnimationRequest, ObjectId, Program, SoundEvent, ThreadId};
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Playback {
    /// Native foreground skin +0x68, set by operand flag 0x08.
    pub interruptible: bool,
    /// Native pickup foreground (+0x269); suppresses a separate carry layer.
    pub pickup: bool,
    pub reverse: bool,
    pub clip: String,
    pub thread: ThreadId,
    pub started_tick: u64,
    pub last_tick: u64,
    pub frame_fifths: u64,
    pub speed: u64,
    pub next_event: usize,
    pub pending: VecDeque<i16>,
    /// Queue entries consumed, including out-of-order events ignored natively.
    pub delivered: usize,
    pub numbered_events: i32,
    pub substitute_events: u8,
    pub event_count: i8,
    pub last_yield_tick: Option<u64>,
    pub ended: bool,
    /// The primitive has returned true, but the foreground pose is retained
    /// until another animation/reset replaces it (native animator +0x0c).
    pub completed: bool,
    pub looping: bool,
    /// Left/right presentation gestures; no skeleton or graphics device is needed.
    pub hands: [i16; 2],
}
pub(crate) fn validate_clip(clip: &Animation, looping: bool) -> Result<(), String> {
    if clip.frames == 0 || clip.frames > 108_001 {
        return Err("invalid animation frame count".into());
    }
    for event in &clip.events {
        if event.time_ms >= 0 && event.time_ms as u64 * 30 > u64::from(clip.frames) * 1000 {
            return Err("animation event exceeds duration".into());
        }
        for (key, value) in &event.pairs {
            use sims_vm::animation::{Property, property};
            match property(key, value)? {
                Property::Event(_) if looping => {
                    return Err("looping numbered animation events unsupported".into());
                }
                Property::Unsupported => {
                    return Err(format!("unsupported animation time property {key}"));
                }
                Property::Sound if value.is_empty() || value.len() > 1024 => {
                    return Err("invalid animation sound".into());
                }
                _ => (),
            }
        }
    }
    Ok(())
}
fn playback_events(
    clip: &Animation,
    reverse: bool,
) -> Vec<&sims_formats::animation::TimedProperty> {
    if reverse {
        clip.events
            .iter()
            .rev()
            .filter(|e| e.time_ms >= 0)
            .collect()
    } else {
        clip.events.iter().filter(|e| e.time_ms >= 0).collect()
    }
}
fn event_count(clip: &Animation, frame: u64, reverse: bool) -> usize {
    playback_events(clip, reverse).partition_point(|e| {
        let time = e.time_ms as u64 * 150;
        let time = if reverse {
            u64::from(clip.frames.saturating_sub(1)) * 5000
                - time.min(u64::from(clip.frames.saturating_sub(1)) * 5000)
        } else {
            time
        };
        time <= frame * 1000
    })
}
impl Simulation {
    /// Missing clips requested by suspended Animate Sim primitives or automatic
    /// carry selection. The content
    /// host resolves these exact embedded names without loading every unused
    /// StringSet entry. This query never advances or completes a primitive.
    pub fn pending_animation_assets(
        &self,
        program: &Program,
    ) -> Result<std::collections::BTreeSet<String>, String> {
        let mut names = self.pending_required_animation_assets(program)?;
        names.extend(self.pending_native_command_animation_assets(program)?);
        Ok(names)
    }
    pub fn pending_native_command_animation_assets(
        &self,
        program: &Program,
    ) -> Result<std::collections::BTreeSet<String>, String> {
        let mut names = std::collections::BTreeSet::new();
        for (&id, thread) in &self.vm.threads {
            let sims_vm::Status::AwaitingAnimation(request) = &thread.status else {
                continue;
            };
            if self.characters.contains_key(&request.caller) {
                if let Some(name) = self.native_command_asset(program, id, request)? {
                    names.insert(name);
                }
            }
        }
        Ok(names)
    }
    pub fn pending_required_animation_assets(
        &self,
        program: &Program,
    ) -> Result<std::collections::BTreeSet<String>, String> {
        let mut names = self.pending_carry_assets(program)?;
        names.extend(self.native_update_asset_names(program)?);
        names.extend(self.native_reach_asset_names(program));
        for (&id, t) in &self.vm.threads {
            let sims_vm::Status::AwaitingAnimation(request) = &t.status else {
                continue;
            };
            if self.characters.contains_key(&request.caller) {
                continue;
            }
            if matches!(sims_vm::animation::mode(request.operands[5]), 1 | 2)
                && request.operands[..2] == [0, 0]
            {
                continue;
            }
            let (name, _, _) = self.animation_name(program, id, request)?;
            if !program.animations.contains_key(&name) {
                names.insert(name);
            }
        }
        Ok(names)
    }
    fn animation_name(
        &self,
        program: &Program,
        id: ThreadId,
        request: &AnimationRequest,
    ) -> Result<(String, bool, u64), String> {
        let bytes = request.operands;
        let number = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
        // ID 0 resets to the actor's posture animation; the source byte is ignored.
        // The original stand function uses source 1 here.
        if bytes[3] != 0
            || bytes[7] != 0
            || (number != 0 && bytes[4] != 0 && bytes[4] != 3)
            || bytes[5] & !0x7b != 0
            || ![0, 1, 2, 3].contains(&sims_vm::animation::mode(bytes[5]))
        {
            return Err(format!(
                "unsupported Animate Sim source/mode/flags/event-count: {bytes:02X?}"
            ));
        }
        let frame = self.vm.threads[&id]
            .stack
            .last()
            .ok_or("missing animation frame")?;
        // The native event destination is validated only when an event is
        // delivered. Eventless clips can carry unused out-of-range local bytes.
        let actor = &self.vm.world.objects[&request.caller];
        let person = actor
            .person
            .as_ref()
            .ok_or("animation caller must be a person")?;
        let age = *person.get(&58).ok_or("actor age field is missing")?;
        if number == 0 {
            person.get(&42).ok_or("head seek state field is missing")?;
            let posture = *person.get(&0).ok_or("actor posture field is missing")?;
            if !(0..=2).contains(&posture) {
                return Err(
                    "default animation for this posture requires saved native animator mode".into(),
                );
            }
            let index = sims_vm::animation::posture_mode(posture, 0) as usize;
            let special = *person
                .get(&64)
                .ok_or("actor special animation field is missing")?;
            let species = *person.get(&65).ok_or("actor species field is missing")?;
            let name =
                sims_vm::animation::posture_name(age, special, species, index, |table, index| {
                    let (source, number) = match table {
                        sims_vm::animation::NameTable::Private(number) => (
                            actor.definition.as_ref().map(|d| d.source.as_str())?,
                            number,
                        ),
                        sims_vm::animation::NameTable::Global(number) => {
                            (program.globals.as_str(), number)
                        }
                    };
                    program
                        .modules
                        .get(source)?
                        .strings
                        .get(&number)?
                        .get(index)
                        .map(String::as_str)
                })
                .ok_or("missing native default posture animation")?;
            Ok((name.to_ascii_lowercase(), true, 6))
        } else if bytes[4] == 3 {
            let table = sims_vm::animation::global_action_table(age);
            let name = program
                .modules
                .get(&program.globals)
                .and_then(|m| m.strings.get(&table))
                .and_then(|v| v.get(number))
                .filter(|s| !s.is_empty())
                .ok_or("missing global age-profile animation string")?;
            Ok((name.to_ascii_lowercase(), false, 6))
        } else {
            let owner = program
                .modules
                .get(&frame.code_owner)
                .ok_or("missing animation code owner")?;
            let def = self
                .vm
                .world
                .objects
                .get(&frame.callee)
                .and_then(|o| o.definition.as_ref())
                .ok_or("animation code owner needs object definition")?;
            if def.source != frame.code_owner {
                return Err("animation owner differs from callee definition".into());
            }
            let foreground = sims_vm::animation::mode(bytes[5]) == 0;
            let active = self
                .vm
                .world
                .actions
                .as_ref()
                .and_then(|s| s.actors.get(&request.caller))
                .and_then(|a| a.active.as_ref());
            let active_matches = active.is_some_and(|a| a.target == frame.callee);
            let icon_age = if foreground && active_matches {
                active
                    .and_then(|a| self.vm.world.objects.get(&a.icon_owner))
                    .filter(|o| {
                        o.definition
                            .as_ref()
                            .and_then(|d| {
                                program
                                    .modules
                                    .get(&d.source)?
                                    .object_definition_words
                                    .get(&d.object)?
                                    .get(9)
                            })
                            .copied()
                            == Some(2)
                    })
                    .and_then(|o| o.person.as_ref())
                    .map(|p| {
                        p.get(&58)
                            .copied()
                            .ok_or("animation partner age field is missing")
                    })
                    .transpose()?
            } else {
                None
            };
            let table =
                sims_vm::animation::object_action_table(age, foreground, active_matches, icon_age);
            let table = owner
                .strings
                .get(&table)
                .ok_or("missing object animation table")?;
            let name = table
                .get(number)
                .filter(|s| !s.is_empty())
                .ok_or("animation index outside string table")?;
            let walk = *actor.data.get(&17).ok_or("walk style field is missing")?;
            if !matches!(walk, 0 | 1) {
                return Err("unsupported walk style".into());
            }
            Ok((
                name.to_ascii_lowercase(),
                false,
                if walk == 1 && bytes[5] & 64 != 0 {
                    12
                } else {
                    6
                },
            ))
        }
    }
    pub(crate) fn service_animation(
        &mut self,
        program: &Program,
        id: ThreadId,
        request: &AnimationRequest,
        events: &mut Vec<Event>,
    ) -> Result<bool, String> {
        if self.characters.contains_key(&request.caller) {
            return self.service_native_animation(program, id, request, events);
        }
        if sims_vm::animation::mode(request.operands[5]) == 1 {
            let number = u16::from_le_bytes([request.operands[0], request.operands[1]]);
            if number == 0 {
                self.backgrounds.remove(&request.caller);
            } else {
                if request.operands[5] & 2 != 0 {
                    return Err("reverse background layer unsupported".into());
                }
                let (name, _, _) = self.animation_name(program, id, request)?;
                let clip = &program
                    .animations
                    .get(&name)
                    .ok_or_else(|| format!("missing background animation {name}"))?
                    .clip;
                if clip.frames == 0 || !clip.events.is_empty() {
                    return Err("timed background animation events unsupported".into());
                }
                self.backgrounds.insert(
                    request.caller,
                    crate::carry::Layer {
                        clip: name,
                        started_tick: self.vm.tick,
                    },
                );
            }
            self.vm
                .finish_animation(program, id, request.id, AnimationOutcome::Completed)?;
            return Ok(true);
        }
        if sims_vm::animation::mode(request.operands[5]) == 2 {
            if request.operands[5] & 2 != 0 {
                return Err("reverse script carry layers unsupported".into());
            }
            let number = u16::from_le_bytes([request.operands[0], request.operands[1]]);
            let layer = if number == 0 {
                None
            } else {
                let (name, _, _) = self.animation_name(program, id, request)?;
                let clip = &program
                    .animations
                    .get(&name)
                    .ok_or_else(|| format!("missing carry animation {name}"))?
                    .clip;
                if clip.frames == 0 || !clip.events.is_empty() {
                    return Err("timed script carry events unsupported".into());
                }
                Some(crate::carry::Layer {
                    clip: name,
                    started_tick: self.vm.tick,
                })
            };
            let state = self.carry.entry(request.caller).or_default();
            state.selector.script_clip(layer.is_some());
            state.layer = layer;
            self.vm
                .finish_animation(program, id, request.id, AnimationOutcome::Completed)?;
            return Ok(true);
        }
        let (name, looping, speed) = self.animation_name(program, id, request)?;
        let reverse = request.operands[5] & 2 != 0;
        let asset = program
            .animations
            .get(&name)
            .ok_or_else(|| format!("missing loaded animation {name}"))?;
        validate_clip(&asset.clip, looping)?;
        if reverse && looping {
            return Err(
                "reverse timed properties and reverse posture loops are unsupported".into(),
            );
        }
        if looping
            || self
                .animations
                .get(&request.caller)
                .is_none_or(|p| p.clip != name || p.looping || p.completed || p.reverse != reverse)
        {
            if sims_vm::animation::mode(request.operands[5]) == 3 {
                if let Some(c) = self.carry.get_mut(&request.caller) {
                    c.layer = None;
                    c.selector.selection = -1;
                }
            }
            self.animations.insert(
                request.caller,
                Playback {
                    interruptible: request.operands[5] & 8 != 0,
                    pickup: sims_vm::animation::mode(request.operands[5]) == 3,
                    reverse,
                    clip: name.clone(),
                    thread: id,
                    started_tick: self.vm.tick,
                    last_tick: self.vm.tick,
                    frame_fifths: 0,
                    speed,
                    next_event: 0,
                    pending: VecDeque::new(),
                    delivered: 0,
                    numbered_events: 0,
                    substitute_events: 0,
                    event_count: request.operands[6] as i8,
                    last_yield_tick: None,
                    ended: false,
                    completed: false,
                    looping,
                    hands: [0; 2],
                },
            );
            self.event(events,id,format!("started {}animation {name}: {} frames, {} ms in source, speed {speed}/5 frames per tick",if looping {"idle loop "} else {""},asset.clip.frames,asset.clip.duration_ms));
            if looping {
                let person = self
                    .vm
                    .world
                    .objects
                    .get_mut(&request.caller)
                    .unwrap()
                    .person
                    .as_mut()
                    .unwrap();
                person.insert(19, 0);
                let head = person
                    .get_mut(&42)
                    .ok_or("head seek state field is missing")?;
                if *head == 1 {
                    *head = 4;
                }
                self.vm
                    .finish_animation(program, id, request.id, AnimationOutcome::Completed)?;
                return Ok(true);
            }
        }
        // Original 0x58950b consults the existing person interrupt predicate
        // only after finding an empty event queue and an unfinished clip.
        let player = &self.animations[&request.caller];
        let interrupted = if player.interruptible && !player.ended && player.pending.is_empty() {
            let actions = self
                .vm
                .world
                .actions
                .as_ref()
                .ok_or("interruptible animation needs action storage")?;
            actions
                .actors
                .get(&request.caller)
                .ok_or("missing animation actor queue")?
                .queue
                .should_interrupt(&self.vm.world.objects[&request.caller], actions.live_family)?
        } else {
            false
        };
        let player = self.animations.get_mut(&request.caller).unwrap();
        if player.thread != id
            || player.speed != speed
            || player.event_count != request.operands[6] as i8
            || player.interruptible != (request.operands[5] & 8 != 0)
        {
            return Err("animation continuation changed owner or playback speed".into());
        }
        if player.last_yield_tick == Some(self.vm.tick) {
            return Ok(false);
        }
        let event = player.pending.pop_front();
        if event.is_some() {
            player.delivered += 1;
        }
        let result = sims_vm::animation::poll_events(
            player.event_count,
            reverse,
            &mut player.numbered_events,
            event,
            player.ended || interrupted,
        );
        if let sims_vm::animation::Poll::Event(value) = result {
            if event.is_none() {
                player.substitute_events += 1;
            }
            self.vm
                .finish_animation(program, id, request.id, AnimationOutcome::Event(value))?;
            self.event(
                events,
                id,
                format!("delivered xevt {value}; animation false branch, cursor preserved"),
            );
            Ok(true)
        } else if result == sims_vm::animation::Poll::Completed {
            self.vm
                .world
                .objects
                .get_mut(&request.caller)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .insert(19, 0);
            self.vm
                .finish_animation(program, id, request.id, AnimationOutcome::Completed)?;
            // 0x58950b clears the active continuation and rebuilds the
            // background posture, but retains the foreground clip. An early
            // interrupted return does not move its playback cursor to the end.
            let player = self.animations.get_mut(&request.caller).unwrap();
            player.completed = true;
            player.pickup = false;
            self.event(
                events,
                id,
                format!(
                    "animation {name} primitive completed; interrupted={interrupted}; branch true"
                ),
            );
            Ok(true)
        } else {
            player.last_yield_tick = Some(self.vm.tick);
            Ok(false)
        }
    }
    pub(crate) fn advance_animations(
        &mut self,
        program: &Program,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let ids: Vec<_> = self.animations.keys().copied().collect();
        for actor in ids {
            if self.characters.contains_key(&actor) {
                return Err("native character also owns a legacy animation cursor".into());
            }
            let mut p = self.animations[&actor].clone();
            if p.last_tick == self.vm.tick {
                continue;
            }
            if p.last_tick.checked_add(1) != Some(self.vm.tick) {
                return Err("animation clock skipped a tick".into());
            }
            let clip = &program
                .animations
                .get(&p.clip)
                .ok_or("active animation missing from program")?
                .clip;
            let limit = u64::from(clip.frames) * 5;
            let mut next = p.frame_fifths + p.speed;
            if p.looping {
                while next >= limit {
                    self.emit_animation_events(actor, clip, &mut p, limit, events)?;
                    next -= limit;
                    p.next_event = 0;
                }
            } else {
                next = next.min(limit);
                p.ended = next == limit;
            }
            self.emit_animation_events(actor, clip, &mut p, next, events)?;
            p.frame_fifths = next;
            p.last_tick = self.vm.tick;
            self.animations.insert(actor, p);
        }
        Ok(())
    }
    fn emit_animation_events(
        &mut self,
        actor: ObjectId,
        clip: &Animation,
        p: &mut Playback,
        frame: u64,
        events: &mut Vec<Event>,
    ) -> Result<(), String> {
        let end = event_count(clip, frame, p.reverse);
        for property in &playback_events(clip, p.reverse)[p.next_event..end] {
            for (key, value) in &property.pairs {
                use sims_vm::animation::{Property, property as decode_property};
                match decode_property(key, value)? {
                    Property::Event(v) => p.pending.push_back(v as i16),
                    Property::Hand(side, v) => {
                        p.hands[side] = v;
                        self.vm
                            .world
                            .objects
                            .get_mut(&actor)
                            .unwrap()
                            .person
                            .as_mut()
                            .unwrap()
                            .insert(22 + side as i16, v);
                    }
                    Property::Censor(v) => {
                        self.vm
                            .world
                            .objects
                            .get_mut(&actor)
                            .unwrap()
                            .person
                            .as_mut()
                            .unwrap()
                            .insert(30, v);
                    }
                    Property::Interruptible(v) => p.interruptible = v != 0,
                    Property::Footstep(_) | Property::Ignored => (),
                    Property::Sound => {
                        if self.vm.sounds.len() >= 4096 {
                            return Err("sound queue is full".into());
                        }
                        self.vm.sounds.push(SoundEvent {
                            stop: false,
                            tick: self.vm.tick,
                            object: actor,
                            name: value.clone(),
                            flags: 0,
                            sample_rate: 0,
                            volume: 0,
                        });
                    }
                    Property::Unsupported => {
                        return Err(format!("unsupported animation property {key}"));
                    }
                }
                self.event(
                    events,
                    p.thread,
                    format!(
                        "animation {} at {} ms: {key}={value}",
                        p.clip, property.time_ms
                    ),
                );
            }
        }
        p.next_event = end;
        Ok(())
    }
    pub(crate) fn validate_animations(&self, program: &Program) -> Result<(), String> {
        for (&actor, p) in &self.animations {
            let clip = &program
                .animations
                .get(&p.clip)
                .ok_or("saved animation resource is missing")?
                .clip;
            validate_clip(clip, p.looping)?;
            if p.completed
                && (p.pickup
                    || p.looping
                    || (!p.ended && !p.interruptible)
                    || p.numbered_events < i32::from(p.event_count))
            {
                return Err("invalid completed foreground animation".into());
            }
            if p.reverse && p.looping {
                return Err("unsupported saved reverse animation".into());
            }
            let t = self
                .vm
                .threads
                .get(&p.thread)
                .ok_or("animation owner thread is missing")?;
            // A synchronous callback temporarily changes the recipient's
            // check flag; its ordinary animation remains suspended underneath.
            // VM snapshot validation already binds every callback boundary.
            let ordinary_check = t
                .callback
                .as_ref()
                .map_or(t.check, |b| b.chain().last().unwrap().resume_check);
            if (!self.bodies.contains_key(&actor) && !self.retains_native_offlot_pose(actor))
                || self.vm.world.objects[&actor].person.is_none()
                || (ordinary_check
                    && !p.completed
                    && !self
                        .immediate
                        .as_ref()
                        .is_some_and(|e| e.thread == p.thread)
                    && !self
                        .reach_callback
                        .as_ref()
                        .is_some_and(|e| e.thread == p.thread))
                || t.stack.first().is_some_and(|f| f.caller != actor)
            {
                return Err("invalid animation actor/owner".into());
            }
            if p.started_tick > p.last_tick
                || p.last_tick > self.vm.tick
                || !matches!(p.speed, 6 | 12)
            {
                return Err("invalid animation clock".into());
            }
            let elapsed = (p.last_tick - p.started_tick)
                .checked_mul(p.speed)
                .ok_or("animation clock overflow")?;
            let limit = u64::from(clip.frames) * 5;
            let expected = if p.looping {
                elapsed % limit
            } else {
                elapsed.min(limit)
            };
            let cursor = if elapsed == 0 {
                0
            } else {
                event_count(clip, expected, p.reverse)
            };
            if p.frame_fifths != expected
                || p.next_event != cursor
                || p.ended != (!p.looping && elapsed >= limit)
            {
                return Err("animation cursor disagrees with clock".into());
            }
            let emitted: Vec<_> = playback_events(clip, p.reverse)[..cursor]
                .iter()
                .flat_map(|e| e.pairs.iter())
                .filter(|(k, _)| k == "xevt")
                .map(|(_, v)| v.parse::<i16>().unwrap())
                .collect();
            if p.delivered > emitted.len()
                || p.pending.iter().copied().collect::<Vec<_>>() != emitted[p.delivered..]
            {
                return Err("saved animation event queue/cursor mismatch".into());
            }
            if p.last_yield_tick
                .is_some_and(|t| t < p.started_tick || t > p.last_tick)
                || (p.substitute_events != 0 && !p.ended && !p.interruptible)
                || i32::from(p.substitute_events) > i32::from(p.event_count).max(0)
            {
                return Err("invalid animation event continuation".into());
            }
            let mut numbered = 0;
            for &event in &emitted[..p.delivered] {
                sims_vm::animation::poll_events(
                    p.event_count,
                    p.reverse,
                    &mut numbered,
                    Some(event),
                    false,
                );
            }
            for _ in 0..p.substitute_events {
                if !matches!(
                    sims_vm::animation::poll_events(
                        p.event_count,
                        p.reverse,
                        &mut numbered,
                        None,
                        true
                    ),
                    sims_vm::animation::Poll::Event(_)
                ) {
                    return Err("invalid substitute animation event count".into());
                }
            }
            if !matches!(t.status, sims_vm::Status::AwaitingReach(_))
                && !self
                    .reach_callback
                    .as_ref()
                    .is_some_and(|e| e.thread == p.thread)
                && numbered != p.numbered_events
            {
                return Err("animation numbered event cursor mismatch".into());
            }
        }
        Ok(())
    }
}
