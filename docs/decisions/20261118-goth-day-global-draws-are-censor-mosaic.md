# The Goth day's global draws from update 4155 are the censor mosaic

*Decided 2026-11-18.*

In track 2's recording `goth-day-20261115` the global random stream
(state 0x6e54ac, draw 0x46f457) does not move from the load to update
4154. It then moves after 4155, 4185, 4215, 4245, 4275, 4305, 4335 and
4365 (3, 21, 24, 24, 24, 24, 21 and 6 draws), each 15 past a multiple of
30 (the clock's second 14, 8:49 to 8:56), and never again that day. The
port's global stream never moves. The lot stream and the C runtime's
stream match the recording through these updates.

**What draws.** The house view's draw (0x4a8e6c, and the other drawing
paths 0x4b3510 and 0x4b3b68) calls 0x4ac707 after it draws the lot. That
walks the view's objects ([[[0x6e48bc]]+0x14]+0x84), and for each one
whose person data word 30 (+0x5c8, the censor word, read by 0x592992) is
set, it takes the object's screen rectangle (0x5929a0, +0x230; 0x7fffffff
for none), pads it by the zoom (1, 2 or 4 pixels), and pixelates it
(0x4ac301). That cuts the rectangle into blocks of 2, 4 or 8 pixels by
the zoom. A block is drawn only if one of its pixels is nearer than the
person's depth (its z-buffer word above the bound 0x49a5d3 gives). Each
block drawn takes its average colour and three random draws, `(rand &
0xf) - 8` for each of red, green and blue (0x4ac593 to 0x4ac5be), from the
global stream, since a draw outside the lot update draws from the
global stream.

**The recording fits it.** Person 18's word 30 is 128 from about
update 4131–4140 (it showers) to about 4380–4390, and no other person
has a word 30 set all day. The stream moves only in that span, and only
once per 30 updates, as frames are drawn (see
`20261118-goth-day-frame-schedule.md`): no frame was drawn between the
updates that moved. Every burst is a multiple of three. How many blocks
are drawn depends on the zoom, the camera, and what stands in front of
the person, so the burst sizes are a property of the frame, not of the
simulation.

**Decision.** The draws are presentation. The strict boot draws no
picture, so it does not draw them, and its global stream keeps its
value. Nothing in the lot's update reads the global stream on this day
(the lot stream is swapped in around the scheduler pass), so leaving
them out changes nothing the simulation computes; the lot and C runtime
streams match the recording to update 16111 under the recording's frames
(`the_goth_day_under_the_recordings_frames_keeps_its_streams`). A
drawing port that wants the mosaic's noise draws it from the global
stream after the lot, three per block drawn.

**Unverified.** The code path is read from the disassembly; no capture
has caught a write to 0x6e54ac at 0x46f457 called from 0x4ac301. A
native capture with a write watch on 0x6e54ac over updates 4150 to 4370
would show it. The block test's exact bound and the rectangle's
padding were read but not run.

**What would reopen it.** A global draw in the recording outside a
censored person's frames, or a simulation tree that reads the global
stream while a person is censored.
