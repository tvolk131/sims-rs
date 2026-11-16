# Sound files decode (track 3, sound)

*Decided 2026-11-14.*

A new crate, `sims-audio`, reads the game's sound files and decodes them
to PCM (16-bit signed samples). The simulation does not depend on it.
Every sound file in the Complete Collection install decodes.

Terms used here. *XA* is Maxis's 4-bit ADPCM (adaptive differential PCM)
sound format. A *FAR* archive is the game's file bundle.

What was decided:

- **Three formats, no others.** The install holds 18,155 XA files, 285
  WAV files (all 16-bit PCM at 22,050 Hz) and 141 MP3 files (the radio
  stations, MPEG-1 Layer III at 44,100 Hz, stereo). No file is UTK (The
  Sims Online's codec) and no file starts with its magic number. Outside
  those extensions, only the animation `.cfp` files start with bytes that
  look like an MPEG frame header, by chance.
- **XA is decoded from public notes.** The layout follows FFmpeg's Maxis
  XA demuxer and decoder and the Niotso wiki: a 24-byte header, then
  frames of 28 samples per channel in `15 * channels` bytes. The header's
  bytes 2 and 3 hold 0, `J` or 0x12 and are not read. One stereo file
  (`sting_crystalball.xa`) ends half way through a frame and needs that
  half's samples to reach its stated size, so a partial last frame
  decodes as far as it goes. Output is cut to the size in the header.
- **MP3 uses the `symphonia` decoder** (pure Rust, MPL-2.0). Its output
  is floating point converted to 16-bit, so its digest is pinned on one
  machine only. Music is presentation; nothing in the simulation reads it.
- **Sound files are found by install-relative path.** The sound tables
  name files such as `sounddata\phone\Phone_ring.xa`; that file is the
  member `Phone\Phone_Ring.XA` of `SoundData/SoundData.far`. An
  archive's members count as files of the archive's directory, paths
  ignore case, a loose file replaces a member, and the first of two
  members with one path is kept, as in `sims_content::Index`.
  **Unverified** that the sound manager resolves paths this way.

How it was checked:

- Synthetic tests: hand-computed XA samples (each predictor, clamping,
  stereo order, a partial frame), an XA encoder written apart from the
  decoder whose output decodes to exactly what it predicted, WAV chunk
  walking, and silent MP3 frames.
- A gated test (`SIMS_INSTALL`) decodes every sound file, shadowed
  duplicates included, and pins counts per format and variant, total
  frames, and an FNV-1a digest per format. It takes about 40 seconds in a
  debug build. An ignored test decodes all eight hours of MP3 in full;
  run once in a release build (24 seconds), every MP3 file decoded and
  gave as many frames as its frame headers count.
- **Against FFmpeg.** FFmpeg's `adpcm_ea_maxis_xa` and PCM decoders
  (FFmpeg through PyAV 15.1.0, in a private virtual environment) decoded
  all 17,915 distinct XA files and 285 WAV files. After cutting to the
  size each XA header states, every file's PCM equals ours, bit for bit.
  FFmpeg does not cut: for 17,129 files it returns the rest of the last
  frame too.

Not checked against the original. The game's own XA decoder was not
traced in the executable, and the harness runs with `-nosound`.
