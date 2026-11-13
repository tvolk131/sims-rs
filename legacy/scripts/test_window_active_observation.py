"""Authored mock-C controls; UNRUN until exercised by the integrating parent.

Run separately: python3 -B -m unittest discover -s scripts
               -p test_window_active_observation.py -v

Only temporary copies enable the Child test branch. Production files and the
legacy gate are unchanged. No native child, process identity, ABI qualification,
or actual process-control behavior is established by these synthetic fixtures.
Compilers and mock executables are bounded subprocesses, with no DYLD settings.
"""

import hashlib
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
NATIVE = ROOT / 'crates/sims-app/native'
CLANG = '/Library/Developer/CommandLineTools/usr/bin/clang'
SDK = '/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk'
PINS = {
    'window_owned_child_darwin.c': '20967abd1ac970e908fbb0facd31f7c250a168865f4328caa74042a6010646ee',
    'window_owned_child_darwin.h': 'b2294ecdba9a395f8a11d5a775bd440d4df4258452e6e10fcb7b25ee132be9b3',
    'window_owned_child_python.c': 'bed6393d66860cbc1ee56c66925e564ad1b8d8633d265c453458d4ddf6e6cb1a',
}
ENV = {'PATH': '/usr/bin:/bin', 'LC_ALL': 'C', 'TMPDIR': '/private/tmp',
       'SDKROOT': SDK, 'DEVELOPER_DIR': '/Library/Developer/CommandLineTools'}
CASES = (
    'live_repeat_cache', 'live_statuses', 'terminal_kinds', 'live_then_terminal',
    'first_terminal_stable', 'terminal_contradictions', 'wait_failures',
    'bsd_identity', 'bsd_failures', 'creator_identity', 'disposition_failures',
    'exit_transition', 'clock_failures', 'failed_terminal_clock',
    'admission_guards', 'entry_budget', 'snapshot_guards', 'handoff_retention',
)


def forbidden_imports(path):
    """Read Mach-O symbol metadata, without invoking a process-inspection tool."""
    data = path.read_bytes()
    if data[:4] != b'\xcf\xfa\xed\xfe':
        raise AssertionError('Expected a little-endian 64-bit authored Mach-O')
    commands = struct.unpack_from('<I', data, 16)[0]
    offset = 32
    tables = []
    for _ in range(commands):
        command, size = struct.unpack_from('<II', data, offset)
        if size < 8 or offset + size > len(data):
            raise AssertionError('Malformed authored Mach-O load command')
        if command == 2:
            if size != 24:
                raise AssertionError('Unexpected symbol command size')
            tables.append(struct.unpack_from('<IIII', data, offset + 8))
        offset += size
    if len(tables) != 1:
        raise AssertionError('Expected exactly one authored symbol table')
    symbols, count, strings, string_bytes = tables[0]
    if symbols + count * 16 > len(data) or strings + string_bytes > len(data):
        raise AssertionError('Authored Mach-O symbol table is outside the file')
    native = {'getpid', 'getuid', 'getsid', 'sigaction', 'clock_gettime',
              'waitid', 'waitpid', 'kill', 'killpg', 'fcntl', 'close',
              'fork', 'vfork', 'execve', 'system', 'popen', 'calloc', 'malloc', 'free'}
    found = []
    for index in range(count):
        string_index, kind = struct.unpack_from('<IB', data, symbols + index * 16)
        if kind & 0xE0 or kind & 0x0E or not string_index:
            continue
        if string_index >= string_bytes:
            raise AssertionError('Invalid authored symbol string')
        start = strings + string_index
        end = data.find(b'\0', start, strings + string_bytes)
        if end < 0:
            raise AssertionError('Unterminated authored symbol')
        name = data[start:end].decode('ascii').lstrip('_').split('$', 1)[0]
        if name in native or name.startswith(('proc_', 'posix_spawn')):
            found.append(name)
    return found


class ActiveObservationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.original = {name: (NATIVE / name).read_bytes() for name in PINS}
        for name, value in cls.original.items():
            if hashlib.sha256(value).hexdigest() != PINS[name]:
                raise AssertionError('Observer source admission changed: ' + name)
        cls.temp = tempfile.TemporaryDirectory(prefix='active-observer-mock-', dir='/private/tmp')
        cls.addClassCleanup(cls.temp.cleanup)
        cls.directory = Path(cls.temp.name)
        cls.executables = {}
        for enabled in (0, 1):
            directory = cls.directory / str(enabled)
            directory.mkdir()
            header = cls.original['window_owned_child_darwin.h']
            source = cls.original['window_owned_child_darwin.c']
            if enabled:
                before = b'    WOC_CHILD_NATIVE_QUALIFIED = 0,'
                after = b'    WOC_CHILD_NATIVE_QUALIFIED = 1,'
                if header.count(before) != 1:
                    raise AssertionError('Expected one private Child-gate edit')
                header = header.replace(before, after)
                before = b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 0, "Child native gate must stay disabled");'
                after = b'_Static_assert(WOC_CHILD_NATIVE_QUALIFIED == 1, "Child native gate must stay disabled");'
                if source.count(before) != 1:
                    raise AssertionError('Expected one matching private assertion edit')
                source = source.replace(before, after)
            if header.count(b'    WOC_NATIVE_QUALIFIED = 0,') != 1:
                raise AssertionError('Legacy gate changed in private fixture')
            (directory / 'window_owned_child_darwin.h').write_bytes(header)
            (directory / 'window_owned_child_darwin.c').write_bytes(source)
            shutil.copy2(ROOT / 'scripts/fixtures/window_active_observation.c', directory / 'fixture.c')
            executable = directory / 'fixture'
            command = [CLANG, '-std=c11', '-O0', '-g0', '-Wall', '-Wextra',
                       '-Werror=implicit-function-declaration', '-isysroot', SDK,
                       '-DTEST_ACTIVE_MOCK=' + str(enabled), '-I', str(directory),
                       str(directory / 'fixture.c'), '-o', str(executable)]
            result = subprocess.run(command, env=ENV, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, timeout=45, check=False)
            if result.returncode:
                raise AssertionError('Mock fixture compile refused:\n' + result.stderr.decode(errors='replace'))
            if forbidden_imports(executable):
                raise AssertionError('Mock retained native imports: ' + repr(forbidden_imports(executable)))
            cls.executables[enabled] = executable

    @classmethod
    def tearDownClass(cls):
        for name, original in cls.original.items():
            if (NATIVE / name).read_bytes() != original:
                raise AssertionError('Production source changed during fixture: ' + name)

    def run_case(self, case, enabled=1):
        result = subprocess.run([str(self.executables[enabled]), case], env=ENV,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                timeout=5, check=False)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors='replace'))
        self.assertEqual(result.stdout, ('PASS ' + case + '\n').encode('ascii'))
        self.assertEqual(result.stderr, b'')

    def test_production_gate_refuses_poisoned_arguments_before_io(self):
        self.run_case('production_gate', enabled=0)


def install_case(case):
    def test(self):
        self.run_case(case)
    test.__name__ = 'test_' + case
    setattr(ActiveObservationTests, test.__name__, test)


for _case in CASES:
    install_case(_case)


if __name__ == '__main__':
    unittest.main()
