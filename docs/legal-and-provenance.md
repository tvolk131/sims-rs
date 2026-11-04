# Licensing, provenance and publication boundary

Publication audit: November 3, 2026. This is a practical engineering record and
contribution policy, **not legal advice or a guarantee of noninfringement**.
Licensing, copyright, contracts and reverse-engineering restrictions require
context-specific review. Publishing source does not resolve those questions.

## What the project licenses

Project-contributed source and documentation are offered under the MIT license
in `LICENSE`. See `THIRD_PARTY_NOTICES.md` for the separate third-party boundaries
and consulted references. The project cannot license Electronic Arts/Maxis game
content, upstream source that it does not own, or third-party trademarks.

The project is an unofficial, experimental compatibility implementation. It is
not affiliated with or endorsed by Electronic Arts or Maxis. Its name identifies
the software it investigates; no game branding assets are provided. It is not a
distribution of the original game and is not a claim that that game is freeware
or that ownership of a copy automatically authorizes every kind of analysis.

Development has used behavioral observations, executable analysis and published
open-source format references. **Do not describe the project as clean-room.**
Preserve the reference disclosures even when historical research notes are kept
private. A fresh Git history improves the upload boundary; it does not erase
development provenance, cure infringement or remove upstream license duties.

## Findings that changed the publication plan

The pre-publication tree was not safe to upload merely because installation and
cache directories were ignored. Review found original executable instruction
witnesses in Rust source and tests, game-authored tutorial/notification prose in
tests, large native-capture fixture corpora and separate QEMU-related tooling
with its own licensing boundary. Some nominally authored tests mixed synthetic
owners with actual original-code witnesses.

The original native-suspension validator embedded a 32-byte executable witness.
It now compares the SHA-256 of caller-supplied bytes with the existing witness's
digest. The exact input length, pinned image, address and caller checks remain;
the validator does not ship those original instructions or accept invented bytes.
Other literal instruction-identity checks likewise use digests of separately
supplied bytes, with their existing fixed-length reads preserved. Exact game
notification matching uses text digests rather than shipping the game prose.

Tests containing executable witnesses or game prose are retained only in ignored
local modules. Tests that embed the excluded corpus, and identified runtime
readers of that corpus, require the explicit `private-fixtures` feature. Other
asset-dependent manual tests remain ignored and need separately supplied inputs.
The default public build and unit suites do not need those files.
The feature is a local capability switch, **not permission to redistribute its
inputs**. Enabling every feature in a fresh public clone is intentionally not a
supported self-contained test configuration.

## Deliberately not published

- Original installations, archives, resources, scripts, music, dialogue, fonts,
  meshes, textures, screenshots, executable images and original save files.
- Extracted assets, process-memory/code images, replay captures and the original
  `crates/*/tests/fixtures/` corpus, including compressed JSON evidence.
- Private test modules containing original instruction witnesses, game-authored
  prose or captured storage tails; small captured comparison JSON records not
  needed by the public implementation are excluded too.
- `scripts/research/`, including instruction-bearing scripts, third-party source
  anchors and QEMU patch-generation tooling. No emulator or guest image is shipped.
- Historical `docs/` research material, other than the explicitly reviewed public
  legal, dependency and repository-workflow documents.
- The earlier local Git history, ignored local checkouts, credentials, build
  outputs, application bundles and private publication backups.

Files remain on the maintainer's machine rather than being destroyed. The old
history and pre-sanitization working tree are backed up under the ignored local
cache. They must never be pushed, attached to issues, added to releases, included
in source archives or uploaded to Actions artifacts. No old commit is a parent
of the public baseline. Do not publish a Git bundle or use `git push --mirror`.

## What remains as compatibility information

The implementation retains identifiers, hashes, resource names, format-recognition
signatures, field layouts, scalar compatibility constants and project-authored
synthetic tests. These are not an original executable image or a game asset
bundle. Hashes identify separately supplied local inputs; they do not supply them.

The reviewed `autonomy_resources/initial-input.json` is a small numeric initial
configuration and source-fingerprint record used by production code. It contains
scalar words and provenance identifiers, not original instructions, dialogue or
resource payloads. Its exact bytes, like the other admitted JSON files, are pinned
in the repository policy. Do not treat a `.json`, `.rs` or `.py` extension, an
"authored" label, or a passing size check as evidence of redistribution rights.
The admitted `fixtures/suites/` manifests are project-authored input schedules
and case references, not the private captures to which those references point.

The consulted FreeSO, Simitone and SneakyLib revisions and limitations are recorded
in `THIRD_PARTY_NOTICES.md`. No claim of a comprehensive source-comparison or
legal-clearance review is made. If an upstream implementation is found in the
tree, resolve its file-level licensing and notices before further distribution.

## Rules for contributions and releases

1. Submit only material you have the right to contribute. Identify upstream
   implementations, translations and adaptations with the exact source revision,
   affected files, license and required notices. Do not claim sole authorship of
   someone else's implementation or silently place it under MIT.
2. Prefer project-authored synthetic fixtures and explain how they were created.
   Real-game captures remain local unless separately reviewed and cleared. Do not
   submit entire executable functions, copied game prose or resource payloads,
   even encoded as hex, arrays, JSON, gzip or generated source.
3. Keep game installations and private evidence outside the repository, or in
   the explicitly ignored local locations. Do not upload them to an issue,
   screenshot, CI log, package, release, Actions cache or test artifact.
4. Run the working-tree, staged and reachable-history publication gates before
   pushing. They check known risks, not arbitrary copyright or all secrets. Human
   provenance review remains necessary, including for new generated content.
5. Review binary redistribution separately: actual linked dependencies, license
   texts, NOTICE files, system components, fonts, assets and all accompanying
   files. An SPDX metadata table alone does not satisfy every license obligation.
6. Obtain any permissions or qualified legal review needed for your intended
   reverse-engineering, interoperability and distribution activities. No policy
   here overrides an applicable law, contract, license or anti-circumvention rule.

Primary background references, not a determination of this project's legal status:

- U.S. Copyright Office, *Copyright Registration of Computer Programs*:
  <https://www.copyright.gov/circs/circ61.pdf>.
- Mozilla, MPL 2.0 FAQ and license text:
  <https://www.mozilla.org/en-US/MPL/2.0/FAQ/> and
  <https://www.mozilla.org/en-US/MPL/2.0/>.
- QEMU licensing documentation:
  <https://www.qemu.org/docs/master/about/license.html>.
