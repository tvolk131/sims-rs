# Third-party notices and reference provenance

The MIT license in `LICENSE` covers project-contributed source and documentation,
not third-party software, original-game content, trademarks, or privately retained
research material. Nothing here grants rights that the contributors do not own.

## Original game

The Sims, its game content and its associated names and marks belong to their
respective rightsholders, including Electronic Arts and Maxis. This is an
unofficial compatibility project, not an EA product or an endorsed distribution.
No original-game license is provided. Obtain any required game installation
separately and comply with the rights and terms applicable to its use.

## Consulted projects

Historical development records identify these pinned references:

| Reference | Revision | Recorded use |
| --- | --- | --- |
| FreeSO | `4c6b3e8f5835b228723caea3c9f683c62f244f73` | FAR/IFF and chunk-reader format references, resource semantics and VM registration names. |
| Simitone | `4906db3156616bfaffc10ce0963cf08062975fef` | Compatibility and save-wrapper behavior comparison. |
| SneakyLib | `8b86187bbb46ce0da661809767e06aa748652d44` | Historical format research reference; this notice does not assert a license grant. |

FreeSO's pinned repository identifies its license as MPL-2.0:
<https://github.com/riperiperi/FreeSO/blob/4c6b3e8f5835b228723caea3c9f683c62f244f73/LICENSE.md>.
The relevant FreeSO chunk-reader directory is
<https://github.com/riperiperi/FreeSO/tree/4c6b3e8f5835b228723caea3c9f683c62f244f73/TSOClient/tso.files/Formats/IFF/Chunks>.
The Simitone reference is
<https://github.com/riperiperi/Simitone/tree/4906db3156616bfaffc10ce0963cf08062975fef>.

Existing project records describe the Rust readers as independently written
using these references. The publication audit did not establish an implementation
port, but neither it nor this notice is a line-by-line legal clearance. This is
**not a clean-room provenance claim**. Consultation does not by itself identify
which, if any, implementation is adapted from an upstream source. Any discovered
adaptation must be reviewed, correctly attributed and distributed under its
applicable terms; the project MIT license does not override them. Do not copy or
translate upstream implementation code into an MIT-only file without that review.

## Original developer documentation

Development notes also cite the original virtual-machine and graphics design
documents hosted by Don Hopkins at <https://donhopkins.com/home/TheSimsDesignDocuments/>.
They are reference material, not project-authored documents or a license grant;
no copies of those documents are included in this repository.

## Excluded QEMU tooling

Private research tooling includes a patch generator targeting QEMU 7.2.22 and a
generated header marked `GPL-2.0-or-later`. That tooling, upstream source anchors,
generated patches and emulator binaries are excluded from this public baseline.
They are not relicensed under the project MIT license. Publishing them later
requires a separate per-file upstream-license and notice review.

## Rust dependencies and binary redistribution

Dependencies retain their own licenses. The lockfile-resolved package inventory
and declared license metadata are in [dependency-licenses.md](docs/dependency-licenses.md).
That inventory is metadata, not a bundle of license texts or a complete
binary-distribution compliance report. Target selection and features affect what
is actually linked. Some dependencies require more than one license notice.

This baseline distributes source, not ready-made game bundles, fonts, game assets
or application binaries. Before distributing a built application, inventory its
actual dependency closure, retain required license texts/NOTICE files and meet
the selected licenses' other obligations. Review bundled system libraries and
fonts separately. Crates.io publication is disabled in the workspace packages
pending a deliberate release review.
