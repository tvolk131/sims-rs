//! Physical-query adapter for Burn. Native links and published bounds are
//! authoritative; rendering bodies and recomputed object footprints are not.
use super::{Object, Wall};
use crate::{Simulation, lot_geometry::Lot, lot_placement};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, Program};

/// Identity of the resource returned by 43ac10, corresponding to object+11c.
/// A GUID alone is not a substitute for this loaded-resource identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Resource {
    pub source: String,
    pub definition: u16,
}
impl Resource {
    /// Search only the linked catalog. The asset host must distinguish an
    /// unloaded resource from an actual failed original resource lookup.
    pub fn loaded_fire(program: &Program) -> Result<Option<Self>, String> {
        let mut found = None;
        for module in program.modules.values() {
            for (&definition, words) in &module.object_definition_words {
                if words.len() > 15
                    && u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16)
                        == super::FIRE_GUID
                {
                    if found.is_some() {
                        return Err("ambiguous loaded Burn fire resource".into());
                    }
                    found = Some(Self {
                        source: module.source.clone(),
                        definition,
                    });
                }
            }
        }
        Ok(found)
    }
}

pub struct Live<'a> {
    program: &'a Program,
    world: &'a mut sims_vm::World,
    placement: &'a lot_placement::State,
    lot: &'a Lot,
    fire: Option<&'a Resource>,
}
impl<'a> Live<'a> {
    pub fn new(
        program: &'a Program,
        simulation: &'a mut Simulation,
        fire: Option<&'a Resource>,
    ) -> Result<Self, String> {
        Ok(Self {
            program,
            world: &mut simulation.vm.world,
            placement: &simulation
                .native_lot
                .as_ref()
                .ok_or("Burn native physical state absent")?
                .placement,
            lot: simulation
                .architecture
                .as_ref()
                .and_then(|a| a.original.as_ref())
                .ok_or("Burn original architecture absent")?,
            fire,
        })
    }
}
impl super::World for Live<'_> {
    fn size(&self) -> usize {
        self.placement.links.size
    }
    fn object(&self, id: ObjectId) -> Result<Option<Object>, String> {
        let Some(object) = self.world.objects.get(&id) else {
            return Ok(None);
        };
        let definition = object.definition.as_ref().ok_or("Burn definition absent")?;
        let module = self
            .program
            .modules
            .get(&definition.source)
            .ok_or("Burn object module absent")?;
        let words = module
            .object_definition_words
            .get(&definition.object)
            .filter(|words| words.len() > 10)
            .ok_or("Burn object OBJD absent")?;
        let word = |index| {
            object
                .data
                .get(&index)
                .copied()
                .ok_or_else(|| format!("Burn object #{id} word {index} absent"))
        };
        let position = *self
            .world
            .positions
            .get(&id)
            .ok_or("Burn position absent")?;
        let group_excluded = if words[10] != 0 && word(40)? as u16 & 32 != 0 {
            if object.multitile.is_none() {
                return Err("Burn multipart resource lacks native group links".into());
            }
            // 42344e reads this resource's function-8 action; an index beyond
            // the loaded OBJf vector returns zero in 455fa9.
            module
                .object_functions
                .get(&definition.object)
                .ok_or("Burn multipart function table absent")?
                .get(8)
                .is_some_and(|function| function.action != 0)
        } else {
            false
        };
        Ok(Some(Object {
            position: [position.x, position.y],
            level: position.level,
            tile: sims_vm::loading::coarse_position(position),
            direction: word(1)?,
            flags: word(8)? as u16,
            flags2: word(40)? as u16,
            uses: word(62)?,
            kind: words[9],
            master: words[10] != 0,
            group_excluded,
            head: object.multitile.as_ref().and_then(|m| m.head),
            next: object.multitile.as_ref().and_then(|m| m.next),
            fire_resource: self.fire.is_some_and(|r| {
                r.source == definition.source && r.definition == definition.object
            }),
            bounds: *self
                .placement
                .bounds
                .get(&id)
                .ok_or("Burn published bounds absent")?,
        }))
    }
    fn on_lot(&mut self, id: ObjectId) -> Result<bool, String> {
        // 432595 calls the descriptor capture, including normalization of a
        // dangling physical-slot-zero predecessor. Preserve that side effect.
        Ok(self.placement.links.capture(self.world, id)?.on_lot)
    }
    fn tile(&mut self, tile: [i8; 3]) -> Result<Vec<ObjectId>, String> {
        match self.placement.links.head(self.world, tile) {
            Some(head) => self.placement.links.walk(self.world, head),
            None => Ok(vec![]),
        }
    }
    fn wall(&mut self, [column, row, level]: [i8; 3]) -> Result<Wall, String> {
        let wall = self
            .lot
            .wall_grid(i32::from(level))?
            .expanded(column as usize, row as usize)?;
        let mut styles = [0; 6];
        for (i, style) in styles.iter_mut().enumerate() {
            if wall.segments & (1 << i) != 0 {
                *style = wall.styles[i].ok_or("Burn wall segment style absent")? as i16;
            }
        }
        Ok(Wall {
            mask: u32::from(wall.segments),
            styles,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::burn::World;

    #[test]
    fn reads_physical_order_published_bounds_and_exact_resource_identity() {
        let (mut p, mut s) = crate::lot_runtime::tests::fixture();
        s.native_lot
            .as_mut()
            .unwrap()
            .placement
            .bounds
            .insert(3, [61, 52, 37, 41]);
        // A logical child follows the physical slot-zero subtree, independent
        // of numeric registry order and the presentation body's geometry.
        s.native_lot
            .as_mut()
            .unwrap()
            .placement
            .links
            .next
            .insert(3, 0);
        s.vm.world.objects.get_mut(&3).unwrap().slots = vec![Some(2)];
        let child = s.vm.world.objects.get_mut(&2).unwrap();
        child.container = Some((3, 0));
        child.data.extend([(14, 1), (26, 3)]);
        let words = p.modules["objects"].object_definition_words[&1].clone();
        p.modules
            .get_mut("objects")
            .unwrap()
            .object_definition_words
            .insert(2, words);
        s.vm.world
            .objects
            .get_mut(&3)
            .unwrap()
            .definition
            .as_mut()
            .unwrap()
            .object = 2;
        let resource = Resource {
            source: "objects".into(),
            definition: 1,
        };
        let mut w = Live::new(&p, &mut s, Some(&resource)).unwrap();
        assert_eq!(w.tile([2, 2, 1]).unwrap(), [1, 3, 2]);
        assert_eq!(w.object(3).unwrap().unwrap().bounds, [61, 52, 37, 41]);
        assert!(w.object(1).unwrap().unwrap().fire_resource);
        assert!(!w.object(3).unwrap().unwrap().fire_resource);
        assert!(w.on_lot(2).unwrap());
        assert!(!w.on_lot(4).unwrap());
        assert!(w.tile([-1, 2, 1]).unwrap().is_empty());
    }

    #[test]
    fn group_exclusion_reads_function_eight_without_running_it() {
        let (mut p, mut s) = crate::lot_runtime::tests::fixture();
        let module = p.modules.get_mut("objects").unwrap();
        module.object_definition_words.get_mut(&1).unwrap()[10] = 1;
        let object = s.vm.world.objects.get_mut(&3).unwrap();
        object.data.insert(40, 32);
        object.multitile = Some(sims_vm::multitile::Part {
            head: Some(1),
            next: Some(2),
            origin: [0; 3],
            offset: [0; 3],
        });
        for action in [0, 4096, 0] {
            p.modules
                .get_mut("objects")
                .unwrap()
                .object_functions
                .get_mut(&1)
                .unwrap()[8]
                .action = action;
            let w = Live::new(&p, &mut s, None).unwrap();
            let object = w.object(3).unwrap().unwrap();
            assert_eq!(object.group_excluded, action != 0);
            assert_eq!((object.head, object.next), (Some(1), Some(2)));
        }
        p.modules
            .get_mut("objects")
            .unwrap()
            .object_functions
            .get_mut(&1)
            .unwrap()
            .truncate(8);
        assert!(
            !Live::new(&p, &mut s, None)
                .unwrap()
                .object(3)
                .unwrap()
                .unwrap()
                .group_excluded
        );
    }
}
