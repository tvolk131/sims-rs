import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


PACKAGE_SCRIPT = Path(__file__).resolve().with_name("package-room-macos.sh")
STUB_CARGO = """#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

with Path("cargo-invocations.jsonl").open("a", encoding="utf-8") as log:
    log.write(json.dumps(sys.argv[1:]) + "\\n")
if os.environ.get("STUB_FAIL"):
    sys.exit(19)
profile = "release" if "--release" in sys.argv else "debug"
binary = Path("target", profile, "sims-room")
binary.parent.mkdir(parents=True, exist_ok=True)
binary.write_bytes(os.environ.get("STUB_BYTES", "first executable\\n").encode())
if os.environ.get("STUB_MUTATE"):
    with Path(os.environ["STUB_MUTATE"]).open("a", encoding="utf-8") as source:
        source.write("changed during build\\n")
if os.environ.get("STUB_DELETE"):
    Path(os.environ["STUB_DELETE"]).unlink()
"""


class PackageRoomMacosTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="sims-room-package-")
        self.addCleanup(temporary.cleanup)
        self.workspace = Path(temporary.name, "workspace ' with spaces")
        self.workspace.mkdir()
        self.write("scripts/package-room-macos.sh", PACKAGE_SCRIPT.read_text())
        self.write("Cargo.toml", '[workspace]\nmembers = ["crates/sims-app"]\n')
        self.write("Cargo.lock", "version = 4\n")
        self.write("crates/sims-app/Cargo.toml", '[package]\nname = "sims-app"\n')
        self.write("crates/sims-app/src/main.rs", "fn main() {}\n")
        cargo = self.write("bin/cargo", STUB_CARGO)
        cargo.chmod(0o755)
        self.environment = {
            name: value for name, value in os.environ.items()
            if not name.startswith(("GIT_", "STUB_"))
        }
        self.environment["PATH"] = str(cargo.parent) + os.pathsep + os.environ["PATH"]

    def write(self, relative, content):
        path = self.workspace / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def package(self, *arguments, **environment):
        return subprocess.run(
            ["sh", str(self.workspace / "scripts/package-room-macos.sh"), *arguments],
            cwd=self.workspace.parent,
            env={**self.environment, **environment},
            capture_output=True,
            text=True,
            timeout=15,
        )

    def assert_packaged(self, *arguments, **environment):
        result = self.package(*arguments, **environment)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def bundle(self, profile="debug"):
        return self.workspace / "target" / profile / "Sims Room.app"

    def binary(self, profile="debug"):
        return self.bundle(profile) / "Contents/MacOS/sims-room"

    def receipt(self, profile="debug"):
        return json.loads((self.bundle(profile) / "Contents/Resources/build-receipt.json").read_text())

    def fingerprint(self):
        return self.receipt()["workspace_source"]["sha256"]

    def snapshot(self):
        return {
            str(path.relative_to(self.bundle())): (path.stat().st_ino, path.read_bytes())
            for path in self.bundle().rglob("*") if path.is_file()
        }

    def test_debug_and_release_select_expected_cargo_command(self):
        for arguments, profile in (((), "debug"), (("--release",), "release")):
            with self.subTest(profile=profile):
                self.assert_packaged(*arguments)
                self.assertTrue(self.binary(profile).is_file())
                self.assertEqual(self.receipt(profile)["profile"], profile)
        calls = (self.workspace / "cargo-invocations.jsonl").read_text().splitlines()
        self.assertEqual([json.loads(call) for call in calls], [
            ["build", "--locked", "-p", "sims-app", "--bin", "sims-room"],
            ["build", "--locked", "--release", "-p", "sims-app", "--bin", "sims-room"],
        ])

    def test_receipt_identifies_actual_packaged_bytes_without_git(self):
        from datetime import datetime, timezone

        self.assert_packaged(STUB_BYTES="the actual packaged bytes\n")
        receipt = self.receipt()
        self.assertEqual(receipt["schema"], "sims-room-build-receipt-v1")
        self.assertEqual(receipt["git"], {"head": None, "dirty": None})
        self.assertEqual(receipt["executable_sha256"], hashlib.sha256(self.binary().read_bytes()).hexdigest())
        self.assertEqual(receipt["workspace_source"]["scope"], "sims-room-workspace-source-v2")
        self.assertRegex(self.fingerprint(), r"^[0-9a-f]{64}$")
        self.assertTrue(receipt["build_time_utc"].endswith("Z"))
        timestamp = datetime.fromisoformat(receipt["build_time_utc"].replace("Z", "+00:00"))
        self.assertEqual(timestamp.utcoffset(), timezone.utc.utcoffset(timestamp))
        self.assertTrue(os.access(self.binary(), os.X_OK))
        self.assertFalse(list((self.bundle() / "Contents").glob(".package-room.*")))

    def test_republication_preserves_open_and_hardlinked_prior_inode(self):
        self.assert_packaged()
        previous_bytes = self.binary().read_bytes()
        previous_inode = self.binary().stat().st_ino
        hardlink = self.workspace / "prior executable"
        os.link(self.binary(), hardlink)
        with self.binary().open("rb") as opened:
            self.assert_packaged(STUB_BYTES="replacement executable\n")
            self.assertNotEqual(self.binary().stat().st_ino, previous_inode)
            self.assertEqual(opened.read(), previous_bytes)
            self.assertEqual(os.fstat(opened.fileno()).st_ino, previous_inode)
            self.assertEqual(hardlink.read_bytes(), previous_bytes)
            self.assertEqual(hardlink.stat().st_ino, previous_inode)
        self.assertEqual(self.binary().read_bytes(), b"replacement executable\n")
        self.assertEqual(self.receipt()["executable_sha256"], hashlib.sha256(self.binary().read_bytes()).hexdigest())

    def test_available_git_metadata_is_recorded(self):
        git = self.write("bin/git", """#!/bin/sh
case "$*" in
    'rev-parse --show-prefix') printf '\\n' ;;
    'rev-parse --verify HEAD') printf '%s\\n' aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ;;
    'status --porcelain=v1 --untracked-files=normal') printf '%s' "${STUB_GIT_STATUS:-}" ;;
    *) exit 1 ;;
esac
""")
        git.chmod(0o755)
        self.assert_packaged(STUB_GIT_STATUS="?? untracked.rs\n")
        self.assertEqual(self.receipt()["git"], {"head": "a" * 40, "dirty": True})
        self.assert_packaged()
        self.assertEqual(self.receipt()["git"], {"head": "a" * 40, "dirty": False})

    def test_failed_cargo_leaves_existing_package_untouched(self):
        self.assert_packaged()
        previous = self.snapshot()
        result = self.package(STUB_FAIL="1")
        self.assertEqual(result.returncode, 19, result.stderr)
        self.assertEqual(self.snapshot(), previous)

    def test_sources_changed_during_build_refuse_publication(self):
        self.assert_packaged()
        previous = self.snapshot()
        for environment in (
            {"STUB_MUTATE": "crates/sims-app/src/main.rs"},
            {"STUB_MUTATE": "crates/sims-app/src/new.rs"},
            {"STUB_DELETE": "crates/sims-app/src/new.rs"},
            {"STUB_MUTATE": "crates/sims-app/src/new.json"},
            {"STUB_DELETE": "crates/sims-app/src/new.json"},
            {"STUB_MUTATE": "Cargo.lock"},
        ):
            with self.subTest(environment=environment):
                result = self.package(**environment)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("sources changed during Cargo build", result.stderr)
                self.assertEqual(self.snapshot(), previous)

    def test_invalid_arguments_do_not_invoke_cargo(self):
        for arguments in (("--debug",), ("",), ("--release", "extra"), ("", "--release")):
            with self.subTest(arguments=arguments):
                result = self.package(*arguments)
                self.assertEqual(result.returncode, 2)
                self.assertIn("Usage:", result.stderr)
        self.assertFalse((self.workspace / "cargo-invocations.jsonl").exists())
        self.assertFalse((self.workspace / "target").exists())

    def test_fingerprint_is_stable_and_includes_untracked_source_paths_and_bytes(self):
        self.assert_packaged()
        original = self.fingerprint()
        for relative in (
            "target/generated.rs", "game-data/private.rs", "assets/private.wgsl",
            "cache/capture.rs", "docs/research/capture.rs",
            "crates/sims-app/target/generated.rs", "crates/sims-app/assets/private.wgsl",
            "crates/sims-app/fixtures/private/capture.rs",
            "crates/sims-app/research/captures/capture.rs",
            "crates/sims-vm/tests/fixtures/private/managed-save.json",
            "game-data/private.json",
        ):
            self.write(relative, "not a source input\n")
        self.assert_packaged()
        self.assertEqual(self.fingerprint(), original)
        source = self.write("crates/sims-app/src/new 'shader\nsource.wgsl", "first bytes\n")
        self.assert_packaged()
        added = self.fingerprint()
        self.assertNotEqual(added, original)
        source.write_text("other bytes\n")
        self.assert_packaged()
        modified = self.fingerprint()
        self.assertNotEqual(modified, added)
        source = source.rename(source.with_name("renamed.wgsl"))
        self.assert_packaged()
        self.assertNotEqual(self.fingerprint(), modified)
        source.unlink()
        self.assert_packaged()
        self.assertEqual(self.fingerprint(), original)

    def test_fingerprint_includes_authored_json_sources_and_fixtures(self):
        self.assert_packaged()
        original = self.fingerprint()
        for relative in (
            "crates/sims-vm/src/inputs 'with\nspaces.json",
            "crates/sims-app/tests/fixtures/compiled-input.json",
        ):
            with self.subTest(relative=relative):
                source = self.write(relative, '{"value":1}\n')
                self.assert_packaged()
                added = self.fingerprint()
                self.assertNotEqual(added, original)
                source.write_text('{"value":2}\n', encoding="utf-8")
                self.assert_packaged()
                self.assertNotEqual(self.fingerprint(), added)
                source.unlink()
                self.assert_packaged()
                self.assertEqual(self.fingerprint(), original)

    def test_cargo_configuration_manifests_and_source_modules_affect_fingerprint(self):
        self.assert_packaged()
        previous = self.fingerprint()
        for relative in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
                         "crates/sims-app/Cargo.toml", "crates/sims-app/build.rs",
                         "crates/sims-app/src/assets/authored.rs"):
            with self.subTest(relative=relative):
                self.write(relative, "changed source input\n")
                self.assert_packaged()
                self.assertNotEqual(self.fingerprint(), previous)
                previous = self.fingerprint()

    def test_symlinked_source_is_rejected_without_reading_target(self):
        self.assert_packaged()
        previous = self.snapshot()
        link = self.workspace / "crates/sims-app/src/linked.rs"
        link.symlink_to(self.workspace / "game-data/missing-private.rs")
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Refusing symlinked source input", result.stderr)
        self.assertEqual(self.snapshot(), previous)
        self.assertEqual(len((self.workspace / "cargo-invocations.jsonl").read_text().splitlines()), 1)


if __name__ == "__main__":
    unittest.main()
