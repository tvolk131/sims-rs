"""UNRUN actual-PyC mapping tests with synthetic roots and mocked native calls.

Parent invocation (separate process): python3 -B -m unittest discover -s scripts
                                     -p test_window_active_mapping.py -v

The positive temporary copy has exactly THREE asserted source changes: Child
enum0->1, producer assertion0->1, PyC assertion0->1. The negative copy has zero.
All production source bytes and legacy0 stay unchanged. Fixture-only root
seeding is not native creation, ownership, process/ABI or alpha qualification.
No interpreter-wide allocator hooks, native process controls or clocks are used.
Compilation and bounded isolated Python execution belong to the parent.
"""

import hashlib
import importlib.util
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
NATIVE = ROOT / 'crates/sims-app/native'
CLANG = '/Library/Developer/CommandLineTools/usr/bin/clang'
PYTHON = '/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/bin/python3.9'
PYTHON_INCLUDE = '/Library/Developer/CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9/include/python3.9'
SDK = '/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk'
ENV = {'PATH': '/usr/bin:/bin', 'LC_ALL': 'C', 'TMPDIR': '/private/tmp',
       'SDKROOT': SDK, 'DEVELOPER_DIR': '/Library/Developer/CommandLineTools'}
PINS = {
    'window_owned_child_darwin.c': '20967abd1ac970e908fbb0facd31f7c250a168865f4328caa74042a6010646ee',
    'window_owned_child_darwin.h': 'b2294ecdba9a395f8a11d5a775bd440d4df4258452e6e10fcb7b25ee132be9b3',
    'window_owned_child_python.c': 'bed6393d66860cbc1ee56c66925e564ad1b8d8633d265c453458d4ddf6e6cb1a',
}
FIXTURE_PIN = 'afa730470e12582b8627f33830b84a9c9faa30c2d29cd9c128c8722a9830722c'
CHECKER_PIN = '1ca28fcc47b9cf300c042f27c535708c31f82aeee2e97437187a38499c6f3b34'
TOOL_PINS = {
    CLANG: 'f30550eab15fdf5ab8c0dc54c52679711241e5d4b636b027e18c09fef531775d',
    PYTHON: 'bdea59019a38eb6600cc9e71e984a97fedadc406448431281e7657030f54987e',
}
CASES = (
    'empty_cache', 'live_cache', 'live_transition_exit', 'inexit_transition',
    'exit_normal', 'exit_signal', 'exit_core', 'first_terminal_immutable',
    'exact_types', 'foreign_roots', 'child_generation', 'sink_generation',
    'missing_sink_root', 'module_reentry', 'phase_refusal',
    'dictionary_loss_live', 'dictionary_loss_terminal', 'snapshot_dictionary_loss',
    'failed_start_after_live', 'failed_start_after_exit', 'failed_terminal_finish',
    'wait_failure', 'partial_bsd', 'creator_failure', 'terminal_contradiction',
    'handoff_retention',
)


CHILD_SCRIPT = r'''
import importlib.util
from pathlib import Path
import sys

bundle, consumer_directory, case = sys.argv[1:]
spec = importlib.util.spec_from_file_location('_window_owned_child_bridge', bundle)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
sys.path.insert(0, consumer_directory)
import window_owned_child_bridge as protocol


def denied(kind, function, *arguments):
    try:
        function(*arguments)
    except kind as error:
        return error
    raise AssertionError('expected refusal: ' + str(kind))


def schema(value, fresh=False, first=False):
    fields = ('bytes version generation sequence started_ns finished_ns birth_seconds birth_microseconds '
              'entered returned rc system_errno wait_entered wait_returned wait_rc wait_errno '
              'bsd_entered bsd_returned bsd_bytes bsd_status start_clock_rc start_clock_errno '
              'finish_clock_rc finish_clock_errno creator_pid creator_uid observed_sid').split()
    assert set(value) == set(fields + ['fresh_native_call', 'first_terminal',
                                      'collection_evidence', 'record', 'raw_siginfo'])
    assert value['bytes'] == 344 and value['version'] == 1 and value['generation'] == 23
    assert value['fresh_native_call'] is fresh and value['first_terminal'] is first
    assert value['collection_evidence'] is False and type(value['raw_siginfo']) is bytes
    assert protocol._active_report(value, 23, fresh=fresh, first=first) is value
    return value


bridge, child, sink = module._fixture_seed()
assert bridge is module.new_bridge()
assert type(bridge) is module.Bridge and type(child) is module.Child and type(sink) is module.TerminalSink
assert bridge.pending() is child and bridge.history() == (sink,)
assert not any(protocol.QUALIFICATIONS.values())
assert protocol._native_execution_authorized(None) is False
denied(protocol.Unsupported, protocol.production_child_execution, object())

if case == 'production_gate':
    assert module.CHILD_NATIVE_QUALIFIED == 0
    before = module._fixture_stats()
    assert before['calls'] == 0
    module._fixture_scenario('module_busy')
    error = denied(RuntimeError, bridge.active_observe, object())
    assert 'hard-disabled' in str(error)
    for method in (sink.active_observation, sink.active_first_terminal):
        error = denied(RuntimeError, method)
        assert 'hard-disabled' in str(error)
    module._fixture_scenario('module_idle')
    assert module._fixture_stats() == before
    print('PASS production_gate')
    raise SystemExit(0)

assert module.CHILD_NATIVE_QUALIFIED == 1  # PRIVATE MOCK, never production authority.
assert bridge.terminal(child) is sink

if case == 'empty_cache':
    before = module._fixture_stats()
    for first in (False, True):
        value = schema(sink.active_first_terminal() if first else sink.active_observation(), first=first)
        assert value['entered'] == value['sequence'] == value['record']['state'] == 0
        assert value['raw_siginfo'] == b''
    assert module._fixture_stats() == before

elif case == 'live_cache':
    value = schema(bridge.active_observe(child), fresh=True)
    assert value['record']['state'] == 1 and value['record']['uid_evidence'] == 1
    assert value['record']['pid'] == value['record']['pgid'] == value['record']['sid'] == 41
    assert value['record']['ppid'] == value['creator_pid'] == 99
    assert value['record']['uid'] == value['creator_uid'] == 501
    assert (value['birth_seconds'], value['birth_microseconds']) == (123, 45)
    stats = module._fixture_stats()
    assert stats['waits'] == stats['bsds'] == stats['sids'] == 1 and stats['clocks'] == 2
    assert len(value['raw_siginfo']) == stats['raw_bytes']
    assert value['raw_siginfo'] == module._fixture_raw(False)
    value['record']['pid'] = 555
    for _ in range(3):
        cached = schema(sink.active_observation())
        assert cached['record']['pid'] == 41 and cached['sequence'] == 1
    assert module._fixture_stats() == stats
    assert schema(bridge.active_observe(child), fresh=True)['sequence'] == 2

elif case in ('live_transition_exit', 'inexit_transition'):
    if case == 'live_transition_exit':
        assert schema(bridge.active_observe(child), fresh=True)['record']['state'] == 1
    module._fixture_scenario('transition' if case == 'live_transition_exit' else 'inexit')
    value = schema(bridge.active_observe(child), fresh=True)
    assert value['record']['state'] == 3 and value['record']['uid_evidence'] == 2
    assert value['observed_sid'] == 0 and value['record']['exit_kind'] == 0
    assert schema(sink.active_first_terminal(), first=True)['entered'] == 0
    module._fixture_scenario('exit0')
    value = schema(bridge.active_observe(child), fresh=True)
    first = schema(sink.active_first_terminal(), first=True)
    assert value['record']['state'] == 2 and first['sequence'] == value['sequence']

elif case in ('exit_normal', 'exit_signal', 'exit_core'):
    scenario, kind, status = {'exit_normal': ('exit7', 1, 7),
                              'exit_signal': ('signal', 2, 15),
                              'exit_core': ('core', 3, 11)}[case]
    module._fixture_scenario(scenario)
    value = schema(bridge.active_observe(child), fresh=True)
    assert value['record']['state'] == 2 and value['record']['exit_kind'] == kind
    assert value['record']['exit_status'] == status
    assert value['record']['returncode'] == (status if kind == 1 else -status)
    assert value['record']['uid'] == 501 and value['record']['uid_evidence'] == 2
    assert module._fixture_stats()['event_uid'] == 991
    assert value['raw_siginfo'] == module._fixture_raw(False)
    first = schema(sink.active_first_terminal(), first=True)
    assert first['raw_siginfo'] == module._fixture_raw(True) == value['raw_siginfo']
    assert first['record']['reaped'] == first['record']['reap_attempted'] == 0

elif case == 'first_terminal_immutable':
    module._fixture_scenario('exit7')
    schema(bridge.active_observe(child), fresh=True)
    first = schema(sink.active_first_terminal(), first=True)
    raw = first['raw_siginfo']
    first['record']['returncode'] = 99
    assert schema(bridge.active_observe(child), fresh=True)['sequence'] == 2
    retained = schema(sink.active_first_terminal(), first=True)
    assert retained['sequence'] == 1 and retained['record']['returncode'] == 7
    assert retained['raw_siginfo'] == raw

elif case == 'exact_types':
    before = module._fixture_stats()
    for wrong in (None, 41, {}, object(), sink):
        denied(TypeError, bridge.active_observe, wrong)
    for kind in (module.Bridge, module.Child, module.TerminalSink):
        denied(TypeError, kind)
        denied(TypeError, type, 'Subclass', (kind,), {})
    for value in (bridge, child, sink):
        denied(TypeError, value.__copy__)
        denied(TypeError, value.__deepcopy__, {})
        denied(TypeError, value.__reduce_ex__, 4)
    assert module._fixture_stats() == before

elif case == 'foreign_roots':
    before = module._fixture_stats()
    foreign_bridge = module._fixture_foreign('bridge')
    foreign_child = module._fixture_foreign('child')
    foreign_sink = module._fixture_foreign('sink')
    denied(RuntimeError, foreign_bridge.active_observe, child)
    denied(RuntimeError, bridge.active_observe, foreign_child)
    denied(RuntimeError, foreign_sink.active_observation)
    denied(RuntimeError, foreign_sink.active_first_terminal)
    assert module._fixture_stats() == before
    assert bridge.pending() is child and bridge.history() == (sink,)

elif case in ('child_generation', 'sink_generation', 'missing_sink_root'):
    scenario = 'sink_unroot' if case == 'missing_sink_root' else case
    module._fixture_scenario(scenario)
    before = module._fixture_stats()
    denied(RuntimeError, bridge.active_observe, child)
    if case != 'child_generation':
        denied(RuntimeError, sink.active_observation)
    assert module._fixture_stats() == before
    restore = {'child_generation': 'child_generation_restore',
               'sink_generation': 'sink_generation_restore',
               'missing_sink_root': 'sink_root_restore'}[case]
    module._fixture_scenario(restore)
    assert schema(bridge.active_observe(child), fresh=True)['sequence'] == 1

elif case == 'module_reentry':
    module._fixture_scenario('module_busy')
    before = module._fixture_stats()
    denied(RuntimeError, bridge.active_observe, child)
    denied(RuntimeError, sink.active_observation)
    assert module._fixture_stats() == before
    module._fixture_scenario('module_idle')
    assert schema(bridge.active_observe(child), fresh=True)['sequence'] == 1

elif case == 'phase_refusal':
    module._fixture_scenario('cleanup')
    before = module._fixture_stats()
    denied(RuntimeError, bridge.active_observe, child)
    assert module._fixture_stats() == before
    assert schema(sink.active_observation())['sequence'] == 0

elif case in ('dictionary_loss_live', 'dictionary_loss_terminal'):
    if case == 'dictionary_loss_terminal':
        module._fixture_scenario('exit7')
    for index in (1, 2):
        module._fixture_fail_dictionary(index)
        denied(MemoryError, bridge.active_observe, child)
        stats = module._fixture_stats()
        assert stats['sequence'] == stats['waits'] == index and stats['module_busy'] == 0
        cached = schema(sink.active_observation())
        assert cached['sequence'] == index and cached['raw_siginfo'] == module._fixture_raw(False)
        if case == 'dictionary_loss_terminal':
            first = schema(sink.active_first_terminal(), first=True)
            assert first['sequence'] == 1 and first['record']['returncode'] == 7
        assert module._fixture_stats() == stats

elif case == 'snapshot_dictionary_loss':
    module._fixture_scenario('exit0')
    schema(bridge.active_observe(child), fresh=True)
    first = schema(sink.active_first_terminal(), first=True)
    stats = module._fixture_stats()
    module._fixture_fail_dictionary(1)
    denied(MemoryError, sink.active_first_terminal)
    assert module._fixture_stats() == stats
    assert schema(sink.active_first_terminal(), first=True) == first

elif case in ('failed_start_after_live', 'failed_start_after_exit'):
    if case == 'failed_start_after_exit':
        module._fixture_scenario('exit7')
    schema(bridge.active_observe(child), fresh=True)
    first = schema(sink.active_first_terminal(), first=True)
    stats = module._fixture_stats()
    module._fixture_scenario('start_clock_failure')
    denied(RuntimeError, bridge.active_observe, child)
    failed = schema(sink.active_observation())
    assert failed['sequence'] == 2 and failed['rc'] == 3 and failed['record']['error'] == 3
    assert failed['started_ns'] == 0 and failed['start_clock_rc'] == -1
    assert failed['wait_entered'] == failed['wait_returned'] == 0 and failed['raw_siginfo'] == b''
    assert module._fixture_stats()['waits'] == stats['waits']
    assert schema(sink.active_first_terminal(), first=True) == first
    after = module._fixture_stats()
    denied(RuntimeError, bridge.active_observe, child)
    assert module._fixture_stats() == after
    assert schema(sink.active_observation()) == failed

elif case == 'failed_terminal_finish':
    module._fixture_scenario('exit7')
    module._fixture_scenario('finish_clock_failure')
    denied(RuntimeError, bridge.active_observe, child)
    latest = schema(sink.active_observation())
    first = schema(sink.active_first_terminal(), first=True)
    assert latest['sequence'] == first['sequence'] == 1 and first['rc'] == 3
    assert first['finish_clock_rc'] == -1 and first['record']['state'] == 2
    assert first['raw_siginfo'] == latest['raw_siginfo'] == module._fixture_raw(True)

elif case in ('wait_failure', 'partial_bsd', 'creator_failure'):
    module._fixture_scenario({'creator_failure': 'creator_mismatch'}.get(case, case))
    denied(RuntimeError, bridge.active_observe, child)
    failed = schema(sink.active_observation())
    assert failed['entered'] == failed['returned'] == failed['sequence'] == 1 and failed['rc'] != 0
    assert failed['record']['error'] == failed['rc']
    stats = module._fixture_stats()
    assert schema(sink.active_first_terminal(), first=True)['entered'] == 0
    assert schema(sink.active_observation()) == failed and module._fixture_stats() == stats

elif case == 'terminal_contradiction':
    module._fixture_scenario('exit7')
    schema(bridge.active_observe(child), fresh=True)
    first = schema(sink.active_first_terminal(), first=True)
    module._fixture_scenario('exit0')
    denied(RuntimeError, bridge.active_observe, child)
    failed = schema(sink.active_observation())
    assert failed['sequence'] == 2 and failed['rc'] != 0
    assert schema(sink.active_first_terminal(), first=True) == first

elif case == 'handoff_retention':
    module._fixture_scenario('exit0')
    schema(bridge.active_observe(child), fresh=True)
    first = schema(sink.active_first_terminal(), first=True)
    assert module._fixture_handoff() is sink  # collection facts were explicitly synthetic.
    stats = module._fixture_stats()
    assert stats['frees'] == 1 and stats['waits'] == 1
    assert bridge.pending() is None and bridge.history() == (sink,)
    assert schema(sink.active_first_terminal(), first=True) == first
    assert schema(sink.active_observation())['sequence'] == 1
    assert sink.snapshot()['transferred'] == 1
    denied(RuntimeError, bridge.active_observe, child)
    assert module._fixture_stats() == stats

else:
    raise AssertionError('unknown private case')

assert not any(protocol.QUALIFICATIONS.values())
assert protocol._native_execution_authorized(None) is False
print('PASS ' + case)
'''


def digest(value):
    return hashlib.sha256(value).hexdigest()


def admitted_bytes(path):
    details = path.lstat()
    if not stat.S_ISREG(details.st_mode) or details.st_nlink != 1 or path.is_symlink():
        raise AssertionError('Regular single-link mapping input required: ' + str(path))
    return path.read_bytes()


class ActiveMappingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inputs = {NATIVE / name: admitted_bytes(NATIVE / name) for name in PINS}
        for name, pin in PINS.items():
            if digest(cls.inputs[NATIVE / name]) != pin:
                raise AssertionError('Original active mapping source changed: ' + name)
        fixture = ROOT / 'scripts/fixtures/window_active_observation.c'
        checker = ROOT / 'scripts/test_window_active_observation.py'
        cls.inputs[fixture] = admitted_bytes(fixture)
        cls.inputs[checker] = admitted_bytes(checker)
        if digest(cls.inputs[fixture]) != FIXTURE_PIN or digest(cls.inputs[checker]) != CHECKER_PIN:
            raise AssertionError('Admitted native mocks/import checker changed')
        own_fixture = ROOT / 'scripts/fixtures/window_active_mapping.c'
        consumer = ROOT / 'scripts/window_owned_child_bridge.py'
        cls.inputs[own_fixture] = admitted_bytes(own_fixture)
        cls.inputs[consumer] = admitted_bytes(consumer)
        for filename, pin in TOOL_PINS.items():
            if digest(Path(filename).read_bytes()) != pin:
                raise AssertionError('Trusted mapping tool changed: ' + filename)
        # Execute only the admitted pure-Python metadata helper, never its tests.
        spec = importlib.util.spec_from_file_location('_mapping_import_checker', checker)
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        cls.temp = tempfile.TemporaryDirectory(prefix='active-pyc-mock-', dir='/private/tmp')
        cls.addClassCleanup(cls.temp.cleanup)
        cls.directory = Path(cls.temp.name)
        cls.bundles = {}
        for enabled in (0, 1):
            directory = cls.directory / str(enabled)
            directory.mkdir()
            originals = {name: cls.inputs[NATIVE / name] for name in PINS}
            edits = []
            if enabled:
                replacements = (
                    ('window_owned_child_darwin.h', b'    WOC_CHILD_NATIVE_QUALIFIED = 0,',
                     b'    WOC_CHILD_NATIVE_QUALIFIED = 1,'),
                    ('window_owned_child_darwin.c',
                     b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 0, "Child native gate must stay disabled");',
                     b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 1, "Child native gate must stay disabled");'),
                    ('window_owned_child_python.c',
                     b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 0, "Child qualification stays disabled");',
                     b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 1, "Child qualification stays disabled");'),
                )
                for name, before, after in replacements:
                    if originals[name].count(before) != 1 or originals[name].count(after) != 0:
                        raise AssertionError('Expected exact single positive-copy change: ' + name)
                    originals[name] = originals[name].replace(before, after)
                    edits.append(name)
            if len(edits) != (3 if enabled else 0):
                raise AssertionError('Unexpected private copied-source edit count')
            if originals['window_owned_child_darwin.h'].count(b'    WOC_NATIVE_QUALIFIED = 0,') != 1:
                raise AssertionError('Private mapping fixture changed legacy0')
            for name, data in originals.items():
                (directory / name).write_bytes(data)
            (directory / 'window_active_observation.c').write_bytes(cls.inputs[fixture])
            (directory / 'fixture.c').write_bytes(cls.inputs[own_fixture])
            (directory / 'window_owned_child_bridge.py').write_bytes(cls.inputs[consumer])
            bundle = directory / '_window_owned_child_bridge.so'
            command = [CLANG, '-std=c11', '-O0', '-g0', '-Wall', '-Wextra',
                       '-Werror=implicit-function-declaration', '-bundle', '-undefined', 'dynamic_lookup',
                       '-isysroot', SDK, '-I', PYTHON_INCLUDE, '-I', str(directory),
                       '-DTEST_ACTIVE_MOCK=' + str(enabled), str(directory / 'fixture.c'), '-o', str(bundle)]
            result = subprocess.run(command, env=ENV, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, timeout=45, check=False)
            if result.returncode:
                raise AssertionError('Private mapping compile refused:\n' + result.stderr.decode(errors='replace'))
            imports = helper.forbidden_imports(bundle)
            if imports:
                raise AssertionError('Private mapping retained forbidden native imports: ' + repr(imports))
            cls.bundles[enabled] = bundle
            print('PRIVATE MAPPING MOCK enabled=' + str(enabled) + ' source_edits=' + str(len(edits)) +
                  ' bundle_sha256=' + digest(bundle.read_bytes()) +
                  ' consumer_sha256=' + digest(cls.inputs[consumer]), flush=True)

    @classmethod
    def tearDownClass(cls):
        for path, data in cls.inputs.items():
            if path.read_bytes() != data:
                raise AssertionError('Production/admitted fixture input changed during mapping tests: ' + str(path))

    def run_case(self, case, enabled=1):
        bundle = self.bundles[enabled]
        result = subprocess.run([PYTHON, '-I', '-B', '-c', CHILD_SCRIPT,
                                 str(bundle), str(bundle.parent), case],
                                env=ENV, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                timeout=10, check=False)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors='replace'))
        self.assertEqual(result.stdout, ('PASS ' + case + '\n').encode('ascii'))
        self.assertEqual(result.stderr, b'')

    def test_production_gate_precedes_arguments_lifecycle_and_poisoned_native_pointers(self):
        self.run_case('production_gate', enabled=0)


def install_case(case):
    def test(self):
        self.run_case(case)
    test.__name__ = 'test_' + case
    setattr(ActiveMappingTests, test.__name__, test)


for _case in CASES:
    install_case(_case)


if __name__ == '__main__':
    unittest.main()
