#!/usr/bin/env python3
"""Differential test of civrs against CIV.EXE from the same original saved game.

    python3 -I sve_diff.py play GAME_DIR SLOT TURNS OUT_DIR
        Start CIV.EXE in dosbox (dosciv.py) on GAME_DIR (a private writable copy of the game),
        load CIVIL<SLOT>.SVE, then TURNS times: skip the active unit (Space), save to slot 2
        (CIVIL2.SVE) and copy every new turn's save to OUT_DIR/t<TURN>.SVE/.MAP. Space only
        skips a unit, so the human's units keep their orders; check OUT_DIR/shot*.png for
        dialogs the Space/Return keys answered (production menus, advances).
    python3 -I sve_diff.py diff CIVRS START.SVE RESULT.SVE [RESULT.SVE ...]
        For each CIV.EXE result: civrs imports START.SVE, plays (result turn - start turn)
        turns with the human doing nothing (--dump state:N) and is diffed against civrs's
        import of the result (--dump state:0). Prints `field: CIV.EXE | civrs` per differing line.

The SVE keeps no RNG state (CIV.EXE seeds rand() from the clock at program start), so AI moves,
huts and anything else behind random() diverge; the human's cities, units, gold and research
and the turn/year should agree exactly.
"""
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)


def turn_of(path):
    with open(path, 'rb') as f:
        return struct.unpack('<h', f.read(2))[0]


def play(game, slot, turns, out):
    import dosciv
    os.makedirs(out, exist_ok=True)
    dosciv.start(game)
    k = dosciv.keys
    time.sleep(6)
    # graphics VGA, no sound, keyboard only; the title menu's "Load a Saved Game"; drive C:
    k(['1', 'sleep2', '1', 'sleep8', '2', 'sleep10', 'Return', 'sleep5', 'Down', 'Return', 'sleep3', 'Return', 'sleep2'])
    k(['Down'] * int(slot) + ['Return', 'sleep6'])
    save = os.path.join(game, 'CIVIL2.SVE')
    last = turn_of(os.path.join(game, 'CIVIL%s.SVE' % slot))
    end = last + int(turns)
    shot = os.path.join(out, 'now.png')
    cyan = bytes([12, 227, 235])

    def screen():
        dosciv.shot(shot)
        d = subprocess.run(['magick', shot, '-depth', '8', 'ppm:-'], capture_output=True).stdout
        h = d.split(b'\n', 3)
        w = int(h[1].split()[0])
        return lambda x, y: h[3][3 * (y * w + x):3 * (y * w + x) + 3]

    def save_game():
        # the GAME menu opens with Tax Rate highlighted (cyan at (100, 72)); anything else is
        # a dialog (city view, production or research menu), answered with Return
        k(['alt+g', 'sleep1'])
        if screen()(100, 72) != cyan:
            k(['Return', 'sleep2'])
            return False
        k(['Down', 'Down', 'Down', 'Down', 'Return', 'sleep1', 'Return', 'sleep2'])
        # the slot list: slot n is the band from y = 130 + 16 n; walk the cursor to slot 2
        px = screen()
        rows = [n for n in range(10) if px(106, 130 + 16 * n + 4) == cyan]
        if not rows:
            k(['Escape', 'sleep1'])
            return False
        k((['Up'] * (rows[0] - 2) if rows[0] > 2 else ['Down'] * (2 - rows[0])) + ['Return', 'sleep3', 'Return', 'sleep2'])
        return True

    for i in range(4 * int(turns) + 8):
        # Space ends the turn once no unit waits for orders
        k(['space', 'sleep3'])
        dosciv.shot(os.path.join(out, 'shot%02d.png' % i))
        stamp = os.path.getmtime(save) if os.path.exists(save) else 0
        for _ in range(6):
            if save_game() and os.path.exists(save) and os.path.getmtime(save) != stamp:
                break
            dosciv.shot(os.path.join(out, 'stuck%02d.png' % i))
        t = turn_of(save) if os.path.exists(save) else last
        print('step', i, 'turn', t, flush=True)
        if t != last:
            last = t
            shutil.copy(save, os.path.join(out, 't%d.SVE' % t))
            shutil.copy(save[:-3] + 'MAP', os.path.join(out, 't%d.MAP' % t))
        if last >= end:
            break
    dosciv.stop()


def state(civrs, sve, turns):
    with tempfile.NamedTemporaryFile(suffix='.txt', delete=False) as f:
        path = f.name
    subprocess.run([civrs, '--load-sve', sve, '--dump', 'state:%d' % turns, '--out', path], check=True)
    with open(path) as f:
        lines = f.read().splitlines()
    os.remove(path)
    return dict(l.split(' ', 1) if ' ' in l else (l, '') for l in lines)


def diff(civrs, start, results):
    t0 = turn_of(start)
    for r in results:
        n = turn_of(r) - t0
        exe, rs = state(civrs, r, 0), state(civrs, start, n)
        keys = sorted(set(exe) | set(rs), key=lambda k: (k.split('.')[0].rstrip('0123456789,'), k))
        bad = [k for k in keys if exe.get(k) != rs.get(k)]
        print('== %s: %d turn(s), %d of %d fields differ' % (os.path.basename(r), n, len(bad), len(keys)))
        for k in bad:
            print('%s: %s | %s' % (k, exe.get(k, '-'), rs.get(k, '-')))


if __name__ == '__main__':
    if len(sys.argv) >= 6 and sys.argv[1] == 'play':
        play(*sys.argv[2:6])
    elif len(sys.argv) >= 5 and sys.argv[1] == 'diff':
        diff(sys.argv[2], sys.argv[3], sys.argv[4:])
    else:
        print(__doc__)
