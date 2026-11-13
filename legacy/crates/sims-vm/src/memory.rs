use crate::{Program, Thread, Vm, word};
use serde::Serialize;

#[cfg(test)]
#[path = "memory_cache_tests.rs"]
mod cache_tests;
#[cfg(test)]
#[path = "memory_indicator_tests.rs"]
mod indicator_tests;

#[derive(Debug, Clone, Copy)]
enum Cell {
    Literal(i16),
    Attribute(i16, usize),
    ResourceGlobal(usize, usize),
    Data(i16, i16),
    Person(i16, i16),
    Temp(i16, usize),
    Extra(i16, usize),
    Global(i16),
    Arg(usize),
    Local(usize),
    StackObject,
    TreeAdvertisement(usize, usize),
    Motive(i16, usize),
}
#[derive(Debug, Serialize)]
pub struct MemoryProbe {
    pub value: i16,
    pub writable: bool,
    pub float_pointer: bool,
}
fn error(code: u32, message: &str) -> String {
    format!("{message} (native error 0x{code:02x})")
}
fn bounded(index: i16, count: usize, code: u32) -> Result<usize, String> {
    usize::try_from(index)
        .ok()
        .filter(|i| *i < count)
        .ok_or_else(|| error(code, "memory index out of bounds"))
}
impl Vm {
    /// 0x451f27 / 0x452003. Destination resolution precedes object lookups.
    pub(crate) fn spatial_query(
        &mut self,
        program: &Program,
        t: &mut Thread,
        opcode: u16,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let index = i16::from_le_bytes([b[0], b[1]]);
        let destination = if opcode == 11 {
            if !(0..8).contains(&index) {
                return Err("Get Distance destination invalid (native error 0x02)".into());
            }
            None
        } else {
            let (cell, writable) = self.resolve_runtime_memory(program, t, b[2], index, true)?;
            if !writable {
                return Err("Get Direction destination has no word pointer".into());
            }
            Some(cell)
        };
        let flags = if opcode == 11 { b[2] } else { b[4] };
        let scope = if flags & 1 == 0 {
            3
        } else if opcode == 11 {
            b[3]
        } else {
            b[5]
        };
        let source_index = if flags & 1 == 0 {
            11
        } else if opcode == 11 {
            i16::from_le_bytes([b[4], b[5]])
        } else {
            i16::from_le_bytes([b[6], b[7]])
        };
        let source = self.read(program, t, scope, source_index)?;
        let target = t.stack.last().unwrap().stack_object;
        if !self.world.objects.contains_key(&source) {
            return Err("spatial query source missing (native error 0x17)".into());
        }
        let object = self.world.objects.get(&target).ok_or(if opcode == 11 {
            "distance target missing (native error 0x17)"
        } else {
            "direction target missing (native error 0x15)"
        })?;
        let a = *self
            .world
            .positions
            .get(&source)
            .ok_or("spatial query source position unavailable")?;
        let z = *self
            .world
            .positions
            .get(&target)
            .ok_or("spatial query target position unavailable")?;
        let value = if opcode == 11 {
            let d = object
                .definition
                .as_ref()
                .ok_or("distance target definition missing")?;
            let kind = crate::construction::definition_words(program, d)?[9];
            crate::autonomy::distance(a, z, kind) as i32 as i16
        } else {
            crate::routing::direction([a.x, a.y], [z.x, z.y])
        };
        if let Some(cell) = destination {
            self.write_cell(t, cell, value)?;
        } else {
            let caller = t.stack.last().unwrap().caller;
            self.world.objects.get_mut(&caller).unwrap().temps[index as usize] = value;
        }
        Ok((
            true,
            format!("spatial query #{source} to #{target}: {value}"),
        ))
    }
    /// Diagnostic inspection of the same resolver used by execution. No mutation.
    pub fn inspect_memory(
        &self,
        program: &Program,
        thread: u64,
        scope: u8,
        index: i16,
        write: bool,
    ) -> Result<MemoryProbe, String> {
        let t = self.threads.get(&thread).ok_or("unknown thread")?;
        let (cell, writable) = self.resolve_memory(program, t, scope, index, write)?;
        Ok(MemoryProbe {
            value: self.cell_value(t, cell)?,
            writable: writable && !matches!(cell, Cell::Motive(..)),
            float_pointer: writable && matches!(cell, Cell::Motive(..)),
        })
    }
    fn resolve_memory(
        &self,
        program: &Program,
        t: &Thread,
        scope: u8,
        index: i16,
        write: bool,
    ) -> Result<(Cell, bool), String> {
        let f = t.stack.last().ok_or("no memory context")?;
        let object = |id, code| {
            self.world
                .objects
                .get(&id)
                .ok_or_else(|| error(code, "missing object"))
        };
        let caller = object(f.caller, 10)?;
        let temp = |i| bounded(i, 8, 2).map(|i| caller.temps[i]);
        let attribute = |id, index| -> Result<Cell, String> {
            Ok(Cell::Attribute(
                id,
                bounded(index, object(id, 10)?.attributes.len(), 6)?,
            ))
        };
        let person = |id, index| -> Result<Cell, String> {
            let p = object(id, 0x15)?
                .person
                .as_ref()
                .ok_or_else(|| error(0x1c, "object is not a person"))?;
            bounded(index, 256, 0x21)?;
            if !p.contains_key(&index) {
                return Err(format!("person field {index} is unavailable"));
            }
            Ok(Cell::Person(id, index))
        };
        let slot = |id, index| -> Result<Cell, String> {
            let o = object(id, 0x15)?;
            let i = bounded(index, o.slots.len(), 0x1b)?;
            Ok(Cell::Literal(o.slots[i].unwrap_or(0)))
        };
        let cell = match scope {
            0 => attribute(f.caller, index)?,
            1 => attribute(f.stack_object, index)?,
            3 | 4 => {
                bounded(index, 72, 7)?;
                let id = if scope == 3 { f.caller } else { f.stack_object };
                object(id, 10)?;
                Cell::Data(id, index)
            }
            6 => {
                bounded(index, 64, 0x3f)?;
                Cell::Global(index)
            }
            7 => Cell::Literal(index),
            8 => Cell::Temp(f.caller, bounded(index, 8, 2)?),
            9 => Cell::Arg(bounded(index, f.args.len(), 8)?),
            10 => {
                if index != 0 {
                    return Err(error(0xf, "stack object selector must be zero"));
                }
                Cell::StackObject
            }
            11 => Cell::Temp(f.caller, bounded(temp(index)?, 8, 2)?),
            12 | 28 | 29 => Cell::TreeAdvertisement(
                bounded(index, self.world.tree_advertisements.len(), 0x1a)?,
                match scope {
                    12 => 2,
                    28 => 0,
                    _ => 1,
                },
            ),
            13 => {
                object(f.stack_object, 10)?;
                Cell::Temp(f.stack_object, bounded(index, 8, 2)?)
            }
            14 | 15 | 17 => {
                let i = if scope == 17 { temp(index)? } else { index };
                let i = bounded(i, 16, 0x1a)?;
                let id = if scope == 14 {
                    f.caller
                } else {
                    f.stack_object
                };
                let o = object(id, 0x15)?;
                if o.person.is_none() {
                    return Err(error(0x1c, "object is not a person"));
                }
                if o.motives.is_none() {
                    return Err("native motive storage unavailable".into());
                }
                Cell::Motive(id, i)
            }
            16 => slot(f.stack_object, index)?,
            18 => person(f.caller, index)?,
            19 => {
                bounded(index, 256, 0x21)?;
                person(f.stack_object, index)?
            }
            20 => slot(f.caller, index)?,
            21 => {
                bounded(index, 108, 0x23)?;
                let def = object(f.stack_object, 10)?
                    .definition
                    .as_ref()
                    .ok_or("object definition unavailable")?;
                let words = program
                    .modules
                    .get(&def.source)
                    .and_then(|m| m.object_definition_words.get(&def.object))
                    .ok_or("original definition words unavailable")?;
                Cell::Literal(
                    *words
                        .get(index as usize)
                        .ok_or("definition field exceeds source payload")?,
                )
            }
            22 => {
                let i = bounded(index, f.args.len(), 8)?;
                attribute(f.stack_object, f.args[i])?
            }
            23 if index == 3 => {
                let room = temp(0)?;
                Cell::Literal(if room == -5 {
                    0
                } else {
                    (*self
                        .world
                        .room_half_tiles
                        .get(&room)
                        .ok_or_else(|| error(0x28, "missing room"))?
                        / 2) as i16
                })
            }
            23 if index == 1 => {
                let room = temp(0)?;
                Cell::Literal(if room == -5 {
                    1
                } else {
                    i16::from(
                        *self
                            .world
                            .room_outdoors
                            .get(&room)
                            .ok_or_else(|| error(0x28, "missing room"))?,
                    )
                })
            }
            23 if index == 2 => {
                let room = temp(0)?;
                Cell::Literal(if room == -5 {
                    0
                } else {
                    // 0x466fcc must resolve the registered room before
                    // 0x468756 reads the level encoded in that record's ID.
                    if !self.world.room_outdoors.contains_key(&room) {
                        return Err(error(0x28, "missing room"));
                    }
                    (((room as u16 & 0xc00) >> 10) + 1) as i16
                })
            }
            24 if index == 5 => Cell::Literal(
                self.world
                    .person
                    .as_ref()
                    .ok_or("neighbor directory unavailable")?
                    .neighbor_friend_count(
                        program.neighbor_resource_flags.as_ref(),
                        f.stack_object,
                        self.world.autonomy_constants.as_ref(),
                    )?
                    .value,
            ),
            24 if index == 6 => {
                let world = self
                    .world
                    .person
                    .as_ref()
                    .ok_or("neighbor home directory unavailable")?;
                Cell::Literal(world.neighbor_home(f.stack_object)?)
            }
            24 if index == 9 => Cell::Literal(
                self.world
                    .person
                    .as_ref()
                    .ok_or("neighbor directory unavailable")?
                    .family_friends(f.stack_object, self.world.autonomy_constants.as_ref())?,
            ),
            24 => {
                // 0x425b5d: a missing neighborhood slot returns zero before
                // selecting a field. Results have no writable word pointer.
                let state = self
                    .world
                    .person
                    .as_ref()
                    .ok_or("neighbor directory unavailable")?;
                let neighbor = if let Some(slots) = &state.neighbor_slots {
                    usize::try_from(f.stack_object)
                        .ok()
                        .and_then(|i| i.checked_sub(1))
                        .and_then(|i| slots.get(i))
                        .and_then(|id| *id)
                        .and_then(|id| state.neighbors.get(&id))
                } else {
                    state.neighbors.get(&f.stack_object)
                };
                Cell::Literal(if let Some(neighbor) = neighbor {
                    match index {
                        // Native matches linked resource identity in person-vector
                        // order, not the person's saved neighbor/family fields.
                        0 => state
                            .person_order
                            .iter()
                            .copied()
                            .find(|id| {
                                self.world
                                    .objects
                                    .get(id)
                                    .and_then(|o| o.definition.as_ref())
                                    .is_some_and(|d| d.guid == neighbor.guid)
                            })
                            .unwrap_or(0),
                        // The manager's selected family owns a GUID vector;
                        // saved neighbor field 61 is not this membership test.
                        1 => i16::from(state.families.get(&state.controlled_family).is_some_and(
                            |members| {
                                members.iter().any(|id| {
                                    state
                                        .neighbors
                                        .get(id)
                                        .is_some_and(|n| n.guid == neighbor.guid)
                                })
                            },
                        )),
                        2 => *neighbor
                            .person
                            .get(&58)
                            .ok_or("neighbor age field unavailable")?,
                        7 | 8 => {
                            return Err(format!("unsupported neighbor metadata field {index}"));
                        }
                        _ => 0,
                    }
                } else {
                    0
                })
            }
            25 => Cell::Local(bounded(index, f.locals.len(), 8)?),
            26 => Cell::Literal(program.tuning(&f.code_owner, index as u16)?),
            27 => {
                bounded(index, 8, 2)?;
                let o = object(f.stack_object, 10)?;
                Cell::Extra(
                    f.stack_object,
                    bounded(temp(index)?, o.dynamic_sprite_flags.len(), 0x31)?,
                )
            }
            30 => {
                if caller.person.is_none() {
                    return Err(error(0x1c, "object is not a person"));
                }
                person(f.caller, temp(index)?)?
            }
            31 => {
                if object(f.stack_object, 0x15)?.person.is_none() {
                    return Err(error(0x1c, "object is not a person"));
                }
                person(f.stack_object, temp(index)?)?
            }
            32 => {
                let neighbor = self
                    .world
                    .person
                    .as_ref()
                    .ok_or("neighborhood person directory unavailable")?
                    .neighbors
                    .get(&f.stack_object)
                    .ok_or_else(|| error(0x37, "missing neighbor"))?;
                bounded(index, 256, 0x21)?;
                Cell::Literal(
                    *neighbor
                        .person
                        .get(&index)
                        .ok_or("neighbor person field unavailable")?,
                )
            }
            33 => {
                let catalog = program
                    .careers
                    .as_ref()
                    .ok_or("career catalog unavailable")?;
                let career = caller.temps[0];
                let row = catalog
                    .ids
                    .iter()
                    .position(|&id| id == i32::from(career))
                    .ok_or_else(|| error(0x39, "career record missing"))?;
                let record = catalog
                    .records
                    .as_ref()
                    .and_then(|records| records.get(row))
                    .ok_or("career field data unavailable")?;
                let value = if index == 0 {
                    record.levels.len() as i16
                } else {
                    let level = if career == -1 { 0 } else { caller.temps[1] };
                    let level = usize::try_from(level)
                        .ok()
                        .and_then(|i| record.levels.get(i))
                        .ok_or_else(|| error(0x3a, "career level or field invalid"))?;
                    match index {
                        1 => level.values[17] as i16,
                        2..=11 => level.values[index as usize - 2] as i16,
                        12 => level.values[18] as i16,
                        13 => level.values[19] as i16,
                        14..=20 => level.values[index as usize - 4] as i16,
                        21 => level.values[20] as i16,
                        22 => i16::from(!level.strings[1].is_empty()),
                        _ => return Err(error(0x3a, "career level or field invalid")),
                    }
                };
                Cell::Literal(value)
            }
            35 => {
                let def = object(f.stack_object, 10)?
                    .definition
                    .as_ref()
                    .ok_or("callback lookup definition unavailable")?;
                let entries = program
                    .modules
                    .get(&def.source)
                    .and_then(|m| m.object_functions.get(&def.object))
                    .ok_or("callback table unavailable")?;
                Cell::Literal(i16::from(
                    usize::try_from(index)
                        .ok()
                        .and_then(|i| entries.get(i))
                        .is_some_and(|e| e.action != 0),
                ))
            }
            34 => {
                if !(1..=15).contains(&index) {
                    return Err(error(0x3c, "neighborhood index out of bounds"));
                }
                Cell::Literal(
                    *self
                        .world
                        .neighborhood
                        .get(&index)
                        .ok_or("neighborhood field unavailable")?,
                )
            }
            36 | 37 => {
                let id = if scope == 36 {
                    f.caller
                } else {
                    f.stack_object
                };
                object(id, 10)?;
                if index < 0 {
                    return Err(error(0x40, "resource-global index out of bounds"));
                }
                if program.resource_storage.is_none() {
                    return Err("resource-global catalog has not been loaded".into());
                }
                let key = crate::resource_storage::key(&self.world, program, id)?;
                let record = self
                    .world
                    .resource_storage
                    .records
                    .iter()
                    .position(|r| r.guid == key);
                let count =
                    record.map_or(0, |i| self.world.resource_storage.records[i].values.len());
                let index = bounded(index, count, 0x40)?;
                Cell::ResourceGlobal(record.unwrap(), index)
            }
            2 | 5 | 39..=255 => return Err(error(9, "invalid native memory scope")),
            _ => {
                return Err(format!(
                    "unsupported native read scope {scope:#04x}, index {index}"
                ));
            }
        };
        let writable = match cell {
            Cell::Literal(_) => false,
            Cell::Global(i) => i == 34 || (write && i == 36),
            _ => write,
        };
        Ok((cell, writable))
    }
    fn cell_value(&self, t: &Thread, cell: Cell) -> Result<i16, String> {
        let f = t.stack.last().ok_or("no memory frame")?;
        Ok(match cell {
            Cell::Literal(v) => v,
            Cell::Attribute(id, i) => self.world.objects[&id].attributes[i],
            Cell::ResourceGlobal(i, j) => self.world.resource_storage.records[i].values[j],
            Cell::Temp(id, i) => self.world.objects[&id].temps[i],
            Cell::Extra(id, i) => self.world.objects[&id].dynamic_sprite_flags[i],
            Cell::Data(id, i) => {
                let o = &self.world.objects[&id];
                match i {
                    11 => o.data.get(&11).copied().unwrap_or(id),
                    2 => o
                        .data
                        .get(&2)
                        .copied()
                        .unwrap_or_else(|| o.container.map_or(0, |c| c.0)),
                    3 => o
                        .data
                        .get(&3)
                        .copied()
                        .unwrap_or_else(|| o.container.map_or(0, |c| c.1 as i16)),
                    _ => *o
                        .data
                        .get(&i)
                        .ok_or_else(|| format!("object {id} field {i} has no engine value"))?,
                }
            }
            Cell::Person(id, i) => self.world.objects[&id].person.as_ref().unwrap()[&i],
            Cell::Global(i) => *self
                .world
                .globals
                .get(&i)
                .ok_or_else(|| format!("global {i} has no engine value"))?,
            Cell::Arg(i) => f.args[i],
            Cell::Local(i) => {
                if f.opaque_locals.contains(&i) {
                    return Err("read of reused opaque native pointer word is unsupported".into());
                }
                f.locals[i]
            }
            Cell::StackObject => f.stack_object,
            Cell::TreeAdvertisement(i, j) => self.world.tree_advertisements[i][j],
            Cell::Motive(id, i) => {
                let value = f32::from_bits(self.world.objects[&id].motives.unwrap()[i]);
                if !value.is_finite() || !(i32::MIN as f32..i32::MAX as f32).contains(&value) {
                    return Err("motive needs unvalidated x87 conversion policy".into());
                }
                (value.round() as i32) as i16
            }
        })
    }
    pub(crate) fn read(
        &mut self,
        program: &Program,
        t: &mut Thread,
        scope: u8,
        index: i16,
    ) -> Result<i16, String> {
        let (cell, _) = self.resolve_runtime_memory(program, t, scope, index, false)?;
        self.cell_value(t, cell)
    }
    fn resolve_runtime_memory(
        &mut self,
        program: &Program,
        t: &mut Thread,
        scope: u8,
        index: i16,
        write: bool,
    ) -> Result<(Cell, bool), String> {
        if scope == 24 && matches!(index, 5 | 9) {
            let frame = t.stack.last().ok_or("no memory context")?;
            // Preserve the common resolver's caller check before cache writes.
            self.world
                .objects
                .get(&frame.caller)
                .ok_or_else(|| error(10, "missing object"))?;
            let slot = frame.stack_object;
            let directory = self
                .world
                .person
                .as_mut()
                .ok_or("neighbor directory unavailable")?;
            if index == 9 {
                return Ok((
                    Cell::Literal(
                        directory
                            .resolve_family_friends(slot, self.world.autonomy_constants.as_ref())?,
                    ),
                    false,
                ));
            }
            let read = directory.neighbor_friend_count(
                program.neighbor_resource_flags.as_ref(),
                slot,
                self.world.autonomy_constants.as_ref(),
            )?;
            if let Some((id, count)) = read.cache {
                directory.neighbors.get_mut(&id).unwrap().friend_count = Some(count);
            }
            // Native can update its cache even when the expression later fails
            // because this scope supplies no writable destination pointer.
            return Ok((Cell::Literal(read.value), false));
        }
        let result = self.resolve_memory(program, t, scope, index, write)?;
        if let Cell::Data(object, 25) = result.0 {
            self.wake_object(program, t, object)?;
        }
        if write {
            if let Cell::Person(person, field @ (77 | 81)) = result.0 {
                self.world.notify_indicator_word(person, field)?;
            }
        }
        if write && matches!(result.0, Cell::Person(_, 68 | 74 | 84)) {
            // 4539c7 uses the executing caller's resource, even for scope 19.
            // The native helper ignores its argument 1 and invalidates by GUID.
            let caller = t.stack.last().ok_or("no memory frame")?.caller;
            let guid = self
                .world
                .objects
                .get(&caller)
                .and_then(|o| o.definition.as_ref())
                .ok_or("person-field caller resource absent")?
                .guid;
            self.world.image_cache.invalidate(guid)?;
            // Unlike 4eb0eb's optional UI notification, the resolver always
            // performs this second invalidation, including a cache miss.
            self.world.image_cache.invalidate_ui()?;
        }
        Ok(result)
    }
    pub(crate) fn write(
        &mut self,
        program: &Program,
        t: &mut Thread,
        scope: u8,
        index: i16,
        value: i16,
    ) -> Result<bool, String> {
        let (cell, writable) = self.resolve_runtime_memory(program, t, scope, index, true)?;
        if !writable {
            return Ok(false);
        }
        self.write_cell(t, cell, value)
    }
    pub(crate) fn relationship_cell(
        &mut self,
        program: &Program,
        t: &mut Thread,
        scope: u8,
        index: i16,
        relationship_value: Option<i16>,
    ) -> Result<i16, String> {
        // Native resolves with write intent in BOTH directions. It can still
        // read a literal/tuning value, but cannot store a relationship into it.
        let (cell, writable) = self.resolve_runtime_memory(program, t, scope, index, true)?;
        if let Some(relationship_value) = relationship_value {
            // 44d546 discards the resolver's returned word in this direction;
            // only its destination pointer is used. An inherited opaque local
            // can therefore be overwritten without interpreting its old bits.
            if !writable || !self.write_cell(t, cell, relationship_value)? {
                return Err(
                    "relationship destination has no word pointer (native error 0x13)".into(),
                );
            }
            Ok(relationship_value)
        } else {
            self.cell_value(t, cell)
        }
    }
    fn write_cell(&mut self, t: &mut Thread, cell: Cell, value: i16) -> Result<bool, String> {
        let f = t.stack.last_mut().ok_or("no memory frame")?;
        let target = match cell {
            Cell::Attribute(id, i) => &mut self.world.objects.get_mut(&id).unwrap().attributes[i],
            Cell::ResourceGlobal(i, j) => &mut self.world.resource_storage.records[i].values[j],
            Cell::Temp(id, i) => &mut self.world.objects.get_mut(&id).unwrap().temps[i],
            Cell::Extra(id, i) => {
                &mut self
                    .world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .dynamic_sprite_flags[i]
            }
            Cell::Data(id, i) => {
                if matches!(i, 2 | 3 | 11) {
                    return Err(format!(
                        "object field {i} needs an unsupported native side effect"
                    ));
                }
                self.world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .data
                    .get_mut(&i)
                    .ok_or("object data unavailable")?
            }
            Cell::Person(id, i) => self
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .person
                .as_mut()
                .unwrap()
                .get_mut(&i)
                .unwrap(),
            Cell::Global(i) => self.world.globals.get_mut(&i).ok_or("global unavailable")?,
            Cell::Arg(i) => &mut f.args[i],
            Cell::Local(i) => {
                f.opaque_locals.remove(&i);
                &mut f.locals[i]
            }
            Cell::StackObject => &mut f.stack_object,
            Cell::TreeAdvertisement(i, j) => &mut self.world.tree_advertisements[i][j],
            Cell::Literal(_) | Cell::Motive(..) => return Ok(false),
        };
        *target = value;
        Ok(true)
    }
    pub(crate) fn random_primitive(
        &mut self,
        program: &Program,
        t: &mut Thread,
        bytes: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let scope = |offset| {
            u8::try_from(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
                .map_err(|_| error(9, "invalid native memory scope"))
        };
        let ds = scope(2)?;
        let di = word(bytes, 0);
        let (dest, writable) = self.resolve_runtime_memory(program, t, ds, di, true)?;
        // Resolution also reads the old cell, so invalid/missing data faults before RNG.
        if !matches!(dest,Cell::Local(i) if t.stack.last().unwrap().opaque_locals.contains(&i)) {
            self.cell_value(t, dest)?;
        }
        let rs = scope(6)?;
        let ri = word(bytes, 4);
        let (range_cell, range_pointer) = self.resolve_runtime_memory(program, t, rs, ri, false)?;
        let range = self.cell_value(t, range_cell)?;
        if range == 0 {
            return Err(error(4, "Random Number range is zero"));
        }
        // Read-only resolution of global 34 unusually overwrites the integer
        // destination pointer. Preserve this observable native aliasing behavior.
        let (dest, writable) = if range_pointer && !matches!(range_cell, Cell::Motive(..)) {
            (range_cell, true)
        } else {
            (dest, writable)
        };
        if !writable {
            return Err(error(0x13, "Random Number destination is not writable"));
        }
        // Validate supported integer writes before consuming a draw.
        if !matches!(dest, Cell::Motive(..)) {
            let old = match dest {
                Cell::Local(i) if t.stack.last().unwrap().opaque_locals.contains(&i) => 0,
                _ => self.cell_value(t, dest)?,
            };
            self.write_cell(t, dest, old)?;
        }
        let value = crate::rng::next(&mut self.rng_state) % (i32::from(range) as u32);
        if let Cell::Motive(id, i) = dest {
            self.world
                .objects
                .get_mut(&id)
                .unwrap()
                .motives
                .as_mut()
                .unwrap()[i] = (value as i32 as f32).to_bits();
        } else {
            self.write_cell(t, dest, value as i16)?;
        }
        Ok((true, format!("original RNG modulo {range} → {value}")))
    }
    pub(crate) fn expression(
        &mut self,
        program: &Program,
        t: &mut Thread,
        bytes: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let (li, ri, operator, ls, rs) =
            (word(bytes, 0), word(bytes, 2), bytes[5], bytes[6], bytes[7]);
        // Sims 1.91 0x00452c45 ignores byte 4 and resolves RHS before LHS.
        // Integer behavior is checked by executing that original routine in the private oracle.
        let rhs = i32::from(self.read(program, t, rs, ri)?);
        let write = matches!(operator, 3..=7 | 9..=13 | 17..=20);
        let (destination, writable) = self.resolve_runtime_memory(program, t, ls, li, write)?;
        let lhs = if operator == 5
            && matches!(destination, Cell::Local(i) if t.stack.last().unwrap().opaque_locals.contains(&i))
        {
            0
        } else {
            i32::from(self.cell_value(t, destination)?)
        };
        if let Cell::Motive(id, index) = destination {
            if matches!(operator, 3..=7) {
                let old = f64::from(f32::from_bits(
                    self.world.objects[&id].motives.unwrap()[index],
                ));
                let value = match operator {
                    3 => old + f64::from(rhs),
                    4 => old - f64::from(rhs),
                    5 => f64::from(rhs),
                    6 => old * f64::from(rhs),
                    7 if rhs != 0 => old / f64::from(rhs),
                    _ => return Err(error(0x26, "expression division by zero")),
                } as f32;
                let value = value.clamp(-100.0, 100.0);
                self.world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .motives
                    .as_mut()
                    .unwrap()[index] = value.to_bits();
                return Ok((true, format!("motive {index} = {value}")));
            }
            if matches!(operator, 11 | 17) {
                return Err("native float increment/decrement dereferences a null integer cell; unsupported".into());
            }
            if matches!(operator, 9 | 10) && !(0..=16).contains(&rhs) {
                return Err(error(0x0d, "flag index out of bounds"));
            }
            if matches!(operator, 9 | 10 | 12 | 13 | 18 | 19 | 20) {
                return Err(error(0x13, "expression needs integer destination"));
            }
        }
        if matches!(operator, 11 | 17) {
            let value = (lhs + if operator == 11 { 1 } else { -1 }) as i16;
            if !writable || !self.write_cell(t, destination, value)? {
                return Err("expression destination is not writable (native error 0x13)".into());
            }
            let result = if operator == 11 {
                i32::from(value) < rhs
            } else {
                i32::from(value) > rhs
            };
            return Ok((
                result,
                format!("scope {ls}[{li}] = {value}; compare with original RHS {rhs}"),
            ));
        }
        let bit = || {
            if !(0..=16).contains(&rhs) {
                Err(format!("flag index {rhs} exceeds 0–16 (native error 0x0d)"))
            } else {
                // Native x86 masks the shift count. Index zero tests the sign bit
                // after sign extension; setting/clearing it leaves a 16-bit word unchanged.
                Ok(1i32.wrapping_shl((rhs - 1) as u32))
            }
        };
        let predicate = match operator {
            0 => Some(lhs > rhs),
            1 => Some(lhs < rhs),
            2 => Some(lhs == rhs),
            8 => Some(lhs & bit()? != 0),
            14 => Some(lhs >= rhs),
            15 => Some(lhs <= rhs),
            16 => Some(lhs != rhs),
            _ => None,
        };
        if let Some(result) = predicate {
            return Ok((result, format!("expression op {operator}: {lhs}, {rhs}")));
        }
        let value = match operator {
            3 => lhs + rhs,
            4 => lhs - rhs,
            5 => rhs,
            6 => lhs * rhs,
            7 => {
                if rhs == 0 {
                    return Err("expression division by zero (native error 0x26)".into());
                } else {
                    lhs / rhs
                }
            }
            9 => lhs | bit()?,
            10 => lhs & !bit()?,
            12 => {
                // The original requires a writable integer cell before checking zero.
                if !writable {
                    return Err("expression destination is not writable (native error 0x13)".into());
                }
                if rhs == 0 {
                    return Err("expression remainder by zero (native error 0x26)".into());
                }
                // Abs is stored in a 16-bit cell before signed remainder. MIN stays negative.
                let left = (lhs as i16).wrapping_abs();
                let right = (rhs as i16).wrapping_abs();
                i32::from(left) % i32::from(right)
            }
            13 => lhs & rhs,
            18 => lhs | rhs,
            19 => lhs ^ rhs,
            20 if rhs >= 0 => (rhs as u32).isqrt() as i32,
            20 => {
                return Err(
                    "negative square root needs original x87 exception-policy validation".into(),
                );
            }
            _ => {
                return Err(format!(
                    "unknown expression operator {operator} (native error 0x01)"
                ));
            }
        } as i16;
        let ok = writable && self.write_cell(t, destination, value)?;
        if !ok {
            return Err("expression destination is not writable (native error 0x13)".into());
        }
        Ok((
            ok,
            format!("expression op {operator}: scope {ls}[{li}] {lhs} → {value}, rhs {rhs}"),
        ))
    }
}
