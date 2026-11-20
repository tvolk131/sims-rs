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
//! ways ([`Machine::step_both_ways`]): once on the lot as it is, and once
//! on a copy with the refreshed animator. If the two runs give the same
//! result and leave the same lot, apart from that person's animator, and
//! ask, note and play the same things, it does not matter which the
//! original made: the lot goes on with the first run, and both animators
//! are kept apart as the step left them. Otherwise the step fails.

use crate::Trap;
use crate::interpreter::{AtOnce, Machine, Status};
use crate::placement::{Kept, Pending, Refreshing};
use std::cell::RefCell;

/// A copy of what one of a machine's shared cells holds, so a step can
/// run apart on it.
fn apart<T: Clone>(cell: Option<&RefCell<T>>) -> Option<RefCell<T>> {
    cell.map(|c| RefCell::new(c.borrow().clone()))
}

/// Whether two of those cells hold the same.
fn same<T: PartialEq>(a: Option<&RefCell<T>>, b: Option<&RefCell<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => *a.borrow() == *b.borrow(),
        (None, None) => true,
        _ => false,
    }
}

impl Machine<'_, '_> {
    /// Runs the current instruction, primitive `opcode`, both ways, with the animator refresh
    /// `refreshing` holds pending: see the module's documentation. The
    /// refresh is taken out of `refreshing` while the two runs are made,
    /// so neither refuses to read the animator, and put back with the two
    /// animators the step left. A step that leaves another refresh
    /// pending (it took a person off the lot) fails.
    pub(crate) fn step_both_ways(
        &mut self,
        refreshing: &RefCell<Refreshing>,
        opcode: u16,
    ) -> Result<Status, Trap> {
        let (person, pending) = {
            let mut r = refreshing.borrow_mut();
            let pending = r.pending.take().expect("a refresh pending");
            r.both_ways = true;
            (r.person, pending)
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

        let mut drawn = self.world.clone();
        pending.refreshed.restore(&mut drawn, person);
        let asked = apart(self.asked);
        let deleted = apart(self.deleted);
        let sounds = apart(self.sounds);
        let menu = apart(self.menu);
        let action = apart(self.running_at_once.map(|a| a.action));
        let letting_go = apart(self.letting_go);
        let deleting = apart(self.deleting);
        let taken_off = apart(self.taken_off);
        let drawn_refreshing = RefCell::new(Refreshing {
            person,
            pending: None,
            both_ways: true,
        });
        let drawn_result = Machine {
            program: self.program,
            catalog: self.catalog,
            world: &mut drawn,
            me: self.me,
            stream: self.stream,
            asked: asked.as_ref(),
            deleted: deleted.as_ref(),
            sounds: sounds.as_ref(),
            base: self.base,
            synchronous: self.synchronous,
            checking: self.checking,
            making_person: self.making_person,
            menu: menu.as_ref(),
            running_at_once: self
                .running_at_once
                .zip(action.as_ref())
                .map(|(a, action)| AtOnce {
                    person: a.person,
                    action,
                }),
            letting_go: letting_go.as_ref(),
            deleting: deleting.as_ref(),
            view_rotation: self.view_rotation,
            refreshing: Some(&drawn_refreshing),
            taken_off: taken_off.as_ref(),
        }
        .step();

        let result = self.step();

        let another =
            refreshing.borrow().pending.is_some() || drawn_refreshing.borrow().pending.is_some();
        let left = Kept::of(self.world, person);
        let refreshed = Kept::of(&drawn, person);
        {
            let mut r = refreshing.borrow_mut();
            r.pending = Some(Pending { left, refreshed });
            r.both_ways = false;
        }
        if another {
            return Err(differ("takes a person off the lot"));
        }
        if result != drawn_result {
            return Err(differ("gives another result"));
        }
        let left = Kept::of(self.world, person);
        left.restore(&mut drawn, person);
        let same_cells = same(self.asked, asked.as_ref())
            && same(self.deleted, deleted.as_ref())
            && same(self.sounds, sounds.as_ref())
            && same(self.menu, menu.as_ref())
            && same(self.running_at_once.map(|a| a.action), action.as_ref())
            && same(self.letting_go, letting_go.as_ref())
            && same(self.deleting, deleting.as_ref())
            && same(self.taken_off, taken_off.as_ref());
        if !same_cells || drawn.encode() != self.world.encode() {
            return Err(differ("leaves another lot"));
        }
        result
    }
}
