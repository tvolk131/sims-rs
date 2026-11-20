# Objects holding a person draw after the people

*Decided 2026-11-20.*

A frame with people in it now draws in three passes:

1. `sims_render::draw_lot`: the lot, without the objects that hold a person.
2. The people (`People::draw_on_lot`).
3. `sims_render::draw_held`: those objects, each pixel tested against the depth the people wrote.

`sims_render::draw` still draws every object in one pass, for pictures with no people. sims-game's frame uses the three passes.

**Which objects.** `sims_render::holds_person` finds them by walking each object's holder (data words 2 and 3) up to the outermost holder. If a person is anywhere in that holder's tree, the holder and everything in its tree, at any depth, are in the set; the people themselves are not. A person held by nothing is the root of their own tree, so what they hold is in the set too. Holders that hold each other form one tree, named by the lowest ID in their loop.

**What the original does.** Its per-frame list builder 0x4aacb9 puts an object on the "moving" list (view +0x1c8) while its state word +0x15c is 1. A drawn object leaves that state once it has gone unchanged for longer than its class's time-out (vtable +0x18): it is checked against the clock at 0x6e54e0 minus the object's +0x158, then dropped by 0x434657(0, 0).

- A person's animation calls 0x434101 on the person. Those calls are at 0x5859fa, 0x585b80, 0x58da79, 0x58f873 and 0x591dfb.
- 0x434101 climbs the holders (0x4325b5) to the outermost one. It then calls 0x434657(1, …), which sets the holder moving, stamps the time, and recurses into everything in its slots (0x4347bc).
- A person's own time-out is 0x7fffffff (0x4226b6). A person animates every tick, so their tree stays on the list for as long as they are in it.

After the lot's picture, the frame draws the people on list +0x1d4 (0x4a9458), then the moving objects (0x4aa1d7, called from 0x4a94ab). 0x4aa1d7 applies the same level and cutaway tests as the lot pass.

This also explains the depth dumps of the original's lot picture taken for the compositing work (#125): they held no people and no in-use bench.

**Checked against.** A screenshot of the original at 7:12 AM (tick 1260): near zoom, rotation 2, walls down, camera origin (591, −2018). In it, Mortimer reads on the porch bench, and our frame matches the screenshot to the pixel elsewhere.

- The backrest's soft top edge blends over his coat: (32, 16, 8) and (65, 32, 8) at (385, 311) and (405, 305).
- Before this change, ours blended that edge over the floor: (196, 159, 110) and (136, 90, 45). All 25 edge pixels along his back now match.
- The live original's slot table at that moment lists exactly what `holds_person` finds: bench 174 holding Mortimer (21), bench 173 holding person 16, and the plumb bob (383) in Mortimer's slot.
- The test `a_bench_in_use_draws_after_the_sim_on_it` (sims-people, needs `SIMS_INSTALL`) checks both edge pixels and the held set.

**Not ported:**

- *Other moving objects.* An object whose state changed recently also stays on the list until its time-out. Each class computes its own time-out (0x43458c for plain objects). That needs the time of each object's last change, which the render state doesn't keep. Such an object draws in the lot pass here, so a person who overlaps it within its time-out is drawn over it, not under it.
- *Flag 0xca bit 1.* An object that holds an object whose definition (+0x118) has bit 1 set at +0xca is always moving (0x4aad77 to 0x4aadff, and 0x4346cf). Objects 112 and 113 on the Goth lot have it; what that bit means is not read.
- *Order within the pass.* The original draws the moving objects in the order of its object list. Ours keeps the lot pass's far-to-near order. The two can differ only where two such objects' soft edges overlap. **Unverified.**
- *Upper floors.* 0x4aa2c1 draws a moving object one level above the camera's under a further condition (word +0x5dc). Ours draws the held objects on the levels the camera shows, from the lowest up.

**Left over at that camera.** About five pixels at the left end of the backrest's edge now blend over Mortimer's coat where the original blends over the floor. So our mesh reaches slightly further behind the backrest there than the original's does. That mesh is hidden everywhere else. Person 16, standing by the other bench, stands in a different pose from the original's; that is sim timing, not drawing.

Reopen this if a frame shows an object drawn over a person who is neither on it nor in its tree. The time-out path above is the likely cause.
