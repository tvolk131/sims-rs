"""Scripted input for a capture: point at objects, choose pie-menu entries, press keys.

capture.py runs a scenario with --scenario FILE --guest-dir DIR. The game is
only ever told things through the pointer and the keyboard: the commands are
xdotool calls that a small agent in the guest (guest/input-agent.sh) runs
against the game's window in Xvfb. QEMU's own input devices do not reach the
game, because the guest's X server is Xvfb, which reads no device. Every
check of what the game did is a read of guest memory at a lot-update stop.

A scenario is a text file, one step per line, words split as in a shell,
`#` starts a comment:

  until 5                      wait until lot update 5 has returned
  wait 10                      let 10 more lot updates return
  move X Y                     move the pointer (window coordinates)
  click X Y                    move there, rest, press and release the left button
  answer-dialog X Y            wait for a dialog to show, then click X Y
  key KEYSYM                   press a key (an xdotool key name, e.g. F2)
  click-object ID              aim at object ID until the game's picker tool
                               reports it under the pointer, click, and keep
                               the pie menu only if its actions aim at ID
                               (else close it with Escape and aim again)
  choose LABEL                 click the open pie menu's entry LABEL
  expect-queued PERSON [TARGET] [within N]
                               fail unless the pie menu pushed an action on
                               PERSON (aimed at object TARGET) within N updates
  shot NAME                    save the game window as <capture>/input-shots/NAME.png
  press X Y                    press a button of the game's panels (the buy-mode
                               button, a catalog entry): rest, move over it, press
  expect-mode live|buy|N [within N]
                               fail unless the lot's mode word becomes N
  expect-new-object [within N] fail unless an object appears in the registry
                               (logs it and the budget's change)

Each step starts at a lot-update return, after the previous step ended. A
step that fails is logged, and the scenario stops there; the capture goes
on. Everything is logged to <capture>/input.jsonl with the lot update it
happened at (see `Runner`).

Memory layout read here (addresses in Sims.exe 1.91 as it runs, found with
Ghidra on the decrypted image and checked in the running game):

- The game object is the word at 0x6e48bc. Its +0x18 is the tool manager,
  whose first word points at 12 tools; tool 6 is the object picker
  (cObjPickerTool, vtable 0x667f1c; 0x51bb42 fetches it).
- Picker +0x2e (16 bits): the ID of the object under the pointer (0x48c834
  sets it from the pointer moves).
- Picker +0x58: the open pie menu's entries, a pointer to a vector
  (begin, end) of 0x28-byte records: +0 the label (begin, end, capacity of
  its characters), +0x14 nonzero for an entry that opens a submenu, +0x24
  the action it queues. +0x64 is the index of the first shown entry.
- The people panel (cWinPeople) is the word at 0x6f3b2c. Its +0xdc and
  +0xe0 are the two pie menus (cTSPieMenu, vtable 0x674b54) it draws.
  Pie +0x13c, +0x140: the menu window's origin on the screen; +0xd4: the
  highlighted item (-1 for none); +0x190: the items, a vector of 0x28-byte
  records: +0 the label, +0x10 enabled, +0x14 angle (float), +0x18 x,
  +0x1c y, +0x20 width, +0x24 height, relative to the origin (hit test at
  0x5eb7c9).
- The house view: zoom at 0x6f00d8, the screen position of tile (0, 0) at
  0x6f00d0 and 0x6f00d4. 0x4b3c88 puts a tile (a, b, level) at
  x = (a - b) * (8 << zoom) + x0, y = (a + b) * (4 << zoom) + y0, minus a
  level height above level 1. An object's position is at +0xf4 and +0xf8
  in sixteenths of a tile; a = (+0xf8) / 16, b = (+0xf4) / 16.
  **Unverified:** rotation. The formula was checked only in the house's
  starting view; aiming always confirms through the picker.
- The lot (the lot update's ecx at entry, ebx at return): +0x36 the mode
  (0 live, 2 buy), +0x90 the budget, signed, +0xb0 the registry, whose
  +0x18..+0x1c are the object slots. In buy mode the lot update still runs
  (without advancing time), so the runner goes on there.
- 0x48ddcb is the call in the pie menu's select handler (0x48dd67) that
  pushes the chosen action onto the person's queue (0x45d20b): edi (= ecx)
  is the person, esi the action. capture.py stops there to record it.
"""
import json
import re
import shlex
import struct
import time
from pathlib import Path

GAME = 0x6e48bc
PEOPLE_PANEL = 0x6f3b2c
VIEW_ZOOM, VIEW_X0, VIEW_Y0 = 0x6f00d8, 0x6f00d0, 0x6f00d4
QUEUE_PUSH = 0x48ddcb
ACTIVE_ACTION = 0xa48  # a person's running action (0x44 bytes); the queue is at +0x820
PICKER_TOOL = 6
WINDOW = '0xe00013'

STEPS = {
    'until': (1, 1), 'wait': (1, 1), 'move': (2, 2), 'click': (2, 2), 'answer-dialog': (2, 2),
    'key': (1, 1), 'click-object': (1, 1), 'choose': (1, 1), 'expect-queued': (1, 4), 'shot': (1, 1),
    'press': (2, 2), 'expect-mode': (1, 3), 'expect-new-object': (0, 2),
}
# The lot's mode word (sim +0x36), as read in the running game: 0 in live
# mode, 2 after the buy-mode button. Build mode's number is not checked yet;
# expect-mode takes a number too.
MODES = {'live': 0, 'buy': 2}


class ScenarioError(ValueError):
    pass


class StepFailed(RuntimeError):
    pass


def parse_scenario(text):
    """The steps of a scenario: [dict(line=, text=, op=, args=[...])]."""
    steps = []
    shots = set()
    for n, raw in enumerate(text.splitlines(), 1):
        words = shlex.split(raw, comments=True)
        if not words:
            continue
        op, args = words[0], words[1:]
        if op not in STEPS:
            raise ScenarioError(f'line {n}: unknown step {op!r}')
        lo, hi = STEPS[op]
        if not lo <= len(args) <= hi:
            raise ScenarioError(f'line {n}: {op} takes {lo} to {hi} arguments')
        step = dict(line=n, text=raw.strip(), op=op)
        try:
            if op in ('until', 'wait', 'click-object'):
                step['args'] = [int(args[0])]
            elif op in ('move', 'click', 'answer-dialog', 'press'):
                step['args'] = [int(args[0]), int(args[1])]
            elif op == 'expect-queued':
                person, target, within = int(args[0]), None, 100
                rest = args[1:]
                if rest and rest[0] != 'within':
                    target, rest = int(rest[0]), rest[1:]
                if rest:
                    if len(rest) != 2 or rest[0] != 'within':
                        raise ScenarioError(f'line {n}: expect-queued PERSON [TARGET] [within N]')
                    within = int(rest[1])
                step['args'] = [person, target, within]
            elif op in ('expect-mode', 'expect-new-object'):
                rest, args_out = list(args), []
                if op == 'expect-mode':
                    name = rest.pop(0)
                    args_out.append(MODES[name] if name in MODES else int(name))
                within = 200
                if rest:
                    if len(rest) != 2 or rest[0] != 'within':
                        raise ScenarioError(f'line {n}: {op} ... [within N]')
                    within = int(rest[1])
                step['args'] = args_out + [within]
            else:
                step['args'] = [args[0]]
        except ValueError as e:
            if isinstance(e, ScenarioError):
                raise
            raise ScenarioError(f'line {n}: {e}') from None
        if op == 'shot':  # each shot has its own file in the guest and in the capture
            if args[0] in shots:
                raise ScenarioError(f'line {n}: a shot named {args[0]!r} is taken already')
            shots.add(args[0])
        steps.append(step)
    return steps


# Pointer and key commands (run by the guest agent).

def move_command(x, y, window=WINDOW):
    """Move the pointer to (x, y) in the window, then one pixel right: the game
    acts on a pointer that has moved over a place, not one that jumped there."""
    return f'xdotool mousemove --window {window} {x - 1} {y} sleep 0.2 mousemove --window {window} {x} {y}'


# A press on the house view or a pie menu is held while lot updates return.
# capture.py holds the game still for seconds at each stop; a short press
# (down and up 0.3 s apart) can fall inside one stop, and the game, which
# reads the button between frames, then never sees it down. Seen twice: a
# press on the porch bench and one on the bookcase, each with the picker on
# the object, opened no pie menu in 200 updates. The settling counts the
# update return at which the press is seen to have ended, so at least
# HOLD_UPDATES - 1 whole updates run with the button down.
HOLD_UPDATES = 3


def press_down_command():
    return 'xdotool mousedown 1'


def press_up_command():
    return 'xdotool mouseup 1'


def dialog_press_command(x, y, window=WINDOW):
    """A press on a dialog's button, as answer.sh makes it: the game takes it
    only after the pointer has rested and then moved over the button."""
    return (f'xdotool windowfocus --sync {window} sleep 3 mousemove --window {window} {x} {y} sleep 4 '
            f'mousemove --window {window} {x + 2} {y} sleep 2 mousedown 1 sleep 3 mouseup 1')


def key_command(keysym):
    if not keysym.replace('_', '').isalnum():
        raise ScenarioError(f'not a key name: {keysym!r}')
    return f'xdotool key {keysym}'


def shot_command(window, guest_path):
    return f'xwd -id {window} -silent -out {guest_path}'


# Projection and aiming.

def project(position, view):
    """The screen point of an object's position (the +0xf4, +0xf8 words), in the
    house view described by view = dict(zoom, x0, y0)."""
    b, a = position[0] / 16, position[1] / 16
    z = view['zoom']
    return round((a - b) * (8 << z) + view['x0']), round((a + b) * (4 << z) + view['y0'])


def aim_points(x, y, zoom):
    """Where to look for an object anchored at (x, y): from just above the anchor
    upward (sprites stand above their anchor), then a little to each side."""
    scale = 1 << zoom
    out = []
    for dx in (0, -4, 4):
        for dy in (-4, -10, -16, -22, -28, -34, -40, 2):
            out.append((x + dx * scale, y + dy * scale))
    return out


def clamp(point, width=800, height=600):
    return min(max(point[0], 1), width - 2), min(max(point[1], 1), height - 2)


# Reading the game (all reads).

def signed(v):
    return struct.unpack('<i', struct.pack('<I', v))[0]


def read_string(m, p):
    """A label: three words (begin, end, capacity) of single-byte characters."""
    begin, end = m.word(p), m.word(p + 4)
    if not begin or end <= begin or end - begin > 256:
        return ''
    return m.read(begin, end - begin).split(b'\0')[0].decode('latin-1')


def read_view(m):
    return dict(zoom=m.word(VIEW_ZOOM), x0=signed(m.word(VIEW_X0)), y0=signed(m.word(VIEW_Y0)))


def picker(m):
    return m.word(m.word(m.word(m.word(GAME) + 0x18)) + PICKER_TOOL * 4)


def read_hover(m):
    return m.half(picker(m) + 0x2e)


def read_entries(m):
    """The open pie menu's entries as the picker holds them."""
    p = picker(m)
    vec = m.word(p + 0x58)
    if not vec:
        return []
    begin, end = m.word(vec), m.word(vec + 4)
    if end < begin or (end - begin) % 0x28 or end - begin > 0x28 * 64:
        return []
    out = []
    for q in range(begin, end, 0x28):
        action = m.word(q + 0x24)
        out.append(dict(label=read_string(m, q), submenu=m.byte(q + 0x14), action=action))
    return out


def read_pie(m):
    """The pie menu on the screen, or None: dict(pointer, origin, highlighted, items)
    with items [dict(label, enabled, x, y, w, h)] in screen coordinates."""
    panel = m.word(PEOPLE_PANEL)
    if not panel:
        return None
    for off in (0xdc, 0xe0):
        pie = m.word(panel + off)
        if not pie:
            continue
        begin, end = m.word(pie + 0x190), m.word(pie + 0x194)
        if not begin or end <= begin or (end - begin) % 0x28 or end - begin > 0x28 * 64:
            continue
        ox, oy = signed(m.word(pie + 0x13c)), signed(m.word(pie + 0x140))
        items = []
        for q in range(begin, end, 0x28):
            x, y, w, h = (signed(v) for v in m.words(q + 0x18, 4))
            items.append(dict(label=read_string(m, q), enabled=m.byte(q + 0x10), x=ox + x, y=oy + y, w=w, h=h))
        return dict(pointer=pie, origin=[ox, oy], highlighted=signed(m.word(pie + 0xd4)), items=items)
    return None


def find_object(m, sim, oid):
    """The pointer of the object with ID oid in the lot's registry, or 0."""
    reg = m.word(sim + 0xb0)
    for q in range(m.word(reg + 0x18), m.word(reg + 0x1c), 4):
        p = m.word(q)
        if p and m.object_id(p) == oid:
            return p
    return 0


def object_slots(m, sim):
    """The object pointers in the lot's registry: one read of its slot
    vector, cheap enough for every update return."""
    reg = m.word(sim + 0xb0)
    first, end = m.word(reg + 0x18), m.word(reg + 0x1c)
    if not (first <= end and end - first <= 131072 and (end - first) % 4 == 0):
        raise MemoryError(f'registry slots {first:#x}..{end:#x}')
    return {p for p in m.words(first, (end - first) // 4) if p}


def placed_object(m, p):
    """What a replay needs to place object p: its ID, its type, its position
    and level, its definition (0xa6 bytes) and its 72 data words, read as
    capture.py's full state reads them, so a replay can decode them the same
    way. Which bytes hold the GUID and the facing is not checked here."""
    definition = m.word(p + 0x118)
    return dict(id=m.object_id(p), pointer=p, type=m.half(definition + 0x12), position=object_position(m, p),
                level=m.byte(p + 0xfe), definition=m.read(definition, 0xa6).hex(), data=m.words(p + 0x4a, 72, 2))


def read_mode(m, sim):
    return m.half(sim + 0x36)


def read_budget(m, sim):
    return signed(m.word(sim + 0x90))


def object_position(m, p):
    return [signed(v) for v in m.words(p + 0xf4, 2)]


def selected_person(m, sim):
    """The ID of the selected person (object flags +0x114 bit 1, as 0x4456e4 finds it), or None."""
    reg = m.word(sim + 0xb0)
    for q in range(m.word(reg + 0x18), m.word(reg + 0x1c), 4):
        p = m.word(q)
        if p and m.half(m.word(p + 0x118) + 0x12) == 2 and m.word(p + 0x114) & 2:
            return m.object_id(p)
    return None


def read_queue(m, person):
    first, end = m.word(person + 0xa40), m.word(person + 0xa44)
    if (end - first) & 0xffffffff > 8:
        return []
    return [m.action(person + 0x820 + (i & 7) * 0x44) for i in range(first, end)]


def same_action(a, b):
    """The same interaction: the fields the pie menu's entry sets. The uid is
    given only when the queue takes the action, and the flags and the
    attenuation change as it runs."""
    return all(a.get(k) == b.get(k) for k in ('caller', 'target', 'routine', 'menu', 'args'))


def check_window(window):
    """The window ID as xdotool takes it; anything else is refused, since it
    goes into guest shell commands."""
    if not re.fullmatch(r'0x[0-9a-fA-F]{1,8}|[0-9]{1,10}', str(window)):
        raise ScenarioError(f'not an X window ID: {window!r}')
    return str(window)


def match_item(items, label):
    """The index of the item whose label is label (case and surrounding space ignored), or None."""
    want = label.strip().lower()
    for i, item in enumerate(items):
        if item['label'].strip().lower() == want:
            return i
    return None


# The guest agent.

class Agent:
    """Hands commands to guest/input-agent.sh through <share>/input."""

    def __init__(self, share, window=WINDOW):
        self.share = Path(share)
        self.dir = self.share / 'input'
        self.window = check_window(window)
        self.n = 0

    def numbered(self):
        """The highest command number used in the input folder (as .cmd,
        claimed as .run, or ended as .done)."""
        return max([int(p.stem) for p in self.dir.iterdir() if p.stem.isdigit()] + [0])

    def start(self):
        """Start the agent in the guest, unless one already answers. Returns
        True if one was asked to start. A second agent is harmless: it exits
        while a live one holds the agent's lock, and each command is claimed
        by a rename, so only one agent runs it."""
        self.dir.mkdir(exist_ok=True)
        self.n = self.numbered()
        if (self.dir / 'agent.pid').exists() and self.alive():
            return False
        source = Path(__file__).parent / 'guest' / 'input-agent.sh'
        (self.share / 'input-agent.sh').write_text(source.read_text())
        from guest import submit  # reserves the job number (other processes hand the guest jobs too)
        submit(self.share, 'sh /research/input-agent.sh\n')
        return True

    def alive(self, seconds=5):
        """True if an agent runs a probe command within `seconds` (the pid
        file outlives a guest restart, so it alone proves nothing)."""
        n = self.submit('true')
        end = time.time() + seconds
        while time.time() < end:
            if self.result(n) is not None:
                return True
            time.sleep(0.1)
        return False

    def submit(self, command):
        self.n += 1
        tmp = self.dir / f'{self.n:05d}.tmp'
        tmp.write_text(command + '\n')
        tmp.rename(self.dir / f'{self.n:05d}.cmd')
        return self.n

    def result(self, n):
        """None while command n runs, else its exit code."""
        done = self.dir / f'{n:05d}.done'
        if not done.exists():
            return None
        text = done.read_text().strip()
        return int(text) if text else None


# The scenario runner.

class Job:
    """Yielded by a step: run this command in the guest, then wait `settle`
    lot updates after it ended."""

    def __init__(self, kind, command, settle=3, wait=True, **info):
        self.kind, self.command, self.settle, self.wait, self.info = kind, command, settle, wait, info


class Event:
    """Yielded by a step: wait until capture.py has reported more than `after`
    dialog stops of this kind (dialog_show, dialog_result, dialog_read). A
    modal dialog holds the lot update it shows in, so these waits are driven
    by the dialog stops, not by update returns."""

    def __init__(self, kind, after):
        self.kind, self.after = kind, after


class Fresh:
    """Yielded by a step before it reads guest memory: go on at once if the
    runner's memory was read at this update return, else at the next one.
    (At a dialog stop, the memory the runner holds is an older stop's.)"""


class Updates:
    """Yielded by a step: let n more lot updates return."""

    def __init__(self, n):
        self.n = n


class UntilPush:
    """Yielded by a step: go on at the next update return, or at the first
    stop of any kind (a dialog stop too) once more than `pushes` queue pushes
    have been seen. A choice can push and then open a modal dialog, which
    stops update returns."""

    def __init__(self, pushes):
        self.pushes = pushes


class Runner:
    """Runs a scenario from capture.py's stops.

    capture.py calls on_update(update, m, sim) at the return of every lot
    update on the lot (update is the number of updates returned so far),
    on_queue_push(update, in_update, m, regs) at QUEUE_PUSH (update is the
    number of the lot update that comes next), and on_dialog(kind, update)
    for the dialog stops. m reads guest memory.

    input.jsonl rows (each has `kind`, `update` and `time`):
      step     a step started, ended (`status` done) or failed (`error`).
      command  a command handed to the guest: `what` (move, press, key, shot),
               `x`, `y` or `key`, and the command.
      command_ended  the stop at which its end was seen, with its exit `code`.
      command_wait_ended  a dialog showed while the step waited on the
               command: the step went on (its end is still checked later).
      hover    the object under the pointer after a move during aiming.
      pie      the pie menu after it opened or before a choice: its items
               and the picker's entries (label, submenu, action).
      queued   the action the pie menu pushed on a person, read at the push.
               `update` N means before lot update N (`in_update` false) or
               inside it (true). This is the record to replay: give the
               action to the person before update N.
      mode     expect-mode saw the mode.
      placed   expect-new-object's new objects (ID, pointer, type, position,
               level, the definition's 0xa6 bytes and the 72 data words)
               with the budget before and after and the cost.
      taken    expect-queued found the pushed action on the person: `where`
               is queue or active (the queue had already started it), with
               the person's active action and queue at that update return.
      scenario `status` done (every step and command ended) or unfinished
               (the capture ended first, with the commands still unchecked).
    """

    def __init__(self, steps, agent, log_path, shots_dir=None, guest_share='/research'):
        self.steps = steps
        self.agent = agent
        self.log = open(log_path, 'a')
        self.shots_dir = Path(shots_dir) if shots_dir else None
        self.guest_share = guest_share
        self.index = 0
        self.gen = None
        self.waiting = None  # ('job', n, settle, seen, row), ('updates', left), ('event', kind, after),
        #                      ('fresh',), ('push', pushes) or ('drain',)
        self.pushes = []
        self.push_cursor = 0  # pushes before it were matched by an expect-queued
        self.fresh = False  # self.m was read at this update return (not at a dialog stop)
        self.unchecked = []  # (n, row): commands whose exit code is still to be checked
        self.dialogs = []
        self.finished = False
        self.failed = None
        self.update = 0
        self.m = None
        self.sim = None
        # (object pointers, budget), for expect-new-object: the registry at the
        # scenario's first update return or the last expect-new-object, less
        # the objects removed since (so a pointer or ID used again is new).
        self.known = None

    def close(self):
        if not self.finished:
            self.write(kind='scenario', status='unfinished', index=self.index,
                       unchecked=[n for n, _ in self.unchecked])
        self.log.close()

    def write(self, **row):
        row.setdefault('update', self.update)
        row['time'] = time.time()
        self.log.write(json.dumps(row, default=str) + '\n')
        self.log.flush()

    # Calls from capture.py.

    def on_queue_push(self, update, in_update, m, regs):
        person, action = regs['edi'], regs['esi']
        try:
            row = dict(kind='queued', update=update, in_update=in_update, person=m.object_id(person),
                       action=m.action(action), action_pointer=action, step=self.index)
        except Exception as e:  # reading the push must never end the capture
            self.write(kind='queued_unread', update=update, in_update=in_update, person_pointer=person,
                       action_pointer=action, error=f'{type(e).__name__}: {e}')
            return
        self.pushes.append(row)
        self.write(**row)

    def on_dialog(self, kind, update):
        """A dialog stop. A modal dialog holds the lot update it shows in, so
        no update returns until it is answered. So here a command's end is
        checked but no settling is counted, and a dialog that shows while a
        command settles ends the settling: if the command has not ended yet,
        its exit code is checked at a later stop (the agent runs commands in
        order, so a later press still comes after it; a shot's file is
        converted then). Memory is not read here (see Fresh)."""
        self.dialogs.append((kind, update))
        self.fresh = False
        if self.finished:
            return
        try:
            if kind == 'dialog_show' and self.waiting and self.waiting[0] == 'job':
                _, n, settle, seen, row = self.waiting
                if seen is None and self.agent.result(n) is None:
                    self.unchecked.append((n, row))
                elif seen is None and not self.ended(n, row):
                    return
                self.write(kind='command_wait_ended', id=n, reason='dialog_show', in_update=update)
                self.waiting = None
            if self.waiting and self.waiting[0] not in ('event', 'job', 'push', 'drain'):
                return
            self.tick(at_update=False)
        except Exception as e:
            self.fail(f'{type(e).__name__}: {e}')

    def count(self, kind):
        return sum(1 for d in self.dialogs if d[0] == kind)

    def on_update(self, update, m, sim):
        self.update, self.m, self.sim = update, m, sim
        self.fresh = True
        if self.finished:
            return
        try:
            slots = object_slots(m, sim)
            if self.known is None:
                self.known = (slots, read_budget(m, sim))
            else:
                self.known = (self.known[0] & slots, self.known[1])
            self.tick(at_update=True)
        except Exception as e:  # a scenario must never end the capture
            self.fail(f'{type(e).__name__}: {e}')

    def ended(self, n, row):
        """Log command n's end; False (and the scenario failed) if it failed."""
        code = self.agent.result(n)
        self.write(kind='command_ended', id=n, code=code, what=row['what'])
        if code:
            self.fail(f'guest command {n} ({row["what"]}) failed with {code}')
            return False
        if row['what'] == 'shot':
            local = self.agent.dir / f'shot-{row["name"]}.xwd'
            self.shots_dir.mkdir(exist_ok=True)
            from guest import xwd_to_png
            xwd_to_png(local, self.shots_dir / f'{row["name"]}.png')
            local.unlink()
        return True

    def tick(self, at_update=True):
        for item in list(self.unchecked):
            if self.agent.result(item[0]) is not None:
                self.unchecked.remove(item)
                if not self.ended(*item):
                    return
        if self.waiting:
            kind = self.waiting[0]
            if kind == 'job':
                _, n, settle, seen, row = self.waiting
                if seen is None:
                    if self.agent.result(n) is None:
                        return
                    if not self.ended(n, row):
                        return
                    seen = 0
                if seen < settle:
                    if at_update:
                        seen += 1
                    if seen < settle:
                        self.waiting = ('job', n, settle, seen, row)
                        return
            elif kind == 'event':
                if self.count(self.waiting[1]) <= self.waiting[2]:
                    return
            elif kind == 'fresh':
                if not self.fresh:
                    return
            elif kind == 'push':
                if not at_update and len(self.pushes) <= self.waiting[1]:
                    return
            elif kind == 'drain':
                if self.unchecked:
                    return
            else:
                left = self.waiting[1] - 1
                if left > 0:
                    self.waiting = ('updates', left)
                    return
            self.waiting = None
        self.advance()

    # Stepping.

    def advance(self):
        while not self.finished and not self.waiting:
            if self.gen is None and self.index >= len(self.steps):
                if self.unchecked:  # a dialog cut a command's wait short: see its end first
                    self.waiting = ('drain',)
                    return
                self.finished = True
                self.write(kind='scenario', status='done')
                return
            if self.gen is None:
                step = self.steps[self.index]
                self.write(kind='step', status='start', index=self.index, line=step['line'], text=step['text'])
                self.gen = getattr(self, 'step_' + step['op'].replace('-', '_'))(*step['args'])
                send = None
            else:
                send = self.m
            try:
                want = self.gen.send(send) if send is not None else next(self.gen)
            except StopIteration:
                self.write(kind='step', status='done', index=self.index)
                self.gen = None
                self.index += 1
                continue
            except (StepFailed, MemoryError) as e:
                self.fail(str(e))
                return
            if isinstance(want, Job):
                n = self.agent.submit(want.command)
                row = dict(kind='command', id=n, what=want.kind, command=want.command, **want.info)
                self.write(**row)
                if want.wait:
                    self.waiting = ('job', n, want.settle, None, row)
                else:  # not waited on, but its exit code is still checked
                    self.unchecked.append((n, row))
            elif isinstance(want, Event):
                self.waiting = ('event', want.kind, want.after)
            elif isinstance(want, Fresh):
                if not self.fresh:
                    self.waiting = ('fresh',)
            elif isinstance(want, Updates):
                if want.n > 0:
                    self.waiting = ('updates', want.n)
            elif isinstance(want, UntilPush):
                if len(self.pushes) <= want.pushes:
                    self.waiting = ('push', want.pushes)
            else:
                raise TypeError(want)

    def fail(self, error):
        self.failed = error
        self.finished = True
        self.gen = None
        self.write(kind='step', status='failed', index=self.index, error=error)

    # The steps. Each is a generator; after each yield self.m reads the game at
    # the update return where the wait ended.

    def step_until(self, n):
        while self.update < n:
            yield Updates(1)

    def step_wait(self, n):
        yield Updates(n)

    def step_move(self, x, y):
        yield Job('move', move_command(x, y, self.agent.window), x=x, y=y)

    def step_click(self, x, y):
        yield Job('move', move_command(x, y, self.agent.window), x=x, y=y)
        yield from self.held_press(x=x, y=y)

    def held_press(self, **info):
        """Press the left button where the pointer is, keep it down until
        HOLD_UPDATES lot updates have returned, then let it go. The release
        is not waited on (its exit code is still checked): the press can open
        a modal dialog, which stops update returns, and the agent runs
        commands in order, so a later press still comes after the release."""
        yield Job('press', press_down_command(), settle=HOLD_UPDATES, **info)
        yield Job('release', press_up_command(), wait=False, **info)

    def step_answer_dialog(self, x, y):
        """Wait for a dialog to be up, press (x, y) once, and wait for the
        dialog's result. The press is not waited on: no update returns while
        a modal dialog is up. If the game misses the press, the step waits
        until the dialog is answered some other way (answer.sh)."""
        shows, results = self.count('dialog_show'), self.count('dialog_result')
        if shows <= results:
            yield Event('dialog_show', shows)
        results = self.count('dialog_result')
        yield Job('press', dialog_press_command(x, y, self.agent.window), wait=False, x=x, y=y)
        yield Event('dialog_result', results)

    def step_press(self, x, y):
        """A press on a button of the game's panels (as on a dialog's)."""
        yield Job('press', dialog_press_command(x, y, self.agent.window), x=x, y=y)

    def step_expect_mode(self, mode, within):
        yield Fresh()
        start = self.update
        while read_mode(self.m, self.sim) != mode:
            if self.update - start >= within:
                raise StepFailed(f'mode is {read_mode(self.m, self.sim)}, not {mode}, after {within} updates')
            yield Updates(1)
        self.write(kind='mode', mode=mode)

    def step_expect_new_object(self, within):
        """Wait for objects that were not in the registry at the scenario's
        first update return (or the last expect-new-object), and log them
        with the budget then and now."""
        yield Fresh()
        start = self.update
        while True:
            known, budget = self.known
            slots = object_slots(self.m, self.sim)
            new = slots - known
            if new:
                rows = sorted((placed_object(self.m, p) for p in new), key=lambda o: o['id'])
                now = read_budget(self.m, self.sim)
                self.write(kind='placed', objects=rows, budget_before=budget, budget=now, cost=budget - now)
                self.known = (slots, now)
                return
            if self.update - start >= within:
                raise StepFailed(f'no new object within {within} updates')
            yield Updates(1)

    def step_key(self, keysym):
        yield Job('key', key_command(keysym), key=keysym)

    def step_shot(self, name):
        if not self.shots_dir:
            raise StepFailed('shot needs a capture directory')
        if not name.replace('-', '').replace('_', '').isalnum():
            raise StepFailed(f'bad shot name {name!r}')
        guest = f'{self.guest_share}/input/shot-{name}.xwd'
        # The PNG is made when the command is seen to end (Runner.ended).
        yield Job('shot', shot_command(self.agent.window, guest), settle=1, name=name)

    def step_click_object(self, oid):
        """Aim, press, and accept the pie menu only if its actions are aimed at
        oid. The picker's hover lags the pointer (the game takes pointer moves
        late), so it only says when to press; the pie menu's actions say what
        was clicked. A menu for something else is closed and the next aim
        point tried."""
        tried = []
        for attempt in range(len(aim_points(0, 0, 0))):
            yield Fresh()
            p = find_object(self.m, self.sim, oid)
            if not p:
                raise StepFailed(f'no object {oid}')
            view = read_view(self.m)
            anchor = project(object_position(self.m, p), view)
            point = clamp(aim_points(*anchor, view['zoom'])[attempt])
            yield Job('move', move_command(*point, self.agent.window), x=point[0], y=point[1],
                      aim=oid, anchor=anchor)
            yield Fresh()
            hover = yield from self.steady_hover()
            self.write(kind='hover', x=point[0], y=point[1], object=hover, want=oid)
            tried.append((point, hover))
            if hover != oid:
                continue
            yield from self.held_press(x=point[0], y=point[1], object=oid)
            yield Fresh()
            entries = yield from self.wait_entries(200)
            if not entries:
                raise StepFailed(f'no pie menu opened on object {oid}')
            targets = self.log_pie(read_pie(self.m), entries)
            if not targets or oid in targets:
                return
            self.write(kind='wrong_menu', want=oid, targets=sorted(targets))
            yield from self.close_pie()
        raise StepFailed(f'object {oid} was never clicked: {tried}')

    def steady_hover(self, reads=3, most=40):
        """The picker's hover once it reads the same `reads` times in a row."""
        last, same = None, 0
        for _ in range(most):
            hover = read_hover(self.m)
            same = same + 1 if hover == last else 1
            last = hover
            if same >= reads:
                break
            yield Updates(1)
        return last

    def wait_entries(self, most):
        for _ in range(most):
            entries = read_entries(self.m)
            if entries:
                return entries
            yield Updates(1)
        return []

    def close_pie(self):
        yield Job('key', key_command('Escape'), key='Escape')
        yield Fresh()
        for _ in range(200):
            if not read_entries(self.m):
                return
            yield Updates(1)
        raise StepFailed('the pie menu did not close')

    def log_pie(self, pie, entries):
        rows = []
        for e in entries:
            row = dict(label=e['label'], submenu=e['submenu'])
            if e['action']:
                row['action'] = self.m.action(e['action'])
            rows.append(row)
        self.write(kind='pie', items=pie and pie['items'], origin=pie and pie['origin'], entries=rows)
        return {r['action']['target'] for r in rows if 'action' in r}

    def step_choose(self, label):
        yield Fresh()
        pie = read_pie(self.m)
        if not pie or not read_entries(self.m):
            raise StepFailed('no pie menu is open')
        i = match_item(pie['items'], label)
        if i is None:
            raise StepFailed(f'no entry {label!r} in {[it["label"] for it in pie["items"]]}')
        item = pie['items'][i]
        point = clamp((item['x'] + item['w'] // 2, item['y'] + item['h'] // 2))
        pushes = len(self.pushes)
        self.push_cursor = pushes  # expect-queued looks only at what this choice pushes
        yield Job('move', move_command(*point, self.agent.window), x=point[0], y=point[1], entry=label)
        yield Fresh()
        for _ in range(60):  # the game takes the move late
            pie = read_pie(self.m)
            if pie and pie['highlighted'] == i:
                break
            yield Updates(1)
        else:
            raise StepFailed(f'entry {label!r} ({i}) is not highlighted: {pie and pie["highlighted"]}')
        yield from self.held_press(x=point[0], y=point[1], entry=label)
        for _ in range(200):
            if len(self.pushes) > pushes:  # known without reading memory (a dialog may be up)
                return
            yield Fresh()
            if not read_entries(self.m):
                return
            yield UntilPush(pushes)
        raise StepFailed(f'choosing {label!r} did nothing')

    def step_expect_queued(self, person, target, within):
        """Wait for a push on person (aimed at target), then for the person to
        hold that action, queued or already running."""
        yield Fresh()
        start = self.update
        push = None
        while True:
            if push is None:
                for i in range(self.push_cursor, len(self.pushes)):
                    r = self.pushes[i]
                    if r['person'] == person and (target is None or r['action']['target'] == target):
                        push, self.push_cursor = r, i + 1
                        break
            p = push and find_object(self.m, self.sim, person)
            if p:
                queue, active = read_queue(self.m, p), self.m.action(p + ACTIVE_ACTION)
                where = ('active' if same_action(active, push['action']) else
                         'queue' if any(same_action(a, push['action']) for a in queue) else None)
                if where:
                    self.write(kind='taken', person=person, where=where, pushed_update=push['update'],
                               active=active, queue=queue)
                    return
            if self.update - start >= within:
                if push:
                    raise StepFailed(f'the action pushed on {person} was in neither its queue nor its active '
                                     f'action within {within} updates')
                raise StepFailed(f'no action was queued on {person} within {within} updates')
            yield Updates(1)
