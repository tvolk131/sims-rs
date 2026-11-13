# Retained live person panels

The replacement UI reads the selected live object's person data only after its
definition GUID resolves to an image-cache entry and a controlled-family
neighbor. Imported object IDs and NBRS person field 31 are not identity sources.
Category inputs carry this identity; detail inputs also identify their actual
trait, relationship target, job, or skill. Modal, hidden, pending, and stale
commands do not activate controls. Actual category activation publishes resource
4500/4501/4504/4503 through the existing Buttons owner, independently of selection
and highlighting.

Personality uses fields 7/6/3/5/2, signed division by 100, and the audited low,
middle, and high descriptions from original Live.iff STR#131. Skills use fields
10/12/11/17/18/15, divide by 100, and clamp display to 0..10. Relationship daily
and lifetime values come from columns 2 and 0 of the selected neighbor's directed
row, not the target's feelings. Target neighbor and cache GUIDs must agree.
Relationship tutorial highlighting belongs to the retained directed target
widget, survives refresh only for an unchanged selected/target identity, and
does not activate a control or select a household resident. Missing target
widgets remain explicit gate failures. The narrow sidebar categories use two
rows of two controls rather than overflowing one row.
Job title and description come from the acquired localized career/level/gender;
hours use current CARR values 18/19 and carpool `(start + 23) % 24`. Promotion
requirements compare raw next-level requirements to raw skill values. Missing
records disable their detail input rather than inventing a job or skill value.

This is a meaningful replacement presentation, not a claim to host the complete
native Windows panel core. Relationship targets use retained named buttons rather
than constructing a second original portrait window. Promotion shortfalls use
explicit text rather than native underlining. Popup text is loaded from original
Live.iff and retained in the client checkpoint.

The one retained popup toggles open/same-close/different-replace and publishes
only its actual displayed identity: motives, per.ity, rel, job, or skill. Refresh
preserves the selected category and an unchanged person's popup. Selection loss,
identity change, or removal of its detail data closes that owner. No timer closes
it and no tutorial flag, temporary, result, or countdown is modified. Original
BHAV4143 alone owns its twenty-tick countdown after the popup ceases to match.

Completed native selection changes refresh this owner before the viewer retains
its pending scene during a mode/publication transition. An Options cursor-reset
checkpoint can therefore save the current validated selected person (or actual
absence), rather than a stale previously displayed owner. Native selection work,
clock work, and client portrait work still prevent this refresh; identity
validation remains strict. Refresh never selects a person or changes native
selection flags, and only a real identity change/loss closes an owned popup.

Source audit: `cache/playable-alpha/tutorial-panel-contract-01.md` and
`cache/playable-alpha/tutorial-panel-contract-audit-10.json`. Focus observation
remains owned by `original_viewer::poll_application_ui` and Buttons activation.

Pure/headless test filter: `tutorial_panel_` in sims-render and sims-app.
Person-panel and Options test modules use explicit source paths. The earlier
shared04 artifact predates this correction and selects zero app tests with that
filter; it is not evidence that the new tests passed.

# Native Options mode

Original Options control 2009 calls the existing mode-change owner with target
3. Its native ClearTool, window pause 1/overlay 0, notices, dirty publication,
mode selector, and GLOBAL19 publication are not replaced by a shell toggle.
Options Show refreshes an explicitly bound, checkpointed visible content owner
before acknowledging visibility. World publication's WindowReset resets the
actual retained renderer cursor to 0 before the checked runtime completion;
it is not the prohibited household Reset control.

Graphics, Sound, and Play controls 4957/4958/4959 select retained visible
sections. Graphics and Play display real camera, free-will, speed, and pause
sources read-only. Audio and native settings editing remain unhosted, not
silently acknowledged. Save 4950 uses the existing production managed Save;
Continue Live uses the existing native Live transition. Unsupported native
Neighborhood 4951, Exit 4952, and Reset 4960 are not exposed. The separate
Application button retains the replacement save/load/Continue menu without
claiming a native mode transition or original control activation. Revision,
visibility, stale input, pending work, and modal checks gate Options controls.

The `tutorial_panel_native_options_mode_cursor_boundary_restore_and_separate_menu`
test is asset-dependent and ignored by default. It drives the actual mode and
cursor publication owners and checks restore, GLOBAL19, selected Options, and
the distinct application menu. Other `tutorial_panel_` tests are pure or
headless. Source tests require the parent's next coordinated build; no new
build or native application execution is performed by this implementation.
