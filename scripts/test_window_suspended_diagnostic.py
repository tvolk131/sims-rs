"""Guarded ACTUAL-MODULE tests, authored UNRUN; no native authenticity claims.

Parent-only, separate interpreter filters (ALL UNRUN)::

    python3 -B -m unittest discover -s scripts -p test_window_suspended_diagnostic.py
    PYTHONPATH=scripts python3 -B -m unittest test_window_suspended_diagnostic.Admission
    PYTHONPATH=scripts python3 -B -m unittest test_window_suspended_diagnostic.Recovery
    PYTHONPATH=scripts python3 -B -m unittest test_window_suspended_diagnostic.AbortProtocol

No AST rewriting, native extension, OS child, clock, scheduler or IPC is used.
Guards surround actual imports and every test; runner clocks remain outside the
guard. The permanent audit hook is active only inside a guard. Do not combine
this interpreter with suites that install incompatible permanent guard hooks.
These doubles exercise the protocol seam, never the platform or actual C ABI.
"""

import builtins
import copy
import importlib.machinery
import os
import pickle
import sys
import time
import unittest


_ACTIVE_GUARD = None
_REAL_IMPORT = builtins.__import__
_FORBIDDEN = frozenset((
    'subprocess', '_posixsubprocess', 'ctypes', '_ctypes', 'socket', '_socket',
    'multiprocessing', 'psutil', 'signal', 'resource', 'platform', 'select',
    'selectors', 'asyncio', 'threading', '_thread', '_window_owned_child_bridge',
))


def _deny(*arguments, **keywords):
    raise AssertionError('Forbidden native/process/clock/IPC/I/O in pure diagnostic tests')


def _guard_import(name, globals=None, locals=None, fromlist=(), level=0):
    if name.split('.')[0] in _FORBIDDEN:
        _deny()
    existing = sys.modules.get(name)
    filename = getattr(existing, '__file__', '')
    if type(filename) is str and filename.endswith(('.so', '.dylib', '.pyd')):
        _deny()
    return _REAL_IMPORT(name, globals, locals, fromlist, level)


def _audit(event, arguments):
    guard = _ACTIVE_GUARD
    if guard is None:
        return
    if event.startswith(('subprocess.', 'ctypes.', 'socket.', 'os.exec', 'os.spawn',
                         'os.posix_spawn', 'os.fork', 'os.kill', 'os.system',
                         'os.startfile', 'os.putenv', 'os.unsetenv')):
        _deny()
    if event == 'import' and len(arguments) > 1:
        filename = arguments[1]
        if type(filename) is str and filename.endswith(('.so', '.dylib', '.pyd')):
            _deny()
    if not guard.source_import and event in ('open', 'os.listdir', 'os.scandir',
                                             'os.chdir', 'os.remove', 'os.rename', 'os.mkdir'):
        _deny()


class Guards:
    def __init__(self, source_import=False):
        self.source_import = source_import
        self.saved = []

    def replace(self, target, name, value):
        if hasattr(target, name):
            self.saved.append((target, name, getattr(target, name)))
            setattr(target, name, value)

    def __enter__(self):
        global _ACTIVE_GUARD
        assert _ACTIVE_GUARD is None
        _ACTIVE_GUARD = self
        self.replace(builtins, '__import__', _guard_import)
        self.replace(builtins, 'open', _deny)
        self.replace(importlib.machinery.ExtensionFileLoader, 'create_module', _deny)
        self.replace(importlib.machinery.ExtensionFileLoader, 'exec_module', _deny)
        for name in tuple(vars(os)):
            if name.startswith(('exec', 'spawn')) or name in (
                'fork', 'forkpty', 'posix_spawn', 'posix_spawnp', 'system', 'popen',
                'wait', 'wait3', 'wait4', 'waitid', 'waitpid', 'kill', 'killpg',
                'getpid', 'getppid', 'getuid', 'geteuid', 'getgid', 'getegid',
                'getpgid', 'getsid', 'setsid', 'setpgid', 'pipe', 'pipe2', 'open',
                'read', 'write', 'close', 'stat', 'lstat', 'fstat', 'statvfs', 'fstatvfs',
            ):
                self.replace(os, name, _deny)
        for name in ('sleep', 'time', 'time_ns', 'monotonic', 'monotonic_ns',
                     'perf_counter', 'perf_counter_ns', 'process_time', 'process_time_ns',
                     'thread_time', 'thread_time_ns', 'clock_gettime', 'clock_gettime_ns'):
            self.replace(time, name, _deny)
        return self

    def __exit__(self, *error):
        global _ACTIVE_GUARD
        for target, name, value in reversed(self.saved):
            setattr(target, name, value)
        _ACTIVE_GUARD = None


sys.addaudithook(_audit)
with Guards(source_import=True):
    import window_suspended_diagnostic as diagnostic
    import window_owned_child_bridge as schema


def data(value):
    if type(value) is dict:
        return {key: data(item) for key, item in value.items()}
    if type(value) is tuple:
        return tuple(data(item) for item in value)
    return value


def record(**changes):
    result = dict.fromkeys(schema.HEADER_FIELDS + schema.RECORD_FIELDS, 0)
    result.update(bytes=200, version=1, pid=71, pgid=71, uid=12, ppid=7, sid=71,
                  state=1, uid_evidence=1, sigchld_default=1, pid_bytes=4, capacity=2)
    result.update(changes)
    return result


def registration(**changes):
    value = dict.fromkeys(schema._REGISTRATION_SCALARS, 0)
    value.update(entered=1, returned=1, raw_siginfo_bytes=16,
                 idtype=1, target_pid=71, options=123)
    value.update(changes)
    return value


def observation(**changes):
    value = dict.fromkeys(schema._ABORT_OBSERVATION_SCALARS, 0)
    value.update(changes)
    return value


def child_report(no_creation=False):
    abort = dict.fromkeys(schema._ABORT_SCALARS, 0)
    abort.update(max_entries=16, max_observations=8, origin_ns=1000,
                 deadline_ns=1300, now_ns=1000, exit_observation_index=-1,
                 record=dict.fromkeys(schema.HEADER_FIELDS + schema.RECORD_FIELDS, 0),
                 trace=(), observations=tuple(observation() for _ in range(9)))
    value = dict.fromkeys(schema._CHILD_SCALARS, 0)
    value.update(generation=41, state=4, pid_valid=1, registered=1,
                 registration_bsd_status=4, registration_pid=71, registration_ppid=7,
                 registration_pgid=71, registration_uid=12, registration_sid=71,
                 spawn_entered=1, spawn_returned=1, spawn_pid_raw=71,
                 launch_entries=2, launch_fact_count=1, launch_deadline_ns=1100,
                 resources_closed=1, max_cleanup_entries=5,
                 sigchld_preflight_entered=1, sigchld_preflight_returned=1,
                 sigchld_preflight_policy_valid=1, sigchld_preflight_default=1,
                 sigchld_preflight_accepted=1, record=record(), abort=abort,
                 registration_observation=registration(), launch_facts=((1, 1, 1, 0, 0),))
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
    def __init__(self, bridge, report):
        self.bridge = bridge
        self.reg_raw = b'R' * 16
        self.raws = tuple(bytes((65 + i,)) * 16 for i in range(9))
        self.value = dict.fromkeys(schema._TERMINAL_SCALARS, 0)
        self.value.update(generation=41, rooted=1, known_no_creation=int(bridge.no_creation),
                          kind=1 if bridge.no_creation else 0, raw_siginfo=b'\0' * 16,
                          decoded_record=dict.fromkeys(schema.HEADER_FIELDS + schema.RECORD_FIELDS, 0),
                          final_report=report, registration_observation=report['registration_observation'])

    def snapshot(self):
        self.bridge.event('snapshot')
        return data(self.value)

    def registration_observation(self):
        self.bridge.event('registration_raw')
        slot = self.value['registration_observation']
        assert slot['entered'] == slot['returned'] == 1
        return self.reg_raw

    def abort_observation(self, index):
        self.bridge.event('abort_raw:' + str(index))
        slot = self.value['final_report']['abort']['observations'][index]
        assert slot['entered'] == slot['returned'] == 1
        return self.raws[index]


class Bridge:
    def __init__(self, no_creation=False):
        self.no_creation = no_creation
        self.child = self.sink = None
        self.calls, self.actions, self.hooks = [], [], {}
        self.arguments = None
        self.exit_next = False
        self.on_create = None
        self.returned_override = None

    def fail(self, event, error=None):
        self.hooks[event] = error if error is not None else KeyboardInterrupt(event)

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
        self.event('terminal')
        assert child is self.child
        return self.sink

    def child_snapshot(self, child):
        self.event('child_snapshot')
        assert child is self.child
        return data(self.sink.value['final_report'])

    def create_suspended(self, *arguments):
        self.actions.append('create_suspended')
        self.arguments = arguments
        self.event('create.before')
        assert self.child is None and self.sink is None
        self.child = Child()
        self.sink = Sink(self, child_report(self.no_creation))
        if self.on_create is not None:
            self.on_create(self.sink.value)
        self.event('create.after')
        return self.child if self.returned_override is None else self.returned_override

    def start_action(self, name, child):
        self.actions.append(name)
        self.event(name + '.before')
        assert child is self.child
        return self.sink.value['final_report']['abort']

    def finish_action(self, name):
        self.event(name + '.after')
        return data(self.sink.value['final_report'])

    def abort_begin(self, child):
        a = self.start_action('abort_begin', child)
        a.update(started=1, phase=1, entries=a['entries'] + 1)
        self.event('abort_begin.entered')
        a.update(phase=3, last_operation=1, last_rc=0, kill_entered=1, kill_returned=1,
                 record=record(no_signals=1, signal_attempted=1),
                 trace=((1, 1, 1, 0, 0),), trace_count=1)
        a['observations'][0].update(entered=1, returned=1, raw_siginfo_bytes=16)
        return self.finish_action('abort_begin')

    def abort_observe(self, child):
        a = self.start_action('abort_observe', child)
        a.update(entries=a['entries'] + 1, observation_count=a['observation_count'] + 1)
        self.event('abort_observe.entered')
        index = a['observation_count']
        a['observations'][index].update(entered=1, returned=1, raw_siginfo_bytes=16)
        a.update(last_operation=2, last_rc=0)
        if self.exit_next:
            a['observations'][index].update(pid=71, uid=9999, code=2, status=9)
            a.update(phase=4, exit_valid=1, exit_observation_index=index,
                     record=record(state=2, no_signals=1, signal_attempted=1,
                                   exit_kind=2, exit_status=9, returncode=-9, uid_evidence=2))
        return self.finish_action('abort_observe')

    def abort_reap(self, child):
        a = self.start_action('abort_reap', child)
        t = self.sink.value
        a.update(entries=a['entries'] + 1, phase=5)
        t.update(reap_attempted=1, wait_entered=1)
        self.event('abort_reap.entered')
        a['record'].update(reap_attempted=1, reaped=1)
        t.update(kind=3, wait_returned=1, wait_rc=0, wait_errno=0, wait_pid=71,
                 wait_uid=9999, wait_code=2, wait_status=9, consumed=1, decoded=1,
                 certificate_matches=1, decoded_record=data(a['record']), raw_siginfo=b'W' * 16)
        a.update(phase=6, last_operation=3, last_rc=0)
        return self.finish_action('abort_reap')

    def abort_handoff(self, child, sink):
        a = self.start_action('abort_handoff', child)
        assert sink is self.sink
        a.update(entries=a['entries'] + 1, phase=7, last_operation=4, last_rc=0)
        sink.value['transferred'] = 1
        self.child = None
        self.event('abort_handoff.after')
        return sink

    def handoff(self, child, sink):
        self.start_action('handoff', child)
        assert sink is self.sink and self.no_creation
        sink.value['final_report']['state'] = 11
        sink.value['transferred'] = 1
        self.child = None
        self.event('handoff.after')
        return sink

    # A call to ANY ordinary entry point is a test failure before effects.
    create = resume = publish = cleanup_begin = child_observe = child_signal = _deny
    child_reap = signal = kill = wait = poll = release = reset = _deny


class Binding:
    Bridge = Bridge
    Child = Child
    TerminalSink = Sink


OPTIONS = dict(executable=b'/usr/bin/true', argv=(b'/usr/bin/true',), env=(b'LANG=C',),
               cwd=b'/private/parent-admitted', stdio=(8, 9, 10), launch_ns=100,
               cleanup_ns=200, grace_ns=0, max_entries=5)


class GuardedCase(unittest.TestCase):
    def setUp(self):
        self.guard = Guards()
        self.guard.__enter__()
        self.addCleanup(self.guard.__exit__)

    def attempt(self, bridge=None, **changes):
        bridge = bridge if bridge is not None else Bridge()
        options = dict(OPTIONS)
        options.update(changes)
        return bridge, diagnostic.Attempt(Binding, bridge, **options)

    def suspended(self, bridge=None):
        bridge, attempt = self.attempt(bridge)
        value = attempt.create_once()
        self.assertTrue(value['fresh'])
        self.assertFalse(value['retirement_proven'])
        return bridge, attempt

    def ready_to_reap(self, bridge=None):
        bridge, attempt = self.suspended(bridge)
        self.assertEqual(attempt.step()['action'], 'abort_begin')
        bridge.exit_next = True
        self.assertEqual(attempt.step(observation=True)['action'], 'abort_observe')
        return bridge, attempt

    def reaped(self, bridge=None):
        bridge, attempt = self.ready_to_reap(bridge)
        self.assertEqual(attempt.step()['action'], 'abort_reap')
        return bridge, attempt


class Admission(GuardedCase):
    def test_exact_arguments_forwarded_once(self):
        bridge, attempt = self.suspended()
        self.assertEqual(bridge.arguments, tuple(OPTIONS[key] for key in
                         ('executable', 'argv', 'env', 'cwd', 'stdio', 'launch_ns',
                          'cleanup_ns', 'grace_ns', 'max_entries')))
        for i in (0, 1, 2, 3, 4):
            self.assertIs(bridge.arguments[i], tuple(OPTIONS.values())[i])
        with self.assertRaises(schema.BridgeProtocolError):
            attempt.create_once()
        self.assertEqual(bridge.actions, ['create_suspended'])
        with self.assertRaises(schema.BridgeProtocolError):
            diagnostic.Attempt(Binding, bridge, **OPTIONS)

    def test_pre_admission_refusal_has_no_bridge_or_os_io(self):
        invalid = (
            {'executable': b'/bin/true'}, {'argv': (b'/usr/bin/true', b'bad')},
            {'argv': [b'/usr/bin/true']}, {'env': [b'A=B']}, {'env': (b'=bad',)},
            {'env': (b'A=\0',)}, {'cwd': b'relative'}, {'stdio': (False, 1, 2)},
            {'launch_ns': True}, {'cleanup_ns': 0}, {'grace_ns': 201}, {'max_entries': 513},
        )
        for changes in invalid:
            with self.subTest(changes=changes):
                bridge = Bridge()
                with self.assertRaises(schema.BridgeProtocolError):
                    self.attempt(bridge, **changes)
                self.assertEqual(bridge.calls, [])
                self.assertEqual(bridge.actions, [])

    def test_binding_access_rooted_before_interruption(self):
        bridge = Bridge()
        error = KeyboardInterrupt('binding read lost')
        class InterruptedBinding:
            @property
            def Bridge(self):
                self.seen = diagnostic._ATTEMPTS[-1]
                raise error
        binding = InterruptedBinding()
        with self.assertRaises(KeyboardInterrupt):
            diagnostic.Attempt(binding, bridge, **OPTIONS)
        self.assertIs(binding.seen._binding, binding)
        self.assertIs(binding.seen._bridge, bridge)
        self.assertIs(binding.seen._errors[0], error)
        self.assertEqual(bridge.calls, [])

    def test_binding_exact_distinct_nonscalar_types(self):
        class Bad:
            Bridge = Bridge
            Child = int
            TerminalSink = Sink
        class Same:
            Bridge = Bridge
            Child = Sink
            TerminalSink = Sink
        for binding in (Bad, Same):
            with self.assertRaises(schema.BridgeProtocolError):
                diagnostic.Attempt(binding, Bridge(), **OPTIONS)
        class SubBridge(Bridge):
            pass
        with self.assertRaises(schema.BridgeProtocolError):
            diagnostic.Attempt(Binding, SubBridge(), **OPTIONS)

    def test_no_copies_reinitialization_or_authority(self):
        bridge, attempt = self.suspended()
        for operation in (copy.copy, copy.deepcopy, pickle.dumps):
            with self.assertRaises(TypeError):
                operation(attempt)
        with self.assertRaises(schema.BridgeProtocolError):
            attempt.__init__(Binding, bridge, **OPTIONS)
        self.assertFalse(attempt.native_qualified)
        self.assertFalse(attempt.continue_authorized)
        for name in ('reset', 'rearm', 'release', 'resume', 'publish', 'poll', 'returncode'):
            self.assertFalse(hasattr(attempt, name))
        with self.assertRaises(schema.Unsupported):
            schema.production_child_execution()

    def test_guards_forbid_execution_native_clock_and_ipc(self):
        for operation in (time.monotonic, time.sleep, os.getpid, os.waitpid, os.kill,
                          os.posix_spawn, os.pipe, builtins.open):
            with self.assertRaises(AssertionError):
                operation()
        for name in _FORBIDDEN:
            with self.assertRaises(AssertionError):
                builtins.__import__(name)


class Recovery(GuardedCase):
    def test_preparation_rejection_is_not_healthy_completion(self):
        bridge, attempt = self.attempt()
        original = RuntimeError('preparation failed before root')
        bridge.fail('create.before', original)
        result = attempt.create_once()
        self.assertEqual(result['retirement'], 'preparation_rejected')
        self.assertTrue(result['retirement_proven'])
        self.assertFalse(result['healthy'])
        self.assertEqual(bridge.actions, ['create_suspended'])
        self.assertIs(attempt._errors[0], original)
        self.assertFalse(attempt.cached()['retirement_proven'])

    def test_create_lost_reply_recovers_exact_roots(self):
        bridge, attempt = self.attempt()
        original = KeyboardInterrupt('create reply lost')
        bridge.fail('create.after', original)
        result = attempt.create_once()
        self.assertTrue(result['fresh'])
        self.assertIs(attempt._child, bridge.child)
        self.assertIs(attempt._sink, bridge.sink)
        self.assertEqual(result['terminal']['generation'], 41)
        self.assertIs(attempt._errors[0], original)
        self.assertEqual(attempt.step()['action'], 'abort_begin')

    def test_each_recovery_read_can_lose_reply_without_losing_roots(self):
        for event in ('pending', 'history', 'snapshot', 'terminal', 'child_snapshot', 'registration_raw'):
            with self.subTest(event=event):
                bridge, attempt = self.suspended()
                child, sink = bridge.child, bridge.sink
                bridge.fail(event)
                self.assertFalse(attempt.read()['retirement_proven'])
                self.assertTrue(attempt.read()['fresh'])
                self.assertIs(attempt._child, child)
                self.assertIs(attempt._sink, sink)
                self.assertTrue(attempt.cached()['failures'])

    def test_interrupted_first_raw_read_keeps_partial_scalar_snapshot(self):
        bridge, attempt = self.attempt()
        bridge.fail('registration_raw')
        value = attempt.create_once()
        self.assertFalse(value['fresh'])
        self.assertIsNone(value['terminal'])
        self.assertEqual(value['retained_snapshot']['generation'], 41)
        self.assertEqual(value['retained_snapshot']['final_report']['state'], 4)
        self.assertFalse(value['retirement_proven'])
        value['retained_snapshot']['final_report']['state'] = 11
        self.assertEqual(attempt.cached()['retained_snapshot']['final_report']['state'], 4)
        self.assertTrue(attempt.read()['fresh'])

    def test_interrupted_preflight_cannot_create_or_adopt(self):
        for event in ('pending', 'history'):
            bridge, attempt = self.attempt()
            bridge.fail(event)
            self.assertFalse(attempt.create_once()['fresh'])
            self.assertFalse(attempt.read()['retirement_proven'])
            with self.assertRaises(schema.BridgeProtocolError):
                attempt.create_once()
            self.assertEqual(bridge.actions, [])

    def test_nonempty_preflight_never_adopted(self):
        bridge = Bridge()
        bridge.child = Child()
        bridge.sink = Sink(bridge, child_report())
        bridge, attempt = self.attempt(bridge)
        self.assertFalse(attempt.create_once()['retirement_proven'])
        self.assertIsNone(attempt._child)
        self.assertIsNone(attempt._sink)
        self.assertEqual(bridge.actions, [])
        self.assertFalse(attempt.step()['retirement_proven'])

    def test_replaced_child_sink_generation_or_mapping_never_adopted(self):
        for replacement in ('child', 'sink', 'generation', 'child_mapping', 'history_missing'):
            with self.subTest(replacement=replacement):
                bridge, attempt = self.suspended()
                original_child, original_sink = attempt._child, attempt._sink
                if replacement == 'child':
                    bridge.child = Child()
                elif replacement == 'sink':
                    bridge.sink = Sink(bridge, child_report())
                elif replacement == 'generation':
                    bridge.sink.value['generation'] += 1
                elif replacement == 'child_mapping':
                    bridge.hooks['child_snapshot'] = lambda: bridge.sink.value['final_report'].update(rc=3)
                else:
                    bridge.sink = None
                self.assertFalse(attempt.step()['retirement_proven'])
                self.assertEqual(bridge.actions, ['create_suspended'])
                self.assertIs(attempt._child, original_child)
                self.assertIs(attempt._sink, original_sink)

    def test_unentered_vs_failed_registration_raw(self):
        for entered, returned in ((0, 0), (1, 0), (1, 1)):
            bridge = Bridge()
            def setup(t):
                t['final_report'].update(state=8, rc=3, blocked=1, registered=0,
                                         launch_failure_mask=1 << 6)
                t['registration_observation'].update(entered=entered, returned=returned,
                                                      rc=-1, system_errno=5)
            bridge.on_create = setup
            bridge, attempt = self.suspended(bridge)
            raw = attempt.cached()['registration_raw']
            self.assertEqual(raw['status'], 'returned' if returned else 'unentered')
            self.assertEqual(bridge.calls.count('registration_raw'), 1 if returned else 0)
            self.assertIsNone(attempt.step()['action'])
            if returned:
                self.assertEqual(raw['bytes'], b'R' * 16)
                self.assertEqual(raw['scalar']['rc'], -1)

    def test_raw_length_and_immutability_and_cached_reads_after_loss(self):
        bridge, attempt = self.suspended()
        facts = attempt.read()
        facts['terminal']['final_report']['abort']['deadline_ns'] = 99999
        facts['registration_raw']['scalar']['rc'] = -9
        self.assertEqual(attempt.cached()['terminal']['final_report']['abort']['deadline_ns'], 1300)
        self.assertEqual(attempt.cached()['registration_raw']['scalar']['rc'], 0)
        before = len(bridge.calls)
        attempt.cached()
        self.assertEqual(len(bridge.calls), before)
        bridge.fail('snapshot')
        result = attempt.read()
        self.assertFalse(result['fresh'])
        self.assertEqual(result['retained_registration_raw'][1], b'R' * 16)
        bridge.sink.reg_raw = b'wrong length'
        self.assertFalse(attempt.read()['fresh'])
        self.assertIsNone(attempt.step()['action'])

    def test_same_length_raw_rewrite_and_registration_slot_disagreement(self):
        for kind in ('bytes', 'scalar'):
            bridge, attempt = self.suspended()
            if kind == 'bytes':
                bridge.sink.reg_raw = b'X' * 16
            else:
                bridge.sink.value['registration_observation'] = registration(rc=-1)
            self.assertFalse(attempt.read()['fresh'])
            self.assertTrue(attempt.cached()['protocol_faults'])
            self.assertEqual(attempt.cached()['retained_registration_raw'][1], b'R' * 16)
            self.assertIsNone(attempt.step()['action'])

    def test_numeric_creation_result_is_not_an_owned_child(self):
        bridge, attempt = self.attempt()
        bridge.returned_override = 71
        value = attempt.create_once()
        self.assertTrue(value['protocol_faults'])
        self.assertIs(attempt._child, bridge.child)  # same-Bridge recovery retains the real root
        self.assertIsNone(attempt.step()['action'])
        self.assertFalse(value['healthy'])


class AbortProtocol(GuardedCase):
    def test_successful_spawn_raw_errno_does_not_block_abort(self):
        bridge = Bridge()
        bridge.on_create = lambda t: t['final_report'].update(spawn_errno=17)
        bridge, attempt = self.reaped(bridge)
        self.assertEqual(attempt.step()['action'], 'abort_handoff')
        value = attempt.read()
        self.assertEqual(value['retirement'], 'kind3_unresumed_abort')
        self.assertEqual(value['terminal']['final_report']['spawn_rc'], 0)
        self.assertEqual(value['terminal']['final_report']['spawn_errno'], 17)
        self.assertEqual(bridge.actions.count('abort_begin'), 1)
        self.assertEqual(bridge.actions.count('abort_reap'), 1)

    def test_successful_sigchld_raw_errno_does_not_block_abort(self):
        bridge = Bridge()
        bridge.on_create = lambda t: t['final_report'].update(sigchld_preflight_errno=23)
        bridge, attempt = self.reaped(bridge)
        self.assertEqual(attempt.step()['action'], 'abort_handoff')
        value = attempt.read()
        self.assertTrue(value['retirement_proven'])
        report = value['terminal']['final_report']
        self.assertEqual(report['sigchld_preflight_rc'], 0)
        self.assertEqual(report['sigchld_preflight_errno'], 23)
        self.assertEqual(bridge.actions.count('abort_begin'), 1)

    def test_successful_consuming_wait_raw_errno_survives_lost_reply_and_transfer(self):
        bridge, attempt = self.ready_to_reap()
        error = KeyboardInterrupt('reap reply lost after successful wait')
        def lost_success():
            bridge.sink.value['wait_errno'] = 29
            raise error
        bridge.hooks['abort_reap.after'] = lost_success
        self.assertEqual(attempt.step()['action'], 'abort_reap')
        self.assertIn(error, attempt._errors)
        self.assertEqual(attempt.step()['action'], 'abort_handoff')
        value = attempt.read()
        self.assertTrue(value['retirement_proven'])
        self.assertTrue(value['failures'])
        self.assertEqual(value['terminal']['wait_rc'], 0)
        self.assertEqual(value['terminal']['wait_errno'], 29)
        self.assertEqual(value['terminal']['raw_siginfo'], b'W' * 16)
        self.assertEqual(bridge.actions.count('abort_reap'), 1)
        self.assertEqual(bridge.actions.count('abort_handoff'), 1)

    def test_one_action_per_step_and_scheduled_observations(self):
        bridge, attempt = self.suspended()
        count = len(bridge.actions)
        result = attempt.step()
        self.assertEqual(result['action'], 'abort_begin')
        self.assertEqual(len(bridge.actions), count + 1)
        self.assertFalse(result['fresh'])
        self.assertFalse(result['retirement_proven'])
        for _ in range(3):
            value = attempt.step()
            self.assertEqual(value['reason'], 'need_observation')
            self.assertTrue(value['needs_observation'])
        self.assertEqual(bridge.actions.count('abort_observe'), 0)
        self.assertEqual(attempt.step(observation=True)['action'], 'abort_observe')
        self.assertEqual(bridge.actions.count('abort_observe'), 1)
        self.assertEqual(attempt.read()['observations_remaining'], 7)

    def test_eight_observations_no_budget_or_deadline_reset(self):
        bridge, attempt = self.suspended()
        attempt.step()
        for remaining in range(7, -1, -1):
            self.assertEqual(attempt.step(observation=True)['action'], 'abort_observe')
            value = attempt.read()
            self.assertEqual(value['observations_remaining'], remaining)
            a = value['terminal']['final_report']['abort']
            self.assertEqual((a['origin_ns'], a['deadline_ns'], a['max_entries'], a['max_observations']),
                             (1000, 1300, 16, 8))
        self.assertIsNone(attempt.step(observation=True)['action'])
        self.assertFalse(attempt.read()['needs_observation'])
        self.assertEqual(bridge.actions.count('abort_observe'), 8)
        self.assertFalse(attempt.read()['retirement_proven'])

    def test_original_state4_and_eligible_state8_transfer_not_healthy(self):
        for state in (4, 8):
            bridge = Bridge()
            if state == 8:
                def setup(t):
                    t['final_report'].update(state=8, rc=5, blocked=1,
                        launch_failure_mask=1 << 10, launch_first_failure_reason=11,
                        launch_first_failure_phase=5, launch_first_woc_rc=5)
                bridge.on_create = setup
            bridge, attempt = self.reaped(bridge)
            original = data(attempt.cached()['terminal']['final_report'])
            self.assertEqual(attempt.step()['action'], 'abort_handoff')
            value = attempt.read()
            self.assertEqual(value['retirement'], 'kind3_unresumed_abort')
            self.assertTrue(value['retirement_proven'])
            self.assertEqual(value['terminal']['final_report']['state'], state)
            for name in diagnostic._FAILURE_FIELDS:
                self.assertEqual(value['terminal']['final_report'][name], original[name])
            self.assertEqual(value['terminal']['wait_uid'], 9999)  # NOT owner identity
            self.assertFalse(value['healthy'])
            self.assertFalse(value['continue_authorized'])
            self.assertEqual(bridge.actions.count('abort_reap'), 1)
            self.assertEqual(bridge.actions.count('abort_handoff'), 1)
            self.assertIsNone(attempt.step()['action'])
            self.assertFalse(attempt.cached()['retirement_proven'])

    def test_kind1_no_creation_separate_once_only_handoff(self):
        bridge, attempt = self.suspended(Bridge(no_creation=True))
        child, sink = attempt._child, attempt._sink
        bridge.fail('handoff.after')
        self.assertEqual(attempt.step()['action'], 'handoff')
        value = attempt.read()
        self.assertEqual(value['retirement'], 'kind1_no_creation')
        self.assertTrue(value['retirement_proven'])
        self.assertTrue(value['failures'])
        self.assertIs(attempt._child, child)
        self.assertIs(attempt._sink, sink)
        self.assertEqual(bridge.actions, ['create_suspended', 'handoff'])
        self.assertFalse(value['terminal']['consumed'])
        self.assertIsNone(attempt.step()['action'])

    def test_each_action_reply_loss_reconciles_without_repetition(self):
        for name in ('abort_begin', 'abort_observe', 'abort_reap', 'abort_handoff'):
            with self.subTest(name=name):
                if name == 'abort_begin':
                    bridge, attempt = self.suspended()
                elif name == 'abort_observe':
                    bridge, attempt = self.suspended()
                    attempt.step()
                elif name == 'abort_reap':
                    bridge, attempt = self.ready_to_reap()
                else:
                    bridge, attempt = self.reaped()
                error = KeyboardInterrupt(name + ' lost reply')
                bridge.fail(name + '.after', error)
                result = attempt.step(observation=name == 'abort_observe')
                self.assertEqual(result['action'], name)
                self.assertIn(error, attempt._errors)
                self.assertTrue(attempt.read()['intent_resolved'])
                attempt.step()
                self.assertEqual(bridge.actions.count(name), 1)

    def test_unknown_preentry_and_midaction_intents_never_retried(self):
        for name in ('abort_begin', 'abort_observe', 'abort_reap', 'abort_handoff', 'handoff'):
            events = ('before',) if name in ('abort_handoff', 'handoff') else ('before', 'entered')
            for event in events:
                with self.subTest(name=name, event=event):
                    if name == 'handoff':
                        bridge, attempt = self.suspended(Bridge(no_creation=True))
                    elif name == 'abort_handoff':
                        bridge, attempt = self.reaped()
                    elif name == 'abort_reap':
                        bridge, attempt = self.ready_to_reap()
                    else:
                        bridge, attempt = self.suspended()
                        if name == 'abort_observe':
                            attempt.step()
                    bridge.fail(name + '.' + event)
                    self.assertEqual(attempt.step(observation=True)['action'], name)
                    for _ in range(2):
                        value = attempt.step(observation=True)
                        self.assertIsNone(value['action'])
                        self.assertFalse(value['retirement_proven'])
                    self.assertEqual(bridge.actions.count(name), 1)
                    self.assertIsNotNone(attempt.cached()['pending_intent'])

    def test_lost_raw_reads_and_failed_observation_bytes_are_retained(self):
        bridge, attempt = self.suspended()
        attempt.step()
        bridge.fail('abort_raw:0')
        self.assertFalse(attempt.read()['fresh'])
        self.assertTrue(attempt.read()['fresh'])
        a = bridge.sink.value['final_report']['abort']
        # The next native observation returns a failure with completed raw output.
        def failed_observation():
            index = a['observation_count']
            a['observations'][index].update(rc=-1, system_errno=5)
            a.update(phase=8, failure_rc=3, failure_system_errno=5, last_rc=3)
        bridge.hooks['abort_observe.after'] = failed_observation
        attempt.step(observation=True)
        value = attempt.read()
        self.assertEqual(value['abort_raw'][1]['bytes'], b'B' * 16)
        self.assertEqual(value['abort_raw'][1]['scalar']['rc'], -1)
        self.assertTrue(value['native_failures'])
        self.assertIsNone(attempt.step(observation=True)['action'])

    def test_unreturned_abort_slot_getter_is_not_entered_or_repeated(self):
        bridge, attempt = self.suspended()
        def unfinished():
            bridge.sink.value['final_report']['abort']['observations'][0]['returned'] = 0
        bridge.hooks['abort_begin.after'] = unfinished
        attempt.step()
        value = attempt.read()
        self.assertTrue(value['fresh'])
        self.assertEqual(value['abort_raw'][0]['status'], 'unentered')
        self.assertNotIn('abort_raw:0', bridge.calls)
        self.assertFalse(value['intent_resolved'])
        self.assertIsNone(attempt.step(observation=True)['action'])
        self.assertEqual(bridge.actions, ['create_suspended', 'abort_begin'])

    def test_completed_abort_bytes_cannot_be_rewritten(self):
        bridge, attempt = self.suspended()
        attempt.step()
        attempt.read()
        bridge.sink.raws = (b'X' * 16,) + bridge.sink.raws[1:]
        self.assertFalse(attempt.read()['fresh'])
        self.assertEqual(attempt.cached()['retained_abort_raw'][0][1][1], b'A' * 16)
        self.assertIsNone(attempt.step(observation=True)['action'])

    def test_reserved_terminal_bytes_never_prove_collection(self):
        bridge, attempt = self.suspended()
        value = attempt.read()
        self.assertEqual(len(value['terminal']['raw_siginfo']), 16)
        self.assertFalse(value['terminal']['wait_entered'])
        self.assertFalse(value['retirement_proven'])
        value['terminal'].update(transferred=1, kind=3, consumed=1, decoded=1, certificate_matches=1)
        value['retirement_proven'] = True
        self.assertFalse(attempt.cached()['retirement_proven'])
        self.assertFalse(attempt.read()['retirement_proven'])

    def test_ineligible_registration_resource_unknown_spawn_retained(self):
        for reason in ('registration', 'resource', 'unknown_spawn', 'other_launch', 'unreturned_fact'):
            bridge = Bridge()
            def setup(t):
                r = t['final_report']
                r.update(state=8, blocked=1, rc=3)
                if reason == 'registration':
                    r.update(registered=0, launch_failure_mask=1 << 6)
                elif reason == 'resource':
                    r.update(resource_error=1, launch_failure_mask=1 << 8)
                elif reason == 'unknown_spawn':
                    r.update(state=3, spawn_returned=0, registered=0, pid_valid=0,
                             launch_failure_mask=1 << 4)
                elif reason == 'other_launch':
                    r.update(launch_failure_mask=1 << 12, launch_first_failure_reason=13)
                else:
                    r['launch_facts'] = ((1, 1, 0, 0, 0),)
            bridge.on_create = setup
            bridge, attempt = self.suspended(bridge)
            self.assertIsNone(attempt.step(observation=True)['action'])
            self.assertEqual(bridge.actions, ['create_suspended'])
            self.assertFalse(attempt.read()['retirement_proven'])

    def test_native_deadline_budget_regression_or_root_disappearance(self):
        for change in ('deadline_ns', 'origin_ns', 'max_entries', 'max_observations', 'disappear'):
            bridge, attempt = self.suspended()
            if change == 'disappear':
                bridge.child = bridge.sink = None
            else:
                bridge.sink.value['final_report']['abort'][change] += 1
            self.assertFalse(attempt.step()['retirement_proven'])
            self.assertEqual(bridge.actions, ['create_suspended'])

    def test_copied_kind3_facts_without_local_intents_cannot_transfer(self):
        bridge, attempt = self.suspended()
        t = bridge.sink.value
        a = t['final_report']['abort']
        decoded = record(state=2, no_signals=1, reap_attempted=1, reaped=1,
                         exit_kind=2, exit_status=9, returncode=-9, uid_evidence=2)
        a.update(started=1, phase=6, entries=2, exit_valid=1, record=data(decoded))
        t.update(kind=3, reap_attempted=1, wait_entered=1, wait_returned=1,
                 consumed=1, decoded=1, certificate_matches=1, decoded_record=decoded)
        self.assertFalse(attempt.read()['fresh'])
        self.assertTrue(attempt.cached()['protocol_faults'])
        self.assertIsNone(attempt.step()['action'])
        self.assertEqual(bridge.actions, ['create_suspended'])

    def test_exact_observation_permission_and_native_entry_reserve(self):
        bridge, attempt = self.suspended()
        for permission in (1, 0, 'yes', None):
            before = tuple(bridge.calls)
            with self.assertRaises(schema.BridgeProtocolError):
                attempt.step(observation=permission)
            self.assertEqual(tuple(bridge.calls), before)
        attempt.step()
        a = bridge.sink.value['final_report']['abort']
        # A fabricated entry jump is not budget supplied by this consumer.
        a['entries'] = 15
        self.assertIsNone(attempt.step(observation=True)['action'])
        self.assertEqual(bridge.actions, ['create_suspended', 'abort_begin'])

    def test_native_deadline_expiry_retains_but_memory_handoff_can_be_late(self):
        bridge, attempt = self.suspended()
        a = bridge.sink.value['final_report']['abort']
        a['now_ns'] = a['deadline_ns']
        self.assertIsNone(attempt.step()['action'])
        bridge, attempt = self.reaped()
        a = bridge.sink.value['final_report']['abort']
        a['now_ns'] = a['deadline_ns']
        self.assertEqual(attempt.step()['action'], 'abort_handoff')
        self.assertEqual(attempt.read()['retirement'], 'kind3_unresumed_abort')

    def test_reentrancy_is_refused_before_any_nested_action(self):
        bridge, attempt = self.suspended()
        def reenter():
            with self.assertRaises(schema.BridgeProtocolError):
                attempt.step(observation=True)
        bridge.hooks['abort_begin.before'] = reenter
        self.assertEqual(attempt.step()['action'], 'abort_begin')
        self.assertEqual(bridge.actions, ['create_suspended', 'abort_begin'])

    def test_transfer_failure_history_stays_separate_from_retirement(self):
        bridge, attempt = self.reaped()
        bridge.fail('snapshot')
        self.assertFalse(attempt.read()['fresh'])
        bridge.fail('abort_handoff.after')
        attempt.step()
        value = attempt.read()
        self.assertEqual(value['retirement'], 'kind3_unresumed_abort')
        self.assertTrue(value['retirement_proven'])
        self.assertGreaterEqual(len(value['failures']), 2)
        self.assertFalse(value['healthy'])
        self.assertFalse(value['native_qualified'])


if __name__ == '__main__':
    unittest.main()
