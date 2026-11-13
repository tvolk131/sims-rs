"""Guarded Worker A actual-source controls. AUTHORED, ALL UNRUN.

Parent-only filters (fresh interpreter, never run by workers)::

    PYTHONPATH=scripts python3 -B -m unittest test_window_normal_admission.ImportPurity
    PYTHONPATH=scripts python3 -B -m unittest test_window_normal_admission.SchemaControls
    PYTHONPATH=scripts python3 -B -m unittest test_window_normal_admission.ResourceControls
    PYTHONPATH=scripts python3 -B -m unittest test_window_normal_admission.IssuanceControls
    PYTHONPATH=scripts python3 -B -m unittest test_window_normal_admission.PreparationControls
    PYTHONPATH=scripts python3 -B -m unittest test_window_normal_child_protocol.NormalErrnoTests

The last existing suite stays untouched. All fixtures here have SYNTHETIC
provenance. Actual source is loaded without rewriting, under permanent audit
and import guards. No native extension, process, real filesystem resource,
FIFO traffic, clock, signal, normal Child creation or target execution is used.
Factory forwarding controls explicitly replace resource/native revalidation;
they cannot authenticate a private native Admission. Parent owns actual-image,
filesystem/bootstrap/durability/host-exit acceptance and all runtime permission.
Run separately from other suites that install permanent audit guards.
"""

import builtins
import copy
import hashlib
import importlib.machinery
import importlib.util
import json
import os
import pathlib
import pickle
import resource
import signal
import stat
import sys
import time
import types
import unittest
from unittest import mock


_ACTIVE = None
_IMPORT = builtins.__import__
_SCRIPTS = os.path.dirname(os.path.abspath(__file__))
_SOURCES = frozenset(('window_owned_child_bridge.py', 'window_suspended_host.py',
                      'window_normal_admission.py'))
_FORBIDDEN = frozenset(('subprocess', '_posixsubprocess', 'ctypes', '_ctypes', 'socket',
                        '_socket', 'multiprocessing', 'psutil', 'threading', '_thread',
                        'asyncio', 'selectors', 'select', '_window_owned_child_bridge'))
_CONTROL_ROOTS = []


def deny(*arguments, **keywords):
    raise AssertionError('Synthetic admission control attempted actual native/process/IO/clock access')


def guarded_import(name, globals=None, locals=None, fromlist=(), level=0):
    if name.split('.')[0] in _FORBIDDEN:
        deny()
    return _IMPORT(name, globals, locals, fromlist, level)


def audit(event, arguments):
    if _ACTIVE is None:
        return
    if event.startswith(('subprocess.', 'ctypes.', 'socket.', 'os.exec', 'os.spawn',
                         'os.posix_spawn', 'os.fork', 'os.kill', 'os.system',
                         'os.putenv', 'os.unsetenv')):
        deny()
    if event == 'import' and len(arguments) > 1 and type(arguments[1]) is str and arguments[1].endswith(('.so', '.dylib', '.pyd')):
        deny()
    if event == 'open':
        path = arguments[0]
        if not (_ACTIVE.source_import and type(path) is str and
                os.path.dirname(path) == _SCRIPTS and os.path.basename(path) in _SOURCES):
            deny()
    if event in ('os.listdir', 'os.scandir', 'os.chdir', 'os.remove', 'os.rename', 'os.mkdir'):
        deny()


class Guard:
    def __init__(self):
        self.source_import = False
        self.saved = []

    def replace(self, target, name, replacement):
        if hasattr(target, name):
            self.saved.append((target, name, getattr(target, name)))
            setattr(target, name, replacement)

    def __enter__(self):
        global _ACTIVE
        if _ACTIVE is not None:
            raise RuntimeError('Nested guard')
        _ACTIVE = self
        self.replace(builtins, '__import__', guarded_import)
        self.replace(builtins, 'open', deny)
        self.replace(importlib.machinery.ExtensionFileLoader, 'create_module', deny)
        self.replace(importlib.machinery.ExtensionFileLoader, 'exec_module', deny)
        for name in tuple(vars(os)):
            if name.startswith(('exec', 'spawn')) or name in (
                    'fork', 'forkpty', 'posix_spawn', 'system', 'popen', 'wait', 'wait3',
                    'wait4', 'waitid', 'waitpid', 'kill', 'killpg', 'getpid', 'getppid',
                    'getuid', 'geteuid', 'getgid', 'getegid', 'getpgid', 'getsid',
                    'setsid', 'setpgid', 'open', 'read', 'write', 'close', 'lseek',
                    'stat', 'lstat', 'fstat', 'statvfs', 'fstatvfs', 'fsync', 'fchmod',
                    'listdir', 'scandir', 'mkfifo', 'chdir', 'unlink', 'remove', 'rename'):
                self.replace(os, name, deny)
        for name in ('sleep', 'time', 'time_ns', 'monotonic', 'monotonic_ns',
                     'perf_counter', 'perf_counter_ns', 'process_time', 'process_time_ns',
                     'thread_time', 'thread_time_ns', 'clock_gettime', 'clock_gettime_ns'):
            self.replace(time, name, deny)
        for name in ('signal', 'getsignal', 'pause', 'pthread_sigmask', 'sigwait',
                     'sigwaitinfo', 'sigtimedwait', 'raise_signal', 'alarm', 'setitimer'):
            self.replace(signal, name, deny)
        for name in ('setrlimit', 'getrlimit', 'getrusage'):
            self.replace(resource, name, deny)
        return self

    def __exit__(self, *unused):
        global _ACTIVE
        for target, name, value in reversed(self.saved):
            setattr(target, name, value)
        _ACTIVE = None


def actual_source(name):
    path = _SCRIPTS + '/' + name + '.py'
    raw = importlib.machinery.SourceFileLoader(name, path).get_data(path)
    module = types.ModuleType(name)
    module.__file__, module.__package__ = path, ''
    sys.modules[name] = module
    _CONTROL_ROOTS.append(module)
    exec(compile(raw, path, 'exec', dont_inherit=True), module.__dict__)
    return module


sys.addaudithook(audit)


class Control(unittest.TestCase):
    def setUp(self):
        self.guard = Guard()
        self.guard.__enter__()
        self.addCleanup(self.guard.__exit__)
        self.guard.source_import = True
        try:
            self.sealed = actual_source('window_suspended_host')
            self.protocol = actual_source('window_owned_child_bridge')
            self.normal = actual_source('window_normal_admission')
        finally:
            self.guard.source_import = False

    def patch(self, target, name, replacement):
        context = mock.patch.object(target, name, replacement)
        result = context.__enter__()
        self.addCleanup(context.__exit__, None, None, None)
        return result

    def synthetic_issuance(self):
        """Explicit internal fixture; NEVER genuine native admission evidence."""
        normal = self.normal
        admission = object.__new__(normal.Admission)
        record = normal._Record(admission)
        normal._ADMISSIONS.append(record)
        class MockBridge:
            def __init__(self):
                self.calls = []
            def pending(self):
                self.calls.append('pending')
                return None
            def history(self):
                self.calls.append('history')
                return ()
            def create(self, *arguments):
                self.calls.append(('create', arguments))
                raise RuntimeError('Synthetic creation boundary: no Child or OS call')
        class MockChild:
            pass
        class MockSink:
            pass
        binding = types.SimpleNamespace(Bridge=MockBridge, Child=MockChild, TerminalSink=MockSink)
        record.binding, record.bridge = binding, MockBridge()
        record.child_type, record.terminal_type = MockChild, MockSink
        record.cwd = b'/synthetic-only-never-opened/child-cwd'
        record.options = ((b'PATH=/usr/bin:/bin', b'LC_ALL=C'), (37, 37, 37),
                          10_000_000_000, 10_000_000_000, 100_000_000, 128)
        record.manifest = dict(profile=normal.PROFILE, nested=dict(provenance='SYNTHETIC'))
        record.ready = True
        return admission, record


class ImportPurity(Control):
    def test_import_keeps_all_registries_and_suspended_globals_unchanged(self):
        self.assertEqual(self.normal._ADMISSIONS, [])
        self.assertEqual(self.protocol._CHILD_EXECUTIONS, [])
        self.assertEqual(self.sealed._LIFETIME, [])
        self.assertEqual(self.sealed.PROFILE, 'SUSPENDED_TRUE_ONLY_NO_RESUME_NO_PUBLICATION')
        self.assertNotIn('window_normal_admission.py', self.sealed.FILES)

    def test_public_paths_remain_closed_before_any_binding_access(self):
        for action in (self.protocol.ChildExecution, self.protocol.production_child_execution,
                       self.protocol.production_backend, self.protocol.require_launch_ready):
            with self.assertRaises(self.protocol.Unsupported):
                action(object())
        self.assertFalse(self.protocol._native_execution_authorized(object()))
        with self.assertRaises(TypeError):
            self.normal.Admission()


class SchemaControls(Control):
    def manifest(self):
        identity = dict(device=1, inode=2, uid=12, mode=0o700)
        return dict(schema=1, profile=self.normal.PROFILE, root=identity,
                    fifo=dict(identity, inode=3, mode=0o600), token='a' * 32,
                    child_cwd=dict(identity, inode=4),
                    files={name:dict(bytes=self.normal.IMAGE_BYTES if name.endswith('.so') else 1,
                                     mode=0o400, sha256=self.normal.IMAGE_SHA256 if name.endswith('.so') else 'b' * 64)
                           for name in self.normal.FILES},
                    target=dict(path='/usr/bin/true', device=5, inode=6, bytes=8,
                                uid=0, mode=0o755, mtime_ns=9, sha256='c' * 64),
                    python=dict(path=self.normal.PYTHON, sha256=self.normal.PYTHON_SHA256))

    def parse(self, value):
        raw = json.dumps(value).encode('utf-8')
        return self.normal._manifest(raw, hashlib.sha256(raw).hexdigest(), 12)

    def test_exact_normal_manifest_and_sealed_suspended_profile(self):
        value = self.manifest()
        self.assertEqual(self.parse(value), value)
        value['profile'] = self.sealed.PROFILE
        with self.assertRaises(ValueError):
            self.parse(value)
        value = self.manifest()
        raw = json.dumps(value).encode('utf-8')
        with self.assertRaises(ValueError):
            self.sealed._manifest(raw, hashlib.sha256(raw).hexdigest(), 12)

    def test_duplicate_keys_wrong_digest_and_fixed_image_refused(self):
        raw = json.dumps(self.manifest()).encode('utf-8')
        duplicate = b'{"schema":1,' + raw[1:]
        for payload, digest in ((duplicate, hashlib.sha256(duplicate).hexdigest()), (raw, '0' * 64)):
            with self.assertRaises(ValueError):
                self.normal._manifest(payload, digest, 12)
        for field, replacement in (('bytes', self.normal.IMAGE_BYTES + 1), ('sha256', '0' * 64),
                                   ('mode', 0o644), ('bytes', True)):
            value = self.manifest()
            value['files']['_window_owned_child_bridge.so'][field] = replacement
            with self.assertRaises(ValueError):
                self.parse(value)

    def test_no_extra_files_or_target_fallback_or_boolean_identity(self):
        for mutate in (lambda v:v['files'].update(extra=dict(bytes=1, mode=0o400, sha256='0'*64)),
                       lambda v:v['target'].update(path='/bin/true'),
                       lambda v:v['target'].update(uid=True),
                       lambda v:v['root'].update(inode=True),
                       lambda v:v.update(schema=True)):
            value = self.manifest()
            mutate(value)
            with self.assertRaises(ValueError):
                self.parse(value)

    def test_environment_is_fixed_bounded_and_profile_specific(self):
        fixed = dict(PATH='/usr/bin:/bin', LC_ALL='C', TMPDIR='/private/tmp',
                     SIMS_NORMAL_MANIFEST_SHA256='a'*64)
        self.assertEqual(self.normal._environment(fixed), 'a'*64)
        self.assertEqual(self.normal._environment(dict(fixed, __CF_USER_TEXT_ENCODING='0x0:0:0')), 'a'*64)
        for change in (dict(PATH='/usr/bin:/bin:/usr/local/bin'), dict(PYTHONPATH='scripts'),
                       dict(__CF_USER_TEXT_ENCODING='x'*129), dict(__CF_USER_TEXT_ENCODING='\0'),
                       dict(SIMS_SUSPENDED_MANIFEST_SHA256='a'*64)):
            with self.assertRaises(ValueError):
                self.normal._environment(dict(fixed, **change))


class ResourceControls(Control):
    def test_claim_uses_exclusive_open_and_both_fsyncs_before_return(self):
        admission, record = self.synthetic_issuance()
        events = []
        record.root = types.SimpleNamespace(fd=9, validate=lambda:events.append('validate'))
        record.uid, record.pid, record.pin = 12, 71, 'a'*64
        def opened(path, flags, mode, **keywords):
            self.assertEqual(path, 'host.claim')
            self.assertTrue(flags & os.O_EXCL and flags & os.O_CREAT and flags & os.O_NOFOLLOW)
            self.assertEqual(keywords, dict(dir_fd=9))
            self.assertEqual(mode, 0o600)
            events.append('exclusive-open')
            return 42
        def written(fd, raw):
            self.assertIn(fd, record.descriptors)
            self.assertEqual(record.claim, fd)
            events.append('write')
            return min(17, len(raw))
        self.patch(os, 'open', opened)
        self.patch(os, 'write', written)
        self.patch(os, 'fchmod', lambda fd, mode:events.append(('mode', fd, mode)))
        self.patch(os, 'fsync', lambda fd:events.append(('fsync', fd)))
        self.patch(os, 'fstat', lambda fd:'SYNTHETIC-STAT')
        self.patch(self.normal, '_check_file', lambda current, entry:events.append('check-file'))
        self.normal._claim(record)
        self.assertEqual(events[:2], ['validate', 'exclusive-open'])
        self.assertEqual(events[-4:], [('mode', 42, 0o400), ('fsync', 42), ('fsync', 9), 'check-file'])
        self.assertEqual(record.descriptors, [42])
        self.assertEqual(record.files['host.claim']['fd'], 42)

    def test_open_retains_descriptor_before_first_stat_and_uncertain_read(self):
        admission, record = self.synthetic_issuance()
        record.root = types.SimpleNamespace(fd=9)
        self.patch(os, 'open', lambda *a, **k:37)
        failure = OSError('Synthetic read loss')
        def lost(fd, *arguments, **keywords):
            self.assertIn(fd, record.descriptors)
            self.assertEqual(record.files['source']['fd'], fd)
            raise failure
        self.patch(self.sealed, '_read_fd', lost)
        with self.assertRaises(OSError) as caught:
            self.normal._file(record, 'source', 10, uid=12)
        self.assertIs(caught.exception, failure)
        self.assertEqual(record.descriptors, [37])

    def test_retained_file_rejects_replaced_path_even_with_identical_bytes(self):
        admission, record = self.synthetic_issuance()
        record.root = types.SimpleNamespace(fd=9)
        before = types.SimpleNamespace(st_dev=1, st_ino=2, st_uid=12,
            st_mode=stat.S_IFREG|0o400, st_size=1, st_nlink=1, st_mtime_ns=3, st_ctime_ns=4)
        other = types.SimpleNamespace(**dict(vars(before), st_ino=99))
        entry = dict(fd=37, name='source', maximum=10, uid=12, mode=0o400,
                     pin=None, path=None, stat=before, raw=b'x')
        self.patch(os, 'lseek', lambda *a:0)
        self.patch(self.sealed, '_read_fd', lambda *a, **k:(b'x', before))
        self.patch(os, 'stat', lambda *a, **k:other)
        with self.assertRaises(ValueError):
            self.normal._check_file(record, entry)

    def test_native_module_name_and_flag_cannot_authenticate_foreign_origin(self):
        admission, record = self.synthetic_issuance()
        binding = types.ModuleType('_window_owned_child_bridge')
        binding.__file__ = '/foreign/_window_owned_child_bridge.so'
        record.binding = binding
        record.root = types.SimpleNamespace(path=pathlib.Path('/synthetic-family'))
        with mock.patch.dict(sys.modules, {'_window_owned_child_bridge':binding}):
            with self.assertRaises(ValueError):
                self.normal._check_native(record)


class IssuanceControls(Control):
    def test_distinct_factory_forwarding_and_fixed_claim_without_mock_factory(self):
        admission, record = self.synthetic_issuance()
        validations = []
        def validate(current):
            self.assertIs(current, record)
            self.assertIsNotNone(record.execution)
            self.assertTrue(admission.owns_execution(record.execution))
            self.assertIs(record.execution._bridge, None)
            validations.append(current)
        self.patch(self.normal, '_revalidate', validate)
        self.patch(self.protocol.ChildExecution, 'for_injected_mock', deny)
        session = admission.create_execution()
        self.assertEqual(len(validations), 1)
        self.assertIs(type(session), self.protocol.ChildExecution)
        self.assertIs(session, admission.execution)
        self.assertIs(session._binding, admission.binding)
        self.assertIs(session._bridge, admission.bridge)
        self.assertIs(session._claim, admission)
        self.assertEqual(session._options, record.options)
        self.assertEqual(record.bridge.calls, [])
        self.assertFalse(session.native_qualified)
        self.assertFalse(admission.native_qualified)
        self.assertFalse(admission.owns_execution(37))
        self.assertFalse(admission.owns_execution(object()))
        self.patch(self.normal, '_revalidate', lambda current:validations.append(current))
        session._admission_started = True  # Synthetic original session intent, no Bridge.create.
        session._busy = True
        self.assertEqual(admission._admit(session), ((b'/usr/bin/true',), record.cwd))
        self.assertEqual(len(validations), 2)
        self.assertEqual(record.bridge.calls, [])
        with self.assertRaises(ValueError):
            admission._admit(session)
        with self.assertRaises(ValueError):
            admission.create_execution()

    def test_revalidation_loss_retains_exact_partial_session_without_retry(self):
        admission, record = self.synthetic_issuance()
        failure = OSError('Synthetic resource revalidation reply loss')
        def lost(current):
            self.assertIs(current.execution, admission.execution)
            self.assertTrue(admission.owns_execution(current.execution))
            raise failure
        self.patch(self.normal, '_revalidate', lost)
        with self.assertRaises(OSError) as caught:
            admission.create_execution()
        self.assertIs(caught.exception, failure)
        session = admission.execution
        self.assertTrue(admission.owns_execution(session))
        self.assertIs(session._bridge, None)
        self.assertIs(session._claim, None)
        self.assertIn(failure, record.errors)
        self.assertIn(failure, session._errors)
        with self.assertRaises(ValueError):
            admission.create_execution()
        with self.assertRaises(self.protocol.BridgeProtocolError):
            session.admit()
        self.assertEqual(record.bridge.calls, [])

    def test_lost_factory_return_recovers_only_original_property(self):
        admission, record = self.synthetic_issuance()
        self.patch(self.normal, '_revalidate', lambda current:None)
        factory = self.protocol._private_normal_execution
        def lost(original):
            factory(original)
            raise RuntimeError('Synthetic publication reply loss')
        self.patch(self.protocol, '_private_normal_execution', lost)
        with self.assertRaises(RuntimeError):
            admission.create_execution()
        session = admission.execution
        self.assertTrue(admission.owns_execution(session))
        self.assertIs(session._claim, admission)
        self.assertTrue(record.execution_ready)
        with self.assertRaises(ValueError):
            admission.create_execution()
        # Uncertain factory publication stays fail-closed, even if all fields exist.
        with self.assertRaises(ValueError):
            session.admit()
        self.assertEqual(record.bridge.calls, [])

    def test_unissued_duck_copies_serialization_and_reentry_refused(self):
        admission, record = self.synthetic_issuance()
        for action in (lambda:copy.copy(admission), lambda:copy.deepcopy(admission),
                       lambda:pickle.dumps(admission), lambda:self.normal._record(object()),
                       lambda:self.normal._record(object.__new__(self.normal.Admission))):
            with self.assertRaises((ValueError, TypeError)):
                action()
        self.patch(self.normal, '_revalidate', lambda current:admission.create_execution())
        with self.assertRaises(ValueError):
            admission.create_execution()
        self.assertTrue(admission.owns_execution(admission.execution))
        self.assertEqual(record.bridge.calls, [])

    def test_heap_mock_session_never_becomes_an_admission_or_native_authority(self):
        admission, record = self.synthetic_issuance()
        binding = record.binding
        session = self.protocol.ChildExecution.for_injected_mock(binding, record.bridge,
            bridge_type=binding.Bridge, child_type=binding.Child, terminal_type=binding.TerminalSink)
        self.assertFalse(session.native_qualified)
        self.assertFalse(admission.owns_execution(session))
        with self.assertRaises(ValueError):
            self.normal._record(session)
        with self.assertRaises(self.protocol.BridgeProtocolError):
            self.protocol.ChildExecution.for_injected_mock(object(), object(),
                bridge_type=type(object()), child_type=type(None), terminal_type=type(Ellipsis))


class PreparationControls(Control):
    def test_reserved_pid_refuses_before_image_target_or_family_access_and_roots_error(self):
        self.patch(os, 'getpid', lambda:47707)
        with self.assertRaises(ValueError):
            self.normal.prepare_private_true('/private/tmp/sims-normal-true-'+'a'*32, 'b'*64)
        self.assertEqual(len(self.normal._ADMISSIONS), 1)
        record = self.normal._ADMISSIONS[0]
        self.assertEqual(record.pid, 47707)
        self.assertEqual(record.descriptors, [])
        self.assertEqual(len(record.errors), 1)
        self.assertIsNone(record.execution)

    def test_exact_family_spelling_before_any_filesystem_access(self):
        self.patch(os, 'getpid', lambda:71)
        record = self.normal._Record(object.__new__(self.normal.Admission))
        for family in ('/tmp/sims-normal-true-'+'a'*32,
                       '/private/tmp/sims-normal-true-'+'A'*32,
                       '/private/tmp/sims-normal-true-'+'a'*31,
                       '/private/tmp/sims-normal-true-'+'a'*32+'/child',
                       '/private/tmp/sims-suspended-true-'+'a'*32):
            with self.assertRaises(ValueError):
                self.normal._context(record, family, 'b'*64)

    def test_claim_precedes_loading_and_prepare_never_calls_execution_factory(self):
        events = []
        for name in ('_context', '_bootstrap', '_resources', '_claim', '_load_native', '_revalidate'):
            def phase(record, *arguments, name=name):
                self.assertTrue(any(item is record for item in self.normal._ADMISSIONS))
                self.assertIsNone(record.execution)
                events.append(name)
            self.patch(self.normal, name, phase)
        self.patch(self.protocol, '_private_normal_execution', deny)
        self.patch(self.protocol.ChildExecution, 'for_injected_mock', deny)
        result = self.normal.prepare_private_true('SYNTHETIC-NO-PATH', 'SYNTHETIC-NO-PIN')
        self.assertEqual(events, ['_context', '_bootstrap', '_resources', '_claim', '_load_native', '_revalidate'])
        self.assertIsNone(result.execution)
        self.assertEqual(self.protocol._CHILD_EXECUTIONS, [])
        with self.assertRaises(ValueError):
            self.normal.prepare_private_true('SYNTHETIC-NO-PATH', 'SYNTHETIC-NO-PIN')
        self.assertEqual(len(events), 6)
        self.assertEqual(len(self.normal._ADMISSIONS), 2)

    def test_claim_failure_permanently_roots_resources_without_native_access(self):
        failure = OSError('Synthetic exclusive claim/fsync failure')
        for name in ('_context', '_bootstrap', '_resources'):
            self.patch(self.normal, name, lambda record, *arguments:record.descriptors.append(37))
        def lost(record):
            record.descriptors.append(38)
            raise failure
        self.patch(self.normal, '_claim', lost)
        self.patch(self.normal, '_load_native', deny)
        with self.assertRaises(OSError) as caught:
            self.normal.prepare_private_true('SYNTHETIC-NO-PATH', 'SYNTHETIC-NO-PIN')
        self.assertIs(caught.exception, failure)
        record = self.normal._ADMISSIONS[0]
        self.assertEqual(record.descriptors, [37, 37, 37, 38])
        self.assertIs(record.errors[0], failure)
        self.assertIsNone(record.execution)
        self.assertFalse(record.ready)


if __name__ == '__main__':
    unittest.main()
