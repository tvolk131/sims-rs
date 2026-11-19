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
        if p == self.remote.current.get('unreadable'):
            raise MemoryError(f'read {p:#x}')
        return bytes(n)

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
            dict(pc=capture.ENTRY), dict(pc=capture.RETURN)],
            ['--updates', '4', '--late-watch', '2-3,state=0x70088'])
        self.assertEqual(remote.watch_calls, [(0x70088, 4)])
        self.assertEqual(remote.cleared_watches, [(0x70088, 4)])
        armed = self.events(output, 'watch_armed')
        self.assertEqual([r['update'] for r in armed], [1])  # at the entry of update 2
        ended = self.events(output, 'watch_ended')
        self.assertEqual([r['update'] for r in ended], [3])  # at the entry of update 4
        rows = self.rows(output / 'watch.jsonl')
        self.assertEqual(len(rows), 1)

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
