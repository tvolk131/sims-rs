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
use crate::sounds::SoundEvent;
use sims_formats::saved_objects::Interaction;
use sims_sim::World;
use std::cell::{RefCell, RefMut};
use std::collections::{BTreeMap, BTreeSet};

/// Copies of what each cell an interpreter shares with the rest of the
/// tick holds, so that a run can be made apart on them, compared with
/// another, and kept or dropped. Every `RefCell` an [`Interpreter`] (and
/// so a [`Machine`]) holds is here; the rest of either is values or
/// shared inputs that nothing writes (the program, the catalog).
pub(crate) struct Cells {
    asked: Option<RefCell<Vec<Asked>>>,
    deleted: Option<RefCell<Vec<u16>>>,
    sounds: Option<RefCell<Vec<SoundEvent>>>,
    menu: Option<RefCell<Checking>>,
    /// The action of `running_at_once`.
    action: Option<RefCell<Interaction>>,
    letting_go: Option<RefCell<BTreeSet<u16>>>,
    deleting: Option<RefCell<BTreeSet<u16>>>,
    refreshing: Option<RefCell<Refreshing>>,
    taken_off: Option<RefCell<BTreeMap<u16, Pending>>>,
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
            menu: copy(interpreter.menu),
            action: copy(interpreter.running_at_once.map(|a| a.action)),
            letting_go: copy(interpreter.letting_go),
            deleting: copy(interpreter.deleting),
            refreshing: copy(interpreter.refreshing),
            taken_off: copy(interpreter.taken_off),
        }
    }

    /// `interpreter` with these cells in place of its own.
    pub(crate) fn on<'b>(&'b self, interpreter: &Interpreter<'b>) -> Interpreter<'b> {
        Interpreter {
            asked: self.asked.as_ref(),
            deleted: self.deleted.as_ref(),
            sounds: self.sounds.as_ref(),
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
            ..*interpreter
        }
    }

    /// Another copy of what these cells hold.
    pub(crate) fn copy(&self) -> Self {
        Self {
            asked: copy(self.asked.as_ref()),
            deleted: copy(self.deleted.as_ref()),
            sounds: copy(self.sounds.as_ref()),
            menu: copy(self.menu.as_ref()),
            action: copy(self.action.as_ref()),
            letting_go: copy(self.letting_go.as_ref()),
            deleting: copy(self.deleting.as_ref()),
            refreshing: copy(self.refreshing.as_ref()),
            taken_off: copy(self.taken_off.as_ref()),
        }
    }

    /// Whether two runs left the same in every cell, floats bit for bit.
    pub(crate) fn same(&self, other: &Self) -> bool {
        alike(&self.asked, &other.asked, PartialEq::eq)
            && alike(&self.deleted, &other.deleted, PartialEq::eq)
            && alike(&self.sounds, &other.sounds, PartialEq::eq)
            && alike(&self.menu, &other.menu, same_checking)
            && alike(&self.action, &other.action, |a, b| bits(a) == bits(b))
            && alike(&self.letting_go, &other.letting_go, PartialEq::eq)
            && alike(&self.deleting, &other.deleting, PartialEq::eq)
            && self.same_refresh(other)
            && alike(&self.taken_off, &other.taken_off, PartialEq::eq)
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
        let menu = take(interpreter.menu);
        let action = take(interpreter.running_at_once.map(|a| a.action));
        let letting_go = take(interpreter.letting_go);
        let deleting = take(interpreter.deleting);
        let taken_off = take(interpreter.taken_off);
        *world = lot;
        put(asked, self.asked);
        put(deleted, self.deleted);
        put(sounds, self.sounds);
        put(menu, self.menu);
        put(action, self.action);
        put(letting_go, self.letting_go);
        put(deleting, self.deleting);
        put(taken_off, self.taken_off);
    }
}

/// Whether two lots are the same: their encodings, and each object's
/// count of its thread's restarts (`FrameStorage::restarts`), which the
/// encoding leaves out and the interpreter reads.
pub(crate) fn same_lot(a: &World, b: &World) -> bool {
    let restarts = |w: &World| {
        w.frame_storage
            .iter()
            .map(|(id, s)| (*id, s.restarts))
            .collect::<Vec<_>>()
    };
    a.encode() == b.encode() && restarts(a) == restarts(b)
}

impl<'a> Machine<'a, '_> {
    /// This machine on `world` and `cells` in place of its own.
    fn apart<'b>(&self, world: &'b mut World, cells: &'b Cells) -> Machine<'b, 'b>
    where
        'a: 'b,
    {
        let interpreter = cells.on(&self.interpreter());
        Machine {
            program: self.program,
            catalog: self.catalog,
            world,
            me: self.me,
            stream: self.stream,
            asked: interpreter.asked,
            deleted: interpreter.deleted,
            sounds: interpreter.sounds,
            base: self.base,
            synchronous: self.synchronous,
            checking: self.checking,
            making_person: self.making_person,
            menu: interpreter.menu,
            running_at_once: interpreter.running_at_once,
            letting_go: interpreter.letting_go,
            deleting: interpreter.deleting,
            view_rotation: self.view_rotation,
            view_centre: self.view_centre,
            refreshing: interpreter.refreshing,
            taken_off: interpreter.taken_off,
        }
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
            menu: Some(RefCell::new(Checking {
                action: action(menu_bits),
                added: vec![],
            })),
            action: Some(RefCell::new(action(action_bits))),
            letting_go: None,
            deleting: None,
            refreshing: None,
            taken_off: None,
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
        let interpreter = Interpreter {
            asked: Some(&asked),
            sounds: Some(&sounds),
            ..Interpreter::new(&program, &catalog, crate::Stream::Lot)
        };
        let mut cells = Cells::of(&interpreter);
        cells.asked = Some(RefCell::new(vec![Asked {
            object: 1,
            function: 9,
        }]));
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
}
