"""Actual-source Admission/consumer/host interoperability under explicit mocks.

The real Admission record/factory code is exercised, but preparation is NOT:
tests inject the record and replace _revalidate with a no-IO fixture. Bridge,
Child, Sink, clock, journal and root are Python mocks. No actual Admission,
native image/type/ownership/ABI/OS/durability or host-runtime qualification.
No for_injected_mock promotion occurs in the private-factory forwarding cases.
"""
import hashlib
import importlib.machinery
import importlib.util
import json
import os
import pathlib
import resource
import signal
import stat
import sys
import time
import types
import unittest
from unittest import mock

import test_window_normal_host as host_fixtures
import test_window_normal_diagnostic as consumer_fixtures
import window_normal_admission as admission
import window_normal_diagnostic as consumer
import window_owned_child_bridge as protocol

with host_fixtures.guards.Guard(source_import=True):
    host=host_fixtures.guards._actual_source('window_normal_host')

class Interoperability(consumer_fixtures.GuardedCase):
    def patch(self,target,name,value):
        context=mock.patch.object(target,name,value);result=context.__enter__()
        self.addCleanup(context.__exit__,None,None,None);return result

    def launch_facts(self,bridge):
        # Source C LC_* kinds. Positive raw values are successful identity,
        # descriptor and byte-count returns; raw errno remains diagnostic.
        bridge.r['launch_facts']=((1,1,1,0,5),(2,1,1,7,5),(3,1,1,12,5),
            (4,1,1,31,5),(14,1,1,136,5),(15,1,1,17,5),(11,1,1,0,5),(16,1,1,0,5))
        bridge.r['launch_fact_count']=len(bridge.r['launch_facts'])

    def private(self,*,attempt=True,ledger=False):
        a=object.__new__(admission.Admission)
        record=admission._Record(a);admission._ADMISSIONS.append(record)
        bridge=consumer_fixtures.fixtures.MockBridge();bridge.active_state=2
        record.ready=True;record.bridge=bridge;record.root=object()
        record.manifest={'provenance':'explicit injected record; not native authentication'}
        record.child_type=consumer_fixtures.fixtures.MockChild
        record.terminal_type=consumer_fixtures.fixtures.MockSink
        record.binding=types.SimpleNamespace(Bridge=type(bridge),Child=record.child_type,TerminalSink=record.terminal_type)
        record.cwd=b'/synthetic-private-factory-cwd'
        record.options=((b'PATH=/usr/bin:/bin',b'LC_ALL=C'),(9,9,9),10_000_000_000,10_000_000_000,100_000_000,128)
        self.patch(admission,'_revalidate',lambda r:None)  # EXPLICIT preparation/authentication bypass in tests only.
        if ledger:self.launch_facts(bridge)
        return a,record,bridge,consumer.Attempt(a) if attempt else None

    def drive(self,attempt):
        result=attempt.create_once()
        self.assertEqual(result['status'],'waiting',result['protocol_faults'])
        result=attempt.step(observation=True)
        self.assertIsNotNone(result['first_active'],result['protocol_faults'])
        for unused in range(6):result=attempt.step()
        self.assertTrue(result['completed'],result['protocol_faults'])
        self.assertFalse(result['fresh']);self.assertFalse(result['retirement_proven'])
        return result

    def test_real_admission_detached_manifest_copy_contract(self):
        a,record,bridge,attempt=self.private()
        self.assertEqual(a.manifest,a.manifest);self.assertIsNot(a.manifest,a.manifest)
        with mock.patch.object(protocol.ChildExecution,'for_injected_mock',side_effect=AssertionError('Private path promoted a mock')):
            result=attempt.create_once()
        self.assertEqual(result['status'],'waiting',result['protocol_faults'])
        self.assertEqual(bridge.calls.count('create'),1)
        self.assertIs(a.execution,attempt._session);self.assertIs(a.execution._claim,a)
        self.assertFalse(a.native_qualified);self.assertFalse(a.execution.native_qualified)

    def test_manifest_content_change_retains_original_execution(self):
        a,record,bridge,attempt=self.private()
        self.assertEqual(attempt.create_once()['status'],'waiting')
        record.manifest['changed']=True
        self.assertEqual(attempt.read()['status'],'retained')
        self.assertIs(a.execution,attempt._session)
        self.assertNotIn('cleanup_begin',bridge.calls)

    def test_positive_launch_return_values_are_not_errors(self):
        attempt,bridge,session=self.make();self.launch_facts(bridge)
        result=self.drive(attempt)
        self.assertEqual(result['terminal']['final_report']['launch_facts'],bridge.r['launch_facts'])
        self.assertTrue(attempt.read()['retirement_proven'])
        self.assertEqual(bridge.calls.count('child_reap'),1)
        self.assertNotIn('child_signal',bridge.calls)

    def test_inconsistent_launch_return_values_are_retained(self):
        for kind,value in ((2,8),(3,13),(4,2),(14,0),(15,18),(11,1),(20,0)):
            attempt,bridge,session=self.make()
            bridge.r['launch_facts']=((kind,1,1,value,0),);bridge.r['launch_fact_count']=1
            result=attempt.create_once()
            self.assertEqual(result['status'],'retained',(kind,value,result['protocol_faults']))
            self.assertNotIn('cleanup_begin',bridge.calls)
            self.assertIs(session,attempt._session)

    def test_private_factory_consumer_uses_only_final_explicit_read(self):
        a,record,bridge,attempt=self.private(ledger=True)
        with mock.patch.object(protocol.ChildExecution,'for_injected_mock',side_effect=AssertionError('Private path promoted a mock')):
            self.drive(attempt)
        self.assertEqual(attempt.cached()['recovery_reads'],0)
        fresh=attempt.read();self.assertTrue(fresh['retirement_proven'])
        self.assertEqual(fresh['scope'],'private-normal-diagnostic')
        self.assertEqual(fresh['recovery_reads'],1)
        self.assertEqual(bridge.calls.count('create'),1)

    def host_case(self,*,lost_handoff=False):
        a,record,bridge,unused=self.private(attempt=False,ledger=True)
        if lost_handoff:
            original_ready=admission._execution_ready
            def ready_and_inject(original_admission,session):
                original_ready(original_admission,session)
                original_finish=session.finish_step
                lost=[False]
                def lose_return_after_handoff():
                    reply=original_finish()
                    if bridge.t['transferred'] and not lost[0]:
                        lost[0]=True
                        raise MemoryError('synthetic delivery loss AFTER original finish_step returned')
                    return reply
                session.finish_step=lose_return_after_handoff
            self.patch(admission,'_execution_ready',ready_and_inject)
        journal=host_fixtures.Journal();fifo=host_fixtures.Fifo()
        self.patch(host,'_free',lambda root:None)
        def pause(seconds):self.clock.now+=round(seconds*1_000_000_000)
        h=host._Host(host_fixtures.io,protocol,consumer,a,journal,fifo,host_fixtures.TOKEN,pause)
        return h,a,bridge,journal

    def test_host_progress_reserves_fresh_read_for_independent_exit(self):
        h,a,bridge,journal=self.host_case()
        self.assertFalse(h.dispatch('RUN'))
        self.assertTrue(h.coordinator.facts['completed'],h.coordinator.errors)
        self.assertFalse(h.coordinator.facts['fresh'])
        self.assertEqual(h.attempt.cached()['recovery_reads'],0)
        self.assertFalse(h.exit_ready)
        self.assertTrue(h.dispatch('EXIT'),h.errors)
        self.assertEqual(h.attempt.cached()['recovery_reads'],1)
        self.assertEqual(journal.events[-1]['kind'],'EXIT_READY')
        self.assertEqual(len(journal.events[-1]['proof']['snapshots']),2)
        self.assertEqual(len(journal.events[-1]['proof']['registry']),2)

    def test_host_recovers_handoff_without_replay_then_checks_deadline(self):
        h,a,bridge,journal=self.host_case(lost_handoff=True)
        self.assertFalse(h.dispatch('RUN'))
        self.assertTrue(h.coordinator.facts['completed'],h.coordinator.errors)
        self.assertEqual(h.attempt.cached()['recovery_reads'],1)
        self.assertEqual(bridge.calls.count('handoff'),1)
        self.assertEqual(bridge.calls.count('child_reap'),1)
        self.assertTrue(h.dispatch('EXIT'),h.errors)
        self.assertEqual(h.attempt.cached()['recovery_reads'],2)
        self.assertEqual(bridge.calls.count('handoff'),1)

    def test_host_refuses_injected_mock_completion_scope(self):
        attempt,bridge,session=self.make();self.drive(attempt)
        facts=attempt.read();self.assertTrue(facts['retirement_proven'])
        self.assertEqual(facts['scope'],'injected-mock')
        with self.assertRaises(ValueError):host._check_view(facts)

if __name__=='__main__':unittest.main()
