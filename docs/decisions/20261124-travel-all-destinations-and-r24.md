# Departure tokens for every destination, and routine 24

*Decided 2026-11-24.*

Before this, only Downtown departures wrote the travelers' tokens
(Generic Sims Call 17, 0x45a22d). Every other destination was reached by
the API alone, with no tokens. Also, a trip a script asked for through
routine 17 could only use the parties the API allows, so a phone
plugin's lone trip to Vacation Island was refused. Routine 24, the
outfit editor, was refused outright.

## Routine 17 on every destination

Each destination's travel tree passes routine 17 a house, an arrival
hour and minute, and a mode. The values below were read from the trees
(lifted with `sims-antics lift`). 0x45a785 then adds the hour and minute
to the clock. For houses 40 to 49 asked for from a residential lot, it
takes them as the time itself instead.

| Destination | Tree, node | Hour, minute | Mode |
|---|---|---|---|
| Downtown (phone) | 0x1004, 13 | 0, 30 | 0 |
| Vacation Island (phone) | 0x1004, 12 | 14, random | dialog choice (attr 8) |
| Old Town (phone) | 0x1004, 13 | 0, 15 | dialog choice (attr 8) |
| Studio Town (phone) | 0x1004, 12 | 0, 30 | 0 |
| Magic Town (phone) | 0x1004, 12 | 14, random | 1 |
| Magic Town (hole) | 0x1001, 6 | 14, random | 0 |
| Magic Town (beanstalk, house 99) | 0x1004 | 14, random | 0 |

"Random" is one Random Number draw below 41 on the lot stream. A draw
of 20 or more becomes 13:(draw − 20), and a smaller one 14:(draw + 40),
so arrivals land on 13:00–13:20 or 14:40–14:59. Every return tree
passes house −1. Downtown's, Studio Town's and Magic Town's pass 0, 30.
Vacation Island's mode is 1 exactly when person word 77 ≥ 0. Old Town's
mode is 1 when global 9 is not 0.

Decisions:

- `depart_to_destination` now runs routine 17 (`request_trip`) for
  every party with a native arrival (`ArrivalFamily::Solo` or `Family`),
  with that destination's arrival time. It draws on the frozen home
  lot's stream exactly where the tree would. Group trips write nothing,
  as before: `HouseholdGroup`, and Magic Town's residential lots 90–92,
  where 0x4f1ae9 makes no arrival. The destination boot leaves restoring
  the motives to the arrival marker's routine 21 for every
  non-Group arrival.
- `depart_from_request` validates the party with `PartyRules::Native`.
  0x45a22d checks only that the caller is a person, and the tree decides
  who goes. So mode 0 (one Sim, or a date when the leader holds a kind-1
  token) and mode 1 (the whole family) are accepted for every
  destination. The API's per-destination rules (`PartyRules::Api`) still
  apply to `depart_to_destination` and `validate_travel_plan`.
- A scripted trip whose arrival marker cannot take its party fares as it
  would natively. The Studio Town and Magic Town markers (tree 0x100e)
  call no routine 19, so a date's companion never comes. Studio Town's
  marker places only family member 0, but no native tree sends a whole
  family there.
- Mode 0 takes along whoever the leader's first kind-1 token names
  (0x45a34b). An API departure of a lone traveler who still holds such a
  token would rewrite the saved state of a Sim who is not going. So when
  routine 17 writes any inventory outside the validated party,
  `depart_to_destination` refuses with `InvalidParty`. Nothing has been
  committed at that point. A whole family is refused the same way when
  routine 17, which reads the live household (0x45a3ef), skips a member
  the saved household has.

## Routine 24, the outfit editor (0x44ff6e)

1. The object is cast to a person (0x635abe). If it is not one, error
   0x1c (0x44ff8b).
2. The editor is built (0x408b3b) and person word 8 (+0x59c) is saved.
   The character's clothing rows are loaded (0x409315, which is
   `outfits::init_clothing_rows`).
3. If temp 1 is negative (0x44ffd0), nothing is set. Otherwise temp 0
   picks a category (0x44ffda). If temp 1 is below the category's count,
   the current place is read and kept, and place temp 1 is set without a
   commit (set(temp 1, 0)):

   | Category | Count / get / set | Slot → row | List |
   |---|---|---|---|
   | 0 body | 0x40d077 / 0x40d0a4 / 0x40d10b | 2 → 1 | 0x408bf5(0) |
   | 1 formal | 0x40d4ff / 0x40d52f / 0x40d599 | 0x1f → 30 | 0x408d8e |
   | 2 swimwear | 0x40d817 / 0x40d847 / 0x40d8b1 | 0x20 → 31 | 0x408e7b |
   | 3 pajamas | 0x40db2f / 0x40db5f / 0x40dbc9 | 0x21 → 32 | 0x408f68 |
   | 4 winter | 0x40de47 / 0x40de77 / 0x40dee1 | 0x22 → 33 | 0x409055 |
   | 5 high fashion | 0x40e15f / 0x40e18f / 0x40e1f9 | 0x23 → 34 | 0x409142 |

4. The commit (0x450154) runs 0x409492(1), which runs the check
   0x40a588 before writing the rows back (0x587aec, 0x587bbd). For pets
   (gender 2–5), rows 2 and 30–34 become "NONE" (0x40a5d5–0x40a619). For
   humans, a missing head or body falls back to its list's first entry
   (0x40abce, 0x40ac34). If either list is empty, rows 1, 2, 17, 18, 15
   and 16 are cleared (0x40ac57). The clothing rows are then validated
   (0x40acbc–0x40af1c).
5. Word 8 is restored. The portraits are refreshed (0x4572af →
   0x4e90c9, 0x4eb0cc → 0x4e897e) and the interface is flagged
   (0x4ceceb(0x20000000)). Both are presentation only.
6. Temp 1 becomes the kept place, and the routine branches true
   (0x450656 → 0x44f7f0).

Body and head lists: 0x408bf5 takes the gender from row 12 (0x40ccac).
Humans (0–1) get the human list. Dogs (2–3) get 0x6e488c and cats (4–5)
0x6e4888, except that the head list (arg 1) is empty for pets. Any other
gender gets no list. The clothing lists' slot is
(gender ≠ 0) + 2·child (0x40cbdd), built by 0x4098f1.

Deviations:

- When nothing is set, native temp 1 gets the low word of the frame
  pointer. The port leaves temp 1 as it was. Only a clothing rack's
  trees read temp 1 back, so a rack given a place out of range stores
  garbage natively and its old value in the port.
- A pet asked for a clothing category (1–5) would read a null list
  natively. The port refuses it as `Unsupported` before changing
  anything. An unknown gender is refused the same way, since its commit
  reads a null body list (0x40d0a4).
- Routines 29 (routine 24 on the stack object), 30 and 31 (0x450528,
  0x4503f2) are not ported. They stay in the residuals.
- The native commit writes the STR# back into the character's file
  (0x587bbd), so another lot sees the change. The port writes the rows
  to `World::made_outfits`, which belongs to the lot, as the load's own
  clothing commit already did. Carrying the edited rows to the
  neighborhood's character data is a follow-up.
