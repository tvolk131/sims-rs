"""SOURCE-ONLY owned-child dependency; not selected by any current builder.

C/ABI/platform: UNCOMPILED/UNRUN. Import performs no native load, platform probe,
process inspection, clock read, or launch. Production is unconditionally disabled;
there is no environment/CLI switch and no loader. Injected backends and clocks
below describe authored mocks only, not an executable production adapter.

Public source basis (not identification of this machine or its running kernel):
https://github.com/apple-oss-distributions/xnu/blob/e3723e1f17661b24996789d8afc084c0c3303b26/bsd/kern/proc_info.c
https://github.com/apple-oss-distributions/xnu/blob/e3723e1f17661b24996789d8afc084c0c3303b26/bsd/kern/kern_exit.c
https://github.com/python/cpython/blob/db3ff76da19004f266b62e98a81bdfd322861436/Modules/posixmodule.c

The proposed certificate is ONLY one retained exited direct-child anchor in a
two-pid buffer, bracketed by equal WNOWAIT exit observations. The pinned internal
nprocs+20 capacity can truncate larger lists. Full, empty, partial, missing-anchor,
multiple-zombie or unknown samples never prove absence. Positive member checks
are not negative completeness evidence. Native si_uid is unavailable here and
must not be compared to getuid; UID policy instead uses live registration and an
explicit stable-credentials assumption. Nonstandard/high-bit exit statuses fail
closed, including changes between pinned-kernel WNOWAIT observations.

Trust assumptions: reviewed OS process publication; a freshly created session
with PID=PGID=SID; an exclusive direct-child waiter; no uncoordinated native
SIGCHLD changes; a trusted workload that never escapes/reenters/invites unrelated
members and never changes credentials. Groups are NOT containment. Native
sigaction, not Python's cached getsignal, must still be checked on each operation.
Positive checks and signals are not atomic; a member leaving/exiting between
checks can invalidate evidence. The future platform review must examine these
races; this package confers no support/qualification on any OS.

One absolute cleanup deadline and one finite total backend-call budget persist
across TERM/grace/KILL/certificate/reap and repeated cleanup calls. Kernel calls
can stall despite WNOHANG; this is not an unconditional whole-call wall-time bound.
Grace is an offset from cleanup start, never a fresh post-TERM time allowance.
There are no sleeps or auxiliary processes. Signal errors remain historical
failures even if a later certificate permits final reap. Compiler37394/EPERM is
still failed, not waived by this source-only candidate.

After a group-signal syscall error, no further signals are attempted. Only a
well-formed native signal-attempt receipt with preserved owned identity can
continue sampling for terminal evidence. Earlier native ownership/ABI errors or
ambiguous adapter exceptions abort; they cannot be relabeled signal errors.

Future prerequisites, ALL separately parent-approved: independent source review;
trusted bootstrap compiler/artifact provenance (not the unavailable builder);
compiled SDK/ABI/platform validation; reviewed process-publication/race witnesses;
a real in-process extension mapping this PUBLIC header ABI (no private ctypes
layouts), preserving return/error fields and strong opaque owner handles; and a
separate builder/group adapter with authentic CPython Popen identity, waiter-lock,
cached-status/destructor and exclusive-wait validation. Hash consistency is not
mapped-image proof. A future loader must check the still-disabled gate BEFORE any
load/probe. No app, alpha, build, native or runtime-backend qualification follows.

Mock adapter protocol: describe() -> exact description mapping; open(pid,pgid,uid)
-> (strong opaque owner, exact record mapping); observe/members/member/signal/
latch/reap use that owner. Integer enums below are OUR header ABI, never guessed
Darwin flags or private kernel layouts. Adapter must check C function return code
as well as header error before constructing a mapping, especially short buffers.
The native owner remains retained on every unresolved or publication-failure
path; only a fully reconciled adapter may release its already-reaped C allocation.
"""


ABI_VERSION = 1
DESCRIPTION_BYTES = 32
RECORD_BYTES = 200
LIVE = 1
EXITED = 2
NORMAL = 1
SIGNALED = 2
COREDUMP = 3
UID_CURRENT = 1
UID_RETAINED = 2
TERM = 1
KILL = 2
QUALIFICATIONS = {
    'build': False, 'native': False, 'abi': False, 'platform': False,
    'runtime_backend': False, 'app': False, 'alpha': False,
    'containment': False, 'running_kernel_identified': False,
    'compiler37394_failure_resolved': False,
}
ASSUMPTIONS = (
    'fresh_session', 'exclusive_waiter', 'stable_credentials',
    'reviewed_publication', 'reviewed_platform', 'stable_native_sigchld',
    'trusted_no_escape_reentry_or_unrelated_members',
)
HEADER_FIELDS = ('bytes', 'version', 'error', 'system_errno')
RECORD_FIELDS = (
    'pid', 'pgid', 'uid', 'ppid', 'sid', 'state', 'exit_kind', 'exit_status',
    'wait_uid_available', 'uid_evidence', 'sigchld_default', 'no_cldwait',
    'raw_bytes', 'pid_bytes', 'capacity', 'count', 'member0', 'member1',
    'no_signals', 'reap_attempted', 'reaped', 'returncode', 'signal_attempted',
)
DESCRIPTION_FIELDS = ('record_bytes', 'pid_bytes', 'platform', 'enabled')
RETAINED_HANDLES = {}


class Unsupported(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise Unsupported(message)


def integer(value, minimum, maximum, name):
    require(type(value) is int and minimum <= value <= maximum,
            'Invalid strict integer: ' + name)
    return value


def seconds(value, name, positive=False):
    require(type(value) in (int, float) and value == value and
            -float('inf') < value < float('inf'), 'Invalid finite clock/bound: ' + name)
    require(value > 0 if positive else value >= 0, 'Invalid time bound: ' + name)
    return value


def production_backend():
    raise Unsupported('Production backend hard-disabled; bootstrap/adapter/native approval absent')


def require_launch_ready():
    raise Unsupported('Launch forbidden: source-only dependency, no production backend')


def checked_record(value, operation, description=False):
    fields = DESCRIPTION_FIELDS if description else RECORD_FIELDS
    require(type(value) is dict and set(value) == set(HEADER_FIELDS + fields),
            operation + ': invalid exact ABI fields')
    value = dict(value)
    for name in HEADER_FIELDS + fields:
        integer(value[name], -(2 ** 63), 2 ** 63 - 1, name)
    require(value['bytes'] == (DESCRIPTION_BYTES if description else RECORD_BYTES) and
            value['version'] == ABI_VERSION, operation + ': ABI size/version mismatch')
    integer(value['error'], 0, 5, 'error')
    integer(value['system_errno'], 0, 2 ** 31 - 1, 'system_errno')
    require(value['error'] == 3 or value['system_errno'] == 0,
            operation + ': malformed error/errno fields')
    require(value['error'] == 0,
            operation + ': backend error=' + str(value['error']) +
            ' errno=' + str(value['system_errno']))
    return value


class RetainedChild:
    def __init__(self, process, manager):
        self.process = process
        self.manager = manager
        self.token = object()
        self.owner = None
        self.lock = None
        self.locked = False
        self.pid = None
        self.uid = None
        self.parent = None
        self.registered = False
        self.blocked = False
        self.no_signals = False
        self.reap_attempted = False
        self.resolved = False
        self.deadline = None
        self.kill_at = None
        self.last_clock = None
        self.calls = 0
        self.term_attempted = False
        self.kill_attempted = False
        self.signal_failed = False
        self.receipt = {
            'registration': [], 'ownership': [], 'observations': [], 'signals': [],
            'errors': [], 'reap': None, 'publication': None, 'retained': True,
            'reaped': False, 'resolved': False, 'success': False,
            'no_further_group_signals': False, 'backend_calls': 0,
            'deadline': None, 'qualifications': dict(QUALIFICATIONS),
        }


class OwnedChildren:
    """Injected authored-response policy only; never launches or loads a backend."""

    def __init__(self, backend=None, clock=None, timeout=2.0, grace=0.5, max_calls=128):
        self.backend = backend
        self.clock = clock
        self.timeout = seconds(timeout, 'timeout', positive=True)
        self.grace = seconds(grace, 'grace')
        require(self.grace <= self.timeout <= 60, 'Invalid single cleanup window')
        self.max_calls = integer(max_calls, 1, 512, 'max_calls')
        self.handles = {}
        self.pid_bytes = None

    def assert_registration_available(self):
        require(not RETAINED_HANDLES, 'Unresolved strong ownership handle blocks new launch/registration')
        require(self.backend is not None and callable(self.clock),
                'Missing injected backend/clock; launch remains forbidden')

    def register_owned(self, process, uid, assumptions):
        self.assert_registration_available()
        child = RetainedChild(process, self)
        RETAINED_HANDLES[child.token] = child
        self.handles[child.token] = child
        try:
            require(type(assumptions) is dict and set(assumptions) == set(ASSUMPTIONS) and
                    all(value is True for value in assumptions.values()),
                    'All ownership assumptions must be explicitly acknowledged')
            child.pid = integer(process.pid, 2, 2 ** 31 - 1, 'owned pid')
            child.uid = integer(uid, 0, 2 ** 32 - 1, 'UID policy')
            require(process.returncode is None, 'Cached Popen status is not ownership proof')
            child.lock = process._waitpid_lock
            require(child.lock.acquire(False) is True, 'Competing/unknown Popen waiter')
            child.locked = True
            description = checked_record(self.backend.describe(), 'describe', description=True)
            require(description['record_bytes'] == RECORD_BYTES and description['platform'] == 1 and
                    description['enabled'] == 0 and description['pid_bytes'] in (1, 2, 4, 8),
                    'Unsupported source-only platform/ABI descriptor')
            self.pid_bytes = description['pid_bytes']
            opened = self.backend.open(child.pid, child.pid, child.uid)
            require(type(opened) is tuple and len(opened) == 2, 'Invalid owner registration tuple')
            child.owner = opened[0]
            require(child.owner is not None and not isinstance(child.owner, (
                int, float, complex, str, bytes, bytearray, list, tuple, dict, set, frozenset,
            )), 'Missing strong opaque native owner; numeric PID/pointer is insufficient')
            record = checked_record(opened[1], 'open')
            child.parent = integer(record['ppid'], 1, 2 ** 31 - 1, 'direct parent')
            self._identity(child, record)
            require(record['state'] == LIVE and record['uid_evidence'] == UID_CURRENT,
                    'Registration requires positive live direct-child UID/group/session evidence')
            child.receipt['registration'].append(record)
            child.registered = True
        except BaseException as error:
            child.receipt['registration'].append({'error': type(error).__name__})
            child.receipt['errors'].append('registration: ' + self._describe(error))
        return child

    @staticmethod
    def _describe(error):
        try:
            return type(error).__name__ + ': ' + str(error)
        except BaseException:
            return type(error).__name__ + ': unprintable exception'

    def _identity(self, child, record, terminal=False, signal=False):
        require(child.process.returncode is None and child.locked,
                'Cached status/competing wait owner cannot prove identity')
        require(record['pid'] == child.pid and record['pgid'] == child.pid and
                record['uid'] == child.uid and record['ppid'] == child.parent and
                record['sid'] == child.pid, 'Owned child PID/group/UID/parent/session mismatch')
        require(record['sigchld_default'] == 1 and record['no_cldwait'] == 0,
                'Native SIGCHLD disposition/SA_NOCLDWAIT unsupported')
        require(record['wait_uid_available'] == 0,
                'Unexpected waitid UID capability; no si_uid equality assumption')
        require(record['pid_bytes'] == self.pid_bytes and record['capacity'] == 2,
                'Native capacity/PID ABI mismatch')
        require(record['no_signals'] == int(child.no_signals) and
                record['reap_attempted'] == int(child.reap_attempted) and
                record['reaped'] == int(terminal) and record['signal_attempted'] == int(signal),
                'Native owner/latch/reap/signal state mismatch')
        if record['state'] == LIVE:
            require(not terminal and record['uid_evidence'] == UID_CURRENT and
                    record['exit_kind'] == 0 and record['exit_status'] == 0 and
                    record['returncode'] == 0, 'Invalid live observation')
        else:
            require(record['state'] == EXITED and record['uid_evidence'] == UID_RETAINED,
                    'Unknown child state/UID provenance')
            self._exit_code(record)

    @staticmethod
    def _exit_code(record):
        kind, status = record['exit_kind'], record['exit_status']
        require((kind == NORMAL and 0 <= status <= 255) or
                (kind in (SIGNALED, COREDUMP) and 1 <= status <= 127),
                'Unknown or nonstandard native exit status')
        code = status if kind == NORMAL else -status
        require(record['returncode'] == code, 'Native decoded returncode mismatch')
        return code

    def _now(self, child):
        now = seconds(self.clock(), 'injected clock')
        require(child.last_clock is None or now >= child.last_clock, 'Clock moved backwards')
        child.last_clock = now
        return now

    def _call(self, child, operation, *arguments):
        require(child.calls < self.max_calls, 'Single total backend-call budget exhausted')
        require(self._now(child) < child.deadline, 'Single absolute cleanup deadline exhausted')
        child.calls += 1
        child.receipt['backend_calls'] = child.calls
        item = {'operation': operation, 'raw': None, 'record': None, 'error': None}
        child.receipt['observations'].append(item)
        try:
            item['raw'] = getattr(self.backend, operation)(child.owner, *arguments)
            record = checked_record(item['raw'], operation)
            item['record'] = record
            return record
        except BaseException as error:
            item['error'] = self._describe(error)
            raise

    def _observe(self, child):
        record = self._call(child, 'observe')
        self._identity(child, record)
        child.receipt['ownership'].append(record)
        return record

    @staticmethod
    def _same(before, after):
        fields = ('pid', 'pgid', 'uid', 'ppid', 'sid', 'state', 'exit_kind',
                  'exit_status', 'wait_uid_available', 'uid_evidence', 'returncode')
        return all(before[name] == after[name] for name in fields)

    def _sample(self, child):
        before = self._observe(child)
        members = self._call(child, 'members')
        require(members['pid'] == child.pid and members['pgid'] == child.pid and
                members['uid'] == child.uid and members['ppid'] == child.parent and
                members['sid'] == child.pid and members['wait_uid_available'] == 0 and
                members['uid_evidence'] == UID_RETAINED and members['sigchld_default'] == 1 and
                members['no_cldwait'] == 0 and members['no_signals'] == 0 and
                members['reap_attempted'] == 0 and members['reaped'] == 0 and
                members['signal_attempted'] == 0 and
                members['pid_bytes'] == self.pid_bytes and members['capacity'] == 2,
                'Membership identity/disposition/ABI mismatch')
        raw = members['raw_bytes']
        require(0 < raw <= 2 * self.pid_bytes and raw % self.pid_bytes == 0 and
                members['count'] == raw // self.pid_bytes, 'Empty/error/partial/oversized membership')
        count = members['count']
        pids = (members['member0'], members['member1'])[:count]
        require(all(2 <= pid <= 2 ** 31 - 1 for pid in pids) and len(set(pids)) == count and
                child.pid in pids and (count == 2 or members['member1'] == 0),
                'Invalid/missing retained membership anchor')
        after = self._observe(child)
        require(self._same(before, after), 'WNOWAIT identity changed across membership')
        certificate = (before['state'] == EXITED and count == 1 and
                       raw == self.pid_bytes and pids == (child.pid,))
        return after, pids, certificate

    def _send(self, child, kind, pids):
        require(not child.no_signals and not child.reap_attempted,
                'No group signal permitted after latch/reap attempt')
        before = self._observe(child)
        positive = 0
        if before['state'] == EXITED:
            candidates = [pid for pid in pids if pid != child.pid]
            require(len(candidates) == 1, 'Exited leader has no positive live-member evidence')
            positive = candidates[0]
            member = self._call(child, 'member', positive)
            require(member['pid'] == positive and member['pgid'] == child.pid and
                    member['uid'] == child.uid and member['sid'] == child.pid and
                    member['state'] == LIVE and member['uid_evidence'] == UID_CURRENT and
                    member['wait_uid_available'] == 0 and member['pid_bytes'] == self.pid_bytes and
                    member['capacity'] == 2 and member['exit_kind'] == 0 and
                    member['exit_status'] == 0 and member['returncode'] == 0 and
                    member['sigchld_default'] == 1 and member['no_cldwait'] == 0 and
                    member['no_signals'] == 0 and member['reap_attempted'] == 0 and
                    member['reaped'] == 0, 'No current positive same-live-group member')
            require(member['signal_attempted'] == 0, 'Invalid positive-member signal receipt')
            after = self._observe(child)
            require(self._same(before, after), 'Leader identity raced positive member check')
        entry = {'kind': kind, 'pgid': child.pid, 'positive_member': positive, 'error': None}
        child.receipt['signals'].append(entry)
        try:
            record = self._call(child, 'signal', kind, positive)
            self._identity(child, record, signal=True)
            require(self._same(before, record), 'Signal reply changed owned identity')
        except BaseException as error:
            entry['error'] = self._describe(error)
            child.receipt['errors'].append('signal: ' + entry['error'])
            child.signal_failed = True
            raw = child.receipt['observations'][-1]['raw']
            if isinstance(error, Exception) and type(raw) is dict:
                integer(raw['error'], 0, 5, 'signal error')
                integer(raw['system_errno'], 0, 2 ** 31 - 1, 'signal errno')
                normalized = dict(raw)
                normalized['error'] = 0
                normalized['system_errno'] = 0
                checked_record(normalized, 'failed signal ownership')
                self._identity(child, normalized, signal=True)
                require(self._same(before, normalized), 'Failed signal lost owned identity')
                require(raw['error'] == 3 and raw['system_errno'] > 0,
                        'Non-signal ownership/ABI error remains fail-closed')
            else:
                raise

    def _finish(self, child, observation):
        require(observation['state'] == EXITED and not child.reap_attempted,
                'Final reap requires retained exited identity')
        child.no_signals = True
        child.receipt['no_further_group_signals'] = True
        latched = self._call(child, 'latch', observation['exit_kind'], observation['exit_status'])
        self._identity(child, latched)
        require(self._same(observation, latched), 'Native latch certificate changed')
        child.reap_attempted = True
        result = self._call(child, 'reap')
        child.receipt['reap'] = result
        self._identity(child, result, terminal=True)
        child.receipt['reaped'] = True
        require(self._same(observation, result), 'Native final reap changed retained exit status')
        code = self._exit_code(result)
        child.receipt['publication'] = {'returncode': code, 'error': None}
        child.process.returncode = code
        require(type(child.process.returncode) is int and child.process.returncode == code,
                'Popen returncode publication failed')
        child.lock.release()
        child.locked = False
        child.resolved = True
        child.receipt['resolved'] = True
        child.receipt['retained'] = False
        RETAINED_HANDLES.pop(child.token, None)
        self.handles.pop(child.token, None)

    def cleanup(self, child):
        require(type(child) is RetainedChild and child.manager is self,
                'Foreign or missing retained ownership handle')
        if child.resolved or child.blocked or child.no_signals or not child.registered:
            return child.receipt
        try:
            if child.deadline is None:
                start = self._now(child)
                child.deadline = start + self.timeout
                child.kill_at = start + self.grace
                child.receipt['deadline'] = child.deadline
            while True:
                observation, pids, certificate = self._sample(child)
                if certificate:
                    self._finish(child, observation)
                    break
                if child.signal_failed:
                    continue
                if not child.term_attempted:
                    child.term_attempted = True
                    self._send(child, TERM, pids)
                elif not child.kill_attempted and self._now(child) >= child.kill_at:
                    child.kill_attempted = True
                    self._send(child, KILL, pids)
        except BaseException as error:
            child.blocked = True
            message = self._describe(error)
            child.receipt['errors'].append(message)
            if child.receipt['publication'] is not None:
                child.receipt['publication']['error'] = message
        child.receipt['success'] = child.resolved and not child.receipt['errors']
        return child.receipt
