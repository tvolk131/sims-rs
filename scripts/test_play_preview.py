import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import play_preview


class PreviewTests(unittest.TestCase):
    def prepare(self, directory):
        root = Path(directory).resolve()
        bundle = root / 'cache/performance-preview'
        bundle.mkdir(parents=True)
        binary = bundle / 'sims-room'
        binary.write_text('synthetic executable stand-in, never executed\n')
        binary.chmod(0o700)
        supported = bundle / 'supported-save.json'
        supported.write_text(json.dumps({'session': {'source': 'synthetic-house.iff'}}))
        game_root = root / 'separately-installed'
        game_root.mkdir()
        manifest = {'schema': 1, 'qualification': {'passed': True},
                    'binary': {'path': 'cache/performance-preview/sims-room', 'sha256': play_preview.digest(binary)},
                    'supported_save': {'path': 'cache/performance-preview/supported-save.json',
                                       'sha256': play_preview.digest(supported)},
                    'game_root': str(game_root)}
        filename = bundle / 'manifest.json'
        filename.write_text(json.dumps(manifest))
        return root, filename, manifest, supported

    def test_launch_uses_unique_private_copy_and_distinct_safe_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root, filename, manifest, supported = self.prepare(directory)
            original = supported.read_bytes()
            with mock.patch.object(play_preview, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()), \
                    mock.patch.object(play_preview, 'verify_qualification', return_value={'passed': True}):
                for attempt in range(2):
                    with mock.patch.object(play_preview.subprocess, 'run', return_value=SimpleNamespace(returncode=0)) as launch:
                        self.assertEqual(play_preview.launch(filename), 0)
                    command = launch.call_args.args[0]
                    executable = Path(command[0])
                    load = Path(command[command.index('--load')+1])
                    output = Path(command[command.index('--save')+1])
                    self.assertNotEqual(load, supported)
                    self.assertNotEqual(load, output)
                    self.assertNotEqual(executable, root / manifest['binary']['path'])
                    self.assertEqual(executable.parent, load.parent)
                    self.assertEqual(play_preview.digest(executable), manifest['binary']['sha256'])
                    self.assertEqual(executable.stat().st_mode & 0o777, 0o500)
                    self.assertEqual(load.read_bytes(), original)
                    self.assertTrue(load.is_relative_to(root / 'cache/preview-runs'))
                    self.assertEqual(load.stat().st_mode & 0o222, 0)
                    self.assertFalse(output.exists())
                    self.assertIn('--paused', command)
                    self.assertNotIn('--window-session-output', command)
                    receipt = json.loads((load.parent / 'launch.json').read_text())
                    self.assertTrue(receipt['supported_input_unchanged'])
                    self.assertTrue(receipt['starting_copy_unchanged'])
                    self.assertTrue(receipt['executable_copy_unchanged'])
                self.assertEqual(len(list((root / 'cache/preview-runs').iterdir())), 2)
            self.assertEqual(supported.read_bytes(), original)

    def test_passed_flag_without_evidence_never_launches(self):
        with tempfile.TemporaryDirectory() as directory:
            root, filename, manifest, supported = self.prepare(directory)
            with mock.patch.object(play_preview, 'REPOSITORY', root), \
                    mock.patch.object(play_preview.subprocess, 'run') as launch, \
                    contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaisesRegex(ValueError, 'three consecutive evidence'):
                    play_preview.launch(filename)
                launch.assert_not_called()
                self.assertFalse((root / 'cache/preview-runs').exists())

    def test_inputs_changed_during_qualification_never_launch(self):
        for name in ('binary', 'supported_save', 'manifest'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root, filename, manifest, supported = self.prepare(directory)
                selected = filename if name == 'manifest' else root / manifest[name]['path']
                def mutate(*arguments):
                    selected.write_text('changed after the initial identity check')
                    return {'passed': True}
                with mock.patch.object(play_preview, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()), \
                        mock.patch.object(play_preview, 'verify_qualification', side_effect=mutate), \
                        mock.patch.object(play_preview.subprocess, 'run') as launch, \
                        self.assertRaisesRegex(ValueError, 'changed'):
                    play_preview.launch(filename)
                launch.assert_not_called()

    def test_rebuilding_the_bundle_cannot_replace_the_verified_executable_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            root, filename, manifest, supported = self.prepare(directory)
            binary = root / manifest['binary']['path']
            original = binary.read_bytes()
            def execute(command, **arguments):
                binary.write_text('replacement bundle executable, never executed')
                self.assertNotEqual(Path(command[0]), binary)
                self.assertEqual(Path(command[0]).read_bytes(), original)
                return SimpleNamespace(returncode=0)
            with mock.patch.object(play_preview, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()), \
                    mock.patch.object(play_preview, 'verify_qualification', return_value={'passed': True}), \
                    mock.patch.object(play_preview.subprocess, 'run', side_effect=execute):
                self.assertEqual(play_preview.launch(filename), 0)
            receipt = json.loads(next((root / 'cache/preview-runs').glob('*/launch.json')).read_text())
            self.assertTrue(receipt['executable_copy_unchanged'])
            self.assertFalse(receipt['bundle_binary_unchanged'])

    def test_original_house_identity_comes_from_the_verified_private_save(self):
        with tempfile.TemporaryDirectory() as directory:
            root, filename, manifest, supported = self.prepare(directory)
            read_bytes = Path.read_bytes
            def private_only(source):
                if source == supported:
                    raise AssertionError('Parsed a mutable bundle save instead of its verified private copy')
                return read_bytes(source)
            with mock.patch.object(play_preview, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()), \
                    mock.patch.object(play_preview, 'verify_qualification', return_value={'passed': True}), \
                    mock.patch.object(Path, 'read_bytes', private_only), \
                    mock.patch.object(play_preview.subprocess, 'run', return_value=SimpleNamespace(returncode=0)) as launch:
                self.assertEqual(play_preview.launch(filename), 0)
                command = launch.call_args.args[0]
                self.assertEqual(command[command.index('--original')+1], 'synthetic-house.iff')

    def test_missing_private_input_is_recorded_as_failure_after_child_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            root, filename, manifest, supported = self.prepare(directory)
            def removed(command, **arguments):
                Path(command[command.index('--load')+1]).unlink()
                return SimpleNamespace(returncode=0)
            with mock.patch.object(play_preview, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()), \
                    mock.patch.object(play_preview, 'verify_qualification', return_value={'passed': True}), \
                    mock.patch.object(play_preview.subprocess, 'run', side_effect=removed), \
                    self.assertRaisesRegex(ValueError, 'read-only preview input changed'):
                play_preview.launch(filename)
            receipt = json.loads(next((root / 'cache/preview-runs').glob('*/launch.json')).read_text())
            self.assertEqual(receipt['exit_code'], 0)
            self.assertFalse(receipt['starting_copy_unchanged'])
            self.assertTrue(receipt['supported_input_unchanged'])

    def test_unqualified_manifest_never_launches(self):
        with tempfile.TemporaryDirectory() as directory:
            root, filename, manifest, supported = self.prepare(directory)
            manifest['qualification']['passed'] = False
            filename.write_text(json.dumps(manifest))
            with mock.patch.object(play_preview, 'REPOSITORY', root), mock.patch.object(play_preview.subprocess, 'run') as launch:
                with self.assertRaisesRegex(ValueError, 'qualification'):
                    play_preview.launch(filename)
                launch.assert_not_called()
                self.assertFalse((root / 'cache/preview-runs').exists())

    def test_changed_binary_or_supported_save_never_launches(self):
        for name in ('binary', 'supported_save'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root, filename, manifest, supported = self.prepare(directory)
                (root / manifest[name]['path']).write_text('changed')
                with mock.patch.object(play_preview, 'REPOSITORY', root), mock.patch.object(play_preview.subprocess, 'run') as launch:
                    with self.assertRaisesRegex(ValueError, 'changed'):
                        play_preview.launch(filename)
                    launch.assert_not_called()

    def test_private_paths_reject_escape_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root, filename, manifest, supported = self.prepare(directory)
            outside = root / 'public'
            outside.mkdir()
            (root / 'cache/linked').symlink_to(outside, target_is_directory=True)
            with mock.patch.object(play_preview, 'REPOSITORY', root):
                for candidate in ('cache/../public', 'public', 'cache/linked/run'):
                    with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                        play_preview.private_path(candidate, exists=False)
            cache = root / 'cache'
            relocated = root / 'relocated-cache'
            cache.rename(relocated)
            cache.symlink_to(relocated, target_is_directory=True)
            with mock.patch.object(play_preview, 'REPOSITORY', root), self.assertRaisesRegex(ValueError, 'real, private'):
                play_preview.private_path(filename)

    def test_child_failure_is_recorded_not_reported_as_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root, filename, manifest, supported = self.prepare(directory)
            with mock.patch.object(play_preview, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()), \
                    mock.patch.object(play_preview, 'verify_qualification', return_value={'passed': True}), \
                    mock.patch.object(play_preview.subprocess, 'run', return_value=SimpleNamespace(returncode=7)):
                self.assertEqual(play_preview.launch(filename), 7)
            receipt = next((root / 'cache/preview-runs').glob('*/launch.json'))
            self.assertEqual(json.loads(receipt.read_text())['exit_code'], 7)


if __name__ == '__main__':
    unittest.main()
