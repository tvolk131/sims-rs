#!/usr/bin/env python3
"""Run a pinned, headless gameplay campaign with a process-level watchdog.

No build, GUI launch, original-engine run, or player-save write is performed.
The Rust runner refuses an existing output directory. Reports remain available
when a branch fails; this wrapper additionally captures panics/process hangs.
Executable, save, and plan copies are pinned; root assets remain mutable.
"""
import argparse
from contextlib import ExitStack
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import time


def digest(path):
    if not stat.S_ISREG(path.lstat().st_mode):
        raise OSError(f"not a regular file: {path}")
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
    pinned = args.output.with_name(args.output.name + "-runner")
    pinned_load = args.output.with_name(args.output.name + "-pinned-load.json")
    pinned_plan = args.output.with_name(args.output.name + "-pinned-plan.json")
    log_path = args.output.with_name(args.output.name + ".log")
    fallback = args.output.with_name(args.output.name + "-driver-manifest.json")
    for path in (args.output, pinned, pinned_load, pinned_plan, log_path, fallback):
        if os.path.lexists(path):
            parser.error(f"evidence already exists: {path}; use a new evidence directory")
    staged = {"binary": pinned, "load": pinned_load, "plan": pinned_plan}
    originals = {"binary": args.binary, "load": args.load, "plan": args.plan}
    with ExitStack() as evidence:
        fallback_file = evidence.enter_context(fallback.open("x"))
        log = evidence.enter_context(log_path.open("xb"))
        for name, path in staged.items():
            with originals[name].open("rb") as source, path.open("xb") as target:
                shutil.copyfileobj(source, target)
        pinned.chmod(0o755)
        load_bytes = pinned_load.read_bytes()
        identities = {"binary": digest(pinned), "load": hashlib.sha256(load_bytes).hexdigest(),
                      "plan": digest(pinned_plan)}
        loaded = json.loads(load_bytes)
        source = loaded.get("session", loaded)["source"]
        command = [str(pinned), "--original", source, "--root", str(args.root.absolute()),
                   "--load", str(pinned_load), "--exploration-plan", str(pinned_plan),
                   "--exploration-output", str(args.output)]
        started = time.monotonic()
        timed_out = False
        child_exit_code = None
        launch_error = None
        try:
            process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        except OSError as error:
            launch_error = str(error)
            code = 1
        else:
            try:
                code = process.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait()
                code = 124
            child_exit_code = process.returncode
        log.close()
        errors = []
        integrity = {}
        for name, path in staged.items():
            actual = None
            error_text = None
            try:
                actual = digest(path)
            except OSError as error:
                error_text = str(error)
            verified = actual == identities[name]
            integrity[name] = {"path": str(path), "pre_run_sha256": identities[name],
                               "post_run_sha256": actual, "verified": verified, "error": error_text}
            if not verified:
                errors.append(f"{name}: {error_text or 'staged bytes changed'}")
        try:
            log_hash = digest(log_path)
        except OSError as error:
            log_hash = None
            errors.append(f"log: {error}")
        artifacts = {}
        destination = fallback
        destination_file = fallback_file
        if args.output.is_dir() and not args.output.is_symlink():
            try:
                for path in sorted(args.output.rglob("*")):
                    if not path.is_dir() or path.is_symlink():
                        artifacts[str(path.relative_to(args.output))] = digest(path)
            except OSError as error:
                errors.append(f"artifacts: {error}")
            try:
                destination = args.output / "driver-manifest.json"
                destination_file = evidence.enter_context(destination.open("x"))
            except OSError as error:
                errors.append(f"manifest: {error}")
                destination = fallback
        elif os.path.lexists(args.output):
            errors.append("output is not a regular evidence directory")
        if errors and code == 0:
            code = 1
        manifest = {"schema": 1, "command": command, "exit_code": code, "timed_out": timed_out,
                    "seconds": time.monotonic() - started, "binary_sha256": identities["binary"],
                    "input_sha256": identities["load"], "plan_sha256": identities["plan"],
                    "log_sha256": log_hash, "original_engine_executed": False,
                    "child_exit_code": child_exit_code, "launch_error": launch_error,
                    "input_identity": "pre_run_staged_bytes", "input_integrity": integrity,
                    "evidence_integrity": not errors, "evidence_errors": errors,
                    "root_assets_pinned": False,
                    "source_paths": {name: str(path.absolute()) for name, path in originals.items()}}
        if args.output.is_dir() and not args.output.is_symlink():
            manifest["artifacts"] = artifacts
        manifest_files = [fallback_file]
        if destination_file is not fallback_file:
            manifest_files.append(destination_file)
        for file in manifest_files:
            json.dump(manifest, file, indent=2)
            file.write("\n")
    print(f"exit={code}, timeout={timed_out}; evidence: {destination}")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
