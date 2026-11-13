"""Pure suspended-diagnostic consumer. SOURCE ONLY; all validation UNRUN.

Parent wiring (the parent admits the private profile and authentic loaded image)::

    attempt = Attempt(binding, bridge, executable=b'/usr/bin/true',
                      argv=(b'/usr/bin/true',), env=exact_env, cwd=exact_cwd,
                      stdio=exact_fds, launch_ns=launch_ns, cleanup_ns=cleanup_ns,
                      grace_ns=grace_ns, max_entries=max_entries)
    facts = attempt.create_once()
    facts = attempt.read()              # fresh recovery, including after loss
    facts = attempt.step()              # at most one action
    facts = attempt.step(observation=True)  # ONLY at a parent-scheduled slot

The parent owns delays, serialization, durable receipts and lifetime decisions.
There is no scheduler, loader, clock, I/O, numeric owner, normal execution,
production integration or native authenticity here. Injected protocol types,
including test doubles, can never establish qualification. The caller must not
operate the Bridge separately. Eight follow-up observations follow begin's slot
zero; their native prepare-origin deadline and 16-entry budget are never renewed.

Only a fresh coherent read can return retirement_proven=True, for this diagnostic
only. create/step can contain a fresh read made BEFORE its action; after dispatch
another read is required. cached() never grants exit permission.
Every result is detached data; copying/serializing the Attempt is forbidden.
All attempts, bindings, roots, replies and original exceptions are retained for
the interpreter lifetime. Private Python mutation is not a security boundary.
"""

from window_owned_child_bridge import (
    BridgeProtocolError, _child_report, _integer, _require, _terminal_report,
)


SCOPE = 'suspended-diagnostic-only'
_ATTEMPTS = []
_SCALARS = (int, bool, str, bytes, type(None))
_NON_ROOTS = (int, float, complex, str, bytes, bytearray, list, tuple, dict,
              set, frozenset, range, memoryview)
_CONSUMING = ('reap_attempted', 'wait_entered', 'wait_returned', 'consumed',
              'decoded', 'certificate_matches')
_FAILURE_FIELDS = ('rc', 'system_errno', 'blocked', 'launch_failure_mask',
                   'launch_first_failure_phase', 'launch_first_failure_reason',
                   'launch_first_woc_rc', 'launch_first_system_errno',
                   'resource_error', 'clock_rc', 'clock_errno', 'signal_failed')


def _copy(value):
    """Copy only the bounded, validated protocol's exact builtin data."""
    if type(value) in _SCALARS:
        return value
    if type(value) is dict:
        _require(all(type(key) is str for key in value), 'Non-string fact key')
        return {key: _copy(item) for key, item in value.items()}
    if type(value) is tuple:
        return tuple(_copy(item) for item in value)
    raise BridgeProtocolError('Non-data protocol fact')


def _string(value, name):
    _require(type(value) is bytes and 0 < len(value) < 65536 and b'\0' not in value,
             'Exact bounded bytes required: ' + name)


def _no_consuming(t):
    return not any(t[name] for name in _CONSUMING)


class Attempt:
    """One permanently rooted Bridge/creation lifetime; no reset or release.

    Constructor arguments are mandatory. executable/argv are fixed to true;
    env/cwd/stdio are exact parent-admitted values, not inspected OS resources.
    binding.Bridge/Child/TerminalSink must be distinct exact nonscalar types.
    Constructor refusal can itself leave a permanently retained diagnostic root.
    """

    def __new__(cls, *arguments, **keywords):
        _require(cls is Attempt, 'Exact Attempt required')
        return object.__new__(cls)

    def __init__(self, binding, bridge, *, executable, argv, env, cwd, stdio,
                 launch_ns, cleanup_ns, grace_ns, max_entries):
        _require(not any(item is self for item in _ATTEMPTS), 'No reinitialization')
        duplicate = any(item._bridge is bridge for item in _ATTEMPTS)
        self._binding, self._bridge = binding, bridge
        self._child = self._sink = self._generation = None
        self._child_type = self._sink_type = None
        self._arguments = None
        self._ready = self._requested = self._dispatched = self._busy = False
        self._returned_child = False
        self._roots, self._replies, self._errors = [], [], []
        self._failures, self._protocol_faults, self._intents = [], [], []
        self._once, self._pending_intent = set(), None
        self._last = self._last_snapshot = self._previous = self._original = self._bounds = None
        self._registration_raw, self._abort_raw = None, {}
        self._native_failures = []
        _ATTEMPTS.append(self)  # BEFORE binding access or anything that can lose a reply.
        try:
            _require(not duplicate, 'One Bridge belongs to one retained Attempt')
            _require(binding is not None, 'Explicit binding required')
            kinds = (binding.Bridge, binding.Child, binding.TerminalSink)
            _require(all(type(kind) is type and kind not in (object, type, type(None))
                         and not issubclass(kind, _NON_ROOTS) for kind in kinds)
                     and len(set(kinds)) == 3, 'Distinct concrete binding types required')
            _require(type(bridge) is kinds[0], 'Exact binding Bridge required')
            self._child_type, self._sink_type = kinds[1:]
            _require(type(executable) is bytes and executable == b'/usr/bin/true'
                     and type(argv) is tuple and argv == (executable,)
                     and type(argv[0]) is bytes, 'Fixed exact true executable/argv required')
            _string(cwd, 'cwd')
            _require(cwd.startswith(b'/'), 'Absolute parent-admitted cwd required')
            _require(type(env) is tuple and len(env) <= 128, 'Exact bounded env tuple required')
            for item in env:
                _string(item, 'environment')
                _require(b'=' in item and not item.startswith(b'='), 'Environment key required')
            _require(type(stdio) is tuple and len(stdio) == 3, 'Exact stdio triple required')
            for fd in stdio:
                _integer(fd, 0, 2 ** 31 - 1, 'stdio')
            for number in (launch_ns, cleanup_ns):
                _integer(number, 1, 60000000000, 'native timeout')
            _integer(grace_ns, 0, cleanup_ns, 'native grace')
            _integer(max_entries, 1, 512, 'cleanup budget')
            self._arguments = (executable, argv, env, cwd, stdio, launch_ns,
                               cleanup_ns, grace_ns, max_entries)
            self._ready = True
        except BaseException as error:
            self._remember('admission', error)
            raise

    @property
    def native_qualified(self):
        return False

    @property
    def continue_authorized(self):
        return False

    def __copy__(self):
        raise TypeError('Attempt cannot be copied')

    def __deepcopy__(self, memo):
        raise TypeError('Attempt cannot be copied')

    def __reduce_ex__(self, protocol):
        raise TypeError('Attempt cannot be serialized')

    def _enter(self):
        _require(type(self) is Attempt and any(item is self for item in _ATTEMPTS)
                 and self._ready and not self._busy, 'Unadmitted/reentrant Attempt')
        self._busy = True

    def _remember(self, where, error):
        self._errors.append(error)  # Retain the ORIGINAL exception, including interruption.
        fact = (where, type(error).__name__, str(error))
        if fact not in self._failures:
            self._failures.append(fact)

    def _check(self, condition, message):
        if not condition:
            if message not in self._protocol_faults:
                self._protocol_faults.append(message)
            raise BridgeProtocolError(message)

    def _hold(self, root):
        if root is not None and not any(item is root for item in self._roots):
            self._roots.append(root)
        return root

    def _pending(self):
        pending = self._hold(self._bridge.pending())
        if pending is not None:
            self._check(type(pending) is self._child_type, 'Nonexact pending Child')
            if self._child is None:
                self._child = pending
            self._check(pending is self._child, 'Pending Child replaced')
        return pending

    def _history(self):
        history = self._bridge.history()
        self._replies.append(history)
        if type(history) is tuple:
            for item in history:
                self._hold(item)
        self._check(type(history) is tuple and len(history) <= 1, 'Nonexact one-attempt history')
        if history:
            sink = history[0]
            self._check(type(sink) is self._sink_type, 'Nonexact history TerminalSink')
            if self._sink is None:
                self._sink = sink
            self._check(sink is self._sink, 'History TerminalSink replaced')
        return history

    def _snapshot(self):
        reply = self._sink.snapshot()
        self._replies.append(reply)
        _terminal_report(reply, self._generation)
        terminal = _copy(reply)
        self._last_snapshot = _copy(terminal)  # Diagnostic access survives later read loss.
        if self._generation is None:
            self._generation = terminal['generation']
        self._check(terminal['registration_observation'] ==
                    terminal['final_report']['registration_observation'],
                    'Registration scalar slots disagree')
        self._progress(terminal)
        return terminal

    def _progress(self, t):
        r, a = t['final_report'], t['final_report']['abort']
        self._check(not a['started'] or ('abort_begin', 0) in self._once,
                    'Abort without this Attempt begin intent')
        self._check(not any(t[name] for name in _CONSUMING) or ('abort_reap', 0) in self._once,
                    'Consuming facts without this Attempt reap intent')
        self._check(a['observation_count'] <= sum(intent[0] == 'abort_observe' for intent in self._intents)
                    and a['entries'] <= sum(intent[0].startswith('abort_') for intent in self._intents),
                    'Native entries exceed this Attempt intents')
        self._check(r['max_cleanup_entries'] == self._arguments[-1], 'Cleanup budget changed')
        original = {name: value for name, value in r.items() if name not in ('abort', 'state')}
        if self._original is None:
            self._original = _copy(original)
        self._check(original == self._original, 'Original launch facts changed')
        bounds = (r['launch_deadline_ns'], a['origin_ns'], a['deadline_ns'],
                  a['max_entries'], a['max_observations'])
        if self._bounds is None:
            self._bounds = bounds
        self._check(bounds == self._bounds, 'Prepare-origin deadline/budgets changed')
        self._check(r['resume_entered'] == r['resume_returned'] == r['published'] == 0,
                    'Suspended-only boundary violated')
        self._check(not r['cleanup_started'] and not r['cleanup_entries'] and
                    not r['term_attempted'] and not r['kill_attempted'] and not r['latch_attempted'],
                    'Ordinary cleanup boundary violated')
        previous = self._previous
        if previous is not None:
            p, b = previous['final_report'], previous['final_report']['abort']
            self._check(r['state'] == p['state'] or
                        t['kind'] == 1 and t['transferred'] and p['state'] == 7 and r['state'] == 11,
                        'Original state changed outside no-creation transfer')
            for name in ('entries', 'now_ns', 'started', 'kill_entered', 'kill_returned',
                         'observation_count', 'trace_count', 'exit_valid'):
                self._check(a[name] >= b[name], 'Abort history regressed: ' + name)
            self._check(a['phase'] >= b['phase'], 'Abort phase regressed')
            self._check(not b['failure_rc'] or
                        (a['failure_rc'], a['failure_system_errno']) ==
                        (b['failure_rc'], b['failure_system_errno']), 'Abort failure changed')
            if b['kill_returned']:
                self._check(all(a[name] == b[name] for name in ('kill_rc', 'kill_errno')),
                            'Completed kill output rewritten')
            if b['exit_valid']:
                self._check(a['exit_observation_index'] == b['exit_observation_index'],
                            'First abort exit slot changed')
            for old, new in zip(b['trace'], a['trace']):
                self._check(old == new or old[2] == 0 and old[:2] == new[:2],
                            'Abort trace rewritten')
            for old, new in zip(b['observations'], a['observations']):
                self._check((not old['entered'] or new['entered']) and
                            (not old['returned'] or old == new), 'Abort raw slot rewritten')
            for name in _CONSUMING + ('transferred',):
                self._check(t[name] >= previous[name], 'Terminal intent regressed: ' + name)
            if previous['wait_returned']:
                self._check(all(t[name] == previous[name] for name in
                                ('raw_siginfo', 'wait_rc', 'wait_errno', 'wait_pid',
                                 'wait_uid', 'wait_code', 'wait_status')),
                            'Completed consuming output rewritten')
        failure = tuple(r[name] for name in _FAILURE_FIELDS) + (
            a['failure_rc'], a['failure_system_errno'])
        if any(failure) and failure not in self._native_failures:
            self._native_failures.append(failure)
        self._previous = _copy(t)

    def _raw(self, scalar, key):
        # Reserved SDK storage, even nonempty, has NO collection authority.
        if scalar['entered'] != 1 or scalar['returned'] != 1:
            return {'status': 'unentered', 'scalar': _copy(scalar), 'bytes': None}
        raw = (self._sink.registration_observation() if key is None else
               self._sink.abort_observation(key))
        self._replies.append(raw)
        self._check(type(raw) is bytes and len(raw) == scalar['raw_siginfo_bytes']
                    and 0 < len(raw) <= 4096, 'Exact raw slot length/type mismatch')
        fact = (_copy(scalar), raw)
        previous = self._registration_raw if key is None else self._abort_raw.get(key)
        self._check(previous is None or previous == fact, 'Completed raw bytes changed')
        if key is None:
            self._registration_raw = fact
        else:
            self._abort_raw[key] = fact
        return {'status': 'returned', 'scalar': _copy(scalar), 'bytes': raw}

    def _reconcile(self, t):
        intent = self._pending_intent
        if intent is None:
            return True
        name, index, entries, count = intent
        a = t['final_report']['abort']
        if name == 'handoff':
            done = t['transferred'] == 1 and t['kind'] == 1
        elif name == 'abort_handoff':
            done = t['transferred'] == 1 and t['kind'] == 3 and a['phase'] == 7
        else:
            operation = {'abort_begin': 1, 'abort_observe': 2, 'abort_reap': 3}[name]
            done = (a['entries'] == entries + 1 and a['last_operation'] == operation
                    and a['phase'] in (3, 4, 6, 8))
            if name == 'abort_begin':
                done = done and a['started'] == 1
            if name == 'abort_observe':
                done = done and a['observation_count'] == count + 1
            if name == 'abort_reap':
                done = done and a['phase'] in (6, 8)
            done = (done and a['kill_entered'] == a['kill_returned']
                    and t['wait_entered'] == t['wait_returned']
                    and all(fact[1] == fact[2] == 1 for fact in a['trace'])
                    and all(slot['entered'] == slot['returned'] for slot in a['observations']))
        if done:
            self._pending_intent = None
        return bool(done)

    def _recover(self):
        self._check(self._requested and self._dispatched, 'No dispatched creation to recover')
        pending, history = self._pending(), self._history()
        if pending is None and not history:
            self._check(self._child is None and self._sink is None and not self._returned_child,
                        'Original root/history disappeared')
            self._check(self._pending() is None and self._history() == (),
                        'Empty registry read changed')
            return {'terminal': None, 'registration_raw': None, 'abort_raw': (),
                    'retirement': 'preparation_rejected' if self._dispatched else None,
                    'intent_resolved': True}
        self._check(len(history) == 1, 'Root without its permanent terminal history')
        t = self._snapshot()
        if not t['transferred']:
            self._check(pending is self._child and pending is not None, 'Missing original pending Child')
            sink = self._hold(self._bridge.terminal(self._child))
            self._check(sink is self._sink, 'Child terminal identity changed')
            reply = self._bridge.child_snapshot(self._child)
            self._replies.append(reply)
            _child_report(reply, self._generation)
            self._check(reply == t['final_report'], 'Child/sink snapshots disagree')
        else:
            self._check(pending is None and self._child is not None, 'Unproven transferred root')
            name = 'handoff' if t['kind'] == 1 else 'abort_handoff'
            self._check(any(intent[0] == name for intent in self._intents),
                        'Transfer without this Attempt handoff intent')
        registration = self._raw(t['registration_observation'], None)
        observations = tuple(self._raw(slot, index) for index, slot in
                             enumerate(t['final_report']['abort']['observations']))
        self._check(self._snapshot() == t, 'Terminal changed during coherent read')
        self._check(self._pending() is pending and self._history() == history,
                    'Registry changed during coherent read')
        resolved = self._reconcile(t)
        retirement = self._retirement(t) if resolved else None
        return {'terminal': t, 'registration_raw': registration, 'abort_raw': observations,
                'retirement': retirement, 'intent_resolved': resolved}

    def _read(self):
        try:
            self._last = self._recover()
            return True
        except BaseException as error:
            self._remember('read', error)
            return False

    def _no_creation(self, t, transferred=False):
        r, a = t['final_report'], t['final_report']['abort']
        return (t['known_no_creation'] == 1 and t['kind'] == 1 and _no_consuming(t)
                and r['spawn_entered'] == r['spawn_returned'] == 0
                and r['resources_closed'] == 1 and r['resource_error'] == 0
                and r['state'] == (11 if transferred else 7)
                and a['phase'] == a['started'] == a['entries'] == a['kill_entered'] == a['trace_count'] == 0
                and not any(slot['entered'] for slot in a['observations']))

    def _provenance(self, t):
        r = t['final_report']
        # These APIs retain raw errno even on success. The return code and
        # owning C facts qualify success; raw errno remains diagnostic evidence.
        return (not t['known_no_creation'] and r['spawn_entered'] == r['spawn_returned'] == 1
                and r['spawn_rc'] == 0
                and r['registered'] == r['pid_valid'] == 1
                and r['registration_pid'] == r['spawn_pid_raw'] == r['registration_pgid'] ==
                r['registration_sid'] > 1 and r['registration_ppid'] > 1
                and r['registration_uid'] >= 0 and r['registration_bsd_status'] == 4
                and r['resources_closed'] == 1 and r['resource_error'] == 0
                and r['resume_entered'] == r['published'] == 0)

    def _certificate(self, t, transferred=False):
        r, d = t['final_report'], t['decoded_record']
        a = r['abort']
        index = a['exit_observation_index']
        if not (self._provenance(t) and t['kind'] == 3 and r['state'] in (4, 8)
                and a['phase'] == (7 if transferred else 6) and a['started'] == a['exit_valid'] == 1
                and a['last_operation'] == (4 if transferred else 3)
                and a['failure_rc'] == a['last_rc'] == 0
                and all(t[name] == 1 for name in _CONSUMING)
                and t['wait_rc'] == 0 and 0 < len(t['raw_siginfo']) <= 4096
                and 0 <= index <= 8):
            return False
        slot = a['observations'][index]
        valid_exit = (d['exit_kind'] == 1 and 0 <= d['exit_status'] <= 255 and
                      d['returncode'] == d['exit_status'] or
                      d['exit_kind'] in (2, 3) and 1 <= d['exit_status'] <= 127 and
                      d['returncode'] == -d['exit_status'])
        return (valid_exit and d['state'] == 2 and d['error'] == d['system_errno'] == 0
                and d['reaped'] == d['reap_attempted'] == d['no_signals'] == 1
                and d['pid'] == t['wait_pid'] == r['registration_pid']
                and d['pid'] == d['pgid'] == d['sid']
                and d['ppid'] == r['registration_ppid'] and d['uid'] == r['registration_uid']
                and d['wait_uid_available'] == 0 and d['uid_evidence'] == 2
                and t['wait_code'] > 0 and t['wait_status'] == d['exit_status']
                and slot['entered'] == slot['returned'] == 1 and slot['rc'] == 0
                and slot['pid'] == d['pid'] and slot['code'] == t['wait_code']
                and slot['status'] == d['exit_status'] and index in self._abort_raw
                and all(a['record'][name] == d[name] for name in
                        ('pid', 'pgid', 'ppid', 'uid', 'sid', 'state', 'exit_kind',
                         'exit_status', 'returncode', 'reaped', 'reap_attempted', 'no_signals')))

    def _retirement(self, t):
        if self._protocol_faults or not t['transferred']:
            return None
        if self._no_creation(t, transferred=True):
            return 'kind1_no_creation'
        if self._certificate(t, transferred=True):
            return 'kind3_unresumed_abort'
        return None

    def _eligible(self, t):
        r, a = t['final_report'], t['final_report']['abort']
        allowed = (1 << 9) | (1 << 10)
        launch_ok = (r['state'] == 4 and r['blocked'] == r['rc'] == r['launch_failure_mask'] == 0
                     or r['state'] == 8 and r['blocked'] == 1 and r['rc'] != 0
                     and r['launch_failure_mask'] != 0 and r['launch_first_failure_reason'] in (10, 11))
        return (self._provenance(t) and launch_ok and _no_consuming(t) and t['kind'] == 0
                and r['launch_failure_mask'] & ~allowed == 0
                and r['signal_failed'] == 0 and all(fact[1] == fact[2] == 1 for fact in r['launch_facts'])
                and r['sigchld_preflight_entered'] == r['sigchld_preflight_returned'] ==
                r['sigchld_preflight_policy_valid'] == r['sigchld_preflight_default'] ==
                r['sigchld_preflight_accepted'] == 1
                and r['sigchld_preflight_rc'] == r['sigchld_preflight_no_cldwait'] == 0
                and a['phase'] == a['started'] == a['entries'] == a['trace_count'] == a['kill_entered'] == 0
                and a['deadline_ns'] > 0 and not any(slot['entered'] for slot in a['observations']))

    def _dispatch(self, name, t, index=0):
        key = (name, index)
        self._check(key not in self._once, 'Uncertain action cannot be repeated')
        a = t['final_report']['abort']
        intent = (name, index, a['entries'], a['observation_count'])
        self._once.add(key)
        self._intents.append(intent)
        self._pending_intent = intent  # BEFORE method access/native dispatch.
        try:
            method = getattr(self._bridge, name)
            reply = method(self._child, self._sink) if name in ('handoff', 'abort_handoff') else method(self._child)
            self._replies.append(reply)
            if name in ('handoff', 'abort_handoff'):
                self._hold(reply)
                _require(reply is self._sink, 'Handoff reply is not the original sink')
            else:
                _child_report(reply, self._generation)
        except BaseException as error:
            self._remember(name, error)

    def _view(self, fresh=False, action=None, reason='fresh_read_required'):
        facts = _copy(self._last) if self._last is not None else {
            'terminal': None, 'registration_raw': None, 'abort_raw': (),
            'retirement': None, 'intent_resolved': False}
        retirement = facts['retirement'] if fresh and not self._protocol_faults else None
        t = facts['terminal']
        a = t['final_report']['abort'] if t is not None else None
        needs_observation = bool(fresh and t is not None and not t['transferred']
                                 and not self._protocol_faults and facts['intent_resolved']
                                 and self._observation_allowed(t))
        facts.update(scope=SCOPE, native_qualified=False, continue_authorized=False,
                     healthy=False, fresh=fresh, action=action, reason=reason,
                     retirement=retirement, retirement_proven=retirement is not None,
                     needs_observation=needs_observation,
                     observations_remaining=8 - a['observation_count'] if a else None,
                     failures=tuple(self._failures), protocol_faults=tuple(self._protocol_faults),
                     native_failures=tuple(self._native_failures), intents=tuple(self._intents),
                     pending_intent=self._pending_intent)
        # Raw bytes successfully read BEFORE a later read failure remain accessible.
        facts['retained_registration_raw'] = _copy(self._registration_raw)
        facts['retained_abort_raw'] = tuple((index, _copy(value)) for index, value in
                                           sorted(self._abort_raw.items()))
        # This schema-checked snapshot may precede a failed coherence/identity
        # check. It is retained evidence ONLY, never used to grant retirement.
        facts['retained_snapshot'] = _copy(self._last_snapshot)
        return facts

    def _observation_allowed(self, t):
        r, a = t['final_report'], t['final_report']['abort']
        return (self._provenance(t) and a['started'] == 1 and a['phase'] == 3
                and a['kill_entered'] == a['kill_returned'] == 1 and a['kill_rc'] == 0
                and a['failure_rc'] == 0 and a['last_rc'] == 0
                and a['now_ns'] < a['deadline_ns'] and a['entries'] < 15
                and a['observation_count'] < 8 and _no_consuming(t))

    def create_once(self):
        self._enter()
        try:
            _require(not self._requested, 'Creation is once-only, including preflight loss')
            self._requested = True
            try:
                # Retain pre-existing objects, but NEVER adopt them into this attempt.
                pending = self._hold(self._bridge.pending())
                history = self._bridge.history()
                self._replies.append(history)
                if type(history) is tuple:
                    for item in history:
                        self._hold(item)
                self._check(pending is None and type(history) is tuple and history == (),
                            'Nonempty fresh Bridge; adoption forbidden')
                self._dispatched = True  # Native creation intent, never repeated.
                try:
                    child = self._hold(self._bridge.create_suspended(*self._arguments))
                    self._check(type(child) is self._child_type, 'Nonexact creation Child')
                    self._child, self._returned_child = child, True
                except BaseException as error:
                    self._remember('create_suspended', error)
            except BaseException as error:
                self._remember('preflight', error)
            fresh = self._read() if self._dispatched else False
            return self._view(fresh, reason='creation_read' if fresh else 'creation_unresolved')
        finally:
            self._busy = False

    def read(self):
        """Finite, fresh same-root reconciliation; exceptions remain diagnostic facts."""
        self._enter()
        try:
            fresh = self._read()
            return self._view(fresh, reason='coherent_read' if fresh else 'read_unresolved')
        finally:
            self._busy = False

    def cached(self):
        """No bridge access; copied retained facts, never retirement permission."""
        return self._view()

    def step(self, *, observation=False):
        """One action maximum. observation=True is an explicit parent scheduling grant."""
        _require(type(observation) is bool, 'Exact observation scheduling bool required')
        self._enter()
        try:
            fresh = self._read()
            if not fresh or self._protocol_faults or not self._last['intent_resolved']:
                return self._view(fresh, reason='unresolved_retained')
            t = self._last['terminal']
            if t is None or t['transferred']:
                return self._view(True, reason='retirement_read')
            r, a = t['final_report'], t['final_report']['abort']
            name, index = None, 0
            if self._no_creation(t):
                name = 'handoff'
            elif self._certificate(t) and a['entries'] < 16:
                name = 'abort_handoff'  # Memory-only late transfer, no deadline reset.
            elif a['failure_rc'] == a['last_rc'] == 0 and a['now_ns'] < a['deadline_ns'] and a['entries'] < 15:
                if self._eligible(t):
                    name = 'abort_begin'
                elif self._observation_allowed(t) and observation:
                    name, index = 'abort_observe', a['observation_count'] + 1
                elif (self._provenance(t) and a['phase'] == 4 and a['started'] == a['exit_valid'] == 1
                      and _no_consuming(t) and 0 <= a['exit_observation_index'] <= 8
                      and a['exit_observation_index'] in self._abort_raw
                      and a['record']['no_signals'] == 1):
                    name = 'abort_reap'
            if name is None:
                return self._view(True, reason='need_observation' if self._observation_allowed(t)
                                  else 'ineligible_or_budget_retained')
            self._dispatch(name, t, index)
            return self._view(False, action=name, reason='fresh_read_required')
        except BaseException as error:
            self._remember('step', error)
            return self._view(False, reason='unresolved_retained')
        finally:
            self._busy = False
