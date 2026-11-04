//! Live registry projection for444cb9. No new object IDs or VM calls.
use super::{Object, Objects};
use crate::Simulation;
use sims_vm::{ObjectId, Program};
use std::collections::BTreeMap;

impl Simulation {
    pub fn value_original_objects(
        &mut self,
        program: &Program,
        size: usize,
    ) -> Result<Objects, String> {
        super::objects(&self.original_valuation_inputs(program, size)?)
    }
    ///The retained registry-order inputs also support private original-code
    ///differential probes; constructing this projection performs the native
    ///location getter's slot normalization, exactly as valuation does.
    pub fn original_valuation_inputs(
        &mut self,
        program: &Program,
        size: usize,
    ) -> Result<Vec<Object>, String> {
        let native = self
            .native_lot
            .as_ref()
            .ok_or("valuation native lot absent")?;
        if size < 3 || size > native.placement.links.size {
            return Err("valuation active size differs".into());
        }
        let ids = self
            .vm
            .world
            .person
            .as_ref()
            .ok_or("valuation registry absent")?
            .object_order
            .clone();
        let indices: BTreeMap<_, _> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
        if indices.len() != ids.len() {
            return Err("duplicate valuation registry object".into());
        }
        let mut rows = Vec::with_capacity(ids.len());
        for &id in &ids {
            let object = self
                .vm
                .world
                .objects
                .get(&id)
                .ok_or("valuation registry object absent")?;
            let def = object
                .definition
                .as_ref()
                .ok_or("valuation definition absent")?;
            let words = sims_vm::construction::definition_words(program, def)?;
            let module = &program.modules[&def.source];
            let master = if words[10] == 0 {
                words
            } else {
                module
                    .object_definition_order
                    .iter()
                    .find_map(|id| {
                        let w = &module.object_definition_words[id];
                        (w[10] == words[10] && w[11] == -1).then_some(w.as_slice())
                    })
                    .ok_or("valuation catalog master absent")?
            };
            let data = |key| {
                object
                    .data
                    .get(&key)
                    .copied()
                    .ok_or("valuation object data absent")
            };
            let link = |id: ObjectId| {
                indices
                    .get(&id)
                    .copied()
                    .ok_or("valuation multipart sibling unregistered")
            };
            let (head, next) = if let Some(part) = &object.multitile {
                (
                    part.head.map(link).transpose()?,
                    part.next.map(link).transpose()?,
                )
            } else {
                (None, None)
            };
            let position = *self
                .vm
                .world
                .positions
                .get(&id)
                .ok_or("valuation position absent")?;
            let [x, y, level] = sims_vm::loading::coarse_position(position);
            let in_world = x > 0 && y > 0 && (x as usize) < size - 1 && (y as usize) < size - 1;
            let tile_flags = if in_world {
                let f = usize::try_from(i32::from(level) - 1)
                    .ok()
                    .and_then(|i| native.placement.flags.get(i))
                    .ok_or("valuation physical floor absent")?;
                *f.get(x as usize * native.placement.links.size + y as usize)
                    .ok_or("valuation physical tile absent")?
            } else {
                0
            };
            rows.push(Object {
                category: data(59)?,
                kind: words[9],
                group: words[10],
                multipart_class: object.multitile.is_some(),
                head,
                next,
                in_world,
                tile_flags,
                flags43: data(43)? as u16,
                price: data(41)?,
                depreciated: data(15)? != 0,
                width: words[39],
                height: words[40],
                build_type: words[69],
                catalog_build_type: master[69],
            });
        }
        //432595's location getter may normalize a dangling zero-slot link to
        //-1. Only objects actually queried by444cb9 receive that side effect;
        //classification still precedes the primary-part and global-kind gates.
        for (i, (&id, row)) in ids.iter().zip(&rows).enumerate() {
            let primary =
                !(row.group != 0 && row.multipart_class && row.head.is_some_and(|head| head != i));
            if row.category == 1 || primary && row.kind != 7 {
                native.placement.links.capture(&mut self.vm.world, id)?;
            }
        }
        Ok(rows)
    }
}
