#!/usr/bin/env python3
"""Launch an ordinary household paused, or explicitly Load or Continue a managed save.

Strict startup and deep save restoration remain the production runtime's job.
This launcher does not qualify performance or enable inferred gameplay policies.
"""
import argparse
from abc import ABC, abstractmethod
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import weakref

import window_owned_child_bridge as _child_protocol


REPOSITORY = Path(__file__).resolve().parent.parent
HOUSEHOLDS = {
    'newbie': 'UserData/Houses/House07.iff',
    'goth': 'UserData/Houses/House05.iff',
    'mashuga': 'UserData2/Houses/House09.iff',
}


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(chunk)
    return value.hexdigest()


def protected_path(value):
    path = Path(value)
    if '..' in path.parts:
        raise ValueError('Paths must not contain traversal')
    if not path.is_absolute():
        path = REPOSITORY / path
    if any(component.is_symlink() for component in (path, *path.parents)):
        raise ValueError('Protected paths must not use symlinks: ' + str(path))
    return path


def overlaps(first, second):
    return first.is_relative_to(second) or second.is_relative_to(first)


def session_root():
    root = protected_path(REPOSITORY / 'cache/alpha-sessions')
    root.mkdir(parents=True, exist_ok=True)
    return root.resolve(strict=True)


def read_json(path):
    def unique_object(pairs):
        value = {}
        for name, item in pairs:
            if name in value:
                raise ValueError('Duplicate JSON field: ' + name)
            value[name] = item
        return value

    def invalid_constant(value):
        raise ValueError('Invalid JSON constant: ' + value)

    try:
        return json.loads(path.read_bytes(), object_pairs_hook=unique_object,
                          parse_constant=invalid_constant)
    except RecursionError as error:
        raise ValueError('JSON nesting exceeds parser depth') from error


def envelope(path):
    value = read_json(path)
    session = value.get('session') if isinstance(value, dict) else None
    if not isinstance(session, dict) or type(session.get('version')) is not int or session['version'] != 2:
        raise ValueError('Select a version-2 managed player save')
    source = session.get('source')
    if not isinstance(source, str) or not source.strip() or '\0' in source or '\\' in source:
        raise ValueError('The managed save has no valid household identity')
    identity = PurePosixPath(source)
    if identity.is_absolute() or '..' in identity.parts or identity.as_posix() != source:
        raise ValueError('The managed save household must be a relative installation path')
    if not isinstance(value.get('dialog_client'), dict):
        raise ValueError('The managed save is missing its player interface state')
    return source


def atomic_json(path, value):
    protected_path(path)
    descriptor, temporary = tempfile.mkstemp(prefix='.session-', dir=path.parent)
    temporary = Path(temporary)
    try:
        with os.fdopen(descriptor, 'w', encoding='utf-8') as stream:
            json.dump(value, stream, indent=2, allow_nan=False)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def file_state(path):
    try:
        protected_path(path)
        before = path.stat()
        if not stat.S_ISREG(before.st_mode):
            raise ValueError('Expected a regular input file')
        checksum = digest(path)
        after = path.stat()
        fields = ('st_mode', 'st_ino', 'st_dev', 'st_nlink', 'st_size', 'st_mtime_ns', 'st_ctime_ns')
        if any(getattr(before, field) != getattr(after, field) for field in fields):
            raise ValueError('File changed while hashing')
        protected_path(path)
        return {'sha256': checksum, 'size': after.st_size,
                'mode': stat.S_IMODE(after.st_mode), 'device': after.st_dev,
                'inode': after.st_ino, 'links': after.st_nlink,
                'mtime_ns': after.st_mtime_ns, 'ctime_ns': after.st_ctime_ns}
    except (OSError, ValueError) as error:
        return {'error': str(error)}


def pin_input(receipt, kind, source, target, executable=False):
    item = {'kind': kind, 'source': str(source), 'pinned': str(target),
            'before': file_state(source), 'pinned_before': None}
    receipt['inputs'].append(item)
    if 'error' in item['before']:
        raise ValueError('Cannot pin ' + kind + ': ' + item['before']['error'])
    if kind == 'executable':
        receipt['binary_sha256'] = item['before']['sha256']
    elif kind == 'selected_save':
        receipt['input_sha256'] = item['before']['sha256']
    protected_path(target)
    with source.open('rb') as reader, target.open('xb') as writer:
        shutil.copyfileobj(reader, writer)
    target.chmod(0o500 if executable else 0o400)
    item['pinned_before'] = file_state(target)
    if (item['pinned_before'].get('sha256') != item['before']['sha256']
            or file_state(source) != item['before']):
        raise ValueError('A launch input changed while being pinned: ' + kind)
    if executable and not os.access(source, os.X_OK):
        raise ValueError('The selected sims-room binary must be executable')
    return item['before']['sha256']


def check_inputs(receipt):
    for item in receipt['inputs']:
        item['after'] = file_state(Path(item['source']))
        item['pinned_after'] = file_state(Path(item['pinned']))
        item['unchanged'] = (item['before'] == item['after'] and 'error' not in item['after']
                             and item['pinned_before'] == item['pinned_after']
                             and 'error' not in item['pinned_after'])
    unchanged = bool(receipt['inputs']) and all(item['unchanged'] for item in receipt['inputs'])
    receipt['inputs_unchanged'] = unchanged
    if not unchanged:
        receipt['integrity_error'] = 'A source or read-only pinned input changed or disappeared'
    return unchanged


def last_save(root, game_root):
    pointer = protected_path(root / 'last-session.json')
    if not pointer.is_file():
        raise ValueError('No saved session; start a household and use Save game')
    value = read_json(pointer)
    if not isinstance(value, dict) or type(value.get('schema')) is not int or value['schema'] != 1:
        raise ValueError('The last-session record is invalid or unsupported')
    if value.get('game_root') != str(game_root):
        raise ValueError('The last session belongs to a different installation')
    selected = value.get('save')
    if not isinstance(selected, str) or not selected or '\0' in selected:
        raise ValueError('The last-session record has no valid save path')
    path = Path(selected)
    if not path.is_absolute() or not path.is_relative_to(root):
        raise ValueError('The last-session save must remain inside the session directory')
    protected_path(path)
    relative = path.relative_to(root)
    if len(relative.parts) != 2 or not relative.parts[0].startswith('household-') or relative.parts[1] != 'player-save.json':
        raise ValueError('The last-session save is not a launcher session output')
    before = file_state(path)
    if 'error' in before or before['sha256'] != value.get('sha256'):
        raise ValueError('The last saved session is missing or changed')
    if envelope(path) != value.get('source') or file_state(path) != before:
        raise ValueError('The last-session household is invalid or changed')
    return path, before['sha256'], value['source']


def window_arguments(output, library, game_root, sessions, inputs, run, receipt):
    if output is None:
        if library is not None:
            raise ValueError('--window-session-present-library requires --window-session-output')
        return []
    destination = protected_path(output)
    protected = [game_root, sessions, *inputs]
    if library is not None:
        library = protected_path(library)
        protected.append(library)
    if not destination.is_relative_to(sessions.parent) or any(overlaps(destination, path) for path in protected):
        raise ValueError('Window output must be separate from installation, inputs and sessions, inside cache')
    if destination.exists() or not destination.parent.is_dir():
        raise ValueError('Window output requires a fresh path and an existing real parent directory')
    arguments = ['--window-session-output', str(destination)]
    receipt['window_session_output'] = str(destination)
    if library is not None:
        target = run / 'window-present.dylib'
        pin_input(receipt, 'presentation_library', library, target)
        arguments.extend(['--window-session-present-library', str(target)])
    return arguments


@dataclass(frozen=True, eq=False)
class PreparedLaunch:
    """An in-process plan capability; serializing launch.json cannot recreate it."""

    argv: tuple
    cwd: Path
    receipt_path: Path
    save_output: Path


@dataclass(frozen=True)
class CompletionObservation:
    """Observed completion bound to the exact admitted command and directory."""

    argv: tuple
    cwd: Path
    status: str
    returncode: object
    error: object = None


class TrustedCompletion(ABC):
    """Trust boundary implemented by the parent that retained the execution handle.

    observe() must read the authoritative terminal event for that handle, bind its
    exact admitted argv/cwd, and report unresolved while completion is unknown.
    This interface does not authenticate events or prove a process ran. A real
    parent tool adapter remains an integration prerequisite. Do not implement it
    by reading launch.json, accepting user exit codes, or decoding a result file.
    There is deliberately no CLI or serialized completion entry point.
    """

    @abstractmethod
    def observe(self):
        """Return one CompletionObservation from the retained handle."""
        raise NotImplementedError


class LaunchPreparationError(ValueError):
    """Preparation failed after attempting to retain a failure receipt."""


def _directory_identity(path):
    protected_path(path)
    information = path.stat()
    if not stat.S_ISDIR(information.st_mode):
        raise ValueError('A planned directory is no longer a real directory')
    return (information.st_dev, information.st_ino, information.st_mode)


class _PreparedState:
    def __init__(self, prepared, root, game_root, original, timeout, receipt, receipt_state):
        self.identity = (prepared.argv, prepared.cwd, prepared.receipt_path, prepared.save_output)
        self.root, self.game_root, self.original, self.timeout = root, game_root, original, timeout
        self.receipt_json = json.dumps(receipt, allow_nan=False)
        self.receipt_state = receipt_state
        _check_retained_receipt(prepared.receipt_path, receipt)
        self.directories = {path: _directory_identity(path)
                            for path in (prepared.cwd, root, game_root, prepared.receipt_path.parent)}
        self.command_issued = False
        self.finalized = False
        self.finalization_lock = threading.Lock()
        self.child_claim = None
        self.admission_lock = threading.Lock()


_PREPARED = weakref.WeakKeyDictionary()
_OWNED_CHILD_CLAIMS = []  # Strong roots survive failures and weak-map loss.


def owned_child_execution(*arguments, **keywords):
    """Production stays closed before examining any binding/loader/clock."""
    raise _child_protocol.Unsupported('Production prepared Child admission is disabled')


class _OwnedChildClaim:
    """One exact plan/session admission; neither serialized nor reconstructible.

    Private Python identity is an in-process accidental-misuse boundary, not
    authentication against hostile mutation of module internals. Only a future
    reviewed parent can provide native-image/type authority; no such hook is
    enabled by this source consumer.
    """

    def __init__(self, prepared, state, session):
        self.prepared, self.state, self.session = prepared, state, session
        self.argv, self.cwd = state.identity[:2]
        self.started = False
        self.validated = False

    def __copy__(self):
        raise TypeError('Prepared admission claims cannot be copied')

    def __deepcopy__(self, memo):
        raise TypeError('Prepared admission claims cannot be copied')

    def __reduce_ex__(self, protocol):
        raise TypeError('Prepared admission claims cannot be serialized')

    def _admit(self, session):
        state = self.state
        if not state.admission_lock.acquire(blocking=False):
            raise ValueError('Reentrant prepared admission')
        try:
            if (self.started or session is not self.session or session._claim is not self
                    or state.child_claim is not self
                    or not any(claim is self for claim in _OWNED_CHILD_CLAIMS)):
                raise ValueError('Repeated, copied or cross-session prepared admission')
            # Burn BEFORE revalidation, including any failed/reentrant checks.
            self.started = True
            if _PREPARED.get(self.prepared) is not state or state.finalized:
                raise ValueError('Original prepared admission state was lost or finalized')
            command = _prepared_command(self.prepared, self)
            if tuple(command) != self.argv or self.prepared.cwd != self.cwd:
                raise ValueError('Prepared admission command/directory changed')
            argv = tuple(os.fsencode(item) for item in self.argv)
            cwd = os.fsencode(self.cwd)
            if (not argv or len(argv) > 128 or
                    any(type(item) is not bytes or not 0 < len(item) < 65536 or b'\0' in item
                        for item in (*argv, cwd))):
                raise ValueError('Prepared command does not fit the exact native string bounds')
            self.validated = True
            return argv, cwd
        finally:
            state.admission_lock.release()


def _owned_child_for_injected_mock(prepared, binding, bridge, **options):
    """Explicit UNQUALIFIED fixture entry point, with the actual one-shot claim.

    The binding is retained, never loaded/authenticated here. options supplies
    explicit mock Bridge/Child/TerminalSink types and native byte/budget inputs.
    The returned session cannot authorize healthy TrustedCompletion for life.
    """
    if type(prepared) is not PreparedLaunch or prepared not in _PREPARED:
        raise ValueError('Use the original in-process prepared launch')
    if any(claim.prepared is prepared for claim in _OWNED_CHILD_CLAIMS):
        raise ValueError('This prepared launch already has a retained admission claim')
    state = _PREPARED[prepared]
    if not state.admission_lock.acquire(blocking=False):
        raise ValueError('Prepared admission is already being claimed')
    try:
        if state.child_claim is not None or state.finalized or state.finalization_lock.locked():
            raise ValueError('This prepared launch cannot admit another Child')
        session = _child_protocol.ChildExecution.for_injected_mock(binding, bridge, **options)
        claim = _OwnedChildClaim(prepared, state, session)
        # Session is already rooted; root the exact claim before any I/O or
        # callback in prepared_command. No weak mapping is the sole root.
        _OWNED_CHILD_CLAIMS.append(claim)
        state.child_claim = claim
        session._claim = claim
        return session
    finally:
        state.admission_lock.release()


class _OwnedChildCompletion(TrustedCompletion):
    """Private observer of one claimed session's exact permanent TerminalSink."""

    def __init__(self, claim):
        self._claim = claim

    def observe(self):
        claim = self._claim
        if (type(claim) is not _OwnedChildClaim or claim.state.child_claim is not claim
                or not any(item is claim for item in _OWNED_CHILD_CLAIMS)
                or type(claim.session) is not _child_protocol.ChildExecution
                or claim.session._claim is not claim or not claim.validated):
            raise ValueError('Original admitted PreparedLaunch/session claim required')
        status, returncode, error = claim.session.read_completion()
        if not _child_protocol._native_execution_authorized(claim.session):
            status, returncode, error = ('unresolved', None,
                                        'Injected mock/native authority is unqualified')
        return CompletionObservation(claim.argv, claim.cwd, status, returncode, error)


def _owned_child_completion(prepared, session):
    if type(prepared) is not PreparedLaunch or type(session) is not _child_protocol.ChildExecution:
        raise TypeError('Exact prepared launch and ChildExecution required')
    claim = session._claim
    if (type(claim) is not _OwnedChildClaim or claim.prepared is not prepared
            or claim.session is not session or claim.state.child_claim is not claim
            or not any(item is claim for item in _OWNED_CHILD_CLAIMS)):
        raise ValueError('Completion belongs to a different prepared launch/session')
    return _OwnedChildCompletion(claim)


def _check_plan_identity(prepared, state):
    if (prepared.argv, prepared.cwd, prepared.receipt_path, prepared.save_output) != state.identity:
        raise ValueError('The immutable prepared launch identity changed')
    for path, identity in state.directories.items():
        if _directory_identity(path) != identity:
            raise ValueError('A planned directory changed')


def _check_retained_receipt(path, receipt):
    expected = (json.dumps(receipt, indent=2, allow_nan=False) + '\n').encode('utf-8')
    actual = file_state(path)
    if actual.get('sha256') != hashlib.sha256(expected).hexdigest() or actual.get('links') != 1:
        raise ValueError('The retained launch receipt does not match the planned evidence')


def _prepared_state(prepared):
    if type(prepared) is not PreparedLaunch or prepared not in _PREPARED:
        raise ValueError('Use the original in-process prepared launch')
    state = _PREPARED[prepared]
    if state.finalized:
        raise ValueError('This launch has already been finalized')
    _check_plan_identity(prepared, state)
    if file_state(prepared.receipt_path) != state.receipt_state:
        raise ValueError('The prepared launch receipt changed or disappeared')
    return state


def _separate_output(prepared, state, receipt, fresh):
    output = prepared.save_output
    # Check parent directories here; validate a written output itself below so a
    # bad child save still receives the original final-receipt diagnostics.
    protected_path(output.parent)
    protected = [state.game_root]
    protected.extend(Path(item[key]) for item in receipt['inputs'] for key in ('source', 'pinned'))
    if any(overlaps(output, path) for path in protected):
        raise ValueError('The session save output must be separate from inputs and installation')
    if fresh and (output.exists() or output.is_symlink()):
        raise ValueError('The session save output must remain fresh before launch')
    window = receipt.get('window_session_output')
    if window is not None:
        window = protected_path(window)
        if (not window.is_relative_to(state.root.parent)
                or any(overlaps(window, path) for path in (state.root, *protected))):
            raise ValueError('The window output must remain separate from inputs and sessions')
        if fresh and (window.exists() or not window.parent.is_dir()):
            raise ValueError('The window output must remain fresh before launch')


def prepared_command(prepared):
    """Revalidate immediately before admission; return only the exact pinned argv.

    This function and prepare_launch never create a child. The trusted parent
    must retain the handle for this exact argv and prepared.cwd when admitting it.
    """
    return _prepared_command(prepared)


def _prepared_command(prepared, claim=None):
    state = _prepared_state(prepared)
    if state.child_claim is not None and (claim is not state.child_claim or not claim.started):
        raise ValueError('This command is reserved by its one-shot Child admission')
    receipt = json.loads(state.receipt_json)
    if not check_inputs(receipt):
        raise ValueError(receipt['integrity_error'])
    _separate_output(prepared, state, receipt, fresh=True)
    _prepared_state(prepared)
    state.command_issued = True
    return list(prepared.argv)


def prepare_launch(binary, game_root, household=None, load=None, resume=False, *,
                   timeout=None, window_session_output=None, window_session_present_library=None):
    """Select and pin inputs, reserve fresh output, and retain the exact argv/cwd.

    The launching receipt records intent only. This phase creates no child; the
    parent must revalidate with prepared_command and supervise any admitted run.
    PreparedLaunch and its private validation state must stay in this process.
    """
    if sum((household is not None, load is not None, bool(resume))) > 1:
        raise ValueError('Choose one of --household, --load, or --continue')
    if household is not None and household not in HOUSEHOLDS:
        raise ValueError('Unknown household')
    if timeout is not None and (timeout <= 0 or not float('-inf') < timeout < float('inf')):
        raise ValueError('Timeout must be a positive finite number')
    game_root = Path(game_root).resolve(strict=True)
    root = session_root()
    if not game_root.is_dir() or overlaps(root, game_root):
        raise ValueError('Installation and session directories must be separate real directories')
    run = Path(tempfile.mkdtemp(prefix='household-', dir=root))
    output = run / 'player-save.json'
    receipt_path = run / 'launch.json'
    receipt = {'schema': 1, 'game_root': str(game_root), 'command': [],
               'cwd': str(REPOSITORY), 'inputs': [],
               'binary_sha256': None, 'input': None, 'input_sha256': None,
               'save_output': str(output), 'save_sha256': None,
               'process': {'status': 'not_started', 'returncode': None},
               'exit_code': None, 'performance_qualified': False,
               'validation_scope': 'managed-v2-envelope-and-dialog-client-structure-only',
               'continue_candidate': False}
    atomic_json(receipt_path, receipt)
    original = None
    try:
        binary = protected_path(binary)
        executable = run / 'sims-room'
        receipt['binary_sha256'] = pin_input(receipt, 'executable', binary, executable, executable=True)
        package_receipt = None
        if binary.parent.name == 'MacOS' and binary.parent.parent.name == 'Contents':
            package_receipt = binary.parent.parent / 'Resources/build-receipt.json'
            if package_receipt.exists() or package_receipt.is_symlink():
                target = run / 'build-receipt.json'
                pin_input(receipt, 'build_receipt', package_receipt, target)
                build = read_json(target)
                if (not isinstance(build, dict) or build.get('schema') != 'sims-room-build-receipt-v1'
                        or build.get('executable_sha256') != receipt['binary_sha256']):
                    raise ValueError('The package build receipt does not identify this executable')
                receipt['build_receipt'] = build
        if resume:
            load, expected_sha256, expected_source = last_save(root, game_root)
            receipt['continue_identity'] = {'sha256': expected_sha256, 'source': expected_source}
        if load is not None:
            load = protected_path(load)
            receipt['input'] = str(load)
            starting = run / 'starting-save.json'
            receipt['input_sha256'] = pin_input(receipt, 'selected_save', load, starting)
            original = envelope(starting)
            if resume and (receipt['input_sha256'] != expected_sha256 or original != expected_source):
                raise ValueError('The pinned save no longer matches the verified last-session identity')
        else:
            original = HOUSEHOLDS[household or 'newbie']
        protected = [binary]
        if load is not None:
            protected.append(load)
        if package_receipt is not None:
            protected.append(package_receipt)
        command = [str(executable), '--original', original, '--root', str(game_root),
                   '--save', str(output), '--paused']
        if load is not None:
            command.extend(['--load', str(starting)])
        command.extend(window_arguments(window_session_output, window_session_present_library,
                                        game_root, root, protected, run, receipt))
        if output.exists() or output.is_symlink() or any(overlaps(output, path) for path in (game_root, *protected)):
            raise ValueError('The session save output must be fresh and separate from inputs and installation')
        receipt['save_output_before'] = {'exists': False}
        receipt['source'] = original
        receipt['command'] = command
        receipt['process']['status'] = 'launching'
        atomic_json(receipt_path, receipt)
        prepared = PreparedLaunch(tuple(command), REPOSITORY, receipt_path, output)
        _PREPARED[prepared] = _PreparedState(prepared, root, game_root, original, timeout,
                                            receipt, file_state(receipt_path))
        return prepared
    except (OSError, ValueError, TypeError) as error:
        receipt['launch_error'] = str(error)
        receipt['save_output_after'] = file_state(output)
        check_inputs(receipt)
        receipt['launcher_exit_code'] = 1
        try:
            atomic_json(receipt_path, receipt)
        except (OSError, ValueError, TypeError) as retention_error:
            print('Cannot retain final launch receipt: ' + str(retention_error), file=sys.stderr)
        raise LaunchPreparationError(str(error)) from error


def finalize_launch(prepared, completion):
    """Finalize through one retained-handle observer at a time.

    Concurrent or reentrant finalizers are rejected. Invalid observations release
    the lock without consuming the plan; an accepted finalization, including an
    unresolved/error outcome, remains single-use even if retaining evidence fails.
    A claimed Child's unresolved observation instead retains its original plan
    and receipt for later incremental recovery; it never publishes Continue.
    """
    state = _prepared_state(prepared)
    if not state.finalization_lock.acquire(blocking=False):
        raise ValueError('This launch is already being finalized')
    try:
        return _finalize_launch(prepared, completion)
    finally:
        state.finalization_lock.release()


def _finalize_launch(prepared, completion):
    """Finalize once, using the trusted parent's retained-handle observation.

    Python callers are inside this trust boundary. Subclassing TrustedCompletion
    or constructing an observation is not proof of execution; the parent adapter
    must supply that authority. No JSON or exit-code-only input is accepted.
    """
    state = _prepared_state(prepared)
    if not state.command_issued:
        raise ValueError('Revalidate the prepared command before admitting execution')
    if not isinstance(completion, TrustedCompletion):
        raise TypeError('A trusted in-process completion adapter is required')
    if state.child_claim is not None and (type(completion) is not _OwnedChildCompletion or
                                         completion._claim is not state.child_claim):
        raise TypeError('This prepared launch requires its exact Child completion adapter')
    observation = completion.observe()
    if (type(observation) is not CompletionObservation
            or type(observation.argv) is not tuple or observation.argv != prepared.argv
            or observation.cwd != prepared.cwd):
        raise ValueError('Completion does not bind the exact admitted argv and cwd')
    if (observation.status not in ('exited', 'unresolved', 'timeout', 'launch_error',
                                   'diagnostic_failed', 'interrupted', 'launching')
            or (observation.returncode is not None and type(observation.returncode) is not int)):
        raise ValueError('Completion has an invalid process status or return code')
    # Observing the trusted handle may itself take time; detect receipt and plan
    # changes again before consuming the capability or writing final evidence.
    state = _prepared_state(prepared)
    if state.child_claim is not None and (type(completion) is not _OwnedChildCompletion or
                                         completion._claim is not state.child_claim):
        raise TypeError('Prepared admission changed during completion observation')
    if state.child_claim is not None and observation.status == 'unresolved':
        # Keep the claim and original receipt intact for later incremental native
        # recovery. An unresolved/mock completion cannot consume or publish it.
        return 1
    receipt = json.loads(state.receipt_json)
    _separate_output(prepared, state, receipt, fresh=False)
    state.finalized = True
    output, receipt_path = prepared.save_output, prepared.receipt_path
    root, game_root, original = state.root, state.game_root, state.original
    candidate = None
    receipt['exit_code'] = observation.returncode
    receipt['process'] = {'status': observation.status, 'returncode': observation.returncode}
    if observation.error is not None:
        receipt['process']['error'] = str(observation.error)
    code = (observation.returncode if observation.status == 'exited' and observation.returncode is not None
            else 124 if observation.status == 'timeout'
            else 130 if observation.status == 'interrupted' else 1)
    if isinstance(completion, _LegacyCompletion):
        if completion.diagnostic is not None:
            receipt['diagnostic'] = completion.diagnostic
            code = completion.diagnostic['exit_code']
        if completion.launch_error is not None:
            receipt['launch_error'] = completion.launch_error
    if receipt['process']['status'] == 'exited' and code == 0 and observation.returncode == 0:
        if output.exists() or output.is_symlink():
            try:
                before = file_state(output)
                if 'error' in before:
                    raise ValueError(before['error'])
                if before['links'] != 1:
                    raise ValueError('The written save must not be a hard-linked input or existing save')
                if envelope(output) != original or file_state(output) != before:
                    raise ValueError('The written save has a different household or changed during validation')
                receipt['save_sha256'] = before['sha256']
                receipt['save_validation'] = 'structurally_valid_not_runtime_restore_verified'
                candidate = {'schema': 1, 'game_root': str(game_root), 'save': str(output),
                             'sha256': before['sha256'], 'source': original}
            except (OSError, ValueError, TypeError) as error:
                receipt['save_error'] = str(error)
                code = 1
                print('Save could not become Continue Last Session: ' + str(error), file=sys.stderr)
        else:
            receipt['save_validation'] = 'no_save_written'
    receipt['save_output_after'] = file_state(output)
    if not check_inputs(receipt):
        candidate = None
        print('Launch input integrity failed; the previous Continue save was preserved.', file=sys.stderr)
        if code == 0:
            code = 1
    receipt['continue_candidate'] = candidate is not None
    receipt['launcher_exit_code'] = code
    try:
        _check_plan_identity(prepared, state)
        if file_state(receipt_path) != state.receipt_state:
            raise ValueError('The prepared launch receipt changed during finalization')
        atomic_json(receipt_path, receipt)
    except (OSError, ValueError, TypeError) as error:
        print('Cannot retain final launch receipt; Continue was not updated: ' + str(error), file=sys.stderr)
        return code if code else 1
    if candidate is not None:
        try:
            _check_plan_identity(prepared, state)
            _check_retained_receipt(receipt_path, receipt)
            if not check_inputs(receipt):
                raise ValueError(receipt['integrity_error'])
            if file_state(output) != before:
                raise ValueError('The save changed before Continue publication')
            atomic_json(root / 'last-session.json', candidate)
        except (OSError, ValueError, TypeError) as error:
            receipt['publication_error'] = str(error)
            receipt['continue_candidate'] = False
            receipt['save_output_after'] = file_state(output)
            receipt['launcher_exit_code'] = 1
            print('Continue was not updated: ' + str(error), file=sys.stderr)
            try:
                atomic_json(receipt_path, receipt)
            except (OSError, ValueError, TypeError):
                pass
            return 1
    return code


class _LegacyCompletion(TrustedCompletion):
    """Private adapter for the existing blocking subprocess/child_runner path."""

    def __init__(self, observation, diagnostic=None, launch_error=None):
        self.observation = observation
        self.diagnostic = diagnostic
        self.launch_error = launch_error

    def observe(self):
        return self.observation


def launch(binary, game_root, household=None, load=None, resume=False, *,
           timeout=None, window_session_output=None, window_session_present_library=None,
           child_runner=None):
    """Legacy blocking wrapper; the CLI continues to use this subprocess path."""
    try:
        prepared = prepare_launch(binary, game_root, household, load, resume,
                                  timeout=timeout, window_session_output=window_session_output,
                                  window_session_present_library=window_session_present_library)
    except LaunchPreparationError as error:
        print('Unable to open household: ' + str(error), file=sys.stderr)
        return 1
    state = _PREPARED[prepared]
    diagnostic, launch_error = None, None
    status, returncode, error_message = 'launching', None, None
    try:
        command = prepared_command(prepared)
        print('Opening household paused. Strict startup errors are not bypassed.', flush=True)
        print('Use Save game to retain progress. Session save: ' + str(prepared.save_output), flush=True)
        try:
            if child_runner is None:
                result = subprocess.run(command, cwd=prepared.cwd, check=False, timeout=timeout)
            else:
                result = child_runner(command, cwd=prepared.cwd, check=False, timeout=timeout,
                                      launch_receipt=json.loads(state.receipt_json))
            returncode, status = result.returncode, 'exited'
            if child_runner is not None:
                diagnostic = result.diagnostic
                status = diagnostic['status']
        except subprocess.TimeoutExpired as error:
            status, error_message = 'timeout', str(error)
            print('Household timed out: ' + str(error), file=sys.stderr)
        except OSError as error:
            status, error_message = 'launch_error', str(error)
            print('Unable to start sims-room: ' + str(error), file=sys.stderr)
    except (OSError, ValueError, TypeError) as error:
        launch_error = str(error)
        print('Unable to open household: ' + str(error), file=sys.stderr)
        # A failed pre-admission check can still retain integrity diagnostics;
        # the private legacy adapter cannot authorize Continue in this state.
        state.command_issued = True
    observation = CompletionObservation(prepared.argv, prepared.cwd, status, returncode, error_message)
    return finalize_launch(prepared, _LegacyCompletion(observation, diagnostic, launch_error))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=REPOSITORY / 'target/release/sims-room')
    parser.add_argument('--root', type=Path, default=REPOSITORY / 'game-data/cabinet-extracted')
    choices = parser.add_mutually_exclusive_group()
    choices.add_argument('--household', choices=HOUSEHOLDS)
    choices.add_argument('--continue', dest='resume', action='store_true', help='Continue Last Session')
    choices.add_argument('--load', type=Path, help='Load a version-2 managed player save')
    parser.add_argument('--timeout', type=float, help='Optional child timeout in seconds')
    parser.add_argument('--window-session-output', help='Fresh observation path inside cache, outside alpha-sessions')
    parser.add_argument('--window-session-present-library', help='Existing presentation library, pinned read-only')
    arguments = parser.parse_args()
    try:
        return launch(arguments.binary, arguments.root, arguments.household, arguments.load, arguments.resume,
                      timeout=arguments.timeout, window_session_output=arguments.window_session_output,
                      window_session_present_library=arguments.window_session_present_library)
    except (OSError, ValueError, TypeError) as error:
        print('Unable to open household: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
