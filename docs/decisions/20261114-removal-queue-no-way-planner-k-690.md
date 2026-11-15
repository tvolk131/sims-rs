# The removal queue, no way in the planner, and K = 690

*Decided 2026-11-14.*

Remove Object Instance now queues as the original's removal queue does
(0x442d70), taking the object off through `take_off`. Only while the load
has not worked the rooms out (`room_at` answers that there are none) does
it take the narrower `leave_lot`, which the load was compared through:
the general way looks up the room of the off-lot place, which the load
cannot answer yet. The narrower way now also works the object's
footprint out again at the off-lot place, as `take_off`'s move does
(the factory's may differ: object 44, removed during the load, had
one); anything else it cannot match fails loudly. A fault in a callback the removal runs keeps
the object, tree and node it happened at (`Fault::nested`), as do Snap's
placement faults. Reopen if an object is removed during the load in a way
`leave_lot` refuses.

The first capture to 1000 updates stopped at 609 with a harness error the
spatial-order reader fix (#47) already addressed; a fifth capture with
that reader recorded the full state after updates 600 to 1000, and its
streams equal the fourth's to 608, so ticks 600 to 690 are compared with
it. K = 690.
