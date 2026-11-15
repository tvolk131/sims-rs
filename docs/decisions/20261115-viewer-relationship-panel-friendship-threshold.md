# The viewer's relationship panel reads the friendship threshold from the tuning

*Decided 2026-11-15.*

`sims-viewer --hud` now fills the relationship panel as the window does
(`Snapshot::with_relations`, read only while the panel is open). It loads
the faces of everyone the world's neighborhood records hold, not only the
household. Moving the pointer onto, off or between entries redraws the
panel (`hud::lit_relation`), since an entry is drawn lit under the
pointer though it takes no press. The viewer has no simulation program, so the smileys'
friendship threshold comes from `household::friendship_threshold`. It reads
the global file as `Program` does: every `FCNS` chunk parsed in file
order, deleted and replaced ones included (a malformed one is an error,
as it stops the program's load), and the last with ID 2 is the table. Its
entry "friendship threshold" (any case, 25.0 when missing) is truncated
with the same `__ftol` model (`Ext::to_i32_truncate`: the low word of
the 64-bit truncation; NaN, infinity and values past 64 bits give 0).

- **Verified**: on the install, the reader and the program both give 50
  (`crates/sims-game/tests/tuning.rs`). Synthetic files pin the chunk
  order, the replacement, the malformed table and the truncation
  (`crates/sims-ui/tests/tuning.rs`).
- **Unverified**: the viewer's panel has not been compared with the
  original on screen. It shows the same snapshot the window's panel does.
