"""Unqualified, fail-closed owned-group census prototype for the source builder.

The SDK ps(1) documents -g selection, Z states, and non-exact snapshots;
compat(5) documents COMMAND_MODE. Neither promises census completeness.
Execution stays disabled pending parent-owned platform/completeness witnesses.
There is no escaped-descendant containment or fallback to native libraries.
"""

import os
import re
import select
import signal
import subprocess
import sys
import threading
import time


PS_PATH = '/bin/ps'
PS_ENVIRONMENT = {'PATH': '/usr/bin:/bin', 'LANG': 'C', 'LC_ALL': 'C',
                  'COMMAND_MODE': 'unix2003'}
CENSUS_WITNESSED = False
MAX_OUTPUT_BYTES = 65536
MAX_ROWS = 1024
INSPECTION_SECONDS = 0.5
AUXILIARY_REAP_SECONDS = 0.5
POLL_SECONDS = 0.05
MAX_CENSUSES = 128
RETAINED_GROUPS = {}
RETAINED_GROUP_HANDLES = {}
RETAINED_AUXILIARIES = {}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def describe_error(error):
    return type(error).__name__ + ': ' + str(error)


def backend_plan(state_reader):
    identity = state_reader(PS_PATH, executable=True)
    require(identity['uid'] == 0 and identity['mode'] & 0o022 == 0,
            'The pinned /bin/ps must be root-owned and not group/world writable')
    return {'path': PS_PATH, 'before': identity, 'environment': dict(PS_ENVIRONMENT),
            'selection': '-g OWNED_PGID -o pid=,pgid=,uid=,state=',
            'max_output_bytes': MAX_OUTPUT_BYTES, 'max_rows': MAX_ROWS,
            'inspection_seconds': INSPECTION_SECONDS,
            'auxiliary_reap_seconds': AUXILIARY_REAP_SECONDS,
            'status': 'unsupported_until_parent_witness',
            'census_completeness_qualified': False, 'escaped_descendants_contained': False}


def require_wait_owner(process=None):
    require(sys.platform == 'darwin', 'Owned-group inspection is macOS-only')
    require(all(hasattr(os, name) for name in (
        'waitid', 'P_PID', 'WEXITED', 'WNOWAIT', 'WNOHANG',
        'CLD_EXITED', 'CLD_KILLED', 'CLD_DUMPED')),
        'Unsupported: standard os.waitid/WNOWAIT child identity is unavailable')
    require(callable(os.waitid), 'Unsupported: standard os.waitid is not callable')
    require(threading.active_count() == 1, 'Exclusive single-threaded wait ownership is required')
    require(signal.getsignal(signal.SIGCHLD) == signal.SIG_DFL,
            'Exclusive wait ownership needs default SIGCHLD')
    if process is not None:
        require(type(process.pid) is int and process.pid > 1, 'Invalid owned group leader')
        require(process.returncode is None, 'Owned leader was already reaped; identity unavailable')


def require_execution_support():
    require_wait_owner()
    require(CENSUS_WITNESSED,
            'Unsupported: /bin/ps census completeness/platform behavior has no parent witness')


def child_identity(process, uid):
    require_wait_owner(process)
    observation = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOWAIT | os.WNOHANG)
    if observation is None or observation.si_pid == 0:
        return {'pid': process.pid, 'exited': False, 'returncode': None}
    require(observation.si_pid == process.pid and observation.si_uid == uid,
            'Waitable child identity/UID mismatch')
    require(observation.si_code in (os.CLD_EXITED, os.CLD_KILLED, os.CLD_DUMPED),
            'Unknown waitable child status')
    status = observation.si_status
    require(type(status) is int and status >= 0, 'Invalid waitable child status')
    require(observation.si_code != os.CLD_EXITED or status <= 255, 'Invalid child exit code')
    require(observation.si_code == os.CLD_EXITED or 0 < status < signal.NSIG,
            'Invalid child termination signal')
    return {'pid': process.pid, 'exited': True,
            'returncode': status if observation.si_code == os.CLD_EXITED else -status}


def inspect_group(backend, pgid, state_reader, timeout=INSPECTION_SECONDS):
    evidence = {'command': [PS_PATH, '-g', str(pgid), '-o', 'pid=,pgid=,uid=,state='],
                'environment': dict(PS_ENVIRONMENT), 'before': None, 'after': None,
                'pid': None, 'returncode': None, 'reaped': False, 'kill': None,
                'stdout': '', 'stderr': '', 'truncated': False, 'errors': [],
                'auxiliary_errors': [], 'retained': False}
    process = None
    try:
        require_execution_support()
        require(type(pgid) is int and pgid > 1, 'Invalid owned PGID')
        require(0 < timeout <= INSPECTION_SECONDS, 'Invalid inspection deadline')
        require(not RETAINED_AUXILIARIES, 'Unresolved auxiliary prevents inspection')
        require(backend['path'] == PS_PATH and backend['environment'] == PS_ENVIRONMENT,
                'Inspector plan changed')
        evidence['before'] = state_reader(PS_PATH, executable=True)
        require(evidence['before'] == backend['before'], 'Inspector changed before launch')
        deadline = time.monotonic() + timeout
        process = subprocess.Popen(
            evidence['command'], cwd='/', env=dict(PS_ENVIRONMENT), stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
            close_fds=True, shell=False,
        )
        evidence['pid'] = process.pid
        buffers = {'stdout': bytearray(), 'stderr': bytearray()}
        streams = {process.stdout: 'stdout', process.stderr: 'stderr'}
        while streams:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired(evidence['command'], timeout)
            readable, unused_write, unused_error = select.select(list(streams), [], [], remaining)
            for stream in readable:
                total = sum(len(data) for data in buffers.values())
                chunk = os.read(stream.fileno(), min(4096, MAX_OUTPUT_BYTES - total + 1))
                if not chunk:
                    del streams[stream]
                    continue
                buffers[streams[stream]].extend(chunk[:MAX_OUTPUT_BYTES - total])
                for name, data in buffers.items():
                    evidence[name] = data.decode('ascii', errors='backslashreplace')
                if total + len(chunk) > MAX_OUTPUT_BYTES:
                    evidence['truncated'] = True
                    raise ValueError('Inspector output truncated at bounded capture')
                chunk.decode('ascii')
        remaining = deadline - time.monotonic()
        require(remaining > 0, 'Inspector exhausted deadline before reap')
        evidence['returncode'] = process.wait(timeout=remaining)
        evidence['reaped'] = True
        require(evidence['returncode'] == 0, 'Inspector returned nonzero')
        require(not evidence['stderr'], 'Inspector diagnostic/denial is not a census')
    except (Exception, KeyboardInterrupt) as error:
        evidence['errors'].append(describe_error(error))
    finally:
        if process is not None:
            if not evidence['reaped']:
                try:
                    process.kill()
                    evidence['kill'] = 'sent_to_owned_handle'
                except (Exception, KeyboardInterrupt) as error:
                    evidence['kill'] = 'error'
                    evidence['auxiliary_errors'].append('kill: ' + describe_error(error))
                try:
                    evidence['returncode'] = process.wait(timeout=AUXILIARY_REAP_SECONDS)
                    evidence['reaped'] = True
                except (Exception, KeyboardInterrupt) as error:
                    evidence['auxiliary_errors'].append('reap: ' + describe_error(error))
                    evidence['retained'] = True
                    RETAINED_AUXILIARIES[process.pid] = process
            for stream in (process.stdout, process.stderr):
                try:
                    stream.close()
                except (Exception, KeyboardInterrupt) as error:
                    evidence['auxiliary_errors'].append('close: ' + describe_error(error))
        try:
            evidence['after'] = state_reader(PS_PATH, executable=True)
            require(evidence['after'] == backend['before'], 'Inspector changed after inspection')
        except (Exception, KeyboardInterrupt) as error:
            evidence['errors'].append(describe_error(error))
    return evidence


def parse_census(evidence, pgid, uid):
    require(not evidence['errors'] and not evidence['auxiliary_errors'] and evidence['reaped']
            and evidence['returncode'] == 0 and not evidence['truncated'] and not evidence['stderr'],
            'Inspector did not supply a clean, bounded census')
    output = evidence['stdout']
    require(output and output.endswith('\n') and '\r' not in output,
            'Empty or incomplete census')
    lines = output.splitlines()
    require(0 < len(lines) <= MAX_ROWS, 'Census row bound exceeded')
    rows = []
    seen = set()
    for line in lines:
        require(len(line) <= 128, 'Census line bound exceeded')
        match = re.fullmatch(r' *([0-9]+) +([0-9]+) +([0-9]+) +([IRSTUZ][+<>AELNSsVWX]*) *', line)
        require(match is not None, 'Malformed, denied, or unknown census state')
        pid, group, owner = (int(value) for value in match.groups()[:3])
        require(1 < pid <= 2147483647 and pid not in seen, 'Invalid or duplicate census PID')
        require(group == pgid and owner == uid, 'Foreign group or UID in census')
        seen.add(pid)
        rows.append({'pid': pid, 'pgid': group, 'uid': owner, 'state': match[4]})
    require(pgid in seen, 'Unreaped leader anchor is absent from census')
    return sorted(rows, key=lambda row: row['pid'])


def empty_cleanup(attempted=False):
    return {'attempted': attempted, 'term': None, 'kill': None, 'reaped': False,
            'returncode': None, 'waits': [], 'errors': [], 'inspection_errors': [],
            'censuses': [], 'signals': [], 'no_live_members': False,
            'retained': False, 'identity': None,
            'scope': 'Owned PGID only; escaped descendants are not contained; census unqualified'}


def cleanup_group(process, backend, state_reader, grace_seconds=2):
    cleanup = empty_cleanup(attempted=True)
    uid = os.getuid()
    RETAINED_GROUPS[process.pid] = process

    def census(deadline):
        snapshot = {'before': None, 'after': None, 'inspection': None, 'rows': None, 'error': None}
        require(len(cleanup['censuses']) < MAX_CENSUSES, 'Cleanup census budget exhausted')
        cleanup['censuses'].append(snapshot)
        try:
            snapshot['before'] = child_identity(process, uid)
            remaining = deadline - time.monotonic()
            require(remaining > 0, 'Group census deadline exhausted')
            snapshot['inspection'] = inspect_group(backend, process.pid, state_reader,
                                                   min(INSPECTION_SECONDS, remaining))
            snapshot['rows'] = parse_census(snapshot['inspection'], process.pid, uid)
            snapshot['after'] = child_identity(process, uid)
            require(snapshot['before'] == snapshot['after'], 'Child identity changed across census')
            anchor = next(row for row in snapshot['rows'] if row['pid'] == process.pid)
            require((anchor['state'][0] == 'Z') == snapshot['after']['exited'],
                    'Leader census disagrees with waitable child identity')
            cleanup['identity'] = snapshot['after']
            return snapshot['rows']
        except (Exception, KeyboardInterrupt) as error:
            snapshot['error'] = describe_error(error)
            cleanup['inspection_errors'].append(snapshot['error'])
            if snapshot['inspection'] is not None:
                cleanup['inspection_errors'].extend(snapshot['inspection']['errors'])
                cleanup['inspection_errors'].extend(snapshot['inspection']['auxiliary_errors'])
            raise

    def no_live(rows, deadline):
        if any(row['state'][0] != 'Z' for row in rows):
            return False
        confirmed = census(deadline)
        if rows != confirmed:
            cleanup['inspection_errors'].append('Inconsistent terminal censuses')
            cleanup['censuses'][-1]['error'] = 'Inconsistent terminal censuses'
            raise ValueError('Inconsistent terminal censuses')
        cleanup['no_live_members'] = True
        return True

    def send(name, signum):
        entry = {'signal': signum, 'pgid': process.pid, 'identity': None, 'error': None}
        cleanup['signals'].append(entry)
        try:
            entry['identity'] = child_identity(process, uid)
            require(entry['identity'] == cleanup['identity'], 'Identity changed before group signal')
            os.killpg(process.pid, signum)
            cleanup[name] = 'sent'
        except (Exception, KeyboardInterrupt) as error:
            cleanup[name] = 'error'
            entry['error'] = describe_error(error)
            cleanup['errors'].append(name + ': ' + entry['error'])
            if isinstance(error, (ChildProcessError, ValueError)):
                raise

    def grace(name, deadline):
        for unused in range(MAX_CENSUSES):
            if time.monotonic() >= deadline:
                cleanup['waits'].append({'after': name, 'no_live_members': False, 'error': 'deadline'})
                return False
            rows = census(deadline)
            if no_live(rows, deadline):
                cleanup['waits'].append({'after': name, 'no_live_members': True, 'error': None})
                return True
            time.sleep(min(POLL_SECONDS, max(0, deadline - time.monotonic())))
        raise ValueError('Group grace census budget exhausted')

    try:
        require_execution_support()
        require(0 < grace_seconds <= 2, 'Invalid group grace bound')
        deadline = time.monotonic() + grace_seconds
        rows = census(deadline)
        if not no_live(rows, deadline):
            send('term', signal.SIGTERM)
            if not grace('term', time.monotonic() + grace_seconds):
                deadline = time.monotonic() + grace_seconds
                rows = census(deadline)
                if not no_live(rows, deadline):
                    send('kill', signal.SIGKILL)
                    require(grace('kill', time.monotonic() + grace_seconds),
                            'Live group members remain after KILL grace')
        require(cleanup['no_live_members'], 'No independent no-live-members evidence')
        identity = child_identity(process, uid)
        require(identity['exited'] and identity == cleanup['identity'], 'Leader is not waitable at reap')
        cleanup['returncode'] = process.wait(timeout=grace_seconds)
        cleanup['reaped'] = True
        RETAINED_GROUPS.pop(process.pid, None)
        require(cleanup['returncode'] == identity['returncode'], 'Reaped child status changed')
    except (Exception, KeyboardInterrupt) as error:
        cleanup['errors'].append(describe_error(error))
    cleanup['retained'] = not cleanup['reaped']
    return cleanup
