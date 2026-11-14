#!/usr/bin/env python3
"""Drive the guest through the share: run a shell job, take a screenshot, click.

  guest.py <share> run '<shell>' [timeout]
  guest.py <share> shot <out.png>
  guest.py <share> click X Y [hold_seconds]     (left button, coordinates in the game window $SIMS_WINDOW)
  guest.py <share> move X Y
The guest's /sims-init.sh runs /research/cmd/<n>.sh and writes <n>.out, <n>.done.
click and move exit with the guest job's exit code; click stops if the pointer cannot be placed.
"""
import os, re, struct, sys, time, zlib, binascii
from pathlib import Path

WINDOW = re.compile(r'0x[0-9a-fA-F]{1,8}|[0-9]{1,10}')
NUMBER = re.compile(r'-?[0-9]{1,6}')
SECONDS = re.compile(r'[0-9]{1,3}(\.[0-9]{1,3})?')


def submit(share, script):
    """Hand a shell job to the guest and return its number. Safe when several
    processes submit at once: a number is reserved by creating its .tmp file
    exclusively, and used only if no job file for it exists yet (a .done left
    without its .sh would make the guest skip the job)."""
    cmd = Path(share) / 'cmd'
    cmd.mkdir(exist_ok=True)
    n = len(list(cmd.glob('*.sh'))) + 1
    while True:
        tmp = cmd / f'{n:05d}.tmp'
        try:
            fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
        except FileExistsError:
            n += 1
            continue
        if any((cmd / f'{n:05d}{suffix}').exists() for suffix in ('.sh', '.started', '.out', '.done')):
            os.close(fd)
            tmp.unlink()
            n += 1
            continue
        with os.fdopen(fd, 'w') as f:
            f.write(script)
        tmp.rename(cmd / f'{n:05d}.sh')  # only the holder of n.tmp writes n.sh
        return n


def run(share, script, timeout=600):
    """Run a shell job in the guest; (exit code, output)."""
    n = submit(share, 'set -u\nexport DISPLAY=:98\n' + script + '\n')
    cmd = Path(share) / 'cmd'
    job = cmd / f'{n:05d}.sh'
    deadline = time.time() + timeout
    done = cmd / f'{n:05d}.done'
    while not done.exists():
        if time.time() > deadline:
            raise TimeoutError(f'guest job {job} did not finish')
        time.sleep(0.5)
    time.sleep(0.2)
    code = int(done.read_text().strip() or '1')
    return code, (cmd / f'{n:05d}.out').read_text(errors='replace')


def xwd_to_png(src, dst):
    raw = Path(src).read_bytes()
    h = struct.unpack('>25I', raw[:100])
    width, height, stride = h[4], h[5], h[12]
    bpp = 16 if h[3] == 16 else 32
    pixels = raw[h[0] + 12 * h[19]:]
    scan = bytearray()
    for y in range(height):
        scan.append(0)
        row = pixels[y * stride:y * stride + (bpp // 8) * width]
        if bpp == 16:
            for (v,) in struct.iter_unpack('<H', row):
                scan.extend((((v >> 11) & 31) * 255 // 31, ((v >> 5) & 63) * 255 // 63, (v & 31) * 255 // 31))
        elif bpp == 32:
            for i in range(0, len(row), 4):
                scan.extend((row[i + 2], row[i + 1], row[i]))
        else:
            raise ValueError(f'{bpp} bits per pixel')
    chunk = lambda t, p: struct.pack('>I', len(p)) + t + p + struct.pack('>I', binascii.crc32(t + p) & 0xffffffff)
    Path(dst).write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0))
                          + chunk(b'IDAT', zlib.compress(bytes(scan))) + chunk(b'IEND', b''))


def checked(pattern, value, what):
    if not pattern.fullmatch(value):
        sys.exit(f'bad {what}: {value!r}')
    return value


def main():
    share, what, *rest = sys.argv[1:]
    if what == 'run':
        code, out = run(share, rest[0], int(rest[1]) if len(rest) > 1 else 600)
        sys.stdout.write(out)
        sys.exit(code)
    if what == 'shot':
        name = f'shot-{time.time():.0f}.xwd'
        win = checked(WINDOW, os.environ.get('SIMS_WINDOW', '0xe00013'), 'window')
        code, out = run(share, f'xwd -id {win} -silent -out /research/{name}', 300)
        if code:
            sys.exit(out)
        xwd_to_png(Path(share) / name, rest[0])
        os.remove(Path(share) / name)
        print(rest[0])
        return
    if what in ('move', 'click'):
        x, y = (checked(NUMBER, v, 'coordinate') for v in rest[:2])
        win = checked(WINDOW, os.environ.get('SIMS_WINDOW', '0xe00013'), 'window')
        move = f'xdotool mousemove --window {win} {x} {y}'
        if what == 'move':
            code, out = run(share, move, 120)
        else:
            hold = checked(SECONDS, rest[2] if len(rest) > 2 else '3', 'hold')
            # Stop if the pointer cannot be placed; once the button is down, always let it go.
            code, out = run(share, f'{move} || exit $?\nsleep 1\nxdotool mousedown 1 || exit $?\n'
                                   f'sleep {hold}\nxdotool mouseup 1', 300)
        sys.stdout.write(out)
        sys.exit(code)
    sys.exit(__doc__)


if __name__ == '__main__':
    main()
