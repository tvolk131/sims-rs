//! A step run both ways while a person's animator refresh is pending.
//!
//! A person that takes itself off the lot in its own tick has its
//! animator refreshed there (0x58da12) only if the house view drew it at
//! its last draw (animator byte +0x109, presentation, not kept). The
//! refresh is worked out on a copy (`Interpreter::refresh_at_take_off`),
//! and the two animators, as the take-off left it and as the refresh
//! would have, are kept apart ([`crate::placement::Pending`]) until the
//! person's animator's tick settles them
//! (`Interpreter::tick_animator_after_take_off`).
//!
//! A primitive that reads nothing of an animator runs as usual in
//! between ([`crate::placement::reads_no_animator`]). Any other runs both
//! ways ([`Machine::step_both_ways`]): once on a copy of the lot as it
//! is, and once on a copy with the refreshed animator, each with its own
//! copies of the cells the machine shares with the rest of the tick
//! ([`Cells`]). If the two runs give the same result, leave the same lot
//! apart from that person's animator, and leave the same in every cell,
//! it does not matter which the original made: the first run is kept,
//! and both animators are kept apart as the step left them. That holds
//! for a result that is a fault too: a step that faults the same way
//! both ways keeps what it did before the fault, as any step does.
//! Otherwise the step fails, and the lot and the cells are as they were
//! before it.

use crate::Trap;
use crate::interpreter::{Asked, AtOnce, Interpreter, Machine, Status};
use crate::orders::Checking;
use crate::placement::{Kept, Pending, Refreshing};
use crate::primitives::ui_effect::UiEffectEvent;
use crate::sounds::SoundEvent;
use sims_formats::saved_objects::Interaction;
use sims_sim::World;
use std::cell::{Cell, RefCell, RefMut};
use std::collections::{BTreeMap, BTreeSet};

/// Copies of what each cell an interpreter shares with the rest of the
/// tick holds, so that a run can be made apart on them, compared with
/// another, and kept or dropped. Every `RefCell` and `Cell` an
/// [`Interpreter`] (and so a [`Machine`]) holds is here; the rest of
/// either is values or shared inputs that nothing writes (the program,
/// the catalog).
pub(crate) struct Cells {
    asked: Option<RefCell<Vec<Asked>>>,
    deleted: Option<RefCell<Vec<u16>>>,
    sounds: Option<RefCell<Vec<SoundEvent>>>,
    ui_effects: Option<RefCell<Vec<UiEffectEvent>>>,
    masks_rebuilt: Option<Cell<u8>>,
    menu: Option<RefCell<Checking>>,
    /// The action of `running_at_once`.
    action: Option<RefCell<Interaction>>,
    letting_go: Option<RefCell<BTreeSet<u16>>>,
    deleting: Option<RefCell<BTreeSet<u16>>>,
    refreshing: Option<RefCell<Refreshing>>,
    taken_off: Option<RefCell<BTreeMap<u16, Pending>>>,
    coop: Option<RefCell<crate::coop::CoopVmContext>>,
    special_panels: Option<RefCell<crate::primitives::dialog::SpecialPanels>>,
}

/// A copy of what one cell holds.
fn copy<T: Clone>(cell: Option<&RefCell<T>>) -> Option<RefCell<T>> {
    cell.map(|c| RefCell::new(c.borrow().clone()))
}

/// Whether two runs' copies of a cell hold the same, by `alike`.
fn alike<T>(
    a: &Option<RefCell<T>>,
    b: &Option<RefCell<T>>,
    alike: impl Fn(&T, &T) -> bool,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => alike(&a.borrow(), &b.borrow()),
        (None, None) => true,
        _ => false,
    }
}

/// An action written out, its float (the attenuation) as its bits:
/// `==` on it calls -0.0 and 0.0 alike and NaNs different, and its
/// `Debug` calls every NaN alike. The other floats in the cells are
/// [`sims_core::float::F32`]s, whose `==` compares their bits.
fn bits(action: &Interaction) -> Vec<u8> {
    postcard::to_allocvec(action).expect("an action is written out")
}

/// Whether two listings' checks hold the same, their actions bit for bit.
fn same_checking(a: &Checking, b: &Checking) -> bool {
    bits(&a.action) == bits(&b.action)
        && a.added.len() == b.added.len()
        && a.added
            .iter()
            .zip(&b.added)
            .all(|(a, b)| bits(&a.action) == bits(&b.action) && a.name == b.name)
}

/// Whether two refreshes are the same, their animators bit for bit.
fn same_refreshing(a: &Refreshing, b: &Refreshing) -> bool {
    a.person == b.person && a.pending == b.pending && a.both_ways == b.both_ways
}

/// A cell borrowed to be written, before anything is.
fn take<T>(cell: Option<&RefCell<T>>) -> Option<RefMut<'_, T>> {
    cell.map(RefCell::borrow_mut)
}

/// Puts what `copy` holds into the cell `held` borrows.
fn put<T>(held: Option<RefMut<'_, T>>, copy: Option<RefCell<T>>) {
    if let (Some(mut held), Some(copy)) = (held, copy) {
        *held = copy.into_inner();
    }
}

impl Cells {
    /// Copies of `interpreter`'s cells.
    pub(crate) fn of(interpreter: &Interpreter) -> Self {
        Self {
            asked: copy(interpreter.asked),
            deleted: copy(interpreter.deleted),
            sounds: copy(interpreter.sounds),
            ui_effects: copy(interpreter.ui_effects),
            masks_rebuilt: interpreter.masks_rebuilt.map(|c| Cell::new(c.get())),
            menu: copy(interpreter.menu),
            action: copy(interpreter.running_at_once.map(|a| a.action)),
            letting_go: copy(interpreter.letting_go),
            deleting: copy(interpreter.deleting),
            refreshing: copy(interpreter.refreshing),
            taken_off: copy(interpreter.taken_off),
            coop: copy(interpreter.coop),
            special_panels: copy(interpreter.special_panels),
        }
    }

    /// `interpreter` with these cells in place of its own.
    pub(crate) fn on<'b>(&'b self, interpreter: &Interpreter<'b>) -> Interpreter<'b> {
        Interpreter {
            asked: self.asked.as_ref(),
            deleted: self.deleted.as_ref(),
            sounds: self.sounds.as_ref(),
            ui_effects: self.ui_effects.as_ref(),
            masks_rebuilt: self.masks_rebuilt.as_ref(),
            menu: self.menu.as_ref(),
            running_at_once: interpreter.running_at_once.zip(self.action.as_ref()).map(
                |(a, action)| AtOnce {
                    person: a.person,
                    action,
                },
            ),
            letting_go: self.letting_go.as_ref(),
            deleting: self.deleting.as_ref(),
            refreshing: self.refreshing.as_ref(),
            taken_off: self.taken_off.as_ref(),
            coop: self.coop.as_ref(),
            special_panels: self.special_panels.as_ref(),
            inline_special_panel: None,
            in_both_ways: true,
            ..*interpreter
        }
    }

    /// Another copy of what these cells hold.
    pub(crate) fn copy(&self) -> Self {
        Self {
            asked: copy(self.asked.as_ref()),
            deleted: copy(self.deleted.as_ref()),
            sounds: copy(self.sounds.as_ref()),
            ui_effects: copy(self.ui_effects.as_ref()),
            masks_rebuilt: self.masks_rebuilt.as_ref().map(|c| Cell::new(c.get())),
            menu: copy(self.menu.as_ref()),
            action: copy(self.action.as_ref()),
            letting_go: copy(self.letting_go.as_ref()),
            deleting: copy(self.deleting.as_ref()),
            refreshing: copy(self.refreshing.as_ref()),
            taken_off: copy(self.taken_off.as_ref()),
            coop: copy(self.coop.as_ref()),
            special_panels: copy(self.special_panels.as_ref()),
        }
    }

    /// Whether two runs left the same in every cell, floats bit for bit.
    pub(crate) fn same(&self, other: &Self) -> bool {
        alike(&self.asked, &other.asked, PartialEq::eq)
            && alike(&self.deleted, &other.deleted, PartialEq::eq)
            && alike(&self.sounds, &other.sounds, PartialEq::eq)
            && alike(&self.ui_effects, &other.ui_effects, PartialEq::eq)
            && self.masks_rebuilt.as_ref().map(Cell::get)
                == other.masks_rebuilt.as_ref().map(Cell::get)
            && alike(&self.menu, &other.menu, same_checking)
            && alike(&self.action, &other.action, |a, b| bits(a) == bits(b))
            && alike(&self.letting_go, &other.letting_go, PartialEq::eq)
            && alike(&self.deleting, &other.deleting, PartialEq::eq)
            && self.same_refresh(other)
            && alike(&self.taken_off, &other.taken_off, PartialEq::eq)
            && alike(&self.coop, &other.coop, PartialEq::eq)
            && alike(&self.special_panels, &other.special_panels, PartialEq::eq)
    }

    /// Whether these cells hold the same refresh as `other`'s.
    pub(crate) fn same_refresh(&self, other: &Self) -> bool {
        alike(&self.refreshing, &other.refreshing, same_refreshing)
    }

    /// Keeps the run these cells and `lot` hold: puts `lot` in `world`'s
    /// place, and what these cells hold into `interpreter`'s, but for the
    /// refresh pending, which the caller keeps. Every cell is borrowed
    /// first, so that one held elsewhere panics before anything is kept.
    pub(crate) fn keep(self, interpreter: &Interpreter, world: &mut World, lot: World) {
        let asked = take(interpreter.asked);
        let deleted = take(interpreter.deleted);
        let sounds = take(interpreter.sounds);
        let ui_effects = take(interpreter.ui_effects);
        let menu = take(interpreter.menu);
        let action = take(interpreter.running_at_once.map(|a| a.action));
        let letting_go = take(interpreter.letting_go);
        let deleting = take(interpreter.deleting);
        let taken_off = take(interpreter.taken_off);
        let coop = take(interpreter.coop);
        let special_panels = take(interpreter.special_panels);
        *world = lot;
        put(asked, self.asked);
        put(deleted, self.deleted);
        put(sounds, self.sounds);
        put(ui_effects, self.ui_effects);
        if let (Some(held), Some(copy)) = (interpreter.masks_rebuilt, self.masks_rebuilt) {
            held.set(copy.get());
        }
        put(menu, self.menu);
        put(action, self.action);
        put(letting_go, self.letting_go);
        put(deleting, self.deleting);
        put(taken_off, self.taken_off);
        put(coop, self.coop);
        put(special_panels, self.special_panels);
    }
}

/// Cheap copy-on-write snapshot of a [`World`] for repeated [`same_lot`]
/// comparisons without encoding the reference world to a heap buffer.
pub(crate) struct LotSnapshot {
    world: World,
}

impl PartialEq for LotSnapshot {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        same_lot(&self.world, &other.world)
    }
}

impl Eq for LotSnapshot {}

impl LotSnapshot {
    #[inline]
    pub(crate) fn of(w: &World) -> Self {
        Self { world: w.clone() }
    }
}

#[inline]
fn same_f32_slice(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

#[inline]
fn same_interaction_bits(a: &Interaction, b: &Interaction) -> bool {
    a.join == b.join
        && a.uid == b.uid
        && a.caller == b.caller
        && a.target == b.target
        && a.icon == b.icon
        && a.table_index == b.table_index
        && a.args == b.args
        && a.priority == b.priority
        && a.action == b.action
        && a.attenuation.to_bits() == b.attenuation.to_bits()
        && a.flags == b.flags
}

fn same_frame(
    a: &sims_formats::saved_objects::Frame,
    b: &sims_formats::saved_objects::Frame,
) -> bool {
    a.stack_object == b.stack_object
        && a.tree == b.tree
        && a.node == b.node
        && a.parameters == b.parameters
        && a.locals == b.locals
        && a.primitive_state == b.primitive_state
        && a.code_owner_type == b.code_owner_type
}

fn same_person(
    a: &sims_formats::saved_objects::Person,
    b: &sims_formats::saved_objects::Person,
) -> bool {
    a.animation_events == b.animation_events
        && a.engaged == b.engaged
        && a.appearance == b.appearance
        && a.accessories.len() == b.accessories.len()
        && a.accessories
            .iter()
            .zip(&b.accessories)
            .all(|(x, y)| x.name == y.name && x.binding == y.binding)
        && a.animations == b.animations
        && a.routing_state == b.routing_state
        && same_f32_slice(&a.floats, &b.floats)
        && same_f32_slice(&a.previous_motives, &b.previous_motives)
        && same_f32_slice(&a.motives, &b.motives)
        && a.data == b.data
        && a.routing_frames == b.routing_frames
        && same_interaction_bits(&a.active, &b.active)
        && same_interaction_bits(&a.last, &b.last)
        && a.queue.len() == b.queue.len()
        && a.queue
            .iter()
            .zip(&b.queue)
            .all(|(x, y)| same_interaction_bits(x, y))
        && a.object_uses.len() == b.object_uses.len()
        && a.object_uses.iter().zip(&b.object_uses).all(|(x, y)| {
            x.target == y.target && x.stack_depth == y.stack_depth && x.unknown == y.unknown
        })
        && a.motive_deltas.len() == b.motive_deltas.len()
        && a.motive_deltas.iter().zip(&b.motive_deltas).all(|(x, y)| {
            x.motive == y.motive
                && x.delta.to_bits() == y.delta.to_bits()
                && x.stop_at.to_bits() == y.stop_at.to_bits()
        })
}

fn same_object(a: &sims_sim::Object, b: &sims_sim::Object) -> bool {
    a.id == b.id
        && a.guid == b.guid
        && a.footprint == b.footprint
        && a.position == b.position
        && a.unknown_short == b.unknown_short
        && a.attributes == b.attributes
        && a.temps == b.temps
        && a.data == b.data
        && a.obsolete_word == b.obsolete_word
        && a.stack_capacity_bytes == b.stack_capacity_bytes
        && a.stack.len() == b.stack.len()
        && a.stack.iter().zip(&b.stack).all(|(x, y)| same_frame(x, y))
        && a.relationship_flag == b.relationship_flag
        && a.relationships.len() == b.relationships.len()
        && a.relationships
            .iter()
            .zip(&b.relationships)
            .all(|(x, y)| x.present == y.present && x.target == y.target && x.values == y.values)
        && a.slots.len() == b.slots.len()
        && a.slots
            .iter()
            .zip(&b.slots)
            .all(|(x, y)| x.unknown == y.unknown && x.object == y.object)
        && a.dynamic_sprites == b.dynamic_sprites
        && match (&a.multipart, &b.multipart) {
            (Some(x), Some(y)) => {
                x.offset == y.offset && x.group == y.group && x.parent == y.parent
            }
            (None, None) => true,
            _ => false,
        }
        && match (&a.portal, &b.portal) {
            (Some(x), Some(y)) => same_f32_slice(x, y),
            (None, None) => true,
            _ => false,
        }
        && match (&a.person, &b.person) {
            (Some(x), Some(y)) => same_person(x, y),
            (None, None) => true,
            _ => false,
        }
        && a.registry_word == b.registry_word
        && a.last_result == b.last_result
        && a.removal_skips_resets == b.removal_skips_resets
        && a.selected == b.selected
}

fn same_budget(a: &sims_sim::Budget, b: &sims_sim::Budget) -> bool {
    a.funds == b.funds
        && a.pending_funds == b.pending_funds
        && a.lot_value == b.lot_value
        && a.objects_value == b.objects_value
        && a.architecture_value == b.architecture_value
        && a.days == b.days
}

fn same_array(a: &sims_formats::lot::Array, b: &sims_formats::lot::Array) -> bool {
    a.width == b.width
        && a.height == b.height
        && a.stride == b.stride
        && a.encoding == b.encoding
        && a.data == b.data
}

/// Whether two lots are the same: every encoded field bit-for-bit (using
/// `Arc::ptr_eq` CoW short-circuits and `f32::to_bits()`), each object's
/// `FrameStorage::restarts`, and every unencoded auxiliary table the
/// interpreter reads.
pub(crate) fn same_lot(a: &World, b: &World) -> bool {
    a.ticks == b.ticks
        && a.scheduler_cursor == b.scheduler_cursor
        && a.lot_rng == b.lot_rng
        && a.global_rng == b.global_rng
        && a.crt_rng == b.crt_rng
        && a.size == b.size
        && a.event_owner == b.event_owner
        && a.indicators == b.indicators
        && a.action_serial == b.action_serial
        && a.usage == b.usage
        && a.neighborhood_data == b.neighborhood_data
        && a.advertisement_record == b.advertisement_record
        && a.magic_history == b.magic_history
        && same_budget(&a.budget, &b.budget)
        && a.globals == b.globals
        && a.order == b.order
        && a.people == b.people
        && a.removals == b.removals
        && a.fires == b.fires
        && a.types == b.types
        && a.type_attributes == b.type_attributes
        && a.instances == b.instances
        && a.changed_types == b.changed_types
        && a.next_parts == b.next_parts
        && a.skeleton_files == b.skeleton_files
        && a.rooms == b.rooms
        && a.dialogs == b.dialogs
        && a.person_ticks == b.person_ticks
        && a.animators == b.animators
        && a.routing == b.routing
        && a.families == b.families
        && a.headlines == b.headlines
        && a.base_layers == b.base_layers
        && a.translated_bones == b.translated_bones
        && a.held_poses == b.held_poses
        && a.stops_asked == b.stops_asked
        && a.characters == b.characters
        && a.portraits == b.portraits
        && a.dirty_portraits == b.dirty_portraits
        && a.saved_portraits == b.saved_portraits
        && a.generated_portrait_2002 == b.generated_portrait_2002
        && a.made_portrait_head == b.made_portrait_head
        && a.ambiguous_portraits == b.ambiguous_portraits
        && a.loaded_indexed_skeletons == b.loaded_indexed_skeletons
        && a.loaded_indexed_suits == b.loaded_indexed_suits
        && a.loaded_indexed_suit_bones == b.loaded_indexed_suit_bones
        && a.made_outfits == b.made_outfits
        && a.person_outfits == b.person_outfits
        && a.scratch_ptrs == b.scratch_ptrs
        && a.runtime_added_suits == b.runtime_added_suits
        && a.suit_scan_cache == b.suit_scan_cache
        && a.neighbors.ptr_eq_or(&b.neighbors, PartialEq::eq)
        && a.arrays.ptr_eq_or(&b.arrays, same_array)
        && a.frame_storage.ptr_eq_or(&b.frame_storage, PartialEq::eq)
        && a.objects.ptr_eq_or(&b.objects, same_object)
}

impl<'a> Machine<'a, '_> {
    /// This machine on `world` and `cells` in place of its own.
    fn apart<'b>(&self, world: &'b mut World, cells: &'b Cells) -> Machine<'b, 'b>
    where
        'a: 'b,
    {
        let interpreter = cells.on(&self.interpreter());
        let mut machine = interpreter.machine(world, self.me, self.base, self.synchronous);
        machine.checking = self.checking;
        machine
    }

    /// Runs the current instruction, primitive `opcode`, both ways, with
    /// the animator refresh `refreshing` holds pending: see the module's
    /// documentation. Each run has the refresh taken out of its copy of
    /// `refreshing`, so neither refuses to read the animator. A step that
    /// leaves another refresh pending fails: it took a person off the lot,
    /// itself (a refresh pending in its copy of `refreshing`) or another
    /// (an entry added to or changed in its copy of `taken_off`).
    pub(crate) fn step_both_ways(
        &mut self,
        refreshing: &RefCell<Refreshing>,
        opcode: u16,
    ) -> Result<Status, Trap> {
        let (person, pending) = {
            let r = refreshing.borrow();
            (r.person, r.pending.clone().expect("a refresh pending"))
        };
        let differ = |what: &str| {
            Trap::Unsupported(format!(
                "{} (opcode {opcode}) after person {person} left the lot and before its \
                 animator's tick {what} whether or not the animator's refresh at the take-off \
                 (0x58da12) was made, which depends on whether the house view drew the person \
                 (animator byte +0x109, not kept)",
                crate::primitives::primitive_name(opcode),
            ))
        };

        // The cells as the step starts, with the refresh taken out.
        let start = Cells::of(&self.interpreter());
        if let Some(r) = &start.refreshing {
            let mut r = r.borrow_mut();
            r.pending = None;
            r.both_ways = true;
        }

        let mut left = self.world.clone();
        let left_cells = start.copy();
        let result = self.apart(&mut left, &left_cells).step();
        let mut drawn = self.world.clone();
        pending.refreshed.restore(&mut drawn, person);
        let drawn_cells = start.copy();
        let drawn_result = self.apart(&mut drawn, &drawn_cells).step();

        let another = |cells: &Cells| {
            cells
                .refreshing
                .as_ref()
                .is_some_and(|r| r.borrow().pending.is_some())
                || !alike(&cells.taken_off, &start.taken_off, PartialEq::eq)
        };
        if another(&left_cells) || another(&drawn_cells) {
            return Err(differ("takes a person off the lot"));
        }
        if result != drawn_result {
            return Err(differ("gives another result"));
        }
        let kept_left = Kept::of(&left, person);
        let kept_refreshed = Kept::of(&drawn, person);
        kept_left.restore(&mut drawn, person);
        if !left_cells.same(&drawn_cells) || !same_lot(&left, &drawn) {
            return Err(differ("leaves another lot"));
        }

        let mut refreshing = refreshing.borrow_mut();
        left_cells.keep(&self.interpreter(), self.world, left);
        refreshing.pending = Some(Pending {
            left: kept_left,
            refreshed: kept_refreshed,
        });
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_compared_bit_for_bit() {
        // `==` calls -0.0 and 0.0 alike and two NaNs different, and
        // `Debug` calls every NaN alike; two runs reach the same state
        // only when the bits are the same.
        let action = |bits: u32| Interaction {
            join: 0,
            uid: 0,
            caller: 0,
            target: 0,
            icon: 0,
            table_index: 0,
            args: [0; 4],
            priority: 0,
            action: 0,
            attenuation: f32::from_bits(bits),
            flags: 0,
        };
        let cells = |action_bits: u32, menu_bits: u32| Cells {
            asked: None,
            deleted: None,
            sounds: None,
            ui_effects: None,
            masks_rebuilt: None,
            menu: Some(RefCell::new(Checking {
                action: action(menu_bits),
                added: vec![],
            })),
            action: Some(RefCell::new(action(action_bits))),
            letting_go: None,
            deleting: None,
            refreshing: None,
            taken_off: None,
            coop: None,
            special_panels: None,
        };
        let (nan, other_nan) = (0x7fc0_0001, 0x7fc0_0002);
        assert!(!cells(0, 0).same(&cells(0x8000_0000, 0)), "0.0 and -0.0");
        assert!(cells(nan, nan).same(&cells(nan, nan)), "one NaN");
        assert!(!cells(nan, nan).same(&cells(other_nan, nan)), "the action");
        assert!(
            !cells(nan, nan).same(&cells(nan, other_nan)),
            "the menu's action"
        );
    }

    #[test]
    fn a_cell_held_elsewhere_stops_the_keep_before_anything_is_kept() {
        let program = crate::program::Program {
            global: 0,
            libraries: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            careers: None,
            career_texts_end: false,
            labels_hold_no_token: false,
            zoning: BTreeMap::new(),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: Default::default(),
        };
        let catalog = sims_content::Catalog {
            global: 0,
            objects: vec![],
            by_guid: Default::default(),
            semi_globals: Default::default(),
            diagnostics: vec![],
        };
        let asked = RefCell::new(vec![]);
        let sounds = RefCell::new(vec![]);
        let masks_rebuilt = Cell::new(0);
        let interpreter = Interpreter {
            asked: Some(&asked),
            sounds: Some(&sounds),
            masks_rebuilt: Some(&masks_rebuilt),
            ..Interpreter::new(&program, &catalog, crate::Stream::Lot)
        };
        let mut cells = Cells::of(&interpreter);
        cells.asked = Some(RefCell::new(vec![Asked {
            object: 1,
            function: 9,
        }]));
        cells.masks_rebuilt = Some(Cell::new(3));
        let mut world = World::empty();
        let mut lot = World::empty();
        lot.globals.push(7);
        let before = world.encode();
        let held = sounds.borrow();
        let kept = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cells.keep(&interpreter, &mut world, lot)
        }));
        drop(held);
        assert!(kept.is_err());
        assert_eq!(world.encode(), before, "the lot was not kept");
        assert!(asked.borrow().is_empty(), "no cell was kept");
        assert_eq!(masks_rebuilt.get(), 0, "masks_rebuilt was not kept");
    }

    #[test]
    fn masks_rebuilt_is_isolated_compared_and_kept() {
        let program = crate::program::Program {
            global: 0,
            libraries: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            careers: None,
            career_texts_end: false,
            labels_hold_no_token: false,
            zoning: BTreeMap::new(),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: Default::default(),
        };
        let catalog = sims_content::Catalog {
            global: 0,
            objects: vec![],
            by_guid: Default::default(),
            semi_globals: Default::default(),
            diagnostics: vec![],
        };
        let masks_rebuilt = Cell::new(1);
        let interpreter = Interpreter {
            masks_rebuilt: Some(&masks_rebuilt),
            ..Interpreter::new(&program, &catalog, crate::Stream::Lot)
        };
        let left = Cells::of(&interpreter);
        let right = left.copy();
        assert!(left.same(&right));
        // Mutating the isolated copy leaves the original and the sibling untouched.
        left.on(&interpreter).masks_rebuilt.unwrap().set(3);
        assert_eq!(masks_rebuilt.get(), 1);
        assert_eq!(right.on(&interpreter).masks_rebuilt.unwrap().get(), 1);
        assert!(!left.same(&right));
        right.on(&interpreter).masks_rebuilt.unwrap().set(3);
        assert!(left.same(&right));
        // A cell set differs from none.
        let none = Cells::of(&Interpreter::new(&program, &catalog, crate::Stream::Lot));
        assert!(!left.same(&none));
        // Keeping writes the isolated bitmask back to the interpreter.
        let mut world = World::empty();
        left.keep(&interpreter, &mut world, World::empty());
        assert_eq!(masks_rebuilt.get(), 3);
    }

    #[test]
    fn lots_that_differ_only_in_restarts_are_not_the_same() {
        let mut a = World::empty();
        a.frame_storage.insert(7, Default::default());
        let mut b = a.clone();
        assert!(same_lot(&a, &b));
        b.frame_storage.get_mut(&7).unwrap().restarts = 1;
        assert_eq!(a.encode(), b.encode(), "restarts are not encoded");
        assert!(!same_lot(&a, &b));
        a.frame_storage.get_mut(&7).unwrap().restarts = 1;
        assert!(same_lot(&a, &b));
    }

    #[test]
    fn lots_that_differ_only_in_portrait_flags_are_not_the_same() {
        let a = World::empty();
        let mut b = a.clone();
        b.dirty_portraits.insert(0x1234_5678);
        assert_ne!(a.encode(), b.encode(), "dirty_portraits is encoded");
        assert!(!same_lot(&a, &b));

        let mut c = a.clone();
        c.generated_portrait_2002
            .insert(sims_sim::PortraitFile::Resource(7));
        assert_eq!(
            a.encode(),
            c.encode(),
            "generated_portrait_2002 is not encoded"
        );
        assert!(!same_lot(&a, &c));

        let mut d = a.clone();
        d.made_portrait_head.insert(0x1234_5678, Some(true));
        assert_eq!(a.encode(), d.encode(), "made_portrait_head is not encoded");
        assert!(!same_lot(&a, &d));

        let mut e = a.clone();
        e.ambiguous_portraits.insert(0x1234_5678);
        assert_eq!(a.encode(), e.encode(), "ambiguous_portraits is not encoded");
        assert!(!same_lot(&a, &e));
    }

    #[test]
    fn inline_reference_book_refuses_silently_skipping_under_both_ways() {
        let mut behaviors = BTreeMap::new();
        behaviors.insert(
            4096,
            sims_formats::behavior::Behavior {
                format: 0x8002,
                kind: 0,
                args: 0,
                locals: 0,
                revision: 0,
                instructions: vec![sims_formats::behavior::Instruction {
                    opcode: 36,
                    on_true: 0xfe,
                    on_false: 0xff,
                    operands: [0, 0, 0, 0, 0, 12, 0, 0],
                }],
            },
        );
        let mut libraries = BTreeMap::new();
        libraries.insert(
            0,
            crate::Library {
                behaviors,
                ..crate::Library::default()
            },
        );
        let program = crate::Program {
            global: 0,
            libraries,
            semi_globals: BTreeMap::new(),
            careers: None,
            career_texts_end: false,
            labels_hold_no_token: false,
            zoning: BTreeMap::new(),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: Default::default(),
        };
        let catalog = sims_content::Catalog {
            global: 0,
            objects: vec![sims_content::Object {
                guid: 0x2001,
                resource: 0,
                chunk: 1,
                kind: 4,
                master: 0,
            }],
            by_guid: [(0x2001, 0)].into_iter().collect(),
            semi_globals: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
        let mut world = World::empty();
        world.types.insert(1, 0x2001);
        world.objects.insert(
            1,
            sims_sim::Object {
                id: 1,
                guid: 0x2001,
                footprint: [0; 4],
                position: [0; 3],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: vec![0; 8],
                data: vec![0; 64],
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: vec![sims_formats::saved_objects::Frame {
                    tree: 4096,
                    node: 0,
                    stack_object: 0,
                    parameters: Vec::new(),
                    locals: Vec::new(),
                    primitive_state: 0,
                    code_owner_type: 1,
                }],
                relationship_flag: 0,
                relationships: Vec::new(),
                slots: Vec::new(),
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: None,
                person: None,
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            },
        );
        let interpreter = Interpreter::new(&program, &catalog, crate::Stream::Lot);
        let cells = Cells::of(&interpreter);
        let probe = cells.on(&interpreter);
        assert!(probe.in_both_ways);
        let err = probe
            .step(&mut world, 1)
            .expect_err("Spellbook under both_ways must trap Unsupported rather than skipping");
        assert!(matches!(err.trap, crate::Trap::Unsupported(_)), "{err:?}");
    }

    #[test]
    fn same_lot_cow_shortcut_matches_encode_on_floats_and_scratch_ptrs() {
        let mut a = World::empty();
        a.objects.insert(
            1,
            sims_sim::Object {
                id: 1,
                guid: 0x1000,
                footprint: [0; 4],
                position: [0; 3],
                unknown_short: 0,
                attributes: Vec::new(),
                temps: vec![0; 8],
                data: vec![0; 64],
                obsolete_word: 0,
                stack_capacity_bytes: 0,
                stack: Vec::new(),
                relationship_flag: 0,
                relationships: Vec::new(),
                slots: Vec::new(),
                dynamic_sprites: Vec::new(),
                multipart: None,
                portal: Some(vec![0.0, 1.5, -2.25, 3.0, 4.0, 5.0]),
                person: None,
                registry_word: None,
                last_result: false,
                removal_skips_resets: false,
                selected: false,
            },
        );
        // CoW clone shares all Arcs -> fast path returns true.
        let mut b = a.clone();
        assert!(same_lot(&a, &b));
        assert_eq!(a.encode(), b.encode());

        // -0.0 vs +0.0 differs in postcard wire bytes -> same_lot must return false.
        b.objects.get_mut(&1).unwrap().portal.as_mut().unwrap()[0] = -0.0;
        assert_ne!(a.encode(), b.encode());
        assert!(!same_lot(&a, &b));

        // Identical NaN bit patterns encode identically -> same_lot must return true.
        let nan1 = f32::from_bits(0x7fc0_0001);
        let nan2 = f32::from_bits(0x7fc0_0002);
        a.objects.get_mut(&1).unwrap().portal.as_mut().unwrap()[0] = nan1;
        b.objects.get_mut(&1).unwrap().portal.as_mut().unwrap()[0] = nan1;
        assert_eq!(a.encode(), b.encode());
        assert!(same_lot(&a, &b));

        // Distinct NaN payloads differ in postcard bytes -> same_lot must return false.
        b.objects.get_mut(&1).unwrap().portal.as_mut().unwrap()[0] = nan2;
        assert_ne!(a.encode(), b.encode());
        assert!(!same_lot(&a, &b));

        // ScratchPtrs difference is caught even when encoded World matches.
        let mut c = a.clone();
        c.scratch_ptrs.set(1, sims_sim::SCRATCH_PTR_SET);
        assert_eq!(a.encode(), c.encode());
        assert!(!same_lot(&a, &c));
    }
}
