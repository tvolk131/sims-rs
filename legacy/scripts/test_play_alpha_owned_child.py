"""UNRUN: guarded synthetic Child/Terminal consumer tests, in a separate process.

Parent invocation: python3 -B -m unittest discover -s scripts
                  -p test_play_alpha_owned_child.py -v
No extension is imported, no executable is run, and mocks confer no native ABI,
image/type authority, ownership, production qualification or alpha acceptance.
Actual, unmodified modules are imported before the deny guards; no AST rewriting.
"""

import builtins
import contextlib
import copy
import importlib.machinery
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

import play_alpha as alpha
import window_owned_child_bridge as protocol


def deny(*arguments, **keywords):
    raise AssertionError('Forbidden execution/native/network/process/wait/sleep/clock in source mocks')


_ORIGINAL_IMPORT = builtins.__import__
_FORBIDDEN = frozenset((
    'ctypes', '_ctypes', 'socket', '_socket', 'multiprocessing', 'psutil',
    'signal', 'platform', 'resource', '_posixsubprocess', '_window_owned_child_bridge',
    'subprocess',
))


def guarded_import(name, globals=None, locals=None, fromlist=(), level=0):
    if name.split('.')[0] in _FORBIDDEN:
        deny()
    filename = getattr(sys.modules.get(name), '__file__', '')
    if type(filename) is str and filename.endswith(('.so', '.dylib', '.pyd')):
        deny()
    return _ORIGINAL_IMPORT(name, globals, locals, fromlist, level)


def audit(event, arguments):
    if event.startswith(('subprocess.', 'ctypes.', 'socket.', 'os.exec', 'os.spawn',
                         'os.posix_spawn', 'os.fork', 'os.kill', 'os.system')):
        deny()
    if event == 'import' and len(arguments) > 1:
        filename = arguments[1]
        if type(filename) is str and filename.endswith(('.so', '.dylib', '.pyd')):
            deny()


sys.addaudithook(audit)


def record(**changes):
    # Independent native 200-byte record schema, including reserved zero slots.
    result = dict(bytes=200, version=1, error=0, system_errno=0,
                  pid=17, pgid=17, uid=12, ppid=7, sid=17, state=1,
                  exit_kind=0, exit_status=0, wait_uid_available=0, uid_evidence=1,
                  sigchld_default=1, no_cldwait=0, raw_bytes=0, pid_bytes=4,
                  capacity=2, count=0, member0=0, member1=0, no_signals=0,
                  reap_attempted=0, reaped=0, returncode=0, signal_attempted=0)
    result.update(changes)
    return result


def reserved_record():
    return dict.fromkeys(record(), 0)


def active_report(sequence=0, state=1, exit_kind=1, exit_status=0):
    result = dict.fromkeys((
        'bytes version generation sequence started_ns finished_ns birth_seconds birth_microseconds '
        'entered returned rc system_errno wait_entered wait_returned wait_rc wait_errno '
        'bsd_entered bsd_returned bsd_bytes bsd_status start_clock_rc start_clock_errno '
        'finish_clock_rc finish_clock_errno creator_pid creator_uid observed_sid'
    ).split(), 0)
    result.update(bytes=344, version=1, generation=23, sequence=sequence,
                  fresh_native_call=False, first_terminal=False, collection_evidence=False,
                  record=reserved_record(), raw_siginfo=b'')
    if not sequence:
        return result
    result.update(started_ns=sequence * 10, finished_ns=sequence * 10 + 1,
                  birth_seconds=123, birth_microseconds=45,
                  entered=1, returned=1, wait_entered=1, wait_returned=1,
                  creator_pid=7, creator_uid=12, raw_siginfo=b'authored nonconsuming raw')
    result['record'] = record(state=state, uid_evidence=1 if state == 1 else 2, capacity=0)
    if state == 2:
        result['record'].update(exit_kind=exit_kind, exit_status=exit_status,
                                returncode=exit_status if exit_kind == 1 else -exit_status)
    else:
        result.update(bsd_entered=1, bsd_returned=1, bsd_bytes=136,
                      bsd_status=5 if state == 3 else 2, observed_sid=17 if state == 1 else 0)
    return result


def registration():
    return dict(entered=0, returned=0, rc=0, system_errno=0, idtype=0, target_pid=0,
                options=0, signo=0, code=0, pid=0, uid=0, status=0, raw_siginfo_bytes=0)


def abort_report():
    return dict(last_operation=0, last_rc=0, last_system_errno=0, phase=0, started=0,
                entries=0, max_entries=16, deadline_ns=2000, origin_ns=1, now_ns=1,
                observation_count=0, max_observations=8, trace_count=0, failure_rc=0,
                failure_system_errno=0, kill_entered=0, kill_returned=0, kill_rc=0,
                kill_errno=0, preexisting_exit=0, exit_observation_index=-1, exit_valid=0,
                bsd_status=0, bsd_pid=0, bsd_ppid=0, bsd_uid=0, bsd_pgid=0, sid=0,
                trace=(), observations=tuple(dict(entered=0, returned=0, rc=0, system_errno=0,
                    raw_siginfo_bytes=0, pid=0, uid=0, code=0, status=0) for _ in range(9)),
                record=reserved_record())


def child_report():
    return dict(generation=23, state=6, rc=0, system_errno=0, blocked=0, published=1,
                pid_valid=1, registered=1, registration_bsd_status=3, registration_pid=17,
                registration_ppid=7, registration_pgid=17, registration_uid=12,
                registration_sid=17, spawn_entered=1, spawn_returned=1, spawn_rc=0,
                spawn_errno=0, spawn_pid_raw=17, resume_entered=1, resume_returned=1,
                resume_rc=0, resume_errno=0, launch_entries=3, launch_fact_count=0,
                launch_facts=(), launch_deadline_ns=1000, resources_closed=1, resource_error=0,
                cleanup_started=0, cleanup_entries=0, max_cleanup_entries=128,
                cleanup_deadline_ns=0, cleanup_kill_at_ns=0, cleanup_now_ns=0,
                clock_rc=0, clock_errno=0, term_attempted=0, kill_attempted=0,
                signal_failed=0, term_returned=0, term_rc=0, term_errno=0,
                kill_returned=0, kill_rc=0, kill_errno=0, latch_attempted=0,
                sigchld_preflight_entered=1, sigchld_preflight_returned=1,
                sigchld_preflight_rc=0, sigchld_preflight_errno=0,
                sigchld_preflight_policy_valid=1, sigchld_preflight_default=1,
                sigchld_preflight_no_cldwait=0, sigchld_preflight_accepted=1,
                launch_failure_mask=0, launch_first_failure_phase=0,
                launch_first_failure_reason=0, launch_first_woc_rc=0,
                launch_first_system_errno=0, record=record(), abort=abort_report(),
                registration_observation=registration())


def terminal_report():
    return dict(generation=23, rooted=1, transferred=0, known_no_creation=0, kind=0,
                reap_attempted=0, wait_entered=0, wait_returned=0, wait_rc=0, wait_errno=0,
                wait_pid=0, wait_uid=0, wait_code=0, wait_status=0, consumed=0,
                decoded=0, certificate_matches=0, raw_siginfo=b'\0' * 128,
                decoded_record=reserved_record(), final_report=child_report(),
                registration_observation=registration())


class MockChild:
    """Explicit source token, never a native Child or a Popen."""


class MockSink:
    def __init__(self, bridge):
        self.bridge = bridge

    def snapshot(self):
        self.bridge.calls.append('snapshot')
        self.bridge.action('snapshot')
        return copy.deepcopy(self.bridge.t)

    def active_observation(self):
        self.bridge.calls.append('active_snapshot')
        self.bridge.action('active_snapshot')
        return copy.deepcopy(self.bridge.active)

    def active_first_terminal(self):
        self.bridge.calls.append('active_first_terminal')
        self.bridge.action('active_first_terminal')
        return copy.deepcopy(self.bridge.first_active)


class MockBridge:
    """Synthetic implementation of the existing protocol; no host operations."""

    def __init__(self):
        self.calls, self.arguments, self.actions = [], None, {}
        self.child = self.sink = None
        self.t = terminal_report()
        self.exited = True
        self.members = (17,)
        self.on_create = None
        self.active, self.first_active = active_report(), active_report()
        self.first_active['first_terminal'] = True
        self.active_state = 1
        self.active_exit_kind = 1
        self.active_exit_status = 0

    @property
    def r(self):
        return self.t['final_report']

    def action(self, name):
        actions = self.actions.get(name, [])
        if actions:
            value = actions.pop(0)
            if isinstance(value, BaseException):
                raise value
            value(self)

    def pending(self):
        self.calls.append('pending')
        self.action('pending')
        return self.child

    def history(self):
        self.calls.append('history')
        self.action('history')
        return () if self.sink is None else (self.sink,)

    def terminal(self, child):
        assert child is self.child
        self.calls.append('terminal')
        self.action('terminal')
        return self.sink

    def child_snapshot(self, child):
        assert child is self.child
        self.calls.append('child_snapshot')
        self.action('child_snapshot')
        return copy.deepcopy(self.r)

    def create(self, *arguments):
        self.calls.append('create')
        self.arguments = arguments
        if self.on_create:
            self.on_create()
        self.child, self.sink = MockChild(), MockSink(self)
        self.action('create')  # Native roots already exist at an interrupted return.
        return self.child

    def reply(self, name):
        self.action(name)  # Native effects precede a lost mapping/reply.
        return copy.deepcopy(self.r)

    def active_observe(self, child):
        assert child is self.child
        self.calls.append('active_observe')
        self.action('active_before')
        self.active = active_report(self.active['sequence'] + 1, self.active_state,
                                    self.active_exit_kind, self.active_exit_status)
        if self.active_state == 2 and not self.first_active['entered']:
            self.first_active = copy.deepcopy(self.active)
            self.first_active['first_terminal'] = True
        self.action('active_observe')
        result = copy.deepcopy(self.active)
        result['fresh_native_call'] = True
        return result

    def cleanup_begin(self, child):
        assert child is self.child
        self.calls.append('cleanup_begin')
        self.r.update(cleanup_started=1, cleanup_entries=1,
                      cleanup_deadline_ns=500, cleanup_now_ns=10)
        return self.reply('cleanup_begin')

    def entry(self, child, name):
        assert child is self.child
        self.calls.append(name)
        self.r['cleanup_entries'] += 1
        self.r['cleanup_now_ns'] += 10

    def child_observe(self, child):
        self.entry(child, 'child_observe')
        self.r['record'] = record(state=2, uid_evidence=2, exit_kind=1) if self.exited else record()
        return self.reply('child_observe')

    def child_members(self, child):
        self.entry(child, 'child_members')
        self.r['record'] = record(state=0, uid_evidence=2, count=len(self.members),
                                 raw_bytes=len(self.members) * 4, member0=self.members[0],
                                 member1=self.members[1] if len(self.members) == 2 else 0)
        return self.reply('child_members')

    def child_signal(self, child, kind, positive):
        self.entry(child, 'child_signal')
        assert (positive == 0 and not self.exited) or positive in self.members[1:]
        if kind == 1:
            self.r.update(term_attempted=1, term_returned=1, term_rc=0,
                          cleanup_kill_at_ns=self.r['cleanup_now_ns'] + 30)
        else:
            assert self.r['cleanup_now_ns'] >= self.r['cleanup_kill_at_ns']
            self.r.update(kill_attempted=1, kill_returned=1, kill_rc=0)
        self.r['record'] = record(signal_attempted=1)
        return self.reply('child_signal')

    def child_latch(self, child, kind, status):
        self.entry(child, 'child_latch')
        assert self.members == (17,) and self.exited and (kind, status) == (1, 0)
        self.r.update(latch_attempted=1)
        self.r['record'] = record(state=2, uid_evidence=2, exit_kind=kind,
                                 exit_status=status, returncode=status, no_signals=1)
        return self.reply('child_latch')

    def consuming_facts(self, kind):
        decoded = record(state=2, uid_evidence=2, exit_kind=1, no_signals=1,
                         reap_attempted=1, reaped=1)
        self.t.update(kind=kind, reap_attempted=1, wait_entered=1, wait_returned=1,
                      wait_pid=17, wait_code=1, consumed=1, decoded=1,
                      certificate_matches=1, decoded_record=decoded)
        return copy.deepcopy(decoded)

    def child_reap(self, child):
        self.entry(child, 'child_reap')
        self.r['record'] = self.consuming_facts(2)
        self.r['state'] = 10
        return self.reply('child_reap')

    def handoff(self, child, sink):
        assert child is self.child and sink is self.sink
        self.calls.append('handoff')
        if not self.t['known_no_creation']:
            self.r['cleanup_entries'] += 1
        self.t['transferred'] = 1
        self.r['state'] = 11
        self.child = None
        self.action('handoff')
        return sink

    def abort_begin(self, child):
        assert child is self.child
        self.calls.append('abort_begin')
        self.r['abort'].update(started=1, phase=3, entries=1, kill_entered=1,
                               kill_returned=1, record=record(no_signals=1))
        return self.reply('abort_begin')

    def abort_observe(self, child):
        assert child is self.child
        self.calls.append('abort_observe')
        self.r['abort'].update(phase=4, entries=2, observation_count=1, exit_valid=1,
                               exit_observation_index=1)
        return self.reply('abort_observe')

    def abort_reap(self, child):
        assert child is self.child
        self.calls.append('abort_reap')
        self.r['abort'].update(phase=6, entries=3, record=self.consuming_facts(3))
        return self.reply('abort_reap')

    def abort_handoff(self, child, sink):
        assert child is self.child and sink is self.sink
        self.calls.append('abort_handoff')
        self.r['abort'].update(phase=7, entries=4)
        self.t['transferred'] = 1
        self.child = None
        self.action('abort_handoff')
        return sink


class OwnedChildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='owned-child-UNRUN-fixture-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.installation = self.root / 'installation'
        self.installation.mkdir()
        self.binary = self.root / 'source-executable'
        self.binary.write_bytes(b'SYNTHETIC FILE ONLY; NEVER EXECUTE\n')
        self.binary.chmod(0o700)
        self.sequence = 0
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.stack.enter_context(mock.patch.object(alpha, 'REPOSITORY', self.root))
        # Replace tempfile's internal PID-dependent name selection with synthetic
        # file-only allocation. Production preparation itself stays unmodified.
        self.stack.enter_context(mock.patch.object(tempfile, 'mkdtemp', self.mkdtemp))
        self.stack.enter_context(mock.patch.object(tempfile, 'mkstemp', self.mkstemp))
        self.stack.enter_context(mock.patch.object(builtins, '__import__', guarded_import))
        for name in ('create_module', 'exec_module'):
            self.stack.enter_context(mock.patch.object(importlib.machinery.ExtensionFileLoader, name, deny))
        for name in ('Popen', 'run', 'call', 'check_call', 'check_output'):
            self.stack.enter_context(mock.patch.object(subprocess, name, deny))
        for name in tuple(vars(os)):
            if name.startswith(('exec', 'spawn')) or name in (
                'fork', 'forkpty', 'posix_spawn', 'posix_spawnp', 'system', 'popen',
                'wait', 'wait3', 'wait4', 'waitid', 'waitpid', 'kill', 'killpg',
                'getpid', 'getppid', 'getuid', 'geteuid', 'getgid', 'getegid',
                'getpgid', 'getsid', 'setsid', 'setpgid', 'setuid', 'seteuid',
            ):
                self.stack.enter_context(mock.patch.object(os, name, deny))
        for name in ('sleep', 'monotonic', 'monotonic_ns', 'time', 'time_ns',
                     'perf_counter', 'perf_counter_ns', 'process_time', 'thread_time'):
            if hasattr(time, name):
                self.stack.enter_context(mock.patch.object(time, name, deny))

    def fresh(self, parent, prefix):
        self.sequence += 1
        return Path(parent) / (prefix + str(self.sequence))

    def mkdtemp(self, suffix=None, prefix=None, dir=None):
        path = self.fresh(dir, prefix or 'directory-')
        path.mkdir(mode=0o700)
        return str(path)

    def mkstemp(self, suffix=None, prefix=None, dir=None, text=False):
        path = self.fresh(dir, prefix or 'file-')
        return os.open(path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600), str(path)

    def prepare(self, **options):
        return alpha.prepare_launch(self.binary, self.installation, **options)

    def session(self, prepared=None, bridge=None, **options):
        prepared = prepared or self.prepare()
        bridge = bridge or MockBridge()
        session = alpha._owned_child_for_injected_mock(
            prepared, object(), bridge, bridge_type=MockBridge, child_type=MockChild,
            terminal_type=MockSink, **options)
        return prepared, bridge, session

    def complete(self, session, steps=9):
        for _ in range(steps):
            session.finish_step()
        return session.read_completion()

    def test_guards_forbid_execution_native_network_process_clock_and_wait(self):
        for function in (subprocess.run, subprocess.Popen, os.getpid, os.waitpid,
                         os.kill, time.sleep, time.monotonic):
            with self.assertRaises(AssertionError):
                function()
        for name in ('ctypes', 'socket', '_window_owned_child_bridge'):
            with self.assertRaises(AssertionError):
                __import__(name)

    def test_production_refuses_before_binding_properties_loader_or_clock(self):
        class Bomb:
            def __getattribute__(self, name):
                deny()
            def __call__(self, *arguments, **keywords):
                deny()
        bomb = Bomb()
        for constructor in (protocol.ChildExecution, protocol.production_child_execution,
                            alpha.owned_child_execution, protocol.production_backend, protocol.Backend):
            with self.assertRaises(protocol.Unsupported):
                constructor(bomb, binding=bomb, loader=bomb, clock=bomb, native_qualified=True)
        self.assertTrue(all(value is False for value in protocol.QUALIFICATIONS.values()))

    def test_exact_command_cwd_and_root_before_create_and_single_admission(self):
        prepared, bridge, session = self.session(env=(b'MOCK=value',), stdio=(3, 4, 5))
        def rooted():
            self.assertIn(session, protocol._CHILD_EXECUTIONS)
            self.assertIs(session._claim.prepared, prepared)
            self.assertIn(session._claim, alpha._OWNED_CHILD_CLAIMS)
            self.assertTrue(session._claim.validated)
            with self.assertRaises(protocol.BridgeProtocolError):
                session.admit()
        bridge.on_create = rooted
        session.admit()
        argv = tuple(os.fsencode(item) for item in prepared.argv)
        self.assertEqual(bridge.arguments, (argv[0], argv, (b'MOCK=value',),
                         os.fsencode(prepared.cwd), (3, 4, 5), 1000000000, 1000000000,
                         100000000, 128))
        self.assertIs(session._sink, bridge.sink)
        self.assertEqual(session._generation, 23)
        with self.assertRaises(protocol.BridgeProtocolError):
            session.admit()
        with self.assertRaises(ValueError):
            self.session(prepared)
        with self.assertRaises(ValueError):
            alpha.prepared_command(prepared)
        self.assertEqual(bridge.calls.count('create'), 1)

    def test_prepared_revalidation_failure_burns_claim_without_bridge_access(self):
        prepared, bridge, session = self.session()
        Path(prepared.argv[0]).chmod(0o600)
        Path(prepared.argv[0]).write_bytes(b'changed')
        with self.assertRaises(ValueError):
            session.admit()
        self.assertEqual(bridge.calls, [])
        self.assertTrue(session._claim.started)
        with self.assertRaises(protocol.BridgeProtocolError):
            session.admit()
        with self.assertRaises(ValueError):
            self.session(prepared)

    def test_reentrant_prepared_validation_cannot_claim_or_admit_twice(self):
        prepared, bridge, session = self.session()
        original = alpha.check_inputs
        def validate(receipt):
            with self.assertRaises(ValueError):
                self.session(prepared)
            with self.assertRaises(protocol.BridgeProtocolError):
                session.admit()
            return original(receipt)
        with mock.patch.object(alpha, 'check_inputs', validate):
            session.admit()
        self.assertEqual(bridge.calls.count('create'), 1)

    def test_changed_cwd_and_argv_cannot_be_admitted(self):
        for field, value in (('cwd', self.installation), ('argv', ('/forged',))):
            prepared, bridge, session = self.session()
            object.__setattr__(prepared, field, value)
            with self.assertRaises(ValueError):
                session.admit()
            self.assertNotIn('create', bridge.calls)

    def test_copied_serialized_cross_session_and_lost_mapping_claims_refused(self):
        prepared, bridge, session = self.session()
        fake = alpha.PreparedLaunch(prepared.argv, prepared.cwd, prepared.receipt_path, prepared.save_output)
        with self.assertRaises(ValueError):
            self.session(fake)
        with self.assertRaises(TypeError):
            copy.copy(session._claim)
        with self.assertRaises(TypeError):
            session._claim.__reduce_ex__(4)
        for copier in (copy.copy, copy.deepcopy):
            with self.assertRaises(TypeError):
                copier(session)
        with self.assertRaises(TypeError):
            session.__reduce_ex__(4)
        other = self.session()[2]
        with self.assertRaises(ValueError):
            session._claim._admit(other)
        session.admit()
        del alpha._PREPARED[prepared]
        self.assertIs(session._claim.prepared, prepared)
        with self.assertRaises(ValueError):
            self.session(prepared)
        self.assertEqual(self.complete(session)[0], 'exited')
        self.assertFalse(session.native_qualified)

    def test_creation_interrupt_recovers_same_pending_history_and_terminal(self):
        prepared, bridge, session = self.session()
        bridge.actions['create'] = [KeyboardInterrupt()]
        with self.assertRaises(KeyboardInterrupt):
            session.admit()
        self.assertIsNone(session._child)
        self.assertEqual(session.read_completion()[0], 'unresolved')
        self.assertIs(session._child, bridge.child)
        self.assertIs(session._sink, bridge.sink)
        self.assertEqual(self.complete(session)[0], 'exited')
        self.assertEqual(bridge.calls.count('create'), 1)

    def test_failed_recovery_retains_binding_claim_and_bridge_and_can_read_later(self):
        prepared, bridge, session = self.session()
        binding = session._binding
        bridge.actions['create'] = [MemoryError()]
        with self.assertRaises(MemoryError):
            session.admit()
        bridge.actions['history'] = [MemoryError()]
        self.assertEqual(session.read_completion()[0], 'unresolved')
        self.assertIs(session._binding, binding)
        self.assertIn(session, protocol._CHILD_EXECUTIONS)
        self.assertIs(session._claim.prepared, prepared)
        self.assertEqual(self.complete(session)[0], 'exited')

    def test_no_pending_or_history_never_proves_no_creation_and_never_retries(self):
        prepared, bridge, session = self.session()
        def lose_return(peer):
            peer.child = peer.sink = None
            raise MemoryError()
        bridge.actions['create'] = [lose_return]
        with self.assertRaises(MemoryError):
            session.admit()
        self.assertEqual(session.read_completion()[0], 'unresolved')
        self.assertEqual(session.finish_step()[0], 'unresolved')
        self.assertEqual(bridge.calls.count('create'), 1)

    def test_running_child_or_historical_bridge_is_never_adopted(self):
        for existing in ('child', 'sink'):
            bridge = MockBridge()
            setattr(bridge, existing, MockChild() if existing == 'child' else MockSink(bridge))
            prepared, bridge, session = self.session(bridge=bridge)
            with self.assertRaises(protocol.BridgeProtocolError):
                session.admit()
            self.assertNotIn('create', bridge.calls)

    def test_wrong_copied_child_sink_bridge_and_generation_remain_unresolved(self):
        for change in ('child', 'sink', 'generation', 'bridge'):
            prepared, bridge, session = self.session()
            session.admit()
            if change == 'child':
                bridge.child = MockChild()
            elif change == 'sink':
                bridge.sink = MockSink(bridge)
            elif change == 'generation':
                bridge.t['generation'] += 1
                bridge.r['generation'] += 1
            else:
                session._bridge = MockBridge()  # Cannot turn mapping loss into adoption.
            self.assertEqual(session.finish_step()[0], 'unresolved')
            self.assertNotIn('child_reap', bridge.calls)

    def test_numeric_or_terminal_dict_create_return_is_not_child_admission(self):
        for fake in (17, {'returncode': 0}, copy.copy(MockChild())):
            prepared, bridge, session = self.session()
            with mock.patch.object(bridge, 'create', return_value=fake):
                with self.assertRaises(protocol.BridgeProtocolError):
                    session.admit()
            self.assertEqual(session.finish_step()[0], 'unresolved')

    def test_numeric_peer_type_and_its_subclass_are_not_child_types(self):
        class NumericChild(int):
            pass
        for kind in (int, float, dict, NumericChild):
            with self.assertRaises(protocol.BridgeProtocolError):
                alpha._owned_child_for_injected_mock(self.prepare(), object(), MockBridge(),
                    bridge_type=MockBridge, child_type=kind, terminal_type=MockSink)

    def test_missing_malformed_native_fields_fail_closed(self):
        for mutate in (
            lambda t: t.pop('consumed'),
            lambda t: t.update(rooted=True),
            lambda t: t['final_report'].pop('launch_failure_mask'),
            lambda t: t['final_report'].update(cleanup_entries=True),
            lambda t: t['final_report']['abort'].update(max_entries=17),
            lambda t: t.update(raw_siginfo='copied text'),
        ):
            prepared, bridge, session = self.session()
            session.admit()
            mutate(bridge.t)
            self.assertEqual(session.finish_step()[0], 'unresolved')
            self.assertNotIn('cleanup_begin', bridge.calls)

    def test_normal_completion_reads_exact_sink_after_handoff_without_more_operations(self):
        prepared, bridge, session = self.session()
        session.admit()
        self.assertEqual(self.complete(session), ('exited', 0, None))
        self.assertEqual(bridge.calls.count('child_reap'), 1)
        self.assertEqual(bridge.calls.count('handoff'), 1)
        self.assertIs(session._sink, bridge.sink)
        self.assertIsNotNone(session._child)  # Keep original wrapper even after invalidation.
        for _ in range(3):
            self.assertEqual(session.finish_step(), ('exited', 0, None))
        self.assertEqual(bridge.calls.count('child_reap'), 1)
        self.assertEqual(bridge.calls.count('handoff'), 1)

    def test_mapping_loss_after_reap_and_handoff_never_repeats_consuming_action(self):
        for operation, failure in (('child_reap', MemoryError()), ('handoff', KeyboardInterrupt())):
            prepared, bridge, session = self.session()
            bridge.actions[operation] = [failure]
            session.admit()
            self.assertEqual(self.complete(session), ('exited', 0, None))
            self.assertEqual(bridge.calls.count('child_reap'), 1)
            self.assertEqual(bridge.calls.count('handoff'), 1)

    def test_malformed_reap_reply_recovers_permanent_facts_and_late_sink_read(self):
        prepared, bridge, session = self.session()
        original = bridge.child_reap
        def malformed(child):
            original(child)
            return {'returncode': 0}  # Not an initialized native report.
        session.admit()
        with mock.patch.object(bridge, 'child_reap', malformed):
            self.assertEqual(self.complete(session)[0], 'exited')
        bridge.actions['snapshot'] = [MemoryError()]
        self.assertEqual(session.read_completion()[0], 'unresolved')
        self.assertEqual(session.read_completion()[0], 'exited')
        self.assertEqual(bridge.calls.count('child_reap'), 1)
        self.assertEqual(bridge.calls.count('handoff'), 1)

    def test_failed_wait_or_decode_never_permits_a_second_consuming_wait(self):
        for field in ('wait_returned', 'consumed', 'decoded', 'certificate_matches'):
            prepared, bridge, session = self.session()
            session.admit()
            for _ in range(5):
                session.finish_step()
            self.assertEqual(bridge.calls.count('child_reap'), 1)
            bridge.t[field] = 0
            self.assertEqual(self.complete(session)[0], 'unresolved')
            self.assertEqual(bridge.calls.count('child_reap'), 1)
            self.assertNotIn('handoff', bridge.calls)

    def test_lost_reply_before_native_intent_is_not_retried(self):
        prepared, bridge, session = self.session()
        session.admit()
        with mock.patch.object(bridge, 'cleanup_begin', side_effect=MemoryError()) as begin:
            session.finish_step()
            session.finish_step()
            self.assertEqual(begin.call_count, 1)
        self.assertEqual(session.read_completion()[0], 'unresolved')

    def test_no_creation_handoff_is_always_launch_failure(self):
        prepared, bridge, session = self.session()
        bridge.t.update(kind=1, known_no_creation=1)
        bridge.r.update(state=7, registered=0, pid_valid=0, spawn_entered=0,
                        spawn_returned=0, resume_entered=0, resume_returned=0, published=0,
                        rc=3, system_errno=22, launch_failure_mask=1)
        bridge.actions['create'] = [RuntimeError('known no-creation')]
        with self.assertRaises(RuntimeError):
            session.admit()
        self.assertEqual(self.complete(session)[0:2], ('launch_error', None))
        self.assertNotIn('child_reap', bridge.calls)
        self.assertNotIn('cleanup_begin', bridge.calls)

    def test_unresumed_abort_preserves_original_failure_and_recovers_lost_replies(self):
        for state, lost in ((state, lost) for state in (4, 8)
                            for lost in ('abort_begin', 'abort_reap', 'abort_handoff')):
            prepared, bridge, session = self.session()
            bridge.r.update(state=state, blocked=1, published=0, resume_entered=0, resume_returned=0,
                            rc=5, launch_failure_mask=1 << 10, launch_first_failure_reason=11,
                            launch_first_woc_rc=5)
            original = {name: bridge.r[name] for name in
                        ('state', 'rc', 'blocked', 'launch_failure_mask')}
            bridge.actions['create'] = [RuntimeError('pre-resume deadline')]
            bridge.actions[lost] = [MemoryError()]
            with self.assertRaises(RuntimeError):
                session.admit()
            self.assertEqual(self.complete(session)[0:2], ('launch_error', 0))
            self.assertEqual({name: bridge.r[name] for name in original}, original)
            self.assertEqual(bridge.calls.count('abort_reap'), 1)
            self.assertEqual(bridge.calls.count('abort_handoff'), 1)
            self.assertNotIn('child_reap', bridge.calls)

    def test_native_deadline_budget_and_successful_term_grace_are_never_reset(self):
        prepared, bridge, session = self.session()
        bridge.exited = False
        session.admit()
        for _ in range(8):
            session.finish_step()
        self.assertEqual(bridge.r['cleanup_deadline_ns'], 500)
        self.assertEqual(bridge.r['max_cleanup_entries'], 128)
        self.assertEqual(bridge.calls.count('cleanup_begin'), 1)
        self.assertTrue(bridge.r['term_returned'])
        self.assertTrue(bridge.r['kill_attempted'])
        self.assertEqual(bridge.calls.count('child_signal'), 2)
        bridge.r['cleanup_entries'] = 128
        before = bridge.calls.count('child_observe')
        session.finish_step()
        self.assertEqual(bridge.calls.count('child_observe'), before)
        self.assertEqual(session.read_completion()[0], 'unresolved')

    def test_native_deadline_or_budget_expansion_blocks_any_further_action(self):
        for field in ('cleanup_deadline_ns', 'max_cleanup_entries'):
            prepared, bridge, session = self.session()
            session.admit()
            session.finish_step()
            session.read_completion()
            bridge.r[field] += 1
            self.assertEqual(session.finish_step()[0], 'unresolved')
            self.assertNotIn('child_observe', bridge.calls)

    def test_zero_exit_forced_teardown_signal_and_historical_failure_are_not_healthy(self):
        prepared, bridge, session = self.session()
        session.admit()
        self.assertEqual(self.complete(session)[0], 'exited')
        normal = copy.deepcopy(bridge.t)
        for change in (
            {'term_attempted': 1}, {'kill_attempted': 1}, {'signal_failed': 1},
            {'launch_failure_mask': 1}, {'clock_rc': -1}, {'blocked': 1},
            {'launch_first_woc_rc': 5}, {'resume_rc': 1},
        ):
            terminal = copy.deepcopy(normal)
            terminal['final_report'].update(change)
            self.assertEqual(protocol._classify_child_terminal(terminal, 23)[0], 'diagnostic_failed')
        for name in ('consumed', 'decoded', 'certificate_matches', 'wait_returned'):
            terminal = copy.deepcopy(normal)
            terminal[name] = 0
            with self.assertRaises(protocol.BridgeProtocolError):
                protocol._classify_child_terminal(terminal, 23)
        terminal = copy.deepcopy(normal)
        terminal['decoded_record']['returncode'] = 19
        with self.assertRaises(protocol.BridgeProtocolError):
            protocol._classify_child_terminal(terminal, 23)
        terminal = copy.deepcopy(normal)
        terminal.update(wait_code=2, wait_status=9)
        for entry in (terminal['decoded_record'], terminal['final_report']['record']):
            entry.update(exit_kind=2, exit_status=9, returncode=-9)
        self.assertEqual(protocol._classify_child_terminal(terminal, 23)[0], 'diagnostic_failed')

    def test_mock_completion_cannot_publish_continue_or_be_promoted_by_bool(self):
        prepared, bridge, session = self.session()
        session.admit()
        self.assertEqual(self.complete(session)[0], 'exited')
        prepared.save_output.write_text(json.dumps({'session': {'version': 2,
            'source': alpha.HOUSEHOLDS['newbie']}, 'dialog_client': {}}))
        receipt = prepared.receipt_path.read_bytes()
        completion = alpha._owned_child_completion(prepared, session)
        self.assertIsInstance(completion, alpha.TrustedCompletion)
        session._native_qualified = True  # No instance flag is an authority hook.
        with self.assertRaises(AttributeError):
            session.native_qualified = True
        for _ in range(3):
            self.assertEqual(completion.observe().status, 'unresolved')
            self.assertEqual(alpha.finalize_launch(prepared, completion), 1)
        self.assertEqual(prepared.receipt_path.read_bytes(), receipt)
        self.assertFalse((prepared.receipt_path.parent.parent / 'last-session.json').exists())
        self.assertEqual(bridge.calls.count('create'), 1)
        self.assertEqual(bridge.calls.count('child_reap'), 1)
        self.assertFalse(alpha._PREPARED[prepared].finalized)

    def test_fabricated_completion_and_cross_prepared_adapter_cannot_bypass_claim(self):
        prepared, bridge, session = self.session()
        session.admit()
        observation = alpha.CompletionObservation(prepared.argv, prepared.cwd, 'exited', 0)
        with self.assertRaises(TypeError):
            alpha.finalize_launch(prepared, alpha._LegacyCompletion(observation))
        with self.assertRaises(ValueError):
            alpha._owned_child_completion(self.prepare(), session)

    def test_completion_read_does_not_begin_cleanup_or_adapt_diagnostic_runner(self):
        prepared, bridge, session = self.session()
        session.admit()
        for _ in range(2):
            self.assertEqual(session.read_completion()[0], 'unresolved')
        self.assertNotIn('cleanup_begin', bridge.calls)
        self.assertNotIn('child_observe', bridge.calls)
        self.assertFalse(hasattr(session, 'poll'))
        self.assertFalse(hasattr(session, 'returncode'))

    def test_public_preparation_revalidates_without_spawning_and_legacy_remains(self):
        prepared = self.prepare(household='goth')
        self.assertEqual(alpha.prepared_command(prepared), list(prepared.argv))
        self.assertEqual(alpha.prepared_command(prepared), list(prepared.argv))
        self.assertIn('--paused', prepared.argv)
        self.assertEqual(prepared.argv[prepared.argv.index('--original') + 1], alpha.HOUSEHOLDS['goth'])
        calls = []
        def legacy(command, **options):
            calls.append((command, options))
            self.assertEqual(options['cwd'], self.root)
            result = subprocess.CompletedProcess(command, 0)
            result.diagnostic = {'status': 'exited', 'exit_code': 0}
            return result
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(alpha.launch(self.binary, self.installation, child_runner=legacy), 0)
        self.assertEqual(len(calls), 1)
        self.assertIn('--paused', calls[0][0])

    def test_active_empty_caches_never_query_or_claim_liveness(self):
        _, bridge, session = self.session()
        session.admit()
        for first in (False, True):
            result = session.read_active(first_terminal=first)
            self.assertEqual((result['entered'], result['sequence'], result['record']['state']), (0, 0, 0))
            self.assertIs(result['fresh_native_call'], False)
            self.assertIs(result['first_terminal'], first)
            self.assertIs(result['collection_evidence'], False)
        self.assertNotIn('active_observe', bridge.calls)
        self.assertNotIn('cleanup_begin', bridge.calls)

    def test_active_unadmitted_phase_refuses_without_bridge_access_or_poisoning_admission(self):
        _, bridge, session = self.session()
        for operation in (session.observe_active, session.read_active):
            with self.assertRaises(protocol.Unsupported):
                operation()
            self.assertEqual(bridge.calls, [])
            self.assertIsNone(session._fault)
        session.admit()
        self.assertEqual(session.observe_active()['sequence'], 1)
        self.assertEqual(bridge.calls.count('create'), 1)

    def test_structurally_blocked_precreation_session_cannot_later_create_a_child(self):
        _, bridge, session = self.session()
        self.assertEqual(session.read_completion()[0], 'unresolved')
        self.assertIsNotNone(session._fault)
        with self.assertRaises(protocol.BridgeProtocolError):
            session.admit()
        self.assertEqual(bridge.calls, [])
        self.assertIsNone(bridge.child)

    def test_active_live_fresh_query_and_cached_reads_preserve_cleanup(self):
        _, bridge, session = self.session()
        session.admit()
        before = copy.deepcopy(bridge.r)
        for sequence in (1, 2):
            fresh = session.observe_active()
            self.assertEqual(fresh['sequence'], sequence)
            self.assertEqual(fresh['record']['state'], 1)
            self.assertIs(fresh['fresh_native_call'], True)
            fresh['record']['pid'] = 900
            for _ in range(3):
                cached = session.read_active()
                self.assertEqual((cached['sequence'], cached['record']['pid']), (sequence, 17))
                self.assertIs(cached['fresh_native_call'], False)
            self.assertEqual(bridge.r, before)
        self.assertEqual(bridge.calls.count('active_observe'), 2)
        self.assertFalse(session.native_qualified)
        self.assertNotIn('cleanup_begin', bridge.calls)
        self.assertFalse(hasattr(session, 'poll'))

    def test_active_exit_transition_is_distinct_from_live_and_completion(self):
        _, bridge, session = self.session()
        session.admit()
        bridge.active_state = 3
        transition = session.observe_active()
        self.assertEqual(transition['record']['state'], 3)
        self.assertEqual(transition['observed_sid'], 0)
        self.assertEqual(transition['record']['exit_kind'], 0)
        self.assertEqual(session.read_completion()[0], 'unresolved')
        bridge.active_state = 2
        exited = session.observe_active()
        self.assertEqual(exited['record']['state'], 2)
        self.assertEqual(session.read_active(first_terminal=True)['sequence'], 2)
        self.assertEqual(session.read_completion()[0], 'unresolved')
        self.assertNotIn('child_reap', bridge.calls)
        self.assertNotIn('cleanup_begin', bridge.calls)

    def test_active_normal_signal_core_first_terminal_is_immutable_and_uncollected(self):
        for kind, status in ((1, 0), (1, 255), (2, 15), (3, 11)):
            _, bridge, session = self.session()
            session.admit()
            bridge.active_state, bridge.active_exit_kind, bridge.active_exit_status = 2, kind, status
            result = session.observe_active()
            first = session.read_active(first_terminal=True)
            self.assertEqual(first['record']['returncode'], status if kind == 1 else -status)
            self.assertEqual(first['raw_siginfo'], result['raw_siginfo'])
            first['raw_siginfo'] = b'caller mutation'
            self.assertEqual(session.observe_active()['sequence'], 2)
            retained = session.read_active(first_terminal=True)
            self.assertEqual(retained['sequence'], 1)
            self.assertEqual(retained['raw_siginfo'], b'authored nonconsuming raw')
            self.assertIs(retained['collection_evidence'], False)
            self.assertEqual(session.read_completion()[0], 'unresolved')

    def test_active_lost_mapping_recovers_cache_without_repeating_query(self):
        for loss in (MemoryError(), KeyboardInterrupt()):
            _, bridge, session = self.session()
            session.admit()
            bridge.actions['active_observe'] = [loss]
            with self.assertRaises(protocol.ReconciliationRequired):
                session.observe_active()
            self.assertEqual(bridge.calls.count('active_observe'), 1)
            cached = session.read_active()
            self.assertEqual(cached['sequence'], 1)
            self.assertIs(cached['fresh_native_call'], False)
            self.assertEqual(bridge.calls.count('active_observe'), 1)
            self.assertEqual(session.observe_active()['sequence'], 2)
            self.assertEqual(bridge.calls.count('active_observe'), 2)

    def test_active_preentry_loss_preserves_budget_and_only_explicit_later_query_retries(self):
        _, bridge, session = self.session()
        session.admit()
        bridge.actions['active_before'] = [MemoryError()]
        with self.assertRaises(protocol.ReconciliationRequired):
            session.observe_active()
        self.assertEqual(session.read_active()['sequence'], 0)
        self.assertEqual(bridge.calls.count('active_observe'), 1)
        self.assertEqual(session.observe_active()['sequence'], 1)
        self.assertEqual(session._active_attempts, 2)

    def test_active_malformed_replies_fail_closed_and_retain_exact_roots(self):
        mutations = (
            lambda a: a.update(bytes=343), lambda a: a.update(version=2),
            lambda a: a.update(generation=24), lambda a: a.update(generation=True),
            lambda a: a.update(fresh_native_call=False), lambda a: a.update(first_terminal=True),
            lambda a: a.update(collection_evidence=True), lambda a: a.update(raw_siginfo=b''),
            lambda a: a.update(returned=0), lambda a: a.update(birth_seconds=0),
            lambda a: a.pop('creator_pid'), lambda a: a['record'].update(reaped=1),
            lambda a: a['record'].update(pid=18), lambda a: a['record'].update(uid=99),
        )
        for mutate in mutations:
            _, bridge, session = self.session()
            session.admit()
            child, sink = bridge.child, bridge.sink
            original = bridge.active_observe
            def altered(value, original=original, mutate=mutate):
                result = original(value)
                mutate(result)
                return result
            bridge.active_observe = altered
            with self.assertRaises(protocol.BridgeProtocolError):
                session.observe_active()
            self.assertIs(session._child, child)
            self.assertIs(session._sink, sink)
            self.assertEqual(bridge.calls.count('active_observe'), 1)
            self.assertNotIn('child_reap', bridge.calls)
            self.assertNotIn('cleanup_begin', bridge.calls)

    def test_active_same_sequence_cache_mutation_is_rejected(self):
        _, bridge, session = self.session()
        session.admit()
        session.observe_active()
        bridge.active['raw_siginfo'] = b'changed retained bytes'
        with self.assertRaises(protocol.BridgeProtocolError):
            session.read_active()
        self.assertEqual(bridge.calls.count('active_observe'), 1)

    def test_active_birth_creator_and_clock_cannot_change_between_calls(self):
        changes = ({'birth_seconds': 124}, {'birth_microseconds': 46},
                   {'creator_pid': 8}, {'creator_uid': 13},
                   {'started_ns': 1, 'finished_ns': 2})
        for change in changes:
            _, bridge, session = self.session()
            session.admit()
            session.observe_active()
            bridge.actions['active_observe'] = [lambda b, change=change: b.active.update(change)]
            with self.assertRaises(protocol.BridgeProtocolError):
                session.observe_active()
            self.assertEqual(bridge.calls.count('active_observe'), 2)
            self.assertNotIn('cleanup_begin', bridge.calls)

    def test_active_phase_refusal_does_not_poison_cleanup_or_completed_read(self):
        _, bridge, session = self.session()
        session.admit()
        session.finish_step()
        self.assertEqual(bridge.calls.count('cleanup_begin'), 1)
        with self.assertRaises(protocol.Unsupported):
            session.observe_active()
        self.assertIsNone(session._fault)
        self.assertNotIn('active_observe', bridge.calls)
        self.assertEqual(self.complete(session)[0:2], ('exited', 0))
        with self.assertRaises(protocol.Unsupported):
            session.observe_active()
        self.assertEqual(session.read_completion()[0:2], ('exited', 0))

    def test_active_both_lifetime_budgets_refuse_before_query(self):
        for budget in ('attempts', 'sequence'):
            _, bridge, session = self.session()
            session.admit()
            if budget == 'attempts':
                session._active_attempts = protocol._ACTIVE_LIMIT
            else:
                session._active_sequence = protocol._ACTIVE_LIMIT
                bridge.active = active_report(protocol._ACTIVE_LIMIT)
            with self.assertRaises(protocol.Unsupported):
                session.observe_active()
            self.assertIsNone(session._fault)
            self.assertNotIn('active_observe', bridge.calls)

    def test_active_native_failure_facts_remain_readable_and_never_healthy(self):
        _, bridge, session = self.session()
        session.admit()
        def failed(b):
            b.active.update(rc=3, system_errno=5)
            b.active['record'].update(error=3, system_errno=5)
            b.r.update(blocked=1, rc=3, system_errno=5)
            raise RuntimeError('native entered observation failed')
        bridge.actions['active_observe'] = [failed]
        with self.assertRaises(protocol.ReconciliationRequired):
            session.observe_active()
        cached = session.read_active()
        self.assertEqual((cached['rc'], cached['record']['error']), (3, 3))
        self.assertIs(cached['fresh_native_call'], False)
        self.assertTrue(session._native_failure_seen)
        with self.assertRaises(protocol.Unsupported):
            session.observe_active()
        self.assertEqual(bridge.calls.count('active_observe'), 1)
        self.assertEqual(session.read_completion()[0], 'unresolved')

    def test_active_start_clock_failure_after_live_or_exit_keeps_both_caches_readable(self):
        for prior_state in (1, 2):
            _, bridge, session = self.session()
            session.admit()
            bridge.active_state = prior_state
            session.observe_active()
            first = session.read_active(first_terminal=True)
            def failed_start(b):
                # Actual C resets this slot/increments sequence BEFORE start
                # clock, and does not enter creator/disposition/wait/BSD calls.
                b.active.update(rc=3, system_errno=5, started_ns=0,
                                start_clock_rc=-1, start_clock_errno=5,
                                wait_entered=0, wait_returned=0, raw_siginfo=b'',
                                bsd_entered=0, bsd_returned=0, bsd_bytes=0, bsd_status=0,
                                creator_pid=0, creator_uid=0, observed_sid=0)
                b.active['record'] = record(state=0, error=3, system_errno=5,
                                             uid_evidence=2, sigchld_default=0, capacity=0)
                b.r.update(blocked=1, rc=3, system_errno=5)
                raise RuntimeError('native start clock failed')
            bridge.actions['active_observe'] = [failed_start]
            with self.assertRaises(protocol.ReconciliationRequired):
                session.observe_active()
            for _ in range(2):
                failed = session.read_active()
                self.assertEqual((failed['sequence'], failed['rc'], failed['started_ns']), (2, 3, 0))
                self.assertEqual(failed['start_clock_rc'], -1)
                self.assertEqual(failed['raw_siginfo'], b'')
                self.assertEqual(session.read_active(first_terminal=True), first)
                self.assertIsNone(session._fault)
            with self.assertRaises(protocol.Unsupported):
                session.observe_active()
            self.assertEqual(bridge.calls.count('active_observe'), 2)
            self.assertFalse(session.native_qualified)

    def test_active_terminal_cache_survives_real_mock_handoff_without_more_queries(self):
        _, bridge, session = self.session()
        session.admit()
        bridge.active_state = 2
        session.observe_active()
        first = session.read_active(first_terminal=True)
        self.assertEqual(self.complete(session)[0:2], ('exited', 0))
        self.assertIsNone(bridge.child)
        self.assertEqual(session.read_active(first_terminal=True), first)
        self.assertEqual(bridge.calls.count('active_observe'), 1)
        self.assertEqual(bridge.calls.count('child_reap'), 1)

    def test_active_first_terminal_and_consuming_wait_must_agree(self):
        _, bridge, session = self.session()
        session.admit()
        bridge.active_state, bridge.active_exit_status = 2, 7
        session.observe_active()
        self.assertEqual(self.complete(session)[0], 'unresolved')
        self.assertEqual(bridge.calls.count('child_reap'), 1)
        self.assertEqual(bridge.calls.count('active_observe'), 1)

    def test_active_first_terminal_cannot_be_erased_or_rewritten(self):
        for erase in (False, True):
            _, bridge, session = self.session()
            session.admit()
            bridge.active_state = 2
            session.observe_active()
            if erase:
                bridge.first_active = active_report()
                bridge.first_active['first_terminal'] = True
            else:
                bridge.first_active['raw_siginfo'] = b'rewritten first terminal'
            with self.assertRaises(protocol.BridgeProtocolError):
                session.read_active(first_terminal=True)
            self.assertEqual(bridge.calls.count('active_observe'), 1)

    def test_active_out_of_band_or_regressed_cache_is_not_adopted(self):
        for regress in (False, True):
            _, bridge, session = self.session()
            session.admit()
            if regress:
                session.observe_active()
            bridge.active = active_report(0 if regress else 1)
            before = bridge.calls.count('active_observe')
            with self.assertRaises(protocol.BridgeProtocolError):
                session.read_active()
            self.assertEqual(bridge.calls.count('active_observe'), before)

    def test_active_reentry_retains_roots_without_another_native_query(self):
        _, bridge, session = self.session()
        session.admit()
        bridge.actions['active_observe'] = [lambda b: session.read_active()]
        with self.assertRaises(protocol.BridgeProtocolError):
            session.observe_active()
        self.assertIs(session._child, bridge.child)
        self.assertIs(session._sink, bridge.sink)
        self.assertEqual(bridge.calls.count('active_observe'), 1)


if __name__ == '__main__':
    unittest.main()
