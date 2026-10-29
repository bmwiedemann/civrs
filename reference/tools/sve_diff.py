#!/usr/bin/env python3
"""Differential test of civrs against CIV.EXE from the same original saved game.

    python3 -I sve_diff.py play GAME_DIR SLOT TURNS OUT_DIR [--actions SCRIPT] [--seed N]
        Start CIV.EXE in dosbox (dosciv.py) on GAME_DIR (a private writable copy of the game),
        load CIVIL<SLOT>.SVE (SLOT `new`: start a new game, Chieftain, 7 civilizations, the
        Romans), turn Instant Advice and AutoSave off and End of Turn on, then
        TURNS times:
        perform the script's actions of the turn (src/save/actions.rs; examples in
        actions/), skip the active unit (Space) until the turn ends, save to slot 2
        (CIVIL2.SVE) and copy every new turn's save to OUT_DIR/t<TURN>.SVE/.MAP. Space only
        skips a unit, so the human's units keep their orders; check OUT_DIR/shot*.png and
        act*.png for dialogs the Space/Return keys answered (production menus, advances).
        The palace screen gets its first section and style, the copy-protection quiz
        (every 50th turn in BC) its right answer (quiz.py).
    python3 -I sve_diff.py diff CIVRS START.SVE RESULT.SVE [RESULT.SVE ...] [--actions SCRIPT] [--seed N]
        For each CIV.EXE result: civrs imports START.SVE (`new`: starts the new game of the
        seed with the same setup, `civrs --dump state:N:SEED`), plays (result turn - start turn)
        turns with the human doing only the script's actions (--dump state:N) and is diffed
        against civrs's import of the result (--dump state:0). Prints `field: CIV.EXE | civrs`
        per differing line.

The SVE keeps no RNG state (CIV.EXE seeds rand() from the clock at program start), so AI moves,
huts and anything else behind random() diverge; the human's cities, units, gold and research
and the turn/year should agree exactly.

--seed N: GAME_DIR's CIV.EXE is built by civ_seed_patch.py with --trace --quiet-ui. play sets
CIVSEED=N (rand() seeded with N & 0x7fff) and keeps CIV.EXE's draws (GAME_DIR/RANDOM.LOG, emptied at
the start) in OUT_DIR/random.log; diff runs civrs with the same seed (--dump state:N:SEED
--rng-log) and reports, besides the fields, the first draw where the two streams part (the
draw's index, CIV.EXE's call address and civrs's, n and the result), with the draws around it.
Both draw nothing for pictures (--quiet-ui), so a loaded game's first draw is its first game
draw.
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
    if k == 'ret':
        return ['Return', 'sleep2']
    if k[0] == 'k':
        # kN:ROW: the move opens a menu (a caravan's "Will you?"), answered with its row
        if ':' in k:
            return [KEYPAD[k[1]], 'sleep2'] + rows(k.split(':')[1]) + ['Return', 'sleep2']
        return [KEYPAD[k[1]], 'sleep2']
    if k == 'b':
        # the name prompt takes the proposed name, then the founding animation and city view
        return ['b', 'sleep2', 'Return', 'sleep8', 'Escape', 'sleep2']
    return [{'P': 'shift+p', 'D': 'shift+d'}.get(k, k), 'sleep2']


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
        # the map view may still be scrolling to the unit the last key made active
        k(['sleep1'])
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
    elif w[0] == 'menu':
        # the treaty question a goto step opened (0x656d -> 0x1a3b4): row 0 cancels, 1 breaks
        # the treaty; a Senate overrule after that needs one more Return
        k(rows(w[1]) + ['Return', 'sleep2'])
    else:
        raise ValueError('unknown action %s' % w)


def play(game, slot, turns, out, script=None, seed=None):
    import dosciv
    import quiz
    os.makedirs(out, exist_ok=True)
    actions = parse_actions(script) if script else {}
    log = os.path.join(game, 'RANDOM.LOG')
    if seed is not None:
        os.environ['CIVSEED'] = str(int(seed))
        open(log, 'wb').close()
    dosciv.start(game)
    k = dosciv.keys
    time.sleep(6)
    save = os.path.join(game, 'CIVIL2.SVE')
    if os.path.exists(save):
        os.remove(save)
    if slot == 'new':
        # graphics VGA, no sound, mouse and keyboard; "Start a New Game", the story,
        # Chieftain, 7 civilizations, the Romans, "Caesar", the leader's introduction, the
        # first turn's help overlay and its note (Instant Advice is on at Chieftain)
        k(['1', 'sleep2', '1', 'sleep8', '1', 'sleep10', 'Return', 'sleep5'])
        k(['Return', 'sleep3', 'Return', 'sleep3', 'Return', 'sleep3', 'Return', 'sleep3', 'Return', 'sleep6', 'Return', 'sleep4', 'space', 'sleep12'])
        k(['Return', 'sleep3', 'Return', 'sleep3'])
        # a new game's options: Instant Advice and AutoSave on, End of Turn off
        opts, last = 3, 0
    else:
        # graphics VGA, no sound, mouse and keyboard; the title menu's "Load a Saved Game"; drive C:
        k(['1', 'sleep2', '1', 'sleep8', '1', 'sleep10', 'Return', 'sleep5', 'Down', 'Return', 'sleep3', 'Return', 'sleep2'])
        k(['Down'] * int(slot) + ['Return', 'sleep6'])
        # Options (SVE 0x8aa4): Instant Advice off (its notes would take keys) and End of
        # Turn on (else the turn ends by itself after the last unit's move, before the
        # script is done)
        with open(os.path.join(game, 'CIVIL%s.SVE' % slot), 'rb') as f:
            f.seek(0x8aa4)
            opts = f.read(1)[0]
        last = turn_of(os.path.join(game, 'CIVIL%s.SVE' % slot))
    # and AutoSave off (its save every 50th turn to CIVIL4..9, game_turn 0x2796, would take
    # the harness's Return for its drive prompt and overwrite a slot of GAME_DIR)
    toggle = (['Return'] if opts & 1 else []) + ['Down'] + (['Return'] if opts & 2 else []) + ['Down'] + (['Return'] if not opts & 4 else [])
    k(['alt+g', 'sleep1', 'Down', 'Down', 'Down', 'Return', 'sleep1'] + toggle + ['Escape', 'sleep1'])
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

    def palace(px):
        # the palace screen (overlay 17, o 0x15f75): "Which section shall we improve?" in a
        # grey text box at (40, 16) of the screen over the sky of CBACK.PIC
        return px(81, 48) == b'\0\0\0' and px(83, 48) == b'\xff\xff\xff' and px(100, 40) == bytes([170] * 3) and min(px(40, 20)) > 200

    def improve_palace():
        # a section key 1..7 is taken while its level is below 4 (unbuilt ones snap to the
        # nearest built edge), a part key A..C while its level is below 3; the box goes once a
        # key is taken. Then '1' is the first style (a part takes it as the closing key), and
        # while the new palace stands under its sky it waits for a key: Escape. A Return
        # would be taken on the map, where the End of Turn prompt ends the turn unsaved.
        for key in ['4', '3', '5', '2', '6', '1', '7', 'a', 'b', 'c']:
            k([key, 'sleep3'])
            if not palace(screen()):
                k(['1', 'sleep6'])
                for _ in range(3):
                    if min(screen()(40, 20)) <= 200:
                        break
                    k(['Escape', 'sleep4'])
                return

    def save_game():
        # the GAME menu opens with Tax Rate highlighted (cyan at (100, 32)); anything else is
        # a dialog (city view, production or research menu), answered with Return, or the
        # palace screen
        k(['alt+g', 'sleep1'])
        px = screen()
        if px(100, 32) != cyan:
            if palace(px):
                print('palace screen', flush=True)
                improve_palace()
            elif quiz.is_quiz(px) and not quiz.is_question(px):
                # the quiz's verdict ("*CORRECT") waits for a key
                k(['Escape', 'sleep3'])
            elif quiz.is_quiz(px):
                # the copy-protection quiz (overlay 16): a second wrong answer would delete
                # every unit of the human. The menu follows the mouse pointer (menu_select
                # 0x1c867), so the answer is a click on its line
                dosciv.park()
                k(['sleep1'])
                q = os.path.join(out, 'quiz%02d.png' % n_shot[0])
                dosciv.shot(q)
                adv, lines, y = quiz.solve(game, q)
                print('quiz', adv, y, lines, flush=True)
                # not all the lines: the menu is still being drawn
                if y is not None:
                    dosciv.click(160, y)
                    k(['sleep6'])
                    # the verdict waits for a key on the quiz's page (its palette dimmed, the
                    # title still black); left alone, the next alt+g would close it
                    for _ in range(3):
                        p2 = screen()
                        bg = p2(5, 5)
                        if not (p2(300, 20) == p2(214, 20) == b'\0\0\0' and p2(600, 390) == bg and len(set(bg)) == 1 and bg[0] > 100):
                            break
                        # a second question (after a wrong answer) asks again
                        if quiz.is_question(p2):
                            break
                        k(['Escape', 'sleep3'])
                else:
                    k(['sleep2'])
            else:
                k(['Return', 'sleep2'])
            return False
        k(['Down', 'Down', 'Down', 'Down', 'Return', 'sleep2'])
        # the slot list: slot n is the band from y = 90 + 16 n; walk the cursor to slot 2.
        # The drive prompt comes first, but not after the palace screen (its require_disk):
        # a Return there would save to the slot under the cursor at once
        def slots():
            px = screen()
            return [n for n in range(10) if px(106, 90 + 16 * n + 4) == cyan]
        rows_ = slots()
        if not rows_:
            k(['Return', 'sleep2'])
            rows_ = slots()
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
    if seed is not None:
        shutil.copy(log, os.path.join(out, 'random.log'))


def state(civrs, sve, turns, script=None, seed=None):
    """civrs's state text after `turns` turns, and with a seed its draws [(at, n, r)]."""
    with tempfile.NamedTemporaryFile(suffix='.txt', delete=False) as f:
        path = f.name
    spec = 'state:%d' % turns + (':%d' % int(seed) if seed is not None else '')
    extra = ['--rng-log', path + '.rng'] if seed is not None else []
    load = [] if sve == 'new' else ['--load-sve', sve]
    subprocess.run([civrs] + load + ['--dump', spec, '--out', path] + (['--actions', script] if script else []) + extra, check=True)
    with open(path) as f:
        lines = f.read().splitlines()
    os.remove(path)
    draws = []
    if seed is not None:
        with open(path + '.rng') as f:
            draws = [(int(a, 16), int(n), int(r)) for a, n, r in (l.split() for l in f if l.strip())]
        os.remove(path + '.rng')
    return dict(l.split(' ', 1) if ' ' in l else (l, '') for l in lines), draws


# overlay number, code start and end in civ.overlay.bin (the catalogue's flat addresses)
OVL = [(1, 0x00375, 0x0076f), (2, 0x00b75, 0x02531), (3, 0x02775, 0x033ed), (4, 0x03775, 0x03bef),
       (5, 0x04175, 0x05ee3), (6, 0x06375, 0x0891f), (7, 0x08d75, 0x0aa93), (8, 0x0af75, 0x0c76b),
       (9, 0x0cb75, 0x0e0d1), (10, 0x0e375, 0x0ea1b), (11, 0x0ed75, 0x0fd0f), (12, 0x10175, 0x11ccd),
       (13, 0x11f75, 0x1291d), (14, 0x12d75, 0x148e7), (15, 0x14b75, 0x154ef), (16, 0x15775, 0x15cd1),
       (17, 0x15f75, 0x1700b), (18, 0x17575, 0x18c47), (19, 0x19175, 0x1ae19), (20, 0x1b375, 0x1c7b9),
       (21, 0x1cb75, 0x1cf9f), (22, 0x1d575, 0x1e6c7), (23, 0x1e975, 0x1f0fb)]
OVL_AREA = 0x22f30   # the overlays' load area: a call at offset k of an overlay's code logs 0x22f30 + k


def exe_draws(path):
    """CIV.EXE's draws from its RANDOM.LOG (civ_seed_patch.py --trace): [(call address, n,
    result, the caller's return address)]."""
    d = open(path, 'rb').read()
    out = []
    size = 16
    for i in range(len(d) // size):
        ip, cs, n, r, ip2, cs2 = struct.unpack_from('<6H', d, size * i)
        at = ((cs * 16 + ip) & 0xfffff) - 5
        out.append((at, n - 0x10000 if n >= 0x8000 else n, r - 0x10000 if r >= 0x8000 else r, (cs2 * 16 + ip2) & 0xfffff))
    return out


def same_site(exe_at, rs_at):
    if rs_at == 0:
        return True
    if rs_at & 0x100000:
        flat = rs_at & 0xfffff
        return any(s <= flat < e and exe_at == OVL_AREA + flat - s for _, s, e in OVL)
    return exe_at == rs_at


def name(exe_at):
    if exe_at >= OVL_AREA:
        k = exe_at - OVL_AREA
        return '/'.join('o%#x' % (s + k) for _, s, e in OVL if s + k < e) + ' (overlay +%#x)' % k
    return '%#x' % exe_at


def first_split(exe, rs):
    """The index of the first draw where the streams differ (n, result or call site)."""
    for i, (a, b) in enumerate(zip(exe, rs)):
        if a[1:3] != b[1:3] or not same_site(a[0], b[0]):
            return i
    return None if len(rs) <= len(exe) else len(exe)


def diff(civrs, start, results, script=None, seed=None):
    t0 = 0 if start == 'new' else turn_of(start)
    exe_log = None
    if seed is not None:
        exe_log = exe_draws(os.path.join(os.path.dirname(os.path.abspath(results[0])), 'random.log'))
    for r in results:
        n = turn_of(r) - t0
        (exe, _), (rs, draws) = state(civrs, r, 0), state(civrs, start, n, script, seed)
        keys = sorted(set(exe) | set(rs), key=lambda k: (k.split('.')[0].rstrip('0123456789,'), k))
        bad = [k for k in keys if exe.get(k) != rs.get(k)]
        print('== %s: %d turn(s), %d of %d fields differ' % (os.path.basename(r), n, len(bad), len(keys)))
        for k in bad:
            print('%s: %s | %s' % (k, exe.get(k, '-'), rs.get(k, '-')))
        if exe_log is not None:
            i = first_split(exe_log, draws)
            if i is None:
                print('random: the %d draws agree' % len(draws))
                continue
            print('random: draw %d of %d differs (CIV.EXE: address n result [from] | civrs: address n result)' % (i, len(draws)))
            for j in range(max(0, i - 4), min(max(len(draws), len(exe_log)), i + 5)):
                e = exe_log[j] if j < len(exe_log) else None
                c = draws[j] if j < len(draws) else None
                es = '%s %d %d [%#x]' % (name(e[0]), e[1], e[2], e[3]) if e else '-'
                cs = '%#x %d %d' % c if c else '-'
                print('%s %4d  %s | %s' % ('>' if j == i else ' ', j, es, cs))


if __name__ == '__main__':
    args = sys.argv[1:]
    script = seed = None
    if '--actions' in args:
        i = args.index('--actions')
        script = args[i + 1]
        del args[i:i + 2]
    if '--seed' in args:
        i = args.index('--seed')
        seed = int(args[i + 1])
        del args[i:i + 2]
    if len(args) >= 5 and args[0] == 'play':
        play(*args[1:5], script=script, seed=seed)
    elif len(args) >= 4 and args[0] == 'diff':
        diff(args[1], args[2], args[3:], script, seed)
    else:
        print(__doc__)
