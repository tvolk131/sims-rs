//! Borrowed original-lot adapter for the presenter classification service.
//! The enclosing F6 camera/floor/record pass is a separate caller. No tick or
//! renderer silently invokes this service or supplies missing native fields.
use crate::lot_links::Links;
use serde::{Deserialize, Serialize};
use sims_vm::{Object, ObjectId, Program, World, presenter::*, refresh::DrawState};
use std::collections::BTreeMap;
pub mod camera;
pub mod camera_command;
pub mod centering;
pub mod forward_projection;
pub mod ground;
pub mod hover;
pub mod inverse_projection;
pub mod lifecycle;
pub mod motion;
pub mod open;
pub mod person_refresh;
pub mod redraw;
pub mod room_masks;
pub mod tile_scan;
pub mod level_geometry;
pub mod tile_depth;
pub mod row_tail;
pub mod terrain_shade;
pub mod terrain_walls;
pub mod floor_prep;
pub mod floor_light;
pub mod sprite_full;
pub mod sprite_clipped;
pub mod sprite2_clipped;
pub mod contained_slot;
pub mod physical_next;
pub mod object_cutaway;
pub mod object_shadow;
pub mod sprite_gradient;
pub mod dual_sprite;
pub mod terrain_clip;
pub mod terrain_raster;
pub mod scroll;
pub mod transition;
pub mod viewport;
pub mod visibility;
pub mod walls;

/// Presenter host fields, distinct from object data and sprite presence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    /// Process singleton +5c, not a classification-derived flag.
    pub force_dynamic: bool,
    /// Ordinary object +180, supplied by retained graphic ownership4376f6.
    pub graphic_ready: BTreeMap<ObjectId, u8>,
    /// Explicit +188 input for process/debug states without a retained object
    /// byte. A supplied value must agree with a known constructor-owned value.
    pub keep_dynamic: BTreeMap<ObjectId, u8>,
}

/// Borrowed immutable definition storage. Captured replay providers must bind
/// the actual source/chunk identity; absent words are never defaulted.
pub trait Definitions {
    fn words(&self, definition: &sims_vm::Definition) -> Result<&[i16], String>;
}
impl Definitions for Program {
    fn words(&self, definition: &sims_vm::Definition) -> Result<&[i16], String> {
        sims_vm::construction::definition_words(self, definition)
    }
}

pub struct View<'a> {
    world: &'a mut World,
    definitions: &'a dyn Definitions,
    links: &'a Links,
    inputs: &'a Inputs,
    /// The original440349 cache, already serviced by the caller.
    order: &'a [ObjectId],
}
impl<'a> View<'a> {
    pub fn new(
        world: &'a mut World,
        program: &'a Program,
        links: &'a Links,
        inputs: &'a Inputs,
        order: &'a [ObjectId],
    ) -> Result<Self, String> {
        Self::with_definitions(world, program, links, inputs, order)
    }
    pub fn with_definitions(
        world: &'a mut World,
        definitions: &'a dyn Definitions,
        links: &'a Links,
        inputs: &'a Inputs,
        order: &'a [ObjectId],
    ) -> Result<Self, String> {
        let view = Self {
            world,
            definitions,
            links,
            inputs,
            order,
        };
        view.validate()?;
        Ok(view)
    }
    fn object(&self, id: ObjectId) -> Result<&Object, String> {
        self.world
            .objects
            .get(&id)
            .ok_or_else(|| "presenter object absent".into())
    }
    fn words(&self, id: ObjectId) -> Result<&[i16], String> {
        self.definitions.words(
            self.object(id)?
                .definition
                .as_ref()
                .ok_or("presenter definition absent")?,
        )
    }
}
impl OrderView for View<'_> {
    fn rotation(&self) -> u32 {
        u32::from(self.world.camera_rotation)
    }
    fn size(&self) -> i32 {
        self.world
            .map_size
            .map_or(self.links.size as i32, |map| map.active_size)
    }
    fn registered(&self) -> &[ObjectId] {
        self.world
            .person
            .as_ref()
            .map_or(&[], |p| p.object_order.as_slice())
    }
    fn cached_tile(&self, id: ObjectId) -> Result<[i8; 3], String> {
        let p = self
            .world
            .positions
            .get(&id)
            .ok_or("presenter position absent")?;
        // The native cached column,row pair reverses the engine point's XY.
        Ok([(p.y >> 4) as i8, (p.x >> 4) as i8, p.level as i8])
    }
}
impl DrawView for View<'_> {
    fn slots(&self, id: ObjectId) -> Result<&[Option<ObjectId>], String> {
        Ok(&self.object(id)?.slots)
    }
    fn exists(&self, id: ObjectId) -> bool {
        self.world.objects.contains_key(&id)
    }
    fn definition_flags(&self, id: ObjectId) -> Result<u16, String> {
        self.words(id)?
            .get(101)
            .map(|v| *v as u16)
            .ok_or_else(|| "presenter OBJD flags absent".into())
    }
    fn force_dynamic(&self) -> bool {
        self.inputs.force_dynamic
    }
    fn visible_level(&self) -> i32 {
        self.world
            .refresh
            .as_ref()
            .expect("validated presenter refresh")
            .visible_level
    }
    fn clock(&self) -> u32 {
        self.world
            .refresh
            .as_ref()
            .expect("validated presenter refresh")
            .render_clock
    }
    fn draw(&self, id: ObjectId) -> Result<DrawState, String> {
        self.world
            .refresh
            .as_ref()
            .and_then(|r| r.draw.get(&id))
            .cloned()
            .ok_or_else(|| "presenter draw state absent".into())
    }
    fn write_draw(&mut self, id: ObjectId, draw: DrawState) -> Result<(), String> {
        *self
            .world
            .refresh
            .as_mut()
            .and_then(|r| r.draw.get_mut(&id))
            .ok_or("presenter draw state absent")? = draw;
        Ok(())
    }
}
impl DurationView for View<'_> {
    fn is_person(&self, id: ObjectId) -> Result<bool, String> {
        Ok(self.object(id)?.person.is_some())
    }
    fn keep_dynamic(&self, id: ObjectId) -> Result<bool, String> {
        let retained = self.object(id)?.native_keep_dynamic;
        let supplied = self.inputs.keep_dynamic.get(&id).copied();
        if let (Some(a), Some(b)) = (retained, supplied) {
            if a != b {
                return Err("presenter keep-dynamic input contradicts object storage".into());
            }
        }
        retained
            .or(supplied)
            .map(|v| v != 0)
            .ok_or_else(|| "presenter keep-dynamic byte absent".into())
    }
}
impl ClassificationView for View<'_> {
    fn order(&self) -> &[ObjectId] {
        self.order
    }
    fn word34(&self, id: ObjectId) -> Result<i16, String> {
        self.object(id)?
            .data
            .get(&34)
            .copied()
            .ok_or_else(|| "presenter object word34 absent".into())
    }
    fn graphic_ready(&self, id: ObjectId) -> Result<bool, String> {
        if self.is_person(id)? {
            Ok(true)
        } else {
            self.inputs
                .graphic_ready
                .get(&id)
                .map(|v| *v != 0)
                .ok_or_else(|| "presenter ordinary readiness absent".into())
        }
    }
    fn has_drawings(&self, id: ObjectId) -> Result<bool, String> {
        let words = self.words(id)?;
        Ok(*words.get(10).ok_or("presenter OBJD group absent")? != 0
            || *words.get(4).ok_or("presenter OBJD graphics count absent")? != 0)
    }
    fn parent(&mut self, id: ObjectId) -> Result<Option<ObjectId>, String> {
        Ok(self.links.capture(self.world, id)?.container)
    }
    fn duration(&self, id: ObjectId) -> Result<i32, String> {
        duration(self, id)
    }
    fn outside_interior(&self, id: ObjectId) -> Result<bool, String> {
        Ok(outside(self.cached_tile(id)?, self.size()))
    }
}
impl View<'_> {
    pub fn validate(&self) -> Result<(), String> {
        if self.world.refresh.is_none() || self.world.person.is_none() {
            return Err("presenter native context absent".into());
        }
        Ok(())
    }
    /// Continuous software presentation redraws the viewport; retain the object
    /// invalidation before acknowledging434657's pending flag store.
    pub fn invalidate(&mut self, id: ObjectId) -> Result<(), String> {
        self.draw(id)?;
        self.world.refresh.as_mut().unwrap().invalidated.insert(id);
        Ok(())
    }
}

pub mod palette_select;
