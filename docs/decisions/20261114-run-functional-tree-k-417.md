# Run Functional Tree, and K = 417

*Decided 2026-11-14.*

Tick 417 of the Goth house stopped on Run Functional Tree (opcode 20),
node 7 of person 21's bookcase tree (0x1014). The handler is `0x44f681`.
It is reached through the general primitive table for every object; the
person's dispatcher does not take opcode 20.

- **The function table** is the jump table at `0x44f76a`:
  - functions 0 to 7 name callbacks 0x12 to 0x19;
  - 8 names 0x1a, 9 names 0x1b, 10 names 0x0e, 11 names 0x1c;
  - 12 names 0x10, 13 names 0x11, 14 names 0x1d.

  Any other function is false; the comparison is unsigned, so a function
  below 0 is false too. Function 8 asks the same person check as Find
  Best Object for Function (`may_ask_function_8`). Unlike that
  primitive's table, functions 4, 5 and 9 have callbacks here.
- **The action** is the first word of the callback's entry (`0x455fa9`),
  and 0 past the table's end. An action of 0 is false.
- **The push** is the running object's virtual function 0x34, the push
  Run Tree By Name's mode 2 makes: the running object is woken, the tree
  of the stack object's type gets the running object's temps (a null
  arguments pointer, `0x47484a`), and a person starts using the stack
  object (`0x45e59b`).
- **The flag** (byte 2, bit 0) goes into the person's use. A set flag
  also writes person +0xa54, which is not modeled, so it fails loudly.
- **The return branches like a call.** The handler returns 3 without
  writing the frame's state (`0x44f75c`), while Run Tree By Name writes
  1 first (`0x44bc3d`). So the return takes the branch of the tree's
  result, as a subroutine's does. This contradicts the legacy note that
  Run Functional Tree resumes; the interpreter comment is corrected.
- **Error codes.** A missing stack object is native error 0x15, thrown
  before the function is read. A tree the type lacks fails loudly here;
  the original's push would give false.
- **K = 417.** After the tick, person 21's frames and object uses equal
  the capture's: 174 at depth 7, 52 at depth 8.
- **The verifier compares the carry layer now.** The capture shows the
  carry slot filled, as ours is. The layer's own fields are on a page
  the capture lacks, so they stay unverified.

Reopen if a capture shows a functional tree's return resuming the
primitive instead of branching.
