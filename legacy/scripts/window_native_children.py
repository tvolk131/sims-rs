"""Source-only direct-native Child manager; native C path UNRUN and disabled.

No subprocess, Popen adoption, numeric PID adoption, returned-object facade, mock
backend injection, cached status, implicit environment, or Python waiter exists.
The exact C Child is returned to the caller. Native pending/history roots cover
pre-return interruption and mapping failures; Python receipts are supplementary.

The bootstrap guard is unconditional. The unreachable binding below describes the
intended integration after independently reviewed native/bootstrap qualification;
it is not a loader or a route to enable this candidate. Hostile Python mutation
is not an authenticity/security boundary. Existing builder/legacy bridge untouched.

After a caller-side interruption, a fresh reviewed manager obtains the same
library-rooted Bridge and can use pending/history without resetting native
deadlines or counters. This also recovers an interrupted Python busy-latch
cleanup; Python bookkeeping is never the durable ownership authority.
"""

QUALIFICATIONS = {
    'native': False, 'layer_b': False, 'build': False, 'abi': False,
    'platform': False, 'exclusive_waiter': False, 'sigchld': False,
    'lifecycle': False, 'trusted_bootstrap': False, 'mapped_image': False,
    'builder_selected': False, 'driver_enabled': False,
    'runtime_backend': False, 'app': False, 'alpha': False,
    'creation_error_contract': False, 'suspended_start': False, 'resume': False,
    'pre_return_interruption': False, 'terminal_handoff': False,
    'compiler37394_failure_resolved': False,
}
LIVE, EXITED = 1, 2
UID_CURRENT, UID_RETAINED = 1, 2
TERM, KILL = 1, 2
ACTIVE, ABORTED, REAP_RECORDED = 6, 7, 10


class Unsupported(RuntimeError):
    pass


class ProtocolError(Unsupported):
    pass


class NativeFailure(ProtocolError):
    def __init__(self, operation, report):
        super().__init__(operation + ': native failure; exact native roots retained')
        self.operation = operation
        self.report = report


def production_manager(*arguments, **keywords):
    raise Unsupported('Production native Child factory disabled before import/load/describe/clock/I/O')


def require_launch_ready(*arguments, **keywords):
    raise Unsupported('Native Child/bootstrap/lifecycle/installed ABI qualification is absent')


def _require(condition, message):
    if not condition:
        raise ProtocolError(message)


def _same_exit(a, b):
    return all(a[name] == b[name] for name in
               ('pid', 'pgid', 'uid', 'ppid', 'sid', 'state', 'exit_kind', 'exit_status'))


class NativeChildren:
    def __init__(self, *arguments, **keywords):
        raise Unsupported('Production NativeChildren construction is unconditionally disabled')

    @classmethod
    def _after_reviewed_bootstrap(cls):
        # Unconditional guard precedes even an import or module attribute lookup.
        # No caller-supplied module/backend is accepted as authentic.
        require_launch_ready()
        import _window_owned_child_bridge as native
        _require(cls is NativeChildren and type(native.CHILD_NATIVE_QUALIFIED) is int and
                 native.CHILD_NATIVE_QUALIFIED == 1, 'Independent Child qualification required')
        result = object.__new__(NativeChildren)
        result._native = native
        result._bridge = native.new_bridge()
        _require(type(result._bridge) is native.Bridge, 'Exact extension Bridge required')
        result._receipts = {}
        result._busy = False
        return result

    def _child(self, child):
        _require(type(child) is self._native.Child, 'Exact native-created Child required')
        return child

    def _snapshot(self, child):
        self._child(child)
        report = self._bridge.child_snapshot(child)
        _require(type(report) is dict and type(report['generation']) is int and
                 1 <= report['generation'] < 2 ** 64, 'Invalid native generation')
        return report

    def _receipt(self, child, report):
        receipt = self._receipts.get(child)
        if receipt is None:
            receipt = {'generation': report['generation'], 'child': child, 'terminal': None,
                       'anchor': None, 'resolved': False, 'error': None, 'last_report': report}
            self._receipts[child] = receipt
        _require(receipt['generation'] == report['generation'], 'Native generation changed')
        receipt['last_report'] = report
        return receipt

    def _registered_anchor(self, child, report, receipt):
        self._child(child)
        _require(receipt['child'] is child and report['generation'] == receipt['generation'] and
                 report['registered'] == report['pid_valid'] == report['spawn_entered'] ==
                 report['spawn_returned'] == 1 and report['spawn_rc'] == 0,
                 'Registered native-created Child provenance required')
        # Native registration accepted full BSD/SSTOP/creator identity. Its
        # immutable fields, not the last operation/member record, recover this
        # supplementary anchor. Exact Bridge roots remain the native authority.
        anchor = (report['generation'], report['registration_pid'], report['registration_pgid'],
                  report['registration_uid'], report['registration_ppid'], report['registration_sid'])
        generation, pid, pgid, uid, ppid, sid = anchor
        _require(all(type(value) is int for value in anchor) and 1 <= generation < 2 ** 64 and
                 pid > 1 and pid == report['spawn_pid_raw'] == pgid == sid and uid >= 0 and ppid > 1,
                 'Accepted registration/spawn identity mismatch')
        if receipt['anchor'] is None:
            receipt['anchor'] = anchor
        _require(receipt['anchor'] == anchor, 'Native registered identity changed')
        return receipt['anchor']

    def _call(self, child, operation, *args):
        self._child(child)
        report = getattr(self._bridge, operation)(child, *args)
        receipt = self._receipt(child, report)
        self._registered_anchor(child, report, receipt)
        # The native value counts admitted cleanup backend entries, including
        # begin/latch/reap/handoff. It does NOT count OS syscalls or guarantee their
        # latency. Launch has its own prepare/spawn/resume entries and API trace.
        _require(1 <= report['cleanup_entries'] <= report['max_cleanup_entries'] <= 512,
                 'Native cleanup backend-entry budget invalid')
        if report['rc']:
            if (operation == 'child_signal' and report['rc'] == 3 and
                    report['system_errno'] > 0 and report['signal_failed'] == 1 and
                    not report['blocked'] and
                    report['record']['signal_attempted'] == 1):
                receipt['error'] = 'Signal failed; native latch prohibits further signals'
            else:
                raise NativeFailure(operation, report)
        return report

    def create(self, *, executable, argv, env, cwd, stdio,
               launch_timeout_ns=1000000000, cleanup_timeout_ns=1000000000,
               grace_ns=100000000, max_calls=128):
        _require(not self._busy, 'Reentrant manager')
        try:
            self._busy = True
            _require(type(max_calls) is int and 1 <= max_calls <= 512,
                     'Cleanup backend-entry cap must remain within 1..512')
            # Native copies absolute executable/cwd, exact argv/env byte tuples
            # and duplicates all three explicit stdio FDs before posix_spawn.
            child = self._bridge.create(executable, argv, env, cwd, stdio,
                                        launch_timeout_ns, cleanup_timeout_ns, grace_ns, max_calls)
            self._child(child)
            report = self._snapshot(child)
            _require(report['rc'] == 0 and report['state'] == ACTIVE and
                     report['pid_valid'] == report['registered'] == report['published'] == 1 and
                     report['spawn_returned'] == report['resume_returned'] == 1 and
                     report['spawn_rc'] == report['resume_rc'] == 0 and
                     report['launch_entries'] == 3 and not report['resource_error'],
                     'Attempt is not registered/resumed native Child ownership')
            receipt = self._receipt(child, report)
            self._registered_anchor(child, report, receipt)
            return child
        finally:
            self._busy = False

    def pending(self):
        # Native root includes unpublished/failed/unknown attempts, not just
        # successful publications. This read performs no process probe or wait.
        child = self._bridge.pending()
        if child is not None:
            self._child(child)
            self._receipt(child, self._snapshot(child))
        return child

    def history(self):
        sinks = self._bridge.history()
        _require(type(sinks) is tuple and len(sinks) <= 64 and
                 all(type(sink) is self._native.TerminalSink for sink in sinks),
                 'Exact bounded native terminal history required')
        return sinks

    @staticmethod
    def _identity(anchor, record, target=None, *, copied=False):
        _, pid, pgid, uid, ppid, sid = anchor
        # Child-v1 wait UID is unavailable. record.uid is accepted BSD evidence;
        # terminal/copy evidence retains history, not a final-credential claim.
        _require(type(record) is dict and record['pid'] == (pid if target is None else target) and
                 (record['pgid'], record['uid'], record['ppid'], record['sid']) == (pgid, uid, ppid, sid) and
                 pid == pgid == sid and record['wait_uid_available'] == 0 and
                 record['uid_evidence'] == (UID_RETAINED if copied or record['state'] == EXITED
                                            else UID_CURRENT) and
                 (copied or record['state'] in (LIVE, EXITED)) and
                 record['sigchld_default'] == 1 and record['no_cldwait'] == 0 and
                 record['pid_bytes'] in (1, 2, 4, 8) and record['capacity'] == 2,
                 'Retained native identity/disposition mismatch')

    def _sample(self, child, anchor):
        pid = anchor[1]
        before = self._call(child, 'child_observe')['record']
        self._identity(anchor, before)
        members = self._call(child, 'child_members')['record']
        self._identity(anchor, members, copied=True)
        count = members['count']
        pids = (members['member0'], members['member1'])[:count]
        _require(count in (1, 2) and members['raw_bytes'] == count * members['pid_bytes'] and
                 len(set(pids)) == count and pid in pids and
                 all(type(pid) is int and pid > 1 for pid in pids) and
                 (count == 2 or members['member1'] == 0), 'Ambiguous/truncated group membership')
        after_report = self._call(child, 'child_observe')
        after = after_report['record']
        self._identity(anchor, after)
        _require(before['state'] in (LIVE, EXITED) and after['state'] in (LIVE, EXITED),
                 'Invalid native observation')
        stable = _same_exit(before, after)
        certificate = stable and after['state'] == EXITED and pids == (pid,)
        return after_report, pids, stable, certificate

    def _terminal_identity(self, child, anchor, observation, receipt, terminal, transferred):
        _require(terminal['generation'] == receipt['generation'] and terminal['rooted'] == 1 and
                 terminal['transferred'] == transferred and terminal['reap_attempted'] ==
                 terminal['wait_entered'] == terminal['wait_returned'] == terminal['consumed'] ==
                 terminal['decoded'] == terminal['certificate_matches'] == 1 and
                 terminal['wait_rc'] == 0 and terminal['wait_pid'] == anchor[1],
                 'Recorded native terminal certificate required')
        self._registered_anchor(child, terminal['final_report'], receipt)
        decoded = terminal['decoded_record']
        self._identity(anchor, decoded)
        _require(decoded['state'] == EXITED and decoded['no_signals'] == decoded['reap_attempted'] ==
                 decoded['reaped'] == 1 and _same_exit(observation, decoded),
                 'Native decoded terminal identity/certificate mismatch')

    def _finish(self, child, observation, receipt, report):
        # Mapping or assignment may fail after either irreversible native step.
        # Recover from native facts, never repeat latch/reap to recreate a reply.
        anchor = self._registered_anchor(child, report, receipt)
        self._identity(anchor, observation)
        _require(observation['state'] == EXITED, 'Native terminal observation required')
        if report['state'] == ACTIVE:
            if not report['latch_attempted']:
                report = self._call(child, 'child_latch', observation['exit_kind'], observation['exit_status'])
            self._identity(anchor, report['record'])
            _require(report['rc'] == 0 and not report['blocked'] and report['latch_attempted'] == 1 and
                     report['record']['state'] == EXITED and report['record']['no_signals'] == 1 and
                     report['record']['reap_attempted'] == 0,
                     'Native successful latch required before the single consuming reap')
            report = self._call(child, 'child_reap')
        reaped = report
        self._identity(anchor, reaped['record'])
        _require(reaped['rc'] == 0 and not reaped['blocked'] and
                 reaped['state'] == REAP_RECORDED and reaped['record']['reap_attempted'] == 1 and
                 reaped['record']['reaped'] == 1 and _same_exit(observation, reaped['record']),
                 'Native consuming reap changed certificate')
        sink = self._bridge.terminal(child)
        _require(type(sink) is self._native.TerminalSink, 'Exact native TerminalSink required')
        terminal = sink.snapshot()
        self._terminal_identity(child, anchor, observation, receipt, terminal, 0)
        receipt['terminal'] = sink
        transferred = self._bridge.handoff(child, sink)
        _require(transferred is sink, 'Concrete native terminal root transfer required')
        # Mapping can fail here; native history still retains the complete raw
        # wait and creation/cleanup facts. Old Child has already been invalidated.
        terminal = sink.snapshot()
        self._terminal_identity(child, anchor, observation, receipt, terminal, 1)
        receipt['terminal_report'] = terminal
        receipt['resolved'] = True

    def cleanup(self, child):
        self._child(child)
        _require(not self._busy, 'Reentrant manager')
        receipt = None
        try:
            self._busy = True
            report = self._snapshot(child)
            receipt = self._receipt(child, report)
            if report['state'] == ABORTED:
                _require(report['spawn_entered'] == 0 and report['resources_closed'] == 1 and
                         report['resource_error'] == 0, 'Abort resources remain unresolved')
                sink = self._bridge.terminal(child)
                _require(type(sink) is self._native.TerminalSink, 'Exact native abort sink required')
                _require(self._bridge.handoff(child, sink) is sink, 'Concrete abort root transfer required')
                receipt['terminal'] = sink
                terminal = sink.snapshot()
                _require(terminal['generation'] == receipt['generation'] and
                         terminal['known_no_creation'] == terminal['transferred'] == 1 and
                         terminal['wait_entered'] == 0, 'Never-entered abort certificate mismatch')
                receipt['terminal_report'] = terminal
                receipt['resolved'] = True
                receipt['error'] = 'Creation aborted before spawn; no child ownership claimed'
                return receipt
            _require(report['state'] in (ACTIVE, REAP_RECORDED) and
                     report['registered'] == report['pid_valid'] == 1 and
                     not report['blocked'], 'Unknown/failed creation remains retained for reconciliation')
            anchor = self._registered_anchor(child, report, receipt)
            if report['state'] == REAP_RECORDED or report['latch_attempted']:
                _require(report['cleanup_started'] == 1,
                         'Retained cleanup phase must keep its original deadline and count')
                self._finish(child, report['record'], receipt, report)
                return receipt
            if not report['cleanup_started']:
                self._call(child, 'cleanup_begin')
            # Re-entry reuses the native deadline/total entry count; no reset.
            while True:
                report, pids, stable, certificate = self._sample(child, anchor)
                if certificate:
                    self._finish(child, report['record'], receipt, report)
                    return receipt
                if not stable or report['signal_failed']:
                    continue
                kind = None
                if not report['term_attempted']:
                    kind = TERM
                elif not report['kill_attempted'] and report['cleanup_now_ns'] >= report['cleanup_kill_at_ns']:
                    kind = KILL
                if kind is not None:
                    positive = 0
                    if report['record']['state'] == EXITED:
                        _require(len(pids) == 2, 'Exited anchor needs a unique positive live member')
                        positive = next(pid for pid in pids if pid != anchor[1])
                        member = self._call(child, 'child_member', positive)['record']
                        self._identity(anchor, member, positive)
                        _require(member['state'] == LIVE, 'Positive member not live')
                    self._call(child, 'child_signal', kind, positive)
        except BaseException as error:
            if receipt is not None:
                receipt['error'] = type(error).__name__
            raise
        finally:
            self._busy = False
