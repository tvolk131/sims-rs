#!/usr/bin/env python3
"""Plan an opt-in macOS observer build; never load, launch, probe, or qualify it.

Only --execute may start a compiler, and the census prototype currently fails
closed as unsupported until parent-owned platform/completeness witnesses exist.
Outputs and failure logs stay in a fresh,
private direct child of checkout cache. The compiler's own -v diagnostic supplies
its version without a second compiler process. No caller environment is used.
The canonical SDK root and SDKSettings metadata are recorded, not its full closure.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shutil
import signal
import stat
import subprocess
import sys

import play_alpha
import window_process_group as owned_group


REPOSITORY = play_alpha.REPOSITORY
DEFAULT_SDK = Path('/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk')
RESERVE_BYTES = 6 * 1024**3
MAX_FILE_BYTES = 512 * 1024**2
MAX_TIMEOUT_SECONDS = 600
CLEANUP_TIMEOUT_SECONDS = 2
SOURCE_HASHES = {
    'crates/sims-app/native/window_present_macos.h':
        'af8bfedc623d8f6cea9c0aed31dbc1d69a624822296c75f016f3c5133e9c0fdb',
    'crates/sims-app/native/window_present_macos.m':
        'fdb8c118c964c5725168182ac06d5343371136581d1757717b109c6450d10c5c',
    'crates/sims-app/src/window_session/presentation.rs':
        'c679300b96095eebb5bd6813921d67d2820cb176bae6d6a47c30a9153800a063',
}
QUALIFICATIONS = {
    name: False for name in (
        'native_build', 'native_test', 'native_load', 'presentation', 'latency',
        'eligible_duration', 'eligible_day', 'alpha',
    )
}
SCOPE = ('Source build record only, not an acceptance verifier. API-reported '
         'presentedTime is not photon evidence. Independent review and actual '
         'integration are still required; the driver and analyzer stay unavailable.')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def state_fields(info):
    return {
        'device': info.st_dev, 'inode': info.st_ino, 'mode': info.st_mode,
        'uid': info.st_uid, 'links': info.st_nlink, 'size': info.st_size,
        'mtime_ns': info.st_mtime_ns, 'ctime_ns': info.st_ctime_ns,
    }


def regular_state(value, expected=None, executable=False):
    path = play_alpha.protected_path(value)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, 'rb') as stream:
        before = os.fstat(stream.fileno())
        require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1,
                'Expected a non-linked regular file: ' + str(path))
        require(before.st_size <= MAX_FILE_BYTES, 'File exceeds source-build bound: ' + str(path))
        require(not executable or before.st_mode & 0o111, 'Compiler is not executable')
        checksum = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            checksum.update(chunk)
        after = os.fstat(stream.fileno())
    require(state_fields(before) == state_fields(after), 'File changed while reading: ' + str(path))
    require(state_fields(after) == state_fields(path.lstat()), 'File path changed: ' + str(path))
    result = dict(state_fields(after), sha256=checksum.hexdigest())
    require(expected is None or result['sha256'] == expected, 'Source pin mismatch: ' + str(path))
    return result


def directory_state(value, private=False):
    path = play_alpha.protected_path(value)
    info = path.lstat()
    require(stat.S_ISDIR(info.st_mode), 'Expected a real directory: ' + str(path))
    require(info.st_uid == os.getuid(), 'Directory must belong to this user: ' + str(path))
    require(info.st_mode & 0o022 == 0, 'Directory must not be group/world writable: ' + str(path))
    require(not private or info.st_mode & 0o077 == 0, 'Output directory is not private')
    return {'device': info.st_dev, 'inode': info.st_ino}


def validate_output(value, protected):
    output = play_alpha.protected_path(value)
    cache = play_alpha.protected_path(REPOSITORY / 'cache')
    require(output.is_relative_to(cache) and output != cache,
            'Output must be strictly beneath this checkout cache')
    require(output.parent == cache, 'Output must be a direct cache child')
    require(not output.exists(), 'Output must be a fresh directory')
    for source in protected:
        require(not play_alpha.overlaps(output, play_alpha.protected_path(source)),
                'Output overlaps a protected input: ' + str(source))
    if cache.exists():
        directory_state(cache)
    require(output.parent.is_dir() or output.parent == cache and not cache.exists(),
            'Output needs an existing real parent (only cache may be created)')
    parent = output.parent if output.parent.exists() else REPOSITORY
    directory_state(parent)
    require(shutil.disk_usage(parent).free >= RESERVE_BYTES, 'At least six GiB free is required')
    return output


def plan_build(output='cache/window-presentation-build', compiler='/usr/bin/clang', timeout=120, sdk=None):
    require(sys.platform == 'darwin', 'Window presentation builds are macOS-only')
    require(all(hasattr(select, name) for name in ('kqueue', 'kevent', 'KQ_FILTER_PROC',
                                                  'KQ_EV_ADD', 'KQ_EV_ONESHOT', 'KQ_NOTE_EXIT', 'KQ_EV_ERROR')),
            'Non-reaping kqueue observation is required for safe process-group cleanup')
    require(signal.getsignal(signal.SIGCHLD) == signal.SIG_DFL,
            'A default SIGCHLD handler is required to retain the compiler group identity')
    require(isinstance(timeout, (int, float)) and 0 < timeout <= MAX_TIMEOUT_SECONDS,
            'Timeout must be positive and at most 600 seconds')
    require(Path(compiler).is_absolute(), 'Compiler path must be absolute')
    compiler = play_alpha.protected_path(compiler)
    compiler_before = regular_state(compiler, executable=True)
    cleanup_backend = owned_group.backend_plan(regular_state)
    requested_sdk = Path(DEFAULT_SDK if sdk is None else sdk)
    require(requested_sdk.is_absolute(), 'SDK path must be absolute')
    sdk_root = play_alpha.protected_path(requested_sdk.resolve(strict=True))
    sdk_info = sdk_root.lstat()
    require(stat.S_ISDIR(sdk_info.st_mode) and sdk_info.st_uid in (0, os.getuid())
            and sdk_info.st_mode & 0o022 == 0, 'SDK root must be trusted and not group/world writable')
    sdk_settings = sdk_root / 'SDKSettings.json'
    sdk_before = regular_state(sdk_settings)
    sources = [
        {'path': str(REPOSITORY / name), 'sha256': expected,
         'before': regular_state(REPOSITORY / name, expected)}
        for name, expected in SOURCE_HASHES.items()
    ]
    sources.append({'path': str(sdk_settings), 'sha256': sdk_before['sha256'], 'before': sdk_before})
    protected = [item['path'] for item in sources] + [compiler, sdk_root, cleanup_backend['path'],
                                                    REPOSITORY / 'scripts', REPOSITORY / '.git']
    output = validate_output(output, protected)
    native = output / 'native'
    product = output / 'window-present-macos.dylib'
    command = [
        str(compiler), '-v', '-std=c11', '-O2', '-fobjc-arc', '-fblocks',
        '-fvisibility=hidden', '-mmacosx-version-min=14.0', '-isysroot', str(sdk_root), '-dynamiclib',
        str(native / 'window_present_macos.m'), '-framework', 'Foundation',
        '-framework', 'QuartzCore', '-framework', 'Metal', '-framework', 'AppKit',
        '-o', str(product),
    ]
    environment = {
        'PATH': '/usr/bin:/bin', 'LANG': 'C', 'LC_ALL': 'C',
        'TMPDIR': str(output / 'tmp'), 'HOME': str(output / 'home'),
        'CLANG_MODULE_CACHE_PATH': str(output / 'module-cache'),
    }
    return {
        'schema': 1, 'kind': 'window_presentation_source_build', 'mode': 'plan',
        'scope': SCOPE, 'output_directory': str(output), 'output_path': str(product),
        'receipt_path': str(output / 'build-receipt.json'), 'command': command,
        'environment': environment, 'timeout_seconds': timeout,
        'cleanup_timeout_seconds': CLEANUP_TIMEOUT_SECONDS,
        'cleanup_backend': cleanup_backend,
        'minimum_free_bytes': RESERVE_BYTES,
        'compiler': {'path': str(compiler), 'before': compiler_before},
        'sdk_root': str(sdk_root),
        'sources': sources, 'qualifications': dict(QUALIFICATIONS),
    }


def write_new(directory_fd, name, data, mode=0o600):
    require(Path(name).name == name and name not in ('.', '..'), 'Unsafe output filename')
    descriptor = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                         mode, dir_fd=directory_fd)
    with os.fdopen(descriptor, 'wb') as stream:
        os.fchmod(stream.fileno(), mode)
        stream.write(data)


def pinned_bytes(path, expected, prefix_bytes=None):
    path = play_alpha.protected_path(path)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, 'rb') as stream:
        before = os.fstat(stream.fileno())
        require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1,
                'Expected a non-linked regular file')
        require(state_fields(before) == {key: expected[key] for key in state_fields(before)},
                'Input changed before bounded read')
        data = stream.read(MAX_FILE_BYTES + 1 if prefix_bytes is None else prefix_bytes)
        require(len(data) <= MAX_FILE_BYTES, 'Input exceeds bounded read')
        require(state_fields(before) == state_fields(os.fstat(stream.fileno())), 'Input changed during read')
    if prefix_bytes is None:
        require(hashlib.sha256(data).hexdigest() == expected['sha256'], 'Input digest changed during read')
    require(state_fields(before) == state_fields(path.lstat()), 'Input path changed during read')
    return data


def checked_state(path, errors, label, expected=None):
    try:
        return regular_state(path, expected)
    except (OSError, ValueError) as error:
        errors.append(label + ': ' + str(error))
        return None


def wait_without_reaping(process, timeout):
    """Keep the group leader's PID reserved until every group signal is sent."""
    event = select.kevent(process.pid, filter=select.KQ_FILTER_PROC,
                          flags=select.KQ_EV_ADD | select.KQ_EV_ONESHOT, fflags=select.KQ_NOTE_EXIT)
    queue = select.kqueue()
    try:
        events = queue.control([event], 1, timeout)
    finally:
        queue.close()
    if not events:
        raise subprocess.TimeoutExpired(process.args, timeout)
    require(len(events) == 1 and events[0].ident == process.pid
            and events[0].filter == select.KQ_FILTER_PROC
            and events[0].fflags == select.KQ_NOTE_EXIT
            and not events[0].flags & select.KQ_EV_ERROR, 'Compiler exit observation failed')


def run_compiler(plan, stdout, stderr):
    result = {'attempted': False, 'pid': None, 'returncode': None,
              'timed_out': False, 'error': None}
    cleanup = owned_group.empty_cleanup()
    process = None
    try:
        owned_group.require_execution_support()
        require(not owned_group.RETAINED_GROUPS and not owned_group.RETAINED_GROUP_HANDLES
                and not owned_group.RETAINED_AUXILIARIES,
                'Unresolved owned handles prevent another launch')
        result['attempted'] = True
        process = subprocess.Popen(
            plan['command'], cwd=plan['output_directory'], env=plan['environment'],
            stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
            start_new_session=True, close_fds=True, shell=False,
        )
        owned_group.RETAINED_GROUPS[process.pid] = process
        require(isinstance(process.pid, int) and process.pid > 1, 'Invalid compiler process ID')
        result['pid'] = process.pid
        try:
            wait_without_reaping(process, plan['timeout_seconds'])
            identity = owned_group.child_identity(process, os.getuid())
            require(identity['exited'], 'Exit event has no waitable child exit status')
            result['returncode'] = identity['returncode']
        except subprocess.TimeoutExpired as error:
            result['timed_out'] = True
            result['error'] = str(error)
    except (Exception, KeyboardInterrupt) as error:
        result['error'] = type(error).__name__ + ': ' + str(error)
    finally:
        if process is not None and result['pid'] is not None:
            try:
                cleanup = owned_group.cleanup_group(process, plan['cleanup_backend'], regular_state,
                                                    plan['cleanup_timeout_seconds'])
            except (Exception, KeyboardInterrupt) as error:
                cleanup = owned_group.empty_cleanup(attempted=True)
                owned_group.RETAINED_GROUP_HANDLES[id(process)] = process
                cleanup['retained'] = True
                cleanup['errors'].append('cleanup dispatch: ' + owned_group.describe_error(error))
        elif process is not None:
            cleanup = owned_group.empty_cleanup(attempted=True)
            owned_group.RETAINED_GROUP_HANDLES[id(process)] = process
            cleanup['retained'] = True
            cleanup['errors'].append('Leader identity not published; handle retained without signals or reap')
    return result, cleanup


def execute_build(plan):
    fresh = plan_build(plan['output_directory'], plan['compiler']['path'], plan['timeout_seconds'], plan['sdk_root'])
    require(fresh == plan, 'Plan or inputs changed before execution')
    output = Path(plan['output_directory'])
    cache = play_alpha.protected_path(REPOSITORY / 'cache')
    if not cache.exists():
        cache.mkdir(mode=0o700)
    directory_state(cache)
    validate_output(output, [item['path'] for item in plan['sources']] + [plan['compiler']['path']])
    output.mkdir(mode=0o700)
    identity = directory_state(output, private=True)
    directory_fd = os.open(output, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        require(identity == {'device': os.fstat(directory_fd).st_dev,
                             'inode': os.fstat(directory_fd).st_ino}, 'Output directory changed')
    except (Exception, KeyboardInterrupt):
        os.close(directory_fd)
        raise
    receipt = dict(plan, mode='execute', outcome=None, integrity_errors=[],
                   process={'attempted': False, 'pid': None, 'returncode': None,
                            'timed_out': False, 'error': None},
                   cleanup=owned_group.empty_cleanup(),
                   logs={}, output={'path': plan['output_path'], 'state': None},
                   setup_error=None, receipt_write_error=None)
    receipt['compiler'] = dict(plan['compiler'], after=None, version=None)
    receipt['sources'] = [dict(item, after=None, staged_path=None,
                               staged_before=None, staged_after=None) for item in plan['sources']]
    errors = receipt['integrity_errors']
    try:
        for name in ('native', 'tmp', 'home', 'module-cache'):
            os.mkdir(name, mode=0o700, dir_fd=directory_fd)
        native_fd = os.open('native', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                            dir_fd=directory_fd)
        try:
            for item in receipt['sources'][:2]:
                source = Path(item['path'])
                require(regular_state(source, item['sha256']) == item['before'],
                        'Source changed before staging: ' + str(source))
                write_new(native_fd, source.name, pinned_bytes(source, item['before']), mode=0o400)
                item['staged_path'] = str(output / 'native' / source.name)
                item['staged_before'] = regular_state(item['staged_path'], item['sha256'])
        finally:
            os.close(native_fd)
        for item in receipt['sources']:
            require(regular_state(item['path'], item['sha256']) == item['before'],
                    'Source changed before compiler invocation')
        require(regular_state(plan['compiler']['path'], executable=True) == plan['compiler']['before'],
                'Compiler changed before invocation')
        require(directory_state(output, private=True) == identity, 'Output directory changed')
        require(shutil.disk_usage(output).free >= RESERVE_BYTES, 'At least six GiB free is required')
        stdout_fd = os.open('compiler.stdout.log', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                            0o600, dir_fd=directory_fd)
        with os.fdopen(stdout_fd, 'wb') as stdout:
            stderr_fd = os.open('compiler.stderr.log', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                0o600, dir_fd=directory_fd)
            with os.fdopen(stderr_fd, 'wb') as stderr:
                receipt['process'], receipt['cleanup'] = run_compiler(plan, stdout, stderr)
    except (Exception, KeyboardInterrupt) as error:
        receipt['setup_error'] = type(error).__name__ + ': ' + str(error)
    finally:
        try:
            for item in receipt['sources']:
                item['after'] = checked_state(item['path'], errors, 'source', item['sha256'])
                if item['after'] != item['before']:
                    errors.append('Source changed: ' + item['path'])
                if item['staged_path']:
                    item['staged_after'] = checked_state(item['staged_path'], errors, 'staged source', item['sha256'])
                    if item['staged_after'] != item['staged_before']:
                        errors.append('Staged source changed: ' + item['staged_path'])
            receipt['compiler']['after'] = checked_state(plan['compiler']['path'], errors, 'compiler')
            if receipt['compiler']['after'] != plan['compiler']['before']:
                errors.append('Compiler changed')
            for key in ('stdout', 'stderr'):
                logfile = output / ('compiler.' + key + '.log')
                receipt['logs'][key] = {'path': str(logfile), 'state': checked_state(logfile, errors, key + ' log')}
            if receipt['logs']['stderr']['state']:
                diagnostic = pinned_bytes(output / 'compiler.stderr.log',
                                          receipt['logs']['stderr']['state'], prefix_bytes=65536).decode('utf-8', errors='replace')
                version = re.search(r'(?m)^(?:Apple )?clang version [^\r\n]+', diagnostic)
                receipt['compiler']['version'] = version.group(0) if version else None
            receipt['output']['state'] = checked_state(plan['output_path'], errors, 'output dylib')
            if receipt['output']['state']:
                require(receipt['output']['state']['size'] > 0, 'Compiler output is empty')
            require(directory_state(output, private=True) == identity, 'Output directory changed')
        except (Exception, KeyboardInterrupt) as error:
            errors.append(type(error).__name__ + ': ' + str(error))
        process = receipt['process']
        if process['timed_out']:
            receipt['outcome'] = 'compiler_timeout'
        elif process['returncode'] not in (None, 0):
            receipt['outcome'] = 'compiler_failure'
        elif process['error']:
            receipt['outcome'] = 'process_error'
        elif receipt['setup_error']:
            receipt['outcome'] = 'setup_error'
        elif receipt['cleanup']['errors'] or not receipt['cleanup']['reaped']:
            receipt['outcome'] = 'cleanup_failure'
        elif errors:
            receipt['outcome'] = 'integrity_failure'
        elif receipt['compiler']['version'] is None:
            receipt['outcome'] = 'compiler_version_unavailable'
        else:
            receipt['outcome'] = 'built_unqualified'
        try:
            write_new(directory_fd, 'build-receipt.json',
                      (json.dumps(receipt, indent=2, allow_nan=False) + '\n').encode('utf-8'), mode=0o400)
        except (OSError, ValueError) as error:
            receipt['receipt_write_error'] = type(error).__name__ + ': ' + str(error)
        finally:
            os.close(directory_fd)
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', default='cache/window-presentation-build')
    parser.add_argument('--compiler', default='/usr/bin/clang')
    parser.add_argument('--sdk', help='absolute SDK selector; canonical root and metadata are recorded')
    parser.add_argument('--timeout-seconds', type=float, default=120)
    parser.add_argument('--execute', action='store_true', help='explicitly start one macOS compiler driver')
    args = parser.parse_args(argv)
    try:
        plan = plan_build(args.output, args.compiler, args.timeout_seconds, args.sdk)
        result = execute_build(plan) if args.execute else plan
    except (OSError, ValueError) as error:
        print(json.dumps({'error': str(error), 'qualifications': QUALIFICATIONS}), file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, allow_nan=False))
    return 0 if result['mode'] == 'plan' or (
        result['outcome'] == 'built_unqualified' and result['receipt_write_error'] is None
    ) else 1


if __name__ == '__main__':
    raise SystemExit(main())
