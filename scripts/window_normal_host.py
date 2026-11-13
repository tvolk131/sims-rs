"""Private normal-true retained coordinator and independent exit boundary.

Parent implementation in progress; not packaged or runtime-qualified. Injection
into these private policy classes is a source-test seam, never native admission.
The eventual entry must use the original sealed Admission/Attempt/IO/schema.
No production, Continue, healthy or playable-alpha authority is provided.
"""
import hashlib
import json
import os
import stat
import sys
import types

PROFILE='NORMAL_TRUE_ONLY_PRIVATE_DIAGNOSTIC_NO_PRODUCTION'
MAX_OPERATIONS=32
MAX_OBSERVATIONS=8
MAX_REFRESHES=4
MAX_PROOFS=4
MAX_ERRORS=256
MAX_FAILURES=64
DELAY=0.01
_LIFETIME=[]
_SIGNALS=0
_STARTED=False
PYTHON='/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/bin/python3.9'
PYTHON_SHA256='bdea59019a38eb6600cc9e71e984a97fedadc406448431281e7657030f54987e'
FILES=('window_normal_host.py','window_normal_admission.py','window_normal_diagnostic.py',
       'window_owned_child_bridge.py','window_suspended_host.py','window_suspended_host_io.py',
       '_window_owned_child_bridge.so')
MODULES=('window_suspended_host','window_suspended_host_io','window_owned_child_bridge',
         'window_normal_admission','window_normal_diagnostic')

def _require(condition,message):
    if not condition:raise ValueError(message)

def _free(root):
    root.validate()
    s=os.fstatvfs(root.fd)
    _require(s.f_bavail*s.f_frsize>=6*1024**3,'At least6GiB free required')

def _dormant():
    import signal
    import time
    while True:
        try:signal.pause()
        except BaseException:
            try:time.sleep(60)
            except BaseException:pass

def _check_view(facts):
    _require(type(facts) is dict and facts.get('scope')=='private-normal-diagnostic',
             'Exact private normal diagnostic scope required')
    _require(all(facts.get(name) is False for name in
                 ('native_qualified','continue_authorized','healthy')),
             'Diagnostic cannot confer execution authority')
    _require(all(type(facts.get(name)) is bool for name in
                 ('fresh','retirement_proven','intent_resolved','needs_observation',
                  'completed','private_diagnostic_completed')),
             'Exact diagnostic flags required')
    _require(type(facts.get('protocol_faults')) is tuple and
             facts.get('status') in ('new','waiting','pending','completion_candidate','completed','retained'),
             'Exact diagnostic status and fault history required')
    _require(facts['completed']==facts['private_diagnostic_completed'],
             'Private completion scope required')
    if facts['completed']:
        _require(facts['intent_resolved'] and not facts['protocol_faults'],
                 'Resolved fault-free completion candidate required')
    if facts['retirement_proven']:
        _require(facts['fresh'] and facts['completed'] and facts['intent_resolved']
                 and not facts['protocol_faults'],'Fresh normal retirement required')
    return facts

class _Coordinator:
    """Finite policy; no disk, native loader, OS observation, local clock or exit."""
    def __init__(self,attempt):
        self.attempt=attempt
        self.started=self.busy=False
        self.operations=self.observation_grants=self.refreshes=0
        self.events,self.errors=[],[]
        self.facts=None
        self.status='not_started'

    def _error(self,where,error):
        self.errors.append((where,error))
        self.status='unresolved_retained'

    def _operation(self,name,observation=False):
        if self.operations>=MAX_OPERATIONS:
            self.status='operation_budget_retained';return False
        self.operations+=1
        event=dict(operation=self.operations,method=name,observation=observation)
        self.events.append(event)  # BEFORE method lookup, with no replay on loss.
        try:
            method=getattr(self.attempt,name)
            reply=method(observation=observation) if name=='step' else method()
            event['reply']=reply
            self.facts=_check_view(reply)
            return True
        except BaseException as error:
            self.facts=None;event['error']=(type(error).__name__,str(error)[:4096])
            self._error(name,error);return False

    def start(self,pause):
        _require(not self.started and not self.busy,'One coordinator creation lifetime')
        self.started=self.busy=True  # BEFORE create_once method lookup.
        try:
            self._operation('create_once')
            return self._drive(pause)
        finally:self.busy=False

    def advance(self,pause):
        _require(self.started and not self.busy,'Original nonreentrant coordinator required')
        self.busy=True
        try:return self._drive(pause)
        finally:self.busy=False

    def _drive(self,pause):
        while self.operations<MAX_OPERATIONS:
            # Normal progress is provisional. Spend the four explicit reads
            # only on uncertain/lost replies, leaving a NEW final read to EXIT.
            if self.facts is None or (self.facts.get('pending') is not None
                                       and not self.facts['protocol_faults']):
                if not self._operation('read'):
                    self.status='read_unresolved_retained';break
            facts=self.facts
            if facts['completed']:
                self.status='normal_completion_requires_new_host_read';break
            if facts['protocol_faults'] or not facts['intent_resolved'] or facts['status']=='retained':
                self.status='uncertain_intent_or_fault_retained';break
            terminal=facts.get('terminal')
            candidate=facts['status']=='completion_candidate'
            if type(terminal) is not dict or (terminal.get('transferred') and not candidate):
                self.status='ineligible_retained';break
            # A recovered handoff candidate needs one deadline-checked step;
            # the consumer performs no repeated handoff/consuming operation.
            observation=facts['needs_observation']
            if observation:
                if self.observation_grants>=MAX_OBSERVATIONS:
                    self.status='observation_budget_retained';break
                self.observation_grants+=1
                try:pause(DELAY)
                except BaseException as error:
                    self._error('observation_delay',error);break
            self._operation('step',observation)
        else:self.status='operation_budget_retained'
        return self.facts

    def refresh(self):
        _require(self.started and not self.busy and self.refreshes<MAX_REFRESHES,
                 'Finite original refresh required')
        self.refreshes+=1;self.busy=True
        event=dict(refresh=self.refreshes,method='read');self.events.append(event)
        try:
            reply=self.attempt.read();event['reply']=reply
            self.facts=_check_view(reply);return self.facts
        except BaseException as error:
            self.facts=None;event['error']=(type(error).__name__,str(error)[:4096])
            self._error('refresh',error);return None
        finally:self.busy=False

    def cached(self):
        facts=_check_view(self.attempt.cached())
        _require(not facts['fresh'] and not facts['retirement_proven'],
                 'Cached facts cannot authorize retirement')
        return facts

class _Host:
    """Retains all roots; only a fresh independent durable proof permits exit."""
    def __new__(cls,*args,**kwargs):
        result=object.__new__(cls)
        result.intent_started=result.native_may_exist=result.exit_ready=False
        _LIFETIME.append(result)
        return result

    def __init__(self,io,schema,consumer,admission,journal,fifo,token,pause):
        self.io,self.schema,self.consumer,self.admission=io,schema,consumer,admission
        self.journal,self.fifo,self.pause=journal,fifo,pause
        self.commands=io.Commands(token)
        self.binding,self.bridge,self.root=admission.binding,admission.bridge,admission.root
        self.run_consumed=self.storage_failed=False
        self.lines=self.read_failures=self.proof_reads=self.protocol_faults=0
        self.first_error=None
        self.errors,self.proofs,self.replies=[],[],[]
        self.attempt=consumer.Attempt(admission)
        self.coordinator=_Coordinator(self.attempt)

    def remember(self,where,error):
        if self.first_error is None:self.first_error=error
        if len(self.errors)<MAX_ERRORS:self.errors.append((where,error))

    def record(self,kind,**facts):
        if self.storage_failed:return False
        try:
            self.journal.append(dict(kind=kind,signals=_SIGNALS,**facts));return True
        except BaseException as error:
            self.storage_failed=True;self.remember('journal_'+kind,error);return False

    def transcript(self,kind):
        c=self.coordinator
        return self.record(kind,events=c.events,facts=c.facts,operations=c.operations,
                           observation_grants=c.observation_grants,refreshes=c.refreshes,status=c.status,
                           errors=tuple((w,type(e).__name__,str(e)[:4096]) for w,e in self.errors),
                           coordinator_errors=tuple((w,type(e).__name__,str(e)[:4096]) for w,e in c.errors))

    def _empty_registry(self):
        evidence=[]
        for unused in range(2):
            pending=self.bridge.pending();self.replies.append(pending)
            history=self.bridge.history();self.replies.append(history)
            _require(pending is None and type(history) is tuple and not history,
                     'Twice-empty original registry required')
            evidence.append(dict(pending_none=True,history_empty=True))
        return evidence

    def _exit_proof(self):
        _require(not self.protocol_faults and not self.storage_failed and self.proof_reads<MAX_PROOFS,
                 'No protocol/storage fault and finite fresh proof required')
        self.proof_reads+=1  # Before any getters or reads can lose replies.
        if not self.intent_started:
            _require(not self.native_may_exist and not self.coordinator.started
                     and self.admission.execution is None,'No creation intent/execution required')
            return dict(retirement='no_attempt',registry=self._empty_registry())
        _require(self.native_may_exist and self.coordinator.started,'Original creation intent required')
        facts=self.attempt.read();self.replies.append(facts)  # NEW read, never cached/coordinator result.
        _check_view(facts)
        _require(facts['fresh'] and facts['retirement_proven'] and facts['intent_resolved']
                 and not facts['protocol_faults'],'Fresh fault-free normal retirement required')
        session=self.admission.execution
        _require(type(session) is self.schema.ChildExecution and self.admission.owns_execution(session)
                 and session._claim is self.admission and session._binding is self.binding
                 and session._bridge is self.bridge and session._fault is None
                 and not session._native_failure_seen and session._active_pending is None
                 and session._admission_started and session._create_entered and not session._busy,
                 'Original ready private execution and resolved intents required')
        sink,child,generation=session._sink,session._child,session._generation
        _require(type(sink) is self.binding.TerminalSink and type(child) is self.binding.Child
                 and type(generation) is int and generation>0,'Original exact Child/Sink/generation required')
        terminal=facts['terminal'];self.schema._terminal_report(terminal,generation)
        encoded=self.io.encode(terminal)  # Immutable comparison also catches in-place fake mutations.
        proof=dict(facts=facts,registry=[],snapshots=[],first_active=None)
        self.proofs.append(proof)  # Before independent bridge/sink getters can fail.
        def registry():
            pending=self.bridge.pending();self.replies.append(pending)
            history=self.bridge.history();self.replies.append(history)
            _require(pending is None and type(history) is tuple and len(history)==1 and history[0] is sink,
                     'Stable original terminal history required')
            proof['registry'].append(dict(pending_none=True,original_sink=True,generation=generation))
        registry()
        for unused in range(2):
            snapshot=sink.snapshot();proof['snapshots'].append(snapshot)
            self.schema._terminal_report(snapshot,generation)
            _require(self.io.encode(snapshot)==encoded,'Independent original sink snapshot changed')
        first=sink.active_first_terminal();proof['first_active']=first
        self.schema._active_report(first,generation,fresh=False,first=True)
        registry()  # Both original-sink snapshots are independently bracketed.
        _require(first['entered']==first['returned']==1 and first['rc']==0
                 and 1<=first['sequence']<=MAX_OBSERVATIONS and first['record']['state']==2
                 and first['record']['exit_kind']==1 and first['record']['exit_status']==0
                 and first['record']['returncode']==0,'First active normal exit0 required')
        final=terminal['final_report'];decoded=terminal['decoded_record']
        _require(terminal['rooted']==terminal['transferred']==1 and terminal['kind']==2
                 and terminal['known_no_creation']==0 and final['state']==11
                 and all(terminal[n]==1 for n in ('reap_attempted','wait_entered','wait_returned',
                                                'consumed','decoded','certificate_matches'))
                 and terminal['wait_rc']==0 and final['resources_closed']==1
                 and final['resource_error']==0,'Transferred matching normal consuming certificate required')
        identity=('pid','pgid','ppid','uid','sid','exit_kind','exit_status','returncode')
        _require(all(first['record'][n]==decoded[n] for n in identity)
                 and first['creator_pid']==final['registration_ppid']
                 and first['creator_uid']==final['registration_uid'],
                 'First active exit and consuming certificate disagree')
        _require(self.schema._classify_child_terminal(terminal,generation)==('exited',0,None),
                 'No historical native failure, abort or forced teardown allowed')
        return proof

    def dispatch(self,command):
        try:
            if command=='RUN':
                if self.run_consumed or self.protocol_faults or self.storage_failed:
                    self.protocol_faults+=1;return False
                self.run_consumed=self.intent_started=True
                _free(self.root)
                if not self.record('RUN_INTENT'):return False
                self.native_may_exist=True  # Before coordinator/method access.
                self.coordinator.start(self.pause)
                self.transcript('RUN_RESULT')
            elif command=='ADVANCE':
                if (not self.native_may_exist or not self.coordinator.started or self.storage_failed
                        or self.protocol_faults or self.coordinator.operations>=MAX_OPERATIONS):return False
                if self.record('ADVANCE_INTENT',operations=self.coordinator.operations):
                    self.coordinator.advance(self.pause);self.transcript('ADVANCE_RESULT')
            elif command=='READ':
                if self.coordinator.started and self.coordinator.refreshes<MAX_REFRESHES:
                    self.coordinator.refresh();self.transcript('READ_RESULT')
            elif command=='CACHED':
                if self.coordinator.started:self.record('CACHED_RESULT',facts=self.coordinator.cached())
            elif command=='EXIT':
                proof=self._exit_proof()
                if self.record('EXIT_READY',proof=proof,proof_reads=self.proof_reads,
                               run_consumed=self.run_consumed,intent_started=self.intent_started,
                               native_may_exist=self.native_may_exist):
                    self.exit_ready=True;return True
            else:self.protocol_faults+=1
        except BaseException as error:
            self.remember('command_'+str(command),error)
            self.transcript('COMMAND_FAILURE')
        return False

    def serve(self):
        while self.lines<self.io.MAX_COMMANDS and self.read_failures<MAX_FAILURES:
            try:
                chunk=self.fifo.read()
                _require(type(chunk) is bytes and 0<len(chunk)<=128,'Nonempty bounded control chunk required')
                for command in self.commands.feed(chunk):
                    if self.lines>=self.io.MAX_COMMANDS:break
                    self.lines+=1
                    if self.dispatch(command):return 0
            except BaseException as error:
                self.read_failures+=1;self.remember('control_read',error)
                self.record('CONTROL_FAILURE',failure=self.read_failures,
                            error=(type(error).__name__,str(error)[:4096]))
                if self.read_failures<MAX_FAILURES:
                    try:self.pause(1)
                    except BaseException as delay_error:self.remember('control_delay',delay_error)
        _dormant()  # Exhaustion grants no exit, logging growth or further native action.

def _hex(value,size):
    _require(type(value) is str and len(value)==size and
             all(c in '0123456789abcdef' for c in value),'Exact lowercase hex required')

def _identity(s):
    return dict(device=s.st_dev,inode=s.st_ino,uid=s.st_uid,mode=stat.S_IMODE(s.st_mode))

def _stable(s):
    return (_identity(s),s.st_size,s.st_nlink,s.st_mtime_ns,s.st_ctime_ns)

def _unique(pairs):
    result={}
    for key,value in pairs:
        _require(key not in result,'Duplicate JSON key');result[key]=value
    return result

def _read_file(name,maximum,uid,*,root_fd=None,pin=None,mode=None):
    fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=root_fd)
    _LIFETIME.append(fd)  # Keep even partial reads/failures; never close/retry here.
    before=os.fstat(fd)
    _require(stat.S_ISREG(before.st_mode) and before.st_nlink==1 and before.st_uid==uid
             and 0<before.st_size<=maximum and not stat.S_IMODE(before.st_mode)&0o022
             and (mode is None or stat.S_IMODE(before.st_mode)==mode),'Owned regular stable file required')
    chunks=[];remaining=before.st_size+1
    while remaining:
        chunk=os.read(fd,min(65536,remaining))
        if not chunk:break
        chunks.append(chunk);remaining-=len(chunk)
    raw=b''.join(chunks)
    path_stat=os.stat(name,dir_fd=root_fd,follow_symlinks=False)
    _require(_stable(before)==_stable(os.fstat(fd))==_stable(path_stat)
             and len(raw)==before.st_size,'Same-descriptor source/path changed')
    if pin is not None:
        _require(type(pin) is dict and set(pin)=={'bytes','mode','sha256'}
                 and type(pin['bytes']) is int and type(pin['mode']) is int
                 and len(raw)==pin['bytes'] and stat.S_IMODE(before.st_mode)==pin['mode']
                 and hashlib.sha256(raw).hexdigest()==pin['sha256'],'Sealed source digest required')
    return raw

def _environment(environ):
    key='SIMS_NORMAL_MANIFEST_SHA256'
    fixed=dict(PATH='/usr/bin:/bin',LC_ALL='C',TMPDIR='/private/tmp')
    _require(set(environ) in (set(fixed)|{key},set(fixed)|{key,'__CF_USER_TEXT_ENCODING'})
             and all(environ[k]==v for k,v in fixed.items()),'Exact minimal normal environment required')
    _hex(environ[key],64)
    if '__CF_USER_TEXT_ENCODING' in environ:
        value=environ['__CF_USER_TEXT_ENCODING']
        _require(type(value) is str and len(value)<=128 and '\0' not in value,'Bounded encoding required')
    return environ[key]

def _bootstrap_sources(family,pin,uid):
    prefix='sims-normal-true-';name=os.path.basename(family)
    _require(os.path.dirname(family)=='/private/tmp' and name.startswith(prefix),'Fresh direct normal family required')
    _hex(name[len(prefix):],32)
    before=os.lstat(family)
    _require(stat.S_ISDIR(before.st_mode) and before.st_uid==uid
             and stat.S_IMODE(before.st_mode)==0o700,'Owned nonsymlink normal family required')
    fd=os.open(family,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
    _LIFETIME.append(fd)
    _require(_identity(before)==_identity(os.fstat(fd))==_identity(os.lstat(family)),
             'Original bootstrap family required')
    _require(set(os.listdir(fd))==set(FILES)|{'manifest.json','control.fifo','child-cwd'},
             'Exact initial normal siblings required')
    raw=_read_file('manifest.json',1024**2,uid,root_fd=fd,mode=0o400)
    _require(hashlib.sha256(raw).hexdigest()==pin,'Parent manifest digest required')
    manifest=json.loads(raw.decode('utf-8'),object_pairs_hook=_unique,
                        parse_constant=lambda unused:_require(False,'Finite JSON required'))
    _require(type(manifest) is dict and set(manifest)=={'schema','profile','root','fifo','token','files','child_cwd','target','python'}
             and type(manifest['schema']) is int and manifest['schema']==1
             and manifest['profile']==PROFILE and manifest['root']==_identity(before),
             'Exact private normal manifest required')
    _require(type(manifest['files']) is dict and set(manifest['files'])==set(FILES),
             'Exact sealed normal source set required')
    sources={}
    for filename in FILES:
        item=manifest['files'][filename]
        _require(type(item) is dict and set(item)=={'bytes','mode','sha256'}
                 and type(item['bytes']) is int and 1<=item['bytes']<=4*1024**2
                 and type(item['mode']) is int and item['mode'] in (0o400,0o500),
                 'Bounded exact sealed source pin required')
        _hex(item['sha256'],64)
        sources[filename]=_read_file(filename,4*1024**2,uid,root_fd=fd,pin=item)
    image=manifest['files']['_window_owned_child_bridge.so']
    _require(image['bytes']==109552 and image['sha256']=='2b591afbf653005501243afcc848f94d5e38b95e7323c2fb9826595c6cf27bf6',
             'Only the reviewed private image pin is admitted')
    python=_read_file(PYTHON,32*1024**2,0)
    _require(hashlib.sha256(python).hexdigest()==PYTHON_SHA256
             and manifest['python']==dict(path=PYTHON,sha256=PYTHON_SHA256),
             'Actual pinned interpreter required')
    _require(_identity(os.fstat(fd))==_identity(os.lstat(family))==manifest['root'],
             'Bootstrap family changed')
    return manifest,sources

def _source(name,raw,path):
    _require(name not in sys.modules,'No module reload or adoption')
    module=types.ModuleType(name);module.__file__=path;module.__package__=''
    _LIFETIME.append(module);sys.modules[name]=module
    exec(compile(raw,path,'exec',dont_inherit=True),module.__dict__)
    return module

def _interrupt(signum,frame):
    global _SIGNALS
    _SIGNALS|={1:1,2:2,15:4}.get(signum,0)

def _policies():
    import resource
    import signal
    resource.setrlimit(resource.RLIMIT_CORE,(0,0))
    soft,hard=resource.getrlimit(resource.RLIMIT_FSIZE);cap=64*1024**2
    if hard!=resource.RLIM_INFINITY:cap=min(cap,hard)
    if soft!=resource.RLIM_INFINITY:cap=min(cap,soft)
    resource.setrlimit(resource.RLIMIT_FSIZE,(cap,cap))
    for name in ('SIGINT','SIGTERM','SIGHUP'):
        signal.signal(getattr(signal,name),_interrupt)  # Never change SIGCHLD.

def main():
    global _STARTED
    _require(not _STARTED,'One fresh normal host lifetime');_STARTED=True
    pid=os.getpid()
    _require(pid!=47707,'Reserved process is forbidden')  # Before image/target/path access.
    _require(sys.platform=='darwin' and sys.version_info[:2]==(3,9) and sys.executable==PYTHON
             and len(sys.argv)==1 and os.path.abspath(sys.argv[0])==os.path.abspath(__file__)
             and os.path.basename(__file__)==FILES[0] and sys.flags.isolated==1
             and sys.flags.no_site==1 and sys.flags.no_user_site==1
             and sys.flags.dont_write_bytecode==1 and 'site' not in sys.modules,
             'Pinned fresh normal Python -I -B -S entry required')
    pin=_environment(os.environ)
    _require(all(name not in sys.modules for name in MODULES+('_window_owned_child_bridge',)),
             'Fresh canonical modules required')
    host=journal=None
    try:
        family=os.path.dirname(os.path.abspath(__file__))
        manifest,sources=_bootstrap_sources(family,pin,os.getuid())
        loaded={}
        for name in MODULES:
            loaded[name]=_source(name,sources[name+'.py'],family+'/'+name+'.py')
        _policies()
        admission=loaded['window_normal_admission'].prepare_private_true(family,pin)
        _LIFETIME.append(admission)
        io=loaded['window_suspended_host_io'];root=admission.root
        _require(admission.manifest==manifest and root.admitted==manifest['root'],
                 'Original authenticated normal Admission required')
        journal=io.Journal(root);_LIFETIME.append(journal)
        fifo=io.Fifo(root,manifest['fifo']);_LIFETIME.append(fifo)
        import time
        host=_Host(io,loaded['window_owned_child_bridge'],loaded['window_normal_diagnostic'],
                   admission,journal,fifo,manifest['token'],time.sleep)
        _require(host.record('HOST_READY',profile=PROFILE,manifest_sha256=pin,
                             production_child=0,production_legacy=0,alpha_gates=(False,)*7),
                 'Durable normal host admission required')
        result=host.serve()
        _require(result==0 and host.exit_ready,'Only durable EXIT_READY permits host return')
        return 0
    except BaseException as error:
        must_retain=host is not None and (host.intent_started or host.native_may_exist)
        try:
            _LIFETIME.append(error)
            if host is not None:
                host.remember('main',error);host.transcript('HOST_FAILURE')
            elif journal is not None:
                try:journal.append(dict(kind='SETUP_FAILURE',error=(type(error).__name__,str(error)[:4096])))
                except BaseException as journal_error:_LIFETIME.append(journal_error)
        finally:
            if must_retain:_dormant()  # Even reporting/allocation failures cannot release roots.
        return 1  # Setup refusal before any possible Child dispatch only.

if __name__=='__main__':sys.exit(main())
