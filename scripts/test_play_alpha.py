import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

import play_alpha


def saved(source, tick=42):
    return {'session': {'version': 2, 'source': source, 'synthetic_tick': tick},
            'dialog_client': {'synthetic': True}}


class AlphaLauncherTests(unittest.TestCase):
    def prepare(self, directory):
        root = Path(directory).resolve()
        binary = root / 'sims-room'
        binary.write_bytes(b'synthetic executable; never executed')
        binary.chmod(0o700)
        installation = root / 'installed'
        installation.mkdir()
        return root, binary, installation

    def child(self, source, tick=42, code=0):
        def run(command, **kwargs):
            self.assertIn('--paused', command)
            output = Path(command[command.index('--save') + 1])
            output.write_text(json.dumps(saved(source, tick)))
            return SimpleNamespace(returncode=code)
        return run

    def test_fresh_save_exit_and_continue_use_the_saved_progress_without_overwriting_it(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            source = play_alpha.HOUSEHOLDS['goth']
            with mock.patch.object(play_alpha, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()):
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(source, 123)) as child:
                    self.assertEqual(play_alpha.launch(binary, installation, household='goth'), 0)
                command = child.call_args.args[0]
                self.assertIn('--buy-preview', command)
                self.assertNotIn('--load', command)
                self.assertNotIn('--window-session-output', command)
                self.assertNotIn('--window-session-present-library', command)
                first_save = Path(command[command.index('--save') + 1])
                original = first_save.read_bytes()
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(source, 456)) as child:
                    self.assertEqual(play_alpha.launch(binary, installation, resume=True), 0)
                command = child.call_args.args[0]
                restored = Path(command[command.index('--load') + 1])
                output = Path(command[command.index('--save') + 1])
                self.assertEqual(restored.read_bytes(), original)
                self.assertEqual(restored.stat().st_mode & 0o222, 0)
                self.assertNotEqual(output, first_save)
                self.assertEqual(first_save.read_bytes(), original)
                self.assertNotIn('--buy-preview', command)
                self.assertNotIn('--neighborhood-policy', command)
                pointer = json.loads((root / 'cache/alpha-sessions/last-session.json').read_text())
                self.assertEqual(Path(pointer['save']), output)
                self.assertEqual(pointer['sha256'], play_alpha.digest(output))

    def test_explicit_load_supports_another_household_and_always_saves_to_a_new_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            source = play_alpha.HOUSEHOLDS['mashuga']
            selected = root / 'my-save.json'
            selected.write_text(json.dumps(saved(source, 77)))
            original = selected.read_bytes()
            with mock.patch.object(play_alpha, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()), \
                    mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(source, 88)) as child:
                self.assertEqual(play_alpha.launch(binary, installation, load=selected), 0)
            command = child.call_args.args[0]
            self.assertEqual(command[command.index('--original') + 1], source)
            self.assertNotEqual(Path(command[command.index('--save') + 1]), selected)
            self.assertEqual(selected.read_bytes(), original)

    def test_missing_continue_and_invalid_saves_never_start_the_application(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            with mock.patch.object(play_alpha, 'REPOSITORY', root), mock.patch.object(play_alpha.subprocess, 'run') as child:
                with self.assertRaisesRegex(ValueError, 'No saved session'):
                    play_alpha.launch(binary, installation, resume=True)
                invalid = root / 'invalid.json'
                for value in ('broken JSON', json.dumps({'session': {'version': 1}}),
                              json.dumps({'session': {'version': 2, 'source': 'house'}})):
                    invalid.write_text(value)
                    with self.assertRaises(ValueError):
                        play_alpha.launch(binary, installation, load=invalid)
                    self.assertEqual(invalid.read_text(), value)
                child.assert_not_called()

    def test_failed_process_or_invalid_output_preserves_the_previous_continue_save(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            source = play_alpha.HOUSEHOLDS['newbie']
            with mock.patch.object(play_alpha, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()), \
                    contextlib.redirect_stderr(io.StringIO()):
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(source)):
                    play_alpha.launch(binary, installation, household='newbie')
                pointer = root / 'cache/alpha-sessions/last-session.json'
                previous = pointer.read_bytes()
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(source, code=7)):
                    self.assertEqual(play_alpha.launch(binary, installation, resume=True), 7)
                self.assertEqual(pointer.read_bytes(), previous)
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child('different-house')):
                    play_alpha.launch(binary, installation, resume=True)
                self.assertEqual(pointer.read_bytes(), previous)
                def invalid(command, **kwargs):
                    Path(command[command.index('--save') + 1]).write_text('truncated JSON')
                    return SimpleNamespace(returncode=0)
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=invalid):
                    play_alpha.launch(binary, installation, resume=True)
                self.assertEqual(pointer.read_bytes(), previous)

    def test_malformed_continue_records_are_actionable_and_left_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            with mock.patch.object(play_alpha, 'REPOSITORY', root), \
                    mock.patch.object(play_alpha.subprocess, 'run') as child:
                sessions = play_alpha.session_root()
                pointer = sessions / 'last-session.json'
                invalid = [None, [], 'record', 7, {}, {'schema': 2},
                           {'schema': 1, 'game_root': str(installation)}]
                invalid.extend({'schema': 1, 'game_root': str(installation), 'save': value}
                               for value in (None, [], 7, '', 'bad\0path'))
                for value in invalid:
                    with self.subTest(value=value):
                        original = json.dumps(value).encode()
                        pointer.write_bytes(original)
                        with self.assertRaisesRegex(ValueError, 'last-session record.*--load'):
                            play_alpha.launch(binary, installation, resume=True)
                        self.assertEqual(pointer.read_bytes(), original)
                for original in (b'truncated JSON', b'\xff'):
                    with self.subTest(original=original):
                        pointer.write_bytes(original)
                        with self.assertRaisesRegex(ValueError, 'last-session record.*--load'):
                            play_alpha.launch(binary, installation, resume=True)
                        self.assertEqual(pointer.read_bytes(), original)
                child.assert_not_called()

    def test_changed_continue_save_and_other_installation_do_not_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            source = play_alpha.HOUSEHOLDS['newbie']
            with mock.patch.object(play_alpha, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()):
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(source)):
                    play_alpha.launch(binary, installation)
                other = root / 'other-installation'
                other.mkdir()
                with mock.patch.object(play_alpha.subprocess, 'run') as child:
                    with self.assertRaisesRegex(ValueError, 'different installation'):
                        play_alpha.launch(binary, other, resume=True)
                    pointer = json.loads((root / 'cache/alpha-sessions/last-session.json').read_text())
                    Path(pointer['save']).write_text(json.dumps(saved(source, 999)))
                    with self.assertRaisesRegex(ValueError, 'missing or changed'):
                        play_alpha.launch(binary, installation, resume=True)
                    child.assert_not_called()

    def test_symlinked_cache_is_rejected_before_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            elsewhere = root / 'elsewhere'
            elsewhere.mkdir()
            (root / 'cache').symlink_to(elsewhere, target_is_directory=True)
            with mock.patch.object(play_alpha, 'REPOSITORY', root), mock.patch.object(play_alpha.subprocess, 'run') as child:
                with self.assertRaisesRegex(ValueError, 'real directory'):
                    play_alpha.launch(binary, installation)
                child.assert_not_called()

    def test_observation_arguments_are_forwarded_unchanged_for_fresh_continue_and_explicit_load(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            observations = root / 'cache/observations'
            observations.mkdir(parents=True)
            library = observations / 'present library.dylib'
            library.write_bytes(b'synthetic library; never loaded')
            library_argument = 'cache//observations/present library.dylib'
            source = play_alpha.HOUSEHOLDS['goth']
            first_save, original = None, None
            with mock.patch.object(play_alpha, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()):
                for phase, tick in (('fresh', 123), ('continue', 456), ('load', 789)):
                    with self.subTest(phase=phase):
                        options = {'household': 'goth'} if phase == 'fresh' else (
                            {'resume': True} if phase == 'continue' else {'load': first_save})
                        output_argument = 'cache//observations/' + phase + ' telemetry'
                        with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(source, tick)) as child:
                            self.assertEqual(play_alpha.launch(
                                binary, installation, **options,
                                window_session_output=output_argument,
                                window_session_present_library=library_argument), 0)
                        command = child.call_args.args[0]
                        child.assert_called_once_with(command, cwd=root, check=False)
                        self.assertEqual(command[-4:], ['--window-session-output', output_argument,
                                                       '--window-session-present-library', library_argument])
                        executable = Path(command[0])
                        self.assertNotEqual(executable, binary)
                        self.assertEqual(executable.stat().st_mode & 0o222, 0)
                        output = Path(command[command.index('--save') + 1])
                        receipt = json.loads((output.parent / 'launch.json').read_text())
                        self.assertEqual(receipt['command'], command)
                        self.assertFalse(receipt['performance_qualified'])
                        self.assertEqual(receipt['exit_code'], 0)
                        self.assertTrue(receipt['continue_available'])
                        self.assertTrue(receipt['executable_unchanged'])
                        self.assertTrue(receipt['starting_copy_unchanged'])
                        if phase == 'fresh':
                            first_save, original = output, output.read_bytes()
                            self.assertIn('--inferred-routing', command)
                            self.assertIn('--buy-preview', command)
                            self.assertEqual(command[command.index('--neighborhood-policy') + 1],
                                             'installed-houses-ascending-v1')
                            self.assertNotIn('--load', command)
                        else:
                            starting = Path(command[command.index('--load') + 1])
                            self.assertEqual(starting.read_bytes(), original)
                            self.assertEqual(starting.stat().st_mode & 0o222, 0)
                            self.assertNotEqual(output, first_save)
                            self.assertEqual(first_save.read_bytes(), original)
                            self.assertEqual(receipt['input'], str(first_save))
                            self.assertEqual(receipt['input_sha256'], play_alpha.digest(first_save))
                            self.assertNotIn('--inferred-routing', command)
                            self.assertNotIn('--buy-preview', command)
                            self.assertNotIn('--neighborhood-policy', command)
                        pointer = json.loads((root / 'cache/alpha-sessions/last-session.json').read_text())
                        self.assertEqual(pointer['save'], str(output))
                        self.assertEqual(pointer['sha256'], play_alpha.digest(output))
                        self.assertEqual(json.loads(output.read_text())['session']['synthetic_tick'], tick)
                        self.assertFalse((root / output_argument).exists())
            self.assertEqual(library.read_bytes(), b'synthetic library; never loaded')

    def test_cli_observation_output_alone_is_opt_in_and_forwarded_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            observations = root / 'cache/observations'
            observations.mkdir(parents=True)
            output = str(observations) + '//fresh telemetry'
            arguments = ['play_alpha.py', '--binary', str(binary), '--root', str(installation),
                         '--household', 'newbie', '--window-session-output', output]
            with mock.patch.object(play_alpha, 'REPOSITORY', root), mock.patch.object(sys, 'argv', arguments), \
                    contextlib.redirect_stdout(io.StringIO()), \
                    mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(play_alpha.HOUSEHOLDS['newbie'])) as child:
                self.assertEqual(play_alpha.main(), 0)
            command = child.call_args.args[0]
            self.assertEqual(command[-2:], ['--window-session-output', output])
            self.assertNotIn('--window-session-present-library', command)

    def test_observed_child_failure_keeps_the_original_continue_pointer_and_selected_save(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            observations = root / 'cache/observations'
            observations.mkdir(parents=True)
            library = observations / 'present.dylib'
            library.write_bytes(b'synthetic library; never loaded')
            source = play_alpha.HOUSEHOLDS['newbie']
            with mock.patch.object(play_alpha, 'REPOSITORY', root), contextlib.redirect_stdout(io.StringIO()):
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(source)):
                    play_alpha.launch(binary, installation)
                pointer = root / 'cache/alpha-sessions/last-session.json'
                original_pointer = pointer.read_bytes()
                selected = Path(json.loads(original_pointer)['save'])
                original_save = selected.read_bytes()
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(source, 99, code=7)) as child:
                    self.assertEqual(play_alpha.launch(
                        binary, installation, resume=True,
                        window_session_output=observations / 'telemetry',
                        window_session_present_library=library), 7)
                command = child.call_args.args[0]
                self.assertEqual(command[-4:], ['--window-session-output', str(observations / 'telemetry'),
                                               '--window-session-present-library', str(library)])
                self.assertEqual(pointer.read_bytes(), original_pointer)
                self.assertEqual(selected.read_bytes(), original_save)
                receipt = json.loads((Path(command[0]).parent / 'launch.json').read_text())
                self.assertEqual(receipt['exit_code'], 7)
                self.assertNotIn('continue_available', receipt)
                self.assertFalse(receipt['performance_qualified'])

    def test_unsafe_observation_outputs_never_stage_or_start_a_child_or_change_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            observations = root / 'cache/observations'
            observations.mkdir(parents=True)
            existing = observations / 'existing-file'
            existing.write_bytes(b'keep existing evidence')
            selected = observations / 'selected-save.json'
            selected.write_text(json.dumps(saved(play_alpha.HOUSEHOLDS['newbie'])))
            original = selected.read_bytes()
            (observations / 'existing-directory').mkdir()
            (observations / 'linked-file').symlink_to(existing)
            (observations / 'broken-link').symlink_to(observations / 'absent')
            (observations / 'linked-parent').symlink_to(observations, target_is_directory=True)
            (observations / 'escape').symlink_to(installation, target_is_directory=True)
            cached_installation = observations / 'installed'
            cached_installation.mkdir()
            with mock.patch.object(play_alpha, 'REPOSITORY', root), mock.patch.object(play_alpha.subprocess, 'run') as child:
                sessions = play_alpha.session_root()
                pointer = sessions / 'last-session.json'
                pointer.write_bytes(b'existing pointer left unchanged')
                invalid = [root / 'outside-cache', installation / 'telemetry', observations,
                           existing, selected, observations / 'existing-directory',
                           observations / 'missing-parent/telemetry',
                           observations / '../traversal', observations / '../../outside-cache',
                           observations / 'linked-file', observations / 'broken-link',
                           observations / 'linked-parent/telemetry', observations / 'escape/telemetry',
                           sessions / 'telemetry', sessions / 'household-next/player-save.json',
                           '', 'bad\0path', b'cache/observations/bytes', 7]
                for output in invalid:
                    with self.subTest(output=output), self.assertRaises((OSError, ValueError, TypeError)):
                        play_alpha.launch(binary, installation, load=selected, window_session_output=output)
                with self.assertRaisesRegex(ValueError, 'separate from installation'):
                    play_alpha.launch(binary, cached_installation,
                                      window_session_output=cached_installation / 'telemetry')
                self.assertEqual(list(sessions.iterdir()), [pointer])
                self.assertEqual(pointer.read_bytes(), b'existing pointer left unchanged')
                self.assertEqual(selected.read_bytes(), original)
                self.assertEqual(existing.read_bytes(), b'keep existing evidence')
                self.assertEqual(list(installation.iterdir()), [])
                child.assert_not_called()

    def test_observation_library_requires_output_and_an_existing_non_symlink_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            observations = root / 'cache/observations'
            observations.mkdir(parents=True)
            library = observations / 'present.dylib'
            library.write_bytes(b'synthetic library; never loaded')
            linked = observations / 'linked.dylib'
            linked.symlink_to(library)
            parent = observations / 'linked-parent'
            parent.symlink_to(observations, target_is_directory=True)
            output = observations / 'telemetry'
            with mock.patch.object(play_alpha, 'REPOSITORY', root), mock.patch.object(play_alpha.subprocess, 'run') as child:
                with self.assertRaisesRegex(ValueError, 'requires --window-session-output'):
                    play_alpha.launch(binary, installation, window_session_present_library=library)
                for invalid in (observations / 'missing.dylib', observations, linked, parent / library.name,
                                '', 'bad\0library', b'bytes.dylib', 7):
                    with self.subTest(library=invalid), self.assertRaises((OSError, ValueError, TypeError)):
                        play_alpha.launch(binary, installation, window_session_output=output,
                                          window_session_present_library=invalid)
                self.assertFalse(output.exists())
                self.assertEqual(list((root / 'cache/alpha-sessions').iterdir()), [])
                child.assert_not_called()

    def test_cli_forwards_both_observation_options_and_rejects_library_without_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root, binary, installation = self.prepare(directory)
            observations = root / 'cache/observations'
            observations.mkdir(parents=True)
            library = observations / 'present.dylib'
            library.write_bytes(b'synthetic library; never loaded')
            arguments = ['play_alpha.py', '--binary', str(binary), '--root', str(installation),
                         '--window-session-present-library', str(library)]
            with mock.patch.object(play_alpha, 'REPOSITORY', root), mock.patch.object(sys, 'argv', arguments), \
                    contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()) as errors:
                with mock.patch.object(play_alpha.subprocess, 'run') as child:
                    self.assertEqual(play_alpha.main(), 1)
                    self.assertIn('requires --window-session-output', errors.getvalue())
                    child.assert_not_called()
                arguments.extend(['--window-session-output', str(observations / 'telemetry')])
                with mock.patch.object(play_alpha.subprocess, 'run', side_effect=self.child(play_alpha.HOUSEHOLDS['newbie'])) as child:
                    self.assertEqual(play_alpha.main(), 0)
                self.assertEqual(child.call_args.args[0][-4:], [
                    '--window-session-output', str(observations / 'telemetry'),
                    '--window-session-present-library', str(library)])


if __name__ == '__main__':
    unittest.main()
