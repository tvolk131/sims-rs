"""Entirely mocked child/helper lifecycle tests using synthetic private files."""
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

import play_alpha
import window_evidence as evidence
import window_session as driver
from test_window_evidence import ACTION, BASE, binding as evidence_binding, frame, payload, record


def binding():
    value = evidence_binding()
    value.pop('launch_unix_seconds')
    value.update({'schema': 'sims-window-binding-v2', 'kernel_identity': {
        'source': 'proc_pidinfo(PROC_PIDTBSDINFO)+proc_pidpath', 'pid': value['pid'],
        'start_tvsec': 1000, 'start_tvusec': 123456, 'executable': value['executable']}})
    return value


class FakeClock:
    def __init__(self):
        self.now = 0.0
        self.reads = 0
        self.on_sleep = lambda: None

    def monotonic(self):
        return self.now

    def time_ns(self):
        self.reads += 1
        return BASE + round(self.now * 1_000_000_000) + self.reads

    def sleep(self, duration):
        self.now += duration
        self.on_sleep()


class FakeChild:
    pid = 42

    def __init__(self, backend):
        self.backend = backend
        self.returncode = backend.start_code

    def poll(self):
        return self.returncode

    def wait(self, timeout):
        if self.returncode is None:
            raise subprocess.TimeoutExpired('fake-owned-child', timeout)
        return self.returncode

    def terminate(self):
        self.backend.signals.append('terminate')
        if self.backend.cleanup_fails:
            raise OSError('mock terminate failed')
        self.returncode = 0 if self.backend.forced_zero else -15

    def kill(self):
        self.backend.signals.append('kill')
        if self.backend.kill_fails:
            raise OSError('mock kill failed')
        self.returncode = -9


class FakeBackend:
    def __init__(self, clock):
        self.clock = clock
        clock.on_sleep = self.advance
        self.command = None
        self.calls, self.signals = [], []
        self.items = []
        self.start_code = None
        self.key_error = None
        self.close_ignored = self.cleanup_fails = self.kill_fails = self.forced_zero = False
        self.block_baseline = self.modal = self.write_save = False
        self.after_close = lambda: None
        self.after_start = lambda: None
        self.sequence = 0
        self.counter = 0
        self.pending_observations = 0

    def elapsed(self):
        return round(self.clock.now * 1_000_000)

    def flush(self):
        self.path.write_bytes(payload(self.items))

    def add_frame(self, center=None, fence=1):
        self.counter += 1
        self.items.append(frame(self.counter, self.elapsed(), center, sequence=self.sequence,
                                fence=fence, modal=self.modal))
        self.flush()

    def start(self, command, cwd, stdout, stderr):
        self.command = command
        self.telemetry = Path(command[command.index('--window-session-output') + 1])
        self.telemetry.mkdir()
        self.path = self.telemetry / 'events.jsonl'
        self.items = [record('session', 0, {'schema': 1, 'pid': 42, 'unix_ns': str(BASE)})]
        self.clock.now += .001
        self.add_frame(fence=0)
        self.child = FakeChild(self)
        self.after_start()
        return self.child

    def advance(self):
        if not self.command or self.child.returncode is not None:
            return
        fence = self.telemetry / 'observer-fence'
        if fence.exists() and fence.read_bytes() == (1).to_bytes(8, 'little') and not self.block_baseline:
            self.add_frame([1, 0] if self.sequence else None)

    def helper(self, executable, request, timeout):
        self.calls.append(copy.deepcopy(request))
        operation = request['operation']
        current = binding()
        current['executable'] = self.command[0]
        current['kernel_identity']['executable'] = self.command[0]
        if operation == 'observe' and self.pending_observations:
            self.pending_observations -= 1
            return subprocess.CompletedProcess([], 0, json.dumps({'status': 'not_ready',
                'reason': 'owned_window_unavailable', 'kernel_identity': current['kernel_identity']}), '')
        response = {'operation': operation, 'binding': current}
        if operation == 'key':
            if self.key_error:
                raise self.key_error
            start = self.clock.time_ns()
            self.clock.now += .001
            self.sequence = 1
            self.items.append(record('input', self.elapsed(), {'input_sequence': 1, 'fence': 1,
                'received_unix_ns': self.clock.time_ns(), 'input': evidence.expected_receipts(ACTION)[0]}))
            self.clock.now += .010
            self.add_frame([1, 0])
            self.clock.now += .009
            self.sequence = 2
            self.items.append(record('input', self.elapsed(), {'input_sequence': 2, 'fence': 1,
                'received_unix_ns': self.clock.time_ns(), 'input': evidence.expected_receipts(ACTION)[1]}))
            self.flush()
            response.update({'code': 124, 'press_ms': 20, 'released': True, 'posts': [
                {'down': True, 'unix_before_ns': start, 'unix_after_ns': start + 100,
                 'host_before_ns': start, 'host_after_ns': start + 100},
                {'down': False, 'unix_before_ns': self.clock.time_ns(),
                 'unix_after_ns': self.clock.time_ns() + 100,
                 'host_before_ns': self.clock.time_ns(), 'host_after_ns': self.clock.time_ns() + 100}]})
        elif operation == 'close':
            if not self.close_ignored:
                self.clock.now += .001
                self.items.append(record('session_end', self.elapsed(), {'dropped_records': 0}))
                self.flush()
                if self.write_save:
                    output = Path(self.command[self.command.index('--save') + 1])
                    source = self.command[self.command.index('--original') + 1]
                    output.write_text(json.dumps({'session': {'version': 2, 'source': source,
                        'options': {'routing': 'Strict'}}, 'dialog_client': {}}))
                self.child.returncode = 0
                self.after_close()
        if self.cleanup_fails and operation in ('close', 'release'):
            return subprocess.CompletedProcess([], 1, '{"partial":"mock"}', 'mock cleanup helper failed')
        return subprocess.CompletedProcess([], 0, json.dumps(response), '')


class KernelIdentityTests(unittest.TestCase):
    def test_v2_uses_integral_kernel_lifetime_without_launchservices_metadata(self):
        current = binding()
        self.assertNotIn('launch_unix_seconds', current)
        self.assertNotIn('launch_services', current)
        self.assertEqual(driver.validate_binding(current, 42, '/private/fake'), current)
        for microseconds in (0, 999999):
            current['kernel_identity']['start_tvusec'] = microseconds
            self.assertEqual(driver.validate_binding(current, 42, '/private/fake'), current)

    def test_v1_and_incomplete_or_malformed_bindings_are_rejected(self):
        for current in (None, [], {}, evidence_binding(), {**binding(), 'launch_unix_seconds': 1000.0},
                        {**binding(), 'kernel_identity': {}}, {**binding(), 'schema': 'sims-window-binding-v1'}):
            with self.subTest(current=current), self.assertRaises(ValueError):
                driver.validate_binding(current, 42, '/private/fake')
        for name in binding():
            current = binding()
            del current[name]
            with self.subTest(missing=name), self.assertRaises(ValueError):
                driver.validate_binding(current, 42, '/private/fake')

    def test_kernel_fields_are_strict_integral_and_exact(self):
        invalid = {'pid': [None, True, False, 42.0, '42', 0, -1, 43, 2**31],
                   'start_tvsec': [None, True, False, 0, -1, 1000.0, '1000', 2**64],
                   'start_tvusec': [None, True, False, -1, 1_000_000, 123456.0, '123456', 2**64],
                   'source': [None, True, 'LaunchServices', 'proc_pidpath'],
                   'executable': [None, True, '/other', 'private/fake', '/private/fake\0']}
        for name, values in invalid.items():
            for value in values:
                current = binding()
                current['kernel_identity'][name] = value
                with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                    driver.validate_binding(current, 42, '/private/fake')
        for name in binding()['kernel_identity']:
            current = binding()
            del current['kernel_identity'][name]
            with self.subTest(missing=name), self.assertRaises(ValueError):
                driver.validate_binding(current, 42, '/private/fake')
        current = binding()
        current['kernel_identity']['launch_date'] = 'diagnostics cannot substitute'
        with self.assertRaises(ValueError):
            driver.validate_binding(current, 42, '/private/fake')

    def test_same_pid_different_kernel_lifetime_or_path_is_rejected(self):
        original = binding()
        for name, value in (('start_tvsec', 1001), ('start_tvusec', 123457), ('executable', '/other')):
            current = copy.deepcopy(original)
            current['kernel_identity'][name] = value
            with self.subTest(name=name), self.assertRaises(ValueError):
                driver.validate_binding(current, 42, '/private/fake', original)

    def test_optional_metadata_crosscheck_never_substitutes_for_kernel_identity(self):
        for metadata in ({'registered': False}, {'registered': True},
                         {'registered': True, 'executable': '/private/fake'},
                         {'registered': True, 'launch_date': 'diagnostic only'}):
            driver.validate_launch_services(metadata, '/private/fake')
        for metadata in (None, {}, {'registered': 1}, {'registered': True, 'executable': '/foreign'},
                         {'registered': False, 'executable': '/private/fake'},
                         {'registered': True, 'launch_date': 1000.0},
                         {'registered': True, 'launch_unix_seconds': 1000.0}):
            with self.subTest(metadata=metadata), self.assertRaises(ValueError):
                driver.validate_launch_services(metadata, '/private/fake')
        incomplete = binding()
        del incomplete['kernel_identity']
        incomplete['launch_services'] = {'registered': True, 'executable': '/private/fake',
                                         'launch_date': 'cannot substitute'}
        with self.assertRaises(ValueError):
            driver.validate_binding(incomplete, 42, '/private/fake')

    def test_top_level_pid_window_focus_and_display_guards_remain_strict(self):
        original = binding()
        for name, value in (('pid', True), ('pid', 42.0), ('window_id', True), ('window_id', 71.0),
                            ('window_id', 2**32), ('frontmost', 1), ('frontmost', False),
                            ('on_screen', 1), ('on_screen', False), ('bounds', [10, 10, 100, 625]),
                            ('bounds', [float('nan'), 10, 800, 625])):
            current = copy.deepcopy(original)
            current[name] = value
            with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                driver.validate_binding(current, 42, '/private/fake', original)
        for name, value in (('mode_id', 3), ('active', False), ('asleep', True), ('display_id', 7)):
            current = copy.deepcopy(original)
            current['displays'][0][name] = value
            with self.subTest(display=name), self.assertRaises(ValueError):
                driver.validate_binding(current, 42, '/private/fake', original)

    def swift_function(self, name):
        source = (driver.SOURCE / 'window_session_macos.swift').read_text()
        return source.split('func ' + name + '(', 1)[1].split('\n}', 1)[0]

    def test_swift_source_contract_rejects_numeric_aliases_before_binding_equality(self):
        strict = self.swift_function('validateExpected')
        for field in ('frontmost', 'on_screen'):
            self.assertIn('isBoolean(provided["' + field + '"])', strict)
        for field in ('display_id', 'mode_id'):
            self.assertIn('unsignedInteger(display["' + field + '"]', strict)
        for field in ('width', 'height', 'pixel_width', 'pixel_height', 'maximum_frames_per_second',
                      'builtin', 'active', 'asleep'):
            self.assertIn('"' + field + '"', strict)
        self.assertIn('unsignedInteger(display[name]', strict)
        self.assertIn('isBoolean(display[name])', strict)
        self.assertIn('finiteNumber(display["nominal_refresh_hz"])', strict)
        self.assertIn('bounds.allSatisfy({ finiteNumber($0) })', strict)
        self.assertLess(strict.index('for display in providedDisplays'), strict.index('same(provided, current)'))
        self.assertIn('CFGetTypeID(number) == CFBooleanGetTypeID()', self.swift_function('isBoolean'))
        self.assertIn('CFGetTypeID(number) != CFBooleanGetTypeID()', self.swift_function('finiteNumber'))

    def test_python_binding_rejects_boolean_and_float_display_aliases(self):
        original = binding()
        for field in ('display_id', 'mode_id', 'width', 'height', 'pixel_width',
                      'pixel_height', 'maximum_frames_per_second'):
            for value in (True, float(original['displays'][0][field])):
                current = copy.deepcopy(original)
                current['displays'][0][field] = value
                with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                    driver.validate_binding(current, 42, '/private/fake', original)
        for field in ('builtin', 'active', 'asleep'):
            current = copy.deepcopy(original)
            current['displays'][0][field] = int(current['displays'][0][field])
            with self.subTest(field=field), self.assertRaises(ValueError):
                driver.validate_binding(current, 42, '/private/fake', original)

    def test_swift_source_contract_uses_sdk_kernel_reads_and_integer_fields(self):
        process = self.swift_function('processInfo')
        self.assertIn('MemoryLayout<proc_bsdinfo>.size', process)
        self.assertIn('proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, $0, Int32(size))', process)
        self.assertIn('count == Int32(size)', process)
        for check in ('info.pbi_pid == UInt32(pid)', 'info.pbi_start_tvsec > 0',
                      'info.pbi_start_tvusec < 1_000_000', 'UInt32(SRUN)', 'UInt32(SSLEEP)',
                      'info.pbi_flags & UInt32(PROC_FLAG_INEXIT) == 0'):
            self.assertIn(check, process)
        self.assertNotIn('notReady', process)
        identity = self.swift_function('kernelIdentity')
        self.assertIn('count: processPathCapacity', identity)
        self.assertIn('let processPathCapacity = 4 * Int(MAXPATHLEN)',
                      (driver.SOURCE / 'window_session_macos.swift').read_text())
        self.assertIn('proc_pidpath(pid, $0.baseAddress, UInt32(capacity))', identity)
        self.assertIn('count > 0, count < Int32(capacity)', identity)
        self.assertIn('Int(count) == end || Int(count) == end + 1', identity)
        self.assertIn('path == executable', identity)
        self.assertIn('before.pbi_start_tvsec == after.pbi_start_tvsec', identity)
        self.assertIn('before.pbi_start_tvusec == after.pbi_start_tvusec', identity)
        self.assertEqual(identity.count('try processInfo(pid)'), 2)
        for forbidden in ('Double(', 'Float(', 'timeIntervalSince1970', 'launchDate', 'notReady'):
            self.assertNotIn(forbidden, identity)

    def test_swift_source_contract_launchservices_is_optional_not_identity(self):
        metadata = self.swift_function('launchServices')
        self.assertIn('else { return ["registered": false] }', metadata)
        self.assertIn('if let path = application.executableURL?', metadata)
        self.assertIn('if let launched = application.launchDate', metadata)
        self.assertIn('path == executable', metadata)
        self.assertNotIn('notReady', metadata)
        self.assertNotIn('kernel_identity', metadata)
        owner = self.swift_function('windowBinding')
        self.assertIn('"schema": "sims-window-binding-v2"', owner)
        self.assertIn('"kernel_identity": identity', owner)
        for forbidden in ('launchDate', 'executableURL', 'launch_unix_seconds'):
            self.assertNotIn(forbidden, owner)

    def test_swift_source_contract_repeats_window_focus_display_and_lifetime(self):
        owner = self.swift_function('windowBinding')
        for check in ('kCGWindowOwnerPID', 'kCGWindowLayer', 'records.count == 1', 'kCGWindowIsOnscreen',
                      'kCGWindowAlpha', '.optionIncludingWindow, targetWindow',
                      'identifier == UInt64(targetWindow)', 'try focusedOwnedWindow(pid,',
                      'try displays(bounds)'):
            self.assertIn(check, owner)
        focus = self.swift_function('focusedOwnedWindow')
        for check in ('AXIsProcessTrusted()', 'AXUIElementCreateApplication(pid)', 'windows.count == 1',
                      'kAXFocusedWindowAttribute', 'CFEqual(focused, target)', 'owner == pid',
                      'kAXFrontmostAttribute', 'isBoolean(frontmost)', 'if !active',
                      'Accessibility and on-screen window geometry disagree'):
            self.assertIn(check, focus)
        self.assertNotIn('application.isActive', owner)
        repeat = self.swift_function('binding')
        self.assertEqual(repeat.count('try kernelIdentity(pid, executable)'), 2)
        self.assertEqual(repeat.count('try windowBinding(pid, executable,'), 2)
        self.assertIn('same(identity, confirmedIdentity)', repeat)
        self.assertIn('same(identity, targetIdentity)', repeat)
        self.assertIn('same(confirmedIdentity, targetIdentity)', repeat)
        self.assertIn('same(first, second)', repeat)
        source = (driver.SOURCE / 'window_session_macos.swift').read_text()
        for forbidden in ('.activate(', 'activateWithOptions', 'NSWorkspace.shared',
                          '.optionOnScreenOnly', 'kCGNullWindowID',
                          'CGRequestPostEventAccess', 'AXIsProcessTrustedWithOptions'):
            self.assertNotIn(forbidden, source)

    def test_swift_source_contract_existing_post_release_and_close_gates_remain(self):
        run = self.swift_function('run')
        self.assertLess(run.index('let current = try binding(pid, executable, targetWindow, targetIdentity)'), run.index('event.postToPid(pid)'))
        self.assertLess(run.index('try validateExpected(provided, current)'), run.index('CGPreflightPostEventAccess()'))
        self.assertIn('Input requires an existing owned window binding', run)
        self.assertIn('unsignedInteger(request["code"]) == 124', run)
        self.assertIn('unsignedInteger(request["press_ms"]) == 20', run)
        self.assertIn('heldRelease = release', run)
        self.assertIn('try post(release, down: false)', run)
        self.assertIn('if let heldRelease', run)
        self.assertIn('try post(heldRelease, down: false)', run)
        close = self.swift_function('closeOwnedWindow')
        self.assertIn('AXUIElementGetPid(target, &owner) == .success && owner == pid', close)
        self.assertLess(close.index('same(current, expected)'), close.index('AXUIElementPerformAction'))
        strict = self.swift_function('validateExpected')
        for field in ('identity["pid"]', 'identity["start_tvsec"]', 'identity["start_tvusec"]'):
            self.assertIn('unsignedInteger(' + field, strict)
        self.assertIn('CFBooleanGetTypeID()', self.swift_function('unsignedInteger'))


class DriverTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.cache = self.root / 'cache'
        self.cache.mkdir()
        self.game = self.root / 'installation'
        self.game.mkdir()
        self.binary = self.root / 'synthetic-room'
        self.binary.write_bytes(b'synthetic application bytes, never executable')
        self.binary.chmod(0o700)
        self.helper = self.root / 'synthetic-helper'
        self.helper.write_bytes(b'synthetic helper bytes, never executable')
        self.plan_file, self.endpoint_file = self.root / 'plan.json', self.root / 'endpoints.json'
        self.plan = {'schema': 1, 'timeout_seconds': 10, 'household': 'newbie',
                     'steps': [{'name': 'camera-right', 'input': ACTION}]}
        self.plan_file.write_text(json.dumps(self.plan))
        self.specification = {'schema': 1, 'plan_sha256': play_alpha.digest(self.plan_file),
            'controls': [{'step': 'camera-right', 'conditions': evidence.CAMERA_CONDITIONS}]}
        self.endpoint_file.write_text(json.dumps(self.specification))
        self.receipt_file = self.root / 'helper-build.json'
        self.build = {'schema': 'sims-window-helper-build-v1',
            'source_sha256': play_alpha.digest(driver.SOURCE / 'window_session_macos.swift'),
            'executable_sha256': play_alpha.digest(self.helper),
            'source_path': str(driver.SOURCE / 'window_session_macos.swift'), 'binary_path': str(self.helper),
            'compiler': 'synthetic swift compiler attestation; not compiled in tests',
            'command': ['/explicit/swiftc', str(driver.SOURCE / 'window_session_macos.swift'), '-o', str(self.helper)]}
        self.receipt_file.write_text(json.dumps(self.build))
        for filename in driver.SCHEMA_PATHS:
            target = self.root / filename
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('synthetic schema source for filesystem-only tests\n')
        for patch in (mock.patch.object(play_alpha, 'REPOSITORY', self.root),
                      mock.patch.object(subprocess, 'run', side_effect=AssertionError('Unmocked subprocess.run')),
                      mock.patch.object(subprocess, 'Popen', side_effect=AssertionError('Unmocked subprocess.Popen'))):
            patch.start()
            self.addCleanup(patch.stop)
        self.count = 0

    def prepare(self):
        self.count += 1
        runner = driver.prepare(self.cache / ('diagnostic-' + str(self.count)), self.binary, self.game,
                                self.helper, self.receipt_file, self.plan_file, self.endpoint_file)
        clock = FakeClock()
        runner.clock = clock
        runner.started = 0
        runner.deadline, runner.active_deadline = 10, 2
        runner.backend = FakeBackend(clock)
        self.runner = runner
        return runner

    def invoke(self, runner=None, **options):
        runner = runner or self.prepare()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            result = play_alpha.launch(self.binary, self.game, timeout=10,
                window_session_output=runner.directory / 'telemetry', child_runner=runner, **options)
        self.report = json.loads((runner.directory / 'result.json').read_bytes())
        receipts = list((self.cache / 'alpha-sessions').glob('household-*/launch.json'))
        self.launch_report = next(json.loads(path.read_bytes()) for path in receipts
            if json.loads(path.read_bytes()).get('window_session_output') == str(runner.directory / 'telemetry'))
        return result

    def save(self, routing='Strict'):
        selected = self.root / 'selected.json'
        selected.write_text(json.dumps({'session': {'version': 2, 'source': play_alpha.HOUSEHOLDS['newbie'],
            'options': {'routing': routing}}, 'dialog_client': {}}))
        return selected

    def test_success_is_diagnostic_only_actual_child_pid_and_pins(self):
        self.assertEqual(self.invoke(), 0)
        self.assertEqual(self.report['pid'], 42)
        self.assertEqual(self.report['actual_child_returncode'], 0)
        self.assertTrue(self.report['graceful_end'])
        self.assertFalse(self.report['forced_teardown'])
        self.assertTrue(self.report['inputs_unchanged'])
        self.assertTrue(all(item['unchanged'] for item in self.report['inputs']))
        self.assertAlmostEqual(self.report['control_effect']['latency_us']['p95'], 10_000, delta=.1)
        self.assertFalse(any(self.report[name] for name in evidence.QUALIFICATIONS))
        self.assertEqual([item['operation'] for item in self.runner.backend.calls],
                         ['observe', 'observe', 'key', 'observe', 'close'])
        self.assertTrue(all(item['pid'] == 42 for item in self.runner.backend.calls))
        self.assertFalse(self.report['bundle_closure_claimed'])

    def test_observer_fence_matches_the_rust_complete_u64_token(self):
        runner = self.prepare()
        self.assertEqual(self.invoke(runner), 0)
        self.assertEqual((runner.telemetry / 'observer-fence').read_bytes(), (1).to_bytes(8, 'little'))

    def test_initial_unavailable_window_never_posts_before_a_bound_observation(self):
        runner = self.prepare()
        runner.backend.pending_observations = 2
        self.assertEqual(self.invoke(runner), 0)
        operations = [request['operation'] for request in runner.backend.calls]
        self.assertEqual(operations[:3], ['observe', 'observe', 'observe'])
        self.assertGreater(operations.index('key'), 2)

    def test_optional_launchservices_metadata_never_becomes_lifetime_authority(self):
        for metadata in (None, {'registered': False}, {'registered': True},
                         {'registered': True, 'launch_date': 'diagnostic only'}):
            runner = self.prepare()
            helper = runner.backend.helper
            def diagnostic(executable, request, timeout):
                result = helper(executable, request, timeout)
                response = json.loads(result.stdout)
                if metadata is not None:
                    response['launch_services'] = metadata
                return subprocess.CompletedProcess([], result.returncode, json.dumps(response), result.stderr)
            runner.backend.helper = diagnostic
            with self.subTest(metadata=metadata):
                self.assertEqual(self.invoke(runner), 0)
                self.assertEqual(self.report['binding']['kernel_identity']['start_tvusec'], 123456)
                self.assertTrue(self.report['graceful_end'])

    def test_unbound_input_release_and_close_are_blocked_before_helper(self):
        runner = self.prepare()
        runner.child = FakeChild(runner.backend)
        for operation in ('key', 'release', 'close'):
            with self.subTest(operation=operation), self.assertRaisesRegex(ValueError, 'existing owned'):
                runner.native(operation)
        self.assertEqual(runner.backend.calls, [])

    def test_initial_not_ready_requires_complete_kernel_identity_and_allowed_reason(self):
        for response in ({'status': 'not_ready'}, {'status': 'not_ready', 'reason': 'launch_identity_unavailable',
                           'kernel_identity': binding()['kernel_identity']},
                         {'status': 'not_ready', 'reason': 'owned_window_unavailable', 'kernel_identity': {}},
                         {'status': 'not_ready', 'reason': 'owned_window_unavailable',
                           'kernel_identity': {**binding()['kernel_identity'], 'pid': 43}}):
            runner = self.prepare()
            def unavailable(executable, request, timeout):
                runner.backend.calls.append(copy.deepcopy(request))
                return subprocess.CompletedProcess([], 0, json.dumps(response), '')
            runner.backend.helper = unavailable
            with self.subTest(response=response):
                self.assertEqual(self.invoke(runner), -15)
                self.assertEqual(self.report['actual_child_returncode'], -15)
                self.assertIsNone(self.report['binding'])
                self.assertEqual(self.report['binding_state'], 'unbound')
                self.assertEqual([call['operation'] for call in runner.backend.calls], ['observe'])
                self.assertEqual(runner.backend.signals, ['terminate'])

    def test_initial_kernel_failure_is_not_a_readiness_retry(self):
        for failure in ('Incomplete or failed kernel process identity read', 'Invalid kernel process lifetime',
                        'Owned kernel process is not live and runnable',
                        'Incomplete, invalid or wrong kernel executable path',
                        'Owned kernel process lifetime changed during read',
                        'LaunchServices executable disagrees with kernel identity'):
            runner = self.prepare()
            def failed(executable, request, timeout):
                runner.backend.calls.append(copy.deepcopy(request))
                return subprocess.CompletedProcess([], 1, json.dumps({'error': failure}), failure)
            runner.backend.helper = failed
            with self.subTest(failure=failure):
                self.assertEqual(self.invoke(runner), -15)
                self.assertEqual(self.report['actual_child_returncode'], -15)
                self.assertIn(failure, self.report['primary_failure'])
                self.assertIsNone(self.report['native_posts'])
                self.assertEqual(len(runner.backend.calls), 1)
                self.assertEqual(runner.backend.signals, ['terminate'])

    def test_initial_foreground_registration_retry_stays_bounded_and_kernel_pinned(self):
        runner = self.prepare()
        runner.backend.pending_observations = 2
        helper = runner.backend.helper
        def pending(executable, request, timeout):
            result = helper(executable, request, timeout)
            response = json.loads(result.stdout)
            if response.get('status') == 'not_ready':
                response['reason'] = 'foreground_application_unavailable'
            return subprocess.CompletedProcess([], result.returncode, json.dumps(response), result.stderr)
        runner.backend.helper = pending
        self.assertEqual(self.invoke(runner), 0)
        self.assertEqual([call['operation'] for call in runner.backend.calls[:3]], ['observe'] * 3)
        self.assertEqual(self.report['kernel_identity'], self.report['binding']['kernel_identity'])
        self.assertTrue(self.report['graceful_end'])

    def test_reused_lifetime_during_initial_wait_cannot_bind_or_post(self):
        for field in ('start_tvsec', 'start_tvusec', 'executable'):
            runner = self.prepare()
            runner.backend.pending_observations = 1
            helper = runner.backend.helper
            def changed(executable, request, timeout):
                result = helper(executable, request, timeout)
                response = json.loads(result.stdout)
                if 'binding' in response:
                    identity = response['binding']['kernel_identity']
                    identity[field] = '/foreign' if field == 'executable' else identity[field] + 1
                return subprocess.CompletedProcess([], result.returncode, json.dumps(response), result.stderr)
            runner.backend.helper = changed
            with self.subTest(field=field):
                self.assertEqual(self.invoke(runner), -15)
                self.assertEqual(self.report['actual_child_returncode'], -15)
                self.assertEqual([call['operation'] for call in runner.backend.calls], ['observe', 'observe'])
                self.assertIsNone(self.report['binding'])
                self.assertIsNotNone(self.report['kernel_identity'])
                self.assertIsNone(self.report['native_posts'])
                self.assertEqual(runner.backend.signals, ['terminate'])

    def test_later_not_ready_focus_window_or_identity_loss_never_posts_key(self):
        for failure in ('not_ready', 'focus', 'window', 'lifetime', 'executable', 'kernel_failure'):
            runner = self.prepare()
            helper = runner.backend.helper
            def lost(executable, request, timeout):
                if 'binding' not in request:
                    return helper(executable, request, timeout)
                runner.backend.calls.append(copy.deepcopy(request))
                current = copy.deepcopy(request['binding'])
                if failure == 'not_ready':
                    response = {'status': 'not_ready', 'reason': 'owned_window_unavailable',
                                'kernel_identity': current['kernel_identity']}
                elif failure == 'kernel_failure':
                    return subprocess.CompletedProcess([], 1, '{"error":"kernel identity lost"}', 'kernel identity lost')
                else:
                    if failure == 'focus':
                        current['frontmost'] = False
                    elif failure == 'window':
                        current['window_id'] += 1
                    elif failure == 'lifetime':
                        current['kernel_identity']['start_tvusec'] += 1
                    else:
                        current['kernel_identity']['executable'] = '/foreign'
                    response = {'operation': request['operation'], 'binding': current}
                return subprocess.CompletedProcess([], 0, json.dumps(response), '')
            runner.backend.helper = lost
            with self.subTest(failure=failure):
                self.assertEqual(self.invoke(runner), -15)
                self.assertEqual(self.report['actual_child_returncode'], -15)
                self.assertEqual([call['operation'] for call in runner.backend.calls],
                                 ['observe', 'observe', 'release', 'close'])
                self.assertEqual(runner.backend.signals, ['terminate'])
                self.assertIsNotNone(self.report['binding'])
                self.assertIsNone(self.report['native_posts'])
                self.assertTrue(all(item['error'] for item in self.report['cleanup']
                                    if item['operation'] in ('release', 'close')))
                self.assertFalse(self.launch_report['continue_candidate'])

    def test_bound_key_timeout_still_releases_and_closes_only_owned_child(self):
        runner = self.prepare()
        runner.backend.key_error = subprocess.TimeoutExpired('mock-helper', 2)
        self.assertEqual(self.invoke(runner), 124)
        self.assertEqual([call['operation'] for call in runner.backend.calls],
                         ['observe', 'observe', 'key', 'release', 'close'])
        self.assertTrue(all(call['pid'] == runner.child.pid for call in runner.backend.calls))
        self.assertTrue(all(call['binding'] == runner.binding for call in runner.backend.calls[1:]))
        self.assertEqual(runner.backend.signals, [])
        self.assertEqual(self.report['status'], 'timeout')
        self.assertEqual(self.report['actual_child_returncode'], 0)

    def test_post_receipt_rejects_float_boolean_legacy_or_changed_kernel_identity(self):
        self.assertEqual(self.invoke(), 0)
        original = self.report['native_posts']
        for field, value in (('pid', 42.0), ('start_tvsec', 1000.0), ('start_tvsec', True),
                             ('start_tvusec', 123456.0), ('start_tvusec', 123457), ('executable', '/foreign')):
            post = copy.deepcopy(original)
            post['binding']['kernel_identity'][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                driver.validate_post(post, self.report['binding'])
        post = copy.deepcopy(original)
        post['binding'] = evidence_binding()
        with self.assertRaises(ValueError):
            driver.validate_post(post, self.report['binding'])

    def test_unbound_wait_inspects_stat_without_parsing_journal(self):
        runner = self.prepare()
        runner.backend.pending_observations = 1000
        with mock.patch.object(evidence, 'journal', wraps=evidence.journal) as parse:
            self.assertEqual(self.invoke(runner), 124)
        self.assertEqual(parse.call_count, 0)
        self.assertFalse(self.report["journal_final_observation"]["complete"])
        self.assertIsNone(self.report['binding'])
        self.assertEqual(self.report['journal_buffer']['bytes'], len(runner.payload))
        self.assertGreater(self.report['journal_on_disk']['stat']['bytes'], 0)
        self.assertEqual(self.report['journal_on_disk']['status'], 'inspected')
        self.assertIsNone(self.report['journal_on_disk']['sha256'])
        self.assertEqual(runner.backend.signals, ['terminate'])
        self.assertTrue(all(call['operation'] == 'observe' for call in runner.backend.calls))

    def test_unbound_journal_crossing_cap_fails_before_binding_without_read_or_truncation(self):
        runner = self.prepare()
        runner.backend.pending_observations = 1000
        helper = runner.backend.helper
        def growing(executable, request, timeout):
            result = helper(executable, request, timeout)
            with runner.backend.path.open('r+b') as stream:
                stream.seek(evidence.MAX_JOURNAL_BYTES)
                stream.write(b'\n')
            return result
        runner.backend.helper = growing
        with mock.patch.object(evidence, 'journal', wraps=evidence.journal) as parse:
            self.assertEqual(self.invoke(runner), -15)
        self.assertEqual(self.report['actual_child_returncode'], -15)
        self.assertEqual(self.report['signal'], 15)
        parse.assert_not_called()
        self.assertIsNone(self.report['binding'])
        self.assertIsNone(self.report['native_posts'])
        self.assertEqual(len(runner.backend.calls), 1)
        self.assertEqual(runner.backend.path.stat().st_size, evidence.MAX_JOURNAL_BYTES + 1)
        self.assertEqual(self.report['primary_failure'], 'Unsafe or oversized journal')
        self.assertEqual(self.report['status'], 'diagnostic_failed')
        self.assertEqual(self.report['journal_buffer']['bytes'], 0)
        self.assertEqual(self.report['journal_bytes'], 0)
        self.assertEqual(self.report['journal_buffer']['sha256'], driver.hashlib.sha256(b'').hexdigest())
        self.assertEqual(self.report['journal_on_disk']['stat']['bytes'], evidence.MAX_JOURNAL_BYTES + 1)
        self.assertEqual(self.report['journal_on_disk']['error'], 'Unsafe or oversized journal')
        self.assertIsNone(self.report['journal_on_disk']['sha256'])
        self.assertTrue(self.report['journal_inspection_errors'])
        self.assertTrue(self.report['journal_read_errors'])
        self.assertTrue(any('Final journal: Unsafe or oversized journal' in error for error in self.report['post_errors']))
        self.assertTrue(any(entry['operation'] == 'wait_close' for entry in self.report['cleanup']))
        self.assertEqual(runner.backend.signals, ['terminate'])
        self.assertFalse(self.report['graceful_end'])
        self.assertFalse(self.launch_report['continue_candidate'])

    def test_primary_kernel_error_is_separate_from_cleanup_and_oversized_final_journal(self):
        runner = self.prepare()
        def failed(executable, request, timeout):
            runner.backend.calls.append(copy.deepcopy(request))
            with runner.backend.path.open('r+b') as stream:
                stream.seek(evidence.MAX_JOURNAL_BYTES)
                stream.write(b'\n')
            return subprocess.CompletedProcess([], 1, '{"error":"kernel failure"}', 'kernel failure')
        runner.backend.helper = failed
        self.assertEqual(self.invoke(runner), -15)
        self.assertEqual(self.report['actual_child_returncode'], -15)
        self.assertEqual(self.report['primary_failure'], 'Native helper failed: kernel failure')
        self.assertEqual(self.report['journal_bytes'], 0)
        self.assertEqual(self.report['journal_on_disk']['stat']['bytes'], evidence.MAX_JOURNAL_BYTES + 1)
        self.assertTrue(any('Unsafe or oversized journal' in error for error in self.report['post_errors']))
        self.assertTrue(any(entry.get('error') for entry in self.report['cleanup']))
        self.assertIsNone(self.report['binding'])

    def test_unbound_inspection_rejects_replaced_linked_deleted_or_shrunk_journal(self):
        for failure in ('replace', 'hardlink', 'symlink', 'delete', 'shrink'):
            runner = self.prepare()
            runner.backend.pending_observations = 1000
            helper = runner.backend.helper
            def unsafe(executable, request, timeout):
                result = helper(executable, request, timeout)
                path = runner.backend.path
                if failure == 'replace':
                    replacement = path.with_name('replacement')
                    replacement.write_bytes(path.read_bytes())
                    replacement.replace(path)
                elif failure == 'hardlink':
                    os.link(path, path.with_name('linked'))
                elif failure == 'symlink':
                    path.unlink()
                    path.symlink_to(self.plan_file)
                elif failure == 'delete':
                    path.unlink()
                else:
                    path.write_bytes(b'{}\n')
                return result
            runner.backend.helper = unsafe
            with self.subTest(failure=failure):
                self.assertEqual(self.invoke(runner), -15)
                self.assertEqual(self.report['actual_child_returncode'], -15)
                self.assertEqual(len(runner.backend.calls), 1)
                self.assertIsNone(self.report['binding'])
                self.assertEqual(self.report['journal_buffer']['bytes'], 0)
                self.assertTrue(self.report['journal_inspection_errors'])
                self.assertEqual(runner.backend.signals, ['terminate'])

    def test_strict_load_uses_pinned_selected_save_without_rewriting(self):
        selected = self.save()
        before = selected.read_bytes()
        self.assertEqual(self.invoke(load=selected), 0)
        self.assertEqual(selected.read_bytes(), before)
        command = self.runner.command
        pinned = Path(command[command.index('--load') + 1])
        self.assertNotEqual(pinned, selected)
        self.assertEqual(pinned.read_bytes(), before)
        self.assertFalse(any('inferred' in item or 'preview' in item or 'policy' in item for item in command))

    def test_non_strict_or_missing_routing_rejected_before_child_start(self):
        for routing in ('Inferred', 'strict', None, True):
            selected = self.save(routing)
            self.assertEqual(self.invoke(load=selected), 1)
            self.assertIsNone(self.runner.child)
            self.assertIn('session.options.routing', self.report['primary_failure'])
            self.assertEqual(self.report['actual_child_returncode'], None)
        selected = self.save()
        value = json.loads(selected.read_bytes())
        del value['session']['options']
        selected.write_text(json.dumps(value))
        self.assertEqual(self.invoke(load=selected), 1)

    def test_continue_still_requires_successful_save_and_source_carry(self):
        first = self.prepare()
        first.backend.write_save = True
        self.assertEqual(self.invoke(first), 0)
        pointer = self.cache / 'alpha-sessions/last-session.json'
        before = pointer.read_bytes()
        self.assertEqual(self.invoke(resume=True), 0)
        self.assertEqual(pointer.read_bytes(), before)
        expected = json.loads(before)
        self.assertEqual(self.launch_report['continue_identity'],
                         {'sha256': expected['sha256'], 'source': expected['source']})

    def test_non_strict_continue_rejected_without_publishing(self):
        first = self.prepare()
        first.backend.write_save = True
        self.assertEqual(self.invoke(first), 0)
        pointer = self.cache / 'alpha-sessions/last-session.json'
        value = json.loads(pointer.read_bytes())
        selected = Path(value['save'])
        save = json.loads(selected.read_bytes())
        save['session']['options']['routing'] = 'Inferred'
        selected.write_text(json.dumps(save))
        value['sha256'] = play_alpha.digest(selected)
        pointer.write_text(json.dumps(value))
        before = pointer.read_bytes()
        self.assertEqual(self.invoke(resume=True), 1)
        self.assertIsNone(self.runner.child)
        self.assertEqual(pointer.read_bytes(), before)

    def test_input_mutation_after_child_preserves_zero_outcome_not_continue(self):
        runner = self.prepare()
        runner.backend.write_save = True
        runner.backend.after_close = lambda: self.binary.write_bytes(b'mutated fake executable')
        self.assertEqual(self.invoke(runner), 1)
        self.assertEqual(self.report['actual_child_returncode'], 0)
        self.assertEqual(self.launch_report['process']['returncode'], 0)
        self.assertFalse(self.report['inputs_unchanged'])
        self.assertFalse(self.launch_report['continue_candidate'])
        self.assertFalse((self.cache / 'alpha-sessions/last-session.json').exists())

    def test_plan_mutation_before_launch_fails_without_child(self):
        runner = self.prepare()
        self.plan_file.write_text('{}')
        self.assertEqual(self.invoke(runner), 1)
        self.assertIsNone(runner.child)
        self.assertFalse(self.report['inputs_unchanged'])

    def test_child_signal_outcome_retained_even_with_integrity_failure(self):
        runner = self.prepare()
        runner.backend.start_code = -11
        runner.backend.after_start = lambda: self.helper.unlink()
        self.assertEqual(self.invoke(runner), -11)
        self.assertEqual(self.report['signal'], 11)
        self.assertEqual(self.launch_report['exit_code'], -11)
        self.assertFalse(self.report['inputs_unchanged'])
        self.assertEqual(runner.backend.calls, [])
        self.assertEqual(runner.backend.signals, [])

    def test_timeout_and_partial_native_logs_preserve_primary_and_child(self):
        runner = self.prepare()
        runner.backend.key_error = subprocess.TimeoutExpired('mock-helper', 2, output='partial out', stderr='partial err')
        self.assertEqual(self.invoke(runner), 124)
        self.assertEqual(self.report['status'], 'timeout')
        self.assertEqual(self.report['actual_child_returncode'], 0)
        failed = next(item for item in self.report['native_commands'] if 'partial_stdout' in item)
        self.assertEqual(failed['partial_stdout'], 'partial out')
        self.assertEqual(failed['partial_stderr'], 'partial err')
        self.assertFalse(self.launch_report['continue_candidate'])

    def test_interrupt_and_cleanup_errors_do_not_replace_primary(self):
        runner = self.prepare()
        runner.backend.key_error = KeyboardInterrupt()
        runner.backend.close_ignored = runner.backend.cleanup_fails = True
        self.assertEqual(self.invoke(runner), 130)
        self.assertEqual(self.report['primary_failure'], 'KeyboardInterrupt')
        self.assertEqual(self.report['status'], 'interrupted')
        self.assertEqual(self.report['actual_child_returncode'], -9)
        self.assertEqual(runner.backend.signals, ['terminate', 'kill'])
        self.assertTrue(any(item.get('error') for item in self.report['cleanup']))

    def test_forced_zero_exit_is_not_graceful_or_continue(self):
        runner = self.prepare()
        runner.backend.close_ignored = runner.backend.forced_zero = True
        self.assertEqual(self.invoke(runner), 124)
        self.assertEqual(self.report['actual_child_returncode'], 0)
        self.assertTrue(self.report['forced_teardown'])
        self.assertFalse(self.report['graceful_end'])
        self.assertFalse(self.launch_report['continue_candidate'])

    def test_uncollected_child_outcome_is_not_fabricated(self):
        runner = self.prepare()
        runner.backend.key_error = KeyboardInterrupt()
        runner.backend.close_ignored = runner.backend.cleanup_fails = runner.backend.kill_fails = True
        self.assertEqual(self.invoke(runner), 130)
        self.assertIsNone(self.report['actual_child_returncode'])
        self.assertFalse(self.report['graceful_end'])

    def test_modal_or_pending_never_receives_key_or_auto_dismiss(self):
        runner = self.prepare()
        runner.backend.modal = True
        self.assertEqual(self.invoke(runner), 124)
        operations = [item['operation'] for item in runner.backend.calls]
        self.assertNotIn('key', operations)
        self.assertLessEqual(runner.clock.monotonic(), 10)

    def test_pins_reject_mutation_deletion_hardlink_symlink_and_names(self):
        directory = self.cache / 'pins'
        directory.mkdir()
        pins = driver.Pins(directory)
        target = pins.add('copy', self.helper)
        self.assertTrue(pins.check())
        target.chmod(0o600)
        target.write_bytes(b'changed')
        self.assertFalse(pins.check())
        target.unlink()
        self.assertFalse(pins.check())
        linked = self.root / 'hardlink'
        os.link(self.helper, linked)
        with self.assertRaises(ValueError):
            driver.regular(self.helper)
        linked.unlink()
        linked.symlink_to(self.helper)
        with self.assertRaises(ValueError):
            driver.regular(linked)
        for name in ('../escape', '/escape', '..'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                pins.add(name, self.helper)

    def test_private_outputs_reject_overwrite_traversal_symlink_overlap(self):
        destination = self.cache / 'output'
        driver.private_directory(destination, [self.game])
        for target, protected in ((destination, []), (self.cache / '..' / 'escape', []),
                                  (self.cache / 'nested', [self.cache]),
                                  (self.cache / 'parent', [self.cache / 'parent/input'])):
            with self.subTest(target=target), self.assertRaises(ValueError):
                driver.private_directory(target, protected)
        linked = self.cache / 'linked'
        linked.symlink_to(self.game, target_is_directory=True)
        with self.assertRaises(ValueError):
            driver.private_directory(linked / 'new', [])
        occupied = self.cache / 'existing-file'
        occupied.write_bytes(b'private')
        with self.assertRaises(ValueError):
            driver.private_directory(occupied, [])

    def test_reserve_is_enforced_without_build_or_native_probe(self):
        with mock.patch.object(driver.shutil, 'disk_usage', return_value=mock.Mock(free=driver.RESERVE_BYTES - 1)):
            with self.assertRaisesRegex(ValueError, 'Six GiB'):
                self.prepare()

    def test_relative_installation_root_is_rejected_before_output_reservation(self):
        output = self.cache / 'installed' / 'probe'
        for game_root in (Path('installed'), Path('../installed')):
            with self.subTest(game_root=game_root), \
                    mock.patch.object(driver, 'private_directory') as reservation, \
                    self.assertRaisesRegex(ValueError, 'Absolute installation root'):
                driver.prepare(output, self.binary, game_root, self.helper,
                               self.receipt_file, self.plan_file, self.endpoint_file)
            reservation.assert_not_called()
            self.assertFalse(output.exists())

    def test_absolute_installation_overlap_is_rejected_before_pinning(self):
        installation = self.cache / 'installed'
        installation.mkdir()
        output = installation / 'probe'
        with mock.patch.object(driver.Pins, 'add') as pin, \
                self.assertRaisesRegex(ValueError, 'overlaps a protected input'):
            driver.prepare(output, self.binary, installation, self.helper,
                           self.receipt_file, self.plan_file, self.endpoint_file)
        pin.assert_not_called()
        self.assertFalse(output.exists())

    def test_helper_requires_source_and_explicit_build_receipt(self):
        for field, value in (('source_sha256', '0' * 64), ('executable_sha256', '0' * 64),
                             ('compiler', ''), ('command', ['swiftc', 'unbound']),
                             ('source_path', '/arbitrary/source.swift')):
            changed = dict(self.build)
            changed[field] = value
            self.receipt_file.write_text(json.dumps(changed))
            with self.subTest(field=field), self.assertRaises(ValueError):
                driver.helper_receipt(self.receipt_file, self.helper, driver.SOURCE / 'window_session_macos.swift')

    def test_binding_guard_rejects_pid_reuse_focus_geometry_and_display_changes(self):
        original = binding()
        driver.validate_binding(original, 42, '/private/fake')
        reused = {**original['kernel_identity'], 'start_tvusec': 123457}
        for name, value in (('pid', 43), ('window_id', 72), ('kernel_identity', reused),
                            ('frontmost', False), ('bounds', [11, 10, 800, 625]), ('executable', '/other')):
            changed = copy.deepcopy(original)
            changed[name] = value
            with self.subTest(name=name), self.assertRaises(ValueError):
                driver.validate_binding(changed, 42, '/private/fake', original)

    def test_plan_and_endpoints_reject_scope_expansion_or_wrong_hash(self):
        evidence.validate_specification(self.plan, self.specification, self.specification['plan_sha256'])
        for name, value in (('timeout_seconds', 301), ('timeout_seconds', float('nan')),
                            ('household', 'goth'), ('steps', [])):
            plan = copy.deepcopy(self.plan)
            plan[name] = value
            with self.subTest(name=name), self.assertRaises(ValueError):
                evidence.validate_specification(plan, self.specification, self.specification['plan_sha256'])
        with self.assertRaises(ValueError):
            evidence.validate_specification(self.plan, self.specification, '0' * 64)

    def test_native_post_receipt_requires_binding_release_and_bounded_clocks(self):
        self.assertEqual(self.invoke(), 0)
        post, owner = self.report['native_posts'], self.report['binding']
        self.assertIn('press_duration_us', post)
        for mutate in (
            lambda value: value.update(released=False),
            lambda value: value.update(code=123),
            lambda value: value.update(operation='click'),
            lambda value: value['binding'].update(pid=43),
            lambda value: value['posts'][0].update(unix_after_ns=0),
            lambda value: value['posts'][1].update(host_before_ns=0),
            lambda value: value['posts'][1].update(host_after_ns=value['posts'][0]['host_before_ns'] + 2_000_000_000),
        ):
            changed = copy.deepcopy(post)
            mutate(changed)
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                driver.validate_post(changed, owner)

    def test_journal_links_replacement_and_truncation_rejected(self):
        self.assertEqual(self.invoke(), 0)
        runner = self.runner
        path = runner.telemetry / 'events.jsonl'
        original = path.read_bytes()
        replacement = path.with_name('replacement')
        replacement.write_bytes(original)
        replacement.replace(path)
        with self.assertRaisesRegex(ValueError, 'identity changed'):
            runner.read_records()
        runner.journal_identity = None
        path.write_bytes(b'{}\n')
        with self.assertRaisesRegex(ValueError, 'truncated'):
            runner.read_records()
        path.unlink()
        path.symlink_to(self.plan_file)
        with self.assertRaises(ValueError):
            runner.read_records()

    def test_write_new_never_overwrites_existing_private_artifact(self):
        path = self.cache / 'result.json'
        driver.write_new(path, {'private': True})
        before = path.read_bytes()
        with self.assertRaises(FileExistsError):
            driver.write_new(path, {})
        self.assertEqual(path.read_bytes(), before)

    def test_report_failure_preserves_primary_and_suppresses_continue(self):
        for timed_out in (False, True):
            runner = self.prepare()
            runner.backend.write_save = True
            if timed_out:
                runner.backend.key_error = subprocess.TimeoutExpired('mock-helper', 2)
            with mock.patch.object(driver, 'write_new', side_effect=OSError('mock report write failed')), \
                    contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                code = play_alpha.launch(self.binary, self.game, timeout=10,
                    window_session_output=runner.directory / 'telemetry', child_runner=runner)
            self.assertEqual(code, 124 if timed_out else 1)
            receipts = [json.loads(path.read_bytes()) for path in
                        (self.cache / 'alpha-sessions').glob('household-*/launch.json')]
            receipt = next(item for item in receipts if item.get('window_session_output') ==
                           str(runner.directory / 'telemetry'))
            self.assertEqual(receipt['diagnostic']['status'], 'timeout' if timed_out else 'diagnostic_failed')
            self.assertEqual(receipt['process']['returncode'], 0)
            self.assertFalse(receipt['continue_candidate'])
            self.assertIsNone(receipt['diagnostic']['control_effect'])

    def test_malformed_missing_duplicate_post_end_or_error_journal_cannot_pass(self):
        def missing(backend):
            backend.items.pop()
            backend.flush()
        def duplicate(backend):
            backend.items.append(copy.deepcopy(backend.items[-1]))
            backend.flush()
        def post_end(backend):
            backend.items.append(record('span_end', backend.elapsed(), {}))
            backend.flush()
        def error(backend):
            backend.items.insert(-1, record('application_error', backend.elapsed(), {'error': 'mock'}))
            backend.flush()
        def partial(backend):
            backend.path.write_bytes(backend.path.read_bytes()[:-1])
        for mutate in (missing, duplicate, post_end, error, partial):
            runner = self.prepare()
            runner.backend.after_close = lambda mutate=mutate: mutate(runner.backend)
            with self.subTest(mutate=mutate.__name__):
                self.assertEqual(self.invoke(runner), 1)
                self.assertEqual(self.report['actual_child_returncode'], 0)
                self.assertFalse(self.report['graceful_end'])
                self.assertIsNone(self.report['control_effect'])
                self.assertFalse(self.launch_report['continue_candidate'])


class TutorialBackend(FakeBackend):
    def __init__(self, clock):
        super().__init__(clock)
        self.answered = False
        self.suppress_acknowledgment = False
        self.mutate_frame = lambda value: None

    def add_frame(self, center=None, fence=1):
        from test_window_evidence import opening_answer, opening_dialog
        self.counter += 1
        item = frame(self.counter, self.elapsed(), sequence=self.sequence, fence=fence,
                     modal=not self.answered,
                     dialog=None if self.answered else opening_dialog(),
                     dialog_answer=opening_answer() if self.answered else None)
        item['data']['state']['state_id'] = self.counter + 100
        self.mutate_frame(item['data']['state']['value'])
        self.items.append(item)
        self.flush()

    def helper(self, executable, request, timeout):
        operation = request['operation']
        if operation != 'key':
            result = super().helper(executable, request, timeout)
            if operation == 'release' and result.returncode == 0:
                response_value = json.loads(result.stdout)
                response_value.update({'code': request.get('code', 124), 'released': True})
                return subprocess.CompletedProcess([], 0, json.dumps(response_value), '')
            return result
        self.calls.append(copy.deepcopy(request))
        if self.key_error:
            raise self.key_error
        if request.get('code') != 36 or request.get('press_ms') != 20:
            raise AssertionError('Authored tutorial backend accepts only Enter/20ms')
        current = binding()
        current['executable'] = self.command[0]
        current['kernel_identity']['executable'] = self.command[0]
        started = self.clock.time_ns()
        for receipt in evidence.expected_receipts(evidence.TUTORIAL_ACTION):
            self.clock.now += .001 if self.sequence == 0 else .019
            self.sequence += 1
            self.items.append(record('input', self.elapsed(), {'input_sequence': self.sequence,
                'fence': 1, 'received_unix_ns': self.clock.time_ns(), 'input': receipt}))
            if self.sequence == 1 and not self.suppress_acknowledgment:
                from test_window_evidence import opening_answer
                self.items.append(record('dialog_answer', self.elapsed(), opening_answer()))
        self.answered = not self.suppress_acknowledgment
        self.flush()
        finished = self.clock.time_ns()
        response_value = {'operation': operation, 'binding': current, 'code': 36,
                          'press_ms': 20, 'released': True, 'posts': [
            {'down': True, 'unix_before_ns': started, 'unix_after_ns': started + 100,
             'host_before_ns': started, 'host_after_ns': started + 100},
            {'down': False, 'unix_before_ns': finished, 'unix_after_ns': finished + 100,
             'host_before_ns': finished, 'host_after_ns': finished + 100}]}
        return subprocess.CompletedProcess([], 0, json.dumps(response_value), '')


class TutorialDriverTests(unittest.TestCase):
    setUp = DriverTests.setUp
    invoke = DriverTests.invoke
    save = DriverTests.save

    def prepare(self):
        self.plan = {'schema': 2, 'mode': evidence.TUTORIAL_MODE, 'timeout_seconds': 10,
                     'household': 'newbie', 'steps': [{'name': 'tutorial-opening-enter',
                                                     'input': dict(evidence.TUTORIAL_ACTION)}]}
        self.plan_file.write_text(json.dumps(self.plan))
        self.specification = {'schema': 2, 'plan_sha256': play_alpha.digest(self.plan_file),
                              'controls': evidence.TUTORIAL_CONTROLS}
        self.endpoint_file.write_text(json.dumps(self.specification))
        runner = DriverTests.prepare(self)
        runner.backend = TutorialBackend(runner.clock)
        return runner

    def test_explicit_tutorial_mode_posts_one_enter_and_no_camera_or_play(self):
        self.assertEqual(self.invoke(), 0)
        self.assertEqual(self.report['diagnostic_mode'], evidence.TUTORIAL_MODE)
        self.assertIsNone(self.report['control_effect'])
        self.assertTrue(self.report['offered_response_acknowledgment']['complete'])
        self.assertEqual([call['operation'] for call in self.runner.backend.calls],
                         ['observe', 'observe', 'key', 'observe', 'close'])
        keys = [call for call in self.runner.backend.calls if call['operation'] == 'key']
        self.assertEqual(len(keys), 1)
        self.assertEqual((keys[0]['code'], keys[0]['press_ms']), (36, 20))
        self.assertFalse(any(self.report[name] for name in evidence.QUALIFICATIONS))
        self.assertFalse(self.report['first_tick_claimed'])
        self.assertFalse(self.report['tutorial_completion_claimed'])
        self.assertEqual(self.report['journal_buffer']['bytes'], self.report['journal_bytes'])
        self.assertIsNone(self.report['journal_on_disk']['sha256'])

    def test_camera_mode_still_refuses_the_actually_offered_modal(self):
        runner = DriverTests.prepare(self)
        runner.backend = TutorialBackend(runner.clock)
        self.assertEqual(DriverTests.invoke(self, runner), 124)
        self.assertFalse(any(call['operation'] == 'key' for call in runner.backend.calls))
        self.assertIsNone(self.report['offered_response_acknowledgment'])

    def test_required_native_observation_absent_is_unsupported_before_post(self):
        for field in ('dialog', 'dialog_answer'):
            runner = self.prepare()
            runner.backend.mutate_frame = lambda value, field=field: value.pop(field)
            with self.subTest(field=field):
                self.assertEqual(self.invoke(runner), 1)
                self.assertIn('Unsupported', self.report['primary_failure'])
                self.assertFalse(any(call['operation'] == 'key' for call in runner.backend.calls))

    def test_no_unoffered_unrelated_or_stale_modal_is_answered(self):
        for change in ({'status': 2}, {'enter_result': None}, {'offered_results': [2, 3]},
                       {'source': 'Other.iff'}):
            runner = self.prepare()
            runner.backend.mutate_frame = lambda value, change=change: value['dialog'].update(change)
            with self.subTest(change=change):
                self.assertEqual(self.invoke(runner), 1)
                self.assertFalse(any(call['operation'] == 'key' for call in runner.backend.calls))

    def test_reserved_identity_is_rechecked_on_the_immediately_preceding_draw(self):
        runner = self.prepare()
        original = runner.backend.helper
        def changed(executable, request, timeout):
            result = original(executable, request, timeout)
            if request['operation'] == 'observe' and len(runner.backend.calls) == 2:
                runner.backend.mutate_frame = lambda value: value['dialog'].update({'id': 8})
                runner.backend.add_frame()
            return result
        runner.backend.helper = changed
        self.assertEqual(self.invoke(runner), 1)
        self.assertIn('immediately before', self.report['primary_failure'])
        self.assertFalse(any(call['operation'] == 'key' for call in runner.backend.calls))

    def test_tutorial_submit_is_reserved_once_even_after_helper_timeout(self):
        runner = self.prepare()
        runner.backend.key_error = subprocess.TimeoutExpired('authored-mock', 2)
        self.assertEqual(self.invoke(runner), 124)
        keys = [call for call in runner.backend.calls if call['operation'] == 'key']
        self.assertEqual(len(keys), 1)
        self.assertTrue(runner.key_attempted)
        self.assertIsNotNone(self.report['reserved_opening'])
        self.assertIsNone(self.report['offered_response_acknowledgment'])

    def test_input_or_answer_after_the_latest_draw_prevents_post(self):
        from test_window_evidence import opening_answer
        for kind in ('input', 'dialog_answer'):
            runner = self.prepare()
            original = runner.backend.helper
            def changed(executable, request, timeout, kind=kind):
                result = original(executable, request, timeout)
                if request['operation'] == 'observe' and len(runner.backend.calls) == 2:
                    data = opening_answer() if kind == 'dialog_answer' else {
                        'fence': 1, 'input_sequence': 1, 'received_unix_ns': runner.clock.time_ns(), 'input': None}
                    runner.backend.items.append(record(kind, runner.backend.elapsed(), data))
                    runner.backend.flush()
                return result
            runner.backend.helper = changed
            with self.subTest(kind=kind):
                self.assertEqual(self.invoke(runner), 1)
                self.assertIn('immediately preceding draw', self.report['primary_failure'])
                self.assertFalse(any(call['operation'] == 'key' for call in runner.backend.calls))

    def test_tick_change_on_the_fresh_pre_post_draw_prevents_enter(self):
        runner = self.prepare()
        original = runner.backend.helper
        def changed(executable, request, timeout):
            result = original(executable, request, timeout)
            if request['operation'] == 'observe' and len(runner.backend.calls) == 2:
                runner.backend.mutate_frame = lambda value: value.update({'tick': 192924})
                runner.backend.add_frame()
            return result
        runner.backend.helper = changed
        self.assertEqual(self.invoke(runner), 1)
        self.assertIn('tick change', self.report['primary_failure'])
        self.assertFalse(any(call['operation'] == 'key' for call in runner.backend.calls))

    def test_changed_telemetry_geometry_on_the_fresh_draw_prevents_enter(self):
        runner = self.prepare()
        original = runner.backend.helper
        def changed(executable, request, timeout):
            result = original(executable, request, timeout)
            if request['operation'] == 'observe' and len(runner.backend.calls) == 2:
                runner.backend.mutate_frame = lambda value: value.update({'display_size': [8011, 600]})
                runner.backend.add_frame()
            return result
        runner.backend.helper = changed
        self.assertEqual(self.invoke(runner), 1)
        self.assertIn('geometry', self.report['primary_failure'])
        self.assertFalse(any(call['operation'] == 'key' for call in runner.backend.calls))

    def test_separate_cleanup_helper_releases_enter_not_arrowright(self):
        runner = self.prepare()
        runner.backend.key_error = OSError('authored partial Enter post')
        self.assertEqual(self.invoke(runner), 1)
        release = [call for call in runner.backend.calls if call['operation'] == 'release']
        self.assertEqual(len(release), 1)
        self.assertEqual(release[0]['code'], 36)
        self.assertTrue(all(call.get('code') != 124 for call in runner.backend.calls))
        self.assertEqual(self.report['primary_failure'], 'authored partial Enter post')

    def test_wrong_cleanup_release_receipt_cannot_hide_primary_failure(self):
        runner = self.prepare()
        runner.backend.key_error = OSError('authored primary')
        original = runner.backend.helper
        def changed(executable, request, timeout):
            result = original(executable, request, timeout)
            if request['operation'] == 'release':
                value = json.loads(result.stdout)
                value['code'] = 124
                return subprocess.CompletedProcess([], 0, json.dumps(value), '')
            return result
        runner.backend.helper = changed
        self.assertEqual(self.invoke(runner), 1)
        self.assertEqual(self.report['primary_failure'], 'authored primary')
        self.assertIn('match Enter', self.report['cleanup'][0]['error'])

    def test_enter_submission_without_acknowledgment_times_out_without_resubmission(self):
        runner = self.prepare()
        runner.backend.suppress_acknowledgment = True
        self.assertEqual(self.invoke(runner), 124)
        self.assertEqual(len([call for call in runner.backend.calls if call['operation'] == 'key']), 1)
        self.assertIsNone(self.report['offered_response_acknowledgment'])

    def test_key_attempt_cannot_be_repeated_even_with_an_owned_mock_child(self):
        runner = self.prepare()
        self.assertEqual(self.invoke(runner), 0)
        runner.child.returncode = None
        calls = len(runner.backend.calls)
        with self.assertRaisesRegex(ValueError, 'never be resubmitted'):
            runner.native('key')
        self.assertEqual(len(runner.backend.calls), calls)
        runner.child.returncode = 0

    def test_changed_owned_kernel_binding_prevents_enter(self):
        runner = self.prepare()
        original = runner.backend.helper
        def changed(executable, request, timeout):
            result = original(executable, request, timeout)
            if request['operation'] == 'observe' and len(runner.backend.calls) == 2:
                value = json.loads(result.stdout)
                value['binding']['kernel_identity']['start_tvusec'] += 1
                return subprocess.CompletedProcess([], 0, json.dumps(value), '')
            return result
        runner.backend.helper = changed
        self.assertEqual(self.invoke(runner), 1)
        self.assertEqual(self.report['actual_child_returncode'], 0)
        self.assertFalse(any(call['operation'] == 'key' for call in runner.backend.calls))
        self.assertEqual(self.report['binding']['kernel_identity']['start_tvusec'], 123456)

    def test_tutorial_keeps_the_unchanged_unbound_sixty_four_mib_cap(self):
        runner = self.prepare()
        runner.backend.pending_observations = 1000
        original = runner.backend.helper
        def growing(executable, request, timeout):
            result = original(executable, request, timeout)
            with runner.backend.path.open('r+b') as stream:
                stream.seek(evidence.MAX_JOURNAL_BYTES)
                stream.write(b'\n')
            return result
        runner.backend.helper = growing
        with mock.patch.object(evidence, 'journal', wraps=evidence.journal) as parse:
            self.assertEqual(self.invoke(runner), -15)
        parse.assert_not_called()
        self.assertEqual(evidence.MAX_JOURNAL_BYTES, 64 * 1024 * 1024)
        self.assertEqual(self.report['primary_failure'], 'Unsafe or oversized journal')
        self.assertIsNone(self.report['binding'])
        self.assertEqual([call['operation'] for call in runner.backend.calls], ['observe'])

    def test_first_next_blocker_or_modal_stops_without_further_input(self):
        for change in ({'pending': True}, {'clock_pending': True}, {'playing': True},
                       {'tick': 192924}, {'modal': True}, {'error': 'authored next error'}):
            runner = self.prepare()
            def changed(value, change=change):
                if value['dialog_answer'] is not None:
                    value.update(change)
            runner.backend.mutate_frame = changed
            with self.subTest(change=change):
                self.assertEqual(self.invoke(runner), 1)
                self.assertEqual(len([call for call in runner.backend.calls if call['operation'] == 'key']), 1)
                self.assertIsNone(self.report['offered_response_acknowledgment'])

    def test_status_five_is_not_fabricated_result_two_acknowledgment(self):
        runner = self.prepare()
        def changed(value):
            if value['dialog_answer'] is not None:
                value['dialog_answer']['status'] = 5
        runner.backend.mutate_frame = changed
        self.assertEqual(self.invoke(runner), 1)
        self.assertIn('dialog_answer', self.report['primary_failure'])

    def test_post_receipt_must_match_enter_and_strict_scalar_types(self):
        for change in ({'code': 124}, {'code': 36.0}, {'press_ms': 20.0}, {'released': 1}):
            runner = self.prepare()
            original = runner.backend.helper
            def changed(executable, request, timeout, change=change):
                result = original(executable, request, timeout)
                if request['operation'] == 'key':
                    value = json.loads(result.stdout)
                    value.update(change)
                    return subprocess.CompletedProcess([], 0, json.dumps(value), '')
                return result
            runner.backend.helper = changed
            with self.subTest(change=change):
                self.assertEqual(self.invoke(runner), 1)
                self.assertIsNone(self.report['offered_response_acknowledgment'])

    def test_tutorial_keeps_strict_pinned_selected_save_rules(self):
        selected = self.save(routing='Synthetic')
        self.assertEqual(self.invoke(load=selected), 1)
        self.assertEqual(self.runner.backend.calls, [])

    def test_tutorial_unbound_wait_is_stat_only_and_never_posts(self):
        runner = self.prepare()
        runner.backend.pending_observations = 100
        with mock.patch.object(evidence, 'journal', wraps=evidence.journal) as parse:
            self.assertEqual(self.invoke(runner), 124)
        self.assertEqual(parse.call_count, 0)
        self.assertFalse(self.report["journal_final_observation"]["complete"])
        self.assertEqual(self.report['actual_child_returncode'], -15)
        self.assertEqual(self.report['journal_buffer']['bytes'], len(runner.payload))
        self.assertIsNone(self.report['binding'])
        self.assertIsNotNone(self.report['kernel_identity'])
        self.assertFalse(any(call['operation'] != 'observe' for call in runner.backend.calls))
        self.assertGreater(self.report['journal_on_disk']['stat']['bytes'], 0)
        self.assertIsNone(self.report['journal_on_disk']['sha256'])

    def test_cleanup_failure_retains_primary_and_exact_owned_child_signals(self):
        runner = self.prepare()
        runner.backend.key_error = subprocess.TimeoutExpired('authored-helper', 2)
        runner.backend.cleanup_fails = True
        runner.backend.close_ignored = True
        self.assertEqual(self.invoke(runner), 124)
        self.assertEqual(self.report['status'], 'timeout')
        self.assertEqual(runner.backend.signals, ['terminate', 'kill'])
        self.assertEqual(self.report['actual_child_returncode'], -9)
        self.assertTrue(self.report['forced_teardown'])
        self.assertIsNone(self.report['offered_response_acknowledgment'])

    def test_final_read_failure_suppresses_acknowledgment_but_retains_partial(self):
        runner = self.prepare()
        runner.backend.after_close = lambda: runner.backend.path.write_bytes(runner.backend.path.read_bytes()[:-1])
        self.assertEqual(self.invoke(runner), 1)
        self.assertIsNone(self.report['offered_response_acknowledgment'])
        self.assertIsNotNone(self.report['partial_offered_response_diagnostic'])
        self.assertTrue(self.report['journal_read_errors'])
        self.assertTrue(self.report['post_errors'])
        self.assertFalse(self.launch_report['continue_candidate'])

    def test_report_write_failure_never_exposes_response_success(self):
        runner = self.prepare()
        with mock.patch.object(driver, 'write_new', side_effect=OSError('authored report failure')), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            code = play_alpha.launch(self.binary, self.game, timeout=10,
                window_session_output=runner.directory / 'telemetry', child_runner=runner)
        self.assertEqual(code, 1)
        receipts = [json.loads(path.read_bytes()) for path in
                    (self.cache / 'alpha-sessions').glob('household-*/launch.json')]
        diagnostic = receipts[-1]['diagnostic']
        self.assertIsNone(diagnostic['offered_response_acknowledgment'])
        self.assertIsNotNone(diagnostic['partial_offered_response_diagnostic'])
        self.assertFalse(receipts[-1]['continue_candidate'])

    def test_helper_source_keeps_direct_quartz_brackets_and_matching_key_release(self):
        source = (driver.SOURCE / 'window_session_macos.swift').read_text()
        self.assertIn('let processPathCapacity = 4 * Int(MAXPATHLEN)', source)
        self.assertNotIn('PROC_PIDPATHINFO_MAXSIZE', source)
        self.assertIn('virtualKey: CGKeyCode(code), keyDown: true', source)
        self.assertIn('virtualKey: CGKeyCode(code), keyDown: false', source)
        self.assertIn('operation == "release" && request["code"] != nil', source)
        self.assertIn('code == 124 || code == 36', source)
        self.assertIn('let quartzBefore = CACurrentMediaTime()\n        event.postToPid(pid)\n'
                      '        let quartzAfter = CACurrentMediaTime()', source)
        for forbidden in ('CGRequestPostEventAccess', 'AXIsProcessTrustedWithOptions', '.activate('):
            self.assertNotIn(forbidden, source)


class ReservationHistoryDriverTests(unittest.TestCase):
    setUp = TutorialDriverTests.setUp
    prepare = TutorialDriverTests.prepare
    invoke = TutorialDriverTests.invoke

    def test_late_valid_pre_command_draw_before_press_latches_baseline_mismatch(self):
        self.baseline_mismatch()

    def test_pre_command_baseline_mismatch_remains_latched_through_cleanup_errors(self):
        self.baseline_mismatch(cleanup_failure=True)

    def baseline_mismatch(self, cleanup_failure=False):
        runner = self.prepare()
        runner.backend.close_ignored = runner.backend.cleanup_fails = cleanup_failure
        original = runner.backend.helper
        def changed(executable, request, timeout):
            if request['operation'] == 'key':
                runner.backend.add_frame()
                runner.backend.items[-1]['data']['draw_started_unix_ns'] = (
                    runner.reservation['pre_post_frame']['draw_started_unix_ns'] + 1)
                runner.backend.flush()
            return original(executable, request, timeout)
        runner.backend.helper = changed
        self.assertEqual(self.invoke(runner), -9 if cleanup_failure else 1)
        self.assertIn('not the immediately preceding observed draw', self.report['primary_failure'])
        self.assertEqual(self.report['reservation_history_failure'], self.report['primary_failure'])
        self.assertIsNone(self.report['offered_response_acknowledgment'])
        self.assertTrue(any(item['operation'] == 'release' for item in self.report['cleanup']))
        self.assertEqual(len([call for call in runner.backend.calls if call['operation'] == 'key']), 1)
        if cleanup_failure:
            self.assertTrue(any(item.get('error') for item in self.report['cleanup']))

    def late_replay(self, cleanup_failure=False):
        runner = self.prepare()
        runner.backend.close_ignored = runner.backend.cleanup_fails = cleanup_failure
        original = runner.backend.helper
        def changed(executable, request, timeout):
            result = original(executable, request, timeout)
            if request['operation'] == 'key':
                runner.backend.add_frame()
                runner.backend.items[-1]['data']['draw_started_unix_ns'] = (
                    runner.reservation['pre_post_frame']['draw_started_unix_ns'] + 1)
                runner.backend.flush()
            return result
        runner.backend.helper = changed
        self.assertEqual(self.invoke(runner), 1 if not cleanup_failure else -9)
        self.assertIn('Late pre-command observation', self.report['primary_failure'])
        self.assertEqual(self.report['reservation_history_failure'], self.report['primary_failure'])
        self.assertIsNone(self.report['offered_response_acknowledgment'])
        self.assertEqual(len([call for call in runner.backend.calls if call['operation'] == 'key']), 1)
        self.assertTrue(any(item['operation'] == 'release' for item in self.report['cleanup']))
        if cleanup_failure:
            self.assertEqual(runner.backend.signals, ['terminate', 'kill'])
            self.assertTrue(any(item.get('error') for item in self.report['cleanup']))
        return runner

    def test_late_history_failure_during_acknowledgment_is_latched(self):
        self.late_replay()

    def test_late_history_failure_survives_owned_cleanup_failures(self):
        self.late_replay(cleanup_failure=True)

    def test_final_acknowledgment_history_failure_is_latched(self):
        runner = self.prepare()
        original = evidence.offered_history
        reason = 'Opening reservation history blocker: authored final replay rejection'
        def checked(records, reservation, fence, started_unix_ns=None):
            if runner.child is not None and runner.child.returncode is not None:
                raise ValueError(reason)
            return original(records, reservation, fence, started_unix_ns)
        with mock.patch.object(evidence, 'offered_history', side_effect=checked):
            self.assertEqual(self.invoke(runner), 1)
        self.assertEqual(self.report['primary_failure'], reason)
        self.assertEqual(self.report['reservation_history_failure'], reason)
        self.assertIsNone(self.report['offered_response_acknowledgment'])
        self.assertIsNotNone(self.report['partial_offered_response_diagnostic'])
        self.assertEqual(self.report['actual_child_returncode'], 0)

    def test_nonhistory_acknowledgment_failure_does_not_claim_history_rejection(self):
        runner = self.prepare()
        reason = 'Authored acknowledgment failure outside reservation history'
        with mock.patch.object(evidence, 'offered_response', side_effect=ValueError(reason)):
            self.assertEqual(self.invoke(runner), 1)
        self.assertEqual(self.report['primary_failure'], reason)
        self.assertIsNone(self.report['reservation_history_failure'])
        self.assertIsNone(self.report['offered_response_acknowledgment'])

    def test_first_history_failure_is_not_replaced_by_later_validation(self):
        runner = self.prepare()
        with mock.patch.object(evidence, 'offered_history',
                               side_effect=[ValueError('first history reason'), ValueError('later reason')]):
            with self.assertRaisesRegex(ValueError, 'first history reason'):
                runner.check_opening_history([])
            with self.assertRaisesRegex(ValueError, 'later reason'):
                runner.check_opening_history([])
        self.assertEqual(runner.reservation_history_failure, 'first history reason')

    def reject_recovery(self, name):
        from test_window_evidence import history_blocker
        runner = self.prepare()
        original = runner.backend.helper
        def changed(executable, request, timeout):
            result = original(executable, request, timeout)
            if request['operation'] == 'observe' and len(runner.backend.calls) == 2:
                runner.backend.mutate_frame = lambda value: history_blocker(value, name)
                runner.backend.add_frame()
                runner.backend.mutate_frame = lambda value: None
                runner.backend.add_frame()
            return result
        runner.backend.helper = changed
        self.assertEqual(self.invoke(runner), 1)
        self.assertFalse(any(call['operation'] == 'key' for call in runner.backend.calls))
        self.assertIsNotNone(self.report['reservation_history_failure'])
        self.assertEqual(self.report['primary_failure'], self.report['reservation_history_failure'])
        self.assertIsNone(self.report['offered_response_acknowledgment'])
        self.assertIsNone(self.report['native_posts'])
        self.assertNotIn('pre_post_frame', self.report['reserved_opening'])
        self.assertFalse(self.launch_report['continue_candidate'])

    def test_pending_then_recovered_draw_posts_zero_keys(self):
        self.reject_recovery('pending')

    def test_playing_then_recovered_draw_posts_zero_keys(self):
        self.reject_recovery('playing')

    def test_changed_modal_then_recovered_draw_posts_zero_keys(self):
        self.reject_recovery('changed-modal')

    def test_tick_then_recovered_draw_posts_zero_keys(self):
        self.reject_recovery('tick')

    def test_geometry_then_recovered_draw_posts_zero_keys(self):
        self.reject_recovery('geometry')

    def test_other_pending_owners_then_recovered_draws_post_zero_keys(self):
        for name in ('clock_pending', 'presentation_children_pending'):
            with self.subTest(name=name):
                self.reject_recovery(name)

    def test_valid_pre_post_draw_does_not_rebase_original_reservation(self):
        runner = self.prepare()
        original = runner.backend.helper
        def changed(executable, request, timeout):
            result = original(executable, request, timeout)
            if request['operation'] == 'observe' and len(runner.backend.calls) == 2:
                runner.backend.add_frame()
            return result
        runner.backend.helper = changed
        self.assertEqual(self.invoke(runner), 0)
        reservation = self.report['reserved_opening']
        self.assertLess(reservation['frame']['frame_id'], reservation['pre_post_frame']['frame_id'])
        self.assertIsNone(self.report['reservation_history_failure'])
        self.assertEqual(len([call for call in runner.backend.calls if call['operation'] == 'key']), 1)


class CameraExportDriverTests(unittest.TestCase):
    """Authored asset-free guarded mocks; UNRUN in the source-only candidate."""

    invoke = DriverTests.invoke

    def prepare(self):
        runner = DriverTests.prepare(self)

        def timestamp():
            runner.clock.reads += 1
            return BASE + round(runner.clock.now * 1_000_000_000) + runner.clock.reads * 1000

        runner.clock.time_ns = timestamp
        return runner

    def setUp(self):
        DriverTests.setUp(self)
        for name in ('start', 'helper'):
            guard = mock.patch.object(driver.NativeBackend, name,
                side_effect=AssertionError('Native backend forbidden in export mocks'))
            guard.start()
            self.addCleanup(guard.stop)

    def assert_no_export(self):
        self.assertEqual(self.report['control_export_status'], 'failed')
        self.assertEqual(self.report['exported_controls'], [])
        self.assertEqual(self.report['control_native_associations'], [])
        self.assertFalse(self.report['control_export_qualified'])
        self.assertFalse(self.report['control_export_integration_reviewed'])
        self.assertFalse(self.report['control_export_driver_enabled'])

    def add_late_frame(self, runner, **changes):
        backend = runner.backend
        backend.counter += 1
        late = frame(backend.counter, backend.elapsed(), [1, 0], sequence=2, **changes)
        late['data']['state']['state_id'] = backend.counter + 2
        backend.items.insert(-1, late)
        backend.flush()
        return late

    def test_actual_enclosing_times_and_indices_include_readiness_without_new_calls(self):
        runner = self.prepare()
        runner.backend.pending_observations = 2
        events = []
        native, time_ns = runner.native, runner.clock.time_ns

        def timestamp():
            value = time_ns()
            events.append(('time', value))
            return value

        def observed(operation, cleanup=False):
            events.append(('enter', operation, len(runner.native_log)))
            result = native(operation, cleanup)
            events.append(('leave', operation))
            return result

        runner.clock.time_ns, runner.native = timestamp, observed
        self.assertEqual(self.invoke(runner), 0)
        control = self.report['exported_controls'][0]
        association = self.report['control_native_associations'][0]
        self.assertEqual(association, {'control_index': 0, 'before_command_index': 3,
                                      'key_command_index': 4, 'after_command_index': 5})
        key_enter = next(index for index, event in enumerate(events) if event[:2] == ('enter', 'key'))
        key_leave = next(index for index, event in enumerate(events) if event[:2] == ('leave', 'key'))
        self.assertEqual(control['started_unix_ns'], events[key_enter - 1][1])
        self.assertEqual(control['finished_unix_ns'], events[key_leave + 1][1])
        commands = self.report['native_commands']
        key = commands[association['key_command_index']]
        for post in control['native_result']['posts']:
            self.assertLessEqual(key['started_unix_ns'], post['unix_before_ns'])
            self.assertLessEqual(post['unix_after_ns'], key['finished_unix_ns'])
        self.assertEqual([item['request']['operation'] for item in commands],
                         ['observe', 'observe', 'observe', 'observe', 'key', 'observe', 'close'])
        self.assertEqual([json.loads(item['stdout']).get('status') for item in commands[:2]],
                         ['not_ready', 'not_ready'])
        self.assertEqual(len(commands), len(runner.backend.calls))
        self.assertEqual(set(control), {'control_effect', 'native_result', 'binding_before', 'binding_after',
            'fence', 'started_unix_ns', 'finished_unix_ns', 'next_started_unix_ns'})
        self.assertEqual(control['fence'], runner.fence)
        self.assertIsNone(control['next_started_unix_ns'])
        self.assertEqual(self.report['close_started_unix_ns'], runner.next_started)
        self.assertLessEqual(control['finished_unix_ns'], self.report['close_started_unix_ns'])
        self.assertEqual(json.loads((runner.directory / 'controls.json').read_bytes()), [control])
        self.assertEqual(self.report['control_export_contract'], driver.CAMERA_EXPORT_CONTRACT)
        self.assertTrue(self.report['control_export_offline_only'])
        self.assertFalse(any(self.report[name] for name in evidence.QUALIFICATIONS))

    def test_returned_binding_and_raw_snapshots_are_independent_nested_copies(self):
        runner = self.prepare()
        native, returned = runner.native, []

        def observed(operation, cleanup=False):
            result = native(operation, cleanup)
            returned.append((operation, result))
            return result

        runner.native = observed
        self.assertEqual(self.invoke(runner), 0)
        before, after = returned[1][1]['binding'], returned[3][1]['binding']
        capture = runner.camera_capture
        self.assertIsNot(capture['binding_before'], before)
        self.assertIsNot(capture['binding_after'], after)
        self.assertIsNot(capture['binding_before'], capture['binding_after'])
        self.assertIsNot(capture['binding_before']['kernel_identity'], before['kernel_identity'])
        self.assertIsNot(capture['binding_after']['displays'], after['displays'])
        expected = copy.deepcopy(capture)
        before['bounds'][0] += 1
        after['kernel_identity']['start_tvusec'] += 1
        self.assertEqual(capture, expected)
        raw = copy.deepcopy(runner.raw_key_result)
        runner.post['posts'][0]['unix_before_ns'] += 1
        self.assertEqual(runner.raw_key_result, raw)
        self.assertNotIn('press_duration_us', raw)
        self.assertIn('press_duration_us', self.report['native_posts'])

    def test_raw_stdout_export_and_deterministically_enriched_legacy_posts_agree(self):
        self.assertEqual(self.invoke(), 0)
        association = self.report['control_native_associations'][0]
        raw = json.loads(self.report['native_commands'][association['key_command_index']]['stdout'])
        self.assertEqual(raw, self.report['native_posts_raw_v2'])
        self.assertEqual(raw, self.report['exported_controls'][0]['native_result'])
        self.assertEqual(driver.validate_post(copy.deepcopy(raw), raw['binding']), self.report['native_posts'])
        self.assertNotIn('press_duration_us', raw)
        self.assertEqual([item['operation'] for item in self.runner.backend.calls],
                         ['observe', 'observe', 'key', 'observe', 'close'])

    def test_final_whole_window_is_independently_recomputed_and_keeps_legacy_cutoff(self):
        runner = self.prepare()
        later = []
        runner.backend.after_close = lambda: later.append(self.add_late_frame(runner))
        original = evidence.control_effect
        with mock.patch.object(evidence, 'control_effect', wraps=original) as analyzer:
            self.assertEqual(self.invoke(runner), 0)
        full = self.report['exported_controls'][0]['control_effect']
        legacy = self.report['control_effect']
        self.assertEqual(full['closing_frame_id'], later[0]['data']['frame_id'])
        self.assertNotEqual(full['closing_frame_id'], legacy['closing_frame_id'])
        final = analyzer.call_args_list[-1]
        self.assertIs(final.args[0], runner.final_records)
        self.assertIsNot(runner.final_records, runner.records)
        self.assertEqual(final.args[3], {'baseline': runner.camera_capture['baseline']})
        self.assertEqual(final.args[4:7], (runner.camera_capture['fence'],
            runner.camera_capture['started_unix_ns'], runner.camera_capture['finished_unix_ns']))
        self.assertEqual(final.kwargs, {'next_started_unix_ns': None})
        self.assertEqual(runner.records[-1]['kind'], 'session_end')
        self.assertEqual(self.report['journal_bytes'], len(payload(runner.backend.items)))

    def test_late_wrong_fence_or_unusable_draw_has_distinct_export_failure(self):
        for changes in ({'fence': 2}, {'modal': True}):
            runner = self.prepare()
            runner.backend.after_close = lambda changes=changes: self.add_late_frame(runner, **changes)
            with self.subTest(changes=changes):
                self.assertEqual(self.invoke(runner), 1)
                self.assert_no_export()
                self.assertIsNone(self.report['primary_failure'])
                self.assertIn('unusable or wrong-fence draw', self.report['control_export_error'])
                self.assertIsNotNone(self.report['partial_control_effect_diagnostic'])
                self.assertEqual(self.report['actual_child_returncode'], 0)
                self.assertEqual(self.launch_report['process']['returncode'], 0)
                self.assertFalse((runner.directory / 'controls.json').exists())

    def test_changed_complete_recollection_is_rejected_not_repaired_for_export(self):
        runner = self.prepare()
        read_records, completed, later = runner.read_records, [], []

        def recollect(complete=False):
            if complete:
                completed.append(True)
                if len(completed) == 1:
                    later.append(self.add_late_frame(runner))
            return read_records(complete)

        runner.read_records = recollect
        self.assertEqual(self.invoke(runner), 1)
        self.assertEqual(len(completed), 1)
        self.assert_no_export()
        self.assertIsNone(self.report['primary_failure'])
        self.assertTrue(any('replaced its previous bytes' in error for error in self.report['post_errors']))
        self.assertTrue(self.report['journal_read_errors'])
        self.assertEqual(self.report['partial_control_effect_diagnostic'], runner.control)
        self.assertNotEqual(runner.control['closing_frame_id'], later[0]['data']['frame_id'])

    def test_raw_stdout_duration_contradiction_is_not_overwritten_into_export_success(self):
        runner = self.prepare()
        helper = runner.backend.helper

        def contradictory(executable, request, timeout):
            result = helper(executable, request, timeout)
            if request['operation'] == 'key':
                value = json.loads(result.stdout)
                value['press_duration_us'] = {'lower': -1, 'upper': -1, 'clock': 'contradiction'}
                result.stdout = json.dumps(value)
            return result

        runner.backend.helper = contradictory
        self.assertEqual(self.invoke(runner), 1)
        self.assert_no_export()
        self.assertIsNone(self.report['primary_failure'])
        self.assertIn('contradicts deterministic legacy enrichment', self.report['control_export_error'])
        self.assertEqual(self.report['native_posts_raw_v2']['press_duration_us']['lower'], -1)
        self.assertGreaterEqual(self.report['native_posts']['press_duration_us']['lower'], 0)

    def test_unrelated_late_input_is_not_trimmed_or_rebased_for_export(self):
        runner = self.prepare()

        def late_input():
            backend = runner.backend
            backend.items.insert(-1, record('input', backend.elapsed(), {'input_sequence': 3, 'fence': 1,
                'received_unix_ns': runner.clock.time_ns(),
                'input': {'kind': 'key_pressed', 'physical_key': 'Code(ArrowLeft)'}}))
            backend.flush()

        runner.backend.after_close = late_input
        self.assertEqual(self.invoke(runner), 1)
        self.assert_no_export()
        self.assertIsNone(self.report['primary_failure'])
        self.assertIn('unrelated or reordered native receipt', self.report['control_export_error'])
        self.assertEqual(len(runner.records), len(runner.backend.items))

    def test_missing_actual_capture_is_not_reconstructed_from_native_log(self):
        runner = self.prepare()
        runner.backend.after_close = lambda: runner.camera_capture.pop('binding_before')
        self.assertEqual(self.invoke(runner), 1)
        self.assert_no_export()
        self.assertIn('actual before/key/after capture', self.report['control_export_error'])
        self.assertEqual(len(self.report['native_commands']), 5)
        self.assertEqual(self.report['actual_child_returncode'], 0)

    def test_failed_key_retains_partial_output_cleanup_and_never_invents_finished_capture(self):
        runner = self.prepare()
        runner.backend.key_error = subprocess.TimeoutExpired('authored-mock', 2, output='partial raw key',
                                                            stderr='partial key stderr')
        self.assertEqual(self.invoke(runner), 124)
        self.assert_no_export()
        self.assertEqual(self.report['status'], 'timeout')
        self.assertIsNone(self.report['native_posts_raw_v2'])
        self.assertNotIn('finished_unix_ns', self.report['control_capture_diagnostic'])
        self.assertIsNone(self.report['close_started_unix_ns'])
        failed = next(item for item in self.report['native_commands'] if item['request']['operation'] == 'key')
        self.assertEqual(failed['partial_stdout'], 'partial raw key')
        self.assertEqual(failed['partial_stderr'], 'partial key stderr')
        self.assertEqual([item['operation'] for item in runner.backend.calls],
                         ['observe', 'observe', 'key', 'release', 'close'])
        self.assertEqual([item['operation'] for item in self.report['cleanup']], ['release', 'close'])

    def test_partial_posts_remain_raw_diagnostics_not_complete_export_or_retry(self):
        runner = self.prepare()
        helper = runner.backend.helper

        def partial(executable, request, timeout):
            result = helper(executable, request, timeout)
            if request['operation'] == 'key':
                value = json.loads(result.stdout)
                value['posts'].pop()
                result.stdout = json.dumps(value)
            return result

        runner.backend.helper = partial
        self.assertEqual(self.invoke(runner), 1)
        self.assert_no_export()
        self.assertIn('Missing native post timestamps', self.report['primary_failure'])
        self.assertEqual(len(self.report['native_posts_raw_v2']['posts']), 1)
        self.assertNotIn('finished_unix_ns', self.report['control_capture_diagnostic'])
        self.assertEqual(sum(item['operation'] == 'key' for item in runner.backend.calls), 1)
        self.assertEqual([item['request']['operation'] for item in self.report['native_commands']],
                         ['observe', 'observe', 'key', 'release', 'close'])

    def test_before_and_after_observation_failures_leave_only_actual_capture(self):
        for failing_observation in (2, 3):
            runner = self.prepare()
            native, observations = runner.native, []

            def fail_observation(operation, cleanup=False):
                result = native(operation, cleanup)
                if operation == 'observe':
                    observations.append(result)
                    if len(observations) == failing_observation:
                        raise ValueError('authored observation capture failure')
                return result

            runner.native = fail_observation
            with self.subTest(observation=failing_observation):
                self.assertEqual(self.invoke(runner), 1)
                self.assert_no_export()
                capture = self.report['control_capture_diagnostic']
                self.assertNotIn('binding_before' if failing_observation == 2 else 'binding_after', capture)
                self.assertEqual(sum(item['operation'] == 'key' for item in runner.backend.calls),
                                 0 if failing_observation == 2 else 1)
                self.assertIn('authored observation capture failure', self.report['primary_failure'])

    def test_controls_write_failure_is_distinct_and_changes_launcher_diagnostic_code(self):
        runner = self.prepare()
        write = driver.write_new

        def fail_controls(path, value):
            if path.name == 'controls.json':
                raise OSError('authored controls write failure')
            return write(path, value)

        with mock.patch.object(driver, 'write_new', side_effect=fail_controls):
            self.assertEqual(self.invoke(runner), 1)
        self.assert_no_export()
        self.assertEqual(self.report['controls_write_error'], 'authored controls write failure')
        self.assertIsNone(self.report['control_export_error'])
        self.assertIsNone(self.report['primary_failure'])
        self.assertEqual(self.report['actual_child_returncode'], 0)
        self.assertEqual(self.launch_report['exit_code'], 0)
        self.assertEqual(self.launch_report['launcher_exit_code'], 1)
        self.assertIn('press_duration_us', self.report['native_posts'])
        self.assertIsNotNone(self.report['native_posts_raw_v2'])

    def test_controls_and_report_write_failures_preserve_partial_control_and_both_reasons(self):
        runner = self.prepare()
        write, reports = driver.write_new, []

        def fail_outputs(path, value):
            if path.name == 'controls.json':
                raise OSError('authored controls write failure')
            if path.name == 'result.json':
                reports.append(value)
                raise OSError('authored result write failure')
            return write(path, value)

        with mock.patch.object(driver, 'write_new', side_effect=fail_outputs), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            result = play_alpha.launch(self.binary, self.game, timeout=10,
                window_session_output=runner.directory / 'telemetry', child_runner=runner)
        self.assertEqual(result, 1)
        self.assertEqual(len(reports), 1)
        self.report = reports[0]
        self.assertIsNotNone(self.report['partial_control_effect_diagnostic'])
        self.assertEqual(self.report['partial_control_effect_diagnostic'], runner.control)
        self.assertTrue(self.report['partial_control_effect_diagnostic']['complete'])
        self.assert_no_export()
        self.assertEqual(self.report['controls_write_error'], 'authored controls write failure')
        self.assertEqual(self.report['report_error'], 'authored result write failure')
        self.assertIn('Camera controls write: authored controls write failure', self.report['post_errors'])
        self.assertIsNone(self.report['control_export_error'])
        self.assertIsNone(self.report['primary_failure'])
        self.assertIsNone(self.report['control_effect'])
        self.assertEqual(self.report['actual_child_returncode'], 0)
        self.assertEqual(self.report['exit_code'], 1)
        self.assertEqual(self.report['status'], 'diagnostic_failed')
        self.assertFalse(self.report['graceful_end'])
        self.assertFalse((runner.directory / 'controls.json').exists())
        self.assertEqual([item['operation'] for item in runner.backend.calls],
                         ['observe', 'observe', 'key', 'observe', 'close'])

    def test_report_write_failure_keeps_controls_artifact_but_suppresses_all_export_claims(self):
        runner = self.prepare()
        write, reports = driver.write_new, []

        def fail_report(path, value):
            if path.name == 'result.json':
                reports.append(value)
                raise OSError('authored result write failure')
            return write(path, value)

        with mock.patch.object(driver, 'write_new', side_effect=fail_report), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            result = play_alpha.launch(self.binary, self.game, timeout=10,
                window_session_output=runner.directory / 'telemetry', child_runner=runner)
        self.assertEqual(result, 1)
        self.report = reports[0]
        self.assert_no_export()
        self.assertEqual(self.report['report_error'], 'authored result write failure')
        self.assertEqual(self.report['actual_child_returncode'], 0)
        self.assertIsNone(self.report['primary_failure'])
        self.assertTrue((runner.directory / 'controls.json').exists())
        self.assertEqual(len(json.loads((runner.directory / 'controls.json').read_bytes())), 1)
        self.assertIsNone(self.report['control_effect'])
        self.assertEqual([item['operation'] for item in runner.backend.calls],
                         ['observe', 'observe', 'key', 'observe', 'close'])

    def test_tutorial_enter_attempt_and_legacy_output_have_no_camera_v2_export(self):
        runner = TutorialDriverTests.prepare(self)
        self.assertEqual(self.invoke(runner), 0)
        self.assertTrue(self.report['offered_response_acknowledgment']['complete'])
        self.assertIsNone(self.report['control_effect'])
        self.assertNotIn('control_export_contract', self.report)
        self.assertNotIn('native_posts_raw_v2', self.report)
        self.assertFalse((runner.directory / 'controls.json').exists())
        self.assertEqual([item['operation'] for item in runner.backend.calls],
                         ['observe', 'observe', 'key', 'observe', 'close'])
        keys = [item for item in runner.backend.calls if item['operation'] == 'key']
        self.assertEqual([(item['code'], item['press_ms']) for item in keys], [(36, 20)])


class FinalJournalDriverTests(unittest.TestCase):
    """Additive guarded synthetic controls; authored, never run for this artifact."""

    prepare = CameraExportDriverTests.prepare
    invoke = DriverTests.invoke
    assert_no_export = CameraExportDriverTests.assert_no_export

    def setUp(self):
        CameraExportDriverTests.setUp(self)
        from test_window_journal_observation import SyntheticGuard
        sources = [driver.SOURCE / name for name in ('play_alpha.py', 'window_session.py',
            'window_evidence.py', 'window_journal_observation.py', 'window_session_macos.swift')]
        sources.extend(play_alpha.REPOSITORY / name for name in driver.SCHEMA_PATHS)
        guard = SyntheticGuard(self.root, sources=sources)
        guard.__enter__()
        self.addCleanup(guard.__exit__, None, None, None)

    def test_independent_final_bytes_preserve_legacy_buffer_and_actual_camera_associations(self):
        runner = self.prepare()
        self.assertEqual(self.invoke(runner), 0)
        final = self.report['journal_final_observation']
        raw = (runner.telemetry / 'events.jsonl').read_bytes()
        self.assertEqual(final['contract'], 'sims-window-journal-final-observation-v1')
        self.assertTrue(final['complete'])
        self.assertTrue(final['child_collected'])
        self.assertEqual(final['wholefile_sha256'], driver.hashlib.sha256(raw).hexdigest())
        self.assertEqual(final['bytes_read'], len(raw))
        self.assertEqual(self.report['journal_buffer']['bytes'], len(runner.payload))
        self.assertEqual(final['retained_buffer']['bytes'], len(runner.payload))
        self.assertEqual(runner.final_raw, raw)
        self.assertEqual((runner.directory / 'journal-final-observed.raw').read_bytes(), raw)
        control = self.report['exported_controls'][0]
        self.assertEqual(set(control), {'control_effect', 'native_result', 'binding_before', 'binding_after',
                                       'fence', 'started_unix_ns', 'finished_unix_ns', 'next_started_unix_ns'})
        self.assertIsNone(control['next_started_unix_ns'])
        self.assertIsNotNone(self.report['close_started_unix_ns'])
        self.assertEqual([(item['code'], item['press_ms']) for item in runner.backend.calls
                          if item['operation'] == 'key'], [(124, 20)])

    def test_primary_cleanup_final_read_artifact_and_report_errors_remain_independent(self):
        runner = self.prepare()
        runner.backend.key_error = subprocess.TimeoutExpired('authored fake key', 2, output='partial key')
        runner.backend.close_ignored = runner.backend.cleanup_fails = runner.backend.kill_fails = True
        observe = driver.journal_observation.JournalObservation.observe
        reports = []
        def failed_observe(observer, **keywords):
            result, raw = observe(observer, **keywords)
            observer.fail(result, OSError('authored final read error'))
            return result, raw
        def failed_snapshot(observer, result, raw):
            result['snapshot'].update(status='failed', error='authored artifact error')
        write = driver.write_new
        def failed_report(path, value):
            if path.name == 'result.json':
                reports.append(value)
                raise OSError('authored report error')
            return write(path, value)
        with mock.patch.object(driver.journal_observation.JournalObservation, 'observe', failed_observe), \
                mock.patch.object(driver.journal_observation.JournalObservation, 'snapshot', failed_snapshot), \
                mock.patch.object(driver, 'write_new', failed_report), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            result = play_alpha.launch(self.binary, self.game, timeout=10,
                window_session_output=runner.directory / 'telemetry', child_runner=runner)
        self.assertNotEqual(result, 0)
        self.report = reports[0]
        self.assert_no_export()
        self.assertIsNotNone(self.report['primary_failure'])
        self.assertTrue(any(item.get('error') for item in self.report['cleanup']))
        self.assertTrue(self.report['journal_read_errors'])
        self.assertIn('authored final read error', self.report['journal_final_observation']['error'])
        self.assertEqual(self.report['journal_final_observation']['snapshot']['error'], 'authored artifact error')
        self.assertEqual(self.report['report_error'], 'authored report error')
        self.assertIsNone(self.report['actual_child_returncode'])
        self.assertFalse(self.report['journal_final_observation']['child_collected'])

    def test_malformed_final_raw_is_retained_without_replacing_previous_parsed_records(self):
        runner = self.prepare()
        native = runner.native
        previous = []
        def malformed(operation, cleanup=False):
            result = native(operation, cleanup)
            if operation == 'close':
                previous.append((runner.payload, copy.deepcopy(runner.records)))
                with (runner.telemetry / 'events.jsonl').open('ab') as stream:
                    stream.write(b'{malformed final record}\n')
            return result
        runner.native = malformed
        self.assertNotEqual(self.invoke(runner), 0)
        self.assert_no_export()
        final = self.report['journal_final_observation']
        self.assertTrue(final['complete'])
        self.assertEqual(final['semantic']['status'], 'failed')
        self.assertTrue(runner.final_raw.endswith(b'{malformed final record}\n'))
        self.assertEqual((runner.payload, runner.records), previous[0])
        self.assertEqual((runner.directory / 'journal-final-observed.raw').read_bytes(), runner.final_raw)

    def test_controls_and_report_errors_still_preserve_original_partial_result(self):
        runner = self.prepare()
        reports = []
        write = driver.write_new
        def failed(path, value):
            if path.name == 'controls.json':
                raise OSError('authored controls write failure')
            if path.name == 'result.json':
                reports.append(value)
                raise OSError('authored result write failure')
            return write(path, value)
        with mock.patch.object(driver, 'write_new', failed), \
                contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            result = play_alpha.launch(self.binary, self.game, timeout=10,
                window_session_output=runner.directory / 'telemetry', child_runner=runner)
        self.assertNotEqual(result, 0)
        self.report = reports[0]
        self.assert_no_export()
        self.assertTrue(self.report['partial_control_effect_diagnostic']['complete'])
        self.assertEqual(self.report['partial_control_effect_diagnostic'], runner.control)
        self.assertEqual(self.report['controls_write_error'], 'authored controls write failure')
        self.assertEqual(self.report['report_error'], 'authored result write failure')

    def test_successful_raw_observation_does_not_recover_primary_failure(self):
        runner = self.prepare()
        runner.backend.key_error = ValueError('authored primary helper failure')
        self.assertNotEqual(self.invoke(runner), 0)
        self.assert_no_export()
        self.assertTrue(self.report['journal_final_observation']['complete'])
        self.assertEqual(self.report['primary_failure'], 'authored primary helper failure')
        self.assertFalse(self.report['graceful_end'])


if __name__ == '__main__':
    unittest.main()
