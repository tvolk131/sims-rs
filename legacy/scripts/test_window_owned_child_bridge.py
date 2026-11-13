"""UNRUN guarded authored mocks only; C checks are STRUCTURAL TEXT ONLY.

No passing receipt is supplied. Run only in a separate parent-approved guarded
interpreter slot. This file neither loads the extension nor qualifies native ABI,
platform, a real Popen, waiter ownership, publication races or reconciliation.
"""

import ast
import builtins
import copy
import importlib.machinery
import os
from pathlib import Path
import sys
import time
import unittest


def _deny(*arguments, **keywords):
    raise AssertionError('Forbidden process/native/network/sleep operation in mock-only tests')


_ORIGINAL_IMPORT = builtins.__import__
_FORBIDDEN_MODULES = frozenset((
    'subprocess', '_posixsubprocess', 'ctypes', '_ctypes', 'socket', '_socket',
    'multiprocessing', 'psutil', 'signal', 'platform', 'resource',
    '_window_owned_child_bridge',
))


def _guard_import(name, globals=None, locals=None, fromlist=(), level=0):
    if name.split('.')[0] in _FORBIDDEN_MODULES:
        _deny()
    existing = sys.modules.get(name)
    filename = getattr(existing, '__file__', '')
    if type(filename) is str and filename.endswith(('.so', '.dylib', '.pyd')):
        _deny()
    return _ORIGINAL_IMPORT(name, globals, locals, fromlist, level)


def _audit(event, arguments):
    if event.startswith(('subprocess.', 'ctypes.', 'socket.', 'os.exec', 'os.spawn',
                         'os.posix_spawn', 'os.fork', 'os.kill', 'os.system')):
        _deny()
    if event == 'import' and len(arguments) > 1:
        filename = arguments[1]
        if type(filename) is str and filename.endswith(('.so', '.dylib', '.pyd')):
            _deny()


def _install_guards():
    builtins.__import__ = _guard_import
    importlib.machinery.ExtensionFileLoader.create_module = _deny
    importlib.machinery.ExtensionFileLoader.exec_module = _deny
    for name in (
        'fork', 'forkpty', 'posix_spawn', 'posix_spawnp', 'system', 'popen',
        'wait', 'wait3', 'wait4', 'waitid', 'waitpid', 'kill', 'killpg',
        'getpid', 'getppid', 'getuid', 'geteuid', 'getgid', 'getegid',
        'getpgid', 'getsid', 'setsid', 'setpgid', 'setuid', 'seteuid',
    ):
        if hasattr(os, name):
            setattr(os, name, _deny)
    for name in tuple(vars(os)):
        if name.startswith(('exec', 'spawn')):
            setattr(os, name, _deny)
    time.sleep = _deny
    sys.addaudithook(_audit)


_install_guards()

import window_owned_child_bridge as facade
import window_owned_child as policy


def description(**changes):
    result = dict(bytes=32, version=1, error=0, system_errno=0,
                  record_bytes=200, pid_bytes=4, platform=1, enabled=0)
    result.update(changes)
    return result


def record(**changes):
    result = dict.fromkeys(facade.HEADER_FIELDS + facade.RECORD_FIELDS, 0)
    result.update(bytes=200, version=1, pid=17, pgid=17, uid=12, ppid=7, sid=17,
                  state=1, uid_evidence=1, sigchld_default=1, pid_bytes=4, capacity=2)
    result.update(changes)
    return result


def reply(operation, value, rc=0, owner=None, initialized=True):
    return dict(operation=operation, rc=rc, initialized=initialized,
                description=value if operation == 'describe' else None,
                record=None if operation == 'describe' else value, owner=owner)


class FakeBridge:
    """Authored token registry, not a real process/extension/ABI implementation."""

    def __init__(self):
        self.token = object()
        self.calls = []
        self.roots = []
        self.actions = {}
        self.exited = False
        self.exit_kind = 1
        self.exit_status = 0

    def _take(self, operation, value):
        self.calls.append(operation)
        choices = self.actions.get(operation, [])
        if choices:
            action = choices.pop(0)
            if isinstance(action, BaseException):
                raise action
            return action(value) if callable(action) else action
        return value

    def _observation(self, **changes):
        result = record()
        if self.exited:
            result.update(state=2, uid_evidence=2, exit_kind=self.exit_kind,
                          exit_status=self.exit_status,
                          returncode=self.exit_status if self.exit_kind == 1 else -self.exit_status)
        result.update(changes)
        return result

    def describe(self):
        return self._take('describe', reply('describe', description()))

    def open(self, pid, pgid, uid):
        result = reply('open', record(pid=pid, pgid=pgid, uid=uid, sid=pid), owner=self.token)
        self.roots.append(result)
        return self._take('open', result)

    def observe(self, owner):
        assert owner is self.token
        return self._take('observe', reply('observe', self._observation()))

    def members(self, owner):
        assert owner is self.token
        return self._take('members', reply('members', record(
            state=0, uid_evidence=2, count=1, raw_bytes=4, member0=17)))

    def member(self, owner, pid):
        assert owner is self.token
        return self._take('member', reply('member', record(pid=pid)))

    def signal(self, owner, kind, positive):
        assert owner is self.token
        return self._take('signal', reply('signal', self._observation(signal_attempted=1)))

    def latch(self, owner, kind, status):
        assert owner is self.token
        self.exit_kind, self.exit_status = kind, status
        self.exited = True
        return self._take('latch', reply('latch', self._observation(no_signals=1)))

    def reap(self, owner):
        assert owner is self.token
        return self._take('reap', reply('reap', self._observation(
            no_signals=1, reap_attempted=1, reaped=1)))

    def release(self, owner):
        _deny()


class FakeLock:
    def __init__(self, fail_release=False):
        self.held = False
        self.fail_release = fail_release

    def acquire(self, blocking):
        assert blocking is False
        self.held = True
        return True

    def release(self):
        if self.fail_release:
            raise RuntimeError('authored lock publication failure')
        self.held = False


class FakeProcess:
    """Deliberate fake; no claim of Popen authenticity from this class or its name."""

    pid = 17

    def __init__(self, fail_publication=False, fail_release=False):
        self._returncode = None
        self.fail_publication = fail_publication
        self._waitpid_lock = FakeLock(fail_release)

    @property
    def returncode(self):
        return self._returncode

    @returncode.setter
    def returncode(self, value):
        if self.fail_publication:
            raise RuntimeError('authored status publication failure')
        self._returncode = value


class AuthoredClock:
    def __init__(self):
        self.value = 0

    def __call__(self):
        self.value += 0.01
        return self.value


class BridgeMocks(unittest.TestCase):
    def setUp(self):
        self.children = []

    def tearDown(self):
        for child in self.children:
            policy.RETAINED_HANDLES.pop(child.token, None)

    def backend(self):
        bridge = FakeBridge()
        backend = facade.Backend.for_injected_mock(bridge)
        backend.describe()
        return bridge, backend

    def opened(self):
        bridge, backend = self.backend()
        owner, value = backend.open(17, 17, 12)
        self.assertEqual(value, record())
        return bridge, backend, owner

    def managed(self, bridge, backend, process=None, max_calls=128):
        manager = policy.OwnedChildren(backend, AuthoredClock(), timeout=2, grace=0,
                                       max_calls=max_calls)
        process = process if process is not None else FakeProcess()
        child = manager.register_owned(process, 12, dict.fromkeys(policy.ASSUMPTIONS, True))
        self.children.append(child)
        self.assertTrue(child.registered)
        return manager, child

    def test_disabled_construction_never_touches_loader_clock_or_keywords(self):
        class Bomb:
            def __getattribute__(self, name):
                _deny()

            def __call__(self, *arguments, **keywords):
                _deny()

        for constructor in (facade.production_backend, facade.Backend, facade.require_launch_ready):
            with self.assertRaises(facade.Unsupported):
                constructor(loader=Bomb(), bridge=Bomb(), clock=Bomb(), enabled=True)
        self.assertTrue(all(value is False for value in facade.QUALIFICATIONS.values()))
        self.assertTrue(all(value is False for value in policy.QUALIFICATIONS.values()))

    def test_guard_precedes_facade_and_policy_imports(self):
        self.assertIs(builtins.__import__, _guard_import)
        self.assertIs(time.sleep, _deny)
        self.assertIs(os.waitpid, _deny)
        self.assertIs(importlib.machinery.ExtensionFileLoader.exec_module, _deny)
        for name in _FORBIDDEN_MODULES:
            with self.assertRaises(AssertionError):
                _guard_import(name)

    def test_facade_source_has_no_import_clock_loader_or_finalizer(self):
        source = Path(facade.__file__).read_text()
        tree = ast.parse(source)
        # The authenticated private factory may import only its Admission owner.
        # Public/mock paths remain import-free, with no loader/clock fallback.
        private = [node for node in tree.body if isinstance(node, ast.FunctionDef)
                   and node.name == '_private_normal_execution']
        self.assertEqual(len(private), 1)
        imports = [node for node in ast.walk(tree)
                   if isinstance(node, (ast.Import, ast.ImportFrom))]
        self.assertEqual(len(imports), 1)
        self.assertIsInstance(imports[0], ast.Import)
        self.assertEqual([(name.name, name.asname) for name in imports[0].names],
                         [('window_normal_admission', None)])
        self.assertIn(imports[0], list(ast.walk(private[0])))
        for node in tree.body:
            if node is not private[0]:
                self.assertFalse(any(isinstance(child, (ast.Import, ast.ImportFrom))
                                     for child in ast.walk(node)))
        blocked = {'__import__', 'eval', 'exec', 'open', 'getenv', 'monotonic',
                   'monotonic_ns', 'sleep', 'spec_from_file_location',
                   'module_from_spec', 'exec_module', 'load_module'}
        for node in ast.walk(tree):
            if isinstance(node, ast.Call):
                name = (node.func.id if isinstance(node.func, ast.Name) else
                        node.func.attr if isinstance(node.func, ast.Attribute) else None)
                if name == 'open' and isinstance(node.func, ast.Attribute):
                    # This is the established mock/owned protocol operation,
                    # not builtin/OS file opening or a loader fallback.
                    receiver = node.func.value
                    self.assertIsInstance(receiver, ast.Attribute)
                    self.assertEqual(receiver.attr, '_bridge')
                    self.assertIsInstance(receiver.value, ast.Name)
                    self.assertEqual(receiver.value.id, 'self')
                else:
                    self.assertNotIn(name, blocked)
        self.assertNotIn('def __del__', source)
        self.assertNotIn('getenv(', source)

    def test_exact_description_fields_types_and_sizes(self):
        invalid = [description(bytes=31), description(version=0), description(enabled=1),
                   description(pid_bytes=3), description(platform=2), description(error=True),
                   description(system_errno=-1), description(record_bytes=199)]
        missing = description()
        missing.pop('error')
        invalid.extend((missing, dict(description(), extra=0)))
        for value in invalid:
            with self.subTest(value=value):
                bridge = FakeBridge()
                bridge.actions['describe'] = [reply('describe', value)]
                backend = facade.Backend.for_injected_mock(bridge)
                with self.assertRaises(facade.BridgeProtocolError):
                    backend.describe()

    def test_reply_shape_and_uninitialized_outputs_fail_closed(self):
        changes = [dict(initialized=False, record=None), dict(initialized=1),
                   dict(operation='members'), dict(description=description()),
                   dict(owner=object()), dict(rc=True), dict(rc=-1), dict(rc=2 ** 31)]
        for change in changes:
            bridge, backend, owner = self.opened()
            raw = reply('observe', record())
            raw.update(change)
            bridge.actions['observe'] = [raw]
            with self.assertRaises(facade.BridgeProtocolError) as error:
                backend.observe(owner)
            self.assertIs(error.exception.reply, raw)
            self.assertTrue(backend._slots[0].blocked)
            self.assertIs(backend._slots[0].native_owner, bridge.token)

    def test_each_exact_record_field_rejects_bool(self):
        for field in facade.HEADER_FIELDS + facade.RECORD_FIELDS:
            bridge, backend, owner = self.opened()
            value = record()
            value[field] = True
            bridge.actions['observe'] = [reply('observe', value)]
            with self.subTest(field=field), self.assertRaises(facade.BridgeProtocolError):
                backend.observe(owner)

    def test_return_header_mismatches_preserve_positive_negative_and_errno(self):
        for rc, error_code, system_errno in ((0, 3, 5), (3, 0, 0), (-1, 3, 5),
                                             (3, -1, 5), (5, 5, 9), (3, 3, -9)):
            bridge, backend, owner = self.opened()
            raw = reply('signal', record(error=error_code, system_errno=system_errno,
                                         signal_attempted=1), rc=rc)
            bridge.actions['signal'] = [raw]
            with self.assertRaises(facade.BridgeProtocolError) as error:
                backend.signal(owner, 1, 0)
            self.assertIs(error.exception.reply, raw)
            self.assertEqual((raw['rc'], raw['record']['error'], raw['record']['system_errno']),
                             (rc, error_code, system_errno))

    def test_consistent_native_errors_keep_raw_fields(self):
        bridge, backend, owner = self.opened()
        raw = reply('members', record(error=3, system_errno=5, raw_bytes=-1), rc=3)
        bridge.actions['members'] = [raw]
        with self.assertRaises(facade.NativeFailure) as error:
            backend.members(owner)
        self.assertEqual(error.exception.rc, 3)
        self.assertEqual(error.exception.record['raw_bytes'], -1)
        self.assertIs(error.exception.reply, raw)

    def test_signal_error_receipt_is_not_normalized(self):
        bridge, backend, owner = self.opened()
        raw = reply('signal', record(error=3, system_errno=5, signal_attempted=1), rc=3)
        bridge.actions['signal'] = [raw]
        value = backend.signal(owner, 1, 0)
        self.assertEqual(value, raw['record'])
        self.assertEqual((value['error'], value['system_errno']), (3, 5))
        with self.assertRaises(policy.Unsupported):
            policy.checked_record(value, 'signal')
        self.assertEqual(backend.observe(owner), record())
        with self.assertRaises(facade.BridgeProtocolError):
            backend.signal(owner, 2, 0)
        self.assertEqual(bridge.calls.count('signal'), 1)

    def test_ambiguous_signal_never_becomes_a_signal_receipt(self):
        for changes in (dict(signal_attempted=0), dict(pid=18), dict(system_errno=0)):
            bridge, backend, owner = self.opened()
            value = record(error=3, system_errno=5, signal_attempted=1)
            value.update(changes)
            bridge.actions['signal'] = [reply('signal', value, rc=3)]
            with self.assertRaises(facade.BridgeProtocolError):
                backend.signal(owner, 1, 0)
            self.assertTrue(backend._slots[0].blocked)

    def test_owner_identity_copy_numeric_foreign_and_reused_tokens(self):
        bridge, backend, owner = self.opened()
        for invalid in (17, 0, None, object(), object.__new__(facade._Owner)):
            with self.assertRaises(facade.BridgeProtocolError):
                backend.observe(invalid)
        for copier in (copy.copy, copy.deepcopy):
            with self.assertRaises(TypeError):
                copier(owner)
        other_bridge, other_backend = self.backend()
        with self.assertRaises(facade.BridgeProtocolError):
            other_backend.observe(owner)
        other_bridge.token = bridge.token
        with self.assertRaises(facade.BridgeProtocolError):
            other_backend.open(17, 17, 12)
        self.assertTrue(other_backend._slots[0].blocked)

    def test_open_retains_before_interruption_and_malformed_result(self):
        for problem in (MemoryError('authored allocation failure'), KeyboardInterrupt(),
                        reply('open', record(bytes=1), owner=object()),
                        reply('open', record(), owner=17), None):
            bridge, backend = self.backend()
            bridge.actions['open'] = [problem]
            with self.assertRaises(BaseException):
                backend.open(17, 17, 12)
            slot = backend._slots[0]
            self.assertTrue(any(item is slot for item in facade._RETAINED))
            self.assertTrue(slot.blocked)
            self.assertTrue(bridge.roots)
            with self.assertRaises(facade.BridgeProtocolError):
                backend.open(17, 17, 12)
            self.assertEqual(bridge.calls.count('open'), 1)

    def test_retention_and_reentrancy_guards_precede_mock_open_dispatch(self):
        bridge, backend = self.backend()

        def inside_open(raw):
            self.assertEqual(len(backend._slots), 1)
            self.assertTrue(any(item is backend._slots[0] for item in facade._RETAINED))
            self.assertTrue(backend._slots[0].busy)
            with self.assertRaises(facade.BridgeProtocolError):
                backend.open(17, 17, 12)
            with self.assertRaises(facade.BridgeProtocolError):
                backend.describe()
            return raw

        bridge.actions['open'] = [inside_open]
        backend.open(17, 17, 12)
        self.assertEqual(bridge.calls.count('open'), 1)

    def test_caller_return_publication_failure_does_not_discard_mock_slot(self):
        bridge, backend = self.backend()

        class RejectPublication:
            @property
            def result(self):
                raise AssertionError('Unreadable authored publication target')

            @result.setter
            def result(self, value):
                raise RuntimeError('authored caller assignment failure, not real Popen coverage')

        with self.assertRaises(RuntimeError):
            RejectPublication().result = backend.open(17, 17, 12)
        self.assertTrue(backend._slots[0].published)
        self.assertIs(backend._slots[0].native_owner, bridge.token)
        with self.assertRaises(facade.BridgeProtocolError):
            backend.open(17, 17, 12)

    def test_release_without_reconciliation_never_dispatches_or_reaps(self):
        bridge, backend, owner = self.opened()
        before = list(bridge.calls)
        with self.assertRaises(facade.ReconciliationRequired):
            backend.release(owner)
        with self.assertRaises(facade.BridgeProtocolError):
            backend.release(owner)
        with self.assertRaises(facade.BridgeProtocolError):
            backend.observe(owner)
        self.assertEqual(bridge.calls, before)

    def test_python_result_allocation_failure_retains_bridge_owner(self):
        bridge, backend = self.backend()
        original = facade._decode

        def failed_decode(raw, operation, description=False):
            if operation == 'open':
                raise MemoryError('authored mapping/result allocation failure')
            return original(raw, operation, description)

        facade._decode = failed_decode
        try:
            with self.assertRaises(MemoryError):
                backend.open(17, 17, 12)
        finally:
            facade._decode = original
        self.assertIs(backend._slots[0].native_owner, bridge.token)
        self.assertTrue(backend._slots[0].blocked)

    def test_native_error_open_with_owner_is_retained_not_released(self):
        bridge, backend = self.backend()
        raw = reply('open', record(error=4), rc=4, owner=bridge.token)
        bridge.actions['open'] = [raw]
        with self.assertRaises(facade.NativeFailure) as error:
            backend.open(17, 17, 12)
        self.assertIs(error.exception.reply, raw)
        self.assertIs(backend._slots[0].native_owner, bridge.token)
        self.assertNotIn('release', bridge.calls)

    def test_unresolved_operation_interruption_blocks_followup(self):
        for problem in (KeyboardInterrupt(), MemoryError(), RuntimeError()):
            bridge, backend, owner = self.opened()
            bridge.actions['observe'] = [problem]
            with self.assertRaises(BaseException):
                backend.observe(owner)
            with self.assertRaises(facade.BridgeProtocolError):
                backend.members(owner)
            self.assertEqual(bridge.calls.count('observe'), 1)

    def test_reap_is_one_attempt_and_release_is_not_reap(self):
        bridge, backend, owner = self.opened()
        backend.latch(owner, 2, 15)
        value = backend.reap(owner)
        self.assertEqual(value['returncode'], -15)
        for operation in (backend.reap, backend.observe, backend.members):
            with self.assertRaises(facade.BridgeProtocolError):
                operation(owner)
        with self.assertRaises(facade.ReconciliationRequired):
            backend.release(owner)
        with self.assertRaises(facade.BridgeProtocolError):
            backend.release(owner)
        self.assertEqual(bridge.calls.count('reap'), 1)
        self.assertNotIn('release', bridge.calls)
        self.assertIs(backend._slots[0].native_owner, bridge.token)

    def test_failed_reap_and_latch_never_retry_or_signal(self):
        for operation in ('latch', 'reap'):
            bridge, backend, owner = self.opened()
            if operation == 'reap':
                backend.latch(owner, 1, 0)
            bridge.actions[operation] = [KeyboardInterrupt()]
            with self.assertRaises(KeyboardInterrupt):
                backend.latch(owner, 1, 0) if operation == 'latch' else backend.reap(owner)
            with self.assertRaises(facade.BridgeProtocolError):
                backend.signal(owner, 1, 0)
            with self.assertRaises(facade.BridgeProtocolError):
                backend.reap(owner)
            self.assertEqual(bridge.calls.count(operation), 1)

    def test_reap_status_mismatch_is_retained_and_never_retried(self):
        bridge, backend, owner = self.opened()
        backend.latch(owner, 1, 0)
        bridge.actions['reap'] = [reply('reap', record(
            state=2, uid_evidence=2, exit_kind=1, exit_status=1, returncode=1,
            no_signals=1, reap_attempted=1, reaped=1))]
        with self.assertRaises(facade.BridgeProtocolError):
            backend.reap(owner)
        with self.assertRaises(facade.BridgeProtocolError):
            backend.reap(owner)
        self.assertEqual(bridge.calls.count('reap'), 1)
        self.assertIs(backend._slots[0].native_owner, bridge.token)

    def test_members_keep_raw_capacity_and_reject_truncation(self):
        bridge, backend, owner = self.opened()
        value = backend.members(owner)
        self.assertEqual((value['raw_bytes'], value['pid_bytes'], value['capacity']), (4, 4, 2))
        self.assertEqual(backend.member(owner, 18)['pid'], 18)
        for changes in (dict(raw_bytes=3), dict(raw_bytes=12), dict(count=0),
                        dict(capacity=3), dict(member0=18), dict(member1=17)):
            bridge, backend, owner = self.opened()
            value = record(state=0, uid_evidence=2, raw_bytes=4, count=1, member0=17)
            value.update(changes)
            bridge.actions['members'] = [reply('members', value)]
            with self.assertRaises(facade.BridgeProtocolError):
                backend.members(owner)

    def test_policy_success_preserves_allocation_retention(self):
        bridge, backend = self.backend()
        manager, child = self.managed(bridge, backend)
        bridge.exited = True
        receipt = manager.cleanup(child)
        self.assertTrue(receipt['resolved'])
        self.assertTrue(receipt['success'])
        self.assertEqual(child.process.returncode, 0)
        self.assertFalse(child.lock.held)
        self.assertTrue(backend._slots[0].reaped)
        self.assertNotIn('release', bridge.calls)
        with self.assertRaises(facade.BridgeProtocolError):
            backend.open(17, 17, 12)

    def test_policy_historical_signal_error_stays_failure_after_reap(self):
        bridge, backend = self.backend()
        manager, child = self.managed(bridge, backend)

        def failed_attempt(raw):
            raw['rc'] = 3
            raw['record'].update(error=3, system_errno=5)
            bridge.exited = True
            return raw

        bridge.actions['signal'] = [failed_attempt]
        receipt = manager.cleanup(child)
        self.assertTrue(receipt['resolved'])
        self.assertFalse(receipt['success'])
        self.assertTrue(receipt['errors'])
        self.assertEqual(bridge.calls.count('signal'), 1)
        self.assertEqual(bridge.calls.count('reap'), 1)
        raw = next(item['raw'] for item in receipt['observations'] if item['operation'] == 'signal')
        self.assertEqual((raw['error'], raw['system_errno']), (3, 5))

    def test_policy_ambiguous_signal_aborts_instead_of_sampling(self):
        bridge, backend = self.backend()
        manager, child = self.managed(bridge, backend)
        bridge.actions['signal'] = [reply('signal', record(
            error=3, system_errno=5, signal_attempted=1), rc=-1)]
        receipt = manager.cleanup(child)
        self.assertFalse(receipt['resolved'])
        self.assertFalse(receipt['success'])
        self.assertEqual(bridge.calls[-1], 'signal')
        self.assertIsNone(receipt['observations'][-1]['raw'])

    def _policy_publication_failure(self, fail_publication=False, fail_release=False):
        bridge, backend = self.backend()
        manager, child = self.managed(bridge, backend, FakeProcess(fail_publication, fail_release))
        bridge.exited = True
        receipt = manager.cleanup(child)
        self.assertFalse(receipt['resolved'])
        self.assertFalse(receipt['success'])
        self.assertTrue(receipt['retained'])
        self.assertIn(child.token, policy.RETAINED_HANDLES)
        self.assertTrue(child.lock.held)
        self.assertTrue(backend._slots[0].reaped)
        self.assertNotIn('release', bridge.calls)
        manager.cleanup(child)
        self.assertEqual(bridge.calls.count('reap'), 1)

    def test_policy_status_publication_failure_keeps_owner_retained(self):
        self._policy_publication_failure(fail_publication=True)

    def test_policy_lock_release_failure_keeps_owner_retained(self):
        self._policy_publication_failure(fail_release=True)

    def test_policy_total_call_budget_and_deadline_remain_sticky(self):
        bridge, backend = self.backend()
        manager, child = self.managed(bridge, backend, max_calls=2)
        receipt = manager.cleanup(child)
        calls, deadline = child.calls, child.deadline
        self.assertEqual(calls, 2)
        self.assertTrue(child.blocked)
        self.assertFalse(receipt['success'])
        manager.cleanup(child)
        self.assertEqual((child.calls, child.deadline), (calls, deadline))

    def test_c_text_only_public_header_retention_and_no_release_call(self):
        source = (Path(__file__).resolve().parent.parent /
                  'crates/sims-app/native/window_owned_child_python.c').read_text()
        self.assertIn('#include <Python.h>', source)
        self.assertIn('#include "window_owned_child_darwin.h"', source)
        self.assertIn('_Static_assert(WOC_NATIVE_QUALIFIED == 0', source)
        self.assertLess(source.index('retained_owner = owner;'), source.index('int rc = woc_open('))
        self.assertIn('&owner->native, &out, sizeof(out), WOC_VERSION', source)
        self.assertNotIn('woc_release(', source)
        self.assertNotIn('Py_BEGIN_ALLOW_THREADS', source)
        self.assertNotIn('PyLong_FromVoidPtr', source)
        self.assertNotIn('PyLong_AsVoidPtr', source)
        self.assertNotIn('dlsym(', source)
        self.assertNotIn('dlopen(', source)
        initializer = source.split('PyMODINIT_FUNC PyInit__window_owned_child_bridge(void)', 1)[1]
        self.assertNotIn('woc_describe(', initializer)
        self.assertNotIn('woc_open(', initializer)
        destructor = source.split('static void owner_dealloc', 1)[1].split('static PyMethodDef', 1)[0]
        self.assertNotIn('woc_', destructor)
        self.assertNotIn('wait', destructor)
        self.assertNotIn('signal', destructor)


if __name__ == '__main__':
    unittest.main()
