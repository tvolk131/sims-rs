"""Private suspended-true host entry. Source-only implementation; controls UNRUN.

Only main performs admission and loads the parent-admitted private C/PyC image.
The underscore helpers are protocol test seams, never native authentication.
Parent owns packaging, the manifest pin, static/image admission and execution.
No production, Continue, healthy, publication or normal-execution authority.
"""

import hashlib
import importlib.machinery
import importlib.util
import json
import os
import stat
import sys
import types


PYTHON = '/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/bin/python3.9'
PYTHON_SHA256 = 'bdea59019a38eb6600cc9e71e984a97fedadc406448431281e7657030f54987e'
PROFILE = 'SUSPENDED_TRUE_ONLY_NO_RESUME_NO_PUBLICATION'
FILES = ('window_suspended_host.py', 'window_suspended_host_core.py',
         'window_suspended_host_io.py', 'window_suspended_diagnostic.py',
         'window_owned_child_bridge.py', '_window_owned_child_bridge.so')
MODULES = ('window_suspended_host_io', 'window_suspended_host_core',
           'window_owned_child_bridge', 'window_suspended_diagnostic')
MAX_FILE = 4 * 1024**2
MIN_FREE = 6 * 1024**3
MAX_FAILURES = 64
MAX_PROOFS = 4
MAX_ERRORS = 256  # Covers 64 commands, 64 read failures, 63 failed delays and setup.
_STARTED = False
_SIGNALS = 0
_LIFETIME = []


def _require(condition, message):
    if not condition:
        raise ValueError(message)


def _keys(value, names):
    _require(type(value) is dict and set(value) == set(names), 'Exact schema keys required')


def _integer(value, low, high):
    _require(type(value) is int and low <= value <= high, 'Exact bounded integer required')


def _hex(value, size):
    _require(type(value) is str and len(value) == size and
             all(c in '0123456789abcdef' for c in value), 'Exact lowercase hex required')


def _identity(value, mode, uid):
    _keys(value, ('device', 'inode', 'uid', 'mode'))
    for key in ('device', 'inode', 'uid', 'mode'):
        _integer(value[key], 0, 2**64 - 1)
    _require(value['mode'] == mode and value['uid'] == uid, 'Owned identity required')


def _id(s):
    return dict(device=s.st_dev, inode=s.st_ino, uid=s.st_uid,
                mode=stat.S_IMODE(s.st_mode))


def _stable(s):
    return (_id(s), s.st_size, s.st_nlink, s.st_mtime_ns, s.st_ctime_ns)


def _unique(pairs):
    result = {}
    for key, value in pairs:
        _require(key not in result, 'Duplicate JSON key')
        result[key] = value
    return result


def _manifest(raw, pin, uid):
    _hex(pin, 64)
    _require(type(raw) is bytes and len(raw) <= 1024**2 and
             hashlib.sha256(raw).hexdigest() == pin, 'Parent manifest digest required')
    value = json.loads(raw.decode('utf-8'), object_pairs_hook=_unique,
                       parse_constant=lambda unused: _require(False, 'Finite JSON required'))
    _keys(value, ('schema', 'profile', 'root', 'fifo', 'token', 'files',
                  'child_cwd', 'target', 'python'))
    _require(type(value['schema']) is int and value['schema'] == 1 and
             value['profile'] == PROFILE, 'Private suspended-only schema required')
    _identity(value['root'], 0o700, uid)
    _identity(value['fifo'], 0o600, uid)
    _identity(value['child_cwd'], 0o700, uid)
    _hex(value['token'], 32)
    _keys(value['files'], FILES)
    for item in value['files'].values():
        _keys(item, ('bytes', 'mode', 'sha256'))
        _integer(item['bytes'], 1, MAX_FILE)
        _require(type(item['mode']) is int and item['mode'] in (0o400, 0o500),
                 'Sealed source/image mode required')
        _hex(item['sha256'], 64)
    target = value['target']
    _keys(target, ('path', 'device', 'inode', 'bytes', 'uid', 'mode', 'mtime_ns', 'sha256'))
    _require(target['path'] == '/usr/bin/true', 'Only the fixed true target is allowed')
    for key in ('device', 'inode', 'mtime_ns'):
        _integer(target[key], 0, 2**64 - 1)
    _integer(target['bytes'], 1, MAX_FILE)
    _require(type(target['uid']) is int and target['uid'] == 0 and
             type(target['mode']) is int and target['mode'] == 0o755, 'Root target pin required')
    _hex(target['sha256'], 64)
    _keys(value['python'], ('path', 'sha256'))
    _require(value['python'] == dict(path=PYTHON, sha256=PYTHON_SHA256), 'Pinned Python required')
    return value


def _environment(environ):
    expected = dict(PATH='/usr/bin:/bin', LC_ALL='C', TMPDIR='/private/tmp')
    _require(set(environ) in (set(expected) | {'SIMS_SUSPENDED_MANIFEST_SHA256'},
                             set(expected) | {'SIMS_SUSPENDED_MANIFEST_SHA256',
                                              '__CF_USER_TEXT_ENCODING'}),
             'Only the minimal parent environment is allowed')
    _require(all(environ[key] == value for key, value in expected.items()),
             'Exact fixed environment required')
    _hex(environ['SIMS_SUSPENDED_MANIFEST_SHA256'], 64)
    if '__CF_USER_TEXT_ENCODING' in environ:
        text = environ['__CF_USER_TEXT_ENCODING']
        _require(type(text) is str and len(text) <= 128 and '\0' not in text,
                 'Bounded Darwin encoding required')
    return environ['SIMS_SUSPENDED_MANIFEST_SHA256']


def _read_fd(fd, maximum, *, uid, mode=None, pin=None):
    before = os.fstat(fd)
    _require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1 and
             before.st_uid == uid and 0 <= before.st_size <= maximum and
             not stat.S_IMODE(before.st_mode) & 0o022 and
             (mode is None or stat.S_IMODE(before.st_mode) == mode),
             'Owned stable single-link regular file required')
    chunks, remaining = [], before.st_size + 1
    while remaining:
        chunk = os.read(fd, min(65536, remaining))
        if not chunk:
            break
        chunks.append(chunk)
        remaining -= len(chunk)
    raw = b''.join(chunks)
    _require(_stable(before) == _stable(os.fstat(fd)) and len(raw) == before.st_size,
             'File changed during same-descriptor read')
    if pin is not None:
        _require(len(raw) == pin['bytes'] and stat.S_IMODE(before.st_mode) == pin['mode']
                 and hashlib.sha256(raw).hexdigest() == pin['sha256'], 'Pinned file mismatch')
    return raw, before


def _file_at(fd, name, maximum, uid, mode=None, pin=None):
    opened = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
    try:
        return _read_fd(opened, maximum, uid=uid, mode=mode, pin=pin)
    finally:
        os.close(opened)  # A failed close is never retried.


def _bootstrap(path, pin, uid):
    _require(os.path.dirname(path) == '/private/tmp' and
             os.path.basename(path).startswith('sims-suspended-true-'), 'Fresh direct family required')
    before = os.lstat(path)
    _require(stat.S_ISDIR(before.st_mode) and _id(before)['mode'] == 0o700 and
             before.st_uid == uid, 'Owned nonsymlink family required')
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
    _LIFETIME.append(fd)
    _require(_id(before) == _id(os.fstat(fd)) == _id(os.lstat(path)), 'Family changed')
    _require(set(os.listdir(fd)) == set(FILES) | {'manifest.json', 'control.fifo', 'child-cwd'},
             'Only exact fresh family siblings are admitted')
    raw, unused = _file_at(fd, 'manifest.json', 1024**2, uid, 0o400)
    manifest = _manifest(raw, pin, uid)
    _require(_id(before) == manifest['root'], 'Manifest family identity mismatch')
    sources = {}
    for name in FILES:
        sources[name], unused = _file_at(fd, name, MAX_FILE, uid, pin=manifest['files'][name])
    _require(_id(os.fstat(fd)) == _id(os.lstat(path)) == manifest['root'], 'Family changed')
    return manifest, sources


def _source(name, raw, path):
    _require(name not in sys.modules, 'No module reload or adoption')
    module = types.ModuleType(name)
    module.__file__ = path
    module.__package__ = ''
    sys.modules[name] = module
    _LIFETIME.append(module)  # Retain partial module even if compile/exec raises.
    exec(compile(raw, path, 'exec', dont_inherit=True), module.__dict__)
    return module


def _claim(root, io, pin, pid):
    root.validate()
    raw = io.encode(dict(profile=PROFILE, manifest_sha256=pin, host_pid=pid))
    fd = os.open('host.claim', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW |
                 os.O_CLOEXEC, 0o600, dir_fd=root.fd)
    try:
        position = 0
        while position < len(raw):
            count = os.write(fd, raw[position:])
            _require(count > 0, 'Incomplete host claim')
            position += count
        os.fchmod(fd, 0o400)
        os.fsync(fd)
    finally:
        os.close(fd)
    os.fsync(root.fd)


def _native(root, io, manifest):
    # Called only by main after complete file admission and durable host.claim.
    name = '_window_owned_child_bridge'
    _require(name not in sys.modules, 'No native reload or adoption')
    path = str(root.path / (name + '.so'))
    pin = manifest['files'][name + '.so']
    root.validate()
    unused, before = _file_at(root.fd, name + '.so', MAX_FILE, os.getuid(), pin=pin)
    spec = importlib.util.spec_from_file_location(name, path)
    _require(spec is not None and type(spec.loader) is importlib.machinery.ExtensionFileLoader,
             'Exact extension loader required')
    module = importlib.util.module_from_spec(spec)
    _LIFETIME.append(module)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    unused, after = _file_at(root.fd, name + '.so', MAX_FILE, os.getuid(), pin=pin)
    root.validate()
    _require(_stable(before) == _stable(after), 'Image metadata changed across load')
    _require(type(module.CHILD_NATIVE_QUALIFIED) is int and module.CHILD_NATIVE_QUALIFIED == 1,
             'Private gate1 image required')
    kinds = (module.Bridge, module.Child, module.TerminalSink)
    _require(len(set(kinds)) == 3 and all(type(k) is type and k.__module__ == name and
             not k.__flags__ & (1 << 9) and k.__name__ == expected
             for k, expected in zip(kinds, ('Bridge', 'Child', 'TerminalSink'))),
             'Exact distinct native concrete types required')
    bridge = module.new_bridge()
    _LIFETIME.append(bridge)
    _require(type(bridge) is module.Bridge and module.new_bridge() is bridge,
             'Stable exact native Bridge required')
    _empty_registry(bridge, _LIFETIME)
    return module, bridge


def _empty_registry(bridge, retained=None):
    evidence = []
    for unused in range(2):
        pending = bridge.pending()
        if retained is not None:
            retained.append(pending)
        history = bridge.history()
        if retained is not None:
            retained.append(history)
        _require(pending is None and type(history) is tuple and history == (),
                 'Twice empty exact registry required')
        evidence.append(dict(pending_none=True, history=()))
    return tuple(evidence)


def _target(pin):
    # Identity evidence only; this is not atomic pathname-exec proof against root mutation.
    fd = os.open('/usr/bin/true', os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    _LIFETIME.append(fd)
    raw, s = _read_fd(fd, MAX_FILE, uid=0, mode=0o755, pin=pin)
    _require(all(_id(s)[key] == pin[key] for key in ('device', 'inode', 'uid', 'mode'))
             and s.st_mtime_ns == pin['mtime_ns'], 'Target identity mismatch')
    return fd


def _python():
    fd = os.open(PYTHON, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    _LIFETIME.append(fd)
    raw, unused = _read_fd(fd, 32 * 1024**2, uid=0)
    _require(hashlib.sha256(raw).hexdigest() == PYTHON_SHA256, 'Interpreter digest mismatch')


def _resources(root, manifest):
    root.validate()
    before = os.stat('child-cwd', dir_fd=root.fd, follow_symlinks=False)
    _require(stat.S_ISDIR(before.st_mode) and _id(before) == manifest['child_cwd'],
             'Exact owned child cwd required')
    cwd_fd = os.open('child-cwd', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW |
                     os.O_CLOEXEC, dir_fd=root.fd)
    _LIFETIME.append(cwd_fd)
    _require(_id(os.fstat(cwd_fd)) == _id(before) ==
             _id(os.stat('child-cwd', dir_fd=root.fd, follow_symlinks=False)), 'Cwd changed')
    before_null = os.lstat('/dev/null')
    null_fd = os.open('/dev/null', os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC)
    _LIFETIME.append(null_fd)
    after_null = os.fstat(null_fd)
    _require(stat.S_ISCHR(before_null.st_mode) and stat.S_ISCHR(after_null.st_mode) and
             _id(before_null) == _id(after_null) == _id(os.lstat('/dev/null')) and
             before_null.st_rdev == after_null.st_rdev, 'Exact character /dev/null required')
    return os.fsencode(str(root.path / 'child-cwd')), null_fd


def _arguments(cwd, null_fd):
    result = dict(executable=b'/usr/bin/true', argv=(b'/usr/bin/true',),
                  env=(b'PATH=/usr/bin:/bin', b'LC_ALL=C'), cwd=cwd,
                  stdio=(null_fd,) * 3, launch_ns=10000000000,
                  cleanup_ns=10000000000, grace_ns=0, max_entries=128)
    _require(type(null_fd) is int and 0 <= null_fd < 2**31, 'Owned exact fd required')
    _require(type(cwd) is bytes and cwd.startswith(b'/'), 'Absolute owned cwd required')
    strings = (result['executable'], cwd) + result['argv'] + result['env']
    _require(all(type(s) is bytes and 0 < len(s) < 65536 and b'\0' not in s for s in strings)
             and sum(len(s) + 1 for s in strings) <= 65536 and
             len(result['argv']) <= 128 and len(result['env']) <= 128,
             'C per-string/vector/aggregate copied-string bounds required')
    return result


def _free(root):
    s = os.fstatvfs(root.fd)
    _require(s.f_bavail * s.f_frsize >= MIN_FREE, 'At least 6GiB free required')


def _interrupt(signum, frame):
    global _SIGNALS
    # Only these three handlers are installed; never inspect or alter SIGCHLD.
    _SIGNALS |= {1: 1, 2: 2, 15: 4}.get(signum, 0)


def _policies():
    import resource
    import signal
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
    cap = 64 * 1024**2
    if hard != resource.RLIM_INFINITY:
        cap = min(cap, hard)
    if soft != resource.RLIM_INFINITY:
        cap = min(cap, soft)
    resource.setrlimit(resource.RLIMIT_FSIZE, (cap, cap))
    for name in ('SIGINT', 'SIGTERM', 'SIGHUP'):
        signal.signal(getattr(signal, name), _interrupt)


def _dormant():
    import signal
    import time
    while True:
        try:
            signal.pause()  # No further actions, evidence growth or controller dependency.
        except BaseException:
            try:
                time.sleep(60)
            except BaseException:
                pass


class _Host:
    """Retained command state; injected objects here confer no native authority."""

    def __init__(self, io, core, schema, consumer, binding, bridge, root, journal,
                 fifo, token, arguments, pause):
        self.io, self.core, self.schema, self.consumer = io, core, schema, consumer
        self.binding, self.bridge, self.root = binding, bridge, root
        self.journal, self.fifo, self.pause = journal, fifo, pause
        self.commands = io.Commands(token)
        self.attempt = self.coordinator = None
        self.run_consumed = self.intent_started = self.native_may_exist = False
        self.storage_failed = False
        self.lines = self.read_failures = self.proof_reads = self.protocol_faults = 0
        self.first_error = None
        self.errors, self.proofs, self.replies = [], [], []
        _LIFETIME.append(self)  # BEFORE Attempt construction or method access.
        self.attempt = consumer.Attempt(binding, bridge, **arguments)
        self.coordinator = core.Coordinator(self.attempt)

    def remember(self, where, error):
        if self.first_error is None:
            self.first_error = error
        if len(self.errors) < MAX_ERRORS:
            self.errors.append((where, error))

    def record(self, kind, **facts):
        if self.storage_failed:
            return False
        try:
            self.journal.append(dict(kind=kind, signals=_SIGNALS, **facts))
            return True
        except BaseException as error:
            self.storage_failed = True
            self.remember('journal_' + kind, error)
            return False

    def transcript(self, kind):
        coordinator = self.coordinator
        self.record(kind, events=coordinator.events, facts=coordinator.facts,
                    operations=coordinator.operations,
                    observation_grants=coordinator.observation_grants,
                    refreshes=coordinator.refreshes, status=coordinator.status,
                    errors=tuple((where, type(error).__name__, str(error)[:4096])
                                 for where, error in self.errors),
                    coordinator_errors=tuple((where, type(error).__name__, str(error)[:4096])
                                             for where, error in coordinator.errors))

    def _no_roots(self):
        a = self.attempt
        _require(a._child is None and a._sink is None and not a._roots and
                 not a._returned_child and a._generation is None, 'No known roots required')

    def _exit_proof(self):
        _require(not self.protocol_faults and self.proof_reads < MAX_PROOFS,
                 'No protocol fault and finite fresh exit budget required')
        self.proof_reads += 1  # Before read or method access; a lost reply consumes its slot.
        if not self.intent_started:
            _require(not self.attempt._requested and not self.coordinator.started,
                     'No creation intent required')
            self._no_roots()
            return dict(retirement='no_attempt', known_roots_empty=True,
                        registry=_empty_registry(self.bridge, self.replies))
        _require(self.native_may_exist and self.coordinator.started,
                 'A failed creation intent cannot be replayed or adopted')
        facts = self.attempt.read()  # NEW coherent read; never coordinator.facts/cached().
        self.replies.append(facts)
        _require(self.core.retired(facts), 'Fresh scoped retirement required')
        if facts['retirement'] == 'preparation_rejected':
            self._no_roots()
            _require(facts['terminal'] is None and self.attempt._dispatched,
                     'Exact preparation rejection required')
            return dict(facts=facts, known_roots_empty=True,
                        registry=_empty_registry(self.bridge, self.replies))
        a, t = self.attempt, facts['terminal']
        sink, child = a._sink, a._child
        _require(type(sink) is self.binding.TerminalSink and
                 type(child) is self.binding.Child and type(a._generation) is int and
                 a._generation > 0, 'Original exact native Child/Sink required')
        snapshots, registries = [], []
        proof = dict(facts=facts, snapshots=snapshots, registry=registries)
        self.proofs.append(proof)  # Keep partial proof if independent native reads fail.
        def registry():
            pending = self.bridge.pending()
            self.replies.append(pending)  # Before history access can lose a reply.
            history = self.bridge.history()
            self.replies.append(history)
            _require(pending is None and type(history) is tuple and len(history) == 1
                     and history[0] is sink, 'Stable original terminal history required')
            registries.append(dict(pending_none=True, original_sink=True,
                                   generation=a._generation))
        registry()
        for unused in range(2):
            snapshot = sink.snapshot()
            snapshots.append(snapshot)
            self.schema._terminal_report(snapshot, a._generation)
            _require(snapshot == t, 'Independent original sink snapshot changed')
        registry()  # Bracket both original-sink snapshots with independent registry reads.
        r = t['final_report']
        _require(t['generation'] == a._generation and t['rooted'] == t['transferred'] == 1
                 and r['generation'] == a._generation and
                 r['resources_closed'] == 1 and r['resource_error'] == 0 and
                 r['resume_entered'] == r['resume_returned'] == r['published'] == 0,
                 'Closed resources and suspended-only transfer required')
        consuming = ('reap_attempted', 'wait_entered', 'wait_returned', 'consumed',
                     'decoded', 'certificate_matches')
        if facts['retirement'] == 'kind1_no_creation':
            _require(t['kind'] == 1 and t['known_no_creation'] == 1 and r['state'] == 11
                     and r['spawn_entered'] == r['spawn_returned'] == 0 and
                     all(t[name] == 0 for name in consuming), 'Exact kind1 retirement required')
        else:
            _require(facts['retirement'] == 'kind3_unresumed_abort' and t['kind'] == 3
                     and t['known_no_creation'] == 0 and r['state'] in (4, 8) and
                     r['registered'] == r['pid_valid'] == 1 and
                     r['spawn_entered'] == r['spawn_returned'] == 1 and r['spawn_rc'] == 0
                     and r['abort']['phase'] == 7 and all(t[name] == 1 for name in consuming)
                     and t['wait_rc'] == 0, 'Exact kind3 certificate required')
        # Successful return codes do NOT require raw errno == 0.
        return proof

    def dispatch(self, command):
        """True only after a fresh proof has been durably journaled."""
        try:
            if command == 'RUN':
                if self.run_consumed or self.protocol_faults or self.storage_failed:
                    self.protocol_faults += 1
                    return False
                self.run_consumed = True  # BEFORE free-space check/journal/method access.
                self.intent_started = True
                _free(self.root)
                if not self.record('RUN_INTENT'):
                    return False
                self.native_may_exist = True  # BEFORE accessing Coordinator.start.
                self.coordinator.start(self.pause)  # Full finite chain; NO intervening disk I/O.
                self.transcript('RUN_RESULT')
            elif command == 'ADVANCE':
                if (not self.native_may_exist or not self.coordinator.started or
                        self.storage_failed or self.protocol_faults or
                        self.coordinator.operations >= self.core.MAX_OPERATIONS):
                    return False
                if self.record('ADVANCE_INTENT', operations=self.coordinator.operations):
                    self.coordinator.advance(self.pause)
                    self.transcript('ADVANCE_RESULT')
            elif command == 'READ':
                if self.coordinator.started and self.coordinator.refreshes < self.core.MAX_REFRESHES:
                    self.coordinator.refresh()
                    self.transcript('READ_RESULT')
            elif command == 'CACHED':
                if self.coordinator.started:
                    facts = self.coordinator.cached()
                    self.record('CACHED_RESULT', facts=facts)
            elif command == 'EXIT':
                proof = self._exit_proof()
                return self.record('EXIT_READY', proof=proof, proof_reads=self.proof_reads,
                                   run_consumed=self.run_consumed, intent_started=self.intent_started,
                                   native_may_exist=self.native_may_exist)
            else:
                self.protocol_faults += 1  # Unknown/overflow never dispatches a native operation.
        except BaseException as error:
            self.remember('command_' + str(command), error)
            self.transcript('COMMAND_FAILURE')
        return False

    def serve(self):
        while self.lines < self.io.MAX_COMMANDS and self.read_failures < MAX_FAILURES:
            try:
                chunk = self.fifo.read()
                _require(type(chunk) is bytes and 0 < len(chunk) <= 128,
                         'Unexpected empty/invalid control chunk')
                for command in self.commands.feed(chunk):
                    if self.lines >= self.io.MAX_COMMANDS:
                        break
                    self.lines += 1
                    if self.dispatch(command):
                        return 0
            except BaseException as error:
                self.read_failures += 1
                self.remember('control_read', error)
                self.record('CONTROL_FAILURE', failure=self.read_failures,
                            error=(type(error).__name__, str(error)[:4096]))
                if self.read_failures < MAX_FAILURES:
                    try:
                        self.pause(1)
                    except BaseException as delay_error:
                        self.remember('control_recovery_delay', delay_error)
        _dormant()  # Exhaustion grants no exit or additional native action/logging.


def main():
    global _STARTED
    _require(not _STARTED, 'One fresh interpreter lifetime required')
    _STARTED = True
    pid = os.getpid()
    _require(pid != 47707, 'Reserved process is forbidden')
    _require(sys.platform == 'darwin' and sys.version_info[:2] == (3, 9) and
             sys.executable == PYTHON and len(sys.argv) == 1 and
             os.path.abspath(sys.argv[0]) == os.path.abspath(__file__) and
             os.path.basename(__file__) == FILES[0] and sys.flags.isolated == 1 and
             sys.flags.no_site == 1 and sys.flags.no_user_site == 1 and
             sys.flags.dont_write_bytecode == 1 and 'site' not in sys.modules,
             'Pinned fresh Python -I -B -S entry required')
    pin = _environment(os.environ)
    _require(all(name not in sys.modules for name in MODULES + ('_window_owned_child_bridge',)),
             'Fresh modules required')
    host = journal = None
    try:
        uid = os.getuid()
        path = os.path.dirname(os.path.abspath(__file__))
        _python()
        manifest, sources = _bootstrap(path, pin, uid)
        loaded = {}
        for name in MODULES:
            loaded[name] = _source(name, sources[name + '.py'], path + '/' + name + '.py')
        io, core = loaded[MODULES[0]], loaded[MODULES[1]]
        root = io.FamilyRoot(path)
        _LIFETIME.append(root)
        _require(root.admitted == manifest['root'], 'Bootstrap/root identity changed')
        journal = io.Journal(root)
        _LIFETIME.append(journal)
        _policies()
        _target(manifest['target'])
        cwd, null_fd = _resources(root, manifest)
        fifo = io.Fifo(root, manifest['fifo'])  # O_RDWR keeps its own writer.
        _LIFETIME.append(fifo)
        _claim(root, io, pin, pid)
        binding, bridge = _native(root, io, manifest)
        import time
        host = _Host(io, core, loaded[MODULES[2]], loaded[MODULES[3]], binding,
                     bridge, root, journal, fifo, manifest['token'],
                     _arguments(cwd, null_fd), time.sleep)
        _require(host.record('HOST_READY', profile=PROFILE, manifest_sha256=pin,
                             production_child=0, production_legacy=0,
                             alpha_gates=(False,) * 7), 'Durable host admission required')
        return host.serve()
    except BaseException as error:
        must_retain = host is not None and (host.intent_started or host.native_may_exist)
        try:
            _LIFETIME.append(error)
            if host is not None:
                host.remember('main', error)
                host.transcript('HOST_FAILURE')
            elif journal is not None:
                try:
                    journal.append(dict(kind='SETUP_FAILURE', error=(type(error).__name__, str(error)[:4096])))
                except BaseException as journal_error:
                    _LIFETIME.append(journal_error)
        finally:
            # Even failure reporting/allocation must not escape after a possible
            # creation. Active exception frames and all roots stay alive here.
            if must_retain:
                _dormant()
        return 1  # Setup refusal before any possible Child creation only.


if __name__ == '__main__':
    sys.exit(main())
