"""Tests for capture.py, guest.py and rsp.py that need no Docker and no guest.

Run with: python3 -B -m unittest discover -s tools/harness -p 'test_*.py'

capture.main runs against a fake GDB stub (FakeRemote) and fake guest memory;
the cases come from a review of the harness (the defects it found, fixed).
"""
import io
import argparse
import json
import os
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
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
    past it, with step_pc and step_cr3 the registers after the step).
    pending_traps counts traps that can interrupt a step or resume without
    executing an instruction. The attach query itself leaves them pending."""

    def __init__(self, stops):
        self.stops = iter(stops)
        self.current = next(self.stops)
        self.stopped = True
        self.packets = 0
        self.watch_calls = []
        self.cleared_watches = []
        self.watch_stops = []  # (set or clear, the pc stopped at)
        self.memory_reads = []  # (pc, address)
        self.step_calls = []  # (pc, whether its breakpoint is armed)
        self.armed = set()  # the breakpoints set
        self.resumed_armed = []  # the breakpoints set at each resume

    def stop_reply(self):
        return self.current.get('attach_reply', 'T05')

    def clear_break(self, a):
        self.armed.discard(a)

    def set_break(self, a):
        self.armed.add(a)

    def set_watch(self, a, size):
        self.watch_calls.append((a, size))
        self.watch_stops.append(('set', self.current['pc']))

    def clear_watch(self, a, size):
        self.cleared_watches.append((a, size))
        self.watch_stops.append(('clear', self.current['pc']))

    def resume(self):
        self.resumed_armed.append(set(self.armed))
        self.stopped = False

    def wait(self):
        self.stopped = True
        if self.current.get('pending_traps'):
            return self.pending_trap()
        self.current = next(self.stops)
        return self.current.get('reply', 'T05')

    def pending_trap(self):
        self.current = dict(self.current, pending_traps=self.current['pending_traps'] - 1)
        return self.current.get('pending_reply', 'T05')

    def registers(self):
        regs = dict(eip=self.current['pc'], ecx=0x20000, ebx=0x20000, ebp=0x90000, esp=0x90010)
        raw = bytearray(88)
        struct.pack_into('<I', raw, 64, 0xa0000)
        struct.pack_into('<I', raw, 84, self.current.get('cr3', GAME_CR3))
        return regs, raw

    def step(self):
        pc = self.current['pc']
        self.step_calls.append((pc, pc in self.armed))
        if self.current.get('pending_traps'):
            return self.pending_trap()
        reply = self.current.get('step', 'T05')
        self.current = dict(self.current, pc=self.current.get('step_pc', pc + 1),
                            cr3=self.current.get('step_cr3', self.current.get('cr3', GAME_CR3)))
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
        if self.remote.current.get('lot_unreadable'):
            raise MemoryError('the lot')
        return self.remote.current.get('lot', 5)

    def word(self, p):
        words = {0x20000 + 0xb0: 0x30000, 0x30000 + 0x18: 0x40000, 0x30000 + 0x1c: 0x40004, 0x40000: 0x50000,
                 0x50000 + 0x118: 0x60000, 0x50000 + 0xadc: self.remote.current.get('animator', 0x70000),
                 0x20000 + 0x98: 100, 0x90010 + 4: self.remote.current.get('result', 0)}
        return words.get(p, 0)

    def words(self, p, n, width=4):
        return [self.word(p + 4 * i) for i in range(n)]

    def half(self, p):
        return 2

    def object_id(self, p):
        ids = self.remote.current.get('ids', {0x50000: 259})
        return ids.get(p, 0)

    def byte(self, p):
        return 1

    def read(self, p, n):
        self.remote.memory_reads.append((self.remote.current['pc'], p))
        if p == self.remote.current.get('unreadable'):
            raise MemoryError(f'read {p:#x}')
        # The bytes this stop has written (`bytes`, by address); every other byte is 0.
        written = self.remote.current.get('bytes', {})
        return bytes(written.get(p + i, 0) for i in range(n))

    def summary(self, sim, teb):
        return dict(tick=100, lot_rng=1, global_rng=2, crt=3, scheduler_cursor=0)

    def clock(self, sim):
        return list(self.remote.current.get('clock', [6, 10, 0])), [16, 1, 21, 6, 1997]

    def registry(self, sim):
        return list(self.remote.current.get('objects', [0x50000]))

    def snapshot(self, kind, regs, sim, teb):
        return dict(tick=100), {}, {}


class CaptureTest(unittest.TestCase):
    def run_capture(self, stops, extra, prepare=None):
        temporary = tempfile.TemporaryDirectory(prefix='sims-capture-test-')
        self.addCleanup(temporary.cleanup)
        output = Path(temporary.name) / 'capture'
        if prepare:
            output.mkdir()
            prepare(output)
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

    def test_a_late_watch_is_set_only_for_its_updates(self):
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY), dict(pc=0x123456, reply='T05watch:70088;'), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.RETURN)],  # the entry is no breakpoint once the watch has ended
            ['--updates', '4', '--late-watch', '2-3,state=0x70088'])
        self.assertEqual(remote.watch_calls, [(0x70088, 4)])
        self.assertEqual(remote.cleared_watches, [(0x70088, 4)])
        armed = self.events(output, 'watch_armed')
        self.assertEqual([r['update'] for r in armed], [1])  # at the end of update 1
        ended = self.events(output, 'watch_ended')
        self.assertEqual([r['update'] for r in ended], [3])  # at the end of update 3
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual(len(rows), 1)

    def test_a_late_watch_is_set_and_cleared_at_the_ends_of_updates(self):
        remote, _, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.RETURN)],  # the entry is no breakpoint once the watch has ended
            ['--updates', '3', '--late-watch', '2-2,state=0x70088'])
        # Set at the end of update 1, cleared at the end of update 2.
        self.assertEqual(remote.watch_stops, [('set', capture.RETURN), ('clear', capture.RETURN)])

    def test_a_late_watch_keeps_the_entry_breakpoint_only_while_it_is_set(self):
        remote, _, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.RETURN),  # update 2: the entry is not a breakpoint
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.RETURN)],  # update 4: nor here
            ['--updates', '4', '--late-watch', '3-3,state=0x70088'])
        entry = [capture.ENTRY in armed for armed in remote.resumed_armed]
        # Set for the baseline and not after it, set again at the end of
        # update 2 for the watch, and cleared with it at the end of update 3.
        self.assertEqual(entry, [True, False, False, True, True, False])
        self.assertEqual(remote.watch_stops, [('set', capture.RETURN), ('clear', capture.RETURN)])

    def test_a_late_watch_from_update_1_waits_for_the_game(self):
        # A dialog stop before the baseline does not set it.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.DIALOG_SHOW, cr3=OTHER_CR3),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '1', '--late-watch', '1,state=0x70088'])
        kinds = [r['kind'] for r in self.rows(output / 'events.jsonl')]
        self.assertLess(kinds.index('baseline'), kinds.index('watch_armed'))
        self.assertEqual(remote.watch_stops[0], ('set', capture.ENTRY))

    def test_a_late_watch_already_over_is_not_set_on_resume(self):
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text(''.join(
                json.dumps(dict(update=n, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n'
                for n in (1, 2, 3)))
        remote, output, _ = self.run_capture(
            [dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '4', '--resume', '--late-watch', '2-3,state=0x70088'], prepare=prepare)
        self.assertEqual(remote.watch_calls, [])
        self.assertEqual(self.events(output, 'watch_armed'), [])
        self.assertEqual(self.events(output, 'watch_ended'), [])

    def test_a_run_taken_over_off_a_breakpoint_sets_a_late_watch_there(self):
        # The earlier run stopped after stepping past update 1's return.
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text(
                json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n')
        remote, output, _ = self.run_capture(
            [dict(pc=0x123456), dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--resume', '--late-watch', '2-2,state=0x70088'], prepare=prepare)
        self.assertEqual(remote.watch_stops, [('set', 0x123456), ('clear', capture.RETURN)])

    def test_a_write_seen_before_a_taken_over_run_knows_where_it_is_has_no_update(self):
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text(
                json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n')
        remote, output, _ = self.run_capture(
            [dict(pc=0x123456), dict(pc=0x123457, reply='T05watch:70088;'),
             dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--resume', '--late-watch', '2-2,state=0x70088'], prepare=prepare)
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['update'], r['after_update'], r['in_update']) for r in rows], [(None, 1, None)])

    def test_a_moving_late_watch_is_not_moved_at_the_stop_where_it_ends(self):
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN, animator=0x80000),
            dict(pc=capture.RETURN, animator=0x80000)],
            ['--updates', '3', '--late-watch', '2-2,direction=person:259@0xadc+0x88'])
        self.assertEqual(remote.watch_calls, [(0x70088, 4)])
        self.assertEqual(self.events(output, 'watch_moved'), [])
        ended = self.events(output, 'watch_ended')
        self.assertEqual([(r['update'], r['set']) for r in ended], [(2, True)])

    def test_a_late_watch_never_set_ends_saying_so(self):
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY, animator=0), dict(pc=capture.RETURN, animator=0),
            dict(pc=capture.ENTRY, animator=0), dict(pc=capture.RETURN, animator=0),
            dict(pc=capture.RETURN, animator=0)],
            ['--updates', '3', '--late-watch', '2-2,direction=person:259@0xadc+0x88'])
        self.assertEqual(remote.watch_calls, [])
        ended = self.events(output, 'watch_ended')
        self.assertEqual([(r['update'], r['set']) for r in ended], [(2, False)])

    def test_a_late_watch_set_by_the_run_taken_over_ends_as_set(self):
        # The earlier run sets it at the end of update 1 and is stopped
        # there; this one starts in another process and next stops at
        # update 2's return, where it ends. Once from the events the earlier
        # run wrote, once from an earlier revision's, which had no `late` or
        # lot and does not count.
        _, first, _ = self.run_capture(
            [dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '1', '--late-watch', '2-2,state=0x70088'])
        armed = self.events(first, 'watch_armed')
        self.assertEqual([(r['name'], r['late']) for r in armed], [('state', True)])
        for old in (False, True):
            def prepare(output):
                for name in ('events.jsonl', 'updates.jsonl', 'status.json'):
                    text = (first / name).read_text()
                    if old and name == 'events.jsonl':
                        rows = [json.loads(line) for line in text.splitlines()]
                        for r in rows:
                            r.pop('late', None)
                            r.pop('lot', None)
                        text = ''.join(json.dumps(r) + '\n' for r in rows)
                    (output / name).write_text(text)
            remote, output, _ = self.run_capture(
                [dict(pc=0x123456, cr3=OTHER_CR3), dict(pc=capture.RETURN)],
                ['--updates', '2', '--resume', '--late-watch', '2-2,state=0x70088'], prepare=prepare)
            self.assertEqual(remote.watch_calls, [])
            ended = self.events(output, 'watch_ended')
            # An earlier revision's arming is not counted.
            self.assertEqual([(r['update'], r['set']) for r in ended], [(2, not old)], old)

    def test_only_an_arming_on_the_lot_within_its_updates_counts_on_resume(self):
        # Each alone would count but for one test: before the baseline
        # (another lot), with another text under the same name, below and
        # above the watch's updates, not a late watch, and on another lot.
        armed = lambda expr, update, **more: dict(dict(kind='watch_armed', name='state', expr=expr, late=True,
                                                       lot=5, address=0x70088, value=0, update=update), **more)
        legacy = {k: v for k, v in armed('2-2,state=0x70088', 1).items() if k not in ('late', 'lot')}
        cases = [
            [armed('2-2,state=0x70088', 1), dict(kind='baseline', cr3=GAME_CR3)],
            [dict(kind='baseline', cr3=GAME_CR3), armed('3-3,state=object:5', 1)],
            [dict(kind='baseline', cr3=GAME_CR3), armed('2-2,state=0x70088', 0)],
            [dict(kind='baseline', cr3=GAME_CR3), armed('2-2,state=0x70088', 2)],
            [dict(kind='baseline', cr3=GAME_CR3), armed('2-2,state=0x70088', 1, late=False)],
            [dict(kind='baseline', cr3=GAME_CR3), armed('2-2,state=0x70088', 1, lot=7)],
            # An earlier revision's, which recorded no lot (nor `late`).
            [dict(kind='baseline', cr3=GAME_CR3), legacy],
            # And one that counts.
            [dict(kind='baseline', cr3=GAME_CR3), armed('2-2,state=0x70088', 1)],
        ]
        for n, rows in enumerate(cases):
            def prepare(output):
                (output / 'events.jsonl').write_text(''.join(json.dumps(r) + '\n' for r in rows))
                (output / 'updates.jsonl').write_text(
                    json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n')
            remote, output, _ = self.run_capture(
                [dict(pc=0x123456, cr3=OTHER_CR3), dict(pc=capture.RETURN)],
                ['--updates', '2', '--resume', '--late-watch', '2-2,state=0x70088'], prepare=prepare)
            ended = self.events(output, 'watch_ended')
            self.assertEqual([(r['update'], r['set']) for r in ended], [(2, n == len(cases) - 1)], n)

    def test_a_late_watch_does_not_go_through_another_lots_simulator(self):
        # Taken over after update 1: a return on lot 7 gives the simulator's
        # pointer, then a dialog on lot 5 stops before the next entry. The
        # object watch is not worked out from lot 7's simulator there. Then
        # the same with an entry on lot 5 first, where it is set.
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text(
                json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n')
        for first, expected in ((dict(pc=capture.RETURN, lot=7), False), (dict(pc=capture.ENTRY), True)):
            remote, output, _ = self.run_capture(
                [first, dict(pc=capture.DIALOG_SHOW), dict(pc=capture.RETURN)],
                ['--updates', '2', '--resume', '--late-watch', '2-2,direction=person:259@0xadc+0x88'],
                prepare=prepare)
            self.assertEqual(bool(remote.watch_calls), expected)
            ended = self.events(output, 'watch_ended')
            self.assertEqual([(r['update'], r['set']) for r in ended], [(2, expected)])

    def test_a_write_after_a_return_elsewhere_has_no_tick_but_its_values(self):
        # Taken over after update 1: a return on lot 7, then a dialog on lot
        # 5 where the late watch is set, then a write before lot 5's next
        # entry: its tick is not read through lot 7's simulator. Then the
        # same with an entry on lot 5 first.
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text(
                json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n')
        for first, tick in ((dict(pc=capture.RETURN, lot=7), None), (dict(pc=capture.ENTRY), 100)):
            remote, output, _ = self.run_capture(
                [first, dict(pc=capture.DIALOG_SHOW), dict(pc=0x123456, reply='T05watch:70088;'),
                 dict(pc=capture.RETURN)],
                ['--updates', '2', '--resume', '--late-watch', '2-2,state=0x70088'], prepare=prepare)
            rows = self.rows(output / 'watch.jsonl')
            self.assertEqual([(r['tick'], 'before' in r, 'after' in r) for r in rows], [(tick, True, True)])

    def test_a_moving_late_watch_is_taken_off_on_another_lot(self):
        # Set at the end of update 1; the guest goes to lot 7 (an entry and
        # a return there), where it is taken off; back on lot 5 it is set
        # again at the next entry, and ends at update 2's return.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY, lot=7), dict(pc=capture.RETURN, lot=7),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--late-watch', '2-2,direction=person:259@0xadc+0x88'])
        armed = self.events(output, 'watch_armed')
        self.assertEqual([(r['lot'], r['update']) for r in armed], [(5, 1), (5, 1)])
        lost = self.events(output, 'watch_lost')
        self.assertEqual([r.get('elsewhere') for r in lost], [True])
        self.assertEqual(len(remote.watch_calls), 2)
        self.assertEqual(len(remote.cleared_watches), 2)
        ended = self.events(output, 'watch_ended')
        self.assertEqual([(r['update'], r['set']) for r in ended], [(2, True)])

    def test_a_write_on_another_lot_is_not_logged_under_a_moving_late_watch(self):
        # Set at the end of update 1; the guest goes to lot 7 and writes the
        # watched address before any breakpoint there. The watch is taken
        # off at that stop, and the row names no watch and says the lot.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=0x123456, lot=7, reply='T05watch:70088;'),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--late-watch', '2-2,direction=person:259@0xadc+0x88'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['name'], r['lot'], 'before' in r) for r in rows], [(None, 7, False)])
        lost = self.events(output, 'watch_lost')
        self.assertEqual([r.get('elsewhere') for r in lost], [True])
        ended = self.events(output, 'watch_ended')
        self.assertEqual([(r['update'], r['set']) for r in ended], [(2, True)])

    def test_a_write_on_another_lot_is_matched_to_the_watches_as_set(self):
        # An ordinary moving watch, set on lot 5 at person 259's animator
        # 0x70000; on lot 7 the animator is 0x80000 when 0x70088 is
        # written. The row names the watch as it was set, which is moved
        # only at the next breakpoint.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=0x123456, lot=7, animator=0x80000, reply='T05watch:70088;'),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--watch', 'direction=person:259@0xadc+0x88'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['name'], r['lot'], 'before' in r) for r in rows], [('direction', 7, True)])

    def test_after_a_write_on_another_lot_a_dialog_does_not_set_a_late_watch(self):
        # A moving late watch, taken off at a write on lot 7, is not set
        # again at a dialog on lot 5 through the simulator seen before:
        # a write there names no watch. The next entry on lot 5 sets it.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=0x123456, lot=7, reply='T05watch:70088;'),
            dict(pc=capture.DIALOG_SHOW), dict(pc=0x123456, reply='T05watch:70088;'),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--late-watch', '2-2,direction=person:259@0xadc+0x88'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['name'], r['lot']) for r in rows], [(None, 7), (None, 5)])
        armed = self.events(output, 'watch_armed')
        self.assertEqual([r['update'] for r in armed], [1, 1])
        ended = self.events(output, 'watch_ended')
        self.assertEqual([(r['update'], r['set']) for r in ended], [(2, True)])

    def test_after_a_dialog_on_another_lot_a_dialog_here_does_not_set_a_late_watch(self):
        # A moving late watch, taken off at a dialog on lot 7, is not set
        # again at a dialog on lot 5 before lot 5's next entry or return
        # (the simulator seen before may be lot 7's by then): a write there
        # names no watch. The next entry on lot 5 sets it.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.DIALOG_SHOW, lot=7),
            dict(pc=capture.DIALOG_SHOW), dict(pc=0x123456, reply='T05watch:70088;'),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--late-watch', '2-2,direction=person:259@0xadc+0x88'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['name'], r['lot'], r['tick']) for r in rows], [(None, 5, None)])
        lost = self.events(output, 'watch_lost')
        self.assertEqual([r.get('elsewhere') for r in lost], [True])
        armed = self.events(output, 'watch_armed')
        self.assertEqual([r['update'] for r in armed], [1, 1])
        ended = self.events(output, 'watch_ended')
        self.assertEqual([(r['update'], r['set']) for r in ended], [(2, True)])

    def test_a_write_on_another_lot_keeps_an_ordinary_watch_a_moving_late_watch_overlaps(self):
        # An ordinary 8-byte watch at 0x70088 and a moving late watch at
        # 0x7008c; on lot 7, 0x7008c is written, and the reply names the
        # late watch's watchpoint (the newer). The late watch is left out,
        # and the ordinary watch's value changed, so the row is its own,
        # with its values.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=0x123456, lot=7, reply='T05watch:7008c;', bytes={0x7008c: 1}),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--watch', 'pair=0x70088:8',
             '--late-watch', '2-2,direction=person:259@0xadc+0x8c'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['name'], r['lot'], 'before' in r) for r in rows], [('pair', 7, True)])

    def test_a_late_watch_waiting_is_not_set_through_another_lots_simulator(self):
        # Waiting at the end of update 1 (no animator yet), the watch sees
        # a dialog on lot 7, or on a lot that cannot be read, and then one
        # on lot 5 before lot 5's next entry or return: it is not set there
        # through the simulator seen before, only at the entry.
        for elsewhere in (dict(lot=7), dict(lot_unreadable=True)):
            remote, output, _ = self.run_capture([
                dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN, animator=0),
                dict(pc=capture.DIALOG_SHOW, **elsewhere), dict(pc=capture.DIALOG_SHOW),
                dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
                ['--updates', '2', '--late-watch', '2-2,direction=person:259@0xadc+0x88'])
            self.assertEqual(remote.watch_stops[0], ('set', capture.ENTRY), elsewhere)
            ended = self.events(output, 'watch_ended')
            self.assertEqual([(r['update'], r['set']) for r in ended], [(2, True)], elsewhere)

    def test_a_stop_whose_lot_cannot_be_read_takes_a_moving_late_watch_off(self):
        # Set at the end of update 1; a dialog in the game's process whose
        # lot cannot be read takes it off, and a write on lot 5 before its
        # next entry names no watch and has no tick. The entry sets it again.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.DIALOG_SHOW, lot_unreadable=True),
            dict(pc=0x123456, reply='T05watch:70088;'),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--late-watch', '2-2,direction=person:259@0xadc+0x88'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['name'], r['lot'], r['tick']) for r in rows], [(None, 5, None)])
        lost = self.events(output, 'watch_lost')
        self.assertEqual([r.get('elsewhere') for r in lost], [True])
        armed = self.events(output, 'watch_armed')
        self.assertEqual([r['update'] for r in armed], [1, 1])

    def test_a_run_taken_over_off_a_breakpoint_whose_lot_cannot_be_read_sets_an_ordinary_watch(self):
        # Taken over after update 1 off a breakpoint, in the game's process,
        # with the lot unreadable: the ordinary watch's address and value can
        # be read, so it is set there and the write before the entry is seen.
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text(
                json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n')
        remote, output, _ = self.run_capture(
            [dict(pc=0x123456, lot_unreadable=True), dict(pc=0x123457, reply='T05watch:70088;'),
             dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--resume', '--watch', 'state=0x70088'], prepare=prepare)
        self.assertEqual(remote.watch_stops[0], ('set', 0x123456))
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([r['name'] for r in rows], ['state'])

    def test_a_write_on_another_lot_is_not_logged_under_an_ordinary_watch_it_left_as_it_was(self):
        # An ordinary watch at 0x70086:4 and a moving 8-byte late watch at
        # 0x70088; on lot 7, 0x7008c is written, and the reply names the
        # late watch's watchpoint, 0x70088, which the ordinary one also
        # covers. The late watch is left out, and the ordinary one did not
        # change, so the row names no watch.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=0x123456, lot=7, reply='T05watch:70088;', bytes={0x7008c: 1}),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--watch', 'edge=0x70086:4',
             '--late-watch', '2-2,direction=person:259@0xadc+0x88:8'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['name'], r['lot']) for r in rows], [(None, 7)])

    def test_a_write_over_overlapping_watches_is_logged_under_each_that_changed(self):
        # An ordinary 8-byte watch at 0x70088 and a late watch at 0x7008c,
        # both set on lot 5; 0x7008c is written, and the reply names only
        # the newer watchpoint. The ordinary watch's upper half changed, so
        # it has a row too.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY), dict(pc=0x123456, reply='T05watch:7008c;', bytes={0x7008c: 1}),
            dict(pc=capture.RETURN)],
            ['--updates', '2', '--watch', 'pair=0x70088:8',
             '--late-watch', '2-2,direction=person:259@0xadc+0x8c'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['name'], r['before'], r['after']) for r in rows],
                         [('pair', '00' * 8, '00000000' + '01000000'), ('direction', '00' * 4, '01000000')])

    def test_a_watch_elsewhere_that_changed_without_a_stop_is_marked_only_as_changed(self):
        # Two ordinary watches apart, at 0x70088 and 0x70100; 0x70100
        # changed without a stop of its own, and 0x70088 is written. The
        # watch the reply names is named; the other's row says only that it
        # changed, not that this store wrote it, and its value is taken up,
        # so the change is logged once.
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY),
            dict(pc=0x123456, reply='T05watch:70088;', bytes={0x70088: 1, 0x70100: 1}),
            dict(pc=capture.RETURN)],
            ['--updates', '1', '--watch', 'state=0x70088', '--watch', 'far=0x70100'])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual([(r['name'], r['match']) for r in rows], [('state', 'named'), ('far', 'changed')])

    def watch_rows(self, steps, flags):
        """(name, match) for each watch row of a run with ENTRY and RETURN around the stops `steps`,
        over two updates; the late watches start at update 2, after the ordinary ones."""
        remote, output, _ = self.run_capture(
            [dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN)] + steps
            + [dict(pc=capture.ENTRY), dict(pc=capture.RETURN)], ['--updates', '2'] + flags)
        return remote, [(r['name'], r['match']) for r in self.rows(output / 'watch.jsonl')]

    def test_watchpoints_on_the_base_a_stop_gives_are_each_one_of(self):
        # low (4 bytes) then pair (8) at 0x70088: a stop giving 0x70088 may
        # be either's, whatever was written.
        remote, rows = self.watch_rows(
            [dict(pc=0x123456, reply='T05watch:70088;', bytes={0x7008c: 1})],
            ['--watch', 'low=0x70088:4', '--late-watch', '2-2,pair=0x70088:8'])
        self.assertEqual(remote.watch_calls, [(0x70088, 4), (0x70088, 8)])
        self.assertEqual(rows, [('low', 'one_of'), ('pair', 'one_of')])

    def test_a_watch_off_the_base_a_stop_gives_that_kept_its_value_has_no_row(self):
        # pair (0x70088:8) takes in high (0x7008c:4). A stop giving 0x7008c
        # need not have written high (an unaligned store is checked from its
        # page's base), so it does not show pair was written either.
        for reply, bytes_, named in [('T05watch:7008c;', {}, 'high'), ('T05watch:7008c;', {0x70091: 1}, 'high')]:
            remote, rows = self.watch_rows(
                [dict(pc=0x123456, reply=reply, bytes=bytes_)],
                ['--watch', 'pair=0x70088:8', '--late-watch', '2-2,high=0x7008c:4'])
            self.assertEqual(rows, [(named, 'named')])

    def test_every_watch_that_changed_has_a_row_wherever_it_is(self):
        # One instruction may be several stores, and a store crossing pages
        # is checked a page at a time, so a change anywhere may be this
        # store's: older or newer than the watchpoint that fired, beyond a
        # newer one, or on the next page. The row says only that it changed.
        cases = [
            # MOVUPS at 0x70088: two 8-byte stores; the first fires pair.
            (['--watch', 'pair=0x70088:8', '--late-watch', '2-2,next=0x70090:8'],
             'T05watch:70088;', range(0x70088, 0x70098), [('pair', 'named'), ('next', 'changed')]),
            # far (0x70094:4) beyond a newer mid (0x70090:4).
            (['--watch', 'far=0x70094:4', '--watch', 'low=0x70088:4', '--late-watch', '2-2,mid=0x70090:4'],
             'T05watch:70088;', [0x70088, 0x70094], [('far', 'changed'), ('low', 'named')]),
            # A dword store at 0x72ffe across the page.
            (['--watch', 'low=0x72ffe:2', '--late-watch', '2-2,high=0x73000:2'],
             'T05watch:72ffe;', range(0x72ffe, 0x73002), [('low', 'named'), ('high', 'changed')]),
        ]
        for flags, reply, written, expected in cases:
            remote, rows = self.watch_rows(
                [dict(pc=0x123456, reply=reply, bytes={a: 1 for a in written})], flags)
            self.assertEqual(rows, expected, flags)

    def test_a_stop_giving_no_watchpoints_base_names_the_watch_that_holds_it(self):
        remote, rows = self.watch_rows(
            [dict(pc=0x123456, reply='T05watch:7008a;')],
            ['--watch', 'pair=0x70088:8'])
        self.assertEqual(rows, [('pair', 'at')])

    def test_the_stop_for_a_step_that_hit_a_watchpoint_is_passed_over(self):
        # Stepping past the entry breakpoint writes the watched word: QEMU
        # gives the watch stop, then, at the next resume, a plain stop at
        # the instruction after.
        for thread in ('', 'thread:1;', 'thread:p1.1;'):
            with self.subTest(thread=thread):
                _, output, _ = self.run_capture([
                    dict(pc=0), dict(pc=capture.ENTRY, step='T05watch:70088;' + thread,
                                     step_pc=0x4100, bytes={0x70088: 1}),
                    dict(pc=0x4100, reply='T05' + thread), dict(pc=capture.RETURN)],
                    ['--updates', '1', '--watch', 'state=0x70088'])
                self.assertEqual([r['name'] for r in self.rows(output / 'watch.jsonl')], ['state'])
                self.assertEqual([r['pc'] for r in self.events(output, 'step_after_watch')], [0x4100])
                self.assertIsNone(json.loads((output / 'status.json').read_text())['stepped_to'])

    def test_a_plain_stop_with_a_different_thread_after_a_stepped_watch_is_unexpected(self):
        for stepped_thread, next_thread in (
            ('thread:1;', 'thread:2;'),
            ('thread:p1.1;', 'thread:p2.1;'),
            ('thread:p1.1;', 'thread:1;'),
            ('thread:p1.1;', ''),
        ):
            with self.subTest(stepped_thread=stepped_thread, next_thread=next_thread):
                with self.assertRaisesRegex(RuntimeError, 'unexpected stop at 0x4100'):
                    self.run_capture([
                        dict(pc=0), dict(pc=capture.ENTRY, step='T05watch:70088;' + stepped_thread,
                                         step_pc=0x4100),
                        dict(pc=0x4100, reply='T05' + next_thread)],
                        ['--updates', '1', '--watch', 'state=0x70088'])

    def test_a_plain_stop_elsewhere_after_a_step_that_hit_a_watchpoint_is_still_unexpected(self):
        with self.assertRaisesRegex(RuntimeError, 'unexpected stop at 0x123456'):
            self.run_capture([
                dict(pc=0), dict(pc=capture.ENTRY, step='T05watch:70088;', step_pc=0x4100),
                dict(pc=0x123456, reply='T05'), dict(pc=capture.RETURN)],
                ['--updates', '1', '--watch', 'state=0x70088'])

    def test_a_stop_after_a_stepped_watch_hit_is_passed_over_only_if_it_is_the_steps_own(self):
        # A pause (T02), a stop in another process, a second watch stop and
        # an armed breakpoint at the stepped-to pc are each dealt with as
        # usual.
        step = dict(pc=capture.ENTRY, step='T05watch:70088;', step_pc=0x4100, bytes={0x70088: 1})
        for stop, error in [
            (dict(pc=0x4100, reply='T02'), 'unexpected stop at 0x4100'),
            (dict(pc=0x4100, reply='T05', cr3=OTHER_CR3), 'unexpected stop at 0x4100'),
        ]:
            with self.assertRaisesRegex(RuntimeError, error):
                self.run_capture([dict(pc=0), step, stop, dict(pc=capture.RETURN)],
                                 ['--updates', '1', '--watch', 'state=0x70088'])
        remote, output, _ = self.run_capture([
            dict(pc=0), step, dict(pc=0x4100, reply='T05watch:70088;', bytes={0x70088: 2}),
            dict(pc=capture.RETURN)],
            ['--updates', '1', '--watch', 'state=0x70088'])
        self.assertEqual([r['name'] for r in self.rows(output / 'watch.jsonl')], ['state', 'state'])
        self.assertEqual(self.events(output, 'step_after_watch'), [])
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY, step='T05watch:70088;', step_pc=capture.RETURN,
                             bytes={0x70088: 1}),
            dict(pc=capture.RETURN, reply='T05')],
            ['--updates', '1', '--watch', 'state=0x70088'])
        self.assertEqual(self.events(output, 'step_after_watch'), [])
        self.assertEqual(len(self.rows(output / 'updates.jsonl')), 1, 'the return breakpoint was dealt with')

    def test_takeover_off_a_breakpoint_passes_over_the_saved_pending_trap(self):
        _, original, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY),
            dict(pc=capture.RETURN, step='T05watch:70088;thread:p1.1;', step_pc=0x4100)],
            ['--updates', '1', '--watch', 'state=0x70088'])
        self.assertEqual(json.loads((original / 'status.json').read_text())['stepped_to'],
                         [0x4100, GAME_CR3, 'p1.1'])

        def prepare(output):
            for name in ('events.jsonl', 'updates.jsonl', 'watch.jsonl', 'status.json'):
                (output / name).write_bytes((original / name).read_bytes())

        _, output, _ = self.run_capture([
            dict(pc=0x4100, pending_traps=1, pending_reply='T05thread:p1.1;'),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--resume', '--watch', 'state=0x70088'], prepare=prepare)
        self.assertEqual([r['pc'] for r in self.events(output, 'took_over_off_break')], [0x4100])
        self.assertEqual([r['pc'] for r in self.events(output, 'step_after_watch')], [0x4100])
        self.assertEqual([r['update'] for r in self.rows(output / 'updates.jsonl')], [1, 2])
        self.assertEqual(len(self.rows(output / 'watch.jsonl')), 1)
        self.assertIsNone(json.loads((output / 'status.json').read_text())['stepped_to'])

    def test_takeover_discards_a_pending_trap_for_a_different_stop(self):
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'status.json').write_text(json.dumps(dict(stepped_to=[0x4100, GAME_CR3, 'p1.1'])))

        for attached in (
            dict(pc=0x4101, attach_reply='T05thread:p1.1;'),
            dict(pc=0x4100, cr3=OTHER_CR3, attach_reply='T05thread:p1.1;'),
            dict(pc=0x4100, attach_reply='T05thread:p2.1;'),
        ):
            with self.subTest(attached=attached):
                with self.assertRaisesRegex(RuntimeError, 'unexpected stop at 0x4100'):
                    self.run_capture([attached, dict(pc=0x4100, reply='T05thread:p1.1;')],
                                     ['--updates', '1', '--resume', '--watch', 'state=0x70088'], prepare=prepare)

    def test_takeover_at_a_return_retries_the_step_without_recording_the_return_twice(self):
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'status.json').write_text(json.dumps(dict(stepped_to=[capture.RETURN, GAME_CR3, 'p1.1'])))

        remote, output, _ = self.run_capture([
            dict(pc=capture.RETURN, attach_reply='T05thread:p1.1;', pending_traps=1,
                 pending_reply='T05thread:p1.1;'),
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--resume', '--watch', 'state=0x70088'], prepare=prepare)
        self.assertEqual(remote.step_calls, [(capture.RETURN, False), (capture.RETURN, False),
                                             (capture.ENTRY, False), (capture.RETURN, False)])
        self.assertEqual([r['pc'] for r in self.events(output, 'step_swallowed')], [capture.RETURN])
        self.assertEqual([r['update'] for r in self.rows(output / 'updates.jsonl')], [1, 2])
        self.assertEqual(len(self.events(output, 'took_over_at_return')), 1)
        self.assertIsNone(json.loads((output / 'status.json').read_text())['stepped_to'])

    def test_a_step_whose_retry_is_swallowed_fails(self):
        with patch.object(FakeRemote, 'step', autospec=True, side_effect=FakeRemote.step) as step:
            with self.assertRaisesRegex(RuntimeError, 'left EIP and CR3 unchanged after retry'):
                self.run_capture([dict(pc=0), dict(pc=capture.ENTRY, pending_traps=2)],
                                 ['--updates', '1', '--watch', 'state=0x70088'])
        self.assertEqual(step.call_count, 2)

    def test_before_the_games_process_is_known_a_watch_stop_is_unknown(self):
        for unreadable in (False, True):
            with self.subTest(unreadable=unreadable):
                remote, output, _ = self.run_capture([
                    dict(pc=0), dict(pc=0x123456, reply='T05watch:70088;', cr3=OTHER_CR3,
                                     lot_unreadable=unreadable, bytes={0x70088: 0x64}),
                    dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
                    ['--updates', '1', '--watch', 'state=0x70088'])
                self.assertEqual(self.rows(output / 'watch.jsonl'), [])
                self.assertEqual([(r['address'], r['cr3']) for r in self.events(output, 'unknown_watch')],
                                 [(0x70088, OTHER_CR3)])
                self.assertNotIn((0x123456, 0x70088), remote.memory_reads)

    def test_no_watch_is_armed_or_sampled_before_the_baseline(self):
        # A dialog breakpoint in another process, before the first lot
        # update, with a readable lot and different bytes at 0x70088.
        remote, output, _ = self.run_capture([
            dict(pc=0),
            dict(pc=capture.DIALOG_SHOW, cr3=OTHER_CR3, bytes={0x70088: 0x64}),
            dict(pc=capture.ENTRY, bytes={0x70088: 1}),
            dict(pc=0x123456, reply='T05watch:70088;', bytes={0x70088: 2}),
            dict(pc=capture.RETURN)],
            ['--updates', '1', '--watch', 'state=0x70088'])
        self.assertEqual([pc for action, pc in remote.watch_stops if action == 'set'], [capture.ENTRY])
        self.assertNotIn((capture.DIALOG_SHOW, 0x70088), remote.memory_reads)
        self.assertEqual([(r['before'], r['after']) for r in self.rows(output / 'watch.jsonl')],
                         [('01000000', '02000000')])

    def test_a_run_taken_over_at_a_breakpoint_whose_lot_cannot_be_read_sets_an_ordinary_watch(self):
        # As off a breakpoint: taken over at a dialog's breakpoint with the
        # lot unreadable, the ordinary watch is set there.
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text(
                json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n')
        remote, output, _ = self.run_capture(
            [dict(pc=capture.DIALOG_SHOW, lot_unreadable=True, step='T05watch:70088;'),
             dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '2', '--resume', '--watch', 'state=0x70088'], prepare=prepare)
        self.assertEqual(remote.watch_stops[0], ('set', capture.DIALOG_SHOW))
        self.assertEqual([r['name'] for r in self.rows(output / 'watch.jsonl')], ['state'])

    def test_a_run_taken_over_at_update_1s_return_records_it(self):
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text('')
        remote, output, _ = self.run_capture(
            [dict(pc=capture.RETURN)], ['--updates', '1', '--resume'], prepare=prepare)
        self.assertEqual([r['update'] for r in self.rows(output / 'updates.jsonl')], [1])

    def test_a_watch_armed_records_the_lot_it_was_armed_on(self):
        remote, output, _ = self.run_capture(
            [dict(pc=0), dict(pc=capture.ENTRY, lot=7), dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '1', '--watch', 'state=0x70088'])
        armed = self.events(output, 'watch_armed')
        self.assertEqual([r['lot'] for r in armed], [5])

    def test_a_late_watch_is_set_only_at_a_stop_on_the_lot(self):
        # Taken over after update 1: stops on lot 7 (an entry and a return
        # there) set nothing; update 2's return on lot 5 ends the watch. Then
        # the same stops on lot 5 set it.
        for lot, expected in ((7, False), (5, True)):
            def prepare(output):
                (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
                (output / 'updates.jsonl').write_text(
                    json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n')
            remote, output, _ = self.run_capture(
                [dict(pc=capture.ENTRY, lot=lot), dict(pc=capture.RETURN, lot=7), dict(pc=capture.RETURN)],
                ['--updates', '2', '--resume', '--late-watch', '2-2,state=0x70088'], prepare=prepare)
            self.assertEqual(bool(remote.watch_calls), expected, lot)
            armed = self.events(output, 'watch_armed')
            self.assertEqual([r['lot'] for r in armed], [5] if expected else [], lot)
            ended = self.events(output, 'watch_ended')
            self.assertEqual([(r['update'], r['set']) for r in ended], [(2, expected)], lot)

    def test_an_ordinary_watch_armed_before_is_not_a_late_watch_set(self):
        # Given as --watch with the same text, it was armed by the earlier run.
        def prepare(output):
            (output / 'events.jsonl').write_text(''.join(json.dumps(r) + '\n' for r in [
                dict(kind='baseline', cr3=GAME_CR3),
                dict(kind='watch_armed', name='2-2,state', expr='2-2,state=0x70088', late=False,
                     address=0x70088, value=0, update=1),
                # An earlier revision's, without `late`: its name has the updates.
                dict(kind='watch_armed', name='2-2,state', expr='2-2,state=0x70088',
                     address=0x70088, value=0, update=1)]))
            (output / 'updates.jsonl').write_text(
                json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0)) + '\n')
        remote, output, _ = self.run_capture(
            [dict(pc=0x123456, cr3=OTHER_CR3), dict(pc=capture.RETURN)],
            ['--updates', '2', '--resume', '--late-watch', '2-2,state=0x70088'], prepare=prepare)
        ended = self.events(output, 'watch_ended')
        self.assertEqual([(r['update'], r['set']) for r in ended], [(2, False)])

    def test_a_late_watch_given_twice_is_an_argument_error(self):
        with self.assertRaises(SystemExit):
            self.run_capture([dict(pc=0)], ['--updates', '1', '--late-watch', '2,x=0x70088',
                                            '--late-watch', '2,x=0x70088'])

    def test_a_watch_name_may_be_any_text_and_an_object_alone_takes_the_default_size(self):
        w = capture.Watch('\u00e9tat=0x70088:2')
        self.assertEqual((w.name, w.start, w.size), ('\u00e9tat', 0x70088, 2))
        w = capture.Watch.late('2,x=object:259')
        self.assertEqual((w.object, w.steps, w.size), (259, [], 4))
        w = capture.Watch.late('2,x=person:259:2')
        self.assertEqual((w.object, w.person, w.size), (259, True, 2))

    def test_a_late_watch_that_does_not_parse_is_an_argument_error(self):
        for text in ['3-2,x=0x70088', '3-,x=0x70088', '3x=0x70088', '0,x=0x70088', '2,x=0x70088:3', '2,=0x70088', '\u00b2,x=0x70088', '2,x=0x70088:\u0664']:
            with self.assertRaises(argparse.ArgumentTypeError, msg=text):
                capture.Watch.late(text)

    def test_a_late_watch_reads_its_updates(self):
        w = capture.Watch.late('4150-4370,rng=0x6e54ac:4')
        self.assertEqual((w.first, w.last, w.name, w.start, w.size), (4150, 4370, 'rng', 0x6e54ac, 4))
        w = capture.Watch.late('5250,w50=object:21+0xae:2')
        self.assertEqual((w.first, w.last, w.object, w.steps, w.size), (5250, None, 21, [('+', 0xae)], 2))

    def test_two_watches_can_share_a_watchpoint(self):
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY, animator=0x80000),  # direction moves onto x's address
            dict(pc=0x123456, animator=0x80000, reply='T05watch:80088;'),
            dict(pc=capture.RETURN, animator=0x80000),
            dict(pc=capture.ENTRY, animator=0x90000),  # and off it again
            dict(pc=capture.RETURN, animator=0x90000)],
            ['--updates', '3', '--watch', 'x=0x80088', '--watch', 'direction=person:259@0xadc+0x88'])
        self.assertEqual(remote.watch_calls, [(0x80088, 4), (0x70088, 4), (0x90088, 4)])
        # x's watchpoint stays until the end; only then is everything cleared.
        self.assertEqual(remote.cleared_watches, [(0x70088, 4), (0x80088, 4), (0x90088, 4)])
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual(sorted(r['name'] for r in rows), ['direction', 'x'])

    def test_an_unreadable_new_address_puts_the_watch_back_to_wait(self):
        remote, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.RETURN),
            dict(pc=capture.ENTRY, animator=0xa0000, unreadable=0xa0088),
            dict(pc=capture.RETURN, animator=0xa0000)],
            ['--updates', '2', '--watch', 'direction=person:259@0xadc+0x88'])
        self.assertEqual([r['address'] for r in self.events(output, 'watch_lost')], [0x70088])
        self.assertEqual([r['address'] for r in self.events(output, 'watch_armed')], [0x70088, 0xa0088])
        self.assertEqual(json.loads((output / 'status.json').read_text())['state'], 'complete')

    def resumed_at_entry(self, done):
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text('')
            d = output / 'dump-0001'
            d.mkdir()
            (d / '00010000.bin').write_bytes(b'partial')
            if done:
                (d / 'registers.json').write_text('{}')
        return prepare

    def test_a_dump_cut_short_is_redone_on_resume(self):
        # The earlier run was killed while dumping at update 1's entry; the guest is still stopped there.
        remote, output, calls = self.run_capture(
            [dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '1', '--dump', '1', '--guest-dir', '/tmp/synthetic-guest', '--resume'],
            prepare=self.resumed_at_entry(done=False))
        self.assertEqual([c[2].name for c in calls], ['dump-0001'])
        self.assertEqual(len(self.events(output, 'redo_incomplete_dump')), 1)
        self.assertFalse((output / 'dump-0001' / '00010000.bin').exists())

    def test_a_finished_dump_is_kept_on_resume(self):
        remote, output, calls = self.run_capture(
            [dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '1', '--dump', '1', '--guest-dir', '/tmp/synthetic-guest', '--resume'],
            prepare=self.resumed_at_entry(done=True))
        self.assertEqual(calls, [])
        self.assertEqual(len(self.events(output, 'dump_done_already')), 1)

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


class SparseTest(CaptureTest):
    """--around and --before-hour: which updates are saved, and why."""

    def updates(self, stops, extra, entry=None):
        # By default update 1's entry has the first return's objects and clock (nothing changes in update 1).
        if entry is None:
            entry = {k: stops[0][k] for k in ('objects', 'clock', 'ids') if k in stops[0]}
        _, output, _ = self.run_capture([dict(pc=0), dict(pc=capture.ENTRY, **entry)] + stops, extra)
        return output, self.rows(output / 'updates.jsonl')

    def saved(self, rows):
        return {r['update']: r['saved_for'] for r in rows if 'saved' in r}

    def test_updates_to_hour(self):
        self.assertEqual(capture.updates_to_hour([6, 59, 58]), 1)
        self.assertEqual(capture.updates_to_hour([6, 59, 56]), 2)
        self.assertEqual(capture.updates_to_hour([6, 0, 0]), 1800)
        self.assertEqual(capture.updates_to_hour([6, 50, 44]), 278)

    def test_a_new_object_saves_its_update_and_the_next_ones(self):
        stops = [dict(pc=capture.RETURN, objects=[0x50000])] * 2 + \
                [dict(pc=capture.RETURN, objects=[0x50000, 0x51000])] * 5
        output, rows = self.updates(stops, ['--updates', '7', '--around', '2', '--full', '1'])
        self.assertEqual(self.saved(rows), {1: ['full'], 3: ['objects'], 4: ['around'], 5: ['around']})
        changed = self.events(output, 'objects_changed')
        self.assertEqual([(e['update'], e['added'], e['removed']) for e in changed],
                         [(3, [dict(id=0, pointer=0x51000)], [])])
        self.assertEqual([r['objects'] for r in rows], [1, 1, 2, 2, 2, 2, 2])

    def test_a_removed_object_is_named(self):
        stops = [dict(pc=capture.RETURN, objects=[0x50000, 0x51000]), dict(pc=capture.RETURN, objects=[0x51000])]
        output, rows = self.updates(stops, ['--updates', '2', '--around', '1'])
        changed = self.events(output, 'objects_changed')
        self.assertEqual(changed[0]['removed'], [dict(id=259, pointer=0x50000)])
        self.assertEqual(self.saved(rows), {2: ['objects']})

    def test_the_hour_turning_and_a_dialog_save_too(self):
        stops = [dict(pc=capture.RETURN, clock=[23, 59, 58]), dict(pc=capture.RETURN, clock=[24, 0, 0]),
                 dict(pc=capture.RETURN, clock=[24, 0, 2]), dict(pc=capture.RETURN, clock=[24, 0, 4]),
                 dict(pc=capture.DIALOG_SHOW), dict(pc=capture.RETURN, clock=[24, 0, 6]),
                 dict(pc=capture.RETURN, clock=[24, 0, 8])]
        output, rows = self.updates(stops, ['--updates', '6', '--around', '1'])
        self.assertEqual(self.saved(rows), {2: ['hour'], 3: ['around'], 5: ['dialog'], 6: ['around']})
        self.assertEqual([(e['update'], e['clock'], e['previous']) for e in self.events(output, 'hour_turned')],
                         [(2, [24, 0, 0], [23, 59, 58])])
        self.assertEqual(rows[0]['date'], [16, 1, 21, 6, 1997])

    def test_the_last_updates_before_the_hour(self):
        stops = [dict(pc=capture.RETURN, clock=[6, 59, s]) for s in (52, 54, 56, 58)] + \
                [dict(pc=capture.RETURN, clock=[7, 0, 0])]
        output, rows = self.updates(stops, ['--updates', '5', '--before-hour', '2'])
        self.assertEqual(self.saved(rows), {3: ['before_hour'], 4: ['before_hour']})
        self.assertEqual(self.events(output, 'objects_changed'), [])

    def test_without_the_options_the_rows_are_as_before(self):
        _, rows = self.updates([dict(pc=capture.RETURN)], ['--updates', '1', '--full', '1'])
        self.assertEqual(set(rows[0]), {'update', 'advanced', 'tick', 'lot_rng', 'global_rng', 'crt',
                                        'scheduler_cursor', 'saved', 'saved_for', 'seconds'})

    def test_a_resumed_run_keeps_its_window(self):
        def prepare(output):
            (output / 'events.jsonl').write_text(json.dumps(dict(kind='baseline', cr3=GAME_CR3)) + '\n')
            (output / 'updates.jsonl').write_text(json.dumps(dict(update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0,
                                                                  around_until=3, clock=[6, 10, 0])) + '\n')
        _, output, _ = self.run_capture([dict(pc=capture.RETURN), dict(pc=capture.RETURN), dict(pc=capture.RETURN)],
                                        ['--updates', '4', '--around', '2', '--resume'], prepare=prepare)
        rows = self.rows(output / 'updates.jsonl')[1:]
        self.assertEqual(self.saved(rows), {2: ['around'], 3: ['around']})

    def test_an_object_made_at_a_freed_objects_address_is_another_object(self):
        stops = [dict(pc=capture.RETURN, objects=[0x50000, 0x51000], ids={0x50000: 259, 0x51000: 7}),
                 dict(pc=capture.RETURN, objects=[0x50000, 0x51000], ids={0x50000: 999, 0x51000: 7}),
                 dict(pc=capture.RETURN, objects=[0x51000], ids={0x51000: 7})]
        output, rows = self.updates(stops, ['--updates', '3', '--around', '1'])
        changed = [(e['update'], e['added'], e['removed']) for e in self.events(output, 'objects_changed')]
        self.assertEqual(changed, [(2, [dict(id=999, pointer=0x50000)], [dict(id=259, pointer=0x50000)]),
                                   (3, [], [dict(id=999, pointer=0x50000)])])
        self.assertEqual(self.saved(rows), {2: ['objects'], 3: ['objects']})

    def test_a_change_in_update_1_is_seen(self):
        stops = [dict(pc=capture.RETURN, objects=[0x50000], clock=[7, 0, 0])]
        output, rows = self.updates(stops, ['--updates', '1', '--around', '1'],
                                    entry=dict(objects=[0x50000, 0x51000], ids={0x50000: 259, 0x51000: 7},
                                               clock=[6, 59, 58]))
        self.assertEqual([e['removed'] for e in self.events(output, 'objects_changed')],
                         [[dict(id=7, pointer=0x51000)]])
        self.assertEqual([e['update'] for e in self.events(output, 'hour_turned')], [1])
        self.assertEqual(self.saved(rows), {1: ['objects', 'hour']})

    def test_the_result_is_the_argument(self):
        _, output, _ = self.run_capture([
            dict(pc=0), dict(pc=capture.ENTRY), dict(pc=capture.DIALOG_SHOW),
            dict(pc=capture.DIALOG_RESULT, result=3), dict(pc=capture.RETURN)], ['--updates', '1'])
        self.assertEqual([e['result'] for e in self.events(output, 'dialog_result')], [3])
        self.assertNotIn('result', self.events(output, 'dialog_show')[0])

    def resumed_after(self, status, events=()):
        """An earlier run recorded update 1 (registry [0x50000] -> 259) and was killed in update 2."""
        def prepare(output):
            rows = [dict(kind='baseline', cr3=GAME_CR3)] + list(events)
            (output / 'events.jsonl').write_text(''.join(json.dumps(r) + '\n' for r in rows))
            (output / 'updates.jsonl').write_text(json.dumps(dict(
                update=1, tick=99, lot_rng=0, global_rng=0, crt=0, scheduler_cursor=0, clock=[6, 59, 58])) + '\n')
            (output / 'status.json').write_text(json.dumps(status))
        return prepare

    def test_a_resumed_run_knows_the_registry_and_redoes_the_return(self):
        # Killed in update 2's return after its hour event was written: the event is dropped and written again.
        hour = dict(kind='hour_turned', update=2, tick=100, clock=[7, 0, 0], previous=[6, 59, 58], time=1)
        _, output, _ = self.run_capture(
            [dict(pc=capture.RETURN, objects=[0x50000, 0x51000], ids={0x50000: 259, 0x51000: 7}, clock=[7, 0, 0])],
            ['--updates', '2', '--around', '1', '--resume'],
            prepare=self.resumed_after(dict(registry=[[0x50000, 259]], registry_update=1), [hour]))
        self.assertEqual([e['update'] for e in self.events(output, 'hour_turned')], [2])
        self.assertEqual([e['added'] for e in self.events(output, 'objects_changed')],
                         [[dict(id=7, pointer=0x51000)]])
        self.assertEqual(self.events(output, 'resume_dropped')[0]['events'], [dict(kind='hour_turned', update=2)])
        status = json.loads((output / 'status.json').read_text())
        self.assertEqual((status['registry'], status['registry_update']), ([[0x50000, 259], [0x51000, 7]], 2))

    def test_a_registry_from_another_update_is_not_used(self):
        # An older run's change in the update being redone is kept: this run cannot write it again.
        change = dict(kind='objects_changed', update=2, tick=100, added=[dict(id=7, pointer=0x51000)], removed=[],
                      count=2, time=1)
        _, output, _ = self.run_capture(
            [dict(pc=capture.RETURN, objects=[0x50000, 0x51000], ids={0x50000: 259, 0x51000: 7})],
            ['--updates', '2', '--around', '1', '--resume'],
            prepare=self.resumed_after(dict(registry=[], registry_update=0), [change]))
        self.assertEqual([e['update'] for e in self.events(output, 'objects_changed')], [2])
        self.assertEqual(self.events(output, 'resume_dropped'), [])
        self.assertEqual(len(self.events(output, 'registry_unknown')), 1)

    def test_a_resumed_run_keeps_the_registry_it_was_given(self):
        # Taken over again before its first return, the next run still finds the registry.
        _, output, _ = self.run_capture(
            [dict(pc=capture.RETURN)], ['--updates', '1', '--around', '1', '--resume'],
            prepare=self.resumed_after(dict(registry=[[0x50000, 259]], registry_update=1)))
        status = json.loads((output / 'status.json').read_text())
        self.assertEqual((status['registry'], status['registry_update']), ([[0x50000, 259]], 1))

    def test_a_resumed_run_knows_the_open_dialog(self):
        shown = dict(kind='dialog_show', update=2, record=0x4000, time=1)
        pressed = dict(kind='dialog_auto_answer', update=2, record=0x4000, press=1, job=7, time=2)
        with tempfile.TemporaryDirectory(prefix='sims-guest-test-') as guest_dir, \
                patch.object(capture, 'DialogAnswerer') as answerer:
            _, output, _ = self.run_capture(
                [dict(pc=capture.RETURN, clock=[6, 59, 58])],
                ['--updates', '2', '--around', '1', '--resume', '--answer-stuck', '100', '--answer-from', '2',
                 '--guest-dir', guest_dir],
                prepare=self.resumed_after(dict(registry=[[0x50000, 259]], registry_update=1), [shown, pressed]))
        # The press the earlier run handed to the guest goes with it, so the dialog's result cancels it.
        answerer.return_value.shown.assert_called_once_with(2, 0x4000, [7])
        self.assertEqual(self.events(output, 'resume_open_dialogs')[0]['dialogs'],
                         [dict(record=0x4000, update=2, jobs=[7])])
        self.assertEqual(self.saved(self.rows(output / 'updates.jsonl')), {2: ['dialog']})

    def test_negative_windows_are_refused(self):
        with self.assertRaises(SystemExit), patch.object(sys, 'stderr', io.StringIO()):
            self.run_capture([dict(pc=0)], ['--around', '-1'])


class DialogAnswererTest(unittest.TestCase):
    def answerer(self, after=0.05):
        temporary = tempfile.TemporaryDirectory(prefix='sims-answer-test-')
        self.addCleanup(temporary.cleanup)
        events, jobs = [], []
        a = capture.DialogAnswerer(Path(temporary.name), '0xe00013', after, lambda **r: events.append(r),
                                   Path(temporary.name) / 'input-log.txt', tries=2)
        self.addCleanup(a.close)
        def submit(share, script):
            (share / 'cmd').mkdir(exist_ok=True)
            jobs.append(script)
            return len(jobs)
        patcher = patch.object(capture.guest_jobs, 'submit', side_effect=submit)
        patcher.start()
        self.addCleanup(patcher.stop)
        return a, events, jobs, Path(temporary.name) / 'input-log.txt'

    def test_a_stuck_dialog_gets_return_twice(self):
        a, events, jobs, log = self.answerer()
        a.shown(901)
        deadline = time.time() + 5
        while len(events) < 2 and time.time() < deadline:
            time.sleep(0.01)
        time.sleep(0.15)
        self.assertEqual([(e['update'], e['press']) for e in events], [(901, 1), (901, 2)])
        self.assertIn('key Return', jobs[0])
        self.assertIn('0xe00013', jobs[0])
        self.assertIn('dialog of update 901', log.read_text())

    def test_an_answered_dialog_gets_nothing(self):
        a, events, jobs, _ = self.answerer(after=0.1)
        a.shown(5)
        a.answered()
        time.sleep(0.3)
        self.assertEqual((events, jobs), ([], []))

    def test_a_result_ends_only_its_own_dialogs_presses(self):
        a, events, jobs, _ = self.answerer(after=0.1)
        a.shown(5, 0xa)
        a.shown(6, 0xb)
        a.answered(0xb)
        time.sleep(0.15)
        a.answered(0xa)
        time.sleep(0.2)
        self.assertEqual([(e['kind'], e['update'], e['record']) for e in events],
                         [('dialog_auto_answer', 5, 0xa), ('dialog_auto_answer_cancelled', 5, 0xa)])

    def test_a_result_cancels_a_press_already_handed_to_the_guest(self):
        a, events, jobs, _ = self.answerer(after=0.05)
        a.shown(5, 0xa)
        deadline = time.time() + 5
        while not jobs and time.time() < deadline:
            time.sleep(0.01)
        a.answered(0xa)
        cancel = a.share / 'cmd' / '00001.cancel'
        self.assertTrue(cancel.exists())
        self.assertEqual(events[-1]['jobs'], [1])
        # The job itself: with its .cancel file present it stops before the press.
        bin_ = a.share / 'bin'
        bin_.mkdir()
        fake = bin_ / 'xdotool'
        fake.write_text('#!/bin/sh\necho "$@" >> "$XDOTOOL_LOG"\n')
        fake.chmod(0o755)
        job = a.share / 'cmd' / '00001.sh'
        job.write_text(jobs[0])
        env = dict(os.environ, PATH=f'{bin_}:{os.environ["PATH"]}', XDOTOOL_LOG=str(a.share / 'xdotool.log'))
        r = subprocess.run(['/bin/sh', str(job)], env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 3)
        self.assertNotIn('key Return', (a.share / 'xdotool.log').read_text())
        cancel.unlink()
        subprocess.run(['/bin/sh', str(job)], env=env, check=True)
        self.assertIn('key Return', (a.share / 'xdotool.log').read_text())

    def test_the_jobs_of_an_earlier_run_are_cancelled_and_counted(self):
        a, events, jobs, _ = self.answerer(after=0.05)
        (a.share / 'cmd').mkdir()
        a.shown(5, 0xa, [3, 4])  # two of the two tries made already
        time.sleep(0.15)
        a.answered(0xa)
        self.assertEqual(jobs, [])
        self.assertTrue((a.share / 'cmd' / '00003.cancel').exists() and (a.share / 'cmd' / '00004.cancel').exists())
        self.assertEqual(events[-1]['jobs'], [3, 4])

    def test_no_return_when_the_window_cannot_be_focused(self):
        a, events, jobs, _ = self.answerer(after=0.05)
        a.shown(5, 0xa)
        deadline = time.time() + 5
        while not jobs and time.time() < deadline:
            time.sleep(0.01)
        a.close()
        bin_ = a.share / 'bin'
        bin_.mkdir()
        (bin_ / 'xdotool').write_text('#!/bin/sh\necho "$@" >> "$XDOTOOL_LOG"\n[ "$1" != windowfocus ]\n')
        (bin_ / 'xdotool').chmod(0o755)
        job = a.share / 'cmd' / '00001.sh'
        job.write_text(jobs[0])
        log = a.share / 'xdotool.log'
        env = dict(os.environ, PATH=f'{bin_}:{os.environ["PATH"]}', XDOTOOL_LOG=str(log))
        r = subprocess.run(['/bin/sh', str(job)], env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 4)
        self.assertNotIn('key Return', log.read_text())

    def test_only_the_open_dialog_is_pressed(self):
        a, events, jobs, _ = self.answerer(after=0.1)
        a.shown(5)
        a.answered()
        a.shown(6)
        time.sleep(0.15)
        a.answered()
        time.sleep(0.2)
        self.assertEqual([e['update'] for e in events if e['kind'] == 'dialog_auto_answer'], [6])
        self.assertEqual([e['update'] for e in events if e['kind'] == 'dialog_auto_answer_cancelled'], [6])


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
