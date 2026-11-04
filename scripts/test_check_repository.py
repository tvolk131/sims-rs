import json
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().with_name("check_repository.py")


class RepositoryHygieneTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="sims-repository-hygiene-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.environment = {
            name: value for name, value in os.environ.items()
            if not name.startswith("GIT_")
        }
        self.environment.update({
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": "Test",
            "GIT_AUTHOR_EMAIL": "test@example.invalid",
            "GIT_COMMITTER_NAME": "Test",
            "GIT_COMMITTER_EMAIL": "test@example.invalid",
        })
        self.git("init", "--quiet")
        self.write(".gitignore", "/cache/\n.env\n")
        self.policy = {
            "schema": 2, "max_blob_bytes": 1024, "max_tree_bytes": 1024 * 1024,
            "large_blob_allowlist": [],
            "reviewed_data": [], "blocked_payloads": [],
        }
        self.save_policy()

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(content, bytes):
            path.write_bytes(content)
        else:
            path.write_text(content, encoding="utf-8")
        return path

    def git(self, *arguments, input_bytes=None):
        return subprocess.check_output(
            ["git", *arguments], cwd=self.root, env=self.environment, input=input_bytes
        )

    def save_policy(self):
        self.write(".github/repository-policy.json", json.dumps(self.policy))

    def check(self, *arguments):
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPT), *arguments],
            cwd=self.root, env=self.environment, capture_output=True, text=True, timeout=15,
        )

    def assert_passes(self, *arguments):
        result = self.check(*arguments)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def assert_fails(self, message, *arguments):
        result = self.check(*arguments)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(message, result.stderr)
        return result

    def snapshot(self, parent=None):
        tree = self.git("write-tree").decode().strip()
        arguments = ["commit-tree", tree, "-m", "isolated test snapshot"]
        if parent:
            arguments.extend(["-p", parent])
        return self.git(*arguments).decode().strip()

    def test_small_source_passes(self):
        self.write("src/main.rs", "fn main() {}\n")
        self.assert_passes()

    def test_new_large_file_fails(self):
        self.write("fixtures/new.json", b"a" * 2048)
        self.assert_fails("exceeds 1,024")

    def test_many_small_files_cannot_exceed_tree_budget(self):
        self.policy["max_tree_bytes"] = 1024
        self.save_policy()
        for number in range(6):
            self.write(f"fixtures/{number}.txt", b"a" * 300)
        self.assert_fails("total tree size")

    def test_history_versions_do_not_count_against_checkout_budget(self):
        self.policy["max_tree_bytes"] = 1024
        self.save_policy()
        self.write("source.txt", b"a" * 700)
        self.git("add", ".")
        first = self.snapshot()
        self.write("source.txt", b"b" * 700)
        self.git("add", ".")
        second = self.snapshot(first)
        self.assert_passes()
        self.assert_passes("--history", second)

    def test_exact_large_blob_exception_does_not_allow_mutation(self):
        content = b"a" * 2048
        self.write("fixtures/existing.txt", content)
        object_id = self.git("hash-object", "--stdin", input_bytes=content).decode().strip()
        self.policy["large_blob_allowlist"] = [
            {"path": "fixtures/existing.txt", "object": object_id, "bytes": len(content)}
        ]
        self.save_policy()
        self.assert_passes()
        self.write("fixtures/existing.txt", b"b" * 2048)
        self.assert_fails("exceeds 1,024")

    def test_exception_does_not_allow_same_content_under_another_path(self):
        content = b"a" * 2048
        object_id = self.git("hash-object", "--stdin", input_bytes=content).decode().strip()
        self.policy["large_blob_allowlist"] = [
            {"path": "fixtures/existing.txt", "object": object_id, "bytes": len(content)}
        ]
        self.save_policy()
        self.write("fixtures/new.txt", content)
        self.assert_fails("fixtures/new.txt")

    def test_possible_secret_is_redacted(self):
        token = "ghp_" + "A" * 36
        self.write("settings.txt", token)
        result = self.assert_fails("possible GitHub token")
        self.assertNotIn(token, result.stdout + result.stderr)

    def test_private_key_and_aws_patterns(self):
        for content, label in [
            ("-----BEGIN " + "PRIVATE KEY-----", "private key"),
            ("AKIA" + "Z" * 16, "AWS access key"),
        ]:
            with self.subTest(label=label):
                self.write("settings.txt", content)
                self.assert_fails(label)

    def test_ignored_outputs_are_not_read_but_forced_tracking_is_rejected(self):
        self.write("cache/large.json", b"a" * 2048)
        self.assert_passes()
        self.git("add", "-f", "cache/large.json")
        self.assert_fails("local/private directory")

    def test_original_asset_and_environment_paths_are_rejected(self):
        for name in ["assets/House01.iff", "fixtures/private/capture.json", "config/.env.local"]:
            with self.subTest(name=name):
                path = self.write(name, "small")
                self.assert_fails(name)
                path.unlink()

    def test_staged_mode_reads_index_not_worktree_bytes(self):
        self.write("source.txt", "small")
        self.git("add", ".")
        self.write("source.txt", b"a" * 2048)
        self.assert_passes("--staged")
        self.git("add", "source.txt")
        self.write("source.txt", "small again")
        self.assert_fails("source.txt", "--staged")

    def test_staged_policy_cannot_be_relaxed_by_unstaged_changes(self):
        self.write("source.txt", b"a" * 2048)
        self.git("add", ".")
        content = b"a" * 2048
        object_id = self.git("hash-object", "--stdin", input_bytes=content).decode().strip()
        self.policy["large_blob_allowlist"] = [
            {"path": "source.txt", "object": object_id, "bytes": len(content)}
        ]
        self.save_policy()
        self.assert_passes()
        self.assert_fails("source.txt", "--staged")

    def test_nested_gitlink_is_rejected(self):
        object_id = self.git("hash-object", "-w", "--stdin", input_bytes=b"test").decode().strip()
        self.git("add", ".")
        self.git("update-index", "--add", "--cacheinfo", f"160000,{object_id},nested")
        self.assert_fails("nested Git checkout", "--staged")

    def test_history_keeps_rejecting_a_removed_secret(self):
        self.write("settings.txt", "ghp_" + "A" * 36)
        self.git("add", ".")
        first = self.snapshot()
        self.write("settings.txt", "removed")
        self.git("add", ".")
        second = self.snapshot(first)
        self.assert_passes()
        self.assert_fails("possible GitHub token", "--history", second)

    def test_history_checks_paths_even_when_content_is_shared(self):
        self.write("safe.txt", "same bytes")
        self.git("add", ".")
        first = self.snapshot()
        self.write("fixtures/private/same.txt", "same bytes")
        self.git("add", ".")
        second = self.snapshot(first)
        self.assert_fails("historical private fixture directory", "--history", second)

    def test_reference_corpus_and_research_cannot_be_forced_into_source_tree(self):
        for name in [
            "crates/example/tests/fixtures/reference.json",
            "scripts/research/observe.py",
            "docs/research/report.md",
            "crates/example/src/private_tests.rs",
        ]:
            with self.subTest(name=name):
                path = self.write(name, "small")
                self.assert_fails(name)
                path.unlink()

    def test_reviewed_json_requires_exact_path_size_and_digest(self):
        content = b'{"synthetic":[1,2,3]}'
        self.write("data/example.json", content)
        self.assert_fails("exact-byte provenance review")
        self.policy["reviewed_data"] = [{
            "path": "data/example.json", "bytes": len(content),
            "sha256": hashlib.sha256(content).hexdigest(),
            "provenance": "Project-authored synthetic test values",
        }]
        self.save_policy()
        self.assert_passes()
        self.write("data/example.json", b'{"synthetic":[3,2,1]}')
        self.assert_fails("exact-byte provenance review")
        self.write("data/example.json", content)
        self.write("data/renamed.json", content)
        self.assert_fails("data/renamed.json")

    def test_known_private_payloads_are_rejected_in_source_encodings(self):
        payload = b"Synthetic instruction witness for this test only"
        self.policy["blocked_payloads"] = [{
            "bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest(),
        }]
        self.save_policy()
        for content in [
            payload,
            'const RAW: &str = "' + payload.hex() + '";',
            "const RAW: &[u8] = &[" + ", ".join(f"0x{value:02x}" for value in payload) + "];",
            "const RAW: &[u8] = &[" + ", ".join(str(value) for value in payload) + "];",
            "const TEXT: &str = " + json.dumps(payload.decode()) + ";",
        ]:
            with self.subTest(content_type=type(content).__name__):
                self.write("source.rs", content)
                result = self.assert_fails("known private game payload")
                self.assertNotIn(payload.decode(), result.stdout + result.stderr)

    def test_rust_raw_strings_preserve_quotes_hashes_and_newlines(self):
        payload = b'Synthetic "quoted" witness # with\na second line'
        self.policy["blocked_payloads"] = [{
            "bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest(),
        }]
        self.save_policy()
        for hashes in ["#", "##", "###"]:
            with self.subTest(hashes=hashes):
                self.write("source.rs", 'const TEXT: &str = r' + hashes + '"'
                           + payload.decode() + '"' + hashes + ';')
                result = self.assert_fails("known private game payload")
                self.assertNotIn(payload.decode(), result.stdout + result.stderr)

    def test_binary_content_cannot_hide_under_a_source_extension(self):
        self.write("source.rs", b"synthetic\0binary")
        self.assert_fails("binary content")

    def test_long_encoded_source_payload_requires_review(self):
        self.write("source.rs", 'const RAW: &str = "' + "ab" * 200 + '";')
        self.assert_fails("long encoded payload")

    def test_history_policy_cannot_be_relaxed_by_uncommitted_changes(self):
        payload = b"A synthetic private payload for the history regression"
        self.policy["blocked_payloads"] = [{
            "bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest(),
        }]
        self.save_policy()
        self.write("source.rs", json.dumps(payload.decode()))
        self.git("add", ".")
        first = self.snapshot()
        self.write("source.rs", "removed")
        self.git("add", ".")
        second = self.snapshot(first)
        self.policy["blocked_payloads"] = []
        self.save_policy()
        self.assert_passes()
        self.assert_fails("known private game payload", "--history", second)


class PublicBuildManifestTests(unittest.TestCase):
    def test_linux_window_backend_is_explicit_and_target_scoped(self):
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--offline", "--locked", "--no-deps", "--format-version", "1"],
            cwd=SCRIPT.parent.parent, text=True, timeout=30,
        ))
        app = next(package for package in metadata["packages"] if package["name"] == "sims-app")
        iced_dependencies = [dependency for dependency in app["dependencies"] if dependency["name"] == "iced"]
        linux = next(dependency for dependency in iced_dependencies
                     if dependency["target"] == 'cfg(target_os = "linux")')
        self.assertEqual(linux["features"], ["wayland"])
        for dependency in iced_dependencies:
            self.assertFalse(dependency["uses_default_features"])
            if dependency["target"] is None:
                self.assertFalse({"x11", "wayland"}.intersection(dependency["features"]))


if __name__ == "__main__":
    unittest.main()
