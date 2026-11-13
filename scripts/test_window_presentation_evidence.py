"""Synthetic source-schema tests only. Run: python3 -I -B <this file>.

The audit guard is installed before the analyzer import. It denies process,
network, dynamic-library, native-framework and signal operations, including
write-mode opens. Only the Python stdlib and the new pure analyzer are imported.
No root analyzer/tests, driver, compiler, asset, save or native API is executed.
"""

import sys


FORBIDDEN_MODULES = {
    'subprocess', '_posixsubprocess', 'socket', '_socket', 'ctypes', '_ctypes',
    'multiprocessing', 'objc', 'AppKit', 'ApplicationServices', 'CoreGraphics',
    'Darwin', 'Foundation', 'Quartz', 'QuartzCore', 'Metal', 'ScreenCaptureKit',
    'CoreFoundation', 'Cocoa', 'PyObjCTools', 'CoreVideo', 'CoreMedia', 'AVFoundation',
    'MetalKit', 'ScriptingBridge', 'IOKit', 'cffi', '_cffi_backend',
    'window_presentation', 'test_window_presentation', 'window_session', 'window_evidence',
}
FORBIDDEN_EVENTS = {
    'os.system', 'os.fork', 'os.forkpty', 'os.exec', 'os.posix_spawn',
    'os.spawn', 'os.kill', 'os.killpg', 'os.startfile', 'pty.spawn',
}


def audit_guard(event, arguments):
    if event == 'import' and arguments[0].split('.')[0] in FORBIDDEN_MODULES:
        raise RuntimeError('Audit guard: forbidden import ' + arguments[0])
    if (event in FORBIDDEN_EVENTS or event.startswith(('subprocess.', 'socket.', 'ctypes.', 'signal.'))
            or event.startswith(('os.exec', 'os.spawn'))):
        raise RuntimeError('Audit guard: forbidden operation ' + event)
    if event == 'open':
        mode, flags = arguments[1:3]
        if (isinstance(mode, str) and any(character in mode for character in 'wax+')) or flags & (1 | 2 | 64 | 512 | 1024):
            raise RuntimeError('Audit guard: write-mode open')


sys.addaudithook(audit_guard)

import ast
import copy
import json
import math
from pathlib import Path
import unittest


def deny_signal(*arguments, **keywords):
    raise RuntimeError('Audit guard: signal operation')


for signal_module in ('signal', '_signal'):
    for signal_name in ('signal', 'alarm', 'setitimer', 'raise_signal', 'pthread_kill',
                        'pthread_sigmask', 'sigwait', 'sigwaitinfo', 'sigtimedwait',
                        'set_wakeup_fd', 'pause'):
        if hasattr(sys.modules.get(signal_module), signal_name):
            setattr(sys.modules[signal_module], signal_name, deny_signal)

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

import window_presentation_evidence as evidence


UNIX = 1_000_000_000_000
HOST = 8
WIDTH = 2 ** -20


def unix_time(delta):
    return UNIX + int(delta * 1_000_000_000)


def state(center):
    return {'tick': 7, 'playing': False, 'pending': False, 'clock_pending': False,
            'presentation_children_pending': False, 'modal': False, 'camera': {'center': [center, 0]},
            'display_size': [800, 600], 'terminals': None, 'hud_normal_ready': None,
            'hud_transport_ready': None, 'observation_capabilities': {
                'action_terminals': False, 'route_terminals': False, 'transport_readiness': False},
            'error': None, 'menu_error': None, 'shell': None}


def owned_binding():
    return {'schema': 'sims-window-binding-v2', 'pid': 42, 'window_id': 91,
            'executable': '/synthetic/never-executed', 'kernel_identity': {
                'source': 'proc_pidinfo(PROC_PIDTBSDINFO)+proc_pidpath', 'pid': 42,
                'start_tvsec': 123, 'start_tvusec': 456, 'executable': '/synthetic/never-executed'},
            'frontmost': True, 'on_screen': True, 'bounds': [0, 0, 800, 600],
            'display_scope': 'Current CoreGraphics mode and NSScreen maximum; not instantaneous refresh',
            'displays': [{'display_id': 1, 'mode_id': 2, 'width': 800, 'height': 600,
                          'pixel_width': 1600, 'pixel_height': 1200, 'nominal_refresh_hz': 60,
                          'maximum_frames_per_second': 60, 'builtin': True, 'active': True, 'asleep': False}]}


def fixture(control_count=1):
    """Independently authored full journal/control schema, not a root test copy."""
    scheduled, controls, frame_data = [], [], []

    def record(kind, delta, data):
        value = {'kind': kind, 'elapsed_us': int(delta * 1_000_000), 'dropped_records': 0, 'data': data}
        scheduled.append((delta, len(scheduled), value))
        return value

    def native(kind, delta, flags=0, **values):
        event = {name: 0 for name in evidence.EVENT_INTEGERS + evidence.EVENT_REALS}
        event.update(abi_version=1, struct_size=192, kind=kind, flags=flags, status=0,
                     host_time=HOST + delta)
        event.update(values)
        record('presentation_event', delta, event)

    def frame(delta, sequence, center):
        identifier = len(frame_data) + 1
        data = {'frame_id': identifier, 'fence': 1, 'input_sequence': sequence,
                'draw_started_unix_ns': unix_time(delta), 'cursor': None, 'widget_draw_cpu_us': 50,
                'draw_interval_us': None, 'state': {'state_id': center + 1, 'value': state(center),
                                                 'diagnostics': {'clock': None, 'worker_busy': False}}}
        frame_data.append(data)
        identity = {'draw_context_sequence': identifier, 'frame_id': identifier, 'state_id': center + 1}
        native(2, delta, 4, **identity)
        record('frame', delta, data)
        identity.update(acquisition_sequence=identifier, layer_lifetime_id=1,
                        acquisition_enter_host_time=HOST + delta + 1 / 8192)
        native(3, delta + 1 / 8192, 4 | 4096, **identity)
        identity.update(acquisition_call_host_time=HOST + delta + 2 / 8192,
                        acquisition_return_host_time=HOST + delta + 3 / 8192,
                        drawable_id=identifier - 1, texture_identity=100 + identifier,
                        drawable_width=1600, drawable_height=1200,
                        display_sync_enabled=1, maximum_drawable_count=3)
        native(4, delta + 3 / 8192, 4 | 8 | 16 | 64 | 4096, **identity)
        native(5, delta + 4 / 8192, 4 | 8 | 16 | 64 | 4096, **identity)
        identity.update(bind_count=1, bound_frame_id=identifier, bound_state_id=center + 1)
        native(7, delta + 5 / 8192, 4 | 8 | 16 | 32, **identity)
        native(6, delta + 3 / 1024, 4 | 8 | 16 | 32, **identity,
               presented_host_time=HOST + delta + 2 / 1024,
               callback_delivery_host_time=HOST + delta + 3 / 1024)
        return data

    def sleep(phase, delta):
        absolute = int((HOST + delta) * 1_000_000_000)
        record('presentation_sleep_clock', delta, {
            'schema': 1, 'phase': phase,
            'clock': 'mach timebase ticks; absolute stops during sleep; continuous includes sleep',
            'clock_api': evidence.CLOCK_API, 'quartz_before_seconds': absolute / 1_000_000_000 - 1e-9,
            'quartz_after_seconds': (absolute + 2) / 1_000_000_000 + 1e-9,
            'sample': {'absolute_before': absolute, 'continuous': absolute + 1_000_000_001,
                       'absolute_after': absolute + 2, 'timebase_numer': 1, 'timebase_denom': 1}})

    record('session', -1, {'schema': 1, 'pid': 42, 'unix_ns': str(UNIX - 1_000_000_000),
                         'gpu_time_available': False, 'present_time_available': False})
    sleep('begin', -1 / 8)
    record('presentation_library', -1 / 16, {
        'source_contract': evidence.SOURCE_CONTRACT, 'source_only': True, 'abi_version': 1,
        'event_size': 192, 'counters_size': 304, 'summary_size': 408,
        'source_sha256': evidence.SOURCE_PINS['root/crates/sims-app/native/window_present_macos.m']})
    record('presentation_begin', 1 / 512, {'abi_version': 1, 'status': 0,
        'clock': evidence.CLOCK_DESCRIPTION, 'clock_api': evidence.CLOCK_API, 'boundary': evidence.BOUNDARY})
    native(1, 0)
    native(9, 1 / 1024, platform_notification=6)
    for index in range(control_count):
        base = index
        baseline = frame(base + 1 / 32, index * 2, index)
        press, release = base + 1 / 16, base + 3 / 32
        receipts = []
        for ordinal, (delta, kind) in enumerate(((press, 'key_pressed'), (release, 'key_released')), 1):
            data = {'fence': 1, 'input_sequence': index * 2 + ordinal,
                    'received_unix_ns': unix_time(delta + 1 / 1024),
                    'input': {'kind': kind, 'physical_key': 'Code(ArrowRight)'}}
            receipts.append(data)
            record('input', delta + 1 / 1024, data)
        effect_frame = frame(base + 5 / 64, index * 2 + 1, index + 1)
        closing = frame(base + 7 / 64, index * 2 + 2, index + 1)
        latency = (effect_frame['draw_started_unix_ns'] - receipts[0]['received_unix_ns']) / 1000
        control = {'complete': True, 'origin': evidence.CONTROL_ORIGIN,
                   'source_origin': 'window_controls.summarize_controls',
                   'latency_us': {'samples': 1, 'p50': latency, 'p95': latency, 'p99': latency, 'worst': latency},
                   'p95_limit_us': 100_000, 'maximum_limit_us': 250_000, 'within_fixed_limits': True,
                   'baseline_frame_id': baseline['frame_id'], 'receipts': copy.deepcopy(receipts),
                   'effect': {'frame_id': effect_frame['frame_id'], 'state_id': index + 2,
                              'unix_ns': effect_frame['draw_started_unix_ns'], 'latency_us': latency},
                   'closing_frame_id': closing['frame_id'],
                   'scope': 'Fresh key-press receipt to first acknowledged exact-effect draw before next command; '
                            'not native-post, action-completion, displayed-frame or input-to-photon latency; '
                            'gesture duration is not subtracted'}
        posts = [{'down': down, 'unix_before_ns': unix_time(delta),
                  'unix_after_ns': unix_time(delta + WIDTH),
                  'host_before_ns': 900_000_000_000 + int(delta * 1_000_000_000),
                  'host_after_ns': 900_000_000_000 + int((delta + WIDTH) * 1_000_000_000),
                  'clock_api': evidence.CLOCK_API, 'quartz_before_seconds': HOST + delta,
                  'quartz_after_seconds': HOST + delta + WIDTH}
                 for down, delta in ((True, press), (False, release))]
        native_result = {'operation': 'key', 'binding': owned_binding(), 'code': 124, 'press_ms': 20,
                         'released': True, 'posts': posts}
        controls.append({'control_effect': control, 'native_result': native_result,
                         'binding_before': owned_binding(), 'binding_after': owned_binding(),
                         'fence': 1, 'started_unix_ns': unix_time(press - 1 / 1024),
                         'finished_unix_ns': unix_time(release + 1 / 2048),
                         'next_started_unix_ns': None})
    for index in range(len(controls) - 1):
        controls[index]['next_started_unix_ns'] = controls[index + 1]['started_unix_ns']
        controls[index]['control_effect']['closing_frame_id'] = (index + 1) * 3 + 1
    cutoff = control_count - 1 + 9 / 64
    native(11, cutoff, 1)
    native(13, cutoff + 2 / 1024, 1)
    sleep('end', control_count - 1 + 1 / 4)
    summary = {'abi_version': 1, 'struct_size': 408, 'status': 0, 'session_state': 4,
               'capabilities': 3, 'flags': 12, 'begin_host_time': HOST,
               'cutoff_host_time': HOST + cutoff, 'drain_end_host_time': HOST + cutoff + 2 / 1024,
               'snapshot_host_time': HOST + cutoff + 3 / 1024,
               'last_event_sequence': 0, 'last_acquisition_sequence': len(frame_data),
               'unresolved_acquisitions': 0, 'unbound_acquisitions': 0,
               'unresolved_unbound_acquisitions': 0, 'counters': {name: 0 for name in evidence.COUNTERS}}
    record('presentation_end', control_count - 1 + 9 / 32, {'status': 0, 'summary': summary})
    record('session_end', control_count - 1 + 5 / 16, {'dropped_records': 0})
    scheduled.sort(key=lambda entry: entry[:2])
    records = [entry[2] for entry in scheduled]
    origin_elapsed = records[0]['elapsed_us']
    for value in records:
        value['elapsed_us'] -= origin_elapsed
    renumber(records)
    return records, controls


def events(records, kind=None, acquisition=None):
    return [record['data'] for record in records if record['kind'] == 'presentation_event'
            and (kind is None or record['data']['kind'] == kind)
            and (acquisition is None or record['data']['acquisition_sequence'] == acquisition)]


def data(records, kind):
    return next(record['data'] for record in records if record['kind'] == kind)


def renumber(records):
    native = events(records)
    for index, event in enumerate(native, 1):
        event['event_sequence'] = index
    summary = data(records, 'presentation_end')['summary']
    summary['last_event_sequence'] = len(native)
    counters = {name: 0 for name in evidence.COUNTERS}
    for name, kind in evidence.COUNTED.items():
        counters[name] = sum(event['kind'] == kind for event in native)
    count = counters['acquisition_entries']
    counters.update(emitted_events=len(native), successful_acquisitions=count,
                    handler_registration_attempts=count, successful_binds=count,
                    positive_presentations=count)
    summary['counters'] = counters


class PresentationEvidenceTests(unittest.TestCase):
    def invalid(self, records, controls, fragment=None):
        result = evidence.analyze(records, controls)
        self.assertFalse(result['valid'], result)
        self.assertTrue(result['errors'])
        self.assertEqual(result['samples'], [])
        self.assertFalse(result['api_presentation_latency']['within_fixed_limits'])
        self.assert_no_qualification(result)
        if fragment:
            self.assertIn(fragment, result['errors'][0])

    def assert_no_qualification(self, result):
        self.assertTrue(result['diagnostic_only'])
        for name, value in result.items():
            if name.endswith(('_qualified', '_proof', '_proven', '_reviewed')) or name == 'driver_enabled':
                self.assertIs(value, False, name)
        for name in ('eligible_seconds', 'civil_days', 'visibility', 'input_to_photon', 'full_readiness'):
            self.assertIsNone(result[name], name)

    def test_full_source_schema_positive_through_actual_analyzer(self):
        records, controls = fixture()
        parsed = evidence.parse_json(json.dumps({'records': records, 'controls': controls}))
        result = evidence.analyze(parsed['records'], parsed['controls'])
        self.assertTrue(result['valid'], result['errors'])
        self.assertEqual(result['samples'][0]['frame_id'], 2)
        self.assertEqual(result['samples'][0]['state_id'], 2)
        self.assertEqual(result['samples'][0]['lower_us'], 17577)
        self.assertEqual(result['samples'][0]['upper_us'], 17579)
        self.assertTrue(result['api_presentation_latency']['within_fixed_limits'])
        self.assertEqual(result['receipt_to_semantic_draw_us'], [14648.438])
        self.assert_no_qualification(result)

    def test_multiple_ordered_controls_and_distribution(self):
        records, controls = fixture(20)
        result = evidence.analyze(records, controls)
        self.assertTrue(result['valid'], result['errors'])
        self.assertEqual(len(result['samples']), 20)
        self.assertEqual(result['api_presentation_latency']['upper_bound_us']['p95'], 17579)

    def test_exact_fixed_thresholds_and_nearest_rank(self):
        for values, expected in (([100000], True), ([100001], False),
                                 ([100000] * 19 + [250000], True),
                                 ([100000] * 19 + [250001], False),
                                 ([100000] * 18 + [100001, 250000], False), ([], False)):
            with self.subTest(values=values):
                self.assertIs(evidence.fixed_limits(values)['within_fixed_limits'], expected)
        self.assertEqual(evidence.P95_LIMIT_US, 100000)
        self.assertEqual(evidence.MAXIMUM_LIMIT_US, 250000)

    def test_exact_rational_bounds_round_outward_without_epsilon(self):
        self.assertEqual(evidence.latency_bounds(8.25, 8, 8.125),
                         {'lower_us': 125000, 'upper_us': 250000, 'post_bracket_us': 125000})
        self.assertEqual(evidence.latency_bounds(8 + 2 ** -20, 8, 8)['upper_us'], 1)
        bounds = evidence.latency_bounds(8.25 + 2 ** -20, 8, 8.125)
        self.assertEqual(bounds['upper_us'], 250001)
        self.assertFalse(evidence.fixed_limits([100000] * 19 + [bounds['upper_us']])['within_fixed_limits'])

    def test_conservative_post_uncertainty_not_midpoint(self):
        records, controls = fixture()
        callback = events(records, 6, 2)[0]
        callback['presented_host_time'] = HOST + 1 / 16 + .1 + WIDTH / 2
        callback['callback_delivery_host_time'] = callback['host_time'] = HOST + .17
        target = next(record for record in records if record['kind'] == 'presentation_event'
                      and record['data'] is callback)
        records.remove(target)
        cutoff_index = next(index for index, record in enumerate(records)
                            if record['kind'] == 'presentation_event' and record['data']['kind'] == 11)
        records.insert(cutoff_index + 1, target)
        callback['flags'] |= 1
        target['elapsed_us'] = records[cutoff_index]['elapsed_us'] + 1
        summary = data(records, 'presentation_end')['summary']
        summary['cutoff_host_time'] = HOST + .18
        events(records, 11)[0]['host_time'] = HOST + .18
        callback['flags'] &= ~1
        records.remove(target)
        records.insert(cutoff_index, target)
        summary['drain_end_host_time'] = HOST + .19
        summary['snapshot_host_time'] = HOST + .20
        events(records, 13)[0]['host_time'] = HOST + .19
        target['elapsed_us'] = records[cutoff_index - 1]['elapsed_us']
        renumber(records)
        result = evidence.analyze(records, controls)
        self.assertTrue(result['valid'], result['errors'])
        self.assertLessEqual(result['samples'][0]['lower_us'], 100000)
        self.assertGreater(result['samples'][0]['upper_us'], 100000)
        self.assertFalse(result['api_presentation_latency']['within_fixed_limits'])

    def test_no_gesture_subtraction_or_unix_dispatch_domain_conversion(self):
        records, controls = fixture()
        expected = evidence.analyze(records, controls)
        for post in controls[0]['native_result']['posts']:
            post['host_before_ns'] += 50_000_000_000
            post['host_after_ns'] += 50_000_000_000
        actual = evidence.analyze(records, controls)
        self.assertEqual(actual['samples'], expected['samples'])
        self.assertGreater(actual['samples'][0]['lower_us'], 0)

    def test_callback_delivery_never_substitutes_for_presented_time(self):
        records, controls = fixture()
        first = evidence.analyze(records, controls)
        events(records, 6, 2)[0]['presented_host_time'] += 1 / 8192
        second = evidence.analyze(records, controls)
        self.assertTrue(second['valid'], second['errors'])
        self.assertGreater(second['samples'][0]['upper_us'], first['samples'][0]['upper_us'])
        events(records, 6, 2)[0]['presented_host_time'] = 0
        self.invalid(records, controls)

    def test_all_abi_integer_fields_reject_boolean_and_negative(self):
        for name in evidence.EVENT_INTEGERS:
            for value in (True, -1):
                with self.subTest(name=name, value=value):
                    records, controls = fixture()
                    events(records, 6, 2)[0][name] = value
                    self.invalid(records, controls)

    def test_all_abi_real_fields_reject_nonfinite_negative_and_boolean(self):
        for name in evidence.EVENT_REALS:
            for value in (math.inf, -math.inf, math.nan, -1, True, '1'):
                with self.subTest(name=name, value=value):
                    records, controls = fixture()
                    events(records, 6, 2)[0][name] = value
                    self.invalid(records, controls)

    def test_every_counter_is_typed_and_exactly_reconciled(self):
        for name in evidence.COUNTERS:
            for value in (True, -1, 100):
                with self.subTest(name=name, value=value):
                    records, controls = fixture()
                    data(records, 'presentation_end')['summary']['counters'][name] = value
                    self.invalid(records, controls)

    def test_missing_extra_event_and_accounting_controls(self):
        for kind in (1, 2, 3, 4, 5, 6, 7, 9, 11, 13):
            with self.subTest(kind=kind):
                records, controls = fixture()
                target = next(record for record in records if record['kind'] == 'presentation_event'
                              and record['data']['kind'] == kind)
                records.remove(target)
                renumber(records)
                self.invalid(records, controls)
        for name in ('last_event_sequence', 'last_acquisition_sequence', 'unresolved_acquisitions',
                     'unbound_acquisitions', 'unresolved_unbound_acquisitions', 'session_state', 'status',
                     'abi_version', 'struct_size', 'flags', 'capabilities'):
            with self.subTest(name=name):
                records, controls = fixture()
                data(records, 'presentation_end')['summary'][name] = 999
                self.invalid(records, controls)

    def test_every_fault_or_incoherent_event_flag_rejected(self):
        for bit in range(18):
            with self.subTest(bit=bit):
                records, controls = fixture()
                events(records, 6, 2)[0]['flags'] ^= 1 << bit
                self.invalid(records, controls)
        for kind in (10, 12, 14):
            records, controls = fixture()
            events(records, 6, 2)[0]['kind'] = kind
            self.invalid(records, controls)

    def test_binding_identity_and_per_acquisition_lifetime_fields(self):
        for name in evidence.RETURN + evidence.BIND:
            with self.subTest(name=name):
                records, controls = fixture()
                events(records, 6, 2)[0][name] += 1
                self.invalid(records, controls)

    def test_identical_drawable_ids_reused_lifetimes_do_not_resolve_stale_binding(self):
        records, controls = fixture()
        for event in events(records, acquisition=2):
            if event['kind'] != 3:
                event['drawable_id'] = 0
                event['texture_identity'] = 101
        self.invalid(records, controls, 'Reused identifier')
        records, controls = fixture()
        events(records, 6, 2)[0]['acquisition_sequence'] = 1
        self.invalid(records, controls)

    def test_completed_identifier_recycling_has_distinct_acquisition_identity(self):
        records, controls = fixture()
        old = copy.deepcopy(events(records, 6, 1)[0])
        returned = events(records, 4, 2)[0]
        old.update(kind=8, flags=old['flags'] | 1024, host_time=returned['host_time'] + WIDTH)
        insertion = next(index for index, record in enumerate(records) if record['data'] is returned)
        records.insert(insertion, {'kind': 'presentation_event', 'elapsed_us': records[insertion]['elapsed_us'],
                                   'dropped_records': 0, 'data': old})
        for event in events(records, acquisition=2):
            if event['kind'] != 3:
                event.update(drawable_id=0, texture_identity=101)
        renumber(records)
        result = evidence.analyze(records, controls)
        self.assertTrue(result['valid'], result['errors'])
        self.assertEqual(result['samples'][0]['acquisition_sequence'], 2)
        self.assertEqual(result['samples'][0]['drawable_id'], 0)
        events(records, 6, 2)[0]['bound_frame_id'] = 1
        self.invalid(records, controls)

    def test_native_begin_callbacks_precede_successful_bridge_return(self):
        records, controls = fixture()
        begin = next(index for index, record in enumerate(records) if record['kind'] == 'presentation_begin')
        initial = [index for index, record in enumerate(records)
                   if record['kind'] == 'presentation_event' and record['data']['kind'] in (1, 9)]
        self.assertTrue(all(index < begin for index in initial))
        self.assertTrue(evidence.analyze(records, controls)['valid'])
        returned = records.pop(begin)
        records.insert(initial[0], returned)
        self.invalid(records, controls, 'begin/end')

    def test_supersession_before_replacement_entry_is_not_native_recycling(self):
        records, controls = fixture()
        old = copy.deepcopy(events(records, 6, 1)[0])
        old.update(kind=8, flags=old['flags'] | 1024, host_time=HOST + 3 / 64)
        insertion = next(index for index, record in enumerate(records) if record['kind'] == 'input')
        records.insert(insertion, {'kind': 'presentation_event', 'elapsed_us': records[insertion - 1]['elapsed_us'],
                                   'dropped_records': 0, 'data': old})
        for event in events(records, acquisition=2):
            if event['kind'] != 3:
                event.update(drawable_id=0, texture_identity=101)
        renumber(records)
        self.invalid(records, controls, 'supersession')

    def test_span_end_identity_rejects_boolean_and_float_aliases(self):
        for identity in (True, 1.0):
            with self.subTest(identity=identity):
                records, controls = fixture()
                elapsed = records[-1]['elapsed_us']
                records[-1:-1] = [
                    {'kind': 'span_begin', 'elapsed_us': elapsed - 2, 'dropped_records': 0,
                     'data': {'span_id': 1, 'name': 'authored'}},
                    {'kind': 'span_end', 'elapsed_us': elapsed - 1, 'dropped_records': 0,
                     'data': {'span_id': identity, 'name': 'authored', 'wall_us': 1}},
                ]
                self.invalid(records, controls, 'span')

    def test_independent_span_enqueue_preserves_raw_partial_order(self):
        records, controls = fixture()
        original_records = copy.deepcopy(records)
        start = next(index for index, record in enumerate(records) if record['kind'] == 'presentation_begin')
        records.insert(start, {'kind': 'span_begin', 'elapsed_us': records[start]['elapsed_us'] + 2,
                              'dropped_records': 0, 'data': {'span_id': 7, 'name': 'authored'}})
        records[-1:-1] = [{'kind': 'span_end', 'elapsed_us': records[-1]['elapsed_us'] - 1,
                          'dropped_records': 0, 'data': {'span_id': 7, 'name': 'authored', 'wall_us': 1}}]
        result = evidence.analyze(records, controls)
        self.assertTrue(result['valid'], result['errors'])
        self.assertEqual([record for record in records if record['kind'] not in ('span_begin', 'span_end')],
                         original_records)
        records[-2]['elapsed_us'] = records[start]['elapsed_us'] - 1
        self.invalid(records, controls, 'span')

    def test_clock_regression_discontinuity_sleep_and_incompatible_tags(self):
        mutations = (
            lambda records, controls: data(records, 'presentation_begin').update(clock_api='DispatchTime'),
            lambda records, controls: data(records, 'presentation_begin').update(boundary='callback delivery'),
            lambda records, controls: controls[0]['native_result']['posts'][0].update(clock_api='mach_absolute_time'),
            lambda records, controls: controls[0]['native_result']['posts'][0].update(quartz_after_seconds=7),
            lambda records, controls: events(records, 4, 2)[0].update(host_time=HOST),
            lambda records, controls: [record['data']['sample'].update(continuous=99_000_000_000)
                                      for record in records if record['kind'] == 'presentation_sleep_clock'
                                      and record['data']['phase'] == 'end'],
            lambda records, controls: [record['data'].update(quartz_before_seconds=HOST + 2,
                                                             quartz_after_seconds=HOST + 2 + WIDTH)
                                      for record in records if record['kind'] == 'presentation_sleep_clock'
                                      and record['data']['phase'] == 'end'],
        )
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index):
                records, controls = fixture()
                mutation(records, controls)
                self.invalid(records, controls)

    def test_post_interval_ambiguity_and_presentation_before_input(self):
        for value in (HOST + 1 / 16 - WIDTH, HOST + 1 / 16 + .002, math.inf, True):
            records, controls = fixture()
            controls[0]['native_result']['posts'][0]['quartz_after_seconds'] = value
            self.invalid(records, controls)
        with self.assertRaises(ValueError):
            evidence.latency_bounds(8.0001, 8, 8.0002)
        with self.assertRaises(ValueError):
            evidence.latency_bounds(7, 8, 8)
        records, controls = fixture()
        controls[0]['native_result']['posts'][0].update(
            quartz_before_seconds=HOST + .08, quartz_after_seconds=HOST + .08 + WIDTH)
        self.invalid(records, controls)

    def test_malformed_or_unmatched_control_effects(self):
        for value in (None, [], {}, {'frame_id': 999}, {'frame_id': 2, 'state_id': 99, 'unix_ns': 1, 'latency_us': 0}):
            records, controls = fixture()
            controls[0]['control_effect']['effect'] = value
            self.invalid(records, controls)
        for field, value in (('complete', False), ('origin', 'draw only'), ('baseline_frame_id', 2),
                             ('closing_frame_id', 2), ('source_origin', 'unreviewed source')):
            records, controls = fixture()
            controls[0]['control_effect'][field] = value
            self.invalid(records, controls)

    def test_receipt_identity_freshness_order_and_command_windows(self):
        mutations = (
            lambda records, controls: controls[0]['control_effect']['receipts'].reverse(),
            lambda records, controls: controls[0]['control_effect']['receipts'][0].update(fence=2),
            lambda records, controls: controls[0].update(started_unix_ns=UNIX),
            lambda records, controls: controls[0].update(finished_unix_ns=UNIX),
            lambda records, controls: controls[0]['control_effect']['effect'].update(unix_ns=UNIX),
            lambda records, controls: controls[0]['control_effect']['effect'].update(latency_us=0),
            lambda records, controls: data(records, 'input').update(input_sequence=2),
        )
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index):
                records, controls = fixture()
                mutation(records, controls)
                self.invalid(records, controls)

    def test_ownership_focus_display_geometry_and_input_release_preserved(self):
        for name, value in (('pid', 43), ('window_id', 0), ('frontmost', False), ('on_screen', False),
                            ('launch_unix_seconds', 0), ('bounds', [0, 0, 1, 1]), ('displays', [])):
            records, controls = fixture()
            controls[0]['binding_after'][name] = value
            self.invalid(records, controls)
        for name, value in (('code', 123), ('press_ms', 10), ('released', False), ('error', 'synthetic error')):
            records, controls = fixture()
            controls[0]['native_result'][name] = value
            self.invalid(records, controls)
        for notification in (1, 2, 3, 4, 5, 7, 8):
            records, controls = fixture()
            events(records, 9)[0]['platform_notification'] = notification
            self.invalid(records, controls)

    def test_drops_errors_unresolved_and_incomplete_drains(self):
        for kind in ('session', 'frame', 'input', 'presentation_event', 'session_end'):
            records, controls = fixture()
            next(record for record in records if record['kind'] == kind)['dropped_records'] = 1
            self.invalid(records, controls)
        records, controls = fixture()
        records.pop()
        self.invalid(records, controls)
        records, controls = fixture()
        data(records, 'frame')['state']['value']['error'] = 'synthetic'
        self.invalid(records, controls)

    def test_post_cutoff_delivery_is_only_reconciled_not_used_as_presentation(self):
        records, controls = fixture()
        callback = events(records, 6, 3)[0]
        target = next(record for record in records if record['kind'] == 'presentation_event'
                      and record['data'] is callback)
        records.remove(target)
        index = next(index for index, record in enumerate(records)
                     if record['kind'] == 'presentation_event' and record['data']['kind'] == 13)
        cutoff = data(records, 'presentation_end')['summary']['cutoff_host_time']
        callback.update(host_time=cutoff + 1 / 1024, callback_delivery_host_time=cutoff + 1 / 1024)
        callback['flags'] |= 1
        target['elapsed_us'] = records[index - 1]['elapsed_us'] + 1
        records.insert(index, target)
        renumber(records)
        result = evidence.analyze(records, controls)
        self.assertTrue(result['valid'], result['errors'])
        callback['presented_host_time'] = cutoff + 1 / 2048
        self.invalid(records, controls)

    def test_post_end_event_or_summary_cannot_hide_late_work(self):
        records, controls = fixture()
        records.insert(-1, copy.deepcopy(next(record for record in records
                                             if record['kind'] == 'presentation_event')))
        renumber(records)
        self.invalid(records, controls)
        records, controls = fixture()
        ending = next(record for record in records if record['kind'] == 'presentation_end')
        records.remove(ending)
        records.insert(3, ending)
        self.invalid(records, controls)

    def test_all_summary_and_post_clock_fields_are_finite_and_typed(self):
        for name in ('begin_host_time', 'cutoff_host_time', 'drain_end_host_time', 'snapshot_host_time'):
            for value in (None, True, -1, math.inf, math.nan, '8'):
                with self.subTest(name=name, value=value):
                    records, controls = fixture()
                    data(records, 'presentation_end')['summary'][name] = value
                    self.invalid(records, controls)
        for name in ('quartz_before_seconds', 'quartz_after_seconds', 'unix_before_ns',
                     'unix_after_ns', 'host_before_ns', 'host_after_ns'):
            for value in (None, True, -1, math.inf, math.nan, '8'):
                with self.subTest(name=name, value=value):
                    records, controls = fixture()
                    controls[0]['native_result']['posts'][0][name] = value
                    self.invalid(records, controls)

    def test_preserved_control_metric_cannot_be_malformed_or_relabelled(self):
        for field, value in (('latency_us', None), ('p95_limit_us', 100001),
                             ('maximum_limit_us', 250001), ('baseline_frame_id', True),
                             ('within_fixed_limits', 'yes')):
            records, controls = fixture()
            controls[0]['control_effect'][field] = value
            self.invalid(records, controls)

    def test_equality_cannot_hide_boolean_or_float_identity_fields(self):
        for name, value in (('input_sequence', 1.0), ('fence', True), ('received_unix_ns', 1.0)):
            records, controls = fixture()
            controls[0]['control_effect']['receipts'][0][name] = value
            self.invalid(records, controls)
        records, controls = fixture()
        controls[0]['native_result']['binding']['pid'] = 42.0
        self.invalid(records, controls)
        for field in ('samples', 'p50', 'p95', 'p99', 'worst'):
            records, controls = fixture()
            controls[0]['control_effect']['latency_us'][field] = 0
            self.invalid(records, controls)

    def test_consistent_binding_snapshots_reject_non_u32_display_identities(self):
        for field in ('display_id', 'mode_id'):
            with self.subTest(field=field):
                records, controls = fixture()
                for snapshot in (controls[0]['binding_before'], controls[0]['binding_after'],
                                 controls[0]['native_result']['binding']):
                    snapshot['displays'][0][field] = 2**32
                self.invalid(records, controls)

    def test_consistent_binding_snapshots_reject_unproduced_display_fields(self):
        records, controls = fixture()
        for snapshot in (controls[0]['binding_before'], controls[0]['binding_after'],
                         controls[0]['native_result']['binding']):
            snapshot['displays'][0]['instantaneous_refresh_hz'] = 60
        self.invalid(records, controls)

    def test_consistent_binding_snapshots_reject_nul_executable(self):
        records, controls = fixture()
        for snapshot in (controls[0]['binding_before'], controls[0]['binding_after'],
                         controls[0]['native_result']['binding']):
            snapshot['executable'] += '\x00ignored'
            snapshot['kernel_identity']['executable'] = snapshot['executable']
        self.invalid(records, controls)

    def test_consistent_binding_snapshots_reject_out_of_range_swift_ints(self):
        for field in ('width', 'height', 'pixel_width', 'pixel_height', 'maximum_frames_per_second'):
            with self.subTest(field=field):
                records, controls = fixture()
                for snapshot in (controls[0]['binding_before'], controls[0]['binding_after'],
                                 controls[0]['native_result']['binding']):
                    snapshot['displays'][0][field] = 2**63
                self.invalid(records, controls)

    def test_u32_display_identity_boundaries_are_supported(self):
        for mode_id in (0, 2**32 - 1):
            with self.subTest(mode_id=mode_id):
                records, controls = fixture()
                for snapshot in (controls[0]['binding_before'], controls[0]['binding_after'],
                                 controls[0]['native_result']['binding']):
                    snapshot['displays'][0].update(display_id=2**32 - 1, mode_id=mode_id)
                result = evidence.analyze(records, controls)
                self.assertTrue(result['valid'], result['errors'])

    def test_producer_publishes_context_before_each_frame(self):
        records, controls = fixture()
        for index, record in enumerate(records):
            if record['kind'] == 'frame':
                context = records[index - 1]
                self.assertEqual(context['kind'], 'presentation_event')
                self.assertEqual(context['data']['kind'], 2)
                self.assertEqual(context['data']['frame_id'], record['data']['frame_id'])
                self.assertEqual(context['data']['state_id'], record['data']['state']['state_id'])
        result = evidence.analyze(records, controls)
        self.assertTrue(result['valid'], result['errors'])

    def test_missing_or_ambiguous_clock_cover_and_lifecycle_metadata(self):
        for kind in ('presentation_sleep_clock', 'presentation_library', 'presentation_begin', 'presentation_end'):
            records, controls = fixture()
            target = next(record for record in records if record['kind'] == kind)
            records.remove(target)
            self.invalid(records, controls)
        for field, value in (('clock_api', 'DispatchTime'), ('quartz_after_seconds', HOST + 1),
                             ('schema', True)):
            records, controls = fixture()
            data(records, 'presentation_sleep_clock')[field] = value
            self.invalid(records, controls)
        records, controls = fixture()
        data(records, 'presentation_sleep_clock')['sample']['timebase_denom'] = 0
        self.invalid(records, controls)
        for field, value in (('event_size', 191), ('counters_size', 303), ('summary_size', 407),
                             ('source_sha256', '0' * 64), ('source_only', False)):
            records, controls = fixture()
            data(records, 'presentation_library')[field] = value
            self.invalid(records, controls)

    def test_state_freshness_earliest_effect_and_unsupported_readiness(self):
        for field, value in (('pending', True), ('modal', True), ('playing', True), ('error', 'synthetic')):
            records, controls = fixture()
            data(records, 'frame')['state']['value'][field] = value
            self.invalid(records, controls)
        records, controls = fixture()
        data(records, 'frame')['state']['value']['observation_capabilities']['route_terminals'] = 0
        self.invalid(records, controls)
        records, controls = fixture()
        frames = [record['data'] for record in records if record['kind'] == 'frame']
        frames[2]['state']['value']['camera']['center'] = [99, 0]
        self.invalid(records, controls)
        records, controls = fixture()
        controls[0]['control_effect']['effect']['frame_id'] = 3
        self.invalid(records, controls)

    def test_duplicate_reordered_events_receipts_and_controls_fail_closed(self):
        records, controls = fixture()
        events(records, 6, 2)[0]['event_sequence'] = 1
        self.invalid(records, controls)
        records, controls = fixture(2)
        controls.reverse()
        self.invalid(records, controls)
        records, controls = fixture(2)
        controls[0]['control_effect']['effect'] = copy.deepcopy(controls[1]['control_effect']['effect'])
        self.invalid(records, controls)
        records, controls = fixture()
        data(records, 'input')['input']['physical_key'] = 'Code(ArrowLeft)'
        self.invalid(records, controls)

    def test_no_input_injection_can_enable_qualification(self):
        records, controls = fixture()
        data(records, 'session').update(alpha_qualified=True, performance_qualified=True,
                                       native_qualified=True, eligible_seconds=999, civil_days=999)
        result = evidence.analyze(records, controls)
        self.assertTrue(result['valid'], result['errors'])
        self.assert_no_qualification(result)
        controls[0]['control_effect']['alpha_qualified'] = True
        self.invalid(records, controls)

    def test_existing_unbound_source_reader_metadata_is_not_enabled(self):
        records, controls = fixture()
        data(records, 'presentation_library').clear()
        data(records, 'presentation_library').update(abi_version=1)
        self.invalid(records, controls)
        records, controls = fixture()
        for post in controls[0]['native_result']['posts']:
            del post['clock_api']
            del post['quartz_before_seconds']
            del post['quartz_after_seconds']
        self.invalid(records, controls)

    def test_strict_json_finite_exponents_duplicate_fields_and_types(self):
        for payload in ('{"a":1,"a":2}', '1e9999', '-1e9999', '1e-9999', 'NaN',
                        'Infinity', '{"host_time":true,"host_time":1}', b'"\xff"', '[' * 10000):
            with self.subTest(payload=repr(payload[:80])):
                with self.assertRaises(ValueError):
                    evidence.parse_json(payload)
        self.assertEqual(evidence.parse_json('1e2'), 100)
        self.assertEqual(evidence.parse_json('0e-9999'), 0)

    def test_input_and_output_are_not_mutated_or_aliased(self):
        records, controls = fixture()
        before = copy.deepcopy((records, controls))
        first = evidence.analyze(records, controls)
        self.assertEqual((records, controls), before)
        first['samples'][0]['frame_id'] = 999
        first['api_presentation_latency']['p95_limit_us'] = 999
        self.assertEqual((records, controls), before)
        second = evidence.analyze(records, controls)
        self.assertTrue(second['valid'], second['errors'])
        self.assertEqual(second['samples'][0]['frame_id'], 2)
        self.assertEqual(second['api_presentation_latency']['p95_limit_us'], 100000)
        self.assert_no_qualification(second)

    def test_kernel_binding_requires_exact_integral_v2_lifetime(self):
        for field in ('pid', 'start_tvsec', 'start_tvusec'):
            for replacement in (True, float(owned_binding()['kernel_identity'][field]), -1):
                candidate = owned_binding()
                candidate['kernel_identity'][field] = replacement
                with self.subTest(field=field, replacement=replacement), self.assertRaises(ValueError):
                    evidence.binding(candidate, 42)
        for field, replacement in (('source', 'LaunchServices'), ('pid', 43), ('start_tvsec', 0),
                                   ('start_tvsec', 2**64), ('start_tvusec', 1_000_000),
                                   ('executable', '/synthetic/other')):
            candidate = owned_binding()
            candidate['kernel_identity'][field] = replacement
            with self.subTest(field=field), self.assertRaises(ValueError):
                evidence.binding(candidate, 42)
        for candidate in ({**owned_binding(), 'schema': 'sims-window-binding-v1'},
                          {**owned_binding(), 'launch_unix_seconds': 123},
                          {**owned_binding(), 'kernel_identity': {}}):
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                evidence.binding(candidate, 42)
        evidence.binding(owned_binding(), 42)

    def test_kernel_binding_lifetime_change_cannot_join_input(self):
        journal, controls = fixture()
        controls[0]['binding_after']['kernel_identity']['start_tvusec'] += 1
        result = evidence.analyze(journal, controls)
        self.assertFalse(result['valid'])
        self.assert_no_qualification(result)

    def test_audit_guard_rejects_imports_execution_network_native_and_signals(self):
        for module in FORBIDDEN_MODULES:
            with self.subTest(module=module):
                with self.assertRaises(RuntimeError):
                    sys.audit('import', module, None, None, None, None)
        for event in FORBIDDEN_EVENTS | {'socket.connect', 'ctypes.dlopen', 'subprocess.Popen', 'signal.signal'}:
            with self.subTest(event=event):
                with self.assertRaises(RuntimeError):
                    sys.audit(event, None)
        with self.assertRaises(RuntimeError):
            sys.audit('open', '/synthetic/not-opened', 'w', 1)
        self.assertFalse(FORBIDDEN_MODULES.intersection(sys.modules))

    def test_analyzer_imports_only_pure_diagnostic_dependencies(self):
        tree = ast.parse(Path(evidence.__file__).read_text())
        names = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                names.update(alias.name for alias in node.names)
            elif isinstance(node, ast.ImportFrom):
                names.add(node.module)
        self.assertEqual(names, {'json', 'math', 'fractions'})
        swift = Path(__file__).with_name('window_session_macos.swift').read_text()
        self.assertIn('let quartzBefore = CACurrentMediaTime()\n        event.postToPid(pid)\n'
                      '        let quartzAfter = CACurrentMediaTime()', swift)
        for field in ('unix_before_ns', 'unix_after_ns', 'host_before_ns', 'host_after_ns'):
            self.assertIn('"' + field + '"', swift)


if __name__ == '__main__':
    unittest.main(verbosity=2)
