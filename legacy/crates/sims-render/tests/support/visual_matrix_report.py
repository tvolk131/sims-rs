#!/usr/bin/env python3
"""Create a local HTML index and SHA-256 manifest for a completed visual matrix.

Usage: python3 crates/sims-render/tests/support/visual_matrix_report.py OUTPUT_DIR
       [--baseline PREVIOUS_OUTPUT_DIR] [--fail-on-change]
No image mutation, original game access, or simulation execution.
"""
import hashlib
import html
import json
import argparse
from pathlib import Path
import shutil
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--fail-on-change", action="store_true")
    args = parser.parse_args()
    output = args.output.resolve()
    root = Path(__file__).resolve().parents[4]
    if not output.is_relative_to(root / "cache"):
        raise ValueError("matrix report belongs in the private cache")
    matrix = json.loads((output / "matrix.json").read_text())
    provenance_path = root / "cache/visual-geometry-20261022/manifest.json"
    provenance = json.loads(provenance_path.read_text())
    recorded = {row["path"]: row["sha256"] for row in provenance["files"]}
    for reference in matrix["original_references"]:
        for field in ["context", "original_image"]:
            path = root / reference[field]
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            if digest != recorded[path.name]:
                raise ValueError(f"original capture differs from recorded provenance: {path}")
    cases = matrix["cases"]
    expected = {
        (house, floor, rotation, zoom, mode)
        for house in ["newbie", "goth", "mashuga"]
        for floor in [1, 2]
        for rotation in range(4)
        for zoom in [1, 2, 3]
        for mode in ["walls-up", "walls-down", "cutaway-first-room"]
    }
    actual = {
        (c["household"], c["floor"], c["rotation"], c["native_zoom"], c["wall_mode"])
        for c in cases
    }
    if actual != expected or len(cases) != len(expected):
        raise ValueError("incomplete or duplicated 216-view matrix")
    checks = ["finite_geometry", "valid_resources", "no_uncovered_background", "cold_pixels_equal", "repeat_pixels_equal"]
    if not all(c[k] for c in cases for k in checks):
        raise ValueError("a matrix invariant failed")
    cards = []
    for case in cases:
        key = case["key"]
        if not key or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-" for c in key):
            raise ValueError("invalid frame key")
        if not (output / f"{key}.png").is_file():
            raise ValueError(f"missing matrix image: {key}")
        cards.append(
            f'<article data-house="{case["household"]}" data-floor="{case["floor"]}" '
            f'data-mode="{case["wall_mode"]}" data-zoom="{case["native_zoom"]}">'
            f'<a href="{key}.png"><img loading="lazy" src="{key}.png" alt="{key}"></a>'
            f'<p>{html.escape(key)}</p></article>'
        )
    references = []
    for reference in matrix["original_references"]:
        name = reference["wall_mode"]
        references.append(
            f'<section><h3>Newbie · {html.escape(name)}</h3>'
            f'<p>Original tick {reference["original_tick"]}; Rust tick {reference["rust_tick"]}. '
            'Matched camera, zoom, floor and wall inputs; scene state differs. '
            f'{reference["native_wall_segments_compared"]} original wall segments agree.</p>'
            '<div class="compare">'
            f'<div><p>Original · <a href="reference-newbie-{name}-original.png">full capture</a></p>'
            f'<div class="crop"><img src="reference-newbie-{name}-original.png"></div></div>'
            f'<div><p>Rust</p><div class="crop"><img src="reference-newbie-{name}-rust.png"></div></div>'
            '</div></section>'
        )
    document = """<!doctype html><meta charset="utf-8"><title>Sims visual regression matrix</title>
<style>
body{font:15px system-ui;background:#142026;color:#edf1f0;margin:28px}a{color:#72d9c3}
h1{font-size:27px}.note{max-width:1000px;line-height:1.6}.filters{display:flex;gap:14px;flex-wrap:wrap;margin:24px 0}
select{padding:8px;background:#263c44;color:white;border:1px solid #557078;border-radius:5px}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(310px,1fr));gap:16px}article{background:#1e3037;padding:9px}
article img{width:100%;display:block}article p{font-size:12px;overflow-wrap:anywhere}article[hidden]{display:none}
.compare{display:grid;grid-template-columns:1fr 1fr;gap:18px;max-width:1640px}.crop{aspect-ratio:8/5;overflow:hidden}.crop img{width:100%;display:block}
@media(max-width:700px){.compare{grid-template-columns:1fr}}
</style>
<h1>Sims visual regression matrix</h1>
<p class="note">216 Rust views passed geometry, resource, background coverage, repeat-render and cold-restore checks.
Drawing did not advance or change any household's saved simulation. Immutable assets were loaded with the original starting-save namespaces absent.
These checks are independent visual invariants, not original-game pixel parity or gameplay-tick parity.</p>
<p class="note">There are <b>zero strict whole-scene original reference matches</b>. Three Newbie examples below match presentation inputs,
but original and Rust gameplay states differ. Original Goth/Mashuga reference captures and the other camera combinations remain missing.
Cutaway matrix views use an explicit first-indoor-room corner mask, not a claim that the original input producer generated that mask.</p>
<h2>Original reference context</h2>
""" + "".join(references) + """
<h2>Invariant matrix</h2><div class="filters">
<label>House <select id="house"><option value="">All</option><option>newbie</option><option>goth</option><option>mashuga</option></select></label>
<label>Floor <select id="floor"><option value="">All</option><option>1</option><option>2</option></select></label>
<label>Walls <select id="mode"><option value="">All</option><option>walls-up</option><option>walls-down</option><option>cutaway-first-room</option></select></label>
<label>Zoom <select id="zoom"><option value="">All</option><option>1</option><option>2</option><option>3</option></select></label>
<span id="count"></span></div><div class="grid">""" + "".join(cards) + """</div>
<script>
const keys=['house','floor','mode','zoom'];
function filter(){let count=0;for(const card of document.querySelectorAll('article')){
card.hidden=keys.some(key=>document.getElementById(key).value&&card.dataset[key]!==document.getElementById(key).value);
if(!card.hidden)count++;}document.getElementById('count').textContent=count+' views';}
for(const key of keys)document.getElementById(key).addEventListener('change',filter);filter();
</script>"""
    (output / "index.html").write_text(document)
    tracked = list(output.glob("*.png")) + [output / "matrix.json", output / "index.html", Path(__file__).resolve(), root / "crates/sims-render/tests/visual_matrix.rs"]
    for household in matrix["households"]:
        tracked.append(root / household["source_checkpoint"])
    for reference in matrix["original_references"]:
        tracked += [root / reference["context"], root / reference["original_image"]]
    roof = root / "cache/visual-geometry-20261022/original-roof.png"
    if hashlib.sha256(roof.read_bytes()).hexdigest() != recorded[roof.name]:
        raise ValueError("illustrative roof capture differs from recorded provenance")
    shutil.copyfile(roof, output / "illustrative-newbie-roof-original.png")
    tracked += [provenance_path, roof, output / "illustrative-newbie-roof-original.png"]
    manifest = {
        "schema": 1,
        "matrix_cases": len(cases),
        "invariants_passed": checks,
        "strict_whole_scene_reference_matches": [],
        "matched_presentation_input_references": matrix["original_references"],
        "unmatched_illustrative_references": [{
            "image": "illustrative-newbie-roof-original.png",
            "household": "newbie",
            "reason": "Authentic recorded original frame; no corresponding per-image roof/camera snapshot or aligned Rust scene was established, so no equality comparison is made."
        }],
        "missing_strict_reference_cells": [{"household": h, "floor": f, "rotation": r, "zoom": z, "walls": m} for h, f, r, z, m in sorted(expected)],
        "original_capture_requirements": ["original source save and immutable-asset hashes", "paused scene state or a proven aligned tick", "camera center/viewport/rotation/zoom", "floor, wall/roof mode, cutaway mask", "original screenshot and capture provenance"],
        "files": {str(p.relative_to(root)): {"bytes": p.stat().st_size, "sha256": hashlib.sha256(p.read_bytes()).hexdigest()} for p in tracked},
    }
    if args.baseline:
        baseline = args.baseline.resolve()
        if not baseline.is_relative_to(root / "cache"):
            raise ValueError("baseline must be a saved private matrix")
        old = json.loads((baseline / "manifest.json").read_text())
        old_files = {Path(path).name: info["sha256"] for path, info in old["files"].items()}
        changed = []
        same = []
        for case in cases:
            name = case["key"] + ".png"
            digest = manifest["files"][str((output / name).relative_to(root))]["sha256"]
            (same if old_files.get(name) == digest else changed).append(case["key"])
        manifest["saved_render_comparison"] = {
            "baseline": str(baseline.relative_to(root)),
            "unchanged_frames": same, "changed_or_missing_frames": changed,
            "scope": "Exact saved-render regression only; not original-game pixel equality. Review input changes before treating differences as defects."
        }
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"216 views verified; report: {output / 'index.html'}")
    if args.fail_on_change and (not args.baseline or changed):
        raise SystemExit("saved-render baseline is missing or contains changed frames")


if __name__ == "__main__":
    main()
