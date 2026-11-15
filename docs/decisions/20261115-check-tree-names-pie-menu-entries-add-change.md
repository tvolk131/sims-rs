# A check tree names pie menu entries with Add/Change Action String

*Decided 2026-11-15.*

While the pie menu's listing checks an action, Add/Change Action String
adds an entry to the menu (0x458789): a copy of the action being checked,
with flags 4 and 8 set and 0x10 clear, the string as its name, and the
string's four arguments, the first being the frame's stack object when
the formatter left it 0 (0x44cca3). The entry goes into the menu at once,
whether or not the check passes, and the action itself is then left out
(0x458724 compares the list's length). The menu's entries are now
`MenuEntry` values, an action and an optional name; picking one queues
its action. The queue does not keep the name, which the original keeps in
the action (+0x28) for the interface.

The string's formatter (0x44bf1a) is not ported. Each of its passes looks
for a token starting with "$" and does nothing without one, so a string
with no "$" comes out unchanged and so do the arguments; a string with
"$" fails loudly. A string the running code's file lacks also fails
loudly, since what the original does with the null string 0x4704fa
returns for it is not read. On the Goth house the fridge now offers "Have
Quick Breakfast", "Have Breakfast" and "Serve Breakfast", and each
staircase "Go Upstairs"; 85 objects offer entries, and 9 fail in check
trees on Manage Inventory operation 4 and scopes 24 and 12. Reopen when a
string with a token is reached.
