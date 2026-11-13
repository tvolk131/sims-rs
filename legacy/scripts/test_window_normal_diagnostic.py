"""UNRUN guarded synthetic controls for Worker B's source-only consumer.

Parent filter (do NOT run in worker):
python3.9 -B -m unittest discover -s scripts -p test_window_normal_diagnostic.py -v

Uses the actual source ChildExecution and existing authored Python Bridge facts.
The stand-in Admission below tests forwarding/lost issuance only; it deliberately
uses the mock factory and does NOT implement/authenticate Worker A's real factory.
These are synthetic provenance controls, never native/host/image/type/IPC proof.
Real Admission, host EXIT_READY, packaging and native validation remain parent
work. Imports precede deny guards; tests perform no file/native/process actions.
The consumer's clock is replaced explicitly with an authored integer sequence.
"""

import builtins
import contextlib
import copy
import importlib.machinery
import os
import subprocess
import sys
import time
import types
import unittest
from unittest import mock

import test_play_alpha_owned_child as fixtures
import window_owned_child_bridge as protocol
import window_normal_diagnostic as diagnostic


class Clock:
    def __init__(self):
        self.now, self.calls = 1_000_000_000, 0

    def __call__(self):
        self.calls += 1
        return self.now


class SyntheticClaim:
    def __init__(self, session):
        self.session, self.calls = session, 0

    def _admit(self, session):
        if session is not self.session or self.calls:
            raise protocol.BridgeProtocolError('Synthetic claim is one-shot')
        self.calls += 1
        return (b'/usr/bin/true',), b'/synthetic-only-never-opened'


class SyntheticAdmission:
    """Test-only replacement module; NEVER used by the real integration path."""

    def __init__(self):
        self.execution = None
        self.binding, self.bridge = object(), fixtures.MockBridge()
        self.child_type, self.terminal_type = fixtures.MockChild, fixtures.MockSink
        self.root, self.manifest = object(), {'provenance': 'synthetic-only'}
        self.factory_calls = self.claim_calls = 0
        self.after_factory = self.before_admit = None

    def owns_execution(self, value):
        return value is self.execution and value is not None

    def create_execution(self):
        self.factory_calls += 1
        if self.factory_calls != 1:
            raise protocol.BridgeProtocolError('Synthetic factory replay')
        self.execution = protocol.ChildExecution.for_injected_mock(
            self.binding, self.bridge, bridge_type=fixtures.MockBridge,
            child_type=self.child_type, terminal_type=self.terminal_type,
            env=(b'PATH=/usr/bin:/bin', b'LC_ALL=C'), stdio=(9, 9, 9),
            launch_ns=10_000_000_000, cleanup_ns=10_000_000_000,
            grace_ns=100_000_000, max_entries=128)
        self.execution._claim = self  # Explicit TEST substitution, not native issuance.
        if self.after_factory is not None:
            self.after_factory(self)
        return self.execution

    def _admit(self, session):
        if not self.owns_execution(session) or self.claim_calls:
            raise protocol.BridgeProtocolError('Foreign synthetic execution')
        self.claim_calls += 1
        if self.before_admit is not None:
            self.before_admit(self)
        return (b'/usr/bin/true',), b'/synthetic-admission-cwd'


def lose(*arguments):
    raise MemoryError('authored reply loss')


class GuardedCase(unittest.TestCase):
    def setUp(self):
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(mock.patch.object(builtins, '__import__', fixtures.guarded_import))
        stack.enter_context(mock.patch.object(builtins, 'open', fixtures.deny))
        for name in ('create_module', 'exec_module'):
            stack.enter_context(mock.patch.object(importlib.machinery.ExtensionFileLoader,
                                                  name, fixtures.deny))
        for name in ('Popen', 'run', 'call', 'check_call', 'check_output'):
            stack.enter_context(mock.patch.object(subprocess, name, fixtures.deny))
        for name in tuple(vars(os)):
            if name.startswith(('exec', 'spawn')) or name in (
                'fork', 'forkpty', 'posix_spawn', 'posix_spawnp', 'system', 'popen',
                'wait', 'wait3', 'wait4', 'waitid', 'waitpid', 'kill', 'killpg',
                'getpid', 'getppid', 'getuid', 'geteuid', 'getgid', 'getegid',
                'getpgid', 'getsid', 'setsid', 'setpgid', 'setuid', 'seteuid',
                'open', 'read', 'write', 'close', 'stat', 'lstat', 'fstat', 'statvfs',
                'fstatvfs', 'fsync', 'fchmod', 'listdir', 'scandir', 'mkfifo',
                'chdir', 'unlink', 'remove', 'rename', 'replace',
            ):
                stack.enter_context(mock.patch.object(os, name, fixtures.deny))
        for name in ('sleep', 'time', 'time_ns', 'monotonic', 'monotonic_ns',
                     'perf_counter', 'perf_counter_ns', 'process_time', 'thread_time'):
            if hasattr(time, name):
                stack.enter_context(mock.patch.object(time, name, fixtures.deny))
        self.clock = Clock()
        stack.enter_context(mock.patch.object(diagnostic, '_monotonic_ns', self.clock))

    def make(self, active_state=2):
        bridge = fixtures.MockBridge()
        bridge.active_state = active_state
        session = protocol.ChildExecution.for_injected_mock(object(), bridge,
            bridge_type=fixtures.MockBridge, child_type=fixtures.MockChild,
            terminal_type=fixtures.MockSink)
        session._claim = SyntheticClaim(session)
        return diagnostic.Attempt.for_injected_mock(session), bridge, session

    def admitted(self, active_state=2):
        attempt, bridge, session = self.make(active_state)
        result = attempt.create_once()
        self.assertEqual(result['status'], 'waiting')
        self.assertTrue(result['needs_observation'])
        return attempt, bridge, session

    def observed(self):
        attempt, bridge, session = self.admitted()
        result = attempt.step(observation=True)
        self.assertEqual(result['first_active']['record']['state'], 2)
        self.assertFalse(result['needs_observation'])
        self.assertNotIn('cleanup_begin', bridge.calls)
        return attempt, bridge, session

    def finish(self, attempt):
        for unused in range(6):
            result = attempt.step()
        return result

    def assert_retained(self, attempt, bridge, session):
        self.assertTrue(any(item is attempt for item in diagnostic._ATTEMPTS))
        self.assertTrue(any(item is session for item in protocol._CHILD_EXECUTIONS))
        for root in (session, bridge, session._binding, session._claim):
            self.assertTrue(any(item is root for item in attempt._roots))
        if session._child is not None:
            self.assertTrue(any(item is session._child for item in attempt._roots))
        if session._sink is not None:
            self.assertTrue(any(item is session._sink for item in attempt._roots))
        result = attempt.cached()
        for name in ('native_qualified', 'continue_authorized', 'healthy'):
            self.assertIs(result[name], False)
            self.assertIs(getattr(attempt, name), False)
        self.assertNotIn('child_signal', bridge.calls)
        self.assertFalse(any(name.startswith('abort_') for name in bridge.calls))
        self.assertLessEqual(result['operations'], 32)
        self.assertLessEqual(result['active_dispatches'], 8)
        self.assertLessEqual(result['recovery_reads'], 4)

    def fake_admission(self):
        module = types.ModuleType('window_normal_admission')
        module.Admission = SyntheticAdmission
        patcher = mock.patch.dict(sys.modules, {'window_normal_admission': module})
        patcher.start()
        self.addCleanup(patcher.stop)
        return SyntheticAdmission()


class ForwardingTests(GuardedCase):
    def test_exact_admission_forwarding_and_scoped_detached_views(self):
        admission = self.fake_admission()
        admission.bridge.active_state = 2
        attempt = diagnostic.Attempt(admission)
        def before_create():
            self.assertIn(attempt, diagnostic._ATTEMPTS)
            self.assertIs(attempt._session, admission.execution)
            self.assertEqual(attempt._pending, 'admit')
            self.assertIsNotNone(attempt._deadline_ns)
        admission.bridge.on_create = before_create
        attempt.create_once()
        self.assertEqual(admission.factory_calls, 1)
        self.assertEqual(admission.claim_calls, 1)
        self.assertEqual(admission.bridge.arguments,
            (b'/usr/bin/true', (b'/usr/bin/true',), (b'PATH=/usr/bin:/bin', b'LC_ALL=C'),
             b'/synthetic-admission-cwd', (9, 9, 9), 10_000_000_000,
             10_000_000_000, 100_000_000, 128))
        attempt.step(observation=True)
        result = self.finish(attempt)
        self.assertTrue(result['completed'])
        self.assertEqual(result['scope'], 'private-normal-diagnostic')
        self.assertFalse(result['fresh'])
        self.assertFalse(result['retirement_proven'])
        view = attempt.read()
        self.assertTrue(view['fresh'])
        self.assertTrue(view['retirement_proven'])
        self.assertTrue(view['private_diagnostic_completed'])
        self.assert_retained(attempt, admission.bridge, admission.execution)

    def test_lost_factory_reply_recovers_issued_identity_without_factory_replay(self):
        admission = self.fake_admission()
        admission.after_factory = lose
        attempt = diagnostic.Attempt(admission)
        result = attempt.create_once()
        self.assertEqual(result['pending'], 'create_execution')
        original = admission.execution
        self.assertEqual(attempt.read()['status'], 'waiting')
        self.assertIs(attempt._session, original)
        attempt.create_once()  # Idempotent, never repeats factory or admit.
        self.assertEqual(admission.factory_calls, 1)
        attempt.step()
        self.assertEqual(admission.claim_calls, 1)
        self.assertEqual(admission.bridge.calls.count('create'), 1)
        self.assert_retained(attempt, admission.bridge, original)

    def test_factory_attribute_loss_reserves_intent_without_adopting_empty_execution(self):
        admission = self.fake_admission()
        attempt = diagnostic.Attempt(admission)
        def missing_factory(owner):
            self.assertTrue(attempt._factory_requested)
            self.assertEqual(attempt._pending, 'create_execution')
            lose()
        with mock.patch.object(SyntheticAdmission, 'create_execution', property(missing_factory)):
            result = attempt.create_once()
        self.assertEqual(result['pending'], 'create_execution')
        for unused in range(2):
            self.assertEqual(attempt.read()['pending'], 'create_execution')
            attempt.step()
            attempt.create_once()
        self.assertEqual(admission.factory_calls, 0)
        self.assertIsNone(admission.execution)
        self.assertFalse(attempt.cached()['completed'])

    def test_foreign_issuance_and_changed_admission_resources_are_refused(self):
        for changed in ('owns_execution', 'root', 'manifest', 'bridge'):
            with self.subTest(changed=changed):
                admission = self.fake_admission()
                attempt = diagnostic.Attempt(admission)
                if changed == 'owns_execution':
                    admission.owns_execution = lambda value: False
                    result = attempt.create_once()
                    self.assertNotIn('create', admission.bridge.calls)
                else:
                    attempt.create_once()
                    setattr(admission, changed, object())
                    result = attempt.step(observation=True)
                self.assertEqual(result['status'], 'retained')
                self.assertFalse(result['completed'])

    def test_mock_seam_refuses_non_session_missing_claim_and_native_type_shapes(self):
        for value in (None, 17, object(), self.fake_admission()):
            before = len(diagnostic._ATTEMPTS)
            with self.assertRaises(protocol.BridgeProtocolError):
                diagnostic.Attempt.for_injected_mock(value)
            refused = diagnostic._ATTEMPTS[-1]
            self.assertEqual(len(diagnostic._ATTEMPTS), before + 1)
            self.assertTrue(any(item is value for item in refused._roots) or value is None)
        for change in ('claim', 'child_type'):
            bridge = fixtures.MockBridge()
            session = protocol.ChildExecution.for_injected_mock(object(), bridge,
                bridge_type=fixtures.MockBridge, child_type=fixtures.MockChild,
                terminal_type=fixtures.MockSink)
            session._claim = SyntheticClaim(session)
            if change == 'claim':
                session._claim = None
            else:
                session._child_type = type(None)  # Nonheap stand-in for prohibited native type.
            with self.assertRaises(protocol.BridgeProtocolError):
                diagnostic.Attempt.for_injected_mock(session)
            self.assertNotIn('create', bridge.calls)

    def test_copy_serialization_reinitialization_subclass_and_reentry_refused(self):
        attempt, bridge, session = self.make()
        for copier in (copy.copy, copy.deepcopy):
            with self.assertRaises(TypeError):
                copier(attempt)
        with self.assertRaises(TypeError):
            attempt.__reduce_ex__(4)
        class Alternate(diagnostic.Attempt):
            pass
        with self.assertRaises(protocol.BridgeProtocolError):
            Alternate.for_injected_mock(session)
        def reenter():
            with self.assertRaises(protocol.BridgeProtocolError):
                attempt.step(observation=True)
        bridge.on_create = reenter
        result = attempt.create_once()
        self.assertFalse(result['completed'])
        self.assertEqual(bridge.calls.count('create'), 1)
        with self.assertRaises(protocol.BridgeProtocolError):
            attempt.__init__(object())
        self.assertIs(attempt._session, session)


class NormalFlowTests(GuardedCase):
    def test_immediate_terminal_then_exact_six_steps_and_fresh_retirement(self):
        attempt, bridge, session = self.observed()
        for index, operation in enumerate(diagnostic._CLEANUP, 1):
            before = len(session._intent)
            result = attempt.step()
            self.assertEqual(len(session._intent), before + 1)
            self.assertEqual(session._intent[-1][0], operation)
            self.assertEqual(result['cleanup_index'], index)
        self.assertTrue(result['completed'])
        self.assertFalse(result['private_diagnostic_completed'])
        self.assertEqual(result['scope'], 'injected-mock')
        self.assertFalse(attempt.cached()['retirement_proven'])
        view = attempt.read()
        self.assertTrue(view['retirement_proven'])
        self.assertTrue(view['intent_resolved'])
        self.assertEqual(view['terminal']['kind'], 2)
        self.assertEqual(view['terminal']['final_report']['state'], 11)
        self.assertEqual(view['first_active'], view['first_terminal'])
        self.assert_retained(attempt, bridge, session)

    def test_live_transition_terminal_requires_scheduling_without_cleanup_polling(self):
        attempt, bridge, session = self.admitted(active_state=1)
        self.assertTrue(attempt.step(observation=True)['needs_observation'])
        for unused in range(3):
            attempt.step()
        bridge.active_state = 3
        self.clock.now += diagnostic.QUERY_DELAY_NS - 1
        attempt.step(observation=True)
        self.assertEqual(bridge.calls.count('active_observe'), 1)
        self.clock.now += 1
        transition = attempt.step(observation=True)
        self.assertEqual(transition['active']['record']['state'], 3)
        self.assertTrue(transition['needs_observation'])
        self.assertNotIn('cleanup_begin', bridge.calls)
        bridge.active_state = 2
        self.clock.now += diagnostic.QUERY_DELAY_NS
        self.assertEqual(attempt.step(observation=True)['first_active']['record']['state'], 2)
        self.assertTrue(self.finish(attempt)['completed'])
        self.assert_retained(attempt, bridge, session)

    def test_read_cached_are_memory_only_and_all_returned_facts_are_detached(self):
        attempt, bridge, session = self.observed()
        calls = self.clock.calls
        effects = (bridge.calls.count('active_observe'), len(session._intent))
        with mock.patch.object(diagnostic, '_monotonic_ns', fixtures.deny):
            view = attempt.read()
            view['terminal']['final_report']['registered'] = 0
            view['first_active']['record']['exit_status'] = 9
            cached = attempt.cached()
        self.assertEqual(self.clock.calls, calls)
        self.assertEqual((bridge.calls.count('active_observe'), len(session._intent)), effects)
        self.assertEqual(cached['terminal']['final_report']['registered'], 1)
        self.assertEqual(cached['first_active']['record']['exit_status'], 0)
        self.assertFalse(cached['fresh'])
        self.assert_retained(attempt, bridge, session)

    def test_positive_successful_raw_errno_is_preserved_not_normalized_failure(self):
        attempt, bridge, session = self.make()
        bridge.actions['create'] = [lambda b: b.r.update(
            spawn_errno=31, resume_errno=32, sigchld_preflight_errno=33, clock_errno=34)]
        attempt.create_once()
        def active_errno(b):
            b.active.update(wait_errno=35, start_clock_errno=36, finish_clock_errno=37)
            b.first_active.update(wait_errno=35, start_clock_errno=36, finish_clock_errno=37)
        bridge.actions['active_observe'] = [active_errno]
        attempt.step(observation=True)
        bridge.actions['child_reap'] = [lambda b: b.t.update(wait_errno=38)]
        result = self.finish(attempt)
        self.assertTrue(result['completed'])
        self.assertEqual(result['active']['wait_errno'], 35)
        self.assertEqual(result['terminal']['wait_errno'], 38)
        self.assertEqual(result['terminal']['final_report']['spawn_errno'], 31)
        self.assert_retained(attempt, bridge, session)


class LostReplyTests(GuardedCase):
    def test_admit_attribute_loss_burns_create_intent_before_method_access(self):
        attempt, bridge, session = self.make()
        error = MemoryError('synthetic admit attribute loss')
        def missing_admit(owner):
            self.assertIs(owner, session)
            self.assertTrue(attempt._admit_requested)
            self.assertEqual(attempt._pending, 'admit')
            raise error
        with mock.patch.object(protocol.ChildExecution, 'admit', property(missing_admit)):
            self.assertEqual(attempt.create_once()['pending'], 'admit')
        attempt.create_once()
        attempt.step(observation=True)
        self.assertNotIn('create', bridge.calls)
        self.assertEqual(attempt.read()['status'], 'retained')
        self.assertTrue(any(item is error for item in attempt._errors))
        self.assertEqual(session._claim.calls, 0)

    def test_lost_create_reply_recovers_same_child_and_never_readmits(self):
        attempt, bridge, session = self.make()
        error = KeyboardInterrupt('synthetic create return loss')
        bridge.actions['create'] = [error]
        result = attempt.create_once()
        self.assertEqual(result['pending'], 'admit')
        self.assertFalse(result['completed'])
        self.assertEqual(attempt.read()['status'], 'waiting')
        attempt.create_once()
        attempt.step(observation=True)
        self.assertTrue(self.finish(attempt)['completed'])
        self.assertEqual(session._claim.calls, 1)
        self.assertEqual(bridge.calls.count('create'), 1)
        self.assertTrue(any(item is error for item in attempt._errors))
        self.assert_retained(attempt, bridge, session)

    def test_lost_active_reply_recovers_original_slot_before_any_next_query(self):
        attempt, bridge, session = self.admitted(active_state=1)
        error = MemoryError('synthetic active mapping loss')
        bridge.actions['active_observe'] = [error]
        self.assertEqual(attempt.step(observation=True)['pending'], 'observe_active')
        self.clock.now += diagnostic.QUERY_DELAY_NS
        attempt.step(observation=True)
        self.assertEqual(bridge.calls.count('active_observe'), 1)
        result = attempt.read()
        self.assertTrue(result['intent_resolved'])
        self.assertEqual(result['active']['sequence'], 1)
        bridge.active_state = 2
        attempt.step(observation=True)
        self.assertEqual(bridge.calls.count('active_observe'), 2)
        self.assertTrue(self.finish(attempt)['completed'])
        self.assert_retained(attempt, bridge, session)

    def test_lost_cleanup_latch_reap_handoff_reconcile_without_replay(self):
        for operation in diagnostic._CLEANUP:
            with self.subTest(operation=operation):
                attempt, bridge, session = self.observed()
                error = MemoryError('synthetic ' + operation + ' mapping loss')
                bridge.actions[operation] = [error]
                result = self.finish(attempt)
                if result['pending'] is not None:
                    attempt.read()
                    result = attempt.step()
                self.assertTrue(result['completed'])
                self.assertEqual(bridge.calls.count(operation), 1)
                self.assertTrue(any(item is error for item in attempt._errors))
                self.assert_retained(attempt, bridge, session)

    def test_unknown_preentry_active_and_consuming_intents_never_retried(self):
        for operation in ('active', 'reap', 'handoff'):
            with self.subTest(operation=operation):
                attempt, bridge, session = self.admitted()
                if operation == 'active':
                    bridge.actions['active_before'] = [lose]
                    attempt.step(observation=True)
                else:
                    attempt.step(observation=True)
                    steps = 4 if operation == 'reap' else 5
                    for unused in range(steps):
                        attempt.step()
                    # Method-access loss occurs AFTER the adapter reserves its intent.
                    method = 'child_reap' if operation == 'reap' else 'handoff'
                    with mock.patch.object(bridge, method, lose):
                        attempt.step()
                pending = attempt.cached()['pending']
                self.assertIsNotNone(pending)
                for unused in range(2):
                    attempt.read()
                    attempt.step(observation=True)
                self.assertEqual(attempt.cached()['pending'], pending)
                self.assertFalse(attempt.cached()['completed'])
                self.assert_retained(attempt, bridge, session)

    def test_failed_memory_read_retains_original_error_and_later_facts(self):
        attempt, bridge, session = self.observed()
        error = MemoryError('synthetic retained snapshot loss')
        bridge.actions['snapshot'] = [error]
        self.assertFalse(attempt.read()['fresh'])
        self.assertTrue(any(item is error for item in attempt._errors))
        self.assertTrue(attempt.read()['fresh'])
        self.assertTrue(self.finish(attempt)['completed'])
        self.assert_retained(attempt, bridge, session)


class RefusalTests(GuardedCase):
    def test_fresh_finish_reread_cannot_select_signal_after_safe_preflight(self):
        for changed in ('live', 'extra_members'):
            attempt, bridge, session = self.observed()
            for unused in range(3):
                attempt.step()
            self.assertEqual(attempt.cached()['cleanup_index'], 3)
            self.assertTrue(attempt._guarded)
            def change_during_finish(b):
                if changed == 'live':
                    b.r['record'] = fixtures.record()
                else:
                    b.r['record'].update(count=2, member1=18, raw_bytes=8)
            bridge.actions['snapshot'] = [change_during_finish]
            result = attempt.step()
            self.assertEqual(result['status'], 'retained')
            self.assertNotIn('child_signal', bridge.calls)
            self.assertNotIn('child_latch', bridge.calls)
            self.assertEqual(len(session._intent), 3)
            self.assert_retained(attempt, bridge, session)

    def test_permanent_operation_guard_cannot_be_replaced_or_bypassed(self):
        attempt, bridge, session = self.observed()
        attempt.step()
        original = attempt._operation_method
        session._operation = original
        self.assertEqual(attempt.step()['status'], 'retained')
        self.assertNotIn('child_observe', bridge.calls)
        self.assertFalse(attempt.cached()['completed'])

    def test_nonzero_first_exit_and_signal_exit_never_begin_cleanup(self):
        for kind, status in ((1, 4), (2, 9), (3, 6)):
            attempt, bridge, session = self.admitted()
            bridge.active_exit_kind, bridge.active_exit_status = kind, status
            self.assertEqual(attempt.step(observation=True)['status'], 'retained')
            self.assertNotIn('cleanup_begin', bridge.calls)
            self.assert_retained(attempt, bridge, session)

    def test_unexpected_live_or_extra_members_stop_without_signals(self):
        for change in ('live', 'members', 'raw_bytes'):
            attempt, bridge, session = self.observed()
            if change == 'live':
                bridge.exited = False
            elif change == 'members':
                bridge.members = (17, 18)
            else:
                bridge.actions['child_members'] = [lambda b: b.r['record'].update(raw_bytes=8)]
            self.assertFalse(self.finish(attempt)['completed'])
            self.assertNotIn('child_latch', bridge.calls)
            self.assert_retained(attempt, bridge, session)

    def test_changed_owner_registry_child_sink_generation_or_first_exit_is_sticky(self):
        for changed in ('owner', 'registry', 'child', 'sink', 'generation', 'first'):
            with self.subTest(changed=changed):
                attempt, bridge, session = self.observed()
                if changed == 'owner':
                    session._bridge = object()
                elif changed == 'registry':
                    bridge.history = lambda: ()
                elif changed == 'child':
                    bridge.child = fixtures.MockChild()
                elif changed == 'sink':
                    bridge.sink = fixtures.MockSink(bridge)
                elif changed == 'generation':
                    bridge.t['generation'] += 1
                else:
                    bridge.first_active['record']['exit_status'] = 1
                self.assertEqual(attempt.read()['status'], 'retained')
                attempt.step()
                self.assertNotIn('cleanup_begin', bridge.calls)
                self.assertFalse(attempt.cached()['completed'])
                self.assertTrue(any(item is session for item in attempt._roots))

    def test_consuming_disagreement_uncertified_and_untransferred_never_complete(self):
        for changed in ('exit', 'certificate', 'raw', 'wait_rc', 'wait_code', 'handoff'):
            with self.subTest(changed=changed):
                attempt, bridge, session = self.observed()
                def alter(b):
                    if changed == 'exit':
                        b.t['decoded_record'].update(exit_status=1, returncode=1)
                        b.r['record'].update(exit_status=1, returncode=1)
                    elif changed == 'certificate':
                        b.t['certificate_matches'] = 0
                    elif changed == 'raw':
                        b.t['raw_siginfo'] = b''
                    elif changed == 'wait_rc':
                        b.t.update(wait_rc=-1, wait_errno=31)
                    elif changed == 'wait_code':
                        b.t['wait_code'] = 2
                if changed == 'handoff':
                    bridge.handoff = lose
                else:
                    bridge.actions['child_reap'] = [alter]
                self.assertFalse(self.finish(attempt)['completed'])
                self.assertLessEqual(bridge.calls.count('child_reap'), 1)
                self.assert_retained(attempt, bridge, session)

    def test_historical_failure_resources_abort_and_out_of_band_cleanup_retained(self):
        for changed in ('launch', 'resource', 'abort', 'foreign_cleanup'):
            attempt, bridge, session = self.observed()
            if changed == 'launch':
                bridge.r['launch_failure_mask'] = 1
            elif changed == 'resource':
                bridge.r['resource_error'] = 3
            elif changed == 'abort':
                bridge.r['abort']['started'] = 1
            else:
                session._intent.append(('child_signal', (1,)))
            self.assertEqual(attempt.read()['status'], 'retained')
            self.assertNotIn('child_reap', bridge.calls)
            self.assert_retained(attempt, bridge, session)

    def test_empty_history_after_lost_create_never_invents_no_creation(self):
        attempt, bridge, session = self.make()
        def erase_reply(b):
            b.child = b.sink = None
            lose()
        bridge.actions['create'] = [erase_reply]
        attempt.create_once()
        result = attempt.read()
        self.assertEqual(result['status'], 'retained')
        self.assertFalse(result['retirement_proven'])
        attempt.create_once()
        self.assertEqual(bridge.calls.count('create'), 1)
        self.assertIs(attempt._session, session)


class BudgetTests(GuardedCase):
    def test_native_deadline_or_budget_replacement_refused_without_next_operation(self):
        for changed in ('cleanup_deadline_ns', 'max_cleanup_entries'):
            attempt, bridge, session = self.observed()
            attempt.step()
            bridge.r[changed] += 1
            self.assertEqual(attempt.step()['status'], 'retained')
            self.assertNotIn('child_observe', bridge.calls)
            self.assert_retained(attempt, bridge, session)

    def test_fixed_deadline_nonrenewal_and_expiry_before_create_or_cleanup(self):
        attempt, bridge, session = self.admitted(active_state=1)
        deadline = attempt.cached()['deadline_ns']
        self.assertEqual(deadline, self.clock.now + diagnostic.DEADLINE_NS)
        self.clock.now += 12_000_000_000
        attempt.step(observation=True)
        self.assertEqual(attempt.cached()['deadline_ns'], deadline)
        self.clock.now = deadline
        self.assertEqual(attempt.step(observation=True)['status'], 'retained')
        self.assertNotIn('cleanup_begin', bridge.calls)
        self.assert_retained(attempt, bridge, session)

    def test_boolean_negative_overflow_and_regressed_monotonic_values_refused(self):
        for value in (True, -1, 1.0, 2 ** 64, 2 ** 64 - diagnostic.DEADLINE_NS):
            attempt, bridge, session = self.make()
            self.clock.now = value
            self.assertEqual(attempt.create_once()['status'], 'retained')
            self.assertNotIn('create', bridge.calls)
        self.clock.now = 1_000_000_000
        attempt, bridge, session = self.admitted()
        self.clock.now -= 1
        self.assertEqual(attempt.step(observation=True)['status'], 'retained')
        self.assertNotIn('active_observe', bridge.calls)

    def test_active_and_memory_recovery_caps_are_reserved_without_dispatch(self):
        attempt, bridge, session = self.admitted(active_state=1)
        for unused in range(8):
            attempt.step(observation=True)
            self.clock.now += diagnostic.QUERY_DELAY_NS
        self.assertEqual(bridge.calls.count('active_observe'), 8)
        self.assertEqual(attempt.step(observation=True)['status'], 'retained')
        self.assertEqual(bridge.calls.count('active_observe'), 8)
        other, b, s = self.admitted()
        for unused in range(4):
            other.read()
        before = len(b.calls)
        self.assertEqual(other.read()['status'], 'retained')
        self.assertEqual(len(b.calls), before)
        self.assert_retained(other, b, s)

    def test_coordinator_cap_and_exact_observation_permission(self):
        attempt, bridge, session = self.admitted()
        # The normal finite path uses fewer than32 effects. Inject exhaustion
        # solely to reach the defensive coordinator guard without native work.
        attempt._operations = 32
        self.assertEqual(attempt.step(observation=True)['status'], 'retained')
        self.assertNotIn('active_observe', bridge.calls)
        other, b, s = self.admitted()
        self.assertEqual(other.step(observation=1)['status'], 'retained')
        self.assertNotIn('active_observe', b.calls)
        self.assert_retained(other, b, s)

    def test_consumption_followed_by_deadline_refusal_retains_consumed_facts(self):
        attempt, bridge, session = self.observed()
        for unused in range(4):
            attempt.step()
        deadline = attempt.cached()['deadline_ns']
        bridge.actions['child_reap'] = [lambda b: setattr(self.clock, 'now', deadline)]
        self.assertEqual(attempt.step()['status'], 'retained')
        self.assertEqual(bridge.t['consumed'], 1)
        result = attempt.read()
        self.assertEqual(result['terminal']['consumed'], 1)
        self.assertFalse(result['completed'])
        self.assertFalse(result['retirement_proven'])
        attempt.step()
        self.assertNotIn('handoff', bridge.calls)
        self.assertEqual(bridge.calls.count('child_reap'), 1)
        self.assert_retained(attempt, bridge, session)

    def test_recovered_handoff_requires_deadline_checked_step_then_new_read(self):
        attempt, bridge, session = self.observed()
        for unused in range(5):
            attempt.step()
        # Lose only the consumer's post-handoff memory read. Source handoff
        # itself succeeds, and read() later proves identity without an OS clock.
        original = session.read_completion
        remaining = [True]
        def memory_loss():
            if remaining:
                remaining.pop()
                raise MemoryError('synthetic post-handoff read loss')
            return original()
        with mock.patch.object(session, 'read_completion', memory_loss):
            result = attempt.step()
        self.assertFalse(result['completed'])
        self.assertEqual(attempt.read()['status'], 'completion_candidate')
        self.assertFalse(attempt.cached()['retirement_proven'])
        self.assertTrue(attempt.step()['completed'])
        self.assertTrue(attempt.read()['retirement_proven'])
        self.assertEqual(bridge.calls.count('handoff'), 1)
        self.assert_retained(attempt, bridge, session)


if __name__ == '__main__':
    unittest.main()
