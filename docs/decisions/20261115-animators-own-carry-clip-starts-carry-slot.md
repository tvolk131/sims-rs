# The animator's own carry clip starts in the carry slot; a changed skin sorts its layers by priority

*Decided 2026-11-15.*

**Unverified.** No capture has a person carrying with the carry clip the
tick chooses. In the Goth captures (updates 1 to 1000) only person 21
has a carry layer, a script's (carry mode 6, from update 417), and the
captures hold no layer pages.

The animator's tick chooses the carry clip at 0x5903a3. A change of
choice clears the carry slot at once (virtual function 0x104,
0x592878). It no longer fails when a layer is there, since the tick's
own carry layer is one. A change to clip 0 starts the clip at once
(0x5903fc to 0x590421):

- It looks up the posture's clip 0x26 (virtual function 0xf4, 0x592625,
  which reads the appearance again when stale, then 0x59db2f).
- It starts that clip in the carry slot (virtual function 0x100,
  0x592726) with flag 0: forward, speed 1, end mode 0 (0x592823), so the
  clip stops at its end. 0x592726 sets end mode 0 for every slot. Only
  Animate Sim's carry path then sets 1 (0x58977c).
- It gives the layer priority 10 (0x59867f), which marks the skin
  changed.
- The carry mode stays 1, not 6, so each tick chooses again. Empty hands
  or a pickup clip (+0x1a4) choose -1, which removes the layer.
- Later updates are those of any carry layer: the skin's update, with
  timed events handled.

The skin's update sorts its layers by priority before it updates any of
them (0x5991fd, called first by 0x5992b3). The sort is a bubble sort,
lowest priority first, and runs only when the skin has changed (skin
+0x94). Layers of the same priority keep their order. Every change of
layer or priority marks the skin changed, so the layers always update by
priority. The 2026-11-14 carry entry kept only the insertion order
(0x599128). So a carry layer (10) now updates after a foreground layer
of raised priority (6), not before it.

`Animator::carry_first` stays the insertion order and is not written by
the sort. It decides only a tie. The original's list keeps the sorted
order instead. The two differ only when two layers had different
priorities at a sort and the same priority later, and nothing ported
does that: foreground layers take priority 1, 5 or 6, carry layers 10.
Keeping the field as it was leaves snapshots and the K=1000 pins
unchanged. Writing the sorted order into it changed the tick 417 digest,
where person 21's script carry layer plays with a foreground, though
nothing else differed. No format bump is needed.

Still failing loudly: a carry clip with no name (0x59db2f) or one no
file holds (0x59dc32). The original leaves the slot empty then and
still notes clip 0 as chosen.

**On Goth.** The snack order's step 820 (tick 89263) now starts
Mortimer's carry clip, `a2o-rarm-carry-loop`, at priority 10. The rest
of his tick stops on person word 1 (5, the money just spent), which the
person's tick hands to the interface (0x4a69aa). Strict mode on the Goth
house (1102 ticks) is unchanged. Reopen when a capture holds a carry
layer the tick chose, or the skin's layer list.
