"""Bounded descriptor observations, not adversarial locking or durable capture.

Deadlines are checked between operations; no per-syscall time bound is claimed.
Future appends and collector callbacks lost behind Finalize are outside this
observation. Raw bytes and semantic parsing deliberately have separate outcomes.
"""

import hashlib
import os
import stat
import time


CONTRACT = 'sims-window-journal-final-observation-v1'
MAX_BYTES = 64 * 1024**2
RESERVE_BYTES = 6 * 1024**3
CHUNK_BYTES = 64 * 1024
SNAPSHOT_NAME = 'journal-final-observed.raw'


def fingerprint(info):
    return {name: getattr(info, 'st_' + attribute) for name, attribute in (
        ('device', 'dev'), ('inode', 'ino'), ('mode', 'mode'), ('links', 'nlink'),
        ('uid', 'uid'), ('size', 'size'), ('mtime_ns', 'mtime_ns'), ('ctime_ns', 'ctime_ns'))}


def identity(value):
    return {name: value[name] for name in ('device', 'inode', 'mode', 'uid')}


def require(condition, message):
    if not condition:
        raise ValueError(message)


class JournalObservation:
    def __init__(self, directory, journal, *, deadline, clock=time, limit=MAX_BYTES):
        self.directory, self.path = str(directory), str(journal)
        for value in (self.directory, self.path):
            require(value.startswith('/') and '\\' not in value and '\0' not in value
                    and all(part not in ('', '.', '..') for part in value.split('/')[1:]),
                    'Explicit normalized absolute observation paths required')
        require(self.path.startswith(self.directory + '/'), 'Journal outside explicitly owned output')
        require(type(limit) is int and 0 <= limit <= MAX_BYTES, 'Invalid observation byte bound')
        self.deadline, self.clock, self.limit = deadline, clock, limit
        self.earlier, self.previous_size = {}, 0

    def check(self):
        if self.clock.monotonic() >= self.deadline:
            raise TimeoutError('Final journal observation deadline exceeded')

    def _open(self, target):
        opened, edges = [], []
        flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | getattr(os, 'O_CLOEXEC', 0)
        try:
            self.check()
            parent = os.open('/', flags | os.O_DIRECTORY)
            opened.append(parent)
            root = fingerprint(os.fstat(parent))
            require(stat.S_ISDIR(root['mode']), 'Unsafe root directory')
            edges.append((None, '/', parent, root, '/'))
            current = ''
            for component in target.split('/')[1:]:
                self.check()
                current += '/' + component
                leaf = current == self.path
                before = fingerprint(os.stat(component, dir_fd=parent, follow_symlinks=False))
                require((stat.S_ISREG(before['mode']) and before['links'] == 1) if leaf
                        else stat.S_ISDIR(before['mode']), 'Unsafe journal path component')
                child = os.open(component, flags | (0 if leaf else os.O_DIRECTORY), dir_fd=parent)
                opened.append(child)
                self.check()
                require(before == fingerprint(os.fstat(child)), 'Journal component raced during open')
                edges.append((parent, component, child, before, current))
                parent = child
            self._validate(edges)
            return opened, edges
        except BaseException:
            for descriptor in reversed(opened):
                os.close(descriptor)
            raise

    def _validate(self, edges, *, exact=True):
        for parent, name, descriptor, before, absolute in reversed(edges):
            self.check()
            descriptor_state = fingerprint(os.fstat(descriptor))
            path_state = fingerprint(os.stat(name, dir_fd=parent, follow_symlinks=False))
            require(descriptor_state == path_state, 'Journal descriptor/path substitution')
            require((descriptor_state == before) if exact else
                    (identity(descriptor_state) == identity(before)), 'Journal component fingerprint changed')
            require(absolute not in self.earlier or identity(descriptor_state) == self.earlier[absolute],
                    'Original journal ancestor or leaf identity changed')
        self.check()

    def remember(self, *, directory_only=False):
        opened, edges = self._open(self.directory if directory_only else self.path)
        try:
            self._validate(edges)
            for _, _, _, state, absolute in edges:
                self.earlier.setdefault(absolute, identity(state))
            if not directory_only:
                require(edges[-1][3]['size'] >= self.previous_size,
                        'Journal truncated since previous inspection')
                self.previous_size = edges[-1][3]['size']
            self.check()
        finally:
            for descriptor in reversed(opened):
                os.close(descriptor)
        self.check()

    def observe(self, *, child_collected, forced_teardown, retained=b''):
        result = {'contract': CONTRACT, 'status': 'failed', 'child_collected': child_collected,
                  'forced_teardown': forced_teardown,
                  'source': {'output_directory': self.directory, 'path': self.path},
                  'limit_bytes': self.limit, 'complete': False, 'eof': False,
                  'bytes_read': 0, 'prefix_sha256': hashlib.sha256(b'').hexdigest(),
                  'size_bytes': None, 'wholefile_sha256': None,
                  'components_before': [], 'components_after': [],
                  'earlier_components': [{'path': name, 'identity': value.copy()}
                                         for name, value in self.earlier.items()],
                  'previous_size': self.previous_size,
                  'retained_buffer': {'bytes': len(retained), 'sha256': hashlib.sha256(retained).hexdigest(),
                                      'prefix_matches': None},
                  'semantic': {'status': 'not_attempted', 'records': None, 'error': None},
                  'snapshot': {'status': 'not_requested', 'path': None, 'bytes_written': 0,
                               'prefix_sha256': hashlib.sha256(b'').hexdigest(),
                               'sha256': None, 'fingerprint': None, 'error': None}, 'error': None}
        opened, chunks, hasher = [], [], hashlib.sha256()
        try:
            self.check()
            require(type(child_collected) is bool and type(forced_teardown) is bool,
                    'Explicit lifecycle booleans required')
            require(child_collected and not forced_teardown,
                    'Final journal requires collected child without forced teardown')
            require(self.directory in self.earlier, 'Owned directory was not observed before collection')
            opened, edges = self._open(self.path)
            result['components_before'] = [{'path': absolute, 'fingerprint': state.copy()}
                                           for _, _, _, state, absolute in edges]
            size = edges[-1][3]['size']
            require(size >= self.previous_size, 'Journal truncated since previous inspection')
            require(size <= self.limit, 'Journal exceeds byte bound')
            self.check()
            require(os.lseek(opened[-1], 0, os.SEEK_SET) == 0, 'Independent journal offset unavailable')
            while True:
                self.check()
                chunk = os.read(opened[-1], min(CHUNK_BYTES, self.limit - result['bytes_read'] + 1))
                if not chunk:
                    result['eof'] = True
                    break
                chunks.append(chunk)
                result['bytes_read'] += len(chunk)
                hasher.update(chunk)
                self.check()
                require(result['bytes_read'] <= self.limit, 'Journal exceeds byte bound sentinel')
            self._validate(edges)
            require(result['bytes_read'] == size, 'Journal byte count changed during final read')
            result['components_after'] = [{'path': absolute, 'fingerprint': fingerprint(os.fstat(descriptor))}
                                          for _, _, descriptor, _, absolute in edges]
            self._validate(edges)
            self.check()
            result.update(complete=True, size_bytes=size, wholefile_sha256=hasher.hexdigest(), status='observed')
        except Exception as error:
            self.fail(result, error)
        finally:
            for descriptor in reversed(opened):
                try:
                    os.close(descriptor)
                except OSError as error:
                    self.fail(result, error)
        raw = b''.join(chunks)
        result['prefix_sha256'] = hasher.hexdigest()
        result['retained_buffer']['prefix_matches'] = raw.startswith(retained) if result['complete'] else None
        try:
            self.check()
            if result['complete']:
                require(result['retained_buffer']['prefix_matches'],
                        'Journal truncated or replaced its previous bytes')
        except Exception as error:
            self.fail(result, error)
        return result, raw

    @staticmethod
    def fail(result, error):
        message = str(error) or type(error).__name__
        result.update(status='failed', complete=False, size_bytes=None, wholefile_sha256=None)
        result['error'] = message if result['error'] is None else result['error'] + '; ' + message

    def parse(self, result, raw, parser):
        try:
            self.check()
            require(result['complete'], 'Incomplete raw observation cannot be parsed as whole journal')
            records = parser(raw)
            self.check()
            require(type(records) is list, 'Expected parsed journal record list')
            result['semantic'].update(status='parsed', records=len(records))
            return records
        except Exception as error:
            result['semantic'].update(status='failed', error=str(error) or type(error).__name__)
            if isinstance(error, TimeoutError):
                self.fail(result, error)
            return None

    def snapshot(self, result, raw):
        snapshot = result['snapshot']
        opened, descriptor, hasher = [], None, hashlib.sha256()
        try:
            self.check()
            opened, edges = self._open(self.directory)
            space = os.fstatvfs(opened[-1])
            require(space.f_bavail * space.f_frsize >= RESERVE_BYTES + len(raw) + 1024**2,
                    'Six GiB free reserve required for raw snapshot')
            self.check()
            snapshot['path'] = self.directory + '/' + SNAPSHOT_NAME
            descriptor = os.open(SNAPSHOT_NAME, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
                                 | os.O_NONBLOCK | getattr(os, 'O_CLOEXEC', 0), 0o600, dir_fd=opened[-1])
            before = fingerprint(os.fstat(descriptor))
            require(stat.S_ISREG(before['mode']) and before['links'] == 1
                    and stat.S_IMODE(before['mode']) == 0o600, 'Unsafe raw snapshot')
            while snapshot['bytes_written'] < len(raw):
                self.check()
                start = snapshot['bytes_written']
                chunk = raw[start:start + CHUNK_BYTES]
                count = os.write(descriptor, chunk)
                require(0 < count <= len(chunk), 'Short raw snapshot write made no progress')
                hasher.update(chunk[:count])
                snapshot['bytes_written'] += count
                self.check()
            after = fingerprint(os.fstat(descriptor))
            require(after == fingerprint(os.stat(SNAPSHOT_NAME, dir_fd=opened[-1], follow_symlinks=False))
                    and identity(after) == identity(before) and after['links'] == 1
                    and after['size'] == len(raw), 'Raw snapshot identity or byte count changed')
            self._validate(edges, exact=False)
            space = os.fstatvfs(opened[-1])
            require(space.f_bavail * space.f_frsize >= RESERVE_BYTES,
                    'Six GiB free reserve exhausted during raw snapshot')
            self.check()
            snapshot.update(status='written', sha256=hasher.hexdigest(), fingerprint=after)
        except Exception as error:
            snapshot.update(status='failed', sha256=None, error=str(error) or type(error).__name__)
            if isinstance(error, TimeoutError):
                self.fail(result, error)
        finally:
            snapshot['prefix_sha256'] = hasher.hexdigest()
            for item in ([descriptor] if descriptor is not None else []) + list(reversed(opened)):
                try:
                    os.close(item)
                except OSError as error:
                    snapshot.update(status='failed', sha256=None, error=str(error) or type(error).__name__)
        try:
            self.check()
        except Exception as error:
            snapshot.update(status='failed', sha256=None, error=str(error) or type(error).__name__)
            self.fail(result, error)

