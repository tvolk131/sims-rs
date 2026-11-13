//! World services used by the original adult Main. Supplied records are explicit
//! laboratory/loaded-world data, not a reconstruction of person construction.
use crate::{ObjectId, Program, Thread, Vm};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct InventoryEntry {
    pub kind: u8,
    pub guid: u32,
    pub count: i16,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Neighbor {
    pub relationships: crate::relationship::Table,
    pub dirty: bool,
    /// Native +20 cache. None preserves the uninitialized resource-constructor
    /// case; only the NBRS/default constructor supplies zero. A relationship
    /// notification can make recomputation valid before a value exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friend_count: Option<i32>,
    pub guid: u32,
    /// NBRS person-data revision (native record +0x18), independent of file version.
    pub person_revision: i32,
    pub person: BTreeMap<i16, i16>,
    pub inventory: Vec<InventoryEntry>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorldState {
    /// Native object registry +0x12; relationship writes also dirty both neighbors.
    pub relationships_dirty: bool,
    pub neighbors: BTreeMap<i16, Neighbor>,
    /// Exact native vector, including trailing vacant entries. Authored fixtures
    /// may omit it; dynamic native neighbor construction requires it.
    pub neighbor_slots: Option<Vec<Option<i16>>>,
    /// Loaded family membership, in neighbor IDs.
    pub families: BTreeMap<i16, Vec<i16>>,
    /// Native family +138 flags and +124 dirty byte, retained independently of membership.
    #[serde(default)]
    pub family_metadata: BTreeMap<i16, crate::family::Metadata>,
    /// Ordered 40760d(0xe8,0) notifications for the presentation host.
    #[serde(default)]
    pub family_changes: Vec<crate::family::Change>,
    /// Native neighborhood family vector after descending number/identity sort.
    /// Authored worlds may omit it when a query has only one matching family.
    #[serde(default)]
    pub family_order: Option<Vec<i16>>,
    /// Authored/imported family home numbers, distinct from family identities.
    pub family_homes: BTreeMap<i16, i16>,
    /// Neighborhood +0x11c: lot numbers ordered by distance from the loaded lot.
    /// Absence means the native load context has not supplied this vector.
    #[serde(default)]
    pub nearby_houses: Option<Vec<i32>>,
    pub free_will: bool,
    pub pet_autonomy_disabled: bool,
    pub controlled_family: i16,
    /// Successful native inventory queries/removals publish the UI dirty flag.
    /// A zero-quantity removal succeeds and publishes even without a token.
    pub inventory_dirty: bool,
    pub lot_zoning: BTreeMap<i16, i32>,
    /// Lazily populated in numeric registry order, then appended on registration.
    pub resource_instances: BTreeMap<u32, Vec<i16>>,
    pub object_order: Vec<i16>,
    pub person_order: Vec<i16>,
    pub tile_chains: Vec<Vec<i16>>,
    pub autonomy: Option<crate::autonomy::Context>,
}
/// 0x45962b adds these slots after the base resource/sentinel slot. The head
/// attachment is slot 1; the ordinary carried-object slot is 0 (height code 6).
pub const BUILTIN_SLOT_HEIGHTS: [i32; 5] = [6, 0, 1, 1, 1];
pub const BUILTIN_SLOT_BONES: [Option<&str>; 5] = [None, Some("HEAD"), None, None, None];
/// Native 45962b slot-record +1c. HEAD accepts the selection indicator.
pub const BUILTIN_SLOT_FLAGS: [u32; 5] = [0, 2, 0, 0, 0];

///45962b writes zero offsets, fixed heights, and the HEAD anchor for logical slot1.
/// Other anchor words are untouched heap storage (the poison-vector oracle
/// preserves them), so their absence is unknown rather than a native null.
pub fn builtin_slot_geometry() -> Vec<crate::slot_geometry::Slot> {
    BUILTIN_SLOT_HEIGHTS
        .iter()
        .enumerate()
        .map(|(i, &height)| crate::slot_geometry::Slot {
            offset: [0; 3],
            height,
            anchor: BUILTIN_SLOT_BONES[i]
                .map(|name| crate::slot_geometry::Anchor::Named(name.as_bytes().to_vec())),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessoryCommand {
    pub request: u64,
    pub origin: crate::Location,
    pub caller: ObjectId,
    pub name: String,
    pub remove: bool,
}

impl Vm {
    pub fn native_accessory_commands(
        &self,
        p: &Program,
    ) -> Result<Vec<(crate::ThreadId, AccessoryCommand)>, String> {
        if !self.native_accessory_host {
            return Ok(vec![]);
        }
        self.threads
            .iter()
            .filter(|(_, t)| matches!(t.status, crate::Status::AwaitingAccessory { .. }))
            .map(|(&id, _)| self.native_accessory_command(p, id).map(|r| (id, r)))
            .collect()
    }
    fn native_accessory_command(
        &self,
        p: &Program,
        id: crate::ThreadId,
    ) -> Result<AccessoryCommand, String> {
        if !self.native_accessory_host {
            return Err("native accessory host disabled".into());
        }
        let t = self.threads.get(&id).ok_or("accessory thread absent")?;
        let crate::Status::AwaitingAccessory {
            request,
            origin,
            caller,
            name,
        } = &t.status
        else {
            return Err("stale or duplicate accessory completion".into());
        };
        let ins = crate::current(p, t)?;
        let f = t.stack.last().ok_or("accessory frame absent")?;
        if *request == 0
            || *request >= self.next_request
            || ins.opcode != 6
            || ins.operands[1] == 1
            || f.location != *origin
            || f.caller != *caller
            || self.accessory_label(p, t, &ins.operands)? != *name
        {
            return Err("invalid native accessory request".into());
        }
        Ok(AccessoryCommand {
            request: *request,
            origin: origin.clone(),
            caller: *caller,
            name: name.clone(),
            remove: ins.operands[2] & 1 != 0,
        })
    }
    pub fn finish_native_accessory(
        &mut self,
        p: &Program,
        id: crate::ThreadId,
        request: &AccessoryCommand,
    ) -> Result<(), String> {
        if self.native_accessory_command(p, id)? != *request {
            return Err("mismatched native accessory completion".into());
        }
        let ins = crate::current(p, &self.threads[&id])?.clone();
        self.complete_host_primitive(p, id, true, ins.on_true, ins.on_false)
    }
    pub(crate) fn change_accessory(
        &mut self,
        p: &Program,
        t: &Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let f = t.stack.last().ok_or("missing accessory frame")?;
        self.world
            .objects
            .get(&f.caller)
            .filter(|o| o.person.is_some())
            .ok_or("Change Suit/Accessory requires a person (native error 0x05)")?;
        if b[1] == 1 {
            // 0x45d962 only changes native person fields here. Skin selection
            // happens later in the animator; removing a built-in outfit is a no-op.
            if b[2] & 1 == 0 {
                let person = self
                    .world
                    .objects
                    .get_mut(&f.caller)
                    .unwrap()
                    .person
                    .as_mut()
                    .unwrap();
                let variant = if b[0] < 8 {
                    0
                } else {
                    let previous = *person.get(&8).ok_or("missing current outfit field")?;
                    let old_variant = *person.get(&21).ok_or("missing outfit variant field")?;
                    let age = *person.get(&58).ok_or("missing person age field")?;
                    if age == 0 || age > 17 {
                        loop {
                            let next = (crate::rng::next(&mut self.rng_state) % 10) as i16;
                            if previous != i16::from(b[0]) || next != old_variant {
                                break next;
                            }
                        }
                    } else if previous == i16::from(b[0]) {
                        i16::from(old_variant == 0)
                    } else {
                        (crate::rng::next(&mut self.rng_state) & 1) as i16
                    }
                };
                person.insert(21, variant);
                person.insert(8, i16::from(b[0]));
            }
            return Ok((
                true,
                format!(
                    "built-in outfit {}{}",
                    b[0],
                    if b[2] & 1 != 0 {
                        " removal ignored"
                    } else {
                        " selected"
                    }
                ),
            ));
        }
        let name = self.accessory_label(p, t, b)?;
        let remove = b[2] & 1 != 0;
        // The native animator silently retains an empty skin on load failure.
        // Keep that unresolved resource-loading behavior explicit here.
        if !remove && !p.accessories.contains_key(&name) {
            return Err(format!("accessory appearance {name:?} has not been loaded"));
        }
        let accessories = &mut self.world.objects.get_mut(&f.caller).unwrap().accessories;
        // 0x58d88f removes all matching layers before appending the new skin.
        accessories.retain(|n| n != &name);
        if !remove {
            accessories.push(name.clone());
        }
        Ok((
            true,
            format!(
                "{} accessory {name:?}",
                if remove { "removed" } else { "added" }
            ),
        ))
    }

    pub(crate) fn accessory_label(
        &self,
        p: &Program,
        t: &Thread,
        b: &[u8; 8],
    ) -> Result<String, String> {
        let f = t.stack.last().ok_or("missing accessory frame")?;
        self.world
            .objects
            .get(&f.caller)
            .filter(|o| o.person.is_some())
            .ok_or("Change Suit/Accessory requires a person (native error 0x05)")?;
        let source = match b[1] {
            0 => &p.globals,
            2 => &f.code_owner,
            _ => return Err("invalid accessory resource (native error 0x2f)".into()),
        };
        let module = p
            .modules
            .get(source)
            .ok_or("missing accessory resource (native error 0x2f)")?;
        let name = module
            .strings
            .get(&304)
            .and_then(|s| s.get(usize::from(b[0])))
            .filter(|s| !s.is_empty())
            .ok_or("empty accessory name (native error 0x30)")?;
        if !name.is_ascii() || name.len() > 255 || name.contains('\0') {
            return Err("accessory name outside the supported native string profile".into());
        }
        let name = name.to_ascii_lowercase();
        Ok(name)
    }
    pub fn pending_accessory_assets(&self, p: &Program) -> std::collections::BTreeSet<String> {
        self.threads
            .values()
            .filter_map(|t| match &t.status {
                crate::Status::AwaitingAccessory { name, .. }
                    if !p.accessories.contains_key(name) =>
                {
                    Some(name.clone())
                }
                _ => None,
            })
            .collect()
    }
    pub fn resume_accessory_assets(&mut self, p: &Program) {
        if self.native_accessory_host {
            return;
        }
        for t in self.threads.values_mut() {
            if matches!(&t.status,crate::Status::AwaitingAccessory{name,..} if p.accessories.contains_key(name))
            {
                t.status = crate::Status::Ready;
            }
        }
    }

    pub(crate) fn change_action_string(
        &mut self,
        p: &Program,
        thread: crate::ThreadId,
        t: &Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        // A synchronous check clears native 0x6c4d6c. Residential autonomy
        // does not establish the three-pointer pie-menu builder context.
        // Such checks resolve the string but publish none. A live player-menu
        // builder instead owns explicit entries emitted during this callback.
        let f = t.stack.last().ok_or("missing action-string frame")?;
        let table = u16::from_le_bytes([b[0], b[1]]);
        let strings = p
            .modules
            .get(&f.code_owner)
            .and_then(|m| m.strings.get(&table))
            .ok_or("Change Action String resource is not loaded")?;
        let label = usize::from(b[4])
            .checked_sub(1)
            .and_then(|i| strings.get(i))
            .map_or("", String::as_str);
        // 44cc07 calls the same 44bf1a formatter as Dialog before selecting
        // the publication branch. Formatting does not depend on a live menu.
        let label = crate::dialog::format_action_text(p, &self.world, f, label.to_owned())?;
        if let Some(query) = &mut self.player_menu {
            //44ccbb supplies the frame's Stack Object if formatting left
            //parameter0 empty. Literal labels leave all four words zero.
            query.publish(thread, label.clone(), [f.stack_object, 0, 0, 0])?;
            return Ok((true, format!("published dynamic menu entry {label:?}")));
        }
        let published = !t.check
            && self
                .world
                .objects
                .get(&f.caller)
                .is_some_and(|o| o.person.is_some());
        if published {
            // 0x44cc07 selects the caller's embedded active record (+0xa48),
            // not the frame's Stack Object. 0x41f1ae copies its string then
            // sets bit 0x80 without changing the action identity or priority.
            let action = self
                .world
                .actions
                .as_mut()
                .and_then(|s| s.actors.get_mut(&f.caller))
                .and_then(|s| s.active.as_mut())
                .ok_or("live action label without an active action record is unsupported")?;
            action.label = Some(label.clone());
            action.flags |= 0x80;
        }
        Ok((
            true,
            format!("resolved literal action label {label:?}; published={published}"),
        ))
    }

    pub(crate) fn named_tree_binding(
        &self,
        p: &Program,
        frame: &crate::Frame,
        b: &[u8; 8],
    ) -> Result<Option<(String, u16)>, String> {
        let strings_owner = if b[2] & 1 != 0 {
            &p.globals
        } else {
            &frame.code_owner
        };
        let table = u16::from_le_bytes([b[0], b[1]]);
        let name = p
            .modules
            .get(strings_owner)
            .and_then(|m| m.strings.get(&table))
            .and_then(|s| usize::from(b[4]).checked_sub(1).and_then(|i| s.get(i)));
        let Some(name) = name.filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        if b[5] > 2 {
            return Err("Run Tree By Name mode invalid (native error 0x2c)".into());
        }
        self.resolve_named_tree(p, frame.stack_object, name)
    }
    pub fn resolve_named_tree(
        &self,
        p: &Program,
        target: ObjectId,
        name: &str,
    ) -> Result<Option<(String, u16)>, String> {
        let def = self
            .world
            .objects
            .get(&target)
            .and_then(|o| o.definition.as_ref())
            .ok_or("Run Tree By Name target missing (native error 0x15)")?;
        let module = p
            .modules
            .get(&def.source)
            .ok_or("named behavior module missing")?;
        if !name.is_ascii() {
            return Err("non-ASCII named behavior lookup is unsupported".into());
        }
        // Original 0x4021f2 searches private, semiglobal, global resources in
        // two passes. Exact labels precede the case-insensitive fallback.
        for insensitive in [false, true] {
            for m in std::iter::once(module)
                .chain(module.semiglobal.as_ref().and_then(|s| p.modules.get(s)))
                .chain(p.modules.get(&p.globals))
            {
                let mut matches = m.routines.iter().filter(|(_, r)| {
                    if insensitive {
                        r.name.eq_ignore_ascii_case(name)
                    } else {
                        r.name == name
                    }
                });
                if let Some((&bhav, _)) = matches.next() {
                    if matches.next().is_some() {
                        return Err("ambiguous named behavior directory ordering".into());
                    }
                    return Ok(Some((def.source.clone(), bhav)));
                }
            }
        }
        Ok(None)
    }
    pub(crate) fn push_target_tree(
        &mut self,
        p: &Program,
        t: &mut Thread,
        owner: String,
        bhav: u16,
        target: i16,
        continuation: u32,
    ) -> Result<(), String> {
        self.push_target_tree_args(p, t, owner, bhav, target, continuation, None)
    }
    pub(crate) fn push_target_tree_args(
        &mut self,
        p: &Program,
        t: &mut Thread,
        owner: String,
        bhav: u16,
        target: i16,
        continuation: u32,
        supplied: Option<&[i16]>,
    ) -> Result<(), String> {
        let frame = t.stack.last().ok_or("missing target-tree frame")?.clone();
        let (source, routine) = p.resolve(&owner, bhav)?;
        let (argc, locals) = crate::dimensions(routine)?;
        if t.stack.len() >= crate::MAX_STACK {
            return Err("named tree call stack limit".into());
        }
        let temps = self.world.objects[&frame.caller].temps;
        let values = supplied.unwrap_or(&temps);
        let args = values
            .get(..argc)
            .ok_or("named-tree arguments exceed eight temporaries")?
            .to_vec();
        let entry = values.first().copied().unwrap_or(0);
        let person = self.world.objects[&frame.caller].person.is_some();

        let count = if person {
            Some(
                *self.world.objects[&target]
                    .data
                    .get(&62)
                    .ok_or("missing named-tree target use count")?,
            )
        } else {
            None
        };
        let context = self.retained_entry_context(t, target, &owner)?;
        let offset =
            frame.storage_offset + crate::frame_words(frame.args.len(), frame.locals.len());
        crate::retained_frames::ensure_capacity(t, offset + crate::frame_words(argc, locals))?;
        self.wake_object(p, t, frame.caller)?;
        crate::save_frame_storage(t)?;
        t.frame_memory.resize(
            t.frame_memory
                .len()
                .max(offset + crate::frame_words(argc, locals)),
            Some(0),
        );
        let values = (0..locals)
            .map(|i| t.frame_memory[offset + 8 + argc + i].unwrap_or(0))
            .collect();
        let opaque_locals = (0..locals)
            .filter(|i| t.frame_memory[offset + 8 + argc + i].is_none())
            .collect();
        t.stack.last_mut().unwrap().continuation = continuation;
        crate::retained_frames::finish_entry(
            t,
            crate::Frame {
                continuation: 0,
                action_target: person.then_some(target),
                action_icon: false,
                code_owner: owner,
                location: crate::Location {
                    source: source.into(),
                    bhav,
                    instruction: crate::entry_pc(routine, entry)?,
                },
                caller: frame.caller,
                callee: target,
                stack_object: target,
                args,
                locals: values,
                opaque_locals,
                storage_offset: offset,
            },
            context,
        )?;
        if let Some(count) = count {
            self.world
                .objects
                .get_mut(&target)
                .unwrap()
                .data
                .insert(62, count.wrapping_add(1));
        }
        Ok(())
    }

    pub(crate) fn run_named_tree(
        &mut self,
        p: &Program,
        id: crate::ThreadId,
        t: &mut Thread,
        ins: &sims_formats::behavior::Instruction,
    ) -> Result<String, String> {
        let frame = t.stack.last().ok_or("missing named-tree frame")?.clone();
        let result = if frame.continuation == 3 {
            let call = self
                .named_calls
                .last()
                .ok_or("named callback continuation absent")?;
            if call.requester != id || call.depth != t.stack.len() {
                return Err("named callback return ordering mismatch".into());
            }
            let result = call.result.ok_or("named callback has not returned")?;
            self.named_calls.pop();
            result
        } else if frame.continuation == 2 {
            t.callback_result
                .take()
                .ok_or("named callback result missing")?
        } else if frame.continuation == 1 {
            t.last_result
        } else if let Some((owner, bhav)) = self.named_tree_binding(p, &frame, &ins.operands)? {
            let target = frame.stack_object;
            if ins.operands[5] == 0 || ins.operands[5] == 1 && target == frame.caller {
                t.stack.last_mut().unwrap().continuation = 2;
                self.push_callback_start(
                    p,
                    t,
                    crate::Start {
                        owner: &owner,
                        bhav,
                        caller: frame.caller,
                        callee: target,
                        stack_object: target,
                        args: self.world.objects[&frame.caller].temps.to_vec(),
                        check: true,
                    },
                )?;
                return Ok(format!(
                    "Run Tree By Name synchronously entered {bhav} on #{target}"
                ));
            }
            if ins.operands[5] == 1 {
                self.push_named_callback(p, id, t, owner, bhav, target)?;
                return Ok(format!(
                    "Run Tree By Name synchronously entered {bhav} on recipient VM #{target}"
                ));
            }
            self.push_target_tree(p, t, owner, bhav, target, 1)?;
            return Ok(format!("Run Tree By Name entered {bhav} on #{target}"));
        } else {
            false
        };
        t.stack.last_mut().unwrap().continuation = 0;
        t.last_result = result;
        self.branch(p, t, if result { ins.on_true } else { ins.on_false })?;
        Ok(format!("Run Tree By Name returned {result}"))
    }

    pub(crate) fn generic_person_call(
        &mut self,
        t: &Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let sub = u16::from_le_bytes([b[0], b[1]]);
        if sub == 0 {
            crate::neighborhood::finish_tutorial(&mut self.world)?;
            return Ok((
                true,
                "native tutorial completion retained in managed neighborhood and lot state".into(),
            ));
        }
        if sub == 43 {
            crate::magic_history::restore(&mut self.world)?;
            return Ok((true, "native retained magic-history synchronization".into()));
        }
        if sub == 4 {
            let neighbor = t
                .stack
                .last()
                .ok_or("family addition frame absent")?
                .stack_object;
            let result = crate::family::add(&mut self.world, neighbor, &mut crate::family::Live)?;
            return Ok((
                result,
                format!("native family addition neighbor #{neighbor} → {result}"),
            ));
        }
        if sub == 6 {
            let neighbor = t
                .stack
                .last()
                .ok_or("family removal frame absent")?
                .stack_object;
            let result =
                crate::family::remove(&mut self.world, neighbor, &mut crate::family::Live)?;
            return Ok((
                result,
                format!("native family removal neighbor #{neighbor} → {result}"),
            ));
        }
        if sub == 10 || sub == 11 {
            let caller = t
                .stack
                .last()
                .ok_or("fire registration frame missing")?
                .caller;
            if !self.world.objects.contains_key(&caller) {
                return Err("fire registration caller absent".into());
            }
            // 44fd29/44fd3a pass ESI (the executing object), not Stack Object.
            // Neither path performs a person cast or a fire-GUID test.
            if sub == 10 {
                self.world.fires.register(caller);
            } else {
                self.world.fires.unregister(caller);
            }
            return Ok((
                true,
                format!("native fire registry subcall {sub} for #{caller}"),
            ));
        }
        if sub == 12 {
            let frame = t.stack.last().ok_or("tile-depth frame missing")?;
            if frame.stack_object <= 0 || !self.world.objects.contains_key(&frame.stack_object) {
                return Err("tile-depth target missing (native error 0x0a)".into());
            }
            let p = self
                .world
                .positions
                .get(&frame.stack_object)
                .ok_or("tile-depth target position unavailable")?;
            // Native uses its cached signed tile bytes (column, row), not the
            // sub-tile point, height, or selected object's direction.
            let [x, y] = [(p.y >> 4) as i8, (p.x >> 4) as i8];
            let [x, y] = [i16::from(x), i16::from(y)];
            let camera = self.world.camera_rotation;
            if camera > 3 {
                return Err("invalid native camera rotation".into());
            }
            if camera != 0 && (!(0..64).contains(&x) || !(0..64).contains(&y)) {
                return Err("rotated tile-depth outside original 64x64 lookup table".into());
            }
            // 0x407704 initializes 0x434f58's three rotation maps. The
            // primitive writes -(rotated column + rotated row) to Temp 0.
            let [x, y] = match camera {
                0 => [x, y],
                1 => [63 - y, x],
                2 => [63 - x, 63 - y],
                _ => [y, 63 - x],
            };
            let value = -(x + y);
            self.world
                .objects
                .get_mut(&frame.caller)
                .ok_or("tile-depth caller missing")?
                .temps[0] = value;
            return Ok((true, format!("native camera tile-depth → {value}")));
        }
        if sub == 2 {
            let frame = t.stack.last().unwrap();
            if !self.world.objects.contains_key(&frame.stack_object) {
                return Err("change action icon target missing (native error 0x0a)".into());
            }
            if self.world.objects[&frame.caller].person.is_none() {
                return Err("change action icon caller is not a person (native error 0x1c)".into());
            }
            let actor = self
                .world
                .actions
                .as_mut()
                .and_then(|s| s.actors.get_mut(&frame.caller))
                .ok_or("change action icon needs native action storage")?;
            if let Some(action) = &mut actor.active {
                action.icon_owner = frame.stack_object;
            }
            return Ok((
                true,
                format!("active action icon owner → #{}", frame.stack_object),
            ));
        }
        if sub == 14 {
            // 44fe22 -> 46d9c4: fixed global 31 receives caller Temp 0.
            // The original setter writes one word, without notification/RNG.
            let caller = t
                .stack
                .last()
                .ok_or("global assignment frame missing")?
                .caller;
            let value = self
                .world
                .objects
                .get(&caller)
                .ok_or("global assignment caller missing")?
                .temps[0];
            *self
                .world
                .globals
                .get_mut(&31)
                .ok_or("global 31 storage missing")? = value;
            return Ok((true, format!("native global 31 assignment -> {value}")));
        }
        if sub == 15 {
            let caller = t.stack.last().unwrap().caller;
            let object = self
                .world
                .objects
                .get_mut(&caller)
                .ok_or("footprint caller missing")?;
            let mask = object.temps[0];
            let person = object
                .person
                .as_mut()
                .ok_or("footprint caller is not a person (native error 0x1c)")?;
            // The request and the applied extension are distinct native fields.
            // Clearing restores ordinary geometry; the authored spatial host
            // currently uses that ordinary collision proxy throughout.
            person.insert(72, mask);
            person.insert(73, 0);
            return Ok((
                true,
                format!("native temporary footprint request {mask}; applied extension cleared"),
            ));
        }
        if sub != 28 {
            return Err(format!("unsupported Generic Sims Call subfunction {sub}"));
        }
        let caller = t.stack.last().unwrap().caller;
        let lot = self.world.objects[&caller].temps[0];
        let value = match lot {
            0 | 90..=92 => 0,
            93..=98 => 1,
            _ => match self
                .world
                .person
                .as_ref()
                .ok_or("lot zoning unavailable")?
                .lot_zoning
                .get(&lot)
                .unwrap_or(&0)
            {
                1 => 0,
                2 => 1,
                _ => -1,
            },
        };
        self.world.objects.get_mut(&caller).unwrap().temps[0] = value;
        Ok((true, format!("native lot classification {lot}: {value}")))
    }
    /// 0x45d87f scans the person's target-use records (+0xb00), represented by
    /// our live action/functional frames. Queued menus and icon ownership do not
    /// establish use. Only mode 1 resolves another person; all other bytes use
    /// the executing person, including legacy/noncanonical operand values.
    pub(crate) fn test_interacting(
        &mut self,
        p: &Program,
        t: &mut Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let f = t.stack.last().ok_or("interaction predicate frame absent")?;
        let (caller, target) = (f.caller, f.stack_object);
        let actor = if b[0] == 1 {
            let actor = self.read(p, t, b[1], i16::from_le_bytes([b[2], b[3]]))?;
            if self
                .world
                .objects
                .get(&actor)
                .is_none_or(|o| o.person.is_none())
            {
                return Err("interaction predicate requires a person (native error 0x1c)".into());
            }
            actor
        } else {
            caller
        };
        let result = std::iter::once(&*t)
            .chain(self.threads.values())
            .any(|thread| {
                thread
                    .stack
                    .iter()
                    .any(|frame| frame.caller == actor && frame.action_target == Some(target))
                    || thread
                        .completed_actions
                        .iter()
                        .any(|r| r.0 == actor && r.1 == target)
            });
        Ok((
            result,
            format!("native interaction use #{actor} → #{target}: {result}"),
        ))
    }

    pub(crate) fn set_to_next(
        &mut self,
        p: &Program,
        t: &mut Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let (scope, index) = if b[4] & 128 == 0 {
            (10, 0)
        } else {
            (b[5], i16::from(b[7]))
        };
        let current = self.read(p, t, scope, index)?;
        // Resolver is invoked for a writable integer pointer before dispatch.
        if !self.write(p, t, scope, index, current)? {
            return Err("Set To Next destination unavailable (native error 0x13)".into());
        }
        let mode = b[4] & 127;
        let next = match mode {
            11 => {
                let houses = self
                    .world
                    .person
                    .as_ref()
                    .and_then(|p| p.nearby_houses.as_ref())
                    .ok_or("native nearby-house order unavailable")?;
                // 0x44f08d compares a sign-extended cursor to 32-bit lot IDs.
                // Zero starts at the beginning. An unknown nonzero cursor
                // exhausts the scan, retaining its value. Success is independent
                // of the narrowed result, including an actual zero entry.
                let offset = if current == 0 {
                    Some(0)
                } else {
                    houses
                        .iter()
                        .position(|&id| id == i32::from(current))
                        .map(|i| i + 1)
                };
                let Some(&next) = offset.and_then(|i| houses.get(i)) else {
                    return Ok((
                        false,
                        "Set To Next exhausted nearby houses; destination retained".into(),
                    ));
                };
                let next = next as i16;
                self.write(p, t, scope, index, next)?;
                return Ok((true, format!("Set To Next 11: {current} → {next}")));
            }
            12 => {
                let Some(next) = next_family_member(&self.world, current)? else {
                    return Ok((
                        false,
                        "Set To Next exhausted family cycle; destination retained".into(),
                    ));
                };
                next
            }
            10 => {
                let jobs = &p.careers.as_ref().ok_or("career catalog unavailable")?.ids;
                // 0x404cb3 matches the sign-extended VM cursor against 32-bit
                // IDs. Missing IDs restart at the first entry; exhaustion
                // retains the cursor. A present next ID succeeds even if its
                // narrowed word is zero (unlike the common object-ID modes).
                let offset = jobs
                    .iter()
                    .position(|&id| id == i32::from(current))
                    .map_or(0, |i| i + 1);
                let Some(&next) = jobs.get(offset) else {
                    return Ok((
                        false,
                        "Set To Next exhausted careers; destination retained".into(),
                    ));
                };
                let next = next as i16;
                self.write(p, t, scope, index, next)?;
                return Ok((true, format!("Set To Next 10: {current} → {next}")));
            }
            3 => crate::multitile::next(&self.world, current),
            4 => {
                let guid = u32::from_le_bytes(b[..4].try_into().unwrap());
                let resources: Vec<_> = p
                    .modules
                    .values()
                    .flat_map(|m| {
                        m.object_definition_words
                            .iter()
                            .filter_map(move |(&id, w)| {
                                (w.len() > 15
                                    && (w[14] as u16 as u32 | ((w[15] as u16 as u32) << 16))
                                        == guid)
                                    .then_some((&m.source, id))
                            })
                    })
                    .collect();
                if resources.is_empty() {
                    return Ok((
                        false,
                        "Set To Next resource absent; destination retained".into(),
                    ));
                }
                if resources.len() != 1 {
                    return Err("ambiguous GUID in loaded resource registry".into());
                }
                let (source, definition) = resources[0];
                let initial: Vec<_> = self
                    .world
                    .objects
                    .iter()
                    .filter_map(|(&id, o)| {
                        o.definition
                            .as_ref()
                            .filter(|d| &d.source == source && d.object == definition)
                            .map(|_| id)
                    })
                    .collect();
                let state = self
                    .world
                    .person
                    .as_mut()
                    .ok_or("resource registry unavailable")?;
                let order = state.resource_instances.entry(guid).or_insert(initial);
                let position = if current == 0 {
                    Some(0)
                } else {
                    order.iter().position(|&id| id == current).map(|i| i + 1)
                };
                position.and_then(|i| order.get(i)).copied().unwrap_or(0)
            }
            9 => {
                let f = t.stack.last().unwrap();
                let index = usize::from(b[6]);
                let reference = *f
                    .locals
                    .get(index)
                    .ok_or("adjacent search local out of bounds (native error 0x08)")?;
                if f.opaque_locals.contains(&index) {
                    return Err("adjacent search reads opaque local".into());
                }
                let object = self
                    .world
                    .objects
                    .get(&reference)
                    .ok_or("adjacent search reference missing (native error 0x0a)")?;
                let origin = *self
                    .world
                    .positions
                    .get(&reference)
                    .ok_or("adjacent reference position unavailable")?;
                let cursor = self.world.positions.get(&current).map(|p| {
                    [
                        (p.y >> 4).wrapping_sub(origin.y >> 4) as i8,
                        (p.x >> 4).wrapping_sub(origin.x >> 4) as i8,
                    ]
                });
                let mut found = None;
                for [dx, dy] in adjacent_offsets(cursor) {
                    let tile = (
                        (origin.y >> 4) + i32::from(dx),
                        (origin.x >> 4) + i32::from(dy),
                    );
                    let state = self
                        .world
                        .person
                        .as_ref()
                        .ok_or("adjacent tile registry unavailable")?;
                    let first = state
                        .tile_chains
                        .iter()
                        .filter_map(|c| c.first())
                        .find(|id| {
                            self.world.positions.get(id).is_some_and(|p| {
                                (p.y >> 4, p.x >> 4) == tile && p.level == origin.level
                            })
                        });
                    if let Some(&id) = first
                        && self.world.objects[&id].data.get(&29) == object.data.get(&29)
                    {
                        found = Some(id);
                        break;
                    }
                }
                let Some(next) = found else {
                    return Ok((
                        false,
                        "Set To Next exhausted adjacent tiles; destination retained".into(),
                    ));
                };
                next
            }
            8 => {
                let state = self
                    .world
                    .person
                    .as_ref()
                    .ok_or("tile enumeration unavailable")?;
                let next = state.tile_chains.iter().find_map(|chain| {
                    chain
                        .iter()
                        .position(|&id| id == current)
                        .and_then(|i| chain.get((i + 1) % chain.len()))
                        .copied()
                });
                let Some(next) = next else {
                    return Ok((
                        false,
                        "Set To Next exhausted tile chain; destination retained".into(),
                    ));
                };
                next
            }
            0 | 1 | 2 | 6 => {
                let state = self
                    .world
                    .person
                    .as_ref()
                    .ok_or("native object enumeration unavailable")?;
                let order = if mode != 1 {
                    &state.object_order
                } else {
                    &state.person_order
                };
                let offset = if current == 0 {
                    Some(0)
                } else {
                    order.iter().position(|&id| id == current).map(|i| i + 1)
                };
                let remaining = offset.and_then(|i| order.get(i..)).unwrap_or_default();
                // Mode 6 traverses the same linked registry as mode 0, comparing
                // data 59 against this frame's first argument (0x44f211).
                let category = if mode == 6 && !remaining.is_empty() {
                    Some(
                        *t.stack
                            .last()
                            .unwrap()
                            .args
                            .first()
                            .ok_or("category enumeration needs argument zero")?,
                    )
                } else {
                    None
                };
                let mut next = None;
                for id in remaining {
                    let matches = match mode {
                        6 => {
                            Some(
                                *self.world.objects[id]
                                    .data
                                    .get(&59)
                                    .ok_or("enumerated object category absent")?,
                            ) == category
                        }
                        2 => self.world.objects[id]
                            .definition
                            .as_ref()
                            .and_then(|d| {
                                p.modules
                                    .get(&d.source)?
                                    .object_definition_words
                                    .get(&d.object)
                            })
                            .is_some_and(|w| w[9] != 2),
                        _ => true,
                    };
                    if matches {
                        next = Some(*id);
                        break;
                    }
                }
                if next.is_none() && mode != 1 {
                    return Ok((
                        false,
                        "Set To Next exhausted object chain; destination retained".into(),
                    ));
                }
                next.unwrap_or(0)
            }
            5 | 7 => {
                let state = self
                    .world
                    .person
                    .as_ref()
                    .ok_or("neighborhood person directory unavailable")?;
                if current < 0 {
                    0
                } else {
                    // 0x425b15 returns the next occupied vector index, not the
                    // record's stored ID. Mode 7 then follows record +8 to the
                    // linked resource and compares its OBJD GUID (0x4572af).
                    // Linked resource identity is represented by Neighbor.guid.
                    let guid = u32::from_le_bytes(b[..4].try_into().unwrap());
                    let matches = |neighbor: &Neighbor| mode == 5 || neighbor.guid == guid;
                    if let Some(slots) = &state.neighbor_slots {
                        let mut next = 0;
                        for (i, id) in slots.iter().enumerate().skip(current as usize) {
                            let Some(id) = id else { continue };
                            let neighbor = state
                                .neighbors
                                .get(id)
                                .ok_or("neighbor slot record absent")?;
                            if matches(neighbor) {
                                next = (i + 1)
                                    .try_into()
                                    .map_err(|_| "neighbor index exceeds signed native profile")?;
                                break;
                            }
                        }
                        next
                    } else {
                        // Authored fixtures without a supplied native vector
                        // use their positive IDs as occupied slot indices.
                        state
                            .neighbors
                            .iter()
                            .find_map(|(&id, neighbor)| {
                                (id > current && matches(neighbor)).then_some(id)
                            })
                            .unwrap_or(0)
                    }
                }
            }
            mode => return Err(format!("unsupported Set To Next mode {mode}")),
        };
        self.write(p, t, scope, index, next)?;
        Ok((
            next != 0,
            format!("Set To Next {}: {current} → {next}", b[4] & 127),
        ))
    }
    pub(crate) fn look_towards(
        &mut self,
        t: &Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let mode = u16::from_le_bytes([b[0], b[1]]);
        if mode != 0 {
            return Err(format!("unsupported Look Towards mode {mode}"));
        }
        let frame = t.stack.last().unwrap();
        let person = self
            .world
            .objects
            .get_mut(&frame.caller)
            .and_then(|o| o.person.as_mut())
            .ok_or("look caller is not a person")?;
        for (i, value) in [
            (41, frame.stack_object),
            (42, i16::from(frame.stack_object != 0)),
            (43, 0),
            (44, 1),
            (45, 0),
        ] {
            person.insert(i, value);
        }
        Ok((true, format!("head seek target #{}", frame.stack_object)))
    }
    pub(crate) fn autonomy_disabled(&self, t: &Thread) -> Result<bool, String> {
        let world = self
            .world
            .person
            .as_ref()
            .ok_or("autonomy settings unavailable")?;
        let person = self.world.objects[&t.stack.last().unwrap().caller]
            .person
            .as_ref()
            .ok_or("autonomy caller is not a person")?;
        let field = |i| {
            person
                .get(&i)
                .copied()
                .ok_or_else(|| format!("autonomy person field {i} unavailable"))
        };
        let autonomy = field(32)?;
        let species = field(65)?;
        let disabled = !world.free_will
            && if autonomy == 0 {
                species & 6 == 0 || world.pet_autonomy_disabled
            } else {
                field(61)? == world.controlled_family && species <= 1
            };
        Ok(disabled)
    }

    pub(crate) fn inventory(
        &mut self,
        t: &mut Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let caller = t.stack.last().unwrap().caller;
        let actor = &self.world.objects[&caller];
        let flags = u16::from_le_bytes([b[2], b[3]]);
        let selected = if flags & 0x2000 != 0 {
            actor.temps[4]
        } else {
            caller
        };
        let person = self
            .world
            .objects
            .get(&selected)
            .filter(|o| o.person.is_some())
            .ok_or("inventory owner is not a person (native error 0x1c)")?;
        let owner_guid = person
            .definition
            .as_ref()
            .ok_or("inventory owner definition unavailable")?
            .guid;
        let mut guid = u32::from_le_bytes(b[4..].try_into().unwrap());
        if guid == 0 {
            guid = self
                .world
                .objects
                .get(&t.stack.last().unwrap().stack_object)
                .and_then(|o| o.definition.as_ref())
                .ok_or("inventory token definition unavailable")?
                .guid;
        }
        let world = self
            .world
            .person
            .as_mut()
            .ok_or("neighborhood inventory unavailable")?;
        let record = world
            .neighbors
            .values_mut()
            .find(|n| n.guid == owner_guid)
            .ok_or("inventory owner has no neighborhood record")?;
        if b[0] == 1 {
            //4552f8 selects a signed caller temp only when bit1 is set.
            let quantity = if flags & 2 != 0 {
                actor.temps[usize::from((flags >> 2) & 3)]
            } else {
                1
            };
            if quantity == 0 {
                world.inventory_dirty = true;
                return Ok((true, "inventory remove: zero quantity".into()));
            }
            let Some(index) = record
                .inventory
                .iter()
                .position(|e| e.kind == b[1] && e.guid == guid)
            else {
                return Ok((false, "inventory remove: token absent".into()));
            };
            //41f427 passes literal1 to41f471, irrespective of requested quantity.
            //The signed count clamps at0; quantity-1 additionally erases the
            //whole first matching entry. Neither path writes caller temps.
            let entry = &mut record.inventory[index];
            entry.count = if entry.count <= 1 { 0 } else { entry.count - 1 };
            let remaining = if entry.count == 0 || quantity == -1 {
                record.inventory.remove(index);
                0
            } else {
                entry.count
            };
            world.inventory_dirty = true;
            return Ok((
                true,
                format!("inventory remove: index {index}, requested {quantity}, remaining {remaining}"),
            ));
        }
        if b[0] != 3 {
            return Err(format!("unsupported inventory operation {}", b[0]));
        }
        let Some((index, entry)) = record
            .inventory
            .iter()
            .enumerate()
            .find(|(_, e)| e.kind == b[1] && e.guid == guid)
        else {
            return Ok((false, "inventory query: token absent".into()));
        };
        if entry.count == 0 {
            return Ok((false, "inventory query: zero count".into()));
        }
        let count = entry.count;
        let index =
            i16::try_from(index).map_err(|_| "inventory index exceeds signed native range")?;
        let temps = &mut self.world.objects.get_mut(&caller).unwrap().temps;
        temps[usize::from((flags >> 5) & 3)] = count;
        temps[usize::from((flags >> 11) & 3)] = index;
        world.inventory_dirty = true;
        Ok((
            true,
            format!("inventory query: count {count}, index {index}"),
        ))
    }
}

/// Native Set To Next mode 12 (0x44f0d9): cycle through the live person vector
/// after the current person, wrap once, and exclude the current person itself.
/// Family is person data 61, not household membership inferred from a save.
pub fn next_family_member(
    world: &crate::World,
    current: ObjectId,
) -> Result<Option<ObjectId>, String> {
    if current == 0 {
        return Ok(None);
    }
    let family = *world
        .objects
        .get(&current)
        .and_then(|o| o.person.as_ref())
        .ok_or("family cursor is not a person (native error 0x1c)")?
        .get(&61)
        .ok_or("family cursor field absent")?;
    let order = &world
        .person
        .as_ref()
        .ok_or("person registry absent")?
        .person_order;
    let offset = order
        .iter()
        .position(|&id| id == current)
        .ok_or("family cursor is missing from the native person vector")?;
    for &id in order[offset + 1..].iter().chain(&order[..offset]) {
        let other = world
            .objects
            .get(&id)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.get(&61))
            .ok_or("family vector member field absent")?;
        if *other == family {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Native cardinal-neighbor scan on packed tile coordinates: +X,+Y,-X,-Y.
/// A placed cursor resumes by its relative signs; exhaustion keeps the VM word.
pub fn adjacent_offsets(cursor: Option<[i8; 2]>) -> Vec<[i8; 2]> {
    let mut result = vec![];
    let mut delta = cursor.unwrap_or([1, 0]);
    if cursor.is_none() {
        result.push(delta);
    }
    loop {
        delta = if delta[0] >= 1 {
            [0, 1]
        } else if delta[0] < 0 {
            [0, -1]
        } else if delta[1] >= 1 {
            [-1, 0]
        } else {
            break;
        };
        result.push(delta);
    }
    result
}

/// 0x4271c5: both first directed columns must reach the supplied signed
/// process threshold. The caller owns its provenance; it is not a constant.
pub fn mutual_friend(a: &[i32], b: &[i32], threshold: i32) -> bool {
    a.first().is_some_and(|v| *v >= threshold) && b.first().is_some_and(|v| *v >= threshold)
}
#[cfg(test)]
mod family_tests {
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn original_mutual_friend_threshold() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/home-friends.json")).unwrap();
        for c in v["pairs"].as_array().unwrap() {
            assert_eq!(
                super::mutual_friend(
                    &[c["a"].as_i64().unwrap() as i32],
                    &[c["b"].as_i64().unwrap() as i32],
                    25, // explicit threshold of this retained original fixture
                ),
                c["result"].as_bool().unwrap(),
                "{c}"
            );
        }
    }
}
