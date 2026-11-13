#!/usr/bin/env python3
"""Run independent gameplay plans with bounded concurrency and retain every outcome.

Manifest: {"jobs": [{"name": "case", "load": "checkpoint.json", "plan": {...}}]}.
Paths are relative to the invoking directory. This never builds or opens a GUI.
Each child uses explore_gameplay.py's fresh executable and process watchdog.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ("binary", "root", "manifest", "output"):
        p.add_argument(f"--{name}", type=Path, required=True)
    p.add_argument("--workers", type=int, default=2, choices=range(1, 5))
    p.add_argument("--timeout", type=float, default=900)
    a = p.parse_args()
    jobs = json.loads(a.manifest.read_text())["jobs"]
    names = [j["name"] for j in jobs]
    if len(set(names)) != len(names) or any(not re.fullmatch(r"[a-z0-9-]+", n) for n in names):
        p.error("unique safe case names required")
    a.output.mkdir()  # Never replace evidence.
    # Pin once for the entire campaign, not only once per child. A concurrent
    # build/cache cleanup must not change or remove the executable for later jobs.
    pinned = (a.output / "campaign-runner").absolute()
    with a.binary.open("rb") as source, pinned.open("xb") as target:
        shutil.copyfileobj(source, target)
    pinned.chmod(0o755)
    binary_hash = hashlib.sha256(pinned.read_bytes()).hexdigest()
    (a.output / "manifest.json").write_text(json.dumps({"jobs": jobs}, indent=2) + "\n")
    def run(job):
        name = job["name"]
        plan = a.output / f"{name}-plan.json"
        plan.write_text(json.dumps(job["plan"], indent=2) + "\n")
        start = time.monotonic()
        command = [sys.executable, str(Path(__file__).with_name("explore_gameplay.py")),
                   "--binary", str(pinned), "--root", str(a.root), "--load", job["load"],
                   "--plan", str(plan), "--output", str(a.output / name), "--timeout", str(a.timeout)]
        result = subprocess.run(command, capture_output=True, text=True)
        (a.output / f"{name}-driver.log").write_text(result.stdout + result.stderr)
        row = {"name": name, "exit_code": result.returncode, "seconds": time.monotonic()-start,
               "outcome": "process_failure"}
        report = a.output / name / "run.json"
        if report.exists():
            r = json.loads(report.read_text())
            last = next((s["after"] for s in reversed(r["steps"]) if s.get("after")), r["initial"])
            row.update(outcome=r["outcome"], completed_ticks=last["tick"]-r["initial"]["tick"],
                       tick_comparisons=r["tick_comparisons"], commands=len(r["steps"]),
                       cold_restores=r["cold_restores"], failure=r["failure"], stop=r["stop"])
        # A report that passed but failed during rendering/capture is not a pass.
        row["passed"] = result.returncode == 0 and row["outcome"] == "passed"
        return row
    rows = []
    with (a.output / "results.jsonl").open("x") as journal, ThreadPoolExecutor(max_workers=a.workers) as pool:
        for future in as_completed([pool.submit(run, j) for j in jobs]):
            row = future.result()
            rows.append(row)
            journal.write(json.dumps(row) + "\n")
            journal.flush()
            print(f'{row["name"]}: {row["outcome"]}, {row.get("completed_ticks", 0)} ticks', flush=True)
    summary = {"cases": sorted(rows, key=lambda r: r["name"]), "passed": sum(r["passed"] for r in rows),
               "total": len(rows), "binary_sha256": binary_hash, "original_engine_executed": False, "strict_native_parity_ticks": 0}
    (a.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    return 0 if summary["passed"] == summary["total"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
