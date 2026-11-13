"""Bounded private-normal consumer; SOURCE ONLY, all controls UNRUN.

Parent API: Attempt(original_admission), create_once(), read(), cached(),
step(observation=False). Only step(observation=True) permits an active query.
The first query also needs that permission; later queries need >=10ms since
the previous dispatch. The parent schedules calls; this module never sleeps.
The external monotonic 30s deadline starts before the factory/create intent.
read/cached are memory-only, and a read cannot certify a recovered handoff:
a later step must check the external deadline before completion is reported.
After the first terminal, a permanent per-session _operation guard admits only
the reserved next cleanup operation. The ORIGINAL finish_step and _operation
implementations still perform selection, sticky reservation and dispatch. This
private source dependency closes the re-read-to-signal gap without editing the
shared module, wrapping native types or accepting a caller-supplied callback.

Admission is imported lazily only by the real constructor. Its source is a
separate integration dependency, absent from source2196-v1. No loader, native
import, process operation, environment switch, CLI, cleanup-on-error or finalizer
is provided here. for_injected_mock accepts an already-rooted source session
with explicit Python heap mocks and an attached synthetic claim. It never
constructs an Admission, authenticates native types or promotes that session.

All attempts, inputs, issued identities, replies and original exceptions remain
rooted. Detached results carry scope and false qualification/health/Continue
flags. Completion is private diagnostic evidence, never permission for host
exit: the parent still needs a NEW read, registry-bracketed identical original
sink snapshots and durable fsynced EXIT_READY. Python internals and same-UID
mutation are not a security boundary. Pathname-exec/rename limitations belong
to the admitting host; no wall-time bound on native libc calls is claimed.
"""

from time import monotonic_ns as _monotonic_ns

import window_owned_child_bridge as _protocol


PROFILE = 'NORMAL_TRUE_ONLY_PRIVATE_DIAGNOSTIC_NO_PRODUCTION'
PRIVATE_SCOPE = 'private-normal-diagnostic'
MOCK_SCOPE = 'injected-mock'
DEADLINE_NS = 30_000_000_000
QUERY_DELAY_NS = 10_000_000
MAX_OPERATIONS = 32
MAX_ACTIVE = 8
MAX_RECOVERY_READS = 4
_MAX_NS = 2 ** 64 - 1
_ATTEMPTS = []
_CLEANUP = ('cleanup_begin', 'child_observe', 'child_members', 'child_latch',
            'child_reap', 'handoff')
_CONSUMING = ('reap_attempted', 'wait_entered', 'wait_returned', 'consumed',
              'decoded', 'certificate_matches')
_IDENTITY = ('pid', 'pgid', 'ppid', 'uid', 'sid', 'exit_kind', 'exit_status', 'returncode')
_LAUNCH = tuple(name for name in _protocol._CHILD_SCALARS
                if not name.startswith(('cleanup_', 'term_', 'kill_', 'latch_'))
                and name not in ('state', 'rc', 'system_errno', 'blocked',
                                 'clock_rc', 'clock_errno', 'signal_failed'))


def _copy(value):
    if type(value) in (int, bool, str, bytes, type(None)):
        return value
    if type(value) is tuple:
        return tuple(_copy(item) for item in value)
    if type(value) is dict and all(type(key) is str for key in value):
        return {key: _copy(item) for key, item in value.items()}
    raise _protocol.BridgeProtocolError('Non-data diagnostic fact')


def _heap(kind):
    return (type(kind) is type and bool(kind.__flags__ & (1 << 9)) and
            kind.__base__ is object)


def _launch_succeeded(fact, report):
    # C's LC_* ledger preserves raw returns, not normalized error statuses.
    # PID/UID/descriptor/session/byte-count APIs legitimately return positives.
    # The fixed producer validates the exact SDK BSD size before registered=1.
    kind, entered, returned, value, raw_errno = fact
    if not (1 <= kind <= 19 and entered == returned == 1):
        return False
    if kind == 2:  # LC_GETPID
        return value == report['registration_ppid'] and 1 < value <= 2**31-1
    if kind == 3:  # LC_GETUID
        return value == report['registration_uid'] and 0 <= value <= 2**32-1
    if kind == 4:  # LC_DUP
        return 3 <= value <= 2**31-1
    if kind == 14:  # LC_PROCINFO: successful byte count, exact size checked in C.
        return 0 < value <= 2**31-1
    if kind == 15:  # LC_GETSID
        return value == report['registration_sid'] == report['registration_pid'] and 1 < value <= 2**31-1
    return value == 0  # All other declared LC_* operations return status codes.


class Attempt:
    """One original Admission/session lifetime. No copy, reset, adoption or retry."""

    def __new__(cls, *arguments, **keywords):
        result = object.__new__(cls)
        result._initialized = False
        result._roots, result._errors, result._failures = [], [], []
        _ATTEMPTS.append(result)  # Before validation, properties or imports.
        return result

    def __init__(self, admission):
        self._initialize(admission, False)

    @classmethod
    def for_injected_mock(cls, session):
        result = Attempt.__new__(cls)
        result._initialize(session, True)
        return result

    def _initialize(self, supplied, synthetic):
        self._hold(supplied)
        if self._initialized:
            error = _protocol.BridgeProtocolError('Attempt cannot be reinitialized')
            self._remember('constructor', error)
            raise error
        self._initialized = True
        self._ready = self._busy = self._blocked = self._completed = False
        self._scope = MOCK_SCOPE if synthetic else PRIVATE_SCOPE
        self._admission = None if synthetic else supplied
        self._session = self._binding = self._bridge = self._claim = None
        self._child_type = self._terminal_type = None
        self._child = self._sink = self._generation = None
        self._root = self._manifest = None
        self._options = None
        self._operation_method = None
        self._guarded = False
        self._create_requested = self._factory_requested = self._admit_requested = False
        self._operations = self._active_dispatches = self._recovery_reads = 0
        self._origin_ns = self._deadline_ns = self._checked_ns = self._last_active_ns = None
        self._pending = self._expected_sequence = None
        self._cleanup_index = 0
        self._intents = []
        self._protocol_faults = []
        self._terminal = self._active = self._first = self._original_launch = None
        self._previous = self._first_facts = self._exit = self._bounds = None
        self._session_intents = ()
        try:
            self._check(type(self) is Attempt, 'Exact Attempt required')
            self._check(not any(item is not self and
                ((not synthetic and getattr(item, '_admission', None) is supplied) or
                 (synthetic and getattr(item, '_session', None) is supplied))
                for item in _ATTEMPTS), 'One owner belongs to one retained Attempt')
            if synthetic:
                self._attach(supplied)
            else:
                from window_normal_admission import Admission
                self._check(type(supplied) is Admission, 'Original exact Admission required')
                self._check(supplied.execution is None, 'Existing execution cannot be adopted')
            self._ready = True
        except BaseException as error:
            self._remember('constructor', error)
            raise

    @property
    def native_qualified(self):
        return False

    @property
    def continue_authorized(self):
        return False

    @property
    def healthy(self):
        return False

    def __copy__(self):
        raise TypeError('Attempt cannot be copied')

    def __deepcopy__(self, memo):
        raise TypeError('Attempt cannot be copied')

    def __reduce_ex__(self, protocol):
        raise TypeError('Attempt cannot be serialized')

    def _hold(self, value):
        if value is not None and not any(item is value for item in self._roots):
            self._roots.append(value)
        return value

    def _remember(self, where, error):
        self._hold(error)
        if isinstance(error, _protocol.BridgeProtocolError):
            self._blocked = True
            fault = where + ': ' + type(error).__name__
            if fault not in self._protocol_faults:
                self._protocol_faults.append(fault)
        if not any(item is error for item in self._errors):
            self._errors.append(error)
            # Do not call arbitrary exception __str__/repr during reconciliation.
            self._failures.append((where, type(error).__name__))

    def _check(self, condition, message):
        if not condition:
            self._blocked = True
            if message not in self._protocol_faults:
                self._protocol_faults.append(message)
            raise _protocol.BridgeProtocolError(message)

    def _enter(self):
        if (type(self) is not Attempt or not any(item is self for item in _ATTEMPTS) or
                not self._ready or self._busy):
            error = _protocol.BridgeProtocolError('Unready/reentrant Attempt')
            self._remember('entry', error)
            raise error
        self._busy = True

    def _attach(self, session):
        self._hold(session)  # Original issuance is retained even on refusal.
        self._check(type(session) is _protocol.ChildExecution and
                    any(item is session for item in _protocol._CHILD_EXECUTIONS),
                    'Actual already-rooted source ChildExecution required')
        if self._session is not None:
            self._check(session is self._session, 'Issued execution replaced')
            return
        if self._admission is not None:
            self._check(self._admission.execution is session and
                        self._admission.owns_execution(session) is True,
                        'Admission did not issue this execution')
        binding = self._hold(session._binding)
        bridge = self._hold(session._bridge)
        claim = self._hold(session._claim)
        child_type, terminal_type = session._child_type, session._terminal_type
        self._check(not any(item is not self and getattr(item, '_bridge', None) is bridge
                            for item in _ATTEMPTS), 'Bridge already retained by an Attempt')
        self._check(not session._admission_started and not session._create_entered and
                    session._child is session._sink is session._generation is None and
                    session._fault is None and not session._intent and
                    not session._native_failure_seen and not session._busy,
                    'Only a fresh issued execution is accepted')
        if self._admission is None:
            kinds = (type(bridge), child_type, terminal_type)
            self._check(all(_heap(kind) for kind in kinds) and len(set(kinds)) == 3,
                        'Only explicit distinct Python heap mock types')
            for kind, methods in ((type(bridge), ('create', 'pending', 'history', 'terminal',
                                      'child_snapshot', 'active_observe', 'cleanup_begin',
                                      'child_observe', 'child_members', 'child_latch',
                                      'child_reap', 'handoff')),
                                  (terminal_type, ('snapshot', 'active_observation',
                                                   'active_first_terminal'))):
                self._check(all(type(kind.__dict__.get(name)) is type(_heap) for name in methods),
                            'Mock operations must be explicit Python functions')
            self._check(_heap(type(claim)) and
                        type(type(claim).__dict__.get('_admit')) is type(_heap),
                        'Attached explicit synthetic claim required')
        else:
            admission = self._admission
            self._check(claim is admission and admission.binding is binding and
                        admission.bridge is bridge and admission.child_type is child_type and
                        admission.terminal_type is terminal_type,
                        'Admission/session identities differ')
            self._root = self._hold(admission.root)
            # Admission exposes a detached value, not stable Python identity.
            self._manifest = self._hold(_copy(admission.manifest))
        self._session, self._binding, self._bridge, self._claim = session, binding, bridge, claim
        self._child_type, self._terminal_type = child_type, terminal_type
        self._options = _copy(session._options)

    def _owner(self):
        s = self._hold(self._session)
        self._check(type(s) is _protocol.ChildExecution and
                    any(item is s for item in _protocol._CHILD_EXECUTIONS),
                    'Original rooted session changed')
        for value in (s._binding, s._bridge, s._claim, s._child, s._sink, s._history):
            self._hold(value)
        self._check(s._binding is self._binding and s._bridge is self._bridge and
                    s._claim is self._claim and s._child_type is self._child_type and
                    s._terminal_type is self._terminal_type and s._options == self._options,
                    'Original session/owner/options changed')
        self._check(type(s._active_attempts) is int and
                    0 <= s._active_attempts <= self._active_dispatches,
                    'Foreign active dispatch or changed source counter')
        if self._child is not None:
            self._check(s._child is self._child and s._sink is self._sink and
                        s._generation == self._generation, 'Original Child/sink/generation changed')
        if self._guarded:
            method = self._hold(s._operation)
            self._check(type(method) is type(self._owner) and method.__self__ is self and
                        method.__func__ is Attempt._guard_operation,
                        'Permanent cleanup dispatch guard replaced')
        if self._admission is not None:
            a = self._admission
            self._check(a.execution is s and a.owns_execution(s) is True and
                        a.binding is self._binding and a.bridge is self._bridge and
                        a.root is self._root and a.manifest == self._manifest and
                        a.child_type is self._child_type and a.terminal_type is self._terminal_type,
                        'Admission resources or original issuance changed')
        for error in s._errors:
            self._remember('session', error)
        self._check(s._fault is None and not s._native_failure_seen,
                    'Historical native/protocol failure retained')

    def _clock(self):
        try:
            now = _monotonic_ns()
            self._check(type(now) is int and 0 <= now <= _MAX_NS,
                        'Exact bounded monotonic integer required')
            self._check(self._checked_ns is None or now >= self._checked_ns,
                        'External monotonic clock regressed')
            self._checked_ns = now
            if self._deadline_ns is None:
                self._check(now <= _MAX_NS - DEADLINE_NS, 'External deadline overflow')
                self._origin_ns, self._deadline_ns = now, now + DEADLINE_NS
            self._check(now < self._deadline_ns, 'External deadline exhausted; uncertainty retained')
            return now
        except BaseException as error:
            self._blocked = True
            self._remember('external-clock', error)
            raise

    def _reserve(self, name):
        self._check(self._operations < MAX_OPERATIONS, 'Coordinator operation cap exhausted')
        self._operations += 1
        self._intents.append(name)
        self._pending = name  # BEFORE any method/property dispatch.

    def _dispatch(self, name):
        self._reserve(name)
        try:
            if name != 'create_execution':
                self._owner()
            if name == 'create_execution':
                self._factory_requested = True
                issued = self._hold(self._admission.create_execution())
                self._attach(issued)
                self._pending = None
            elif name == 'admit':
                self._admit_requested = True
                self._session.admit()
            elif name == 'observe_active':
                self._active_dispatches += 1
                self._last_active_ns = self._checked_ns
                self._expected_sequence = 1 if self._active is None else self._active['sequence'] + 1
                self._hold(self._session.observe_active())
            else:
                self._hold(self._session.finish_step())
        except BaseException as error:
            self._remember(name, error)
            if isinstance(error, _protocol.BridgeProtocolError):
                self._blocked = True
            return False
        finally:
            # Even a consumed status/lost reply cannot bypass the external bound.
            self._clock()
        if name != 'create_execution':
            self._refresh()
        return self._pending is None

    def _install_guard(self):
        if self._guarded:
            return
        self._owner()
        method = self._hold(self._session._operation)
        self._check(type(method) is type(self._owner) and
                    method.__self__ is self._session and
                    method.__func__ is _protocol.ChildExecution._operation,
                    'Original source operation method required')
        self._operation_method = method
        self._session._operation = self._guard_operation
        self._hold(self._session._operation)
        self._guarded = True  # Never restore/reset; all roots remain retained.

    def _guard_operation(self, name, *arguments):
        # finish_step may select from a NEW snapshot. Refuse an unexpected
        # selection BEFORE the original source operation can access the Bridge.
        self._check(self._busy and self._guarded and self._pending in _CLEANUP and
                    name == self._pending and
                    arguments == ((1, 0) if name == 'child_latch' else ()),
                    'Unexpected cleanup selection; no signal/abort/retry permitted')
        self._check(len(self._session._intent) == self._cleanup_index,
                    'Cleanup operation already has an unresolved source intent')
        return self._operation_method(name, *arguments)

    def create_once(self):
        self._enter()
        try:
            if self._create_requested or self._blocked:
                return self._facts()
            self._create_requested = True
            self._clock()  # Fixed deadline precedes factory and session admission.
            if self._admission is not None:
                if not self._dispatch('create_execution'):
                    return self._facts()
            self._dispatch('admit')
        except BaseException as error:
            self._remember('create_once', error)
        finally:
            self._busy = False
        return self._facts()

    def _roots_from_session(self):
        s = self._session
        child, sink = self._hold(s._child), self._hold(s._sink)
        self._check(type(child) is self._child_type and type(sink) is self._terminal_type,
                    'Original exact Child/sink required')
        if self._child is None:
            self._child, self._sink, self._generation = child, sink, s._generation
        self._check(child is self._child and sink is self._sink and
                    s._generation == self._generation, 'Child/sink/generation replaced')
        self._check(type(s._history) is tuple and len(s._history) == 1 and
                    s._history[0] is self._sink, 'Original one-entry registry required')

    def _refresh(self):
        if self._session is None:
            self._check(self._factory_requested, 'No original execution requested')
            issued = self._hold(self._admission.execution)
            if issued is None:
                return  # Never repeat the factory or infer no-creation.
            self._attach(issued)
            self._pending = None
        self._owner()
        if not self._admit_requested:
            return
        old_errors = len(self._session._errors)
        self._session.read_completion()  # Existing memory-only recovery, no cleanup.
        self._owner()
        if len(self._session._errors) != old_errors:
            raise _protocol.ReconciliationRequired('Retained memory read was lost')
        self._roots_from_session()
        terminal = self._hold(self._session._last)
        _protocol._terminal_report(terminal, self._generation)
        self._check(len(terminal['raw_siginfo']) <= 4096, 'Unbounded terminal bytes')
        t = _copy(terminal)
        self._progress(t)
        self._terminal = t
        latest = self._hold(self._session.read_active())
        first = self._hold(self._session.read_active(first_terminal=True))
        self._owner()
        final_terminal = self._hold(self._session._last)
        self._check(final_terminal == terminal, 'Terminal facts changed across memory reads')
        _protocol._active_report(latest, self._generation, fresh=False, first=False)
        _protocol._active_report(first, self._generation, fresh=False, first=True)
        self._check(latest['rc'] == first['rc'] == 0, 'Failed active observation retained')
        self._check(latest['sequence'] <= self._active_dispatches,
                    'Active sequence exceeds reserved dispatches')
        self._active, self._first = _copy(latest), _copy(first)
        if first['entered']:
            facts = _protocol._active_facts(first)
            self._check(self._first_facts is None or self._first_facts == facts,
                        'First active terminal changed')
            self._first_facts = facts
            record = first['record']
            self._check(record['state'] == 2 and record['exit_kind'] == 1 and
                        record['exit_status'] == record['returncode'] == 0,
                        'First active terminal is not normal exit0')
            self._exit = tuple(record[name] for name in _IDENTITY)
        self._resolve()

    def _progress(self, t):
        r = t['final_report']
        self._check(t['known_no_creation'] == 0 and r['state'] in (6, 9, 10, 11),
                    'Unsupported normal state; no abort/no-creation adoption')
        self._check(all(r[name] == 0 for name in (
            'rc', 'system_errno', 'blocked', 'launch_failure_mask', 'launch_first_failure_phase',
            'launch_first_failure_reason', 'launch_first_woc_rc', 'launch_first_system_errno',
            'resource_error', 'clock_rc', 'term_attempted', 'kill_attempted', 'signal_failed',
            'term_returned', 'kill_returned', 'term_rc', 'kill_rc', 'term_errno',
            'kill_errno', 'cleanup_kill_at_ns')) and all(r['abort'][name] == 0 for name in (
                'started', 'phase', 'entries', 'observation_count', 'trace_count',
                'kill_entered', 'kill_returned', 'failure_rc', 'failure_system_errno',
                'last_operation', 'last_rc', 'last_system_errno', 'preexisting_exit', 'exit_valid')),
            'Historical failure, abort or signal; retained without forced teardown')
        self._check(all(r[name] == 1 for name in (
            'published', 'pid_valid', 'registered', 'spawn_entered', 'spawn_returned',
            'resume_entered', 'resume_returned', 'resources_closed',
            'sigchld_preflight_entered', 'sigchld_preflight_returned',
            'sigchld_preflight_policy_valid', 'sigchld_preflight_default',
            'sigchld_preflight_accepted')) and
            r['spawn_rc'] == r['resume_rc'] == r['sigchld_preflight_rc'] ==
            r['sigchld_preflight_no_cldwait'] == 0 and
            all(_launch_succeeded(fact, r) for fact in r['launch_facts']),
            'Successful registered/resumed/published launch required')
        for name in ('spawn_errno', 'resume_errno', 'sigchld_preflight_errno', 'clock_errno'):
            self._check(0 <= r[name] <= 2 ** 31 - 1, 'Malformed raw successful errno')
        launch = tuple(r[name] for name in _LAUNCH) + (r['launch_facts'], _copy(r['registration_observation']))
        self._check(self._original_launch is None or launch == self._original_launch,
                    'Original launch/owner facts changed')
        self._original_launch = launch
        self._check(t['registration_observation'] == r['registration_observation'],
                    'Registration slots disagree')
        if self._previous is not None:
            p = self._previous
            for name in _CONSUMING + ('transferred',):
                self._check(t[name] >= p[name], 'Terminal consuming intent regressed')
            for name in ('cleanup_started', 'cleanup_entries', 'cleanup_now_ns', 'latch_attempted'):
                self._check(r[name] >= p['final_report'][name], 'Cleanup intent/time regressed')
            if p['wait_returned']:
                self._check(all(t[name] == p[name] for name in (
                    'wait_rc', 'wait_errno', 'wait_pid', 'wait_uid', 'wait_code', 'wait_status',
                    'raw_siginfo', 'decoded_record')), 'Consuming certificate changed')
        if r['cleanup_started']:
            bounds = (r['cleanup_deadline_ns'], r['max_cleanup_entries'])
            self._check(bounds[0] > 0 and (self._bounds is None or self._bounds == bounds),
                        'Native cleanup bounds renewed')
            self._bounds = bounds
        self._previous = _copy(t)
        # No out-of-band cleanup or consuming wait can be credited to us.
        native = tuple(self._session._intent)
        self._check(len(native) <= self._cleanup_index + (self._pending in _CLEANUP) and
                    tuple(item[0] for item in native) == _CLEANUP[:len(native)],
                    'Foreign cleanup/signal intent')
        if any(t[name] for name in _CONSUMING):
            self._check('child_reap' in self._intents and self._exit is not None,
                        'Unowned consuming wait')
            self._certificate(t)
        self._session_intents = native

    def _certificate(self, t):
        d, r = t['decoded_record'], t['final_report']
        self._check(all(t[name] == 1 for name in _CONSUMING) and t['kind'] == 2 and
                    t['wait_rc'] == 0 and 0 <= t['wait_errno'] <= 2 ** 31 - 1 and
                    0 < len(t['raw_siginfo']) <= 4096 and t['wait_code'] == 1 and
                    t['wait_pid'] == r['registration_pid'] == r['spawn_pid_raw'] and
                    t['wait_status'] == 0 and
                    tuple(d[name] for name in _IDENTITY) == self._exit and
                    r['record'] == d and
                    d['state'] == 2 and d['error'] == d['system_errno'] == 0 and
                    d['reap_attempted'] == d['reaped'] == d['no_signals'] == 1 and
                    d['signal_attempted'] == d['wait_uid_available'] == 0 and d['uid_evidence'] == 2,
                    'Unmatched/uncertified consuming wait')

    def _resolve(self):
        t, a = self._terminal, self._active
        r = t['final_report']
        name = self._pending
        if name == 'admit':
            self._check(r['state'] == 6 and not r['cleanup_started'] and
                        not any(t[key] for key in _CONSUMING), 'Creation is not untouched state6')
            self._pending = None
        elif name == 'observe_active':
            if a['sequence'] != self._expected_sequence:
                return  # Lost before entry: do not redispatch the uncertain call.
            self._check(a['entered'] == a['returned'] == 1,
                        'Active intent lacks a complete retained call')
            self._pending = None
        elif name in _CLEANUP:
            index = _CLEANUP.index(name)
            if len(self._session_intents) != index + 1:
                return
            record = r['record']
            done = (
                r['cleanup_started'] == 1 and r['cleanup_entries'] == 1,
                r['cleanup_entries'] == 2 and record['state'] == 2,
                r['cleanup_entries'] == 3 and record['state'] == 0 and record['count'] == 1,
                r['cleanup_entries'] == 4 and r['latch_attempted'] == record['no_signals'] == 1,
                r['cleanup_entries'] == 5 and t['consumed'] == 1,
                t['transferred'] == 1 and r['state'] == 11,
            )[index]
            if done:
                self._cleanup_index = index + 1
                self._pending = None
        if self._exit is not None and r['cleanup_started']:
            self._safe_next()

    def _safe_next(self):
        t = self._terminal
        r, index = t['final_report'], self._cleanup_index
        record = r['record']
        self._check(self._exit is not None and self._first_facts is not None,
                    'Observation must precede all cleanup')
        if index == 0:
            self._check(r['state'] == 6 and not r['cleanup_started'], 'Unexpected cleanup entry')
        elif index == 1:
            self._check(r['cleanup_entries'] == 1, 'Unexpected first cleanup slot')
        elif index == 2:
            self._check(record['state'] == 2 and
                        record['error'] == record['system_errno'] ==
                        record['signal_attempted'] == record['reap_attempted'] == record['reaped'] == 0 and
                        tuple(record[name] for name in _IDENTITY) == self._exit,
                        'Cleanup observation disagrees with first exit; no signal polling')
        elif index == 3:
            self._check(record['state'] == 0 and record['count'] == 1 and
                        record['member0'] == record['pid'] == self._exit[0] and
                        record['member1'] == record['error'] == record['system_errno'] == 0 and
                        record['raw_bytes'] == record['pid_bytes'] == 4 and
                        record['signal_attempted'] == record['no_signals'] ==
                        record['reap_attempted'] == record['reaped'] == 0 and
                        tuple(record[name] for name in _IDENTITY[:5]) == self._exit[:5] and
                        self._session._exit_certificate == (1, 0),
                        'Membership not anchor-only; no TERM/KILL permitted')
        elif index == 4:
            self._check(r['latch_attempted'] == record['no_signals'] == 1 and
                        record['error'] == record['system_errno'] == record['signal_attempted'] ==
                        record['reap_attempted'] == record['reaped'] == 0 and not t['reap_attempted'] and
                        tuple(record[name] for name in _IDENTITY) == self._exit,
                        'Uncertain latch cannot select a consuming retry')
        elif index >= 5:
            self._certificate(t)
        if index < 6 and r['cleanup_started']:
            self._check(r['cleanup_entries'] < r['max_cleanup_entries'] and
                        r['cleanup_now_ns'] < r['cleanup_deadline_ns'],
                        'Native deadline/counter exhausted; resources retained')

    def _candidate(self):
        if self._terminal is None or self._pending is not None or self._cleanup_index != 6:
            return False
        t = self._terminal
        status, code, error = _protocol._classify_child_terminal(t, self._generation)
        self._check(status == 'exited' and code == 0 and error is None and
                    t['kind'] == 2 and t['transferred'] == 1 and
                    t['final_report']['state'] == 11, 'Unsuccessful terminal transfer')
        return True

    def read(self):
        """At most four explicit recovery reads; no clock, query or cleanup."""
        self._enter()
        fresh = False
        try:
            self._check(self._recovery_reads < MAX_RECOVERY_READS, 'Memory recovery cap exhausted')
            self._recovery_reads += 1  # Reserve before any property/method access.
            if self._create_requested:
                self._refresh()
                fresh = self._terminal is not None and not self._blocked
        except BaseException as error:
            self._remember('read', error)
        finally:
            self._busy = False
        return self._facts(fresh=fresh)

    def cached(self):
        self._enter()
        try:
            return self._facts()
        finally:
            self._busy = False

    def step(self, observation=False):
        self._enter()
        try:
            self._check(type(observation) is bool, 'Exact observation permission required')
            if self._blocked or not self._create_requested:
                return self._facts()
            now = self._clock()
            if self._pending is not None:
                return self._facts()  # Only explicit bounded read may reconcile loss.
            if not self._admit_requested:
                self._dispatch('admit')
            elif self._exit is None:
                if observation:
                    self._check(self._active_dispatches < MAX_ACTIVE, 'Active dispatch cap exhausted')
                    if self._last_active_ns is None or now - self._last_active_ns >= QUERY_DELAY_NS:
                        self._dispatch('observe_active')
            elif self._cleanup_index < 6:
                self._safe_next()
                self._install_guard()
                self._dispatch(_CLEANUP[self._cleanup_index])
            if not self._blocked and self._candidate():
                self._completed = True  # Current step checked the external deadline.
        except BaseException as error:
            self._remember('step', error)
        finally:
            self._busy = False
        return self._facts()

    def _facts(self, fresh=False):
        completed = self._completed and not self._blocked and self._pending is None
        status = ('retained' if self._blocked else 'completed' if completed else
                  'pending' if self._pending is not None else
                  'completion_candidate' if self._cleanup_index == 6 else
                  'new' if not self._create_requested else 'waiting')
        return dict(scope=self._scope, profile=PROFILE, status=status,
                    completed=completed, private_diagnostic_completed=completed and self._scope == PRIVATE_SCOPE,
                    fresh=fresh, retirement_proven=fresh and completed,
                    intent_resolved=self._pending is None and not self._blocked,
                    needs_observation=(self._admit_requested and self._terminal is not None and
                                       self._exit is None and self._pending is None and not self._blocked),
                    protocol_faults=tuple(self._protocol_faults[:64]),
                    native_qualified=False, continue_authorized=False, healthy=False,
                    create_requested=self._create_requested, factory_requested=self._factory_requested,
                    admit_requested=self._admit_requested, pending=self._pending,
                    operations=self._operations, active_dispatches=self._active_dispatches,
                    recovery_reads=self._recovery_reads, origin_ns=self._origin_ns,
                    deadline_ns=self._deadline_ns, deadline_checked_ns=self._checked_ns,
                    cleanup_index=self._cleanup_index, intents=tuple(self._intents),
                    errors_count=len(self._errors), failures=tuple(self._failures[:64]),
                    retained_roots=len(self._roots), terminal=_copy(self._terminal),
                    active=_copy(self._active), first_terminal=_copy(self._first),
                    first_active=_copy(self._first))
