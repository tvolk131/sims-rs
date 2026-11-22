# Stereo/TV Streaming Radio Stations & Co-op Multiplayer UI

- **Date:** 2026-11-21
- **Scope:** `crates/sims-audio/src/radio.rs`, `crates/sims-audio/src/music.rs`, `crates/sims-ui/src/coop.rs`
- **References:** The Sims Complete Collection 1.91 (`stations::STATIONS`, `placement::project`, `levels`, `UIGraphics` `3008` / `17` / `5107`) and `docs/vision.md` (**Multiplayer (v1)**)

## 1. File-Backed Streaming & In-Lot Radio/TV Stations (`crates/sims-audio/src/radio.rs`)

- **Shared `File`-Backed Streaming (`music.rs` → `radio.rs`):**
  - Added `TrackData::{File, Bytes}` and `TrackSource::open_track` to `crates/sims-audio/src/music.rs`. For loose files on disk (`SoundFiles`), `StreamDeck` now peeks the 12-byte header and passes `std::fs::File` directly to `symphonia`'s `MediaSourceStream` for MP3 tracks instead of reading multi-megabyte compressed MP3 files into RAM, while decoding WAV and Maxis XA tracks into PCM buffers.
  - Both `ModeMusicPlayer` and `RadioPlayer` share `StreamDeck` and the deterministic `splitmix64` / `xorshift` shuffle primitives.
- **Station Discovery (`RadioCatalog`):**
  - Discovers all 14 built-in Complete Collection stereo stations under `Music\Stations\*` (`Country`, `Rock`, `Latin`, `Classica`, `CountryD`, `Rap`, `Rave`, `Disco`, `Beach`, `EZ`, `EZExp`, `UnlCheap`, `UnlSpeak`, `Superstar`), plus all 8 built-in television channels under `SoundData\TVStations\*` and `ExpansionPack6\Sound\TVStations\*` (including their `a..e` subdirectories of `.XA` clips).
  - Automatically picks up custom `.mp3`/`.wav` tracks dropped into any existing `Music\Stations\<Station>\` folder as well as user-created custom station folders (`Music\Stations\<CustomName>\`) via `StationId::Custom`.
- **Spatial Attenuation, Volume Groups, and Shuffle (`RadioPlayer`):**
  - Multi-emitter mixer keyed by deterministic `BTreeMap<u32, EmitterChannel>` (one per active stereo/TV object on the lot) with smooth station crossfades (`DEFAULT_STATION_CROSSFADE_MS = 150 ms`) and per-station non-repeating Fisher-Yates shuffle.
  - `SpatialAttenuation` supports both Complete Collection 1.91's exact screen-space 3D placement (`placement::project`) and a tile-distance + 3/5 other-floor attenuation helper, mapped through `levels::{attenuation, pan, gains}` to 16.16 stereo fixed-point gains.
  - Stereo stations scale by the Music volume slider (`VOLUME_STEPS`, `kGroupMusic`); TV channels scale by the FX volume slider (`VOLUME_STEPS`, `kGroupFx`), both multiplied by master volume.

## 2. Co-op Multiplayer Lobby Dialog & In-Lot HUD Overlay (`crates/sims-ui/src/coop.rs`)

- **Native 1.91 Art Chrome:**
  - Uses `PopupInfoTiles.bmp` (`3008`), `WinBtn.bmp` (`17`, horizontally 3-sliced to preserve rounded button caps at arbitrary widths), `HotspotPopupTiler.TGA` (`5107`), and `cTSWinTextEdit2` dark-navy input styling (`rgb565(0x000a)` with 500 ms blinking caret).
  - Decoupled from `sims-net` and `sims-game` via read-only `CoopSnapshot` and typed `CoopCommand`.
- **Host / Join Lobby Dialog (`CoopLobbyDialog`):**
  - **Host tab:** start/stop hosting, read-only invite ticket box with one-click Copy button (`CopyInviteTicket`), "Host-only time control" lock checkbox (`SetHostOnlyTimeControl`), and connected player list (color badge, host tag, selected Sim, ping).
  - **Join tab:** focusable ticket input (`insert_char`, `paste_text` stripping whitespace/control chars, `backspace`, `Enter` submit), Connect/Cancel actions, and status/error callouts for `Connecting`, `ManifestMismatch` (local/remote summaries + sample mismatched files), and `Error`.
- **In-Lot Co-op Status & Attribution Overlay (`CoopHudOverlay`):**
  - Compact top-right player pill/panel showing connected players (badge color, name, selected Sim, Buy/Build mode tag), host-lock indicator, and active pause reasons (`"Paused by <Name>"`, `"<Name> is in Buy Mode"`, `"<Name> is in Build Mode"`, `"Waiting on dialog"`).
  - Transient speed/pause attribution toast (`"<Player> set Ultra Speed"`, etc.) and drift-resyncing progress banner with byte progress bar.
  - Responsive layout guarantees all three overlay regions are strictly non-overlapping across viewports from `320×240` to `1920×1080`.
