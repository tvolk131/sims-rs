"""SOURCE-ONLY Layer A, UNRUN; production construction always fails before I/O.

No import-time loader, clock, Popen creation, waiting, probing or builder selection.
The distinct private normal factory consumes an original rooted Admission; it
does not confer public/native qualification or use the injected-mock seam.
Injected bridges are authored mocks, never authenticated native backends. The
opaque facade handle proves only local registry identity, not process ownership.
The C candidate roots an attempt before native open and never relinquishes it.
Mock injection relies on the same retention obligation at its return boundary;
pure Python cannot close a bridge's pre-return interruption/publication gap.

Reap consumes status, not allocation lifetime. Release is deliberately absent:
Layer B must supply separately reviewed authentic creation, exclusive waiting,
cached-status/destructor and reconciliation ownership. A boolean, PID or duck
object is not that handoff. Even successful mock reap keeps strong registries.
No finalizer cleans up; unresolved operations block this mock instance forever.
The C process-wide retained attempt additionally blocks all native open attempts.
Hostile mutation of Python internals is not an authenticity/security boundary.
"""

ABI_VERSION = 1
DESCRIPTION_BYTES = 32
RECORD_BYTES = 200
HEADER_FIELDS = ('bytes', 'version', 'error', 'system_errno')
DESCRIPTION_FIELDS = ('record_bytes', 'pid_bytes', 'platform', 'enabled')
RECORD_FIELDS = (
    'pid', 'pgid', 'uid', 'ppid', 'sid', 'state', 'exit_kind', 'exit_status',
    'wait_uid_available', 'uid_evidence', 'sigchld_default', 'no_cldwait',
    'raw_bytes', 'pid_bytes', 'capacity', 'count', 'member0', 'member1',
    'no_signals', 'reap_attempted', 'reaped', 'returncode', 'signal_attempted',
)
REPLY_FIELDS = ('operation', 'rc', 'initialized', 'description', 'record', 'owner')
QUALIFICATIONS = {
    'build': False, 'native': False, 'abi': False, 'platform': False,
    'runtime_backend': False, 'app': False, 'alpha': False,
    'layer_b': False, 'authentic_popen': False, 'exclusive_waiter': False,
    'pre_return_interruption': False, 'reconciliation_release': False,
    'trusted_bootstrap': False, 'mapped_image': False, 'builder_selected': False,
    'driver_enabled': False, 'compiler37394_failure_resolved': False,
}
_RETAINED = []


class Unsupported(RuntimeError):
    pass


class BridgeProtocolError(Unsupported):
    def __init__(self, message, reply=None):
        super().__init__(message)
        self.reply = reply


class NativeFailure(BridgeProtocolError):
    def __init__(self, operation, reply, record):
        super().__init__(operation + ': native error (unaltered reply retained)', reply)
        self.record = record
        self.rc = reply['rc']


class ReconciliationRequired(Unsupported):
    pass


def production_backend(*arguments, **keywords):
    raise Unsupported('Production hard-disabled before import/load/describe/clock')


def require_launch_ready(*arguments, **keywords):
    raise Unsupported('Layer A is not a production launch or builder backend')


def _require(condition, message, reply=None):
    if not condition:
        raise BridgeProtocolError(message, reply)


def _integer(value, minimum, maximum, name, reply=None):
    _require(type(value) is int and minimum <= value <= maximum,
             'Invalid exact integer: ' + name, reply)
    return value


def _fields(value, fields, reply):
    _require(type(value) is dict and all(type(key) is str for key in value) and
             set(value) == set(fields), 'Invalid exact mapping fields', reply)


def _decode(reply, operation, description=False):
    _fields(reply, REPLY_FIELDS, reply)
    _require(type(reply['operation']) is str and reply['operation'] == operation,
             'Reply operation mismatch', reply)
    _integer(reply['rc'], -(2 ** 31), 2 ** 31 - 1, 'rc', reply)
    _require(type(reply['initialized']) is bool and reply['initialized'] is True,
             'Uninitialized/short/unknown output cannot become a mapping', reply)
    name = 'description' if description else 'record'
    _require(reply['record' if description else 'description'] is None,
             'Ambiguous output kind', reply)
    _require(operation == 'open' or reply['owner'] is None, 'Unexpected owner', reply)
    record = reply[name]
    fields = DESCRIPTION_FIELDS if description else RECORD_FIELDS
    _fields(record, HEADER_FIELDS + fields, reply)
    for field in HEADER_FIELDS + fields:
        minimum, maximum = -(2 ** 63), 2 ** 63 - 1
        if field in ('bytes', 'version') or description and field in DESCRIPTION_FIELDS:
            minimum, maximum = 0, 2 ** 32 - 1
        elif field in ('error', 'system_errno'):
            minimum, maximum = -(2 ** 31), 2 ** 31 - 1
        _integer(record[field], minimum, maximum, field, reply)
    _require(record['bytes'] == (DESCRIPTION_BYTES if description else RECORD_BYTES) and
             record['version'] == ABI_VERSION, 'Exact ABI size/version mismatch', reply)
    _require(0 <= reply['rc'] <= 5 and reply['rc'] == record['error'],
             'Native return/header disagreement or unknown return', reply)
    _require(record['system_errno'] >= 0 and
             (record['error'] == 3 or record['system_errno'] == 0),
             'Malformed error/errno combination', reply)
    return dict(record)


class _Owner:
    __slots__ = ()

    def __new__(cls):
        raise TypeError('Opaque facade owners are not constructible')

    def __repr__(self):
        return '<opaque injected-mock owner; unqualified>'

    def __copy__(self):
        raise TypeError('Owned handles cannot be copied')

    def __deepcopy__(self, memo):
        raise TypeError('Owned handles cannot be copied')

    def __reduce_ex__(self, protocol):
        raise TypeError('Owned handles cannot be serialized')


class _Slot:
    def __init__(self, backend, pid, uid):
        self.backend = backend
        self.handle = object.__new__(_Owner)
        self.pid = pid
        self.uid = uid
        self.parent = None
        self.native_owner = None
        self.reply = None
        self.busy = True
        self.blocked = False
        self.published = False
        self.no_signals = False
        self.signal_failed = False
        self.latch_attempted = False
        self.latched = False
        self.latched_exit = None
        self.reap_attempted = False
        self.reaped = False
        self.release_requested = False


class Backend:
    def __init__(self, *arguments, **keywords):
        raise Unsupported('Production Backend construction is unconditionally disabled')

    @classmethod
    def for_injected_mock(cls, bridge):
        _require(cls is Backend, 'No alternate backend subclasses')
        result = object.__new__(Backend)
        result._bridge = bridge
        result._description = None
        result._slots = []
        result._blocked = False
        result._busy = False
        return result

    def describe(self):
        _require(not self._blocked and not self._busy, 'Blocked/reentrant mock backend')
        self._busy = True
        try:
            reply = self._bridge.describe()
            record = _decode(reply, 'describe', description=True)
            if reply['rc']:
                raise NativeFailure('describe', reply, record)
            _require(record['record_bytes'] == RECORD_BYTES and record['platform'] == 1 and
                     record['enabled'] == 0 and record['pid_bytes'] in (1, 2, 4, 8),
                     'Unqualified public description required', reply)
            _require(self._description is None or self._description == record,
                     'Description changed', reply)
            self._description = dict(record)
            return record
        except BaseException:
            self._blocked = True
            raise
        finally:
            self._busy = False

    def open(self, pid, pgid, uid):
        _require(not self._blocked and not self._busy and not self._slots and
                 self._description is not None, 'Retained attempt/missing description blocks open')
        _integer(pid, 2, 2 ** 31 - 1, 'pid')
        _integer(pgid, 2, 2 ** 31 - 1, 'pgid')
        _integer(uid, 0, 2 ** 32 - 1, 'uid')
        _require(pid == pgid, 'Fresh-session identity required, not PID adoption')
        slot = _Slot(self, pid, uid)
        _RETAINED.append(slot)
        try:
            self._slots.append(slot)
            self._busy = True
            slot.reply = self._bridge.open(pid, pgid, uid)
            if type(slot.reply) is dict:
                slot.native_owner = slot.reply.get('owner')
            record = _decode(slot.reply, 'open')
            if slot.reply['rc']:
                raise NativeFailure('open', slot.reply, record)
            _require(slot.native_owner is not None and not isinstance(slot.native_owner, (
                int, float, complex, str, bytes, bytearray, list, tuple, dict, set, frozenset,
            )), 'Missing opaque injected token (not native authenticity proof)', slot.reply)
            _require(all(other is slot or other.native_owner is not slot.native_owner
                         for other in _RETAINED), 'Reused/foreign injected token', slot.reply)
            slot.parent = _integer(record['ppid'], 1, 2 ** 31 - 1, 'ppid', slot.reply)
            self._validate(slot, record, 'open')
            result = (slot.handle, record)
            slot.published = True
            slot.busy = False
            return result
        except BaseException:
            slot.blocked = True
            self._blocked = True
            raise
        finally:
            self._busy = False

    def _slot(self, owner, releasing=False):
        _require(type(owner) is _Owner, 'Exact local facade owner required')
        slot = next((item for item in self._slots if item.handle is owner), None)
        _require(slot is not None and slot.backend is self and slot.published and
                 slot.native_owner is not None and not slot.busy and not slot.release_requested,
                 'Foreign/copied/unpublished/released owner')
        _require(releasing or not (self._blocked or slot.blocked or slot.reap_attempted or
                                   slot.reaped), 'Unresolved or terminal owner blocks operation')
        return slot

    def _validate(self, slot, record, operation, arguments=()):
        target = arguments[0] if operation == 'member' else slot.pid
        _require(record['pid'] == target and record['pgid'] == slot.pid and
                 record['uid'] == slot.uid and record['ppid'] == slot.parent and
                 record['sid'] == slot.pid, 'Owned identity mismatch', slot.reply)
        _require(record['wait_uid_available'] == 0 and record['sigchld_default'] == 1 and
                 record['no_cldwait'] == 0 and record['pid_bytes'] == self._description['pid_bytes'] and
                 record['capacity'] == 2, 'Disposition/capability/capacity mismatch', slot.reply)
        _require(record['no_signals'] == int(slot.no_signals) and
                 record['reap_attempted'] == int(slot.reap_attempted) and
                 record['reaped'] == int(operation == 'reap') and
                 record['signal_attempted'] == int(operation == 'signal'),
                 'Owner operation flags mismatch', slot.reply)
        if operation == 'members':
            count, width = record['count'], record['pid_bytes']
            _require(count in (1, 2) and record['raw_bytes'] == count * width and
                     record['state'] == 0 and record['uid_evidence'] == 2,
                     'Malformed/truncated membership metadata', slot.reply)
            members = (record['member0'], record['member1'])[:count]
            _require(all(type(pid) is int and 2 <= pid <= 2 ** 31 - 1 for pid in members) and
                     len(set(members)) == count and slot.pid in members and
                     (count == 2 or record['member1'] == 0), 'Missing/duplicate anchor', slot.reply)
        elif record['state'] == 1:
            _require(operation not in ('latch', 'reap') and record['uid_evidence'] == 1 and
                     record['exit_kind'] == record['exit_status'] == record['returncode'] == 0,
                     'Invalid live record', slot.reply)
        else:
            kind, status = record['exit_kind'], record['exit_status']
            _require(operation not in ('open', 'member') and record['state'] == 2 and
                     record['uid_evidence'] == 2 and
                     ((kind == 1 and 0 <= status <= 255) or
                      (kind in (2, 3) and 1 <= status <= 127)) and
                     record['returncode'] == (status if kind == 1 else -status),
                     'Invalid terminal record', slot.reply)
            if operation == 'latch':
                _require((kind, status) == arguments, 'Latch status mismatch', slot.reply)
            if operation == 'reap':
                _require((kind, status) == slot.latched_exit, 'Reap changed latched status', slot.reply)

    def _call(self, operation, owner, *arguments):
        slot = self._slot(owner)
        _require(not self._busy and
                 (not slot.no_signals or operation == 'reap' and slot.latched),
                 'Reentrant or post-latch operation')
        _require(operation != 'signal' or not slot.signal_failed, 'No signal after signal failure')
        _require(operation != 'reap' or slot.latched, 'Reap requires successful latch')
        slot.busy = True
        self._busy = True
        try:
            if operation == 'latch':
                slot.latch_attempted = True
                slot.no_signals = True
                slot.latched_exit = arguments
            if operation == 'reap':
                slot.reap_attempted = True
            slot.reply = getattr(self._bridge, operation)(slot.native_owner, *arguments)
            record = _decode(slot.reply, operation)
            if slot.reply['rc']:
                if operation != 'signal' or record['error'] != 3 or record['system_errno'] <= 0:
                    raise NativeFailure(operation, slot.reply, record)
                self._validate(slot, record, operation, arguments)
                slot.signal_failed = True
                return record
            self._validate(slot, record, operation, arguments)
            if operation == 'latch':
                slot.latched = True
            if operation == 'reap':
                slot.reaped = True
            return record
        except BaseException:
            slot.blocked = True
            self._blocked = True
            raise
        finally:
            slot.busy = False
            self._busy = False

    def observe(self, owner):
        return self._call('observe', owner)

    def members(self, owner):
        return self._call('members', owner)

    def member(self, owner, pid):
        _integer(pid, 2, 2 ** 31 - 1, 'member pid')
        return self._call('member', owner, pid)

    def signal(self, owner, kind, positive):
        _integer(kind, 1, 2, 'signal kind')
        _integer(positive, 0, 2 ** 31 - 1, 'positive member')
        return self._call('signal', owner, kind, positive)

    def latch(self, owner, kind, status):
        _integer(kind, 1, 3, 'exit kind')
        _integer(status, 0 if kind == 1 else 1, 255 if kind == 1 else 127, 'exit status')
        return self._call('latch', owner, kind, status)

    def reap(self, owner):
        return self._call('reap', owner)

    def release(self, owner):
        slot = self._slot(owner, releasing=True)
        slot.release_requested = True
        slot.blocked = True
        self._blocked = True
        raise ReconciliationRequired('No Layer B handoff; native allocation remains strongly retained')


# Child-v1 is a separate protocol, never a Backend/Popen/PID adapter. No loader
# or native authority is implemented here. The future parent must authenticate
# the loaded image, exact types, interpreter and exclusive Bridge before adding
# a reviewed production entry point. All currently constructible sessions are
# permanently mocks; changing an instance attribute cannot qualify one.
_CHILD_EXECUTIONS = []
_CHILD_SCALARS = (
    'generation state rc system_errno blocked published pid_valid registered '
    'registration_bsd_status registration_pid registration_ppid registration_pgid '
    'registration_uid registration_sid spawn_entered spawn_returned spawn_rc '
    'spawn_errno spawn_pid_raw resume_entered resume_returned resume_rc resume_errno '
    'launch_entries launch_fact_count launch_deadline_ns resources_closed resource_error '
    'cleanup_started cleanup_entries max_cleanup_entries cleanup_deadline_ns '
    'cleanup_kill_at_ns cleanup_now_ns clock_rc clock_errno term_attempted kill_attempted '
    'signal_failed term_returned term_rc term_errno kill_returned kill_rc kill_errno '
    'latch_attempted sigchld_preflight_entered sigchld_preflight_returned '
    'sigchld_preflight_rc sigchld_preflight_errno sigchld_preflight_policy_valid '
    'sigchld_preflight_default sigchld_preflight_no_cldwait sigchld_preflight_accepted '
    'launch_failure_mask launch_first_failure_phase launch_first_failure_reason '
    'launch_first_woc_rc launch_first_system_errno'
).split()
_ABORT_SCALARS = (
    'last_operation last_rc last_system_errno phase started entries max_entries '
    'deadline_ns origin_ns now_ns observation_count max_observations trace_count '
    'failure_rc failure_system_errno kill_entered kill_returned kill_rc kill_errno '
    'preexisting_exit exit_observation_index exit_valid bsd_status bsd_pid bsd_ppid '
    'bsd_uid bsd_pgid sid'
).split()
_REGISTRATION_SCALARS = (
    'entered returned rc system_errno idtype target_pid options signo code pid uid '
    'status raw_siginfo_bytes'
).split()
_ABORT_OBSERVATION_SCALARS = (
    'entered returned rc system_errno raw_siginfo_bytes pid uid code status'
).split()
_TERMINAL_SCALARS = (
    'generation rooted transferred known_no_creation kind reap_attempted '
    'wait_entered wait_returned wait_rc wait_errno wait_pid wait_uid wait_code '
    'wait_status consumed decoded certificate_matches'
).split()
_ACTIVE_SCALARS = (
    'bytes version generation sequence started_ns finished_ns birth_seconds birth_microseconds '
    'entered returned rc system_errno wait_entered wait_returned wait_rc wait_errno '
    'bsd_entered bsd_returned bsd_bytes bsd_status start_clock_rc start_clock_errno '
    'finish_clock_rc finish_clock_errno creator_pid creator_uid observed_sid'
).split()
_ACTIVE_BYTES = 344  # Additive Darwin64 v1 structure; not SDK/ABI qualification.
_ACTIVE_LIMIT = 1048576
_EXIT_IDENTITY = ('pid', 'pgid', 'ppid', 'uid', 'sid', 'exit_kind', 'exit_status', 'returncode')


def production_child_execution(*arguments, **keywords):
    raise Unsupported('Production ChildExecution disabled before binding/loader/clock access')


def _native_execution_authorized(session):
    # No token, peer, bool, type name, mapping or instance flag is native authority.
    # Public qualification stays closed even for a privately admitted image.
    return False


def _new_child_execution():
    """Inert state only; a root is not creation or native authority."""
    result = object.__new__(ChildExecution)
    result._binding = result._bridge = None
    result._child_type = result._terminal_type = None
    result._options = None
    result._claim = result._child = result._sink = result._generation = None
    result._admission_started = result._create_entered = result._busy = False
    result._fault = None
    result._errors, result._intent, result._once = [], [], set()
    result._history = result._pending = result._last = None
    result._bounds = result._abort_bounds = result._previous = None
    result._exit_certificate = None
    result._native_failure_seen = False
    result._active_sequence = result._active_attempts = 0
    result._active_pending = result._active_latest = result._active_first = None
    result._active_birth = result._active_exit = None
    _CHILD_EXECUTIONS.append(result)
    return result


def _private_normal_execution(admission):
    """Private actual-image path, never a promotion of a synthetic session.

    Admission's own issuance record retains the inert execution before binding
    access/revalidation. On any lost reply its original property is the only
    recovery path. All public production gates remain unconditionally closed.
    """
    result = _new_child_execution()
    try:
        import window_normal_admission
        binding, bridge, child_type, terminal_type, options = (
            window_normal_admission._bind_execution(admission, result))
        result._binding, result._bridge = binding, bridge
        result._child_type, result._terminal_type = child_type, terminal_type
        result._options, result._claim = options, admission
        window_normal_admission._execution_ready(admission, result)
        return result
    except BaseException as error:
        result._errors.append(error)
        result._fault = error
        raise


def _scalar_mapping(value, names, extra=()):
    _fields(value, tuple(names) + tuple(extra), value)
    for name in names:
        _integer(value[name], -(2 ** 63), 2 ** 64 - 1, name, value)


def _child_record(value, reserved=False, active=False):
    _scalar_mapping(value, HEADER_FIELDS + RECORD_FIELDS)
    # C calloc reserves these exact all-zero records before the corresponding
    # observation/decode. A reserved slot is never an initialized certificate.
    if reserved and all(item == 0 for item in value.values()):
        return
    _require(value['bytes'] == RECORD_BYTES and value['version'] == ABI_VERSION,
             'Child record ABI mismatch', value)
    _integer(value['error'], 0, 5, 'record error', value)
    _integer(value['system_errno'], 0, 2 ** 31 - 1, 'record errno', value)
    _integer(value['state'], 0, 3 if active else 2, 'record state', value)
    _integer(value['exit_kind'], 0, 3, 'record exit kind', value)
    for name in ('no_signals', 'reap_attempted', 'reaped', 'signal_attempted',
                 'wait_uid_available', 'sigchld_default', 'no_cldwait'):
        _integer(value[name], 0, 1, name, value)


def _facts(value, count, bound):
    _require(type(value) is tuple and len(value) == count and 0 <= count <= bound,
             'Malformed native trace', value)
    for fact in value:
        _require(type(fact) is tuple and len(fact) == 5, 'Malformed native fact', value)
        for item in fact:
            _integer(item, -(2 ** 63), 2 ** 63 - 1, 'native fact', value)
        _require(fact[0] > 0 and fact[1] in (0, 1) and fact[2] in (0, 1) and
                 fact[2] <= fact[1] and fact[4] >= 0, 'Malformed native intent', value)


def _registration(value):
    _scalar_mapping(value, _REGISTRATION_SCALARS)
    for name in ('entered', 'returned'):
        _integer(value[name], 0, 1, name, value)
    _require(value['returned'] <= value['entered'] and value['raw_siginfo_bytes'] >= 0,
             'Malformed native registration slot', value)


def _child_report(value, generation):
    _scalar_mapping(value, _CHILD_SCALARS,
                    ('record', 'abort', 'registration_observation', 'launch_facts'))
    _require(value['generation'] == generation, 'Child generation changed', value)
    _integer(value['state'], 1, 11, 'Child state', value)
    _integer(value['rc'], 0, 5, 'Child rc', value)
    _integer(value['system_errno'], 0, 2 ** 31 - 1, 'Child errno', value)
    for name in ('blocked', 'published', 'pid_valid', 'registered', 'spawn_entered',
                 'spawn_returned', 'resume_entered', 'resume_returned', 'resources_closed',
                 'cleanup_started', 'term_attempted', 'kill_attempted', 'signal_failed',
                 'term_returned', 'kill_returned', 'latch_attempted',
                 'sigchld_preflight_entered', 'sigchld_preflight_returned',
                 'sigchld_preflight_policy_valid', 'sigchld_preflight_default',
                 'sigchld_preflight_no_cldwait', 'sigchld_preflight_accepted'):
        _integer(value[name], 0, 1, name, value)
    _integer(value['max_cleanup_entries'], 1, 512, 'cleanup budget', value)
    _integer(value['cleanup_entries'], 0, value['max_cleanup_entries'], 'entries', value)
    for name in ('cleanup_deadline_ns', 'cleanup_now_ns', 'cleanup_kill_at_ns',
                 'launch_deadline_ns', 'launch_failure_mask'):
        _integer(value[name], 0, 2 ** 64 - 1, name, value)
    _child_record(value['record'], reserved=not value['registered'] and not value['pid_valid'])
    _facts(value['launch_facts'], value['launch_fact_count'], 64)
    _registration(value['registration_observation'])
    abort = value['abort']
    _scalar_mapping(abort, _ABORT_SCALARS, ('record', 'trace', 'observations'))
    _child_record(abort['record'], reserved=not abort['started'])
    _facts(abort['trace'], abort['trace_count'], 64)
    _integer(abort['phase'], 0, 8, 'abort phase', abort)
    for name in ('started', 'kill_entered', 'kill_returned', 'preexisting_exit', 'exit_valid'):
        _integer(abort[name], 0, 1, name, abort)
    for name in ('last_rc', 'failure_rc'):
        _integer(abort[name], 0, 5, name, abort)
    _integer(abort['last_operation'], 0, 4, 'abort operation', abort)
    for name in ('deadline_ns', 'origin_ns', 'now_ns'):
        _integer(abort[name], 0, 2 ** 64 - 1, name, abort)
    _require(abort['max_entries'] == 16 and abort['max_observations'] == 8,
             'Abort bounds changed', abort)
    _integer(abort['entries'], 0, 16, 'abort entries', abort)
    _integer(abort['observation_count'], 0, 8, 'abort observations', abort)
    _require(type(abort['observations']) is tuple and len(abort['observations']) == 9,
             'Malformed abort raw slots', abort)
    for observation in abort['observations']:
        _scalar_mapping(observation, _ABORT_OBSERVATION_SCALARS)
        for name in ('entered', 'returned'):
            _integer(observation[name], 0, 1, name, observation)
        _require(observation['returned'] <= observation['entered'] and
                 observation['raw_siginfo_bytes'] >= 0, 'Malformed native abort slot', observation)
    return value


def _terminal_report(value, generation=None):
    _scalar_mapping(value, _TERMINAL_SCALARS,
                    ('raw_siginfo', 'decoded_record', 'final_report', 'registration_observation'))
    _integer(value['generation'], 1, 2 ** 64 - 1, 'terminal generation', value)
    _require(generation is None or value['generation'] == generation,
             'Terminal generation changed', value)
    for name in ('rooted', 'transferred', 'known_no_creation', 'reap_attempted',
                 'wait_entered', 'wait_returned', 'consumed', 'decoded', 'certificate_matches'):
        _integer(value[name], 0, 1, name, value)
    _require(value['rooted'] == 1 and type(value['raw_siginfo']) is bytes,
             'Unrooted/malformed terminal', value)
    _require(value['wait_returned'] <= value['wait_entered'] <= value['reap_attempted'] and
             value['certificate_matches'] <= value['decoded'] <= value['consumed'] <= value['wait_returned'],
             'Malformed consuming-wait intent', value)
    _integer(value['kind'], 0, 3, 'terminal kind', value)
    _child_record(value['decoded_record'], reserved=not value['decoded'])
    _registration(value['registration_observation'])
    _child_report(value['final_report'], value['generation'])
    return value


def _active_facts(value):
    # Immutable retained facts; a caller mutating its returned dict cannot
    # rewrite the sequence, raw bytes or first-terminal history held here.
    return (tuple(value[name] for name in _ACTIVE_SCALARS),
            tuple(value['record'][name] for name in HEADER_FIELDS + RECORD_FIELDS),
            value['raw_siginfo'])


def _active_report(value, generation, *, fresh, first):
    _scalar_mapping(value, _ACTIVE_SCALARS,
                    ('fresh_native_call', 'first_terminal', 'collection_evidence',
                     'record', 'raw_siginfo'))
    _require(value['bytes'] == _ACTIVE_BYTES and value['version'] == 1 and
             value['generation'] == generation, 'Active observation ABI/generation differs', value)
    _require(value['fresh_native_call'] is fresh and value['first_terminal'] is first and
             value['collection_evidence'] is False,
             'Cached/first observation is not a fresh query or collection', value)
    for name in ('entered', 'returned', 'wait_entered', 'wait_returned', 'bsd_entered', 'bsd_returned'):
        _integer(value[name], 0, 1, name, value)
    for name in ('sequence', 'started_ns', 'finished_ns', 'birth_seconds', 'birth_microseconds'):
        _integer(value[name], 0, 2 ** 64 - 1, name, value)
    _integer(value['sequence'], 0, _ACTIVE_LIMIT, 'active sequence', value)
    _integer(value['rc'], 0, 5, 'active rc', value)
    for name in ('system_errno', 'wait_errno', 'start_clock_errno', 'finish_clock_errno'):
        _integer(value[name], 0, 2 ** 31 - 1, name, value)
    _require(value['returned'] <= value['entered'] and
             value['wait_returned'] <= value['wait_entered'] <= value['entered'] and
             value['bsd_returned'] <= value['bsd_entered'] <= value['wait_returned'],
             'Malformed active entered/returned facts', value)
    _require(type(value['raw_siginfo']) is bytes and len(value['raw_siginfo']) <= 4096 and
             bool(value['raw_siginfo']) == bool(value['wait_returned']),
             'Malformed retained active raw bytes', value)
    record = value['record']
    _child_record(record, reserved=not value['entered'], active=True)
    if not value['entered']:
        _require(not fresh and all(value[name] == 0 for name in _ACTIVE_SCALARS
                                  if name not in ('bytes', 'version', 'generation')) and
                 all(item == 0 for item in record.values()), 'Nonempty reserved active slot', value)
        return value
    _require(value['returned'] == 1 and value['sequence'] > 0 and
             value['birth_seconds'] > 0 and 0 <= value['birth_microseconds'] < 1000000 and
             record['wait_uid_available'] == record['reap_attempted'] == record['reaped'] ==
             record['no_signals'] == record['signal_attempted'] == 0,
             'Active observation cannot consume, signal or collect', value)
    if first:
        _require(record['state'] == 2, 'First active terminal is not terminal', value)
    if value['rc'] != 0:
        _require(not fresh, 'Fresh active entry point cannot return a failed mapping', value)
        return value  # Preserve failed-call facts; never turn them into liveness.
    # Raw errno after a successful wait/clock call remains diagnostic data.
    # The operation rc, normalized error and ownership facts decide success.
    _require(record['error'] == record['system_errno'] == value['system_errno'] == 0 and
             value['start_clock_rc'] == value['finish_clock_rc'] == 0 and
             value['finished_ns'] >= value['started_ns'] and
             value['wait_entered'] == value['wait_returned'] == 1 and
             value['wait_rc'] == 0 and
             record['sigchld_default'] == 1 and record['no_cldwait'] == 0,
             'Unsuccessful active observation cannot report liveness/exit', value)
    if record['state'] == 1:
        _require(value['bsd_entered'] == value['bsd_returned'] == 1 and value['bsd_bytes'] > 0 and
                 value['bsd_status'] in (1, 2, 3, 4) and record['uid_evidence'] == 1 and
                 value['observed_sid'] == record['sid'], 'Invalid active live identity', value)
    elif record['state'] == 3:
        _require(value['bsd_entered'] == value['bsd_returned'] == 1 and value['bsd_bytes'] > 0 and
                 record['uid_evidence'] == 2 and value['observed_sid'] == 0,
                 'Exit transition is neither live nor terminal', value)
    elif record['state'] == 2:
        _require(value['bsd_entered'] == value['bsd_returned'] == 0 and
                 value['observed_sid'] == 0 and record['uid_evidence'] == 2 and
                 ((record['exit_kind'] == 1 and 0 <= record['exit_status'] <= 255 and
                   record['returncode'] == record['exit_status']) or
                  (record['exit_kind'] in (2, 3) and 1 <= record['exit_status'] <= 127 and
                   record['returncode'] == -record['exit_status'])),
                 'Invalid nonconsuming active terminal', value)
    else:
        raise BridgeProtocolError('Successful active observation has no supported state', value)
    if record['state'] != 2:
        _require(record['exit_kind'] == record['exit_status'] == record['returncode'] == 0,
                 'Nonterminal active observation contains exit status', value)
    return value


def _classify_child_terminal(terminal, generation):
    """Internal protocol classification only; never native/Continue authority."""
    t = _terminal_report(terminal, generation)
    r = t['final_report']
    if not t['transferred']:
        return 'unresolved', None, 'Native terminal handoff is unresolved'
    _require(r['resources_closed'] == 1 and r['resource_error'] == 0,
             'Invalid transferred terminal', t)
    _require((t['kind'] == 3 and r['state'] in (4, 8)) or
             (t['kind'] in (1, 2) and r['state'] == 11),
             'Invalid transferred terminal disposition', t)
    if t['kind'] == 1 and t['known_no_creation'] and not r['spawn_entered']:
        _require(not any(t[name] for name in ('reap_attempted', 'wait_entered',
                                            'wait_returned', 'consumed', 'decoded')),
                 'No-creation terminal contains consuming facts', t)
        return 'launch_error', None, 'Native attempt proved no child creation'
    d = t['decoded_record']
    # A successful consuming return can preserve a nonzero raw errno. Keep it
    # unchanged, while requiring the original complete matching certificate.
    consumed = (t['reap_attempted'] == t['wait_entered'] == t['wait_returned'] == 1 and
                t['wait_rc'] == 0 and t['consumed'] ==
                t['decoded'] == t['certificate_matches'] == 1 and len(t['raw_siginfo']) > 0)
    valid_exit = (d['state'] == 2 and d['error'] == 0 and d['reaped'] == 1 and
                  d['reap_attempted'] == d['no_signals'] == 1 and
                  ((d['exit_kind'] == 1 and 0 <= d['exit_status'] <= 255 and
                    d['returncode'] == d['exit_status']) or
                   (d['exit_kind'] in (2, 3) and 1 <= d['exit_status'] <= 127 and
                    d['returncode'] == -d['exit_status'])))
    _require(consumed and valid_exit and not t['known_no_creation'] and
             t['wait_pid'] == d['pid'] == r['registration_pid'] == r['spawn_pid_raw'] and
             t['wait_pid'] > 1 and t['wait_status'] == d['exit_status'] and
             d['pgid'] == d['sid'] == d['pid'] == r['registration_pgid'] == r['registration_sid'] and
             d['ppid'] == r['registration_ppid'] and d['uid'] == r['registration_uid'] and
             d['wait_uid_available'] == 0 and d['uid_evidence'] == 2 and
             t['wait_code'] > 0, 'Unproven/mismatched consuming wait', t)
    if t['kind'] == 3:
        _require(not r['resume_entered'] and r['abort']['phase'] == 7,
                 'Invalid unresumed abort terminal', t)
        return 'launch_error', d['returncode'], 'Unresumed launch abort is not healthy completion'
    _require(t['kind'] == 2, 'Unknown terminal disposition', t)
    final = r['record']
    _require(all(final[name] == d[name] for name in
                 ('pid', 'pgid', 'ppid', 'uid', 'sid', 'exit_kind', 'exit_status', 'returncode')),
             'Final/decoded returncode or certificate identity mismatch', t)
    healthy = (r['rc'] == r['system_errno'] == r['blocked'] == r['launch_failure_mask'] ==
               r['launch_first_failure_reason'] == r['launch_first_woc_rc'] ==
               r['launch_first_failure_phase'] == r['launch_first_system_errno'] ==
               r['resource_error'] == r['clock_rc'] ==
               r['term_attempted'] == r['kill_attempted'] ==
               r['signal_failed'] == r['abort']['started'] == 0 and
               r['published'] == r['registered'] == r['pid_valid'] == r['spawn_entered'] ==
               r['spawn_returned'] == r['resume_entered'] == r['resume_returned'] == 1 and
               r['spawn_rc'] == r['resume_rc'] == 0 and
               r['sigchld_preflight_rc'] ==
               r['sigchld_preflight_no_cldwait'] == 0 and
               r['sigchld_preflight_entered'] == r['sigchld_preflight_returned'] ==
               r['sigchld_preflight_policy_valid'] == r['sigchld_preflight_default'] ==
               r['sigchld_preflight_accepted'] == 1 and d['exit_kind'] == 1 and
               all(fact[1] == fact[2] == 1 for fact in r['launch_facts']))
    if not healthy:
        return 'diagnostic_failed', d['returncode'], 'Historical failure, signal or forced teardown'
    return 'exited', d['returncode'], None


class ChildExecution:
    """One PreparedLaunch attempt, rooted forever, with finite protocol steps.

    admit() is one-shot even when validation or create raises. finish_step() is
    an explicit cleanup request, NOT active observation/poll. Each call admits at
    most one cleanup/abort operation. The parent schedules later steps; no local clock,
    waits, sleeps, reset, destructor, PID adoption or registry release is used.
    """

    def __init__(self, *arguments, **keywords):
        raise Unsupported('Production ChildExecution construction is unconditionally disabled')

    @classmethod
    def for_injected_mock(cls, binding, bridge, *, bridge_type, child_type, terminal_type,
                          env=(), stdio=(0, 1, 2), launch_ns=1000000000,
                          cleanup_ns=1000000000, grace_ns=100000000, max_entries=128):
        _require(cls is ChildExecution, 'No alternate ChildExecution subclasses')
        for kind in (bridge_type, child_type, terminal_type):
            _require(type(kind) is type and kind not in (object, type, type(None)) and
                     kind.__flags__ & (1 << 9) and
                     not issubclass(kind, (int, float, complex, str, bytes, bytearray,
                                          list, tuple, dict, set, frozenset, range, memoryview)),
                     'Explicit mock protocol types required')
        _require(type(bridge) is bridge_type and len({bridge_type, child_type, terminal_type}) == 3,
                 'Distinct exact mock protocol types required')
        _require(not any(s._bridge is bridge for s in _CHILD_EXECUTIONS),
                 'A Bridge belongs to one permanently retained attempt')
        _require(type(env) is tuple and len(env) <= 128 and
                 all(type(item) is bytes and 0 < len(item) < 65536 and b'\0' not in item
                     and b'=' in item and not item.startswith(b'=') for item in env),
                 'Exact bounded environment bytes required')
        _require(type(stdio) is tuple and len(stdio) == 3, 'Exact stdio tuple required')
        for fd in stdio:
            _integer(fd, 0, 2 ** 31 - 1, 'stdio')
        for number in (launch_ns, cleanup_ns):
            _integer(number, 1, 60000000000, 'native timeout')
        _integer(grace_ns, 0, cleanup_ns, 'native grace')
        _integer(max_entries, 1, 512, 'native budget')
        result = _new_child_execution()
        result._binding, result._bridge = binding, bridge
        result._child_type, result._terminal_type = child_type, terminal_type
        result._options = (env, stdio, launch_ns, cleanup_ns, grace_ns, max_entries)
        return result

    @property
    def native_qualified(self):
        return _native_execution_authorized(self)

    def __copy__(self):
        raise TypeError('Prepared ChildExecution cannot be copied')

    def __deepcopy__(self, memo):
        raise TypeError('Prepared ChildExecution cannot be copied')

    def __reduce_ex__(self, protocol):
        raise TypeError('Prepared ChildExecution cannot be serialized')

    def _enter(self):
        _require(type(self) is ChildExecution and any(s is self for s in _CHILD_EXECUTIONS),
                 'Exact rooted ChildExecution required')
        _require(not self._busy, 'Reentrant ChildExecution operation')
        self._busy = True

    def admit(self):
        self._enter()
        try:
            _require(not self._admission_started and self._claim is not None and self._fault is None,
                     'One original prepared claim required')
            self._admission_started = True
            argv, cwd = self._claim._admit(self)
            # Empty provenance is checked on this exact Bridge BEFORE create.
            # A running Child or historical terminal can never be adopted here.
            self._pending = self._bridge.pending()
            self._history = self._bridge.history()
            _require(self._pending is None and type(self._history) is tuple and
                     not self._history, 'Fresh Bridge required; no adoption')
            self._create_entered = True
            try:
                self._child = self._bridge.create(argv[0], argv, self._options[0], cwd,
                                                  *self._options[1:])
                _require(type(self._child) is self._child_type,
                         'create must return the exact Child, never numeric/dict/Popen')
                self._recover()
            except BaseException as error:
                self._errors.append(error)
                if isinstance(error, BridgeProtocolError):
                    self._fault = error
                raise
        finally:
            # On an interrupted create, recovery is a later finite read through
            # the SAME Bridge. Neither the session nor the claim is discarded.
            self._busy = False

    def _recover(self):
        _require(self._create_entered and self._fault is None,
                 'Unadmitted or structurally blocked execution remains retained')
        history = self._bridge.history()
        self._history = history
        _require(type(history) is tuple and len(history) == 1 and
                 type(history[0]) is self._terminal_type, 'Exact one-attempt terminal history required')
        sink = history[0]
        if self._sink is None:
            self._sink = sink
        _require(sink is self._sink, 'Copied/foreign/replaced TerminalSink')
        terminal = _terminal_report(self._sink.snapshot(), self._generation)
        if self._generation is None:
            self._generation = terminal['generation']
        pending = self._bridge.pending()
        self._pending = pending
        if not terminal['transferred']:
            _require(type(pending) is self._child_type, 'Exact pending Child required')
            if self._child is None:
                self._child = pending
            _require(pending is self._child and self._bridge.terminal(self._child) is self._sink,
                     'Foreign/copied Child or Bridge terminal identity')
            report = _child_report(self._bridge.child_snapshot(self._child), self._generation)
            _require(report == terminal['final_report'], 'Native child/sink snapshot disagreement')
        else:
            _require(pending is None, 'Transferred terminal still has a pending Child')
        report = terminal['final_report']
        if (report['rc'] or report['blocked'] or report['launch_failure_mask'] or
                report['clock_rc'] or report['signal_failed']):
            self._native_failure_seen = True
        self._check_progress(report)
        self._last = terminal
        return terminal, report

    def _check_progress(self, r):
        _require(r['max_cleanup_entries'] == self._options[-1], 'Native budget changed')
        if r['cleanup_started']:
            bounds = (r['cleanup_deadline_ns'], r['max_cleanup_entries'])
            _require(bounds[0] > 0 and (self._bounds is None or self._bounds == bounds),
                     'Cleanup deadline/budget reset')
            self._bounds = bounds
        a = r['abort']
        bounds = (a['deadline_ns'], a['origin_ns'], a['max_entries'], a['max_observations'])
        _require(self._abort_bounds is None or self._abort_bounds == bounds,
                 'Reserved abort deadline/budget changed')
        self._abort_bounds = bounds
        if self._previous is not None:
            old = self._previous
            for name in ('cleanup_entries', 'cleanup_now_ns', 'cleanup_started',
                         'term_attempted', 'kill_attempted', 'signal_failed', 'latch_attempted',
                         'spawn_entered', 'resume_entered', 'blocked', 'registered', 'published'):
                _require(r[name] >= old[name], 'Native intent/time regressed: ' + name)
            _require(r['launch_failure_mask'] & old['launch_failure_mask'] == old['launch_failure_mask'],
                     'Native launch failure history regressed')
            for name in ('entries', 'now_ns', 'started', 'kill_entered', 'observation_count'):
                _require(a[name] >= old['abort'][name], 'Native abort intent/time regressed')
            if old['term_returned'] and old['term_rc'] == 0:
                _require(r['cleanup_kill_at_ns'] == old['cleanup_kill_at_ns'], 'TERM grace changed')
        self._previous = r

    def _operation(self, name, *arguments):
        # All irreversible intents precede dispatch and are never retried, even
        # when an exception occurs before the extension records its own intent.
        once = name not in ('child_observe', 'child_members', 'child_member', 'abort_observe')
        key = (name, arguments[:1] if name == 'child_signal' else ()) if once else (name, arguments)
        _require(not once or key not in self._once, 'Uncertain operation cannot be repeated')
        if once:
            self._once.add(key)
        self._intent.append(key)
        try:
            if name in ('handoff', 'abort_handoff'):
                result = getattr(self._bridge, name)(self._child, self._sink)
                if result is not self._sink:
                    self._fault = BridgeProtocolError('Handoff returned a copied/foreign sink')
                    raise self._fault
            else:
                result = getattr(self._bridge, name)(self._child, *arguments)
                _child_report(result, self._generation)
        except BaseException as error:
            self._errors.append(error)
            raise ReconciliationRequired('Operation reply uncertain; recover exact terminal facts') from error

    def _active_pair(self, child_report):
        latest = _active_report(self._sink.active_observation(), self._generation,
                                fresh=False, first=False)
        first = _active_report(self._sink.active_first_terminal(), self._generation,
                               fresh=False, first=True)
        sequence = latest['sequence']
        allowed = (self._active_sequence,) if self._active_pending is None else (
            self._active_sequence, self._active_pending)
        _require(sequence in allowed, 'Out-of-band or regressed active sequence', latest)
        facts = _active_facts(latest)
        if self._active_latest is not None:
            if sequence == self._active_sequence:
                _require(facts == self._active_latest, 'Retained active facts changed without a query', latest)
            elif latest['rc'] == 0 and self._active_latest[0][_ACTIVE_SCALARS.index('rc')] == 0:
                _require(latest['started_ns'] >= self._active_latest[0][
                    _ACTIVE_SCALARS.index('finished_ns')], 'Active clock regressed between calls', latest)
        birth = self._active_birth
        for observation in (latest, first):
            if not observation['entered']:
                continue
            record = observation['record']
            _require(all(record[name] == child_report['registration_' + native] for name, native in
                         (('pid', 'pid'), ('pgid', 'pgid'), ('ppid', 'ppid'),
                          ('uid', 'uid'), ('sid', 'sid'))) and record['pid_bytes'] == 4,
                     'Active observation belongs to another registered Child', observation)
            current_birth = (observation['birth_seconds'], observation['birth_microseconds'])
            _require(birth is None or birth == current_birth, 'Active birth identity changed', observation)
            birth = current_birth
            if observation['rc'] == 0:
                _require(observation['creator_pid'] == child_report['registration_ppid'] and
                         observation['creator_uid'] == child_report['registration_uid'],
                         'Active creator identity differs', observation)
        first_facts = _active_facts(first) if first['entered'] else None
        _require(self._active_first is None or first_facts == self._active_first,
                 'First active terminal was erased or changed', first)
        exit_identity = self._active_exit
        if first['entered']:
            _require(first['sequence'] <= sequence, 'First active terminal is from the future', first)
            if first['sequence'] == sequence:
                _require(first_facts == facts, 'Latest/first active facts disagree', latest)
            exit_identity = tuple(first['record'][name] for name in _EXIT_IDENTITY)
            if latest['rc'] == 0:
                _require(latest['record']['state'] == 2 and
                         tuple(latest['record'][name] for name in _EXIT_IDENTITY) == exit_identity,
                         'Active terminal changed or became live', latest)
        elif latest['entered'] and latest['rc'] == 0:
            _require(latest['record']['state'] != 2, 'First active terminal is missing', latest)
        # Commit only validated immutable facts. An exception before this point
        # retains the outstanding intent; a later cached read never repeats it.
        self._active_sequence, self._active_latest = sequence, facts
        self._active_birth, self._active_first = birth, first_facts
        self._active_exit, self._active_pending = exit_identity, None
        if latest['rc'] or first['rc']:
            self._native_failure_seen = True
        return latest, first

    def read_active(self, *, first_terminal=False):
        """Memory-only retained facts. Never a fresh query, poll or collection."""
        _require(type(first_terminal) is bool, 'Exact active snapshot selector required')
        self._enter()
        try:
            if not self._create_entered:
                raise Unsupported('No admitted Child active cache; no query performed')
            _, report = self._recover()
            latest, first = self._active_pair(report)
            return first if first_terminal else latest
        except BridgeProtocolError as error:
            self._fault = error
            self._errors.append(error)
            raise
        finally:
            self._busy = False

    def observe_active(self):
        """One explicit nonconsuming query of this exact resumed Child.

        No cleanup, local clock, sleep, automatic retry or completion authority.
        Mapping loss leaves an intent resolved only by later retained reads.
        """
        self._enter()
        try:
            if not self._create_entered:
                raise Unsupported('No admitted Child for active observation; no query performed')
            terminal, report = self._recover()
            if terminal['transferred'] or report['state'] != 6 or not report['published'] or \
                    report['blocked'] or report['rc'] or self._native_failure_seen or \
                    report['cleanup_started'] or report['abort']['started'] or \
                    report['resume_entered'] != 1 or report['resume_returned'] != 1 or report['resume_rc']:
                raise Unsupported('Active observation unavailable in this Child phase; no query performed')
            self._active_pair(report)
            if self._active_sequence >= _ACTIVE_LIMIT or self._active_attempts >= _ACTIVE_LIMIT:
                raise Unsupported('Active observation lifetime budget exhausted; no query performed')
            expected = self._active_sequence + 1
            self._active_pending = expected
            self._active_attempts += 1
            returned = self._bridge.active_observe(self._child)
            _active_report(returned, self._generation, fresh=True, first=False)
            latest, _ = self._active_pair(report)
            _require(returned['sequence'] == expected and _active_facts(returned) == _active_facts(latest),
                     'Fresh active reply disagrees with its exact retained call', returned)
            return returned
        except BridgeProtocolError as error:
            self._fault = error
            self._errors.append(error)
            raise
        except Unsupported:
            raise  # Phase/budget refusal does not poison an ongoing cleanup.
        except BaseException as error:
            self._errors.append(error)
            raise ReconciliationRequired('Active reply uncertain; read retained facts before another query') from error
        finally:
            self._busy = False

    def read_completion(self):
        """Read only permanent native terminal facts; never initiate cleanup."""
        self._enter()
        try:
            try:
                terminal, report = self._recover()
                return self._classification(terminal)
            except BridgeProtocolError as error:
                self._fault = error
                self._errors.append(error)
                return 'unresolved', None, 'Native terminal identity/schema is unresolved'
            except BaseException as error:
                self._errors.append(error)
                return 'unresolved', None, 'Native terminal read is unresolved'
        finally:
            self._busy = False

    def finish_step(self):
        """Explicitly begin/advance cleanup by at most one protocol operation.

        Only native-reported deadline/time/budget and sticky intent select work.
        Live grace observation is incremental. Native C rechecks eligibility and
        deadline at entry. Returning unresolved asks the parent to schedule later.
        """
        self._enter()
        try:
            try:
                t, r = self._recover()
                if t['transferred']:
                    return self._classification(t)
                a = r['abort']
                if t['known_no_creation']:
                    _require(t['kind'] == 1 and not r['spawn_entered'] and r['state'] == 7,
                             'Invalid no-creation attempt')
                    self._operation('handoff')
                elif a['started'] or not r['resume_entered']:
                    self._abort_step(t, r)
                elif not r['blocked'] and (r['rc'] == 0 or
                        r['signal_failed'] and r['rc'] == 3 and r['system_errno'] > 0):
                    self._cleanup_step(t, r)
                # A failed step is not reset. All roots survive this return.
                return 'unresolved', None, 'Cleanup requires a later native terminal read/step'
            except BridgeProtocolError as error:
                self._fault = error
                self._errors.append(error)
                return 'unresolved', None, 'Protocol validation failed; exact roots retained'
            except BaseException as error:
                self._errors.append(error)
                return 'unresolved', None, 'Operation reply lost; recover exact native facts later'
        finally:
            self._busy = False

    def _classification(self, terminal):
        if self._active_attempts:
            self._active_pair(terminal['final_report'])
            if terminal['consumed'] and self._active_exit is not None:
                _require(tuple(terminal['decoded_record'][name] for name in _EXIT_IDENTITY) == self._active_exit,
                         'Consuming terminal contradicts the first active exit', terminal)
        status, code, error = _classify_child_terminal(terminal, self._generation)
        if status == 'exited' and self._native_failure_seen:
            return 'diagnostic_failed', code, 'A previously observed native failure is retained'
        return status, code, error

    def _abort_step(self, t, r):
        a = r['abort']
        if a['phase'] == 6 and t['kind'] == 3 and t['consumed'] and t['decoded'] and t['certificate_matches']:
            self._operation('abort_handoff')  # native permits memory-only late handoff
            return
        if (a['failure_rc'] or a['phase'] == 8 or a['now_ns'] >= a['deadline_ns'] or
                a['entries'] >= a['max_entries'] - 1):
            return
        if not a['started']:
            allowed = (1 << 9) | (1 << 10)  # pre-resume clock/deadline only
            if (r['registered'] and r['pid_valid'] and r['spawn_returned'] and not r['spawn_rc'] and
                    not r['published'] and not r['cleanup_started'] and not r['resource_error'] and
                    r['resources_closed'] and r['launch_failure_mask'] & ~allowed == 0 and
                    r['state'] in (4, 8) and not t['reap_attempted']):
                self._operation('abort_begin')
        elif a['phase'] == 3 and a['observation_count'] < a['max_observations']:
            self._operation('abort_observe')
        elif a['phase'] == 4 and a['exit_valid'] and not t['reap_attempted'] and not t['wait_entered']:
            self._operation('abort_reap')

    def _cleanup_step(self, t, r):
        if not r['cleanup_started']:
            if r['state'] == 6 and r['published']:
                self._operation('cleanup_begin')
            return
        if (r['cleanup_entries'] >= r['max_cleanup_entries'] or
                r['cleanup_now_ns'] >= r['cleanup_deadline_ns']):
            return
        if r['cleanup_entries'] == 1:
            self._operation('child_observe')
            return
        if t['reap_attempted'] or t['wait_entered']:
            if t['kind'] == 2 and t['consumed'] and t['decoded'] and t['certificate_matches']:
                self._operation('handoff')
            return
        record = r['record']
        if r['latch_attempted']:
            if record['no_signals'] and not record['error']:
                self._operation('child_reap')
            return
        # These snapshots only guide cleanup. They are never active liveness,
        # Popen.returncode, collected status or permission to finalize Continue.
        if record['state'] == 0 and record['count'] == 1 and self._exit_certificate is not None:
            _require(record['member0'] == record['pid'] and record['member1'] == 0,
                     'Membership anchor changed')
            self._operation('child_latch', *self._exit_certificate)
        elif record['state'] == 0 and record['count'] == 2 and self._exit_certificate is not None:
            positive = record['member1'] if record['member0'] == record['pid'] else record['member0']
            self._signal_or_observe(r, positive)
        elif record['state'] == 2:
            self._exit_certificate = (record['exit_kind'], record['exit_status'])
            self._operation('child_members')
        elif record['state'] == 1:
            self._signal_or_observe(r, 0)
        else:
            self._operation('child_observe')

    def _signal_or_observe(self, r, positive):
        if not r['signal_failed'] and not r['term_attempted']:
            self._operation('child_signal', 1, positive)
        elif (not r['signal_failed'] and r['term_returned'] and r['term_rc'] == 0 and
              r['cleanup_kill_at_ns'] > 0 and r['cleanup_now_ns'] >= r['cleanup_kill_at_ns'] and
              not r['kill_attempted']):
            self._operation('child_signal', 2, positive)
        else:
            self._operation('child_observe')
