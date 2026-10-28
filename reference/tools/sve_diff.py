#!/usr/bin/env python3
"""Differential test of civrs against CIV.EXE from the same original saved game.

    python3 -I sve_diff.py play GAME_DIR SLOT TURNS OUT_DIR [--actions SCRIPT]
        Start CIV.EXE in dosbox (dosciv.py) on GAME_DIR (a private writable copy of the game),
        load CIVIL<SLOT>.SVE, turn Instant Advice off and End of Turn on, then TURNS times:
        perform the script's actions of the turn (src/save/actions.rs; examples in
        actions/), skip the active unit (Space) until the turn ends, save to slot 2
        (CIVIL2.SVE) and copy every new turn's save to OUT_DIR/t<TURN>.SVE/.MAP. Space only
        skips a unit, so the human's units keep their orders; check OUT_DIR/shot*.png and
        act*.png for dialogs the Space/Return keys answered (production menus, advances).
    python3 -I sve_diff.py diff CIVRS START.SVE RESULT.SVE [RESULT.SVE ...] [--actions SCRIPT]
        For each CIV.EXE result: civrs imports START.SVE, plays (result turn - start turn)
        turns with the human doing only the script's actions (--dump state:N) and is diffed
        against civrs's import of the result (--dump state:0). Prints `field: CIV.EXE | civrs`
        per differing line.

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
sys.dont_write_bytecode = True


def turn_of(path):
    with open(path, 'rb') as f:
        return struct.unpack('<h', f.read(2))[0]


# keypad digit -> X key (CIV.EXE reads the keypad scan codes; NumLock is off)
KEYPAD = {'1': 'KP_End', '2': 'KP_Down', '3': 'KP_Next', '4': 'KP_Left', '6': 'KP_Right', '7': 'KP_Home', '8': 'KP_Up', '9': 'KP_Prior'}


def parse_actions(path):
    """The action script (src/save/actions.rs) as {turn: [words, ...]}."""
    out, turn = {}, None
    for line in open(path):
        w = line.split('#')[0].split()
        if not w:
            continue
        if w[0] == 'turn':
            turn = int(w[1])
        else:
            out.setdefault(turn, []).append(w)
    return out


def rows(n):
    n = int(n)
    return ['Down'] * n if n > 0 else ['Up'] * -n


def unit_keys(k):
    if k[0] == 'k':
        return [KEYPAD[k[1]], 'sleep1']
    if k == 'b':
        # the name prompt takes the proposed name, then the founding animation and city view
        return ['b', 'sleep2', 'Return', 'sleep8', 'Escape', 'sleep2']
    return [{'F': 'shift+f', 'P': 'shift+p'}.get(k, k), 'sleep1']


def steps_to(dx, dy):
    """Keypad steps for the city screen's square cursor ('p') from the centre to (dx, dy)."""
    out = []
    while (dx, dy) != (0, 0):
        sx, sy = (dx > 0) - (dx < 0), (dy > 0) - (dy < 0)
        out.append(KEYPAD[str({(-1, 1): 1, (0, 1): 2, (1, 1): 3, (-1, 0): 4, (1, 0): 6, (-1, -1): 7, (0, -1): 8, (1, -1): 9}[(sx, sy)])])
        dx, dy = dx - sx, dy - sy
    return out


def act(w, k, click, shot):
    """Perform one script action in CIV.EXE (the keys of the map view and the city screen)."""
    if w[0] == 'unit':
        for key in w[2:]:
            k(unit_keys(key))
    elif w[0] == 'select':
        # FindCity centres the map on the city; then a click on the unit's square
        (x, y), name, (cx, cy) = map(int, w[1].split(',')), w[2], map(int, w[3].split(','))
        k(['slash', 'sleep1'] + list(name.lower()) + ['Return', 'sleep2'])
        vx, vy = (cx - 7) % 80, min(max(cy - 6, 0), 38)
        click(80 + 16 * ((x - vx) % 80) + 8, 8 + 16 * (y - vy) + 8)
        # pick_unit_from_stack (0x9dc5) lists even a lone unit: Return takes it
        k(['sleep1', 'Return', 'sleep1'])
    elif w[0] in ('tax', 'lux'):
        # '=' ('+') and '-' open the rate menus at the current rate
        k([{'tax': 'equal', 'lux': 'minus'}[w[0]], 'sleep1'] + rows(w[1]) + ['Return', 'sleep2'])
    elif w[0] == 'revolution':
        # GAME menu row 5, "_Yes, we need a new government.", then the newspaper
        k(['alt+g', 'sleep1'] + ['Down'] * 5 + ['Return', 'sleep1', 'Down', 'Return', 'sleep3', 'Return', 'sleep2'])
    elif w[0] == 'city':
        # FindCity ('/') centres the map on the city (column 7, row y - clamp(y - 6, 0, 38));
        # a click on it opens the city screen
        name, (x, y), op = w[1], map(int, w[2].split(',')), w[3:]
        k(['slash', 'sleep1'] + list(name.lower()) + ['Return', 'sleep2'])
        row = y - min(max(y - 6, 0), 38)
        click(80 + 7 * 16 + 8, 8 + 16 * row + 8)
        time.sleep(2)
        shot()
        if op[0] == 'change':
            k(['c', 'sleep2'] + rows(op[1]) + ['Return', 'sleep2'])
        elif op[0] == 'buy':
            k(['b', 'sleep2', 'Return', 'sleep2'])
        elif op[0] == 'sell':
            k(['s', 'sleep1'] + rows(op[1]) + ['Return', 'sleep2', 'Down', 'Return', 'sleep2'])
        elif op[0] == 'spec':
            k([op[1], 'sleep2'])
        elif op[0] == 'tile':
            dx, dy = map(int, op[1].split(','))
            k(['p', 'sleep1'] + steps_to(dx, dy) + ['Return', 'sleep2', 'Escape', 'sleep1'])
        shot()
        k(['Escape', 'sleep2'])
    else:
        raise ValueError('unknown action %s' % w)


def play(game, slot, turns, out, script=None):
    import dosciv
    os.makedirs(out, exist_ok=True)
    actions = parse_actions(script) if script else {}
    dosciv.start(game)
    k = dosciv.keys
    time.sleep(6)
    # graphics VGA, no sound, mouse and keyboard; the title menu's "Load a Saved Game"; drive C:
    k(['1', 'sleep2', '1', 'sleep8', '1', 'sleep10', 'Return', 'sleep5', 'Down', 'Return', 'sleep3', 'Return', 'sleep2'])
    k(['Down'] * int(slot) + ['Return', 'sleep6'])
    # Options (SVE 0x8aa4): Instant Advice off (its notes would take keys) and End of Turn on
    # (else the turn ends by itself after the last unit's move, before the script is done)
    with open(os.path.join(game, 'CIVIL%s.SVE' % slot), 'rb') as f:
        f.seek(0x8aa4)
        opts = f.read(1)[0]
    toggle = (['Return'] if opts & 1 else []) + ['Down', 'Down'] + (['Return'] if not opts & 4 else [])
    k(['alt+g', 'sleep1', 'Down', 'Down', 'Down', 'Return', 'sleep1'] + toggle + ['Escape', 'sleep1'])
    save = os.path.join(game, 'CIVIL2.SVE')
    last = turn_of(os.path.join(game, 'CIVIL%s.SVE' % slot))
    end = last + int(turns)
    shot = os.path.join(out, 'now.png')
    cyan = bytes([12, 227, 235])
    n_shot = [0]

    def screen():
        dosciv.shot(shot)
        d = subprocess.run(['magick', shot, '-depth', '8', 'ppm:-'], capture_output=True).stdout
        h = d.split(b'\n', 3)
        w = int(h[1].split()[0])
        return lambda x, y: h[3][3 * (y * w + x):3 * (y * w + x) + 3]

    def act_shot():
        n_shot[0] += 1
        dosciv.shot(os.path.join(out, 'act%03d.png' % n_shot[0]))

    def run_actions(t):
        for w in actions.get(t, []):
            print('turn', t, 'action', ' '.join(w), flush=True)
            act(w, k, dosciv.click, act_shot)
            act_shot()

    def save_game():
        # the GAME menu opens with Tax Rate highlighted (cyan at (100, 32)); anything else is
        # a dialog (city view, production or research menu), answered with Return
        k(['alt+g', 'sleep1'])
        if screen()(100, 32) != cyan:
            k(['Return', 'sleep2'])
            return False
        k(['Down', 'Down', 'Down', 'Down', 'Return', 'sleep1', 'Return', 'sleep2'])
        # the slot list: slot n is the band from y = 90 + 16 n; walk the cursor to slot 2
        px = screen()
        rows_ = [n for n in range(10) if px(106, 90 + 16 * n + 4) == cyan]
        if not rows_:
            k(['Escape', 'sleep1'])
            return False
        k((['Up'] * (rows_[0] - 2) if rows_[0] > 2 else ['Down'] * (2 - rows_[0])) + ['Return', 'sleep3', 'Return', 'sleep2'])
        return True

    run_actions(last)
    for i in range(6 * int(turns) + 8):
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
            run_actions(t)
    dosciv.stop()


def state(civrs, sve, turns, script=None):
    with tempfile.NamedTemporaryFile(suffix='.txt', delete=False) as f:
        path = f.name
    subprocess.run([civrs, '--load-sve', sve, '--dump', 'state:%d' % turns, '--out', path] + (['--actions', script] if script else []), check=True)
    with open(path) as f:
        lines = f.read().splitlines()
    os.remove(path)
    return dict(l.split(' ', 1) if ' ' in l else (l, '') for l in lines)


def diff(civrs, start, results, script=None):
    t0 = turn_of(start)
    for r in results:
        n = turn_of(r) - t0
        exe, rs = state(civrs, r, 0), state(civrs, start, n, script)
        keys = sorted(set(exe) | set(rs), key=lambda k: (k.split('.')[0].rstrip('0123456789,'), k))
        bad = [k for k in keys if exe.get(k) != rs.get(k)]
        print('== %s: %d turn(s), %d of %d fields differ' % (os.path.basename(r), n, len(bad), len(keys)))
        for k in bad:
            print('%s: %s | %s' % (k, exe.get(k, '-'), rs.get(k, '-')))


if __name__ == '__main__':
    args = sys.argv[1:]
    script = None
    if '--actions' in args:
        i = args.index('--actions')
        script = args[i + 1]
        del args[i:i + 2]
    if len(args) >= 5 and args[0] == 'play':
        play(*args[1:5], script=script)
    elif len(args) >= 4 and args[0] == 'diff':
        diff(args[1], args[2], args[3:], script)
    else:
        print(__doc__)
