# Trip requests: cancel, transport, answers, the trip home; CAF modes

*Decided 2026-11-24.*

Four sims-vm follow-ups from the routine 17 trip wiring (#431), and one
from the Neighborhood.iff writer (#429). Trees were read with
`sims-antics lift`; addresses are in the original `Sims.exe`.

## Cancelling a request

The original has no refusal: once routine 17 (0x450660 → 0x45a22d)
records a request, the main loop always leaves the house (0x4fc713).
The shell can decline one, so `Lot::cancel_trip_request` drops
`World::trip_request`. As an abort does, it takes back the kind-2
tokens (saved state and arrival clock) routine 17 wrote for the request's
`saved` neighbors. The script's tail then goes on on the same lot and
restores nothing. A companion's kind-1 token the script wrote stays.
The Sim's next routine 17 asks again.

The arrival clock can land outside `saved`. In mode 1, when member 0 is
not on the lot, routine 17 writes the clock into the caller's inventory,
and a caller missing from its family list has no saved
state written. `saved` keeps its meaning: whose saved state was written.
That list decides the party check and the arrival's motives. A cancel
and a trip home therefore also erase the clock tokens, and only those
(kind 2, GUIDs 7 and 8), from the caller's inventory.

## The shell's transport, answers and frames

A request names only the house and the party. `depart_from_request_with`
takes a `TripDeparture` with the transport the lot picker chose, the
destination's dialog answers, its frame schedule and its first camera
centre. The transport must serve the destination. Otherwise the call
returns `IncompatibleTransport` and the request stays. The fare stays 0,
since the plugin's own Alter Budget pays it. `depart_from_request` keeps
its behavior: the default transport, answers `[3]` and the default
frames.

## The trip home a script asks for

Each phone plugin's tree 0x1004 serves both directions. On a destination
(call 320/424 true), the Go Downtown plugin runs n58 temp1 = 0,
n59 temp2 = 30, n92 temp3 = 0, n71 temp0 = −1, then n13 routine 17.
Vacation's version is n43–n112 (14:random), then n117 (mode 0 when
person word 77 < 0, else 1), then n51 temp0 = −1, then n12. 0x45a22d
turns house −1 into the house of the leader's family, with companion
kind 3.

The house then reloads with the caller's stack frozen just after
routine 17, and the tail runs:

- Go Downtown: n47 routine 23 (0x450b2b; the clock goes to locals 5 and
  6, which nothing reads), then n15/n18 routine 21 (0x45098a) on self,
  then the cab and "find good location for me", then n14 routine 20
  (0x45087d) on self, then n90 routine 21 on the companion when temp0 > 0.
- Vacation: n33 routine 23, then n13 routine 21 on self, then n30/n26,
  then (attr 8 = 1) a wait for the family, then n81/n79 routine 20 on
  self, then n89 routine 21 on the companion.

Routine 20 runs in either mode, on the caller's inventory.

The loop that would move the home clock to the arrival time (Go Downtown
n51–n56, calls 366/367; Vacation n37–n42) cannot be reached from the
tail. So natively the home clock stays where it froze, as the port's
restore leaves it.

`return_from_request` carries out such a request when its house is the
trip's home house and its caller is one of the trip's travelers. Both
are checked against the trip persisted in REDB, not against the session
the shell hands in. It
returns as `return_from_destination` does, which places the travelers
from their live state, since the curb placement drops the frozen stack
the tail needs. It then leaves the tokens as the tail would: every kind-2
token the request saved is erased, and one is taken from the caller's
first kind-3 token. The request is cleared once the return succeeds.

Deviations:

- A companion the kind-3 token names who did not travel is not made on
  the home lot. Natively, routine 20 makes it.
- A request for another house is refused (`SessionMismatch`) and left
  for the shell to cancel. That is a destination's Go Somewhere Else,
  tree 0x1016, routine 17 (lot, 0, 30, 0). Destination-to-destination
  trips are not carried out.

## CAF person words for negative modes

The shared NBRS serializer 0x423e08 writes a neighbor's person words
whenever its mode is nonzero: `cmp [esi+0x18], 0; je 0x423ecb` at
0x423e8e. Version 8 and later write 0x100 words (0x423e99). `encode_nbrs`
tested `> 0`, so a negative mode lost its words. It now tests `!= 0`, as
the sims-formats parser already does. The CAF import still refuses a
negative mode (`TemplateInvalid`). That is the field-revision table's
policy and is unchanged.
