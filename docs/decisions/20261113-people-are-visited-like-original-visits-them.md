# People are visited like the original visits them, and the load runs until the interface takes over

*Decided 2026-11-13.*

Load step 13 visits every object on the lot. It used to stop at the first
person. `sims boot` now visits people too, finishes the step, and runs
steps 14 to 16. On the Goth house all 382 visits run. The boot stops
after step 16, before the original's interface selects a Sim.

Some terms. A person's *neighborhood record* is its entry in the
neighborhood file: an ID, the person's type, and 256 person data words.
A record has a *revision*, which says how many of those words it keeps
for the person. A type has *changed* when the scripts installed now
differ from the ones the house was saved with. Function 3 of an object is
its Reset tree and function 0 its Init tree.

What was decided:

- **A person's visit is `Interpreter::visit` too** (0x459bf6). It differs
  from an object's in three ways. The person first takes the fields its
  neighborhood record keeps (`attach_neighbor`, 0x4258a3). It is not
  asked its Load function. And if its type changed it resets in its own
  way (`reset_person`, 0x45e791).
- **The record's fields.** The record's ID goes into person data word 31.
  Then the words `persistent_fields` lists for the record's revision are
  copied from the record: 29 at revision 1, 48 at revision 10. The record
  is the first of the person's type in the file.
- **A person's reset**, in this order: person data word 30 is cleared;
  the gender, age and skin tone are read again from the body strings of
  the person's type (`reload_profile`); the thread restarts at Main and
  then the Reset tree runs on top of it; person data words 0, 40, 59, 33,
  71, 72 and 73 and data words 34 and 9 are cleared, with any motive
  changes in progress; and each queued action is dropped, its target
  being asked function 4. An object's reset runs the Reset tree first and
  restarts the thread afterwards. A person's does it the other way round.
- **Only a person that is doing nothing can be reset.** A person that is
  using an object, has a running action with a target, is in the middle
  of a route, is inside another object or off the lot, or has person data
  word 73 set fails. The original does more for each. No person of the
  Goth house is in any of those states.
- **The cached room and the motive curves are not kept.** After the
  visit the original notes the person's room, which it uses to light the
  room, and loads the curves that turn each motive into its share of the
  mood. The lighting is not modeled. Nothing reads the curves until a
  person's motives are updated in a tick. `curve_table` says which string
  table the curves come from. So `World` did not change, and the snapshot
  format is still 7.
- **`People` now carries each person type's profile**: three of its body
  strings (gender, age, skin tone) and the skin tone names from its
  semi-global file.
- **Step 14 is `Interpreter::ensure_indicators`** (0x445e4a): create the
  two indicators unless the lot has them. Step 16 calls it again.
- **Step 15 is `World::finish_selection`.** After the visits the function
  that selected the household may delete people who do not belong, create
  a tutorial controller, reset the lot, and work out who is a visitor.
  None of that is ported. The step checks that none of it could do
  anything, and then changes nothing: every person is of the selected
  family and not a visitor, global 26 is 0, and the family's record does
  not ask for a reset (flag 4). Otherwise it fails and says which.
- **The roadmap's step list was wrong at its end and is corrected.** It
  had one last step, "create the indicator objects and finish
  selection". The trace shows nine more stops after the visits. Steps 14
  to 16 belong to the loader and are ported. Steps 17 to 22 are the
  interface's: it clears and sets the selected Sim, moves an indicator
  into that Sim's slot, goes to live mode, and updates the rooms. They
  are written down in the roadmap from the trace and are not ported.

How it was checked. The private trace of the original loading the Goth
house holds a full state at the return of step 13, and at three stops
after it: the return of the indicators' creation after the last visit,
the return of the lighting update that ends the household selection, and
the return of the indicators' creation called when the house is
announced. For every visit it holds each call of the function that runs
an object's callback (0x422d46). Our run was compared with it outside
the repository (`people_verify.py` in the private scratchpad, which
prints one line for each thing compared).

*Equal to the trace:*

- **Every call of every visit.** For each of the 382 visits: the object
  visited, and the calls it makes, as object and function, in order.
  1,781 calls. The three people are visits 202 to 204, objects 21, 18
  and 16, and each is asked function 3 and then function 0.
- **The whole state at the return of step 13.** All 386 objects: GUID,
  the 72 data words, temps, attributes, scheduler word, last result,
  every frame's tree, node, stack object, parameters, locals and saved
  primitive state, the relationships, the number of slots, the position
  and the removal-without-resets mark. For the three people also the 256
  person data words, the 16 motives and the 16 previous motives bit for
  bit, the running action and the last action (ten fields each), the
  queued actions and the number of objects in use. The tick, the 64
  globals, both game random streams and the C runtime's, the funds, the
  removal queue, the indicator references, the registration order, the
  instance lists, which rooms wait to be counted (8 of 11), and the
  arrays inside the lot's size, the object grids included.
- **The same at the three later stops**, after our steps 14, 15 and 16.
  The trace changes nothing compared between them, and neither do we.
- **The two stops in the middle of visit 140**, as in [20261113-objects-are-visited-up-first-person-frames-keep](20261113-objects-are-visited-up-first-person-frames-keep.md),
  now with the people's motives and actions compared as well.

This is the first check of what is not an object field after visit 140.
[20261113-objects-are-visited-up-first-person-frames-keep](20261113-objects-are-visited-up-first-person-frames-keep.md) called that unverified: the global random stream, the
globals, the arrays and object grids, the removal queue and the rooms
waiting to be counted. All are equal at the step's return. The global
random stream moves during the step, so its being equal is not empty.

What the trace shows of a person's visit, and what it cannot show:

- It shows the record's fields being taken: person data words 46 to 55
  change, to the record's values. The other words copied already hold
  the record's values.
- It shows the thread restarting: three frames become one Main frame at
  node 0 with no stack object.
- It shows what the Reset and Init trees do: the last result becomes
  true and seven motives change. Ours are equal bit for bit.
- It cannot show the words the reset clears or reloads being written.
  In this house they already hold the values written: the cleared words
  are 0, and the gender, age and skin tone read from the body strings
  are the saved ones. So the trace shows them unchanged, and so do we.
- It cannot show whether Main is restarted before or after the Reset
  tree runs. The result is the same here.

*Consistent with the old prototype's isolated comparisons only:*

- The order inside a person's reset, and which words it clears. The
  prototype ran the original's reset alone on 16 cases.
- The list of fields a record keeps, by revision. The prototype compared
  126 cases. The Goth records have revision 10.
- The body strings read by the reset and how they become numbers. The
  prototype compared 216 cases.
- The conditions of step 15, which the prototype compared on 192 cases.
- A queued action being dropped and its target asked function 4. No
  person of this house has a queued action.

What did not match, and why it does not matter. Object 19's second frame
has tree ID 0x8118 in the trace and 0x118 here and in the house file, at
every stop before its visit. Its visit is number 206. The visit makes
the same three calls here as in the trace (functions 2, 3 and 0), and at
the step's return object 19 is equal in every field compared: its reset
replaced the thread. So the bit does not change what the visit does.
What sets it is still not known. Six type attribute words belong to the
game that was running; with those copied from the trace's start they are
equal.

What changed in earlier work, and why:

- **The gated test of step 13 now runs the whole step.** It still shows
  the state pinned after visit 201: it makes the first 201 visits one at
  a time on a copy and gets the old digest. No other pinned digest
  changed, because `World`'s encoding did not.
- **`sims boot` no longer says it stops before the indicators.** It
  reports steps 14 to 16 and stops before step 17.
- **The roadmap's "Progress" said the boot runs through step 12.** It had
  been running into step 13 since [20261113-objects-are-visited-up-first-person-frames-keep](20261113-objects-are-visited-up-first-person-frames-keep.md).

What is **unverified**:

- **A person that is doing something**, as listed above. It fails.
- **A neighborhood record of a revision below 1**, which holds no person
  data, and a person with no record. Both fail.
- **The motive changes in progress being cleared.** The trace does not
  hold them, and the Goths have none.
- **Which motive curves a person gets** (`curve_table`). Nothing loads
  them yet. The trace holds the two global tables at the end of the
  load, not which person uses which.
- **Step 15 on any other lot**: a visitor, a person of no family, the
  tutorial house, a family that asks for a reset. All fail.
- **Steps 17 to 22**, which are not ported. What the trace shows of them
  is in the roadmap. Two things about them are not known from the
  trace: how the game picks the Sim it selects, and what makes the six
  indoor rooms still wait to be counted after the room update.

What is not modeled: a person's animations, graphics, action icons and
accessories during a reset; the guard against a reset inside a reset;
the cached room and the lighting it drives; the lighting update at the
end of step 15; and the signal that announces the house, beyond the
indicators it asks for.

What fails loudly: resetting a person in any of the states above; a
person type with no profile or no neighborhood record; a record with no
person data; a body string age that overflows 32 bits; and each
condition of step 15.

What would reopen this: a house whose people are in the middle of
something when saved; a house with visitors; a capture that holds a
person's motive changes or shows which curves it uses; anything that
shows a reset writing a value that differs from what the trees or the
save left.
