#!/usr/bin/env python3
"""Stop the original at its lot updates and save the state after each one.

Run on the host while the VM container runs the game, before the house is
entered:

  capture.py --container sims-harness-vm --output <new dir> --lot 5 \
      --updates 1000 --full 1-100,110-1000/10

--resume takes over a guest that an earlier run left stopped at one of its
breakpoints (that run was killed while the guest was stopped, so it never
detached) and continues the same output directory and numbering.

Breakpoints (hardware, in QEMU's stub; no guest memory or register is
written): the lot update's entry 0x46d345 (until the first one on the lot)
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
"""
import argparse
import gzip
import json
import os
import struct
import sys
import time
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from rsp import Remote  # noqa: E402
import qemu_retained_owners  # noqa: E402
import qemu_neighborhood_owners as neighborhood_owners  # noqa: E402

ENTRY, RETURN = 0x46d345, 0x46d51d
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
                    self.read(a, 0x2a0)
                    wanted |= {a & ~4095, (a + 0x2a0) & ~4095}
                    fg = self.word(a + 0xc)
                    if fg:
                        self.read(fg, 0x70)
                        wanted |= {fg & ~4095, (fg + 0x70) & ~4095}
        pages = {str(k): self.pages[k].hex() for k in sorted(wanted)}
        return state, full, pages


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


def write_gz(path, value):
    with gzip.open(path, 'wt', compresslevel=6) as f:
        json.dump(value, f, separators=(',', ':'))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--container', default='sims-harness-vm')
    ap.add_argument('--output', required=True, type=Path)
    ap.add_argument('--lot', type=int, default=5)
    ap.add_argument('--updates', type=int, default=100, help='stop after this many lot updates on the lot')
    ap.add_argument('--full', default='1-100', help='lot updates whose full state is saved, e.g. 1-100,110-1000/10')
    ap.add_argument('--stop-file', type=Path, help='end the capture when this file appears')
    ap.add_argument('--resume', action='store_true',
                    help='take over a guest that an earlier run of this script left stopped at a breakpoint '
                         '(it was killed, not detached): continue its output directory and numbering')
    args = ap.parse_args()
    full_set = parse_set(args.full)
    out = args.output
    out.mkdir(parents=True, exist_ok=args.resume)
    events = open(out / 'events.jsonl', 'a')
    updates_log = open(out / 'updates.jsonl', 'a')
    status = dict(started=time.time(), updates=0, state='attaching', settings={k: str(v) for k, v in vars(args).items()})

    def event(**row):
        row['time'] = time.time()
        events.write(json.dumps(row, default=str) + '\n')
        events.flush()

    def save_status(**kw):
        status.update(kw, now=time.time())
        tmp = out / 'status.json.tmp'
        tmp.write_text(json.dumps(status, indent=1, default=str))
        tmp.rename(out / 'status.json')

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
    baseline = False
    updates = 0
    elsewhere = 0
    if args.resume:
        rows = [json.loads(line) for line in open(out / 'events.jsonl')]
        sims_cr3 = next(r['cr3'] for r in rows if r['kind'] == 'baseline')
        baseline = True
        updates = sum(1 for _ in open(out / 'updates.jsonl'))
    try:
        reply = remote.stop_reply()
        event(kind='attach', reply=reply, resume=args.resume, updates=updates)
        for a in (ENTRY, RETURN, DIALOG_SHOW, DIALOG_RESULT, DIALOG_READ, 0x415eff):
            remote.clear_break(a)
            if args.resume:
                remote.clear_break(a)
        for a in ((RETURN,) if baseline else (ENTRY,)) + (DIALOG_SHOW, DIALOG_RESULT, DIALOG_READ):
            arm(a)
        save_status(state='running', breaks=sorted(breaks))
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
            if took_over and pc not in breaks:
                # The earlier run was killed between stepping past a breakpoint and resuming.
                event(kind='took_over_off_break', pc=pc, registers=regs)
                continue
            if took_over and pc == RETURN:
                last = json.loads(open(out / 'updates.jsonl').read().splitlines()[-1])
                now = m.summary(regs['ebx'], fs_base)
                event(kind='took_over_at_return', last=last, now=now)
                if all(last[k] == now[k] for k in now):
                    raise RuntimeError('took over at a return already recorded; refusing to guess')
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
                    event(kind='baseline', tick=state['tick'], lot=lot, cr3=cr3, teb=fs_base, seconds=time.time() - t0,
                          packets=remote.packets, sim=regs['ecx'])
                    disarm(ENTRY)
                    arm(RETURN)
                else:
                    elsewhere += 1
                    if elsewhere in (1, 10, 100) or elsewhere % 1000 == 0:
                        event(kind='entry_elsewhere', lot=lot, cr3=cr3, count=elsewhere)
            elif pc == RETURN:
                sim = regs['ebx']
                if lot != args.lot:
                    event(kind='return_elsewhere', lot=lot)
                else:
                    updates += 1
                    advanced = m.byte(regs['ebp'] - 1)
                    row = dict(update=updates, advanced=advanced, **m.summary(sim, fs_base))
                    if updates in full_set:
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
                        row['seconds'] = round(time.time() - t0, 2)
                    updates_log.write(json.dumps(row) + '\n')
                    updates_log.flush()
                    save_status(updates=updates, last=row)
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
                except MemoryError as e:
                    row['error'] = str(e)
                event(**row)
                if pc == DIALOG_SHOW:
                    save_status(dialog=dict(update=updates + 1, record=record, shown=time.time()))
            # Step past the breakpoint we stopped at (if it is still armed).
            armed = pc in breaks
            if armed:
                remote.clear_break(pc)
            remote.step()
            if armed:
                remote.set_break(pc)
        event(kind='complete', updates=updates)
        save_status(state='complete')
    except BaseException as e:
        event(kind='error', error=repr(e))
        save_status(state='error', error=repr(e))
        raise
    finally:
        try:
            if not remote.stopped:
                remote.interrupt()
            for a in list(breaks):
                remote.clear_break(a)
        finally:
            remote.detach()


if __name__ == '__main__':
    main()
