#!/usr/bin/env python3
"""Local fake-child tests; never launch sims-room or read installed game assets."""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest import mock

import play_alpha


def saved(source, tick=12):
    return {'session': {'version': 2, 'source': source, 'tick': tick},
            'dialog_client': {}, 'floor': 1, 'zoom': 1.0, 'resident': 1, 'selected': None}


class FabricatedCompletion(play_alpha.TrustedCompletion):
    """Test fixture only: it does not establish trusted runtime authority."""

    def __init__(self, observation):
        self.observation = observation

    def observe(self):
        return self.observation


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.workspace = Path(self.temporary.name).resolve()
        self.binary = self.workspace / 'sims-room-source'
        self.binary.write_bytes(b'fake executable, not a native program\n')
        self.binary.chmod(0o700)
        self.installation = self.workspace / 'installation'
        self.installation.mkdir()
        self.sessions = self.workspace / 'cache/alpha-sessions'
        self.commands = []
        self.repository_patch = mock.patch.object(play_alpha, 'REPOSITORY', self.workspace)
        self.repository_patch.start()
        self.addCleanup(self.repository_patch.stop)

    def write_json(self, path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value), encoding='utf-8')
        return path

    def option(self, command, name):
        return command[command.index(name) + 1]

    def child(self, command, **arguments):
        source = self.option(command, '--original')
        self.write_json(Path(self.option(command, '--save')), saved(source))
        return subprocess.CompletedProcess(command, 0)

    def invoke(self, effect=None, **arguments):
        def run(command, **options):
            self.commands.append(command)
            return (effect or self.child)(command, **options)

        stderr = io.StringIO()
        with mock.patch.object(play_alpha.subprocess, 'run', side_effect=run) as child, \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(stderr):
            result = play_alpha.launch(self.binary, self.installation, **arguments)
        self.last_child = child
        self.last_stderr = stderr.getvalue()
        return result

    def receipt(self, command=None):
        if command is None and self.commands:
            command = self.commands[-1]
        if command is not None:
            path = Path(self.option(command, '--save')).parent / 'launch.json'
        else:
            path = next(self.sessions.glob('household-*/launch.json'))
        return json.loads(path.read_bytes())

    def previous(self):
        self.assertEqual(self.invoke(), 0)
        pointer = self.sessions / 'last-session.json'
        return pointer, pointer.read_bytes()

    def test_household_mappings_and_no_inferred_or_policy_defaults(self):
        expected = {'newbie': 'UserData/Houses/House07.iff', 'goth': 'UserData/Houses/House05.iff',
                    'mashuga': 'UserData2/Houses/House09.iff'}
        for household, source in expected.items():
            with self.subTest(household=household):
                self.assertEqual(self.invoke(household=household), 0)
                command = self.commands[-1]
                self.assertEqual(self.option(command, '--original'), source)
                self.assertEqual(command[1:], ['--original', source, '--root', str(self.installation),
                                              '--save', self.option(command, '--save'), '--paused'])
                self.assertFalse(any('policy' in argument or 'inferred' in argument or 'preview' in argument
                                     for argument in command))

    def test_default_is_newbie_and_each_child_has_a_fresh_run(self):
        self.assertEqual(self.invoke(), 0)
        self.assertEqual(self.invoke(), 0)
        first, second = self.commands
        self.assertNotEqual(Path(first[0]).parent, Path(second[0]).parent)
        self.assertEqual(self.option(first, '--original'), play_alpha.HOUSEHOLDS['newbie'])

    def test_load_uses_separate_readonly_copies_and_paused_cli(self):
        source = play_alpha.HOUSEHOLDS['mashuga']
        selected = self.write_json(self.workspace / 'selected.json', saved(source, 91))
        original_save = selected.read_bytes()
        original_binary = self.binary.read_bytes()

        def child(command, **options):
            pinned = Path(command[0])
            starting = Path(self.option(command, '--load'))
            output = Path(self.option(command, '--save'))
            self.assertEqual(pinned.read_bytes(), original_binary)
            self.assertEqual(starting.read_bytes(), original_save)
            self.assertNotEqual(pinned.stat().st_ino, self.binary.stat().st_ino)
            self.assertNotEqual(starting.stat().st_ino, selected.stat().st_ino)
            self.assertEqual(pinned.stat().st_mode & 0o777, 0o500)
            self.assertEqual(starting.stat().st_mode & 0o777, 0o400)
            self.assertNotEqual(output, starting)
            self.assertNotEqual(output, selected)
            self.assertFalse(output.exists())
            self.assertNotIn('--inferred-routing', command)
            self.assertEqual(self.option(command, '--original'), source)
            self.assertIn('--paused', command)
            return self.child(command, **options)

        self.assertEqual(self.invoke(child, load=selected), 0)
        self.assertEqual(selected.read_bytes(), original_save)
        self.assertEqual(self.binary.read_bytes(), original_binary)

    def test_continue_loads_previous_save_into_a_new_readonly_copy(self):
        pointer, previous = self.previous()
        selected = Path(json.loads(previous)['save'])
        original = selected.read_bytes()
        self.assertEqual(self.invoke(resume=True), 0)
        command = self.commands[-1]
        self.assertEqual(Path(self.option(command, '--load')).read_bytes(), original)
        self.assertEqual(selected.read_bytes(), original)
        self.assertNotEqual(Path(self.option(command, '--save')), selected)
        self.assertNotEqual(pointer.read_bytes(), previous)

    def test_successful_save_pointer_binds_canonical_installation_path_hash_and_household(self):
        self.assertEqual(self.invoke(household='goth'), 0)
        pointer = json.loads((self.sessions / 'last-session.json').read_bytes())
        output = Path(self.option(self.commands[-1], '--save'))
        self.assertEqual(pointer, {'schema': 1, 'game_root': str(self.installation.resolve()),
                                  'save': str(output), 'sha256': play_alpha.digest(output),
                                  'source': play_alpha.HOUSEHOLDS['goth']})

    def test_continue_rejects_save_replaced_after_pointer_verification(self):
        for replacement_source in (play_alpha.HOUSEHOLDS['goth'], play_alpha.HOUSEHOLDS['newbie']):
            with self.subTest(replacement_source=replacement_source):
                pointer, previous = self.previous()
                identity = json.loads(previous)
                existing_runs = set(self.sessions.glob('household-*'))
                select_save = play_alpha.last_save

                def replace_verified_save(root, game_root):
                    selected = select_save(root, game_root)
                    self.write_json(Path(identity['save']), saved(replacement_source, 91))
                    return selected

                with mock.patch.object(play_alpha, 'last_save', side_effect=replace_verified_save):
                    self.assertEqual(self.invoke(resume=True), 1)
                self.last_child.assert_not_called()
                self.assertEqual(pointer.read_bytes(), previous)
                new_runs = set(self.sessions.glob('household-*')) - existing_runs
                self.assertEqual(len(new_runs), 1)
                receipt = json.loads((new_runs.pop() / 'launch.json').read_bytes())
                self.assertEqual(receipt['process']['status'], 'not_started')
                self.assertIn('last-session identity', receipt['launch_error'])
                self.assertFalse(receipt['continue_candidate'])

    def test_no_save_preserves_previous_pointer(self):
        pointer, previous = self.previous()
        self.assertEqual(self.invoke(lambda command, **options: subprocess.CompletedProcess(command, 0)), 0)
        self.assertEqual(pointer.read_bytes(), previous)
        self.assertEqual(self.receipt()['save_validation'], 'no_save_written')

    def test_nonzero_child_preserves_pointer_and_actual_status_even_with_save(self):
        pointer, previous = self.previous()

        def child(command, **options):
            self.child(command, **options)
            return subprocess.CompletedProcess(command, 23)

        self.assertEqual(self.invoke(child), 23)
        self.assertEqual(pointer.read_bytes(), previous)
        self.assertEqual(self.receipt()['process'], {'status': 'exited', 'returncode': 23})

    def test_signal_status_is_not_reported_as_success(self):
        pointer, previous = self.previous()
        self.assertEqual(self.invoke(lambda command, **options: subprocess.CompletedProcess(command, -9)), -9)
        self.assertEqual(self.receipt()['exit_code'], -9)
        self.assertEqual(pointer.read_bytes(), previous)

    def test_timeout_preserves_pointer_and_is_recorded_without_fabricated_exit_status(self):
        pointer, previous = self.previous()

        def child(command, **options):
            self.assertEqual(options['timeout'], 0.25)
            self.child(command, **options)
            raise subprocess.TimeoutExpired(command, 0.25)

        self.assertEqual(self.invoke(child, timeout=0.25), 124)
        receipt = self.receipt()
        self.assertEqual(receipt['process']['status'], 'timeout')
        self.assertIn('Household timed out', self.last_stderr)
        self.assertIsNone(receipt['exit_code'])
        self.assertEqual(pointer.read_bytes(), previous)

    def test_launch_error_preserves_pointer_and_receipt(self):
        pointer, previous = self.previous()

        def child(command, **options):
            raise OSError('fake launch denied')

        self.assertEqual(self.invoke(child), 1)
        self.assertEqual(self.receipt()['process']['status'], 'launch_error')
        self.assertIn('fake launch denied', self.receipt()['process']['error'])
        self.assertIn('fake launch denied', self.last_stderr)
        self.assertEqual(pointer.read_bytes(), previous)

    def test_invalid_or_foreign_save_output_preserves_pointer(self):
        pointer, previous = self.previous()
        for value in ('{', '[]', json.dumps(saved(play_alpha.HOUSEHOLDS['goth'])),
                      json.dumps({'session': {'version': 2, 'source': play_alpha.HOUSEHOLDS['newbie']}})):
            with self.subTest(value=value):
                def child(command, **options):
                    Path(self.option(command, '--save')).write_text(value)
                    return subprocess.CompletedProcess(command, 0)

                self.assertEqual(self.invoke(child), 1)
                self.assertEqual(pointer.read_bytes(), previous)
                self.assertIn('save_error', self.receipt())

    def test_over_nested_save_output_retains_actual_child_outcome(self):
        pointer, previous = self.previous()

        def child(command, **options):
            source = json.dumps(self.option(command, '--original'))
            payload = ('{"session":{"version":2,"source":' + source
                       + '},"dialog_client":{},"nested":' + '[' * 1200 + '0' + ']' * 1200 + '}')
            Path(self.option(command, '--save')).write_text(payload)
            return subprocess.CompletedProcess(command, 0)

        self.assertEqual(self.invoke(child), 1)
        receipt = self.receipt()
        self.assertEqual(receipt['process'], {'status': 'exited', 'returncode': 0})
        self.assertEqual(receipt['exit_code'], 0)
        self.assertEqual(receipt['launcher_exit_code'], 1)
        self.assertIn('nesting', receipt['save_error'])
        self.assertFalse(receipt['continue_candidate'])
        self.assertEqual(pointer.read_bytes(), previous)

    def test_over_nested_json_is_a_handled_validation_error(self):
        selected = self.workspace / 'deep.json'
        selected.write_text('[' * 1200 + '0' + ']' * 1200)
        with self.assertRaisesRegex(ValueError, 'nesting'):
            play_alpha.read_json(selected)

    def test_structural_envelope_is_not_deep_runtime_restore_validation(self):
        minimal = self.write_json(self.workspace / 'minimal.json', {
            'session': {'version': 2, 'source': play_alpha.HOUSEHOLDS['newbie']}, 'dialog_client': {}})
        self.assertEqual(self.invoke(load=minimal), 0)
        self.assertEqual(self.receipt()['save_validation'], 'structurally_valid_not_runtime_restore_verified')

    def test_invalid_selected_envelopes_never_launch(self):
        values = ['broken', 'null', '[]', '{"session":{"version":2,"source":"x"},"dialog_client":NaN}',
                  '{"session":{},"session":{},"dialog_client":{}}']
        for version in (1, True, 2.0, '2', None):
            value = saved(play_alpha.HOUSEHOLDS['newbie'])
            value['session']['version'] = version
            values.append(json.dumps(value))
        for client in (None, [], 7):
            value = saved(play_alpha.HOUSEHOLDS['newbie'])
            value['dialog_client'] = client
            values.append(json.dumps(value))
        for source in ('', '/foreign/house.iff', '../house.iff', 'house\0.iff', 'dir\\house.iff'):
            values.append(json.dumps(saved(source)))
        for value in values:
            with self.subTest(value=value):
                selected = self.workspace / 'invalid.json'
                selected.write_text(value)
                self.assertEqual(self.invoke(load=selected), 1)
                self.last_child.assert_not_called()
                self.assertEqual(selected.read_text(), value)

    def test_missing_selected_input_retains_pinning_error_receipt(self):
        self.assertEqual(self.invoke(load=self.workspace / 'missing.json'), 1)
        self.last_child.assert_not_called()
        receipt = self.receipt()
        self.assertEqual(receipt['process']['status'], 'not_started')
        self.assertEqual(receipt['inputs'][-1]['kind'], 'selected_save')
        self.assertIn('error', receipt['inputs'][-1]['after'])

    def test_selected_source_disappearing_during_copy_retains_receipt(self):
        selected = self.write_json(self.workspace / 'selected.json', saved(play_alpha.HOUSEHOLDS['newbie']))
        copy = play_alpha.shutil.copyfileobj

        def copy_input(reader, writer):
            copy(reader, writer)
            if Path(reader.name) == selected:
                selected.unlink()

        with mock.patch.object(play_alpha.shutil, 'copyfileobj', side_effect=copy_input):
            self.assertEqual(self.invoke(load=selected), 1)
        self.last_child.assert_not_called()
        self.assertIn('launch_error', self.receipt())
        self.assertFalse(self.receipt()['inputs'][-1]['unchanged'])
        self.assertEqual(self.receipt()['input_sha256'], self.receipt()['inputs'][-1]['before']['sha256'])

    def test_binary_copy_error_and_source_deletion_retain_hash_and_primary_error(self):
        checksum = play_alpha.digest(self.binary)

        def copy_input(reader, writer):
            self.binary.unlink()
            raise OSError('primary pinning copy failure')

        with mock.patch.object(play_alpha.shutil, 'copyfileobj', side_effect=copy_input):
            self.assertEqual(self.invoke(), 1)
        self.last_child.assert_not_called()
        receipt = self.receipt()
        self.assertEqual(receipt['binary_sha256'], checksum)
        self.assertIn('primary pinning copy failure', receipt['launch_error'])
        self.assertEqual(receipt['process']['status'], 'not_started')
        self.assertIn('error', receipt['inputs'][0]['after'])

    def test_child_is_blocking_without_shell_or_capture_and_uses_repository_cwd(self):
        def child(command, **options):
            self.assertEqual(options, {'cwd': self.workspace, 'check': False, 'timeout': None})
            self.assertEqual(self.receipt(command)['process']['status'], 'launching')
            return self.child(command, **options)

        self.assertEqual(self.invoke(child), 0)

    def test_source_and_pinned_mutations_and_deletions_reject_publication(self):
        pointer, previous = self.previous()
        for kind in ('source_binary', 'pinned_binary', 'source_save', 'pinned_save'):
            for operation in ('mutate', 'delete'):
                with self.subTest(kind=kind, operation=operation):
                    self.binary.write_bytes(b'fresh fake executable')
                    self.binary.chmod(0o700)
                    selected = self.write_json(self.workspace / 'selected.json', saved(play_alpha.HOUSEHOLDS['newbie']))

                    def child(command, **options):
                        target = {'source_binary': self.binary, 'pinned_binary': Path(command[0]),
                                  'source_save': selected, 'pinned_save': Path(self.option(command, '--load'))}[kind]
                        target.chmod(0o700)
                        if operation == 'delete':
                            target.unlink()
                        else:
                            target.write_bytes(b'mutated input')
                        return self.child(command, **options)

                    self.assertEqual(self.invoke(child, load=selected), 1)
                    self.assertEqual(pointer.read_bytes(), previous)
                    self.assertEqual(self.receipt()['exit_code'], 0)
                    self.assertFalse(self.receipt()['inputs_unchanged'])

    def test_input_disappearance_does_not_mask_nonzero_process_error(self):
        pointer, previous = self.previous()

        def child(command, **options):
            Path(command[0]).unlink()
            self.binary.unlink()
            return subprocess.CompletedProcess(command, 37)

        self.assertEqual(self.invoke(child), 37)
        receipt = self.receipt()
        self.assertEqual(receipt['process']['returncode'], 37)
        self.assertIn('integrity_error', receipt)
        self.assertEqual(pointer.read_bytes(), previous)

    def test_input_disappearance_does_not_mask_launch_error_or_timeout(self):
        pointer, previous = self.previous()
        for failure in ('launch_error', 'timeout'):
            with self.subTest(failure=failure):
                self.binary.write_bytes(b'fresh fake executable')
                self.binary.chmod(0o700)

                def child(command, **options):
                    self.binary.unlink()
                    Path(command[0]).unlink()
                    if failure == 'timeout':
                        raise subprocess.TimeoutExpired(command, 3)
                    raise OSError('primary process failure')

                self.assertEqual(self.invoke(child), 124 if failure == 'timeout' else 1)
                self.assertEqual(self.receipt()['process']['status'], failure)
                self.assertIn('error', self.receipt()['process'])
                self.assertEqual(pointer.read_bytes(), previous)

    def test_missing_binary_retains_receipt_without_launch(self):
        self.binary.unlink()
        self.assertEqual(self.invoke(), 1)
        self.last_child.assert_not_called()
        self.assertEqual(self.receipt()['process']['status'], 'not_started')

    def test_nonexecutable_binary_is_rejected_without_launch(self):
        self.binary.chmod(0o600)
        self.assertEqual(self.invoke(), 1)
        self.last_child.assert_not_called()

    def test_continue_without_pointer_never_falls_back_to_fresh(self):
        self.assertEqual(self.invoke(resume=True), 1)
        self.last_child.assert_not_called()

    def test_continue_rejects_changed_or_missing_save(self):
        pointer, previous = self.previous()
        selected = Path(json.loads(previous)['save'])
        selected.write_bytes(b'changed')
        self.assertEqual(self.invoke(resume=True), 1)
        self.last_child.assert_not_called()
        selected.unlink()
        self.assertEqual(self.invoke(resume=True), 1)
        self.last_child.assert_not_called()
        self.assertEqual(pointer.read_bytes(), previous)

    def test_continue_rejects_foreign_installation(self):
        pointer, previous = self.previous()
        self.installation = self.workspace / 'other-installation'
        self.installation.mkdir()
        self.assertEqual(self.invoke(resume=True), 1)
        self.last_child.assert_not_called()
        self.assertEqual(pointer.read_bytes(), previous)

    def test_continue_canonicalizes_installation_alias(self):
        self.previous()
        alias = self.workspace / 'installation-alias'
        alias.symlink_to(self.installation, target_is_directory=True)
        self.installation = alias
        self.assertEqual(self.invoke(resume=True), 0)
        self.assertEqual(self.receipt()['game_root'], str(alias.resolve()))

    def test_continue_rejects_corrupt_pointer_schema_identity_and_hash(self):
        pointer, previous = self.previous()
        baseline = json.loads(previous)
        invalid = ['{', '[]', 'null']
        for field, value in (('schema', 2), ('schema', True), ('source', 'other'), ('sha256', 'wrong'),
                             ('save', None), ('save', 'relative.json'), ('save', 'bad\0path')):
            invalid.append(json.dumps({**baseline, field: value}))
        for value in invalid:
            with self.subTest(value=value):
                pointer.write_text(value)
                self.assertEqual(self.invoke(resume=True), 1)
                self.last_child.assert_not_called()
                self.assertEqual(pointer.read_text(), value)

    def test_continue_rejects_outside_session_and_traversal_paths(self):
        pointer, previous = self.previous()
        baseline = json.loads(previous)
        external = self.write_json(self.workspace / 'external.json', saved(baseline['source']))
        for path in (external, self.sessions / 'arbitrary.json',
                     self.sessions / 'household-any/../arbitrary.json'):
            with self.subTest(path=path):
                if '..' not in path.parts:
                    self.write_json(path, saved(baseline['source']))
                value = {**baseline, 'save': str(path), 'sha256': play_alpha.digest(external)}
                self.write_json(pointer, value)
                before = pointer.read_bytes()
                self.assertEqual(self.invoke(resume=True), 1)
                self.last_child.assert_not_called()
                self.assertEqual(pointer.read_bytes(), before)

    def test_continue_rejects_symlink_pointer(self):
        pointer, previous = self.previous()
        target = self.workspace / 'external-pointer.json'
        target.write_bytes(previous)
        pointer.unlink()
        pointer.symlink_to(target)
        self.assertEqual(self.invoke(resume=True), 1)
        self.last_child.assert_not_called()
        self.assertEqual(target.read_bytes(), previous)

    def test_continue_rejects_symlink_save_and_parent_escape(self):
        pointer, previous = self.previous()
        baseline = json.loads(previous)
        external = self.write_json(self.workspace / 'foreign/player-save.json', saved(baseline['source']))
        selected = Path(baseline['save'])
        selected.unlink()
        selected.symlink_to(external)
        self.assertEqual(self.invoke(resume=True), 1)
        self.last_child.assert_not_called()
        selected.unlink()
        escaped = self.sessions / 'household-escaped'
        escaped.symlink_to(external.parent, target_is_directory=True)
        self.write_json(pointer, {**baseline, 'save': str(escaped / 'player-save.json')})
        self.assertEqual(self.invoke(resume=True), 1)
        self.last_child.assert_not_called()

    def test_selected_input_symlinks_are_rejected(self):
        selected = self.write_json(self.workspace / 'selected.json', saved(play_alpha.HOUSEHOLDS['newbie']))
        alias = self.workspace / 'alias.json'
        alias.symlink_to(selected)
        self.assertEqual(self.invoke(load=alias), 1)
        self.last_child.assert_not_called()

    def test_cache_or_session_symlink_is_rejected(self):
        external = self.workspace / 'foreign-cache'
        external.mkdir()
        cache = self.workspace / 'cache'
        cache.symlink_to(external, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.invoke()
        cache.unlink()
        cache.mkdir()
        self.sessions.symlink_to(external, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.invoke()
        self.assertEqual(list(external.iterdir()), [])

    def test_session_output_cannot_be_in_installation(self):
        self.installation = self.workspace
        with self.assertRaisesRegex(ValueError, 'separate'):
            self.invoke()

    def test_preexisting_output_is_rejected_without_launch(self):
        create = play_alpha.tempfile.mkdtemp

        def create_run(*arguments, **options):
            path = Path(create(*arguments, **options))
            (path / 'player-save.json').write_bytes(b'keep existing output')
            return str(path)

        with mock.patch.object(play_alpha.tempfile, 'mkdtemp', side_effect=create_run):
            self.assertEqual(self.invoke(), 1)
        self.last_child.assert_not_called()
        self.assertEqual(next(self.sessions.glob('household-*/player-save.json')).read_bytes(), b'keep existing output')

    def test_symlink_save_output_cannot_publish(self):
        pointer, previous = self.previous()
        external = self.write_json(self.workspace / 'external.json', saved(play_alpha.HOUSEHOLDS['newbie']))

        def child(command, **options):
            Path(self.option(command, '--save')).symlink_to(external)
            return subprocess.CompletedProcess(command, 0)

        self.assertEqual(self.invoke(child), 1)
        self.assertEqual(pointer.read_bytes(), previous)

    def test_output_changed_or_deleted_during_validation_cannot_publish(self):
        pointer, previous = self.previous()
        inspect = play_alpha.envelope
        for operation in ('change', 'delete'):
            with self.subTest(operation=operation):
                def envelope(path):
                    value = inspect(path)
                    if path.name == 'player-save.json':
                        if operation == 'delete':
                            path.unlink()
                        else:
                            path.write_bytes(b'changed during validation')
                    return value

                with mock.patch.object(play_alpha, 'envelope', side_effect=envelope):
                    self.assertEqual(self.invoke(), 1)
                self.assertEqual(pointer.read_bytes(), previous)

    def test_input_mutation_during_save_validation_cannot_publish(self):
        pointer, previous = self.previous()
        inspect = play_alpha.envelope

        def envelope(path):
            value = inspect(path)
            if path.name == 'player-save.json':
                self.binary.write_bytes(b'changed during output validation')
            return value

        with mock.patch.object(play_alpha, 'envelope', side_effect=envelope):
            self.assertEqual(self.invoke(), 1)
        self.assertEqual(pointer.read_bytes(), previous)
        self.assertFalse(self.receipt()['inputs_unchanged'])

    def test_input_or_output_mutation_at_final_receipt_boundary_cannot_publish(self):
        pointer, previous = self.previous()
        write = play_alpha.atomic_json
        for kind in ('pinned_input', 'save_output'):
            with self.subTest(kind=kind):
                def atomic_json(path, value):
                    write(path, value)
                    if path.name == 'launch.json' and value.get('continue_candidate'):
                        target = (Path(value['command'][0]) if kind == 'pinned_input'
                                  else Path(value['save_output']))
                        target.chmod(0o600)
                        target.write_bytes(b'changed at publication boundary')

                with mock.patch.object(play_alpha, 'atomic_json', side_effect=atomic_json):
                    self.assertEqual(self.invoke(), 1)
                self.assertEqual(pointer.read_bytes(), previous)
                self.assertIn('publication_error', self.receipt())

    def test_hardlinked_existing_save_cannot_be_published_as_new_output(self):
        pointer, previous = self.previous()
        external = self.write_json(self.workspace / 'external.json', saved(play_alpha.HOUSEHOLDS['newbie']))

        def child(command, **options):
            os.link(external, Path(self.option(command, '--save')))
            return subprocess.CompletedProcess(command, 0)

        self.assertEqual(self.invoke(child), 1)
        self.assertEqual(pointer.read_bytes(), previous)

    def test_atomic_pointer_failure_preserves_previous_pointer_and_records_process(self):
        pointer, previous = self.previous()
        replace = play_alpha.os.replace

        def replace_file(source, target):
            if target == pointer:
                raise OSError('fake pointer replacement denied')
            return replace(source, target)

        with mock.patch.object(play_alpha.os, 'replace', side_effect=replace_file):
            self.assertEqual(self.invoke(), 1)
        self.assertEqual(pointer.read_bytes(), previous)
        self.assertEqual(self.receipt()['process']['returncode'], 0)
        self.assertIn('publication_error', self.receipt())

    def test_final_receipt_failure_preserves_previous_receipt_and_pointer(self):
        pointer, previous = self.previous()
        replace = play_alpha.os.replace
        calls = []

        def replace_file(source, target):
            if target.name == 'launch.json':
                calls.append(target)
                if len(calls) == 3:
                    raise OSError('fake final receipt replacement denied')
            return replace(source, target)

        with mock.patch.object(play_alpha.os, 'replace', side_effect=replace_file):
            self.assertEqual(self.invoke(), 1)
        self.assertEqual(pointer.read_bytes(), previous)
        self.assertEqual(self.receipt()['process']['status'], 'launching')
        self.assertFalse(list(Path(self.commands[-1][0]).parent.glob('.session-*')))

    def test_atomic_json_serialization_failure_preserves_existing_evidence(self):
        path = self.workspace / 'evidence.json'
        path.write_bytes(b'old evidence')
        with self.assertRaises(ValueError):
            play_alpha.atomic_json(path, {'invalid': float('nan')})
        self.assertEqual(path.read_bytes(), b'old evidence')
        self.assertFalse(list(self.workspace.glob('.session-*')))

    def test_metadata_records_hashes_integrity_and_no_performance_qualification(self):
        selected = self.write_json(self.workspace / 'selected.json', saved(play_alpha.HOUSEHOLDS['newbie']))
        self.assertEqual(self.invoke(load=selected), 0)
        receipt = self.receipt()
        self.assertFalse(receipt['performance_qualified'])
        self.assertEqual(receipt['binary_sha256'], play_alpha.digest(self.binary))
        self.assertEqual(receipt['input_sha256'], play_alpha.digest(selected))
        self.assertEqual(receipt['save_sha256'], play_alpha.digest(Path(receipt['save_output'])))
        self.assertTrue(receipt['inputs_unchanged'])
        self.assertTrue(all(item['before'] == item['after'] and item['pinned_before'] == item['pinned_after']
                            for item in receipt['inputs']))
        self.assertIn('structure-only', receipt['validation_scope'])

    def package_binary(self, checksum=None):
        target = self.workspace / 'Sims Room.app/Contents/MacOS/sims-room'
        target.parent.mkdir(parents=True)
        target.write_bytes(self.binary.read_bytes())
        target.chmod(0o700)
        self.binary = target
        path = target.parent.parent / 'Resources/build-receipt.json'
        self.write_json(path, {'schema': 'sims-room-build-receipt-v1',
                              'executable_sha256': checksum or play_alpha.digest(target),
                              'profile': 'release'})
        return path

    def test_package_receipt_identity_is_verified_and_recorded_without_building(self):
        self.package_binary()
        self.assertEqual(self.invoke(), 0)
        receipt = self.receipt()
        self.assertEqual(receipt['build_receipt']['executable_sha256'], receipt['binary_sha256'])
        self.assertIn('build_receipt', [item['kind'] for item in receipt['inputs']])

    def test_package_receipt_mutation_or_deletion_preserves_previous_pointer(self):
        pointer, previous = self.previous()
        path = self.package_binary()
        value = path.read_bytes()
        for operation in ('mutate', 'delete'):
            with self.subTest(operation=operation):
                path.write_bytes(value)

                def child(command, **options):
                    if operation == 'mutate':
                        path.write_bytes(b'changed package receipt')
                    else:
                        path.unlink()
                    return self.child(command, **options)

                self.assertEqual(self.invoke(child), 1)
                self.assertEqual(pointer.read_bytes(), previous)
                self.assertFalse(self.receipt()['inputs_unchanged'])

    def test_invalid_package_receipt_prevents_launch(self):
        path = self.package_binary('wrong')
        for value in (path.read_text(), '{', '[]'):
            with self.subTest(value=value):
                path.write_text(value)
                self.assertEqual(self.invoke(), 1)
                self.last_child.assert_not_called()

    def test_absent_package_receipt_is_not_fabricated_or_required(self):
        path = self.package_binary()
        path.unlink()
        self.assertEqual(self.invoke(), 0)
        self.assertNotIn('build_receipt', self.receipt())

    def test_optional_native_flags_forward_fresh_output_and_pinned_library_only(self):
        library = self.workspace / 'presentation.dylib'
        library.write_bytes(b'fake library')
        output = self.workspace / 'cache/window-session'

        def child(command, **options):
            self.assertEqual(self.option(command, '--window-session-output'), str(output))
            pinned = Path(self.option(command, '--window-session-present-library'))
            self.assertNotEqual(pinned, library)
            self.assertEqual(pinned.read_bytes(), library.read_bytes())
            self.assertEqual(pinned.stat().st_mode & 0o777, 0o400)
            self.assertFalse(output.exists())
            return self.child(command, **options)

        self.assertEqual(self.invoke(child, window_session_output=output,
                                     window_session_present_library=library), 0)
        self.assertFalse(self.receipt()['performance_qualified'])

    def test_presentation_library_tampering_preserves_previous_pointer(self):
        pointer, previous = self.previous()
        library = self.workspace / 'presentation.dylib'
        library.write_bytes(b'fake library')
        output = self.workspace / 'cache/window-observation'

        def child(command, **options):
            Path(self.option(command, '--window-session-present-library')).unlink()
            return self.child(command, **options)

        self.assertEqual(self.invoke(child, window_session_output=output,
                                     window_session_present_library=library), 1)
        self.assertEqual(pointer.read_bytes(), previous)

    def test_native_output_cannot_overlap_selected_input_or_binary(self):
        self.sessions.mkdir(parents=True)
        selected = self.write_json(self.workspace / 'cache/selected.json', saved(play_alpha.HOUSEHOLDS['newbie']))
        binary = self.workspace / 'cache/source-binary'
        self.binary.rename(binary)
        self.binary = binary
        for output in (selected, selected / 'child', binary, binary / 'child', self.workspace / 'cache'):
            with self.subTest(output=output):
                self.assertEqual(self.invoke(load=selected, window_session_output=output), 1)
                self.last_child.assert_not_called()

    def test_symlinked_binary_or_input_parent_prevents_launch(self):
        alias = self.workspace / 'binary-alias'
        alias.symlink_to(self.binary)
        original = self.binary
        self.binary = alias
        self.assertEqual(self.invoke(), 1)
        self.last_child.assert_not_called()
        self.binary = original
        selected = self.write_json(self.workspace / 'external-input/selected.json', saved(play_alpha.HOUSEHOLDS['newbie']))
        directory = self.workspace / 'input-alias'
        directory.symlink_to(selected.parent, target_is_directory=True)
        self.assertEqual(self.invoke(load=directory / selected.name), 1)
        self.last_child.assert_not_called()

    def test_directory_input_is_not_treated_as_a_save_or_executable(self):
        self.assertEqual(self.invoke(load=self.installation), 1)
        self.last_child.assert_not_called()
        self.binary = self.installation
        self.assertEqual(self.invoke(), 1)
        self.last_child.assert_not_called()

    def test_native_output_rejects_existing_protected_outside_and_traversal_paths(self):
        self.sessions.mkdir(parents=True)
        existing = self.workspace / 'cache/existing'
        existing.mkdir()
        alias = self.workspace / 'cache/alias'
        alias.symlink_to(self.installation, target_is_directory=True)
        outputs = (existing, self.installation / 'output', self.workspace / 'outside',
                   self.sessions / 'output', self.workspace / 'cache/../output', alias / 'output',
                   self.binary, self.workspace / 'cache/missing-parent/output')
        for output in outputs:
            with self.subTest(output=output):
                self.assertEqual(self.invoke(window_session_output=output), 1)
                self.last_child.assert_not_called()

    def test_native_library_requires_output_and_cannot_collide_with_output(self):
        library = self.workspace / 'cache/presentation.dylib'
        library.parent.mkdir()
        library.write_bytes(b'fake library')
        self.assertEqual(self.invoke(window_session_present_library=library), 1)
        self.last_child.assert_not_called()
        self.assertEqual(self.invoke(window_session_output=library,
                                     window_session_present_library=library), 1)
        self.last_child.assert_not_called()

    def test_cli_modes_are_mutually_exclusive(self):
        for arguments in (['--household', 'goth', '--continue'], ['--load', 'selected.json', '--continue'],
                          ['--household', 'newbie', '--load', 'selected.json']):
            with self.subTest(arguments=arguments), mock.patch.object(play_alpha, 'launch') as launch, \
                    mock.patch.object(play_alpha.sys, 'argv', ['play_alpha.py', *arguments]), \
                    contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error:
                    play_alpha.main()
                self.assertEqual(error.exception.code, 2)
                launch.assert_not_called()

    def test_cli_passes_explicit_paths_timeout_and_native_flags(self):
        arguments = ['play_alpha.py', '--binary', str(self.binary), '--root', str(self.installation),
                     '--continue', '--timeout', '5', '--window-session-output', 'cache/observation',
                     '--window-session-present-library', 'fake.dylib']
        with mock.patch.object(play_alpha.sys, 'argv', arguments), \
                mock.patch.object(play_alpha, 'launch', return_value=19) as launch:
            self.assertEqual(play_alpha.main(), 19)
        self.assertEqual(launch.call_args.args, (self.binary, self.installation, None, None, True))
        self.assertEqual(launch.call_args.kwargs['timeout'], 5)
        self.assertEqual(launch.call_args.kwargs['window_session_output'], 'cache/observation')

    def test_cli_validation_errors_return_failure(self):
        with mock.patch.object(play_alpha.sys, 'argv', ['play_alpha.py']), \
                mock.patch.object(play_alpha, 'launch', side_effect=ValueError('strict failure')), \
                contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(play_alpha.main(), 1)

    def test_direct_api_rejects_conflicting_modes_and_invalid_timeouts(self):
        for arguments in ({'household': 'newbie', 'resume': True}, {'household': 'unknown'},
                          {'timeout': 0}, {'timeout': -1}, {'timeout': float('nan')}, {'timeout': float('inf')}):
            with self.subTest(arguments=arguments), self.assertRaises(ValueError):
                self.invoke(**arguments)


    def test_opt_in_runner_gets_staged_command_and_default_does_not_run(self):
        captured = []

        def runner(command, **options):
            captured.append((command, options))
            self.assertEqual(options['cwd'], self.workspace)
            self.assertFalse(options['check'])
            self.assertEqual(options['timeout'], 20)
            receipt = options['launch_receipt']
            self.assertEqual(receipt['process']['status'], 'launching')
            self.assertEqual(Path(command[0]).read_bytes(), self.binary.read_bytes())
            self.assertTrue(play_alpha.check_inputs(receipt))
            result = self.child(command)
            result.diagnostic = {'status': 'exited', 'exit_code': 0, 'pid': 8123}
            return result

        self.assertEqual(self.invoke(child_runner=runner, timeout=20), 0)
        self.last_child.assert_not_called()
        self.assertEqual(len(captured), 1)
        receipt = self.receipt(captured[0][0])
        self.assertEqual(receipt['diagnostic']['pid'], 8123)
        self.assertTrue(receipt['continue_candidate'])

    def test_failed_opt_in_runner_keeps_actual_returncode_and_previous_continue(self):
        pointer, previous = self.previous()

        def runner(command, **options):
            self.commands.append(command)
            result = self.child(command)
            result.diagnostic = {'status': 'timeout', 'exit_code': 124,
                                 'cleanup': [{'operation': 'terminate', 'error': 'mock'}]}
            return result

        self.assertEqual(self.invoke(child_runner=runner), 124)
        receipt = self.receipt()
        self.assertEqual(receipt['process'], {'status': 'timeout', 'returncode': 0})
        self.assertEqual(receipt['exit_code'], 0)
        self.assertFalse(receipt['continue_candidate'])
        self.assertEqual(pointer.read_bytes(), previous)

    def test_opt_in_prelaunch_rejection_still_finalizes_launcher_pins(self):
        def runner(command, **options):
            raise ValueError('mock Strict validation refused before application launch')

        self.assertEqual(self.invoke(child_runner=runner), 1)
        self.last_child.assert_not_called()
        receipt = self.receipt()
        self.assertTrue(receipt['inputs_unchanged'])
        self.assertFalse(receipt['continue_candidate'])


    def prepare(self, **arguments):
        return play_alpha.prepare_launch(self.binary, self.installation, **arguments)

    def completion(self, prepared, status='exited', returncode=0, **changes):
        values = dict(argv=prepared.argv, cwd=prepared.cwd, status=status, returncode=returncode)
        values.update(changes)
        return FabricatedCompletion(play_alpha.CompletionObservation(**values))

    def finalize(self, prepared, completion=None):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return play_alpha.finalize_launch(prepared, completion or self.completion(prepared))

    def admit(self, prepared):
        return play_alpha.prepared_command(prepared)

    def prepared_save(self, prepared, source=None):
        return self.write_json(prepared.save_output, saved(source or self.option(prepared.argv, '--original')))

    def test_phase_prepare_continue_is_child_free_and_uses_actual_last_save(self):
        pointer, previous = self.previous()
        identity = json.loads(previous)
        selected = Path(identity['save'])
        self.write_json(self.workspace / 'decoy.json', saved(play_alpha.HOUSEHOLDS['goth']))
        with mock.patch.object(play_alpha.subprocess, 'run') as run, \
                mock.patch.object(play_alpha.subprocess, 'Popen') as popen, \
                mock.patch.object(play_alpha, 'last_save', wraps=play_alpha.last_save) as select:
            prepared = self.prepare(resume=True)
            command = self.admit(prepared)
        run.assert_not_called()
        popen.assert_not_called()
        select.assert_called_once_with(self.sessions, self.installation)
        receipt = play_alpha.read_json(prepared.receipt_path)
        self.assertEqual(receipt['command'], command)
        self.assertEqual(receipt['cwd'], str(prepared.cwd))
        self.assertEqual(receipt['process'], {'status': 'launching', 'returncode': None})
        self.assertEqual(receipt['input'], str(selected))
        self.assertEqual(receipt['continue_identity'], {'sha256': identity['sha256'], 'source': identity['source']})
        self.assertEqual(receipt['input_sha256'], identity['sha256'])
        self.assertEqual(command, [str(prepared.receipt_path.parent / 'sims-room'),
                                  '--original', identity['source'], '--root', str(self.installation),
                                  '--save', str(prepared.save_output), '--paused',
                                  '--load', str(prepared.receipt_path.parent / 'starting-save.json')])
        pinned = Path(self.option(command, '--load'))
        self.assertEqual(pinned.read_bytes(), selected.read_bytes())
        self.assertNotEqual(pinned.stat().st_ino, selected.stat().st_ino)
        self.assertEqual(pinned.stat().st_mode & 0o777, 0o400)
        self.assertEqual(Path(command[0]).stat().st_mode & 0o777, 0o500)
        self.assertFalse(prepared.save_output.exists())
        self.assertEqual(pointer.read_bytes(), previous)

    def test_phase_prepared_command_is_a_copy_of_the_immutable_pinned_argv(self):
        prepared = self.prepare()
        command = self.admit(prepared)
        command[0] = str(self.binary)
        command.extend(['--extra'])
        self.assertEqual(self.admit(prepared), list(prepared.argv))
        self.assertNotEqual(prepared.argv[0], str(self.binary))
        with self.assertRaises(AttributeError):
            prepared.argv = tuple(command)
        cloned = play_alpha.PreparedLaunch(prepared.argv, prepared.cwd, prepared.receipt_path, prepared.save_output)
        with self.assertRaisesRegex(ValueError, 'original in-process'):
            self.admit(cloned)

    def test_phase_preparation_requires_verified_continue_and_fresh_runs(self):
        with self.assertRaises(play_alpha.LaunchPreparationError):
            self.prepare(resume=True)
        first, second = self.prepare(), self.prepare()
        self.assertNotEqual(first.save_output.parent, second.save_output.parent)
        self.assertFalse((self.sessions / 'last-session.json').exists())

    def test_phase_revalidation_rejects_source_and_pinned_input_drift_without_child(self):
        for kind in ('source_binary', 'pinned_binary', 'source_save', 'pinned_save'):
            for operation in ('mutate', 'delete'):
                with self.subTest(kind=kind, operation=operation):
                    self.binary.write_bytes(b'fresh fake executable')
                    self.binary.chmod(0o700)
                    selected = self.write_json(self.workspace / 'selected.json', saved(play_alpha.HOUSEHOLDS['newbie']))
                    prepared = self.prepare(load=selected)
                    target = {'source_binary': self.binary, 'pinned_binary': Path(prepared.argv[0]),
                              'source_save': selected, 'pinned_save': Path(self.option(prepared.argv, '--load'))}[kind]
                    target.chmod(0o700)
                    if operation == 'delete':
                        target.unlink()
                    else:
                        target.write_bytes(b'changed before admission')
                    with mock.patch.object(play_alpha.subprocess, 'run') as child, \
                            self.assertRaisesRegex(ValueError, 'input changed'):
                        self.admit(prepared)
                    child.assert_not_called()
                    self.assertFalse((self.sessions / 'last-session.json').exists())

    def test_phase_revalidation_rejects_presentation_and_build_receipt_drift(self):
        package = self.package_binary()
        package_bytes = package.read_bytes()
        library = self.workspace / 'presentation.dylib'
        for kind in ('build_receipt', 'presentation_library'):
            for location in ('source', 'pinned'):
                with self.subTest(kind=kind, location=location):
                    package.write_bytes(package_bytes)
                    library.write_bytes(b'fake presentation library')
                    prepared = self.prepare(window_session_output=self.workspace / 'cache/window-observation',
                                            window_session_present_library=library)
                    receipt = play_alpha.read_json(prepared.receipt_path)
                    item = next(item for item in receipt['inputs'] if item['kind'] == kind)
                    target = Path(item[location])
                    target.chmod(0o600)
                    target.write_bytes(b'changed')
                    with self.assertRaisesRegex(ValueError, 'input changed'):
                        self.admit(prepared)

    def test_phase_revalidation_requires_fresh_save_and_window_outputs(self):
        for kind in ('save', 'broken_save_symlink', 'window'):
            with self.subTest(kind=kind):
                window = self.workspace / ('cache/window-' + kind)
                prepared = self.prepare(window_session_output=window)
                target = window if kind == 'window' else prepared.save_output
                if kind == 'broken_save_symlink':
                    target.symlink_to(self.workspace / 'missing')
                else:
                    target.write_bytes(b'preexisting')
                with self.assertRaises(ValueError):
                    self.admit(prepared)
                self.assertFalse((self.sessions / 'last-session.json').exists())

    def test_phase_revalidation_and_finalization_reject_mutated_plan_fields(self):
        pointer, previous = self.previous()
        for phase in ('admission', 'finalization'):
            for field in ('argv', 'cwd', 'receipt_path', 'save_output'):
                with self.subTest(phase=phase, field=field):
                    prepared = self.prepare()
                    if phase == 'finalization':
                        self.admit(prepared)
                        self.prepared_save(prepared)
                    replacement = ((*prepared.argv, '--extra') if field == 'argv'
                                   else self.workspace / ('changed-' + field))
                    # Deliberately bypass frozen=True to exercise the private
                    # plan snapshot, not merely the dataclass's assignment guard.
                    object.__setattr__(prepared, field, replacement)
                    with self.assertRaisesRegex(ValueError, 'identity changed'):
                        self.admit(prepared) if phase == 'admission' else self.finalize(prepared)
                    self.assertEqual(pointer.read_bytes(), previous)

    def test_phase_receipt_mutation_or_replacement_rejected_before_admission_or_finalization(self):
        pointer, previous = self.previous()
        for phase in ('admission', 'finalization'):
            for field in ('command', 'cwd', 'source', 'save_output', 'same_bytes_new_inode'):
                with self.subTest(phase=phase, field=field):
                    prepared = self.prepare()
                    if phase == 'finalization':
                        self.admit(prepared)
                        self.prepared_save(prepared)
                    if field == 'same_bytes_new_inode':
                        data = prepared.receipt_path.read_bytes()
                        prepared.receipt_path.unlink()
                        prepared.receipt_path.write_bytes(data)
                    else:
                        receipt = play_alpha.read_json(prepared.receipt_path)
                        receipt[field] = ['substituted'] if field == 'command' else 'substituted'
                        self.write_json(prepared.receipt_path, receipt)
                    with self.assertRaisesRegex(ValueError, 'receipt changed'):
                        self.admit(prepared) if phase == 'admission' else self.finalize(prepared)
                    self.assertEqual(pointer.read_bytes(), previous)

    def test_phase_finalization_requires_admission_and_explicit_trusted_protocol(self):
        prepared = self.prepare()
        with self.assertRaisesRegex(ValueError, 'Revalidate'):
            self.finalize(prepared)
        self.admit(prepared)
        for completion in (0, {'status': 'exited', 'returncode': 0},
                           subprocess.CompletedProcess(prepared.argv, 0),
                           play_alpha.CompletionObservation(prepared.argv, prepared.cwd, 'exited', 0)):
            with self.subTest(completion=completion), self.assertRaises(TypeError):
                play_alpha.finalize_launch(prepared, completion)
        self.assertFalse((self.sessions / 'last-session.json').exists())

    def test_phase_completion_binds_exact_pinned_argv_cwd_and_strict_status_types(self):
        pointer, previous = self.previous()
        prepared = self.prepare()
        self.admit(prepared)
        self.prepared_save(prepared)
        changes = ({'argv': (str(self.binary), *prepared.argv[1:])},
                   {'argv': list(prepared.argv)}, {'cwd': self.workspace / 'elsewhere'},
                   {'status': 'success'}, {'returncode': True})
        for change in changes:
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.finalize(prepared, self.completion(prepared, **change))
            self.assertEqual(pointer.read_bytes(), previous)
        with self.assertRaises(ValueError):
            self.finalize(prepared, FabricatedCompletion({'status': 'exited', 'returncode': 0}))

    def test_phase_unresolved_errors_signals_and_nonzero_never_publish_even_with_save(self):
        pointer, previous = self.previous()
        cases = [('unresolved', None, 1), ('unresolved', 0, 1), ('exited', None, 1),
                 ('exited', 23, 23), ('exited', -9, -9), ('timeout', None, 124),
                 ('launch_error', None, 1), ('diagnostic_failed', 0, 1), ('interrupted', None, 130)]
        for status, returncode, expected in cases:
            with self.subTest(status=status, returncode=returncode):
                prepared = self.prepare()
                self.admit(prepared)
                self.prepared_save(prepared)
                self.assertEqual(self.finalize(prepared, self.completion(prepared, status, returncode)), expected)
                receipt = play_alpha.read_json(prepared.receipt_path)
                self.assertEqual(receipt['process'], {'status': status, 'returncode': returncode})
                self.assertEqual(receipt['exit_code'], returncode)
                self.assertFalse(receipt['continue_candidate'])
                self.assertEqual(pointer.read_bytes(), previous)
                with self.assertRaisesRegex(ValueError, 'already been finalized'):
                    self.finalize(prepared)

    def test_phase_input_drift_at_finalization_preserves_pointer_and_actual_completion(self):
        pointer, previous = self.previous()
        for kind in ('source_binary', 'pinned_binary', 'source_save', 'pinned_save'):
            with self.subTest(kind=kind):
                self.binary.write_bytes(b'fresh fake executable')
                self.binary.chmod(0o700)
                selected = self.write_json(self.workspace / 'selected.json', saved(play_alpha.HOUSEHOLDS['newbie']))
                prepared = self.prepare(load=selected)
                self.admit(prepared)
                self.prepared_save(prepared)
                target = {'source_binary': self.binary, 'pinned_binary': Path(prepared.argv[0]),
                          'source_save': selected, 'pinned_save': Path(self.option(prepared.argv, '--load'))}[kind]
                target.chmod(0o600)
                target.write_bytes(b'changed after admission')
                self.assertEqual(self.finalize(prepared), 1)
                receipt = play_alpha.read_json(prepared.receipt_path)
                self.assertEqual(receipt['exit_code'], 0)
                self.assertFalse(receipt['inputs_unchanged'])
                self.assertFalse(receipt['continue_candidate'])
                self.assertEqual(pointer.read_bytes(), previous)

    def test_phase_malformed_wrong_source_symlink_hardlink_and_directory_outputs_rejected(self):
        pointer, previous = self.previous()
        external = self.write_json(self.workspace / 'external-save.json', saved(play_alpha.HOUSEHOLDS['newbie']))
        external_before = external.read_bytes()
        for kind in ('malformed', 'wrong_source', 'symlink', 'hardlink', 'directory'):
            with self.subTest(kind=kind):
                prepared = self.prepare()
                self.admit(prepared)
                if kind == 'malformed':
                    prepared.save_output.write_bytes(b'{')
                elif kind == 'wrong_source':
                    self.prepared_save(prepared, play_alpha.HOUSEHOLDS['goth'])
                elif kind == 'symlink':
                    prepared.save_output.symlink_to(external)
                elif kind == 'hardlink':
                    os.link(external, prepared.save_output)
                else:
                    prepared.save_output.mkdir()
                self.assertEqual(self.finalize(prepared), 1)
                self.assertIn('save_error', play_alpha.read_json(prepared.receipt_path))
                self.assertEqual(pointer.read_bytes(), previous)
                self.assertEqual(external.read_bytes(), external_before)
                if kind == 'hardlink':
                    prepared.save_output.unlink()

    def test_phase_receipt_is_retained_before_atomic_continue_publication_and_repeat_rejected(self):
        pointer, previous = self.previous()
        prepared = self.prepare(resume=True)
        self.admit(prepared)
        self.prepared_save(prepared)
        write = play_alpha.atomic_json
        writes = []

        def retain(path, value):
            writes.append(path)
            self.assertEqual(pointer.read_bytes(), previous)
            if path == pointer:
                receipt = play_alpha.read_json(prepared.receipt_path)
                self.assertEqual(receipt['command'], list(prepared.argv))
                self.assertEqual(receipt['cwd'], str(prepared.cwd))
                self.assertTrue(receipt['continue_candidate'])
                self.assertEqual(receipt['process'], {'status': 'exited', 'returncode': 0})
                self.assertEqual(receipt['save_sha256'], value['sha256'])
            return write(path, value)

        with mock.patch.object(play_alpha, 'atomic_json', side_effect=retain):
            self.assertEqual(self.finalize(prepared), 0)
        self.assertEqual(writes, [prepared.receipt_path, pointer])
        published = pointer.read_bytes()
        self.assertNotEqual(published, previous)
        self.assertEqual(play_alpha.last_save(self.sessions, self.installation)[0], prepared.save_output)
        with self.assertRaisesRegex(ValueError, 'already been finalized'):
            self.finalize(prepared)
        self.assertEqual(pointer.read_bytes(), published)

    def test_phase_final_receipt_failure_preserves_pointer_and_consumes_finalization(self):
        pointer, previous = self.previous()
        prepared = self.prepare()
        self.admit(prepared)
        self.prepared_save(prepared)
        launching = prepared.receipt_path.read_bytes()
        with mock.patch.object(play_alpha, 'atomic_json', side_effect=OSError('final receipt refused')):
            self.assertEqual(self.finalize(prepared), 1)
        self.assertEqual(pointer.read_bytes(), previous)
        self.assertEqual(prepared.receipt_path.read_bytes(), launching)
        with self.assertRaisesRegex(ValueError, 'already been finalized'):
            self.finalize(prepared)

    def test_phase_receipt_drift_during_completion_observation_rejected(self):
        pointer, previous = self.previous()
        prepared = self.prepare()
        self.admit(prepared)
        self.prepared_save(prepared)
        completion = self.completion(prepared)
        observation = completion.observe()

        def observe():
            prepared.receipt_path.write_bytes(b'changed by observation')
            return observation

        with mock.patch.object(completion, 'observe', side_effect=observe), \
                self.assertRaisesRegex(ValueError, 'receipt changed'):
            self.finalize(prepared, completion)
        self.assertEqual(pointer.read_bytes(), previous)

    def test_phase_mutation_at_final_receipt_boundary_cannot_publish(self):
        pointer, previous = self.previous()
        write = play_alpha.atomic_json
        for kind in ('receipt', 'prepared_argv', 'output'):
            with self.subTest(kind=kind):
                prepared = self.prepare()
                self.admit(prepared)
                self.prepared_save(prepared)

                def retain(path, value):
                    write(path, value)
                    if path == prepared.receipt_path and value.get('continue_candidate'):
                        if kind == 'receipt':
                            path.write_bytes(b'changed final receipt')
                        elif kind == 'prepared_argv':
                            object.__setattr__(prepared, 'argv', (*prepared.argv, '--substituted'))
                        else:
                            prepared.save_output.write_bytes(b'changed final output')

                with mock.patch.object(play_alpha, 'atomic_json', side_effect=retain):
                    self.assertEqual(self.finalize(prepared), 1)
                self.assertEqual(pointer.read_bytes(), previous)
                self.assertIn('publication_error', play_alpha.read_json(prepared.receipt_path))

    def test_phase_concurrent_and_reentrant_finalizers_cannot_observe_or_publish_twice(self):
        prepared = self.prepare()
        self.admit(prepared)
        self.prepared_save(prepared)
        completion = self.completion(prepared)
        observation = completion.observe()
        entered, release = threading.Event(), threading.Event()
        outcomes, failures, writes = [], [], []
        write = play_alpha.atomic_json

        def observe():
            entered.set()
            if not release.wait(5):
                raise AssertionError('test did not release the authoritative observer')
            # A callback must not recursively consume the same plan either.
            with self.assertRaisesRegex(ValueError, 'already being finalized'):
                self.finalize(prepared)
            return observation

        def finalize_first():
            try:
                outcomes.append(self.finalize(prepared, completion))
            except BaseException as error:
                failures.append(error)

        def retain(path, value):
            writes.append(path)
            return write(path, value)

        with mock.patch.object(completion, 'observe', side_effect=observe), \
                mock.patch.object(play_alpha, 'atomic_json', side_effect=retain):
            first = threading.Thread(target=finalize_first)
            first.start()
            try:
                self.assertTrue(entered.wait(5))
                contender = self.completion(prepared)
                with mock.patch.object(contender, 'observe', wraps=contender.observe) as observer:
                    with self.assertRaisesRegex(ValueError, 'already being finalized'):
                        self.finalize(prepared, contender)
                    observer.assert_not_called()
            finally:
                release.set()
                first.join(5)
            self.assertFalse(first.is_alive())
        self.assertEqual(failures, [])
        self.assertEqual(outcomes, [0])
        self.assertEqual(writes.count(self.sessions / 'last-session.json'), 1)
        with self.assertRaisesRegex(ValueError, 'already been finalized'):
            self.finalize(prepared)

    def test_phase_cli_has_no_serialized_finalizer_or_exit_code_attestation(self):
        for arguments in (['--exit-code', '0'], ['--finalize', 'launch.json'],
                          ['--completion', 'result.json'], ['--prepare']):
            with self.subTest(arguments=arguments), mock.patch.object(play_alpha.sys, 'argv', ['play_alpha.py', *arguments]), \
                    mock.patch.object(play_alpha, 'launch') as launch, contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error:
                    play_alpha.main()
                self.assertEqual(error.exception.code, 2)
                launch.assert_not_called()


if __name__ == '__main__':
    unittest.main()
