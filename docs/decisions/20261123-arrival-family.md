# Travel arrival: the traveler leads the destination's family

*Decided 2026-11-23.*

After a Magic Hole trip, and on every Downtown trip, the arriving Sims were
put beside the arrival portal. The middle ped marker then moved them again,
and some ticks later their walk jumped them back to the portal. The marker
also never placed a Sim who traveled without the household's first member.
Both came from the port's arrival, not from the marker:

- **The portal placement is invented.** The original makes each traveler
  with the person factory (0x4e9719 → 0x440b2d), which leaves the person
  off the lot. Only the middle ped marker's Find Location puts it on the
  lot. Our walk went on from the portal tile, so after the marker's put it
  jumped back.
- **The destination's family was always the whole household.** The
  marker's Generic Sims Call 18 names member 0 of the family the game
  holds (`game +0x10`, read by 0x4e97dc(0)). On a solo trip the original
  makes that family the traveler alone.

## The native flow

- Generic Sims Call 17 (0x450660 → 0x45a22d) asks for a trip with temps
  (house, hour, minute, mode).
  - Mode 0 calls 0x4fc713(house, the person's GUID, 0).
  - Mode 1 calls 0x4fc713(house, 0, the person's family).
  - 0x4fc713 stores the house, GUID and family at +0x132, +0x134 and +0x138.
- 0x4fc59b turns those into a flag, then loads the lot through 0x4f1ae9:
  - Downtown (21–30) always sets +0x140.
  - Vacation, Magic Town and zoning 2: a GUID sets +0x140, a family +0x141.
  - Studio Town: a GUID sets +0x142, a family +0x141.
- The lot load 0x428041 picks the house's family (0x425d95), else the
  family numbered 0 (0x42827c). Global 9 becomes its number (0x42830d).
  Funds are 0 when that number is 0 (0x428328).
- With any of the three flags set, edit mode is skipped
  (0x4f1b7c..0x4f1b9d jump to 0x4f1c32).
- **Solo** (+0x140 or +0x142, 0x4f1c87..0x4f1cec):
  - family 0's members are erased and the traveler added (0x410a41 →
    0x4d3fe6, 0x410849), so the traveler is member 0;
  - the traveler is made by 0x4e9719 (off the lot);
  - the funds become the traveler's own family's +0x118
    (0x4f1cc0..0x4f1ce7);
  - global 9 stays 0.
- **Family** (+0x141, 0x4f21bd..):
  - the game holds the home family and global 9 becomes its number
    (0x4f2211..0x4f222b);
  - every member is made by 0x4e9719 in member order (0x4f2253 loop).

## What was decided

- **A trip records its arrival family** (`ArrivalFamily`):
  - Single and Date → `Solo`;
  - WholeFamily → `Family`;
  - HouseholdGroup → `Group`, which no native mode makes.
- **Solo:** the destination loads with family number 0, then family 0 is
  made the traveler alone and given the home family's funds less the fare.
  Global 9 stays 0.
- **Family:** the destination loads with the home family, as before. The
  travelers are made in the family's member order.
- **Solo and Family travelers stay off the lot.** The middle ped marker
  places them, so the marker's placement is the only jump.
- **HouseholdGroup keeps its old behaviour.** The game holds the home
  family, global 9 names it, and the travelers are put beside the arrival
  portal. The marker then does what it did before. Keeping the old path
  was chosen over mapping it to Family, which would make members who did
  not travel.
- **Find Location mode 0 refreshes a person after the put** (the object's
  virtual 0x74 at 0x44b66e, which for a person is 0x45e519). This clears
  the walk position.
- **Readers of "the controlled family" with global 9 at 0 now read family
  0** from the World, as `game +0x10` would be:
  - the visitor pass, autonomy's free-will and first-member checks
    (0x45dd87, 0x45ddb0), Generic Sims Call 18's first member, and the
    reselect scan (0x445501);
  - co-op's interpreters, which pass no fixed family when the World keeps
    family 0;
  - the HUD's roster (0x4e97dc).
- **Funds round-trip unchanged:** the destination's funds become the home
  family's budget on return. A purchase made on a solo visit is charged to
  those funds and comes home.
- **The shop and edit mode use the household, not global 9.** A solo visit
  is played, not edited, and the shop opens.
- **The travel envelope goes to version 3.** The arrival family follows
  the payload. Older builds refuse a version 3 trip. Versions 1 and 2 still
  load:
  - a date, or one traveler from a household of more than one, is `Solo`;
  - every household member traveling is `Family`;
  - anything else is `Group`.
- Saving while away is still refused, and co-op is still refused on a
  travel lot, so no other persisted shape changes.

## Not done

- Routine 17's kind-2, 7 and 8 tokens (saved motives, the arrival clock).
- The date's companion is still put on the destination by the departure,
  rather than made by routine 19 from the token alone.
- A family arrival makes only the members who traveled; the original makes
  every member.
