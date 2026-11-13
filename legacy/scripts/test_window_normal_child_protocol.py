"""Pure normal-Child errno controls; no native image, IO or process authority.

Run in its own interpreter. Existing authored records/Bridge fixtures exercise
the actual unmodified ChildExecution and validators. They do not authenticate a
native binding or qualify a normal executable, consuming wait or production.
"""
import builtins
import contextlib
import copy
import importlib.machinery
import os
import subprocess
import time
import unittest
from unittest import mock

import test_play_alpha_owned_child as fixtures
import window_owned_child_bridge as protocol


class Claim:
    def __init__(self, session):
        self.session, self.used = session, False

    def _admit(self, session):
        assert session is self.session and not self.used
        self.used = True
        return (b'/authored-only-never-executed',), b'/authored-only-never-opened'


class NormalErrnoTests(unittest.TestCase):
    def setUp(self):
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(mock.patch.object(builtins, '__import__', fixtures.guarded_import))
        stack.enter_context(mock.patch.object(builtins, 'open', fixtures.deny))
        for name in ('create_module', 'exec_module'):
            stack.enter_context(mock.patch.object(importlib.machinery.ExtensionFileLoader, name, fixtures.deny))
        for name in ('Popen', 'run', 'call', 'check_call', 'check_output'):
            stack.enter_context(mock.patch.object(subprocess, name, fixtures.deny))
        for name in tuple(vars(os)):
            if name.startswith(('exec', 'spawn')) or name in (
                'fork', 'forkpty', 'posix_spawn', 'system', 'popen', 'wait', 'wait3',
                'wait4', 'waitid', 'waitpid', 'kill', 'killpg', 'getpid', 'getppid',
                'getuid', 'geteuid', 'getgid', 'getegid', 'getpgid', 'getsid',
                'setsid', 'setpgid', 'open', 'read', 'write', 'close', 'stat',
                'lstat', 'fstat', 'statvfs', 'fstatvfs', 'fsync', 'fchmod',
                'listdir', 'scandir', 'mkfifo', 'chdir', 'unlink', 'remove', 'rename',
            ):
                stack.enter_context(mock.patch.object(os, name, fixtures.deny))
        for name in ('sleep', 'time', 'time_ns', 'monotonic', 'monotonic_ns',
                     'perf_counter', 'perf_counter_ns', 'process_time', 'thread_time'):
            if hasattr(time, name):
                stack.enter_context(mock.patch.object(time, name, fixtures.deny))

    def session(self, before_create=None):
        bridge = fixtures.MockBridge()
        if before_create is not None:
            bridge.actions['create'] = [before_create]
        session = protocol.ChildExecution.for_injected_mock(object(), bridge,
            bridge_type=fixtures.MockBridge, child_type=fixtures.MockChild,
            terminal_type=fixtures.MockSink)
        session._claim = Claim(session)  # Explicit synthetic claim, never PreparedLaunch.
        session.admit()
        self.assertFalse(session.native_qualified)
        return bridge, session

    def complete(self, session):
        for unused in range(9):
            session.finish_step()
        return session.read_completion()

    def assert_roots(self, bridge, session):
        self.assertTrue(any(s is session for s in protocol._CHILD_EXECUTIONS))
        self.assertIs(session._sink, bridge.sink)
        self.assertIsNotNone(session._child)
        self.assertFalse(session.native_qualified)
        self.assertEqual(bridge.calls.count('create'), 1)

    def active(self, field, value, state=1):
        bridge, session = self.session()
        bridge.active_state = state
        def alter(b):
            b.active[field] = value
            # Actual C copies the complete first terminal AFTER native calls.
            # Keep this existing authored fixture's early copy consistent.
            if b.first_active['entered'] and b.first_active['sequence'] == b.active['sequence']:
                b.first_active[field] = value
        bridge.actions['active_observe'] = [alter]
        return bridge, session

    def test_consuming_positive_wait_errno_preserves_certificate_and_exact_roots(self):
        bridge, session = self.session()
        bridge.actions['child_reap'] = [lambda b: b.t.update(wait_errno=31)]
        self.assertEqual(self.complete(session), ('exited', 0, None))
        self.assertEqual(bridge.t['wait_errno'], 31)
        original = copy.deepcopy(bridge.t)
        self.assertEqual(session.read_completion(), ('exited', 0, None))
        self.assertEqual(bridge.t, original)
        self.assertEqual(bridge.calls.count('child_reap'), 1)
        self.assert_roots(bridge, session)

    def test_terminal_failed_rc_or_bad_certificate_remains_unresolved(self):
        for changes in ({'wait_rc': -1, 'wait_errno': 31},
                        {'certificate_matches': 0, 'wait_errno': 31},
                        {'decoded': 0, 'wait_errno': 31}):
            bridge, session = self.session()
            bridge.actions['child_reap'] = [lambda b, c=changes: b.t.update(c)]
            self.assertNotEqual(self.complete(session)[0], 'exited')
            for unused in range(3):
                self.assertNotEqual(session.read_completion()[0], 'exited')
            self.assertEqual(bridge.calls.count('child_reap'), 1)
            self.assert_roots(bridge, session)

    def test_active_same_sequence_errno_mutation_is_still_refused(self):
        bridge, session = self.active('wait_errno', 31)
        self.assertEqual(session.observe_active()['wait_errno'], 31)
        bridge.active['wait_errno'] = 32
        with self.assertRaises(protocol.BridgeProtocolError):
            session.read_active()
        self.assertEqual(bridge.calls.count('active_observe'), 1)
        self.assertNotIn('cleanup_begin', bridge.calls)
        self.assert_roots(bridge, session)

    def test_negative_and_boolean_raw_errno_are_still_schema_errors(self):
        for value in (-1, True):
            bridge, session = self.active('wait_errno', value)
            with self.assertRaises(protocol.BridgeProtocolError):
                session.observe_active()
            self.assertNotIn('cleanup_begin', bridge.calls)
            self.assert_roots(bridge, session)


def ready_errno_case(field):
    def test(self):
        bridge, session = self.session(lambda b: b.r.update({field: 31}))
        self.assertEqual(self.complete(session), ('exited', 0, None))
        self.assertEqual(bridge.r[field], 31)
        self.assertEqual(bridge.calls.count('child_reap'), 1)
        self.assert_roots(bridge, session)
    return test


def active_errno_case(field, state):
    def test(self):
        bridge, session = self.active(field, 31, state)
        observed = session.observe_active()
        self.assertEqual(observed[field], 31)
        self.assertEqual(observed['record']['state'], state)
        self.assertEqual(session.read_active()[field], 31)
        if state == 2:
            self.assertEqual(session.read_active(first_terminal=True)[field], 31)
        self.assertNotIn('cleanup_begin', bridge.calls)
        self.assertNotIn('child_reap', bridge.calls)
        self.assert_roots(bridge, session)
    return test


def failed_rc_case(field):
    def test(self):
        bridge, session = self.session(lambda b: b.r.update({field: -1}))
        self.assertNotEqual(self.complete(session)[0], 'exited')
        self.assert_roots(bridge, session)
    return test


def failed_active_rc_case(field):
    def test(self):
        bridge, session = self.active(field, -1)
        with self.assertRaises(protocol.BridgeProtocolError):
            session.observe_active()
        self.assertNotIn('cleanup_begin', bridge.calls)
        self.assert_roots(bridge, session)
    return test


for _field in ('spawn_errno', 'resume_errno', 'sigchld_preflight_errno', 'clock_errno'):
    setattr(NormalErrnoTests, 'test_positive_ready_' + _field, ready_errno_case(_field))
for _state in (1, 2, 3):
    setattr(NormalErrnoTests, 'test_positive_active_wait_errno_state' + str(_state), active_errno_case('wait_errno', _state))
for _field in ('start_clock_errno', 'finish_clock_errno'):
    # Already diagnostic before the correction; preserve the existing behavior.
    setattr(NormalErrnoTests, 'test_positive_active_' + _field, active_errno_case(_field, 1))
for _field in ('spawn_rc', 'resume_rc', 'sigchld_preflight_rc', 'clock_rc'):
    setattr(NormalErrnoTests, 'test_failed_ready_' + _field, failed_rc_case(_field))
for _field in ('wait_rc', 'start_clock_rc', 'finish_clock_rc'):
    setattr(NormalErrnoTests, 'test_failed_active_' + _field, failed_active_rc_case(_field))


if __name__ == '__main__':
    unittest.main()
