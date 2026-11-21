# A step runs both ways while a refresh is pending, and a dialog's portrait makes a person

*Decided 2026-11-20.*

Strict mode on the Goth house stopped at update 24716
(`20261119-generic-sims-call-6-and-the-saved-neighbor-record.md`). Three
changes take it through the whole day:

- a step that may read an animator while a refresh is pending runs both
  ways;
- a route from no room has no way;
- a dialog's portrait is built the way the interface builds it, on a
  tile searched for from the house view's centre, which is an input.

With these, the C runtime stream matches the recording
`goth-day-20261115` at every update of the day. The lot stream matches
through update 36717. After that it moves one update early or late at a
few updates (see below).

## The refresh's branch is not known, so a step runs both ways

A person that takes itself off the lot in its own tick has its animator
refreshed at the take-off (0x433125, 0x58da12). The refresh is made only
if animator byte +0x109 is set, which happens when the house view drew
the person at its last draw. That byte is presentation and is not kept.

The disassembly gives the refresh's timing: it runs at the take-off,
and the port already works it out there (`Interpreter::refresh_at_take_off`).
What the port cannot know is whether the refresh was made at all. Until
the person's animator's tick, the port keeps both animators: the one the
take-off left, and the one the refresh would have left
(`placement::Pending`).

So far, only primitives that read no animator were allowed to run in
between (`placement::reads_no_animator`), and any other primitive failed.
Now any other primitive, except Remove Object Instance (which checks for
itself), runs twice (`Machine::step_both_ways`, `crates/sims-vm/src/both_ways.rs`):

- once on a copy of the lot as it is;
- once on a copy that has the refreshed animator.

Each run also has its own copies of every cell the machine shares with
the rest of the tick (`both_ways::Cells`). Both runs must give the same
result and leave the same lot, apart from that person's animator. The
lots are compared by their encodings and by each thread's count of
restarts, which the encoding leaves out. The runs must also leave the
same in every cell:

- the questions asked;
- the objects deleted;
- the sounds;
- the pie menu;
- the action run at once;
- the objects let go;
- the objects being deleted;
- the people another object took off;
- the refresh pending.

Floats are compared bit for bit. An action's attenuation is a plain
`f32`, which `==` would call equal at -0.0 and 0.0 and `Debug` would call
equal for any two NaNs, so actions (the one run at once, and the pie
menu's) are compared written out with postcard, which keeps the bits.
Every other float in the cells is an `F32`, whose `==` compares bits.

If they do, the original reached the same state whichever branch it took.
The lot and the cells go on with the first run, and both animators are
kept, as each run left them, for the animator's tick to settle. If the
runs differ, or either run takes a person off the lot (itself, which
leaves a refresh pending in its copy, or another, which changes its copy
of the people taken off), the step fails loudly and names 0x58da12 and
byte +0x109. Neither run is kept then: the lot and the cells are as they
were before the step. When the runs agree on a fault, the first run is
kept with the fault, as any step that faults keeps what it did before
it. Keeping a run borrows every cell before it writes anything, so a cell
held elsewhere panics before the lot or any cell is kept.

A person queued for removal by another person's tick has its refresh
kept for the scheduler's pass (`taken_off`). A person's tick run on its
own (`Interpreter::tick_person`) gets a pass of its own for its whole
tick, its thread's run and its animator's tick alike. No other person's
tick follows in it, so a refresh left there at the end fails loudly
rather than being dropped, which would hide it from the comparison. A
tree run outside any person's tick still drops it.

The refresh itself (`Interpreter::refresh_at_take_off`) and the
animator's tick that settles the two animators
(`Interpreter::tick_animator_after_take_off`) work the same way. Each
branch runs on its own copies of the lot and the cells, and they must
agree in all of them. The tick alone is kept only when they do.

This stands in for the branch the original took, which is not known. It
is not a substitute for the refresh's timing, which is ported. Strict
parity is not weakened: a step whose outcome depends on the branch
stops the run.

`a_step_both_ways_that_differs_fails_loudly` makes the two runs disagree
and checks that the step fails. In it, a clip's timed event is due at
the clock the refresh reads, so only the refreshed animator has the event
queued. Animate Sim then takes the event on one run and waits on the
other. `a_step_both_ways_that_fails_keeps_neither_run` has a step that
takes the person off the lot again, and checks that the lot is left as
the take-off left it.
`after_taking_itself_off_a_step_that_agrees_both_ways_goes_on` is the
agreeing case. `actions_are_compared_bit_for_bit`,
`a_cell_held_elsewhere_stops_the_keep_before_anything_is_kept` and
`a_person_removed_in_a_persons_tick_outside_the_schedulers_pass_fails_loudly`
cover the comparison, the keep and the removal above. A run can take another person off only through Remove
Object Instance, which never runs both ways, or a tree that a primitive
run both ways calls. No synthetic test reaches that case. The check is
there so that such a run fails rather than passes.

The refresh lasts only for the person's own tick (`Refreshing`), and the
people another object took off last only for the scheduler pass
(`taken_off`). Carrying two branches further would need a World-side
store with a format bump, and it would need the animator's tick run both
ways until the branches meet. #255's two person-reset branches would need
both, and neither is done here.

## A route from no room has no way (0x45c3ca to 0x45c3e4)

Route to slot (0x45c38e) looks up the person's room (data word 29) in
the lot's rooms (0x466fcc, a map's find that gives null at its end). A
person in no room, such as one off the lot, gets the answer 0 (0x45c4c7),
and the candidates are dropped (0x481898), as when the planner finds no
way. The port used to fail there.

At 24716, person 12 has taken itself off the lot and runs Go To Routing
Slot (tree 0x2040 node 7). The step runs both ways, and both runs answer
0 before reading the animator.

## A dialog's portrait (0x43601c, 0x4e92a8, 0x4e8c5d)

At 24719, object 18 (GUID 0x512C81F6, made at 24716) shows a dialog
(tree 0x100a node 1, operands [0,0,1,0,0,0,2,4]) with icon mode 2: the
neighbor record that the stack object (328) names. The recording's C
runtime stream moves 1,920 draws there, one cat skeleton's copy
(0x598da3). No object is made or removed in the recorded states at 24718
and 24719.

The ordinary dialog record's constructor (0x43601c, from 0x4443ab's
default case) picks the person whose portrait it shows (0x436279):

- mode 0: the type the text found (below), or else the type the stack
  object's definition stands for (0x436308 to 0x43634d, 0x45728d through
  0x43ae0a): its own, or a tile's group master's. A tile whose group has
  no master leaves the type null, and no portrait is shown (Sol's tenth
  check);
- mode 2: the neighbor record's type (0x425afe, record +8).

It then has the interface build that type's portrait:

1. 0x4e9753 finds the type in the interface's table of person types
   (0x6f2698). At startup the table gets one entry for each person
   definition in the catalog (0x4e8b3d), and 0x4e8a44 adds entries for
   later ones.
2. 0x4e92a8 builds the entry's first image if it has none.
3. 0x4e8c5d draws the first person of that type on the lot (0x4e9608).
4. If there is no such person, it makes one (0x4e9652, the factory
   0x440b2d), puts it on a tile near the house view's centre (0x4347f0
   from 0x4a677c, then virtual function 0x88), draws it, and removes it
   at once (0x4e8fdb, 0x44123f).

`World::portraits` (snapshot format 31) keeps which types have their
image. The Dialog primitive does what 0x4e92a8 does when it makes a
record (`Machine::dialog_portrait`). In the recording the cat's type had
no image yet, so a cat was made and removed. That accounts for the
1,920 draws, and the objects after it match the recording.

A person made later can take its type's image away again. At the end of
making a person (0x44e6b6 to 0x44e70b, after its traits and flag 0x20's
turn), if person word 84 bit 0, word 68 bit 0, or word 74 bit 0 or 1 is
set, the original calls 0x45810c, then 0x4eb0eb and 0x4e895e, which
reset the type's image slots, so the next dialog builds the portrait
again. `make_person` takes the type out of `World::portraits` then
(Sol's seventh check). Test:
`a_person_made_with_person_word_68_74_or_84_set_resets_its_types_portrait`.

What is not ported:

- **Text tokens.** This part is ported. The record's strings come from
  STR# 301 of the running code's file (0x4360b8, 0x47028b):
  - the buttons' labels its kind loads (0x436175);
  - the title (operand 6);
  - the message (operand 2);
  - in icon mode 4, operand 1's string (0x436297 to 0x4362c0), with no
    place for a type.

  Each string is formatted in that order (0x44bf1a, from 0x436219 to
  0x436265) in every icon mode, before the mode is read (0x436279), so
  the checks below that do not concern the portrait run in every mode
  (Sol's eighth check).
  - A string that is number 0, that the file lacks or that is empty is
    replaced by the default its caller gives (0x4369de, 0x436a21 to
    0x436a35): a button's label from STR# 142 of `GameData/UIText.iff`
    (0x435fee, 0x43600a), or else "Missing string!!!" (0x6c29bc). The
    port leaves such strings out only when the link finds every string
    table of ID 142 in that file (STR# and CST, deleted chunks included)
    free of any `$` byte, its resource map matches its chunks (the
    original reads the map as it is, 0x41bfdd to 0x41c032, so a map that
    points ID 142 at another chunk changes the labels), its table of ID
    142 is one the loader reads as ported (with the map's flags), since
    one read otherwise may be read past its end, and the file is the only
    component of the `UIText` resource set: the composite opener also
    opens a `.fam`, `.wll` or `.flr` of the name, loose or in an archive
    in `GameData`, and a `.dir` of the name that is a directory there,
    which the directory reader (selected at 0x46f50f and 0x46b5d9) lists
    with a `*.*` wildcard (0x5c3099 to 0x5c3102) (0x46f50f to 0x46f542,
    0x46b627 to 0x46b699, searched at 0x406a29), none of which is ported
    (`Program::labels_hold_no_token`, Sol's eleventh to fourteenth
    checks). The check also counts a `.dir` that is a file or an archive
    member, which the original does not open: stricter than need be, not
    a port. The tables must not be ones the loader reads again for
    English (entries, none in US English: 0x47006b to 0x4700d0), which
    can read a string's bytes as a language byte, and an archive there
    that cannot be read as a regular file (a dangling link or named
    pipe, say) makes it false without blocking. Otherwise a dialog with
    a default string fails loudly. The
    install's labels (OK, Cancel, Yes, No) hold no `$`. Tests:
    `the_default_button_labels_hold_no_token` (`tests/boot.rs`),
    `a_default_string_fails_loudly_unless_the_labels_hold_no_token`,
    `the_default_labels_hold_no_token_only_when_no_string_table_of_them_holds_a_dollar`.
  - The table is read in the application's language (the table's
    language byte is interface +0x64 less one, 0x4702b1 to 0x4702bb;
    0x4704fa reads that language's strings and falls back on no other).
    The port reads US English (language 1), the recorded install's: its
    registry export has `SIMS_LANGUAGE` USEnglish, which 0x4f2ddc to
    0x4f2df8 read after `SIMS_OTHERLANGUAGE`. The interface's byte for it
    is **unverified**. No other language is supported: its strings may
    hold other tokens.
  - A table with entries in more than one language is read in the order
    the original's loader gives (0x46ffcc to 0x470069, Sol's tenth
    check). The loader counts the entries of each run of one language
    from 1, starting again where the language changes (0x470002), and
    inserts each entry of the wanted language at its place in its run
    (0x470482). So a later run goes in front of the earlier ones. An
    entry with language 0 counts as the wanted language (0x46fffc). The
    Goth day's dialog tables are multilingual: the first version of this
    check failed loudly on them at tick 1. The loader reads the resource
    of the ID (deleted chunks skipped, 0x41c542), not the last chunk of
    it, and copies each entry's bytes as they are (0x470404 to 0x470434),
    so an entry starting `ff fe` is not a byte order mark (Sol's eleventh
    check). `Library::loaded_strings` keeps the loader's view where it
    differs from `Library::strings`'s (None for a file whose chunks of
    the ID are all deleted), and only the dialog reads it. A deleted
    chunk that does not parse no longer stops the link. Whether the other
    readers of `Library::strings` (animations, action strings, suits)
    load through the same loader is **unverified**. In the recorded
    install it makes no difference: a scan of its loose IFFs and
    uncompressed archive members found 2,407 multilingual STR# tables
    (Sol's wider scan 3,661; which files each counted is
    **unverified**), each with one run of US English, so the loader's
    order is the file's for all of them. Tests:
    `a_later_run_of_the_language_goes_in_front` (`program.rs`),
    `a_string_table_is_read_as_the_loader_finds_it` (`tests/program.rs`).

    The loader's other ways are not ported, and fail loudly when the
    dialog names a string (`Library::unported_strings`,
    `program::unported_strings`, Sol's eleventh and twelfth checks):
    - a table whose entries have a language and none in US English is
      read again for English (0x47006b to 0x4700d0), stepping through it
      one byte at a time in a way that may read a string's bytes as
      entries;
    - a resource without flag 0x10 has its header byte-swapped first
      (0x41d3a7 to 0x41d3d4, through 0x470351). The flags are the ones
      the resource map states when the file has one (0x41b7fa to
      0x41b813, `Iff::flags`), not the chunk header's (Sol's reading,
      **unverified**);
    - a format other than -1, -2 and -3 (version 0 and the classic Mac
      layout) is read with its leading word as the count, so a version-0
      table has no entries, and with bytes 0xa1 and 0xd5 rewritten as `o`
      and `'` (0x4700d7 to 0x47014c);
    - a count is a signed word (0x46ff50, 0x46ff92), so one above 0x7fff
      is none, and a table with no count is read past its end.

    Every STR# 301 of the recorded install is of format -3 or -2 with
    flag 0x10. A file whose resource map does not match its chunks, which
    the original reads as it is (0x41bfdd to 0x41c032), fails the link
    (`Iff::resource_order`), so no dialog reads it.
  - A file with no STR# 301 but a legacy CST 301, which the loader reads
    instead (0x4702c7, 0x47018e to 0x4701e6), is not ported, and fails
    loudly when the dialog names a string. The loader compares the tag
    with 0x435354, and the header reader reads the type's bytes
    big-endian (0x41a5af, 0x472fbf), so the tag on disk is `\0CST` (Sol's
    eleventh check; the install has none). Tests:
    `a_dialogs_strings_are_read_in_the_order_the_loader_gives`,
    `a_string_table_is_read_as_the_loader_finds_it` (`tests/program.rs`).

  The strings share one place for a type, cleared first (0x43620d), which
  only mode 0 reads. Only two tokens write that place:
  - `$Object` (0x44c1af) gives the stack object's type;
  - `$Neighbor` (0x44c69f) gives the type of the neighbor record that
    the stack object names (0x425afe). Its pass comes after `$Object`'s.
    It finds `$Neighbor` anywhere in the text as the passes before it
    left it.

  The last type found is kept, and mode 0 shows it in place of the
  definition's (0x436308). Test:
  `with_nothing_found_mode_0_shows_the_type_the_definition_stands_for`. Mode 2 does not read it. The first version of this
  change failed on any `$` in a mode 0 dialog's strings. On the Goth
  house that stopped the run at update 901 (object 8, tree 0x101d), so
  the passes are ported in order instead, and the ones that are not
  fail loudly where their token is (below).
- **Placement.** The put (virtual function 0x88, 0x45e4e9) is the
  placement 0x433340, on a tile the interface searches for (0x4347f0,
  with the defaults of 0x4347d1: a spiral on the ground level, in the
  start tile's room, on an empty tile, no random start) from the house
  view's centre: the view's pick at the centre of its rectangle
  (0x4a677c, 0x4a7b58). The camera is the player's, so the tile at the
  view's centre is an input, as the view's rotation is
  (`Interpreter::view_centre`, `Lot::set_view_centre`, `sims boot
  --view-centre X,Y`, `Lot::load_viewed`, saved with a lot from save
  format 2). The interface's work on the pick is not ported, so the input
  is the tile the search starts from (see "The view's centre at 24719"
  below). When the tile
  is given, the search runs from it as in the original, its place tests'
  callbacks included. When it is not, the search runs from every tile it
  may start from (`Interpreter::search_from_anywhere`), on a copy of the
  lot and of the machine's cells. Every callback tree the place tests
  run (function 5 of the person and of the objects in its way, 0x434b94,
  0x433a1a, 0x433a35) must then leave both as they were, since which
  tiles the original tries depends on the start. In practice any callback
  run fails this: it leaves its frame's words in the object's stack
  storage (`FrameStorage`), which is part of the lot, unless that storage
  held the same words already. So without a view centre the portrait
  stops at the first callback its search runs. At the Goths' 24719 none
  is run.

  `Interpreter::portrait_put` does what is the same on every tile the
  search may end on: the interface's work for an object come from
  outside (0x44752e, `placed_from_outside`). The person's removal is
  taken to undo the rest of the put (the rooms, 0x440385; the tiles,
  0x4330d7; **unverified**). It fails loudly where the tile would leave
  the lot otherwise than the removal finds it:
  - the person's function 9 (0x433580), 6 (walls changed, 0x435110),
    25 or 26 names a tree. A person with function 25 or 26 is a seat:
    the objects on the tiles beside it are told, and may be asked their
    function 9 (0x43355d, 0x433612). So does its function 10 (before the
    removal, 0x433148, which sees the room it was put in). Its function 5
    (0x4337f6) is the search's to ask: run as in the original with a view
    centre, held to change nothing without one. Its Init (function 0) runs
    when it is made, as for any person;
  - after the search (whose callbacks may set them), its data word 7 is
    not 0, which marks the room it is put in and taken from as not
    counted for its light (0x433ed8, 0x4412c6);
  - after the search, its data word 40 has bit 0x400, which marks the
    room it lands in as not counted (0x433340);
  - noting the room the search ends in (with no view centre, any it may
    end in) changes a room's mark of who is in it (0x45e43d, 0x468802),
    for a person of kind 0 or 1 (person word 65; a pet is above 1). The
    put marks an indoor room that is not a pool and not marked. It also
    takes the mark off the room the person was last seen in (data word 29
    as its first tick noted it) when nobody else was last seen there. Its
    removal leaves the marks. This is tried on a copy of the lot for each
    room (`note_room`);
  - no tile is found (from the view centre given, or with none from some
    start). The original then removes it without drawing it (0x4e9706),
    and 0x4e938c builds again in the same dialog unless the type has
    image 2002. That is not ported;
  - the lot has no indicators yet. The interface's work for an object
    come from outside (0x44752e) then makes them (0x445e4a), and their
    Init runs while the original's person stands on its tile, in its
    room. Here it does not, since the put is only as far as is the same
    on every tile.

  The build runs on a copy of the lot and of the machine's cells (the
  sounds its Init plays, the objects it deletes), which are kept only
  when it is done, so a build that fails leaves nothing behind (the
  original removes the person on every path, 0x4e8fdb, 0x4e9706). A
  build that changes the refresh pending fails loudly. Tests:
  `a_portrait_whose_person_runs_code_when_put_down_fails_loudly`,
  `a_portrait_whose_put_depends_on_its_tile_fails_loudly`,
  `a_portrait_searches_from_the_view_centre_when_it_is_given`,
  `a_callback_the_search_runs_must_change_nothing_when_no_centre_is_given`,
  `a_portrait_put_on_a_lot_with_no_indicators_fails_loudly`.
- **The formatter's passes.** They run in this order (0x44bf1a). The
  tokens were matched to their strings (0x6c4054 to 0x6c4110) through the
  static constructors (0x44bc00 to 0x44bf1a) in the memory capture. Each
  pass searches the text as the passes before it left it, from the
  start, then from one past each place it looked at (0x4558a0).
  1. `$FamilyAssets` (0x44bf66) and `$Family` (0x44c034) are not ported.
     `$Family` writes the name of a family (0x425d69), which may hold
     any token, and loops as `$Me` does (0x44c0cf to 0x44c0f7). A text
     that holds `$Family` fails loudly (Sol's ninth check).
  2. `$Me` (0x44c0fc) is ported, because the Goth day needs it: at 7321
     the message is "What vision, what talent! $Me received one
     Creativity Skill Point.", with Bella both running the tree and as
     the stack object. The formatter's `this` is the record's object
     (0x436200), which is the object running the tree.
  3. `$Object` (0x44c155) is ported: the stack object's name, and its
     type for the portrait. With no stack object the token stays
     (0x44c183 to 0x44c1a6; Sol's ninth check).
  4. `$Local` (0x44c1f2) is ported, because the Goth day needs it (the
     dialog at 902). Each token, with its `:` and numbers (0x44caf6), is
     replaced by the value of the local its first number names, in
     decimal (0x46f941, `%d`), when the number is one of the tree's
     locals; otherwise it stays. The local is read as any read of a
     local is, so one that starts as half of a pointer fails.
  5. `$Attrib` (0x44c280), `$NameAttrib` (0x44c330), `$NameLocal`
     (0x44c3ff) and `$TokenNameLocal` (0x44c490) are not ported. They
     read the stack object's attributes and the tree's locals: with no
     stack object `$Attrib` and `$NameAttrib` fault (0x44c2dd, 0x44c381),
     and `$TokenNameLocal` throws for an object that is not a person
     (0x44c57b) or looks up, and may add, an inventory (0x42a8a1). A text
     that holds one of their tokens fails loudly, in every mode and
     whatever the locals (Sol's seventh to ninth checks).
  6. `$NeighborLocal` (0x44c590 to 0x44c65a) replaces a token with a
     person type's name when the token's number is one of the tree's
     locals, the local names a neighbor record (0x4256f4), and the
     record's type is in the interface's table of person types
     (0x4e9753). The name comes from the interface's table (0x4e9c46,
     0x4e893e), which is not ported, so that fails loudly. A record
     whose type is not in the table ends the pass (0x44c616). A token not
     replaced stays, and `$Neighbor` is found in it.
  7. `$Neighbor` (0x44c65f) is ported: the neighbor's type's name
     (0x4573d9), and its type for the portrait (Sol's ninth check).
  8. `$CatalogLocal` (0x44c6ea), `$FameTitleLocal` (0x44c797) and
     `$Grade` (0x44c861) are not ported, and a text that holds one fails
     loudly for any object running (Sol's tenth check). `$CatalogLocal`
     writes a catalog name (0x4573de, 0x44c771), which may hold its token
     again, so the pass would never end. `$FameTitleLocal` (0x405241,
     0x44c83d) and `$Grade` (0x4034ce, which writes nothing for some
     numbers, 0x44c8ae) may join the text around them into a career token,
     for example `$Jo$Grade:16b`, which faults for an object that is not a
     person.
  9. `$TimeLocal` (0x44c8c4) is presentation: an hour local in [0, 24)
     written in one of fixed formats (0x4a6ab1), which hold no token.
  10. The career's, `$JobOffer`, `$JobDesc` and `$Job` (0x44c966), write
     career texts. Each career token casts the running object to a person
     (0x44c9bc) and reads it (0x44ca27), on every path, which faults for an
     object that is not one: that fails loudly (Sol's ninth check). For a
     person each pass searches again from one past the place it wrote
     (0x44ca9f to 0x44cad3), so a text that holds a career token past its
     first character, or ends in the start of one, may never end (Sol's
     tenth check: STR# 4 entry 15 of `work.iff`, loaded by 0x404328, set
     to `x$JobDesc:0`). `Program::career_texts_end` is whether no text
     the passes may write does. The careers' strings come from the `Work`
     resource set of `ExpansionShared` (0x403da2 to 0x403dd9, the
     directory found at 0x415336), which the composite opener makes of a
     `.iff`, `.fam`, `.wll` and `.flr` of the name, loose or in an archive
     there (any entry not a directory whose name ends in `far`, with no
     dot needed, 0x5ba7d4 to 0x5ba8a7, one that cannot be read as a
     regular file — a dangling link or named pipe, say — counting as any
     without blocking; a member whose name holds a path is placed in a
     subdirectory, 0x5b9dbc, so is not there), and a `.dir` of the name
     that is a directory there, listed with a `*.*` wildcard (0x5c3099
     to 0x5c3102) (0x46f50f to 0x46f542, 0x46b5d9 to 0x46b699,
     0x406a29). A `.dir` that is a file or an archive member also makes
     the check false, though the original does not open it: stricter
     than need be, not a port. Only a set of one `work.iff` there is ported,
     and it must be the index's only `work.iff`, which `careers` reads
     (Sol's eleventh to fourteenth checks). Every string table of it
     (STR#, and CST, which the loader falls back to), deleted chunks
     included, must hold no `$` byte at all, since the loader's ways not
     ported may read a string's bytes as entries; each STR# resource must
     be one the loader reads as ported (with the map's flags), since one
     read otherwise may be read past its end, and not one it reads again
     for English (0x403f37 loads with the fallback on; entries, none in
     US English, 0x47006b to 0x4700d0); and every career's name and
     level strings must hold no career token.
     The install's STR# tables hold no `$`; its level strings hold one (in
     suit names such as `B004MA$b_01`) but no career token. When it is
     false (any other `Work` set, another `work.iff`, a `Careers.txt`, or
     a string that holds one) a person's career token fails loudly. A
     career `careers` lists (the file's CARR resources, as the original
     loads them at 0x403e3f to 0x403f1f) that does not parse fails the
     link first; a CARR chunk it does not list (deleted, or left out by
     the map) that does not parse clears `career_texts_end`. That the career texts all
     come from the `Work` set is **unverified**. On the Goth day the
     promotion at 15298 ("You have been promoted! (to $Job:4:5)",
     `$JobDesc:4:5`, `$TimeLocal:2`) is a person's. Tests:
     `the_installs_career_texts_hold_no_career_token` (`tests/boot.rs`),
     `a_career_token_past_the_first_character_or_the_start_of_one_at_the_end`
     (`dialog.rs`),
     `career_texts_end_only_when_work_holds_no_career_token`
     (`tests/program.rs`).

  The passes after `$Neighbor`'s are taken to change nothing but the
  text (**unverified**).

  The names (`$Me`, `$Object`, `$Neighbor`) are the type's catalog name
  (0x4573d9 to 0x457d3b): an entry of the CTSS table definition word 41
  names, read through 0x4061e3, or the definition's own name when that
  entry is empty (0x457db4). Which entry 0x4061e3 picks is not ported,
  so every entry is tried in the token's place, and the passes after it
  must agree on the portrait for all of them. The names are known only
  when the table has an entry in language 1, no entry is empty, and the
  file has no legacy CATS table, which 0x4061e3 checks first (Sol's
  reading, **unverified** here). Each token is replaced from the start
  of the text, then from one past each place replaced (0x4558a0,
  0x455750). A name that holds the token past its first character puts
  it back past each place the search goes on from, so the pass never
  ends; one that makes it with the text after it uses that text up. The
  dialog fails loudly where a name never ends, or where the names are
  not known and the text holds the token, whatever the portrait.

  Every number in a token is read from its first 15 digits (0x44cb52
  makes a string of 16 bytes for them, filled at most to 15, 0x46f78d to
  0x46f79d), then by the C runtime's 32-bit conversion (0x636029), which
  wraps, and compared with the locals' count signed (Sol's ninth check).
  The digit test (0x636993) is handed each byte sign-extended (0x44cb70).
  So a byte above 0x7f is looked up below the C runtime's table, and
  some pass as digits that the conversion reads as none (`«`, 0xab). A
  token whose digits stop at such a byte fails loudly (Sol's tenth check).
  Test: `a_tokens_digits_up_to_a_byte_above_0x7f_fail_loudly`.

  Tests: `a_dialogs_text_can_show_the_stack_objects_neighbor_instead`,
  `an_object_name_that_may_hold_a_neighbor_token_fails_loudly`,
  `neighbor_is_replaced_by_the_neighbors_name_and_may_never_end`,
  `a_token_of_a_pass_not_ported_fails_loudly`,
  `the_passes_after_neighbors_are_presentation_for_a_person`,
  `local_is_replaced_by_the_locals_value`,
  `me_is_replaced_by_the_running_objects_name_before_object_is`,
  `object_is_replaced_again_from_one_past_each_place`,
  `a_neighbor_local_named_in_a_dialogs_text_fails_loudly`,
  `a_neighbor_local_the_original_left_unknown_fails_loudly`.
- **Other builders.** Only the dialog's builds are counted. The household
  panel draws the family's portraits, so a portrait of a family member's
  type with nobody of it on the lot fails loudly. A type made while the
  game ran (`World::characters`) fails loudly too, since the table may
  not have it. The relationship panel and other interface builders are
  not counted. In the recording nothing else built the cat's image.

## The view's centre at 24719, and the stand-in

At 24719 the Goths' cat portrait is built with nobody of its type on the
lot, so the search runs. Of the 3,844 tiles it may start from (inside the
border of a 64-tile lot), four find no tile: 19,24; 23,31; 26,28 and
26,31, each in room -5 (none). From every other start a tile is found,
and the build leaves the same lot. The fitted-frame test checks this
(`every_start_gives_one_state`). At 24718 it saves the lot, steps 24719
once from each start, and checks two things: the four starts stop where
the original would remove the cat unput, and all the others reach one
state hash.
So the start does not matter outside those four cells.

The recording's state shows a single 1,920-draw build at 24719 and no
second one. That does not rule out the four cells. The cat's type
(Waggles, neighbor 328, 0xDC7419A2) has image 2002 (`BMP_` 2002, "faces",
in `User00036.iff`; checked in the guest's and the install's copies). So
from a start that finds no tile the original makes the cat once (1,920
draws), removes it unput (0x4e9706), and does not build again (0x4e938c).
The recording's draws are the same either way, so it does not say where
the camera was. A start on one of the four cells takes that branch, which
is not ported, and stops loudly.

Without the input, a boot of the Goths stops at 24719, as the adoption
dialog at 17707 does without `--answers`. The Goth day's tests give the
lot's middle tile, 32,32 (`GOTH_VIEW_CENTRE`). It is labelled in the code,
the tests and here as **a stand-in start tile, not the original's
camera**. (An earlier draft of the label said the recording showed the
camera was not on a no-room cell. That was wrong, for the reason above.)

The input is the tile the search starts from, not the view's pick. By a
reading of the disassembly (Sol's, **unverified** here), the interface
checks the pick against the view's bounds with a fallback (0x434891),
undoes the view's rotation (0x43491c) and clamps it before the spiral.
None of that is ported, so `--view-centre` and `Lot::set_view_centre`
take the tile after it. `Lot::load_viewed` takes it before the load, so
the load's own steps see it too.

Open items:

- read the house view's centre from a capture (scroll, zoom, rotation);
- port the screen-centre pick (0x4a7b58), with its bounds check and
  fallback (0x434891), the rotation undone (0x43491c) and the clamp;
- port the no-tile branch (0x4e9706, 0x4e938c: no second build for a type
  with image 2002);
- both tie to the open question of which camera the original opens the
  Goths' house with.

## The cat at 24719 was the portrait

`READ_BEFORE_THE_FIRST_LOAD` lists the kat file, because each cat made
takes one skeleton's worth of draws. Its doc used to count 24719 among
the cats made, from the draws alone. That was the portrait, and the doc
now says so. The dog file is listed too (#275), and the dog the Goths'
lot makes at 24766 draws 1,776, as in the recording.

## The rest of the day

Under the fitted frames, the C runtime stream matches the recording at
every update to 43200. The lot stream matches through 36717. At a few
updates after that (36718, 36929, 37058, 37126, 37215, 41445, 41858 to
41892, 42724, 42745, 42948 and 43003) it moves one update before or after
the recording's, and it matches again at the next update. Each time it is
the same draws a frame early or late, which is taken to be the recording's
frames drifting against the fitted schedule. **Unverified.**

Objects match the recording at 24716, 24718 to 24720, 24765 to 24767 and
24800. The only difference is the global stream, which is the censor
mosaic (`20261118-goth-day-global-draws-are-censor-mosaic.md`). The
fitted-frame test now checks the streams at 24716, 24719, 24766, 36717
and 43200.

## Effect on the pins

`World::portraits` is a new field at the end, so the snapshot format goes
to 31 and every pin was taken again. Each pinned test also checks that the
format-30 encoding equals `main`'s pin. That holds at every pin of the
default-frames Goth day and the Newbies, so those runs are unchanged apart
from the new field. In particular, no person is made for a portrait on
those runs: each type they build has someone on the lot.
