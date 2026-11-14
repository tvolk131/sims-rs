//! Routing: how a person finds its way to an object's slot.
//!
//! Go To Routing Slot ([`crate::primitives`], opcode 45) makes a route
//! context for the slot ([`sims_sim::route::Context`], kept on the person
//! at +0xaf0), finds where the person could stand to use it
//! ([`slot`]), plans a path there ([`planner`]) and hands the path to the
//! person's animator, which walks it tick by tick.
//!
//! The planner searches a graph of free rectangles ([`regions`],
//! [`search`]) among the obstacles of the room's objects and of the room
//! itself ([`rooms`]).
//!
//! Ported from the original's code and checked against the capture of the
//! Goth house's first route (update 14). Only what that route reaches is
//! ported; every other branch fails loudly.

mod planner;
pub(crate) mod portals;
mod regions;
mod rooms;
mod search;
pub(crate) mod slot;
