# Create-a-Family & Create-a-Sim UI Screens (`sims-ui::caf`)

## Context

The Sims Complete Collection 1.91 implements household creation across two
full-screen UI views backed by `800×600` bitmaps in `UIGraphics/Res_Nbhd.h` and
localized strings in `GameData/UIText.iff`:

- **Create-a-Family (`cWinEditFamily` / `cWinCreateFamily`)**: `kDesignFamilyBkg`
  (`5022`), `kDesignFamilyAdd` (`5023`), `kDesignFamilyDelete` (`5024`),
  `kDesignFamilyEdit` (`5025`), `kDesignFamilyDone` (`5026`),
  `kDesignFamilyCancel` (`5027`), and `UIText.iff STR# 129`.
- **Create-a-Sim (`cWinEditPerson`)**: `kDesignCharBkg` (`5042`), age/gender/skin
  toggles (`5028`, `5029`, `5035..=5039`), head/body skin arrows (`5030`,
  `5032`), personality LEDs (`5040`), remaining 25-point bar (`5203`), Done/Cancel
  (`5033`, `5034`), and `UIText.iff STR# 130` + `STR# 164` (zodiac names).

Layout coordinates and interactive flows were cross-checked against the verified
`800×600` Complete Collection 1.91 oracle capture (`caf-create-family-20261121`).

## Decision

1. **Decoupled UI module (`crates/sims-ui/src/caf.rs`)**:
   - `CafController` owns UI state (`CafScreen::Family` / `CafScreen::EditSim`),
     draft household/Sim data (`FamilyDraft`, `SimDraft`, `Personality`,
     `SimAppearance`), text-input focus/caret blink (`dialog::CARET_BLINK_US`),
     and native confirmation dialogs (`DeleteSim`, `CancelFamily`,
     `FinishFamily`, `MorePersonality`).
   - User interactions emit typed [`CafCommand`] values (`CommitFamily`,
     `CancelFamily`, `OpenSimEditor`, `SimSaved`, `SimDeleted`, `CancelSim`,
     `AppearanceChanged`, `RotatePreview`) so the host (`sims-game`) can wire
     Track 4's character/family IFF writer and Track 5's `sims-people` 3D
     turntable without coupling `sims-ui` to simulation or rendering internals.
2. **Exact native personality budget & zodiac calculation**:
   - `Personality` enforces `0..=10` per trait and the native 25-point total
     budget across `[Neat, Outgoing, Active, Playful, Nice]`.
   - `Personality::zodiac()` delegates directly to `crate::household::zodiac`
     (matching `0x4841fc`'s Euclidean nearest-sign profile) and returns `None`
     when 0 points are spent, matching the oracle screenshots (`(7, 5, 3, 4, 6)`
     -> Cancer).
3. **3D turntable integration point**:
   - `CafLayout::preview_rect` exposes the exact `(574, 118, 182, 263)` viewport
     (at `800×600`, scaled cleanly in compact mode), and `CafController::draw`
     accepts an optional `preview_image: Option<&Image>` while supporting both
     rotate buttons and pointer drag inside the viewport.
4. **Dynamic strings & compact reflow**:
   - All user-visible strings load dynamically from `GameData/UIText.iff`
     (`STR# 129`, `130`, `142`, `164`) with zero verbatim game prose in source
     or tests.
   - Above `800×600`, the `800×600` CAF canvas centers cleanly; below `800×600`,
     `CafLayout` scales all control rectangles proportionally to fit the frame.

## Consequences

- `sims-ui` stays strictly read-only and decoupled from `sims-game`,
  `sims-people`, and `sims-render`.
- Follow-up gameplay integration can wire `CafController` into the Neighborhood
  / Family Bin flow and feed `sims-people` turntable frames directly into
  `CafController::draw`.
