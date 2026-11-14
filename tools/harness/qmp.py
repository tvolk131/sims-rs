"""A small QMP client for QEMU inside the VM container, used for memory dumps.

QMP listens on the container's loopback (start_vm.sh); a perl relay started
with `docker exec -i` carries the bytes, as for the GDB stub (rsp.py). Only
`info mem` (through human-monitor-command) and `memsave` are sent: both read
the guest, neither writes it. `memsave` reads guest virtual memory through
the page tables the CPU holds, so a dump taken while the GDB stub holds the
guest stopped in the game's process reads that process.
"""
import json
import os
import select
import subprocess
import time

from rsp import RELAY


class Qmp:
    def __init__(self, container, port=4444):
        self.p = subprocess.Popen(['docker', 'exec', '-i', container, 'perl', '-e', RELAY, str(port)],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        self.buf = b''
        self._line()  # the greeting
        self.call('qmp_capabilities')

    def _line(self, timeout=120):
        deadline = time.monotonic() + timeout
        while b'\n' not in self.buf:
            left = deadline - time.monotonic()
            if left <= 0:
                raise TimeoutError('no reply from QMP')
            if select.select([self.p.stdout], [], [], left)[0]:
                data = os.read(self.p.stdout.fileno(), 65536)
                if not data:
                    raise RuntimeError('QMP relay ended: ' + self.p.stderr.read().decode())
                self.buf += data
        line, _, self.buf = self.buf.partition(b'\n')
        return json.loads(line)

    def call(self, command, timeout=600, **arguments):
        assert command in ('qmp_capabilities', 'human-monitor-command', 'memsave'), command
        if command == 'human-monitor-command':
            assert arguments['command-line'] in ('info mem',), arguments
        msg = {'execute': command}
        if arguments:
            msg['arguments'] = arguments
        self.p.stdin.write(json.dumps(msg).encode() + b'\r\n')
        while True:
            reply = self._line(timeout)
            if 'event' in reply:
                continue
            if 'error' in reply:
                raise RuntimeError(f'{command}: {reply["error"]}')
            return reply['return']

    def user_ranges(self):
        """The present user-mode ranges of the current address space, from `info mem`:
        [(start, size, protection)], merged where adjacent."""
        text = self.call('human-monitor-command', **{'command-line': 'info mem'})
        out = []
        for line in text.splitlines():
            parts = line.split()
            if len(parts) != 3 or '-' not in parts[0]:
                continue
            a, b = (int(x, 16) for x in parts[0].split('-'))
            prot = parts[2]
            if not prot.startswith('u') or a >= 0xc0000000:
                continue
            if out and out[-1][0] + out[-1][1] == a and out[-1][2] == prot:
                out[-1] = (out[-1][0], out[-1][1] + b - a, prot)
            else:
                out.append((a, b - a, prot))
        return out

    def close(self):
        self.p.terminate()
        self.p.wait(timeout=10)
