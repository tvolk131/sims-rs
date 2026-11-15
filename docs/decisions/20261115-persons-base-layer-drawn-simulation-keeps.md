# A person's base layer is drawn as the simulation keeps it

*Decided 2026-11-15.*

`sims_people` now poses a person's base layer from the simulation's
playback of it (`World::base_layers`, the layer in the animator's base
slot): its clip, its phase, its end mode and its weight. Before, the
simulation did not keep the layer, so the drawing stood in with the
saved person record's base clip (the standing loop, for each Goth)
repeating at the animator's clock, or a script's clip from when it
started. A seated person was posed on the standing loop under the
foreground clip.

Sampled every 50 ticks over the Goths' first 3,400 ticks (strict mode),
a person sits in 94 of 226 samples; the base clip is `a2o-sit-loop`
there, and `a2o-standing-loop` or `c2o-standing-loop` otherwise. The
child sits on the adult's sit loop too: that is the clip the posture's
tables name for him (`posture_clip`), which the drawing takes as it is.
In every one of those samples a foreground clip plays at full weight,
and those clips move every bone, so the drawn day hardly changes: at
ticks 500, 1100, 1300, 2900 and 3050 (medium zoom, the porch and the
living room) the people are the same pixel for pixel. The change counts
where a foreground clip is partial, fading or absent, and it removes a
stand-in.

**Checked** only in part. Against the original at medium zoom (Mortimer
and Alexander reading on the porch bench at 7:38), both sit as the
original's do, but they did before too: the reading clip drives every
bone. No screenshot of the original yet shows a seated person whose
foreground leaves bones to the base layer. The original's people are
drawn lighter than ours, and it shows the plumb bob over the selected
Sim, which is not drawn here; neither is about the base layer.

A caller that has no layers for a person (passes None) gets the
stand-in as before. A state kept before the simulation held the layer
(encodings before format 25) is refused when loaded, not migrated.
