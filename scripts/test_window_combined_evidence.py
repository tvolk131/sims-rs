"""Audited, stdlib-only synthetic diagnostics: python3 -I -B <this file>.

All receipt/process/binary claims are fabricated. Frozen native/collector source
TEXT is copied to synthetic files to exercise source contracts. The Rust
collector is independently pinned current source, not the Python analyzer.
No native library is loaded and no root test, driver, launcher or builder is
imported. The audit guard precedes tested imports and confines file accesses to
declared source text, stdlib imports and this test's private temporary directory.
"""

import sys
import os


FORBIDDEN_MODULES = frozenset(('subprocess', '_posixsubprocess', 'socket', '_socket', 'ctypes', '_ctypes',
                              'multiprocessing', 'objc', 'cffi', '_cffi_backend', 'window_session',
                              'play_alpha', 'build_window_presentation', 'window_presentation',
                              'test_window_presentation', 'test_window_presentation_evidence',
                              'test_window_evidence'))
TEST_ROOT = None
SOURCE_ROOT = __file__.rsplit('/scripts/', 1)[0]
SOURCE_READS = frozenset(SOURCE_ROOT + '/' + name for name in (
    'scripts/window_combined_evidence.py', 'scripts/test_window_combined_evidence.py',
    'scripts/window_presentation_evidence.py', 'scripts/window_evidence.py',
    'crates/sims-app/native/window_present_macos.m', 'crates/sims-app/native/window_present_macos.h',
    'crates/sims-app/src/window_session/presentation.rs'))
OPEN_CONTEXT = []
OPEN_DESCRIPTORS = {}
ORIGINAL_OS_OPEN, ORIGINAL_OS_CLOSE, ORIGINAL_OS_READ = os.open, os.close, os.read


def descriptor_path(descriptor_fd):
    entry = OPEN_DESCRIPTORS.get(descriptor_fd)
    if entry is None:
        raise RuntimeError('Audit guard: unknown descriptor')
    info = os.fstat(descriptor_fd)
    if (info.st_dev, info.st_ino) != entry[1]:
        raise RuntimeError('Audit guard: stale descriptor')
    return entry[0]


def guarded_open(path, flags, mode=0o777, *, dir_fd=None):
    raw = os.fsdecode(path)
    if os.path.isabs(raw):
        absolute = os.path.normpath(raw)
    elif dir_fd is not None:
        absolute = os.path.normpath(os.path.join(descriptor_path(dir_fd), raw))
    else:
        raise RuntimeError('Audit guard: unanchored relative open')
    OPEN_CONTEXT.append((raw, absolute))
    try:
        descriptor_fd = ORIGINAL_OS_OPEN(path, flags, mode, dir_fd=dir_fd)
        info = os.fstat(descriptor_fd)
        OPEN_DESCRIPTORS[descriptor_fd] = (absolute, (info.st_dev, info.st_ino))
        return descriptor_fd
    finally:
        OPEN_CONTEXT.pop()


def guarded_close(descriptor_fd):
    OPEN_DESCRIPTORS.pop(descriptor_fd, None)
    return ORIGINAL_OS_CLOSE(descriptor_fd)


def guarded_read(descriptor_fd, length):
    descriptor_path(descriptor_fd)
    return ORIGINAL_OS_READ(descriptor_fd, length)


class GuardedOpen:
    def __call__(self, path, flags, mode=0o777, *, dir_fd=None):
        return guarded_open(path, flags, mode, dir_fd=dir_fd)


def audit_guard(event, arguments):
    if event == 'os.chdir':
        raise RuntimeError('Audit guard: changing read anchors')
    if event in ('os.scandir', 'os.listdir'):
        target = arguments[0]
        resolved = descriptor_path(target) if type(target) is int else os.path.abspath(os.fsdecode(target))
        standard_library = any(resolved == prefix or resolved.startswith(prefix + '/')
                               for prefix in (sys.base_prefix, sys.prefix))
        if not standard_library and (TEST_ROOT is None or not (
                resolved == TEST_ROOT or resolved.startswith(TEST_ROOT + '/'))):
            raise RuntimeError('Audit guard: directory enumeration outside synthetic root')
    if event == 'import':
        name = arguments[0].split('.')[0]
        if name in FORBIDDEN_MODULES or name[:1].isupper():
            raise RuntimeError('Audit guard: forbidden import ' + name)
    if event.startswith(('subprocess.', 'socket.', 'ctypes.', 'signal.', 'os.exec', 'os.spawn',
                         'os.posix_spawn', 'os.fork', 'os.kill', 'os.system', 'pty.spawn', 'os.startfile')):
        raise RuntimeError('Audit guard: forbidden operation ' + event)
    if event == 'open':
        path, mode, flags = arguments
        if type(path) is int:
            descriptor_path(path)
            return
        if type(path) is bytes:
            path = path.decode('utf-8')
        if type(path) is not str:
            raise RuntimeError('Audit guard: unexpected file path')
        if not os.path.isabs(path):
            if not OPEN_CONTEXT or OPEN_CONTEXT[-1][0] != path:
                raise RuntimeError('Audit guard: unanchored relative read')
            path = OPEN_CONTEXT[-1][1]
        path = os.path.normpath(path)
        writing = ((type(mode) is str and any(character in mode for character in 'wax+'))
                   or bool(flags & (os.O_WRONLY | os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_APPEND)))
        inside = TEST_ROOT is not None and (path == TEST_ROOT or path.startswith(TEST_ROOT + '/'))
        if inside:
            resolved = os.path.realpath(path)
            if resolved != TEST_ROOT and not resolved.startswith(TEST_ROOT + '/'):
                raise RuntimeError('Audit guard: synthetic symlink escapes root')
        if writing and not inside:
            raise RuntimeError('Audit guard: write outside synthetic directory')
        if inside or path in SOURCE_READS:
            return
        if not writing and path.endswith(('.py', '.pyc')) and any(
                path.startswith(prefix + '/') for prefix in (sys.base_prefix, sys.prefix)):
            return
        if not writing and flags & os.O_DIRECTORY and any(
                candidate.startswith(path.rstrip('/') + '/')
                for candidate in tuple(SOURCE_READS) + ((TEST_ROOT,) if TEST_ROOT else ())):
            return
        raise RuntimeError('Audit guard: undeclared file read ' + path)


sys.addaudithook(audit_guard)
sys.dont_write_bytecode = True
os.open, os.close, os.read = GuardedOpen(), guarded_close, guarded_read

import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile
import types
import unittest


def deny_signal(*arguments, **keywords):
    raise RuntimeError('Audit guard: process-control operation')


for signal_module in ('signal', '_signal'):
    for name in ('signal', 'alarm', 'setitimer', 'raise_signal', 'pthread_kill', 'pthread_sigmask',
                 'sigwait', 'sigwaitinfo', 'sigtimedwait', 'set_wakeup_fd', 'pause'):
        if hasattr(sys.modules.get(signal_module), name):
            setattr(sys.modules[signal_module], name, deny_signal)

SOURCE_READ_PINS = {}


def read_source(filename):
    data = Path(filename).read_bytes()
    source_hash = hashlib.sha256(data).hexdigest()
    previous = SOURCE_READ_PINS.setdefault(filename, {'sha256': source_hash, 'size': len(data), 'reads': 0})
    if previous['sha256'] != source_hash or previous['size'] != len(data):
        raise RuntimeError('Source changed during guarded synthetic suite')
    previous['reads'] += 1
    return data


EXECUTED_SOURCE_PINS = {}
for module_name in ('window_evidence', 'window_presentation_evidence', 'window_combined_evidence'):
    filename = SOURCE_ROOT + '/scripts/' + module_name + '.py'
    source_bytes = read_source(filename)
    loaded_module = types.ModuleType(module_name)
    loaded_module.__file__ = filename
    loaded_module.__package__ = ''
    sys.modules[module_name] = loaded_module
    exec(compile(source_bytes, filename, 'exec'), loaded_module.__dict__)
    EXECUTED_SOURCE_PINS[module_name] = hashlib.sha256(source_bytes).hexdigest()
combined = sys.modules['window_combined_evidence']


UNIX = 1_000_000_000_000
HOST = 8
WIDTH = 2**-20


def unix(delta):
    return UNIX + int(delta * 1_000_000_000)


def frame_state(center):
    return {'tick': 7, 'playing': False, 'pending': False, 'clock_pending': False,
            'presentation_children_pending': False, 'modal': False, 'camera': {'center': [center, 0]},
            'display_size': [800, 600], 'terminals': None, 'hud_normal_ready': None,
            'hud_transport_ready': None, 'observation_capabilities': {
                'action_terminals': False, 'route_terminals': False, 'transport_readiness': False},
            'error': None, 'menu_error': None, 'shell': None}


def synthetic_binding(executable):
    return {'schema': 'sims-window-binding-v2', 'pid': 42, 'window_id': 91, 'executable': executable,
            'kernel_identity': {'source': 'proc_pidinfo(PROC_PIDTBSDINFO)+proc_pidpath', 'pid': 42,
                                'start_tvsec': 123, 'start_tvusec': 456, 'executable': executable},
            'frontmost': True, 'on_screen': True, 'bounds': [0, 0, 800, 600],
            'display_scope': 'Current CoreGraphics mode and NSScreen maximum; not instantaneous refresh',
            'displays': [{'display_id': 1, 'mode_id': 2, 'width': 800, 'height': 600,
                          'pixel_width': 1600, 'pixel_height': 1200, 'nominal_refresh_hz': 60,
                          'maximum_frames_per_second': 60, 'builtin': True, 'active': True, 'asleep': False}]}


def synthetic_session(binding):
    records, frames = [], []
    analyzer = combined.presentation

    def record(kind, delta, data):
        records.append({'kind': kind, 'elapsed_us': int((delta + 1) * 1_000_000),
                        'dropped_records': 0, 'data': data})

    def native(kind, delta, flags=0, **values):
        event = dict.fromkeys(analyzer.EVENT_INTEGERS + analyzer.EVENT_REALS, 0)
        event.update(abi_version=1, struct_size=192, kind=kind, flags=flags, status=0,
                     host_time=HOST + delta)
        event.update(values)
        record('presentation_event', delta, event)

    def clocks(phase, delta):
        absolute = int((HOST + delta) * 1_000_000_000)
        record('presentation_sleep_clock', delta, {
            'schema': 1, 'phase': phase,
            'clock': 'mach timebase ticks; absolute stops during sleep; continuous includes sleep',
            'clock_api': analyzer.CLOCK_API, 'quartz_before_seconds': absolute / 1e9 - 1e-9,
            'quartz_after_seconds': (absolute + 2) / 1e9 + 1e-9,
            'sample': {'absolute_before': absolute, 'continuous': absolute + 1_000_000_001,
                       'absolute_after': absolute + 2, 'timebase_numer': 1, 'timebase_denom': 1}})

    def draw(delta, sequence, center):
        identifier = len(frames) + 1
        data = {'frame_id': identifier, 'fence': 1, 'input_sequence': sequence,
                'draw_started_unix_ns': unix(delta), 'cursor': None, 'widget_draw_cpu_us': 50,
                'draw_interval_us': None, 'state': {'state_id': center + 1, 'value': frame_state(center),
                                                 'diagnostics': {'clock': None, 'worker_busy': False}}}
        frames.append(data)
        identity = {'draw_context_sequence': identifier, 'frame_id': identifier, 'state_id': center + 1}
        native(2, delta, 4, **identity)
        record('frame', delta, data)
        identity.update(acquisition_sequence=identifier, layer_lifetime_id=1,
                        acquisition_enter_host_time=HOST + delta + 1 / 8192)
        native(3, delta + 1 / 8192, 4100, **identity)
        identity.update(acquisition_call_host_time=HOST + delta + 2 / 8192,
                        acquisition_return_host_time=HOST + delta + 3 / 8192,
                        drawable_id=identifier - 1, texture_identity=100 + identifier,
                        drawable_width=1600, drawable_height=1200,
                        display_sync_enabled=1, maximum_drawable_count=3)
        native(4, delta + 3 / 8192, 4188, **identity)
        native(5, delta + 4 / 8192, 4188, **identity)
        identity.update(bind_count=1, bound_frame_id=identifier, bound_state_id=center + 1)
        native(7, delta + 5 / 8192, 60, **identity)
        native(6, delta + 3 / 1024, 60, **identity,
               presented_host_time=HOST + delta + 2 / 1024,
               callback_delivery_host_time=HOST + delta + 3 / 1024)
        return data

    record('session', -1, {'schema': 1, 'pid': 42, 'unix_ns': str(UNIX - 1_000_000_000),
                          'gpu_time_available': False, 'present_time_available': False})
    clocks('begin', -1 / 8)
    record('presentation_library', -1 / 16, {
        'source_contract': analyzer.SOURCE_CONTRACT, 'source_only': True, 'abi_version': 1,
        'event_size': 192, 'counters_size': 304, 'summary_size': 408,
        'source_sha256': analyzer.SOURCE_PINS['root/crates/sims-app/native/window_present_macos.m']})
    native(1, 0)
    native(9, 1 / 1024, platform_notification=6)
    record('presentation_begin', 1 / 512, {'abi_version': 1, 'status': 0,
        'clock': analyzer.CLOCK_DESCRIPTION, 'clock_api': analyzer.CLOCK_API, 'boundary': analyzer.BOUNDARY})
    baseline = draw(1 / 32, 0, 0)
    press, release = 1 / 16, 3 / 32
    receipts = [{'fence': 1, 'input_sequence': ordinal, 'received_unix_ns': unix(delta + 1 / 1024),
                 'input': {'kind': kind, 'physical_key': 'Code(ArrowRight)'}}
                for ordinal, (delta, kind) in enumerate(((press, 'key_pressed'), (release, 'key_released')), 1)]
    record('input', press + 1 / 1024, receipts[0])
    effect = draw(5 / 64, 1, 1)
    record('input', release + 1 / 1024, receipts[1])
    closing = draw(7 / 64, 2, 1)
    latency = (effect['draw_started_unix_ns'] - receipts[0]['received_unix_ns']) / 1000
    control = {'complete': True, 'origin': analyzer.CONTROL_ORIGIN,
               'source_origin': 'window_controls.summarize_controls',
               'latency_us': {'samples': 1, 'p50': latency, 'p95': latency, 'p99': latency, 'worst': latency},
               'p95_limit_us': 100_000, 'maximum_limit_us': 250_000, 'within_fixed_limits': True,
               'baseline_frame_id': baseline['frame_id'], 'receipts': copy.deepcopy(receipts),
               'effect': {'frame_id': effect['frame_id'], 'state_id': 2,
                          'unix_ns': effect['draw_started_unix_ns'], 'latency_us': latency},
               'closing_frame_id': closing['frame_id'], 'scope': 'Synthetic receipt-to-draw metric, not visibility'}
    posts = [{'down': down, 'unix_before_ns': unix(delta), 'unix_after_ns': unix(delta + WIDTH),
              'host_before_ns': 900_000_000_000 + int(delta * 1e9),
              'host_after_ns': 900_000_000_000 + int((delta + WIDTH) * 1e9),
              'clock_api': analyzer.CLOCK_API, 'quartz_before_seconds': HOST + delta,
              'quartz_after_seconds': HOST + delta + WIDTH} for down, delta in ((True, press), (False, release))]
    controls = [{'control_effect': control,
                 'native_result': {'operation': 'key', 'binding': copy.deepcopy(binding), 'code': 124,
                                   'press_ms': 20, 'released': True, 'posts': posts},
                 'binding_before': copy.deepcopy(binding), 'binding_after': copy.deepcopy(binding),
                 'fence': 1, 'started_unix_ns': unix(press - 1 / 1024),
                 'finished_unix_ns': unix(release + 1 / 2048), 'next_started_unix_ns': None}]
    cutoff = 9 / 64
    native(11, cutoff, 1)
    native(13, cutoff + 2 / 1024, 1)
    clocks('end', 1 / 4)
    events = [item['data'] for item in records if item['kind'] == 'presentation_event']
    for sequence, event in enumerate(events, 1):
        event['event_sequence'] = sequence
    counters = dict.fromkeys(analyzer.COUNTERS, 0)
    for name, kind in analyzer.COUNTED.items():
        counters[name] = sum(event['kind'] == kind for event in events)
    counters.update(emitted_events=len(events), successful_acquisitions=3,
                    handler_registration_attempts=3, successful_binds=3, positive_presentations=3)
    record('presentation_end', 9 / 32, {'status': 0, 'summary': {
        'abi_version': 1, 'struct_size': 408, 'status': 0, 'session_state': 4, 'capabilities': 3, 'flags': 12,
        'begin_host_time': HOST, 'cutoff_host_time': HOST + cutoff,
        'drain_end_host_time': HOST + cutoff + 2 / 1024, 'snapshot_host_time': HOST + cutoff + 3 / 1024,
        'last_event_sequence': len(events), 'last_acquisition_sequence': 3,
        'unresolved_acquisitions': 0, 'unbound_acquisitions': 0,
        'unresolved_unbound_acquisitions': 0, 'counters': counters}})
    record('session_end', 5 / 16, {'dropped_records': 0})
    return records, controls


class CombinedEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        global TEST_ROOT
        base = Path(tempfile.gettempdir()).resolve()
        cls.root = base / ('window-combined-synthetic-' + next(tempfile._get_candidate_names()))
        TEST_ROOT = str(cls.root)
        cls.root.mkdir(mode=0o700)
        if shutil.disk_usage(cls.root).free < 6 * 1024**3 + 64 * 1024**2:
            raise RuntimeError('At least six GiB reserve required')

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.root)

    def setUp(self):
        self.directory = self.root / self._testMethodName
        self.directory.mkdir()
        self.payloads, self.observations = {}, {}
        self.spec = {'schema': combined.INPUT_SCHEMA, 'artifacts': {}, 'sources': {}, 'binaries': {}}
        self.policy = {'schema': combined.POLICY_SCHEMA, 'approved_roots': [str(self.directory)],
                       'expected_pins': {}, 'source_inventory': {}, 'bindings': {},
                       'budgets': {'metadata_bytes': 8 * 1024**2, 'binary_bytes': 1024**2, 'artifacts': 64}}
        self.values = {}
        for name in ('application', 'application_pinned', 'helper', 'helper_pinned', 'presentation', 'presentation_pinned'):
            content = ('NEVER EXECUTED synthetic ' + name.removesuffix('_pinned')).encode()
            self.add('binary:' + name, content)
            self.policy['bindings'][name] = 'binary:' + name
        for binding, name in (('launcher_source', 'scripts/play_alpha.py'), ('helper_source', 'scripts/window_session_macos.swift'),
                              ('driver_source', 'scripts/window_session.py'),
                              ('analyzer_source', combined.ANALYZER_SOURCE), ('collector_source', combined.COLLECTOR_SOURCE)):
            content = (read_source(SOURCE_ROOT + '/' + name) if binding in ('collector_source', 'analyzer_source')
                       else ('synthetic source ' + name).encode())
            self.add('source:' + name, content)
            self.policy['bindings'][binding] = 'source:' + name
        pairs = {}
        for suffix in ('.m', '.h'):
            name = 'crates/sims-app/native/window_present_macos' + suffix
            content = read_source(SOURCE_ROOT + '/' + name)
            self.add('source:' + name, content)
            self.add('source:staged/' + name, content)
            pairs['source:' + name] = 'source:staged/' + name
        name = 'crates/sims-app/src/window_session/presentation.rs'
        pairs['source:' + name] = None
        self.policy['bindings']['presentation_sources'] = pairs
        executable = self.item('application_pinned')['path']
        binding = synthetic_binding(executable)
        self.records, self.controls = synthetic_session(binding)
        self.records[0]['data'].update(kernel_identity=copy.deepcopy(binding['kernel_identity']),
            build_ids={name + '_sha256': self.item(role)['sha256'] for name, role in (
                ('application', 'application_pinned'), ('helper', 'helper_pinned'), ('presentation', 'presentation_pinned'))})
        library = next(record['data'] for record in self.records if record['kind'] == 'presentation_library')
        library.update(path=self.item('presentation_pinned')['path'],
                       header_sha256=self.policy['source_inventory']['crates/sims-app/native/window_present_macos.h'],
                       loaded_binary_receipt=False, loaded_binary_source_attested=False)
        self.add('journal', self.journal_bytes())
        self.add_json('controls', self.controls)
        self.add_json('plan', {'schema': 1, 'timeout_seconds': 10, 'household': 'newbie',
                              'steps': [{'name': 'camera-right', 'input': {'kind': 'key', 'code': 124, 'press_ms': 20}}]})
        self.add_json('endpoints', {'schema': 1, 'plan_sha256': self.spec['artifacts']['plan']['sha256'],
                                   'controls': [{'step': 'camera-right', 'conditions': combined.ordinary.CAMERA_CONDITIONS}]})
        helper = self.item('helper')
        source = self.item('helper_source')
        self.add_json('helper_build', {'schema': 'sims-window-helper-build-v1', 'source_sha256': source['sha256'],
            'executable_sha256': helper['sha256'], 'source_path': source['path'], 'binary_path': helper['path'],
            'compiler': 'synthetic, never invoked', 'command': ['/never/run/swiftc', source['path'], '-o', helper['path']]})
        diagnostic = {'status': 'exited', 'exit_code': 0, 'actual_child_returncode': 0, 'signal': None,
            'journal_read_errors': [], 'journal_inspection_errors': [],
            'child_outcome': 'exited', 'primary_failure': None, 'post_errors': [], 'cleanup': [],
            'forced_teardown': False, 'graceful_end': True, 'inputs_unchanged': True, 'pid': 42,
            'binding': binding, 'kernel_identity': copy.deepcopy(binding['kernel_identity']),
            'journal_on_disk': {'sha256': self.spec['artifacts']['journal']['sha256']},
            'journal_bytes': len(self.payloads['journal']), 'journal_sha256': combined.digest(self.payloads['journal']),
            'journal_buffer': {'bytes': len(self.payloads['journal']), 'sha256': combined.digest(self.payloads['journal'])},
            'control_effect': copy.deepcopy(self.controls[0]['control_effect']),
            'native_posts': copy.deepcopy(self.controls[0]['native_result']),
            'control_native_associations': [{'control_index': 0, 'before_command_index': 0,
                                              'key_command_index': 1, 'after_command_index': 2}],
            'native_commands': [self.native_command(operation, binding, index)
                                for index, operation in enumerate(('observe', 'key', 'observe'))],
            'inputs': [self.receipt_pin('native-helper', helper, self.item('helper_pinned'), 'name')] + [
                self.receipt_pin(name + '.json', self.spec['artifacts'][name], self.spec['artifacts'][name], 'name')
                for name in ('plan', 'endpoints')] + [self.receipt_pin(
                    'helper-build.json', self.spec['artifacts']['helper_build'], self.spec['artifacts']['helper_build'], 'name')]}
        self.add_json('result', diagnostic)
        launch = {'exit_code': 0, 'launcher_exit_code': 0, 'process': {'status': 'exited', 'returncode': 0},
                  'diagnostic': copy.deepcopy(diagnostic),
                  'command': [executable, '--root', '/installation/DO-NOT-READ', '--save', '/saves/DO-NOT-READ',
                              '--window-session-present-library', self.item('presentation_pinned')['path']],
                  'binary_sha256': self.item('application')['sha256'], 'inputs': [
                      self.receipt_pin('executable', self.item('application'), self.item('application_pinned')),
                      self.receipt_pin('presentation_library', self.item('presentation'), self.item('presentation_pinned'))]}
        self.add_json('launch', launch)
        inventory = copy.deepcopy(self.policy['source_inventory'])
        self.add_json('campaign_build', {'exit_code': 0, 'timed_out': False, 'source_unchanged': True,
            'source_before': inventory, 'source_after': inventory,
            'built_executables': {self.item('application')['path']: self.item('application')['sha256']},
            'pid': 900, 'executable': '/synthetic/Python-NOT-APPLICATION'})
        source_inputs = {self.item(name)['path']: self.item(name)['sha256'] for name in (
            'launcher_source', 'helper_source', 'driver_source', 'collector_source', 'analyzer_source')}
        candidate_inputs = {self.item('application')['path']: self.item('application')['sha256']}
        self.add_json('campaign_run', {'exit_code': 0, 'timed_out': False, 'source_unchanged': True,
            'inputs_unchanged': True, 'built_inputs_unchanged': True, 'executable_unchanged': True,
            'source_before': inventory, 'source_after': inventory, 'pid': 12345,
            'executable': '/synthetic/Python-NOT-APPLICATION', 'inputs_before': source_inputs,
            'inputs_after': copy.deepcopy(source_inputs), 'built_inputs_before': candidate_inputs,
            'built_inputs_after': copy.deepcopy(candidate_inputs),
            'build_receipt': self.spec['artifacts']['campaign_build']['path']})
        sources = []
        for role, staged in pairs.items():
            original = self.policy['expected_pins'][role]
            staging = self.policy['expected_pins'][staged] if staged is not None else None
            sources.append({'path': original['path'], 'sha256': original['sha256'],
                'before': self.state(original), 'after': self.state(original),
                'staged_path': staging['path'] if staging else None,
                'staged_before': self.state(staging) if staging else None,
                'staged_after': self.state(staging) if staging else None})
        product = self.item('presentation')
        self.add_json('presentation_build', {'schema': 1, 'kind': 'window_presentation_source_build',
            'mode': 'execute', 'outcome': 'built_unqualified', 'compiler': {'path': '/never/run/clang'},
            'command': ['/never/run/clang', '-dynamiclib',
                        self.policy['expected_pins'][pairs['source:crates/sims-app/native/window_present_macos.m']]['path'],
                        '-o', product['path']],
            'process': {'attempted': True, 'returncode': 0, 'timed_out': False, 'error': None},
            'cleanup': {'reaped': True, 'returncode': 0, 'errors': []},
            'setup_error': None, 'receipt_write_error': None, 'integrity_errors': [],
            'qualifications': {'build': False, 'alpha': False}, 'sources': sources,
            'output_path': product['path'], 'output': {'path': product['path'], 'state': self.state(product)}})
        if not getattr(self, 'legacy_final_observation', False):
            FinalObservationReaderTests.tagged(self)

    def add(self, role, payload):
        if role.startswith('source:'):
            group, name = 'sources', role.split(':', 1)[1]
        elif role.startswith('binary:'):
            group, name = 'binaries', role.split(':', 1)[1]
        else:
            group, name = 'artifacts', role
        path = self.directory / (role.replace('/', '__').replace(':', '_') + '.synthetic')
        if role.startswith('source:staged/'):
            path = self.directory / 'staged' / role.rsplit('/', 1)[-1]
            path.parent.mkdir(exist_ok=True)
        path.write_bytes(payload)
        item = {'path': str(path), 'size': len(payload), 'sha256': combined.digest(payload)}
        self.spec[group][name] = item
        self.policy['expected_pins'][role] = copy.deepcopy(item)
        if group == 'sources' and not name.startswith('staged/'):
            self.policy['source_inventory'][name] = item['sha256']
        self.payloads[role] = payload

    def add_json(self, role, value):
        self.values[role] = copy.deepcopy(value)
        self.add(role, combined.canonical(value))
        if role == 'result' and 'launch' in self.values:
            launch = copy.deepcopy(self.values['launch'])
            launch['diagnostic'] = copy.deepcopy(value)
            self.add_json('launch', launch)

    def native_command(self, operation, binding, index):
        request = {'operation': operation, 'pid': 42, 'executable': binding['executable'],
                   'binding': copy.deepcopy(binding)}
        response = {'operation': operation, 'binding': copy.deepcopy(binding)}
        if operation == 'key':
            request.update(code=124, press_ms=20)
            response = copy.deepcopy(self.controls[0]['native_result'])
        started = unix(0) if index == 0 else self.controls[0]['finished_unix_ns'] + 1
        finished = started + 1
        if operation == 'key':
            started = self.controls[0]['started_unix_ns'] + 1
            finished = self.controls[0]['finished_unix_ns'] - 1
        return {'request': request, 'returncode': 0, 'stderr': '',
                'stdout': combined.canonical(response).decode(),
                'started_unix_ns': started, 'finished_unix_ns': finished}

    def item(self, binding):
        return self.policy['expected_pins'][self.policy['bindings'][binding]]

    def state(self, item):
        return {'sha256': item['sha256'], 'size': item['size'], 'links': 1, 'mode': 0o100400,
                'device': 1, 'inode': 1, 'uid': 1, 'mtime_ns': 1, 'ctime_ns': 1}

    def receipt_pin(self, kind, source, pinned, selector='kind'):
        return {selector: kind, 'source': source['path'], 'pinned': pinned['path'],
                'before': self.state(source), 'after': self.state(source),
                'pinned_before': self.state(pinned), 'pinned_after': self.state(pinned)}

    def journal_bytes(self):
        return b''.join(combined.canonical(record) + b'\n' for record in self.records)

    def update_journal(self):
        self.add('journal', self.journal_bytes())
        diagnostic = self.values['result']
        payload = self.payloads['journal']
        diagnostic.update(journal_bytes=len(payload), journal_sha256=combined.digest(payload),
                          journal_buffer={'bytes': len(payload), 'sha256': combined.digest(payload)},
                          journal_on_disk={'sha256': combined.digest(payload)})
        self.add_json('result', diagnostic)
        if not getattr(self, 'legacy_final_observation', False):
            FinalObservationReaderTests.tagged(self)

    def analyze(self):
        bundle = {'schema': combined.BUNDLE_SCHEMA, 'input': self.spec,
                  'identity': combined.digest(combined.canonical(self.spec)),
                  'payloads': self.payloads, 'observations': self.observations}
        return combined.analyze_combined(bundle, policy=self.policy)

    def edge(self, result, name):
        return next(item['class'] for item in result['joins'] if item['edge'] == name)

    def assert_unqualified(self, result):
        self.assertIs(result['diagnostic_only'], True)
        for name, value in result.items():
            if name.endswith(('_qualified', '_proof', '_proven', '_attested', '_authenticated', '_reviewed', '_closure')) or name == 'driver_enabled':
                self.assertIs(value, False, name)
        for name in ('eligible_seconds', 'civil_days', 'visibility', 'full_readiness', 'worker_quiescence', 'input_to_photon'):
            self.assertIsNone(result[name], name)

    def assert_invalid(self, result):
        self.assertFalse(result['contract_valid'], result)
        self.assertTrue(result['errors'] or any(not item['scope_only'] and item['code'] !=
                                              'PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING'
                                              for item in result['blockers']), result)
        self.assert_unqualified(result)

    def assert_consistent_except_missing_save(self, result):
        self.assertFalse(result['contract_valid'], result)
        self.assertEqual(result['errors'], [], result)
        self.assertEqual({item['code'] for item in result['blockers'] if not item['scope_only']},
                         {'PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING'}, result)
        self.assertEqual(self.edge(result, 'selected_save_session_identity'), 'unverifiable')

    def test_complete_synthetic_joins_are_not_qualification(self):
        result = self.analyze()
        self.assert_consistent_except_missing_save(result)
        self.assertTrue(result['presentation_analyzer']['valid'], result)
        self.assertTrue(result['api_presentation_latency']['within_fixed_limits'])
        self.assertEqual(result['api_presentation_latency']['p95_limit_us'], 100_000)
        self.assertEqual(result['api_presentation_latency']['maximum_limit_us'], 250_000)
        self.assertEqual(self.edge(result, 'campaign_run'), 'matched')
        self.assertEqual(self.edge(result, 'loaded_image_attestation'), 'unverifiable')
        self.assertFalse(result['original_analyzer']['presentation']['available'])
        self.assertTrue(result['blockers'])
        self.assert_unqualified(result)

    def test_descriptor_reader_complete_synthetic_files(self):
        result = combined.read_combined(self.spec, reader=combined.DescriptorReader(), policy=self.policy)
        self.assert_consistent_except_missing_save(result)
        self.assertEqual(set(result['reader_observations']), set(self.policy['expected_pins']))
        for role, observation in result['reader_observations'].items():
            self.assertTrue(combined.valid_file_fingerprint(observation['before'], self.policy['expected_pins'][role]['size']))
            self.assertEqual(observation['before'], observation['after'])
        self.assert_unqualified(result)

    def test_pure_analysis_and_reader_do_not_mutate_inputs_or_files(self):
        before = copy.deepcopy((self.spec, self.policy, self.payloads, self.values))
        result = self.analyze()
        result['outcomes']['result']['cleanup'].append('output only')
        combined.read_combined(self.spec, reader=combined.DescriptorReader(), policy=self.policy)
        self.assertEqual(before, (self.spec, self.policy, self.payloads, self.values))
        for role, item in self.policy['expected_pins'].items():
            self.assertEqual(Path(item['path']).read_bytes(), self.payloads[role])

    def test_missing_current_producer_contracts_are_named(self):
        self.spec['artifacts'].pop('controls')
        self.policy['expected_pins'].pop('controls')
        self.payloads.pop('controls')
        self.records[0]['data'].pop('kernel_identity')
        self.records[0]['data'].pop('build_ids')
        self.update_journal()
        result = self.analyze()
        self.assert_invalid(result)
        codes = {item['code'] for item in result['blockers']}
        self.assertIn('PRODUCER_CONTROL_EXPORT_MISSING', codes)
        self.assertIn('PRODUCER_JOURNAL_LIFETIME_BUILD_IDS_MISSING', codes)
        self.assertEqual(self.edge(result, 'exported_controls'), 'missing')
        self.assertIsNone(result['presentation_analyzer'])

    def test_nested_posts_are_not_reconstructed_into_controls(self):
        self.add_json('controls', [{'native_result': self.controls[0]['native_result']}])
        self.assert_invalid(self.analyze())

    def test_current_last_control_close_cutoff_conflict(self):
        self.controls[-1]['next_started_unix_ns'] = unix(1 / 8)
        self.add_json('controls', self.controls)
        result = self.analyze()
        self.assert_invalid(result)
        self.assertIn('LAST_CONTROL_CLOSE_CUTOFF_CONFLICT', {item['code'] for item in result['blockers']})

    def test_independent_policy_is_required_before_any_read(self):
        class PermissiveReader:
            def read(self, item):
                raise AssertionError('must never be called')
        result = combined.read_combined(self.spec, reader=PermissiveReader(), policy=self.policy)
        self.assert_invalid(result)
        self.spec['artifacts']['launch']['sha256'] = '0' * 64
        self.assert_invalid(self.analyze())

    def test_role_map_and_scalar_types_are_exact(self):
        for mutation in ('unknown', 'missing', 'bool_size', 'alias'):
            spec = copy.deepcopy(self.spec)
            if mutation == 'unknown':
                spec['artifacts']['installation'] = spec['artifacts']['launch']
            elif mutation == 'missing':
                spec['artifacts'].pop('helper_build')
            elif mutation == 'bool_size':
                spec['artifacts']['launch']['size'] = True
            else:
                spec['artifacts']['launch'] = spec['artifacts']['result']
            result = combined.read_combined(spec, reader=combined.DescriptorReader(), policy=self.policy)
            self.assert_invalid(result)

    def test_aggregate_metadata_binary_and_artifact_budgets(self):
        for budget in self.policy['budgets']:
            original = self.policy['budgets'][budget]
            self.policy['budgets'][budget] = 1
            self.assert_invalid(self.analyze())
            self.policy['budgets'][budget] = original

    def test_hash_size_and_own_identity_substitution(self):
        self.payloads['launch'] += b' '
        self.assert_invalid(self.analyze())
        self.payloads['launch'] = self.payloads['launch'][:-1]
        bundle = {'schema': combined.BUNDLE_SCHEMA, 'input': self.spec, 'identity': '0' * 64,
                  'payloads': self.payloads, 'observations': {}}
        self.assert_invalid(combined.analyze_combined(bundle, policy=self.policy))

    def test_strict_json_encoding_duplicates_numbers_nesting(self):
        for payload in (b'{"a":1,"a":2}', b'"\xff"', b'{"x":NaN}', b'{"x":1e400}',
                        b'{"x":1e-999}', b'{"x":5e-324}', b'"\\ud800"', b'[' * 65 + b'0' + b']' * 65,
                        b'{"x":18446744073709551616}'):
            with self.subTest(payload=payload[:50]), self.assertRaises((ValueError, UnicodeError)):
                combined.parse_json(payload)
        with self.assertRaises(ValueError):
            combined.parse_json(b' ' * (combined.MAX_JSON_BYTES + 1))

    def test_source_build_helper_and_library_substitution(self):
        for role, field in (('campaign_build', 'source_after'), ('helper_build', 'source_sha256'),
                            ('presentation_build', 'output_path')):
            original = copy.deepcopy(self.values[role])
            self.values[role][field] = {} if field == 'source_after' else 'unrelated'
            self.add_json(role, self.values[role])
            self.assert_invalid(self.analyze())
            self.add_json(role, original)
        self.values['launch']['command'][-1] = '/unrelated/library'
        self.add_json('launch', self.values['launch'])
        self.assert_invalid(self.analyze())

    def test_campaign_top_executable_pid_never_application_identity(self):
        self.values['campaign_build']['built_executables'] = {'/synthetic/Python-NOT-APPLICATION': self.item('application')['sha256']}
        self.add_json('campaign_build', self.values['campaign_build'])
        result = self.analyze()
        self.assert_invalid(result)
        self.assertEqual(self.edge(result, 'campaign_build_sources_application'), 'missing')

    def test_package_hash_is_not_source_proof(self):
        self.add_json('app_package_build', {'executable_sha256': self.item('application')['sha256']})
        result = self.analyze()
        self.assert_invalid(result)
        self.assertEqual(self.edge(result, 'app_package_build_source'), 'unverifiable')

    def test_pid_reuse_window_focus_display_and_exact_geometry(self):
        for field, value in (('window_id', 92), ('frontmost', False), ('pid', True), ('bounds', [0.0, 0, 800, 600])):
            controls = copy.deepcopy(self.controls)
            controls[0]['binding_after'][field] = value
            self.add_json('controls', controls)
            self.assert_invalid(self.analyze())
        controls = copy.deepcopy(self.controls)
        controls[0]['binding_after']['kernel_identity']['start_tvusec'] += 1
        self.add_json('controls', controls)
        self.assert_invalid(self.analyze())

    def test_session_kernel_build_id_substitution(self):
        self.records[0]['data']['kernel_identity']['start_tvsec'] += 1
        self.update_journal()
        self.assert_invalid(self.analyze())
        self.records[0]['data']['kernel_identity']['start_tvsec'] -= 1
        self.records[0]['data']['build_ids']['helper_sha256'] = '0' * 64
        self.update_journal()
        self.assert_invalid(self.analyze())

    def test_fence_sequence_acquisition_and_clock_substitution(self):
        original = copy.deepcopy(self.records)
        for mutation in ('fence', 'sequence', 'acquisition', 'clock', 'counter'):
            self.records = copy.deepcopy(original)
            if mutation == 'fence':
                next(item['data'] for item in self.records if item['kind'] == 'frame')['fence'] = 2
            elif mutation == 'sequence':
                next(item['data'] for item in self.records if item['kind'] == 'input')['input_sequence'] = 2
            elif mutation == 'acquisition':
                next(item['data'] for item in self.records if item['kind'] == 'presentation_event'
                     and item['data']['kind'] == 6)['acquisition_sequence'] = 99
            elif mutation == 'clock':
                next(item['data'] for item in self.records if item['kind'] == 'presentation_sleep_clock')['clock_api'] = 'Unix'
            else:
                next(item['data'] for item in self.records if item['kind'] == 'presentation_end')['summary']['counters']['emitted_events'] += 1
            self.update_journal()
            self.assert_invalid(self.analyze())

    def test_context_precedes_frame_two_pass_without_sort(self):
        result = self.analyze()
        self.assert_consistent_except_missing_save(result)
        context_index = next(index for index, item in enumerate(self.records)
                             if item['kind'] == 'presentation_event' and item['data']['kind'] == 2)
        self.records[context_index], self.records[context_index + 1] = self.records[context_index + 1], self.records[context_index]
        self.update_journal()
        self.assert_invalid(self.analyze())

    def test_malformed_truncated_drops_late_records_and_open_spans(self):
        original = copy.deepcopy(self.records)
        for mutation in ('truncated', 'dropped', 'late', 'open_span', 'unknown', 'explicit_error'):
            self.records = copy.deepcopy(original)
            if mutation == 'truncated':
                self.add('journal', self.journal_bytes()[:-1])
            else:
                if mutation == 'dropped':
                    self.records[-1]['data']['dropped_records'] = 1
                elif mutation == 'late':
                    self.records.append(copy.deepcopy(self.records[-2]))
                elif mutation == 'open_span':
                    self.records.insert(-1, {'kind': 'span_begin', 'elapsed_us': 1_300_000, 'dropped_records': 0,
                                             'data': {'span_id': 1, 'name': 'worker'}})
                elif mutation == 'unknown':
                    self.records[2]['kind'] = 'presentation_cached_summary'
                else:
                    self.records[2]['data']['observer_error'] = 'synthetic failure'
                self.update_journal()
            self.assert_invalid(self.analyze())

    def test_record_control_and_journal_hard_bounds(self):
        with self.assertRaises(ValueError):
            combined.journal_records(b'{}\n' * (combined.MAX_RECORDS + 1))
        with self.assertRaises(ValueError):
            combined.journal_records(b'\n' * (combined.MAX_JOURNAL_BYTES + 1))
        self.add_json('controls', self.controls * (combined.MAX_CONTROLS + 1))
        self.assert_invalid(self.analyze())

    def test_cached_records_buffer_disk_claims_never_replace_reparse(self):
        for field, value in (('records', []), ('journal_bytes', 1), ('journal_buffer', {'bytes': 1, 'sha256': '0' * 64}),
                             ('journal_on_disk', {'sha256': '0' * 64})):
            original = copy.deepcopy(self.values['result'])
            self.values['result'][field] = value
            self.add_json('result', self.values['result'])
            self.assert_invalid(self.analyze())
            self.add_json('result', original)

    def test_signal_timeout_interruption_cleanup_report_errors_preserved(self):
        for status, raw, code, extra in (
            ('exited', -9, -9, {'signal': 9}),
            ('timeout', 0, 124, {'timed_out': True}),
            ('interrupted', 0, 130, {'interrupted': 'KeyboardInterrupt'}),
            ('diagnostic_failed', 0, 1, {'report_error': 'cannot retain report'}),
            ('exited', 0, 0, {'forced_teardown': True}),
            ('exited', 0, 0, {'post_errors': ['changed input']}),
            ('exited', 0, 0, {'cleanup': [{'error': 'cleanup failed'}]})):
            diagnostic = copy.deepcopy(self.values['result'])
            diagnostic.update(status=status, actual_child_returncode=raw, exit_code=code, **extra)
            self.add_json('result', diagnostic)
            result = self.analyze()
            self.assert_invalid(result)
            self.assertEqual(result['outcomes']['raw_child_exit'], raw)
            self.assertEqual(result['outcomes']['diagnostic_code'], code)
            for field, value in extra.items():
                self.assertEqual(result['outcomes']['result'][field], value)

    def test_presentation_process_cleanup_staging_and_report_failures(self):
        original = copy.deepcopy(self.values['presentation_build'])
        for mutation in ('process', 'cleanup', 'staging', 'report'):
            value = copy.deepcopy(original)
            if mutation == 'process':
                value['process']['returncode'] = True
            elif mutation == 'cleanup':
                value['cleanup']['reaped'] = False
            elif mutation == 'staging':
                value['sources'][0]['staged_after']['sha256'] = '0' * 64
            else:
                value['receipt_write_error'] = 'cannot write receipt'
            self.add_json('presentation_build', value)
            self.assert_invalid(self.analyze())

    def test_descriptor_paths_traversal_nul_and_unapproved_roots(self):
        for path in ('relative', str(self.directory) + '/../escape', str(self.directory) + '/nul\0x', '/saves/private.json'):
            item = self.spec['artifacts']['launch']
            item['path'] = path
            self.policy['expected_pins']['launch'] = copy.deepcopy(item)
            self.assert_invalid(self.analyze())

    def test_nofollow_rejects_file_and_directory_symlinks(self):
        item = self.spec['artifacts']['launch']
        original = Path(item['path'])
        link = self.directory / 'symlink'
        link.symlink_to(original)
        with self.assertRaises((OSError, ValueError)):
            combined.DescriptorReader().read(dict(item, path=str(link)))
        directory_link = self.directory / 'directory-link'
        directory_link.symlink_to(self.directory, target_is_directory=True)
        with self.assertRaises((OSError, ValueError)):
            combined.DescriptorReader().read(dict(item, path=str(directory_link / original.name)))

    def test_hardlinks_and_fifo_are_rejected_without_blocking(self):
        item = self.spec['artifacts']['launch']
        hardlink = self.directory / 'hardlink'
        os.link(item['path'], hardlink)
        with self.assertRaises(ValueError):
            combined.DescriptorReader().read(item)
        fifo = self.directory / 'fifo'
        os.mkfifo(fifo)
        with self.assertRaises(ValueError):
            combined.DescriptorReader().read(dict(item, path=str(fifo)))

    def test_descriptor_mutation_and_path_substitution_races(self):
        item = copy.deepcopy(self.spec['artifacts']['launch'])
        original_read = os.read
        for mutation in ('inplace', 'replace'):
            Path(item['path']).write_bytes(self.payloads['launch'])
            invoked = []

            def racing_read(descriptor_fd, length):
                chunk = original_read(descriptor_fd, length)
                if not invoked:
                    invoked.append(True)
                    if mutation == 'inplace':
                        Path(item['path']).write_bytes(b'X' * item['size'])
                    else:
                        replacement = self.directory / 'replacement'
                        replacement.write_bytes(self.payloads['launch'])
                        os.replace(replacement, item['path'])
                return chunk

            os.read = racing_read
            try:
                with self.assertRaises(ValueError):
                    combined.DescriptorReader().read(item)
            finally:
                os.read = original_read

    def test_descriptor_open_substitution_race_and_stream_chunk_bound(self):
        item = copy.deepcopy(self.spec['artifacts']['launch'])
        original_open = os.open
        invoked = []

        def racing_open(path, flags, *arguments, **keywords):
            if path == Path(item['path']).name and not invoked:
                invoked.append(True)
                replacement = self.directory / 'replacement-before-open'
                replacement.write_bytes(self.payloads['launch'])
                os.replace(replacement, item['path'])
            return original_open(path, flags, *arguments, **keywords)

        os.open = racing_open
        try:
            with self.assertRaises(ValueError):
                combined.DescriptorReader().read(item)
        finally:
            os.open = original_open
        self.add('binary:large', b'a' * (combined.CHUNK_BYTES * 3 + 17))
        lengths, original_read = [], os.read

        def bounded_read(descriptor_fd, length):
            lengths.append(length)
            return original_read(descriptor_fd, length)

        os.read = bounded_read
        try:
            observation = combined.DescriptorReader().read(self.policy['expected_pins']['binary:large'])
        finally:
            os.read = original_read
        self.assertGreater(len(lengths), 3)
        self.assertLessEqual(max(lengths), combined.CHUNK_BYTES)
        self.assertEqual(observation['before'], observation['after'])

    def test_audit_guard_denies_process_network_native_and_private_reads(self):
        for event in ('subprocess.Popen', 'socket.connect', 'ctypes.dlopen', 'os.kill', 'os.posix_spawn', 'signal.signal'):
            with self.assertRaises(RuntimeError):
                sys.audit(event)
        for module in ('subprocess', 'socket', 'ctypes', 'AppKit', 'window_session', 'play_alpha'):
            with self.assertRaises(RuntimeError):
                audit_guard('import', (module,))
        with self.assertRaises(RuntimeError):
            open('/saves/private-journal.json', 'rb')
        self.assertFalse(any(name in sys.modules for name in FORBIDDEN_MODULES))

    def test_helper_exact_schema_command_and_driver_pin(self):
        original = copy.deepcopy(self.values['helper_build'])
        for mutation in ('extra_field', 'wrong_output', 'duplicate_source', 'empty_compiler'):
            value = copy.deepcopy(original)
            if mutation == 'extra_field':
                value['qualified'] = True
            elif mutation == 'wrong_output':
                value['command'][-1] = '/unrelated/helper'
            elif mutation == 'duplicate_source':
                value['command'].append(value['source_path'])
            else:
                value['compiler'] = ' '
            self.add_json('helper_build', value)
            self.assert_invalid(self.analyze())
        self.add_json('helper_build', original)
        diagnostic = copy.deepcopy(self.values['result'])
        diagnostic['inputs'][0]['pinned_after']['sha256'] = '0' * 64
        self.add_json('result', diagnostic)
        self.assert_invalid(self.analyze())

    def test_loaded_attestation_library_source_and_path_claims_do_not_pass(self):
        library = next(item['data'] for item in self.records if item['kind'] == 'presentation_library')
        original = copy.deepcopy(library)
        for field, value in (('path', '/unrelated/descriptive-library'), ('source_sha256', '0' * 64),
                             ('loaded_binary_receipt', True), ('loaded_binary_source_attested', True)):
            library.clear()
            library.update(original)
            library[field] = value
            self.update_journal()
            self.assert_invalid(self.analyze())

    def test_false_unchanged_flags_and_mutated_build_inputs(self):
        diagnostic = copy.deepcopy(self.values['result'])
        diagnostic['inputs'][0]['unchanged'] = False
        self.add_json('result', diagnostic)
        self.assert_invalid(self.analyze())
        build = copy.deepcopy(self.values['campaign_build'])
        build['source_unchanged'] = 1
        self.add_json('campaign_build', build)
        self.assert_invalid(self.analyze())

    def test_forced_cleanup_cannot_hide_behind_zero_exit(self):
        diagnostic = copy.deepcopy(self.values['result'])
        diagnostic['cleanup'] = [{'operation': 'kill', 'attempted': True, 'error': None}]
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assert_invalid(result)
        self.assertEqual(result['outcomes']['raw_child_exit'], 0)
        self.assertEqual(result['outcomes']['result']['cleanup'], diagnostic['cleanup'])

    def test_helper_request_pid_and_lifetime_are_not_wrapper_pid(self):
        diagnostic = copy.deepcopy(self.values['result'])
        diagnostic['native_commands'][0]['request']['pid'] = 12345
        self.add_json('result', diagnostic)
        self.assert_invalid(self.analyze())
        diagnostic['native_commands'][0]['request']['pid'] = 42
        self.add_json('result', diagnostic)
        self.assert_consistent_except_missing_save(self.analyze())

    def test_reader_cannot_be_an_instance_callback_and_always_has_hard_limit(self):
        reader = combined.DescriptorReader()
        with self.assertRaises(AttributeError):
            reader.read = lambda item: b'permissive'
        item = dict(self.spec['artifacts']['launch'], size=combined.MAX_BINARY_FILE_BYTES + 1)
        with self.assertRaises(ValueError):
            reader.read(item)

    def test_corrupt_observations_and_unapproved_payload_roles(self):
        role = 'launch'
        self.observations[role] = {'contract': 'descriptor-nofollow-nonblocking-streaming-prepost-v1',
            'sha256': self.spec['artifacts'][role]['sha256'], 'size': self.spec['artifacts'][role]['size'],
            'before': {'inode': 1}, 'after': {'inode': 2}}
        self.assert_invalid(self.analyze())
        self.observations.clear()
        self.payloads['unapproved'] = b'never authorized'
        self.assert_invalid(self.analyze())

    def test_receipt_paths_are_never_added_to_read_authority(self):
        launch = copy.deepcopy(self.values['launch'])
        launch['save'] = '/saves/private-journal.json'
        launch['input'] = '/installation/private-save.json'
        launch['build_receipt'] = '/unapproved/package.json'
        self.add_json('launch', launch)
        result = combined.read_combined(self.spec, reader=combined.DescriptorReader(), policy=self.policy)
        self.assert_consistent_except_missing_save(result)

    def test_large_json_integer_is_rejected_before_conversion(self):
        with self.assertRaises(ValueError):
            combined.parse_json(b'{"value":' + b'1' * 100_000 + b'}')

    def test_producer_result_is_preserved_separately_without_endorsement(self):
        result = self.analyze()
        self.assertEqual(result['producer_result'], self.values['result'])
        result['producer_result']['pid'] = -1
        self.assertEqual(self.values['result']['pid'], 42)
        self.assertFalse(result['runtime_proof'])

    def test_analyzer_and_rust_collector_have_separate_source_identities(self):
        result = self.analyze()
        identity = result['analyzer_source_identity']
        self.assertEqual(identity['sha256'], self.item('analyzer_source')['sha256'])
        self.assertEqual(result['collector_source_identity']['sha256'], self.item('collector_source')['sha256'])
        self.assertNotEqual(identity['sha256'], result['collector_source_identity']['sha256'])
        self.assert_consistent_except_missing_save(result)

    def test_malformed_role_payloads_produce_structured_errors(self):
        for role in ('launch', 'result', 'helper_build', 'controls', 'campaign_build', 'presentation_build'):
            original = self.payloads[role]
            self.add(role, b'null')
            self.assert_invalid(self.analyze())
            self.add(role, original)

    def test_false_error_scalar_is_not_silently_projected_away(self):
        self.records[2]['data']['error'] = False
        self.update_journal()
        result = self.analyze()
        self.assert_invalid(result)
        self.assertIsNone(result['original_analyzer'])

    def test_missing_and_late_native_lifecycle_are_not_trimmed_or_repaired(self):
        original = copy.deepcopy(self.records)
        for mutation in ('missing_end', 'late_callback', 'snapshot_accounting'):
            self.records = copy.deepcopy(original)
            if mutation == 'missing_end':
                self.records = [item for item in self.records if item['kind'] != 'presentation_end']
            elif mutation == 'late_callback':
                callback = copy.deepcopy(next(item for item in self.records if item['kind'] == 'presentation_event'
                                              and item['data']['kind'] == 6))
                self.records.insert(-1, callback)
            else:
                next(item['data'] for item in self.records if item['kind'] == 'presentation_end')['summary']['last_event_sequence'] -= 1
            self.update_journal()
            before = copy.deepcopy(self.records)
            self.assert_invalid(self.analyze())
            self.assertEqual(self.records, before)

    def test_driver_metadata_source_paths_are_not_followed(self):
        diagnostic = copy.deepcopy(self.values['result'])
        for item in diagnostic['inputs']:
            if item['name'] in ('plan.json', 'endpoints.json', 'helper-build.json'):
                item['source'] = '/never/read/original/' + item['name']
        self.add_json('result', diagnostic)
        result = combined.read_combined(self.spec, reader=combined.DescriptorReader(), policy=self.policy)
        self.assert_consistent_except_missing_save(result)

    def test_presentation_compiler_command_cannot_substitute_staged_source(self):
        build = copy.deepcopy(self.values['presentation_build'])
        build['command'][2] = '/never/read/unrelated-native-source.m'
        self.add_json('presentation_build', build)
        result = self.analyze()
        self.assert_invalid(result)
        self.assertEqual(self.edge(result, 'presentation_build_source_output_library'), 'mismatch')


    def test_analyzer_and_collector_roles_cannot_be_swapped(self):
        bindings = self.policy['bindings']
        bindings['analyzer_source'], bindings['collector_source'] = (
            bindings['collector_source'], bindings['analyzer_source'])
        result = self.analyze()
        self.assertEqual(self.edge(result, 'offline_collector_source_contract'), 'mismatch')
        self.assert_invalid(result)

    def test_consistently_pinned_wrong_analyzer_is_not_this_analyzer_contract(self):
        self.add('source:' + combined.ANALYZER_SOURCE, b'authored unrelated Python analyzer')
        result = self.analyze()
        self.assertEqual(self.edge(result, 'offline_collector_source_contract'), 'mismatch')
        self.assert_invalid(result)

    def test_save_identity_is_unavailable_even_with_synthetic_save_fields(self):
        self.records[0]['data']['save_identity'] = {'path': '/saves/never-follow', 'sha256': '1' * 64}
        diagnostic = copy.deepcopy(self.values['result'])
        diagnostic['selected_save_identity'] = {'path': '/other-save/never-follow', 'sha256': '2' * 64}
        self.add_json('result', diagnostic)
        self.update_journal()
        self.assert_consistent_except_missing_save(self.analyze())

    def test_missing_embedded_launcher_result_is_missing_not_matched(self):
        launch = copy.deepcopy(self.values['launch'])
        launch.pop('diagnostic')
        self.add_json('launch', launch)
        result = self.analyze()
        self.assertEqual(self.edge(result, 'process_outcomes'), 'missing')
        self.assert_invalid(result)

    def test_mismatched_embedded_launcher_result_is_rejected(self):
        launch = copy.deepcopy(self.values['launch'])
        launch['diagnostic']['pid'] = 43
        self.add_json('launch', launch)
        result = self.analyze()
        self.assertEqual(self.edge(result, 'process_outcomes'), 'mismatch')
        self.assert_invalid(result)

    def test_native_result_requires_explicit_commands_associations_and_posts(self):
        original = copy.deepcopy(self.values['result'])
        for field in ('native_commands', 'control_native_associations', 'native_posts'):
            with self.subTest(field=field):
                diagnostic = copy.deepcopy(original)
                diagnostic.pop(field)
                self.add_json('result', diagnostic)
                result = self.analyze()
                self.assertEqual(self.edge(result, 'control_native_results'), 'missing')
                self.assert_invalid(result)

    def test_native_helper_semantics_cannot_be_replaced_by_unrelated_exports(self):
        original = copy.deepcopy(self.values['result'])
        for mutation in ('operation', 'code', 'press_ms', 'released', 'posts', 'error', 'release_error'):
            with self.subTest(mutation=mutation):
                diagnostic = copy.deepcopy(original)
                response = json.loads(diagnostic['native_commands'][1]['stdout'])
                if mutation == 'operation':
                    response[mutation] = 'observe'
                elif mutation in ('code', 'press_ms'):
                    response[mutation] += 1
                elif mutation == 'released':
                    response[mutation] = False
                elif mutation == 'posts':
                    response[mutation][0]['unix_before_ns'] += 1
                else:
                    response[mutation] = 'authored helper failure'
                diagnostic['native_commands'][1]['stdout'] = combined.canonical(response).decode()
                self.add_json('result', diagnostic)
                result = self.analyze()
                self.assertEqual(self.edge(result, 'control_native_results'), 'mismatch')
                self.assert_invalid(result)

    def test_native_posts_must_equal_the_actual_associated_key_response(self):
        diagnostic = copy.deepcopy(self.values['result'])
        diagnostic['native_posts']['code'] = 36
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assertEqual(self.edge(result, 'control_native_results'), 'mismatch')
        self.assert_invalid(result)

    def test_native_associations_reject_reordered_boolean_and_wrong_control_indices(self):
        original = copy.deepcopy(self.values['result'])
        for field, value in (('key_command_index', True), ('before_command_index', 2),
                             ('after_command_index', 999), ('control_index', 1)):
            with self.subTest(field=field):
                diagnostic = copy.deepcopy(original)
                diagnostic['control_native_associations'][0][field] = value
                self.add_json('result', diagnostic)
                result = self.analyze()
                self.assertEqual(self.edge(result, 'control_native_results'), 'mismatch')
                self.assert_invalid(result)

    def test_unassociated_additional_key_command_is_rejected(self):
        diagnostic = copy.deepcopy(self.values['result'])
        diagnostic['native_commands'].append(copy.deepcopy(diagnostic['native_commands'][1]))
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assertEqual(self.edge(result, 'control_native_results'), 'mismatch')
        self.assert_invalid(result)

    def relocate_staged(self, suffix, relative):
        role = 'source:staged/crates/sims-app/native/window_present_macos' + suffix
        item = copy.deepcopy(self.policy['expected_pins'][role])
        old_path = item['path']
        replacement = self.directory / relative
        replacement.parent.mkdir(exist_ok=True)
        replacement.write_bytes(self.payloads[role])
        item['path'] = str(replacement)
        self.policy['expected_pins'][role] = item
        self.spec['sources'][role.split(':', 1)[1]] = copy.deepcopy(item)
        build = copy.deepcopy(self.values['presentation_build'])
        next(entry for entry in build['sources'] if entry['staged_path'] == old_path)['staged_path'] = str(replacement)
        build['command'] = [str(replacement) if value == old_path else value for value in build['command']]
        self.add_json('presentation_build', build)

    def test_matching_staged_header_hash_in_another_directory_is_not_compiler_input(self):
        self.relocate_staged('.h', 'unrelated/window_present_macos.h')
        result = self.analyze()
        self.assertEqual(self.edge(result, 'presentation_build_source_output_library'), 'mismatch')
        self.assert_invalid(result)

    def test_flattened_staged_native_names_do_not_match_import_recipe(self):
        self.relocate_staged('.m', 'staged/flattened.synthetic')
        result = self.analyze()
        self.assertEqual(self.edge(result, 'presentation_build_source_output_library'), 'mismatch')
        self.assert_invalid(result)

    def test_audit_rejects_outside_directory_before_descriptor_traversal(self):
        with self.assertRaisesRegex(RuntimeError, 'undeclared file read'):
            os.open('/etc', os.O_RDONLY | os.O_DIRECTORY)
        root_fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY)
        try:
            with self.assertRaisesRegex(RuntimeError, 'undeclared file read'):
                os.open('etc', os.O_RDONLY | os.O_DIRECTORY, dir_fd=root_fd)
        finally:
            os.close(root_fd)

    def test_audit_rejects_raw_relative_basename_even_with_a_directory_descriptor(self):
        directory_fd = os.open(str(self.directory), os.O_RDONLY | os.O_DIRECTORY)
        try:
            basename = Path(self.spec['artifacts']['launch']['path']).name
            with self.assertRaisesRegex(RuntimeError, 'unanchored relative read'):
                ORIGINAL_OS_OPEN(basename, os.O_RDONLY, dir_fd=directory_fd)
        finally:
            os.close(directory_fd)

    def test_audit_rejects_unregistered_descriptor_relative_and_integer_reads(self):
        unknown_fd = ORIGINAL_OS_OPEN('/', os.O_RDONLY | os.O_DIRECTORY)
        OPEN_DESCRIPTORS.pop(unknown_fd, None)
        try:
            with self.assertRaisesRegex(RuntimeError, 'unknown descriptor'):
                os.open('etc', os.O_RDONLY, dir_fd=unknown_fd)
            with self.assertRaisesRegex(RuntimeError, 'unknown descriptor'):
                os.read(unknown_fd, 1)
            with self.assertRaisesRegex(RuntimeError, 'unknown descriptor'):
                open(unknown_fd, 'rb', closefd=False)
            with self.assertRaisesRegex(RuntimeError, 'unknown descriptor'):
                os.listdir(unknown_fd)
        finally:
            ORIGINAL_OS_CLOSE(unknown_fd)

    def test_audit_rejects_stale_descriptor_identity(self):
        directory_fd = os.open(str(self.directory), os.O_RDONLY | os.O_DIRECTORY)
        absolute, identity = OPEN_DESCRIPTORS[directory_fd]
        OPEN_DESCRIPTORS[directory_fd] = (absolute, (identity[0], identity[1] + 1))
        try:
            with self.assertRaisesRegex(RuntimeError, 'stale descriptor'):
                os.open('never-read', os.O_RDONLY, dir_fd=directory_fd)
        finally:
            os.close(directory_fd)

    def test_exact_compiled_python_sources_match_declared_module_files(self):
        for name, source_hash in EXECUTED_SOURCE_PINS.items():
            self.assertEqual(combined.digest(Path(sys.modules[name].__file__).read_bytes()), source_hash)
        self.assertEqual(EXECUTED_SOURCE_PINS['window_presentation_evidence'], combined.ANALYZER_SHA256)

    def test_helper_native_brackets_do_not_reconstruct_or_contradict_enclosing_exports(self):
        diagnostic = copy.deepcopy(self.values['result'])
        diagnostic['native_commands'][1]['started_unix_ns'] = self.controls[0]['started_unix_ns'] - 1
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assertEqual(self.edge(result, 'control_native_results'), 'mismatch')
        self.assert_invalid(result)

    def test_posts_must_fit_inside_the_associated_key_helper_invocation(self):
        diagnostic = copy.deepcopy(self.values['result'])
        command = diagnostic['native_commands'][1]
        command['started_unix_ns'] = self.controls[0]['finished_unix_ns'] - 2
        command['finished_unix_ns'] = self.controls[0]['finished_unix_ns'] - 1
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assertEqual(self.edge(result, 'control_native_results'), 'mismatch')
        self.assert_invalid(result)

    def test_falsy_malformed_outcome_errors_are_independent_mismatches(self):
        diagnostic_original = copy.deepcopy(self.values['result'])
        launch_original = copy.deepcopy(self.values['launch'])
        cases = [('result', name, value)
                 for name in ('post_errors', 'journal_read_errors', 'journal_inspection_errors')
                 for value in (False, 0, 0.0, '', {}, None)]
        cases += [(role, name, value)
                  for role, names in (('result', ('report_error',)), ('launch', (
                      'launch_error', 'save_error', 'publication_error', 'integrity_error', 'report_error')))
                  for name in names for value in (False, 0, 0.0, [], {})]
        for role, name, value in cases:
            with self.subTest(role=role, field=name, value=value):
                self.add_json('result', diagnostic_original)
                self.add_json('launch', launch_original)
                mutated = copy.deepcopy(self.values[role])
                mutated[name] = value
                self.add_json(role, mutated)
                result = self.analyze()
                self.assertEqual(self.edge(result, 'process_outcomes'), 'mismatch')
                self.assertEqual(result['outcomes']['result' if role == 'result' else 'launcher'][name], value)
                self.assert_invalid(result)


class VersionedCameraExportTests(unittest.TestCase):
    """Guarded synthetic contracts only; UNRUN in the source-only candidate."""

    setUpClass = classmethod(CombinedEvidenceTests.setUpClass.__func__)
    tearDownClass = classmethod(CombinedEvidenceTests.tearDownClass.__func__)
    setUp = CombinedEvidenceTests.setUp
    add = CombinedEvidenceTests.add
    add_json = CombinedEvidenceTests.add_json
    native_command = CombinedEvidenceTests.native_command
    item = CombinedEvidenceTests.item
    state = CombinedEvidenceTests.state
    receipt_pin = CombinedEvidenceTests.receipt_pin
    journal_bytes = CombinedEvidenceTests.journal_bytes
    update_journal = CombinedEvidenceTests.update_journal
    analyze = CombinedEvidenceTests.analyze
    edge = CombinedEvidenceTests.edge
    assert_unqualified = CombinedEvidenceTests.assert_unqualified
    assert_invalid = CombinedEvidenceTests.assert_invalid
    assert_consistent_except_missing_save = CombinedEvidenceTests.assert_consistent_except_missing_save

    def versioned(self):
        diagnostic = copy.deepcopy(self.values['result'])
        raw = copy.deepcopy(self.controls[0]['native_result'])
        posts = raw['posts']
        legacy = copy.deepcopy(raw)
        legacy['press_duration_us'] = {
            'lower': (posts[1]['host_before_ns'] - posts[0]['host_after_ns']) / 1000,
            'upper': (posts[1]['host_after_ns'] - posts[0]['host_before_ns']) / 1000,
            'clock': 'DispatchTime uptime nanoseconds; post-call brackets, not app receipt or presentation time'}
        diagnostic.update(control_export_contract='sims-window-camera-control-raw-v2',
            control_export_status='exported_unqualified', control_export_offline_only=True,
            control_export_qualified=False, control_export_integration_reviewed=False,
            control_export_driver_enabled=False, control_export_error=None, controls_write_error=None,
            exported_controls=copy.deepcopy(self.controls), native_posts_raw_v2=raw, native_posts=legacy,
            close_started_unix_ns=unix(1 / 8))
        self.add_json('result', diagnostic)
        return diagnostic

    def assert_edge_mismatch(self, result, edge, detail=None):
        self.assertEqual(self.edge(result, edge), 'mismatch', result)
        self.assertTrue(any(item['edge'] == edge for item in result['errors']), result)
        if detail is not None:
            joined = next(item for item in result['joins'] if item['edge'] == edge)
            self.assertIn(detail, joined['detail'])
        self.assert_invalid(result)
        self.assertIn('PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING',
                      {item['code'] for item in result['blockers']})

    def test_exact_v2_join_keeps_missing_save_and_all_qualification_gates_false(self):
        self.versioned()
        result = self.analyze()
        self.assert_consistent_except_missing_save(result)
        self.assertEqual(self.edge(result, 'exported_controls'), 'matched')
        self.assertEqual(self.edge(result, 'control_native_results'), 'matched')
        self.assertTrue(result['presentation_analyzer']['valid'])
        self.assertEqual(self.edge(result, 'loaded_image_attestation'), 'unverifiable')
        self.assert_unqualified(result)
        self.assertNotIn('press_duration_us', result['producer_result']['native_posts_raw_v2'])
        self.assertIn('press_duration_us', result['producer_result']['native_posts'])

    def test_legacy_fixture_is_unchanged_and_raw_field_presence_never_selects_v2(self):
        original = copy.deepcopy(self.values['result'])
        self.assertNotIn('control_export_contract', original)
        self.assert_consistent_except_missing_save(self.analyze())
        versioned = self.versioned()
        versioned.pop('control_export_contract')
        self.add_json('result', versioned)
        self.assert_edge_mismatch(self.analyze(), 'control_native_results', 'native_posts differs')
        self.add_json('result', original)
        self.assert_consistent_except_missing_save(self.analyze())

    def test_unknown_or_falsy_tags_do_not_enable_raw_result_fallback(self):
        original = self.versioned()
        for tag in (None, False, 0, '', [], {}, 'sims-window-camera-control-raw-v1',
                    'sims-window-camera-control-raw-v2 '):
            with self.subTest(tag=tag):
                diagnostic = copy.deepcopy(original)
                diagnostic['control_export_contract'] = tag
                self.add_json('result', diagnostic)
                self.assert_edge_mismatch(self.analyze(), 'exported_controls', 'Unknown control export contract')

    def test_raw_export_envelope_stdout_and_legacy_enrichment_mismatch_independently(self):
        original = self.versioned()
        for target in ('raw', 'envelope', 'stdout', 'legacy', 'missing_duration'):
            with self.subTest(target=target):
                diagnostic = copy.deepcopy(original)
                controls = copy.deepcopy(self.controls)
                if target == 'raw':
                    diagnostic['native_posts_raw_v2']['code'] = 36
                elif target == 'envelope':
                    controls[0]['native_result']['code'] = 36
                elif target == 'stdout':
                    response = json.loads(diagnostic['native_commands'][1]['stdout'])
                    response['opaque_diagnostic'] = 'must not be stripped'
                    diagnostic['native_commands'][1]['stdout'] = combined.canonical(response).decode()
                elif target == 'legacy':
                    diagnostic['native_posts']['press_duration_us']['lower'] += 1
                else:
                    diagnostic['native_posts'].pop('press_duration_us')
                diagnostic['exported_controls'] = copy.deepcopy(controls)
                self.add_json('controls', controls)
                self.add_json('result', diagnostic)
                self.assert_edge_mismatch(self.analyze(), 'control_native_results')

    def test_contradictory_stdout_duration_is_preserved_and_rejected_not_stripped(self):
        diagnostic = self.versioned()
        controls = copy.deepcopy(self.controls)
        raw = copy.deepcopy(controls[0]['native_result'])
        raw['press_duration_us'] = {'lower': -1, 'upper': -1, 'clock': 'authored contradiction'}
        controls[0]['native_result'] = copy.deepcopy(raw)
        diagnostic['native_posts_raw_v2'] = copy.deepcopy(raw)
        diagnostic['native_commands'][1]['stdout'] = combined.canonical(raw).decode()
        diagnostic['exported_controls'] = copy.deepcopy(controls)
        self.add_json('controls', controls)
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assert_edge_mismatch(result, 'control_native_results', 'Raw stdout press duration contradicts')
        self.assertEqual(result['producer_result']['native_posts_raw_v2']['press_duration_us'],
                         raw['press_duration_us'])

    def test_controls_file_list_is_bound_exactly_to_diagnostic_not_cached_effect(self):
        self.versioned()
        controls = copy.deepcopy(self.controls)
        controls[0]['finished_unix_ns'] += 1
        self.add_json('controls', controls)
        self.assert_edge_mismatch(self.analyze(), 'exported_controls', 'exact diagnostic exported list')

    def test_extra_stdout_fields_cannot_disappear_from_legacy_derived_copy(self):
        diagnostic = self.versioned()
        controls = copy.deepcopy(self.controls)
        raw = copy.deepcopy(controls[0]['native_result'])
        raw['opaque_diagnostic'] = {'retain': ['all', 'decoded', 'fields']}
        controls[0]['native_result'] = copy.deepcopy(raw)
        diagnostic['native_posts_raw_v2'] = copy.deepcopy(raw)
        diagnostic['native_commands'][1]['stdout'] = combined.canonical(raw).decode()
        diagnostic['exported_controls'] = copy.deepcopy(controls)
        self.add_json('controls', controls)
        self.add_json('result', diagnostic)
        self.assert_edge_mismatch(self.analyze(), 'control_native_results', 'deterministic raw-result enrichment')
        diagnostic['native_posts']['opaque_diagnostic'] = copy.deepcopy(raw['opaque_diagnostic'])
        self.add_json('result', diagnostic)
        self.assert_consistent_except_missing_save(self.analyze())

    def test_readiness_and_cleanup_entries_are_preserved_without_reindex_inference(self):
        diagnostic = self.versioned()
        binding = diagnostic['binding']
        readiness = {'request': {'operation': 'observe', 'pid': 42, 'executable': binding['executable']},
            'started_unix_ns': unix(-1 / 32), 'finished_unix_ns': unix(-1 / 64), 'returncode': 0,
            'stderr': '', 'stdout': combined.canonical({'status': 'not_ready',
                'reason': 'owned_window_unavailable', 'kernel_identity': binding['kernel_identity']}).decode()}
        diagnostic['native_commands'].insert(0, readiness)
        for name in ('before_command_index', 'key_command_index', 'after_command_index'):
            diagnostic['control_native_associations'][0][name] += 1
        for operation in ('release', 'close'):
            command = self.native_command(operation, binding, 4)
            command['started_unix_ns'] = diagnostic['native_commands'][-1]['finished_unix_ns'] + 1
            command['finished_unix_ns'] = command['started_unix_ns'] + 1
            if operation == 'release':
                response = json.loads(command['stdout'])
                response.update(code=124, released=True)
                command['stdout'] = combined.canonical(response).decode()
            diagnostic['native_commands'].append(command)
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assert_consistent_except_missing_save(result)
        self.assertEqual(self.edge(result, 'control_native_results'), 'matched')
        self.assertEqual(result['outcomes']['native_commands'], diagnostic['native_commands'])
        self.assertEqual(diagnostic['control_native_associations'][0], {'control_index': 0,
            'before_command_index': 1, 'key_command_index': 2, 'after_command_index': 3})
        diagnostic['control_native_associations'][0]['key_command_index'] = 1
        self.add_json('result', diagnostic)
        self.assert_edge_mismatch(self.analyze(), 'control_native_results', 'association indices')

    def test_v2_posts_still_fit_helper_own_brackets_not_just_enclosing_window(self):
        diagnostic = self.versioned()
        command = diagnostic['native_commands'][1]
        command['started_unix_ns'] = self.controls[0]['finished_unix_ns'] - 2
        command['finished_unix_ns'] = self.controls[0]['finished_unix_ns'] - 1
        self.add_json('result', diagnostic)
        self.assert_edge_mismatch(self.analyze(), 'control_native_results', 'escape their associated helper invocation')

    def test_v2_requires_exact_actual_indices_and_retains_no_extra_key_guard(self):
        original = self.versioned()
        for name, value in (('control_index', True), ('before_command_index', False),
                            ('key_command_index', 2), ('after_command_index', 1)):
            with self.subTest(name=name, value=value):
                diagnostic = copy.deepcopy(original)
                diagnostic['control_native_associations'][0][name] = value
                self.add_json('result', diagnostic)
                self.assert_edge_mismatch(self.analyze(), 'control_native_results')
        diagnostic = copy.deepcopy(original)
        diagnostic['native_commands'].append(copy.deepcopy(diagnostic['native_commands'][1]))
        self.add_json('result', diagnostic)
        self.assert_edge_mismatch(self.analyze(), 'control_native_results', 'Unassociated or repeated key command')

    def test_new_export_errors_and_success_flags_fail_closed_even_when_falsy(self):
        original = self.versioned()
        cases = [(name, value) for name in ('control_export_error', 'controls_write_error')
                 for value in (False, 0, 0.0, '', [], {})]
        cases += [('control_export_status', 'failed'), ('control_export_offline_only', False),
                  ('control_export_qualified', True), ('control_export_integration_reviewed', True),
                  ('control_export_driver_enabled', True)]
        for name, value in cases:
            with self.subTest(name=name, value=value):
                diagnostic = copy.deepcopy(original)
                diagnostic[name] = value
                self.add_json('result', diagnostic)
                result = self.analyze()
                self.assert_edge_mismatch(result, 'exported_controls', 'unqualified and offline-only')
                if name in ('control_export_error', 'controls_write_error', 'control_export_status'):
                    self.assertEqual(result['outcomes']['result'][name], value)

    def test_existing_falsy_error_rejection_survives_v2_native_and_outcome_contract(self):
        original = self.versioned()
        for name in ('post_errors', 'journal_read_errors', 'journal_inspection_errors'):
            diagnostic = copy.deepcopy(original)
            diagnostic[name] = False
            self.add_json('result', diagnostic)
            self.assert_edge_mismatch(self.analyze(), 'process_outcomes')
        diagnostic = copy.deepcopy(original)
        diagnostic['native_commands'][0]['error'] = False
        self.add_json('result', diagnostic)
        self.assert_edge_mismatch(self.analyze(), 'control_native_results', 'malformed')

    def test_v2_does_not_supply_missing_journal_ids_wholefile_or_selected_save_identity(self):
        self.versioned()
        for name in ('kernel_identity', 'build_ids'):
            self.records[0]['data'].pop(name)
        self.update_journal()
        diagnostic = copy.deepcopy(self.values['result'])
        diagnostic['journal_on_disk']['sha256'] = None
        diagnostic.pop('journal_final_observation', None)
        diagnostic['session_save_identity'] = {'selected_sha256': '0' * 64, 'synthetic': True}
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assertEqual(self.edge(result, 'application_identity'), 'missing')
        self.assertEqual(self.edge(result, 'journal_wholefile_claims'), 'missing')
        self.assertEqual(self.edge(result, 'selected_save_session_identity'), 'unverifiable')
        self.assertIn('PRODUCER_JOURNAL_LIFETIME_BUILD_IDS_MISSING',
                      {item['code'] for item in result['blockers']})
        self.assertIn('PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING',
                      {item['code'] for item in result['blockers']})
        self.assert_unqualified(result)

    def test_v2_does_not_relabel_close_cutoff_or_relax_full_window_presentation_checks(self):
        diagnostic = self.versioned()
        controls = copy.deepcopy(self.controls)
        controls[0]['next_started_unix_ns'] = diagnostic['close_started_unix_ns']
        diagnostic['exported_controls'] = copy.deepcopy(controls)
        self.add_json('controls', controls)
        self.add_json('result', diagnostic)
        self.assert_edge_mismatch(self.analyze(), 'exported_controls', 'must be None')
        self.add_json('controls', self.controls)
        self.versioned()
        closing = next(item['data'] for item in self.records if item['kind'] == 'frame'
                       and item['data']['frame_id'] == self.controls[0]['control_effect']['closing_frame_id'])
        closing['fence'] = 2
        self.update_journal()
        result = self.analyze()
        self.assertEqual(self.edge(result, 'exported_controls'), 'matched')
        self.assertFalse(result['presentation_analyzer']['valid'])
        self.assertTrue(any(item['code'] == 'PRESENTATION_ANALYZER_INVALID' for item in result['errors']))
        self.assert_unqualified(result)


class FinalObservationReaderTests(unittest.TestCase):
    """Additive independently supplied synthetic bytes; authored controls, UNRUN."""

    legacy_final_observation = True

    setUpClass = classmethod(CombinedEvidenceTests.setUpClass.__func__)
    tearDownClass = classmethod(CombinedEvidenceTests.tearDownClass.__func__)
    setUp = CombinedEvidenceTests.setUp
    add = CombinedEvidenceTests.add
    add_json = CombinedEvidenceTests.add_json
    native_command = CombinedEvidenceTests.native_command
    item = CombinedEvidenceTests.item
    state = CombinedEvidenceTests.state
    receipt_pin = CombinedEvidenceTests.receipt_pin
    journal_bytes = CombinedEvidenceTests.journal_bytes
    analyze = CombinedEvidenceTests.analyze
    edge = CombinedEvidenceTests.edge
    assert_unqualified = CombinedEvidenceTests.assert_unqualified

    def tagged(self):
        diagnostic = copy.deepcopy(self.values['result'])
        payload = self.payloads['journal']
        source = self.spec['artifacts']['journal']
        parts = source['path'].split('/')[1:]
        paths = ['/'] + ['/' + '/'.join(parts[:index]) for index in range(1, len(parts) + 1)]
        components = [{'path': path, 'fingerprint': combined.fingerprint(os.stat(path, follow_symlinks=False))}
                      for path in paths]
        observed = combined.DescriptorReader().read(source)
        observed.pop('bytes')
        self.observations['journal'] = observed
        diagnostic['journal_final_observation'] = {
            'contract': combined.FINAL_OBSERVATION_CONTRACT, 'status': 'observed', 'child_collected': True,
            'forced_teardown': False, 'source': {'output_directory': str(self.directory), 'path': source['path']},
            'limit_bytes': combined.MAX_JOURNAL_BYTES, 'complete': True, 'eof': True,
            'bytes_read': len(payload), 'prefix_sha256': combined.digest(payload), 'size_bytes': len(payload),
            'wholefile_sha256': combined.digest(payload), 'components_before': components,
            'components_after': copy.deepcopy(components),
            'earlier_components': [{'path': entry['path'], 'identity': {name: entry['fingerprint'][name]
                for name in ('device', 'inode', 'mode', 'uid')}} for entry in components],
            'previous_size': 0, 'retained_buffer': {'bytes': len(payload), 'sha256': combined.digest(payload),
                                                 'prefix_matches': True},
            'semantic': {'status': 'parsed', 'records': len(self.records), 'error': None},
            'snapshot': {'status': 'not_requested', 'path': None, 'bytes_written': 0,
                         'prefix_sha256': combined.digest(b''), 'sha256': None, 'fingerprint': None, 'error': None},
            'error': None}
        self.add_json('result', diagnostic)
        return diagnostic

    def assert_wholefile_not_matched(self):
        result = self.analyze()
        self.assertIn(self.edge(result, 'journal_wholefile_claims'), ('missing', 'mismatch'), result)
        self.assertIn('PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING', {item['code'] for item in result['blockers']})
        self.assert_unqualified(result)
        return result

    def test_missing_previously_observed_leaf_chain_cannot_match_nonempty_prefix(self):
        original = self.tagged()
        for prefix in ('previous_size', 'retained_buffer'):
            with self.subTest(prefix=prefix):
                value = copy.deepcopy(original)
                final = value['journal_final_observation']
                final['earlier_components'] = final['earlier_components'][:-1]
                if prefix == 'previous_size':
                    final['previous_size'] = 1
                self.add_json('result', value)
                self.assert_wholefile_not_matched()

    def test_populated_legacy_disk_claims_are_checked_separately_from_tagged_whole_bytes(self):
        original = self.tagged()
        for disk in ({'sha256': '0' * 64}, {'sha256': None, 'size': 1}):
            with self.subTest(disk=disk):
                value = copy.deepcopy(original)
                value['journal_on_disk'] = disk
                self.add_json('result', value)
                result = self.analyze()
                self.assertEqual(self.edge(result, 'journal_legacy_disk_claims'), 'mismatch')
                self.assertEqual(self.edge(result, 'journal_wholefile_claims'), 'matched')
                self.assert_unqualified(result)
        value = copy.deepcopy(original)
        value['journal_on_disk'] = {'sha256': None}
        self.add_json('result', value)
        self.assertEqual(self.edge(self.analyze(), 'journal_legacy_disk_claims'), 'matched')

    def test_tagged_wholefile_is_diagnostic_consistency_not_execution_or_save_proof(self):
        self.tagged()
        result = self.analyze()
        self.assertEqual(self.edge(result, 'journal_wholefile_claims'), 'matched', result)
        self.assertEqual(self.edge(result, 'journal_retained_buffer_claims'), 'matched', result)
        self.assertIn('PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING', {item['code'] for item in result['blockers']})
        self.assertEqual(self.edge(result, 'loaded_image_attestation'), 'unverifiable')
        self.assert_unqualified(result)

    def test_legacy_populated_hash_and_field_presence_never_become_final_observation(self):
        self.assertTrue(self.values['result']['journal_on_disk']['sha256'])
        self.assert_wholefile_not_matched()
        for malformed in (None, False, 0, '', [], {},
                          {'wholefile_sha256': self.spec['artifacts']['journal']['sha256']}):
            diagnostic = copy.deepcopy(self.values['result'])
            diagnostic['journal_final_observation'] = malformed
            self.add_json('result', diagnostic)
            self.assert_wholefile_not_matched()

    def test_unknown_falsy_missing_and_mistyped_tags_never_fall_back(self):
        original = self.tagged()
        for tag in (None, False, 0, '', [], {}, 'sims-window-journal-final-observation-v0',
                    'sims-window-journal-final-observation-v1 '):
            diagnostic = copy.deepcopy(original)
            diagnostic['journal_final_observation']['contract'] = tag
            self.add_json('result', diagnostic)
            self.assert_wholefile_not_matched()
        diagnostic = copy.deepcopy(original)
        diagnostic['journal_final_observation'].pop('contract')
        self.add_json('result', diagnostic)
        self.assert_wholefile_not_matched()

    def test_scalar_int_nonbool_list_and_metadata_types_are_strict(self):
        original = self.tagged()
        changes = [('bytes_read', True), ('size_bytes', False), ('limit_bytes', 64.0),
                   ('previous_size', '0'), ('complete', 1), ('eof', 1), ('child_collected', 1),
                   ('forced_teardown', 0), ('components_before', {}), ('components_after', {}),
                   ('earlier_components', {}), ('prefix_sha256', False), ('wholefile_sha256', 1),
                   ('error', ''), ('status', True), ('size_bytes', 2**64), ('previous_size', -1)]
        for field, value in changes:
            with self.subTest(field=field, value=value):
                diagnostic = copy.deepcopy(original)
                diagnostic['journal_final_observation'][field] = value
                self.add_json('result', diagnostic)
                self.assert_wholefile_not_matched()
        for group, field, value in (('semantic', 'records', True), ('semantic', 'status', False),
                                    ('retained_buffer', 'bytes', True), ('retained_buffer', 'prefix_matches', 1),
                                    ('snapshot', 'bytes_written', False), ('snapshot', 'error', 'write failure')):
            diagnostic = copy.deepcopy(original)
            diagnostic['journal_final_observation'][group][field] = value
            self.add_json('result', diagnostic)
            self.assert_wholefile_not_matched()

    def test_failed_child_completeness_and_semantics_cannot_be_repaired_by_hash(self):
        original = self.tagged()
        for field, value in (('complete', False), ('eof', False), ('child_collected', False),
                             ('forced_teardown', True), ('status', 'failed'), ('error', 'read failed')):
            diagnostic = copy.deepcopy(original)
            diagnostic['journal_final_observation'][field] = value
            self.add_json('result', diagnostic)
            self.assert_wholefile_not_matched()
        for value in (None, True, 0.0, '0'):
            diagnostic = copy.deepcopy(original)
            diagnostic['actual_child_returncode'] = value
            self.add_json('result', diagnostic)
            self.assert_wholefile_not_matched()
        diagnostic = copy.deepcopy(original)
        diagnostic['journal_final_observation']['semantic'].update(status='failed', error='malformed record')
        self.add_json('result', diagnostic)
        self.assert_wholefile_not_matched()

    def test_source_manifest_and_actual_object_associations_are_mandatory(self):
        original = self.tagged()
        for path in ('/never/read/receipt-directed/events.jsonl', str(self.directory / 'different')):
            diagnostic = copy.deepcopy(original)
            diagnostic['journal_final_observation']['source']['path'] = path
            self.add_json('result', diagnostic)
            self.assert_wholefile_not_matched()
        self.add_json('result', original)
        original_reader = copy.deepcopy(self.observations['journal'])
        self.observations.clear()
        self.assert_wholefile_not_matched()
        self.observations['journal'] = original_reader
        self.observations['journal']['before']['inode'] += 1
        self.observations['journal']['after']['inode'] += 1
        self.assert_wholefile_not_matched()

    def test_ancestor_leaf_and_original_identity_fingerprints_cannot_be_substituted(self):
        original = self.tagged()
        for index in (0, -2, -1):
            diagnostic = copy.deepcopy(original)
            diagnostic['journal_final_observation']['components_after'][index]['fingerprint']['inode'] += 1
            self.add_json('result', diagnostic)
            self.assert_wholefile_not_matched()
        diagnostic = copy.deepcopy(original)
        diagnostic['journal_final_observation']['earlier_components'][-1]['identity']['inode'] += 1
        self.add_json('result', diagnostic)
        self.assert_wholefile_not_matched()
        diagnostic = copy.deepcopy(original)
        diagnostic['journal_final_observation']['components_before'][-1]['fingerprint']['links'] = True
        diagnostic['journal_final_observation']['components_after'] = copy.deepcopy(
            diagnostic['journal_final_observation']['components_before'])
        self.add_json('result', diagnostic)
        self.assert_wholefile_not_matched()

    def test_independently_supplied_differing_bytes_mismatch_observation_interval(self):
        original = self.tagged()
        self.records[-1]['elapsed_us'] += 1
        self.add('journal', self.journal_bytes())
        self.add_json('result', original)
        self.assert_wholefile_not_matched()

    def test_future_append_is_not_covered_by_prior_wholefile_observation(self):
        original = self.tagged()
        self.add('journal', self.payloads['journal'] + b'{}\n')
        self.add_json('result', original)
        self.assert_wholefile_not_matched()

    def test_retained_prefix_is_separate_and_wrong_legacy_claim_does_not_satisfy_wholefile(self):
        diagnostic = self.tagged()
        diagnostic['journal_bytes'] = 0
        diagnostic['journal_sha256'] = combined.digest(b'')
        diagnostic['journal_buffer'] = {'bytes': 0, 'sha256': combined.digest(b'')}
        diagnostic['journal_final_observation']['retained_buffer'].update(bytes=0, sha256=combined.digest(b''))
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assertEqual(self.edge(result, 'journal_wholefile_claims'), 'matched')
        self.assertEqual(self.edge(result, 'journal_retained_buffer_claims'), 'matched')
        diagnostic['journal_sha256'] = '0' * 64
        self.add_json('result', diagnostic)
        result = self.analyze()
        self.assertEqual(self.edge(result, 'journal_retained_buffer_claims'), 'mismatch')
        diagnostic['journal_final_observation']['wholefile_sha256'] = None
        self.add_json('result', diagnostic)
        self.assert_wholefile_not_matched()

    def test_snapshot_requires_explicit_manifest_bytes_object_and_mode(self):
        payload = self.payloads['journal']
        target = self.directory / 'journal-final-observed.raw'
        target.write_bytes(payload)
        target.chmod(0o600)
        role = 'binary:final-snapshot'
        item = {'path': str(target), 'size': len(payload), 'sha256': combined.digest(payload)}
        self.spec['binaries']['final-snapshot'] = item
        self.policy['expected_pins'][role] = copy.deepcopy(item)
        self.payloads[role] = payload
        observed = combined.DescriptorReader().read(item)
        observed.pop('bytes')
        self.observations[role] = observed
        diagnostic = self.tagged()
        snapshot = diagnostic['journal_final_observation']['snapshot']
        snapshot.update(status='written', path=str(target), bytes_written=len(payload),
            prefix_sha256=combined.digest(payload), sha256=combined.digest(payload),
            fingerprint=copy.deepcopy(observed['before']))
        self.add_json('result', diagnostic)
        self.assertEqual(self.edge(self.analyze(), 'journal_wholefile_claims'), 'matched')
        self.payloads[role] = payload + b'x'
        self.assert_wholefile_not_matched()
        self.payloads[role] = payload
        snapshot['fingerprint']['mode'] = 0o100644
        self.add_json('result', diagnostic)
        self.assert_wholefile_not_matched()
        snapshot['fingerprint'] = copy.deepcopy(observed['before'])
        self.add_json('result', diagnostic)
        self.observations.pop(role)
        self.assert_wholefile_not_matched()
        self.observations[role] = observed
        self.spec['binaries'].pop('final-snapshot')
        self.policy['expected_pins'].pop(role)
        self.payloads.pop(role)
        self.observations.pop(role)
        self.assert_wholefile_not_matched()


class SelectedSaveAssociationTests(unittest.TestCase):
    """Authored v2 synthetic consumer controls, UNRUN; no real restore/runtime proof."""

    setUpClass = classmethod(CombinedEvidenceTests.setUpClass.__func__)
    tearDownClass = classmethod(CombinedEvidenceTests.tearDownClass.__func__)
    add = CombinedEvidenceTests.add
    add_json = CombinedEvidenceTests.add_json
    native_command = CombinedEvidenceTests.native_command
    item = CombinedEvidenceTests.item
    state = CombinedEvidenceTests.state
    receipt_pin = CombinedEvidenceTests.receipt_pin
    journal_bytes = CombinedEvidenceTests.journal_bytes
    update_journal = CombinedEvidenceTests.update_journal
    analyze = CombinedEvidenceTests.analyze
    edge = CombinedEvidenceTests.edge
    assert_unqualified = CombinedEvidenceTests.assert_unqualified

    def setUp(self):
        CombinedEvidenceTests.setUp(self)
        self.selected_raw = combined.canonical({'session': {
            'source': 'UserData/Houses/House10.iff', 'options': {'routing': 'Strict'}},
            'floor': 1, 'zoom': 1.0, 'resident': 1})
        self.add('selected_save', self.selected_raw)
        self.add('selected_save_pinned', self.selected_raw)
        source, pinned = (self.spec['artifacts'][role] for role in ('selected_save', 'selected_save_pinned'))

        def state(item, inode):
            return {'sha256': item['sha256'], 'size': item['size'], 'mode': 0o400,
                    'device': 1, 'inode': inode, 'links': 1, 'mtime_ns': 1, 'ctime_ns': 1}

        self.selected_pin = {'kind': 'selected_save', 'source': source['path'], 'pinned': pinned['path'],
            'before': state(source, 11), 'after': state(source, 11),
            'pinned_before': state(pinned, 12), 'pinned_after': state(pinned, 12), 'unchanged': True}
        launch = copy.deepcopy(self.values['launch'])
        launch.update(input=source['path'], input_sha256=source['sha256'], source='UserData/Houses/House10.iff')
        launch['inputs'].append(copy.deepcopy(self.selected_pin))
        launch['command'].extend(['--original', launch['source'], '--load', pinned['path']])
        self.add_json('launch', launch)
        descriptor = {'bytes': len(self.selected_raw), 'regular': True, 'modified_unix_ns': '1',
                      'device': 1, 'inode': 12, 'mode': 0o100400, 'change_seconds': 0, 'change_nanoseconds': 1}
        self.origin = {'boundary': 'player_application_envelope', 'sha256': source['sha256'],
            'byte_count': len(self.selected_raw), 'requested_path': pinned['path'], 'path_identity_observed': False,
            'descriptor_before': descriptor, 'descriptor_after': copy.deepcopy(descriptor), 'descriptor_status': 'consistent'}
        slot = {'lifetime': 1, 'present': True, 'input': self.origin, 'install': 'installed'}
        self.loaded = {'extension': combined.LOADED_INPUT_CONTRACT, 'viewer_lifetime_ordinal': 1,
            'transition_ordinal': 1, 'live_lifetime': 1, 'accounted_slot': slot,
            'last_recorded_owner': copy.deepcopy(slot), 'invalidation_reasons': [], 'last_operation': 'owner_prepared',
            'closed': False, 'ownership_status': 'owner_prepared_delivery_pending', 'unresolved_in_flight': True,
            'current_owner_asserted': True, 'eligible': True, 'snapshot_only': True,
            'worker_finalized': False, 'quiescence_proven': False, 'capture_owner_matches_account': True,
            'owner_preparation_complete': True, 'owner_mutation_pending': False, 'publication_delivery_pending': True,
            'capture_boundary': 'owner_prepared_before_read_only_publication', 'ui_retirement_intent_lifetime': None,
            'request_channel_closed_after_ui_retirement': False, 'retired_lifetime': None}
        self.records[0]['data']['extensions'] = [combined.LOADED_INPUT_CONTRACT]
        for frame in self.frames():
            frame['state']['state_id'] = frame['frame_id']
            frame['state']['value']['loaded_input'] = copy.deepcopy(self.loaded)
            frame['state']['diagnostics']['loaded_input_work'] = {
                'work_ordinal': frame['frame_id'], 'active_work': [frame['frame_id']]}
        for record in self.records:
            event = record['data']
            if record['kind'] == 'presentation_event':
                if event['state_id']:
                    event['state_id'] = event['frame_id']
                if event['bound_state_id']:
                    event['bound_state_id'] = event['bound_frame_id']
        self.footer = copy.deepcopy(self.loaded)
        self.footer.pop('capture_owner_matches_account')
        self.footer.update(work_ordinal=3, active_work=[], last_operation='success', closed=True,
            ownership_status='accounted_at_snapshot', unresolved_in_flight=False, current_owner_asserted=False,
            eligible=False, publication_delivery_pending=False, capture_boundary='completed_owner_work')
        self.records[-1]['data']['loaded_input'] = self.footer
        self.update_journal()

    def frames(self):
        return [record['data'] for record in self.records if record['kind'] == 'frame']

    def associate(self, **overrides):
        arguments = {'selected_bytes': self.payloads['selected_save'],
            'selected_descriptor': self.spec['artifacts']['selected_save'],
            'pinned_bytes': self.payloads['selected_save_pinned'],
            'pinned_descriptor': self.spec['artifacts']['selected_save_pinned'],
            'launch_receipt': self.values['launch']}
        arguments.update(overrides)
        return combined.selected_save_association(self.records, self.controls, **arguments)

    def test_complete_synthetic_input_join_never_qualifies_execution(self):
        association = self.associate()
        self.assertEqual(association['frame_ids'], [1, 2, 3])
        self.assertEqual(association['sha256'], combined.digest(self.selected_raw))
        result = self.analyze()
        self.assertEqual(self.edge(result, 'selected_save_session_identity'), 'matched', result)
        self.assertTrue(result['contract_valid'], result)
        self.assertTrue(result['blockers'])
        self.assertFalse(any(item['code'] == 'PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING' for item in result['blockers']))
        self.assert_unqualified(result)

    def test_explicit_selected_roles_use_existing_descriptor_reader(self):
        result = combined.read_combined(self.spec, reader=combined.DescriptorReader(), policy=self.policy)
        for role in ('selected_save', 'selected_save_pinned'):
            self.assertEqual(result['artifact_descriptors'][role], self.spec['artifacts'][role])
            self.assertEqual(result['reader_observations'][role]['contract'],
                             'descriptor-nofollow-nonblocking-streaming-prepost-v1')
        self.assertEqual(self.edge(result, 'selected_save_session_identity'), 'matched', result)
        self.assert_unqualified(result)

    def test_raw_whitespace_is_not_parsed_json_identity(self):
        changed = self.selected_raw + b' \n'
        item = copy.deepcopy(self.spec['artifacts']['selected_save_pinned'])
        item.update(size=len(changed), sha256=combined.digest(changed))
        with self.assertRaisesRegex(ValueError, 'raw bytes differ'):
            self.associate(pinned_bytes=changed, pinned_descriptor=item)

    def test_size_hash_and_nonbytes_substitution_fail_before_receipt_join(self):
        for override in ({'selected_bytes': self.selected_raw + b' '},
                         {'pinned_bytes': bytearray(self.selected_raw)},
                         {'selected_descriptor': {**self.spec['artifacts']['selected_save'], 'sha256': '0' * 64}},
                         {'pinned_descriptor': {**self.spec['artifacts']['selected_save_pinned'], 'size': True}}):
            with self.subTest(override=tuple(override)), self.assertRaises(ValueError):
                self.associate(**override)

    def test_actual_non_strict_routing_and_source_are_checked(self):
        for routing in ('InferredRectangles', 'strict', None, True):
            value = combined.parse_json(self.selected_raw)
            value['session']['options']['routing'] = routing
            raw = combined.canonical(value)
            source = {**self.spec['artifacts']['selected_save'], 'size': len(raw), 'sha256': combined.digest(raw)}
            pinned = {**self.spec['artifacts']['selected_save_pinned'], 'size': len(raw), 'sha256': combined.digest(raw)}
            with self.subTest(routing=routing), self.assertRaisesRegex(ValueError, 'must be Strict'):
                self.associate(selected_bytes=raw, selected_descriptor=source, pinned_bytes=raw, pinned_descriptor=pinned)
        launch = copy.deepcopy(self.values['launch'])
        launch['source'] = 'UserData/Houses/House99.iff'
        with self.assertRaisesRegex(ValueError, 'input/source receipt'):
            self.associate(launch_receipt=launch)

    def test_duplicate_json_keys_cannot_supply_a_strict_policy(self):
        raw = b'{"session":{"source":"house","options":{"routing":"Inferred","routing":"Strict"}}}'
        source = {**self.spec['artifacts']['selected_save'], 'size': len(raw), 'sha256': combined.digest(raw)}
        pinned = {**self.spec['artifacts']['selected_save_pinned'], 'size': len(raw), 'sha256': combined.digest(raw)}
        with self.assertRaisesRegex(ValueError, 'Duplicate JSON key'):
            self.associate(selected_bytes=raw, selected_descriptor=source, pinned_bytes=raw, pinned_descriptor=pinned)

    def test_unique_cli_source_and_selected_pin_chain_are_required(self):
        for mutation in ('duplicate_load', 'wrong_original', 'output_as_load', 'duplicate_pin', 'changed_pin', 'linked_pin'):
            launch = copy.deepcopy(self.values['launch'])
            pin = launch['inputs'][-1]
            if mutation == 'duplicate_load':
                launch['command'].extend(['--load', pin['pinned']])
            elif mutation == 'wrong_original':
                launch['command'][launch['command'].index('--original') + 1] = 'other-house'
            elif mutation == 'output_as_load':
                launch['command'][launch['command'].index('--load') + 1] = '/saves/DO-NOT-READ'
            elif mutation == 'duplicate_pin':
                launch['inputs'].append(copy.deepcopy(pin))
            elif mutation == 'changed_pin':
                pin['pinned_after']['inode'] += 1
            else:
                pin['before']['links'] = pin['after']['links'] = 2
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.associate(launch_receipt=launch)

    def test_cli_separator_and_equals_duplicates_keep_selected_edge_blocked(self):
        original = copy.deepcopy(self.values['launch'])
        for mutation in ('separator', 'equals_load', 'equals_original'):
            launch = copy.deepcopy(original)
            if mutation == 'separator':
                launch['command'].insert(1, '--')
            else:
                launch['command'].append('--load=/other.json' if mutation == 'equals_load'
                                         else '--original=other-house')
            self.add_json('launch', launch)
            result = self.analyze()
            with self.subTest(mutation=mutation):
                self.assertNotEqual(self.edge(result, 'selected_save_session_identity'), 'matched')
                self.assertIn('PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING',
                              {item['code'] for item in result['blockers']})
                self.assert_unqualified(result)

    def test_normalized_same_open_object_is_not_a_hash_only_check(self):
        for field, replacement in (('inode', 99), ('device', 99), ('mode', 0o100600),
                                   ('modified_unix_ns', '2'), ('change_seconds', 1), ('regular', False)):
            records = copy.deepcopy(self.records)
            for record in records:
                if record['kind'] == 'frame':
                    origin = record['data']['state']['value']['loaded_input']['accounted_slot']['input']
                    origin['descriptor_before'][field] = origin['descriptor_after'][field] = replacement
                    record['data']['state']['value']['loaded_input']['last_recorded_owner'] = copy.deepcopy(
                        record['data']['state']['value']['loaded_input']['accounted_slot'])
            previous, self.records = self.records, records
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.associate()
            self.records = previous

    def test_old_or_missing_extension_retains_unconditional_producer_blocker(self):
        for extensions in (None, [], ['sims-window-loaded-input-observation-v1']):
            self.records[0]['data']['extensions'] = extensions
            self.update_journal()
            result = self.analyze()
            self.assertEqual(self.edge(result, 'selected_save_session_identity'), 'unverifiable')
            self.assertIn('PRODUCER_JOURNAL_SAVE_IDENTITY_MISSING', {item['code'] for item in result['blockers']})
            self.assert_unqualified(result)

    def test_duplicate_mixed_or_untyped_extension_declarations_fail_closed(self):
        for extensions in ([combined.LOADED_INPUT_CONTRACT] * 2,
                           [combined.LOADED_INPUT_CONTRACT, 'other', 'other'],
                           [combined.LOADED_INPUT_CONTRACT, True],
                           [combined.LOADED_INPUT_CONTRACT, 'sims-window-loaded-input-observation-v1']):
            self.records[0]['data']['extensions'] = extensions
            with self.subTest(extensions=extensions), self.assertRaisesRegex(ValueError, 'v2 extension required'):
                self.associate()

    def test_missing_raw_role_cannot_be_reconstructed_from_receipt_paths(self):
        self.spec['artifacts'].pop('selected_save')
        self.policy['expected_pins'].pop('selected_save')
        self.payloads.pop('selected_save')
        result = self.analyze()
        self.assertEqual(self.edge(result, 'selected_save_session_identity'), 'missing')
        self.assertFalse(result['contract_valid'])
        self.assert_unqualified(result)

    def test_required_value_and_same_capture_diagnostic_fields_fail_closed(self):
        for name in ('owner_preparation_complete', 'capture_owner_matches_account', 'retired_lifetime'):
            frame = self.frames()[0]
            original = frame['state']['value']['loaded_input'].pop(name)
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'schema differ'):
                self.associate()
            frame['state']['value']['loaded_input'][name] = original
        frame = self.frames()[0]
        frame['state']['diagnostics']['loaded_input_work']['active_work'] = [99]
        with self.assertRaisesRegex(ValueError, 'work history'):
            self.associate()

    def test_applying_preparation_is_not_read_only_publication_pending(self):
        frame = self.frames()[0]
        frame['state']['value']['loaded_input'].update(owner_preparation_complete=False,
            owner_mutation_pending=True, publication_delivery_pending=False, eligible=False,
            current_owner_asserted=False, ownership_status='unresolved_in_flight',
            capture_boundary='owner_preparation_incomplete', last_operation='in_flight')
        with self.assertRaisesRegex(ValueError, 'owner mutation'):
            self.associate()

    def test_completed_empty_scope_captures_are_also_historical_input_associations(self):
        for frame in self.frames():
            frame['state']['value']['loaded_input'].update(last_operation='success',
                ownership_status='accounted_at_snapshot', unresolved_in_flight=False,
                publication_delivery_pending=False, capture_boundary='completed_owner_work')
            frame['state']['diagnostics']['loaded_input_work']['active_work'] = []
        self.assertEqual(self.associate()['frame_ids'], [1, 2, 3])

    def test_v2_header_does_not_upgrade_v1_or_mismatched_actual_owner_fields(self):
        frame = self.frames()[0]['state']['value']['loaded_input']
        frame['extension'] = 'sims-window-loaded-input-observation-v1'
        with self.assertRaisesRegex(ValueError, 'Unknown loaded input extension'):
            self.associate()
        frame['extension'] = combined.LOADED_INPUT_CONTRACT
        frame['capture_owner_matches_account'] = False
        with self.assertRaisesRegex(ValueError, 'actual owner differs'):
            self.associate()

    def test_terminal_open_failure_and_quiescence_allegations_fail_closed(self):
        for mutation in ({'active_work': [3], 'unresolved_in_flight': True},
                         {'invalidation_reasons': ['result_delivery_failed'], 'last_operation': 'failed'},
                         {'worker_finalized': True}, {'quiescence_proven': True}, {'current_owner_asserted': True}):
            original = copy.deepcopy(self.footer)
            self.footer.update(mutation)
            with self.subTest(mutation=tuple(mutation)), self.assertRaises(ValueError):
                self.associate()
            self.footer.clear(); self.footer.update(original)

    def test_away_and_back_without_draws_and_new_lifetime_cannot_use_stale_frames(self):
        self.footer['transition_ordinal'] = 3
        with self.assertRaisesRegex(ValueError, 'stale publication'):
            self.associate()
        self.footer['transition_ordinal'] = 1
        self.footer['viewer_lifetime_ordinal'] = self.footer['live_lifetime'] = 2
        self.footer['accounted_slot']['lifetime'] = self.footer['last_recorded_owner']['lifetime'] = 2
        with self.assertRaisesRegex(ValueError, 'stale publication'):
            self.associate()

    def test_unreferenced_intermediate_frame_is_checked(self):
        closing = self.frames()[-1]
        closing['frame_id'] = closing['state']['state_id'] = 4
        closing['state']['diagnostics']['loaded_input_work'] = {'work_ordinal': 4, 'active_work': [4]}
        self.controls[0]['control_effect']['closing_frame_id'] = 4
        extra = copy.deepcopy(next(record for record in self.records if record['kind'] == 'frame'))
        extra['elapsed_us'] = 1_101_562
        extra['data'].update(frame_id=3, input_sequence=2, draw_started_unix_ns=unix(13 / 128))
        extra['data']['state']['state_id'] = 3
        extra['data']['state']['value']['loaded_input']['transition_ordinal'] = 2
        extra['data']['state']['diagnostics']['loaded_input_work'] = {'work_ordinal': 3, 'active_work': [3]}
        position = next(index for index, record in enumerate(self.records)
                        if record['kind'] == 'frame' and record['data']['frame_id'] == 4)
        self.records.insert(position, extra)
        self.footer['work_ordinal'] = 4
        with self.assertRaisesRegex(ValueError, 'interval or tail'):
            self.associate()

    def test_unreferenced_tail_is_checked(self):
        extra = copy.deepcopy(next(record for record in self.records if record['kind'] == 'frame'))
        extra['elapsed_us'] = self.records[-1]['elapsed_us'] - 1
        extra['data'].update(frame_id=4, input_sequence=2, draw_started_unix_ns=unix(1 / 8))
        extra['data']['state']['state_id'] = 4
        extra['data']['state']['value']['loaded_input']['transition_ordinal'] = 2
        self.records.insert(-1, extra)
        with self.assertRaisesRegex(ValueError, 'interval or tail'):
            self.associate()
        extra['data']['state']['value']['loaded_input']['transition_ordinal'] = 1
        extra['data']['state']['diagnostics']['loaded_input_work'] = {'work_ordinal': 4, 'active_work': [4]}
        self.footer['work_ordinal'] = 4
        self.assertEqual(self.associate()['frame_ids'], [1, 2, 3, 4])

    def test_live_success_and_precise_registration_retirement_remain_snapshot_only(self):
        self.assertEqual(self.associate()['transition_ordinal'], 1)
        self.footer.update(live_lifetime=None, retired_lifetime=1, ui_retirement_intent_lifetime=1,
            request_channel_closed_after_ui_retirement=True, last_operation='viewer_registration_closed',
            ownership_status='no_live_registration', owner_preparation_complete=False,
            capture_boundary='owner_preparation_incomplete')
        self.assertEqual(self.associate()['transition_ordinal'], 1)
        self.footer['request_channel_closed_after_ui_retirement'] = False
        with self.assertRaisesRegex(ValueError, 'retirement lacks exact source facts'):
            self.associate()

    def test_save_association_does_not_clear_other_identity_or_outcome_gates(self):
        self.records[0]['data'].pop('build_ids')
        self.update_journal()
        result = self.analyze()
        self.assertEqual(self.edge(result, 'selected_save_session_identity'), 'matched')
        self.assertEqual(self.edge(result, 'application_identity'), 'missing')
        self.assertFalse(result['contract_valid'])
        self.assertIn('PRODUCER_JOURNAL_LIFETIME_BUILD_IDS_MISSING', {item['code'] for item in result['blockers']})
        self.assert_unqualified(result)


if __name__ == '__main__':
    read_source(__file__)
    test_result = unittest.main(verbosity=2, exit=False).result
    for source_path in tuple(SOURCE_READ_PINS):
        read_source(source_path)
    print(json.dumps({'tests_run': test_result.testsRun, 'failures': len(test_result.failures),
                      'errors': len(test_result.errors), 'skipped': len(test_result.skipped),
                      'successful': test_result.wasSuccessful(), 'executed_source_sha256': EXECUTED_SOURCE_PINS,
                      'source_reads': SOURCE_READ_PINS, 'source_loading': 'Exact captured Python bytes; no local pyc',
                      'guard_scope': 'Instrumented ordinary file APIs plus deny audit; not adversarial Python isolation',
                      'native_qualification': False, 'alpha_qualification': False}, sort_keys=True))
    raise SystemExit(not test_result.wasSuccessful())
