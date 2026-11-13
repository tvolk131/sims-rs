"""Guarded actual-source host controls. ALL UNRUN; parent execution only.

Parent-only filters, in a fresh test interpreter (all UNRUN)::

    PYTHONPATH=scripts python3 -B -m unittest test_window_suspended_host.ImportPurity
    PYTHONPATH=scripts python3 -B -m unittest test_window_suspended_host.Admission
    PYTHONPATH=scripts python3 -B -m unittest test_window_suspended_host.FixedAttempt
    PYTHONPATH=scripts python3 -B -m unittest test_window_suspended_host.Retention
    PYTHONPATH=scripts python3 -B -m unittest test_window_suspended_host.ExitProof
    PYTHONPATH=scripts python3 -B -m unittest test_window_suspended_host.BootstrapOrder

Uses the actual entry, coordinator, IO/schema and consumer source without AST
rewriting. No native image, filesystem/FIFO, process, clock, signal or runtime is
exercised. Fixtures confer NO native authentication. File metadata and durability
are simulated here; parent owns actual file/IPC/native controls and acceptance.
Do not combine with other suites' permanent audit guards in one interpreter.
"""

import builtins
import hashlib
import importlib.machinery
import importlib.util
import json
import os
import pathlib  # Preload the IO support's stdlib dependency before source-only guards.
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
_SOURCE_NAMES = frozenset(('window_suspended_host.py', 'window_suspended_host_io.py',
                          'window_suspended_host_core.py', 'window_suspended_diagnostic.py',
                          'window_owned_child_bridge.py'))
_FORBIDDEN = frozenset(('subprocess', '_posixsubprocess', 'ctypes', '_ctypes', 'socket',
                        '_socket', 'multiprocessing', 'psutil', 'threading', '_thread',
                        'asyncio', 'selectors', 'select', '_window_owned_child_bridge'))


def _deny(*args, **kwargs):
    raise AssertionError('Host control attempted real native/process/clock/IPC/file access')


def _import(name, globals=None, locals=None, fromlist=(), level=0):
    if name.split('.')[0] in _FORBIDDEN:
        _deny()
    return _IMPORT(name, globals, locals, fromlist, level)


def _audit(event, args):
    guard = _ACTIVE
    if guard is None:
        return
    if event.startswith(('subprocess.', 'ctypes.', 'socket.', 'os.exec', 'os.spawn',
                         'os.posix_spawn', 'os.fork', 'os.kill', 'os.system',
                         'os.putenv', 'os.unsetenv')):
        _deny()
    if event == 'import' and len(args) > 1 and type(args[1]) is str and args[1].endswith(('.so', '.dylib', '.pyd')):
        _deny()
    if event == 'open':
        path = args[0]
        if not (guard.source_import and type(path) is str and
                os.path.basename(path) in _SOURCE_NAMES and os.path.dirname(path) == _SCRIPTS):
            _deny()
    if event in ('os.listdir', 'os.scandir', 'os.chdir', 'os.remove', 'os.rename', 'os.mkdir'):
        _deny()


_SCRIPTS = os.path.dirname(os.path.abspath(__file__))


class Guard:
    def __init__(self, source_import=False):
        self.source_import = source_import
        self.saved = []

    def replace(self, target, name, replacement):
        if hasattr(target, name):
            self.saved.append((target, name, getattr(target, name)))
            setattr(target, name, replacement)

    def __enter__(self):
        global _ACTIVE
        assert _ACTIVE is None
        _ACTIVE = self
        self.replace(builtins, '__import__', _import)
        self.replace(builtins, 'open', _deny)
        self.replace(importlib.machinery.ExtensionFileLoader, 'create_module', _deny)
        self.replace(importlib.machinery.ExtensionFileLoader, 'exec_module', _deny)
        for name in tuple(vars(os)):
            if name.startswith(('exec', 'spawn')) or name in (
                'fork', 'forkpty', 'posix_spawn', 'system', 'popen', 'wait', 'wait3',
                'wait4', 'waitid', 'waitpid', 'kill', 'killpg', 'getpid', 'getppid',
                'getuid', 'geteuid', 'getgid', 'getegid', 'getpgid', 'getsid',
                'setsid', 'setpgid', 'pipe', 'pipe2', 'open', 'read', 'write', 'close',
                'stat', 'lstat', 'fstat', 'statvfs', 'fstatvfs', 'fsync', 'fchmod',
                'listdir', 'scandir', 'mkfifo', 'chdir', 'unlink', 'remove', 'rename',
            ):
                self.replace(os, name, _deny)
        for name in ('sleep', 'time', 'time_ns', 'monotonic', 'monotonic_ns',
                     'perf_counter', 'perf_counter_ns', 'process_time', 'process_time_ns',
                     'thread_time', 'thread_time_ns', 'clock_gettime', 'clock_gettime_ns'):
            self.replace(time, name, _deny)
        for name in ('signal', 'getsignal', 'pause', 'pthread_sigmask', 'sigwait',
                     'sigwaitinfo', 'sigtimedwait', 'raise_signal', 'alarm', 'setitimer'):
            self.replace(signal, name, _deny)
        for name in ('setrlimit', 'getrlimit', 'getrusage'):
            self.replace(resource, name, _deny)
        return self

    def __exit__(self, *unused):
        global _ACTIVE
        for target, name, value in reversed(self.saved):
            setattr(target, name, value)
        _ACTIVE = None


_SOURCE_BYTES = {}


def _actual_source(name, filename=None):
    filename = filename or (_SCRIPTS + '/' + name + '.py')
    raw = importlib.machinery.SourceFileLoader(name, filename).get_data(filename)
    _SOURCE_BYTES[os.path.basename(filename)] = raw
    module = types.ModuleType(name)
    module.__file__, module.__package__ = filename, ''
    sys.modules[name] = module
    exec(compile(raw, filename, 'exec', dont_inherit=True), module.__dict__)
    return module


sys.addaudithook(_audit)
with Guard(source_import=True):
    # Direct actual-source loads avoid even attempting a cached .pyc read.
    host = _actual_source('window_suspended_host')
    host_io = _actual_source('window_suspended_host_io')
    core = _actual_source('window_suspended_host_core')
    schema = _actual_source('window_owned_child_bridge')
    consumer = _actual_source('window_suspended_diagnostic')


def data(value):
    if type(value) is dict:
        return {k: data(v) for k, v in value.items()}
    if type(value) in (tuple, list):
        return type(value)(data(v) for v in value)
    return value


def record(**changes):
    value = dict.fromkeys(schema.HEADER_FIELDS + schema.RECORD_FIELDS, 0)
    value.update(bytes=200, version=1, pid=71, pgid=71, uid=12, ppid=7, sid=71,
                 state=1, uid_evidence=1, sigchld_default=1, pid_bytes=4, capacity=2)
    value.update(changes)
    return value


def registration(**changes):
    value = dict.fromkeys(schema._REGISTRATION_SCALARS, 0)
    value.update(entered=1, returned=1, raw_siginfo_bytes=16, idtype=1, target_pid=71, options=123)
    value.update(changes)
    return value


def report(no_creation):
    abort = dict.fromkeys(schema._ABORT_SCALARS, 0)
    abort.update(max_entries=16, max_observations=8, origin_ns=1000,
                 deadline_ns=1300, now_ns=1000, exit_observation_index=-1,
                 record=dict.fromkeys(schema.HEADER_FIELDS + schema.RECORD_FIELDS, 0),
                 trace=(), observations=tuple(dict.fromkeys(schema._ABORT_OBSERVATION_SCALARS, 0)
                                               for unused in range(9)))
    value = dict.fromkeys(schema._CHILD_SCALARS, 0)
    value.update(generation=41, state=4, pid_valid=1, registered=1,
                 registration_bsd_status=4, registration_pid=71, registration_ppid=7,
                 registration_pgid=71, registration_uid=12, registration_sid=71,
                 spawn_entered=1, spawn_returned=1, spawn_pid_raw=71,
                 spawn_errno=19, system_errno=23, clock_errno=24,
                 launch_entries=2, launch_fact_count=1, launch_deadline_ns=1100,
                 resources_closed=1, max_cleanup_entries=128,
                 sigchld_preflight_entered=1, sigchld_preflight_returned=1,
                 sigchld_preflight_policy_valid=1, sigchld_preflight_default=1,
                 sigchld_preflight_accepted=1, record=record(), abort=abort,
                 registration_observation=registration(system_errno=25),
                 launch_facts=((1, 1, 1, 0, 26),))
    if no_creation:
        for name in ('pid_valid', 'registered', 'spawn_entered', 'spawn_returned',
                     'spawn_pid_raw', 'registration_pid', 'registration_ppid',
                     'registration_pgid', 'registration_uid', 'registration_sid',
                     'registration_bsd_status'):
            value[name] = 0
        value.update(state=7, rc=3, system_errno=9, launch_failure_mask=1,
                     launch_first_failure_phase=1, launch_first_failure_reason=1,
                     launch_first_woc_rc=3, launch_first_system_errno=9,
                     record=dict.fromkeys(schema.HEADER_FIELDS + schema.RECORD_FIELDS, 0),
                     registration_observation=registration(entered=0, returned=0, raw_siginfo_bytes=0))
    return value


class Child:
    pass


class Sink:
    def __init__(self, bridge):
        self.bridge = bridge
        r = report(bridge.no_creation)
        self.value = dict.fromkeys(schema._TERMINAL_SCALARS, 0)
        self.value.update(generation=41, rooted=1, known_no_creation=int(bridge.no_creation),
                          kind=1 if bridge.no_creation else 0, raw_siginfo=b'\0' * 16,
                          decoded_record=dict.fromkeys(schema.HEADER_FIELDS + schema.RECORD_FIELDS, 0),
                          final_report=r, registration_observation=r['registration_observation'])

    def snapshot(self):
        self.bridge.event('snapshot')
        return data(self.value)

    def registration_observation(self):
        return b'R' * 16

    def abort_observation(self, index):
        return bytes((65 + index,)) * 16


class Bridge:
    def __init__(self, no_creation=False, reject=False):
        self.no_creation, self.reject = no_creation, reject
        self.child = self.sink = None
        self.calls, self.actions, self.hooks = [], [], {}
        self.exit_next = True
        self.arguments = None
        self.assert_durable = lambda: None

    def event(self, name):
        self.calls.append(name)
        hook = self.hooks.pop(name, None)
        if isinstance(hook, BaseException):
            raise hook
        if hook is not None:
            hook()

    def pending(self):
        self.event('pending')
        return self.child

    def history(self):
        self.event('history')
        return () if self.sink is None else (self.sink,)

    def terminal(self, child):
        assert child is self.child
        return self.sink

    def child_snapshot(self, child):
        assert child is self.child
        return data(self.sink.value['final_report'])

    def create_suspended(self, *arguments):
        self.assert_durable()
        self.actions.append('create_suspended')
        self.arguments = arguments
        self.event('create.before')
        if self.reject:
            raise ValueError('preparation rejected before roots')
        self.child, self.sink = Child(), Sink(self)
        self.event('create.after')
        return self.child

    def start_action(self, name, child):
        assert child is self.child
        self.actions.append(name)
        self.event(name + '.before')
        return self.sink.value['final_report']['abort']

    def finish(self, name):
        self.event(name + '.after')
        return data(self.sink.value['final_report'])

    def abort_begin(self, child):
        a = self.start_action('abort_begin', child)
        a.update(started=1, phase=3, entries=a['entries'] + 1, last_operation=1,
                 last_rc=0, kill_entered=1, kill_returned=1, kill_errno=27,
                 record=record(no_signals=1, signal_attempted=1),
                 trace=((1, 1, 1, 0, 27),), trace_count=1)
        a['observations'][0].update(entered=1, returned=1, raw_siginfo_bytes=16)
        return self.finish('abort_begin')

    def abort_observe(self, child):
        a = self.start_action('abort_observe', child)
        a.update(entries=a['entries'] + 1, observation_count=a['observation_count'] + 1,
                 last_operation=2, last_rc=0, last_system_errno=28)
        index = a['observation_count']
        a['observations'][index].update(entered=1, returned=1, raw_siginfo_bytes=16,
                                         system_errno=29)
        if self.exit_next:
            a['observations'][index].update(pid=71, uid=9999, code=2, status=9)
            a.update(phase=4, exit_valid=1, exit_observation_index=index,
                     record=record(state=2, no_signals=1, signal_attempted=1,
                                   exit_kind=2, exit_status=9, returncode=-9, uid_evidence=2))
        return self.finish('abort_observe')

    def abort_reap(self, child):
        a = self.start_action('abort_reap', child)
        a.update(entries=a['entries'] + 1, phase=6, last_operation=3, last_rc=0)
        a['record'].update(reap_attempted=1, reaped=1)
        self.sink.value.update(kind=3, reap_attempted=1, wait_entered=1, wait_returned=1,
                               wait_rc=0, wait_errno=30, wait_pid=71, wait_uid=9999,
                               wait_code=2, wait_status=9, consumed=1, decoded=1,
                               certificate_matches=1, decoded_record=data(a['record']),
                               raw_siginfo=b'W' * 16)
        return self.finish('abort_reap')

    def abort_handoff(self, child, sink):
        a = self.start_action('abort_handoff', child)
        assert sink is self.sink
        a.update(entries=a['entries'] + 1, phase=7, last_operation=4, last_rc=0)
        sink.value['transferred'], self.child = 1, None
        self.event('abort_handoff.after')
        return sink

    def handoff(self, child, sink):
        self.start_action('handoff', child)
        assert sink is self.sink and self.no_creation
        sink.value['final_report']['state'] = 11
        sink.value['transferred'], self.child = 1, None
        self.event('handoff.after')
        return sink

    create = resume = publish = cleanup_begin = child_observe = child_signal = _deny
    child_reap = signal = kill = wait = poll = release = reset = _deny


class Binding:
    Bridge, Child, TerminalSink = Bridge, Child, Sink


class Journal:
    def __init__(self):
        self.events, self.durable = [], []
        self.failure = None
        self.on_append = None

    def append(self, event):
        if self.on_append:
            self.on_append(event)
        if self.failure is not None:
            raise self.failure
        host_io.encode(event)  # Exercise actual bounded evidence encoding, memory only.
        self.events.append(data(event))
        self.durable.append(event['kind'])  # Simulates return after both fsyncs.
        return dict(file='fixture', bytes=0, sha256='0' * 64)


class Fifo:
    def __init__(self, chunks=(), failure=None):
        self.chunks, self.failure = list(chunks), failure
        self.reads = 0

    def read(self):
        self.reads += 1
        if self.chunks:
            return self.chunks.pop(0)
        if self.failure is not None:
            raise self.failure
        return b''


class Park(BaseException):
    pass


def park():
    raise Park('retained without process exit')


class Guarded(unittest.TestCase):
    def setUp(self):
        self.guard = Guard()
        self.guard.__enter__()
        self.addCleanup(self.guard.__exit__)
        self.guard.replace(host, '_free', lambda root: None)
        self.guard.replace(host, '_dormant', park)

    def make(self, bridge=None, fifo=None):
        bridge = Bridge() if bridge is None else bridge
        journal = Journal()
        pauses = []
        app = host._Host(host_io, core, schema, consumer, Binding, bridge,
                         types.SimpleNamespace(fd=91), journal, fifo or Fifo(),
                         'a' * 32, host._arguments(b'/private/tmp/sims-suspended-true-fixture/child-cwd', 9),
                         pauses.append)
        bridge.assert_durable = lambda: self.assertIn('RUN_INTENT', journal.durable)
        return app, bridge, journal, pauses


class ImportPurity(Guarded):
    def test_import_has_no_admission_or_runtime_side_effects(self):
        self.guard.source_import = True
        name = 'suspended_host_import_purity_probe'
        module = _actual_source(name, host.__file__)
        self.assertEqual(module._LIFETIME, [])
        self.assertFalse(module._STARTED)
        self.assertEqual(module._SIGNALS, 0)
        self.assertNotIn('_window_owned_child_bridge', sys.modules)

    def test_reserved_pid_refuses_before_environment_modules_or_targets(self):
        self.guard.replace(host, '_STARTED', False)
        self.guard.replace(os, 'getpid', lambda: 47707)
        with self.assertRaisesRegex(ValueError, 'Reserved'):
            host.main()
        self.assertNotIn('_window_owned_child_bridge', sys.modules)

    def test_signal_handlers_only_accumulate_scalar_bits(self):
        self.guard.replace(host, '_SIGNALS', 0)
        for unused in range(100):
            for signum in (1, 2, 15):
                host._interrupt(signum, None)
        self.assertEqual(host._SIGNALS, 7)

    def test_policy_never_touches_sigchld(self):
        infinity = resource.RLIM_INFINITY
        for incoming, expected in (((infinity, infinity), 64 * 1024**2),
                                   ((128 * 1024**2, 128 * 1024**2), 64 * 1024**2),
                                   ((16 * 1024**2, 32 * 1024**2), 16 * 1024**2),
                                   ((8 * 1024**2, infinity), 8 * 1024**2)):
            with self.subTest(incoming=incoming):
                calls, limits = [], []
                self.guard.replace(signal, 'signal', lambda signum, handler: calls.append((signum, handler)))
                self.guard.replace(resource, 'getrlimit', lambda kind: incoming)
                self.guard.replace(resource, 'setrlimit', lambda kind, bounds: limits.append((kind, bounds)))
                host._policies()
                self.assertEqual({s for s, h in calls}, {signal.SIGINT, signal.SIGTERM, signal.SIGHUP})
                self.assertTrue(all(h is host._interrupt for s, h in calls))
                self.assertEqual(limits, [(resource.RLIMIT_CORE, (0, 0)),
                                         (resource.RLIMIT_FSIZE, (expected, expected))])


def manifest():
    identity = dict(device=1, inode=2, uid=12, mode=0o700)
    return dict(schema=1, profile=host.PROFILE, root=identity,
                fifo=dict(identity, inode=3, mode=0o600), token='a' * 32,
                files={name: dict(bytes=1, mode=0o500 if name.endswith('.so') else 0o400,
                                 sha256=hashlib.sha256(b'x').hexdigest()) for name in host.FILES},
                child_cwd=dict(identity, inode=4),
                target=dict(path='/usr/bin/true', device=1, inode=5, bytes=1,
                            uid=0, mode=0o755, mtime_ns=6, sha256='b' * 64),
                python=dict(path=host.PYTHON, sha256=host.PYTHON_SHA256))


def packed(value):
    raw = json.dumps(value).encode()
    return raw, hashlib.sha256(raw).hexdigest()


def file_stat(**changes):
    fields = dict(st_mode=stat.S_IFREG | 0o400, st_dev=1, st_ino=2, st_uid=12,
                  st_size=1, st_nlink=1, st_mtime_ns=3, st_ctime_ns=4, st_rdev=0)
    fields.update(changes)
    return types.SimpleNamespace(**fields)


class Admission(Guarded):
    def test_exact_manifest_and_minimal_environment(self):
        value = manifest()
        self.assertEqual(host._manifest(*packed(value), 12), value)
        env = dict(PATH='/usr/bin:/bin', LC_ALL='C', TMPDIR='/private/tmp',
                   SIMS_SUSPENDED_MANIFEST_SHA256='a' * 64)
        self.assertEqual(host._environment(env), 'a' * 64)
        self.assertEqual(host._environment(dict(env, __CF_USER_TEXT_ENCODING='0x0:0:0')), 'a' * 64)
        for key in ('DYLD_INSERT_LIBRARIES', 'PYTHONPATH', 'HOME', 'UNLISTED'):
            with self.subTest(key=key), self.assertRaises(ValueError):
                host._environment(dict(env, **{key: 'bad'}))
        for key in env:
            changed = dict(env)
            changed.pop(key)
            with self.subTest(missing=key), self.assertRaises(ValueError):
                host._environment(changed)

    def test_manifest_hash_precedes_json_and_duplicate_keys_are_rejected(self):
        with mock.patch.object(json, 'loads', _deny):
            with self.assertRaises(ValueError):
                host._manifest(b'{}', '0' * 64, 12)
        raw = b'{"schema":1,"schema":1}'
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            host._manifest(raw, hashlib.sha256(raw).hexdigest(), 12)

    def test_schema_keys_exact_types_identity_bounds_and_no_extra_image(self):
        mutations = (
            lambda v: v.update(schema=True), lambda v: v.update(profile='healthy'),
            lambda v: v.update(extra=1), lambda v: v['files'].pop(host.FILES[0]),
            lambda v: v['files'].update(other=dict(bytes=1, mode=0o400, sha256='b' * 64)),
            lambda v: v['files'][host.FILES[0]].update(mode=True),
            lambda v: v['files'][host.FILES[0]].update(bytes=host.MAX_FILE + 1),
            lambda v: v['files'][host.FILES[0]].update(sha256='A' * 64),
            lambda v: v['root'].update(uid=0), lambda v: v['fifo'].update(mode=0o666),
            lambda v: v['child_cwd'].update(inode=True), lambda v: v.update(token='a' * 31),
            lambda v: v['target'].update(path='/bin/true'),
            lambda v: v['target'].update(uid=True), lambda v: v['target'].update(mode=0o777),
            lambda v: v['python'].update(sha256='b' * 64),
        )
        for mutate in mutations:
            value = manifest()
            mutate(value)
            with self.subTest(mutation=mutate), self.assertRaises(ValueError):
                host._manifest(*packed(value), 12)
        self.assertNotIn('_window_owned_child_bridge', sys.modules)

    def test_same_descriptor_file_checks_and_changed_metadata_refuse(self):
        pin = dict(bytes=1, mode=0o400, sha256=hashlib.sha256(b'x').hexdigest())
        for change in ({'st_nlink': 2}, {'st_uid': 0}, {'st_size': host.MAX_FILE + 1},
                       {'st_mode': stat.S_IFLNK | 0o400}, {'st_mode': stat.S_IFREG | 0o600}):
            self.guard.replace(os, 'fstat', lambda fd, s=file_stat(**change): s)
            with self.subTest(change=change), self.assertRaises(ValueError):
                host._read_fd(7, host.MAX_FILE, uid=12, mode=0o400, pin=pin)
        reads = iter((file_stat(), file_stat(st_ctime_ns=99)))
        self.guard.replace(os, 'fstat', lambda fd: next(reads))
        chunks = iter((b'x', b''))
        self.guard.replace(os, 'read', lambda fd, size: next(chunks))
        with self.assertRaisesRegex(ValueError, 'changed'):
            host._read_fd(7, host.MAX_FILE, uid=12, pin=pin)

    def test_uncertain_close_is_not_retried(self):
        closes = []
        error = OSError('close reply lost')
        self.guard.replace(os, 'open', lambda *args, **kwargs: 7)
        self.guard.replace(host, '_read_fd', lambda *args, **kwargs: (b'x', file_stat()))
        def close(fd):
            closes.append(fd)
            raise error
        self.guard.replace(os, 'close', close)
        with self.assertRaises(OSError) as caught:
            host._file_at(3, 'manifest.json', host.MAX_FILE, 12)
        self.assertIs(caught.exception, error)
        self.assertEqual(closes, [7])

    def test_fifo_uses_own_rdwr_writer_and_exact_identity(self):
        calls = []
        s = file_stat(st_mode=stat.S_IFIFO | 0o600)
        self.guard.replace(os, 'open', lambda name, flags, **kw: calls.append((name, flags, kw)) or 9)
        self.guard.replace(os, 'fstat', lambda fd: s)
        self.guard.replace(os, 'getuid', lambda: 12)
        root = types.SimpleNamespace(fd=8, validate=lambda: None)
        fifo = host_io.Fifo(root, host_io.identity(s))
        self.assertEqual(fifo.fd, 9)
        self.assertEqual(calls, [('control.fifo', os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC,
                                 dict(dir_fd=8))])


class FixedAttempt(Guarded):
    def test_one_automatic_run_exact_arguments_durability_and_raw_evidence(self):
        app, bridge, journal, pauses = self.make()
        self.assertFalse(app.dispatch('RUN'))
        self.assertEqual(bridge.actions, ['create_suspended', 'abort_begin', 'abort_observe',
                                          'abort_reap', 'abort_handoff'])
        self.assertEqual(bridge.arguments, (b'/usr/bin/true', (b'/usr/bin/true',),
                         (b'PATH=/usr/bin:/bin', b'LC_ALL=C'),
                         b'/private/tmp/sims-suspended-true-fixture/child-cwd', (9, 9, 9),
                         10000000000, 10000000000, 0, 128))
        self.assertEqual(pauses, [0.01])
        self.assertEqual(journal.durable, ['RUN_INTENT', 'RUN_RESULT'])
        self.assertTrue(app.coordinator.facts['retirement_proven'])
        self.assertEqual(app.coordinator.facts['terminal']['wait_errno'], 30)
        self.assertEqual(app.coordinator.facts['registration_raw']['bytes'], b'R' * 16)
        self.assertEqual(journal.events[-1]['events'], data(app.coordinator.events))
        self.assertFalse(app.coordinator.facts['native_qualified'])
        self.assertFalse(app.coordinator.facts['continue_authorized'])
        self.assertFalse(app.coordinator.facts['healthy'])

    def test_no_journal_writes_inside_native_chain(self):
        app, bridge, journal, unused = self.make()
        def native_started():
            self.assertEqual(journal.durable, ['RUN_INTENT'])
            journal.on_append = _deny
        bridge.hooks['create.before'] = native_started
        bridge.hooks['abort_handoff.after'] = lambda: setattr(journal, 'on_append', None)
        app.dispatch('RUN')
        self.assertTrue(app.coordinator.facts['retirement_proven'])
        self.assertEqual(journal.durable[-1], 'RUN_RESULT')

    def test_journal_failure_consumes_run_and_preserves_original_exception(self):
        app, bridge, journal, unused = self.make()
        error = OSError('file or directory fsync lost')
        journal.failure = error
        app.dispatch('RUN')
        journal.failure = None
        app.dispatch('RUN')
        self.assertTrue(app.run_consumed)
        self.assertTrue(app.intent_started)
        self.assertFalse(app.native_may_exist)
        self.assertIs(app.first_error, error)
        self.assertEqual(bridge.actions, [])
        self.assertFalse(app.dispatch('EXIT'))

    def test_lost_start_method_cannot_repeat_run(self):
        app, bridge, journal, unused = self.make()
        error = KeyboardInterrupt('start method lookup lost')
        class Lost:
            def __getattribute__(self, name):
                if name == 'start':
                    raise error
                return getattr(original, name)
        original = app.coordinator
        app.coordinator = Lost()
        app.dispatch('RUN')
        app.dispatch('RUN')
        self.assertTrue(app.native_may_exist)
        self.assertIs(app.first_error, error)
        self.assertEqual(bridge.actions, [])
        self.assertEqual(journal.durable[0], 'RUN_INTENT')

    def test_unknown_commands_and_replay_never_dispatch_effects(self):
        app, bridge, journal, unused = self.make()
        for command in (None, 'create', 'RESUME', 'RUN 71', 'eval', 'ADVANCE', 'READ', 'CACHED'):
            self.assertFalse(app.dispatch(command))
        self.assertEqual(bridge.actions, [])
        self.assertFalse(app.dispatch('RUN'))
        self.assertEqual(bridge.actions, [])
        app, bridge, journal, unused = self.make()
        app.dispatch('RUN')
        actions = list(bridge.actions)
        app.dispatch('RUN')
        self.assertEqual(bridge.actions, actions)
        self.assertFalse(app.dispatch('EXIT'))

    def test_per_string_and_aggregate_bounds_before_dispatch(self):
        for cwd in (b'relative', b'/a\0b', b'/' + b'x' * 65534, b'/' + b'x' * 65535):
            with self.subTest(length=len(cwd)), self.assertRaises(ValueError):
                host._arguments(cwd, 9)
        for fd in (True, -1, 2**31):
            with self.assertRaises(ValueError):
                host._arguments(b'/private/tmp/child-cwd', fd)

    def test_observation_operation_refresh_and_exit_budgets_are_lifetime_caps(self):
        bridge = Bridge()
        bridge.exit_next = False
        app, bridge, journal, pauses = self.make(bridge)
        app.dispatch('RUN')
        self.assertEqual(pauses, [0.01] * 8)
        for unused in range(70):
            app.dispatch('ADVANCE')
            app.dispatch('READ')
        self.assertLessEqual(app.coordinator.operations, 32)
        self.assertEqual(app.coordinator.refreshes, 4)
        self.assertEqual(app.coordinator.observation_grants, 8)
        for unused in range(8):
            self.assertFalse(app.dispatch('EXIT'))
        self.assertEqual(app.proof_reads, 4)
        self.assertEqual(bridge.actions.count('create_suspended'), 1)


class Retention(Guarded):
    def test_top_level_logging_failure_after_creation_cannot_escape_retention(self):
        app, bridge, journal, unused = self.make()
        app.dispatch('RUN')  # Actual pure consumer/coordinator with fake roots only.
        self.assertTrue(app.native_may_exist)
        original = MemoryError('serve failed after creation')
        logging_error = MemoryError('failure transcript allocation failed')
        def failed_serve(): raise original
        def failed_transcript(kind): raise logging_error
        self.guard.replace(app, 'serve', failed_serve)
        self.guard.replace(app, 'transcript', failed_transcript)
        value = manifest()
        root = types.SimpleNamespace(fd=91, admitted=value['root'])
        io = types.SimpleNamespace(FamilyRoot=lambda path: root,
            Journal=lambda root: journal, Fifo=lambda root, pin: Fifo())
        modules = dict(zip(host.MODULES, (io, core, schema, consumer)))
        fake_sys = types.SimpleNamespace(platform='darwin', version_info=(3, 9),
            executable=host.PYTHON, argv=[host.__file__], modules={},
            flags=types.SimpleNamespace(isolated=1, no_site=1, no_user_site=1,
                                        dont_write_bytecode=1))
        self.guard.replace(host, 'sys', fake_sys)
        self.guard.replace(host, '_STARTED', False)
        self.guard.replace(os, 'getpid', lambda: 101)
        self.guard.replace(os, 'getuid', lambda: 12)
        self.guard.replace(host, '_environment', lambda unused: 'a' * 64)
        self.guard.replace(host, '_python', lambda: None)
        self.guard.replace(host, '_bootstrap', lambda *args: (value, {n + '.py': b'x' for n in host.MODULES}))
        self.guard.replace(host, '_source', lambda name, raw, path: modules[name])
        self.guard.replace(host, '_policies', lambda: None)
        self.guard.replace(host, '_target', lambda pin: None)
        self.guard.replace(host, '_resources', lambda root, manifest: (b'/private/tmp/child-cwd', 9))
        self.guard.replace(host, '_claim', lambda *args: None)
        self.guard.replace(host, '_native', lambda *args: (Binding, bridge))
        self.guard.replace(host, '_Host', lambda *args: app)
        with self.assertRaises(Park):
            host.main()
        self.assertIs(app.first_error, original)
        self.assertIs(app.attempt._sink, bridge.sink)
        self.assertIsNotNone(app.attempt._child)
        self.assertTrue(any(item is app for item in host._LIFETIME))

    def test_post_creation_journal_failure_keeps_all_roots_and_exceptions(self):
        app, bridge, journal, unused = self.make()
        native_error = KeyboardInterrupt('create reply lost')
        disk_error = OSError('result durability lost')
        def lost():
            journal.failure = disk_error
            raise native_error
        bridge.hooks['create.after'] = lost
        app.dispatch('RUN')
        self.assertIn(native_error, app.attempt._errors)
        self.assertIs(app.first_error, disk_error)
        self.assertIs(app.attempt._sink, bridge.sink)
        self.assertIsNotNone(app.attempt._child)
        self.assertIn(app.attempt, consumer._ATTEMPTS)
        self.assertTrue(any(item is app for item in host._LIFETIME))
        self.assertIs(app.coordinator.attempt, app.attempt)
        self.assertIs(app.binding, Binding)
        self.assertFalse(app.dispatch('EXIT'))
        self.assertNotIn('EXIT_READY', journal.durable)

    def test_controller_loss_and_empty_chunks_retain_with_finite_recovery(self):
        for failure in (None, OSError('controller read lost'), KeyboardInterrupt('interrupted')):
            fifo = Fifo([(b'a' * 32) + b' RUN\n'], failure)
            app, bridge, journal, pauses = self.make(fifo=fifo)
            with self.subTest(failure=failure), self.assertRaises(Park):
                app.serve()
            self.assertEqual(app.read_failures, 64)
            self.assertEqual(fifo.reads, 65)
            self.assertEqual(pauses.count(1), 63)
            self.assertEqual(bridge.actions.count('create_suspended'), 1)
            self.assertLessEqual(len(app.errors), host.MAX_ERRORS)
            if failure is not None:
                self.assertIs(app.first_error, failure)
            self.assertNotIn('EXIT_READY', journal.durable)

    def test_command_exhaustion_and_overflow_park_without_exit_or_native_action(self):
        fifo = Fifo([b'x\n' * 64, (b'a' * 32) + b' RUN\n'])
        app, bridge, journal, unused = self.make(fifo=fifo)
        with self.assertRaises(Park):
            app.serve()
        self.assertEqual(app.lines, 64)
        self.assertEqual(fifo.reads, 1)
        self.assertEqual(bridge.actions, [])
        commands = host_io.Commands('a' * 32)
        self.assertEqual(commands.feed(b'x' * 128), ())
        self.assertEqual(commands.feed(b'x\n'), (None,))

    def test_no_growth_or_native_actions_after_recovery_exhaustion(self):
        app, bridge, journal, unused = self.make()
        app.read_failures = 64
        sizes = (len(app.errors), len(journal.events), len(bridge.actions))
        with self.assertRaises(Park):
            app.serve()
        self.assertEqual((len(app.errors), len(journal.events), len(bridge.actions)), sizes)

    def test_failed_recovery_delays_are_bounded_and_original_errors_retained(self):
        app, bridge, journal, unused = self.make(fifo=Fifo(failure=OSError('read failed')))
        delay_error = KeyboardInterrupt('sleep interrupted')
        def broken_delay(seconds):
            raise delay_error
        app.pause = broken_delay
        with self.assertRaises(Park):
            app.serve()
        self.assertEqual(app.read_failures, 64)
        self.assertEqual(sum(error is delay_error for where, error in app.errors), 63)
        self.assertEqual(bridge.actions, [])


class ExitProof(Guarded):
    def test_serve_returns_only_after_durable_fresh_exit_proof(self):
        fifo = Fifo([(b'a' * 32) + b' RUN\n', (b'a' * 32) + b' EXIT\n'])
        app, bridge, journal, unused = self.make(fifo=fifo)
        seen = []
        def before_durability(event):
            if event['kind'] == 'EXIT_READY':
                self.assertEqual(app.proof_reads, 1)
                self.assertTrue(event['proof']['facts']['fresh'])
                self.assertTrue(event['proof']['facts']['retirement_proven'])
                self.assertEqual(len(event['proof']['registry']), 2)
                self.assertNotIn('EXIT_READY', journal.durable)
                seen.append(event)
        journal.on_append = before_durability
        self.assertEqual(app.serve(), 0)
        self.assertEqual(len(seen), 1)
        self.assertEqual(journal.durable[-1], 'EXIT_READY')
        self.assertEqual(fifo.reads, 2)

    def test_both_actual_shaped_retirements_allow_exit_and_preserve_positive_errno(self):
        for no_creation in (False, True):
            app, bridge, journal, unused = self.make(Bridge(no_creation=no_creation))
            app.dispatch('RUN')
            before = len(bridge.calls)
            self.assertTrue(app.dispatch('EXIT'))
            self.assertEqual(journal.durable[-1], 'EXIT_READY')
            proof = journal.events[-1]['proof']
            self.assertTrue(proof['facts']['fresh'])
            self.assertEqual(len(proof['snapshots']), 2)
            self.assertEqual(len(proof['registry']), 2)
            self.assertGreater(len(bridge.calls), before)
            self.assertEqual(proof['facts']['terminal']['final_report']['system_errno'],
                             9 if no_creation else 23)
            if not no_creation:
                self.assertEqual(proof['facts']['terminal']['wait_errno'], 30)

    def test_empty_never_started_and_preparation_rejected_proofs(self):
        app, bridge, journal, unused = self.make()
        self.assertTrue(app.dispatch('EXIT'))
        self.assertEqual(journal.events[-1]['proof']['retirement'], 'no_attempt')
        self.assertEqual(bridge.calls, ['pending', 'history', 'pending', 'history'])
        self.assertEqual(bridge.actions, [])
        app, bridge, journal, unused = self.make(Bridge(reject=True))
        app.dispatch('RUN')
        self.assertTrue(app.dispatch('EXIT'))
        self.assertEqual(journal.events[-1]['proof']['facts']['retirement'], 'preparation_rejected')
        self.assertEqual(bridge.actions, ['create_suspended'])

    def test_exit_proof_is_new_and_failure_to_fsync_refuses_exit(self):
        app, bridge, journal, unused = self.make()
        app.dispatch('RUN')
        refreshes, operations = app.coordinator.refreshes, app.coordinator.operations
        error = OSError('EXIT_READY directory fsync failed')
        journal.failure = error
        self.assertFalse(app.dispatch('EXIT'))
        self.assertEqual(app.proof_reads, 1)
        self.assertEqual(app.coordinator.refreshes, refreshes)
        self.assertEqual(app.coordinator.operations, operations)
        self.assertIs(app.first_error, error)
        self.assertNotIn('EXIT_READY', journal.durable)

    def test_cached_partial_untransferred_pending_foreign_or_changed_roots_refuse(self):
        mutations = (
            lambda app, b: setattr(b, 'child', Child()),
            lambda app, b: b.sink.value.update(transferred=0),
            lambda app, b: b.sink.value.update(certificate_matches=0),
            lambda app, b: b.sink.value.update(generation=42),
            lambda app, b: b.sink.value['final_report'].update(resources_closed=0),
            lambda app, b: b.sink.value['final_report'].update(resume_entered=1),
            lambda app, b: b.sink.value['final_report']['abort'].update(phase=6),
            lambda app, b: setattr(app.attempt, 'read', app.attempt.cached),
            lambda app, b: setattr(b, 'history', lambda: (object(),)),
        )
        for mutate in mutations:
            app, bridge, journal, unused = self.make()
            app.dispatch('RUN')
            mutate(app, bridge)
            with self.subTest(mutation=mutate):
                self.assertFalse(app.dispatch('EXIT'))
                self.assertNotIn('EXIT_READY', journal.durable)

    def test_independent_proof_refuses_changed_registry_after_coherent_read(self):
        app, bridge, journal, unused = self.make()
        app.dispatch('RUN')
        read = app.attempt.read
        foreign = Sink(Bridge())
        def fresh_then_changed():
            facts = read()
            bridge.history = lambda: (foreign,)
            return facts
        app.attempt.read = fresh_then_changed
        self.assertFalse(app.dispatch('EXIT'))
        self.assertTrue(any(type(reply) is tuple and reply == (foreign,) for reply in app.replies))
        self.assertNotIn('EXIT_READY', journal.durable)

    def test_registry_change_during_last_independent_snapshot_refuses_exit(self):
        app, bridge, journal, unused = self.make()
        app.dispatch('RUN')
        read = app.attempt.read
        foreign = Sink(Bridge())
        def fresh_then_changes():
            facts = read()
            snapshot, count = bridge.sink.snapshot, [0]
            def changed_snapshot():
                reply = snapshot()
                count[0] += 1
                if count[0] == 2:
                    bridge.history = lambda: (foreign,)
                return reply
            bridge.sink.snapshot = changed_snapshot
            return facts
        app.attempt.read = fresh_then_changes
        self.assertFalse(app.dispatch('EXIT'))
        self.assertNotIn('EXIT_READY', journal.durable)

    def test_independent_snapshot_loss_retains_original_error_and_partial_proof(self):
        app, bridge, journal, unused = self.make()
        app.dispatch('RUN')
        read = app.attempt.read
        error = KeyboardInterrupt('independent snapshot reply lost')
        def fresh_then_lost():
            facts = read()
            bridge.hooks['snapshot'] = error
            return facts
        app.attempt.read = fresh_then_lost
        self.assertFalse(app.dispatch('EXIT'))
        self.assertIs(app.first_error, error)
        self.assertEqual(len(app.proofs), 1)
        self.assertIs(app.attempt._sink, bridge.sink)


class BootstrapOrder(Guarded):
    def test_actual_verified_sources_bootstrap_without_script_search_path(self):
        saved = {name: sys.modules.pop(name) for name in host.MODULES}
        saved_path = sys.path
        try:
            sys.path = []
            loaded = []
            for name in host.MODULES:
                loaded.append(host._source(name, _SOURCE_BYTES[name + '.py'],
                                           _SCRIPTS + '/' + name + '.py'))
            self.assertEqual([m.__name__ for m in loaded], list(host.MODULES))
            self.assertTrue(all(type(m) is types.ModuleType and sys.modules[m.__name__] is m
                                for m in loaded))
            self.assertIs(loaded[-1].BridgeProtocolError, loaded[-2].BridgeProtocolError)
            with self.assertRaisesRegex(ValueError, 'reload'):
                host._source(host.MODULES[0], _SOURCE_BYTES[host.MODULES[0] + '.py'], 'unused')
        finally:
            sys.path = saved_path
            sys.modules.update(saved)

    def test_claim_requires_file_and_directory_durability_and_exclusive_creation(self):
        events = []
        root = types.SimpleNamespace(fd=8, validate=lambda: events.append('validate'))
        self.guard.replace(os, 'open', lambda name, flags, mode, **kw:
                           events.append(('open', name, flags, mode, kw)) or 9)
        self.guard.replace(os, 'write', lambda fd, raw: events.append(('write', fd)) or len(raw))
        self.guard.replace(os, 'fchmod', lambda fd, mode: events.append(('mode', fd, mode)))
        self.guard.replace(os, 'fsync', lambda fd: events.append(('fsync', fd)))
        self.guard.replace(os, 'close', lambda fd: events.append(('close', fd)))
        host._claim(root, host_io, 'a' * 64, 80)
        self.assertEqual(events[-3:], [('fsync', 9), ('close', 9), ('fsync', 8)])
        self.assertTrue(events[1][2] & os.O_EXCL)
        self.assertTrue(events[1][2] & os.O_NOFOLLOW)

    def test_setup_fails_before_native_loading_on_bad_family_or_manifest(self):
        self.guard.replace(os, 'lstat', lambda path: file_stat(st_mode=stat.S_IFDIR | 0o700))
        self.guard.replace(os, 'fstat', lambda fd: file_stat(st_mode=stat.S_IFDIR | 0o700))
        self.guard.replace(os, 'open', lambda *args, **kwargs: 8)
        self.guard.replace(os, 'listdir', lambda fd: list(host.FILES) + ['manifest.json', 'control.fifo', 'child-cwd'])
        self.guard.replace(host, '_file_at', lambda *args, **kwargs: (b'{}', file_stat()))
        for path in ('/tmp/sims-suspended-true-test', '/private/tmp/unadmitted',
                     '/private/tmp/sims-suspended-true-test'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                host._bootstrap(path, 'a' * 64, 12)
        self.assertNotIn('_window_owned_child_bridge', sys.modules)

    def test_sibling_identity_rejection_precedes_any_source_execution(self):
        s = file_stat(st_mode=stat.S_IFDIR | 0o700)
        self.guard.replace(os, 'lstat', lambda path: s)
        self.guard.replace(os, 'fstat', lambda fd: s)
        self.guard.replace(os, 'open', lambda *args, **kwargs: 8)
        self.guard.replace(os, 'listdir', lambda fd: list(host.FILES))
        with self.assertRaisesRegex(ValueError, 'siblings'):
            host._bootstrap('/private/tmp/sims-suspended-true-test', 'a' * 64, 12)

    def test_fake_binding_fixture_has_no_native_authentication(self):
        self.assertTrue(Binding.Bridge.__flags__ & (1 << 9))  # Heap Python fixture, never a C type.
        self.assertNotEqual(Binding.Bridge.__module__, '_window_owned_child_bridge')
        with self.assertRaises(AssertionError):
            importlib.machinery.ExtensionFileLoader('_window_owned_child_bridge', '/forbidden.so').create_module(None)
        self.assertNotIn('_window_owned_child_bridge', sys.modules)


if __name__ == '__main__':
    unittest.main()
