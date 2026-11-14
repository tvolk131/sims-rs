"""Tests for capture.py, guest.py and rsp.py that need no Docker and no guest.

Run with: python3 -B -m unittest discover -s tools/harness -p 'test_*.py'

capture.main runs against a fake GDB stub (FakeRemote) and fake guest memory;
the cases come from a review of the harness (the defects it found, fixed).
"""
import io
import json
import os
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import capture  # noqa: E402
import guest  # noqa: E402
import rsp  # noqa: E402

GAME_CR3, OTHER_CR3 = 0x111000, 0x222000
IDENTITY = {'Sims.exe': '3' * 64, 'version.dll': '2' * 64}


class FakeRemote:
    """A stub that stops at the given stops in turn. A stop is a dict: pc,
    and optionally cr3, reply (the stop reply), animator (person 259's
    animator pointer while stopped there), and step (the reply to stepping
    past it, with step_cr3 the CR3 after the step)."""

    def __init__(self, stops):
        self.stops = iter(stops)
        self.current = next(self.stops)
        self.stopped = True
        self.packets = 0
        self.watch_calls = []
        self.cleared_watches = []

    def stop_reply(self):
        return 'T05'

    def clear_break(self, a):
        pass

    def set_break(self, a):
        pass

    def set_watch(self, a, size):
        self.watch_calls.append((a, size))

    def clear_watch(self, a, size):
        self.cleared_watches.append((a, size))

    def resume(self):
        self.stopped = False

    def wait(self):
        self.current = next(self.stops)
        self.stopped = True
        return self.current.get('reply', 'T05')

    def registers(self):
        regs = dict(eip=self.current['pc'], ecx=0x20000, ebx=0x20000, ebp=0x90000, esp=0x90010)
        raw = bytearray(88)
        struct.pack_into('<I', raw, 64, 0xa0000)
        struct.pack_into('<I', raw, 84, self.current.get('cr3', GAME_CR3))
        return regs, raw

    def step(self):
        reply = self.current.get('step', 'T05')
        if 'step_cr3' in self.current:
            self.current = dict(self.current, cr3=self.current['step_cr3'])
        return reply

    def detach(self):
        pass

    def interrupt(self):
        self.stopped = True


class FakeMemory:
    """Lot 5's simulator at 0x20000; one object, person 259, at 0x50000."""

    def __init__(self, remote):
        self.remote = remote

    def lot(self):
        return 5

    def word(self, p):
        words = {0x20000 + 0xb0: 0x30000, 0x30000 + 0x18: 0x40000, 0x30000 + 0x1c: 0x40004, 0x40000: 0x50000,
                 0x50000 + 0x118: 0x60000, 0x50000 + 0xadc: self.remote.current.get('animator', 0x70000),
                 0x20000 + 0x98: 100}
        return words.get(p, 0)

    def words(self, p, n, width=4):
        return [self.word(p + 4 * i) for i in range(n)]

    def half(self, p):
        return 2

    def object_id(self, p):
        return 259 if p == 0x50000 else 0

    def byte(self, p):
        return 1

    def read(self, p, n):
        return bytes(n)

    def summary(self, sim, teb):
        return dict(tick=100, lot_rng=1, global_rng=2, crt=3, scheduler_cursor=0)

    def snapshot(self, kind, regs, sim, teb):
        return dict(tick=100), {}, {}


class CaptureTest(unittest.TestCase):
    def run_capture(self, stops, extra):
        temporary = tempfile.TemporaryDirectory(prefix='sims-capture-test-')
        self.addCleanup(temporary.cleanup)
        output = Path(temporary.name) / 'capture'
        remote = FakeRemote(stops)
        dump_calls = []
        with patch.object(capture, 'Remote', return_value=remote), \
                patch.object(capture, 'Memory', FakeMemory), \
                patch.object(capture, 'game_identity', return_value=IDENTITY), \
                patch.object(capture, 'dump', side_effect=lambda *a: dump_calls.append(a) or 0), \
                patch.object(sys, 'argv', ['capture.py', '--output', str(output), '--full', '', *extra]):
            capture.main()
        return remote, output, dump_calls

    def rows(self, path):
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def events(self, output, kind):
        return [r for r in self.rows(output / 'events.jsonl') if r['kind'] == kind]

    def test_a_write_by_another_process_is_not_the_games(self):
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY),
            dict(pc=0x123456, cr3=OTHER_CR3, reply='T05watch:70088;'),
            dict(pc=0x123456, reply='T05watch:70088;'),
            dict(pc=capture.RETURN)], ['--updates', '1', '--watch', 'direction=0x70088'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual(len(rows), 1)  # only the game's own write
        self.assertEqual(rows[0]['registers']['cr3'], GAME_CR3)
        foreign = self.events(output, 'foreign_watch')
        self.assertEqual([(r['address'], r['cr3']) for r in foreign], [(0x70088, OTHER_CR3)])

    def test_a_write_found_by_stepping_is_checked_too(self):
        # A breakpoint hit in another process; the instruction stepped over writes the watched address.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.DIALOG_SHOW, cr3=OTHER_CR3, step='T05watch:70088;'),
            dict(pc=capture.DIALOG_SHOW, step='T05watch:70088;'),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--watch', 'direction=0x70088'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([r['registers']['cr3'] for r in rows], [GAME_CR3])
        self.assertEqual(len(self.events(output, 'foreign_watch')), 1)

    def test_a_pointer_watch_follows_a_new_animator(self):
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY, animator=0x80000), dict(pc=capture.RETURN, animator=0x80000),
            dict(pc=capture.ENTRY, animator=0), dict(pc=capture.RETURN, animator=0x90000)],
            ['--updates', '3', '--watch', 'direction=person:259@0xadc+0x88'])
        self.assertEqual(remote.watch_calls, [(0x70088, 4), (0x80088, 4), (0x90088, 4)])
        self.assertEqual(remote.cleared_watches[:2], [(0x70088, 4), (0x80088, 4)])
        moved = self.events(output, 'watch_moved')
        self.assertEqual([(r['old_address'], r['address']) for r in moved], [(0x70088, 0x80088)])
        lost = self.events(output, 'watch_lost')
        self.assertEqual([r['address'] for r in lost], [0x80088])  # no animator: it waits again
        armed = self.events(output, 'watch_armed')
        self.assertEqual([r['address'] for r in armed], [0x70088, 0x90088])

    def test_a_fixed_address_watch_is_not_moved(self):
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN, animator=0x80000)],
            ['--updates', '1', '--watch', 'x=0x70088'])
        self.assertEqual(remote.watch_calls, [(0x70088, 4)])
        self.assertEqual(self.events(output, 'watch_moved'), [])

    def test_dump_1_is_saved_at_the_baseline(self):
        remote, output, calls = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '1', '--dump', '1', '--guest-dir', '/tmp/synthetic-guest'])
        self.assertEqual([c[2].name for c in calls], ['dump-0001'])
        self.assertEqual([r['update'] for r in self.events(output, 'dump')], [1])

    def test_later_dumps_still_work(self):
        remote, output, calls = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--dump', '2', '--guest-dir', '/tmp/synthetic-guest'])
        self.assertEqual([c[2].name for c in calls], ['dump-0002'])

    def test_the_verified_identity_is_recorded(self):
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN)], ['--updates', '1'])
        status = json.loads((output / 'status.json').read_text())
        self.assertEqual(status['game_identity'], IDENTITY)
        self.assertEqual(self.events(output, 'game_identity')[0]['verified'], True)

    def test_identity_is_read_from_the_share(self):
        ok = SimpleNamespace(returncode=0, stdout=f'{"3" * 64}  Sims.exe\n{"2" * 64}  version.dll\n')
        with patch.object(capture.subprocess, 'run', return_value=ok) as run:
            self.assertEqual(capture.game_identity('sims-harness-input'), IDENTITY)
        self.assertEqual(run.call_args[0][0][:3], ['docker', 'exec', 'sims-harness-input'])
        missing = SimpleNamespace(returncode=1, stdout='')
        with patch.object(capture.subprocess, 'run', return_value=missing):
            self.assertIsNone(capture.game_identity('sims-harness-input'))


class PagesTest(unittest.TestCase):
    def test_an_animator_ending_on_a_page_boundary(self):
        class PageRemote:
            def read(self, p, n):
                return bytes(n)

        class SnapshotMemory(capture.Memory):
            def word(self, p):
                return {0x20000 + 0xb0: 0x30000, 0x30000 + 0x18: 0x40000, 0x30000 + 0x1c: 0x40004,
                        0x30000 + 0x24: 0x45000, 0x40000: 0x50000, 0x50000 + 0xadc: 0x70d60}.get(p, 0)

            def words(self, p, n, width=4):
                return [0] * n

            def half(self, p):
                return 0

            def lot(self):
                return 5

            def crt(self, p):
                return {}

            def object(self, p, f):
                self.read(p, 0xae0)
                return dict(id=259, pointer=p, person={})

        m = SnapshotMemory(PageRemote())
        with patch.object(capture.qemu_retained_owners, 'capture', return_value={}), \
                patch.object(capture.neighborhood_owners, 'capture', return_value={}):
            state, full, pages = m.snapshot('test', dict(eip=capture.RETURN), 0x20000, 0xa0000)
        # The animator (0x2a0 bytes at 0x70d60) ends at 0x71000, so it lies on page 0x70000 only.
        self.assertEqual(sorted(int(k) for k in pages), [0x50000, 0x70000])

    def test_span(self):
        m = capture.Memory(SimpleNamespace(read=lambda p, n: bytes(n)))
        self.assertEqual(m.span(0x70d60, 0x2a0), {0x70000})
        self.assertEqual(m.span(0x70d60, 0x2a1), {0x70000, 0x71000})
        self.assertEqual(m.span(0x70000, 0x1000), {0x70000})


class RspTest(unittest.TestCase):
    def remote(self, wire):
        r = rsp.Remote.__new__(rsp.Remote)
        r.p = SimpleNamespace(stdin=io.BytesIO())
        r.pending = bytearray(wire)
        r.packets = 0
        return r

    def test_a_bad_checksum_is_refused(self):
        with self.assertRaises(rsp.ProtocolError):
            self.remote(b'$0123#00').read(0x10000, 2)  # the checksum is c6

    def test_a_good_checksum_is_read(self):
        self.assertEqual(self.remote(b'$0123#c6').read(0x10000, 2), b'\x01\x23')

    def test_a_write_is_refused(self):
        with self.assertRaises(rsp.ProtocolError):
            self.remote(b'').send('M10000,1:00')

    def test_the_checks_hold_under_optimize(self):
        code = ('import io, sys; from types import SimpleNamespace; import rsp\n'
                'r = rsp.Remote.__new__(rsp.Remote); r.p = SimpleNamespace(stdin=io.BytesIO()); r.packets = 0\n'
                'r.pending = bytearray(b"$0123#00")\n'
                'try:\n    r.read(0x10000, 2)\n    sys.exit("checksum accepted")\nexcept rsp.ProtocolError:\n    pass\n'
                'try:\n    r.send("M10000,1:00")\n    sys.exit("write sent")\nexcept rsp.ProtocolError:\n    pass\n')
        r = subprocess.run([sys.executable, '-O', '-B', '-c', code], cwd=os.path.dirname(os.path.abspath(__file__)),
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr + r.stdout)


class GuestTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='sims-guest-test-')
        self.addCleanup(temporary.cleanup)
        self.share = Path(temporary.name)

    def test_concurrent_jobs_get_their_own_numbers(self):
        cmd = self.share / 'cmd'
        cmd.mkdir()
        (cmd / '00001.done').write_text('0\n')  # left without its .sh: the guest would skip number 1
        (cmd / '00002.tmp').write_text('')  # left by a submitter that died
        numbers, barrier = [], threading.Barrier(16)

        def submit(i):
            barrier.wait()
            numbers.append((guest.submit(self.share, f'echo {i}\n'), i))

        threads = [threading.Thread(target=submit, args=(i,)) for i in range(16)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(len({n for n, _ in numbers}), 16)
        self.assertTrue(all(n > 2 for n, _ in numbers))
        for n, i in numbers:
            self.assertEqual((cmd / f'{n:05d}.sh').read_text(), f'echo {i}\n')
        self.assertEqual(sorted(p.name for p in cmd.glob('*.tmp')), ['00002.tmp'])

    def main(self, *argv, env=None):
        calls = []

        def run(share, script, timeout=600):
            calls.append(script)
            return self.code, 'out\n'

        with patch.object(guest, 'run', side_effect=run), patch.object(sys, 'argv', ['guest.py', str(self.share), *argv]), \
                patch.dict(os.environ, env or {}), patch.object(sys, 'stdout', io.StringIO()):
            with self.assertRaises(SystemExit) as caught:
                guest.main()
        return caught.exception.code, calls

    def test_click_and_move_pass_on_the_exit_code(self):
        self.code = 1
        self.assertEqual(self.main('click', '10', '20')[0], 1)
        self.assertEqual(self.main('move', '10', '20')[0], 1)
        self.code = 0
        self.assertEqual(self.main('click', '10', '20')[0], 0)

    def test_bad_values_never_reach_the_guest(self):
        self.code = 0
        for argv, env in ((('click', '10;reboot', '20'), {}), (('move', '10', '20'), {'SIMS_WINDOW': '0x1; reboot'}),
                          (('click', '10', '20', '1;reboot'), {}), (('shot', 'x.png'), {'SIMS_WINDOW': '$(reboot)'})):
            code, calls = self.main(*argv, env=env)
            self.assertIn('bad', str(code))
            self.assertEqual(calls, [])

    def test_click_stops_when_the_pointer_cannot_be_placed(self):
        self.code = 0
        _, (script,) = self.main('click', '10', '20', '0')
        bin_dir = self.share / 'bin'
        bin_dir.mkdir()
        log = self.share / 'xdotool.log'
        fake = bin_dir / 'xdotool'
        fake.write_text(f'#!/bin/sh\necho "$@" >> "{log}"\n[ "$1" != mousemove ]\n')
        fake.chmod(fake.stat().st_mode | stat.S_IEXEC)
        r = subprocess.run(['sh', '-c', script], env=dict(os.environ, PATH=f'{bin_dir}{os.pathsep}{os.environ["PATH"]}'),
                           timeout=30)
        self.assertNotEqual(r.returncode, 0)
        self.assertEqual(log.read_text().split('\n')[0].split()[0], 'mousemove')
        self.assertNotIn('mousedown', log.read_text())


if __name__ == '__main__':
    unittest.main()
