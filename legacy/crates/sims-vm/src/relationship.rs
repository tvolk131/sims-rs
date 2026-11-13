//! Original primitive 0x1a (0x44d17c): directed, growable signed-dword rows.
use crate::{Object, ObjectId, Program, Thread, Vm};
use std::borrow::Cow;
use std::collections::BTreeMap;
pub type Table = BTreeMap<i16, Vec<i32>>;

pub fn valid(table: &Table) -> bool {
    table.len() <= 32767 && table.values().all(|row| row.len() <= 256)
}

impl Object {
    /// Ordered native row enumeration (465209/465212). Empty and singleton
    /// legacy maps have an unambiguous order; larger ones need retained history.
    pub fn relationship_keys(&self) -> Result<Cow<'_, [ObjectId]>, String> {
        if let Some(order) = &self.relationship_order {
            if order.len() != self.relationships.len()
                || order
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != order.len()
                || order.iter().any(|id| !self.relationships.contains_key(id))
            {
                return Err("object relationship order differs from its rows".into());
            }
            Ok(Cow::Borrowed(order))
        } else if self.relationships.len() <= 1 {
            Ok(Cow::Owned(self.relationships.keys().copied().collect()))
        } else {
            Err("object relationship insertion order unavailable in legacy snapshot".into())
        }
    }

    fn record_relationship_insertion(&mut self, target: ObjectId) {
        if let Some(order) = &mut self.relationship_order {
            order.push(target);
        } else if self.relationships.len() <= 2 {
            // Called after insertion: at most one older row has a known order.
            let mut order: Vec<_> = self
                .relationships
                .keys()
                .copied()
                .filter(|id| *id != target)
                .collect();
            order.push(target);
            self.relationship_order = Some(order);
        }
    }

    pub fn insert_relationship(&mut self, target: ObjectId, values: Vec<i32>) -> Option<Vec<i32>> {
        let old = self.relationships.insert(target, values);
        if old.is_none() {
            self.record_relationship_insertion(target);
        }
        old
    }
}

impl Vm {
    pub(crate) fn relationship(
        &mut self,
        p: &Program,
        t: &mut Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        self.relationship_impl(p, t, b, None)
    }
    pub(crate) fn legacy_relationship(
        &mut self,
        p: &Program,
        t: &mut Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        if (b[1] as i8) < 0 {
            return Err("negative legacy relationship column is outside bounded storage".into());
        }
        let f = t.stack.last().ok_or("missing legacy relationship frame")?;
        let arg = if matches!(b[3], 2 | 3) {
            Some(
                *f.args
                    .first()
                    .ok_or("legacy relationship argument 0 unavailable (native error 0x08)")?,
            )
        } else {
            None
        };
        let index = i16::from(b[2] as i8).to_le_bytes();
        self.relationship_impl(
            p,
            t,
            &[
                b[1],
                b[3],
                (b[4] & 3) | if b[0] != 0 { 4 } else { 0 },
                0,
                9,
                0,
                index[0],
                index[1],
            ],
            arg,
        )
    }
    fn relationship_impl(
        &mut self,
        p: &Program,
        t: &mut Thread,
        b: &[u8; 8],
        arg_override: Option<i16>,
    ) -> Result<(bool, String), String> {
        let f = t.stack.last().ok_or("missing relationship frame")?;
        let arg = if let Some(arg) = arg_override {
            arg
        } else if matches!(b[1], 2 | 3) {
            if f.opaque_locals.contains(&usize::from(b[3])) {
                return Err("relationship reads opaque native local".into());
            }
            *f.locals
                .get(usize::from(b[3]))
                .ok_or("relationship local out of bounds (native error 0x33)")?
        } else {
            0
        };
        let mut neighbor = b[2] & 2 != 0;
        let caller = if neighbor && b[1] <= 1 {
            *self
                .world
                .objects
                .get(&f.caller)
                .and_then(|o| o.person.as_ref())
                .ok_or("relationship caller is not a person (native error 0x1c)")?
                .get(&31)
                .ok_or("missing caller neighbor ID")?
        } else {
            f.caller
        };
        let (mut source, mut target) = match b[1] {
            0 => (caller, f.stack_object),
            1 => (f.stack_object, caller),
            2 => (f.stack_object, arg),
            3 => (arg, f.stack_object),
            _ => return Err("invalid relationship direction (native error 0x15)".into()),
        };
        if !neighbor {
            let a = self
                .world
                .objects
                .get(&source)
                .ok_or("missing relationship object (native error 0x15)")?;
            let z = self
                .world
                .objects
                .get(&target)
                .ok_or("missing relationship object (native error 0x15)")?;
            let is_person = |o: &crate::Object| {
                o.definition
                    .as_ref()
                    .and_then(|d| {
                        p.modules
                            .get(&d.source)?
                            .object_definition_words
                            .get(&d.object)
                    })
                    .and_then(|words| words.get(9))
                    .copied()
                    == Some(2)
            };
            if is_person(a) && is_person(z) {
                source = *a
                    .person
                    .as_ref()
                    .and_then(|v| v.get(&31))
                    .ok_or("missing source neighbor ID")?;
                target = *z
                    .person
                    .as_ref()
                    .and_then(|v| v.get(&31))
                    .ok_or("missing target neighbor ID")?;
                neighbor = true;
            }
        }
        let writing = b[2] & 4 != 0;
        let table = if neighbor {
            let directory = self
                .world
                .person
                .as_mut()
                .ok_or("neighbor directory unavailable")?;
            if !directory.neighbors.contains_key(&source)
                || !directory.neighbors.contains_key(&target)
            {
                return Err("missing relationship neighbor (native error 0x15)".into());
            }
            // Notification precedes the absent-column test, even on false/error.
            if writing {
                directory.mark_relationship_dirty(source)?;
                directory.mark_relationship_dirty(target)?;
                directory.relationships_dirty = true;
            }
            &mut directory.neighbors.get_mut(&source).unwrap().relationships
        } else {
            &mut self.world.objects.get_mut(&source).unwrap().relationships
        };
        let column = usize::from(b[0]);
        let mut inserted = false;
        if table.get(&target).map_or(0, Vec::len) <= column {
            if b[2] & 1 != 0 {
                return Ok((false, "relationship column absent".into()));
            }
            inserted = !table.contains_key(&target);
            table.entry(target).or_default().resize(column + 1, 0);
        }
        let old = table[&target][column] as i16;
        if inserted && !neighbor {
            self.world
                .objects
                .get_mut(&source)
                .unwrap()
                .record_relationship_insertion(target);
        }
        let value = self.relationship_cell(
            p,
            t,
            b[4],
            i16::from_le_bytes([b[6], b[7]]),
            (!writing).then_some(old),
        )?;
        if writing {
            let table = if neighbor {
                &mut self
                    .world
                    .person
                    .as_mut()
                    .unwrap()
                    .neighbors
                    .get_mut(&source)
                    .unwrap()
                    .relationships
            } else {
                &mut self.world.objects.get_mut(&source).unwrap().relationships
            };
            table.get_mut(&target).unwrap()[column] = i32::from(value);
        }
        Ok((
            true,
            format!(
                "{} relationship {source} -> {target} column {column}",
                if writing { "wrote" } else { "read" }
            ),
        ))
    }
}
