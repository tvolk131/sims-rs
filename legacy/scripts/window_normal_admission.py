"""Private normal-true preparation. SOURCE ONLY; all controls UNRUN.

Parent wiring: prepare_private_true(family, manifest_sha256) -> Admission, then
Attempt(admission). Only the explicit later ChildExecution.admit may create.
Import and preparation never create/resume/publish a Child. Public production,
Continue, healthy and all seven alpha qualifications remain closed.

Uses the unchanged suspended schema primitives and stable descriptor reader,
never its profile-dependent bootstrap/FamilyRoot/native loader or globals.
Every partial Admission, descriptor, native load and exception remains rooted.
There is no retry, close, destructor, adoption, target fallback or release.

Python internals are not a security boundary. Descriptor/path comparisons do
not defeat a same-UID rename-and-restore attack; extension loading and true exec
use pathnames, not atomic execution of the admitted descriptors. The parent
owns bootstrap, packaging, actual-source/image controls and runtime permission.
The fixed image pin is an input requirement, not permission to locate an image.
"""

import hashlib
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import stat
import sys
import types

import window_owned_child_bridge as _protocol
import window_suspended_host as _sealed


PROFILE = 'NORMAL_TRUE_ONLY_PRIVATE_DIAGNOSTIC_NO_PRODUCTION'
FILES = ('window_normal_host.py', 'window_normal_admission.py',
         'window_normal_diagnostic.py', 'window_owned_child_bridge.py',
         'window_suspended_host.py', 'window_suspended_host_io.py',
         '_window_owned_child_bridge.so')
PYTHON = _sealed.PYTHON
PYTHON_SHA256 = _sealed.PYTHON_SHA256
IMAGE_BYTES = 109552
IMAGE_SHA256 = '2b591afbf653005501243afcc848f94d5e38b95e7323c2fb9826595c6cf27bf6'
_NATIVE = '_window_owned_child_bridge'
_ADMISSIONS = []
_require = _sealed._require


def _copy(value):
    if type(value) is dict:
        return {key: _copy(item) for key, item in value.items()}
    if type(value) is tuple:
        return tuple(_copy(item) for item in value)
    return value


def _manifest(raw, pin, uid):
    _sealed._hex(pin, 64)
    _require(type(raw) is bytes and len(raw) <= 1024**2 and
             hashlib.sha256(raw).hexdigest() == pin, 'Exact parent manifest digest required')
    value = json.loads(raw.decode('utf-8'), object_pairs_hook=_sealed._unique,
                       parse_constant=lambda unused: _require(False, 'Finite JSON required'))
    _sealed._keys(value, ('schema', 'profile', 'root', 'fifo', 'token', 'files',
                          'child_cwd', 'target', 'python'))
    _require(type(value['schema']) is int and value['schema'] == 1 and
             type(value['profile']) is str and value['profile'] == PROFILE,
             'Private normal-only schema required')
    for key, mode in (('root', 0o700), ('fifo', 0o600), ('child_cwd', 0o700)):
        _sealed._identity(value[key], mode, uid)
    _sealed._hex(value['token'], 32)
    _sealed._keys(value['files'], FILES)
    for pin_file in value['files'].values():
        _sealed._keys(pin_file, ('bytes', 'mode', 'sha256'))
        _sealed._integer(pin_file['bytes'], 1, _sealed.MAX_FILE)
        _require(type(pin_file['mode']) is int and pin_file['mode'] in (0o400, 0o500),
                 'Sealed source/image mode required')
        _sealed._hex(pin_file['sha256'], 64)
    image = value['files'][_NATIVE + '.so']
    _require(image['bytes'] == IMAGE_BYTES and image['sha256'] == IMAGE_SHA256,
             'Only the fixed parent-qualified private image pin is admitted')
    target = value['target']
    _sealed._keys(target, ('path', 'device', 'inode', 'bytes', 'uid', 'mode', 'mtime_ns', 'sha256'))
    _require(type(target['path']) is str and target['path'] == '/usr/bin/true',
             'Only the fixed true target is admitted')
    for key in ('device', 'inode', 'mtime_ns'):
        _sealed._integer(target[key], 0, 2**64 - 1)
    _sealed._integer(target['bytes'], 1, _sealed.MAX_FILE)
    _require(type(target['uid']) is int and target['uid'] == 0 and
             type(target['mode']) is int and target['mode'] == 0o755, 'Root target pin required')
    _sealed._hex(target['sha256'], 64)
    _sealed._keys(value['python'], ('path', 'sha256'))
    _require(value['python'] == dict(path=PYTHON, sha256=PYTHON_SHA256), 'Pinned Python required')
    return value


def _environment(environ):
    fixed = dict(PATH='/usr/bin:/bin', LC_ALL='C', TMPDIR='/private/tmp')
    key = 'SIMS_NORMAL_MANIFEST_SHA256'
    _require(set(environ) in (set(fixed) | {key},
                             set(fixed) | {key, '__CF_USER_TEXT_ENCODING'}),
             'Only the fixed minimal normal bootstrap environment is allowed')
    _require(all(environ[name] == value for name, value in fixed.items()),
             'Exact normal bootstrap environment required')
    _sealed._hex(environ[key], 64)
    if '__CF_USER_TEXT_ENCODING' in environ:
        value = environ['__CF_USER_TEXT_ENCODING']
        _require(type(value) is str and len(value) <= 128 and '\0' not in value,
                 'Bounded Darwin encoding required')
    return environ[key]


class _Root:
    """Normal-family root only. Does not broaden suspended FamilyRoot."""
    def validate(self):
        before, after = os.fstat(self.fd), self.path.lstat()
        _require(stat.S_ISDIR(before.st_mode) and stat.S_ISDIR(after.st_mode) and
                 _sealed._id(before) == _sealed._id(after) == self.admitted,
                 'Original normal root identity changed')


class _Record:
    def __init__(self, admission):
        self.admission = admission
        self.root = self.manifest = self.raw_manifest = self.pin = None
        self.binding = self.bridge = self.child_type = self.terminal_type = None
        self.spec = self.loader = self.new_bridge = self.kinds = None
        self.execution = self.cwd = self.null_fd = self.options = None
        self.uid = self.pid = None
        self.ready = self.busy = self.factory_intent = self.execution_ready = self.admit_intent = False
        self.descriptors, self.files, self.roots, self.errors = [], {}, [], []
        self.claim = self.claim_stat = self.cwd_stat = self.null_stat = self.fifo_stat = None
        self.cwd_fd = self.fifo_fd = None


def _record(admission):
    _require(type(admission) is Admission, 'Exact original Admission required')
    for record in _ADMISSIONS:
        if record.admission is admission:
            return record
    raise ValueError('Unissued Admission; numeric reports/tokens/types are not authority')


def _open(record, path, flags, mode=0o777, *, dir_fd=None):
    fd = os.open(path, flags | os.O_NOFOLLOW | os.O_CLOEXEC, mode, dir_fd=dir_fd)
    record.descriptors.append(fd)  # Before fstat/read/validation; never close on uncertainty.
    return fd


def _file(record, name, maximum, *, uid, mode=None, pin=None, path=None):
    fd = _open(record, name if path is None else path, os.O_RDONLY,
               dir_fd=record.root.fd if path is None else None)
    entry = dict(fd=fd, name=name, maximum=maximum, uid=uid, mode=mode, pin=pin, path=path)
    record.files[name] = entry  # Retain partial evidence before the first read.
    raw, before = _sealed._read_fd(fd, maximum, uid=uid, mode=mode, pin=pin)
    entry.update(raw=raw, stat=before)
    _check_file(record, entry)
    return raw, before


def _check_file(record, entry):
    fd = entry['fd']
    os.lseek(fd, 0, os.SEEK_SET)
    raw, now = _sealed._read_fd(fd, entry['maximum'], uid=entry['uid'],
                               mode=entry['mode'], pin=entry['pin'])
    path_stat = (os.stat(entry['name'], dir_fd=record.root.fd, follow_symlinks=False)
                 if entry['path'] is None else os.lstat(entry['path']))
    _require(stat.S_ISREG(path_stat.st_mode) and
             _sealed._stable(entry['stat']) == _sealed._stable(now) == _sealed._stable(path_stat)
             and raw == entry['raw'], 'Original retained file/path changed')


def _context(record, family, pin):
    # Reserved PID is refused before family, interpreter image or target access.
    record.pid = os.getpid()
    _require(type(record.pid) is int and record.pid != 47707, 'Reserved process is forbidden')
    _require(type(family) is str and os.path.dirname(family) == '/private/tmp',
             'Fresh direct normal family required')
    prefix = 'sims-normal-true-'
    name = os.path.basename(family)
    _require(name.startswith(prefix), 'Exact normal family prefix required')
    _sealed._hex(name[len(prefix):], 32)
    _sealed._hex(pin, 64)
    _require(sys.platform == 'darwin' and sys.version_info[:2] == (3, 9) and
             sys.executable == PYTHON and sys.flags.isolated == 1 and sys.flags.no_site == 1
             and sys.flags.no_user_site == 1 and sys.flags.dont_write_bytecode == 1
             and 'site' not in sys.modules and type(sys.argv) is list and
             sys.argv == [family + '/window_normal_host.py'],
             'Pinned fresh normal Python -I -B -S entry required')
    _require(_environment(os.environ) == pin, 'Bootstrap/argument manifest digest disagreement')
    for name, module in (('window_normal_admission', sys.modules.get(__name__)),
                         ('window_owned_child_bridge', _protocol),
                         ('window_suspended_host', _sealed), ('__main__', sys.modules.get('__main__'))):
        filename = 'window_normal_host.py' if name == '__main__' else name + '.py'
        _require(type(module) is types.ModuleType and sys.modules.get(name) is module and
                 module.__file__ == family + '/' + filename, 'Original canonical source module required')
    _require(_NATIVE not in sys.modules, 'No native reload or adoption')
    record.uid = os.getuid()
    _sealed._integer(record.uid, 0, 2**32 - 1)
    record.pin = pin


def _bootstrap(record, family):
    root = object.__new__(_Root)
    record.root = root
    record.roots.append(root)
    root.path = Path(family)
    before = root.path.lstat()
    _require(stat.S_ISDIR(before.st_mode) and stat.S_IMODE(before.st_mode) == 0o700 and
             before.st_uid == record.uid, 'Owned nonsymlink normal root required')
    root.admitted = _sealed._id(before)
    root.fd = _open(record, family, os.O_RDONLY | os.O_DIRECTORY)
    root.validate()
    _sealed._free(root)
    _require(set(os.listdir(root.fd)) == set(FILES) | {'manifest.json', 'control.fifo', 'child-cwd'},
             'Only the exact fresh initial normal siblings are admitted')
    raw, unused = _file(record, 'manifest.json', 1024**2, uid=record.uid, mode=0o400)
    record.raw_manifest = raw
    manifest = _manifest(raw, record.pin, record.uid)
    record.manifest = manifest
    _require(root.admitted == manifest['root'], 'Manifest/root identity mismatch')
    for name in FILES:
        _file(record, name, _sealed.MAX_FILE, uid=record.uid, pin=manifest['files'][name])
    _file(record, '@python', 32 * 1024**2, uid=0, path=PYTHON)
    _require(hashlib.sha256(record.files['@python']['raw']).hexdigest() == PYTHON_SHA256,
             'Original pinned interpreter bytes required')
    unused, target = _file(record, '@target', _sealed.MAX_FILE, uid=0, mode=0o755,
                           pin=manifest['target'], path='/usr/bin/true')
    _require(all(_sealed._id(target)[key] == manifest['target'][key]
                 for key in ('device', 'inode', 'uid', 'mode')) and
             target.st_mtime_ns == manifest['target']['mtime_ns'], 'Exact target identity required')
    root.validate()


def _resources(record):
    root, manifest = record.root, record.manifest
    root.validate()
    record.cwd_fd = _open(record, 'child-cwd', os.O_RDONLY | os.O_DIRECTORY, dir_fd=root.fd)
    record.cwd_stat = os.fstat(record.cwd_fd)
    record.fifo_fd = _open(record, 'control.fifo', os.O_RDWR | os.O_NONBLOCK, dir_fd=root.fd)
    record.fifo_stat = os.fstat(record.fifo_fd)
    before_null = os.lstat('/dev/null')
    record.null_fd = _open(record, '/dev/null', os.O_RDWR)
    record.null_stat = os.fstat(record.null_fd)
    _require(_sealed._id(before_null) == _sealed._id(record.null_stat) and
             before_null.st_rdev == record.null_stat.st_rdev, 'Null changed during open')
    record.cwd = os.fsencode(str(root.path / 'child-cwd'))
    arguments = _sealed._arguments(record.cwd, record.null_fd)
    record.options = (arguments['env'], arguments['stdio'], 10_000_000_000,
                      10_000_000_000, 100_000_000, 128)
    _check_resources(record)


def _check_resources(record):
    root, manifest = record.root, record.manifest
    root.validate()
    for name, fd, before, kind, key in (
            ('child-cwd', record.cwd_fd, record.cwd_stat, stat.S_ISDIR, 'child_cwd'),
            ('control.fifo', record.fifo_fd, record.fifo_stat, stat.S_ISFIFO, 'fifo')):
        current = os.fstat(fd)
        path_stat = os.stat(name, dir_fd=root.fd, follow_symlinks=False)
        _require(kind(current.st_mode) and kind(path_stat.st_mode) and
                 _sealed._id(before) == _sealed._id(current) == _sealed._id(path_stat) == manifest[key],
                 'Original cwd/FIFO resource changed')
        if key == 'fifo':
            _require(before.st_nlink == current.st_nlink == path_stat.st_nlink == 1,
                     'Single-link FIFO required')
    current, path_stat = os.fstat(record.null_fd), os.lstat('/dev/null')
    _require(all(stat.S_ISCHR(s.st_mode) and s.st_uid == 0 for s in
                 (record.null_stat, current, path_stat)) and
             _sealed._id(record.null_stat) == _sealed._id(current) == _sealed._id(path_stat) and
             record.null_stat.st_rdev == current.st_rdev == path_stat.st_rdev,
             'Original retained character /dev/null required')
    _sealed._free(root)


def _claim(record):
    root = record.root
    root.validate()
    raw = (json.dumps(dict(profile=PROFILE, manifest_sha256=record.pin, host_pid=record.pid),
                      sort_keys=True, separators=(',', ':')) + '\n').encode('ascii')
    fd = _open(record, 'host.claim', os.O_RDWR | os.O_CREAT | os.O_EXCL, 0o600, dir_fd=root.fd)
    record.claim = fd
    position = 0
    while position < len(raw):
        count = os.write(fd, raw[position:])
        _require(type(count) is int and 0 < count <= len(raw) - position, 'Incomplete exclusive claim')
        position += count
    os.fchmod(fd, 0o400)
    os.fsync(fd)
    os.fsync(root.fd)  # Must succeed before the first native loader access.
    record.claim_stat = os.fstat(fd)
    record.files['host.claim'] = dict(fd=fd, name='host.claim', maximum=1024**2,
                                      uid=record.uid, mode=0o400, pin=None, path=None,
                                      stat=record.claim_stat, raw=raw)
    _check_file(record, record.files['host.claim'])


def _check_native(record):
    binding, path = record.binding, str(record.root.path / (_NATIVE + '.so'))
    _require(type(binding) is types.ModuleType and sys.modules.get(_NATIVE) is binding and
             binding.__name__ == _NATIVE and binding.__file__ == path and
             binding.__spec__ is record.spec and binding.__loader__ is record.loader and
             record.spec.origin == path and record.spec.name == _NATIVE and
             record.spec.loader is record.loader and
             type(record.loader) is importlib.machinery.ExtensionFileLoader and
             record.loader.name == _NATIVE and record.loader.path == path,
             'Actual original canonical extension origin required')
    _require(type(binding.CHILD_NATIVE_QUALIFIED) is int and binding.CHILD_NATIVE_QUALIFIED == 1,
             'Fixed private gate1 image required; no public qualification')
    kinds = (binding.Bridge, binding.Child, binding.TerminalSink)
    _require(kinds == record.kinds and len(set(kinds)) == 3 and
             all(type(kind) is type and not kind.__flags__ & (1 << 9) and
                 kind.__module__ == _NATIVE and kind.__name__ == expected
                 for kind, expected in zip(kinds, ('Bridge', 'Child', 'TerminalSink'))),
             'Original exact distinct nonheap native types required')
    _require(type(record.new_bridge) is types.BuiltinFunctionType and
             record.new_bridge.__self__ is binding and binding.new_bridge is record.new_bridge and
             type(record.bridge) is kinds[0] and record.new_bridge() is record.bridge,
             'Original actual native singleton Bridge required')


def _load_native(record):
    _require(record.claim is not None and record.claim_stat is not None and
             _NATIVE not in sys.modules, 'Durable exclusive claim and fresh native module required')
    path = str(record.root.path / (_NATIVE + '.so'))
    _check_file(record, record.files[_NATIVE + '.so'])
    record.spec = importlib.util.spec_from_file_location(_NATIVE, path)
    record.roots.append(record.spec)
    _require(record.spec is not None and
             type(record.spec.loader) is importlib.machinery.ExtensionFileLoader and
             record.spec.origin == path and record.spec.name == _NATIVE,
             'Exact canonical extension specification required')
    record.loader = record.spec.loader
    record.roots.append(record.loader)
    record.binding = importlib.util.module_from_spec(record.spec)
    record.roots.append(record.binding)
    sys.modules[_NATIVE] = record.binding
    record.loader.exec_module(record.binding)
    _check_file(record, record.files[_NATIVE + '.so'])
    record.root.validate()
    record.kinds = (record.binding.Bridge, record.binding.Child, record.binding.TerminalSink)
    record.child_type, record.terminal_type = record.kinds[1:]
    record.new_bridge = record.binding.new_bridge
    # Authenticate the builtin factory before invoking it. A type name/flag is insufficient.
    _require(type(record.new_bridge) is types.BuiltinFunctionType and
             record.new_bridge.__self__ is record.binding, 'Actual extension factory required')
    record.bridge = record.new_bridge()
    record.roots.append(record.bridge)
    _check_native(record)
    _sealed._empty_registry(record.bridge, record.roots)


def _revalidate(record):
    _require(record.ready and not record.errors and record.claim is not None,
             'Original ready private admission required')
    _require(os.getpid() == record.pid and record.pid != 47707 and os.getuid() == record.uid,
             'Original interpreter owner required')
    record.root.validate()
    for entry in record.files.values():
        _check_file(record, entry)
    _require(_manifest(record.raw_manifest, record.pin, record.uid) == record.manifest,
             'Original manifest changed')
    _check_resources(record)
    _check_native(record)
    _sealed._empty_registry(record.bridge, record.roots)


def _bind_execution(admission, execution):
    """Called only by the distinct bridge factory; consumes actual issuance."""
    record = _record(admission)
    _require(record.ready and record.busy and record.factory_intent and not record.errors
             and record.execution is None and type(execution) is _protocol.ChildExecution
             and any(item is execution for item in _protocol._CHILD_EXECUTIONS)
             and execution._bridge is None and execution._claim is None,
             'One original ready Admission and inert rooted ChildExecution required')
    record.execution = execution  # Before revalidation or any binding access.
    record.roots.append(execution)
    _revalidate(record)
    _require(not any(item is not execution and item._bridge is record.bridge
                     for item in _protocol._CHILD_EXECUTIONS), 'Bridge already belongs to an execution')
    return record.binding, record.bridge, record.child_type, record.terminal_type, record.options


def _execution_ready(admission, execution):
    record = _record(admission)
    _require(record.execution is execution and not record.execution_ready and
             record.busy and record.factory_intent and not record.errors and
             execution._binding is record.binding and execution._bridge is record.bridge and
             execution._claim is admission and execution._child_type is record.child_type and
             execution._terminal_type is record.terminal_type and execution._options == record.options,
             'Exact original private execution must be bound before publication')
    record.execution_ready = True


class Admission:
    """Exact, noncopyable issuance; no public constructor or reset/release."""
    __slots__ = ()

    def __init__(self, *arguments, **keywords):
        raise TypeError('Use prepare_private_true; no Admission adoption')

    @property
    def execution(self):
        return _record(self).execution

    @property
    def binding(self):
        return _record(self).binding

    @property
    def bridge(self):
        return _record(self).bridge

    @property
    def root(self):
        return _record(self).root

    @property
    def manifest(self):
        return _copy(_record(self).manifest)

    @property
    def child_type(self):
        return _record(self).child_type

    @property
    def terminal_type(self):
        return _record(self).terminal_type

    @property
    def native_qualified(self):
        return False

    def owns_execution(self, execution):
        record = _record(self)
        return (execution is not None and record.execution is execution and
                type(execution) is _protocol.ChildExecution and
                any(item is execution for item in _protocol._CHILD_EXECUTIONS))

    def create_execution(self):
        record = _record(self)
        _require(record.ready and not record.errors and not record.busy and
                 not record.factory_intent, 'One private execution factory intent only')
        record.factory_intent = record.busy = True
        try:
            return _protocol._private_normal_execution(self)
        except BaseException as error:
            record.errors.append(error)
            raise
        finally:
            record.busy = False

    def _admit(self, execution):
        record = _record(self)
        _require(record.ready and not record.errors and not record.busy and
                 not record.admit_intent and record.execution_ready and
                 self.owns_execution(execution), 'One original private execution admission only')
        record.admit_intent = record.busy = True
        try:
            _require(execution._binding is record.binding and execution._bridge is record.bridge
                     and execution._claim is self and execution._child_type is record.child_type
                     and execution._terminal_type is record.terminal_type
                     and execution._options == record.options and execution._admission_started is True
                     and execution._busy is True
                     and not execution._create_entered and execution._fault is None,
                     'Original bound execution/intent/resources required')
            _revalidate(record)
            return (b'/usr/bin/true',), record.cwd
        except BaseException as error:
            record.errors.append(error)
            raise
        finally:
            record.busy = False

    def __copy__(self):
        raise TypeError('Admission cannot be copied')

    def __deepcopy__(self, memo):
        raise TypeError('Admission cannot be copied')

    def __reduce_ex__(self, protocol):
        raise TypeError('Admission cannot be serialized')


def prepare_private_true(family: str, manifest_sha256: str) -> Admission:
    """Admit only fixed resources; no Child creation, resume or publication."""
    admission = object.__new__(Admission)
    record = _Record(admission)
    _ADMISSIONS.append(record)  # BEFORE validation, path/PID access or native loading.
    try:
        _require(len(_ADMISSIONS) == 1, 'One fresh normal preparation per interpreter; no retry')
        _context(record, family, manifest_sha256)
        _bootstrap(record, family)
        _resources(record)
        _claim(record)
        _load_native(record)
        record.ready = True
        _revalidate(record)
        return admission
    except BaseException as error:
        record.errors.append(error)
        raise
