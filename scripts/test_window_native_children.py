"""Python guards and synthetic private recovery controls; no C/native proof."""
import builtins
import copy
import sys
import unittest
from unittest.mock import patch

import window_native_children as candidate


class Poison:
    def __getattribute__(self, name):
        raise AssertionError('Guard touched argument attribute: ' + name)

    def __iter__(self):
        raise AssertionError('Guard iterated argument')

    def __fspath__(self):
        raise AssertionError('Guard probed path')


class DisabledEntryControls(unittest.TestCase):
    def test_public_entries_raise_before_import_open_or_argument_probe(self):
        poison = Poison()
        entries = (candidate.production_manager, candidate.require_launch_ready,
                   candidate.NativeChildren)
        for entry in entries:
            with patch.object(builtins, '__import__', side_effect=AssertionError('import')), \
                 patch.object(builtins, 'open', side_effect=AssertionError('I/O')):
                with self.assertRaises(candidate.Unsupported):
                    entry(poison, backend=poison, clock=poison, loader=poison)

    def test_unavailable_bootstrap_raises_before_native_import(self):
        with patch.object(builtins, '__import__', side_effect=AssertionError('native import')), \
             patch.object(builtins, 'open', side_effect=AssertionError('I/O')):
            with self.assertRaises(candidate.Unsupported):
                candidate.NativeChildren._after_reviewed_bootstrap()
        self.assertTrue(candidate.QUALIFICATIONS)
        self.assertTrue(all(value is False for value in candidate.QUALIFICATIONS.values()))


class SyntheticChild:
    """Private controller stimulus, not an extension handle or ownership proof."""


class SyntheticSink:
    def __init__(self, bridge):
        self.bridge = bridge
        self.transferred = 0
        self.fail_after_transfer = False
        self.decoded_overrides = {}
        self.terminal_overrides = {}

    def snapshot(self):
        if self.transferred and self.fail_after_transfer:
            raise MemoryError('synthetic terminal reply allocation failure')
        decoded = copy.deepcopy(self.bridge.report['record'])
        decoded.update(self.decoded_overrides)
        result = {'generation': 1, 'rooted': 1, 'transferred': self.transferred,
                'reap_attempted': 1, 'wait_entered': 1, 'wait_returned': 1,
                'wait_rc': 0, 'wait_pid': 17, 'wait_uid': 0,
                'consumed': 1, 'decoded': 1, 'certificate_matches': 1,
                'decoded_record': decoded,
                'final_report': copy.deepcopy(self.bridge.report)}
        result.update(self.terminal_overrides)
        return result


class SyntheticBridge:
    def __init__(self, already_reaped):
        self.child = SyntheticChild()
        self.sink = SyntheticSink(self)
        self.calls = []
        self.report = {'generation': 1, 'state': candidate.REAP_RECORDED if already_reaped else candidate.ACTIVE,
            'rc': 0, 'system_errno': 0, 'registered': 1, 'pid_valid': 1, 'blocked': 0,
            'spawn_entered': 1, 'spawn_returned': 1, 'spawn_rc': 0, 'spawn_pid_raw': 17,
            'registration_pid': 17, 'registration_pgid': 17, 'registration_uid': 12,
            'registration_ppid': 7, 'registration_sid': 17,
            'latch_attempted': 1, 'cleanup_started': 1, 'cleanup_entries': 5,
            'max_cleanup_entries': 128, 'cleanup_deadline_ns': 100, 'cleanup_now_ns': 10,
            'record': {'pid': 17, 'pgid': 17, 'uid': 12, 'ppid': 7, 'sid': 17,
                       'state': candidate.EXITED, 'exit_kind': 1, 'exit_status': 0,
                       'wait_uid_available': 0, 'uid_evidence': candidate.UID_RETAINED,
                       'sigchld_default': 1, 'no_cldwait': 0, 'pid_bytes': 4, 'capacity': 2,
                       'no_signals': 1, 'reap_attempted': int(already_reaped), 'reaped': int(already_reaped)}}
        self.leader_record = copy.deepcopy(self.report['record'])

    def child_snapshot(self, child):
        assert child is self.child
        self.calls.append('snapshot')
        return copy.deepcopy(self.report)

    def child_reap(self, child):
        assert child is self.child and self.report['state'] == candidate.ACTIVE
        self.calls.append('reap')
        self.report['cleanup_entries'] += 1
        self.report['state'] = candidate.REAP_RECORDED
        self.report['record'].update(reap_attempted=1, reaped=1)
        return copy.deepcopy(self.report)

    def child_observe(self, child):
        assert child is self.child and self.report['state'] == candidate.ACTIVE
        self.calls.append('observe')
        self.report['cleanup_entries'] += 1
        self.report['record'] = copy.deepcopy(self.leader_record)
        return copy.deepcopy(self.report)

    def child_members(self, child):
        assert child is self.child and self.report['state'] == candidate.ACTIVE
        self.calls.append('members')
        self.report['cleanup_entries'] += 1
        self.report['record'] = copy.deepcopy(self.leader_record)
        self.report['record'].update(state=0, exit_kind=0, exit_status=0, count=1,
                                     member0=17, member1=0, raw_bytes=4,
                                     uid_evidence=candidate.UID_RETAINED)
        return copy.deepcopy(self.report)

    def child_latch(self, child, exit_kind, exit_status):
        assert child is self.child and self.report['state'] == candidate.ACTIVE
        assert self.report['latch_attempted'] == 0
        assert (exit_kind, exit_status) == (1, 0)
        self.calls.append('latch')
        self.report['cleanup_entries'] += 1
        self.report['latch_attempted'] = 1
        self.report['record'].update(no_signals=1)
        return copy.deepcopy(self.report)

    def terminal(self, child):
        assert child is self.child
        self.calls.append('terminal')
        return self.sink

    def handoff(self, child, sink):
        assert child is self.child and sink is self.sink
        self.calls.append('handoff')
        self.report['cleanup_entries'] += 1
        sink.transferred = 1
        return sink

    def history(self):
        return (self.sink,)

    def create(self, *arguments):
        raise AssertionError('Widened cap reached the synthetic bridge')

    def __getattr__(self, name):
        raise AssertionError('Repeated/reset native step: ' + name)


class SyntheticRecoveryControls(unittest.TestCase):
    """Private Python control-flow checks only; no authentic native injection API."""

    def manager(self, already_reaped):
        # Deliberate private-state construction cannot qualify a C handle/backend.
        # The actual public constructor/bootstrap remain hard-disabled above.
        bridge = SyntheticBridge(already_reaped)
        manager = object.__new__(candidate.NativeChildren)
        manager._native = type('SyntheticTypes', (), {'Child': SyntheticChild, 'TerminalSink': SyntheticSink})
        manager._bridge = bridge
        manager._receipts = {}
        manager._busy = False
        return manager, bridge

    def test_latch_reply_loss_continues_without_observe_or_repeated_latch(self):
        manager, bridge = self.manager(False)
        receipt = manager.cleanup(bridge.child)
        self.assertTrue(receipt['resolved'])
        self.assertEqual(bridge.calls, ['snapshot', 'reap', 'terminal', 'handoff'])
        self.assertEqual(bridge.report['cleanup_entries'], 7)
        self.assertEqual(bridge.report['cleanup_deadline_ns'], 100)

    def test_reap_reply_loss_hands_off_without_another_consuming_wait(self):
        manager, bridge = self.manager(True)
        receipt = manager.cleanup(bridge.child)
        self.assertTrue(receipt['resolved'])
        self.assertEqual(bridge.calls, ['snapshot', 'terminal', 'handoff'])
        self.assertEqual(bridge.report['cleanup_entries'], 6)
        self.assertEqual(bridge.report['cleanup_deadline_ns'], 100)

    def test_interrupt_after_busy_store_does_not_strand_create_or_cleanup(self):
        for operation in ('create', 'cleanup'):
            manager, bridge = self.manager(True)
            method = getattr(manager, operation)
            fired = []

            def interrupt(frame, event, argument):
                if frame.f_code is method.__func__.__code__:
                    frame.f_trace_opcodes = True
                    if event == 'opcode' and manager._busy and not fired:
                        fired.append(True)
                        raise KeyboardInterrupt('synthetic opcode interruption')
                return interrupt

            previous = sys.gettrace()
            try:
                sys.settrace(interrupt)
                with self.assertRaises(KeyboardInterrupt):
                    if operation == 'cleanup':
                        method(bridge.child)
                    else:
                        method(executable=b'/not-executed', argv=(), env=(), cwd=b'/', stdio=(0, 1, 2))
            finally:
                sys.settrace(previous)
            self.assertTrue(fired)
            self.assertFalse(manager._busy, operation)
            self.assertEqual(bridge.calls, [])

    def test_manager_rejects_widened_cap_before_any_native_call(self):
        manager, bridge = self.manager(True)
        for cap in (0, 513, 4096, True):
            with self.subTest(cap=cap), self.assertRaises(candidate.ProtocolError):
                manager.create(executable=b'/not-executed', argv=(), env=(), cwd=b'/',
                               stdio=(0, 1, 2), max_calls=cap)
        self.assertFalse(manager._busy)
        self.assertEqual(bridge.calls, [])

    def test_terminal_mapping_failure_keeps_sink_without_resolved_success(self):
        manager, bridge = self.manager(True)
        bridge.sink.fail_after_transfer = True
        with self.assertRaises(MemoryError):
            manager.cleanup(bridge.child)
        self.assertFalse(manager._receipts[bridge.child]['resolved'])
        self.assertIs(manager.history()[0], bridge.sink)
        self.assertEqual(bridge.sink.transferred, 1)
        self.assertEqual(bridge.calls, ['snapshot', 'terminal', 'handoff'])

    def test_cleanup_recovers_registration_anchor_after_member_record(self):
        manager, bridge = self.manager(False)
        bridge.report['latch_attempted'] = 0
        bridge.leader_record['no_signals'] = 0
        bridge.report['record'].update(pid=19, state=candidate.LIVE, exit_kind=0, exit_status=0,
                                      no_signals=0, uid_evidence=candidate.UID_CURRENT)
        receipt = manager.cleanup(bridge.child)
        self.assertTrue(receipt['resolved'])
        self.assertEqual(receipt['anchor'], (1, 17, 17, 12, 7, 17))
        self.assertEqual(bridge.calls, ['snapshot', 'observe', 'members', 'observe',
                                        'latch', 'reap', 'terminal', 'handoff'])
        self.assertEqual(bridge.report['cleanup_deadline_ns'], 100)

    def test_registered_anchor_is_immutable_and_rejects_changed_registration(self):
        manager, bridge = self.manager(True)
        receipt = manager._receipt(bridge.child, bridge.report)
        anchor = manager._registered_anchor(bridge.child, bridge.report, receipt)
        with self.assertRaises(TypeError):
            anchor[1] = 19
        bridge.report['record'].update(pid=19, uid=99)
        self.assertIs(manager._registered_anchor(bridge.child, bridge.report, receipt), anchor)
        bridge.report['registration_uid'] = 99
        with self.assertRaises(candidate.ProtocolError):
            manager._registered_anchor(bridge.child, bridge.report, receipt)
        bridge.report['registration_uid'] = 12
        bridge.report['generation'] = 2
        with self.assertRaises(candidate.ProtocolError):
            manager._registered_anchor(bridge.child, bridge.report, receipt)
        self.assertEqual(bridge.calls, [])

    def test_cleanup_rejects_unregistered_or_failed_spawn_before_native_cleanup(self):
        mutations = ({'registered': 0}, {'pid_valid': 0}, {'spawn_entered': 0},
                     {'spawn_returned': 0}, {'spawn_rc': 5}, {'spawn_pid_raw': 19},
                     {'registration_pid': 19}, {'registration_pgid': 19},
                     {'registration_sid': 19}, {'registration_uid': -1}, {'registration_ppid': 0})
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                manager, bridge = self.manager(True)
                bridge.report.update(mutation)
                with self.assertRaises(candidate.ProtocolError):
                    manager.cleanup(bridge.child)
                self.assertEqual(bridge.calls, ['snapshot'])
                self.assertFalse(manager._receipts[bridge.child]['resolved'])
        manager, bridge = self.manager(True)
        with self.assertRaises(candidate.ProtocolError):
            manager.cleanup(17)
        self.assertEqual(bridge.calls, [])

    def test_uid_evidence_is_operation_specific_and_wait_uid_unavailable(self):
        manager, bridge = self.manager(True)
        receipt = manager._receipt(bridge.child, bridge.report)
        anchor = manager._registered_anchor(bridge.child, bridge.report, receipt)
        for state, evidence, copied in ((candidate.LIVE, candidate.UID_CURRENT, False),
                                        (candidate.EXITED, candidate.UID_RETAINED, False),
                                        (0, candidate.UID_RETAINED, True)):
            with self.subTest(state=state, copied=copied):
                record = dict(bridge.report['record'], state=state, uid_evidence=evidence)
                manager._identity(anchor, record, copied=copied)
                for mutation in ({'wait_uid_available': 1}, {'uid_evidence': 0},
                                 {'uid_evidence': 3 - evidence}, {'uid': 0}):
                    with self.subTest(mutation=mutation), self.assertRaises(candidate.ProtocolError):
                        manager._identity(anchor, dict(record, **mutation), copied=copied)
        member = dict(bridge.report['record'], pid=19, state=candidate.LIVE,
                      uid_evidence=candidate.UID_CURRENT)
        manager._identity(anchor, member, 19)

    def test_terminal_wait_uid_remains_diagnostic(self):
        for wait_uid in (0, 12, 99):
            with self.subTest(wait_uid=wait_uid):
                manager, bridge = self.manager(True)
                bridge.sink.terminal_overrides['wait_uid'] = wait_uid
                receipt = manager.cleanup(bridge.child)
                self.assertTrue(receipt['resolved'])
                self.assertEqual(receipt['terminal_report']['wait_uid'], wait_uid)
                self.assertEqual(receipt['terminal_report']['decoded_record']['uid'], 12)
                self.assertEqual(bridge.calls, ['snapshot', 'terminal', 'handoff'])

    def test_decoded_terminal_identity_mismatch_prevents_handoff_without_reap_retry(self):
        mutations = ({'pid': 19}, {'pgid': 19}, {'uid': 99}, {'ppid': 9}, {'sid': 19},
                     {'state': candidate.LIVE}, {'exit_kind': 2}, {'exit_status': 1},
                     {'wait_uid_available': 1}, {'uid_evidence': candidate.UID_CURRENT},
                     {'no_signals': 0}, {'reap_attempted': 0}, {'reaped': 0})
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                manager, bridge = self.manager(True)
                bridge.sink.decoded_overrides.update(mutation)
                with self.assertRaises(candidate.ProtocolError):
                    manager.cleanup(bridge.child)
                self.assertEqual(bridge.calls, ['snapshot', 'terminal'])
                self.assertFalse(manager._receipts[bridge.child]['resolved'])

    def test_unproven_consumer_facts_prevent_handoff_without_reap_retry(self):
        mutations = ({'reap_attempted': 0}, {'wait_entered': 0}, {'wait_returned': 0},
                     {'wait_rc': -1}, {'wait_pid': 0}, {'wait_pid': 19},
                     {'consumed': 0}, {'decoded': 0}, {'certificate_matches': 0})
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                manager, bridge = self.manager(True)
                bridge.sink.terminal_overrides.update(mutation)
                with self.assertRaises(candidate.ProtocolError):
                    manager.cleanup(bridge.child)
                self.assertEqual(bridge.calls, ['snapshot', 'terminal'])
                self.assertFalse(manager._receipts[bridge.child]['resolved'])


if __name__ == '__main__':
    unittest.main()
