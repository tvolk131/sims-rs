"""Standalone stdlib authored mocks, never a native/platform validation run.

Run with the existing interpreter: python3 -B scripts/test_window_owned_child.py
The guard is installed BEFORE importing the tested policy; no subprocess, native
binding, real clock/sleep, wait, signal, census or network operation is permitted.
The runner itself imports only sys and the pure-Python policy, not unittest's
transitive dependencies. Audit self-tests emit forbidden events, never operations.
"""

import sys


def forbidden(*arguments, **keywords):
    raise RuntimeError('Forbidden real operation in authored mocks')


def audit_guard(event, arguments):
    denied = (
        'subprocess.', 'ctypes.', 'socket.', 'urllib.', 'http.', 'multiprocessing.',
        'os.fork', 'os.posix_spawn', 'os.exec', 'os.spawn', 'os.system', 'os.kill',
        'os.wait', 'os.getpgid', 'os.getsid', 'os.getpid', 'os.getppid', 'os.getuid',
        'os.geteuid', 'os.sysconf', 'time.sleep', 'signal.', 'resource.',
        'os.chown', 'os.chmod', 'os.setuid', 'os.setgid', 'os.getpriority',
    )
    if event.startswith(denied):
        forbidden()
    if event == 'import':
        name = arguments[0]
        filename = arguments[1] if len(arguments) > 1 else None
        if name != 'window_owned_child' or (filename and not filename.endswith('.py')):
            forbidden()
    if event == 'open':
        path = arguments[0]
        if type(path) is str and (path.startswith(('/proc/', '/dev/')) or
                                 path.endswith(('.so', '.dylib', '.dll'))):
            forbidden()
    if event in ('os.listdir', 'os.scandir'):
        path = arguments[0]
        if type(path) is not str or path == '/proc' or path.startswith(('/proc/', '/dev/')):
            forbidden()


def install_guard():
    sys.dont_write_bytecode = True
    sys.addaudithook(audit_guard)
    names = (
        'fork', 'forkpty', 'posix_spawn', 'posix_spawnp', 'system', 'kill', 'killpg',
        'wait', 'wait3', 'wait4', 'waitid', 'waitpid', 'getpgid', 'getpgrp', 'getsid',
        'getpid', 'getppid', 'getuid', 'geteuid', 'getgid', 'getegid', 'getgroups',
        'sysconf', 'sched_getaffinity', 'execv', 'execve', 'execvp', 'execvpe',
        'getresuid', 'getresgid', 'getpriority', 'setpriority', 'nice', 'access',
    )
    for module_name in ('posix', 'os'):
        module = sys.modules.get(module_name)
        if module is not None:
            for name in names:
                if hasattr(module, name):
                    setattr(module, name, forbidden)
    for module_name, names in (
        ('time', ('sleep', 'monotonic', 'monotonic_ns', 'perf_counter', 'time', 'time_ns')),
        ('_signal', ('signal', 'getsignal', 'pthread_kill', 'sigwait', 'sigwaitinfo',
                     'sigtimedwait', 'raise_signal', 'alarm', 'setitimer')),
    ):
        module = sys.modules.get(module_name)
        if module is not None:
            for name in names:
                if hasattr(module, name):
                    setattr(module, name, forbidden)
    for module_name in ('subprocess', 'ctypes', '_ctypes', 'socket', '_socket',
                        'multiprocessing', 'resource', 'signal'):
        if module_name in sys.modules:
            forbidden()


def check(condition, message='authored assertion failed'):
    if not condition:
        raise AssertionError(message)


def raises(exception_type, function, *arguments, **keywords):
    try:
        function(*arguments, **keywords)
    except exception_type:
        return
    raise AssertionError('Expected ' + exception_type.__name__)


CASES = []


def case(function):
    CASES.append(function)
    return function


class FakeClock:
    def __init__(self, step=0.001):
        self.now = 0.0
        self.step = step
        self.values = []
        self.reads = 0

    def __call__(self):
        self.reads += 1
        if self.values:
            return self.values.pop(0)
        value = self.now
        self.now += self.step
        return value

    sleep = forbidden


class FakeLock:
    def __init__(self):
        self.held = False
        self.busy = False
        self.requests = []
        self.release_error = None

    def acquire(self, blocking):
        self.requests.append(blocking)
        check(blocking is False, 'waiter lock acquisition must never block')
        if self.busy or self.held:
            return False
        self.held = True
        return True

    def release(self):
        if self.release_error is not None:
            raise self.release_error
        check(self.held)
        self.held = False


class FakeProcess:
    def __init__(self):
        self.pid = 42
        self._returncode = None
        self._waitpid_lock = FakeLock()
        self.publication_error = None

    @property
    def returncode(self):
        return self._returncode

    @returncode.setter
    def returncode(self, value):
        if self.publication_error is not None:
            raise self.publication_error
        self._returncode = value

    poll = forbidden
    wait = forbidden
    terminate = forbidden
    kill = forbidden
    send_signal = forbidden


def record(**updates):
    value = {name: 0 for name in policy.HEADER_FIELDS + policy.RECORD_FIELDS}
    value.update(bytes=200, version=1, pid=42, pgid=42, uid=501, ppid=7, sid=42,
                 state=policy.LIVE, uid_evidence=policy.UID_CURRENT,
                 sigchld_default=1, pid_bytes=4, capacity=2)
    value.update(updates)
    return value


def exited(**updates):
    value = record(state=policy.EXITED, exit_kind=policy.NORMAL,
                   uid_evidence=policy.UID_RETAINED)
    value.update(updates)
    return value


class FakeBackend:
    def __init__(self):
        self.owner = object()
        self.events = []
        self.scripts = {}
        self.leader = record()
        self.pids = [42]
        self.member_state = policy.LIVE
        self.no_signals = False
        self.attempted = False
        self.reaped = False
        self.on_signal = None
        self.on_open = None
        self.delivered = []

    def take(self, name, default, *arguments):
        self.events.append((name, arguments))
        items = self.scripts.get(name, [])
        value = items.pop(0) if items else default
        if isinstance(value, BaseException):
            raise value
        if callable(value):
            return value(*arguments)
        return value

    def state(self):
        value = dict(self.leader)
        value.update(no_signals=int(self.no_signals), reap_attempted=int(self.attempted),
                     reaped=int(self.reaped))
        return value

    def owned(self, owner):
        check(owner is self.owner, 'PID is not an independent native owner handle')

    def describe(self):
        return self.take('describe', {'bytes': 32, 'version': 1, 'error': 0,
                                     'system_errno': 0, 'record_bytes': 200,
                                     'pid_bytes': 4, 'platform': 1, 'enabled': 0})

    def open(self, pid, pgid, uid):
        check((pid, pgid, uid) == (42, 42, 501))
        result = self.take('open', (self.owner, record()))
        if self.on_open:
            self.on_open()
        return result

    def observe(self, owner):
        self.owned(owner)
        return self.take('observe', self.state())

    def members(self, owner):
        self.owned(owner)
        value = record(state=0, uid_evidence=policy.UID_RETAINED,
                       raw_bytes=4 * len(self.pids), count=len(self.pids),
                       member0=self.pids[0] if self.pids else 0,
                       member1=self.pids[1] if len(self.pids) > 1 else 0)
        return self.take('members', value)

    def member(self, owner, pid):
        self.owned(owner)
        return self.take('member', record(pid=pid, state=self.member_state), pid)

    def signal(self, owner, kind, positive):
        self.owned(owner)
        check(not self.no_signals and not self.attempted and not self.reaped)
        value = self.state()
        value['signal_attempted'] = 1
        value = self.take('signal', value, kind, positive)
        if value['error'] == 0:
            self.delivered.append((kind, 42, positive))
        if self.on_signal:
            self.on_signal()
        return value

    def latch(self, owner, kind, status):
        self.owned(owner)
        check(not self.no_signals and not self.attempted)
        self.no_signals = True
        return self.take('latch', self.state(), kind, status)

    def reap(self, owner):
        self.owned(owner)
        check(self.no_signals and not self.attempted)
        self.attempted = True
        self.reaped = True
        return self.take('reap', self.state())


def fixture(timeout=2, grace=0, max_calls=96, clock=None, backend=None, process=None):
    backend = backend if backend is not None else FakeBackend()
    clock = clock if clock is not None else FakeClock()
    process = process if process is not None else FakeProcess()
    manager = policy.OwnedChildren(backend, clock, timeout=timeout, grace=grace,
                                   max_calls=max_calls)
    assumptions = dict.fromkeys(policy.ASSUMPTIONS, True)
    child = manager.register_owned(process, 501, assumptions)
    return manager, child, backend, clock, process


def terminal_fixture(**keywords):
    result = fixture(**keywords)
    result[2].leader = exited()
    return result


def operations(backend, name):
    return [item for item in backend.events if item[0] == name]


def retained(child):
    check(child.token in policy.RETAINED_HANDLES and child.receipt['retained'])
    check(not child.receipt['success'])


@case
def guard_rejects_real_operations_before_policy_use():
    for event in ('subprocess.Popen', 'ctypes.dlopen', 'socket.connect', 'os.kill',
                  'os.waitpid', 'os.getpgid', 'os.getpid', 'time.sleep', 'signal.signal'):
        raises(RuntimeError, sys.audit, event, ())
    raises(RuntimeError, sys.audit, 'os.listdir', '/proc')
    raises(RuntimeError, sys.audit, 'os.scandir', '/proc/42')
    for name in ('ctypes', '_ctypes', 'subprocess', 'socket', 'signal', 'math'):
        raises(RuntimeError, __import__, name)
    for module_name, name in (('posix', 'getpid'), ('posix', 'waitpid'),
                              ('posix', 'killpg'), ('time', 'sleep'), ('time', 'monotonic')):
        module = sys.modules.get(module_name)
        if module is not None and hasattr(module, name):
            raises(RuntimeError, getattr(module, name))


@case
def import_is_pure_and_production_gate_is_unconditional():
    check(not any(policy.QUALIFICATIONS.values()))
    raises(policy.Unsupported, policy.production_backend)
    raises(policy.Unsupported, policy.require_launch_ready)
    check(not policy.RETAINED_HANDLES)


@case
def missing_backend_prevents_registration_and_launch():
    manager = policy.OwnedChildren()
    raises(policy.Unsupported, manager.assert_registration_available)
    raises(policy.Unsupported, manager.register_owned, FakeProcess(), 501, {})
    check(not policy.RETAINED_HANDLES)


@case
def strict_configuration_scalars():
    for field, values in (
        ('timeout', (True, 0, -1, '2', float('nan'), float('inf'), 61)),
        ('grace', (False, -1, '0', float('nan'), float('inf'), 3)),
        ('max_calls', (True, 0, -1, 1.0, '2', 513)),
    ):
        for value in values:
            raises(policy.Unsupported, policy.OwnedChildren, **{field: value})


@case
def strict_abi_headers_and_payload_scalars():
    for name in policy.HEADER_FIELDS + policy.RECORD_FIELDS:
        for value in (True, False, 1.0, '1', None, 2 ** 64):
            altered = record(**{name: value})
            raises(policy.Unsupported, policy.checked_record, altered, 'authored')
    for value in ([], (), None, dict(record(), extra=0)):
        raises(policy.Unsupported, policy.checked_record, value, 'authored')
    for updates in ({'bytes': 199}, {'bytes': 201}, {'version': 2}, {'error': 6},
                    {'error': -1}, {'system_errno': -1}, {'system_errno': 1},
                    {'error': 3, 'system_errno': 1}):
        raises(policy.Unsupported, policy.checked_record, record(**updates), 'authored')


@case
def description_and_missing_artifact_refusals():
    for updates in ({'bytes': 31}, {'version': 2}, {'record_bytes': 191},
                    {'pid_bytes': 3}, {'platform': 0}, {'platform': True},
                    {'enabled': 1}, {'error': 2}):
        policy.RETAINED_HANDLES.clear()
        backend = FakeBackend()
        description = backend.describe()
        description.update(updates)
        backend.scripts['describe'] = [description]
        manager, child, backend, clock, process = fixture(backend=backend)
        check(not child.registered)
        retained(child)
        check(not operations(backend, 'open'))
    policy.RETAINED_HANDLES.clear()
    backend = FakeBackend()
    backend.scripts['describe'] = [OSError('authored missing artifact')]
    manager, child, backend, clock, process = fixture(backend=backend)
    retained(child)
    check(not child.registered)


@case
def native_owner_must_be_strong_opaque_handle_not_numeric_pid_or_container():
    for owner in (None, True, 42, 42.0, '42', b'42', [], (), {}, set()):
        policy.RETAINED_HANDLES.clear()
        backend = FakeBackend()
        backend.scripts['open'] = [(owner, record())]
        manager, child, backend, clock, process = fixture(backend=backend)
        retained(child)
        check(not child.registered and not operations(backend, 'observe'))


@case
def explicit_assumptions_are_required_not_qualifications():
    for missing in policy.ASSUMPTIONS:
        policy.RETAINED_HANDLES.clear()
        manager = policy.OwnedChildren(FakeBackend(), FakeClock())
        assumptions = dict.fromkeys(policy.ASSUMPTIONS, True)
        assumptions[missing] = 1
        child = manager.register_owned(FakeProcess(), 501, assumptions)
        retained(child)
        check(not child.registered)


@case
def pid_and_uid_are_strict_and_failed_registration_retains_handle():
    for pid in (True, 0, 1, -42, 42.0, '42', 2 ** 31):
        policy.RETAINED_HANDLES.clear()
        process = FakeProcess()
        process.pid = pid
        manager, child, backend, clock, process = fixture(process=process)
        retained(child)
        check(not child.registered)
        raises(policy.Unsupported, manager.assert_registration_available)
    for uid in (True, -1, 501.0, '501', 2 ** 32):
        policy.RETAINED_HANDLES.clear()
        manager = policy.OwnedChildren(FakeBackend(), FakeClock())
        child = manager.register_owned(FakeProcess(), uid, dict.fromkeys(policy.ASSUMPTIONS, True))
        retained(child)


@case
def unknown_native_sigchld_and_no_cldwait_refuse():
    for updates in ({'sigchld_default': 0}, {'sigchld_default': -1}, {'no_cldwait': 1}):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = fixture()
        backend.leader.update(updates)
        manager.cleanup(child)
        retained(child)
        check(not operations(backend, 'signal') and not operations(backend, 'reap'))


@case
def direct_child_uid_group_parent_session_mismatch_refuse():
    for updates in ({'pid': 43}, {'pgid': 99}, {'uid': 502}, {'ppid': 8}, {'sid': 99},
                    {'state': 7}, {'uid_evidence': policy.UID_RETAINED}):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = fixture()
        backend.leader.update(updates)
        manager.cleanup(child)
        retained(child)
        check(not operations(backend, 'signal'))


@case
def waitid_uid_unavailable_does_not_require_current_uid_equality():
    manager, child, backend, clock, process = terminal_fixture()
    check(backend.leader['wait_uid_available'] == 0 and backend.leader['uid'] == 501)
    check(manager.cleanup(child)['success'])
    check(process.returncode == 0)


@case
def unexpected_waitid_uid_capability_refuses():
    manager, child, backend, clock, process = terminal_fixture()
    backend.leader['wait_uid_available'] = 1
    manager.cleanup(child)
    retained(child)


@case
def live_leader_term_then_singleton_reap():
    manager, child, backend, clock, process = fixture()
    backend.on_signal = lambda: setattr(backend, 'leader', exited())
    receipt = manager.cleanup(child)
    check(receipt['success'] and receipt['resolved'] and receipt['reaped'])
    check(backend.delivered == [(policy.TERM, 42, 0)])
    check(process._waitpid_lock.requests == [False] and not process._waitpid_lock.held)
    check(not policy.RETAINED_HANDLES)


@case
def exited_leader_requires_positive_live_member_for_owned_group_signal():
    manager, child, backend, clock, process = terminal_fixture()
    backend.pids = [43, 42]
    backend.on_signal = lambda: setattr(backend, 'pids', [42])
    check(manager.cleanup(child)['success'])
    check(backend.delivered == [(policy.TERM, 42, 43)])
    check(len(operations(backend, 'member')) == 1)


@case
def multiple_zombies_never_become_negative_certificate():
    manager, child, backend, clock, process = terminal_fixture()
    backend.pids = [42, 43]
    backend.member_state = policy.EXITED
    manager.cleanup(child)
    retained(child)
    check(not backend.delivered and not operations(backend, 'reap'))
    events = list(backend.events)
    manager.cleanup(child)
    check(events == backend.events)


@case
def full_two_entry_buffer_is_not_completion_even_if_repeated():
    manager, child, backend, clock, process = terminal_fixture(max_calls=32)
    backend.pids = [42, 43]
    manager.cleanup(child)
    retained(child)
    check(not operations(backend, 'reap') and child.calls <= 32)


@case
def enumeration_zero_partial_oversized_missing_duplicate_and_errors_refuse():
    for updates in (
        {'raw_bytes': 0, 'count': 0}, {'raw_bytes': -1}, {'raw_bytes': 3},
        {'raw_bytes': 5}, {'raw_bytes': 9}, {'raw_bytes': 12, 'count': 3},
        {'raw_bytes': 4, 'count': 2}, {'raw_bytes': 4, 'member0': 43},
        {'raw_bytes': 8, 'count': 2, 'member0': 42, 'member1': 42},
        {'raw_bytes': 4, 'count': 1, 'member0': 42, 'member1': 43},
        {'raw_bytes': 4, 'count': 1, 'member0': 42, 'pgid': 99},
        {'error': 3, 'system_errno': 1}, {'error': 5},
    ):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = terminal_fixture()
        sample = record(state=0, uid_evidence=policy.UID_RETAINED,
                        raw_bytes=4, count=1, member0=42)
        sample.update(updates)
        backend.scripts['members'] = [sample]
        manager.cleanup(child)
        retained(child)
        check(not operations(backend, 'signal') and not operations(backend, 'reap'))


@case
def singleton_certificate_brackets_equal_wnowait_and_latches_before_reap():
    manager, child, backend, clock, process = terminal_fixture()
    check(manager.cleanup(child)['success'])
    names = [item[0] for item in backend.events]
    check(names == ['describe', 'open', 'observe', 'members', 'observe', 'latch', 'reap'])
    check(child.no_signals and child.reap_attempted and len(operations(backend, 'reap')) == 1)
    manager.cleanup(child)
    check(names == [item[0] for item in backend.events])


@case
def changed_wnowait_status_or_live_exit_bracket_refuses():
    for before, after in ((exited(), exited(exit_status=1, returncode=1)),
                          (record(), exited()), (exited(), record())):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = terminal_fixture()
        backend.scripts['observe'] = [before, after]
        manager.cleanup(child)
        retained(child)
        check(not operations(backend, 'latch') and not backend.delivered)


@case
def positive_member_disappeared_zombie_group_uid_or_session_races_refuse():
    for value in (OSError('authored ESRCH'), record(pid=43, state=policy.EXITED),
                  record(pid=43, pgid=99), record(pid=43, uid=502), record(pid=43, sid=99),
                  record(pid=44), record(pid=43, state=0)):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = terminal_fixture()
        backend.pids = [42, 43]
        backend.scripts['member'] = [value]
        manager.cleanup(child)
        retained(child)
        check(not backend.delivered)


@case
def positive_member_leader_status_race_refuses():
    manager, child, backend, clock, process = terminal_fixture()
    backend.pids = [42, 43]
    backend.scripts['observe'] = [exited(), exited(), exited(),
                                  exited(exit_status=1, returncode=1)]
    manager.cleanup(child)
    retained(child)
    check(not operations(backend, 'signal'))


@case
def native_last_moment_positive_race_aborts_without_reap_or_more_signals():
    manager, child, backend, clock, process = terminal_fixture()
    backend.pids = [42, 43]
    backend.scripts['signal'] = [exited(error=5)]
    backend.on_signal = lambda: setattr(backend, 'pids', [42])
    receipt = manager.cleanup(child)
    retained(child)
    check(not receipt['reaped'] and not receipt['resolved'])
    check(receipt['signals'][0]['error'] and not backend.delivered)


@case
def signal_error_stage_and_malformed_errno_cannot_hide_ownership_loss():
    for updates in ({'error': 3, 'system_errno': 10, 'signal_attempted': 0},
                    {'error': 3, 'system_errno': 1, 'signal_attempted': 0},
                    {'error': 3, 'system_errno': True, 'signal_attempted': 1},
                    {'error': 4, 'signal_attempted': 1},
                    {'error': 3, 'system_errno': 3, 'signal_attempted': 1, 'pgid': 99}):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = fixture()
        backend.scripts['signal'] = [record(**updates)]
        backend.on_signal = lambda: setattr(backend, 'leader', exited())
        manager.cleanup(child)
        retained(child)
        check(len(operations(backend, 'signal')) == 1 and not operations(backend, 'reap'))


@case
def exited_leader_signal_eperm_remains_failure_after_terminal_reap():
    manager, child, backend, clock, process = terminal_fixture()
    backend.pids = [42, 43]
    backend.scripts['signal'] = [exited(error=3, system_errno=1, signal_attempted=1)]
    backend.on_signal = lambda: setattr(backend, 'pids', [42])
    receipt = manager.cleanup(child)
    check(receipt['resolved'] and receipt['reaped'] and not receipt['success'])
    check(len(operations(backend, 'signal')) == 1)


@case
def sticky_eperm_and_esrch_cannot_turn_into_success():
    for native_errno in (1, 3):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = fixture()
        backend.scripts['signal'] = [record(error=3, system_errno=native_errno,
                                            signal_attempted=1)]
        backend.on_signal = lambda: setattr(backend, 'leader', exited())
        receipt = manager.cleanup(child)
        check(receipt['reaped'] and not receipt['success'])
        check(any('errno=' + str(native_errno) in message for message in receipt['errors']))
        check(not receipt['retained'] and receipt['signals'][0]['error'])


@case
def signal_error_without_terminal_certificate_retains_failure():
    manager, child, backend, clock, process = fixture(max_calls=24)
    backend.scripts['signal'] = [record(error=3, system_errno=1, signal_attempted=1)]
    manager.cleanup(child)
    retained(child)
    check(len(child.receipt['signals']) == 1 and not operations(backend, 'reap'))


@case
def final_reap_echild_denial_missing_and_changed_status_are_failures():
    for value in (exited(error=3, system_errno=10), exited(error=3, system_errno=1),
                  exited(no_signals=1, reap_attempted=1, reaped=0),
                  exited(no_signals=1, reap_attempted=1, reaped=1,
                         exit_status=2, returncode=2), None):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = terminal_fixture()
        backend.scripts['reap'] = [value]
        manager.cleanup(child)
        retained(child)
        check(child.no_signals and child.reap_attempted and process.returncode is None)
        events = list(backend.events)
        manager.cleanup(child)
        check(events == backend.events and len(operations(backend, 'reap')) == 1)


@case
def signaled_and_core_status_reconcile_without_python_wait_poll():
    for kind in (policy.SIGNALED, policy.COREDUMP):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = terminal_fixture()
        backend.leader = exited(exit_kind=kind, exit_status=9, returncode=-9)
        check(manager.cleanup(child)['success'] and process.returncode == -9)


@case
def unknown_highbit_exit_status_refuses():
    for updates in ({'exit_kind': 0}, {'exit_status': 256, 'returncode': 256},
                    {'exit_kind': policy.SIGNALED, 'exit_status': 0},
                    {'returncode': 1}):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = terminal_fixture()
        backend.leader.update(updates)
        manager.cleanup(child)
        retained(child)
        check(not operations(backend, 'reap'))


@case
def native_latch_failure_or_interrupt_forbids_any_later_signal_or_reap():
    for value in (exited(error=5), RuntimeError('authored latch failure'), KeyboardInterrupt()):
        policy.RETAINED_HANDLES.clear()
        manager, child, backend, clock, process = terminal_fixture()
        backend.scripts['latch'] = [value]
        manager.cleanup(child)
        retained(child)
        check(child.no_signals and not operations(backend, 'reap'))
        events = list(backend.events)
        manager.cleanup(child)
        check(events == backend.events)
        raises(policy.Unsupported, manager._send, child, policy.KILL, (42,))


@case
def publication_failure_retains_strong_handle_after_reap_without_numeric_signals():
    manager, child, backend, clock, process = terminal_fixture()
    process.publication_error = RuntimeError('authored publication failure')
    receipt = manager.cleanup(child)
    retained(child)
    check(receipt['reaped'] and receipt['publication']['error'])
    check(child.process is process and process._waitpid_lock.held)
    process.pid = -1
    raises(policy.Unsupported, manager.assert_registration_available)
    events = list(backend.events)
    manager.cleanup(child)
    check(events == backend.events and not backend.delivered)


@case
def lock_release_failure_retains_even_with_published_returncode():
    manager, child, backend, clock, process = terminal_fixture()
    process._waitpid_lock.release_error = RuntimeError('authored release failure')
    manager.cleanup(child)
    retained(child)
    check(child.no_signals and process.returncode == 0)


@case
def invalidated_pid_and_lost_manager_local_map_still_block_new_registration():
    manager, child, backend, clock, process = fixture()
    process.pid = None
    manager.handles.clear()
    other = policy.OwnedChildren(FakeBackend(), FakeClock())
    raises(policy.Unsupported, other.assert_registration_available)
    check(policy.RETAINED_HANDLES[child.token].process is process)
    backend.leader = exited()
    check(manager.cleanup(child)['success'])
    other.assert_registration_available()


@case
def cached_popen_status_is_never_kernel_ownership_proof():
    manager, child, backend, clock, process = terminal_fixture()
    process.returncode = 0
    manager.cleanup(child)
    retained(child)
    check(not operations(backend, 'reap') and not backend.delivered)


@case
def competing_waiter_and_cached_registration_status_refuse_nonblocking():
    for busy, cached in ((True, None), (False, 0)):
        policy.RETAINED_HANDLES.clear()
        process = FakeProcess()
        process._waitpid_lock.busy = busy
        process.returncode = cached
        manager, child, backend, clock, process = fixture(process=process)
        retained(child)
        check(not child.registered and not operations(backend, 'open'))
        check(all(value is False for value in process._waitpid_lock.requests))


@case
def registration_and_publication_interruptions_retain_without_fabrication():
    for phase in ('describe', 'open', 'publication'):
        for error in (RuntimeError('authored failure'), KeyboardInterrupt(), SystemExit(7)):
            policy.RETAINED_HANDLES.clear()
            backend = FakeBackend()
            if phase != 'publication':
                backend.scripts[phase] = [error]
            manager, child, backend, clock, process = fixture(backend=backend)
            if phase == 'publication':
                process.publication_error = error
                backend.leader = exited()
                manager.cleanup(child)
                check(child.no_signals and child.receipt['reaped'])
            retained(child)
            check(process.returncode is None)


@case
def single_deadline_covers_term_grace_kill_and_certificate_without_reset():
    manager, child, backend, clock, process = fixture(timeout=0.25, grace=0.09,
                                                    clock=FakeClock(0.01), max_calls=96)
    receipt = manager.cleanup(child)
    retained(child)
    check(receipt['deadline'] == 0.25 and child.deadline == 0.25 and child.kill_at == 0.09)
    check([entry['kind'] for entry in receipt['signals']] == [policy.TERM, policy.KILL])
    calls = child.calls
    manager.cleanup(child)
    check(child.calls == calls and child.deadline == 0.25)


@case
def finite_overall_budget_handles_nonadvancing_fake_clock():
    manager, child, backend, clock, process = fixture(max_calls=17, clock=FakeClock(0))
    manager.cleanup(child)
    retained(child)
    check(child.calls == 17)
    check(sum(len(operations(backend, name)) for name in
              ('observe', 'members', 'member', 'signal', 'latch', 'reap')) == 17)


@case
def budget_exhaustion_after_latch_still_blocks_signals_and_reap_retries():
    manager, child, backend, clock, process = terminal_fixture(max_calls=4)
    manager.cleanup(child)
    retained(child)
    check(child.no_signals and child.reap_attempted and not operations(backend, 'reap'))
    events = list(backend.events)
    manager.cleanup(child)
    check(events == backend.events)


@case
def fake_clock_backward_nan_bool_or_deadline_exhaustion_refuse():
    for values in ([1.0, 0.0], [0.0, float('nan')], [0.0, True], [0.0, 3.0]):
        policy.RETAINED_HANDLES.clear()
        clock = FakeClock()
        clock.values = list(values)
        manager, child, backend, clock, process = fixture(clock=clock)
        manager.cleanup(child)
        retained(child)
        check(not backend.delivered)


@case
def stalled_fake_backend_does_not_claim_unconditional_wall_time():
    manager, child, backend, clock, process = terminal_fixture()
    def delayed_observation():
        clock.now = 1000
        return exited()
    backend.scripts['observe'] = [delayed_observation]
    manager.cleanup(child)
    retained(child)
    check(not operations(backend, 'members') and child.calls == 1)


@case
def exceptions_and_interruptions_fail_closed_at_every_observation_phase():
    for phase in ('observe', 'members', 'member', 'signal', 'latch', 'reap'):
        for error in (RuntimeError('authored failure'), KeyboardInterrupt(), SystemExit(7)):
            policy.RETAINED_HANDLES.clear()
            manager, child, backend, clock, process = terminal_fixture(max_calls=24)
            if phase in ('member', 'signal'):
                backend.pids = [42, 43]
            backend.scripts[phase] = [error]
            manager.cleanup(child)
            retained(child)
            check(any(phase == item['operation'] and item['error']
                      for item in child.receipt['observations']))
            if phase in ('latch', 'reap'):
                check(child.no_signals)
            check(len(operations(backend, 'reap')) <= 1)


@case
def foreign_manager_handle_refused_without_backend_operation():
    manager, child, backend, clock, process = fixture()
    other = policy.OwnedChildren(FakeBackend(), FakeClock())
    raises(policy.Unsupported, other.cleanup, child)
    check(not operations(backend, 'observe'))


@case
def legacy_source_bridge_is_disabled_and_uses_compiled_sdk_only_not_runtime_validation():
    root = __file__.rsplit('/scripts/', 1)[0]
    with open(root + '/crates/sims-app/native/window_owned_child_darwin.h') as source:
        header = source.read()
    with open(root + '/crates/sims-app/native/window_owned_child_darwin.c') as source:
        body = source.read()
    check('WOC_NATIVE_QUALIFIED = 0' in header)
    check('WOC_CHILD_NATIVE_QUALIFIED = 0' in header)
    # These adoption/group checks cover the legacy API prefix. Child's native
    # creation and direct-child abort have their own zero-gated protocol.
    legacy, child_gate, child = body.partition('_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 0,')
    check(bool(child_gate) and bool(child))
    check('proc_listpids(PROC_PGRP_ONLY' in legacy and 'PROC_ALL_PIDS' not in body)
    check('WEXITED | WNOWAIT | WNOHANG' in legacy and 'WEXITED | WNOHANG' in legacy)
    check('sigaction(SIGCHLD, NULL' in legacy and 'SA_NOCLDWAIT' in legacy)
    check('killpg(owner->pgid' in legacy and 'kill(' not in legacy)
    check('si_uid' not in legacy and 'proc_listpgrppids' not in body)
    check(legacy.index('owner->no_signals = 1') < legacy.index('int woc_reap('))


def main():
    install_guard()
    guard_rejects_real_operations_before_policy_use()
    print('PREIMPORT GUARD SELF-CHECK PASS: no real operation was performed')
    global policy
    policy = __import__('window_owned_child')
    failures = []
    print('GUARDED AUTHORED MOCKS ONLY; C/ABI/platform UNCOMPILED/UNRUN')
    for function in CASES:
        policy.RETAINED_HANDLES.clear()
        try:
            function()
            check(not any(policy.QUALIFICATIONS.values()))
            print('PASS ' + function.__name__)
        except BaseException as error:
            failures.append(function.__name__)
            print('FAIL ' + function.__name__ + ': ' + type(error).__name__ + ': ' + str(error))
        finally:
            policy.RETAINED_HANDLES.clear()
    print('RESULT cases=' + str(len(CASES)) + ' passed=' + str(len(CASES) - len(failures)) +
          ' failed=' + str(len(failures)))
    print('ALL QUALIFICATIONS FALSE; no native load/build, real process/wait/census/signal/sleep/network')
    return int(bool(failures))


if __name__ == '__main__':
    raise SystemExit(main())
