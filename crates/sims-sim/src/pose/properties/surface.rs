//! 5890a8 footstep material precedence; world queries stay explicit and lazy.
pub trait World {
    fn level(&self) -> i8;
    fn water(&mut self) -> Result<bool, String>;
    /// GUIDs of physically enumerated, uncontained objects, in native order.
    fn objects(&mut self) -> Result<Vec<u32>, String>;
    fn pool(&mut self) -> Result<u8, String>;
    fn covering(&mut self) -> Result<u16, String>;
    fn category(&mut self, covering: u16) -> Result<u8, String>;
}

/// Low byte of 49fd95's category/RGB label parser; matching is case-sensitive.
pub fn category(label: &str) -> u8 {
    match label.as_bytes().first() {
        Some(b'C') => 3,
        Some(b'M') => 1,
        Some(b'S') => 2,
        _ => 0,
    }
}

pub fn query(world: &mut impl World) -> Result<i32, String> {
    if world.level() == 0 && world.water()? {
        return Ok(10);
    }
    let mut surface = 0;
    for guid in world.objects()? {
        let object = match guid {
            0x96e3398d | 0xa2daa08c | 0xc02b406a | 0x65274a4f => 6,
            0xbf62f653 | 0x7f907075 => 7,
            0x63416ba1 => 8,
            99_999_999 => 9,
            0x9be030b9 | 0x3e7470f6 | 0x5c67fc8f => 10,
            _ => 0,
        };
        surface = surface.max(object);
    }
    if surface != 0 {
        return Ok(surface);
    }
    if world.level() == 1 {
        match world.pool()? {
            0..=15 => return Ok(11),
            16 => return Ok(1),
            _ => (),
        }
    }
    Ok(match world.covering()? {
        0 => 0,
        255 => 2,
        covering => match world.category(covering)? {
            0 => 3,
            1 => 2,
            2 => 4,
            3 => 1,
            _ => 0,
        },
    })
}

struct Live<'a> {
    s: &'a mut crate::Simulation,
    tile: [i8; 3],
    categories: &'a std::collections::BTreeMap<u16, u8>,
}
impl Live<'_> {
    fn lot(&self) -> Result<&crate::lot_geometry::Lot, String> {
        self.s
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .ok_or("footstep original lot absent".into())
    }
    fn cell(&self) -> Result<usize, String> {
        let size = self.lot()?.size;
        let [x, y, _] = self.tile;
        if x < 0 || y < 0 || x as usize >= size || y as usize >= size {
            return Err("footstep point outside original arrays".into());
        }
        Ok(x as usize * size + y as usize)
    }
}
impl World for Live<'_> {
    fn level(&self) -> i8 {
        self.tile[2]
    }
    fn water(&mut self) -> Result<bool, String> {
        // 478f4a itself gates on level 1, even though 5890a8 calls it
        // only at level 0. Preserve that original, unreachable early return.
        Ok(self.tile[2] == 1 && self.lot()?.water[self.cell()?] != 255)
    }
    fn objects(&mut self) -> Result<Vec<u32>, String> {
        let links = &self
            .s
            .native_lot
            .as_ref()
            .ok_or("footstep physical links absent")?
            .placement
            .links;
        let Some(head) = links.head(&self.s.vm.world, self.tile) else {
            return Ok(vec![]);
        };
        let mut guids = vec![];
        let mut cursor = crate::lot_links::Cursor::new(head);
        let mut seen = std::collections::BTreeSet::new();
        while let Some(id) = cursor.current {
            if !seen.insert(id) {
                return Err("cyclic footstep physical enumeration".into());
            }
            // 43265e/4325b5 also normalizes an unresolved slot-zero predecessor.
            if links.capture(&mut self.s.vm.world, id)?.container.is_none() {
                guids.push(
                    self.s.vm.world.objects[&id]
                        .definition
                        .as_ref()
                        .ok_or("footstep object resource absent")?
                        .guid,
                );
            }
            cursor.advance(links, &self.s.vm.world)?;
        }
        Ok(guids)
    }
    fn pool(&mut self) -> Result<u8, String> {
        Ok(self.lot()?.pool[self.cell()?])
    }
    fn covering(&mut self) -> Result<u16, String> {
        Ok(self
            .lot()?
            .floor(i32::from(self.tile[2]))
            .ok_or("footstep floor absent")?
            .source
            .coverings[self.cell()?])
    }
    fn category(&mut self, covering: u16) -> Result<u8, String> {
        self.categories
            .get(&covering)
            .copied()
            .ok_or_else(|| format!("footstep floor category {covering} not loaded"))
    }
}
impl crate::Simulation {
    pub fn original_footstep_surface(
        &mut self,
        actor: sims_vm::ObjectId,
        categories: &std::collections::BTreeMap<u16, u8>,
    ) -> Result<i32, String> {
        let r = self
            .characters
            .get(&actor)
            .ok_or("footstep character absent")?;
        let level = self
            .vm
            .world
            .positions
            .get(&actor)
            .ok_or("footstep person position absent")?
            .level;
        let Some(tile) = super::footstep_tile(
            [r.character.placement.base[0], r.character.placement.base[1]],
            level,
        )?
        else {
            return Ok(-1);
        };
        query(&mut Live {
            s: self,
            tile,
            categories,
        })
    }
}
