# Decision: Loading Screen, Travel Destination Chrome, and Simlish Voice Bank Resolver

- **Date:** 2026-11-21
- **Status:** Accepted
- **Scope:** `crates/sims-ui/src/{loading,travel}.rs`, `crates/sims-audio/src/voices.rs`

## Context

Complete Collection 1.91 renders dedicated transition loading screens (`cWinLoading`), expansion sub-neighborhood top control bars and filter legends for Downtown, Vacation Island, Studio Town, and Magic Town (matching the four expansion maps rendered by `sims-render::neighborhood`), and organizes over 10,000 Simlish voice clips across `SoundData/SoundData.far` and `ExpansionPack*/Sound/Sound.far` / `ExpansionShared/Sound/Sound.far`.

To keep UI and audio library lanes cleanly decoupled from `sims-game` while remaining verified against the real Complete Collection install, these three subsystems are added as standalone library modules.

## Decisions

### 1. Native Loading Screen (`sims_ui::loading`)
- Loads all 19 Complete Collection startup and travel splash artwork IDs from `UIGraphics` (`Res_Other.RT` `kSimsLogo`, `Res_Nbhd.RT` 800×600 and 1024×768 Downtown, Vacation, Community Bus, Studio Town, and Magic Town transition bitmaps).
- Reads localized rotating captions directly from `GameData/UIText.iff` (`STR# 155` `splashprogress` and `STR# 271` Superstar celebrity loading strings) with zero verbatim game prose in source or tests.
- Emits `LoadingAudioCue::{StartLoadLoop, StopLoadLoop}` (`mode_id = 5` / `Music\Modes\Load\loadloop.wav`) so hosts can coordinate `MusicMode::Load` without coupling `sims-ui` to `sims-audio`.
- Centers native 800×600 (or 1024×768) splash artwork on viewports `>= 800×600`, and switches to a compact non-overlapping layout on viewports `< 800×600`.

### 2. Travel Destination Chrome & Confirm Dialog (`sims_ui::travel`)
- Models all four expansion sub-neighborhoods (`Downtown`, `VacationIsland`, `StudioTown`, `MagicTown`) with pairwise-disjoint top-bar controls (Return Home, Bulldoze, Import, Destination Switcher) and a secondary filter/legend strip (`STR# 171`, `NghUI/Filter*Btn.bmp`).
- Exposes destination-specific chrome: Vacation Island hotel/rental/campground lodging legend and No Vacancy indicator, Studio Town half-star fame badge and studio lot tags, and Magic Town MagiCoins balance pill.
- Provides `TravelDialog` / `TravelDialogState` covering Taxi, Shuttle, and Magic Hole confirmations with Simoleon/MagiCoin affordability and non-empty traveler validation.

### 3. Simlish Voice Bank Index & Resolver (`sims_audio::voices`)
- Indexes voice `.XA` and `.WAV` clips and `.HIT`/`.HOT` voice script sets over `SoundFiles` across `SoundData\Voice\*`, `SoundData\Phone\Vox_*`, `SoundData\VOX\*`, `ExpansionShared\Sound\Vox\*`, `ExpansionPack\`, and `ExpansionPack*\Sound\{vox,voxcat,voxdog,npc}\*`.
- Classifies clips deterministically into `VoiceAgeGender` (`AdultMale`, `AdultFemale`, `Child`, `Baby`, `Dog`, `Cat`, `Npc`, `Unspecified`), provides deterministic `pick(voice, category, seed)` selection, and reuses `crate::decode` (`xa` / `wav`) + `crate::levels::rate` for pitch-shifted linear resampling and chunked `VoiceStream` playback.
