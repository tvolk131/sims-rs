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
//! and both animators are kept apart as the step left them. Otherwise
//! the step fails, and the lot and the cells are as they were before it.

use crate::Trap;
use crate::interpreter::{Asked, AtOnce, Interpreter, Machine, Status};
use crate::orders::Checking;
use crate::placement::{Kept, Pending, Refreshing};
use crate::sounds::SoundEvent;
use sims_formats::saved_objects::Interaction;
use sims_sim::World;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

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

/// What a cell holds, written out, to compare two runs' cells. The
/// written form tells -0.0 from 0.0 and calls two NaNs alike, where `==`
/// on the floats in an action does neither.
fn shown<T: Debug>(cell: &Option<RefCell<T>>) -> Option<String> {
    cell.as_ref().map(|c| format!("{:?}", c.borrow()))
}

/// Puts what `copy` holds into `cell`.
fn put<T>(cell: Option<&RefCell<T>>, copy: Option<RefCell<T>>) {
    if let (Some(cell), Some(copy)) = (cell, copy) {
        *cell.borrow_mut() = copy.into_inner();
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

    /// Whether two runs left the same in every cell.
    pub(crate) fn same(&self, other: &Self) -> bool {
        shown(&self.asked) == shown(&other.asked)
            && shown(&self.deleted) == shown(&other.deleted)
            && shown(&self.sounds) == shown(&other.sounds)
            && shown(&self.menu) == shown(&other.menu)
            && shown(&self.action) == shown(&other.action)
            && shown(&self.letting_go) == shown(&other.letting_go)
            && shown(&self.deleting) == shown(&other.deleting)
            && shown(&self.refreshing) == shown(&other.refreshing)
            && shown(&self.taken_off) == shown(&other.taken_off)
    }

    /// Whether these cells hold the same refresh as `other`'s.
    pub(crate) fn same_refresh(&self, other: &Self) -> bool {
        shown(&self.refreshing) == shown(&other.refreshing)
    }

    /// Puts what these cells hold into `interpreter`'s, but for the
    /// refresh pending, which the caller keeps.
    pub(crate) fn keep(self, interpreter: &Interpreter) {
        put(interpreter.asked, self.asked);
        put(interpreter.deleted, self.deleted);
        put(interpreter.sounds, self.sounds);
        put(interpreter.menu, self.menu);
        put(interpreter.running_at_once.map(|a| a.action), self.action);
        put(interpreter.letting_go, self.letting_go);
        put(interpreter.deleting, self.deleting);
        put(interpreter.taken_off, self.taken_off);
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
                || shown(&cells.taken_off) != shown(&start.taken_off)
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

        *self.world = left;
        left_cells.keep(&self.interpreter());
        refreshing.borrow_mut().pending = Some(Pending {
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
    fn cells_are_compared_as_written_out() {
        // `==` calls -0.0 and 0.0 alike and two NaNs different; the
        // written form does the opposite, which is what a run reaching
        // the same state means.
        let cell = |v: f32| Some(RefCell::new(vec![v]));
        assert_ne!(shown(&cell(0.0)), shown(&cell(-0.0)));
        assert_eq!(shown(&cell(f32::NAN)), shown(&cell(f32::NAN)));
        assert_eq!(shown::<Vec<f32>>(&None), None);
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
