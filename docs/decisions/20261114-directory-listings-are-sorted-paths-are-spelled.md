# Directory listings are sorted, and paths are spelled as on disk

*Decided 2026-11-14.*

From the back review of main:

- **Listings.** `std::fs::read_dir` joins the clippy bans, since the
  order it lists names in depends on the filesystem. The few listings
  left sort what they read and say so beside an `allow`.
- **Spelling.** `sims_content::install::resolve` now always returns the
  spelling the directory listing gives, and a case-insensitive match
  takes the first name in sorted order. Before, a case-insensitive
  filesystem (macOS) returned the spelling asked for and a
  case-sensitive one the spelling on disk. Paths reach state (a
  skeleton's file), so the same install could hash differently on two
  machines. The people crate's lookup takes the first match in sorted
  order too.
- **Other bans.** The bans also cover `asinh`, `acosh` and `atanh` for
  `f32` and `f64`, and `Instant::elapsed` and `SystemTime::elapsed`.
- **Not banned: environment reads.** No library crate reads the
  environment. Only tests (the `SIMS_INSTALL` gate) and tool entry
  points do, and the install path enters as data. Reopen if a library
  crate needs one.
