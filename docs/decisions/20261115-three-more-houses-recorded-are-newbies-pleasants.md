# The three more houses recorded are the Newbies, the Pleasants in 6 Sim Lane and the Roomies in 10 Sim Lane

*Decided 2026-11-15.*

The roadmap's step 3 asks for 1000-update recordings of three more
base-game houses, picked for code the Goths do not reach. In the
install's base neighborhood (`UserData`) only the Goths (House05) and
the Newbies (House07) live in a house; the other eight base houses are
empty, and the three base families without a house (the Pleasants,
the Roomies, the Bachelor) wait in the family bin. The houses in
`UserData2` to `UserData8` belong to neighborhoods the game has not
opened yet (their families are imported on the first visit) and do not
load in strict mode yet. So two of the three are moves into empty base
houses, made in the original and saved once before the recording.

**The Newbies (House07).** The other resident base family: two adults,
no job yet, a different lot, and objects the Goth house does not hold
(by object file: `ShowerC`, `MedicineCabinet`, `Stereos`, `TVs`,
`flamingo`, `lampgarden`, `comeandsee`, `HelpSystem`). No save is
needed; the recording reads the install's own `UserData`.

**The Pleasants in 6 Sim Lane (House06).** Two adults and two
children. The Goths have one child (Cassandra). With two, the school
bus can take two children, and child-to-child interactions and the
siblings' relationship can run, which a one-child house never starts.
**Unverified**: the 1000 updates cover 7:10 to 7:43 in the morning, and
whether they reach this code has not been checked.
House06 is the furnished empty base house the family can afford
(§10,206 of their §20,000), with a bed, fridge, shower, toilet and
phone; its objects are base game only. The move-in is a player action
in the neighborhood view, made once (steps in the capture's
`movein-input-log.txt`), then the game was saved and the whole
`UserData` folder frozen next to the capture.

**The Roomies in 10 Sim Lane (House10).** Two unrelated adults (no
couple, no child), in a house with a fireplace (`Fireplaces.iff`, which no Goth
object uses) and a different floor plan and lot. The Roomies bought it
for §14,143 of their §20,000 and kept §5,856; the two larger empty
houses (2 Sim Lane with the pool at §50,776 and 3 Sim Lane at §24,793)
cost more than any family in the bin has. The Bachelor, the other
family left, is one adult, which runs less than two. Moved in and saved
in the original the same way as the Pleasants (steps in the capture's
`movein-input-log.txt`); the save writes the house with SIMI version
0x49.

**What the save holds that is not lot content.** The Complete
Collection adds the expansion packs' controller objects to every lot on
load (the Goth house goes from 245 saved objects to 377 at update 1),
and a save writes them into the house file: the saved House06 names
objects from every expansion pack and from Downloads (controllers,
phone plugins, inventory tokens such as gifts and collars) beside its
103 base objects. A boot from the frozen snapshot has to load those
definitions, as a load of an unsaved house makes them.

**What our side reaches on them** (strict mode; for the Pleasants and
the Roomies, with the frozen save as the neighborhood). The Newbies run
all 1000 updates. Their first differences: the globals at two of updates
1 to 3, the animators from update 1, positions from update 32 and
motives from update 154. One neighborhood record differs from update 629
on, in a record where the two neighborhood files themselves differ. The
Roomies load equal to the original. The Pleasants also load equal, but
for object 243 (data word 50 and its last result) and object 241's frame
(tree 0x2029, against the original's 0xa029). Both stop in update 1 at
the same place: a person's route with no route context (tree 0x0176 node
26, 0x45ae65), which is not ported. At that stop, person word 27 is one
behind the original's completed update 1, because that person's turn has
not finished.

**Reopen** if a house in `UserData2` to `UserData8` loads in strict mode
and offers more unreached code than a move-in does.
