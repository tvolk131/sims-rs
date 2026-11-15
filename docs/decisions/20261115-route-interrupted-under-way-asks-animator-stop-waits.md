# A route interrupted under way asks its animator to stop and waits

*Decided 2026-11-15.*

**Unverified** against a recording: no capture has a route interrupted
while its animator walks. Track 1 found it by cancelling the snack order
while Mortimer walks; a player order given while he walks on an action
of his own (priority 2, below the order's 50) reaches it the same way.

Go To Routing Slot's step 3 tests for an interruption before it reads
the animator's route (0x45b2b7), so step 11 (0x45b3c2) can find the
animator still under way (its route end, +0x188, 1 or -1). The stop
(0x589e2d) then stops nothing: it sets animator +0x284 to 1 and returns
false, and the step jumps to the run's common exit (0x45b9a0), which
keeps step 11, writes the context back and returns the run's result,
still the 2 it started with, so the thread yields. Steps 9 and 10 come
after a route result of 1 or 0, which an animator under way never gives,
so only step 11 reaches this; any other step fails loudly. The end of a
route (0x45b6ea) and Look Towards stop the same way, and now note the
request too.

The animator reads +0x284 in the walk's step, after the person is put on
the next tile (0x58eeeb): when it is set, the route end is still 1 and
the foreground clip may be interrupted (layer +0x68), a person whose
word 64 is 0 has the route end set to 3, blocked (0x58ef29 to 0x58f682),
and the update goes to its tail, as for a step the person does not
fit: the step ends false, so at the end of the path the position is not
copied to the destination and the state is not run again (track 3's
strict run against the original caught an earlier version that went on
as if the step had been taken). On the next tick step 11 stops the
animator, plays `end_route` and fails the route with result 4. A person
whose word 64 is set instead plays the stop clip (state 7, clip 0x13,
0x58f010), which is not ported and fails loudly. Starting a route clears
+0x284 (0x589d38).

- +0x284 is kept as `World::stop_requests`, a set of people, added at
  the end of the World (snapshot format 29, after main's 27 and 28) so
  that format 28 encodings stay a prefix. The animator's constructor sets +0x284 to 1 (0x5887b7),
  but only a walk after a route starts reads it, and the start clears it,
  so a new animator is left out of the set.
- Every pinned state (the Goth ticks 33, 1103, 2881, 3432, 3602, 3603,
  3912 and 9906, and the Newbies' 1802 and 4052) is the format 28 state
  plus an empty set: the strict runs never stop a route under way.
- Track 1's test of the cancelled snack order
  (`the_player_cancels_the_snack_queued_and_running`) now plays on: at
  step 182 the stop is asked for and the walk ends the animator's route
  blocked, at step 183 the route fails with result 4 and the snack ends,
  and play goes on without an error to step 400.
- The walk's tests of person word 64 use the value read when the
  animator's tick began (0x58dcaf, kept at [ebp-0x24]), not one read
  again after placement callbacks have run.
- Reopen if a recording interrupts a route under way and the original
  takes another tick, or none, to stop.

**The snack cancels again.** Track 1's sweep had cancels at steps 150
to 536, 543 to 590 and 821 to 829 stop here. Cancels at steps 150, 151,
200, 300, 500, 536, 543, 560, 590, 821, 822, 825 and 829, each run 400
steps on, now catch no error (a manual check, not a test). The one at
step 400 runs to step 729, where person 18's walk blends a layer
below weight 1 into what the bones held (0x597c4c, not ported).
The test of the cancel at step 825 now says what follows: the stop at
step 828, the route's result 4 at step 829, the snack put down at step
894 and Mortimer free at step 913.

    snack, stopped at the unported route stop below; since the route
    stop is ported, he stops the walk, carries the snack to (296, 456)
    and puts it down
    (`a_snack_cancelled_on_the_way_to_the_chair_stops_the_walk_and_is_put_down`).
