# The pie menu is ported from the original's code

*Decided 2026-11-15.*

`sims_ui::pie` lays out, hit-tests and draws the pie menu, and groups an
object's actions into submenus, as the original's menu window
(cTSPieMenu, vtable 0x674b54) and the object picker do. It is pure: the
host passes the click, the screen size and the action names (from
`Lot::pie_menu`), keeps a `pie::Pie`, and gets back the chosen action's
index. The 3D head in the middle is the host's to render and pass in.

**Read from the code.** Layout 0x5ea8e1, drawing 0x5eb04c, the label box
0x5eae63, the hit test 0x5eb7c9 and placement 0x5ebab2. The people panel
(0x518665) makes the game's two menus and sets what differs from the
constructor's defaults: radii 60 and 30 (not 40 and 20), a dead zone of
35 pixels, a reach of 150, resource 800 (`PieFace1.bmp`, 201 by 201, all
colour key) in the middle, and the colours (boxes 0,40,140; labels
165,195,214; the highlighted label white, or 0,255,255 while a button is
held, which is also the title's colour). The picker (0x48d56c, 0x48d8a0)
puts a name with a `/` into the submenu named by the text before the
first `/` (exact match), shows at most 12 entries, and opens a submenu
centered on the entry that opened it, with the first menu's entries
hidden. A press in a submenu's dead zone goes back to the first menu.

**Checked.** The window origin and label box agree to the pixel with the
two menus the input track read from the running original ("Sit" and "Go
Here" on the Goth house, `crates/sims-ui/tests/pie.rs`).

**Unverified.** How the drawn menu looks has not been compared with a
screenshot: whether the box fill is opaque (the surface's fill, function
0x40, is taken as opaque), the text's exact position in the box, and how
the head sits in the face. The picker pages past 12 entries on an answer
of -3; what sends -3 is not traced, so only the first 12 are shown. The
menu window's own sounds (`UI_piemenu_appear`, `_highlight`, `_select`)
play only when its flag +0x17c is set, and the people panel never sets
it, so the game's pie menu plays none of them.

**Reopen** when a screenshot of an open pie menu disagrees, or when the
paging trigger is found.
