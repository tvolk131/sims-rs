# One sims-vm test binary, the install cache in travel, a tick-level fuzzer, and retiring legacy sims-runtime

*Decided 2026-11-25.*

This change does four things in `sims-vm` and `legacy/`. It changes no
simulation behaviour, and the Goth `TICK_43200` pin is unchanged.

## One integration test binary

The fourteen files in `crates/sims-vm/tests/*.rs` were fourteen test
binaries, and each linked the whole crate graph. They are now one binary,
`tests/integration/main.rs`, with one module per former file (moved with
`git mv`, so history follows) and a shared `common` module.
`tests/interpreter` stays its own binary, because it needs no game data.

`common::base` builds the install's index (with the `UserData`
neighborhood's characters), the catalog and the program linked over the
whole catalog once per process, in a `OnceLock`. Only values no test
changes are shared. A test that needs to change the program clones it,
and a lot, a world or a store is never shared. `resize` and `program` use
it today. The other modules boot lots, which build their own state through
the crate's public API, so caching for them would mean changing what they
test.

The modules already ran side by side, because cargo runs a binary's tests
on threads: temporary names include the module, the process and a counter,
and no test changes the environment or the working directory.

The `--list` count is unchanged at 121 for the moved tests (118 pass,
3 ignored, as before). This change adds four tests, so the binary now
lists 125.

| | Binaries | Size on disk | Release run time |
| --- | --- | --- | --- |
| Before | 14 | 100.1 MiB | 87 s, one binary after another |
| After (moved tests only) | 1 | 15.0 MiB | 44.8 s |
| After, with the 4 new tests | 1 | 15.1 MiB | 26 to 67 s |

The after times vary with load on the machine: runs of the binary took
from 26 s on a quiet machine to 67 s with many other builds running.

The run time falls because one process runs every suite's tests on all
cores, rather than one binary's at a time. Building also links one binary
rather than fourteen.

## The install cache in travel

`travel.rs` read the install again at every transition, using its own
private `StoreRuntime`. Each public transition now has a twin with
`_with_cache` added to its name that takes a `VmInstallCache`:

- `depart_to_destination`, `depart_from_request`, `depart_from_request_with`
- `resume_active_trip`, `abort_active_trip`
- `return_from_destination`, `return_from_request`
- `travel_on_from_request`
- `open_destination_for_edit`

The existing signatures don't change. Both forms go through one private
function that takes a `Base`, which is either `Root(path)` or
`Cache(cache)`. A cached splice goes through the lifecycle's
`from_snapshot_with_cache`, by way of `VmInstallCache::transition`. It then
sets what travel's own runtime sets beyond the lifecycle's: the program's
expansions, and the ordered wardrobe. The cache builds the wardrobe once,
when it is first needed. A cached transition also checks a destination's
expansion against the cache's install, the one it boots from, rather than
scanning the install again. The cache holds the inputs it read when it
was loaded, and the wardrobe from its first use. Other files, such as a
destination's description, are still read from disk, so the cache must be
loaded again after the install changes.

The test `travel::cached_travel_matches_uncached_bit_for_bit` runs the same
Studio Town trip twice: enter the lot, depart, drop and resume, play at the
destination, return home, then open the destination for edit. One run is
uncached and the other cached, each against its own copy of one imported
store. It compares:

- each booted lot's `World::hash()`, and a SHA-256 of the `Debug` form of
  its linked program and its catalog's objects. That form is
  deterministic, but two NaNs with different payloads print alike. The
  catalog's diagnostics are left out, because the cached splice lists them
  in another order;
- the neighborhood's character files after the departure and the return;
- the trip the store holds after the departure, as a resume reads it;
- the sessions, the return report, the stored neighborhood and the final
  saved home and destination lots.

The trip and store IDs are blanked first: the trip ID is a process-wide
counter, and each REDB file has its own store ID. Everything else is
identical.

Warm timings in release on the development Mac, over several runs.
Loading the cache takes about 0.2 s once.

| Transition | Uncached | Cached |
| --- | --- | --- |
| Depart | 0.49 to 0.52 s | 0.26 to 0.29 s |
| Resume | 0.49 to 0.74 s | 0.17 to 0.19 s |
| Return | 0.37 to 0.39 s | 0.08 s |
| Open for edit | 0.47 to 0.65 s | 0.17 s |

`sims-game` keeps calling the uncached forms. Moving it to the cached forms
is a follow-up for that crate.

## A tick-level determinism fuzzer

The test `fuzz::seeded_commands_replay_across_warm_forks_and_cold_restores`
ports the legacy exploration fuzzer
(`legacy-final:crates/sims-runtime/src/exploration.rs`) to the tick level.
`crates/sims-sim/tests/integration/state_fuzzer.rs` already covers the state layer.
This test maps the legacy `Driver` onto `sims-vm`:

| Legacy | Port |
| --- | --- |
| candidates and execute | a seeded SplitMix64 choice of `sims_net::protocol::Command` from the live lot (a pie-menu pick, a cancel, speed, pause, free will, selection, camera, a dialog's answer, or nothing), run by `Lot::apply_coop_tick_batch`. A dialog's answer is one of the buttons it offers; a special panel's is its cancel or dismissal |
| observe | after every tick: the tick count, `World::hash()`, the co-op world hash, the session's combined `StateFingerprint`, the two game RNG streams and the CRT stream, and each command's outcome (applied, refused, ignored and so on) |
| checkpoint and restore | `Lot::coop_snapshot_bytes` (`Lot::save` and `Saved::encode`), then `Lot::restore_coop_snapshot` (`Saved::decode` and `Lot::restore`) on a second lot |
| fork | `Lot::fork_coop` takes an opaque `CoopFork` (the reference's in-memory `Saved` and a clone of its co-op context), and `Lot::restore_coop_fork` goes on from it |

`Lot` stays without `Clone`. A Buy cursor token names one lot, and a lot's
store origin fences one save, so a cloned lot would break both. The warm
fork is an in-memory restore instead, and its world shares copy-on-write
storage with the reference's. `Lot::fork_coop` and `Lot::restore_coop_fork`
are new. The restore installs the cloned context without
`attach_coop_context`'s initialisation, which
applies the session's requested speed and selections to the world. On a
residential lot nobody holds the speed of, the idle loop runs at 1000 while
the session's requested speed stands, so an attach would change a valid
lot. `fuzz::a_warm_fork_keeps_a_speed_the_session_did_not_ask_for` covers
this. `Lot::coop_fork_blocker` decides when a fork is refused, and both
the API and the fuzzer use it. It refuses a lot that is halted, waits on a
dialog or special panel, or has an active Buy or Build cursor. The waiting
case covers the lot itself and its context, including a dialog answered
while paused whose interrupted tick has not run yet. An in-memory save
keeps neither what the lot waits on nor a cursor, and only the snapshot
path asks an open dialog again. The `CoopFork` is taken from the source lot
as a whole, so a caller can't pair a save with a context that hides a
pending prompt. The warm boundary is drawn from the boundaries the lot
allows a fork at, and a run that never takes its warm fork fails. When the
drawn run itself failed, the report also gives that failure, in case the
replay doesn't repeat it.

The test keeps what the legacy fuzzer required:

- weighted choice with rejection sampling and no modulo bias;
- several cold-restore boundaries, deduplicated, and every one required;
- a per-tick trace, with a failure naming the first step and tick that
  differ;
- the journal written before each step runs;
- caught panics;
- a journal that replays without the RNG and traces what the drawn run
  traced;
- delta-debugging minimisation. A shorter journal only counts if it fails
  the same way (a divergence, a script error, a panic and so on), and the
  report keeps the original failure. So a divergence can't shrink into a
  script error caused by cutting the step that queued an interaction.

It also checks that a branch leaves its parent unchanged: the reference's
snapshot bytes are the same after the branch has run, so nothing wrote
through shared storage. A run with branches must trace what a run with
none did.

The default is 3 seeds of 20 steps, which takes about 5 to 6 s in release.
`SIMS_FUZZ_SEEDS`, `SIMS_FUZZ_STEPS` and `SIMS_FUZZ_SEED_BASE` choose more,
longer or other runs. Lots step strictly, as the shell's co-op does, so a
script error fails the run. `SIMS_FUZZ_PLAY=1` steps in play mode instead,
where the game resets the faulting object, to look for divergence past
known script errors. `fuzz::the_fuzzer_reports_and_shrinks_a_failure`
tests the reporting and shrinking on a synthetic check, with no install.

### Results of longer sweeps

Determinism held wherever a sweep ran: 16 seeds of 40 steps in play mode
(seeds 100 to 115), and strict single seeds 101 to 106. These sweeps were
run before and after the stricter trace (state fingerprint, command
outcomes) and the valid dialog answers were added, with the same results.

Strict seeds 100, 107 and 108 stopped at script errors that the default
seeds don't reach. These are follow-ups, not divergences: the error happens
in the reference run, before any branch is compared.

- Seed 0x64 shrinks to two steps: `SelectPerson(18)`, two idle ticks, then
  a pick for person 16. It fails with "object 16, tree 0x2010 node 3: Go To
  Routing Slot has no stack object (native error 0x34)". Seed 0x6c fails
  the same way on person 18 at tick 2. A pick queued for a person other
  than the selected one seems to reach Go To Routing Slot without its stack
  object.
- Seed 0x6b fails at tick 14 with "object 259, tree 0x1016 node 13:
  missing object (native error 0x0a)".

## The fixture_replay comparator, recorded

`legacy-final:crates/sims-runtime/src/fixture_replay.rs` held the skeleton
of an oracle comparator. The oracle comparison is still done by private
scripts outside the repository, so there is nothing in the repository to
absorb it into. This entry records its contract instead, for the in-repo
comparator that replaces those scripts.

The driver (`:30-47`) is
`Driver { tick(), observe() -> Value, input(&InputEvent) -> Boundary, poll() -> Poll }`.
`Poll` is `CompletedTick`, `PartialPass` or `Boundary(Boundary)`.

The comparator works in two phases. Errors in the first phase are plain
errors. Only the second phase produces a report with an outcome.

**Preparing a case** (`Prepared::load`, `:105-210`) checks the reference
before anything runs. Any failure here is an error, before a report exists:

- a supported schema, observation contract and resume contract, and valid
  limits;
- exactly one initial reference, which must be a resumable state, not an
  observation, with an explicit tick;
- a trace within the observation and completed-tick limits, made only of
  observations with explicit ticks, starting at the initial boundary;
- completed ticks that advance by exactly 1 with no gap, and other
  boundaries that do not advance time.

**Running a case** (`:219-397`) first checks that the driver's
observation and resume contracts are the ones the case was prepared with.
A mismatch is an error, before a report exists. It then restores a driver
from the initial reference (a failed restore gives an `Incomplete` report
with no driver). It then works through a schedule of `Input`, `WaitCompletedTicks`
and `WaitBoundary` steps, compares the observation at every boundary with
the next row of the reference trace, and keeps these invariants:

1. An input must not advance time, and must not claim a completed tick.
2. A partial pass (a registry pass that does not finish a tick) must not
   advance time.
3. A completed tick advances the tick by exactly 1.
4. A service boundary must not advance time, and must not claim a completed
   tick.
5. Poll and completed-tick limits apply while running. Exceeding one makes
   the outcome `Incomplete`, not a pass. (The observation limit is checked
   on the reference when the case is prepared.)
6. A gap in the reference's completed ticks is rejected when the case is
   prepared (`:187`), as an error. It is never turned into a wait.
7. If the run doesn't agree, the driver is returned for diagnosis when one
   was restored, and its observation at the stop is reported when
   `observe` succeeds. A failed restore leaves no driver and no tick to
   report, and the outcome stays `Incomplete`.
8. `first_difference(expected, actual)` (`:407-445`) walks two JSON values
   and returns the first difference as an RFC 6901 pointer, with the
   expected and actual values. Object keys are visited in sorted order,
   with `~` and `/` escaped as `~0` and `~1`. Arrays are compared index by
   index up to the longer length. A missing value (`None`) stays distinct
   from JSON `null`. A difference in the boundary itself is reported under
   `/boundary`.
9. The outcome is `Agreement`, `Diverged` or `Incomplete`. `Agreement` also
   requires every reference observation to have been compared.

The synthetic tests in `fixture_replay/tests.rs:191-420` exercise each of
these. The fixture-store and qualification parts (`sims_fixtures` blobs,
contracts, manifests) are dropped, as the vision drops evidence tooling.
The tick fuzzer's `first_difference` follows the same idea for its
traces: it names the first step and tick whose observation differs.

## Retiring legacy sims-runtime

With both of its remaining items done (the fuzzer ported, the comparator's
contract recorded here), the mining audit marks `legacy/crates/sims-runtime`
DELETE-SAFE. It is removed with `git rm -r`: 428 files and 129,029 lines.
Its files are byte-identical at `legacy-final`, where it is read as
`legacy-final:crates/sims-runtime/<path>`. Legacy `sims-app` still names
the crate, but legacy crates are not built. `legacy/fixtures/suites/` held
manifests for the crate's conformance runner and now has no reader. It is
left for a later cleanup.

## What would reopen this

- A failure in a determinism sweep: a warm branch and a cold one that trace
  differently.
- An in-repo oracle comparator. It should carry the invariants above, and
  this entry's list should then point to it.
- A test that needs the shared base to be mutable. It should clone the base
  rather than change the cache.
