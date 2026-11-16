# An edge-on diagonal wall stands 300 nearer, anchored halfway along

*Decided 2026-11-15.*

At the bay's two ends the original shows the outside shingles; we showed
the inside wallpaper at the left end. It is not a side the original
picks. What the original shows there is mostly the straight bay walls'
side faces, which come out from behind the diagonal; the edge-on
diagonal (frame 3) is hidden behind them. We drew the diagonal too
near, so it covered them.

**The side (read, not ported).** Each run in the original stores two
patterns per entry: ascending runs [p2, p3] and descending runs
[p3, p2], slot 0 (0x53f6c1, 0x53f3e9). When the view rotates, the run is
turned (0x53d93a). It is reversed when its end sorts before its start
(0x44054c), which swaps every entry's sides, and the slot comes from the
table at 0x53db21. So the original's edge-on diagonal shows the side
facing view direction (−1, 1). Which lot side that is changes with the
rotation.

Ours shows `patterns[3]` for every edge-on diagonal at every rotation,
so at some rotations it is the other side. That is not changed here.
Where the original's diagonal does show (column x = 75 above), neither
side's texels match the original's. That is the texel offset already
recorded as unread, so the screenshots cannot check a side rule. Reopen
with that offset.

**The depth.** For wall kind 0x20 (edge-on), 0x4c7be2 stores −300.
The row loop (0x4c4907) adds it to every row's depth. Each row's depth
otherwise comes from 0x49a5d3, which is `Depth::stand`. Kind 0x10 (the
diagonal face-on, frame 2) and the straight walls get 0.

**The anchor.** With the −300 alone, the anchor has to be halfway
between the wall's two ends on screen, not at its lower end. At the
medium-zoom depth capture (rotation 0, origin [335, −560]) that gives
the original's depth exactly:

- in column x = 75, rows 180, 210, 240 and 270, the one column where the
  original's diagonal shows: 42795, 43012, 43229 and 43446;
- in the columns beside it at both bay ends, where the side walls now
  win as they do in the original.

The anchor itself was not read; it is fitted to these rows. At that
point the height is the mean of the two ends' heights. That is
**unverified**: the Goth bay stands on flat ground.

**Gains (exact colour, whole screen):**

| Camera | Before | After | Notes |
|---|---|---|---|
| Near zoom, rotation 0, [619, −1189] | 61.2% | 61.7% | both bay ends show the shingles; 4 blocks of 50 px went from 0 to 350 matching pixels |
| Medium zoom, rotation 0 | 88.9% | 89.0% | non-green 87.9% → 88.3% |
| Far zoom, rotation 2, [431, −245] | 88.4% | 88.4% | 60 pixels changed: 3 that matched no longer do, and none of the rest match either way |

Still different at near zoom: rows 436 to 520 below the left end. There
the original shows grass and a light sprite where we draw the wall's
foot. That is not this wall's depth or side, and is left to the
near-zoom work.
