#!/usr/bin/env python3
"""Stop the original at its lot updates and save the state after each one.

Run on the host while the VM container runs the game, before the house is
entered:

  capture.py --container sims-harness-vm --output <new dir> --lot 5 \
      --updates 1000 --full 1-100,110-1000/10

--resume takes over a guest that an earlier run left stopped and continues
the same output directory and numbering.

Breakpoints (hardware, in QEMU's stub; no guest memory or register is
written): the lot update's entry 0x46d345 (until the first one on the lot,
and again while a watch, a due --dump or a --scenario needs it; see below)
and its end 0x46d51d (the body is done, the epilogue not yet run); the
dialog functions 0x436da0 (a record is shown), 0x4370be (a button's result)
and 0x43718f (a script reads the answer), which are logged only.

Output: entry-0000/ (the state at the entry of the first lot update on the
lot, which is the state the load ends in) and update-NNNN/ (the state at the
end of the N-th lot update), each with state.json.gz (the scheduler, random
streams, globals and every object, as the old prototype's tick capture read them),
full-state.json.gz (the registry, the neighborhood and the object pointers)
and pages.json.gz (the memory pages of the people's animators);
updates.jsonl (one line per lot update: tick, full pass or not, the three
random streams); events.jsonl (dialogs and other stops); status.json.

--watch NAME=EXPR[:SIZE] (repeatable) also sets a write watchpoint (GDB's Z2;
nothing is written) on SIZE bytes (default 4) at the address EXPR names, as
soon as EXPR can be worked out from the baseline on (the first lot update,
where the game's CR3 is known), and logs a row to watch.jsonl at each stop
at its watchpoint, with the stopped instruction's registers, its
frame-pointer chain and the top of its stack, the lot update it falls in
and whether it falls inside the lot update's body. A row is what the stop
and memory show, not a write: its address is the watchpoint base the stop
gives (see the README for its match). A watch stop before the baseline is
logged as an unknown_watch event, and one in another process (another CR3)
as a foreign_watch event, neither in watch.jsonl.
EXPR is a start, a hexadecimal address, object:<id> (the object with that
ID) or person:<id> (the same, once it is a person), followed by steps: +HEX
adds, @HEX reads the word at the address plus HEX. For example
person:259@0xadc+0x88 is the direction of person 259's animator. With --watch
the lot update's entry stays armed, so that each write can be placed inside
or outside the update. A watch that goes through an object or a pointer is
worked out again at every breakpoint stop in the game (not at a stop at a
watchpoint); when the address moved (an
animator was replaced), the watchpoint is moved (a watch_moved event), and
when it cannot be worked out it waits again (watch_lost). Writes between the
change and that stop are not seen.

--late-watch FROM[-TO],NAME=EXPR[:SIZE] is a watch set at the end of lot
update FROM - 1 (at the entry of update 1 for FROM 1, once the game's
process is known) and, if TO is given, cleared at the end of update TO. So
it covers the stops between the rows of updates FROM - 1 and TO in
updates.jsonl, which are those of the writes that change rows FROM to TO: a
frame drawn between two updates changes the row of the update after it. A
stop there is logged as any watch's is: one inside a lot update's body with
that update and in_update true (from update FROM on, so for FROM 1 a stop
in update 1 has update 1), one between two updates with the update before
it and in_update false. A moving watch misses what the paragraph above says it
misses; a stop at a watchpoint does not work its address out again, as
for --watch, but a watch is not moved at the stop where it ends. A run
taken over (--resume) sets the watches whose updates have come at its first
stop; one that goes through an object waits for the next entry or return
if that stop does not give the simulator's pointer, and none is set while
the guest is stopped in another process. A
row before the taken-over run knows whether the guest is inside a lot
update has update null (after_update is the last update recorded). A
late watch's watch_ended event says whether it was ever set, by this run or
one it took over (set false: no stop in its updates was seen). The lot
update's entry is a breakpoint for a late watch only while it is set or
waits to be set in its updates (one whose address cannot be worked out yet
is tried again there); the baseline, --watch, --scenario and a due --dump
arm it too. A late watch is set (or moved) only at a stop on the lot
captured; at a stop on another lot, one that goes through an object is
taken off (watch_lost with elsewhere true) until it can be set there
again, and that includes a stop at a watchpoint on another lot: the row
is logged with no moving late watch's name, and such a watch is not set
through the simulator seen before that stop until this lot's next entry or
return. Each watch.jsonl row says the lot it was
seen on (lot). A run taken over does not count a late watch an earlier revision
set (those recorded no lot): it ends with set false.
So a late watch on a busy address (the random state) costs nothing while
the load and the other updates run.

The executable digests that guest/launch.sh verified (share/game-identity.txt)
are recorded in status.json and as a game_identity event.

Sparse recordings (a whole game day): --full saves a fixed set of updates
(for example every tenth, 10-50000/10). --around N also saves the update in
which something happens and the N updates after it: an object appears on or
leaves the registry (an objects_changed event names them), a dialog is shown,
or the hour (global 0) turns (an hour_turned event; the day turns at hour
24). --before-hour B also saves the last B updates before each hour turns,
worked out from the clock (globals 5 and 6; a full pass adds 2 seconds).
Every line of updates.jsonl also holds the clock (globals 0, 5 and 6), the
date globals 1 to 3 and 7 to 8, and the number of objects in the registry;
a saved update's line says why it was saved (saved_for). --answer-stuck S
(with --guest-dir and --window) presses Return in the game window when a
dialog shown in update --answer-from or later has had no result for S
seconds, and again every S seconds (at most three times); each press is
logged (dialog_auto_answer, and input-log.txt), and a result cancels its own
dialog's presses not yet made. dialog_result's result is the function's
argument (the record's status at that stop is the old value).

--scenario FILE (with --guest-dir) drives the game through a scripted input
scenario (input.py has the format) and logs it to input.jsonl. It adds a
breakpoint at 0x48ddcb, where the pie menu pushes the chosen action on a
person, and keeps the lot update's entry armed, so each push is placed
before or inside a lot update. Input reaches the game only as pointer moves,
button presses and keys; nothing is written to guest memory.
"""
import argparse
import gzip
import json
import os
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rsp import Remote  # noqa: E402
from qmp import Qmp  # noqa: E402
import qemu_retained_owners  # noqa: E402
import qemu_neighborhood_owners as neighborhood_owners  # noqa: E402
import input as scenario_input  # noqa: E402
import guest as guest_jobs  # noqa: E402

ENTRY, RETURN = 0x46d345, 0x46d51d
# The clock's globals (see crates/sims-vm/src/tick.rs) and those of the date.
HOURS, MINUTES, SECONDS = 0, 5, 6
DATE_GLOBALS = (1, 2, 3, 7, 8)
# The most bytes one guest store is taken to write: a 16-byte SSE store. A store that crosses into a
# page writes no further into it than this.
DIALOG_SHOW, DIALOG_RESULT, DIALOG_READ = 0x436da0, 0x4370be, 0x43718f
OWNER, GLOBAL_RNG, ACTION_SERIAL, CRT_TLS_INDEX = 0x6e48bc, 0x6e54ac, 0x6e4904, 0x6db360


class Memory:
    """Reads by 4 KiB page through the stub, each page once per stop."""

    def __init__(self, remote):
        self.remote = remote
        self.pages = {}

    def read(self, p, n):
        if p < 0x10000 or n < 0 or n > 1 << 24:
            raise MemoryError(f'invalid range {p:#x}+{n}')
        out = bytearray()
        while n:
            page = p & ~4095
            if page not in self.pages:
                self.pages[page] = self.remote.read(page, 4096)
            take = min(n, 4096 - (p - page))
            out.extend(self.pages[page][p - page:p - page + take])
            p += take
            n -= take
        return bytes(out)

    def span(self, p, n):
        """Read n bytes at p; the pages they lie on (the last is that of p + n - 1)."""
        self.read(p, n)
        return set(range(p & ~4095, ((p + n - 1) & ~4095) + 1, 4096))

    def word(self, p):
        return struct.unpack('<I', self.read(p, 4))[0]

    def half(self, p):
        return struct.unpack('<H', self.read(p, 2))[0]

    def byte(self, p):
        return self.read(p, 1)[0]

    def words(self, p, n, width=4):
        if not 0 <= n <= 65536:
            raise MemoryError('invalid vector length')
        return list(struct.unpack('<' + ('H' if width == 2 else 'I') * n, self.read(p, n * width))) if n else []

    def object_id(self, p):
        return self.half(p + 0xf0) if p else 0

    # The readers below are capture_qemu_ticks.py's (the old prototype's
    # scripts/research), unchanged in what they read.
    def relationship_table(self, p):
        def vector(pointer, max_items):
            first, end, capacity = self.words(pointer, 3)
            if first == 0:
                if end or capacity:
                    raise RuntimeError('partial null relationship vector')
                return first, end, capacity
            if first < 0x10000 or end < first or capacity < end or (end - first) % 4 or (end - first) // 4 > max_items:
                raise RuntimeError('invalid relationship vector extent')
            return first, end, capacity
        first, end, capacity = vector(p, 32767)
        rows, order = {}, []
        for record in self.words(first, (end - first) // 4):
            start, stop, cap = vector(record, 256)
            target = struct.unpack('<i', self.read(record + 12, 4))[0]
            order.append(target)
            rows[str(target)] = [struct.unpack('<i', struct.pack('<I', v))[0] for v in self.words(start, (stop - start) // 4)]
        return dict(order=order, rows=rows)

    def action(self, p):
        return dict(uid=self.word(p + 60), caller=self.object_id(self.word(p + 4)), target=self.object_id(self.word(p + 8)),
                    icon=self.object_id(self.word(p + 12)), menu=self.word(p + 16), args=self.words(p + 20, 4, 2),
                    priority=self.word(p + 28), routine=self.half(p + 32), attenuation_bits=self.word(p + 36),
                    flags=self.word(p + 64))

    def object(self, p, flags):
        definition = self.word(p + 0x118)
        first, end = self.word(p + 0x10), self.word(p + 0x14)
        assert end >= first and (end - first) % 4 == 0 and end - first <= 4096
        frames = []
        for q in range(first, end, 4):
            f = self.word(q)
            argc, locals_ = self.byte(f + 7), self.byte(f + 6)
            frames.append(dict(tree=self.half(f), node=self.half(f + 2), stack_object=self.half(f + 4),
                               continuation=self.word(f + 8), code_owner_pointer=self.word(f + 12),
                               args=self.words(f + 16, argc, 2), locals=self.words(f + 16 + argc * 2, locals_, 2)))
        value = dict(id=self.object_id(p), pointer=p, registry_word=flags, definition=self.read(definition, 0xa6).hex(),
                     base_storage=self.read(p, 0x188).hex(), data=self.words(p + 0x4a, 72, 2),
                     temps=self.words(p + 0x3a, 8, 2),
                     attributes=self.words(self.word(p + 0x2c), self.word(p + 0x30), 2),
                     last_result=self.byte(p + 0x20), frames=frames,
                     relationships=self.relationship_table(p + 0xe4))
        if self.half(definition + 0x12) == 2:
            first, end = self.word(p + 0xa40), self.word(p + 0xa44)
            assert (end - first) & 0xffffffff <= 8
            uses, uses_end = self.word(p + 0xb00), self.word(p + 0xb04)
            assert uses_end >= uses and (uses_end - uses) % 12 == 0 and uses_end - uses <= 12288
            value['person'] = dict(
                data=self.words(p + 0x58c, 256, 2), motives_bits=self.words(p + 0x78c, 16),
                previous_motives_bits=self.words(p + 0x7cc, 16), active=self.action(p + 0xa48),
                previous=self.action(p + 0xa8c),
                queue=[self.action(p + 0x820 + (i & 7) * 0x44) for i in range(first, end)],
                uses=[[self.object_id(self.word(q)), self.word(q + 4), self.byte(q + 8)] for q in range(uses, uses_end, 12)],
                last_decay=self.word(p + 0x810), room=struct.unpack('<h', self.read(p + 0xafc, 2))[0])
        if self.half(definition + 0x12) == 8:
            # A door's portal costs (+0x1ac, a vector of floats, one column per
            # number in object data word 6), as their bit patterns.
            first, end = self.word(p + 0x1ac), self.word(p + 0x1b0)
            assert end >= first and (end - first) % 4 == 0 and end - first <= 4096
            value['portal_bits'] = self.words(first, (end - first) // 4)
        return value

    def crt(self, teb):
        index = self.word(CRT_TLS_INDEX)
        assert index < 64
        p = self.word(teb + 0xe10 + index * 4)
        return dict(teb=teb, tls_index=index, thread_data=p, state=self.word(p + 0x14), thread_id=self.word(p))

    def lot(self):
        owner = self.word(OWNER)
        n = self.word(owner + 0x24) if owner else 0
        return self.word(n + 0x164) if n else None

    def clock(self, sim):
        """[hours, minutes, seconds] and the date globals (1, 2, 3, 7, 8)."""
        g = self.words(sim + 0x10, 9, 2)
        return [g[HOURS], g[MINUTES], g[SECONDS]], [g[i] for i in DATE_GLOBALS]

    def registry(self, sim):
        """The registry's object pointers, in order (zeros left out)."""
        reg = self.word(sim + 0xb0)
        first, end = self.word(reg + 0x18), self.word(reg + 0x1c)
        assert end >= first and (end - first) % 4 == 0 and end - first < 131072
        return [p for p in self.words(first, (end - first) // 4) if p]

    def summary(self, sim, teb):
        return dict(tick=self.word(sim + 0x98), lot_rng=self.word(sim + 0xac), global_rng=self.word(GLOBAL_RNG),
                    crt=self.crt(teb)['state'], scheduler_cursor=self.word(sim + 0x1a8))

    def snapshot(self, kind, regs, sim, teb):
        reg = self.word(sim + 0xb0)
        first, end, flags = self.word(reg + 0x18), self.word(reg + 0x1c), self.word(reg + 0x24)
        assert end >= first and (end - first) % 4 == 0 and end - first < 131072
        state = dict(schema=2, kind=kind, registers=regs, eip=regs['eip'], tick=self.word(sim + 0x98),
                     lot=self.lot(), global_rng=self.word(GLOBAL_RNG), lot_rng=self.word(sim + 0xac),
                     action_serial=self.word(ACTION_SERIAL), scheduler_cursor=self.word(sim + 0x1a8),
                     paused=self.half(sim + 0x32), mode=self.half(sim + 0x36), globals=self.words(sim + 0x10, 64, 2),
                     simulator_storage=self.read(sim, 0x1b0).hex(), crt=self.crt(teb),
                     budget_balance=struct.unpack('<i', self.read(sim + 0x90, 4))[0], objects=[])
        for q in range(first, end, 4):
            p = self.word(q)
            if p:
                state['objects'].append(self.object(p, self.word(flags + q - first)))
        full = dict(objects=[dict(id=o['id'], pointer=o['pointer']) for o in state['objects']])
        for row in full['objects']:
            if any(o['id'] == row['id'] and 'person' in o for o in state['objects']):
                row['animator'] = self.word(row['pointer'] + 0xadc)
        full['retained_owners'] = qemu_retained_owners.capture(self, state, sim, reg)
        full['neighborhood_owners'] = neighborhood_owners.capture(self)
        # The people's animators (person +0xadc) and their foreground layers.
        wanted = set()
        for o in state['objects']:
            if 'person' in o:
                a = self.word(o['pointer'] + 0xadc)
                wanted.add((o['pointer'] + 0xadc) & ~4095)
                if a:
                    wanted |= self.span(a, 0x2a0)
                    fg = self.word(a + 0xc)
                    if fg:
                        wanted |= self.span(fg, 0x70)
        pages = {str(k): self.pages[k].hex() for k in sorted(wanted)}
        return state, full, pages


class Watch:
    """A write watchpoint given as NAME=EXPR[:SIZE] (see the module's doc)."""

    def __init__(self, text):
        self.first, self.last = 1, None  # the lot updates it is set for (a late watch's)
        self.set = False  # whether it was ever set (a late watch's)
        self.name, _, rest = text.partition('=')
        if not rest.isascii():
            # int() takes other scripts' digits, which no one means here. The name may be anything.
            raise ValueError(f'not ASCII: {rest!r}')
        expr, _, size = rest.rpartition(':')
        if not size.isdigit() or expr in ('object', 'person'):
            # object:N or person:N alone is the selector, with the default size.
            expr, size = rest, '4'
        self.size = int(size)
        assert self.name and expr and self.size in (1, 2, 4, 8), text
        self.text = text
        tokens = []
        i = 0
        while i < len(expr):
            j = i + 1
            while j < len(expr) and expr[j] not in '+@':
                j += 1
            tokens.append(expr[i:j])
            i = j
        start, self.steps = tokens[0], [(t[0], int(t[1:], 16)) for t in tokens[1:]]
        assert all(op in '+@' for op, _ in self.steps), text
        self.person = start.startswith('person:')
        if start.startswith(('object:', 'person:')):
            self.object, self.start = int(start[7:]), None
        else:
            self.object, self.start = None, int(start, 16)
        self.address = None
        self.value = None
        self.pointer = None  # the object's pointer when last worked out
        self.moves = self.object is not None or any(op == '@' for op, _ in self.steps)

    @classmethod
    def late(cls, text):
        """A watch given as FROM[-TO],NAME=EXPR[:SIZE] (see the module's doc)."""
        span, comma, rest = text.partition(',')
        first, dash, last = span.partition('-')
        digits = lambda t: t.isascii() and t.isdigit()
        if not (comma and digits(first) and (digits(last) or not dash)):
            raise argparse.ArgumentTypeError(f'not FROM[-TO],NAME=EXPR[:SIZE]: {text!r}')
        try:
            w = cls(rest)
        except (AssertionError, ValueError, IndexError) as e:
            raise argparse.ArgumentTypeError(f'not a watch: {rest!r}') from e
        if not w.name or w.size not in (1, 2, 4, 8):  # checked by assertions, which python -O skips
            raise argparse.ArgumentTypeError(f'not a watch: {rest!r}')
        w.first, w.last = int(first), int(last) if dash else None
        if w.first < 1 or (w.last is not None and w.last < w.first):
            raise argparse.ArgumentTypeError(f'not 1 <= FROM <= TO: {text!r}')
        w.text = text
        return w

    def resolve(self, m, sim):
        """The address, or None while a pointer on the way is 0 or the object does not exist."""
        if self.object is None:
            a = self.start
        else:
            if sim is None:
                return None
            reg = m.word(sim + 0xb0)
            first, end = m.word(reg + 0x18), m.word(reg + 0x1c)
            if not (first <= end and end - first <= 131072):
                return None
            pointers = m.words(first, (end - first) // 4)
            if self.pointer in pointers and m.object_id(self.pointer) == self.object:
                a = self.pointer
            else:
                a = next((p for p in pointers if p and m.object_id(p) == self.object), 0)
            self.pointer = a or None
            if not a or (self.person and m.half(m.word(a + 0x118) + 0x12) != 2):
                return None
        for op, n in self.steps:
            if op == '+':
                a = (a + n) & 0xffffffff
            else:
                a = m.word(a + n)
                if a < 0x10000:
                    return None
        return a

    def read(self, m):
        return self.read_at(m, self.address)

    def read_at(self, m, address):
        return m.read(address, self.size).hex()


def frame_chain(m, ebp, n=24):
    """The return addresses along the frame-pointer chain from ebp (as far as it is readable)."""
    out = []
    for _ in range(n):
        if ebp < 0x10000:
            break
        try:
            out.append(m.word(ebp + 4))
            nxt = m.word(ebp)
        except MemoryError:
            break
        if nxt <= ebp:
            break
        ebp = nxt
    return out


def parse_set(text):
    """'1-100,110-1000/10' -> set of update numbers."""
    out = set()
    for part in filter(None, text.split(',')):
        step = 1
        if '/' in part:
            part, step = part.split('/')
            step = int(step)
        a, _, b = part.partition('-')
        out |= set(range(int(a), int(b or a) + 1, step))
    return out


def updates_to_hour(clock):
    """How many more full passes until the hour turns, from [hours, minutes, seconds] (a pass adds 2 seconds)."""
    _, minutes, seconds = clock
    return max(1, ((59 - minutes) * 60 + 60 - seconds + 1) // 2)


class DialogAnswerer:
    """Presses Return in the game window when a dialog has had no result for
    `after` seconds, and again every `after` seconds, at most `tries` times. It
    only writes guest jobs to the share (the guest runs while a modal dialog is
    up); the capture's stops tell it when a dialog is shown and answered. The
    dialogs are told apart by their record, so a result ends only its own
    dialog's presses.

    A press job checks for its own `.cancel` file right before it presses, and
    a result writes that file for every job of its dialog. The capture reports
    a result while the guest is stopped, so a job that has not reached its
    check by then never presses. One already past its check when the result
    stop came (the press that may have made the result) still lands."""

    def __init__(self, share, window, after, log, input_log, tries=3):
        self.share, self.window, self.after, self.log, self.input_log = share, window, after, log, input_log
        self.tries = tries
        self.lock = threading.Lock()
        self.open = {}  # record -> dict(serial, update, timer, jobs) of a dialog waiting for a result
        self.serial = 0

    def shown(self, update, record=None, jobs=()):
        """A dialog was shown. `jobs`: the press jobs an earlier run already handed to the guest for it (a
        resumed run), which its result cancels too; the presses go on from there."""
        with self.lock:
            self._end(record, 'shown_again')
            self.serial += 1
            self.open[record] = dict(serial=self.serial, update=update, timer=None, jobs=list(jobs))
            if len(jobs) < self.tries:
                self._schedule(record, len(jobs) + 1)

    def answered(self, record=None):
        with self.lock:
            self._end(record, 'answered')

    def _end(self, record, why):
        entry = self.open.pop(record, None)
        if not entry:
            return
        if entry['timer']:
            entry['timer'].cancel()
        for job in entry['jobs']:
            (Path(self.share) / 'cmd' / f'{job:05d}.cancel').write_text(why + '\n')
        if entry['jobs']:
            self.log(kind='dialog_auto_answer_cancelled', update=entry['update'], record=record, jobs=entry['jobs'],
                     why=why)

    def _schedule(self, record, n):
        entry = self.open[record]
        entry['timer'] = threading.Timer(self.after, self._press, (record, entry['serial'], n))
        entry['timer'].daemon = True
        entry['timer'].start()

    def _press(self, record, serial, n):
        with self.lock:
            entry = self.open.get(record)
            if not entry or entry['serial'] != serial:
                return
            update = entry['update']
            command = f'xdotool windowfocus --sync {self.window} sleep 1 key Return'
            script = (f'set -u\nexport DISPLAY=:98\nxdotool windowfocus --sync {self.window} sleep 1 || exit 4\n'
                      '[ -e "${0%.sh}.cancel" ] && { echo cancelled; exit 3; }\nxdotool key Return\n')
            job = guest_jobs.submit(self.share, script)
            entry['jobs'].append(job)
            with open(self.input_log, 'a') as f:
                f.write(f'{time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}\n'
                        f'auto answer (dialog of update {update}, press {n}): {command}\n')
            self.log(kind='dialog_auto_answer', update=update, record=record, press=n, job=job, command=command)
            if n < self.tries:
                self._schedule(record, n + 1)

    def close(self):
        with self.lock:
            for entry in self.open.values():
                if entry['timer']:
                    entry['timer'].cancel()
            self.open.clear()


def open_dialogs(rows):
    """{record: (update, [job])} of the dialogs shown in these events and given no result yet, with the press
    jobs handed to the guest for each (and not cancelled)."""
    out = {}
    for r in rows:
        if r['kind'] == 'dialog_show':
            out[r.get('record')] = (r['update'], [])
        elif r['kind'] == 'dialog_result':
            out.pop(r.get('record'), None)
        elif r['kind'] == 'dialog_auto_answer' and 'record' in r and r['record'] in out:
            out[r['record']][1].append(r['job'])
    return out


def has_baseline(path):
    """Whether these events have a baseline row, read up to the first one. A row that is not JSON (one cut
    short by a kill) is not one."""
    try:
        with open(path) as f:
            for line in f:
                try:
                    if json.loads(line).get('kind') == 'baseline':
                        return True
                except (ValueError, AttributeError):
                    continue
    except OSError:
        pass
    return False


def plain_trap(reply):
    """Whether a stop reply is a plain trap: T05 with no stop reason (a watchpoint or a breakpoint)."""
    if not reply.startswith('T05'):
        return False
    reasons = {'watch', 'rwatch', 'awatch', 'swbreak', 'hwbreak'}
    return not any(field.partition(':')[0] in reasons for field in reply[3:].split(';'))


def drop_redone_events(path, updates, kinds):
    """Before a resume: drop the events of these kinds (of hour_turned and objects_changed, the ones the
    resumed run can write again) that a lot update's return produced for an update after the last recorded
    row. The resumed run redoes that return and writes them again. Returns the dropped rows."""
    if not path.exists():
        return []
    keep, dropped = [], []
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        (dropped if r['kind'] in kinds and r.get('update', 0) > updates else keep).append(line)
    if dropped:
        tmp = path.with_suffix('.jsonl.tmp')
        tmp.write_text(''.join(line + '\n' for line in keep))
        tmp.rename(path)
    return [json.loads(line) for line in dropped]


def dump(container, guest_dir, d, regs, raw):
    """Write every present user page of the stopped process: d/ranges.json ([start, size, protection, file]),
    d/registers.json (the stop's registers and the stub's whole register packet, which holds the segment
    bases and the FPU state), and one file per range. QEMU writes into the guest directory; the files are
    then moved to d. Returns the number of bytes."""
    q = Qmp(container)
    try:
        ranges = q.user_ranges()
        tmp = guest_dir / 'dump-tmp'
        tmp.mkdir(exist_ok=True)
        d.mkdir()
        rows, total = [], 0
        for a, size, prot in ranges:
            name = f'{a:08x}.bin'
            q.call('memsave', val=a, size=size, filename=f'/g/dump-tmp/{name}')
            os.replace(tmp / name, d / name)
            rows.append([a, size, prot, name])
            total += size
        tmp.rmdir()
        (d / 'ranges.json').write_text(json.dumps(rows))
        (d / 'registers.json').write_text(json.dumps(dict(registers=regs, packet=raw.hex())))
        return total
    finally:
        q.close()


def game_identity(container):
    """The digests guest/launch.sh verified before it started the game
    ({file: sha256}), read from the container's share, or None if the file is
    not there (the game was started some other way)."""
    try:
        r = subprocess.run(['docker', 'exec', container, 'cat', '/g/share/game-identity.txt'],
                           capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if r.returncode:
        return None
    files = {}
    for line in r.stdout.splitlines():
        digest, _, name = line.strip().partition('  ')
        if len(digest) == 64 and name:
            files[name] = digest
    return files or None


def write_gz(path, value):
    with gzip.open(path, 'wt', compresslevel=6) as f:
        json.dump(value, f, separators=(',', ':'))



def stop_thread(reply):
    """The thread a stop reply names (its thread: field), or None."""
    for field in reply[3:].split(';'):
        if field.startswith('thread:'):
            return field[len('thread:'):]
    return None

def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--container', default='sims-harness-vm')
    ap.add_argument('--output', required=True, type=Path)
    ap.add_argument('--lot', type=int, default=5)
    ap.add_argument('--updates', type=int, default=100, help='stop after this many lot updates on the lot')
    ap.add_argument('--full', default='1-100', help='lot updates whose full state is saved, e.g. 1-100,110-1000/10')
    ap.add_argument('--stop-file', type=Path, help='end the capture when this file appears')
    ap.add_argument('--dump', default='', help='lot updates at whose entry the whole user memory of the game is '
                    'dumped (dump-NNNN/), e.g. 14,47; needs --guest-dir')
    ap.add_argument('--guest-dir', type=Path, help='the guest directory on the host (the container\'s /g), '
                    'where QEMU writes the dump before it is moved to the output')
    ap.add_argument('--watch', action='append', default=[], type=Watch,
                    help='NAME=EXPR[:SIZE] logs observations at watch stops to watch.jsonl from the baseline onward')
    ap.add_argument('--late-watch', action='append', default=[], type=Watch.late,
                    help='FROM[-TO],NAME=EXPR[:SIZE] logs observations at watch stops in the given update window')
    ap.add_argument('--scenario', type=Path, help='run this input scenario (see input.py); needs --guest-dir. '
                    'Logged to input.jsonl')
    ap.add_argument('--window', default=os.environ.get('SIMS_WINDOW', scenario_input.WINDOW),
                    help='the game window in the guest, for --scenario')
    ap.add_argument('--around', type=int, default=0,
                    help='also save the update in which an object appears or leaves, a dialog is shown or the hour '
                         'turns, and this many updates after it')
    ap.add_argument('--before-hour', type=int, default=0,
                    help='also save the last this many updates before each hour turns')
    ap.add_argument('--answer-stuck', type=float, default=0,
                    help='press Return when a dialog has had no result for this many seconds (needs --guest-dir)')
    ap.add_argument('--answer-from', type=int, default=3,
                    help='--answer-stuck answers only dialogs shown in this lot update or later')
    ap.add_argument('--resume', action='store_true',
                    help='take over a guest that an earlier run left stopped and continue its output directory '
                         'and numbering')
    args = ap.parse_args()
    full_set = parse_set(args.full)
    dump_set = parse_set(args.dump)
    late_texts = [w.text for w in args.late_watch]
    if len(set(late_texts)) != len(late_texts):
        # A taken-over run tells them apart by their text (watch_armed's expr).
        ap.error('a --late-watch is given twice')
    if dump_set and not args.guest_dir:
        ap.error('--dump needs --guest-dir')
    if args.scenario and not args.guest_dir:
        ap.error('--scenario needs --guest-dir')
    if args.answer_stuck and not args.guest_dir:
        ap.error('--answer-stuck needs --guest-dir')
    if args.around < 0 or args.before_hour < 0 or args.answer_stuck < 0:
        ap.error('--around, --before-hour and --answer-stuck cannot be negative')
    if args.scenario and args.resume:
        # A new runner would start the scenario again from its first step.
        ap.error('--scenario cannot be resumed')
    try:
        steps = scenario_input.parse_scenario(args.scenario.read_text()) if args.scenario else None
        if steps is not None or args.answer_stuck:
            scenario_input.check_window(args.window)
    except scenario_input.ScenarioError as e:
        ap.error(str(e))
    out = args.output
    if args.resume and not has_baseline(out / 'events.jsonl'):
        # Before the baseline nothing is recorded that a takeover could go on from (entry-0000 may be
        # half written, and the game's process is not known). Checked before anything is made or
        # changed, or the guest is attached.
        ap.error(f'{out} has no baseline event: --resume takes over only a run that reached its '
                 'baseline; start a new capture')
    out.mkdir(parents=True, exist_ok=args.resume)
    prior_status, dropped = {}, []
    if args.resume:
        try:
            prior_status = json.loads((out / 'status.json').read_text())
        except (OSError, ValueError):
            pass
        lines = (out / 'updates.jsonl').read_text().splitlines() if (out / 'updates.jsonl').exists() else []
        # Only what the redone return can write again: an hour turn needs the last row's clock, an object
        # change the registry saved with the last row.
        kinds = []
        if lines and 'clock' in json.loads(lines[-1]):
            kinds.append('hour_turned')
        if args.around and prior_status.get('registry_update') == len(lines) and prior_status.get('registry') is not None:
            kinds.append('objects_changed')
        dropped = drop_redone_events(out / 'events.jsonl', len(lines), kinds)
    events = open(out / 'events.jsonl', 'a')
    watch_log = open(out / 'watch.jsonl', 'a') if args.watch or args.late_watch else None
    updates_log = open(out / 'updates.jsonl', 'a')
    status = dict(started=time.time(), updates=0, state='attaching', settings={k: str(v) for k, v in vars(args).items()},
                  stepped_watch=prior_status.get('stepped_watch'), stepped_maybe=None)

    event_lock = threading.Lock()

    def event(**row):
        row['time'] = time.time()
        with event_lock:
            events.write(json.dumps(row, default=str) + '\n')
            events.flush()

    def save_status(**kw):
        status.update(kw, now=time.time())
        tmp = out / 'status.json.tmp'
        tmp.write_text(json.dumps(status, indent=1, default=str))
        tmp.rename(out / 'status.json')

    identity = game_identity(args.container)
    status['game_identity'] = identity
    event(kind='game_identity', files=identity, verified=identity is not None)
    runner = None
    if steps is not None:
        agent = scenario_input.Agent(args.guest_dir / 'share', args.window)
        runner = scenario_input.Runner(steps, agent, out / 'input.jsonl', shots_dir=out / 'input-shots')
        runner.write(kind='scenario', status='loaded', file=str(args.scenario), steps=len(steps),
                     agent_started=agent.start())
    answerer = None
    if args.answer_stuck:
        answerer = DialogAnswerer(args.guest_dir / 'share', args.window, args.answer_stuck, event,
                                  out / 'input-log.txt')
    remote = Remote(args.container)
    breaks = set()

    def arm(a):
        if a not in breaks:
            remote.set_break(a)
            breaks.add(a)

    def disarm(a):
        if a in breaks:
            remote.clear_break(a)
            breaks.discard(a)

    sims_cr3 = None
    sim = None
    sim_lot = None  # the lot whose update entry or return gave sim
    in_update = False
    watches = args.watch + args.late_watch
    pending = list(args.watch)
    late = list(args.late_watch)  # watches that wait for their first update
    live_watches = []  # watches set, each at its .address
    hardware = {}  # (address, size) -> how many armed watches use that watchpoint

    def hardware_add(a, size):
        if not hardware.get((a, size)):
            remote.set_watch(a, size)
        hardware[(a, size)] = hardware.get((a, size), 0) + 1

    def hardware_remove(a, size):
        hardware[(a, size)] -= 1
        if not hardware[(a, size)]:
            del hardware[(a, size)]
            remote.clear_watch(a, size)

    def locate(w, m):
        """(address, value) for watch w now, or None if it cannot be worked out or read. A late
        watch goes through the simulator only if the last entry or return that gave it was on this
        lot's (a stop after a return elsewhere still has the other lot's)."""
        try:
            a = w.resolve(m, sim if sim_lot == args.lot or w not in args.late_watch else None)
            return None if a is None else (a, w.read_at(m, a))
        except MemoryError:
            return None

    def arm_watches(m, here, lot):
        for w in list(pending):
            if not here and w in args.late_watch:
                continue
            found = locate(w, m)
            if found is None:
                continue
            w.address, w.value = found
            hardware_add(w.address, w.size)
            w.set = True
            live_watches.append(w)
            pending.remove(w)
            event(kind='watch_armed', name=w.name, expr=w.text, late=w in args.late_watch, lot=lot,
                  address=w.address, value=w.value, update=updates)

    def retire_elsewhere(lot):
        """At a stop on another lot (`lot`, None if it cannot be read): take each moving late watch
        off, to wait to be set again here. The simulator last seen is not used for them again until
        this lot's next entry or return, so a stop here before then (at a dialog breakpoint) cannot
        set one through another lot's simulator."""
        nonlocal sim_lot
        sim_lot = lot
        for w in list(live_watches):
            if w.moves and w in args.late_watch:
                hardware_remove(w.address, w.size)
                live_watches.remove(w)
                event(kind='watch_lost', name=w.name, expr=w.text, address=w.address, update=updates,
                      elsewhere=True)
                w.address = w.value = None
                pending.append(w)

    def check_watches(m, here, lot):
        """At a stop in the game, work out the moving watches again: move a
        watchpoint whose address changed, and put one that cannot be worked
        out (or read) back to wait. A late watch only at a stop on this lot:
        at a stop on another, a moving late watch is taken off and waits to be
        set again here (its address may be another lot's object's by then)."""
        if not here:
            retire_elsewhere(lot)
        for w in list(live_watches):
            if not w.moves:
                continue
            found = locate(w, m)
            if found is not None and found[0] == w.address:
                continue
            old = w.address
            hardware_remove(old, w.size)
            if found is None:
                live_watches.remove(w)
                w.address = w.value = None
                pending.append(w)
                event(kind='watch_lost', name=w.name, expr=w.text, address=old, update=updates)
                continue
            w.address, w.value = found
            hardware_add(w.address, w.size)
            event(kind='watch_moved', name=w.name, expr=w.text, old_address=old, address=w.address, value=w.value,
                  update=updates)

    def watch_hit(reply, regs, m):
        hit = int(reply.split('watch:')[1].split(';')[0], 16)
        if sims_cr3 is None:
            event(kind='unknown_watch', address=hit, pc=regs['eip'], cr3=regs['cr3'], update=updates)
            return
        if regs['cr3'] != sims_cr3:
            # The same virtual address in another process.
            event(kind='foreign_watch', address=hit, pc=regs['eip'], cr3=regs['cr3'], update=updates)
            return
        try:
            lot = m.lot()
        except MemoryError:
            lot = None
        # The watchpoints as set when the write was made (before any is taken off below).
        on_base = [p for p in hardware if p[0] == hit]
        candidates = list(live_watches)
        if lot != args.lot:
            # A write at a stop on another lot (before its next entry or dialog breakpoint) is not
            # this lot's object's: it is not matched to a moving late watch (before the exact and
            # range matches, so one cannot hide an ordinary watch on the same bytes), and those
            # watches are taken off, as at any stop there (the address may be another lot's object's
            # by then). The simulator last seen is not used for them again until this lot's next
            # entry or return. Other watches are worked out again at the next breakpoint, as always.
            candidates = [w for w in candidates if not (w.moves and w in args.late_watch)]
            retire_elsewhere(lot)
        def changed(w):
            try:
                return w.read(m) != w.value
            except MemoryError:
                return False
        # The stop reply gives the base of the watchpoint that fired, not the bytes written, and
        # which of several watchpoints fires, and whether it was written at all, depends on how
        # QEMU split and checked the store (7.2: the newest watchpoint a check overlaps fires; an
        # unaligned store on a watched page is checked from the page's base; one instruction may
        # be several stores). So each row says only what the stop and the memory show (match):
        # - named: the watch is on the watchpoint whose base the stop gives (one_of: one of the
        #   several watchpoints on that base);
        # - changed: the watch's value is not the one last seen. This store may have written it,
        #   or an earlier one that stopped nothing (through another mapping of the same memory);
        # - at: the stop gives no watchpoint's base (not one QEMU gives), and the watch holds it.
        # A watch that is not on the base the stop gives and kept its value has no row, even if
        # the store wrote it the value it held: the stop does not show that.
        hits = []
        for w in candidates:
            key = (w.address, w.size)
            if key in on_base:
                hits.append((w, 'named' if len(on_base) == 1 else 'one_of'))
            elif not on_base and w.address <= hit < w.address + w.size:
                hits.append((w, 'at'))
            elif changed(w):
                hits.append((w, 'changed'))
        for w, match in hits or [(None, None)]:
            # Taken over off a breakpoint, whether the guest is in a lot update is not known until its
            # next entry or return (in_update None): the write is in update `updates` or `updates` + 1,
            # so update is null and after_update says which was recorded last.
            row = dict(update=None if in_update is None else updates + (1 if in_update else 0),
                       after_update=updates, in_update=in_update, address=hit, lot=lot,
                       name=w.name if w else None, match=match, registers=regs, eip=regs['eip'])
            # The tick through this lot's simulator only (after a return elsewhere, sim is another
            # lot's until the next entry here), read apart so the values are kept if it fails.
            try:
                row['tick'] = m.word(sim + 0x98) if sim and sim_lot == args.lot else None
            except MemoryError as e:
                row['tick'], row['tick_error'] = None, str(e)
            try:
                if w:
                    row['before'], w.value = w.value, w.read(m)
                    row['after'] = w.value
                row['frames'] = frame_chain(m, regs['ebp'])
                row['stack'] = m.read(regs['esp'], 0x400).hex()
            except MemoryError as e:
                row['error'] = str(e)
            row['time'] = time.time()
            watch_log.write(json.dumps(row) + '\n')
            watch_log.flush()

    def save_dump(n, regs, raw):
        d = out / f'dump-{n:04d}'
        if d.exists():
            # Left by the run this one took over from: done if its last file was written, else redone.
            if (d / 'registers.json').exists():
                event(kind='dump_done_already', update=n)
                return
            for f in d.iterdir():
                f.unlink()
            d.rmdir()
            event(kind='redo_incomplete_dump', update=n)
        t0 = time.time()
        size = dump(args.container, args.guest_dir, d, regs, raw)
        event(kind='dump', update=n, bytes=size, seconds=time.time() - t0)

    baseline = False
    updates = 0
    elsewhere = 0
    around_until = 0  # with --around: save every update up to this one
    last_clock = None
    last_objects = None  # {pointer: id} of the registry at the last return (None: not known)
    dialog_pending = False  # a dialog was shown since the last return
    prior_show = None  # with --resume: the last dialog_show event recorded
    waiting = {}  # with --resume: the dialogs recorded as shown and given no result yet
    if args.resume:
        rows = [json.loads(line) for line in open(out / 'events.jsonl')]
        first_baseline = next(i for i, r in enumerate(rows) if r['kind'] == 'baseline')
        sims_cr3 = rows[first_baseline]['cr3']
        baseline = True
        lines = open(out / 'updates.jsonl').read().splitlines()
        updates = len(lines)
        if lines:
            last = json.loads(lines[-1])
            around_until = last.get('around_until', 0)
            last_clock = last.get('clock')
        if args.around:
            # The registry at the last recorded return, saved with it in status.json.
            if prior_status.get('registry_update') == updates and prior_status.get('registry') is not None:
                last_objects = {p: i for p, i in prior_status['registry']}
                # Kept in the new status.json too, in case this run is taken over before its first return.
                status.update(registry=prior_status['registry'], registry_update=updates)
            else:
                event(kind='registry_unknown', updates=updates, registry_update=prior_status.get('registry_update'))
        if dropped:
            event(kind='resume_dropped', events=[dict(kind=r['kind'], update=r['update']) for r in dropped])
        # A late watch an earlier run set counts as set (watch_ended's set covers its whole window).
        # Its event has its name, its whole text (with the updates), late true and the lot it was
        # armed on; only an arming on this lot after the baseline within its updates counts (it is
        # set at the end of update first - 1 at the earliest). Earlier revisions recorded neither
        # `late` nor the lot, and could arm on another lot, so their armings do not count: such a
        # watch ends with set false, and its watch_armed events have to be read by hand.
        since = rows[first_baseline:]
        for w in late:
            w.set = any(r['kind'] == 'watch_armed' and r.get('late') is True and r.get('lot') == args.lot
                        and r.get('name') == w.name and r.get('expr') == w.text
                        and r.get('update', -1) + 1 >= w.first
                        and (w.last is None or r['update'] + 1 <= w.last) for r in since)
        # A dialog shown since the last recorded return, and the dialogs still waiting for a result.
        dialog_pending = any(r['kind'] == 'dialog_show' and r['update'] > updates for r in rows)
        waiting = open_dialogs(rows)
        # The last dialog shown: a takeover at its stop (the earlier run was killed before stepping
        # past it) must not show it again, which would log it twice and reset its press jobs.
        # Only if it is the last dialog event (a later read or result shows the guest went on past it).
        prior_show = next((r for r in reversed(rows) if r['kind'] in ('dialog_show', 'dialog_result', 'dialog_read')),
                          None)
        if prior_show is not None and prior_show['kind'] != 'dialog_show':
            prior_show = None
        if prior_show is not None and (out / 'watch.jsonl').exists():
            # So does a watch row after it.
            with open(out / 'watch.jsonl') as f:
                for line in f:
                    try:
                        if json.loads(line).get('time', 0) > prior_show['time']:
                            prior_show = None
                            break
                    except (ValueError, AttributeError):
                        continue
        if waiting:
            event(kind='resume_open_dialogs', dialogs=[dict(record=k, update=u, jobs=j) for k, (u, j) in waiting.items()])
        if answerer:
            for record, (shown_in, jobs) in waiting.items():
                if shown_in >= args.answer_from:
                    answerer.shown(shown_in, record, jobs)
    def same_bytes(m, show):
        """Whether the dialog record's bytes and the top of the stack are as this logged show has them."""
        try:
            return (m.read(show['record'], 0x80).hex() == show.get('record_bytes')
                    and m.read(show['registers']['esp'], 0x20).hex() == show.get('stack'))
        except MemoryError:
            return False

    def tend_watches(m, foreign):
        """At a stop: start the late watches whose updates have come, work out the moving ones again,
        end those whose updates are over, and set those waiting. At a stop whose lot cannot be read
        (at a breakpoint or off one, as when a run is taken over), those waiting are set as far as
        they can be without the lot. Arming and sampling start at the baseline."""
        if sims_cr3 is None:
            return
        # A late watch waits for its first update. A takeover after its window never sets it.
        for w in [w for w in late if updates + 1 >= w.first]:
            late.remove(w)
            if w.last is None or updates + 1 <= w.last:
                pending.append(w)
                arm(ENTRY)
        # Those whose updates are over end first, so that one is not moved at the stop where it ends.
        ended = [w for w in live_watches + pending if w.last is not None and updates + 1 > w.last]
        for w in ended:
            if w in live_watches:
                live_watches.remove(w)
                hardware_remove(w.address, w.size)
            else:
                pending.remove(w)
            # set false: it never could be set (its address never worked out, or every stop in its
            # updates was in another process), so writes in its updates were not seen.
            event(kind='watch_ended', name=w.name, expr=w.text, update=updates, set=w.set)
        if ended and not (live_watches or pending or args.watch or runner or updates + 1 in dump_set):
            disarm(ENTRY)
        if foreign:
            return
        # A late watch is set or moved only at a stop on this lot: another lot's objects are not
        # this lot's, and one set there would count as set when the run is taken over. At a stop in
        # the game's process whose lot cannot be read, or on another lot, the moving late watches are
        # taken off and the simulator seen before is not used for them again until this lot's next
        # entry or return, whether or not any is set now (one waiting could be set at a later stop
        # here before then).
        try:
            lot = m.lot()
            readable = True
        except MemoryError:
            lot, readable = None, False
        here = lot == args.lot
        if not here:
            retire_elsewhere(lot)
        if live_watches and readable:
            check_watches(m, here, lot)
        if pending:
            arm_watches(m, here, lot)

    try:
        reply = remote.stop_reply()
        event(kind='attach', reply=reply, resume=args.resume, updates=updates)
        for a in (ENTRY, RETURN, DIALOG_SHOW, DIALOG_RESULT, DIALOG_READ, 0x415eff, scenario_input.QUEUE_PUSH):
            remote.clear_break(a)
            if args.resume:
                remote.clear_break(a)
        for a in ((RETURN,) if baseline else (ENTRY,)) + (DIALOG_SHOW, DIALOG_RESULT, DIALOG_READ):
            arm(a)
        if baseline and updates + 1 in dump_set:
            arm(ENTRY)  # taken over before a dump at the next entry (or in the middle of it)
        if runner:
            arm(scenario_input.QUEUE_PUSH)
        if args.resume:
            # The late watches whose updates have come: set at the first stop, even off a breakpoint.
            for w in [w for w in late if updates + 1 >= w.first]:
                late.remove(w)
                if w.last is None or updates + 1 <= w.last:
                    pending.append(w)
        if args.watch or pending or runner:
            arm(ENTRY)
            if args.resume:
                # Whether the guest is inside a lot update is known again at its next entry or return.
                in_update = None
        # Where a step past a breakpoint that stopped at a watchpoint left the guest (pc, cr3, thread):
        # QEMU then stops once more there, a plain trap for the step itself, when it is resumed.
        stepped_to = prior_status.get('stepped_to')
        save_status(state='running', breaks=sorted(breaks),
                    stepped_to=tuple(stepped_to) if stepped_to else None)
        if status['stepped_watch']:
            # The run taken over was killed after a step stopped at a watchpoint and before it wrote all
            # that stop's rows: they may be missing from watch.jsonl. Kept in status.json until this is
            # logged (a run killed before then hands it on; one killed just after may log it twice).
            event(kind='unsettled_watch', reply=status['stepped_watch'], stepped_to=stepped_to)
            save_status(stepped_watch=None)
        first = args.resume
        while updates < args.updates:
            if args.stop_file and args.stop_file.exists():
                event(kind='requested_stop', updates=updates)
                break
            took_over = first
            if first:
                # The guest is still stopped where the earlier run left it.
                first = False
            else:
                remote.resume()
                reply = remote.wait()
            regs, raw = remote.registers()
            pc = regs['eip']
            fs_base, cr3 = struct.unpack_from('<I', raw, 64)[0], struct.unpack_from('<I', raw, 84)[0]
            regs = {k: v for k, v in regs.items() if not k.startswith('_')}
            regs.update(fs_base=fs_base, cr3=cr3)
            m = Memory(remote)
            stepped_to = status['stepped_to']
            if stepped_to is not None:
                thread = stop_thread(reply)
                # The attach query leaves the trap pending and may omit the thread.
                same_stop = (pc, cr3) == stepped_to[:2] and (
                    thread == stepped_to[2] or (took_over and thread is None))
                plain = plain_trap(reply)
                if not took_over or not (plain and same_stop):
                    save_status(stepped_to=None)
                if not took_over and plain and same_stop and pc not in breaks:
                    event(kind='step_after_watch', pc=pc)
                    continue
            maybe = status.get('stepped_maybe')
            if maybe is not None and not took_over:
                save_status(stepped_maybe=None)
                if plain_trap(reply) and (pc, cr3) == tuple(maybe[:2]) \
                        and maybe[2] in (None, stop_thread(reply)) and pc not in breaks:
                    # Only a step's pending trap stops plainly off a breakpoint: the earlier run's
                    # step stopped at a watchpoint, and was killed before it recorded that.
                    event(kind='step_after_watch', pc=pc, unrecorded_watch=True)
                    continue
            if took_over and pc not in breaks:
                # The earlier run was killed between stepping past a breakpoint and resuming.
                event(kind='took_over_off_break', pc=pc, registers=regs)
                if status['stepped_to'] is None and (args.watch or args.late_watch):
                    # Killed before it kept where that step left the guest, its pending trap is
                    # matched on the pc, the CR3 and the thread the reattach gives (if it gives one).
                    # Only a step that stopped at a watchpoint leaves one, so only with watches.
                    save_status(stepped_maybe=(pc, cr3, stop_thread(reply)))
                tend_watches(m, sims_cr3 is not None and cr3 != sims_cr3)
                continue
            if took_over and pc == RETURN:
                # Only a return on this lot in the game's process can be one already recorded; any
                # other is dealt with below as usual (its simulator may not be readable).
                try:
                    ours = cr3 == sims_cr3 and m.lot() == args.lot
                except MemoryError:
                    ours = False
                if ours:
                    recorded = out / 'updates.jsonl'
                    lines = recorded.read_text().splitlines() if recorded.exists() else []
                    last = json.loads(lines[-1]) if lines else None
                    now = m.summary(regs['ebx'], fs_base)
                    event(kind='took_over_at_return', last=last, now=now)
                    # With nothing recorded yet, this is update 1's return.
                    if last is not None and all(last[k] == now[k] for k in now):
                        raise RuntimeError('took over at a return already recorded; refusing to guess')
                else:
                    event(kind='took_over_at_return', elsewhere=True, cr3=cr3)
            if not took_over and 'watch:' in reply:
                watch_hit(reply, regs, m)
                continue
            if pc not in breaks:
                event(kind='unexpected_stop', reply=reply, registers=regs)
                raise RuntimeError(f'unexpected stop at {pc:#x}: {reply}')
            foreign = sims_cr3 is not None and cr3 != sims_cr3
            lot = None
            if not foreign:
                try:
                    lot = m.lot()
                except MemoryError:
                    foreign = True
            if foreign:
                event(kind='foreign_stop', pc=pc, cr3=cr3)
            elif pc == ENTRY:
                sim, sim_lot = regs['ecx'], lot
                if lot == args.lot and not baseline:
                    sims_cr3 = cr3
                    t0 = time.time()
                    state, full, pages = m.snapshot('lot_update_entry', regs, regs['ecx'], fs_base)
                    d = out / 'entry-0000'
                    d.mkdir()
                    write_gz(d / 'state.json.gz', state)
                    write_gz(d / 'full-state.json.gz', full)
                    write_gz(d / 'pages.json.gz', pages)
                    baseline = True
                    # The registry and the clock at update 1's entry, so a change in update 1 is seen.
                    if args.around or args.before_hour:
                        last_clock = m.clock(sim)[0]
                    if args.around:
                        last_objects = {p: m.object_id(p) for p in m.registry(sim)}
                    event(kind='baseline', tick=state['tick'], lot=lot, cr3=cr3, teb=fs_base, seconds=time.time() - t0,
                          packets=remote.packets, sim=regs['ecx'])
                    if 1 in dump_set:  # the baseline is the entry of lot update 1
                        save_dump(1, regs, raw)
                    if not (live_watches or pending or args.watch or runner):
                        disarm(ENTRY)
                    arm(RETURN)
                    in_update = True
                elif lot == args.lot and baseline:
                    in_update = True
                    if updates + 1 in dump_set:
                        save_dump(updates + 1, regs, raw)
                    if not (live_watches or pending or args.watch or runner):
                        disarm(ENTRY)
                else:
                    elsewhere += 1
                    if elsewhere in (1, 10, 100) or elsewhere % 1000 == 0:
                        event(kind='entry_elsewhere', lot=lot, cr3=cr3, count=elsewhere)
            elif pc == RETURN:
                sim, sim_lot = regs['ebx'], lot
                in_update = False
                if lot != args.lot:
                    event(kind='return_elsewhere', lot=lot)
                else:
                    updates += 1
                    advanced = m.byte(regs['ebp'] - 1)
                    row = dict(update=updates, advanced=advanced, **m.summary(sim, fs_base))
                    reasons = ['full'] if updates in full_set else []
                    if args.around or args.before_hour:
                        row['clock'], row['date'] = m.clock(sim)
                        pointers = m.registry(sim)
                        row['objects'] = len(pointers)
                        triggers = []
                        now = None
                        if args.around:
                            # Every ID is read again: an object made at a freed object's address is
                            # another object. The IDs lie on about one page for every four objects.
                            now = {p: m.object_id(p) for p in pointers}
                        if last_objects is not None and now is not None and \
                                list(now.items()) != list(last_objects.items()):
                            before, after = set(last_objects.items()), set(now.items())
                            added = [dict(id=i, pointer=p) for p, i in now.items() if (p, i) not in before]
                            removed = [dict(id=i, pointer=p) for p, i in last_objects.items() if (p, i) not in after]
                            event(kind='objects_changed', update=updates, tick=row['tick'], added=added,
                                  removed=removed, count=len(pointers))
                            triggers.append('objects')
                        if last_clock is not None and row['clock'][0] != last_clock[0]:
                            event(kind='hour_turned', update=updates, tick=row['tick'], clock=row['clock'],
                                  date=row['date'], previous=last_clock)
                            triggers.append('hour')
                        if dialog_pending:
                            triggers.append('dialog')
                        dialog_pending = False
                        last_objects, last_clock = now, row['clock']
                        if args.around and triggers:
                            around_until = max(around_until, updates + args.around)
                            reasons += triggers
                        elif args.around and updates <= around_until:
                            reasons.append('around')
                        if args.before_hour and updates_to_hour(row['clock']) <= args.before_hour:
                            reasons.append('before_hour')
                        if around_until > updates:
                            row['around_until'] = around_until
                    if reasons:
                        t0 = time.time()
                        state, full, pages = m.snapshot('lot_update_return', regs, sim, fs_base)
                        state['advanced'] = advanced
                        d = out / f'update-{updates:04d}'
                        if args.resume and d.exists():
                            # Left incomplete by the run this one took over from.
                            for f in d.iterdir():
                                f.unlink()
                            d.rmdir()
                            event(kind='redo_incomplete', update=updates)
                        d.mkdir()
                        write_gz(d / 'state.json.gz', state)
                        write_gz(d / 'full-state.json.gz', full)
                        write_gz(d / 'pages.json.gz', pages)
                        row['saved'] = d.name
                        row['saved_for'] = reasons
                        row['seconds'] = round(time.time() - t0, 2)
                    updates_log.write(json.dumps(row) + '\n')
                    updates_log.flush()
                    if args.around and last_objects is not None:
                        save_status(updates=updates, last=row, registry=list(last_objects.items()),
                                    registry_update=updates)
                    else:
                        save_status(updates=updates, last=row)
                    if runner and baseline:
                        runner.on_update(updates, m, sim)
                    if updates + 1 in dump_set:
                        arm(ENTRY)
            elif runner and pc == scenario_input.QUEUE_PUSH:
                runner.on_queue_push(updates + 1, bool(in_update), m, regs)
            elif (took_over and pc == DIALOG_SHOW and prior_show is not None
                  and prior_show.get('registers') == regs and prior_show['update'] == updates + 1
                  and regs['ecx'] in waiting and same_bytes(m, prior_show)):
                # The earlier run logged this show and was killed before stepping past it: the guest
                # is still at that stop (every register, the record's bytes and the top of the stack
                # as logged, and no dialog event or watch row since).
                # It was shown to the answerer again above with its press jobs, so it is not shown
                # again here. (A second show of the same record with no result between it and the
                # first, and every register the same, would be taken for it too.)
                event(kind='took_over_at_dialog_show', record=regs['ecx'], update=updates + 1)
                save_status(dialog=dict(update=updates + 1, record=regs['ecx'], shown=prior_show['time']))
            else:
                # A dialog function. ecx is the dialog record (0x436da0 keeps it in esi).
                record = regs['ecx']
                row = dict(kind={DIALOG_SHOW: 'dialog_show', DIALOG_RESULT: 'dialog_result',
                                 DIALOG_READ: 'dialog_read'}[pc], pc=pc, record=record, registers=regs,
                           update=updates + 1)
                try:
                    row['record_bytes'] = m.read(record, 0x80).hex()
                    row['status'] = m.word(record + 0x74)
                    row['modal'] = m.byte(record + 0x69)
                    row['stack'] = m.read(regs['esp'], 0x20).hex()
                    if pc == DIALOG_RESULT:
                        # The stop is at the function's entry: the result is its argument, and the
                        # record's status still holds the old value.
                        row['result'] = m.word(regs['esp'] + 4)
                except MemoryError as e:
                    row['error'] = str(e)
                event(**row)
                if runner:
                    runner.on_dialog(row['kind'], updates + 1)
                if pc == DIALOG_SHOW:
                    dialog_pending = True
                    if answerer and updates + 1 >= args.answer_from:
                        answerer.shown(updates + 1, record)
                elif pc == DIALOG_RESULT and answerer:
                    answerer.answered(record)
                if pc == DIALOG_SHOW:
                    save_status(dialog=dict(update=updates + 1, record=record, shown=time.time()))
            # Only another process counts as foreign here: tend_watches reads the lot again, and a lot
            # that cannot be read in the game's process still takes the moving late watches off.
            tend_watches(m, sims_cr3 is not None and cr3 != sims_cr3)
            # Step past the breakpoint we stopped at (if it is still armed).
            armed = pc in breaks
            if armed:
                remote.clear_break(pc)
            # Every register before the step: a step the pending trap took leaves them all as they
            # were (EIP alone stays put under a step of one round of a rep string instruction).
            step_from = remote.registers()
            for attempt in range(2):
                reply = remote.step()
                watch_stop = 'watch:' in reply
                if not reply.startswith('T05'):
                    # Not a trap (an error such as E22, or the guest gone): where the guest is is
                    # not known, so the capture cannot go on.
                    event(kind='step_failed', pc=pc, cr3=cr3, reply=reply, attempt=attempt)
                    raise RuntimeError(f'step at {pc:#x} gave {reply!r}, not a trap')
                if not watch_stop and status['stepped_to'] is not None:
                    save_status(stepped_to=None)
                regs, raw = remote.registers()
                step_cr3 = struct.unpack_from('<I', raw, 84)[0]
                if watch_stop:
                    # Kept before the rows are written, with the reply: a run killed before they
                    # all are is taken over with the stop known (took_over_off_break's
                    # unsettled_watch), and the pending trap accepted.
                    save_status(stepped_to=(regs['eip'], step_cr3, stop_thread(reply)), stepped_watch=reply)
                    break
                if (regs, raw) != step_from:
                    break
                # A pending debug trap can stop the step before the instruction executes.
                if attempt:
                    raise RuntimeError(f'step at {pc:#x} left every register unchanged after retry: {reply}')
                event(kind='step_swallowed', pc=pc, cr3=cr3)
            if armed:
                remote.set_break(pc)
            if watch_stop:
                regs = {k: v for k, v in regs.items() if not k.startswith('_')}
                regs.update(fs_base=struct.unpack_from('<I', raw, 64)[0], cr3=step_cr3)
                watch_hit(reply, regs, Memory(remote))
                save_status(stepped_watch=None)
        event(kind='complete', updates=updates)
        save_status(state='complete')
    except BaseException as e:
        event(kind='error', error=repr(e))
        save_status(state='error', error=repr(e))
        raise
    finally:
        if answerer:
            answerer.close()
        if runner:
            runner.close()
        try:
            if not remote.stopped:
                remote.interrupt()
            for a in list(breaks):
                remote.clear_break(a)
            for a, size in list(hardware):
                remote.clear_watch(a, size)
        finally:
            try:
                remote.detach()
            finally:
                for f in (events, watch_log, updates_log):
                    if f:
                        f.close()


if __name__ == '__main__':
    main()
