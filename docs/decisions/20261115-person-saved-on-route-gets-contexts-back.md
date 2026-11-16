# A person saved part way through a route gets its route contexts back empty

*Decided 2026-11-15.*

The Pleasants stopped at tick 1: person 117 was saved walking, and the
route's steps found no context to work on. The save keeps only how many
route contexts a person had (0x460443). The load makes that many, each
a copy of one its default constructor makes (0x4815f1: the slot record's
defaults from 0x46dd48, then 0x481693 with no person, no object and no
slot), and "reads" each with a function that reads nothing (0x5fe4ae,
`ret 8`). `crate::routing::slot::loaded_context` is that context. Two
of its words (+0x74 and +0x90) are what the stack held; they are 0
here, and nothing ported reads them on a context that names no object.
A negative count fails, as the original's resize would erase before the
vector's start.

The route's steps (0x45ad63) then take two exits that were refusals:

- **0x45ae71**: the last context names no object (+0x48 is 0). The
  frame's step goes back to 0, the context is popped (0x45f8d9) and the
  thread yields. The next run starts the route again, on top of any
  context left.
- **0x45ae65**: at step 0 with the stack object gone or off the lot, or
  past step 0 with no context, the run gives false and writes nothing.

The person reset no longer refuses a person with a saved route count:
the count is now contexts, which the reset erases (0x45e8aa) without
reading them. The tick's route usage sample reads the contexts alone.

**Against the captures.** The Pleasants (`house06-pleasants-ticks-20261115`)
and the Roomies (`house10-roomies-ticks-20261115`) still stop at the
load, on persons 117 and 176, whom the load does not reset: the
animator restore fails loudly for such a person, whose animator keeps
what the restore read. That live restore is next. An earlier build that
assumed the reset for the Pleasants, with these contexts, ran them 933
ticks, so these contexts are what lets person 117's route start again
at update 1 (the stop at 0x45ae65 that the house picks recorded).

**Strict runs**: no Goth or Newbie person was saved on a route, so
their states and every pin are unchanged.
