# Objects holding a person draw after the people

*Decided 2026-11-20.*

A frame with people in it now draws in three passes:

1. `sims_render::draw_lot`: the lot, without the objects that hold a person.
2. The people (`People::draw_on_lot`).
3. `sims_render::draw_held`: those objects, each pixel tested against the depth the people wrote.

`sims_render::draw` still draws every object in one pass, for pictures with no people. sims-game's frame uses the three passes.

**Which objects.** `sims_render::holds_person` climbs from each person to their outermost holder. The set is that holder plus everything reached through its slots, at any depth; the people themselves are not in it.

- Holders are found by the link words, as 0x4325b5 finds them (the simulation ports it as `placement::capture`). Data word 14 is the slot, counted from 1: −1 for none, 0 for an object that follows another on its tile. Word 26 is the object it is in, or follows.
- An object with slot word 0 is held by whatever the first object of its run is in.
- The tree is walked down through each holder's slot list, as 0x434657 recurses (0x4347bc). The list's first entry is skipped: it links to the next object on the tile, not to a held one. 0x434657 starts at the second entry (offset 0x24 at 0x434681), and so does the list builder's scan at 0x4aad77.
- Data word 2 (the container word the drawing uses for slot offsets) is not consulted.
- A person held by nothing is the root of their own tree, so what they hold is in the set too.
- Holders that hold each other form one tree, named by the lowest ID in their loop.

**What the original does.** Its per-frame list builder 0x4aacb9 puts an object on the "moving" list (view +0x1c8) while its state word +0x15c is 1. Only objects with no holder go on the list. A held object is turned away at 0x4aae97, because 0x43265e finds it a holder, and is drawn with its holder's contents.

A moving object leaves that state by 0x434657(0, 0) when two things hold:

- it has gone unchanged for longer than its class's time-out (vtable +0x18, checked against the clock at 0x6e54e0 minus the object's +0x158);
- it has been drawn since it was marked: +0x154 has left 0x7fffffff, and the draw sets it at 0x4378ef.

The lot's own pass draws only objects that are not moving (+0x15c tested at 0x4aea06).

- A person's animation code calls 0x434101 on the person, each call on its own condition. The calls are at 0x5859fa, 0x585b80, 0x58da79, 0x58f873 and 0x591dfb.
- 0x434101 climbs the holders (0x4325b5) to the outermost one. It then calls 0x434657(1, …), which sets the holder moving, stamps the time, and recurses into everything in its slots (0x4347bc).
- A person's own time-out is 0x7fffffff (0x4226b6).
- With the lot paused at 7:12 AM, the live original still drew the bench Mortimer sat on in its moving pass every frame; it was the only object it drew there.

After the lot's picture, the frame draws the people on list +0x1d4 (0x4a9458), then the moving objects (0x4aa1d7, called from 0x4a94ab).

This also explains the depth dumps of the original's lot picture taken for the compositing work (#125): they held no people and no in-use bench.

**Checked against.** A screenshot of the original showing 7:12 AM: near zoom, rotation 2, walls down, camera origin (591, −2018). In it, Mortimer reads on the porch bench. Our frame, rendered at tick 1260, aligns with it at offset (0, 0). The colour comparison below is approximate, with a tolerance per channel.

- The backrest's soft top edge blends over his coat: (32, 16, 8) and (65, 32, 8) at (385, 311) and (405, 305).
- Before this change, ours blended that edge over the floor: (196, 159, 110) and (136, 90, 45). Now it shows (35, 20, 12) and (68, 34, 11), within 12 of the original in each channel after 16-bit quantizing.
- Along his back, 25 edge pixels were off by more than 24 in some channel before; none are now. About five new ones are off (below).
- The live original's slot lists at that moment agree with `holds_person`: bench 174 holds Mortimer (21), bench 173 holds person 16, and Mortimer holds the plumb bob (383). Person 18's first entry names object 67, but that entry is the tile link, so 67 is not held.
- The test `a_bench_in_use_draws_after_the_sim_on_it` (sims-people, needs `SIMS_INSTALL`) checks both edge pixels and the held set.
- `the_frame_draws_a_bench_in_use_after_the_sim_on_it` (sims-game) checks the same pixels in the window's own frame.

**Not ported:**

- *Other moving objects.* An object whose state changed recently also stays on the list until its time-out. Each class computes its own time-out (0x43458c for plain objects). That needs the time of each object's last change, which the render state doesn't keep. Such an object draws in the lot pass here, so a person who overlaps it within its time-out is drawn over it, not under it.
- *Flag 0xca bit 1.* Mask 0x01 at +0xca of a definition (+0x118) makes objects moving, subject to the list builder's other checks. It counts when an object's own definition has it (0x4aae05), or when the definition of an object in its slots does (0x4aad77 to 0x4aadff, and 0x4346cf). Objects 112 and 113 on the Goth lot have it; what that bit means is not read.
- *Order within the pass.* The original draws the moving objects in the order of its list. That list is built from its object list (0x440349), whose order is not read here. Ours keeps the lot pass's far-to-near order, then ID. Where two such objects overlap at equal depth, the one drawn later wins, which changes both the colour and the object a click finds. **Unverified.**
- *Upper floors.* A moving object one level above the camera's is drawn (0x4aa2a1 to 0x4aa2e2) only if it is a person with the word at +0x5dc set; an ordinary moving object there is left out. Ours draws held objects only on the levels the camera shows, which agrees for objects. The person case belongs to the people pass.
- *Cutaway.* The lot pass's cutaway test has a bypass on view byte +0x51 (0x4aea93) that the moving pass lacks. So a wall-hiding object can show in one pass and hide in the other. Ours applies the same cutaway to both, and +0x51 is not read.

**Left over at that camera.** About five pixels at the left end of the backrest's edge now blend over Mortimer's coat where the original blends over the floor. So our mesh reaches slightly further behind the backrest there than the original's does. That mesh is hidden everywhere else. Person 16, standing by the other bench, stands in a different pose from the screenshot's, with either draw order. A capture of the original at tick 1260 agrees with ours on that animator's position and on its foreground and carry phases. So what causes the gap stays open until a frame of the original is captured at the same tick.

Reopen this if a frame shows an object drawn over a person who is neither on it nor in its tree. The time-out path above is the likely cause.
