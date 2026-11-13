#!/usr/bin/env python3
"""Asset-free mocked tests; the audit hook rejects accidental host operations.

Run directly with Python's standard library only. No root test module is
imported, and no compiler, application, native framework, or dylib is invoked.
"""

import sys


def audit_guard(event, arguments):
    forbidden = (
        'subprocess.', 'socket.', 'ctypes.', 'os.exec', 'os.spawn',
        'os.posix_spawn', 'os.fork', 'os.kill', 'os.wait', 'os.system', 'pty.spawn',
    )
    if event.startswith(forbidden):
        raise RuntimeError('Audit guard blocked ' + event)
    if event == 'import':
        module = arguments[0].split('.')[0]
        if module[:1].isupper() or module in {'ctypes', '_ctypes', 'objc', 'cffi', '_cffi_backend', 'rubicon'}:
            raise RuntimeError('Audit guard blocked native import ' + module)


sys.addaudithook(audit_guard)

import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import select
from types import SimpleNamespace
import unittest
from unittest import mock


HOST_GUARD = contextlib.ExitStack()
for boundary, names in ((os, ('waitid', 'waitpid', 'wait', 'wait3', 'wait4', 'getuid')),
                        (select, ('select', 'kqueue', 'kevent'))):
    for name in names:
        if hasattr(boundary, name):
            HOST_GUARD.enter_context(mock.patch.object(
                boundary, name, side_effect=RuntimeError('Unmocked host boundary: ' + name)))


sys.path.insert(0, str(Path(__file__).absolute().parent))
import build_window_presentation as build
import window_process_group as groups


ACTUAL_INSPECT_GROUP = groups.inspect_group
ACTUAL_BACKEND_PLAN = groups.backend_plan
SOURCE_REPOSITORY = Path(__file__).absolute().parent.parent
PRODUCTION_SOURCE_HASHES = dict(build.SOURCE_HASHES)
NATIVE_BYTES = {name: (SOURCE_REPOSITORY / name).read_bytes() for name in build.SOURCE_HASHES}
FIXTURE_SOURCE_HASHES = {name: hashlib.sha256(data).hexdigest()
                         for name, data in NATIVE_BYTES.items()}


def mock_kqueue_backend():
    return SimpleNamespace(
        kqueue=mock.Mock(side_effect=AssertionError('Unexpected host kqueue')),
        kevent=mock.Mock(), KQ_FILTER_PROC=-5, KQ_EV_ADD=1, KQ_EV_ONESHOT=16,
        KQ_NOTE_EXIT=0x80000000, KQ_EV_ERROR=16384,
    )


class AuthoredClock:
    def __init__(self):
        self.now = 0

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class AuthoredChild:
    def __init__(self, returncode=0):
        self.pid = 24680
        self.args = ['authored-mock']
        self.returncode = None
        self.exit_code = None
        self.planned_returncode = returncode
        self.descendants = {}
        self.history = []
        self.wait = mock.Mock(side_effect=self.reap)
        self.poll = mock.Mock(side_effect=AssertionError('Observation must not poll/reap'))

    def observation(self):
        self.history.append('waitid')
        if self.returncode is not None:
            raise ChildProcessError('Authored child is no longer waitable')
        if self.exit_code is None:
            return None
        return SimpleNamespace(si_pid=self.pid, si_uid=os.getuid(),
                               si_code=os.CLD_EXITED if self.exit_code >= 0 else os.CLD_KILLED,
                               si_status=abs(self.exit_code))

    def reap(self, timeout):
        self.history.append('reap')
        if self.exit_code is None:
            raise build.subprocess.TimeoutExpired(self.args, timeout)
        self.returncode = self.exit_code
        return self.returncode

    def rows(self):
        state = 'Z' if self.exit_code is not None else 'S'
        rows = [(self.pid, state)] + sorted(self.descendants.items())
        return ''.join(f'{pid} {self.pid} {os.getuid()} {status}\n' for pid, status in rows)


def authored_evidence(output):
    return {'stdout': output, 'stderr': '', 'errors': [], 'auxiliary_errors': [],
            'reaped': True, 'returncode': 0, 'truncated': False}


class BuildTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='window-presentation-mock-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.compiler = self.root / 'mock-clang'
        self.compiler.write_bytes(b'Authored fake compiler; this file is never executed.\n')
        self.compiler.chmod(0o500)
        self.sdk = self.root / 'authored-sdk'
        self.sdk.mkdir(mode=0o755)
        self.sdk_settings = self.sdk / 'SDKSettings.json'
        self.sdk_settings.write_text('{"scope":"authored mocked SDK metadata only"}\n')
        for name, data in NATIVE_BYTES.items():
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        (self.root / 'scripts').mkdir()
        (self.root / '.git').write_text('Synthetic non-Git fixture\n')
        self.output = self.root / 'cache' / 'fresh-build'
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(mock.patch.object(os, 'getuid', return_value=self.root.stat().st_uid))
        self.stack.enter_context(mock.patch.object(build, 'REPOSITORY', self.root))
        self.stack.enter_context(mock.patch.object(build, 'SOURCE_HASHES', FIXTURE_SOURCE_HASHES))
        self.stack.enter_context(mock.patch.object(build.play_alpha, 'REPOSITORY', self.root))
        self.stack.enter_context(mock.patch.object(build.sys, 'platform', 'darwin'))
        self.stack.enter_context(mock.patch.object(build, 'DEFAULT_SDK', self.sdk, create=True))
        self.disk = self.stack.enter_context(mock.patch.object(
            build.shutil, 'disk_usage', return_value=SimpleNamespace(free=build.RESERVE_BYTES)))
        self.spawn = self.stack.enter_context(mock.patch.object(
            build.subprocess, 'Popen', side_effect=AssertionError('Unexpected compiler invocation')))
        self.signals = self.stack.enter_context(mock.patch.object(
            build.os, 'killpg', side_effect=self.signal_group))
        self.stack.enter_context(mock.patch.object(build.signal, 'getsignal', return_value=build.signal.SIG_DFL))
        self.stack.enter_context(mock.patch.object(build, 'select', mock_kqueue_backend()))
        self.nonreaping_wait = self.stack.enter_context(mock.patch.object(
            build, 'wait_without_reaping', return_value=None))
        self.clock = AuthoredClock()
        self.stack.enter_context(mock.patch.object(groups, 'time', self.clock))
        self.stack.enter_context(mock.patch.object(groups, 'CENSUS_WITNESSED', True))
        self.stack.enter_context(mock.patch.object(groups.threading, 'active_count', return_value=1))
        for name, value in {'P_PID': 1, 'WEXITED': 4, 'WNOWAIT': 32, 'WNOHANG': 1,
                            'CLD_EXITED': 1, 'CLD_KILLED': 2, 'CLD_DUMPED': 3}.items():
            self.stack.enter_context(mock.patch.object(os, name, value, create=True))
        self.waitid = self.stack.enter_context(mock.patch.object(
            os, 'waitid', side_effect=lambda *args: self.child.observation(), create=True))
        self.backend = groups.backend_plan(lambda *args, **kwargs: {
            'uid': 0, 'mode': stat.S_IFREG | 0o555, 'sha256': 'authored-ps-never-executed'})
        self.stack.enter_context(mock.patch.object(groups, 'backend_plan', return_value=self.backend))
        self.inspector = self.stack.enter_context(mock.patch.object(
            groups, 'inspect_group', side_effect=lambda *args: authored_evidence(self.child.rows())))
        groups.RETAINED_GROUPS.clear()
        groups.RETAINED_GROUP_HANDLES.clear()
        groups.RETAINED_AUXILIARIES.clear()
        self.addCleanup(groups.RETAINED_GROUPS.clear)
        self.addCleanup(groups.RETAINED_GROUP_HANDLES.clear)
        self.addCleanup(groups.RETAINED_AUXILIARIES.clear)

    def signal_group(self, pgid, signum):
        self.assertEqual(pgid, self.child.pid)
        self.assertIsNone(self.child.returncode, 'Post-reap group signal')
        self.child.history.append(('signal', signum))
        if self.child.exit_code is None:
            self.child.exit_code = -signum
        if signum == build.signal.SIGKILL:
            self.child.descendants = {pid: 'Z' for pid in self.child.descendants}

    def plan(self, **changes):
        arguments = {'output': self.output, 'compiler': self.compiler, 'timeout': 120}
        arguments.update(changes)
        return build.plan_build(**arguments)

    def simulated_compiler(self, returncode=0, observation_error=None, error_exit_code=None,
                           mutation=None, version=True, product=True):
        process = AuthoredChild(returncode)
        self.child = process

        def observe(unused_process, unused_timeout):
            process.history.append('exit_observation')
            if observation_error is not None:
                if not isinstance(observation_error, build.subprocess.TimeoutExpired):
                    process.exit_code = returncode if error_exit_code is None else error_exit_code
                raise observation_error
            process.exit_code = returncode

        self.nonreaping_wait.side_effect = observe

        def start(command, **kwargs):
            kwargs['stdout'].write(b'Authored mock compiler stdout\n')
            diagnostic = b'Apple clang version 99.0.0 (authored mock, not a toolchain)\n' if version else b''
            kwargs['stderr'].write(diagnostic + b'Authored mock compiler diagnostic\n')
            if product:
                Path(command[-1]).write_bytes(b'Authored non-Mach-O fake product, never loaded.\n')
            if mutation:
                mutation(command)
            return process

        self.spawn.side_effect = start
        return process

    def execute(self, **simulation):
        plan = self.plan()
        process = self.simulated_compiler(**simulation)
        return build.execute_build(plan), process

    def test_default_cli_is_plan_only_without_writes_or_process_controls(self):
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            status = build.main(['--compiler', str(self.compiler)])
        result = json.loads(stdout.getvalue())
        self.assertEqual(status, 0)
        self.assertEqual(result['mode'], 'plan')
        self.assertFalse((self.root / 'cache').exists())
        self.assertFalse(any(result['qualifications'].values()))
        self.spawn.assert_not_called()
        self.signals.assert_not_called()

    def test_exact_plan_flags_environment_and_schema(self):
        plan = self.plan()
        self.assertEqual(set(plan), {
            'schema', 'kind', 'mode', 'scope', 'output_directory', 'output_path',
            'receipt_path', 'command', 'environment', 'timeout_seconds',
            'cleanup_timeout_seconds', 'cleanup_backend', 'minimum_free_bytes', 'compiler', 'sources', 'qualifications', 'sdk_root',
        })
        self.assertEqual(plan['command'], [
            str(self.compiler), '-v', '-std=c11', '-O2', '-fobjc-arc', '-fblocks',
            '-fvisibility=hidden', '-mmacosx-version-min=14.0', '-isysroot', str(self.sdk), '-dynamiclib',
            str(self.output / 'native/window_present_macos.m'), '-framework', 'Foundation',
            '-framework', 'QuartzCore', '-framework', 'Metal', '-framework', 'AppKit',
            '-o', str(self.output / 'window-present-macos.dylib'),
        ])
        self.assertEqual(plan['minimum_free_bytes'], 6 * 1024**3)
        self.assertEqual(plan['cleanup_timeout_seconds'], 2)
        self.assertEqual(plan['environment'], {
            'PATH': '/usr/bin:/bin', 'LANG': 'C', 'LC_ALL': 'C',
            'TMPDIR': str(self.output / 'tmp'), 'HOME': str(self.output / 'home'),
            'CLANG_MODULE_CACHE_PATH': str(self.output / 'module-cache'),
        })
        self.assertEqual(set(plan['compiler']), {'path', 'before'})
        self.assertEqual(plan['sdk_root'], str(self.sdk))
        self.assertEqual(len(plan['sources']), 4)
        self.assertEqual(plan['sources'][0]['sha256'], next(iter(build.SOURCE_HASHES.values())))

    def test_execute_is_explicit_and_starts_exactly_one_driver(self):
        self.simulated_compiler()
        with contextlib.redirect_stdout(io.StringIO()):
            status = build.main(['--compiler', str(self.compiler), '--output', str(self.output), '--execute'])
        self.assertEqual(status, 0)
        self.spawn.assert_called_once()
        command = self.spawn.call_args.args[0]
        options = self.spawn.call_args.kwargs
        self.assertEqual(command[0], str(self.compiler))
        self.assertFalse(options['shell'])
        self.assertTrue(options['start_new_session'])
        self.assertTrue(options['close_fds'])
        self.assertEqual(options['stdin'], build.subprocess.DEVNULL)
        self.assertEqual(options['cwd'], str(self.output))

    def test_write_once_receipt_binds_sources_command_compiler_version_and_product(self):
        receipt, process = self.execute()
        self.assertEqual(receipt['outcome'], 'built_unqualified')
        self.assertEqual(set(receipt), {
            'schema', 'kind', 'mode', 'scope', 'output_directory', 'output_path',
            'receipt_path', 'command', 'environment', 'timeout_seconds',
            'cleanup_timeout_seconds', 'cleanup_backend', 'minimum_free_bytes', 'compiler', 'sources', 'qualifications', 'sdk_root',
            'outcome', 'integrity_errors', 'process', 'cleanup', 'logs', 'output',
            'setup_error', 'receipt_write_error',
        })
        self.assertEqual(receipt['schema'], 1)
        self.assertEqual(receipt['kind'], 'window_presentation_source_build')
        self.assertEqual(receipt['process'], {
            'attempted': True, 'pid': 24680, 'returncode': 0, 'timed_out': False, 'error': None,
        })
        self.assertEqual(set(receipt['cleanup']), set(groups.empty_cleanup()))
        self.assertEqual(set(receipt['compiler']), {'path', 'before', 'after', 'version'})
        self.assertTrue(receipt['compiler']['version'].startswith('Apple clang version 99.0.0'))
        self.assertEqual(receipt['compiler']['before'], receipt['compiler']['after'])
        self.assertEqual(receipt['compiler']['before']['sha256'], hashlib.sha256(self.compiler.read_bytes()).hexdigest())
        for item in receipt['sources']:
            self.assertEqual(set(item), {'path', 'sha256', 'before', 'after', 'staged_path',
                                         'staged_before', 'staged_after'})
            self.assertEqual(item['before'], item['after'])
            self.assertEqual(item['sha256'], item['after']['sha256'])
            if item['staged_path']:
                self.assertEqual(item['staged_before'], item['staged_after'])
                self.assertEqual(item['sha256'], item['staged_after']['sha256'])
                self.assertEqual(Path(item['staged_path']).stat().st_mode & 0o777, 0o400)
        self.assertEqual(receipt['output']['state']['sha256'], hashlib.sha256(
            Path(receipt['output_path']).read_bytes()).hexdigest())
        self.assertEqual(set(receipt['logs']), {'stdout', 'stderr'})
        self.assertFalse(any(receipt['qualifications'].values()))
        receipt_path = Path(receipt['receipt_path'])
        self.assertEqual(json.loads(receipt_path.read_text()), receipt)
        self.assertEqual(receipt_path.stat().st_mode & 0o777, 0o400)
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o700)
        descriptor = os.open(self.output, os.O_RDONLY | os.O_DIRECTORY)
        try:
            with self.assertRaises(FileExistsError):
                build.write_new(descriptor, 'build-receipt.json', b'overwrite')
        finally:
            os.close(descriptor)
        self.assertEqual(process.wait.call_args_list, [mock.call(timeout=2)])
        self.assertEqual(self.nonreaping_wait.call_args_list, [mock.call(process, 120)])
        self.assertTrue(receipt['cleanup']['no_live_members'])
        self.assertEqual(len(receipt['cleanup']['censuses']), 2)
        self.signals.assert_not_called()

    def test_environment_never_inherits_dyld_or_other_caller_injection(self):
        with mock.patch.dict(os.environ, {'DYLD_INSERT_LIBRARIES': '/not-loaded',
                                         'DYLD_LIBRARY_PATH': '/not-used', 'SDKROOT': '/not-used',
                                         'CPATH': '/not-used', 'PATH': '/not-used'}):
            receipt, unused = self.execute()
        environment = self.spawn.call_args.kwargs['env']
        self.assertEqual(environment, receipt['environment'])
        self.assertFalse(any(key.startswith('DYLD') for key in environment))
        self.assertNotIn('SDKROOT', environment)
        self.assertNotIn('CPATH', environment)
        self.assertEqual(environment['PATH'], '/usr/bin:/bin')

    def test_wrong_host_rejects_plan_and_execution_without_spawn(self):
        with mock.patch.object(build.sys, 'platform', 'linux'):
            with self.assertRaisesRegex(ValueError, 'macOS-only'):
                self.plan()
        self.spawn.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_nondefault_child_handler_fails_closed_before_any_spawn(self):
        with mock.patch.object(build.signal, 'getsignal', return_value=build.signal.SIG_IGN):
            with self.assertRaisesRegex(ValueError, 'default SIGCHLD'):
                self.plan()
        self.spawn.assert_not_called()
        self.signals.assert_not_called()

    def test_missing_nonreaping_backend_fails_closed_before_any_spawn(self):
        del build.select.kqueue
        with self.assertRaisesRegex(ValueError, 'Non-reaping kqueue'):
            self.plan()
        self.spawn.assert_not_called()

    def test_low_disk_reserve_rejects_without_output(self):
        self.disk.return_value.free = build.RESERVE_BYTES - 1
        with self.assertRaisesRegex(ValueError, 'six GiB'):
            self.plan()
        self.spawn.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_disk_is_rechecked_immediately_before_compiler(self):
        plan = self.plan()
        self.disk.side_effect = [SimpleNamespace(free=build.RESERVE_BYTES),
                                 SimpleNamespace(free=build.RESERVE_BYTES), SimpleNamespace(free=0)]
        receipt = build.execute_build(plan)
        self.assertEqual(receipt['outcome'], 'setup_error')
        self.assertIn('six GiB', receipt['setup_error'])
        self.spawn.assert_not_called()

    def test_changed_pinned_source_rejects_before_creating_output(self):
        source = self.root / next(iter(build.SOURCE_HASHES))
        source.write_bytes(source.read_bytes() + b'changed\n')
        with self.assertRaisesRegex(ValueError, 'Source pin mismatch'):
            self.plan()
        self.spawn.assert_not_called()

    def test_sources_rechecked_between_plan_and_execution(self):
        plan = self.plan()
        source = self.root / list(build.SOURCE_HASHES)[1]
        source.write_bytes(source.read_bytes() + b'changed\n')
        with self.assertRaisesRegex(ValueError, 'Source pin mismatch'):
            build.execute_build(plan)
        self.spawn.assert_not_called()

    def test_tampered_plan_is_rejected(self):
        plan = self.plan()
        plan['command'].insert(1, '-unreviewed-flag')
        with self.assertRaisesRegex(ValueError, 'Plan or inputs changed'):
            build.execute_build(plan)
        self.spawn.assert_not_called()

    def test_changed_compiler_between_plan_and_execution(self):
        plan = self.plan()
        self.compiler.chmod(0o700)
        self.compiler.write_bytes(b'Changed authored fake compiler\n')
        with self.assertRaisesRegex(ValueError, 'Plan or inputs changed'):
            build.execute_build(plan)
        self.spawn.assert_not_called()

    def test_source_change_after_spawn_is_recorded(self):
        source = self.root / next(iter(build.SOURCE_HASHES))
        receipt, unused = self.execute(mutation=lambda command: source.write_bytes(b'changed source\n'))
        self.assertEqual(receipt['outcome'], 'integrity_failure')
        self.assertTrue(any('Source changed' in error for error in receipt['integrity_errors']))
        self.assertIsNone(receipt['sources'][0]['after'])

    def test_staged_source_change_after_spawn_is_recorded(self):
        def mutate(command):
            source = Path(next(argument for argument in command if argument.endswith('/native/window_present_macos.m')))
            source.chmod(0o600)
            source.write_bytes(b'changed staged source\n')
        receipt, unused = self.execute(mutation=mutate)
        self.assertEqual(receipt['outcome'], 'integrity_failure')
        self.assertTrue(any('Staged source changed' in error for error in receipt['integrity_errors']))

    def test_compiler_change_after_spawn_is_recorded(self):
        receipt, unused = self.execute(mutation=lambda command: self.compiler.chmod(0o700))
        self.assertEqual(receipt['outcome'], 'integrity_failure')
        self.assertIn('Compiler changed', receipt['integrity_errors'])

    def test_traversal_and_output_outside_cache_are_rejected(self):
        for output in (self.root / 'cache/../escape', self.root / 'other', self.root / 'cache'):
            with self.subTest(output=output), self.assertRaises(ValueError):
                self.plan(output=output)
        self.spawn.assert_not_called()

    def test_existing_output_is_never_reused(self):
        self.output.mkdir(parents=True, mode=0o700)
        with self.assertRaisesRegex(ValueError, 'fresh directory'):
            self.plan()

    def test_symlinked_cache_ancestor_is_rejected(self):
        real = self.root / 'real-cache'
        real.mkdir()
        (self.root / 'cache').symlink_to(real, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlinks'):
            self.plan()

    def test_dangling_output_symlink_is_rejected(self):
        self.output.parent.mkdir()
        self.output.symlink_to(self.root / 'absent')
        with self.assertRaisesRegex(ValueError, 'symlinks'):
            self.plan()

    def test_hardlinked_source_is_rejected(self):
        source = self.root / next(iter(build.SOURCE_HASHES))
        os.link(source, self.root / 'source-alias')
        with self.assertRaisesRegex(ValueError, 'non-linked regular'):
            self.plan()

    def test_symlinked_source_is_rejected(self):
        source = self.root / next(iter(build.SOURCE_HASHES))
        other = self.root / 'source-copy'
        source.rename(other)
        source.symlink_to(other)
        with self.assertRaisesRegex(ValueError, 'symlinks'):
            self.plan()

    def test_symlinked_source_ancestor_is_rejected(self):
        native = self.root / 'crates/sims-app/native'
        other = self.root / 'native-copy'
        native.rename(other)
        native.symlink_to(other, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlinks'):
            self.plan()

    def test_linked_or_nonexecutable_compiler_is_rejected(self):
        os.link(self.compiler, self.root / 'compiler-alias')
        with self.assertRaisesRegex(ValueError, 'non-linked regular'):
            self.plan()
        (self.root / 'compiler-alias').unlink()
        self.compiler.chmod(0o400)
        with self.assertRaisesRegex(ValueError, 'not executable'):
            self.plan()

    def test_relative_or_symlink_compiler_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'absolute'):
            self.plan(compiler='mock-clang')
        alias = self.root / 'compiler-symlink'
        alias.symlink_to(self.compiler)
        with self.assertRaisesRegex(ValueError, 'symlinks'):
            self.plan(compiler=alias)

    def test_explicit_output_overlap_is_rejected(self):
        self.output.parent.mkdir()
        protected = self.output / 'protected-input'
        with self.assertRaisesRegex(ValueError, 'overlaps'):
            build.validate_output(self.output, [protected])
        with self.assertRaisesRegex(ValueError, 'overlaps'):
            build.validate_output(self.output, [self.output.parent])

    def test_group_writable_cache_is_rejected(self):
        self.output.parent.mkdir()
        self.output.parent.chmod(0o770)
        with self.assertRaisesRegex(ValueError, 'group/world writable'):
            self.plan()

    def test_sdk_selection_is_explicit_and_ignores_inherited_sdkroot(self):
        with mock.patch.dict(os.environ, {'SDKROOT': '/authored-untrusted-inherited-root'}):
            plan = self.plan()
        self.assertEqual(plan['sdk_root'], str(self.sdk))
        self.assertEqual(plan['command'][plan['command'].index('-isysroot') + 1], str(self.sdk))
        self.assertEqual(plan['sources'][-1]['path'], str(self.sdk_settings))
        self.assertEqual(plan['sources'][-1]['sha256'], hashlib.sha256(self.sdk_settings.read_bytes()).hexdigest())
        self.assertNotIn('SDKROOT', plan['environment'])

    def test_sdk_metadata_change_before_execution_rejects_without_spawn(self):
        plan = self.plan()
        self.sdk_settings.write_text('{"scope":"authored changed metadata"}\n')
        with self.assertRaisesRegex(ValueError, 'changed before execution'):
            build.execute_build(plan)
        self.spawn.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_sdk_metadata_change_after_spawn_preserves_integrity_failure(self):
        receipt, unused = self.execute(mutation=lambda command: self.sdk_settings.write_text('changed'))
        self.assertEqual(receipt['outcome'], 'integrity_failure')
        self.assertTrue(any('Source changed' in error for error in receipt['integrity_errors']))

    def test_writable_sdk_root_is_rejected_without_spawn(self):
        self.sdk.chmod(0o777)
        with self.assertRaisesRegex(ValueError, 'SDK.*writable'):
            self.plan()
        self.spawn.assert_not_called()

    def test_sdk_selector_is_resolved_and_bound_before_plan(self):
        selector = self.root / 'sdk-selector'
        selector.symlink_to(self.sdk, target_is_directory=True)
        with mock.patch.object(build, 'DEFAULT_SDK', selector):
            plan = self.plan()
        self.assertEqual(plan['sdk_root'], str(self.sdk))
        self.assertNotIn(str(selector), plan['command'])

    def test_hardlinked_sdk_metadata_is_rejected_without_spawn(self):
        os.link(self.sdk_settings, self.root / 'metadata-alias')
        with self.assertRaisesRegex(ValueError, 'non-linked regular'):
            self.plan()
        self.spawn.assert_not_called()

    def test_nested_private_output_beneath_writable_ancestor_is_rejected(self):
        cache = self.output.parent
        cache.mkdir(mode=0o755)
        shared = cache / 'shared'
        shared.mkdir(mode=0o777)
        shared.chmod(0o777)
        private = shared / 'private'
        private.mkdir(mode=0o700)
        with self.assertRaisesRegex(ValueError, 'direct cache child'):
            self.plan(output=private / 'build')
        self.spawn.assert_not_called()
        self.assertFalse((private / 'build').exists())

    def test_nested_output_is_rejected_even_with_private_parents(self):
        parent = self.output.parent / 'private'
        parent.mkdir(parents=True, mode=0o700)
        with self.assertRaisesRegex(ValueError, 'direct cache child'):
            self.plan(output=parent / 'build')
        self.spawn.assert_not_called()
        self.assertFalse((parent / 'build').exists())

    def test_compiler_nonzero_keeps_primary_failure_logs_and_cleanup(self):
        receipt, unused = self.execute(returncode=17, product=False)
        self.assertEqual(receipt['outcome'], 'compiler_failure')
        self.assertEqual(receipt['process']['returncode'], 17)
        self.assertFalse(receipt['process']['timed_out'])
        self.assertEqual(receipt['cleanup']['returncode'], 17)
        self.assertEqual(Path(receipt['logs']['stdout']['path']).read_bytes(), b'Authored mock compiler stdout\n')
        self.assertIn(b'Authored mock compiler diagnostic', Path(receipt['logs']['stderr']['path']).read_bytes())
        self.assertTrue(Path(receipt['receipt_path']).is_file())

    def test_timeout_does_not_become_cleanup_returncode(self):
        timeout = build.subprocess.TimeoutExpired(['authored-mock'], 120)
        receipt, process = self.execute(observation_error=timeout, product=False)
        self.assertEqual(receipt['outcome'], 'compiler_timeout')
        self.assertTrue(receipt['process']['timed_out'])
        self.assertIsNone(receipt['process']['returncode'])
        self.assertEqual(receipt['cleanup']['returncode'], -15)
        self.assertEqual(len(process.wait.call_args_list), 1)
        self.assertEqual(len(self.nonreaping_wait.call_args_list), 1)
        self.assertEqual(self.signals.call_args_list, [mock.call(24680, build.signal.SIGTERM)])

    def test_failure_and_cleanup_errors_are_both_preserved(self):
        self.signals.side_effect = PermissionError('authored mock signal failure')
        plan = self.plan()
        self.simulated_compiler(returncode=23, product=False)
        self.child.descendants = {24681: 'S'}
        receipt = build.execute_build(plan)
        self.assertEqual(receipt['outcome'], 'compiler_failure')
        self.assertEqual(receipt['process']['returncode'], 23)
        self.assertGreaterEqual(len(receipt['cleanup']['errors']), 3)
        self.assertFalse(receipt['cleanup']['reaped'])
        self.assertIsNone(receipt['setup_error'])

    def test_failure_and_source_drift_do_not_mask_primary_status(self):
        source = self.root / next(iter(build.SOURCE_HASHES))
        receipt, unused = self.execute(returncode=37, mutation=lambda command: source.write_bytes(b'changed\n'))
        self.assertEqual(receipt['outcome'], 'compiler_failure')
        self.assertEqual(receipt['process']['returncode'], 37)
        self.assertTrue(receipt['integrity_errors'])

    def test_timeout_signal_and_group_verification_errors_are_all_preserved(self):
        timeout = build.subprocess.TimeoutExpired('authored-mock', 120)
        self.signals.side_effect = RuntimeError('authored mock cleanup error')
        receipt, unused = self.execute(observation_error=timeout, product=False)
        self.assertEqual(receipt['outcome'], 'compiler_timeout')
        self.assertTrue(receipt['process']['timed_out'])
        self.assertGreaterEqual(len(receipt['cleanup']['errors']), 3)
        self.assertEqual(len(receipt['cleanup']['waits']), 2)
        self.assertFalse(receipt['cleanup']['reaped'])

    def test_cleanup_failure_alone_is_not_success(self):
        self.signals.side_effect = PermissionError('authored mock cleanup error')
        plan = self.plan()
        self.simulated_compiler()
        self.child.descendants = {24681: 'S'}
        receipt = build.execute_build(plan)
        self.assertEqual(receipt['outcome'], 'cleanup_failure')
        self.assertEqual(receipt['process']['returncode'], 0)

    def test_lost_child_identity_prevents_any_group_signal(self):
        plan = self.plan()
        process = self.simulated_compiler(returncode=41)
        process.returncode = 41
        receipt = build.execute_build(plan)
        self.assertEqual(receipt['outcome'], 'process_error')
        self.assertIsNone(receipt['process']['returncode'])
        self.assertFalse(receipt['cleanup']['reaped'])
        self.assertTrue(receipt['cleanup']['errors'])
        self.signals.assert_not_called()

    def test_spawn_error_is_recorded_without_process_controls(self):
        plan = self.plan()
        self.spawn.side_effect = OSError('authored mock spawn failure')
        receipt = build.execute_build(plan)
        self.assertEqual(receipt['outcome'], 'process_error')
        self.assertIn('authored mock spawn failure', receipt['process']['error'])
        self.assertFalse(receipt['cleanup']['attempted'])
        self.signals.assert_not_called()

    def test_unwitnessed_backend_cannot_launch_compiler_or_inspector(self):
        plan = self.plan()
        with mock.patch.object(groups, 'CENSUS_WITNESSED', False):
            receipt = build.execute_build(plan)
        self.assertEqual(receipt['outcome'], 'process_error')
        self.assertFalse(receipt['process']['attempted'])
        self.assertIn('no parent witness', receipt['process']['error'])
        self.assertFalse(receipt['cleanup']['attempted'])
        self.assertEqual(receipt['cleanup_backend']['status'], 'unsupported_until_parent_witness')
        self.assertFalse(receipt['cleanup_backend']['census_completeness_qualified'])
        self.assertFalse(receipt['cleanup_backend']['escaped_descendants_contained'])
        self.assertTrue(Path(receipt['receipt_path']).exists())
        self.spawn.assert_not_called()
        self.inspector.assert_not_called()
        self.signals.assert_not_called()

    def test_missing_waitid_blocks_launch_and_retains_failure_receipt(self):
        plan = self.plan()
        with mock.patch.object(os, 'waitid'):
            del os.waitid
            receipt = build.execute_build(plan)
        self.assertIn('waitid/WNOWAIT', receipt['process']['error'])
        self.assertFalse(receipt['process']['attempted'])
        self.spawn.assert_not_called()

    def test_production_collector_pin_still_rejects_current_source_without_spawn(self):
        with mock.patch.object(build, 'SOURCE_HASHES', PRODUCTION_SOURCE_HASHES):
            with self.assertRaisesRegex(ValueError, 'Source pin mismatch'):
                self.plan()
        self.assertFalse(self.output.exists())
        self.spawn.assert_not_called()
        self.inspector.assert_not_called()
        self.signals.assert_not_called()

    def test_invalid_spawn_identity_retains_handle_without_numeric_signals(self):
        plan = self.plan()
        process = self.simulated_compiler()
        process.pid = 1
        receipt = build.execute_build(plan)
        self.assertEqual(receipt['outcome'], 'process_error')
        self.assertTrue(receipt['cleanup']['retained'])
        self.assertIs(groups.RETAINED_GROUPS[1], process)
        self.signals.assert_not_called()
        process.wait.assert_not_called()

    def test_retained_owned_handle_blocks_later_launches(self):
        groups.RETAINED_GROUPS[24680] = AuthoredChild()
        receipt = build.execute_build(self.plan())
        self.assertFalse(receipt['process']['attempted'])
        self.assertIn('Unresolved owned handles', receipt['process']['error'])
        self.spawn.assert_not_called()

    def test_interrupted_registration_retains_handle_and_blocks_next_launch(self):
        class InterruptedRegistration(dict):
            def __setitem__(self, key, value):
                raise KeyboardInterrupt('Authored registration interruption')

        plan = self.plan()
        process = self.simulated_compiler()
        with mock.patch.object(groups, 'RETAINED_GROUPS', InterruptedRegistration()), \
                mock.patch.object(groups, 'RETAINED_GROUP_HANDLES', {}):
            receipt = build.execute_build(plan)
            self.assertIn('registration interruption', receipt['process']['error'])
            self.assertIsNone(receipt['process']['pid'])
            self.assertTrue(receipt['cleanup']['retained'])
            self.assertIs(groups.RETAINED_GROUP_HANDLES.get(id(process)), process)
            self.spawn.reset_mock()
            next_plan = self.plan(output=self.root / 'cache' / 'next-build')
            next_receipt = build.execute_build(next_plan)
            self.assertFalse(next_receipt['process']['attempted'])
            self.assertIn('Unresolved owned handles', next_receipt['process']['error'])
            self.spawn.assert_not_called()
        self.nonreaping_wait.assert_not_called()
        self.inspector.assert_not_called()
        self.signals.assert_not_called()
        process.wait.assert_not_called()

    def test_interrupted_cleanup_dispatch_retains_handle_after_registry_removal(self):
        plan = self.plan()
        process = self.simulated_compiler(returncode=37)

        def interrupted_cleanup(*args):
            groups.RETAINED_GROUPS.pop(process.pid)
            raise KeyboardInterrupt('Authored cleanup dispatch interruption')

        with mock.patch.object(groups, 'cleanup_group', side_effect=interrupted_cleanup), \
                mock.patch.object(groups, 'RETAINED_GROUP_HANDLES', {}):
            receipt = build.execute_build(plan)
            self.assertEqual(receipt['process']['returncode'], 37)
            self.assertIn('cleanup dispatch interruption', receipt['cleanup']['errors'][0])
            self.assertTrue(receipt['cleanup']['retained'])
            self.assertIs(groups.RETAINED_GROUP_HANDLES.get(id(process)), process)
        self.signals.assert_not_called()
        process.wait.assert_not_called()

    def test_unhashable_spawn_identity_retains_handle_without_identity_operations(self):
        plan = self.plan()
        process = self.simulated_compiler()
        process.pid = []
        receipt = build.execute_build(plan)
        self.assertIn('TypeError', receipt['process']['error'])
        self.assertIsNone(receipt['process']['pid'])
        self.assertTrue(receipt['cleanup']['retained'])
        self.assertIs(groups.RETAINED_GROUP_HANDLES[id(process)], process)
        self.nonreaping_wait.assert_not_called()
        self.inspector.assert_not_called()
        self.signals.assert_not_called()
        process.wait.assert_not_called()

    def test_unexpected_cleanup_dispatch_error_does_not_erase_primary_failure(self):
        plan = self.plan()
        self.simulated_compiler(returncode=37)
        with mock.patch.object(groups, 'cleanup_group', side_effect=RuntimeError('Authored dispatch failure')):
            receipt = build.execute_build(plan)
        self.assertEqual(receipt['outcome'], 'compiler_failure')
        self.assertEqual(receipt['process']['returncode'], 37)
        self.assertIn('dispatch failure', receipt['cleanup']['errors'][0])
        self.assertTrue(receipt['cleanup']['retained'])
        self.assertIs(groups.RETAINED_GROUPS[self.child.pid], self.child)
        self.signals.assert_not_called()

    def test_primary_inspection_cleanup_and_integrity_failures_stay_distinct(self):
        source = self.root / next(iter(build.SOURCE_HASHES))
        plan = self.plan()
        self.simulated_compiler(returncode=23, mutation=lambda command: source.write_bytes(b'drift'))
        self.child.descendants = {24681: 'S'}
        def inspect(*args):
            evidence = authored_evidence(self.child.rows())
            return evidence if self.inspector.call_count == 1 else dict(
                evidence, errors=['Authored inspector failure'])

        self.inspector.side_effect = inspect
        self.signals.side_effect = PermissionError(1, 'Authored EPERM')
        receipt = build.execute_build(plan)
        self.assertEqual(receipt['outcome'], 'compiler_failure')
        self.assertEqual(receipt['process']['returncode'], 23)
        self.assertIn('EPERM', receipt['cleanup']['errors'][0])
        self.assertIn('Authored inspector failure', receipt['cleanup']['inspection_errors'])
        self.assertTrue(receipt['integrity_errors'])
        self.assertIsNone(receipt['cleanup']['returncode'])
        self.assertTrue(receipt['cleanup']['retained'])

    def test_wait_error_keeps_primary_error_separate_from_cleanup(self):
        receipt, unused = self.execute(observation_error=RuntimeError('authored primary wait failure'),
                                      error_exit_code=-9)
        self.assertEqual(receipt['outcome'], 'process_error')
        self.assertIn('authored primary wait failure', receipt['process']['error'])
        self.assertEqual(receipt['cleanup']['returncode'], -9)

    def test_interruption_keeps_primary_error_and_still_cleans_up(self):
        receipt, unused = self.execute(observation_error=KeyboardInterrupt('authored mock interruption'),
                                      error_exit_code=-9)
        self.assertEqual(receipt['outcome'], 'process_error')
        self.assertIn('KeyboardInterrupt', receipt['process']['error'])
        self.assertTrue(receipt['cleanup']['attempted'])
        self.assertEqual(receipt['cleanup']['returncode'], -9)

    def test_missing_version_never_reports_success(self):
        receipt, unused = self.execute(version=False)
        self.assertEqual(receipt['outcome'], 'compiler_version_unavailable')

    def test_missing_product_never_reports_success(self):
        receipt, unused = self.execute(product=False)
        self.assertEqual(receipt['outcome'], 'integrity_failure')
        self.assertIsNone(receipt['output']['state'])

    def test_empty_product_never_reports_success(self):
        receipt, unused = self.execute(mutation=lambda command: Path(command[-1]).write_bytes(b''))
        self.assertEqual(receipt['outcome'], 'integrity_failure')

    def test_symlink_product_is_rejected(self):
        def replace(command):
            product = Path(command[-1])
            product.unlink()
            product.symlink_to(self.compiler)
        receipt, unused = self.execute(mutation=replace)
        self.assertEqual(receipt['outcome'], 'integrity_failure')
        self.assertIsNone(receipt['output']['state'])

    def test_hardlinked_product_is_rejected(self):
        receipt, unused = self.execute(mutation=lambda command: os.link(command[-1], self.output / 'alias'))
        self.assertEqual(receipt['outcome'], 'integrity_failure')
        self.assertIsNone(receipt['output']['state'])

    def test_receipt_collision_never_overwrites_or_masks_compiler_failure(self):
        def collide(command):
            (self.output / 'build-receipt.json').write_bytes(b'Authored collision sentinel\n')
        receipt, unused = self.execute(returncode=29, mutation=collide)
        self.assertEqual(receipt['outcome'], 'compiler_failure')
        self.assertEqual(receipt['process']['returncode'], 29)
        self.assertIn('FileExistsError', receipt['receipt_write_error'])
        self.assertEqual((self.output / 'build-receipt.json').read_bytes(), b'Authored collision sentinel\n')

    def test_failed_cli_returns_nonzero_and_preserves_receipt(self):
        self.simulated_compiler(returncode=31, product=False)
        with contextlib.redirect_stdout(io.StringIO()):
            status = build.main(['--compiler', str(self.compiler), '--output', str(self.output), '--execute'])
        self.assertEqual(status, 1)
        self.assertEqual(json.loads((self.output / 'build-receipt.json').read_text())['process']['returncode'], 31)

    def test_timeout_argument_is_finite_positive_and_bounded(self):
        for timeout in (0, -1, 601, float('inf'), float('nan')):
            with self.subTest(timeout=timeout), self.assertRaisesRegex(ValueError, 'Timeout'):
                self.plan(timeout=timeout)

    def test_write_new_rejects_traversal_and_symlinks(self):
        descriptor = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        try:
            with self.assertRaisesRegex(ValueError, 'Unsafe output filename'):
                build.write_new(descriptor, '../escape', b'')
            (self.root / 'link').symlink_to(self.compiler)
            with self.assertRaises(FileExistsError):
                build.write_new(descriptor, 'link', b'')
        finally:
            os.close(descriptor)


class GroupCleanupTests(unittest.TestCase):
    def setUp(self):
        BuildTests.setUp(self)
        self.child = AuthoredChild()

    signal_group = BuildTests.signal_group

    def cleanup(self):
        return groups.cleanup_group(self.child, self.backend, mock.Mock(), 2)

    def assert_failed_closed(self, cleanup):
        self.assertTrue(cleanup['errors'])
        self.assertFalse(cleanup['reaped'])
        self.assertTrue(cleanup['retained'])
        self.assertIs(groups.RETAINED_GROUPS[self.child.pid], self.child)
        self.child.wait.assert_not_called()
        self.child.poll.assert_not_called()

    def test_zombie_only_group_skips_all_signals_and_reaps_after_two_censuses(self):
        self.child.exit_code = 0
        self.child.descendants = {24681: 'Z'}
        cleanup = self.cleanup()
        self.assertFalse(cleanup['errors'])
        self.assertTrue(cleanup['no_live_members'])
        self.assertTrue(cleanup['reaped'])
        self.assertEqual(len(cleanup['censuses']), 2)
        self.assertEqual(self.child.history[-1], 'reap')
        self.assertNotIn(self.child.pid, groups.RETAINED_GROUPS)
        self.signals.assert_not_called()

    def test_exited_leader_with_live_descendant_gets_full_term_grace_then_kill(self):
        self.child.exit_code = 0
        self.child.descendants = {24681: 'S'}
        cleanup = self.cleanup()
        self.assertFalse(cleanup['errors'])
        self.assertEqual(self.signals.call_args_list, [mock.call(24680, build.signal.SIGTERM),
                                                     mock.call(24680, build.signal.SIGKILL)])
        self.assertGreaterEqual(self.clock.now, 2)
        self.assertEqual(cleanup['waits'], [
            {'after': 'term', 'no_live_members': False, 'error': 'deadline'},
            {'after': 'kill', 'no_live_members': True, 'error': None},
        ])
        self.assertEqual(cleanup['returncode'], 0)
        self.assertEqual(self.child.history[-1], 'reap')

    def test_term_grace_is_group_wide_not_leader_only(self):
        self.child.descendants = {24681: 'R'}

        def change_during_grace(seconds):
            self.clock.now += seconds
            if self.clock.now >= 0.2:
                self.child.descendants[24681] = 'Z'

        self.clock.sleep = change_during_grace
        cleanup = self.cleanup()
        self.assertFalse(cleanup['errors'])
        self.assertGreaterEqual(self.clock.now, 0.2)
        self.assertIsNone(cleanup['kill'])
        self.assertEqual(cleanup['returncode'], -build.signal.SIGTERM)
        self.assertGreater(len(cleanup['censuses']), 3)

    def test_eperm_is_preserved_even_if_group_later_becomes_zombie_only(self):
        self.child.exit_code = 13
        self.child.descendants = {24681: 'S'}

        def denied_then_exit(pgid, signum):
            self.child.descendants[24681] = 'Z'
            raise PermissionError(1, 'Authored EPERM')

        self.signals.side_effect = denied_then_exit
        cleanup = self.cleanup()
        self.assertTrue(cleanup['reaped'])
        self.assertEqual(cleanup['returncode'], 13)
        self.assertEqual(cleanup['term'], 'error')
        self.assertIn('PermissionError', cleanup['errors'][0])
        self.assertIn('EPERM', cleanup['signals'][0]['error'])

    def test_esrch_is_an_error_not_no_live_members_proof(self):
        self.child.exit_code = 0
        self.child.descendants = {24681: 'S'}
        self.signals.side_effect = ProcessLookupError('Authored ESRCH')
        cleanup = self.cleanup()
        self.assert_failed_closed(cleanup)
        self.assertFalse(cleanup['no_live_members'])
        self.assertEqual(len(cleanup['signals']), 2)

    def test_kill_grace_must_prove_group_has_no_live_members(self):
        self.child.exit_code = 0
        self.child.descendants = {24681: 'U'}
        self.signals.side_effect = lambda *args: None
        cleanup = self.cleanup()
        self.assert_failed_closed(cleanup)
        self.assertIn('Live group members remain', cleanup['errors'][-1])

    def test_failed_censuses_never_signal_or_reap_and_keep_evidence(self):
        self.child.exit_code = 0
        anchor = self.child.rows()
        invalid = {
            'absent': '', 'missing_anchor': f'24681 24680 {os.getuid()} Z\n',
            'wrong_group': f'24680 24681 {os.getuid()} Z\n',
            'foreign_uid': f'24680 24680 {os.getuid() + 1} Z\n',
            'unknown_state': f'24680 24680 {os.getuid()} D\n',
            'denied_state': f'24680 24680 {os.getuid()} ?\n',
            'malformed': 'PID PGID UID STATE\n', 'truncated_line': anchor.rstrip('\n'),
            'duplicate': anchor + anchor, 'extra_field': anchor.rstrip() + ' ignored\n',
            'negative_pid': f'-24680 24680 {os.getuid()} Z\n',
            'carriage_return': anchor.replace('\n', '\r\n'),
            'wrong_live_anchor': anchor.replace(' Z\n', ' S\n'),
        }
        for name, output in invalid.items():
            with self.subTest(name=name):
                self.inspector.side_effect = None
                self.inspector.return_value = authored_evidence(output)
                cleanup = self.cleanup()
                self.assert_failed_closed(cleanup)
                self.assertTrue(cleanup['inspection_errors'])
                self.assertEqual(cleanup['censuses'][0]['inspection']['stdout'], output)
        self.signals.assert_not_called()

    def test_inspector_errors_denial_truncation_and_unreaped_auxiliary_fail_closed(self):
        self.child.exit_code = 0
        variants = ({'errors': ['denied']}, {'stderr': 'Operation not permitted\n'},
                    {'truncated': True}, {'returncode': 1}, {'reaped': False},
                    {'auxiliary_errors': ['reap: timed out']})
        for changes in variants:
            with self.subTest(changes=changes):
                evidence = dict(authored_evidence(self.child.rows()), **changes)
                self.inspector.side_effect = None
                self.inspector.return_value = evidence
                cleanup = self.cleanup()
                self.assert_failed_closed(cleanup)
                self.assertTrue(cleanup['inspection_errors'])
        self.signals.assert_not_called()

    def test_inconsistent_terminal_censuses_do_not_authorize_reap(self):
        self.child.exit_code = 0
        anchor = self.child.rows()
        self.inspector.side_effect = [authored_evidence(anchor), authored_evidence(
            anchor + f'24681 24680 {os.getuid()} S\n')]
        cleanup = self.cleanup()
        self.assert_failed_closed(cleanup)
        self.assertIn('Inconsistent terminal', cleanup['errors'][-1])
        self.signals.assert_not_called()

    def test_leader_state_transition_during_census_is_not_silently_accepted(self):
        def race(*args):
            self.child.exit_code = 0
            return authored_evidence(self.child.rows())

        self.inspector.side_effect = race
        cleanup = self.cleanup()
        self.assert_failed_closed(cleanup)
        self.assertIn('changed across census', cleanup['inspection_errors'][0])
        self.signals.assert_not_called()

    def test_child_lost_before_inspection_stops_all_numeric_group_signals(self):
        self.waitid.side_effect = ChildProcessError('Authored ECHILD')
        cleanup = self.cleanup()
        self.assert_failed_closed(cleanup)
        self.inspector.assert_not_called()
        self.signals.assert_not_called()

    def test_child_lost_immediately_before_signal_stops_escalation(self):
        self.waitid.side_effect = [None, None, ChildProcessError('Authored ECHILD')]
        cleanup = self.cleanup()
        self.assert_failed_closed(cleanup)
        self.assertEqual(cleanup['term'], 'error')
        self.assertIsNone(cleanup['kill'])
        self.signals.assert_not_called()

    def test_reaped_handle_is_never_used_for_group_signals(self):
        self.child.returncode = 0
        cleanup = self.cleanup()
        self.assert_failed_closed(cleanup)
        self.waitid.assert_not_called()
        self.signals.assert_not_called()

    def test_second_cleanup_cannot_signal_previously_reaped_group_number(self):
        self.child.exit_code = 0
        self.assertTrue(self.cleanup()['reaped'])
        self.signals.reset_mock()
        self.assert_failed_closed_after_second_cleanup(self.cleanup())

    def assert_failed_closed_after_second_cleanup(self, cleanup):
        self.assertFalse(cleanup['reaped'])
        self.assertTrue(cleanup['errors'])
        self.signals.assert_not_called()

    def test_waitid_wrong_pid_uid_and_unknown_status_fail_closed(self):
        observations = (
            SimpleNamespace(si_pid=24681, si_uid=os.getuid(), si_code=os.CLD_EXITED, si_status=0),
            SimpleNamespace(si_pid=24680, si_uid=os.getuid() + 1, si_code=os.CLD_EXITED, si_status=0),
            SimpleNamespace(si_pid=24680, si_uid=os.getuid(), si_code=999, si_status=0),
            SimpleNamespace(si_pid=24680, si_uid=os.getuid(), si_code=os.CLD_EXITED, si_status=256),
            SimpleNamespace(si_pid=24680, si_uid=os.getuid(), si_code=os.CLD_KILLED, si_status=0),
        )
        for observation in observations:
            with self.subTest(observation=observation):
                self.waitid.side_effect = None
                self.waitid.return_value = observation
                self.assert_failed_closed(self.cleanup())
        self.signals.assert_not_called()

    def test_nonreaping_identity_uses_only_owned_p_pid_wnowait(self):
        self.child.exit_code = -9
        identity = groups.child_identity(self.child, os.getuid())
        self.assertEqual(identity['returncode'], -9)
        self.waitid.assert_called_once_with(os.P_PID, 24680, os.WEXITED | os.WNOWAIT | os.WNOHANG)
        self.child.wait.assert_not_called()
        self.assertIsNone(self.child.returncode)

    def test_unknown_waitid_capability_or_competing_waiter_is_unsupported(self):
        with mock.patch.object(os, 'waitid', None):
            self.assert_failed_closed(self.cleanup())
        with mock.patch.object(groups.threading, 'active_count', return_value=2):
            self.assert_failed_closed(self.cleanup())
        with mock.patch.object(groups.os, 'WNOWAIT'):
            del groups.os.WNOWAIT
            self.assert_failed_closed(self.cleanup())
        self.signals.assert_not_called()

    def test_reap_errors_and_status_mismatch_remain_cleanup_errors(self):
        self.child.exit_code = 0
        self.child.wait.side_effect = build.subprocess.TimeoutExpired('authored', 2)
        self.assert_failed_closed_after_reap_attempt(self.cleanup())

    def test_reap_status_mismatch_is_not_accepted_as_clean_success(self):
        self.child.exit_code = 0

        def wrong_status(timeout):
            self.child.returncode = 17
            return 17

        self.child.wait.side_effect = wrong_status
        cleanup = self.cleanup()
        self.assertTrue(cleanup['reaped'])
        self.assertFalse(cleanup['retained'])
        self.assertIn('status changed', cleanup['errors'][-1])
        self.signals.assert_not_called()

    def assert_failed_closed_after_reap_attempt(self, cleanup):
        self.assertFalse(cleanup['reaped'])
        self.assertTrue(cleanup['no_live_members'])
        self.assertTrue(cleanup['retained'])
        self.assertTrue(cleanup['errors'])
        self.signals.assert_not_called()

    def test_final_child_identity_must_still_be_waitable_before_reap(self):
        self.child.exit_code = 0
        exited = self.child.observation()
        self.waitid.side_effect = [exited, exited, exited, exited, ChildProcessError('Authored lost child')]
        self.assert_failed_closed(self.cleanup())
        self.signals.assert_not_called()


class AuxiliaryInspectorTests(unittest.TestCase):
    def setUp(self):
        BuildTests.setUp(self)
        self.reader = mock.Mock(return_value=self.backend['before'])
        self.pipe_stdout = mock.Mock()
        self.pipe_stdout.fileno.return_value = 101
        self.pipe_stderr = mock.Mock()
        self.pipe_stderr.fileno.return_value = 102
        self.auxiliary = mock.Mock(pid=13579, stdout=self.pipe_stdout, stderr=self.pipe_stderr)
        self.auxiliary.wait.return_value = 0
        self.spawn.side_effect = None
        self.spawn.return_value = self.auxiliary
        self.ready = self.stack.enter_context(mock.patch.object(
            groups.select, 'select', side_effect=lambda streams, *unused: (streams, [], [])))
        self.chunks = {101: [b'24680 24680 501 Z\n', b''], 102: [b'']}
        self.read = self.stack.enter_context(mock.patch.object(
            groups.os, 'read', side_effect=lambda descriptor, bound: self.chunks[descriptor].pop(0)))

    signal_group = BuildTests.signal_group

    def inspect(self):
        return ACTUAL_INSPECT_GROUP(self.backend, 24680, self.reader)

    def test_targeted_pinned_ps_uses_unix2003_and_no_caller_environment(self):
        with mock.patch.dict(os.environ, {'COMMAND_MODE': 'legacy', 'DYLD_INSERT_LIBRARIES': 'never-used',
                                         'COLUMNS': '1', 'PS_FORMAT': 'command'}):
            evidence = self.inspect()
        self.assertFalse(evidence['errors'])
        self.assertFalse(evidence['auxiliary_errors'])
        self.assertTrue(evidence['reaped'])
        self.spawn.assert_called_once_with(
            ['/bin/ps', '-g', '24680', '-o', 'pid=,pgid=,uid=,state='], cwd='/',
            env={'COMMAND_MODE': 'unix2003', 'PATH': '/usr/bin:/bin', 'LANG': 'C', 'LC_ALL': 'C'},
            stdin=build.subprocess.DEVNULL, stdout=build.subprocess.PIPE, stderr=build.subprocess.PIPE,
            start_new_session=True, close_fds=True, shell=False)
        self.assertEqual(evidence['before'], evidence['after'])
        self.assertEqual(self.reader.call_args_list, [mock.call('/bin/ps', executable=True)] * 2)
        self.assertLessEqual(self.auxiliary.wait.call_args.kwargs['timeout'], groups.INSPECTION_SECONDS)
        self.auxiliary.kill.assert_not_called()
        self.auxiliary.poll.assert_not_called()
        self.pipe_stdout.close.assert_called_once_with()
        self.pipe_stderr.close.assert_called_once_with()
        self.signals.assert_not_called()

    def test_auxiliary_timeout_kills_only_owned_handle_and_reaps_boundedly(self):
        def timeout(streams, unused_write, unused_error, remaining):
            self.clock.now += remaining
            return [], [], []

        self.ready.side_effect = timeout
        evidence = self.inspect()
        self.assertIn('TimeoutExpired', evidence['errors'][0])
        self.assertEqual(evidence['kill'], 'sent_to_owned_handle')
        self.assertTrue(evidence['reaped'])
        self.auxiliary.kill.assert_called_once_with()
        self.auxiliary.wait.assert_called_once_with(timeout=groups.AUXILIARY_REAP_SECONDS)
        self.signals.assert_not_called()

    def test_auxiliary_stdout_and_stderr_share_a_strict_capture_bound(self):
        self.chunks = {101: [b'123456789'], 102: [b'123456789']}
        with mock.patch.object(groups, 'MAX_OUTPUT_BYTES', 12):
            evidence = self.inspect()
        self.assertTrue(evidence['truncated'])
        self.assertEqual(len(evidence['stdout']) + len(evidence['stderr']), 12)
        self.assertTrue(evidence['errors'])
        self.auxiliary.kill.assert_called_once_with()
        self.assertTrue(evidence['reaped'])

    def test_auxiliary_wait_timeout_does_not_discard_primary_or_cleanup_error(self):
        self.auxiliary.wait.side_effect = [build.subprocess.TimeoutExpired('authored-ps', 0.5), -9]
        self.auxiliary.kill.side_effect = PermissionError(1, 'Authored auxiliary EPERM')
        evidence = self.inspect()
        self.assertIn('TimeoutExpired', evidence['errors'][0])
        self.assertIn('EPERM', evidence['auxiliary_errors'][0])
        self.assertTrue(evidence['reaped'])
        self.assertEqual(evidence['returncode'], -9)
        self.assertEqual(self.auxiliary.wait.call_args_list,
                         [mock.call(timeout=0.5), mock.call(timeout=groups.AUXILIARY_REAP_SECONDS)])

    def test_unreaped_auxiliary_is_retained_and_blocks_any_new_auxiliary(self):
        self.auxiliary.wait.side_effect = build.subprocess.TimeoutExpired('authored-ps', 0.5)
        self.auxiliary.kill.side_effect = PermissionError(1, 'Authored auxiliary EPERM')
        evidence = self.inspect()
        self.assertFalse(evidence['reaped'])
        self.assertTrue(evidence['retained'])
        self.assertEqual(len(evidence['auxiliary_errors']), 2)
        self.assertIs(groups.RETAINED_AUXILIARIES[13579], self.auxiliary)
        self.spawn.reset_mock()
        blocked = self.inspect()
        self.assertIn('Unresolved auxiliary', blocked['errors'][0])
        self.spawn.assert_not_called()

    def test_executable_source_drift_before_launch_prevents_inspection(self):
        self.reader.side_effect = [dict(self.backend['before'], sha256='changed'), self.backend['before']]
        evidence = self.inspect()
        self.assertIn('changed before', evidence['errors'][0])
        self.assertIsNone(evidence['pid'])
        self.spawn.assert_not_called()

    def test_executable_source_drift_after_reap_invalidates_census(self):
        self.reader.side_effect = [self.backend['before'], dict(self.backend['before'], sha256='changed')]
        evidence = self.inspect()
        self.assertTrue(evidence['reaped'])
        self.assertIn('changed after', evidence['errors'][0])
        with self.assertRaisesRegex(ValueError, 'clean, bounded census'):
            groups.parse_census(evidence, 24680, 501)

    def test_primary_read_close_and_identity_errors_are_all_recorded(self):
        self.read.side_effect = OSError('Authored read error')
        self.pipe_stdout.close.side_effect = OSError('Authored close error')
        self.reader.side_effect = [self.backend['before'], OSError('Authored identity error')]
        evidence = self.inspect()
        self.assertEqual(len(evidence['errors']), 2)
        self.assertIn('read error', evidence['errors'][0])
        self.assertIn('identity error', evidence['errors'][1])
        self.assertIn('close error', evidence['auxiliary_errors'][0])

    def test_nonzero_exit_stderr_denial_and_non_ascii_are_not_clean_censuses(self):
        self.auxiliary.wait.return_value = 1
        self.assertTrue(self.inspect()['errors'])
        self.auxiliary.wait.return_value = 0
        self.chunks = {101: [b'24680 24680 501 Z\n', b''], 102: [b'Permission denied\n', b'']}
        denied = self.inspect()
        self.assertIn('diagnostic/denial', denied['errors'][0])
        self.assertIn('Permission denied', denied['stderr'])
        self.chunks = {101: [b'\xff\n'], 102: [b'']}
        self.assertIn('UnicodeDecodeError', self.inspect()['errors'][0])

    def test_unwitnessed_backend_and_invalid_group_never_start_auxiliary(self):
        with mock.patch.object(groups, 'CENSUS_WITNESSED', False):
            evidence = self.inspect()
        self.assertIn('no parent witness', evidence['errors'][0])
        for pgid in (0, 1, -24680, True, '24680'):
            evidence = ACTUAL_INSPECT_GROUP(self.backend, pgid, self.reader)
            self.assertIn('Invalid owned PGID', evidence['errors'][0])
        self.spawn.assert_not_called()

    def test_backend_requires_trusted_executable_and_retains_unqualified_scope(self):
        for state in ({'uid': 501, 'mode': 0o555}, {'uid': 0, 'mode': 0o777}):
            with self.subTest(state=state), self.assertRaisesRegex(ValueError, 'root-owned'):
                ACTUAL_BACKEND_PLAN(mock.Mock(return_value=state))
        self.assertFalse(self.backend['census_completeness_qualified'])
        self.assertFalse(self.backend['escaped_descendants_contained'])

    def test_census_row_line_pid_and_state_bounds(self):
        for output in ('24680 24680 501 Z\n' * (groups.MAX_ROWS + 1),
                       ' ' * 129 + '24680 24680 501 Z\n',
                       '999999999999999999999 24680 501 Z\n24680 24680 501 Z\n',
                       '24680 24680 501 Zunknown\n'):
            with self.subTest(output=output[:80]), self.assertRaises(ValueError):
                groups.parse_census(authored_evidence(output), 24680, 501)
        for state in ('I', 'R', 'S', 'T', 'U', 'Z', 'Zs', 'S<', 'R+', 'SE'):
            rows = groups.parse_census(authored_evidence(f'24680 24680 501 {state}\n'), 24680, 501)
            self.assertEqual(rows[0]['state'], state)


class SourceContractTests(unittest.TestCase):
    def setUp(self):
        patcher = mock.patch.object(build, 'select', mock_kqueue_backend())
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_kevent_exit_note_is_exact_and_does_not_establish_group_liveness(self):
        process = AuthoredChild()
        queue = mock.Mock()
        queue.control.return_value = [SimpleNamespace(
            ident=24680, filter=build.select.KQ_FILTER_PROC, flags=0,
            fflags=build.select.KQ_NOTE_EXIT | 1)]
        with mock.patch.object(build.select, 'kqueue', return_value=queue), \
                self.assertRaisesRegex(ValueError, 'exit observation failed'):
            build.wait_without_reaping(process, 1)
        process.wait.assert_not_called()
        self.assertIsNone(process.returncode)

    def test_production_census_gate_remains_disabled(self):
        self.assertFalse(groups.CENSUS_WITNESSED)

    def test_nonreaping_wait_observes_exit_without_waiting_or_signaling(self):
        process = mock.Mock(pid=24680, args=['authored-mock'])
        queue = mock.Mock(spec=['control', 'close'])
        queue.control.return_value = [SimpleNamespace(ident=24680, flags=0,
            filter=build.select.KQ_FILTER_PROC, fflags=build.select.KQ_NOTE_EXIT)]
        with mock.patch.object(build.select, 'kqueue', return_value=queue), \
                mock.patch.object(build.select, 'kevent') as event, \
                mock.patch.object(build.os, 'killpg') as signals:
            self.assertIsNone(build.wait_without_reaping(process, 1))
        event.assert_called_once_with(24680, filter=build.select.KQ_FILTER_PROC,
                                      flags=build.select.KQ_EV_ADD | build.select.KQ_EV_ONESHOT,
                                      fflags=build.select.KQ_NOTE_EXIT)
        queue.control.assert_called_once_with([event.return_value], 1, 1)
        queue.close.assert_called_once_with()
        process.wait.assert_not_called()
        signals.assert_not_called()

    def test_nonreaping_wait_timeout_is_bounded_with_all_os_waits_mocked(self):
        process = mock.Mock(pid=24680, args=['authored-mock'])
        queue = mock.Mock(spec=['control', 'close'])
        queue.control.return_value = []
        with mock.patch.object(build.select, 'kqueue', return_value=queue), \
                mock.patch.object(build.select, 'kevent'):
            with self.assertRaises(build.subprocess.TimeoutExpired):
                build.wait_without_reaping(process, 1)
        self.assertEqual(queue.control.call_args.args[1:], (1, 1))
        queue.close.assert_called_once_with()
        process.wait.assert_not_called()

    def test_nonreaping_wait_rejects_kernel_error_events_without_reaping(self):
        process = mock.Mock(pid=24680, args=['authored-mock'])
        queue = mock.Mock(spec=['control', 'close'])
        queue.control.return_value = [SimpleNamespace(ident=24680, flags=build.select.KQ_EV_ERROR,
            filter=build.select.KQ_FILTER_PROC, fflags=build.select.KQ_NOTE_EXIT)]
        with mock.patch.object(build.select, 'kqueue', return_value=queue), \
                mock.patch.object(build.select, 'kevent'):
            with self.assertRaisesRegex(ValueError, 'exit observation failed'):
                build.wait_without_reaping(process, 1)
        queue.close.assert_called_once_with()
        process.wait.assert_not_called()

    def test_nonreaping_wait_closes_queue_after_control_failure(self):
        process = mock.Mock(pid=24680, args=['authored-mock'])
        queue = mock.Mock(spec=['control', 'close'])
        queue.control.side_effect = OSError('authored kernel observation failure')
        with mock.patch.object(build.select, 'kqueue', return_value=queue), \
                mock.patch.object(build.select, 'kevent'):
            with self.assertRaisesRegex(OSError, 'authored kernel observation failure'):
                build.wait_without_reaping(process, 1)
        queue.close.assert_called_once_with()
        process.wait.assert_not_called()

    def test_nonreaping_wait_requires_process_exit_filter_and_note(self):
        process = mock.Mock(pid=24680, args=['authored-mock'])
        for event_filter, event_note in ((0, build.select.KQ_NOTE_EXIT), (build.select.KQ_FILTER_PROC, 0)):
            queue = mock.Mock(spec=['control', 'close'])
            queue.control.return_value = [SimpleNamespace(ident=24680, flags=0,
                filter=event_filter, fflags=event_note)]
            with self.subTest(filter=event_filter, note=event_note), \
                    mock.patch.object(build.select, 'kqueue', return_value=queue), \
                    mock.patch.object(build.select, 'kevent'), \
                    self.assertRaisesRegex(ValueError, 'exit observation failed'):
                build.wait_without_reaping(process, 1)
            queue.close.assert_called_once_with()
        process.wait.assert_not_called()

    def test_native_pins_match_but_current_collector_remains_explicitly_blocked(self):
        collector = 'crates/sims-app/src/window_session/presentation.rs'
        self.assertEqual(build.SOURCE_HASHES[collector],
                         'c679300b96095eebb5bd6813921d67d2820cb176bae6d6a47c30a9153800a063')
        for name, data in NATIVE_BYTES.items():
            with self.subTest(source=name):
                actual = hashlib.sha256(data).hexdigest()
                if name == collector:
                    self.assertNotEqual(actual, build.SOURCE_HASHES[name])
                else:
                    self.assertEqual(actual, build.SOURCE_HASHES[name])

    def test_every_c_and_rust_field_matches_without_ctypes_or_compilation(self):
        header = NATIVE_BYTES['crates/sims-app/native/window_present_macos.h'].decode()
        rust = NATIVE_BYTES['crates/sims-app/src/window_session/presentation.rs'].decode()
        mapping = {'uint32_t': 'u32', 'int32_t': 'i32', 'uint64_t': 'u64',
                   'double': 'f64', 'SimsWindowPresentCounters': 'Counters'}
        for name, expected_size, expected_fields in (('Event', 192, 29), ('Counters', 304, 38), ('Summary', 408, 16)):
            c_body = re.search(r'typedef struct SimsWindowPresent' + name + r' \{(.*?)\}', header, re.S)[1]
            rust_body = re.search(r'struct ' + name + r' \{(.*?)\n\}', rust, re.S)[1]
            c_fields = [(field, mapping[kind]) for kind, field in re.findall(r'(\w+)\s+(\w+);', c_body)]
            rust_fields = re.findall(r'(\w+): (\w+),', rust_body)
            self.assertEqual(c_fields, rust_fields)
            self.assertEqual(len(c_fields), expected_fields)
            offset = 0
            offsets = {}
            for field, kind in c_fields:
                size = 304 if kind == 'Counters' else 4 if kind in ('u32', 'i32') else 8
                alignment = min(size, 8)
                offset = (offset + alignment - 1) // alignment * alignment
                offsets[field] = offset
                offset += size
            self.assertEqual((offset + 7) // 8 * 8, expected_size)
            if name == 'Event':
                self.assertEqual(offsets['host_time'], 104)
            if name == 'Summary':
                self.assertEqual(offsets['counters'], 104)

    def test_audit_guard_rejects_process_network_native_and_signal_events(self):
        for event in ('subprocess.Popen', 'socket.connect', 'ctypes.dlopen', 'os.system',
                      'os.posix_spawn', 'os.exec', 'os.fork', 'os.kill', 'os.killpg'):
            with self.subTest(event=event), self.assertRaisesRegex(RuntimeError, 'Audit guard blocked'):
                sys.audit(event)
        for module in ('ctypes', '_ctypes', 'Foundation', 'objc', 'Metal', 'AppKit',
                       'CoreFoundation', 'CoreGraphics', 'ScreenCaptureKit', 'cffi'):
            with self.subTest(module=module), self.assertRaisesRegex(RuntimeError, 'native import'):
                sys.audit('import', module)

    def test_capabilities_status_and_drain_are_diagnostic_not_qualification(self):
        header = NATIVE_BYTES['crates/sims-app/native/window_present_macos.h'].decode()
        native = (SOURCE_REPOSITORY / 'crates/sims-app/native/window_present_macos.m').read_text()
        rust = NATIVE_BYTES['crates/sims-app/src/window_session/presentation.rs'].decode()
        for declaration in ('SIMS_WINDOW_PRESENT_ABI_VERSION = 1', 'SIMS_WINDOW_PRESENT_CAPACITY = 512',
                            'SIMS_WINDOW_PRESENT_MAX_DRAIN_TIMEOUT_MS = 10000',
                            'SIMS_WINDOW_PRESENT_OK = 0', 'SIMS_WINDOW_PRESENT_DRAIN_TIMEOUT = 1',
                            'SIMS_WINDOW_PRESENT_DUPLICATE_BIND = 2'):
            self.assertIn(declaration, header)
        statuses = re.findall(r'SIMS_WINDOW_PRESENT_ERROR_\w+ = (-\d+)', header)
        self.assertEqual([int(status) for status in statuses], list(range(-1, -16, -1)))
        self.assertIn('SIMS_WINDOW_PRESENT_CAPABILITY_LAYER_SETTINGS = 1u << 0', header)
        self.assertIn('SIMS_WINDOW_PRESENT_CAPABILITY_APP_NOTIFICATIONS = 1u << 1', header)
        self.assertIn('SIMS_WINDOW_PRESENT_CAPABILITY_WORKSPACE_NOTIFICATIONS = 1u << 2', header)
        self.assertIn('sims_capabilities = SIMS_WINDOW_PRESENT_CAPABILITY_LAYER_SETTINGS\n'
                      '            | SIMS_WINDOW_PRESENT_CAPABILITY_APP_NOTIFICATIONS;', native)
        self.assertIn('summary->flags |= SIMS_WINDOW_PRESENT_SUMMARY_WORKSPACE_NOTIFICATIONS_UNAVAILABLE;', native)
        self.assertIn('timeout_ms > SIMS_WINDOW_PRESENT_MAX_DRAIN_TIMEOUT_MS', native)
        self.assertIn('double remaining = deadline - CACurrentMediaTime();', native)
        self.assertIn('pthread_cond_timedwait_relative_np', native)
        self.assertIn('|summary| unsafe { (observer.end)(500, summary, std::mem::size_of::<Summary>() as u32) }', rust)
        self.assertIn('not photons or GPU duration', rust)

    def test_no_existing_root_test_module_is_imported(self):
        self.assertFalse(any(name.startswith(('test_window_', 'test_play_alpha')) for name in sys.modules))


if __name__ == '__main__':
    unittest.main(verbosity=2)
