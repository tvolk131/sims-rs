# The window saves on F5 and goes back to the save on F9, one save per house, in a REDB file

*Decided 2026-11-15.*

The window gets save and load as two keys, not a panel button. The
original saves from its Options panel, but what it saves is the house file
and the neighborhood, which is not ported. A button that looks like the
original's but writes something the original cannot read would mislead.
F5 and F9 are the usual quick-save and quick-load keys, and neither was
bound.

Each house has one save. Saves are kept in a REDB file given by `--saves`
(default `sims-game-saves.redb` in the current directory, like the window's
photos), in the `lots` table of `sims_sim::Snapshots`. Each save's name is
the house file's full path, so two neighborhoods' houses of the same number
keep apart. `Lot::restore` also refuses a save of another house or family.

Going back to a save drops everything the window kept of the lot before:

- why it stopped, and the errors caught;
- the sounds not yet taken;
- an open or listed pie menu;
- the icons pressed;
- the dialog read.

The panel's speed follows the saved lot's. While the idle loop holds the
game at its own speed, the panel shows the speed the game goes back to
(global 18), which the player chose. The view turns to the saved lot's
rotation, keeping the middle of the screen, because the lot's code reads
that rotation (Generic Sims Call 12, Find Location). Keeping the window's
rotation would make the picture and the simulation disagree. The people
and the rooms are read again. Sounds already
playing are not stopped. What a long sound does once its object is gone
is unverified.

The window's tests check two things. Going back to a save made at update
50, after a pie menu, ultra speed, a turn of the view and 100 more
updates, turns the view back and plays on to match a lot that ran 250
updates straight. The panel shows the player's speed while the idle loop's
holds. A save made while the Help System
dialog waits shows the dialog again.
