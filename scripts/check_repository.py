import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys


POLICY_PATH = ".github/repository-policy.json"
PRIVATE_ROOTS = {
    "cache", "extracted", "game-data", "rust-simplicity", "saves", "target",
    "userdata",
}
PRIVATE_COMPONENTS = {".aws", ".gnupg", ".ssh"}
PUBLIC_DOCUMENTS = {
    "docs/legal-and-provenance.md", "docs/dependency-licenses.md",
    "docs/repository-workflow.md",
    "docs/native-window-session-20261103.md",
    "docs/performance-preview.md",
    "docs/playable-alpha.md",
}
PRIVATE_PATHS = {
    "crates/sims-app/src/original_viewer/exploration/startup_play/host_journey/tutorial_camera/go_here/walking/tests.rs",
    "crates/sims-render/tests/clown_household_continuation.rs",
    "crates/sims-render/tests/fresh_household_replay.rs",
    "crates/sims-vm/src/native_registry_reentry/tests.rs",
    "crates/sims-vm/src/native_suspension/tests.rs",
    "crates/sims-vm/src/retained_epoch/tests.rs",
    "crates/sims-vm/src/scheduler/storage/word_tail_tests.rs",
    "crates/sims-app/src/original_viewer_continuation_tests.rs",
    "crates/sims-app/src/tutorial_presentation_tests.rs",
    "crates/sims-sim/src/pose/transform/retained-goth.json",
    "crates/sims-content/src/autonomy_resources/expected-final-process.json",
}
FORBIDDEN_SUFFIXES = {
    ".7z", ".app", ".bcf", ".bin", ".bmf", ".bmp", ".cfp", ".cmx",
    ".dll", ".dmg", ".dylib", ".exe", ".far", ".iff", ".iso", ".key",
    ".mp3", ".o", ".p12", ".pem", ".pfx", ".rar", ".sav", ".skn",
    ".so", ".tar", ".tgz", ".wav", ".xa", ".zip",
    ".cab", ".dat", ".flac", ".gif", ".gz", ".ico", ".jpeg", ".jpg",
    ".jsonl", ".mp4", ".ogg", ".otf", ".pdf", ".png", ".qcow2",
    ".ttf", ".vdi", ".webm", ".webp",
}
HEX_RUN = re.compile(rb"(?<![0-9a-fA-F])[0-9a-fA-F]{16,}(?![0-9a-fA-F])")
BYTE_ARRAY = re.compile(rb"\[\s*(?:0x[0-9a-fA-F]{1,2}|\d{1,3})(?:\s*,\s*(?:0x[0-9a-fA-F]{1,2}|\d{1,3})){2,}\s*,?\s*\]")
HEX_STRING = re.compile(rb'''["']([0-9a-fA-F]{2,})["']''')
RAW_STRING_LITERAL = re.compile(rb'r(?P<hashes>#{0,255})"(?P<body>.*?)"(?P=hashes)', re.DOTALL)
STRING_LITERAL = re.compile(rb'"(?:[^"\\\r\n]|\\.)*"')
SECRET_PATTERNS = {
    "private key": re.compile(
        rb"-----BEGIN (?:RSA |DSA |EC |OPENSSH |ENCRYPTED )?PRIVATE KEY-----"
    ),
    "GitHub token": re.compile(
        rb"(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{60,})"
    ),
    "AWS access key": re.compile(rb"(?:AKIA|ASIA)[A-Z0-9]{16}"),
    "OpenAI-style token": re.compile(rb"sk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{40,}"),
}


def git(root, *arguments, input_bytes=None):
    return subprocess.check_output(
        ["git", "-C", str(root), *arguments], input=input_bytes
    )


def path_violation(name):
    path = PurePosixPath(name.lower())
    if path.parts[0] in PRIVATE_ROOTS or PRIVATE_COMPONENTS.intersection(path.parts):
        return "local/private directory"
    if path.parts[:2] == ("fixtures", "private"):
        return "private fixture directory"
    if path.parts[:2] == ("scripts", "research"):
        return "private research tooling"
    if any(path.parts[index:index + 2] == ("tests", "fixtures") for index in range(len(path.parts) - 1)):
        return "private reference fixture corpus"
    if path.as_posix() in PRIVATE_PATHS or path.name == "private_tests.rs":
        return "private source-level reference evidence"
    if path.parts[0] == "docs" and path.as_posix() not in PUBLIC_DOCUMENTS:
        return "unreviewed historical research document"
    if path.name == "sims-rs.local.toml":
        return "local configuration"
    if path.name == ".env" or (
        path.name.startswith(".env.")
        and path.name not in {".env.example", ".env.sample", ".env.template"}
    ):
        return "environment secrets file"
    if path.name in {"id_rsa", "id_dsa", "id_ecdsa", "id_ed25519", ".ds_store"}:
        return "local credentials/platform file"
    if path.suffix in FORBIDDEN_SUFFIXES:
        return "original-content, credential, archive or build-output extension"
    return None


def load_policy(root, staged, revision=None):
    if revision:
        commit = git(root, "rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}").decode().strip()
        content = git(root, "show", f"{commit}:{POLICY_PATH}")
    elif staged:
        content = git(root, "show", f":{POLICY_PATH}")
    else:
        content = (root / POLICY_PATH).read_bytes()
    policy = json.loads(content)
    if policy["schema"] != 2 or not 0 < policy["max_blob_bytes"] <= 5 * 1024 * 1024:
        raise ValueError("unsupported policy schema or size limit above 5 MiB")
    if not 0 < policy["max_tree_bytes"] <= 500 * 1024 * 1024:
        raise ValueError("unsupported tree budget above 500 MiB")
    for entry in policy["reviewed_data"] + policy["blocked_payloads"]:
        if not re.fullmatch(r"[a-f0-9]{64}", entry["sha256"]) or entry["bytes"] < 0:
            raise ValueError("invalid publication fingerprint")
    if any(not entry["provenance"].strip() for entry in policy["reviewed_data"]):
        raise ValueError("reviewed data requires a provenance statement")
    return policy


def decoded_candidates(content):
    yield content
    for match in HEX_RUN.finditer(content):
        raw = match.group()
        if len(raw) % 2 == 0:
            yield bytes.fromhex(raw.decode("ascii"))
    for match in BYTE_ARRAY.finditer(content):
        values = [
            int(value, 16 if value.startswith(b"0x") else 10)
            for value in re.findall(rb"0x[0-9a-fA-F]{1,2}|\d{1,3}", match.group())
        ]
        if all(value <= 255 for value in values):
            yield bytes(values)
    for match in HEX_STRING.finditer(content):
        if len(match.group(1)) % 2 == 0:
            yield bytes.fromhex(match.group(1).decode("ascii"))
    for match in RAW_STRING_LITERAL.finditer(content):
        yield match.group("body")
    for match in STRING_LITERAL.finditer(content):
        try:
            yield json.loads(match.group()).encode("utf-8")
        except (UnicodeError, ValueError):
            continue


def check_blob(root, policy, name, content, object_id=None):
    violations = []
    reason = path_violation(name)
    if reason:
        violations.append(f"{name}: {reason}")
    for label, pattern in SECRET_PATTERNS.items():
        if pattern.search(content):
            violations.append(f"{name}: possible {label}; value redacted")
    if b"\0" in content:
        violations.append(f"{name}: binary content is not admitted by the source-only policy")
    else:
        try:
            content.decode("utf-8")
        except UnicodeDecodeError:
            violations.append(f"{name}: non-UTF-8 binary content is not admitted by the source-only policy")
    if PurePosixPath(name).suffix.lower() == ".json" and name != POLICY_PATH:
        admitted = any(
            entry["path"] == name
            and entry["bytes"] == len(content)
            and entry["sha256"] == hashlib.sha256(content).hexdigest()
            for entry in policy["reviewed_data"]
        )
        if not admitted:
            violations.append(f"{name}: JSON data requires an exact-byte provenance review")
    fingerprints = {
        (entry["bytes"], entry["sha256"]) for entry in policy["blocked_payloads"]
    }
    lengths = {length for length, digest in fingerprints}
    if any(
        len(candidate) in lengths
        and (len(candidate), hashlib.sha256(candidate).hexdigest()) in fingerprints
        for candidate in decoded_candidates(content)
    ):
        violations.append(f"{name}: known private game payload; content redacted")
    if PurePosixPath(name).suffix.lower() in {".rs", ".py", ".json"} and any(
        len(match.group()) > 256 for match in HEX_RUN.finditer(content)
    ):
        violations.append(f"{name}: long encoded payload requires a provenance review")
    large = len(content) > policy["max_blob_bytes"]
    if large:
        if object_id is None:
            object_id = git(root, "hash-object", "--stdin", input_bytes=content).decode().strip()
        admitted = any(
            entry["path"] == name
            and entry["object"] == object_id
            and entry["bytes"] == len(content)
            for entry in policy["large_blob_allowlist"]
        )
        if not admitted:
            violations.append(
                f"{name}: {len(content):,} bytes exceeds {policy['max_blob_bytes']:,}; "
                "compress/reduce the file or explicitly review an exact-blob exception"
            )
    return violations, large


def index_entries(root):
    for record in git(root, "ls-files", "--stage", "-z").split(b"\0"):
        if record:
            metadata, name = record.split(b"\t", 1)
            mode, object_id, stage = metadata.decode().split()
            yield mode, object_id, stage, os.fsdecode(name)


def history_entries(root, revision):
    commit = git(
        root, "rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}"
    ).decode().strip()
    objects = git(root, "rev-list", "--objects", commit, "--")
    metadata = git(
        root, "cat-file", "--batch-check=%(objecttype) %(objectname) %(objectsize) %(rest)",
        input_bytes=objects,
    )
    entries = []
    for record in metadata.decode().splitlines():
        kind, object_id, size, *name = record.split(" ", 3)
        if kind == "blob":
            entries.append((object_id, int(size), name[0] if name else ""))
    names = git(
        root, "log", "--format=", "--name-only", "-z", "--no-renames",
        "--diff-filter=ACMR", commit, "--",
    )
    paths = {os.fsdecode(name) for name in names.split(b"\0") if name}
    return entries, paths


def read_blobs(root, entries):
    with subprocess.Popen(
        ["git", "-C", str(root), "cat-file", "--batch"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    ) as process:
        try:
            for object_id, size, name in entries:
                process.stdin.write((object_id + "\n").encode())
                process.stdin.flush()
                header = process.stdout.readline().decode().split()
                if header != [object_id, "blob", str(size)]:
                    raise ValueError("unexpected git cat-file header")
                content = process.stdout.read(size)
                if len(content) != size or process.stdout.read(1) != b"\n":
                    raise ValueError("truncated git cat-file output")
                yield object_id, name, content
        finally:
            process.stdin.close()
            process.stdout.close()
        if process.wait() != 0:
            raise ValueError("git cat-file failed")


def working_entries(root):
    names = git(root, "ls-files", "--cached", "--others", "--exclude-standard", "-z")
    for raw in dict.fromkeys(names.split(b"\0")):
        if not raw:
            continue
        name = os.fsdecode(raw)
        path = root / name
        if path.is_symlink():
            yield None, name, os.fsencode(os.readlink(path))
        elif path.is_file():
            yield None, name, path.read_bytes()
        elif path.is_dir():
            yield None, name, None


def audit(root, policy, arguments):
    violations = []
    count = total_bytes = large_count = 0
    if arguments.history:
        entries, paths = history_entries(root, arguments.history)
        violations.extend(
            f"{name}: historical {reason}"
            for name in sorted(paths) if (reason := path_violation(name))
        )
        blobs = read_blobs(root, entries)
    elif arguments.staged:
        entries = []
        for mode, object_id, stage, name in index_entries(root):
            if stage != "0" or mode == "160000":
                violations.append(f"{name}: unmerged entry or nested Git checkout")
                continue
            entries.append((object_id, name))
        metadata = git(
            root, "cat-file", "--batch-check=%(objectname) %(objecttype) %(objectsize)",
            input_bytes="".join(object_id + "\n" for object_id, name in entries).encode(),
        ).decode().splitlines()
        if len(metadata) != len(entries):
            raise ValueError("incomplete index metadata")
        sized_entries = []
        for (object_id, name), row in zip(entries, metadata):
            returned, kind, size = row.split()
            if returned != object_id or kind != "blob":
                raise ValueError("unexpected index object")
            sized_entries.append((object_id, int(size), name))
        blobs = read_blobs(root, sized_entries)
    else:
        blobs = working_entries(root)
    for object_id, name, content in blobs:
        if content is None:
            violations.append(f"{name}: nested Git checkout")
            continue
        errors, large = check_blob(root, policy, name, content, object_id)
        violations.extend(errors)
        count += 1
        total_bytes += len(content)
        large_count += large
    if not arguments.history and total_bytes > policy["max_tree_bytes"]:
        violations.append(
            f"total tree size {total_bytes:,} exceeds {policy['max_tree_bytes']:,} bytes"
        )
    return sorted(set(violations)), count, total_bytes, large_count


def main():
    parser = argparse.ArgumentParser(description="Check source-publication hygiene without printing secret or private payload values.")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--staged", action="store_true", help="check index bytes and staged policy")
    modes.add_argument("--history", metavar="REVISION", help="check all history reachable from a commit")
    arguments = parser.parse_args()
    try:
        root = Path(git(Path.cwd(), "rev-parse", "--show-toplevel").decode().strip())
        policy = load_policy(root, arguments.staged, arguments.history)
        violations, count, total_bytes, large_count = audit(root, policy, arguments)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"Repository hygiene could not complete: {error}", file=sys.stderr)
        return 1
    for violation in violations:
        print(f"ERROR: {violation}", file=sys.stderr)
    print(
        f"Repository hygiene: {count} files/blobs, {total_bytes / 1024**2:.2f} MiB, "
        f"{large_count} large blobs, {len(violations)} violations. "
        "Secret and provenance checks are bounded heuristics, not legal clearance."
    )
    return 1 if violations else 0


if __name__ == "__main__":
    sys.exit(main())
