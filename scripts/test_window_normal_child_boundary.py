"""Actual-source C/PyC normal lifecycle controls using intercepted OS calls.

No native child is created, resumed, queried, signalled or waited for. The real
producer makes its own memory roots and certificates from mocked OS returns;
this fixture never seeds them. Three exact temporary gate edits enable mocks;
production source/public factories/legacy gate remain disabled. Parent-run only.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parent.parent
NATIVE=ROOT/'crates/sims-app/native'
CLANG='/Library/Developer/CommandLineTools/usr/bin/clang'
PYTHON='/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/bin/python3.9'
INCLUDE='/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/include/python3.9'
SDK='/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk'
ENV=dict(PATH='/usr/bin:/bin',LC_ALL='C',TMPDIR='/private/tmp',SDKROOT=SDK,
         DEVELOPER_DIR='/Library/Developer/CommandLineTools')
PINS={
 'window_owned_child_darwin.c':'20967abd1ac970e908fbb0facd31f7c250a168865f4328caa74042a6010646ee',
 'window_owned_child_darwin.h':'b2294ecdba9a395f8a11d5a775bd440d4df4258452e6e10fcb7b25ee132be9b3',
 'window_owned_child_python.c':'bed6393d66860cbc1ee56c66925e564ad1b8d8633d265c453458d4ddf6e6cb1a'}
TOOLS={CLANG:'f30550eab15fdf5ab8c0dc54c52679711241e5d4b636b027e18c09fef531775d',
       PYTHON:'bdea59019a38eb6600cc9e71e984a97fedadc406448431281e7657030f54987e'}
CASES=('immediate_exit','live_transition_exit','positive_errno','memory_reads',
       'active_mapping_loss','reap_mapping_loss','handoff_delivery_loss',
       'foreign_consuming_pid','changed_consuming_status','consuming_failure',
       'late_handoff','repeat_reap','resume_failure','latch_extra_member')

CHILD_SCRIPT=r'''
import importlib.util
import sys
image,directory,case=sys.argv[1:]
spec=importlib.util.spec_from_file_location('_window_owned_child_bridge',image)
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
sys.path.insert(0,directory)
import window_owned_child_bridge as protocol
bridge=module.new_bridge()
args=(b'/usr/bin/true',(b'/usr/bin/true',),(b'PATH=/usr/bin:/bin',b'LC_ALL=C'),
      b'/private/tmp/mock-normal-child-cwd',(9,9,9),10000000000,10000000000,100000000,128)

def raises(kind,function,*args):
    try:function(*args)
    except kind:return
    raise AssertionError('Expected '+kind.__name__)

assert bridge.pending() is None and bridge.history()==()
assert not any(protocol.QUALIFICATIONS.values())
assert protocol._native_execution_authorized(None) is False
raises(protocol.Unsupported,protocol.production_child_execution)
if case=='production_gate':
    assert module.CHILD_NATIVE_QUALIFIED==0
    before=module._fixture_stats()
    raises(RuntimeError,bridge.create,*args)
    assert module._fixture_stats()==before
    assert all(value==0 for key,value in before.items() if key!='raw_bytes')
    assert bridge.pending() is None and bridge.history()==()
    print('PASS '+case);raise SystemExit(0)

assert module.CHILD_NATIVE_QUALIFIED==1
if case=='positive_errno':module._fixture_scenario('positive_errno')
if case=='resume_failure':module._fixture_scenario('resume_error')
if case=='resume_failure':
    raises(RuntimeError,bridge.create,*args)
    child=bridge.pending();assert type(child) is module.Child
else:child=bridge.create(*args)
sink=bridge.terminal(child)
assert bridge.pending() is child and bridge.history()==(sink,)
report=bridge.child_snapshot(child);generation=report['generation']
assert generation==1 and report['registered']==report['pid_valid']==1
assert report['resources_closed']==1 and report['resource_error']==0
assert report['registration_observation']['code']>0
assert report['registration_observation']['uid']==0 and report['registration_uid']==501
assert module._fixture_stats()['spawns']==module._fixture_stats()['resumes']==1
assert module._fixture_stats()['allocations']==2 and module._fixture_stats()['closes']==3

def active():
    result=bridge.active_observe(child)
    assert protocol._active_report(result,generation,fresh=True,first=False) is result
    assert result['collection_evidence'] is False
    return result

def through_latch():
    assert bridge.cleanup_begin(child)['cleanup_entries']==1
    assert bridge.child_observe(child)['record']['state']==2
    assert bridge.child_members(child)['record']['count']==1
    result=bridge.child_latch(child,1,0)
    assert result['rc']==0 and result['latch_attempted']==1
    assert result['record']['no_signals']==1

def final_proof():
    assert bridge.pending() is None and bridge.history()==(sink,)
    one=sink.snapshot();first=sink.active_first_terminal();two=sink.snapshot()
    assert one==two and one['transferred']==1 and one['kind']==2
    assert one['consumed']==one['decoded']==one['certificate_matches']==1
    assert one['wait_pid']==first['record']['pid']==41
    assert one['wait_status']==first['record']['exit_status']==0
    assert one['wait_uid']==0 and one['decoded_record']['uid']==501
    assert one['final_report']['state']==11 and one['final_report']['cleanup_entries']==6
    assert one['final_report']['term_attempted']==one['final_report']['kill_attempted']==0
    assert one['final_report']['abort']['started']==0
    assert protocol._classify_child_terminal(one,generation)==('exited',0,None)
    stats=module._fixture_stats();assert stats['consuming']==stats['frees']==1
    raises(RuntimeError,bridge.child_snapshot,child)
    assert module._fixture_stats()==stats
    return one

if case=='resume_failure':
    assert report['resume_entered']==report['resume_returned']==1 and report['resume_rc']!=0
    assert report['published']==0 and report['blocked']==1
    raises(RuntimeError,bridge.active_observe,child)
    assert module._fixture_stats()['consuming']==module._fixture_stats()['frees']==0
    assert bridge.pending() is child and sink.snapshot()['transferred']==0
elif case=='live_transition_exit':
    for scenario,state in (('live',1),('transition',3),('exit0',2)):
        module._fixture_scenario(scenario)
        assert active()['record']['state']==state
        assert not bridge.child_snapshot(child)['cleanup_started']
        assert module._fixture_stats()['consuming']==0
    through_latch();assert bridge.child_reap(child)['rc']==0
    assert bridge.handoff(child,sink) is sink;final_proof()
elif case=='active_mapping_loss':
    module._fixture_scenario('dictionary_loss')
    raises(MemoryError,bridge.active_observe,child)
    stats=module._fixture_stats();cached=sink.active_observation()
    assert cached['sequence']==1 and cached['record']['state']==2
    assert cached['fresh_native_call'] is False and module._fixture_stats()==stats
    assert sink.active_first_terminal()['sequence']==1
    through_latch();assert bridge.child_reap(child)['rc']==0
    assert bridge.handoff(child,sink) is sink;final_proof()
elif case in ('immediate_exit','positive_errno','memory_reads','handoff_delivery_loss'):
    observed=active();assert observed['record']['state']==2
    if case=='memory_reads':
        stats=module._fixture_stats()
        for unused in range(4):
            bridge.pending();bridge.history();bridge.child_snapshot(child)
            sink.snapshot();sink.active_observation();sink.active_first_terminal()
        assert module._fixture_stats()==stats
    through_latch();assert bridge.child_reap(child)['rc']==0
    if case=='handoff_delivery_loss':
        def lost_delivery():
            assert bridge.handoff(child,sink) is sink
            raise RuntimeError('synthetic coordinator delivery loss after return')
        raises(RuntimeError,lost_delivery)
    else:assert bridge.handoff(child,sink) is sink
    terminal=final_proof()
    if case=='positive_errno':
        assert observed['wait_errno']>0 and terminal['wait_errno']>0
        assert terminal['final_report']['spawn_errno']>0
        assert terminal['final_report']['resume_errno']>0
        assert terminal['final_report']['clock_errno']>0
elif case=='reap_mapping_loss':
    active();through_latch();module._fixture_scenario('dictionary_loss')
    raises(MemoryError,bridge.child_reap,child)
    stats=module._fixture_stats();terminal=sink.snapshot()
    assert terminal['consumed']==terminal['certificate_matches']==1
    assert module._fixture_stats()==stats and stats['consuming']==1
    assert bridge.handoff(child,sink) is sink;final_proof()
elif case in ('foreign_consuming_pid','changed_consuming_status','consuming_failure'):
    active();through_latch()
    scenario={'foreign_consuming_pid':'consume_foreign_pid',
              'changed_consuming_status':'consume_changed_status',
              'consuming_failure':'consume_error'}[case]
    module._fixture_scenario(scenario)
    assert bridge.child_reap(child)['rc']!=0
    terminal=sink.snapshot();assert terminal['certificate_matches']==0 and terminal['transferred']==0
    before=module._fixture_stats();assert bridge.child_reap(child)['rc']!=0
    assert module._fixture_stats()==before and before['consuming']==1
    raises(RuntimeError,bridge.handoff,child,sink)
    assert bridge.pending() is child
elif case=='late_handoff':
    active();through_latch();assert bridge.child_reap(child)['rc']==0
    before=sink.snapshot();deadline=before['final_report']['cleanup_deadline_ns']
    module._fixture_scenario('expire');raises(RuntimeError,bridge.handoff,child,sink)
    after=sink.snapshot();assert after['consumed']==1 and after['transferred']==0
    assert after['final_report']['cleanup_deadline_ns']==deadline
    assert bridge.pending() is child and module._fixture_stats()['consuming']==1
    assert module._fixture_stats()['frees']==0
elif case=='repeat_reap':
    active();through_latch();assert bridge.child_reap(child)['rc']==0
    before=module._fixture_stats();report=bridge.child_reap(child)
    assert report['rc']!=0 and module._fixture_stats()['consuming']==1
    assert module._fixture_stats()['waits']==before['waits']
    assert bridge.pending() is child and sink.snapshot()['transferred']==0
elif case=='latch_extra_member':
    active();assert bridge.cleanup_begin(child)['rc']==0
    assert bridge.child_observe(child)['rc']==0
    assert bridge.child_members(child)['rc']==0
    module._fixture_scenario('extra_member')
    assert bridge.child_latch(child,1,0)['rc']!=0
    assert module._fixture_stats()['consuming']==0 and sink.snapshot()['transferred']==0
    assert bridge.pending() is child
else:raise AssertionError('Unknown finite case')
assert protocol._native_execution_authorized(None) is False
assert not any(protocol.QUALIFICATIONS.values())
print('PASS '+case)
'''

PROFILE_CASES=('profile_immediate_exit', 'profile_positive_errno', 'profile_active_mapping_loss', 'profile_reap_mapping_loss', 'profile_handoff_delivery_loss', 'profile_consuming_disagreement', 'profile_resume_failure')
PROFILE_CHILD_SCRIPT="\nimport importlib.util\nimport json\nimport sys\nimage,directory,case=sys.argv[1:]\nspec=importlib.util.spec_from_file_location('_window_owned_child_bridge',image)\nmodule=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)\nsys.path.insert(0,directory)\nimport window_owned_child_bridge as protocol\nimport window_normal_admission as admission\nimport window_normal_diagnostic as consumer\nimport window_normal_host as host\nimport window_suspended_host_io as io\n\nassert module.CHILD_NATIVE_QUALIFIED==1\nassert all(not kind.__flags__ & (1<<9) for kind in (module.Bridge,module.Child,module.TerminalSink))\nbridge=module.new_bridge()\nassert type(bridge) is module.Bridge and module.new_bridge() is bridge\nassert bridge.pending() is None and bridge.history()==()\nassert not any(protocol.QUALIFICATIONS.values()) and protocol._native_execution_authorized(None) is False\n\n# This is NOT genuine Admission preparation/authentication. Actual native\n# producer roots/certificates are unseeded; only the Python record is synthetic.\na=object.__new__(admission.Admission)\nr=admission._Record(a);admission._ADMISSIONS.append(r)\nr.ready=True;r.binding=module;r.bridge=bridge;r.root=object()\nr.manifest={'scope':'explicit synthetic Admission record; native producer has intercepted OS calls'}\nr.child_type=module.Child;r.terminal_type=module.TerminalSink\nr.cwd=b'/private/tmp/mock-normal-child-cwd'\nr.options=((b'PATH=/usr/bin:/bin',b'LC_ALL=C'),(9,9,9),10_000_000_000,10_000_000_000,100_000_000,128)\nadmission._revalidate=lambda record:None  # Explicit source-test bypass, never preparation evidence.\ndef no_mock_promotion(*args,**kwargs):raise AssertionError('Private factory promoted injected mock')\nprotocol.ChildExecution.for_injected_mock=classmethod(no_mock_promotion)\nhost._free=lambda root:None  # Explicit no-filesystem root/journal/FIFO fixture.\n\nclass Journal:\n    def __init__(self):self.events=[]\n    def append(self,event):\n        raw=io.encode(event)\n        self.events.append(json.loads(raw))\n        return dict(bytes=len(raw))  # Memory fixture; no durable acknowledgement claim.\nclass Fifo:pass\njournal=Journal()\nif case=='profile_positive_errno':module._fixture_scenario('positive_errno')\nif case=='profile_active_mapping_loss':module._fixture_scenario('loss_on_next_active')\nif case=='profile_reap_mapping_loss':module._fixture_scenario('loss_on_next_reap')\nif case=='profile_consuming_disagreement':module._fixture_scenario('consume_changed_status')\nif case=='profile_resume_failure':module._fixture_scenario('resume_error')\nif case=='profile_handoff_delivery_loss':\n    ready=admission._execution_ready\n    lost=[False]\n    def ready_and_lose_return(original_admission,session):\n        ready(original_admission,session)\n        finish=session.finish_step\n        def wrapped():\n            reply=finish()\n            if session._sink.snapshot()['transferred'] and not lost[0]:\n                lost[0]=True\n                raise MemoryError('synthetic delivery loss AFTER original finish_step returned')\n            return reply\n        session.finish_step=wrapped\n    admission._execution_ready=ready_and_lose_return\n\nh=host._Host(io,protocol,consumer,a,journal,Fifo(),'b'*32,lambda delay:None)\nassert not h.dispatch('RUN')\nassert a.execution is h.attempt._session and a.owns_execution(a.execution)\nassert a.execution._binding is module and a.execution._bridge is bridge and a.execution._claim is a\nassert not a.native_qualified and not a.execution.native_qualified\nstats=module._fixture_stats()\nassert stats['spawns']==stats['resumes']==1 and stats['allocations']==2\nassert stats['duplicates']==stats['closes']==3\n\nif case in ('profile_consuming_disagreement','profile_resume_failure'):\n    assert not h.exit_ready\n    assert type(bridge.pending()) is module.Child and len(bridge.history())==1\n    retained=bridge.history()[0].snapshot()\n    assert retained['transferred']==0 and stats['frees']==0\n    assert stats['consuming']==(0 if case=='profile_resume_failure' else 1)\n    if case=='profile_resume_failure':\n        assert retained['final_report']['published']==0 and retained['final_report']['resume_rc']!=0\n        assert bridge.history()[0].active_first_terminal()['entered']==0\n    else:\n        assert retained['consumed']==1 and retained['certificate_matches']==0\n    assert not h.dispatch('EXIT') and not h.exit_ready\n    assert module._fixture_stats()==stats,'Exit refusal dispatched further native work'\n    assert not h.dispatch('RUN')\n    assert module._fixture_stats()==stats,'Duplicate RUN dispatched native work'\nelse:\n    facts=h.coordinator.facts\n    assert facts is not None and facts['completed'],(h.coordinator.status,h.coordinator.errors)\n    assert facts['scope']=='private-normal-diagnostic' and not facts['fresh'] and not facts['retirement_proven']\n    assert facts['active_dispatches']==1 and facts['first_active']['sequence']==1\n    assert stats['consuming']==stats['frees']==1\n    mapping=case in ('profile_active_mapping_loss','profile_reap_mapping_loss')\n    assert stats['mapping_failures']==int(mapping)\n    reads=h.attempt.cached()['recovery_reads']\n    assert reads==(1 if case in ('profile_handoff_delivery_loss','profile_active_mapping_loss') else 0),(case,reads)\n    assert h.dispatch('EXIT') and h.exit_ready,h.errors\n    assert h.attempt.cached()['recovery_reads']==reads+1\n    assert module._fixture_stats()==stats,'Final independent proof dispatched OS calls'\n    terminal=bridge.history()[0].snapshot()\n    assert terminal['transferred']==1 and terminal['kind']==2 and terminal['generation']==1\n    assert terminal['consumed']==terminal['decoded']==terminal['certificate_matches']==1\n    assert terminal['wait_pid']==41 and terminal['wait_uid']==0\n    final=terminal['final_report']\n    assert final['state']==11 and final['cleanup_entries']==6\n    assert final['term_attempted']==final['kill_attempted']==final['abort']['started']==0\n    assert final['resources_closed']==1 and final['resource_error']==0\n    assert protocol._classify_child_terminal(terminal,1)==('exited',0,None)\n    proof=journal.events[-1]['proof']\n    assert journal.events[-1]['kind']=='EXIT_READY'\n    assert len(proof['snapshots'])==len(proof['registry'])==2\n    assert proof['snapshots'][0]==proof['snapshots'][1]\n    assert proof['first_active']['record']['exit_status']==0\n    if case=='profile_positive_errno':\n        assert terminal['wait_errno']>0 and final['spawn_errno']>0 and final['resume_errno']>0\n        assert facts['first_active']['wait_errno']>0\nassert not any(protocol.QUALIFICATIONS.values()) and protocol._native_execution_authorized(a.execution) is False\nprint('PASS '+case)\n"

def digest(raw):return hashlib.sha256(raw).hexdigest()
def admitted(path):
    s=path.lstat()
    if not stat.S_ISREG(s.st_mode) or s.st_nlink!=1 or path.is_symlink():
        raise AssertionError('Regular single-link source required: '+str(path))
    return path.read_bytes()

class NormalBoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inputs={NATIVE/name:admitted(NATIVE/name) for name in PINS}
        for name,pin in PINS.items():
            if digest(cls.inputs[NATIVE/name])!=pin:raise AssertionError('Native source pin changed: '+name)
        for path,pin in TOOLS.items():
            if digest(Path(path).read_bytes())!=pin:raise AssertionError('Tool pin changed: '+path)
        fixture=ROOT/'scripts/fixtures/window_normal_child_boundary.c'
        consumer=ROOT/'scripts/window_owned_child_bridge.py'
        checker=ROOT/'scripts/test_window_active_observation.py'
        support=[ROOT/'scripts'/name for name in ('window_normal_admission.py','window_normal_diagnostic.py','window_normal_host.py','window_suspended_host.py','window_suspended_host_io.py')]
        cls.inputs.update({p:admitted(p) for p in (fixture,consumer,checker,*support)})
        if digest(cls.inputs[checker])!='1ca28fcc47b9cf300c042f27c535708c31f82aeee2e97437187a38499c6f3b34':
            raise AssertionError('Import checker pin changed')
        spec=importlib.util.spec_from_file_location('_normal_import_checker',checker)
        helper=importlib.util.module_from_spec(spec);spec.loader.exec_module(helper)
        cls.directory=Path(tempfile.mkdtemp(prefix='normal-cpyc-mock-',dir='/private/tmp'))
        cls.images={}
        for enabled in (0,1):
            directory=cls.directory/str(enabled);directory.mkdir()
            originals={name:cls.inputs[NATIVE/name] for name in PINS}
            replacements=(
             ('window_owned_child_darwin.h',b'    WOC_CHILD_NATIVE_QUALIFIED = 0,',b'    WOC_CHILD_NATIVE_QUALIFIED = 1,'),
             ('window_owned_child_darwin.c',b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 0, "Child native gate must stay disabled");',b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 1, "Child native gate must stay disabled");'),
             ('window_owned_child_python.c',b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 0, "Child qualification stays disabled");',b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 1, "Child qualification stays disabled");'))
            edits=[]
            if enabled:
                for name,before,after in replacements:
                    if originals[name].count(before)!=1 or after in originals[name]:raise AssertionError('Gate edit admission')
                    originals[name]=originals[name].replace(before,after);edits.append(name)
            assert len(edits)==3*enabled
            assert originals['window_owned_child_darwin.h'].count(b'    WOC_NATIVE_QUALIFIED = 0,')==1
            for name,raw in originals.items():(directory/name).write_bytes(raw)
            (directory/'fixture.c').write_bytes(cls.inputs[fixture])
            (directory/'window_owned_child_bridge.py').write_bytes(cls.inputs[consumer])
            for source in support:(directory/source.name).write_bytes(cls.inputs[source])
            image=directory/'_window_owned_child_bridge.so'
            command=[CLANG,'-std=c11','-O0','-g0','-Wall','-Wextra',
                     '-Werror=implicit-function-declaration','-bundle','-undefined','dynamic_lookup',
                     '-isysroot',SDK,'-I',INCLUDE,'-I',str(directory),str(directory/'fixture.c'),'-o',str(image)]
            result=subprocess.run(command,env=ENV,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=45,check=False)
            (directory/'compile.stdout').write_bytes(result.stdout);(directory/'compile.stderr').write_bytes(result.stderr)
            if result.returncode:raise AssertionError('Mock compile failed: '+str(directory)+'\n'+result.stderr.decode(errors='replace'))
            imports=helper.forbidden_imports(image)
            if imports:raise AssertionError('Unintercepted native imports: '+repr(imports))
            receipt=dict(enabled=enabled,source_edits=edits,image_sha256=digest(image.read_bytes()),
                         inputs={str(p):digest(raw) for p,raw in cls.inputs.items()},forbidden_imports=imports)
            (directory/'admission.json').write_text(json.dumps(receipt,indent=2)+'\n')
            cls.images[enabled]=image
        print('NORMAL MOCK retained evidence: '+str(cls.directory),flush=True)

    @classmethod
    def tearDownClass(cls):
        for path,raw in cls.inputs.items():
            if admitted(path)!=raw:raise AssertionError('Input changed during controls: '+str(path))

    def run_case(self,case,enabled=1):
        image=self.images[enabled]
        result=subprocess.run([PYTHON,'-I','-B','-S','-c',PROFILE_CHILD_SCRIPT if case in PROFILE_CASES else CHILD_SCRIPT,str(image),str(image.parent),case],
                              env=ENV,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=10,check=False)
        (image.parent/(case+'.stdout')).write_bytes(result.stdout)
        (image.parent/(case+'.stderr')).write_bytes(result.stderr)
        self.assertEqual(result.returncode,0,result.stderr.decode(errors='replace'))
        self.assertEqual(result.stdout,('PASS '+case+'\n').encode())
        self.assertEqual(result.stderr,b'')

    def test_production_gate(self):self.run_case('production_gate',0)

def install(case):
    def test(self):self.run_case(case)
    test.__name__='test_'+case
    setattr(NormalBoundaryTests,test.__name__,test)
for case in CASES+PROFILE_CASES:install(case)
if __name__=='__main__':unittest.main()
