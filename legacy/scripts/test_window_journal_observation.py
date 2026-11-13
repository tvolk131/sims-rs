"""Authored synthetic controls; UNRUN. No native/process/network work is allowed.

The guard confines ordinary Python file APIs, resolves descriptor-relative
accesses, and denies process/network/native operations. It is instrumentation,
not adversarial Python isolation. Every fixture is an authored temporary file.
"""

import builtins
import contextlib
import hashlib
import importlib
import io
import json
import os
from pathlib import Path
import shutil
import stat
import sys
import tempfile
import types
import unittest
from unittest import mock


ACTIVE_GUARD = None
FORBIDDEN = frozenset(('subprocess', '_posixsubprocess', 'socket', '_socket', 'ctypes', '_ctypes',
                       'multiprocessing', 'objc', 'cffi', '_cffi_backend', 'resource', 'pty',
                       'window_owned_child', 'window_process_group'))


def deny_operation(*arguments, **keywords):
    raise RuntimeError('Synthetic guard forbids process/network/native operations')


def deny_audit(event, arguments):
    if ACTIVE_GUARD is None:
        return
    if event == 'import' and (arguments[0].split('.')[0] in FORBIDDEN
                              or arguments[0][:1].isupper()
                              or (len(arguments) > 1 and type(arguments[1]) is str
                                  and arguments[1].endswith(('.so', '.dylib', '.pyd')))):
        deny_operation()
    if event.startswith(('subprocess.', 'socket.', 'ctypes.', 'os.exec', 'os.spawn', 'os.posix_spawn',
                         'os.fork', 'os.kill', 'os.system', 'pty.spawn', 'os.startfile')):
        deny_operation()


sys.addaudithook(deny_audit)


class SyntheticGuard(contextlib.AbstractContextManager):
    def __init__(self, root, *, sources=()):
        self.root = os.path.realpath(root)
        self.source = str(Path(__file__).resolve().with_name('window_journal_observation.py'))
        self.sources = frozenset(str(Path(p).resolve()) for p in sources) | {self.source}
        self.descriptors, self.stack = {}, contextlib.ExitStack()
        self.original = {name: getattr(os, name) for name in (
            'open', 'close', 'read', 'write', 'stat', 'lstat', 'fstat', 'lseek', 'mkdir',
            'unlink', 'rmdir', 'rename', 'replace', 'link', 'symlink', 'scandir', 'listdir', 'chmod', 'statvfs', 'fstatvfs')}
        self.builtin_open, self.io_open, self.original_import = builtins.open, io.open, builtins.__import__

    def descriptor_path(self, descriptor):
        saved = self.descriptors.get(descriptor)
        if saved is None:
            raise RuntimeError('Synthetic guard: unknown descriptor')
        info = self.original['fstat'](descriptor)
        if (info.st_dev, info.st_ino) != saved[1]:
            raise RuntimeError('Synthetic guard: stale descriptor')
        return saved[0]

    def checked(self, value, *, dir_fd=None, metadata=False, follow_leaf=True, write=False):
        if type(value) is int:
            return self.checked(self.descriptor_path(value), metadata=metadata, write=write)
        raw = os.fsdecode(value)
        if not os.path.isabs(raw):
            if dir_fd is None:
                raise RuntimeError('Synthetic guard: unanchored relative path')
            raw = os.path.join(self.descriptor_path(dir_fd), raw)
        absolute = os.path.normpath(raw)
        synthetic = absolute == self.root or absolute.startswith(self.root + '/')
        ancestor = self.root.startswith(absolute.rstrip('/') + '/') or any(
            p.startswith(absolute.rstrip('/') + '/') for p in self.sources)
        standard = any(absolute.startswith(prefix + '/') for prefix in (sys.base_prefix, sys.prefix))
        if not (synthetic or (not write and (absolute in self.sources or standard or (metadata and ancestor)))):
            raise RuntimeError('Synthetic guard: access outside authored root')
        current = ''
        parts = absolute.split('/')[1:]
        for index, component in enumerate(parts):
            current += '/' + component
            if index == len(parts) - 1 and not follow_leaf:
                break
            try:
                info = self.original['lstat'](current)
            except FileNotFoundError:
                break
            if stat.S_ISLNK(info.st_mode):
                raise RuntimeError('Synthetic guard: symlink traversal')
        return absolute

    def opened(self, value, flags, mode=0o777, *, dir_fd=None):
        writing = bool(flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC))
        absolute = self.checked(value, dir_fd=dir_fd, metadata=bool(flags & os.O_DIRECTORY),
                                follow_leaf=not bool(flags & os.O_NOFOLLOW), write=writing)
        descriptor = self.original['open'](value, flags, mode, dir_fd=dir_fd)
        info = self.original['fstat'](descriptor)
        self.descriptors[descriptor] = (absolute, (info.st_dev, info.st_ino))
        return descriptor

    def closed(self, descriptor):
        self.descriptor_path(descriptor)
        self.descriptors.pop(descriptor)
        return self.original['close'](descriptor)

    def descriptor_operation(self, name):
        def operation(descriptor, *arguments):
            self.checked(self.descriptor_path(descriptor), metadata=name in ('fstat', 'fstatvfs'), write=name == 'write')
            return self.original[name](descriptor, *arguments)
        return operation

    def file_open(self, original):
        def operation(value, mode='r', *arguments, **keywords):
            self.checked(value, write=any(flag in mode for flag in 'wax+'))
            stream = original(value, mode, *arguments, **keywords)
            info = self.original['fstat'](stream.fileno())
            path = self.descriptor_path(value) if type(value) is int else os.path.abspath(os.fsdecode(value))
            self.descriptors[stream.fileno()] = (path, (info.st_dev, info.st_ino))
            return stream
        return operation

    def path_operation(self, name):
        def operation(value, *arguments, **keywords):
            follow_leaf = name not in ('lstat', 'unlink', 'rmdir', 'mkdir')
            if name == 'stat' and keywords.get('follow_symlinks') is False:
                follow_leaf = False
            self.checked(value, dir_fd=keywords.get('dir_fd'), metadata=name in ('stat', 'lstat'),
                         follow_leaf=follow_leaf, write=name not in ('stat', 'lstat', 'scandir', 'listdir', 'statvfs'))
            return self.original[name](value, *arguments, **keywords)
        return operation

    def pair_operation(self, name):
        def operation(source, destination, *arguments, **keywords):
            self.checked(source, dir_fd=keywords.get('src_dir_fd', keywords.get('dir_fd')), follow_leaf=False)
            self.checked(destination, dir_fd=keywords.get('dst_dir_fd', keywords.get('dir_fd')),
                         follow_leaf=False, write=True)
            return self.original[name](source, destination, *arguments, **keywords)
        return operation

    def imported(self, name, *arguments, **keywords):
        if name.split('.')[0] in FORBIDDEN or name[:1].isupper():
            deny_operation()
        return self.original_import(name, *arguments, **keywords)

    def __enter__(self):
        global ACTIVE_GUARD
        if ACTIVE_GUARD is not None:
            raise RuntimeError('Nested synthetic guards are not supported')
        ACTIVE_GUARD = self
        replacements = {'open': self.opened, 'close': self.closed}
        replacements.update({name: self.descriptor_operation(name) for name in ('read', 'write', 'fstat', 'fstatvfs', 'lseek')})
        replacements.update({name: self.path_operation(name) for name in (
            'stat', 'lstat', 'mkdir', 'unlink', 'rmdir', 'scandir', 'listdir', 'chmod', 'statvfs')})
        replacements.update({name: self.pair_operation(name) for name in ('rename', 'replace', 'link', 'symlink')})
        for name, replacement in replacements.items():
            self.stack.enter_context(mock.patch.object(os, name, replacement))
        self.stack.enter_context(mock.patch.object(builtins, 'open', self.file_open(self.builtin_open)))
        self.stack.enter_context(mock.patch.object(io, 'open', self.file_open(self.io_open)))
        self.stack.enter_context(mock.patch.object(builtins, '__import__', self.imported))
        self.stack.enter_context(mock.patch.object(importlib.machinery.ExtensionFileLoader, 'exec_module', deny_operation))
        for name in ('system', 'fork', 'kill', 'killpg', 'posix_spawn', 'posix_spawnp', 'execv', 'execve',
                     'execl', 'execle', 'execvp', 'execvpe', 'spawnv', 'spawnve', 'popen', 'chdir',
                     'wait', 'waitpid', 'waitid', 'pidfd_open', 'forkpty'):
            if hasattr(os, name):
                self.stack.enter_context(mock.patch.object(os, name, deny_operation))
        for module_name, names in (('subprocess', ('Popen', 'run', 'call', 'check_call', 'check_output')),
                                   ('socket', ('socket', 'create_connection', 'getaddrinfo')),
                                   ('ctypes', ('CDLL', 'PyDLL', 'pythonapi')),
                                   ('signal', ('signal', 'alarm', 'setitimer', 'raise_signal', 'pthread_kill'))):
            module = sys.modules.get(module_name)
            for name in names:
                if module is not None and hasattr(module, name):
                    self.stack.enter_context(mock.patch.object(module, name, deny_operation))
        return self

    def __exit__(self, *arguments):
        global ACTIVE_GUARD
        try:
            return self.stack.__exit__(*arguments)
        finally:
            ACTIVE_GUARD = None


class Clock:
    now = 0

    def monotonic(self):
        return self.now


class FinalJournalObservationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='journal-authored-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        if shutil.disk_usage(self.root).free < 6 * 1024**3 + 3 * 64 * 1024**2:
            raise RuntimeError('Six GiB reserve required')
        self.output = self.root / 'owned'
        self.output.mkdir()
        self.telemetry = self.output / 'telemetry'
        self.telemetry.mkdir()
        self.journal = self.telemetry / 'events.jsonl'
        self.journal.write_bytes(b'{}\n')
        self.clock = Clock()
        self.guard = SyntheticGuard(self.root)
        self.guard.__enter__()
        self.addCleanup(self.guard.__exit__, None, None, None)
        self.module = types.ModuleType('authored_window_journal_observation')
        source = Path(self.guard.source).read_bytes()
        exec(compile(source, self.guard.source, 'exec'), self.module.__dict__)
        self.observer = self.module.JournalObservation(self.output, self.journal, deadline=10, clock=self.clock)
        self.observer.remember(directory_only=True)
        self.observer.remember()

    def observe(self, **keywords):
        return self.observer.observe(child_collected=keywords.get('child_collected', True),
            forced_teardown=keywords.get('forced_teardown', False), retained=keywords.get('retained', b''))

    def assert_incomplete(self, result):
        self.assertFalse(result['complete'])
        self.assertIsNone(result['wholefile_sha256'])
        self.assertIsNone(result['size_bytes'])
        self.assertEqual(result['status'], 'failed')

    def test_exact_64mib_limit_and_eof_are_not_overflow(self):
        self.journal.write_bytes(b'x' * self.module.MAX_BYTES)
        result, raw = self.observe()
        self.assertTrue(result['complete'])
        self.assertTrue(result['eof'])
        self.assertEqual(len(raw), self.module.MAX_BYTES)
        self.assertEqual(result['wholefile_sha256'], hashlib.sha256(raw).hexdigest())

    def test_overflow_size_is_rejected_without_wholefile_claim(self):
        self.journal.write_bytes(b'x' * (self.module.MAX_BYTES + 1))
        result, raw = self.observe()
        self.assert_incomplete(result)
        self.assertEqual(result['bytes_read'], len(raw))
        self.assertLessEqual(len(raw), self.module.MAX_BYTES + 1)

    def test_empty_raw_is_complete_observation_not_semantic_success(self):
        self.observer.previous_size = 0
        self.journal.write_bytes(b'')
        result, raw = self.observe()
        self.assertTrue(result['complete'])
        self.assertEqual(raw, b'')
        self.assertIsNone(self.observer.parse(result, raw, lambda data: json.loads(data)))
        self.assertEqual(result['semantic']['status'], 'failed')

    def test_missing_file_never_manufactures_empty_success(self):
        self.journal.unlink()
        result, raw = self.observe()
        self.assert_incomplete(result)
        self.assertEqual(raw, b'')

    def test_json_newline_and_record_errors_preserve_all_raw_bytes(self):
        for data, reason in ((b'{bad}\n', 'JSON'), (b'{}', 'newline'), (b'[]\n', 'record')):
            with self.subTest(reason=reason):
                self.observer.previous_size = 0
                self.journal.write_bytes(data)
                result, raw = self.observe()
                def reject(payload):
                    self.assertEqual(payload, data)
                    raise ValueError(reason)
                self.assertIsNone(self.observer.parse(result, raw, reject))
                self.assertEqual(raw, data)
                self.assertTrue(result['complete'])
                self.assertEqual(result['semantic']['error'], reason)

    def test_partial_read_error_keeps_prefix_hash_and_not_old_buffer(self):
        read = os.read
        calls = []
        def partial(descriptor, count):
            calls.append(count)
            if len(calls) == 1:
                return read(descriptor, 1)
            raise OSError('authored partial read')
        with mock.patch.object(os, 'read', partial):
            result, raw = self.observe(retained=b'old-buffer')
        self.assert_incomplete(result)
        self.assertEqual(raw, b'{')
        self.assertEqual(result['bytes_read'], 1)
        self.assertEqual(result['prefix_sha256'], hashlib.sha256(b'{').hexdigest())
        self.assertEqual(result['retained_buffer']['bytes'], len(b'old-buffer'))
        self.assertIsNone(result['retained_buffer']['prefix_matches'])

    def test_same_inode_rewrite_append_shrink_during_read_are_rejected(self):
        for change in (b'[]\n', b'{}\n{}\n', b'{'):
            with self.subTest(change=change):
                self.journal.write_bytes(b'{}\n')
                read, changed = os.read, []
                def mutate(descriptor, count):
                    data = read(descriptor, count)
                    if not changed:
                        changed.append(True)
                        self.journal.write_bytes(change)
                    return data
                with mock.patch.object(os, 'read', mutate):
                    result, raw = self.observe()
                self.assert_incomplete(result)
                self.assertEqual(result['bytes_read'], len(raw))

    def test_growth_uses_at_most_one_bounded_sentinel(self):
        self.observer.limit = 3
        read, changed = os.read, []
        def append(descriptor, count):
            if not changed:
                changed.append(True)
                self.journal.write_bytes(b'{}\nx')
            return read(descriptor, count)
        with mock.patch.object(os, 'read', append):
            result, raw = self.observe()
        self.assert_incomplete(result)
        self.assertEqual(raw, b'{}\nx')
        self.assertEqual(result['bytes_read'], 4)

    def test_symlink_and_hardlink_leaves_are_rejected(self):
        other = self.telemetry / 'other'
        other.write_bytes(b'{}\n')
        self.journal.unlink()
        os.symlink(other, self.journal)
        self.assert_incomplete(self.observe()[0])
        self.journal.unlink()
        os.link(other, self.journal)
        self.assert_incomplete(self.observe()[0])

    def test_original_leaf_substitution_before_final_read_is_rejected(self):
        replacement = self.telemetry / 'replacement'
        replacement.write_bytes(b'{}\n')
        os.replace(replacement, self.journal)
        self.assert_incomplete(self.observe()[0])

    def test_same_inode_rewrite_of_retained_prefix_is_not_a_wholefile_success(self):
        self.journal.write_bytes(b'[]\n')
        result, raw = self.observe(retained=b'{}\n')
        self.assert_incomplete(result)
        self.assertEqual(raw, b'[]\n')
        self.assertIs(result['retained_buffer']['prefix_matches'], False)
        self.assertEqual(result['prefix_sha256'], hashlib.sha256(raw).hexdigest())

    def test_original_ancestor_substitution_and_symlink_are_rejected(self):
        retained = self.output / 'retained-telemetry'
        self.telemetry.rename(retained)
        self.telemetry.mkdir()
        self.journal.write_bytes(b'{}\n')
        self.assert_incomplete(self.observe()[0])
        self.journal.unlink()
        self.telemetry.rmdir()
        os.symlink(retained, self.telemetry)
        self.assert_incomplete(self.observe()[0])

    def test_leaf_and_ancestor_substitution_after_read_are_rejected(self):
        for ancestor in (False, True):
            with self.subTest(ancestor=ancestor):
                self.observer = self.module.JournalObservation(self.output, self.journal,
                    deadline=10, clock=self.clock)
                self.observer.remember(directory_only=True)
                self.observer.remember()
                read, changed = os.read, []
                def substitute(descriptor, count):
                    data = read(descriptor, count)
                    if not changed:
                        changed.append(True)
                        if ancestor:
                            self.telemetry.rename(self.output / 'retained')
                            self.telemetry.mkdir()
                        else:
                            self.journal.rename(self.telemetry / 'retained-leaf')
                        self.journal.write_bytes(b'{}\n')
                    return data
                with mock.patch.object(os, 'read', substitute):
                    result, _ = self.observe()
                self.assert_incomplete(result)

    def test_deadline_on_path_read_parse_and_snapshot_never_extends(self):
        self.clock.now = 10
        self.assert_incomplete(self.observe()[0])
        self.clock.now = 0
        result, raw = self.observe()
        def parse(data):
            self.clock.now = 10
            return [json.loads(data)]
        self.assertIsNone(self.observer.parse(result, raw, parse))
        self.assert_incomplete(result)
        self.observer.snapshot(result, raw)
        self.assertEqual(result['snapshot']['status'], 'failed')
        self.assertEqual(self.observer.deadline, 10)

    def test_deadline_after_read_retains_prefix_without_completeness(self):
        read = os.read
        def expire(descriptor, count):
            data = read(descriptor, count)
            self.clock.now = 10
            return data
        with mock.patch.object(os, 'read', expire):
            result, raw = self.observe()
        self.assert_incomplete(result)
        self.assertEqual(raw, b'{}\n')
        self.assertEqual(result['prefix_sha256'], hashlib.sha256(raw).hexdigest())

    def test_deadline_after_last_artifact_write_is_a_failed_attempt(self):
        result, raw = self.observe()
        write = os.write
        def expire(descriptor, data):
            count = write(descriptor, data)
            self.clock.now = 10
            return count
        with mock.patch.object(os, 'write', expire):
            self.observer.snapshot(result, raw)
        self.assert_incomplete(result)
        self.assertEqual(result['snapshot']['status'], 'failed')
        self.assertEqual(result['snapshot']['bytes_written'], len(raw))

    def test_uncollected_and_forced_child_do_not_open_source(self):
        for collected, forced in ((False, False), (True, True)):
            with mock.patch.object(self.observer, '_open', side_effect=AssertionError('must not open')):
                result, raw = self.observe(child_collected=collected, forced_teardown=forced)
            self.assert_incomplete(result)
            self.assertEqual(raw, b'')

    def test_raw_snapshot_is_exclusive_0600_exact_and_collision_retained(self):
        result, raw = self.observe()
        self.observer.snapshot(result, raw)
        target = self.output / self.module.SNAPSHOT_NAME
        self.assertEqual(target.read_bytes(), raw)
        self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o600)
        self.assertEqual(result['snapshot']['sha256'], hashlib.sha256(raw).hexdigest())
        again, other = self.observe()
        self.observer.snapshot(again, other)
        self.assertEqual(again['snapshot']['status'], 'failed')
        self.assertEqual(target.read_bytes(), raw)

    def test_raw_snapshot_partial_write_retains_independent_error_accounting(self):
        result, raw = self.observe()
        write, calls = os.write, []
        def partial(descriptor, data):
            calls.append(True)
            if len(calls) > 1:
                raise OSError('authored snapshot write failure')
            return write(descriptor, data[:1])
        with mock.patch.object(os, 'write', partial):
            self.observer.snapshot(result, raw)
        self.assertTrue(result['complete'])
        self.assertEqual(result['snapshot']['status'], 'failed')
        self.assertEqual(result['snapshot']['bytes_written'], 1)
        self.assertIsNone(result['snapshot']['sha256'])
        self.assertEqual(result['snapshot']['prefix_sha256'], hashlib.sha256(raw[:1]).hexdigest())

    def test_guard_resolves_relative_descriptors_and_denies_escape_and_native_imports(self):
        descriptor = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            with self.assertRaises(RuntimeError):
                os.open('../../etc/passwd', os.O_RDONLY, dir_fd=descriptor)
            for name in ('subprocess', 'socket', 'ctypes', 'objc', 'window_owned_child'):
                with self.assertRaises(RuntimeError):
                    __import__(name)
            with self.assertRaises(RuntimeError):
                os.system('never executed')
        finally:
            os.close(descriptor)


if __name__ == '__main__':
    unittest.main()
