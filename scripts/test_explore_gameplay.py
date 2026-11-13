import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import explore_gameplay


DRIVER = Path(__file__).resolve().with_name("explore_gameplay.py")
BATCH = DRIVER.with_name("explore_gameplay_batch.py")
STUB_RUNNER = """#!/usr/bin/env python3
import argparse
import json
from pathlib import Path
import sys
import time

parser = argparse.ArgumentParser()
for name in ("original", "root", "load", "exploration-plan", "exploration-output"):
    parser.add_argument("--" + name, required=True)
args = parser.parse_args()
output = Path(args.exploration_output)
assert not output.exists(), "runner requires a fresh nonexistent output"
plan_bytes = Path(args.exploration_plan).read_bytes()
plan = json.loads(plan_bytes)
for name, content in plan.get("original_changes", {}).items():
    path = Path(name)
    if content is None:
        path.unlink()
    else:
        path.write_text(content)
output.mkdir()
(output / "consumed-load.json").write_bytes(Path(args.load).read_bytes())
(output / "consumed-plan.json").write_bytes(plan_bytes)
(output / "source.json").write_text(json.dumps(args.original))
(output / "run.json").write_text(json.dumps({
    "outcome": "passed", "initial": {"tick": 0}, "steps": [{"after": {"tick": 2}}],
    "tick_comparisons": 2, "cold_restores": 0, "failure": None, "stop": None
}))
paths = {"binary": Path(sys.argv[0]), "load": Path(args.load), "plan": Path(args.exploration_plan)}
for name, action in plan.get("staged_changes", {}).items():
    if action == "delete":
        paths[name].unlink()
    else:
        paths[name].write_bytes(b"changed staged bytes")
print("local Python child ran", flush=True)
time.sleep(plan.get("sleep", 0))
sys.exit(plan.get("exit_code", 0))
"""


class ExploreGameplayTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="gameplay-pinning-")
        self.addCleanup(temporary.cleanup)
        self.workspace = Path(temporary.name, "workspace ' with spaces")
        self.workspace.mkdir()
        self.binary = self.workspace / "local-python-runner"
        self.binary.write_text(STUB_RUNNER)
        self.binary.chmod(0o755)
        self.load = self.workspace / "original-load.json"
        self.load.write_text('{"session": {"source": "fixture-household", "value": 7}}\n')
        self.plan = self.workspace / "original-plan.json"
        self.plan.write_text('{"steps": []}\n')
        self.root = self.workspace / "mutable root assets"
        self.root.mkdir()
        self.output = self.workspace / "campaign"
        self.originals = {"binary": self.binary, "load": self.load, "plan": self.plan}
        self.original_bytes = {name: path.read_bytes() for name, path in self.originals.items()}
        self.consumed = {}
        self.environment = {**os.environ, "PYTHONDONTWRITEBYTECODE": "1"}

    def arguments(self, output=None, timeout=10):
        return [str(DRIVER), "--binary", str(self.binary), "--root", str(self.root),
                "--load", str(self.load), "--plan", str(self.plan),
                "--output", str(output or self.output), "--timeout", str(timeout)]

    def sidecars(self, output=None):
        output = output or self.output
        return {"binary": output.with_name(output.name + "-runner"),
                "load": output.with_name(output.name + "-pinned-load.json"),
                "plan": output.with_name(output.name + "-pinned-plan.json"),
                "log": output.with_name(output.name + ".log"),
                "manifest": output.with_name(output.name + "-driver-manifest.json")}

    def manifest(self, output=None):
        return json.loads(self.sidecars(output)["manifest"].read_text())

    def launch(self, command, **options):
        self.assertEqual(command[0], str(self.sidecars()["binary"]))
        self.assertFalse(self.output.exists())
        self.assertEqual(options["stderr"], subprocess.STDOUT)
        self.assertTrue(options["start_new_session"])
        self.consumed = {"binary": Path(command[0]).read_bytes(),
                         "load": Path(command[command.index("--load") + 1]).read_bytes(),
                         "plan": Path(command[command.index("--exploration-plan") + 1]).read_bytes()}
        self.output.mkdir()
        (self.output / "run.json").write_text('{"outcome": "passed"}\n')
        process = mock.Mock(pid=12345, returncode=0)
        process.wait.return_value = 0
        return process

    def run_driver(self, launch=None, output=None, timeout=10):
        with mock.patch.object(sys, "argv", self.arguments(output, timeout)), \
                mock.patch.object(explore_gameplay.subprocess, "Popen", side_effect=launch or self.launch), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return explore_gameplay.main()

    def cli(self, timeout=10):
        return subprocess.run([sys.executable, "-B", *self.arguments(timeout=timeout)],
                              capture_output=True, text=True, env=self.environment, timeout=15)

    def assert_identities(self, manifest, expected=None):
        expected = expected or self.original_bytes
        for name, field in (("binary", "binary_sha256"), ("load", "input_sha256"), ("plan", "plan_sha256")):
            with self.subTest(identity=name):
                identity = hashlib.sha256(expected[name]).hexdigest()
                self.assertEqual(manifest[field], identity)
                self.assertEqual(manifest["input_integrity"][name]["pre_run_sha256"], identity)

    def workspace_inventory(self):
        return {str(path.relative_to(self.workspace)): (path.lstat().st_ino, path.read_bytes())
                for path in self.workspace.rglob("*") if path.is_file() and not path.is_symlink()}

    def test_normal_manifest_preserves_fields_and_records_staged_identities(self):
        self.assertEqual(self.run_driver(), 0)
        manifest = self.manifest()
        self.assert_identities(manifest)
        self.assertEqual(self.consumed, self.original_bytes)
        self.assertEqual(manifest["schema"], 1)
        self.assertEqual(manifest["exit_code"], 0)
        self.assertEqual(manifest["child_exit_code"], 0)
        self.assertFalse(manifest["timed_out"])
        self.assertIsNone(manifest["launch_error"])
        self.assertFalse(manifest["original_engine_executed"])
        self.assertFalse(manifest["root_assets_pinned"])
        self.assertTrue(manifest["evidence_integrity"])
        self.assertEqual(manifest["evidence_errors"], [])
        self.assertEqual(manifest["input_identity"], "pre_run_staged_bytes")
        self.assertEqual(manifest["command"][manifest["command"].index("--original") + 1], "fixture-household")
        self.assertEqual(manifest["log_sha256"], hashlib.sha256(b"").hexdigest())
        self.assertEqual(manifest["artifacts"], {"run.json": explore_gameplay.digest(self.output / "run.json")})
        self.assertGreaterEqual(manifest["seconds"], 0)
        self.assertEqual(json.loads((self.output / "driver-manifest.json").read_text()), manifest)
        for name, original in self.originals.items():
            self.assertEqual(original.read_bytes(), self.original_bytes[name])
            self.assertNotEqual(original.stat().st_ino, self.sidecars()[name].stat().st_ino)
            self.assertEqual(manifest["source_paths"][name], str(original.absolute()))
            self.assertTrue(manifest["input_integrity"][name]["verified"])
        self.assertEqual(self.sidecars()["binary"].stat().st_mode & 0o777, 0o755)

    def test_source_selection_uses_staged_load_even_if_original_changes_before_launch(self):
        copy = explore_gameplay.shutil.copyfileobj

        def copy_then_mutate(source, target):
            copy(source, target)
            if source.name == str(self.load):
                self.load.write_text('{"session": {"source": "wrong-household"}}')

        with mock.patch.object(explore_gameplay.shutil, "copyfileobj", side_effect=copy_then_mutate):
            self.assertEqual(self.run_driver(), 0)
        manifest = self.manifest()
        self.assertEqual(manifest["command"][2], "fixture-household")
        self.assertEqual(self.consumed["load"], self.original_bytes["load"])
        self.assert_identities(manifest)

    def test_top_level_source_save_still_works(self):
        self.load.write_text('{"source": "top-level-household"}')
        self.assertEqual(self.run_driver(), 0)
        self.assertEqual(self.manifest()["command"][2], "top-level-household")

    def test_original_mutations_do_not_change_consumed_bytes_or_identities(self):
        def mutate_then_launch(command, **options):
            for path in self.originals.values():
                path.write_bytes(b"original changed after staging")
            return self.launch(command, **options)

        self.assertEqual(self.run_driver(mutate_then_launch), 0)
        self.assertEqual(self.consumed, self.original_bytes)
        self.assert_identities(self.manifest())
        self.assertTrue(self.manifest()["evidence_integrity"])

    def test_original_deletions_do_not_prevent_manifest(self):
        def delete_then_launch(command, **options):
            for path in self.originals.values():
                path.unlink()
            return self.launch(command, **options)

        self.assertEqual(self.run_driver(delete_then_launch), 0)
        self.assertEqual(self.consumed, self.original_bytes)
        self.assert_identities(self.manifest())
        self.assertTrue(self.manifest()["evidence_integrity"])

    def test_staged_mutations_fail_closed_and_preserve_child_success(self):
        for name in self.originals:
            with self.subTest(staged=name), tempfile.TemporaryDirectory(dir=self.workspace) as campaign:
                self.output = Path(campaign) / "run"

                def mutate(command, **options):
                    process = self.launch(command, **options)
                    self.sidecars()[name].write_bytes(b"corrupt staged bytes")
                    return process

                self.assertEqual(self.run_driver(mutate), 1)
                manifest = self.manifest()
                self.assert_identities(manifest)
                self.assertEqual(manifest["child_exit_code"], 0)
                self.assertEqual(manifest["exit_code"], 1)
                self.assertFalse(manifest["evidence_integrity"])
                self.assertFalse(manifest["input_integrity"][name]["verified"])
                self.assertEqual(manifest["input_integrity"][name]["post_run_sha256"],
                                 hashlib.sha256(b"corrupt staged bytes").hexdigest())
                self.assertTrue((self.output / "run.json").is_file())

    def test_staged_deletions_fail_closed_and_preserve_child_outcome(self):
        for name in self.originals:
            with self.subTest(staged=name), tempfile.TemporaryDirectory(dir=self.workspace) as campaign:
                self.output = Path(campaign) / "run"

                def delete(command, **options):
                    process = self.launch(command, **options)
                    self.sidecars()[name].unlink()
                    return process

                self.assertEqual(self.run_driver(delete), 1)
                manifest = self.manifest()
                self.assert_identities(manifest)
                self.assertEqual(manifest["child_exit_code"], 0)
                self.assertFalse(manifest["evidence_integrity"])
                self.assertIsNone(manifest["input_integrity"][name]["post_run_sha256"])
                self.assertTrue(manifest["input_integrity"][name]["error"])
                self.assertTrue((self.output / "driver-manifest.json").is_file())

    def test_staged_symlink_replacements_fail_even_when_bytes_match(self):
        for name in self.originals:
            with self.subTest(staged=name), tempfile.TemporaryDirectory(dir=self.workspace) as campaign:
                self.output = Path(campaign) / "run"

                def replace(command, **options):
                    process = self.launch(command, **options)
                    self.sidecars()[name].unlink()
                    self.sidecars()[name].symlink_to(self.originals[name])
                    return process

                self.assertEqual(self.run_driver(replace), 1)
                self.assertFalse(self.manifest()["input_integrity"][name]["verified"])

    def test_nonzero_child_exit_code_is_preserved(self):
        def fail(command, **options):
            process = self.launch(command, **options)
            process.returncode = 23
            process.wait.return_value = 23
            return process

        self.assertEqual(self.run_driver(fail), 23)
        self.assertEqual(self.manifest()["child_exit_code"], 23)
        self.assertEqual(self.manifest()["exit_code"], 23)
        self.assertTrue(self.manifest()["evidence_integrity"])

    def test_integrity_failure_does_not_mask_nonzero_child(self):
        def corrupt_and_fail(command, **options):
            process = self.launch(command, **options)
            self.sidecars()["plan"].unlink()
            process.returncode = 29
            process.wait.return_value = 29
            return process

        self.assertEqual(self.run_driver(corrupt_and_fail), 29)
        self.assertEqual(self.manifest()["child_exit_code"], 29)
        self.assertFalse(self.manifest()["evidence_integrity"])

    def test_launch_failure_retains_manifest_and_staged_inputs(self):
        self.assertEqual(self.run_driver(mock.Mock(side_effect=OSError("cannot launch stub"))), 1)
        manifest = self.manifest()
        self.assertEqual(manifest["exit_code"], 1)
        self.assertIsNone(manifest["child_exit_code"])
        self.assertEqual(manifest["launch_error"], "cannot launch stub")
        self.assertFalse(manifest["timed_out"])
        self.assertTrue(manifest["evidence_integrity"])
        self.assertFalse(self.output.exists())
        self.assert_identities(manifest)

    def test_nonzero_child_without_output_uses_sidecar_manifest(self):
        process = mock.Mock(returncode=17)
        process.wait.return_value = 17
        self.assertEqual(self.run_driver(mock.Mock(return_value=process)), 17)
        self.assertEqual(self.manifest()["child_exit_code"], 17)
        self.assertFalse(self.output.exists())

    def test_timeout_retains_actual_child_exit_and_integrity_failure(self):
        def timeout(command, **options):
            process = self.launch(command, **options)
            self.sidecars()["load"].unlink()
            process.returncode = -signal.SIGTERM
            process.wait.side_effect = [subprocess.TimeoutExpired(command, 10), -signal.SIGTERM]
            return process

        with mock.patch.object(explore_gameplay.os, "killpg") as kill:
            self.assertEqual(self.run_driver(timeout), 124)
        kill.assert_called_once_with(12345, signal.SIGTERM)
        manifest = self.manifest()
        self.assertEqual(manifest["exit_code"], 124)
        self.assertEqual(manifest["child_exit_code"], -signal.SIGTERM)
        self.assertTrue(manifest["timed_out"])
        self.assertFalse(manifest["evidence_integrity"])

    def test_timeout_escalates_to_kill(self):
        def timeout(command, **options):
            process = self.launch(command, **options)
            process.returncode = -signal.SIGKILL
            process.wait.side_effect = [subprocess.TimeoutExpired(command, 10),
                                        subprocess.TimeoutExpired(command, 5), -signal.SIGKILL]
            return process

        with mock.patch.object(explore_gameplay.os, "killpg") as kill:
            self.assertEqual(self.run_driver(timeout), 124)
        self.assertEqual(kill.call_args_list, [mock.call(12345, signal.SIGTERM), mock.call(12345, signal.SIGKILL)])
        self.assertEqual(self.manifest()["child_exit_code"], -signal.SIGKILL)
        self.assertTrue(self.manifest()["evidence_integrity"])

    def test_timeout_process_exit_race_still_writes_manifest(self):
        process = mock.Mock(pid=12345, returncode=0)
        process.wait.side_effect = [subprocess.TimeoutExpired("stub", 10), 0]
        with mock.patch.object(explore_gameplay.os, "killpg", side_effect=ProcessLookupError):
            self.assertEqual(self.run_driver(mock.Mock(return_value=process)), 124)
        self.assertEqual(self.manifest()["child_exit_code"], 0)
        self.assertTrue(self.manifest()["timed_out"])

    def test_existing_evidence_paths_are_preflighted_without_changes(self):
        for name in ("output", *self.sidecars()):
            with self.subTest(path=name):
                output = self.workspace / ("collision-" + name)
                path = output if name == "output" else self.sidecars(output)[name]
                path.write_bytes(b"prior evidence")
                before = self.workspace_inventory()
                with self.assertRaises(SystemExit) as error:
                    self.run_driver(mock.Mock(side_effect=AssertionError("must not launch")), output=output)
                self.assertEqual(error.exception.code, 2)
                self.assertEqual(self.workspace_inventory(), before)

    def test_dangling_evidence_symlinks_are_not_overwritten(self):
        for name in ("output", *self.sidecars()):
            with self.subTest(path=name):
                output = self.workspace / ("symlink-" + name)
                path = output if name == "output" else self.sidecars(output)[name]
                path.symlink_to(self.workspace / "missing-target")
                before = self.workspace_inventory()
                with self.assertRaises(SystemExit) as error:
                    self.run_driver(mock.Mock(side_effect=AssertionError("must not launch")), output=output)
                self.assertEqual(error.exception.code, 2)
                self.assertTrue(path.is_symlink())
                self.assertEqual(os.readlink(path), str(self.workspace / "missing-target"))
                self.assertEqual(self.workspace_inventory(), before)

    def test_existing_output_directory_and_manifest_are_preserved(self):
        self.output.mkdir()
        prior = self.output / "driver-manifest.json"
        prior.write_bytes(b"prior manifest")
        before = self.workspace_inventory()
        with self.assertRaises(SystemExit):
            self.run_driver()
        self.assertEqual(self.workspace_inventory(), before)

    def test_child_manifest_collision_uses_reserved_fallback(self):
        def collide(command, **options):
            process = self.launch(command, **options)
            (self.output / "driver-manifest.json").write_bytes(b"child evidence")
            return process

        self.assertEqual(self.run_driver(collide), 1)
        self.assertEqual((self.output / "driver-manifest.json").read_bytes(), b"child evidence")
        self.assertEqual(self.manifest()["child_exit_code"], 0)
        self.assertFalse(self.manifest()["evidence_integrity"])
        self.assertTrue(any(error.startswith("manifest:") for error in self.manifest()["evidence_errors"]))

    def test_log_deletion_does_not_prevent_manifest(self):
        def delete_log(command, **options):
            process = self.launch(command, **options)
            self.sidecars()["log"].unlink()
            return process

        self.assertEqual(self.run_driver(delete_log), 1)
        self.assertIsNone(self.manifest()["log_sha256"])
        self.assertEqual(self.manifest()["child_exit_code"], 0)
        self.assertFalse(self.manifest()["evidence_integrity"])

    def test_invalid_timeouts_do_not_create_evidence(self):
        before = self.workspace_inventory()
        for timeout in (0, -1, float("nan"), float("inf")):
            with self.subTest(timeout=timeout), self.assertRaises(SystemExit) as error:
                self.run_driver(timeout=timeout)
            self.assertEqual(error.exception.code, 2)
            self.assertEqual(self.workspace_inventory(), before)

    def test_local_python_cli_consumes_staged_bytes_after_original_deletions(self):
        self.plan.write_text(json.dumps({"original_changes": {str(path): None for path in self.originals.values()}}))
        expected = {name: path.read_bytes() for name, path in self.originals.items()}
        result = self.cli()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual((self.output / "consumed-load.json").read_bytes(), expected["load"])
        self.assertEqual((self.output / "consumed-plan.json").read_bytes(), expected["plan"])
        self.assertEqual(json.loads((self.output / "source.json").read_text()), "fixture-household")
        self.assert_identities(self.manifest(), expected)
        self.assertTrue(self.manifest()["evidence_integrity"])
        self.assertTrue(all(not path.exists() for path in self.originals.values()))

    def test_local_python_cli_records_staged_deletion_and_nonzero_outcome(self):
        self.plan.write_text(json.dumps({"staged_changes": {"plan": "delete"}, "exit_code": 31}))
        result = self.cli()
        self.assertEqual(result.returncode, 31, result.stdout + result.stderr)
        self.assertEqual(self.manifest()["child_exit_code"], 31)
        self.assertFalse(self.manifest()["evidence_integrity"])
        self.assertIsNone(self.manifest()["input_integrity"]["plan"]["post_run_sha256"])

    def test_local_python_cli_timeout_retains_outcome_and_manifest(self):
        self.plan.write_text(json.dumps({"sleep": 10}))
        result = self.cli(timeout=0.5)
        self.assertEqual(result.returncode, 124, result.stdout + result.stderr)
        self.assertTrue(self.manifest()["timed_out"])
        self.assertEqual(self.manifest()["child_exit_code"], -signal.SIGTERM)
        self.assertTrue(self.manifest()["evidence_integrity"])

    def test_local_python_cli_launch_failure_retains_manifest(self):
        self.binary.write_bytes(b"not an executable image")
        result = self.cli()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIsNone(self.manifest()["child_exit_code"])
        self.assertTrue(self.manifest()["launch_error"])
        self.assertTrue(self.manifest()["evidence_integrity"])

    def test_batch_cli_keeps_job_plan_paths_and_normal_manifests_compatible(self):
        jobs = [{"name": name, "load": str(self.load), "plan": {"steps": []}}
                for name in ("case-one", "case-two")]
        manifest_path = self.workspace / "jobs.json"
        manifest_path.write_text(json.dumps({"jobs": jobs}))
        batch_output = self.workspace / "batch"
        result = subprocess.run([sys.executable, "-B", str(BATCH), "--binary", str(self.binary),
                                 "--root", str(self.root), "--manifest", str(manifest_path),
                                 "--output", str(batch_output), "--workers", "2", "--timeout", "10"],
                                capture_output=True, text=True, env=self.environment, timeout=15)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        summary = json.loads((batch_output / "summary.json").read_text())
        self.assertEqual((summary["passed"], summary["total"]), (2, 2))
        for job in jobs:
            output = batch_output / job["name"]
            manifest = self.manifest(output)
            self.assertEqual(json.loads((output / "driver-manifest.json").read_text()), manifest)
            self.assertTrue(manifest["evidence_integrity"])
            original_plan = batch_output / (job["name"] + "-plan.json")
            self.assertEqual(manifest["plan_sha256"], explore_gameplay.digest(original_plan))
            self.assertNotEqual(manifest["command"][manifest["command"].index("--exploration-plan") + 1],
                                str(original_plan))


if __name__ == "__main__":
    unittest.main()
