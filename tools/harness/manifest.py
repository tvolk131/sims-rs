#!/usr/bin/env python3
"""Write manifest.json next to a capture: what it holds and what happened in it.

  manifest.py <capture dir> [--note TEXT]... [--house TEXT]

Read from the capture's own files (updates.jsonl, events.jsonl, status.json,
input-log.txt, input.jsonl): the updates recorded and the ticks they cover,
which updates have a full state (as ranges, with why each was saved), the
clock at the start and the end, how many ticks the game day took (between
two turns of the same hour, when the capture holds them), the hours turned,
the objects that appeared or left, the dialogs (when each was shown and
answered, with its result and whether a press was made), the presses made,
and the queued actions of a scenario. Nothing is read from the guest.
"""
import argparse
import json
import re
from pathlib import Path


def rows(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def ranges(numbers):
    """[1, 2, 3, 5, 10, 20, 30] -> '1-3,5,10-30/10'. A run of three or more with one step is a range."""
    numbers = sorted(set(numbers))
    out, i = [], 0
    while i < len(numbers):
        j = i
        if i + 2 < len(numbers):
            step = numbers[i + 1] - numbers[i]
            while j + 1 < len(numbers) and numbers[j + 1] - numbers[j] == step:
                j += 1
            if j - i < 2:
                j = i
        if j == i:
            out.append(str(numbers[i]))
            i += 1
        else:
            step = numbers[i + 1] - numbers[i]
            out.append(f'{numbers[i]}-{numbers[j]}' + (f'/{step}' if step != 1 else ''))
            i = j + 1
    return ','.join(out)


def presses(text):
    """The presses in input-log.txt: [{time, label, command}]."""
    out, when = [], None
    for line in text.splitlines():
        if re.fullmatch(r'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ', line.strip()):
            when = line.strip()
            continue
        if line.strip():
            label, _, command = line.partition(': ')
            out.append(dict(time=when, label=label, command=command))
    return out


def day_length(hours):
    """Ticks between the first two turns to the same hour (a whole game day), or None."""
    first = {}
    for h in hours:
        hour = h['clock'][0] % 24
        if hour in first:
            return dict(from_update=first[hour]['update'], to_update=h['update'], hour=hour,
                        ticks=h['tick'] - first[hour]['tick'], updates=h['update'] - first[hour]['update'])
        first[hour] = h
    return None


def build(capture, notes=(), house=None):
    capture = Path(capture)
    updates = rows(capture / 'updates.jsonl')
    events = rows(capture / 'events.jsonl')
    status = json.loads((capture / 'status.json').read_text()) if (capture / 'status.json').exists() else {}
    saved = [r for r in updates if 'saved' in r]
    why = {}
    for r in saved:
        for reason in r.get('saved_for', ['full']):
            why.setdefault(reason, []).append(r['update'])
    hours = [dict(update=e['update'], tick=e['tick'], clock=e['clock'], date=e.get('date')) for e in events
             if e['kind'] == 'hour_turned']
    dialogs = []
    for e in events:
        if e['kind'] == 'dialog_show':
            dialogs.append(dict(update=e['update'], record=e.get('record'), shown=e['time'], result=None,
                                answered=None, auto_presses=0))
        elif e['kind'] == 'dialog_result' and dialogs:
            open_ = [d for d in dialogs if d['result'] is None]
            if open_:
                open_[-1].update(result=e.get('status'), answered=e['time'], result_update=e['update'])
        elif e['kind'] == 'dialog_auto_answer' and dialogs:
            dialogs[-1]['auto_presses'] += 1
    for d in dialogs:
        d['seconds_open'] = round(d['answered'] - d['shown'], 1) if d['answered'] else None
        del d['shown'], d['answered']
    manifest = dict(
        capture=capture.name,
        house=house,
        notes=list(notes),
        state=status.get('state'),
        settings=status.get('settings'),
        game_identity=status.get('game_identity'),
        baseline=next((dict(tick=e['tick'], lot=e['lot']) for e in events if e['kind'] == 'baseline'), None),
        updates=len(updates),
        ticks=[updates[0]['tick'], updates[-1]['tick']] if updates else None,
        partial_passes=[r['update'] for r in updates if not r.get('advanced', 1)],
        clock=dict(first=updates[0].get('clock'), last=updates[-1].get('clock'),
                   first_date=updates[0].get('date'), last_date=updates[-1].get('date')) if updates else None,
        full_state=dict(count=len(saved), updates=ranges(r['update'] for r in saved),
                        entry_0000=(capture / 'entry-0000').is_dir(),
                        by_reason={k: ranges(v) for k, v in sorted(why.items())}),
        day=day_length(hours),
        hours_turned=hours,
        objects_changed=[dict(update=e['update'], tick=e['tick'], added=[a['id'] for a in e['added']],
                              removed=[r['id'] for r in e['removed']], count=e['count'])
                         for e in events if e['kind'] == 'objects_changed'],
        dialogs=dialogs,
        presses=presses((capture / 'input-log.txt').read_text()) if (capture / 'input-log.txt').exists() else [],
        queued=[r for r in rows(capture / 'input.jsonl') if r.get('kind') == 'queued'],
        errors=[e for e in events if e['kind'] in ('error', 'unexpected_stop')],
        dumps=[e['update'] for e in events if e['kind'] == 'dump'],
    )
    return manifest


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('capture', type=Path)
    ap.add_argument('--note', action='append', default=[], help='a line of free text for the manifest')
    ap.add_argument('--house', help='the house file and family, as free text')
    args = ap.parse_args()
    manifest = build(args.capture, args.note, args.house)
    out = args.capture / 'manifest.json'
    out.write_text(json.dumps(manifest, indent=1) + '\n')
    print(out)


if __name__ == '__main__':
    main()
