"""A small GDB remote-protocol client for QEMU's stub inside the VM container.

The stub listens on the container's loopback; a perl relay started with
`docker exec -i` carries the bytes. Only reads, hardware breakpoints,
write watchpoints, continue, step and detach are sent: nothing writes guest
memory or registers.
"""
import os
import select
import struct
import subprocess
import time

RELAY = r'''use IO::Socket::INET; use IO::Select; use Socket qw(IPPROTO_TCP TCP_NODELAY);
my $s=IO::Socket::INET->new(PeerAddr=>"127.0.0.1",PeerPort=>shift,Proto=>"tcp") or die $!;
setsockopt($s,IPPROTO_TCP,TCP_NODELAY,pack("i",1)) or die $!;
binmode STDIN; binmode STDOUT; $|=1;
my $sel=IO::Select->new(\*STDIN,$s);
while (1) { for my $fh ($sel->can_read) {
my $n=sysread($fh,my $buf,65536); exit unless $n;
my $dst=fileno($fh)==fileno(STDIN)?$s:\*STDOUT;
my $off=0; while($off<$n) { my $w=syswrite($dst,$buf,$n-$off,$off); die $! unless $w; $off+=$w; }
}}'''

class ProtocolError(RuntimeError):
    """A reply the client cannot trust, or a request it must not send. These
    checks are explicit (not asserts) so they hold under `python -O` too."""


def check(ok, *what):
    if not ok:
        raise ProtocolError(*what)


REGS = ['eax', 'ecx', 'edx', 'ebx', 'esp', 'ebp', 'esi', 'edi', 'eip', 'eflags', 'cs', 'ss', 'ds', 'es', 'fs', 'gs']


class Remote:
    def __init__(self, container, port=1234):
        self.container = container
        self.p = subprocess.Popen(['docker', 'exec', '-i', container, 'perl', '-e', RELAY, str(port)],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        self.pending = bytearray()
        self.stopped = None
        self.packets = 0

    def send(self, payload):
        allowed = ('m', 'Z1,', 'z1,', 'Z2,', 'z2,', 'qXfer:features:read:', 'p')
        check(payload in ('?', 'g', 'c', 's', 'D', 'qSupported') or payload.startswith(allowed), 'refused', payload)
        data = payload.encode()
        self.p.stdin.write(b'$' + data + b'#' + f'{sum(data) & 255:02x}'.encode())
        self.packets += 1

    def _byte(self, deadline):
        while not self.pending:
            left = deadline - time.monotonic() if deadline else 1
            if deadline and left <= 0:
                raise TimeoutError('no reply from the GDB stub')
            if select.select([self.p.stdout], [], [], min(1, max(0, left)))[0]:
                data = os.read(self.p.stdout.fileno(), 65536)
                if not data:
                    raise RuntimeError('relay ended: ' + self.p.stderr.read().decode())
                self.pending.extend(data)
        return self.pending.pop(0)

    def receive(self, timeout=60):
        deadline = time.monotonic() + timeout if timeout else None
        while self._byte(deadline) != ord('$'):
            pass
        wire = bytearray()
        while True:
            b = self._byte(deadline)
            if b == ord('#'):
                break
            wire.append(b)
        digits = bytes([self._byte(deadline), self._byte(deadline)])
        try:
            expected = int(digits, 16)
        except ValueError:
            raise ProtocolError('bad checksum digits', digits) from None
        if sum(wire) & 255 != expected:
            raise ProtocolError(f'checksum {sum(wire) & 255:02x} != {expected:02x}', bytes(wire[:64]))
        self.p.stdin.write(b'+')
        out = bytearray()
        i = 0
        while i < len(wire):
            b = wire[i]
            i += 1
            if b == ord('}'):
                out.append(wire[i] ^ 32)
                i += 1
            elif b == ord('*'):
                out.extend(bytes([out[-1]]) * (wire[i] - 29))
                i += 1
            else:
                out.append(b)
        return out.decode('latin-1')

    def command(self, s, timeout=60):
        self.send(s)
        r = self.receive(timeout)
        while s != '?' and r[:3] in ('T02', 'T05', 'S02', 'S05'):
            r = self.receive(timeout)
        return r

    def stop_reply(self):
        r = self.command('?')
        check(r[:1] in ('T', 'S'), r)
        self.stopped = True
        return r

    def interrupt(self):
        self.p.stdin.write(b'\x03')
        r = self.receive()
        self.stopped = True
        return r

    def resume(self):
        self.send('c')
        self.stopped = False

    def step(self):
        """One instruction (QEMU masks interrupts and timers while it steps)."""
        self.send('s')
        r = self.receive()
        self.stopped = True
        return r

    def wait(self, timeout=None):
        """Wait for the guest to stop (a breakpoint). timeout None waits forever."""
        r = self.receive(timeout)
        self.stopped = True
        return r

    def registers(self):
        data = bytes.fromhex(self.command('g'))
        regs = dict(zip(REGS, struct.unpack('<16I', data[:64])))
        regs['_raw_len'] = len(data)
        return regs, data

    def read(self, address, n):
        out = bytearray()
        while n:
            count = min(n, 2048)
            r = self.command(f'm{address:x},{count:x}')
            if r.startswith('E'):
                raise MemoryError(f'read {address:#x}+{count:#x}: {r}')
            part = bytes.fromhex(r)
            if len(part) != count:
                raise MemoryError(f'short read {address:#x}')
            out.extend(part)
            address += count
            n -= count
        return bytes(out)

    def set_break(self, address):
        r = self.command(f'Z1,{address:x},1')
        check(r == 'OK', hex(address), r)

    def clear_break(self, address):
        r = self.command(f'z1,{address:x},1')
        check(r in ('OK', 'E22'), hex(address), r)

    def set_watch(self, address, size):
        """A write watchpoint (GDB's Z2). The guest stops after the writing instruction; the stop
        reply holds `watch:<address>`."""
        r = self.command(f'Z2,{address:x},{size:x}')
        check(r == 'OK', hex(address), size, r)

    def clear_watch(self, address, size):
        r = self.command(f'z2,{address:x},{size:x}')
        check(r in ('OK', 'E22'), hex(address), size, r)

    def detach(self):
        try:
            if not self.stopped:
                self.interrupt()
            self.command('D')
        finally:
            self.p.terminate()
            self.p.wait(timeout=10)
