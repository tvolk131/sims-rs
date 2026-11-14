//! The room update, as the interpreter runs it: [`World::update_rooms`]
//! with the objects standing on each tile and the constants of the global
//! file.
//!
//! The update itself is in `sims_sim::rooms`. It needs two things only
//! this crate has: each object's type, which is in its definition, and
//! the links from a tile to the objects standing on it (`placement.rs`).

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use crate::placement::{standing_on, stands_on_lot};
use sims_sim::World;
use sims_sim::rooms::{RoomError, Standing};

/// Data word 40, and its flag for a light source.
const FLAGS_2: usize = 40;
const LIGHT_SOURCE: u16 = 0x400;

impl Interpreter<'_> {
    /// The objects standing on a tile, as the room update is told of them
    /// (0x467e05): each object's type, and for a light source whether it
    /// stands on the lot. Asking that of an object whose slot word is 0
    /// with nothing before it puts the word right, as the original's
    /// lookup does (0x4325b5).
    fn standing(&self, world: &mut World, tile: [u8; 3]) -> Result<Vec<Standing>, RoomError> {
        let fail = |why: Trap| RoomError::Tile {
            tile,
            why: why.to_string(),
        };
        let mut list = Vec::new();
        for id in standing_on(world, tile).map_err(fail)? {
            let kind = self.kind(world, id).map_err(fail)?;
            let light = world
                .objects
                .get(&id)
                .and_then(|object| object.data.get(FLAGS_2))
                .is_some_and(|&flags| flags as u16 & LIGHT_SOURCE != 0);
            let on_lot = light && stands_on_lot(world, id).map_err(fail)?;
            list.push(Standing { id, kind, on_lot });
        }
        Ok(list)
    }

    /// The room update for every room (`floor` 0) or one level's rooms
    /// (0x466c3c). See [`World::update_rooms`]. Returns the rooms counted.
    ///
    /// The original runs it for every room when the lot's rooms have been
    /// worked out again and every object has been given its room (signal
    /// 0xf0, load step 10a).
    pub fn update_rooms(&self, world: &mut World, floor: u8) -> Result<Vec<i16>, Fault> {
        let tuning = self.program.room_tuning();
        world
            .update_rooms(&tuning, floor, &mut |world, tile| {
                self.standing(world, tile)
            })
            .map_err(room_fault)
    }

    /// The room update for the levels that have a room waiting, or
    /// nothing if none has (0x467094). The original does this at the end
    /// of reading a house and before it draws each frame. Returns the
    /// rooms counted.
    pub fn update_pending_rooms(&self, world: &mut World) -> Result<Vec<i16>, Fault> {
        let tuning = self.program.room_tuning();
        world
            .update_pending_rooms(&tuning, &mut |world, tile| self.standing(world, tile))
            .map_err(room_fault)
    }
}

fn room_fault(error: RoomError) -> Fault {
    Fault {
        object: 0,
        at: None,
        trap: match error {
            RoomError::Occupancy(_) => Trap::Unsupported(error.to_string()),
            _ => Trap::Invalid(error.to_string()),
        },
    }
}
