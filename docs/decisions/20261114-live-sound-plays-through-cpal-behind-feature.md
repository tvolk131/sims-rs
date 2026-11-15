# Live sound plays through cpal, behind a feature that is off by default

*Decided 2026-11-14.*

`cargo run -p sims-audio --features playback --example play -- <event>...`
plays sound events on the default output device. The player renders
22,050 Hz stereo blocks on the main thread into a ring of frames that is
allocated once. The device callback takes frames from the ring and
converts them to the device's rate (by linear interpolation) and sample
format; it allocates nothing. The device paces the rendering. A device
that stops calling back for two seconds, or reports that the stream is
gone, ends the program with an error. At the end, the program waits for
the device's latency and a tenth of a second more, so the tail plays.

- cpal is optional and only the `play` example needs it, so `cargo
  test` and CI build no audio library and need no device. This matters
  on Linux, where cpal needs ALSA's headers.
- It lives in an example, not the library: the game's real host will
  live in the app, and how it mixes sound with its frame loop is the
  app's choice.
- Run on macOS only (48 kHz, f32): the device opened and the events
  played with no errors or underruns. Nobody has listened to it yet.

Reopen when the app wants live sound: it can take this callback or use
its own.
