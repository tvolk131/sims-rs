# No headline state until a headline can show, and K = 877

*Decided 2026-11-14.*

Set Balloon/Headline is ported only as far as clearing (0x44b724). An
object has a headline when its SLOT has an entry of kind 1 or it is a
person, as its construction builds the list; nothing else is kept. The
priority a call writes is read only while a headline shows, and every
call that shows one writes it first; showing one fails loudly. So while
nothing shows a headline, a clear changes nothing that is kept, and so
do the other readers of the list (Sleep's test, the countdown after a
thread runs, a reset). Keeping the list now would be state that nothing
could ever change. Reopen when a script shows a headline: then the first
headline's duration, priority and tick count are kept per object, and
those readers are ported with it. K = 877; tick 878 turns the hour.
