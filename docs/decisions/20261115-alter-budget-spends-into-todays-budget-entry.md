# Alter Budget spends into today's budget entry; a thread's Add/Change Action String names the running action

*Decided 2026-11-15.*

**Unverified.** No capture covers the snack order.

Alter Budget (0x44eade) now spends: when bit 0 of byte 4 is clear and the
amount is covered, it calls 0x46d91c with the category and the amount.
That routine does nothing in budget mode 1 (global 32). In any other mode
it takes the amount from the funds (simulator +0x90), adds it to the
category's word of today's entry (+0x184), both in 32 bits that wrap, and
writes globals 14 and 13 as the funds setter (0x46d8dc) does: the funds
split at 10,000 by signed division, cut to 16 bits. The setter writes only
in mode 0; this writes in mode 2 too. Then it sends signal 0xda with the
funds (0x40760d).

Today's entry is the first of `Budget::days`. The simulator keeps six days of
eight words: today at +0x184, yesterday at +0xe4, and the four before it
at +0x104 to +0x164. The load (0x46d1e4) reads the save's first day into
+0x184 and the next five into +0xe4 on, so the save's order is the one
`days` already had. Per day the save holds a flag word, then eight words
if the flag is not 0, or seven if it is (0x46d270). The game always writes
the flag as 1, since the same code writes and reads (0x464970). The
parser reads no words for a flag of 0, so a today with flag 0 fails
loudly. So do category 8, which the primitive allows (0x44eb10) but which
adds to +0x1a4, the head of a list that the constructor allocates
(0x46ce0c), and a lot with no days read from a save. Each fails before
anything changes. The Goth save's days are today
[20, 0, 0, 0, 0, 0, 15060, 33654] and five days of zeros.

Signal 0xda is presentation. The game has eight subscribers (the
constructors that call 0x407577): the lot (0x41637f), 0x43c0e4, 0x44302e,
the simulator (0x46d2c2), the architecture (0x47b22a), the house viewer
(0x4a82db), the panels (0x4c3ea5) and the interface (0x4f65bf). Only the
interface has a case for 0xda. It marks the control panel's funds with
0x4000 and redraws it (0x4ce41b), which formats the funds as text
(0x4ce81c, 0x4cecf5). The lot's handler first passes the signal to lot
+0x10, but by a direct call to the empty base handler (0x41639e calls
0x407607, which returns 0), so it does nothing for any signal. That was
unknown before (see `tick.rs`).

The day's turn (0x46d417) is still not ported. When it is: it moves the
days with a forward copy that overlaps, so +0x104 to +0x164 all end up
with yesterday's old words before today moves to +0xe4 and is cleared.
A save of version below 0x34 is reset after it is read (0x46d211 calls
0x46d97a: every day cleared, and global 28, the day count, set to 0). The
load does not do that yet. It matters only for the base game's version
0x33 template houses.

Add/Change Action String (0x44cc07) in a thread's run now names the
person's running action instead of failing. Outside a pie menu's listing
it checks the byte 0x6c4d6c, which starts as 1. Only the synchronous run
(0x4749b5) changes it: it clears it for the run and puts it back after
(0x474a37, 0x474ad0). With the byte set and the object a person
(0x635abe), 0x41f1ae stores the string as the action's name (+0x28) and
sets its flag 0x80 (+0x40). As before for 0x41f1ae, only the flag is
kept. In every run, the string is looked up (0x4704fa) and formatted
(0x44bf1a) before 0x44cc9a asks where it goes. A formatting pass can
raise an error; one raises native error 0x1c for a non-person
(0x44c57b). So a missing string, or one with a "$" token, now fails
loudly in every run, autonomy's check trees included. Before, it failed
only in a pie menu's listing (a Sol review finding). A string without
tokens is not formatted: only the name would hold the result. Another
object's thread does only the lookup. Autonomy's check trees then drop
the string. Other runs at once still fail loudly, because the port's runs
at once have not been matched one by one to 0x4749b5's callers (0x422d7d,
0x44bc1b, 0x44f593, 0x452c37, 0x458505, 0x474b0b).

In play mode, after the Have A Snack order (50 steps, then the fridge's
menu 1), Mortimer's tree 0x1007 spends 5 in category 3 at step 820 (world
tick 89263): funds 2542 to 2537, today's category 3 from 0 to 5, global
13 to 2537. In the same tick tree 0x104a names his running action. His
animator then stops on the carry clip: "a person starts carrying: its
carry clip (0x26, 0x5903fc) is not ported". An install test pins this.
Strict mode on the Goth house is unchanged. Reopen when a recording of
the snack order disagrees, or when a script spends in category 8.
