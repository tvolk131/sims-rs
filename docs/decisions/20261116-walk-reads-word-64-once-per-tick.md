# The walk reads person word 64 once, when the animator's tick begins

*Decided 2026-11-16.*

The animator's tick reads the person's word 64 (in a pool) once, as it
begins (0x58dcaf), and keeps it at [ebp-0x24]. Every test of it in the
walk reads that copy: the pace and the stride (0x58e836, 0x58e910,
0x58e931), the slow-down before the end (0x58ed35), and the stop asked
for while the route was under way (0x58ef25). A placement callback
that runs while the walk puts the person on their next tile (the
person's own placed tree, function 9) can change word 64, and the rest
of that tick does not see the change.

- The port reads it once in `Interpreter::animate` and passes it down
  (`dispatch`, `follow`, `end_of_path`, `head`, `walk`, `walk_step`).
  Before this, the stop asked under way (0x58ef2f) read it again after
  the put.
- The posture clip lookup (0x59dba7) is a separate function that reads
  word 64 itself, and still does.
- This is the part of #188 that #233 did not port: #233 ported the stop
  asked under way (0x589e2d, 0x58eeeb, 0x589d38) as `World::stops_asked`,
  and #188 is closed in favour of it.
