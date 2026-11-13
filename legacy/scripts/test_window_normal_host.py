"""Guarded parent normal-host controls. Synthetic provenance; no native child.

Uses actual host/IO/protocol source with explicit Python heap mock Child/Sink
objects and a synthetic Admission/Attempt. This does NOT authenticate a native
Admission or exercise Worker B. Durability is simulated; no file/FIFO, process,
real clock, signal, extension or target operation is permitted. Parent-only run.
"""
import copy
import os
import sys
import types
import unittest
from unittest import mock

import test_window_suspended_host as guards
import test_play_alpha_owned_child as native_mock

guards._SOURCE_NAMES=guards._SOURCE_NAMES|{'window_normal_host.py'}
protocol=native_mock.protocol
io=guards.host_io
TOKEN='a'*32

def view(terminal=None,*,retired=False,fresh=True,observe=False):
    return dict(scope='private-normal-diagnostic',native_qualified=False,
                continue_authorized=False,healthy=False,fresh=fresh,
                retirement_proven=retired,completed=retired,private_diagnostic_completed=retired,
                status='completed' if retired else 'waiting',pending=None,
                intent_resolved=True,needs_observation=observe,protocol_faults=(),
                terminal=copy.deepcopy(terminal),reason='normal_complete' if retired else 'ready')

class Park(BaseException):pass
def park():raise Park('synthetic dormant retention')

class Journal:
    def __init__(self):self.events=[];self.failure=None
    def append(self,event):
        if self.failure==event['kind']:raise OSError('synthetic journal/fsync failure')
        io.encode(event)
        self.events.append(copy.deepcopy(event))

class Fifo:
    def __init__(self,chunks=()):self.chunks=list(chunks);self.reads=0
    def read(self):
        self.reads+=1
        if self.chunks:return self.chunks.pop(0)
        raise OSError('synthetic lost controller')

class Admission:
    """Synthetic policy input, NEVER private native admission evidence."""
    def __init__(self):
        self.binding=types.SimpleNamespace(Child=native_mock.MockChild,TerminalSink=native_mock.MockSink)
        self.bridge=native_mock.MockBridge();self.root=object();self.execution=None
        self.manifest=dict(root={'synthetic':True},fifo={'synthetic':True},token=TOKEN)
    def owns_execution(self,value):return value is not None and value is self.execution
    def _admit(self,session):
        assert session is self.execution
        return (b'/usr/bin/true',),b'/private/tmp/synthetic-normal-cwd'

class Attempt:
    def __init__(self,admission):
        self.admission=admission;self.creates=self.reads=self.cached_reads=0
        self.reply_loss=False;self.mutate=None
    def create_once(self):
        assert self.creates==0;self.creates+=1
        a=self.admission;b=a.bridge
        session=protocol.ChildExecution.for_injected_mock(a.binding,b,
          bridge_type=type(b),child_type=native_mock.MockChild,terminal_type=native_mock.MockSink)
        a.execution=session;session._claim=a
        session.admit();b.active_state=2;session.observe_active()
        for unused in range(6):session.finish_step()
        assert session.read_completion()==('exited',0,None)
        if self.reply_loss:raise RuntimeError('synthetic coordinator delivery loss after completion')
        return self.facts()
    def facts(self):
        result=view(self.admission.bridge.t,retired=True)
        if self.mutate:self.mutate(result)
        return result
    def read(self):self.reads+=1;return self.facts()
    def cached(self):
        self.cached_reads+=1;result=self.facts();result.update(fresh=False,retirement_proven=False,retirement=None)
        return result
    def step(self,observation=False):raise AssertionError('Already completed synthetic Attempt')

class Guarded(unittest.TestCase):
    def setUp(self):
        self.guard=guards.Guard(source_import=True);self.guard.__enter__()
        self.addCleanup(self.guard.__exit__)
        self.host=guards._actual_source('window_normal_host')
        self.guard.source_import=False
        self.patch(self.host,'_free',lambda root:None)
        self.patch(self.host,'_dormant',park)
    def patch(self,target,name,value):
        context=mock.patch.object(target,name,value);result=context.__enter__()
        self.addCleanup(context.__exit__,None,None,None);return result
    def make(self):
        a=Admission();journal=Journal();fifo=Fifo();delays=[]
        h=self.host._Host(io,protocol,types.SimpleNamespace(Attempt=Attempt),a,journal,fifo,TOKEN,delays.append)
        return h,a,journal,fifo,delays
    def run_complete(self):
        values=self.make();self.assertFalse(values[0].dispatch('RUN'))
        self.assertEqual(values[0].attempt.creates,1);return values

class ExitProof(Guarded):
    def test_no_attempt_exit_independently_requires_twice_empty_registry(self):
        h,a,j,unused,delays=self.make()
        self.assertTrue(h.dispatch('EXIT'));self.assertTrue(h.exit_ready)
        self.assertEqual(a.bridge.calls,['pending','history','pending','history'])
        self.assertEqual(j.events[-1]['kind'],'EXIT_READY');self.assertEqual(delays,[])

    def test_normal_exit_requires_new_read_bracketed_snapshots_and_durable_record(self):
        h,a,j,unused,delays=self.run_complete();before=h.attempt.reads
        calls=len(a.bridge.calls);self.assertTrue(h.dispatch('EXIT'))
        self.assertEqual(h.attempt.reads,before+1)
        self.assertEqual(a.bridge.calls[calls:],['pending','history','snapshot','snapshot','active_first_terminal','pending','history'])
        proof=j.events[-1]['proof'];self.assertEqual(len(proof['registry']),2)
        self.assertEqual(len(proof['snapshots']),2);self.assertTrue(h.exit_ready)
        self.assertEqual([e['kind'] for e in j.events],['RUN_INTENT','RUN_RESULT','EXIT_READY'])

    def test_cached_and_read_never_authorize_host_exit(self):
        h,a,j,unused,delays=self.run_complete()
        self.assertFalse(h.dispatch('CACHED'));self.assertFalse(h.dispatch('READ'))
        self.assertFalse(h.exit_ready);self.assertEqual(h.attempt.cached_reads,1)
        self.assertFalse(j.events[-2]['facts']['retirement_proven'])

    def test_duplicate_run_is_consumed_and_prevents_exit(self):
        h,a,j,unused,delays=self.run_complete()
        self.assertFalse(h.dispatch('RUN'));self.assertFalse(h.dispatch('EXIT'))
        self.assertEqual(h.attempt.creates,1);self.assertGreater(h.protocol_faults,0)

    def test_lost_create_delivery_recovers_original_without_replay(self):
        h,a,j,unused,delays=self.make();h.attempt.reply_loss=True
        self.assertFalse(h.dispatch('RUN'));self.assertEqual(h.attempt.creates,1)
        self.assertEqual(len(h.coordinator.errors),1);self.assertTrue(h.dispatch('EXIT'))

    def test_authority_flag_or_stale_view_cannot_authorize_exit(self):
        for change in ({'native_qualified':True},{'fresh':False},{'intent_resolved':False},
                       {'private_diagnostic_completed':False},{'protocol_faults':('failure',)}):
            h,a,j,unused,delays=self.run_complete()
            h.attempt.mutate=lambda result,change=change:result.update(change)
            self.assertFalse(h.dispatch('EXIT'));self.assertFalse(h.exit_ready)

    def test_independent_second_snapshot_mismatch_retains_original_roots(self):
        h,a,j,unused,delays=self.run_complete()
        a.bridge.actions['snapshot']=[lambda b:None,lambda b:b.t.update(wait_status=7)]
        self.assertFalse(h.dispatch('EXIT'));self.assertFalse(h.exit_ready)
        self.assertIs(a.execution._sink,a.bridge.sink);self.assertTrue(h.proofs)

    def test_registry_replacement_after_snapshots_prevents_exit(self):
        h,a,j,unused,delays=self.run_complete();original=a.execution._sink
        a.bridge.actions['history']=[lambda b:None,lambda b:setattr(b,'sink',native_mock.MockSink(b))]
        self.assertFalse(h.dispatch('EXIT'));self.assertIs(a.execution._sink,original)

    def test_uncertified_consuming_result_prevents_exit(self):
        h,a,j,unused,delays=self.run_complete();a.bridge.t['certificate_matches']=0
        self.assertFalse(h.dispatch('EXIT'));self.assertFalse(h.exit_ready)

    def test_first_active_exit_disagreement_prevents_exit(self):
        h,a,j,unused,delays=self.run_complete()
        a.bridge.first_active['record'].update(exit_status=7,returncode=7)
        self.assertFalse(h.dispatch('EXIT'));self.assertFalse(h.exit_ready)

    def test_historical_native_failure_or_forced_teardown_prevents_exit(self):
        for field in ('term_attempted','kill_attempted','signal_failed','blocked'):
            h,a,j,unused,delays=self.run_complete();a.bridge.r[field]=1
            self.assertFalse(h.dispatch('EXIT'));self.assertFalse(h.exit_ready)

    def test_untransferred_collection_cannot_authorize_exit(self):
        h,a,j,unused,delays=self.run_complete();a.bridge.t['transferred']=0
        self.assertFalse(h.dispatch('EXIT'));self.assertFalse(h.exit_ready)

    def test_lost_exit_ready_durability_does_not_release_host(self):
        h,a,j,unused,delays=self.run_complete();j.failure='EXIT_READY'
        self.assertFalse(h.dispatch('EXIT'));self.assertTrue(h.storage_failed)
        self.assertFalse(h.exit_ready);self.assertIsNotNone(h.first_error)
        self.assertIs(a.execution._sink,a.bridge.sink)

    def test_finite_proof_budget_reserved_before_failed_reads(self):
        h,a,j,unused,delays=self.run_complete()
        h.attempt.mutate=lambda result:result.update(fresh=False)
        for unused in range(6):self.assertFalse(h.dispatch('EXIT'))
        self.assertEqual(h.proof_reads,4);self.assertEqual(h.attempt.reads,4)

    def test_control_loss_exhaustion_retains_host_without_extra_actions(self):
        h,a,j,fifo,delays=self.make()
        with self.assertRaises(Park):h.serve()
        self.assertEqual(fifo.reads,64);self.assertEqual(len(delays),63)
        self.assertEqual(h.attempt.creates,0);self.assertFalse(h.exit_ready)

class Coordinator(Guarded):
    def scripted(self,sequence):
        class Script:
            def __init__(self):self.creates=self.steps=self.reads=0
            def create_once(self):self.creates+=1;return copy.deepcopy(sequence[0])
            def step(self,observation=False):
                self.steps+=1;return copy.deepcopy(sequence[min(self.steps,len(sequence)-1)])
            def read(self):self.reads+=1;return copy.deepcopy(sequence[-1])
            def cached(self):return view({},fresh=False)
        return Script()
    def test_live_followups_are_delayed_and_do_not_recreate(self):
        attempt=self.scripted([view({},observe=True),view({},observe=True),view({},retired=True)])
        c=self.host._Coordinator(attempt);delays=[];c.start(delays.append)
        self.assertEqual((attempt.creates,attempt.steps),(1,2));self.assertEqual(delays,[0.01,0.01])
    def test_immediate_completion_does_not_observe_or_cleanup_again(self):
        attempt=self.scripted([view({},retired=True)]);c=self.host._Coordinator(attempt)
        c.start(lambda value:self.fail('Unexpected delay'));self.assertEqual(attempt.steps,0)
    def test_observation_budget_is_sticky_across_advance(self):
        attempt=self.scripted([view({},observe=True)]);c=self.host._Coordinator(attempt)
        c.start(lambda value:None);c.advance(lambda value:None)
        self.assertEqual(attempt.steps,8);self.assertEqual(c.observation_grants,8)
    def test_total_operation_and_refresh_budgets_do_not_reset(self):
        attempt=self.scripted([view({})]);c=self.host._Coordinator(attempt)
        c.start(lambda value:None);c.advance(lambda value:None)
        self.assertEqual(c.operations,32);self.assertEqual(attempt.creates,1)
        for unused in range(4):c.refresh()
        with self.assertRaises(ValueError):c.refresh()
    def test_interrupted_delay_never_dispatches_observation(self):
        attempt=self.scripted([view({},observe=True)]);c=self.host._Coordinator(attempt)
        def interrupted(value):raise InterruptedError('synthetic delay')
        c.start(interrupted);self.assertEqual(attempt.steps,0);self.assertEqual(len(c.errors),1)
    def test_wrong_scope_never_dispatches_step(self):
        bad=view({});bad['scope']='suspended-diagnostic-only'
        attempt=self.scripted([bad]);c=self.host._Coordinator(attempt);c.start(lambda value:None)
        self.assertEqual(attempt.steps,0);self.assertEqual(attempt.creates,1)

class Bootstrap(Guarded):
    def test_reserved_pid_refused_before_any_path_or_native_access(self):
        self.patch(self.host.os,'getpid',lambda:47707)
        with self.assertRaisesRegex(ValueError,'Reserved'):self.host.main()

    def main_fixture(self,*,serve_error=None,report_error=None):
        host=self.host;family='/private/tmp/sims-normal-true-'+'b'*32
        h,a,j,fifo,delays=self.run_complete()
        self.patch(host,'__file__',family+'/window_normal_host.py')
        self.patch(host.os,'getpid',lambda:99);self.patch(host.os,'getuid',lambda:501)
        self.patch(host.sys,'argv',[family+'/window_normal_host.py'])
        self.patch(host.sys,'flags',types.SimpleNamespace(isolated=1,no_site=1,no_user_site=1,dont_write_bytecode=1))
        self.patch(host,'_environment',lambda environ:'c'*64)
        manifest=a.manifest;manifest['root']={'synthetic':True}
        a.root=types.SimpleNamespace(admitted=manifest['root']);h.root=a.root
        self.patch(host,'_bootstrap_sources',lambda *args:(manifest,{n+'.py':b'guarded fixture' for n in host.MODULES}))
        loaded={'window_normal_admission':types.SimpleNamespace(prepare_private_true=lambda *args:a),
                'window_suspended_host_io':types.SimpleNamespace(Journal=lambda root:j,Fifo=lambda *args:fifo),
                'window_owned_child_bridge':protocol,'window_normal_diagnostic':object(),
                'window_suspended_host':object()}
        self.patch(host,'_source',lambda name,*args:loaded[name]);self.patch(host,'_policies',lambda:None)
        self.patch(host,'_Host',lambda *args:h)
        if serve_error:
            def fail():raise serve_error
            self.patch(h,'serve',fail)
        if report_error:
            def fail_report(*args):raise report_error
            self.patch(h,'transcript',fail_report)
        context=mock.patch.dict(sys.modules);context.__enter__();self.addCleanup(context.__exit__,None,None,None)
        sys.modules.pop('site',None)
        for name in host.MODULES+('_window_owned_child_bridge',):sys.modules.pop(name,None)
        return h
    def test_reporting_memory_error_after_creation_still_enters_retained_state(self):
        self.main_fixture(serve_error=RuntimeError('synthetic serve loss'),report_error=MemoryError('synthetic reporting loss'))
        with self.assertRaises(Park):self.host.main()

if __name__=='__main__':unittest.main()
