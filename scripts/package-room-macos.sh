#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
case "$#:${1:-}" in
    0:) profile=debug ;;
    1:--release) profile=release ;;
    *) printf '%s\n' 'Usage: package-room-macos.sh [--release]' >&2; exit 2 ;;
esac

source_fingerprint() {
    python3 - <<'PY'
import hashlib
import os
from pathlib import Path

excluded = {".git", "target", "node_modules"}
data_directories = {"assets", "game-data", "extracted", "cache", "private", "research", "captures"}
root_inputs = ("Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain",
               "rust-toolchain.toml", ".cargo/config", ".cargo/config.toml")
sources = []

def check_link(path):
    if path.is_symlink():
        raise SystemExit(f"Refusing symlinked source input: {path}")

for name in root_inputs:
    path = Path(name)
    for parent in path.parents:
        check_link(parent)
    check_link(path)
    if path.is_file():
        sources.append(path)

def walk_error(error):
    raise error

check_link(Path("crates"))
for directory, directories, filenames in os.walk("crates", onerror=walk_error):
    parent = Path(directory)
    data_location = len(parent.parts) <= 2 or parent.name == "fixtures"
    directories[:] = [name for name in directories if name not in excluded
                      and not (data_location and name in data_directories)]
    for name in directories:
        check_link(Path(directory, name))
    for name in filenames:
        path = Path(directory, name)
        if (path.suffix in {".rs", ".wgsl", ".json"}
                or name in {"Cargo.toml", "Cargo.lock"}
                or (path.parent.name == ".cargo" and name in {"config", "config.toml"})):
            check_link(path)
            sources.append(path)

fingerprint = hashlib.sha256(b"sims-room-workspace-source-v2\0")
for path in sorted(sources, key=lambda source: os.fsencode(source.as_posix())):
    encoded = os.fsencode(path.as_posix())
    content = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            content.update(chunk)
    fingerprint.update(len(encoded).to_bytes(8, "big"))
    fingerprint.update(encoded)
    fingerprint.update(content.digest())
print(fingerprint.hexdigest())
PY
}

source_before=$(source_fingerprint)
if [ "$profile" = release ]; then
    cargo build --locked --release -p sims-app --bin sims-room
else
    cargo build --locked -p sims-app --bin sims-room
fi
source_after=$(source_fingerprint)
if [ "$source_before" != "$source_after" ]; then
    printf '%s\n' 'Workspace sources changed during Cargo build; package not replaced. Retry with stable sources.' >&2
    exit 1
fi

bundle="target/$profile/Sims Room.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
staging=$(mktemp -d "$bundle/Contents/.package-room.XXXXXX")
trap 'rm -f "$staging/sims-room" "$staging/build-receipt.json" "$staging/Info.plist"; rmdir "$staging"' 0
trap 'exit 1' 1 2 15
# Publish a fresh executable inode; never overwrite a running Mach-O in place.
install -m 755 "target/$profile/sims-room" "$staging/sims-room"
python3 - "$profile" "$source_after" "$staging/sims-room" "$staging/build-receipt.json" <<'PY'
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import subprocess
import sys

def git_output(*arguments):
    try:
        return subprocess.run(["git", *arguments], check=True,
                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL).stdout
    except (OSError, subprocess.CalledProcessError):
        return None

head = dirty = None
if git_output("rev-parse", "--show-prefix") == b"\n":
    revision = git_output("rev-parse", "--verify", "HEAD")
    status = git_output("status", "--porcelain=v1", "--untracked-files=normal")
    head = revision.decode("ascii").strip() if revision is not None else None
    dirty = bool(status) if status is not None else None

executable = hashlib.sha256()
with Path(sys.argv[3]).open("rb") as binary:
    for chunk in iter(lambda: binary.read(1024 * 1024), b""):
        executable.update(chunk)
receipt = {
    "schema": "sims-room-build-receipt-v1",
    "build_time_utc": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
    "profile": sys.argv[1],
    "executable_sha256": executable.hexdigest(),
    "git": {"head": head, "dirty": dirty},
    "workspace_source": {"scope": "sims-room-workspace-source-v2", "sha256": sys.argv[2]},
}
Path(sys.argv[4]).write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
PY
cat > "$staging/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Sims Room</string>
<key>CFBundleDisplayName</key><string>Sims Room</string>
<key>CFBundleIdentifier</key><string>local.sims-rs.room</string>
<key>CFBundleExecutable</key><string>sims-room</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>0.1.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
mv -f "$staging/sims-room" "$bundle/Contents/MacOS/sims-room"
mv -f "$staging/build-receipt.json" "$bundle/Contents/Resources/build-receipt.json"
mv -f "$staging/Info.plist" "$bundle/Contents/Info.plist"
printf '%s\n' "Created $bundle"
printf '%s\n' "Build receipt: $bundle/Contents/Resources/build-receipt.json"
