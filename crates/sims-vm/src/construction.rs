//! Base-object storage contracts from Sims 1.91. Placement, calendar and resource
//! loading are supplied by the host; this is not the person/multitile factory.
use crate::{Definition, Object, ObjectId, Program, refresh::DrawState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ObjectDate {
    pub year: i16,
    pub month: i16,
    pub day: i16,
}
impl Object {
    /// Original ObjM allocation tail440032..440068 and ordinary factory
    /// tail440e4b..440e81. Activation follows post-construction (and factory
    /// Reset/price publication), rather than belonging to the base constructor.
    /// Supported saved-object readers preserve this native bit afterward.
    pub fn activate_native(&mut self) -> Result<(), String> {
        let flags = self
            .native_flags
            .as_mut()
            .ok_or("native activation requires constructed object flags")?;
        *flags |= 0x80;
        Ok(())
    }

    /// Native person storage constructor. Animator/resource post-construction,
    /// neighborhood attachment, reset and Init are separate lifecycle stages.
    pub fn construct_person(
        definition: Definition,
        program: &Program,
        slots: usize,
    ) -> Result<Self, String> {
        if definition_words(program, &definition)?[0x12 / 2] != 2 {
            return Err("person constructor needs a person OBJD".into());
        }
        let mut object = Self::construct(definition, program, slots)?;
        object.person = Some((0..256).map(|i| (i, 0)).collect());
        object.career_appearance = Some(Default::default());
        let (values, state) = crate::motives::UpdateState::constructed();
        object.motives = Some(values);
        object.motive_update = Some(state);
        Ok(object)
    }

    /// The native base constructor zeros 72 data words and eight temporaries,
    /// and allocates max(signed OBJD attribute count, 8) zeroed attributes.
    pub fn construct(
        definition: Definition,
        program: &Program,
        slots: usize,
    ) -> Result<Self, String> {
        let words = definition_words(program, &definition)?;
        let count = words[0x74 / 2].max(8) as usize;
        if count > crate::MAX_VARIABLES || slots > crate::MAX_VARIABLES {
            return Err("object storage exceeds supported limits".into());
        }
        Ok(Self {
            // 474762 initializes the base TreeSim error word before cXObject
            // construction. The supported 474fc3 frame reader does not read it.
            native_tree_error: Some(0),
            // 43189d clears the word, then sets bit 9. Supported ObjM readers
            // (versions 3e..49) retain it; only obsolete version 37 read it.
            native_flags: Some(0x200),
            // 431a64 writes this byte after the rectangle constructors. The
            // supported ObjM reader and base storage reset do not serialize it.
            native_keep_dynamic: Some(0),
            definition: Some(definition),
            relationship_order: Some(Vec::new()),
            attributes: vec![0; count],
            data: (0..72).map(|i| (i, 0)).collect(),
            slots: vec![None; slots],
            ..Self::default()
        })
    }
    /// Reset storage before installing Main and invoking Init. The host has
    /// already retained physical placement/containment and provides the calendar.
    pub fn reset_storage(
        &mut self,
        program: &Program,
        id: ObjectId,
        now: ObjectDate,
    ) -> Result<bool, String> {
        let def = self
            .definition
            .as_ref()
            .ok_or("reset requires definition")?;
        let words = definition_words(program, def)?;
        if self.person.is_some() || words[0x12 / 2] == 2 {
            return Err("person subclass reset is unsupported".into());
        }
        self.reset_base_storage(program, id, now)
    }

    /// Base storage portion of 0x431be1, also called from the person constructor
    /// reset wrapper. Subclass ordering belongs to that wrapper, not this helper.
    pub(crate) fn reset_base_storage(
        &mut self,
        program: &Program,
        id: ObjectId,
        now: ObjectDate,
    ) -> Result<bool, String> {
        let words = definition_words(
            program,
            self.definition
                .as_ref()
                .ok_or("reset requires definition")?,
        )?;
        if (0..72).any(|i| !self.data.contains_key(&i)) {
            return Err("reset requires constructed object data".into());
        }
        let old = self.data.clone();
        self.data = (0..72).map(|i| (i, 0)).collect();
        for i in [21, 24, 29, 41, 45, 46, 47, 67] {
            self.data.insert(i, old[&i]);
        }
        self.data.insert(1, old[&1].rem_euclid(8));
        self.data.insert(11, id);
        self.data.insert(58, words[0x62 / 2]);
        self.data.insert(2, self.container.map_or(0, |c| c.0));
        self.data
            .insert(3, self.container.map_or(-1, |c| c.1 as i16));
        // Native physical slot number includes a sentinel; script slots do not.
        self.data
            .insert(14, self.container.map_or(-1, |c| c.1 as i16 + 1));
        self.data.insert(26, self.container.map_or(0, |c| c.0));
        let mut years = now.year.wrapping_sub(old[&45]);
        let mut months = now.month.wrapping_sub(old[&46]);
        if now.day.wrapping_sub(old[&47]) < 0 {
            months = months.wrapping_sub(1);
        }
        if months < 0 {
            months = months.wrapping_add(12);
            years = years.wrapping_sub(1);
        }
        self.data.insert(18, years);
        self.data
            .insert(48, months.wrapping_add(years.wrapping_mul(12)));
        self.attributes.fill(0);
        self.temps.fill(0);
        // 0x431dc9 replaces the signed countdown, preserving the upper flags.
        self.idle.get_or_insert_with(Default::default).countdown = -1;
        Ok(words[0x14 / 2] == 0)
    }
}
impl DrawState {
    pub fn constructed(level: i32) -> Self {
        Self {
            level,
            dynamic: true,
            expiry: i32::MAX,
            refreshed_at: 0,
        }
    }
}

impl crate::Vm {
    /// Registered base/single-part factory prefix shared by Create Object and
    /// lot-load reconciliation. The host drives synchronous Init and the first
    /// Main visit, then calls `finish_object_factory`. No saved object runs here.
    pub fn begin_object_factory(
        &mut self,
        p: &Program,
        id: ObjectId,
        definition: Definition,
        group: bool,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<(), String> {
        self.register_object_factory(p, id, definition, group, None, now, time)?;
        if group {
            self.prepare_multitile_reset(p, id, now)?;
            self.begin_multitile_init(p, id)?;
        } else {
            self.reset_object_lifecycle(p, id, now)?;
        }
        Ok(())
    }

    /// Storage, registration, subclass postconstruction and date stamp, before
    /// any Reset/Init/Main. A group factory registers all siblings first.
    pub(crate) fn register_object_factory(
        &mut self,
        p: &Program,
        id: ObjectId,
        definition: Definition,
        group: bool,
        head: Option<ObjectId>,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<(), String> {
        self.register_object_storage(p, id, definition, group, head)?;
        self.stamp_object_factory(id, now, time)
    }

    /// Registered single-part ordinary object, at431aaf's final graphic
    /// service. No date stamp, Reset, Main or Init has executed yet.
    pub fn begin_object_factory_graphics(
        &mut self,
        p: &Program,
        id: ObjectId,
        definition: Definition,
    ) -> Result<(), String> {
        let words = definition_words(p, &definition)?;
        if words[10] != 0 || words.get(102).is_none_or(|w| w & 1 == 0) {
            return Err(
                "staged graphic factory requires single-part ordinary 3D definition".into(),
            );
        }
        self.register_object_storage(p, id, definition, false, None)
    }

    /// Complete4376f6 before continuing440b2d's date stamp and Reset(1).
    pub fn finish_object_factory_graphics(
        &mut self,
        p: &Program,
        id: ObjectId,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<(), String> {
        if self.object_threads.contains_key(&id) || self.lifecycles.contains_key(&id) {
            return Err("ordinary graphics completed after Init/Main started".into());
        }
        self.stamp_object_factory(id, now, time)?;
        self.reset_object_lifecycle(p, id, now)?;
        Ok(())
    }

    ///440b2d's multipart registration, stopped inside422872 before the
    ///base postconstructor's graphic service returns. Its raw origin is
    ///published only AFTER431aaf, before the date stamp and next registration.
    pub(crate) fn begin_group_factory_part(
        &mut self,
        p: &Program,
        id: ObjectId,
        definition: Definition,
        head: Option<ObjectId>,
    ) -> Result<(), String> {
        self.register_object_storage(p, id, definition, true, head)?;
        self.world
            .objects
            .get_mut(&id)
            .unwrap()
            .multitile
            .as_mut()
            .unwrap()
            .origin = [0; 3];
        Ok(())
    }
    pub(crate) fn finish_group_factory_part(
        &mut self,
        p: &Program,
        id: ObjectId,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<(), String> {
        if self.object_threads.contains_key(&id) || self.lifecycles.contains_key(&id) {
            return Err("group postconstruction completed after Init/Main started".into());
        }
        let object = self
            .world
            .objects
            .get_mut(&id)
            .ok_or("group postconstructor object absent")?;
        let origin = crate::multitile::origin(definition_words(
            p,
            object
                .definition
                .as_ref()
                .ok_or("group definition absent")?,
        )?)?;
        object
            .multitile
            .as_mut()
            .ok_or("group postconstructor part absent")?
            .origin = origin;
        self.stamp_object_factory(id, now, time)
    }

    fn stamp_object_factory(
        &mut self,
        id: ObjectId,
        now: ObjectDate,
        time: [i16; 2],
    ) -> Result<(), String> {
        let object = self
            .world
            .objects
            .get_mut(&id)
            .ok_or("factory date owner absent")?;
        object.data.extend([
            (45, now.year),
            (46, now.month),
            (47, now.day),
            (24, time[0]),
            (21, time[1]),
        ]);
        Ok(())
    }

    fn register_object_storage(
        &mut self,
        p: &Program,
        id: ObjectId,
        definition: Definition,
        group: bool,
        head: Option<ObjectId>,
    ) -> Result<(), String> {
        self.register_object_storage_kind(p, id, definition, group, head, None)
    }

    ///421afb +4434cd, stopping BEFORE virtual+3c. The single-object4215e6
    ///branch picks up furniture between registration and postconstruction;
    ///the multipart branch postconstructs each cursor before pickup. Captured
    ///logical containment is supplied by43265e's host. No Init/Main/date stamp.
    pub fn register_cursor_storage(
        &mut self,
        p: &Program,
        id: ObjectId,
        definition: Definition,
        underlying: Option<u32>,
        head: Option<ObjectId>,
        captured_container: Option<(i16, i16)>,
    ) -> Result<(), String> {
        if self.world.scheduler_storage.is_none() {
            return Err("cursor registration requires lifetime registry storage".into());
        }
        let words = definition_words(p, &definition)?;
        if definition.guid != (words[14] as u16 as u32 | ((words[15] as u16 as u32) << 16))
            || definition.slots != words[20] as u16
        {
            return Err("cursor resource identity differs from definition".into());
        }
        if let Some(head) = head {
            if self.world.objects.get(&head).is_none_or(|o| {
                o.cursor.is_none()
                    || o.multitile.as_ref().is_none_or(|m| m.head.is_some())
                    || o.definition.as_ref() != Some(&definition)
            }) {
                return Err("cursor constructor requires its registered cursor head".into());
            }
        }
        //The actual tile cursor has no 3D animator. Keep an unknown graphics
        //subclass pending at its caller rather than inventing a completed+1c.
        if definition_words(p, &definition)?
            .get(102)
            .is_some_and(|w| w & 1 != 0)
        {
            return Err("cursor postconstruction requires an ordinary graphic owner".into());
        }
        self.register_object_storage_kind(
            p,
            id,
            definition,
            true,
            head,
            Some(crate::cursor::State::constructed(
                underlying,
                captured_container,
            )),
        )
    }

    ///Cursor virtual+3c:4630d4 jumps to422872, which calls431aaf and then
    ///sets the raw resource origin. This must run at its caller's original
    ///boundary, after pickup for ordinary furniture and before it for groups.
    pub fn finish_cursor_postconstruction(
        &mut self,
        p: &Program,
        id: ObjectId,
    ) -> Result<(), String> {
        let object = self
            .world
            .objects
            .get(&id)
            .ok_or("cursor postconstructor object absent")?;
        if object.cursor.as_ref().is_none_or(|c| c.postconstructed) || object.multitile.is_none() {
            return Err("cursor postconstructor duplicate or missing registration".into());
        }
        let definition = object
            .definition
            .as_ref()
            .ok_or("cursor definition absent")?;
        let words = definition_words(p, definition)?;
        if words.get(102).is_some_and(|w| w & 1 != 0) {
            return Err("cursor postconstruction requires an ordinary graphic owner".into());
        }
        let slots = if definition.slots == 0 {
            0
        } else {
            p.modules[&definition.source]
                .slots
                .get(&definition.slots)
                .map_or(0, |s| s.entries.iter().filter(|s| s.kind == 0).count())
        };
        if slots > crate::MAX_VARIABLES || !object.slots.is_empty() {
            return Err("cursor postconstructor slot storage changed before loading".into());
        }
        let origin = crate::cursor::origin(words)?;
        let dynamic = usize::try_from(words[65]).map_err(|_| "negative dynamic sprite count")?;
        let object = self.world.objects.get_mut(&id).unwrap();
        object.slots = vec![None; slots];
        //431aaf always appends a physical sentinel, including absent SLOT.
        object
            .data
            .extend([(3, -1), (14, -1), (29, -5), (67, slots as i16)]);
        object.multitile.as_mut().unwrap().origin = origin;
        object.dynamic_sprite_flags = vec![0; dynamic];
        object.cursor.as_mut().unwrap().postconstructed = true;
        self.world.positions.insert(
            id,
            crate::autonomy::Placement {
                x: -16,
                y: -16,
                level: 1,
            },
        );
        self.world
            .refresh
            .as_mut()
            .ok_or("cursor drawing registry absent")?
            .draw
            .get_mut(&id)
            .ok_or("cursor drawing storage absent")?
            .level = 1;
        Ok(())
    }

    fn register_object_storage_kind(
        &mut self,
        p: &Program,
        id: ObjectId,
        definition: Definition,
        group: bool,
        head: Option<ObjectId>,
        cursor: Option<crate::cursor::State>,
    ) -> Result<(), String> {
        if id <= 0 || self.world.objects.contains_key(&id) {
            return Err("factory needs an unused positive object ID".into());
        }
        if self.world.person.is_none() || self.world.refresh.is_none() {
            return Err("factory needs object registry and drawing storage".into());
        }
        let module = p
            .modules
            .get(&definition.source)
            .ok_or("factory module missing")?;
        let words = definition_words(p, &definition)?;
        if words.len() <= 0x82 / 2 || cursor.is_none() && [2, 8, 9].contains(&words[0x12 / 2]) {
            return Err("factory definition needs unsupported subclass".into());
        }
        if cursor.is_none() && (words[10] != 0) != group {
            return Err("factory group classification differs from OBJD".into());
        }
        let tail = head
            .map(|head| crate::multitile::chain(&self.world, head))
            .transpose()?
            .and_then(|chain| chain.last().copied());
        if head.is_some() && !group {
            return Err("ordinary factory cannot join a group".into());
        }
        let is_cursor = cursor.is_some();
        let slots = if is_cursor || definition.slots == 0 {
            0
        } else {
            // 431aaf calls 46e2d1 on the local IFF. An absent chunk leaves
            // the constructed slot vectors untouched and returns success;
            // Tutorial.iff, for example, references an absent SLOT 128.
            // Present but malformed chunks are still rejected when loading.
            module.slots.get(&definition.slots).map_or(0, |slots| {
                slots.entries.iter().filter(|s| s.kind == 0).count()
            })
        };
        let mut object = Object::construct(definition.clone(), p, slots)?;
        if group {
            let offset = if let Some(cursor) = &cursor {
                crate::cursor::constructed_offset(&mut self.world, p, cursor.underlying, head)?
            } else {
                [0; 3]
            };
            object.multitile = Some(crate::multitile::Part {
                head,
                next: None,
                origin: if cursor.is_some() {
                    [0; 3]
                } else {
                    crate::multitile::origin(words)?
                },
                offset,
            });
        }
        if !is_cursor {
            // 431aaf publishes (physical slot count - sentinel) at +d0
            // before Init; reset_base_storage explicitly retains this word.
            object
                .data
                .extend([(11, id), (14, -1), (29, -5), (67, slots as i16)]);
            object.dynamic_sprite_flags = vec![
                0;
                usize::try_from(words[0x82 / 2])
                    .map_err(|_| "negative dynamic sprite count")?
            ];
        }
        object.cursor = cursor;
        self.world.register_scheduler_object(id, &mut object)?;
        let registry = self.world.person.as_mut().unwrap();
        if let Some(order) = registry.resource_instances.get_mut(&definition.guid) {
            order.push(id);
        }
        registry.object_order.insert(0, id);
        self.world.objects.insert(id, object);
        self.world.invalidate_registry_order();
        if let Some(tail) = tail {
            self.world
                .objects
                .get_mut(&tail)
                .unwrap()
                .multitile
                .as_mut()
                .unwrap()
                .next = Some(id);
        }
        self.world.positions.insert(
            id,
            crate::autonomy::Placement {
                x: if is_cursor { 0 } else { -16 },
                y: if is_cursor { 0 } else { -16 },
                level: if is_cursor { 0 } else { 1 },
            },
        );
        self.world
            .refresh
            .as_mut()
            .unwrap()
            .draw
            .insert(id, DrawState::constructed(if is_cursor { 0 } else { 1 }));
        Ok(())
    }

    pub fn finish_object_factory(&mut self, p: &Program, id: ObjectId) -> Result<(), String> {
        let thread = *self
            .object_threads
            .get(&id)
            .ok_or("factory thread missing")?;
        if !matches!(
            self.threads[&thread].status,
            crate::Status::Sleeping { .. } | crate::Status::Returned(_)
        ) || self.threads[&thread].callback.is_some()
        {
            return Err("factory Init/Main has not completed its immediate update".into());
        }
        self.finish_object_update(id, true)?;
        self.refresh_registry_flags(p, id)?;
        let def = self.world.objects[&id]
            .definition
            .as_ref()
            .ok_or("factory definition missing")?;
        let price = definition_words(p, def)?[0x24 / 2];
        self.world
            .objects
            .get_mut(&id)
            .unwrap()
            .data
            // 0x440e44 stores at object +0x9c: data word 41. Word 42
            // (+0x9e) contains Init's placement flags and must survive.
            .insert(41, price);
        self.world.objects.get_mut(&id).unwrap().activate_native()?;
        self.world
            .objects
            .get_mut(&id)
            .unwrap()
            .idle
            .as_mut()
            .unwrap()
            .set_flags(1, true);
        self.finish_factory_object_visit(id)?;
        Ok(())
    }
}
pub fn definition_words<'a>(program: &'a Program, def: &Definition) -> Result<&'a [i16], String> {
    let words = program
        .modules
        .get(&def.source)
        .and_then(|m| m.object_definition_words.get(&def.object))
        .ok_or("original definition words unavailable")?;
    if words.len() < 0x76 / 2 {
        return Err("truncated object definition".into());
    }
    Ok(words)
}
