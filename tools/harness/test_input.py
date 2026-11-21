"""Tests of input.py's pure parts, with a fake guest memory. No Docker needed:

  python3 -B -m unittest discover -s tools/harness -p 'test_*.py'
"""
import json
import os
import struct
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

import input as inp


class FakeMemory:
    """Little-endian memory built from writes; reads of unwritten bytes fail."""

    def __init__(self):
        self.bytes = {}
        self.next = 0x100000

    def put(self, p, data):
        for i, b in enumerate(data):
            self.bytes[p + i] = b

    def put_words(self, p, *words):
        self.put(p, struct.pack('<' + 'I' * len(words), *(w & 0xffffffff for w in words)))

    def alloc(self, n):
        p = self.next
        self.next += (n + 15) & ~15
        self.put(p, bytes(n))
        return p

    def string(self, text):
        data = text.encode('latin-1')
        p = self.alloc(len(data) + 1)
        self.put(p, data + b'\0')
        return p, p + len(data)

    def read(self, p, n):
        try:
            return bytes(self.bytes[p + i] for i in range(n))
        except KeyError:
            raise MemoryError(f'unmapped {p:#x}') from None

    def word(self, p):
        return struct.unpack('<I', self.read(p, 4))[0]

    def half(self, p):
        return struct.unpack('<H', self.read(p, 2))[0]

    def byte(self, p):
        return self.read(p, 1)[0]

    def words(self, p, n, width=4):
        return list(struct.unpack('<' + ('H' if width == 2 else 'I') * n, self.read(p, n * width))) if n else []

    def object_id(self, p):
        return self.half(p + 0xf0) if p else 0

    def action(self, p):
        return dict(uid=self.word(p + 60), caller=self.object_id(self.word(p + 4)),
                    target=self.object_id(self.word(p + 8)), icon=self.object_id(self.word(p + 12)),
                    menu=self.word(p + 16), args=self.words(p + 20, 4, 2),
                    routine=self.half(p + 32), priority=self.word(p + 28))


class Game:
    """A fake game: a lot with objects, the view, the picker and the people panel."""

    def __init__(self):
        m = self.m = FakeMemory()
        self.sim = m.alloc(0x200)
        self.registry = m.alloc(0x40)
        m.put_words(self.sim + 0xb0, self.registry)
        self.slots = m.alloc(4 * 16)
        m.put_words(self.registry + 0x18, self.slots, self.slots)
        self.objects = {}
        game = m.alloc(0x40)
        m.put_words(inp.GAME, game)
        manager = m.alloc(8)
        m.put_words(game + 0x18, manager)
        tools = m.alloc(4 * 12)
        m.put_words(manager, tools)
        self.picker = m.alloc(0x100)
        m.put_words(tools + 4 * inp.PICKER_TOOL, self.picker)
        self.panel = m.alloc(0x100)
        m.put_words(inp.PEOPLE_PANEL, self.panel)
        self.pies = [m.alloc(0x1a0), m.alloc(0x1a0)]
        m.put_words(self.panel + 0xdc, *self.pies)
        m.put_words(inp.VIEW_ZOOM, 1)
        m.put_words(inp.VIEW_X0, 368, -130)

    def add_object(self, oid, position, person=False, selected=False):
        m = self.m
        p = m.alloc(0xb10)
        definition = m.alloc(0xa8)
        m.put(definition + 0x12, struct.pack('<H', 2 if person else 4))
        m.put_words(p + 0x118, definition)
        m.put(p + 0xf0, struct.pack('<H', oid))
        m.put_words(p + 0xf4, *position)
        m.put_words(p + 0x114, 2 if selected else 0)
        end = m.word(self.registry + 0x1c)
        m.put_words(end, p)
        m.put_words(self.registry + 0x1c, end + 4)
        self.objects[oid] = p
        return p

    def remove_object(self, oid):
        p = self.objects.pop(oid)
        q = next(q for q in range(self.slots, self.m.word(self.registry + 0x1c), 4) if self.m.word(q) == p)
        self.m.put_words(q, 0)

    def hover(self, oid):
        self.m.put(self.picker + 0x2e, struct.pack('<H', oid))

    def open_pie(self, labels, origin=(309, 110), target=None, caller=None, submenus=(), window=0, first=0):
        """A menu of entries `labels` in pie window `window`, showing the entries
        from `first` on (a paged menu). A submenu opens in the other window
        while the parent window keeps its items, as in the game."""
        m = self.m
        entries = m.alloc(0x28 * len(labels))
        items = m.alloc(0x28 * len(labels))
        for i, label in enumerate(labels):
            b, e = m.string(label)
            action = m.alloc(0x44)
            m.put_words(action + 4, caller or 0, target or 0)
            m.put_words(entries + 0x28 * i, b, e, e + 1)
            m.put_words(entries + 0x28 * i + 0x24, action)
            if label in submenus:
                m.put(entries + 0x28 * i + 0x14, b'\x01')
        shown = labels[first:]
        for i, label in enumerate(shown):
            b, e = m.string(label)
            m.put_words(items + 0x28 * i, b, e, e + 1)
            m.put(items + 0x28 * i + 0x10, b'\x01')
            m.put_words(items + 0x28 * i + 0x18, 10 + 40 * i, 20, 30, 20)
        vec = m.alloc(12)
        m.put_words(vec, entries, entries + 0x28 * len(labels), entries + 0x28 * len(labels))
        m.put_words(self.picker + 0x58, vec)
        m.put_words(self.picker + 0x64, first)
        pie = self.pies[window]
        m.put_words(self.picker + inp.PICKER_ACTIVE_PIE, pie)
        m.put_words(pie + 0x190, items, items + 0x28 * len(shown))
        m.put_words(pie + 0x13c, *origin)
        m.put_words(pie + 0xd4, -1)

    def highlight(self, i):
        self.m.put_words(self.m.word(self.picker + inp.PICKER_ACTIVE_PIE) + 0xd4, i)

    def close_pie(self):
        self.m.put_words(self.picker + 0x58, 0)


class FakeAgent:
    def __init__(self):
        self.dir = Path(tempfile.mkdtemp())
        self.window = '0x1'
        self.commands = []
        self.done = set()

    def submit(self, command):
        self.commands.append(command)
        return len(self.commands)

    def result(self, n):
        return getattr(self, 'codes', {}).get(n, 0) if n in self.done else None

    def finish_all(self):
        self.done |= set(range(1, len(self.commands) + 1))


class ParseTest(unittest.TestCase):
    def test_steps_and_arguments(self):
        steps = inp.parse_scenario('''
            # Mortimer sits on the bench
            until 40
            click-object 173      # the bench
            choose "Sit"
            expect-queued 21 173 within 50
            expect-queued 21
            key F2
            answer-dialog 400 378
            shot after-sit
        ''')
        self.assertEqual([s['op'] for s in steps], ['until', 'click-object', 'choose', 'expect-queued',
                                                     'expect-queued', 'key', 'answer-dialog', 'shot'])
        self.assertEqual(steps[0]['args'], [40])
        self.assertEqual(steps[2]['args'], ['Sit'])
        self.assertEqual(steps[3]['args'], [21, 173, 50])
        self.assertEqual(steps[4]['args'], [21, None, 100])
        self.assertEqual(steps[0]['line'], 3)

    def test_build_and_buy_steps(self):
        steps = inp.parse_scenario('press 93 458\nexpect-mode build within 40\nexpect-mode buy within 50\n'
                                   'expect-mode 1\ndrag 470 380 530 350\nexpect-new-object\nexpect-new-object within 9')
        self.assertEqual([s['args'] for s in steps],
                         [[93, 458], [1, 40], [2, 50], [1, 200], [470, 380, 530, 350], [200], [9]])
        for bad in ('expect-mode flying', 'drag 1 2 3', 'drag a 2 3 4', 'drag -1 2 3 4', 'move -1 5'):
            with self.assertRaises(inp.ScenarioError):
                inp.parse_scenario(bad)

    def test_errors_name_the_line(self):
        for text, message in (('jump 3', 'line 1: unknown step'), ('wait', 'line 1: wait takes'),
                              ('\nclick 1', 'line 2: click takes'), ('wait x', 'line 1:'),
                              ('expect-queued 21 5 after 3', 'line 1: expect-queued')):
            with self.assertRaises(inp.ScenarioError) as e:
                inp.parse_scenario(text)
            self.assertIn(message, str(e.exception))


class GeometryTest(unittest.TestCase):
    def test_project_matches_the_original_formula(self):
        view = dict(zoom=1, x0=368, y0=-130)
        # The mailbox of the Goth house, position (0x2a8, 0x2b8) words (+0xf4, +0xf8).
        self.assertEqual(inp.project([0x2a8, 0x2b8], view), (368 + 16 * (43.5 - 42.5), -130 + 8 * 86))
        self.assertEqual(inp.project([16, 16], dict(zoom=2, x0=0, y0=0)), (0, 32))

    def test_aim_points_start_above_the_anchor_and_scale_with_zoom(self):
        near = inp.aim_points(100, 200, 1)
        self.assertEqual(near[0], (100, 192))
        self.assertTrue(all(y <= 200 for _, y in near[:7]))
        self.assertEqual(inp.aim_points(100, 200, 2)[0], (100, 184))
        self.assertEqual(len(set(near)), len(near))

    def test_clamp(self):
        self.assertEqual(inp.clamp((-5, 700)), (1, 598))

    def test_commands(self):
        self.assertIn('mousemove --window 0x1 9 5 sleep', inp.move_command(10, 5, '0x1'))
        self.assertTrue(inp.move_command(10, 5, '0x1').endswith('mousemove --window 0x1 10 5'))
        self.assertIn('mousemove --window 0x1 1 5 sleep', inp.move_command(0, 5, '0x1'))
        self.assertTrue(inp.move_command(0, 5, '0x1').endswith('mousemove --window 0x1 0 5'))
        self.assertEqual(inp.key_command('F2'), 'xdotool key F2')
        with self.assertRaises(inp.ScenarioError):
            inp.key_command('F2; rm -rf /')

    def test_match_item(self):
        items = [dict(label='Sit'), dict(label='Watch TV ')]
        self.assertEqual(inp.match_item(items, 'watch tv'), 1)
        self.assertIsNone(inp.match_item(items, 'Sleep'))


class ReadTest(unittest.TestCase):
    def test_reads(self):
        g = Game()
        bench = g.add_object(173, [0x1a8, 0x198])
        g.add_object(21, [0x198, 0x198], person=True, selected=True)
        g.hover(173)
        self.assertEqual(inp.read_hover(g.m), 173)
        self.assertEqual(inp.read_view(g.m), dict(zoom=1, x0=368, y0=-130))
        self.assertEqual(inp.find_object(g.m, g.sim, 173), bench)
        self.assertEqual(inp.find_object(g.m, g.sim, 5), 0)
        self.assertEqual(inp.object_position(g.m, bench), [0x1a8, 0x198])
        self.assertEqual(inp.selected_person(g.m, g.sim), 21)
        self.assertEqual(inp.read_entries(g.m), [])
        g.open_pie(['Sit', 'Clean'])
        pie = inp.read_pie(g.m)
        self.assertEqual([i['label'] for i in pie['items']], ['Sit', 'Clean'])
        self.assertEqual((pie['items'][1]['x'], pie['items'][1]['y']), (309 + 50, 130))
        self.assertEqual(pie['highlighted'], -1)
        self.assertEqual([e['label'] for e in inp.read_entries(g.m)], ['Sit', 'Clean'])


class RunnerBase(unittest.TestCase):
    def setUp(self):
        self.g = Game()
        self.bench = self.g.add_object(173, [0x1a8, 0x198])
        self.person = self.g.add_object(21, [0x198, 0x198], person=True)
        self.g.m.put_words(self.person + 0xa40, 0, 0)
        self.agent = FakeAgent()
        self.log = Path(tempfile.mkdtemp()) / 'input.jsonl'
        self.update = 0
        self.runners = []

    def tearDown(self):
        for r in self.runners:
            r.close()

    def runner(self, text):
        r = inp.Runner(inp.parse_scenario(text), self.agent, self.log)
        self.runners.append(r)
        return r

    def tick(self, runner, n=1, finish=True):
        for _ in range(n):
            if finish:
                self.agent.finish_all()
            self.update += 1
            runner.on_update(self.update, self.g.m, self.g.sim)

    def rows(self, kind=None):
        rows = [json.loads(line) for line in self.log.read_text().splitlines()]
        return [r for r in rows if kind is None or r['kind'] == kind]

    def tick_until(self, r, done, most=60):
        for _ in range(most):
            if done():
                return
            self.tick(r)
        self.fail(f'never happened: {self.rows()}')

    def last_command(self):
        return self.agent.commands[-1] if self.agent.commands else ''

    def put_action(self, addr, caller, target, *, icon=0, menu=0, routine=0, uid=0):
        m = self.g.m
        m.put_words(addr + 4, caller, target, icon, menu)
        m.put(addr + 32, struct.pack('<H', routine))
        m.put_words(addr + 0x3c, uid)

    def push(self, r, routine, *, target=None, menu=0, frame=0x80000):
        a = self.g.m.alloc(0x44)
        self.put_action(a, self.person, self.bench if target is None else target, icon=target or 0,
                        menu=menu, routine=routine)
        r.on_queue_push(self.update + 1, False, self.g.m, dict(edi=self.person, esi=a, ebp=frame))
        return a

    def queue_result(self, r, action, uid=None, *, eax=1, frame=0x80000):
        if uid is not None:
            self.g.m.put_words(action + 0x3c, uid)
        r.on_queue_result(self.g.m, dict(eax=eax, ebp=frame))


class RunnerTest(RunnerBase):
    def test_waits(self):
        r = self.runner('until 3\nwait 2')
        self.tick(r, 3)
        self.assertEqual(r.index, 1)
        self.tick(r, 2)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)
        self.assertEqual([(x['status'], x['update']) for x in self.rows('step')],
                         [('start', 1), ('done', 3), ('start', 3), ('done', 5)])

    def test_a_command_waits_for_the_guest_and_settles(self):
        r = self.runner('click 10 20')
        self.tick(r, 1, finish=False)
        self.assertEqual(len(self.agent.commands), 1)
        self.tick(r, 2, finish=False)
        self.assertEqual(len(self.agent.commands), 1)  # the move has not ended
        self.tick(r, 3)
        self.assertEqual(len(self.agent.commands), 2)
        self.assertIn('mousedown 1', self.agent.commands[1])

    def test_a_press_is_held_while_updates_return(self):
        # A short press can fall inside one of capture.py's stops, where the
        # game never sees the button down; the button stays down until
        # HOLD_UPDATES updates have returned, then is let go.
        r = self.runner('click 10 20')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        self.assertEqual(self.last_command(), 'xdotool mousedown 1')
        down = self.update
        self.tick_until(r, lambda: 'mouseup' in self.last_command())
        self.assertEqual(self.last_command(), 'xdotool mouseup 1')
        self.assertGreaterEqual(self.update - down, inp.HOLD_UPDATES)  # its end is seen at the first
        self.assertEqual([c['what'] for c in self.rows('command')], ['move', 'press', 'release'])
        self.tick(r)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)

    def test_a_dialog_while_the_button_is_down_still_releases_it(self):
        r = self.runner('click 10 20\nanswer-dialog 400 400')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        r.on_dialog('dialog_show', self.update + 1)  # the press opened a modal dialog
        self.assertEqual([c['what'] for c in self.rows('command')], ['move', 'press', 'release', 'press'])
        self.assertIn('400 400', self.last_command())
        self.agent.finish_all()
        r.on_dialog('dialog_result', self.update + 1)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)

    def test_a_capture_ending_while_the_button_is_down_releases_it(self):
        r = self.runner('click 10 20')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        self.tick(r)  # the press has ended; the hold goes on
        r.close()
        self.runners.remove(r)
        self.assertEqual(self.last_command(), 'xdotool mouseup 1')
        self.assertEqual(self.rows('command')[-1]['reason'], 'capture ended')
        self.assertEqual(self.rows('scenario')[-1]['status'], 'unfinished')

    def test_a_failure_while_the_button_is_down_releases_it(self):
        r = self.runner('click 10 20')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        self.agent.finish_all()
        r.on_update(self.update + 1, None, self.g.sim)  # memory cannot be read: the step fails
        self.assertTrue(r.failed)
        self.assertEqual(self.last_command(), 'xdotool mouseup 1')
        self.assertEqual(self.rows('command')[-1]['reason'], 'step failed')
        r.close()
        self.runners.remove(r)
        self.assertEqual(sum('mouseup' in c for c in self.agent.commands), 1)

    def test_a_release_that_cannot_be_submitted_is_tried_again(self):
        r = self.runner('click 10 20')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        submit, broken = self.agent.submit, [2]

        def flaky(command):  # the next two submissions fail (the hold's release, then fail()'s)
            if broken[0]:
                broken[0] -= 1
                raise OSError('rename failed')
            return submit(command)
        self.agent.submit = flaky
        self.tick_until(r, lambda: r.failed)  # the hold's release could not be submitted
        self.assertNotIn('mouseup', self.last_command())
        self.assertEqual(self.rows('release_failed')[-1]['reason'], 'step failed')
        r.close()
        self.runners.remove(r)
        self.assertEqual(self.last_command(), 'xdotool mouseup 1')
        self.assertEqual(self.rows('command')[-1]['reason'], 'capture ended')

    def test_a_released_press_is_not_released_again(self):
        r = self.runner('click 10 20\nwait 5')
        self.tick_until(r, lambda: 'mouseup' in self.last_command())
        r.close()
        self.runners.remove(r)
        self.assertEqual(sum('mouseup' in c for c in self.agent.commands), 1)

    def test_click_object_choose_and_expect(self):
        g = self.g
        floor = g.add_object(29, [0, 0])
        r = self.runner('click-object 173\nchoose Sit\nexpect-queued 21 173 within 5')
        self.tick_until(r, lambda: self.rows('hover'))
        self.assertEqual(self.rows('hover')[-1]['object'], 0)  # missed: aim again
        self.assertIn('mousemove', self.last_command())
        g.hover(173)
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        # The press opened a menu for something else: it is closed, and the
        # bench aimed at again.
        g.open_pie(['Go Here'], target=floor, caller=self.person)
        self.tick_until(r, lambda: 'Escape' in self.last_command())
        self.assertEqual(self.rows('wrong_menu')[0]['targets'], [29])
        g.close_pie()
        presses = sum('mousedown' in c for c in self.agent.commands)
        self.tick_until(r, lambda: sum('mousedown' in c for c in self.agent.commands) > presses)
        g.open_pie(['Sit'], target=self.bench, caller=self.person)
        self.tick_until(r, lambda: self.last_command().split()[-2:] == ['334', '140'])
        self.assertEqual(self.rows('pie')[-1]['entries'][0]['label'], 'Sit')
        self.tick(r, 5)
        self.assertNotIn('mousedown', self.last_command())  # not highlighted yet
        g.highlight(0)
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        action = self.push(r, 0)
        self.queue_result(r, action, 6)
        self.tick(r, 4)
        self.assertFalse(r.finished)  # pushed, but the person does not hold it yet
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, uid=6)
        self.tick_until(r, lambda: r.finished)
        self.assertIsNone(r.failed)
        queued = self.rows('queued')
        self.assertEqual((queued[0]['person'], queued[0]['action']['target']), (21, 173))
        self.assertEqual(self.rows('queue_result')[0]['uid'], 6)
        taken = self.rows('taken')[0]
        self.assertEqual((taken['person'], taken['where'], taken['uid'], taken['active']['target']),
                         (21, 'active', 6, 173))

    def open_talk(self, r, **kw):
        """Press "Talk..." on a two-entry parent menu; returns at the press."""
        g = self.g
        g.open_pie(['Dance...', 'Talk...'], target=self.bench, caller=self.person,
                   submenus=('Dance...', 'Talk...'), **kw)
        self.tick(r, 2)
        g.highlight(1)
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        self.tick(r, 3)
        self.assertEqual([x['status'] for x in self.rows('step')], ['start'])  # waiting for the submenu

    def test_choosing_a_submenu_entry_opens_it_and_the_next_choice_picks_from_it(self):
        g = self.g
        r = self.runner('choose Talk...\nchoose Joke\nexpect-queued 21 173 within 20')
        self.open_talk(r)
        g.close_pie()  # empty for a moment between the two menus
        self.tick(r, 3)
        # The submenu opens in the other window; the parent window keeps its items.
        g.open_pie(['Joke', 'Gossip'], target=self.bench, caller=self.person, window=1)
        self.tick_until(r, lambda: len(self.rows('step')) > 2)
        self.assertEqual([e['label'] for e in self.rows('pie')[-1]['entries']], ['Joke', 'Gossip'])
        self.assertEqual(self.rows('step')[1]['status'], 'done')
        self.assertFalse(self.rows('queued'))  # the submenu press pushes nothing
        g.highlight(0)
        self.tick_until(r, lambda: sum('mousedown' in c for c in self.agent.commands) == 2)
        self.assertEqual(self.rows('command')[-1]['entry'], 'Joke')
        self.queue_result(r, self.push(r, 0), 2)
        g.close_pie()
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, uid=2)
        self.tick_until(r, lambda: r.finished)
        self.assertIsNone(r.failed)
        self.assertEqual(self.rows('taken')[0]['where'], 'active')

    def test_the_submenu_wait_ends_when_its_window_shows_it(self):
        g = self.g
        r = self.runner('choose Talk...\nchoose Joke\nwait 1')
        self.open_talk(r)
        g.open_pie(['Joke', 'Gossip'], target=self.bench, caller=self.person, window=1)
        g.m.put_words(g.picker + inp.PICKER_ACTIVE_PIE, g.pies[0])  # the new entries; the parent still shown
        self.tick(r, 5)
        self.assertEqual(self.rows('step')[-1]['index'], 0)
        g.m.put_words(g.picker + inp.PICKER_ACTIVE_PIE, g.pies[1])
        self.tick_until(r, lambda: len(self.rows('step')) > 2)
        self.assertEqual([it['label'] for it in self.rows('pie')[-1]['items']], ['Joke', 'Gossip'])
        self.assertIsNone(r.failed)

    def test_a_submenu_with_the_parents_labels_is_still_seen(self):
        g = self.g
        r = self.runner('choose Talk...\nwait 1')
        self.open_talk(r)
        g.open_pie(['Dance...', 'Talk...'], target=self.bench, caller=self.person, window=1)
        self.tick_until(r, lambda: r.finished)
        self.assertIsNone(r.failed)

    def test_a_submenu_entry_on_a_later_page(self):
        g = self.g
        # Entry 0 is an action, entry 1 ("Talk...") a submenu shown as item 0.
        g.open_pie(['Invite Downtown', 'Talk...'], target=self.bench, caller=self.person,
                   submenus=('Talk...',), first=1)
        r = self.runner('choose Talk...\nwait 1')
        self.tick(r, 2)
        g.highlight(0)
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        g.open_pie(['Joke'], target=self.bench, caller=self.person, window=1)
        self.tick_until(r, lambda: r.finished)
        self.assertIsNone(r.failed)
        self.assertEqual([e['label'] for e in self.rows('pie')[-1]['entries']], ['Joke'])

    def test_expect_queued_after_a_submenu_ignores_an_earlier_push(self):
        g = self.g
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, uid=2)
        r = self.runner('choose Talk...\nexpect-queued 21 173 within 5')
        self.queue_result(r, self.push(r, 0), 2)  # an earlier order, still running
        self.open_talk(r)
        g.open_pie(['Joke'], target=self.bench, caller=self.person, window=1)
        self.tick_until(r, lambda: r.finished)
        self.assertIn('no action was queued', r.failed)

    def test_a_submenu_entry_that_closes_the_menu_fails(self):
        g = self.g
        g.open_pie(['Talk...'], submenus=('Talk...',))
        r = self.runner('choose Talk...\nwait 1')
        self.tick(r, 2)
        g.highlight(0)
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        g.close_pie()
        self.tick(r, 20)
        self.assertFalse(r.finished)  # an empty menu for a moment is not yet a failure
        self.tick_until(r, lambda: r.finished, most=250)
        self.assertIn('closed', r.failed)

    def test_choose_waits_while_the_pie_window_fills(self):
        g = self.g
        g.open_pie(['Sit'], target=self.bench, caller=self.person)
        items = g.m.word(g.pies[0] + 0x190)
        g.m.put_words(g.pies[0] + 0x190, 0, 0)  # entries are there; the window shows nothing yet
        r = self.runner('choose Sit\nwait 1')
        self.tick(r, 3)
        self.assertFalse(r.failed)
        g.m.put_words(g.pies[0] + 0x190, items, items + 0x28)
        g.highlight(0)
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        self.assertIsNone(r.failed)

    def test_click_object_checks_the_menu_its_window_shows(self):
        g = self.g
        floor = g.add_object(29, [0, 0])
        g.hover(173)
        r = self.runner('click-object 173\nwait 1')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        g.open_pie(['Sit'], target=self.bench, caller=self.person)
        g.m.put_words(g.pies[0] + 0x190, 0, 0)  # the bench's entries; its window shows nothing yet
        self.tick(r, 8)
        self.assertFalse(self.rows('pie'))
        g.open_pie(['Go Here'], target=floor, caller=self.person)  # the menu that fills is the floor's
        self.tick_until(r, lambda: self.rows('wrong_menu'))
        self.assertEqual(self.rows('pie')[-1]['entries'][0]['label'], 'Go Here')
        self.assertEqual(self.rows('wrong_menu')[0]['targets'], [29])

    def test_choose_presses_only_the_item_it_moved_to(self):
        g = self.g
        g.open_pie(['Talk...', 'Dance...'], target=self.person, caller=self.person, submenus=('Talk...', 'Dance...'))
        r = self.runner('choose Talk...\nwait 1')
        self.tick_until(r, lambda: 'mousemove' in self.last_command())
        # Another menu takes the window while the move settles; its first item is highlighted.
        g.open_pie(['Hug...'], target=self.person, caller=self.person, submenus=('Hug...',))
        g.highlight(0)
        self.tick_until(r, lambda: r.finished, most=100)
        self.assertIn('not highlighted', r.failed)
        self.assertFalse(any('mousedown' in c for c in self.agent.commands))

    def test_an_item_by_its_place(self):
        items = [dict(label='Joke'), dict(label='Gossip')]
        self.assertEqual((inp.match_item(items, '@1'), inp.match_item(items, '@2'), inp.match_item(items, 'gossip')),
                         (1, None, 1))
        self.assertEqual(inp.parse_scenario('choose @1  # the first item')[0]['args'], ['@1'])
        self.assertEqual(inp.match_item([dict(label='@\u00b2')], '@\u00b2'), 0)  # not a place: a label

    def test_an_active_window_with_no_items_is_no_menu(self):
        g = self.g
        g.open_pie(['Dance...', 'Talk...'], submenus=('Talk...',))
        g.highlight(1)
        # The submenu window is active but shows nothing (paged past its end):
        # the parent's items, still there, are not the menu.
        g.m.put_words(g.picker + inp.PICKER_ACTIVE_PIE, g.pies[1])
        self.assertIsNone(inp.read_pie(g.m))
        g.m.put_words(g.picker + inp.PICKER_ACTIVE_PIE, 0)  # the picker names neither window
        self.assertEqual(inp.read_pie(g.m)['pointer'], g.pies[0])

    def test_a_missing_entry_fails_without_raising(self):
        self.g.open_pie(['Sit'])
        r = self.runner('choose Dance\nwait 1')
        self.tick(r)
        self.assertTrue(r.finished)
        self.assertIn("no entry 'Dance'", r.failed)
        self.assertEqual(self.rows('step')[-1]['status'], 'failed')

    def test_buy_mode_and_a_placed_object(self):
        g = self.g
        g.m.put_words(g.sim + 0x90, 2542)
        r = self.runner('press 93 458\nexpect-mode buy within 20\nexpect-new-object within 20')
        self.tick(r)
        self.assertIn('mousedown 1', self.last_command())
        g.m.put(g.sim + 0x36, b'\x02\x00')
        self.tick_until(r, lambda: self.rows('mode'))
        self.tick(r, 3)
        self.assertFalse(r.finished)
        g.add_object(400, [0x300, 0x280])
        g.m.put_words(g.sim + 0x90, 2242)
        self.tick_until(r, lambda: r.finished)
        self.assertIsNone(r.failed)
        placed = self.rows('placed')[0]
        self.assertEqual(([o['id'] for o in placed['objects']], placed['cost']), ([400], 300))
        self.assertEqual(len(bytes.fromhex(placed['objects'][0]['definition'])), 0xa6)
        self.assertEqual(len(placed['objects'][0]['data']), 72)

    def test_a_purchase_reusing_a_sold_objects_id(self):
        g = self.g
        r = self.runner('wait 2\nexpect-new-object within 20')
        self.tick(r)  # the registry is noted: the bench and the person
        g.remove_object(173)  # sold
        self.tick(r)
        g.add_object(173, [0x300, 0x280])  # bought; the game gives it the free ID
        self.tick_until(r, lambda: r.finished)
        self.assertIsNone(r.failed)
        self.assertEqual([o['id'] for o in self.rows('placed')[0]['objects']], [173])

    def test_expect_mode_times_out(self):
        r = self.runner('expect-mode buy within 3')
        self.tick(r, 5)
        self.assertIn('not 2', r.failed)

    def test_expect_queued_times_out(self):
        r = self.runner('expect-queued 21 within 3')
        self.tick(r, 5)
        self.assertIn('within 3', r.failed)

    def test_answer_dialog_is_driven_by_the_dialog_stops(self):
        # The welcome notices show inside lot update 1, before any update
        # returns, and hold it: the runner must start and go on at dialog stops.
        r = self.runner('answer-dialog 400 400\nanswer-dialog 400 378\nwait 1')
        r.on_dialog('dialog_show', 1)
        self.assertEqual(len(self.agent.commands), 1)
        self.assertIn('400 400', self.agent.commands[0])
        r.on_dialog('dialog_result', 1)
        self.assertEqual(len(self.agent.commands), 1)  # the next dialog is not up yet
        r.on_dialog('dialog_show', 1)
        self.assertIn('400 378', self.agent.commands[1])
        r.on_dialog('dialog_result', 1)
        self.assertFalse(r.finished)
        self.tick(r)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)

    def test_answer_dialog_presses_a_dialog_already_up(self):
        r = self.runner('wait 1\nanswer-dialog 490 378')
        r.on_dialog('dialog_show', 2)  # during the wait: not pressed yet
        self.assertEqual(self.agent.commands, [])
        self.tick(r)
        self.assertEqual(len(self.agent.commands), 1)
        self.agent.finish_all()
        r.on_dialog('dialog_result', 2)
        self.assertTrue(r.finished)

    def test_memory_errors_fail_the_step(self):
        r = self.runner('click-object 999')
        self.tick(r)
        self.assertIn('no object 999', r.failed)


class ReviewFindingsTest(RunnerBase):
    """Cases from the review of the first version."""

    def test_a_dialog_opened_by_a_click_does_not_deadlock(self):
        r = self.runner('click 10 20\nanswer-dialog 400 400')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        # The press opens a modal dialog: no update returns until it is answered.
        r.on_dialog('dialog_show', self.update + 1)
        self.assertIn('400 400', self.last_command())
        rows = self.rows('command_wait_ended')
        self.assertEqual(rows[0]['reason'], 'dialog_show')
        self.agent.finish_all()  # the guest ran both presses before the dialog closed
        r.on_dialog('dialog_result', self.update + 1)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)

    def test_memory_steps_wait_for_an_update_return(self):
        self.g.hover(173)
        r = self.runner('answer-dialog 400 400\nclick-object 173')
        r.on_dialog('dialog_show', 1)
        r.on_dialog('dialog_result', 1)  # no memory has been read yet
        self.assertIsNone(r.failed)
        self.assertEqual(len(self.agent.commands), 1)
        self.tick(r)
        self.assertIsNone(r.failed)
        self.assertIn('mousemove', self.last_command())

    def test_expect_queued_takes_pushes_in_order(self):
        g = self.g
        r = self.runner('expect-queued 21 173 within 5\nexpect-queued 21 173 within 5')
        self.queue_result(r, self.push(r, 8213), 2)
        self.queue_result(r, self.push(r, 8214), 3)
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, routine=8213, uid=2)
        self.put_action(self.person + 0x820, self.person, self.bench, routine=8214, uid=3)
        g.m.put_words(self.person + 0xa40, 0, 1)
        self.tick(r, 2)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)
        self.assertEqual([(t['where'], t['pushed_update']) for t in self.rows('taken')], [('active', 1), ('queue', 1)])

    def test_expect_queued_ignores_pushes_before_the_choice(self):
        g = self.g
        g.open_pie(['Sit'], target=self.bench, caller=self.person)
        r = self.runner('choose Sit\nexpect-queued 21 173 within 5')
        self.queue_result(r, self.push(r, 9999), 2)  # an older interaction, never held
        g.highlight(0)
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        self.queue_result(r, self.push(r, 8213), 3)
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, routine=8213, uid=3)
        self.tick_until(r, lambda: r.finished)
        self.assertIsNone(r.failed)
        self.assertEqual(self.rows('taken')[0]['active']['routine'], 8213)

    def test_expect_queued_finds_a_social_on_a_reused_object(self):
        g = self.g
        r = self.runner('choose "About Interests"\nexpect-queued 21 18 within 10')
        bella = g.add_object(18, [0x1b8, 0x198], person=True)
        thing = g.add_object(260, [0, 0])
        slot = g.m.word(g.registry + 0x1c) - 4
        g.open_pie(['About Interests'], target=bella, caller=self.person)
        g.highlight(0)
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        action = self.push(r, 8273, target=bella, menu=24)
        g.remove_object(260)
        g.m.put_words(slot, thing)  # the new interaction object reuses the ID and address
        g.objects[260] = thing
        queued = self.person + 0x820
        self.put_action(queued, self.person, thing, icon=bella, menu=21, routine=4235, uid=2)
        g.m.put_words(self.person + 0xa40, 0, 1)
        g.m.put(action, g.m.read(queued, 0x44))  # the game copies the converted action back
        self.queue_result(r, action)
        g.close_pie()
        self.tick_until(r, lambda: r.finished)
        self.assertIsNone(r.failed)
        push = self.rows('queued')[0]['action']
        self.assertEqual((push['uid'], push['target'], push['menu'], push['routine']), (0, 18, 24, 8273))
        self.assertEqual([(t['where'], t['uid']) for t in self.rows('taken')], [('queue', 2)])

    def test_a_receiving_social_cannot_confirm_the_push(self):
        r = self.runner('expect-queued 21 18 within 3')
        bella = self.g.add_object(18, [0x1b8, 0x198], person=True)
        self.queue_result(r, self.push(r, 8273, target=bella, menu=24), 2)
        thing = self.g.add_object(260, [0, 0])
        self.put_action(bella + inp.ACTIVE_ACTION, bella, thing, icon=self.person, menu=50, routine=4236, uid=3)
        self.tick_until(r, lambda: r.finished)
        self.assertIn('neither', r.failed)
        self.assertFalse(self.rows('taken'))

    def test_two_social_choices_for_the_same_person_match_their_own_uid(self):
        r = self.runner('expect-queued 21 18 within 5\nexpect-queued 21 18 within 5')
        g = self.g
        bella = g.add_object(18, [0x1b8, 0x198], person=True)
        first = self.push(r, 8273, target=bella, menu=24)
        second = self.push(r, 8274, target=bella, menu=25, frame=0x7ff00)
        self.queue_result(r, second, 4, frame=0x7ff00)
        self.queue_result(r, first, 2)
        self.assertEqual([(q['update'], q['push']) for q in self.rows('queued')], [(1, 1), (1, 2)])
        self.assertEqual([(q['push'], q['uid']) for q in self.rows('queue_result')], [(2, 4), (1, 2)])
        other = g.add_object(261, [0, 0])
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, other, icon=bella, routine=4235, uid=4)
        self.tick(r)
        self.assertFalse(self.rows('taken'))
        thing = g.add_object(260, [0, 0])
        self.put_action(self.person + 0x820, self.person, self.bench, routine=8213, uid=1)
        self.put_action(self.person + 0x820 + 0x44, self.person, thing, icon=bella, routine=4235, uid=2)
        g.m.put_words(self.person + 0xa40, 0, 2)
        self.tick_until(r, lambda: r.finished)
        self.assertIsNone(r.failed)
        self.assertEqual([(t['where'], t['uid']) for t in self.rows('taken')], [('queue', 2), ('active', 4)])

    def test_a_uid_taken_in_the_queue_cannot_be_taken_again_when_active(self):
        r = self.runner('expect-queued 21 18 within 5\nexpect-queued 21 18 within 5')
        g = self.g
        bella = g.add_object(18, [0x1b8, 0x198], person=True)
        thing = g.add_object(260, [0, 0])
        self.queue_result(r, self.push(r, 8273, target=bella), 2)
        queued = self.person + 0x820
        self.put_action(queued, self.person, thing, icon=bella, routine=4235, uid=2)
        g.m.put_words(self.person + 0xa40, 0, 1)
        self.tick(r)
        self.queue_result(r, self.push(r, 8274, target=bella), 2)
        g.m.put(self.person + inp.ACTIVE_ACTION, g.m.read(queued, 0x44))
        g.m.put_words(self.person + 0xa40, 1, 1)
        self.tick_until(r, lambda: r.finished)
        self.assertEqual([(t['where'], t['uid']) for t in self.rows('taken')], [('queue', 2)])
        self.assertIn('already confirmed an earlier push', r.failed)

    def test_a_rejected_push_does_not_take_a_matching_active_action(self):
        r = self.runner('expect-queued 21 173 within 3')
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, routine=8213, uid=5)
        action = self.push(r, 8213)
        self.tick(r)
        self.assertFalse(self.rows('taken'))  # still waiting for the call to return
        self.queue_result(r, action, eax=0x100)  # al is zero
        self.tick(r)
        self.assertTrue(r.finished)
        self.assertIn('did not queue', r.failed)
        self.assertFalse(self.rows('taken'))
        self.assertEqual([(q['accepted'], q['uid']) for q in self.rows('queue_result')], [(False, None)])

    def test_an_unreadable_result_uid_cannot_take_an_unrelated_action_with_the_same_fields(self):
        r = self.runner('expect-queued 21 173 within 3')
        m = self.g.m
        action = self.push(r, 8213)
        m.put_words(action + 0x3c, 6)
        del m.bytes[action + 0x3c]  # the UID read fails at the return
        self.queue_result(r, action)
        # The push with uid 6 was cancelled. A different Sit is active.
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, routine=8213, uid=7)
        self.tick(r)
        self.assertTrue(r.finished)
        self.assertEqual(r.failed, 'the uid the game gave the action pushed on 21 could not be read')
        self.assertFalse(self.rows('taken'))

    def test_an_accepted_push_with_a_zero_uid_fails_at_once(self):
        r = self.runner('expect-queued 21 173 within 3')
        self.queue_result(r, self.push(r, 8213), 0)
        self.tick(r)
        self.assertTrue(r.finished)
        self.assertEqual(r.failed, 'the uid the game gave the action pushed on 21 could not be read')
        self.assertFalse(self.rows('taken'))

    def test_a_refused_unreadable_push_is_still_logged(self):
        r = self.runner('wait 1')
        r.on_queue_push(1, True, self.g.m, dict(edi=0x10, esi=0x20, ebp=0x80000))
        r.on_queue_result(self.g.m, dict(eax=0, ebp=0x80000))
        self.assertEqual(self.rows('queued_unread')[0]['push'], 1)
        self.assertEqual([(q['push'], q['accepted'], q['uid']) for q in self.rows('queue_result')],
                         [(1, False, None)])

    def test_a_push_whose_person_cannot_be_read_is_not_passed_over(self):
        r = self.runner('expect-queued 21 173 within 3')
        r.on_queue_push(1, False, self.g.m, dict(edi=0x10, esi=0x20, ebp=0x80000))  # nothing readable there
        r.on_queue_result(self.g.m, dict(eax=1, ebp=0x80000))
        self.queue_result(r, self.push(r, 8213), 6)
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, routine=8213, uid=6)
        self.tick_until(r, lambda: r.finished)
        self.assertIn('(or another person) could not be read', r.failed)
        self.assertFalse(self.rows('taken'))
        self.assertEqual(self.rows('queued_unread')[0]['step'], 0)

    def test_a_refused_push_still_logs_the_uid_it_used(self):
        r = self.runner('expect-queued 21 173 within 3')
        self.queue_result(r, self.push(r, 8213), 6, eax=0)
        self.tick(r)
        self.assertIn('did not queue', r.failed)
        self.assertEqual([(q['accepted'], q['uid']) for q in self.rows('queue_result')], [(False, 6)])

    def test_a_push_whose_return_is_never_seen_says_so(self):
        r = self.runner('expect-queued 21 173 within 3')
        self.push(r, 8213)
        self.tick_until(r, lambda: r.finished)
        self.assertIn('never seen to return', r.failed)

    def test_a_push_that_keeps_its_old_uid_cannot_take_that_action(self):
        r = self.runner('expect-queued 21 18 within 3')
        g = self.g
        bella = g.add_object(18, [0x1b8, 0x198], person=True)
        thing = g.add_object(260, [0, 0])
        action = g.m.alloc(0x44)
        self.put_action(action, self.person, bella, routine=8273, uid=2)
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, thing, icon=bella, routine=4235, uid=2)
        r.on_queue_push(1, False, g.m, dict(edi=self.person, esi=action, ebp=0x80000))
        self.queue_result(r, action)
        self.tick_until(r, lambda: r.finished)
        self.assertIn('the uid it had before the call (2)', r.failed)
        self.assertFalse(self.rows('taken'))

    def test_expect_queued_finds_a_uid_among_undecodable_actions(self):
        r = self.runner('expect-queued 21 173 within 3')
        self.queue_result(r, self.push(r, 8213), 6)
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, 0x10, uid=9)
        self.put_action(self.person + 0x820, self.person, 0x20, uid=8)
        self.put_action(self.person + 0x820 + 0x44, self.person, self.bench, routine=8213, uid=6)
        self.g.m.put_words(self.person + 0xa40, 0, 2)
        self.tick(r)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)
        taken = self.rows('taken')[0]
        self.assertEqual((taken['where'], taken['uid']), ('queue', 6))
        self.assertIn('MemoryError', taken['error'])

    def test_expect_queued_fails_on_the_first_unreadable_push_on_the_person(self):
        r = self.runner('expect-queued 21 173 within 3')
        self.queue_result(r, self.push(r, 8213, target=0x10), 6)
        self.queue_result(r, self.push(r, 8213), 7)
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, routine=8213, uid=7)
        self.tick(r)
        self.assertTrue(r.finished)
        self.assertIn('the action pushed on 21 could not be read', r.failed)
        self.assertEqual(self.rows('queued_unread')[0]['person'], 21)
        self.assertFalse(self.rows('taken'))

    def test_expect_queued_fails_on_the_first_push_on_the_person_with_a_wrong_target(self):
        r = self.runner('expect-queued 21 173 within 3')
        other = self.g.add_object(29, [0, 0])
        self.queue_result(r, self.push(r, 8213, target=other), 6)
        self.queue_result(r, self.push(r, 8213), 7)
        self.put_action(self.person + inp.ACTIVE_ACTION, self.person, self.bench, routine=8213, uid=7)
        self.tick(r)
        self.assertTrue(r.finished)
        self.assertIn('aimed at 29 (expected 173)', r.failed)
        self.assertFalse(self.rows('taken'))

    def test_an_unreadable_push_is_logged_not_raised(self):
        r = self.runner('wait 1')
        r.on_queue_push(1, True, self.g.m, dict(edi=0x10, esi=0x20, ebp=0x80000))
        self.assertEqual(self.rows('queued_unread')[0]['action_pointer'], 0x20)
        self.assertEqual([p['kind'] for p in r.pushes], ['queued_unread'])

    def test_an_unreadable_push_then_a_dialog_does_not_deadlock(self):
        g = self.g
        g.open_pie(['Sit'], target=self.bench, caller=self.person)
        g.highlight(0)
        r = self.runner('choose Sit\nanswer-dialog 400 400')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        r.on_queue_push(self.update + 1, False, g.m, dict(edi=0x10, esi=0x20, ebp=0x80000))
        self.assertTrue(self.rows('queued_unread'))
        r.on_dialog('dialog_show', self.update + 1)
        self.assertIn('400 400', self.last_command())

    def test_a_dialog_opened_by_a_choice_does_not_deadlock(self):
        g = self.g
        g.open_pie(['Sit'], target=self.bench, caller=self.person)
        g.highlight(0)
        r = self.runner('choose Sit\nanswer-dialog 400 400')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        self.push(r, 8213)
        r.on_dialog('dialog_show', self.update + 1)  # the press is still settling, or running
        self.assertIn('400 400', self.last_command())
        self.agent.finish_all()
        r.on_dialog('dialog_result', self.update + 1)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)

    def test_a_late_push_then_a_dialog_does_not_deadlock(self):
        g = self.g
        g.open_pie(['Sit'], target=self.bench, caller=self.person)
        g.highlight(0)
        r = self.runner('choose Sit\nanswer-dialog 400 400')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        self.tick(r, 3)  # the press has ended and settled; the game has not taken it yet
        self.assertEqual(r.waiting[0], 'push')
        self.push(r, 8213)
        r.on_dialog('dialog_show', self.update + 1)  # no update returns from here
        self.assertIn('400 400', self.last_command())
        self.agent.finish_all()
        r.on_dialog('dialog_result', self.update + 1)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)

    def test_a_last_shot_cut_short_by_a_dialog_is_still_converted(self):
        shots = Path(tempfile.mkdtemp())
        r = inp.Runner(inp.parse_scenario('shot last'), self.agent, self.log, shots_dir=shots)
        self.runners.append(r)
        self.tick(r, 1, finish=False)
        r.on_dialog('dialog_show', self.update + 1)
        self.assertFalse(r.finished)  # the shot's end is still to be seen
        self.fake_xwd(self.agent.dir / 'shot-last.xwd')
        self.agent.finish_all()
        r.on_dialog('dialog_result', self.update + 1)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)
        self.assertTrue((shots / 'last.png').exists())

    def test_a_last_command_failing_after_a_dialog_still_fails(self):
        r = self.runner('click 10 20')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        r.on_dialog('dialog_show', self.update + 1)
        self.assertFalse(r.finished)
        self.agent.codes = {2: 1}
        self.tick(r)
        self.assertIn('failed with 1', r.failed)

    def test_a_dialog_press_failing_after_the_dialog_closed_still_fails(self):
        r = self.runner('answer-dialog 400 400')
        r.on_dialog('dialog_show', 1)
        self.assertIn('400 400', self.last_command())
        r.on_dialog('dialog_result', 1)  # closed before the press's end was seen
        self.assertFalse(r.finished)
        self.agent.codes = {1: 1}
        self.tick(r)
        self.assertIn('failed with 1', r.failed)

    def test_shot_names_are_unique(self):
        with self.assertRaisesRegex(inp.ScenarioError, 'line 2: a shot named'):
            inp.parse_scenario('shot same\nshot same')

    def test_an_unfinished_scenario_is_logged_at_close(self):
        r = self.runner('wait 5')
        self.tick(r)
        r.close()
        self.runners.remove(r)
        self.assertEqual(self.rows('scenario')[-1]['status'], 'unfinished')

    def fake_xwd(self, path):
        header = [100, 7, 2, 16, 2, 1] + [0] * 6 + [4] + [0] * 6 + [0] + [0] * 5
        path.write_bytes(struct.pack('>25I', *header) + b'\x1f\x00\xe0\x07')

    def test_a_shot_running_when_a_dialog_shows(self):
        shots = Path(tempfile.mkdtemp())
        r = inp.Runner(inp.parse_scenario('shot before\nanswer-dialog 400 400'), self.agent, self.log, shots_dir=shots)
        self.runners.append(r)
        self.tick(r, 1, finish=False)
        r.on_dialog('dialog_show', self.update + 1)
        self.assertIn('400 400', self.last_command())  # pressed while the shot still runs
        self.fake_xwd(self.agent.dir / 'shot-before.xwd')
        self.agent.finish_all()
        r.on_dialog('dialog_result', self.update + 1)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)
        self.assertTrue((shots / 'before.png').read_bytes().startswith(b'\x89PNG'))
        self.assertFalse((self.agent.dir / 'shot-before.xwd').exists())

    def test_a_failure_seen_after_a_dialog_still_fails(self):
        r = self.runner('click 10 20\nanswer-dialog 400 400')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        r.on_dialog('dialog_show', self.update + 1)  # the press has not ended yet
        self.agent.codes = {2: 1}
        self.agent.finish_all()
        r.on_dialog('dialog_result', self.update + 1)
        self.assertIn('failed with 1', r.failed)

    def test_window_ids_are_checked(self):
        self.assertEqual(inp.check_window('0xe00013'), '0xe00013')
        for bad in ('0x1; printf injected; #', '$(id)', '0x1 2', ''):
            with self.assertRaises(inp.ScenarioError):
                inp.check_window(bad)
        with self.assertRaises(inp.ScenarioError):
            inp.Agent(tempfile.mkdtemp(), '0x1;reboot')

    def test_capture_refuses_to_resume_a_scenario(self):
        d = Path(tempfile.mkdtemp())
        (d / 's.txt').write_text('wait 1\n')
        capture = Path(inp.__file__).with_name('capture.py')
        for extra, message in ((['--resume'], 'cannot be resumed'), (['--window', '1;id'], 'not an X window')):
            run = subprocess.run([sys.executable, '-B', str(capture), '--container', 'none', '--output', str(d / 'out'),
                                  '--scenario', str(d / 's.txt'), '--guest-dir', str(d)] + extra,
                                 capture_output=True, text=True)
            self.assertEqual(run.returncode, 2, run.stderr)
            self.assertIn(message, run.stderr)
            self.assertFalse((d / 'out').exists())

    def test_drag_holds_button_across_midpoint_and_releases(self):
        r = self.runner('drag 100 200 300 400')
        self.tick_until(r, lambda: r.finished, most=40)
        self.assertIsNone(r.failed)
        cmds = self.rows('command')
        self.assertEqual([c['what'] for c in cmds], ['move', 'press', 'move', 'move', 'release'])
        self.assertEqual([(c['x'], c['y']) for c in cmds],
                         [(100, 200), (100, 200), (200, 300), (300, 400), (300, 400)])
        self.assertEqual(cmds[1]['command'], 'xdotool mousedown 1')
        self.assertEqual(cmds[4]['command'], 'xdotool mouseup 1')
        self.assertGreaterEqual(cmds[2]['update'] - cmds[1]['update'], inp.HOLD_UPDATES)
        self.assertGreaterEqual(cmds[3]['update'] - cmds[2]['update'], inp.HOLD_UPDATES)
        self.assertGreaterEqual(cmds[4]['update'] - cmds[3]['update'], inp.HOLD_UPDATES)

    def test_zero_length_drag_does_not_move_while_held(self):
        r = self.runner('drag 100 200 100 200')
        self.tick_until(r, lambda: r.finished, most=30)
        self.assertIsNone(r.failed)
        cmds = self.rows('command')
        self.assertEqual([c['what'] for c in cmds], ['move', 'press', 'release'])

    def test_drag_releases_and_unblocks_when_dialog_shows_mid_drag(self):
        r = self.runner('drag 0 200 300 400\nanswer-dialog 400 400')
        self.tick_until(r, lambda: 'mousedown' in self.last_command())
        r.on_dialog('dialog_show', self.update + 1)
        cmds = self.rows('command')
        self.assertEqual([c['what'] for c in cmds], ['move', 'press', 'release', 'press'])
        self.assertEqual(cmds[2]['command'], 'xdotool mouseup 1')
        self.assertIn('400 400', cmds[3]['command'])
        self.assertFalse(r.finished)
        self.agent.finish_all()
        r.on_dialog('dialog_result', self.update + 1)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)

    def test_failed_release_is_retried(self):
        r = self.runner('click 10 20')
        self.agent.codes = {3: 1}
        self.tick_until(r, lambda: r.finished, most=30)
        self.assertIn('failed with 1', r.failed)
        releases = [c for c in self.rows('command') if c['what'] == 'release']
        self.assertEqual(len(releases), 2)
        self.assertEqual(releases[1]['reason'], 'step failed')
        self.assertEqual(releases[1]['command'], 'xdotool mouseup 1')

    def test_drag_releases_button_when_midpoint_move_fails(self):
        r = self.runner('drag 100 200 300 400')
        self.agent.codes = {3: 1}
        self.tick_until(r, lambda: r.finished, most=30)
        self.assertIn('failed with 1', r.failed)
        release = [c for c in self.rows('command') if c['what'] == 'release']
        self.assertEqual(len(release), 1)
        self.assertEqual(release[0]['reason'], 'step failed')
        self.assertEqual(release[0]['command'], 'xdotool mouseup 1')

    def test_expect_mode_build(self):
        self.g.m.put(self.g.sim + 0x36, struct.pack('<H', 1))
        r = self.runner('expect-mode build within 5')
        self.tick(r, 2)
        self.assertTrue(r.finished)
        self.assertIsNone(r.failed)
        self.assertEqual(self.rows('mode')[0]['mode'], 1)


class AgentScriptTest(unittest.TestCase):
    """The guest agent itself, run on the host with sh on a temporary folder."""

    def env(self, d):
        """The agent's folder and lock under d; on a host without flock (macOS),
        a stand-in that takes the same lock on fd 9 with fcntl.flock."""
        env = dict(os.environ, INPUT_DIR=str(d), INPUT_LOCK=str(d / 'agent.lock'))
        bin_dir = d / 'bin'
        bin_dir.mkdir()
        stand_in = bin_dir / 'flock'
        stand_in.write_text('#!/bin/sh\nexec "%s" -c "import fcntl; fcntl.flock(9, fcntl.LOCK_EX | fcntl.LOCK_NB)" '
                            '2>/dev/null\n' % sys.executable)
        stand_in.chmod(0o755)
        if not any((Path(x) / 'flock').exists() for x in os.environ.get('PATH', '').split(os.pathsep)):
            env['PATH'] = f'{bin_dir}{os.pathsep}{env["PATH"]}'
        return env

    def test_a_second_agent_exits(self):
        d = Path(tempfile.mkdtemp())
        script = Path(inp.__file__).with_name('guest') / 'input-agent.sh'
        env = self.env(d)
        first = subprocess.Popen(['sh', str(script)], env=env)
        try:
            end = time.time() + 10
            while not (d / 'agent.pid').exists() and time.time() < end:
                time.sleep(0.05)
            second = subprocess.run(['sh', str(script)], env=env, timeout=10)
            self.assertEqual(second.returncode, 0)
            self.assertEqual((d / 'agent.pid').read_text().strip(), str(first.pid))
        finally:
            first.kill()
            first.wait()
        # The lock goes with the process: a new agent starts.
        again = subprocess.Popen(['sh', str(script)], env=env)
        try:
            end = time.time() + 10
            while (d / 'agent.pid').read_text().strip() != str(again.pid) and time.time() < end:
                time.sleep(0.05)
            self.assertEqual((d / 'agent.pid').read_text().strip(), str(again.pid))
        finally:
            again.kill()
            again.wait()

    def test_the_agent_is_started_as_a_reserved_guest_job(self):
        share = Path(tempfile.mkdtemp())
        (share / 'cmd').mkdir()
        (share / 'cmd' / '00001.done').write_text('0\n')  # a number the guest would skip
        self.assertTrue(inp.Agent(share, '0x1').start())
        self.assertEqual([p.name for p in sorted((share / 'cmd').glob('*.sh'))], ['00002.sh'])
        self.assertEqual((share / 'cmd' / '00002.sh').read_text(), 'sh /research/input-agent.sh\n')
        self.assertTrue((share / 'input-agent.sh').exists())

    def test_two_agents_run_each_command_once(self):
        d = Path(tempfile.mkdtemp())
        # An ended command an older agent left as .cmd must not run again.
        (d / '00000.cmd').write_text(f'echo stale >> {d}/ran\n')
        (d / '00000.done').write_text('0\n')
        script = Path(inp.__file__).with_name('guest') / 'input-agent.sh'
        env = dict(os.environ, INPUT_DIR=str(d))
        agents = [subprocess.Popen(['sh', str(script)], env=env) for _ in range(2)]
        try:
            agent = inp.Agent(d.parent, '0x1')
            agent.dir = d
            agent.n = agent.numbered()
            ns = [agent.submit(f'echo {i} >> {d}/ran') for i in range(20)]
            end = time.time() + 20
            while time.time() < end and any(agent.result(n) is None for n in ns):
                time.sleep(0.05)
            self.assertEqual([agent.result(n) for n in ns], [0] * 20)
            self.assertEqual(sorted(int(x) for x in (d / 'ran').read_text().split()), list(range(20)))
            self.assertEqual(agent.numbered(), 20)
            self.assertEqual([p.name for p in d.glob('*.cmd')], ['00000.cmd'])  # left alone
        finally:
            for a in agents:
                a.kill()
                a.wait()


if __name__ == '__main__':
    unittest.main()
