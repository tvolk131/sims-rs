"""Explicit opt-in Strict camera or one offered tutorial response; diagnostic only."""
import argparse
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import shutil
import stat
import subprocess
import time
from types import SimpleNamespace

import play_alpha
import window_evidence as evidence
import window_journal_observation as journal_observation


RESERVE_BYTES = 6 * 1024**3
SOURCE = Path(__file__).resolve().parent
CAMERA_EXPORT_CONTRACT = 'sims-window-camera-control-raw-v2'
SCHEMA_PATHS = (
    'crates/sims-app/src/window_session.rs',
    'crates/sims-app/src/window_session/presentation.rs',
    'crates/sims-app/src/original_viewer/ui_publication.rs',
    'crates/sims-app/src/original_viewer/ui_publication/view.rs',
)


def reserve(directory):
    evidence.require(shutil.disk_usage(directory).free >= RESERVE_BYTES, 'Six GiB free reserve required')


def regular(path):
    path = play_alpha.protected_path(path)
    info = path.lstat()
    evidence.require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1,
                     'Expected a non-linked regular file: ' + str(path))
    state = play_alpha.file_state(path)
    evidence.require('error' not in state and state['links'] == 1, 'Unstable regular file: ' + str(path))
    return path, state


def private_directory(destination, protected):
    destination = play_alpha.protected_path(destination)
    cache = play_alpha.protected_path(play_alpha.REPOSITORY / 'cache')
    evidence.require(destination.is_relative_to(cache) and destination != cache,
                     'Diagnostic output must be beneath this checkout cache')
    evidence.require(not destination.exists() and destination.parent.is_dir(),
                     'Diagnostic output requires a fresh path and an existing real parent')
    for item in protected:
        item = play_alpha.protected_path(item)
        evidence.require(not play_alpha.overlaps(destination, item), 'Diagnostic output overlaps a protected input')
    reserve(destination.parent)
    destination.mkdir(mode=0o700)
    return destination


def write_new(path, value):
    path = play_alpha.protected_path(path)
    with path.open('x', encoding='utf-8') as stream:
        os.chmod(path, 0o600)
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


class Pins:
    def __init__(self, directory):
        self.directory = directory
        self.items = []

    def add(self, name, source, executable=False):
        evidence.require(Path(name).name == name and name not in ('.', '..'), 'Unsafe pinned filename')
        source, before = regular(source)
        target = self.directory / name
        with source.open('rb') as reader, target.open('xb') as writer:
            shutil.copyfileobj(reader, writer)
        target.chmod(0o500 if executable else 0o400)
        pinned = regular(target)[1]
        evidence.require(before == regular(source)[1] and before['sha256'] == pinned['sha256'],
                         'Input changed during pinning: ' + name)
        self.items.append({'name': name, 'source': str(source), 'pinned': str(target),
                           'before': before, 'pinned_before': pinned})
        return target

    def check(self):
        for item in self.items:
            item['after'] = play_alpha.file_state(Path(item['source']))
            item['pinned_after'] = play_alpha.file_state(Path(item['pinned']))
            item['unchanged'] = (item['before'] == item['after']
                                 and item['pinned_before'] == item['pinned_after'])
        return bool(self.items) and all(item['unchanged'] for item in self.items)


def helper_receipt(receipt, helper, swift_source):
    receipt, before = regular(receipt)
    value = play_alpha.read_json(receipt)
    evidence.require(regular(receipt)[1] == before, 'Helper build receipt changed while reading')
    evidence.require(isinstance(value, dict) and set(value) == {
        'schema', 'source_sha256', 'executable_sha256', 'source_path', 'binary_path',
        'compiler', 'command'}, 'An explicit helper build receipt is required')
    evidence.require(value['schema'] == 'sims-window-helper-build-v1'
                     and value['source_sha256'] == regular(swift_source)[1]['sha256']
                     and value['executable_sha256'] == regular(helper)[1]['sha256'],
                     'Helper binary is not bound to the authoritative Swift source')
    command = value['command']
    evidence.require(isinstance(value['compiler'], str) and bool(value['compiler'].strip())
                     and isinstance(command, list) and all(isinstance(item, str) for item in command)
                     and len(command) >= 4 and Path(command[0]).name == 'swiftc'
                     and value['source_path'] == str(swift_source)
                     and value['binary_path'] == str(helper)
                     and command.count(str(swift_source)) == 1 and command.count('-o') == 1
                     and command.index('-o') + 1 < len(command)
                     and command[command.index('-o') + 1] == str(helper),
                     'Helper build receipt lacks exact source/output/compiler invocation')
    return value


def strict_selected_save(command, launch_receipt):
    evidence.require(play_alpha.check_inputs(launch_receipt), 'Launcher inputs changed before native launch')
    evidence.require(all(item['before']['links'] == 1 and item['pinned_before']['links'] == 1
                         for item in launch_receipt['inputs']), 'Linked launcher inputs are unsupported')
    evidence.require('--window-session-present-library' not in command, 'Unbound presentation observer is unsupported')
    executable = next(item for item in launch_receipt['inputs'] if item['kind'] == 'executable')
    evidence.require(command[0] == executable['pinned'], 'Child must use the launcher-pinned executable')
    if '--load' in command:
        selected = Path(command[command.index('--load') + 1])
        item = next(item for item in launch_receipt['inputs'] if item['kind'] == 'selected_save')
        evidence.require(str(selected) == item['pinned'], 'Strict check must use selected pinned save')
        value = play_alpha.read_json(selected)
        options = value['session'].get('options')
        evidence.require(isinstance(options, dict) and options.get('routing') == 'Strict',
                         'Native diagnostic requires actual session.options.routing == Strict; no rewriting')
        evidence.require(play_alpha.file_state(selected) == item['pinned_before'],
                         'Selected pinned save changed during Strict validation')
    evidence.require(command[command.index('--original') + 1] == play_alpha.HOUSEHOLDS['newbie'],
                     'This bounded diagnostic only supports paused Newbie')


def validate_kernel_identity(value, pid, executable, previous=None):
    evidence.require(evidence.integer(pid, 1) and pid <= 2**31 - 1
                     and isinstance(value, dict) and set(value) == {
                         'source', 'pid', 'start_tvsec', 'start_tvusec', 'executable'}
                     and value.get('source') == 'proc_pidinfo(PROC_PIDTBSDINFO)+proc_pidpath'
                     and type(value.get('pid')) is int and value['pid'] == pid
                     and evidence.integer(value.get('start_tvsec'), 1) and value['start_tvsec'] <= 2**64 - 1
                     and evidence.integer(value.get('start_tvusec')) and value['start_tvusec'] < 1_000_000
                     and isinstance(value.get('executable'), str) and value['executable'].startswith('/')
                     and '\0' not in value['executable'] and value['executable'] == str(executable),
                     'Invalid or foreign kernel process identity')
    evidence.require(previous is None or value == previous, 'Kernel PID lifetime or executable changed')
    return value


def validate_launch_services(value, executable):
    evidence.require(isinstance(value, dict) and set(value) <= {'registered', 'executable', 'launch_date'}
                     and type(value.get('registered')) is bool
                     and (value['registered'] or set(value) == {'registered'}),
                     'Malformed optional LaunchServices diagnostics')
    if 'executable' in value:
        evidence.require(value['executable'] == str(executable),
                         'LaunchServices executable disagrees with kernel identity')
    if 'launch_date' in value:
        evidence.require(isinstance(value['launch_date'], str) and bool(value['launch_date'])
                         and '\0' not in value['launch_date'], 'Malformed optional launch date diagnostic')


def validate_binding(value, pid, executable, previous=None):
    evidence.require(isinstance(value, dict) and set(value) == {
                         'schema', 'pid', 'window_id', 'executable', 'kernel_identity',
                         'frontmost', 'on_screen', 'display_scope', 'displays', 'bounds'}
                     and value.get('schema') == 'sims-window-binding-v2'
                     and type(value.get('pid')) is int and value['pid'] == pid
                     and evidence.integer(value.get('window_id'), 1) and value['window_id'] <= 2**32 - 1
                     and value.get('executable') == str(executable)
                     and value.get('frontmost') is True and value.get('on_screen') is True,
                     'Invalid, unfocused or wrong-PID window binding')
    validate_kernel_identity(value.get('kernel_identity'), pid, executable,
                             previous.get('kernel_identity') if previous is not None else None)
    bounds = value.get('bounds')
    evidence.require(isinstance(bounds, list) and len(bounds) == 4
                     and all(type(number) in (int, float) and math.isfinite(number) for number in bounds)
                     and bounds[2] > 100 and bounds[3] > 100, 'Invalid window geometry')
    evidence.validate_displays(value)
    evidence.require(previous is None or value == previous, 'PID lifetime, window or geometry changed')
    return value


def validate_post(value, binding, action=None):
    action = {'kind': 'key', 'code': 124, 'press_ms': 20} if action is None else action
    evidence.expected_receipts(action)
    evidence.require(isinstance(binding, dict), 'Missing owned post binding')
    validate_binding(binding, binding.get('pid'), binding.get('executable'))
    evidence.require(isinstance(value, dict), 'Malformed native post receipt')
    validate_binding(value.get('binding'), binding['pid'], binding['executable'], binding)
    evidence.require(value.get('binding') == binding
                     and value.get('operation') == 'key'
                     and type(value.get('code')) is int and value['code'] == action['code']
                     and type(value.get('press_ms')) is int and value['press_ms'] == 20
                     and value.get('released') is True,
                     'Helper did not acknowledge exact bound key and release')
    posts = value.get('posts')
    evidence.require(isinstance(posts, list) and len(posts) == 2, 'Missing native post timestamps')
    previous = 0
    for post, down in zip(posts, (True, False)):
        evidence.require(isinstance(post, dict) and post.get('down') is down
                         and all(evidence.integer(post.get(name), 1) for name in
                                 ('unix_before_ns', 'unix_after_ns', 'host_before_ns', 'host_after_ns'))
                         and previous <= post['host_before_ns'] <= post['host_after_ns']
                         and post['unix_before_ns'] <= post['unix_after_ns'], 'Invalid native post bracket')
        previous = post['host_after_ns']
    evidence.require(posts[0]['unix_after_ns'] <= posts[1]['unix_before_ns'], 'Regressing native post clocks')
    lower = (posts[1]['host_before_ns'] - posts[0]['host_after_ns']) / 1000
    upper = (posts[1]['host_after_ns'] - posts[0]['host_before_ns']) / 1000
    evidence.require(0 <= lower <= upper <= 1_000_000, 'Unbounded native press interval')
    value['press_duration_us'] = {'lower': lower, 'upper': upper,
        'clock': 'DispatchTime uptime nanoseconds; post-call brackets, not app receipt or presentation time'}
    return value


def targeted_window_request(expected_identity, pid, executable, generation,
                            candidate, publication, operation='observe', *, binding=None, code=None):
    """Construct data only; observations do not grant retained-execution authority.

    The future qualified execution adapter must supply its expected lifetime and
    a current Host publication alongside the matching durable candidate row.
    A historical row alone is insufficient. This function never starts a child,
    invokes a helper, or promotes the still-disabled legacy execution backend.
    """
    validate_kernel_identity(expected_identity, pid, executable)
    evidence.require(evidence.integer(generation) and generation <= 2**64 - 1,
                     'Invalid current owner generation')
    scope = 'observational candidate; independent retained-execution and exact-window verification required'
    evidence.require(isinstance(candidate, dict) and set(candidate) == {
                         'schema', 'window_id', 'generation', 'pid', 'scope'}
                     and candidate.get('schema') == 'sims-owned-window-candidate-v1'
                     and type(candidate.get('pid')) is int and candidate['pid'] == pid
                     and type(candidate.get('generation')) is int and candidate['generation'] == generation
                     and evidence.integer(candidate.get('window_id'), 1) and candidate['window_id'] <= 2**32 - 1
                     and candidate.get('scope') == scope,
                     'Invalid or foreign durable window candidate')
    evidence.require(isinstance(publication, dict) and set(publication) == {
                         'status', 'generation', 'window_number', 'scope'}
                     and publication.get('status') == 'DURABLY_RECORDED'
                     and type(publication.get('generation')) is int and publication['generation'] == generation
                     and type(publication.get('window_number')) is int
                     and publication['window_number'] == candidate['window_id']
                     and publication.get('scope') == scope,
                     'Current Host has not acknowledged this exact window candidate')
    evidence.require(operation in ('observe', 'key', 'release', 'close'), 'Unknown targeted operation')
    target = {'schema': 'sims-window-target-v1', 'window_id': candidate['window_id'],
              'kernel_identity': copy.deepcopy(expected_identity)}
    request = {'operation': operation, 'pid': pid, 'executable': str(executable), 'target': target}
    if binding is None:
        evidence.require(operation == 'observe', 'Input requires an existing exact-window binding')
    else:
        validate_binding(binding, pid, executable)
        validate_kernel_identity(binding['kernel_identity'], pid, executable, expected_identity)
        evidence.require(binding['window_id'] == target['window_id'], 'Binding names another owned window')
        request['binding'] = copy.deepcopy(binding)
    if operation in ('key', 'release'):
        evidence.require(type(code) is int and code in (124, 36), 'Only ArrowRight or Enter is supported')
        request['code'] = code
        if operation == 'key':
            request['press_ms'] = 20
    else:
        evidence.require(code is None, 'Unexpected key code for targeted observation or close')
    return request


def validate_targeted_response(value, request):
    """Check a helper response against the exact immutable request observations.

    No native verification or execution qualification follows from this decoder.
    A future adapter must retain execution and current generation independently.
    """
    evidence.require(isinstance(value, dict) and isinstance(request, dict), 'Malformed targeted helper exchange')
    operation = request.get('operation')
    evidence.require(operation in ('observe', 'key', 'release', 'close'), 'Unknown targeted operation')
    keys = {'operation', 'pid', 'executable', 'target'}
    if 'binding' in request:
        keys.add('binding')
    evidence.require(operation == 'observe' or 'binding' in request, 'Input requires an existing exact-window binding')
    if operation in ('key', 'release'):
        keys.add('code')
        evidence.require(type(request.get('code')) is int and request['code'] in (124, 36), 'Invalid targeted key')
    if operation == 'key':
        keys.add('press_ms')
        evidence.require(type(request.get('press_ms')) is int and request['press_ms'] == 20, 'Unbounded targeted key')
    evidence.require(set(request) == keys, 'Malformed targeted request fields')
    target, pid, executable = request['target'], request['pid'], request['executable']
    evidence.require(isinstance(target, dict) and set(target) == {'schema', 'window_id', 'kernel_identity'}
                     and target.get('schema') == 'sims-window-target-v1'
                     and evidence.integer(target.get('window_id'), 1) and target['window_id'] <= 2**32 - 1,
                     'Malformed exact-window target')
    expected = target['kernel_identity']
    validate_kernel_identity(expected, pid, executable)
    if 'binding' in request:
        prior = validate_binding(request['binding'], pid, executable)
        validate_kernel_identity(prior['kernel_identity'], pid, executable, expected)
        evidence.require(prior['window_id'] == target['window_id'], 'Request binding names another owned window')
    if value.get('status') == 'not_ready':
        evidence.require(request['operation'] == 'observe' and 'binding' not in request
                         and set(value) == {'status', 'reason', 'kernel_identity'}
                         and value.get('reason') in ('owned_window_unavailable', 'foreground_application_unavailable'),
                         'Target disappeared or returned a malformed readiness response')
        validate_kernel_identity(value['kernel_identity'], pid, executable, expected)
        return None
    evidence.require('status' not in value and value.get('operation') == request['operation'],
                     'Mismatched targeted helper operation')
    evidence.require('error' not in value and 'release_error' not in value,
                     'Targeted helper reported a failure')
    current = validate_binding(value.get('binding'), pid, executable, request.get('binding'))
    validate_kernel_identity(current['kernel_identity'], pid, executable, expected)
    evidence.require(current['window_id'] == target['window_id'], 'Helper returned another owned window')
    validate_launch_services(value.get('launch_services'), executable)
    if request['operation'] == 'key':
        validate_post(copy.deepcopy(value), current,
                      {'kind': 'key', 'code': request['code'], 'press_ms': request['press_ms']})
    elif request['operation'] == 'release':
        evidence.require(type(value.get('code')) is int and value['code'] == request['code']
                         and value.get('released') is True, 'Targeted release did not match its request')
    return copy.deepcopy(value)


class NativeBackend:
    def start(self, command, cwd, stdout, stderr):
        raise RuntimeError('Native window launch disabled pending the retained-execution adapter')

    def helper(self, executable, request, timeout):
        raise RuntimeError('Native window helper disabled pending the retained-execution adapter')


class DiagnosticRunner:
    def __init__(self, directory, pins, helper, plan, *, backend=None, clock=time, started=None):
        self.directory, self.pins, self.helper, self.plan = directory, pins, helper, plan
        self.backend, self.clock = backend or NativeBackend(), clock
        self.started = clock.monotonic() if started is None else started
        self.deadline = self.started + plan['timeout_seconds']
        self.active_deadline = self.deadline - 8
        self.child, self.binding, self.command = None, None, None
        self.kernel_identity = None
        self.native_log, self.cleanup_log = [], []
        self.payload = b''
        self.journal_identity = None
        self.journal_size = 0
        self.journal_on_disk = {'status': 'uninspected', 'sha256': None}
        self.journal_inspection_errors, self.journal_read_errors = [], []
        self.control, self.post, self.records = None, None, []
        self.tutorial = plan.get('mode') == evidence.TUTORIAL_MODE
        self.response, self.reservation = None, None
        self.reservation_history_failure = None
        self.key_attempted = False
        self.failure, self.primary_status, self.forced = None, None, False
        self.cleanup_done = False
        self.fence = 1
        self.camera_capture, self.raw_key_result = {}, None
        self.close_started_unix_ns = None
        self.final_observer = None
        self.final_observation, self.final_raw, self.final_records = None, b'', None
        self.final_child_collected = False

    def remaining(self, cleanup=False):
        remaining = (self.deadline if cleanup else self.active_deadline) - self.clock.monotonic()
        if remaining <= 0:
            raise TimeoutError('Bounded native diagnostic deadline exceeded')
        return remaining

    def check_opening_history(self, records, started_unix_ns=None):
        try:
            baseline = evidence.offered_history(records, self.reservation, self.fence, started_unix_ns)
            if started_unix_ns is not None:
                evidence.require(baseline == self.reservation.get('pre_post_frame', self.reservation['frame']),
                                 'Reserved opening is not the immediately preceding observed draw')
            return baseline
        except ValueError as error:
            if self.reservation_history_failure is None:
                self.reservation_history_failure = str(error)
            raise

    def native(self, operation, cleanup=False):
        evidence.require(self.child is not None and self.child.poll() is None,
                         'Owned child exited; refusing stale PID operation')
        evidence.require(operation == 'observe' or self.binding is not None,
                         'Input and window actions require an existing owned window binding')
        request = {'operation': operation, 'pid': self.child.pid, 'executable': self.command[0]}
        if self.binding is not None:
            request['binding'] = self.binding
        if operation == 'key':
            evidence.require(not self.key_attempted, 'Diagnostic key may never be resubmitted')
            self.key_attempted = True
            request.update({'code': self.plan['steps'][0]['input']['code'], 'press_ms': 20})
        elif operation == 'release' and self.tutorial:
            request['code'] = 36
        entry = {'request': request, 'started_unix_ns': self.clock.time_ns()}
        self.native_log.append(entry)
        try:
            result = self.backend.helper(self.helper, request, min(2, self.remaining(cleanup)))
            entry.update({'returncode': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr})
            evidence.require(result.returncode == 0, 'Native helper failed: ' + result.stderr)
            value = evidence.parse_json(result.stdout)
            if operation == 'key':
                self.raw_key_result = copy.deepcopy(value)
            evidence.require(isinstance(value, dict), 'Malformed native helper response')
            if value.get('status') == 'not_ready':
                evidence.require(operation == 'observe' and self.binding is None, 'Target window disappeared')
                evidence.require(set(value) == {'status', 'reason', 'kernel_identity'}
                                 and value.get('reason') in ('owned_window_unavailable',
                                                            'foreground_application_unavailable'),
                                 'Invalid initial window readiness receipt')
                self.kernel_identity = validate_kernel_identity(value.get('kernel_identity'), self.child.pid,
                                                               self.command[0], self.kernel_identity)
                return None
            evidence.require('status' not in value, 'Unexpected native helper status')
            if 'launch_services' in value:
                validate_launch_services(value['launch_services'], self.command[0])
            binding = validate_binding(value.get('binding'), self.child.pid, self.command[0], self.binding)
            self.kernel_identity = validate_kernel_identity(binding['kernel_identity'], self.child.pid,
                                                           self.command[0], self.kernel_identity)
            self.binding = binding
            if operation == 'key':
                validate_post(value, binding, self.plan['steps'][0]['input'])
            else:
                evidence.require(value.get('operation') == operation, 'Mismatched native operation receipt')
                if operation == 'release' and self.tutorial:
                    evidence.require(type(value.get('code')) is int and value['code'] == 36
                                     and value.get('released') is True,
                                     'Cleanup release did not match Enter')
            return value
        except (Exception, KeyboardInterrupt) as error:
            entry['error'] = str(error) or type(error).__name__
            if isinstance(error, subprocess.TimeoutExpired):
                entry.update({'partial_stdout': str(error.stdout or ''), 'partial_stderr': str(error.stderr or '')})
            raise
        finally:
            entry['finished_unix_ns'] = self.clock.time_ns()

    def inspect_journal(self, complete=False):
        path = self.telemetry / 'events.jsonl'
        inspection = {'path': str(path), 'status': 'inspecting', 'sha256': None,
                      'scope': 'On-disk path/stat inspection only; no file hash computed'}
        self.journal_on_disk = inspection
        try:
            play_alpha.protected_path(path)
            try:
                before = path.lstat()
            except FileNotFoundError:
                inspection['status'] = 'missing'
                evidence.require(self.journal_identity is None and not self.payload and not complete,
                                 'Journal disappeared')
                return None
            inspection['stat'] = {'device': before.st_dev, 'inode': before.st_ino, 'mode': before.st_mode,
                                  'links': before.st_nlink, 'bytes': before.st_size}
            evidence.require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1
                             and before.st_size <= evidence.MAX_JOURNAL_BYTES, 'Unsafe or oversized journal')
            identity = (before.st_dev, before.st_ino)
            evidence.require(self.journal_identity is None or self.journal_identity == identity,
                             'Journal file identity changed')
            evidence.require(before.st_size >= self.journal_size, 'Journal truncated since previous inspection')
            self.journal_identity, self.journal_size = identity, before.st_size
            if self.final_observer is not None:
                self.final_observer.remember()
            inspection['status'] = 'inspected'
            return before
        except Exception as error:
            inspection.update({'status': 'error', 'error': str(error) or type(error).__name__})
            self.journal_inspection_errors.append(inspection)
            raise

    def read_records(self, complete=False):
        try:
            if complete:
                evidence.require(self.final_observer is not None, 'Final descriptor observer unavailable')
                self.final_observation, self.final_raw = self.final_observer.observe(
                    child_collected=self.final_child_collected, forced_teardown=self.forced,
                    retained=self.payload)
                self.final_records = self.final_observer.parse(self.final_observation, self.final_raw,
                    lambda raw: evidence.journal(raw, self.child.pid, complete=True))
                self.final_observer.snapshot(self.final_observation, self.final_raw)
                evidence.require(self.final_observation['complete'],
                                 self.final_observation['error'] or 'Final journal observation incomplete')
                evidence.require(self.final_records is not None,
                                 self.final_observation['semantic']['error'] or 'Final journal parse failed')
                evidence.require(self.final_observation['snapshot']['status'] == 'written',
                                 'Final journal artifact: ' + str(self.final_observation['snapshot']['error']))
                return self.final_records
            before = self.inspect_journal(complete)
            if before is None:
                return []
            path = self.telemetry / 'events.jsonl'
            with path.open('rb') as stream:
                opened = os.fstat(stream.fileno())
                evidence.require((opened.st_dev, opened.st_ino) == (before.st_dev, before.st_ino)
                                 and stat.S_ISREG(opened.st_mode) and opened.st_nlink == 1
                                 and before.st_size <= opened.st_size <= evidence.MAX_JOURNAL_BYTES,
                                 'Journal identity or size raced')
                payload = stream.read(evidence.MAX_JOURNAL_BYTES + 1)
            evidence.require(len(payload) <= evidence.MAX_JOURNAL_BYTES, 'Journal exceeds byte bound')
            evidence.require(payload.startswith(self.payload), 'Journal truncated or replaced its previous bytes')
            records = evidence.journal(payload, self.child.pid, complete=complete)
            self.payload = payload
            self.records = records
            return self.records
        except Exception as error:
            self.journal_read_errors.append({'error': str(error) or type(error).__name__, 'complete': complete,
                                             'retained_buffer_bytes': len(self.payload)})
            raise

    def wait_for(self, predicate):
        while True:
            self.remaining()
            evidence.require(self.child.poll() is None, 'Owned child exited before diagnostic completed')
            reserve(self.directory)
            result = predicate(self.read_records())
            if result:
                return result
            self.clock.sleep(min(.05, self.remaining()))

    def observe_camera(self):
        while self.binding is None:
            self.remaining()
            self.inspect_journal()
            self.native('observe')
            self.inspect_journal()
            if self.binding is None:
                self.clock.sleep(min(.05, self.remaining()))
        self.wait_for(lambda records: bool(records))
        fence_file = play_alpha.protected_path(self.telemetry / 'observer-fence')
        with fence_file.open('xb') as stream:
            stream.write(self.fence.to_bytes(8, 'little'))
        def baseline(records):
            frames = [record['data'] for record in records if record['kind'] == 'frame']
            if not frames or frames[-1]['fence'] != self.fence:
                return None
            frame = frames[-1]
            if self.tutorial:
                evidence.offered_opening(frame['state']['value'])
            elif not evidence.usable_camera(frame['state']['value']):
                return None
            size = frame['state']['value'].get('display_size')
            bounds = self.binding['bounds']
            evidence.require(isinstance(size, list) and len(size) == 2
                             and all(evidence.finite(value, 1) for value in size)
                             and abs(size[0] - bounds[2]) <= 1 and 0 <= bounds[3] - size[1] <= 80,
                             'Telemetry geometry cannot be attributed to owned window')
            return frame
        return self.wait_for(baseline)

    def cleanup(self):
        if self.child is None or self.cleanup_done:
            return
        self.cleanup_done = True
        if self.child.poll() is None and self.binding is not None:
            for operation in ('release', 'close'):
                if self.child.poll() is not None:
                    break
                try:
                    self.native(operation, cleanup=True)
                    self.cleanup_log.append({'operation': operation, 'attempted': True, 'error': None})
                except (Exception, KeyboardInterrupt) as error:
                    self.cleanup_log.append({'operation': operation, 'attempted': True,
                                             'error': str(error) or type(error).__name__})
        if self.child.poll() is None:
            try:
                self.child.wait(timeout=min(2, self.remaining(cleanup=True)))
            except (Exception, KeyboardInterrupt) as error:
                self.cleanup_log.append({'operation': 'wait_close', 'error': str(error) or type(error).__name__})
        for operation in ('terminate', 'kill'):
            if self.child.poll() is not None:
                break
            self.forced = True
            try:
                getattr(self.child, operation)()
                self.cleanup_log.append({'operation': operation, 'attempted': True, 'error': None})
                self.child.wait(timeout=min(2, max(0, self.deadline - self.clock.monotonic())))
            except (Exception, KeyboardInterrupt) as error:
                self.cleanup_log.append({'operation': operation, 'attempted': True,
                                         'error': str(error) or type(error).__name__})

    def export_camera_control(self):
        capture = self.camera_capture
        evidence.require(not self.tutorial and set(capture) == {
            'baseline', 'fence', 'binding_before', 'binding_after', 'started_unix_ns',
            'finished_unix_ns', 'before_command_index', 'key_command_index', 'after_command_index'},
            'Camera export lacks actual before/key/after capture')
        evidence.require(isinstance(self.raw_key_result, dict), 'Camera export lacks decoded raw key response')
        derived = validate_post(copy.deepcopy(self.raw_key_result), capture['binding_before'],
                                self.plan['steps'][0]['input'])
        evidence.require(json.dumps(self.post, sort_keys=True, allow_nan=False)
                         == json.dumps(derived, sort_keys=True, allow_nan=False),
                         'Legacy native posts contradict the captured raw key response')
        evidence.require('press_duration_us' not in self.raw_key_result
                         or json.dumps(self.raw_key_result['press_duration_us'], sort_keys=True, allow_nan=False)
                         == json.dumps(derived['press_duration_us'], sort_keys=True, allow_nan=False),
                         'Raw key response contradicts deterministic legacy enrichment')
        evidence.require(self.final_observation is not None and self.final_observation['complete']
                         and self.final_records is not None, 'Independent final whole bytes unavailable')
        result = evidence.control_effect(self.final_records, self.plan['steps'][0]['input'],
            evidence.CAMERA_CONDITIONS, {'baseline': copy.deepcopy(capture['baseline'])},
            capture['fence'], capture['started_unix_ns'], capture['finished_unix_ns'],
            next_started_unix_ns=None)
        evidence.require(result['complete'] is True, 'Final whole-window camera result is incomplete')
        control = {'control_effect': result, 'native_result': copy.deepcopy(self.raw_key_result),
                   'binding_before': copy.deepcopy(capture['binding_before']),
                   'binding_after': copy.deepcopy(capture['binding_after']), 'fence': capture['fence'],
                   'started_unix_ns': capture['started_unix_ns'],
                   'finished_unix_ns': capture['finished_unix_ns'], 'next_started_unix_ns': None}
        association = {'control_index': 0, **{name: capture[name] for name in (
            'before_command_index', 'key_command_index', 'after_command_index')}}
        return control, association

    def __call__(self, command, *, cwd, check, timeout, launch_receipt):
        self.command = command
        self.telemetry = Path(command[command.index('--window-session-output') + 1])
        try:
            self.final_observer = journal_observation.JournalObservation(self.directory,
                self.telemetry / 'events.jsonl', deadline=self.deadline, clock=self.clock)
            self.final_observer.remember(directory_only=True)
            evidence.require(check is False and timeout == self.plan['timeout_seconds'], 'Unexpected runner policy')
            self.remaining()
            strict_selected_save(command, launch_receipt)
            evidence.require(self.pins.check(), 'Driver input changed before native launch')
            reserve(self.directory)
            with (self.directory / 'child.stdout.log').open('xb') as stdout, \
                    (self.directory / 'child.stderr.log').open('xb') as stderr:
                evidence.require(play_alpha.check_inputs(launch_receipt), 'Launcher inputs changed before spawn')
                self.child = self.backend.start(command, cwd, stdout, stderr)
                evidence.require(evidence.integer(self.child.pid, 1), 'Missing actual application child PID')
                try:
                    baseline = self.observe_camera()
                    remembered = {'baseline': baseline['state']['value']}
                    if not self.tutorial:
                        self.camera_capture = {'baseline': copy.deepcopy(baseline['state']['value']),
                                               'fence': self.fence}
                    if self.tutorial:
                        self.reservation = evidence.parse_json(json.dumps({
                            'identity': evidence.offered_opening(baseline['state']['value']), 'frame': baseline}))
                    if not self.tutorial:
                        self.camera_capture['before_command_index'] = len(self.native_log)
                    observed_before = self.native('observe')
                    if not self.tutorial:
                        self.camera_capture['binding_before'] = copy.deepcopy(observed_before['binding'])
                    if self.tutorial:
                        fresh = self.read_records()
                        self.reservation['pre_post_frame'] = self.check_opening_history(fresh)
                    started = self.clock.time_ns()
                    if not self.tutorial:
                        self.camera_capture['started_unix_ns'] = started
                        self.camera_capture['key_command_index'] = len(self.native_log)
                    self.post = self.native('key')
                    finished = self.clock.time_ns()
                    if not self.tutorial:
                        self.camera_capture['finished_unix_ns'] = finished
                    def effect(records):
                        if self.tutorial:
                            self.check_opening_history(records, started)
                            result = evidence.offered_response(records, self.reservation, self.fence,
                                                               started, finished, complete=False)
                            return result if result['complete'] else None
                        frames = [entry['data'] for entry in records if entry['kind'] == 'frame']
                        evidence.require(not frames or evidence.usable_camera(frames[-1]['state']['value']),
                                         'Camera diagnostic encountered modal, pending or playing state')
                        result = evidence.control_effect(records, self.plan['steps'][0]['input'],
                            evidence.CAMERA_CONDITIONS, remembered, self.fence, started, finished, complete=False)
                        return result if result['complete'] else None
                    result = self.wait_for(effect)
                    if self.tutorial:
                        self.response = result
                    else:
                        self.control = result
                    if not self.tutorial:
                        self.camera_capture['after_command_index'] = len(self.native_log)
                    observed_after = self.native('observe')
                    if not self.tutorial:
                        self.camera_capture['binding_after'] = copy.deepcopy(observed_after['binding'])
                    self.next_started = self.clock.time_ns()
                    if not self.tutorial:
                        self.close_started_unix_ns = self.next_started
                    self.native('close')
                    self.child.wait(timeout=self.remaining())
                    self.read_records()
                    evidence.journal(self.payload, self.child.pid, complete=True)
                    if self.tutorial:
                        self.check_opening_history(self.records, started)
                        self.response = evidence.offered_response(self.records, self.reservation, self.fence,
                            started, finished, self.next_started)
                    else:
                        self.control = evidence.control_effect(self.records, self.plan['steps'][0]['input'],
                            evidence.CAMERA_CONDITIONS, remembered, self.fence, started, finished, self.next_started)
                finally:
                    self.cleanup()
        except KeyboardInterrupt as error:
            self.failure, self.primary_status = str(error) or 'KeyboardInterrupt', 'interrupted'
        except (TimeoutError, subprocess.TimeoutExpired) as error:
            self.failure, self.primary_status = str(error), 'timeout'
        except Exception as error:
            self.failure, self.primary_status = str(error) or type(error).__name__, 'diagnostic_failed'
        finally:
            if self.child is not None and self.child.poll() is None:
                self.cleanup()
        actual = self.child.poll() if self.child is not None else None
        self.final_child_collected = self.child is not None and actual is not None
        post_errors = []
        post_errors.extend('Cleanup: ' + item['error'] for item in self.cleanup_log if item.get('error'))
        try:
            pinned_integrity = self.pins.check()
            launcher_integrity = play_alpha.check_inputs(launch_receipt)
            integrity = pinned_integrity and launcher_integrity
            reserve(self.directory)
        except Exception as error:
            integrity = False
            post_errors.append(str(error))
        if not integrity:
            post_errors.append('A pre/post input identity changed or disappeared')
        complete_journal = False
        if self.child is not None:
            try:
                self.inspect_journal(complete=True)
            except Exception as error:
                post_errors.append('Final journal: ' + str(error))
            try:
                self.read_records(complete=True)
                if self.tutorial and self.response is not None:
                    self.check_opening_history(self.final_records, started)
                    self.response = evidence.offered_response(self.final_records, self.reservation, self.fence,
                        started, finished, self.next_started)
                complete_journal = True
            except Exception as error:
                post_errors.append('Final journal: ' + str(error))
        exported_controls, associations = [], []
        control_export_error, controls_write_error = None, None
        if not self.tutorial:
            try:
                evidence.require(complete_journal and integrity and actual == 0 and not self.forced
                                 and self.failure is None and not post_errors and self.control is not None,
                                 'Camera export requires a clean complete final collection and legacy result')
                self.remaining(cleanup=True)
                control, association = self.export_camera_control()
                self.remaining(cleanup=True)
            except Exception as error:
                control_export_error = str(error) or type(error).__name__
                post_errors.append('Final whole-window camera export: ' + control_export_error)
            else:
                try:
                    self.remaining(cleanup=True)
                    write_new(self.directory / 'controls.json', [control])
                    self.remaining(cleanup=True)
                except Exception as error:
                    controls_write_error = str(error) or type(error).__name__
                    post_errors.append('Camera controls write: ' + controls_write_error)
                else:
                    exported_controls, associations = [control], [association]
        graceful = (self.child is not None and actual == 0 and not self.forced
                    and self.failure is None and not post_errors
                    and (self.response is not None if self.tutorial else self.control is not None))
        status = self.primary_status or ('exited' if graceful or actual not in (0, None) else 'diagnostic_failed')
        code = (130 if status == 'interrupted' else 124 if status == 'timeout' else
                actual if actual not in (0, None) else 0 if graceful else 1)
        report = {**evidence.summarize(self.records, self.control if graceful else None,
                                     self.response if graceful else None),
                  'diagnostic_mode': evidence.TUTORIAL_MODE if self.tutorial else 'camera-right',
                  'reserved_opening': self.reservation,
                  'reservation_history_failure': self.reservation_history_failure,
                  'partial_offered_response_diagnostic': self.response if not graceful else None,
                  'partial_control_effect_diagnostic': self.control if not graceful else None,
                  'status': status, 'exit_code': code, 'pid': self.child.pid if self.child else None,
                  'actual_child_returncode': actual, 'signal': -actual if actual is not None and actual < 0 else None,
                  'child_outcome': 'not_started' if self.child is None else 'uncollected' if actual is None else 'exited',
                  'primary_failure': self.failure, 'post_errors': post_errors, 'cleanup': self.cleanup_log,
                  'forced_teardown': self.forced, 'graceful_end': graceful,
                  'inputs_unchanged': integrity, 'inputs': self.pins.items, 'binding': self.binding,
                  'binding_state': 'bound' if self.binding is not None else 'unbound',
                  'binding_scope': 'Last validated snapshot only; not a claim of current process identity or focus',
                  'kernel_identity': self.kernel_identity,
                  'native_commands': self.native_log, 'native_posts': self.post,
                  'journal_buffer': {'bytes': len(self.payload), 'sha256': hashlib.sha256(self.payload).hexdigest(),
                                     'scope': 'Retained reader buffer only; not on-disk size or hash'},
                  'journal_on_disk': self.journal_on_disk,
                  'journal_final_observation': self.final_observation,
                  'journal_inspection_errors': self.journal_inspection_errors,
                  'journal_read_errors': self.journal_read_errors,
                  'journal_bytes_scope': 'Retained reader buffer only; legacy journal_bytes/journal_sha256 aliases',
                  'journal_sha256': hashlib.sha256(self.payload).hexdigest(),
                  'journal_bytes': len(self.payload), 'elapsed_seconds': self.clock.monotonic() - self.started,
                  'bundle_closure_claimed': False}
        if not self.tutorial:
            report.update({'control_export_contract': CAMERA_EXPORT_CONTRACT,
                'control_export_status': 'exported_unqualified' if exported_controls else 'failed',
                'control_export_offline_only': True, 'control_export_qualified': False,
                'control_export_integration_reviewed': False, 'control_export_driver_enabled': False,
                'control_export_error': control_export_error, 'controls_write_error': controls_write_error,
                'exported_controls': exported_controls, 'control_native_associations': associations,
                'native_posts_raw_v2': copy.deepcopy(self.raw_key_result),
                'control_capture_diagnostic': copy.deepcopy(self.camera_capture),
                'close_started_unix_ns': self.close_started_unix_ns})
        try:
            self.remaining(cleanup=True)
            write_new(self.directory / 'result.json', report)
            self.remaining(cleanup=True)
        except Exception as error:
            report['report_error'] = str(error)
            if isinstance(error, TimeoutError) and self.final_observation is not None:
                self.final_observer.fail(self.final_observation, error)
            if not self.tutorial:
                report.update(control_export_status='failed', exported_controls=[], control_native_associations=[])
            if report['control_effect'] is not None:
                report['partial_control_effect_diagnostic'] = report['control_effect']
            report['control_effect'] = None
            if report['offered_response_acknowledgment'] is not None:
                report['partial_offered_response_diagnostic'] = report['offered_response_acknowledgment']
            report['offered_response_acknowledgment'] = None
            if report['status'] == 'exited' and actual == 0:
                report['status'] = 'diagnostic_failed'
            if report['exit_code'] == 0:
                report['exit_code'] = 1
        return SimpleNamespace(returncode=actual, diagnostic=report)


def prepare(output, binary, game_root, helper, build_receipt, plan_file, endpoint_file, selected_save=None):
    game_root = Path(game_root)
    evidence.require(game_root.is_absolute(), 'Absolute installation root required before output reservation')
    started = time.monotonic()
    sources = [SOURCE / name for name in ('play_alpha.py', 'window_session.py', 'window_evidence.py',
                                          'window_session_macos.swift', 'window_journal_observation.py')]
    sources.extend(play_alpha.REPOSITORY / name for name in SCHEMA_PATHS)
    inputs = [binary, helper, build_receipt, plan_file, endpoint_file, *sources]
    if selected_save is not None:
        inputs.append(selected_save)
    directory = private_directory(output, [game_root, play_alpha.REPOSITORY / 'cache/alpha-sessions', *inputs])
    pins = Pins(directory)
    helper = play_alpha.protected_path(helper)
    swift = SOURCE / 'window_session_macos.swift'
    helper_receipt(build_receipt, helper, swift)
    pinned_helper = pins.add('native-helper', helper, executable=True)
    receipt = pins.add('helper-build.json', build_receipt)
    plan_path = pins.add('plan.json', plan_file)
    endpoint_path = pins.add('endpoints.json', endpoint_file)
    for index, source in enumerate(sources):
        pins.add('source-{}-{}'.format(index, source.name), source)
    plan, endpoints = play_alpha.read_json(plan_path), play_alpha.read_json(endpoint_path)
    evidence.validate_specification(plan, endpoints, play_alpha.digest(plan_path))
    helper_receipt(receipt, helper, swift)
    evidence.require(pins.check(), 'Input changed during diagnostic preparation')
    return DiagnosticRunner(directory, pins, pinned_helper, plan, started=started)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--helper', type=Path, required=True)
    parser.add_argument('--helper-build-receipt', type=Path, required=True)
    parser.add_argument('--plan', type=Path, required=True)
    parser.add_argument('--endpoints', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument('--load', type=Path)
    selection.add_argument('--continue', dest='resume', action='store_true')
    arguments = parser.parse_args()
    try:
        runner = prepare(arguments.output, arguments.binary, arguments.root, arguments.helper,
                         arguments.helper_build_receipt, arguments.plan, arguments.endpoints, arguments.load)
        return play_alpha.launch(arguments.binary, arguments.root, load=arguments.load, resume=arguments.resume,
            timeout=runner.plan['timeout_seconds'], window_session_output=runner.directory / 'telemetry',
            child_runner=runner)
    except (OSError, ValueError, TypeError) as error:
        parser.exit(1, str(error) + '\n')


if __name__ == '__main__':
    raise SystemExit(main())
