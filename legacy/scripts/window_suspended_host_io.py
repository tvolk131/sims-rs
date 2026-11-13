"""Owned-family file/FIFO bookkeeping. No process, signal, wait or native binding.

This does not authenticate a hostile same-UID actor or provide atomic exclusion
against rename-and-restore attacks. All operations stay under one admitted root.
"""
import hashlib
import json
import os
from pathlib import Path
import stat

MAX_RECORD_BYTES = 4 * 1024**2
MAX_JOURNAL_BYTES = 32 * 1024**2
MAX_EVENTS = 160
MAX_COMMANDS = 64


def identity(s):
    return dict(device=s.st_dev, inode=s.st_ino, uid=s.st_uid,
                mode=stat.S_IMODE(s.st_mode))


def encode(value):
    remaining = [160000]
    byte_budget = [MAX_RECORD_BYTES]
    def cost(count):
        byte_budget[0] -= count
        if byte_budget[0] < 0: raise ValueError('Evidence byte budget exhausted')
    def walk(v, depth):
        remaining[0] -= 1
        if remaining[0] < 0 or depth > 24:
            raise ValueError('Bounded evidence tree required')
        if type(v) in (bool, type(None)):
            cost(5); return v
        if type(v) is int and -(1 << 127) <= v < (1 << 128):
            cost(40); return v
        if type(v) is str and len(v) <= 16384:
            cost(len(json.dumps(v, ensure_ascii=True))); return v
        if type(v) is bytes and len(v) <= 4096:
            cost(2 * len(v) + 64)
            return dict(encoding='hex', bytes=len(v), hex=v.hex())
        if type(v) in (tuple, list) and len(v) <= 4096:
            cost(2 + len(v))
            return [walk(item, depth + 1) for item in v]
        if type(v) is dict and len(v) <= 4096 and all(type(k) is str and len(k) <= 256 for k in v):
            cost(2 + 2 * len(v) + sum(len(json.dumps(k, ensure_ascii=True)) for k in v))
            return {k: walk(item, depth + 1) for k, item in v.items()}
        raise ValueError('Bounded exact evidence data required')
    raw = (json.dumps(walk(value, 0), sort_keys=True, separators=(',', ':')) + '\n').encode('ascii')
    if len(raw) > MAX_RECORD_BYTES:
        raise ValueError('Evidence record exceeds byte cap')
    return raw


class FamilyRoot:
    def __init__(self, path):
        self.path = Path(path)
        if self.path.parent != Path('/private/tmp') or not self.path.name.startswith('sims-suspended-true-'):
            raise ValueError('Fresh private diagnostic root required')
        s = self.path.lstat()
        if not stat.S_ISDIR(s.st_mode) or s.st_uid != os.getuid() or stat.S_IMODE(s.st_mode) != 0o700:
            raise ValueError('Owned mode0700 diagnostic directory required')
        self.admitted = identity(s)
        self.fd = os.open(str(self.path), os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        try:
            self.validate()
        except BaseException:
            os.close(self.fd)
            raise

    def validate(self):
        s = os.fstat(self.fd)
        if not stat.S_ISDIR(s.st_mode) or identity(s) != self.admitted or identity(self.path.lstat()) != self.admitted:
            raise ValueError('Diagnostic root identity changed')

    @staticmethod
    def name(name):
        if type(name) is not str or not name or '/' in name or name in ('.', '..') or len(name) > 128:
            raise ValueError('One exact child filename required')
        return name

    def read(self, name, pin):
        self.validate(); name = self.name(name)
        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=self.fd)
        try:
            before = os.fstat(fd)
            if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_uid != os.getuid()
                    or stat.S_IMODE(before.st_mode) != pin['mode'] or before.st_size != pin['bytes']
                    or not 0 <= before.st_size <= MAX_RECORD_BYTES):
                raise ValueError('Pinned regular single-link file required')
            chunks, remaining = [], before.st_size + 1
            while remaining:
                chunk = os.read(fd, min(65536, remaining))
                if not chunk: break
                chunks.append(chunk); remaining -= len(chunk)
            raw = b''.join(chunks); after = os.fstat(fd)
            if (identity(before) != identity(after) or before.st_size != after.st_size or
                    before.st_mtime_ns != after.st_mtime_ns or before.st_ctime_ns != after.st_ctime_ns or
                    len(raw) != pin['bytes'] or hashlib.sha256(raw).hexdigest() != pin['sha256']):
                raise ValueError('Pinned file changed during read')
            return raw
        finally:
            os.close(fd)

    def close(self):
        os.close(self.fd)


class Journal:
    def __init__(self, root):
        self.root = root
        self.sequence = 0
        self.bytes = 0
        self.first_error = None
        self.errors = 0

    def append(self, event):
        if self.sequence >= MAX_EVENTS:
            raise ValueError('Journal event budget exhausted')
        self.sequence += 1  # Reserve a new name even if a partial write fails.
        raw = encode(dict(sequence=self.sequence, event=event))
        if len(raw) > MAX_JOURNAL_BYTES - self.bytes:
            raise ValueError('Journal total byte budget exhausted')
        self.bytes += len(raw)  # No retry or reclaimed budget after failure.
        name = 'event-%04d.json' % self.sequence
        fd = None
        try:
            self.root.validate()
            fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                         0o600, dir_fd=self.root.fd)
            position = 0
            while position < len(raw):
                count = os.write(fd, raw[position:])
                if count <= 0: raise OSError('Incomplete journal write')
                position += count
            os.fchmod(fd, 0o400)
            os.fsync(fd)
            closing = fd; fd = None
            os.close(closing)  # Never repeat a close whose return is uncertain.
            os.fsync(self.root.fd)
            return dict(file=name, bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest())
        except BaseException as error:
            self.errors += 1
            if self.first_error is None: self.first_error = error
            raise
        finally:
            if fd is not None: os.close(fd)


class Fifo:
    def __init__(self, root, pin):
        root.validate()
        self.fd = os.open('control.fifo', os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=root.fd)
        try:
            s = os.fstat(self.fd)
            if (not stat.S_ISFIFO(s.st_mode) or s.st_nlink != 1 or stat.S_IMODE(s.st_mode) != 0o600
                    or s.st_uid != os.getuid() or identity(s) != pin):
                raise ValueError('Exact owned control FIFO required')
        except BaseException:
            os.close(self.fd)
            raise

    def read(self):
        # O_RDWR retains an owned writer, so loss of a controller is not EOF.
        return os.read(self.fd, 128)

    def close(self):
        os.close(self.fd)


class Commands:
    def __init__(self, token):
        if type(token) is not str or len(token) != 32 or any(c not in '0123456789abcdef' for c in token):
            raise ValueError('Exact fresh family token required')
        self.allowed = {(token + ' ' + c).encode('ascii'): c for c in ('RUN', 'ADVANCE', 'READ', 'CACHED', 'EXIT')}
        self.buffer = bytearray()
        self.overflow = False

    def feed(self, chunk):
        if type(chunk) is not bytes or len(chunk) > 128:
            raise ValueError('One bounded command chunk required')
        result = []
        for value in chunk:
            if value == 10:
                result.append(None if self.overflow else self.allowed.get(bytes(self.buffer)))
                self.buffer.clear(); self.overflow = False
            elif not self.overflow:
                if len(self.buffer) >= 127:
                    self.buffer.clear(); self.overflow = True
                else:
                    self.buffer.append(value)
        return tuple(result)
