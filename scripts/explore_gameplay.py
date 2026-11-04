#!/usr/bin/env python3
"""Run a pinned, headless gameplay campaign with a process-level watchdog.

No build, GUI launch, original-engine run, or player-save write is performed.
The Rust runner refuses an existing output directory. Reports remain available
when a branch fails; this wrapper additionally captures panics/process hangs.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("binary", "root", "load", "plan", "output"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    parser.add_argument("--timeout", type=float, default=900)
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("timeout must be positive")
    args.output = args.output.absolute()
    if args.output.exists():
        parser.error("output already exists; use a new evidence directory")
    # A fresh inode prevents a concurrent Cargo link from replacing the image
    # mapped by the running process. The pinned image remains with the evidence.
    pinned = args.output.with_name(args.output.name + "-runner")
    with args.binary.open("rb") as source, pinned.open("xb") as target:
        shutil.copyfileobj(source, target)
    pinned.chmod(0o755)
    loaded = json.loads(args.load.read_bytes())
    source = loaded.get("session", loaded)["source"]
    command = [str(pinned), "--original", source, "--root", str(args.root.absolute()),
               "--load", str(args.load.absolute()), "--exploration-plan", str(args.plan.absolute()),
               "--exploration-output", str(args.output)]
    log_path = args.output.with_name(args.output.name + ".log")
    started = time.monotonic()
    timed_out = False
    with log_path.open("xb") as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            code = process.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
            code = 124
    manifest = {"schema": 1, "command": command, "exit_code": code, "timed_out": timed_out,
                "seconds": time.monotonic() - started, "binary_sha256": digest(pinned),
                "input_sha256": digest(args.load), "plan_sha256": digest(args.plan),
                "log_sha256": digest(log_path), "original_engine_executed": False}
    if args.output.is_dir():
        manifest["artifacts"] = {str(p.relative_to(args.output)): digest(p)
                                 for p in sorted(args.output.rglob("*")) if p.is_file()}
        destination = args.output / "driver-manifest.json"
    else:
        destination = args.output.with_name(args.output.name + "-driver-manifest.json")
    with destination.open("x") as file:
        json.dump(manifest, file, indent=2)
        file.write("\n")
    print(f"exit={code}, timeout={timed_out}; evidence: {destination}")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
