# Objects sit one pixel right of the walls

*Decided 2026-11-14.*

At every compared camera, objects were drawn one pixel left of where
the original draws them, measured against the walls and floors around
them. Moving every object one pixel right raised the exact matches at
all three cameras. Moving them one pixel up or down, or two pixels
right, did not help. Moving the walls one pixel either way instead
lowered the house box from 89% to 57%, so the walls were already in
place. The shift helps plain and mirrored sprites alike.

| Camera | House box before | after | Whole screen before | after |
| --- | --- | --- | --- | --- |
| Medium zoom, rotation 0 | 74.1% | 88.9% | 76.4% | 85.8% |
| Far zoom, rotation 0, walls up | 75.7% | 89.0% | 84.4% | 89.8% |
| Far zoom, rotation 3, walls up | 74.0% | 88.5% | 80.8% | 86.2% |
| Far zoom, rotation 0, cutaway | 57.6% | 68.0% | 80.8% | 85.7% |

(The cutaway row uses the settled screenshot with no rooms in focus,
so it is lower than the figures in earlier entries.) A search around
each fitted camera origin still picks the same origin.

The renderer's camera origin is fitted to the tile art, so the
constant is `OBJECT_COLUMN = 1` in the object placement. Where the
original's pixel comes from is **unverified**: its tile rectangle
(0x4ac25d) runs from x - half width - 1, which suggests it draws tile
art one pixel left of the projected point and objects at it. The
floor code around 0x4b17b2 shows no such -1, though, so we have not
confirmed it.

**What still differs at the door at medium zoom:** the door's position
matches pixel for pixel on its right edge and handle. One column of
its left jamb (x 466) still shows the wall in our render and the door
in the original's. Our wall there is 13 depth bytes in front of the
door frame, and its doorway mask opens one column later. Shifting the
doorway masks one pixel either way lowers the overall match, so we keep
them where they are; that does not prove they are exact at this column.
The original's wall may have a deeper edge around the opening that a
flat wall depth does not reproduce. Both are **unverified**. Window and door
frames also differ by one light step in places. The original's
screenshot also has a cardboard box by the front door that a tick-1
world does not have.
