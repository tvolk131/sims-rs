"""Tests for manifest.py against a synthetic capture directory (no Docker, no guest).

Run with: python3 -B -m unittest discover -s tools/harness -p 'test_*.py'
"""
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import manifest  # noqa: E402


def jsonl(path, rows):
    path.write_text(''.join(json.dumps(r) + '\n' for r in rows))


class RangesTest(unittest.TestCase):
    def test_runs_and_steps(self):
        self.assertEqual(manifest.ranges([1, 2, 3, 5, 10, 20, 30]), '1-3,5,10-30/10')
        self.assertEqual(manifest.ranges([30, 10, 20, 20]), '10-30/10')
        self.assertEqual(manifest.ranges([1, 2]), '1,2')
        self.assertEqual(manifest.ranges([]), '')
        self.assertEqual(manifest.ranges([7]), '7')


class DayTest(unittest.TestCase):
    def test_the_first_hour_turned_twice(self):
        # 7 to 23, then the day turns (24 or 0), then 1 to 8.
        clocks = list(range(7, 25)) + list(range(1, 9))
        hours = [dict(update=10 + 1800 * i, tick=100 + 1800 * i, clock=[h, 0, 0]) for i, h in enumerate(clocks)]
        day = manifest.day_length(hours)
        self.assertEqual((day['hour'], day['ticks'], day['updates'], day['from_update']), (7, 43200, 43200, 10))

    def test_no_whole_day(self):
        self.assertIsNone(manifest.day_length([dict(update=1, tick=1, clock=[7, 0, 0])]))


class BuildTest(unittest.TestCase):
    def test_a_small_capture(self):
        with tempfile.TemporaryDirectory(prefix='sims-manifest-test-') as d:
            c = Path(d) / 'goth-day'
            c.mkdir()
            (c / 'entry-0000').mkdir()
            jsonl(c / 'updates.jsonl', [
                dict(update=1, advanced=1, tick=100, clock=[6, 59, 58], date=[1, 2, 3, 4, 5], saved='update-0001',
                     saved_for=['full', 'dialog']),
                dict(update=2, advanced=1, tick=101, clock=[7, 0, 0], date=[1, 2, 3, 4, 5], saved='update-0002',
                     saved_for=['hour']),
                dict(update=3, advanced=0, tick=101, clock=[7, 0, 0], date=[1, 2, 3, 4, 5])])
            jsonl(c / 'events.jsonl', [
                dict(kind='baseline', tick=99, lot=5, time=1),
                dict(kind='dialog_show', update=1, record=7, time=10),
                dict(kind='dialog_auto_answer', update=1, press=1, time=20),
                dict(kind='dialog_result', update=1, record=7, status=1, stack='795d4300' + '03000000' + '00' * 24,
                     time=30.5),
                dict(kind='hour_turned', update=2, tick=101, clock=[7, 0, 0], date=[1, 2, 3, 4, 5], time=40),
                dict(kind='objects_changed', update=2, tick=101, added=[dict(id=8, pointer=1)], removed=[],
                     count=3, time=41)])
            (c / 'status.json').write_text(json.dumps(dict(state='complete', settings={'lot': '5'})))
            (c / 'input-log.txt').write_text('2026-11-15T14:37:47Z\nenter goth: click 658 214 15\n')
            m = manifest.build(c, ['a note'], 'House05')
        self.assertEqual(m['updates'], 3)
        self.assertEqual(m['ticks'], [100, 101])
        self.assertEqual(m['partial_passes'], [3])
        self.assertEqual(m['full_state']['updates'], '1,2')
        self.assertEqual(m['full_state']['by_reason'], {'dialog': '1', 'full': '1', 'hour': '2'})
        self.assertTrue(m['full_state']['entry_0000'])
        # The result is the argument on the stack (3), not the record's status before the call (1).
        self.assertEqual(m['dialogs'], [dict(update=1, record=7, result=3, result_update=1, auto_presses=1,
                                             cancelled_presses=0, seconds_open=20.5)])
        self.assertEqual(m['objects_changed'], [dict(update=2, tick=101, added=[8], removed=[], count=3)])
        self.assertEqual(m['presses'], [dict(time='2026-11-15T14:37:47Z', label='enter goth',
                                             command='click 658 214 15')])
        self.assertEqual((m['house'], m['notes'], m['baseline']), ('House05', ['a note'], dict(tick=99, lot=5)))


class DialogsTest(unittest.TestCase):
    def test_overlapping_dialogs_are_paired_by_record(self):
        events = [dict(kind='dialog_show', update=1, record=0xa, time=0),
                  dict(kind='dialog_show', update=2, record=0xb, time=5),
                  dict(kind='dialog_auto_answer', update=1, record=0xa, press=1, time=6),
                  dict(kind='dialog_result', update=3, record=0xa, result=2, time=10),
                  dict(kind='dialog_auto_answer_cancelled', update=2, record=0xb, jobs=[4, 5], time=11),
                  dict(kind='dialog_result', update=4, record=0xb, result=3, time=20)]
        self.assertEqual(manifest.dialog_rows(events), [
            dict(update=1, record=0xa, result=2, result_update=3, auto_presses=1, cancelled_presses=0,
                 seconds_open=10),
            dict(update=2, record=0xb, result=3, result_update=4, auto_presses=0, cancelled_presses=2,
                 seconds_open=15)])

    def test_a_turn_written_twice_is_counted_once(self):
        hours = [dict(update=10, tick=100, clock=[7, 0, 0]), dict(update=10, tick=100, clock=[7, 0, 0]),
                 dict(update=43210, tick=43300, clock=[7, 0, 0])]
        self.assertEqual(manifest.day_length(hours)['ticks'], 43200)


if __name__ == '__main__':
    unittest.main()
